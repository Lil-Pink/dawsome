//! Standard MIDI files. A file's notes are read into one channel of a pattern, as one edit; a
//! pattern's notes for one channel are written as a format 0 file. Pure: bytes in, an edit or
//! bytes out; the file dialogs and the disk are the caller's.
//!
//! Reading merges every track and every MIDI channel into the one channel, since each channel
//! of a project needs a sound of its own: a track per channel would make silent channels.
//! Positions and lengths are rescaled to the project's PPQ by the one half-up rule. Tempo, time
//! signature, program, controller and pitch-bend events are not applied; the report lists them.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use midly::num::{u15, u24, u28, u4, u7};
use midly::{Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};
use thiserror::Error;

use crate::edit::Edit;
use crate::project::{ChannelId, Note, PatternId, Project, MAX_SONG_BARS};
use crate::song;
use crate::time::{self, MilliBpm, Ticks};

/// The division an exported file is written at.
pub const EXPORT_PPQ: u16 = 96;

/// The velocity a note-off is written with: the middle of the range, as most writers use.
const OFF_VELOCITY: u8 = 64;

/// Why a MIDI file could not be read or written.
#[derive(Debug, Error)]
pub enum SmfError {
    #[error("not a standard MIDI file: {0}")]
    Parse(String),
    #[error("the file is timed in SMPTE frames, not beats; only beat-timed files are read")]
    Smpte,
    #[error("there is no pattern {0}")]
    UnknownPattern(u32),
    #[error("there is no channel {0}")]
    UnknownChannel(u32),
    #[error("the file has no notes, and the channel none to replace")]
    NothingToImport,
    #[error("a note lies past bar {max}, the end of the longest song", max = MAX_SONG_BARS)]
    TooLong,
    #[error("the file could not be written: {0}")]
    Write(#[from] std::io::Error),
}

/// What reading a file did beyond placing its notes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SmfReport {
    /// Notes placed.
    pub notes: u32,
    /// Notes left out because an earlier one (by track, then event) landed on the same tick and
    /// key.
    pub dropped: u32,
    /// Notes without a note-off, ended where their track ends.
    pub unclosed: u32,
    /// Program, controller, pitch-bend, aftertouch and system-exclusive events, not applied.
    pub ignored: u32,
    /// The file's first tempo, not applied (the project keeps its own).
    pub tempo: Option<MilliBpm>,
    /// The file's first time signature as (beats, beat unit), not applied.
    pub time_signature: Option<(u8, u16)>,
}

impl SmfReport {
    /// One line: what was placed, then what was not, such as "4 notes imported; 1 overlapping
    /// note dropped, 1 note closed at track end, 3 other events ignored, tempo 120 BPM not
    /// applied".
    pub fn summary(&self) -> String {
        let count =
            |n: u32, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let mut left = Vec::new();
        if self.dropped > 0 {
            left.push(count(
                self.dropped,
                "overlapping note dropped",
                "overlapping notes dropped",
            ));
        }
        if self.unclosed > 0 {
            left.push(count(
                self.unclosed,
                "note closed at track end",
                "notes closed at track end",
            ));
        }
        if self.ignored > 0 {
            left.push(count(
                self.ignored,
                "other event ignored",
                "other events ignored",
            ));
        }
        if let Some(tempo) = self.tempo {
            left.push(format!("tempo {} BPM not applied", bpm_text(tempo)));
        }
        if let Some((beats, unit)) = self.time_signature {
            left.push(format!("time signature {beats}/{unit} not applied"));
        }
        let placed = count(self.notes, "note imported", "notes imported");
        if left.is_empty() {
            placed
        } else {
            format!("{placed}; {}", left.join(", "))
        }
    }
}

