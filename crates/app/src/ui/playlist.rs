//! The playlist panel: a header strip (snap, zoom, Follow, Fit and the link control) over the
//! ruler (bar numbers over the loop strip), the tracks (headers beside lanes of clips) and the
//! timeline's scroll bar at the panel's foot. Both canvases draw from
//! [`playlist_layout`] positions and the [`Arranger`] state, and only report the cursor in their
//! own coordinates; the app turns that into edits. Each keeps what it draws in a cached layer,
//! cleared by the app whenever that changes, and draws the song position and a held drag's
//! outlines over it every frame.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use dawsome_engine::edit::copies;
use dawsome_engine::project::{
    Automation, AutomationId, ChannelId, Clip, ClipId, ClipItem, Curve, LoopRange, Project,
    StretchMode, TrackId, STRETCH_PITCH_RANGE,
};
use dawsome_engine::time::{self, MeterMap, MeterUnit, SignatureChange, Ticks, TimeSignature};
use iced::alignment::{Horizontal, Vertical};
use iced::widget::canvas::{self, Cache, Event, Frame, Geometry, Path, Stroke, Text};
use iced::widget::{button, column, container, pin, row, space, stack, text, text_input, Id};
use iced::{
    mouse, window, Center, Color, Element, Fill, Point, Rectangle, Renderer, Size, Theme, Vector,
};

use crate::app::{ClipLink, MediaMessage, Message, SelectionLink};
use crate::ui::add_words::{self, Look};
use crate::ui::arranger::{Arranger, PlaylistMessage, Preview, CURVE_MENU};
use crate::ui::clip_gesture::ClipSnap;
use crate::ui::clips::{self, FileLook, FileLooks};
use crate::ui::fonts::size;
use crate::ui::grid_rules::{self, device_line, NUMBER_BAND_HEIGHT, STRONG_COMPANION_ALPHA};
use crate::ui::hint::{
    hover_area, hover_change, link_tip, with_tip, ClipControl, ClipRow, Hover, MenuRow,
    PlaylistTool, Pointer, TrackRow,
};
use crate::ui::menu;
use crate::ui::palette;
use crate::ui::panels::{Panel, TITLE_GAP};
use crate::ui::playlist_layout::{
    self, ClipPart, Grid, HeaderHit, Lanes, ARM_CENTER_X, ARM_RADIUS, AUTOMATION_LINE,
    HANDLE_RADIUS, HEADER_HEIGHT, LED_RADIUS, MARKER_WIDTH, NAME_X, POINT_RADIUS, RULER_HEIGHT,
    SELECTED_OUTLINE, TIME_BAR_HEIGHT, TITLE_HEIGHT, TRACK_HEADER_WIDTH, ZOOM,
};
use crate::ui::pop_out;
use crate::ui::rack::draw_mute_light;
use crate::ui::scroll::Scrollbar;
use crate::ui::style;
use crate::ui::text_fit::{fit_width, text_width, widest};

/// The text box a track name is edited in.
pub const RENAME_INPUT: &str = "playlist-track-rename";

/// The track menu's width, as the rack's channel menu's.
const TRACK_MENU_WIDTH: f32 = 160.0;
/// The curve menu's width: its longest row, "Exponential", and a margin.
pub const CURVE_MENU_WIDTH: f32 = 120.0;
/// The clip menu's width; a "Link back to <name>" row longer than it is cut.
const CLIP_MENU_WIDTH: f32 = 220.0;
/// The room a clip menu row's words have: the menu less the row's side padding.
const CLIP_ROW_ROOM: f32 = CLIP_MENU_WIDTH - 20.0;
/// The gap between a clip's name and its "×N".
const BADGE_GAP: f32 = 4.0;
/// The link control's chain, drawn in a square this wide.
const LINK_ICON: f32 = 16.0;
/// The header's cell right of Fit that holds the link control: the chain and the button's 6 px
/// padding either side. Kept while the control is hidden, so nothing in the header moves when
/// a selection shows it.
pub const LINK_CELL_WIDTH: f32 = LINK_ICON + 12.0;

/// Two presses at most this far apart in time and space are a double press.
const DOUBLE_PRESS_TIME: Duration = Duration::from_millis(400);
const DOUBLE_PRESS_DISTANCE: f32 = 6.0;

/// The note marks' layout in a pattern clip ([`mark_rows`]): the room they keep clear under the
/// title and above the lane's foot (2 px of padding and a 2 px margin), a row's greatest pitch
/// (a drum's, or a key's in even rows), a semitone's, the least device pixels a semitone must
/// have for keys to stand by pitch, the least span a pitched channel spreads its keys over, the
/// marks' least and greatest heights (px), and a pitched mark's height in semitones. Why: marks
/// 1 to 2 px tall read as too thin, rows reaching the title and the clip's foot read as
/// clinging to its edges, and a two-key channel spread over the whole body hid its interval;
/// bars that grow with the lane, a least pitch span and margins read as notes.
const MARK_PADDING: f32 = 4.0;
const DRUM_ROW_MOST: f32 = 6.0;
const SEMITONE_MOST: f32 = 3.0;
const SEMITONE_LEAST_PITCH: f32 = 1.5;
const PITCH_SPAN_LEAST: u8 = 12;
const DRUM_MARK: (f32, f32) = (2.0, 6.0);
const PITCHED_MARK: (f32, f32) = (2.0, 4.0);
const PITCHED_MARK_SEMITONES: f32 = 1.2;
/// The least widths of a note's mark and of a step hit's block, device pixels.
const NOTE_LEAST_WIDTH: f32 = 1.0;
const HIT_LEAST_WIDTH: f32 = 2.0;
/// A velocity-0 note's mark alpha; a full one's is 1.
const NOTE_MARK_SOFTEST: f32 = 0.55;

/// What the playlist shows.
pub struct Playlist<'a> {
    pub project: &'a Project,
    pub arranger: &'a Arranger,
    /// The song position, `None` in pattern mode.
    pub marker: Option<Ticks>,
    /// Device pixels per logical pixel, so 1 px lines can be drawn on whole device pixels.
    pub scale_factor: f32,
    pub ruler_cache: &'a Cache,
    pub tracks_cache: &'a Cache,
    /// The audio channels' sounds, by the path the project names their files by and their
    /// edits.
    pub files: &'a FileLooks,
    /// What each automation sets, as its clips are titled; absent when its target is gone.
    pub names: &'a BTreeMap<AutomationId, String>,
    /// The audio clips whose stretched sound is still being made.
    pub stretching: &'a BTreeSet<ClipId>,
    /// The clip whose properties strip the header shows.
    pub properties: Option<ClipProperties>,
    /// A file is being dragged from the browser: the lanes report where it would land.
    pub browser_drag: bool,
    /// The track whose arm light is lit.
    pub armed: Option<TrackId>,
    /// Whether the view jumps a page as the song position leaves it: the header's Follow.
    pub follow: bool,
    /// The window's last pixels the menus over the tracks keep clear of: the hint bar's.
    pub menu_foot: f32,
    /// How the selected pattern clips share their patterns, while the header's link control
    /// shows: at least one of them shares its pattern or plays a copy.
    pub link: Option<SelectionLink>,
    /// Whether the playlist is in the stack, where its header ends with the pop-out button
    /// ([`crate::ui::pop_out`]); in its own window it is not.
    pub docked: bool,
}

/// What the header's properties strip shows of an audio clip: how its file is fitted to it
/// (none, resampled, time-stretched), its pitch (a time-stretch's) and its stretched length
/// over its file's, in percent, once the file is loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipProperties {
    pub mode: Option<StretchMode>,
    pub pitch: i8,
    pub percent: Option<u128>,
}

