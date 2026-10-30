//! The playlist's UI state: what the panel shows (scroll, zoom, top track), the snap, the clip
//! selection, the gesture in progress and the track being renamed. It turns the canvases'
//! messages into edits through [`clip_gesture`] and never changes the document itself. The
//! selection is UI state, never an edit: a set of clip IDs.
//!
//! Every clip edit commits on release, as one edit (one undo step); a held drag only shows
//! outlines where its release would land. A drag along time held past the lanes' left or right
//! edge scrolls the view towards it on each of the window's ticks
//! ([`Arranger::scroll_at_edge`]), and counts from the place pressed, so its target stays under
//! the pointer as the view moves. Audio clips also trim at both edges, stretch with an Alt drag
//! of the right edge, and take a gain drag on their readout; automation clips take presses on
//! their points and tension handles.

use std::collections::BTreeSet;

use dawsome_engine::edit::Edit;
use dawsome_engine::project::{
    max_song_tick, AutomationId, AutomationPoint, Clip, ClipId, ClipItem, Curve, LoopRange,
    PatternId, Project, StretchMode, TrackId, MAX_TRACKS,
};
use dawsome_engine::time::{self, Ticks};
use iced::keyboard::Modifiers;
use iced::{mouse, Point, Rectangle, Size};

use crate::app::ClipLink;
use crate::ui::clip_gesture::{self, ClipSnap};
use crate::ui::clips;
use crate::ui::hint::{ClipRow, TrackRow};
use crate::ui::note_gesture::{snap_down, snap_nearest};
use crate::ui::playlist::{automation_handles, automation_points};
use crate::ui::playlist_layout::{
    self, ClipPart, Grid, HeaderHit, Lanes, LoopEdge, RulerBand, EDGE_SCROLL_INSET,
    GAIN_READOUT_WIDTH, LANE_HEIGHT, POINT_REACH, TITLE_HEIGHT, TRACK_HEADER_WIDTH, ZOOM_LEVELS,
};
use crate::ui::scroll::{self, BarMessage, Span};

/// The tension "Exponential" sets from the curve menu.
const MENU_TENSION: f32 = 0.5;

/// The curve menu a right press on a tension handle opens, in order: each row's words and the
/// curve it gives the segment.
pub const CURVE_MENU: [(&str, Curve); 4] = [
    ("Linear", Curve::Linear),
    ("Hold", Curve::Hold),
    ("Smooth", Curve::Smooth),
    ("Exponential", Curve::Exp(MENU_TENSION)),
];

/// Decibels of gain one pixel of drag on a clip's readout moves, and with Shift.
const GAIN_DB_PER_PIXEL: f32 = 0.1;
const FINE_GAIN_DB_PER_PIXEL: f32 = 0.01;

/// Tracks the wheel scrolls per notch.
const TRACKS_PER_NOTCH: i64 = 1;
/// Pixels of timeline Shift+wheel scrolls per notch, at any zoom.
const PIXELS_PER_NOTCH: f32 = 64.0;

/// What the playlist's canvases report. Points are in the reporting canvas's coordinates (see
/// [`playlist_layout`]), so a test that sends them drives the real mapping.
#[derive(Debug, Clone, PartialEq)]
pub enum PlaylistMessage {
    /// A press on the tracks: `double` when it repeats the last press quickly in place.
    TracksPressed {
        at: Point,
        button: mouse::Button,
        double: bool,
    },
    /// The cursor moved while a track gesture is held.
    TracksDragged(Point),
    TracksReleased(Point),
    TracksScrolled {
        at: Point,
        delta: mouse::ScrollDelta,
    },
    /// The tracks canvas was laid out at a new size.
    TracksResized(Size),
    /// The whole panel was laid out here, in window coordinates.
    Placed(Rectangle),
    /// While a file is dragged from the browser: the cursor over the tracks, or off them.
    BrowserHover(Option<Point>),
    RulerPressed {
        at: Point,
        button: mouse::Button,
    },
    RulerDragged(Point),
    RulerReleased(Point),
    SnapSelected(ClipSnap),
    ZoomIn,
    ZoomOut,
    /// The header's Fit switch: the lanes fit the track area again, or keep the height they
    /// have now.
    ToggleFit,
    RenameEdited(String),
    RenameSubmitted,
    /// A row of the open track menu.
    TrackMenu(TrackRow),
    /// A row of the open clip menu.
    ClipMenu(ClipRow),
    /// A row of the open curve menu: the curve its segment takes.
    CurveChosen(Curve),
    /// The clip properties strip's Stretch choice: none, resample or time-stretch.
    ClipMode(Option<StretchMode>),
    /// The strip's pitch stepper: this many semitones up (or down).
    ClipPitch(i8),
    /// The strip's close button.
    CloseProperties,
    /// The timeline's scroll bar asks for another view.
    Bar(BarMessage),
}

/// What a playlist gesture asks the app to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Apply this as one undo step.
    Edit(Edit),
    /// Move the song position here.
    Seek(Ticks),
    /// A track name went into editing: give its text box the keyboard.
    FocusRename,
    /// A track's arm light: takes land on that track.
    Arm(TrackId),
    /// An Alt drag of an audio clip's right edge ended here: stretch the clip's file to end
    /// there. The app knows the file's length, which the edit needs.
    Stretch { clip: ClipId, end: Ticks },
    /// The properties strip chose how the clip's file is fitted to it; the app knows the
    /// file's length, which the edit needs.
    StretchMode {
        clip: ClipId,
        mode: Option<StretchMode>,
    },
    /// A double press on a pattern clip's body: show its pattern in the piano roll.
    OpenInRoll(ClipId),
    /// A double press on an audio clip's body: its channel's sound in the sample editor, the
    /// clip's window shown (the properties strip opens beside it).
    EditSample(ClipId),
    /// A clip menu row: unlink the clip or link it back (the app refuses it under a take).
    Link(ClipLink),
}

/// A drag in progress, as it was pressed.
#[derive(Debug, Clone, PartialEq)]
enum Gesture {
    Move {
        ids: Vec<ClipId>,
        from: Point,
        to: Point,
    },
    Resize {
        id: ClipId,
        from: Point,
        to: Point,
    },
    /// Ctrl+drag from empty lane space.
    Box {
        from: Point,
        to: Point,
    },
    /// A new loop range, from the pressed tick to the one under the cursor.
    DrawLoop {
        from: Ticks,
        to: Point,
    },
    MoveLoopEdge {
        range: LoopRange,
        edge: LoopEdge,
        to: Point,
    },
    /// An audio clip's left edge: its start and frame offset move together.
    TrimStart {
        id: ClipId,
        from: Point,
        to: Point,
    },
    /// An audio clip's right edge.
    TrimEnd {
        id: ClipId,
        from: Point,
        to: Point,
    },
    /// An audio clip's right edge with Alt held: its file stretches to the new length.
    Stretch {
        id: ClipId,
        from: Point,
        to: Point,
    },
    /// Up and down on an audio clip's gain readout.
    Gain {
        id: ClipId,
        gain: f32,
        from: Point,
        to: Point,
    },
    /// An automation point, by its data tick, in clip `id`.
    MovePoint {
        id: ClipId,
        tick: Ticks,
        to: Point,
    },
    /// The tension handle of the segment that starts at data tick `tick`, in clip `id`.
    Tension {
        id: ClipId,
        tick: Ticks,
        to: Point,
    },
}

/// A press on a clip: which clip and part, where, with which button, whether it repeats the
/// last press, and whether Alt is held.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ClipPress {
    id: ClipId,
    part: ClipPart,
    at: Point,
    button: mouse::Button,
    double: bool,
    alt: bool,
}

/// The curve menu, open over a tension handle: the automation, the segment's first point and
/// where the press that opened it was (tracks-canvas coordinates; the menu opens there, on the
/// window's top layer).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurveMenu {
    pub automation: AutomationId,
    pub tick: Ticks,
    pub at: Point,
}

/// What a held drag would do if released now, for the canvases to draw.
#[derive(Debug, Clone, PartialEq)]
pub enum Preview {
    /// Outlines of where the clips would land, as (start, end, track index).
    Clips(Vec<(Ticks, Ticks, usize)>),
    /// The box being dragged out, corner to corner.
    Box(Point, Point),
    /// The loop range the ruler would set.
    Loop(LoopRange),
}

/// The playlist's state between frames.
#[derive(Debug, Clone)]
pub struct Arranger {
    pub grid: Grid,
    pub snap: ClipSnap,
    pub selection: BTreeSet<ClipId>,
    /// The track being renamed, and the name typed so far.
    pub rename: Option<(TrackId, String)>,
    /// The curve menu, while it is open.
    pub curve_menu: Option<CurveMenu>,
    /// The track menu, while it is open: its track, and where the press that opened it was
    /// (tracks-canvas coordinates).
    pub track_menu: Option<(TrackId, Point)>,
    /// A pattern clip's menu, while it is open: its clip, and where the Ctrl+right press that
    /// opened it was (tracks-canvas coordinates).
    pub clip_menu: Option<(ClipId, Point)>,
    /// Where a file dragged from the browser is over the tracks.
    pub hover: Option<Point>,
    /// The audio clip whose properties strip the header shows (a double press opens it).
    pub properties: Option<ClipId>,
    gesture: Option<Gesture>,
    /// The view as it was at the press that began the held gesture: a drag counts from the
    /// place pressed, wherever the view has moved since.
    press_view: Grid,
    /// The held drag has scrolled the view past an edge: Follow waits until it is let go of,
    /// as it does for the scroll bar's thumb.
    edge_scrolled: bool,
    /// The tracks canvas's size, as it last reported it.
    tracks_size: Size,
    /// Where the whole panel is in the window, as its drop layer last reported it: what a file
    /// dropped from the desktop is measured against.
    panel: Rectangle,
    /// The scroll bar's thumb is held: the playhead does not move the view meanwhile.
    bar_held: bool,
    /// Whether the lanes fit the tracks into the track area or keep a height chosen; the
    /// height now is [`Grid::lane`], which [`Arranger::clamp`] fits.
    lanes: Lanes,
}

