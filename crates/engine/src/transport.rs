//! The transport: whether playback runs, where the playhead is, and the tempo that maps ticks
//! to samples. The audio thread owns it; the UI changes it only through [`Control`] messages.
//! Where song playback wraps comes from the plan; the transport carries the wrap out.

use crate::project::{ChannelId, EffectId, InsertId, ProcessorOwner, TEMPO_RANGE};
use crate::realtime::LiveControl;
use crate::time::{self, MilliBpm, Ticks};

/// A transport command or a live parameter value from the UI thread. `Copy` and heap-free, so it
/// crosses the control ring without allocating. Commands take effect at the start of the next
/// `Player::process` call, in the order they were sent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Control {
    /// Start playing from the last seek target (tick 0 until a seek). Ignored while playing.
    Play,
    /// Stop, end every live note (every pedal lifts, as [`Control::LiveNotesOff`] does),
    /// silence every voice, and put the playhead back where play starts: the last seek target,
    /// tick 0 until a seek. Play and Seek keep a key still held sounding; Stop does not, so a
    /// key whose release never came stops here.
    Stop,
    /// Make this tick where play starts, and move the playhead there now (jumping, if playing).
    /// Negative ticks count as 0.
    Seek(Ticks),
    /// Change the tempo. While playing, the new tempo starts at the first tick at or after the
    /// start of the call that receives it; earlier ticks keep the old rate. Tempos outside
    /// 10..=522 BPM are ignored, and so is the tempo already in force.
    SetTempo(MilliBpm),
    /// Stop, and suspend every processor: the last call each gets on the audio thread before
    /// the stream stops (the app sends it as it closes). A later Play starts them again.
    Suspend,
    /// A value for a control being dragged, heard at once with no new plan. It holds until the
    /// next plan arrives, which carries the value the drag committed. A target the plan does
    /// not have, and a value that is not a finite number, are ignored; others are clamped to
    /// the control's range. Faders, pans and sends ramp to it (see [`crate::plan`]). While an
    /// automation governs the target, its value rules and this is ignored.
    Param(ParamTarget, f32),
    /// Play with recording: first, when `count_in`, one bar of count-in clicks at the tempo in
    /// force while the playhead waits on the play-from tick; then play from there with the
    /// loop and the song-end wrap off, and, when `tap_master`, every master frame into the
    /// record tap. Ignored while playing; Stop ends it.
    Record { count_in: bool, tap_master: bool },
    /// Ends a recording while the song plays on: the record tap is fed no more, and the loop
    /// and the song-end wrap are back from the next frame (a playhead already past the song's
    /// end plays on, as one started there does). A take kept before Stop sends it. During the
    /// count-in it ends the count-in too and nothing plays; ignored when nothing records.
    EndRecording,
    /// Stops the file being previewed.
    StopPreview,
    /// Plays `key` (0 to 127) live on the plan's live channel, the selected one, from the first
    /// frame of the call, stopped or playing, at `velocity` (0 to 1). A key already held is
    /// ended first.
    LiveNoteOn { key: u8, velocity: f32 },
    /// As [`Control::LiveNoteOn`], on `channel` whatever is selected (a channel the plan lacks:
    /// the live one): the instrument window's notes, which play its channel.
    ChannelNoteOn {
        channel: ChannelId,
        key: u8,
        velocity: f32,
    },
    /// Ends the live note of `key` on the channel it started on.
    LiveNoteOff { key: u8 },
    /// Ends every live note and silences the live voices; every pedal lifts.
    LiveNotesOff,
    /// A performance control from the typing keyboard (its sustain key) on `channel`, or on the
    /// live channel when that is `None` or a channel the plan lacks: as a MIDI device's.
    LiveControl {
        channel: Option<ChannelId>,
        control: LiveControl,
    },
    /// Switches the metronome on or off: while the transport plays, a click on every beat,
    /// accented on a bar's first, after the master (so never recorded).
    Metronome(bool),
}

/// What a [`Control::Param`] sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamTarget {
    /// A channel's volume, linear gain 0 to 4.
    ChannelVolume(ChannelId),
    /// A channel's pan, -1 to 1.
    ChannelPan(ChannelId),
    /// An insert's fader position, 0 to 1.
    InsertFader(InsertId),
    /// An insert's pan, -1 to 1.
    InsertPan(InsertId),
    /// The amount of the send from one insert to another, 0 to 1.
    Send { from: InsertId, to: InsertId },
    /// Parameter `param` (its index in the processor's parameter list) of an effect.
    Effect { effect: EffectId, param: u16 },
    /// Parameter `id` (the plugin's own number for it) of the plugin `owner` plays. The plugin
    /// holds the value: no plan carries it, and the next plan does not reset it.
    Plugin { owner: ProcessorOwner, id: u32 },
}

/// The tempo in force from `start_tick` on: tick `t` falls at
/// `start_sample + tick_to_sample(t - start_tick)`. Ticks count from where play started.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TempoSegment {
    start_tick: Ticks,
    start_sample: u64,
    tempo: MilliBpm,
    sample_rate: u32,
    ppq: u16,
}

