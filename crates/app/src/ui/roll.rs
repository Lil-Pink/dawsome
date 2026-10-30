//! The piano roll's UI state: what the grid shows, the snap, the length of the next drawn note,
//! the selection and the gesture in progress. It turns the canvases' messages into edits
//! through [`note_gesture`] and [`note_ops`] and never changes the document itself. The
//! selection is UI state, never an edit: a set of (channel, tick, key).
//!
//! Selecting: a click on a note selects it alone (a press on a selected note keeps
//! the others until it is let go of unmoved, so they drag together), Shift+click (or
//! Ctrl+click) adds it or takes it out, a drag from empty grid draws a box that selects what it meets (Shift adds to
//! the selection), and a drag on a selected note moves them all. A click on empty grid that
//! does not move draws a note there when it is let go of; a double-click on a note deletes it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::slice;

use dawsome_engine::edit::Edit;
use dawsome_engine::project::{self as model, ClipId, Note, PatternId, Project, MAX_KEY};
use dawsome_engine::time::Ticks;
use iced::keyboard::Modifiers;
use iced::{mouse, Point, Size};

use crate::ui::note_gesture::{self, key_of, Change, NoteKey, Snap, Target};
use crate::ui::note_ops::{self, Clip, Quantize};
use crate::ui::piano_layout::{self, drawn_length, NotePart, Rows, View, ZOOM_LEVELS};
use crate::ui::scale::{Root, Scale, ScaleMode};
use crate::ui::scroll::{self, BarMessage, Span};

/// Rows the wheel scrolls per notch (keys, unless folded).
const KEYS_PER_NOTCH: i32 = 3;
/// Rows shown above the channel's highest note when the roll opens on it.
const ROWS_ABOVE_NOTES: i32 = 5;
/// The top key when the channel has no notes in the pattern.
const EMPTY_TOP_KEY: i32 = 71;
/// How far a press on empty grid may move before it is a box rather than a click that draws.
pub const DRAG_SLOP: f32 = 4.0;
/// The notes' menu opened from the strip's button hangs this far left of the grid's right
/// edge: its width and the strip's margin.
pub const MENU_FROM_RIGHT: f32 = 176.0;

/// A piano roll edit asked for by a key, the notes' menu or the Quantize dialog. The keys'
/// edits act on the selection; the operations from Quantize on act on the selection, or on
/// every note of the channel when nothing is selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteEdit {
    /// The selected notes up (or down) this many semitones.
    Transpose(i32),
    /// The selected notes later (or earlier) by this many snap units.
    Nudge(i32),
    Copy,
    Cut,
    Paste,
    Delete,
    Duplicate,
    Quantize(Quantize),
    /// Cut at every snap line.
    Chop,
    Glue,
    Legato,
    Reverse,
    Invert,
    Humanize,
}

/// The grid the Quantize dialog moves notes to: the roll's snap, or a note value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuantizeGrid {
    Snap,
    /// A 1/n note: 1/16 is a step, 1/1 a bar (four beats).
    Note(u8),
}

impl QuantizeGrid {
    /// Every choice, as the dialog lists them.
    pub const ALL: [QuantizeGrid; 7] = [
        QuantizeGrid::Snap,
        QuantizeGrid::Note(32),
        QuantizeGrid::Note(16),
        QuantizeGrid::Note(8),
        QuantizeGrid::Note(4),
        QuantizeGrid::Note(2),
        QuantizeGrid::Note(1),
    ];

    /// Its grid in ticks: the snap unit `snap`, or 4·ppq/n.
    pub fn ticks(self, snap: Ticks, ppq: u16) -> Ticks {
        match self {
            QuantizeGrid::Snap => snap,
            QuantizeGrid::Note(n) => Ticks((4 * i64::from(ppq) / i64::from(n.max(1))).max(1)),
        }
    }
}

impl fmt::Display for QuantizeGrid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QuantizeGrid::Snap => f.write_str("Snap"),
            QuantizeGrid::Note(1) => f.write_str("1 bar"),
            QuantizeGrid::Note(n) => write!(f, "1/{n}"),
        }
    }
}

/// What the Quantize dialog's controls ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuantizeMessage {
    /// The notes menu's Quantize… row: the dialog opens with the last choice.
    Open,
    Grid(QuantizeGrid),
    /// Per cent, 0 to 100.
    Strength(u8),
    /// Whether the ends move too.
    Ends(bool),
    Apply,
    Cancel,
}

/// What the Quantize dialog chooses: the grid, how far toward it (per cent), and whether the
/// ends move too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuantizeForm {
    pub grid: QuantizeGrid,
    pub strength: u8,
    pub ends: bool,
}

impl Default for QuantizeForm {
    /// The snap, all the way, the starts only.
    fn default() -> Self {
        Self {
            grid: QuantizeGrid::Snap,
            strength: 100,
            ends: false,
        }
    }
}

/// What the piano roll's canvases report. Points are in the reporting canvas's coordinates
/// (see [`piano_layout`]), so a test that sends them drives the real mapping.
#[derive(Debug, Clone, PartialEq)]
pub enum RollMessage {
    GridPressed {
        at: Point,
        button: mouse::Button,
        /// The press repeats the last one quickly in place: a double-click.
        double: bool,
    },
    /// The cursor moved while a grid gesture is held.
    GridDragged(Point),
    GridReleased(Point),
    LanePressed(Point),
    LaneDragged(Point),
    LaneReleased(Point),
    GridScrolled {
        at: Point,
        delta: mouse::ScrollDelta,
    },
    /// The grid canvas was laid out at a new size.
    GridResized(Size),
    SnapSelected(Snap),
    /// The lane's drag handle pressed, at this window y.
    HandlePressed(f32),
    /// The cursor at this window y while the handle is held.
    HandleDragged(f32),
    /// The handle let go of.
    HandleReleased,
    /// The grid's scroll bar asks for another view.
    Bar(BarMessage),
    /// The strip's Notes button: the notes' menu opens under it, or closes.
    MenuButton,
    /// The strip's scale chooser and its root.
    ScaleSelected(ScaleMode),
    RootSelected(Root),
    /// The strip's Fold switch.
    ToggleFold,
}

/// An edit a gesture asks for, and the selection to hold once it is applied. `None` keeps the
/// selection (less any note the edit removed).
#[derive(Debug, Clone, PartialEq)]
pub struct Commit {
    pub edit: Edit,
    pub selection: Option<BTreeSet<NoteKey>>,
}

/// A drag in progress. Moves and resizes act on the notes as they were at the press.
#[derive(Debug, Clone, PartialEq)]
enum Gesture {
    Move {
        grabbed: Note,
        notes: Vec<Note>,
        from: Point,
        to: Point,
    },
    Resize {
        grabbed: Note,
        notes: Vec<Note>,
        from: Point,
        to: Point,
    },
    /// A press on empty grid not yet moved: let go of in place it draws a note at `at`;
    /// moved past [`DRAG_SLOP`] it becomes a box. `add`: Shift was held.
    Pending { at: Point, add: bool },
    /// A box dragged out from empty grid (at once with Ctrl); `add`: what it meets joins the
    /// selection rather than replacing it.
    Box { from: Point, to: Point, add: bool },
    /// A selected stem dragged in the velocity lane: `notes` take `steps`.
    Velocity { notes: Vec<Note>, steps: u8 },
    /// A drag across the lane from anywhere else: each stem it passes takes the height the
    /// cursor passed it at. `notes` are the channel's at the press; `last` where the cursor
    /// was seen last.
    Paint {
        notes: Vec<Note>,
        painted: BTreeMap<NoteKey, (Note, u8)>,
        last: Point,
        steps: u8,
    },
    /// Alt+drag in the lane: a line whose stems (the selected, or all) take its heights.
    Ramp { from: Point, to: Point },
}

