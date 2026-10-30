//! Recording notes. Every live note the player reports playing, with the tick it was heard at,
//! becomes a note of the take's pattern as soon as it ends: one note edit, heard from the
//! player's next call on, and drawn in the piano roll from then on. When the take stops, the notes
//! still held end there and the whole take becomes one undo step.
//!
//! In pattern mode the take writes into the selected pattern as it loops: a note starts where
//! it was heard within the pattern, moved to the nearest snap line when quantizing (a start
//! snapped onto the pattern's end wraps to its start). In song mode the take makes a pattern of
//! its own, "<channel> take N", and one clip of it from the song marker ([`takes::song_take`]):
//! a note starts where it was heard from the marker (quantized the same way, never wrapped), and
//! the pattern and its clip grow by whole bars as the playhead goes ([`NoteTake::follow`]),
//! holding the bar it is in; at the stop they hold what was played, and a take with no note takes
//! them out again (nothing is recorded). Either way a note is as long as it was played,
//! unquantized, so quantizing moves only the start; its velocity is the one it was played at; a
//! note on the same tick and key replaces the one there. Notes heard during the count-in, and
//! notes on a channel that plays no notes (an audio clips' channel), are not recorded.

use std::collections::BTreeMap;

use dawsome_engine::edit::{takes, Edit};
use dawsome_engine::project::{ChannelId, ClipId, Note, PatternId, Project, TrackId};
use dawsome_engine::realtime::EngineEvent;
use dawsome_engine::time::{self, Ticks};

use crate::session::{Session, SessionError, TakeParts};
use crate::ui::note_gesture::snap_nearest;

/// Where a take of notes writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Into {
    /// The selected pattern, as it loops.
    Pattern(PatternId),
    /// The take's own pattern and its clip from tick `start`, `bars` long so far.
    Song {
        pattern: PatternId,
        clip: ClipId,
        /// The track the take added, when no track had room.
        added_track: Option<TrackId>,
        start: Ticks,
        bars: u32,
    },
}

/// A take of notes.
#[derive(Debug)]
pub struct NoteTake {
    into: Into,
    /// The snap unit starts are quantized to; `None` keeps them where they were played.
    quantize: Option<Ticks>,
    /// The keys held, by channel and key: the tick each was heard from, and its velocity.
    held: BTreeMap<(ChannelId, u8), (Ticks, f32)>,
    /// The inverses of every edit the take applied so far, oldest first.
    undo: Vec<Edit>,
    /// Notes written so far.
    written: usize,
    /// The furthest playhead tick the take heard of (song mode): what it holds.
    reached: Ticks,
}

/// What a take of notes left once it stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finished {
    /// Its notes, one undo step.
    Kept,
    /// No note: the document is as it was before the take, with no undo step.
    Nothing,
}

impl NoteTake {
    /// A take into `pattern` as it loops (pattern mode).
    pub fn new(pattern: PatternId, quantize: Option<Ticks>) -> Self {
        Self {
            into: Into::Pattern(pattern),
            quantize,
            held: BTreeMap::new(),
            undo: Vec::new(),
            written: 0,
            reached: Ticks(0),
        }
    }

    /// A take in song mode from tick `start`: its pattern `name` and its clip, a bar long, on
    /// `track` (the armed one) when the project has it, else as [`takes::song_take`] places it,
    /// applied now so the roll and the playlist show them.
    pub fn song(
        session: &mut Session,
        name: String,
        start: Ticks,
        track: Option<TrackId>,
        quantize: Option<Ticks>,
    ) -> Result<Self, SessionError> {
        let take = takes::song_take(session.project(), name, start, track, 1)?;
        let inverse = session.apply_take(take.edit)?;
        Ok(Self {
            into: Into::Song {
                pattern: take.pattern,
                clip: take.clip,
                added_track: take.added_track.then_some(take.track),
                start,
                bars: 1,
            },
            quantize,
            held: BTreeMap::new(),
            undo: vec![inverse],
            written: 0,
            reached: start,
        })
    }

    /// The song tick a song take started at, where its pattern's tick 0 lies; none in pattern
    /// mode.
    pub fn song_start(&self) -> Option<Ticks> {
        match self.into {
            Into::Song { start, .. } => Some(start),
            Into::Pattern(_) => None,
        }
    }

    /// The pattern the take writes into.
    pub fn pattern(&self) -> PatternId {
        match self.into {
            Into::Pattern(pattern) | Into::Song { pattern, .. } => pattern,
        }
    }

