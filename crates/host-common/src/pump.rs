//! What a render needs of every plugin host while it plays: the plugins a project names, by
//! format, and the hosts' main-thread work, each failure reported once.

use std::collections::{BTreeMap, BTreeSet};

use dawsome_engine::compile::plugin_notice;
use dawsome_engine::processor::{HostError, HostNotice, NoticeKind, ProcessorHost};
use dawsome_engine::project::{ProcessorOwner, ProcessorRef, Project};

/// The plugin ids of `format` (`"clap"`, `"vst3"`) that `project` names, on plugin channels and
/// in effect slots, without the format's prefix.
pub fn plugin_ids(project: &Project, format: &str) -> BTreeSet<String> {
    let prefix = format!("{format}:");
    processors(project)
        .filter_map(|(_, reference)| reference.id.strip_prefix(prefix.as_str()))
        .map(str::to_owned)
        .collect()
}

/// The processors `project` names, on plugin channels and in effect slots, by `ProcessorOwner`.
fn processors(project: &Project) -> impl Iterator<Item = (ProcessorOwner, &ProcessorRef)> {
    let channels = project.channels.iter().filter_map(|(&id, channel)| {
        Some((ProcessorOwner::Channel(id), channel.processor.as_ref()?))
    });
    let effects = project
        .mixer
        .inserts
        .iter()
        .flat_map(|insert| &insert.effects)
        .map(|effect| (ProcessorOwner::Effect(effect.id), &effect.processor));
    channels.chain(effects)
}

/// The plugins' main-thread work while an offline render plays (`dawsome render`, `--play-to`,
/// an export): each host's tick, and each way a plugin fails reported once, by name, however
/// many blocks it fails in.
pub struct RenderPump {
    hosts: Vec<Box<dyn ProcessorHost>>,
    references: BTreeMap<ProcessorOwner, ProcessorRef>,
    reported: BTreeSet<(ProcessorOwner, String)>,
}

impl RenderPump {
    /// A pump for `host`, naming the plugins as `project` does.
    pub fn new<H: ProcessorHost + Clone + 'static>(host: &H, project: &Project) -> Self {
        Self::of(vec![Box::new(host.clone())], project)
    }

    /// A pump for every host in `hosts` (one per plugin format the project names), naming the
    /// plugins as `project` does.
    pub fn of(hosts: Vec<Box<dyn ProcessorHost>>, project: &Project) -> Self {
        Self {
            hosts,
            references: processors(project)
                .map(|(owner, reference)| (owner, reference.clone()))
                .collect(),
            reported: BTreeSet::new(),
        }
    }

    /// What each plugin that delays its sound says of its delay, as "<name> (<id>) delays its
    /// sound by N frames; plugin latency is not compensated", in the project's order. Why said:
    /// a render is then late for a reason the user can read.
    pub fn latencies(&self) -> Vec<String> {
        self.references
            .iter()
            .filter_map(|(&owner, reference)| {
                let host = self.hosts.iter().find(|host| {
                    reference
                        .id
                        .strip_prefix(host.format())
                        .is_some_and(|rest| rest.starts_with(':'))
                })?;
                let frames = host.latency(owner).filter(|&frames| frames > 0)?;
                Some(format!(
                    "{} ({}) delays its sound by {frames} frames; plugin latency is not compensated",
                    reference.display_name(),
                    reference.id
                ))
            })
            .collect()
    }

    /// Runs the hosts' main-thread work; returns the failures not reported before, as
    /// "Plugin failed: <name>: <reason>".
    pub fn pump(&mut self) -> Vec<String> {
        let mut new = Vec::new();
        for host in &self.hosts {
            for HostNotice { owner, kind } in host.tick() {
                let NoticeKind::Failed(reason) = kind else {
                    continue;
                };
                let Some(reference) = self.references.get(&owner) else {
                    continue;
                };
                if self.reported.insert((owner, reason.clone())) {
                    new.push(plugin_notice(reference, &HostError::Failed(reason)));
                }
            }
        }
        new
    }
}
