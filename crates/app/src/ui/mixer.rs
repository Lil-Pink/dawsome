//! The mixer panel (F9): the header with the channel routing control, a strip per insert (name,
//! mute light, pan knob, fader, meters, route button and send knob), and the effects area with
//! the selected insert's effects as blocks in chain order (each its light, name, remove cross
//! and knobs) and the add tile after them.
//!
//! One canvas draws it all from [`mixer_layout`] positions and only reports presses and the
//! wheel in its own coordinates; [`MixerState`] turns them into edits. What changes with edits
//! is drawn into a cached layer the app clears on every message; the meters, a refused route's
//! flash and the value under the cursor are drawn over it every frame. The effect menus and the
//! automation menu are [`crate::ui::menu`] menus on the window's top layer.

use std::time::{Duration, Instant};

use dawsome_engine::dsp::laws::fader_gain;
use dawsome_engine::processor::{PluginParam, ProcessorFactory, ProcessorHost, Registry};
use dawsome_engine::project::{
    AutomationTarget, ChannelId, Effect, Insert, InsertId, ProcessorOwner, Project,
};
use iced::alignment::{Horizontal, Vertical};
use iced::widget::canvas::{self, Cache, Event, Frame, Geometry, Path, Stroke, Text};
use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{column, container, pin, scrollable, stack, text_input, Id};
use iced::{mouse, Element, Fill, Point, Rectangle, Renderer, Size, Theme};
use std::collections::BTreeMap;

use crate::app::Message;
use crate::ui::console::Finish;
use crate::ui::fonts::size;
use crate::ui::hint::{hover_change, Hover};
use crate::ui::knob_block::{
    self, draw_ground, draw_knob_cell, draw_param, fraction, head_rect, shows_whole, BLOCK_HEAD,
    BLOCK_INSET, KNOB_ROW_HEIGHT,
};
use crate::ui::menu;
use crate::ui::mixer_layout::{
    self, Hit, BODY_TOP, CAP_LINE, CLIP_OFFSET, CLIP_RADIUS, CLIP_Y, FADER_TOP, FADER_TRACK_OFFSET,
    FADER_TRACK_WIDTH, HEADER_HEIGHT, LABEL_HEIGHT, LED_RADIUS, MARKS_WIDTH, MENU_WIDTH, PAN_SIZE,
    SELECTED_OUTLINE, SEND_SIZE, STRIP_WIDTH,
};
use crate::ui::mixer_state::{
    effect_values, fader_text, is_choice, is_plugin, knob_counts, pan_text, percent_text, Menu,
    MixerMessage, MixerState, ADD_MENU, EFFECT_MENU, PLUGIN_EFFECT_ROW, PLUGIN_KNOBS,
    PLUGIN_PARAMS_ROW,
};
use crate::ui::panels::Panel;
use crate::ui::pop_out;
use crate::ui::rack::{draw_automated_dot, draw_knob, draw_linked_dot, draw_mute_light};
use crate::ui::style;
use crate::ui::text_fit::fit_width;

/// The text box an insert name is edited in.
pub const RENAME_INPUT: &str = "mixer-insert-rename";

/// −6 dB as a linear level, 10^(−6/20), where a meter turns from green to yellow.
pub const MINUS_6_DB: f32 = 0.501_187_2;

/// Two presses at most this far apart in time and space are a double press.
pub(crate) const DOUBLE_PRESS_TIME: Duration = Duration::from_millis(400);
pub(crate) const DOUBLE_PRESS_DISTANCE: f32 = 6.0;

/// What the mixer shows.
pub struct MixerPanel<'a> {
    pub project: &'a Project,
    pub state: &'a MixerState,
    /// The panel's height as the stack shows it ([`crate::ui::panels::stack_heights`], never
    /// under [`mixer_layout::LEAST_HEIGHT`]): its desk is laid out from it, so every strip
    /// shows whole.
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
    /// Whether the mixer is in the stack, where its header ends with the pop-out button
    /// ([`crate::ui::pop_out`]); in its own window it is not.
    pub docked: bool,
}

