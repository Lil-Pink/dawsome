//! The mixer: the master and the inserts, where each insert sends its sound, and the effects in
//! its slots. An insert is named by its position (0 is the master) and a channel names the
//! insert it plays into. Every insert's sound goes through its effects in slot order, its fader
//! and its pan, then on to each insert it sends to. Sends never form a loop, so there is always
//! an order that mixes each insert after every insert that sends to it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};

use super::{Channel, ChannelId, ValidationError, PAN_RANGE};

/// Position of an insert in the mixer: 0 is the master, 1 and up the inserts. Inserts are never
/// added, removed or reordered, so a position names the same insert for the life of a project.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct InsertId(pub u16);

impl InsertId {
    pub const MASTER: Self = Self(0);

    /// Takes a reference so serde can call it to leave the default out of a channel.
    pub fn is_master(&self) -> bool {
        *self == Self::MASTER
    }

    fn index(self) -> usize {
        usize::from(self.0)
    }
}

impl fmt::Display for InsertId {
    /// "Master" or "Insert n", as the mixer labels them.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_master() {
            f.write_str("Master")
        } else {
            write!(f, "Insert {}", self.0)
        }
    }
}

/// Stable ID of an effect in a slot. New IDs come from [`Mixer::next_effect_id`] and are never
/// reused, so an effect keeps its ID (and its running state) when it moves to another slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EffectId(pub u32);

/// Inserts a new mixer has besides the master.
pub const DEFAULT_INSERTS: u16 = 16;

/// The most inserts a mixer holds besides the master.
pub const MAX_INSERTS: u16 = 500;

/// Effect slots per insert; an effect's `slot` is below this.
pub const EFFECT_SLOTS: u8 = 10;

/// The fader position that leaves the level unchanged (gain 1, 0 dB).
pub const UNITY_FADER: f32 = 0.8;

/// Fader positions: 0 is silence, [`UNITY_FADER`] unity, 1 the top (about +5.8 dB).
pub const FADER_RANGE: RangeInclusive<f32> = 0.0..=1.0;

/// Send amounts, linear gain: 1 sends the insert's whole output.
pub const SEND_RANGE: RangeInclusive<f32> = 0.0..=1.0;

/// The prefix every native processor id starts with, such as `"dawsome.delay"`.
pub const NATIVE_ID_PREFIX: &str = "dawsome.";

/// The prefix native processor ids were saved with under the program's former name. Every
/// project read from a file has such ids moved to [`NATIVE_ID_PREFIX`], so a file saved before
/// the rename keeps its effects.
pub const FORMER_NATIVE_ID_PREFIX: &str = "orchard.";

/// The master and the inserts, the solo set, and where effect IDs continue.
///
/// Field order is JSON field order. A mixer equal to [`Mixer::default`] is not written, so
/// projects made before the mixer existed keep their bytes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mixer {
    /// The ID the next new effect gets: above every ID ever given out.
    pub next_effect_id: EffectId,
    /// The soloed inserts. While any is soloed, only the soloed inserts, the inserts that feed
    /// them and the inserts they feed are heard (see [`Mixer::solo_audible`]).
    pub solo: BTreeSet<InsertId>,
    /// By position: the master first, then insert 1, 2, ...
    #[serde(with = "position_list")]
    pub inserts: Vec<Insert>,
}

/// One mixer strip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Insert {
    pub name: String,
    /// In [`FADER_RANGE`]; the gain follows [`crate::dsp::laws::fader_gain`].
    pub fader: f32,
    /// In [`PAN_RANGE`]: -1 folds the right side into the left, 1 the left into the right.
    pub pan: f32,
    /// A muted insert outputs nothing, sends included.
    pub mute: bool,
    /// Where the insert's output goes, after its fader and pan, with the send amount (linear
    /// gain in [`SEND_RANGE`]). Sending to the master is an ordinary send. The master's own
    /// routes are empty: it is where the mix ends.
    #[serde(with = "route_list")]
    pub routes: BTreeMap<InsertId, f32>,
    /// Sorted by slot, at most one per slot.
    pub effects: Vec<Effect>,
}

