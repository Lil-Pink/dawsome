//! MIDI out: the note on and note off messages the player sends for input channels with a MIDI
//! out, and the performance controls a device plays beside its notes (the sustain pedal, pitch
//! bend, the mod wheel), each
//! stamped with the output frame it is due on, through a ring of their own to the thread that
//! sends them to the port.
//!
//! Why a ring of its own and not the event ring: that ring drops what does not fit, and a
//! dropped note off is a note that never ends. Here a note on that does not fit is dropped
//! (counted), always leaving [`OFF_RESERVE`] slots for the note offs of the notes already sent.
//!
//! The audio thread's half, [`MidiOutState`], looks ahead of the playhead by each channel's
//! latency: for the output frames `[F, F + len)` of a piece it sends the notes whose natural
//! frame (the output frame their tick plays on) is in `[sent until, F + len + latency)`, each
//! `latency` frames early but never before `F`, following the plan's wrap. It keeps the note
//! offs still to come in a fixed table and sends them when their frame comes, or at once when
//! the playhead jumps (Play, Seek, Stop, Suspend), and when a new plan takes a channel's MIDI out
//! away or changes it. Every message of a piece is sorted (frame, note offs first, start order)
//! before it goes into the ring, so a render gives the same messages however it is split. Nothing
//! here allocates after [`ring`] made it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use rtrb::{Consumer, Producer, RingBuffer};

use crate::dsp::instrument::velocity_byte;
use crate::plan::{PlayPlan, Playing};
use crate::project::{ChannelId, PatternId};
use crate::render::{gate_frames, EventWalk, SongWalk};
use crate::time::Ticks;
use crate::transport::{TempoSegment, Wrap};

/// Messages that can wait for the sending thread. A reader that reads after every call keeps
/// every message of calls up to this many frames long (the command line's `--midi-out-log`
/// refuses longer ones).
pub const RING_CAPACITY: usize = 4096;

/// Ring slots a note on never takes: they are kept for the note offs of notes already sent.
/// As many as notes can be sounding, so every one of them can always be ended.
pub(crate) const OFF_RESERVE: usize = 256;

/// The most notes sounding on MIDI outs at once; a note on past them is dropped and counted.
const SOUNDING: usize = OFF_RESERVE;

/// The most note ons one piece sends; more are dropped and counted.
const ONS_PER_PIECE: usize = 768;

/// A note off's status nibble, then a note on's, a control change's, a pitch bend's.
const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL: u8 = 0xB0;
const BEND: u8 = 0xE0;

/// The sustain pedal's controller number, and the value from which it is down; the mod wheel's.
pub(crate) const SUSTAIN: u8 = 64;
pub(crate) const PEDAL_DOWN: u8 = 64;
pub(crate) const WHEEL: u8 = 1;

/// The velocity every note off is sent with: 64, the value for "no velocity".
const OFF_VELOCITY: u8 = 0x40;

/// One message for a MIDI out: a note on or a note off, or a performance control a device played
/// (the sustain pedal, pitch bend, the mod wheel), the output frame it is due on, and the rack channel whose MIDI out it goes
/// to. Three bytes at most and only built here, so a message the engine sends can never be a
/// system exclusive or any other kind than these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MidiOutEvent {
    /// The output frame (counted since the link was made) the message is due on.
    pub frame: u64,
    /// The channel whose MIDI out sends it.
    pub channel: ChannelId,
    len: u8,
    bytes: [u8; 3],
}

impl MidiOutEvent {
    fn note(frame: u64, channel: ChannelId, status: u8, key: u8, velocity: u8) -> Self {
        Self {
            frame,
            channel,
            len: 3,
            bytes: [status, key & 0x7F, velocity & 0x7F],
        }
    }

