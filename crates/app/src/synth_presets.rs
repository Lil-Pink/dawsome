//! Dawsome Synth's presets: a sound kept by name, to load into any synth channel. A preset is a
//! small JSON file, `{"synth":"dawsome.synth","name":"Saw Lead","params":{...}}`, whose map holds
//! only the parameters that differ from their defaults: loading one sets exactly its map, so a
//! parameter it does not name plays at its default, and a preset made before a parameter
//! existed still loads as it sounded.
//!
//! The factory presets are the files in `assets/synth-presets/`, built into the program: nothing
//! to install, find or lose. The user's are files in `<data folder>/dawsome/synth-presets/`, each
//! named after its preset, the name kept to letters, digits, spaces, `-` and `_` ([`sanitized`])
//! so any name the user types is a file name on every system.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use dawsome_engine::dsp::synth;
use dawsome_engine::processor::{param_values, ParamInfo};
use serde::{Deserialize, Serialize};

use crate::project_file::{write_atomically, SaveError};

/// The factory presets' files, in the order the list shows them: Init first, then the starting
/// sounds.
const FACTORY_FILES: [&str; 8] = [
    include_str!("../../../assets/synth-presets/init.json"),
    include_str!("../../../assets/synth-presets/saw-lead.json"),
    include_str!("../../../assets/synth-presets/square-bass.json"),
    include_str!("../../../assets/synth-presets/sub.json"),
    include_str!("../../../assets/synth-presets/pad.json"),
    include_str!("../../../assets/synth-presets/pluck.json"),
    include_str!("../../../assets/synth-presets/keys.json"),
    include_str!("../../../assets/synth-presets/brass.json"),
];

/// The longest name a preset keeps, in characters.
const LONGEST_NAME: usize = 64;

/// A sound kept by name: the synth's parameters that differ from their defaults, by key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    /// The instrument it is for: Dawsome Synth's permanent id, whatever the synth is called.
    synth: String,
    pub name: String,
    pub params: BTreeMap<String, f32>,
}

impl Preset {
    /// A preset named `name` of the sound `values` (one per parameter of `params`, in order):
    /// the values that differ from their defaults, by key.
    pub fn of(name: String, params: &[ParamInfo], values: &[f32]) -> Self {
        let params = params
            .iter()
            .zip(values)
            .filter(|(info, &value)| value != info.default)
            .map(|(info, &value)| (info.key.to_owned(), value))
            .collect();
        Self {
            synth: synth::ID.to_owned(),
            name,
            params,
        }
    }

    /// Whether the sound `saved` (a synth channel's map) is this preset's: every parameter of
    /// `params` at the value the preset gives it.
    pub fn sounds_as(&self, params: &[ParamInfo], saved: &BTreeMap<String, f32>) -> bool {
        param_values(params, saved) == param_values(params, &self.params)
    }

    /// The text its file holds.
    pub fn to_json(&self) -> String {
        // A map of strings to numbers always writes; the fallback is never taken.
        let mut text = serde_json::to_string_pretty(self).unwrap_or_default();
        text.push('\n');
        text
    }
}

/// The preset `text` holds, or why it holds none: not JSON of a preset, another instrument's,
/// no name, or a value that is not a number.
pub fn parse(text: &str) -> Result<Preset, String> {
    let preset: Preset = serde_json::from_str(text).map_err(|error| error.to_string())?;
    if preset.synth != synth::ID {
        return Err(format!("a preset for \"{}\", not this synth", preset.synth));
    }
    if preset.name.trim().is_empty() {
        return Err("a preset with no name".to_owned());
    }
    // JSON has no infinity, but a number past f32's range reads as one.
    if let Some((key, _)) = preset.params.iter().find(|(_, value)| !value.is_finite()) {
        return Err(format!("\"{key}\" is not a number the synth takes"));
    }
    Ok(preset)
}

/// The factory presets, in the order the list shows them.
pub fn factory() -> &'static [Preset] {
    static FACTORY: LazyLock<Vec<Preset>> = LazyLock::new(|| {
        // Every file parses: `every_factory_preset_parses_and_names_only_real_keys`.
        FACTORY_FILES
            .iter()
            .filter_map(|text| parse(text).ok())
            .collect()
    });
    &FACTORY
}

/// `name` as a preset is saved under: its letters, digits, spaces, `-` and `_` (every other
/// character left out), at most [`LONGEST_NAME`] of them, without spaces at either end. Empty
/// when nothing is left.
pub fn sanitized(name: &str) -> String {
    let kept: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        .take(LONGEST_NAME)
        .collect();
    kept.trim().to_owned()
}

/// Whether `name` is a factory preset's, in any case: the user's presets take other names, so
/// no two rows of the list read alike.
pub fn is_factory_name(name: &str) -> bool {
    factory()
        .iter()
        .any(|preset| preset.name.eq_ignore_ascii_case(name))
}

/// The file the user's preset `name` (already [`sanitized`]) is kept in, in `dir`.
pub fn file_in(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.json"))
}

/// The user's presets in `dir`, by name (in any case): every `.json` file there that holds one.
/// A file that does not is passed over, as is a folder that cannot be read (none saved yet).
pub fn user_presets(dir: &Path) -> Vec<Preset> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut presets: Vec<Preset> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .filter_map(|path| parse(&fs::read_to_string(path).ok()?).ok())
        .collect();
    presets.sort_by_cached_key(|preset| preset.name.to_lowercase());
    presets
}