    /// The song take's pattern, clip and the track it added, which the document keeps from
    /// changing under it while it runs; none in pattern mode.
    pub fn song_parts(&self) -> Option<TakeParts> {
        match self.into {
            Into::Song {
                pattern,
                clip,
                added_track,
                ..
            } => Some(TakeParts {
                pattern,
                clip,
                track: added_track,
            }),
            Into::Pattern(_) => None,
        }
    }

    /// Takes in one event the player posted: a note heard while the song timeline ran starts,
    /// and its end writes it into the pattern.
    pub fn hear(&mut self, session: &mut Session, event: &EngineEvent) -> Result<(), SessionError> {
        match *event {
            EngineEvent::LiveNoteStarted {
                channel,
                key,
                velocity,
                tick: Some(tick),
                ..
            } => {
                self.held.insert((channel, key), (tick, velocity));
                Ok(())
            }
            EngineEvent::LiveNoteEnded {
                channel,
                key,
                tick: Some(tick),
                ..
            } => match self.held.remove(&(channel, key)) {
                Some((on, velocity)) => self.write(session, channel, key, on, velocity, tick),
                None => Ok(()),
            },
            _ => Ok(()),
        }
    }

    /// The playhead is at `playhead`: a song take's pattern and clip grow to hold the bar it is
    /// in (a pattern take holds its pattern's length). Returns whether they grew.
    pub fn follow(&mut self, session: &mut Session, playhead: Ticks) -> Result<bool, SessionError> {
        let Into::Song { start, bars, .. } = self.into else {
            return Ok(false);
        };
        self.reached = self.reached.max(playhead);
        let wanted =
            takes::bars_holding(Ticks(self.reached.0 - start.0 + 1), session.project().ppq);
        self.grow_to(session, wanted)?;
        Ok(wanted > bars)
    }

    /// Stops the take: the notes still held end at `tick`, and everything written becomes one
    /// undo step. A song take then holds the bars of what was played and of every note, and
    /// one with no note is taken out again. A note that could not be written is reported after
    /// the rest are.
    pub fn finish(mut self, session: &mut Session, tick: Ticks) -> Result<Finished, SessionError> {
        let mut result = Ok(());
        for ((channel, key), (on, velocity)) in std::mem::take(&mut self.held) {
            let written = self.write(session, channel, key, on, velocity, tick);
            if result.is_ok() {
                result = written;
            }
        }
        if let Into::Song {
            pattern,
            clip,
            start,
            bars,
            ..
        } = self.into
        {
            if self.written == 0 {
                self.abandon(session)?;
                return result.map(|()| Finished::Nothing);
            }
            let ppq = session.project().ppq;
            let reached = self.reached.max(tick);
            let last = session.project().patterns.get(&pattern).map_or(0, |held| {
                held.notes
                    .iter()
                    .map(|note| note.start.0)
                    .max()
                    .unwrap_or(0)
            });
            let held = takes::bars_holding(Ticks(reached.0 - start.0), ppq)
                .max(takes::bars_holding(Ticks(last + 1), ppq));
            if held != bars {
                let edit = takes::take_length(pattern, clip, held, ppq);
                match session.apply_take(edit) {
                    Ok(inverse) => self.undo.push(inverse),
                    Err(error) if result.is_ok() => result = Err(error),
                    Err(_) => {}
                }
            }
        }
        if self.undo.is_empty() {
            return result.map(|()| Finished::Nothing);
        }
        // Undone newest first: a note that replaced an earlier note of the take brings that
        // one back before it goes too, and a song take's notes go before its pattern does.
        self.undo.reverse();
        session.record_as(Edit::Transaction(self.undo), "Record notes");
        result.map(|()| Finished::Kept)
    }

    /// Takes back everything the take applied, recording nothing: the document is as it was
    /// before the take.
    pub fn abandon(mut self, session: &mut Session) -> Result<(), SessionError> {
        if self.undo.is_empty() {
            return Ok(());
        }
        self.undo.reverse();
        session.apply_take(Edit::Transaction(self.undo)).map(drop)
    }

    /// Makes a song take's pattern and clip `bars` long when that is more than they are.
    fn grow_to(&mut self, session: &mut Session, bars: u32) -> Result<(), SessionError> {
        let Into::Song {
            pattern,
            clip,
            bars: ref mut held,
            ..
        } = self.into
        else {
            return Ok(());
        };
        if bars <= *held {
            return Ok(());
        }
        let edit = takes::take_length(pattern, clip, bars, session.project().ppq);
        let inverse = session.apply_take(edit)?;
        *held = bars;
        self.undo.push(inverse);
        Ok(())
    }