impl<'a> Playlist<'a> {
    pub fn view(self) -> Element<'a, Message> {
        let preview = self.arranger.preview(self.project);
        // A held loop drag shows the range its release would set, in place of the current one.
        let loop_shown = match &preview {
            Some(Preview::Loop(range)) => Some(*range),
            _ => self.project.playlist.loop_range,
        };
        let ruler = canvas::Canvas::new(Ruler {
            grid: self.arranger.grid,
            meter: &self.arranger.meter,
            changes: &self.project.signature_changes,
            moving: self.arranger.moving_change(),
            loop_shown,
            marker: self.marker,
            scale_factor: self.scale_factor,
            held: self.arranger.ruler_held(),
            cache: self.ruler_cache,
        })
        .width(Fill)
        .height(RULER_HEIGHT);
        let tracks = canvas::Canvas::new(Tracks {
            project: self.project,
            arranger: self.arranger,
            loop_shown,
            preview,
            marker: self.marker,
            scale_factor: self.scale_factor,
            cache: self.tracks_cache,
            files: self.files,
            names: self.names,
            stretching: self.stretching,
            browser_drag: self.browser_drag,
            armed: self.armed,
        })
        .width(Fill)
        .height(Fill);
        let mut layers: Vec<Element<'a, Message>> = vec![tracks.into()];
        layers.extend(self.rename_box());
        layers.extend(self.track_menu());
        layers.extend(self.curve_menu());
        layers.extend(self.clip_menu());
        let tracks: Element<'a, Message> = if layers.len() == 1 {
            layers.remove(0)
        } else {
            stack(layers).into()
        };
        let panel = column![self.header(), ruler, tracks, self.time_bar()];
        let drops = canvas::Canvas::new(DropLayer {
            placed: self.arranger.panel(),
        })
        .width(Fill)
        .height(Fill);
        stack([panel.into(), drops.into()]).into()
    }

    /// The timeline's scroll bar at the panel's foot, as the piano roll's is under its grid: its
    /// rail the panel's whole width, under the track headers too, so the thumb starts at the
    /// panel's left edge at the song's start and ends at its right edge at the farthest scroll.
    /// Why not from the lanes' edge: a thumb 120 px in at the start read as already scrolled.
    fn time_bar(&self) -> Element<'a, Message> {
        canvas::Canvas::new(Scrollbar {
            span: self.arranger.span(self.project),
            message: |message| Message::Playlist(PlaylistMessage::Bar(message)),
        })
        .width(Fill)
        .height(TIME_BAR_HEIGHT)
        .into()
    }

    /// "Playlist", then [`TITLE_GAP`] on the snap choices, the zoom buttons, the Follow and Fit
    /// switches, the link control's cell right of Fit, an audio clip's properties, each 12 px
    /// apart, and, docked, the pop-out button at the right end.
    fn header(&self) -> Element<'a, Message> {
        let snap = self.arranger.snap;
        let snaps = ClipSnap::ALL.map(|choice| {
            let face = button(text(choice.to_string()).size(size::LABEL))
                .padding([2, 8])
                .style(style::tool_button(choice == snap))
                .on_press(Message::Playlist(PlaylistMessage::SnapSelected(choice)));
            hover_area(face, Hover::PlaylistTool(PlaylistTool::Snap))
        });
        let zoom = self.arranger.grid.zoom;
        let zoom_button = |label, message, enabled: bool| {
            button(text(label).size(size::LABEL))
                .padding([2, 8])
                .style(style::tool_button(false))
                .on_press_maybe(enabled.then_some(Message::Playlist(message)))
        };
        let mut content = row![
            row(snaps).spacing(2),
            row![
                hover_area(
                    with_tip(
                        zoom_button("−", PlaylistMessage::ZoomOut, zoom > ZOOM.min),
                        "Zoom out \u{b7} Ctrl+wheel"
                    ),
                    Hover::PlaylistTool(PlaylistTool::ZoomOut)
                ),
                hover_area(
                    with_tip(
                        zoom_button("+", PlaylistMessage::ZoomIn, zoom < ZOOM.max),
                        "Zoom in \u{b7} Ctrl+wheel"
                    ),
                    Hover::PlaylistTool(PlaylistTool::ZoomIn)
                ),
            ]
            .spacing(2),
        ]
        .spacing(12)
        .align_y(Center);
        let follow = button(text("Follow").size(size::LABEL))
            .padding([2, 8])
            .style(style::tool_button(self.follow))
            .on_press(Message::ToggleFollow(Panel::Playlist));
        content = content.push(hover_area(
            follow,
            Hover::PlaylistTool(PlaylistTool::Follow),
        ));
        let fitted = self.arranger.lanes() == Lanes::Fit;
        let fit = button(text("Fit").size(size::LABEL))
            .padding([2, 8])
            .style(style::tool_button(fitted))
            .on_press(Message::Playlist(PlaylistMessage::ToggleFit));
        content = content.push(hover_area(fit, Hover::PlaylistTool(PlaylistTool::Fit)));
        let link = self
            .link
            .as_ref()
            .map_or_else(|| space().into(), |link| link_control(self.project, link));
        content = content.push(container(link).width(LINK_CELL_WIDTH));
        if let Some(properties) = self.properties {
            content = content.push(properties_strip(properties));
        }
        if self.docked {
            content = content
                .push(space().width(Fill))
                .push(pop_out::button(Panel::Playlist));
        }
        let content = row![text("Playlist").size(size::PANEL_TITLE), content]
            .spacing(TITLE_GAP)
            .align_y(Center);
        container(content)
            .padding([0, 12])
            .width(Fill)
            .height(HEADER_HEIGHT)
            .align_y(Center)
            .style(style::strip)
            .into()
    }

    /// The text box over the name of the track being renamed, while its row is in view.
    /// The track menu where the right press on a name was, on the window's top layer, its rows
    /// from [`TrackRow::ALL`].
    fn track_menu(&self) -> Option<Element<'a, Message>> {
        let (_, at) = self.arranger.track_menu?;
        let rows = TrackRow::ALL.map(|row| {
            let label = match row {
                TrackRow::Rename => "Rename",
                TrackRow::Delete => "Delete",
            };
            let pressed = Message::Playlist(PlaylistMessage::TrackMenu(row));
            hover_area(
                menu::row(label, Some(pressed)),
                Hover::Menu(MenuRow::Track(row)),
            )
        });
        let menu = menu::panel(column(rows), TRACK_MENU_WIDTH);
        Some(
            pin(menu::floating(menu, self.menu_foot))
                .x(at.x)
                .y(at.y)
                .into(),
        )
    }

    /// The curve menu where the right press on a tension handle was, on the window's top layer:
    /// a row per curve, from [`CURVE_MENU`].
    fn curve_menu(&self) -> Option<Element<'a, Message>> {
        let at = self.arranger.curve_menu?.at;
        let rows = CURVE_MENU.map(|(label, curve)| {
            menu::row(
                label,
                Some(Message::Playlist(PlaylistMessage::CurveChosen(curve))),
            )
            .into()
        });
        let menu = menu::panel(column(rows), CURVE_MENU_WIDTH);
        Some(
            pin(menu::floating(menu, self.menu_foot))
                .x(at.x)
                .y(at.y)
                .into(),
        )
    }

    /// The clip menu where the Ctrl+right press on a pattern clip was, on the window's top
    /// layer: "Unlink (make unique)" while other clips play its pattern too, "Link back to
    /// <name>" while it plays a copy of a pattern that still exists; with neither, the first row
    /// dimmed, so the menu says what it would do.
    fn clip_menu(&self) -> Option<Element<'a, Message>> {
        let (clip, at) = self.arranger.clip_menu?;
        let rows = clip_rows(self.project, clip)
            .into_iter()
            .map(|(row, label, acts)| {
                let pressed = acts.then_some(Message::Playlist(PlaylistMessage::ClipMenu(row)));
                let label = fit_width(&label, CLIP_ROW_ROOM, size::BODY)
                    .map(|fitted| fitted.into_owned())
                    .unwrap_or_default();
                hover_area(menu::row(label, pressed), Hover::Menu(MenuRow::Clip(row)))
            });
        let menu = menu::panel(column(rows), CLIP_MENU_WIDTH);
        Some(
            pin(menu::floating(menu, self.menu_foot))
                .x(at.x)
                .y(at.y)
                .into(),
        )
    }

    fn rename_box(&self) -> Option<Element<'a, Message>> {
        let (track, name) = self.arranger.rename.as_ref()?;
        let index = self
            .project
            .playlist
            .tracks
            .keys()
            .position(|id| id == track)?;
        let top = self.arranger.grid.row_top_of(index)?;
        let input = text_input("", name)
            .id(Id::new(RENAME_INPUT))
            .on_input(|name| Message::Playlist(PlaylistMessage::RenameEdited(name)))
            .on_submit(Message::Playlist(PlaylistMessage::RenameSubmitted))
            .size(size::LABEL)
            .padding([3, 4])
            .width(TRACK_HEADER_WIDTH - NAME_X);
        Some(pin(input).x(NAME_X - 4.0).y(top + 5.0).into())
    }
}

/// The rows a pattern clip's menu shows for `clip`: each row, its words and whether it acts.
/// Unlink while other clips play its pattern, Link back while it plays a copy of a pattern that
/// still exists (both can apply: a copy other clips play too); neither, Unlink dimmed.
pub fn clip_rows(project: &Project, clip: ClipId) -> Vec<(ClipRow, String, bool)> {
    let shared = matches!(
        copies::link_of(project, clip),
        Some(copies::Link::Linked { .. })
    );
    let mut rows = Vec::new();
    if shared {
        rows.push((ClipRow::Unlink, "Unlink (make unique)".to_owned(), true));
    }
    if let Some(original) = copies::original_of(project, clip) {
        let name = project
            .patterns
            .get(&original)
            .map_or("", |pattern| pattern.name.as_str());
        rows.push((ClipRow::LinkBack, format!("Link back to {name}"), true));
    }
    if rows.is_empty() {
        rows.push((ClipRow::Unlink, "Unlink (make unique)".to_owned(), false));
    }
    if signature_differs(project, &project.meter_map(), clip).is_some() {
        rows.push((
            ClipRow::UseSignature,
            "Use the time signature here".to_owned(),
            true,
        ));
    }
    rows
}

/// The signature `clip`'s pattern counts in, when it is not the song's meter at the clip's
/// start ("Verse · 3/4" in its title, "Use the time signature here" in its menu); `None` for
/// one that matches, and for a clip that plays no pattern.
pub fn signature_differs(
    project: &Project,
    meter: &MeterMap,
    clip: ClipId,
) -> Option<TimeSignature> {
    let clip = project.playlist.clips.get(&clip)?;
    let own = project.pattern_signature(clip.pattern()?);
    (own != meter.signature_at(clip.start)).then_some(own)
}

/// The header's link control for the selected clips, shown as `link`: a chain, with a red X over
/// it once they play copies of their own; its tip says what a click does.
fn link_control<'a>(project: &Project, link: &SelectionLink) -> Element<'a, Message> {
    let unlinked = matches!(link, SelectionLink::Unlinked { .. });
    let icon = canvas::Canvas::new(ChainIcon { unlinked })
        .width(LINK_ICON)
        .height(LINK_ICON);
    let control = button(icon)
        .padding([2, 6])
        .style(style::tool_button(false))
        .on_press(Message::ClipLink(ClipLink::Selected));
    hover_area(
        with_tip(control, link_tip(project, link)),
        Hover::PlaylistTool(PlaylistTool::Link),
    )
}

/// Two links of a chain, drawn as stroked rounded bars along the square's rising diagonal, and
/// a red X over them when `unlinked`.
struct ChainIcon {
    unlinked: bool,
}

impl canvas::Program<Message> for ChainIcon {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        // Drawn in a 16 px square and scaled to the canvas.
        frame.scale(bounds.width.min(bounds.height) / 16.0);
        let line = |color, width| Stroke::default().with_color(color).with_width(width);
        // Each link 9 long and 4.5 wide, their centers 5.7 apart: they overlap as links do.
        for (x, y) in [(6.0, 10.0), (10.0, 6.0)] {
            frame.with_save(|frame| {
                frame.translate(Vector::new(x, y));
                frame.rotate(-std::f32::consts::FRAC_PI_4);
                let link = Path::rounded_rectangle(
                    Point::new(-4.5, -2.25),
                    Size::new(9.0, 4.5),
                    2.25.into(),
                );
                frame.stroke(&link, line(style::palette().text, 1.6));
            });
        }
        if self.unlinked {
            let cross = line(style::RECORD_ARMED, 2.0);
            frame.stroke(
                &Path::line(Point::new(2.5, 2.5), Point::new(13.5, 13.5)),
                cross,
            );
            frame.stroke(
                &Path::line(Point::new(13.5, 2.5), Point::new(2.5, 13.5)),
                cross,
            );
        }
        vec![frame.into_geometry()]
    }
}

/// What a pattern clip shows at the right end of its title when `count` clips play its
/// pattern: "×N" while it is more than one, so a shared pattern shows before an edit; nothing
/// for a clip alone on its pattern.
pub fn link_badge(count: usize) -> Option<String> {
    (count > 1).then(|| format!("\u{d7}{count}"))
}

/// The pitch readout's words for a time-stretched clip `semitones` up or down.
fn pitch_words(semitones: i8) -> String {
    match semitones {
        0 => "0 st".to_owned(),
        up if up > 0 => format!("+{up} st"),
        down => format!("\u{2212}{} st", down.unsigned_abs()),
    }
}

/// The room the pitch readout keeps between − and +: its widest value's over the stretch's
/// range, so + stays put as the pitch steps (the type scale's digits are proportional).
fn pitch_width() -> f32 {
    static WIDTH: OnceLock<f32> = OnceLock::new();
    *WIDTH.get_or_init(|| widest(STRETCH_PITCH_RANGE.map(pitch_words), size::DENSE))
}

/// An audio clip's properties in the header: Stretch "None / Resample / Stretch", the pitch
/// stepper (for a time-stretch), the length in percent of the file's, and a close button.
fn properties_strip<'a>(properties: ClipProperties) -> Element<'a, Message> {
    let send = |message| Message::Playlist(message);
    let small = |control: ClipControl, label: String, lit: bool, message: Message| {
        let face = button(text(label).size(size::LABEL))
            .padding([2, 8])
            .style(style::tool_button(lit))
            .on_press(message);
        hover_area(face, Hover::ClipControl(control))
    };
    let stretched = properties.mode == Some(StretchMode::Stretch);
    let pitch = pitch_words(properties.pitch);
    let mut modes = row![].spacing(2);
    let mut pitches = row![text("Pitch").size(size::LABEL)]
        .spacing(4)
        .align_y(Center);
    let mut close = None;
    // Every control comes from the list the hint table's test walks.
    for control in ClipControl::ALL {
        let mode = |mode: Option<StretchMode>, label: &str| {
            small(
                control,
                label.to_owned(),
                properties.mode == mode,
                send(PlaylistMessage::ClipMode(mode)),
            )
        };
        match control {
            ClipControl::NoStretch => modes = modes.push(mode(None, "None")),
            ClipControl::Resample => {
                modes = modes.push(mode(Some(StretchMode::Resample), "Resample"))
            }
            ClipControl::Stretch => modes = modes.push(mode(Some(StretchMode::Stretch), "Stretch")),
            ClipControl::PitchDown => {
                let step = small(
                    control,
                    "\u{2212}".to_owned(),
                    false,
                    send(PlaylistMessage::ClipPitch(-1)),
                );
                pitches = pitches.push(step).push(
                    text(pitch.clone())
                        .size(size::DENSE)
                        .width(pitch_width())
                        .align_x(Horizontal::Right),
                );
            }
            ClipControl::PitchUp => {
                pitches = pitches.push(small(
                    control,
                    "+".to_owned(),
                    false,
                    send(PlaylistMessage::ClipPitch(1)),
                ));
            }
            ClipControl::Close => {
                close = Some(small(
                    control,
                    "\u{d7}".to_owned(),
                    false,
                    send(PlaylistMessage::CloseProperties),
                ));
            }
        }
    }
    let mut strip = row![text("Stretch").size(size::LABEL), modes]
        .spacing(6)
        .align_y(Center);
    if stretched {
        strip = strip.push(pitches);
    }
    if let Some(percent) = properties.percent {
        strip = strip.push(text(format!("Length {percent}%")).size(size::LABEL));
    }
    strip.push(close).into()
}

