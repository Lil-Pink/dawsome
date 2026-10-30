//! Project documents on disk: opening one, saving atomically, and where autosaves and settings
//! live. The app reads and writes documents only through this module.

use std::env;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use dawsome_engine::dsp::limiter;
use dawsome_engine::project::{
    Channel, ChannelId, Effect, EffectId, InsertId, LoadError, LoadNote, Pattern, PatternId,
    ProcessorRef, Project,
};
use dawsome_engine::time::MilliBpm;
use thiserror::Error;

/// Why a document could not be opened.
#[derive(Debug, Error)]
pub enum OpenError {
    #[error("cannot read {}: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("{}: {source}", path.display())]
    Invalid { path: PathBuf, source: LoadError },
}

/// Why a document could not be saved. The file that was there is left as it was.
#[derive(Debug, Error)]
pub enum SaveError {
    #[error("cannot save {}: {source}", path.display())]
    Write { path: PathBuf, source: io::Error },
    #[error("cannot save {}: a sample path is not valid text ({source})", path.display())]
    Serialize {
        path: PathBuf,
        source: serde_json::Error,
    },
}

/// An opened document: the project and the directory its relative sample paths start at.
#[derive(Debug)]
pub struct Opened {
    pub project: Project,
    pub samples_dir: PathBuf,
}

/// Opens the document at `path`, read as Dawsome JSON (migrated, then validated).
pub fn open(path: &Path) -> Result<Opened, OpenError> {
    open_noting(path).map(|(opened, _)| opened)
}

/// As [`open`], with what reading the document left out of it, for the window to tell.
pub fn open_noting(path: &Path) -> Result<(Opened, Vec<LoadNote>), OpenError> {
    let bytes = fs::read(path).map_err(|source| OpenError::Read {
        path: path.to_owned(),
        source,
    })?;
    let samples_dir = samples_dir(path);
    let invalid = |source| OpenError::Invalid {
        path: path.to_owned(),
        source,
    };
    let text = String::from_utf8(bytes).map_err(|error| {
        let error = io::Error::new(io::ErrorKind::InvalidData, error);
        invalid(LoadError::Json(serde_json::Error::io(error)))
    })?;
    let (project, notes) = Project::load_json_noting(&text).map_err(invalid)?;
    let opened = Opened {
        project,
        samples_dir,
    };
    Ok((opened, notes))
}

/// The directory the sample paths of the project at `path` are relative to: the project
/// file's own directory.
pub fn samples_dir(path: &Path) -> PathBuf {
    path.parent().unwrap_or(Path::new("")).to_owned()
}

/// The rate a new document plays at with no output device open.
pub const FALLBACK_RATE: u32 = 44_100;

/// The rate a new document takes: the rate the open output device plays at on its own
/// (`device`, [`crate::audio::own_rate`]), so the output plays it without resampling and an
/// input on the same audio server opens at it; [`FALLBACK_RATE`] with no device open. A file
/// keeps the rate it was saved at.
pub fn new_document_rate(device: Option<u32>) -> u32 {
    device.unwrap_or(FALLBACK_RATE)
}

/// The name of a sampler channel with no sound yet, as a new document has it.
pub const EMPTY_SAMPLER: &str = "Sampler";

/// The document a window opened with no file, and File → New, start with ([`FALLBACK_RATE`];
/// they set the open output device's rate, [`new_document_rate`]): 44.1 kHz, 120 BPM,
/// Pattern 1 of one bar, the default 8 tracks and 16 inserts, the Limiter at its defaults in the
/// master's first slot, and one sampler channel named "Sampler" with no sound yet (it plays
/// silence, with nothing to warn about, until a file is given), at the volume an add gives a
/// channel ([`Channel::as_added`]). Why not empty: with no pattern and no channel there was
/// nothing to put a step in.
///
/// Why the limiter: every set is made so one full note peaks just under full scale, so two
/// notes together pass it and the output clipped (a limiter on the master is a common default
/// for the same reason). Why here and not in [`Project::new`]: that is the engine's
/// empty document, which tests build on with effect ids of their own; and a file opened keeps
/// the master it was saved with.
pub fn new_document() -> Project {
    let mut project = Project::new(FALLBACK_RATE, MilliBpm(120_000), 96);
    if let Some(master) = project.mixer.get_mut(InsertId::MASTER) {
        master.effects.push(Effect {
            id: EffectId(1),
            slot: 0,
            processor: ProcessorRef::new(limiter::ID),
        });
        project.mixer.next_effect_id = EffectId(2);
    }
    let pattern = Pattern {
        name: "Pattern 1".to_owned(),
        length_bars: 1,
        copied_from: None,
        notes: Vec::new(),
    };
    project.patterns.insert(PatternId(1), pattern);
    project
        .channels
        .insert(ChannelId(1), Channel::sampler(EMPTY_SAMPLER, "").as_added());
    project.next_channel_id = ChannelId(2);
    project
}

