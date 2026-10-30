//! Input latency compensation's placement known answers through the window
//! (tests/fixtures/gen_p23_expected.py): a take of the line in, recorded from the input channel's
//! stand-in device with a latency kept in the settings, lands that many frames earlier; the
//! document it leaves is compared byte for byte, and its file holds every frame fed.

use std::path::{Path, PathBuf};

use dawsome_engine::project::ChannelId;
use dawsome_engine::realtime::Player;
use dawsome_engine::time::Ticks;

use super::super::{App, Message, PlayMode};
use super::RecordMessage;
use crate::document::Host;
use crate::live_input::{DeviceInfo, InputDevices};
use crate::play_to::RecordSource;
use crate::project_file;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

fn fixture(name: &str) -> String {
    let text = std::fs::read_to_string(Path::new(FIXTURES).join(name)).expect("the fixture");
    text.trim_end().to_owned()
}

fn expected() -> serde_json::Value {
    serde_json::from_str(&fixture("p23_expected.json")).expect("parses")
}

fn number(value: &serde_json::Value) -> u64 {
    value.as_u64().expect("a number")
}

fn document(app: &App) -> String {
    app.session.project().to_json().expect("serializes")
}

fn record(app: &mut App, message: RecordMessage) {
    let _ = app.update(Message::Record(message));
}

/// p23_line.json in a folder of the test's own, its settings file holding `settings`, opened in
/// the window with the line input's stand-in listed (no device opens).
pub(super) fn open(test: &str, settings: &serde_json::Value) -> (App, Player, PathBuf) {
    open_with(test, settings, |host| host)
}

/// As [`open`], `adjust` saying what else the host offers (an output, say).
fn open_with(
    test: &str,
    settings: &serde_json::Value,
    adjust: fn(Host) -> Host,
) -> (App, Player, PathBuf) {
    let dir = std::env::temp_dir()
        .join(format!("dawsome-p23-{}", std::process::id()))
        .join(test);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("config/dawsome")).expect("temp dir");
    let path = dir.join("p23_line.json");
    std::fs::copy(Path::new(FIXTURES).join("p23_line.json"), &path).expect("copies");
    let file = dir.join("config/dawsome/settings.json");
    std::fs::write(&file, settings.to_string()).expect("writes the settings");
    let project = project_file::open(&path).expect("opens").project;
    let host = Host {
        audio_inputs: InputDevices::Given(vec![DeviceInfo {
            name: expected()["device"].as_str().expect("a name").to_owned(),
            channels: 2,
            rates: Vec::new(),
        }]),
        settings_file: Some(file),
        ..Host::offline()
    };
    let (app, player) = super::super::tests::app_hosted(project, &path, adjust(host));
    (app, player, dir)
}

/// Frame n of a take's feed, both sides: (n + 1) / 65536 on the left, its negative on the right.
fn take_feed(frames: usize) -> Vec<f32> {
    (0..frames)
        .flat_map(|n| {
            let value = (n + 1) as f32 / 65536.0;
            [value, -value]
        })
        .collect()
}

/// One placement case: a take of `case`'s source in song mode from its marker, `frames` long,
/// with its settings kept; then the document, the take's file, one undo step and the toolbar's
/// report as the generator says.
fn placement(name: &str) {
    let want = &expected()["place"][name];
    let (mut app, mut player, dir) = open(name, &want["settings"]);
    let before = document(&app);
    let input = want["source"] == "input";
    let id = ChannelId(1);
    if input {
        // Checked before arming: armed for the default input, a test would open a real device.
        assert_eq!(app.recording_input_channel(), Some(id));
    }
    let source = if input {
        RecordSource::Input
    } else {
        RecordSource::Master
    };
    record(&mut app, RecordMessage::Source(source));
    record(&mut app, RecordMessage::ToggleCountIn);
    let _ = app.update(Message::ModeSelected(PlayMode::Song));
    app.move_song_marker(Ticks(number(&want["marker"]) as i64));
    record(&mut app, RecordMessage::ToggleArm);
    let _ = app.update(Message::Play);
    assert!(app.recording.recording(), "{name}: a take");
    let frames = usize::try_from(number(&want["frames"])).expect("fits");
    let fed = take_feed(frames);
    if input {
        app.live_inputs.feed_given(id, &fed, 2, Some(0));
        player.process(&mut [[0.0; 2]; 512]);
    } else {
        player.process(&mut vec![[0.0; 2]; frames]);
    }
    let _ = app.update(Message::Stop);
    assert!(!app.recording.recording());
    let after = fixture(want["document"].as_str().expect("a document"));
    assert_eq!(document(&app), after, "{name}");
    let file = dir.join("samples/p23_line take 1.wav");
    let mut reader = hound::WavReader::open(&file).expect("the take's file");
    assert_eq!(reader.duration() as usize, frames, "{name}: the whole take");
    if input {
        let written: Vec<f32> = reader.samples::<f32>().map(|v| v.expect("reads")).collect();
        assert!(written == fed, "{name}: the file holds the frames fed");
    }
    let notice = app.notice.clone().unwrap_or_default();
    match want["report"].as_str() {
        Some(report) => assert_eq!(notice, report, "{name}"),
        None => assert!(
            !notice.contains("placed") && !notice.contains("shifted"),
            "{name}: {notice}"
        ),
    }
    assert_eq!(app.session.history().undo_len(), 1, "{name}: one undo step");
    let _ = app.update(Message::Undo);
    assert_eq!(document(&app), before, "{name}: undone");
    let _ = app.update(Message::Redo);
    assert_eq!(document(&app), after, "{name}: redone");
    assert!(file.is_file(), "{name}: the file stays");
}