/// "120" for 120000 milli-BPM, "93.217" for 93217.
fn bpm_text(tempo: MilliBpm) -> String {
    let (whole, thousandths) = (tempo.0 / 1000, tempo.0 % 1000);
    if thousandths == 0 {
        whole.to_string()
    } else {
        format!("{whole}.{thousandths:03}")
            .trim_end_matches('0')
            .to_owned()
    }
}

/// A file read for one pattern and channel: the edit that places it, one undo step, and what
/// it left out.
#[derive(Debug, Clone, PartialEq)]
pub struct SmfImport {
    pub edit: Edit,
    pub report: SmfReport,
}

/// A note as read, before rescaling: where it is in the file (track, event of its note-on),
/// its start and length at the file's PPQ, its velocity byte and key.
struct ReadNote {
    order: (usize, usize),
    start: u64,
    length: u64,
    velocity: u8,
    key: u8,
}

/// Reads `bytes` into `channel` of `pattern`: the channel's notes in the pattern are replaced by
/// the file's, and the pattern grows to hold the last of them (as a piano-roll edit grows it), as
/// one edit.
///
/// A note-off (or a note-on at velocity 0) ends the oldest open note of its MIDI channel and
/// key in its track; a note never ended ends at its track's end. Velocity is the byte / 128.
pub fn import_into(
    project: &Project,
    pattern: PatternId,
    channel: ChannelId,
    bytes: &[u8],
) -> Result<SmfImport, SmfError> {
    let shown = project
        .patterns
        .get(&pattern)
        .ok_or(SmfError::UnknownPattern(pattern.0))?;
    if !project.channels.contains_key(&channel) {
        return Err(SmfError::UnknownChannel(channel.0));
    }
    let smf = Smf::parse(bytes).map_err(|error| SmfError::Parse(error.to_string()))?;
    let file_ppq = match smf.header.timing {
        Timing::Metrical(ppq) if ppq.as_int() > 0 => ppq.as_int(),
        Timing::Metrical(_) => return Err(SmfError::Parse("division 0".into())),
        Timing::Timecode(..) => return Err(SmfError::Smpte),
    };
    let mut report = SmfReport::default();
    let mut read = Vec::new();
    for (track_index, track) in smf.tracks.iter().enumerate() {
        read_track(track_index, track, &mut read, &mut report)?;
    }
    // In note-on order, track then event, so the first of two notes on one (tick, key) stays.
    read.sort_by_key(|note| note.order);
    let rescale = |ticks: u64| {
        time::rescale_ticks(ticks, file_ppq, project.ppq)
            .and_then(|ticks| i64::try_from(ticks).ok())
            .ok_or(SmfError::TooLong)
    };
    let mut seen = BTreeSet::new();
    let mut added = Vec::with_capacity(read.len());
    for note in read {
        let start = rescale(note.start)?;
        if !seen.insert((start, note.key)) {
            report.dropped += 1;
            continue;
        }
        added.push(Note {
            channel,
            start: Ticks(start),
            length: Ticks(rescale(note.length)?),
            velocity: f32::from(note.velocity) / 128.0,
            key: note.key,
        });
    }
    added.sort_by_key(|note| (note.start, note.key));
    report.notes = u32::try_from(added.len()).unwrap_or(u32::MAX);
    let removed: Vec<Note> = shown
        .notes
        .iter()
        .filter(|note| note.channel == channel)
        .cloned()
        .collect();
    if removed.is_empty() && added.is_empty() {
        return Err(SmfError::NothingToImport);
    }
    let bars = bars_holding(&added, project.ppq)?;
    let replace = Edit::ReplaceNotes {
        pattern,
        remove: removed,
        add: added,
    };
    let edit = if bars > shown.length_bars {
        Edit::Transaction(vec![replace, Edit::SetPatternBars { pattern, bars }])
    } else {
        replace
    };
    Ok(SmfImport { edit, report })
}

/// A note-on waiting for its note-off: its tick, velocity byte and event index in its track.
struct OpenNote {
    start: u64,
    velocity: u8,
    event: usize,
}

