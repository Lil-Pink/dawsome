//! The piano roll's player: the strip's Play / Stop and Loop, and Shift+Space while the roll has
//! the keys. It is the one transport in another mode (the engine's `PlayMode::Channel`): the
//! roll's channel's notes of the roll's pattern, from the pattern's start, looped or once. So it
//! can never play beside the toolbar's: the toolbar's Play takes the transport back to the
//! toolbar's mode from that mode's start, the roll's Play takes it from the song or a pattern the
//! same way, Stop stops either, and the roll's Play is off while a take runs (recording is the
//! toolbar's). The roll showing another channel or pattern while it plays moves the player
//! there with the next plan.

use dawsome_engine::project::{ChannelId, PatternId};
use dawsome_engine::time::Ticks;
use dawsome_engine::transport::Control;

use super::{App, PlayMode};
use crate::ui::piano_roll::RollPlayer;

impl App {
    /// The roll strip's Play / Stop and Shift+Space: plays the roll's channel of its pattern
    /// from the pattern's start, also when the song or a pattern plays (it takes over), or
    /// stops it.
    pub(super) fn roll_play_stop(&mut self) {
        if self.roll_plays {
            self.stop();
            return;
        }
        // Off while a take runs, and while the latency measurement plays clicks of its own.
        if self.recording.recording() || self.measuring_latency() || self.roll_target().is_none() {
            return;
        }
        self.apply_output_choice();
        self.roll_plays = true;
        self.send_mode();
        // Seek jumps a playing transport to the start, Play starts a stopped one there.
        let result = self
            .session
            .send(Control::Seek(Ticks(0)))
            .and_then(|()| self.session.send(Control::Play));
        // Refused (the control ring full), what played before plays on as it was.
        if result.is_ok() {
            self.transport_on = true;
        } else {
            self.roll_plays = false;
            self.send_mode();
        }
        self.report(result);
    }

    /// The roll strip's Loop: the roll's Play loops the pattern, or plays it once. Saved only
    /// when off; while the roll plays, the next plan loops or ends the pass it plays.
    pub(super) fn toggle_roll_loop(&mut self) {
        let on = !self.settings.loops_roll_player();
        self.settings.set_loops_roll_player(on);
        self.save_settings();
        if self.roll_plays {
            self.send_mode();
        }
    }

    /// The roll's pattern played once to its end, where the player stopped by itself as Stop
    /// does: the toolbar's mode has the transport back, as at Stop, but no Stop is sent and the
    /// keys held are not forgotten. Why: the app hears of the stop only at its next timer tick,
    /// and a key struck in between sounds; a second Stop would end it, and a key forgotten would
    /// never send its release.
    pub(super) fn roll_played_out(&mut self) {
        self.transport_on = false;
        self.leave_roll_player();
        self.settle_controllers();
    }

    /// The transport back from the roll's player to the toolbar's mode, from that mode's start:
    /// playing on from there while it plays (the toolbar's Play), waiting there while stopped
    /// (Stop, or the roll's pattern played once to its end).
    pub(super) fn leave_roll_player(&mut self) {
        if !self.roll_plays {
            return;
        }
        self.roll_plays = false;
        self.send_mode();
        let result = self.session.send(Control::Seek(self.mode_start()));
        self.report(result);
    }

    /// Where the toolbar's mode plays from: a pattern's tick 0, the song marker.
    pub(super) fn mode_start(&self) -> Ticks {
        match self.mode {
            PlayMode::Pattern => Ticks(0),
            PlayMode::Song => self.song_marker,
        }
    }

    /// The pattern and channel the roll shows, while the project has both: what its Play plays.
    pub(super) fn roll_target(&self) -> Option<(PatternId, ChannelId)> {
        let project = self.session.project();
        let pattern = self
            .pattern
            .filter(|id| project.patterns.contains_key(id))?;
        let channel = self
            .selected_channel
            .filter(|id| project.channels.contains_key(id))?;
        Some((pattern, channel))
    }

