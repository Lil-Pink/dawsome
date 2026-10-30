//! Recording in the window: the Record button and its menu (what to record, the count-in), the
//! armed state and the armed track, and a take from Play to Stop. The master is recorded from
//! the player's record tap; the input from the selected input channel's device when the
//! selected channel is an input channel whose device is open (its record ring, its channels),
//! else from the system's default input device, open while the Input source is armed so its
//! meter moves before the take. Each take is written by a writer thread of its own and becomes
//! one new audio clip when it stops.
//!
//! With no audio source armed and Notes on, a take records the notes played live (a MIDI
//! device or the typing keyboard), quantized to the piano roll's snap when "Quantize notes to
//! snap" is on, each drawn in the roll the moment it ends: in pattern mode into the selected
//! pattern as it loops, in song mode into a pattern of its own and its clip from the song
//! marker, which grow by whole bars as it plays (the pattern box shows it). One undo step when
//! it stops, or when it is kept before Stop: Record disarmed, the document saved, another
//! pattern chosen in pattern mode or the play mode switched ([`App::keep_take`]); the song plays
//! on.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Instant;

use dawsome_engine::edit::takes;
use dawsome_engine::project::{ChannelId, InsertId, PatternId, TrackId};
use dawsome_engine::realtime::EngineEvent;
use dawsome_engine::time::Ticks;
use dawsome_engine::transport::Control;
use dawsome_media::record::{Feed, Recorded, Recorder, TapFeed};

use super::{App, PlayMode};
use crate::notes::{Finished, NoteTake};
use crate::play_to::RecordSource;
use crate::record::{self, InputStream};
use crate::ui::mixer_state::Meters;

/// What the toolbar says of where a take of the line in went: "Take 2 placed 26 ms early
/// (measured latency)" (late for a negative latency, "latency set by hand" for a figure set so),
/// the ms rounded half up at `rate`; "Take 2 not shifted: shorter than the input latency"; none
/// for a take placed as recorded with no latency kept.
fn shift_words(name: &str, shift: takes::Shift, measured: bool, rate: u32) -> Option<String> {
    match shift {
        takes::Shift::None => None,
        takes::Shift::TooShort => Some(format!(
            "{name} not shifted: shorter than the input latency"
        )),
        takes::Shift::By(frames) => {
            let rate = u64::from(rate.max(1));
            let ms = (frames.unsigned_abs() * 2000 + rate) / (2 * rate);
            let way = if frames > 0 { "early" } else { "late" };
            let how = if measured {
                "measured latency"
            } else {
                "latency set by hand"
            };
            Some(format!("{name} placed {ms} ms {way} ({how})"))
        }
    }
}

/// The notice for an edit refused under a take of notes.
pub(super) const STOP_RECORDING_FIRST: &str = "Stop the recording first";

/// What a take records: the switch beside Record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TakeKind {
    /// The notes played, into the piano roll.
    Notes,
    /// What is heard (or the line in), into a sound file.
    Audio,
}

/// What the Record button, its switch, its menus and their keys ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordMessage {
    /// The Record button, or R: arm or disarm; while armed, Play records.
    ToggleArm,
    /// A right press on the Record button: the menu of what it records now.
    ToggleMenu,
    CloseMenu,
    /// A press on one side of the switch: what the next take records.
    Kind(TakeKind),
    /// The Audio menu's rows: what an audio take records (and audio it is).
    Source(RecordSource),
    /// The Notes menu's "Quantize notes to snap" row.
    ToggleQuantize,
    /// A right press on one side of the switch beside Record: that side's menu.
    Menu(TakeKind),
    /// Ctrl+P, or a menu's count-in row.
    ToggleCountIn,
    /// Ctrl+Shift+R, the Capture button or the Notes menu's row: the notes played lately, while
    /// no take ran, into the pattern (pattern mode) or a new pattern and clip (song mode).
    Capture,
}

/// The window's recording state.
pub(super) struct Recording {
    /// What a take records: notes, or audio (from `source`).
    pub kind: TakeKind,
    /// What an audio take records.
    pub source: RecordSource,
    /// Move recorded notes' starts to the piano roll's snap.
    pub quantize: bool,
    /// A bar of clicks before the song plays.
    pub count_in: bool,
    pub armed: bool,
    /// The menu open: Notes' or Audio's.
    pub menu: Option<TakeKind>,
    /// The track whose arm light is lit: takes land there.
    pub track: Option<TrackId>,
    take: Option<Take>,
    /// A take of notes in progress.
    note_take: Option<NoteTake>,
    /// The pattern the pattern box showed before a song take showed its own, shown again when
    /// the take keeps nothing.
    shown_before: Option<PatternId>,
    /// The song tick the take running started at: the song marker, or 0 for a pattern.
    started: Ticks,
    input: Option<Input>,
    /// The input meter, as shown.
    meter: Meters,
}

impl Default for Recording {
    /// Notes, and what you hear when switched to Audio: no input device opens until the line
    /// in is chosen.
    fn default() -> Self {
        Self {
            kind: TakeKind::Notes,
            source: RecordSource::Master,
            quantize: true,
            count_in: true,
            armed: false,
            menu: None,
            track: None,
            take: None,
            note_take: None,
            shown_before: None,
            started: Ticks(0),
            input: None,
            meter: Meters::default(),
        }
    }
}

impl Recording {
    /// Whether a take is being recorded.
    pub fn recording(&self) -> bool {
        self.take.is_some() || self.note_take.is_some()
    }

    /// Whether a take of notes is being recorded.
    pub fn taking_notes(&self) -> bool {
        self.note_take.is_some()
    }

    /// What a take running records, if one runs.
    fn taking(&self) -> Option<TakeKind> {
        match (&self.note_take, &self.take) {
            (Some(_), _) => Some(TakeKind::Notes),
            (None, Some(_)) => Some(TakeKind::Audio),
            (None, None) => None,
        }
    }

    /// Whether the take's writer stopped on a failed write: the take must stop too.
    pub fn take_failed(&self) -> bool {
        self.take
            .as_ref()
            .is_some_and(|take| take.recorder.failed())
    }

    /// Whether the take holds all it records (one pass of the pattern): it stops.
    pub fn take_full(&self) -> bool {
        self.take.as_ref().is_some_and(|take| take.recorder.full())
    }

    /// Whether the line in is what a take records now.
    fn records_line_in(&self) -> bool {
        self.armed && self.kind == TakeKind::Audio && self.source == RecordSource::Input
    }

    /// The default input's meter level, while that input is open.
    fn default_input_level(&self) -> Option<f32> {
        self.input.as_ref()?;
        let [left, right] = self.meter.level(InsertId::MASTER);
        Some(left.max(right))
    }
}

