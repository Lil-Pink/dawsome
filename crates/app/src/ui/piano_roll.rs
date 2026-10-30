//! The piano roll panel: a strip of controls over the note grid (with its key column), the
//! lane's drag handle and the velocity lane. The canvases draw from [`piano_layout`] positions
//! and the [`Roll`] state, and only report the cursor in their own coordinates; the app turns
//! that into edits. The grid and the lane keep their rows, lines, keys and notes in a cached
//! layer, cleared by the app whenever what they show changes, and the grid draws the playhead
//! and the select box over it every frame.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use dawsome_engine::project::{ChannelId, Note, PatternId, Project};
use dawsome_engine::time::Ticks;
use iced::alignment::{Horizontal, Vertical};
use iced::keyboard::Modifiers;
use iced::widget::canvas::{self, Cache, Event, Frame, Geometry, Path, Stroke, Text};
use iced::widget::{button, column, container, pick_list, pin, row, space, stack, text};
use iced::{mouse, Center, Color, Element, Fill, Point, Rectangle, Renderer, Size, Theme};

use crate::app::Message;
use crate::ui::grip::Grip;
use crate::ui::hint::{hover_area, hover_change, Hover, MenuRow, NoteRow, Pointer, RollPart};
use crate::ui::key_names::key_name;
use crate::ui::live_notes::{self, LiveNotes};
use crate::ui::menu;
use crate::ui::note_gesture::{key_of, velocity_steps, NoteKey, Snap, Target};
use crate::ui::panels::Panel;
use crate::ui::piano_layout::{
    self, View, BAR_LINE_WIDTH, HANDLE_HEIGHT, KEY_COLUMN_WIDTH, LABEL_RIGHT, LABEL_SIZE,
    LINE_WIDTH, NOTE_BORDER, PLAYHEAD_WIDTH, ROW_HEIGHT, SELECTED_BORDER, STRIP_HEIGHT,
};
use crate::ui::roll::{NoteEdit, QuantizeMessage, Roll, RollMessage, MENU_FROM_RIGHT};
use crate::ui::scale::{Root, Scale, ScaleMode};
use crate::ui::scroll::{self, Scrollbar};
use crate::ui::style;
use crate::ui::toolbar::format_length;

/// Keys whose rows are black keys on a piano, by their remainder from 12.
const BLACK_KEYS: [u8; 5] = [1, 3, 6, 8, 10];

/// Two left presses at most this far apart in time and space are a double-click.
const DOUBLE_PRESS_TIME: Duration = Duration::from_millis(400);
const DOUBLE_PRESS_DISTANCE: f32 = 6.0;

/// The grid canvas's memory between events: where the cursor was last seen in a drag, what it
/// is over, and the last left press, to spot a double-click.
#[derive(Debug, Default)]
pub struct GridState {
    pointer: Pointer,
    previous_press: Option<(Point, Instant)>,
}

/// What the piano roll shows: the selected channel's notes in the selected pattern.
pub struct PianoRoll<'a> {
    pub project: &'a Project,
    pub pattern: Option<PatternId>,
    pub channel: Option<ChannelId>,
    pub roll: &'a Roll,
    /// The keys held now, for the snap a drag previews with (Alt: single ticks).
    pub modifiers: Modifiers,
    /// The tick being played, `None` while stopped.
    pub playhead: Option<Ticks>,
    /// The song tick where the pattern's tick 0 lies for the playhead and the live notes: a
    /// song take's start while it records into the pattern shown, else 0 (the song's tick
    /// within the pattern as it loops).
    pub origin: Ticks,
    pub grid_cache: &'a Cache,
    pub lane_cache: &'a Cache,
    /// Whether drawn, moved and clicked notes play: the strip's Preview switch.
    pub preview: bool,
    /// Whether the view jumps a page as the playhead leaves it: the strip's Follow switch.
    pub follow: bool,
    /// Whether the clipboard holds notes, so the menu's Paste acts.
    pub clipboard: bool,
    /// The notes played live now, on every channel.
    pub live: &'a LiveNotes,
    /// Whether a take of notes runs: the strip's "Rec notes" light.
    pub recording_notes: bool,
    /// The window's last pixels a menu keeps clear of (the hint bar while it shows).
    pub menu_foot: f32,
}