impl OpenNote {
    /// The note as read, ended on `tick`.
    fn ended(self, track: usize, tick: u64, key: u8) -> ReadNote {
        ReadNote {
            order: (track, self.event),
            start: self.start,
            length: tick - self.start,
            velocity: self.velocity,
            key,
        }
    }
}

/// The notes of one track, in absolute ticks at the file's PPQ, into `read`; what is left out
/// into `report`.
fn read_track(
    track_index: usize,
    track: &[TrackEvent<'_>],
    read: &mut Vec<ReadNote>,
    report: &mut SmfReport,
) -> Result<(), SmfError> {
    // The notes waiting for their note-off, oldest first, by (MIDI channel, key).
    let mut open: BTreeMap<(u8, u8), VecDeque<OpenNote>> = BTreeMap::new();
    let mut tick = 0u64;
    for (index, event) in track.iter().enumerate() {
        tick = tick
            .checked_add(u64::from(event.delta.as_int()))
            .ok_or(SmfError::TooLong)?;
        match event.kind {
            TrackEventKind::Midi { channel, message } => {
                let channel = channel.as_int();
                match message {
                    MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                        open.entry((channel, key.as_int()))
                            .or_default()
                            .push_back(OpenNote {
                                start: tick,
                                velocity: vel.as_int(),
                                event: index,
                            });
                    }
                    MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => {
                        let waiting = open
                            .get_mut(&(channel, key.as_int()))
                            .and_then(VecDeque::pop_front);
                        if let Some(note) = waiting {
                            read.push(note.ended(track_index, tick, key.as_int()));
                        }
                    }
                    _ => report.ignored += 1,
                }
            }
            TrackEventKind::Meta(MetaMessage::Tempo(per_quarter)) => {
                if report.tempo.is_none() {
                    report.tempo = tempo_of(per_quarter.as_int());
                }
            }
            TrackEventKind::Meta(MetaMessage::TimeSignature(beats, unit_power, ..)) => {
                if report.time_signature.is_none() {
                    let unit = 1u16.checked_shl(u32::from(unit_power)).unwrap_or(0);
                    report.time_signature = Some((beats, unit));
                }
            }
            TrackEventKind::Meta(_) => {}
            TrackEventKind::SysEx(_) | TrackEventKind::Escape(_) => report.ignored += 1,
        }
    }
    // The track ends on its last event.
    for ((_, key), waiting) in open {
        for note in waiting {
            report.unclosed += 1;
            read.push(note.ended(track_index, tick, key));
        }
    }
    Ok(())
}

/// Microseconds per quarter note as milli-BPM, rounded half up: 6·10^10 / µs.
fn tempo_of(per_quarter: u32) -> Option<MilliBpm> {
    let per_quarter = u64::from(per_quarter);
    if per_quarter == 0 {
        return None;
    }
    let milli = (60_000_000_000 + per_quarter / 2) / per_quarter;
    u32::try_from(milli).ok().map(MilliBpm)
}

/// The bars a pattern needs to hold every note, a zero-length one counting a tick: at least 1,
/// and at most [`MAX_SONG_BARS`]. Why the cap: one variable-length delta can place a note
/// 67 million bars out, and a pattern that long is more than the window can draw.
fn bars_holding(notes: &[Note], ppq: u16) -> Result<u32, SmfError> {
    let bar = u64::try_from(time::bars_to_ticks(1, ppq).0.max(1)).unwrap_or(1);
    let last_end = notes
        .iter()
        .map(|note| note.start.0.saturating_add(note.length.0.max(1)))
        .max()
        .unwrap_or(0);
    // Imported notes start at tick 0 or later, so the end is never negative.
    let bars = u64::try_from(last_end).unwrap_or(0).div_ceil(bar).max(1);
    u32::try_from(bars)
        .ok()
        .filter(|&bars| bars <= MAX_SONG_BARS)
        .ok_or(SmfError::TooLong)
}

