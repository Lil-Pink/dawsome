//! Controller links at play: a controller move sets every parameter linked to it at once,
//! live over the control ring, and each parameter's burst of moves becomes one undo step once
//! its controller has been still for 300 ms of audio, or when playback stops. The value jumps to
//! where the controller is (no pickup): min + (max − min)·v/127 over the parameter's range,
//! worked out in f64.
//!
//! A plugin's parameter is held by the plugin, so its moves go to the plugin alone and the
//! window captures the plugin's state when the burst ends; every other parameter is set in the
//! document as it moves.

use std::collections::BTreeMap;

use dawsome_engine::edit::Edit;
use dawsome_engine::processor::{param_values, Registry};
use dawsome_engine::project::{
    AutomationTarget, ProcessorOwner, ProcessorRef, Project, FADER_RANGE, MAX_VOLUME, PAN_RANGE,
    SEND_RANGE,
};
use dawsome_engine::transport::{Control, ParamTarget};

use crate::midi::ControllerMove;
use crate::session::{Session, SessionError};

/// How long a parameter's controller must be still before its burst becomes an undo step.
const STILL_MILLIS: u64 = 300;

/// The highest controller value.
const CONTROLLER_TOP: f64 = 127.0;

/// A linked parameter as a controller moves it.
#[derive(Debug, Clone, PartialEq)]
struct Linked {
    lo: f64,
    hi: f64,
    /// Where its live values go.
    live: ParamTarget,
    /// Its value now.
    value: f32,
    /// The plugin that holds it, for a plugin's parameter.
    plugin: Option<ProcessorOwner>,
}

impl Linked {
    /// The value controller value `value` (0 to 127) sets.
    fn at(&self, value: u8) -> f32 {
        (self.lo + (self.hi - self.lo) * f64::from(value) / CONTROLLER_TOP) as f32
    }
}

/// A parameter's moves since its controller was last still.
#[derive(Debug, Clone, PartialEq)]
struct Burst {
    /// Its value before the burst.
    start: f32,
    /// Undoes the burst: the inverse of its first move; `None` for a plugin's parameter.
    undo: Option<Edit>,
    plugin: Option<ProcessorOwner>,
    /// The frame of its last move.
    last: u64,
}

/// The bursts of controller moves under way.
#[derive(Debug, Default)]
pub struct LinkDriver {
    bursts: BTreeMap<AutomationTarget, Burst>,
}

impl LinkDriver {
    /// Whether no burst waits to become an undo step.
    pub fn is_still(&self) -> bool {
        self.bursts.is_empty()
    }

    /// A controller moved at frame `now`: every parameter linked to it goes to the value it
    /// says, live. A parameter that cannot take it is skipped; the first failure is reported
    /// once the rest have moved.
    pub fn moved(
        &mut self,
        session: &mut Session,
        moved: ControllerMove,
        now: u64,
    ) -> Result<(), SessionError> {
        let targets: Vec<AutomationTarget> = session
            .project()
            .midi_links
            .matching(moved.cc, moved.channel)
            .collect();
        let mut result = Ok(());
        for target in targets {
            let Some(linked) = resolve(session.project(), session.registry(), &target) else {
                continue;
            };
            let value = linked.at(moved.value);
            let undo = match linked.plugin {
                Some(_) => session
                    .send(Control::Param(linked.live, value))
                    .map(|()| None),
                None => match set_edit(session.project(), session.registry(), &target, value) {
                    Some(edit) => session.apply_param(edit, linked.live, value).map(Some),
                    None => continue,
                },
            };
            let undo = match undo {
                Ok(undo) => undo,
                Err(error) => {
                    if result.is_ok() {
                        result = Err(error);
                    }
                    continue;
                }
            };
            let burst = self.bursts.entry(target).or_insert(Burst {
                start: linked.value,
                undo,
                plugin: linked.plugin,
                last: now,
            });
            burst.last = now;
        }
        result
    }

    /// Ends the bursts whose controllers have been still since 300 ms of audio at `rate`
    /// before frame `now`. Returns the plugins whose state should now be captured.
    pub fn settle(
        &mut self,
        session: &mut Session,
        now: u64,
        rate: u32,
    ) -> Result<Vec<ProcessorOwner>, SessionError> {
        let still = u64::from(rate) * STILL_MILLIS / 1000;
        let due: Vec<AutomationTarget> = self
            .bursts
            .iter()
            .filter(|(_, burst)| now.saturating_sub(burst.last) >= still)
            .map(|(target, _)| target.clone())
            .collect();
        self.end(session, due)
    }

    /// Ends every burst: playback stopped, or the document is about to change hands.
    pub fn settle_all(
        &mut self,
        session: &mut Session,
    ) -> Result<Vec<ProcessorOwner>, SessionError> {
        let all = self.bursts.keys().cloned().collect();
        self.end(session, all)
    }