/// The piano roll's state between frames.
#[derive(Debug, Clone)]
pub struct Roll {
    pub view: View,
    pub snap: Snap,
    /// The length of the next drawn note: that of the last note clicked or resized.
    pub draw_length: Ticks,
    pub selection: BTreeSet<NoteKey>,
    gesture: Option<Gesture>,
    /// The grid canvas's size, as it last reported it.
    grid_size: Size,
    /// The velocity lane's height, which its handle drags ([`piano_layout::kept_lane`]).
    lane_height: f32,
    /// The handle held: the window y it was pressed at and the lane's height then.
    handle: Option<(f32, f32)>,
    /// The scroll bar's thumb is held: the playhead does not move the view meanwhile.
    bar_held: bool,
    /// The tick last clicked in each pattern's grid (snapped down), moved on past each paste:
    /// where a paste into that pattern lands while stopped. A pattern never clicked in pastes at
    /// its start.
    paste_at: BTreeMap<dawsome_engine::project::PatternId, Ticks>,
    /// The note the last click drew, so the second press of a double-click leaves it.
    just_drawn: Option<NoteKey>,
    /// The notes' menu, open where the right press was (grid coordinates).
    pub menu: Option<Point>,
    /// The key [`Roll::reveal`] put in view, until the grid is next laid out: a roll opened by
    /// the reveal knows its height only then, and keeps the key in view at it.
    revealed: Option<u8>,
    /// How many times Humanize ran: the next one's seed, so a session humanizes the same way
    /// every time it is replayed.
    humanized: u64,
    /// The Quantize dialog's choice, kept for its next opening.
    pub quantize: QuantizeForm,
    /// The scale the grid shades the keys outside of; none by default.
    pub scale: Scale,
    /// Whether the grid keeps rows only for the keys holding notes and, with a scale, those in
    /// it.
    pub fold: bool,
    /// The playlist clip the roll edits, when it was opened from one (a double-click on it, or
    /// the clip a document opens on): the strip's link control shows how that clip shares its
    /// pattern. `None` when the roll was opened from its key or panel button, which edit the
    /// pattern alone.
    pub clip: Option<ClipId>,
    /// The pattern `clip` played when the roll last looked: a clip whose pattern changes under
    /// an unchanged selection (unlinked, linked back, undone) takes the selection with it.
    clip_pattern: Option<PatternId>,
}

impl Roll {
    /// A roll at the default zoom and snap for a project whose step is `step` ticks, drawing
    /// into a grid of `grid_size`.
    pub fn new(step: Ticks, grid_size: Size) -> Self {
        let step = Ticks(step.0.max(1));
        Self {
            view: View {
                top: piano_layout::clamp_top(EMPTY_TOP_KEY, grid_size.height),
                scroll: Ticks(0),
                zoom: piano_layout::DEFAULT_ZOOM,
                step,
                rows: crate::ui::piano_layout::Rows::ALL,
            },
            snap: Snap::default(),
            draw_length: step,
            selection: BTreeSet::new(),
            gesture: None,
            grid_size,
            lane_height: piano_layout::LANE_HEIGHT,
            handle: None,
            bar_held: false,
            paste_at: BTreeMap::new(),
            just_drawn: None,
            menu: None,
            revealed: None,
            humanized: 0,
            quantize: QuantizeForm::default(),
            scale: Scale::default(),
            fold: false,
            clip: None,
            clip_pattern: None,
        }
    }

    /// The roll edits pattern clip `clip` of `project` from now on.
    pub fn edit_clip(&mut self, project: &Project, clip: ClipId) {
        let pattern = project
            .playlist
            .clips
            .get(&clip)
            .and_then(model::Clip::pattern);
        self.clip = pattern.map(|_| clip);
        self.clip_pattern = pattern;
    }

    /// The roll edits no clip: it was opened from its key or panel button.
    pub fn forget_clip(&mut self) {
        self.clip = None;
        self.clip_pattern = None;
    }

    /// Keeps the roll's clip in line with a changed document and the pattern `selected`: the
    /// clip gone, or the selection moved to another pattern than the clip's (by hand, a take),
    /// the roll forgets it; the clip's pattern changed while the selection stayed on the one it
    /// played (unlinked, linked back, either undone), the roll follows it, and the pattern it
    /// now plays is returned for the pattern box to select.
    pub fn keep_clip(
        &mut self,
        project: &Project,
        selected: Option<PatternId>,
    ) -> Option<PatternId> {
        let clip = self.clip?;
        let now = project
            .playlist
            .clips
            .get(&clip)
            .and_then(model::Clip::pattern);
        match now {
            Some(now) if Some(now) == selected => {
                self.clip_pattern = Some(now);
                None
            }
            Some(now) if selected.is_some() && selected == self.clip_pattern => {
                self.clip_pattern = Some(now);
                Some(now)
            }
            _ => {
                self.forget_clip();
                None
            }
        }
    }

    /// The same roll with its velocity lane `height` tall, as [`piano_layout::kept_lane`] keeps
    /// it: the height the settings keep.
    pub fn with_lane_height(mut self, height: f32) -> Self {
        self.lane_height = piano_layout::kept_lane(height);
        self
    }

    pub fn grid_size(&self) -> Size {
        self.grid_size
    }

    /// The velocity lane's height.
    pub fn lane_height(&self) -> f32 {
        self.lane_height
    }

    /// Whether the lane's handle is held, so its drags and its release are reported wherever
    /// the cursor is.
    pub fn handle_held(&self) -> bool {
        self.handle.is_some()
    }

    /// Shows `target`'s channel from the top again, as when the roll opens or the channel
    /// changes: the top row 5 keys above its highest note (key 71 without notes), nothing
    /// selected, no gesture.
    pub fn show(&mut self, target: Option<&Target>) {
        self.fit_rows(target);
        let highest = target.and_then(|target| target.channel_notes().map(|n| n.key).max());
        let top = highest.map_or(EMPTY_TOP_KEY, |key| i32::from(key) + ROWS_ABOVE_NOTES);
        self.view.top = self.view.clamp_top(top, self.grid_size.height);
        self.revealed = None;
        self.forget_notes();
    }

    /// Shows `target`'s channel as [`show`](Self::show) does, scrolled to its first note at
    /// or after pattern tick `from` (its first note, when none is): the bar holding that note
    /// at the grid's left edge, and its row in view, 5 keys under the top when the top
    /// [`show`](Self::show) chose leaves it out. A double-click on a clip opens the roll so.
    pub fn reveal(&mut self, target: Option<&Target>, from: Ticks) {
        self.show(target);
        let Some(target) = target else {
            return;
        };
        let mut notes = target.channel_notes();
        let first = notes
            .clone()
            .find(|note| note.start >= from)
            .or_else(|| notes.next());
        let Some(first) = first else {
            return;
        };
        let bar = self.view.bar().0.max(1);
        self.view.scroll = Ticks(first.start.0.div_euclid(bar) * bar);
        self.clamp_scroll(notes_end(target));
        self.keep_in_view(first.key);
        self.revealed = Some(first.key);
    }

    /// The top moved, when `key`'s row is not whole in the grid, to 5 keys over `key`.
    fn keep_in_view(&mut self, key: u8) {
        let y = self.view.row_top(key);
        if !(0.0..=self.grid_size.height - piano_layout::ROW_HEIGHT).contains(&y) {
            let top = i32::from(key) + ROWS_ABOVE_NOTES;
            self.view.top = self.view.clamp_top(top, self.grid_size.height);
        }
    }

    /// The rows the grid keeps for `target`'s channel: every key, or folded the keys holding
    /// its notes and (with a scale) the keys in the scale; every key when that leaves none.
    pub fn fit_rows(&mut self, target: Option<&Target>) {
        self.view.rows = match target.filter(|_| self.fold) {
            Some(target) => {
                let scale = self.scale;
                let in_scale = (0..=MAX_KEY).filter(move |&key| scale.is_on() && scale.holds(key));
                Rows::of(target.channel_notes().map(|note| note.key).chain(in_scale))
            }
            None => Rows::ALL,
        };
        self.view.top = self
            .view
            .clamp_top(i32::from(self.view.top), self.grid_size.height);
    }

    /// Drops the selection and any gesture: the notes they name belong to another pattern or
    /// channel now.
    pub fn forget_notes(&mut self) {
        self.selection.clear();
        self.gesture = None;
        self.menu = None;
    }

    /// Ends a gesture without committing it.
    pub fn cancel_gesture(&mut self) {
        self.gesture = None;
    }