    /// Note on of `key` on MIDI channel `status_channel` (0 to 15) at `velocity` (0 to 1, as
    /// a byte from 1 to 127).
    pub(crate) fn note_on(
        frame: u64,
        channel: ChannelId,
        status_channel: u8,
        key: u8,
        velocity: f32,
    ) -> Self {
        let status = NOTE_ON | (status_channel & 0x0F);
        Self::note(frame, channel, status, key, velocity_byte(velocity))
    }

    /// Note off of `key` on MIDI channel `status_channel` (0 to 15).
    pub(crate) fn note_off(frame: u64, channel: ChannelId, status_channel: u8, key: u8) -> Self {
        let status = NOTE_OFF | (status_channel & 0x0F);
        Self::note(frame, channel, status, key, OFF_VELOCITY)
    }

    /// The sustain pedal at `value` (0 to 127) on MIDI channel `status_channel` (0 to 15).
    pub(crate) fn sustain(frame: u64, channel: ChannelId, status_channel: u8, value: u8) -> Self {
        Self::control(frame, channel, status_channel, SUSTAIN, value)
    }

    /// The mod wheel at `value` (0 to 127) on MIDI channel `status_channel` (0 to 15).
    pub(crate) fn mod_wheel(frame: u64, channel: ChannelId, status_channel: u8, value: u8) -> Self {
        Self::control(frame, channel, status_channel, WHEEL, value)
    }

    fn control(frame: u64, channel: ChannelId, status_channel: u8, cc: u8, value: u8) -> Self {
        Self {
            frame,
            channel,
            len: 3,
            bytes: [CONTROL | (status_channel & 0x0F), cc & 0x7F, value & 0x7F],
        }
    }

    /// Pitch bend at `value` (−8192 to 8191, 0 at rest) on MIDI channel `status_channel` (0 to
    /// 15): En ll mm, the 14-bit value + 8192, its low 7 bits first.
    pub(crate) fn pitch_bend(
        frame: u64,
        channel: ChannelId,
        status_channel: u8,
        value: i16,
    ) -> Self {
        let raw = (i32::from(value) + 8192).clamp(0, 0x3FFF) as u16;
        Self {
            frame,
            channel,
            len: 3,
            bytes: [
                BEND | (status_channel & 0x0F),
                (raw & 0x7F) as u8,
                (raw >> 7) as u8,
            ],
        }
    }

    /// The message's bytes, as they go to the port.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.len)]
    }

    #[cfg(test)]
    fn is_note_on(&self) -> bool {
        self.bytes[0] & 0xF0 == NOTE_ON
    }

    /// Whether it ends something sounding: a note off, or the pedal let go of. Such a message may
    /// take the ring's last slots (the reserve), and one lost is counted as a lost off; any other
    /// leaves the reserve free and is counted dropped.
    fn ends(&self) -> bool {
        match self.bytes[0] & 0xF0 {
            NOTE_OFF => true,
            CONTROL => self.bytes[1] == SUSTAIN && self.bytes[2] < PEDAL_DOWN,
            _ => false,
        }
    }
}

/// What the audio thread counts, for the reader to see.
#[derive(Debug, Default)]
struct Counts {
    dropped: AtomicU64,
    lost_offs: AtomicU64,
}

/// The sending thread's half of the MIDI-out ring: the messages the player has sent, oldest
/// first, in the order they are due. Take it once from the link
/// ([`crate::realtime::RealtimeLink::take_midi_out`]).
pub struct MidiOutReader {
    events: Consumer<MidiOutEvent>,
    counts: Arc<Counts>,
}

impl MidiOutReader {
    /// The next message waiting, if any.
    pub fn pop(&mut self) -> Option<MidiOutEvent> {
        self.events.pop().ok()
    }

    /// Moves every message waiting into `out`, oldest first, and returns how many.
    pub fn read_into(&mut self, out: &mut Vec<MidiOutEvent>) -> usize {
        let waiting = self.events.slots();
        out.reserve(waiting);
        let mut moved = 0;
        while let Ok(event) = self.events.pop() {
            out.push(event);
            moved += 1;
        }
        moved
    }

