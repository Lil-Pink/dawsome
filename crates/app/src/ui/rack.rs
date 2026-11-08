//! The channel rack: one row per channel with its mute light, name button, pan and volume
//! knobs, and step buttons, under a header with the pattern's swing knob and the selected
//! pattern's bar count; under the rows "+ Instrument…" opens the instrument picker. One canvas
//! draws the rows from [`layout`] positions, another the header. The rows
//! live in a cached layer; the playhead has a cheap layer of its own, redrawn every frame. A
//! sampler row takes a sound file: dropped on it (from the browser or the desktop), picked from
//! its name's menu, or picked with a double press on the name while it has none; while it has
//! none, a dim hint right of its steps says so.

use std::collections::BTreeMap;
use std::f32::consts::PI;
use std::ops::RangeInclusive;
use std::time::{Duration, Instant};

use dawsome_engine::edit::Edit;
use dawsome_engine::project::{
    AutomationTarget, Channel, ChannelId, ChannelKind, InputSource, PatternId, Project, MAX_VOLUME,
    PAN_RANGE,
};
use dawsome_engine::time::SWING_RANGE;
use dawsome_engine::transport::ParamTarget;
use iced::widget::canvas::{self, Cache, Frame, Geometry, LineCap, Path, Stroke, Text};
use iced::{alignment, mouse, window, Point, Rectangle, Renderer, Size, Theme, Vector};

use crate::app::{MediaMessage, Message, PatternMessage, SWING_STEP};
use crate::ui::add_words::{self, Look};
use crate::ui::fonts::size;
use crate::ui::hint::{hover_change, Hover, RackPart};
use crate::ui::layout::{
    self, HeaderHit, RackHit, CONTROL_SIZE, PAN_KNOB_X, RACK_HEADER_HEIGHT, RACK_TITLE_X,
    STEPS_PER_GROUP, VOLUME_KNOB_X,
};
use crate::ui::mixer::TIP_HEIGHT;
use crate::ui::mixer_state::{pan_text, percent_text};
use crate::ui::panels::Panel;
use crate::ui::picker::PickerTarget;
use crate::ui::rack_rows::{RackRow, RackRows};
use crate::ui::scroll::{BarMessage, Span};
use crate::ui::step_grid::{self, StepGrid};
use crate::ui::style;
use crate::ui::text_fit::fit_width;

/// The room left of and right of a name button's text: a longer name is cut with an ellipsis to
/// fit.
const NAME_PADDING: f32 = 8.0;

/// The value tip over a knob: its width, centered on the knob, and the gap between it and the
/// knob.
const KNOB_TIP_WIDTH: f32 = 52.0;
const KNOB_TIP_GAP: f32 = 2.0;
/// A knob turns through 270 degrees, from down-left through straight up to down-right.
const KNOB_SWEEP: f32 = 1.5 * PI;
/// The dot an automated control carries, across.
const AUTOMATED_DOT: f32 = 4.0;
/// The dot a control linked to a MIDI controller carries, across, and how far right of the
/// control's box it sits (clear of the automated dot).
const LINKED_DOT: f32 = 2.0;
const LINKED_DOT_GAP: f32 = 1.0;
/// Two presses on one name button this close together open the channel's plugin; on the
/// header's swing, they set it straight.
const DOUBLE_PRESS: Duration = Duration::from_millis(400);

/// A channel knob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Knob {
    Pan,
    Volume,
}

impl Knob {
    /// The values the knob sweeps, from its left stop to its right stop.
    pub fn range(self) -> RangeInclusive<f32> {
        match self {
            // Linear in gain: the mixer's faders have the curve.
            Knob::Volume => 0.0..=MAX_VOLUME,
            Knob::Pan => PAN_RANGE,
        }
    }

    pub fn value(self, channel: &Channel) -> f32 {
        match self {
            Knob::Volume => channel.volume,
            Knob::Pan => channel.pan,
        }
    }

    /// The edit that sets this knob of `channel` to `value`.
    pub fn edit(self, channel: ChannelId, value: f32) -> Edit {
        match self {
            Knob::Volume => Edit::SetChannelVolume { channel, value },
            Knob::Pan => Edit::SetChannelPan { channel, value },
        }
    }

    /// Where the live value goes while this knob of `channel` turns.
    pub fn target(self, channel: ChannelId) -> ParamTarget {
        match self {
            Knob::Volume => ParamTarget::ChannelVolume(channel),
            Knob::Pan => ParamTarget::ChannelPan(channel),
        }
    }

    /// What an automation of this knob of `channel` sets.
    pub fn automation(self, channel: ChannelId) -> AutomationTarget {
        match self {
            Knob::Volume => AutomationTarget::ChannelVolume(channel),
            Knob::Pan => AutomationTarget::ChannelPan(channel),
        }
    }

    /// `value` as the knob's tip and the hint bar say it: "100%", or "Center", "50% L".
    pub fn text(self, value: f32) -> String {
        match self {
            Knob::Volume => percent_text(value),
            Knob::Pan => pan_text(value),
        }
    }

    /// Where `value` sits on the sweep: 0 at the left stop, 1 at the right (an automation's
    /// normalized value, as the knob's range is the automation's).
    pub fn fraction(self, value: f32) -> f32 {
        let range = self.range();
        (value - range.start()) / (range.end() - range.start())
    }
}

/// The rack for one pattern of a project.
pub struct ChannelRack<'a> {
    pub project: &'a Project,
    pub pattern: Option<PatternId>,
    pub selected: Option<ChannelId>,
    /// The step under the playhead, `None` while stopped.
    pub playhead: Option<u32>,
    /// The automated controls, with the value each automation set last while it governs: an
    /// automated knob shows that value, and a dot.
    pub automated: &'a BTreeMap<AutomationTarget, Option<f32>>,
    /// Holds the drawn rows; the view holding it clears it whenever what they show changes.
    pub cache: &'a Cache,
    /// A file from the browser is being dragged.
    pub browser_drag: bool,
    /// The sampler row the dragged file is over, as the window last heard.
    pub drop_sampler: Option<ChannelId>,
    /// The rows, the audio takes folded under their header ([`RackRows`]).
    pub rows: RackRows,
}