/// Writes `preset` into `dir` (made when missing) as its file, over one of the same name.
pub fn save(dir: &Path, preset: &Preset) -> Result<(), SaveError> {
    let path = file_in(dir, &preset.name);
    fs::create_dir_all(dir).map_err(|source| SaveError::Write {
        path: path.clone(),
        source,
    })?;
    write_atomically(&path, preset.to_json().as_bytes())
}

#[cfg(test)]
mod tests {
    use dawsome_engine::processor::Registry;

    use super::*;

    fn synth_params() -> &'static [ParamInfo] {
        Registry::native()
            .get(synth::ID)
            .expect("registered")
            .params()
    }

    // The eight files of assets/synth-presets each parse, in the list's order, Init first and
    // empty (the defaults); each names only the synth's own parameters, inside their ranges,
    // whole where a parameter must be, and none at its default (a file holds what differs).
    // The factory values, by hand: Saw Lead's second oscillator a saw 7 cents up, its glide 60 ms.
    #[test]
    fn every_factory_preset_parses_and_names_only_real_keys() {
        let names: Vec<String> = FACTORY_FILES
            .iter()
            .map(|text| parse(text).expect("a factory preset parses").name)
            .collect();
        assert_eq!(
            names,
            [
                "Init",
                "Saw Lead",
                "Square Bass",
                "Sub",
                "Pad",
                "Pluck",
                "Keys",
                "Brass"
            ]
        );
        assert_eq!(factory().len(), 8, "all eight are offered");
        assert!(factory()[0].params.is_empty(), "Init is the defaults");
        let params = synth_params();
        for preset in factory() {
            for (key, &value) in &preset.params {
                let info = params
                    .iter()
                    .find(|info| info.key == key)
                    .unwrap_or_else(|| panic!("{}: \"{key}\" is no parameter", preset.name));
                assert_eq!(info.clamp(value), value, "{}: {key} {value}", preset.name);
                assert_ne!(value, info.default, "{}: {key} at its default", preset.name);
            }
        }
        let lead = &factory()[1].params;
        assert_eq!(lead.get("osc2_wave"), Some(&2.0));
        assert_eq!(lead.get("osc2_cents"), Some(&7.0));
        assert_eq!(lead.get("glide_ms"), Some(&60.0));
    }

    // A name keeps its letters, digits, spaces, - and _: a slash, a colon, dots and accents go,
    // so no name climbs out of the folder or names a file no system can hold; spaces at either
    // end go; past 64 characters the rest goes; nothing kept is empty.
    #[test]
    fn a_name_keeps_letters_digits_spaces_dashes_and_underscores() {
        assert_eq!(sanitized("My Lead_2-b"), "My Lead_2-b");
        assert_eq!(sanitized("Bass: v2/final!"), "Bass v2final");
        assert_eq!(sanitized("../../etc/passwd"), "etcpasswd");
        assert_eq!(sanitized("  Café  "), "Caf");
        assert_eq!(sanitized("?*<>|"), "");
        assert_eq!(sanitized(&"a".repeat(100)).len(), 64);
        assert!(is_factory_name("saw lead"), "in any case");
        assert!(!is_factory_name("Saw Lead 2"));
    }

    // A preset saved into a temp data folder reads back as itself: only the values off their
    // defaults (the volume at −12 dB, the second oscillator a triangle), its file named by its
    // name; a file of another instrument's, one that is not JSON and one with no name beside it
    // are passed over; presets list by name in any case.
    #[test]
    fn a_saved_preset_reads_back_as_itself() {
        let dir = std::env::temp_dir().join(format!(
            "dawsome-synth-presets-{}-round-trip",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let params = synth_params();
        let mut values: Vec<f32> = params.iter().map(|info| info.default).collect();
        let at = |key: &str| params.iter().position(|info| info.key == key).expect(key);
        values[at("volume_db")] = -12.0;
        values[at("osc2_wave")] = 1.0;
        let preset = Preset::of("my lead".to_owned(), params, &values);
        assert_eq!(
            preset.params,
            BTreeMap::from([
                ("osc2_wave".to_owned(), 1.0),
                ("volume_db".to_owned(), -12.0)
            ])
        );
        save(&dir, &preset).expect("saves");
        let other = Preset::of("Alpha".to_owned(), params, &values);
        save(&dir, &other).expect("saves");
        fs::write(dir.join("strange.json"), "not json").expect("writes");
        fs::write(
            dir.join("other.json"),
            r#"{"synth":"other.synth","name":"Other","params":{}}"#,
        )
        .expect("writes");
        fs::write(
            dir.join("nameless.json"),
            r#"{"synth":"dawsome.synth","name":" ","params":{}}"#,
        )
        .expect("writes");
        let text = fs::read_to_string(file_in(&dir, "my lead")).expect("its file");
        assert_eq!(parse(&text), Ok(preset.clone()));
        assert_eq!(user_presets(&dir), [other, preset.clone()]);
        let mut saved = BTreeMap::from([("volume_db".to_owned(), -12.0)]);
        assert!(!preset.sounds_as(params, &saved));
        saved.insert("osc2_wave".to_owned(), 1.0);
        saved.insert("cutoff".to_owned(), 1.0);
        assert!(
            preset.sounds_as(params, &saved),
            "a default named is a default"
        );
        fs::remove_dir_all(&dir).expect("cleans up");
        assert!(user_presets(&dir).is_empty(), "no folder: none");
    }
}
