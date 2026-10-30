//! The playlist panel: a header strip (snap, zoom, Follow, Fit and the link control) over the
//! ruler (bar numbers over the loop strip), the tracks (headers beside lanes of clips) and the
//! timeline's scroll bar at the panel's foot. Both canvases draw from
//! [`playlist_layout`] positions and the [`Arranger`] state, and only report the cursor in their
//! own coordinates; the app turns that into edits. Each keeps what it draws in a cached layer,
//! cleared by the app whenever that changes, and draws the song position and a held drag's
//! outlines over it every frame.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use dawsome_engine::edit::copies;
use dawsome_engine::project::{
    Automation, AutomationId, ChannelId, Clip, ClipId, ClipItem, Curve, LoopRange, Project,
    StretchMode, TrackId,
};
use dawsome_engine::time::{self, Ticks};
use iced::alignment::{Horizontal, Vertical};
use iced::widget::canvas::{self, Cache, Event, Frame, Geometry, Path, Stroke, Text};
use iced::widget::{button, column, container, pin, row, space, stack, text, text_input, Id};
use iced::{mouse, window, Center, Element, Fill, Point, Rectangle, Renderer, Size, Theme, Vector};

use crate::app::{ClipLink, MediaMessage, Message, SelectionLink};
use crate::ui::arranger::{Arranger, PlaylistMessage, Preview, CURVE_MENU};
use crate::ui::clip_gesture::ClipSnap;
use crate::ui::clips::{self, FileLook, FileLooks};
use crate::ui::hint::{
    hover_area, hover_change, link_tip, with_tip, ClipControl, ClipRow, Hover, MenuRow,
    PlaylistTool, Pointer, TrackRow,
};
use crate::ui::menu;
use crate::ui::panels::Panel;
use crate::ui::playlist_layout::{
    self, ClipPart, Grid, HeaderHit, Lanes, ARM_CENTER_X, ARM_RADIUS, AUTOMATION_LINE,
    HANDLE_RADIUS, HEADER_HEIGHT, LABEL_INSET, LABEL_SIZE, LED_RADIUS, MARKER_WIDTH,
    MIN_BEAT_SPACING, NAME_SIZE, NAME_X, NOTE_MARK_WIDTH, NUMBER_BAND_HEIGHT, POINT_RADIUS,
    RULER_HEIGHT, SELECTED_OUTLINE, TIME_BAR_HEIGHT, TITLE_HEIGHT, TITLE_SIZE, TRACK_HEADER_WIDTH,
    ZOOM_LEVELS,
};
use crate::ui::scroll::Scrollbar;
use crate::ui::style;
use crate::ui::text_fit::{fit_width, text_width};

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

/// Pixels a bar number needs per digit, plus a margin, so labels never run into each other.
const LABEL_DIGIT_WIDTH: f32 = 7.0;
const LABEL_MARGIN: f32 = 6.0;