impl ChannelRack<'_> {
    fn channel_at(&self, row: usize) -> Option<ChannelId> {
        self.rows.channel(row)
    }

    /// Channel `id`, when it is a sampler.
    fn sampler(&self, id: ChannelId) -> Option<&Channel> {
        self.project
            .channels
            .get(&id)
            .filter(|channel| channel.kind == ChannelKind::Sampler)
    }

    /// The sampler whose row (anywhere along it) the cursor is over.
    fn sampler_at(&self, bounds: Rectangle, cursor: mouse::Cursor) -> Option<ChannelId> {
        let point = cursor.position_in(bounds)?;
        let row = layout::row_at(point.y, self.rows.len())?;
        let id = self.channel_at(row)?;
        self.sampler(id).map(|_| id)
    }

    /// A file over the rows: held from the browser, the sampler row it is over (said when that
    /// changes); dropped from the desktop onto a sampler row, that row takes it. A desktop drop
    /// anywhere else is left to the window, which places the file on the playlist. Why by the
    /// cursor the window last saw: the desktop's drop event carries no position of its own, so
    /// a drag from another program, during which the window may never see the cursor, can land
    /// on the playlist even when let go over a row.
    fn file_moved_or_dropped(
        &self,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) if self.browser_drag => {
                let over = self.sampler_at(bounds, cursor);
                (over != self.drop_sampler).then(|| {
                    canvas::Action::publish(Message::Media(MediaMessage::OverSampler(over)))
                })
            }
            canvas::Event::Window(window::Event::FileDropped(file)) => {
                let channel = self.sampler_at(bounds, cursor)?;
                let dropped = MediaMessage::DroppedOnSampler {
                    channel,
                    file: file.clone(),
                };
                Some(canvas::Action::publish(Message::Media(dropped)).and_capture())
            }
            _ => None,
        }
    }

    /// The value tip of `knob` in `row`: centered over the knob, or under it in the first row,
    /// which has no room above; its value as the knob draws it (an automation's while one
    /// governs it).
    fn knob_tip(&self, row: usize, knob: Knob) -> Option<(Rectangle, String)> {
        let id = self.channel_at(row)?;
        let channel = self.project.channels.get(&id)?;
        let automated = self.automated.get(&knob.automation(id)).copied().flatten();
        let value = automated.unwrap_or(knob.value(channel));
        let knob_x = match knob {
            Knob::Pan => PAN_KNOB_X,
            Knob::Volume => VOLUME_KNOB_X,
        };
        let center = layout::knob_center(knob_x, row);
        let half = CONTROL_SIZE / 2.0;
        let above = center.y - half - KNOB_TIP_GAP - TIP_HEIGHT;
        let y = if above >= 0.0 {
            above
        } else {
            center.y + half + KNOB_TIP_GAP
        };
        let rect = Rectangle::new(
            Point::new(center.x - KNOB_TIP_WIDTH / 2.0, y),
            Size::new(KNOB_TIP_WIDTH, TIP_HEIGHT),
        );
        Some((rect, knob.text(value)))
    }

    fn hit(&self, bounds: Rectangle, cursor: mouse::Cursor) -> Option<RackHit> {
        let point = cursor.position_in(bounds)?;
        let steps = step_grid::step_count(self.project, self.pattern);
        layout::hit_test(point, self.rows.len(), steps)
    }

    /// Whether `cursor` is over "+ Instrument…" under the rows.
    fn on_add(&self, bounds: Rectangle, cursor: mouse::Cursor) -> bool {
        cursor
            .position_in(bounds)
            .is_some_and(|point| layout::add_instrument_rect(self.rows.len()).contains(point))
    }

    /// The rows with their steps (and an empty sampler's hint) moved `steps` px left: those
    /// first, then the light, name and knobs over them, on the well's ground where the steps
    /// are scrolled under them. Why drawn over rather than clipped: the software renderer draws
    /// a clipped frame's shapes and text whole.
    fn draw_rows(&self, frame: &mut Frame, steps: f32) {
        let grid = StepGrid::new(self.project, self.pattern);
        // A channel's color is its place among the channels, as the piano roll colors its
        // notes, whatever the rows the takes fold away.
        let shown: Vec<(usize, usize, ChannelId, &Channel)> = self
            .rows
            .channels()
            .filter_map(|(row, id)| {
                let (place, (_, channel)) = self
                    .project
                    .channels
                    .iter()
                    .enumerate()
                    .find(|(_, (&other, _))| other == id)?;
                Some((row, place, id, channel))
            })
            .collect();
        frame.with_save(|frame| {
            frame.translate(Vector::new(-steps, 0.0));
            for &(row, place, id, channel) in &shown {
                for step in 0..grid.steps() {
                    let color = if grid.is_lit(id, step) {
                        style::channel_color(place)
                    } else {
                        style::unlit_step(step)
                    };
                    let rect = layout::step_rect(row, step);
                    frame.fill_rectangle(rect.position(), rect.size(), color);
                    if step > 0 && step % STEPS_PER_GROUP == 0 {
                        draw_beat_line(frame, row, step);
                    }
                }
                if is_empty_sampler(channel) {
                    draw_empty_hint(frame, row, grid.steps());
                }
                if let Some(input) = channel.input.as_ref() {
                    draw_input_badge(frame, row, grid.steps(), input);
                }
            }
        });
        if steps > 0.0 {
            let height = frame.height();
            let column = Size::new(layout::STEPS_X, height);
            frame.fill_rectangle(Point::ORIGIN, column, style::palette().bg);
        }
        for &(row, _, id, channel) in &shown {
            draw_mute_light(
                frame,
                layout::mute_led_center(row),
                layout::MUTE_LED_RADIUS,
                (!channel.mute).then_some(style::LED_ON),
            );
            draw_name_button(frame, row, &channel.name, self.selected == Some(id));
            for (knob, knob_x) in [(Knob::Pan, PAN_KNOB_X), (Knob::Volume, VOLUME_KNOB_X)] {
                let automated = self.automated.get(&knob.automation(id));
                let value = automated.copied().flatten().unwrap_or(knob.value(channel));
                let center = layout::knob_center(knob_x, row);
                draw_knob(frame, center, CONTROL_SIZE, knob.fraction(value));
                if automated.is_some() {
                    draw_automated_dot(frame, center, CONTROL_SIZE);
                }
                if self.project.midi_links.get(&knob.automation(id)).is_some() {
                    draw_linked_dot(frame, center, CONTROL_SIZE);
                }
            }
        }
        if let Some((row, count, open)) = self.rows.takes_header() {
            draw_takes_header(frame, row, count, open);
        }
    }

    /// The rows (cached), the knob's value tip, the playhead along the steps and "+ Instrument…"
    /// under the rows (lit under the pointer, so drawn each frame, as "+ Track" is), with the
    /// steps scrolled `steps` px left; a playhead mark on a step scrolled under the knobs is
    /// left out.
    fn draw_with_steps(
        &self,
        state: &RackState,
        renderer: &Renderer,
        bounds: Rectangle,
        steps: f32,
    ) -> Vec<Geometry> {
        let rows = self.cache.draw(renderer, bounds.size(), |frame| {
            self.draw_rows(frame, steps)
        });
        let mut playhead = Frame::new(renderer, bounds.size());
        // The knob held down, else the one under the cursor, says its value as the mixer's
        // controls do.
        let hovered = match state.hover {
            Some(Hover::Rack {
                row,
                part: RackPart::Pan,
            }) => Some((row, Knob::Pan)),
            Some(Hover::Rack {
                row,
                part: RackPart::Volume,
            }) => Some((row, Knob::Volume)),
            _ => None,
        };
        if let Some((rect, value)) = state
            .held
            .or(hovered)
            .and_then(|(row, knob)| self.knob_tip(row, knob))
        {
            crate::ui::mixer::draw_tip(&mut playhead, rect, &value);
        }
        if let Some(step) = self.playhead {
            for (row, _) in self.rows.channels() {
                let rect = layout::step_rect(row, step);
                let x = rect.x - steps;
                if x < layout::STEPS_X {
                    continue;
                }
                playhead.fill_rectangle(
                    Point::new(x, rect.y),
                    Size::new(rect.width, layout::PLAYHEAD_EDGE),
                    style::accent(),
                );
            }
        }
        let look = if state.hover == Some(Hover::RackAdd) {
            Look::Lit
        } else {
            Look::Resting
        };
        let add = layout::add_instrument_rect(self.rows.len());
        add_words::draw(&mut playhead, layout::ADD_INSTRUMENT_LABEL, add, look);
        vec![rows, playhead.into_geometry()]
    }
}

/// The rack's rows with their steps scrolled along: the light, name and knobs stay put, the
/// steps (and an empty sampler's hint) move `steps.offset` px left under them. Presses and the
/// cursor over the steps reach the step drawn there; Shift+wheel, or a wheel across, over the
/// steps scrolls them; the canvas says its width when it changes, so the bar under the rows
/// knows the view.
pub struct ScrolledRack<'a> {
    pub rack: ChannelRack<'a>,
    /// The steps' scroll, in pixels from the first step's left edge.
    pub steps: Span,
    /// The canvas's width the window last heard.
    pub known_width: f32,
    /// Shift is held: the wheel scrolls the steps rather than the rows.
    pub shift: bool,
}

impl ScrolledRack<'_> {
    /// `cursor` as the unscrolled rack sees it, with `bounds` widened to hold the steps: over
    /// the steps it is moved the scroll right, over the light, name and knobs it stays.
    fn unscrolled(&self, bounds: Rectangle, cursor: mouse::Cursor) -> (Rectangle, mouse::Cursor) {
        let offset = self.steps.offset as f32;
        let wide = Rectangle {
            width: bounds.width + offset,
            ..bounds
        };
        let moved = match (cursor.position(), cursor.position_in(bounds)) {
            (Some(at), Some(local)) if local.x >= layout::STEPS_X => {
                mouse::Cursor::Available(Point::new(at.x + offset, at.y))
            }
            _ => cursor,
        };
        (wide, moved)
    }
}

impl canvas::Program<Message> for ScrolledRack<'_> {
    type State = RackState;

    fn update(
        &self,
        state: &mut RackState,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if let canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) = event {
            let over_steps = cursor
                .position_in(bounds)
                .is_some_and(|at| at.x >= layout::STEPS_X);
            let (mouse::ScrollDelta::Lines { x, y } | mouse::ScrollDelta::Pixels { x, y }) = *delta;
            let (across, notch) = crate::ui::scroll::wheel(x, y);
            if over_steps && (self.shift || across) && notch != 0.0 {
                // Down or right is later, as the timelines' Shift+wheel has it.
                let offset = self.steps.wheeled(notch < 0.0);
                let message = Message::RackBar(BarMessage::To {
                    offset,
                    held: false,
                });
                return Some(canvas::Action::publish(message).and_capture());
            }
        }
        let (wide, moved) = self.unscrolled(bounds, cursor);
        let input = self.rack.update(state, event, wide, moved);
        // A new width is said on the next event that carries no input, so no press is lost.
        input.or_else(|| {
            (bounds.width != self.known_width)
                .then(|| canvas::Action::publish(Message::RackResized(bounds.width)))
        })
    }

    fn draw(
        &self,
        state: &RackState,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        self.rack
            .draw_with_steps(state, renderer, bounds, self.steps.offset as f32)
    }

    fn mouse_interaction(
        &self,
        state: &RackState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        let (wide, moved) = self.unscrolled(bounds, cursor);
        self.rack.mouse_interaction(state, wide, moved)
    }
}

/// What a sampler row without a sound says right of its steps, a line each. Why two lines:
/// one did not fit beside a bar of steps in a 1000 px window.
pub const EMPTY_SAMPLER_HINT: [&str; 2] =
    ["Drag a sound here, or", "double-click one in the Library"];

