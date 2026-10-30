//! The notes a plugin's audio half sends, in any plugin format: which notes sound, when each one
//! ends, and the order a block's note-offs and note-ons go in. Each host turns these events into
//! its own format's (CLAP note events or MIDI messages, VST3 note events). Everything is sized
//! when the schedule is made, on the main thread, so a block allocates nothing.
//!
//! Within one frame the note-offs go before the note-ons, so a note that ends where another
//! starts on the same key does not cut the new one.

use std::sync::atomic::{AtomicU64, Ordering};

use dawsome_engine::processor::PortEvent;

/// Notes the table of sounding notes holds per instance; a note-on that finds it full is not
/// sent. A note that sounded before a block and ends in it has left the table before that
/// block's note-ons look for room, wherever in the block either falls.
pub const SOUNDING_NOTES: usize = 512;

/// Note-ons one block can send: what the engine sends a plugin channel at most.
pub const STARTS_PER_BLOCK: usize = 1024;

/// Note events one block can send, offs and ons: what a host's event room must hold. The notes
/// due in a block leave the table before its note-ons look for room, so a block can end every
/// note of a full table and then every note it starts, up to a table's worth: at most `n`
/// note-ons (`n` the smaller of [`STARTS_PER_BLOCK`] and [`SOUNDING_NOTES`]) and
/// [`SOUNDING_NOTES`] + `n` note-offs.
pub const EVENTS_PER_BLOCK: usize = ENDS_PER_BLOCK + min(STARTS_PER_BLOCK, SOUNDING_NOTES);

/// Note-offs one block can send: [`SOUNDING_NOTES`] + `n`, as [`EVENTS_PER_BLOCK`] counts them.
const ENDS_PER_BLOCK: usize = SOUNDING_NOTES + min(STARTS_PER_BLOCK, SOUNDING_NOTES);

const fn min(a: usize, b: usize) -> usize {
    if a < b {
        a
    } else {
        b
    }
}

/// A note event a block sends a plugin, on frame `frame` of the block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NoteEvent {
    /// A note starts, with the id the schedule gave it; `live` for a note played on a device or
    /// the typing keyboard rather than a sequenced one.
    On {
        frame: u32,
        key: u8,
        velocity: f32,
        id: i32,
        live: bool,
    },
    /// The note with this id and key ends.
    Off {
        frame: u32,
        key: u8,
        id: i32,
        live: bool,
    },
}

/// A note that is sounding: the output-clock frame its note-off falls on, its key and id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Sounding {
    off: u64,
    key: u8,
    id: i32,
    /// A live note, sounding until a note-off of its key sets `off`.
    held: bool,
    /// It was started live (it may have been let go of since).
    live: bool,
}

/// A note-on this block sends.
#[derive(Debug, Clone, Copy)]
struct Start {
    frame: u32,
    key: u8,
    velocity: f32,
    id: i32,
    live: bool,
}

/// The notes an instance has sounding and the ones a block starts.
#[derive(Debug)]
pub struct NoteSchedule {
    /// Whether the plugin takes notes at all; one that does not is sent none.
    takes_notes: bool,
    sounding: Vec<Sounding>,
    ending: Vec<Sounding>,
    starts: Vec<Start>,
    next_id: i32,
}

impl NoteSchedule {
    /// An empty schedule with room for every note it can hold. Runs on the main thread.
    pub fn new(takes_notes: bool) -> Self {
        Self {
            takes_notes,
            sounding: Vec::with_capacity(SOUNDING_NOTES),
            ending: Vec::with_capacity(ENDS_PER_BLOCK),
            starts: Vec::with_capacity(STARTS_PER_BLOCK),
            next_id: 0,
        }
    }

