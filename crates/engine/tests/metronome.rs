//! The metronome: while the transport plays and it is on, the player adds a click on every beat
//! after the master, accented on a bar's first beat, the count-in's own burst; the record tap
//! holds none of it, and it is the same however a render is split. Expected values come from
//! tests/fixtures/gen_p14_expected.py (p14_expected.json), never from the engine.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};

use dawsome_engine::compile::Compiler;
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::Touched;
use dawsome_engine::plan::PlayMode;
use dawsome_engine::project::{ChannelId, Note, PatternId, Project, DEFAULT_KEY};
use dawsome_engine::realtime::{self, Player, RealtimeLink};
use dawsome_engine::time::Ticks;
use dawsome_engine::transport::Control;
use serde_json::Value;

#[path = "../../../tests/support/counting_allocator.rs"]
mod counting_allocator;

use counting_allocator::heap_calls_during;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn expected() -> Value {
    let text = fs::read_to_string(fixtures().join("p14_expected.json"))
        .expect("p14_expected.json (run gen_p14_expected.py --write tests/fixtures)");
    serde_json::from_str(&text).expect("parses")
}

fn load(name: &str) -> Project {
    let text = fs::read_to_string(fixtures().join(name)).expect("a p14 fixture");
    Project::load_json(&text).expect("loads")
}

/// A player of `project`'s pattern 1 (its song when `song`), its files one frame of silence,
/// the metronome on as `metronome` says, playing.
fn playing(project: &Project, song: bool, metronome: bool) -> (RealtimeLink, Player) {
    let silence = |_: &Path| Ok::<_, Infallible>(SampleBuffer::from(vec![0.0]));
    let update = Compiler::new()
        .compile(
            project,
            &Touched::default(),
            if song {
                PlayMode::Song
            } else {
                PlayMode::Pattern(PatternId(1))
            },
            silence,
        )
        .expect("compiles");
    let (mut link, player) = realtime::link(update, project.tempo);
    link.send(Control::Metronome(metronome)).expect("room");
    link.send(Control::Play).expect("room");
    (link, player)
}

/// `frames` frames of `player`, a call per size of `sizes` (the last repeated).
fn render(player: &mut Player, frames: usize, sizes: &[usize]) -> Vec<[f32; 2]> {
    let mut out = vec![[f32::NAN; 2]; frames];
    let mut done = 0;
    let mut sizes = sizes.iter().cycle();
    while done < frames {
        let end = (done + sizes.next().copied().unwrap_or(512)).min(frames);
        player.process(&mut out[done..end]);
        done = end;
    }
    out
}

/// The frames `want` says sound, by frame, both sides the value.
fn sounding(want: &Value) -> BTreeMap<usize, f32> {
    want["nonzero"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|entry| {
            let at = entry[0].as_u64().expect("a frame") as usize;
            (at, entry[1].as_f64().expect("a value") as f32)
        })
        .collect()
}

// Known answers (gen_p14_expected.py): two bars of an empty pattern at 120 and 130 BPM play
// nothing but the clicks, at every beat's first sample (floored at 130 BPM), accented on each
// bar's first, and a song-mode loop of a bar from beat 2 clicks each pass from its beat 2, to
// the bit; whole, and split into blocks of 1, 7, 64, 1000 and 4096 and a mix.
#[test]
fn the_metronome_clicks_every_beat_however_split() {
    let want = expected();
    for (name, answer) in want["renders"].as_object().expect("renders") {
        let project = load(answer["fixture"].as_str().expect("fixture"));
        let frames = answer["frames"].as_u64().expect("frames") as usize;
        let clicks = sounding(answer);
        let song = answer["song"].as_bool().expect("a mode");
        for sizes in [
            &[frames][..],
            &[1],
            &[7],
            &[64],
            &[1000],
            &[4096],
            &[13, 1, 997, 2, 64],
        ] {
            let (_link, mut player) = playing(&project, song, true);
            let out = render(&mut player, frames, sizes);
            for (at, got) in out.iter().enumerate() {
                let value = clicks.get(&at).copied().unwrap_or(0.0);
                assert_eq!(
                    got.map(f32::to_bits),
                    [value.to_bits(); 2],
                    "{name}, blocks {sizes:?}, frame {at}"
                );
            }
        }
        let (_link, mut player) = playing(&project, song, false);
        let quiet = render(&mut player, frames, &[512]);
        assert!(
            quiet.iter().all(|frame| *frame == [0.0; 2]),
            "{name}: off, silent"
        );
    }
}