/// The notes `channel` plays in `pattern` (those inside the pattern, which are heard) as a
/// format 0 file at [`EXPORT_PPQ`], on MIDI channel 1. Tick 0 carries the tempo, 4/4 and the
/// pattern's name; a step (length 0) is written one step long; velocity is round-half-up(v ·
/// 128) from 1 to 127; at one tick note-offs come first, then by key; the track ends on its last
/// event.
pub fn export(
    project: &Project,
    pattern: PatternId,
    channel: ChannelId,
) -> Result<Vec<u8>, SmfError> {
    let shown = project
        .patterns
        .get(&pattern)
        .ok_or(SmfError::UnknownPattern(pattern.0))?;
    if !project.channels.contains_key(&channel) {
        return Err(SmfError::UnknownChannel(channel.0));
    }
    let to_file = |ticks: i64| {
        u64::try_from(ticks)
            .ok()
            .and_then(|ticks| time::rescale_ticks(ticks, project.ppq, EXPORT_PPQ))
            .ok_or(SmfError::TooLong)
    };
    let end = time::bars_to_ticks(shown.length_bars, project.ppq);
    let step = time::step_to_ticks(1, project.ppq);
    let mut timed: Vec<Timed> = Vec::new();
    for note in shown
        .notes
        .iter()
        .filter(|note| note.channel == channel && note.start < end)
    {
        let length = if note.length.0 > 0 { note.length } else { step };
        let on = to_file(note.start.0)?;
        let off = to_file(note.start.0.saturating_add(length.0))?;
        push_note(
            &mut timed,
            on,
            at_least_a_tick(on, off),
            note.key,
            note.velocity,
        );
    }
    timed.sort_by_key(|&(tick, kind, key, _)| (tick, kind, key));
    let mut track = vec![
        meta(MetaMessage::Tempo(per_quarter(project.tempo)?)),
        meta(MetaMessage::TimeSignature(4, 2, 24, 8)),
        meta(MetaMessage::TrackName(shown.name.as_bytes())),
    ];
    push_timed(&mut track, timed, None)?;
    let smf = Smf {
        header: Header::new(Format::SingleTrack, Timing::Metrical(u15::new(EXPORT_PPQ))),
        tracks: vec![track],
    };
    let mut bytes = Vec::new();
    smf.write_std(&mut bytes)?;
    Ok(bytes)
}

/// A note-on and its note-off, timed and ordered as both exports write them: (tick, 0 for a
/// note-off or 1 for a note-on, key, the message).
type Timed = (u64, u8, u8, MidiMessage);

/// A meta event at no distance from the event before it.
fn meta(kind: MetaMessage<'_>) -> TrackEvent<'_> {
    TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(kind),
    }
}

/// The note-on at `on` and the note-off at `off` of `key` into `timed`; velocity
/// round-half-up(v · 128) from 1 to 127, the note-off at [`OFF_VELOCITY`].
fn push_note(timed: &mut Vec<Timed>, on: u64, off: u64, key: u8, velocity: f32) {
    let byte = (f64::from(velocity) * 128.0 + 0.5)
        .floor()
        .clamp(1.0, 127.0) as u8;
    let key7 = u7::new(key);
    timed.push((
        on,
        1,
        key,
        MidiMessage::NoteOn {
            key: key7,
            vel: u7::new(byte),
        },
    ));
    timed.push((
        off,
        0,
        key,
        MidiMessage::NoteOff {
            key: key7,
            vel: u7::new(OFF_VELOCITY),
        },
    ));
}