    /// The notes of a block of `len` frames starting at output-clock frame `clock`, from the
    /// engine's `events`: each note-off and note-on, in frame order, note-offs first on a frame
    /// they share, handed to `emit`. A note-on that finds the sounding table full is not sent
    /// and counted in `dropped`; a note that sounded before the block and ends in it has left
    /// the table by then.
    pub fn block(
        &mut self,
        events: &[PortEvent],
        clock: u64,
        len: usize,
        dropped: &AtomicU64,
        emit: impl FnMut(NoteEvent),
    ) {
        self.starts.clear();
        self.ending.clear();
        let end = clock.saturating_add(len as u64);
        // The notes due in this block leave the table before its note-ons look for room, so a
        // reset with the table full (`end_all`: every note ends on frame 0) does not drop the
        // next block's notes (review 2026-10-26). A held note is never due (its end is
        // u64::MAX) and the events change only held notes, so which notes end, and so the
        // events sent, are what they were whenever the table had room.
        self.take_due(end);
        for &event in events {
            match event {
                PortEvent::Note {
                    frame,
                    key,
                    velocity,
                    length,
                } => {
                    let off = clock
                        .saturating_add(u64::from(frame))
                        .saturating_add(length);
                    self.start_note(frame, key, velocity, Some(off), dropped);
                }
                PortEvent::NoteOn {
                    frame,
                    key,
                    velocity,
                } => self.start_note(frame, key, velocity, None, dropped),
                PortEvent::NoteOff { frame, key } => {
                    let held = self
                        .sounding
                        .iter_mut()
                        .find(|note| note.held && note.key == key);
                    if let Some(note) = held {
                        // A note started in this block lasts a frame at least, as a sequenced
                        // one does: on its own start frame the note-off would go first and
                        // leave the plugin's note sounding.
                        let earliest = self
                            .starts
                            .iter()
                            .find(|start| start.id == note.id)
                            .map_or(0, |start| start.frame.saturating_add(1));
                        note.held = false;
                        note.off = clock.saturating_add(u64::from(frame.max(earliest)));
                    }
                }
                PortEvent::Bend { .. } | PortEvent::Wheel { .. } => {}
            }
        }
        // The notes the block started or let go of that end in it.
        self.take_due(end);
        // Unstable sorts do not allocate; the id breaks ties, so the order is fixed.
        self.ending.sort_unstable_by_key(|note| (note.off, note.id));
        merge(&self.ending, &self.starts, clock, emit);
    }

    /// Moves the notes whose note-off falls before output-clock frame `end` from the table to
    /// the block's ending ones.
    fn take_due(&mut self, end: u64) {
        let ending = &mut self.ending;
        self.sounding.retain(|note| {
            let due = note.off < end;
            if due {
                ending.push(*note);
            }
            !due
        });
    }

