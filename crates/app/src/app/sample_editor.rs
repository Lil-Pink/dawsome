//! The sample editor's life in the app: which channel's sound it shows, the sound itself (its
//! view buffer, [`EditorSound`]), the selection, the cursor and the view of the waveform, the
//! drag under way and Play. Every op is one undo step of the document's own: the channel's
//! edit list one op longer ([`sound_edits::append`]: a crop or a delete moves the channel's
//! clips by its frame map in the same step), committed as any edit is, so Undo, Redo and the
//! song playing on all see it. After every edit the sound shown follows the channel's list
//! ([`App::keep_sample_editor`]): one op more is applied where the frames are, anything else
//! makes the sound again from the file (on a worker while the window runs, the old one shown
//! and the ops dim meanwhile).
//!
//! It is a window of its own beside the main one, as the parameter window is: one at a time
//! (opening it on another channel shows that channel's sound in the same window), opened and
//! closed to match its state after every message ([`App::sync_sample_editor_window`]), above
//! the main window while the app has the keyboard, its size and place kept in the settings
//! (`sample_editor_window`). Esc in it clears the selection, then closes it; its Close, its X,
//! its channel gone and another document close it too. Keys typed in it do what they do in the
//! main window, but for its own: Delete deletes the selection, Ctrl+A selects the whole sound,
//! Space plays the selection ([`crate::keys::Scope::SampleEditor`]).

use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::{sound_edits, EditError};
use dawsome_engine::project::{ChannelId, ClipId, ClipItem, SoundEdits, SoundOp, EDIT_GAIN_RANGE};
use dawsome_engine::source::MemorySource;
use dawsome_engine::time::{self, Ticks};
use iced::widget::{container, space};
use iced::{window, Element, Fill, Size, Task};

use super::editor_sound::{part_of, EditorSound};
use super::files::{blocking, joinable};
use super::instrument::pop_out_level;
use super::{App, Message, WindowChange};
use crate::args::EditorOn;
use crate::desktop::{self, PopOut};
use crate::settings::WindowPlace;
use crate::ui::hint::{self, Hover};
use crate::ui::sample_editor::{
    self as ui, Caches, EditorOp, Part, SampleEditorMessage, SaveDone, Wave,
};
use crate::ui::sample_view::{self, Selection, View};
use crate::ui::scroll::{self, BarMessage};
use crate::ui::style;

/// A pixel of the Gain readout's drag, in hundredths of a dB: up louder, down quieter.
pub const GAIN_PER_PIXEL: f32 = 10.0;

/// Play copies a part longer than this on a worker (a whole ten-minute sound is a 230 MB copy,
/// twice over); a shorter one (under 4 MB of stereo) is copied at once.
pub const COPY_INLINE_SECONDS: u64 = 10;

/// A temporary file of Save as new file… left unwritten this long is one a write cut short
/// left (the app killed under it): a write under way writes to it every few milliseconds.
pub const STALE_PARTIAL: Duration = Duration::from_secs(60);

/// What the window's foot line says while nothing under the cursor has words of its own.
pub const GESTURES: &str =
    "Drag to select \u{b7} click: Play starts there \u{b7} Ctrl+wheel zooms \u{b7} Esc closes";

/// The window's state while it is open.
#[derive(Debug)]
pub(super) struct SampleEditor {
    pub channel: ChannelId,
    /// The audio clip it was opened from (a double press on its body): its window is
    /// bracketed on the ruler while the clip plays the channel's sound.
    pub clip: Option<ClipId>,
    /// The sound shown, once made: the channel's as it is now, or (while the window's
    /// `building` is another) as it was.
    pub sound: Option<EditorSound>,
    pub selection: Option<Selection>,
    /// Where a click put Play's start.
    pub cursor: u64,
    pub view: View,
    press: Option<Press>,
    /// The last click let go of where it went down: when and where, for a double-click.
    click: Option<(Instant, f32)>,
    /// The Gain readout's drag: the window y of its first move, once there is one.
    gain_from: Option<f32>,
    gain_held: bool,
    /// The gain the readout's drag has set, in hundredths of a dB.
    pub gain: i32,
    audition: Option<Audition>,
    /// A save or Play's copy under way on a worker: the ops, Revert, Play, Save, Undo and Redo
    /// are dim meanwhile.
    pub busy: Option<Busy>,
    /// A selection asked for before the sound was made (`--sample-editor-select`), shown fitted
    /// once it is.
    pending: Option<(u64, u64)>,
}

/// A press held on the waveform.
#[derive(Debug, Clone, Copy)]
struct Press {
    /// The selection's edge it holds still: where it went down (snapped), or with Shift the
    /// selection's far edge.
    anchor: u64,
    x: f32,
    moved: bool,
    shift: bool,
}

/// Work of the editor's on a worker, which keeps its controls dim until it ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Busy {
    /// Save as new file… writes.
    Saving,
    /// Play's copy of `frames` frames from `from`, the copy `token` names, is made.
    Copying { token: u64, from: u64, frames: u64 },
}

/// Save as new file…'s write, handed to a worker: the sound's frames as they were when it
/// began, and where they go.
pub(super) struct SaveJob {
    samples: Arc<Vec<f32>>,
    sides: u8,
    rate: u32,
    target: PathBuf,
    dir: PathBuf,
}

impl SaveJob {
    /// Writes the frames as a 32-bit float WAV at the project's rate.
    pub(super) fn run(self) -> SaveDone {
        let result =
            dawsome_media::write_float_frames(&self.samples, self.sides, self.rate, &self.target)
                .map_err(|error| format!("cannot write {}: {error}", self.target.display()));
        SaveDone {
            target: self.target,
            dir: self.dir,
            result,
        }
    }
}

