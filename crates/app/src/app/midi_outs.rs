//! The input channels' MIDI outs in the window: the MIDI OUT group of an input channel's
//! instrument window (its port, MIDI channel, program, latency and echo, each change one undo
//! step), and the MIDI-out thread following the document (each channel's route, told after every
//! change), the session (each new document's player ring), the output stream (its clock) and the
//! window's end (quit ends every route's notes).

use dawsome_engine::edit::Edit;
use dawsome_engine::project::{stable_port_name, ChannelId, ChannelKind, MidiOut};

use crate::audio::Output;
use crate::document::Host;
use crate::midi_out::{self, Counts, MidiOutThread, MidiSink};
use crate::ui::instrument_window::{MidiOutChange, MidiOutView, PortChoice};
use crate::ui::library::is_loopback;

use super::App;

/// The words of the Port picker's first choice, no MIDI out.
const NO_PORT: &str = "None";

/// What the MIDI-out thread of a window with `host` sends into: the system's outputs, the
/// names given's recording, or a test's own recording.
pub(super) fn sink_for(host: &Host) -> Box<dyn MidiSink> {
    #[cfg(test)]
    if let Some(recording) = &host.midi_sent {
        return Box::new(recording.clone());
    }
    host.midi_outputs.sink()
}

/// The MIDI-out thread of a window with `host`, or why it could not start.
pub(super) fn start(host: &Host) -> Result<MidiOutThread, String> {
    MidiOutThread::start(sink_for(host))
}

impl App {
    /// The session is new (the window opened, or a document replaced the last): the thread reads
    /// its player's ring, ends the last document's routes and takes this one's, programs and
    /// all, timed by the stream playing now.
    pub(super) fn open_midi_out(&mut self) {
        let routes = midi_out::routes(self.session.project());
        if let Some(thread) = &self.midi_out {
            thread.open(self.session.take_midi_out(), routes.clone());
            thread.clock(self.audio.as_ref().map(Output::playback_clock));
        }
        self.midi_routes = routes;
    }

    /// After a change to the document: the thread is told the channels' routes when they
    /// changed (a channel's MIDI out set, changed or gone, the channel deleted), which ends the
    /// notes of a route that moved and sets a program set anew.
    pub(super) fn route_midi_out(&mut self) {
        let routes = midi_out::routes(self.session.project());
        if routes == self.midi_routes {
            return;
        }
        if let Some(thread) = &self.midi_out {
            thread.routes(routes.clone());
        }
        self.midi_routes = routes;
    }

    /// The output stream changed: messages are timed by its clock from now on (none while no
    /// stream plays: each goes at once).
    pub(super) fn time_midi_out(&self) {
        if let Some(thread) = &self.midi_out {
            thread.clock(self.audio.as_ref().map(Output::playback_clock));
        }
    }

    /// The window ends: every route's notes end, and the thread with them.
    pub(super) fn quit_midi_out(&mut self) {
        if let Some(thread) = &mut self.midi_out {
            thread.quit();
        }
    }

    /// Lists the MIDI output ports again, for the Port picker.
    pub(super) fn list_midi_outputs(&mut self) {
        self.midi_output_names = self.host.midi_outputs.names();
    }

    /// What input channel `channel`'s window shows of its MIDI out: the ports listed (None first,
    /// the system's loopback last, its own port "not plugged in" when it is not listed), its
    /// settings, and what the MIDI-out thread counted. `None` for any other kind of channel.
    pub(super) fn midi_out_view(&self, channel: ChannelId) -> Option<MidiOutView> {
        let shown = self.session.project().channels.get(&channel)?;
        if shown.kind != ChannelKind::Input {
            return None;
        }
        let none = PortChoice {
            name: None,
            words: NO_PORT.to_owned(),
        };
        let listed = self.midi_output_names.as_deref().unwrap_or_default();
        let (loopbacks, devices): (Vec<&String>, Vec<&String>) =
            listed.iter().partition(|name| is_loopback(name));
        let mut ports = vec![none.clone()];
        ports.extend(devices.into_iter().chain(loopbacks).map(|name| PortChoice {
            name: Some(name.clone()),
            words: if is_loopback(name) {
                format!("{name} (loopback, no device)")
            } else {
                name.clone()
            },
        }));
        let port = match &shown.midi_out {
            None => none,
            Some(out) => {
                let chosen = ports.iter().find(|choice| {
                    choice
                        .name
                        .as_deref()
                        .is_some_and(|name| stable_port_name(name) == stable_port_name(&out.port))
                });
                match chosen {
                    Some(choice) => choice.clone(),
                    None => {
                        let missing = PortChoice {
                            name: Some(out.port.clone()),
                            words: format!("{} (not plugged in)", out.port),
                        };
                        ports.push(missing.clone());
                        missing
                    }
                }
            }
        };
        let counts = match (&self.midi_output_names, &self.midi_out) {
            (Err(problem), _) => Some((problem.clone(), true)),
            (Ok(_), Some(thread)) => counts_words(thread.counts()),
            (Ok(_), None) => None,
        };
        Some(MidiOutView {
            ports,
            port,
            out: shown.midi_out.clone(),
            counts,
        })
    }