impl Arranger {
    /// A playlist at the default zoom and snap, scrolled to the start, for a project at `ppq`,
    /// drawing its tracks into an area of `tracks_size`, its lanes `lanes` (fitted once it has
    /// seen the project: [`Arranger::refresh`]).
    pub fn new(ppq: u16, tracks_size: Size, lanes: Lanes) -> Self {
        let lane = match lanes {
            Lanes::Fit => LANE_HEIGHT,
            Lanes::Fixed(height) => height,
        };
        let grid = Grid {
            scroll: Ticks(0),
            zoom: playlist_layout::DEFAULT_ZOOM,
            bar: time::bars_to_ticks(1, ppq),
            top: 0,
            lane,
        };
        Self {
            grid,
            snap: ClipSnap::default(),
            selection: BTreeSet::new(),
            rename: None,
            curve_menu: None,
            track_menu: None,
            clip_menu: None,
            hover: None,
            properties: None,
            gesture: None,
            press_view: grid,
            edge_scrolled: false,
            tracks_size,
            panel: Rectangle::default(),
            bar_held: false,
            lanes,
        }
    }

    /// Whether the lanes fit the track area or keep a height chosen.
    pub fn lanes(&self) -> Lanes {
        self.lanes
    }

    /// The timeline's scroll, as its bar shows it: the content reaches
    /// [`playlist_layout::BARS_PAST_END`] bars past the song's end (its last clip's, or the
    /// loop range's when that ends later).
    pub fn span(&self, project: &Project) -> Span {
        let playlist = &project.playlist;
        let looped = playlist.loop_range.map_or(Ticks(0), |range| range.end);
        let end = playlist.end().max(looped);
        playlist_layout::time_span(&self.grid, end, self.tracks_size.width)
    }

    /// The scroll bar's thumb is no longer held: what its release does, for a bar that went
    /// without one (its panel closed).
    pub fn let_go_of_bar(&mut self) {
        self.bar_held = false;
    }

    /// Opens the curve menu at the first tension handle of `automation`'s clips in view, as a
    /// right press there does: `--curve-menu`, for a screenshot of the menu. Whether there was
    /// one to open it at.
    pub fn open_curve_menu(&mut self, project: &Project, automation: AutomationId) -> bool {
        let Some(data) = project.automation.list.get(&automation) else {
            return false;
        };
        let handle = project
            .playlist
            .clips
            .values()
            .filter(|clip| clip.automation() == Some(automation))
            .find_map(|clip| {
                let body = self.body_rect(project, clip)?;
                automation_handles(&self.grid, clip, data, body)
                    .into_iter()
                    .next()
            });
        let Some(handle) = handle else {
            return false;
        };
        self.curve_menu = Some(CurveMenu {
            automation,
            tick: handle.tick,
            at: handle.at,
        });
        true
    }

    /// While the song plays, the view jumps a page when the song position `at` leaves it,
    /// starting at the bar that holds it; not while the scroll bar's thumb is held, nor while a
    /// drag that scrolled the view past an edge is. Returns whether the view moved.
    pub fn follow(&mut self, project: &Project, at: Ticks) -> bool {
        if self.bar_held || (self.edge_scrolled && self.gesture.is_some()) {
            return false;
        }
        let span = self.span(project);
        let bar = self.grid.bar.0 as f64;
        let Some(offset) = scroll::follow(span.offset, span.view, at.0 as f64, bar) else {
            return false;
        };
        let before = self.grid.scroll;
        self.grid.scroll = Ticks(offset as i64);
        self.clamp(project);
        self.grid.scroll != before
    }

    pub fn tracks_size(&self) -> Size {
        self.tracks_size
    }

    /// Where the panel is in the window, as last reported.
    pub fn panel(&self) -> Rectangle {
        self.panel
    }

    /// Whether a gesture is held on the tracks.
    pub fn tracks_held(&self) -> bool {
        matches!(
            self.gesture,
            Some(
                Gesture::Move { .. }
                    | Gesture::Resize { .. }
                    | Gesture::Box { .. }
                    | Gesture::TrimStart { .. }
                    | Gesture::TrimEnd { .. }
                    | Gesture::Stretch { .. }
                    | Gesture::Gain { .. }
                    | Gesture::MovePoint { .. }
                    | Gesture::Tension { .. }
            )
        )
    }

    /// The scroll one of the window's ticks gives while the held drag has the pointer past the
    /// lanes' left edge or over their last [`EDGE_SCROLL_INSET`] pixels on the right
    /// ([`scroll::edge_step`], in pixels, so it feels alike at every zoom). `None` when the view
    /// stays: no drag along time held, the pointer over the lanes, or the view at its start.
    pub fn edge_scroll(&self, project: &Project) -> Option<Ticks> {
        let at = self.dragged_along_time()?;
        let right = self.tracks_size.width - EDGE_SCROLL_INSET;
        let pixels = scroll::edge_step(at.x, TRACK_HEADER_WIDTH, right);
        let ticks = (f64::from(pixels) * self.grid.bar.0.max(1) as f64
            / f64::from(self.grid.px_per_bar()))
        .round() as i64;
        let most = max_song_tick(project.ppq).0;
        let scroll = (self.grid.scroll.0 + ticks).clamp(0, most);
        (scroll != self.grid.scroll.0).then_some(Ticks(scroll))
    }

    /// One tick of [`Arranger::edge_scroll`]: the view moves towards the edge the pointer is
    /// past, and the held drag's target with it, since the pointer is now over a later (or
    /// earlier) place. Returns whether the view moved.
    pub fn scroll_at_edge(&mut self, project: &Project) -> bool {
        let Some(scroll) = self.edge_scroll(project) else {
            return false;
        };
        self.grid.scroll = scroll;
        self.edge_scrolled = true;
        true
    }

    /// Where the pointer is while a drag along time is held: a clip's move, resize, trim or
    /// stretch, a box, an automation point, a loop drag. Not the gain readout's or a curve's
    /// tension, which go up and down.
    fn dragged_along_time(&self) -> Option<Point> {
        match self.gesture.as_ref()? {
            Gesture::Move { to, .. }
            | Gesture::Resize { to, .. }
            | Gesture::Box { to, .. }
            | Gesture::DrawLoop { to, .. }
            | Gesture::MoveLoopEdge { to, .. }
            | Gesture::TrimStart { to, .. }
            | Gesture::TrimEnd { to, .. }
            | Gesture::Stretch { to, .. }
            | Gesture::MovePoint { to, .. } => Some(*to),
            Gesture::Gain { .. } | Gesture::Tension { .. } => None,
        }
    }

    /// Whether a loop drag is held on the ruler.
    pub fn ruler_held(&self) -> bool {
        matches!(
            self.gesture,
            Some(Gesture::DrawLoop { .. } | Gesture::MoveLoopEdge { .. })
        )
    }

    /// Ends a gesture without committing it.
    pub fn cancel_gesture(&mut self) {
        self.gesture = None;
    }

    /// Esc: ends a rename without committing it, or else selects nothing.
    pub fn escape(&mut self) {
        if self.track_menu.take().is_some() || self.clip_menu.take().is_some() {
            return;
        }
        if self.rename.take().is_none() {
            self.selection.clear();
        }
    }

    /// Brings the state in line with a changed document: selected clips that are gone are
    /// dropped, a rename of a track that is gone ends, and the view stays on the tracks.
    pub fn refresh(&mut self, project: &Project) {
        let clips = &project.playlist.clips;
        self.selection.retain(|id| clips.contains_key(id));
        if self.properties.is_some_and(|id| !clips.contains_key(&id)) {
            self.properties = None;
        }
        if let Some((track, _)) = &self.rename {
            if !project.playlist.tracks.contains_key(track) {
                self.rename = None;
            }
        }
        if let Some((track, _)) = &self.track_menu {
            if !project.playlist.tracks.contains_key(track) {
                self.track_menu = None;
            }
        }
        if self
            .clip_menu
            .is_some_and(|(id, _)| !clips.contains_key(&id))
        {
            self.clip_menu = None;
        }
        self.clamp(project);
    }

    /// What the held drag would do if released now.
    pub fn preview(&self, project: &Project) -> Option<Preview> {
        let unit = self.unit(project);
        let step = time::step_to_ticks(1, project.ppq);
        match self.gesture.as_ref()? {
            Gesture::Move { ids, from, to } => {
                let outlines = self
                    .move_targets(project, ids, *from, *to)
                    .into_iter()
                    .filter_map(|(id, start, index)| {
                        let clip = project.playlist.clips.get(&id)?;
                        Some((start, Ticks(start.0 + clip.length.0), index))
                    })
                    .collect();
                Some(Preview::Clips(outlines))
            }
            Gesture::Resize { id, from, to } => {
                let clip = project.playlist.clips.get(id)?;
                let index = clip_gesture::index_of(project, clip.track)?;
                let length = clip_gesture::resized_length(clip, self.dragged(*from, *to), unit);
                let end = Ticks(clip.start.0 + length.0);
                Some(Preview::Clips(vec![(clip.start, end, index)]))
            }
            Gesture::Box { from, to } => Some(Preview::Box(self.anchored(*from), *to)),
            Gesture::DrawLoop { from, to } => Some(Preview::Loop(clip_gesture::drawn_loop(
                *from,
                self.grid.tick_at(to.x),
                unit,
                step,
            ))),
            Gesture::MoveLoopEdge { range, edge, to } => Some(Preview::Loop(
                clip_gesture::moved_loop_edge(*range, *edge, self.grid.tick_at(to.x), unit, step),
            )),
            Gesture::TrimStart { id, .. } | Gesture::TrimEnd { id, .. } => {
                let clip = project.playlist.clips.get(id)?;
                let index = clip_gesture::index_of(project, clip.track)?;
                let Some(Edit::TrimAudioClip { start, length, .. }) =
                    self.trim_edit(project, Modifiers::default())
                else {
                    return Some(Preview::Clips(vec![(clip.start, clip.end(), index)]));
                };
                Some(Preview::Clips(vec![(
                    start,
                    Ticks(start.0 + length.0),
                    index,
                )]))
            }
            Gesture::Stretch { id, from, to } => {
                let clip = project.playlist.clips.get(id)?;
                let index = clip_gesture::index_of(project, clip.track)?;
                let end = clip_gesture::stretched_end(clip, self.dragged(*from, *to), unit);
                Some(Preview::Clips(vec![(clip.start, end, index)]))
            }
            Gesture::Gain { .. } | Gesture::MovePoint { .. } | Gesture::Tension { .. } => None,
        }
    }

