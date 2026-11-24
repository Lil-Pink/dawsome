//! Jam through the engine: every case's pattern from `jam::write`, the harmony table, the edit
//! that writes a Jam into a project and its undo, and the project file that keeps a key.
//! Expected values come from tests/fixtures/gen_p36_expected.py (p36_expected.json and its
//! projects), never from the engine.

use std::fs;
use std::path::{Path, PathBuf};

use dawsome_engine::edit::{self, Edit, EditError};
use dawsome_engine::jam::{self, DrumMap, Genre, JamChannel, JamRequest, Pitch, Role, Voice};
use dawsome_engine::project::{
    ChannelId, KeySignature, LoadError, PatternId, PitchClass, Project, Scale,
};
use dawsome_engine::time::TimeSignature;
use serde_json::{json, Value};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn text(name: &str) -> String {
    fs::read_to_string(fixtures().join(name)).expect("a p36 fixture")
}

fn expected() -> Value {
    serde_json::from_str(&text("p36_expected.json"))
        .expect("p36_expected.json (run gen_p36_expected.py --write tests/fixtures)")
}

fn load(name: &str) -> Project {
    Project::load_json(&text(name)).expect("loads")
}

/// A role as the request files spell it (the command line's words).
fn role(text: &str) -> (ChannelId, Role) {
    let (channel, what) = text.split_once('=').expect("N=role");
    let channel = ChannelId(channel.parse().expect("a channel id"));
    let role = match what {
        "drums" => Role::Drums(DrumMap::GM),
        "bass" => Role::Bass,
        "chords" => Role::Chords,
        "melody" => Role::Melody,
        map if map.starts_with("drums:") => {
            let mut drums = DrumMap::default();
            for pair in map["drums:".len()..].split(',') {
                let (voice, key) = pair.split_once('=').expect("voice=key");
                let voice = Voice::from_id(voice).expect("a voice");
                drums = drums.with(voice, Some(key.parse().expect("a key")));
            }
            Role::Drums(drums)
        }
        voice => Role::Drums(DrumMap::single(Voice::from_id(voice).expect("a voice"))),
    };
    (channel, role)
}

/// The request a case's file describes, against its project.
fn request(case: &Value) -> (JamRequest, Project, Value) {
    let req: Value =
        serde_json::from_str(&text(case["request"].as_str().expect("a file"))).expect("parses");
    let project = load(req["project"].as_str().expect("a project"));
    let key = match req.get("key") {
        Some(root) => KeySignature {
            root: root.as_str().expect("a root").parse().expect("a key"),
            scale: req["scale"]
                .as_str()
                .expect("a scale")
                .parse()
                .expect("a scale"),
        },
        None => project.key.expect("the project's own key"),
    };
    let mut channels: Vec<_> = req["roles"]
        .as_array()
        .expect("roles")
        .iter()
        .map(|r| role(r.as_str().expect("a role")))
        .collect();
    channels.sort_by_key(|(id, _)| *id);
    // A request's ranges as the command line spells them: "N=LO-HI".
    let ranges: Vec<(u32, (u8, u8))> = req
        .get("ranges")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|text| {
            let (channel, keys) = text.as_str().expect("a range").split_once('=').expect("N=");
            let (low, high) = keys.split_once('-').expect("LO-HI");
            let key = |text: &str| text.parse().expect("a key");
            (channel.parse().expect("a channel"), (key(low), key(high)))
        })
        .collect();
    // The channels the request holds sustained, by id.
    let sustained: Vec<u64> = req
        .get("sustained")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|id| id.as_u64().expect("a channel"))
        .collect();
    let channels: Vec<JamChannel> = channels
        .into_iter()
        .map(|(channel, role)| JamChannel {
            channel,
            role,
            range: ranges
                .iter()
                .find(|(id, _)| *id == channel.0)
                .map(|&(_, range)| range),
            articulation: if sustained.contains(&u64::from(channel.0)) {
                jam::Articulation::Sustained
            } else {
                jam::Articulation::Plucked
            },
        })
        .collect();
    let request = JamRequest {
        genre: req["genre"]
            .as_str()
            .expect("a genre")
            .parse()
            .expect("a genre"),
        key,
        bars: req["bars"].as_u64().expect("bars") as u32,
        seed: req["seed"].as_u64().expect("a seed"),
        ppq: project.ppq,
        channels,
    };
    (request, project, req)
}