// The metronome is heard, not recorded: Record with a count-in and the master tapped plays the
// count-in's bar of clicks, then the song with the metronome's clicks (here nothing else: the
// pattern is empty), while the tap gets the song alone, silence. The metronome switched off
// mid-song clicks no more.
#[test]
fn the_record_tap_holds_no_click() {
    let want = expected();
    let answer = &want["renders"]["click_120"];
    let project = load("p14_click_120.json");
    let clicks = sounding(answer);
    let bar = 88_200;
    let (mut link, mut player) = playing(&project, false, true);
    link.send(Control::Stop).expect("room");
    let mut tap = link.take_record_tap().expect("the tap");
    link.send(Control::Record {
        count_in: true,
        tap_master: true,
    })
    .expect("room");
    let out = render(&mut player, 2 * bar, &[512]);
    // The count-in's bar and the song's first bar click alike: the same burst on the same beats.
    for (at, got) in out.iter().enumerate() {
        let value = clicks.get(&(at % bar)).copied().unwrap_or(0.0);
        assert_eq!(got.map(f32::to_bits), [value.to_bits(); 2], "frame {at}");
    }
    let mut recorded = Vec::new();
    tap.read_into(&mut recorded);
    assert_eq!(recorded.len(), 2 * bar, "the song's bar, both sides");
    assert!(
        recorded.iter().all(|&value| value == 0.0),
        "no click recorded"
    );
    link.send(Control::Metronome(false)).expect("room");
    let after = render(&mut player, bar, &[512]);
    assert!(after.iter().all(|frame| *frame == [0.0; 2]));
}

// The click never takes the output past full scale. Under the 120 BPM
// clicks, one note of a loud sound (0.9, a bar long, so the pattern's repeat starts it again
// as it ends) plays from tick 0: where a side and a click add up past ±1 the side is held at
// ±1 (at the side's own level, were it past ±1 already), elsewhere the click adds as before.
// The master's frames are the same render with the metronome off, the clicks
// gen_p14_expected.py's; every split gives the same bits.
#[test]
fn the_click_never_takes_the_output_past_full_scale() {
    let want = expected();
    let answer = &want["renders"]["click_120"];
    let clicks = sounding(answer);
    let frames = answer["frames"].as_u64().expect("frames") as usize;
    let mut project = load("p14_click_120.json");
    project
        .patterns
        .get_mut(&PatternId(1))
        .expect("pattern 1")
        .notes
        .push(Note {
            channel: ChannelId(1),
            start: Ticks(0),
            length: Ticks(768),
            velocity: 1.0,
            key: DEFAULT_KEY,
        });
    let loud_player = |metronome: bool| {
        let loud = |_: &Path| Ok::<_, Infallible>(SampleBuffer::from(vec![0.9; frames / 2]));
        let update = Compiler::new()
            .compile(
                &project,
                &Touched::default(),
                PlayMode::Pattern(PatternId(1)),
                loud,
            )
            .expect("compiles");
        let (mut link, player) = realtime::link(update, project.tempo);
        link.send(Control::Metronome(metronome)).expect("room");
        link.send(Control::Play).expect("room");
        (link, player)
    };
    let (_link, mut quiet) = loud_player(false);
    let master = render(&mut quiet, frames, &[512]);
    let peak = |out: &[[f32; 2]]| {
        out.iter()
            .flatten()
            .fold(0.0f32, |peak, v| peak.max(v.abs()))
    };
    let loudest = peak(&master);
    assert!(
        loudest > 0.75 && loudest <= 1.0,
        "the master alone is near full scale: {loudest}"
    );
    let held = |side: f32, click: f32| {
        let limit = side.abs().max(1.0);
        (side + click).clamp(-limit, limit)
    };
    for sizes in [&[frames][..], &[1], &[64], &[4096], &[13, 1, 997, 2, 64]] {
        let (_link, mut player) = loud_player(true);
        let out = render(&mut player, frames, sizes);
        assert!(peak(&out) <= 1.0, "blocks {sizes:?}: peak {}", peak(&out));
        for (at, (got, alone)) in out.iter().zip(&master).enumerate() {
            let want = match clicks.get(&at) {
                Some(&click) => alone.map(|side| held(side, click)),
                None => *alone,
            };
            assert_eq!(
                got.map(f32::to_bits),
                want.map(f32::to_bits),
                "blocks {sizes:?}, frame {at}"
            );
        }
    }
}

// The audio thread's rule: a bar of clicks with the metronome switched on and off makes no heap
// call.
#[test]
fn the_metronome_never_touches_the_heap() {
    let project = load("p14_click_loop.json");
    let (mut link, mut player) = playing(&project, true, true);
    let mut out = vec![[0.0; 2]; 512];
    let mut calls = 0;
    for call in 0..200 {
        if call % 50 == 0 {
            link.send(Control::Metronome(call % 100 == 0))
                .expect("room");
        }
        calls += heap_calls_during(|| player.process(&mut out));
    }
    assert_eq!(calls, 0, "Player::process made {calls} heap calls");
}
