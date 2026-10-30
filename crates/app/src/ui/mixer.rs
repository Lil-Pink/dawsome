//! The mixer panel (F9): the header with the channel routing control, a strip per insert (name,
//! mute light, pan knob, fader, meters, route button and send knob), and the slot panel with
//! the selected insert's effects and the selected effect's parameters.
//!
//! One canvas draws it all from [`mixer_layout`] positions and only reports presses and the
//! wheel in its own coordinates; [`MixerState`] turns them into edits. What changes with edits
//! is drawn into a cached layer the app clears on every message; the meters, a refused route's
//! flash and the value under the cursor are drawn over it every frame. The slot menu and the
//! automation menu are [`crate::ui::menu`] menus on the window's top layer.

use std::time::{Duration, Instant};

use dawsome_engine::dsp::laws::fader_gain;
use dawsome_engine::processor::{PluginParam, ProcessorHost, Registry};
use dawsome_engine::project::{
    AutomationTarget, ChannelId, Insert, InsertId, ProcessorOwner, Project,
};
use iced::alignment::{Horizontal, Vertical};
use iced::widget::canvas::{self, Cache, Event, Frame, Geometry, Path, Stroke, Text};
use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{column, container, pin, scrollable, stack, text_input, Id};
use iced::{mouse, Element, Fill, Point, Rectangle, Renderer, Size, Theme};
use std::collections::BTreeMap;

use crate::app::Message;
use crate::ui::hint::{hover_change, Hover};
use crate::ui::menu;
use crate::ui::mixer_layout::{
    self, Hit, BODY_TOP, CAP_LINE, CLIP_OFFSET, CLIP_RADIUS, CLIP_Y, FADER_TOP, FADER_TRACK_OFFSET,
    FADER_TRACK_WIDTH, HEADER_HEIGHT, LABEL_HEIGHT, LED_RADIUS, NAME_SIZE, PAN_SIZE,
    SELECTED_OUTLINE, SEND_SIZE, SLOT_NAME_INSET, SLOT_NUMBER_INSET, SLOT_PANEL_WIDTH, STRIP_WIDTH,
    TEXT_SIZE,
};
use crate::ui::mixer_state::{
    effect_param_text, effect_values, fader_text, is_plugin, pan_text, percent_text, Menu,
    MixerMessage, MixerState, ADD_MENU, EFFECT_MENU, PLUGIN_EFFECT_ROW, PLUGIN_PARAMS_ROW,
};
use crate::ui::rack::{draw_automated_dot, draw_knob, draw_linked_dot};
use crate::ui::style;
use crate::ui::text_fit::fit_width;

/// The text box an insert name is edited in.
pub const RENAME_INPUT: &str = "mixer-insert-rename";

/// −6 dB as a linear level, 10^(−6/20), where a meter turns from green to yellow.
pub const MINUS_6_DB: f32 = 0.501_187_2;

/// Two presses at most this far apart in time and space are a double press.
const DOUBLE_PRESS_TIME: Duration = Duration::from_millis(400);
const DOUBLE_PRESS_DISTANCE: f32 = 6.0;

/// What the mixer shows.
pub struct MixerPanel<'a> {
    pub project: &'a Project,
    pub state: &'a MixerState,
    /// The panel's height as the dividers left it ([`crate::ui::panels::Kept::mixer_height`]):
    /// its desk is laid out from it.
    pub height: f32,
    pub registry: &'a Registry,
    /// The rack's selected channel: the one the header routes.
    pub channel: Option<ChannelId>,
    pub cache: &'a Cache,
    /// When this frame is drawn, for flashes and notices.
    pub now: Instant,
    /// The automated controls, with the value each automation set last while it governs: an
    /// automated control shows that value, and a dot.
    pub automated: &'a BTreeMap<AutomationTarget, Option<f32>>,
    /// The automation menu, while it is open over the mixer.
    pub menu: Option<Element<'a, Message>>,
    /// The window's last pixels the menus over the mixer keep clear of: the hint bar's.
    pub menu_foot: f32,
}