    /// Brings the state in line with a changed document: selected notes that are gone are
    /// dropped, and the view stays inside the (possibly shorter) pattern.
    pub fn refresh(&mut self, target: Option<&Target>) {
        match target {
            Some(target) => {
                self.selection.retain(|&key| {
                    target
                        .note(key)
                        .is_some_and(|n| n.channel == target.channel)
                });
                self.fit_rows(Some(target));
                self.clamp_scroll(notes_end(target));
            }
            None => self.forget_notes(),
        }
    }

    /// The grid's scroll along time, as its bar shows it: the content reaches
    /// [`piano_layout::BARS_PAST_END`] bars past the pattern's end or its last note's, whichever
    /// is later (a bar's worth past the end without a channel shown).
    pub fn span(&self, target: Option<&Target>) -> Span {
        let end = target.map_or(self.view.bar(), notes_end);
        piano_layout::time_span(&self.view, end, self.grid_size.width)
    }

    /// While playing, the view jumps a page when the playhead `at` (the song's tick, drawn
    /// where it falls in the pattern) leaves it, starting at the bar that holds it; not while the
    /// scroll bar's thumb is held. Returns whether the view moved.
    pub fn follow(&mut self, target: Option<&Target>, at: Ticks) -> bool {
        let Some(target) = target.filter(|_| !self.bar_held) else {
            return false;
        };
        let at = at.0.rem_euclid(target.end().0.max(1));
        let span = self.span(Some(target));
        let bar = self.view.bar().0 as f64;
        let Some(offset) = scroll::follow(span.offset, span.view, at as f64, bar) else {
            return false;
        };
        let before = self.view.scroll;
        self.view.scroll = Ticks(offset as i64);
        self.clamp_scroll(notes_end(target));
        self.view.scroll != before
    }

    /// The scroll bar's thumb is no longer held: what its release does, for a bar that went
    /// without one (its panel closed).
    pub fn let_go_of_bar(&mut self) {
        self.bar_held = false;
    }

    /// Whether a move, resize, box select or a press on empty grid is held on the grid.
    pub fn grid_held(&self) -> bool {
        matches!(
            self.gesture,
            Some(
                Gesture::Move { .. }
                    | Gesture::Resize { .. }
                    | Gesture::Box { .. }
                    | Gesture::Pending { .. }
            )
        )
    }

    /// Where a press on empty grid that has not moved was, while one is held: let go of now,
    /// it draws a note there.
    pub fn pending_draw(&self) -> Option<Point> {
        match self.gesture {
            Some(Gesture::Pending { at, .. }) => Some(at),
            _ => None,
        }
    }

    /// The tick a paste into `pattern` lands on while stopped: the last clicked in its grid,
    /// moved on past each paste; its start when it was never clicked in (a tick clicked in a
    /// longer pattern would paste past this one's end and grow it).
    pub fn paste_at(&self, pattern: dawsome_engine::project::PatternId) -> Ticks {
        self.paste_at.get(&pattern).copied().unwrap_or(Ticks(0))
    }

    /// Whether a move of notes is held on the grid.
    pub fn moving(&self) -> bool {
        matches!(self.gesture, Some(Gesture::Move { .. }))
    }

    /// Whether a velocity drag, paint or ramp is held in the lane.
    pub fn lane_held(&self) -> bool {
        matches!(
            self.gesture,
            Some(Gesture::Velocity { .. } | Gesture::Paint { .. } | Gesture::Ramp { .. })
        )
    }

    /// The velocity a lane drag would set (a paint: the last it set), while one is held.
    pub fn dragged_velocity(&self) -> Option<u8> {
        match self.gesture {
            Some(Gesture::Velocity { steps, .. } | Gesture::Paint { steps, .. }) => Some(steps),
            _ => None,
        }
    }

    /// The ramp being drawn in the lane, end to end, in lane coordinates.
    pub fn ramp_line(&self) -> Option<(Point, Point)> {
        match self.gesture {
            Some(Gesture::Ramp { from, to }) => Some((from, to)),
            _ => None,
        }
    }

    /// The box being dragged out, corner to corner, in grid coordinates.
    pub fn select_box(&self) -> Option<(Point, Point)> {
        match self.gesture {
            Some(Gesture::Box { from, to, .. }) => Some((from, to)),
            _ => None,
        }
    }

    /// What the held drag would change if released now, and the selection it would leave:
    /// the canvases draw this, so a drag shows exactly what its release commits.
    pub fn preview(
        &self,
        target: &Target,
        modifiers: Modifiers,
    ) -> Option<(Change, BTreeSet<NoteKey>)> {
        self.change(self.gesture.as_ref()?, target, modifiers)
    }

    /// Handles one canvas message; returns the edit it commits, if any.
    pub fn update(
        &mut self,
        message: RollMessage,
        target: Option<&Target>,
        modifiers: Modifiers,
    ) -> Option<Commit> {
        match message {
            RollMessage::GridResized(size) => {
                self.grid_size = size;
                self.view.top = self.view.clamp_top(i32::from(self.view.top), size.height);
                if let Some(key) = self.revealed.take() {
                    self.keep_in_view(key);
                }
                if let Some(target) = target {
                    self.clamp_scroll(notes_end(target));
                }
                None
            }
            RollMessage::SnapSelected(snap) => {
                self.snap = snap;
                None
            }
            RollMessage::GridScrolled { at, delta } => {
                self.scroll(target, at, delta, modifiers);
                None
            }
            RollMessage::GridPressed { at, button, double } => {
                self.press_grid(target?, at, button, modifiers, double)
            }
            RollMessage::GridDragged(at) => {
                self.drag_to(at);
                None
            }
            RollMessage::GridReleased(at) => {
                self.drag_to(at);
                self.release(target?, modifiers)
            }
            RollMessage::LanePressed(at) => {
                self.press_lane(target?, at, modifiers);
                None
            }
            RollMessage::LaneDragged(at) => {
                self.drag_to(at);
                None
            }
            RollMessage::LaneReleased(at) => {
                self.drag_to(at);
                self.release(target?, modifiers)
            }
            RollMessage::HandlePressed(y) => {
                self.handle = Some((y, self.lane_height));
                None
            }
            RollMessage::HandleDragged(y) => {
                if let Some((from, start)) = self.handle {
                    self.lane_height = piano_layout::dragged_lane(start, from, y);
                }
                None
            }
            RollMessage::HandleReleased => {
                self.handle = None;
                None
            }
            RollMessage::Bar(BarMessage::To { offset, held }) => {
                self.bar_held = held;
                self.view.scroll = Ticks(offset.round() as i64);
                let end = target.map_or(self.view.bar(), notes_end);
                self.clamp_scroll(end);
                None
            }
            RollMessage::Bar(BarMessage::LetGo) => {
                self.bar_held = false;
                None
            }
            RollMessage::ScaleSelected(mode) => {
                self.scale.mode = mode;
                self.fit_rows(target);
                None
            }
            RollMessage::RootSelected(root) => {
                self.scale.root = root;
                self.fit_rows(target);
                None
            }
            RollMessage::ToggleFold => {
                self.fold = !self.fold;
                self.fit_rows(target);
                None
            }
            RollMessage::MenuButton => {
                self.menu = match self.menu {
                    Some(_) => None,
                    // Under the strip's right end, where the button is.
                    None => Some(Point::new(
                        (self.grid_size.width - MENU_FROM_RIGHT).max(0.0),
                        0.0,
                    )),
                };
                None
            }
        }
    }

    /// Delete: takes the selected notes out.
    pub fn delete_selected(&self, target: &Target) -> Option<Commit> {
        let notes = target.notes_with(&self.selection);
        let edit = note_gesture::deleted(target, &notes).into_edit(target)?;
        Some(Commit {
            edit,
            selection: Some(BTreeSet::new()),
        })
    }

    /// Ctrl+B: copies the selection (every note of the channel when none is selected) one
    /// extent later, rounded up to the snap; the copies become the selection.
    pub fn duplicate_selected(&self, target: &Target) -> Option<Commit> {
        let notes = if self.selection.is_empty() {
            target.channel_notes().cloned().collect()
        } else {
            target.notes_with(&self.selection)
        };
        let unit = self.snap.unit(&self.view);
        let change = note_gesture::duplicated(target, &notes, unit, self.view.step);
        let selection = change.added_keys();
        let edit = change.into_edit(target)?;
        Some(Commit {
            edit,
            selection: Some(selection),
        })
    }