impl<'a> MixerPanel<'a> {
    pub fn view(mut self) -> Element<'a, Message> {
        let rename = self.rename_box();
        let effect_menu = self.effect_menu();
        let menu = self.menu.take();
        let desk_height = mixer_layout::desk_height(self.height);
        let desk = canvas::Canvas::new(Desk { panel: self })
            .width(Fill)
            .height(desk_height);
        let mut layers: Vec<Element<'a, Message>> = vec![desk.into()];
        layers.extend(rename);
        layers.extend(effect_menu);
        layers.extend(menu);
        let desk: Element<'a, Message> = stack(layers).into();
        // Why a scrollable, though the desk is laid out from the height the panel is shown at
        // and never scrolls in it now: it gives the menus over the desk the panel's top as the
        // top of the area they opened in ([`menu::place_below`]), which they flip up no
        // further than while they have room; the panel's own wheel scrolls the strips.
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
            .size(size::LABEL)
            .padding([1, 3])
            .width(STRIP_WIDTH);
        Some(pin(input).x(left).y(mixer_layout::NAME_TOP - 2.0).into())
    }

    /// The open effect menu under its block's head (the add menu under the add tile's), on the
    /// window's top layer: a row per choice, each sending its place in the menu.
    fn effect_menu(&self) -> Option<Element<'a, Message>> {
        let menu = self.state.menu?;
        let rows = menu_rows(menu, self.registry, self.project)
            .into_iter()
            .enumerate()
            .map(|(row, label)| {
                let chosen = Message::Mixer(MixerMessage::MenuChosen(row));
                menu::row(label, Some(chosen)).into()
            });
        let panel = menu::panel(column(rows), MENU_WIDTH);
        let chain = self.chain();
        let place = match menu {
            Menu::Add => chain.len(),
            Menu::Effect { effect } => self
                .project
                .mixer
                .get(self.state.selected)?
                .effects
                .iter()
                .position(|shown| shown.id == effect)?,
        };
        let desk = self.state.desk(self.last());
        let block = *mixer_layout::chain_rects(&chain, desk).get(place)?;
        let at = mixer_layout::menu_anchor(block);
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

    /// The selected insert's effects' knob counts, in chain order: what its blocks lay out by.
    fn chain(&self) -> Vec<usize> {
        knob_counts(self.project, self.registry, self.state.selected)
    }

    /// The inserts whose strips are at least partly left of the effects area, with their left
    /// edges.
    fn visible_strips(&self, width: f32) -> impl Iterator<Item = (InsertId, &Insert, f32)> {
        let area = mixer_layout::effects_left(width, self.last());
        let scroll = self.state.scroll;
        self.project
            .mixer
            .ids()
            .zip(&self.project.mixer.inserts)
            .map(move |(id, insert)| (id, insert, mixer_layout::strip_left(id, scroll)))
            .filter(move |&(_, _, left)| left + STRIP_WIDTH > 0.0 && left < area)
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
        let area = mixer_layout::effects_left(size.width, self.panel.last());
        self.draw_header(frame, size.width);
        let strips = Rectangle::new(
            Point::new(0.0, HEADER_HEIGHT),
            Size::new(area, size.height - HEADER_HEIGHT),
        );
        let solo_audible = self.panel.project.mixer.solo_audible();
        // Why the ground is drawn inside the clipped frame: the renderer lays a clipped frame's
        // shapes under the shapes its parent has not finished yet, so a ground drawn outside
        // would cover the strips.
        frame.with_clip(strips, |frame| {
            frame.fill_rectangle(strips.position(), strips.size(), style::palette().bg);
            for (id, insert, left) in self.panel.visible_strips(size.width) {
                let heard = solo_audible.get(usize::from(id.0)).copied().unwrap_or(true);
                self.draw_strip(frame, (id, insert), left, heard, strips, size.height);
            }
        });
        self.draw_effects(frame, size);
    }

    fn draw_header(&self, frame: &mut Frame, width: f32) {
        frame.fill_rectangle(
            Point::ORIGIN,
            Size::new(width, HEADER_HEIGHT),
            style::palette().raised,
        );
        label(
            frame,
            Panel::Mixer.title(),
            Point::new(mixer_layout::TITLE_X, HEADER_HEIGHT / 2.0),
            style::palette().text,
            size::PANEL_TITLE,
        );
        for (rect, glyph) in [
            (mixer_layout::route_down_rect(width), "\u{2212}"),
            (mixer_layout::route_up_rect(width), "+"),
        ] {
            frame.fill_rectangle(rect.position(), rect.size(), style::palette().button);
            centered(
                frame,
                glyph,
                rect.center(),
                style::palette().text,
                size::LABEL,
            );
        }
        let label_rect = mixer_layout::route_label_rect(width);
        frame.fill_rectangle(
            label_rect.position(),
            label_rect.size(),
            style::palette().lcd,
        );
        let project = self.panel.project;
        let routing = match self.panel.channel.and_then(|id| project.channels.get(&id)) {
            Some(channel) => format!("{} \u{2192} {}", channel.name, channel.insert),
            None => "No channel".into(),
        };
        if let Some(shown) = fit_width(&routing, label_rect.width, size::LABEL) {
            frame.with_clip(label_rect, |frame| {
                centered(
                    frame,
                    &shown,
                    label_rect.center(),
                    style::palette().lcd_text,
                    size::LABEL,
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
            style::palette().strip_selected
        } else {
            style::palette().strip
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
                    .with_color(style::accent())
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
        // name of a strip under the effects area would run into the area's title. Why cut to
        // the clip as well: the software renderer draws a clipped frame's text whole.
        if let Some(clip) = name.intersection(&area).filter(|_| !renaming) {
            let at = Point::new(name.x + 4.0, name.center_y());
            if let Some(shown) = fit_width(&insert.name, clip.x + clip.width - at.x, size::LABEL) {
                frame.with_clip(clip, |frame| {
                    label(frame, &shown, at, style::palette().text, size::LABEL);
                });
            }
        }
        let led = mixer_layout::led_center(left);
        let heard = !insert.mute && heard;
        draw_mute_light(frame, led, LED_RADIUS, heard.then_some(style::TRACK_LED_ON));
        if self.panel.project.mixer.solo.contains(&id) {
            frame.stroke(
                &Path::circle(led, LED_RADIUS + 1.75),
                Stroke::default()
                    .with_color(style::accent())
                    .with_width(1.5),
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
            style::palette().fader_track,
        );
        let fader_automation = automated(AutomationTarget::InsertFader(id));
        let fader = fader_automation.flatten().unwrap_or(insert.fader);
        let cap = mixer_layout::cap_rect(left, fader, desk);
        frame.fill_rectangle(cap.position(), cap.size(), style::palette().fader_cap);
        frame.fill_rectangle(
            Point::new(cap.x, cap.center_y() - CAP_LINE / 2.0),
            Size::new(cap.width, CAP_LINE),
            style::accent(),
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
            frame.fill_rectangle(
                meter.position(),
                meter.size(),
                style::palette().meter_ground,
            );
        }
        if !selected {
            let route = mixer_layout::route_rect(left, desk);
            let sent = self.panel.send_to(id);
            let color = if sent.is_some() {
                style::accent()
            } else {
                style::palette().route_off
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
        if let Some(shown) = fit_width(&number, room, size::DENSE) {
            centered(frame, &shown, label_at, style::palette().text, size::DENSE);
        }
    }

    /// The effects area: its ground (no title: the header's routing control names the selected
    /// insert, so the name over the blocks went), its effects' blocks in chain
    /// order and the add tile, and the marks of lines scrolled out of view. A block's parts show
    /// only where they show whole in the view: the software renderer draws a clipped frame's
    /// text and shapes whole, so a part cut by the view's edge would run over the title.
    fn draw_effects(&self, frame: &mut Frame, size: Size) {
        let desk = self.desk(size);
        let left = mixer_layout::effects_left(size.width, desk.last);
        frame.fill_rectangle(
            Point::new(left, HEADER_HEIGHT),
            Size::new(size.width - left, size.height - HEADER_HEIGHT),
            style::palette().panel,
        );
        let Some(insert) = self.panel.project.mixer.get(self.panel.state.selected) else {
            return;
        };
        let counts = self.panel.chain();
        // The chain as it shows: a block being dragged at the place it would take.
        let order = self.panel.state.chain_order(insert.effects.len());
        let chain: Vec<usize> = order.iter().map(|&place| counts[place]).collect();
        let view = mixer_layout::blocks_view(desk);
        let reorder = self.panel.state.reorder;
        let mut held = None;
        for (place, &block) in mixer_layout::chain_rects(&chain, desk).iter().enumerate() {
            let shown = order
                .get(place)
                .and_then(|&index| insert.effects.get(index));
            match (shown, chain.get(place)) {
                (Some(effect), Some(&count)) if reorder.is_some_and(|r| r.effect == effect.id) => {
                    draw_gap(frame, block, view);
                    held = Some((effect, block, count));
                }
                (Some(effect), Some(&count)) => self.draw_block(frame, effect, block, count, view),
                _ => draw_add_tile(frame, block, view),
            }
        }
        // The dragged block over the others, where the pointer holds it, kept inside the view.
        if let (Some((effect, block, count)), Some(reorder)) = (held, reorder) {
            let top_left = reorder.at - reorder.grip;
            let x = top_left
                .x
                .min(view.x + view.width - block.width)
                .max(view.x);
            let y = top_left
                .y
                .min(view.y + view.height - block.height)
                .max(view.y);
            let moved = Rectangle::new(Point::new(x, y), block.size());
            self.draw_block(frame, effect, moved, count, view);
            frame.stroke_rectangle(
                moved.position(),
                moved.size(),
                Stroke::default()
                    .with_color(style::accent())
                    .with_width(1.0),
            );
        }
        let scroll = mixer_layout::effects_scroll(&chain, desk);
        let more = scroll < mixer_layout::max_effects_scroll(&chain, desk);
        draw_more_marks(frame, view, size.width, scroll > 0.0, more);
    }

    /// `effect`'s block at `block`, with `count` knobs, in `view`: its ground, its head (the
    /// light as a strip's, lit while the effect is on; its name, dim while it is off; the
    /// remove cross) and its knobs.
    fn draw_block(
        &self,
        frame: &mut Frame,
        effect: &Effect,
        block: Rectangle,
        count: usize,
        view: Rectangle,
    ) {
        draw_ground(frame, block, view, Finish::Flat);
        if shows_whole(view, head_rect(block)) {
            let lit = if effect.on {
                style::TRACK_LED_ON
            } else {
                style::palette().led_off
            };
            let led = mixer_layout::effect_led_center(block);
            frame.fill(&Path::circle(led, LED_RADIUS), lit);
            let name = mixer_layout::effect_name_rect(block);
            let color = if effect.on {
                style::palette().text
            } else {
                style::palette().text_disabled
            };
            let shown = effect_name(effect, self.panel.registry);
            if let Some(shown) = fit_width(shown, name.width, size::LABEL) {
                let at = Point::new(name.x, name.center_y());
                label(frame, &shown, at, color, size::LABEL);
            }
            let cross = mixer_layout::remove_rect(block).center();
            centered(
                frame,
                "\u{d7}",
                cross,
                style::palette().text_secondary,
                size::BODY,
            );
            // The arrows, but none past either end of the chain.
            let place = self
                .panel
                .project
                .mixer
                .get(self.panel.state.selected)
                .and_then(|insert| insert.effects.iter().position(|e| e.id == effect.id));
            let count = self
                .panel
                .project
                .mixer
                .get(self.panel.state.selected)
                .map_or(0, |insert| insert.effects.len());
            if place.is_some_and(|place| place > 0) {
                let at = mixer_layout::earlier_rect(block).center();
                centered(
                    frame,
                    "\u{2039}",
                    at,
                    style::palette().text_secondary,
                    size::BODY,
                );
            }
            if place.is_some_and(|place| place + 1 < count) {
                let at = mixer_layout::later_rect(block).center();
                centered(
                    frame,
                    "\u{203a}",
                    at,
                    style::palette().text_secondary,
                    size::BODY,
                );
            }
        }
        let registry = self.panel.registry;
        let words = if let Some(factory) = registry.get(&effect.processor.id) {
            self.draw_native_knobs(frame, effect, factory, (block, count), view);
            None
        } else if let Some(host) = registry.host(&effect.processor.id) {
            self.draw_plugin_knobs(frame, effect, host, (block, count), view);
            // A plugin that is missing has no instance to read, and some have no parameters.
            (count == 0).then_some("No parameters")
        } else {
            Some("Not in this build")
        };
        let row = Rectangle::new(
            Point::new(block.x + BLOCK_INSET, block.y + BLOCK_HEAD),
            Size::new(block.width - 2.0 * BLOCK_INSET, KNOB_ROW_HEIGHT),
        );
        if let Some(words) = words.filter(|_| shows_whole(view, row)) {
            if let Some(shown) = fit_width(words, row.width, size::DENSE) {
                let at = Point::new(row.x, row.center_y());
                label(
                    frame,
                    &shown,
                    at,
                    style::palette().text_secondary,
                    size::DENSE,
                );
            }
        }
    }

    /// A native effect's knobs: each parameter's label, its knob (a choice's button) and value,
    /// the value an automation set last while one governs it, and its automated and linked dots.
    fn draw_native_knobs(
        &self,
        frame: &mut Frame,
        effect: &Effect,
        factory: &dyn ProcessorFactory,
        (block, count): (Rectangle, usize),
        view: Rectangle,
    ) {
        let values = effect_values(effect, self.panel.registry).unwrap_or_default();
        let rate = self.panel.project.sample_rate;
        for (index, (info, &value)) in factory.params().iter().zip(&values).enumerate() {
            let cell = knob_block::knob_cell(block, count, index);
            if !shows_whole(view, cell) {
                continue;
            }
            let target = AutomationTarget::EffectParam {
                effect: effect.id,
                key: info.key.to_owned(),
            };
            let automation = self.panel.automated.get(&target).copied();
            let value = automation.flatten().unwrap_or(value);
            let dots = (
                automation.is_some(),
                self.panel.project.midi_links.get(&target).is_some(),
            );
            draw_param(
                frame,
                cell,
                factory,
                (index, value),
                rate,
                dots,
                Finish::Flat,
            );
        }
    }

    /// A plugin effect's knobs: its first parameters not hidden, read from its host, each with
    /// the plugin's own text for its value.
    fn draw_plugin_knobs(
        &self,
        frame: &mut Frame,
        effect: &Effect,
        host: &dyn ProcessorHost,
        (block, count): (Rectangle, usize),
        view: Rectangle,
    ) {
        let owner = ProcessorOwner::Effect(effect.id);
        let params: Vec<PluginParam> = host
            .params(owner)
            .into_iter()
            .filter(|param| !param.hidden)
            .take(count.min(PLUGIN_KNOBS))
            .collect();
        for (index, param) in params.iter().enumerate() {
            let cell = knob_block::knob_cell(block, count, index);
            if !shows_whole(view, cell) {
                continue;
            }
            let value = host.value(owner, param.id).unwrap_or(param.default);
            let text = host
                .value_text(owner, param.id, value)
                .unwrap_or_else(|| format!("{value:.2}"));
            // Only shown, so f32 is plenty.
            let knob = fraction(value as f32, param.min as f32, param.max as f32);
            draw_knob_cell(
                frame,
                cell,
                (&param.name, &text),
                Some(knob),
                (false, false),
                Finish::Flat,
            );
        }
    }

    /// The pop-out grip, the meters, the clip light, a refusal's flash and notice and the value
    /// under the cursor.
    fn draw_overlay(&self, frame: &mut Frame, cursor: Option<Point>) {
        let (width, desk) = (frame.width(), frame.height());
        // Docked, the pop-out button's mini grip at the header's right end
        // ([`mixer_layout::pop_out_rect`]), lit under the pointer, so drawn every frame.
        if self.panel.docked {
            let button = mixer_layout::pop_out_rect(width);
            let over = cursor.is_some_and(|at| button.contains(at));
            pop_out::draw_glyph(frame, button.position(), pop_out::glyph_color(over));
        }
        let area = mixer_layout::effects_left(width, self.panel.last());
        let strips = Rectangle::new(
            Point::new(0.0, HEADER_HEIGHT),
            Size::new(area, desk - HEADER_HEIGHT),
        );
        let state = self.panel.state;
        let flashing = state.flashing(self.panel.now);
        // Why the meters and the flash are cut to the strips' area as well as clipped: the
        // software renderer draws a clipped frame's shapes whole, so a strip under the effects
        // area would show its meter over the area.
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
                        style::palette().led_off
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
            label(frame, notice, at, style::WARNING, size::LABEL);
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
        let hit = mixer_layout::hit_test(at, self.desk(size), &self.panel.chain())?;
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
        let area = mixer_layout::effects_left(width, self.panel.last());
        Some((tip_rect(left, at.y, area), value))
    }
}

impl Desk<'_> {
    /// Whether knob `knob` of the selected insert's effect at `block` is a choice's button.
    fn is_choice(&self, block: usize, knob: usize) -> bool {
        let panel = &self.panel;
        panel
            .project
            .mixer
            .get(panel.state.selected)
            .and_then(|insert| insert.effects.get(block))
            .and_then(|effect| panel.registry.get(&effect.processor.id))
            .is_some_and(|factory| is_choice(factory, knob))
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
            // A block held by its name follows the pointer anywhere, and is let go anywhere.
            Event::Mouse(mouse::Event::CursorMoved { .. })
                if self.panel.state.reorder.is_some() =>
            {
                cursor
                    .position_from(bounds.position())
                    .map(|at| send(MixerMessage::ReorderMoved(at)).and_capture())
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                if self.panel.state.reorder.is_some() =>
            {
                Some(send(MixerMessage::ReorderEnded).and_capture())
            }
            // The value under the cursor follows it, and the hint bar names what is under it.
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let desk = self.desk(bounds.size());
                let chain = self.panel.chain();
                // The pop-out button shows (and names itself) only while the mixer is docked.
                let now = cursor
                    .position_in(bounds)
                    .and_then(|at| mixer_layout::hit_test(at, desk, &chain))
                    .filter(|&hit| self.panel.docked || hit != mixer_layout::Hit::PopOut)
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
        if self.panel.state.reorder.is_some() {
            return mouse::Interaction::Grabbing;
        }
        let Some(at) = cursor.position_in(bounds) else {
            return mouse::Interaction::default();
        };
        match mixer_layout::hit_test(at, self.desk(bounds.size()), &self.panel.chain()) {
            Some(Hit::EffectName(_)) => mouse::Interaction::Grab,
            Some(Hit::Fader(_)) => mouse::Interaction::ResizingVertically,
            Some(Hit::Knob { block, knob }) if self.is_choice(block, knob) => {
                mouse::Interaction::Pointer
            }
            Some(Hit::Pan(_) | Hit::Send(_) | Hit::Knob { .. }) => {
                mouse::Interaction::ResizingHorizontally
            }
            Some(Hit::Strip(_) | Hit::Name(_) | Hit::Block(_)) | None => {
                mouse::Interaction::default()
            }
            Some(_) => mouse::Interaction::Pointer,
        }
    }
}

/// The marks at the effects area's right edge while lines are scrolled out of view: a small
/// triangle pointing up at the view's top while lines are above it, down at its foot while more
/// follow, so the wheel's reach is seen.
fn draw_more_marks(frame: &mut Frame, view: Rectangle, width: f32, above: bool, below: bool) {
    let x = width - MARKS_WIDTH / 2.0;
    let triangle = |tip: f32, base: f32| {
        Path::new(|path| {
            path.move_to(Point::new(x, tip));
            path.line_to(Point::new(x + 4.0, base));
            path.line_to(Point::new(x - 4.0, base));
            path.close();
        })
    };
    if above {
        let top = view.y + 6.0;
        frame.fill(
            &triangle(top - 3.0, top + 3.0),
            style::palette().text_secondary,
        );
    }
    if below {
        let foot = view.y + view.height - 6.0;
        frame.fill(
            &triangle(foot + 3.0, foot - 3.0),
            style::palette().text_secondary,
        );
    }
}

/// The name an effect's block shows: a native effect's own, a plugin's saved name, else its id.
pub(crate) fn effect_name<'a>(effect: &'a Effect, registry: &'a Registry) -> &'a str {
    registry
        .get(&effect.processor.id)
        .map_or(effect.processor.display_name(), |factory| factory.name())
}

/// The place a dragged block left, while it shows whole in `view`: a dim outline.
fn draw_gap(frame: &mut Frame, gap: Rectangle, view: Rectangle) {
    if !shows_whole(view, gap) {
        return;
    }
    frame.stroke_rectangle(
        Point::new(gap.x + 0.5, gap.y + 0.5),
        Size::new(gap.width - 1.0, gap.height - 1.0),
        Stroke::default()
            .with_color(style::palette().text_secondary)
            .with_width(1.0),
    );
}

/// The add tile at `tile`, as far as it shows in `view` ([`mixer_layout::shown_tile`]): a dim
/// outline round "+ Add effect".
fn draw_add_tile(frame: &mut Frame, tile: Rectangle, view: Rectangle) {
    let Some(tile) = mixer_layout::shown_tile(tile, view) else {
        return;
    };
    frame.stroke_rectangle(
        Point::new(tile.x + 0.5, tile.y + 0.5),
        Size::new(tile.width - 1.0, tile.height - 1.0),
        Stroke::default()
            .with_color(style::palette().text_secondary)
            .with_width(1.0),
    );
    centered(
        frame,
        "+ Add effect",
        tile.center(),
        style::palette().text_secondary,
        size::LABEL,
    );
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

/// The words of an open effect menu's rows, top to bottom, one per row [`Menu::rows`] counts:
/// under the add tile the native effects by name and the plugin row; under an effect Remove,
/// and for a plugin effect its parameters (its parameter window).
fn menu_rows(menu: Menu, registry: &Registry, project: &Project) -> Vec<&'static str> {
    match menu {
        Menu::Add => ADD_MENU
            .iter()
            .map(|id| registry.get(id).map_or(*id, |factory| factory.name()))
            .chain([PLUGIN_EFFECT_ROW])
            .collect(),
        Menu::Effect { effect } => {
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
/// the strip's top), and no further right than `limit`, the effects area's left edge. Why inside
/// the strip: a tip beside the cursor covered the next strip's controls. Why the limit: a strip
/// running under the effects area shows only its left part there, and the area stays whole.
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
    frame.fill_rectangle(rect.position(), rect.size(), style::palette().lcd);
    if let Some(shown) = fit_width(value, rect.width, size::DENSE) {
        centered(
            frame,
            &shown,
            rect.center(),
            style::palette().lcd_text,
            size::DENSE,
        );
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
            docked: true,
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

    // Every row of an effect menu holds its words on one line, 10 px of the row either side of
    // them (`menu::row`). Why: at the menus' 13 px, "Compressor (Pressure4)" ran past the 120 px
    // the slot menu had in the canvas.
    #[test]
    fn an_effect_menus_rows_hold_their_words() {
        use crate::ui::text_fit::text_width;
        let project = Project::load_json(P5_GAIN).expect("loads");
        let registry = Registry::native();
        let added = menu_rows(Menu::Add, &registry, &project);
        assert_eq!(added.len(), ADD_MENU.len() + 1);
        for words in added
            .into_iter()
            .chain(EFFECT_MENU)
            .chain([PLUGIN_PARAMS_ROW])
        {
            let needed = text_width(words, size::BODY) + 2.0 * 10.0;
            println!("{words}: {needed} px");
            assert!(
                needed <= mixer_layout::MENU_WIDTH,
                "{words} needs {needed} px"
            );
        }
    }

    // At 1000 px strip 10 runs under the effects area: its left edge is at 12 + 62·10 = 632 and
    // the area's at 1000 − 332 = 668 (p5_gain's sixteen inserts end at 1068). The tip over its
    // fader starts 2 px inside the strip and stops at the area's edge.
    #[test]
    fn a_tip_stops_at_the_effects_area() {
        let project = Project::load_json(P5_GAIN).expect("loads");
        let state = MixerState::new(1000.0, mixer_layout::DEFAULT_HEIGHT);
        let left = mixer_layout::strip_left(InsertId(10), 0.0);
        assert_eq!(left, 632.0);
        let fader = mixer_layout::fader_rect(left, state.size().height).center();
        let (rect, _) =
            tip_at(&project, &state, &BTreeMap::new(), fader).expect("a tip over the fader");
        assert_eq!(rect.x, 634.0);
        assert!(
            rect.x + rect.width <= 668.0,
            "{rect:?} reaches under the effects area"
        );
    }
}