    /// Where a file dragged from the browser would land if dropped now: the track under the
    /// cursor and the tick there, floored to the snap.
    pub fn drop_target(&self, project: &Project) -> Option<(TrackId, Ticks)> {
        let (index, tick) = self.drop_point(project, self.hover?)?;
        Some((clip_gesture::track_at(project, index)?, tick))
    }

    /// Where something let go of at tracks point `at` lands: the index (in track order) of the
    /// row there, which may be past the last track, and the tick there floored to the snap.
    /// `None` on the track headers.
    pub fn drop_point(&self, project: &Project, at: Point) -> Option<(usize, Ticks)> {
        if at.x < TRACK_HEADER_WIDTH {
            return None;
        }
        let index = usize::try_from(self.grid.track_index_at(at.y)).ok()?;
        let tick = snap_down(self.grid.tick_at(at.x), self.unit(project));
        (tick.0 >= 0).then_some((index, tick))
    }

    /// Handles one canvas or header message; `brush` is the pattern a press on empty lane
    /// space places. Returns what the app should do, if anything.
    pub fn update(
        &mut self,
        message: PlaylistMessage,
        project: &Project,
        brush: Option<PatternId>,
        modifiers: Modifiers,
    ) -> Option<Command> {
        match message {
            PlaylistMessage::TracksResized(size) => {
                self.tracks_size = size;
                self.clamp(project);
                None
            }
            PlaylistMessage::BrowserHover(at) => {
                self.hover = at;
                None
            }
            PlaylistMessage::Placed(bounds) => {
                self.panel = bounds;
                None
            }
            PlaylistMessage::SnapSelected(snap) => {
                self.snap = snap;
                None
            }
            PlaylistMessage::ZoomIn => {
                self.grid.zoom = (self.grid.zoom + 1).min(ZOOM_LEVELS.len() - 1);
                None
            }
            PlaylistMessage::ZoomOut => {
                self.grid.zoom = self.grid.zoom.saturating_sub(1);
                None
            }
            PlaylistMessage::ToggleFit => {
                self.lanes = match self.lanes {
                    Lanes::Fit => Lanes::Fixed(self.grid.lane),
                    Lanes::Fixed(_) => Lanes::Fit,
                };
                self.clamp(project);
                None
            }
            PlaylistMessage::TracksScrolled { at, delta } => {
                self.scroll(project, at, delta, modifiers);
                None
            }
            PlaylistMessage::TracksPressed { at, button, double } => {
                self.press_tracks(project, brush, at, button, double, modifiers)
            }
            PlaylistMessage::RulerPressed { at, button } => self.press_ruler(project, at, button),
            PlaylistMessage::CurveChosen(curve) => {
                let menu = self.curve_menu.take()?;
                Some(Command::Edit(Edit::SetSegmentCurve {
                    automation: menu.automation,
                    tick: menu.tick,
                    curve,
                }))
            }
            PlaylistMessage::TrackMenu(row) => {
                let (id, _) = self.track_menu.take()?;
                match row {
                    TrackRow::Rename => {
                        self.rename = Some((id, project.playlist.tracks.get(&id)?.name.clone()));
                        Some(Command::FocusRename)
                    }
                    TrackRow::Delete => Some(Command::Edit(Edit::DeleteTrack { id })),
                }
            }
            PlaylistMessage::ClipMenu(row) => {
                let (id, _) = self.clip_menu.take()?;
                Some(Command::Link(match row {
                    ClipRow::Unlink => ClipLink::Unlink(id),
                    ClipRow::LinkBack => ClipLink::LinkBack(id),
                }))
            }
            PlaylistMessage::TracksDragged(at) | PlaylistMessage::RulerDragged(at) => {
                self.drag_to(at);
                None
            }
            PlaylistMessage::TracksReleased(at) | PlaylistMessage::RulerReleased(at) => {
                self.drag_to(at);
                self.release(project, modifiers)
            }
            PlaylistMessage::RenameEdited(text) => {
                if let Some((_, name)) = &mut self.rename {
                    *name = text;
                }
                None
            }
            PlaylistMessage::RenameSubmitted => {
                let (id, name) = self.rename.take()?;
                let track = project.playlist.tracks.get(&id)?;
                (track.name != name).then_some(Command::Edit(Edit::RenameTrack { id, name }))
            }
            PlaylistMessage::ClipMode(mode) => self
                .properties
                .map(|clip| Command::StretchMode { clip, mode }),
            PlaylistMessage::ClipPitch(semitones) => {
                clip_gesture::with_pitch(project, self.properties?, semitones).map(Command::Edit)
            }
            PlaylistMessage::CloseProperties => {
                self.properties = None;
                None
            }
            PlaylistMessage::Bar(BarMessage::To { offset, held }) => {
                self.bar_held = held;
                self.grid.scroll = Ticks(offset.round() as i64);
                self.clamp(project);
                None
            }
            PlaylistMessage::Bar(BarMessage::LetGo) => {
                self.bar_held = false;
                None
            }
        }
    }

    /// Delete: takes the selected clips out.
    pub fn delete_selected(&mut self) -> Option<Edit> {
        let edit = clip_gesture::deleted(&self.selection)?;
        self.selection.clear();
        Some(edit)
    }

    /// Ctrl+B: copies the selected clips one selection length later; the copies become the
    /// selection (an edit that is refused leaves IDs that [`refresh`](Self::refresh) drops).
    pub fn duplicate_selected(&mut self, project: &Project) -> Option<Edit> {
        let (edit, copies) = clip_gesture::duplicated(project, &self.selection)?;
        self.selection = copies;
        Some(edit)
    }

    /// The snap unit in ticks.
    fn unit(&self, project: &Project) -> Ticks {
        self.snap.unit(project.ppq)
    }

    /// The ticks between the press at `from`, on the view as it was then, and the cursor at
    /// `to` now: the same as between the two points while the view stays put.
    fn dragged(&self, from: Point, to: Point) -> Ticks {
        Ticks(self.grid.tick_at(to.x).0 - self.press_view.tick_at(from.x).0)
    }

    /// The press at `from` where the view now shows the place it was on.
    fn anchored(&self, from: Point) -> Point {
        Point::new(self.press_view.x_in(from.x, &self.grid), from.y)
    }

    /// A press that may begin a gesture: drags count from the view as it is now.
    fn pressed_here(&mut self) {
        self.press_view = self.grid;
        self.edge_scrolled = false;
    }

    fn move_targets(
        &self,
        project: &Project,
        ids: &[ClipId],
        from: Point,
        to: Point,
    ) -> Vec<(ClipId, Ticks, usize)> {
        let rows = self.grid.track_index_at(to.y) - self.grid.track_index_at(from.y);
        let ticks = self.dragged(from, to);
        clip_gesture::move_targets(project, ids, ticks, rows, self.unit(project))
    }

    /// The clip under tracks-canvas point `at` and the part of it, the one drawn on top when
    /// clips overlap. Clips are drawn in (start, ID) order, so that is the latest of them.
    pub fn clip_under(&self, project: &Project, at: Point) -> Option<(ClipId, ClipPart)> {
        let index = usize::try_from(self.grid.track_index_at(at.y)).ok()?;
        let track = clip_gesture::track_at(project, index)?;
        let top = self.grid.row_top_of(index)?;
        if at.y - top >= self.grid.lane {
            return None;
        }
        project
            .playlist
            .clips
            .iter()
            .filter(|(_, clip)| clip.track == track)
            .filter_map(|(&id, clip)| {
                let rect = playlist_layout::clip_rect(&self.grid, clip, top);
                let trims = clip.audio_channel().is_some();
                Some((
                    clip.start,
                    id,
                    playlist_layout::clip_part(rect, at.x, trims)?,
                ))
            })
            .max_by_key(|&(start, id, _)| (start, id))
            .map(|(_, id, part)| (id, part))
    }

