//! The sample browser's state, without iced: the folders the Library's Samples section shows
//! (the project's folder, Music and the folders added), which are open and what each holds, the
//! search over everything below them, the selection and the file being previewed. Listings and
//! the search index are made by plain functions the app runs off the UI thread.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::project_file::{self, SaveError};

/// The sound files the browser lists, by extension (any case): what decoding reads.
pub const EXTENSIONS: [&str; 4] = ["wav", "flac", "ogg", "mp3"];

/// The Library's root for the project's recordings folder.
pub const RECORDINGS: &str = "Recordings";

/// How deep below each root the search index goes, and how many files it holds at most.
pub const SEARCH_DEPTH: usize = 8;
pub const SEARCH_LIMIT: usize = 50_000;

/// Where a root comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootKind {
    /// The open project's folder.
    Project,
    /// Where the project's takes go (`samples/` beside it, or the per-user recordings folder
    /// for a document with no file), once it exists: "Recordings", after the project's folder.
    Recordings,
    /// The user's Music folder.
    Music,
    /// A folder of sounds shipped with the program; it cannot be removed.
    Shipped,
    /// A folder the user added; it can be removed again.
    User,
    /// The one folder `--browser-root` names.
    Given,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root {
    pub name: String,
    pub path: PathBuf,
    pub kind: RootKind,
}

/// A folder of sounds shipped with the program (a set folder whose `set.json` says
/// `"kind": "samples"`): what the Samples section lists it as, and where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShippedFolder {
    pub name: String,
    pub path: PathBuf,
}

impl Root {
    fn new(path: PathBuf, kind: RootKind) -> Self {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        Self { name, path, kind }
    }
}

/// One thing in a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub folder: bool,
}

/// Whether a file name is one of the sound files the browser lists.
pub fn is_sound(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            EXTENSIONS
                .iter()
                .any(|known| known.eq_ignore_ascii_case(extension))
        })
}

/// What `dir` holds as the browser lists it: folders first, then sound files, each sorted by
/// name ignoring case; hidden entries (a leading dot) and links to folders left out.
pub fn list(dir: &Path) -> io::Result<Vec<Entry>> {
    let mut folders = Vec::new();
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)? {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            folders.push(Entry {
                name,
                path,
                folder: true,
            });
        } else if kind.is_symlink() {
            // A link to a folder could loop back up; a link to a sound file is fine.
            if path.is_file() && is_sound(&name) {
                files.push(Entry {
                    name,
                    path,
                    folder: false,
                });
            }
        } else if is_sound(&name) {
            files.push(Entry {
                name,
                path,
                folder: false,
            });
        }
    }
    let by_name = |entry: &Entry| entry.name.to_lowercase();
    folders.sort_by_key(by_name);
    files.sort_by_key(by_name);
    folders.extend(files);
    Ok(folders)
}

/// Every sound file below `roots`, at most [`SEARCH_DEPTH`] folders down and [`SEARCH_LIMIT`]
/// files in all; and whether the limit cut it short.
pub fn search_index(roots: &[PathBuf]) -> (Vec<Entry>, bool) {
    let mut found = Vec::new();
    let mut stack: Vec<(PathBuf, usize)> =
        roots.iter().rev().map(|root| (root.clone(), 0)).collect();
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = list(&dir) else { continue };
        let mut below = Vec::new();
        for entry in entries {
            if entry.folder {
                if depth < SEARCH_DEPTH {
                    below.push((entry.path, depth + 1));
                }
            } else {
                if found.len() == SEARCH_LIMIT {
                    return (found, true);
                }
                found.push(entry);
            }
        }
        stack.extend(below.into_iter().rev());
    }
    (found, false)
}

/// The entries of `index` whose names hold `query`, ignoring case.
pub fn matches<'a>(index: &'a [Entry], query: &str) -> Vec<&'a Entry> {
    let query = query.to_lowercase();
    index
        .iter()
        .filter(|entry| entry.name.to_lowercase().contains(&query))
        .collect()
}

/// The folders the user added, as `browser.json` keeps them: `{"folders":[...]}`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Folders {
    pub folders: Vec<PathBuf>,
}

