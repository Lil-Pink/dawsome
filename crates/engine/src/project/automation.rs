//! Automation: one point list per automated parameter, played by automation clips on the
//! playlist. A point's value is normalized over the parameter's range (0 is its minimum, 1 its
//! maximum), and its curve shapes the segment from it to the next point.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};

use super::{id_list, ChannelId, EffectId, InsertId, ValidationError};
use crate::time::Ticks;

/// Stable ID of an automation. New IDs come from [`Automations::next_id`] and are never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AutomationId(pub u32);

/// The most automations a project holds.
pub const MAX_AUTOMATIONS: usize = 1024;

/// Point values: normalized over the parameter's range.
pub const VALUE_RANGE: RangeInclusive<f32> = 0.0..=1.0;

/// Tensions of an exponential segment: below 0 it starts fast, above 0 slow.
pub const TENSION_RANGE: RangeInclusive<f32> = -1.0..=1.0;

/// How steep a tension of 1 makes an exponential segment: f(x) = (e^(6kx) − 1)/(e^(6k) − 1).
/// At k = 1 the midpoint sits at 1/(e^3 + 1), 4.7 % of the way.
pub const EXP_STEEPNESS: f64 = 6.0;

/// Tensions smaller than this in size shape a straight line. Why: below it the exponential
/// formula's rounding error is larger than the curve's own bend (both under 1e-8 of the segment),
/// and below about 1.85e-17 it is 0/0. tests/fixtures/gen_p8_expected.py uses the same bound.
pub const TENSION_EPSILON: f64 = 1e-8;

/// The project's automations. Written only when it is not [`Automations::default`], so
/// projects without automation keep their bytes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Automations {
    /// The ID the next new automation gets: above every ID ever given out.
    pub next_id: AutomationId,
    #[serde(with = "id_list")]
    pub list: BTreeMap<AutomationId, Automation>,
}

impl Default for Automations {
    fn default() -> Self {
        Self {
            next_id: AutomationId(1),
            list: BTreeMap::new(),
        }
    }
}

impl Automations {
    pub(super) fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The automation of `target`, if it has one (a target has at most one).
    pub fn of_target(&self, target: &AutomationTarget) -> Option<AutomationId> {
        self.list
            .iter()
            .find(|(_, automation)| automation.target == *target)
            .map(|(&id, _)| id)
    }

    /// Checks the rules [`super::Project::validate`] applies to the automations.
    pub(super) fn validate(&self) -> Result<(), ValidationError> {
        if self.list.len() > MAX_AUTOMATIONS {
            return Err(ValidationError::TooManyAutomations(self.list.len()));
        }
        let mut targets = BTreeSet::new();
        for (&id, automation) in &self.list {
            if id >= self.next_id {
                return Err(ValidationError::UnallocatedAutomationId {
                    id: id.0,
                    next: self.next_id.0,
                });
            }
            check_points(id, &automation.points)?;
            if !targets.insert(&automation.target) {
                return Err(ValidationError::TargetAutomatedTwice { automation: id.0 });
            }
        }
        Ok(())
    }
}

/// The rules a point list follows: at least one point, ticks strictly increasing from tick 0
/// or later, values finite in [`VALUE_RANGE`], tensions finite in [`TENSION_RANGE`]. The edits
/// check the lists they make with it too.
pub(crate) fn check_points(
    id: AutomationId,
    points: &[AutomationPoint],
) -> Result<(), ValidationError> {
    if points.is_empty() {
        return Err(ValidationError::NoAutomationPoints { automation: id.0 });
    }
    let mut previous: Option<Ticks> = None;
    for point in points {
        if point.tick.0 < 0 || previous.is_some_and(|tick| point.tick <= tick) {
            return Err(ValidationError::AutomationPointOutOfOrder {
                automation: id.0,
                tick: point.tick.0,
            });
        }
        previous = Some(point.tick);
        // `contains` is false for NaN and infinity.
        if !VALUE_RANGE.contains(&point.value) {
            return Err(ValidationError::InvalidAutomationValue {
                automation: id.0,
                tick: point.tick.0,
                value: point.value,
            });
        }
        if let Curve::Exp(tension) = point.curve {
            if !TENSION_RANGE.contains(&tension) {
                return Err(ValidationError::InvalidTension {
                    automation: id.0,
                    tick: point.tick.0,
                    tension,
                });
            }
        }
    }
    Ok(())
}

/// One automated parameter's points, in ticks from the start of the automation's data. An
/// automation clip shows a window of them, like a pattern clip shows its pattern, but without
/// repeating: past the last point the value holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Automation {
    pub target: AutomationTarget,
    /// Sorted by tick, at least one, one per tick.
    pub points: Vec<AutomationPoint>,
}

