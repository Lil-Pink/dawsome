//! The input channels in the window: their devices kept open as the document names them
//! ([`crate::live_input`]), their feeds handed to the player, their meters, the input channel
//! Record → Input records from (the selected one, when it is open), the Library's "External
//! input" (a new input channel on the device Settings chose), and the instrument window's
//! device and channel pickers.

use std::time::Instant;

use dawsome_engine::edit::Edit;
use dawsome_engine::input::InputFeeds;
use dawsome_engine::project::{ChannelId, ChannelKind, InputSource, Sound, SoundSource};
use dawsome_engine::time::Ticks;
use iced::Task;

use super::files::blocking;
use super::{App, Message};
use crate::audio::{self, Output};
use crate::live_input::{self, DeviceInfo, InputDevices, InputState, LiveInputs, Opening};
use crate::ui::clip_gesture;
use crate::ui::instrument_window::{ChannelsChoice, DeviceChoice, InputView, InstrumentMessage};

/// What a new input channel is called.
pub const NEW_INPUT_NAME: &str = "External input";

/// The input devices as last listed, for the input channel's window and Settings, and the
/// channel the window last listed them for.
#[derive(Debug, Default)]
pub(super) struct InputListing {
    asked_for: Option<ChannelId>,
    pub devices: Option<Result<Vec<DeviceInfo>, String>>,
}

impl App {
    /// Opens the input channels' devices as the document names them and hands the player their
    /// feeds. Runs after every edit and undo (nothing happens while what is open is what the
    /// document names, for the output playing now), and with `force` for a new document or an
    /// output opened again (its player, clock or block changed). A take recording from an input
    /// channel keeps its device until it stops; the sync its end runs then opens it again when
    /// the output was opened again meanwhile. A device that cannot open says why in the
    /// toolbar; its channels play silence.
    pub(super) fn sync_live_inputs(&mut self, force: bool) {
        if let Some(feeds) = self.live_inputs.take_pending() {
            self.hand_input_feeds(feeds);
        }
        let wanted = live_input::wanted(self.session.project());
        let playback = self.audio.as_ref().map(Output::playback_clock);
        let current =
            self.live_inputs.matches(&wanted) && self.live_inputs.opened_for(playback.as_ref());
        if (!force && current) || self.live_inputs.recording() {
            return;
        }
        let had = !self.live_inputs.is_empty();
        // The old streams close before the new ones open: a device opens once at a time.
        self.live_inputs = LiveInputs::default();
        if wanted.is_empty() {
            // The player lets go of the feeds of the devices just closed.
            if had {
                self.hand_input_feeds(InputFeeds::new());
            }
            return;
        }
        let opening = Opening {
            rate: self.session.project().sample_rate,
            output_block: self.output_block(),
            clock: self.session.record_clock(),
            playback,
        };
        let (inputs, feeds) = LiveInputs::open(&self.host.audio_inputs, &wanted, &opening);
        self.live_inputs = inputs;
        self.hand_input_feeds(feeds);
        if let Some((id, why)) = self.live_inputs.refusal() {
            let name = self
                .session
                .project()
                .channels
                .get(&id)
                .map_or("Input", |channel| channel.name.as_str());
            let said = format!("{name}: {why}");
            // After what the document's sounds said, when they said something.
            self.notice = Some(match self.notice.take() {
                Some(earlier) if earlier.contains(&said) => earlier,
                Some(earlier) => format!("{earlier}; {said}"),
                None => said,
            });
        }
    }

    /// Hands the player `feeds`; when it has not taken the sets already waiting, they wait for
    /// the next [`sync_live_inputs`](Self::sync_live_inputs).
    fn hand_input_feeds(&mut self, feeds: InputFeeds) {
        if let Err(feeds) = self.session.set_input_feeds(feeds) {
            self.live_inputs.keep_pending(feeds);
        }
    }

    /// Frames per buffer of the output: its stream's, else what one asks for.
    pub(super) fn output_block(&self) -> u64 {
        self.audio
            .as_ref()
            .map_or(u64::from(audio::BUFFER_FRAMES), Output::block_frames)
    }

    /// Reads the input channels' meters since the last read. On the playhead's timer.
    pub(super) fn read_live_meters(&mut self) {
        let now = Instant::now();
        let since = now.saturating_duration_since(self.live_meters_at);
        self.live_meters_at = now;
        self.live_inputs.read_meters(since);
    }

    /// Says in the toolbar when an input channel's device has failed. On the audio check's
    /// timer.
    pub(super) fn check_live_inputs(&mut self) {
        if let Some(device) = self.live_inputs.failed_device() {
            self.notice = Some(format!("Input stopped: {device} failed"));
        }
    }

    /// Whether an input meter shows and moves: the instrument window on an input channel.
    pub(super) fn live_meters_shown(&self) -> bool {
        self.instrument_on()
            .is_some_and(|id| self.live_inputs.channel(id).is_some())
    }

