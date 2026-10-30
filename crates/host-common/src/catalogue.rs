//! The plugin catalogue: what a scan found in each plugin file, kept in the user's cache dir, so
//! a plugin is loaded into Dawsome's own process only after a scan child has loaded it without
//! crashing. Plugins are found by format and id here; the file a plugin lives in is a detail of
//! this crate.
//!
//! One catalogue holds every format's files, each record naming its format (CLAP when it names
//! none, so a catalogue written before VST3 reads and writes byte for byte as it was). Why one
//! file: `--cache FILE` keeps isolating every scripted run as it did; two files would need a
//! second flag every test must remember. A scan of some formats replaces their records and keeps
//! the others' as their own last scan left them, so programs that host different formats share
//! the file without undoing each other's work.
//!
//! The app offers one list over every format ([`Catalogue::offered`]): a plugin installed in
//! both formats is one row, its CLAP build ([`twins`]).

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The catalogue file's format version.
pub const VERSION: u32 = 1;

/// A plugin format, as a catalogue record and a processor id (`"<format>:<plugin id>"`) name it.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum PluginFormat {
    /// A `.clap` file.
    #[default]
    Clap,
    /// A `.vst3` module: a bundle folder, or a file (a Windows one from before bundles, or a
    /// Linux one built for older hosts).
    Vst3,
}

impl PluginFormat {
    /// Every format, in the order their folders are searched.
    pub const ALL: &'static [PluginFormat] = &[PluginFormat::Clap, PluginFormat::Vst3];
    /// CLAP alone.
    pub const CLAP: &'static [PluginFormat] = &[PluginFormat::Clap];

    /// Its name, the prefix of its processor ids: `"clap"`, `"vst3"`.
    pub fn name(self) -> &'static str {
        match self {
            PluginFormat::Clap => "clap",
            PluginFormat::Vst3 => "vst3",
        }
    }

    /// The format of the module at `path`, by its extension.
    pub fn of_path(path: &Path) -> Option<PluginFormat> {
        match path.extension()?.to_str()? {
            "clap" => Some(PluginFormat::Clap),
            "vst3" => Some(PluginFormat::Vst3),
            _ => None,
        }
    }

    /// The file whose size and modification time stand for the module at `path`: the file
    /// itself, except for a VST3 bundle, whose binary for this platform is
    /// `Contents/<arch>-linux/<stem>.so` or `Contents/<arch>-win/<stem>.vst3`, the bundle's own
    /// name (the format asks that the two match). Why the binary: a bundle folder's time does
    /// not change when the binary inside it is replaced.
    pub fn binary(self, path: &Path) -> PathBuf {
        match self {
            PluginFormat::Vst3 if path.is_dir() => vst3_binary(path),
            _ => path.to_owned(),
        }
    }

    fn is_clap(&self) -> bool {
        *self == PluginFormat::Clap
    }
}

/// The binary of the VST3 bundle `bundle` for this platform (see [`PluginFormat::binary`]).
fn vst3_binary(bundle: &Path) -> PathBuf {
    let stem = bundle.file_stem().unwrap_or_default();
    let (folder, extension) = if cfg!(windows) {
        (format!("{}-win", vst3_arch()), "vst3")
    } else {
        (format!("{}-linux", vst3_arch()), "so")
    };
    let mut name = stem.to_owned();
    name.push(".");
    name.push(extension);
    bundle.join("Contents").join(folder).join(name)
}

/// This machine's architecture as VST3 bundles name their binaries' folders (the format's
/// "Plug-in Format" page): `x86_64`, `i386` on Linux and `x86` on Windows, `aarch64` on Linux
/// and `arm64` on Windows, `armv7l`.
fn vst3_arch() -> &'static str {
    match (std::env::consts::ARCH, cfg!(windows)) {
        ("x86", false) => "i386",
        ("aarch64", true) => "arm64",
        ("arm", _) => "armv7l",
        (arch, _) => arch,
    }
}

/// Every plugin file a scan has seen, sorted by path; each file's plugins sorted by id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Catalogue {
    pub version: u32,
    pub files: Vec<FileRecord>,
    /// Each file's rank in the order its folders are searched (0: found in the first), as the
    /// scan found it ([`crate::scan::ranked_plugin_files`]), by path. Why: a plugin found in
    /// two files is the one in the folder searched first, as [`search_paths`] orders them, not
    /// the one whose path sorts first, which changed with where the program was installed. A
    /// file without a rank (a catalogue written before ranks were kept) comes after the rest.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ranks: BTreeMap<PathBuf, usize>,
}

/// One plugin file as the scan last saw it. `size` and `mtime` (nanoseconds since the Unix
/// epoch) tell an unchanged file from a changed one, so only changed files are loaded again; for
/// a VST3 bundle they are its binary's ([`PluginFormat::binary`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileRecord {
    pub file: PathBuf,
    /// Written only when not CLAP, so a CLAP-only catalogue keeps its bytes.
    #[serde(default, skip_serializing_if = "PluginFormat::is_clap")]
    pub format: PluginFormat,
    pub size: u64,
    pub mtime: u64,
    pub status: FileStatus,
    /// Why a failed file failed: "crashed: signal 6", "exit code 1", "timed out after 10 s",
    /// "bad output", or what its entry reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub plugins: Vec<PluginRecord>,
}

impl FileRecord {
    /// Whether the file on disk is still the one the scan saw: same size, same modification
    /// time (a VST3 bundle's binary's).
    pub fn is_current(&self) -> bool {
        module_stamp(self.format, &self.file) == Some((self.size, self.mtime))
    }
}

/// The stamp of the `format` module at `path`: its binary's ([`PluginFormat::binary`]).
pub fn module_stamp(format: PluginFormat, path: &Path) -> Option<(u64, u64)> {
    stamp(&format.binary(path))
}

