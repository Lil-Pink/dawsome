//! The offline render loop shared by `dawsome render`, `--play-to` and the app's export: the
//! player renders block after block into a file, on a render thread of its own.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use dawsome_engine::realtime::Player;
use dawsome_engine::time;

use crate::write::{Encoding, Sink, WriteError};

/// Frames from the start of play to where bar `bars` (counting from 0) starts, if one file of
/// `encoding` holds that many. In song mode these are played bars: a wrap moves the playhead,
/// not the length of the file.
pub fn frames_for_bars(player: &Player, bars: u32, ppq: u16, encoding: Encoding) -> Option<u64> {
    player
        .tick_to_sample(time::bars_to_ticks(bars, ppq))
        .filter(|&frames| frames <= encoding.max_frames())
}

/// Renders `frames` frames through `player` into `sink`, `block` frames per call (fewer for the
/// last), calling `progress` with the frames done after each block. Returns `Ok(false)` as soon
/// as `progress` returns false; the sink is then left unfinished for the caller to drop.
pub fn render(
    player: &mut Player,
    frames: u64,
    block: usize,
    sink: &mut Sink,
    progress: &mut dyn FnMut(u64) -> bool,
) -> Result<bool, WriteError> {
    // Never longer than the render itself, so a huge block size does not allocate unused memory.
    let block_len = usize::try_from(frames).map_or(block, |frames| frames.min(block));
    let mut buffer = vec![[0.0f32; 2]; block_len];
    let mut done = 0;
    while done < frames {
        let len =
            usize::try_from(frames - done).map_or(buffer.len(), |left| left.min(buffer.len()));
        let out = &mut buffer[..len];
        player.process(out);
        sink.write(out)?;
        done += len as u64;
        if !progress(done) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// How long the calling thread of [`render_on_thread`] waits between its rounds of `pump`.
const PUMP_PERIOD: Duration = Duration::from_millis(4);

/// What [`render_on_thread`] hands back: the player and the sink (to finish or drop where they
/// were made), and the render's outcome as [`render`] gives it.
pub struct Rendered {
    pub player: Player,
    pub sink: Sink,
    pub result: Result<bool, WriteError>,
}

/// [`render`], with the player and the sink on a render thread of their own while the calling
/// thread runs `pump` every few milliseconds and reports the frames done to `progress`; a
/// `false` from it stops the render after the block in flight, and it is not asked again.
/// `after_block` runs on the render thread after every block (an offline recording drains the
/// record tap there: the render outruns any thread draining it on a timer). The player's
/// processors are suspended on the render thread when it ends.
///
/// Why a thread: the calling thread is the plugins' main thread, where their timers and
/// callbacks (`pump`) must keep running during the render, and some plugins check that the
/// thread calling them to process audio is not their main thread. An offline render then runs
/// plugins exactly as live playback does.
pub fn render_on_thread(
    player: Player,
    frames: u64,
    block: usize,
    sink: Sink,
    after_block: &mut (dyn FnMut() + Send),
    pump: &mut dyn FnMut(),
    progress: &mut dyn FnMut(u64) -> bool,
) -> Result<Rendered, io::Error> {
    render_on_thread_then(
        player,
        frames,
        block,
        sink,
        after_block,
        &mut |_| {},
        pump,
        progress,
    )
}

/// [`render_on_thread`], with `then` called on the render thread once the blocks are done,
/// before the processors are suspended: where a render that ends with a control makes its last
/// `process` call (the MIDI-out log's Stop), on the thread every other call came from.
#[expect(
    clippy::too_many_arguments,
    reason = "render_on_thread's arguments and the one call after its blocks; a struct would \
              only be taken apart again here"
)]
pub fn render_on_thread_then(
    player: Player,
    frames: u64,
    block: usize,
    sink: Sink,
    after_block: &mut (dyn FnMut() + Send),
    then: &mut (dyn FnMut(&mut Player) + Send),
    pump: &mut dyn FnMut(),
    progress: &mut dyn FnMut(u64) -> bool,
) -> Result<Rendered, io::Error> {
    let (done, stop) = (AtomicU64::new(0), AtomicBool::new(false));
    let caller = thread::current();
    thread::scope(|scope| {
        let (done, stop, caller) = (&done, &stop, &caller);
        let worker = thread::Builder::new()
            .name("dawsome-render".into())
            .spawn_scoped(scope, move || {
                let (mut player, mut sink) = (player, sink);
                let result = render(&mut player, frames, block, &mut sink, &mut |frames| {
                    after_block();
                    done.store(frames, Ordering::Relaxed);
                    !stop.load(Ordering::Relaxed)
                });
                then(&mut player);
                player.suspend_processors();
                caller.unpark();
                Rendered {
                    player,
                    sink,
                    result,
                }
            })?;
        loop {
            pump();
            let finished = worker.is_finished();
            // Once told to stop, `progress` is not asked again.
            if !stop.load(Ordering::Relaxed) && !progress(done.load(Ordering::Relaxed)) {
                stop.store(true, Ordering::Relaxed);
            }
            if finished {
                break;
            }
            thread::park_timeout(PUMP_PERIOD);
        }
        // A panic on the render thread is a bug; it goes on here as if the render had run here.
        Ok(worker
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic)))
    })
}

