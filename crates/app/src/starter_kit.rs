//! The starter kit: twelve drums the app ships, listed with their hashes in
//! assets/sounds/kit.json (committed with their hashes; assets/sounds/check.py
//! checks them) and carried in the binary, so the Library lists them wherever the app runs. The
//! first time one is previewed or dragged, the kit is written into the user's starter-kit folder
//! and played from there, as the demo is; a sound already there is left as it is, the copy being
//! the user's to change. The kit's seven pitched sounds are instrument sets instead
//! (assets/instruments/), shipped with the other sets.

use std::fs;
use std::path::{Path, PathBuf};

use dawsome_engine::project::Project;

use crate::project_file::{self, SaveError};

/// One sound of the kit: the name the Library shows and a channel made from it takes, and the
/// WAV file's bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sound {
    pub name: &'static str,
    bytes: &'static [u8],
}

impl Sound {
    /// Its file in the kit folder `dir`: the name and ".wav".
    pub fn path_in(&self, dir: &Path) -> PathBuf {
        dir.join(format!("{}.wav", self.name))
    }
}

/// The kit in the order the Library lists it, kit.json's.
pub const SOUNDS: [Sound; 12] = [
    Sound {
        name: "Kick 808",
        bytes: include_bytes!("../../../assets/sounds/Kick 808.wav"),
    },
    Sound {
        name: "Kick 909",
        bytes: include_bytes!("../../../assets/sounds/Kick 909.wav"),
    },
    Sound {
        name: "Snare",
        bytes: include_bytes!("../../../assets/sounds/Snare.wav"),
    },
    Sound {
        name: "Clap",
        bytes: include_bytes!("../../../assets/sounds/Clap.wav"),
    },
    Sound {
        name: "Hat closed",
        bytes: include_bytes!("../../../assets/sounds/Hat closed.wav"),
    },
    Sound {
        name: "Hat open",
        bytes: include_bytes!("../../../assets/sounds/Hat open.wav"),
    },
    Sound {
        name: "Tom low",
        bytes: include_bytes!("../../../assets/sounds/Tom low.wav"),
    },
    Sound {
        name: "Tom mid",
        bytes: include_bytes!("../../../assets/sounds/Tom mid.wav"),
    },
    Sound {
        name: "Tom high",
        bytes: include_bytes!("../../../assets/sounds/Tom high.wav"),
    },
    Sound {
        name: "Rim",
        bytes: include_bytes!("../../../assets/sounds/Rim.wav"),
    },
    Sound {
        name: "Cowbell",
        bytes: include_bytes!("../../../assets/sounds/Cowbell.wav"),
    },
    Sound {
        name: "Crash",
        bytes: include_bytes!("../../../assets/sounds/Crash.wav"),
    },
];

/// The names of the kit's sounds, in its order.
pub fn names() -> [&'static str; SOUNDS.len()] {
    SOUNDS.map(|sound| sound.name)
}

/// Writes each sound of the kit that the folder `dir` lacks; a file already there is left as it
/// is. Another window may be writing the kit at the same time: a write that fails while the
/// sound is there after all counts as done, since every write is whole or absent.
pub fn install(dir: &Path) -> Result<(), SaveError> {
    for sound in &SOUNDS {
        let path = sound.path_in(dir);
        if path.exists() {
            continue;
        }
        fs::create_dir_all(dir).map_err(|source| SaveError::Write {
            path: path.clone(),
            source,
        })?;
        match project_file::write_atomically(&path, sound.bytes) {
            Err(_) if path.exists() => {}
            written => written?,
        }
    }
    Ok(())
}

/// Writes the kit into `dir` when `project` names one of its sounds by its shipped reference
/// (`dawsome:starter-kit/Kick 808.wav`), so a project made on another machine plays here as it
/// did there; nothing with no folder to write (a scripted run's) or no such sound named.
pub fn install_for(project: &Project, dir: Option<&Path>) -> Result<(), SaveError> {
    let named = project.channels.values().any(|channel| {
        dawsome_media::paths::shipped_part(&channel.sample)
            .and_then(|part| part.strip_prefix(KIT_FOLDER))
            .is_some_and(|file| {
                SOUNDS
                    .iter()
                    .any(|sound| file == format!("{}.wav", sound.name))
            })
    });
    match dir {
        Some(dir) if named => install(dir),
        _ => Ok(()),
    }
}