    /// Note ons dropped so far: the ring (but for its reserve), the table of sounding notes or a
    /// piece had no room for them. Nothing hangs: their note offs were never due.
    pub fn dropped(&self) -> u64 {
        self.counts.dropped.load(Ordering::Relaxed)
    }

    /// Note offs lost so far because the ring was full to its last slot (the reader stalled):
    /// their notes may hang, so the reader should end every note on the port.
    pub fn lost_offs(&self) -> u64 {
        self.counts.lost_offs.load(Ordering::Relaxed)
    }

    /// The MIDI-out log a render writes (`--midi-out-log`, the command line's and the app's
    /// alike): every message `sent`, in order, with its output frame, its rack channel and its
    /// bytes in hex, then what this reader counted dropped and lost; one line of JSON.
    pub fn log(&self, sent: &[MidiOutEvent]) -> String {
        let events: Vec<serde_json::Value> = sent
            .iter()
            .map(|event| {
                let bytes: Vec<String> = event.bytes().iter().map(|b| format!("{b:02X}")).collect();
                serde_json::json!({
                    "frame": event.frame,
                    "channel": event.channel.0,
                    "bytes": bytes.join(" "),
                })
            })
            .collect();
        let log = serde_json::json!({
            "events": events,
            "dropped": self.dropped(),
            "lost_offs": self.lost_offs(),
        });
        format!("{log}\n")
    }
}

/// Where a note comes in the order of starts: its note on's frame, its natural frame, then the
/// order the player found it in. Equal frames mean one piece found both, in song order, so the
/// order is the same however a render is split.
type Start = (u64, u64, u64);

/// A note sent and not yet ended.
#[derive(Debug, Clone, Copy)]
struct Sounding {
    channel: ChannelId,
    status_channel: u8,
    key: u8,
    /// The frame its note on went out on.
    on: u64,
    /// The frame its note off is due on.
    off: u64,
    start: Start,
}

/// A message of the piece being sent, with what it is sorted by.
#[derive(Debug, Clone, Copy)]
struct Outgoing {
    /// 0 for a note off, 1 for a note on: offs go first on a frame.
    kind: u8,
    start: Start,
    event: MidiOutEvent,
}

impl Outgoing {
    fn order(&self) -> (u64, u8, Start) {
        (self.event.frame, self.kind, self.start)
    }
}

/// Makes the MIDI-out ring's two halves; called on the thread that makes the link.
pub(crate) fn ring() -> (MidiOutState, MidiOutReader) {
    let (events, reader) = RingBuffer::new(RING_CAPACITY);
    let counts = Arc::new(Counts::default());
    let state = MidiOutState {
        events,
        counts: Arc::clone(&counts),
        sounding: vec![None; SOUNDING].into_boxed_slice(),
        outgoing: Vec::with_capacity(ONS_PER_PIECE + SOUNDING),
        found: 0,
        room: 0,
    };
    let reader = MidiOutReader {
        events: reader,
        counts,
    };
    (state, reader)
}

/// The audio thread's half: the sounding notes, the piece being sent and the ring's producer.
pub(crate) struct MidiOutState {
    events: Producer<MidiOutEvent>,
    counts: Arc<Counts>,
    sounding: Box<[Option<Sounding>]>,
    /// The piece's messages; its capacity holds a piece's note ons and every sounding note's
    /// off, so pushing never allocates.
    outgoing: Vec<Outgoing>,
    /// Notes found so far, for the order of starts.
    found: u64,
    /// Note ons the ring has room for in this piece, the reserve kept.
    room: usize,
}

/// Where the notes of one stretch of song samples land on the output frames: song sample
/// `sample` plays on output frame `frame + (sample - sample_at)`.
#[derive(Clone, Copy)]
struct Pass {
    timeline: TempoSegment,
    frame: u64,
    sample_at: u64,
    /// The output frame the pass ends on (its wrap), `None` when it plays on.
    end: Option<u64>,
    /// Its first tick: the first at or after the playhead, or the loop's start.
    first_tick: Ticks,
}

