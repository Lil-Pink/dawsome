//! The plugin scan's parent half. The parent finds every plugin module of the formats asked for
//! under the plugin directories (`.clap` files; `.vst3` bundles and files) and, for each one not
//! already in the catalogue as it is now, runs a child process that loads it, describes its
//! plugins on one line of stdout, and exits. A file that
//! crashes, hangs or refuses to load takes only its child down and is recorded as failed, with
//! the reason.
//!
//! How long a child may take: it prints a progress line ([`PROGRESS_PREFIX`]) as it finishes
//! each plugin it describes, and it is killed after [`CHILD_TIMEOUT`] without one, or after
//! [`CHILD_CAP`] in all. Why: a module of hundreds of plugins (LSP's VST3 holds 198) takes
//! longer than one fixed timeout allows while making steady progress, and a hung one must still
//! go in seconds. Only progress lines count: a plugin's own prints do not keep a hung child
//! alive. Progress lines are never kept or recorded.
//!
//! The child's work, loading the file, is the plugin host's (`dawsome_host_clap::scan`); a
//! binary that hosts the child (the command line's `dawsome scan --child <file>`, the app's
//! `--scan-child <file>`) calls the host's `run_child`, which prints its result through
//! [`run_child_with`].

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::catalogue::{self, Catalogue, FileRecord, FileStatus, PluginFormat, PluginRecord};
use crate::child::{self, Tree};

/// How long a child may go without a progress line before it is killed: the most one plugin
/// may take to describe (the module's loading included, for the first).
pub const CHILD_TIMEOUT: Duration = Duration::from_secs(10);

/// The most a child may take for one file in all, progress or not.
pub const CHILD_CAP: Duration = Duration::from_secs(60);

/// The most of a child's stdout that is read; the rest is drained and dropped.
const CHILD_OUTPUT_LIMIT: usize = 1 << 20;

/// How long the end of a child's output is waited for after the child is gone, at least: the
/// reader may not have run since the pipe closed.
const OUTPUT_GRACE: Duration = Duration::from_millis(200);

/// What starts the child line, so a plugin that prints to stdout itself cannot be mistaken for
/// the result.
pub const RESULT_PREFIX: &str = "dawsome-scan-result ";

/// What starts the line a child prints as it finishes describing each plugin (its count so far
/// follows), which restarts its [`CHILD_TIMEOUT`].
pub const PROGRESS_PREFIX: &str = "dawsome-scan-progress ";

/// The program and leading arguments that run a scan child; the file to describe goes last.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildCommand {
    pub program: PathBuf,
    pub args: Vec<OsString>,
}

/// What to scan, into which catalogue file.
#[derive(Debug, Clone)]
pub struct ScanRequest {
    /// Searched recursively for modules of `formats`.
    pub dirs: Vec<PathBuf>,
    /// The formats scanned; the catalogue's records of other formats are kept as they are.
    pub formats: Vec<PluginFormat>,
    pub cache: PathBuf,
    /// Scan every file again, also unchanged ones (the user asked for a rescan).
    pub force: bool,
    /// How long a child may go without a progress line ([`CHILD_TIMEOUT`]).
    pub timeout: Duration,
    /// The most a child may take in all ([`CHILD_CAP`]).
    pub cap: Duration,
    pub child: ChildCommand,
}

/// What a scan did.
#[derive(Debug, Clone, PartialEq)]
pub struct ScanReport {
    /// The files loaded by a child this time, with what was found.
    pub scanned: Vec<FileRecord>,
    /// Files already in the catalogue as they are now, not loaded again.
    pub unchanged: usize,
}

impl ScanReport {
    /// `"2 files: 1 scanned, 1 unchanged"`.
    pub fn summary(&self) -> String {
        let total = self.scanned.len() + self.unchanged;
        let files = if total == 1 { "file" } else { "files" };
        format!(
            "{total} {files}: {} scanned, {} unchanged",
            self.scanned.len(),
            self.unchanged
        )
    }
}

/// The catalogue file could not be written; the scan's results are still in [`Scanned`].
#[derive(Debug, Error)]
#[error("cannot write the plugin catalogue {}: {source}", path.display())]
pub struct UnsavedCatalogue {
    pub path: PathBuf,
    pub source: io::Error,
}

