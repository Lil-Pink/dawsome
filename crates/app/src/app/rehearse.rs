//! A rehearsed take (`--rehearse <events.json>`, a scripted window run): Record armed with no
//! count-in, Play from the song marker the file names (`"from"`, a tick; 0 without), and the
//! file's MIDI messages handed to the player as a device's are, each once the wall clock since
//! Play reaches its call (512 frames a call at the project's rate). With `--null-output` and
//! `--screenshot-every` it films a take with no device and nobody at the keyboard: how the
//! song-take glitch found in use was looked at.

use std::path::Path;
use std::time::Instant;

use dawsome_engine::time::Ticks;
use serde::Deserialize;

use super::{App, Message, PlayMode, RecordMessage};
use crate::midi::Router;
use crate::play_to::MidiEvents;

/// A take being rehearsed.
pub(super) struct Rehearsal {
    /// The messages, by the call they come before, in call order.
    messages: Vec<(u64, Vec<u8>)>,
    next: usize,
    started: Instant,
    router: Router,
}

/// What a rehearsal file adds to `--midi-events`' format.
#[derive(Deserialize)]
struct From {
    #[serde(default)]
    from: i64,
}

impl App {
    /// Starts the rehearsal `path` names; a file that cannot be read is said on stderr and
    /// nothing starts.
    pub fn start_rehearsal(&mut self, path: &Path) {
        let events = match MidiEvents::read(path) {
            Ok(events) => events,
            Err(error) => {
                eprintln!("dawsome-app: --rehearse: {error}");
                return;
            }
        };
        let from = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<From>(&text).ok())
            .map_or(0, |from| from.from);
        if self.recording.count_in {
            let _ = self.update(Message::Record(RecordMessage::ToggleCountIn));
        }
        if self.mode == PlayMode::Song {
            self.move_song_marker(Ticks(from));
        }
        if !self.recording.armed {
            let _ = self.update(Message::Record(RecordMessage::ToggleArm));
        }
        let router = self.router();
        router.links(crate::midi::linked_controls(self.session.project()));
        self.rehearsal = Some(Rehearsal {
            messages: events.messages(),
            next: 0,
            started: Instant::now(),
            router,
        });
        let _ = self.update(Message::Play);
    }

    /// The rehearsal's timer: the messages whose call the wall clock has reached go to the
    /// player.
    pub(super) fn rehearse(&mut self) {
        let rate = f64::from(self.session.project().sample_rate.max(1));
        let Some(rehearsal) = &mut self.rehearsal else {
            return;
        };
        let call = (rehearsal.started.elapsed().as_secs_f64() * rate / 512.0) as u64;
        while let Some((at, bytes)) = rehearsal.messages.get(rehearsal.next) {
            if *at > call {
                break;
            }
            rehearsal.router.deliver(bytes);
            rehearsal.next += 1;
        }
    }

    /// Whether the rehearsal's timer runs.
    pub(super) fn rehearsing(&self) -> bool {
        self.rehearsal
            .as_ref()
            .is_some_and(|rehearsal| rehearsal.next < rehearsal.messages.len())
    }
}