    /// Remembers a note-on on `frame` with the next note id, and when its note-off is due: at
    /// output-clock frame `off`, or, for a live note (`None`), once its note-off comes.
    fn start_note(
        &mut self,
        frame: u32,
        key: u8,
        velocity: f32,
        off: Option<u64>,
        dropped: &AtomicU64,
    ) {
        if !self.takes_notes || self.starts.len() >= STARTS_PER_BLOCK {
            return;
        }
        if self.sounding.len() >= SOUNDING_NOTES {
            dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let id = self.next_id;
        self.next_id = id.checked_add(1).unwrap_or(0);
        let live = off.is_none();
        self.sounding.push(Sounding {
            off: off.unwrap_or(u64::MAX),
            key,
            id,
            held: live,
            live,
        });
        self.starts.push(Start {
            frame,
            key,
            velocity,
            id,
            live,
        });
    }

    /// Forgets every sounding note: the plugin was reset, so none of them sounds any more.
    pub fn clear(&mut self) {
        self.sounding.clear();
    }

    /// Ends every sounding note on frame 0 of the next block, whatever its own end. Why: a
    /// reset of the transport restarts the output clock, so the ends the notes had are of the
    /// old clock, and a plugin that a reset does not silence by itself still hears each note
    /// end.
    pub fn end_all(&mut self) {
        for note in &mut self.sounding {
            note.off = 0;
            note.held = false;
        }
    }
}

/// Hands the block's note-offs and note-ons to `emit` in frame order, note-offs first on a frame
/// they share. A note-off whose time has passed (it cannot have, but a clock is a clock) goes on
/// frame 0.
fn merge(ending: &[Sounding], starts: &[Start], clock: u64, mut emit: impl FnMut(NoteEvent)) {
    let off_frame = |note: &Sounding| u32::try_from(note.off.saturating_sub(clock)).unwrap_or(0);
    let (mut offs, mut ons) = (ending.iter().peekable(), starts.iter().peekable());
    loop {
        let off_first = match (offs.peek(), ons.peek()) {
            (Some(off), Some(on)) => off_frame(off) <= on.frame,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };
        if off_first {
            if let Some(off) = offs.next() {
                emit(NoteEvent::Off {
                    frame: off_frame(off),
                    key: off.key,
                    id: off.id,
                    live: off.live,
                });
            }
        } else if let Some(on) = ons.next() {
            emit(NoteEvent::On {
                frame: on.frame,
                key: on.key,
                velocity: on.velocity,
                id: on.id,
                live: on.live,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The events as (frame, kind, key) rows.
    fn rows(events: &[NoteEvent]) -> Vec<(u32, &'static str, u8)> {
        events
            .iter()
            .map(|event| match *event {
                NoteEvent::On { frame, key, .. } => (frame, "on", key),
                NoteEvent::Off { frame, key, .. } => (frame, "off", key),
            })
            .collect()
    }

    // At one frame, a note ending on frame 10 comes before a note starting there, whatever order
    // they arrive in; frames otherwise in order.
    #[test]
    fn offs_come_before_ons_on_a_shared_frame_and_frames_stay_in_order() {
        let ending = [
            Sounding {
                off: 1010,
                key: 60,
                id: 0,
                held: false,
                live: false,
            },
            Sounding {
                off: 1100,
                key: 62,
                id: 1,
                held: false,
                live: false,
            },
        ];
        let start = |frame, key, id| Start {
            frame,
            key,
            velocity: 1.0,
            id,
            live: false,
        };
        let starts = [start(0, 64, 2), start(10, 60, 3), start(200, 65, 4)];
        let mut events = Vec::new();
        merge(&ending, &starts, 1000, |event| events.push(event));
        assert_eq!(
            rows(&events),
            [
                (0, "on", 64),
                (10, "off", 60),
                (10, "on", 60),
                (100, "off", 62),
                (200, "on", 65)
            ]
        );
        let mut one = Vec::new();
        merge(&ending[..1], &starts[1..2], 1000, |event| one.push(event));
        assert_eq!(rows(&one), [(10, "off", 60), (10, "on", 60)]);
    }

    // A sequenced note ends after its length, a live one at its note-off, both with the ids they
    // started with; a live note let go of in the block it started in lasts a frame; a schedule
    // for a plugin that takes no notes sends none.
    #[test]
    fn notes_end_after_their_length_or_their_note_off() {
        let dropped = AtomicU64::new(0);
        let mut schedule = NoteSchedule::new(true);
        let mut events = Vec::new();
        let notes = [
            PortEvent::Note {
                frame: 4,
                key: 60,
                velocity: 0.5,
                length: 10,
            },
            PortEvent::NoteOn {
                frame: 6,
                key: 64,
                velocity: 1.0,
            },
            PortEvent::NoteOff { frame: 6, key: 64 },
        ];
        schedule.block(&notes, 100, 64, &dropped, |event| events.push(event));
        assert_eq!(
            events,
            [
                NoteEvent::On {
                    frame: 4,
                    key: 60,
                    velocity: 0.5,
                    id: 0,
                    live: false
                },
                NoteEvent::On {
                    frame: 6,
                    key: 64,
                    velocity: 1.0,
                    id: 1,
                    live: true
                },
                NoteEvent::Off {
                    frame: 7,
                    key: 64,
                    id: 1,
                    live: true
                },
                NoteEvent::Off {
                    frame: 14,
                    key: 60,
                    id: 0,
                    live: false
                },
            ]
        );
        let mut none = NoteSchedule::new(false);
        let mut sent = 0;
        none.block(&notes, 0, 64, &dropped, |_| sent += 1);
        assert_eq!((sent, dropped.load(Ordering::Relaxed)), (0, 0));
    }

    // After `end_all` every note still sounding (here a held live note and a long sequenced one)
    // ends on frame 0 of the next block, whatever clock that block is on.
    #[test]
    fn end_all_ends_every_sounding_note_on_the_next_frame_zero() {
        let dropped = AtomicU64::new(0);
        let mut schedule = NoteSchedule::new(true);
        let notes = [
            PortEvent::NoteOn {
                frame: 0,
                key: 69,
                velocity: 1.0,
            },
            PortEvent::Note {
                frame: 1,
                key: 72,
                velocity: 1.0,
                length: 1 << 20,
            },
        ];
        schedule.block(&notes, 5000, 64, &dropped, |_| {});
        schedule.end_all();
        let mut events = Vec::new();
        schedule.block(&[], 0, 64, &dropped, |event| events.push(event));
        assert_eq!(rows(&events), [(0, "off", 69), (0, "off", 72)]);
        schedule.block(&[], 64, 64, &dropped, |_| panic!("nothing sounds"));
    }

    /// A sequenced note on `frame` of `key`, `length` frames long.
    fn note(frame: u32, key: usize, length: u64) -> PortEvent {
        PortEvent::Note {
            frame,
            key: (key % 128) as u8,
            velocity: 1.0,
            length,
        }
    }

    // A reset with the table full (512 notes sounding, then `end_all`) ends them all on frame 0
    // of the next block, so that block's note-ons find room and are all sent, after the offs.
    // (Review 2026-10-26: the starts were recorded before the ending notes left the table, so
    // they were dropped.)
    #[test]
    fn notes_ended_in_a_block_make_room_for_its_note_ons() {
        let dropped = AtomicU64::new(0);
        let mut schedule = NoteSchedule::new(true);
        let full: Vec<PortEvent> = (0..SOUNDING_NOTES).map(|n| note(0, n, 1 << 20)).collect();
        schedule.block(&full, 0, 64, &dropped, |_| {});
        assert_eq!(dropped.load(Ordering::Relaxed), 0, "the table holds 512");
        schedule.end_all();
        let mut events = Vec::new();
        let starts = [note(0, 60, 1 << 20), note(10, 62, 1 << 20)];
        schedule.block(&starts, 64, 64, &dropped, |event| events.push(event));
        assert_eq!(dropped.load(Ordering::Relaxed), 0, "none dropped");
        let rows = rows(&events);
        assert_eq!(rows.len(), SOUNDING_NOTES + 2);
        assert!(rows[..SOUNDING_NOTES]
            .iter()
            .all(|&(frame, kind, _)| (frame, kind) == (0, "off")));
        assert_eq!(rows[SOUNDING_NOTES..], [(0, "on", 60), (10, "on", 62)]);
    }

    // The most a block can send: a full table ended by `end_all`, then a table's worth of notes
    // that start and end in the block. 1024 offs and 512 ons, `EVENTS_PER_BLOCK` (the hosts'
    // rooms, the same 1536 as before), none dropped, and no buffer grew (a block allocates
    // nothing).
    #[test]
    fn the_most_a_block_sends_fits_its_room() {
        let dropped = AtomicU64::new(0);
        let mut schedule = NoteSchedule::new(true);
        let full: Vec<PortEvent> = (0..SOUNDING_NOTES).map(|n| note(0, n, 1 << 20)).collect();
        schedule.block(&full, 0, 512, &dropped, |_| {});
        schedule.end_all();
        let rooms = |schedule: &NoteSchedule| {
            (
                schedule.sounding.capacity(),
                schedule.ending.capacity(),
                schedule.starts.capacity(),
            )
        };
        let before = rooms(&schedule);
        let short: Vec<PortEvent> = (0..SOUNDING_NOTES)
            .map(|n| note((n % 256) as u32, n, 1))
            .collect();
        let (mut offs, mut ons) = (0, 0);
        schedule.block(&short, 512, 512, &dropped, |event| match event {
            NoteEvent::Off { .. } => offs += 1,
            NoteEvent::On { .. } => ons += 1,
        });
        assert_eq!((offs, ons), (2 * SOUNDING_NOTES, SOUNDING_NOTES));
        assert_eq!(offs + ons, EVENTS_PER_BLOCK);
        assert_eq!(EVENTS_PER_BLOCK, SOUNDING_NOTES + STARTS_PER_BLOCK);
        assert_eq!(dropped.load(Ordering::Relaxed), 0);
        assert_eq!(rooms(&schedule), before, "no buffer grew");
        assert!(schedule.sounding.is_empty(), "every note ended");
    }
}