    fn press_tracks(
        &mut self,
        project: &Project,
        brush: Option<PatternId>,
        at: Point,
        button: mouse::Button,
        double: bool,
        modifiers: Modifiers,
    ) -> Option<Command> {
        if self.gesture.is_some() {
            return None;
        }
        self.pressed_here();
        // Any press on the tracks closes the menus over them: a press on a menu is the menu's
        // own, taken on the window's top layer before it could reach the tracks.
        self.rename = None;
        self.track_menu = None;
        self.curve_menu = None;
        self.clip_menu = None;
        if at.x < TRACK_HEADER_WIDTH {
            // A right press on a track's name opens its menu there (Rename, Delete). Why a
            // menu rather than deleting at once: the press did nothing before, and a delete on
            // a press of the name is too easy to make by mistake.
            if button == mouse::Button::Right {
                let rows = project.playlist.tracks.len().saturating_sub(self.grid.top);
                if let Some(HeaderHit::Name { row }) =
                    playlist_layout::header_hit(&self.grid, at, rows)
                {
                    let id = clip_gesture::track_at(project, self.grid.top + row)?;
                    self.track_menu = Some((id, at));
                    return None;
                }
            }
            return self.press_header(project, at, double);
        }
        let index = usize::try_from(self.grid.track_index_at(at.y)).ok()?;
        clip_gesture::track_at(project, index)?;
        let under = self.clip_under(project, at);
        if let Some((id, part)) = under {
            if button == mouse::Button::Right && modifiers.shift() {
                let clip = project.playlist.clips.get(&id)?;
                return Some(Command::Edit(Edit::SetClipMute {
                    ids: vec![id],
                    muted: !clip.muted,
                }));
            }
            // Ctrl+right on a pattern clip opens its menu (unlink, link back). Why not a plain
            // right press: that deletes the clip, and stays one press.
            let pattern_clip = project.playlist.clips.get(&id)?.pattern().is_some();
            if button == mouse::Button::Right && modifiers.command() && pattern_clip {
                self.clip_menu = Some((id, at));
                return None;
            }
            let press = ClipPress {
                id,
                part,
                at,
                button,
                double,
                alt: modifiers.alt(),
            };
            if let Some(command) = self.press_clip_parts(project, press) {
                return command;
            }
        }
        match (button, under) {
            (mouse::Button::Right, Some((id, _))) => {
                let ids = if self.selection.contains(&id) {
                    std::mem::take(&mut self.selection)
                } else {
                    BTreeSet::from([id])
                };
                self.selection.retain(|selected| !ids.contains(selected));
                clip_gesture::deleted(&ids).map(Command::Edit)
            }
            (mouse::Button::Left, Some((id, _))) if modifiers.shift() => {
                if !self.selection.remove(&id) {
                    self.selection.insert(id);
                }
                None
            }
            (mouse::Button::Left, Some((id, ClipPart::RightEdge))) => {
                self.gesture = Some(Gesture::Resize {
                    id,
                    from: at,
                    to: at,
                });
                None
            }
            (mouse::Button::Left, Some((id, ClipPart::Body))) => {
                if !self.selection.contains(&id) {
                    self.selection = BTreeSet::from([id]);
                }
                self.gesture = Some(Gesture::Move {
                    ids: self.selection.iter().copied().collect(),
                    from: at,
                    to: at,
                });
                None
            }
            (mouse::Button::Left, None) if modifiers.command() => {
                self.gesture = Some(Gesture::Box { from: at, to: at });
                None
            }
            (mouse::Button::Left, None) => {
                self.selection.clear();
                let tick = self.grid.tick_at(at.x);
                clip_gesture::place(project, brush?, index, tick, self.unit(project))
                    .map(Command::Edit)
            }
            _ => None,
        }
    }

    /// A press in the track header column: the mute light, a name (a double press renames it)
    /// or the "+ Track" button.
    fn press_header(&mut self, project: &Project, at: Point, double: bool) -> Option<Command> {
        let playlist = &project.playlist;
        let rows = playlist.tracks.len().saturating_sub(self.grid.top);
        let track = |row: usize| clip_gesture::track_at(project, self.grid.top + row);
        match playlist_layout::header_hit(&self.grid, at, rows)? {
            HeaderHit::Led { row } => {
                let id = track(row)?;
                let mute = !playlist.tracks.get(&id)?.mute;
                Some(Command::Edit(Edit::SetTrackMute { id, mute }))
            }
            HeaderHit::Name { row } if double => {
                let id = track(row)?;
                self.rename = Some((id, playlist.tracks.get(&id)?.name.clone()));
                Some(Command::FocusRename)
            }
            HeaderHit::Name { .. } => None,
            HeaderHit::Arm { row } => Some(Command::Arm(track(row)?)),
            HeaderHit::AddTrack if playlist.tracks.len() < MAX_TRACKS => {
                let id = playlist.next_track_id;
                let name = format!("Track {}", id.0);
                Some(Command::Edit(Edit::AddTrack { id, name }))
            }
            HeaderHit::AddTrack => None,
        }
    }

    fn press_ruler(
        &mut self,
        project: &Project,
        at: Point,
        button: mouse::Button,
    ) -> Option<Command> {
        if self.gesture.is_some() || at.x < TRACK_HEADER_WIDTH {
            return None;
        }
        self.pressed_here();
        self.rename = None;
        let tick = self.grid.tick_at(at.x);
        let current = project.playlist.loop_range;
        match (playlist_layout::ruler_band(at.y)?, button) {
            (RulerBand::Numbers, mouse::Button::Left) => Some(Command::Seek(Ticks(
                snap_down(tick, self.unit(project)).0.max(0),
            ))),
            (RulerBand::LoopStrip, mouse::Button::Right) => {
                current.map(|_| Command::Edit(Edit::SetLoop { value: None }))
            }
            (RulerBand::LoopStrip, mouse::Button::Left) => {
                let edge = current.and_then(|range| {
                    Some((
                        range,
                        playlist_layout::loop_edge_under(&self.grid, &range, at.x)?,
                    ))
                });
                self.gesture = Some(match edge {
                    Some((range, edge)) => Gesture::MoveLoopEdge {
                        range,
                        edge,
                        to: at,
                    },
                    None => Gesture::DrawLoop { from: tick, to: at },
                });
                None
            }
            _ => None,
        }
    }

    fn drag_to(&mut self, at: Point) {
        match &mut self.gesture {
            Some(
                Gesture::Move { to, .. }
                | Gesture::Resize { to, .. }
                | Gesture::Box { to, .. }
                | Gesture::DrawLoop { to, .. }
                | Gesture::MoveLoopEdge { to, .. }
                | Gesture::TrimStart { to, .. }
                | Gesture::TrimEnd { to, .. }
                | Gesture::Stretch { to, .. }
                | Gesture::Gain { to, .. }
                | Gesture::MovePoint { to, .. }
                | Gesture::Tension { to, .. },
            ) => *to = at,
            None => {}
        }
    }

    /// Ends the held gesture: a box selects what it meets; everything else commits one edit,
    /// or none when it changed nothing.
    fn release(&mut self, project: &Project, modifiers: Modifiers) -> Option<Command> {
        let gesture = self.gesture.take()?;
        let unit = self.unit(project);
        let edit = match gesture {
            Gesture::TrimStart { .. } | Gesture::TrimEnd { .. } => {
                self.gesture = Some(gesture);
                let edit = self.trim_edit(project, modifiers);
                self.gesture = None;
                edit
            }
            Gesture::Stretch { id, from, to } => {
                let clip = project.playlist.clips.get(&id)?;
                let end = clip_gesture::stretched_end(clip, self.dragged(from, to), unit);
                return (end != clip.end()).then_some(Command::Stretch { clip: id, end });
            }
            Gesture::Gain { id, gain, from, to } => {
                let step = if modifiers.shift() {
                    FINE_GAIN_DB_PER_PIXEL
                } else {
                    GAIN_DB_PER_PIXEL
                };
                let moved = clips::gain_moved(gain, (from.y - to.y) * step);
                (moved != gain).then_some(Edit::SetClipGain {
                    clip: id,
                    gain: moved,
                })
            }
            Gesture::MovePoint { id, tick, to } => {
                self.moved_point(project, id, tick, to, modifiers)
            }
            Gesture::Tension { id, tick, to } => self.tension_edit(project, id, tick, to),
            Gesture::Box { from, to } => {
                self.selection = self.clips_meeting(project, self.anchored(from), to);
                return None;
            }
            Gesture::Move { ids, from, to } => {
                let targets = self.move_targets(project, &ids, from, to);
                clip_gesture::moved(project, &targets)
            }
            Gesture::Resize { id, from, to } => {
                clip_gesture::resized(project, id, self.dragged(from, to), unit)
            }
            Gesture::DrawLoop { .. } | Gesture::MoveLoopEdge { .. } => {
                self.gesture = Some(gesture);
                let preview = self.preview(project);
                self.gesture = None;
                let Some(Preview::Loop(range)) = preview else {
                    return None;
                };
                (project.playlist.loop_range != Some(range))
                    .then_some(Edit::SetLoop { value: Some(range) })
            }
        };
        edit.map(Command::Edit)
    }

    /// The body of `clip` (under its title strip) in the row that shows it.
    fn body_rect(&self, project: &Project, clip: &Clip) -> Option<Rectangle> {
        let index = clip_gesture::index_of(project, clip.track)?;
        let top = self.grid.row_top_of(index)?;
        let rect = playlist_layout::clip_rect(&self.grid, clip, top);
        Some(Rectangle::new(
            Point::new(rect.x, top + TITLE_HEIGHT),
            Size::new(rect.width, self.grid.lane - TITLE_HEIGHT),
        ))
    }