/// The notes menu's width: its longest row, "Chop at snap", and a margin.
pub const NOTE_MENU_WIDTH: f32 = MENU_FROM_RIGHT - 2.0 * menu::PADDING - 12.0;

/// The message a notes menu row sends; none when it would do nothing (Cut, Copy and Delete
/// with nothing selected, Paste with an empty clipboard).
pub fn note_row_message(row: NoteRow, selected: bool, clipboard: bool) -> Option<Message> {
    let edit = |edit| Some(Message::NoteEdit(edit));
    match row {
        NoteRow::Cut if selected => edit(NoteEdit::Cut),
        NoteRow::Copy if selected => edit(NoteEdit::Copy),
        NoteRow::Paste if clipboard => edit(NoteEdit::Paste),
        NoteRow::Delete if selected => edit(NoteEdit::Delete),
        NoteRow::Cut | NoteRow::Copy | NoteRow::Paste | NoteRow::Delete => None,
        NoteRow::Duplicate => edit(NoteEdit::Duplicate),
        NoteRow::Quantize => Some(Message::Quantize(QuantizeMessage::Open)),
        NoteRow::Chop => edit(NoteEdit::Chop),
        NoteRow::Glue => edit(NoteEdit::Glue),
        NoteRow::Legato => edit(NoteEdit::Legato),
        NoteRow::Reverse => edit(NoteEdit::Reverse),
        NoteRow::Invert => edit(NoteEdit::Invert),
        NoteRow::Humanize => edit(NoteEdit::Humanize),
    }
}

/// `channel`'s live notes as the grid draws them: each key with the tick it started at, and the
/// playhead their bars grow to, both counted from `origin` (where the pattern's tick 0 lies).
pub(crate) fn live_from(
    live: &LiveNotes,
    channel: ChannelId,
    playhead: Option<Ticks>,
    origin: Ticks,
) -> (Vec<(u8, Option<Ticks>)>, Option<Ticks>) {
    let from_origin = |tick: Ticks| Ticks(tick.0 - origin.0);
    let keys = live
        .of(channel)
        .into_iter()
        .map(|(key, held)| (key, held.tick.map(from_origin)))
        .collect();
    (keys, playhead.map(from_origin))
}

/// A note as the canvases draw it.
#[derive(Debug, Clone)]
struct Shown {
    note: Note,
    selected: bool,
}

