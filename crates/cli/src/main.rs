//! `dawsome`, the command line.
//! - `dawsome render <project.json> <out.wav|out.mp3> [--song] [--block N] [--bars N]
//!   [--bits 16|24|32f] [--no-dither] [--input-feed <file>] [--midi-out-log <file>]` renders
//!   pattern 1, or
//!   with `--song` the playlist, through `Player::process`, the same function live playback uses;
//!   an input channel plays `--input-feed`'s frames (the file standing in for its device), else
//!   silence; `--midi-out-log` writes the messages the MIDI outs were sent, as JSON.
//! - `dawsome scan [--plugin-path DIR].. [--cache FILE] [--force]` scans the CLAP and VST3
//!   plugin directories into the plugin catalogue, one child process (`dawsome scan --child
//!   <file>`) per plugin file or VST3 module.
//! - `dawsome smf-import <project.json> <file.mid> <out.json> [--pattern P] [--channel C]` puts
//!   a MIDI file's notes on one channel of a pattern, as the app's Import MIDI does;
//!   `dawsome smf-export <project.json> <out.mid> [--pattern P] [--channel C]` writes them out.
//! - `dawsome export-midi <project.json> <out.mid>` writes the song (the playlist, its clips
//!   unrolled) as one MIDI file, a track per channel; `dawsome export-stems <project.json> <dir>
//!   [--bits 16|24|32f] [--no-dither]` renders it once per playlist track (the others' clips
//!   muted) and once
//!   whole into `<dir>/<project> stems/`; `dawsome export-dawproject <project.json>
//!   <out.dawproject>` packs it as a DAWproject file (Bitwig Studio, Studio One, Cubase). Each
//!   export writes a report beside what it wrote (`<file>.report.txt`, printed too) naming what
//!   did not carry over.
// No unsafe code here, and none may come: the plugin hosts hold the unsafe boundary.
#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::iter;
use std::num::{NonZeroU32, NonZeroUsize};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::str::FromStr;

use dawsome_engine::compile::{CompileError, Compiler, MixerError};
use dawsome_engine::edit::{self, EditError, Touched};
use dawsome_engine::input::InputFeeds;
use dawsome_engine::midi_out;
use dawsome_engine::plan::PlayMode;
use dawsome_engine::processor::Registry;
use dawsome_engine::project::{ChannelId, ChannelKind, LoadError, PatternId, Project};
use dawsome_engine::realtime::{self, ControlRingFull, BLOCK_FRAMES};
use dawsome_engine::smf::{self, SmfError};
use dawsome_engine::transport::Control;
use dawsome_host_clap::ClapHost;
use dawsome_host_common::catalogue::{FileStatus, PluginFormat};
use dawsome_host_common::scan::{self, ChildCommand, Scanned};
use dawsome_host_common::{PluginSetup, RenderPump};
use dawsome_host_vst3::Vst3Host;
use dawsome_media::decode::{self, DecodeError};
use dawsome_media::export::{self, Report};
use dawsome_media::record::monitor_ring;
use dawsome_media::{BitDepth, Dither, Encoding, MediaSounds, Playback, Sink, WriteError};
use thiserror::Error;