/// A file's size and modification time (nanoseconds since the Unix epoch), as a record keeps
/// them; `None` when the file cannot be read.
pub fn stamp(file: &Path) -> Option<(u64, u64)> {
    let meta = fs::metadata(file).ok()?;
    let since = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some((
        meta.len(),
        u64::try_from(since.as_nanos()).unwrap_or(u64::MAX),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileStatus {
    Ok,
    /// The file crashed, hung or refused to load; its plugins are not offered until it changes.
    Failed,
}

/// One plugin a file offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginRecord {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub kind: PluginKind,
    pub gui: GuiSupport,
    /// How many parameters it has.
    pub params: u32,
}

/// A plugin as the one list over every format offers it: its format, its record, and whether
/// it is also installed as VST3 (a CLAP plugin whose VST3 twin is not offered).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offered {
    pub format: PluginFormat,
    pub plugin: PluginRecord,
    pub twin: bool,
}

impl Offered {
    /// The processor id a project names it by: `"<format>:<id>"`.
    pub fn processor_id(&self) -> String {
        format!("{}:{}", self.format.name(), self.plugin.id)
    }
}

/// Whether `vst3` and `clap` are one plugin installed in two formats: the same kind, the same
/// name and the same maker, both trimmed, lower-cased and their spaces collapsed, and the
/// maker's whole words "CLAP", "VST3" and "VST" left out. Why the maker's words: makers name
/// themselves per format ("LSP CLAP", "LSP VST3"); why not the id: the formats' ids have nothing
/// in common.
pub fn twins(clap: &PluginRecord, vst3: &PluginRecord) -> bool {
    twin_key(clap) == twin_key(vst3)
}

/// What [`twins`] compares.
fn twin_key(plugin: &PluginRecord) -> (PluginKind, String, String) {
    let words =
        |text: &str| -> Vec<String> { text.split_whitespace().map(str::to_lowercase).collect() };
    let maker: Vec<String> = words(&plugin.vendor)
        .into_iter()
        .filter(|word| !matches!(word.as_str(), "clap" | "vst3" | "vst"))
        .collect();
    (plugin.kind, words(&plugin.name).join(" "), maker.join(" "))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginKind {
    /// Plays notes: a plugin channel.
    Instrument,
    /// Processes audio: a mixer slot.
    Effect,
}

/// How a plugin can show its own interface on this desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuiSupport {
    /// Inside a window of ours (X11 embedding).
    Embed,
    /// In a window it makes itself.
    Floating,
    None,
}

/// Why the catalogue file could not be read.
#[derive(Debug, Error)]
pub enum CatalogueError {
    #[error("cannot read {}: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("{} is not a plugin catalogue: {source}", path.display())]
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
}

impl Default for Catalogue {
    fn default() -> Self {
        Self {
            version: VERSION,
            files: Vec::new(),
            ranks: BTreeMap::new(),
        }
    }
}

impl Catalogue {
    /// The catalogue at `path`. A file that does not exist is an empty catalogue, and so is one
    /// of another format version: either way every plugin file gets scanned.
    pub fn load(path: &Path) -> Result<Self, CatalogueError> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(source) => {
                return Err(CatalogueError::Read {
                    path: path.to_owned(),
                    source,
                })
            }
        };
        let catalogue: Self =
            serde_json::from_str(&text).map_err(|source| CatalogueError::Parse {
                path: path.to_owned(),
                source,
            })?;
        Ok(if catalogue.version == VERSION {
            catalogue.sorted()
        } else {
            Self::default()
        })
    }

    /// Writes the catalogue to `path`, sorted, through a temporary file renamed over it, so a
    /// reader never sees half a catalogue. Creates the directory.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            fs::create_dir_all(dir)?;
        }
        let mut text = serde_json::to_string(&self.clone().sorted()).map_err(io::Error::other)?;
        text.push('\n');
        let mut temp = path.as_os_str().to_owned();
        temp.push(format!(".{}.tmp", std::process::id()));
        let temp = PathBuf::from(temp);
        fs::write(&temp, text)?;
        fs::rename(&temp, path).inspect_err(|_| {
            let _ = fs::remove_file(&temp);
        })
    }

    /// Files by path, each file's plugins by id.
    pub fn sorted(mut self) -> Self {
        self.files.sort_by(|a, b| a.file.cmp(&b.file));
        for file in &mut self.files {
            file.plugins.sort_by(|a, b| a.id.cmp(&b.id));
        }
        self
    }

    /// The `format` plugin with id `id` and the file it is in, from files that scanned well: of
    /// two files offering it, the one in the folder searched first, then the first by path.
    pub fn find(&self, format: PluginFormat, id: &str) -> Option<(&FileRecord, &PluginRecord)> {
        self.in_search_order()
            .into_iter()
            .filter(|file| file.status == FileStatus::Ok && file.format == format)
            .find_map(|file| Some((file, file.plugins.iter().find(|p| p.id == id)?)))
    }

    /// Every plugin of `kind` of every format, as the app lists them: each format's
    /// [`plugins_of`](Self::plugins_of) (a plugin in two files of one format once), a VST3 plugin
    /// left out when a CLAP plugin [`twins`] it (that CLAP one marked `twin`), sorted by name,
    /// then format (CLAP first), then id. Why the CLAP build: it opens its own window, every
    /// project and test so far names it, and both builds play alike; the user never has to pick
    /// a format. A VST3 plugin left out is still found for a project naming it
    /// ([`find`](Self::find)). A CLAP-only catalogue offers what `plugins` lists, in its order.
    pub fn offered(&self, kind: PluginKind) -> Vec<Offered> {
        let clap = self.plugins_of(PluginFormat::Clap, kind);
        let vst3 = self.plugins_of(PluginFormat::Vst3, kind);
        let twinned = |plugin: &PluginRecord, others: &[&PluginRecord]| {
            others
                .iter()
                .any(|other| twin_key(other) == twin_key(plugin))
        };
        let mut offered: Vec<Offered> = clap
            .iter()
            .map(|&plugin| Offered {
                format: PluginFormat::Clap,
                plugin: plugin.clone(),
                twin: twinned(plugin, &vst3),
            })
            .chain(
                vst3.iter()
                    .filter(|&&plugin| !twinned(plugin, &clap))
                    .map(|&plugin| Offered {
                        format: PluginFormat::Vst3,
                        plugin: plugin.clone(),
                        twin: false,
                    }),
            )
            .collect();
        offered.sort_by(|a, b| {
            (&a.plugin.name, a.format, &a.plugin.id).cmp(&(&b.plugin.name, b.format, &b.plugin.id))
        });
        offered
    }

    /// Every `format` plugin of `kind`, sorted by name with ties broken by id, as the picker
    /// lists them. A plugin found in two files (one the user installed, one bundled with the
    /// program) is listed once, as the file [`find`](Self::find) gives for its id, the one a
    /// project loads.
    pub fn plugins_of(&self, format: PluginFormat, kind: PluginKind) -> Vec<&PluginRecord> {
        let mut seen = BTreeSet::new();
        let mut plugins: Vec<&PluginRecord> = self
            .in_search_order()
            .into_iter()
            .filter(|file| file.status == FileStatus::Ok && file.format == format)
            .flat_map(|file| &file.plugins)
            .filter(|plugin| seen.insert(plugin.id.as_str()))
            .filter(|plugin| plugin.kind == kind)
            .collect();
        plugins.sort_by(|a, b| (&a.name, &a.id).cmp(&(&b.name, &b.id)));
        plugins
    }

    /// The files by the rank of their folders in the search order, then by path.
    fn in_search_order(&self) -> Vec<&FileRecord> {
        let rank = |file: &FileRecord| self.ranks.get(&file.file).copied().unwrap_or(usize::MAX);
        let mut files: Vec<&FileRecord> = self.files.iter().collect();
        files.sort_by(|a, b| (rank(a), &a.file).cmp(&(rank(b), &b.file)));
        files
    }

    /// Every file that failed, of every format, by path: what the app lists under the plugins.
    pub fn failed_all(&self) -> Vec<&FileRecord> {
        let mut failed: Vec<&FileRecord> = self
            .files
            .iter()
            .filter(|file| file.status == FileStatus::Failed)
            .collect();
        failed.sort_by(|a, b| a.file.cmp(&b.file));
        failed
    }

    /// The `format` files that failed, by path.
    pub fn failed_of(&self, format: PluginFormat) -> impl Iterator<Item = &FileRecord> {
        self.files
            .iter()
            .filter(move |file| file.status == FileStatus::Failed && file.format == format)
    }
}

