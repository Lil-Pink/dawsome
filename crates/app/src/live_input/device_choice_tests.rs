//! Which input devices the window lists and which one a stored name opens, on the lists the
//! audio hosts give.

use super::*;
use cpal::{DeviceDirection, DeviceId, HostId};

/// What cpal 0.18.2's PipeWire host (the app's host while PipeWire runs) listed among the
/// inputs on a desktop machine on 2026-11-23, in its order: the name it gives each (the node's
/// description), the node's name (its id) and the direction it says the node has. Captured with
/// a probe printing `HostInput::of` for each device `input_devices()` gave. `pw-dump` named the
/// same nodes: the `alsa_output…` ones are `Audio/Sink` (cpal lists a sink as playing and
/// recording, and records from it what it plays, its monitor), the four `alsa_input…` ones
/// `Audio/Source`, and the last two are other programs' recording streams (no description).
/// The USB sound card's sink and source share one description.
const PIPEWIRE_INPUTS: [(&str, &str, DeviceDirection); 16] = [
    ("default_sink", "sink_default", DeviceDirection::Duplex),
    ("default_input", "input_default", DeviceDirection::Input),
    (
        "Radeon High Definition Audio Controller Pro",
        "alsa_output.pci-0000_0d_00.1.pro-output-3",
        DeviceDirection::Duplex,
    ),
    (
        "Radeon High Definition Audio Controller Pro 7",
        "alsa_output.pci-0000_0d_00.1.pro-output-7",
        DeviceDirection::Duplex,
    ),
    (
        "AD103 High Definition Audio Controller Digital Stereo (HDMI) [27M2V]",
        "alsa_output.pci-0000_01_00.1.hdmi-stereo",
        DeviceDirection::Duplex,
    ),
    (
        "USB Audio S/PDIF Output",
        "alsa_output.usb-Generic_USB_Audio-00.HiFi__SPDIF__sink",
        DeviceDirection::Duplex,
    ),
    (
        "USB Audio Speakers",
        "alsa_output.usb-Generic_USB_Audio-00.HiFi__Speaker__sink",
        DeviceDirection::Duplex,
    ),
    (
        "USB Audio Front Headphones",
        "alsa_output.usb-Generic_USB_Audio-00.HiFi__Headphones__sink",
        DeviceDirection::Duplex,
    ),
    (
        "USB Audio Line Input",
        "alsa_input.usb-Generic_USB_Audio-00.HiFi__Line__source",
        DeviceDirection::Input,
    ),
    (
        "USB Audio Microphone",
        "alsa_input.usb-Generic_USB_Audio-00.HiFi__Mic__source",
        DeviceDirection::Input,
    ),
    (
        "Arctis Nova 7 Analog Stereo",
        "alsa_output.usb-SteelSeries_Arctis_Nova_7-00.analog-stereo",
        DeviceDirection::Duplex,
    ),
    (
        "Arctis Nova 7 Mono",
        "alsa_input.usb-SteelSeries_Arctis_Nova_7-00.mono-fallback",
        DeviceDirection::Input,
    ),
    (
        "USB Advanced Audio Device Analog Stereo",
        "alsa_output.usb-C-Media_Electronics_Inc._USB_Advanced_Audio_Device-00.analog-stereo",
        DeviceDirection::Duplex,
    ),
    (
        "USB Advanced Audio Device Analog Stereo",
        "alsa_input.usb-C-Media_Electronics_Inc._USB_Advanced_Audio_Device-00.analog-stereo",
        DeviceDirection::Input,
    ),
    ("unknown", "dawsome-app", DeviceDirection::Input),
    ("unknown", "WEBRTC VoiceEngine", DeviceDirection::Input),
];

/// The USB sound card's microphone, as `pactl get-default-source` named it.
const MIC_NODE: &str =
    "alsa_input.usb-C-Media_Electronics_Inc._USB_Advanced_Audio_Device-00.analog-stereo";

/// The name Settings showed for the microphone and saved as `audio_input`.
const MIC_NAME: &str = "USB Advanced Audio Device Analog Stereo";

fn pipewire(listed: &[(&str, &str, DeviceDirection)]) -> Vec<HostInput> {
    listed
        .iter()
        .map(|&(name, node, direction)| HostInput {
            name: name.to_owned(),
            id: Some(DeviceId::new(HostId::PipeWire, node)),
            direction,
        })
        .collect()
}