impl Folders {
    /// The folders in `file`; none when it is missing or not that shape.
    pub fn load(file: Option<&Path>) -> Self {
        let folders = file
            .and_then(|file| fs::read_to_string(file).ok())
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|mut value| value.get_mut("folders").map(serde_json::Value::take))
            .and_then(|folders| serde_json::from_value::<Vec<PathBuf>>(folders).ok())
            .unwrap_or_default();
        Self { folders }
    }

    /// Writes them to `file`, atomically.
    pub fn save(&self, file: &Path) -> Result<(), SaveError> {
        let text = serde_json::to_string(&serde_json::json!({ "folders": self.folders })).map_err(
            |source| SaveError::Serialize {
                path: file.to_path_buf(),
                source,
            },
        )?;
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir).map_err(|source| SaveError::Write {
                path: file.to_path_buf(),
                source,
            })?;
        }
        project_file::write_atomically(file, format!("{text}\n").as_bytes())
    }
}

/// The user's Music folder: on Linux `XDG_MUSIC_DIR` in `$XDG_CONFIG_HOME/user-dirs.dirs`, on
/// Windows the Music known folder; else `~/Music`.
pub fn music_dir() -> Option<PathBuf> {
    dirs::audio_dir().or_else(|| dirs::home_dir().map(|home| home.join("Music")))
}

/// What a row of the browser shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// A root; a folder of the user's can be removed.
    Root {
        user: bool,
        open: bool,
    },
    Folder {
        open: bool,
    },
    File,
    /// "+ Add folder…" under the roots.
    AddFolder,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Levels below a root.
    pub depth: usize,
    pub name: String,
    pub path: PathBuf,
    pub kind: RowKind,
    /// For a search result: the folder it is in, shown dimmed.
    pub parent: Option<String>,
}

/// The browser's state.
#[derive(Debug, Clone, Default)]
pub struct Browser {
    pub roots: Vec<Root>,
    /// Folders open, roots included.
    pub open: BTreeSet<PathBuf>,
    /// What each listed folder holds.
    pub listings: BTreeMap<PathBuf, Vec<Entry>>,
    /// The selected row.
    pub selected: Option<usize>,
    /// The file being previewed.
    pub previewing: Option<PathBuf>,
    /// The instrument row the Library previews: its keys and the devices no link routes play it
    /// until a rack channel is chosen, an instrument added or another row pressed.
    pub playing: Option<Playing>,
    /// What the search box holds; empty shows the tree.
    pub search: String,
    /// The search index, once built, and whether its limit cut it short.
    pub index: Option<(Vec<Entry>, bool)>,
    /// The user's folders can be added and removed (not with `--browser-root`).
    pub editable: bool,
}

/// An instrument row of the Library by what it plays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Playing {
    /// The starter kit's sound at this index.
    KitSound(usize),
    /// An instrument set, by its main file.
    Set(PathBuf),
    /// A plugin instrument, by its processor id (`"<format>:<id>"`).
    Plugin(String),
}

impl Browser {
    /// A browser of the project folder (when there is one), Music and `added`.
    pub fn new(project: Option<PathBuf>, music: Option<PathBuf>, added: &[PathBuf]) -> Self {
        let mut roots: Vec<Root> = project
            .into_iter()
            .map(|path| Root::new(path, RootKind::Project))
            .chain(
                music
                    .into_iter()
                    .map(|path| Root::new(path, RootKind::Music)),
            )
            .collect();
        for path in added {
            if !roots.iter().any(|root| root.path == *path) {
                roots.push(Root::new(path.clone(), RootKind::User));
            }
        }
        Self {
            roots,
            editable: true,
            ..Self::default()
        }
    }

    /// A browser of `root` alone, open (`--browser-root`).
    pub fn given(root: PathBuf) -> Self {
        let root = std::path::absolute(&root).unwrap_or(root);
        Self {
            open: BTreeSet::from([root.clone()]),
            roots: vec![Root::new(root, RootKind::Given)],
            editable: false,
            ..Self::default()
        }
    }

    /// The user's folders, to save.
    pub fn user_folders(&self) -> Folders {
        Folders {
            folders: self
                .roots
                .iter()
                .filter(|root| root.kind == RootKind::User)
                .map(|root| root.path.clone())
                .collect(),
        }
    }