/// What the editor's sound is made from: the folder the channel's file is read beside, the
/// file, the project's rate and the channel's list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Build {
    dir: PathBuf,
    file: PathBuf,
    rate: u32,
    edits: SoundEdits,
}

/// A sound made on a worker, and what it was made from.
type Made = (Build, EditorSound);

/// Play of a part of the sound: from where, how many frames, since when.
#[derive(Debug, Clone, Copy)]
struct Audition {
    from: u64,
    frames: u64,
    started: Instant,
}

/// The window's own window, its place and size, and what made sounds come back on.
pub(super) struct EditorWindow {
    pub window: Option<window::Id>,
    pub place: WindowPlace,
    pub size: Option<Size>,
    above: bool,
    /// The sound being made on a worker, the editor open or not. Why here, not on the editor:
    /// a sound that lands after the editor closed must still be taken off `built` (a ten-minute
    /// one is 232 MB), and an editor opened again meanwhile waits for it instead of making a
    /// second.
    building: Option<Build>,
    built: (mpsc::Sender<Made>, mpsc::Receiver<Made>),
    pub caches: Caches,
    /// The last copy for Play handed to a worker, by number: a copy that lands after another
    /// Play (or a Stop) is let go of.
    copies: u64,
    /// Save as new file…'s writes on workers, which the app's quit waits for (the editor
    /// open or not): a write cut short leaves its temporary file in the samples folder.
    saves: Vec<JoinHandle<()>>,
}

impl EditorWindow {
    /// Closed, opening at `place` (its first place when none is kept).
    pub fn new(place: Option<WindowPlace>) -> Self {
        Self {
            window: None,
            place: place.unwrap_or_else(|| WindowPlace::first(PopOut::SampleEditor)),
            size: None,
            above: false,
            building: None,
            built: mpsc::channel(),
            caches: Caches::default(),
            copies: 0,
            saves: Vec::new(),
        }
    }
}

impl App {
    /// Opens the editor on `channel`'s sound (a second opening on another channel shows that
    /// one in the same window); a channel that plays no file of its own says so and opens
    /// nothing. Opened so (the rack's menu, the instrument window), it brackets no clip.
    pub(super) fn open_sample_editor(&mut self, channel: ChannelId) {
        if let Some(open) = self
            .sample_editor
            .as_mut()
            .filter(|open| open.channel == channel)
        {
            open.clip = None;
            return;
        }
        let Some(shown) = self.session.project().channels.get(&channel) else {
            return;
        };
        if !shown.plays_a_file() || shown.sample.as_os_str().is_empty() {
            self.notice = Some(format!("{} plays no sound file to edit", shown.name));
            return;
        }
        self.stop_audition();
        let width = ui::waveform_width(self.sample_editor_size().width);
        self.sample_editor = Some(SampleEditor {
            channel,
            clip: None,
            sound: None,
            selection: None,
            cursor: 0,
            view: View::fit(0, width),
            press: None,
            click: None,
            gain_from: None,
            gain_held: false,
            gain: 0,
            audition: None,
            busy: None,
            pending: None,
        });
        self.keep_sample_editor();
    }

    /// A double press on audio clip `clip`'s body: the editor on its channel's sound, the
    /// clip's window bracketed on the ruler and shown across the window (once the sound is
    /// made, when it is made in the background).
    pub(super) fn open_sample_editor_on_clip(&mut self, clip: ClipId) {
        let Some(channel) = self
            .session
            .project()
            .playlist
            .clips
            .get(&clip)
            .and_then(|shown| shown.audio_channel())
        else {
            return;
        };
        self.open_sample_editor(channel);
        let Some(open) = self
            .sample_editor
            .as_mut()
            .filter(|open| open.channel == channel)
        else {
            return;
        };
        open.clip = Some(clip);
        self.show_clip_window();
    }

    /// The window of the clip the editor was opened from, in frames of the sound shown, while
    /// that clip plays the editor's channel and reaches into the sound.
    pub(super) fn clip_window(&self) -> Option<Selection> {
        let open = self.sample_editor.as_ref()?;
        let frames = open.sound.as_ref()?.frames();
        let project = self.session.project();
        let clip = project.playlist.clips.get(&open.clip?)?;
        let ClipItem::Audio {
            channel,
            frame_offset,
            stretch,
            ..
        } = clip.item
        else {
            return None;
        };
        if channel != open.channel {
            return None;
        }
        let sample = |tick: Ticks| {
            time::tick_to_sample(
                Ticks(tick.0.max(0)),
                project.sample_rate,
                project.tempo,
                project.ppq,
            )
            .unwrap_or(0)
        };
        let played = sample(clip.end()).saturating_sub(sample(clip.start));
        let stretched = stretch.map(|stretch| stretch.frames);
        sample_view::clip_window(frame_offset, played, stretched, frames)
    }

    /// The bracketed clip's window across the editor's window, when there is one.
    fn show_clip_window(&mut self) {
        if let Some(part) = self.clip_window() {
            self.with_editor(|open| open.view = open.view.showing(part.from, part.to));
        }
    }

    /// Closes the editor: Play stops.
    pub(super) fn close_sample_editor(&mut self) -> bool {
        self.stop_audition();
        self.sample_editor.take().is_some()
    }

    /// Esc in the editor: a drag under way ends unapplied, then the selection goes, then the
    /// window.
    fn escape_sample_editor(&mut self) {
        let Some(open) = &mut self.sample_editor else {
            return;
        };
        if open.press.take().is_some() || open.gain_held {
            open.gain_held = false;
            open.gain_from = None;
            open.gain = 0;
        } else if open.selection.take().is_none() {
            self.close_sample_editor();
        }
    }

