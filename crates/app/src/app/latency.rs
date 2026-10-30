//! Settings → Input latency: how late a take of the line in records what the output plays, kept
//! per (output device, input device) in the settings ([`InputLatency`]), and the measurement that
//! finds it.
//!
//! A measurement is a short take through the existing record path, nothing on the audio thread
//! of its own: the plans play nothing of the document ([`Session::set_measuring`]), the player
//! records the master through its record tap and the line in is recorded from the device a take
//! of it uses now (the selected input channel's, else the system's default input), while the
//! click burst ([`latency::burst`]) plays as a preview. Both recordings stop at
//! [`latency::recording_frames`]; a thread then finds the burst in the master's recording and in
//! the input's ([`latency::measure_files`]), and the difference is the latency. The user loops a
//! cable from the output into the input (or holds the microphone to the speaker).
//!
//! [`Session::set_measuring`]: crate::session::Session::set_measuring

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::project::ChannelId;
use dawsome_engine::source::MemorySource;
use dawsome_engine::transport::Control;
use dawsome_media::latency::{self, MeasureError, NotFound};
use dawsome_media::record::{Recorder, TapFeed};

use super::App;
use crate::play_to::RecordSource;
use crate::settings::InputLatency;

/// How long a measurement may run past its recordings' length before it stops anyway: an input
/// that delivers nothing never fills its recording.
const SPARE: Duration = Duration::from_secs(2);

/// The figure set by hand stays within the latencies a measurement searches: 50 ms early to
/// 500 ms late.
const HAND_MS: std::ops::RangeInclusive<i64> = -50..=500;

/// What Settings → Input latency's buttons ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LatencyMessage {
    /// Measure the latency of the line in a take uses now, with the output Settings chose.
    Measure,
    /// The figure set by hand, a millisecond less or more.
    Down,
    Up,
    /// Forget the figure: takes land as recorded (late).
    Clear,
}

/// A measurement in progress.
pub(super) enum Measuring {
    Recording(Recording),
    /// The recordings are made; a thread looks for the burst in them.
    Finding {
        result: mpsc::Receiver<Result<(i64, u32), MeasureError>>,
        output: String,
        input: String,
    },
}

/// The measurement's two recordings while they run.
pub(super) struct Recording {
    input: Recorder,
    master: Recorder,
    /// The input channel whose record ring the input's recording holds; none for the default
    /// input's.
    channel: Option<ChannelId>,
    output_name: String,
    input_name: String,
    deadline: Instant,
    dir: PathBuf,
}

/// A folder of its own for each measurement's two files (a thread may still read the last
/// one's), in the system's temporary folder; the thread that reads them deletes it.
fn measurement_dir() -> PathBuf {
    static MADE: AtomicU32 = AtomicU32::new(0);
    let number = MADE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("dawsome-latency-{}-{number}", std::process::id()))
}

impl App {
    pub(super) fn latency_message(&mut self, message: LatencyMessage) {
        match message {
            LatencyMessage::Measure => self.measure_latency(),
            LatencyMessage::Down => self.step_latency(-1),
            LatencyMessage::Up => self.step_latency(1),
            LatencyMessage::Clear => {
                let (output, input) = self.line_in_devices();
                self.settings.forget_input_latency(&output, &input);
                self.save_settings();
            }
        }
    }

    /// The devices a take of the line in uses now, by name ("" for a system default): the output
    /// Settings chose, and the selected input channel's device, else the default input.
    pub(super) fn line_in_devices(&self) -> (String, String) {
        let output = self.settings.audio_output().unwrap_or_default().to_owned();
        let input = self
            .recording_input_channel()
            .and_then(|id| self.session.project().channels.get(&id))
            .and_then(|channel| channel.input.as_ref())
            .map_or_else(String::new, |source| source.device.clone());
        (output, input)
    }

    /// The latency kept for the devices a take of the line in uses now.
    pub(super) fn line_in_latency(&self) -> Option<&InputLatency> {
        let (output, input) = self.line_in_devices();
        self.settings.input_latency(&output, &input)
    }