impl Pass {
    /// The first tick of the pass at or after output frame `frame`.
    fn tick_at(&self, frame: u64) -> Ticks {
        let sample = self
            .sample_at
            .saturating_add(frame.saturating_sub(self.frame));
        self.timeline
            .first_tick_from(sample)
            .unwrap_or(Ticks(i64::MAX))
    }

    /// The output frame `tick` of the pass plays on.
    fn frame_of(&self, tick: Ticks) -> u64 {
        self.timeline
            .tick_to_sample(tick)
            .map_or(u64::MAX, |sample| {
                self.frame
                    .saturating_add(sample.saturating_sub(self.sample_at))
            })
    }
}

/// How the song plays on from output frame `at`: song sample `position` of `timeline` plays
/// there, and playback wraps as `wrap` says (the player's: none while recording). Its passes
/// are the playhead's way to the wrap, then each round of the loop on the timeline restarted.
#[derive(Clone, Copy)]
pub(crate) struct Course {
    pub(crate) timeline: TempoSegment,
    pub(crate) position: u64,
    pub(crate) wrap: Option<Wrap>,
    pub(crate) at: u64,
}

impl Course {
    /// The wrap with its round's first and end samples on the restarted timeline, when the
    /// transport takes it: only a wrap whose round is not empty.
    fn looped(&self) -> Option<(Wrap, u64, u64)> {
        let wrap = self.wrap?;
        let restarted = self.timeline.restarted();
        let to = restarted.tick_to_sample(wrap.to)?;
        let back = restarted.tick_to_sample(wrap.at)?;
        (to < back).then_some((wrap, to, back))
    }

    /// The first pass: from the playhead to the wrap when the wrap lies ahead, playing on when
    /// the playhead is past it. A playhead standing on the wrap (between two calls) takes it
    /// first, as the next piece will.
    fn first(&self) -> Pass {
        let from_playhead = Pass {
            timeline: self.timeline,
            frame: self.at,
            sample_at: self.position,
            end: None,
            first_tick: self
                .timeline
                .first_tick_from(self.position)
                .unwrap_or(Ticks(i64::MAX)),
        };
        let Some((wrap, to, back)) = self.looped() else {
            return from_playhead;
        };
        let ahead = self.timeline.tick_to_sample(wrap.at);
        match ahead.and_then(|ahead| ahead.checked_sub(self.position)) {
            Some(0) => self.round(self.at, wrap, to, back),
            Some(frames) => Pass {
                end: Some(self.at.saturating_add(frames)),
                ..from_playhead
            },
            None => from_playhead,
        }
    }

    /// The pass after `pass`, when `pass` ends on the wrap: a round of the loop.
    fn after(&self, pass: &Pass) -> Option<Pass> {
        let (wrap, to, back) = self.looped()?;
        Some(self.round(pass.end?, wrap, to, back))
    }

    fn round(&self, frame: u64, wrap: Wrap, to: u64, back: u64) -> Pass {
        Pass {
            timeline: self.timeline.restarted(),
            frame,
            sample_at: to,
            end: Some(frame.saturating_add(back - to)),
            first_tick: wrap.to,
        }
    }

    /// The tick a pass of this course ends before: the wrap's, or none when it plays on.
    fn end_tick(&self, pass: &Pass) -> Ticks {
        match (pass.end, self.wrap) {
            (Some(_), Some(wrap)) => wrap.at,
            _ => Ticks(i64::MAX),
        }
    }
}