impl Insert {
    /// Insert `id` as a new mixer has it: fader at unity, centered, unmuted, no effects, and
    /// sending its whole output to the master (the master itself sends nowhere).
    pub fn numbered(id: InsertId) -> Self {
        let routes = if id.is_master() {
            BTreeMap::new()
        } else {
            BTreeMap::from([(InsertId::MASTER, 1.0)])
        };
        Self {
            name: id.to_string(),
            fader: UNITY_FADER,
            pan: 0.0,
            mute: false,
            routes,
            effects: Vec::new(),
        }
    }
}

/// An effect in an insert slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Effect {
    pub id: EffectId,
    /// Below [`EFFECT_SLOTS`]; effects run in slot order.
    pub slot: u8,
    pub processor: ProcessorRef,
}

/// Which processor an effect or a plugin channel runs, and its settings, as saved.
///
/// A native processor is saved as parameter values by name. Why plain names and numbers rather
/// than the processor's own state: the document must load and save the same bytes whether or
/// not this build knows the processor. A missing key means the parameter's default; keys the
/// processor does not know are kept as they are.
///
/// A plugin (`"clap:<plugin id>"`) is saved as its display name and the plugin's own state
/// bytes as base64 text, both kept verbatim: the engine never decodes the state, so a plugin
/// that is not installed still saves byte for byte, and its name still says what is missing.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ProcessorRef {
    /// The registry key, such as `"dawsome.delay"`, or a plugin's `"<format>:<plugin id>"`.
    pub id: String,
    /// A plugin's name, for showing it even when it is missing. Native processors have none.
    #[serde(default)]
    pub name: Option<String>,
    /// A plugin's state as standard padded base64; `None` until it is first captured, so the
    /// plugin starts with its defaults.
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub params: BTreeMap<String, f32>,
}

impl ProcessorRef {
    /// Processor `id` with every parameter at its default.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: None,
            state: None,
            params: BTreeMap::new(),
        }
    }

    /// Plugin `id` (`"<format>:<plugin id>"`) named `name`, with its default state.
    pub fn plugin(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            name: Some(name.into()),
            ..Self::new(id)
        }
    }

    /// The plugin format the id names (`"clap"` for `"clap:org.example.synth"`), or `None` for
    /// a native id. Native ids never hold a colon.
    pub fn format(&self) -> Option<&str> {
        self.id.split_once(':').map(|(format, _)| format)
    }

    /// The name to show: the saved name, else the id.
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }
}

impl Serialize for ProcessorRef {
    /// `id`, then `name` and `state` when present, then `params`. A reference with a name or a
    /// state (a plugin) leaves out an empty `params`; one with neither (a native effect) always
    /// writes it, so every file saved before plugins existed keeps its bytes.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Written<'a> {
            id: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            name: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            state: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            params: Option<&'a BTreeMap<String, f32>>,
        }
        let plugin_like = self.name.is_some() || self.state.is_some();
        Written {
            id: &self.id,
            name: self.name.as_deref(),
            state: self.state.as_deref(),
            params: (!plugin_like || !self.params.is_empty()).then_some(&self.params),
        }
        .serialize(serializer)
    }
}

impl Default for Mixer {
    /// The master and [`DEFAULT_INSERTS`] inserts as [`Insert::numbered`] makes them, nothing
    /// soloed, effect IDs from 1.
    fn default() -> Self {
        Self {
            next_effect_id: EffectId(1),
            solo: BTreeSet::new(),
            inserts: (0..=DEFAULT_INSERTS)
                .map(|id| Insert::numbered(InsertId(id)))
                .collect(),
        }
    }
}

impl Mixer {
    pub(super) fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Moves each effect id that starts with [`FORMER_NATIVE_ID_PREFIX`] to
    /// [`NATIVE_ID_PREFIX`]; plugin ids (`"<format>:<plugin id>"`) never start with it.
    pub(super) fn adopt_former_native_ids(&mut self) {
        let effects = self
            .inserts
            .iter_mut()
            .flat_map(|insert| &mut insert.effects);
        for processor in effects.map(|effect| &mut effect.processor) {
            if let Some(name) = processor.id.strip_prefix(FORMER_NATIVE_ID_PREFIX) {
                processor.id = format!("{NATIVE_ID_PREFIX}{name}");
            }
        }
    }