    /// The channel the editor is open on.
    pub(super) fn sample_editor_on(&self) -> Option<ChannelId> {
        self.sample_editor.as_ref().map(|open| open.channel)
    }

    /// The sound the editor shows follows its channel after every edit: closed when the channel
    /// went or plays no file now; one op more applied where the frames are; anything else (an
    /// undo, a revert, another file) made again from the file.
    pub(super) fn keep_sample_editor(&mut self) {
        let Some(channel) = self.sample_editor_on() else {
            return;
        };
        let Some(shown) = self.session.project().channels.get(&channel) else {
            self.close_sample_editor();
            return;
        };
        if !shown.plays_a_file() || shown.sample.as_os_str().is_empty() {
            self.close_sample_editor();
            return;
        }
        let (file, edits) = (shown.sample.clone(), shown.edits.clone());
        let Some(open) = &mut self.sample_editor else {
            return;
        };
        match &mut open.sound {
            Some(sound) if sound.holds(&file, &edits) => return,
            Some(sound) => {
                if let Some(&op) = sound.one_more(&file, &edits) {
                    sound.append(op);
                    let frames = sound.frames();
                    open.view = open.view.with_frames(frames);
                    open.selection = open.selection.and_then(|part| part.within(frames));
                    open.cursor = open.cursor.min(frames);
                    return;
                }
            }
            None => {}
        }
        let Some(build) = self.editor_build() else {
            return;
        };
        if self.editor.building.as_ref() == Some(&build) {
            return;
        }
        self.build_editor_sound(build);
    }

    /// What the open editor's sound is made from now: its channel's file and list beside the
    /// document's samples, at the project's rate.
    fn editor_build(&self) -> Option<Build> {
        let open = self.sample_editor.as_ref()?;
        let project = self.session.project();
        let channel = project.channels.get(&open.channel)?;
        Some(Build {
            dir: self.session.samples_dir().to_path_buf(),
            file: channel.sample.clone(),
            rate: project.sample_rate,
            edits: channel.edits.clone(),
        })
    }

    /// Makes the editor's sound from `build`: on a worker while the window runs (the sound
    /// shown until it lands), else at once.
    fn build_editor_sound(&mut self, build: Build) {
        if !self.host.background {
            let sound = EditorSound::build(&build.dir, &build.file, build.rate, &build.edits);
            self.install_editor_sound(&build, sound);
            return;
        }
        // One at a time, the editor open or not: a sound asked for while another is made is
        // made once that one lands.
        if self.editor.building.is_some() {
            return;
        }
        let done = self.editor.built.0.clone();
        let asked = build.clone();
        let spawned = std::thread::Builder::new()
            .name("dawsome-sample-editor".into())
            .spawn(move || {
                let sound = EditorSound::build(&asked.dir, &asked.file, asked.rate, &asked.edits);
                let _ = done.send((asked, sound));
            });
        if spawned.is_ok() {
            self.editor.building = Some(build);
        }
    }

    /// A sound made from `made`: shown when the editor is open on that sound as it is now (the
    /// view fitted to it the first time); else let go of, and while the editor is open its
    /// channel's made in its place.
    fn install_editor_sound(&mut self, made: &Build, sound: EditorSound) {
        if self.sample_editor.is_none() {
            return;
        }
        if self.editor_build().as_ref() != Some(made) {
            self.keep_sample_editor();
            return;
        }
        let Some(open) = &mut self.sample_editor else {
            return;
        };
        let frames = sound.frames();
        // Made on the editor's timer, whose messages leave the drawings as they are.
        self.editor.caches.wave.clear();
        let first = open.sound.is_none();
        open.view = match open.sound {
            Some(_) => open.view.with_frames(frames),
            None => View::fit(frames, open.view.width),
        };
        open.selection = open.selection.and_then(|part| part.within(frames));
        open.cursor = open.cursor.min(frames);
        open.sound = Some(sound);
        if let Some((from, to)) = open.pending.take() {
            open.selection = Selection::between(from, to).and_then(|part| part.within(frames));
            // Opened from a clip, the clip's window is what shows.
            if let (Some(part), None) = (open.selection, open.clip) {
                open.view = open.view.showing(part.from, part.to);
            }
        }
        if first {
            self.show_clip_window();
        }
    }

    /// Whether the editor shows its channel's sound as it is now: what its ops need.
    fn editor_ready(&self) -> bool {
        let Some(open) = &self.sample_editor else {
            return false;
        };
        let Some(channel) = self.session.project().channels.get(&open.channel) else {
            return false;
        };
        open.busy.is_none()
            && open.sound.as_ref().is_some_and(|sound| {
                sound.holds(&channel.sample, &channel.edits) && sound.problem.is_none()
            })
    }

    /// Whether the file the editor's sound plays is not the one its channel's edits were made
    /// on: decoded now, another length than the list's first op found, or the project at
    /// another rate (the ops still apply, by frame).
    pub(super) fn editor_file_changed(&self) -> bool {
        let Some(open) = &self.sample_editor else {
            return false;
        };
        let project = self.session.project();
        let (Some(sound), Some(channel)) = (&open.sound, project.channels.get(&open.channel))
        else {
            return false;
        };
        sound.problem.is_none()
            && sound.file == channel.sample
            && channel
                .edits
                .file_changed(sound.file_frames, project.sample_rate)
    }

