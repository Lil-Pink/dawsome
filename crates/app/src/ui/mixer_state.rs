//! The mixer's UI state: the selected insert and effect, how far the strips are scrolled, an
//! open slot menu, a rename in progress, the meters as shown, and the notice for a refused
//! send. It turns the canvas's presses into edits and drags through [`mixer_layout`] and never
//! changes the document itself.
//!
//! Clicks commit on press, as one edit each; drags (fader, pan, send, parameter) are run by the
//! app, live while they move and one edit on release.

use std::collections::BTreeSet;
use std::ops::RangeInclusive;
use std::time::{Duration, Instant};

use dawsome_engine::dsp::airwindows::{density, pressure4};
use dawsome_engine::dsp::{delay, eq, limiter, reverb};
use dawsome_engine::edit::Edit;
use dawsome_engine::processor::{param_values, ParamInfo, ProcessorFactory, Registry};
use dawsome_engine::project::{
    AutomationTarget, ChannelId, Effect, EffectId, Insert, InsertId, ProcessorRef, Project,
    EFFECT_SLOTS, FADER_RANGE, PAN_RANGE, SEND_RANGE, UNITY_FADER,
};
use dawsome_engine::transport::ParamTarget;
use iced::keyboard::Modifiers;
use iced::{mouse, Point, Size};

use crate::ui::mixer_layout::{self, Hit, SCROLL_PER_NOTCH};

/// The effects the add menu offers, in menu order: EQ, Delay, Reverb, Compressor, Saturator,
/// Limiter. Why the limiter last: the rows before it keep the places they had.
pub const ADD_MENU: [&str; 6] = [
    eq::ID,
    delay::ID,
    reverb::ID,
    pressure4::ID,
    density::ID,
    limiter::ID,
];

/// The add menu's last row, after the native effects: it opens the plugin picker.
pub const PLUGIN_EFFECT_ROW: &str = "Plugin effect\u{2026}";

/// The rows of a filled slot's menu.
pub const EFFECT_MENU: [&str; 3] = ["Remove", "Move up", "Move down"];

/// A filled slot's menu's last row, for a plugin effect: its parameter window, named as a plugin
/// channel's menu names it (it read "Generic parameters" while it opened a strip in the main
/// window).
pub const PLUGIN_PARAMS_ROW: &str = "Parameters\u{2026}";

/// How long a refused route button flashes, and how long the header names the loop.
pub const FLASH: Duration = Duration::from_millis(300);
pub const NOTICE: Duration = Duration::from_secs(3);

/// How fast a meter falls, and how long it holds its peak mark.
pub const FALL_DB_PER_SECOND: f32 = 24.0;
pub const PEAK_HOLD: Duration = Duration::from_millis(1500);
/// A meter below this shows nothing (the −60 dB floor).
pub const METER_FLOOR: f32 = 0.001;

/// What "Mixer insert…" gives a channel ([`own_insert`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnInsert {
    /// It plays into this insert and no other channel does: nothing to change.
    Own(InsertId),
    /// It plays into the master or an insert another channel plays into too: this insert is the
    /// first free one.
    Free(InsertId),
    /// It has none of its own and every insert is taken.
    NoneFree,
}

/// The insert "Mixer insert…" plays `channel` into, the channel menu's and the instrument
/// window's easy way to a strip of its own: the one it plays into when no other channel does,
/// else the first insert as a new mixer has it, free for it (no channel plays into it, no insert
/// routes or sends into it, its output goes to the master alone, it holds no effect and its name
/// is still its number). `None` for a channel that is not there. Why "free" and not the insert after it, as the header's + does: a
/// second channel given a strip the same way would land on the first one's.
pub fn own_insert(project: &Project, channel: ChannelId) -> Option<OwnInsert> {
    let now = project.channels.get(&channel)?.insert;
    let played_into = |insert: InsertId, besides: Option<ChannelId>| {
        project
            .channels
            .iter()
            .any(|(id, other)| Some(*id) != besides && other.insert == insert)
    };
    if !now.is_master() && !played_into(now, Some(channel)) {
        return Some(OwnInsert::Own(now));
    }
    let fed = |insert: InsertId| {
        project
            .mixer
            .ids()
            .filter(|&from| from != insert)
            .filter_map(|from| project.mixer.get(from))
            .any(|from| from.routes.contains_key(&insert))
    };
    let free = project
        .mixer
        .ids()
        .filter(|id| !id.is_master())
        .find(|&id| {
            project.mixer.get(id).is_some_and(|insert| {
                insert.effects.is_empty()
                    && insert.name == id.to_string()
                    && insert.routes == Insert::numbered(id).routes
                    && !played_into(id, None)
                    && !fed(id)
            })
        });
    Some(free.map_or(OwnInsert::NoneFree, OwnInsert::Free))
}

/// What the mixer canvas and its slot menu report. Points are in the canvas's own coordinates
/// (see [`mixer_layout`]), so a test that sends them drives the real mapping.
#[derive(Debug, Clone, PartialEq)]
pub enum MixerMessage {
    /// A press: `double` when it repeats the last press quickly in place.
    Pressed {
        at: Point,
        button: mouse::Button,
        double: bool,
    },
    Scrolled {
        at: Point,
        delta: mouse::ScrollDelta,
    },
    /// The canvas was laid out at a new size.
    Resized(Size),
    RenameEdited(String),
    RenameSubmitted,
    /// A row of the open slot menu, counted from its top: what the row offers is done.
    MenuChosen(usize),
}