impl TempoSegment {
    /// This segment's tempo from tick 0 at sample 0: a timeline as it is when play starts.
    pub(crate) fn restarted(&self) -> Self {
        Self {
            start_tick: Ticks(0),
            start_sample: 0,
            ..*self
        }
    }

    /// The absolute sample where `tick` falls, or `None` for a tick before this segment or a
    /// sample beyond `u64::MAX`.
    pub(crate) fn tick_to_sample(&self, tick: Ticks) -> Option<u64> {
        let since = tick.0.checked_sub(self.start_tick.0).filter(|&t| t >= 0)?;
        let offset = time::tick_to_sample(Ticks(since), self.sample_rate, self.tempo, self.ppq)?;
        self.start_sample.checked_add(offset)
    }

    /// The sample where `tick` falls, also before this segment, as a signed number: the
    /// segment's rate carried back past its start. Equal to [`tick_to_sample`] from the start on.
    ///
    /// Why carry it back: a clip that began before a live tempo change still has a first sample
    /// its frames count from. The answer is the same function of the tick for every block, so
    /// renders stay split-invariant.
    ///
    /// [`tick_to_sample`]: Self::tick_to_sample
    pub(crate) fn sample_at(&self, tick: Ticks) -> i128 {
        // A tick below 2^63 times 6·10^4 times a rate below 2^32 stays below 2^127.
        let since = i128::from(tick.0) - i128::from(self.start_tick.0);
        let numerator = since * 60 * 1000 * i128::from(self.sample_rate);
        let denominator = (i128::from(self.tempo.0) * i128::from(self.ppq)).max(1);
        i128::from(self.start_sample) + numerator.div_euclid(denominator)
    }

    pub(crate) fn tempo(&self) -> MilliBpm {
        self.tempo
    }

    /// The tick sample `sample` is in: the last tick that starts at or before it (the
    /// segment's first tick for a sample before the segment).
    pub(crate) fn tick_at(&self, sample: u64) -> Ticks {
        // One before the first tick at or after the next sample. That first tick is above the
        // segment start, because a positive tempo needs a positive tick count to reach a later
        // sample, so the subtraction stays within the segment.
        self.first_tick_at_or_after(sample.saturating_add(1))
            .map_or(self.start_tick, |tick| Ticks(tick.0 - 1))
    }

    /// The first tick that falls at or after absolute sample `sample`, or `None` for a sample
    /// before this segment.
    pub(crate) fn first_tick_at_or_after(&self, sample: u64) -> Option<Ticks> {
        let since = sample.checked_sub(self.start_sample)?;
        let ticks = time::first_tick_at_or_after(since, self.sample_rate, self.tempo, self.ppq)?;
        self.start_tick.0.checked_add(ticks.0).map(Ticks)
    }

    /// The first tick of this segment that falls at or after `sample`: the segment's first tick
    /// for a sample before it.
    ///
    /// Why a sample before the segment can ask: after a tempo change the playhead stands before
    /// the tick the new tempo starts on ([`Self::retimed`]), and that tick is the first one still
    /// to play from there.
    pub(crate) fn first_tick_from(&self, sample: u64) -> Option<Ticks> {
        if sample <= self.start_sample {
            return Some(self.start_tick);
        }
        self.first_tick_at_or_after(sample)
    }

    /// A segment at `tempo` that starts at the first tick at or after `sample`.
    ///
    /// Why a tick and not `sample` itself: notes sit on ticks, so the new rate must begin on
    /// one. The tick before it keeps this segment's rate, so no tick moves backwards or fires
    /// twice.
    fn retimed(&self, sample: u64, tempo: MilliBpm) -> Option<Self> {
        let start_tick = self.first_tick_at_or_after(sample)?;
        Some(Self {
            start_tick,
            start_sample: self.tick_to_sample(start_tick)?,
            tempo,
            ..*self
        })
    }
}

/// Where song playback jumps: the frame that would play song tick `at` plays tick `to`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Wrap {
    pub(crate) at: Ticks,
    pub(crate) to: Ticks,
}

/// Play state, playhead and tempo. Positions are absolute samples on a timeline where play
/// started at tick 0, sample 0, so every note's sample comes from its absolute tick.
#[derive(Debug)]
pub(crate) struct Transport {
    playing: bool,
    /// Absolute sample of the next frame to render on the song timeline. It jumps back at a
    /// wrap.
    position: u64,
    /// Frames rendered since the last Play, Seek or Stop. It never jumps, so voices are timed
    /// by it: a sound started before a wrap rings on after it.
    clock: u64,
    /// Where `Play` starts.
    play_from: Ticks,
    /// The tick the playhead last jumped to; while stopped, the playhead stays there.
    jumped_to: Ticks,
    timeline: TempoSegment,
}