/// Where the catalogue lives: `dawsome/clap-catalogue.json` in the per-user cache folder,
/// `$DAWSOME_CACHE_HOME` when that is an absolute path, else the platform's: `$XDG_CACHE_HOME`
/// or `~/.cache` on Linux, `%LOCALAPPDATA%` on Windows. `None` without any. Why a variable of
/// our own: tests and scripted runs move the folder the same way on every platform. The media
/// crate finds its cache folder by the same rule; neither crate depends on the other.
pub fn default_path() -> Option<PathBuf> {
    let cache = env::var_os("DAWSOME_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(dirs::cache_dir)?;
    Some(cache.join("dawsome").join("clap-catalogue.json"))
}

/// The directories CLAP plugins are installed in, in the order the CLAP standard gives:
/// `~/.clap` and `/usr/lib/clap` on Linux, `%COMMONPROGRAMFILES%\CLAP` and
/// `%LOCALAPPDATA%\Programs\Common\CLAP` on Windows, then each directory of `$CLAP_PATH`
/// (separated as the platform's `PATH` is), then the plugins bundled with the program
/// ([`bundled_plugins`]). A plugin found in two of them is the one in the first
/// ([`Catalogue::find`]): a copy the user installed wins over the bundled one.
pub fn search_paths() -> Vec<PathBuf> {
    search_paths_for(PluginFormat::CLAP)
}

/// The directories plugins of `formats` are installed in: each format's standard folders
/// (CLAP's as [`search_paths`] gives them; VST3's in the order its "Plug-in Locations" page
/// gives: `~/.vst3`, `/usr/lib64/vst3`, `/usr/lib/vst3`, `/usr/local/lib64/vst3`,
/// `/usr/local/lib/vst3` on Linux, `%LOCALAPPDATA%\Programs\Common\VST3` and
/// `%COMMONPROGRAMFILES%\VST3` on Windows), then `$CLAP_PATH`, then `$VST3_PATH` (not in
/// Steinberg's documents, but honored by other hosts, and how tests point at their own), then
/// the plugins bundled with the program. "The first found plug-in has to be used", VST3's rule
/// too, is the catalogue's rank rule.
pub fn search_paths_for(formats: &[PluginFormat]) -> Vec<PathBuf> {
    ordered_paths(
        formats,
        |format| match format {
            PluginFormat::Clap => standard_paths(),
            PluginFormat::Vst3 => vst3_standard_paths(),
        },
        |variable| env::var_os(variable),
        bundled_plugins(),
    )
}

/// The search order of [`search_paths_for`] over the folders and variables it is given: each
/// format's `standard` folders, then each format's variable, then `bundled`.
fn ordered_paths(
    formats: &[PluginFormat],
    standard: impl Fn(PluginFormat) -> Vec<PathBuf>,
    variable: impl Fn(&str) -> Option<std::ffi::OsString>,
    bundled: Vec<PathBuf>,
) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = formats
        .iter()
        .flat_map(|&format| standard(format))
        .collect();
    for format in formats {
        let name = match format {
            PluginFormat::Clap => "CLAP_PATH",
            PluginFormat::Vst3 => "VST3_PATH",
        };
        if let Some(extra) = variable(name) {
            dirs.extend(env::split_paths(&extra).filter(|dir| !dir.as_os_str().is_empty()));
        }
    }
    dirs.extend(bundled);
    dirs
}

/// The folder of plugins bundled with the program, beside it in a release zip.
pub const BUNDLED: &str = "plugins";

/// The folders of plugins bundled with this program ([`bundled_folders`] of its own file).
pub fn bundled_plugins() -> Vec<PathBuf> {
    env::current_exe()
        .map(|program| bundled_folders(&program))
        .unwrap_or_default()
}

/// The folders of plugins bundled with the program at `program`, each that is a folder:
/// [`BUNDLED`] beside it (a release zip), then, on other systems than Windows,
/// `lib/dawsome/clap` under the prefix it is installed in (`<prefix>/bin/dawsome-app`, as
/// scripts/install.sh puts it). Both are searched when both are there.
pub fn bundled_folders(program: &Path) -> Vec<PathBuf> {
    let Some(dir) = program.parent() else {
        return Vec::new();
    };
    let installed = dir
        .parent()
        .filter(|_| !cfg!(windows))
        .map(|prefix| prefix.join("lib").join("dawsome").join("clap"));
    std::iter::once(dir.join(BUNDLED))
        .chain(installed)
        .filter(|folder| folder.is_dir())
        .collect()
}