/// The bar-number band over the loop strip, the song's changes of signature flagged in it.
struct Ruler<'a> {
    grid: Grid,
    meter: &'a MeterMap,
    /// The song's changes, each flagged at its bar line with its signature.
    changes: &'a [SignatureChange],
    /// A flag held in a drag, and the tick it would land on.
    moving: Option<(Ticks, Ticks)>,
    loop_shown: Option<LoopRange>,
    marker: Option<Ticks>,
    scale_factor: f32,
    /// Whether a loop drag is held, so moves and the release are reported anywhere.
    held: bool,
    cache: &'a Cache,
}

impl Ruler<'_> {
    fn draw_body(&self, frame: &mut Frame) {
        let size = frame.size();
        frame.fill_rectangle(Point::ORIGIN, size, style::palette().bg);
        if let Some(range) = &self.loop_shown {
            if let Some((left, right)) =
                visible_span(&self.grid, range.start, range.end, size.width)
            {
                let color = if range.on {
                    style::LOOP_ON
                } else {
                    style::LOOP_OFF
                };
                frame.fill_rectangle(
                    Point::new(left, NUMBER_BAND_HEIGHT),
                    Size::new(right - left, RULER_HEIGHT - NUMBER_BAND_HEIGHT),
                    color,
                );
            }
        }
        // A bar a change starts shows its flag where its number would be: text drawn under a
        // flag shows through it in the software renderer.
        let flagged: BTreeSet<i64> = self
            .changes
            .iter()
            .map(|change| self.meter.bar_at(change.at).bar + 1)
            .collect();
        let bars: Vec<(i64, f32)> =
            playlist_layout::visible_bars(&self.grid, self.meter, size.width).collect();
        // Numbered as often as the narrowest bar in view has room for: a change to a shorter
        // bar must not run its numbers into each other. (A bar as long as the first meter's
        // keeps the zoom's own pixels per bar, exactly.)
        let (from, to) = (self.grid.scroll, self.grid.tick_at(size.width));
        let segments = self.meter.segments();
        let narrowest = segments
            .iter()
            .enumerate()
            .filter(|&(index, segment)| {
                let ends = segments
                    .get(index + 1)
                    .map_or(Ticks(i64::MAX), |next| next.start);
                segment.start <= to && ends > from
            })
            .map(|(_, segment)| segment.signature.bar_ticks(self.meter.ppq()))
            .filter(|&bar| bar < self.grid.bar)
            .map(|bar| self.grid.px_of(bar))
            .fold(self.grid.px_per_bar(), f32::min);
        let shown: Vec<(i64, f32)> = bars
            .into_iter()
            .filter(|(number, _)| !flagged.contains(number))
            .collect();
        grid_rules::draw_bar_numbers(frame, &shown, narrowest, self.scale_factor);
    }

    /// The changes' flags, over the numbers: drawn every frame (not cached), so a flag held in
    /// a drag follows the pointer.
    fn draw_flags(&self, frame: &mut Frame) {
        let width = frame.width();
        for change in self.changes {
            let at = match self.moving {
                Some((from, to)) if from == change.at => to,
                _ => change.at,
            };
            draw_flag(frame, &self.grid, at, change.signature, width);
        }
    }
}

/// A change of signature's flag in the ruler's number band ([`playlist_layout::flag_rect`]):
/// the readouts' glass with the signature in their digits' colour, its pole down the bar line
/// ([`playlist_layout::FLAG_POLE`] wide) in the accent's dimmer tone ([`palette::slider_fill`]);
/// none for a bar line off the timeline. Why the glass and not the accent's tone under the
/// digits: text on that tone is under WCAG's 4.5:1 in every theme (3.0 to 4.1), the readouts'
/// pair is held above it.
fn draw_flag(frame: &mut Frame, grid: &Grid, at: Ticks, signature: TimeSignature, width: f32) {
    let Some(rect) = playlist_layout::flag_rect(grid, at, signature, width) else {
        return;
    };
    frame.fill_rectangle(rect.position(), rect.size(), style::palette().lcd);
    frame.fill_rectangle(
        rect.position(),
        Size::new(playlist_layout::FLAG_POLE, rect.height),
        palette::slider_fill(),
    );
    frame.fill_text(canvas::Text {
        content: signature.to_string(),
        position: Point::new(
            rect.x + playlist_layout::FLAG_POLE + playlist_layout::FLAG_PAD,
            NUMBER_BAND_HEIGHT / 2.0,
        ),
        color: style::palette().lcd_text,
        size: size::DENSE.into(),
        align_y: Vertical::Center,
        ..canvas::Text::default()
    });
}

/// Where a clip is drawn: its ID, the top of its row, whether it is selected and its track
/// audible, and the canvas's top in the window, so horizontal lines can be laid on whole
/// device pixels.
#[derive(Debug, Clone, Copy)]
struct ClipAt {
    id: ClipId,
    top: f32,
    selected: bool,
    audible: bool,
    origin_y: f32,
    /// How many clips play this clip's pattern (0 for other kinds): its "×N".
    shares: usize,
}

/// An audio clip's gain, right-aligned at the end of its title strip `rect`, cut to start no
/// further left than `shown`, the part of the strip in view.
fn draw_gain(frame: &mut Frame, rect: Rectangle, shown: Rectangle, gain: f32) {
    let end = rect.x + rect.width - 3.0;
    let words = clips::gain_text(gain);
    let Some(fitted) = fit_width(&words, end - shown.x, size::TINY) else {
        return;
    };
    frame.fill_text(Text {
        content: fitted.into_owned(),
        position: Point::new(end, rect.y + TITLE_HEIGHT / 2.0),
        color: style::CLIP_TITLE_TEXT,
        size: size::TINY.into(),
        align_x: Horizontal::Right.into(),
        align_y: Vertical::Center,
        ..Text::default()
    });
}

/// A segment's tension handle: where it is drawn, and the tick of the point that starts the
/// segment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Handle {
    pub at: Point,
    pub tick: Ticks,
}

/// The points of `automation` that `clip` shows, in a clip body `body`: each one's data tick
/// and where it is drawn.
pub fn automation_points(
    grid: &Grid,
    clip: &Clip,
    automation: &Automation,
    body: Rectangle,
) -> Vec<(Ticks, Point)> {
    let (top, bottom) = (body.y, body.y + body.height);
    let base = clip.start.0 - clip.offset.0;
    automation
        .points
        .iter()
        .filter_map(|point| {
            let tick = Ticks(base + point.tick.0);
            (clip.start..=clip.end()).contains(&tick).then(|| {
                let y = playlist_layout::automation_y(top, bottom, point.value);
                (point.tick, Point::new(grid.x_of(tick), y))
            })
        })
        .collect()
}

/// The tension handles `clip` shows in `body`: one at the middle of each segment that runs
/// between two values (a hold steps, so it has none), on the curve.
pub fn automation_handles(
    grid: &Grid,
    clip: &Clip,
    automation: &Automation,
    body: Rectangle,
) -> Vec<Handle> {
    let (top, bottom) = (body.y, body.y + body.height);
    let base = clip.start.0 - clip.offset.0;
    automation
        .points
        .windows(2)
        .filter(|pair| pair[0].value != pair[1].value && pair[0].curve != Curve::Hold)
        .filter_map(|pair| {
            let middle = Ticks(base + (pair[0].tick.0 + pair[1].tick.0) / 2);
            (clip.start..clip.end()).contains(&middle).then(|| {
                let value = clips::value_at(automation, clip, middle);
                Handle {
                    at: Point::new(
                        grid.x_of(middle),
                        playlist_layout::automation_y(top, bottom, value),
                    ),
                    tick: pair[0].tick,
                }
            })
        })
        .collect()
}

/// Where the mark of a note from `x` to `end_x` lies, its left edge and width, on whole device
/// pixels at `scale_factor`: a bar from the note's start to one device pixel before its end, at
/// least `least` device pixels wide (a step hit's end one step on: a block a step wide);
/// starting inside the clip's `body` (from right of the clip's start line to where the next
/// clip's start line begins), cut at the body's end (a note running past the clip's end, on any
/// pass) and to the visible `span`; none when nothing of it shows.
///
/// Why bars: a mark at each note's start alone made a recorded take read as scattered dots, not
/// the notes played as the piano roll shows them. Why a pixel short of the end: notes played
/// legato, each ending where the next starts, fused into one bar. Why inside the body: a note
/// on a pattern's first tick lies on the clip's start line, and one on its last steps reaches
/// past the clip's end once zoomed out; drawn there, the line being near the lane's color, the
/// mark read as a note outside the clip, or as the end of the clip before. Why device pixels: a
/// mark edge between two pixels is blended into the clip's body, so the one-pixel gap between
/// two notes would be two half-lit pixels.
fn mark_span(
    x: f32,
    end_x: f32,
    least: f32,
    body: (f32, f32),
    span: (f32, f32),
    scale_factor: f32,
) -> Option<(f32, f32)> {
    let scale = if scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    // The body's edges lie on device pixels already (`device_line`), the view's near enough.
    let device = |x: f32| (x * scale).round();
    let (start, end) = (device(body.0), device(body.1));
    let left = (x * scale).floor().min(end - least).max(start);
    let right = ((end_x * scale).floor() - 1.0).min(end).max(left + least);
    let (left, right) = (left.max(device(span.0)), right.min(device(span.1)));
    (left < right).then_some((left / scale, (right - left) / scale))
}

/// A note's mark colour: the marks' colour, its alpha from [`NOTE_MARK_SOFTEST`] at velocity 0
/// to 1 at full, so accents and ghost notes read apart in a clip as in the piano roll.
fn note_mark_color(velocity: f32) -> Color {
    Color {
        a: NOTE_MARK_SOFTEST + (1.0 - NOTE_MARK_SOFTEST) * velocity.clamp(0.0, 1.0),
        ..style::CLIP_NOTE
    }
}

/// One channel's rows in a pattern clip: the keys it uses, highest first, and whether they are
/// drum rows (one key, or step hits alone), stacked a row a key rather than placed by pitch.
#[derive(Debug, Clone, PartialEq)]
struct MarkBand {
    keys: Vec<u8>,
    drum: bool,
}

impl MarkBand {
    /// The semitones a pitched band spreads its keys over: theirs, at least
    /// [`PITCH_SPAN_LEAST`], centred on them.
    fn span(&self) -> f32 {
        f32::from(self.used().max(PITCH_SPAN_LEAST))
    }

    fn used(&self) -> u8 {
        match (self.keys.first(), self.keys.last()) {
            (Some(&high), Some(&low)) => high.saturating_sub(low),
            _ => 0,
        }
    }

    /// The band's height at its fullest (px): by pitch, the span at [`SEMITONE_MOST`] a semitone
    /// and the tallest mark's room and a pixel, so its top and bottom keys' marks stay inside it,
    /// clear of the next band's; in even rows (a drum's), [`DRUM_ROW_MOST`] a key.
    fn fullest(&self, by_pitch: bool) -> f32 {
        if by_pitch {
            self.span() * SEMITONE_MOST + PITCHED_MARK.1 + 1.0
        } else {
            self.keys.len() as f32 * DRUM_ROW_MOST
        }
    }
}