const USAGE: &str = "usage: dawsome render <project.json> <out.wav|out.mp3> [--song] [--block N] [--bars N] [--bits 16|24|32f]
                      [--no-dither] [--stream-threshold FRAMES] [--plugin-path DIR].. [--cache FILE]
                      [--cache-dir DIR] [--input-feed FILE] [--midi-out-log FILE]
       dawsome scan [--plugin-path DIR].. [--cache FILE] [--force]
       dawsome smf-import <project.json> <file.mid> <out.json> [--pattern P] [--channel C]
       dawsome smf-export <project.json> <out.mid> [--pattern P] [--channel C]
       dawsome export-midi <project.json> <out.mid>
       dawsome export-stems <project.json> <dir> [--bits 16|24|32f] [--no-dither]
                            [--plugin-path DIR].. [--cache FILE] [--cache-dir DIR]
       dawsome export-dawproject <project.json> <out.dawproject>
       dawsome --version
  --song         play the playlist from tick 0 instead of looping pattern 1
  --block N      frames per processing block (default 512)
  --bars N       bars to render, counted as played: loop and song-end wraps count (default 1)
  --bits         WAV sample format: 16 or 24-bit integer, or 32-bit float (default 32f); an .mp3
                 output is 320 kbps
  --no-dither    write 16 bits by plain rounding: without it a 16-bit WAV is dithered (TPDF
                 noise of one step at most, the same bytes every run); 24-bit, 32-bit float
                 and MP3 are never dithered
  --stream-threshold  audio clip files longer than this many frames (at the project's rate)
                 stream from disk (default 30 seconds of frames); 0 streams every one
  --input-feed   the sound file every input channel plays, standing in for its device (each
                 channel takes the file's channels it names), from the render's first frame;
                 at the project's rate. Without it an input channel renders silence
  --midi-out-log  write the messages input channels with a MIDI out send to FILE, as JSON:
                 each with its output frame, rack channel and bytes, then how many note ons
                 were dropped and note offs lost; the render ends with Stop, which ends the
                 notes still sounding. No MIDI port is opened. With it --block is at most
                 4096, the messages the log's ring holds between two reads
  --plugin-path  look for CLAP and VST3 plugins here instead of the standard folders
                 (~/.clap and /usr/lib/clap, ~/.vst3, /usr/lib64/vst3, /usr/lib/vst3,
                 /usr/local/lib64/vst3 and /usr/local/lib/vst3; %COMMONPROGRAMFILES%\\CLAP,
                 %LOCALAPPDATA%\\Programs\\Common\\CLAP,
                 %LOCALAPPDATA%\\Programs\\Common\\VST3 and %COMMONPROGRAMFILES%\\VST3 on
                 Windows), $CLAP_PATH, $VST3_PATH and the plugins bundled with the program
                 (plugins/ beside it, or lib/dawsome/clap under its install prefix) (repeatable)
  --cache        the plugin catalogue file (default: dawsome/clap-catalogue.json in the user's
                 cache folder)
  --cache-dir    where stretched audio clips are kept (default: dawsome/stretch in the user's
                 cache folder)
  --force        scan every plugin file again, also unchanged ones
  --pattern, --channel  the pattern and channel a MIDI file is read into or written from, by
                 id (default: the first of each)
render writes pattern 1 (with --song, the playlist) as a WAV or MP3 file, the same audio the
app plays; scan finds the CLAP and VST3 plugins and writes the plugin catalogue; smf-import
puts a MIDI file's notes on one channel of a pattern, and smf-export writes them out as a MIDI
file.
export-midi writes the song as one MIDI file, a track per channel; export-stems writes
<dir>/<project> stems/: NN <track>.wav for each playlist track (the song with the other tracks'
clips muted) and master.wav, every one the song's whole bars; export-dawproject packs the
project as a DAWproject file, a track per channel. Each export writes a report beside what it
wrote (<file>.report.txt) naming what did not carry over, and prints it.
A sound's edits play as the project holds them, applied before a render starts; a long edited
sound is kept in dawsome/edited in the user's cache folder. The user's cache folder is
$DAWSOME_CACHE_HOME, else $XDG_CACHE_HOME or ~/.cache on Linux and %LOCALAPPDATA% on Windows.
A project's sound paths that start with dawsome: (shipped instrument sets and starter-kit
sounds) are read from dawsome/ in the user's data folder: $DAWSOME_DATA_HOME, else
$XDG_DATA_HOME or ~/.local/share on Linux and %LOCALAPPDATA% on Windows.
A project is a .dawsome or a .json file: the same JSON, read by its content.";

/// The arguments of `dawsome render`; `block` and `bars` are at least 1.
struct RenderArgs {
    project: PathBuf,
    out: PathBuf,
    block: usize,
    bars: u32,
    /// Play the playlist rather than pattern 1. A flag, not a project field, because the play
    /// mode is playback state and is never saved.
    song: bool,
    encoding: Encoding,
    plugins: PluginArgs,
    /// Audio clip files longer than this stream; `None` is the default, 30 s of frames.
    stream_threshold: Option<u64>,
    /// Where stretched clips are kept; `None` is the user's cache folder.
    cache_dir: Option<PathBuf>,
    /// The sound file the input channels play; `None`: they render silence.
    input_feed: Option<PathBuf>,
    /// Where the MIDI outs' messages are written; `None`: nowhere.
    midi_out_log: Option<PathBuf>,
}

/// The arguments of `dawsome smf-import` and `dawsome smf-export`: the project, the other
/// paths in order, and the pattern and channel (the first of each when not given).
struct SmfArgs {
    project: PathBuf,
    paths: Vec<PathBuf>,
    pattern: Option<PatternId>,
    channel: Option<ChannelId>,
}

/// The arguments of an export: the project and where the export goes.
struct ExportArgs {
    project: PathBuf,
    out: PathBuf,
}

/// The arguments of `dawsome export-stems`: the project, the folder the stems folder goes in,
/// and how the files render.
struct StemsArgs {
    project: PathBuf,
    dir: PathBuf,
    bits: BitDepth,
    dither: Dither,
    plugins: PluginArgs,
    cache_dir: Option<PathBuf>,
}

/// One render: `project` (its sound paths relative to `base`, read from `project_path`)
/// playing `mode` for `bars` played bars into `out`.
struct RenderJob<'a> {
    project: &'a Project,
    project_path: &'a Path,
    base: &'a Path,
    out: &'a Path,
    mode: PlayMode,
    bars: u32,
    block: usize,
    encoding: Encoding,
    plugins: &'a PluginArgs,
    stream_threshold: Option<u64>,
    cache_dir: Option<PathBuf>,
    feeds: InputFeeds,
    /// Where the MIDI outs' messages are written; `None`: nowhere.
    midi_out_log: Option<&'a Path>,
}

/// Where plugins are looked for and the catalogue is kept, from `--plugin-path` and `--cache`.
#[derive(Default)]
struct PluginArgs {
    dirs: Vec<PathBuf>,
    cache: Option<PathBuf>,
}

