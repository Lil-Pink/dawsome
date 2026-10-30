//! Where the window lands on the music. A document opens on what its playlist plays (the rule
//! asked for on 2026-10-25: "the piano roll always opens on Track 1"), not on an empty first
//! pattern, which played nothing; a double-click on a pattern clip shows that clip's pattern in
//! the piano roll. The opening rule reads the document alone, so it holds for files saved before
//! it, and nothing is written for it.

use dawsome_engine::project::{ChannelId, ClipId, ClipItem, PatternId, Project};
use dawsome_engine::time::{self, Ticks};
use iced::Task;

use super::{App, Message, PlayMode};
use crate::ui::note_gesture::Target;
use crate::ui::panels::Panel;

/// What an opened document selects, from the song's first clip: its pattern and the channel
/// heard in it. The window then plays the song (song mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Landing {
    /// The pattern the first clip plays; `None` for an audio clip, when the pattern box keeps
    /// the first pattern as before.
    pub pattern: Option<PatternId>,
    /// The first channel in rack order with notes in that pattern, or the audio clip's channel;
    /// `None` (a pattern without notes) keeps the first channel as before.
    pub channel: Option<ChannelId>,
    /// The first clip itself when it is a pattern clip: the clip the piano roll edits, whose
    /// link control it shows.
    pub clip: Option<ClipId>,
}

impl Landing {
    /// Where `project` opens: the first clip on Track 1 by its start (the lower id on a tie),
    /// else the first on the topmost track holding one; pattern and audio clips only, since an
    /// automation clip plays no music of its own. `None` when there is no such clip: the window
    /// opens as before, on the first pattern and channel.
    pub fn of(project: &Project) -> Option<Self> {
        project
            .playlist
            .clips
            .iter()
            .filter_map(|(&id, clip)| {
                let landing = match clip.item {
                    ClipItem::Pattern { pattern } => Landing {
                        pattern: Some(pattern),
                        channel: first_channel_with_notes(project, pattern),
                        clip: Some(id),
                    },
                    ClipItem::Audio { channel, .. } => Landing {
                        pattern: None,
                        channel: Some(channel),
                        clip: None,
                    },
                    ClipItem::Automation { .. } => return None,
                };
                // Track ids are in track order, Track 1 the lowest.
                Some(((clip.track, clip.start, id), landing))
            })
            .min_by_key(|&(key, _)| key)
            .map(|(_, landing)| landing)
    }
}

/// Whether a window starting on `project` lands on its music: not in a scripted run, which
/// opens as its flags say (`--song`), so its screenshots stay as they were. The landing, when
/// there is one, and whether the window starts in song mode (`song`: the `--song` flag).
pub fn at_start(project: &Project, scripted: bool, song: bool) -> (Option<Landing>, bool) {
    let landing = if scripted { None } else { Landing::of(project) };
    (landing, song || landing.is_some())
}

/// The first channel in rack order with a note in `pattern`.
pub fn first_channel_with_notes(project: &Project, pattern: PatternId) -> Option<ChannelId> {
    let notes = &project.patterns.get(&pattern)?.notes;
    project
        .channels
        .keys()
        .copied()
        .find(|&channel| notes.iter().any(|note| note.channel == channel))
}

impl App {
    /// Selects where `landing` says: its pattern in the pattern box and its channel in the rack,
    /// the piano roll and the keys, as a click on each would, and the roll edits its clip. The
    /// caller has the session play the song.
    pub fn land(&mut self, landing: Landing) {
        let patterns = &self.session.project().patterns;
        if let Some(pattern) = landing.pattern.filter(|id| patterns.contains_key(id)) {
            self.pattern = Some(pattern);
            self.roll.forget_notes();
        }
        match landing.channel {
            Some(channel) => {
                let result = self.change_selected_channel(Some(channel));
                self.report(result);
            }
            None => self.show_roll(),
        }
        if let Some(clip) = landing.clip {
            self.roll.edit_clip(self.session.project(), clip);
        }
        debug_assert_eq!(self.mode, PlayMode::Song, "a landing plays the song");
    }

