//! The mixer's UI state: the selected insert, how far the strips and the effects' lines are
//! scrolled, an open effect menu, a block being dragged along the chain, a rename in progress,
//! the meters as shown, and the notice for a refused send. It turns the canvas's presses into
//! edits and drags through [`mixer_layout`] and never changes the document itself.
//!
//! Clicks commit on press, as one edit each; drags (fader, pan, send, an effect's knob) are run
//! by the app, live while they move and one edit on release. A block dragged by its name moves
//! only in the drawing while it is held, and its new place is one edit when it is let go.

use std::collections::BTreeSet;
use std::ops::RangeInclusive;
use std::time::{Duration, Instant};

use dawsome_engine::dsp::airwindows::{density, pressure4};
use dawsome_engine::dsp::{
    chorus, delay, distortion, eq, filter, flanger, gate, limiter, phaser, reverb, width,
};
use dawsome_engine::edit::Edit;
use dawsome_engine::processor::{param_values, ParamInfo, ProcessorFactory, Registry};
use dawsome_engine::project::{
    AutomationTarget, ChannelId, Effect, EffectId, Insert, InsertId, ProcessorOwner, ProcessorRef,
    Project, EFFECT_SLOTS, FADER_RANGE, PAN_RANGE, SEND_RANGE, UNITY_FADER,
};
use dawsome_engine::transport::ParamTarget;
use iced::keyboard::Modifiers;
use iced::{mouse, Point, Size};

use crate::ui::mixer_layout::{self, Hit, EFFECTS_SCROLL_PER_NOTCH, SCROLL_PER_NOTCH};

/// The effects the add menu offers, in menu order: EQ, Delay, Reverb, Compressor, Saturator,
/// Limiter, then Filter, Distortion, Chorus, Flanger, Phaser, Gate and Stereo width. Why new
/// effects go after the limiter: the rows before them keep the places they had.
pub const ADD_MENU: [&str; 13] = [
    eq::ID,
    delay::ID,
    reverb::ID,
    pressure4::ID,
    density::ID,
    limiter::ID,
    filter::ID,
    distortion::ID,
    chorus::ID,
    flanger::ID,
    phaser::ID,
    gate::ID,
    width::ID,
];

/// The add menu's last row, after the native effects: it opens the plugin picker.
pub const PLUGIN_EFFECT_ROW: &str = "Plugin effect\u{2026}";

/// The rows of an effect's menu (a right-click on its block). Why no Move up or Move down: the
/// chain is the order the effects were added in.
pub const EFFECT_MENU: [&str; 1] = ["Remove"];

/// An effect's menu's last row, for a plugin effect: its parameter window, named as a plugin
/// channel's menu names it (it read "Generic parameters" while it opened a strip in the main
/// window).
pub const PLUGIN_PARAMS_ROW: &str = "Parameters\u{2026}";

/// The most of a plugin effect's parameters its block shows as knobs: the first of those not
/// hidden. Its parameter window (its menu's Parameters…) has them all.
pub const PLUGIN_KNOBS: usize = 10;

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
    /// A row of the open effect menu, counted from its top: what the row offers is done.
    MenuChosen(usize),
    /// The pointer moved, in the canvas's coordinates, while a block is dragged.
    ReorderMoved(Point),
    /// The button was let go while a block is dragged.
    ReorderEnded,
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
    /// Open the plugin picker for an effect at the end of this insert's chain.
    PickPlugin { insert: InsertId },
    /// Open this plugin effect's window.
    OpenPlugin(EffectId),
    /// Open this plugin effect's parameter window.
    PluginParams(EffectId),
    /// Start dragging knob `index` of this plugin effect (among its parameters that are not
    /// hidden).
    DragPlugin { effect: EffectId, index: usize },
    /// A right press on a control at `at`: the automation menu for what it sets, its value
    /// now normalized over its range.
    Automate {
        target: AutomationTarget,
        value: f32,
        at: Point,
    },
    /// A right press at `at` on knob `index` of this plugin effect (among its parameters that
    /// are not hidden): the automation menu for it, which the app finds by asking the plugin.
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
    /// Parameter `index` of an effect, on its knob.
    Param {
        effect: EffectId,
        index: usize,
    },
}