impl PluginArgs {
    /// Takes `arg` (and its value) when it is a plugin option; returns whether it was.
    fn take(
        &mut self,
        arg: &OsString,
        args: &mut impl Iterator<Item = OsString>,
    ) -> Result<bool, CliError> {
        if arg == "--plugin-path" {
            let dir = args
                .next()
                .ok_or_else(|| usage("--plugin-path needs a folder"))?;
            self.dirs.push(PathBuf::from(dir));
        } else if arg == "--cache" {
            let file = args.next().ok_or_else(|| usage("--cache needs a file"))?;
            if self.cache.replace(PathBuf::from(file)).is_some() {
                return Err(usage("--cache was given twice"));
            }
        } else {
            return Ok(false);
        }
        Ok(true)
    }

    /// The setup these options give, for CLAP and VST3, with this binary as the scan child.
    fn setup(&self) -> Result<PluginSetup, CliError> {
        let program = std::env::current_exe().map_err(CliError::OwnProgram)?;
        let child = ChildCommand {
            program,
            args: vec!["scan".into(), "--child".into()],
        };
        PluginSetup::for_formats(
            PluginFormat::ALL,
            self.dirs.clone(),
            self.cache.clone(),
            child,
        )
        .ok_or(CliError::NoCacheDir)
    }
}

/// The arguments of `dawsome scan`.
struct ScanArgs {
    plugins: PluginArgs,
    force: bool,
}

/// Everything that can stop a command. Each message says what failed and on which file.
#[derive(Debug, Error)]
enum CliError {
    #[error("{0}\n{usage}", usage = USAGE)]
    Usage(String),
    #[error("cannot read {}: {source}", .path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("cannot find this program's own file, which scans plugins: {0}")]
    OwnProgram(io::Error),
    #[error("project {}: {source}", .path.display())]
    LoadProject { path: PathBuf, source: LoadError },
    #[error("refusing to write {}: it is also an input file of this project", .path.display())]
    OutputIsInput { path: PathBuf },
    #[error("--bars {bars} is too long for one WAV file (4 GiB at most)")]
    TooLong { bars: u32 },
    #[error("cannot write {}: {source}", .path.display())]
    Write { path: PathBuf, source: WriteError },
    #[error("cannot write {}: {source}", .path.display())]
    WriteFile { path: PathBuf, source: io::Error },
    #[error("the player refused a transport command: {0}")]
    Control(#[from] ControlRingFull),
    #[error("project {}: {source}", .path.display())]
    Mixer { path: PathBuf, source: MixerError },
    #[error(
        "there is no cache folder for the plugin catalogue: set DAWSOME_CACHE_HOME or pass --cache"
    )]
    NoCacheDir,
    #[error(transparent)]
    Scan(#[from] scan::UnsavedCatalogue),
    #[error("cannot start the render thread: {0}")]
    Thread(io::Error),
    #[error("{}: {source}", .path.display())]
    Smf { path: PathBuf, source: SmfError },
    #[error("the import was refused: {0}")]
    Edit(#[from] EditError),
    #[error("project {}: it has no pattern or no channel", .path.display())]
    NothingToTarget { path: PathBuf },
    #[error("project {}: the song is empty: no clip on the playlist", .path.display())]
    EmptySong { path: PathBuf },
    #[error("cannot make the folder {}: {source}", .path.display())]
    Folder { path: PathBuf, source: io::Error },
    #[error("cannot read the input feed {}: {source}", .path.display())]
    Feed { path: PathBuf, source: DecodeError },
    #[error("the input feed {} is at {rate} Hz and the project at {project} Hz; a feed plays at the project's rate (it is not resampled)", .path.display())]
    FeedRate {
        path: PathBuf,
        rate: u32,
        project: u32,
    },
}

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let result = match args.next() {
        Some(command) if command == "render" => {
            parse_render_args(args).and_then(|args| render(&args))
        }
        Some(command) if command == "smf-import" => parse_smf_args(
            args,
            3,
            "smf-import takes 3 paths (project, MIDI file, output)",
        )
        .and_then(|args| smf_import(&args)),
        Some(command) if command == "smf-export" => {
            parse_smf_args(args, 2, "smf-export takes 2 paths (project, output)")
                .and_then(|args| smf_export(&args))
        }
        Some(command) if command == "export-midi" => {
            parse_export_args(args, "export-midi").and_then(|args| export_midi(&args))
        }
        Some(command) if command == "export-stems" => {
            parse_stems_args(args).and_then(|args| export_stems(&args))
        }
        Some(command) if command == "export-dawproject" => {
            parse_export_args(args, "export-dawproject").and_then(|args| export_dawproject(&args))
        }
        Some(command) if command == "scan" => {
            let mut args = args.peekable();
            if args.next_if(|arg| arg == "--child").is_some() {
                return match (args.next(), args.next()) {
                    (Some(file), None) => {
                        let file = Path::new(&file);
                        // The child loads the file as its format's host does.
                        let code = match PluginFormat::of_path(file) {
                            Some(PluginFormat::Vst3) => dawsome_host_vst3::scan::run_child(file),
                            _ => dawsome_host_clap::scan::run_child(file),
                        };
                        ExitCode::from(u8::try_from(code).unwrap_or(1))
                    }
                    _ => {
                        eprintln!("dawsome: scan --child takes one plugin file");
                        ExitCode::from(2)
                    }
                };
            }
            parse_scan_args(args).and_then(|args| run_scan(&args))
        }
        Some(flag) if flag == "--version" => {
            println!("dawsome {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Some(flag) if flag == "--help" || flag == "-h" => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some(other) => Err(CliError::Usage(format!(
            "unknown command '{}'",
            other.to_string_lossy()
        ))),
        None => Err(CliError::Usage("no command given".into())),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("dawsome: {error}");
            // The usual convention: 2 for a mistyped command line, 1 for every other failure.
            ExitCode::from(if matches!(error, CliError::Usage(_)) {
                2
            } else {
                1
            })
        }
    }
}