    /// A double-click on pattern clip `clip`: its pattern in the pattern box, the first channel
    /// with notes in it selected (else the channel stays), the piano roll opened when it is
    /// closed (as its key opens it, making room the same way), editing that clip (its link
    /// control shows), and scrolled to the clip's first note. Anything else does nothing.
    pub(super) fn open_clip_in_roll(&mut self, clip: ClipId) -> Task<Message> {
        let project = self.session.project();
        let Some((pattern, offset)) = project
            .playlist
            .clips
            .get(&clip)
            .and_then(|clip| Some((clip.pattern()?, clip.offset)))
        else {
            return Task::none();
        };
        let Some(length) = project.patterns.get(&pattern).map(|p| p.length_bars) else {
            return Task::none();
        };
        // Where the clip starts in its pattern: a clip trimmed on the left starts past its
        // pattern's first notes.
        let end = time::bars_to_ticks(length, project.ppq).0.max(1);
        let from = Ticks(offset.0.rem_euclid(end));
        let channel = first_channel_with_notes(project, pattern);
        if Some(pattern) != self.pattern {
            self.keep_take_for_another_pattern();
        }
        self.select_pattern(pattern);
        if let Some(channel) = channel.filter(|&id| Some(id) != self.selected_channel) {
            self.select_channel(channel);
        }
        let opened = if self.panels.piano_roll {
            Task::none()
        } else {
            self.handle(Message::TogglePanel(Panel::PianoRoll))
        };
        // After the opening, which (as the key's) edits no clip.
        self.roll.edit_clip(self.session.project(), clip);
        let target = Target::of(self.session.project(), self.pattern, self.selected_channel);
        self.roll.reveal(target.as_ref(), from);
        opened
    }
}

#[cfg(test)]
mod tests {
    //! The opening rule on documents built by hand, each expected landing read off the document
    //! as written (which clip is first on which track, which channel has notes), and opened
    //! through File → Open; and the double-click on a clip through the real widgets.

    use std::fs;
    use std::path::{Path, PathBuf};

    use dawsome_engine::project::{Channel, Clip, Note, Pattern, TrackId};
    use dawsome_engine::time::MilliBpm;
    use iced::{mouse, Point, Size};