impl<'a> PianoRoll<'a> {
    pub fn view(self) -> Element<'a, Message> {
        let Some(target) = Target::of(self.project, self.pattern, self.channel) else {
            return container(text("No channel selected").size(13).color(style::TEXT_DIM))
                .center(Fill)
                .style(style::rack_well)
                .into();
        };
        let rack_row = |id: ChannelId| self.project.channels.keys().position(|&c| c == id);
        let color = style::channel_color(rack_row(target.channel).unwrap_or(0));
        let ghosts = target
            .notes
            .iter()
            .filter(|note| note.channel != target.channel)
            .map(|note| {
                let color = style::channel_color(rack_row(note.channel).unwrap_or(0));
                (note.clone(), color)
            })
            .collect();
        let shown = self.shown_notes(&target);
        let end = target.end();
        let (live, song_tick) = live_from(self.live, target.channel, self.playhead, self.origin);
        let grid = canvas::Canvas::new(Grid {
            view: self.roll.view,
            end,
            shown: shown.clone(),
            ghosts,
            color,
            playhead: self
                .playhead
                .map(|tick| Ticks((tick.0 - self.origin.0).rem_euclid(end.0.max(1)))),
            select_box: self.roll.select_box(),
            live,
            song_tick,
            scale: self.roll.scale,
            held: self.roll.grid_held(),
            known_size: self.roll.grid_size(),
            cache: self.grid_cache,
        })
        .width(Fill)
        .height(Fill);
        let mut layers = vec![Element::from(grid)];
        layers.extend(self.notes_menu());
        let grid = stack(layers).width(Fill).height(Fill);
        let height = self.roll.lane_height();
        let lane = canvas::Canvas::new(Lane {
            view: self.roll.view,
            shown,
            color,
            height,
            held: self.roll.lane_held(),
            ramp: self.roll.ramp_line(),
            cache: self.lane_cache,
        })
        .width(Fill)
        .height(height);
        let bar = canvas::Canvas::new(Scrollbar {
            span: self.roll.span(Some(&target)),
            message: |message| Message::Roll(RollMessage::Bar(message)),
        })
        .width(Fill)
        .height(scroll::BAR_HEIGHT);
        // Under the key column the bar's row is the well: the bar runs under the timeline.
        let under_keys = container(space())
            .width(KEY_COLUMN_WIDTH)
            .height(scroll::BAR_HEIGHT)
            .style(style::rack_well);
        let handle = canvas::Canvas::new(Grip {
            held: self.roll.handle_held(),
            pressed: |y| Message::Roll(RollMessage::HandlePressed(y)),
            dragged: |y| Message::Roll(RollMessage::HandleDragged(y)),
            released: Message::Roll(RollMessage::HandleReleased),
            reset: None,
            hover: Hover::Roll(RollPart::LaneHandle),
        })
        .width(Fill)
        .height(HANDLE_HEIGHT);
        let name = self
            .project
            .channels
            .get(&target.channel)
            .map_or("", |channel| channel.name.as_str());
        column![self.strip(name), grid, row![under_keys, bar], handle, lane].into()
    }

    /// The channel's name, the snap choice, the draw length, the Preview and Follow switches, the
    /// velocity while a lane drag sets one, the scale, Fold, and Notes at the right end.
    fn strip(&self, name: &'a str) -> Element<'a, Message> {
        let snap = pick_list(Snap::ALL, Some(self.roll.snap), |snap| {
            Message::Roll(RollMessage::SnapSelected(snap))
        })
        .text_size(12)
        .padding([2, 6]);
        let snap = hover_area(snap, Hover::Roll(RollPart::Snap));
        let length = format_length(self.roll.draw_length, self.project.ppq);
        let preview = button(text("Preview").size(12))
            .padding([2, 8])
            .style(style::tool_button(self.preview))
            .on_press(Message::ToggleNotePreview);
        let preview = hover_area(preview, Hover::Roll(RollPart::Preview));
        let follow = button(text("Follow").size(12))
            .padding([2, 8])
            .style(style::tool_button(self.follow))
            .on_press(Message::ToggleFollow(Panel::PianoRoll));
        let follow = hover_area(follow, Hover::Roll(RollPart::Follow));
        let mut content = row![
            text(name).size(13),
            text("Snap").size(12).color(style::TEXT_DIM),
            snap,
            text(format!("Len {length}")).size(12),
            preview,
            follow,
        ]
        .spacing(10)
        .align_y(Center);
        if let Some(steps) = self.roll.dragged_velocity() {
            content = content.push(text(format!("Vel {steps}")).size(12).color(style::ACCENT));
        }
        if self.recording_notes {
            let light = text("\u{25cf} Rec notes")
                .size(12)
                .color(style::RECORD_ARMED);
            content = content.push(hover_area(light, Hover::Roll(RollPart::RecNotes)));
        }
        let scale = pick_list(ScaleMode::ALL, Some(self.roll.scale.mode), |mode| {
            Message::Roll(RollMessage::ScaleSelected(mode))
        })
        .text_size(12)
        .padding([2, 6]);
        content = content.push(hover_area(scale, Hover::Roll(RollPart::Scale)));
        if self.roll.scale.is_on() {
            let root = pick_list(Root::ALL, Some(self.roll.scale.root), |root| {
                Message::Roll(RollMessage::RootSelected(root))
            })
            .text_size(12)
            .padding([2, 6]);
            content = content.push(hover_area(root, Hover::Roll(RollPart::ScaleRoot)));
        }
        let fold = button(text("Fold").size(12))
            .padding([2, 8])
            .style(style::tool_button(self.roll.fold))
            .on_press(Message::Roll(RollMessage::ToggleFold));
        content = content.push(hover_area(fold, Hover::Roll(RollPart::Fold)));
        let notes = button(text("Notes \u{25be}").size(12))
            .padding([2, 8])
            .style(style::tool_button(self.roll.menu.is_some()))
            .on_press(Message::Roll(RollMessage::MenuButton));
        content = content.push(space().width(Fill));
        // Notes stays last, the strip's right edge by design.
        content = content.push(hover_area(notes, Hover::Roll(RollPart::NotesMenu)));
        container(content)
            .padding([0, 12])
            .width(Fill)
            .height(STRIP_HEIGHT)
            .align_y(Center)
            .style(style::strip)
            .into()
    }

    /// The notes menu where it was opened, on the window's top layer: a row per
    /// [`NoteRow::ALL`].
    fn notes_menu(&self) -> Option<Element<'a, Message>> {
        let at = self.roll.menu?;
        let selected = !self.roll.selection.is_empty();
        let rows = NoteRow::ALL.map(|row| {
            let message = note_row_message(row, selected, self.clipboard);
            hover_area(
                menu::row(row.label(), message),
                Hover::Menu(MenuRow::Notes(row)),
            )
        });
        let panel = menu::panel(column(rows), NOTE_MENU_WIDTH);
        Some(
            pin(menu::floating(panel, self.menu_foot))
                .x(at.x)
                .y(at.y)
                .into(),
        )
    }

    /// The channel's notes as they would be if the held drag were released now.
    fn shown_notes(&self, target: &Target) -> Vec<Shown> {
        let Some((change, selection)) = self.roll.preview(target, self.modifiers) else {
            return target
                .channel_notes()
                .map(|note| Shown {
                    selected: self.roll.selection.contains(&key_of(note)),
                    note: note.clone(),
                })
                .collect();
        };
        let removed: BTreeSet<NoteKey> = change.remove.iter().map(key_of).collect();
        let mut notes: Vec<Note> = target
            .channel_notes()
            .filter(|note| !removed.contains(&key_of(note)))
            .cloned()
            .chain(change.add)
            .collect();
        notes.sort_by_key(|note| (note.start, note.key));
        notes
            .into_iter()
            .map(|note| Shown {
                selected: selection.contains(&key_of(&note)),
                note,
            })
            .collect()
    }
}