    pub fn get(&self, id: InsertId) -> Option<&Insert> {
        self.inserts.get(id.index())
    }

    pub fn get_mut(&mut self, id: InsertId) -> Option<&mut Insert> {
        self.inserts.get_mut(id.index())
    }

    /// The master and every insert, by position.
    pub fn ids(&self) -> impl Iterator<Item = InsertId> + '_ {
        // `validate` caps the count far below u16::MAX; a longer list stops at the cap.
        (0..=u16::MAX)
            .zip(&self.inserts)
            .map(|(id, _)| InsertId(id))
    }

    /// The effect with this ID and the insert it is in.
    pub fn effect(&self, id: EffectId) -> Option<(InsertId, &Effect)> {
        self.ids().zip(&self.inserts).find_map(|(insert, strip)| {
            let effect = strip.effects.iter().find(|effect| effect.id == id)?;
            Some((insert, effect))
        })
    }

    /// Whether `from`'s sound reaches `to` through one or more sends.
    pub fn feeds(&self, from: InsertId, to: InsertId) -> bool {
        let mut seen = vec![false; self.inserts.len()];
        let mut stack = vec![from];
        while let Some(at) = stack.pop() {
            let Some(insert) = self.get(at) else {
                continue;
            };
            for &next in insert.routes.keys() {
                if next == to {
                    return true;
                }
                if let Some(seen) = seen.get_mut(next.index()) {
                    if !*seen {
                        *seen = true;
                        stack.push(next);
                    }
                }
            }
        }
        false
    }

    /// The order the inserts are mixed in: inserts 1 and up, each after every insert that sends
    /// to it (the lowest ready ID first, so the order is fixed by the routes alone), then the
    /// master. Sends to the master or to an insert that does not exist are left out of the
    /// sort: the master comes last anyway, and `validate` refuses the others.
    ///
    /// Fails with the loop, as the IDs around it starting from the lowest and ending where it
    /// started (`[1, 2, 3, 1]`), when the sends form one.
    pub fn schedule(&self) -> Result<Vec<InsertId>, Vec<InsertId>> {
        let count = self.inserts.len();
        let inside = |id: &InsertId| (1..count).contains(&id.index());
        // How many senders of each insert are still to be mixed.
        let mut waiting = vec![0_usize; count];
        for insert in self.inserts.iter().skip(1) {
            for to in insert.routes.keys().filter(|to| inside(to)) {
                waiting[to.index()] += 1;
            }
        }
        let mut ready: BTreeSet<InsertId> = self
            .ids()
            .skip(1)
            .filter(|id| waiting[id.index()] == 0)
            .collect();
        let mut order = Vec::with_capacity(count);
        while let Some(id) = ready.pop_first() {
            order.push(id);
            for &to in self.inserts[id.index()]
                .routes
                .keys()
                .filter(|to| inside(to))
            {
                waiting[to.index()] -= 1;
                if waiting[to.index()] == 0 {
                    ready.insert(to);
                }
            }
        }
        if order.len() + 1 < count {
            return Err(self.loop_among(&waiting));
        }
        order.push(InsertId::MASTER);
        Ok(order)
    }

    /// A loop among the inserts that `schedule` could not place (`waiting` above 0). Each of
    /// them has a sender that could not be placed either, so walking from sender to sender
    /// must come back to an insert already passed: the walk from there on is a loop, backwards.
    fn loop_among(&self, waiting: &[usize]) -> Vec<InsertId> {
        let stuck = |id: InsertId| waiting.get(id.index()).is_some_and(|&left| left > 0);
        let sender_of = |to: InsertId| {
            self.ids()
                .zip(&self.inserts)
                .find(|&(from, insert)| stuck(from) && insert.routes.contains_key(&to))
                .map(|(from, _)| from)
        };
        let Some(start) = self.ids().skip(1).find(|&id| stuck(id)) else {
            return Vec::new();
        };
        let mut walked = vec![start];
        while let Some(sender) = walked.last().copied().and_then(sender_of) {
            if let Some(again) = walked.iter().position(|&id| id == sender) {
                let mut found: Vec<InsertId> = walked.split_off(again);
                found.reverse();
                // From the lowest ID, back to it, so the same loop always reads the same way.
                let lowest = found
                    .iter()
                    .enumerate()
                    .min_by_key(|&(_, id)| *id)
                    .map_or(0, |(at, _)| at);
                found.rotate_left(lowest);
                found.push(found[0]);
                return found;
            }
            walked.push(sender);
        }
        walked
    }

    /// Whether solo lets each insert be heard, by position. With nothing soloed every insert
    /// can be; otherwise only the soloed inserts, every insert whose sound reaches one of them
    /// (so the soloed insert still gets its input) and every insert one of them reaches (so it
    /// still gets out). The master is always heard. Mute is separate: a muted insert is silent
    /// whatever this says.
    pub fn solo_audible(&self) -> Vec<bool> {
        let count = self.inserts.len();
        if self.solo.is_empty() {
            return vec![true; count];
        }
        let mut senders: Vec<Vec<InsertId>> = vec![Vec::new(); count];
        for (from, insert) in self.ids().zip(&self.inserts) {
            for to in insert.routes.keys() {
                if let Some(list) = senders.get_mut(to.index()) {
                    list.push(from);
                }
            }
        }
        let downstream = self.reach_from_solo(|at| {
            self.get(at)
                .map(|insert| insert.routes.keys().copied().collect())
                .unwrap_or_default()
        });
        let upstream =
            self.reach_from_solo(|at| senders.get(at.index()).cloned().unwrap_or_default());
        let mut audible: Vec<bool> = downstream
            .iter()
            .zip(&upstream)
            .map(|(&down, &up)| down || up)
            .collect();
        if let Some(master) = audible.first_mut() {
            *master = true;
        }
        audible
    }

    /// By position, whether an insert is soloed or reached from a soloed one by repeatedly
    /// stepping to the inserts `next` names.
    fn reach_from_solo(&self, next: impl Fn(InsertId) -> Vec<InsertId>) -> Vec<bool> {
        let mut reached = vec![false; self.inserts.len()];
        let mut stack: Vec<InsertId> = self.solo.iter().copied().collect();
        while let Some(at) = stack.pop() {
            match reached.get_mut(at.index()) {
                Some(seen) if !*seen => *seen = true,
                _ => continue,
            }
            stack.extend(next(at));
        }
        reached
    }

    /// Checks the mixer's rules, and that every channel plays into an insert that exists.
    pub(super) fn validate(
        &self,
        channels: &BTreeMap<ChannelId, Channel>,
    ) -> Result<(), ValidationError> {
        let besides_master = self.inserts.len().saturating_sub(1);
        if !(1..=usize::from(MAX_INSERTS)).contains(&besides_master) {
            return Err(ValidationError::InsertCount(besides_master));
        }
        let mut effect_ids = BTreeSet::new();
        for (id, insert) in self.ids().zip(&self.inserts) {
            self.validate_insert(id, insert, &mut effect_ids)?;
        }
        if let Some(&id) = self.solo.iter().find(|id| self.get(**id).is_none()) {
            return Err(ValidationError::UnknownSolo(id.0));
        }
        if let Some(channel) = channels
            .values()
            .find(|channel| self.get(channel.insert).is_none())
        {
            return Err(ValidationError::UnknownChannelInsert {
                channel: channel.name.clone(),
                insert: channel.insert.0,
            });
        }
        self.schedule()
            .map(drop)
            .map_err(|path| ValidationError::RoutingCycle {
                path: path.iter().map(|id| id.0).collect(),
            })
    }

    fn validate_insert(
        &self,
        id: InsertId,
        insert: &Insert,
        effect_ids: &mut BTreeSet<EffectId>,
    ) -> Result<(), ValidationError> {
        // `contains` is false for NaN, which is not a position.
        if !FADER_RANGE.contains(&insert.fader) {
            return Err(ValidationError::InvalidFader {
                insert: id.0,
                fader: insert.fader,
            });
        }
        if !PAN_RANGE.contains(&insert.pan) {
            return Err(ValidationError::InvalidInsertPan {
                insert: id.0,
                pan: insert.pan,
            });
        }
        if id.is_master() && !insert.routes.is_empty() {
            return Err(ValidationError::MasterSends);
        }
        for (&to, &amount) in &insert.routes {
            if self.get(to).is_none() {
                return Err(ValidationError::UnknownSendTarget {
                    from: id.0,
                    to: to.0,
                });
            }
            if to == id {
                return Err(ValidationError::SendToItself(id.0));
            }
            if !SEND_RANGE.contains(&amount) {
                return Err(ValidationError::InvalidSendAmount {
                    from: id.0,
                    to: to.0,
                    amount,
                });
            }
        }
        if insert.effects.len() > usize::from(EFFECT_SLOTS) {
            return Err(ValidationError::TooManyEffects {
                insert: id.0,
                count: insert.effects.len(),
            });
        }
        let mut previous_slot = None;
        for effect in &insert.effects {
            let in_order = previous_slot.is_none_or(|previous| effect.slot > previous);
            if effect.slot >= EFFECT_SLOTS || !in_order {
                return Err(ValidationError::InvalidEffectSlot {
                    insert: id.0,
                    slot: effect.slot,
                });
            }
            previous_slot = Some(effect.slot);
            if effect.id >= self.next_effect_id {
                return Err(ValidationError::UnallocatedEffectId {
                    id: effect.id.0,
                    next: self.next_effect_id.0,
                });
            }
            if !effect_ids.insert(effect.id) {
                return Err(ValidationError::EffectIdTwice(effect.id.0));
            }
            if let Some((key, &value)) = effect
                .processor
                .params
                .iter()
                .find(|(_, value)| !value.is_finite())
            {
                return Err(ValidationError::InvalidEffectParam {
                    effect: effect.id.0,
                    key: key.clone(),
                    value,
                });
            }
        }
        Ok(())
    }
}