fn parse_render_args(mut args: impl Iterator<Item = OsString>) -> Result<RenderArgs, CliError> {
    let mut paths = Vec::new();
    let mut block = 512;
    let mut bars = 1;
    let mut song = false;
    let mut bits = None;
    let mut dither = Dither::Tpdf;
    let mut plugins = PluginArgs::default();
    let mut stream_threshold = None;
    let mut cache_dir = None;
    let mut input_feed = None;
    let mut midi_out_log = None;
    while let Some(arg) = args.next() {
        if plugins.take(&arg, &mut args)? {
            continue;
        }
        if arg == "--input-feed" {
            let file = args
                .next()
                .ok_or_else(|| usage("--input-feed needs a sound file"))?;
            if input_feed.replace(PathBuf::from(file)).is_some() {
                return Err(usage("--input-feed was given twice"));
            }
        } else if arg == "--midi-out-log" {
            let file = args
                .next()
                .ok_or_else(|| usage("--midi-out-log needs a file"))?;
            if midi_out_log.replace(PathBuf::from(file)).is_some() {
                return Err(usage("--midi-out-log was given twice"));
            }
        } else if arg == "--cache-dir" {
            let dir = args
                .next()
                .ok_or_else(|| usage("--cache-dir needs a folder"))?;
            if cache_dir.replace(PathBuf::from(dir)).is_some() {
                return Err(usage("--cache-dir was given twice"));
            }
        } else if arg == "--stream-threshold" {
            stream_threshold = Some(whole::<u64>("--stream-threshold", args.next())?);
        } else if arg == "--song" {
            song = true;
        } else if arg == "--block" {
            block = positive::<NonZeroUsize>("--block", args.next())?.get();
        } else if arg == "--bars" {
            bars = positive::<NonZeroU32>("--bars", args.next())?.get();
        } else if arg == "--bits" {
            bits = Some(bit_depth(args.next())?);
        } else if arg == "--no-dither" {
            dither = Dither::Off;
        } else if arg.to_string_lossy().starts_with('-') {
            return Err(usage(&format!(
                "unknown option '{}'",
                arg.to_string_lossy()
            )));
        } else {
            paths.push(PathBuf::from(arg));
        }
    }
    let [project, out] = <[PathBuf; 2]>::try_from(paths).map_err(|paths| {
        usage(&format!(
            "render takes 2 paths (project, output), got {}",
            paths.len()
        ))
    })?;
    // The log reads the MIDI-out ring after each call: a longer call could fill it and drop
    // notes, and the log would then depend on the block size.
    if midi_out_log.is_some() && block > midi_out::RING_CAPACITY {
        return Err(usage(&format!(
            "--block is at most {} with --midi-out-log, the messages its ring holds between two reads",
            midi_out::RING_CAPACITY
        )));
    }
    // 32-bit float is the default: the format every render had before depths existed.
    // Why --no-dither is taken whatever the output: it asks for no noise, which a format that
    // is never dithered already has, so one script line serves every format.
    let encoding = Encoding::for_path(&out, bits.unwrap_or(BitDepth::Float32), dither);
    if encoding == Encoding::Mp3 && bits.is_some() {
        return Err(usage("--bits sets a WAV format; an .mp3 output has none"));
    }
    Ok(RenderArgs {
        project,
        out,
        block,
        bars,
        song,
        encoding,
        plugins,
        stream_threshold,
        cache_dir,
        input_feed,
        midi_out_log,
    })
}

fn parse_smf_args(
    mut args: impl Iterator<Item = OsString>,
    paths: usize,
    wrong_count: &str,
) -> Result<SmfArgs, CliError> {
    let mut found = Vec::new();
    let (mut pattern, mut channel) = (None, None);
    while let Some(arg) = args.next() {
        if arg == "--pattern" {
            let id = positive::<NonZeroU32>("--pattern", args.next())?.get();
            pattern = Some(PatternId(id));
        } else if arg == "--channel" {
            let id = positive::<NonZeroU32>("--channel", args.next())?.get();
            channel = Some(ChannelId(id));
        } else if arg.to_string_lossy().starts_with('-') {
            return Err(usage(&format!(
                "unknown option '{}'",
                arg.to_string_lossy()
            )));
        } else {
            found.push(PathBuf::from(arg));
        }
    }
    if found.len() != paths {
        return Err(usage(wrong_count));
    }
    let project = found.remove(0);
    Ok(SmfArgs {
        project,
        paths: found,
        pattern,
        channel,
    })
}