    /// Writes the note of `key` played on `channel` from tick `on` to tick `off`.
    fn write(
        &mut self,
        session: &mut Session,
        channel: ChannelId,
        key: u8,
        on: Ticks,
        velocity: f32,
        off: Ticks,
    ) -> Result<(), SessionError> {
        let notes = self.notes(session.project(), channel, key, on, velocity, off);
        let Some(last) = notes.iter().map(|note| note.start.0).max() else {
            return Ok(());
        };
        // A song take's note snapped onto its pattern's end: the pattern grows to hold it.
        let bars = takes::bars_holding(Ticks(last + 1), session.project().ppq);
        self.grow_to(session, bars)?;
        let pattern = self.pattern();
        let Some(held) = session.project().patterns.get(&pattern) else {
            return Ok(());
        };
        let remove = held
            .notes
            .iter()
            .filter(|old| {
                old.channel == channel
                    && old.key == key
                    && notes.iter().any(|note| note.start == old.start)
            })
            .cloned()
            .collect();
        let edit = Edit::ReplaceNotes {
            pattern,
            remove,
            add: notes,
        };
        let inverse = session.apply_take(edit)?;
        self.undo.push(inverse);
        self.written += 1;
        Ok(())
    }

    /// The notes of `key` played on `channel` from `on` to `off`, where the take writes them;
    /// none when the channel plays no notes, the pattern is gone, or (song mode) it was heard
    /// before the take's start. In pattern mode a note running past the pattern's end is split
    /// there ([`split_at_loop`]); a song take's pattern grows to hold it whole.
    fn notes(
        &self,
        project: &Project,
        channel: ChannelId,
        key: u8,
        on: Ticks,
        velocity: f32,
        off: Ticks,
    ) -> Vec<Note> {
        let Some(found) = project.channels.get(&channel) else {
            return Vec::new();
        };
        let Some(pattern) = project.patterns.get(&self.pattern()) else {
            return Vec::new();
        };
        if !found.takes_midi_input() {
            return Vec::new();
        }
        let snap = |tick: i64| match self.quantize {
            Some(unit) => snap_nearest(Ticks(tick), unit).0,
            None => tick,
        };
        // A jump back (a seek) between the two ends would make it negative: it is then a hit.
        let played = (off.0 - on.0).max(0);
        let note = |start: i64, length: i64| Note {
            channel,
            start: Ticks(start),
            length: Ticks(length),
            velocity,
            key,
        };
        match self.into {
            Into::Pattern(_) => {
                let length = time::bars_to_ticks(pattern.length_bars, project.ppq).0;
                if length <= 0 {
                    return Vec::new();
                }
                let start = snap(on.0.rem_euclid(length)).rem_euclid(length);
                split_at_loop(start, played, length)
                    .into_iter()
                    .map(|(start, length)| note(start, length))
                    .collect()
            }
            Into::Song { start, .. } => match on.0.checked_sub(start.0).filter(|from| *from >= 0) {
                Some(from) => vec![note(snap(from), played)],
                None => Vec::new(),
            },
        }
    }
}

