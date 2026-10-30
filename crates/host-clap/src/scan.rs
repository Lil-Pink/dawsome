//! The plugin scan's child half for CLAP files: [`describe_file`] loads one `.clap` file in
//! this process and describes the plugins Dawsome can host; [`run_child`] prints that for the
//! parent. The parent half (finding files, running children, the catalogue) is
//! `dawsome_host_common::scan`, re-exported here.

use std::ffi::{CStr, CString};
use std::path::Path;

use clack_extensions::audio_ports::{AudioPortInfoBuffer, PluginAudioPorts};
use clack_extensions::gui::{GuiConfiguration, PluginGui};
use clack_extensions::note_ports::PluginNotePorts;
use clack_extensions::params::PluginParams;
use clack_host::prelude::{HostInfo, PluginEntry, PluginInstance};

pub use dawsome_host_common::scan::*;

use crate::catalogue::{GuiSupport, PluginKind, PluginRecord};

/// The child's whole job: describe `file` and print the result line. Returns the process exit
/// code: 0 whenever a result line was printed, also for a file that refused to load.
pub fn run_child(file: &Path) -> i32 {
    run_child_with(file, |file, progress| describe_file(file, progress))
}

/// Loads `file` in this process and describes every plugin in it that Dawsome can host: an
/// instrument or an effect whose main audio ports are mono or stereo. Each plugin is created,
/// queried and destroyed, and `progress` called once it is (the scan child's progress line).
/// `Err` when the file will not load.
pub fn describe_file(file: &Path, mut progress: impl FnMut()) -> Result<Vec<PluginRecord>, String> {
    // SAFETY: loading a plugin file runs its code. That is what a scan is for, and it is why
    // the scan runs this in a child process that may crash.
    let entry = unsafe { PluginEntry::load(file) }.map_err(|error| error.to_string())?;
    let factory = entry
        .get_plugin_factory()
        .ok_or_else(|| "it offers no plugins".to_owned())?;
    let host = HostInfo::new("Dawsome", "Dawsome", "", env!("CARGO_PKG_VERSION"))
        .map_err(|error| error.to_string())?;
    let mut plugins = Vec::new();
    for descriptor in factory.plugin_descriptors() {
        let Some(id) = descriptor.id() else {
            continue;
        };
        let text = |value: Option<&CStr>| {
            value.map_or_else(String::new, |value| value.to_string_lossy().into_owned())
        };
        let features: Vec<String> = descriptor
            .features()
            .map(|feature| feature.to_string_lossy().into_owned())
            .collect();
        let shape = describe_plugin(&entry, id, &host);
        progress();
        let Some(shape) = shape else {
            continue;
        };
        let Some(kind) = classify(&features, &shape) else {
            continue;
        };
        plugins.push(PluginRecord {
            id: text(Some(id)),
            name: text(descriptor.name()),
            vendor: text(descriptor.vendor()),
            version: text(descriptor.version()),
            kind,
            gui: shape.gui,
            params: shape.params,
        });
    }
    plugins.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(plugins)
}

/// What a live instance says about itself.
struct Shape {
    /// Channels of the main audio input and output ports, when there are such ports.
    audio_in: Option<u32>,
    audio_out: Option<u32>,
    note_in: bool,
    gui: GuiSupport,
    params: u32,
}

/// Creates plugin `id` with a host that offers nothing, reads its shape and destroys it;
/// `None` when it cannot be created.
fn describe_plugin(entry: &PluginEntry, id: &CStr, host: &HostInfo) -> Option<Shape> {
    let id = CString::from(id);
    let mut instance = PluginInstance::<()>::new(|_| (), |_| (), entry, &id, host).ok()?;
    let shared = instance.plugin_shared_handle();
    let (audio, notes, gui, params) = (
        shared.get_extension::<PluginAudioPorts>(),
        shared.get_extension::<PluginNotePorts>(),
        shared.get_extension::<PluginGui>(),
        shared.get_extension::<PluginParams>(),
    );
    let handle = instance.plugin_handle();
    let main_port = |is_input| {
        let audio = audio.as_ref()?;
        if audio.count(&handle, is_input) == 0 {
            return None;
        }
        let mut buffer = AudioPortInfoBuffer::new();
        audio
            .get(&handle, 0, is_input, &mut buffer)
            .map(|port| port.channel_count)
    };
    let supports = |floating| {
        gui.as_ref().is_some_and(|gui| {
            gui.is_api_supported(
                &handle,
                GuiConfiguration {
                    api_type: crate::window::API,
                    is_floating: floating,
                },
            )
        })
    };
    let gui = if supports(false) {
        GuiSupport::Embed
    } else if supports(true) {
        GuiSupport::Floating
    } else {
        GuiSupport::None
    };
    Some(Shape {
        audio_in: main_port(true),
        audio_out: main_port(false),
        note_in: notes.is_some_and(|notes| notes.count(&handle, true) > 0),
        gui,
        params: params.map_or(0, |params| params.count(&handle)),
    })
}

/// A plugin's kind: from its `instrument` or `audio-effect` feature, else from its ports (notes
/// in and audio out with no audio in make an instrument, audio in and out an effect). `None`
/// for anything else, and for main audio ports that are neither mono nor stereo.
fn classify(features: &[String], shape: &Shape) -> Option<PluginKind> {
    let has = |name: &str| features.iter().any(|feature| feature == name);
    let kind = if has("instrument") {
        PluginKind::Instrument
    } else if has("audio-effect") {
        PluginKind::Effect
    } else {
        match (shape.note_in, shape.audio_in, shape.audio_out) {
            (true, None, Some(_)) => PluginKind::Instrument,
            (_, Some(_), Some(_)) => PluginKind::Effect,
            _ => return None,
        }
    };
    let usable = |channels: Option<u32>| channels.is_some_and(|count| (1..=2).contains(&count));
    match kind {
        PluginKind::Instrument if usable(shape.audio_out) => Some(kind),
        PluginKind::Effect if usable(shape.audio_out) && usable(shape.audio_in) => Some(kind),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(note_in: bool, audio_in: Option<u32>, audio_out: Option<u32>) -> Shape {
        Shape {
            audio_in,
            audio_out,
            note_in,
            gui: GuiSupport::None,
            params: 0,
        }
    }

    #[test]
    fn features_name_the_kind_and_ports_decide_without_them() {
        let features = |names: &[&str]| -> Vec<String> {
            names.iter().map(|name| (*name).to_owned()).collect()
        };
        let synth = shape(true, None, Some(2));
        let effect = shape(false, Some(2), Some(2));
        assert_eq!(
            classify(&features(&["instrument"]), &effect),
            Some(PluginKind::Instrument)
        );
        assert_eq!(
            classify(&features(&["audio-effect", "stereo"]), &effect),
            Some(PluginKind::Effect)
        );
        assert_eq!(classify(&[], &synth), Some(PluginKind::Instrument));
        assert_eq!(classify(&[], &effect), Some(PluginKind::Effect));
        assert_eq!(classify(&[], &shape(false, None, Some(2))), None);
        assert_eq!(
            classify(&features(&["instrument"]), &shape(true, None, Some(6))),
            None,
            "a six-channel output is not mono or stereo"
        );
        assert_eq!(
            classify(&features(&["audio-effect"]), &shape(false, None, Some(2))),
            None,
            "an effect needs an input"
        );
        assert_eq!(
            classify(
                &features(&["audio-effect"]),
                &shape(false, Some(1), Some(1))
            ),
            Some(PluginKind::Effect)
        );
    }
}