/// The kit's folder in the data folder's `dawsome/`, as a shipped reference names it.
const KIT_FOLDER: &str = "starter-kit/";

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::process::Command;

    use super::*;

    fn kit_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/sounds")
    }

    fn temp_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("dawsome-starter-kit-{}", std::process::id()))
            .join(test);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// What kit.json states of the kit's drums, in its order: the starter kit's sounds.
    fn drums() -> (u64, Vec<serde_json::Value>) {
        let text = fs::read_to_string(kit_dir().join("kit.json")).expect("kit.json");
        let manifest: serde_json::Value = serde_json::from_str(&text).expect("parses");
        let drums = manifest["sounds"]
            .as_array()
            .expect("sounds")
            .iter()
            .filter(|sound| sound["note"] == "drum")
            .cloned()
            .collect();
        (manifest["rate"].as_u64().expect("rate"), drums)
    }

    // Known answer: the sounds the binary carries are kit.json's drums, in its order, and each
    // holds what kit.json states: 24-bit at the kit's rate, its channels, its frames and its
    // peak (the largest absolute sample).
    #[test]
    fn each_sound_has_the_rate_channels_length_and_peak_kit_json_states() {
        let (rate, stated) = drums();
        let stated_names: Vec<&str> = stated
            .iter()
            .map(|sound| sound["name"].as_str().expect("a name"))
            .collect();
        assert_eq!(names().to_vec(), stated_names);
        for (sound, stated) in SOUNDS.iter().zip(&stated) {
            let mut reader = hound::WavReader::new(Cursor::new(sound.bytes)).expect("a WAV");
            let spec = reader.spec();
            assert_eq!(
                (
                    u64::from(spec.sample_rate),
                    u64::from(spec.channels),
                    spec.bits_per_sample
                ),
                (rate, stated["channels"].as_u64().expect("channels"), 24),
                "{}",
                sound.name
            );
            let samples: Vec<i32> = reader
                .samples::<i32>()
                .collect::<Result<_, _>>()
                .expect("samples");
            let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
            let frames = samples.len() / usize::from(spec.channels);
            assert_eq!(
                (frames as u64, u64::from(peak)),
                (
                    stated["frames"].as_u64().expect("frames"),
                    stated["peak"].as_u64().expect("peak")
                ),
                "{}",
                sound.name
            );
        }
    }

    // The kit goes into its folder once: each file as carried, and a file the user changed
    // stays as they left it when the kit is written again.
    #[test]
    fn the_kit_is_written_once_and_a_changed_copy_is_kept() {
        let dir = temp_dir("install").join("starter-kit");
        install(&dir).expect("installs");
        for sound in &SOUNDS {
            assert_eq!(
                fs::read(sound.path_in(&dir)).expect("written"),
                sound.bytes,
                "{}",
                sound.name
            );
        }
        let kick = SOUNDS[0].path_in(&dir);
        fs::write(&kick, b"changed").expect("writes");
        install(&dir).expect("installs");
        assert_eq!(fs::read(&kick).expect("reads"), b"changed");
    }

    /// check.py's --check of the kit in `sounds`, its sets in `sets`: whether it passed, and
    /// what it said.
    fn manifest_check(sounds: &Path, sets: &Path) -> (bool, String) {
        // Windows installs Python 3 as `python`; its `python3` may be a store shortcut that
        // runs nothing.
        let python = if cfg!(windows) { "python" } else { "python3" };
        let output = Command::new(python)
            .arg("-I")
            .arg(kit_dir().join("check.py"))
            .arg("--check")
            .arg(sounds)
            .arg(sets)
            .output()
            .expect("python runs");
        (
            output.status.success(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }

    /// Copies the folder `from` into `to`, files and folders.
    fn copy_dir(from: &Path, to: &Path) {
        fs::create_dir_all(to).expect("dir");
        for entry in fs::read_dir(from).expect("lists") {
            let entry = entry.expect("an entry");
            let target = to.join(entry.file_name());
            if entry.file_type().expect("a type").is_dir() {
                copy_dir(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).expect("copies");
            }
        }
    }

    /// Flips the last bit of `file`'s last byte.
    fn flip(file: &Path) {
        let mut bytes = fs::read(file).expect("reads");
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        fs::write(file, &bytes).expect("writes");
    }

    // The check scripts/check.sh runs passes on the kit and its sets as committed, and fails,
    // naming the file, when one byte of a drum or of a set's sound differs from what kit.json
    // states, or a sound it does not list lies beside the drums.
    #[test]
    fn the_manifest_check_fails_when_a_sound_differs() {
        let dir = temp_dir("check");
        let (sounds, sets) = (dir.join("sounds"), dir.join("instruments"));
        fs::create_dir_all(&sounds).expect("dir");
        for entry in fs::read_dir(kit_dir()).expect("the kit's folder") {
            let path = entry.expect("an entry").path();
            let name = path.file_name().expect("a name").to_owned();
            if path
                .extension()
                .is_some_and(|ext| ext == "wav" || ext == "json")
            {
                fs::copy(&path, sounds.join(name)).expect("copies");
            }
        }
        copy_dir(&kit_dir().join("../instruments"), &sets);
        assert_eq!(manifest_check(&sounds, &sets), (true, String::new()));
        let kick = sounds.join("Kick 808.wav");
        flip(&kick);
        let (passed, said) = manifest_check(&sounds, &sets);
        assert!(!passed && said.contains("Kick 808.wav"), "{said}");
        flip(&kick);
        let lead = sets.join("lead-c4/samples/Lead C4.wav");
        flip(&lead);
        let (passed, said) = manifest_check(&sounds, &sets);
        assert!(!passed && said.contains("Lead C4.wav"), "{said}");
        flip(&lead);
        fs::write(sounds.join("Extra.wav"), b"RIFF").expect("writes");
        let (passed, said) = manifest_check(&sounds, &sets);
        assert!(!passed && said.contains("Extra.wav"), "{said}");
    }

    // A project that names a kit sound by its shipped reference gets the kit written into the
    // kit folder when it opens; one that does not, or a run with no folder to write, gets none.
    #[test]
    fn a_project_naming_a_kit_sound_gets_the_kit_written() {
        use dawsome_engine::project::{Channel, ChannelId};
        use dawsome_engine::time::MilliBpm;
        let dir = temp_dir("install_for");
        let mut project = Project::new(44_100, MilliBpm(120_000), 96);
        project
            .channels
            .insert(ChannelId(1), Channel::sampler("Kick", "kick.wav"));
        project.next_channel_id = ChannelId(2);
        install_for(&project, Some(&dir)).expect("nothing to write");
        assert!(!dir.join("Kick 808.wav").exists(), "no kit sound named");
        project.channels.get_mut(&ChannelId(1)).expect("ch").sample =
            PathBuf::from("dawsome:starter-kit/Kick 808.wav");
        install_for(&project, None).expect("no folder");
        assert!(!dir.join("Kick 808.wav").exists(), "no folder to write");
        install_for(&project, Some(&dir)).expect("writes");
        assert_eq!(
            fs::read(dir.join("Kick 808.wav")).expect("written"),
            fs::read(kit_dir().join("Kick 808.wav")).expect("the kit's")
        );
    }

    // Two windows writing the kit at once, for the first time, both succeed: each writes its own
    // temporary files, so neither renames the other's away. Every sound ends whole and no
    // temporary file is left. Before, both wrote `<file>.tmp` and about half the installs failed.
    #[test]
    fn two_first_installs_at_once_both_succeed() {
        let root = temp_dir("concurrent");
        for round in 0..50 {
            let dir = root.join(format!("r{round}"));
            let start = std::sync::Barrier::new(2);
            let results: Vec<Result<(), SaveError>> = std::thread::scope(|scope| {
                let writers: Vec<_> = (0..2)
                    .map(|_| {
                        scope.spawn(|| {
                            start.wait();
                            install(&dir)
                        })
                    })
                    .collect();
                writers
                    .into_iter()
                    .map(|writer| writer.join().expect("the writer ran"))
                    .collect()
            });
            for result in results {
                result.unwrap_or_else(|error| panic!("round {round}: {error}"));
            }
            for sound in &SOUNDS {
                assert_eq!(
                    fs::read(sound.path_in(&dir)).expect("written"),
                    sound.bytes,
                    "round {round}: {}",
                    sound.name
                );
            }
            let files = fs::read_dir(&dir).expect("lists").count();
            assert_eq!(files, SOUNDS.len(), "round {round}: no temporary left");
        }
    }
}
