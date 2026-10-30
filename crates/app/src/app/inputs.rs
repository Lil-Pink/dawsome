//! The MIDI inputs in the window and the Settings window's General page that chooses them. The
//! system's ports are listed again every 2 s while the Settings window or the Library (which
//! lists them too) is open and every 10 s otherwise (there is no hotplug signal); the ones the
//! settings say play are connected, delivering notes to the session's player (on the channel
//! the document links the port to, else the selected one) and controller moves to the window's
//! queue. Closing a connection lets go of the keys its port holds, as their note-offs can no
//! longer come (every live note ends when the inbox has no room to say so). The choice is saved
//! by port name in the app's settings file.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::time::Duration;

use dawsome_engine::project::{stable_port_name, Channel, ChannelId, ChannelKind};
use dawsome_engine::realtime::EngineEvent;
use dawsome_engine::transport::Control;
use iced::Task;

use super::files::blocking;
use super::latency::LatencyMessage;
use super::{App, Message};
use crate::audio::{self, Chosen, ListedOutput, OutputChoice, OutputLister};
use crate::document::OutputSource;
use crate::midi::{self, Activity, Router};
use crate::ui::keys_page::KeysHit;
use crate::ui::library::{self as library_rows, Port};
use crate::ui::panels::Panels;
use crate::ui::settings_layout::{Hit, Layout, SettingsPage};
use crate::ui::settings_panel::{SettingsMessage, SettingsPanel};

/// What the toolbar says when a stream would not hand its player back.
const STOPPED_RESPONDING: &str =
    "Audio stopped: the output device stopped responding. Choose an output in Settings.";

/// How often the system's ports are listed again, with a panel that lists them open and with
/// none.
pub(super) const LISTING_OPEN: Duration = Duration::from_secs(2);
pub(super) const LISTING_CLOSED: Duration = Duration::from_secs(10);

/// How often the system's ports are listed with `panels` open: often while one that lists them
/// (the settings or the Library) is, so a device plugged in shows soon.
fn listing_period_for(panels: Panels) -> Duration {
    if panels.settings || panels.browser {
        LISTING_OPEN
    } else {
        LISTING_CLOSED
    }
}

impl App {
    /// Where this window's MIDI inputs deliver: the session's player and the window's queue,
    /// stamping the ports' lights on the window's clock.
    pub(super) fn router(&self) -> Router {
        Router::new(self.session.midi_inbox(), self.controller_queue.clone())
            .with_activity(self.midi_activity.clone())
    }

    /// The Library's MIDI device rows: each port listed (`links` naming the channel linked to
    /// each), its light, and where an unlinked port that plays sends its notes now.
    pub(super) fn library_ports(&self, links: &[(String, String)]) -> Vec<Port> {
        let mut ports = library_rows::ports(self.ports.names(), &self.settings.midi_inputs, links);
        let (goes, silent) = self.unlinked_target();
        let project = self.session.project();
        for port in &mut ports {
            port.lit = self.lit_ports.contains(&port.name);
            // By the device's stable name, so its input row names the channel its output
            // serves. The rows are the input ports': a device that is an output only has no row
            // to say it on (the plan's risk 10).
            port.out = project
                .channels
                .values()
                .find(|channel| {
                    channel.midi_out.as_ref().is_some_and(|out| {
                        stable_port_name(&out.port) == stable_port_name(&port.name)
                    })
                })
                .map(|channel| channel.name.clone());
            if port.channel.is_none() && port.plays && !port.unplugged {
                port.goes = Some(goes.clone());
                port.silent.clone_from(&silent);
            }
        }
        ports
    }

    /// Where the notes of a port no channel is linked to go now, as its Library row says it
    /// after the arrow ([`midi::port_channel`]'s rule: the Library's preview, else the
    /// instrument window's channel while it is the selected one, else the selected channel),
    /// and why that channel plays nothing when it cannot.
    fn unlinked_target(&self) -> (String, Option<String>) {
        if let (Some(_), Some(preview)) = (&self.browse.model.playing, self.session.preview()) {
            return (format!("preview: {}", preview.name), None);
        }
        let project = self.session.project();
        match self
            .selected_channel
            .and_then(|id| project.channels.get(&id))
        {
            Some(channel) => (
                format!("selected: {}", channel.name),
                silent_reason(channel),
            ),
            None => ("nothing selected".to_owned(), None),
        }
    }