/// The distance between the hint's two lines' middles, and its room right of the steps: with a
/// bar of steps (730 px) the rack stays inside a 1000 px window.
const HINT_LINE: f32 = 13.0;
pub const HINT_WIDTH: f32 = 260.0;

/// The dim hint right of the steps of `row`, a sampler without a sound, `steps` steps long, each
/// line cut with an ellipsis to [`HINT_WIDTH`].
fn draw_empty_hint(frame: &mut Frame, row: usize, steps: u32) {
    let left = layout::rack_size(row + 1, steps).width;
    let middle = layout::name_button_rect(row).center_y();
    for (line, offset) in EMPTY_SAMPLER_HINT.iter().zip([-0.5, 0.5]) {
        let Some(words) = fit_width(line, HINT_WIDTH - 8.0, size::LABEL) else {
            continue;
        };
        frame.fill_text(Text {
            content: words.into_owned(),
            position: Point::new(left, middle + offset * HINT_LINE),
            color: style::palette().text_secondary,
            size: size::LABEL.into(),
            align_y: alignment::Vertical::Center,
            ..Text::default()
        });
    }
}

/// The "IN" badge's size and the gap after it.
const BADGE: Size = Size::new(20.0, 13.0);
const BADGE_GAP: f32 = 6.0;

/// An input channel's badge right of its steps, where an empty sampler says what to do: "IN"
/// on the accent, then its device and channels, dim, cut with an ellipsis to [`HINT_WIDTH`].
fn draw_input_badge(frame: &mut Frame, row: usize, steps: u32, input: &InputSource) {
    let left = layout::rack_size(row + 1, steps).width;
    let middle = layout::name_button_rect(row).center_y();
    let badge = Rectangle::new(Point::new(left, middle - BADGE.height / 2.0), BADGE);
    frame.fill_rectangle(badge.position(), badge.size(), style::accent());
    frame.fill_text(Text {
        content: "IN".into(),
        position: badge.center(),
        color: style::palette().bg,
        size: size::LABEL.into(),
        align_x: alignment::Horizontal::Center.into(),
        align_y: alignment::Vertical::Center,
        ..Text::default()
    });
    let words = format!(
        "{} \u{b7} {}",
        crate::live_input::device_label(&input.device),
        crate::live_input::channels_words(input.channels)
    );
    let room = HINT_WIDTH - 8.0 - BADGE.width - BADGE_GAP;
    if let Some(words) = fit_width(&words, room, size::LABEL) {
        frame.fill_text(Text {
            content: words.into_owned(),
            position: Point::new(badge.x + BADGE.width + BADGE_GAP, middle),
            color: style::palette().text_secondary,
            size: size::LABEL.into(),
            align_y: alignment::Vertical::Center,
            ..Text::default()
        });
    }
}

/// Whether `channel` is a sampler with no sound yet.
pub fn is_empty_sampler(channel: &Channel) -> bool {
    channel.kind == ChannelKind::Sampler && channel.sample.as_os_str().is_empty()
}

/// The rack's header: its title, the selected pattern's bar count between "−" and "+" (dim where
/// the count can go no further), "+ Sampler…", and the pattern's swing knob with its readout.
/// A strip of a few shapes, drawn whole each frame.
pub struct RackHeader {
    /// The selected pattern's length in bars; `None` without a pattern.
    pub bars: Option<u32>,
    /// Whether "−" can shorten the pattern. Why the window says so rather than the header
    /// working it out: the window's rule decides the press, so the header dims exactly the
    /// presses it would refuse.
    pub shorter: bool,
    /// Whether "+" can lengthen it.
    pub longer: bool,
    /// The selected pattern's swing, in tenths of a per cent; `None` without a pattern, when
    /// the knob and its readout are dim and take nothing.
    pub swing: Option<u16>,
}

/// What the header's cursor was over last, and the last press on its swing, to tell a double
/// press.
#[derive(Debug, Default)]
pub struct HeaderState {
    hovered: Option<Hover>,
    last_swing_press: Option<Instant>,
}

/// A swing as its readout, the hint and the quantize dialog show it: "66.7%". Integer
/// arithmetic, so every value reads exactly as stored.
pub fn swing_text(tenths: u16) -> String {
    format!("{}.{}%", tenths / 10, tenths % 10)
}

/// Where the swing knob points for `tenths`: 0 at straight, 1 at its far stop.
fn swing_fraction(tenths: u16) -> f32 {
    let (low, high) = (*SWING_RANGE.start(), *SWING_RANGE.end());
    f32::from(tenths.saturating_sub(low)) / f32::from(high - low)
}

impl RackHeader {
    /// The control under `cursor`: the swing only while there is a pattern to swing.
    fn hit(&self, bounds: Rectangle, cursor: mouse::Cursor) -> Option<HeaderHit> {
        let hit = layout::header_hit_test(cursor.position_in(bounds)?)?;
        (hit != HeaderHit::Swing || self.swing.is_some()).then_some(hit)
    }

    /// The bars "−" or "+" adds now, if it can change the count.
    fn nudge(&self, hit: HeaderHit) -> Option<i32> {
        match hit {
            HeaderHit::BarsDown => self.shorter.then_some(-1),
            HeaderHit::BarsUp => self.longer.then_some(1),
            HeaderHit::Swing => None,
        }
    }

    /// What a single press on `hit` asks for, if anything: on the swing, a drag (the window
    /// runs it, as the tempo's).
    fn message(&self, hit: HeaderHit) -> Option<Message> {
        match hit {
            HeaderHit::Swing => self.swing.map(|_| Message::SwingPressed),
            nudge => self
                .nudge(nudge)
                .map(|by| Message::Pattern(PatternMessage::Bars(by))),
        }
    }
}

/// Whether a press now is the second of a double press after the one at `last`, which then
/// forgets it, so a third press starts over.
fn double_press(last: &mut Option<Instant>) -> bool {
    let now = crate::ui::clock::now();
    let double = last.is_some_and(|when| now.duration_since(when) < DOUBLE_PRESS);
    *last = (!double).then_some(now);
    double
}

impl canvas::Program<Message> for RackHeader {
    type State = HeaderState;

    fn update(
        &self,
        state: &mut HeaderState,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let canvas::Event::Mouse(event) = event else {
            return None;
        };
        let message = match event {
            mouse::Event::CursorMoved { .. } => {
                let now = self.hit(bounds, cursor).map(Hover::RackHeader);
                return hover_change(&mut state.hovered, now);
            }
            mouse::Event::ButtonPressed(mouse::Button::Left) => {
                let hit = self.hit(bounds, cursor)?;
                let message = self.message(hit)?;
                if hit == HeaderHit::Swing && double_press(&mut state.last_swing_press) {
                    Message::Pattern(PatternMessage::SwingReset)
                } else {
                    message
                }
            }
            // Up raises the swing 1 % per scroll event, whatever its size (the window makes it
            // 0.1 % with Shift), as the tempo's wheel.
            mouse::Event::WheelScrolled { delta } => {
                if self.hit(bounds, cursor) != Some(HeaderHit::Swing) {
                    return None;
                }
                let rise = match *delta {
                    mouse::ScrollDelta::Lines { y, .. } | mouse::ScrollDelta::Pixels { y, .. } => y,
                };
                let by = if rise > 0.0 {
                    SWING_STEP
                } else if rise < 0.0 {
                    -SWING_STEP
                } else {
                    return None;
                };
                Message::Pattern(PatternMessage::SwingBy(by))
            }
            _ => return None,
        };
        Some(canvas::Action::publish(message).and_capture())
    }

    fn draw(
        &self,
        _state: &HeaderState,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), style::palette().raised);
        let middle = RACK_HEADER_HEIGHT / 2.0;
        // The panel's title, at the panel titles' size.
        frame.fill_text(Text {
            content: Panel::Rack.title().to_owned(),
            position: Point::new(RACK_TITLE_X, middle),
            color: style::palette().text,
            size: size::PANEL_TITLE.into(),
            align_y: alignment::Vertical::Center,
            ..Text::default()
        });
        header_text(
            &mut frame,
            "Bars",
            Point::new(layout::bars_label_x(), middle),
            style::palette().text_secondary,
        );
        for (rect, glyph, hit) in [
            (layout::bars_down_rect(), "\u{2212}", HeaderHit::BarsDown),
            (layout::bars_up_rect(), "+", HeaderHit::BarsUp),
        ] {
            frame.fill_rectangle(rect.position(), rect.size(), style::palette().button);
            let color = if self.nudge(hit).is_some() {
                style::palette().text
            } else {
                style::palette().text_disabled
            };
            centered_text(&mut frame, glyph, rect.center(), color);
        }
        let count = layout::bars_count_rect();
        frame.fill_rectangle(count.position(), count.size(), style::palette().lcd);
        if let Some(bars) = self.bars {
            centered_text(
                &mut frame,
                &bars.to_string(),
                count.center(),
                style::palette().lcd_text,
            );
        }
        header_text(
            &mut frame,
            "Swing",
            Point::new(layout::swing_label_x(), middle),
            style::palette().text_secondary,
        );
        let knob = layout::swing_knob_rect();
        let readout = layout::swing_count_rect();
        frame.fill_rectangle(readout.position(), readout.size(), style::palette().lcd);
        match self.swing {
            Some(swing) => {
                draw_knob(&mut frame, knob.center(), knob.width, swing_fraction(swing));
                centered_text(
                    &mut frame,
                    &swing_text(swing),
                    readout.center(),
                    style::palette().lcd_text,
                );
            }
            // No pattern to swing: a dial with no pointer over an empty readout, as the bars'.
            None => frame.fill(
                &Path::circle(knob.center(), knob.width / 2.0),
                style::palette().knob_ring,
            ),
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _state: &HeaderState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        let pressable = self.hit(bounds, cursor).and_then(|hit| self.message(hit));
        match pressable {
            Some(_) => mouse::Interaction::Pointer,
            None => mouse::Interaction::default(),
        }
    }
}

