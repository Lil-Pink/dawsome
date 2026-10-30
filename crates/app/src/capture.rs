//! Capture: the notes played while no take runs, kept for Capture (Ctrl+Shift+R) to turn into
//! notes after the fact. Each live note the player reports is
//! timed by the output clock's frame it sounded from ([`EngineEvent::LiveNoteStarted`]'s
//! `frame`), which never jumps, so notes played while stopped are timed as exactly as notes
//! played along the song. The window is the 30 s up to the newest note's start (by those
//! frames), and nothing once the last note ended more than 30 s ago (by the wall clock). Bounded:
//! at most [`MOST_NOTES`] notes, the oldest dropped first; a key still held is not captured.
//!
//! Laid out at the current tempo: the first note's start is tick 0, a note at frame `f` starts
//! at the playhead's tick for `f − f0` (the last tick that starts at or before it) and ends at
//! the one for its end, quantized as a take is (the start to the nearest snap line).

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use dawsome_engine::project::{ChannelId, Note};
use dawsome_engine::realtime::EngineEvent;
use dawsome_engine::time::{self, MilliBpm, Ticks};

use crate::ui::note_gesture::snap_nearest;

/// How far back Capture reaches.
pub const WINDOW: Duration = Duration::from_secs(30);

/// The most notes kept; the oldest go first.
pub const MOST_NOTES: usize = 4096;

/// A note played, by the output clock's frames.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Played {
    channel: ChannelId,
    key: u8,
    velocity: f32,
    start: u64,
    end: u64,
    /// The playhead's tick when it started, while the song timeline ran.
    tick: Option<Ticks>,
}

/// The notes played lately, for Capture.
#[derive(Debug, Default)]
pub struct Capture {
    played: VecDeque<Played>,
    /// The keys held, by channel and key: when each started, its velocity and tick.
    held: BTreeMap<(ChannelId, u8), (u64, f32, Option<Ticks>)>,
    /// When the newest note ended, by the wall clock.
    last: Option<Instant>,
    /// Keys whose next start is the piano roll's preview, not played: left out.
    previews: Vec<u8>,
}

impl Capture {
    /// Takes in what the player reported, heard at `now`.
    pub fn hear(&mut self, events: &[EngineEvent], now: Instant) {
        for event in events {
            match *event {
                EngineEvent::LiveNoteStarted {
                    channel,
                    key,
                    velocity,
                    tick,
                    frame,
                } => {
                    if let Some(at) = self.previews.iter().position(|&held| held == key) {
                        self.previews.remove(at);
                        self.held.remove(&(channel, key));
                        continue;
                    }
                    self.held.insert((channel, key), (frame, velocity, tick));
                }
                EngineEvent::LiveNoteEnded {
                    channel,
                    key,
                    frame,
                    ..
                } => {
                    let Some((start, velocity, tick)) = self.held.remove(&(channel, key)) else {
                        continue;
                    };
                    self.played.push_back(Played {
                        channel,
                        key,
                        velocity,
                        start,
                        end: frame.max(start),
                        tick,
                    });
                    if self.played.len() > MOST_NOTES {
                        self.played.pop_front();
                    }
                    self.last = Some(now);
                }
                EngineEvent::StreamUnderrun { .. } | EngineEvent::StreamFailed { .. } => {}
            }
        }
    }

    /// The piano roll previews `key`: its next start is not played, and is left out.
    pub fn preview(&mut self, key: u8) {
        self.previews.push(key);
        if self.previews.len() > 16 {
            self.previews.remove(0);
        }
    }