/// What a mixer gesture asks the app to do.
#[derive(Debug, Clone, PartialEq)]
pub enum MixerCommand {
    /// Apply this as one undo step.
    Edit(Edit),
    /// Start dragging this control.
    Drag(MixerDrag),
    /// An insert name went into editing: give its text box the keyboard.
    FocusRename,
    /// Open the plugin picker for this empty slot.
    PickPlugin { insert: InsertId, slot: u8 },
    /// Open this plugin effect's window.
    OpenPlugin(EffectId),
    /// Open this plugin effect's parameter window.
    PluginParams(EffectId),
    /// Start dragging parameter row `index` of this plugin effect (among its parameters that
    /// are not hidden).
    DragPlugin { effect: EffectId, index: usize },
    /// A right press on a control at `at`: the automation menu for what it sets, its value
    /// now normalized over its range.
    Automate {
        target: AutomationTarget,
        value: f32,
        at: Point,
    },
    /// A right press at `at` on parameter row `index` of this plugin effect (among its
    /// parameters that are not hidden): the automation menu for it, which the app finds by
    /// asking the plugin.
    AutomatePlugin {
        effect: EffectId,
        index: usize,
        at: Point,
    },
}

/// A mixer control a drag moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MixerDrag {
    Fader(InsertId),
    Pan(InsertId),
    Send {
        from: InsertId,
        to: InsertId,
    },
    /// Parameter `index` of an effect, on its slider.
    Param {
        effect: EffectId,
        index: usize,
    },
}

/// Pixels of drag that sweep a fader, pan or send from one end to the other.
const SWEEP_PIXELS: f32 = 200.0;

impl MixerDrag {
    /// The control's value now; `None` once it is gone from the document.
    pub fn value(self, project: &Project, registry: &Registry) -> Option<f32> {
        let mixer = &project.mixer;
        match self {
            MixerDrag::Fader(id) => mixer.get(id).map(|insert| insert.fader),
            MixerDrag::Pan(id) => mixer.get(id).map(|insert| insert.pan),
            MixerDrag::Send { from, to } => mixer.get(from)?.routes.get(&to).copied(),
            MixerDrag::Param { effect, index } => {
                let (_, effect) = mixer.effect(effect)?;
                effect_values(effect, registry)?.get(index).copied()
            }
        }
    }

    /// The parameter's description, for an effect parameter.
    fn param_info(self, project: &Project, registry: &Registry) -> Option<ParamInfo> {
        let MixerDrag::Param { effect, index } = self else {
            return None;
        };
        let (_, effect) = project.mixer.effect(effect)?;
        registry
            .get(&effect.processor.id)?
            .params()
            .get(index)
            .copied()
    }

    /// The values the control sweeps.
    pub fn range(self, project: &Project, registry: &Registry) -> RangeInclusive<f32> {
        match self {
            MixerDrag::Fader(_) => FADER_RANGE,
            MixerDrag::Pan(_) => PAN_RANGE,
            MixerDrag::Send { .. } => SEND_RANGE,
            MixerDrag::Param { .. } => self
                .param_info(project, registry)
                .map_or(0.0..=1.0, |info| info.min..=info.max),
        }
    }

    /// `value` as the control takes it: inside its range, whole for a whole parameter.
    pub fn clamp(self, value: f32, project: &Project, registry: &Registry) -> f32 {
        match self.param_info(project, registry) {
            Some(info) => info.clamp(value),
            None => {
                let range = self.range(project, registry);
                value.clamp(*range.start(), *range.end())
            }
        }
    }

    /// Pixels of drag from one end of the range to the other.
    pub fn sweep_pixels(self) -> f32 {
        match self {
            MixerDrag::Param { .. } => mixer_layout::SLIDER_WIDTH,
            _ => SWEEP_PIXELS,
        }
    }

    /// Whether the control moves left to right (a slider) rather than up and down.
    pub fn horizontal(self) -> bool {
        matches!(self, MixerDrag::Param { .. })
    }

    /// The edit that sets the control to `value`.
    pub fn edit(self, value: f32, project: &Project, registry: &Registry) -> Option<Edit> {
        Some(match self {
            MixerDrag::Fader(insert) => Edit::SetInsertVolume {
                insert,
                fader: value,
            },
            MixerDrag::Pan(insert) => Edit::SetInsertPan { insert, pan: value },
            MixerDrag::Send { from, to } => Edit::SetSend {
                from,
                to,
                amount: Some(value),
            },
            MixerDrag::Param { effect, .. } => Edit::SetEffectParam {
                effect,
                key: self.param_info(project, registry)?.key.to_owned(),
                value: Some(value),
            },
        })
    }

    /// What an automation of the control sets.
    pub fn automation(self, project: &Project, registry: &Registry) -> Option<AutomationTarget> {
        Some(match self {
            MixerDrag::Fader(id) => AutomationTarget::InsertFader(id),
            MixerDrag::Pan(id) => AutomationTarget::InsertPan(id),
            MixerDrag::Send { from, to } => AutomationTarget::Send { from, to },
            MixerDrag::Param { effect, .. } => AutomationTarget::EffectParam {
                effect,
                key: self.param_info(project, registry)?.key.to_owned(),
            },
        })
    }