impl<'a> MixerPanel<'a> {
    pub fn view(mut self) -> Element<'a, Message> {
        let rename = self.rename_box();
        let slot_menu = self.slot_menu();
        let menu = self.menu.take();
        let desk_height = mixer_layout::desk_height(self.height);
        let desk = canvas::Canvas::new(Desk { panel: self })
            .width(Fill)
            .height(desk_height);
        let mut layers: Vec<Element<'a, Message>> = vec![desk.into()];
        layers.extend(rename);
        layers.extend(slot_menu);
        layers.extend(menu);
        let desk: Element<'a, Message> = stack(layers).into();
        // Why a scrollable: stacked under other panels the mixer can get less than its height;
        // its own wheel scrolls the strips, so the bar scrolls it up and down.
        let desk = scrollable(desk)
            .direction(Direction::Vertical(Scrollbar::default()))
            .width(Fill)
            .height(Fill);
        // Its top edge, the divider over it, is the panels' (`App::panels_view`). Why the
        // column of one: a container lays its content out loose, so the scrollable would leave a
        // desk shorter than the panel short; a column's fill child gets the whole height, which
        // the scrollable stretches its content to (the mixer alone fills the window).
        container(column![desk])
            .width(Fill)
            .height(Fill)
            .style(style::rack_well)
            .into()
    }

    /// The text box over the name of the insert being renamed, while its strip is in view.
    fn rename_box(&self) -> Option<Element<'a, Message>> {
        let (id, name) = self.state.rename.as_ref()?;
        let left = mixer_layout::strip_left(*id, self.state.scroll);
        let input = text_input("", name)
            .id(Id::new(RENAME_INPUT))
            .on_input(|name| Message::Mixer(MixerMessage::RenameEdited(name)))
            .on_submit(Message::Mixer(MixerMessage::RenameSubmitted))
            .size(NAME_SIZE)
            .padding([1, 3])
            .width(STRIP_WIDTH);
        Some(pin(input).x(left).y(mixer_layout::NAME_TOP - 2.0).into())
    }

    /// The open slot menu under its slot, on the window's top layer: a row per choice, each
    /// sending its place in the menu.
    fn slot_menu(&self) -> Option<Element<'a, Message>> {
        let menu = self.state.menu?;
        let rows = slot_menu_rows(menu, self.registry, self.project)
            .into_iter()
            .enumerate()
            .map(|(row, label)| {
                let chosen = Message::Mixer(MixerMessage::MenuChosen(row));
                menu::row(label, Some(chosen)).into()
            });
        let panel = menu::panel(column(rows), mixer_layout::MENU_WIDTH);
        let at = mixer_layout::menu_anchor(menu.slot(), self.state.size().width);
        Some(
            pin(menu::floating(panel, self.menu_foot))
                .x(at.x)
                .y(at.y)
                .into(),
        )
    }

    fn last(&self) -> InsertId {
        self.project.mixer.ids().last().unwrap_or(InsertId::MASTER)
    }

    /// The inserts whose strips are at least partly left of the slot panel, with their left
    /// edges.
    fn visible_strips(&self, width: f32) -> impl Iterator<Item = (InsertId, &Insert, f32)> {
        let slots_left = mixer_layout::slot_panel_left(width);
        let scroll = self.state.scroll;
        self.project
            .mixer
            .ids()
            .zip(&self.project.mixer.inserts)
            .map(move |(id, insert)| (id, insert, mixer_layout::strip_left(id, scroll)))
            .filter(move |&(_, _, left)| left + STRIP_WIDTH > 0.0 && left < slots_left)
    }

    /// The amount the selected insert sends to `to`, when it does and `to` is another insert.
    fn send_to(&self, to: InsertId) -> Option<f32> {
        let selected = self.state.selected;
        if to == selected {
            return None;
        }
        self.project.mixer.get(selected)?.routes.get(&to).copied()
    }
}

/// The mixer canvas.
struct Desk<'a> {
    panel: MixerPanel<'a>,
}

/// The canvas's memory between events.
#[derive(Debug, Default)]
struct DeskState {
    /// The last left press, to spot a double press.
    previous_press: Option<(Point, Instant)>,
    /// What the cursor was over last.
    hover: Option<Hover>,
}