    /// The ports' lights against the clock, and in a `--midi-monitor` run what arrived and
    /// what the player made of it, printed. Runs when a port sends after a quiet spell and on a
    /// timer while a light is lit; redraws the Library only when a light changed.
    pub(super) fn midi_activity_tick(&mut self) {
        let now = self.midi_activity.now();
        let lit: BTreeSet<String> = self
            .ports
            .last_heard()
            .into_iter()
            .filter(|&(_, stamp)| Activity::lit(stamp, now))
            .map(|(name, _)| name.to_owned())
            .collect();
        if lit != self.lit_ports {
            self.lit_ports = lit;
            self.browse.cache.clear();
        }
        if self.midi_monitor {
            self.print_heard();
            self.read_player_events();
        }
    }

    /// Whether the activity timer runs: while a light is lit and the Library shows it, or for
    /// the monitor of a `--midi-monitor` run.
    pub(super) fn midi_lights_due(&self) -> bool {
        self.midi_monitor || (self.panels.browser && !self.lit_ports.is_empty())
    }

    /// `--midi-monitor`: prints each message the ports' monitors kept since the last call, a
    /// line each: the port, the bytes, what they are and where a note went.
    fn print_heard(&self) {
        for (port, heard) in self.ports.heard() {
            println!("midi-monitor: {}", self.heard_line(port, &heard));
        }
    }

    /// The monitor's line for `heard` from `port`.
    pub(super) fn heard_line(&self, port: &str, heard: &midi::Heard) -> String {
        let bytes: Vec<String> = heard
            .bytes()
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect();
        let project = self.session.project();
        let name = |id: ChannelId| {
            project.channels.get(&id).map_or_else(
                || format!("channel {}", id.0),
                |channel| channel.name.clone(),
            )
        };
        let what = match midi::parse(heard.bytes()) {
            Some(midi::Incoming::Note(note)) => {
                let target = match heard.channel() {
                    Some(id) if id == crate::session::PREVIEW_CHANNEL => "the preview".to_owned(),
                    Some(id) => name(id),
                    None => match self.selected_channel {
                        Some(id) => format!("selected {}", name(id)),
                        None => "nothing selected".to_owned(),
                    },
                };
                format!("{note:?} -> {target}")
            }
            Some(midi::Incoming::Controller(moved)) => format!("{moved:?}"),
            Some(midi::Incoming::Bend(bend)) => format!("pitch bend {bend}"),
            None => "ignored".to_owned(),
        };
        format!("{port} [{}] {what}", bytes.join(" "))
    }

    /// `--midi-monitor`: the player's live notes as it started and ended them, a line each.
    pub(super) fn print_live_notes(&self, events: &[EngineEvent]) {
        if !self.midi_monitor {
            return;
        }
        let project = self.session.project();
        for event in events {
            let (started, channel) = match event {
                EngineEvent::LiveNoteStarted { channel, .. } => (true, *channel),
                EngineEvent::LiveNoteEnded { channel, .. } => (false, *channel),
                _ => continue,
            };
            let name = project
                .channels
                .get(&channel)
                .map_or_else(|| format!("channel {}", channel.0), |c| c.name.clone());
            let what = if started { "started" } else { "ended" };
            println!("midi-monitor: player {what} a note on {name}: {event:?}");
        }
    }

    /// A port was just let in with no channel selected: the first channel that can play notes
    /// is selected, so the port plays something.
    fn select_a_player(&mut self) {
        if self.selected_channel.is_some() {
            return;
        }
        let project = self.session.project();
        let first = project
            .channels
            .iter()
            .find(|(_, channel)| silent_reason(channel).is_none())
            .map(|(&id, _)| id);
        if let Some(id) = first {
            self.select_channel(id);
        }
    }

    /// Connects the ports that play and closes the rest; a closed one lets go of its keys, and
    /// when that cannot be queued every live note ends.
    pub(super) fn connect_ports(&mut self) {
        let router = self.router();
        let inputs = self.settings.midi_inputs.clone();
        if self.ports.connect(|name| inputs.plays(name), &router) {
            let result = self.session.send(Control::LiveNotesOff);
            self.report(result);
        }
        self.route_ports();
    }

    /// A new document has a player of its own: every connection opens again to it.
    pub(super) fn reconnect_ports(&mut self) {
        let router = self.router();
        let inputs = self.settings.midi_inputs.clone();
        self.ports.reconnect(|name| inputs.plays(name), &router);
        self.route_ports();
    }

    /// Each connected port's notes go to the channel the document links it to, else to the
    /// Library's preview while there is one, else to the instrument window's while it is open,
    /// else to the selected one ([`midi::port_channel`]), and on to that channel's MIDI out
    /// unless its instrument hears the port's keys already ([`midi::port_echo`]). Run after
    /// every change to the document, as the window opens and closes, as a preview starts and
    /// ends and as the selection moves, so the connections follow the document's links and
    /// echoes, the preview, the window and the selection.
    pub(super) fn route_ports(&self) {
        let project = self.session.project();
        let window = self.live_target();
        let selected = self.selected_channel;
        self.ports.route(midi::linked_controls(project), |name| {
            let channel = midi::port_channel(project, name, window);
            let out = midi::port_echo(project, name, channel.or(selected));
            let guarded = midi::has_midi_out(project, channel.or(selected));
            (channel, out, guarded)
        });
    }