    /// Forgets everything: a new document's player counts its frames from 0.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// The notes Capture lays out at `now`, at `rate` and `tempo`, the first starting at tick 0,
    /// starts quantized to `quantize`; and the playhead tick the first was heard at while the
    /// song timeline ran. Empty when nothing was played in the window.
    pub fn notes(
        &self,
        rate: u32,
        tempo: MilliBpm,
        ppq: u16,
        quantize: Option<Ticks>,
        now: Instant,
    ) -> (Vec<Note>, Option<Ticks>) {
        let fresh = self
            .last
            .is_some_and(|last| now.saturating_duration_since(last) <= WINDOW);
        let Some(newest) = self
            .played
            .iter()
            .map(|note| note.start)
            .max()
            .filter(|_| fresh)
        else {
            return (Vec::new(), None);
        };
        let reach = WINDOW.as_secs() * u64::from(rate);
        let kept: Vec<&Played> = self
            .played
            .iter()
            .filter(|note| note.start + reach >= newest)
            .collect();
        let Some(first) = kept.iter().min_by_key(|note| note.start) else {
            return (Vec::new(), None);
        };
        let at = |frame: u64| -> i64 {
            let after = frame.saturating_sub(first.start).saturating_add(1);
            time::first_tick_at_or_after(after, rate, tempo, ppq).map_or(0, |tick| tick.0 - 1)
        };
        let notes = kept
            .iter()
            .map(|note| {
                let start = at(note.start);
                let snapped = quantize.map_or(start, |unit| snap_nearest(Ticks(start), unit).0);
                Note {
                    channel: note.channel,
                    start: Ticks(snapped),
                    length: Ticks((at(note.end) - start).max(0)),
                    velocity: note.velocity,
                    key: note.key,
                }
            })
            .collect();
        (notes, first.tick)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 44_100;
    const TEMPO: MilliBpm = MilliBpm(120_000);

    fn started(key: u8, frame: u64) -> EngineEvent {
        EngineEvent::LiveNoteStarted {
            channel: ChannelId(1),
            key,
            velocity: 0.5,
            tick: None,
            frame,
        }
    }

    fn ended(key: u8, frame: u64) -> EngineEvent {
        EngineEvent::LiveNoteEnded {
            channel: ChannelId(1),
            key,
            tick: None,
            frame,
        }
    }

    fn laid_out(capture: &Capture, quantize: Option<Ticks>, now: Instant) -> Vec<(i64, i64, u8)> {
        capture
            .notes(RATE, TEMPO, 96, quantize, now)
            .0
            .iter()
            .map(|note| (note.start.0, note.length.0, note.key))
            .collect()
    }

    // By hand, at 44.1 kHz and 120 BPM (229.6875 frames a tick): a note at frame 1000 (the
    // first: tick 0) to 21480 (20480 later: tick 89); one 51200 frames after the first to
    // 76800 after (ticks 222 to 334, length 112), quantized to the step: 216. A key still held
    // is left out, and so is a note started more than 30 s (1323000 frames) before the newest.
    #[test]
    fn notes_are_laid_out_from_the_first_at_the_tempo() {
        let now = Instant::now();
        let mut capture = Capture::default();
        capture.hear(
            &[
                started(72, 0),
                ended(72, 100),
                started(60, 1_400_000),
                ended(60, 1_420_480),
                started(64, 1_451_200),
                ended(64, 1_476_800),
                started(67, 1_480_000),
            ],
            now,
        );
        assert_eq!(laid_out(&capture, None, now), [(0, 89, 60), (222, 112, 64)]);
        assert_eq!(
            laid_out(&capture, Some(Ticks(24)), now),
            [(0, 89, 60), (216, 112, 64)]
        );
        // The key held ends now: it counts from here on.
        capture.hear(&[ended(67, 1_500_000)], now);
        assert_eq!(laid_out(&capture, None, now).len(), 3);
    }

    // Nothing ended in the last 30 s by the clock: nothing to capture. A key the piano roll
    // previews is left out, as is its end. Cleared, it holds nothing; never more than its most.
    #[test]
    fn the_window_previews_and_the_bound() {
        let then = Instant::now();
        let mut capture = Capture::default();
        capture.hear(&[started(60, 0), ended(60, 4410)], then);
        assert_eq!(laid_out(&capture, None, then).len(), 1);
        let later = then + WINDOW + Duration::from_millis(1);
        assert!(laid_out(&capture, None, later).is_empty(), "stale");
        capture.preview(62);
        capture.hear(&[started(62, 5000), ended(62, 6000)], then);
        assert_eq!(laid_out(&capture, None, then), [(0, 19, 60)]);
        capture.clear();
        assert!(laid_out(&capture, None, then).is_empty());
        for note in 0..MOST_NOTES as u64 + 10 {
            capture.hear(&[started(60, note * 10), ended(60, note * 10 + 5)], then);
        }
        assert_eq!(capture.played.len(), MOST_NOTES);
        assert_eq!(capture.played.front().map(|note| note.start), Some(100));
    }
}