    /// A press on an audio clip's edges or gain readout, or on an automation clip's points,
    /// handles or body; `None` when it falls through to the moves every clip takes. The inner
    /// `None` is a press handled with nothing to commit yet.
    fn press_clip_parts(&mut self, project: &Project, press: ClipPress) -> Option<Option<Command>> {
        let ClipPress {
            id,
            part,
            at,
            button,
            double,
            alt,
        } = press;
        let clip = project.playlist.clips.get(&id)?;
        let left = button == mouse::Button::Left;
        match clip.item {
            ClipItem::Audio { gain, .. } => {
                let index = clip_gesture::index_of(project, clip.track)?;
                let top = self.grid.row_top_of(index)?;
                let rect = playlist_layout::clip_rect(&self.grid, clip, top);
                let readout = at.y < top + TITLE_HEIGHT
                    && at.x >= rect.x + rect.width - GAIN_READOUT_WIDTH
                    && part == ClipPart::Body;
                match (left, part, readout) {
                    (true, ClipPart::LeftEdge, _) => {
                        self.gesture = Some(Gesture::TrimStart {
                            id,
                            from: at,
                            to: at,
                        });
                        Some(None)
                    }
                    (true, ClipPart::RightEdge, _) if alt => {
                        self.gesture = Some(Gesture::Stretch {
                            id,
                            from: at,
                            to: at,
                        });
                        Some(None)
                    }
                    (true, ClipPart::RightEdge, _) => {
                        self.gesture = Some(Gesture::TrimEnd {
                            id,
                            from: at,
                            to: at,
                        });
                        Some(None)
                    }
                    (true, ClipPart::Body, true) if double => {
                        Some((gain != 1.0).then_some(Command::Edit(Edit::SetClipGain {
                            clip: id,
                            gain: 1.0,
                        })))
                    }
                    (true, ClipPart::Body, false) if double => {
                        self.properties = Some(id);
                        Some(Some(Command::EditSample(id)))
                    }
                    (true, ClipPart::Body, true) => {
                        self.gesture = Some(Gesture::Gain {
                            id,
                            gain,
                            from: at,
                            to: at,
                        });
                        Some(None)
                    }
                    _ => None,
                }
            }
            ClipItem::Automation { automation: owner } => {
                let automation = project.automation.list.get(&owner)?;
                let body = self.body_rect(project, clip)?;
                if !body.contains(at) {
                    return None;
                }
                let near = |point: Point| point.distance(at) <= POINT_REACH;
                let point = automation_points(&self.grid, clip, automation, body)
                    .into_iter()
                    .find(|&(_, drawn)| near(drawn));
                if let Some((tick, _)) = point {
                    return Some(match button {
                        mouse::Button::Left => {
                            self.gesture = Some(Gesture::MovePoint { id, tick, to: at });
                            None
                        }
                        mouse::Button::Right if automation.points.len() > 1 => {
                            Some(Command::Edit(Edit::DeleteAutomationPoint {
                                automation: owner,
                                tick,
                            }))
                        }
                        _ => None,
                    });
                }
                let handle = automation_handles(&self.grid, clip, automation, body)
                    .into_iter()
                    .find(|handle| near(handle.at));
                if let Some(handle) = handle {
                    return Some(match (button, double) {
                        (mouse::Button::Left, true) => Some(Command::Edit(Edit::SetSegmentCurve {
                            automation: owner,
                            tick: handle.tick,
                            curve: Curve::Linear,
                        })),
                        (mouse::Button::Left, false) => {
                            self.gesture = Some(Gesture::Tension {
                                id,
                                tick: handle.tick,
                                to: at,
                            });
                            None
                        }
                        (mouse::Button::Right, _) => {
                            self.curve_menu = Some(CurveMenu {
                                automation: owner,
                                tick: handle.tick,
                                at,
                            });
                            None
                        }
                        _ => None,
                    });
                }
                if left && double {
                    let song = snap_down(self.grid.tick_at(at.x), self.unit(project));
                    let tick = Ticks(song.0 - clip.start.0 + clip.offset.0);
                    let value =
                        playlist_layout::automation_value(body.y, body.y + body.height, at.y);
                    let taken = automation.point_at(tick).is_some();
                    return Some((!taken && tick.0 >= 0).then_some(Command::Edit(
                        Edit::AddAutomationPoint {
                            automation: owner,
                            point: AutomationPoint::linear(tick, value),
                        },
                    )));
                }
                None
            }
            // A double press on the body opens the clip in the piano roll, the clip selected
            // alone; its edges keep their drags, and every other press its move.
            ClipItem::Pattern { .. } if left && double && part == ClipPart::Body => {
                self.selection = BTreeSet::from([id]);
                Some(Some(Command::OpenInRoll(id)))
            }
            ClipItem::Pattern { .. } => None,
        }
    }

    /// The trim a held edge drag would commit: the right edge moves the end; the left edge moves
    /// the start and the frame offset with it at the project's tempo, and stops where the file
    /// starts. Both snap to the grid (Alt does not) and keep the clip a tick long.
    fn trim_edit(&self, project: &Project, modifiers: Modifiers) -> Option<Edit> {
        let (id, from, to, left) = match self.gesture.as_ref()? {
            Gesture::TrimStart { id, from, to } => (*id, *from, *to, true),
            Gesture::TrimEnd { id, from, to } => (*id, *from, *to, false),
            _ => return None,
        };
        let clip = project.playlist.clips.get(&id)?;
        let ClipItem::Audio { frame_offset, .. } = clip.item else {
            return None;
        };
        let moved = |tick: Ticks| {
            let raw = Ticks(tick.0 + self.dragged(from, to).0);
            if modifiers.alt() {
                raw
            } else {
                snap_nearest(raw, self.unit(project))
            }
        };
        let sample = |tick: Ticks| {
            time::tick_to_sample(tick, project.sample_rate, project.tempo, project.ppq)
        };
        let (start, end, offset) = if left {
            let mut start = Ticks(moved(clip.start).0.clamp(0, clip.end().0 - 1));
            let old = sample(clip.start)?;
            if sample(start)? + frame_offset < old {
                // The file starts here: no frame before frame 0.
                let first = old - frame_offset;
                start = time::first_tick_at_or_after(
                    first,
                    project.sample_rate,
                    project.tempo,
                    project.ppq,
                )?;
            }
            let offset = (frame_offset + sample(start)?).checked_sub(old)?;
            (start, clip.end(), offset)
        } else {
            let end = Ticks(moved(clip.end()).0.max(clip.start.0 + 1));
            (clip.start, end, frame_offset)
        };
        let changed = (start, end, offset) != (clip.start, clip.end(), frame_offset);
        changed.then_some(Edit::TrimAudioClip {
            clip: id,
            start,
            length: Ticks(end.0 - start.0),
            frame_offset: offset,
        })
    }

    /// The move a held point drag commits: the tick under the cursor (snapped, unless Alt),
    /// kept strictly between the point's neighbors, and the value at the cursor's height.
    fn moved_point(
        &self,
        project: &Project,
        id: ClipId,
        tick: Ticks,
        to: Point,
        modifiers: Modifiers,
    ) -> Option<Edit> {
        let clip = project.playlist.clips.get(&id)?;
        let owner = clip.automation()?;
        let automation = project.automation.list.get(&owner)?;
        let body = self.body_rect(project, clip)?;
        let (index, point) = automation.point_at(tick)?;
        let raw = self.grid.tick_at(to.x);
        let song = if modifiers.alt() {
            raw
        } else {
            snap_nearest(raw, self.unit(project))
        };
        let lowest = index
            .checked_sub(1)
            .map_or(0, |before| automation.points[before].tick.0 + 1);
        let highest = automation
            .points
            .get(index + 1)
            .map_or(i64::MAX, |after| after.tick.0 - 1);
        let new_tick =
            Ticks((song.0 - clip.start.0 + clip.offset.0).clamp(lowest, highest.max(lowest)));
        let value = playlist_layout::automation_value(body.y, body.y + body.height, to.y);
        ((new_tick, value) != (point.tick, point.value)).then_some(Edit::MoveAutomationPoint {
            automation: owner,
            from_tick: tick,
            tick: new_tick,
            value,
        })
    }

    /// The tension a held handle drag commits: the curve whose midpoint lands on the cursor,
    /// from the cursor's place between the segment's two values.
    fn tension_edit(&self, project: &Project, id: ClipId, tick: Ticks, to: Point) -> Option<Edit> {
        let clip = project.playlist.clips.get(&id)?;
        let owner = clip.automation()?;
        let automation = project.automation.list.get(&owner)?;
        let body = self.body_rect(project, clip)?;
        let (index, point) = automation.point_at(tick)?;
        let next = automation.points.get(index + 1)?;
        let value = playlist_layout::automation_value(body.y, body.y + body.height, to.y);
        if next.value == point.value {
            return None;
        }
        let fraction = (value - point.value) / (next.value - point.value);
        let curve = Curve::Exp(clips::tension_for(fraction));
        (curve != point.curve).then_some(Edit::SetSegmentCurve {
            automation: owner,
            tick,
            curve,
        })
    }

    /// The clips on visible rows whose drawn rectangles meet the box from `a` to `b`.
    fn clips_meeting(&self, project: &Project, a: Point, b: Point) -> BTreeSet<ClipId> {
        project
            .playlist
            .clips
            .iter()
            .filter(|(_, clip)| {
                clip_gesture::index_of(project, clip.track)
                    .and_then(|index| self.grid.row_top_of(index))
                    .is_some_and(|top| {
                        let rect = playlist_layout::clip_rect(&self.grid, clip, top);
                        playlist_layout::meets_box(rect, a, b)
                    })
            })
            .map(|(&id, _)| id)
            .collect()
    }

    /// The wheel over the tracks: tracks, Shift (or a wheel across, a trackpad's) for time, Ctrl
    /// to zoom around the cursor, Alt for the lane height. One step per wheel event, whatever
    /// its size.
    fn scroll(
        &mut self,
        project: &Project,
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
        if modifiers.alt() && !across {
            // A height chosen: the lanes keep it from now on, however many tracks there are.
            self.grid.lane = self.grid.stepped_lane(up);
            self.lanes = Lanes::Fixed(self.grid.lane);
        } else if modifiers.command() && !across {
            let zoom = if up {
                (self.grid.zoom + 1).min(ZOOM_LEVELS.len() - 1)
            } else {
                self.grid.zoom.saturating_sub(1)
            };
            // Keep the tick under the cursor where it is: the new scroll puts it back under x.
            let under = self.grid.tick_at(at.x);
            let unscrolled = Grid {
                zoom,
                scroll: Ticks(0),
                ..self.grid
            };
            self.grid.zoom = zoom;
            self.grid.scroll = Ticks((under.0 - unscrolled.tick_at(at.x).0).max(0));
        } else if modifiers.shift() || across {
            let ticks = (f64::from(PIXELS_PER_NOTCH) * self.grid.bar.0 as f64
                / f64::from(self.grid.px_per_bar())) as i64;
            let moved = self.grid.scroll.0 + if up { -ticks } else { ticks };
            self.grid.scroll = Ticks(moved.max(0));
        } else {
            let rows = if up {
                -TRACKS_PER_NOTCH
            } else {
                TRACKS_PER_NOTCH
            };
            self.grid.top = usize::try_from(self.grid.top as i64 + rows).unwrap_or(0);
        }
        self.clamp(project);
    }