fn is_black(key: u8) -> bool {
    BLACK_KEYS.contains(&(key % 12))
}

fn row_color(key: u8) -> Color {
    if is_black(key) {
        style::ROLL_ROW_BLACK
    } else {
        style::ROLL_ROW_WHITE
    }
}

/// `key`'s row color with `scale` chosen: darker outside it.
fn scaled_row(key: u8, scale: Scale) -> Color {
    let row = row_color(key);
    if scale.holds(key) {
        row
    } else {
        style::out_of_scale(row)
    }
}

/// The keys with a row at least partly inside a grid `height` tall, top row first.
fn visible_keys(view: &View, height: f32) -> impl Iterator<Item = u8> + '_ {
    // Row n starts at y = 12n, so rows n = 0..ceil(height/12) show.
    let rows = (height / ROW_HEIGHT).ceil().max(1.0) as u16;
    (0..rows).filter_map(move |row| {
        u8::try_from(view.key_at(f32::from(row) * ROW_HEIGHT))
            .ok()
            .filter(|&key| key <= dawsome_engine::project::MAX_KEY)
    })
}

/// Fills `rect` with `edge`, then all but a `border`-wide rim of it with `fill`.
fn fill_bordered(frame: &mut Frame, rect: Rectangle, border: f32, edge: Color, fill: Color) {
    frame.fill_rectangle(rect.position(), rect.size(), edge);
    if rect.width > 2.0 * border && rect.height > 2.0 * border {
        frame.fill_rectangle(
            Point::new(rect.x + border, rect.y + border),
            Size::new(rect.width - 2.0 * border, rect.height - 2.0 * border),
            fill,
        );
    }
}