impl Desk<'_> {
    /// The desk as a press finds it, at `size` (the canvas's bounds).
    fn desk(&self, size: Size) -> mixer_layout::Desk {
        mixer_layout::Desk {
            size,
            ..self.panel.state.desk(self.panel.last())
        }
    }

    fn draw_body(&self, frame: &mut Frame) {
        let size = frame.size();
        let slots_left = mixer_layout::slot_panel_left(size.width);
        self.draw_header(frame, size.width);
        let strips = Rectangle::new(
            Point::new(0.0, HEADER_HEIGHT),
            Size::new(slots_left, size.height - HEADER_HEIGHT),
        );
        let solo_audible = self.panel.project.mixer.solo_audible();
        // Why the ground is drawn inside the clipped frame: the renderer lays a clipped frame's
        // shapes under the shapes its parent has not finished yet, so a ground drawn outside
        // would cover the strips.
        frame.with_clip(strips, |frame| {
            frame.fill_rectangle(strips.position(), strips.size(), style::RACK_WELL);
            for (id, insert, left) in self.panel.visible_strips(size.width) {
                let heard = solo_audible.get(usize::from(id.0)).copied().unwrap_or(true);
                self.draw_strip(frame, (id, insert), left, heard, strips, size.height);
            }
        });
        self.draw_slot_panel(frame, size);
    }

    fn draw_header(&self, frame: &mut Frame, width: f32) {
        frame.fill_rectangle(
            Point::ORIGIN,
            Size::new(width, HEADER_HEIGHT),
            style::HEADER,
        );
        label(
            frame,
            "Mixer",
            Point::new(12.0, HEADER_HEIGHT / 2.0),
            style::TEXT,
            12.0,
        );
        for (rect, glyph) in [
            (mixer_layout::route_down_rect(), "\u{2212}"),
            (mixer_layout::route_up_rect(), "+"),
        ] {
            frame.fill_rectangle(rect.position(), rect.size(), style::BUTTON);
            centered(frame, glyph, rect.center(), style::TEXT, 12.0);
        }
        let label_rect = mixer_layout::route_label_rect();
        frame.fill_rectangle(label_rect.position(), label_rect.size(), style::LCD);
        let project = self.panel.project;
        let routing = match self.panel.channel.and_then(|id| project.channels.get(&id)) {
            Some(channel) => format!("{} \u{2192} {}", channel.name, channel.insert),
            None => "No channel".into(),
        };
        if let Some(shown) = fit_width(&routing, label_rect.width, TEXT_SIZE) {
            frame.with_clip(label_rect, |frame| {
                centered(
                    frame,
                    &shown,
                    label_rect.center(),
                    style::LCD_TEXT,
                    TEXT_SIZE,
                );
            });
        }
    }

    /// Insert `id`'s strip at `left`, its name kept inside `area`, the strips' part of the
    /// panel, on a desk `desk` tall.
    fn draw_strip(
        &self,
        frame: &mut Frame,
        (id, insert): (InsertId, &Insert),
        left: f32,
        heard: bool,
        area: Rectangle,
        desk: f32,
    ) {
        let selected = id == self.panel.state.selected;
        let body = mixer_layout::body_rect(left, desk);
        let color = if selected {
            style::STRIP_SELECTED
        } else {
            style::STRIP
        };
        frame.fill_rectangle(body.position(), body.size(), color);
        if selected {
            let inset = SELECTED_OUTLINE / 2.0;
            frame.stroke_rectangle(
                Point::new(body.x + inset, body.y + inset),
                Size::new(
                    body.width - SELECTED_OUTLINE,
                    body.height - SELECTED_OUTLINE,
                ),
                Stroke::default()
                    .with_color(style::ACCENT)
                    .with_width(SELECTED_OUTLINE),
            );
        }
        let name = mixer_layout::name_rect(left);
        let renaming = self
            .panel
            .state
            .rename
            .as_ref()
            .is_some_and(|(renamed, _)| *renamed == id);
        // Why the area too: a clip inside a clip bounds its drawing by its own box only, so a
        // name of a strip under the slot panel would run into the panel's title. Why cut to
        // the clip as well: the software renderer draws a clipped frame's text whole.
        if let Some(clip) = name.intersection(&area).filter(|_| !renaming) {
            let at = Point::new(name.x + 4.0, name.center_y());
            if let Some(shown) = fit_width(&insert.name, clip.x + clip.width - at.x, NAME_SIZE) {
                frame.with_clip(clip, |frame| {
                    label(frame, &shown, at, style::TEXT, NAME_SIZE);
                });
            }
        }
        let led = mixer_layout::led_center(left);
        let lit = if !insert.mute && heard {
            style::TRACK_LED_ON
        } else {
            style::LED_OFF
        };
        frame.fill(&Path::circle(led, LED_RADIUS), lit);
        if self.panel.project.mixer.solo.contains(&id) {
            frame.stroke(
                &Path::circle(led, LED_RADIUS + 1.75),
                Stroke::default().with_color(style::ACCENT).with_width(1.5),
            );
        }
        let automated = |target: AutomationTarget| self.panel.automated.get(&target).copied();
        let linked =
            |target: AutomationTarget| self.panel.project.midi_links.get(&target).is_some();
        let pan_automation = automated(AutomationTarget::InsertPan(id));
        let pan_value = pan_automation.flatten().unwrap_or(insert.pan);
        let pan = mixer_layout::pan_rect(left);
        draw_knob(frame, pan.center(), PAN_SIZE, (pan_value + 1.0) / 2.0);
        if pan_automation.is_some() {
            draw_automated_dot(frame, pan.center(), PAN_SIZE);
        }
        if linked(AutomationTarget::InsertPan(id)) {
            draw_linked_dot(frame, pan.center(), PAN_SIZE);
        }
        frame.fill_rectangle(
            Point::new(left + FADER_TRACK_OFFSET, FADER_TOP),
            Size::new(FADER_TRACK_WIDTH, mixer_layout::fader_span(desk)),
            style::FADER_TRACK,
        );
        let fader_automation = automated(AutomationTarget::InsertFader(id));
        let fader = fader_automation.flatten().unwrap_or(insert.fader);
        let cap = mixer_layout::cap_rect(left, fader, desk);
        frame.fill_rectangle(cap.position(), cap.size(), style::FADER_CAP);
        frame.fill_rectangle(
            Point::new(cap.x, cap.center_y() - CAP_LINE / 2.0),
            Size::new(cap.width, CAP_LINE),
            style::ACCENT,
        );
        let track = Point::new(
            left + FADER_TRACK_OFFSET + FADER_TRACK_WIDTH / 2.0,
            FADER_TOP,
        );
        if fader_automation.is_some() {
            draw_automated_dot(frame, track, FADER_TRACK_WIDTH + 8.0);
        }
        if linked(AutomationTarget::InsertFader(id)) {
            draw_linked_dot(frame, track, FADER_TRACK_WIDTH + 8.0);
        }
        for side in 0..2 {
            let meter = mixer_layout::meter_rect(left, side, desk);
            frame.fill_rectangle(meter.position(), meter.size(), style::METER_GROUND);
        }
        if !selected {
            let route = mixer_layout::route_rect(left, desk);
            let sent = self.panel.send_to(id);
            let color = if sent.is_some() {
                style::ACCENT
            } else {
                style::ROUTE_OFF
            };
            frame.fill_rectangle(route.position(), route.size(), color);
            if let Some(amount) = sent {
                let from = self.panel.state.selected;
                let send = automated(AutomationTarget::Send { from, to: id });
                let knob = mixer_layout::send_rect(left, desk);
                draw_knob(
                    frame,
                    knob.center(),
                    SEND_SIZE,
                    send.flatten().unwrap_or(amount),
                );
                if send.is_some() {
                    draw_automated_dot(frame, knob.center(), SEND_SIZE);
                }
                if linked(AutomationTarget::Send { from, to: id }) {
                    draw_linked_dot(frame, knob.center(), SEND_SIZE);
                }
            }
        }
        let number = if id.is_master() {
            "M".to_owned()
        } else {
            id.0.to_string()
        };
        let label_top = mixer_layout::label_top(desk);
        let label_at = Point::new(left + STRIP_WIDTH / 2.0, label_top + LABEL_HEIGHT / 2.0);
        // Centered, so the room is twice the nearer distance to the area's sides.
        let room = 2.0 * (label_at.x - area.x).min(area.x + area.width - label_at.x);
        if let Some(shown) = fit_width(&number, room, TEXT_SIZE) {
            centered(frame, &shown, label_at, style::TEXT, TEXT_SIZE);
        }
    }

    fn draw_slot_panel(&self, frame: &mut Frame, size: Size) {
        let width = size.width;
        let left = mixer_layout::slot_panel_left(width);
        frame.fill_rectangle(
            Point::new(left, HEADER_HEIGHT),
            Size::new(SLOT_PANEL_WIDTH, size.height - HEADER_HEIGHT),
            style::PANEL,
        );
        let state = self.panel.state;
        let Some(insert) = self.panel.project.mixer.get(state.selected) else {
            return;
        };
        let title = Point::new(
            left + SLOT_NUMBER_INSET,
            mixer_layout::SLOT_NAME_TOP + mixer_layout::SLOT_NAME_HEIGHT / 2.0,
        );
        label(frame, &insert.name, title, style::TEXT, NAME_SIZE);
        for slot in 0..dawsome_engine::project::EFFECT_SLOTS {
            let row = mixer_layout::slot_rect(slot, width);
            let effect = insert.effects.iter().find(|effect| effect.slot == slot);
            if effect.is_some_and(|effect| Some(effect.id) == state.selected_effect) {
                frame.fill_rectangle(row.position(), row.size(), style::STRIP_SELECTED);
            }
            let number = Point::new(row.x + SLOT_NUMBER_INSET, row.center_y());
            label(
                frame,
                &(slot + 1).to_string(),
                number,
                style::TEXT_DIM,
                TEXT_SIZE,
            );
            let name_at = Point::new(row.x + SLOT_NAME_INSET, row.center_y());
            match effect {
                Some(effect) => {
                    let name = self
                        .panel
                        .registry
                        .get(&effect.processor.id)
                        .map_or(effect.processor.display_name(), |factory| factory.name());
                    label(frame, name, name_at, style::TEXT, NAME_SIZE);
                }
                None => label(frame, "\u{2014}", name_at, style::TEXT_DIM, NAME_SIZE),
            }
        }
        self.draw_params(frame, size);
    }

    /// The selected effect's parameters: a label and a slider with the value on it, per row.
    /// The selected effect's parameters, a row each from the one the wheel scrolled to, as many
    /// as the desk has rows for, and a mark of the rows above and below when there are more.
    fn draw_params(&self, frame: &mut Frame, size: Size) {
        let width = size.width;
        let rows = mixer_layout::param_rows(size.height);
        let top = self.panel.state.params_top();
        let left = mixer_layout::slot_panel_left(width);
        let Some((_, effect)) = self
            .panel
            .state
            .selected_effect
            .and_then(|id| self.panel.project.mixer.effect(id))
        else {
            return;
        };
        let Some(factory) = self.panel.registry.get(&effect.processor.id) else {
            if let Some(host) = self.panel.registry.host(&effect.processor.id) {
                let owner = ProcessorOwner::Effect(effect.id);
                self.draw_plugin_params(frame, size, host, owner);
                return;
            }
            let at = Point::new(left + SLOT_NUMBER_INSET, mixer_layout::PARAMS_TOP + 10.0);
            label(
                frame,
                "Not available in this build",
                at,
                style::TEXT_DIM,
                TEXT_SIZE,
            );
            return;
        };
        let values = effect_values(effect, self.panel.registry).unwrap_or_default();
        draw_more_marks(frame, width, size.height, top, factory.params().len());
        for (index, (info, &value)) in factory
            .params()
            .iter()
            .zip(&values)
            .enumerate()
            .skip(top)
            .take(rows)
        {
            let row = mixer_layout::param_row_rect(index - top, width);
            let at = Point::new(left + mixer_layout::PARAM_LABEL_INSET, row.center_y());
            label(frame, info.label, at, style::TEXT_DIM, 10.0);
            let target = AutomationTarget::EffectParam {
                effect: effect.id,
                key: info.key.to_owned(),
            };
            let automation = self.panel.automated.get(&target).copied();
            let value = automation.flatten().unwrap_or(value);
            let slider = mixer_layout::slider_rect(index - top, width);
            if automation.is_some() {
                let dot = Point::new(slider.x - 6.0, slider.center_y());
                draw_automated_dot(frame, dot, 4.0);
            }
            if self.panel.project.midi_links.get(&target).is_some() {
                // At the slider's top right corner, as a knob's sits at its box's.
                let corner = Point::new(slider.x + slider.width, slider.y);
                draw_linked_dot(frame, corner, 0.0);
            }
            frame.fill_rectangle(slider.position(), slider.size(), style::FADER_TRACK);
            let span = info.max - info.min;
            let fraction = if span > 0.0 {
                ((value - info.min) / span).clamp(0.0, 1.0)
            } else {
                0.0
            };
            frame.fill_rectangle(
                slider.position(),
                Size::new(slider.width * fraction, slider.height),
                style::SLIDER_FILL,
            );
            let rate = self.panel.project.sample_rate;
            let shown = effect_param_text(factory, index, value, rate).unwrap_or_default();
            centered(frame, &shown, slider.center(), style::TEXT, 10.0);
        }
    }

    /// A plugin effect's first parameters (those not hidden), read from its host: a label and a
    /// slider with the plugin's own text for the value, per row.
    fn draw_plugin_params(
        &self,
        frame: &mut Frame,
        size: Size,
        host: &dyn ProcessorHost,
        owner: ProcessorOwner,
    ) {
        let width = size.width;
        let left = mixer_layout::slot_panel_left(width);
        let shown: Vec<PluginParam> = host
            .params(owner)
            .into_iter()
            .filter(|param| !param.hidden)
            .collect();
        let top = self.panel.state.params_top();
        draw_more_marks(frame, width, size.height, top, shown.len());
        let params: Vec<PluginParam> = shown
            .into_iter()
            .skip(top)
            .take(mixer_layout::param_rows(size.height))
            .collect();
        if params.is_empty() {
            // A plugin that is missing has no instance to read, and some have no parameters.
            let at = Point::new(left + SLOT_NUMBER_INSET, mixer_layout::PARAMS_TOP + 10.0);
            label(
                frame,
                "No parameters to show",
                at,
                style::TEXT_DIM,
                TEXT_SIZE,
            );
            return;
        }
        for (index, param) in params.iter().enumerate() {
            let row = mixer_layout::param_row_rect(index, width);
            let at = Point::new(left + mixer_layout::PARAM_LABEL_INSET, row.center_y());
            label(frame, &param.name, at, style::TEXT_DIM, 10.0);
            let slider = mixer_layout::slider_rect(index, width);
            frame.fill_rectangle(slider.position(), slider.size(), style::FADER_TRACK);
            let value = host.value(owner, param.id).unwrap_or(param.default);
            let span = param.max - param.min;
            let fraction = if span > 0.0 {
                ((value - param.min) / span).clamp(0.0, 1.0) as f32
            } else {
                0.0
            };
            frame.fill_rectangle(
                slider.position(),
                Size::new(slider.width * fraction, slider.height),
                style::SLIDER_FILL,
            );
            let text = host
                .value_text(owner, param.id, value)
                .unwrap_or_else(|| format!("{value:.2}"));
            centered(frame, &text, slider.center(), style::TEXT, 10.0);
        }
    }

    /// The meters, the clip light, a refusal's flash and notice and the value under the cursor.
    fn draw_overlay(&self, frame: &mut Frame, cursor: Option<Point>) {
        let (width, desk) = (frame.width(), frame.height());
        let slots_left = mixer_layout::slot_panel_left(width);
        let strips = Rectangle::new(
            Point::new(0.0, HEADER_HEIGHT),
            Size::new(slots_left, desk - HEADER_HEIGHT),
        );
        let state = self.panel.state;
        let flashing = state.flashing(self.panel.now);
        // Why the meters and the flash are cut to the strips' area as well as clipped: the
        // software renderer draws a clipped frame's shapes whole, so a strip under the slot
        // panel would show its meter over the panel.
        frame.with_clip(strips, |frame| {
            for (id, _, left) in self.panel.visible_strips(width) {
                for side in 0..2 {
                    let Some(meter) =
                        mixer_layout::meter_rect(left, side, desk).intersection(&strips)
                    else {
                        continue;
                    };
                    draw_meter(
                        frame,
                        meter,
                        state.meters.level(id)[side],
                        state.meters.peak_mark(id)[side],
                    );
                }
                if id.is_master() {
                    let light = if state.meters.clip {
                        style::METER_RED
                    } else {
                        style::LED_OFF
                    };
                    let center = Point::new(left + CLIP_OFFSET, CLIP_Y);
                    frame.fill(&Path::circle(center, CLIP_RADIUS), light);
                }
                if flashing == Some(id) && id != state.selected {
                    let route = mixer_layout::route_rect(left, desk);
                    if let Some(route) = route.intersection(&strips) {
                        frame.fill_rectangle(route.position(), route.size(), style::REFUSED);
                    }
                }
            }
        });
        if let Some(notice) = state.notice(self.panel.now) {
            let at = Point::new(80.0, HEADER_HEIGHT / 2.0);
            label(frame, notice, at, style::WARNING, TEXT_SIZE);
        }
        if let Some((rect, value)) = cursor.and_then(|at| self.tip_at(at, frame.size())) {
            // Clipped as the strips are: a value wider than a narrowed tip stays off the panel.
            frame.with_clip(strips, |frame| draw_tip(frame, rect, &value));
        }
    }

    /// Where the tip for the control at `at` shows, and its value in units: what the strip
    /// draws, so while an automation governs the control, the value it set last.
    fn tip_at(&self, at: Point, size: Size) -> Option<(Rectangle, String)> {
        let state = self.panel.state;
        let mixer = &self.panel.project.mixer;
        let width = size.width;
        let hit = mixer_layout::hit_test(at, self.desk(size))?;
        let shown = |target: AutomationTarget, stored: f32| {
            let automated = self.panel.automated.get(&target).copied().flatten();
            automated.unwrap_or(stored)
        };
        let (strip, value) = match hit {
            Hit::Fader(id) => {
                let fader = shown(AutomationTarget::InsertFader(id), mixer.get(id)?.fader);
                (id, fader_text(fader_gain(fader)))
            }
            Hit::Pan(id) => (
                id,
                pan_text(shown(AutomationTarget::InsertPan(id), mixer.get(id)?.pan)),
            ),
            Hit::Send(id) => {
                let from = state.selected;
                let amount = shown(
                    AutomationTarget::Send { from, to: id },
                    self.panel.send_to(id)?,
                );
                (id, percent_text(amount))
            }
            _ => return None,
        };
        let left = mixer_layout::strip_left(strip, state.scroll);
        Some((
            tip_rect(left, at.y, mixer_layout::slot_panel_left(width)),
            value,
        ))
    }
}