/// Pixels of drag that sweep a fader, a pan, a send or an effect's knob from one end to the
/// other (a rack knob's sweep).
pub const SWEEP_PIXELS: f32 = 200.0;

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

    /// Whether the control moves left to right (a pan, send or effect knob) rather than up and
    /// down (the fader).
    pub fn horizontal(self) -> bool {
        match self {
            MixerDrag::Fader(_) => false,
            MixerDrag::Pan(_) | MixerDrag::Send { .. } | MixerDrag::Param { .. } => true,
        }
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

/// How many knobs each of insert `insert`'s effects shows, in chain order: a native effect's
/// parameters, a plugin's first [`PLUGIN_KNOBS`] not hidden, none for an effect this build does
/// not know. The effects area lays its blocks out from these ([`mixer_layout::chain_rects`]).
pub fn knob_counts(project: &Project, registry: &Registry, insert: InsertId) -> Vec<usize> {
    project.mixer.get(insert).map_or_else(Vec::new, |insert| {
        insert
            .effects
            .iter()
            .map(|effect| knob_count(effect, registry))
            .collect()
    })
}

fn knob_count(effect: &Effect, registry: &Registry) -> usize {
    let processor = &effect.processor.id;
    if let Some(factory) = registry.get(processor) {
        return factory.params().len();
    }
    registry.host(processor).map_or(0, |host| {
        host.params(ProcessorOwner::Effect(effect.id))
            .iter()
            .filter(|param| !param.hidden)
            .take(PLUGIN_KNOBS)
            .count()
    })
}

/// Whether parameter `index` of `factory` is a choice (the filter's type, the phaser's stages):
/// a whole parameter whose values have names, which its block shows as a button stepping
/// through them rather than as a knob.
pub fn is_choice(factory: &dyn ProcessorFactory, index: usize) -> bool {
    factory
        .params()
        .get(index)
        .is_some_and(|info| info.whole && factory.value_name(index, info.min).is_some())
}

/// The edit that puts `processor` at the end of insert `insert`'s chain, switched off: the add
/// menu's and the plugin picker's. Why off: an effect added waits for its light, so the sound is
/// heard without it first. The effects keep their order; any that sit
/// apart (slots left free by a removal, or by a file) move down first to close the gaps, in the
/// same undo step, so the last slot is never taken while one before it is free. `None` when the
/// insert is full or not there.
pub fn append_effect(project: &Project, insert: InsertId, processor: ProcessorRef) -> Option<Edit> {
    let effects = &project.mixer.get(insert)?.effects;
    let slot = u8::try_from(effects.len())
        .ok()
        .filter(|&slot| slot < EFFECT_SLOTS)?;
    let add = Edit::AddEffect {
        insert,
        slot,
        id: project.mixer.next_effect_id,
        processor,
        on: false,
    };
    let mut edits: Vec<Edit> = effects
        .iter()
        .zip(0u8..)
        .filter(|(effect, place)| effect.slot != *place)
        .map(|(effect, place)| Edit::MoveEffect {
            effect: effect.id,
            slot: place,
        })
        .collect();
    if edits.is_empty() {
        return Some(add);
    }
    edits.push(add);
    Some(Edit::Transaction(edits))
}

/// The edit that takes the effect at place `from` of `effects` (in chain order) to place `to`,
/// those between moving a place toward `from`: a swap with each neighbour on the way (the
/// engine's [`Edit::MoveEffect`] onto the neighbour's slot), one edit for one place, a
/// transaction for more, one undo step either way. `None` when it stays, or a place is not
/// there.
pub fn reorder_edit(effects: &[Effect], from: usize, to: usize) -> Option<Edit> {
    let effect = effects.get(from)?.id;
    let passed = if to > from {
        effects.get(from + 1..=to)?.to_vec()
    } else {
        let mut before = effects.get(to..from)?.to_vec();
        before.reverse();
        before
    };
    let mut moves: Vec<Edit> = passed
        .iter()
        .map(|neighbour| Edit::MoveEffect {
            effect,
            slot: neighbour.slot,
        })
        .collect();
    match moves.len() {
        0 => None,
        1 => moves.pop(),
        _ => Some(Edit::Transaction(moves)),
    }
}

/// A block dragged by its name along the chain: the document keeps the old order until it is
/// let go; the drawing shows the new one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reorder {
    pub effect: EffectId,
    /// Its place in the chain when it was pressed, and the place it would take now.
    pub from: usize,
    pub place: usize,
    /// The pointer, and where on the block it took hold (from the block's top left).
    pub at: Point,
    pub grip: iced::Vector,
}

/// An open effect menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Menu {
    /// Under the add tile: the effects the chain can take at its end.
    Add,
    /// Under an effect's block: what can be done with it.
    Effect { effect: EffectId },
}

