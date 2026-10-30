//! A channel given another sound (a Library sound, set or plugin dropped on its header): one
//! edit, whose inverse gives the channel back as it was, its plugin's state too. Expected
//! documents come from tests/fixtures/gen_p15_expected.py.

use std::collections::BTreeSet;
use std::path::Path;

use dawsome_engine::edit::{self, Edit, EditError};
use dawsome_engine::project::{
    AutomationId, AutomationTarget, ChannelId, ChannelKind, ProcessorOwner, ProcessorRef, Project,
    Sound, SoundSource,
};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

fn text(name: &str) -> String {
    let text = std::fs::read_to_string(Path::new(FIXTURES).join(name)).expect("the fixture");
    text.trim_end().to_owned()
}

fn expected() -> serde_json::Value {
    serde_json::from_str(&text("p15_expected.json")).expect("parses")
}

fn json(project: &Project) -> String {
    project.to_json().expect("serializes")
}

/// What each case's drop gives, as the generator says it dropped it.
fn source(case: &serde_json::Value) -> SoundSource {
    let after = &case["channel_after"];
    match after["kind"].as_str().expect("a kind") {
        "instrument" => SoundSource::instrument(after["instrument"].as_str().expect("a set")),
        "sampler" => SoundSource::sampler(after["sample"].as_str().expect("a file")),
        "plugin" => SoundSource::plugin(ProcessorRef::plugin(
            after["processor"]["id"].as_str().expect("an id"),
            after["processor"]["name"].as_str().expect("a name"),
        )),
        kind => panic!("no drop gives a {kind} channel"),
    }
}

// Each known answer: the edit gives exactly its document; its inverse gives the base back byte
// for byte (the plugin's stored state too); the redo makes the document again. A plugin coming
// or going hands the instance its state, so the plugin is touched with the channel.
#[test]
fn p15_each_drop_replaces_the_sound_and_undo_restores_the_bytes() {
    let base = text("p15_base.json");
    let cases = expected()["cases"].clone();
    for (name, case) in cases.as_object().expect("cases") {
        let mut project = Project::load_json(&base).expect("loads");
        let id = ChannelId(u32::try_from(case["channel"].as_u64().expect("id")).expect("fits"));
        let source = source(case);
        let had_plugin = project.channels[&id].processor.is_some();
        let gets_plugin = source.processor.is_some();
        let sound = project.channels[&id].sound.kept_for(source.kind);
        let edit = Edit::ReplaceChannelSound {
            channel: id,
            name: case["channel_after"]["name"]
                .as_str()
                .expect("a name")
                .to_owned(),
            source: Box::new(source),
            sound,
        };
        let applied = edit::apply(&mut project, edit).expect("applies");
        assert_eq!(
            json(&project),
            text(case["document"].as_str().expect("file")),
            "{name}"
        );
        assert_eq!(applied.touched.channels, BTreeSet::from([id]), "{name}");
        let plugin = (had_plugin || gets_plugin).then_some(ProcessorOwner::Channel(id));
        assert_eq!(
            applied.touched.processors,
            plugin.into_iter().collect(),
            "{name}"
        );
        project.validate().expect("valid");
        let undone = edit::apply(&mut project, applied.inverse).expect("undoes");
        assert_eq!(json(&project), base, "{name}: undo restores the bytes");
        edit::apply(&mut project, undone.inverse).expect("redoes");
        assert_eq!(
            json(&project),
            text(case["document"].as_str().expect("file")),
            "{name}"
        );
    }
}

// A plugin channel given another plugin, or a set,
// loses in the same edit the automations of its old plugin's parameters (with every clip of
// them) and the controller links to those parameters, since a plugin's parameter numbers are
// its own and would move the new plugin's parameter of the same number. The channel's volume
// and pan automation and links stay, and so do another channel's. The undo puts everything
// back byte for byte and the redo takes it out again. Known answers from gen_p15_expected.py.
#[test]
fn p15_a_plugin_replaced_takes_its_parameters_automations_and_links_with_it() {
    let base = text("p15_tied_base.json");
    let want: serde_json::Value =
        serde_json::from_str(&text("p15_tied_expected.json")).expect("parses");
    for (name, case) in want["cases"].as_object().expect("cases") {
        let mut project = Project::load_json(&base).expect("loads");
        let id = ChannelId(u32::try_from(case["channel"].as_u64().expect("id")).expect("fits"));
        let source = source(case);
        let sound = project.channels[&id].sound.kept_for(source.kind);
        let edit = Edit::ReplaceChannelSound {
            channel: id,
            name: case["channel_after"]["name"]
                .as_str()
                .expect("a name")
                .to_owned(),
            source: Box::new(source),
            sound,
        };
        let applied = edit::apply(&mut project, edit).expect("applies");
        let after = text(case["document"].as_str().expect("file"));
        assert_eq!(json(&project), after, "{name}");
        let gone: BTreeSet<AutomationId> = case["automations_gone"]
            .as_array()
            .expect("a list")
            .iter()
            .map(|id| AutomationId(u32::try_from(id.as_u64().expect("id")).expect("fits")))
            .collect();
        assert_eq!(applied.touched.automation, gone, "{name}");
        assert!(applied.touched.playlist, "{name}: their clips went");
        assert_eq!(applied.touched.channels, BTreeSet::from([id]), "{name}");
        assert_eq!(
            applied.touched.processors,
            BTreeSet::from([ProcessorOwner::Channel(id)]),
            "{name}"
        );
        project.validate().expect("valid");
        let undone = edit::apply(&mut project, applied.inverse).expect("undoes");
        assert_eq!(json(&project), base, "{name}: undo restores the bytes");
        edit::apply(&mut project, undone.inverse).expect("redoes");
        assert_eq!(json(&project), after, "{name}: redone");
    }
}