/// What a scan found and did.
#[derive(Debug)]
pub struct Scanned {
    pub catalogue: Catalogue,
    pub report: ScanReport,
    /// Why the catalogue file was not written, when it was not: the plugins found can still be
    /// used, but the next scan loads every file again.
    pub unsaved: Option<UnsavedCatalogue>,
}

/// Scans `request.dirs` for `request.formats` into the catalogue at `request.cache`. Files of
/// those formats that are gone are dropped from the catalogue, and the records of other formats
/// are kept with their ranks; a catalogue file that cannot be read is scanned anew.
pub fn scan(request: &ScanRequest) -> Scanned {
    let known = Catalogue::load(&request.cache).unwrap_or_default();
    let mut catalogue = Catalogue::default();
    let mut report = ScanReport {
        scanned: Vec::new(),
        unchanged: 0,
    };
    for (file, rank) in ranked_plugin_files(&request.dirs, &request.formats) {
        let Some(format) = PluginFormat::of_path(&file) else {
            continue;
        };
        let Some((size, mtime)) = catalogue::module_stamp(format, &file) else {
            continue;
        };
        catalogue.ranks.insert(file.clone(), rank);
        let cached = known.files.iter().find(|record| {
            record.file == file
                && record.format == format
                && record.size == size
                && record.mtime == mtime
        });
        let record = match cached {
            Some(record) if !request.force => {
                report.unchanged += 1;
                record.clone()
            }
            _ => {
                let limits = Limits {
                    quiet: request.timeout,
                    total: request.cap,
                };
                let mut record = scan_file(&file, size, mtime, &request.child, limits);
                record.format = format;
                report.scanned.push(record.clone());
                record
            }
        };
        catalogue.files.push(record);
    }
    for record in known
        .files
        .iter()
        .filter(|record| !request.formats.contains(&record.format))
    {
        if let Some(&rank) = known.ranks.get(&record.file) {
            catalogue.ranks.insert(record.file.clone(), rank);
        }
        catalogue.files.push(record.clone());
    }
    let catalogue = catalogue.sorted();
    let unsaved = catalogue
        .save(&request.cache)
        .err()
        .map(|source| UnsavedCatalogue {
            path: request.cache.clone(),
            source,
        });
    Scanned {
        catalogue,
        report,
        unsaved,
    }
}

/// Every module of `formats` under `dirs`, recursively and through links, each once (by its real
/// path), sorted: `.clap` files, `.vst3` folders (a VST3 bundle, not looked into: the format's
/// module finder takes any `.vst3` entry as a module) and `.vst3` files. Directories that do
/// not exist are skipped. Each with its rank: the place in `dirs` of the first of them it is
/// found under (through links too), so of two files offering one plugin the one in the folder
/// searched first can be told.
pub fn ranked_plugin_files(dirs: &[PathBuf], formats: &[PluginFormat]) -> BTreeMap<PathBuf, usize> {
    let wanted =
        |path: &Path| PluginFormat::of_path(path).filter(|format| formats.contains(format));
    let mut found = BTreeMap::new();
    let mut seen_dirs = BTreeSet::new();
    for (rank, top) in dirs.iter().enumerate() {
        let mut stack = vec![top.clone()];
        while let Some(dir) = stack.pop() {
            // A real path seen before is a link loop or a directory listed twice; its files
            // keep the rank of the folder they were found under first.
            let Ok(real) = fs::canonicalize(&dir) else {
                continue;
            };
            if !seen_dirs.insert(real.clone()) {
                continue;
            }
            let Ok(entries) = fs::read_dir(&real) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                // `metadata` follows links, so a linked file or directory counts as what it
                // names.
                let Ok(meta) = fs::metadata(&path) else {
                    continue;
                };
                match (wanted(&path), meta.is_dir()) {
                    // A VST3 bundle is a module; a folder of any other name is searched.
                    (Some(PluginFormat::Vst3), _) | (Some(PluginFormat::Clap), false) => {
                        if let Ok(file) = fs::canonicalize(&path) {
                            found.entry(file).or_insert(rank);
                        }
                    }
                    (_, true) => stack.push(path),
                    (_, false) => {}
                }
            }
        }
    }
    found
}