    /// Keeps the view on the song (no further than its longest possible end) and on the tracks
    /// (the last one can scroll up to the top row, no further).
    /// Keeps the view on the tracks and the song, and fitted lanes fitted to the tracks there
    /// are and the track area's height.
    fn clamp(&mut self, project: &Project) {
        let tracks = project.playlist.tracks.len();
        if self.lanes == Lanes::Fit {
            self.grid.lane = playlist_layout::fitted_lane(tracks, self.tracks_size.height);
        }
        let last = tracks.saturating_sub(1);
        self.grid.top = self.grid.top.min(last);
        let most = max_song_tick(project.ppq).0;
        self.grid.scroll = Ticks(self.grid.scroll.0.clamp(0, most));
    }
}

#[cfg(test)]
mod tests {
    use dawsome_engine::edit;

    use super::*;

    const P4_TWO_CLIPS: &str = include_str!("../../../../tests/fixtures/p4_two_clips.json");
    const P4_LOOP: &str = include_str!("../../../../tests/fixtures/p4_loop.json");

    fn load(text: &str) -> Project {
        Project::load_json(text).expect("fixture loads")
    }

    /// The playlist these tests drive: lanes kept at the lowest height, 32 px, the height
    /// their points were written for (fitted lanes are the default; those
    /// are tested apart).
    fn opened() -> Arranger {
        Arranger::new(96, Size::new(1000.0, 452.0), Lanes::Fixed(LANE_HEIGHT))
    }

    fn press(at: (f32, f32)) -> PlaylistMessage {
        PlaylistMessage::TracksPressed {
            at: Point::new(at.0, at.1),
            button: mouse::Button::Left,
            double: false,
        }
    }

    fn release(at: (f32, f32)) -> PlaylistMessage {
        PlaylistMessage::TracksReleased(Point::new(at.0, at.1))
    }

    const NONE: Modifiers = Modifiers::empty();

    /// Applies `command`'s edit to `project`, as the app would.
    fn apply(project: &mut Project, command: Option<Command>) {
        let Some(Command::Edit(change)) = command else {
            panic!("expected an edit, got {command:?}");
        };
        edit::apply(project, change).expect("the edit applies");
    }

    // p4_two_clips: clip 1 at bar 1 (x 120..184) and clip 2 at bar 3 (x 248..312) on track 1
    // (y 0..32). An empty press at x 330 (tick 1260, bar 4 at 1152) on track 2 (y 33..65)
    // places the brush there.
    #[test]
    fn a_press_on_empty_space_places_the_brush_and_clears_the_selection() {
        let project = load(P4_TWO_CLIPS);
        let mut arranger = opened();
        arranger.selection.insert(ClipId(1));
        let placed = arranger.update(press((330.0, 40.0)), &project, Some(PatternId(2)), NONE);
        let clip = dawsome_engine::project::Clip::of_pattern(
            PatternId(2),
            TrackId(2),
            Ticks(1152),
            Ticks(384),
        );
        assert_eq!(
            placed,
            Some(Command::Edit(Edit::PlaceClips {
                first: ClipId(3),
                clips: vec![clip]
            }))
        );
        assert!(arranger.selection.is_empty());
        assert_eq!(
            arranger.update(press((330.0, 40.0)), &project, None, NONE),
            None,
            "no brush, nothing placed"
        );
        assert_eq!(
            arranger.update(press((330.0, 300.0)), &project, Some(PatternId(2)), NONE),
            None,
            "below the last track"
        );
    }

    // A body press selects clip 1 and grabs it; a drag of 64 px (one bar) right and 33 px (one
    // row) down moves it to bar 2 on track 2 on release, one MoveClips. A drag that comes back
    // commits nothing.
    #[test]
    fn a_body_drag_moves_the_clip_on_release() {
        let project = load(P4_TWO_CLIPS);
        let mut arranger = opened();
        arranger.update(press((140.0, 16.0)), &project, None, NONE);
        assert!(arranger.tracks_held());
        assert_eq!(arranger.selection, BTreeSet::from([ClipId(1)]));
        arranger.update(
            PlaylistMessage::TracksDragged(Point::new(204.0, 49.0)),
            &project,
            None,
            NONE,
        );
        assert_eq!(
            arranger.preview(&project),
            Some(Preview::Clips(vec![(Ticks(384), Ticks(768), 1)]))
        );
        let moved = arranger.update(release((204.0, 49.0)), &project, None, NONE);
        assert_eq!(
            moved,
            Some(Command::Edit(Edit::MoveClips {
                moves: vec![(ClipId(1), Ticks(384), TrackId(2))]
            }))
        );
        assert!(!arranger.tracks_held());
        arranger.update(press((140.0, 16.0)), &project, None, NONE);
        arranger.update(
            PlaylistMessage::TracksDragged(Point::new(300.0, 90.0)),
            &project,
            None,
            NONE,
        );
        assert_eq!(
            arranger.update(release((150.0, 20.0)), &project, None, NONE),
            None
        );
    }

    // Shift adds a clip to the selection or takes it out; a body drag then moves both.
    #[test]
    fn shift_toggles_and_the_selection_moves_together() {
        let project = load(P4_TWO_CLIPS);
        let mut arranger = opened();
        arranger.update(press((140.0, 16.0)), &project, None, NONE);
        arranger.update(release((140.0, 16.0)), &project, None, NONE);
        arranger.update(press((260.0, 16.0)), &project, None, Modifiers::SHIFT);
        assert_eq!(arranger.selection, BTreeSet::from([ClipId(1), ClipId(2)]));
        arranger.update(press((260.0, 16.0)), &project, None, Modifiers::SHIFT);
        assert_eq!(arranger.selection, BTreeSet::from([ClipId(1)]));
        arranger.update(press((260.0, 16.0)), &project, None, Modifiers::SHIFT);
        arranger.update(press((260.0, 16.0)), &project, None, NONE);
        let moved = arranger.update(release((324.0, 16.0)), &project, None, NONE);
        assert_eq!(
            moved,
            Some(Command::Edit(Edit::MoveClips {
                moves: vec![
                    (ClipId(1), Ticks(384), TrackId(1)),
                    (ClipId(2), Ticks(1152), TrackId(1))
                ]
            }))
        );
    }

    // A press 3 px inside clip 1's right edge (x 181) dragged 128 px right: 768 ticks, the
    // end floors to 1152.
    #[test]
    fn a_right_edge_drag_resizes_on_release() {
        let project = load(P4_TWO_CLIPS);
        let mut arranger = opened();
        arranger.update(press((181.0, 16.0)), &project, None, NONE);
        arranger.update(
            PlaylistMessage::TracksDragged(Point::new(309.0, 16.0)),
            &project,
            None,
            NONE,
        );
        assert_eq!(
            arranger.preview(&project),
            Some(Preview::Clips(vec![(Ticks(0), Ticks(1152), 0)]))
        );
        assert_eq!(
            arranger.update(release((309.0, 16.0)), &project, None, NONE),
            Some(Command::Edit(Edit::ResizeClip {
                id: ClipId(1),
                length: Ticks(1152)
            }))
        );
    }

    // A right press deletes the clip under it, or the whole selection when it is in it.
    #[test]
    fn a_right_press_deletes_the_clip_or_the_selection() {
        let project = load(P4_TWO_CLIPS);
        let right = |x| PlaylistMessage::TracksPressed {
            at: Point::new(x, 16.0),
            button: mouse::Button::Right,
            double: false,
        };
        let mut arranger = opened();
        assert_eq!(
            arranger.update(right(260.0), &project, None, NONE),
            Some(Command::Edit(Edit::DeleteClips {
                ids: vec![ClipId(2)]
            }))
        );
        arranger.selection = BTreeSet::from([ClipId(1), ClipId(2)]);
        assert_eq!(
            arranger.update(right(140.0), &project, None, NONE),
            Some(Command::Edit(Edit::DeleteClips {
                ids: vec![ClipId(1), ClipId(2)]
            }))
        );
        assert!(arranger.selection.is_empty());
        assert_eq!(arranger.update(right(220.0), &project, None, NONE), None);
    }

    #[test]
    fn a_ctrl_drag_on_empty_space_selects_what_its_box_meets() {
        let project = load(P4_TWO_CLIPS);
        let mut arranger = opened();
        arranger.update(press((200.0, 50.0)), &project, None, Modifiers::CTRL);
        assert!(matches!(arranger.preview(&project), Some(Preview::Box(..))));
        arranger.update(release((260.0, 10.0)), &project, None, NONE);
        assert_eq!(arranger.selection, BTreeSet::from([ClipId(2)]));
        arranger.update(press((200.0, 50.0)), &project, None, Modifiers::CTRL);
        arranger.update(release((100.0, 0.0)), &project, None, NONE);
        assert_eq!(arranger.selection, BTreeSet::from([ClipId(1)]));
    }