// The same plugin given again (a fresh reference to the plugin channel 4 plays, its state not
// yet captured) keeps what names its parameters: they are that plugin's still. Only a sound of
// another plugin, or none, takes them out.
#[test]
fn the_same_plugin_given_again_keeps_its_parameters_automations_and_links() {
    let mut project = Project::load_json(&text("p15_tied_base.json")).expect("loads");
    let bells = ProcessorRef::plugin("clap:org.example.bells", "Bell Keys");
    let edit = Edit::ReplaceChannelSound {
        channel: ChannelId(4),
        name: "Bell Keys".to_owned(),
        source: Box::new(SoundSource::plugin(bells)),
        sound: Sound::default(),
    };
    let applied = edit::apply(&mut project, edit).expect("applies");
    let parameter = AutomationTarget::ChannelParam {
        channel: ChannelId(4),
        param_id: 7,
    };
    assert_eq!(
        project.automation.of_target(&parameter),
        Some(AutomationId(1))
    );
    assert!(project.midi_links.get(&parameter).is_some());
    assert!(applied.touched.automation.is_empty());
}

// Refused, and the project left as it was: an audio channel (its clips are measured in its
// file's frames) and an audio source; a source its kind does not hold (a plugin channel with
// no plugin); settings the new kind cannot take (reverse on a set); a channel that is not there.
#[test]
fn a_replacement_the_channel_cannot_take_is_refused() {
    let p8 = std::fs::read_to_string(Path::new(FIXTURES).join("p8_ui.json")).expect("p8_ui");
    let mut audio = Project::load_json(&p8).expect("loads");
    assert_eq!(audio.channels[&ChannelId(1)].kind, ChannelKind::Audio);
    let before = json(&audio);
    let to_set = |channel, sound| Edit::ReplaceChannelSound {
        channel,
        name: "Pad".to_owned(),
        source: Box::new(SoundSource::instrument("sets/pad/pad.sfz")),
        sound,
    };
    assert_eq!(
        edit::apply(&mut audio, to_set(ChannelId(1), Sound::default())),
        Err(EditError::AudioSound(1))
    );
    let audio_source = Edit::ReplaceChannelSound {
        channel: ChannelId(2),
        name: "x".to_owned(),
        source: Box::new(SoundSource {
            kind: ChannelKind::Audio,
            ..SoundSource::sampler("impulse.wav")
        }),
        sound: Sound::default(),
    };
    assert_eq!(
        edit::apply(&mut audio, audio_source),
        Err(EditError::AudioSound(2))
    );
    assert_eq!(json(&audio), before);

    let mut project = Project::load_json(&text("p15_base.json")).expect("loads");
    let before = json(&project);
    let no_plugin = Edit::ReplaceChannelSound {
        channel: ChannelId(1),
        name: "x".to_owned(),
        source: Box::new(SoundSource {
            kind: ChannelKind::Plugin,
            ..SoundSource::sampler("")
        }),
        sound: Sound::default(),
    };
    assert!(matches!(
        edit::apply(&mut project, no_plugin),
        Err(EditError::Invalid(_))
    ));
    let reverse = project.channels[&ChannelId(1)].sound;
    assert!(reverse.reverse);
    assert_eq!(
        edit::apply(&mut project, to_set(ChannelId(1), reverse)),
        Err(EditError::InvalidSound {
            channel: 1,
            setting: "reverse"
        })
    );
    assert_eq!(
        edit::apply(&mut project, to_set(ChannelId(9), Sound::default())),
        Err(EditError::UnknownChannel(9))
    );
    assert_eq!(json(&project), before);
}

// The settings a new kind keeps, by hand: a sampler all of them, a set all but reverse and the
// start, a plugin none.
#[test]
fn the_settings_kept_are_those_the_new_kind_takes() {
    let sound = Sound {
        semitones: 3,
        reverse: true,
        start: 0.5,
        release: 1.0,
        ..Sound::default()
    };
    assert_eq!(sound.kept_for(ChannelKind::Sampler), sound);
    assert_eq!(
        sound.kept_for(ChannelKind::Instrument),
        Sound {
            semitones: 3,
            release: 1.0,
            ..Sound::default()
        }
    );
    assert_eq!(sound.kept_for(ChannelKind::Plugin), Sound::default());
    for kind in [
        ChannelKind::Sampler,
        ChannelKind::Instrument,
        ChannelKind::Plugin,
    ] {
        assert_eq!(sound.kept_for(kind).problem(kind), None, "{kind:?}");
    }
}