/// An audio take in progress: its writer, the tick its clip starts at, its channel's name, and
/// the input channel whose record feed it holds, if any.
struct Take {
    recorder: Recorder,
    start: Ticks,
    name: String,
    source: RecordSource,
    input_channel: Option<ChannelId>,
    /// A take of the line in: the output and input devices it recorded with ("" for a system
    /// default), whose latency kept places it.
    line_in: Option<(String, String)>,
}

/// The open input device, the writer's half of its ring (with the take's writer while one
/// records) and its meter's peak cell.
struct Input {
    stream: InputStream,
    feed: Option<Box<dyn Feed>>,
    peak: Arc<AtomicU32>,
}

impl App {
    pub(super) fn record_message(&mut self, message: RecordMessage) {
        match message {
            RecordMessage::ToggleArm => {
                // The button the first-run note on recording points at: using it puts the note
                // away.
                self.put_away_record_callout();
                if self.recording.armed {
                    self.keep_take("Take kept: Record is off");
                }
                self.recording.armed = !self.recording.armed;
                self.recording.menu = None;
                self.update_input();
            }
            RecordMessage::ToggleMenu => {
                let open = self.recording.menu.is_some();
                self.recording.menu = (!open).then_some(self.recording.kind);
            }
            RecordMessage::Menu(kind) => {
                let open = self.recording.menu == Some(kind);
                self.recording.menu = (!open).then_some(kind);
            }
            RecordMessage::CloseMenu => self.recording.menu = None,
            RecordMessage::Kind(kind) => {
                // The next take records it; one running goes on as it began.
                self.recording.kind = kind;
                self.recording.menu = None;
                self.update_input();
            }
            RecordMessage::Source(source) => {
                self.recording.source = source;
                self.recording.kind = TakeKind::Audio;
                self.recording.menu = None;
                self.update_input();
            }
            RecordMessage::ToggleQuantize => {
                self.recording.quantize = !self.recording.quantize;
                self.recording.menu = None;
            }
            RecordMessage::ToggleCountIn => self.recording.count_in = !self.recording.count_in,
            RecordMessage::Capture => {
                self.recording.menu = None;
                self.capture_played();
            }
        }
    }

    /// Lights the arm light of `track`, or puts it out when it is lit (at most one is).
    pub(super) fn arm_track(&mut self, track: TrackId) {
        self.recording.track = (self.recording.track != Some(track)).then_some(track);
    }

    /// The input meter's level while Record → Input is armed: the input channel's it records
    /// from, else the default input's while that is open.
    pub(super) fn input_level(&self) -> Option<f32> {
        let armed = self.recording.records_line_in();
        match self.recording_input_channel().filter(|_| armed) {
            Some(id) => self.live_inputs.channel(id).map(|input| input.level()),
            None => self.recording.default_input_level(),
        }
    }

    /// Opens the system's default input device while the Input source is armed and no input
    /// channel is recorded from instead, and closes it otherwise. A take in progress keeps it.
    /// A take or a latency measurement in progress keeps it.
    pub(super) fn update_input(&mut self) {
        let wanted = self.recording.records_line_in() && self.recording_input_channel().is_none();
        let held = self.recording.take.is_some() || self.latency.is_some();
        if held || wanted == self.recording.input.is_some() {
            return;
        }
        if !wanted {
            self.recording.input = None;
            return;
        }
        self.open_default_input();
    }

    /// Opens the system's default input device, its meter and the writer's half of its ring;
    /// says why when it cannot.
    fn open_default_input(&mut self) {
        let rate = self.session.project().sample_rate;
        let playback = self
            .audio
            .as_ref()
            .map(crate::audio::Output::playback_clock);
        match InputStream::open(rate, self.session.record_clock(), playback) {
            Ok((stream, feed, peak)) => {
                self.recording.input = Some(Input {
                    stream,
                    feed: Some(Box::new(feed)),
                    peak,
                });
            }
            Err(error) => self.notice = Some(format!("Recording off: {error}")),
        }
    }

    /// Play while armed: starts a take of what the switch says, after the count-in: notes, or
    /// audio. Returns false, with the reason shown, when it cannot start.
    pub(super) fn start_take(&mut self) -> bool {
        self.recording.started = match self.mode {
            PlayMode::Pattern => Ticks(0),
            PlayMode::Song => self.song_marker,
        };
        match self.recording.kind {
            TakeKind::Notes => self.start_note_take(),
            TakeKind::Audio => self.start_audio_take(),
        }
    }

    /// An audio take into the recordings folder as "<project> take N.wav", its clip from the
    /// song marker: in song mode until Stop, in pattern mode one pass of the pattern, after
    /// which it stops by itself. Returns false, with the reason shown, when it cannot start.
    fn start_audio_take(&mut self) -> bool {
        let Some(dir) = self.recordings_dir() else {
            self.notice = Some("Save the project first: recordings go beside it".to_owned());
            return false;
        };
        let cap = match self.mode {
            PlayMode::Song => None,
            PlayMode::Pattern => {
                let project = self.session.project();
                let Some(pattern) = self.pattern.and_then(|id| project.patterns.get(&id)) else {
                    self.notice = Some("Recording a pattern needs a pattern".to_owned());
                    return false;
                };
                let length = dawsome_engine::time::bars_to_ticks(pattern.length_bars, project.ppq);
                dawsome_engine::time::tick_to_sample(
                    length,
                    project.sample_rate,
                    project.tempo,
                    project.ppq,
                )
            }
        };
        let (path, number) = record::next_take_path(&dir, &self.take_project_name());
        self.start_audio_into(path, record::take_channel_name(number), cap)
    }

    /// The project's name as takes are named after it: its file's name without the extension,
    /// "Untitled" for a document with none.
    fn take_project_name(&self) -> String {
        std::path::Path::new(&self.document.name)
            .file_stem()
            .map_or_else(
                || "Untitled".to_owned(),
                |stem| stem.to_string_lossy().into_owned(),
            )
    }

    /// Play while armed with Notes as the only source: after the count-in, in pattern mode the
    /// selected pattern loops and the notes played go into it; in song mode the song plays from
    /// the song marker and they go into a new pattern of the selected channel's and its clip
    /// there, which the pattern box then shows. Returns false, with the reason shown, when it
    /// cannot start.
    fn start_note_take(&mut self) -> bool {
        let quantize = self
            .recording
            .quantize
            .then(|| self.roll.snap.unit(&self.roll.view));
        // Notes heard before the take are none of its business.
        let _ = self.session.take_events();
        let take = match self.mode {
            PlayMode::Pattern => {
                let Some(pattern) = self.pattern else {
                    self.notice = Some("Note recording needs a pattern".to_owned());
                    return false;
                };
                NoteTake::new(pattern, quantize)
            }
            PlayMode::Song => match self.start_song_take(quantize) {
                Some(take) => take,
                None => return false,
            },
        };
        let result = self.session.send(Control::Record {
            count_in: self.recording.count_in,
            tap_master: false,
        });
        let started = result.is_ok();
        self.report(result);
        if started {
            self.session.guard_take(take.song_parts());
            self.recording.note_take = Some(take);
        } else {
            let result = take.abandon(&mut self.session);
            self.show_again_before_take();
            self.after_edit(result);
        }
        started
    }