    /// The listing timer: the ports again, connected as the settings say. The Library lists
    /// them anew when they changed.
    pub(super) fn list_ports(&mut self) {
        if self.ports.list() {
            self.browse.cache.clear();
        }
        self.connect_ports();
        self.list_midi_outputs();
    }

    /// How often the listing timer runs, or `None` when the ports are names given at start.
    pub(super) fn listing_period(&self) -> Option<Duration> {
        self.ports
            .lists_devices()
            .then_some(listing_period_for(self.panels))
    }

    /// Lists the output devices for Settings: the system's in the background, or the devices the
    /// host gives (a scripted run's none) at once.
    pub(super) fn list_outputs(&mut self) -> Task<Message> {
        match &self.host.audio_outputs {
            OutputSource::Given(listed) => {
                self.listed_outputs = Ok(listed.clone());
                Task::none()
            }
            OutputSource::System => {
                let work = || audio::SystemOutputs.outputs();
                Task::perform(blocking(work), |listed| {
                    Message::OutputsListed(
                        listed.unwrap_or_else(|| Err("the devices could not be listed".to_owned())),
                    )
                })
            }
        }
    }

    /// The output device the settings chose, named as it was shown when chosen; none for the
    /// system's default.
    pub(super) fn chosen_output(&self) -> Option<Chosen<'_>> {
        self.settings
            .audio_output()
            .map(|id| Chosen::new(id, self.settings.audio_output_name()))
    }

    /// The chosen output's name as Settings shows it, "the system default" for none.
    fn chosen_output_name(&self) -> String {
        self.chosen_output()
            .map_or("the system default", |chosen| chosen.name)
            .to_owned()
    }

    /// What Settings offers to play through, and why the list is short when it is.
    fn output_choices(&self) -> (Vec<OutputChoice>, Option<String>) {
        let chosen = self.chosen_output();
        match &self.listed_outputs {
            Ok(listed) => audio::output_choices(listed.as_slice(), chosen),
            Err(problem) => {
                let none: &[ListedOutput] = &[];
                let (choices, _) = audio::output_choices(none, chosen);
                (choices, Some(problem.clone()))
            }
        }
    }

    /// What Settings offers new input channels, and why the list is short when it is.
    fn input_choices(&self) -> (Vec<OutputChoice>, Option<String>) {
        let chosen = self
            .settings
            .audio_input()
            .map(|name| Chosen::new(name, None));
        let (names, problem): (Vec<String>, _) = match &self.input_listing.devices {
            Some(Ok(devices)) => (
                devices
                    .iter()
                    .filter(|device| !device.name.is_empty())
                    .map(|device| device.name.clone())
                    .collect(),
                None,
            ),
            Some(Err(problem)) => (Vec::new(), Some(problem.clone())),
            None => (Vec::new(), None),
        };
        let (choices, _) = audio::output_choices(names.as_slice(), chosen);
        (choices, problem)
    }

    /// Chooses input choice `index` of the list Settings shows for new input channels: saved.
    fn choose_input(&mut self, index: usize) {
        let (choices, _) = self.input_choices();
        let Some(choice) = choices.into_iter().nth(index) else {
            return;
        };
        self.settings.set_audio_input(choice.id);
        self.save_settings();
    }

    /// Chooses output choice `index` of the list Settings shows: saved, and played through at
    /// once ([`switch_output`](Self::switch_output)); a player parked without a device tries it
    /// at once. While a take records or the input latency is measured, the stream keeps its
    /// device until the next Play, so neither is cut off halfway.
    fn choose_output(&mut self, index: usize) {
        let (choices, _) = self.output_choices();
        let Some(choice) = choices.into_iter().nth(index) else {
            return;
        };
        let before = (
            self.settings.audio_output().map(str::to_owned),
            self.settings.audio_output_name().map(str::to_owned),
        );
        self.output_refused = None;
        self.settings.choose_audio_output(choice.id, choice.name);
        if !self.recording.recording() && !self.measuring_latency() {
            match self.audio {
                Some(_) => self.switch_output(before),
                None => self.retry_audio(),
            }
        }
        self.save_settings();
    }

    /// The stream opened again on the device the settings chose, when it plays on another: the
    /// player and every plugin in it carried over, the input channels' devices opened again for
    /// it. A device that does not open leaves the sound where it was: the settings go back to
    /// `before` (what they kept and the name), the stream opens on that device again, and why
    /// shows under the list.
    fn switch_output(&mut self, before: (Option<String>, Option<String>)) {
        let chosen = self.settings.audio_output().map(str::to_owned);
        let Some(output) = self.audio.take() else {
            return;
        };
        if output.asked() == chosen.as_deref() {
            self.audio = Some(output);
            return;
        }
        self.audio_note = None;
        let Some(player) = output.stop() else {
            self.audio_problem = Some(Cow::Borrowed(STOPPED_RESPONDING));
            return;
        };
        let rate = self.session.project().sample_rate;
        match self.open_chosen(player, rate) {
            Ok(output) => {
                self.install_output(output);
                self.time_midi_out();
                self.sync_live_inputs(true);
            }
            Err(failure) => {
                self.output_refused = Some(format!(
                    "Not switched to {}: {}",
                    self.chosen_output_name(),
                    failure.error
                ));
                self.settings.choose_audio_output(before.0, before.1);
                match failure.player {
                    Some(player) => self.start_audio(*player, rate),
                    None => self.audio_problem = Some(Cow::Borrowed(STOPPED_RESPONDING)),
                }
            }
        }
    }

    /// Before Play: a stream opened on another device than the one chosen now (one chosen while
    /// a take recorded or the latency was measured) opens again on the chosen one (the player
    /// and every plugin in it carried over); a player parked without one tries it. A device
    /// that cannot open says so in the toolbar, as at start.
    pub(super) fn apply_output_choice(&mut self) {
        let chosen = self.settings.audio_output().map(str::to_owned);
        match &self.audio {
            Some(output) if output.asked() != chosen.as_deref() => {}
            Some(_) => return,
            None => {
                self.retry_audio();
                return;
            }
        }
        let Some(output) = self.audio.take() else {
            return;
        };
        self.audio_note = None;
        match output.stop() {
            Some(player) => {
                let rate = self.session.project().sample_rate;
                self.start_audio(player, rate);
            }
            None => {
                self.audio_problem = Some(Cow::Borrowed(STOPPED_RESPONDING));
            }
        }
    }

    /// The line under the output devices: what plays now, and what plays from the next Play
    /// when another device is chosen (one chosen while a take records or the latency is
    /// measured, or with no stream open).
    fn output_status(&self) -> String {
        let chosen = self.settings.audio_output();
        match &self.audio {
            Some(output) if output.asked() == chosen => output.playing_through(),
            Some(output) => format!(
                "{}; {} from the next Play",
                output.playing_through(),
                self.chosen_output_name()
            ),
            None => format!(
                "No output open; {} from the next Play",
                self.chosen_output_name()
            ),
        }
    }

    pub(super) fn settings_message(&mut self, message: SettingsMessage) {
        let at = match message {
            SettingsMessage::Pressed(at) => at,
            SettingsMessage::KeysPressed(at) => return self.keys_page_pressed(at),
        };
        if let Some(page) = crate::ui::settings_layout::tab_at(at) {
            self.show_settings_page(page);
            return;
        }
        if self.settings_page != SettingsPage::General {
            return;
        }
        if let Some(index) = self.settings_panel().audio_layout().hit(at) {
            self.choose_output(index);
            return;
        }
        if let Some(index) = self.settings_panel().input_layout().hit(at) {
            self.choose_input(index);
            return;
        }
        if let Some(hit) = self.settings_panel().latency_layout().hit(at) {
            let message = match hit {
                Hit::LatencyDown => LatencyMessage::Down,
                Hit::LatencyUp => LatencyMessage::Up,
                Hit::LatencyClear => LatencyMessage::Clear,
                _ => LatencyMessage::Measure,
            };
            self.latency_message(message);
            return;
        }
        match self.settings_layout().hit(at) {
            Some(Hit::AllPorts) => {
                self.settings.midi_inputs.toggle_all();
                self.inputs_changed();
            }
            Some(Hit::Port(index)) => {
                let Some(name) = self.ports.names().get(index).cloned() else {
                    return;
                };
                self.settings.midi_inputs.toggle(&name, self.ports.names());
                self.inputs_changed();
            }
            Some(Hit::OctaveDown) => self.move_octave(-1),
            Some(Hit::OctaveUp) => self.move_octave(1),
            Some(Hit::ShowHints) => {
                let on = !self.shows_hints();
                self.settings.set_show_hints(on);
                if self.hints_flag.is_some() {
                    self.hints_flag = Some(on);
                }
                self.save_settings();
            }
            Some(Hit::Page(page)) => self.show_settings_page(page),
            Some(Hit::Measure | Hit::LatencyDown | Hit::LatencyUp | Hit::LatencyClear) | None => {}
        }
    }

    /// Shows the settings on `page`; a row waiting for its keys gives up.
    pub(crate) fn show_settings_page(&mut self, page: SettingsPage) {
        self.settings_page = page;
        self.rebinding = None;
    }

    /// A press on the Keys page: a layout chosen, the keys reset, or a row that waits for its
    /// new keys (a second press on it gives up).
    fn keys_page_pressed(&mut self, at: iced::Point) {
        match crate::ui::keys_page::hit(at) {
            Some(KeysHit::Layout(layout)) => self.choose_note_layout(layout),
            Some(KeysHit::Reset) => {
                self.rebinding = None;
                self.keys = crate::keys::Keys::default();
                self.keys_changed();
            }
            Some(KeysHit::Action(action)) => {
                self.rebinding = (self.rebinding != Some(action)).then_some(action);
            }
            None => {}
        }
    }

    /// The choice of ports changed: saved (unless the run is scripted), shown in the Library's
    /// ticks and connected.
    pub(super) fn inputs_changed(&mut self) {
        self.browse.cache.clear();
        self.save_settings();
        self.connect_ports();
        let plays = self
            .ports
            .names()
            .iter()
            .any(|name| self.settings.midi_inputs.plays(name));
        if plays {
            self.select_a_player();
        }
    }

    /// Writes the settings, unless the run is scripted; a failure shows as a notice.
    pub(super) fn save_settings(&mut self) {
        if let (true, Some(file)) = (self.host.saves_settings, &self.host.settings_file) {
            if let Err(error) = self.settings.save(file) {
                self.notice = Some(format!("Settings not saved: {error}"));
            }
        }
    }

    pub(super) fn settings_panel(&self) -> SettingsPanel<'_> {
        let (choices, outputs_problem) = self.output_choices();
        let (inputs_offered, inputs_problem) = self.input_choices();
        SettingsPanel {
            ports: self.ports.names(),
            inputs: &self.settings.midi_inputs,
            dropped: self.router().dropped(),
            lost: self.session.lost_live_events(),
            problem: self.ports.problem(),
            octave: super::typing::octave_name(self.typing.base),
            hints: self.shows_hints(),
            outputs: choices,
            playing: self.output_status(),
            resampling: self.audio_note.clone(),
            refused: self.output_refused.clone(),
            outputs_problem,
            inputs_offered,
            inputs_problem,
            latency_devices: self.latency_devices_words(),
            latency: self
                .line_in_latency()
                .map(crate::settings::InputLatency::words),
            measuring: self.measuring_latency(),
        }
    }

    /// The latency section's line naming the devices a take of the line in uses now: the
    /// output by the name Settings shows it by.
    fn latency_devices_words(&self) -> String {
        let (_, input) = self.line_in_devices();
        let output = self
            .chosen_output()
            .map_or("System default", |chosen| chosen.name);
        format!(
            "Line in: {}; output: {output}",
            crate::live_input::device_label(&input)
        )
    }

    fn settings_layout(&self) -> Layout {
        self.settings_panel().layout()
    }
}