/// Every note mark row of a pattern clip in a lane from `lane_top`, `lane` tall, at
/// `scale_factor`: for each key of each of `bands` (the channels in rack order), in that order,
/// its mark's top and height (px), on whole device pixels.
///
/// The marks keep [`MARK_PADDING`] clear under the title and above the lane's foot, always.
/// Each band takes its fullest height while that room holds all of them, the whole stack
/// centred, so a clip of few rows is a compact block in the middle, not spread from edge to
/// edge; with less room every band shrinks by one factor. A drum's rows stack evenly, a mark
/// its row's pitch less a pixel ([`DRUM_MARK`]). A pitched channel's keys stand by semitone over
/// its span, so a chord shows its intervals and a melody its contour, a mark
/// [`PITCHED_MARK_SEMITONES`] semitones tall ([`PITCHED_MARK`]), while the shrink leaves a
/// semitone [`SEMITONE_LEAST_PITCH`] device pixels; with less, its keys stack evenly as a
/// drum's, 2 px marks (then the bands are shared again, those rows at their own fullest). Rows
/// under 3 px apart cannot hold a 2 px mark and a gap each; there they touch, as they must.
fn mark_rows(lane_top: f32, lane: f32, bands: &[MarkBand], scale_factor: f32) -> Vec<(f32, f32)> {
    let scale = if scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    // In device pixels from here on.
    let device = |px: f32| px * scale;
    let body_top = device(lane_top + TITLE_HEIGHT + MARK_PADDING);
    let room = device(lane - TITLE_HEIGHT - 2.0 * MARK_PADDING);
    let total = |by_pitch: &[bool]| -> f32 {
        bands
            .iter()
            .zip(by_pitch)
            .map(|(band, &by_pitch)| device(band.fullest(by_pitch)))
            .sum()
    };
    // A length at the fullest as the shrink to `whole` leaves it. (Multiplied before dividing,
    // so a length that comes out a whole number of pixels is one.)
    let shrunk = |length: f32, whole: f32| {
        if whole > room {
            length * room / whole
        } else {
            length
        }
    };
    let pitched: Vec<bool> = bands.iter().map(|band| !band.drum).collect();
    let first = total(&pitched);
    let by_pitch: Vec<bool> = pitched
        .iter()
        .map(|&pitched| pitched && shrunk(device(SEMITONE_MOST), first) >= SEMITONE_LEAST_PITCH)
        .collect();
    let whole = total(&by_pitch);
    let top = (body_top + ((room - whole) / 2.0).max(0.0)).round();
    let place = |length: f32| top + shrunk(length, whole);
    // A mark from its row's centre, on whole pixels.
    let mark_at =
        |centre: f32, height: f32| ((centre - height / 2.0).round() / scale, height / scale);
    let least = device(DRUM_MARK.0).round();
    let mut rows = Vec::new();
    let mut before = 0.0;
    for (band, &by_pitch) in bands.iter().zip(&by_pitch) {
        if by_pitch {
            let semitone = shrunk(device(SEMITONE_MOST), whole);
            let height = (PITCHED_MARK_SEMITONES * semitone)
                .round()
                .clamp(least, device(PITCHED_MARK.1).round());
            let edge = device(PITCHED_MARK.1 + 1.0) / 2.0;
            let high = f32::from(band.keys[0]) + (band.span() - f32::from(band.used())) / 2.0;
            rows.extend(band.keys.iter().map(|&key| {
                let offset = edge + (high - f32::from(key)) * device(SEMITONE_MOST);
                mark_at(place(before + offset), height)
            }));
        } else {
            let pitch = shrunk(device(DRUM_ROW_MOST), whole);
            // A drum's mark a pixel less than the pitch's whole pixels, which two row tops on
            // whole pixels lie apart at least: a pixel's gap between rows.
            let height = if band.drum {
                (pitch.floor() - 1.0).clamp(least, device(DRUM_MARK.1).round())
            } else {
                least
            };
            rows.extend((0..band.keys.len()).map(|row| {
                let offset = (row as f32 + 0.5) * device(DRUM_ROW_MOST);
                mark_at(place(before + offset), height)
            }));
        }
        before += device(band.fullest(by_pitch));
    }
    rows
}

/// The visible x span of ticks `start..end` on a canvas `width` wide, right of the headers.
fn visible_span(grid: &Grid, start: Ticks, end: Ticks, width: f32) -> Option<(f32, f32)> {
    let left = grid.x_of(start).max(TRACK_HEADER_WIDTH);
    let right = grid.x_of(end).min(width);
    (right > left).then_some((left, right))
}

impl canvas::Program<Message> for Ruler<'_> {
    type State = Pointer;

    fn update(
        &self,
        state: &mut Pointer,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let send = |message| canvas::Action::publish(Message::Playlist(message));
        let local = |position: Point| Point::new(position.x - bounds.x, position.y - bounds.y);
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(
                button @ (mouse::Button::Left | mouse::Button::Right),
            )) => cursor.position_in(bounds).map(|at| {
                state.last = Some(at);
                send(PlaylistMessage::RulerPressed {
                    at,
                    button: *button,
                })
                .and_capture()
            }),
            Event::Mouse(mouse::Event::CursorMoved { position }) if self.held => {
                let at = local(*position);
                state.last = Some(at);
                Some(send(PlaylistMessage::RulerDragged(at)))
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let now = cursor
                    .position_in(bounds)
                    .filter(|at| at.x >= TRACK_HEADER_WIDTH)
                    .and_then(|at| playlist_layout::ruler_band(at.y))
                    .map(Hover::Ruler);
                hover_change(&mut state.hover, now)
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if self.held => cursor
                .position()
                .map(local)
                .or(state.last)
                .map(|at| send(PlaylistMessage::RulerReleased(at))),
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => cursor
                .position_in(bounds)
                .map(|at| send(PlaylistMessage::RulerScrolled { at, delta: *delta }).and_capture()),
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &Pointer,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let body = self
            .cache
            .draw(renderer, bounds.size(), |frame| self.draw_body(frame));
        let mut overlay = Frame::new(renderer, bounds.size());
        self.draw_flags(&mut overlay);
        if let Some(tick) = self.marker {
            let x = self.grid.x_of(tick);
            if (TRACK_HEADER_WIDTH..bounds.width).contains(&x) {
                let half = MARKER_WIDTH / 2.0;
                let triangle = Path::new(|path| {
                    path.move_to(Point::new(x - half, 0.0));
                    path.line_to(Point::new(x + half, 0.0));
                    path.line_to(Point::new(x, MARKER_WIDTH));
                    path.close();
                });
                overlay.fill(&triangle, style::accent());
            }
        }
        vec![body, overlay.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _state: &Pointer,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if self.held {
            return mouse::Interaction::ResizingHorizontally;
        }
        let Some(at) = cursor.position_in(bounds) else {
            return mouse::Interaction::default();
        };
        let near_edge = self.loop_shown.as_ref().is_some_and(|range| {
            playlist_layout::ruler_band(at.y) == Some(playlist_layout::RulerBand::LoopStrip)
                && playlist_layout::loop_edge_under(&self.grid, range, at.x).is_some()
        });
        if near_edge {
            mouse::Interaction::ResizingHorizontally
        } else if at.x >= TRACK_HEADER_WIDTH {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}

/// A layer over the whole panel, drawing nothing and leaving the cursor to what is under it: a
/// file dropped from the desktop on the panel is its, so the window does not place it too, and
/// it says where the window saw the cursor then ([`MediaMessage::DroppedOnPlaylist`]); the app
/// asks the window system where the pointer is and lands the file by that. It reports where the
/// panel is in the window ([`PlaylistMessage::Placed`]), which the drop is measured against.
struct DropLayer {
    /// Where the panel was last reported.
    placed: Rectangle,
}

impl canvas::Program<Message> for DropLayer {
    type State = ();

    fn update(
        &self,
        _state: &mut (),
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            Event::Window(window::Event::FileDropped(file)) => {
                cursor.position_over(bounds).map(|seen| {
                    let dropped = MediaMessage::DroppedOnPlaylist {
                        file: file.clone(),
                        seen,
                    };
                    canvas::Action::publish(Message::Media(dropped)).and_capture()
                })
            }
            // Reported on an event of any kind, which goes on to the panel under the layer.
            _ => (bounds != self.placed).then(|| {
                canvas::Action::publish(Message::Playlist(PlaylistMessage::Placed(bounds)))
            }),
        }
    }

    fn draw(
        &self,
        _state: &(),
        _renderer: &Renderer,
        _theme: &Theme,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        Vec::new()
    }
}

/// The track headers and their lanes of clips.
struct Tracks<'a> {
    project: &'a Project,
    arranger: &'a Arranger,
    loop_shown: Option<LoopRange>,
    preview: Option<Preview>,
    marker: Option<Ticks>,
    scale_factor: f32,
    cache: &'a Cache,
    files: &'a FileLooks,
    names: &'a BTreeMap<AutomationId, String>,
    stretching: &'a BTreeSet<ClipId>,
    browser_drag: bool,
    armed: Option<TrackId>,
}

/// The tracks canvas's memory between events.
#[derive(Debug, Default)]
struct TracksState {
    /// Where the cursor was last seen during a drag, for a release with no cursor position.
    last: Option<Point>,
    /// The last left press, to spot a double press.
    previous_press: Option<(Point, Instant)>,
    /// What the cursor was over last.
    hover: Option<Hover>,
}