/// Header text starting at `at`, centered on it vertically.
fn header_text(frame: &mut Frame, content: &str, at: Point, color: iced::Color) {
    frame.fill_text(Text {
        content: content.to_owned(),
        position: at,
        color,
        size: size::LABEL.into(),
        align_y: alignment::Vertical::Center,
        ..Text::default()
    });
}

/// Header text centered on `center`.
fn centered_text(frame: &mut Frame, content: &str, center: Point, color: iced::Color) {
    frame.fill_text(Text {
        content: content.to_owned(),
        position: center,
        color,
        size: size::LABEL.into(),
        align_x: alignment::Horizontal::Center.into(),
        align_y: alignment::Vertical::Center,
        ..Text::default()
    });
}

/// The last press on a name button, to tell a double press, what the cursor was over, and the
/// knob held down (its row), whose tip shows until the button is let go.
#[derive(Debug, Default)]
pub struct RackState {
    last_name_press: Option<(Instant, usize)>,
    hover: Option<Hover>,
    held: Option<(usize, Knob)>,
}

/// What the hint bar names for rack hit `hit`.
pub fn hover_of(hit: RackHit) -> Hover {
    let rack = |row, part| Hover::Rack { row, part };
    match hit {
        RackHit::MuteLed { row } => rack(row, RackPart::Mute),
        RackHit::NameButton { row } => rack(row, RackPart::Name),
        RackHit::PanKnob { row } => rack(row, RackPart::Pan),
        RackHit::VolumeKnob { row } => rack(row, RackPart::Volume),
        RackHit::Step { row, step } => rack(row, RackPart::Step(step)),
    }
}

impl canvas::Program<Message> for ChannelRack<'_> {
    type State = RackState;

    fn update(
        &self,
        state: &mut RackState,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if let Some(action) = self.file_moved_or_dropped(event, bounds, cursor) {
            return Some(action);
        }
        // While a file is held over the rows, the row it would land on is what counts.
        if let canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) = event {
            if self.browser_drag {
                return None;
            }
            let now = if self.on_add(bounds, cursor) {
                Some(Hover::RackAdd)
            } else {
                self.hit(bounds, cursor).map(hover_of)
            };
            return hover_change(&mut state.hover, now);
        }
        if let canvas::Event::Mouse(mouse::Event::ButtonReleased(_)) = event {
            // A single press on a name only selects: the window opens on a double press
            // (a click that opened a window got in the way).
            state.held = None;
            return None;
        }
        let canvas::Event::Mouse(mouse::Event::ButtonPressed(button)) = event else {
            return None;
        };
        state.held = None;
        if self.on_add(bounds, cursor) && *button == mouse::Button::Left {
            let open = Message::OpenPicker(PickerTarget::AddInstrument);
            return Some(canvas::Action::publish(open).and_capture());
        }
        let hit = self.hit(bounds, cursor)?;
        // The Recordings header: a left press anywhere on it opens or folds the takes.
        let row = hit.row();
        if self.rows.channel(row).is_none() {
            let header = matches!(self.rows.get(row), Some(RackRow::Takes { .. }));
            if header && *button == mouse::Button::Left {
                return Some(canvas::Action::publish(Message::ToggleTakes).and_capture());
            }
            return None;
        }
        // Right on a name button: the channel's menu, every kind's (its instrument window,
        // rename, clone, delete; a plugin channel's adds its plugin's window and its parameters:
        // before, its parameters had opened at once, uninvited); on a knob, the
        // automation menu.
        if *button == mouse::Button::Right {
            let at = cursor.position_in(bounds)?;
            let message = match hit {
                RackHit::NameButton { row } => {
                    let channel = self.channel_at(row)?;
                    Message::Media(MediaMessage::SamplerMenu { channel, at })
                }
                RackHit::PanKnob { row } => Message::KnobMenu {
                    channel: self.channel_at(row)?,
                    knob: Knob::Pan,
                    at,
                },
                RackHit::VolumeKnob { row } => Message::KnobMenu {
                    channel: self.channel_at(row)?,
                    knob: Knob::Volume,
                    at,
                },
                _ => return None,
            };
            return Some(canvas::Action::publish(message).and_capture());
        }
        if *button != mouse::Button::Left {
            return None;
        }
        let knob_pressed = |row, knob| {
            let channel = self.channel_at(row)?;
            Some(Message::KnobPressed { channel, knob })
        };
        let message = match hit {
            RackHit::MuteLed { row } => Message::MuteToggled(self.channel_at(row)?),
            RackHit::NameButton { row } => {
                let now = crate::ui::clock::now();
                let double = state.last_name_press.is_some_and(|(when, last)| {
                    last == row && now.duration_since(when) < DOUBLE_PRESS
                });
                state.last_name_press = (!double).then_some((now, row));
                let channel = self.channel_at(row)?;
                let silent = self
                    .sampler(channel)
                    .is_some_and(|sampler| sampler.sample.as_os_str().is_empty());
                match (double, silent) {
                    (true, true) => Message::Media(MediaMessage::ChangeSample(channel)),
                    (true, false) => Message::ChannelOpened(channel),
                    (false, _) => Message::ChannelSelected(channel),
                }
            }
            RackHit::PanKnob { row } => {
                state.held = Some((row, Knob::Pan));
                knob_pressed(row, Knob::Pan)?
            }
            RackHit::VolumeKnob { row } => {
                state.held = Some((row, Knob::Volume));
                knob_pressed(row, Knob::Volume)?
            }
            RackHit::Step { row, step } => Message::StepClicked {
                channel: self.channel_at(row)?,
                step,
            },
        };
        Some(canvas::Action::publish(message).and_capture())
    }

    fn draw(
        &self,
        state: &RackState,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        self.draw_with_steps(state, renderer, bounds, 0.0)
    }

    fn mouse_interaction(
        &self,
        _state: &RackState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if self.hit(bounds, cursor).is_some() || self.on_add(bounds, cursor) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}

/// The width of a mute light's ring while it is off.
const MUTE_RING: f32 = 1.5;

/// A mute light (the rack's, a track's, a mixer strip's): heard, a dot `radius` round in `lit`;
/// off (`None`), a ring in the palette's strong line, its outer edge where the dot's is and the
/// ground showing through. Why a ring and not a darker dot: an unlit dot on the rack's well was
/// 1.25:1, so a muted channel barely showed.
pub(crate) fn draw_mute_light(
    frame: &mut Frame,
    center: Point,
    radius: f32,
    lit: Option<iced::Color>,
) {
    match lit {
        Some(color) => frame.fill(&Path::circle(center, radius), color),
        None => frame.stroke(
            &Path::circle(center, radius - MUTE_RING / 2.0),
            Stroke::default()
                .with_color(style::palette().line_strong)
                .with_width(MUTE_RING),
        ),
    }
}

/// The soft line between two beat groups of `row`'s steps, before step `step` (the first of a
/// group): 2 px down the middle of the gap, as tall as a step, in the palette's soft line. Why:
/// the unlit steps' two tones are a shade down so every channel's lit step stands 3:1 off both,
/// which leaves the tones 1.16:1 apart; the line keeps the beat groups apart (with the
/// soft line, not a lighter tone).
fn draw_beat_line(frame: &mut Frame, row: usize, step: u32) {
    let before = layout::step_rect(row, step - 1);
    let after = layout::step_rect(row, step);
    let middle = (before.x + before.width + after.x) / 2.0;
    frame.fill_rectangle(
        Point::new(middle - BEAT_LINE / 2.0, after.y),
        Size::new(BEAT_LINE, after.height),
        style::palette().line_soft,
    );
}

/// The beat line's width.
const BEAT_LINE: f32 = 2.0;

fn draw_name_button(frame: &mut Frame, row: usize, name: &str, selected: bool) {
    let rect = layout::name_button_rect(row);
    frame.fill_rectangle(rect.position(), rect.size(), style::palette().name_button);
    if selected {
        let width = 1.5;
        // Inset by half the line width so the outline stays inside the button.
        let inset = width / 2.0;
        frame.stroke_rectangle(
            Point::new(rect.x + inset, rect.y + inset),
            Size::new(rect.width - width, rect.height - width),
            Stroke::default()
                .with_color(style::accent())
                .with_width(width),
        );
    }
    frame.fill_text(Text {
        content: shown_name(name),
        position: Point::new(rect.x + NAME_PADDING, rect.center_y()),
        color: style::palette().text,
        size: size::BODY.into(),
        align_y: alignment::Vertical::Center,
        ..Text::default()
    });
}