    // Ruler: a number-band press at x 258 (tick 828) seeks to bar 3 (768); a loop-strip drag
    // from x 194 (tick 444) to 258 draws 384..1152; on p4_loop (384..768, x 184..248) a press at
    // 250 drags the end to x 376 (tick 1536); a right press clears the range.
    #[test]
    fn the_ruler_seeks_and_draws_moves_and_clears_the_loop() {
        let project = load(P4_TWO_CLIPS);
        let mut arranger = opened();
        let ruler = |x, y, button| PlaylistMessage::RulerPressed {
            at: Point::new(x, y),
            button,
        };
        let left = mouse::Button::Left;
        assert_eq!(
            arranger.update(ruler(258.0, 8.0, left), &project, None, NONE),
            Some(Command::Seek(Ticks(768)))
        );
        arranger.update(ruler(194.0, 20.0, left), &project, None, NONE);
        assert!(arranger.ruler_held());
        let range = |start, end, on| LoopRange {
            start: Ticks(start),
            end: Ticks(end),
            on,
        };
        assert_eq!(
            arranger.update(
                PlaylistMessage::RulerReleased(Point::new(258.0, 20.0)),
                &project,
                None,
                NONE
            ),
            Some(Command::Edit(Edit::SetLoop {
                value: Some(range(384, 1152, true))
            }))
        );
        let looped = load(P4_LOOP);
        arranger.update(ruler(250.0, 20.0, left), &looped, None, NONE);
        arranger.update(
            PlaylistMessage::RulerDragged(Point::new(376.0, 20.0)),
            &looped,
            None,
            NONE,
        );
        assert_eq!(
            arranger.preview(&looped),
            Some(Preview::Loop(range(384, 1536, true)))
        );
        assert_eq!(
            arranger.update(
                PlaylistMessage::RulerReleased(Point::new(376.0, 20.0)),
                &looped,
                None,
                NONE
            ),
            Some(Command::Edit(Edit::SetLoop {
                value: Some(range(384, 1536, true))
            }))
        );
        assert_eq!(
            arranger.update(
                ruler(300.0, 20.0, mouse::Button::Right),
                &looped,
                None,
                NONE
            ),
            Some(Command::Edit(Edit::SetLoop { value: None }))
        );
        assert_eq!(
            arranger.update(
                ruler(300.0, 20.0, mouse::Button::Right),
                &project,
                None,
                NONE
            ),
            None,
            "nothing to clear"
        );
        assert_eq!(
            arranger.update(ruler(60.0, 8.0, left), &project, None, NONE),
            None,
            "the corner over the track headers"
        );
    }

    // Header: the light mutes (track 2's light is at y 49); a double press on a name renames
    // it, Enter commits, Esc cancels; the button under the last track adds track 9.
    #[test]
    fn the_track_headers_mute_rename_and_add() {
        let mut project = load(P4_TWO_CLIPS);
        let mut arranger = opened();
        assert_eq!(
            arranger.update(press((10.0, 49.0)), &project, None, NONE),
            Some(Command::Edit(Edit::SetTrackMute {
                id: TrackId(2),
                mute: true
            }))
        );
        assert_eq!(
            arranger.update(press((60.0, 16.0)), &project, None, NONE),
            None
        );
        let double = PlaylistMessage::TracksPressed {
            at: Point::new(60.0, 16.0),
            button: mouse::Button::Left,
            double: true,
        };
        assert_eq!(
            arranger.update(double.clone(), &project, None, NONE),
            Some(Command::FocusRename)
        );
        assert_eq!(arranger.rename, Some((TrackId(1), "Track 1".into())));
        arranger.update(
            PlaylistMessage::RenameEdited("Drums".into()),
            &project,
            None,
            NONE,
        );
        let renamed = arranger.update(PlaylistMessage::RenameSubmitted, &project, None, NONE);
        assert_eq!(
            renamed,
            Some(Command::Edit(Edit::RenameTrack {
                id: TrackId(1),
                name: "Drums".into()
            }))
        );
        assert_eq!(arranger.rename, None);
        arranger.update(double, &project, None, NONE);
        arranger.escape();
        assert_eq!(arranger.rename, None, "Esc cancels the rename");
        let add = arranger.update(press((60.0, 275.0)), &project, None, NONE);
        assert_eq!(
            add,
            Some(Command::Edit(Edit::AddTrack {
                id: TrackId(9),
                name: "Track 9".into()
            }))
        );
        apply(&mut project, add);
        // Nine tracks: the button moved down a row.
        assert_eq!(
            arranger.update(press((60.0, 275.0)), &project, None, NONE),
            None
        );
    }

    // The wheel scrolls one track; Shift scrolls 64 px of time (one bar at 64 px per bar);
    // Ctrl zooms in keeping the tick under the cursor: x 248 is tick 768 at 64 and at 128 px.
    #[test]
    fn the_wheel_scrolls_tracks_and_time_and_zooms() {
        let project = load(P4_TWO_CLIPS);
        let mut arranger = opened();
        let wheel = |y| PlaylistMessage::TracksScrolled {
            at: Point::new(248.0, 100.0),
            delta: mouse::ScrollDelta::Lines { x: 0.0, y },
        };
        arranger.update(wheel(-1.0), &project, None, NONE);
        assert_eq!(arranger.grid.top, 1);
        arranger.update(wheel(1.0), &project, None, NONE);
        arranger.update(wheel(1.0), &project, None, NONE);
        assert_eq!(arranger.grid.top, 0, "never above the first track");
        arranger.update(wheel(-1.0), &project, None, Modifiers::SHIFT);
        assert_eq!(arranger.grid.scroll, Ticks(384));
        arranger.update(wheel(1.0), &project, None, Modifiers::SHIFT);
        assert_eq!(arranger.grid.scroll, Ticks(0));
        arranger.update(wheel(1.0), &project, None, Modifiers::CTRL);
        assert_eq!(arranger.grid.px_per_bar(), 128.0);
        assert_eq!(arranger.grid.tick_at(248.0), Ticks(768));
        arranger.update(PlaylistMessage::ZoomOut, &project, None, NONE);
        arranger.update(PlaylistMessage::ZoomOut, &project, None, NONE);
        arranger.update(PlaylistMessage::ZoomOut, &project, None, NONE);
        arranger.update(PlaylistMessage::ZoomOut, &project, None, NONE);
        assert_eq!(
            arranger.grid.px_per_bar(),
            16.0,
            "16 px per bar is the farthest out"
        );
        for _ in 0..9 {
            arranger.update(PlaylistMessage::ZoomIn, &project, None, NONE);
        }
        assert_eq!(arranger.grid.px_per_bar(), 256.0);
    }

    // P6, by hand on p4_two_clips (the song ends at 1152): zoomed to 256 px a bar a 1000 px
    // canvas shows 1320 ticks; playing, the view stays while the song position is in it, jumps to
    // bar 4's start (1152) when it reaches 1320, back to 0 when the song wraps; not while the
    // bar's thumb is held. A wheel across scrolls time 64 px, a bar here at 64 px a bar.
    #[test]
    fn the_song_position_pages_the_view_and_a_wheel_across_scrolls_time() {
        let project = load(P4_TWO_CLIPS);
        let mut arranger = opened();
        let across = PlaylistMessage::TracksScrolled {
            at: Point::new(248.0, 100.0),
            delta: mouse::ScrollDelta::Pixels { x: -3.0, y: 0.0 },
        };
        arranger.update(across, &project, None, NONE);
        assert_eq!((arranger.grid.scroll, arranger.grid.top), (Ticks(384), 0));
        let to = |offset, held| PlaylistMessage::Bar(BarMessage::To { offset, held });
        arranger.update(to(0.0, false), &project, None, NONE);
        arranger.grid.zoom = 4;
        let span = arranger.span(&project);
        assert_eq!((span.content, span.view), (2688.0, 1320.0));
        assert!(!arranger.follow(&project, Ticks(1319)));
        assert!(arranger.follow(&project, Ticks(1320)));
        assert_eq!(arranger.grid.scroll, Ticks(1152));
        assert!(arranger.follow(&project, Ticks(10)), "the song wrapped");
        assert_eq!(arranger.grid.scroll, Ticks(0));
        arranger.update(to(0.0, true), &project, None, NONE);
        assert!(!arranger.follow(&project, Ticks(2000)), "the thumb is held");
        arranger.update(
            PlaylistMessage::Bar(BarMessage::LetGo),
            &project,
            None,
            NONE,
        );
        assert!(arranger.follow(&project, Ticks(2000)));
        assert_eq!(arranger.grid.scroll, Ticks(1920));
    }

    // The curve menu is drawn on the window's top layer, its rows widgets of their own: a
    // right press on a tension handle opens the curve
    // menu there; its "Hold" row sets that segment's curve, one edit, and closes it; reopened,
    // a press elsewhere on the tracks closes it and edits nothing. p8_ui's automation clip on
    // track 2: one segment from tick 0, 1.0 down to 0.0, its handle at the bar's middle.
    #[test]
    fn the_curve_menu_sets_its_segments_curve() {
        let project = load(include_str!("../../../../tests/fixtures/p8_ui.json"));
        let mut arranger = opened();
        let clip = &project.playlist.clips[&ClipId(2)];
        let automation = &project.automation.list[&AutomationId(1)];
        let body = arranger.body_rect(&project, clip).expect("in view");
        let at = automation_handles(&arranger.grid, clip, automation, body)
            .first()
            .map(|handle| handle.at)
            .expect("a handle");
        let right = PlaylistMessage::TracksPressed {
            at,
            button: mouse::Button::Right,
            double: false,
        };
        assert_eq!(arranger.update(right.clone(), &project, None, NONE), None);
        assert_eq!(
            arranger.curve_menu,
            Some(CurveMenu {
                automation: AutomationId(1),
                tick: Ticks(0),
                at
            })
        );
        let chosen = PlaylistMessage::CurveChosen(CURVE_MENU[1].1);
        assert_eq!(
            arranger.update(chosen, &project, None, NONE),
            Some(Command::Edit(Edit::SetSegmentCurve {
                automation: AutomationId(1),
                tick: Ticks(0),
                curve: Curve::Hold
            }))
        );
        assert_eq!(arranger.curve_menu, None, "closed");
        arranger.update(right, &project, None, NONE);
        assert!(arranger.curve_menu.is_some());
        assert_eq!(
            arranger.update(press((900.0, 300.0)), &project, None, NONE),
            None
        );
        assert_eq!(arranger.curve_menu, None, "another press closes it");
    }