/// The first of [`bundled_folders`], the one searched first; `None` when there is none.
pub fn bundled_beside(program: &Path) -> Option<PathBuf> {
    bundled_folders(program).into_iter().next()
}

#[cfg(unix)]
fn standard_paths() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".clap"));
    }
    dirs.push(PathBuf::from("/usr/lib/clap"));
    dirs
}

#[cfg(unix)]
fn vst3_standard_paths() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".vst3"));
    }
    for dir in [
        "/usr/lib64/vst3",
        "/usr/lib/vst3",
        "/usr/local/lib64/vst3",
        "/usr/local/lib/vst3",
    ] {
        dirs.push(PathBuf::from(dir));
    }
    dirs
}

#[cfg(windows)]
fn vst3_standard_paths() -> Vec<PathBuf> {
    let local = env::var_os("LOCALAPPDATA").map(|dir| {
        PathBuf::from(dir)
            .join("Programs")
            .join("Common")
            .join("VST3")
    });
    let common = env::var_os("COMMONPROGRAMFILES").map(|dir| PathBuf::from(dir).join("VST3"));
    local.into_iter().chain(common).collect()
}

#[cfg(windows)]
fn standard_paths() -> Vec<PathBuf> {
    let common = env::var_os("COMMONPROGRAMFILES").map(|dir| PathBuf::from(dir).join("CLAP"));
    let local = env::var_os("LOCALAPPDATA").map(|dir| {
        PathBuf::from(dir)
            .join("Programs")
            .join("Common")
            .join("CLAP")
    });
    common.into_iter().chain(local).collect()
}