// Known answer 2: from tick 192 with 1134 frames kept at 44.1 kHz (1234 at 48 kHz) the clip
// starts at tick 188, its first 234 frames skipped by its offset, 96 ticks long.
#[test]
fn p23_a_take_lands_earlier_by_the_latency_kept() {
    placement("shifted");
}

// Known answer 3: from tick 0 the clip starts at 0 and its offset skips the first 1234 frames.
#[test]
fn p23_a_take_from_tick_0_skips_its_first_frames() {
    placement("at_zero");
}

// Known answer 4: a latency longer than the take leaves nothing after the offset: the take is
// placed as before, and says it was not shifted.
#[test]
fn p23_a_take_shorter_than_the_latency_is_not_shifted() {
    placement("saturated");
}

// A figure set by hand 960 frames early places the take 20 ms later, at tick 196.
#[test]
fn p23_a_negative_latency_places_the_take_later() {
    placement("late");
}

// Known answer 4: no figure kept, the take lands as before.
#[test]
fn p23_with_no_latency_kept_a_take_lands_as_before() {
    placement("unset");
}

// A take of the master comes from the player's own frames: the figure kept for the line in does
// not move it.
#[test]
fn p23_a_take_of_the_master_is_never_shifted() {
    placement("master");
}

// ---------- the measurement, through Settings → Input latency ----------

/// A feed file as the stand-in device delivers it: interleaved stereo, q/32768, then silence to
/// the measurement's recording length.
fn feed_of(name: &str, frames: usize) -> Vec<f32> {
    let mut reader = hound::WavReader::open(Path::new(FIXTURES).join(name)).expect("the feed");
    let mut values: Vec<f32> = reader
        .samples::<i16>()
        .map(|value| f32::from(value.expect("reads")) / 32768.0)
        .collect();
    values.resize((2 * frames).max(values.len()), 0.0);
    values
}

fn press_latency(
    app: &mut App,
    button: fn(&crate::ui::settings_layout::LatencyLayout) -> iced::Rectangle,
) {
    let at = button(&app.settings_panel().latency_layout()).center();
    let _ = app.update(Message::Settings(
        crate::ui::settings_panel::SettingsMessage::Pressed(at),
    ));
}