/// The first of `inputs` that is the same file as `out`. Why check: creating the output
/// truncates it, which would destroy the project or a sample. Paths are compared after resolving
/// links and `..`. An output that does not exist yet cannot be an input; an input that cannot be
/// resolved is not checked (reading it reports the problem).
pub fn overwritten_input(out: &Path, inputs: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    let out = fs::canonicalize(out).ok()?;
    inputs
        .into_iter()
        .find(|input| fs::canonicalize(input).is_ok_and(|resolved| resolved == out))
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use dawsome_engine::compile::Compiler;
    use dawsome_engine::dsp::sampler::SampleBuffer;
    use dawsome_engine::edit::Touched;
    use dawsome_engine::plan::PlayMode;
    use dawsome_engine::project::{PatternId, Project};
    use dawsome_engine::realtime;
    use dawsome_engine::transport::Control;

    use super::*;
    use crate::write::{BitDepth, Dither, Encoding};

    const F1: &str = include_str!("../../../tests/fixtures/f1_four_on_floor.json");

    /// F1's player, told to play, with a one-frame click for its sample.
    fn player() -> Player {
        let project = Project::load_json(F1).expect("F1 loads");
        let click = |_: &_| Ok::<_, Infallible>(SampleBuffer::from(vec![1.0]));
        let update = Compiler::new()
            .compile(
                &project,
                &Touched::default(),
                PlayMode::Pattern(PatternId(1)),
                click,
            )
            .expect("compiles");
        let (mut link, player) = realtime::link(update, project.tempo);
        link.send(Control::Play).expect("room");
        // The link may go: the player keeps what it was sent.
        player
    }

    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dawsome-media-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        dir.join(name)
    }

    const WAV: Encoding = Encoding::Wav(BitDepth::Float32, Dither::Tpdf);

    #[test]
    fn a_render_on_its_own_thread_writes_the_same_bytes_and_pumps_meanwhile() {
        let here = temp_file("here.wav");
        let mut sink = Sink::create(&here, 44_100, WAV).expect("creates");
        let mut local = player();
        assert!(render(&mut local, 88_200, 512, &mut sink, &mut |_| true).expect("renders"));
        sink.finish().expect("finishes");

        let there = temp_file("there.wav");
        let sink = Sink::create(&there, 44_100, WAV).expect("creates");
        let mut pumped = 0;
        let mut last = 0;
        let rendered = render_on_thread(
            player(),
            88_200,
            512,
            sink,
            &mut || {},
            &mut || pumped += 1,
            &mut |done| {
                assert!(done >= last, "progress never runs backwards");
                last = done;
                true
            },
        )
        .expect("the render thread starts");
        assert!(rendered.result.expect("renders"));
        rendered.sink.finish().expect("finishes");
        assert!(pumped >= 1);
        assert_eq!(last, 88_200, "the last report is the whole render");
        assert_eq!(fs::read(&here).ok(), fs::read(&there).ok());
    }

    // An offline recording drains the record tap after every block on the render thread, so a
    // main thread that pumps its plugins slowly (50 ms a round here, while the render runs many
    // times faster than realtime) never lets the 4 s tap overflow: the recording holds every
    // frame played, the bytes of the render itself.
    #[test]
    fn a_threaded_render_does_its_block_work_after_every_block() {
        use crate::record::{self, Feed, RecordingFile, TapFeed};
        let project = Project::load_json(F1).expect("F1 loads");
        let click = |_: &_| Ok::<_, Infallible>(SampleBuffer::from(vec![1.0]));
        let update = Compiler::new()
            .compile(
                &project,
                &Touched::default(),
                PlayMode::Pattern(PatternId(1)),
                click,
            )
            .expect("compiles");
        let (mut link, player) = realtime::link(update, project.tempo);
        let tap = link.take_record_tap().expect("the tap");
        link.send(Control::Record {
            count_in: false,
            tap_master: true,
        })
        .expect("room");
        let mut feed = TapFeed { tap, rate: 44_100 };
        let taped = temp_file("taped.wav");
        let mut file = RecordingFile::create(&taped, 2, 44_100, 44_100).expect("creates");
        let (mut buffer, mut padded, mut blocks) = (Vec::new(), feed.dropped(), 0);
        let frames = 44_100 * 20;
        let mut drain = || {
            blocks += 1;
            record::drain(&mut feed, &mut file, &mut buffer, &mut padded).expect("writes");
        };
        let played = temp_file("played.wav");
        let sink = Sink::create(&played, 44_100, WAV).expect("creates");
        let rendered = render_on_thread(
            player,
            frames,
            512,
            sink,
            &mut drain,
            &mut || thread::sleep(Duration::from_millis(50)),
            &mut |_| true,
        )
        .expect("the render thread starts");
        assert!(rendered.result.expect("renders"));
        rendered.sink.finish().expect("finishes");
        assert_eq!(blocks, frames.div_ceil(512));
        assert_eq!(feed.dropped(), 0, "the tap never overflowed");
        assert_eq!(file.finish().expect("finishes"), frames);
        assert!(
            fs::read(&taped).ok() == fs::read(&played).ok(),
            "taped == played"
        );
    }

    #[test]
    fn progress_can_stop_a_threaded_render() {
        let path = temp_file("stopped.wav");
        let sink = Sink::create(&path, 44_100, WAV).expect("creates");
        let rendered = render_on_thread(
            player(),
            44_100 * 600,
            512,
            sink,
            &mut || {},
            &mut || {},
            &mut |_| false,
        )
        .expect("the render thread starts");
        assert!(!rendered.result.expect("no write error"), "stopped early");
    }
}
