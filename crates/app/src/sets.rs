//! The instrument sets the Library lists, found on disk: the shipped ones, each a folder holding
//! a .sfz, its sounds and a `set.json` (name, maker, license), in the data folder's
//! `instruments/` or among the sets installed with the program, the user's folder shadowing an
//! installed set folder of the same name; and the user's own, the .sfz files in the folders
//! added to the Samples section and their subfolders two deep. Each is measured: the memory its
//! sounds take decoded. A shipped set folder whose `set.json` says `"kind": "samples"` holds
//! plain sounds instead: the Samples section lists it as a folder of its own.
//!
//! Why the user's sets come from the Samples section's folders rather than a folder list of
//! their own: one place to add a folder, and a set folder added there shows both its sounds and
//! the set that plays them.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use dawsome_engine::project::{Gate, Sound};
use dawsome_media::decode::probe;
use dawsome_media::set_files::{self, SetFolder};

use crate::ui::browser_model::ShippedFolder;
use crate::ui::library::{InstrumentSet, Kind, Kinds, Sets};

/// How deep under an added folder its .sfz files are looked for: the folder, its subfolders
/// and theirs.
const DEPTH: usize = 2;

/// The most sets of the user's own listed. Why a limit: an added folder may be a whole drive.
pub const MAX_OWN_SETS: usize = 500;

/// What a look on disk found: the instrument sets, and the shipped folders of plain sounds the
/// Samples section lists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    pub sets: Sets,
    pub folders: Vec<ShippedFolder>,
}

/// The sets shipped in `shipped` (one folder of set folders) and those in `folders`, each list
/// sorted by name ignoring case: [`scan_roots`] of one root, as the tests look.
#[cfg(test)]
pub fn scan(shipped: Option<&Path>, folders: &[PathBuf]) -> Sets {
    let roots: Vec<PathBuf> = shipped.map(Path::to_path_buf).into_iter().collect();
    scan_roots(&roots, folders).sets
}

/// The sets and folders of sounds shipped in `shipped`, folders of set folders in the order they
/// are looked in (the data folder's `instruments/` before the installed sets, as a
/// `dawsome:instruments/` reference is read: a set folder of the same name in an earlier one
/// shadows a later one's whole), and the sets in `folders`; each list sorted by name ignoring
/// case.
pub fn scan_roots(shipped: &[PathBuf], folders: &[PathBuf]) -> Found {
    let mut sets = Vec::new();
    let mut sounds = Vec::new();
    for folder in set_folders(shipped) {
        let manifest = manifest(&folder);
        if manifest["kind"] == "samples" {
            let samples = folder.join("samples");
            sounds.push(ShippedFolder {
                name: folder_name(&folder, &manifest),
                path: if samples.is_dir() { samples } else { folder },
            });
        } else if let Some(set) = shipped_set(&folder, &manifest) {
            sets.push(set);
        }
    }
    sort(&mut sets);
    sounds.sort_by(|a, b| (a.name.to_lowercase(), &a.path).cmp(&(b.name.to_lowercase(), &b.path)));
    Found {
        sets: Sets {
            shipped: sets,
            mine: own_sets(folders),
        },
        folders: sounds,
    }
}

/// The kinds of the dawsome-kit's sets, by folder name: the kit's `set.json` files name none
/// yet, and the kit is read-only here. A set whose `set.json` has a `category` goes by that.
const KIT_KINDS: [(&str, Kind); 10] = [
    ("drums-acoustic", Kind::Drums),
    ("drums-electronic", Kind::Drums),
    ("bass-electric", Kind::Bass),
    ("bass-synth", Kind::Bass),
    ("piano", Kind::Keys),
    ("organ", Kind::Keys),
    ("pad-choir", Kind::Pads),
    ("pad-sweep", Kind::Pads),
    ("strings", Kind::Strings),
    ("mallets", Kind::Mallets),
];