impl canvas::Program<Message> for Desk<'_> {
    type State = DeskState;

    fn update(
        &self,
        state: &mut DeskState,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let send = |message| canvas::Action::publish(Message::Mixer(message));
        let input = match event {
            Event::Mouse(mouse::Event::ButtonPressed(
                button @ (mouse::Button::Left | mouse::Button::Right),
            )) => cursor.position_in(bounds).map(|at| {
                let now = crate::ui::clock::now();
                let double = *button == mouse::Button::Left
                    && state.previous_press.is_some_and(|(previous, time)| {
                        now.duration_since(time) <= DOUBLE_PRESS_TIME
                            && previous.distance(at) <= DOUBLE_PRESS_DISTANCE
                    });
                // A double press is not the first of another one.
                state.previous_press =
                    (*button == mouse::Button::Left && !double).then_some((at, now));
                send(MixerMessage::Pressed {
                    at,
                    button: *button,
                    double,
                })
                .and_capture()
            }),
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => cursor
                .position_in(bounds)
                .map(|at| send(MixerMessage::Scrolled { at, delta: *delta }).and_capture()),
            // The value under the cursor follows it, and the hint bar names what is under it.
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let desk = self.desk(bounds.size());
                let now = cursor
                    .position_in(bounds)
                    .and_then(|at| mixer_layout::hit_test(at, desk))
                    .map(Hover::Mixer);
                hover_change(&mut state.hover, now)
                    .or_else(|| cursor.is_over(bounds).then(canvas::Action::request_redraw))
            }
            _ => None,
        };
        // A new size is reported on the next event that carries no input, so no press is lost.
        input.or_else(|| {
            (bounds.size() != self.panel.state.size())
                .then(|| send(MixerMessage::Resized(bounds.size())))
        })
    }

    fn draw(
        &self,
        _state: &DeskState,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let body = self
            .panel
            .cache
            .draw(renderer, bounds.size(), |frame| self.draw_body(frame));
        let mut overlay = Frame::new(renderer, bounds.size());
        self.draw_overlay(&mut overlay, cursor.position_in(bounds));
        vec![body, overlay.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _state: &DeskState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        let Some(at) = cursor.position_in(bounds) else {
            return mouse::Interaction::default();
        };
        match mixer_layout::hit_test(at, self.desk(bounds.size())) {
            Some(Hit::Fader(_) | Hit::Pan(_) | Hit::Send(_)) => {
                mouse::Interaction::ResizingVertically
            }
            Some(Hit::Param(_)) => mouse::Interaction::ResizingHorizontally,
            Some(Hit::Strip(_) | Hit::Name(_)) | None => mouse::Interaction::default(),
            Some(_) => mouse::Interaction::Pointer,
        }
    }
}