/// `[1, 2, 3, 1]` as "1 → 2 → 3 → 1", for messages about a routing loop.
pub fn loop_text(path: &[u16]) -> String {
    let ids: Vec<String> = path.iter().map(u16::to_string).collect();
    ids.join(" → ")
}

/// Serde for the inserts: a JSON array of objects whose first field, `"id"`, must be the
/// insert's position, so the file always says which insert is which.
mod position_list {
    use serde::de::Error as _;
    use serde::ser::{Error as _, SerializeSeq};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::Insert;

    #[derive(Serialize)]
    struct EntryRef<'a> {
        id: u16,
        #[serde(flatten)]
        insert: &'a Insert,
    }

    #[derive(Deserialize)]
    struct Entry {
        id: u16,
        #[serde(flatten)]
        insert: Insert,
    }

    pub(super) fn serialize<S: Serializer>(
        inserts: &[Insert],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(inserts.len()))?;
        for (position, insert) in inserts.iter().enumerate() {
            let id = u16::try_from(position).map_err(|_| S::Error::custom("too many inserts"))?;
            seq.serialize_element(&EntryRef { id, insert })?;
        }
        seq.end()
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<Insert>, D::Error> {
        let entries = Vec::<Entry>::deserialize(deserializer)?;
        let mut inserts = Vec::with_capacity(entries.len());
        for (position, entry) in entries.into_iter().enumerate() {
            if usize::from(entry.id) != position {
                let message = format_args!(
                    "insert {} is listed at position {position}; inserts are listed in order, the master (0) first",
                    entry.id
                );
                return Err(D::Error::custom(message));
            }
            inserts.push(entry.insert);
        }
        Ok(inserts)
    }
}