fn parse_export_args(
    args: impl Iterator<Item = OsString>,
    command: &str,
) -> Result<ExportArgs, CliError> {
    let mut paths = Vec::new();
    for arg in args {
        if arg.to_string_lossy().starts_with('-') {
            return Err(usage(&format!(
                "unknown option '{}'",
                arg.to_string_lossy()
            )));
        }
        paths.push(PathBuf::from(arg));
    }
    let [project, out] = <[PathBuf; 2]>::try_from(paths).map_err(|paths| {
        usage(&format!(
            "{command} takes 2 paths (project, output), got {}",
            paths.len()
        ))
    })?;
    Ok(ExportArgs { project, out })
}

fn parse_stems_args(mut args: impl Iterator<Item = OsString>) -> Result<StemsArgs, CliError> {
    let mut paths = Vec::new();
    let mut bits = None;
    let mut dither = Dither::Tpdf;
    let mut plugins = PluginArgs::default();
    let mut cache_dir = None;
    while let Some(arg) = args.next() {
        if plugins.take(&arg, &mut args)? {
            continue;
        }
        if arg == "--bits" {
            bits = Some(bit_depth(args.next())?);
        } else if arg == "--no-dither" {
            dither = Dither::Off;
        } else if arg == "--cache-dir" {
            let dir = args
                .next()
                .ok_or_else(|| usage("--cache-dir needs a folder"))?;
            if cache_dir.replace(PathBuf::from(dir)).is_some() {
                return Err(usage("--cache-dir was given twice"));
            }
        } else if arg.to_string_lossy().starts_with('-') {
            return Err(usage(&format!(
                "unknown option '{}'",
                arg.to_string_lossy()
            )));
        } else {
            paths.push(PathBuf::from(arg));
        }
    }
    let [project, dir] = <[PathBuf; 2]>::try_from(paths).map_err(|paths| {
        usage(&format!(
            "export-stems takes 2 paths (project, folder), got {}",
            paths.len()
        ))
    })?;
    Ok(StemsArgs {
        project,
        dir,
        // 32-bit float, as a render writes unless told otherwise.
        bits: bits.unwrap_or(BitDepth::Float32),
        dither,
        plugins,
        cache_dir,
    })
}

/// The value after `--bits` as a WAV sample format.
fn bit_depth(value: Option<OsString>) -> Result<BitDepth, CliError> {
    let value = value.ok_or_else(|| usage("--bits needs a value"))?;
    value.to_str().and_then(BitDepth::parse).ok_or_else(|| {
        let shown = value.to_string_lossy();
        usage(&format!("--bits expects 16, 24 or 32f, got '{shown}'"))
    })
}

fn parse_scan_args(mut args: impl Iterator<Item = OsString>) -> Result<ScanArgs, CliError> {
    let mut plugins = PluginArgs::default();
    let mut force = false;
    while let Some(arg) = args.next() {
        if plugins.take(&arg, &mut args)? {
            continue;
        }
        if arg == "--force" {
            force = true;
        } else {
            return Err(usage(&format!(
                "unknown argument '{}'",
                arg.to_string_lossy()
            )));
        }
    }
    Ok(ScanArgs { plugins, force })
}

/// Prints what a scan loaded this time, one line a file, then the counts.
fn print_scan(scanned: &Scanned) {
    for record in &scanned.report.scanned {
        let file = record.file.display();
        match (record.status, &record.reason) {
            (FileStatus::Failed, Some(reason)) => println!("scanned {file}: failed ({reason})"),
            _ => println!("scanned {file}: {} plugins", record.plugins.len()),
        }
    }
    println!("{}", scanned.report.summary());
}

fn run_scan(args: &ScanArgs) -> Result<(), CliError> {
    let scanned = args.plugins.setup()?.scan(args.force);
    print_scan(&scanned);
    match scanned.unsaved {
        Some(unsaved) => Err(unsaved.into()),
        None => Ok(()),
    }
}

/// The plugin hosts a project needs: CLAP's when it names CLAP plugins, VST3's when it names
/// VST3 ones, over one catalogue.
#[derive(Default)]
struct Hosts {
    clap: Option<ClapHost>,
    vst3: Option<Vst3Host>,
}

impl Hosts {
    fn is_empty(&self) -> bool {
        self.clap.is_none() && self.vst3.is_none()
    }

    /// Each host, for the registry or the pump.
    fn boxed(&self) -> Vec<Box<dyn dawsome_engine::processor::ProcessorHost>> {
        let mut hosts: Vec<Box<dyn dawsome_engine::processor::ProcessorHost>> = Vec::new();
        if let Some(clap) = &self.clap {
            hosts.push(Box::new(clap.clone()));
        }
        if let Some(vst3) = &self.vst3 {
            hosts.push(Box::new(vst3.clone()));
        }
        hosts
    }
}