impl Tracks<'_> {
    /// What the hint bar names at tracks point `at`: a track header's control (its row counted
    /// from the first track), a clip and the part of it, or an empty lane.
    fn hover_at(&self, at: Point) -> Option<Hover> {
        let tracks = self.project.playlist.tracks.len();
        if at.x < TRACK_HEADER_WIDTH {
            let top = self.grid().top;
            let rows = tracks.saturating_sub(top);
            let hit = match playlist_layout::header_hit(self.grid(), at, rows)? {
                HeaderHit::Led { row } => HeaderHit::Led { row: top + row },
                HeaderHit::Name { row } => HeaderHit::Name { row: top + row },
                HeaderHit::Arm { row } => HeaderHit::Arm { row: top + row },
                HeaderHit::AddTrack => HeaderHit::AddTrack,
            };
            return Some(Hover::Track(hit));
        }
        if let Some((clip, part)) = self.arranger.clip_under(self.project, at) {
            return Some(Hover::Clip { clip, part });
        }
        let index = self.grid().shown_index_at(at.y)?;
        (index < tracks).then_some(Hover::Lane)
    }

    fn grid(&self) -> &Grid {
        &self.arranger.grid
    }

    /// The tracks from the top row down, with their rows: those in the rows shown whole.
    fn shown_tracks(
        &self,
    ) -> impl Iterator<Item = (usize, TrackId, &dawsome_engine::project::Track)> {
        self.project
            .playlist
            .tracks
            .iter()
            .skip(self.grid().top)
            .take(self.grid().rows())
            .enumerate()
            .map(|(row, (&id, track))| (row, id, track))
    }

    /// Where the lanes end: under the last track shown, at most the canvas's height.
    fn lanes_bottom(&self, height: f32) -> f32 {
        let below = self
            .project
            .playlist
            .tracks
            .len()
            .saturating_sub(self.grid().top);
        let rows = below.min(self.grid().rows());
        (self.grid().row_top(rows) - playlist_layout::LANE_GAP).clamp(0.0, height)
    }

    fn draw_body(&self, frame: &mut Frame, origin_y: f32) {
        let size = frame.size();
        let grid = self.grid();
        frame.fill_rectangle(Point::ORIGIN, size, style::palette().bg);
        let tracks: Vec<_> = self.shown_tracks().collect();
        let tops: Vec<f32> = tracks.iter().map(|&(row, ..)| grid.row_top(row)).collect();
        let lane_width = (size.width - TRACK_HEADER_WIDTH).max(0.0);
        for &top in &tops {
            frame.fill_rectangle(
                Point::new(TRACK_HEADER_WIDTH, top),
                Size::new(lane_width, grid.lane),
                style::palette().lane,
            );
        }
        self.draw_shading(frame, &tops, size.width);
        // Over the shading: the loop range's lanes keep its tint whole.
        let tint = self
            .loop_shown
            .filter(|range| range.on)
            .and_then(|range| visible_span(grid, range.start, range.end, size.width));
        if let Some((left, right)) = tint {
            for &top in &tops {
                frame.fill_rectangle(
                    Point::new(left, top),
                    Size::new(right - left, grid.lane),
                    style::palette().loop_tint,
                );
            }
        }
        self.draw_lines(frame, size);
        let rows: BTreeMap<TrackId, (usize, bool)> = tracks
            .iter()
            .map(|&(row, id, track)| (id, (row, !track.mute)))
            .collect();
        let mut clips: Vec<(ClipId, &Clip)> = self
            .project
            .playlist
            .clips
            .iter()
            .filter(|(_, clip)| rows.contains_key(&clip.track))
            .map(|(&id, clip)| (id, clip))
            .collect();
        // The order clip hits are resolved in: a later start is drawn over an earlier one.
        clips.sort_by_key(|&(id, clip)| (clip.start, id));
        // Counted over the whole playlist once, not per clip: a clip off screen shares too.
        let counts = copies::clip_counts(self.project);
        for (id, clip) in clips {
            let (row, audible) = rows[&clip.track];
            let selected = self.arranger.selection.contains(&id);
            let shares = clip
                .pattern()
                .and_then(|pattern| counts.get(&pattern))
                .copied()
                .unwrap_or(0);
            let at = ClipAt {
                id,
                top: grid.row_top(row),
                selected,
                audible,
                origin_y,
                shares,
            };
            self.draw_clip(frame, clip, at);
        }
        // The headers go on top, covering clips scrolled to the left of the timeline.
        self.draw_headers(frame, &tracks);
    }

    /// The shaded bars ([`playlist_layout::shaded_spans`]) in the lanes whose tops are `tops`,
    /// as one path. Their edges are moved onto the device pixels the bar lines start on, so the
    /// lines cover them and no blended pixel shows beside a line.
    fn draw_shading(&self, frame: &mut Frame, tops: &[f32], width: f32) {
        let grid = self.grid();
        let on_line = |x: f32| device_line(x, self.scale_factor).0;
        let spans: Vec<(f32, f32)> =
            playlist_layout::shaded_spans(grid, &self.arranger.meter, width)
                .map(|(left, right)| (on_line(left), on_line(right)))
                .collect();
        if spans.is_empty() || tops.is_empty() {
            return;
        }
        let shading = Path::new(|path| {
            for &top in tops {
                for &(left, right) in &spans {
                    path.rectangle(Point::new(left, top), Size::new(right - left, grid.lane));
                }
            }
        });
        frame.fill(&shading, style::palette().lane_alt);
    }

    /// The lines through every lane, 1 px on whole device pixels, in the song's meter (each
    /// change's beats and bars): the beat lines at the strength the zoom gives a beat of their
    /// meter ([`grid_rules::line_alpha`] of its width; none while it is 0), then a line at every
    /// bar, the one starting every four bars by number ([`grid_rules::strong`]) in the brighter
    /// `bar_line` ([`crate::ui::palette::Palette::bar_line`]) with a fainter one right of it.
    fn draw_lines(&self, frame: &mut Frame, size: Size) {
        let grid = self.grid();
        let meter = &self.arranger.meter;
        let bottom = self.lanes_bottom(size.height);
        let last = grid.tick_at(size.width);
        let snapped = |tick: Ticks| device_line(grid.x_of(tick), self.scale_factor);
        let mut fill = |left: f32, width: f32, color: Color| {
            frame.fill_rectangle(Point::new(left, 0.0), Size::new(width, bottom), color);
        };
        let mut tick = meter.snap_down(grid.scroll, MeterUnit::Beat);
        if tick < grid.scroll {
            tick = meter.snap_next(tick, MeterUnit::Beat);
        }
        while tick <= last {
            let beat = meter.signature_at(tick).beat_ticks(meter.ppq());
            let alpha = grid_rules::line_alpha(grid.px_of(beat));
            if alpha > 0.0 && meter.bar_at(tick).start != tick {
                let (left, width) = snapped(tick);
                let color = Color {
                    a: alpha,
                    ..style::palette().lane_beat_line
                };
                fill(left, width, color);
            }
            tick = meter.snap_next(tick, MeterUnit::Beat);
        }
        let companion = Color {
            a: STRONG_COMPANION_ALPHA,
            ..style::palette().bar_line
        };
        for (number, x) in playlist_layout::visible_bars(grid, meter, size.width) {
            let (left, width) = device_line(x, self.scale_factor);
            if grid_rules::strong(number - 1) {
                fill(left, width, style::palette().bar_line);
                // On the device pixels right after the line's: snapped again, the sum could
                // floor back onto the line's last pixel.
                fill(left + width, width, companion);
            } else {
                fill(left, width, style::palette().lane_bar_line);
            }
        }
    }

    /// One clip in the row whose top is `at.top`: its title strip and name (and, for a
    /// stretched audio clip, its stretch), its body, what it plays (a mark for every note of a
    /// pattern clip's window, an audio clip's waveform, an automation clip's line and points),
    /// and an outline when selected.
    fn draw_clip(&self, frame: &mut Frame, clip: &Clip, at: ClipAt) {
        let ClipAt {
            id,
            top,
            selected,
            audible,
            origin_y,
            shares,
        } = at;
        let grid = self.grid();
        let lane = grid.lane;
        let rect = playlist_layout::clip_rect(grid, clip, top);
        let Some((left, right)) = visible_span(grid, clip.start, clip.end(), frame.width()) else {
            return;
        };
        let muted = clip.muted || !audible;
        let width = right - left;
        let title_rect = Rectangle::new(Point::new(left, top), Size::new(width, TITLE_HEIGHT));
        let body_rect = Rectangle::new(
            Point::new(left, top + TITLE_HEIGHT),
            Size::new(width, lane - TITLE_HEIGHT),
        );
        let (title_color, body_color, name) = match clip.item {
            // The pattern's signature after its name when the song's meter differs there.
            ClipItem::Pattern { pattern } => (
                style::CLIP_TITLE,
                style::CLIP_BODY,
                self.project.patterns.get(&pattern).map(|shown| {
                    match signature_differs(self.project, &self.arranger.meter, id) {
                        Some(own) => format!("{} \u{b7} {own}", shown.name),
                        None => shown.name.clone(),
                    }
                }),
            ),
            ClipItem::Audio {
                channel, stretch, ..
            } => {
                let (body, name) = self.audio_look(channel);
                let words = clips::stretch_words(
                    stretch,
                    self.file_frames(channel),
                    self.stretching.contains(&id),
                );
                (style::AUDIO_TITLE, body, Some(name + &words))
            }
            ClipItem::Automation { automation } => (
                style::AUTOMATION_TITLE,
                style::AUTOMATION_BODY,
                Some(
                    self.names
                        .get(&automation)
                        .cloned()
                        .unwrap_or_else(|| "(missing target)".to_owned()),
                ),
            ),
        };
        let body_color = if muted { style::CLIP_MUTED } else { body_color };
        frame.fill_rectangle(title_rect.position(), title_rect.size(), title_color);
        frame.fill_rectangle(body_rect.position(), body_rect.size(), body_color);
        // A dark line down the start, so clips that touch read as separate clips.
        if rect.x >= left {
            let (line, width) = device_line(rect.x, self.scale_factor);
            frame.fill_rectangle(
                Point::new(line, top),
                Size::new(width, lane),
                style::palette().button_edge,
            );
        }
        // Why the title and the gain are cut as well as clipped: the software renderer draws a
        // clipped frame's text whole, over the clips after this one.
        let at = Point::new(rect.x.max(left) + 3.0, top + TITLE_HEIGHT / 2.0);
        let mut room = title_rect.x + title_rect.width - at.x;
        // A shared pattern's "×N" at the clip's right end, when it fits after the name's start;
        // the name is cut before it.
        let end = rect.x + rect.width - 3.0;
        let badge = link_badge(shares)
            .map(|badge| (text_width(&badge, size::TINY), badge))
            .filter(|(width, _)| end - width >= at.x);
        if let Some((width, badge)) = badge {
            room = room.min(end - width - BADGE_GAP - at.x);
            frame.with_clip(title_rect, |frame| {
                frame.fill_text(Text {
                    content: badge,
                    position: Point::new(end, top + TITLE_HEIGHT / 2.0),
                    color: style::CLIP_TITLE_TEXT,
                    size: size::TINY.into(),
                    align_x: Horizontal::Right.into(),
                    align_y: Vertical::Center,
                    ..Text::default()
                });
            });
        }
        let shown = name
            .as_deref()
            .and_then(|name| fit_width(name, room, size::TINY));
        if let Some(shown) = shown {
            frame.with_clip(title_rect, |frame| {
                frame.fill_text(Text {
                    content: shown.into_owned(),
                    position: at,
                    color: style::CLIP_TITLE_TEXT,
                    size: size::TINY.into(),
                    align_y: Vertical::Center,
                    ..Text::default()
                });
            });
        }
        match clip.item {
            ClipItem::Pattern { pattern } => {
                if let Some(pattern) = self.project.patterns.get(&pattern) {
                    self.draw_marks(frame, clip, pattern, top, (left, right));
                }
            }
            ClipItem::Audio { channel, gain, .. } => {
                if gain != 1.0 {
                    frame.with_clip(title_rect, |frame| {
                        draw_gain(frame, rect, title_rect, gain);
                    });
                }
                if !muted {
                    self.draw_waveform(frame, clip, channel, body_rect, origin_y);
                }
            }
            ClipItem::Automation { automation } => {
                if let Some(automation) = self.project.automation.list.get(&automation) {
                    self.draw_automation(frame, clip, automation, body_rect);
                }
            }
        }
        if selected {
            let inset = SELECTED_OUTLINE / 2.0;
            frame.stroke_rectangle(
                Point::new(rect.x + inset, top + inset),
                Size::new(rect.width - SELECTED_OUTLINE, lane - SELECTED_OUTLINE),
                Stroke::default()
                    .with_color(style::accent())
                    .with_width(SELECTED_OUTLINE),
            );
        }
    }

    /// The length of the sound `channel` plays (its file through its edits), at the project's
    /// rate, once it is loaded.
    fn file_frames(&self, channel: ChannelId) -> Option<u64> {
        let channel = self.project.channels.get(&channel)?;
        match clips::look_of(self.files, channel)? {
            FileLook::Ready(peaks) => Some(peaks.frames),
            FileLook::Loading | FileLook::Missing => None,
        }
    }

    /// An audio channel's clip body color and title: its file's name; "(missing) name" and
    /// the missing body when the file cannot be read; "loading…" while it loads. The body is the
    /// theme's ([`crate::ui::palette::Palette::audio_clip_body`]), a shade lighter on Gilt's
    /// lanes.
    fn audio_look(&self, channel: ChannelId) -> (iced::Color, String) {
        let Some(channel) = self.project.channels.get(&channel) else {
            return (style::AUDIO_MISSING, "(missing)".to_owned());
        };
        let name = channel.sample.file_name().map_or_else(
            || channel.name.clone(),
            |name| name.to_string_lossy().into_owned(),
        );
        let body = style::palette().audio_clip_body;
        match clips::look_of(self.files, channel) {
            Some(FileLook::Missing) => (style::AUDIO_MISSING, format!("(missing) {name}")),
            Some(FileLook::Loading) | None => (body, "loading\u{2026}".to_owned()),
            Some(FileLook::Ready(_)) => (body, name),
        }
    }

    /// The waveform of the file `channel` plays, in `body`: for every pixel column, a 1 px line
    /// from the lowest to the highest value the frames under it hold (times the clip's gain,
    /// cut at the body), around a center line; a stereo file's left side in the top half, its
    /// right in the bottom.
    fn draw_waveform(
        &self,
        frame: &mut Frame,
        clip: &Clip,
        channel: ChannelId,
        body: Rectangle,
        origin_y: f32,
    ) {
        let ClipItem::Audio {
            frame_offset,
            gain,
            stretch,
            ..
        } = clip.item
        else {
            return;
        };
        let Some(channel) = self.project.channels.get(&channel) else {
            return;
        };
        let Some(FileLook::Ready(peaks)) = clips::look_of(self.files, channel) else {
            return;
        };
        let project = self.project;
        let sample = |tick: Ticks| {
            time::tick_to_sample(
                Ticks(tick.0.max(0)),
                project.sample_rate,
                project.tempo,
                project.ppq,
            )
            .unwrap_or(0)
        };
        let start = sample(clip.start);
        // The file frame song tick `tick` of the clip plays: a frame of the stretched file is
        // the file's frame at the same share of its length.
        let file_frame = |tick: Ticks| {
            let played = (frame_offset + sample(tick)).saturating_sub(start);
            match stretch.filter(|stretch| stretch.frames > 0) {
                Some(stretch) => {
                    let scaled =
                        u128::from(played) * u128::from(peaks.frames) / u128::from(stretch.frames);
                    u64::try_from(scaled).unwrap_or(u64::MAX)
                }
                None => played,
            }
        };
        let grid = self.grid();
        let sides = peaks.channels.len().clamp(1, 2);
        let band = body.height / sides as f32;
        let half = band / 2.0;
        for side in 0..sides {
            let top = body.y + band * side as f32;
            let center = top + half;
            // The canvas may start between device pixels, so the line is laid on whole device
            // pixels of the window rather than of the canvas.
            let (line, height) = device_line(origin_y + center.floor(), self.scale_factor);
            frame.fill_rectangle(
                Point::new(body.x, line - origin_y),
                Size::new(body.width, height),
                style::AUDIO_CENTER,
            );
            let mut x = body.x.floor();
            while x < body.x + body.width {
                let from = file_frame(grid.tick_at(x));
                let to = file_frame(grid.tick_at(x + 1.0)).max(from + 1);
                if let Some((low, high)) = peaks.span(side, from, to) {
                    let y_high = (center - high * gain * half).clamp(top, top + band);
                    let y_low = (center - low * gain * half).clamp(top, top + band);
                    frame.fill_rectangle(
                        Point::new(x, y_high),
                        Size::new(1.0, (y_low - y_high).max(1.0)),
                        style::AUDIO_WAVE,
                    );
                }
                x += 1.0;
            }
        }
    }

    /// The automation's value line in `body`, v = 1 near its top and v = 0 near its bottom,
    /// its points as filled circles, and a hollow handle at the middle of each segment that
    /// moves between two values.
    fn draw_automation(
        &self,
        frame: &mut Frame,
        clip: &Clip,
        automation: &Automation,
        body: Rectangle,
    ) {
        let grid = self.grid();
        let (top, bottom) = (body.y, body.y + body.height);
        let y_of = |value| playlist_layout::automation_y(top, bottom, value);
        let line = Path::new(|path| {
            let mut x = body.x;
            let mut first = true;
            while x <= body.x + body.width {
                let tick = Ticks(grid.tick_at(x).0.clamp(clip.start.0, clip.end().0));
                let at = Point::new(x, y_of(clips::value_at(automation, clip, tick)));
                if first {
                    path.move_to(at);
                    first = false;
                } else {
                    path.line_to(at);
                }
                x += 1.0;
            }
        });
        // Why no clipped frame: the renderer lays a clipped frame's shapes under the ones this
        // frame has not finished, so the clip's own body would cover them. The line stays
        // inside the body; only a point on the clip's edge reaches its radius past it.
        frame.stroke(
            &line,
            Stroke::default()
                .with_color(style::AUTOMATION_LINE)
                .with_width(AUTOMATION_LINE),
        );
        let shown = |at: Point| (body.x..=body.x + body.width).contains(&at.x);
        let handles = automation_handles(grid, clip, automation, body);
        for handle in handles.into_iter().filter(|handle| shown(handle.at)) {
            frame.stroke(
                &Path::circle(handle.at, HANDLE_RADIUS),
                Stroke::default()
                    .with_color(style::AUTOMATION_LINE)
                    .with_width(1.0),
            );
        }
        let points = automation_points(grid, clip, automation, body);
        for (_, at) in points.into_iter().filter(|&(_, at)| shown(at)) {
            frame.fill(&Path::circle(at, POINT_RADIUS + 1.0), style::palette().bg);
            frame.fill(&Path::circle(at, POINT_RADIUS), style::AUTOMATION_LINE);
        }
    }

    /// A mark for each note the clip plays inside `span` (x), one row per channel and key the
    /// pattern uses, so drum patterns read as step rows and melodies as a small roll: rows laid
    /// out by [`mark_rows`], each mark's span by [`mark_span`] (a step hit a block a step wide),
    /// its colour by [`note_mark_color`].
    fn draw_marks(
        &self,
        frame: &mut Frame,
        clip: &Clip,
        pattern: &dawsome_engine::project::Pattern,
        top: f32,
        span: (f32, f32),
    ) {
        let grid = self.grid();
        // The pattern's own length: its signature's bars, wherever the clip lies.
        let length = clip
            .pattern()
            .map_or(Ticks(1), |id| self.project.pattern_ticks(id))
            .0
            .max(1);
        let notes: Vec<_> = pattern
            .notes
            .iter()
            .filter(|note| (0..length).contains(&note.start.0))
            .collect();
        let rack: BTreeMap<ChannelId, usize> = self
            .project
            .channels
            .keys()
            .enumerate()
            .map(|(row, &id)| (id, row))
            .collect();
        let line_of = |note: &dawsome_engine::project::Note| {
            (
                rack.get(&note.channel).copied().unwrap_or(0),
                Reverse(note.key),
            )
        };
        let mut lines: Vec<(usize, Reverse<u8>)> = notes.iter().map(|note| line_of(note)).collect();
        lines.sort();
        lines.dedup();
        if lines.is_empty() {
            return;
        }
        // A channel's keys in order, a drum's when it uses one key or plays step hits alone.
        let bands: Vec<MarkBand> = lines
            .chunk_by(|a, b| a.0 == b.0)
            .map(|band| {
                let rack_row = band[0].0;
                let keys: Vec<u8> = band.iter().map(|&(_, Reverse(key))| key).collect();
                let hits_alone = notes
                    .iter()
                    .filter(|note| line_of(note).0 == rack_row)
                    .all(|note| note.length.0 == 0);
                MarkBand {
                    drum: keys.len() == 1 || hits_alone,
                    keys,
                }
            })
            .collect();
        let rows = mark_rows(top, grid.lane, &bands, self.scale_factor);
        // A step hit draws as a block a step (a sixteenth) wide.
        let step = i64::from(self.project.ppq / 4).max(1);
        // Only the passes of the pattern that reach into the visible span.
        let visible_from = grid.tick_at(span.0).0.max(clip.start.0);
        let visible_to = grid.tick_at(span.1).0.saturating_add(1).min(clip.end().0);
        let offset = clip.offset.0.rem_euclid(length);
        let base = clip.start.0 - offset;
        // Every mark inside the clip's body: right of its start line, ending where the next
        // clip's start line would begin (`draw_clip` draws both, near the lane's color).
        let rect = playlist_layout::clip_rect(grid, clip, top);
        let (line, line_width) = device_line(rect.x, self.scale_factor);
        let (next_line, _) = device_line(rect.x + rect.width, self.scale_factor);
        let body = (line + line_width, next_line);
        // From the pass before the first in view: a note of it may run into the view.
        let first_pass = (visible_from - base).div_euclid(length) - 1;
        let last_pass = (visible_to - base).div_euclid(length);
        for pass in first_pass..=last_pass {
            for note in &notes {
                let tick = base + pass * length + note.start.0;
                if !(clip.start.0..clip.end().0).contains(&tick) {
                    continue;
                }
                let x = grid.x_of(Ticks(tick));
                let (end, least) = if note.length.0 > 0 {
                    (note.length.0, NOTE_LEAST_WIDTH)
                } else {
                    (step, HIT_LEAST_WIDTH)
                };
                let end_x = grid.x_of(Ticks(tick.saturating_add(end)));
                let Some((left, width)) = mark_span(x, end_x, least, body, span, self.scale_factor)
                else {
                    continue;
                };
                let index = lines.binary_search(&line_of(note)).unwrap_or(0);
                let (y, height) = rows[index];
                frame.fill_rectangle(
                    Point::new(left, y),
                    Size::new(width, height),
                    note_mark_color(note.velocity),
                );
            }
        }
    }

    fn draw_headers(
        &self,
        frame: &mut Frame,
        tracks: &[(usize, TrackId, &dawsome_engine::project::Track)],
    ) {
        let renaming = self.arranger.rename.as_ref().map(|(id, _)| *id);
        let grid = self.grid();
        for &(row, id, track) in tracks {
            let top = grid.row_top(row);
            frame.fill_rectangle(
                Point::new(0.0, top),
                Size::new(TRACK_HEADER_WIDTH, grid.lane),
                style::palette().panel,
            );
            draw_mute_light(
                frame,
                playlist_layout::led_center(grid, row),
                LED_RADIUS,
                (!track.mute).then_some(style::TRACK_LED_ON),
            );
            // The Record button's own two colors, so the light reads as part of recording.
            let arm = if self.armed == Some(id) {
                style::RECORD_ARMED
            } else {
                style::RECORD_OFF
            };
            frame.fill(
                &Path::circle(playlist_layout::arm_center(grid, row), ARM_RADIUS),
                arm,
            );
            if renaming == Some(id) {
                continue;
            }
            let name = Rectangle::new(
                Point::new(NAME_X, top),
                Size::new(ARM_CENTER_X - ARM_RADIUS - NAME_X - 2.0, grid.lane),
            );
            // Why cut as well as clipped: the software renderer draws a clipped frame's text
            // whole, over the arm button and into the lane.
            let Some(shown) = fit_width(&track.name, name.width, size::LABEL) else {
                continue;
            };
            frame.with_clip(name, |frame| {
                frame.fill_text(Text {
                    content: shown.into_owned(),
                    position: Point::new(NAME_X, top + grid.lane / 2.0),
                    color: style::palette().text,
                    size: size::LABEL.into(),
                    align_y: Vertical::Center,
                    ..Text::default()
                });
            });
        }
    }

    /// The "+ Track" words under the last track, while their row shows ([`add_words`]): lit
    /// under the pointer, disabled at the most tracks. Drawn over the cached body, as the
    /// pointer lights them.
    fn draw_add_track(&self, frame: &mut Frame, hover: Option<&Hover>) {
        let grid = self.grid();
        let tracks = self.project.playlist.tracks.len();
        let below = tracks.saturating_sub(grid.top);
        if !grid.shows_add_track(below) {
            return;
        }
        let look = if tracks >= dawsome_engine::project::MAX_TRACKS {
            Look::Disabled
        } else if hover == Some(&Hover::Track(HeaderHit::AddTrack)) {
            Look::Lit
        } else {
            Look::Resting
        };
        let rect = playlist_layout::add_track_rect(grid, below);
        add_words::draw(frame, playlist_layout::ADD_TRACK_LABEL, rect, look);
    }
}