    /// The input channel Record → Input records from: the selected channel, when it is an input
    /// channel whose device is open. With none, the system's default input is recorded.
    pub(super) fn recording_input_channel(&self) -> Option<ChannelId> {
        let id = self.selected_channel?;
        let input = self.live_inputs.channel(id)?;
        matches!(input.state, InputState::Open { .. }).then_some(id)
    }

    /// Lists the input devices: the system's in the background, or the ones the host names (a
    /// scripted run's none) at once.
    pub(super) fn list_inputs(&mut self) -> Task<Message> {
        match &self.host.audio_inputs {
            InputDevices::Given(devices) => {
                self.input_listing.devices = Some(Ok(devices.clone()));
                Task::none()
            }
            InputDevices::System => Task::perform(blocking(live_input::system_devices), |listed| {
                Message::InputsListed(
                    listed.unwrap_or_else(|| Err("the devices could not be listed".to_owned())),
                )
            }),
        }
    }

    /// After every message: the devices are listed as the instrument window comes to an input
    /// channel (opened or retargeted), so its picker offers what is plugged in now.
    pub(super) fn list_inputs_for_window(&mut self) -> Task<Message> {
        let on_input = self.instrument_on().filter(|id| {
            self.session
                .project()
                .channels
                .get(id)
                .is_some_and(|channel| channel.kind == ChannelKind::Input)
        });
        if on_input.is_none() || on_input == self.input_listing.asked_for {
            self.input_listing.asked_for = on_input;
            return Task::none();
        }
        self.input_listing.asked_for = on_input;
        self.list_inputs()
    }

    /// The input devices listed, as the pickers know them: none before a listing came back.
    fn listed_devices(&self) -> &[DeviceInfo] {
        match &self.input_listing.devices {
            Some(Ok(devices)) => devices,
            _ => &[],
        }
    }

    /// The source a new input channel plays: the device Settings chose (else the system's
    /// default input), its first two channels as a stereo sound.
    fn new_input_source(&self) -> InputSource {
        InputSource {
            device: self.settings.audio_input().unwrap_or("").to_owned(),
            ..InputSource::default_device()
        }
    }

    /// The new input channel's id and the edit adding it at the end of the rack.
    fn new_input_channel(&self) -> (ChannelId, Edit) {
        let id = self.session.project().next_channel_id;
        let add = Edit::AddInputChannel {
            id,
            name: NEW_INPUT_NAME.to_owned(),
            input: self.new_input_source(),
        };
        (id, add)
    }

    /// Adds an input channel to the end of the rack, one undo step, and selects it: what a
    /// double press or Enter on the Library's "External input" does.
    pub(super) fn add_input_channel(&mut self) -> ChannelId {
        let (id, add) = self.new_input_channel();
        self.picker = None;
        self.commit(add);
        if self.error.is_none() {
            self.select_channel(id);
        }
        id
    }

    /// "External input" let go of: on a channel's header, that channel plays the input; on a
    /// playlist lane, a new input channel and a clip of the rack's pattern at the floor-snapped
    /// tick there, one undo step, as an instrument set's drop; elsewhere on the rack, a new input
    /// channel; the channel selected. Anywhere else, nothing.
    pub(super) fn drop_input(&mut self) {
        if let (true, Some(channel)) = (self.browse.rack_hover, self.browse.drop_header) {
            let source = SoundSource::input(self.new_input_source());
            self.replace_sound(channel, source, NEW_INPUT_NAME);
            return;
        }
        let (id, add) = self.new_input_channel();
        let project = self.session.project();
        let edit = if let Some((track, tick)) = self.arranger.drop_target(project) {
            let place = self.pattern.and_then(|pattern| {
                let index = clip_gesture::index_of(project, track)?;
                clip_gesture::place(project, pattern, index, tick, Ticks(1))
            });
            match place {
                Some(place) => Edit::Transaction(vec![add, place]),
                None => add,
            }
        } else if self.browse.rack_hover {
            add
        } else {
            return;
        };
        self.commit(edit);
        if self.error.is_none() {
            self.select_channel(id);
        }
    }

    /// The instrument window's input messages for input channel `channel`: another device (its
    /// channels kept where the device has them, else its first pair, else its first channel)
    /// or other channels, each one undo step; Refresh lists the devices again.
    pub(super) fn input_message(&mut self, channel: ChannelId, message: InstrumentMessage) {
        let Some(shown) = self.session.project().channels.get(&channel) else {
            return;
        };
        let Some(source) = shown.input.clone() else {
            return;
        };
        let chosen = match message {
            InstrumentMessage::InputDevice(device) => {
                let count = self.device_channels(&device);
                let fits = source
                    .channels
                    .sides()
                    .iter()
                    .all(|&side| count.is_none_or(|count| side < count));
                let channels = if fits {
                    source.channels
                } else {
                    live_input::channel_choices(count.unwrap_or(2))[0]
                };
                InputSource { device, channels }
            }
            InstrumentMessage::InputChannels(channels) => InputSource {
                channels,
                ..source.clone()
            },
            InstrumentMessage::RefreshInputs => {
                self.input_listing.asked_for = None;
                self.list_midi_outputs();
                return;
            }
            _ => return,
        };
        if chosen == source {
            return;
        }
        self.commit(Edit::ReplaceChannelSound {
            channel,
            name: shown.name.clone(),
            source: Box::new(SoundSource::input(chosen)),
            sound: Sound::default(),
        });
    }