/// The note grid with its key column.
struct Grid<'a> {
    view: View,
    /// The pattern's end: rows past it are dimmed.
    end: Ticks,
    shown: Vec<Shown>,
    /// The other channels' notes in the pattern, with their rack colors.
    ghosts: Vec<(Note, Color)>,
    color: Color,
    playhead: Option<Ticks>,
    select_box: Option<(Point, Point)>,
    /// The shown channel's notes played live: each key and the song tick it started at.
    live: Vec<(u8, Option<Ticks>)>,
    /// The song's tick while it plays, where live bars grow to.
    song_tick: Option<Ticks>,
    /// The scale whose outside keys shade darker.
    scale: Scale,
    /// Whether a move, resize or box select is held, so drags and the release are reported
    /// wherever the cursor is.
    held: bool,
    /// The size the app last heard of; a different size is reported.
    known_size: Size,
    cache: &'a Cache,
}

impl Grid<'_> {
    fn draw_body(&self, frame: &mut Frame) {
        let size = frame.size();
        let view = &self.view;
        let keys = || visible_keys(view, size.height);
        let timeline_end = size.width.max(KEY_COLUMN_WIDTH);
        let end_x = view.x_of(self.end).clamp(KEY_COLUMN_WIDTH, timeline_end);
        for key in keys() {
            let y = view.row_top(key);
            let color = scaled_row(key, self.scale);
            frame.fill_rectangle(
                Point::new(KEY_COLUMN_WIDTH, y),
                Size::new(end_x - KEY_COLUMN_WIDTH, ROW_HEIGHT),
                color,
            );
            frame.fill_rectangle(
                Point::new(end_x, y),
                Size::new(timeline_end - end_x, ROW_HEIGHT),
                style::dimmed(color),
            );
        }
        self.draw_lines(frame, size);
        for (note, color) in &self.ghosts {
            // Another channel's note on a key the fold leaves out has no row to show in.
            if !view.rows.shows(note.key) {
                continue;
            }
            let row = scaled_row(note.key, self.scale);
            let row = if note.start >= self.end {
                style::dimmed(row)
            } else {
                row
            };
            let rect = view.note_rect(note);
            frame.fill_rectangle(rect.position(), rect.size(), style::ghost(*color, row));
        }
        for shown in &self.shown {
            let (edge, border) = if shown.selected {
                (style::ACCENT, SELECTED_BORDER)
            } else {
                (style::BUTTON_EDGE, NOTE_BORDER)
            };
            fill_bordered(frame, view.note_rect(&shown.note), border, edge, self.color);
        }
        // The key column goes on top, covering notes scrolled to the left of the timeline.
        for key in keys() {
            let y = view.row_top(key);
            let face = if is_black(key) {
                style::KEY_BLACK
            } else {
                style::KEY_WHITE
            };
            let face = if self.scale.holds(key) {
                face
            } else {
                style::out_of_scale(face)
            };
            frame.fill_rectangle(
                Point::new(0.0, y),
                Size::new(KEY_COLUMN_WIDTH, ROW_HEIGHT),
                face,
            );
            frame.fill_rectangle(
                Point::new(0.0, y + ROW_HEIGHT - 1.0),
                Size::new(KEY_COLUMN_WIDTH, 1.0),
                style::KEY_EDGE,
            );
            if key % 12 == 0 {
                frame.fill_text(Text {
                    content: key_name(key),
                    position: Point::new(LABEL_RIGHT, y + ROW_HEIGHT / 2.0),
                    color: style::KEY_LABEL,
                    size: LABEL_SIZE.into(),
                    align_x: Horizontal::Right.into(),
                    align_y: Vertical::Center,
                    ..Text::default()
                });
            }
        }
    }

    /// What the hint bar names at grid point `at`: the key column's key, a note (its key,
    /// velocity and whether on its right edge) or the empty grid's row.
    fn hover_at(&self, at: Point) -> Option<Hover> {
        grid_hover(&self.view, self.shown.iter().map(|shown| &shown.note), at)
    }

    /// A line at every line unit: bars 2 px wide, beats and finer lines 1 px. Lines are not
    /// dimmed past the end.
    fn draw_lines(&self, frame: &mut Frame, size: Size) {
        let view = &self.view;
        let (unit, beat, bar) = (view.line_unit().0.max(1), view.beat().0, view.bar().0);
        let first = (view.scroll.0 + unit - 1).div_euclid(unit) * unit;
        let last = view.tick_at(size.width).0;
        let mut tick = first;
        while tick <= last {
            let x = view.x_of(Ticks(tick));
            let (color, width) = if tick % bar == 0 {
                (style::BAR_LINE, BAR_LINE_WIDTH)
            } else if tick % beat == 0 {
                (style::BEAT_LINE, LINE_WIDTH)
            } else {
                (style::GRID_LINE, LINE_WIDTH)
            };
            frame.fill_rectangle(Point::new(x, 0.0), Size::new(width, size.height), color);
            tick += unit;
        }
    }
}

