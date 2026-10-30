//! Controller links: which MIDI controller (a CC number, on one MIDI channel or on any) moves
//! which parameter. A link names its parameter as an automation does, by the stable IDs of what
//! it belongs to, so it survives edits; one whose parameter is gone does nothing, as such an
//! automation does.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::{AutomationTarget, ValidationError};

/// The highest CC number a link can listen to. Why not 127: CC 120 to 127 are the channel mode
/// messages (all sound off, reset all controllers, local control, all notes off, omni and poly
/// modes), which no control knob sends.
pub const MAX_CONTROLLER: u8 = 119;

/// MIDI channels, 0 to 15 as the status byte counts them (shown as 1 to 16).
pub const MIDI_CHANNELS: u8 = 16;

/// A controller on a MIDI device: a CC number, on one MIDI channel or, `None`, on any.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Controller {
    pub cc: u8,
    pub channel: Option<u8>,
}

impl Controller {
    /// Whether a CC message `cc` on MIDI channel `channel` comes from this controller.
    pub fn hears(self, cc: u8, channel: u8) -> bool {
        self.cc == cc && self.channel.is_none_or(|own| own == channel)
    }
}

/// The project's controller links, at most one per parameter. Written as a list sorted by
/// parameter, `[{"cc":7,"channel":null,"target":{"insert_fader":1}}]`, and left out when empty,
/// so projects without links keep their bytes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MidiLinks(BTreeMap<AutomationTarget, Controller>);

impl MidiLinks {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The controller linked to `target`.
    pub fn get(&self, target: &AutomationTarget) -> Option<Controller> {
        self.0.get(target).copied()
    }

    /// Every linked parameter, in the order the file lists them.
    pub fn targets(&self) -> impl Iterator<Item = &AutomationTarget> {
        self.0.keys()
    }

    /// Links `target` to `controller`, returning the controller it had. The edits keep the one
    /// target per controller rule; this does not.
    pub fn insert(
        &mut self,
        target: AutomationTarget,
        controller: Controller,
    ) -> Option<Controller> {
        self.0.insert(target, controller)
    }

    /// Unlinks `target`, returning the controller it had.
    pub fn remove(&mut self, target: &AutomationTarget) -> Option<Controller> {
        self.0.remove(target)
    }

    /// The parameter `controller` (exactly that CC on exactly that channel, or on any) moves.
    pub fn target_of(&self, controller: Controller) -> Option<&AutomationTarget> {
        self.0
            .iter()
            .find(|(_, linked)| **linked == controller)
            .map(|(target, _)| target)
    }

    /// The parameters a CC message `cc` on MIDI channel `channel` moves: the links on that
    /// channel and the links on any channel.
    pub fn matching(&self, cc: u8, channel: u8) -> impl Iterator<Item = AutomationTarget> + '_ {
        self.0
            .iter()
            .filter(move |(_, controller)| controller.hears(cc, channel))
            .map(|(target, _)| target.clone())
    }

    /// Checks the rules [`super::Project::validate`] applies: CC numbers up to
    /// [`MAX_CONTROLLER`], channels below [`MIDI_CHANNELS`], one parameter per controller.
    pub(super) fn validate(&self) -> Result<(), ValidationError> {
        let mut seen = BTreeSet::new();
        for &controller in self.0.values() {
            let Controller { cc, channel } = controller;
            if cc > MAX_CONTROLLER || channel.is_some_and(|channel| channel >= MIDI_CHANNELS) {
                return Err(ValidationError::InvalidMidiLink { cc, channel });
            }
            if !seen.insert(controller) {
                return Err(ValidationError::ControllerLinkedTwice { cc, channel });
            }
        }
        Ok(())
    }
}

/// A link as the file writes it.
#[derive(Serialize, Deserialize)]
struct LinkFile {
    cc: u8,
    channel: Option<u8>,
    target: AutomationTarget,
}

impl Serialize for MidiLinks {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.0.iter().map(|(target, controller)| LinkFile {
            cc: controller.cc,
            channel: controller.channel,
            target: target.clone(),
        }))
    }
}

impl<'de> Deserialize<'de> for MidiLinks {
    /// Refuses a parameter listed twice: it has one link at most.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut links = BTreeMap::new();
        for LinkFile {
            cc,
            channel,
            target,
        } in Vec::<LinkFile>::deserialize(deserializer)?
        {
            if links.insert(target, Controller { cc, channel }).is_some() {
                return Err(serde::de::Error::custom("a parameter is linked twice"));
            }
        }
        Ok(Self(links))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{ChannelId, InsertId};

    #[test]
    fn a_controller_on_any_channel_hears_every_channel() {
        let any = Controller {
            cc: 7,
            channel: None,
        };
        let first = Controller {
            cc: 7,
            channel: Some(0),
        };
        assert!(any.hears(7, 0) && any.hears(7, 15));
        assert!(first.hears(7, 0) && !first.hears(7, 1) && !first.hears(8, 0));
    }

    #[test]
    fn links_write_as_a_list_by_parameter_and_read_back() {
        let mut links = MidiLinks::default();
        links.insert(
            AutomationTarget::InsertFader(InsertId(1)),
            Controller {
                cc: 7,
                channel: None,
            },
        );
        links.insert(
            AutomationTarget::ChannelPan(ChannelId(1)),
            Controller {
                cc: 10,
                channel: Some(0),
            },
        );
        let text = serde_json::to_string(&links).expect("serializes");
        assert_eq!(
            text,
            r#"[{"cc":10,"channel":0,"target":{"channel_pan":1}},{"cc":7,"channel":null,"target":{"insert_fader":1}}]"#
        );
        assert_eq!(
            serde_json::from_str::<MidiLinks>(&text).expect("reads"),
            links
        );
        let twice = r#"[{"cc":1,"channel":null,"target":{"insert_fader":1}},{"cc":2,"channel":null,"target":{"insert_fader":1}}]"#;
        assert!(serde_json::from_str::<MidiLinks>(twice).is_err());
    }
}