/// The hosts for `project`'s plugins, with their catalogue, after a scan when it lacks one of
/// them. What the scan reports goes to stderr.
fn plugin_hosts(project: &Project, plugins: &PluginArgs) -> Result<Hosts, CliError> {
    let clap = dawsome_host_clap::plugin_ids(project);
    let vst3 = dawsome_host_vst3::plugin_ids(project);
    if clap.is_empty() && vst3.is_empty() {
        return Ok(Hosts::default());
    }
    let wanted = clap
        .iter()
        .map(|id| (PluginFormat::Clap, id.clone()))
        .chain(
            vst3.iter()
                .map(|id| (PluginFormat::Vst3, canonical_vst3(id))),
        )
        .collect();
    let (catalogue, scanned) = plugins.setup()?.catalogue_for_plugins(&wanted);
    if let Some(unsaved) = scanned.and_then(|scanned| scanned.unsaved) {
        eprintln!("dawsome: warning: {unsaved}");
    }
    Ok(Hosts {
        clap: (!clap.is_empty()).then(|| ClapHost::new(catalogue.clone())),
        vst3: (!vst3.is_empty()).then(|| Vst3Host::new(catalogue)),
    })
}

/// A VST3 class id as the catalogue writes it (upper-case hex), whatever case a project wrote.
fn canonical_vst3(id: &str) -> String {
    dawsome_host_vst3::parse_class_id(id).map_or_else(
        || id.to_owned(),
        |cid| dawsome_host_vst3::class_id_text(&cid),
    )
}

/// The plugins' main-thread work while a render runs; each new failure goes to stderr as one
/// write, so a plugin printing from its own threads cannot split the line.
fn pump(pump: Option<&mut RenderPump>) {
    for failure in pump.map(RenderPump::pump).unwrap_or_default() {
        let line = format!("dawsome: warning: {failure}\n");
        let _ = io::stderr().write_all(line.as_bytes());
    }
}

fn usage(message: &str) -> CliError {
    CliError::Usage(message.to_owned())
}

/// Parses the value after `flag` as a whole number, 0 included.
fn whole<T: FromStr>(flag: &str, value: Option<OsString>) -> Result<T, CliError> {
    let value = value.ok_or_else(|| usage(&format!("{flag} needs a value")))?;
    value
        .to_str()
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| {
            let shown = value.to_string_lossy();
            usage(&format!("{flag} expects a whole number, got '{shown}'"))
        })
}

/// Parses the value after `flag` as a whole number above 0.
fn positive<T: FromStr>(flag: &str, value: Option<OsString>) -> Result<T, CliError> {
    let value = value.ok_or_else(|| usage(&format!("{flag} needs a value")))?;
    value
        .to_str()
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| {
            let shown = value.to_string_lossy();
            usage(&format!(
                "{flag} expects a whole number above 0, got '{shown}'"
            ))
        })
}

fn render(args: &RenderArgs) -> Result<(), CliError> {
    let project = load_project(&args.project)?;

    // Sound paths in the project are relative to the project file (absolute ones as they are,
    // shipped ones in the data folder).
    let base = args.project.parent().unwrap_or(Path::new(""));
    let inputs = || {
        iter::once(args.project.clone())
            .chain(args.input_feed.clone())
            .chain(dawsome_media::input_files(&project, base))
    };
    for out in iter::once(&args.out).chain(&args.midi_out_log) {
        if let Some(path) = dawsome_media::overwritten_input(out, inputs()) {
            return Err(CliError::OutputIsInput { path });
        }
    }
    let feeds = match &args.input_feed {
        Some(path) => input_feeds(&project, path)?,
        None => InputFeeds::new(),
    };
    // Pattern 1 is the lowest pattern ID. A project without patterns has nothing a clip could
    // play either, so song mode renders the same silence.
    let mode = match project.patterns.keys().next() {
        Some(&id) if !args.song => PlayMode::Pattern(id),
        _ => PlayMode::Song,
    };
    render_project(RenderJob {
        project: &project,
        project_path: &args.project,
        base,
        out: &args.out,
        mode,
        bars: args.bars,
        block: args.block,
        encoding: args.encoding,
        plugins: &args.plugins,
        stream_threshold: args.stream_threshold,
        cache_dir: args.cache_dir.clone(),
        feeds,
        midi_out_log: args.midi_out_log.as_deref(),
    })
}