    /// Makes `dir` the project folder's root, first; none for an untitled document. Returns
    /// whether the roots changed.
    pub fn set_project(&mut self, dir: Option<PathBuf>) -> bool {
        let current = self
            .roots
            .iter()
            .find(|root| root.kind == RootKind::Project);
        if current.map(|root| &root.path) == dir.as_ref() {
            return false;
        }
        self.roots.retain(|root| root.kind != RootKind::Project);
        if let Some(dir) = dir {
            self.roots.insert(0, Root::new(dir, RootKind::Project));
        }
        self.index = None;
        self.selected = None;
        true
    }

    /// Makes `dir` the Recordings root, after the project folder's; none while there is no
    /// recordings folder. Returns whether the roots changed.
    pub fn set_recordings(&mut self, dir: Option<PathBuf>) -> bool {
        let current = self
            .roots
            .iter()
            .find(|root| root.kind == RootKind::Recordings);
        if current.map(|root| &root.path) == dir.as_ref() {
            return false;
        }
        self.roots.retain(|root| root.kind != RootKind::Recordings);
        if let Some(dir) = dir {
            let at = usize::from(
                self.roots
                    .first()
                    .is_some_and(|root| root.kind == RootKind::Project),
            );
            let root = Root {
                name: RECORDINGS.to_owned(),
                path: dir,
                kind: RootKind::Recordings,
            };
            self.roots.insert(at, root);
        }
        self.index = None;
        self.selected = None;
        true
    }

    /// Makes `folders` the shipped folders of sounds the tree lists, after Music and before the
    /// user's folders; a browser of one given root lists none. Returns whether the roots
    /// changed.
    pub fn set_shipped(&mut self, folders: &[ShippedFolder]) -> bool {
        if !self.editable {
            return false;
        }
        let listed: Vec<ShippedFolder> = self
            .roots
            .iter()
            .filter(|root| root.kind == RootKind::Shipped)
            .map(|root| ShippedFolder {
                name: root.name.clone(),
                path: root.path.clone(),
            })
            .collect();
        if listed == folders {
            return false;
        }
        self.roots.retain(|root| root.kind != RootKind::Shipped);
        let at = self
            .roots
            .iter()
            .position(|root| root.kind == RootKind::User)
            .unwrap_or(self.roots.len());
        let shipped = folders.iter().map(|folder| Root {
            name: folder.name.clone(),
            path: folder.path.clone(),
            kind: RootKind::Shipped,
        });
        self.roots.splice(at..at, shipped);
        self.index = None;
        self.selected = None;
        true
    }

    /// Adds a folder of the user's; false when it is a root already.
    pub fn add_folder(&mut self, path: PathBuf) -> bool {
        if self.roots.iter().any(|root| root.path == path) {
            return false;
        }
        self.roots.push(Root::new(path, RootKind::User));
        self.index = None;
        true
    }

    /// Takes a folder of the user's away again.
    pub fn remove_folder(&mut self, path: &Path) {
        self.roots
            .retain(|root| !(root.kind == RootKind::User && root.path == path));
        self.index = None;
        self.selected = None;
    }

    /// The rows shown, top to bottom: the open tree with "+ Add folder…" under it, or, while
    /// the search box holds text, the files that match.
    pub fn rows(&self) -> Vec<Row> {
        if !self.search.is_empty() {
            let Some((index, _)) = &self.index else {
                return Vec::new();
            };
            return matches(index, &self.search)
                .into_iter()
                .map(|entry| Row {
                    depth: 0,
                    name: entry.name.clone(),
                    path: entry.path.clone(),
                    kind: RowKind::File,
                    parent: entry
                        .path
                        .parent()
                        .map(|parent| parent.display().to_string()),
                })
                .collect();
        }
        let mut rows = Vec::new();
        for root in &self.roots {
            rows.push(Row {
                depth: 0,
                name: root.name.clone(),
                path: root.path.clone(),
                kind: RowKind::Root {
                    user: root.kind == RootKind::User,
                    open: self.open.contains(&root.path),
                },
                parent: None,
            });
            if self.open.contains(&root.path) {
                self.push_open(&root.path, 1, &mut rows);
            }
        }
        if self.editable {
            rows.push(Row {
                depth: 0,
                name: "+ Add folder\u{2026}".to_owned(),
                path: PathBuf::new(),
                kind: RowKind::AddFolder,
                parent: None,
            });
        }
        rows
    }