/// What the hint bar names at grid point `at` of a grid showing `view` with the channel's
/// `notes`: the key column's key, a note (its key, velocity and whether on its right edge) or
/// the empty grid's row; nothing above or below the keyboard.
pub fn grid_hover<'n>(
    view: &View,
    notes: impl DoubleEndedIterator<Item = &'n Note>,
    at: Point,
) -> Option<Hover> {
    let key = u8::try_from(view.key_at(at.y))
        .ok()
        .filter(|&key| key <= dawsome_engine::project::MAX_KEY)?;
    if at.x < KEY_COLUMN_WIDTH {
        return Some(Hover::Roll(RollPart::Keys { key }));
    }
    let part = match piano_layout::note_under(view, notes, at.x, at.y) {
        Some((note, part)) => RollPart::Note {
            key: note.key,
            velocity: velocity_steps(note.velocity),
            edge: part == piano_layout::NotePart::RightEdge,
        },
        None => RollPart::Empty { key },
    };
    Some(Hover::Roll(part))
}

impl canvas::Program<Message> for Grid<'_> {
    type State = GridState;

    fn update(
        &self,
        state: &mut GridState,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let roll = |message| canvas::Action::publish(Message::Roll(message));
        let local = |position: Point| Point::new(position.x - bounds.x, position.y - bounds.y);
        let input = match event {
            Event::Mouse(mouse::Event::ButtonPressed(
                button @ (mouse::Button::Left | mouse::Button::Right),
            )) => cursor.position_in(bounds).map(|at| {
                state.pointer.last = Some(at);
                let left = *button == mouse::Button::Left;
                let now = crate::ui::clock::now();
                let double = left
                    && state.previous_press.is_some_and(|(previous, time)| {
                        now.duration_since(time) <= DOUBLE_PRESS_TIME
                            && previous.distance(at) <= DOUBLE_PRESS_DISTANCE
                    });
                // A double-click is not the first of another one.
                state.previous_press = (left && !double).then_some((at, now));
                roll(RollMessage::GridPressed {
                    at,
                    button: *button,
                    double,
                })
                .and_capture()
            }),
            Event::Mouse(mouse::Event::CursorMoved { position }) if self.held => {
                let at = local(*position);
                state.pointer.last = Some(at);
                Some(roll(RollMessage::GridDragged(at)))
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let now = cursor.position_in(bounds).and_then(|at| self.hover_at(at));
                hover_change(&mut state.pointer.hover, now)
            }
            // The window says only this when the cursor leaves it: the hover goes, so the roll
            // does not keep the keys it gives.
            Event::Mouse(mouse::Event::CursorLeft) => hover_change(&mut state.pointer.hover, None),
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if self.held => cursor
                .position()
                .map(local)
                .or(state.pointer.last)
                .map(|at| roll(RollMessage::GridReleased(at))),
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => cursor
                .position_in(bounds)
                .map(|at| roll(RollMessage::GridScrolled { at, delta: *delta }).and_capture()),
            _ => None,
        };
        // A new size is reported on the next event that carries no input, so no press is lost.
        input.or_else(|| {
            (bounds.size() != self.known_size)
                .then(|| roll(RollMessage::GridResized(bounds.size())))
        })
    }

    fn draw(
        &self,
        _state: &GridState,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let body = self
            .cache
            .draw(renderer, bounds.size(), |frame| self.draw_body(frame));
        let mut overlay = Frame::new(renderer, bounds.size());
        if let Some(tick) = self.playhead {
            let x = self.view.x_of(tick);
            if x >= KEY_COLUMN_WIDTH {
                overlay.fill_rectangle(
                    Point::new(x, 0.0),
                    Size::new(PLAYHEAD_WIDTH, bounds.height),
                    style::ACCENT,
                );
            }
        }
        // Live notes: the key lit in the key column, and the bar (or the strip's mark).
        let live = Color {
            a: 0.55,
            ..style::ACCENT
        };
        for &(key, started) in &self.live {
            if !self.view.rows.shows(key) {
                continue;
            }
            let lit = live_notes::lit_key(&self.view, key);
            overlay.fill_rectangle(lit.position(), lit.size(), style::ACCENT);
            for bar in live_notes::bars(&self.view, self.end, self.song_tick, key, started) {
                overlay.fill_rectangle(bar.position(), bar.size(), live);
            }
        }
        if let Some((from, to)) = self.select_box {
            let corner = Point::new(from.x.min(to.x), from.y.min(to.y));
            let size = Size::new((from.x - to.x).abs(), (from.y - to.y).abs());
            overlay.stroke(
                &Path::rectangle(corner, size),
                Stroke::default().with_color(style::ACCENT).with_width(1.0),
            );
        }
        vec![body, overlay.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _state: &GridState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if self.held {
            return mouse::Interaction::Grabbing;
        }
        let Some(at) = cursor.position_in(bounds) else {
            return mouse::Interaction::default();
        };
        if at.x < KEY_COLUMN_WIDTH {
            return mouse::Interaction::default();
        }
        let notes = self.shown.iter().map(|shown| &shown.note);
        match piano_layout::note_under(&self.view, notes, at.x, at.y) {
            Some((_, piano_layout::NotePart::RightEdge)) => {
                mouse::Interaction::ResizingHorizontally
            }
            Some(_) => mouse::Interaction::Grab,
            None => mouse::Interaction::Crosshair,
        }
    }
}