/// How long a child may take: `quiet` without a progress line, `total` in all.
#[derive(Debug, Clone, Copy)]
struct Limits {
    quiet: Duration,
    total: Duration,
}

/// What the child prints after [`RESULT_PREFIX`].
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ChildResult {
    Plugins(Vec<PluginRecord>),
    Error(String),
}

/// Runs a child on `file` and records what it said, or how it died.
fn scan_file(
    file: &Path,
    size: u64,
    mtime: u64,
    child: &ChildCommand,
    limits: Limits,
) -> FileRecord {
    let (status, reason, plugins) = match run_child_process(file, child, limits) {
        Ok(ChildResult::Plugins(plugins)) => (FileStatus::Ok, None, plugins),
        Ok(ChildResult::Error(reason)) | Err(reason) => {
            (FileStatus::Failed, Some(reason), Vec::new())
        }
    };
    FileRecord {
        file: file.to_owned(),
        format: PluginFormat::default(),
        size,
        mtime,
        status,
        reason,
        plugins,
    }
}

/// What the reader of a child's output tells the wait.
enum Heard {
    /// A progress line.
    Progress,
    /// The output has ended: what was kept of it (every line but the progress lines, up to
    /// [`CHILD_OUTPUT_LIMIT`]).
    End(Vec<u8>),
}

/// Starts the child on `file` and waits for it and its output within `limits`; `Err` carries
/// the reason a failed file is recorded with.
fn run_child_process(
    file: &Path,
    child: &ChildCommand,
    limits: Limits,
) -> Result<ChildResult, String> {
    // A process tree of its own, so whatever it starts can be killed with it.
    let (mut process, tree) = Tree::spawn(
        Command::new(&child.program)
            .args(&child.args)
            .arg(file)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
    )
    .map_err(|error| format!("cannot start the scan: {error}"))?;
    let Some(stdout) = process.stdout.take() else {
        tree.kill();
        let _ = process.kill();
        let _ = process.wait();
        return Err("cannot start the scan: no output pipe".into());
    };
    let started = Instant::now();
    // Read on a thread of its own, so a child that fills the pipe cannot block the wait below.
    // Never joined: a process that left the tree can hold the pipe open for ever, and the scan
    // does not wait for it.
    let (sender, heard) = mpsc::channel();
    thread::spawn(move || read_output(stdout, &sender));
    let mut clock = Clock {
        cap: started + limits.total,
        quiet_until: started + limits.quiet,
        limits,
    };
    let mut kept = None;
    let status = loop {
        while let Ok(news) = heard.try_recv() {
            clock.hear(news, &mut kept);
        }
        match process.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < clock.deadline() => {
                thread::sleep(Duration::from_millis(5));
            }
            Ok(None) | Err(_) => {
                tree.kill();
                let _ = process.kill();
                let _ = process.wait();
                return Err(clock.timed_out());
            }
        }
    };
    // What the child left running goes with it: a helper still holding the pipe would keep its
    // output from ending.
    tree.kill();
    let grace = Instant::now() + OUTPUT_GRACE;
    let output = loop {
        if let Some(output) = kept.take() {
            break output;
        }
        let until = clock.deadline().max(grace);
        match heard.recv_timeout(until.saturating_duration_since(Instant::now())) {
            Ok(news) => clock.hear(news, &mut kept),
            Err(_) => return Err(clock.timed_out()),
        }
    };
    if let Some(reason) = child::failure(status) {
        return Err(reason);
    }
    let text = String::from_utf8_lossy(&output);
    text.lines()
        .rev()
        .find_map(|line| line.strip_prefix(RESULT_PREFIX))
        .and_then(|json| serde_json::from_str(json).ok())
        .ok_or_else(|| "bad output".to_owned())
}

/// When a child is killed: `limits.quiet` after its last progress line (or its start), and
/// `limits.total` after its start at the latest.
struct Clock {
    cap: Instant,
    quiet_until: Instant,
    limits: Limits,
}

