// One of every playlist edit p4_loop accepts and one of every mixer edit p5_delay_sync accepts,
// and the constructors they are written with. Two test crates run the lists: the engine's
// tests/edits.rs, which includes this file as a module (each edit undone byte for byte, touching
// what it says), and src/compile.rs's tests (no edit of them recompiles a pattern's event list,
// which only the crate itself can read). One list, so an edit added here is checked both ways.
// The engine's names come from the module that includes the file (`use super::*`):
// `dawsome_engine::` in one crate, `crate::` in the other. Why `//` and not `//!`: compile.rs
// takes the file by `include!`, which refuses inner doc comments (a `#[path]` module inside its
// inline `mod tests` would be looked for under src/compile/, which does not exist).

use super::*;

pub fn pattern_clip(pattern: u32, track: u32, start: i64, length: i64) -> Clip {
    Clip::of_pattern(
        PatternId(pattern),
        TrackId(track),
        Ticks(start),
        Ticks(length),
    )
}

pub fn range(start: i64, end: i64, on: bool) -> LoopRange {
    LoopRange {
        start: Ticks(start),
        end: Ticks(end),
        on,
    }
}

/// One of every playlist edit that p4_loop accepts.
pub fn every_playlist_edit() -> Vec<Edit> {
    vec![
        Edit::PlaceClips {
            first: ClipId(4),
            clips: vec![pattern_clip(3, 2, 0, 384), pattern_clip(1, 8, 37, 250)],
        },
        Edit::UnplaceClips {
            first: ClipId(3),
            count: 1,
        },
        Edit::DuplicateClips {
            ids: vec![ClipId(2), ClipId(1)],
            first: ClipId(4),
        },
        Edit::DeleteClips {
            ids: vec![ClipId(3), ClipId(1)],
        },
        Edit::MoveClips {
            moves: vec![
                (ClipId(1), Ticks(1152), TrackId(2)),
                (ClipId(3), Ticks(0), TrackId(1)),
            ],
        },
        Edit::ResizeClip {
            id: ClipId(3),
            length: Ticks(96),
        },
        Edit::SetLoop { value: None },
        Edit::SetLoop {
            value: Some(range(0, 96, false)),
        },
        Edit::AddTrack {
            id: TrackId(9),
            name: "Track 9".into(),
        },
        Edit::RemoveTrack { id: TrackId(8) },
        Edit::RenameTrack {
            id: TrackId(1),
            name: "Drums".into(),
        },
        Edit::SetTrackMute {
            id: TrackId(2),
            mute: true,
        },
    ]
}

pub fn insert(id: u16) -> InsertId {
    InsertId(id)
}

fn mixer_only() -> Touched {
    Touched {
        mixer: true,
        ..Touched::default()
    }
}

fn effects_touched(id: u32, mixer: bool) -> Touched {
    Touched {
        mixer,
        effects: BTreeSet::from([EffectId(id)]),
        ..Touched::default()
    }
}

/// One of every mixer edit p5_delay_sync accepts, with what it touches.
pub fn every_mixer_edit() -> Vec<(Edit, Touched)> {
    let param = |key: &str, value| Edit::SetEffectParam {
        effect: EffectId(1),
        key: key.into(),
        value,
    };
    vec![
        (
            Edit::SetInsertVolume {
                insert: insert(2),
                fader: 0.5,
            },
            mixer_only(),
        ),
        (
            Edit::SetInsertPan {
                insert: insert(3),
                pan: -0.25,
            },
            mixer_only(),
        ),
        (
            Edit::MuteInsert {
                insert: insert(4),
                mute: true,
            },
            mixer_only(),
        ),
        (
            Edit::RenameInsert {
                insert: insert(1),
                name: "Echo".into(),
            },
            mixer_only(),
        ),
        (
            Edit::SoloInsert {
                set: BTreeSet::from([insert(1), insert(5)]),
            },
            mixer_only(),
        ),
        (
            Edit::RouteChannel {
                channel: ChannelId(1),
                insert: insert(7),
            },
            Touched {
                channels: BTreeSet::from([ChannelId(1)]),
                ..Touched::default()
            },
        ),
        (
            Edit::SetSend {
                from: insert(2),
                to: insert(3),
                amount: Some(0.5),
            },
            mixer_only(),
        ),
        (
            Edit::SetSend {
                from: insert(1),
                to: InsertId::MASTER,
                amount: Some(0.25),
            },
            mixer_only(),
        ),
        (
            Edit::SetSend {
                from: insert(1),
                to: InsertId::MASTER,
                amount: None,
            },
            mixer_only(),
        ),
        (
            Edit::AddEffect {
                insert: insert(1),
                slot: 3,
                id: EffectId(2),
                processor: ProcessorRef::new("dawsome.reverb"),
                on: true,
            },
            effects_touched(2, true),
        ),
        (
            Edit::RemoveEffect { id: EffectId(1) },
            effects_touched(1, true),
        ),
        (param("feedback", Some(0.75)), effects_touched(1, false)),
        (param("time_ms", Some(20.0)), effects_touched(1, false)),
        (param("mix", None), effects_touched(1, false)),
        // A value outside the processor's range is the processor's to clamp.
        (param("feedback", Some(5.0)), effects_touched(1, false)),
        (
            Edit::MoveEffect {
                effect: EffectId(1),
                slot: 4,
            },
            mixer_only(),
        ),
        (
            Edit::SetEffectOn {
                effect: EffectId(1),
                on: false,
            },
            mixer_only(),
        ),
        (
            Edit::AddEffect {
                insert: insert(1),
                slot: 1,
                id: EffectId(2),
                processor: ProcessorRef::new("dawsome.eq3"),
                on: false,
            },
            effects_touched(2, true),
        ),
    ]
}