    /// Where the live value goes while the control moves.
    pub fn target(self) -> ParamTarget {
        match self {
            MixerDrag::Fader(id) => ParamTarget::InsertFader(id),
            MixerDrag::Pan(id) => ParamTarget::InsertPan(id),
            MixerDrag::Send { from, to } => ParamTarget::Send { from, to },
            MixerDrag::Param { effect, index } => ParamTarget::Effect {
                effect,
                // At most a handful of parameters; a u16 holds every index.
                param: u16::try_from(index).unwrap_or(u16::MAX),
            },
        }
    }
}

/// The values an effect's parameters have, in its processor's order; `None` for a processor
/// this build does not know.
pub fn effect_values(effect: &Effect, registry: &Registry) -> Option<Vec<f32>> {
    let factory = registry.get(&effect.processor.id)?;
    Some(param_values(factory.params(), &effect.processor.params))
}

/// An open slot menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Menu {
    /// Under an empty slot: the effects it can take.
    Add { slot: u8 },
    /// Under a filled slot: what can be done with its effect.
    Effect { effect: EffectId, slot: u8 },
}

impl Menu {
    pub fn slot(self) -> u8 {
        match self {
            Menu::Add { slot } | Menu::Effect { slot, .. } => slot,
        }
    }

    /// How many rows the menu has in `project`: the native effects and the plugin row under an
    /// empty slot; Remove, Move up and Move down under a filled one, and for a plugin effect its
    /// parameters too.
    pub fn rows(self, project: &Project) -> usize {
        match self {
            Menu::Add { .. } => ADD_MENU.len() + 1,
            Menu::Effect { effect, .. } => {
                EFFECT_MENU.len() + usize::from(is_plugin(project, effect))
            }
        }
    }
}

/// Whether effect `id` is a plugin.
pub fn is_plugin(project: &Project, id: EffectId) -> bool {
    project
        .mixer
        .effect(id)
        .is_some_and(|(_, effect)| effect.processor.format().is_some())
}

/// A send the model refused, as the mixer shows it: the button flashes, the header explains.
#[derive(Debug, Clone, PartialEq)]
struct Refusal {
    button: InsertId,
    message: String,
    at: Instant,
}

/// The meters as shown: each side falls from its last peak at [`FALL_DB_PER_SECOND`] and keeps
/// a peak mark for [`PEAK_HOLD`]; the master's clip light stays on, once lit, until clicked.
#[derive(Debug, Clone, Default)]
pub struct Meters {
    shown: Vec<[f32; 2]>,
    held: Vec<[(f32, Option<Instant>); 2]>,
    /// The master went past 1.0 (0 dB) since the light was last cleared.
    pub clip: bool,
    last: Option<Instant>,
}

impl Meters {
    /// Takes in the peaks each insert reached since the last update, by insert ID.
    pub fn update(&mut self, peaks: &[[f32; 2]], now: Instant) {
        let elapsed = self.last.map_or(0.0, |last| {
            now.saturating_duration_since(last).as_secs_f32()
        });
        self.last = Some(now);
        let fall = 10f32.powf(-FALL_DB_PER_SECOND * elapsed / 20.0);
        self.shown.resize(peaks.len(), [0.0; 2]);
        self.held.resize(peaks.len(), [(0.0, None); 2]);
        for ((shown, held), peak) in self.shown.iter_mut().zip(&mut self.held).zip(peaks) {
            for side in 0..2 {
                let level = peak[side].max(shown[side] * fall);
                shown[side] = if level < METER_FLOOR / 10.0 {
                    0.0
                } else {
                    level
                };
                let (mark, since) = held[side];
                let expired = since.is_none_or(|since| now.duration_since(since) > PEAK_HOLD);
                if level >= mark || expired {
                    held[side] = (level, Some(now));
                }
            }
        }
        if peaks
            .first()
            .is_some_and(|master| master.iter().any(|&peak| peak > 1.0))
        {
            self.clip = true;
        }
    }

    /// The level shown for `id`, left then right.
    pub fn level(&self, id: InsertId) -> [f32; 2] {
        self.shown
            .get(usize::from(id.0))
            .copied()
            .unwrap_or([0.0; 2])
    }

    /// The peak mark shown for `id`.
    pub fn peak_mark(&self, id: InsertId) -> [f32; 2] {
        self.held
            .get(usize::from(id.0))
            .map_or([0.0; 2], |held| [held[0].0, held[1].0])
    }

    /// Whether any meter still shows something, so the meters need more updates.
    pub fn moving(&self) -> bool {
        let lit = |level: &f32| *level >= METER_FLOOR;
        self.shown.iter().flatten().any(lit)
            || self.held.iter().flatten().any(|(mark, _)| lit(mark))
    }
}

/// The mixer's state between frames.
#[derive(Debug, Clone)]
pub struct MixerState {
    pub selected: InsertId,
    /// The effect whose parameters the slot panel shows.
    pub selected_effect: Option<EffectId>,
    /// How far the strips are scrolled to the left, in pixels.
    pub scroll: f32,
    pub menu: Option<Menu>,
    /// The insert being renamed, and the name typed so far.
    pub rename: Option<(InsertId, String)>,
    pub meters: Meters,
    refusal: Option<Refusal>,
    /// The canvas's size, as it last reported it.
    size: Size,
    /// The effect whose parameter rows the wheel scrolled, and the parameter in its first
    /// row; another effect shows its first parameter first.
    params_scroll: Option<(EffectId, usize)>,
}