    /// Something the editor reported.
    pub(super) fn sample_editor_message(&mut self, message: SampleEditorMessage) -> Task<Message> {
        // A save ends, and a sound made on a worker is taken, whether the window is still open
        // or not.
        if let SampleEditorMessage::Saved(done) = message {
            self.saved_sound(done);
            return Task::none();
        }
        if let SampleEditorMessage::Tick = message {
            self.editor_tick();
            return Task::none();
        }
        if self.sample_editor.is_none() {
            return Task::none();
        }
        match message {
            SampleEditorMessage::Pressed(x) => self.editor_pressed(x),
            SampleEditorMessage::Dragged(x) => self.editor_dragged(x),
            SampleEditorMessage::Released(x) => self.editor_released(x),
            SampleEditorMessage::Wheel { x, across, up } => self.editor_wheel(x, across, up),
            SampleEditorMessage::Bar(BarMessage::To { offset, .. }) => {
                self.with_editor(|open| open.view = open.view.scrolled_to(offset));
            }
            SampleEditorMessage::Bar(BarMessage::LetGo) => {}
            SampleEditorMessage::Play => return self.play_audition(),
            SampleEditorMessage::Stop => self.stop_audition(),
            SampleEditorMessage::PlayStop => return self.toggle_audition(),
            SampleEditorMessage::SelectAll => self.select_whole_sound(),
            SampleEditorMessage::Fit => self.with_editor(|open| {
                open.view = View::fit(open.view.frames, open.view.width);
            }),
            SampleEditorMessage::ZoomToSelection => self.with_editor(|open| {
                if let Some(part) = open.selection {
                    open.view = open.view.showing(part.from, part.to);
                }
            }),
            SampleEditorMessage::Snap => {
                let on = !self.settings.snap_to_zero();
                self.settings.set_snap_to_zero(on);
                self.save_settings();
            }
            SampleEditorMessage::Op(op) => self.run_editor_op(op),
            SampleEditorMessage::GainPressed => self.with_editor(|open| {
                open.gain_held = true;
                open.gain_from = None;
                open.gain = 0;
            }),
            SampleEditorMessage::GainMoved(y) => self.with_editor(|open| {
                if !open.gain_held {
                    return;
                }
                let from = *open.gain_from.get_or_insert(y);
                let gain = ((from - y) * GAIN_PER_PIXEL).round() as i32;
                open.gain = gain.clamp(*EDIT_GAIN_RANGE.start(), *EDIT_GAIN_RANGE.end());
            }),
            SampleEditorMessage::GainReleased => {
                let gain = self.sample_editor.as_mut().and_then(|open| {
                    let held = std::mem::take(&mut open.gain_held);
                    open.gain_from = None;
                    let gain = std::mem::take(&mut open.gain);
                    (held && gain != 0).then_some(gain)
                });
                if let Some(centi_db) = gain {
                    self.apply_editor_op(|from, to| SoundOp::Gain { from, to, centi_db });
                }
            }
            SampleEditorMessage::Revert => self.revert_sound(),
            SampleEditorMessage::SaveAs => return self.save_edited_sound(),
            SampleEditorMessage::Close => {
                self.close_sample_editor();
            }
            SampleEditorMessage::Select { from, to } => self.select_in_editor(from, to),
            SampleEditorMessage::Tick | SampleEditorMessage::Saved(_) => {}
            SampleEditorMessage::Copied { token, part } => self.copied_for_play(token, part),
        }
        Task::none()
    }

    /// Runs `change` on the open editor.
    fn with_editor(&mut self, change: impl FnOnce(&mut SampleEditor)) {
        if let Some(open) = &mut self.sample_editor {
            change(open);
        }
    }

    /// Selects frames `[from, to)` of the sound, or, before it is made, once it is (fitted to
    /// the part then).
    fn select_in_editor(&mut self, from: u64, to: u64) {
        self.with_editor(|open| match &open.sound {
            Some(sound) => {
                open.selection =
                    Selection::between(from, to).and_then(|part| part.within(sound.frames()));
            }
            None => open.pending = Some((from, to)),
        });
    }

    /// Selects the whole sound: Ctrl+A, a double-click.
    fn select_whole_sound(&mut self) {
        self.with_editor(|open| {
            let frames = open.sound.as_ref().map_or(0, EditorSound::frames);
            open.selection = Selection::between(0, frames);
        });
    }

    /// The edge under `x`, moved to the nearest zero crossing in reach while Snap to zero is on.
    fn editor_edge(&self, x: f32) -> Option<u64> {
        let open = self.sample_editor.as_ref()?;
        let edge = open.view.edge_at(x);
        let sound = open.sound.as_ref()?;
        if !self.settings.snap_to_zero() {
            return Some(edge);
        }
        let reach = sample_view::snap_reach(self.session.project().sample_rate);
        Some(sound.snap(edge, reach))
    }

    fn editor_pressed(&mut self, x: f32) {
        let now = Instant::now();
        let shift = self.modifiers.shift();
        let Some(edge) = self.editor_edge(x) else {
            return;
        };
        let Some(open) = &mut self.sample_editor else {
            return;
        };
        let double = !shift
            && open.click.is_some_and(|(when, at)| {
                now.duration_since(when) < sample_view::DOUBLE_CLICK
                    && (at - x).abs() <= sample_view::CLICK_TRAVEL
            });
        if double {
            open.click = None;
            open.press = None;
            self.select_whole_sound();
            return;
        }
        let anchor = match (shift, open.selection) {
            // Shift: the selection stretches to the press from its edge further from it.
            (true, Some(part)) if edge.abs_diff(part.from) > edge.abs_diff(part.to) => part.from,
            (true, Some(part)) => part.to,
            (true, None) => open.cursor,
            (false, _) => edge,
        };
        open.press = Some(Press {
            anchor,
            x,
            moved: false,
            shift,
        });
        if shift {
            open.selection = Selection::between(anchor, edge);
        }
    }