/// Where `new` must send a channel's notes from, when `old` had sent every one whose natural
/// frame is before `mark`: the output frame where the first tick `old` did not send lands on
/// `new`. Pass k of one course is matched with pass k of the other: of each, `old` sent a run of
/// ticks from its first, and `new` sends from the first of its own ticks outside that run.
///
/// Why: the mark is an output frame, worked out on `old`'s tempo and wrap. Kept on a new tempo
/// or wrap it would send a note again (its tick now later) or never (its tick now earlier).
fn first_unsent(old: &Course, new: &Course, mark: u64) -> u64 {
    let mark = mark.max(old.at);
    let (mut sent, mut sending) = (old.first(), new.first());
    loop {
        let whole = sent.end.is_some_and(|end| mark >= end);
        // `old` sent its pass's ticks from its first up to (not with) this one.
        let upto = if whole {
            old.end_tick(&sent)
        } else {
            sent.tick_at(mark)
        };
        if sending.first_tick < sent.first_tick || sending.first_tick >= upto {
            return sending.frame;
        }
        if upto < new.end_tick(&sending) {
            return sending.frame_of(upto);
        }
        // All of `new`'s pass was sent: on to the next of each, if `old` sent into its next.
        match (whole, old.after(&sent), new.after(&sending)) {
            (true, Some(next_sent), Some(next_sending)) => {
                (sent, sending) = (next_sent, next_sending);
            }
            _ => return sending.end.unwrap_or(u64::MAX),
        }
    }
}

/// After a tempo change on output frame `before.at`, which is no jump: every channel's notes sent
/// ahead were placed by the old tempo, so its mark moves to where its first unsent tick lands on
/// the new one ([`first_unsent`]). The notes sent go on as they were.
pub(crate) fn retimed(plan: &mut PlayPlan, before: &Course, after: &Course) {
    let outs = plan.channels.iter_mut();
    for out in outs.filter_map(|channel| channel.midi_out.as_mut()) {
        out.sent_until = first_unsent(before, after, out.sent_until);
    }
}

/// What a plan plays, comparable across plans (a pattern's place in one plan's list is not).
#[derive(PartialEq, Eq)]
enum Plays {
    Song,
    Pattern(PatternId),
    Nothing,
}

fn plays(plan: &PlayPlan) -> Plays {
    match plan.playing {
        Playing::Song => Plays::Song,
        Playing::Pattern(index) => plan
            .patterns
            .get(index)
            .map_or(Plays::Nothing, |pattern| Plays::Pattern(pattern.id)),
        Playing::Nothing => Plays::Nothing,
    }
}

impl MidiOutState {
    /// Sends the MIDI of the piece that plays output frames `at..at + len` from song sample
    /// `position` on `timeline`, wrapping as `wrap` says: the notes of each channel with a MIDI
    /// out whose natural frame falls in its window, and the note offs due before the piece's
    /// end.
    pub(crate) fn piece(
        &mut self,
        plan: &mut PlayPlan,
        timeline: &TempoSegment,
        position: u64,
        wrap: Option<Wrap>,
        at: u64,
        len: usize,
    ) {
        let end = at.saturating_add(len as u64);
        let (mut from, mut until) = (u64::MAX, 0);
        for out in plan.channels.iter().filter_map(|channel| channel.midi_out) {
            from = from.min(out.sent_until.max(at));
            until = until.max(end.saturating_add(out.latency_frames));
        }
        if from < until {
            self.room = self.events.slots().saturating_sub(OFF_RESERVE);
            let course = Course {
                timeline: *timeline,
                position,
                wrap,
                at,
            };
            self.find(plan, &course, end, from, until);
        }
        for channel in &mut plan.channels {
            if let Some(out) = &mut channel.midi_out {
                out.sent_until = out.sent_until.max(end.saturating_add(out.latency_frames));
            }
        }
        self.end_where(|note| (note.off < end).then_some(note.off));
        self.send();
    }

    /// Sends the notes with natural frames in `from..until`, pass after pass of `course`, in a
    /// piece that starts on `course.at` and ends before `end`.
    fn find(&mut self, plan: &PlayPlan, course: &Course, end: u64, from: u64, until: u64) {
        let mut pass = course.first();
        loop {
            let low = from.max(pass.frame);
            let high = until.min(pass.end.unwrap_or(u64::MAX));
            if low < high {
                self.pass_notes(plan, &pass, course.at, end, low, high);
            }
            match course.after(&pass) {
                Some(next) if next.frame < until => pass = next,
                _ => break,
            }
        }
    }