    /// The figure set by hand `by` milliseconds from the one kept (0 without one), within
    /// [`HAND_MS`], at the project's rate.
    fn step_latency(&mut self, by: i64) {
        let (output, input) = self.line_in_devices();
        let rate = self.session.project().sample_rate;
        let ms = self
            .settings
            .input_latency(&output, &input)
            .map_or(0, |kept| kept.frames_at(1000));
        let ms = (ms + by).clamp(*HAND_MS.start(), *HAND_MS.end());
        self.settings.set_input_latency(InputLatency {
            output,
            input,
            input_latency_frames: ms * i64::from(rate) / 1000,
            rate,
            measured: false,
        });
        self.save_settings();
    }

    /// The output device the measurement running keeps its figure under.
    #[cfg(test)]
    pub(super) fn latency_output(&self) -> Option<&str> {
        match &self.latency {
            Some(Measuring::Recording(recording)) => Some(&recording.output_name),
            Some(Measuring::Finding { output, .. }) => Some(output),
            None => None,
        }
    }

    /// The folder the measurement recording writes its two files into.
    #[cfg(test)]
    pub(super) fn latency_folder(&self) -> Option<PathBuf> {
        match &self.latency {
            Some(Measuring::Recording(recording)) => Some(recording.dir.clone()),
            _ => None,
        }
    }

    /// Whether a measurement runs (its recordings, or the search after them).
    pub(super) fn measuring_latency(&self) -> bool {
        self.latency.is_some()
    }

    /// Starts a measurement: the line in and the master recorded, the document silent, the burst
    /// played. Refused, with the reason shown, while the song plays or a take runs, or without a
    /// line in to record. Without an output nothing is heard, which the search then says.
    fn measure_latency(&mut self) {
        if self.latency.is_some() {
            return;
        }
        if self.recording.recording() || self.transport_on || self.session.playing() {
            self.notice = Some("Stop playback first: measuring plays clicks of its own".into());
            return;
        }
        // An output chosen in Settings plays from the next Play, and this is one: the clicks
        // play through the device the figure is kept under (found 2026-10-26). Before the
        // feeds are taken: an output opened anew opens the input channels' devices anew.
        self.apply_output_choice();
        let (output_name, input_name) = self.line_in_devices();
        let channel = self.recording_input_channel();
        let Some(feed) = self.take_line_in(channel) else {
            return;
        };
        let Some(tap) = self.session.take_record_tap() else {
            self.give_back(RecordSource::Input, channel, feed);
            self.notice = Some("Cannot measure while recording.".into());
            return;
        };
        let rate = self.session.project().sample_rate;
        let frames = latency::recording_frames(rate);
        let cap = Some(frames);
        let dir = measurement_dir();
        let input = match Recorder::start_capped(dir.join("input.wav"), feed, rate, cap) {
            Ok(input) => input,
            Err((error, feed)) => {
                if let Some(feed) = feed {
                    self.give_back(RecordSource::Input, channel, feed);
                }
                self.session.return_record_tap(tap);
                self.notice = Some(format!("Cannot measure: {error}"));
                return;
            }
        };
        let tapped = Box::new(TapFeed { tap, rate });
        let master = match Recorder::start_capped(dir.join("master.wav"), tapped, rate, cap) {
            Ok(master) => master,
            Err((error, feed)) => {
                if let Ok(recorded) = input.finish() {
                    self.give_back(RecordSource::Input, channel, recorded.feed);
                }
                if let Some(feed) = feed {
                    self.give_back(RecordSource::Master, None, feed);
                }
                let _ = std::fs::remove_dir_all(&dir);
                self.notice = Some(format!("Cannot measure: {error}"));
                return;
            }
        };
        // In this order: the silent plan reaches the player no later than Record does, and the
        // burst starts in the buffer the recordings start in or the next.
        let silent = self.session.set_measuring(true);
        self.report(silent);
        if self.metronome {
            let result = self.session.send(Control::Metronome(false));
            self.report(result);
        }
        let result = self.session.send(Control::Record {
            count_in: false,
            tap_master: true,
        });
        self.report(result);
        let burst = SampleBuffer::mono(latency::burst(rate));
        self.session
            .play_preview(Box::new(MemorySource::new(burst)));
        let length = Duration::from_secs_f64(frames as f64 / f64::from(rate.max(1)));
        self.latency = Some(Measuring::Recording(Recording {
            input,
            master,
            channel,
            output_name,
            input_name,
            deadline: Instant::now() + length + SPARE,
            dir,
        }));
        self.notice = Some("Measuring the input latency: clicks play through the output".into());
    }