    /// How many channels `device` has, when it is listed.
    fn device_channels(&self, device: &str) -> Option<u16> {
        self.listed_devices()
            .iter()
            .find(|info| info.name == device)
            .map(|info| info.channels)
    }

    /// What input channel `channel`'s window shows of its input: the devices listed (its own
    /// among them, "not plugged in" when it is not), the channels its device has, its level and
    /// how far behind it is heard, or why its device did not open.
    pub(super) fn input_view(&self, channel: ChannelId) -> Option<InputView> {
        let shown = self.session.project().channels.get(&channel)?;
        let source = shown.input.as_ref()?;
        let words = |name: &str| live_input::device_label(name).to_owned();
        let mut devices: Vec<DeviceChoice> = vec![DeviceChoice {
            name: String::new(),
            words: words(""),
        }];
        devices.extend(
            self.listed_devices()
                .iter()
                .filter(|info| !info.name.is_empty())
                .map(|info| DeviceChoice {
                    name: info.name.clone(),
                    words: words(&info.name),
                }),
        );
        let device = match devices.iter().find(|choice| choice.name == source.device) {
            Some(choice) => choice.clone(),
            None => {
                let missing = DeviceChoice {
                    name: source.device.clone(),
                    words: format!("{} (not plugged in)", words(&source.device)),
                };
                devices.push(missing.clone());
                missing
            }
        };
        let live = self.live_inputs.channel(channel);
        let open_channels = live.and_then(|input| match input.state {
            InputState::Open {
                device_channels, ..
            } => Some(device_channels),
            InputState::Refused(_) => None,
        });
        let count = self
            .device_channels(&source.device)
            .or(open_channels)
            .unwrap_or(2);
        let mut channels: Vec<ChannelsChoice> = live_input::channel_choices(count)
            .into_iter()
            .map(ChannelsChoice)
            .collect();
        let picked = ChannelsChoice(source.channels);
        if !channels.contains(&picked) {
            channels.push(picked);
        }
        let output_block = self.output_block();
        let rate = self.session.project().sample_rate;
        let (status, refused) = match live.map(|input| (&input.state, input)) {
            Some((InputState::Refused(why), _)) => (why.clone(), true),
            Some((
                InputState::Open {
                    block, resampled, ..
                },
                input,
            )) => {
                // A device block reaches the monitor resampled, as that many project frames.
                let block = block.map(|frames| match resampled {
                    Some(resampled) => {
                        live_input::project_frames(frames, resampled.device_rate, rate)
                    }
                    None => frames,
                });
                let cushion = input
                    .cushion()
                    .unwrap_or_else(|| live_input::first_cushion(block, output_block));
                let delay = resampled.map_or(0, |resampled| resampled.delay);
                let mut words = live_input::behind_words(cushion, output_block, delay, rate);
                let dry = input.underruns();
                if dry > 0 {
                    words.push_str(&format!(" \u{b7} ran dry {dry} times"));
                }
                (words, false)
            }
            None => ("Not open".to_owned(), true),
        };
        Some(InputView {
            devices,
            device,
            channels,
            picked,
            level: live.map_or(0.0, live_input::ChannelInput::level),
            status,
            refused,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use dawsome_engine::edit::Edit;
    use dawsome_engine::project::ChannelKind;
    use dawsome_engine::project::{InputChannels, InputSource, Sound, SoundSource};
    use dawsome_engine::realtime::Player;

    use super::super::tests::app_hosted;
    use super::super::{Message, RecordMessage};
    use super::*;
    use crate::document::Host;
    use crate::live_input::{ChannelInput, DeviceInfo, InputDevices};
    use crate::play_to::RecordSource;
    use crate::project_file;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");
    /// The device p16_unity.json's input channel names.
    const LINE_INPUT: &str = "USB Audio Line Input";

    fn line_input() -> Vec<DeviceInfo> {
        vec![DeviceInfo {
            name: LINE_INPUT.to_owned(),
            channels: 2,
            rates: Vec::new(),
        }]
    }

    /// The project at `path` in the window, the input devices `devices` listed (none opens).
    fn app_with(path: &Path, devices: Vec<DeviceInfo>) -> (App, Player) {
        let project = project_file::open(path).expect("project opens").project;
        let host = Host {
            audio_inputs: InputDevices::Given(devices),
            ..Host::offline()
        };
        app_hosted(project, path, host)
    }

    fn unity() -> PathBuf {
        Path::new(FIXTURES).join("p16_unity.json")
    }

    /// `count` device buffers of 512 stereo frames: frame n's left side (n + 1) / 65536 counted
    /// from `from`, its right side silence.
    fn buffers(from: usize, count: usize) -> Vec<Vec<f32>> {
        (0..count)
            .map(|buffer| {
                (0..512)
                    .flat_map(|frame| {
                        let n = from + buffer * 512 + frame;
                        [(n + 1) as f32 / 65536.0, 0.0]
                    })
                    .collect()
            })
            .collect()
    }

    // The document's input channel opens its device as the app starts, and the player plays the
    // device's frames through the channel while stopped (monitoring): once a cushion of one
    // input block and one output block (512 + 512 frames) waits, the frames fed are the frames
    // heard, in order (unity: volume 1, centered, stereo).
    #[test]
    fn an_input_channel_plays_its_device_through_the_player() {
        let (mut app, mut player) = app_with(&unity(), line_input());
        let id = ChannelId(1);
        assert!(matches!(
            app.live_inputs.channel(id).map(|input| &input.state),
            Some(InputState::Open { .. })
        ));
        let mut out = [[1.0f32; 2]; 512];
        player.process(&mut out);
        assert_eq!(out, [[0.0; 2]; 512], "nothing fed: silence");
        for buffer in buffers(0, 4) {
            app.live_inputs.feed_given(id, &buffer, 2, None);
        }
        let mut heard = Vec::new();
        for _ in 0..4 {
            player.process(&mut out);
            heard.extend_from_slice(&out);
        }
        let fed: Vec<[f32; 2]> = buffers(0, 4)
            .concat()
            .chunks_exact(2)
            .map(|pair| [pair[0], pair[1]])
            .collect();
        assert!(heard == fed, "the frames fed, in order");
        let _ = app.update(Message::Tick);
        assert!(
            app.live_inputs.channel(id).expect("open").level() > 0.0,
            "metered"
        );
    }

    /// A stand-in for a line input as PipeWire offers one: 48000 Hz only.
    fn line_input_at_48k() -> Vec<DeviceInfo> {
        vec![DeviceInfo {
            name: LINE_INPUT.to_owned(),
            channels: 2,
            rates: vec![48_000..=48_000],
        }]
    }

    /// p16_unity (its input channel at volume 1, centered, stereo) made a 44100 Hz project, its
    /// input on the 48 kHz stand-in.
    fn unity_at_44k() -> (App, Player) {
        let mut project = project_file::open(&unity()).expect("opens").project;
        project.sample_rate = 44_100;
        let host = Host {
            audio_inputs: InputDevices::Given(line_input_at_48k()),
            ..Host::offline()
        };
        app_hosted(project, &unity(), host)
    }

    /// The source's level: −6 dBFS.
    const TONE_LEVEL: f64 = 0.5;

    /// The 48 kHz source's frame `n`: a 440 Hz sine at [`TONE_LEVEL`] on the left, silence on the
    /// right.
    fn tone_frame(n: usize) -> [f32; 2] {
        let phase = 2.0 * std::f64::consts::PI * 440.0 * n as f64 / 48_000.0;
        [(TONE_LEVEL * phase.sin()) as f32, 0.0]
    }

    /// The player's output over `calls` calls of 512 frames, the stand-in fed before each call
    /// up to the device frames that call plays (ceil((call + 1) · 512 · 48000 / 44100)) and 1024
    /// more, in buffers of `split` frames, the last one short when it must be. So what waits in
    /// the monitor before each call is the same whatever the split, and stays between its
    /// cushion and the most it keeps (no frame dropped, none missing). Also the monitor's
    /// counts: how often it ran dry and frames it dropped.
    fn resampled_run(split: usize, calls: usize) -> (Vec<[f32; 2]>, u64, u64) {
        let (mut app, mut player) = unity_at_44k();
        let id = ChannelId(1);
        let stats = app
            .live_inputs
            .channel(id)
            .and_then(ChannelInput::monitor_stats)
            .expect("open");
        let mut fed = 0;
        let mut heard = Vec::new();
        let mut out = [[0.0f32; 2]; 512];
        for call in 0..calls as u64 {
            let wanted = ((call + 1) * 512 * 48_000).div_ceil(44_100);
            let target = 1024 + usize::try_from(wanted).expect("small");
            while fed < target {
                let len = split.min(target - fed);
                let buffer: Vec<f32> = (fed..fed + len).flat_map(tone_frame).collect();
                app.live_inputs.feed_given(id, &buffer, 2, None);
                fed += len;
            }
            player.process(&mut out);
            heard.extend_from_slice(&out);
        }
        let counts =
            |count: &std::sync::atomic::AtomicU64| count.load(std::sync::atomic::Ordering::Relaxed);
        (heard, counts(&stats.underruns), counts(&stats.skipped))
    }

    // An input device that does not offer the project's rate opens at its own and is
    // resampled. A line input found in use offers 48000 Hz only; on a 44100 Hz
    // project a 440 Hz sine at −6 dBFS fed through it plays, through the input channel, at
    // 440 Hz (zero crossings over a steady half second; unresampled it would be
    // 440 · 44100 / 48000 = 404.25 Hz) at the source's RMS (0.5 / √2, within 0.1 dB; the
    // half second is 220 whole periods), and the same bits for device buffers of 1, 7, 64 and
    // 4096 frames from the first frame played (when play starts follows the device's buffers:
    // the monitor's cushion is sized by them). Nothing ran dry or was
    // dropped. The window's line adds the resampler's delay.
    #[test]
    fn an_input_at_another_rate_is_resampled_to_the_project_rate() {
        let (mut app, _player) = unity_at_44k();
        let id = ChannelId(1);
        let state = app
            .live_inputs
            .channel(id)
            .expect("the channel's input")
            .state
            .clone();
        let InputState::Open {
            resampled: Some(resampled),
            block,
            ..
        } = state
        else {
            panic!("not opened resampled: {state:?}");
        };
        assert_eq!(resampled.device_rate, 48_000);
        app.open_instrument(id);
        let _ = app.update(Message::Tick);
        let view = app.input_view(id).expect("an input channel");
        let cushion = live_input::first_cushion(
            block.map(|frames| live_input::project_frames(frames, 48_000, 44_100)),
            512,
        );
        assert_eq!(
            (view.status, view.refused),
            (
                live_input::behind_words(cushion, 512, resampled.delay, 44_100),
                false
            )
        );
        let (heard, dry, dropped) = resampled_run(4096, 80);
        assert_eq!((dry, dropped), (0, 0), "ran dry, dropped");
        // From the first frame played: the monitor's wait before it follows the device's
        // buffers, the frames after it do not.
        let played = |heard: &[[f32; 2]]| -> Vec<[f32; 2]> {
            let first = heard
                .iter()
                .position(|frame| frame[0] != 0.0)
                .unwrap_or(heard.len());
            heard[first..].to_vec()
        };
        let heard = played(&heard);
        assert!(
            heard.len() >= 12_288 + 22_050,
            "{} frames played",
            heard.len()
        );
        let steady: Vec<f64> = heard[12_288..12_288 + 22_050]
            .iter()
            .map(|frame| f64::from(frame[0]))
            .collect();
        let crossings: Vec<f64> = steady
            .windows(2)
            .enumerate()
            .filter(|(_, pair)| pair[0] < 0.0 && pair[1] >= 0.0)
            .map(|(at, pair)| at as f64 + pair[0] / (pair[0] - pair[1]))
            .collect();
        let span = crossings.last().expect("crossings") - crossings[0];
        let hz = (crossings.len() - 1) as f64 * 44_100.0 / span;
        let rms =
            (steady.iter().map(|value| value * value).sum::<f64>() / steady.len() as f64).sqrt();
        let db = 20.0 * (rms / (TONE_LEVEL / 2f64.sqrt())).log10();
        println!("{hz:.3} Hz, {db:+.4} dB, delay {} frames", resampled.delay);
        assert!((hz - 440.0).abs() <= 1.0, "{hz} Hz");
        assert!(db.abs() <= 0.1, "{db} dB");
        assert!(
            heard.iter().all(|frame| frame[1] == 0.0),
            "the right side silent"
        );
        let bits = |heard: &[[f32; 2]]| -> Vec<[u32; 2]> {
            heard.iter().map(|frame| frame.map(f32::to_bits)).collect()
        };
        let whole = bits(&heard);
        for split in [1, 7, 64] {
            let (other, dry, dropped) = resampled_run(split, 80);
            assert_eq!((dry, dropped), (0, 0), "{split}: ran dry, dropped");
            let other = bits(&played(&other));
            let common = other.len().min(whole.len());
            assert!(common >= 30_000, "{split}: {common} frames played");
            assert!(
                other[..common] == whole[..common],
                "buffers of {split} frames differ"
            );
        }
    }

    // The input callback's path through the resampler never touches the heap: odd buffer sizes
    // from the 48 kHz stand-in into a 44.1 kHz project's monitor and record rings, a take's
    // first frame given and not.
    #[test]
    fn a_resampled_input_reaches_its_rings_without_the_heap() {
        use crate::counting_allocator::heap_calls_during;
        let control = heap_calls_during(|| drop(std::hint::black_box(vec![0u8; 64])));
        assert!(control > 0, "the counting allocator is not counting");
        let (mut app, _player) = unity_at_44k();
        let buffer = vec![0.25f32; 2 * 1500];
        let mut calls = 0;
        for round in 0..300usize {
            let frames = 1 + (round * 37) % 1500;
            let keep = (round % 2 == 0).then_some(round % 50);
            calls += heap_calls_during(|| {
                app.live_inputs
                    .feed_given(ChannelId(1), &buffer[..2 * frames], 2, keep);
            });
        }
        assert_eq!(calls, 0, "the input path made {calls} heap calls");
    }

    // A device that is not listed refuses its channel: the toolbar says why and the channel
    // plays silence. Choosing a listed device is one undo step that opens it; undo closes it
    // and refuses again.
    #[test]
    fn a_missing_device_is_refused_and_another_input_opens_as_chosen() {
        let other = vec![DeviceInfo {
            name: "Generic USB Audio".to_owned(),
            channels: 4,
            rates: Vec::new(),
        }];
        let (mut app, _player) = app_with(&unity(), other);
        let id = ChannelId(1);
        let refused = |app: &App| {
            matches!(
                app.live_inputs.channel(id).map(|input| &input.state),
                Some(InputState::Refused(_))
            )
        };
        assert!(refused(&app));
        let notice = app.notice.clone().unwrap_or_default();
        assert!(
            notice.contains("Oriole") && notice.contains(LINE_INPUT),
            "{notice}"
        );
        let chosen = InputSource {
            device: "Generic USB Audio".to_owned(),
            channels: InputChannels::Stereo(2, 3),
        };
        app.commit(Edit::ReplaceChannelSound {
            channel: id,
            name: "Oriole".to_owned(),
            source: Box::new(SoundSource::input(chosen.clone())),
            sound: Sound::default(),
        });
        let input = app.live_inputs.channel(id).expect("the channel's input");
        assert_eq!(input.source, chosen);
        assert_eq!(
            input.state,
            InputState::Open {
                device_channels: 4,
                block: Some(512),
                resampled: None,
            }
        );
        assert_eq!(
            app.session.history().undo_name(),
            Some("Change input"),
            "one named step"
        );
        let _ = app.update(Message::Undo);
        assert!(refused(&app), "undo gives the missing device back");
    }

    // Deleting the input channel closes its device and takes its feed from the player: the
    // player swaps in the empty set at its next call and the feed comes back to be dropped
    // here, so nothing but this test holds what the feed shared with the window.
    #[test]
    fn an_input_channel_deleted_closes_its_device() {
        let (mut app, mut player) = app_with(&unity(), line_input());
        let stats = app
            .live_inputs
            .channel(ChannelId(1))
            .and_then(ChannelInput::monitor_stats)
            .expect("open");
        player.process(&mut [[0.0; 2]; 64]);
        assert_eq!(
            std::sync::Arc::strong_count(&stats),
            3,
            "the window, the feed, this"
        );
        app.commit(Edit::DeleteChannel { id: ChannelId(1) });
        assert!(app.live_inputs.is_empty());
        player.process(&mut [[0.0; 2]; 64]);
        let _ = app.update(Message::Tick);
        assert_eq!(
            std::sync::Arc::strong_count(&stats),
            1,
            "the feed handed to the player is gone"
        );
        let _ = app.update(Message::Undo);
        assert!(
            app.live_inputs.channel(ChannelId(1)).is_some(),
            "back with undo"
        );
    }

    fn temp_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("dawsome-app-inputs-{}", std::process::id()))
            .join(test);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    // Record → Input with the input channel selected records its device's frames (its record
    // ring, its two channels) and opens no default input; the take ends as a new audio clip of
    // exactly the frames fed, and the channel gets its record feed back for the next take.
    #[test]
    fn record_input_takes_the_selected_input_channel() {
        let dir = temp_dir("record");
        let path = dir.join("p16_unity.json");
        fs::copy(unity(), &path).expect("copies the project");
        let (mut app, _player) = app_with(&path, line_input());
        let id = ChannelId(1);
        // Checked before arming: armed for the default input, a test would open a real device.
        assert_eq!(app.recording_input_channel(), Some(id));
        for message in [
            RecordMessage::Source(RecordSource::Input),
            RecordMessage::ToggleArm,
        ] {
            let _ = app.update(Message::Record(message));
        }
        assert!(!app.default_input_open(), "no default input opened");
        let take = dir.join("take.wav");
        assert!(app.start_take_into(take.clone()), "the take starts");
        for buffer in buffers(0, 3) {
            app.live_inputs.feed_given(id, &buffer, 2, Some(0));
        }
        app.finish_take();
        let mut reader = hound::WavReader::open(&take).expect("the take's file");
        assert_eq!(reader.spec().channels, 2);
        let written: Vec<f32> = reader.samples::<f32>().map(|v| v.expect("reads")).collect();
        assert!(written == buffers(0, 3).concat(), "the frames fed");
        assert!(
            app.live_inputs.take_record_feed(id).is_some(),
            "the record feed came back"
        );
    }

    // A take from an input running at another rate than the project's keeps
    // the device's frames in its record ring, and the take's writer converts them as it does for
    // the default input: three 512-frame buffers at 48 kHz are ceil(1536 · 44100 / 48000) =
    // 1412 frames in a 44.1 kHz file.
    #[test]
    fn a_take_from_a_resampled_input_is_written_at_the_project_rate() {
        let dir = temp_dir("record_resampled");
        let path = dir.join("p16_unity.json");
        fs::copy(unity(), &path).expect("copies the project");
        let mut project = project_file::open(&path).expect("opens").project;
        project.sample_rate = 44_100;
        let host = Host {
            audio_inputs: InputDevices::Given(line_input_at_48k()),
            ..Host::offline()
        };
        let (mut app, _player) = app_hosted(project, &path, host);
        let id = ChannelId(1);
        assert_eq!(app.recording_input_channel(), Some(id));
        for message in [
            RecordMessage::Source(RecordSource::Input),
            RecordMessage::ToggleArm,
        ] {
            let _ = app.update(Message::Record(message));
        }
        let take = dir.join("take.wav");
        assert!(app.start_take_into(take.clone()), "the take starts");
        for buffer in buffers(0, 3) {
            app.live_inputs.feed_given(id, &buffer, 2, Some(0));
        }
        app.finish_take();
        let reader = hound::WavReader::open(&take).expect("the take's file");
        assert_eq!(
            (
                reader.spec().sample_rate,
                reader.spec().channels,
                reader.duration()
            ),
            (44_100, 2, 1412)
        );
    }

    fn devices() -> Vec<DeviceInfo> {
        vec![
            DeviceInfo {
                name: LINE_INPUT.to_owned(),
                channels: 2,
                rates: Vec::new(),
            },
            DeviceInfo {
                name: "Generic USB Audio".to_owned(),
                channels: 4,
                rates: Vec::new(),
            },
        ]
    }

    // The instrument window on an input channel lists the devices as it opens: its picker
    // offers the system's default, then each device; its channel picker the device's pairs, then
    // each channel; its status how far behind the input is heard (a 512-frame input block and a
    // 512-frame output block of cushion, and the 512-frame output: 1536 frames at 48 kHz, 32
    // ms). Another device keeps the channels where it has them; other channels are one step
    // each; Refresh lists the devices again.
    #[test]
    fn the_window_of_an_input_channel_picks_its_device_and_channels() {
        let (mut app, _player) = app_with(&unity(), devices());
        let id = ChannelId(1);
        app.open_instrument(id);
        let _ = app.update(Message::Tick);
        let view = app.input_view(id).expect("an input channel");
        let words: Vec<&str> = view.devices.iter().map(|d| d.words.as_str()).collect();
        assert_eq!(
            words,
            ["System default input", LINE_INPUT, "Generic USB Audio"]
        );
        assert_eq!(view.device.name, LINE_INPUT);
        let channels: Vec<InputChannels> = view.channels.iter().map(|c| c.0).collect();
        assert_eq!(
            channels,
            [
                InputChannels::Stereo(0, 1),
                InputChannels::Mono(0),
                InputChannels::Mono(1)
            ]
        );
        assert_eq!(
            (view.status.as_str(), view.refused),
            ("live, about 32 ms behind", false)
        );
        let instrument = |message| Message::Instrument(message);
        let _ = app.update(instrument(InstrumentMessage::InputDevice(
            "Generic USB Audio".to_owned(),
        )));
        let source = |app: &App| app.session.project().channels[&id].input.clone();
        assert_eq!(
            source(&app),
            Some(InputSource {
                device: "Generic USB Audio".to_owned(),
                channels: InputChannels::Stereo(0, 1),
            })
        );
        assert_eq!(app.session.history().undo_name(), Some("Change input"));
        let view = app.input_view(id).expect("an input channel");
        assert_eq!(view.channels.len(), 2 + 4, "two pairs and four channels");
        let _ = app.update(instrument(InstrumentMessage::InputChannels(
            InputChannels::Mono(3),
        )));
        assert_eq!(
            source(&app).map(|source| source.channels),
            Some(InputChannels::Mono(3))
        );
        assert_eq!(app.session.history().undo_len(), 2);
        // Back on the two-channel device, channel 4 is not there: its first pair is taken.
        let _ = app.update(instrument(InstrumentMessage::InputDevice(
            LINE_INPUT.to_owned(),
        )));
        assert_eq!(
            source(&app).map(|source| source.channels),
            Some(InputChannels::Stereo(0, 1))
        );
        let _ = app.update(instrument(InstrumentMessage::RefreshInputs));
        assert_eq!(app.input_listing.asked_for, Some(id), "listed again");
    }

    // A device that is not plugged in still shows as the channel's, marked so, and the window
    // says why it plays nothing.
    #[test]
    fn a_missing_device_shows_in_the_picker_marked_and_its_reason() {
        let (mut app, _player) = app_with(&unity(), Vec::new());
        app.open_instrument(ChannelId(1));
        let _ = app.update(Message::Tick);
        let view = app.input_view(ChannelId(1)).expect("an input channel");
        assert_eq!(view.device.words, format!("{LINE_INPUT} (not plugged in)"));
        assert!(
            view.refused && view.status.contains("is not there"),
            "{}",
            view.status
        );
    }

    // The Library's External input: a right press adds it and opens its window; dragged onto the
    // rack it adds another, selected; let go of on a channel's header it gives that channel the
    // input in one step, the renamed channel keeping its name.
    #[test]
    fn the_library_external_input_adds_drops_and_replaces() {
        use crate::ui::browser::{BrowserMessage, Pick};
        let path = Path::new(FIXTURES).join("p16_muted.json");
        let (mut app, _player) = app_with(&path, line_input());
        let picked = |button, at| {
            Message::Browser(BrowserMessage::Picked {
                pick: Pick::ExternalInput,
                button,
                at,
            })
        };
        let _ = app.update(picked(iced::mouse::Button::Right, iced::Point::ORIGIN));
        assert_eq!(app.instrument_on(), Some(ChannelId(3)), "added and opened");
        assert_eq!(
            app.session.project().channels[&ChannelId(3)].kind,
            ChannelKind::Input
        );
        let from = iced::Point::new(100.0, 300.0);
        let _ = app.update(picked(iced::mouse::Button::Left, from));
        let _ = app.update(Message::Browser(BrowserMessage::Moved(iced::Point::new(
            400.0, 300.0,
        ))));
        let _ = app.update(Message::Media(super::super::MediaMessage::RackHover(true)));
        let _ = app.update(Message::Browser(BrowserMessage::Released));
        assert_eq!(
            app.selected_channel,
            Some(ChannelId(4)),
            "dropped on the rack"
        );
        let _ = app.update(picked(iced::mouse::Button::Left, from));
        let _ = app.update(Message::Browser(BrowserMessage::Moved(iced::Point::new(
            400.0, 300.0,
        ))));
        let _ = app.update(Message::Media(super::super::MediaMessage::RackHover(true)));
        let _ = app.update(Message::Media(super::super::MediaMessage::OverHeader(
            Some(ChannelId(2)),
        )));
        let _ = app.update(Message::Browser(BrowserMessage::Released));
        let hit = &app.session.project().channels[&ChannelId(2)];
        assert_eq!((hit.name.as_str(), hit.kind), ("hit", ChannelKind::Input));
        assert_eq!(app.session.history().undo_name(), Some("Change input"));
        assert!(
            app.live_inputs.channel(ChannelId(2)).is_some(),
            "its device open"
        );
    }

    /// A fake output device: any name opens, holding the player as a stream would.
    fn open_any(
        player: Player,
        name: Option<&str>,
    ) -> Result<crate::audio::Output, crate::audio::StartFailure> {
        Ok(match name {
            Some(name) => crate::audio::Output::holding_on(player, name),
            None => crate::audio::Output::holding(player),
        })
    }

    // A stale clock (found 2026-10-23): another output chosen while a take records
    // from an input channel opens the output again, but the take keeps the input's device; once
    // the take ends, the input opens again for the new output (its clock and block), so the
    // next take's frames are lined up by the output that plays them.
    #[test]
    fn an_output_opened_again_during_a_take_reopens_the_input_after_it() {
        let dir = temp_dir("output_during_take");
        let path = dir.join("p16_unity.json");
        fs::copy(unity(), &path).expect("copies the project");
        let project = project_file::open(&path).expect("opens").project;
        let host = Host {
            audio: crate::document::AudioChoice::FakeNamed(open_any),
            audio_inputs: InputDevices::Given(line_input()),
            ..Host::offline()
        };
        let (mut app, player) = app_hosted(project, &path, host);
        app.audio = Some(crate::audio::Output::holding(player));
        app.sync_live_inputs(true);
        let id = ChannelId(1);
        let feed_of = |app: &App| {
            app.live_inputs
                .channel(id)
                .and_then(ChannelInput::monitor_stats)
                .expect("open")
        };
        let before = feed_of(&app);
        assert_eq!(app.recording_input_channel(), Some(id));
        for message in [
            RecordMessage::Source(RecordSource::Input),
            RecordMessage::ToggleArm,
        ] {
            let _ = app.update(Message::Record(message));
        }
        assert!(app.start_take_into(dir.join("take.wav")), "the take starts");
        app.settings.set_audio_output(Some("USB Audio".to_owned()));
        app.apply_output_choice();
        assert_eq!(
            app.audio.as_ref().and_then(crate::audio::Output::asked),
            Some("USB Audio")
        );
        assert!(
            std::sync::Arc::ptr_eq(&before, &feed_of(&app)),
            "the take keeps its device"
        );
        app.finish_take();
        assert!(
            !std::sync::Arc::ptr_eq(&before, &feed_of(&app)),
            "opened again for the new output"
        );
    }
}
