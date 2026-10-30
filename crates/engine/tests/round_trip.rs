//! Every current-format fixture and example re-saves to its own bytes: load, then the canonical
//! JSON the app's Save writes. Plugin references keep their names and opaque states verbatim,
//! native effects keep their shape, and a project whose newest pattern was never deleted writes
//! no pattern ID counter, so an untouched project never changes on disk.

use std::fs;
use std::path::Path;

use dawsome_engine::project::Project;

#[test]
fn every_current_format_fixture_saves_back_to_its_own_bytes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut checked = Vec::new();
    let entries = ["tests/fixtures", "examples"]
        .into_iter()
        .flat_map(|dir| fs::read_dir(root.join(dir)).expect("fixtures dir"));
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let text = fs::read_to_string(&path).expect("read fixture");
        // Version 1 files are migrated, and p5_cycle is refused by design.
        if !text.starts_with(r#"{"format_version":2"#) {
            continue;
        }
        let name = path
            .file_name()
            .expect("name")
            .to_string_lossy()
            .into_owned();
        if name == "p5_cycle.json" || name == "p7_catalogue.json" {
            continue;
        }
        let project = Project::load_json(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        let saved = project.to_json().expect("serializes");
        assert_eq!(saved, text.trim_end_matches('\n'), "{name}");
        checked.push(name);
    }
    for required in [
        "p7_sine.json",
        "p7_missing.json",
        "p7_fx.json",
        "p5_reverb.json",
        "demo.json",
    ] {
        assert!(
            checked.iter().any(|name| name == required),
            "{required} not checked"
        );
    }
}

/// A key a channel does not know is read and dropped: the file loads equal to the same file
/// without it, and a save writes it nowhere. Why it matters: a build that adds a channel field
/// can open files from another build, and this one must not refuse them or write the field back
/// half-understood.
#[test]
fn an_unknown_channel_key_is_read_and_dropped() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/p16_unity.json");
    let plain = fs::read_to_string(path).expect("read fixture");
    let plain = plain.trim_end_matches('\n');
    assert!(plain.starts_with(r#"{"format_version":6"#), "fixture is v6");
    let marker = r#""kind":"input","#;
    assert_eq!(plain.matches(marker).count(), 1, "one input channel");
    let with_key = plain.replace(
        marker,
        &format!(r#"{marker}"device_panel":{{"slot":3,"on":true}},"#),
    );
    assert_ne!(with_key, plain);

    let from_plain = Project::load_json(plain).expect("plain loads");
    let from_keyed = Project::load_json(&with_key).expect("file with the unknown key loads");
    assert_eq!(from_keyed, from_plain);
    let saved = from_keyed.to_json().expect("serializes");
    assert!(!saved.contains("device_panel"), "the key is written back");
    assert_eq!(saved, plain);
}