    use super::super::files::FileMessage;
    use super::super::tests::app_on;
    use super::super::widget_events::Window;
    use super::*;
    use crate::project_file;
    use crate::ui::layout::TOOLBAR_HEIGHT;
    use crate::ui::piano_layout;
    use crate::ui::playlist_layout::{self, HEADER_HEIGHT, RULER_HEIGHT};

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    fn temp_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("dawsome-app-landing-{}", std::process::id()))
            .join(test);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn note(channel: u32, start: i64, key: u8) -> Note {
        Note {
            channel: ChannelId(channel),
            start: Ticks(start),
            length: Ticks(48),
            velocity: 0.8,
            key,
        }
    }

    /// 120 BPM at 96 PPQ (a bar is 384 ticks), the default 8 tracks, the starter document's
    /// four rows (Kick 909, Snare, Hat closed, Clap as channels 1 to 4, no sounds needed: a
    /// missing sound only makes a notice) and an empty one-bar "Pattern 1".
    fn base() -> Project {
        let mut project = Project::new(44_100, MilliBpm(120_000), 96);
        for (id, name) in (1..).zip(["Kick 909", "Snare", "Hat closed", "Clap"]) {
            project
                .channels
                .insert(ChannelId(id), Channel::sampler(name, ""));
        }
        project.next_channel_id = ChannelId(5);
        project.patterns.insert(
            PatternId(1),
            Pattern {
                name: "Pattern 1".to_owned(),
                length_bars: 1,
                copied_from: None,
                notes: Vec::new(),
            },
        );
        project
    }

    fn add_pattern(project: &mut Project, id: u32, name: &str, notes: Vec<Note>) {
        let pattern = Pattern {
            name: name.to_owned(),
            length_bars: 4,
            copied_from: None,
            notes,
        };
        project.patterns.insert(PatternId(id), pattern);
    }

    fn place(project: &mut Project, clip: Clip) {
        let id = project.playlist.next_clip_id;
        project.playlist.clips.insert(id, clip);
        project.playlist.next_clip_id = ClipId(id.0 + 1);
    }

    /// The app on a document with no clips, then `project` written to a temp file and opened
    /// as File → Open opens it.
    fn opened(test: &str, project: &Project) -> App {
        assert_eq!(project.validate(), Ok(()));
        let dir = temp_dir(test);
        let path = dir.join("song.dawsome");
        fs::write(&path, project.to_json().expect("serializes")).expect("writes");
        let (mut app, _player) = app_on(&Path::new(FIXTURES).join("a6_starter.json"));
        assert_eq!(app.mode, PlayMode::Pattern);
        let _ = app.update(Message::File(FileMessage::OpenPicked(Some(path))));
        assert_eq!(app.title(), "song - Dawsome", "opened");
        app
    }

    /// The notes the piano roll shows: the selected channel's in the selected pattern, as
    /// (start, key).
    fn roll_notes(app: &App) -> Vec<(i64, u8)> {
        Target::of(app.session.project(), app.pattern, app.selected_channel)
            .map(|target| target.channel_notes().map(|n| (n.start.0, n.key)).collect())
            .unwrap_or_default()
    }

    // A real saved song (2026-10-25), in small: an empty
    // "Pattern 1", the piano's take as pattern 2 with notes on channel 1 alone, its one clip on
    // Track 1. It opens on pattern 2, the piano (channel 1) selected, in song mode, and the
    // roll shows the take's notes.
    #[test]
    fn a_take_on_track_1_opens_on_its_pattern_in_song_mode() {
        let mut project = base();
        project.channels.get_mut(&ChannelId(1)).expect("ch 1").name = "Upright Piano".to_owned();
        let take = vec![note(1, 0, 60), note(1, 96, 64), note(1, 192, 67)];
        add_pattern(&mut project, 2, "Upright Piano take 1", take);
        place(
            &mut project,
            Clip::of_pattern(PatternId(2), TrackId(1), Ticks(0), Ticks(1536)),
        );
        let app = opened("take_on_track_1", &project);
        assert_eq!(app.pattern, Some(PatternId(2)));
        assert_eq!(app.selected_channel, Some(ChannelId(1)));
        assert_eq!(app.mode, PlayMode::Song);
        assert_eq!(roll_notes(&app), [(0, 60), (96, 64), (192, 67)]);
        assert_eq!(
            app.session.mode(),
            dawsome_engine::plan::PlayMode::Song,
            "the player plays the song"
        );
    }

    // Clips on Track 3 only: pattern 3's at bar 3 comes after pattern 2's at bar 1, so pattern
    // 2 is chosen; its notes are on the Hat (3) and the Snare (2), and the Snare is first in
    // rack order. A clip on Track 5 starting earlier does not count: Track 3 is higher.
    #[test]
    fn clips_on_a_lower_track_open_on_its_first_clip() {
        let mut project = base();
        add_pattern(
            &mut project,
            2,
            "Beat",
            vec![note(3, 0, 60), note(2, 96, 60)],
        );
        add_pattern(&mut project, 3, "Fill", vec![note(4, 0, 60)]);
        add_pattern(&mut project, 4, "Late", vec![note(1, 0, 60)]);
        place(
            &mut project,
            Clip::of_pattern(PatternId(3), TrackId(3), Ticks(768), Ticks(384)),
        );
        place(
            &mut project,
            Clip::of_pattern(PatternId(2), TrackId(3), Ticks(0), Ticks(384)),
        );
        place(
            &mut project,
            Clip::of_pattern(PatternId(4), TrackId(5), Ticks(0), Ticks(384)),
        );
        let expected = Landing {
            pattern: Some(PatternId(2)),
            channel: Some(ChannelId(2)),
            clip: Some(ClipId(2)),
        };
        assert_eq!(Landing::of(&project), Some(expected));
        let app = opened("track_3", &project);
        assert_eq!(
            (app.pattern, app.selected_channel, app.mode),
            (Some(PatternId(2)), Some(ChannelId(2)), PlayMode::Song)
        );
        assert_eq!(roll_notes(&app), [(96, 60)]);
    }

    // No clip, or automation clips only: as before, the first pattern and the first channel,
    // in the mode the window was in (pattern mode).
    #[test]
    fn a_project_without_clips_opens_as_before() {
        let mut project = base();
        add_pattern(&mut project, 2, "Beat", vec![note(3, 0, 60)]);
        assert_eq!(Landing::of(&project), None);
        let app = opened("no_clips", &project);
        assert_eq!(
            (app.pattern, app.selected_channel, app.mode),
            (Some(PatternId(1)), Some(ChannelId(1)), PlayMode::Pattern)
        );
        // p8_auto_linear's song without its pattern clip: one automation clip, on Track 2.
        let mut automated = project_file::open(&Path::new(FIXTURES).join("p8_auto_linear.json"))
            .expect("opens")
            .project;
        assert!(Landing::of(&automated).is_some(), "its pattern clip counts");
        automated.playlist.clips.remove(&ClipId(1));
        assert!(automated
            .playlist
            .clips
            .values()
            .all(|c| c.automation().is_some()));
        assert_eq!(Landing::of(&automated), None);
    }

    // Audio clips only (an audio take on Track 2): the take's channel is selected, the pattern
    // box keeps the first pattern, and the window plays the song.
    #[test]
    fn audio_clips_alone_open_on_their_channel_in_song_mode() {
        let mut project = base();
        project
            .channels
            .insert(ChannelId(5), Channel::audio("Take 1", "song take 1.wav"));
        project.next_channel_id = ChannelId(6);
        place(
            &mut project,
            Clip::of_audio(ChannelId(5), TrackId(2), Ticks(384), Ticks(768)),
        );
        let app = opened("audio_only", &project);
        assert_eq!(
            (app.pattern, app.selected_channel, app.mode),
            (Some(PatternId(1)), Some(ChannelId(5)), PlayMode::Song)
        );
    }

    // A window started on a file: a run nobody scripts lands on the music (song mode); a
    // scripted run opens as its flags say.
    #[test]
    fn a_scripted_start_opens_as_its_flags_say() {
        let mut project = base();
        add_pattern(&mut project, 2, "Take", vec![note(1, 0, 60)]);
        place(
            &mut project,
            Clip::of_pattern(PatternId(2), TrackId(1), Ticks(0), Ticks(384)),
        );
        let landing = Landing::of(&project);
        assert!(landing.is_some());
        assert_eq!(at_start(&project, false, false), (landing, true));
        assert_eq!(at_start(&project, true, false), (None, false));
        assert_eq!(at_start(&project, true, true), (None, true));
        assert_eq!(at_start(&base(), false, false), (None, false));
    }

    // ---------------------------------------------------------------------------------------
    // Double-clicks on clips through the real widgets (app/widget_events.rs): the playlist alone
    // under the toolbar of a 1000 x 560 window, the hint bar off, so tracks point (x, y) is
    // window point (x, y + toolbar + header + ruler). Where a clip lies is read off the
    // playlist's own layout once the canvas has reported its size.
    // ---------------------------------------------------------------------------------------

    fn playlist_alone(path: &Path) -> (App, dawsome_engine::realtime::Player) {
        let (mut app, player) = app_on(path);
        let _ = app.update(Message::TogglePanel(Panel::Playlist));
        let _ = app.update(Message::TogglePanel(Panel::Rack));
        let _ = app.update(Message::WindowResized(Size::new(1000.0, 560.0)));
        app.hints_flag = Some(false);
        assert!(app.panels.playlist && !app.panels.rack && !app.panels.piano_roll);
        let mut window = Window::new(&app);
        window.mouse(
            &mut app,
            mouse::Event::CursorLeft,
            mouse::Cursor::Unavailable,
        );
        (app, player)
    }

    /// The window point in the middle of `clip`'s body.
    fn on_clip(app: &App, clip: ClipId) -> Point {
        let project = app.session.project();
        let shown = &project.playlist.clips[&clip];
        let index = crate::ui::clip_gesture::index_of(project, shown.track).expect("its track");
        let grid = &app.arranger.grid;
        let top = grid.row_top_of(index).expect("its row is shown");
        let rect = playlist_layout::clip_rect(grid, shown, top);
        let below = TOOLBAR_HEIGHT + HEADER_HEIGHT + RULER_HEIGHT;
        Point::new(rect.center_x(), rect.y + rect.height * 0.75 + below)
    }

    // A song of two clips: the empty "Pattern 1" on Track 1 at bar 1, and the 8-bar "Verse" on
    // Track 2 from bar 3, whose notes are on the Hat (3) alone, the first at tick 1152 (bar 4)
    // on key 48, then key 72 at 1536. The window starts on "Pattern 1" with the rack's first
    // channel. A single click on the Verse clip selects it and opens nothing; a double-click
    // selects the Verse (the pattern box follows) and the Hat, opens the piano roll (closed until
    // then, as its key opens it: it takes the keys) and scrolls it to the first note: bar 4 at
    // the left edge, key 48's row in view though the highest note's top row (key 72 + 5) would
    // leave it below. Nothing is edited, and the clip stays selected for Delete.
    #[test]
    fn a_double_click_on_a_pattern_clip_opens_it_in_the_piano_roll() {
        let mut project = base();
        let verse = Pattern {
            name: "Verse".to_owned(),
            length_bars: 8,
            copied_from: None,
            notes: vec![note(3, 1152, 48), note(3, 1536, 72)],
        };
        project.patterns.insert(PatternId(2), verse);
        place(
            &mut project,
            Clip::of_pattern(PatternId(1), TrackId(1), Ticks(0), Ticks(384)),
        );
        place(
            &mut project,
            Clip::of_pattern(PatternId(2), TrackId(2), Ticks(768), Ticks(3072)),
        );
        let dir = temp_dir("double_click");
        let path = dir.join("song.dawsome");
        fs::write(&path, project.to_json().expect("serializes")).expect("writes");
        let (mut app, _player) = playlist_alone(&path);
        assert_eq!(
            (app.pattern, app.selected_channel),
            (Some(PatternId(1)), Some(ChannelId(1)))
        );
        let before = app.session.project().clone();
        let mut window = Window::new(&app);
        let verse = on_clip(&app, ClipId(2));
        window.click(&mut app, verse);
        assert!(!app.panels.piano_roll, "a single click opens nothing");
        assert_eq!(app.pattern, Some(PatternId(1)));
        assert_eq!(app.arranger.selection, [ClipId(2)].into());
        window.click(&mut app, verse);
        assert_eq!(app.pattern, Some(PatternId(2)));
        assert_eq!(app.selected_channel, Some(ChannelId(3)));
        assert!(app.panels.piano_roll, "the roll opened");
        assert_eq!(app.focus, Panel::PianoRoll, "as its key opens it");
        assert_eq!(app.arranger.selection, [ClipId(2)].into());
        assert_eq!(app.session.project(), &before, "nothing edited");
        assert_eq!(app.roll.view.scroll, Ticks(1152));
        let in_view = |app: &App| {
            let y = app.roll.view.row_top(48);
            (0.0..=app.roll.grid_size().height - piano_layout::ROW_HEIGHT).contains(&y)
        };
        assert!(in_view(&app), "key 48 in view: top {}", app.roll.view.top);
        // Laid out at its size under the playlist, the first note is still in view.
        window.mouse(
            &mut app,
            mouse::Event::CursorLeft,
            mouse::Cursor::Unavailable,
        );
        assert!(in_view(&app), "key 48 in view: top {}", app.roll.view.top);
        assert_eq!(app.roll.view.scroll, Ticks(1152));
    }

    // An audio clip double-clicked opens its properties strip, as before, and nothing else:
    // the roll stays closed, the pattern and channel stay.
    #[test]
    fn a_double_click_on_an_audio_clip_opens_its_strip_and_nothing_else() {
        let (mut app, _player) = playlist_alone(&Path::new(FIXTURES).join("p9_stretch_ui.json"));
        let project = app.session.project().clone();
        let (&clip, _) = project
            .playlist
            .clips
            .iter()
            .find(|(_, clip)| clip.audio_channel().is_some())
            .expect("an audio clip");
        let (pattern, channel) = (app.pattern, app.selected_channel);
        let mut window = Window::new(&app);
        let at = on_clip(&app, clip);
        window.click(&mut app, at);
        window.click(&mut app, at);
        assert_eq!(app.arranger.properties, Some(clip));
        assert!(!app.panels.piano_roll);
        assert_eq!((app.pattern, app.selected_channel), (pattern, channel));
    }
}