/// The Recordings header's row: a triangle (right while folded, down while open) and
/// "Recordings (N)" on the name button's ground, nothing in the light's, knobs' or steps' place.
fn draw_takes_header(frame: &mut Frame, row: usize, count: usize, open: bool) {
    let rect = layout::name_button_rect(row);
    frame.fill_rectangle(rect.position(), rect.size(), style::palette().name_button);
    let arrow = if open { "\u{25be}" } else { "\u{25b8}" };
    frame.fill_text(Text {
        content: format!("{arrow} {TAKES_HEADER} ({count})"),
        position: Point::new(rect.x + NAME_PADDING, rect.center_y()),
        color: style::palette().text_secondary,
        size: size::BODY.into(),
        align_y: alignment::Vertical::Center,
        ..Text::default()
    });
}

/// The header the audio takes fold under.
pub const TAKES_HEADER: &str = "Recordings";

/// A channel's name as its button shows it: whole when it fits, else cut with an ellipsis (the
/// hint bar names it whole).
fn shown_name(name: &str) -> String {
    let room = layout::NAME_BUTTON_WIDTH - 2.0 * NAME_PADDING;
    fit_width(name, room, size::BODY).map_or_else(String::new, std::borrow::Cow::into_owned)
}

/// A dark dial `size` px across with an accent pointer; `fraction` 0 points down-left, 0.5 up,
/// 1 down-right.
pub(crate) fn draw_knob(frame: &mut Frame, center: Point, size: f32, fraction: f32) {
    let radius = size / 2.0;
    frame.fill(&Path::circle(center, radius), style::palette().knob_ring);
    frame.fill(
        &Path::circle(center, radius - 2.0),
        style::palette().knob_face,
    );
    draw_pointer(frame, center, radius, fraction);
}

/// The accent pointer of a knob `radius` px round at `center`, pointing as [`draw_knob`]'s.
pub(crate) fn draw_pointer(frame: &mut Frame, center: Point, radius: f32, fraction: f32) {
    let angle = knob_angle(fraction);
    let along = |distance: f32| {
        Point::new(
            center.x + distance * angle.sin(),
            center.y - distance * angle.cos(),
        )
    };
    frame.stroke(
        &Path::line(along(radius * 0.3), along(radius - 4.0)),
        Stroke::default()
            .with_color(style::accent())
            .with_width(2.5)
            .with_line_cap(LineCap::Round),
    );
}

/// The 4 px dot at the top right of an automated control of `size` centered at `center`.
pub(crate) fn draw_automated_dot(frame: &mut Frame, center: Point, size: f32) {
    let offset = size / 2.0 - AUTOMATED_DOT / 2.0;
    frame.fill(
        &Path::circle(
            Point::new(center.x + offset, center.y - offset),
            AUTOMATED_DOT / 2.0,
        ),
        style::accent(),
    );
}

/// The 2 px dot just right of the top right corner of a control of `size` centered at `center`
/// that a MIDI controller moves.
pub(crate) fn draw_linked_dot(frame: &mut Frame, center: Point, size: f32) {
    frame.fill_rectangle(
        Point::new(
            center.x + size / 2.0 + LINKED_DOT_GAP,
            center.y - size / 2.0,
        ),
        Size::new(LINKED_DOT, LINKED_DOT),
        style::accent(),
    );
}

/// Pointer angle in radians, clockwise from straight up.
fn knob_angle(fraction: f32) -> f32 {
    (fraction.clamp(0.0, 1.0) - 0.5) * KNOB_SWEEP
}

/// Over the rack's rows while a MIDI device from the Library is held: says which row it is over
/// when that changes (a channel that plays notes; an audio channel's row is none), and outlines
/// that row, where letting go links the device to it. It takes no event, so the rows under it
/// keep theirs.
pub struct DeviceDrop<'a> {
    pub project: &'a Project,
    /// The row the window last heard the device is over.
    pub over: Option<ChannelId>,
    /// The rack's rows, as it draws them.
    pub rows: RackRows,
}

impl DeviceDrop<'_> {
    /// The channel that plays notes on the row under the cursor, and that row.
    fn under(&self, bounds: Rectangle, cursor: mouse::Cursor) -> Option<(usize, ChannelId)> {
        let point = cursor.position_in(bounds)?;
        let row = layout::row_at(point.y, self.rows.len())?;
        let id = self.rows.channel(row)?;
        let channel = self.project.channels.get(&id)?;
        channel.takes_midi_input().then_some((row, id))
    }
}

impl canvas::Program<Message> for DeviceDrop<'_> {
    type State = ();

    fn update(
        &self,
        _state: &mut (),
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) = event else {
            return None;
        };
        let over = self.under(bounds, cursor).map(|(_, id)| id);
        (over != self.over)
            .then(|| canvas::Action::publish(Message::Media(MediaMessage::OverChannel(over))))
    }

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let row = self.over.and_then(|over| self.rows.row_of(over));
        if let Some(row) = row {
            let top = layout::row_top(row);
            frame.stroke_rectangle(
                Point::new(2.0, top - 1.0),
                Size::new(bounds.width - 4.0, layout::ROW_HEIGHT),
                Stroke::default()
                    .with_color(style::accent())
                    .with_width(1.5),
            );
        }
        vec![frame.into_geometry()]
    }
}

/// Over the rack's rows while a sound, a set or a plugin instrument from the Library is held:
/// says which channel's header (its light, name and knobs, left of its steps) it is over when
/// that changes, and outlines that header, where letting go gives the channel the sound held.
/// An audio channel's header is none: its clips are measured in its file's frames. It takes no
/// event, so the rows under it keep theirs.
pub struct SoundDrop<'a> {
    pub project: &'a Project,
    /// The channel whose header the window last heard the sound is over.
    pub over: Option<ChannelId>,
    /// The rack's rows, as it draws them.
    pub rows: RackRows,
}

impl SoundDrop<'_> {
    /// The channel whose header is under the cursor, when it takes another sound.
    fn under(&self, bounds: Rectangle, cursor: mouse::Cursor) -> Option<ChannelId> {
        let point = cursor.position_in(bounds)?;
        let row = layout::channel_header_at(point, self.rows.len())?;
        let id = self.rows.channel(row)?;
        let channel = self.project.channels.get(&id)?;
        (channel.kind != ChannelKind::Audio).then_some(id)
    }
}

/// The outline's width around a header a sound would land on.
const DROP_OUTLINE: f32 = 2.0;