/// Serde for an insert's sends: a JSON array of `{"to": id, "amount": gain}` sorted by `to`.
/// Loading rejects a target listed twice.
mod route_list {
    use std::collections::BTreeMap;

    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::InsertId;

    #[derive(Serialize, Deserialize)]
    struct Route {
        to: InsertId,
        amount: f32,
    }

    pub(super) fn serialize<S: Serializer>(
        routes: &BTreeMap<InsertId, f32>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(routes.iter().map(|(&to, &amount)| Route { to, amount }))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<InsertId, f32>, D::Error> {
        let mut routes = BTreeMap::new();
        for Route { to, amount } in Vec::<Route>::deserialize(deserializer)? {
            if routes.insert(to, amount).is_some() {
                let message = format_args!("the send to insert {} appears twice", to.0);
                return Err(D::Error::custom(message));
            }
        }
        Ok(routes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{LoadError, Project};

    const P5_CHAIN: &str = include_str!("../../../../tests/fixtures/p5_chain.json");
    const P5_CYCLE: &str = include_str!("../../../../tests/fixtures/p5_cycle.json");
    const P5_SOLO: &str = include_str!("../../../../tests/fixtures/p5_solo.json");
    const P5_GAIN: &str = include_str!("../../../../tests/fixtures/p5_gain.json");
    const P5_DELAY_SYNC: &str = include_str!("../../../../tests/fixtures/p5_delay_sync.json");

    fn load(text: &str) -> Project {
        Project::load_json(text).expect("fixture loads")
    }

    fn ids(list: &[u16]) -> Vec<InsertId> {
        list.iter().copied().map(InsertId).collect()
    }

    /// p5_gain with one change to its mixer, validated.
    fn validated(change: impl FnOnce(&mut Mixer)) -> Result<(), ValidationError> {
        let mut project = load(P5_GAIN);
        change(&mut project.mixer);
        project.validate()
    }

    // A new mixer: master plus 16 inserts, "Insert k", fader 0.8, centered, unmuted, every insert
    // sending everything to the master, which sends nowhere.
    #[test]
    fn a_new_mixer_has_the_master_and_16_inserts_sending_to_it() {
        let mixer = Mixer::default();
        assert_eq!(mixer.inserts.len(), 17);
        assert_eq!(mixer.next_effect_id, EffectId(1));
        assert!(mixer.solo.is_empty());
        let master = &mixer.inserts[0];
        assert_eq!(master.name, "Master");
        assert!(master.routes.is_empty());
        for (k, insert) in mixer.inserts.iter().enumerate().skip(1) {
            assert_eq!(insert.name, format!("Insert {k}"));
            assert_eq!((insert.fader, insert.pan, insert.mute), (0.8, 0.0, false));
            assert_eq!(insert.routes, BTreeMap::from([(InsertId::MASTER, 1.0)]));
            assert!(insert.effects.is_empty());
        }
    }

    // p5_chain sends 3 → 2 → 1 → master: 3 is ready first, then 2, then 1, then 4..16 in id
    // order, and the master last.
    #[test]
    fn the_schedule_mixes_each_insert_after_its_senders() {
        let chain = load(P5_CHAIN);
        let mut expected = ids(&[3, 2, 1]);
        expected.extend((4..=16).map(InsertId));
        expected.push(InsertId::MASTER);
        assert_eq!(chain.mixer.schedule(), Ok(expected));
        let mut plain: Vec<InsertId> = (1..=16).map(InsertId).collect();
        plain.push(InsertId::MASTER);
        assert_eq!(Mixer::default().schedule(), Ok(plain));
    }

    // p5_cycle sends 1 → 2 → 3 → 1 (and 1 → master): the loop reads from its lowest insert.
    #[test]
    fn a_routing_loop_is_refused_with_its_path() {
        let error = Project::load_json(P5_CYCLE).expect_err("a loop is refused");
        let LoadError::Invalid(ValidationError::RoutingCycle { path }) = &error else {
            panic!("wrong error {error:?}");
        };
        assert_eq!(path, &[1, 2, 3, 1]);
        assert_eq!(
            error.to_string(),
            "routing cycle through inserts 1 → 2 → 3 → 1"
        );
        // A loop reached through an insert outside it still reads from its own lowest insert.
        let mut mixer = Mixer::default();
        mixer.inserts[5].routes.insert(InsertId(4), 1.0);
        mixer.inserts[4].routes.insert(InsertId(6), 1.0);
        mixer.inserts[6].routes.insert(InsertId(4), 1.0);
        assert_eq!(mixer.schedule(), Err(ids(&[4, 6, 4])));
    }

    // Solo on p5_solo (solo {3}; 5 → 3 → {master, 4}): 3 itself, 5 upstream, 4 downstream and the
    // master are heard; 1, 2 and 6..16 are not.
    #[test]
    fn solo_keeps_the_inserts_that_feed_and_are_fed_by_the_soloed_one() {
        let solo = load(P5_SOLO);
        let audible: Vec<u16> = solo
            .mixer
            .ids()
            .zip(solo.mixer.solo_audible())
            .filter(|(_, heard)| *heard)
            .map(|(id, _)| id.0)
            .collect();
        assert_eq!(audible, [0, 3, 4, 5]);
        assert!(Mixer::default().solo_audible().iter().all(|&heard| heard));
    }

    #[test]
    fn feeds_follows_sends_transitively() {
        let chain = load(P5_CHAIN).mixer;
        assert!(chain.feeds(InsertId(3), InsertId(1)));
        assert!(chain.feeds(InsertId(3), InsertId::MASTER));
        assert!(!chain.feeds(InsertId(1), InsertId(3)));
        assert!(!chain.feeds(InsertId(3), InsertId(3)));
        assert!(!chain.feeds(InsertId::MASTER, InsertId(1)));
    }

    #[test]
    fn mixer_rules_are_checked() {
        use ValidationError as V;
        assert_eq!(validated(|_| {}), Ok(()));
        assert_eq!(validated(|m| m.inserts.truncate(1)), Err(V::InsertCount(0)));
        assert_eq!(
            validated(|m| m.inserts.resize(502, Insert::numbered(InsertId(9)))),
            Err(V::InsertCount(501))
        );
        assert_eq!(
            validated(|m| m.inserts.resize(501, Insert::numbered(InsertId(9)))),
            Ok(())
        );
        assert_eq!(
            validated(|m| m.inserts[2].fader = 1.5),
            Err(V::InvalidFader {
                insert: 2,
                fader: 1.5
            })
        );
        assert!(matches!(
            validated(|m| m.inserts[2].pan = f32::NAN),
            Err(V::InvalidInsertPan { insert: 2, .. })
        ));
        assert_eq!(
            validated(|m| {
                m.inserts[0].routes.insert(InsertId(1), 1.0);
            }),
            Err(V::MasterSends)
        );
        assert_eq!(
            validated(|m| {
                m.inserts[1].routes.insert(InsertId(17), 1.0);
            }),
            Err(V::UnknownSendTarget { from: 1, to: 17 })
        );
        assert_eq!(
            validated(|m| {
                m.inserts[1].routes.insert(InsertId(1), 1.0);
            }),
            Err(V::SendToItself(1))
        );
        assert_eq!(
            validated(|m| {
                m.inserts[1].routes.insert(InsertId(2), 1.5);
            }),
            Err(V::InvalidSendAmount {
                from: 1,
                to: 2,
                amount: 1.5
            })
        );
        assert_eq!(
            validated(|m| {
                m.solo.insert(InsertId(17));
            }),
            Err(V::UnknownSolo(17))
        );
    }

    #[test]
    fn effect_rules_are_checked() {
        use ValidationError as V;
        let delay = |id, slot| Effect {
            id: EffectId(id),
            slot,
            processor: ProcessorRef::new("dawsome.delay"),
        };
        let with = |effects: Vec<Effect>, next| {
            validated(move |m| {
                m.inserts[1].effects = effects;
                m.next_effect_id = EffectId(next);
            })
        };
        assert_eq!(with(vec![delay(1, 0), delay(2, 9)], 3), Ok(()));
        assert_eq!(
            with(vec![delay(1, 10)], 2),
            Err(V::InvalidEffectSlot {
                insert: 1,
                slot: 10
            })
        );
        assert_eq!(
            with(vec![delay(1, 3), delay(2, 3)], 3),
            Err(V::InvalidEffectSlot { insert: 1, slot: 3 })
        );
        assert_eq!(
            with(vec![delay(2, 3), delay(1, 1)], 3),
            Err(V::InvalidEffectSlot { insert: 1, slot: 1 })
        );
        assert_eq!(
            with(vec![delay(2, 0)], 2),
            Err(V::UnallocatedEffectId { id: 2, next: 2 })
        );
        assert_eq!(
            with(
                (0..11)
                    .map(|slot| delay(u32::from(slot) + 1, slot))
                    .collect(),
                20
            ),
            Err(V::TooManyEffects {
                insert: 1,
                count: 11
            })
        );
        assert_eq!(
            validated(|m| {
                m.inserts[1].effects = vec![delay(1, 0)];
                m.inserts[2].effects = vec![delay(1, 0)];
                m.next_effect_id = EffectId(2);
            }),
            Err(V::EffectIdTwice(1))
        );
        let mut infinite = delay(1, 0);
        infinite
            .processor
            .params
            .insert("mix".into(), f32::INFINITY);
        assert!(matches!(
            with(vec![infinite], 2),
            Err(V::InvalidEffectParam { effect: 1, .. })
        ));
    }

    #[test]
    fn a_channel_must_play_into_an_insert_that_exists() {
        let mut project = load(P5_GAIN);
        let channel = project.channels.values_mut().next().expect("a channel");
        channel.insert = InsertId(17);
        assert!(matches!(
            project.validate(),
            Err(ValidationError::UnknownChannelInsert { insert: 17, .. })
        ));
    }

    // The file names inserts by position: an id out of place, or a send listed twice, is not a
    // project.
    #[test]
    fn inserts_are_listed_by_position_and_sends_once_each() {
        let master = r#"{"id":0,"name":"Master""#;
        assert!(P5_GAIN.contains(master));
        let moved = P5_GAIN.replacen(master, r#"{"id":1,"name":"Master""#, 1);
        assert!(matches!(
            Project::load_json(&moved),
            Err(LoadError::Json(_))
        ));
        let send = r#""routes":[{"to":0,"amount":1.0}]"#;
        assert!(P5_GAIN.contains(send));
        let twice = P5_GAIN.replacen(
            send,
            r#""routes":[{"to":0,"amount":1.0},{"to":0,"amount":0.5}]"#,
            1,
        );
        assert!(matches!(
            Project::load_json(&twice),
            Err(LoadError::Json(_))
        ));
    }

    // A file saved with the former native-id prefix reads as the same project, and saves back
    // to the current file's bytes.
    #[test]
    fn a_native_id_saved_under_the_former_prefix_loads_under_the_current_one() {
        assert!(P5_DELAY_SYNC.contains(&format!("\"{NATIVE_ID_PREFIX}delay\"")));
        let former = P5_DELAY_SYNC.replace(NATIVE_ID_PREFIX, FORMER_NATIVE_ID_PREFIX);
        let project = load(&former);
        assert_eq!(project, load(P5_DELAY_SYNC));
        assert_eq!(
            project.to_json().expect("serializes"),
            P5_DELAY_SYNC.trim_end()
        );
    }

    // Only a native id moves: a plugin id holding the former prefix after its format stays.
    #[test]
    fn a_plugin_id_holding_the_former_prefix_is_kept() {
        let plugin_id = format!("clap:{FORMER_NATIVE_ID_PREFIX}synth");
        let mut mixer = Mixer::default();
        mixer.inserts[1].effects = vec![
            Effect {
                id: EffectId(1),
                slot: 0,
                processor: ProcessorRef::new(format!("{FORMER_NATIVE_ID_PREFIX}reverb")),
            },
            Effect {
                id: EffectId(2),
                slot: 1,
                processor: ProcessorRef::plugin(plugin_id.clone(), "Synth"),
            },
        ];
        mixer.adopt_former_native_ids();
        let ids: Vec<&str> = mixer.inserts[1]
            .effects
            .iter()
            .map(|effect| effect.processor.id.as_str())
            .collect();
        assert_eq!(ids, [format!("{NATIVE_ID_PREFIX}reverb"), plugin_id]);
    }
}