/// The kind of each of the shipped sets `shipped`: the `category` its folder's `set.json`
/// names, else the kit's kind for its folder's name ([`KIT_KINDS`]); a set with neither is
/// left out (the Library lists it as [`Kind::Other`]).
pub fn kinds(shipped: &[InstrumentSet]) -> Kinds {
    let kind_of = |set: &InstrumentSet| {
        let folder = set.sfz.parent()?;
        let named = manifest(folder)["category"]
            .as_str()
            .and_then(Kind::from_category);
        named.or_else(|| {
            let name = folder.file_name()?.to_str()?;
            KIT_KINDS
                .iter()
                .find(|(kit, _)| *kit == name)
                .map(|&(_, kind)| kind)
        })
    };
    Kinds(
        shipped
            .iter()
            .filter_map(|set| Some((set.sfz.clone(), kind_of(set)?)))
            .collect(),
    )
}

/// The set folders of `roots`, each folder name once: the first root's of that name. Hidden
/// folders (an install's half-copied one) are none.
fn set_folders(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut found = Vec::new();
    for root in roots {
        let Ok(entries) = fs::read_dir(root) else {
            continue;
        };
        let mut folders: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        folders.sort();
        for folder in folders {
            let Some(name) = folder.file_name() else {
                continue;
            };
            if !name.to_string_lossy().starts_with('.') && seen.insert(name.to_owned()) {
                found.push(folder);
            }
        }
    }
    found
}

/// A set folder's `set.json`, or nothing when it has none it can read.
fn manifest(folder: &Path) -> serde_json::Value {
    set_files::read_text(&folder.join("set.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// The name `set.json` gives, else the folder's own.
fn folder_name(folder: &Path, manifest: &serde_json::Value) -> String {
    manifest["name"].as_str().map_or_else(
        || {
            folder
                .file_name()
                .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
        },
        str::to_owned,
    )
}

/// The set in `folder` when it holds a .sfz: the one its `set.json` names (`"sfz"`), else its
/// first by name; named and credited by `set.json`, else after the folder.
fn shipped_set(folder: &Path, manifest: &serde_json::Value) -> Option<InstrumentSet> {
    let named = manifest["sfz"].as_str().map(|name| folder.join(name));
    let sfz = named
        .filter(|file| file.is_file())
        .or_else(|| sfz_files(folder).into_iter().next())?;
    Some(InstrumentSet {
        name: folder_name(folder, manifest),
        maker: manifest["maker"].as_str().unwrap_or_default().to_owned(),
        bytes: decoded_bytes(&sfz, &SetFolder::of(&sfz)),
        sfz,
    })
}

/// What a headless run prints of `found`, a line each, tab between the fields: `set`, a
/// shipped set's name, maker and main file; `mine`, the same of one of the user's own; and
/// `samples`, a shipped folder of sounds' name and folder.
pub fn listing(found: &Found) -> String {
    let mut out = String::new();
    for (kind, sets) in [("set", &found.sets.shipped), ("mine", &found.sets.mine)] {
        for set in sets {
            out.push_str(&format!(
                "{kind}\t{}\t{}\t{}\n",
                set.name,
                set.maker,
                set.sfz.display()
            ));
        }
    }
    for folder in &found.folders {
        out.push_str(&format!(
            "samples\t{}\t{}\n",
            folder.name,
            folder.path.display()
        ));
    }
    out
}

/// The .sfz files in `folders` and their subfolders [`DEPTH`] deep, those another one includes
/// left out (they are parts of a set, not sets), at most [`MAX_OWN_SETS`]; each measured as it
/// plays, reading inside the folder added that holds it.
fn own_sets(folders: &[PathBuf]) -> Vec<InstrumentSet> {
    let mut found: Vec<PathBuf> = Vec::new();
    for folder in folders {
        walk(folder, DEPTH, &mut found);
    }
    found.sort();
    found.dedup();
    let included: BTreeSet<PathBuf> = found.iter().flat_map(|file| includes(file)).collect();
    let mut sets: Vec<InstrumentSet> = found
        .into_iter()
        .filter(|file| !included.contains(file))
        .take(MAX_OWN_SETS)
        .map(|sfz| {
            let folder = match set_files::library_root(&sfz, folders) {
                Some(root) => SetFolder::inside(&sfz, &root),
                None => SetFolder::of(&sfz),
            };
            InstrumentSet {
                name: sfz
                    .file_stem()
                    .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned()),
                maker: String::new(),
                bytes: decoded_bytes(&sfz, &folder),
                sfz,
            }
        })
        .collect();
    sort(&mut sets);
    sets
}

fn walk(folder: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    if found.len() >= MAX_OWN_SETS * 2 {
        return;
    }
    found.extend(sfz_files(folder));
    if depth == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    let mut folders: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    folders.sort();
    for sub in folders {
        walk(&sub, depth - 1, found);
    }
}

/// The .sfz files directly in `folder`, by name.
fn sfz_files(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("sfz"))
        })
        .collect();
    files.sort();
    files
}