    /// A take of notes in song mode from the song marker: its pattern, named after the selected
    /// channel ("Kick take 1"), and its clip, on the armed track or as the take places it, shown
    /// in the pattern box. `None`, with the reason shown, when they cannot be added.
    fn start_song_take(
        &mut self,
        quantize: Option<dawsome_engine::time::Ticks>,
    ) -> Option<NoteTake> {
        let project = self.session.project();
        let channel = self
            .selected_channel
            .and_then(|id| project.channels.get(&id))
            .map_or("Notes", |channel| channel.name.as_str());
        let name = takes::take_name(project, channel);
        let started = NoteTake::song(
            &mut self.session,
            name,
            self.song_marker,
            self.recording.track,
            quantize,
        );
        match started {
            Ok(take) => {
                self.recording.shown_before = self.pattern;
                self.pattern = Some(take.pattern());
                self.roll.forget_notes();
                self.after_edit(Ok(()));
                self.clear_panel_caches();
                Some(take)
            }
            Err(error) => {
                self.after_edit(Err(error));
                None
            }
        }
    }

    /// The pattern box shows again what it showed before a song take that kept nothing.
    fn show_again_before_take(&mut self) {
        let before = self.recording.shown_before.take();
        if let Some(before) = before.filter(|id| self.session.project().patterns.contains_key(id)) {
            self.pattern = Some(before);
            self.roll.forget_notes();
        }
    }

    /// Takes in what the player reported for the take of notes, if one is running; `playhead`,
    /// read after the events, is where a song take must reach.
    pub(super) fn hear_notes(&mut self, events: &[EngineEvent], playhead: Ticks) {
        let Some(take) = &mut self.recording.note_take else {
            return;
        };
        let mut result = Ok(());
        for event in events {
            let heard = take.hear(&mut self.session, event);
            if result.is_ok() {
                result = heard;
            }
        }
        // A song take holds the bar the playhead is in.
        let mut grew = false;
        if self.session.playing() {
            match take.follow(&mut self.session, playhead) {
                Ok(more) => grew = more,
                Err(error) if result.is_ok() => result = Err(error),
                Err(_) => {}
            }
        }
        if result.is_err() {
            self.after_edit(result);
        } else if grew
            || events
                .iter()
                .any(|event| matches!(event, EngineEvent::LiveNoteEnded { tick: Some(_), .. }))
        {
            // A note landed, or the take grew: the rack, the piano roll and the playlist show it.
            self.after_edit(Ok(()));
            self.clear_panel_caches();
        }
    }

    /// Before Stop reaches the player: what it reported so far goes into the take of notes,
    /// and the playhead it stops at, where held notes end.
    pub(super) fn note_take_end(&mut self) -> Option<Ticks> {
        self.recording.note_take.as_ref()?;
        // Through the one reader, so the roll's live notes and the lost-events check see what
        // the take hears (an end read here alone left its key lit).
        self.read_player_events();
        Some(self.session.position().ticks)
    }

    /// Completes the take of notes, its held notes ending at `tick`: one undo step. A song take
    /// with no note leaves nothing, and the pattern box shows what it showed before.
    pub(super) fn finish_note_take(&mut self, tick: Ticks) {
        let Some(take) = self.recording.note_take.take() else {
            return;
        };
        self.session.guard_take(None);
        let result = take.finish(&mut self.session, tick);
        if matches!(result, Ok(Finished::Nothing)) {
            self.show_again_before_take();
        }
        self.recording.shown_before = None;
        self.after_edit(result.map(drop));
        self.clear_panel_caches();
    }

    /// The notices' line's first words while a take runs: what it records and for how long,
    /// "Recording notes 0:04", counted on the song from where it started (0:00 in the count-in).
    pub(super) fn recording_words(&self) -> Option<String> {
        let what = match self.recording.taking()? {
            TakeKind::Notes => "notes",
            TakeKind::Audio => "audio",
        };
        let project = self.session.project();
        let elapsed = Ticks((self.session.position().ticks.0 - self.recording.started.0).max(0));
        let seconds = dawsome_engine::time::tick_to_sample(
            elapsed,
            project.sample_rate,
            project.tempo,
            project.ppq,
        )
        .map_or(0, |frames| frames / u64::from(project.sample_rate.max(1)));
        Some(format!(
            "Recording {what} {}:{:02}",
            seconds / 60,
            seconds % 60
        ))
    }

    /// The song tick where the piano roll's tick 0 lies: a song take's start while
    /// it records into the pattern shown, whose tick 0 lies there and which grows with the
    /// playhead; else 0, the roll showing the song's tick within the pattern as it loops. The
    /// roll's playhead, its live notes and its Follow all count from here.
    pub(super) fn roll_origin(&self) -> Ticks {
        let take = self.recording.note_take.as_ref();
        take.filter(|take| Some(take.pattern()) == self.pattern)
            .and_then(NoteTake::song_start)
            .unwrap_or(Ticks(0))
    }

    /// Where the piano roll draws the playhead (as `PianoRoll` maps it): the song's tick from
    /// [`roll_origin`](Self::roll_origin), within the pattern shown; none while stopped.
    #[cfg(test)]
    pub(super) fn roll_playhead(&self) -> Option<Ticks> {
        let project = self.session.project();
        let end = crate::ui::note_gesture::Target::of(project, self.pattern, self.selected_channel)
            .map_or(1, |target| target.end().0.max(1));
        let origin = self.roll_origin();
        self.playhead
            .map(|tick| Ticks((tick.0 - origin.0).rem_euclid(end)))
    }

    /// Where the piano roll draws `key`'s live bars on the selected channel now, as its grid
    /// maps them ([`live_from`](crate::ui::piano_roll::live_from), then
    /// [`bars`](crate::ui::live_notes::bars)); none while the key is not held.
    #[cfg(test)]
    pub(super) fn roll_live_bars(&self, key: u8) -> Vec<iced::Rectangle> {
        let project = self.session.project();
        let Some(target) =
            crate::ui::note_gesture::Target::of(project, self.pattern, self.selected_channel)
        else {
            return Vec::new();
        };
        let (live, song_tick) = crate::ui::piano_roll::live_from(
            &self.live_notes,
            target.channel,
            self.playhead,
            self.roll_origin(),
        );
        live.into_iter()
            .filter(|&(held, _)| held == key)
            .flat_map(|(_, started)| {
                crate::ui::live_notes::bars(&self.roll.view, target.end(), song_tick, key, started)
            })
            .collect()
    }