    /// Ctrl+A: selects every note of the channel.
    pub fn select_all(&mut self, target: &Target) {
        self.selection = target.channel_notes().map(key_of).collect();
    }

    /// The selected notes, in the pattern's order.
    fn selected(&self, target: &Target) -> Vec<Note> {
        target.notes_with(&self.selection)
    }

    /// `change` as one edit, the notes it puts in becoming the selection.
    fn commit_selecting(change: Change, target: &Target) -> Option<Commit> {
        let selection = change.added_keys();
        let edit = change.into_edit(target)?;
        Some(Commit {
            edit,
            selection: Some(selection),
        })
    }

    /// Up/Down (Shift: an octave): the selected notes up or down `keys` semitones, together.
    pub fn transpose_selected(&self, target: &Target, keys: i32) -> Option<Commit> {
        let change = note_ops::transposed(target, &self.selected(target), keys);
        Self::commit_selecting(change, target)
    }

    /// Left/Right: the selected notes earlier or later by `units` of the snap.
    pub fn nudge_selected(&self, target: &Target, units: i32) -> Option<Commit> {
        let ticks = Ticks(self.snap.unit(&self.view).0 * i64::from(units));
        let change = note_ops::nudged(target, &self.selected(target), ticks);
        Self::commit_selecting(change, target)
    }

    /// Ctrl+C: the selected notes as a clip; none with nothing selected.
    pub fn copy_selected(&self, target: &Target) -> Option<Clip> {
        Clip::of(&self.selected(target), self.view.step)
    }

    /// The notes an operation acts on: the selection, or every note of the channel when
    /// nothing is selected.
    fn acted_on(&self, target: &Target) -> Vec<Note> {
        if self.selection.is_empty() {
            target.channel_notes().cloned().collect()
        } else {
            self.selected(target)
        }
    }

    /// Quantize, Chop, Glue, Legato, Reverse, Invert or Humanize on the notes acted on, as one
    /// edit; a selection stays on the notes it became. Chop cuts at the snap (a step with Snap
    /// None); Humanize draws
    /// from its own count, so each run differs and a replay is the same.
    pub fn operate(&mut self, target: &Target, edit: NoteEdit) -> Option<Commit> {
        let notes = self.acted_on(target);
        let step = self.view.step;
        let change = match edit {
            NoteEdit::Quantize(how) => note_ops::quantized(target, &notes, how),
            // Snap None would cut a note into single ticks: it chops at the step then.
            NoteEdit::Chop => {
                let unit = match self.snap {
                    Snap::None => self.view.step,
                    snap => snap.unit(&self.view),
                };
                note_ops::chopped(target, &notes, unit)
            }
            NoteEdit::Glue => note_ops::glued(target, &notes),
            NoteEdit::Legato => note_ops::legato(target, &notes),
            NoteEdit::Reverse => note_ops::reversed(target, &notes, step),
            NoteEdit::Invert => note_ops::inverted(target, &notes),
            NoteEdit::Humanize => {
                let seed = self.humanized;
                self.humanized += 1;
                note_ops::humanized(target, &notes, seed, step)
            }
            _ => return None,
        };
        let selection = if self.selection.is_empty() {
            BTreeSet::new()
        } else {
            change.added_keys()
        };
        let edit = change.into_edit(target)?;
        Some(Commit {
            edit,
            selection: Some(selection),
        })
    }

    /// Ctrl+V: `clip` on the channel from tick `at`, the pasted notes becoming the selection;
    /// the next paste lands after them (their span rounded up to the snap).
    pub fn paste(&mut self, target: &Target, clip: &Clip, at: Ticks) -> Option<Commit> {
        let unit = self.snap.unit(&self.view).0.max(1);
        let span = clip.span.0.max(1);
        let next = Ticks(at.0 + (span + unit - 1).div_euclid(unit) * unit);
        self.paste_at.insert(target.pattern, next);
        Self::commit_selecting(note_ops::pasted(target, clip, at), target)
    }

    /// The snap unit for a gesture: the chosen snap, or single ticks while Alt is held.
    fn unit(&self, modifiers: Modifiers) -> Ticks {
        if modifiers.alt() {
            Ticks(1)
        } else {
            self.snap.unit(&self.view)
        }
    }

    fn press_grid(
        &mut self,
        target: &Target,
        at: Point,
        button: mouse::Button,
        modifiers: Modifiers,
        double: bool,
    ) -> Option<Commit> {
        if self.gesture.is_some() {
            return None;
        }
        let hit = piano_layout::note_under(&self.view, target.channel_notes(), at.x, at.y);
        let drawn = self.just_drawn.take();
        if button == mouse::Button::Left && at.x >= piano_layout::KEY_COLUMN_WIDTH {
            let tick = Ticks(self.view.tick_at(at.x).0.max(0));
            let clicked = note_gesture::snap_down(tick, self.unit(modifiers));
            self.paste_at.insert(target.pattern, clicked);
        }
        match (button, hit) {
            // The notes' menu: a right press on a note not selected selects it alone first, so
            // the menu acts on what was pressed.
            (mouse::Button::Right, hit) if at.x >= piano_layout::KEY_COLUMN_WIDTH => {
                if let Some((note, _)) = hit {
                    let key = key_of(note);
                    if !self.selection.contains(&key) {
                        self.selection = BTreeSet::from([key]);
                    }
                }
                self.menu = Some(at);
                None
            }
            // The second press of the double-click whose first click drew this note keeps it:
            // a double-click on empty grid draws one note.
            (mouse::Button::Left, Some((note, _))) if double && drawn == Some(key_of(note)) => None,
            (mouse::Button::Left, Some((note, _))) if double => {
                let edit =
                    note_gesture::deleted(target, slice::from_ref(note)).into_edit(target)?;
                Some(Commit {
                    edit,
                    selection: None,
                })
            }
            (mouse::Button::Left, Some((note, _))) if modifiers.command() || modifiers.shift() => {
                let key = key_of(note);
                if !self.selection.remove(&key) {
                    self.selection.insert(key);
                }
                None
            }
            (mouse::Button::Left, Some((note, part))) => {
                self.draw_length = drawn_length(note, self.view.step);
                let key = key_of(note);
                if !self.selection.contains(&key) {
                    self.selection = BTreeSet::from([key]);
                }
                let (grabbed, notes) = (note.clone(), target.notes_with(&self.selection));
                self.gesture = Some(match part {
                    NotePart::Body => Gesture::Move {
                        grabbed,
                        notes,
                        from: at,
                        to: at,
                    },
                    NotePart::RightEdge => Gesture::Resize {
                        grabbed,
                        notes,
                        from: at,
                        to: at,
                    },
                });
                None
            }
            (mouse::Button::Left, None) if modifiers.command() => {
                self.gesture = Some(Gesture::Box {
                    from: at,
                    to: at,
                    add: modifiers.shift(),
                });
                None
            }
            (mouse::Button::Left, None) if at.x >= piano_layout::KEY_COLUMN_WIDTH => {
                self.gesture = Some(Gesture::Pending {
                    at,
                    add: modifiers.shift(),
                });
                None
            }
            _ => None,
        }
    }

    /// A click on empty grid let go of in place: a new note at the snapped-down tick and the
    /// row's key; the selection empties.
    fn draw_at(&mut self, target: &Target, at: Point, modifiers: Modifiers) -> Option<Commit> {
        if at.x < piano_layout::KEY_COLUMN_WIDTH {
            return None;
        }
        let key = u8::try_from(self.view.key_at(at.y))
            .ok()
            .filter(|&key| key <= MAX_KEY)?;
        let tick = note_gesture::snap_down(self.view.tick_at(at.x), self.unit(modifiers));
        let change = note_gesture::draw(target, tick, key, self.draw_length);
        self.just_drawn = change.added_keys().into_iter().next();
        let edit = change.into_edit(target)?;
        Some(Commit {
            edit,
            selection: Some(BTreeSet::new()),
        })
    }