    /// Sends the notes of `pass` whose natural frames are in `low..high`, in song order.
    fn pass_notes(&mut self, plan: &PlayPlan, pass: &Pass, at: u64, end: u64, low: u64, high: u64) {
        let timeline = &pass.timeline;
        let first = pass.sample_at.saturating_add(low - pass.frame);
        let last = pass.sample_at.saturating_add(high - pass.frame);
        // From the tick a new tempo starts on, for a playhead still before it.
        let Some(from) = timeline.first_tick_from(first) else {
            return;
        };
        let natural = |sample: u64| pass.frame.saturating_add(sample - pass.sample_at);
        match plan.playing {
            Playing::Pattern(index) => {
                let Some(pattern) = plan.patterns.get(index) else {
                    return;
                };
                let mut walk = EventWalk::from_tick(pattern, from);
                while let Some((sample, tick, event)) = walk.next_before(timeline, last) {
                    let gate = gate_frames(timeline, plan.ppq, sample, tick, event, None);
                    self.offer(
                        plan,
                        event.channel,
                        event.key,
                        event.velocity,
                        natural(sample),
                        gate,
                        at,
                        end,
                    );
                }
            }
            Playing::Song => {
                let to = timeline
                    .first_tick_at_or_after(last)
                    .unwrap_or(Ticks(i64::MAX));
                let mut walk = SongWalk::new(plan, from, to);
                while let Some((sample, tick, event, clip_end)) = walk.next_before(timeline, last) {
                    let gate = gate_frames(timeline, plan.ppq, sample, tick, event, Some(clip_end));
                    self.offer(
                        plan,
                        event.channel,
                        event.key,
                        event.velocity,
                        natural(sample),
                        gate,
                        at,
                        end,
                    );
                }
            }
            Playing::Nothing => {}
        }
    }