    /// Another pattern chosen: in pattern mode a take running is kept (it recorded the pattern
    /// shown); a song take records on into its own pattern.
    pub(super) fn keep_take_for_another_pattern(&mut self) {
        if self.mode == PlayMode::Pattern {
            self.keep_take("Take kept: another pattern was chosen");
        }
    }

    /// Keeps the take running before Stop, saying `why` in the toolbar: Record disarmed, the
    /// document saved, another pattern chosen in pattern mode, the play mode switched. The take
    /// becomes its undo step as at Stop (held notes end at the playhead), and the song plays on
    /// without recording; in the count-in nothing has played, so it stops.
    pub(super) fn keep_take(&mut self, why: &str) {
        if !self.recording.recording() {
            return;
        }
        let counting_in = self.session.counting_in();
        let note_end = self.note_take_end();
        let control = if counting_in {
            self.transport_on = false;
            Control::Stop
        } else {
            Control::EndRecording
        };
        let result = self.session.send(control);
        self.report(result);
        if counting_in {
            self.forget_live_notes();
        }
        self.finish_take();
        if let Some(tick) = note_end {
            self.finish_note_take(tick);
        }
        self.notice = Some(why.to_owned());
    }

    /// Starts an audio take written to `path`, its channel named after the file, with no end
    /// of its own. Returns false, with the reason shown, when it cannot start.
    #[cfg(test)]
    pub(super) fn start_take_into(&mut self, path: PathBuf) -> bool {
        let name = path
            .file_stem()
            .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
        self.start_audio_into(path, name, None)
    }

    /// Starts an audio take of the source chosen, written to `path`, its channel to be named
    /// `name`, at most `cap` frames long. Returns false, with the reason shown, when it cannot
    /// start.
    fn start_audio_into(&mut self, path: PathBuf, name: String, cap: Option<u64>) -> bool {
        let source = self.recording.source;
        let rate = self.session.project().sample_rate;
        let input_channel = match source {
            RecordSource::Input => self.recording_input_channel(),
            RecordSource::Master => None,
        };
        let feed: Box<dyn Feed> = match (source, input_channel) {
            (RecordSource::Master, _) => match self.session.take_record_tap() {
                Some(tap) => Box::new(TapFeed { tap, rate }),
                None => {
                    self.notice =
                        Some("Cannot record while the input latency is measured.".to_owned());
                    return false;
                }
            },
            (RecordSource::Input, channel) => match self.take_line_in(channel) {
                Some(feed) => feed,
                None => return false,
            },
        };
        let recorder = match Recorder::start_capped(path.clone(), feed, rate, cap) {
            Ok(recorder) => recorder,
            Err((error, feed)) => {
                match feed {
                    Some(feed) => self.give_back(source, input_channel, feed),
                    // The writer thread never started and took the input channel's record feed
                    // with it: its device opens again, with rings of its own, so no take holds
                    // it for good.
                    None if input_channel.is_some() => {
                        self.live_inputs = crate::live_input::LiveInputs::default();
                        self.sync_live_inputs(true);
                    }
                    None => {}
                }
                self.notice = Some(format!("Cannot record to {}: {error}", path.display()));
                return false;
            }
        };
        let result = self.session.send(Control::Record {
            count_in: self.recording.count_in,
            tap_master: source == RecordSource::Master,
        });
        let started = result.is_ok();
        self.report(result);
        let line_in = (source == RecordSource::Input).then(|| self.line_in_devices());
        self.recording.take = Some(Take {
            recorder,
            start: self.song_marker,
            name,
            source,
            input_channel,
            line_in,
        });
        if !started {
            self.finish_take();
        }
        started
    }