    /// A change in input channel `channel`'s MIDI OUT group: one `SetChannelMidiOut` edit, the
    /// port stored by its stable name (a new MIDI out on MIDI channel 1, no program, no latency,
    /// echo off); a latency drag is one undo step on release. Settings other than the port wait
    /// for one.
    pub(super) fn midi_out_message(&mut self, channel: ChannelId, change: MidiOutChange) {
        let Some(shown) = self.session.project().channels.get(&channel) else {
            return;
        };
        let before = shown.midi_out.clone();
        let after = match (change, before.clone()) {
            (MidiOutChange::Port(None), _) => None,
            (MidiOutChange::Port(Some(name)), out) => {
                let port = stable_port_name(&name).to_owned();
                Some(match out {
                    Some(out) => MidiOut { port, ..out },
                    None => MidiOut {
                        port,
                        channel: 1,
                        program: None,
                        latency_ms: 0,
                        echo: false,
                    },
                })
            }
            (_, None) => return,
            (MidiOutChange::Channel(number), Some(out)) => Some(MidiOut {
                channel: number,
                ..out
            }),
            (MidiOutChange::Program(program), Some(out)) => Some(MidiOut { program, ..out }),
            (MidiOutChange::Echo(echo), Some(out)) => Some(MidiOut { echo, ..out }),
            (MidiOutChange::Latency(latency_ms), Some(out)) => {
                if latency_ms != out.latency_ms {
                    let out = Some(MidiOut { latency_ms, ..out });
                    self.drag_edit(Edit::SetChannelMidiOut { channel, out });
                }
                return;
            }
        };
        if after != before {
            self.commit(Edit::SetChannelMidiOut {
                channel,
                out: after,
            });
        }
    }
}