/// The files `sfz`'s own `#include` lines name, resolved beside it.
fn includes(sfz: &Path) -> Vec<PathBuf> {
    let Ok(text) = set_files::read_text(sfz) else {
        return Vec::new();
    };
    let dir = sfz.parent().unwrap_or(Path::new(""));
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("#include"))
        .map(|rest| dir.join(rest.trim().trim_matches('"').replace('\\', "/")))
        .collect()
}

/// The memory the sounds of the set at `sfz`, read inside `folder`, take decoded (as 32-bit
/// floats, two channels at most, at their files' own rates), each file once; `None` when the
/// set cannot be read.
fn decoded_bytes(sfz: &Path, folder: &SetFolder) -> Option<u64> {
    let read = set_files::read_set_in(sfz, folder).ok()?;
    let dir = sfz.parent().unwrap_or(Path::new(""));
    let samples: BTreeSet<&str> = read
        .regions
        .iter()
        .map(|region| region.sample.as_str())
        .collect();
    let bytes = samples
        .into_iter()
        .filter_map(|sample| probe(&dir.join(sample)).ok())
        .map(|info| info.frames.unwrap_or(0) * info.channels.min(2) as u64 * 4)
        .sum();
    Some(bytes)
}

/// The sound file of the zone a note of middle C at velocity 100 sounds in the set at `sfz`
/// (read as the Library reads it: inside a folder of `folders` it lies in, else inside its
/// own), else its first zone's: a note at that zone's root key, what a press on the set's row
/// previews. `None` when the set cannot be read or has no zone.
pub fn root_sound(sfz: &Path, folders: &[PathBuf]) -> Option<PathBuf> {
    let folder = match set_files::library_root(sfz, folders) {
        Some(root) => SetFolder::inside(sfz, &root),
        None => SetFolder::of(sfz),
    };
    let read = set_files::read_set_in(sfz, &folder).ok()?;
    let (key, velocity) = (60, 100);
    let region = read
        .regions
        .iter()
        .find(|region| {
            (region.lokey..=region.hikey).contains(&key)
                && (region.lovel..=region.hivel).contains(&velocity)
        })
        .or_else(|| read.regions.first())?;
    folder.file(&region.sample).ok()
}

/// One zone of a set as the instrument window shows it: its keys, velocities, root and sound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneInfo {
    pub keys: (u8, u8),
    pub velocities: (u8, u8),
    pub root: u8,
    /// The sound file as the set names it.
    pub sample: String,
}

/// The regions of the set at `sfz`, read as the Library reads it (inside a folder of `folders`
/// it lies in, else inside its own), in file order; none when it cannot be read.
fn regions(sfz: &Path, folders: &[PathBuf]) -> Vec<dawsome_engine::sfz::Region> {
    let folder = match set_files::library_root(sfz, folders) {
        Some(root) => SetFolder::inside(sfz, &root),
        None => SetFolder::of(sfz),
    };
    set_files::read_set_in(sfz, &folder).map_or_else(|_| Vec::new(), |read| read.regions)
}

/// The sound settings a channel of the set at `sfz` is added (and previewed) with: the
/// defaults, cutting itself when the `set.json` beside it says `"cut_itself": true` (the two
/// 808 basses, `assets/sounds/check.py --write-sets` from kit.json): a new note ends the note
/// before it, as a bass string does. Read as the set is added, so a channel keeps what it was
/// added with, and a project made before keeps its channels as they were.
pub fn added_sound(sfz: &Path) -> Sound {
    let cuts = sfz
        .parent()
        .is_some_and(|folder| manifest(folder)["cut_itself"] == true);
    Sound {
        cut_itself: cuts,
        ..Sound::default()
    }
}