/// The velocity lane: a stem per note of the channel, as tall as its velocity.
struct Lane<'a> {
    view: View,
    shown: Vec<Shown>,
    color: Color,
    /// The lane's height, which its velocity scale spans.
    height: f32,
    /// Whether a velocity drag is held, so drags and the release are reported anywhere.
    held: bool,
    /// The ramp being drawn (Alt+drag), end to end.
    ramp: Option<(Point, Point)>,
    cache: &'a Cache,
}

impl canvas::Program<Message> for Lane<'_> {
    type State = Pointer;

    fn update(
        &self,
        state: &mut Pointer,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let roll = |message| canvas::Action::publish(Message::Roll(message));
        let local = |position: Point| Point::new(position.x - bounds.x, position.y - bounds.y);
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                cursor.position_in(bounds).map(|at| {
                    state.last = Some(at);
                    roll(RollMessage::LanePressed(at)).and_capture()
                })
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) if self.held => {
                let at = local(*position);
                state.last = Some(at);
                Some(roll(RollMessage::LaneDragged(at)))
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let notes = self.shown.iter().map(|shown| &shown.note);
                let now = cursor
                    .position_in(bounds)
                    .and_then(|at| piano_layout::stem_under(&self.view, notes, at.x))
                    .map(|_| Hover::Roll(RollPart::Stem));
                hover_change(&mut state.hover, now)
            }
            Event::Mouse(mouse::Event::CursorLeft) => hover_change(&mut state.hover, None),
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if self.held => cursor
                .position()
                .map(local)
                .or(state.last)
                .map(|at| roll(RollMessage::LaneReleased(at))),
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
        let lane = self.cache.draw(renderer, bounds.size(), |frame| {
            frame.fill_rectangle(Point::ORIGIN, frame.size(), style::RACK_WELL);
            for shown in &self.shown {
                let steps = velocity_steps(shown.note.velocity);
                let stem = self.view.stem_rect(&shown.note, steps, self.height);
                // Stems of notes scrolled off to the left of the timeline are not drawn.
                if stem.x < KEY_COLUMN_WIDTH {
                    continue;
                }
                let color = if shown.selected {
                    style::ACCENT
                } else {
                    self.color
                };
                frame.fill_rectangle(stem.position(), stem.size(), color);
            }
        });
        let Some((from, to)) = self.ramp else {
            return vec![lane];
        };
        let mut line = Frame::new(renderer, bounds.size());
        line.stroke(
            &Path::line(from, to),
            Stroke::default().with_color(style::ACCENT).with_width(2.0),
        );
        vec![lane, line.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _state: &Pointer,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if self.held || cursor.is_over(bounds) {
            mouse::Interaction::ResizingVertically
        } else {
            mouse::Interaction::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_keys_follow_the_piano() {
        let black: Vec<u8> = (60..72).filter(|&key| is_black(key)).collect();
        assert_eq!(black, [61, 63, 66, 68, 70]);
        assert_eq!(row_color(60), style::ROLL_ROW_WHITE);
        assert_eq!(row_color(61), style::ROLL_ROW_BLACK);
    }

    // A6: with C major chosen, C (60) keeps its row color and C# (61) shades darker (0.7 of
    // its color over black); with no scale nothing shades.
    #[test]
    fn keys_outside_the_scale_shade_darker() {
        use crate::ui::scale::{Root, Scale, ScaleMode};
        let c_major = Scale {
            root: Root(0),
            mode: ScaleMode::Major,
        };
        assert_eq!(scaled_row(60, c_major), style::ROLL_ROW_WHITE);
        assert_eq!(
            scaled_row(61, c_major),
            style::out_of_scale(style::ROLL_ROW_BLACK)
        );
        assert_eq!(scaled_row(61, Scale::default()), style::ROLL_ROW_BLACK);
        assert_ne!(
            style::out_of_scale(style::ROLL_ROW_BLACK),
            style::ROLL_ROW_BLACK
        );
    }

    // 340 px is 28⅓ rows: keys 69 down to 41 are at least partly visible.
    #[test]
    fn the_grid_draws_every_row_it_shows() {
        let view = View {
            top: 69,
            scroll: Ticks(0),
            zoom: piano_layout::DEFAULT_ZOOM,
            step: Ticks(24),
            rows: crate::ui::piano_layout::Rows::ALL,
        };
        let keys: Vec<u8> = visible_keys(&view, 340.0).collect();
        assert_eq!(keys.first(), Some(&69));
        assert_eq!(keys.last(), Some(&41));
        assert_eq!(
            visible_keys(&view, 336.0).last(),
            Some(42),
            "exactly 28 rows"
        );
        let low = View { top: 10, ..view };
        assert_eq!(visible_keys(&low, 340.0).last(), Some(0));
    }
}