impl Clock {
    fn deadline(&self) -> Instant {
        self.quiet_until.min(self.cap)
    }

    /// A progress line restarts the quiet time; the output's end is kept in `kept`.
    fn hear(&mut self, news: Heard, kept: &mut Option<Vec<u8>>) {
        match news {
            Heard::Progress => self.quiet_until = Instant::now() + self.limits.quiet,
            Heard::End(output) => *kept = Some(output),
        }
    }

    /// The reason a child killed at the deadline is recorded with: the limit it reached.
    fn timed_out(&self) -> String {
        let limit = if self.cap <= self.quiet_until {
            self.limits.total
        } else {
            self.limits.quiet
        };
        format!("timed out after {}", seconds(limit))
    }
}

/// Reads a child's stdout to its end, line by line: each progress line is told to the wait as it
/// comes and dropped, the rest kept up to [`CHILD_OUTPUT_LIMIT`] and handed over at the end. A
/// line is read [`CHILD_OUTPUT_LIMIT`] bytes at most at a time, so one without an end cannot
/// grow without bound.
fn read_output(stdout: impl Read, heard: &mpsc::Sender<Heard>) {
    let mut reader = io::BufReader::new(stdout);
    let mut kept = Vec::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        match (&mut reader)
            .take(CHILD_OUTPUT_LIMIT as u64)
            .read_until(b'\n', &mut line)
        {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if line.starts_with(PROGRESS_PREFIX.as_bytes()) {
            let _ = heard.send(Heard::Progress);
            continue;
        }
        let room = CHILD_OUTPUT_LIMIT - kept.len();
        kept.extend_from_slice(&line[..line.len().min(room)]);
    }
    let _ = heard.send(Heard::End(kept));
}

/// A duration as the reason text writes it: "10 s", "0.2 s".
fn seconds(duration: Duration) -> String {
    let seconds = duration.as_secs_f64();
    if seconds.fract() == 0.0 {
        format!("{seconds:.0} s")
    } else {
        format!("{seconds} s")
    }
}