/// The marks beside the parameter rows when the selected effect has more than the desk shows:
/// a small triangle at the slot panel's right edge, up at the first row while rows are
/// scrolled above it, down at the last while more follow, so the wheel's reach is seen.
fn draw_more_marks(frame: &mut Frame, width: f32, desk: f32, top: usize, count: usize) {
    let rows = mixer_layout::param_rows(desk);
    if rows == 0 || count <= rows {
        return;
    }
    let x = width - 8.0;
    let first = mixer_layout::param_row_rect(0, width).center_y();
    let last = mixer_layout::param_row_rect(rows - 1, width).center_y();
    let triangle = |tip: f32, base: f32| {
        Path::new(|path| {
            path.move_to(Point::new(x, tip));
            path.line_to(Point::new(x + 4.0, base));
            path.line_to(Point::new(x - 4.0, base));
            path.close();
        })
    };
    if top > 0 {
        frame.fill(&triangle(first - 3.0, first + 3.0), style::TEXT_DIM);
    }
    if top + rows < count {
        frame.fill(&triangle(last + 3.0, last - 3.0), style::TEXT_DIM);
    }
}

/// A meter column filled from the bottom to `level`: green to −6 dB, yellow to 0 dB, red above,
/// with a one-pixel mark at `mark`.
fn draw_meter(frame: &mut Frame, meter: Rectangle, level: f32, mark: f32) {
    let bottom = meter.y + meter.height;
    // The meter's whole column, which may be cut by the strips' area at the top.
    let span = meter.height;
    let height = mixer_layout::meter_height(level, span);
    let (minus_6_db, zero_db) = (
        mixer_layout::meter_height(MINUS_6_DB, span),
        mixer_layout::meter_height(1.0, span),
    );
    let bands = [
        (0.0, minus_6_db, style::METER_GREEN),
        (minus_6_db, zero_db, style::METER_YELLOW),
        (zero_db, span, style::METER_RED),
    ];
    for (from, to, color) in bands {
        let top = height.min(to);
        if top > from {
            frame.fill_rectangle(
                Point::new(meter.x, bottom - top),
                Size::new(meter.width, top - from),
                color,
            );
        }
    }
    let mark_height = mixer_layout::meter_height(mark, span);
    if mark_height > 0.0 {
        let color = bands
            .iter()
            .find(|(_, to, _)| mark_height <= *to)
            .map_or(style::METER_RED, |&(_, _, color)| color);
        frame.fill_rectangle(
            Point::new(meter.x, bottom - mark_height),
            Size::new(meter.width, 1.0),
            color,
        );
    }
}