impl MixerState {
    /// The mixer with the master selected, a panel `height` tall and `width` wide.
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            selected: InsertId::MASTER,
            selected_effect: None,
            scroll: 0.0,
            menu: None,
            rename: None,
            meters: Meters::default(),
            refusal: None,
            size: Size::new(width, mixer_layout::desk_height(height)),
            params_scroll: None,
        }
    }

    pub fn size(&self) -> Size {
        self.size
    }

    /// The desk is `height` tall from now on: the divider over the mixer moved, so a press
    /// before the canvas reports its new size finds what is drawn.
    pub fn set_desk_height(&mut self, height: f32) {
        self.size.height = height;
    }

    /// The parameter the slot panel's first parameter row shows.
    pub fn params_top(&self) -> usize {
        match (self.params_scroll, self.selected_effect) {
            (Some((scrolled, top)), Some(shown)) if scrolled == shown => top,
            _ => 0,
        }
    }

    /// The desk as a press finds it, for a project whose highest insert is `last`.
    pub fn desk(&self, last: InsertId) -> mixer_layout::Desk {
        mixer_layout::Desk {
            size: self.size,
            scroll: self.scroll,
            last,
            params_top: self.params_top(),
        }
    }

    /// Selects insert `id`, keeping the shown effect only if it is in that insert.
    pub fn select(&mut self, id: InsertId, project: &Project) {
        if project.mixer.get(id).is_none() {
            return;
        }
        self.selected = id;
        let in_insert = |effect: &EffectId| {
            project
                .mixer
                .effect(*effect)
                .is_some_and(|(insert, _)| insert == id)
        };
        self.selected_effect = self.selected_effect.filter(in_insert);
    }

    /// Brings the state in line with a changed document: what is gone is let go of.
    pub fn refresh(&mut self, project: &Project) {
        let mixer = &project.mixer;
        if mixer.get(self.selected).is_none() {
            self.selected = InsertId::MASTER;
        }
        self.select(self.selected, project);
        if let Some(Menu::Effect { effect, .. }) = self.menu {
            if mixer.effect(effect).is_none() {
                self.menu = None;
            }
        }
        if let Some((id, _)) = &self.rename {
            if mixer.get(*id).is_none() {
                self.rename = None;
            }
        }
        let last = mixer.ids().last().unwrap_or(InsertId::MASTER);
        self.scroll = self
            .scroll
            .clamp(0.0, mixer_layout::max_scroll(last, self.size.width));
    }

    /// Esc: ends a rename without committing it, or closes a menu. Returns whether it did
    /// either.
    pub fn escape(&mut self) -> bool {
        self.rename.take().is_some() || self.menu.take().is_some()
    }

    /// Shows that the model refused a send to `button`'s insert, saying why.
    pub fn refuse(&mut self, button: InsertId, message: String, now: Instant) {
        self.refusal = Some(Refusal {
            button,
            message,
            at: now,
        });
    }

    /// The route button flashing for a refusal, if one is.
    pub fn flashing(&self, now: Instant) -> Option<InsertId> {
        self.refusal
            .as_ref()
            .filter(|refusal| now.duration_since(refusal.at) < FLASH)
            .map(|refusal| refusal.button)
    }

    /// The header's notice for a refusal, while it shows.
    pub fn notice(&self, now: Instant) -> Option<&str> {
        self.refusal
            .as_ref()
            .filter(|refusal| now.duration_since(refusal.at) < NOTICE)
            .map(|refusal| refusal.message.as_str())
    }

    /// Whether something on the mixer still changes with time: a meter, a flash, a notice.
    pub fn animating(&self, now: Instant) -> bool {
        self.meters.moving() || self.notice(now).is_some()
    }

    /// Carries out a canvas message; returns what the app must do.
    pub fn update(&mut self, message: MixerMessage, context: &Context<'_>) -> Option<MixerCommand> {
        match message {
            MixerMessage::Pressed { at, button, double } => self.press(at, button, double, context),
            MixerMessage::Scrolled { at, delta } => self.scrolled(at, delta, context),
            MixerMessage::Resized(size) => {
                self.size = size;
                self.refresh(context.project);
                None
            }
            MixerMessage::RenameEdited(name) => {
                if let Some((_, typed)) = &mut self.rename {
                    *typed = name;
                }
                None
            }
            MixerMessage::RenameSubmitted => {
                let (insert, name) = self.rename.take()?;
                Some(MixerCommand::Edit(Edit::RenameInsert { insert, name }))
            }
            MixerMessage::MenuChosen(row) => {
                let menu = self.menu.take()?;
                if row >= menu.rows(context.project) {
                    return None;
                }
                self.menu_choice(menu, row, context.project)
            }
        }
    }

    fn last(&self, project: &Project) -> InsertId {
        project.mixer.ids().last().unwrap_or(InsertId::MASTER)
    }

    fn press(
        &mut self,
        at: Point,
        button: mouse::Button,
        double: bool,
        context: &Context<'_>,
    ) -> Option<MixerCommand> {
        let project = context.project;
        // Any press on the mixer closes its slot menu: a press on the menu is the menu's own,
        // taken on the window's top layer before it could reach the mixer.
        self.menu = None;
        let hit = mixer_layout::hit_test(at, self.desk(self.last(project)))?;
        let mixer = &project.mixer;
        let routes_here = |to: InsertId| {
            to != self.selected
                && mixer
                    .get(self.selected)
                    .is_some_and(|insert| insert.routes.contains_key(&to))
        };
        let right = button == mouse::Button::Right;
        match hit {
            Hit::RouteDown => self.route_channel(-1, context),
            Hit::RouteUp => self.route_channel(1, context),
            Hit::RouteLabel => None,
            Hit::Led(id) if right || context.modifiers.control() => {
                let mut set = mixer.solo.clone();
                if right && context.modifiers.shift() {
                    if !set.remove(&id) {
                        set.insert(id);
                    }
                } else if set == BTreeSet::from([id]) {
                    set.clear();
                } else {
                    set = BTreeSet::from([id]);
                }
                Some(MixerCommand::Edit(Edit::SoloInsert { set }))
            }
            Hit::Led(id) => {
                let mute = !mixer.get(id)?.mute;
                Some(MixerCommand::Edit(Edit::MuteInsert { insert: id, mute }))
            }
            Hit::ClipLight => {
                self.meters.clip = false;
                None
            }
            Hit::Slot(slot) => self.press_slot(slot, button, project),
            Hit::Fader(id) if right => self.automate(MixerDrag::Fader(id), at, context),
            Hit::Pan(id) if right => self.automate(MixerDrag::Pan(id), at, context),
            Hit::Send(id) if right && routes_here(id) => {
                let send = MixerDrag::Send {
                    from: self.selected,
                    to: id,
                };
                self.automate(send, at, context)
            }
            Hit::Param(index)
                if right
                    && self
                        .selected_effect
                        .is_some_and(|effect| !is_plugin(project, effect)) =>
            {
                let effect = self.selected_effect?;
                self.automate(MixerDrag::Param { effect, index }, at, context)
            }
            Hit::Param(index) if right => {
                let effect = self.selected_effect?;
                Some(MixerCommand::AutomatePlugin { effect, index, at })
            }
            _ if right => None,
            Hit::Pan(id) => {
                self.select(id, project);
                Some(if double {
                    MixerCommand::Edit(Edit::SetInsertPan {
                        insert: id,
                        pan: 0.0,
                    })
                } else {
                    MixerCommand::Drag(MixerDrag::Pan(id))
                })
            }
            Hit::Fader(id) => {
                self.select(id, project);
                Some(if double {
                    MixerCommand::Edit(Edit::SetInsertVolume {
                        insert: id,
                        fader: UNITY_FADER,
                    })
                } else {
                    MixerCommand::Drag(MixerDrag::Fader(id))
                })
            }
            Hit::Route(id) if id != self.selected => {
                let amount = if routes_here(id) { None } else { Some(1.0) };
                Some(MixerCommand::Edit(Edit::SetSend {
                    from: self.selected,
                    to: id,
                    amount,
                }))
            }
            Hit::Send(id) if routes_here(id) => Some(MixerCommand::Drag(MixerDrag::Send {
                from: self.selected,
                to: id,
            })),
            Hit::Name(id) if double => {
                self.select(id, project);
                self.rename = Some((id, mixer.get(id)?.name.clone()));
                Some(MixerCommand::FocusRename)
            }
            Hit::Name(id) | Hit::Strip(id) | Hit::Route(id) | Hit::Send(id) => {
                self.select(id, project);
                None
            }
            Hit::Param(index)
                if self
                    .selected_effect
                    .is_some_and(|id| is_plugin(project, id)) =>
            {
                let effect = self.selected_effect?;
                Some(MixerCommand::DragPlugin { effect, index })
            }
            Hit::Param(index) => {
                let effect = self.selected_effect?;
                let (_, shown) = mixer.effect(effect)?;
                let info = context
                    .registry
                    .get(&shown.processor.id)?
                    .params()
                    .get(index)?;
                Some(if double {
                    MixerCommand::Edit(Edit::SetEffectParam {
                        effect,
                        key: info.key.to_owned(),
                        value: None,
                    })
                } else {
                    MixerCommand::Drag(MixerDrag::Param { effect, index })
                })
            }
        }
    }

    /// The automation menu for `control`, pressed at `at`.
    fn automate(
        &self,
        control: MixerDrag,
        at: Point,
        context: &Context<'_>,
    ) -> Option<MixerCommand> {
        let (project, registry) = (context.project, context.registry);
        let target = control.automation(project, registry)?;
        let range = control.range(project, registry);
        let span = range.end() - range.start();
        let value = control.value(project, registry)?;
        let value = if span > 0.0 {
            (value - range.start()) / span
        } else {
            0.0
        };
        Some(MixerCommand::Automate { target, value, at })
    }

    /// A press on slot row `slot` of the selected insert: a filled slot shows its effect's
    /// parameters (and opens a plugin's window), or its menu on the right button; an empty one
    /// opens the add menu.
    fn press_slot(
        &mut self,
        slot: u8,
        button: mouse::Button,
        project: &Project,
    ) -> Option<MixerCommand> {
        let insert = project.mixer.get(self.selected)?;
        let filled = insert.effects.iter().find(|effect| effect.slot == slot);
        match (filled, button) {
            (Some(effect), mouse::Button::Right) => {
                self.menu = Some(Menu::Effect {
                    effect: effect.id,
                    slot,
                });
            }
            (Some(effect), _) => {
                self.selected_effect = Some(effect.id);
                if effect.processor.format().is_some() {
                    return Some(MixerCommand::OpenPlugin(effect.id));
                }
            }
            (None, mouse::Button::Left) => self.menu = Some(Menu::Add { slot }),
            (None, _) => {}
        }
        None
    }

    fn menu_choice(&mut self, menu: Menu, row: usize, project: &Project) -> Option<MixerCommand> {
        match menu {
            Menu::Add { slot } if row == ADD_MENU.len() => Some(MixerCommand::PickPlugin {
                insert: self.selected,
                slot,
            }),
            Menu::Add { slot } => {
                let id = project.mixer.next_effect_id;
                self.selected_effect = Some(id);
                Some(MixerCommand::Edit(Edit::AddEffect {
                    insert: self.selected,
                    slot,
                    id,
                    processor: ProcessorRef::new(*ADD_MENU.get(row)?),
                }))
            }
            Menu::Effect { effect, .. } if row == EFFECT_MENU.len() => {
                Some(MixerCommand::PluginParams(effect))
            }
            Menu::Effect { effect, slot } => {
                let target = match row {
                    0 => return Some(MixerCommand::Edit(Edit::RemoveEffect { id: effect })),
                    1 => slot.checked_sub(1)?,
                    _ => slot.checked_add(1).filter(|&next| next < EFFECT_SLOTS)?,
                };
                Some(MixerCommand::Edit(Edit::MoveEffect {
                    effect,
                    slot: target,
                }))
            }
        }
    }

    /// The selected channel moved `step` inserts along (clamped to the mixer), and that insert
    /// selected.
    fn route_channel(&mut self, step: i32, context: &Context<'_>) -> Option<MixerCommand> {
        let project = context.project;
        let channel = context.channel?;
        let now = project.channels.get(&channel)?.insert;
        let last = self.last(project);
        let moved = (i32::from(now.0) + step).clamp(0, i32::from(last.0));
        let insert = InsertId(u16::try_from(moved).ok()?);
        if insert == now {
            return None;
        }
        self.select(insert, project);
        Some(MixerCommand::Edit(Edit::RouteChannel { channel, insert }))
    }

    fn scrolled(
        &mut self,
        at: Point,
        delta: mouse::ScrollDelta,
        context: &Context<'_>,
    ) -> Option<MixerCommand> {
        let rise = match delta {
            mouse::ScrollDelta::Lines { y, .. } | mouse::ScrollDelta::Pixels { y, .. } => y,
        };
        if rise == 0.0 {
            return None;
        }
        if mixer_layout::route_label_rect().contains(at) {
            return self.route_channel(if rise > 0.0 { 1 } else { -1 }, context);
        }
        if at.x < mixer_layout::slot_panel_left(self.size.width) {
            let last = self.last(context.project);
            let max = mixer_layout::max_scroll(last, self.size.width);
            let notches = if rise > 0.0 { -1.0 } else { 1.0 };
            self.scroll = (self.scroll + notches * SCROLL_PER_NOTCH).clamp(0.0, max);
        } else if at.y >= mixer_layout::PARAMS_TOP {
            self.scroll_params(rise > 0.0, context);
        }
        None
    }

    /// The wheel over the parameter rows: a row up or down, while the selected effect has more
    /// parameters than the desk has rows; the last parameter stays in the last row.
    fn scroll_params(&mut self, up: bool, context: &Context<'_>) {
        let Some(effect) = self.selected_effect else {
            return;
        };
        let count = context.param_count(effect);
        let rows = mixer_layout::param_rows(self.size.height);
        let most = count.saturating_sub(rows);
        let top = self.params_top();
        let top = if up {
            top.saturating_sub(1)
        } else {
            (top + 1).min(most)
        };
        self.params_scroll = Some((effect, top));
    }
}