impl canvas::Program<Message> for Tracks<'_> {
    type State = TracksState;

    fn update(
        &self,
        state: &mut TracksState,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let send = |message| canvas::Action::publish(Message::Playlist(message));
        let local = |position: Point| Point::new(position.x - bounds.x, position.y - bounds.y);
        let held = self.arranger.tracks_held();
        let input = match event {
            Event::Mouse(mouse::Event::ButtonPressed(
                button @ (mouse::Button::Left | mouse::Button::Right),
            )) => cursor.position_in(bounds).map(|at| {
                state.last = Some(at);
                // The canvases' clock, which a test holds (ui/clock.rs).
                let now = crate::ui::clock::now();
                let double = *button == mouse::Button::Left
                    && state.previous_press.is_some_and(|(previous, time)| {
                        now.duration_since(time) <= DOUBLE_PRESS_TIME
                            && previous.distance(at) <= DOUBLE_PRESS_DISTANCE
                    });
                // A double press is not the first of another one.
                state.previous_press =
                    (*button == mouse::Button::Left && !double).then_some((at, now));
                send(PlaylistMessage::TracksPressed {
                    at,
                    button: *button,
                    double,
                })
                .and_capture()
            }),
            Event::Mouse(mouse::Event::CursorMoved { position }) if held => {
                let at = local(*position);
                state.last = Some(at);
                Some(send(PlaylistMessage::TracksDragged(at)))
            }
            // A file held over the lanes from the browser: where it would land, or that it
            // left them.
            Event::Mouse(mouse::Event::CursorMoved { .. }) if self.browser_drag => {
                let at = cursor.position_in(bounds);
                (at != self.arranger.hover).then(|| send(PlaylistMessage::BrowserHover(at)))
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if held => cursor
                .position()
                .map(local)
                .or(state.last)
                .map(|at| send(PlaylistMessage::TracksReleased(at))),
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let now = cursor.position_in(bounds).and_then(|at| self.hover_at(at));
                hover_change(&mut state.hover, now)
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                cursor.position_in(bounds).map(|at| {
                    send(PlaylistMessage::TracksScrolled { at, delta: *delta }).and_capture()
                })
            }
            _ => None,
        };
        // A new size is reported on the next event that carries no input, so no press is lost.
        input.or_else(|| {
            (bounds.size() != self.arranger.tracks_size())
                .then(|| send(PlaylistMessage::TracksResized(bounds.size())))
        })
    }

    fn draw(
        &self,
        state: &TracksState,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let body = self.cache.draw(renderer, bounds.size(), |frame| {
            self.draw_body(frame, bounds.y)
        });
        let mut overlay = Frame::new(renderer, bounds.size());
        self.draw_add_track(&mut overlay, state.hover.as_ref());
        let grid = self.grid();
        let bottom = self.lanes_bottom(bounds.height);
        if let Some(tick) = self.marker {
            let x = grid.x_of(tick);
            if (TRACK_HEADER_WIDTH..bounds.width).contains(&x) {
                let (line, width) = device_line(x, self.scale_factor);
                overlay.fill_rectangle(
                    Point::new(line, 0.0),
                    Size::new(width, bottom),
                    style::accent(),
                );
            }
        }
        let outline = Stroke::default()
            .with_color(style::accent())
            .with_width(1.0);
        match &self.preview {
            Some(Preview::Clips(targets)) => {
                for &(start, end, index) in targets {
                    let Some(top) = grid.row_top_of(index) else {
                        continue;
                    };
                    let (left, right) = (grid.x_of(start), grid.x_of(end));
                    overlay.stroke_rectangle(
                        Point::new(left + 0.5, top + 0.5),
                        Size::new(right - left - 1.0, grid.lane - 1.0),
                        outline,
                    );
                }
            }
            Some(Preview::Box(from, to)) => {
                let corner = Point::new(from.x.min(to.x), from.y.min(to.y));
                let size = Size::new((from.x - to.x).abs(), (from.y - to.y).abs());
                overlay.stroke(&Path::rectangle(corner, size), outline);
            }
            Some(Preview::Loop(_)) | None => {}
        }

        if let (true, Some(at)) = (self.browser_drag, self.arranger.hover) {
            if let Some((track, tick)) = self.arranger.drop_target(self.project) {
                let index = crate::ui::clip_gesture::index_of(self.project, track);
                if let Some(top) = index.and_then(|index| grid.row_top_of(index)) {
                    let x = grid.x_of(tick);
                    overlay.stroke_rectangle(
                        Point::new(x + 0.5, top + 0.5),
                        Size::new((at.x - x).max(8.0), grid.lane - 1.0),
                        outline,
                    );
                }
            }
        }
        vec![body, overlay.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _state: &TracksState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if self.arranger.tracks_held() {
            return mouse::Interaction::Grabbing;
        }
        let Some(at) = cursor.position_in(bounds) else {
            return mouse::Interaction::default();
        };
        if at.x < TRACK_HEADER_WIDTH {
            let rows = self
                .project
                .playlist
                .tracks
                .len()
                .saturating_sub(self.grid().top);
            return match playlist_layout::header_hit(self.grid(), at, rows) {
                Some(HeaderHit::Led { .. } | HeaderHit::Arm { .. } | HeaderHit::AddTrack) => {
                    mouse::Interaction::Pointer
                }
                Some(HeaderHit::Name { .. }) | None => mouse::Interaction::default(),
            };
        }
        match self.arranger.clip_under(self.project, at) {
            Some((_, ClipPart::RightEdge | ClipPart::LeftEdge)) => {
                mouse::Interaction::ResizingHorizontally
            }
            Some((_, ClipPart::Body)) => mouse::Interaction::Grab,
            None => mouse::Interaction::Crosshair,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The grid at `zoom` pixels per bar, its 13 rows those a 434 px track area holds whole (the
    /// playlist alone in a 1000 x 560 window).
    fn grid(zoom: f32) -> Grid {
        Grid {
            scroll: Ticks(0),
            zoom,
            bar: Ticks(384),
            top: 0,
            lane: playlist_layout::LANE_HEIGHT,
            area: 434.0,
        }
    }

    // A two-digit bar number needs 2·7 + 6 = 20 px: every bar at 32 px per bar and more, every
    // 2nd at 16. Five digits need 41 px: every bar at 64, every 2nd at 32, every 4th at 16.
    #[test]
    fn bar_numbers_thin_out_when_they_would_collide() {
        let every =
            |zoom: f32, largest| grid_rules::label_interval(grid(zoom).px_per_bar(), largest);
        assert_eq!(every(64.0, 13), 1);
        assert_eq!(every(32.0, 13), 1);
        assert_eq!(every(16.0, 13), 2);
        assert_eq!(every(64.0, 10_000), 1);
        assert_eq!(every(32.0, 10_000), 2);
        assert_eq!(every(16.0, 10_000), 4);
    }

    // A clip from x 248 to 312 at scale 1, 64 px a bar (a step 4 px): its start line is pixel
    // 248, so its body runs from 249 to the next clip's line at 312. A step hit is a block a step
    // wide less a pixel, at least 2: on its first tick (x 248) from 249 to 251, mid-clip at 280
    // 280..283, at 311.5 (its last ticks, zoomed out) 310..312, ending on the next clip's line.
    // At 1.7 a clip from 184 to 248: its line is device pixels 312-313 (183.53 to 184.71
    // logical), the next clip's starts on pixel 421 (247.65); a first-tick hit (x 184..188,
    // device 312.8..319.6) covers pixels 314..318, a last-tick one (247.9) 419..421. Each note a
    // bar from its start to one pixel before its end, cut at the clip's end and to the view.
    #[test]
    fn note_marks_stay_inside_their_clip() {
        let body = (249.0, 312.0);
        let view = (120.0, 1000.0);
        let hit = |x: f32, body, scale| mark_span(x, x + 4.0, HIT_LEAST_WIDTH, body, view, scale);
        assert_eq!(hit(248.0, body, 1.0), Some((249.0, 2.0)));
        assert_eq!(hit(280.0, body, 1.0), Some((280.0, 3.0)));
        assert_eq!(hit(311.5, body, 1.0), Some((310.0, 2.0)));
        let (line, width) = device_line(184.0, 1.7);
        let (next, _) = device_line(248.0, 1.7);
        let body = (line + width, next);
        assert!((body.0 - 314.0 / 1.7).abs() < 1e-4, "{body:?}");
        assert!((body.1 - 421.0 / 1.7).abs() < 1e-4, "{body:?}");
        let in_device = |mark: Option<(f32, f32)>| {
            mark.map(|(left, width)| ((left * 1.7).round(), (width * 1.7).round()))
        };
        assert_eq!(in_device(hit(184.0, body, 1.7)), Some((314.0, 4.0)));
        assert_eq!(in_device(hit(247.9, body, 1.7)), Some((419.0, 2.0)));

        // The marks as bars, by hand, at 64 px a bar (a tick 1/6 px; tick t at x 120 + t / 6),
        // a one-bar pattern's clip at ticks 768..1152 (x 248..312, its body 249..312 at scale
        // 1), every pixel of it in view (120..1000): a short note (tick 0, 6 long: x 248..249)
        // one pixel wide from the body's start, 249..250; a long one (tick 96, 144 long: x
        // 264..288) 264..287; one crossing the clip's end (tick 336, 96 long: x 304..320) cut at
        // the end, 304..312; a step hit on tick 0 a step to x 252, 249..251. Scrolled so the view
        // starts at 270, the long one's visible part alone, 270..287; with the view ending at
        // 300, the crossing one not at all.
        let body = (249.0, 312.0);
        let x = |tick: f32| 120.0 + tick / 6.0;
        let mark = |start: f32, length: f32, view| {
            let start = 768.0 + start;
            if length > 0.0 {
                mark_span(
                    x(start),
                    x(start + length),
                    NOTE_LEAST_WIDTH,
                    body,
                    view,
                    1.0,
                )
            } else {
                mark_span(x(start), x(start + 24.0), HIT_LEAST_WIDTH, body, view, 1.0)
            }
        };
        assert_eq!(mark(0.0, 6.0, view), Some((249.0, 1.0)), "the short one");
        assert_eq!(mark(96.0, 144.0, view), Some((264.0, 23.0)), "the long one");
        assert_eq!(
            mark(336.0, 96.0, view),
            Some((304.0, 8.0)),
            "the crossing one"
        );
        assert_eq!(mark(0.0, 0.0, view), Some((249.0, 2.0)), "a step hit");
        assert_eq!(mark(96.0, 144.0, (270.0, 1000.0)), Some((270.0, 17.0)));
        assert_eq!(mark(336.0, 96.0, (120.0, 300.0)), None);
        // The clip two passes long (ticks 768..1536, x 248..376): the crossing note's first pass
        // (tick 1104, x 304..320) whole less its last pixel, its second (tick 1488, x 368..384)
        // cut at the clip's end, 368..376.
        let body = (249.0, 376.0);
        let pass = |pass: f32| {
            let start = 768.0 + 384.0 * pass + 336.0;
            mark_span(x(start), x(start + 96.0), NOTE_LEAST_WIDTH, body, view, 1.0)
        };
        assert_eq!(
            [pass(0.0), pass(1.0)],
            [Some((304.0, 15.0)), Some((368.0, 8.0))]
        );
    }

    // Notes played legato, each ending on the tick the next starts, read as separate marks one
    // device pixel apart. At scale 1, three quarter notes from tick 96 at 64 px a bar (x 264,
    // 280, 296, 312): marks 264..279, 280..295, 296..311, pixels 279 and 295 left empty. At 1.25,
    // a note from x 263.3 to 279.9 and the next to 296.5 (device 329.125, 349.875, 370.625): the
    // first covers pixels 329..348, the second 349..369, pixel 348 between them.
    #[test]
    fn legato_notes_keep_a_device_pixel_apart() {
        let view = (120.0, 1000.0);
        let body = (249.0, 376.0);
        let run: Vec<_> = [264.0, 280.0, 296.0]
            .into_iter()
            .map(|x| mark_span(x, x + 16.0, NOTE_LEAST_WIDTH, body, view, 1.0))
            .collect();
        assert_eq!(
            run,
            [
                Some((264.0, 15.0)),
                Some((280.0, 15.0)),
                Some((296.0, 15.0))
            ]
        );
        let body = (248.8, 312.0);
        let in_device = |mark: Option<(f32, f32)>| {
            mark.map(|(left, width)| ((left * 1.25).round(), (width * 1.25).round()))
        };
        let first = mark_span(263.3, 279.9, NOTE_LEAST_WIDTH, body, view, 1.25);
        let second = mark_span(279.9, 296.5, NOTE_LEAST_WIDTH, body, view, 1.25);
        assert_eq!(in_device(first), Some((329.0, 19.0)));
        assert_eq!(in_device(second), Some((349.0, 20.0)));
    }

    // A note's mark alpha: 0.55 + 0.45 x velocity, the marks' own colour.
    #[test]
    fn softer_notes_draw_softer_marks() {
        let alpha = |velocity: f32| note_mark_color(velocity).a;
        assert_eq!(alpha(1.0), 1.0);
        assert_eq!(alpha(0.0), 0.55);
        assert!((alpha(0.5) - 0.775).abs() < 1e-6);
        assert_eq!(alpha(1.5), 1.0, "past full is full");
        let color = note_mark_color(0.3);
        let note = style::CLIP_NOTE;
        assert_eq!((color.r, color.g, color.b), (note.r, note.g, note.b));
    }

    fn pitched(keys: &[u8]) -> MarkBand {
        MarkBand {
            keys: keys.to_vec(),
            drum: false,
        }
    }

    fn drum(keys: &[u8]) -> MarkBand {
        MarkBand {
            keys: keys.to_vec(),
            drum: true,
        }
    }

    // One channel's keys 71, 67, 64 and 60 (a held chord), by hand. Its span is 12 semitones
    // (its own 11, at least 12), centred: 71 half a semitone under the span's top. By pitch at
    // its fullest the band is 12 x 3 + 4 + 1 = 41 px. The marks' room is the body less 4 px
    // under the title and above the foot: from 14, the lane less 18. 64 px lane: 46 px, the band
    // whole, centred from round(14 + 2.5) = 17; centres 17 + 2.5 (the edge) + 1.5 = 21, then 4,
    // 7 and 11 semitones of 3 px down: 33, 42, 54; marks 4 px (3.6 rounded) from 19, 31, 40,
    // 52. 32 px lane: 14 px, 14 / 41 of the fullest, 1.02 px a semitone, under 1.5: the keys
    // stack evenly, four rows of 6 px shrunk to 14 / 24, 3.5 px apart, centres 15.75, 19.25,
    // 22.75, 26.25, marks 2 px from 15, 18, 22, 25. At scale 2 that lane's 28 device pixels
    // give a semitone 6 x 28 / 82 = 2.05 device pixels, so the chord stands by pitch: marks 4
    // device pixels (2.46 rounded, at least 2 px), centres 28 + (8, 32, 50, 74) x 28 / 82 =
    // 30.73, 38.93, 45.07, 53.27, from device 29, 37, 43, 51 (14.5, 18.5, 21.5, 25.5 px).
    #[test]
    fn a_chord_stands_by_semitone_where_a_semitone_has_room_and_stacks_where_not() {
        let chord = [pitched(&[71, 67, 64, 60])];
        let rows = |lane, scale| mark_rows(0.0, lane, &chord, scale);
        assert_eq!(
            rows(64.0, 1.0),
            [(19.0, 4.0), (31.0, 4.0), (40.0, 4.0), (52.0, 4.0)]
        );
        assert_eq!(
            rows(32.0, 1.0),
            [(15.0, 2.0), (18.0, 2.0), (22.0, 2.0), (25.0, 2.0)]
        );
        assert_eq!(
            rows(32.0, 2.0),
            [(14.5, 2.0), (18.5, 2.0), (21.5, 2.0), (25.5, 2.0)]
        );
    }

    // Drum rows, by hand. One key in a 50 px lane (the playlist probe's): its 6 px row centred in
    // the marks' 32 px room from 14, from 27, the mark 5 px (the row less a pixel) from
    // round(27.5) = 28, over the row the probe reads at y 30. Three one-key channels (a
    // drums-only Jam on a kit) in a 32 px lane: 18 px at their fullest in a 14 px room, rows
    // 14 / 3 = 4.67 px apart, centres 16.33, 21, 25.67, marks 3 px from 15, 20 (19.5 rounded
    // up) and 24, the last ending 5 px over the lane's foot. One channel's four keys of step
    // hits (a drums-only Jam on one set) there: rows 3.5 px apart, marks 2 px from 15, 18, 22,
    // 25.
    #[test]
    fn drum_rows_are_their_pitch_less_a_pixel() {
        assert_eq!(mark_rows(0.0, 50.0, &[drum(&[36])], 1.0), [(28.0, 5.0)]);
        let kit = [drum(&[60]), drum(&[60]), drum(&[60])];
        assert_eq!(
            mark_rows(0.0, 32.0, &kit, 1.0),
            [(15.0, 3.0), (20.0, 3.0), (24.0, 3.0)]
        );
        assert_eq!(
            mark_rows(0.0, 32.0, &[drum(&[46, 42, 38, 36])], 1.0),
            [(15.0, 2.0), (18.0, 2.0), (22.0, 2.0), (25.0, 2.0)]
        );
    }

    // The Jam's clip of p36_band: drums on 4 keys of step hits (24 px at their fullest), bass
    // on 3 keys over 12 semitones (41), chords on 10 over their own 17 (56), melody 6 over 12
    // (41): 162 px. In a 64 px lane's 46 px room a semitone would have 3 x 46 / 162 = 0.85 px,
    // under 1.5, so the three pitched channels stack evenly too: 23 rows of 6 px, 138, shrunk
    // to 46, a row every 2 px from 14, 2 px marks from 14, 16, ... 58, touching, as they must
    // under 3 px. In a 32 px lane the same rows, closer still.
    #[test]
    fn a_dense_clip_stacks_every_channel_evenly() {
        let jam = [
            drum(&[46, 42, 38, 36]),
            pitched(&[47, 45, 40]),
            pitched(&[69, 67, 65, 64, 62, 60, 59, 57, 55, 52]),
            pitched(&[72, 69, 67, 64, 62, 60]),
        ];
        let rows = mark_rows(0.0, 64.0, &jam, 1.0);
        let even: Vec<(f32, f32)> = (0..23).map(|row| (14.0 + 2.0 * row as f32, 2.0)).collect();
        assert_eq!(rows, even);
        let rows = mark_rows(0.0, 32.0, &jam, 1.0);
        assert_eq!(rows.len(), 23);
        assert!(rows.iter().all(|&(_, height)| height == 2.0), "{rows:?}");
        let inside = |&(y, height): &(f32, f32)| y >= TITLE_HEIGHT && y + height <= 32.0;
        assert!(rows.iter().all(inside), "{rows:?}");
    }

    // A drum and a chord (6 + 41 = 47 px at their fullest; in the 64 px lane's 46 px room
    // shrunk a little, the chord still by pitch). In every lane height from 64 up, at the zooms
    // the app offers: every mark under the title and inside the lane, on device pixels, 2 to 6
    // px tall, ending at least a device pixel above the next row's. In the 32 and 48 px lanes, shrunk, every mark is still inside, at least 2 px tall.
    // The chord alone (its band the same at either end) stands centred from 64 up: the space
    // above its first mark and under its last within 2 device pixels of each other.
    #[test]
    fn a_few_rows_stand_whole_and_centred_where_the_lane_has_room() {
        let bands = [drum(&[36]), pitched(&[71, 67, 64, 60])];
        for lane in playlist_layout::LANE_HEIGHTS {
            for scale in [1.0, 1.25, 1.5, 1.75, 2.0] {
                let top = 117.0;
                let rows = mark_rows(top, lane, &bands, scale);
                let device = |v: f32| (v * scale).round();
                let at = format!("lane {lane}, scale {scale}: {rows:?}");
                for (index, &(y, height)) in rows.iter().enumerate() {
                    assert!(y >= top + TITLE_HEIGHT && y + height <= top + lane, "{at}");
                    assert!((y * scale - device(y)).abs() < 1e-3, "off a pixel: {at}");
                    assert!(device(height) >= device(2.0), "{at}");
                    assert!(device(height) <= device(6.0), "{at}");
                    if lane >= 64.0 {
                        if let Some(&(next, _)) = rows.get(index + 1) {
                            assert!(device(y + height) + 1.0 <= device(next), "touching: {at}");
                        }
                    }
                }
                if lane >= 64.0 {
                    let rows = mark_rows(top, lane, &bands[1..], scale);
                    let above = device(rows[0].0) - device(top + TITLE_HEIGHT + MARK_PADDING);
                    let (last, height) = rows[rows.len() - 1];
                    let below = device(top + lane - MARK_PADDING) - device(last + height);
                    assert!(
                        (above - below).abs() <= 2.0,
                        "{above} over, {below} under: {at}"
                    );
                }
            }
        }
    }

    // At scale 1 a line is the logical pixel itself; at 1.7, x 184 is device pixel 312.8, so
    // the line starts on pixel 312 (183.53 logical) and is 2 device pixels wide.
    #[test]
    fn lines_land_on_whole_device_pixels() {
        assert_eq!(device_line(184.0, 1.0), (184.0, 1.0));
        let (left, width) = device_line(184.0, 1.7);
        assert_eq!(((left * 1.7).round(), (width * 1.7).round()), (312.0, 2.0));
        assert!(left <= 184.0 && (left + width) * 1.7 >= 184.0 * 1.7 + 1.0);
        assert_eq!(device_line(10.0, 2.0), (10.0, 1.0));
        assert_eq!(device_line(10.0, 0.0), (10.0, 1.0), "no scale reported yet");
        // Known answer at the system's 1.0 times a zoom of 125 %: x 10.3 is device pixel
        // 12.875, so the line starts on pixel 12, 12 / 1.25 = 9.6, one device pixel wide,
        // 1 / 1.25 = 0.8.
        let (left, width) = device_line(10.3, 1.25);
        assert!(
            (left - 9.6).abs() < 1e-5 && (width - 0.8).abs() < 1e-6,
            "{left} {width}"
        );
    }

    #[test]
    fn spans_are_clipped_to_the_timeline() {
        let grid = grid(64.0);
        assert_eq!(
            visible_span(&grid, Ticks(384), Ticks(768), 1000.0),
            Some((184.0, 248.0))
        );
        // Scrolled 576 ticks (96 px): tick 384 is left of the timeline, 768 is at 152.
        let scrolled = Grid {
            scroll: Ticks(576),
            ..grid
        };
        assert_eq!(
            visible_span(&scrolled, Ticks(384), Ticks(768), 1000.0),
            Some((120.0, 152.0))
        );
        assert_eq!(visible_span(&scrolled, Ticks(0), Ticks(384), 1000.0), None);
        assert_eq!(
            visible_span(&grid, Ticks(0), Ticks(100_000), 300.0),
            Some((120.0, 300.0))
        );
    }
}