    /// Sends the note of `key` found on `channel` at `natural` frame, sounding `gate` frames,
    /// when the channel sends it there: it has a MIDI out, is not muted, and the frame is in the
    /// channel's window. Sent its latency early, never before `at`; its note off its latency
    /// before its end, and after its note on: for a note whose note on had to wait for `at` (the
    /// jump frame), at least its whole gate after it.
    #[expect(
        clippy::too_many_arguments,
        reason = "one found note and the piece it is found in"
    )]
    fn offer(
        &mut self,
        plan: &PlayPlan,
        channel: ChannelId,
        key: u8,
        velocity: f32,
        natural: u64,
        gate: u64,
        at: u64,
        end: u64,
    ) {
        let Some(target) = plan.channel(channel) else {
            return;
        };
        let Some(out) = target.midi_out.filter(|_| !target.muted) else {
            return;
        };
        let latency = out.latency_frames;
        if natural < out.sent_until.max(at) || natural >= end.saturating_add(latency) {
            return;
        }
        // Sent already and still sounding: found again because a plan swapped away and back
        // within the latency (another pattern and back, the loop off and on) moved the mark back
        // over it. Struck once; a second note on would end it early and strike it again.
        let sent = |note: &Sounding| {
            let (_, its_natural, _) = note.start;
            note.channel == channel && note.key == key && its_natural == natural
        };
        if self.sounding.iter().flatten().any(sent) {
            return;
        }
        let early = natural.checked_sub(latency).filter(|&early| early >= at);
        let on = early.unwrap_or(at);
        // A note whose note on had to wait for the jump frame keeps its full length: its latency
        // before its end could leave it a one-frame blip, or nothing.
        let shortest = if early.is_some() { 1 } else { gate };
        let off = natural
            .saturating_add(gate)
            .saturating_sub(latency)
            .max(on.saturating_add(shortest));
        self.found += 1;
        let start = (on, natural, self.found);
        // The key's latest note on this channel, while it still sounds at `on`.
        let before = self
            .sounding
            .iter()
            .enumerate()
            .filter_map(|(slot, note)| note.map(|note| (slot, note)))
            .filter(|(_, note)| note.channel == channel && note.key == key && note.off >= on)
            .max_by_key(|(_, note)| note.start);
        if before.is_some_and(|(_, note)| note.on == on) {
            // A second note on of the key on the frame its first went out on: left out.
            return;
        }
        let free = self.sounding.iter().position(Option::is_none);
        let fits = self.room > 0 && self.outgoing.len() < ONS_PER_PIECE;
        let Some(free) = free.filter(|_| fits) else {
            self.counts.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        };
        if let Some((slot, _)) = before {
            // Ended where the key starts again, before the new note on.
            if let Some(note) = &mut self.sounding[slot] {
                note.off = on;
            }
        }
        self.room -= 1;
        self.sounding[free] = Some(Sounding {
            channel,
            status_channel: out.status_channel,
            key,
            on,
            off,
            start,
        });
        self.outgoing.push(Outgoing {
            kind: 1,
            start,
            event: MidiOutEvent::note_on(on, channel, out.status_channel, key, velocity),
        });
    }

    /// Ends every sounding note on `at`: the playhead jumped. Each channel's notes are sent
    /// from `at` on again.
    pub(crate) fn jump(&mut self, plan: &mut PlayPlan, at: u64) {
        self.end_where(|_| Some(at));
        self.send();
        for channel in &mut plan.channels {
            if let Some(out) = &mut channel.midi_out {
                out.sent_until = at;
            }
        }
    }

    /// After `new` replaced `old` on output frame `at`: the notes of channels whose MIDI out
    /// `new` took away or changed end on `at`, and those channels' notes are sent from `at` on;
    /// the others go on where they were. While playing (`courses`: the song's course under each
    /// plan), a plan that plays other notes (another pattern, the song for a pattern) has sent
    /// none of them, so they too are sent from `at` on, and one that moves the wrap sends from
    /// its first tick not yet sent ([`first_unsent`]). The notes sent go on as they were.
    pub(crate) fn swapped(
        &mut self,
        old: &PlayPlan,
        new: &mut PlayPlan,
        at: u64,
        courses: Option<(Course, Course)>,
    ) {
        let route = |plan: &PlayPlan, channel: ChannelId| {
            plan.channel(channel)
                .and_then(|target| target.midi_out)
                .map(|out| (out.status_channel, out.latency_frames))
        };
        self.end_where(|note| {
            let kept = route(new, note.channel)
                .is_some_and(|routed| route(old, note.channel) == Some(routed));
            (!kept).then_some(at)
        });
        self.send();
        let other_notes = plays(old) != plays(new);
        for channel in &mut new.channels {
            let id = channel.id;
            if let Some(out) = &mut channel.midi_out {
                let before = old.channel(id).and_then(|target| target.midi_out);
                out.sent_until = match before {
                    Some(before) if before.sends_as(out) => match &courses {
                        Some(_) if other_notes => at,
                        Some((was, now)) if was.wrap != now.wrap => {
                            first_unsent(was, now, before.sent_until)
                        }
                        _ => before.sent_until,
                    },
                    _ => at,
                };
            }
        }
    }

    /// Sends a live message now; `false` when the ring had no room for it (one that ends nothing
    /// leaves the reserve free).
    pub(crate) fn live(&mut self, event: MidiOutEvent) -> bool {
        let room = if event.ends() { 1 } else { OFF_RESERVE + 1 };
        if self.events.slots() < room || self.events.push(event).is_err() {
            count_lost(&self.counts, &event);
            return false;
        }
        true
    }

    /// Moves the sounding notes `due` gives a frame for (their own note off's, or another) out
    /// of the table, their note offs into the piece on that frame.
    fn end_where(&mut self, due: impl Fn(&Sounding) -> Option<u64>) {
        for slot in self.sounding.iter_mut() {
            let Some(note) = *slot else { continue };
            let Some(frame) = due(&note) else { continue };
            *slot = None;
            // The capacity holds every sounding note's off besides a piece's note ons.
            self.outgoing.push(Outgoing {
                kind: 0,
                start: note.start,
                event: MidiOutEvent::note_off(frame, note.channel, note.status_channel, note.key),
            });
        }
    }

    /// Sorts the piece's messages (frame, offs first, start order) into the ring.
    fn send(&mut self) {
        // Unstable is exact here (no two messages share an order) and, unlike a stable sort,
        // never allocates.
        let Self {
            events,
            counts,
            outgoing,
            ..
        } = self;
        outgoing.sort_unstable_by_key(Outgoing::order);
        for outgoing in outgoing.drain(..) {
            // Only a stalled reader leaves no room: a note on was given room in its piece.
            if events.push(outgoing.event).is_err() {
                count_lost(counts, &outgoing.event);
            }
        }
    }
}