/// A written Jam as the generator states it (the command line's `--print`, less the request's
/// echo).
fn as_printed(jam: &jam::JamPattern) -> Value {
    let channels: Vec<Value> = jam
        .parts
        .iter()
        .map(|part| {
            let notes: Vec<Value> = part
                .notes
                .iter()
                .map(|n| {
                    json!({"tick": n.tick.0, "length": n.length.0, "key": n.key,
                           "velocity": f64::from(n.velocity) / 100.0})
                })
                .collect();
            json!({"channel": part.channel.0, "role": part.role.id(), "notes": notes,
                   "dropped": part.dropped})
        })
        .collect();
    // Each part's variant counted from 1, as the file lists them.
    let variants: serde_json::Map<String, Value> = jam::PARTS
        .iter()
        .zip(jam.variants)
        .map(|(part, picked)| ((*part).to_owned(), json!(picked + 1)))
        .collect();
    json!({"progression": jam.progression, "tempo_mbpm": jam.tempo.0,
           "swing_tenths": jam.swing_tenths, "variants": variants, "channels": channels})
}

// Known answer 1: every case's progression, tempo, swing and each channel's notes (tick,
// length, key, velocity), draw for draw as the oracle writes them.
#[test]
fn every_case_writes_the_oracles_pattern() {
    let want = expected();
    let cases = want["cases"].as_object().expect("cases");
    assert_eq!(cases.len(), 20);
    for (name, case) in cases {
        let (request, _, _) = request(case);
        let got = as_printed(&jam::write(&request));
        let printed = &case["print"];
        for field in [
            "progression",
            "tempo_mbpm",
            "swing_tenths",
            "variants",
            "channels",
        ] {
            assert_eq!(got[field], printed[field], "{name}: {field}");
        }
    }
}

// Known answer 2: the harmony table, for the roots A and C in every scale: the chords on each
// degree (triads and sevenths), the bass's five pitches under each, and the melody's pool.
#[test]
fn harmony_matches_the_oracles_table() {
    let want = expected();
    for (name, row) in want["harmony"].as_object().expect("a table") {
        let (root, scale) = name.split_once(' ').expect("root scale");
        let key = KeySignature {
            root: root.parse().expect("a root"),
            scale: scale.parse().expect("a scale"),
        };
        for degree in 1..=7u8 {
            let at = usize::from(degree - 1);
            assert_eq!(
                json!(jam::chord(key, degree, false)),
                row["triads"][at],
                "{name} {degree}"
            );
            assert_eq!(
                json!(jam::chord(key, degree, true)),
                row["sevenths"][at],
                "{name} {degree}"
            );
            let bass = &row["bass"][at];
            for (word, pitch) in [
                ("root", Pitch::Root),
                ("fifth", Pitch::Fifth),
                ("octave_up", Pitch::OctaveUp),
                ("octave_down", Pitch::OctaveDown),
                ("next_degree", Pitch::NextDegree),
            ] {
                assert_eq!(
                    json!(jam::bass_note(key, degree, pitch)),
                    bass[word],
                    "{name} {degree} {word}"
                );
            }
        }
        assert_eq!(json!(jam::melody_pool(key)), row["melody_pool"], "{name}");
    }
}

// Known answer 3: the PRNG's first ten values from seeds 1 and 0.
#[test]
fn the_rng_matches_the_oracle() {
    let want = expected();
    for (genre, field) in [
        (Genre::LoFi, "lofi_seed_1"),
        (Genre::BoomBap, "boombap_seed_1"),
    ] {
        let mut rng = jam::JamRng::for_jam(genre, 1);
        let got: Vec<u32> = (0..10).map(|_| rng.next_u32()).collect();
        assert_eq!(json!(got), want["rng"][field]);
    }
    let mixed: Vec<u64> = [0, 1, 0x9E37_79B9_7F4A_7C15]
        .into_iter()
        .map(jam::splitmix64)
        .collect();
    assert_eq!(json!(mixed), want["rng"]["splitmix64"]);
}

/// The saved project's parts the generator states: version, key, tempo and the Jam pattern.
fn saved_parts(saved: &str) -> Value {
    let file: Value = serde_json::from_str(saved).expect("parses");
    let pattern = file["patterns"]
        .as_array()
        .expect("patterns")
        .iter()
        .find(|p| p["name"] == "Jam 1")
        .cloned()
        .expect("the Jam pattern");
    json!({"format_version": file["format_version"], "key": file["key"],
           "tempo_mbpm": file["tempo_mbpm"], "pattern": pattern})
}