/// Why `channel` plays no notes, as a device row's hint says it: an audio channel plays its
/// clips, an input channel its input, an empty sampler has no sound; `None` when it plays them.
pub(super) fn silent_reason(channel: &Channel) -> Option<String> {
    match channel.kind {
        ChannelKind::Audio => Some(format!(
            "{} is an audio channel: it plays its clips, not notes",
            channel.name
        )),
        ChannelKind::Input => Some(format!(
            "{} is an input channel: it plays its audio input, not notes",
            channel.name
        )),
        ChannelKind::Sampler if channel.sample.as_os_str().is_empty() => {
            Some(format!("{} has no sound yet", channel.name))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use dawsome_engine::project::ChannelId;
    use dawsome_engine::realtime::EngineEvent;
    use iced::Point;

    use super::super::tests::app_on;
    use super::*;
    use crate::midi::{MidiPorts, PortSource};
    use crate::settings::MidiInputs;

    const F1: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/f1_four_on_floor.json"
    );

    fn press(app: &mut App, at: Point) {
        let _ = app.update(Message::Settings(SettingsMessage::Pressed(at)));
    }

    // Given names (no device is opened): "All ports" switches every port on and off, a port's
    // own box switches it alone, and the stepper moves the typing keyboard's octave. A scripted
    // run's choice is not written anywhere.
    #[test]
    fn the_panel_switches_ports_and_moves_the_octave() {
        let (mut app, _player) = app_on(Path::new(F1));
        let names = vec!["Port A".to_owned(), "Port B".to_owned()];
        app.ports = MidiPorts::new(PortSource::Given(names));
        let layout = app.settings_layout();
        press(&mut app, layout.checkbox(0).center());
        assert_eq!(app.settings.midi_inputs, MidiInputs::All);
        press(&mut app, layout.checkbox(1).center());
        assert!(!app.settings.midi_inputs.plays("Port A"));
        assert!(app.settings.midi_inputs.plays("Port B"));
        press(&mut app, layout.label(1).center());
        assert!(
            app.settings.midi_inputs.plays("Port A"),
            "the name switches too"
        );
        let base = app.typing.base;
        press(&mut app, layout.octave_up().center());
        assert_eq!(app.typing.base, base + 12);
        press(&mut app, layout.octave_down().center());
        assert_eq!(app.typing.base, base);
        assert_eq!(
            app.listing_period(),
            None,
            "given names are never listed again"
        );
    }

    // The ports are listed every 2 s while a panel that shows them (the settings, the Library)
    // is open, every 10 s otherwise.
    #[test]
    fn the_ports_are_listed_often_while_a_panel_shows_them() {
        let mut panels = Panels::default();
        assert_eq!(listing_period_for(panels), LISTING_CLOSED);
        panels.browser = true;
        assert_eq!(listing_period_for(panels), LISTING_OPEN);
        panels.browser = false;
        panels.settings = true;
        assert_eq!(listing_period_for(panels), LISTING_OPEN);
    }

    // A MIDI note delivered the way a port's connection delivers it plays the selected
    // channel at its velocity, stopped; the counts show once anything is dropped.
    #[test]
    fn a_ports_notes_play_the_selected_channel() {
        let (mut app, mut player) = app_on(Path::new(F1));
        app.router().deliver(&[0x93, 72, 64]);
        player.process(&mut [[0.0; 2]; 64]);
        assert_eq!(
            app.session.take_events(),
            [EngineEvent::LiveNoteStarted {
                channel: ChannelId(1),
                key: 72,
                velocity: 0.5,
                tick: None,
                frame: 0
            }]
        );
        assert_eq!(app.settings_panel().dropped, 0);
        // CC 7: a controller only the window's queue takes (CC 1 and CC 64 the player takes too).
        for _ in 0..600 {
            app.router().deliver(&[0xB0, 7, 1]);
        }
        assert_eq!(app.settings_panel().dropped, 88, "512 wait, 88 dropped");
        assert_eq!(app.settings_layout().notes, 1);
    }

    /// The fake devices of the output tests: "Gone" cannot open, every other name (and the
    /// default) holds the player as a stream would; a sink of [`SINKS`] is shown by its
    /// description.
    fn open_named(
        player: dawsome_engine::realtime::Player,
        name: Option<&str>,
    ) -> Result<crate::audio::Output, crate::audio::StartFailure> {
        match name {
            Some("Gone") => Err(crate::audio::StartFailure {
                error: crate::audio::StartError::DeviceGone {
                    name: "Gone".to_owned(),
                },
                player: Some(Box::new(player)),
            }),
            Some(id) => Ok(match SINKS.iter().find(|(kept, _)| *kept == id) {
                Some((kept, shown)) => crate::audio::Output::holding_as(player, kept, shown),
                None => crate::audio::Output::holding_on(player, id),
            }),
            None => Ok(crate::audio::Output::holding(player)),
        }
    }

    /// Two of the PipeWire host's sinks in audio.rs's capture of a machine's outputs: what the
    /// settings keep for each and its description.
    const SINKS: [(&str, &str); 2] = [
        (
            "sink:alsa_output.usb-Generic_USB_Audio-00.HiFi__Speaker__sink",
            "USB Audio Speakers",
        ),
        (
            "sink:alsa_output.usb-Generic_USB_Audio-00.HiFi__Headphones__sink",
            "USB Audio Front Headphones",
        ),
    ];

    /// A window on F1 whose outputs are `listed`, opened by [`open_named`], its settings saved
    /// in a temporary folder (removed by the caller), with Settings open.
    fn app_with_outputs(test: &str, listed: Vec<ListedOutput>) -> (App, std::path::PathBuf) {
        use crate::document::{AudioChoice, Host, OutputSource};
        let dir = std::env::temp_dir().join(format!("dawsome-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let host = Host {
            audio: AudioChoice::FakeNamed(open_named),
            audio_outputs: OutputSource::Given(listed),
            settings_file: Some(dir.join("dawsome/settings.json")),
            saves_settings: true,
            ..Host::offline()
        };
        let project = crate::project_file::open(Path::new(F1))
            .expect("opens")
            .project;
        let (mut app, player) = super::super::tests::app_hosted(project, Path::new(F1), host);
        app.audio = Some(crate::audio::Output::holding(player));
        let _ = app.update(Message::TogglePanel(crate::ui::panels::Panel::Settings));
        (app, dir)
    }

    fn asked(app: &App) -> Option<Option<String>> {
        app.audio
            .as_ref()
            .map(|output| output.asked().map(str::to_owned))
    }

    fn press_output(app: &mut App, row: usize) {
        let at = app.settings_panel().audio_layout().row(row).center();
        press(app, at);
    }

    // B8, 2026-10-27 (a device picked showed chosen, and "Playing through:" still named the one
    // before until the next Play): Settings lists "System default" first and the
    // devices the host lists; a press on one saves it and plays through it at once, the line
    // naming it. A device that does not open leaves the sound on the one before: the choice
    // goes back to it (saved so too), and why shows under the list until the next choice. With
    // no stream open (the player parked), a choice is tried at once; Play still opens a device
    // chosen while none could.
    #[test]
    fn an_output_chosen_in_settings_plays_at_once() {
        let listed = ["Speakers", "USB Audio", "Gone"].map(ListedOutput::named);
        let (mut app, dir) = app_with_outputs("outputs", listed.to_vec());
        let file = dir.join("dawsome/settings.json");
        let panel = app.settings_panel();
        let words: Vec<String> = panel
            .outputs
            .iter()
            .map(crate::ui::settings_panel::output_words)
            .collect();
        assert_eq!(words, ["System default", "Speakers", "USB Audio", "Gone"]);
        assert_eq!(
            panel.playing,
            "Playing through: Test output, 48000 Hz, 512 frames"
        );
        press_output(&mut app, 2);
        assert_eq!(app.settings.audio_output(), Some("USB Audio"));
        let saved = crate::settings::Settings::load(Some(&file));
        assert_eq!(saved.audio_output(), Some("USB Audio"));
        assert_eq!(asked(&app), Some(Some("USB Audio".to_owned())), "at once");
        assert_eq!(
            app.settings_panel().playing,
            "Playing through: USB Audio, 48000 Hz, 512 frames"
        );
        assert!(app.settings_panel().outputs[2].chosen);
        press_output(&mut app, 3);
        assert_eq!(
            asked(&app),
            Some(Some("USB Audio".to_owned())),
            "still there"
        );
        assert_eq!(app.settings.audio_output(), Some("USB Audio"));
        let saved = crate::settings::Settings::load(Some(&file));
        assert_eq!(saved.audio_output(), Some("USB Audio"));
        let panel = app.settings_panel();
        assert_eq!(
            panel.refused.as_deref(),
            Some(
                "Not switched to Gone: the output device Gone is not there; choose another in \
                 Settings"
            )
        );
        assert_eq!(
            panel.playing,
            "Playing through: USB Audio, 48000 Hz, 512 frames"
        );
        assert!(panel.outputs[2].chosen && !panel.outputs[3].chosen);
        assert_eq!(app.audio_problem, None);
        assert!(!app.session.is_parked(), "nothing went silent");
        press_output(&mut app, 0);
        assert_eq!(asked(&app), Some(None));
        assert_eq!(app.output_refused, None);
        assert_eq!(app.settings.audio_output(), None);
        app.settings.set_audio_output(Some("Gone".to_owned()));
        let _ = app.update(Message::Play);
        assert!(app.audio.is_none());
        assert_eq!(
            app.audio_problem.as_deref(),
            Some("Audio off: the output device Gone is not there; choose another in Settings")
        );
        assert!(app.session.is_parked(), "the player waits for a device");
        assert_eq!(
            app.settings_panel().playing,
            "No output open; Gone from the next Play"
        );
        let _ = app.update(Message::Stop);
        press_output(&mut app, 1);
        assert_eq!(
            asked(&app),
            Some(Some("Speakers".to_owned())),
            "tried at once"
        );
        assert_eq!(app.audio_problem, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // The case reported: the stream plays through the interface's Speakers sink, and a press on
    // its Front Headphones sink plays through that at once: kept by its node, shown by its
    // description on the row and the line, the description saved beside the node.
    #[test]
    fn a_sink_chosen_plays_at_once_kept_by_its_node_and_shown_by_its_description() {
        let listed = SINKS.map(|(id, name)| ListedOutput {
            id: id.to_owned(),
            name: name.to_owned(),
        });
        let (mut app, dir) = app_with_outputs("sinks", listed.to_vec());
        let [(speakers, speakers_name), (headphones, headphones_name)] = SINKS;
        app.settings
            .choose_audio_output(Some(speakers.to_owned()), Some(speakers_name.to_owned()));
        let player = app
            .audio
            .take()
            .and_then(crate::audio::Output::stop)
            .expect("the player");
        app.audio = Some(crate::audio::Output::holding_as(
            player,
            speakers,
            speakers_name,
        ));
        let panel = app.settings_panel();
        assert_eq!(
            panel.playing,
            "Playing through: USB Audio Speakers, 48000 Hz, 512 frames"
        );
        assert!(panel.outputs[1].chosen);
        press_output(&mut app, 2);
        assert_eq!(asked(&app), Some(Some(headphones.to_owned())));
        let panel = app.settings_panel();
        assert_eq!(
            panel.playing,
            "Playing through: USB Audio Front Headphones, 48000 Hz, 512 frames"
        );
        assert!(!panel.outputs[1].chosen && panel.outputs[2].chosen);
        let saved = crate::settings::Settings::load(Some(&dir.join("dawsome/settings.json")));
        assert_eq!(
            (saved.audio_output(), saved.audio_output_name()),
            (Some(headphones), Some(headphones_name))
        );
        assert_eq!(
            app.settings_panel().latency_devices,
            "Line in: System default input; output: USB Audio Front Headphones"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A choice made while a take records waits for the next Play, so the take is not cut off:
    // the line says what plays and what plays from the next Play; Stop, then Play, opens it.
    #[test]
    fn an_output_chosen_while_a_take_records_plays_from_the_next_play() {
        use crate::app::recording::RecordMessage;
        let listed = ["Speakers", "USB Audio"].map(ListedOutput::named);
        let (mut app, dir) = app_with_outputs("outputs-take", listed.to_vec());
        let _ = app.update(Message::Record(RecordMessage::ToggleArm));
        let _ = app.update(Message::Play);
        assert!(app.recording.recording(), "a take records");
        press_output(&mut app, 2);
        assert_eq!(app.settings.audio_output(), Some("USB Audio"));
        assert_eq!(asked(&app), Some(None), "the take keeps its device");
        assert_eq!(
            app.settings_panel().playing,
            "Playing through: Test output, 48000 Hz, 512 frames; USB Audio from the next Play"
        );
        let _ = app.update(Message::Stop);
        let _ = app.update(Message::Record(RecordMessage::ToggleArm));
        let _ = app.update(Message::Play);
        assert_eq!(asked(&app), Some(Some("USB Audio".to_owned())));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Settings lists the input devices under the outputs, "System default input" first; a press
    // on one saves it by name, and the Library's External input then adds a channel playing it
    // (its first pair, stereo). Given devices: nothing is opened for real.
    #[test]
    fn the_input_chosen_in_settings_is_what_a_new_input_channel_plays() {
        use crate::document::Host;
        use crate::live_input::{DeviceInfo, InputDevices};
        use crate::ui::browser::{BrowserMessage, Pick};
        use dawsome_engine::project::{InputChannels, InputSource};
        let dir = std::env::temp_dir().join(format!("dawsome-inputs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("dawsome/settings.json");
        let line = DeviceInfo {
            name: "USB Audio Line Input".to_owned(),
            channels: 2,
            rates: Vec::new(),
        };
        let host = Host {
            audio_inputs: InputDevices::Given(vec![line]),
            settings_file: Some(file.clone()),
            saves_settings: true,
            ..Host::offline()
        };
        let project = crate::project_file::open(Path::new(F1))
            .expect("opens")
            .project;
        let (mut app, _player) = super::super::tests::app_hosted(project, Path::new(F1), host);
        let _ = app.update(Message::TogglePanel(crate::ui::panels::Panel::Settings));
        let panel = app.settings_panel();
        let words: Vec<String> = panel
            .inputs_offered
            .iter()
            .map(crate::ui::settings_panel::input_words)
            .collect();
        assert_eq!(words, ["System default input", "USB Audio Line Input"]);
        assert!(
            panel.input_layout().title().y > panel.audio_layout().line(0).y,
            "under the outputs"
        );
        let line_row = panel.input_layout().row(1).center();
        press(&mut app, line_row);
        assert_eq!(app.settings.audio_input(), Some("USB Audio Line Input"));
        let saved = crate::settings::Settings::load(Some(&file));
        assert_eq!(saved.audio_input(), Some("USB Audio Line Input"));
        let id = app.session.project().next_channel_id;
        let pick = BrowserMessage::Picked {
            pick: Pick::ExternalInput,
            button: iced::mouse::Button::Left,
            at: Point::ORIGIN,
        };
        let _ = app.update(Message::Browser(pick.clone()));
        let _ = app.update(Message::Browser(BrowserMessage::Released));
        let _ = app.update(Message::Browser(pick));
        let channel = &app.session.project().channels[&id];
        assert_eq!(channel.name, "External input");
        assert_eq!(
            channel.input,
            Some(InputSource {
                device: "USB Audio Line Input".to_owned(),
                channels: InputChannels::Stereo(0, 1),
            })
        );
        assert_eq!(app.selected_channel, Some(id));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