/// Whether the catalogue at `path` needs a scan for `formats`: it does not exist or cannot be
/// read, one of `dirs` changed (a plugin was added or removed there) after it was written, or
/// the plugin files of those formats under `dirs`, subfolders included, are not the ones it
/// lists as they are (by path, size and modification time), or the files of a format it ranks
/// are in another order in `dirs` (the folders were reordered), or one of them is not found
/// there. Records of other formats count for nothing. A catalogue written before ranks were kept
/// ranks nothing, so it is used as it is until its next scan.
pub fn is_stale_for(path: &Path, dirs: &[PathBuf], formats: &[PluginFormat]) -> bool {
    let Some(written) = modified(path) else {
        return true;
    };
    if dirs
        .iter()
        .filter_map(|dir| modified(dir))
        .any(|changed| changed > written)
    {
        return true;
    }
    let Ok(catalogue) = Catalogue::load(path) else {
        return true;
    };
    let ranked = crate::scan::ranked_plugin_files(dirs, formats);
    // Rank numbers count folders of the list they were made from: the command line's scan of
    // both formats ranks a `.clap` in `$CLAP_PATH` after the VST3 folders, the app's CLAP-only
    // list right after CLAP's own. So only the order they give each format's files is compared,
    // by (rank, path), which is all `in_search_order` gives `find` and `plugins_of` (both look
    // at one format). Comparing the numbers made every catalogue the command line wrote stale
    // to the app, which then rescanned at every start (review 2026-10-26).
    let mut listed_order = Vec::new();
    let mut found_order = Vec::new();
    for (file, &rank) in &catalogue.ranks {
        let Some(format) = PluginFormat::of_path(file).filter(|format| formats.contains(format))
        else {
            continue;
        };
        let Some(&found) = ranked.get(file) else {
            return true;
        };
        listed_order.push((format, rank, file));
        found_order.push((format, found, file));
    }
    listed_order.sort_unstable();
    found_order.sort_unstable();
    let listed_files = listed_order.iter().map(|&(_, _, file)| file);
    if !listed_files.eq(found_order.iter().map(|&(_, _, file)| file)) {
        return true;
    }
    let listed: BTreeSet<(PathBuf, u64, u64)> = catalogue
        .files
        .into_iter()
        .filter(|record| formats.contains(&record.format))
        .map(|record| (record.file, record.size, record.mtime))
        .collect();
    let found: BTreeSet<(PathBuf, u64, u64)> = ranked
        .into_keys()
        .filter_map(|file| {
            let (size, mtime) = module_stamp(PluginFormat::of_path(&file)?, &file)?;
            Some((file, size, mtime))
        })
        .collect();
    found != listed
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|meta| meta.modified()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../../tests/fixtures/p7_catalogue.json");

    fn temp_dir(name: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("dawsome-catalogue-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// Gives the file or folder at `path` the modification time `time`.
    fn touch(path: &Path, time: SystemTime) {
        // Windows sets a time only through a handle that may write attributes, and opens a
        // folder only with backup semantics.
        #[cfg(windows)]
        let opened = {
            use std::os::windows::fs::OpenOptionsExt;
            use windows_sys::Win32::Storage::FileSystem::{
                FILE_FLAG_BACKUP_SEMANTICS, FILE_WRITE_ATTRIBUTES,
            };
            fs::OpenOptions::new()
                .access_mode(FILE_WRITE_ATTRIBUTES)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                .open(path)
        };
        #[cfg(not(windows))]
        let opened = fs::File::open(path);
        opened.expect("opens").set_modified(time).expect("touches");
    }

    // The plugins bundled with the program are found from its place: `plugins/` beside it (a
    // release zip) first, else, not on Windows, `lib/dawsome/clap` under the prefix of
    // `<prefix>/bin/` (where scripts/install.sh puts them); none when neither is a folder.
    #[test]
    fn the_bundled_plugins_are_found_beside_the_program() {
        let root = temp_dir("bundled");
        let program = root.join("bin").join("dawsome-app");
        fs::create_dir_all(root.join("bin")).expect("dirs");
        assert_eq!(bundled_beside(&program), None, "none bundled");
        let installed = root.join("lib").join("dawsome").join("clap");
        fs::create_dir_all(&installed).expect("dirs");
        let found = bundled_beside(&program);
        if cfg!(windows) {
            assert_eq!(found, None);
        } else {
            assert_eq!(found, Some(installed));
        }
        let beside = root.join("bin").join("plugins");
        fs::create_dir_all(&beside).expect("dirs");
        assert_eq!(bundled_beside(&program), Some(beside));
    }

    // A plugin found in two files (one installed by the user, one bundled) is offered once:
    // the one the host loads for its id, the first file by path.
    #[test]
    fn a_plugin_in_two_files_is_listed_once_as_the_one_loaded() {
        let mut catalogue: Catalogue = serde_json::from_str(FIXTURE).expect("parses");
        let mut second = catalogue.files[2].clone();
        second.file = PathBuf::from("/usr/lib/clap/example.clap");
        second.plugins[0].version = "2.0.0".to_owned();
        catalogue.files.push(second);
        let catalogue = catalogue.sorted();
        let bells: Vec<(&str, &str)> = catalogue
            .plugins_of(PluginFormat::Clap, PluginKind::Instrument)
            .iter()
            .filter(|plugin| plugin.id == "org.example.bells")
            .map(|plugin| (plugin.name.as_str(), plugin.version.as_str()))
            .collect();
        assert_eq!(bells, [("Bell Keys", "1.0.0")]);
        let (file, _) = catalogue
            .find(PluginFormat::Clap, "org.example.bells")
            .expect("listed");
        assert_eq!(file.file, Path::new("/plugins/example.clap"));
    }

    // Both folders of bundled plugins are searched when both are there: `plugins/` beside the
    // program first, then, not on Windows, `lib/dawsome/clap` under its prefix.
    #[test]
    fn both_bundled_plugin_folders_are_searched() {
        let root = temp_dir("bundled-both");
        let program = root.join("bin").join("dawsome-app");
        fs::create_dir_all(root.join("bin")).expect("dirs");
        assert!(bundled_folders(&program).is_empty(), "none bundled");
        let installed = root.join("lib").join("dawsome").join("clap");
        let beside = root.join("bin").join("plugins");
        fs::create_dir_all(&installed).expect("dirs");
        fs::create_dir_all(&beside).expect("dirs");
        let want = if cfg!(windows) {
            vec![beside]
        } else {
            vec![beside, installed]
        };
        assert_eq!(bundled_folders(&program), want);
    }

    // A plugin in two files is found and listed as the file whose folder ranks first in the
    // search, though its path sorts last; a file of no rank comes after the ranked ones.
    #[test]
    fn a_plugin_in_two_files_is_the_one_whose_folder_is_searched_first() {
        let mut catalogue: Catalogue = serde_json::from_str(FIXTURE).expect("parses");
        let mut user = catalogue.files[2].clone();
        user.file = PathBuf::from("/usr/lib/clap/example.clap");
        user.plugins[0].version = "2.0.0".to_owned();
        catalogue.files.push(user);
        let mut catalogue = catalogue.sorted();
        catalogue
            .ranks
            .insert(PathBuf::from("/usr/lib/clap/example.clap"), 1);
        let versions = |catalogue: &Catalogue| -> (String, Vec<String>) {
            let (_, found) = catalogue
                .find(PluginFormat::Clap, "org.example.bells")
                .expect("found");
            let listed = catalogue
                .plugins_of(PluginFormat::Clap, PluginKind::Instrument)
                .iter()
                .filter(|plugin| plugin.id == "org.example.bells")
                .map(|plugin| plugin.version.clone())
                .collect();
            (found.version.clone(), listed)
        };
        assert_eq!(
            versions(&catalogue),
            ("2.0.0".to_owned(), vec!["2.0.0".to_owned()])
        );
        catalogue
            .ranks
            .insert(PathBuf::from("/plugins/example.clap"), 0);
        assert_eq!(
            versions(&catalogue),
            ("1.0.0".to_owned(), vec!["1.0.0".to_owned()])
        );
    }

    #[test]
    fn the_fixture_parses_and_lists_plugins_by_kind_and_name() {
        let catalogue: Catalogue = serde_json::from_str(FIXTURE).expect("parses");
        let names = |kind| -> Vec<&str> {
            catalogue
                .plugins_of(PluginFormat::Clap, kind)
                .iter()
                .map(|plugin| plugin.name.as_str())
                .collect()
        };
        assert_eq!(
            names(PluginKind::Instrument),
            ["Analog Lead", "Bell Keys", "Dawsome Test Sine"]
        );
        assert_eq!(names(PluginKind::Effect), ["Dawsome Test Gain"]);
        let failed: Vec<&str> = catalogue
            .failed_of(PluginFormat::Clap)
            .filter_map(|file| file.reason.as_deref())
            .collect();
        assert_eq!(failed, ["crashed: signal 6"]);
        let (file, plugin) = catalogue
            .find(PluginFormat::Clap, "org.dawsome.test.sine")
            .expect("listed");
        assert_eq!(file.file, Path::new("/plugins/dawsome_test_plugin.clap"));
        assert_eq!(plugin.kind, PluginKind::Instrument);
        assert!(catalogue
            .find(PluginFormat::Clap, "org.example.missing")
            .is_none());
    }

    #[test]
    fn a_saved_catalogue_is_sorted_and_reads_back() {
        let mut catalogue: Catalogue = serde_json::from_str(FIXTURE).expect("parses");
        catalogue.files.reverse();
        for file in &mut catalogue.files {
            file.plugins.reverse();
        }
        let path = temp_dir("save").join("nested").join("catalogue.json");
        catalogue.save(&path).expect("saves");
        let text = fs::read_to_string(&path).expect("written");
        assert_eq!(
            text,
            format!("{FIXTURE}\n"),
            "sorted back to the fixture's order"
        );
        assert_eq!(Catalogue::load(&path).expect("loads"), catalogue.sorted());
    }

    #[test]
    fn a_missing_or_old_catalogue_is_empty_and_a_broken_one_is_an_error() {
        let dir = temp_dir("load");
        assert_eq!(
            Catalogue::load(&dir.join("none.json")).expect("empty"),
            Catalogue::default()
        );
        let old = dir.join("old.json");
        fs::write(&old, r#"{"version":0,"files":[]}"#).expect("written");
        assert_eq!(Catalogue::load(&old).expect("empty"), Catalogue::default());
        let broken = dir.join("broken.json");
        fs::write(&broken, "not json").expect("written");
        assert!(matches!(
            Catalogue::load(&broken),
            Err(CatalogueError::Parse { .. })
        ));
    }

    #[test]
    fn a_catalogue_older_than_a_plugin_dir_is_stale() {
        let dir = temp_dir("stale");
        let plugins = dir.join("plugins");
        fs::create_dir_all(&plugins).expect("plugin dir");
        let path = dir.join("catalogue.json");
        assert!(
            is_stale_for(&path, std::slice::from_ref(&plugins), PluginFormat::CLAP),
            "missing"
        );
        Catalogue::default().save(&path).expect("saves");
        let written = modified(&path).expect("has a time");
        touch(&path, written + std::time::Duration::from_secs(60));
        assert!(
            !is_stale_for(&path, std::slice::from_ref(&plugins), PluginFormat::CLAP),
            "fresh"
        );
        touch(&plugins, written + std::time::Duration::from_secs(120));
        assert!(
            is_stale_for(&path, &[plugins], PluginFormat::CLAP),
            "a dir changed after it"
        );
    }

    // Folder times say nothing about subfolders or about a file changed in place, so the files
    // themselves are compared: a new file in a subfolder, then one that changed, make the
    // catalogue stale though it is newer than every folder.
    #[test]
    fn a_catalogue_that_does_not_list_the_files_as_they_are_is_stale() {
        let dir = temp_dir("stale-files");
        let sub = dir.join("plugins").join("vendor");
        fs::create_dir_all(&sub).expect("plugin dirs");
        let path = dir.join("catalogue.json");
        let save_later = |catalogue: &Catalogue| {
            catalogue.save(&path).expect("saves");
            let written = modified(&path).expect("has a time");
            touch(&path, written + std::time::Duration::from_secs(60));
        };
        let dirs = [dir.join("plugins")];
        save_later(&Catalogue::default());
        assert!(
            !is_stale_for(&path, &dirs, PluginFormat::CLAP),
            "no files, none listed"
        );
        let plugin = sub.join("new.clap");
        fs::write(&plugin, b"one").expect("file");
        touch(&sub, modified(&path).expect("has a time"));
        assert!(
            is_stale_for(&path, &dirs, PluginFormat::CLAP),
            "a new file in a subfolder"
        );
        let meta = fs::metadata(&plugin).expect("metadata");
        let mtime = meta
            .modified()
            .expect("a time")
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("after the epoch");
        let listed = Catalogue {
            files: vec![FileRecord {
                file: fs::canonicalize(&plugin).expect("real path"),
                format: PluginFormat::Clap,
                size: meta.len(),
                mtime: u64::try_from(mtime.as_nanos()).expect("fits"),
                status: FileStatus::Ok,
                reason: None,
                plugins: Vec::new(),
            }],
            ..Catalogue::default()
        };
        save_later(&listed);
        assert!(
            !is_stale_for(&path, &dirs, PluginFormat::CLAP),
            "listed as it is"
        );
        fs::write(&plugin, b"changed").expect("rewritten");
        assert!(
            is_stale_for(&path, &dirs, PluginFormat::CLAP),
            "changed in place"
        );
    }

    // The command line scans both formats, so a `.clap` in a folder searched after the VST3
    // folders ($CLAP_PATH, the bundled ones) ranks after them; the app's CLAP-only search ranks
    // it right after the CLAP standard folders. Nothing on disk changed, so the catalogue the
    // command line wrote is fresh in both views, until two CLAP folders really swap. (Review
    // 2026-10-26, confirmed 1: the app rescanned at every start after a `dawsome scan`.)
    #[test]
    fn a_catalogue_of_both_formats_is_fresh_in_a_view_of_one() {
        let dir = temp_dir("stale-views");
        let clap_standard = dir.join("clap");
        let vst3_standard = dir.join("vst3");
        let clap_extra = dir.join("clap-extra");
        for folder in [&clap_standard, &vst3_standard, &clap_extra] {
            fs::create_dir_all(folder).expect("plugin dir");
        }
        fs::write(clap_standard.join("a.clap"), b"one").expect("file");
        fs::write(vst3_standard.join("Single.vst3"), b"two").expect("file");
        fs::write(clap_extra.join("b.clap"), b"three").expect("file");
        let all_dirs = [
            clap_standard.clone(),
            vst3_standard.clone(),
            clap_extra.clone(),
        ];
        let clap_dirs = [clap_standard.clone(), clap_extra.clone()];
        // As `scan()` builds it: every file `ranked_plugin_files` finds, stamped, with its rank.
        let mut catalogue = Catalogue::default();
        for (file, rank) in crate::scan::ranked_plugin_files(&all_dirs, PluginFormat::ALL) {
            let format = PluginFormat::of_path(&file).expect("a module");
            let (size, mtime) = module_stamp(format, &file).expect("stamped");
            catalogue.ranks.insert(file.clone(), rank);
            catalogue.files.push(FileRecord {
                file,
                format,
                size,
                mtime,
                status: FileStatus::Ok,
                reason: None,
                plugins: Vec::new(),
            });
        }
        assert_eq!(
            catalogue.ranks.values().copied().collect::<Vec<_>>(),
            [0, 2, 1],
            "a.clap, b.clap after the VST3 folder, Single.vst3"
        );
        let path = dir.join("catalogue.json");
        catalogue.sorted().save(&path).expect("saves");
        let written = modified(&path).expect("has a time");
        touch(&path, written + std::time::Duration::from_secs(60));
        assert!(
            !is_stale_for(&path, &all_dirs, PluginFormat::ALL),
            "fresh in the order it was written in"
        );
        assert!(
            !is_stale_for(&path, &clap_dirs, PluginFormat::CLAP),
            "fresh in the app's CLAP-only view"
        );
        assert!(
            !is_stale_for(&path, &all_dirs, &[PluginFormat::Vst3]),
            "fresh in a VST3-only view"
        );
        assert!(
            is_stale_for(&path, &[clap_extra, clap_standard], PluginFormat::CLAP),
            "stale once two CLAP folders swap"
        );
    }

    /// A VST3 record of `bundle` with no plugins, stamped as the scan stamps it.
    fn vst3_record(bundle: &Path) -> FileRecord {
        let (size, mtime) = module_stamp(PluginFormat::Vst3, bundle).expect("its binary is there");
        FileRecord {
            file: bundle.to_owned(),
            format: PluginFormat::Vst3,
            size,
            mtime,
            status: FileStatus::Ok,
            reason: None,
            plugins: Vec::new(),
        }
    }

    /// A VST3 bundle `<dir>/<name>.vst3` holding a binary of this platform with `bytes`.
    fn bundle(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let bundle = dir.join(format!("{name}.vst3"));
        let (arch, binary) = if cfg!(windows) {
            ("x86_64-win", format!("{name}.vst3"))
        } else {
            ("x86_64-linux", format!("{name}.so"))
        };
        let folder = bundle.join("Contents").join(arch);
        fs::create_dir_all(&folder).expect("bundle folders");
        fs::write(folder.join(&binary), bytes).expect("binary");
        assert_eq!(
            PluginFormat::Vst3.binary(&bundle),
            folder.join(binary),
            "a bundle's binary is its own name under Contents/<arch>"
        );
        bundle
    }

    // A VST3 record stands for its bundle's binary: replaced in place, with the bundle folder's
    // time put back as it was, the record is no longer current and the catalogue is stale for
    // VST3, while a scan for CLAP alone does not look at VST3 records at all. A single-file
    // module is its own binary.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn a_vst3_bundle_is_stamped_by_its_binary() {
        let dir = temp_dir("vst3-stamp");
        let plugins = dir.join("plugins");
        let bundle = bundle(&plugins, "Synth", b"one");
        let record = vst3_record(&fs::canonicalize(&bundle).expect("real path"));
        assert!(record.is_current());
        let path = dir.join("catalogue.json");
        let listed = Catalogue {
            files: vec![record.clone()],
            ..Catalogue::default()
        };
        listed.save(&path).expect("saves");
        let written = modified(&path).expect("has a time");
        touch(&path, written + std::time::Duration::from_secs(60));
        let dirs = [plugins.clone()];
        assert!(!is_stale_for(&path, &dirs, PluginFormat::ALL), "as scanned");
        let folder_time = modified(&bundle).expect("has a time");
        fs::write(PluginFormat::Vst3.binary(&bundle), b"changed").expect("replaced");
        touch(&bundle, folder_time);
        assert!(!record.is_current(), "the binary changed");
        assert!(is_stale_for(&path, &dirs, PluginFormat::ALL));
        assert!(
            !is_stale_for(&path, &dirs, PluginFormat::CLAP),
            "CLAP alone: no CLAP file there, none listed"
        );
        let single = plugins.join("Single.vst3");
        fs::write(&single, b"elf").expect("file");
        assert_eq!(PluginFormat::Vst3.binary(&single), single);
        assert_eq!(PluginFormat::of_path(&single), Some(PluginFormat::Vst3));
        assert_eq!(
            PluginFormat::of_path(Path::new("a.clap")),
            Some(PluginFormat::Clap)
        );
        assert_eq!(PluginFormat::of_path(Path::new("a.so")), None);
    }

    // tests/fixtures/gen_p27_expected.py's catalogue of both formats (a CLAP file, a failed and a
    // good VST3 module, ranks) reads and writes back to its own bytes: the format field where the
    // generator puts it, and only on the VST3 records.
    #[test]
    fn a_catalogue_of_both_formats_reads_and_writes_byte_for_byte() {
        let fixture = include_str!("../../../tests/fixtures/p27_catalogue.json");
        let catalogue: Catalogue = serde_json::from_str(fixture).expect("parses");
        let formats: Vec<PluginFormat> = catalogue.files.iter().map(|file| file.format).collect();
        assert_eq!(
            formats,
            [PluginFormat::Clap, PluginFormat::Vst3, PluginFormat::Vst3]
        );
        let path = temp_dir("both-formats").join("catalogue.json");
        catalogue.save(&path).expect("saves");
        assert_eq!(
            fs::read_to_string(&path).expect("written"),
            format!("{fixture}\n")
        );
        assert_eq!(
            catalogue
                .plugins_of(PluginFormat::Vst3, PluginKind::Effect)
                .iter()
                .map(|plugin| plugin.name.as_str())
                .collect::<Vec<_>>(),
            ["Dawsome Test Gain", "Dawsome Test Latency"]
        );
    }

    // A VST3 record names its format right after its file; a CLAP record names none, so the
    // fixture's CLAP-only catalogue keeps its bytes (the test above), and a record that names no
    // format reads as CLAP.
    #[test]
    fn a_vst3_record_names_its_format_and_a_clap_one_does_not() {
        let mut catalogue: Catalogue = serde_json::from_str(FIXTURE).expect("parses");
        assert!(catalogue
            .files
            .iter()
            .all(|file| file.format == PluginFormat::Clap));
        let mut vst3 = catalogue.files[2].clone();
        vst3.file = PathBuf::from("/vst3/Dawsome Test VST3.vst3");
        vst3.format = PluginFormat::Vst3;
        vst3.plugins.truncate(1);
        vst3.plugins[0].id = "446177736F6D65546573744761696E31".to_owned();
        catalogue.files.push(vst3);
        let text = serde_json::to_string(&catalogue.clone().sorted()).expect("writes");
        assert!(
            text.contains(
                r#"{"file":"/vst3/Dawsome Test VST3.vst3","format":"vst3","size":2048,"mtime":1,"status":"ok","plugins":[{"id":"446177736F6D65546573744761696E31","#
            ),
            "{text}"
        );
        assert_eq!(text.matches(r#""format""#).count(), 1, "{text}");
        let back: Catalogue = serde_json::from_str(&text).expect("reads back");
        assert_eq!(back, catalogue.sorted());
    }

    // One id in two formats' files is two plugins: each format finds and lists its own.
    #[test]
    fn each_format_finds_and_lists_its_own_plugins() {
        let mut catalogue: Catalogue = serde_json::from_str(FIXTURE).expect("parses");
        let mut vst3 = catalogue.files[2].clone();
        vst3.file = PathBuf::from("/vst3/Bells.vst3");
        vst3.format = PluginFormat::Vst3;
        vst3.plugins.truncate(1);
        vst3.plugins[0].name = "VST3 Bells".to_owned();
        catalogue.files.push(vst3);
        let catalogue = catalogue.sorted();
        let (file, plugin) = catalogue
            .find(PluginFormat::Vst3, "org.example.bells")
            .expect("the VST3 one");
        assert_eq!(
            (file.file.as_path(), plugin.name.as_str()),
            (Path::new("/vst3/Bells.vst3"), "VST3 Bells")
        );
        let (file, _) = catalogue
            .find(PluginFormat::Clap, "org.example.bells")
            .expect("the CLAP one");
        assert_eq!(file.file, Path::new("/plugins/example.clap"));
        let names = |plugins: Vec<&PluginRecord>| -> Vec<String> {
            plugins.iter().map(|plugin| plugin.name.clone()).collect()
        };
        assert_eq!(
            names(catalogue.plugins_of(PluginFormat::Clap, PluginKind::Instrument)),
            ["Analog Lead", "Bell Keys", "Dawsome Test Sine"],
            "the app's list is CLAP's alone"
        );
        assert_eq!(
            names(catalogue.plugins_of(PluginFormat::Vst3, PluginKind::Instrument)),
            ["VST3 Bells"]
        );
        assert_eq!(catalogue.failed_of(PluginFormat::Clap).count(), 1);
        assert_eq!(catalogue.failed_of(PluginFormat::Vst3).count(), 0);
    }

    // The search order: each format's standard folders, then $CLAP_PATH, then $VST3_PATH (each
    // split as PATH is), then the bundled folders; CLAP alone leaves VST3's out.
    #[test]
    fn the_search_order_is_standard_folders_then_variables_then_bundled() {
        let standard =
            |format: PluginFormat| vec![PathBuf::from(format!("/std/{}", format.name()))];
        let joined = |dirs: &[&str]| env::join_paths(dirs).expect("joins");
        let variable = |name: &str| match name {
            "CLAP_PATH" => Some(joined(&["/clap/a", "/clap/b"])),
            "VST3_PATH" => Some(joined(&["/vst3/a"])),
            _ => None,
        };
        let bundled = vec![PathBuf::from("/bundled")];
        let all = ordered_paths(PluginFormat::ALL, standard, variable, bundled.clone());
        let want: Vec<PathBuf> = [
            "/std/clap",
            "/std/vst3",
            "/clap/a",
            "/clap/b",
            "/vst3/a",
            "/bundled",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        assert_eq!(all, want);
        let clap = ordered_paths(PluginFormat::CLAP, standard, variable, bundled);
        let picked = [&want[0], &want[2], &want[3], &want[5]].map(PathBuf::clone);
        assert_eq!(clap, picked);
    }

    /// The catalogue a twin case of tests/fixtures/p28_expected.json describes.
    fn twin_catalogue(case: &serde_json::Value) -> Catalogue {
        let files = case["files"].as_array().expect("files");
        let files = files
            .iter()
            .map(|file| FileRecord {
                file: PathBuf::from(file["file"].as_str().expect("a path")),
                format: serde_json::from_value(file["format"].clone()).expect("a format"),
                size: 1,
                mtime: 1,
                status: serde_json::from_value(file["status"].clone()).expect("a status"),
                reason: file["reason"].as_str().map(str::to_owned),
                plugins: serde_json::from_value(file["plugins"].clone()).expect("records"),
            })
            .collect();
        Catalogue {
            files,
            ..Catalogue::default()
        }
        .sorted()
    }

    // The twin rule, case by case as gen_p28_expected.py states it: one plugin in both formats is
    // one row, the CLAP one marked; another maker, another kind or a CLAP file that failed leave
    // both (or the VST3 one) listed; names and makers are compared trimmed, without case, their
    // spaces collapsed and the maker's words CLAP, VST3 and VST left out (whole words only).
    #[test]
    fn a_plugin_in_both_formats_is_offered_once_as_its_clap_build() {
        let expected: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/p28_expected.json"))
                .expect("p28_expected.json parses");
        for case in expected["twins"].as_array().expect("cases") {
            let catalogue = twin_catalogue(case);
            for (kind, name) in [
                (PluginKind::Instrument, "instrument"),
                (PluginKind::Effect, "effect"),
            ] {
                let offered: Vec<serde_json::Value> = catalogue
                    .offered(kind)
                    .into_iter()
                    .map(|row| {
                        serde_json::json!({
                            "format": row.format.name(),
                            "id": row.plugin.id,
                            "twin": row.twin,
                        })
                    })
                    .collect();
                assert_eq!(
                    serde_json::Value::Array(offered),
                    case["offered"][name],
                    "{}: {name}",
                    case["case"]
                );
            }
        }
    }

    // A catalogue of CLAP alone offers what `plugins` lists, in its order, none marked, and its
    // failed files of every format are its CLAP ones; the VST3 file that failed in p27's
    // catalogue of both formats is among `failed_all`, by path.
    #[test]
    fn a_clap_only_catalogue_offers_what_it_did() {
        let catalogue: Catalogue = serde_json::from_str(FIXTURE).expect("parses");
        for kind in [PluginKind::Instrument, PluginKind::Effect] {
            let offered = catalogue.offered(kind);
            let plugins = catalogue.plugins_of(PluginFormat::Clap, kind);
            assert_eq!(offered.len(), plugins.len());
            for (row, plugin) in offered.iter().zip(plugins) {
                assert_eq!(
                    (row.format, &row.plugin, row.twin),
                    (PluginFormat::Clap, plugin, false)
                );
                assert_eq!(row.processor_id(), format!("clap:{}", plugin.id));
            }
        }
        let paths = |files: Vec<&FileRecord>| -> Vec<PathBuf> {
            files.into_iter().map(|file| file.file.clone()).collect()
        };
        assert_eq!(
            paths(catalogue.failed_all()),
            paths(catalogue.failed_of(PluginFormat::Clap).collect())
        );
        let both: Catalogue =
            serde_json::from_str(include_str!("../../../tests/fixtures/p27_catalogue.json"))
                .expect("parses");
        assert_eq!(
            paths(both.failed_all()),
            [PathBuf::from("/vst3/Dawsome Test VST3 Crash.vst3")]
        );
        let names: Vec<(String, PluginFormat, bool)> = both
            .offered(PluginKind::Instrument)
            .into_iter()
            .map(|row| (row.plugin.name, row.format, row.twin))
            .collect();
        assert_eq!(
            names,
            [
                ("Dawsome Test Fail".to_owned(), PluginFormat::Vst3, false),
                ("Dawsome Test Sine".to_owned(), PluginFormat::Clap, true),
                ("Dawsome Test Tick".to_owned(), PluginFormat::Vst3, false),
                ("Dawsome Test View".to_owned(), PluginFormat::Vst3, false),
            ]
        );
    }
}