    /// Stop while recording: the take's file is completed and becomes one new audio clip from
    /// where the take started (an empty take, stopped in the count-in, is deleted); a take of the
    /// line in from where it was played, when a latency is kept for its devices
    /// ([`takes::input_take`]), which the toolbar says.
    pub(super) fn finish_take(&mut self) {
        let Some(take) = self.recording.take.take() else {
            return;
        };
        let finished = take.recorder.finish();
        let recorded = match finished {
            Ok(recorded) => recorded,
            Err((error, feed)) => {
                self.give_back(take.source, take.input_channel, feed);
                self.notice = Some(format!("Recording failed: {error}"));
                self.update_input();
                return;
            }
        };
        let Recorded { path, frames, feed } = recorded;
        self.give_back(take.source, take.input_channel, feed);
        self.update_input();
        let sample = self.sample_path(&path);
        let kept = take
            .line_in
            .as_ref()
            .and_then(|(output, input)| self.settings.input_latency(output, input))
            .cloned();
        let project = self.session.project();
        let rate = project.sample_rate;
        let late = kept.as_ref().map_or(0, |kept| kept.frames_at(rate));
        let track = self.recording.track;
        let name = take.name.clone();
        let placed = takes::input_take(project, name, sample, take.start, frames, track, late);
        match placed {
            Some((edit, shift)) => {
                self.commit(edit);
                // The Library lists the take, under Recordings, to drag anywhere.
                self.show_recordings(&path);
                let measured = kept.is_some_and(|kept| kept.measured);
                if let Some(words) = shift_words(&take.name, shift, measured, rate) {
                    self.notice = Some(words);
                }
            }
            None => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    /// The line in's feed, for a take or a latency measurement: input channel `channel`'s record
    /// ring (its device open), else the system's default input's, opened now when it is not
    /// open. `None`, with the reason shown, when there is none to take.
    pub(super) fn take_line_in(&mut self, channel: Option<ChannelId>) -> Option<Box<dyn Feed>> {
        match channel {
            Some(id) => {
                let feed = self.live_inputs.take_record_feed(id);
                if feed.is_none() {
                    self.notice = Some("Recording off: the input channel is recording".to_owned());
                }
                feed
            }
            None => {
                if self.recording.input.is_none() {
                    self.open_default_input();
                }
                self.recording
                    .input
                    .as_mut()
                    .and_then(|input| input.feed.take())
            }
        }
    }

    /// Gives a take's feed back to where it came from: the record tap to the session, an input
    /// channel's record ring to its live input (whose device may then open anew as the document
    /// says, held while the take ran), the default input's ring to the open input.
    pub(super) fn give_back(
        &mut self,
        source: RecordSource,
        input_channel: Option<ChannelId>,
        feed: Box<dyn Feed>,
    ) {
        match (source, input_channel) {
            (RecordSource::Master, _) => {
                if let Some(tap) = feed.into_tap() {
                    self.session.return_record_tap(tap);
                }
            }
            (RecordSource::Input, Some(id)) => {
                self.live_inputs.give_back(id, feed);
                self.sync_live_inputs(false);
            }
            (RecordSource::Input, None) => {
                if let Some(input) = &mut self.recording.input {
                    input.feed = Some(feed);
                }
            }
        }
    }

    /// Where takes go: `samples/` beside the document's file, or the per-user
    /// recordings folder for a document with no file.
    pub(super) fn recordings_dir(&self) -> Option<PathBuf> {
        match &self.document.path {
            Some(path) => Some(path.parent()?.join("samples")),
            None => self.host.recordings.clone(),
        }
    }

    /// Reads the input's peak since the last read into its meter, and says when the device
    /// failed.
    pub(super) fn read_input_meter(&mut self) {
        let Some(input) = &self.recording.input else {
            return;
        };
        let peak = f32::from_bits(input.peak.swap(0, Ordering::Relaxed));
        self.recording.meter.update(&[[peak, peak]], Instant::now());
        if input.stream.failed() {
            self.notice = Some("Recording input stopped: the input device failed".to_owned());
        }
    }

    /// Whether the input meter still moves, so the timer keeps running.
    pub(super) fn input_moving(&self) -> bool {
        self.input_level().is_some()
    }

    /// Capture: the notes played in the last 30 s while no take ran, at the current tempo, the
    /// first on a bar's start. In pattern mode into the selected pattern from the bar start
    /// nearest where the first was heard (its first bar while stopped), the pattern growing by
    /// whole bars to hold them; in song mode into a new pattern of the selected channel's and
    /// its clip from the bar start nearest the playhead (the song marker while stopped), shown
    /// in the pattern box. One undo step; what was captured is not captured again.
    pub(super) fn capture_played(&mut self) {
        if self.recording.recording() {
            self.notice = Some(STOP_RECORDING_FIRST.to_owned());
            return;
        }
        let quantize = self
            .recording
            .quantize
            .then(|| self.roll.snap.unit(&self.roll.view));
        let project = self.session.project();
        let (mut notes, heard_at) = self.capture.notes(
            project.sample_rate,
            project.tempo,
            project.ppq,
            quantize,
            Instant::now(),
        );
        if notes.is_empty() {
            self.notice = Some("Nothing to capture: play something first".to_owned());
            return;
        }
        let bar = dawsome_engine::time::bars_to_ticks(1, project.ppq).0;
        let nearest_bar = |tick: i64| (tick + bar / 2).div_euclid(bar) * bar;
        let built = match self.mode {
            PlayMode::Pattern => {
                let Some((id, pattern)) = self
                    .pattern
                    .and_then(|id| Some((id, project.patterns.get(&id)?)))
                else {
                    return;
                };
                let length =
                    dawsome_engine::time::bars_to_ticks(pattern.length_bars, project.ppq).0;
                // Played along the pattern: from the bar start nearest where the first note was
                // heard in it (a bar start at its end is its first).
                let base = heard_at
                    .map(|tick| nearest_bar(tick.0.rem_euclid(length.max(1))))
                    .filter(|&base| base < length)
                    .unwrap_or(0);
                for note in &mut notes {
                    note.start.0 += base;
                }
                Ok(takes::capture_into(project, id, notes))
            }
            PlayMode::Song => {
                let at = self.playhead.unwrap_or(self.song_marker);
                let start = Ticks(nearest_bar(at.0).max(0));
                let channel = self
                    .selected_channel
                    .and_then(|id| project.channels.get(&id))
                    .map_or("Notes", |channel| channel.name.as_str());
                let name = takes::take_name(project, channel);
                takes::capture_song(project, name, start, self.recording.track, notes)
            }
        };
        let edit = match built {
            Ok(Some(edit)) => edit,
            Ok(None) => return,
            Err(error) => {
                self.after_edit(Err(error.into()));
                return;
            }
        };
        let added = self.session.project().next_pattern_id();
        match self.session.apply_live(edit) {
            Ok(inverse) => {
                self.session.record_as(inverse, "Capture");
                self.capture.clear();
                if self.mode == PlayMode::Song {
                    if let Some(id) =
                        added.filter(|id| self.session.project().patterns.contains_key(id))
                    {
                        self.pattern = Some(id);
                        self.roll.forget_notes();
                    }
                }
                self.after_edit(Ok(()));
                self.clear_panel_caches();
            }
            Err(error) => self.after_edit(Err(error)),
        }
    }

    /// Ends what the old document's recording held, before a new document replaces it: a take
    /// is completed into the old document (its file stays where it was written), and the input
    /// closes, as it reads the old player's clock. [`update_input`](Self::update_input) opens it
    /// again for the new one. Capture forgets what was played: the new player counts its frames
    /// from 0.
    pub(super) fn reset_recording(&mut self) {
        self.cancel_latency();
        self.capture.clear();
        if let Some(tick) = self.note_take_end() {
            self.finish_note_take(tick);
        }
        self.finish_take();
        self.recording.input = None;
        self.recording.track = None;
    }
}

#[cfg(test)]
impl App {
    pub(crate) fn recording_take(&self) -> bool {
        self.recording.recording()
    }

    /// Whether the system's default input device is open for Record → Input.
    pub(crate) fn default_input_open(&self) -> bool {
        self.recording.input.is_some()
    }
}

#[cfg(test)]
mod p17;
#[cfg(test)]
mod p23;

#[cfg(test)]
mod tests {
    use std::path::Path;

    use dawsome_engine::project::{ChannelId, PatternId};
    use dawsome_engine::realtime::Player;

    use super::super::{Message, PatternMessage};
    use super::*;
    use crate::midi::{ControllerQueue, Router};

    const P9_RECORD: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/p9_record.json"
    );

    /// p9_record in the window, Record armed with Notes as the only source and no count-in.
    fn armed_for_notes() -> (App, Player) {
        let (mut app, player) = super::super::tests::app_on(Path::new(P9_RECORD));
        for message in [RecordMessage::ToggleCountIn, RecordMessage::ToggleArm] {
            let _ = app.update(Message::Record(message));
        }
        assert_eq!(app.recording.kind, TakeKind::Notes);
        (app, player)
    }

    // A fresh document records the notes played and no audio: the input stays off (and no
    // input device is opened) until it is chosen in Audio's menu; Audio would record what is
    // heard.
    #[test]
    fn a_fresh_document_arms_notes_only() {
        let (app, _player) = super::super::tests::app_on(Path::new(P9_RECORD));
        assert_eq!(app.recording.kind, TakeKind::Notes, "Notes");
        assert_eq!(app.recording.source, RecordSource::Master);
        assert!(app.recording.input.is_none(), "no input device open");
    }

    // gen_p9_expected.py's known answer (3) through the window: six messages before 512-frame calls
    // 87/120/150/170/171/180 of two bars (playhead ticks 193/267/334/378/381/401), recorded into
    // the looping pattern with the piano roll's snap (a step): 381 → 384 wraps to 0, 193 → 192, 334
    // → 336, lengths as played. The take is one undo step.
    #[test]
    fn a_take_of_notes_from_a_device_lands_quantized_in_one_undo_step() {
        let (mut app, mut player) = armed_for_notes();
        let device = Router::new(app.session.midi_inbox(), ControllerQueue::default());
        let _ = app.update(Message::Play);
        assert!(app.recording.taking_notes());
        assert_eq!(app.mode, PlayMode::Pattern, "notes record the pattern");
        let messages: [(u64, [u8; 3]); 6] = [
            (87, [0x90, 60, 100]),
            (120, [0x80, 60, 0]),
            (150, [0x90, 64, 127]),
            (170, [0x90, 64, 0]),
            (171, [0x90, 67, 64]),
            (180, [0x80, 67, 64]),
        ];
        for call in 0..345 {
            for (_, bytes) in messages.iter().filter(|(at, _)| *at == call) {
                device.deliver(bytes);
            }
            player.process(&mut [[0.0; 2]; 512]);
            let _ = app.update(Message::Tick);
        }
        let _ = app.update(Message::Undo);
        assert_eq!(
            app.notice.as_deref(),
            Some("Stop the recording first"),
            "no undo under a take"
        );
        let _ = app.update(Message::Stop);
        assert!(!app.recording.recording());
        let notes: Vec<(i64, i64, u8, f32, ChannelId)> = app.session.project().patterns
            [&PatternId(1)]
            .notes
            .iter()
            .map(|note| {
                (
                    note.start.0,
                    note.length.0,
                    note.key,
                    note.velocity,
                    note.channel,
                )
            })
            .collect();
        assert_eq!(
            notes,
            [
                (0, 20, 67, 0.5, ChannelId(1)),
                (192, 74, 60, 0.78125, ChannelId(1)),
                (336, 44, 64, 0.992_187_5, ChannelId(1)),
            ]
        );
        assert_eq!(app.session.history().undo_len(), 1);
        let _ = app.update(Message::Undo);
        assert!(app.session.project().patterns[&PatternId(1)]
            .notes
            .is_empty());
    }

    // Recording v2: song mode records notes too (it said "Note recording works in pattern mode"
    // and recorded nothing); a take there with no note leaves nothing. And a key held at Stop
    // ends there.
    #[test]
    fn note_takes_play_both_modes_and_end_held_notes_at_stop() {
        let (mut app, mut player) = armed_for_notes();
        let before = app.session.project().to_json().expect("serializes");
        let _ = app.update(Message::ModeSelected(PlayMode::Song));
        let _ = app.update(Message::Play);
        assert!(app.recording.taking_notes() && app.transport_on);
        assert_eq!(app.notice, None);
        let _ = app.update(Message::Stop);
        assert_eq!(app.session.project().to_json().expect("serializes"), before);
        let _ = app.update(Message::ModeSelected(PlayMode::Pattern));
        let _ = app.update(Message::Record(RecordMessage::ToggleQuantize));
        let _ = app.update(Message::Play);
        let device = Router::new(app.session.midi_inbox(), ControllerQueue::default());
        for call in 0..40 {
            if call == 10 {
                device.deliver(&[0x90, 62, 64]);
            }
            player.process(&mut [[0.0; 2]; 512]);
            let _ = app.update(Message::Tick);
        }
        let _ = app.update(Message::Stop);
        // Heard from call 10 (sample 5120, tick floor(5120 / 229.6875) = 22) to the end of
        // call 39 (sample 20480, tick 89), unquantized.
        let notes: Vec<(i64, i64, u8)> = app.session.project().patterns[&PatternId(1)]
            .notes
            .iter()
            .map(|note| (note.start.0, note.length.0, note.key))
            .collect();
        assert_eq!(notes, [(22, 67, 62)]);
    }

    // Delete is refused under a take of notes, as Undo is, and dim in the pattern menu: the
    // take's undo step, saved at Stop, names the pattern it records into. After Stop the take
    // is one undo step and the new pattern's add another, and both undo.
    #[test]
    fn a_pattern_is_not_deleted_under_a_take_of_notes() {
        let (mut app, mut player) = armed_for_notes();
        let _ = app.update(Message::Pattern(PatternMessage::Next));
        assert_eq!(
            app.pattern,
            Some(PatternId(2)),
            "a new pattern to record into"
        );
        let _ = app.update(Message::Play);
        assert!(app.recording.taking_notes());
        let device = Router::new(app.session.midi_inbox(), ControllerQueue::default());
        for call in 0..40 {
            match call {
                10 => device.deliver(&[0x90, 60, 100]),
                20 => device.deliver(&[0x80, 60, 0]),
                _ => {}
            }
            player.process(&mut [[0.0; 2]; 512]);
            let _ = app.update(Message::Tick);
        }
        let notes = |app: &App| {
            app.session
                .project()
                .patterns
                .get(&PatternId(2))
                .map(|pattern| pattern.notes.len())
        };
        assert_eq!(notes(&app), Some(1), "the note landed");
        assert!(!app.offers_delete(), "Delete is dim under the take");
        let _ = app.update(Message::Pattern(PatternMessage::Delete));
        assert_eq!(notes(&app), Some(1), "the pattern is still there");
        assert_eq!(app.notice.as_deref(), Some("Stop the recording first"));
        let _ = app.update(Message::Stop);
        assert!(app.offers_delete());
        let _ = app.update(Message::Undo);
        assert_eq!(app.error, None);
        assert_eq!(notes(&app), Some(0), "the take undone");
        let _ = app.update(Message::Undo);
        assert_eq!(notes(&app), None, "the pattern's add undone");
        assert_eq!(app.pattern, Some(PatternId(1)));
    }

    // Notes is on by default with the computer keyboard on (as a new
    // window has it), whatever ports are ticked, and the roll's strip lights "Rec notes" while
    // the take runs. Overdub, by hand (229.6875 samples a tick, a bar of 384 ticks is 172.3
    // calls of 512): the first pass holds C5 from call 10 (tick 22) to call 20 (tick 44); the
    // second holds E5 from call 190 (sample 97280, tick 423, 39 into the loop) to call 200
    // (tick 445). Quantized to the step, 22 → 24 and 39 → 48, lengths as played (22 each):
    // both kept, the take one undo step.
    #[test]
    fn a_second_pass_over_the_loop_adds_to_the_first_in_one_undo_step() {
        let (mut app, mut player) = armed_for_notes();
        assert!(app.typing.on && app.recording.kind == TakeKind::Notes);
        let device = Router::new(app.session.midi_inbox(), ControllerQueue::default());
        let _ = app.update(Message::Play);
        assert!(
            app.recording.taking_notes(),
            "the strip's Rec notes light is on"
        );
        for call in 0..210 {
            match call {
                10 => device.deliver(&[0x90, 60, 100]),
                20 => device.deliver(&[0x80, 60, 0]),
                190 => device.deliver(&[0x90, 64, 100]),
                200 => device.deliver(&[0x80, 64, 0]),
                _ => {}
            }
            player.process(&mut [[0.0; 2]; 512]);
            let _ = app.update(Message::Tick);
        }
        let _ = app.update(Message::Stop);
        assert!(!app.recording.taking_notes());
        let notes: Vec<(i64, i64, u8)> = app.session.project().patterns[&PatternId(1)]
            .notes
            .iter()
            .map(|note| (note.start.0, note.length.0, note.key))
            .collect();
        assert_eq!(notes, [(24, 22, 60), (48, 22, 64)]);
        assert_eq!(app.session.history().undo_len(), 1);
        let _ = app.update(Message::Undo);
        assert!(app.session.project().patterns[&PatternId(1)]
            .notes
            .is_empty());
    }

    // A key let go of just before Stop. Its end, read
    // at Stop for the take, now reaches the roll's live set too: none lit, the timer done,
    // the note recorded once.
    #[test]
    fn a_key_let_go_just_before_stop_is_not_left_lit() {
        let (mut app, mut player) = armed_for_notes();
        let _ = app.update(Message::TogglePanel(crate::ui::panels::Panel::PianoRoll));
        let device = Router::new(app.session.midi_inbox(), ControllerQueue::default());
        let _ = app.update(Message::Play);
        let mut call = |app: &mut App, tick: bool| {
            player.process(&mut [[0.0; 2]; 512]);
            if tick {
                let _ = app.update(Message::Tick);
            }
        };
        for _ in 0..20 {
            call(&mut app, true);
        }
        device.deliver(&[0x90, 60, 100]);
        call(&mut app, true);
        assert_eq!(
            app.live_notes.of(ChannelId(1)).len(),
            1,
            "held before the release"
        );
        device.deliver(&[0x80, 60, 0]);
        call(&mut app, false);
        let _ = app.update(Message::Stop);
        for _ in 0..5 {
            call(&mut app, true);
        }
        assert!(app.live_notes.of(ChannelId(1)).is_empty(), "60 let go of");
        assert!(!app.live_notes_due(), "the timer stops");
        assert_eq!(app.session.project().patterns[&PatternId(1)].notes.len(), 1);
        assert_eq!(app.session.history().undo_len(), 1);
    }

    /// A take of notes running on `app`: C5 held from call 10 to call 20, heard and written.
    fn a_take_with_one_note(app: &mut App, player: &mut Player) {
        let device = Router::new(app.session.midi_inbox(), ControllerQueue::default());
        let _ = app.update(Message::Play);
        assert!(app.recording.taking_notes());
        for call in 0..30 {
            match call {
                10 => device.deliver(&[0x90, 60, 100]),
                20 => device.deliver(&[0x80, 60, 0]),
                _ => {}
            }
            player.process(&mut [[0.0; 2]; 512]);
            let _ = app.update(Message::Tick);
        }
    }

    fn notes_in(app: &App, pattern: u32) -> usize {
        app.session.project().patterns[&PatternId(pattern)]
            .notes
            .len()
    }

    // Recording v2, a take kept before Stop: Record disarmed keeps the take as Stop would (its
    // note, one undo step) and says so; the song plays on, recording no more.
    #[test]
    fn disarming_keeps_the_take_and_the_song_plays_on() {
        let (mut app, mut player) = armed_for_notes();
        a_take_with_one_note(&mut app, &mut player);
        let _ = app.update(Message::Record(RecordMessage::ToggleArm));
        assert!(!app.recording.recording() && !app.recording.armed);
        assert_eq!(app.notice.as_deref(), Some("Take kept: Record is off"));
        assert_eq!(app.session.history().undo_len(), 1);
        assert_eq!(notes_in(&app, 1), 1);
        assert!(app.session.controls_sent().contains(&Control::EndRecording));
        player.process(&mut [[0.0; 2]; 512]);
        let _ = app.update(Message::Tick);
        assert!(app.transport_on && app.playing, "the song plays on");
    }

    // Another pattern chosen in pattern mode, or the play mode switched, keeps the take: its
    // note stays in pattern 1 and the take is behind whatever comes after it.
    #[test]
    fn another_pattern_or_the_mode_keeps_the_take() {
        for message in [
            Message::Pattern(PatternMessage::Next),
            Message::ModeSelected(PlayMode::Song),
        ] {
            let (mut app, mut player) = armed_for_notes();
            a_take_with_one_note(&mut app, &mut player);
            let _ = app.update(message);
            assert!(!app.recording.recording());
            assert!(
                app.notice
                    .as_deref()
                    .is_some_and(|notice| notice.starts_with("Take kept")),
                "{:?}",
                app.notice
            );
            assert_eq!(notes_in(&app, 1), 1);
            assert!(app.session.history().undo_len() >= 1);
        }
    }

    // Saving keeps the take first: the file holds its note and the document is clean.
    #[test]
    fn saving_keeps_the_take_and_the_file_holds_it() {
        let dir = std::env::temp_dir().join(format!("dawsome-keep-on-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        for name in ["p9_record.json", "impulse.wav"] {
            let from = Path::new(P9_RECORD).with_file_name(name);
            std::fs::copy(from, dir.join(name)).expect("copies");
        }
        let path = dir.join("p9_record.json");
        let (mut app, mut player) = super::super::tests::app_on(&path);
        for message in [RecordMessage::ToggleCountIn, RecordMessage::ToggleArm] {
            let _ = app.update(Message::Record(message));
        }
        a_take_with_one_note(&mut app, &mut player);
        let _ = app.update(Message::File(super::super::FileMessage::Save));
        assert!(!app.recording.recording());
        assert!(!app.session.is_dirty());
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("saved")).expect("parses");
        assert_eq!(
            saved["patterns"][0]["notes"].as_array().map(Vec::len),
            Some(1)
        );
    }

    // While a song take runs, the edits its undo step could not take back past are refused with
    // "Stop the recording first": a new pattern, the take's clip deleted, a note drawn into its
    // pattern; another channel's volume goes through. After Stop they all do again.
    #[test]
    fn a_song_take_keeps_its_pattern_and_clip_from_other_edits() {
        let (mut app, mut player) = armed_for_notes();
        let _ = app.update(Message::ModeSelected(PlayMode::Song));
        let _ = app.update(Message::Play);
        for _ in 0..4 {
            player.process(&mut [[0.0; 2]; 512]);
            let _ = app.update(Message::Tick);
        }
        assert!(app.recording.taking_notes());
        let take = app.pattern.expect("the take's pattern");
        let clip = *app
            .session
            .project()
            .playlist
            .clips
            .keys()
            .last()
            .expect("the take's clip");
        let refused = Some("Stop the recording first");
        let _ = app.update(Message::Pattern(PatternMessage::Next));
        assert_eq!(app.error.as_deref(), refused, "a new pattern");
        app.commit(dawsome_engine::edit::Edit::DeleteClips { ids: vec![clip] });
        assert_eq!(app.error.as_deref(), refused, "the take's clip");
        let note = dawsome_engine::project::Note {
            channel: ChannelId(1),
            start: dawsome_engine::time::Ticks(0),
            length: dawsome_engine::time::Ticks(24),
            velocity: 0.5,
            key: 60,
        };
        app.commit(dawsome_engine::edit::Edit::ReplaceNotes {
            pattern: take,
            remove: Vec::new(),
            add: vec![note],
        });
        assert_eq!(
            app.error.as_deref(),
            refused,
            "a note in the take's pattern"
        );
        app.commit(dawsome_engine::edit::Edit::SetChannelVolume {
            channel: ChannelId(1),
            value: 0.5,
        });
        assert_eq!(app.error, None, "another change goes through");
        // A clip unlinked or linked back (clip link) is refused before it reaches the session.
        app.notice = None;
        let before = app.session.project().clone();
        let _ = app.update(Message::ClipLink(super::super::ClipLink::Unlink(clip)));
        assert_eq!(app.notice.as_deref(), refused, "the take's clip unlinked");
        assert_eq!(app.session.project(), &before);
        let _ = app.update(Message::Stop);
        let _ = app.update(Message::Pattern(PatternMessage::Next));
        assert_eq!(app.error, None, "after Stop");
        assert_eq!(app.session.project().patterns.len(), 2);
    }

    const P17_CAPTURE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/p17_capture.json"
    );