/// The starter kit's sounds a starter document's rows play, top to bottom: a beat's four
/// parts, as a beginner reaches for them.
pub const STARTER_ROWS: [&str; 4] = ["Kick 909", "Snare", "Hat closed", "Clap"];

/// The document a window opened with no file starts with, and File → New, where the starter
/// kit can be written ([`first_document`]): [`new_document`]'s, with four sampler channels
/// playing the starter kit's kick, snare, closed hat and clap by their shipped references
/// (`dawsome:starter-kit/Kick 909.wav`) in place of the silent "Sampler", their steps empty,
/// each at the volume an add gives it. Why: the first press on a step then makes a sound.
pub fn starter_document() -> Project {
    let mut project = new_document();
    project.channels.clear();
    for (index, name) in (1..).zip(STARTER_ROWS) {
        let sample = format!(
            "{}starter-kit/{name}.wav",
            dawsome_media::paths::SHIPPED_PREFIX
        );
        project
            .channels
            .insert(ChannelId(index), Channel::sampler(name, sample).as_added());
    }
    project.next_channel_id = ChannelId(STARTER_ROWS.len() as u32 + 1);
    project
}

/// The document a new window or File → New starts with: the [`starter_document`] when there
/// is a folder to write the starter kit into (`kit_dir`), else the [`new_document`]. Why the
/// empty one without a folder (a scripted run's): its four rows would name sounds that are
/// nowhere, and open with four notices about them.
pub fn first_document(kit_dir: Option<&Path>) -> Project {
    match kit_dir {
        Some(_) => starter_document(),
        None => new_document(),
    }
}

/// The text `project` saves as at `path`: its canonical JSON with its sample paths, relative to
/// `samples_dir`, rewritten so they name the same files from `path`'s directory (unchanged when
/// that is the same one), and a final newline, as text files end.
pub fn json_for(project: &Project, samples_dir: &Path, path: &Path) -> Result<String, SaveError> {
    let mut saved = project.clone();
    dawsome_media::rebase_samples(&mut saved, samples_dir, &self::samples_dir(path));
    let mut text = saved.to_json().map_err(|source| SaveError::Serialize {
        path: path.to_owned(),
        source,
    })?;
    text.push('\n');
    Ok(text)
}

/// Writes `bytes` to `path` so that the file is either the old one or the new one, never half
/// of each: into a temporary file beside it, flushed to disk, then renamed over it. Each write
/// has a temporary file of its own (`<name>.<process>-<count>.tmp`), so two writers of one file
/// at once (two windows writing the starter kit) never rename each other's away; the last
/// rename wins, and either way the file is whole.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), SaveError> {
    static WRITES: AtomicU64 = AtomicU64::new(0);
    let failed = |source| SaveError::Write {
        path: path.to_owned(),
        source,
    };
    let count = WRITES.fetch_add(1, Ordering::Relaxed);
    let mut name = path.file_name().map(OsString::from).unwrap_or_default();
    name.push(format!(".{}-{count}.tmp", std::process::id()));
    let temporary = path.with_file_name(name);
    let written = File::create(&temporary).and_then(|mut file| {
        file.write_all(bytes)?;
        file.sync_all()
    });
    if let Err(error) = written.and_then(|()| fs::rename(&temporary, path)) {
        // The old file is untouched; only the half-written copy goes.
        let _ = fs::remove_file(&temporary);
        return Err(failed(error));
    }
    Ok(())
}

/// The extension Save and Save As give a project: `song.dawsome`. The file holds the same
/// JSON a `.json` project does; the extension only names the type, so a file manager opens it
/// with the app (`assets/dawsome-mime.xml`, `application/x-dawsome-project`).
pub const EXTENSION: &str = "dawsome";

/// The extensions a Dawsome project opens from: its own, and `.json`, which every project had
/// before it (the known-answer fixtures, the demo and the files saved until then keep it).
pub const PROJECT_EXTENSIONS: [&str; 2] = [EXTENSION, "json"];