/// The MIDI OUT group's line for what the MIDI-out thread counted, when it counted anything,
/// and whether that is a warning (a message refused or not taken by its port).
fn counts_words(counts: &Counts) -> Option<(String, bool)> {
    let (late, refused, failed) = (counts.late(), counts.refused(), counts.failed());
    let parts: Vec<String> = [
        (late, "sent late"),
        (refused, "refused"),
        (failed, "not taken by the port"),
    ]
    .into_iter()
    .filter(|&(count, _)| count > 0)
    .map(|(count, words)| format!("{count} {words}"))
    .collect();
    (!parts.is_empty()).then(|| {
        (
            format!("MIDI out: {}", parts.join(", ")),
            refused + failed > 0,
        )
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::super::tests::app_hosted;
    use super::super::Message;
    use super::*;
    use crate::midi_out::{MidiOutputs, Recording};

    const SYNTH: &str = "Synth:Synth MIDI 1";
    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    /// The window on `fixture` with given MIDI outputs, whose thread sends into the recording
    /// it returns.
    fn app_recording(fixture: &str) -> (App, dawsome_engine::realtime::Player, Recording) {
        let path = Path::new(FIXTURES).join(fixture);
        let project = crate::project_file::open(&path).expect("opens").project;
        let recording = Recording::default();
        let host = Host {
            midi_outputs: MidiOutputs::Given(vec![SYNTH.to_owned()]),
            midi_sent: Some(recording.clone()),
            ..Host::offline()
        };
        let (app, player) = app_hosted(project, &path, host);
        (app, player, recording)
    }

    fn sent(app: &App, recording: &Recording) -> Vec<(String, String)> {
        if let Some(thread) = &app.midi_out {
            thread.sync();
        }
        recording.take()
    }

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(port, bytes)| ((*port).to_owned(), (*bytes).to_owned()))
            .collect()
    }

    fn set_out(app: &mut App, channel: u32, out: Option<MidiOut>) {
        app.commit(Edit::SetChannelMidiOut {
            channel: ChannelId(channel),
            out,
        });
    }

    fn out_of(app: &App, channel: u32) -> MidiOut {
        let channels = &app.session.project().channels;
        channels[&ChannelId(channel)]
            .midi_out
            .clone()
            .expect("a MIDI out")
    }

    // p19_rack: "Synth A" (1) on MIDI channel 1, "Synth B" (2) on 3 and "Synth muted" (3) on 4,
    // all to the Synth's port, no program. Each trigger ends the notes of the route it moves or
    // takes away (CC 123, then CC 120, on the old MIDI channel): a MIDI channel changed, a port
    // changed, the MIDI out taken away, the channel deleted, File → New, the window closed.
    // Settings that leave the route where it is (latency, echo) send nothing.
    #[test]
    fn every_route_that_moves_or_goes_ends_its_notes() {
        let (mut app, _player, recording) = app_recording("p19_rack.json");
        assert!(sent(&app, &recording).is_empty(), "no program to set");
        let moved = MidiOut {
            channel: 2,
            ..out_of(&app, 1)
        };
        set_out(&mut app, 1, Some(moved));
        let off = |port: &str, channel: u8| {
            pairs(&[
                (port, &format!("B{channel:X} 7B 00")),
                (port, &format!("B{channel:X} 78 00")),
            ])
        };
        assert_eq!(sent(&app, &recording), off(SYNTH, 0), "MIDI channel 1 to 2");
        let slower = MidiOut {
            latency_ms: 40,
            echo: true,
            ..out_of(&app, 1)
        };
        set_out(&mut app, 1, Some(slower));
        assert!(sent(&app, &recording).is_empty(), "latency and echo");
        let elsewhere = MidiOut {
            port: "Other:Other MIDI 1".to_owned(),
            ..out_of(&app, 2)
        };
        set_out(&mut app, 2, Some(elsewhere));
        assert_eq!(sent(&app, &recording), off(SYNTH, 2), "another port");
        set_out(&mut app, 2, None);
        assert_eq!(
            sent(&app, &recording),
            off("Other:Other MIDI 1", 2),
            "the port set to none"
        );
        app.commit(Edit::DeleteChannel { id: ChannelId(3) });
        assert_eq!(sent(&app, &recording), off(SYNTH, 3), "the channel deleted");
        let _ = app.update(Message::Undo);
        assert!(sent(&app, &recording).is_empty(), "the channel back");
        let opened = crate::project_file::Opened {
            project: crate::project_file::new_document(),
            samples_dir: std::path::PathBuf::new(),
        };
        assert!(app.install(opened, crate::document::Document::untitled()));
        let mut left = off(SYNTH, 1);
        left.extend(off(SYNTH, 3));
        assert_eq!(sent(&app, &recording), left, "File → New");
        let (mut app, _player, recording) = app_recording("p19_rack.json");
        let _ = app.update(Message::File(
            super::super::files::FileMessage::MainWindowClosed,
        ));
        let mut every = off(SYNTH, 0);
        every.extend(off(SYNTH, 2));
        every.extend(off(SYNTH, 3));
        assert_eq!(recording.take(), every, "the window closed");
    }

    fn change(app: &mut App, change: MidiOutChange) {
        let _ = app.update(Message::Instrument(
            crate::ui::instrument_window::InstrumentMessage::MidiOut(change),
        ));
    }

    // p19_echo's "Synth" (1) in its instrument window, outputs "Synth:Synth MIDI 1" and the
    // loopback given: the group lists None, the Synth, the loopback last, and picks the Synth by
    // its stable name. Each change is one undo step named "Change MIDI out", and goes out as its
    // route says (the program at once, all notes off where the MIDI channel was); a latency drag
    // is one undo step on release; the port set to None takes the MIDI out away, and a port
    // picked again starts at the defaults. The Library's Synth row reads "→ out: Synth".
    #[test]
    fn the_midi_out_group_changes_the_midi_out_one_undo_step_each() {
        use crate::ui::instrument_window::InstrumentMessage;
        let path = Path::new(FIXTURES).join("p19_echo.json");
        let project = crate::project_file::open(&path).expect("opens").project;
        let recording = Recording::default();
        let host = Host {
            midi_outputs: MidiOutputs::Given(vec![
                "Midi Through:Midi Through Port-0 14:0".to_owned(),
                "Synth:Synth MIDI 1 24:0".to_owned(),
            ]),
            midi_sent: Some(recording.clone()),
            ..Host::offline()
        };
        let (mut app, _player) = app_hosted(project, &path, host);
        app.open_instrument_chosen(ChannelId(1));
        let view = app.midi_out_view(ChannelId(1)).expect("an input channel");
        let words: Vec<&str> = view.ports.iter().map(|port| port.words.as_str()).collect();
        assert_eq!(
            words,
            [
                "None",
                "Synth:Synth MIDI 1 24:0",
                "Midi Through:Midi Through Port-0 14:0 (loopback, no device)"
            ]
        );
        assert_eq!(view.port.words, "Synth:Synth MIDI 1 24:0");
        assert_eq!(view.out.as_ref().map(|out| out.channel), Some(1));
        let steps = app.session.history().undo_len();
        change(&mut app, MidiOutChange::Program(Some(7)));
        change(&mut app, MidiOutChange::Channel(4));
        change(&mut app, MidiOutChange::Echo(true));
        assert_eq!(
            sent(&app, &recording),
            pairs(&[
                (SYNTH, "C0 07"),
                (SYNTH, "B0 7B 00"),
                (SYNTH, "B0 78 00"),
                (SYNTH, "C3 07")
            ])
        );
        for ms in [10, 20, 32] {
            change(&mut app, MidiOutChange::Latency(ms));
        }
        let _ = app.update(Message::Instrument(InstrumentMessage::Released));
        let out = out_of(&app, 1);
        assert_eq!(
            (out.program, out.channel, out.echo, out.latency_ms),
            (Some(7), 4, true, 32)
        );
        assert_eq!(
            app.session.history().undo_len(),
            steps + 4,
            "the drag is one step"
        );
        assert_eq!(app.session.history().undo_name(), Some("Change MIDI out"));
        let _ = app.update(Message::Undo);
        assert_eq!(out_of(&app, 1).latency_ms, 0, "the whole drag undone");
        let rows = app.library_ports(&[]);
        let synth = rows
            .iter()
            .find(|row| row.name == "Synth:Synth MIDI 1 24:0");
        assert!(
            synth.is_none(),
            "an output only: no input row says it (risk 10)"
        );
        app.ports = crate::midi::MidiPorts::new(crate::midi::PortSource::Given(vec![
            "Synth:Synth MIDI 1 24:0".to_owned(),
        ]));
        let rows = app.library_ports(&[]);
        assert_eq!(rows[0].arrow().as_deref(), Some("\u{2192} out: Synth"));
        change(&mut app, MidiOutChange::Port(None));
        assert_eq!(app.session.project().channels[&ChannelId(1)].midi_out, None);
        let view = app.midi_out_view(ChannelId(1)).expect("an input channel");
        assert_eq!((view.port.words.as_str(), view.out), ("None", None));
        change(&mut app, MidiOutChange::Channel(9));
        assert_eq!(
            app.session.project().channels[&ChannelId(1)].midi_out,
            None,
            "no port, no settings"
        );
        change(
            &mut app,
            MidiOutChange::Port(Some("Synth:Synth MIDI 1 24:0".to_owned())),
        );
        let out = out_of(&app, 1);
        assert_eq!(
            (
                out.port.as_str(),
                out.channel,
                out.program,
                out.latency_ms,
                out.echo
            ),
            (SYNTH, 1, None, 0, false),
            "stored by its stable name, at the defaults"
        );
        assert!(app.midi_out_view(ChannelId(2)).is_some());
    }

    // A program is sent once when the document opens and once each time it is set to another,
    // never on Play or Stop, nor when another setting of its MIDI out changes.
    #[test]
    fn a_program_goes_out_when_it_is_set_and_never_on_play() {
        let (mut app, _player, recording) = app_recording("p19_rack.json");
        let programmed = MidiOut {
            program: Some(41),
            ..out_of(&app, 2)
        };
        set_out(&mut app, 2, Some(programmed));
        assert_eq!(sent(&app, &recording), pairs(&[(SYNTH, "C2 29")]));
        let _ = app.update(Message::Play);
        let _ = app.update(Message::Stop);
        let _ = app.update(Message::Play);
        let slower = MidiOut {
            latency_ms: 12,
            ..out_of(&app, 2)
        };
        set_out(&mut app, 2, Some(slower));
        assert!(sent(&app, &recording).is_empty(), "Play, Stop and latency");
        let path = Path::new(FIXTURES).join("p19_rack.json");
        let mut project = crate::project_file::open(&path).expect("opens").project;
        let channel = project.channels.get_mut(&ChannelId(1)).expect("Synth A");
        if let Some(out) = &mut channel.midi_out {
            out.program = Some(0);
        }
        let opened = crate::project_file::Opened {
            project,
            samples_dir: std::path::PathBuf::new(),
        };
        assert!(app.install(opened, crate::document::Document::untitled()));
        let mut expected = pairs(&[(SYNTH, "B0 7B 00"), (SYNTH, "B0 78 00")]);
        expected.extend(pairs(&[(SYNTH, "B2 7B 00"), (SYNTH, "B2 78 00")]));
        expected.extend(pairs(&[(SYNTH, "B3 7B 00"), (SYNTH, "B3 78 00")]));
        expected.extend(pairs(&[(SYNTH, "C0 00")]));
        assert_eq!(
            sent(&app, &recording),
            expected,
            "a document opened: the last one's routes end, its program is set"
        );
        let _ = app.update(Message::Play);
        assert!(sent(&app, &recording).is_empty(), "never on Play");
    }
}