impl Automation {
    /// The point on `tick`, and its index.
    pub fn point_at(&self, tick: Ticks) -> Option<(usize, &AutomationPoint)> {
        let index = self
            .points
            .binary_search_by_key(&tick, |point| point.tick)
            .ok()?;
        Some((index, &self.points[index]))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AutomationPoint {
    pub tick: Ticks,
    /// In [`VALUE_RANGE`].
    pub value: f32,
    /// The shape of the segment that starts here. Written only when it is not linear.
    #[serde(default, skip_serializing_if = "Curve::is_linear")]
    pub curve: Curve,
}

impl AutomationPoint {
    /// A point at `tick` with `value`, starting a straight segment.
    pub fn linear(tick: Ticks, value: f32) -> Self {
        Self {
            tick,
            value,
            curve: Curve::Linear,
        }
    }
}

/// How a segment runs from its point's value to the next point's: v = v0 + (v1 − v0)·f(x) for
/// x from 0 at the segment's start to 1 at its end.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Curve {
    /// f(x) = x.
    #[default]
    Linear,
    /// f(x) = 0: the value steps to the next point's on its tick.
    Hold,
    /// f(x) = x²(3 − 2x), smoothstep.
    Smooth,
    /// f(x) = (e^(6kx) − 1)/(e^(6k) − 1) for tension k in [`TENSION_RANGE`]; k = 0 is linear.
    Exp(f32),
}

impl Curve {
    /// Takes a reference so serde can call it to leave the default out.
    pub fn is_linear(&self) -> bool {
        *self == Curve::Linear
    }

    /// f(x) for x in 0..=1, in f64.
    ///
    /// Why exactly x at tension 0 and below [`TENSION_EPSILON`]: (e^0 − 1)/(e^0 − 1) is 0/0,
    /// and an untouched handle must draw and play the straight line bit for bit.
    pub fn shape(self, x: f64) -> f64 {
        match self {
            Curve::Linear => x,
            Curve::Hold => 0.0,
            Curve::Smooth => x * x * (3.0 - 2.0 * x),
            Curve::Exp(tension) if f64::from(tension).abs() < TENSION_EPSILON => x,
            // Written as the formula reads, e^y − 1 rather than `exp_m1`, so the known answers
            // computed from the formula match bit for bit.
            Curve::Exp(tension) => {
                let steepness = EXP_STEEPNESS * f64::from(tension);
                ((steepness * x).exp() - 1.0) / (steepness.exp() - 1.0)
            }
        }
    }
}

/// A parameter an automation sets, by the stable IDs of what it belongs to.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "TargetFile", into = "TargetFile")]
pub enum AutomationTarget {
    /// A channel's volume, linear gain 0 to 4.
    ChannelVolume(ChannelId),
    /// A channel's pan, -1 to 1.
    ChannelPan(ChannelId),
    /// An insert's fader position, 0 to 1.
    InsertFader(InsertId),
    /// An insert's pan, -1 to 1.
    InsertPan(InsertId),
    /// The amount of the send from one insert to another, 0 to 1.
    Send { from: InsertId, to: InsertId },
    /// A native effect's parameter, by the key it is saved under.
    EffectParam { effect: EffectId, key: String },
    /// A plugin effect's parameter, by the plugin's own number for it.
    EffectPlugin { effect: EffectId, param_id: u32 },
    /// A plugin channel's instrument parameter, by the plugin's own number for it.
    ChannelParam { channel: ChannelId, param_id: u32 },
}

/// A target as the file writes it: `{"channel_volume":1}`, `{"effect":{"id":3,"param":"mix"}}`,
/// `{"effect":{"id":3,"param_id":7}}`, `{"channel_param":{"channel":2,"param_id":7}}`.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TargetFile {
    ChannelVolume(ChannelId),
    ChannelPan(ChannelId),
    InsertFader(InsertId),
    InsertPan(InsertId),
    Send {
        from: InsertId,
        to: InsertId,
    },
    Effect {
        id: EffectId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        param: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        param_id: Option<u32>,
    },
    ChannelParam {
        channel: ChannelId,
        param_id: u32,
    },
}

impl TryFrom<TargetFile> for AutomationTarget {
    type Error = String;

    fn try_from(file: TargetFile) -> Result<Self, String> {
        Ok(match file {
            TargetFile::ChannelVolume(id) => Self::ChannelVolume(id),
            TargetFile::ChannelPan(id) => Self::ChannelPan(id),
            TargetFile::InsertFader(id) => Self::InsertFader(id),
            TargetFile::InsertPan(id) => Self::InsertPan(id),
            TargetFile::Send { from, to } => Self::Send { from, to },
            TargetFile::Effect {
                id,
                param: Some(key),
                param_id: None,
            } => Self::EffectParam { effect: id, key },
            TargetFile::Effect {
                id,
                param: None,
                param_id: Some(param_id),
            } => Self::EffectPlugin {
                effect: id,
                param_id,
            },
            TargetFile::Effect { id, .. } => {
                return Err(format!(
                    "automation target effect {}: names a param or a param_id, one of them",
                    id.0
                ))
            }
            TargetFile::ChannelParam { channel, param_id } => {
                Self::ChannelParam { channel, param_id }
            }
        })
    }
}

