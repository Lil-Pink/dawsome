//! MIDI device links: a channel that plays the notes of one MIDI input port, whatever channel is
//! selected. In the model a link is `Channel::midi_input`, one channel per port and one port per
//! channel, set by an edit whose inverse puts back every link it moved; on the player a note
//! handed in for a channel plays that channel, and with none, or a channel the plan lacks, the
//! selected one. Projects come from tests/fixtures/gen_p10_expected.py; the expected values are
//! written out beside each test.

use std::convert::Infallible;
use std::path::Path;

use dawsome_engine::compile::Compiler;
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::{self, Edit, EditError, History, Touched};
use dawsome_engine::plan::{PlanUpdate, PlayMode};
use dawsome_engine::project::{
    ChannelId, LoadError, LoadNote, PatternId, Project, ValidationError,
};
use dawsome_engine::realtime::{self, EngineEvent, LiveNote, Player, RealtimeLink};
use dawsome_engine::transport::Control;

#[path = "../../../tests/support/counting_allocator.rs"]
mod counting_allocator;

use counting_allocator::heap_calls_during;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

/// The port p10_link.json links to "keys" (channel 2); "kick" (channel 1) is the first channel.
const LINKED: &str = "USB Keyboard MIDI 1";
const KICK: ChannelId = ChannelId(1);
const KEYS: ChannelId = ChannelId(2);