    fn push_open(&self, dir: &Path, depth: usize, rows: &mut Vec<Row>) {
        for entry in self.listings.get(dir).into_iter().flatten() {
            let open = entry.folder && self.open.contains(&entry.path);
            rows.push(Row {
                depth,
                name: entry.name.clone(),
                path: entry.path.clone(),
                kind: if entry.folder {
                    RowKind::Folder { open }
                } else {
                    RowKind::File
                },
                parent: None,
            });
            if open {
                self.push_open(&entry.path, depth + 1, rows);
            }
        }
    }

    /// Opens or closes the folder at `path`; returns it when it opens and is not listed yet,
    /// for the app to list.
    pub fn toggle(&mut self, path: &Path) -> Option<PathBuf> {
        if self.open.remove(path) {
            return None;
        }
        self.open.insert(path.to_path_buf());
        (!self.listings.contains_key(path)).then(|| path.to_path_buf())
    }

    /// The open folders not listed yet.
    pub fn unlisted(&self) -> Vec<PathBuf> {
        self.open
            .iter()
            .filter(|path| !self.listings.contains_key(*path))
            .cloned()
            .collect()
    }

    /// What a folder holds, as listed.
    pub fn listed(&mut self, dir: PathBuf, entries: Vec<Entry>) {
        self.listings.insert(dir, entries);
    }

    /// Moves the selection by `rows` (up is negative), within the rows shown.
    pub fn move_selection(&mut self, rows: i64) {
        let count = self.rows().len();
        if count == 0 {
            self.selected = None;
            return;
        }
        let at = self.selected.map_or(0, |at| {
            (at as i64 + rows).clamp(0, count as i64 - 1) as usize
        });
        self.selected = Some(at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/browser")
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("dawsome-browser-{}", std::process::id()))
            .join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    // The tree p8_ui's screenshot shows: "browser" open, holding the folder "drums" then
    // "tone.wav"; notes.txt is not a sound file.
    #[test]
    fn a_given_root_lists_folders_first_and_sound_files_only() {
        let mut browser = Browser::given(fixtures());
        for dir in browser.unlisted() {
            let entries = list(&dir).expect("lists");
            browser.listed(dir, entries);
        }
        let names: Vec<String> = browser.rows().into_iter().map(|row| row.name).collect();
        assert_eq!(names, ["browser", "drums", "tone.wav"]);
    }

    // Recording v2: the recordings folder is a root of its own, "Recordings", right after the
    // project's folder (first without one); set again to the same folder nothing changes, and
    // none takes it away.
    #[test]
    fn recordings_root_follows_the_project_folder() {
        let mut browser = Browser::new(
            Some(PathBuf::from("/songs")),
            Some(PathBuf::from("/music")),
            &[],
        );
        assert!(browser.set_recordings(Some(PathBuf::from("/songs/samples"))));
        let names: Vec<(&str, RootKind)> = browser
            .roots
            .iter()
            .map(|root| (root.name.as_str(), root.kind))
            .collect();
        assert_eq!(
            names,
            [
                ("songs", RootKind::Project),
                (RECORDINGS, RootKind::Recordings),
                ("music", RootKind::Music)
            ]
        );
        assert!(!browser.set_recordings(Some(PathBuf::from("/songs/samples"))));
        assert!(browser.set_recordings(None));
        assert_eq!(browser.roots.len(), 2);
        let mut untitled = Browser::new(None, Some(PathBuf::from("/music")), &[]);
        assert!(untitled.set_recordings(Some(PathBuf::from("/data/recordings"))));
        assert_eq!(untitled.roots[0].kind, RootKind::Recordings);
    }

    #[test]
    fn listing_sorts_ignoring_case_and_skips_hidden_and_folder_links() {
        let dir = temp("sorting");
        for name in ["b.WAV", "A.flac", ".hidden.wav", "c.mp3", "d.txt", "e.ogg"] {
            fs::write(dir.join(name), b"").expect("writes");
        }
        fs::create_dir(dir.join("Zeta")).expect("dir");
        fs::create_dir(dir.join("alpha")).expect("dir");
        fs::create_dir(dir.join(".git")).expect("dir");
        // Links need no privilege on Linux only; on Windows the rest still holds without one.
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.join("alpha"), dir.join("loop")).expect("link");
        let names: Vec<String> = list(&dir)
            .expect("lists")
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(
            names,
            ["alpha", "Zeta", "A.flac", "b.WAV", "c.mp3", "e.ogg"]
        );
    }

