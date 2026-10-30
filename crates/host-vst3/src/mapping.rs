//! A plugin's MIDI mapping (`IMidiMapping`, asked of its controller): which parameters a
//! device's pitch bend and mod wheel move, and every parameter any MIDI controller moves. The
//! format has no MIDI messages: a host plays a bend or a wheel as a point of the parameter the
//! mapping names (the SDK's hosting rule), and a parameter any controller moves belongs to the
//! MIDI input, not to the sound, so the panels leave it out (JUCE gives every plugin 16 × 130 of
//! them, "MIDI CC 0|0" to "MIDI CC 15|129").
//!
//! Asked on the main thread after `setActive(true)` and whenever the plugin restarts with
//! `kMidiCCAssignmentChanged`; the bend's and the wheel's parameters go to the audio half in
//! atomics it reads per block.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU32, Ordering};

use vst3::Steinberg::kResultOk;
use vst3::Steinberg::Vst::{ControllerNumbers_, IMidiMapping, IMidiMappingTrait, ParamID};

use crate::parts::Parts;

/// No parameter: what the format's `kNoParamId` is.
pub(crate) const NO_PARAM: ParamID = vst3::Steinberg::Vst::kNoParamId;

/// The MIDI channels and controllers asked about: every channel of bus 0, every controller the
/// format numbers (the 128 CCs, aftertouch and pitch bend).
const CHANNELS: i16 = 16;
const CONTROLLERS: i16 = ControllerNumbers_::kCountCtrlNumber as i16;
const PITCH_BEND: i16 = ControllerNumbers_::kPitchBend as i16;
const MOD_WHEEL: i16 = ControllerNumbers_::kCtrlModWheel as i16;

/// The parameters a device's bend and wheel are sent to, [`NO_PARAM`] for none; shared by the
/// main thread, which sets them, and the audio half, which reads them.
#[derive(Debug)]
pub(crate) struct Controls {
    pub(crate) bend: AtomicU32,
    pub(crate) wheel: AtomicU32,
}

impl Controls {
    pub(crate) fn new(mapping: &Mapping) -> Self {
        let controls = Self {
            bend: AtomicU32::new(NO_PARAM),
            wheel: AtomicU32::new(NO_PARAM),
        };
        controls.set(mapping);
        controls
    }

    pub(crate) fn set(&self, mapping: &Mapping) {
        self.bend
            .store(mapping.bend.unwrap_or(NO_PARAM), Ordering::Release);
        self.wheel
            .store(mapping.wheel.unwrap_or(NO_PARAM), Ordering::Release);
    }
}

/// What a plugin's MIDI mapping says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Mapping {
    /// The parameter pitch bend on MIDI channel 1 (0 here) moves.
    pub(crate) bend: Option<ParamID>,
    /// The parameter the mod wheel (CC 1) on MIDI channel 1 moves.
    pub(crate) wheel: Option<ParamID>,
    /// Every parameter a controller on any channel of bus 0 moves.
    pub(crate) mapped: BTreeSet<ParamID>,
}

/// The mapping of `parts`' controller; none when it has no `IMidiMapping`. Why channel 1 for
/// the bend and the wheel: the app is omni, and the CLAP host sends them on MIDI channel 1, so
/// one control is one value in either format.
pub(crate) fn read(parts: &Parts) -> Mapping {
    let Some(mapping) = parts
        .controller
        .as_ref()
        .and_then(|controller| controller.cast::<IMidiMapping>())
    else {
        return Mapping::default();
    };
    let assigned = |channel: i16, controller: i16| {
        let mut id: ParamID = NO_PARAM;
        // SAFETY: the controller is live, called on the main thread; `id` is our out parameter.
        let found = unsafe { mapping.getMidiControllerAssignment(0, channel, controller, &mut id) };
        (found == kResultOk && id != NO_PARAM).then_some(id)
    };
    let mut mapped = BTreeSet::new();
    for channel in 0..CHANNELS {
        for controller in 0..CONTROLLERS {
            mapped.extend(assigned(channel, controller));
        }
    }
    Mapping {
        bend: assigned(0, PITCH_BEND),
        wheel: assigned(0, MOD_WHEEL),
        mapped,
    }
}

/// A device's bend (−8192 to 8191, held there) as the point the format carries it as:
/// (b + 8192) / 16383. Why 16383: both ends are exact, and the value decodes back to b exactly
/// through both decoders plugins use, `round(v·16383)` and `min(16383, floor(v·16384))`
/// (gen_p28_expected.py proves all 16,384); over 16384 the first gives 8,190 for 8,191.
pub(crate) fn bend_point(bend: i16) -> f64 {
    f64::from(i32::from(bend).clamp(-8192, 8191) + 8192) / 16383.0
}

/// A device's mod wheel as the point the format carries it as: its low 7 bits (the byte the
/// CLAP host's MIDI message carries) over 127.
pub(crate) fn wheel_point(wheel: u8) -> f64 {
    f64::from(wheel & 0x7F) / 127.0
}

#[cfg(test)]
mod tests {
    use super::*;

    // By hand: the ends and the center of the bend, a bend past the end held there, the wheel's
    // ends and a wheel past 127 masked as its MIDI byte is (200 & 0x7F = 72).
    #[test]
    fn controls_become_the_points_the_format_carries() {
        assert_eq!(bend_point(-8192), 0.0);
        assert_eq!(bend_point(8191), 1.0);
        assert_eq!(bend_point(0), 8192.0 / 16383.0);
        assert_eq!(bend_point(30000), 1.0);
        assert_eq!(bend_point(i16::MIN), 0.0);
        assert_eq!(wheel_point(0), 0.0);
        assert_eq!(wheel_point(127), 1.0);
        assert_eq!(wheel_point(200), 72.0 / 127.0);
    }
}