/// The child's whole job, given the host's `describe` of one file: describe `file` and print
/// the result line. `describe` calls the progress function it is given as it finishes each
/// plugin, which prints a progress line. Returns the process exit code: 0 whenever a result
/// line was printed, also for a file that refused to load.
pub fn run_child_with(
    file: &Path,
    describe: impl FnOnce(&Path, &mut dyn FnMut()) -> Result<Vec<PluginRecord>, String>,
) -> i32 {
    child::prepare_child();
    let mut described = 0usize;
    // A line that cannot be written is lost, not a panic: it only keeps the parent waiting.
    let mut progress = || {
        described += 1;
        let _ = writeln!(io::stdout().lock(), "{PROGRESS_PREFIX}{described}");
    };
    let result = match describe(file, &mut progress) {
        Ok(plugins) => ChildResult::Plugins(plugins),
        Err(reason) => ChildResult::Error(reason),
    };
    match serde_json::to_string(&result) {
        Ok(json) => {
            println!("{RESULT_PREFIX}{json}");
            0
        }
        Err(_) => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scan's own limits.
    const SCAN: Limits = Limits {
        quiet: CHILD_TIMEOUT,
        total: CHILD_CAP,
    };

    /// The scan's cap, with `quiet` as the time a child may go without a progress line.
    fn quiet(quiet: Duration) -> Limits {
        Limits { quiet, ..SCAN }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dawsome-scan-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn plugin_files_are_found_recursively_through_links_once_each() {
        let dir = temp_dir("files");
        let nested = dir.join("a").join("b");
        fs::create_dir_all(&nested).expect("nested");
        fs::write(nested.join("deep.clap"), b"").expect("file");
        fs::write(dir.join("top.clap"), b"").expect("file");
        fs::write(dir.join("notes.txt"), b"").expect("file");
        // Links need no privilege on Linux only; on Windows the rest still holds without them.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(nested.join("deep.clap"), dir.join("alias.clap"))
                .expect("link");
            std::os::unix::fs::symlink(&dir, nested.join("loop")).expect("loop link");
        }
        let found: Vec<PathBuf> = ranked_plugin_files(
            &[dir.clone(), dir.join("missing"), dir.clone()],
            PluginFormat::CLAP,
        )
        .into_keys()
        .collect();
        let real = fs::canonicalize(&dir).expect("real");
        assert_eq!(
            found,
            [
                real.join("a").join("b").join("deep.clap"),
                real.join("top.clap")
            ],
            "the link names deep.clap; the loop and the repeated dir add nothing"
        );
    }

    // A `.vst3` folder is a module, never looked into, and so is a `.vst3` file; a scan for CLAP
    // alone finds neither (and looks into the folder as into any other).
    #[test]
    fn vst3_bundles_and_files_are_modules_and_found_only_when_asked_for() {
        let dir = temp_dir("vst3-files");
        let inner = dir.join("Synth.vst3").join("Contents").join("Resources");
        fs::create_dir_all(&inner).expect("bundle");
        fs::write(inner.join("inside.vst3"), b"").expect("a module-like name inside");
        fs::write(inner.join("inside.clap"), b"").expect("a CLAP-like name inside");
        fs::write(dir.join("Single.vst3"), b"").expect("file");
        fs::write(dir.join("top.clap"), b"").expect("file");
        let real = fs::canonicalize(&dir).expect("real");
        assert_eq!(
            ranked_plugin_files(std::slice::from_ref(&dir), PluginFormat::ALL)
                .into_keys()
                .collect::<Vec<_>>(),
            [
                real.join("Single.vst3"),
                real.join("Synth.vst3"),
                real.join("top.clap")
            ]
        );
        assert_eq!(
            ranked_plugin_files(std::slice::from_ref(&dir), PluginFormat::CLAP)
                .into_keys()
                .collect::<Vec<_>>(),
            [inner_real(&real), real.join("top.clap")]
        );
        assert_eq!(
            ranked_plugin_files(&[dir], &[PluginFormat::Vst3])
                .into_keys()
                .collect::<Vec<_>>(),
            [real.join("Single.vst3"), real.join("Synth.vst3")]
        );
    }

    fn inner_real(real: &Path) -> PathBuf {
        real.join("Synth.vst3")
            .join("Contents")
            .join("Resources")
            .join("inside.clap")
    }

    // A scan for CLAP keeps the VST3 records (and their ranks) a scan for both made, without
    // loading them, and lists its own files as before; the catalogue then is fresh for both.
    #[cfg(unix)]
    #[test]
    fn a_scan_of_one_format_keeps_the_records_of_the_others() {
        let dir = temp_dir("keeps-formats");
        let plugins = dir.join("plugins");
        fs::create_dir_all(&plugins).expect("folder");
        fs::write(plugins.join("one.clap"), b"").expect("file");
        fs::write(plugins.join("Two.vst3"), b"").expect("a single-file module");
        let child = shell(&format!(
            r#"echo '{RESULT_PREFIX}{{"plugins":[{{"id":"'"$(basename "$1")"'","name":"P","vendor":"V","version":"1","kind":"instrument","gui":"none","params":0}}]}}'"#
        ));
        let request = |formats: &[PluginFormat]| ScanRequest {
            dirs: vec![plugins.clone()],
            formats: formats.to_vec(),
            cache: dir.join("catalogue.json"),
            force: true,
            timeout: CHILD_TIMEOUT,
            cap: CHILD_CAP,
            child: child.clone(),
        };
        let both = scan(&request(PluginFormat::ALL));
        assert_eq!(both.report.scanned.len(), 2);
        let clap = scan(&request(PluginFormat::CLAP));
        assert_eq!(clap.report.summary(), "1 file: 1 scanned, 0 unchanged");
        assert_eq!(
            clap.catalogue, both.catalogue,
            "the VST3 record and rank kept"
        );
        let saved = Catalogue::load(&dir.join("catalogue.json")).expect("saved");
        let (file, _) = saved
            .find(PluginFormat::Vst3, "Two.vst3")
            .expect("the VST3 plugin");
        assert_eq!(file.format, PluginFormat::Vst3);
        assert!(saved.find(PluginFormat::Clap, "Two.vst3").is_none());
        assert!(saved.find(PluginFormat::Clap, "one.clap").is_some());
        let cache = dir.join("catalogue.json");
        let written = fs::metadata(&cache)
            .and_then(|meta| meta.modified())
            .expect("a time");
        fs::File::open(&cache)
            .expect("opens")
            .set_modified(written + Duration::from_secs(60))
            .expect("touches");
        for formats in [PluginFormat::ALL, PluginFormat::CLAP, &[PluginFormat::Vst3]] {
            assert!(
                !catalogue::is_stale_for(&cache, std::slice::from_ref(&plugins), formats),
                "{formats:?}"
            );
        }
    }

    // Two folders hold a file offering one plugin id, and the scan keeps both. The copy found
    // and listed is the one in the folder searched first, whichever path sorts first: `z/`
    // searched before `a/`, then the other way round. A catalogue scanned in one order is stale
    // in the other. (The child, a shell script, names the plugin's version after the folder its
    // file is in.)
    #[cfg(unix)]
    #[test]
    fn a_plugin_in_two_folders_is_the_copy_in_the_folder_searched_first() {
        use crate::catalogue::PluginKind;

        let dir = temp_dir("two-folders");
        for folder in ["a", "z"] {
            fs::create_dir_all(dir.join(folder)).expect("folder");
            fs::write(dir.join(folder).join("same.clap"), b"").expect("file");
        }
        let child = shell(&format!(
            r#"v=$(basename "$(dirname "$1")"); echo '{RESULT_PREFIX}{{"plugins":[{{"id":"org.example.same","name":"Same","vendor":"Example","version":"'"$v"'","kind":"instrument","gui":"none","params":0}}]}}'"#
        ));
        for (order, want) in [(["z", "a"], "z"), (["a", "z"], "a")] {
            let request = ScanRequest {
                dirs: order.iter().map(|folder| dir.join(folder)).collect(),
                formats: vec![PluginFormat::Clap],
                cache: dir.join(format!("{want}-first.json")),
                force: true,
                timeout: CHILD_TIMEOUT,
                cap: CHILD_CAP,
                child: child.clone(),
            };
            scan(&request);
            let catalogue = Catalogue::load(&request.cache).expect("saved");
            assert_eq!(catalogue.files.len(), 2, "both files are kept");
            let (_, found) = catalogue
                .find(PluginFormat::Clap, "org.example.same")
                .expect("found");
            assert_eq!(found.version, want, "{order:?}");
            let listed: Vec<&str> = catalogue
                .plugins_of(PluginFormat::Clap, PluginKind::Instrument)
                .iter()
                .map(|plugin| plugin.version.as_str())
                .collect();
            assert_eq!(listed, [want], "{order:?}");
            assert!(!catalogue::is_stale_for(
                &request.cache,
                &request.dirs,
                PluginFormat::CLAP
            ));
            let reversed: Vec<PathBuf> = request.dirs.iter().rev().cloned().collect();
            assert!(catalogue::is_stale_for(
                &request.cache,
                &reversed,
                PluginFormat::CLAP
            ));
        }
    }

    /// `sh -c <script>`. The file path the scan adds last becomes the script's `$1`, so nothing
    /// runs it.
    fn shell(script: &str) -> ChildCommand {
        ChildCommand {
            program: PathBuf::from("sh"),
            args: vec!["-c".into(), script.into(), "child".into()],
        }
    }

    /// `cmd /c <words> & rem`. The file path the scan adds last follows `rem`, so nothing runs
    /// it. Separate words, as no word holds a space: the standard library then adds no quotes,
    /// which cmd would read its own way.
    fn cmd(words: &[&str]) -> ChildCommand {
        let words = ["/c"].iter().chain(words).chain(&["&", "rem"]);
        ChildCommand {
            program: PathBuf::from("cmd"),
            args: words.map(OsString::from).collect(),
        }
    }

    /// A child that prints nothing for 30 s while a process it waits for holds the output pipe:
    /// `sleep` under `sh`, or `ping` under `cmd` (one echo a second, the Windows shell's way to
    /// wait).
    fn silent_child() -> ChildCommand {
        if cfg!(windows) {
            cmd(&["ping", "-n", "30", "127.0.0.1"])
        } else {
            shell("sleep 30; true")
        }
    }

    // A child that never answers is killed at the deadline and its file recorded with the
    // reason (the scan's 10 s, here a fifth of a second).
    #[test]
    fn a_child_that_hangs_is_killed_and_recorded_as_timed_out() {
        let started = Instant::now();
        let record = scan_file(
            Path::new("/plugins/hang.clap"),
            1,
            2,
            &silent_child(),
            quiet(Duration::from_millis(200)),
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "killed, not waited out"
        );
        assert_eq!(record.status, FileStatus::Failed);
        assert_eq!(record.reason.as_deref(), Some("timed out after 0.2 s"));
        assert_eq!(seconds(CHILD_TIMEOUT), "10 s");
    }

    // The timeout bounds the whole wait, not only the child's own life: a grandchild still
    // holding the output pipe goes with the child at the deadline, and one a child leaves behind
    // as it exits is not waited out either (the scan's 10 s). On Linux that child prints its
    // result first. On Windows it is `ping` behind `start /b`, and the child prints nothing:
    // cmd cannot be given the result line, whose quotes the standard library would escape in a
    // way cmd does not read, so the file is recorded as bad output, at once.
    #[test]
    fn a_grandchild_holding_the_output_does_not_outlast_the_timeout() {
        let started = Instant::now();
        let hung = scan_file(
            Path::new("/plugins/linger.clap"),
            1,
            2,
            &silent_child(),
            quiet(Duration::from_millis(500)),
        );
        let waited = started.elapsed();
        assert!(waited < Duration::from_secs(5), "waited {waited:?}");
        assert_eq!(hung.status, FileStatus::Failed);
        assert_eq!(hung.reason.as_deref(), Some("timed out after 0.5 s"));

        let (lingering, expected) = if cfg!(windows) {
            (
                cmd(&["start", "/b", "ping", "-n", "30", "127.0.0.1"]),
                (FileStatus::Failed, Some("bad output")),
            )
        } else {
            let answered = format!(r#"sleep 25 & echo '{RESULT_PREFIX}{{"plugins":[]}}'"#);
            (shell(&answered), (FileStatus::Ok, None))
        };
        let started = Instant::now();
        let record = scan_file(Path::new("/plugins/helper.clap"), 1, 2, &lingering, SCAN);
        let waited = started.elapsed();
        assert!(waited < Duration::from_secs(5), "waited {waited:?}");
        assert_eq!((record.status, record.reason.as_deref()), expected);
    }

    // An exit code, a signal and output without the result line, each recorded with its reason.
    // Linux only: the child is a POSIX shell script (`exit`, `kill`, `echo` with quotes), which
    // Windows does not have.
    #[cfg(unix)]
    #[test]
    fn exits_and_bad_output_are_recorded_with_their_reasons() {
        let reason =
            |script: &str| scan_file(Path::new("/x.clap"), 0, 0, &shell(script), SCAN).reason;
        assert_eq!(reason("exit 3").as_deref(), Some("exit code 3"));
        assert_eq!(
            reason("kill -ABRT $$").as_deref(),
            Some("crashed: signal 6")
        );
        assert_eq!(reason("echo hello").as_deref(), Some("bad output"));
        let fine = format!(r#"echo noise; echo '{RESULT_PREFIX}{{"plugins":[]}}'"#);
        let record = scan_file(Path::new("/x.clap"), 0, 0, &shell(&fine), SCAN);
        assert_eq!((record.status, record.reason), (FileStatus::Ok, None));
        let refused = format!(r#"echo '{RESULT_PREFIX}{{"error":"it offers no plugins"}}'"#);
        assert_eq!(reason(&refused).as_deref(), Some("it offers no plugins"));
    }

    /// `sh -c` with `script` printing a progress line every 100 ms, `lines` of them (none: for
    /// ever), then running `after`.
    #[cfg(unix)]
    fn progressing(lines: Option<u32>, after: &str) -> ChildCommand {
        let count = lines.map_or_else(|| "true".to_owned(), |n| format!("[ $i -lt {n} ]"));
        shell(&format!(
            r#"i=0; while {count}; do echo "{PROGRESS_PREFIX}$i"; sleep 0.1; i=$((i+1)); done; {after}"#
        ))
    }

    // CHILD_TIMEOUT is time without a progress line, not the child's whole life: a child that
    // prints one every 100 ms for 12 s, then its result, is not killed at 10 s, and its
    // progress lines are not taken for output. (Linux: the children are shell scripts, as
    // above.)
    #[cfg(unix)]
    #[test]
    fn a_child_that_makes_progress_outlives_the_timeout() {
        let result = format!(r#"echo '{RESULT_PREFIX}{{"plugins":[]}}'"#);
        let started = Instant::now();
        let record = scan_file(
            Path::new("/plugins/many.vst3"),
            1,
            2,
            &progressing(Some(120), &result),
            SCAN,
        );
        let waited = started.elapsed();
        assert_eq!((record.status, record.reason), (FileStatus::Ok, None));
        assert!(waited >= Duration::from_secs(12), "ran {waited:?}");
    }

    // A child that prints progress for 3 s, then nothing: killed CHILD_TIMEOUT after its last
    // line (at about 13 s), not after its start, with the timeout's reason.
    #[cfg(unix)]
    #[test]
    fn a_child_silent_for_the_timeout_is_killed() {
        let started = Instant::now();
        let record = scan_file(
            Path::new("/plugins/stuck.vst3"),
            1,
            2,
            &progressing(Some(30), "sleep 30"),
            SCAN,
        );
        let waited = started.elapsed();
        assert_eq!(record.status, FileStatus::Failed);
        assert_eq!(record.reason.as_deref(), Some("timed out after 10 s"));
        assert!(
            waited >= Duration::from_secs(12) && waited < Duration::from_secs(17),
            "killed after {waited:?}"
        );
    }

    // Progress or not, a child goes at the cap (the scan's 60 s, here 2 s, with 1 s of quiet),
    // recorded with the cap's reason.
    #[cfg(unix)]
    #[test]
    fn a_child_that_never_ends_goes_at_the_cap() {
        let started = Instant::now();
        let limits = Limits {
            quiet: Duration::from_secs(1),
            total: Duration::from_secs(2),
        };
        let record = scan_file(
            Path::new("/plugins/endless.vst3"),
            1,
            2,
            &progressing(None, "true"),
            limits,
        );
        let waited = started.elapsed();
        assert_eq!(record.reason.as_deref(), Some("timed out after 2 s"));
        assert!(waited < Duration::from_secs(5), "killed after {waited:?}");
        assert_eq!(seconds(CHILD_CAP), "60 s");
    }

    // Only progress lines restart the quiet time: a plugin printing its own lines every 100 ms
    // is killed at the timeout (here 0.5 s) all the same.
    #[cfg(unix)]
    #[test]
    fn other_output_does_not_keep_a_child_alive() {
        let started = Instant::now();
        let record = scan_file(
            Path::new("/plugins/chatty.clap"),
            1,
            2,
            &shell("while true; do echo 'loading...'; sleep 0.1; done"),
            quiet(Duration::from_millis(500)),
        );
        assert_eq!(record.reason.as_deref(), Some("timed out after 0.5 s"));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    // Progress lines are dropped as they come, not kept with the output: 50,000 of them (1.2 MB,
    // past the 1 MiB kept) before the result still leave the result to be read.
    #[cfg(unix)]
    #[test]
    fn progress_lines_are_not_kept() {
        let script = format!(
            r#"yes '{PROGRESS_PREFIX}1' | head -n 50000; echo '{RESULT_PREFIX}{{"plugins":[]}}'"#
        );
        assert!(50_000 * (PROGRESS_PREFIX.len() + 2) > CHILD_OUTPUT_LIMIT);
        let record = scan_file(Path::new("/x.vst3"), 0, 0, &shell(&script), SCAN);
        assert_eq!((record.status, record.reason), (FileStatus::Ok, None));
    }

    #[test]
    fn the_summary_counts_files() {
        let report = ScanReport {
            scanned: Vec::new(),
            unchanged: 2,
        };
        assert_eq!(report.summary(), "2 files: 0 scanned, 2 unchanged");
    }
}