    fn editor_dragged(&mut self, x: f32) {
        let edge = self.editor_edge(x);
        let Some(open) = &mut self.sample_editor else {
            return;
        };
        let Some(press) = &mut open.press else {
            return;
        };
        press.moved |= (x - press.x).abs() > sample_view::CLICK_TRAVEL;
        if press.moved || press.shift {
            if let Some(edge) = edge {
                open.selection = Selection::between(press.anchor, edge);
            }
        }
    }

    fn editor_released(&mut self, x: f32) {
        self.editor_dragged(x);
        let Some(open) = &mut self.sample_editor else {
            return;
        };
        let Some(press) = open.press.take() else {
            return;
        };
        if !press.moved && !press.shift {
            // A click: Play starts here, and nothing is selected.
            open.cursor = open.view.edge_at(x);
            open.selection = None;
            open.click = Some((Instant::now(), x));
        } else {
            open.click = None;
        }
    }

    /// The wheel over the waveform: Ctrl zooms around the pointer; the wheel, Shift+wheel and a
    /// wheel across scroll, as the playlist's time does. One step per event, whatever its size.
    fn editor_wheel(&mut self, x: f32, across: f32, up: f32) {
        let command = self.modifiers.command();
        self.with_editor(|open| {
            let (sideways, notch) = scroll::wheel(across, up);
            if notch == 0.0 {
                return;
            }
            open.view = if command && !sideways {
                open.view.zoomed(x, notch > 0.0)
            } else {
                // Down, or right across, is later.
                open.view.wheeled(notch < 0.0)
            };
        });
    }

    /// The frames an op acts on: the selection, else the whole sound; none while the sound
    /// shown is not the channel's as it is now.
    fn op_range(&self) -> Option<(u64, u64, u64)> {
        if !self.editor_ready() {
            return None;
        }
        let open = self.sample_editor.as_ref()?;
        let frames = open.sound.as_ref()?.frames();
        let (from, to) = open
            .selection
            .map_or((0, frames), |part| (part.from, part.to));
        (from < to).then_some((from, to, frames))
    }

    /// An op of the op bar on the selection (with none, the whole sound; a crop and a delete
    /// need one).
    fn run_editor_op(&mut self, op: EditorOp) {
        let selected = self
            .sample_editor
            .as_ref()
            .is_some_and(|open| open.selection.is_some());
        if op.needs_selection() && !selected {
            return;
        }
        self.apply_editor_op(|from, to| match op {
            EditorOp::Crop => SoundOp::Crop { from, to },
            EditorOp::Delete => SoundOp::Delete { from, to },
            EditorOp::Silence => SoundOp::Silence { from, to },
            EditorOp::FadeIn => SoundOp::FadeIn { from, to },
            EditorOp::FadeOut => SoundOp::FadeOut { from, to },
            EditorOp::Normalize => SoundOp::Normalize {
                from,
                to,
                centi_db: 0,
            },
            EditorOp::Reverse => SoundOp::Reverse { from, to },
        });
    }

    /// The op `make` gives for the frames an op acts on, as one undo step of the channel's
    /// list one op longer (and its clips moved, for a crop or a delete); then the selection a
    /// crop or a delete took away goes, the cursor where the part was.
    fn apply_editor_op(&mut self, make: impl FnOnce(u64, u64) -> SoundOp) {
        let Some((from, to, frames)) = self.op_range() else {
            return;
        };
        let Some(channel) = self.sample_editor_on() else {
            return;
        };
        let op = make(from, to);
        let step = match sound_edits::append(self.session.project(), channel, op, frames) {
            Ok(step) => step,
            Err(EditError::NoFramesLeft(_)) => {
                self.notice = Some("A sound keeps at least one frame: select less".to_owned());
                return;
            }
            Err(error) => {
                self.error = Some(error.to_string());
                return;
            }
        };
        self.stop_audition();
        self.commit(step);
        if let Some(open) = &mut self.sample_editor {
            match op {
                SoundOp::Crop { .. } => {
                    open.selection = None;
                    open.cursor = 0;
                }
                SoundOp::Delete { from, .. } => {
                    open.selection = None;
                    open.cursor = from;
                }
                _ => {}
            }
        }
    }