    // The search walks the folders below the roots, ignoring case; past the limit it stops
    // and says so.
    #[test]
    fn the_search_finds_files_below_the_roots() {
        let (index, cut) = search_index(&[fixtures()]);
        assert!(!cut);
        let names: Vec<&str> = matches(&index, "KICK")
            .iter()
            .map(|e| e.name.as_str())
            .collect();
        assert_eq!(names, ["kick.wav"]);
        assert_eq!(matches(&index, ".wav").len(), 2);
        let mut browser = Browser::given(fixtures());
        browser.index = Some((index, cut));
        browser.search = "tone".into();
        let rows = browser.rows();
        assert_eq!(rows.len(), 1);
        assert!(rows[0]
            .parent
            .as_deref()
            .is_some_and(|parent| parent.ends_with("browser")));
    }

    #[test]
    fn the_added_folders_round_trip_through_their_file() {
        let dir = temp("config");
        let file = dir.join("dawsome/browser.json");
        let folders = Folders {
            folders: vec![PathBuf::from("/music/kits"), PathBuf::from("/music/loops")],
        };
        folders.save(&file).expect("saves");
        assert_eq!(
            fs::read_to_string(&file).expect("reads"),
            "{\"folders\":[\"/music/kits\",\"/music/loops\"]}\n"
        );
        assert_eq!(Folders::load(Some(&file)), folders);
        assert_eq!(
            Folders::load(Some(&dir.join("none.json"))),
            Folders::default()
        );
        let mut browser = Browser::new(None, None, &folders.folders);
        assert!(!browser.add_folder(PathBuf::from("/music/kits")), "once");
        browser.remove_folder(Path::new("/music/kits"));
        assert_eq!(
            browser.user_folders().folders,
            [PathBuf::from("/music/loops")]
        );
    }

    // The shipped folders of sounds sit after Music and before the user's folders, named as
    // given, and cannot be removed; given again unchanged they change nothing, and a browser of
    // one given root lists none.
    #[test]
    fn shipped_folders_sit_before_the_users_and_cannot_be_removed() {
        let mut browser = Browser::new(
            Some(PathBuf::from("/songs")),
            Some(PathBuf::from("/music")),
            &[PathBuf::from("/kits")],
        );
        let loops = ShippedFolder {
            name: "One-shots and Loops".to_owned(),
            path: PathBuf::from("/usr/share/dawsome/instruments/one-shots/samples"),
        };
        assert!(browser.set_shipped(std::slice::from_ref(&loops)));
        assert!(
            !browser.set_shipped(std::slice::from_ref(&loops)),
            "unchanged"
        );
        let rows: Vec<(String, RowKind)> = browser
            .rows()
            .into_iter()
            .map(|row| (row.name, row.kind))
            .collect();
        let root = |user| RowKind::Root { user, open: false };
        assert_eq!(
            rows,
            [
                ("songs".to_owned(), root(false)),
                ("music".to_owned(), root(false)),
                ("One-shots and Loops".to_owned(), root(false)),
                ("kits".to_owned(), root(true)),
                ("+ Add folder\u{2026}".to_owned(), RowKind::AddFolder),
            ]
        );
        browser.remove_folder(&loops.path);
        assert_eq!(browser.roots.len(), 4, "a shipped folder stays");
        assert_eq!(browser.user_folders().folders, [PathBuf::from("/kits")]);
        assert!(browser.set_shipped(&[]));
        assert_eq!(browser.roots.len(), 3);
        let mut given = Browser::given(fixtures());
        assert!(!given.set_shipped(&[loops]));
        assert_eq!(given.roots.len(), 1);
    }

    #[test]
    fn the_selection_moves_within_the_rows() {
        let mut browser = Browser::given(fixtures());
        let dir = browser.roots[0].path.clone();
        browser.listed(dir.clone(), list(&dir).expect("lists"));
        browser.move_selection(1);
        assert_eq!(browser.selected, Some(0));
        browser.move_selection(5);
        assert_eq!(browser.selected, Some(2));
        browser.move_selection(-1);
        assert_eq!(browser.selected, Some(1));
        assert_eq!(browser.toggle(&dir), None, "closing needs no listing");
        assert_eq!(browser.rows().len(), 1);
    }
}