/// What a press needs to know beyond the mixer's own state.
pub struct Context<'a> {
    pub project: &'a Project,
    pub registry: &'a Registry,
    /// The channel selected in the rack: the one the header routes.
    pub channel: Option<ChannelId>,
    pub modifiers: Modifiers,
}

impl Context<'_> {
    /// How many parameter rows effect `id` has in the slot panel: a native effect's
    /// parameters, a plugin's not hidden.
    fn param_count(&self, id: EffectId) -> usize {
        let Some((_, effect)) = self.project.mixer.effect(id) else {
            return 0;
        };
        let processor = &effect.processor.id;
        if let Some(factory) = self.registry.get(processor) {
            return factory.params().len();
        }
        self.registry.host(processor).map_or(0, |host| {
            host.params(dawsome_engine::project::ProcessorOwner::Effect(id))
                .iter()
                .filter(|param| !param.hidden)
                .count()
        })
    }
}

/// A fader position as the mixer shows it: "−6.0 dB", "0.0 dB", "+5.8 dB", or "-inf" at 0.
pub fn fader_text(gain: f32) -> String {
    if gain <= 0.0 {
        return "-inf".into();
    }
    signed(20.0 * gain.log10(), 1, " dB")
}

/// A pan as the mixer shows it: "50% L", "Center", "100% R".
pub fn pan_text(pan: f32) -> String {
    let percent = (pan.abs() * 100.0).round();
    if percent == 0.0 {
        "Center".into()
    } else if pan < 0.0 {
        format!("{percent:.0}% L")
    } else {
        format!("{percent:.0}% R")
    }
}