    fn ctrl_shift_r(app: &mut App) {
        let _ = app.update(Message::Record(RecordMessage::Capture));
    }

    // Capture with nothing played says so and changes nothing; during a take it is refused; the
    // notes a take heard are not Capture's.
    #[test]
    fn capture_needs_notes_played_outside_takes() {
        let (mut app, mut player) = armed_for_notes();
        ctrl_shift_r(&mut app);
        assert_eq!(
            app.notice.as_deref(),
            Some("Nothing to capture: play something first")
        );
        a_take_with_one_note(&mut app, &mut player);
        ctrl_shift_r(&mut app);
        assert_eq!(app.notice.as_deref(), Some("Stop the recording first"));
        let _ = app.update(Message::Stop);
        ctrl_shift_r(&mut app);
        assert_eq!(
            app.notice.as_deref(),
            Some("Nothing to capture: play something first"),
            "the take's note is the take's"
        );
        assert_eq!(app.session.history().undo_len(), 1, "the take alone");
    }

    // By hand: p17_capture's pattern made 4 bars (1536 ticks) and played (not recorded). C5 heard
    // at call 400 (sample 204800, tick 891 of the pattern) and let go of at call 420 (10240
    // frames later: 44 ticks). Capture lays it from the bar start nearest where it was heard
    // (891 → 768), 44 ticks long, in one undo step; a second Capture has nothing left.
    #[test]
    fn capture_played_along_the_pattern_starts_on_the_nearest_bar() {
        let (mut app, mut player) = super::super::tests::app_on(Path::new(P17_CAPTURE));
        app.commit(dawsome_engine::edit::Edit::SetPatternBars {
            pattern: PatternId(1),
            bars: 4,
        });
        let device = Router::new(app.session.midi_inbox(), ControllerQueue::default());
        let _ = app.update(Message::Play);
        for call in 0..430 {
            match call {
                400 => device.deliver(&[0x90, 60, 100]),
                420 => device.deliver(&[0x80, 60, 0]),
                _ => {}
            }
            player.process(&mut [[0.0; 2]; 512]);
            let _ = app.update(Message::Tick);
        }
        let _ = app.update(Message::Stop);
        ctrl_shift_r(&mut app);
        let notes: Vec<(i64, i64, u8)> = app.session.project().patterns[&PatternId(1)]
            .notes
            .iter()
            .map(|note| (note.start.0, note.length.0, note.key))
            .collect();
        assert_eq!(notes, [(768, 44, 60)]);
        assert_eq!(
            app.session.history().undo_len(),
            2,
            "the bars, then Capture"
        );
        assert_eq!(app.session.history().undo_name(), Some("Capture"));
        ctrl_shift_r(&mut app);
        assert_eq!(
            app.notice.as_deref(),
            Some("Nothing to capture: play something first")
        );
    }

