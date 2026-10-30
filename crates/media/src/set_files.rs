//! An instrument set's own files: its main .sfz, the files its `#include`s name and the sounds
//! its regions play. Each must lie inside the set's folder (its main file's), written without
//! climbing out of it, and be a regular file; a text is read up to [`MAX_FILE_BYTES`]. A set
//! inside a folder the user added to the Library reads inside that folder instead, at most
//! [`MAX_CLIMB`] folders above its own ([`SetFolder::inside`], [`library_root`]); the home
//! folder, a folder holding it and a filesystem's root widen nothing.
//!
//! Why: a set comes from anywhere (a download, a folder the user added) and its text decides
//! which files are opened. Unchecked, a set read files elsewhere on the machine into the report
//! the app shows, read `/dev/zero` until memory ran out, and waited for ever on a named pipe,
//! on the UI thread (found 2026-10-22). Why a folder added is
//! the limit for the sets in it: libraries are commonly laid out `Instruments/x.sfz` beside
//! `Samples/`, their sets reaching `../Samples/`, and the user chose that whole folder to be
//! read by adding it. Why not any folder added, and not without a bound: with the home folder
//! or `/` added, a set downloaded anywhere below read the user's own files (`~/.config/...`)
//! into its report (found 2026-10-22).

use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use dawsome_engine::sfz::{self, Problem, SetFile, Sfz};

/// The most bytes of one file of a set read as text: the reader takes no more than this of all
/// of a set's files together.
pub const MAX_FILE_BYTES: u64 = sfz::MAX_SET_BYTES as u64;

/// How many folders above its own a set inside a folder added to the Library reads: one for
/// `Instruments/x.sfz` playing `../Samples/`, two for a library sorted one level further.
pub const MAX_CLIMB: usize = 2;

/// Why a file a set names is not read.
const OUTSIDE: &str = "outside the set's folder";
const OUTSIDE_ADDED: &str = "outside the folder added to the Library";
const TOO_FAR: &str = "more than two folders above the set's own";
const NOT_A_FILE: &str = "not a regular file";

/// The folder of a set, resolved, and the folder everything the set reads lies inside: the
/// set's own, or a folder added to the Library that holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetFolder {
    /// The main file's folder, where the paths the set writes start.
    dir: PathBuf,
    /// The folder the set reads inside: `dir` or a folder holding it.
    root: PathBuf,
    /// How deep `dir` lies below `root`: how many `..` a path may climb before it leaves.
    depth: usize,
    /// Why a file outside `root` is not read.
    outside: &'static str,
}

impl SetFolder {
    /// The folder of the set whose main file is `main`: its parent, resolved (as written when it
    /// cannot be, since then nothing in it is found either), which the set reads inside.
    pub fn of(main: &Path) -> Self {
        let dir = main_folder(main);
        Self {
            root: dir.clone(),
            dir,
            depth: 0,
            outside: OUTSIDE,
        }
    }

    /// The folder of the set whose main file is `main`, which reads inside `root`, a folder
    /// added to the Library, at most [`MAX_CLIMB`] folders above its own: as
    /// [`of`](Self::of) when `root` does not hold the set.
    pub fn inside(main: &Path, root: &Path) -> Self {
        let own = Self::of(main);
        let Ok(root) = dunce::canonicalize(root) else {
            return own;
        };
        let Ok(below) = own.dir.strip_prefix(&root) else {
            return own;
        };
        let depth = below.components().count();
        if depth > MAX_CLIMB {
            let Some(reach) = own.dir.ancestors().nth(MAX_CLIMB) else {
                return own;
            };
            return Self {
                root: reach.to_path_buf(),
                dir: own.dir,
                depth: MAX_CLIMB,
                outside: TOO_FAR,
            };
        }
        Self {
            depth,
            dir: own.dir,
            root,
            outside: OUTSIDE_ADDED,
        }
    }