fn fixture(name: &str) -> Project {
    let text = std::fs::read_to_string(Path::new(FIXTURES).join(name)).expect("fixture reads");
    Project::load_json(&text).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn json(project: &Project) -> String {
    project.to_json().expect("serializes")
}

fn link(channel: ChannelId, port: Option<&str>) -> Edit {
    Edit::SetChannelMidiInput {
        channel,
        port: port.map(str::to_owned),
    }
}

fn port_of(project: &Project, channel: ChannelId) -> Option<&str> {
    project.channels[&channel].midi_input.as_deref()
}

// Linking the port "keys" plays to "kick" moves it there: one port plays one channel. Like a
// controller link it touches nothing the player's plan holds, so no compile follows; its
// inverse gives the document back byte for byte.
#[test]
fn a_port_linked_to_another_channel_moves_and_one_undo_puts_it_back() {
    let mut project = fixture("p10_link.json");
    let before = json(&project);
    let applied = edit::apply(&mut project, link(KICK, Some(LINKED))).expect("links");
    assert_eq!(port_of(&project, KICK), Some(LINKED));
    assert_eq!(port_of(&project, KEYS), None, "the port left keys");
    assert_eq!(project.channel_for_port(LINKED), Some(KICK));
    assert_eq!(applied.touched, Touched::default());
    edit::apply(&mut project, applied.inverse).expect("undoes");
    assert_eq!(json(&project), before);
}

// A channel given another port lets its first one go (one port per channel); unlinked, the
// port plays nothing of its own; the inverse of each puts the link back.
#[test]
fn a_channel_holds_one_port_and_unlinking_lets_it_go() {
    let mut project = fixture("p10_link.json");
    let before = json(&project);
    let other = edit::apply(&mut project, link(KEYS, Some("Port B"))).expect("links");
    assert_eq!(port_of(&project, KEYS), Some("Port B"));
    assert_eq!(project.channel_for_port(LINKED), None);
    edit::apply(&mut project, other.inverse).expect("undoes");
    assert_eq!(json(&project), before);
    let unlinked = edit::apply(&mut project, link(KEYS, None)).expect("unlinks");
    assert_eq!(project.channel_for_port(LINKED), None);
    assert!(!json(&project).contains("midi_input"), "no link is written");
    edit::apply(&mut project, unlinked.inverse).expect("undoes");
    assert_eq!(json(&project), before);
}

// The link a channel has already: the edit is its own inverse, so it records no undo step.
#[test]
fn the_same_link_again_changes_nothing() {
    let mut project = fixture("p10_link.json");
    let edit = link(KEYS, Some(LINKED));
    let applied = edit::apply(&mut project, edit.clone()).expect("applies");
    assert!(edit::changed_nothing(&edit, &applied.inverse));
}

// A link names a channel there is, one that plays notes (an audio channel's clips play its file,
// never a device's notes), and a port with a name; anything else is refused and changes nothing.
#[test]
fn links_to_nothing_audio_channels_or_nameless_ports_are_refused() {
    let mut project = fixture("p10_link.json");
    project.channels.insert(
        ChannelId(3),
        dawsome_engine::project::Channel::audio("loop", "impulse.wav"),
    );
    project.next_channel_id = ChannelId(4);
    let before = json(&project);
    assert_eq!(
        edit::apply(&mut project, link(ChannelId(9), Some(LINKED))),
        Err(EditError::UnknownChannel(9))
    );
    assert_eq!(
        edit::apply(&mut project, link(ChannelId(3), Some(LINKED))),
        Err(EditError::PlaysNoNotes(3))
    );
    assert_eq!(
        edit::apply(&mut project, link(KICK, Some(""))),
        Err(EditError::EmptyPortName)
    );
    assert_eq!(json(&project), before);
}

// One port on two channels is not a document this build writes, and loading one is refused with
// the port's name; a link on an audio channel loads but plays nothing of its own.
#[test]
fn a_file_linking_one_port_to_two_channels_is_refused() {
    let text = std::fs::read_to_string(Path::new(FIXTURES).join("p10_link.json")).expect("reads");
    let twice = text.replacen(
        r#""sample":"impulse.wav","volume":1.0,"pan":0.0,"mute":false}"#,
        r#""sample":"impulse.wav","volume":1.0,"pan":0.0,"mute":false,"midi_input":"USB Keyboard MIDI 1"}"#,
        1,
    );
    assert_ne!(twice, text);
    assert!(matches!(
        Project::load_json(&twice),
        Err(LoadError::Invalid(ValidationError::PortLinkedTwice { port })) if port == LINKED
    ));
    let audio = text.replace(
        r#""kind":"sampler","sample":"ramp4.wav""#,
        r#""kind":"audio","sample":"ramp4.wav""#,
    );
    assert_ne!(audio, text);
    let project = Project::load_json(&audio).expect("loads");
    assert_eq!(project.channel_for_port(LINKED), None, "inert");
}

/// `project` with every file `frames` frames of 1.0, live notes on "kick", linked to a player.
fn live(project: &Project, frames: usize) -> (RealtimeLink, Player) {
    let mut compiler = Compiler::new();
    compiler.set_live_channel(Some(KICK));
    let ones = |_: &Path| Ok::<_, Infallible>(SampleBuffer::from(vec![1.0; frames]));
    let update: PlanUpdate = compiler
        .compile(
            project,
            &Touched::default(),
            PlayMode::Pattern(PatternId(1)),
            ones,
        )
        .expect("compiles");
    realtime::link(update, project.tempo)
}

fn call(player: &mut Player) -> Vec<[f32; 2]> {
    let mut out = vec![[9.0; 2]; 512];
    player.process(&mut out);
    out
}

fn on(key: u8) -> LiveNote {
    LiveNote::On { key, velocity: 0.5 }
}

// With "kick" selected and "keys" at volume 0.5: a note handed in for "keys" plays it (0.5 ×
// 0.5 = 0.25 from the call's first frame), a note with no channel plays "kick" (0.5), and a
// note for a channel the plan lacks plays "kick" too; each note-off ends the note on the channel
// it started on.
#[test]
fn a_note_for_a_channel_plays_it_whatever_is_selected() {
    let mut project = fixture("p10_link.json");
    project.channels.get_mut(&KEYS).expect("keys").volume = 0.5;
    let (mut link, mut player) = live(&project, 1);
    let inbox = link.midi_inbox();
    assert!(inbox.push_to(KEYS, on(60)));
    let out = call(&mut player);
    assert_eq!(out[0], [0.25; 2]);
    assert!(out[1..].iter().all(|frame| *frame == [0.0; 2]));
    assert!(inbox.push(on(62)));
    assert_eq!(call(&mut player)[0], [0.5; 2]);
    assert!(inbox.push_to(ChannelId(9), on(64)));
    assert_eq!(call(&mut player)[0], [0.5; 2]);
    assert!(inbox.push_to(KICK, LiveNote::Off { key: 60 }));
    assert!(inbox.push(LiveNote::Off { key: 62 }));
    call(&mut player);
    assert_eq!(
        link.take_events(),
        [
            started(KEYS, 60, 0),
            started(KICK, 62, 512),
            started(KICK, 64, 1024),
            ended(KEYS, 60, 1536),
            ended(KICK, 62, 1536),
        ]
    );
}

// The control ring's note for a channel (the instrument window's) plays
// that channel whatever is selected, "keys" at volume 0.5 from the call's first frame while
// "kick" is the live channel; one for a channel the plan lacks plays the live one; LiveNoteOff
// ends each on the channel it started on.
#[test]
fn a_control_note_for_a_channel_plays_it_whatever_is_selected() {
    let mut project = fixture("p10_link.json");
    project.channels.get_mut(&KEYS).expect("keys").volume = 0.5;
    let (mut link, mut player) = live(&project, 1);
    let note = |channel, key| Control::ChannelNoteOn {
        channel,
        key,
        velocity: 0.5,
    };
    link.send(note(KEYS, 60)).expect("room");
    let out = call(&mut player);
    assert_eq!(out[0], [0.25; 2]);
    assert!(out[1..].iter().all(|frame| *frame == [0.0; 2]));
    link.send(note(ChannelId(9), 64)).expect("room");
    assert_eq!(call(&mut player)[0], [0.5; 2]);
    link.send(Control::LiveNoteOff { key: 60 }).expect("room");
    link.send(Control::LiveNoteOff { key: 64 }).expect("room");
    call(&mut player);
    assert_eq!(
        link.take_events(),
        [
            started(KEYS, 60, 0),
            started(KICK, 64, 512),
            ended(KEYS, 60, 1024),
            ended(KICK, 64, 1024),
        ]
    );
}

// The counting-allocator check: 200 notes for a channel, for none and for a missing one make no
// heap call on the audio thread.
#[test]
fn notes_for_a_channel_never_touch_the_heap() {
    let project = fixture("p10_link.json");
    let (mut link, mut player) = live(&project, 50_000);
    let inbox = link.midi_inbox();
    let mut out = vec![[0.0; 2]; 512];
    let mut calls = 0;
    for step in 0..200u8 {
        let key = 30 + step % 40;
        let note = on(key);
        assert!(match step % 3 {
            0 => inbox.push_to(KEYS, note),
            1 => inbox.push(note),
            _ => inbox.push_to(ChannelId(9), note),
        });
        if step % 5 == 0 {
            assert!(inbox.push_to(KEYS, LiveNote::Off { key }));
        }
        calls += heap_calls_during(|| player.process(&mut out));
        link.take_events();
    }
    assert_eq!(calls, 0, "Player::process made {calls} heap calls");
}

/// p10_link.json as the file holds it.
fn p10_link_text() -> String {
    std::fs::read_to_string(Path::new(FIXTURES).join("p10_link.json")).expect("reads")
}

/// p10_link with "keys", which holds the link, made an audio channel: a link no edit makes.
fn link_on_an_audio_channel() -> String {
    let text = p10_link_text();
    let audio = text.replace(
        r#""kind":"sampler","sample":"ramp4.wav""#,
        r#""kind":"audio","sample":"ramp4.wav""#,
    );
    assert_ne!(audio, text);
    audio
}

/// p10_link with the link naming no port: a link no edit makes.
fn link_to_no_name() -> String {
    let text = p10_link_text();
    let nameless = text.replace(
        r#""midi_input":"USB Keyboard MIDI 1""#,
        r#""midi_input":"""#,
    );
    assert_ne!(nameless, text);
    nameless
}

// Every file that loads takes every undo: a link no edit could put back (on an audio channel,
// which plays no device's notes, or naming no port) is left out when the file is read. So
// moving the port, unlinking and linking again, then undoing all of it gives the loaded
// document back, redoing all of it applies too, and the result saved reads back as itself.
// Before, the inverse that put such a link back was refused and stayed on the undo stack, so
// no older step could be undone.
#[test]
fn a_link_no_edit_could_restore_is_left_out_on_load_so_every_undo_applies() {
    for (name, text) in [
        ("audio channel", link_on_an_audio_channel()),
        ("no name", link_to_no_name()),
    ] {
        let mut project = Project::load_json(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        let loaded = json(&project);
        let mut history = History::new();
        for edit in [
            link(KICK, Some(LINKED)),
            link(KEYS, None),
            link(KICK, Some("Port B")),
        ] {
            history
                .apply(&mut project, edit)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        while history.undo_len() > 0 {
            history
                .undo(&mut project)
                .unwrap_or_else(|e| panic!("{name}: every undo applies: {e}"));
        }
        assert_eq!(json(&project), loaded, "{name}");
        while history.redo_len() > 0 {
            history
                .redo(&mut project)
                .unwrap_or_else(|e| panic!("{name}: every redo applies: {e}"));
        }
        let saved = json(&project);
        let read = Project::load_json(&saved).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(json(&read), saved, "{name}: saved and read back");
    }
}

// Reading such a file says what it left out, naming the channel (and the port, when it has a
// name); the document then holds no link. A document holding one is not valid, as no edit makes
// one: validate refuses each, naming the channel.
#[test]
fn reading_says_which_link_it_left_out_and_validate_refuses_each() {
    let (project, notes) = Project::load_json_noting(&link_on_an_audio_channel()).expect("loads");
    assert_eq!(
        notes,
        [LoadNote::PortOnAudioChannel {
            channel: "keys".to_owned(),
            port: LINKED.to_owned(),
        }]
    );
    assert!(!json(&project).contains("midi_input"));
    let (project, notes) = Project::load_json_noting(&link_to_no_name()).expect("loads");
    assert_eq!(
        notes,
        [LoadNote::NamelessPort {
            channel: "keys".to_owned(),
        }]
    );
    assert!(!json(&project).contains("midi_input"));
    let (_, notes) = Project::load_json_noting(&p10_link_text()).expect("loads");
    assert!(notes.is_empty(), "a playable link stays");

    let mut project = fixture("p10_link.json");
    project.channels.get_mut(&KEYS).expect("keys").midi_input = Some(String::new());
    assert_eq!(
        project.validate(),
        Err(ValidationError::NamelessPort {
            channel: "keys".to_owned()
        })
    );
    let mut project = fixture("p10_link.json");
    project.channels.get_mut(&KEYS).expect("keys").kind =
        dawsome_engine::project::ChannelKind::Audio;
    assert_eq!(
        project.validate(),
        Err(ValidationError::PortOnAudioChannel {
            channel: "keys".to_owned(),
            port: LINKED.to_owned(),
        })
    );
}

/// A note of `key` started on `channel` at velocity 0.5 while stopped, from output frame
/// `frame`: the first frame of the call it sounded from (512 a call, from 0).
fn started(channel: ChannelId, key: u8, frame: u64) -> EngineEvent {
    EngineEvent::LiveNoteStarted {
        channel,
        key,
        velocity: 0.5,
        tick: None,
        frame,
    }
}

fn ended(channel: ChannelId, key: u8, frame: u64) -> EngineEvent {
    EngineEvent::LiveNoteEnded {
        channel,
        key,
        tick: None,
        frame,
    }
}

const OFF_60: LiveNote = LiveNote::Off { key: 60 };

// Two devices, each its port's handle on the inbox, hold the same key: the linked one on "keys",
// the other on the selected "kick". Both play, and each note-off ends its own port's note only:
// "keys" ends with the first port's release, "kick" with the second's.
#[test]
fn two_ports_holding_one_key_on_two_channels_each_end_their_own_note() {
    let (mut link, mut player) = live(&fixture("p10_link.json"), 1);
    let inbox = link.midi_inbox();
    let (linked, other) = (inbox.for_port(), inbox.for_port());
    assert!(linked.push_to(KEYS, on(60)));
    assert!(other.push(on(60)));
    call(&mut player);
    assert_eq!(
        link.take_events(),
        [started(KEYS, 60, 0), started(KICK, 60, 0)]
    );
    assert!(linked.push_to(KEYS, OFF_60));
    call(&mut player);
    assert_eq!(link.take_events(), [ended(KEYS, 60, 512)]);
    assert!(other.push(OFF_60));
    call(&mut player);
    assert_eq!(link.take_events(), [ended(KICK, 60, 1024)]);
}

// A channel sounds a key once: a second port's note of a key the first holds on that channel
// ends the first's note, and the first port's note-off then ends nothing; the second's does.
#[test]
fn on_one_channel_a_second_ports_note_of_a_held_key_ends_the_first() {
    let (mut link, mut player) = live(&fixture("p10_link.json"), 1);
    let inbox = link.midi_inbox();
    let (first, second) = (inbox.for_port(), inbox.for_port());
    assert!(first.push_to(KEYS, on(60)));
    assert!(second.push_to(KEYS, on(60)));
    assert!(first.push_to(KEYS, OFF_60));
    call(&mut player);
    assert_eq!(
        link.take_events(),
        [
            started(KEYS, 60, 0),
            ended(KEYS, 60, 0),
            started(KEYS, 60, 0)
        ]
    );
    assert!(second.push_to(KEYS, OFF_60));
    call(&mut player);
    assert_eq!(link.take_events(), [ended(KEYS, 60, 512)]);
}

// A port that goes lets go of every key it holds, after the notes it handed in before (so a
// note-on still waiting ends too), and of nothing another port holds.
#[test]
fn a_released_port_lets_go_of_its_own_keys_only() {
    let (mut link, mut player) = live(&fixture("p10_link.json"), 1);
    let inbox = link.midi_inbox();
    let (gone, staying) = (inbox.for_port(), inbox.for_port());
    assert!(gone.push_to(KEYS, on(60)));
    assert!(staying.push(on(64)));
    call(&mut player);
    assert!(gone.push_to(KEYS, on(62)));
    assert!(gone.release());
    call(&mut player);
    assert_eq!(
        link.take_events(),
        [
            started(KEYS, 60, 0),
            started(KICK, 64, 0),
            started(KEYS, 62, 512),
            ended(KEYS, 60, 512),
            ended(KEYS, 62, 512),
        ]
    );
    assert!(gone.push_to(KEYS, OFF_60));
    assert!(staying.push(LiveNote::Off { key: 64 }));
    call(&mut player);
    assert_eq!(link.take_events(), [ended(KICK, 64, 1024)]);
}

// 256 keys can be held at once (two whole keyboards, here on "kick" and "keys"); a 257th is
// dropped and counted, its note-off ends nothing, and every held key still ends.
#[test]
fn a_key_struck_with_every_hold_taken_is_dropped_and_counted() {
    let mut project = fixture("p10_link.json");
    edit::apply(
        &mut project,
        Edit::AddSamplerChannel {
            id: ChannelId(3),
            name: "third".into(),
            sample: "impulse.wav".into(),
        },
    )
    .expect("adds");
    let (mut link, mut player) = live(&project, 1);
    let inbox = link.midi_inbox();
    let ports = [inbox.for_port(), inbox.for_port(), inbox.for_port()];
    for key in 0..=127 {
        assert!(ports[0].push(on(key)));
        assert!(ports[1].push_to(KEYS, on(key)));
    }
    call(&mut player);
    assert!(ports[2].push_to(ChannelId(3), on(0)));
    call(&mut player);
    assert_eq!(link.take_events().len(), 256, "the 257th never started");
    assert_eq!(link.dropped_notes(), 1);
    assert!(ports[2].push_to(ChannelId(3), LiveNote::Off { key: 0 }));
    for port in &ports[..2] {
        assert!(port.release());
    }
    call(&mut player);
    let events = link.take_events();
    assert_eq!(events.len(), 256);
    assert!(events
        .iter()
        .all(|event| matches!(event, EngineEvent::LiveNoteEnded { channel, .. } if *channel != ChannelId(3))));
}

// The counting-allocator check: notes from three ports' handles, one port's release and a full
// table make no heap call on the audio thread.
#[test]
fn ports_and_releases_never_touch_the_heap() {
    let project = fixture("p10_link.json");
    let (mut link, mut player) = live(&project, 50_000);
    let inbox = link.midi_inbox();
    let ports = [inbox.for_port(), inbox.for_port(), inbox.for_port()];
    let mut out = vec![[0.0; 2]; 512];
    let mut calls = 0;
    for step in 0..200u8 {
        let port = &ports[usize::from(step % 3)];
        let key = step % 128;
        assert!(match step % 2 {
            0 => port.push_to(KEYS, on(key)),
            _ => port.push(on(key)),
        });
        if step % 7 == 0 {
            assert!(port.release());
        }
        calls += heap_calls_during(|| player.process(&mut out));
        link.take_events();
    }
    assert_eq!(calls, 0, "Player::process made {calls} heap calls");
}

/// How midir names the linked device's port on Linux, with the ALSA sequencer's client and port
/// numbers at the end, as two sessions may number it.
const SAVED_ALSA: &str = "USB Keyboard:USB Keyboard MIDI 1 32:0";
const RENUMBERED_ALSA: &str = "USB Keyboard:USB Keyboard MIDI 1 36:0";
/// The part of both that names the device.
const STABLE_ALSA: &str = "USB Keyboard:USB Keyboard MIDI 1";

/// p10_link with its link saved under the whole ALSA name.
fn link_saved_with_alsa_numbers() -> Project {
    let text = p10_link_text().replace(LINKED, SAVED_ALSA);
    Project::load_json(&text).expect("loads")
}

// A link saved under a Linux port name that ends in the system's client and port numbers still
// finds its device when the numbers change (another device was found first), and of course
// under the same numbers; a link stored by the stable part finds the port whatever its numbers.
// Another device's port is not the linked one.
#[test]
fn a_link_finds_its_port_whatever_the_systems_numbers() {
    let project = link_saved_with_alsa_numbers();
    assert_eq!(
        project.channel_for_port(SAVED_ALSA),
        Some(KEYS),
        "exact name"
    );
    assert_eq!(project.channel_for_port(RENUMBERED_ALSA), Some(KEYS));
    assert_eq!(project.channel_for_port(STABLE_ALSA), Some(KEYS));
    assert_eq!(
        project.channel_for_port("USB Keyboard:USB Keyboard MIDI 2 32:1"),
        None
    );
    let mut stable = fixture("p10_link.json");
    edit::apply(&mut stable, link(KEYS, Some(STABLE_ALSA))).expect("links");
    assert_eq!(stable.channel_for_port(RENUMBERED_ALSA), Some(KEYS));
}

// Linking a port by its stable part to "kick" moves the link "keys" holds under the port's
// whole former name: they are one port. The inverse gives the document back byte for byte.
#[test]
fn a_link_moves_from_the_channel_holding_the_port_under_its_former_numbers() {
    let mut project = link_saved_with_alsa_numbers();
    let before = json(&project);
    let applied = edit::apply(&mut project, link(KICK, Some(STABLE_ALSA))).expect("links");
    assert_eq!(port_of(&project, KICK), Some(STABLE_ALSA));
    assert_eq!(port_of(&project, KEYS), None, "the port left keys");
    edit::apply(&mut project, applied.inverse).expect("undoes");
    assert_eq!(json(&project), before);
}

// Two links whose names differ only in the system's numbers name one port, which plays one
// channel: validate refuses them as it refuses one name twice.
#[test]
fn two_links_to_one_port_under_two_numberings_are_not_valid() {
    let mut project = link_saved_with_alsa_numbers();
    project.channels.get_mut(&KICK).expect("kick").midi_input = Some(RENUMBERED_ALSA.to_owned());
    assert!(matches!(
        project.validate(),
        Err(ValidationError::PortLinkedTwice { .. })
    ));
}

// By hand from midir 0.11.1's ALSA backend, which names a port "<client name>:<port name>
// <client number>:<port number>": the stable part drops the trailing numbers. Windows (WinMM)
// names, a device's product name, end in no such pair and stay whole, as does anything that only
// looks close.
#[test]
fn a_ports_stable_name_drops_only_the_systems_trailing_numbers() {
    use dawsome_engine::project::{same_port, stable_port_name};
    for (name, stable) in [
        (SAVED_ALSA, STABLE_ALSA),
        (
            "Midi Through:Midi Through Port-0 14:0",
            "Midi Through:Midi Through Port-0",
        ),
        ("Pads:Pads MIDI 1 128:15", "Pads:Pads MIDI 1"),
    ] {
        assert_eq!(stable_port_name(name), stable, "{name}");
    }
    for whole in [
        LINKED,
        "2- USB MIDI Interface",
        "MIDIIN2 (Mini Keys)",
        "Virtual Port",
        "Digital Piano",
        "32:0",
        " 32:0",
        "Pads 32:",
        "Pads :0",
        "Pads 3a:0",
        "Pads 32:0:1",
        "Pads 32-0",
        "",
    ] {
        assert_eq!(stable_port_name(whole), whole, "{whole:?} stays whole");
    }
    assert!(same_port(SAVED_ALSA, RENUMBERED_ALSA));
    assert!(same_port(SAVED_ALSA, STABLE_ALSA));
    assert!(!same_port(
        SAVED_ALSA,
        "USB Keyboard:USB Keyboard MIDI 2 32:1"
    ));
}

// A file an earlier build wrote, linking two devices whose names differ only in the system's
// numbers to two channels, opens with the first channel in the rack keeping the device and the
// second's link left out, which reading says; one name linked twice stays refused.
#[test]
fn a_second_link_to_one_device_under_other_numbers_is_left_out_on_reading() {
    let text = p10_link_text().replace(LINKED, SAVED_ALSA).replacen(
        r#""sample":"impulse.wav","volume":1.0,"pan":0.0,"mute":false}"#,
        &format!(
            r#""sample":"impulse.wav","volume":1.0,"pan":0.0,"mute":false,"midi_input":"{RENUMBERED_ALSA}"}}"#
        ),
        1,
    );
    let (project, notes) = Project::load_json_noting(&text).expect("loads");
    assert_eq!(port_of(&project, KICK), Some(RENUMBERED_ALSA));
    assert_eq!(port_of(&project, KEYS), None);
    assert_eq!(
        notes,
        [LoadNote::SamePortTwice {
            channel: "keys".to_owned(),
            port: SAVED_ALSA.to_owned(),
            kept: "kick".to_owned(),
        }]
    );
    let twice = text.replace(RENUMBERED_ALSA, SAVED_ALSA);
    assert!(matches!(
        Project::load_json(&twice),
        Err(LoadError::Invalid(ValidationError::PortLinkedTwice { .. }))
    ));
}