    /// A press in the velocity lane: Alt starts a ramp; a selected stem drags every selected
    /// note to one value; anywhere else starts a paint across the stems.
    fn press_lane(&mut self, target: &Target, at: Point, modifiers: Modifiers) {
        if self.gesture.is_some() {
            return;
        }
        if modifiers.alt() {
            self.gesture = Some(Gesture::Ramp { from: at, to: at });
            return;
        }
        let steps = piano_layout::lane_steps(at.y, self.lane_height);
        let picked = piano_layout::stem_under(&self.view, target.channel_notes(), at.x);
        if picked.is_some_and(|note| self.selection.contains(&key_of(note))) {
            self.gesture = Some(Gesture::Velocity {
                notes: target.notes_with(&self.selection),
                steps,
            });
            return;
        }
        self.gesture = Some(Gesture::Paint {
            notes: target.channel_notes().cloned().collect(),
            painted: BTreeMap::new(),
            last: at,
            steps,
        });
        self.paint_to(at);
    }

    /// The paint held reaches `at`: every stem whose middle lies between where the cursor was
    /// last seen and `at` (or within the pick distance of either) takes the height the line
    /// between them has there.
    fn paint_to(&mut self, at: Point) {
        let (view, lane) = (self.view, self.lane_height);
        let Some(Gesture::Paint {
            notes,
            painted,
            last,
            steps,
        }) = &mut self.gesture
        else {
            return;
        };
        let (from, to) = (*last, at);
        let (left, right) = (from.x.min(to.x), from.x.max(to.x));
        let reach = piano_layout::STEM_PICK_DISTANCE;
        for note in notes.iter() {
            let middle = view.x_of(note.start) + piano_layout::STEM_WIDTH / 2.0;
            if middle < left - reach || middle > right + reach {
                continue;
            }
            let along = if to.x == from.x {
                1.0
            } else {
                ((middle - from.x) / (to.x - from.x)).clamp(0.0, 1.0)
            };
            let height = piano_layout::lane_steps(from.y + (to.y - from.y) * along, lane);
            painted.insert(key_of(note), (note.clone(), height));
        }
        *last = at;
        *steps = piano_layout::lane_steps(at.y, lane);
    }

    fn drag_to(&mut self, at: Point) {
        if let Some(Gesture::Pending { at: from, add }) = self.gesture {
            if from.distance(at) > DRAG_SLOP {
                self.gesture = Some(Gesture::Box { from, to: at, add });
            }
            return;
        }
        if matches!(self.gesture, Some(Gesture::Paint { .. })) {
            self.paint_to(at);
            return;
        }
        match &mut self.gesture {
            Some(Gesture::Pending { .. } | Gesture::Paint { .. }) => {}
            Some(Gesture::Ramp { to, .. }) => *to = at,
            Some(
                Gesture::Move { to, .. } | Gesture::Resize { to, .. } | Gesture::Box { to, .. },
            ) => *to = at,
            Some(Gesture::Velocity { steps, .. }) => {
                *steps = piano_layout::lane_steps(at.y, self.lane_height);
            }
            None => {}
        }
    }

    /// Ends the held gesture: a click on empty grid draws a note; a box selects what it meets
    /// (with Shift, adds it); a move, resize or velocity drag commits one edit, or none when it
    /// changed nothing.
    fn release(&mut self, target: &Target, modifiers: Modifiers) -> Option<Commit> {
        let gesture = self.gesture.take()?;
        if let Gesture::Pending { at, .. } = gesture {
            return self.draw_at(target, at, modifiers);
        }
        if let Gesture::Box { from, to, add } = gesture {
            let view = self.view;
            let met = target
                .channel_notes()
                .filter(|note| piano_layout::meets_box(&view, note, from, to))
                .map(key_of);
            if add {
                self.selection.extend(met);
            } else {
                self.selection = met.collect();
            }
            return None;
        }
        // A click on one of several selected notes that did not move selects it alone.
        if let Gesture::Move { grabbed, .. } = &gesture {
            let unmoved = self
                .change(&gesture, target, modifiers)
                .is_none_or(|(change, _)| change.is_empty());
            if unmoved {
                self.selection = BTreeSet::from([key_of(grabbed)]);
                return None;
            }
        }
        let (change, selection) = self.change(&gesture, target, modifiers)?;
        if let Gesture::Resize { grabbed, .. } = &gesture {
            let resized = change.add.iter().find(|n| key_of(n) == key_of(grabbed));
            if let Some(note) = resized {
                self.draw_length = note.length;
            }
        }
        let edit = change.into_edit(target)?;
        Some(Commit {
            edit,
            selection: Some(selection),
        })
    }

    /// What `gesture` changes with the cursor where it is now, and the selection after it.
    fn change(
        &self,
        gesture: &Gesture,
        target: &Target,
        modifiers: Modifiers,
    ) -> Option<(Change, BTreeSet<NoteKey>)> {
        let unit = self.unit(modifiers);
        let dragged = |from: &Point, to: &Point| {
            Ticks(self.view.tick_at(to.x).0 - self.view.tick_at(from.x).0)
        };
        let change = match gesture {
            Gesture::Move {
                grabbed,
                notes,
                from,
                to,
            } => {
                let ticks = note_gesture::move_offset(grabbed, dragged(from, to), unit);
                let keys = self.view.key_at(to.y) - self.view.key_at(from.y);
                let change = note_gesture::moved(target, notes, ticks, keys);
                let selection = change.added_keys();
                return Some((change, selection));
            }
            Gesture::Resize {
                grabbed,
                notes,
                from,
                to,
            } => {
                let step = self.view.step;
                let delta = note_gesture::resize_offset(grabbed, dragged(from, to), unit, step);
                note_gesture::resized(target, notes, delta, unit, step)
            }
            Gesture::Velocity { notes, steps } => {
                note_gesture::with_velocity(target, notes, *steps)
            }
            Gesture::Paint { painted, .. } => {
                let painted: Vec<(Note, u8)> = painted.values().cloned().collect();
                let change = note_ops::with_velocities(target, &painted);
                let selection = painted.iter().map(|(note, _)| key_of(note)).collect();
                return Some((change, selection));
            }
            Gesture::Ramp { from, to } => {
                let lane = self.lane_height;
                // A stem is drawn from its note's start: an end on a stem's middle is that
                // note's tick.
                // Rounded, not floored: the x of a tick read back in single precision may lie a
                // hair left of it, a tick short.
                let end = |at: &Point| {
                    let x = at.x - piano_layout::STEM_WIDTH / 2.0;
                    (
                        self.view.tick_nearest(x),
                        piano_layout::lane_steps(at.y, lane),
                    )
                };
                note_ops::ramped(target, &self.acted_on(target), end(from), end(to))
            }
            Gesture::Box { .. } | Gesture::Pending { .. } => return None,
        };
        // Resizing and velocity keep every key, so the selection stays as it is.
        Some((change, self.selection.clone()))
    }

    /// The wheel over the grid: keys, Shift (or a wheel across, a trackpad's) for time, Ctrl
    /// to zoom around the cursor. One step per wheel event, whatever its size.
    fn scroll(
        &mut self,
        target: Option<&Target>,
        at: Point,
        delta: mouse::ScrollDelta,
        modifiers: Modifiers,
    ) {
        let (mouse::ScrollDelta::Lines { x, y } | mouse::ScrollDelta::Pixels { x, y }) = delta;
        let (across, notch) = scroll::wheel(x, y);
        if notch == 0.0 {
            return;
        }
        let up = notch > 0.0;
        let end = target.map_or(self.view.bar(), notes_end);
        if modifiers.command() && !across {
            let zoom = if up {
                (self.view.zoom + 1).min(ZOOM_LEVELS.len() - 1)
            } else {
                self.view.zoom.saturating_sub(1)
            };
            // Keep the tick under the cursor where it is: the new scroll puts it back under x.
            let under = self.view.tick_at(at.x);
            let unscrolled = View {
                zoom,
                scroll: Ticks(0),
                ..self.view
            };
            self.view.zoom = zoom;
            self.view.scroll = Ticks((under.0 - unscrolled.tick_at(at.x).0).max(0));
        } else if modifiers.shift() || across {
            let beat = self.view.beat().0;
            let moved = self.view.scroll.0 + if up { -beat } else { beat };
            self.view.scroll = Ticks(moved.max(0));
        } else {
            let rows = if up { KEYS_PER_NOTCH } else { -KEYS_PER_NOTCH };
            self.view.top = self.view.top_moved(rows, self.grid_size.height);
        }
        self.clamp_scroll(end);
    }