    /// Revert: the channel's sound as its file is, one undo step ("Revert sound"), its clips
    /// moved back to play what they played.
    fn revert_sound(&mut self) {
        if !self.editor_ready() {
            return;
        }
        let Some((channel, file_frames)) = self.sample_editor.as_ref().and_then(|open| {
            open.sound
                .as_ref()
                .map(|sound| (open.channel, sound.file_frames))
        }) else {
            return;
        };
        match sound_edits::revert(self.session.project(), channel, file_frames) {
            Ok(step) => {
                self.stop_audition();
                self.commit(step);
                self.with_editor(|open| open.selection = None);
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    /// Save as new file…: the sound as it is now, as a 32-bit float WAV at the project's rate
    /// (so what is written is exactly what plays), named "<file> (edited).wav", then "(edited
    /// 2)" and on, in the folder takes go to (`samples` beside the project, else the per-user
    /// recordings folder), written on a worker while the window says "Saving…" and its
    /// buttons are dim. The document does not change, and the file the channel plays is never
    /// written; a toolbar notice names the new file, and the Library shows it.
    fn save_edited_sound(&mut self) -> Task<Message> {
        let Some(job) = self.start_save() else {
            return Task::none();
        };
        if !self.host.background {
            let done = job.run();
            self.saved_sound(done);
            return Task::none();
        }
        let (target, dir) = (job.target.clone(), job.dir.clone());
        let (thread, written) = joinable(move || job.run());
        self.editor.saves.retain(|save| !save.is_finished());
        self.editor.saves.push(thread);
        Task::perform(written, move |done| {
            let done = done.unwrap_or(SaveDone {
                target,
                dir,
                result: Err("the save stopped before it finished".to_owned()),
            });
            Message::SampleEditor(SampleEditorMessage::Saved(done))
        })
    }

    /// The app quits: Save as new file…'s writes still under way end first, the window as it
    /// was last drawn ("Saving…" on the button), so each lands whole under its name.
    pub(super) fn finish_sound_saves(&mut self) {
        for save in self.editor.saves.drain(..) {
            let _ = save.join();
        }
    }

    /// A document opened: what a Save as new file… cut short left in its `samples` folder
    /// goes, its temporary files (named as the writer names them) unwritten for
    /// [`STALE_PARTIAL`], nothing else. Why: the quit waits for the write, an app killed under
    /// it cannot, and the Library lists no such file (one is up to 230 MB).
    pub(super) fn sweep_partial_saves(&self) {
        if self.document.path.is_none() {
            return;
        }
        if let Some(dir) = self.recordings_dir() {
            dawsome_media::remove_stale_partials(&dir, STALE_PARTIAL);
        }
    }

    /// Save as new file…'s part on the window's thread: the new file named in the takes'
    /// folder, the editor dim while it is written, and the write for a worker.
    fn start_save(&mut self) -> Option<SaveJob> {
        if !self.editor_ready() {
            return None;
        }
        let Some(dir) = self.recordings_dir() else {
            self.notice = Some("Save the project first: the new file goes beside it".to_owned());
            return None;
        };
        let file = self.sample_editor.as_ref()?.sound.as_ref()?.file.clone();
        let target = edited_file_path(&dir, &file);
        match self.save_job(target, dir) {
            Ok(job) => {
                self.with_editor(|open| open.busy = Some(Busy::Saving));
                Some(job)
            }
            Err(refused) => {
                self.error = Some(refused);
                None
            }
        }
    }

    /// The write of the editor's sound to `target` in `dir`, refused when it is a file the
    /// project reads (the document, a channel's sound), as an export's output is.
    pub(super) fn save_job(&self, target: PathBuf, dir: PathBuf) -> Result<SaveJob, String> {
        let sound = self
            .sample_editor
            .as_ref()
            .and_then(|open| open.sound.as_ref())
            .ok_or_else(|| "the sample editor shows no sound".to_owned())?;
        let project = self.session.project();
        let inputs = dawsome_media::input_files(project, self.session.samples_dir())
            .into_iter()
            .chain(self.document.path.clone());
        if let Some(input) = dawsome_media::overwritten_input(&target, inputs) {
            return Err(format!(
                "refusing to write {}: it is also an input file of this project",
                input.display()
            ));
        }
        Ok(SaveJob {
            samples: sound.shared(),
            sides: sound.sides(),
            rate: project.sample_rate,
            target,
            dir,
        })
    }

    /// Save as new file…'s write ended: the editor's buttons light again; a notice names the
    /// new file and the Library shows it, or the toolbar says why it could not be written.
    fn saved_sound(&mut self, done: SaveDone) {
        self.with_editor(|open| {
            if open.busy == Some(Busy::Saving) {
                open.busy = None;
            }
        });
        match done.result {
            Ok(()) => {
                let name = done
                    .target
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.notice = Some(format!("Saved {name} in {}", done.dir.display()));
                self.show_recordings(&done.target);
            }
            Err(failed) => self.error = Some(failed),
        }
    }

    /// Play: the selection (with none, from the cursor to the end) of the sound as it is,
    /// alone and at unity, through the preview the Library plays its files through; the song
    /// is not touched. A part longer than [`COPY_INLINE_SECONDS`] is copied on a worker, the
    /// editor dim until it plays.
    fn play_audition(&mut self) -> Task<Message> {
        let Some(open) = &self.sample_editor else {
            return Task::none();
        };
        let Some(sound) = open.sound.as_ref().filter(|_| open.busy.is_none()) else {
            return Task::none();
        };
        let (from, to) = open
            .selection
            .map_or((open.cursor, sound.frames()), |part| (part.from, part.to));
        if from >= to {
            return Task::none();
        }
        let rate = u64::from(self.session.project().sample_rate.max(1));
        if to - from <= COPY_INLINE_SECONDS * rate || !self.host.background {
            let part = sound.part(from, to);
            self.play_part(part, from, to - from);
            return Task::none();
        }
        let (samples, sides) = (sound.shared(), sound.sides());
        self.stop_audition();
        self.editor.copies += 1;
        let token = self.editor.copies;
        self.with_editor(|open| {
            open.busy = Some(Busy::Copying {
                token,
                from,
                frames: to - from,
            });
        });
        Task::perform(
            blocking(move || part_of(&samples, sides, from, to)),
            move |part| {
                Message::SampleEditor(SampleEditorMessage::Copied {
                    token,
                    part: part.map(Part),
                })
            },
        )
    }

    /// `part`, frames `from` to `from + frames` of the sound, played: the Library's preview,
    /// if one plays, ends (the preview plays one sound at a time).
    fn play_part(&mut self, part: SampleBuffer, from: u64, frames: u64) {
        self.stop_preview();
        self.session.play_preview(Box::new(MemorySource::new(part)));
        self.with_editor(|open| {
            open.audition = Some(Audition {
                from,
                frames,
                started: Instant::now(),
            });
        });
    }

    /// Play's copy `token` landed: played when it is the one the editor waits for (not let go
    /// of by a Stop, another Play or the window closing), the buttons lit again.
    fn copied_for_play(&mut self, token: u64, part: Option<Part>) {
        let Some(Busy::Copying {
            token: waited,
            from,
            frames,
        }) = self.sample_editor.as_ref().and_then(|open| open.busy)
        else {
            return;
        };
        if waited != token {
            return;
        }
        self.with_editor(|open| open.busy = None);
        if let Some(Part(part)) = part {
            self.play_part(part, from, frames);
        }
    }

    /// Stops Play, if it plays (or lets go of its copy, if it is still made).
    fn stop_audition(&mut self) {
        self.with_editor(|open| {
            if matches!(open.busy, Some(Busy::Copying { .. })) {
                open.busy = None;
            }
        });
        let playing = self
            .sample_editor
            .as_mut()
            .and_then(|open| open.audition.take());
        if playing.is_some() {
            let result = self.session.stop_preview();
            self.report(result);
        }
    }

    /// Space in the editor: Play, or Stop while it plays (or while its copy is made).
    fn toggle_audition(&mut self) -> Task<Message> {
        let copying = self
            .sample_editor
            .as_ref()
            .is_some_and(|open| matches!(open.busy, Some(Busy::Copying { .. })));
        if self.audition_position().is_some() || copying {
            self.stop_audition();
            Task::none()
        } else {
            self.play_audition()
        }
    }

    /// Where Play is in the sound now, by the clock (drawn only: the player plays it).
    fn audition_position(&self) -> Option<u64> {
        let audition = self.sample_editor.as_ref()?.audition?;
        let rate = f64::from(self.session.project().sample_rate);
        let played = (audition.started.elapsed().as_secs_f64() * rate) as u64;
        (played < audition.frames).then_some(audition.from + played)
    }

    /// The editor's timer: Play's position moves (and Play ends with its part), and a sound
    /// made in the background is taken (and let go of, the editor closed meanwhile).
    fn editor_tick(&mut self) {
        let made: Vec<Made> = self.editor.built.1.try_iter().collect();
        for (build, sound) in made {
            // One sound is made at a time: this is the one asked for.
            self.editor.building = None;
            self.install_editor_sound(&build, sound);
        }
        if self.audition_position().is_none() {
            self.with_editor(|open| open.audition = None);
        }
    }

    /// Whether the editor's timer runs: while Play plays, or while a sound is being made, the
    /// editor open or not.
    pub(super) fn editor_ticks(&self) -> bool {
        self.editor.building.is_some()
            || self
                .sample_editor
                .as_ref()
                .is_some_and(|open| open.audition.is_some())
    }

    /// Whether the Gain readout is being dragged: the cursor's moves anywhere are its.
    pub(super) fn editor_gain_held(&self) -> bool {
        self.sample_editor
            .as_ref()
            .is_some_and(|open| open.gain_held)
    }

    /// Opens the editor's own window while it is open and closes it once it is not, keeping its
    /// place; asks it to the app's level again when the app's windows were last asked another.
    pub(super) fn sync_sample_editor_window(&mut self) -> Task<Message> {
        let level = pop_out_level(self.windows_above);
        let restack = self.editor.above != self.windows_above;
        self.editor.above = self.windows_above;
        match (self.sample_editor.is_some(), self.editor.window) {
            (true, None) => {
                let settings = desktop::pop_out_settings(
                    PopOut::SampleEditor,
                    self.editor.place.size(PopOut::SampleEditor),
                    self.editor.place.position(),
                    level,
                );
                let (id, opened) = window::open(settings);
                self.editor.window = Some(id);
                opened.discard()
            }
            (false, Some(id)) => {
                self.editor.window = None;
                self.editor.size = None;
                self.remember_sample_editor_place();
                window::close(id)
            }
            (true, Some(id)) if restack => window::set_level(id, level),
            _ => Task::none(),
        }
    }

    /// The editor's window opened, was resized or moved: its place is kept, and the waveform
    /// is laid out across its width.
    pub(super) fn sample_editor_window_changed(&mut self, change: WindowChange) {
        let place = &mut self.editor.place;
        let size = match change {
            WindowChange::Opened { size, position } => {
                *place = WindowPlace::of(size, position.or(place.position()));
                Some(size)
            }
            WindowChange::Resized(size) => {
                *place = WindowPlace::of(size, place.position());
                Some(size)
            }
            WindowChange::Moved(at) => {
                *place = WindowPlace::of(place.size(PopOut::SampleEditor), Some(at));
                None
            }
            WindowChange::Rescaled(_) | WindowChange::Focus(_) => None,
        };
        if let Some(size) = size {
            self.editor.size = Some(size);
            let width = ui::waveform_width(size.width);
            self.with_editor(|open| open.view = open.view.resized(width));
        }
    }

    /// Keeps the editor's place in the settings for its next opening.
    pub(super) fn remember_sample_editor_place(&mut self) {
        let place = Some(self.editor.place);
        if self.settings.sample_editor_window != place {
            self.settings.sample_editor_window = place;
            self.save_settings();
        }
    }

    /// The editor's window's size: as it last said, else the size it opens at.
    fn sample_editor_size(&self) -> Size {
        self.editor
            .size
            .unwrap_or_else(|| self.editor.place.size(PopOut::SampleEditor))
    }

    /// The window's own events: its X and its closing close the editor, Esc in it clears the
    /// selection first.
    pub(super) fn sample_editor_window_message(&mut self, message: &Message) {
        match message {
            Message::Windowed(_, change) => self.sample_editor_window_changed(*change),
            Message::WindowCloseRequested(_) => {
                self.close_sample_editor();
            }
            Message::WindowClosed(_) => {
                // Gone already: the editor is closed and its place kept.
                self.editor.window = None;
                self.editor.size = None;
                self.close_sample_editor();
                self.remember_sample_editor_place();
            }
            Message::Escape(_) => self.escape_sample_editor(),
            _ => {}
        }
    }

    /// What the editor's window shows: the editor while it is open, its ground alone in the
    /// moment between its closing and its window going.
    pub(super) fn sample_editor_view(&self) -> Element<'_, Message> {
        let Some(open) = &self.sample_editor else {
            return container(space())
                .width(Fill)
                .height(Fill)
                .style(style::panel)
                .into();
        };
        let project = self.session.project();
        let Some(channel) = project.channels.get(&open.channel) else {
            return container(space()).width(Fill).height(Fill).into();
        };
        let rate = project.sample_rate;
        let file = channel
            .sample
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let about = match &open.sound {
            Some(sound) if sound.problem.is_some() => {
                format!("cannot be read: {}", sound.problem.as_deref().unwrap_or(""))
            }
            Some(sound) => {
                let millis = sound.frames() * 1000 / u64::from(rate.max(1));
                let sides = if sound.sides() == 2 { "stereo" } else { "mono" };
                format!(
                    "{}.{:03} s \u{b7} {rate} Hz \u{b7} {sides}",
                    millis / 1000,
                    millis % 1000
                )
            }
            None => "loading\u{2026}".to_owned(),
        };
        let clips = project
            .playlist
            .clips
            .values()
            .filter(|clip| matches!(clip.item, ClipItem::Audio { channel, .. } if channel == open.channel))
            .count();
        let history = self.session.history();
        ui::view(ui::View {
            name: &channel.name,
            file,
            about,
            clips,
            edits: channel.edits.ops.len(),
            stale: self.editor_file_changed(),
            wave: open.sound.as_ref().map(|sound| Wave {
                samples: sound.samples(),
                sides: sound.sides(),
                peaks: &sound.peaks,
            }),
            view: open.view,
            bracket: self.clip_window(),
            selection: open.selection,
            cursor: open.cursor,
            playhead: self.audition_position(),
            rate,
            snap: self.settings.snap_to_zero(),
            gain: open.gain,
            ready: self.editor_ready(),
            busy: open.busy.is_some(),
            saving: open.busy == Some(Busy::Saving),
            can_undo: history.undo_len() > 0,
            can_redo: history.redo_len() > 0,
            size: self.sample_editor_size(),
            hint: self.sample_editor_hint(),
            caches: &self.editor.caches,
        })
    }

    /// The editor's foot line: what its control under the cursor is and does, while hints
    /// show; else how to use it.
    pub(super) fn sample_editor_hint(&self) -> String {
        match &self.hover {
            Some(hover @ Hover::SampleEditor(_)) if self.shows_hints() => {
                hint::words(hover, &self.hint_context())
            }
            _ => GESTURES.to_owned(),
        }
    }

    /// The editor's title: "<channel> · <file> — Sample editor".
    pub(super) fn sample_editor_title(&self) -> String {
        let Some(channel) = self
            .sample_editor_on()
            .and_then(|id| self.session.project().channels.get(&id))
        else {
            return String::new();
        };
        let file = channel
            .sample
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        format!("{} \u{b7} {file} \u{2014} Sample editor", channel.name)
    }

    /// `--open-sample-editor`: the editor opened on a channel at start, or as a double press
    /// on an audio clip opens it, frames `select` of its sound selected (and fitted, unless the
    /// clip's window shows) once it is made; the run fails when the channel plays no file or there is no such audio clip.
    pub fn open_sample_editor_at_start(
        &mut self,
        on: EditorOn,
        select: Option<(u64, u64)>,
    ) -> Task<Message> {
        // As the doors into it open it.
        let refused = match on {
            EditorOn::Channel(channel) => {
                let _ = self.handle(Message::OpenSampleEditor(ChannelId(channel)));
                format!("--open-sample-editor: channel {channel} plays no sound file")
            }
            EditorOn::Clip(clip) => {
                self.open_sample_editor_on_clip(ClipId(clip));
                format!("--open-sample-editor: clip {clip} is no audio clip playing a file")
            }
        };
        if self.sample_editor.is_none() {
            return self.headless.fail(&refused).map(Message::Headless);
        }
        if let Some((from, to)) = select {
            let _ = self.handle(Message::SampleEditor(SampleEditorMessage::Select {
                from,
                to,
            }));
            self.with_editor(|open| {
                if let (Some(part), None) = (open.selection, open.clip) {
                    open.view = open.view.showing(part.from, part.to);
                }
            });
        }
        Task::none()
    }
}

/// The first free "<stem> (edited).wav", "<stem> (edited 2).wav", … in `dir`, for the sound
/// of `file`.
pub fn edited_file_path(dir: &Path, file: &Path) -> PathBuf {
    let stem = file
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "sound".to_owned());
    (1u32..)
        .map(|number| match number {
            1 => dir.join(format!("{stem} (edited).wav")),
            _ => dir.join(format!("{stem} (edited {number}).wav")),
        })
        .find(|path| !path.exists())
        .unwrap_or_else(|| dir.join(format!("{stem} (edited).wav")))
}

#[cfg(test)]
mod tests;