/// Whether `path` ends in one of the [`PROJECT_EXTENSIONS`], in any case.
fn has_project_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            PROJECT_EXTENSIONS
                .iter()
                .any(|known| known.eq_ignore_ascii_case(extension))
        })
}

/// Where a project the Save dialog was given `picked` for is written: `picked` itself when it
/// ends in `.dawsome` or `.json`, else `picked` with `.dawsome` added, so "song" saves as
/// "song.dawsome" and "Song v1.2" as "Song v1.2.dawsome" (its ".2" is no extension of ours).
pub fn project_path(picked: &Path) -> PathBuf {
    if has_project_extension(picked) {
        return picked.to_path_buf();
    }
    let mut name = picked.as_os_str().to_owned();
    name.push(".");
    name.push(EXTENSION);
    PathBuf::from(name)
}

/// The autosave of the document saved at `path`, beside it: `<stem>.autosave.dawsome` for a
/// `.dawsome` file, `<stem>.autosave.json` for any other (as every autosave was named before
/// the extension). Why the file's own extension: "song.json" and "song.dawsome" in one folder
/// (a `.json` project saved again under the new extension) would otherwise share one autosave,
/// and opening either would offer the other's changes.
pub fn autosave_beside(path: &Path) -> PathBuf {
    let own = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(EXTENSION));
    let stem = path.file_stem().map(OsString::from).unwrap_or_default();
    let mut name = stem;
    name.push(if own {
        ".autosave.dawsome"
    } else {
        ".autosave.json"
    });
    path.with_file_name(name)
}

/// Where per-user files of one kind go: `own`, the value of Dawsome's own variable for that
/// kind, when it is an absolute path, else the platform's folder for that kind. A relative value
/// is ignored, since it would name another folder from every working directory. Why a variable
/// of our own: scripted runs can move the folders the same way on every platform, while on Linux
/// the XDG variables keep working through the platform's folders. The variable's value is passed
/// in rather than read here, so the rule is tested without changing the tests' environment.
fn user_dir(own: Option<OsString>, platform: impl FnOnce() -> Option<PathBuf>) -> Option<PathBuf> {
    own.map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(platform)
}

/// Where per-user settings go: `$DAWSOME_CONFIG_HOME`, else `$XDG_CONFIG_HOME` or `~/.config` on
/// Linux, `%APPDATA%` on Windows.
pub fn config_dir() -> Option<PathBuf> {
    user_dir(env::var_os("DAWSOME_CONFIG_HOME"), dirs::config_dir)
}

/// Where per-user data goes: `$DAWSOME_DATA_HOME`, else `$XDG_DATA_HOME` or `~/.local/share` on
/// Linux, `%LOCALAPPDATA%` on Windows (recordings are large, and stay on this machine rather
/// than roam with the user's profile). Why the media crate's: a project's shipped sounds are
/// read from there by the same rule, so the folder the Library puts them in is the one renders
/// find them in.
fn data_dir() -> Option<PathBuf> {
    dawsome_media::paths::data_dir()
}

/// Where per-user state goes: `$DAWSOME_STATE_HOME`, else `$XDG_STATE_HOME` or `~/.local/state`
/// on Linux, `%LOCALAPPDATA%` on Windows, which has no folder for state of its own.
fn state_dir() -> Option<PathBuf> {
    user_dir(env::var_os("DAWSOME_STATE_HOME"), || {
        dirs::state_dir().or_else(dirs::data_local_dir)
    })
}

/// `dawsome/crashes` in the state folder: where a panic's report goes ([`crate::crash`]).
pub fn crash_dir() -> Option<PathBuf> {
    state_dir().map(|dir| dir.join("dawsome/crashes"))
}

/// `dawsome/recent.json` in the settings folder.
pub fn recent_file() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("dawsome/recent.json"))
}

/// `dawsome/browser.json` in the settings folder: the folders added to the sample browser.
pub fn browser_file() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("dawsome/browser.json"))
}

/// `dawsome/recordings` in the data folder: where a document with no file of its own records
/// to.
pub fn recordings_dir() -> Option<PathBuf> {
    data_dir().map(|dir| dir.join("dawsome/recordings"))
}

/// `dawsome/demo` in the data folder: where the demo project is put the first time it is
/// needed.
pub fn demo_dir() -> Option<PathBuf> {
    data_dir().map(|dir| dir.join("dawsome/demo"))
}

/// `dawsome/starter-kit` in the data folder: where the starter kit's sounds are put the first
/// time one is previewed or dragged.
pub fn starter_kit_dir() -> Option<PathBuf> {
    data_dir().map(|dir| dir.join("dawsome/starter-kit"))
}