// Known answer 4: a Jam written into p36_band.json with auto tempo (lofi_1) gives the
// generator's saved project; undo gives the band's bytes back, redo the Jam's. Then a second
// roll (lofi_2's pattern) replacing that same pattern, without auto tempo: its notes, the
// project's key C major, the tempo kept, the pattern still "Jam 1"; undo gives the first roll's
// bytes back.
#[test]
fn a_jam_is_one_step_and_undoes_byte_for_byte() {
    let want = expected();
    let case = &want["cases"]["lofi_1"];
    let (request, mut project, _) = request(case);
    let before = project.to_json().expect("writes");
    assert_eq!(jam::pattern_name(&project), "Jam 1");
    let pattern = jam::write(&request);
    let pattern_name = jam::pattern_name(&project);
    let applied = edit::apply(
        &mut project,
        Edit::Jam {
            pattern_name,
            replace: None,
            pattern,
            set_tempo: true,
            key: request.key,
        },
    )
    .expect("a Jam applies");
    assert!(applied.touched.tempo);
    assert_eq!(applied.touched.patterns.len(), 1);
    let after = project.to_json().expect("writes");
    assert_eq!(saved_parts(&after), case["saved"]);
    assert_eq!(jam::pattern_name(&project), "Jam 2");

    let redo = edit::apply(&mut project, applied.inverse).expect("undo applies");
    assert_eq!(
        project.to_json().expect("writes"),
        before,
        "undo restores the bytes"
    );
    edit::apply(&mut project, redo.inverse).expect("redo applies");
    assert_eq!(
        project.to_json().expect("writes"),
        after,
        "redo gives the Jam again"
    );

    let second = &want["cases"]["lofi_2"];
    let (request, _, _) = self::request(second);
    let jam_id = PatternId(2);
    let applied = edit::apply(
        &mut project,
        Edit::Jam {
            pattern_name: "unused when replacing".into(),
            replace: Some(jam_id),
            pattern: jam::write(&request),
            set_tempo: false,
            key: request.key,
        },
    )
    .expect("a roll replaces the Jam pattern");
    let rolled = saved_parts(&project.to_json().expect("writes"));
    let mut want_rolled = second["saved"].clone();
    // Without auto tempo the second roll keeps the tempo the first one drew.
    want_rolled["tempo_mbpm"] = case["saved"]["tempo_mbpm"].clone();
    assert_eq!(rolled, want_rolled);
    assert_eq!(project.patterns.len(), 2);
    edit::apply(&mut project, applied.inverse).expect("undo applies");
    assert_eq!(project.to_json().expect("writes"), after);
}

/// A lo-fi Jam for the band's four channels in A minor, written at `ppq`.
fn band_jam(ppq: u16) -> (jam::JamPattern, KeySignature) {
    let key = KeySignature {
        root: PitchClass::new(9).expect("A"),
        scale: Scale::Minor,
    };
    let request = JamRequest {
        genre: Genre::LoFi,
        key,
        bars: 4,
        seed: 1,
        ppq,
        channels: vec![
            (ChannelId(1), Role::Drums(DrumMap::GM)).into(),
            (ChannelId(2), Role::Bass).into(),
            (ChannelId(3), Role::Chords).into(),
            (ChannelId(4), Role::Melody).into(),
        ],
    };
    (jam::write(&request), key)
}

fn jam_edit(pattern: jam::JamPattern, replace: Option<PatternId>, key: KeySignature) -> Edit {
    Edit::Jam {
        pattern_name: "Jam 1".into(),
        replace,
        pattern,
        set_tempo: true,
        key,
    }
}

// The edit refuses a Jam whose 4/4 bars would not be the pattern's: into a 3/4 project
// (p36_waltz.json; applied, 28 of its notes would lie past the pattern's end), and replacing a
// pattern that has a 3/4 signature of its own. Each project keeps its bytes. A new pattern in
// the 4/4 project with that 3/4 pattern follows the project and goes through.
#[test]
fn a_jam_is_refused_outside_4_4() {
    let three_four = TimeSignature::new(3, 4).expect("3/4");
    let (pattern, key) = band_jam(96);
    let mut waltz = load("p36_waltz.json");
    let before = waltz.to_json().expect("writes");
    let error = edit::apply(&mut waltz, jam_edit(pattern.clone(), None, key)).expect_err("refused");
    assert_eq!(error, EditError::JamMeter(three_four));
    assert_eq!(
        error.to_string(),
        "a Jam writes 4/4 patterns and the pattern it writes into is in 3/4"
    );
    assert_eq!(waltz.to_json().expect("writes"), before);

    let mut band = load("p36_band.json");
    edit::apply(
        &mut band,
        Edit::SetPatternTimeSignature {
            pattern: PatternId(1),
            signature: Some(three_four),
        },
    )
    .expect("pattern 1 in 3/4");
    let before = band.to_json().expect("writes");
    let error = edit::apply(
        &mut band,
        jam_edit(pattern.clone(), Some(PatternId(1)), key),
    )
    .expect_err("refused");
    assert_eq!(error, EditError::JamMeter(three_four));
    assert_eq!(band.to_json().expect("writes"), before);
    edit::apply(&mut band, jam_edit(pattern, None, key)).expect("a new pattern follows 4/4");
}