/// The tallest note mark in a clip, and the padding around the marks.
const NOTE_MARK_MAX_HEIGHT: f32 = 4.0;
const MARK_PADDING: f32 = 2.0;

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

    /// The timeline's scroll bar at the panel's foot, under the lanes as the piano roll's is
    /// under its grid; under the track headers its row is the well.
    fn time_bar(&self) -> Element<'a, Message> {
        let bar = canvas::Canvas::new(Scrollbar {
            span: self.arranger.span(self.project),
            message: |message| Message::Playlist(PlaylistMessage::Bar(message)),
        })
        .width(Fill)
        .height(TIME_BAR_HEIGHT);
        let under_headers = container(space())
            .width(TRACK_HEADER_WIDTH)
            .height(TIME_BAR_HEIGHT)
            .style(style::rack_well);
        row![under_headers, bar].into()
    }

    /// "Playlist", the snap choices, the zoom buttons, the Follow and Fit switches, the link
    /// control's cell right of Fit, and an audio clip's properties.
    fn header(&self) -> Element<'a, Message> {
        let snap = self.arranger.snap;
        let snaps = ClipSnap::ALL.map(|choice| {
            let face = button(text(choice.to_string()).size(11))
                .padding([2, 8])
                .style(style::tool_button(choice == snap))
                .on_press(Message::Playlist(PlaylistMessage::SnapSelected(choice)));
            hover_area(face, Hover::PlaylistTool(PlaylistTool::Snap))
        });
        let zoom = self.arranger.grid.zoom;
        let zoom_button = |label, message, enabled: bool| {
            button(text(label).size(11))
                .padding([2, 8])
                .style(style::tool_button(false))
                .on_press_maybe(enabled.then_some(Message::Playlist(message)))
        };
        let mut content = row![
            text("Playlist").size(12),
            row(snaps).spacing(2),
            row![
                hover_area(
                    with_tip(
                        zoom_button("−", PlaylistMessage::ZoomOut, zoom > 0),
                        "Zoom out \u{b7} Ctrl+wheel"
                    ),
                    Hover::PlaylistTool(PlaylistTool::ZoomOut)
                ),
                hover_area(
                    with_tip(
                        zoom_button("+", PlaylistMessage::ZoomIn, zoom + 1 < ZOOM_LEVELS.len()),
                        "Zoom in \u{b7} Ctrl+wheel"
                    ),
                    Hover::PlaylistTool(PlaylistTool::ZoomIn)
                ),
            ]
            .spacing(2),
        ]
        .spacing(12)
        .align_y(Center);
        let follow = button(text("Follow").size(11))
            .padding([2, 8])
            .style(style::tool_button(self.follow))
            .on_press(Message::ToggleFollow(Panel::Playlist));
        content = content.push(hover_area(
            follow,
            Hover::PlaylistTool(PlaylistTool::Follow),
        ));
        let fitted = self.arranger.lanes() == Lanes::Fit;
        let fit = button(text("Fit").size(11))
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
                let label = fit_width(&label, CLIP_ROW_ROOM, menu::TEXT_SIZE)
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
            .size(NAME_SIZE)
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
    rows
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
                frame.stroke(&link, line(style::TEXT, 1.6));
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

/// An audio clip's properties in the header: Stretch "None / Resample / Stretch", the pitch
/// stepper (for a time-stretch), the length in percent of the file's, and a close button.
fn properties_strip<'a>(properties: ClipProperties) -> Element<'a, Message> {
    let send = |message| Message::Playlist(message);
    let small = |control: ClipControl, label: String, lit: bool, message: Message| {
        let face = button(text(label).size(11))
            .padding([2, 8])
            .style(style::tool_button(lit))
            .on_press(message);
        hover_area(face, Hover::ClipControl(control))
    };
    let stretched = properties.mode == Some(StretchMode::Stretch);
    let pitch = match properties.pitch {
        0 => "0 st".to_owned(),
        up if up > 0 => format!("+{up} st"),
        down => format!("\u{2212}{} st", down.unsigned_abs()),
    };
    let mut modes = row![].spacing(2);
    let mut pitches = row![text("Pitch").size(11)].spacing(4).align_y(Center);
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
                pitches = pitches.push(step).push(text(pitch.clone()).size(11));
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
    let mut strip = row![text("Stretch").size(11), modes]
        .spacing(6)
        .align_y(Center);
    if stretched {
        strip = strip.push(pitches);
    }
    if let Some(percent) = properties.percent {
        strip = strip.push(text(format!("Length {percent}%")).size(11));
    }
    strip.push(close).into()
}