/// Settings → Measure in p23_line.json (its input channel selected), the stand-in device fed
/// `feed` (none: silence) from the take's first frame and the player called as the output's
/// callback would be until the recordings are full; then the window's timer until the search is
/// done. The app, and its settings file.
fn measured(test: &str, feed: Option<&str>) -> (App, PathBuf) {
    let want = expected();
    let (mut app, mut player, dir) = open(test, &serde_json::json!({}));
    app.host.saves_settings = true;
    app.metronome = true;
    let before = document(&app);
    assert_eq!(app.recording_input_channel(), Some(ChannelId(1)));
    press_latency(&mut app, crate::ui::settings_layout::LatencyLayout::measure);
    assert!(
        app.measuring_latency(),
        "{test}: measuring ({:?})",
        app.notice
    );
    assert!(app.session.measuring(), "{test}: the document is silent");
    let frames = usize::try_from(number(&want["recording_frames"])).expect("fits");
    let values = match feed {
        Some(name) => feed_of(name, frames),
        None => vec![0.0; 2 * frames],
    };
    app.live_inputs
        .feed_given(ChannelId(1), &values, 2, Some(0));
    for _ in 0..=frames / 512 {
        player.process(&mut [[0.0; 2]; 512]);
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while app.measuring_latency() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(2));
        let _ = app.update(Message::Tick);
    }
    assert!(!app.measuring_latency(), "{test}: the measurement ended");
    assert!(!app.session.measuring(), "{test}: the document plays again");
    assert_eq!(document(&app), before, "{test}: no edit");
    assert_eq!(app.session.history().undo_len(), 0);
    let controls = app.session.controls_sent();
    use dawsome_engine::transport::Control;
    let record = controls.iter().position(|control| {
        *control
            == Control::Record {
                count_in: false,
                tap_master: true,
            }
    });
    let quiet = controls
        .iter()
        .position(|c| *c == Control::Metronome(false));
    let stop = controls.iter().position(|c| *c == Control::Stop);
    let back = controls.iter().position(|c| *c == Control::Metronome(true));
    assert!(
        quiet < record && record < stop && stop < back,
        "{test}: {controls:?}"
    );
    assert!(
        app.session.take_record_tap().is_some(),
        "{test}: the tap is back"
    );
    assert!(
        app.live_inputs.take_record_feed(ChannelId(1)).is_some(),
        "{test}: the input channel's record feed is back"
    );
    (app, dir.join("config/dawsome/settings.json"))
}

// Known answer 1 through the window: the clean feed's 1234 frames, the messy one's 2205 and the
// early one's -960 are measured, kept for the input channel's device with the default output,
// saved, and shown in ms; the measurement plays nothing of the document, records the master
// and the line in from the take's first frame, silences the metronome while it plays, and
// gives the record tap and the input channel's record feed back.
#[test]
fn p23_settings_measures_each_feeds_latency() {
    let want = expected();
    let device = want["device"].as_str().expect("a name");
    for (name, case) in want["measure"].as_object().expect("cases") {
        let Some(late) = case["latency"].as_i64() else {
            continue;
        };
        let (app, file) = measured(name, case["file"].as_str());
        let kept = app.settings.input_latency("", device).expect("kept");
        assert_eq!(
            (kept.input_latency_frames, kept.rate, kept.measured),
            (late, 48_000, true),
            "{name}"
        );
        let shown = case["shown"].as_str().expect("words");
        assert_eq!(
            app.notice.as_deref(),
            Some(&*format!("Input latency: {shown}"))
        );
        assert_eq!(app.settings_panel().latency.as_deref(), Some(shown));
        let saved = std::fs::read_to_string(&file).expect("saved");
        assert!(
            saved.contains(&format!("\"input_latency_frames\": {late}")),
            "{saved}"
        );
    }
}

// Known answer 4 through the window: a burst past the lags searched, hum alone and silence are
// "no click heard"; nothing is kept, and the panel says the latency is unknown.
#[test]
fn p23_settings_hears_no_click_in_silence_hum_or_too_late() {
    let want = expected();
    let device = want["device"].as_str().expect("a name");
    let refused = ["p23_feed_late.wav", "p23_feed_noise.wav"];
    for feed in refused.iter().map(|name| Some(*name)).chain([None]) {
        let (app, _) = measured(feed.unwrap_or("silence").trim_end_matches(".wav"), feed);
        assert_eq!(app.settings.input_latency("", device), None, "{feed:?}");
        let notice = app.notice.clone().unwrap_or_default();
        assert!(notice.starts_with("No click heard"), "{feed:?}: {notice}");
        assert_eq!(app.settings_panel().latency, None);
    }
}

// Measuring while the song plays is refused, and so is Play while measuring; Stop ends the
// measurement, keeping nothing, and the document plays again.
#[test]
fn p23_measuring_waits_for_the_song_to_stop() {
    let (mut app, _player, _dir) = open("refused", &serde_json::json!({}));
    let _ = app.update(Message::Play);
    press_latency(&mut app, crate::ui::settings_layout::LatencyLayout::measure);
    assert!(!app.measuring_latency());
    let notice = app.notice.clone().unwrap_or_default();
    assert!(notice.starts_with("Stop playback first"), "{notice}");
    let _ = app.update(Message::Stop);
    press_latency(&mut app, crate::ui::settings_layout::LatencyLayout::measure);
    assert!(app.measuring_latency());
    let _ = app.update(Message::Play);
    assert!(app.measuring_latency(), "Play waits");
    let _ = app.update(Message::Stop);
    assert!(!app.measuring_latency(), "Stop ends it");
    assert!(!app.session.measuring());
    assert!(app
        .settings
        .input_latency("", "USB Audio Line Input")
        .is_none());
}