/// A pattern take's note from `start` (within a pattern `pattern` ticks long), `length` as
/// played, as it lands in the looping pattern (found in use: a key held past the loop's end
/// drew wrapped but was written as one note running far past the end): cut at the pattern's
/// end, and what is left from tick 0, no longer than the note's own start, so a key held a
/// whole pattern or more covers the pattern once and no note is longer than the pattern. A
/// note within the pattern, a step hit included, stays one.
fn split_at_loop(start: i64, length: i64, pattern: i64) -> Vec<(i64, i64)> {
    let first = length.min(pattern - start);
    let rest = (length - first).min(start);
    let mut notes = vec![(start, first)];
    if rest > 0 {
        notes.push((0, rest));
    }
    notes
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use dawsome_engine::plan::PlayMode;
    use dawsome_engine::project::ChannelKind;

    use super::*;
    use crate::project_file;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    /// p9_record (one 1-bar pattern of 384 ticks, channel 1 "keys" a sampler) in pattern mode.
    fn p9_record() -> Session {
        let dir = PathBuf::from(FIXTURES);
        let opened = project_file::open(&dir.join("p9_record.json")).expect("opens");
        Session::open_in(opened.project, dir, PlayMode::Pattern(PatternId(1)))
            .expect("compiles")
            .0
    }

    fn started(key: u8, velocity: f32, tick: Option<i64>) -> EngineEvent {
        EngineEvent::LiveNoteStarted {
            channel: ChannelId(1),
            key,
            velocity,
            tick: tick.map(Ticks),
            frame: 0,
        }
    }

    fn ended(key: u8, tick: Option<i64>) -> EngineEvent {
        EngineEvent::LiveNoteEnded {
            channel: ChannelId(1),
            key,
            tick: tick.map(Ticks),
            frame: 0,
        }
    }

    fn notes(session: &Session) -> Vec<(i64, i64, u8, f32)> {
        session.project().patterns[&PatternId(1)]
            .notes
            .iter()
            .map(|note| (note.start.0, note.length.0, note.key, note.velocity))
            .collect()
    }

    // By hand, a take played on at 193 off at 267 (60), 334..378 (64), 381..401 (67, across the
    // loop at 384). Quantized to steps of 24: 193 → 192, 334 → 336, 381 → 384 → 0 (the end wraps);
    // lengths 74, 44, 20 as played. Unquantized, 67 runs past the end and is cut there
    // (as found in use): 381 for 3, then 0 for 17.
    #[test]
    fn a_take_writes_each_note_as_it_ends_and_undoes_in_one_step() {
        let quantized = vec![
            (0, 20, 67, 0.5),
            (192, 74, 60, 0.78125),
            (336, 44, 64, 0.992_187_5),
        ];
        let as_played = vec![
            (0, 17, 67, 0.5),
            (193, 74, 60, 0.78125),
            (334, 44, 64, 0.992_187_5),
            (381, 3, 67, 0.5),
        ];
        for (quantize, want) in [(Some(Ticks(24)), quantized), (None, as_played)] {
            let mut session = p9_record();
            let before = session.project().clone();
            let mut take = NoteTake::new(PatternId(1), quantize);
            let events = [
                started(60, 0.78125, Some(193)),
                ended(60, Some(267)),
                started(64, 0.992_187_5, Some(334)),
                ended(64, Some(378)),
                started(67, 0.5, Some(381)),
                ended(67, Some(401)),
            ];
            for (index, event) in events.iter().enumerate() {
                take.hear(&mut session, event).expect("heard");
                if index == 1 {
                    assert_eq!(notes(&session).len(), 1, "written when it ends");
                }
            }
            take.finish(&mut session, Ticks(768)).expect("finished");
            assert_eq!(notes(&session), want, "quantize {quantize:?}");
            assert_eq!(session.history().undo_len(), 1, "one undo step");
            assert!(session.undo().expect("undoes"));
            assert_eq!(session.project(), &before);
        }
    }

    // A note heard in the count-in (no tick) is not recorded, nor its end; a note held at Stop
    // ends at the stop tick; a second note on a tick and key replaces the first, and the undo
    // brings back what the pattern held before the take.
    #[test]
    fn count_in_notes_are_skipped_held_notes_end_at_stop_and_replacements_undo() {
        let mut session = p9_record();
        let before = session.project().clone();
        let mut take = NoteTake::new(PatternId(1), Some(Ticks(24)));
        for event in [
            started(60, 0.5, None),
            ended(60, Some(10)),
            started(62, 0.5, Some(47)),
            ended(62, Some(60)),
            started(62, 0.25, Some(50)),
            ended(62, Some(52)),
            started(64, 1.0, Some(700)),
        ] {
            take.hear(&mut session, &event).expect("heard");
        }
        take.finish(&mut session, Ticks(730)).expect("finished");
        // 47 and 50 both quantize to 48; 700 mod 384 = 316 → 312, held 30 ticks to Stop.
        assert_eq!(notes(&session), [(48, 2, 62, 0.25), (312, 30, 64, 1.0)]);
        assert!(session.undo().expect("undoes"));
        assert_eq!(session.project(), &before);
    }

    #[test]
    fn a_channel_that_plays_no_notes_records_nothing() {
        let dir = PathBuf::from(FIXTURES);
        let mut project = project_file::open(&dir.join("p9_record.json"))
            .expect("opens")
            .project;
        project
            .channels
            .get_mut(&ChannelId(1))
            .expect("channel 1")
            .kind = ChannelKind::Audio;
        let mut session = Session::open_in(project, dir, PlayMode::Pattern(PatternId(1)))
            .expect("compiles")
            .0;
        let mut take = NoteTake::new(PatternId(1), None);
        take.hear(&mut session, &started(60, 0.5, Some(0)))
            .expect("heard");
        take.hear(&mut session, &ended(60, Some(24)))
            .expect("heard");
        take.finish(&mut session, Ticks(48)).expect("finished");
        assert!(notes(&session).is_empty());
        assert_eq!(session.history().undo_len(), 0);
    }

    // A song take from tick 0 of p9_record (8 empty tracks): its pattern "keys take 1" and the
    // clip on track 1, a bar long; the playhead at 300 keeps one bar. A note played 380..390
    // snaps to 384, the pattern's end, so the take grows to two bars to hold it. Stopped at 400
    // the take holds two bars (ceil(400 / 384)); one undo step takes pattern, clip and note out,
    // byte for byte.
    #[test]
    fn a_song_take_grows_to_hold_a_note_snapped_onto_its_end() {
        let mut session = p9_record();
        let before = session.project().clone();
        let mut take = NoteTake::song(
            &mut session,
            "keys take 1".to_owned(),
            Ticks(0),
            None,
            Some(Ticks(24)),
        )
        .expect("starts");
        assert_eq!(take.pattern(), PatternId(2));
        assert!(!take.follow(&mut session, Ticks(300)).expect("follows"));
        take.hear(&mut session, &started(64, 0.5, Some(380)))
            .expect("heard");
        take.hear(&mut session, &ended(64, Some(390)))
            .expect("heard");
        let project = session.project();
        assert_eq!(project.patterns[&PatternId(2)].length_bars, 2);
        assert_eq!(
            project.patterns[&PatternId(2)]
                .notes
                .iter()
                .map(|note| (note.start.0, note.length.0, note.key))
                .collect::<Vec<_>>(),
            [(384, 10, 64)]
        );
        assert_eq!(
            take.finish(&mut session, Ticks(400)).expect("finished"),
            Finished::Kept
        );
        let clip = session
            .project()
            .playlist
            .clips
            .values()
            .next()
            .expect("the clip")
            .clone();
        assert_eq!(
            (clip.track.0, clip.start, clip.length),
            (1, Ticks(0), Ticks(768))
        );
        assert_eq!(session.history().undo_len(), 1);
        assert!(session.undo().expect("undoes"));
        assert_eq!(session.project(), &before);
    }

    // By hand, a 384-tick pattern: a note crossing the end is cut there and the
    // rest goes from tick 0 (300 + 184: 300..384 and 0..100); held a pass and a half from 300 it
    // covers the pattern once (300..384, 0..300); from 0 held longer than the pattern, one note of
    // the pattern's length; a step hit and a note within the pattern stay one; one ending on the
    // end exactly is not cut.
    #[test]
    fn a_note_is_split_at_the_loops_end() {
        for (start, length, want) in [
            (300, 184, vec![(300, 84), (0, 100)]),
            (300, 576, vec![(300, 84), (0, 300)]),
            (0, 500, vec![(0, 384)]),
            (383, 0, vec![(383, 0)]),
            (360, 50, vec![(360, 24), (0, 26)]),
            (100, 50, vec![(100, 50)]),
            (334, 50, vec![(334, 50)]),
        ] {
            assert_eq!(
                split_at_loop(start, length, 384),
                want,
                "{start} + {length}"
            );
        }
    }

    // Through a take, quantized to the step: played 350..400 snaps to 360 and is cut at the end
    // (360 for 24, 0 for 26); a step hit at 383 snaps onto the end and wraps to 0, length 0;
    // unquantized the same hit stays at 383. One undo step takes them all out.
    #[test]
    fn a_quantized_note_and_a_step_hit_across_the_end() {
        let mut session = p9_record();
        let before = session.project().clone();
        let mut take = NoteTake::new(PatternId(1), Some(Ticks(24)));
        for event in [
            started(60, 0.5, Some(350)),
            ended(60, Some(400)),
            started(62, 0.5, Some(383)),
            ended(62, Some(383)),
        ] {
            take.hear(&mut session, &event).expect("heard");
        }
        take.finish(&mut session, Ticks(420)).expect("finished");
        assert_eq!(
            notes(&session),
            [(0, 26, 60, 0.5), (0, 0, 62, 0.5), (360, 24, 60, 0.5)]
        );
        assert!(session.undo().expect("undoes"));
        assert_eq!(session.project(), &before);
        let mut take = NoteTake::new(PatternId(1), None);
        take.hear(&mut session, &started(62, 0.5, Some(383)))
            .expect("heard");
        take.hear(&mut session, &ended(62, Some(383)))
            .expect("heard");
        take.finish(&mut session, Ticks(400)).expect("finished");
        assert_eq!(notes(&session), [(383, 0, 62, 0.5)]);
    }
}