    fn clamp_scroll(&mut self, pattern_end: Ticks) {
        let most = piano_layout::max_scroll(&self.view, pattern_end, self.grid_size.width);
        self.view.scroll = Ticks(self.view.scroll.0.clamp(0, most.0));
    }
}

/// Where `target`'s pattern ends, or its last note as drawn (any channel's) when that is later:
/// a pattern made shorter keeps its notes past its end in reach.
fn notes_end(target: &Target) -> Ticks {
    let step = dawsome_engine::time::step_to_ticks(1, target.ppq);
    let last = target
        .notes
        .iter()
        .map(|note| note.start.0 + drawn_length(note, step).0)
        .max()
        .unwrap_or(0);
    Ticks(target.end().0.max(last))
}

#[cfg(test)]
mod tests {
    use dawsome_engine::project::{ChannelId, PatternId};

    use super::*;

    fn note(channel: u32, tick: i64, length: i64, velocity: f32, key: u8) -> Note {
        Note {
            channel: ChannelId(channel),
            start: Ticks(tick),
            length: Ticks(length),
            velocity,
            key,
        }
    }

    fn p3_notes() -> Vec<Note> {
        vec![
            note(1, 0, 48, 0.78125, 60),
            note(1, 96, 96, 0.5, 64),
            note(2, 192, 0, 1.0, 60),
            note(2, 288, 0, 1.0, 60),
        ]
    }