    /// The file `written` names (a path from the set's main file, as the set writes it, `/`
    /// between its parts) when it may be read: written without an absolute part or a `..`
    /// climbing out of the folder it reads inside, and, when it is there, a regular file inside
    /// that folder with every link followed. A file that is not there is given as it would be,
    /// for the caller to find missing and say so as it does of any missing file.
    pub fn file(&self, written: &str) -> Result<PathBuf, String> {
        let mut depth = self.depth;
        for part in Path::new(written).components() {
            match part {
                Component::Normal(_) => depth += 1,
                Component::CurDir => {}
                Component::ParentDir if depth > 0 => depth -= 1,
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(self.outside.to_owned());
                }
            }
        }
        let path = self.dir.join(written);
        let Ok(metadata) = fs::metadata(&path) else {
            return Ok(path);
        };
        let real = dunce::canonicalize(&path).map_err(|error| error.to_string())?;
        if !real.starts_with(&self.root) {
            Err(self.outside.to_owned())
        } else if !metadata.is_file() {
            Err(NOT_A_FILE.to_owned())
        } else {
            Ok(real)
        }
    }

    /// The text of the file an `#include` names, with what makes it that file (its resolved
    /// path), or why it is not read.
    pub fn include(&self, written: &str) -> Result<SetFile, String> {
        let path = self.file(written)?;
        let text = read_text(&path)?;
        Ok(SetFile {
            id: path.to_string_lossy().into_owned(),
            text,
        })
    }
}

/// The folder of the main file `main`, resolved (as written when it cannot be).
fn main_folder(main: &Path) -> PathBuf {
    let dir = match main.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    dunce::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf())
}

/// The folder of `folders` (the folders added to the Library) that the set whose main file is
/// `main` reads inside: the outermost one holding it that may widen a set's reach
/// ([`widens`]), resolved; `None` when none does. Why the outermost: a folder added inside
/// another adds nothing the outer one does not hold, so the set reads as it would were the
/// inner one not added.
pub fn library_root(main: &Path, folders: &[PathBuf]) -> Option<PathBuf> {
    root_among(main, folders, dirs::home_dir().as_deref())
}

/// [`library_root`] with `home` the home folder.
fn root_among(main: &Path, folders: &[PathBuf], home: Option<&Path>) -> Option<PathBuf> {
    let dir = main_folder(main);
    let home = home.map(|home| dunce::canonicalize(home).unwrap_or_else(|_| home.to_path_buf()));
    folders
        .iter()
        .filter_map(|folder| dunce::canonicalize(folder).ok())
        .filter(|folder| widens(folder, home.as_deref()))
        .filter(|folder| dir.starts_with(folder))
        .min_by_key(|folder| folder.components().count())
}

/// Whether the sets inside `folder`, a folder added to the Library (resolved), may read beyond
/// their own folders: not when it is a filesystem's root, the home folder (resolved, `home`) or
/// a folder holding it. Why: a set downloaded anywhere below would then read the user's own
/// files into its report.
fn widens(folder: &Path, home: Option<&Path>) -> bool {
    folder.parent().is_some() && !home.is_some_and(|home| home.starts_with(folder))
}

/// The text of `path`: a regular file of at most [`MAX_FILE_BYTES`], in UTF-8.
pub fn read_text(path: &Path) -> Result<String, String> {
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err(NOT_A_FILE.to_owned());
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(format!("larger than {} MB", MAX_FILE_BYTES >> 20));
    }
    String::from_utf8(bytes).map_err(|_| "not text in UTF-8".to_owned())
}

