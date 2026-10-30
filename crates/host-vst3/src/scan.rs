//! The plugin scan's child half for VST3 modules: [`describe_module`] loads one module in this
//! process and describes the classes Dawsome can host; [`run_child`] prints that for the parent
//! (`dawsome_host_common::scan`). Each class is made as the host makes it (component,
//! controller, connection, handler, the component's state handed to the controller), described
//! and closed again; no window is made (JUCE plugins build their editor there, and the scan of a
//! module of 198 classes must stay inside the child's time).

use std::path::Path;

use dawsome_host_common::catalogue::{GuiSupport, PluginKind, PluginRecord};
use dawsome_host_common::scan::run_child_with;

use crate::com::handler::{Signals, EDITS};
use crate::com::mark_main_thread;
use crate::module::{ClassInfo, Module};
use crate::parts::Parts;
use crate::uid;

/// The child's whole job: describe `module` and print the result line. Returns the process exit
/// code: 0 whenever a result line was printed, also for a module that refused to load.
pub fn run_child(module: &Path) -> i32 {
    run_child_with(module, |module, progress| describe_module(module, progress))
}

/// Loads the module at `path` in this process and describes every class in it that Dawsome can
/// host: an instrument or an effect whose main buses are mono or stereo. `progress` is called
/// as each audio class is described (the scan child's progress line: a module of 198 classes
/// takes longer than the child's timeout for one). `Err` when the module will not load.
pub fn describe_module(
    path: &Path,
    mut progress: impl FnMut(),
) -> Result<Vec<PluginRecord>, String> {
    mark_main_thread();
    // SAFETY: loading a module runs its code. That is what a scan is for, and it is why the
    // scan runs this in a child process that may crash.
    let module = std::sync::Arc::new(unsafe { Module::load(path)? });
    let mut plugins = Vec::new();
    for class in module.classes() {
        let plugin = describe_class(&module, &class);
        progress();
        plugins.extend(plugin);
    }
    plugins.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(plugins)
}

/// Makes `class`, reads its shape and closes it; `None` when it cannot be made or hosted.
fn describe_class(module: &std::sync::Arc<Module>, class: &ClassInfo) -> Option<PluginRecord> {
    let signals = std::sync::Arc::new(Signals::default());
    let (edits, _) = rtrb::RingBuffer::new(EDITS);
    let parts = Parts::open(module, &class.cid, signals, edits).ok()?;
    parts.share_component_state();
    let (inputs, outputs, events) = parts.shape();
    let params = parts.param_count();
    drop(parts);
    let kind = classify(&class.sub_categories, &inputs, &outputs, events)?;
    Some(PluginRecord {
        id: uid::text(&class.cid),
        name: class.name.clone(),
        vendor: class.vendor.clone(),
        version: class.version.clone(),
        kind,
        // A VST3 plugin's editor is found as its window opens, not by the scan.
        gui: GuiSupport::None,
        params,
    })
}

/// A class's kind: from its sub-categories (`Instrument…` or `Fx…`, `|` between them), else
/// from its buses (an event input and an audio output with no audio input make an instrument,
/// audio in and out an effect). `None` for anything else, and for main buses that are neither
/// mono nor stereo.
fn classify(
    sub_categories: &str,
    inputs: &[u32],
    outputs: &[u32],
    events: bool,
) -> Option<PluginKind> {
    let has = |name: &str| sub_categories.split('|').any(|part| part == name);
    let kind = if has("Instrument") {
        PluginKind::Instrument
    } else if has("Fx") {
        PluginKind::Effect
    } else {
        match (events, inputs.is_empty(), outputs.is_empty()) {
            (true, true, false) => PluginKind::Instrument,
            (_, false, false) => PluginKind::Effect,
            _ => return None,
        }
    };
    let usable = |buses: &[u32]| matches!(buses.first(), Some(1 | 2));
    match kind {
        PluginKind::Instrument if usable(outputs) => Some(kind),
        PluginKind::Effect if usable(outputs) && usable(inputs) => Some(kind),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sub_categories_name_the_kind_and_buses_decide_without_them() {
        let stereo = [2];
        assert_eq!(
            classify("Instrument|Synth", &[], &stereo, true),
            Some(PluginKind::Instrument)
        );
        assert_eq!(
            classify("Fx|Delay", &stereo, &stereo, false),
            Some(PluginKind::Effect)
        );
        assert_eq!(
            classify("", &[], &stereo, true),
            Some(PluginKind::Instrument)
        );
        assert_eq!(
            classify("", &stereo, &stereo, false),
            Some(PluginKind::Effect)
        );
        assert_eq!(classify("", &[], &stereo, false), None, "a generator");
        assert_eq!(
            classify("Instrument", &[], &[6], true),
            None,
            "a six-channel output is not mono or stereo"
        );
        assert_eq!(
            classify("Fx", &[], &stereo, false),
            None,
            "an effect needs an input"
        );
        assert_eq!(classify("Fx", &[1], &[1], false), Some(PluginKind::Effect));
        assert_eq!(
            classify("Fx|Instrument|Sampler", &[], &stereo, true),
            Some(PluginKind::Instrument),
            "an instrument first"
        );
    }
}