/// Renders `job` through `Player::process`, the same function live playback uses, its plugins'
/// main-thread work served here while a render thread plays.
fn render_project(job: RenderJob<'_>) -> Result<(), CliError> {
    let RenderJob {
        project,
        project_path,
        base,
        out,
        mode,
        bars,
        block,
        encoding,
        plugins,
        stream_threshold,
        cache_dir,
        feeds,
        midi_out_log,
    } = job;
    let sample_rate = project.sample_rate;
    let hosts = plugin_hosts(project, plugins)?;
    let mut registry = Registry::native();
    for host in hosts.boxed() {
        registry.add_host(host);
    }
    let mut compiler = Compiler::with_registry(registry);
    // An offline render waits for the disk, so streamed clips render the bytes of memory; a
    // stretched clip is made (or found in the cache) before the render starts.
    let cache_dir = cache_dir.or_else(dawsome_media::stretch_dir);
    // A sound's edits are applied before it starts too, a long edited sound kept in the user's
    // cache folder (dawsome/edited) and streamed from there.
    let playback = Playback::offline(stream_threshold)
        .stretching_into(cache_dir)
        .editing_into(dawsome_media::edited_dir());
    let mut sounds = MediaSounds::new(base, sample_rate, playback);
    let compiled = compiler.compile_with(project, &Touched::default(), mode, &mut sounds);
    for notice in sounds.take_notices() {
        eprintln!("dawsome: warning: {notice}");
    }
    let plan = compiled.map_err(|error| match error {
        CompileError::Load(never) => match never {},
        CompileError::Mixer(source) => CliError::Mixer {
            path: project_path.to_path_buf(),
            source,
        },
    })?;
    for notice in compiler.take_notices() {
        eprintln!("dawsome: warning: {notice}");
    }
    if !hosts.is_empty() && block != BLOCK_FRAMES {
        eprintln!(
            "dawsome: warning: plugin output may depend on the block size (--block {block}; the default is {BLOCK_FRAMES})"
        );
    }
    // The same link live playback uses, given its one plan (and the input feeds) up front.
    let (mut link, player) = realtime::link(plan, project.tempo);
    if !feeds.is_empty() && link.set_input_feeds(feeds).is_err() {
        unreachable!("a new link's feed ring is empty");
    }
    link.send(Control::Play)?;
    // The log's messages, read from the MIDI-out ring after every call.
    let mut midi_out = midi_out_log.and_then(|_| link.take_midi_out());
    let mut sent = Vec::new();

    let frames = dawsome_media::frames_for_bars(&player, bars, project.ppq, encoding)
        .ok_or(CliError::TooLong { bars })?;
    let mut failures = (!hosts.is_empty()).then(|| RenderPump::of(hosts.boxed(), project));
    for note in failures.iter().flat_map(RenderPump::latencies) {
        eprintln!("dawsome: note: {note}");
    }
    let write_error = |source: WriteError| CliError::Write {
        path: out.to_path_buf(),
        source,
    };
    let sink = Sink::create(out, sample_rate, encoding).map_err(write_error)?;
    let logging = midi_out.is_some();
    let mut stopped = Ok(());
    let rendered = dawsome_media::render_on_thread_then(
        player,
        frames,
        block,
        sink,
        &mut || {
            if let Some(reader) = midi_out.as_mut() {
                reader.read_into(&mut sent);
            }
        },
        // A logged render ends as playback does: Stop, taken by one more call, ends the notes
        // still sounding on their MIDI outs, on the render's last frame.
        &mut |player| {
            if logging {
                stopped = link.send(Control::Stop);
                player.process(&mut []);
            }
        },
        &mut || pump(failures.as_mut()),
        &mut |_| true,
    )
    .map_err(CliError::Thread)?;
    rendered.result.map_err(write_error)?;
    rendered.sink.finish().map_err(write_error)?;
    stopped?;
    if let (Some(path), Some(reader)) = (midi_out_log, midi_out.as_mut()) {
        reader.read_into(&mut sent);
        write_file(path, reader.log(&sent).as_bytes())?;
    }
    // The processors drop here, on the plugins' main thread; their instances go with the host.
    drop(rendered.player);
    pump(failures.as_mut());
    Ok(())
}

/// The feeds the input channels of `project` play: the sound file at `path` standing in for
/// their device, each channel taking the file's channels it names, from the render's first
/// frame. The file is pushed through the input callback's ring writer, as a device's buffer
/// would be, into a monitor ring that holds it whole.
fn input_feeds(project: &Project, path: &Path) -> Result<InputFeeds, CliError> {
    let decoded = decode::decode(path).map_err(|source| CliError::Feed {
        path: path.to_owned(),
        source,
    })?;
    if decoded.sample_rate != project.sample_rate {
        return Err(CliError::FeedRate {
            path: path.to_owned(),
            rate: decoded.sample_rate,
            project: project.sample_rate,
        });
    }
    let mut feeds = InputFeeds::new();
    for (&id, channel) in &project.channels {
        let Some(input) = channel
            .input
            .as_ref()
            .filter(|_| channel.kind == ChannelKind::Input)
        else {
            continue;
        };
        let (mut writer, feed, _) = monitor_ring(input.channels, decoded.frames(), None);
        writer.push(&decoded.samples, decoded.channels, Some(0));
        feeds.insert(id, Box::new(feed));
    }
    Ok(feeds)
}

/// Reads the project at `path`.
fn load_project(path: &Path) -> Result<Project, CliError> {
    let text = fs::read_to_string(path).map_err(|source| CliError::Read {
        path: path.to_owned(),
        source,
    })?;
    Project::load_json(&text).map_err(|source| CliError::LoadProject {
        path: path.to_owned(),
        source,
    })
}

/// The pattern and channel `args` names, else the project's first of each.
fn smf_target(args: &SmfArgs, project: &Project) -> Result<(PatternId, ChannelId), CliError> {
    let pattern = args
        .pattern
        .or_else(|| project.patterns.keys().next().copied());
    let channel = args
        .channel
        .or_else(|| project.channels.keys().next().copied());
    pattern
        .zip(channel)
        .ok_or_else(|| CliError::NothingToTarget {
            path: args.project.clone(),
        })
}

