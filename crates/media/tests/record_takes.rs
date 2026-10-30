//! Takes recorded one after another through the real player, record tap and writer thread, in
//! the orders the audio callback and the window can run them.

use std::convert::Infallible;
use std::path::{Path, PathBuf};

use dawsome_engine::compile::Compiler;
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::Touched;
use dawsome_engine::plan::PlayMode;
use dawsome_engine::project::Project;
use dawsome_engine::realtime::{self, Player, RealtimeLink};
use dawsome_engine::transport::Control;
use dawsome_media::record::{Feed, Recorded, Recorder, TapFeed};
use dawsome_media::WriteError;

const RATE: u32 = 44_100;

/// p8_rec (a kick on every beat at 44.1 kHz) whose sample is a 1000-frame ramp, so nearly
/// every frame of the song differs from its neighbors.
fn player() -> (RealtimeLink, Player) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/p8_rec.json");
    let text = std::fs::read_to_string(path).expect("p8_rec reads");
    let project = Project::load_json(&text).expect("p8_rec loads");
    let ramp: Vec<f32> = (0..1000).map(|k| (k + 1) as f32 / 1024.0).collect();
    let sample = |_: &Path| Ok::<_, Infallible>(SampleBuffer::from(ramp.clone()));
    let update = Compiler::new()
        .compile(&project, &Touched::default(), PlayMode::Song, sample)
        .expect("compiles");
    realtime::link(update, project.tempo)
}

fn temp(test: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("dawsome-takes-{}", std::process::id()))
        .join(test);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn start(path: PathBuf, feed: Box<dyn Feed>) -> Recorder {
    Recorder::start(path, feed, RATE)
        .map_err(|(error, _)| error)
        .expect("the take starts")
}

fn finish(recorder: Recorder) -> Recorded {
    recorder
        .finish()
        .map_err(|(error, _): (WriteError, _)| error)
        .expect("the take finishes")
}

fn samples(path: &Path) -> Vec<f32> {
    hound::WavReader::open(path)
        .expect("readable wav")
        .samples::<f32>()
        .collect::<Result<_, _>>()
        .expect("float samples")
}

fn record(link: &mut RealtimeLink) {
    let control = Control::Record {
        count_in: false,
        tap_master: true,
    };
    link.send(control).expect("room");
}

// Stop and the end of a take run at once: the writer's last drain can come before the player
// applies Stop (its callback was part way through a buffer), so one more buffer of the take
// reaches the tap after the take ended. Those frames belong to no take: the next take starts
// with its own first frame and holds exactly what was played for it.
#[test]
fn frames_played_after_a_take_ended_never_open_the_next() {
    let dir = temp("late_frames");
    let (mut link, mut player) = player();
    let mut block = [[0.0f32; 2]; 512];
    let tap = link.take_record_tap().expect("the tap");
    let first = start(dir.join("take1.wav"), Box::new(TapFeed { tap, rate: RATE }));
    record(&mut link);
    for _ in 0..8 {
        player.process(&mut block);
    }
    let recorded = finish(first);
    // The callback that was rendering when Stop was sent.
    player.process(&mut block);
    link.send(Control::Stop).expect("room");
    player.process(&mut block);
    let second = start(dir.join("take2.wav"), recorded.feed);
    record(&mut link);
    for _ in 0..8 {
        player.process(&mut block);
    }
    finish(second);
    let (one, two) = (
        samples(&dir.join("take1.wav")),
        samples(&dir.join("take2.wav")),
    );
    assert_eq!(one.len(), 2 * 8 * 512, "take 1 holds its 8 buffers");
    assert!(
        two == one,
        "take 2 plays the song from its own start ({} values, first {:?})",
        two.len(),
        &two[..4]
    );
}

// A writer that cannot write (a full disk: /dev/full takes the file and refuses every flush)
// stops by itself. The recorder says so while the take still plays, so the window can stop it,
// and finishing gives the error and the tap back. Linux only: Windows has no file that takes a
// create and refuses every write.
#[cfg(target_os = "linux")]
#[test]
fn a_writer_that_cannot_write_says_so_while_the_take_plays() {
    let (mut link, mut player) = player();
    let tap = link.take_record_tap().expect("the tap");
    let recorder = start(
        PathBuf::from("/dev/full"),
        Box::new(TapFeed { tap, rate: RATE }),
    );
    record(&mut link);
    let mut block = [[0.0f32; 2]; 512];
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !recorder.failed() {
        assert!(
            std::time::Instant::now() < deadline,
            "the writer stops by itself"
        );
        player.process(&mut block);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let Err((error, feed)) = recorder.finish() else {
        panic!("a take on a full disk fails");
    };
    assert!(error.to_string().contains("space"), "{error}");
    assert!(feed.into_tap().is_some(), "the tap comes back");
}