/// `timed` (sorted) onto `track` on MIDI channel 1, then the end of the track: on its last
/// event, or on tick `end` when given (at or after the last event).
fn push_timed(
    track: &mut Vec<TrackEvent<'_>>,
    timed: Vec<Timed>,
    end: Option<u64>,
) -> Result<(), SmfError> {
    let delta_to = |tick: u64, last: u64| {
        u32::try_from(tick - last)
            .ok()
            .filter(|&delta| delta <= u28::max_value().as_int())
            .map(u28::new)
            .ok_or(SmfError::TooLong)
    };
    let mut last = 0u64;
    for (tick, _, _, message) in timed {
        let delta = delta_to(tick, last)?;
        last = tick;
        track.push(TrackEvent {
            delta,
            kind: TrackEventKind::Midi {
                channel: u4::new(0),
                message,
            },
        });
    }
    track.push(TrackEvent {
        delta: delta_to(end.unwrap_or(last).max(last), last)?,
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });
    Ok(())
}

/// A note's end on the file's ticks, at least one tick after its start. Why: at a project PPQ
/// finer than the file's, both ends of a short note can round to one tick, and on one tick a
/// note-off is written before a note-on, so a reader would pair the note-on with the next
/// note-off of its key, or leave it sounding.
fn at_least_a_tick(start: u64, end: u64) -> u64 {
    end.max(start.saturating_add(1))
}

/// The tempo as microseconds per quarter note, rounded half up: 6·10^10 / milli-BPM.
fn per_quarter(tempo: MilliBpm) -> Result<u24, SmfError> {
    let tempo = u64::from(tempo.0).max(1);
    u32::try_from((60_000_000_000 + tempo / 2) / tempo)
        .ok()
        .filter(|&micros| micros <= u24::max_value().as_int())
        .map(u24::new)
        .ok_or(SmfError::TooLong)
}

/// What [`export_song`] wrote, and what it left out or changed on the way.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SongFile {
    pub bytes: Vec<u8>,
    /// Notes written.
    pub notes: u32,
    /// Pattern clips left out because they are muted or their track is.
    pub muted_clips: u32,
    /// Notes ended early because their key started again on their channel while they sounded.
    pub shortened: u32,
    /// Notes left out because an earlier one (in clip order) started on the same tick and key
    /// of their channel.
    pub dropped: u32,
    /// The notes of channels whose sound is not notes (audio and input channels), left out, by
    /// channel.
    pub not_notes: BTreeMap<ChannelId, u32>,
}

/// A note of the song for one channel, as written: start and end on the file's ticks, the
/// clip's place in timeline order, velocity and key.
struct SongNote {
    start: u64,
    end: u64,
    order: usize,
    velocity: f32,
    key: u8,
}