impl canvas::Program<Message> for SoundDrop<'_> {
    type State = ();

    fn update(
        &self,
        _state: &mut (),
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) = event else {
            return None;
        };
        let over = self.under(bounds, cursor);
        (over != self.over)
            .then(|| canvas::Action::publish(Message::Media(MediaMessage::OverHeader(over))))
    }

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let row = self.over.and_then(|over| self.rows.row_of(over));
        if let Some(row) = row {
            let rect = layout::channel_header_rect(row);
            let inset = DROP_OUTLINE / 2.0;
            frame.stroke_rectangle(
                Point::new(rect.x + inset, rect.y + inset),
                Size::new(rect.width - DROP_OUTLINE, rect.height - DROP_OUTLINE),
                Stroke::default()
                    .with_color(style::accent())
                    .with_width(DROP_OUTLINE),
            );
        }
        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use iced::widget::canvas::Program as _;

    use super::*;

    #[test]
    fn knob_fractions_span_each_range() {
        assert_eq!(Knob::Volume.fraction(0.0), 0.0);
        assert_eq!(Knob::Volume.fraction(1.0), 0.25);
        assert_eq!(Knob::Volume.fraction(MAX_VOLUME), 1.0);
        assert_eq!(Knob::Pan.fraction(-1.0), 0.0);
        assert_eq!(Knob::Pan.fraction(0.0), 0.5);
        assert_eq!(Knob::Pan.fraction(1.0), 1.0);
    }

    #[test]
    fn knobs_sweep_270_degrees_centered_on_straight_up() {
        let degrees = |fraction: f32| knob_angle(fraction).to_degrees();
        assert!((degrees(0.0) + 135.0).abs() < 1e-4);
        assert!(degrees(0.5).abs() < 1e-4);
        assert!((degrees(1.0) - 135.0).abs() < 1e-4);
        // Out-of-range values pin to the ends.
        assert!((degrees(-1.0) + 135.0).abs() < 1e-4);
        assert!((degrees(2.0) - 135.0).abs() < 1e-4);
    }

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    fn fixture(name: &str) -> Project {
        let text =
            std::fs::read_to_string(std::path::Path::new(FIXTURES).join(name)).expect("reads");
        Project::load_json(&text).expect("loads")
    }

    /// What the rack of `project` (no browser drag, unless `drag` gives the sampler row the
    /// window last heard of) does with `event` at `at`, rack coordinates: the message it sends
    /// and whether it takes the event.
    fn rack_event(
        project: &Project,
        state: &mut RackState,
        drag: Option<Option<ChannelId>>,
        event: canvas::Event,
        at: Point,
    ) -> Option<(Message, bool)> {
        let automated = BTreeMap::new();
        let cache = Cache::new();
        let rack = ChannelRack {
            project,
            pattern: project.patterns.keys().next().copied(),
            selected: None,
            playhead: None,
            automated: &automated,
            cache: &cache,
            browser_drag: drag.is_some(),
            drop_sampler: drag.flatten(),
            rows: RackRows::of(project, false),
        };
        let size = layout::rack_size(project.channels.len(), 16);
        let bounds = Rectangle::new(Point::ORIGIN, size);
        let action = rack.update(state, &event, bounds, mouse::Cursor::Available(at))?;
        let (message, _, status) = action.into_inner();
        Some((message?, status == iced::event::Status::Captured))
    }

    // "+ Instrument…" under the rows (p3_roll's two channels: its words and 8 px either side,
    // 28..126.395 by the add row's 84..108, layout's test): a left press on it opens the
    // instrument picker and takes the press; the pointer over it names it in the hint bar; a
    // right press, or a press right of it in its row (140, 96), does nothing. (It stood in the
    // header, then was a button 28..128 by 86..106.)
    #[test]
    fn the_add_words_under_the_rows_open_the_instrument_picker() {
        let project = fixture("p3_roll.json");
        let mut state = RackState::default();
        let add = layout::add_instrument_rect(2);
        assert_eq!((add.x, add.y, add.height), (28.0, 84.0, 24.0));
        let at = Point::new(60.0, 96.0);
        assert!(matches!(
            rack_event(&project, &mut state, None, press(mouse::Button::Left), at),
            Some((Message::OpenPicker(PickerTarget::AddInstrument), true))
        ));
        let moved = canvas::Event::Mouse(mouse::Event::CursorMoved { position: at });
        assert!(matches!(
            rack_event(&project, &mut state, None, moved, at),
            Some((
                Message::Hover(crate::ui::hint::HoverMessage::Over(Hover::RackAdd)),
                false
            ))
        ));
        assert!(rack_event(&project, &mut state, None, press(mouse::Button::Right), at).is_none());
        let beside = Point::new(140.0, 96.0);
        assert!(rack_event(
            &project,
            &mut state,
            None,
            press(mouse::Button::Left),
            beside
        )
        .is_none());
    }

    // Recording v2: the audio takes fold under one header, Recordings, after the channels.
    // p17_audio_song_after (channel 1 "Kick", takes 1 and 2) folded: two rows, the header's;
    // a left press on the header (its name or anywhere along it) opens or folds them, a right
    // press does nothing. Opened, the takes are rows under it, a press on one selects it; the
    // canvas holds as many rows as are shown.
    #[test]
    fn audio_takes_fold_under_the_recordings_header() {
        let project = fixture("p17_audio_song_after.json");
        let automated = BTreeMap::new();
        let cache = Cache::new();
        let rack = |open: bool| ChannelRack {
            project: &project,
            pattern: Some(dawsome_engine::project::PatternId(1)),
            selected: None,
            playhead: None,
            automated: &automated,
            cache: &cache,
            browser_drag: false,
            drop_sampler: None,
            rows: RackRows::of(&project, open),
        };
        let pressed = |open: bool, at: Point, button| {
            let rack = rack(open);
            let size = layout::rack_size(rack.rows.len(), 16);
            let bounds = Rectangle::new(Point::ORIGIN, size);
            let action = rack.update(
                &mut RackState::default(),
                &press(button),
                bounds,
                mouse::Cursor::Available(at),
            )?;
            action.into_inner().0
        };
        assert_eq!(rack(false).rows.len(), 2);
        let header = layout::name_button_rect(1).center();
        assert!(matches!(
            pressed(false, header, mouse::Button::Left),
            Some(Message::ToggleTakes)
        ));
        let along = layout::step_rect(1, 3).center();
        assert!(matches!(
            pressed(false, along, mouse::Button::Left),
            Some(Message::ToggleTakes)
        ));
        assert!(pressed(false, header, mouse::Button::Right).is_none());
        assert_eq!(rack(true).rows.len(), 4);
        assert!(matches!(
            pressed(
                true,
                layout::name_button_rect(2).center(),
                mouse::Button::Left
            ),
            Some(Message::ChannelSelected(ChannelId(2)))
        ));
    }

    fn press(button: mouse::Button) -> canvas::Event {
        canvas::Event::Mouse(mouse::Event::ButtonPressed(button))
    }

    // A second press on a sampler's name asks for a sound file while it has none (the new
    // document's "Sampler"); with one (F1's kick) it opens the channel as before.
    #[test]
    fn a_double_press_on_a_silent_samplers_name_asks_for_a_file() {
        let name = layout::name_button_rect(0).center();
        let twice = |project: &Project| {
            let mut state = RackState::default();
            let first = rack_event(project, &mut state, None, press(mouse::Button::Left), name);
            assert!(matches!(
                first,
                Some((Message::ChannelSelected(ChannelId(1)), true))
            ));
            rack_event(project, &mut state, None, press(mouse::Button::Left), name)
        };
        assert!(matches!(
            twice(&crate::project_file::new_document()),
            Some((
                Message::Media(MediaMessage::ChangeSample(ChannelId(1))),
                true
            ))
        ));
        assert!(matches!(
            twice(&fixture("f1_four_on_floor.json")),
            Some((Message::ChannelOpened(ChannelId(1)), true))
        ));
    }

    // A press on a name selects its channel; the release on it, on its own, does nothing
    // (a click opens no window), nor does a release off the name;
    // a second press within the double-press time opens the channel's window, and its release
    // is nothing either.
    #[test]
    fn a_press_on_a_name_selects_and_a_double_press_opens_its_window() {
        let project = fixture("f1_four_on_floor.json");
        let name = layout::name_button_rect(0).center();
        let released = canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left));
        let mut state = RackState::default();
        let pressed = rack_event(&project, &mut state, None, press(mouse::Button::Left), name);
        assert!(matches!(
            pressed,
            Some((Message::ChannelSelected(ChannelId(1)), true))
        ));
        assert!(
            rack_event(&project, &mut state, None, released.clone(), name).is_none(),
            "a release on the name is no click"
        );
        let mut state = RackState::default();
        let _ = rack_event(&project, &mut state, None, press(mouse::Button::Left), name);
        let off = Point::new(500.0, 300.0);
        assert!(rack_event(&project, &mut state, None, released.clone(), off).is_none());
        let mut state = RackState::default();
        let _ = rack_event(&project, &mut state, None, press(mouse::Button::Left), name);
        let _ = rack_event(&project, &mut state, None, released.clone(), name);
        let second = rack_event(&project, &mut state, None, press(mouse::Button::Left), name);
        assert!(matches!(
            second,
            Some((Message::ChannelOpened(ChannelId(1)), true))
        ));
        assert!(rack_event(&project, &mut state, None, released, name).is_none());
    }

    // By hand on p3_roll's one bar of steps (they reach 508 px past x 222) in a rack 400 px
    // wide (178 px of steps in view), scrolled 100 px: a press at x 236 lands on step 3 (its left
    // edge 312 is 212 after the scroll), the name stays where it is drawn; Shift+wheel down over
    // the steps asks for an eighth of the view later; a wheel over the knobs, and a plain wheel,
    // are the rows' (the vertical scroll's); a new width is said once.
    #[test]
    fn the_steps_scroll_under_the_fixed_name_and_knobs() {
        let project = fixture("p3_roll.json");
        let automated = BTreeMap::new();
        let cache = Cache::new();
        let rows = |shift| ScrolledRack {
            rack: ChannelRack {
                project: &project,
                pattern: project.patterns.keys().next().copied(),
                selected: None,
                playhead: None,
                automated: &automated,
                cache: &cache,
                browser_drag: false,
                drop_sampler: None,
                rows: RackRows::of(&project, false),
            },
            steps: Span {
                content: 508.0,
                offset: 100.0,
                view: 178.0,
            },
            known_width: 400.0,
            shift,
        };
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(400.0, 200.0));
        let event = |rows: &ScrolledRack, event, at| {
            let cursor = mouse::Cursor::Available(at);
            let action = rows.update(&mut RackState::default(), &event, bounds, cursor)?;
            action.into_inner().0
        };
        let step = layout::step_rect(0, 3).center();
        assert_eq!(step.x, 326.0);
        let pressed = event(
            &rows(false),
            press(mouse::Button::Left),
            Point::new(236.0, step.y),
        );
        assert!(
            matches!(pressed, Some(Message::StepClicked { step: 3, .. })),
            "{pressed:?}"
        );
        let name = layout::name_button_rect(1).center();
        let pressed = event(&rows(false), press(mouse::Button::Left), name);
        assert!(matches!(
            pressed,
            Some(Message::ChannelSelected(ChannelId(2)))
        ));
        let wheel = canvas::Event::Mouse(mouse::Event::WheelScrolled {
            delta: mouse::ScrollDelta::Lines { x: 0.0, y: -1.0 },
        });
        let over_steps = Point::new(300.0, step.y);
        assert!(matches!(
            event(&rows(true), wheel.clone(), over_steps),
            Some(Message::RackBar(BarMessage::To { offset, held: false })) if offset == 100.0 + 178.0 / 8.0
        ));
        assert!(
            event(&rows(false), wheel.clone(), over_steps).is_none(),
            "the rows'"
        );
        let over_knobs = layout::knob_rect(VOLUME_KNOB_X, 0).center();
        assert!(event(&rows(true), wheel, over_knobs).is_none(), "the rows'");
        let resized = ScrolledRack {
            known_width: 300.0,
            ..rows(false)
        };
        let moved = canvas::Event::Mouse(mouse::Event::CursorMoved {
            position: Point::new(5.0, 300.0),
        });
        assert!(matches!(
            event(&resized, moved, Point::new(5.0, 300.0)),
            Some(Message::RackResized(400.0))
        ));
    }

    // Over the steps, a trackpad's swipe across
    // with a little down in it (3 px left, 0.4 px down) scrolls them an eighth of the view later,
    // as a swipe with none does; a swipe mostly up or down is the rows'. The case: p3_roll's
    // 508 px of steps, 178 px in view, scrolled 100 px.
    #[test]
    fn a_swipe_mostly_across_scrolls_the_steps() {
        let project = fixture("p3_roll.json");
        let automated = BTreeMap::new();
        let cache = Cache::new();
        let rows = ScrolledRack {
            rack: ChannelRack {
                project: &project,
                pattern: project.patterns.keys().next().copied(),
                selected: None,
                playhead: None,
                automated: &automated,
                cache: &cache,
                browser_drag: false,
                drop_sampler: None,
                rows: RackRows::of(&project, false),
            },
            steps: Span {
                content: 508.0,
                offset: 100.0,
                view: 178.0,
            },
            known_width: 400.0,
            shift: false,
        };
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(400.0, 200.0));
        let over_steps = Point::new(300.0, layout::step_rect(0, 3).center().y);
        let swipe = |x, y| {
            let event = canvas::Event::Mouse(mouse::Event::WheelScrolled {
                delta: mouse::ScrollDelta::Pixels { x, y },
            });
            let cursor = mouse::Cursor::Available(over_steps);
            let action = rows.update(&mut RackState::default(), &event, bounds, cursor)?;
            action.into_inner().0
        };
        assert!(matches!(
            swipe(-3.0, 0.4),
            Some(Message::RackBar(BarMessage::To { offset, held: false })) if offset == 100.0 + 178.0 / 8.0
        ));
        assert!(swipe(0.4, -3.0).is_none(), "the rows'");
    }

    // A right press on a sampler's name opens its menu where the press was; on a plugin
    // channel's name too (it once opened the plugin's parameters at once), and on an
    // audio channel's (p8_ui's channel 1), where it did nothing.
    #[test]
    fn a_right_press_on_a_samplers_name_opens_its_menu() {
        let name = layout::name_button_rect(0).center();
        let right = |project: &Project| {
            rack_event(
                project,
                &mut RackState::default(),
                None,
                press(mouse::Button::Right),
                name,
            )
        };
        let menu = right(&fixture("f1_four_on_floor.json"));
        assert!(
            matches!(
                menu,
                Some((Message::Media(MediaMessage::SamplerMenu { channel: ChannelId(1), at }), true))
                    if at == name
            ),
            "{menu:?}"
        );
        for plugin_or_audio in ["p7_sine.json", "p8_ui.json"] {
            let menu = right(&fixture(plugin_or_audio));
            assert!(
                matches!(
                    menu,
                    Some((Message::Media(MediaMessage::SamplerMenu { channel: ChannelId(1), at }), true))
                        if at == name
                ),
                "{plugin_or_audio}: {menu:?}"
            );
        }
    }

    // p8_ui's rows: channel 1 plays audio clips, channel 2 is a sampler. A file from the desktop
    // dropped anywhere along row 2 (here over its steps) is that sampler's and the rack takes
    // the drop; on row 1 the rack leaves it to the window.
    #[test]
    fn a_desktop_file_dropped_on_a_sampler_row_is_that_samplers() {
        let project = fixture("p8_ui.json");
        let file = std::path::PathBuf::from("/sounds/kick.wav");
        let drop = |at| {
            let event = canvas::Event::Window(window::Event::FileDropped(file.clone()));
            rack_event(&project, &mut RackState::default(), None, event, at)
        };
        let row_2 = layout::step_rect(1, 3).center();
        let dropped = drop(row_2);
        assert!(
            matches!(
                &dropped,
                Some((Message::Media(MediaMessage::DroppedOnSampler { channel: ChannelId(2), file: f }), true))
                    if *f == file
            ),
            "{dropped:?}"
        );
        assert!(drop(layout::step_rect(0, 3).center()).is_none());
    }

    // A MIDI device held from the Library: over p8_ui's sampler row (channel 2) the rack says
    // so once, and over the audio row (channel 1, whose clips play no notes) that it is over
    // none.
    #[test]
    fn a_device_held_over_the_rows_names_the_channel_that_plays_notes() {
        let project = fixture("p8_ui.json");
        let size = layout::rack_size(project.channels.len(), 16);
        let moved = |heard, row| {
            let target = DeviceDrop {
                project: &project,
                over: heard,
                rows: RackRows::of(&project, false),
            };
            let at = layout::name_button_rect(row).center();
            let event = canvas::Event::Mouse(mouse::Event::CursorMoved { position: at });
            let bounds = Rectangle::new(Point::ORIGIN, size);
            let action = target.update(&mut (), &event, bounds, mouse::Cursor::Available(at))?;
            action.into_inner().0
        };
        assert!(matches!(
            moved(None, 1),
            Some(Message::Media(MediaMessage::OverChannel(Some(ChannelId(
                2
            )))))
        ));
        assert!(moved(Some(ChannelId(2)), 1).is_none(), "said once");
        assert!(matches!(
            moved(Some(ChannelId(2)), 0),
            Some(Message::Media(MediaMessage::OverChannel(None)))
        ));
    }

    // A sound held from the Library over p8_ui's rows: over the sampler's header (row 2,
    // channel 2) the target says so once; over the audio row's header (channel 1) and between
    // rows it is over none.
    #[test]
    fn a_sound_held_over_a_header_names_its_channel() {
        let project = fixture("p8_ui.json");
        let size = layout::rack_size(project.channels.len(), 16);
        let moved = |heard, at: Point| {
            let target = SoundDrop {
                project: &project,
                over: heard,
                rows: RackRows::of(&project, false),
            };
            let event = canvas::Event::Mouse(mouse::Event::CursorMoved { position: at });
            let bounds = Rectangle::new(Point::ORIGIN, size);
            let action = target.update(&mut (), &event, bounds, mouse::Cursor::Available(at))?;
            action.into_inner().0
        };
        let knob = layout::knob_rect(PAN_KNOB_X, 1).center();
        assert!(matches!(
            moved(None, knob),
            Some(Message::Media(MediaMessage::OverHeader(Some(ChannelId(2)))))
        ));
        assert!(moved(Some(ChannelId(2)), knob).is_none(), "said once");
        let audio = layout::name_button_rect(0).center();
        assert!(matches!(
            moved(Some(ChannelId(2)), audio),
            Some(Message::Media(MediaMessage::OverHeader(None)))
        ));
        let between = Point::new(80.0, layout::row_top(1) - 4.0);
        assert!(moved(None, between).is_none(), "none, as before");
    }

    // A file held from the browser: the rack says which sampler row it is over when that
    // changes, and that it left the samplers when it moves onto the audio row.
    #[test]
    fn a_file_held_over_the_rows_names_the_sampler_under_it() {
        let project = fixture("p8_ui.json");
        let moved = |heard, at| {
            let event = canvas::Event::Mouse(mouse::Event::CursorMoved { position: at });
            rack_event(&project, &mut RackState::default(), Some(heard), event, at)
        };
        let row_2 = layout::name_button_rect(1).center();
        assert!(matches!(
            moved(None, row_2),
            Some((
                Message::Media(MediaMessage::OverSampler(Some(ChannelId(2)))),
                false
            ))
        ));
        assert!(moved(Some(ChannelId(2)), row_2).is_none(), "said once");
        assert!(matches!(
            moved(Some(ChannelId(2)), layout::name_button_rect(0).center()),
            Some((Message::Media(MediaMessage::OverSampler(None)), false))
        ));
    }

    // A cursor coming onto a rack control tells the window what it is over (for the hint bar),
    // once; moving off every control tells it the cursor left.
    #[test]
    fn the_rack_says_what_the_cursor_is_over() {
        use crate::ui::hint::{Hover, HoverMessage, RackPart};
        let project = fixture("f1_four_on_floor.json");
        let mut state = RackState::default();
        let moved = canvas::Event::Mouse(mouse::Event::CursorMoved {
            position: Point::ORIGIN,
        });
        let knob = layout::knob_rect(VOLUME_KNOB_X, 0).center();
        let volume = Hover::Rack {
            row: 0,
            part: RackPart::Volume,
        };
        assert!(matches!(
            rack_event(&project, &mut state, None, moved.clone(), knob),
            Some((Message::Hover(HoverMessage::Over(hover)), false)) if hover == volume
        ));
        assert!(
            rack_event(&project, &mut state, None, moved.clone(), knob).is_none(),
            "said once"
        );
        let empty = Point::new(5.0, 300.0);
        assert!(matches!(
            rack_event(&project, &mut state, None, moved, empty),
            Some((Message::Hover(HoverMessage::Left(hover)), false)) if hover == volume
        ));
    }

    /// The value tip of `knob` in `row` of `project`'s rack, nothing automated.
    fn knob_tip(project: &Project, row: usize, knob: Knob) -> Option<(Rectangle, String)> {
        let automated = BTreeMap::new();
        let cache = Cache::new();
        let rack = ChannelRack {
            project,
            pattern: project.patterns.keys().next().copied(),
            selected: None,
            playhead: None,
            automated: &automated,
            cache: &cache,
            browser_drag: false,
            drop_sampler: None,
            rows: RackRows::of(project, false),
        };
        rack.knob_tip(row, knob)
    }

    // A knob says its value in a tip as the mixer's controls do: p3_roll's lead at 100% and
    // centered. The volume knob of row 1 is centered at (196, 60): the 52 x 16 tip stands 2 px
    // above it, at (170, 28); in row 0 there is no room above the knob (center y 22), so the tip
    // stands 2 px under it, at y 38.
    #[test]
    fn a_knob_says_its_value_in_a_tip_over_it() {
        let project = fixture("p3_roll.json");
        let (rect, value) = knob_tip(&project, 1, Knob::Volume).expect("row 1's knob");
        assert_eq!(
            (rect.x, rect.y, rect.width, rect.height),
            (170.0, 28.0, 52.0, 16.0)
        );
        assert_eq!(value, "100%");
        let (rect, value) = knob_tip(&project, 0, Knob::Pan).expect("row 0's knob");
        assert_eq!((rect.x, rect.y), (136.0, 38.0));
        assert_eq!(value, "Center");
        assert!(knob_tip(&project, 2, Knob::Pan).is_none(), "no row 2");
        assert_eq!(Knob::Pan.text(-0.5), "50% L");
    }

    // The tip follows a knob held down until the button is let go, wherever the cursor goes.
    #[test]
    fn a_knob_held_down_keeps_its_tip_until_let_go() {
        let project = fixture("f1_four_on_floor.json");
        let mut state = RackState::default();
        let knob = layout::knob_rect(VOLUME_KNOB_X, 0).center();
        let pressed = rack_event(&project, &mut state, None, press(mouse::Button::Left), knob);
        assert!(matches!(
            pressed,
            Some((
                Message::KnobPressed {
                    knob: Knob::Volume,
                    ..
                },
                true
            ))
        ));
        assert_eq!(state.held, Some((0, Knob::Volume)));
        let released = canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left));
        let _ = rack_event(
            &project,
            &mut state,
            None,
            released,
            Point::new(500.0, 300.0),
        );
        assert_eq!(state.held, None);
    }

    /// `event` at `at` (header coordinates) on a header whose pattern swings by `swing`: the
    /// message it publishes and whether it took the event.
    fn header_event(
        swing: Option<u16>,
        state: &mut HeaderState,
        event: canvas::Event,
        at: Point,
    ) -> Option<(Message, bool)> {
        let header = RackHeader {
            bars: swing.map(|_| 1),
            shorter: false,
            longer: true,
            swing,
        };
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(1000.0, RACK_HEADER_HEIGHT));
        let action = header.update(state, &event, bounds, mouse::Cursor::Available(at))?;
        let (message, _, status) = action.into_inner();
        Some((message?, status == iced::event::Status::Captured))
    }

    fn wheel(y: f32) -> canvas::Event {
        canvas::Event::Mouse(mouse::Event::WheelScrolled {
            delta: mouse::ScrollDelta::Lines { x: 0.0, y },
        })
    }

    // Swing: a press on the knob or its readout starts a drag; a second
    // within the double-press time (400 ms) sets it straight, and the press after that starts
    // over; the wheel up is +1 % (10 tenths), down −1 %, published for the window, which applies
    // Shift. The swing's wheel is the swing's only.
    #[test]
    fn the_swing_takes_a_press_a_double_press_and_the_wheel() {
        let knob = layout::swing_knob_rect().center();
        let readout = layout::swing_count_rect().center();
        let left = press(mouse::Button::Left);
        let start = Instant::now();
        let at = |ms| crate::ui::clock::hold(Some(start + Duration::from_millis(ms)));
        let mut state = HeaderState::default();
        let mut send = |event: &canvas::Event, point| {
            header_event(Some(667), &mut state, event.clone(), point)
        };
        at(0);
        assert!(matches!(
            send(&left, knob),
            Some((Message::SwingPressed, true))
        ));
        at(399);
        assert!(matches!(
            send(&left, readout),
            Some((Message::Pattern(PatternMessage::SwingReset), true))
        ));
        at(500);
        assert!(matches!(
            send(&left, knob),
            Some((Message::SwingPressed, true))
        ));
        at(900);
        assert!(
            matches!(send(&left, knob), Some((Message::SwingPressed, true))),
            "400 ms later is a press of its own"
        );
        crate::ui::clock::hold(None);
        assert!(matches!(
            send(&wheel(1.0), knob),
            Some((Message::Pattern(PatternMessage::SwingBy(10)), true))
        ));
        assert!(matches!(
            send(&wheel(-0.5), readout),
            Some((Message::Pattern(PatternMessage::SwingBy(-10)), true))
        ));
        assert!(send(&wheel(0.0), knob).is_none(), "no rise");
        let bars_up = layout::bars_up_rect().center();
        assert!(send(&wheel(1.0), bars_up).is_none(), "the wheel elsewhere");
        let moved = canvas::Event::Mouse(mouse::Event::CursorMoved { position: knob });
        let _ = send(&moved, knob);
        assert_eq!(state.hovered, Some(Hover::RackHeader(HeaderHit::Swing)));
    }

    // Without a pattern the swing is dim and takes nothing: no press, wheel or hover.
    #[test]
    fn without_a_pattern_the_swing_takes_nothing() {
        let knob = layout::swing_knob_rect().center();
        let mut state = HeaderState::default();
        let left = press(mouse::Button::Left);
        assert!(header_event(None, &mut state, left.clone(), knob).is_none());
        assert!(header_event(None, &mut state, left, knob).is_none());
        assert!(header_event(None, &mut state, wheel(1.0), knob).is_none());
        let moved = canvas::Event::Mouse(mouse::Event::CursorMoved { position: knob });
        let _ = header_event(None, &mut state, moved, knob);
        assert_eq!(state.hovered, None);
        assert_eq!(state.last_swing_press, None);
    }

    // The readout's text, exact from the stored tenths.
    #[test]
    fn a_swing_reads_in_tenths_of_a_per_cent() {
        assert_eq!(swing_text(500), "50.0%");
        assert_eq!(swing_text(667), "66.7%");
        assert_eq!(swing_text(750), "75.0%");
        assert_eq!(swing_text(521), "52.1%");
        assert_eq!(swing_fraction(500), 0.0);
        assert_eq!(swing_fraction(625), 0.5);
        assert_eq!(swing_fraction(750), 1.0);
    }

    // A name longer than its button is cut with an ellipsis to the button's room; one that fits
    // stays whole.
    #[test]
    fn a_long_channel_name_is_cut_with_an_ellipsis() {
        assert_eq!(shown_name("Kick"), "Kick");
        let cut = shown_name("Electronic Drums with a long name");
        assert!(
            cut.starts_with("Electronic") && cut.ends_with('\u{2026}'),
            "{cut}"
        );
        let room = layout::NAME_BUTTON_WIDTH - 2.0 * NAME_PADDING;
        assert!(crate::ui::text_fit::text_width(&cut, size::BODY) <= room);
    }

    // "+ Instrument…" reads at WCAG AA for small text, 4.5:1 at least, at rest and lit, on
    // the rack's ground (no face of its own): the secondary text #9FA3A9 on #1F2125 is 6.36:1,
    // the text #C9CDD3 10.10:1, as the WCAG formula gives them in Python (relative luminances
    // 0.3643, 0.6078 and 0.0151). (Pin moved: the button's text on its #3A3E45 face, 6.73:1.)
    #[test]
    fn the_add_words_have_aa_contrast() {
        let palette = style::palette();
        let resting = style::contrast(palette.text_secondary, palette.bg);
        let lit = style::contrast(palette.text, palette.bg);
        assert!((resting - 6.36).abs() < 0.01, "{resting}");
        assert!((lit - 10.10).abs() < 0.01, "{lit}");
        assert!(resting >= 4.5 && lit >= 4.5);
    }
}