    // A trackpad's swipe across with a little down
    // in it (3 px left, 0.4 px down) scrolls time 64 px, a bar at 64 px a bar, and leaves the
    // tracks, as a swipe with none does.
    #[test]
    fn a_swipe_mostly_across_scrolls_time() {
        let project = load(P4_TWO_CLIPS);
        let mut arranger = opened();
        let swipe = PlaylistMessage::TracksScrolled {
            at: Point::new(248.0, 100.0),
            delta: mouse::ScrollDelta::Pixels { x: -3.0, y: 0.4 },
        };
        arranger.update(swipe, &project, None, NONE);
        assert_eq!((arranger.grid.scroll, arranger.grid.top), (Ticks(384), 0));
    }

    #[test]
    fn delete_and_duplicate_act_on_the_selection() {
        let project = load(P4_TWO_CLIPS);
        let mut arranger = opened();
        assert_eq!(arranger.delete_selected(), None);
        arranger.selection = BTreeSet::from([ClipId(1)]);
        assert_eq!(
            arranger.duplicate_selected(&project),
            Some(Edit::DuplicateClips {
                ids: vec![ClipId(1)],
                first: ClipId(3)
            })
        );
        assert_eq!(arranger.selection, BTreeSet::from([ClipId(3)]));
        // A refused or undone duplicate leaves an ID that is not there; refresh drops it.
        arranger.refresh(&project);
        assert!(arranger.selection.is_empty());
    }

    fn drag(at: (f32, f32)) -> PlaylistMessage {
        PlaylistMessage::TracksDragged(Point::new(at.0, at.1))
    }

    fn scroll_to(arranger: &mut Arranger, project: &Project, offset: f64) {
        let to = PlaylistMessage::Bar(BarMessage::To {
            offset,
            held: false,
        });
        arranger.update(to, project, None, NONE);
    }

    // By hand on p4_two_clips in a 1000 px canvas at 64 px a bar (6 ticks a pixel): the lanes run
    // from x 120 to 998 for the edge scroll. Clip 1's right edge pressed at x 181 (tick 366):
    // over the lanes nothing scrolls, x 998 included; at x 999 (1 px past) a tick moves 4 px,
    // 24 ticks; at x 1064 (66 px past) 22 px, 132 ticks. Three ticks there scroll to 396, and the
    // end follows the pointer: tick 944·6 + 396 = 6060, a drag of 6060 − 366 = 5694, the end
    // 384 + 5694 floored to the bar, 5760. Brought back to x 504 (tick 2304 + 396 = 2700), the
    // release resizes it to 384 + 2334 floored, 2688; the view stays where the ticks took it.
    #[test]
    fn a_drag_past_the_right_edge_scrolls_and_its_end_follows_the_pointer() {
        let project = load(P4_TWO_CLIPS);
        let mut arranger = opened();
        assert_eq!(arranger.edge_scroll(&project), None, "nothing held");
        arranger.update(press((181.0, 16.0)), &project, None, NONE);
        for inside in [500.0, 997.0, 998.0] {
            arranger.update(drag((inside, 16.0)), &project, None, NONE);
            assert_eq!(
                arranger.edge_scroll(&project),
                None,
                "over the lanes at {inside}"
            );
            assert!(!arranger.scroll_at_edge(&project));
        }
        arranger.update(drag((999.0, 16.0)), &project, None, NONE);
        assert_eq!(arranger.edge_scroll(&project), Some(Ticks(24)));
        arranger.update(drag((1064.0, 16.0)), &project, None, NONE);
        assert_eq!(arranger.edge_scroll(&project), Some(Ticks(132)));
        for _ in 0..3 {
            assert!(arranger.scroll_at_edge(&project));
        }
        assert_eq!(arranger.grid.scroll, Ticks(396));
        assert_eq!(
            arranger.preview(&project),
            Some(Preview::Clips(vec![(Ticks(0), Ticks(5760), 0)]))
        );
        arranger.update(drag((504.0, 16.0)), &project, None, NONE);
        assert_eq!(arranger.edge_scroll(&project), None);
        assert_eq!(
            arranger.update(release((504.0, 16.0)), &project, None, NONE),
            Some(Command::Edit(Edit::ResizeClip {
                id: ClipId(1),
                length: Ticks(2688)
            }))
        );
        assert_eq!(arranger.grid.scroll, Ticks(396));
        assert_eq!(arranger.edge_scroll(&project), None, "let go of");
    }

    // The view at bar 3 (768): clip 2 is at x 120..184, clip 1 out of sight. Its body pressed
    // at x 150 (tick 948) and dragged to x 54, 66 px left of the lanes: each tick moves 132
    // ticks back, six of them reach the start (768 − 5·132 = 108, then 0) and the seventh moves
    // nothing. Let go of at x 184 (tick 384), the drag is 384 − 948 = −564, to the nearest whole
    // bar −384: clip 2 lands on bar 2 (384).
    #[test]
    fn a_drag_past_the_left_edge_scrolls_back_to_the_start() {
        let project = load(P4_TWO_CLIPS);
        let mut arranger = opened();
        scroll_to(&mut arranger, &project, 768.0);
        arranger.update(press((150.0, 16.0)), &project, None, NONE);
        arranger.update(drag((54.0, 16.0)), &project, None, NONE);
        assert_eq!(arranger.edge_scroll(&project), Some(Ticks(636)));
        let mut ticks = 0;
        while arranger.scroll_at_edge(&project) {
            ticks += 1;
        }
        assert_eq!((ticks, arranger.grid.scroll), (6, Ticks(0)));
        assert_eq!(arranger.edge_scroll(&project), None, "at the start");
        assert_eq!(
            arranger.update(release((184.0, 16.0)), &project, None, NONE),
            Some(Command::Edit(Edit::MoveClips {
                moves: vec![(ClipId(2), Ticks(384), TrackId(1))]
            }))
        );
    }

    // A box from x 200 (tick 480) on track 2 dragged past the right edge: two ticks scroll 264
    // ticks, and the corner pressed stays on tick 480, now at x 120 + (480 − 264)/6 = 156. Let
    // go of there, it meets clip 2 (x 204..268) and not clip 1 (x 76..140). A loop drawn from
    // tick 384 past the edge, two ticks on: the cursor over tick 5664 + 264 = 5928, the range
    // 384 to the end of the bar holding it, 6144.
    #[test]
    fn a_box_and_a_loop_drag_keep_their_start_as_the_view_scrolls() {
        let project = load(P4_TWO_CLIPS);
        let mut arranger = opened();
        arranger.update(press((200.0, 50.0)), &project, None, Modifiers::CTRL);
        arranger.update(drag((1064.0, 10.0)), &project, None, NONE);
        arranger.scroll_at_edge(&project);
        arranger.scroll_at_edge(&project);
        assert_eq!(arranger.grid.scroll, Ticks(264));
        assert_eq!(
            arranger.preview(&project),
            Some(Preview::Box(
                Point::new(156.0, 50.0),
                Point::new(1064.0, 10.0)
            ))
        );
        arranger.update(release((1064.0, 10.0)), &project, None, NONE);
        assert_eq!(arranger.selection, BTreeSet::from([ClipId(2)]));

        let mut arranger = opened();
        let strip = PlaylistMessage::RulerPressed {
            at: Point::new(184.0, 20.0),
            button: mouse::Button::Left,
        };
        arranger.update(strip, &project, None, NONE);
        let ruler = PlaylistMessage::RulerDragged(Point::new(1064.0, 20.0));
        arranger.update(ruler, &project, None, NONE);
        arranger.scroll_at_edge(&project);
        arranger.scroll_at_edge(&project);
        assert_eq!(
            arranger.preview(&project),
            Some(Preview::Loop(LoopRange {
                start: Ticks(384),
                end: Ticks(6144),
                on: true
            }))
        );
    }

    // Up and down drags never scroll along: the gain readout of p9_stretch_ui's track 1 clip
    // (x 120..376, readout from x 332 in its 10 px title strip) dragged far right.
    #[test]
    fn a_gain_drag_past_the_edge_moves_nothing() {
        let project = load(include_str!(
            "../../../../tests/fixtures/p9_stretch_ui.json"
        ));
        let mut arranger = opened();
        arranger.update(press((340.0, 5.0)), &project, None, NONE);
        assert!(arranger.tracks_held());
        arranger.update(drag((1500.0, 5.0)), &project, None, NONE);
        assert_eq!(arranger.edge_scroll(&project), None);
        assert!(!arranger.scroll_at_edge(&project));
    }

    // At 256 px a bar the view shows 1320 ticks. A drag held over the lanes leaves Follow as it
    // was: the song position at 1320 pages the view. A drag that scrolled the view past an edge
    // holds it until let go of; then Follow pages again.
    #[test]
    fn follow_waits_while_a_drag_scrolls_past_an_edge() {
        let project = load(P4_TWO_CLIPS);
        let mut arranger = opened();
        arranger.grid.zoom = 4;
        arranger.update(press((500.0, 50.0)), &project, None, Modifiers::CTRL);
        arranger.update(drag((700.0, 40.0)), &project, None, NONE);
        assert!(
            arranger.follow(&project, Ticks(1320)),
            "held over the lanes"
        );
        assert_eq!(arranger.grid.scroll, Ticks(1152));
        arranger.update(release((700.0, 40.0)), &project, None, NONE);
        scroll_to(&mut arranger, &project, 0.0);
        arranger.update(press((500.0, 50.0)), &project, None, Modifiers::CTRL);
        arranger.update(drag((1064.0, 40.0)), &project, None, NONE);
        assert!(arranger.scroll_at_edge(&project));
        assert_eq!(
            arranger.grid.scroll,
            Ticks(33),
            "22 px at 1.5 ticks a pixel"
        );
        assert!(
            !arranger.follow(&project, Ticks(5000)),
            "the drag moves the view"
        );
        arranger.update(release((1064.0, 40.0)), &project, None, NONE);
        assert!(arranger.follow(&project, Ticks(5000)));
        assert_eq!(arranger.grid.scroll, Ticks(4992));
    }
}