/// A 0..1 amount as a percentage: "50%".
pub fn percent_text(amount: f32) -> String {
    format!("{:.0}%", amount * 100.0)
}

/// A parameter value with its unit, as the slot panel shows it.
pub fn param_text(info: &ParamInfo, value: f32) -> String {
    match info.unit {
        "%" => percent_text(value),
        "dB" if value == f32::NEG_INFINITY => "-inf".into(),
        "dB" => signed(value, 1, " dB"),
        "Hz" if value == f32::INFINITY => "Max".into(),
        "Hz" if value >= 1000.0 => format!("{:.2} kHz", value / 1000.0),
        "Hz" => format!("{value:.0} Hz"),
        "ms" => format!("{value:.0} ms"),
        "steps" if value == 0.0 => "Off".into(),
        "steps" => format!("{value:.0} steps"),
        _ => format!("{value:.2}"),
    }
}

/// Parameter `index` of `factory`'s effect at `value`, as the slot panel shows it, in a project
/// at `sample_rate`.
pub fn effect_param_text(
    factory: &dyn ProcessorFactory,
    index: usize,
    value: f32,
    sample_rate: u32,
) -> Option<String> {
    let info = factory.params().get(index)?;
    // Only shown, so f32 is plenty.
    Some(param_text(
        info,
        factory.shown(index, value, sample_rate) as f32,
    ))
}