    /// Makes each of `targets`' bursts one undo step, when it left its parameter changed, after
    /// one plan carrying the values. A plan the player cannot take yet keeps them for the next
    /// try.
    fn end(
        &mut self,
        session: &mut Session,
        targets: Vec<AutomationTarget>,
    ) -> Result<Vec<ProcessorOwner>, SessionError> {
        if targets.is_empty() {
            return Ok(Vec::new());
        }
        session.commit_params()?;
        let mut plugins = Vec::new();
        for target in targets {
            let Some(burst) = self.bursts.remove(&target) else {
                continue;
            };
            if let Some(owner) = burst.plugin {
                plugins.push(owner);
                continue;
            }
            // A parameter whose channel, insert or effect is gone has nothing left to restore,
            // and one moved back to where it was changed nothing.
            let now = resolve(session.project(), session.registry(), &target);
            let changed = now.is_some_and(|linked| linked.value != burst.start);
            if let (true, Some(undo)) = (changed, burst.undo) {
                session.record(undo);
            }
        }
        Ok(plugins)
    }
}

/// `target` as a controller moves it; `None` when it is gone (its channel, insert, send or
/// effect removed, or a parameter its processor lacks).
fn resolve(project: &Project, registry: &Registry, target: &AutomationTarget) -> Option<Linked> {
    let native = |live, range: std::ops::RangeInclusive<f32>, value| Linked {
        lo: f64::from(*range.start()),
        hi: f64::from(*range.end()),
        live,
        value,
        plugin: None,
    };
    let mixer = &project.mixer;
    Some(match target {
        AutomationTarget::ChannelVolume(id) => native(
            ParamTarget::ChannelVolume(*id),
            0.0..=MAX_VOLUME,
            project.channels.get(id)?.volume,
        ),
        AutomationTarget::ChannelPan(id) => native(
            ParamTarget::ChannelPan(*id),
            PAN_RANGE,
            project.channels.get(id)?.pan,
        ),
        AutomationTarget::InsertFader(id) => native(
            ParamTarget::InsertFader(*id),
            FADER_RANGE,
            mixer.get(*id)?.fader,
        ),
        AutomationTarget::InsertPan(id) => {
            native(ParamTarget::InsertPan(*id), PAN_RANGE, mixer.get(*id)?.pan)
        }
        AutomationTarget::Send { from, to } => native(
            ParamTarget::Send {
                from: *from,
                to: *to,
            },
            SEND_RANGE,
            *mixer.get(*from)?.routes.get(to)?,
        ),
        AutomationTarget::EffectParam { effect, key } => {
            let (_, placed) = mixer.effect(*effect)?;
            let params = registry.get(&placed.processor.id)?.params();
            let index = params.iter().position(|info| info.key == key)?;
            let info = params[index];
            let live = ParamTarget::Effect {
                effect: *effect,
                param: u16::try_from(index).ok()?,
            };
            let value = param_values(params, &placed.processor.params)[index];
            native(live, info.min..=info.max, value)
        }
        AutomationTarget::EffectPlugin { effect, param_id } => {
            let (_, placed) = mixer.effect(*effect)?;
            let owner = ProcessorOwner::Effect(*effect);
            plugin_param(registry, owner, &placed.processor, *param_id)?
        }
        AutomationTarget::ChannelParam { channel, param_id } => {
            let processor = project.channels.get(channel)?.processor.as_ref()?;
            let owner = ProcessorOwner::Channel(*channel);
            plugin_param(registry, owner, processor, *param_id)?
        }
    })
}

/// Parameter `id` of the plugin `owner` plays, as its host describes it.
fn plugin_param(
    registry: &Registry,
    owner: ProcessorOwner,
    processor: &ProcessorRef,
    id: u32,
) -> Option<Linked> {
    let host = registry.host(&processor.id)?;
    let param = host
        .params(owner)
        .into_iter()
        .find(|param| param.id == id && !param.read_only)?;
    Some(Linked {
        lo: param.min,
        hi: param.max,
        live: ParamTarget::Plugin { owner, id },
        value: host.value(owner, id)? as f32,
        plugin: Some(owner),
    })
}