/// The words of an open slot menu's rows, top to bottom, one per row [`Menu::rows`] counts:
/// under an empty slot the native effects by name and the plugin row; under a filled one
/// Remove, Move up and Move down, and for a plugin effect its parameters (its parameter window).
fn slot_menu_rows(menu: Menu, registry: &Registry, project: &Project) -> Vec<&'static str> {
    match menu {
        Menu::Add { .. } => ADD_MENU
            .iter()
            .map(|id| registry.get(id).map_or(*id, |factory| factory.name()))
            .chain([PLUGIN_EFFECT_ROW])
            .collect(),
        Menu::Effect { effect, .. } => {
            let mut rows = EFFECT_MENU.to_vec();
            if is_plugin(project, effect) {
                rows.push(PLUGIN_PARAMS_ROW);
            }
            rows
        }
    }
}

/// The value tip's height, and how far inside its strip's sides it stays.
pub(crate) const TIP_HEIGHT: f32 = 16.0;
const TIP_INSET: f32 = 2.0;

/// Where the tip over a control of the strip whose left edge is `left` shows, the cursor at
/// height `y`: across the strip, just inside its sides, 22 px above the cursor (no higher than
/// the strip's top), and no further right than `limit`, the slot panel's left edge. Why inside
/// the strip: a tip beside the cursor covered the next strip's controls. Why the limit: a strip
/// running under the slot panel shows only its left part there, and the panel stays whole.
fn tip_rect(left: f32, y: f32, limit: f32) -> Rectangle {
    let x = left + TIP_INSET;
    let right = (left + STRIP_WIDTH - TIP_INSET).min(limit);
    Rectangle::new(
        Point::new(x, (y - 22.0).max(BODY_TOP)),
        Size::new((right - x).max(0.0), TIP_HEIGHT),
    )
}