    fn target(notes: &[Note]) -> Target<'_> {
        Target {
            pattern: PatternId(1),
            length_bars: 1,
            ppq: 96,
            channel: ChannelId(1),
            notes,
        }
    }

    fn opened(notes: &[Note]) -> Roll {
        let mut roll = Roll::new(Ticks(24), Size::new(1000.0, 340.0));
        roll.show(Some(&target(notes)));
        roll
    }

    fn press(at: (f32, f32)) -> RollMessage {
        RollMessage::GridPressed {
            at: Point::new(at.0, at.1),
            button: mouse::Button::Left,
            double: false,
        }
    }

    fn release(at: (f32, f32)) -> RollMessage {
        RollMessage::GridReleased(Point::new(at.0, at.1))
    }

    const NONE: Modifiers = Modifiers::empty();

    // The highest lead key is 64, so the roll opens with 69 at the top; with no notes, 71.
    #[test]
    fn the_roll_opens_five_keys_above_the_highest_note() {
        let notes = p3_notes();
        assert_eq!(opened(&notes).view.top, 69);
        let empty = Target {
            channel: ChannelId(9),
            ..target(&notes)
        };
        let mut roll = Roll::new(Ticks(24), Size::new(1000.0, 340.0));
        roll.show(Some(&empty));
        assert_eq!(roll.view.top, 71);
    }

    #[test]
    fn pressing_a_note_selects_it_and_clicking_it_adopts_its_length() {
        let notes = p3_notes();
        let mut roll = opened(&notes);
        assert_eq!(
            roll.update(press((120.0, 66.0)), Some(&target(&notes)), NONE),
            None
        );
        assert!(roll.grid_held());
        assert_eq!(
            roll.update(release((120.0, 66.0)), Some(&target(&notes)), NONE),
            None
        );
        assert_eq!(roll.selection, BTreeSet::from([key_of(&notes[1])]));
        assert_eq!(roll.draw_length, Ticks(96));
        // Ctrl+press toggles membership and starts no drag.
        roll.update(press((56.0, 114.0)), Some(&target(&notes)), Modifiers::CTRL);
        assert!(!roll.grid_held());
        assert_eq!(roll.selection.len(), 2);
        roll.update(press((56.0, 114.0)), Some(&target(&notes)), Modifiers::CTRL);
        assert_eq!(roll.selection, BTreeSet::from([key_of(&notes[1])]));
    }

    // A drag shows exactly what its release commits.
    #[test]
    fn a_drag_previews_what_its_release_commits() {
        let notes = p3_notes();
        let mut roll = opened(&notes);
        roll.update(press((56.0, 114.0)), Some(&target(&notes)), NONE);
        roll.update(
            RollMessage::GridDragged(Point::new(76.0, 102.0)),
            None,
            NONE,
        );
        let (change, selection) = roll.preview(&target(&notes), NONE).expect("a move is held");
        assert_eq!(change.add, [note(1, 24, 48, 0.78125, 61)]);
        assert_eq!(selection, BTreeSet::from([(ChannelId(1), Ticks(24), 61)]));
        let commit = roll
            .update(release((76.0, 102.0)), Some(&target(&notes)), NONE)
            .expect("the move commits");
        assert_eq!(
            commit.edit,
            Edit::ReplaceNotes {
                pattern: PatternId(1),
                remove: change.remove,
                add: change.add,
            }
        );
        assert_eq!(commit.selection, Some(selection));
        assert!(!roll.grid_held());
    }

    // x 56 → 83 is tick 12 → 52: 40 ticks, nearer 48 than 24. A lands on 48.
    #[test]
    fn a_move_lands_on_the_nearest_snap_line_not_the_one_below() {
        let notes = p3_notes();
        let mut roll = opened(&notes);
        roll.update(press((56.0, 114.0)), Some(&target(&notes)), NONE);
        let commit = roll
            .update(release((83.0, 114.0)), Some(&target(&notes)), NONE)
            .expect("commits");
        let Edit::ReplaceNotes { add, .. } = commit.edit else {
            panic!("a move is one ReplaceNotes: {:?}", commit.edit);
        };
        assert_eq!(add[0].start, Ticks(48));
    }

    // Alt snaps to single ticks: 12 → 42 moves A by exactly 30.
    #[test]
    fn alt_bypasses_the_snap() {
        let notes = p3_notes();
        let mut roll = opened(&notes);
        roll.update(press((56.0, 114.0)), Some(&target(&notes)), Modifiers::ALT);
        let commit = roll
            .update(
                release((76.0, 114.0)),
                Some(&target(&notes)),
                Modifiers::ALT,
            )
            .expect("commits");
        let Edit::ReplaceNotes { add, .. } = commit.edit else {
            panic!("a move is one ReplaceNotes: {:?}", commit.edit);
        };
        assert_eq!(add[0].start, Ticks(30));
    }

    #[test]
    fn a_ctrl_drag_from_empty_grid_selects_what_its_box_meets() {
        let notes = p3_notes();
        let mut roll = opened(&notes);
        roll.update(press((40.0, 50.0)), Some(&target(&notes)), Modifiers::CTRL);
        assert!(roll.select_box().is_some());
        roll.update(
            release((200.0, 130.0)),
            Some(&target(&notes)),
            Modifiers::CTRL,
        );
        let lead: BTreeSet<NoteKey> = notes[..2].iter().map(key_of).collect();
        assert_eq!(roll.selection, lead, "the kicks are ghosts, never selected");
    }

    // A6: Fold keeps rows for the lead's keys (60, 64) alone, so 64 goes on top and A sits on
    // the second row (y 12..24); with C major chosen the fold keeps the scale's keys too (61,
    // C#, has no row, 62 has); unfolded every key has its row again.
    #[test]
    fn fold_keeps_rows_for_the_notes_keys_and_the_scale() {
        use crate::ui::scale::{Root, ScaleMode};
        let notes = p3_notes();
        let mut roll = opened(&notes);
        roll.update(RollMessage::ToggleFold, Some(&target(&notes)), NONE);
        assert!(roll.fold);
        assert_eq!(roll.view.top, 64);
        assert_eq!(roll.view.row_top(60), 12.0);
        let a = piano_layout::note_under(&roll.view, target(&notes).channel_notes(), 56.0, 18.0);
        assert_eq!(a.map(|(note, _)| note.start), Some(Ticks(0)));
        roll.update(
            RollMessage::ScaleSelected(ScaleMode::Major),
            Some(&target(&notes)),
            NONE,
        );
        roll.update(
            RollMessage::RootSelected(Root(0)),
            Some(&target(&notes)),
            NONE,
        );
        assert!(!roll.view.rows.shows(61) && roll.view.rows.shows(62));
        roll.update(RollMessage::ToggleFold, Some(&target(&notes)), NONE);
        assert_eq!(roll.view.rows, Rows::ALL);
        assert_eq!(roll.scale.mode, ScaleMode::Major, "the scale stays chosen");
    }

    // A ramp whose ends lie on the middles of two stems of notes off the step
    // grid reaches both notes at 8 and 32 px a step (and the default 16): the far end's note
    // takes the end's value. Each pair 1 to 40 ticks off the grid, a beat apart.
    #[test]
    fn a_ramp_ending_on_an_unquantized_stem_reaches_its_note() {
        for zoom in [2, 4, 6] {
            for off in 1..=40 {
                let notes = vec![note(1, off, 10, 0.5, 60), note(1, off + 97, 10, 0.5, 62)];
                let mut roll = opened(&notes);
                roll.view.zoom = zoom;
                let middle =
                    |tick: i64| roll.view.x_of(Ticks(tick)) + piano_layout::STEM_WIDTH / 2.0;
                let (from, to) = (middle(off), middle(off + 97));
                let lane = roll.lane_height();
                roll.update(
                    RollMessage::LanePressed(Point::new(from, lane - 0.2)),
                    Some(&target(&notes)),
                    Modifiers::ALT,
                );
                let commit = roll
                    .update(
                        RollMessage::LaneReleased(Point::new(to, 0.0)),
                        Some(&target(&notes)),
                        Modifiers::ALT,
                    )
                    .unwrap_or_else(|| panic!("zoom {zoom}, off {off}: a ramp"));
                let Edit::ReplaceNotes { add, .. } = commit.edit else {
                    panic!("{:?}", commit.edit);
                };
                let set: Vec<(i64, f32)> = add.iter().map(|n| (n.start.0, n.velocity)).collect();
                assert_eq!(set, [(off, 0.0), (off + 97, 1.0)], "zoom {zoom}, off {off}");
            }
        }
    }

    // Chop cuts at the snap: at a beat (96) B (96..192) is one beat and stays, A (0..48) too;
    // with Snap None it cuts at the step rather than every tick: A into 0..24 and 24..48.
    #[test]
    fn chop_cuts_at_the_snap_and_at_the_step_without_one() {
        let notes = p3_notes();
        let mut roll = opened(&notes);
        roll.snap = Snap::Beat;
        assert_eq!(roll.operate(&target(&notes), NoteEdit::Chop), None);
        roll.snap = Snap::None;
        roll.selection = BTreeSet::from([key_of(&notes[0])]);
        let commit = roll
            .operate(&target(&notes), NoteEdit::Chop)
            .expect("chops");
        let Edit::ReplaceNotes { add, .. } = commit.edit else {
            panic!("{:?}", commit.edit);
        };
        assert_eq!(
            add,
            [note(1, 0, 24, 0.78125, 60), note(1, 24, 24, 0.78125, 60)]
        );
    }

    // A7: a new note takes the last drawn length: drawn at 408 a step long (24, x 320..336),
    // its edge pressed 2 px inside (x 334) and dragged 32 px (48 ticks) to x 366, so 72 long;
    // the next click draws 72 long.
    #[test]
    fn a_new_note_takes_the_last_drawn_length() {
        let notes = p3_notes();
        let mut roll = opened(&notes);
        let commit = roll
            .update(press((330.0, 150.0)), Some(&target(&notes)), NONE)
            .or_else(|| roll.update(release((330.0, 150.0)), Some(&target(&notes)), NONE))
            .expect("draws");
        let drawn = note(1, 408, 24, 0.78125, 57);
        let Edit::Transaction(edits) = &commit.edit else {
            panic!("the pattern grows: {:?}", commit.edit);
        };
        assert!(matches!(&edits[0], Edit::ReplaceNotes { add, .. } if *add == [drawn.clone()]));
        let mut after = notes.clone();
        after.push(drawn);
        after.sort_by_key(|note| (note.start, note.channel, note.key));
        let grown = Target {
            length_bars: 2,
            ..target(&after)
        };
        roll.update(press((334.0, 150.0)), Some(&grown), NONE);
        roll.update(release((366.0, 150.0)), Some(&grown), NONE);
        assert_eq!(roll.draw_length, Ticks(72));
        let next = roll
            .update(press((450.0, 150.0)), Some(&grown), NONE)
            .or_else(|| roll.update(release((450.0, 150.0)), Some(&grown), NONE))
            .expect("draws");
        let Edit::ReplaceNotes { add, .. } = next.edit else {
            panic!("{:?}", next.edit);
        };
        assert_eq!(add[0].length, Ticks(72));
    }

    // A3, by hand: a press on empty grid at x 330 (tick 423, down to 408) that moves 3 px and
    // is let go of draws there (at the press, not the release); one that moves 5 px is a box
    // and draws nothing; a box with Shift adds what it meets to the selection.
    #[test]
    fn a_press_on_empty_grid_draws_when_let_go_of_unless_it_moves_into_a_box() {
        let notes = p3_notes();
        let mut roll = opened(&notes);
        let drag = |x: f32, y: f32| RollMessage::GridDragged(Point::new(x, y));
        assert_eq!(
            roll.update(press((330.0, 114.0)), Some(&target(&notes)), NONE),
            None
        );
        assert_eq!(roll.pending_draw(), Some(Point::new(330.0, 114.0)));
        roll.update(drag(333.0, 114.0), Some(&target(&notes)), NONE);
        let commit = roll
            .update(release((333.0, 114.0)), Some(&target(&notes)), NONE)
            .expect("draws");
        let Edit::Transaction(edits) = commit.edit else {
            panic!("past the end, the pattern grows: {:?}", commit.edit);
        };
        assert_eq!(
            edits[0],
            Edit::ReplaceNotes {
                pattern: PatternId(1),
                remove: vec![],
                add: vec![note(1, 408, 24, 0.78125, 60)],
            }
        );
        roll.update(press((300.0, 30.0)), Some(&target(&notes)), NONE);
        roll.update(drag(305.0, 30.0), Some(&target(&notes)), NONE);
        assert!(roll.select_box().is_some(), "past the slop: a box");
        assert_eq!(
            roll.update(release((305.0, 30.0)), Some(&target(&notes)), NONE),
            None
        );
        assert!(roll.selection.is_empty());
        roll.selection = BTreeSet::from([key_of(&notes[1])]);
        roll.update(
            press((50.0, 100.0)),
            Some(&target(&notes)),
            Modifiers::SHIFT,
        );
        roll.update(drag(70.0, 125.0), Some(&target(&notes)), Modifiers::SHIFT);
        roll.update(
            release((70.0, 125.0)),
            Some(&target(&notes)),
            Modifiers::SHIFT,
        );
        let both: BTreeSet<NoteKey> = notes[..2].iter().map(key_of).collect();
        assert_eq!(roll.selection, both, "Shift adds A to B");
    }

    // A5: a right press opens the notes menu where it was (the menu's Delete then takes out
    // what it selected): on B, unselected, B alone becomes the selection; on empty grid the
    // selection stays; on a note already among the selection, the selection stays whole.
    #[test]
    fn a_right_press_opens_the_notes_menu_on_what_it_pressed() {
        let notes = p3_notes();
        let mut roll = opened(&notes);
        let right = |x: f32, y: f32| RollMessage::GridPressed {
            at: Point::new(x, y),
            button: mouse::Button::Right,
            double: false,
        };
        assert_eq!(
            roll.update(right(120.0, 66.0), Some(&target(&notes)), NONE),
            None
        );
        assert_eq!(roll.menu, Some(Point::new(120.0, 66.0)));
        assert_eq!(roll.selection, BTreeSet::from([key_of(&notes[1])]));
        let commit = roll.delete_selected(&target(&notes)).expect("deletes");
        assert_eq!(
            commit.edit,
            Edit::ReplaceNotes {
                pattern: PatternId(1),
                remove: vec![notes[1].clone()],
                add: vec![],
            }
        );
        assert_eq!(
            roll.update(right(330.0, 66.0), Some(&target(&notes)), NONE),
            None
        );
        assert_eq!(roll.menu, Some(Point::new(330.0, 66.0)));
        assert_eq!(roll.selection, BTreeSet::from([key_of(&notes[1])]));
        roll.select_all(&target(&notes));
        roll.update(right(56.0, 114.0), Some(&target(&notes)), NONE);
        assert_eq!(roll.selection.len(), 2, "pressed among the selection");
        roll.update(right(10.0, 114.0), Some(&target(&notes)), NONE);
        assert_eq!(
            roll.menu,
            Some(Point::new(56.0, 114.0)),
            "the key column opens none"
        );
    }

    #[test]
    fn the_wheel_scrolls_keys_time_and_zoom() {
        let notes = p3_notes();
        let mut roll = opened(&notes);
        let wheel = |y| RollMessage::GridScrolled {
            at: Point::new(304.0, 100.0),
            delta: mouse::ScrollDelta::Lines { x: 0.0, y },
        };
        roll.update(wheel(1.0), Some(&target(&notes)), NONE);
        assert_eq!(roll.view.top, 72);
        roll.update(wheel(-1.0), Some(&target(&notes)), Modifiers::SHIFT);
        assert_eq!(roll.view.scroll, Ticks(96), "one beat later");
        roll.update(wheel(1.0), Some(&target(&notes)), Modifiers::SHIFT);
        assert_eq!(roll.view.scroll, Ticks(0));
        // x 304 is tick 384 at 16 px per step; zoomed in to 24 px it must stay under x 304.
        roll.update(wheel(1.0), Some(&target(&notes)), Modifiers::CTRL);
        assert_eq!(roll.view.px_per_step(), 24.0);
        assert_eq!(roll.view.tick_at(304.0), Ticks(384));
        roll.update(wheel(-1.0), Some(&target(&notes)), Modifiers::CTRL);
        roll.update(wheel(-1.0), Some(&target(&notes)), Modifiers::CTRL);
        assert_eq!(roll.view.px_per_step(), 12.0);
        assert_eq!(roll.view.scroll, Ticks(0), "never before tick 0");
    }

    #[test]
    fn refreshing_drops_selected_notes_that_are_gone() {
        let notes = p3_notes();
        let mut roll = opened(&notes);
        roll.select_all(&target(&notes));
        assert_eq!(roll.selection.len(), 2);
        let fewer = &notes[1..];
        roll.refresh(Some(&target(fewer)));
        assert_eq!(roll.selection, BTreeSet::from([key_of(&notes[1])]));
    }

    // The handle drags the lane taller by what it went up, stops at the
    // tallest lane, and a stem press reads velocity on the lane's own height: y 20 of the default
    // 64 px lane is 88, y 20 of a 94 px lane (64 dragged up 30) is round(74·128/94) = 101.
    #[test]
    fn the_handle_resizes_the_lane_and_the_lane_keeps_its_scale() {
        let notes = p3_notes();
        let mut roll = opened(&notes);
        assert_eq!(roll.lane_height(), piano_layout::LANE_HEIGHT);
        roll.update(
            RollMessage::LanePressed(Point::new(50.0, 20.0)),
            Some(&target(&notes)),
            NONE,
        );
        assert_eq!(roll.dragged_velocity(), Some(88));
        roll.update(
            RollMessage::LaneReleased(Point::new(50.0, 20.0)),
            Some(&target(&notes)),
            NONE,
        );
        roll.update(RollMessage::HandlePressed(400.0), None, NONE);
        assert!(roll.handle_held());
        roll.update(RollMessage::HandleDragged(370.0), None, NONE);
        assert_eq!(roll.lane_height(), 94.0);
        roll.update(RollMessage::HandleReleased, None, NONE);
        assert!(!roll.handle_held());
        roll.update(
            RollMessage::LanePressed(Point::new(50.0, 20.0)),
            Some(&target(&notes)),
            NONE,
        );
        assert_eq!(roll.dragged_velocity(), Some(101));
        roll.update(RollMessage::HandleDragged(0.0), None, NONE);
        assert_eq!(
            roll.lane_height(),
            94.0,
            "a move with the handle let go of changes nothing"
        );
        let tall = opened(&notes).with_lane_height(1000.0);
        assert_eq!(tall.lane_height(), piano_layout::LANE_MAX_HEIGHT);
    }

    // P2, by hand on p3_roll at 16 px a step in a 1000 px grid (1428 ticks in view): the bar
    // reaches 4 bars past the one-bar pattern, 1920; dragged to the end it shows 492..1920. A
    // note moved past a pattern made shorter keeps the reach past it. A wheel across (a
    // trackpad's) scrolls time a beat, as Shift+wheel does.
    #[test]
    fn the_bar_scrolls_the_grid_within_its_reach() {
        let notes = p3_notes();
        let mut roll = opened(&notes);
        let span = roll.span(Some(&target(&notes)));
        assert_eq!((span.content, span.view), (1920.0, 1428.0));
        let to = |offset| {
            RollMessage::Bar(BarMessage::To {
                offset,
                held: false,
            })
        };
        roll.update(to(5000.0), Some(&target(&notes)), NONE);
        assert_eq!(roll.view.scroll, Ticks(492), "kept to the reach");
        roll.update(to(100.4), Some(&target(&notes)), NONE);
        assert_eq!(roll.view.scroll, Ticks(100));
        let mut late = notes.clone();
        late.push(note(1, 2400, 96, 0.5, 62));
        let span = roll.span(Some(&target(&late)));
        assert_eq!(span.content, (2496 + 4 * 384) as f64);
        roll.update(to(0.0), Some(&target(&notes)), NONE);
        let across = RollMessage::GridScrolled {
            at: Point::new(304.0, 100.0),
            delta: mouse::ScrollDelta::Pixels { x: -3.0, y: 0.0 },
        };
        roll.update(across, Some(&target(&notes)), NONE);
        assert_eq!(roll.view.scroll, Ticks(96), "a beat later");
        assert_eq!(roll.view.top, 69, "the keys stay");
    }

    // A trackpad's swipe across with a little down
    // in it (3 px left, 0.4 px down) scrolls time a beat and leaves the keys, as a swipe with
    // none does; a swipe mostly up or down scrolls the keys and leaves time.
    #[test]
    fn a_swipe_mostly_across_scrolls_time() {
        let notes = p3_notes();
        let mut roll = opened(&notes);
        let swipe = |x, y| RollMessage::GridScrolled {
            at: Point::new(304.0, 100.0),
            delta: mouse::ScrollDelta::Pixels { x, y },
        };
        roll.update(swipe(-3.0, 0.4), Some(&target(&notes)), NONE);
        assert_eq!((roll.view.scroll, roll.view.top), (Ticks(96), 69));
        roll.update(swipe(0.4, -3.0), Some(&target(&notes)), NONE);
        assert_eq!(roll.view.scroll, Ticks(96), "time stays");
        assert_ne!(roll.view.top, 69, "the keys move");
    }

    // P2: the playhead jumps the view a page, by hand. Zoomed to 48 px a step (two pixels a tick,
    // 476 ticks in view) on a 4-bar pattern: at tick 400 the view stays at 0; at 476 it jumps
    // to bar 2's start, 384; the pattern's wrap back to 0 jumps it back. A held thumb stops
    // the jumps until it is let go of.
    #[test]
    fn the_playhead_pages_the_view_while_it_follows() {
        let notes = p3_notes();
        let long = Target {
            length_bars: 4,
            ..target(&notes)
        };
        let mut roll = opened(&notes);
        roll.view.zoom = 7;
        assert_eq!(roll.span(Some(&long)).view, 476.0);
        assert!(!roll.follow(Some(&long), Ticks(400)));
        assert!(roll.follow(Some(&long), Ticks(476)));
        assert_eq!(roll.view.scroll, Ticks(384));
        assert!(
            roll.follow(Some(&long), Ticks(4 * 384 + 10)),
            "the pattern wrapped"
        );
        assert_eq!(roll.view.scroll, Ticks(0));
        let held = RollMessage::Bar(BarMessage::To {
            offset: 0.0,
            held: true,
        });
        roll.update(held, Some(&long), NONE);
        assert!(!roll.follow(Some(&long), Ticks(900)), "the thumb is held");
        roll.update(RollMessage::Bar(BarMessage::LetGo), Some(&long), NONE);
        assert!(roll.follow(Some(&long), Ticks(900)));
        assert_eq!(roll.view.scroll, Ticks(768));
        assert!(!roll.follow(None, Ticks(0)), "no channel, no view");
    }
}