/// The playlist as a format 1 file at [`EXPORT_PPQ`]: track 0 the conductor (the tempo and 4/4
/// on tick 0), then one track per channel in rack order, named after it, every note on MIDI
/// channel 1. Each pattern clip that plays is unrolled as playback plays it ([`song`]): a clip
/// longer than its pattern repeats it, a note sounds its length (a step hit one step), cut at its
/// clip's end. An audio or input channel gets its track, empty. Per channel and key, in (tick,
/// clip order) order: a second note on the same tick is left out (the first stays), and a note
/// still sounding when the same key starts again ends there, since a MIDI file cannot tell two
/// such notes apart. Velocity and the order on one tick are as [`export`] writes them; every
/// track ends on the song's last note-off.
pub fn export_song(project: &Project) -> Result<SongFile, SmfError> {
    let to_file = |ticks: Ticks| {
        u64::try_from(ticks.0)
            .ok()
            .and_then(|ticks| time::rescale_ticks(ticks, project.ppq, EXPORT_PPQ))
            .ok_or(SmfError::TooLong)
    };
    let mut file = SongFile::default();
    let mut clips: Vec<_> = project
        .playlist
        .clips
        .values()
        .filter(|clip| clip.pattern().is_some())
        .collect();
    let all = clips.len();
    clips.retain(|clip| project.clip_plays(clip));
    file.muted_clips = u32::try_from(all - clips.len()).unwrap_or(u32::MAX);
    // Timeline order: by start, then clip ID (the map's order; the sort is stable).
    clips.sort_by_key(|clip| clip.start);
    let mut by_channel: BTreeMap<ChannelId, Vec<SongNote>> = BTreeMap::new();
    for (order, clip) in clips.iter().enumerate() {
        for note in song::clip_notes(project, clip) {
            let Some(channel) = project.channels.get(&note.channel) else {
                continue;
            };
            if !channel.plays_notes() {
                *file.not_notes.entry(note.channel).or_default() += 1;
                continue;
            }
            let start = to_file(note.start)?;
            by_channel.entry(note.channel).or_default().push(SongNote {
                start,
                end: at_least_a_tick(start, to_file(note.end)?),
                order,
                velocity: note.velocity,
                key: note.key,
            });
        }
    }
    let mut timed_by_channel = BTreeMap::new();
    let mut end = 0u64;
    for (channel, mut notes) in by_channel {
        notes.sort_by_key(|note| (note.key, note.start, note.order));
        let mut kept: Vec<SongNote> = Vec::with_capacity(notes.len());
        for note in notes {
            if let Some(last) = kept.last_mut().filter(|last| last.key == note.key) {
                if last.start == note.start {
                    file.dropped += 1;
                    continue;
                }
                if last.end > note.start {
                    last.end = note.start;
                    file.shortened += 1;
                }
            }
            kept.push(note);
        }
        let mut timed = Vec::with_capacity(kept.len() * 2);
        for note in &kept {
            push_note(&mut timed, note.start, note.end, note.key, note.velocity);
            end = end.max(note.end);
        }
        timed.sort_by_key(|&(tick, kind, key, _)| (tick, kind, key));
        file.notes += u32::try_from(kept.len()).unwrap_or(u32::MAX);
        timed_by_channel.insert(channel, timed);
    }
    let mut conductor = vec![
        meta(MetaMessage::Tempo(per_quarter(project.tempo)?)),
        meta(MetaMessage::TimeSignature(4, 2, 24, 8)),
    ];
    push_timed(&mut conductor, Vec::new(), Some(end))?;
    let mut tracks = vec![conductor];
    for (id, channel) in &project.channels {
        let mut track = vec![meta(MetaMessage::TrackName(channel.name.as_bytes()))];
        let timed = timed_by_channel.remove(id).unwrap_or_default();
        push_timed(&mut track, timed, Some(end))?;
        tracks.push(track);
    }
    let smf = Smf {
        header: Header::new(Format::Parallel, Timing::Metrical(u15::new(EXPORT_PPQ))),
        tracks,
    };
    smf.write_std(&mut file.bytes)?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tempos_round_half_up_both_ways() {
        assert_eq!(tempo_of(500_000), Some(MilliBpm(120_000)));
        assert_eq!(tempo_of(0), None);
        // 6e10 / 643 = 93312597.2..., so 93312597 milli-BPM.
        assert_eq!(tempo_of(643), Some(MilliBpm(93_312_597)));
        assert_eq!(bpm_text(MilliBpm(120_000)), "120");
        assert_eq!(bpm_text(MilliBpm(93_217)), "93.217");
        assert_eq!(bpm_text(MilliBpm(120_500)), "120.5");
    }

    #[test]
    fn the_summary_names_only_what_happened() {
        let report = SmfReport {
            notes: 1,
            ..SmfReport::default()
        };
        assert_eq!(report.summary(), "1 note imported");
        let report = SmfReport {
            notes: 4,
            dropped: 2,
            unclosed: 1,
            ignored: 1,
            tempo: Some(MilliBpm(140_000)),
            time_signature: Some((3, 4)),
        };
        assert_eq!(
            report.summary(),
            "4 notes imported; 2 overlapping notes dropped, 1 note closed at track end, 1 other event ignored, tempo 140 BPM not applied, time signature 3/4 not applied"
        );
    }
}