// The figure by hand: a millisecond more or less from the one kept (0 when none) at the
// project's rate, within -50..500 ms; from the measured 25.7 ms, + gives 27 ms; Clear forgets it.
#[test]
fn p23_the_latency_is_set_by_hand_a_millisecond_at_a_time() {
    use crate::ui::settings_layout::LatencyLayout;
    let (mut app, _player, _dir) = open("by_hand", &serde_json::json!({}));
    let shown = |app: &App| app.settings_panel().latency;
    assert_eq!(shown(&app), None);
    press_latency(&mut app, LatencyLayout::up);
    press_latency(&mut app, LatencyLayout::up);
    assert_eq!(shown(&app).as_deref(), Some("2.0 ms (set by hand)"));
    let kept = app.line_in_latency().expect("kept");
    assert_eq!((kept.input_latency_frames, kept.rate), (96, 48_000));
    press_latency(&mut app, LatencyLayout::down);
    press_latency(&mut app, LatencyLayout::down);
    press_latency(&mut app, LatencyLayout::down);
    assert_eq!(shown(&app).as_deref(), Some("-1.0 ms (set by hand)"));
    for _ in 0..60 {
        press_latency(&mut app, LatencyLayout::down);
    }
    assert_eq!(shown(&app).as_deref(), Some("-50.0 ms (set by hand)"));
    app.settings
        .set_input_latency(crate::settings::InputLatency {
            output: String::new(),
            input: "USB Audio Line Input".to_owned(),
            input_latency_frames: 1234,
            rate: 48_000,
            measured: true,
        });
    press_latency(&mut app, LatencyLayout::up);
    assert_eq!(shown(&app).as_deref(), Some("27.0 ms (set by hand)"));
    press_latency(&mut app, LatencyLayout::clear);
    assert_eq!(shown(&app), None);
}

// ---------- edge cases ----------

/// The last metronome switch the player was sent.
fn last_metronome(app: &App) -> Option<bool> {
    use dawsome_engine::transport::Control;
    app.session
        .controls_sent()
        .iter()
        .rev()
        .find_map(|control| match control {
            Control::Metronome(on) => Some(*on),
            _ => None,
        })
}

// gen_p23 "loop": a loop cable from the output into the input at a gain of 1.25 and 2.0
// (the stand-in's block k is the output's block k - 3 times the gain, clipped, plus a -50 dBFS
// hum), each block fed just before the player's call that plays its time: the measurement keeps
// 1536 frames, and the output is the burst and then silence, never past the burst's level. Before
// the fix the input channel played the line in back out and the loop fed back.
#[test]
fn p23_a_loop_cable_at_any_gain_measures_the_loop() {
    let want = expected();
    let case = &want["loop"];
    let block = usize::try_from(number(&case["block"])).expect("fits");
    let late = usize::try_from(number(&case["blocks_late"])).expect("fits");
    let frames = usize::try_from(number(&want["recording_frames"])).expect("fits");
    let device = want["device"].as_str().expect("a name");
    let level = case["hum"].as_f64().expect("a level") as f32;
    let mut x = u32::try_from(number(&case["hum_seed"])).expect("a seed");
    let mut hum = move || {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        level * ((x >> 8) as f32 / 8_388_608.0 - 1.0)
    };
    let burst = dawsome_media::latency::burst(48_000);
    for gain in case["gains"].as_array().expect("gains") {
        let gain = gain.as_f64().expect("a gain") as f32;
        let (mut app, mut player, _dir) = open(&format!("loop_{gain}"), &serde_json::json!({}));
        press_latency(&mut app, crate::ui::settings_layout::LatencyLayout::measure);
        assert!(app.measuring_latency());
        let mut played: Vec<Vec<f32>> = Vec::new();
        let mut peak = 0.0f32;
        for k in 0..frames / block + late + 2 {
            let looped: Vec<f32> = match k.checked_sub(late) {
                Some(earlier) => played[earlier]
                    .iter()
                    .map(|value| (value * gain).clamp(-1.0, 1.0) + hum())
                    .collect(),
                None => (0..2 * block).map(|_| hum()).collect(),
            };
            app.live_inputs
                .feed_given(ChannelId(1), &looped, 2, Some(0));
            let mut out = vec![[0.0f32; 2]; block];
            player.process(&mut out);
            peak = out.iter().flatten().fold(peak, |most, v| most.max(v.abs()));
            played.push(out.into_iter().flatten().collect());
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while app.measuring_latency() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(2));
            let _ = app.update(Message::Tick);
        }
        let out: Vec<f32> = played.into_iter().flatten().collect();
        let after = out
            .iter()
            .skip(2 * burst.len())
            .filter(|v| **v != 0.0)
            .count();
        assert_eq!(
            after, 0,
            "gain {gain}: the output after the burst (peak {peak})"
        );
        assert!(peak <= 0.25, "gain {gain}: the output peaked at {peak}");
        let kept = app
            .settings
            .input_latency("", device)
            .map(|kept| kept.input_latency_frames);
        assert_eq!(
            kept,
            case["latency"].as_i64(),
            "gain {gain}: {:?}",
            app.notice
        );
        let shown = case["shown"].as_str().expect("words");
        assert_eq!(app.settings_panel().latency.as_deref(), Some(shown));
    }
}