    /// On every read of the playhead: a measurement whose recordings are full (or failed, or
    /// late) stops and its search starts; a search done keeps its figure or says why not.
    pub(super) fn poll_latency(&mut self) {
        let recorded = match &self.latency {
            Some(Measuring::Recording(recording)) => {
                let (input, master) = (&recording.input, &recording.master);
                (input.full() && master.full())
                    || input.failed()
                    || master.failed()
                    || Instant::now() >= recording.deadline
            }
            Some(Measuring::Finding { result, .. }) => match result.try_recv() {
                Ok(found) => {
                    self.latency_found(found);
                    false
                }
                Err(mpsc::TryRecvError::Empty) => false,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.latency = None;
                    self.notice =
                        Some("Measuring failed: the measurement stopped. Try again.".into());
                    false
                }
            },
            None => false,
        };
        if recorded {
            self.end_latency_recording(true);
        }
    }

    /// Stops the measurement's recordings: the song stops, the document plays again, the
    /// metronome comes back as the toolbar shows it now (switched while the clicks played, it
    /// sent the player its new state already; found 2026-10-26), the feeds go back where
    /// they came from; then, when `search`, a
    /// thread looks for the burst, else the files go.
    fn end_latency_recording(&mut self, search: bool) {
        let Some(Measuring::Recording(recording)) = self.latency.take() else {
            return;
        };
        let result = self.session.send(Control::Stop);
        self.report(result);
        self.forget_live_notes();
        let result = self.session.set_measuring(false);
        self.report(result);
        if self.metronome {
            let result = self.session.send(Control::Metronome(true));
            self.report(result);
        }
        let mut failure = None;
        match recording.input.finish() {
            Ok(recorded) => self.give_back(RecordSource::Input, recording.channel, recorded.feed),
            Err((error, feed)) => {
                self.give_back(RecordSource::Input, recording.channel, feed);
                failure = Some(error);
            }
        }
        match recording.master.finish() {
            Ok(recorded) => self.give_back(RecordSource::Master, None, recorded.feed),
            Err((error, feed)) => {
                self.give_back(RecordSource::Master, None, feed);
                failure = Some(error);
            }
        }
        self.update_input();
        let dir = recording.dir;
        if let Some(error) = failure {
            let _ = std::fs::remove_dir_all(&dir);
            self.notice = Some(format!("Measuring failed: {error}"));
            return;
        }
        if !search {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let (sender, result) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("dawsome-latency".into())
            .spawn(move || {
                let found = latency::measure_files(&dir.join("master.wav"), &dir.join("input.wav"));
                let _ = std::fs::remove_dir_all(&dir);
                let _ = sender.send(found);
            });
        match spawned {
            Ok(_) => {
                self.latency = Some(Measuring::Finding {
                    result,
                    output: recording.output_name,
                    input: recording.input_name,
                });
            }
            Err(error) => self.notice = Some(format!("Measuring failed: {error}")),
        }
    }

    /// Ends a measurement before a new document replaces this one, or at Stop: nothing is kept.
    pub(super) fn cancel_latency(&mut self) {
        self.end_latency_recording(false);
        self.latency = None;
    }

    /// The search's answer: the figure kept for the measurement's devices and saved, or why
    /// there is none (the figure kept before stays).
    fn latency_found(&mut self, found: Result<(i64, u32), MeasureError>) {
        let Some(Measuring::Finding { output, input, .. }) = self.latency.take() else {
            return;
        };
        let words = match found {
            Ok((frames, rate)) => {
                let kept = InputLatency {
                    output,
                    input,
                    input_latency_frames: frames,
                    rate,
                    measured: true,
                };
                let words = format!("Input latency: {}", kept.words());
                self.settings.set_input_latency(kept);
                self.save_settings();
                words
            }
            Err(MeasureError::NotFound(NotFound::NotHeard)) => match &self.audio_problem {
                Some(problem) => format!("No click heard: {problem}"),
                None => "No click heard: loop a cable from the output into the input (or hold \
                         the microphone to the speaker) and Measure again"
                    .into(),
            },
            Err(MeasureError::NotFound(NotFound::NotPlayed)) => {
                "No click played: is the master muted?".into()
            }
            Err(error) => format!("Measuring failed: {error}"),
        };
        self.notice = Some(words);
    }
}