/// Counts `event`, which did not go out: one that ends something as lost, any other dropped.
fn count_lost(counts: &Counts, event: &MidiOutEvent) {
    let counter = if event.ends() {
        &counts.lost_offs
    } else {
        &counts.dropped
    };
    counter.fetch_add(1, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    // A message is three bytes of a note: status with the MIDI channel, key, velocity byte.
    #[test]
    fn note_messages_hold_their_bytes() {
        let on = MidiOutEvent::note_on(5, ChannelId(2), 1, 60, 100.0 / 128.0);
        assert_eq!((on.frame, on.channel), (5, ChannelId(2)));
        assert_eq!(on.bytes(), [0x91, 60, 100]);
        assert!(on.is_note_on());
        let loud = MidiOutEvent::note_on(0, ChannelId(1), 0, 67, 1.0);
        assert_eq!(loud.bytes(), [0x90, 67, 127], "128 is held to 127");
        let off = MidiOutEvent::note_off(9, ChannelId(1), 15, 72);
        assert_eq!(off.bytes(), [0x8F, 72, 0x40]);
        assert!(!off.is_note_on());
        assert!(off.ends() && !on.ends());
    }

    // By hand from the MIDI 1.0 status bytes: Bn 40 vv is the sustain pedal, down from 64; its
    // lift ends notes (it may take the reserve), its press does not.
    #[test]
    fn the_pedal_holds_its_bytes_and_its_lift_ends() {
        let down = MidiOutEvent::sustain(3, ChannelId(1), 2, 127);
        assert_eq!(down.bytes(), [0xB2, 64, 127]);
        assert!(!down.ends() && !down.is_note_on());
        assert!(MidiOutEvent::sustain(3, ChannelId(1), 0, 63).ends());
        assert!(!MidiOutEvent::sustain(3, ChannelId(1), 0, 64).ends());
        let wheel = MidiOutEvent::mod_wheel(3, ChannelId(1), 15, 0);
        assert_eq!(wheel.bytes(), [0xBF, 1, 0]);
        assert!(!wheel.ends(), "the wheel at rest ends nothing");
    }

    // By hand: En ll mm carries value + 8192 in 14 bits, low 7 first: 8191 is 7F 7F, -8192 is
    // 00 00, the rest 00 40. None of them ends a note.
    #[test]
    fn pitch_bend_holds_its_fourteen_bits() {
        let bytes = |value| {
            MidiOutEvent::pitch_bend(0, ChannelId(1), 3, value)
                .bytes()
                .to_vec()
        };
        assert_eq!(bytes(8191), [0xE3, 0x7F, 0x7F]);
        assert_eq!(bytes(-8192), [0xE3, 0x00, 0x00]);
        assert_eq!(bytes(0), [0xE3, 0x00, 0x40]);
        assert_eq!(bytes(1), [0xE3, 0x01, 0x40]);
        assert!(!MidiOutEvent::pitch_bend(0, ChannelId(1), 3, 0).ends());
    }
}