    // Recording v2: the notices' line says what a take records and for how long, counted on
    // the song: "Recording notes 0:00" as it starts (p9_record, 44.1 kHz, 120 BPM), "0:04"
    // after 345 calls of 512 (176640 frames: tick 769, whose sample is 176631, 4 s), nothing
    // once it stops.
    #[test]
    fn the_notices_say_a_take_runs_and_for_how_long() {
        let (mut app, mut player) = armed_for_notes();
        assert_eq!(app.recording_words(), None);
        let _ = app.update(Message::Play);
        let notices = app.toolbar().notice_text().unwrap_or_default();
        assert!(notices.starts_with("Recording notes 0:00"), "{notices}");
        for _ in 0..345 {
            player.process(&mut [[0.0; 2]; 512]);
            let _ = app.update(Message::Tick);
        }
        assert_eq!(
            app.recording_words().as_deref(),
            Some("Recording notes 0:04")
        );
        let _ = app.update(Message::Stop);
        assert_eq!(app.recording_words(), None);
    }

    // The switch beside Record: a press on a side picks what the next take records, a right
    // press opens that side's menu (again: closes it); a right press on Record opens the menu
    // of what it records now; an Audio menu row picks audio too.
    #[test]
    fn the_switch_and_its_menus() {
        let (mut app, _player) = super::super::tests::app_on(Path::new(P9_RECORD));
        let record = |app: &mut App, message| {
            let _ = app.update(Message::Record(message));
        };
        record(&mut app, RecordMessage::Kind(TakeKind::Audio));
        assert_eq!(app.recording.kind, TakeKind::Audio);
        record(&mut app, RecordMessage::ToggleMenu);
        assert_eq!(app.recording.menu, Some(TakeKind::Audio));
        record(&mut app, RecordMessage::Menu(TakeKind::Notes));
        assert_eq!(app.recording.menu, Some(TakeKind::Notes));
        record(&mut app, RecordMessage::Menu(TakeKind::Notes));
        assert_eq!(app.recording.menu, None);
        record(&mut app, RecordMessage::Kind(TakeKind::Notes));
        record(&mut app, RecordMessage::Source(RecordSource::Master));
        assert_eq!(
            (app.recording.kind, app.recording.source, app.recording.menu),
            (TakeKind::Audio, RecordSource::Master, None)
        );
    }
}