impl From<AutomationTarget> for TargetFile {
    fn from(target: AutomationTarget) -> Self {
        match target {
            AutomationTarget::ChannelVolume(id) => Self::ChannelVolume(id),
            AutomationTarget::ChannelPan(id) => Self::ChannelPan(id),
            AutomationTarget::InsertFader(id) => Self::InsertFader(id),
            AutomationTarget::InsertPan(id) => Self::InsertPan(id),
            AutomationTarget::Send { from, to } => Self::Send { from, to },
            AutomationTarget::EffectParam { effect, key } => Self::Effect {
                id: effect,
                param: Some(key),
                param_id: None,
            },
            AutomationTarget::EffectPlugin { effect, param_id } => Self::Effect {
                id: effect,
                param: None,
                param_id: Some(param_id),
            },
            AutomationTarget::ChannelParam { channel, param_id } => {
                Self::ChannelParam { channel, param_id }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The curve formulas: smooth is exactly 0.5 at its midpoint, exp at k = 1 is 1/(e^3 + 1) there
    // (4.7 %), and f_{−k}(x) = 1 − f_k(1 − x).
    #[test]
    fn curves_follow_their_formulas() {
        assert_eq!(Curve::Linear.shape(0.25), 0.25);
        assert_eq!(Curve::Hold.shape(0.9), 0.0);
        assert_eq!(Curve::Smooth.shape(0.5), 0.5);
        assert_eq!(Curve::Smooth.shape(1.0), 1.0);
        let midpoint = Curve::Exp(1.0).shape(0.5);
        assert!(
            (midpoint - 1.0 / (3f64.exp() + 1.0)).abs() < 1e-15,
            "{midpoint}"
        );
        for k in [0.25f32, 0.5, 1.0] {
            for x in [0.0, 0.1, 0.37, 0.5, 0.9, 1.0] {
                let mirrored = 1.0 - Curve::Exp(k).shape(1.0 - x);
                let negative = Curve::Exp(-k).shape(x);
                assert!((negative - mirrored).abs() < 1e-12, "k {k}, x {x}");
            }
        }
    }

    // 1e-20 passes validation, and e^(6·1e-20) rounds to exactly 1.0 in f64, so the formula
    // would be 0/0.
    #[test]
    fn a_vanishing_tension_is_the_straight_line() {
        for tension in [1e-20f32, -1e-20, 1e-12, -1e-9] {
            for x in [0.0, 0.1, 0.5, 0.999, 1.0] {
                assert_eq!(Curve::Exp(tension).shape(x), x, "k {tension}, x {x}");
            }
        }
    }

    #[test]
    fn a_tension_of_zero_is_the_straight_line_bit_for_bit() {
        for x in [0.0, 1e-9, 0.1, 0.333, 0.5, 0.999, 1.0] {
            assert_eq!(
                Curve::Exp(0.0).shape(x).to_bits(),
                Curve::Linear.shape(x).to_bits()
            );
        }
    }

    #[test]
    fn targets_and_curves_have_their_file_shapes() {
        let cases = [
            (
                AutomationTarget::ChannelVolume(ChannelId(1)),
                r#"{"channel_volume":1}"#,
            ),
            (
                AutomationTarget::InsertFader(InsertId(3)),
                r#"{"insert_fader":3}"#,
            ),
            (
                AutomationTarget::Send {
                    from: InsertId(1),
                    to: InsertId(2),
                },
                r#"{"send":{"from":1,"to":2}}"#,
            ),
            (
                AutomationTarget::EffectParam {
                    effect: EffectId(4),
                    key: "mix".into(),
                },
                r#"{"effect":{"id":4,"param":"mix"}}"#,
            ),
            (
                AutomationTarget::EffectPlugin {
                    effect: EffectId(4),
                    param_id: 7,
                },
                r#"{"effect":{"id":4,"param_id":7}}"#,
            ),
            (
                AutomationTarget::ChannelParam {
                    channel: ChannelId(2),
                    param_id: 9,
                },
                r#"{"channel_param":{"channel":2,"param_id":9}}"#,
            ),
        ];
        for (target, text) in cases {
            assert_eq!(serde_json::to_string(&target).expect("serializes"), text);
            assert_eq!(
                serde_json::from_str::<AutomationTarget>(text).expect("parses"),
                target
            );
        }
        let both = r#"{"effect":{"id":4,"param":"mix","param_id":7}}"#;
        assert!(serde_json::from_str::<AutomationTarget>(both).is_err());
        let points = [
            AutomationPoint::linear(Ticks(0), 0.25),
            AutomationPoint {
                curve: Curve::Exp(0.5),
                ..AutomationPoint::linear(Ticks(384), 0.0)
            },
            AutomationPoint {
                curve: Curve::Hold,
                ..AutomationPoint::linear(Ticks(768), 1.0)
            },
        ];
        assert_eq!(
            serde_json::to_string(&points).expect("serializes"),
            r#"[{"tick":0,"value":0.25},{"tick":384,"value":0.0,"curve":{"exp":0.5}},{"tick":768,"value":1.0,"curve":"hold"}]"#
        );
    }
}