impl Transport {
    /// Stopped at tick 0, at `tempo`.
    pub(crate) fn new(tempo: MilliBpm, sample_rate: u32, ppq: u16) -> Self {
        Self {
            playing: false,
            position: 0,
            clock: 0,
            play_from: Ticks(0),
            jumped_to: Ticks(0),
            timeline: TempoSegment {
                start_tick: Ticks(0),
                start_sample: 0,
                tempo,
                sample_rate,
                ppq,
            },
        }
    }

    /// Applies one control. Returns `true` when the playhead jumped, so voices that are still
    /// sounding belong to the old position and must be silenced.
    pub(crate) fn apply(&mut self, control: Control) -> bool {
        match control {
            Control::Play => {
                if self.playing {
                    return false;
                }
                self.playing = true;
                self.jump_to(self.play_from);
                true
            }
            Control::Stop | Control::Suspend => {
                self.playing = false;
                self.jump_to(self.play_from);
                true
            }
            Control::Seek(tick) => {
                let tick = Ticks(tick.0.max(0));
                // A tick past the end of a u64 sample timeline has nowhere to play from.
                if self.timeline.restarted().tick_to_sample(tick).is_none() {
                    return false;
                }
                self.play_from = tick;
                self.jump_to(tick);
                true
            }
            Control::SetTempo(tempo) => {
                self.set_tempo(tempo);
                false
            }
            // Parameters belong to the plan; recording, previews and live notes to the player.
            Control::Param(..)
            | Control::Record { .. }
            | Control::EndRecording
            | Control::StopPreview
            | Control::LiveNoteOn { .. }
            | Control::ChannelNoteOn { .. }
            | Control::LiveNoteOff { .. }
            | Control::LiveNotesOff
            | Control::LiveControl { .. }
            | Control::Metronome(_) => false,
        }
    }

    /// Puts the playhead on `tick` of a timeline restarted at the current tempo.
    fn jump_to(&mut self, tick: Ticks) {
        self.timeline = self.timeline.restarted();
        self.jumped_to = tick;
        self.clock = 0;
        // Every tick that gets here has a sample: tick 0, or a seek target checked in `apply`.
        self.position = self.timeline.tick_to_sample(tick).unwrap_or(0);
    }

    /// Takes `wrap` when the playhead stands exactly on its `at` tick: the playhead moves to
    /// its `to` tick on a timeline restarted at the tempo in force, as a seek would, but the
    /// voices and the clock carry on. A playhead that never reaches `at` (one that started past
    /// it) plays on, and a wrap whose pass would be empty is not taken. Returns whether it was
    /// taken.
    pub(crate) fn wrap_if_due(&mut self, wrap: Option<Wrap>) -> bool {
        let Some(wrap) = wrap else {
            return false;
        };
        if self.timeline.tick_to_sample(wrap.at) != Some(self.position) {
            return false;
        }
        let restarted = self.timeline.restarted();
        if let (Some(to), Some(at)) = (
            restarted.tick_to_sample(wrap.to),
            restarted.tick_to_sample(wrap.at),
        ) {
            if to < at {
                self.timeline = restarted;
                self.position = to;
                return true;
            }
        }
        false
    }

    /// Frames from the playhead to `wrap`'s `at` tick, when that lies ahead.
    pub(crate) fn frames_to_wrap(&self, wrap: Option<Wrap>) -> Option<u64> {
        let at = self.timeline.tick_to_sample(wrap?.at)?;
        at.checked_sub(self.position).filter(|&frames| frames > 0)
    }

    fn set_tempo(&mut self, tempo: MilliBpm) {
        // A validated edit never sends one; ignoring it keeps the timeline sane (tempo 0 would
        // put every later tick at no sample at all).
        if !TEMPO_RANGE.contains(&tempo.0) {
            return;
        }
        // Why an unchanged tempo is ignored: a retime restarts the timeline on the next tick,
        // and flooring that tick's sample can pull every later tick one sample earlier, so
        // resending the tempo in force would still move the beat.
        if tempo == self.timeline.tempo {
            return;
        }
        if self.playing {
            if let Some(timeline) = self.timeline.retimed(self.position, tempo) {
                self.timeline = timeline;
            }
        } else {
            self.timeline.tempo = tempo;
            self.jump_to(self.jumped_to);
        }
    }

    /// Moves the playhead and the clock past `frames` rendered frames, if playing.
    pub(crate) fn advance(&mut self, frames: usize) {
        if self.playing {
            // `usize` is at most 64 bits, so the cast is exact.
            self.position = self.position.saturating_add(frames as u64);
            self.clock = self.clock.saturating_add(frames as u64);
        }
    }

    pub(crate) fn playing(&self) -> bool {
        self.playing
    }

    pub(crate) fn position(&self) -> u64 {
        self.position
    }

    pub(crate) fn clock(&self) -> u64 {
        self.clock
    }

    pub(crate) fn timeline(&self) -> &TempoSegment {
        &self.timeline
    }

    /// The tick the playhead is in: the last tick that starts at or before the position.
    pub(crate) fn playhead_tick(&self) -> Ticks {
        self.timeline.tick_at(self.position)
    }
}