fn node(device: &HostInput) -> &str {
    device.id.as_ref().map_or("", DeviceId::id)
}

// The saved name of the microphone opens the microphone (the `alsa_input…` node), not the
// sink of the same name listed before it, whose monitor records what the card plays (silence
// for a card nothing plays through).
#[test]
fn a_saved_microphone_name_opens_the_microphone() {
    let host = pipewire(&PIPEWIRE_INPUTS);
    let opened = named_input(&host, MIC_NAME).map(|index| node(&host[index]));
    assert_eq!(opened, Some(MIC_NODE));
}

// The list offers the four recording nodes `pw-dump` names `Audio/Source`, in the host's order,
// each row the source itself; no sink, no stand-in for the default, no program's stream.
#[test]
fn the_list_offers_the_recording_devices_only() {
    let host = pipewire(&PIPEWIRE_INPUTS);
    let rows: Vec<(&str, &str)> = listed_inputs(&host)
        .into_iter()
        .map(|index| (host[index].name.as_str(), node(&host[index])))
        .collect();
    assert_eq!(
        rows,
        [
            (
                "USB Audio Line Input",
                "alsa_input.usb-Generic_USB_Audio-00.HiFi__Line__source"
            ),
            (
                "USB Audio Microphone",
                "alsa_input.usb-Generic_USB_Audio-00.HiFi__Mic__source"
            ),
            (
                "Arctis Nova 7 Mono",
                "alsa_input.usb-SteelSeries_Arctis_Nova_7-00.mono-fallback"
            ),
            (MIC_NAME, MIC_NODE),
        ]
    );
}

// A name saved before (by a version that listed sinks too) still opens the recording device
// of that name; a sink's name opens nothing (the device "is not there"), never its monitor.
#[test]
fn a_plain_name_opens_a_recording_device_and_never_a_monitor() {
    let host = pipewire(&PIPEWIRE_INPUTS);
    let opened = |name: &str| named_input(&host, name).map(|index| node(&host[index]));
    assert_eq!(
        opened("USB Audio Line Input"),
        Some("alsa_input.usb-Generic_USB_Audio-00.HiFi__Line__source")
    );
    assert_eq!(
        opened("Arctis Nova 7 Mono"),
        Some("alsa_input.usb-SteelSeries_Arctis_Nova_7-00.mono-fallback")
    );
    for sink in [
        "Arctis Nova 7 Analog Stereo",
        "USB Audio Speakers",
        "AD103 High Definition Audio Controller Digital Stereo (HDMI) [27M2V]",
        "default_sink",
    ] {
        assert_eq!(opened(sink), None, "{sink}");
    }
}

// The host's order decides nothing: with the source listed before the sink of its name, or
// with the sink alone, the name opens the source or nothing.
#[test]
fn the_order_the_host_lists_them_in_decides_nothing() {
    let card = &PIPEWIRE_INPUTS[12..14];
    let swapped = pipewire(&[card[1], card[0]]);
    assert_eq!(
        named_input(&swapped, MIC_NAME).map(|index| node(&swapped[index])),
        Some(MIC_NODE)
    );
    assert_eq!(listed_inputs(&swapped), [0]);
    let sink_alone = pipewire(&card[..1]);
    assert_eq!(named_input(&sink_alone, MIC_NAME), None);
    assert!(listed_inputs(&sink_alone).is_empty());
}

// Without a sound server cpal's ALSA host lists a card's PCMs, which record from the card's
// inputs whatever directions they also have: a PCM that plays and records stays a recording
// device, listed and opened by its name.
#[test]
fn an_alsa_device_that_also_plays_is_still_an_input() {
    let host = vec![HostInput {
        name: "USB Advanced Audio Device, USB Audio".to_owned(),
        id: Some(DeviceId::new(HostId::Alsa, "hw:CARD=Device,DEV=0")),
        direction: DeviceDirection::Duplex,
    }];
    assert_eq!(listed_inputs(&host), [0]);
    assert_eq!(
        named_input(&host, "USB Advanced Audio Device, USB Audio"),
        Some(0)
    );
}