/// `dawsome/instruments` in the data folder: where the shipped instrument sets are, each a
/// folder of its own.
pub fn instruments_dir() -> Option<PathBuf> {
    data_dir().map(|dir| dir.join("dawsome/instruments"))
}

/// `dawsome/autosave/untitled.autosave.json` in the state folder: where documents with no file
/// of their own (new ones) autosave.
pub fn untitled_autosave() -> Option<PathBuf> {
    state_dir().map(|dir| dir.join("dawsome/autosave/untitled.autosave.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    fn temp_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("dawsome-app-files-{}", std::process::id()))
            .join(test);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn opens_f1() {
        let opened = open(&Path::new(FIXTURES).join("f1_four_on_floor.json")).expect("F1 opens");
        let names: Vec<&str> = opened
            .project
            .channels
            .values()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(names, ["kick"]);
        assert_eq!(opened.project.patterns.len(), 1);
    }

    #[test]
    fn a_missing_file_is_a_read_error() {
        let result = open(Path::new("/nonexistent/project.json"));
        assert!(matches!(result, Err(OpenError::Read { .. })), "{result:?}");
    }

    #[test]
    fn a_file_that_is_not_a_project_is_invalid() {
        // Cargo.toml exists but is not JSON.
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
        let result = open(Path::new(path));
        assert!(
            matches!(result, Err(OpenError::Invalid { .. })),
            "{result:?}"
        );
    }

    #[test]
    fn the_new_document_is_valid() {
        assert_eq!(new_document().validate(), Ok(()));
    }

    // Known answer (gen_p24_expected.py): a new document, empty or the starter one, saves its
    // master with the Limiter at its defaults in slot 0 as effect 1, the next effect 2; a saved
    // file with no limiter opens with none (an old project is never changed on open).
    #[test]
    fn p24_a_new_document_has_the_limiter_on_its_master() {
        let text = fs::read_to_string(Path::new(FIXTURES).join("p24_expected.json"))
            .expect("p24_expected.json");
        let want: serde_json::Value = serde_json::from_str(&text).expect("parses");
        let want = &want["new_document_master"];
        for project in [new_document(), starter_document()] {
            let saved: serde_json::Value =
                serde_json::from_str(&project.to_json().expect("saves")).expect("parses");
            assert_eq!(saved["mixer"]["inserts"][0], want["insert"]);
            assert_eq!(saved["mixer"]["next_effect_id"], want["next_effect_id"]);
        }
        let old = open(&Path::new(FIXTURES).join("p24_under_plain.json")).expect("opens");
        assert!(old.project.mixer.inserts[0].effects.is_empty());
        assert_eq!(old.project.mixer.next_effect_id.0, 1);
    }

    // The starter document as its fixture holds it (the one ui-check opens), written by hand:
    // the new document's pattern, tracks and inserts, and the kit's kick, snare, closed hat and
    // clap as four sampler channels by their shipped references, no step set.
    #[test]
    fn the_starter_document_holds_four_kit_rows_and_no_note() {
        let starter = starter_document();
        assert_eq!(starter.validate(), Ok(()));
        let fixture =
            fs::read_to_string(Path::new(FIXTURES).join("a6_starter.json")).expect("the fixture");
        assert_eq!(Project::load_json(&fixture).expect("loads"), starter);
        let names: Vec<&str> = starter.channels.values().map(|c| c.name.as_str()).collect();
        assert_eq!(names, STARTER_ROWS);
        assert!(starter
            .patterns
            .values()
            .all(|pattern| pattern.notes.is_empty()));
        assert_eq!(first_document(None), new_document());
        assert_eq!(first_document(Some(Path::new("kit"))), starter);
    }

    #[test]
    fn an_atomic_write_replaces_the_file_or_leaves_it() {
        let dir = temp_dir("atomic");
        let path = dir.join("song.json");
        write_atomically(&path, b"first").expect("writes");
        write_atomically(&path, b"second").expect("replaces");
        assert_eq!(fs::read(&path).expect("reads"), b"second");
        assert!(!dir.join("song.json.tmp").exists(), "no temporary left");
        let missing = dir.join("no/such/dir/song.json");
        assert!(matches!(
            write_atomically(&missing, b"x"),
            Err(SaveError::Write { .. })
        ));
    }

    // Each write's temporary file is its own and goes once renamed: after writes from two
    // threads at once the folder holds the file alone, whole.
    #[test]
    fn atomic_writes_at_once_leave_no_temporary() {
        let dir = temp_dir("atomic_at_once");
        let path = dir.join("song.json");
        std::thread::scope(|scope| {
            for text in [&b"first"[..], b"second"] {
                let path = &path;
                scope.spawn(move || {
                    for _ in 0..20 {
                        write_atomically(path, text).expect("writes");
                    }
                });
            }
        });
        let names: Vec<_> = fs::read_dir(&dir)
            .expect("lists")
            .map(|entry| entry.expect("an entry").file_name())
            .collect();
        assert_eq!(names, ["song.json"]);
        let text = fs::read(&path).expect("reads");
        assert!(text == b"first" || text == b"second", "{text:?}");
    }

    // A read-only directory refuses the temporary file, so the old file stays as it was. Linux
    // only: a Windows folder has no read-only mode bits to refuse new files with.
    #[cfg(unix)]
    #[test]
    fn a_failed_save_keeps_the_old_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("read_only");
        let path = dir.join("song.json");
        fs::write(&path, b"old").expect("writes");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).expect("chmod");
        let result = write_atomically(&path, b"new");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).expect("chmod back");
        assert!(result.is_err(), "the directory takes no new file");
        assert_eq!(fs::read(&path).expect("reads"), b"old");
    }

    // The rule every per-user folder follows: Dawsome's own variable, when it holds an absolute
    // path, wins over the platform's folder without the platform being asked; a relative or
    // empty value is ignored; unset, the platform's folder is used, and without one there is
    // none. The temp dir is only a name that is absolute on every platform; nothing is written.
    #[test]
    fn an_absolute_own_folder_wins_and_a_relative_one_falls_back() {
        let ours = env::temp_dir().join("ours");
        let platform = env::temp_dir().join("platform");
        let the_platform = || Some(platform.clone());
        let unasked = || -> Option<PathBuf> { panic!("the platform is not asked") };
        assert_eq!(user_dir(Some(ours.clone().into()), unasked), Some(ours));
        assert_eq!(
            user_dir(Some("relative/dawsome".into()), the_platform),
            Some(platform.clone())
        );
        assert_eq!(
            user_dir(Some("".into()), the_platform),
            Some(platform.clone())
        );
        assert_eq!(user_dir(None, the_platform), Some(platform.clone()));
        assert_eq!(user_dir(Some("relative".into()), || None), None);
        assert_eq!(user_dir(None, || None), None);
    }

    // A `.json` project keeps the autosave name every project had; a `.dawsome` one takes its
    // own, so the two files of one song in one folder never share an autosave.
    #[test]
    fn autosaves_sit_beside_their_file() {
        assert_eq!(
            autosave_beside(Path::new("/music/song.json")),
            Path::new("/music/song.autosave.json")
        );
        assert_eq!(
            autosave_beside(Path::new("/music/song.dawsome")),
            Path::new("/music/song.autosave.dawsome")
        );
        assert_eq!(
            autosave_beside(Path::new("/music/Song.DAWSOME")),
            Path::new("/music/Song.autosave.dawsome")
        );
    }

    // The Save dialog's answer, written by hand: a name with no extension of ours gets
    // `.dawsome` added (never one replaced), `.dawsome` and `.json` in any case stay.
    #[test]
    fn a_saved_project_takes_the_dawsome_extension_unless_it_has_ours() {
        let cases = [
            ("/music/song", "/music/song.dawsome"),
            ("/music/song.dawsome", "/music/song.dawsome"),
            ("/music/song.json", "/music/song.json"),
            ("/music/Song.JSON", "/music/Song.JSON"),
            ("/music/Song v1.2", "/music/Song v1.2.dawsome"),
            ("/music/song.txt", "/music/song.txt.dawsome"),
        ];
        for (picked, written) in cases {
            assert_eq!(
                project_path(Path::new(picked)),
                Path::new(written),
                "{picked}"
            );
        }
    }

    // The same document under either extension: open decides by content, so a `.dawsome` copy
    // of F1 opens as F1 does.
    #[test]
    fn a_dawsome_file_opens_as_its_json_twin() {
        let dir = temp_dir("dawsome_extension");
        let json = Path::new(FIXTURES).join("f1_four_on_floor.json");
        fs::copy(&json, dir.join("f1.dawsome")).expect("copy");
        let twin = open(&dir.join("f1.dawsome")).expect("opens");
        assert_eq!(twin.project, open(&json).expect("F1 opens").project);
    }
}