/// What the set at `sfz` does at a note's end with no gate of the channel's: one-shot when
/// every zone is `loop_mode=one_shot`, hold when none is; none for a set of both kinds, or one
/// that cannot be read, whose zones each go their own way.
pub fn own_gate(sfz: &Path, folders: &[PathBuf]) -> Option<Gate> {
    let regions = regions(sfz, folders);
    let one_shot = |region: &dawsome_engine::sfz::Region| {
        region.loop_mode == Some(dawsome_engine::sfz::LoopMode::OneShot)
    };
    if regions.is_empty() {
        None
    } else if regions.iter().all(one_shot) {
        Some(Gate::OneShot)
    } else if !regions.iter().any(one_shot) {
        Some(Gate::Hold)
    } else {
        None
    }
}

/// The zones of the set at `sfz`, read as the Library reads it (inside a folder of `folders` it
/// lies in, else inside its own), in file order; none when it cannot be read.
pub fn zones(sfz: &Path, folders: &[PathBuf]) -> Vec<ZoneInfo> {
    regions(sfz, folders)
        .iter()
        .map(|region| ZoneInfo {
            keys: (region.lokey, region.hikey),
            velocities: (region.lovel, region.hivel),
            root: region.pitch_keycenter,
            sample: region.sample.clone(),
        })
        .collect()
}

/// By name ignoring case, then by file, so equal names keep one order.
fn sort(sets: &mut [InstrumentSet]) {
    sets.sort_by(|a, b| (a.name.to_lowercase(), &a.sfz).cmp(&(b.name.to_lowercase(), &b.sfz)));
}