// The PPQ: a project at 4 or 12 (PPQs a project may have, not multiples of 8) refuses a Jam
// written at its own PPQ, where a half step would be 0 ticks or a third of a step; the band at 96
// refuses a Jam written at 192. Nothing changes.
#[test]
fn a_jam_needs_a_ppq_of_eights_and_the_projects_own() {
    for ppq in [4, 12] {
        let mut project = load("p36_band.json");
        project.ppq = ppq;
        project.validate().expect("a PPQ a project may have");
        let before = project.to_json().expect("writes");
        let (pattern, key) = band_jam(ppq);
        let error = edit::apply(&mut project, jam_edit(pattern, None, key)).expect_err("refused");
        assert_eq!(error, EditError::JamPpq(ppq));
        assert_eq!(
            error.to_string(),
            format!("a Jam needs a PPQ that is a multiple of 8, and the project's is {ppq}")
        );
        assert_eq!(project.to_json().expect("writes"), before);
    }
    let mut band = load("p36_band.json");
    let before = band.to_json().expect("writes");
    let (pattern, key) = band_jam(192);
    let error = edit::apply(&mut band, jam_edit(pattern, None, key)).expect_err("refused");
    assert_eq!(
        error,
        EditError::JamPpqMismatch {
            written: 192,
            project: 96
        }
    );
    assert_eq!(band.to_json().expect("writes"), before);
}

// A Jam naming a channel the project lacks is refused and changes nothing.
#[test]
fn a_jam_on_a_missing_channel_is_refused() {
    let mut project = load("p36_band.json");
    let before = project.to_json().expect("writes");
    let request = JamRequest {
        genre: Genre::Trap,
        key: KeySignature {
            root: PitchClass::C,
            scale: Scale::Minor,
        },
        bars: 4,
        seed: 9,
        ppq: 96,
        channels: vec![(ChannelId(9), Role::Bass).into()],
    };
    let refused = edit::apply(
        &mut project,
        Edit::Jam {
            pattern_name: "Jam 1".into(),
            replace: None,
            pattern: jam::write(&request),
            set_tempo: true,
            key: request.key,
        },
    );
    assert!(refused.is_err());
    assert_eq!(project.to_json().expect("writes"), before);
}

// The file: a project with a key writes version 11 and reads back byte for byte; one without
// keeps its version (the band 9, the 3/4 one 10); the key set to none gives the band's bytes;
// version 12 is refused by its number.
#[test]
fn a_key_makes_the_file_version_11() {
    for (name, version) in [
        ("p36_keyed.json", 11),
        ("p36_waltz.json", 10),
        ("p36_band.json", 9),
    ] {
        let file = text(name);
        let project = Project::load_json(&file).expect("loads");
        assert_eq!(project.to_json().expect("writes") + "\n", file, "{name}");
        assert!(
            file.starts_with(&format!("{{\"format_version\":{version},")),
            "{name}"
        );
    }
    let mut keyed = load("p36_keyed.json");
    assert_eq!(
        keyed.key.map(|k| k.to_string()).as_deref(),
        Some("D dorian")
    );
    edit::apply(&mut keyed, Edit::SetKey { key: None }).expect("applies");
    assert_eq!(
        keyed.to_json().expect("writes") + "\n",
        text("p36_band.json")
    );

    let later =
        text("p36_keyed.json").replacen(r#""format_version":11"#, r#""format_version":14"#, 1);
    match Project::load_json(&later) {
        Err(error @ LoadError::UnsupportedVersion(14)) => assert_eq!(
            error.to_string(),
            "format_version 14 is not supported (this build reads versions 1, 2, 4, 5, 6, 7, \
             8, 9, 10, 11, 12 and 13)"
        ),
        other => panic!("refused by its version: {other:?}"),
    }
    let bad_root = text("p36_keyed.json").replacen(r#""root":2"#, r#""root":12"#, 1);
    assert!(
        Project::load_json(&bad_root).is_err(),
        "pitch class 12 refused"
    );
}