/// The edit that sets `target` to `value`, as the control's own drag would; `None` for a
/// plugin's parameter, which the plugin holds, or a parameter that is gone.
fn set_edit(
    project: &Project,
    registry: &Registry,
    target: &AutomationTarget,
    value: f32,
) -> Option<Edit> {
    Some(match target {
        AutomationTarget::ChannelVolume(channel) => Edit::SetChannelVolume {
            channel: *channel,
            value,
        },
        AutomationTarget::ChannelPan(channel) => Edit::SetChannelPan {
            channel: *channel,
            value,
        },
        AutomationTarget::InsertFader(insert) => Edit::SetInsertVolume {
            insert: *insert,
            fader: value,
        },
        AutomationTarget::InsertPan(insert) => Edit::SetInsertPan {
            insert: *insert,
            pan: value,
        },
        AutomationTarget::Send { from, to } => Edit::SetSend {
            from: *from,
            to: *to,
            amount: Some(value),
        },
        AutomationTarget::EffectParam { effect, key } => {
            let (_, placed) = project.mixer.effect(*effect)?;
            let info = registry
                .get(&placed.processor.id)?
                .params()
                .iter()
                .find(|info| info.key == key)?;
            Edit::SetEffectParam {
                effect: *effect,
                key: key.clone(),
                value: Some(info.clamp(value)),
            }
        }
        AutomationTarget::EffectPlugin { .. } | AutomationTarget::ChannelParam { .. } => {
            return None
        }
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use dawsome_engine::plan::PlayMode;
    use dawsome_engine::project::{ChannelId, InsertId, PatternId};

    use super::*;
    use crate::project_file;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    /// p9_cc: cc 7 on any channel moves insert 1's fader, cc 10 on MIDI channel 0 channel 1's
    /// pan.
    fn p9_cc() -> Session {
        let dir = PathBuf::from(FIXTURES);
        let opened = project_file::open(&dir.join("p9_cc.json")).expect("opens");
        Session::open_in(opened.project, dir, PlayMode::Pattern(PatternId(1)))
            .expect("compiles")
            .0
    }

    fn cc(channel: u8, cc: u8, value: u8) -> ControllerMove {
        ControllerMove { channel, cc, value }
    }

    // By hand: the fader 0 + (1 − 0)·100/127 = 0.787401575 → f32 0.78740156; the pan −1 + 2·127/127
    // = 1, then −1 + 2·64/127 = 1/127 = 0.007874016. Status 0xB1 is MIDI channel 1, which the pan's
    // link (channel 0) does not hear. 300 ms at 44.1 kHz is 13230 frames, so the fader's burst (its
    // move at frame 5120) ends at frame 18350 and the pan's (moves at 7168 and 8192, one burst) at
    // 21422: two undo steps.
    #[test]
    fn each_burst_of_moves_is_one_undo_step_once_its_controller_is_still() {
        let mut session = p9_cc();
        let before = session.project().clone();
        let mut links = LinkDriver::default();
        let pan = |session: &Session| session.project().channels[&ChannelId(1)].pan;
        let fader = |session: &Session| session.project().mixer.get(InsertId(1)).expect("1").fader;
        links
            .moved(&mut session, cc(0, 7, 100), 5120)
            .expect("moves");
        links
            .moved(&mut session, cc(1, 10, 0), 6144)
            .expect("moves");
        assert_eq!(pan(&session), 0.0, "another MIDI channel's cc 10");
        links
            .moved(&mut session, cc(0, 10, 127), 7168)
            .expect("moves");
        assert_eq!(pan(&session), 1.0);
        links
            .moved(&mut session, cc(0, 10, 64), 8192)
            .expect("moves");
        assert_eq!(fader(&session), 0.787_401_56);
        assert_eq!(pan(&session), 0.007_874_016);
        assert_eq!(session.history().undo_len(), 0, "live, not yet undo steps");
        links.settle(&mut session, 18_349, 44_100).expect("settles");
        assert_eq!(
            session.history().undo_len(),
            0,
            "the fader moved 13229 frames ago"
        );
        links.settle(&mut session, 18_350, 44_100).expect("settles");
        assert_eq!(session.history().undo_len(), 1);
        links.settle(&mut session, 21_422, 44_100).expect("settles");
        assert_eq!(session.history().undo_len(), 2);
        assert!(links.bursts.is_empty());
        assert!(session.undo().expect("undoes") && session.undo().expect("undoes"));
        assert_eq!(session.project(), &before);
    }

    // A controller moved back to where it started changed nothing: no undo step. Stop ends a
    // burst at once.
    #[test]
    fn a_burst_that_returns_to_its_start_records_nothing_and_stop_ends_bursts() {
        let mut session = p9_cc();
        let mut links = LinkDriver::default();
        links.moved(&mut session, cc(3, 7, 100), 0).expect("moves");
        assert!(!links.bursts.is_empty());
        links.settle_all(&mut session).expect("settles");
        assert_eq!(session.history().undo_len(), 1, "Stop ends the burst");
        links.moved(&mut session, cc(3, 7, 0), 20).expect("moves");
        links.moved(&mut session, cc(3, 7, 100), 30).expect("moves");
        links.settle_all(&mut session).expect("settles");
        assert_eq!(session.history().undo_len(), 1, "back where it started");
        assert!(links.bursts.is_empty());
    }
}