fn open_named(
    player: Player,
    name: Option<&str>,
) -> Result<crate::audio::Output, crate::audio::StartFailure> {
    Ok(match name {
        Some(name) => crate::audio::Output::holding_on(player, name),
        None => crate::audio::Output::holding(player),
    })
}

// An output chosen in Settings plays from the next Play; Measure is such a play, so the
// clicks play through the chosen output and the figure is kept under the device that played
// them (before the fix the old stream played them and the figure went under the new name).
#[test]
fn p23_measure_plays_through_the_output_just_chosen() {
    use crate::document::{AudioChoice, OutputSource};
    let (mut app, player, _dir) = open_with("chosen_output", &serde_json::json!({}), |host| Host {
        audio: AudioChoice::FakeNamed(open_named),
        audio_outputs: OutputSource::Given(
            ["Speakers", "USB Audio"]
                .map(crate::audio::ListedOutput::named)
                .to_vec(),
        ),
        ..host
    });
    app.audio = Some(crate::audio::Output::holding(player));
    app.settings.set_audio_output(Some("USB Audio".to_owned()));
    press_latency(&mut app, crate::ui::settings_layout::LatencyLayout::measure);
    assert!(app.measuring_latency(), "{:?}", app.notice);
    let playing = app.audio.as_ref().and_then(crate::audio::Output::asked);
    assert_eq!(
        playing,
        Some("USB Audio"),
        "the clicks play through the chosen output"
    );
    assert_eq!(
        app.latency_output(),
        Some("USB Audio"),
        "kept under the device that played"
    );
    let _ = app.update(Message::Stop);
    assert!(!app.measuring_latency());
}

// The metronome switched off while a measurement runs stays off after it (the player is
// sent what the toolbar shows, not what it showed when the measurement began); and one switched
// on meanwhile is on after it.
#[test]
fn p23_the_metronome_after_a_measurement_is_the_toolbars() {
    for (before, after) in [(true, false), (false, true)] {
        let (mut app, _player, _dir) = open("metronome", &serde_json::json!({}));
        app.metronome = before;
        press_latency(&mut app, crate::ui::settings_layout::LatencyLayout::measure);
        assert!(app.measuring_latency());
        let _ = app.update(Message::ToggleMetronome);
        let _ = app.update(Message::Stop);
        assert!(!app.measuring_latency());
        assert_eq!(app.metronome, after);
        assert_eq!(last_metronome(&app), Some(after), "{before} then {after}");
    }
}

// Closing the window while a measurement records ends it: its two files and their folder
// are gone from the temporary folder.
#[test]
fn p23_closing_the_window_mid_measurement_leaves_no_files() {
    let (mut app, _player, _dir) = open("close", &serde_json::json!({}));
    let main = iced::window::Id::unique();
    app.set_main_window(main);
    press_latency(&mut app, crate::ui::settings_layout::LatencyLayout::measure);
    let folder = app.latency_folder().expect("recording");
    assert!(folder.join("input.wav").is_file() && folder.join("master.wav").is_file());
    let _ = app.update(Message::WindowCloseRequested(main));
    assert!(app.exiting());
    assert!(!app.measuring_latency());
    assert!(!folder.exists(), "{} left behind", folder.display());
}