/// The bar-number band over the loop strip.
struct Ruler<'a> {
    grid: Grid,
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
        frame.fill_rectangle(Point::ORIGIN, size, style::RACK_WELL);
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
        let bars: Vec<(i64, f32)> = playlist_layout::visible_bars(&self.grid, size.width).collect();
        let every = label_interval(&self.grid, bars.last().map_or(1, |&(number, _)| number));
        for (number, x) in bars {
            let (line, width) = device_line(x, self.scale_factor);
            frame.fill_rectangle(
                Point::new(line, NUMBER_BAND_HEIGHT - 4.0),
                Size::new(width, 4.0),
                style::LANE_BAR_LINE,
            );
            if (number - 1) % every == 0 {
                frame.fill_text(Text {
                    content: number.to_string(),
                    position: Point::new(x + LABEL_INSET, NUMBER_BAND_HEIGHT / 2.0),
                    color: style::TEXT,
                    size: LABEL_SIZE.into(),
                    align_y: Vertical::Center,
                    ..Text::default()
                });
            }
        }
    }
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
    let Some(fitted) = fit_width(&words, end - shown.x, TITLE_SIZE) else {
        return;
    };
    frame.fill_text(Text {
        content: fitted.into_owned(),
        position: Point::new(end, rect.y + TITLE_HEIGHT / 2.0),
        color: style::CLIP_TITLE_TEXT,
        size: TITLE_SIZE.into(),
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

/// Every how many bars a number is written, so that numbers up to `largest` fit between their
/// lines: every bar when there is room, else every 2nd, 4th, ...
fn label_interval(grid: &Grid, largest: i64) -> i64 {
    let digits = largest.max(1).to_string().len() as f32;
    let needed = digits * LABEL_DIGIT_WIDTH + LABEL_MARGIN;
    let mut every = 1;
    while (every as f32) * grid.px_per_bar() < needed {
        every *= 2;
    }
    every
}

/// A 1 px line at logical x, as drawn: its left edge moved down onto a device pixel, and one
/// device pixel per whole scale step wide (2 at scale 1.7), so it covers whole pixels.
///
/// Why: lines a logical pixel wide at a fractional scale straddle device pixels, and the
/// renderer's multisampling then blends them into the lane color, lighter or darker from line
/// to line. Lines on whole device pixels look the same everywhere, in the window and in a
/// screenshot.
fn device_line(x: f32, scale_factor: f32) -> (f32, f32) {
    let scale = if scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    let left = (x * scale).floor() / scale;
    let width = scale.round().max(1.0) / scale;
    (left, width)
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
                overlay.fill(&triangle, style::ACCENT);
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
        let index = usize::try_from(self.grid().track_index_at(at.y)).ok()?;
        (index < tracks).then_some(Hover::Lane)
    }

    fn grid(&self) -> &Grid {
        &self.arranger.grid
    }

    /// The tracks from the top row down, with their rows.
    fn shown_tracks(
        &self,
        height: f32,
    ) -> impl Iterator<Item = (usize, TrackId, &dawsome_engine::project::Track)> {
        let rows = self.grid().visible_rows(height);
        self.project
            .playlist
            .tracks
            .iter()
            .skip(self.grid().top)
            .take(rows)
            .enumerate()
            .map(|(row, (&id, track))| (row, id, track))
    }

    /// Where the lanes end: under the last visible track, at most the canvas's height.
    fn lanes_bottom(&self, height: f32) -> f32 {
        let rows = self
            .project
            .playlist
            .tracks
            .len()
            .saturating_sub(self.grid().top);
        (self.grid().row_top(rows) - playlist_layout::LANE_GAP).clamp(0.0, height)
    }

    fn draw_body(&self, frame: &mut Frame, origin_y: f32) {
        let size = frame.size();
        let grid = self.grid();
        frame.fill_rectangle(Point::ORIGIN, size, style::RACK_WELL);
        let tracks: Vec<_> = self.shown_tracks(size.height).collect();
        let lane_width = (size.width - TRACK_HEADER_WIDTH).max(0.0);
        let tint = self
            .loop_shown
            .filter(|range| range.on)
            .and_then(|range| visible_span(grid, range.start, range.end, size.width));
        for &(row, _, _) in &tracks {
            let top = grid.row_top(row);
            frame.fill_rectangle(
                Point::new(TRACK_HEADER_WIDTH, top),
                Size::new(lane_width, grid.lane),
                style::LANE,
            );
            if let Some((left, right)) = tint {
                frame.fill_rectangle(
                    Point::new(left, top),
                    Size::new(right - left, grid.lane),
                    style::LOOP_TINT,
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

    /// A line at every bar, and at every beat when beats are far enough apart: 1 px wide,
    /// through every lane.
    fn draw_lines(&self, frame: &mut Frame, size: Size) {
        let grid = self.grid();
        let bottom = self.lanes_bottom(size.height);
        let beat = grid.beat().0;
        let bar = grid.bar.0.max(1);
        let beats_shown = grid.px_per_bar() / 4.0 >= MIN_BEAT_SPACING;
        let unit = if beats_shown { beat } else { bar };
        let first = (grid.scroll.0 + unit - 1).div_euclid(unit) * unit;
        let last = grid.tick_at(size.width).0;
        let mut tick = first;
        while tick <= last {
            let color = if tick % bar == 0 {
                style::LANE_BAR_LINE
            } else {
                style::LANE_BEAT_LINE
            };
            let (line, width) = device_line(grid.x_of(Ticks(tick)), self.scale_factor);
            frame.fill_rectangle(Point::new(line, 0.0), Size::new(width, bottom), color);
            tick += unit;
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
            ClipItem::Pattern { pattern } => (
                style::CLIP_TITLE,
                style::CLIP_BODY,
                self.project
                    .patterns
                    .get(&pattern)
                    .map(|pattern| pattern.name.clone()),
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
                style::BUTTON_EDGE,
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
            .map(|badge| (text_width(&badge, TITLE_SIZE), badge))
            .filter(|(width, _)| end - width >= at.x);
        if let Some((width, badge)) = badge {
            room = room.min(end - width - BADGE_GAP - at.x);
            frame.with_clip(title_rect, |frame| {
                frame.fill_text(Text {
                    content: badge,
                    position: Point::new(end, top + TITLE_HEIGHT / 2.0),
                    color: style::CLIP_TITLE_TEXT,
                    size: TITLE_SIZE.into(),
                    align_x: Horizontal::Right.into(),
                    align_y: Vertical::Center,
                    ..Text::default()
                });
            });
        }
        let shown = name
            .as_deref()
            .and_then(|name| fit_width(name, room, TITLE_SIZE));
        if let Some(shown) = shown {
            frame.with_clip(title_rect, |frame| {
                frame.fill_text(Text {
                    content: shown.into_owned(),
                    position: at,
                    color: style::CLIP_TITLE_TEXT,
                    size: TITLE_SIZE.into(),
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
                    .with_color(style::ACCENT)
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
    /// the missing body when the file cannot be read; "loading…" while it loads.
    fn audio_look(&self, channel: ChannelId) -> (iced::Color, String) {
        let Some(channel) = self.project.channels.get(&channel) else {
            return (style::AUDIO_MISSING, "(missing)".to_owned());
        };
        let name = channel.sample.file_name().map_or_else(
            || channel.name.clone(),
            |name| name.to_string_lossy().into_owned(),
        );
        match clips::look_of(self.files, channel) {
            Some(FileLook::Missing) => (style::AUDIO_MISSING, format!("(missing) {name}")),
            Some(FileLook::Loading) | None => (style::AUDIO_BODY, "loading\u{2026}".to_owned()),
            Some(FileLook::Ready(_)) => (style::AUDIO_BODY, name),
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
            frame.fill(&Path::circle(at, POINT_RADIUS + 1.0), style::RACK_WELL);
            frame.fill(&Path::circle(at, POINT_RADIUS), style::AUTOMATION_LINE);
        }
    }

    /// A 2 px mark for each note the clip plays inside `span` (x), one row per channel and key
    /// the pattern uses, so drum patterns read as step rows and melodies as a small roll.
    fn draw_marks(
        &self,
        frame: &mut Frame,
        clip: &Clip,
        pattern: &dawsome_engine::project::Pattern,
        top: f32,
        span: (f32, f32),
    ) {
        let grid = self.grid();
        let length = time::bars_to_ticks(pattern.length_bars, self.project.ppq)
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
        let mut lines: Vec<(usize, std::cmp::Reverse<u8>)> = notes
            .iter()
            .map(|note| {
                (
                    rack.get(&note.channel).copied().unwrap_or(0),
                    std::cmp::Reverse(note.key),
                )
            })
            .collect();
        lines.sort();
        lines.dedup();
        if lines.is_empty() {
            return;
        }
        let area = grid.lane - TITLE_HEIGHT - 2.0 * MARK_PADDING;
        let pitch = area / lines.len() as f32;
        let height = pitch.clamp(1.0, NOTE_MARK_MAX_HEIGHT);
        // Only the passes of the pattern that reach into the visible span.
        let visible_from = grid.tick_at(span.0).0.max(clip.start.0);
        let visible_to = grid.tick_at(span.1).0.saturating_add(1).min(clip.end().0);
        let offset = clip.offset.0.rem_euclid(length);
        let base = clip.start.0 - offset;
        let first_pass = (visible_from - base).div_euclid(length);
        let last_pass = (visible_to - base).div_euclid(length);
        for pass in first_pass..=last_pass {
            for note in &notes {
                let tick = base + pass * length + note.start.0;
                if !(clip.start.0..clip.end().0).contains(&tick) {
                    continue;
                }
                let x = grid.x_of(Ticks(tick));
                if !(span.0..span.1).contains(&x) {
                    continue;
                }
                let line = (
                    rack.get(&note.channel).copied().unwrap_or(0),
                    std::cmp::Reverse(note.key),
                );
                let index = lines.binary_search(&line).unwrap_or(0);
                let y = top
                    + TITLE_HEIGHT
                    + MARK_PADDING
                    + index as f32 * pitch
                    + (pitch - height) / 2.0;
                frame.fill_rectangle(
                    Point::new(x, y),
                    Size::new(NOTE_MARK_WIDTH, height),
                    style::CLIP_NOTE,
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
                style::PANEL,
            );
            let led = if track.mute {
                style::TRACK_LED_OFF
            } else {
                style::TRACK_LED_ON
            };
            frame.fill(
                &Path::circle(playlist_layout::led_center(grid, row), LED_RADIUS),
                led,
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
            let Some(shown) = fit_width(&track.name, name.width, NAME_SIZE) else {
                continue;
            };
            frame.with_clip(name, |frame| {
                frame.fill_text(Text {
                    content: shown.into_owned(),
                    position: Point::new(NAME_X, top + grid.lane / 2.0),
                    color: style::TEXT,
                    size: NAME_SIZE.into(),
                    align_y: Vertical::Center,
                    ..Text::default()
                });
            });
        }
        let below = self
            .project
            .playlist
            .tracks
            .len()
            .saturating_sub(self.grid().top);
        let button = playlist_layout::add_track_rect(grid, below);
        if button.y < frame.height() {
            let room = self.project.playlist.tracks.len() < dawsome_engine::project::MAX_TRACKS;
            frame.fill_rectangle(button.position(), button.size(), style::BUTTON);
            frame.fill_text(Text {
                content: "+ Track".into(),
                position: button.center(),
                color: if room { style::TEXT } else { style::TEXT_DIM },
                size: NAME_SIZE.into(),
                align_x: Horizontal::Center.into(),
                align_y: Vertical::Center,
                ..Text::default()
            });
        }
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
        _state: &TracksState,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let body = self.cache.draw(renderer, bounds.size(), |frame| {
            self.draw_body(frame, bounds.y)
        });
        let mut overlay = Frame::new(renderer, bounds.size());
        let grid = self.grid();
        let bottom = self.lanes_bottom(bounds.height);
        if let Some(tick) = self.marker {
            let x = grid.x_of(tick);
            if (TRACK_HEADER_WIDTH..bounds.width).contains(&x) {
                let (line, width) = device_line(x, self.scale_factor);
                overlay.fill_rectangle(
                    Point::new(line, 0.0),
                    Size::new(width, bottom),
                    style::ACCENT,
                );
            }
        }
        let outline = Stroke::default().with_color(style::ACCENT).with_width(1.0);
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

    fn grid(zoom: usize) -> Grid {
        Grid {
            scroll: Ticks(0),
            zoom,
            bar: Ticks(384),
            top: 0,
            lane: playlist_layout::LANE_HEIGHT,
        }
    }

    // A two-digit bar number needs 2·7 + 6 = 20 px: every bar at 32 px per bar and more, every
    // 2nd at 16. Five digits need 41 px: every bar at 64, every 2nd at 32, every 4th at 16.
    #[test]
    fn bar_numbers_thin_out_when_they_would_collide() {
        assert_eq!(label_interval(&grid(2), 13), 1);
        assert_eq!(label_interval(&grid(1), 13), 1);
        assert_eq!(label_interval(&grid(0), 13), 2);
        assert_eq!(label_interval(&grid(2), 10_000), 1);
        assert_eq!(label_interval(&grid(1), 10_000), 2);
        assert_eq!(label_interval(&grid(0), 10_000), 4);
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
    }

    #[test]
    fn spans_are_clipped_to_the_timeline() {
        let grid = grid(2);
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