/// Reads the set whose main file is `main`, reading inside `folder` ([`SetFolder::of`] its own,
/// or [`SetFolder::inside`] a folder added to the Library): its includes through
/// [`SetFolder::include`], and a region whose sound may not be read ([`SetFolder::file`]) left
/// out and reported. `Err` says why the main file cannot be read.
pub fn read_set_in(main: &Path, folder: &SetFolder) -> Result<Sfz, String> {
    let text = read_text(main)?;
    let name = main
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let id = dunce::canonicalize(main).unwrap_or_else(|_| main.to_path_buf());
    let mut read = sfz::parse_files(&name, &id.to_string_lossy(), &text, &mut |written| {
        folder.include(written)
    });
    let regions = std::mem::take(&mut read.regions);
    let mut refused: Vec<Problem> = Vec::new();
    for region in regions {
        match folder.file(&region.sample) {
            Ok(_) => read.regions.push(region),
            Err(reason) => refused.push(Problem {
                at: region.at.clone(),
                text: format!("sample={}: {reason}; the region is left out", region.sample),
            }),
        }
    }
    // Within the reader's cap on problems, and one line counting the rest past it.
    let room = sfz::MAX_PROBLEMS.saturating_sub(read.problems.len());
    if refused.len() > room {
        let more = refused.len() - room;
        let at = refused[room].at.clone();
        refused.truncate(room);
        refused.push(Problem {
            at,
            text: format!("and {more} more regions left out from here on"),
        });
    }
    read.problems.extend(refused);
    Ok(read)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The set whose main file is `main`, read inside its own folder.
    fn read_set(main: &Path) -> Result<Sfz, String> {
        read_set_in(main, &SetFolder::of(main))
    }

    /// A fresh empty folder of this test's own.
    fn temp_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("dawsome-set-files-{}", std::process::id()))
            .join(test);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    // Inside the folder: plain names, `.` and a `..` that stays inside; a file that is not
    // there yet is given for the caller to find missing. Outside: a `..` climbing out, an
    // absolute path, and a link inside pointing out.
    #[test]
    fn a_set_reads_only_inside_its_folder() {
        let root = temp_dir("inside");
        let set = root.join("set");
        fs::create_dir_all(set.join("samples")).expect("dir");
        fs::write(set.join("samples/a.wav"), b"").expect("written");
        fs::write(root.join("secret.wav"), b"").expect("written");
        let folder = SetFolder::of(&set.join("set.sfz"));
        let real = dunce::canonicalize(&set).expect("resolves");
        assert_eq!(folder.file("samples/a.wav"), Ok(real.join("samples/a.wav")));
        assert_eq!(
            folder.file("./samples/../samples/a.wav"),
            Ok(real.join("samples/a.wav"))
        );
        assert_eq!(folder.file("gone.wav"), Ok(real.join("gone.wav")));
        let outside = Err(OUTSIDE.to_owned());
        assert_eq!(folder.file("../secret.wav"), outside);
        assert_eq!(folder.file("samples/../../secret.wav"), outside);
        let absolute = root.join("secret.wav");
        assert_eq!(folder.file(&absolute.to_string_lossy()), outside);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&absolute, set.join("link.wav")).expect("link");
            assert_eq!(folder.file("link.wav"), outside);
        }
    }

    // Only regular files are read: a folder, a device and a named pipe are refused before
    // anything opens them, and a text past the limit is refused whole.
    #[test]
    fn only_regular_files_of_a_bounded_size_are_read() {
        let set = temp_dir("regular");
        fs::create_dir_all(set.join("folder.sfz")).expect("dir");
        let folder = SetFolder::of(&set.join("set.sfz"));
        assert_eq!(
            folder.include("folder.sfz"),
            Err(NOT_A_FILE.to_owned()),
            "a folder"
        );
        assert_eq!(
            read_text(Path::new("/dev/zero")),
            Err(NOT_A_FILE.to_owned()),
            "a device"
        );
        #[cfg(unix)]
        {
            let made = std::process::Command::new("mkfifo")
                .arg(set.join("pipe.sfz"))
                .status()
                .expect("mkfifo runs");
            assert!(made.success());
            assert_eq!(folder.include("pipe.sfz"), Err(NOT_A_FILE.to_owned()));
        }
        let big = vec![b' '; usize::try_from(MAX_FILE_BYTES).expect("fits") + 1];
        fs::write(set.join("big.sfz"), big).expect("written");
        assert_eq!(
            folder.include("big.sfz"),
            Err("larger than 16 MB".to_owned())
        );
    }

    // One file under two spellings is one file: a set whose part includes itself as ./part.sfz
    // reads it once and reports the cycle; a region whose sound lies outside the folder is left
    // out and reported where it is written.
    #[test]
    fn reading_a_set_knows_its_files_and_keeps_its_sounds_inside() {
        let root = temp_dir("read");
        let set = root.join("set");
        fs::create_dir_all(&set).expect("dir");
        fs::write(
            set.join("part.sfz"),
            "<region> sample=a.wav\n#include \"./part.sfz\"\n",
        )
        .expect("written");
        fs::write(
            set.join("set.sfz"),
            "#include \"part.sfz\"\n<region> sample=../b.wav\n",
        )
        .expect("written");
        let read = read_set(&set.join("set.sfz")).expect("reads");
        let samples: Vec<&str> = read.regions.iter().map(|r| r.sample.as_str()).collect();
        assert_eq!(samples, ["a.wav"]);
        let texts: Vec<String> = read.problems.iter().map(ToString::to_string).collect();
        assert_eq!(
            texts,
            [
                "part.sfz:2: #include \"./part.sfz\" includes a file being read",
                "set.sfz:2: sample=../b.wav: outside the set's folder; the region is left out",
            ]
        );
        assert!(read_set(&set.join("none.sfz")).is_err());
    }

    /// A library laid out as many are, under `root`: `lib/Instruments/a.sfz` playing
    /// `../Samples/a1.wav` and `../Samples/a2.wav`, and `outside.wav` beside `lib/`. Returns
    /// `lib/` and the main file.
    fn library(root: &Path, text: &str) -> (PathBuf, PathBuf) {
        let lib = root.join("lib");
        fs::create_dir_all(lib.join("Instruments")).expect("dir");
        fs::create_dir_all(lib.join("Samples")).expect("dir");
        for name in ["Samples/a1.wav", "Samples/a2.wav"] {
            fs::write(lib.join(name), b"").expect("written");
        }
        fs::write(root.join("outside.wav"), b"").expect("written");
        let main = lib.join("Instruments/a.sfz");
        fs::write(&main, text).expect("written");
        (lib, main)
    }

    // A set in a folder added to the Library reads anywhere inside that folder: lib/ added, its
    // .sfz in lib/Instruments/ plays both sounds in lib/Samples/. Read as a shipped set (its own
    // folder the limit) the same two regions are left out.
    #[test]
    fn a_set_in_a_folder_added_reads_its_sibling_sample_folder() {
        let root = temp_dir("library");
        let (lib, main) = library(
            &root,
            "<region> sample=../Samples/a1.wav\n<region> sample=../Samples/a2.wav\n",
        );
        let read = read_set_in(&main, &SetFolder::inside(&main, &lib)).expect("reads");
        let samples: Vec<&str> = read.regions.iter().map(|r| r.sample.as_str()).collect();
        assert_eq!(samples, ["../Samples/a1.wav", "../Samples/a2.wav"]);
        assert!(read.problems.is_empty(), "{:?}", read.problems);
        let own = read_set(&main).expect("reads");
        assert!(own.regions.is_empty());
        assert_eq!(own.problems.len(), 2);
    }

    // Inside the folder added and nowhere else: a `..` past it, an absolute path and a link
    // pointing out of it are refused, said so in the Library's words; an include past it too.
    #[test]
    fn a_set_in_a_folder_added_reads_nothing_outside_it() {
        let root = temp_dir("library_outside");
        let (lib, main) = library(
            &root,
            "<region> sample=../Samples/a1.wav\n\
             <region> sample=../../outside.wav\n\
             #include \"../../part.sfz\"\n",
        );
        fs::write(root.join("part.sfz"), "<region> sample=a2.wav\n").expect("written");
        let read = read_set_in(&main, &SetFolder::inside(&main, &lib)).expect("reads");
        let samples: Vec<&str> = read.regions.iter().map(|r| r.sample.as_str()).collect();
        assert_eq!(samples, ["../Samples/a1.wav"]);
        let texts: Vec<String> = read.problems.iter().map(ToString::to_string).collect();
        assert_eq!(
            texts,
            [
                "a.sfz:3: #include \"../../part.sfz\": outside the folder added to the Library",
                "a.sfz:2: sample=../../outside.wav: outside the folder added to the Library; \
                 the region is left out",
            ]
        );
        let folder = SetFolder::inside(&main, &lib);
        let outside = Err(OUTSIDE_ADDED.to_owned());
        let absolute = root.join("outside.wav");
        assert_eq!(folder.file(&absolute.to_string_lossy()), outside);
        assert_eq!(folder.file("../Samples/../../outside.wav"), outside);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&absolute, lib.join("Samples/link.wav")).expect("link");
            assert_eq!(folder.file("../Samples/link.wav"), outside);
        }
    }

    /// The set whose main file is `main` read as the Library reads it, with `folders` added and
    /// `home` the home folder.
    fn read_in_library(main: &Path, folders: &[PathBuf], home: &Path) -> Sfz {
        let folder = match root_among(main, folders, Some(home)) {
            Some(root) => SetFolder::inside(main, &root),
            None => SetFolder::of(main),
        };
        read_set_in(main, &folder).expect("reads")
    }

    // The home folder added, a folder holding it, or the filesystem's root widens no set's
    // reach: a set downloaded into `home/Downloads/pack/` that includes
    // `../../.config/app/token.txt` reads inside its own folder, so the user's own file stays
    // out of its report. A folder inside the home folder, added beside it, still widens.
    #[test]
    fn the_home_folder_or_the_root_added_widens_no_set() {
        let root = temp_dir("home");
        let home = root.join("home");
        fs::create_dir_all(home.join(".config/app")).expect("dir");
        fs::write(home.join(".config/app/token.txt"), "API_TOKEN=hunter2\n").expect("written");
        let pack = home.join("Downloads/pack");
        fs::create_dir_all(&pack).expect("dir");
        let main = pack.join("x.sfz");
        fs::write(&main, "#include \"../../.config/app/token.txt\"\n").expect("written");
        for added in [home.clone(), root.clone(), PathBuf::from("/")] {
            assert_eq!(
                root_among(&main, std::slice::from_ref(&added), Some(&home)),
                None,
                "{} added",
                added.display()
            );
            let read = read_in_library(&main, &[added], &home);
            let texts: Vec<String> = read.problems.iter().map(ToString::to_string).collect();
            assert_eq!(
                texts,
                ["x.sfz:1: #include \"../../.config/app/token.txt\": outside the set's folder"]
            );
        }
        let downloads = home.join("Downloads");
        assert_eq!(
            root_among(&main, &[home.clone(), downloads.clone()], Some(&home)),
            Some(dunce::canonicalize(&downloads).expect("resolves"))
        );
    }

    // A set inside a folder added reads at most two folders above its own, though the folder
    // added lies higher: lib/ added, `lib/Instruments/a.sfz` plays `../Samples/` (one up), and
    // `lib/a/b/c/x.sfz` plays `../../near.wav` (two up) but neither `../../../Samples/a1.wav`
    // nor an include three up.
    #[test]
    fn a_set_in_a_folder_added_reads_at_most_two_folders_above_its_own() {
        let root = temp_dir("reach");
        let (lib, main) = library(&root, "<region> sample=../Samples/a1.wav\n");
        let read = read_set_in(&main, &SetFolder::inside(&main, &lib)).expect("reads");
        let samples: Vec<&str> = read.regions.iter().map(|r| r.sample.as_str()).collect();
        assert_eq!(samples, ["../Samples/a1.wav"]);
        assert!(read.problems.is_empty(), "{:?}", read.problems);
        let deep = lib.join("a/b/c");
        fs::create_dir_all(&deep).expect("dir");
        fs::write(lib.join("a/near.wav"), b"").expect("written");
        fs::write(lib.join("part.sfz"), "<region> sample=Samples/a2.wav\n").expect("written");
        let main = deep.join("x.sfz");
        fs::write(
            &main,
            "<region> sample=../../near.wav\n\
             <region> sample=../../../Samples/a1.wav\n\
             #include \"../../../part.sfz\"\n",
        )
        .expect("written");
        let read = read_set_in(&main, &SetFolder::inside(&main, &lib)).expect("reads");
        let samples: Vec<&str> = read.regions.iter().map(|r| r.sample.as_str()).collect();
        assert_eq!(samples, ["../../near.wav"]);
        let texts: Vec<String> = read.problems.iter().map(ToString::to_string).collect();
        assert_eq!(
            texts,
            [
                "x.sfz:3: #include \"../../../part.sfz\": more than two folders above the \
                 set's own",
                "x.sfz:2: sample=../../../Samples/a1.wav: more than two folders above the set's \
                 own; the region is left out",
            ]
        );
    }

    // The folder a set reads inside: the outermost folder added that holds its main file, so a
    // folder added inside another reads as the outer one; a set in no folder added has none.
    // A "folder" that only starts with the same letters holds nothing.
    #[test]
    fn the_folder_a_set_reads_inside_is_the_outermost_added_that_holds_it() {
        let root = temp_dir("library_root");
        let (lib, main) = library(&root, "");
        fs::create_dir_all(root.join("li")).expect("dir");
        let resolved = dunce::canonicalize(&lib).expect("resolves");
        let inner = lib.join("Instruments");
        assert_eq!(
            library_root(&main, &[inner.clone(), lib.clone()]),
            Some(resolved.clone())
        );
        assert_eq!(
            library_root(&main, std::slice::from_ref(&inner)),
            Some(dunce::canonicalize(&inner).expect("resolves"))
        );
        assert_eq!(library_root(&main, &[root.join("li")]), None);
        assert_eq!(library_root(&main, &[]), None);
        assert_eq!(library_root(&root.join("outside.wav"), &[lib]), None);
    }
}