/// `bytes` as people read memory: "512 KB", "3.4 MB", "1.2 GB" (powers of 1000).
pub fn size_text(bytes: u64) -> String {
    let value = bytes as f64;
    if bytes < 1_000_000 {
        format!("{} KB", (value / 1000.0).ceil() as u64)
    } else if bytes < 1_000_000_000 {
        format!("{:.1} MB", value / 1e6)
    } else {
        format!("{:.1} GB", value / 1e9)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Known answers from the test sets' own files: Round robin is two zones on key 60, Layers
    // seven over three key ranges rooted at 48, 60 and 72.
    #[test]
    fn a_sets_zones_are_its_regions_keys_and_roots() {
        let sets = fixtures().join("instruments");
        let rr = zones(&sets.join("rr/rr.sfz"), &[]);
        assert_eq!(rr.len(), 2);
        assert!(rr
            .iter()
            .all(|zone| zone.keys == (60, 60) && zone.root == 60));
        assert_eq!(rr[1].sample, "rr2.wav");
        let layers = zones(&sets.join("layers/layers.sfz"), &[]);
        assert_eq!(layers.len(), 7);
        let roots: Vec<u8> = layers.iter().map(|zone| zone.root).collect();
        assert_eq!(roots, [48, 60, 72, 48, 60, 72, 72]);
        assert_eq!((layers[0].keys, layers[0].velocities), ((0, 54), (1, 64)));
        assert!(zones(&sets.join("missing.sfz"), &[]).is_empty());
    }

    // The shipped 808 basses' set.json say "cut_itself": true (check.py writes it from
    // kit.json), so the Library adds their channels cutting themselves; the other shipped sets,
    // and a set with no set.json, at the defaults.
    #[test]
    fn the_808_sets_are_added_cutting_themselves() {
        let shipped = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/instruments");
        for (folder, cuts) in [
            ("bass-808-c1", true),
            ("bass-808-c2", true),
            ("bass-synth-c2", false),
            ("lead-c4", false),
            ("pad-bright-c3", false),
            ("pad-warm-c3", false),
            ("pluck-c4", false),
        ] {
            let sfz = shipped.join(folder).join(format!("{folder}.sfz"));
            assert!(sfz.is_file(), "{}", sfz.display());
            let want = Sound {
                cut_itself: cuts,
                ..Sound::default()
            };
            assert_eq!(added_sound(&sfz), want, "{folder}");
        }
        assert_eq!(added_sound(Path::new("no/such/set.sfz")), Sound::default());
    }

    // Known answers from the test sets' own files: Layers' middle C at velocity 100 is its loud
    // middle layer, Pad's lower zone, Round robin's first, Macro's "tone a.wav" under its
    // default path.
    #[test]
    fn a_sets_preview_is_the_sound_of_its_middle_c() {
        let sets = fixtures().join("instruments");
        let sound = |set: &str, sfz: &str| root_sound(&sets.join(set).join(sfz), &[]);
        let is = |found: Option<PathBuf>, set: &str, file: &str| {
            let found = found.expect("a sound");
            assert!(found.ends_with(Path::new(set).join(file)), "{found:?}");
            assert!(found.is_file(), "{found:?}");
        };
        is(sound("layers", "layers.sfz"), "layers", "b_loud.wav");
        is(sound("pad", "pad.sfz"), "pad", "ramp.wav");
        is(sound("rr", "rr.sfz"), "rr", "rr1.wav");
        is(sound("macro", "macro.sfz"), "macro", "sounds/tone a.wav");
        assert_eq!(sound("pad", "missing.sfz"), None);
    }

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
    }

    // The test sets as shipped: named by set.json, sorted ignoring case, each measured: layers
    // holds seven mono files of 100 frames (2800 bytes as floats), pad one of 48 (192), rr two
    // of 20 (160), macro a stereo 16 and a mono 60 (128 + 240 = 368).
    #[test]
    fn shipped_sets_are_named_by_their_manifest_and_measured() {
        let sets = scan(Some(&fixtures().join("instruments")), &[]);
        let listed: Vec<(&str, &str, Option<u64>)> = sets
            .shipped
            .iter()
            .map(|set| (set.name.as_str(), set.maker.as_str(), set.bytes))
            .collect();
        assert_eq!(
            listed,
            [
                ("Layers", "Dawsome tests", Some(2800)),
                ("Macro", "Dawsome tests", Some(368)),
                ("Pad", "Dawsome tests", Some(192)),
                ("Round robin", "Dawsome tests", Some(160)),
            ]
        );
        assert!(sets.shipped[1].sfz.ends_with("instruments/macro/macro.sfz"));
        assert!(sets.mine.is_empty());
    }

    // A folder the user added lists its .sfz files two folders deep, named after the file;
    // regions.sfz, which macro.sfz includes, is part of a set and not listed.
    #[test]
    fn the_users_folders_list_their_sets_but_not_their_parts() {
        let sets = scan(None, &[fixtures().join("instruments")]);
        let names: Vec<&str> = sets.mine.iter().map(|set| set.name.as_str()).collect();
        assert_eq!(names, ["layers", "macro", "pad", "rr"]);
        assert!(sets.shipped.is_empty());
        let none = scan(None, &[fixtures().join("browser")]);
        assert!(none.mine.is_empty());
    }

    // The scan reads a set's files inside its folder only, and only regular ones: a set that
    // includes a file above its folder (whose region names a sound there) and a named pipe is
    // measured as the same set without those lines, and the scan does not wait on the pipe.
    #[test]
    fn the_scan_measures_only_what_lies_inside_a_set() {
        let root = std::env::temp_dir()
            .join(format!("dawsome-sets-{}", std::process::id()))
            .join("inside");
        let _ = fs::remove_dir_all(&root);
        let (plain, escaping) = (root.join("added/plain"), root.join("added/escaping"));
        for set in [&plain, &escaping] {
            fs::create_dir_all(set).expect("dir");
            fs::copy(fixtures().join("impulse.wav"), set.join("own.wav")).expect("copies");
        }
        fs::copy(fixtures().join("hold2000.wav"), root.join("big.wav")).expect("copies");
        fs::write(root.join("outside.sfz"), "<region> sample=big.wav\n").expect("written");
        fs::write(plain.join("plain.sfz"), "<region> sample=own.wav\n").expect("written");
        let text = "<region> sample=own.wav\n#include \"../../outside.sfz\"\n".to_owned();
        // Why a new binding and not a push: off Unix there is no pipe, and a `mut` only Unix
        // uses is a Windows build's warning.
        #[cfg(unix)]
        let text = {
            let made = std::process::Command::new("mkfifo")
                .arg(escaping.join("pipe.sfz"))
                .status()
                .expect("mkfifo runs");
            assert!(made.success());
            text + "#include \"pipe.sfz\"\n"
        };
        fs::write(escaping.join("escaping.sfz"), text).expect("written");
        let sets = scan(None, &[root.join("added")]);
        let measured: Vec<(&str, Option<u64>)> = sets
            .mine
            .iter()
            .map(|set| (set.name.as_str(), set.bytes))
            .collect();
        let own = measured[1].1;
        assert!(own.is_some_and(|bytes| bytes > 0));
        assert_eq!(measured, [("escaping", own), ("plain", own)]);
    }

    // A set in a folder added reads inside that folder, so its size counts the sounds its
    // library keeps beside its own folder: lib/Instruments/a.sfz playing ../Samples/s.wav
    // measures as a set playing the same file beside it.
    #[test]
    fn a_library_sets_size_counts_its_sibling_sample_folder() {
        let root = std::env::temp_dir()
            .join(format!("dawsome-sets-{}", std::process::id()))
            .join("library");
        let _ = fs::remove_dir_all(&root);
        let (lib, plain) = (root.join("lib"), root.join("plain"));
        for folder in [lib.join("Instruments"), lib.join("Samples"), plain.clone()] {
            fs::create_dir_all(folder).expect("dir");
        }
        for to in [lib.join("Samples/s.wav"), plain.join("s.wav")] {
            fs::copy(fixtures().join("impulse.wav"), to).expect("copies");
        }
        fs::write(
            lib.join("Instruments/a.sfz"),
            "<region> sample=../Samples/s.wav\n",
        )
        .expect("written");
        fs::write(plain.join("b.sfz"), "<region> sample=s.wav\n").expect("written");
        let sets = scan(None, &[lib, plain]);
        let measured: Vec<(&str, Option<u64>)> = sets
            .mine
            .iter()
            .map(|set| (set.name.as_str(), set.bytes))
            .collect();
        let own = measured[1].1;
        assert!(own.is_some_and(|bytes| bytes > 0));
        assert_eq!(measured, [("a", own), ("b", own)]);
    }

    // Two folders of shipped sets, the user's data folder first: a set folder both hold is
    // listed from the user's alone (here rr's set in a folder named "pad" shadows the pad set);
    // the rest of each are listed. A set folder whose set.json says "kind": "samples" is no
    // instrument set: it is a shipped folder of sounds, its samples/ folder, named by set.json.
    #[test]
    fn the_users_set_shadows_an_installed_one_and_sample_folders_are_found() {
        let root = std::env::temp_dir()
            .join(format!("dawsome-sets-{}", std::process::id()))
            .join("roots");
        let _ = fs::remove_dir_all(&root);
        let (user, installed) = (root.join("user"), root.join("installed"));
        copy_dir(&fixtures().join("instruments/rr"), &user.join("pad"));
        copy_dir(&fixtures().join("instruments/pad"), &installed.join("pad"));
        copy_dir(
            &fixtures().join("instruments/layers"),
            &installed.join("layers"),
        );
        let loops = installed.join("one-shots");
        fs::create_dir_all(loops.join("samples")).expect("dir");
        fs::write(
            loops.join("set.json"),
            r#"{"name":"One-shots and Loops","kind":"samples","maker":"Someone"}"#,
        )
        .expect("written");
        let found = scan_roots(&[user.clone(), installed.clone()], &[]);
        let listed: Vec<(&str, PathBuf)> = found
            .sets
            .shipped
            .iter()
            .map(|set| (set.name.as_str(), set.sfz.clone()))
            .collect();
        assert_eq!(
            listed,
            [
                ("Layers", installed.join("layers/layers.sfz")),
                ("Round robin", user.join("pad/rr.sfz")),
            ]
        );
        assert_eq!(
            found.folders,
            [ShippedFolder {
                name: "One-shots and Loops".to_owned(),
                path: loops.join("samples"),
            }]
        );
    }

    /// Copies the folder `from` into `to`, files and folders.
    fn copy_dir(from: &Path, to: &Path) {
        fs::create_dir_all(to).expect("dir");
        for entry in fs::read_dir(from).expect("lists") {
            let entry = entry.expect("entry");
            let target = to.join(entry.file_name());
            if entry.file_type().expect("type").is_dir() {
                copy_dir(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).expect("copies");
            }
        }
    }

    // The headless listing names each set with its maker and main file, and each shipped
    // folder of sounds, a line each, tab between the fields.
    #[test]
    fn the_listing_names_each_set_and_folder() {
        let found = Found {
            sets: Sets {
                shipped: vec![InstrumentSet {
                    name: "Piano".to_owned(),
                    maker: "FreePats".to_owned(),
                    sfz: PathBuf::from("/s/piano/piano.sfz"),
                    bytes: None,
                }],
                mine: vec![InstrumentSet {
                    name: "pad".to_owned(),
                    maker: String::new(),
                    sfz: PathBuf::from("/m/pad.sfz"),
                    bytes: None,
                }],
            },
            folders: vec![ShippedFolder {
                name: "Loops".to_owned(),
                path: PathBuf::from("/s/loops/samples"),
            }],
        };
        assert_eq!(
            listing(&found),
            "set\tPiano\tFreePats\t/s/piano/piano.sfz\nmine\tpad\t\t/m/pad.sfz\nsamples\tLoops\t/s/loops/samples\n"
        );
    }

    // B6: a set's kind is its set.json's `category` (any case), else the kit's kind for its
    // folder's name; a set with neither, or a category the Library does not know, has none.
    #[test]
    fn a_sets_kind_is_its_category_else_the_kits_for_its_folder() {
        let root = std::env::temp_dir().join(format!("dawsome-kinds-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let set_in = |folder: &str, manifest: Option<&str>| {
            let dir = root.join(folder);
            fs::create_dir_all(&dir).expect("dir");
            if let Some(manifest) = manifest {
                fs::write(dir.join("set.json"), manifest).expect("writes");
            }
            InstrumentSet {
                name: folder.to_owned(),
                maker: String::new(),
                sfz: dir.join("main.sfz"),
                bytes: None,
            }
        };
        let sets = [
            set_in("ep", Some(r#"{"name":"Electric Piano","category":"keys"}"#)),
            set_in("strings", None),
            set_in("piano", Some(r#"{"name":"Piano","category":"Synth"}"#)),
            set_in("pad", None),
            set_in("odd", Some(r#"{"name":"Odd","category":"Woodwinds"}"#)),
        ];
        let kinds = kinds(&sets);
        let of: Vec<Kind> = sets.iter().map(|set| kinds.of(set)).collect();
        assert_eq!(
            of,
            [
                Kind::Keys,
                Kind::Strings,
                Kind::Synth,
                Kind::Other,
                Kind::Other
            ]
        );
        assert_eq!(kinds.0.len(), 3);
        let _ = fs::remove_dir_all(&root);
    }

    // The shipped electronic sets (assets/instruments) say their kinds: the basses Bass, the
    // pads Pads, the lead and the pluck Synth, as check.py writes them.
    #[test]
    fn the_electronic_sets_say_their_kinds() {
        let shipped = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/instruments");
        let found = scan_roots(&[shipped], &[]);
        let kinds = kinds(&found.sets.shipped);
        let mut by_name: Vec<(String, Kind)> = found
            .sets
            .shipped
            .iter()
            .map(|set| (set.name.clone(), kinds.of(set)))
            .collect();
        by_name.sort();
        let expected = [
            ("Bass 808 high", Kind::Bass),
            ("Bass 808 low", Kind::Bass),
            ("Bass synth", Kind::Bass),
            ("Lead", Kind::Synth),
            ("Pad bright", Kind::Pads),
            ("Pad warm", Kind::Pads),
            ("Pluck", Kind::Synth),
        ];
        let expected: Vec<(String, Kind)> = expected
            .iter()
            .map(|&(name, kind)| (name.to_owned(), kind))
            .collect();
        assert_eq!(by_name, expected);
    }

    #[test]
    fn sizes_read_as_kb_mb_and_gb() {
        assert_eq!(size_text(2800), "3 KB");
        assert_eq!(size_text(142_300_000), "142.3 MB");
        assert_eq!(size_text(1_500_000_000), "1.5 GB");
    }
}