impl Menu {
    /// How many rows the menu has in `project`: the native effects and the plugin row for the
    /// add tile; Remove for an effect, and for a plugin effect its parameters too.
    pub fn rows(self, project: &Project) -> usize {
        match self {
            Menu::Add => ADD_MENU.len() + 1,
            Menu::Effect { effect } => EFFECT_MENU.len() + usize::from(is_plugin(project, effect)),
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
    /// How far the strips are scrolled to the left, in pixels.
    pub scroll: f32,
    /// How far the selected insert's effects' lines are scrolled up, in pixels, as the wheel
    /// left it, or as an add left it ([`mixer_layout::SHOW_ADDED`]): shown no further than they
    /// go ([`mixer_layout::effects_scroll`]).
    pub effects_scroll: f32,
    pub menu: Option<Menu>,
    /// A block of the selected insert being dragged along its chain.
    pub reorder: Option<Reorder>,
    /// The insert being renamed, and the name typed so far.
    pub rename: Option<(InsertId, String)>,
    pub meters: Meters,
    refusal: Option<Refusal>,
    /// The canvas's size, as it last reported it.
    size: Size,
}

impl MixerState {
    /// The mixer with the master selected, a panel `height` tall and `width` wide.
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            selected: InsertId::MASTER,
            scroll: 0.0,
            effects_scroll: 0.0,
            menu: None,
            reorder: None,
            rename: None,
            meters: Meters::default(),
            refusal: None,
            size: Size::new(width, mixer_layout::desk_height(height)),
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

    /// The desk as a press finds it, for a project whose highest insert is `last`.
    pub fn desk(&self, last: InsertId) -> mixer_layout::Desk {
        mixer_layout::Desk {
            size: self.size,
            scroll: self.scroll,
            last,
            effects_scroll: self.effects_scroll,
        }
    }

    /// Selects insert `id`; another insert's effects show from their first line.
    pub fn select(&mut self, id: InsertId, project: &Project) {
        if project.mixer.get(id).is_none() {
            return;
        }
        if id != self.selected {
            self.effects_scroll = 0.0;
            self.reorder = None;
        }
        self.selected = id;
    }

    /// The selected insert's chain as it shows, as places in its effects list: in order, or
    /// with a dragged block moved to the place it would take. `count` is how many effects it
    /// has.
    pub fn chain_order(&self, count: usize) -> Vec<usize> {
        let mut order: Vec<usize> = (0..count).collect();
        if let Some(reorder) = self.reorder.filter(|reorder| reorder.from < count) {
            let moved = order.remove(reorder.from);
            order.insert(reorder.place.min(order.len()), moved);
        }
        order
    }

    /// Shows the effect just added at the chain's end, by whole lines, from the first block
    /// while it shows there with the add tile ([`mixer_layout::SHOW_ADDED`]).
    pub fn show_added(&mut self) {
        self.effects_scroll = mixer_layout::SHOW_ADDED;
    }

    /// Brings the state in line with a changed document: what is gone is let go of.
    pub fn refresh(&mut self, project: &Project) {
        let mixer = &project.mixer;
        if mixer.get(self.selected).is_none() {
            self.selected = InsertId::MASTER;
        }
        self.select(self.selected, project);
        if let Some(Menu::Effect { effect }) = self.menu {
            if mixer.effect(effect).is_none() {
                self.menu = None;
            }
        }
        if let Some(reorder) = self.reorder {
            let place = mixer
                .get(self.selected)
                .and_then(|insert| insert.effects.iter().position(|e| e.id == reorder.effect));
            if place != Some(reorder.from) {
                self.reorder = None;
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

    /// Esc: lets go of a dragged block where it was, ends a rename without committing it, or
    /// closes a menu. Returns whether it did any.
    pub fn escape(&mut self) -> bool {
        self.reorder.take().is_some() || self.rename.take().is_some() || self.menu.take().is_some()
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
            MixerMessage::ReorderMoved(at) => {
                self.reorder_to(at, context);
                None
            }
            MixerMessage::ReorderEnded => {
                let reorder = self.reorder.take()?;
                let effects = &context.project.mixer.get(self.selected)?.effects;
                reorder_edit(effects, reorder.from, reorder.place).map(MixerCommand::Edit)
            }
        }
    }

    /// The dragged block follows the pointer at `at`: past a neighbour's middle it takes the
    /// neighbour's place ([`mixer_layout::reorder_place`]), in the chain as it shows.
    fn reorder_to(&mut self, at: Point, context: &Context<'_>) {
        let Some(reorder) = self.reorder else {
            return;
        };
        let counts = knob_counts(context.project, context.registry, self.selected);
        let shown: Vec<usize> = self
            .chain_order(counts.len())
            .into_iter()
            .map(|place| counts[place])
            .collect();
        let desk = self.desk(self.last(context.project));
        let place = mixer_layout::reorder_place(&shown, desk, reorder.place, at);
        self.reorder = Some(Reorder {
            place,
            at,
            ..reorder
        });
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
        // Any press on the mixer closes its effect menu: a press on the menu is the menu's own,
        // taken on the window's top layer before it could reach the mixer. A block still held
        // (its release was missed, outside the window) is let go where it was.
        self.menu = None;
        self.reorder = None;
        let chain = knob_counts(project, context.registry, self.selected);
        let hit = mixer_layout::hit_test(at, self.desk(self.last(project)), &chain)?;
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
            Hit::EffectLed(_)
            | Hit::EffectName(_)
            | Hit::RemoveEffect(_)
            | Hit::MoveEarlier(_)
            | Hit::MoveLater(_)
            | Hit::Knob { .. }
            | Hit::Block(_)
            | Hit::AddEffect => self.press_effects(hit, right, double, at, context),
            Hit::Fader(id) if right => self.automate(MixerDrag::Fader(id), at, context),
            Hit::Pan(id) if right => self.automate(MixerDrag::Pan(id), at, context),
            Hit::Send(id) if right && routes_here(id) => {
                let send = MixerDrag::Send {
                    from: self.selected,
                    to: id,
                };
                self.automate(send, at, context)
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
        }
    }

    /// A press on the selected insert's effects: a block's light switches its effect on or off,
    /// its cross removes it, its arrows move it a place earlier or later, its name starts a drag
    /// along the chain (a double press on a plugin's opens its window), a right press elsewhere
    /// on it opens its menu; its knobs as [`press_knob`](Self::press_knob) says; the add tile
    /// opens the add menu.
    fn press_effects(
        &mut self,
        hit: Hit,
        right: bool,
        double: bool,
        at: Point,
        context: &Context<'_>,
    ) -> Option<MixerCommand> {
        let block = match hit {
            Hit::AddEffect => {
                if !right {
                    self.menu = Some(Menu::Add);
                }
                return None;
            }
            Hit::Knob { block, knob } => {
                let effect = context
                    .project
                    .mixer
                    .get(self.selected)?
                    .effects
                    .get(block)?;
                return self.press_knob(effect, knob, right, double, at, context);
            }
            Hit::EffectLed(block)
            | Hit::EffectName(block)
            | Hit::RemoveEffect(block)
            | Hit::MoveEarlier(block)
            | Hit::MoveLater(block)
            | Hit::Block(block) => block,
            _ => return None,
        };
        let effects = &context.project.mixer.get(self.selected)?.effects;
        let effect = effects.get(block)?;
        let id = effect.id;
        if right {
            self.menu = Some(Menu::Effect { effect: id });
            return None;
        }
        match hit {
            Hit::EffectLed(_) => Some(MixerCommand::Edit(Edit::SetEffectOn {
                effect: id,
                on: !effect.on,
            })),
            Hit::RemoveEffect(_) => Some(MixerCommand::Edit(Edit::RemoveEffect { id })),
            Hit::MoveEarlier(_) => {
                reorder_edit(effects, block, block.checked_sub(1)?).map(MixerCommand::Edit)
            }
            Hit::MoveLater(_) => reorder_edit(effects, block, block + 1).map(MixerCommand::Edit),
            Hit::EffectName(_) if double && effect.processor.format().is_some() => {
                Some(MixerCommand::OpenPlugin(id))
            }
            Hit::EffectName(_) => {
                let counts = knob_counts(context.project, context.registry, self.selected);
                let desk = self.desk(self.last(context.project));
                let held = *mixer_layout::chain_rects(&counts, desk).get(block)?;
                self.reorder = Some(Reorder {
                    effect: id,
                    from: block,
                    place: block,
                    at,
                    grip: at - held.position(),
                });
                None
            }
            _ => None,
        }
    }

    /// A press on knob `index` of `effect`: a drag (a double press resets a native one to its
    /// default), a choice's button steps to its next value (after the last, its first), a right
    /// press opens the automation menu; a plugin's knobs are its parameters not hidden.
    fn press_knob(
        &self,
        effect: &Effect,
        index: usize,
        right: bool,
        double: bool,
        at: Point,
        context: &Context<'_>,
    ) -> Option<MixerCommand> {
        let id = effect.id;
        if effect.processor.format().is_some() {
            return Some(if right {
                MixerCommand::AutomatePlugin {
                    effect: id,
                    index,
                    at,
                }
            } else {
                MixerCommand::DragPlugin { effect: id, index }
            });
        }
        let drag = MixerDrag::Param { effect: id, index };
        if right {
            return self.automate(drag, at, context);
        }
        let factory = context.registry.get(&effect.processor.id)?;
        let info = factory.params().get(index)?;
        let set = |value| {
            MixerCommand::Edit(Edit::SetEffectParam {
                effect: id,
                key: info.key.to_owned(),
                value,
            })
        };
        if is_choice(factory, index) {
            let now = *effect_values(effect, context.registry)?.get(index)?;
            let next = if now + 1.0 > info.max {
                info.min
            } else {
                now + 1.0
            };
            return Some(set(Some(next)));
        }
        Some(if double {
            set(None)
        } else {
            MixerCommand::Drag(drag)
        })
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

    /// Row `row` of `menu` (one it has): the add menu's effects go at the end of the chain,
    /// off, its last row picks a plugin; an effect's menu removes it, or opens a plugin's
    /// parameter window.
    fn menu_choice(&mut self, menu: Menu, row: usize, project: &Project) -> Option<MixerCommand> {
        match menu {
            Menu::Add if row == ADD_MENU.len() => Some(MixerCommand::PickPlugin {
                insert: self.selected,
            }),
            Menu::Add => {
                let processor = ProcessorRef::new(*ADD_MENU.get(row)?);
                let edit = append_effect(project, self.selected, processor)?;
                self.show_added();
                Some(MixerCommand::Edit(edit))
            }
            Menu::Effect { effect } if row == EFFECT_MENU.len() => {
                Some(MixerCommand::PluginParams(effect))
            }
            Menu::Effect { effect } => Some(MixerCommand::Edit(Edit::RemoveEffect { id: effect })),
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
        let last = self.last(context.project);
        let notches = if rise > 0.0 { -1.0 } else { 1.0 };
        if at.x < mixer_layout::effects_left(self.size.width, last) {
            let max = mixer_layout::max_scroll(last, self.size.width);
            self.scroll = (self.scroll + notches * SCROLL_PER_NOTCH).clamp(0.0, max);
        } else {
            // The effects' lines, from where they show (a shorter chain leaves less to scroll).
            let chain = knob_counts(context.project, context.registry, self.selected);
            let desk = self.desk(last);
            let now = mixer_layout::effects_scroll(&chain, desk);
            let max = mixer_layout::max_effects_scroll(&chain, desk);
            self.effects_scroll = (now + notches * EFFECTS_SCROLL_PER_NOTCH).clamp(0.0, max);
        }
        None
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

/// A 0..1 amount as a percentage: "50%". Below zero (a feedback) with decibels' minus,
/// "−95%"; a negative zero, or a share that rounds to 0, reads "0%".
pub fn percent_text(amount: f32) -> String {
    let percent = amount * 100.0;
    if percent.is_sign_negative() {
        return signed(percent, 0, "%");
    }
    format!("{percent:.0}%")
}

/// A parameter value with its unit, as an effect's knob shows it. Small hertz and milliseconds
/// get decimals (an LFO's 0.5 Hz, a flanger's 0.1 ms), long times read in seconds; octaves,
/// semitones and cents are signed, as decibels are.
pub fn param_text(info: &ParamInfo, value: f32) -> String {
    match info.unit {
        "%" => percent_text(value),
        "dB" if value == f32::NEG_INFINITY => "-inf".into(),
        "dB" => signed(value, 1, " dB"),
        "Hz" if value == f32::INFINITY => "Max".into(),
        "Hz" if value >= 1000.0 => format!("{:.2} kHz", value / 1000.0),
        // An exact 0 (an effect's highpass at its lowest) reads as a plain 0, not "0.00 Hz".
        "Hz" if value == 0.0 => "0 Hz".into(),
        "Hz" if value < 10.0 => format!("{value:.2} Hz"),
        "Hz" => format!("{value:.0} Hz"),
        "ms" if value >= 1000.0 => format!("{:.2} s", value / 1000.0),
        "ms" if value < 10.0 => format!("{value:.1} ms"),
        "ms" => format!("{value:.0} ms"),
        "oct" if info.whole => signed(value, 0, " oct"),
        "oct" => format!("{value:.1} oct"),
        "st" if info.whole => signed(value, 0, " st"),
        "st" => signed(value, 1, " st"),
        "ct" => signed(value, 0, " ct"),
        // An angle (the flanger's and the phaser's Spread), in whole degrees.
        "°" => format!("{value:.0}°"),
        "steps" if value == 0.0 => "Off".into(),
        "steps" => format!("{value:.0} steps"),
        _ => format!("{value:.2}"),
    }
}

/// Parameter `index` of `factory`'s effect at `value`, as its knob shows it, in a project
/// at `sample_rate`: the value's name when it is a choice, else the number in its unit.
pub fn effect_param_text(
    factory: &dyn ProcessorFactory,
    index: usize,
    value: f32,
    sample_rate: u32,
) -> Option<String> {
    let info = factory.params().get(index)?;
    if let Some(name) = factory.value_name(index, value) {
        return Some(name.into());
    }
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
    use dawsome_engine::processor::Taper;

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
                    on: true,
                });
        }
        assert_eq!(own_insert(&full, first), Some(OwnInsert::NoneFree));
    }

    // By hand: p5_delay_sync's insert 1 holds the delay (effect 1) in slot 0, so an effect added
    // goes in slot 1, off, as the mixer's next effect (2): one AddEffect. With the EQ as effect 2
    // in slot 9 (the slots between left free, as a removal leaves them), the next goes at the
    // end too: the EQ moves down to slot 1 and the new effect (3) goes in slot 2, one
    // transaction, the order kept. A full insert, or one not there, takes none.
    #[test]
    fn an_added_effect_goes_at_the_end_of_the_chain_switched_off() {
        let mut project = fixture("p5_delay_sync.json");
        let one = InsertId(1);
        let reverb = || ProcessorRef::new(reverb::ID);
        let add = |slot, id| Edit::AddEffect {
            insert: one,
            slot,
            id: EffectId(id),
            processor: reverb(),
            on: false,
        };
        assert_eq!(append_effect(&project, one, reverb()), Some(add(1, 2)));
        project
            .mixer
            .get_mut(one)
            .expect("insert 1")
            .effects
            .push(Effect {
                id: EffectId(2),
                slot: 9,
                processor: ProcessorRef::new(eq::ID),
                on: true,
            });
        project.mixer.next_effect_id = EffectId(3);
        project.validate().expect("valid");
        let edit = append_effect(&project, one, reverb()).expect("room at the end");
        let moved = Edit::MoveEffect {
            effect: EffectId(2),
            slot: 1,
        };
        assert_eq!(edit, Edit::Transaction(vec![moved, add(2, 3)]));
        dawsome_engine::edit::apply(&mut project, edit).expect("applies");
        let chain: Vec<(u32, u8, bool)> = project
            .mixer
            .get(one)
            .expect("insert 1")
            .effects
            .iter()
            .map(|effect| (effect.id.0, effect.slot, effect.on))
            .collect();
        assert_eq!(chain, [(1, 0, true), (2, 1, true), (3, 2, false)]);
        let effects = &mut project.mixer.get_mut(one).expect("insert 1").effects;
        *effects = (0..EFFECT_SLOTS)
            .map(|slot| Effect {
                id: EffectId(u32::from(slot) + 1),
                slot,
                processor: reverb(),
                on: true,
            })
            .collect();
        assert_eq!(append_effect(&project, one, reverb()), None, "full");
        assert_eq!(append_effect(&project, InsertId(99), reverb()), None);
    }

    // The native effects' choices are the distortion's auto gain, the filter's type and slope
    // and the phaser's stages, and only they show as a button (the width's bass mono names its
    // 0 "Off" but is a knob: it is not whole).
    #[test]
    fn only_the_named_whole_parameters_are_choices() {
        let registry = Registry::native();
        let choices: Vec<(&str, &str)> = registry
            .factories()
            .flat_map(|factory| {
                factory
                    .params()
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| is_choice(factory, *index))
                    .map(|(_, info)| (factory.id(), info.key))
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(
            choices,
            [
                (distortion::ID, "auto_gain"),
                (filter::ID, "type"),
                (filter::ID, "slope"),
                (phaser::ID, "stages")
            ]
        );
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
            taper: Taper::Linear,
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

    /// A parameter in `unit`, whole or not.
    fn in_unit(unit: &'static str, whole: bool) -> ParamInfo {
        ParamInfo {
            key: "amount",
            label: "Amount",
            min: -100.0,
            max: 5000.0,
            default: 0.0,
            unit,
            whole,
            taper: Taper::Linear,
        }
    }

    // Each arm by hand: hertz under 10 in hundredths (an LFO's 0.5 Hz no longer reads "0 Hz"),
    // but an exact 0 (−0 too) as "0 Hz", from 10 whole, from 1000 in kHz; milliseconds under 10
    // in tenths, from 1000 in seconds; octaves, semitones and cents signed (a typographic
    // minus, no sign on 0), whole ones without decimals.
    #[test]
    fn each_unit_reads_as_a_knob_shows_it() {
        let hz = in_unit("Hz", false);
        assert_eq!(param_text(&hz, 0.5), "0.50 Hz");
        assert_eq!(param_text(&hz, 2.99), "2.99 Hz");
        assert_eq!(param_text(&hz, 0.0), "0 Hz");
        assert_eq!(param_text(&hz, -0.0), "0 Hz");
        assert_eq!(param_text(&hz, 0.001), "0.00 Hz");
        assert_eq!(param_text(&hz, 10.0), "10 Hz");
        assert_eq!(param_text(&hz, 632.46), "632 Hz");
        assert_eq!(param_text(&hz, 4000.0), "4.00 kHz");
        assert_eq!(param_text(&hz, f32::INFINITY), "Max");
        let degrees = in_unit("°", false);
        assert_eq!(param_text(&degrees, 90.0), "90°");
        assert_eq!(param_text(&degrees, 0.0), "0°");
        assert_eq!(param_text(&degrees, 179.6), "180°");
        let ms = in_unit("ms", false);
        assert_eq!(param_text(&ms, 1.0), "1.0 ms");
        assert_eq!(param_text(&ms, 9.5), "9.5 ms");
        assert_eq!(param_text(&ms, 0.1), "0.1 ms");
        assert_eq!(param_text(&ms, 10.0), "10 ms");
        assert_eq!(param_text(&ms, 999.0), "999 ms");
        assert_eq!(param_text(&ms, 1000.0), "1.00 s");
        assert_eq!(param_text(&ms, 2551.4), "2.55 s");
        let oct = in_unit("oct", false);
        assert_eq!(param_text(&oct, 2.0), "2.0 oct");
        assert_eq!(param_text(&oct, 0.5), "0.5 oct");
        let whole_oct = in_unit("oct", true);
        assert_eq!(param_text(&whole_oct, 1.0), "+1 oct");
        assert_eq!(param_text(&whole_oct, -2.0), "\u{2212}2 oct");
        assert_eq!(param_text(&whole_oct, 0.0), "0 oct");
        assert_eq!(param_text(&whole_oct, -0.0), "0 oct");
        let st = in_unit("st", false);
        assert_eq!(param_text(&st, 3.5), "+3.5 st");
        assert_eq!(param_text(&st, 12.0), "+12.0 st");
        assert_eq!(param_text(&st, -1.5), "\u{2212}1.5 st");
        assert_eq!(param_text(&st, 0.0), "0.0 st");
        let whole_st = in_unit("st", true);
        assert_eq!(param_text(&whole_st, 7.0), "+7 st");
        assert_eq!(param_text(&whole_st, -12.0), "\u{2212}12 st");
        assert_eq!(param_text(&whole_st, 0.0), "0 st");
        let ct = in_unit("ct", false);
        assert_eq!(param_text(&ct, 7.0), "+7 ct");
        assert_eq!(param_text(&ct, 12.4), "+12 ct");
        assert_eq!(param_text(&ct, -100.0), "\u{2212}100 ct");
        assert_eq!(param_text(&ct, 0.0), "0 ct");
        // The units that kept their arms.
        assert_eq!(param_text(&in_unit("%", false), 0.5), "50%");
        assert_eq!(param_text(&in_unit("dB", false), -6.0), "\u{2212}6.0 dB");
        assert_eq!(param_text(&in_unit("", false), 0.723), "0.72");
    }

    /// A factory whose parameter 0 is a choice of three named values and parameter 1 an amount.
    struct Choices;

    const CHOICES: [ParamInfo; 2] = [
        ParamInfo {
            key: "type",
            label: "Type",
            min: 0.0,
            max: 2.0,
            default: 0.0,
            unit: "",
            whole: true,
            taper: Taper::Linear,
        },
        ParamInfo {
            key: "rate",
            label: "Rate",
            min: 0.0,
            max: 5.0,
            default: 1.0,
            unit: "Hz",
            whole: false,
            taper: Taper::Linear,
        },
    ];

    impl ProcessorFactory for Choices {
        fn id(&self) -> &'static str {
            "test.choices"
        }

        fn name(&self) -> &'static str {
            "Choices"
        }

        fn params(&self) -> &'static [ParamInfo] {
            &CHOICES
        }

        fn value_name(&self, index: usize, value: f32) -> Option<&'static str> {
            let names = ["Low-pass", "High-pass", "Notch"];
            (index == 0)
                .then(|| names.get(value as usize))
                .flatten()
                .copied()
        }

        fn create(
            &self,
            _sample_rate: u32,
            _values: &[f32],
        ) -> Box<dyn dawsome_engine::processor::Processor> {
            unreachable!("only its texts are read")
        }
    }

    // A named value shows its name in place of the number; a parameter with no names (and an
    // index past the last) shows as before.
    #[test]
    fn a_choice_shows_its_values_name() {
        assert_eq!(
            effect_param_text(&Choices, 0, 2.0, 44_100).as_deref(),
            Some("Notch")
        );
        assert_eq!(
            effect_param_text(&Choices, 0, 0.0, 44_100).as_deref(),
            Some("Low-pass")
        );
        assert_eq!(
            effect_param_text(&Choices, 1, 0.5, 44_100).as_deref(),
            Some("0.50 Hz")
        );
        assert_eq!(effect_param_text(&Choices, 2, 0.0, 44_100), None);
        // The native effects name the values of their choices (the distortion's auto gain, the
        // filter's type and slope, the phaser's stages) and the width's bass mono at its
        // default, Off; none of any other parameter's.
        let named = [
            (distortion::ID, "auto_gain"),
            (filter::ID, "type"),
            (filter::ID, "slope"),
            (phaser::ID, "stages"),
            (width::ID, "bass_mono"),
        ];
        let registry = Registry::native();
        for factory in registry.factories() {
            for (index, info) in factory.params().iter().enumerate() {
                let name = factory.value_name(index, info.default);
                let listed = named.contains(&(factory.id(), info.key));
                assert_eq!(name.is_some(), listed, "{} {}", factory.id(), info.key);
            }
        }
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
        // 1091.8 ms, from 1000 ms in seconds (it read "1092 ms" before the seconds arm).
        assert_eq!(text(pressure4::ID, 1, 0.2), "1.09 s");
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
        // The ends of the ranges the tenths and seconds arms reach: the limiter's release 1 ms
        // and 1000 ms, the delay's 1 ms and 4000 ms, Pressure4's speed 1 at (1.28 − 1)^5 ·
        // 32768 / 44.1 = 1.27 ms, and Density's highpass 0 (b = 1, cos ω = 1: 0 Hz).
        assert_eq!(text(limiter::ID, 1, 1.0), "1.0 ms");
        assert_eq!(text(limiter::ID, 1, 1000.0), "1.00 s");
        assert_eq!(text(delay::ID, 0, 1.0), "1.0 ms");
        assert_eq!(text(delay::ID, 0, 4000.0), "4.00 s");
        assert_eq!(text(pressure4::ID, 1, 1.0), "1.3 ms");
        assert_eq!(text(density::ID, 1, 0.0), "0 Hz");
    }

    // The filter, the distortion, the gate and the width as their knobs show them: the
    // filter's type and slope by name, its cutoff 20·1000^v Hz (632 Hz at 0.5, 20 kHz at 1),
    // the distortion's tone 1000·20^v Hz and its auto gain by name, a width of 2 as 200%, its
    // bass mono Off at 0 and 50·10^v Hz above (158 Hz at 0.5, 500 Hz at 1); the flanger's and
    // the phaser's spread in degrees.
    #[test]
    fn the_filter_distortion_gate_and_width_show_their_values() {
        let registry = Registry::native();
        let text = |id: &str, index: usize, value: f32| {
            let factory = registry.get(id).expect("a native effect");
            effect_param_text(factory, index, value, 44_100).expect("a parameter")
        };
        assert_eq!(text(filter::ID, 0, 0.0), "Low-pass");
        assert_eq!(text(filter::ID, 0, 3.0), "Notch");
        assert_eq!(text(filter::ID, 1, 0.0), "12 dB");
        assert_eq!(text(filter::ID, 1, 1.0), "24 dB");
        assert_eq!(text(filter::ID, 2, 0.5), "632 Hz");
        assert_eq!(text(filter::ID, 2, 1.0), "20.00 kHz");
        assert_eq!(text(filter::ID, 2, 0.0), "20 Hz");
        assert_eq!(text(filter::ID, 3, 0.1), "10%");
        assert_eq!(text(distortion::ID, 1, 0.0), "1.00 kHz");
        assert_eq!(text(distortion::ID, 1, 1.0), "20.00 kHz");
        assert_eq!(text(distortion::ID, 0, 12.0), "+12.0 dB");
        assert_eq!(text(distortion::ID, 3, 1.0), "On");
        assert_eq!(text(distortion::ID, 3, 0.0), "Off");
        assert_eq!(text(gate::ID, 0, -40.0), "\u{2212}40.0 dB");
        assert_eq!(text(gate::ID, 3, 100.0), "100 ms");
        assert_eq!(text(width::ID, 0, 2.0), "200%");
        assert_eq!(text(width::ID, 1, 0.0), "Off");
        assert_eq!(text(width::ID, 1, 0.5), "158 Hz");
        assert_eq!(text(width::ID, 1, 1.0), "500 Hz");
        assert_eq!(text(flanger::ID, 4, 90.0), "90°");
        assert_eq!(text(phaser::ID, 5, 180.0), "180°");
    }

    // A share below zero reads with the typographic minus, as decibels do (`signed`); one that
    // rounds to 0, a negative zero included, reads "0%", never with a sign. Above zero, no plus.
    #[test]
    fn a_negative_share_reads_with_a_minus() {
        assert_eq!(percent_text(-0.95), "\u{2212}95%");
        assert_eq!(percent_text(-0.5), "\u{2212}50%");
        assert_eq!(percent_text(-0.0), "0%");
        assert_eq!(percent_text(-0.004), "0%");
        assert_eq!(percent_text(0.0), "0%");
        assert_eq!(percent_text(0.95), "95%");
        assert_eq!(percent_text(2.0), "200%");
    }

    // The chorus, the flanger and the phaser as their knobs show them: rates in hertz with
    // decimals below 10 Hz, the flanger's delay in tenths of a millisecond below 10 ms, a
    // feedback of −0.95 as −95%, the phaser's stages by name, its depth in octaves and its center
    // 100·40^v Hz (100 Hz at 0, 632 Hz at 0.5, 4 kHz at 1).
    #[test]
    fn the_chorus_flanger_and_phaser_show_their_values() {
        let registry = Registry::native();
        let text = |id: &str, index: usize, value: f32| {
            let factory = registry.get(id).expect("a native effect");
            effect_param_text(factory, index, value, 44_100).expect("a parameter")
        };
        assert_eq!(text(chorus::ID, 0, 0.8), "0.80 Hz");
        assert_eq!(text(chorus::ID, 1, 0.5), "50%");
        assert_eq!(text(flanger::ID, 0, 2.0), "2.0 ms");
        assert_eq!(text(flanger::ID, 0, 10.0), "10 ms");
        assert_eq!(text(flanger::ID, 2, 0.05), "0.05 Hz");
        // A negative share reads with decibels' minus (`percent_text` through `signed`).
        assert_eq!(text(flanger::ID, 3, -0.95), "\u{2212}95%");
        assert_eq!(text(phaser::ID, 4, -0.95), "\u{2212}95%");
        assert_eq!(text(phaser::ID, 4, 0.0), "0%");
        assert_eq!(text(phaser::ID, 0, 0.0), "4 stages");
        assert_eq!(text(phaser::ID, 0, 1.0), "6 stages");
        assert_eq!(text(phaser::ID, 0, 2.0), "8 stages");
        assert_eq!(text(phaser::ID, 2, 2.0), "2.0 oct");
        assert_eq!(text(phaser::ID, 3, 0.0), "100 Hz");
        assert_eq!(text(phaser::ID, 3, 0.5), "632 Hz");
        assert_eq!(text(phaser::ID, 3, 1.0), "4.00 kHz");
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