/// `value` with `decimals` places, a typographic minus or a plus, then `unit`. Rounds to zero
/// as "0.0", never "−0.0".
fn signed(value: f32, decimals: usize, unit: &str) -> String {
    let text = format!("{:.*}", decimals, value.abs());
    let zero = text.chars().all(|c| c == '0' || c == '.');
    let sign = match (zero, value < 0.0) {
        (true, _) => "",
        (false, true) => "\u{2212}",
        (false, false) => "+",
    };
    format!("{sign}{text}{unit}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Project {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name);
        let text = std::fs::read_to_string(&path).expect("the fixture is there");
        Project::load_json(&text).expect("the fixture loads")
    }

    // By hand from the fixtures. p5_gain's channels play into inserts 3, 1, 2, 4 and 6 (max,
    // unity, half, send, postfader), insert 4 sends to 5 and 6 to 7: each is alone on its
    // insert, so each has its own; with 1 to 7 taken, the first free one is 8. f1's kick plays into the master
    // of a new mixer: Insert 1. p5_chain's click plays into 3, which routes into 2 and 2 into 1,
    // so on the master its first free one is 4 (3's output goes to 2, not the master). Insert 1 shared with another channel, renamed,
    // holding an effect or sent into is not free; with all sixteen taken there is none.
    #[test]
    fn mixer_insert_gives_a_channel_its_own_or_the_first_free_one() {
        let mut gain = fixture("p5_gain.json");
        let own: Vec<_> = (1..=5).map(|id| own_insert(&gain, ChannelId(id))).collect();
        let inserts = [3, 1, 2, 4, 6].map(|n| Some(OwnInsert::Own(InsertId(n))));
        assert_eq!(own, inserts);
        assert_eq!(own_insert(&gain, ChannelId(99)), None);
        gain.channels.get_mut(&ChannelId(2)).expect("unity").insert = InsertId::MASTER;
        assert_eq!(
            own_insert(&gain, ChannelId(2)),
            Some(OwnInsert::Free(InsertId(1))),
            "insert 1 is free once unity left it"
        );
        gain.channels.get_mut(&ChannelId(2)).expect("unity").insert = InsertId(3);
        assert_eq!(
            own_insert(&gain, ChannelId(2)),
            Some(OwnInsert::Free(InsertId(1))),
            "sharing 3 with max"
        );
        gain.channels.get_mut(&ChannelId(2)).expect("unity").insert = InsertId(1);
        gain.channels.get_mut(&ChannelId(1)).expect("max").insert = InsertId::MASTER;
        assert_eq!(
            own_insert(&gain, ChannelId(1)),
            Some(OwnInsert::Free(InsertId(3))),
            "its old 3, free again"
        );
        gain.channels.get_mut(&ChannelId(1)).expect("max").insert = InsertId(1);
        assert_eq!(
            own_insert(&gain, ChannelId(1)),
            Some(OwnInsert::Free(InsertId(3))),
            "sharing 1 with unity"
        );
        gain.mixer.get_mut(InsertId(3)).expect("insert 3").name = "Kick".to_owned();
        assert_eq!(
            own_insert(&gain, ChannelId(1)),
            Some(OwnInsert::Free(InsertId(8))),
            "3 renamed, 4 and 6 played into, 5 and 7 sent into"
        );

        let kick = fixture("f1_four_on_floor.json");
        let first = *kick.channels.keys().next().expect("the kick");
        assert_eq!(own_insert(&kick, first), Some(OwnInsert::Free(InsertId(1))));
        let chain = fixture("p5_chain.json");
        let click = *chain.channels.keys().next().expect("the click");
        assert_eq!(own_insert(&chain, click), Some(OwnInsert::Own(InsertId(3))));
        let mut chain = chain;
        chain.channels.get_mut(&click).expect("the click").insert = InsertId::MASTER;
        assert_eq!(
            own_insert(&chain, click),
            Some(OwnInsert::Free(InsertId(4))),
            "1 and 2 are routed into"
        );

        let mut full = kick;
        let ids: Vec<InsertId> = full.mixer.ids().filter(|id| !id.is_master()).collect();
        assert_eq!(ids.len(), 16);
        for (n, id) in ids.iter().enumerate() {
            full.mixer
                .get_mut(*id)
                .expect("an insert")
                .effects
                .push(Effect {
                    id: EffectId(n as u32 + 1),
                    slot: 0,
                    processor: ProcessorRef::new(eq::ID),
                });
        }
        assert_eq!(own_insert(&full, first), Some(OwnInsert::NoneFree));
    }

    #[test]
    fn values_read_as_the_mixer_shows_them() {
        assert_eq!(fader_text(0.0), "-inf");
        assert_eq!(fader_text(1.0), "0.0 dB");
        assert_eq!(fader_text(0.5), "\u{2212}6.0 dB");
        assert_eq!(fader_text(1.953_125), "+5.8 dB");
        assert_eq!(pan_text(-0.5), "50% L");
        assert_eq!(pan_text(0.0), "Center");
        assert_eq!(pan_text(1.0), "100% R");
        assert_eq!(percent_text(0.5), "50%");
        let steps = ParamInfo {
            key: "sync_steps",
            label: "Sync",
            min: 0.0,
            max: 32.0,
            default: 0.0,
            unit: "steps",
            whole: true,
        };
        assert_eq!(param_text(&steps, 0.0), "Off");
        assert_eq!(param_text(&steps, 3.0), "3 steps");
        let freq = ParamInfo {
            unit: "Hz",
            ..steps
        };
        assert_eq!(param_text(&freq, 1000.0), "1.00 kHz");
        assert_eq!(param_text(&freq, 100.0), "100 Hz");
    }

    // The ported effects' 0..1 controls read in units through the engine's own mappings
    // (values from Python): Pressure4's output 0.5 is 20·log10(0.5) = −6.02 dB; its speed 0.2
    // recovers in (1.28 − 0.2)^5 · 32768 frames at 44.1 kHz = 1091.8 ms; its pressure 0.6
    // drives the input by 20·log10(1/(1 − 0.6·0.95)) = +7.33 dB. Density's highpass 0.5 is a
    // one-pole of a = 0.5³ = 0.125 on each of two memories taking every other sample, so at
    // 22050 Hz: with b = 1 − a = 0.875 its gain is 1/√2 at acos((1 − 3b²)/(2b − 4b²)) ·
    // 22050/2π = 542.0 Hz. At 0.8, b = 1 − 0.512 = 0.488 and its gain peaks (at 11025 Hz) at
    // 2b/(1 + b) = 0.656, under 1/√2: no −3 dB point, shown as the top. Output 0 is "-inf",
    // as on a fader.
    #[test]
    fn the_ported_effects_show_their_values_in_units() {
        let registry = Registry::native();
        let text = |id: &str, index: usize, value: f32| {
            let factory = registry.get(id).expect("a native effect");
            effect_param_text(factory, index, value, 44_100).expect("a parameter")
        };
        assert_eq!(text(pressure4::ID, 3, 0.5), "\u{2212}6.0 dB");
        assert_eq!(text(pressure4::ID, 3, 0.0), "-inf");
        assert_eq!(text(pressure4::ID, 1, 0.2), "1092 ms");
        assert_eq!(text(pressure4::ID, 0, 0.6), "+7.3 dB");
        assert_eq!(text(density::ID, 1, 0.5), "542 Hz");
        assert_eq!(text(density::ID, 1, 0.8), "Max");
        assert_eq!(text(density::ID, 2, 0.5), "\u{2212}6.0 dB");
        // The originals' own displays: mewiness 2C − 1, density 5A − 1.
        assert_eq!(text(pressure4::ID, 2, 0.25), "-0.50");
        assert_eq!(text(density::ID, 0, 0.6), "2.00");
        // The mixes, and the native effects that hold their units already, as before.
        assert_eq!(text(pressure4::ID, 4, 0.5), "50%");
        assert_eq!(text(delay::ID, 0, 250.0), "250 ms");
        assert_eq!(text(limiter::ID, 0, -0.3), "\u{2212}0.3 dB");
        assert_eq!(text(limiter::ID, 1, 100.0), "100 ms");
    }

    // Falling 24 dB a second, a meter at 1.0 shows 10^(−24·0.5/20) = 0.2512 half a second
    // later; its peak mark stays at 1.0 until 1.5 s have passed.
    #[test]
    fn meters_fall_24_db_a_second_and_hold_their_peak_for_1_5_s() {
        let start = Instant::now();
        let mut meters = Meters::default();
        meters.update(&[[1.0, 0.5]], start);
        assert_eq!(meters.level(InsertId::MASTER), [1.0, 0.5]);
        meters.update(&[[0.0; 2]], start + Duration::from_millis(500));
        let [left, _] = meters.level(InsertId::MASTER);
        assert!((left - 0.251_188_6).abs() < 1e-5, "{left}");
        assert_eq!(meters.peak_mark(InsertId::MASTER)[0], 1.0);
        meters.update(&[[0.0; 2]], start + Duration::from_millis(1600));
        assert!(
            meters.peak_mark(InsertId::MASTER)[0] < 1.0,
            "the hold is over"
        );
        assert!(!meters.clip);
        meters.update(&[[1.5, 0.0]], start + Duration::from_millis(1700));
        assert!(meters.clip, "past 0 dB on the master");
        meters.update(&[[0.0; 2]], start + Duration::from_secs(30));
        assert!(!meters.moving(), "silent after a long fall");
        assert!(meters.clip, "the clip light stays on until clicked");
    }
}