/// A small dark box at `rect` with `value` centered in it, cut to the box's width: the value tip
/// of the mixer's controls and of the rack's knobs.
pub(crate) fn draw_tip(frame: &mut Frame, rect: Rectangle, value: &str) {
    frame.fill_rectangle(rect.position(), rect.size(), style::LCD);
    if let Some(shown) = fit_width(value, rect.width, TEXT_SIZE) {
        centered(frame, &shown, rect.center(), style::LCD_TEXT, TEXT_SIZE);
    }
}

/// Left-aligned text, vertically centered on `at`.
pub(crate) fn label(frame: &mut Frame, content: &str, at: Point, color: iced::Color, size: f32) {
    frame.fill_text(Text {
        content: content.to_owned(),
        position: at,
        color,
        size: size.into(),
        align_y: Vertical::Center,
        ..Text::default()
    });
}

/// Text centered on `at`.
pub(crate) fn centered(frame: &mut Frame, content: &str, at: Point, color: iced::Color, size: f32) {
    frame.fill_text(Text {
        content: content.to_owned(),
        position: at,
        color,
        size: size.into(),
        align_x: Horizontal::Center.into(),
        align_y: Vertical::Center,
        ..Text::default()
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const P5_GAIN: &str = include_str!("../../../../tests/fixtures/p5_gain.json");

    /// The tip over mixer point `at` in a 1000 px panel showing `project` as `state` has it,
    /// with `automated` governing: where it shows, and its text.
    fn tip_at(
        project: &Project,
        state: &MixerState,
        automated: &BTreeMap<AutomationTarget, Option<f32>>,
        at: Point,
    ) -> Option<(Rectangle, String)> {
        let registry = Registry::native();
        let cache = Cache::new();
        let panel = MixerPanel {
            project,
            state,
            height: mixer_layout::DEFAULT_HEIGHT,
            registry: &registry,
            channel: None,
            cache: &cache,
            now: Instant::now(),
            automated,
            menu: None,
            menu_foot: 0.0,
        };
        let desk = Desk { panel };
        desk.tip_at(at, state.size())
    }

    // The tip over a fader, a pan knob or a send knob says, in its units, the value the strip
    // draws, inside that strip: the stored value, or while an automation governs the control,
    // the value it set last. Insert 2 of p5_gain stores fader 0.63496, (0.63496 / 0.8)³ = 0.5:
    // −6.02 dB; an automated 0.4 is (0.4 / 0.8)³ = 0.125: −18.06 dB; an automated pan of −0.5
    // is half left. Insert 4 sends 0.5 to insert 5: with insert 4 selected, strip 5's send knob
    // says 50%, and while an automation of that send (from 4 to 5) governs, the 25% it set.
    #[test]
    fn the_tip_says_the_value_the_strip_shows() {
        let project = Project::load_json(P5_GAIN).expect("loads");
        let mut state = MixerState::new(1000.0, mixer_layout::DEFAULT_HEIGHT);
        let desk = state.size().height;
        // The tip over `control` of the strip at `left`, which must show inside that strip.
        let tip = |state: &MixerState,
                   automated: &BTreeMap<AutomationTarget, Option<f32>>,
                   left: f32,
                   control: fn(f32, f32) -> Rectangle| {
            let at = control(left, desk).center();
            tip_at(&project, state, automated, at).map(|(rect, text)| {
                let strip = mixer_layout::body_rect(left, desk);
                let inside = rect.x >= strip.x && rect.x + rect.width <= strip.x + strip.width;
                assert!(inside, "{rect:?} is not inside the strip {strip:?}");
                text
            })
        };
        let (fader, pan, send): (fn(f32, f32) -> Rectangle, _, _) = (
            mixer_layout::fader_rect,
            |left, _| mixer_layout::pan_rect(left),
            mixer_layout::send_rect,
        );
        let two = mixer_layout::strip_left(InsertId(2), 0.0);
        let mut automated = BTreeMap::new();
        assert_eq!(
            tip(&state, &automated, two, fader).as_deref(),
            Some("\u{2212}6.0 dB")
        );
        assert_eq!(tip(&state, &automated, two, pan).as_deref(), Some("Center"));
        // Governed, before the automation has set a value: the stored one still shows.
        automated.insert(AutomationTarget::InsertFader(InsertId(2)), None);
        assert_eq!(
            tip(&state, &automated, two, fader).as_deref(),
            Some("\u{2212}6.0 dB")
        );
        automated.insert(AutomationTarget::InsertFader(InsertId(2)), Some(0.4));
        automated.insert(AutomationTarget::InsertPan(InsertId(2)), Some(-0.5));
        assert_eq!(
            tip(&state, &automated, two, fader).as_deref(),
            Some("\u{2212}18.1 dB")
        );
        assert_eq!(tip(&state, &automated, two, pan).as_deref(), Some("50% L"));

        state.select(InsertId(4), &project);
        let five = mixer_layout::strip_left(InsertId(5), 0.0);
        assert_eq!(tip(&state, &automated, five, send).as_deref(), Some("50%"));
        let four_to_five = AutomationTarget::Send {
            from: InsertId(4),
            to: InsertId(5),
        };
        automated.insert(four_to_five, Some(0.25));
        assert_eq!(tip(&state, &automated, five, send).as_deref(), Some("25%"));
    }

    // Every row of a slot menu holds its words on one line, 10 px of the row either side of
    // them (`menu::row`). Why: at the menus' 13 px, "Compressor (Pressure4)" ran past the 120 px
    // the slot menu had in the canvas.
    #[test]
    fn a_slot_menus_rows_hold_their_words() {
        use crate::ui::text_fit::text_width;
        let project = Project::load_json(P5_GAIN).expect("loads");
        let registry = Registry::native();
        let added = slot_menu_rows(Menu::Add { slot: 0 }, &registry, &project);
        assert_eq!(added.len(), ADD_MENU.len() + 1);
        for words in added
            .into_iter()
            .chain(EFFECT_MENU)
            .chain([PLUGIN_PARAMS_ROW])
        {
            let needed = text_width(words, menu::TEXT_SIZE) + 2.0 * 10.0;
            println!("{words}: {needed} px");
            assert!(
                needed <= mixer_layout::MENU_WIDTH,
                "{words} needs {needed} px"
            );
        }
    }

    // At 1000 px strip 12 runs under the slot panel: its left edge is at 12 + 62·12 = 756 and
    // the panel's at 1000 − 200 = 800. The tip over its fader starts 2 px inside the strip and
    // stops at the panel's edge.
    #[test]
    fn a_tip_stops_at_the_slot_panel() {
        let project = Project::load_json(P5_GAIN).expect("loads");
        let state = MixerState::new(1000.0, mixer_layout::DEFAULT_HEIGHT);
        let left = mixer_layout::strip_left(InsertId(12), 0.0);
        assert_eq!(left, 756.0);
        let fader = mixer_layout::fader_rect(left, state.size().height).center();
        let (rect, _) =
            tip_at(&project, &state, &BTreeMap::new(), fader).expect("a tip over the fader");
        assert_eq!(rect.x, 758.0);
        assert!(
            rect.x + rect.width <= 800.0,
            "{rect:?} reaches under the slot panel"
        );
    }
}