fn smf_import(args: &SmfArgs) -> Result<(), CliError> {
    let [mid, out] = [&args.paths[0], &args.paths[1]];
    if let Some(path) = dawsome_media::overwritten_input(out, iter::once(mid.clone())) {
        return Err(CliError::OutputIsInput { path });
    }
    let mut project = load_project(&args.project)?;
    let (pattern, channel) = smf_target(args, &project)?;
    let bytes = fs::read(mid).map_err(|source| CliError::Read {
        path: mid.clone(),
        source,
    })?;
    let imported =
        smf::import_into(&project, pattern, channel, &bytes).map_err(|source| CliError::Smf {
            path: mid.clone(),
            source,
        })?;
    edit::apply(&mut project, imported.edit)?;
    // Sample paths are relative to the project file; written elsewhere, they are rebased.
    let from = args.project.parent().unwrap_or(Path::new(""));
    dawsome_media::rebase_samples(&mut project, from, out.parent().unwrap_or(Path::new("")));
    let mut json = project.to_json().map_err(|error| CliError::WriteFile {
        path: out.clone(),
        source: io::Error::other(error),
    })?;
    json.push('\n');
    write_file(out, json.as_bytes())?;
    eprintln!("{}", imported.report.summary());
    Ok(())
}

fn smf_export(args: &SmfArgs) -> Result<(), CliError> {
    let out = &args.paths[0];
    if let Some(path) = dawsome_media::overwritten_input(out, iter::once(args.project.clone())) {
        return Err(CliError::OutputIsInput { path });
    }
    let project = load_project(&args.project)?;
    let (pattern, channel) = smf_target(args, &project)?;
    let bytes = smf::export(&project, pattern, channel).map_err(|source| CliError::Smf {
        path: out.clone(),
        source,
    })?;
    write_file(out, &bytes)
}

/// Refuses to write `written` when it is the project file or one of its sound files (sample
/// paths relative to `base`), or the report beside it would be.
fn refuse_overwriting(
    written: &Path,
    project_path: &Path,
    project: &Project,
    base: &Path,
) -> Result<(), CliError> {
    let report = Report {
        exported: written.to_path_buf(),
        lines: Vec::new(),
    }
    .path();
    for out in [written, report.as_path()] {
        let inputs =
            iter::once(project_path.to_path_buf()).chain(dawsome_media::input_files(project, base));
        if let Some(path) = dawsome_media::overwritten_input(out, inputs) {
            return Err(CliError::OutputIsInput { path });
        }
    }
    Ok(())
}

/// Writes `report` beside what the export wrote and prints it.
fn finish_report(report: &Report) -> Result<(), CliError> {
    let text = report.text();
    write_file(&report.path(), text.as_bytes())?;
    eprint!("{text}");
    Ok(())
}

fn export_midi(args: &ExportArgs) -> Result<(), CliError> {
    let project = load_project(&args.project)?;
    let base = args.project.parent().unwrap_or(Path::new(""));
    refuse_overwriting(&args.out, &args.project, &project, base)?;
    let (bytes, lines) = export::song_midi(&project).map_err(|source| CliError::Smf {
        path: args.out.clone(),
        source,
    })?;
    write_file(&args.out, &bytes)?;
    finish_report(&Report {
        exported: args.out.clone(),
        lines,
    })
}

fn export_stems(args: &StemsArgs) -> Result<(), CliError> {
    let project = load_project(&args.project)?;
    let base = args.project.parent().unwrap_or(Path::new(""));
    let stems = export::stems(&project).ok_or_else(|| CliError::EmptySong {
        path: args.project.clone(),
    })?;
    let folder = export::stems_folder(&args.dir, &project_name(&args.project));
    fs::create_dir_all(&folder).map_err(|source| CliError::Folder {
        path: folder.clone(),
        source,
    })?;
    for file in &stems.files {
        let out = folder.join(&file.name);
        refuse_overwriting(&out, &args.project, &project, base)?;
        render_project(RenderJob {
            project: &file.project,
            project_path: &args.project,
            base,
            out: &out,
            mode: PlayMode::Song,
            bars: stems.bars,
            block: BLOCK_FRAMES,
            encoding: Encoding::Wav(args.bits, args.dither),
            plugins: &args.plugins,
            stream_threshold: None,
            cache_dir: args.cache_dir.clone(),
            feeds: InputFeeds::new(),
            midi_out_log: None,
        })?;
    }
    finish_report(&Report {
        exported: folder,
        lines: export::stems_lines(&project),
    })
}

fn export_dawproject(args: &ExportArgs) -> Result<(), CliError> {
    let project = load_project(&args.project)?;
    let base = args.project.parent().unwrap_or(Path::new(""));
    refuse_overwriting(&args.out, &args.project, &project, base)?;
    let title = project_name(&args.project);
    let lines = export::dawproject::write(&project, base, &title, &args.out).map_err(|source| {
        CliError::WriteFile {
            path: args.out.clone(),
            source,
        }
    })?;
    finish_report(&Report {
        exported: args.out.clone(),
        lines,
    })
}

/// The project's name: its file's name without the extension.
fn project_name(path: &Path) -> String {
    path.file_stem().map_or_else(
        || "Untitled".to_owned(),
        |stem| stem.to_string_lossy().into_owned(),
    )
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), CliError> {
    fs::write(path, bytes).map_err(|source| CliError::WriteFile {
        path: path.to_owned(),
        source,
    })
}
