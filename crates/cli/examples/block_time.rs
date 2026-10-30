//! How long the engine takes per block: plays a project through `Player::process` in blocks
//! of 512 frames, as the audio callback does, times each call, and prints the mean, the 99.9th
//! percentile and the worst against the time 512 frames play at 48 kHz (10.67 ms).
//!
//! ```text
//! cargo run --release -p dawsome --example block_time -- <project.json> [--song]
//!     [--seconds N] [--cache <catalogue.json>]
//! ```
//!
//! Plugins come from the catalogue `--cache` names (made by `dawsome scan`); without one the
//! project must name none. As in an offline render, the blocks play on a thread of their own
//! while this thread runs the plugins' main-thread work. The clock is `Instant`, which on
//! Linux reads the CPU's time-stamp counter through the vDSO.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use dawsome_engine::compile::Compiler;
use dawsome_engine::edit::Touched;
use dawsome_engine::plan::PlayMode;
use dawsome_engine::processor::Registry;
use dawsome_engine::project::Project;
use dawsome_engine::realtime::{self, Player, BLOCK_FRAMES};
use dawsome_engine::transport::Control;
use dawsome_host_clap::{Catalogue, ClapHost, RenderPump};
use dawsome_media::{MediaSounds, Playback};

/// What a block may take: the time 512 frames play at 48 kHz.
const BUDGET: Duration = Duration::from_nanos(BLOCK_FRAMES as u64 * 1_000_000_000 / 48_000);

struct Options {
    project: PathBuf,
    song: bool,
    seconds: u32,
    cache: Option<PathBuf>,
}

fn options() -> Result<Options, String> {
    let mut args = std::env::args().skip(1);
    let mut options = Options {
        project: PathBuf::new(),
        song: false,
        seconds: 60,
        cache: None,
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--song" => options.song = true,
            "--seconds" => {
                options.seconds = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .filter(|&seconds| seconds > 0)
                    .ok_or("--seconds needs a whole number above 0")?;
            }
            "--cache" => options.cache = Some(args.next().ok_or("--cache needs a file")?.into()),
            _ if options.project.as_os_str().is_empty() => options.project = arg.into(),
            _ => return Err(format!("unknown argument {arg:?}")),
        }
    }
    if options.project.as_os_str().is_empty() {
        return Err(
            "usage: block_time <project.json> [--song] [--seconds N] [--cache FILE]".into(),
        );
    }
    Ok(options)
}

/// The project's player, told to play, and the plugin host it needs.
fn player(options: &Options) -> Result<(Player, Option<ClapHost>, Project), String> {
    let text = std::fs::read_to_string(&options.project).map_err(|error| error.to_string())?;
    let project = Project::load_json(&text).map_err(|error| error.to_string())?;
    let host = match (
        &options.cache,
        dawsome_host_clap::plugin_ids(&project).is_empty(),
    ) {
        (_, true) => None,
        (Some(cache), false) => {
            let catalogue = Catalogue::load(cache).map_err(|error| error.to_string())?;
            Some(ClapHost::new(catalogue))
        }
        (None, false) => return Err("the project names plugins: give --cache".into()),
    };
    let mut registry = Registry::native();
    if let Some(host) = &host {
        registry.add_host(Box::new(host.clone()));
    }
    let mode = match project.patterns.keys().next() {
        Some(&id) if !options.song => PlayMode::Pattern(id),
        _ => PlayMode::Song,
    };
    let base = options.project.parent().unwrap_or(Path::new(""));
    let mut sounds = MediaSounds::new(base, project.sample_rate, Playback::offline(None));
    let plan = Compiler::with_registry(registry)
        .compile_with(&project, &Touched::default(), mode, &mut sounds)
        .map_err(|error| format!("{error:?}"))?;
    let (mut link, player) = realtime::link(plan, project.tempo);
    link.send(Control::Play)
        .map_err(|error| error.to_string())?;
    Ok((player, host, project))
}

/// Each block's time, in order.
fn time_blocks(mut player: Player, blocks: usize) -> Vec<Duration> {
    let mut block = vec![[0.0f32; 2]; BLOCK_FRAMES];
    let mut times = Vec::with_capacity(blocks);
    for _ in 0..blocks {
        let started = Instant::now();
        player.process(&mut block);
        times.push(started.elapsed());
    }
    player.suspend_processors();
    times
}

fn micros(time: Duration) -> f64 {
    time.as_secs_f64() * 1e6
}

fn main() -> ExitCode {
    let options = match options() {
        Ok(options) => options,
        Err(error) => {
            eprintln!("block_time: {error}");
            return ExitCode::from(2);
        }
    };
    let (player, host, project) = match player(&options) {
        Ok(found) => found,
        Err(error) => {
            eprintln!("block_time: {}: {error}", options.project.display());
            return ExitCode::FAILURE;
        }
    };
    let blocks = (options.seconds as usize * project.sample_rate as usize).div_ceil(BLOCK_FRAMES);
    let mut pump = host.as_ref().map(|host| RenderPump::new(host, &project));
    let mut times = thread::scope(|scope| {
        let worker = scope.spawn(move || time_blocks(player, blocks));
        while !worker.is_finished() {
            for failure in pump.as_mut().map(RenderPump::pump).unwrap_or_default() {
                eprintln!("block_time: {failure}");
            }
            thread::sleep(Duration::from_millis(4));
        }
        worker.join()
    })
    .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
    let first = times[0];
    times.sort_unstable();
    let total: Duration = times.iter().sum();
    let mean = total / u32::try_from(times.len()).unwrap_or(u32::MAX);
    let p999 = times[(times.len() * 999 / 1000).min(times.len() - 1)];
    let worst = times[times.len() - 1];
    println!(
        "{}: {} blocks of {BLOCK_FRAMES} at {} Hz: mean {:.1} µs, p99.9 {:.1} µs, worst {:.1} µs \
         (first {:.1} µs); budget {:.0} µs at 48 kHz, headroom ×{:.0}",
        options.project.display(),
        times.len(),
        project.sample_rate,
        micros(mean),
        micros(p999),
        micros(worst),
        micros(first),
        micros(BUDGET),
        BUDGET.as_secs_f64() / worst.as_secs_f64(),
    );
    ExitCode::SUCCESS
}