    /// The roll's player as its strip and the hints show it.
    pub(super) fn roll_player(&self) -> RollPlayer {
        RollPlayer {
            playing: self.roll_plays,
            looping: self.settings.loops_roll_player(),
            recording: self.recording.recording(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use dawsome_engine::plan::PlayMode as PlanMode;
    use dawsome_engine::realtime::Player;

    use super::super::tests::app_on;
    use super::super::{Message, RecordMessage};
    use super::*;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    /// rp_keys_drums (48 kHz, 120 BPM: 250 samples a tick; pattern 1 a bar, pattern 2 "Verse"
    /// two) with the song marker on bar 2 (tick 384), so a song played from its start is told
    /// from a pattern's. The keys (channel 1) and pattern 1 are selected, as at boot.
    fn keys_drums() -> (App, Player) {
        let (mut app, player) = app_on(&Path::new(FIXTURES).join("rp_keys_drums.json"));
        app.move_song_marker(Ticks(384));
        if app.recording.count_in {
            let _ = app.update(Message::Record(RecordMessage::ToggleCountIn));
        }
        (app, player)
    }

    /// One device buffer (512 frames: the playhead moves two ticks), then the playhead timer.
    fn buffer(app: &mut App, player: &mut Player) {
        player.process(&mut [[0.0; 2]; 512]);
        let _ = app.update(Message::Tick);
    }

    /// What the transport does, as the table below names it.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Plays {
        Stopped,
        Pattern,
        Song,
        Roll,
        Recording,
    }

    /// What `app` plays now: the plan's mode, the play state and a take.
    fn plays(app: &App) -> Plays {
        if !app.playing {
            return Plays::Stopped;
        }
        if app.recording.recording() {
            return Plays::Recording;
        }
        match app.session.mode() {
            PlanMode::Pattern(_) => Plays::Pattern,
            PlanMode::Song => Plays::Song,
            PlanMode::Channel { .. } => Plays::Roll,
            PlanMode::Nothing => panic!("nothing of the document plays"),
        }
    }

    fn press(app: &mut App, press: &str) {
        let message = match press {
            "toolbar Play" => Message::Play,
            "roll Play" => Message::RollPlayStop,
            "Stop" => Message::Stop,
            "Record" => Message::Record(RecordMessage::ToggleArm),
            other => panic!("no press {other}"),
        };
        let _ = app.update(message);
    }

    /// `app` brought to `state`, one buffer played in it.
    fn into_state(app: &mut App, player: &mut Player, state: &str) {
        match state {
            "stopped" => {}
            "song playing" => {
                let _ = app.update(Message::ModeSelected(PlayMode::Song));
                press(app, "toolbar Play");
            }
            "pattern playing" => press(app, "toolbar Play"),
            "roll playing" => press(app, "roll Play"),
            "roll playing, toolbar on Song" => {
                let _ = app.update(Message::ModeSelected(PlayMode::Song));
                press(app, "roll Play");
            }
            "recording" => {
                press(app, "Record");
                press(app, "toolbar Play");
            }
            other => panic!("no state {other}"),
        }
        buffer(app, player);
    }

    // Known answers, worked by hand from the roll player's rules for every state and press: the
    // transport after the press and one more buffer, and the playhead's tick. From a start a
    // buffer reaches tick 2 (512 frames, 250 a tick); played on it reaches tick 4; the song from
    // its marker (384) reaches 386, played on 388. The toolbar's Play while the roll plays takes
    // its mode back from that mode's start; the roll's Play while the song or a pattern plays
    // takes over from the pattern's start; the roll's Play while it plays is its Stop; Stop
    // stops whatever plays; the roll's Play while a take runs does nothing (recording is the
    // toolbar's); Record arms (or, during a take, disarms: the take is kept and the pattern
    // plays on); Play while playing plays on. One transport throughout: the plan's one mode.
    #[test]
    fn every_press_in_every_state_leaves_one_transport_in_the_mode_the_spec_gives() {
        use Plays::{Pattern, Recording, Roll, Song, Stopped};
        let table: [(&str, &str, Plays, Option<i64>); 24] = [
            ("stopped", "toolbar Play", Pattern, Some(2)),
            ("stopped", "roll Play", Roll, Some(2)),
            ("stopped", "Stop", Stopped, None),
            ("stopped", "Record", Stopped, None),
            ("song playing", "toolbar Play", Song, Some(388)),
            ("song playing", "roll Play", Roll, Some(2)),
            ("song playing", "Stop", Stopped, None),
            ("song playing", "Record", Song, Some(388)),
            ("pattern playing", "toolbar Play", Pattern, Some(4)),
            ("pattern playing", "roll Play", Roll, Some(2)),
            ("pattern playing", "Stop", Stopped, None),
            ("pattern playing", "Record", Pattern, Some(4)),
            ("roll playing", "toolbar Play", Pattern, Some(2)),
            ("roll playing", "roll Play", Stopped, None),
            ("roll playing", "Stop", Stopped, None),
            ("roll playing", "Record", Roll, Some(4)),
            (
                "roll playing, toolbar on Song",
                "toolbar Play",
                Song,
                Some(386),
            ),
            ("roll playing, toolbar on Song", "roll Play", Stopped, None),
            ("roll playing, toolbar on Song", "Stop", Stopped, None),
            ("roll playing, toolbar on Song", "Record", Roll, Some(4)),
            ("recording", "toolbar Play", Recording, Some(4)),
            ("recording", "roll Play", Recording, Some(4)),
            ("recording", "Stop", Stopped, None),
            ("recording", "Record", Pattern, Some(4)),
        ];
        for (state, pressed, want, tick) in table {
            let (mut app, mut player) = keys_drums();
            into_state(&mut app, &mut player, state);
            press(&mut app, pressed);
            buffer(&mut app, &mut player);
            let case = format!("{state} × {pressed}");
            assert_eq!(plays(&app), want, "{case}");
            assert_eq!(app.playhead, tick.map(Ticks), "{case}: the playhead");
            assert_eq!(app.roll_plays, want == Roll, "{case}: the roll's state");
            // The toolbar lights its mode unless the roll plays; then neither side.
            let lit = app.toolbar().mode;
            assert_eq!(
                lit,
                (want != Roll).then_some(app.mode),
                "{case}: the switch"
            );
            if want == Stopped {
                let toolbar = match app.mode {
                    PlayMode::Pattern => PlanMode::Pattern(PatternId(1)),
                    PlayMode::Song => PlanMode::Song,
                };
                assert_eq!(
                    app.session.mode(),
                    toolbar,
                    "{case}: the toolbar's mode back"
                );
                assert!(!app.transport_on, "{case}: Space plays next");
            }
        }
    }

    // Armed while the roll plays, the toolbar's Play records a take in the toolbar's mode from
    // its start: the roll's player stops first (a playing player refuses Record).
    #[test]
    fn armed_the_toolbars_play_takes_over_from_the_roll_and_records() {
        let (mut app, mut player) = keys_drums();
        into_state(&mut app, &mut player, "roll playing");
        press(&mut app, "Record");
        press(&mut app, "toolbar Play");
        buffer(&mut app, &mut player);
        assert_eq!(plays(&app), Plays::Recording);
        assert_eq!(app.session.mode(), PlanMode::Pattern(PatternId(1)));
        assert_eq!(app.playhead, Some(Ticks(2)), "from the pattern's start");
        assert!(!app.roll_plays);
    }

    // The roll's player plays what the roll shows: another channel selected, then another
    // pattern, each in the next plan, the transport playing on; Loop off makes the next plan
    // play the pass out once. Loop is saved only when off.
    #[test]
    fn the_roll_player_follows_the_roll_and_its_loop() {
        let (mut app, mut player) = keys_drums();
        into_state(&mut app, &mut player, "roll playing");
        let channel = |pattern, channel, once| PlanMode::Channel {
            pattern: PatternId(pattern),
            channel: ChannelId(channel),
            once,
        };
        assert_eq!(app.session.mode(), channel(1, 1, false));
        let _ = app.update(Message::ChannelSelected(ChannelId(2)));
        assert_eq!(app.session.mode(), channel(1, 2, false), "the drums");
        let _ = app.update(Message::PatternSelected(PatternId(2)));
        assert_eq!(
            app.session.mode(),
            channel(2, 2, false),
            "the Verse's drums"
        );
        let _ = app.update(Message::ToggleRollLoop);
        assert_eq!(app.session.mode(), channel(2, 2, true), "once");
        assert!(!app.settings.loops_roll_player());
        buffer(&mut app, &mut player);
        assert!(app.playing && app.roll_plays, "it played on throughout");
        let _ = app.update(Message::ToggleRollLoop);
        assert!(app.settings.loops_roll_player());
        assert_eq!(app.session.mode(), channel(2, 2, false), "looped again");
    }

    // Loop off: the roll plays its pattern once (a bar, 96000 frames: 188 buffers) and the
    // transport stops by itself at its end; the app hears it stop and hands the transport
    // back to the toolbar's mode, as Stop does, so Space plays the pattern next.
    #[test]
    fn played_once_the_roll_stops_and_the_toolbar_has_the_transport_back() {
        let (mut app, mut player) = keys_drums();
        let _ = app.update(Message::ToggleRollLoop);
        press(&mut app, "roll Play");
        for _ in 0..187 {
            buffer(&mut app, &mut player);
        }
        assert!(app.playing && app.roll_plays, "still in its pass");
        buffer(&mut app, &mut player);
        assert!(!app.playing && !app.roll_plays && !app.transport_on);
        assert_eq!(app.session.mode(), PlanMode::Pattern(PatternId(1)));
        assert_eq!(app.toolbar().mode, Some(PlayMode::Pattern));
        let _ = app.update(Message::TogglePlay);
        buffer(&mut app, &mut player);
        assert_eq!(plays(&app), Plays::Pattern);
    }

    /// Where the strip's text starting with `start` lies, as the window lays it out (`app`'s
    /// view, dumped offscreen): the box of the button (or text) holding it, (left, right).
    fn strip_box(app: &App, start: &str) -> (f32, f32) {
        let dump = super::super::layout_dump::dump(
            app.view(),
            app.window_size,
            crate::zoom::Zoom::DEFAULT,
            1.0,
        );
        let strip = crate::ui::layout::TOOLBAR_HEIGHT
            ..crate::ui::layout::TOOLBAR_HEIGHT + crate::ui::piano_layout::STRIP_HEIGHT;
        let run = dump["texts"]
            .as_array()
            .expect("texts")
            .iter()
            .find(|run| {
                strip.contains(&(run["y"].as_f64().unwrap_or(-1.0) as f32))
                    && run["text"]
                        .as_str()
                        .is_some_and(|text| text.starts_with(start))
            })
            .unwrap_or_else(|| panic!("no \"{start}\" in the strip"));
        let path = run["path"].as_str().expect("a path");
        let holder = path.rsplit_once('.').map_or(path, |(parent, _)| parent);
        let found = dump["boxes"]
            .as_array()
            .expect("boxes")
            .iter()
            .find(|entry| entry["path"].as_str() == Some(holder))
            .unwrap_or(run);
        let left = found["x"].as_f64().expect("x") as f32;
        (left, left + found["width"].as_f64().expect("width") as f32)
    }

    // The Scale chooser is half as wide as one of its longest entry ("Minor pentatonic":
    // 95.99 px of text, 125.99 px as the toolkit sized it), whole pixels down: 62. Opened, its
    // menu is that whole 125.99 px from the chooser's left edge, so every name shows whole, the
    // face's entry found among them. With "Minor pentatonic" chosen the strip's controls end the
    // root chooser (its own width, 46.55: "C#" and the same arrow and padding) and a spacing
    // later. The strip against the docked roll's 700 px is printed: where Loop (the last
    // control) ends, its spacing, the pop-out target (24) and the margin (12).
    #[test]
    fn the_scale_chooser_is_half_as_wide_and_its_menu_shows_every_name_whole() {
        use crate::ui::fonts::size;
        use crate::ui::panels::Panel;
        use crate::ui::piano_layout::{STRIP_HEIGHT, STRIP_PADDING, STRIP_SPACING};
        use crate::ui::scale::{Root, ScaleMode};
        use crate::ui::short_pick_list::natural_width;
        let (mut app, _player) = app_on(&Path::new(FIXTURES).join("p3_roll.json"));
        let _ = app.update(Message::TogglePanel(Panel::PianoRoll));
        let _ = app.update(Message::TogglePanel(Panel::Rack));
        let natural = natural_width(&ScaleMode::ALL, size::LABEL);
        let width = crate::ui::piano_roll::scale_width();
        assert!((natural - 125.99414).abs() < 0.001, "{natural}");
        assert_eq!(width, 62.0);
        assert!(width <= natural / 2.0);
        let (fold, _) = strip_box(&app, "Fold");
        // The strip's last control: Loop, right of Notes.
        let (_, last) = strip_box(&app, "Loop");
        let chooser = fold - STRIP_SPACING - width;
        let at = iced::Point::new(
            chooser + width / 2.0,
            crate::ui::layout::TOOLBAR_HEIGHT + STRIP_HEIGHT / 2.0,
        );
        let mut window = super::super::widget_events::Window::new(&app);
        let _ = window.click(&mut app, at);
        let menu = window.overlays(&app);
        let menu = menu.first().expect("the menu opened");
        assert!(
            (menu.x - chooser).abs() < 0.01,
            "from the chooser's left edge"
        );
        assert!(
            (menu.width - natural).abs() < 0.01,
            "as wide as the longest name"
        );
        // A press before the pointer moved over a row chooses the row the menu opened on: the
        // entry the face shows (cut) is found among the menu's whole names.
        let chosen = window.click_overlay(&app, iced::Point::new(menu.x + 20.0, menu.y + 5.0));
        assert!(
            matches!(
                chosen.as_slice(),
                [Message::Roll(crate::ui::roll::RollMessage::ScaleSelected(
                    ScaleMode::Off
                ))]
            ),
            "{chosen:?}"
        );
        let pentatonic = crate::ui::roll::RollMessage::ScaleSelected(ScaleMode::MinorPentatonic);
        let _ = app.update(Message::Roll(pentatonic));
        let (_, with_scale) = strip_box(&app, "Loop");
        let root = natural_width(&Root::ALL, size::LABEL);
        assert!((with_scale - last - (root + STRIP_SPACING)).abs() < 0.01);
        let docked = |end: f32| end + STRIP_SPACING + crate::ui::pop_out::SIZE + STRIP_PADDING;
        println!(
            "the strip against a docked 700 px roll: {} with no scale, {} with Minor \
             pentatonic and its root",
            docked(last),
            docked(with_scale)
        );
    }

    // The strip as the window lays it out folds as its law says (crate::ui::piano_roll's
    // `the_strip_folds_by_its_width` derives the table by hand): p3_roll's "lead", docked, at
    // 1000 px (the roll alone in a 1000 px window), 700 (beside the Library's 300 in 1000) and
    // 640 (beside it in 940), with no scale and with Minor pentatonic and its root. The Len
    // readout shows only unfolded, Preview and Follow say their words until the glyphs fold,
    // and Loop, the last control, ends at the strip's left + the width its fold needs − the
    // right margin (12) − the grip and its spacing (34) − the player's room after Loop (5):
    //   1000: 733.22 − 51 = 682.22 / 789.77 − 51 = 738.77; 700: 300 + 656.90 − 51 = 905.90 /
    //   300 + 646.70 − 51 = 895.70; 640: 300 + 590.14 − 51 = 839.14 / 300 + 632.70 − 51 = 881.70
    // (pins moved 1 px left when the player took its room: 4 px wider, Loop 5 in from its end)
    // (each with this system's "▾" past 7.911 px, as there). Folded to the scale, its chooser
    // is 48 px: it starts 48 + a spacing before the root's chooser.
    #[test]
    fn the_strip_folds_in_the_window_as_its_law_says() {
        use crate::ui::fonts::size;
        use crate::ui::panels::Panel;
        use crate::ui::piano_layout::{
            SCALE_FOLDED_WIDTH, STRIP_GROUP_GAP, STRIP_HEIGHT, STRIP_SPACING,
        };
        use crate::ui::roll::RollMessage;
        use crate::ui::scale::{Root, ScaleMode};
        use crate::ui::short_pick_list::natural_width;
        use crate::ui::text_fit::text_width;
        let drift = text_width("Notes \u{25be}", size::LABEL) - (36.867_188 + 7.911);
        let shows = |app: &App, start: &str| {
            let dump = super::super::layout_dump::dump(
                app.view(),
                app.window_size,
                crate::zoom::Zoom::DEFAULT,
                1.0,
            );
            let strip =
                crate::ui::layout::TOOLBAR_HEIGHT..crate::ui::layout::TOOLBAR_HEIGHT + STRIP_HEIGHT;
            dump["texts"].as_array().expect("texts").iter().any(|run| {
                strip.contains(&(run["y"].as_f64().unwrap_or(-1.0) as f32))
                    && run["text"]
                        .as_str()
                        .is_some_and(|text| text.starts_with(start))
            })
        };
        for (window, library, scale, loop_end, len, words) in [
            (1000.0, false, false, 682.22, true, true),
            (1000.0, false, true, 738.77, true, true),
            (1000.0, true, false, 905.90, false, true),
            (1000.0, true, true, 895.70, false, false),
            (940.0, true, false, 839.14, false, false),
            (940.0, true, true, 881.70, false, false),
        ] {
            let (mut app, _player) = app_on(&Path::new(FIXTURES).join("p3_roll.json"));
            let _ = app.update(Message::WindowResized(iced::Size::new(window, 560.0)));
            let _ = app.update(Message::TogglePanel(Panel::PianoRoll));
            let _ = app.update(Message::TogglePanel(Panel::Rack));
            if library {
                let _ = app.update(Message::TogglePanel(Panel::Browser));
            }
            if scale {
                let pentatonic = RollMessage::ScaleSelected(ScaleMode::MinorPentatonic);
                let _ = app.update(Message::Roll(pentatonic));
            }
            let case = format!("{window} px, the Library {library}, a scale {scale}");
            assert_eq!(shows(&app, "Len "), len, "{case}: Len");
            assert_eq!(shows(&app, "Preview"), words, "{case}: Preview's words");
            assert_eq!(shows(&app, "Follow"), words, "{case}: Follow's words");
            let (_, end) = strip_box(&app, "Loop");
            assert!(
                (end - drift - loop_end).abs() < 0.01,
                "{case}: Loop ends at {end}"
            );
            assert!(
                end + STRIP_SPACING <= window - 12.0 - 24.0,
                "{case}: clear of the grip"
            );
            if window == 940.0 && scale {
                let (fold, _) = strip_box(&app, "Fold");
                let root = natural_width(&Root::ALL, size::LABEL);
                let chooser = fold - STRIP_SPACING - root - STRIP_SPACING - SCALE_FOLDED_WIDTH;
                // Follow's glyph ends a group's gap before the chooser: 300 + 12 + 27.986 + 48
                // + 60.322 + 20 + 58 = 526.31.
                let follow_end = chooser - STRIP_GROUP_GAP;
                assert!((follow_end - 526.31).abs() < 0.01, "{case}: {follow_end}");
            }
        }
    }

    // Review suspicion: the app hears the roll's once end only at its next timer tick, after
    // the player stopped by itself; a key struck on a MIDI device in between must sound on
    // through that tick (no second Stop ends it). The pass ends in the 188th buffer (96000
    // frames); the key is struck after it, and the tick comes next.
    #[test]
    fn a_key_struck_after_the_roll_stops_by_itself_sounds_on_through_the_next_tick() {
        use dawsome_engine::realtime::EngineEvent;
        let (mut app, mut player) = keys_drums();
        let _ = app.update(Message::ToggleRollLoop);
        press(&mut app, "roll Play");
        for _ in 0..187 {
            buffer(&mut app, &mut player);
        }
        player.process(&mut [[0.0; 2]; 512]);
        let device = crate::midi::Router::new(
            app.session.midi_inbox(),
            crate::midi::ControllerQueue::default(),
        );
        device.deliver(&[0x90, 60, 64]);
        player.process(&mut [[0.0; 2]; 64]);
        let started = app.session.take_events().into_iter().any(|event| {
            matches!(
                event,
                EngineEvent::LiveNoteStarted {
                    key: 60,
                    tick: None,
                    ..
                }
            )
        });
        assert!(started, "the key struck while stopped");
        let _ = app.update(Message::Tick);
        assert!(!app.playing && !app.roll_plays, "the app heard the stop");
        for _ in 0..4 {
            player.process(&mut [[0.0; 2]; 64]);
        }
        let ended: Vec<_> = app
            .session
            .take_events()
            .into_iter()
            .filter(|event| matches!(event, EngineEvent::LiveNoteEnded { key: 60, .. }))
            .collect();
        assert!(ended.is_empty(), "the held key was ended: {ended:?}");
    }

    // Shift+Space is the roll's Play / Stop only while the roll has the keys (after a click in
    // it); elsewhere it does nothing. The roll's Play is off while a take runs.
    #[test]
    fn shift_space_plays_the_roll_only_while_it_has_the_keys() {
        let (mut app, mut player) = keys_drums();
        let _ = app.update(Message::RollPlayStopKey);
        buffer(&mut app, &mut player);
        assert_eq!(plays(&app), Plays::Stopped, "the roll lacks the keys");
        app.panels.piano_roll = true;
        app.focus = crate::ui::panels::Panel::PianoRoll;
        let _ = app.update(Message::RollPlayStopKey);
        buffer(&mut app, &mut player);
        assert_eq!(plays(&app), Plays::Roll);
        let _ = app.update(Message::RollPlayStopKey);
        buffer(&mut app, &mut player);
        assert_eq!(plays(&app), Plays::Stopped);
        into_state(&mut app, &mut player, "recording");
        assert!(app.roll_player().recording, "the strip's Play shows off");
    }
}
