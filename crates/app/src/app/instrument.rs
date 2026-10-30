//! The instrument window's life in the app: which channel it shows, the set's zones read from
//! disk for its map, the key held on the map, the folded zone list, the settings stored for
//! Compare and the drag under way. Its edits are the document's own (sound settings, volume,
//! pan), each slider drag one undo step; a press on its map plays the channel through the live
//! path, the window's channel selected while it is open and the one selected before given back
//! when it closes. It follows every selection the app makes (a click on a name, an add, a
//! sound replaced, the picker, the channel an undo falls back to), so while it is open the
//! typing keyboard and the MIDI ports no link routes play its channel, the selected one.
//!
//! It is a window of its own beside the main one (iced 0.14 multi-window: the app runs as an
//! `iced::daemon`, `window::open` gives this window its id, `view` and `title` take the id).
//! The state here is what it shows; [`App::sync_windows`] opens and closes the
//! window to match after every message, so every path that opens or closes the state (its
//! channel's menu, the Library, Esc, its close button, an undo taking its channel, a new
//! document) opens or closes the window too. Its size and position are kept in the settings
//! when it closes.

use dawsome_engine::edit::Edit;
use dawsome_engine::project::{
    Channel, ChannelId, ChannelKind, Gate, ProcessorOwner, Sound, SoundSource,
};
use dawsome_engine::transport::Control as Transport;
use iced::widget::{container, space};
use iced::{Element, Fill, Task};

use super::{App, Message, WindowChange};
use crate::desktop::{self, PopOut};
use crate::sets::{self, ZoneInfo};
use crate::ui::hint::{self, Hover};
use crate::ui::instrument_window::{self as window, Control, Flag, InstrumentMessage, Map};
use crate::ui::style;

/// The window while it is open.
#[derive(Debug)]
pub(super) struct Instrument {
    channel: ChannelId,
    /// The channel's sound as the window was opened on it: its map and zones are this one's.
    source: SoundSource,
    zones: Vec<ZoneInfo>,
    /// What the channel's sound does at a note's end with no gate of its own: a sampler's
    /// one-shot, a set's by its zones (none for a set of one-shot and held zones both).
    own_gate: Option<Gate>,
    map: Map,
    maker: String,
    /// The key held on the map and its velocity.
    held: Option<(u8, u8)>,
    more: bool,
    /// The settings Store A kept (A), while it holds some.
    compared: Option<Sound>,
    /// The settings A/B switched away from last to play A (B).
    other: Option<Sound>,
    /// Undoes the slider drag under way: the inverse of its first change.
    drag: Option<Edit>,
    /// The channel selected before the window opened, given back when it closes.
    previous: Option<ChannelId>,
}

impl Instrument {
    /// The key pressed on the map is held no more: the player ended it (Stop).
    pub(super) fn forget_held(&mut self) {
        self.held = None;
    }
}

/// The level of the app's pop-out windows: above every window while the app has the keyboard
/// (`app_focused`), so a click on the main window never hides the instrument window behind it;
/// as any window once another program has it, so it does not float over that program. Why not
/// always on top: an always-on-top window covers every other program's windows too. (The
/// window system decides: Windows and X11 keep the level; a Wayland compositor gives an app no
/// say in stacking, and there the level is ignored, which is why the app's windows open on X11
/// under a Wayland session unless `DAWSOME_WINDOW_SYSTEM=wayland`: `desktop::window_system`.)
pub fn pop_out_level(app_focused: bool) -> iced::window::Level {
    if app_focused {
        iced::window::Level::AlwaysOnTop
    } else {
        iced::window::Level::Normal
    }
}

impl App {
    /// Opens the instrument window on `channel` (a second one replaces the first), selecting it
    /// so the window's keys play it, and routing the ports no link routes to it.
    pub(super) fn open_instrument(&mut self, channel: ChannelId) {
        let previous = match self.instrument.take() {
            Some(open) => {
                self.end_map_note(&open);
                open.previous
            }
            None => self.selected_channel,
        };
        let Some(shown) = self.session.project().channels.get(&channel) else {
            // The window it replaced is closed: its channel no longer takes the ports' notes.
            self.route_ports();
            return;
        };
        let zones = self.zones_of(shown);
        let own_gate = match shown.kind {
            ChannelKind::Instrument => sets::own_gate(
                &self.set_file(shown),
                &self.browse.model.user_folders().folders,
            ),
            kind => Some(Gate::of(kind)),
        };
        let maker = self.maker_of(shown);
        let drum = dawsome_media::paths::shipped_part(&shown.sample)
            .is_some_and(|part| part.starts_with("starter-kit/"));
        let map = window::map_of(shown.kind, &shown.name, &zones, drum);
        self.instrument = Some(Instrument {
            channel,
            source: shown.source(),
            zones,
            own_gate,
            map,
            maker,
            held: None,
            more: false,
            compared: None,
            other: None,
            drag: None,
            previous,
        });
        // The Library's preview ends whatever is selected: selecting ends it too, but the
        // channel may be the one selected already, and the keys must play the window's.
        let ended = self.end_library_preview();
        self.report(ended);
        if self.selected_channel != Some(channel) {
            self.select_channel(channel);
        }
        self.route_ports();
    }

    /// Opens the window on `channel` as a click on its rack name does: a window open on it
    /// already stays as it is (its stored A and its zone list too), another retargets to it.
    /// Why the selection it gives back on closing is `channel`: a click on a name chooses that
    /// channel, so closing the window leaves it chosen, rather than going back to the one
    /// selected before some earlier opening.
    pub(super) fn open_instrument_chosen(&mut self, channel: ChannelId) {
        if self.instrument_on() == Some(channel) {
            return;
        }
        self.open_instrument(channel);
        if let Some(open) = &mut self.instrument {
            open.previous = Some(channel);
        }
    }

    /// Closes the window: the key held on its map ends, a slider drag under way becomes its
    /// undo step (unless its channel is gone, which takes the drag with it), and the selection
    /// goes back to the channel selected before it opened when that channel is still there.
    pub(super) fn close_instrument(&mut self) -> bool {
        let Some(open) = self.instrument.take() else {
            return false;
        };
        self.route_ports();
        self.end_map_note(&open);
        if self.session.project().channels.contains_key(&open.channel) {
            self.finish_sound_drag(open.drag);
        }
        let previous = open
            .previous
            .filter(|&id| self.session.project().channels.contains_key(&id));
        if let Some(previous) = previous.filter(|&id| Some(id) != self.selected_channel) {
            // Given back, not chosen: the Library's preview, when there is one, plays on.
            let result = self.change_selected_channel(Some(previous));
            self.report(result);
        }
        true
    }

    /// The window closes, as Esc closes it, when its channel is gone (an undo took it away),
    /// and shows its channel's sound again when that was replaced (a sound let go of on its
    /// header, or the undo of one): its map, zones and gate are the new sound's.
    pub(super) fn keep_instrument(&mut self) {
        let Some(open) = &self.instrument else {
            return;
        };
        let (channel, shown) = (
            open.channel,
            self.session.project().channels.get(&open.channel),
        );
        match shown.map(|shown| shown.source() != open.source) {
            None => {
                self.close_instrument();
            }
            Some(true) => self.open_instrument(channel),
            Some(false) => {}
        }
    }

    /// An instrument set's zones from its file, read as the Library reads sets; none for any
    /// other channel.
    fn zones_of(&self, channel: &Channel) -> Vec<ZoneInfo> {
        if channel.kind != ChannelKind::Instrument {
            return Vec::new();
        }
        sets::zones(
            &self.set_file(channel),
            &self.browse.model.user_folders().folders,
        )
    }

    /// The main file of an instrument channel's set on disk.
    fn set_file(&self, channel: &Channel) -> std::path::PathBuf {
        let roots = dawsome_media::ShippedRoots {
            user: dawsome_media::shipped_root(),
            installed: self.host.installed_instruments.clone(),
        };
        dawsome_media::paths::sound_file_from(
            self.session.samples_dir(),
            &channel.instrument,
            &roots,
        )
    }

    /// Who made a set the Library lists, by its name; empty when nothing says.
    fn maker_of(&self, channel: &Channel) -> String {
        let sets = &self.browse.library.sets;
        sets.shipped
            .iter()
            .chain(&sets.mine)
            .find(|set| set.name == channel.name)
            .map(|set| set.maker.clone())
            .unwrap_or_default()
    }

    pub(super) fn instrument_message(&mut self, message: InstrumentMessage) {
        let Some(channel) = self.instrument.as_ref().map(|open| open.channel) else {
            return;
        };
        let Some(shown) = self.session.project().channels.get(&channel) else {
            return;
        };
        let (kind, sound) = (shown.kind, shown.sound);
        match message {
            InstrumentMessage::Close => {
                self.close_instrument();
            }
            InstrumentMessage::Gate(gate) => {
                // Stored as none when it is what the sound does by itself, so the channel is at
                // the defaults again; a set of both kinds of zones always takes it.
                let own = self.instrument.as_ref().and_then(|open| open.own_gate);
                let sound = match (kind, own) {
                    (ChannelKind::Instrument, Some(own)) => Sound {
                        gate: (gate != own).then_some(gate),
                        ..sound
                    },
                    (ChannelKind::Instrument, None) => Sound {
                        gate: Some(gate),
                        ..sound
                    },
                    _ => sound.with_gate(gate, kind),
                };
                self.commit(Edit::SetChannelSound { channel, sound });
            }
            InstrumentMessage::Set(control, value) => self.drag_sound(channel, control, value),
            InstrumentMessage::Released => self.finish_instrument_drag(),
            InstrumentMessage::Reset(control) => {
                let edit = control_edit(channel, sound, control, window::default_value(control));
                self.commit(edit);
            }
            InstrumentMessage::Toggle(flag) => {
                let sound = match flag {
                    Flag::Reverse => Sound {
                        reverse: !sound.reverse,
                        ..sound
                    },
                    Flag::Filter => Sound {
                        filter: !sound.filter,
                        ..sound
                    },
                };
                self.commit(Edit::SetChannelSound { channel, sound });
            }
            InstrumentMessage::Pressed { key, velocity } => {
                if let Some(open) = self.instrument.take() {
                    self.end_map_note(&open);
                    self.instrument = Some(open);
                }
                let velocity = f32::from(velocity) / 128.0;
                // On the window's channel, whatever the rack selected since it opened.
                let result = self.session.send(Transport::ChannelNoteOn {
                    channel,
                    key,
                    velocity,
                });
                if result.is_ok() {
                    if let Some(open) = &mut self.instrument {
                        open.held = Some((key, (velocity * 128.0).round() as u8));
                    }
                }
                self.report(result);
            }
            InstrumentMessage::Lifted => {
                if let Some(open) = self.instrument.take() {
                    self.end_map_note(&open);
                    self.instrument = Some(Instrument { held: None, ..open });
                }
            }
            InstrumentMessage::Store => {
                if let Some(open) = &mut self.instrument {
                    open.compared = Some(sound);
                    open.other = None;
                }
            }
            InstrumentMessage::Compare => {
                // A stays as stored: settings other than A switch to it (and are kept as B),
                // A switches to B. So an undone switch, which leaves B playing, switches to A
                // again.
                let Some(open) = self.instrument.as_mut() else {
                    return;
                };
                let Some(stored) = open.compared else {
                    return;
                };
                let next = if sound == stored {
                    open.other
                } else {
                    open.other = Some(sound);
                    Some(stored)
                };
                if let Some(next) = next {
                    self.commit(Edit::SetChannelSound {
                        channel,
                        sound: next,
                    });
                }
            }
            InstrumentMessage::More => {
                if let Some(open) = &mut self.instrument {
                    open.more = !open.more;
                }
            }
            InstrumentMessage::OpenPlugin => {
                self.open_plugin(ProcessorOwner::Channel(channel));
            }
            InstrumentMessage::EditSample => self.open_sample_editor(channel),
            InstrumentMessage::Route(insert) => {
                self.commit(Edit::RouteChannel { channel, insert });
            }
            // The mixer's opening gives no task (`App::panel_opened`), so none is lost here.
            InstrumentMessage::MixerInsert => {
                let _ = self.give_mixer_insert(channel);
            }
            message @ (InstrumentMessage::InputDevice(_)
            | InstrumentMessage::InputChannels(_)
            | InstrumentMessage::RefreshInputs) => self.input_message(channel, message),
            InstrumentMessage::MidiOut(change) => self.midi_out_message(channel, change),
        }
    }

    /// A slider moved: the document takes the value at once, the drag's first change kept to
    /// undo it as one step on release.
    fn drag_sound(&mut self, channel: ChannelId, control: Control, value: f32) {
        let Some(shown) = self.session.project().channels.get(&channel) else {
            return;
        };
        self.drag_edit(control_edit(channel, shown.sound, control, value));
    }

    /// A drag of the window moved: the document takes `edit` at once, the drag's first change
    /// kept to undo it as one step on release ([`InstrumentMessage::Released`]).
    pub(super) fn drag_edit(&mut self, edit: Edit) {
        let result = self.session.apply_live(edit);
        if let (Ok(inverse), Some(open)) = (&result, &mut self.instrument) {
            open.drag.get_or_insert_with(|| inverse.clone());
        }
        self.after_edit(result.map(drop));
    }

    /// Ends the slider drag under way, as its release would: what Undo and Redo do first, so
    /// the drag is the newest step rather than lost under the step they take.
    pub(super) fn finish_instrument_drag(&mut self) {
        let drag = self.instrument.as_mut().and_then(|open| open.drag.take());
        self.finish_sound_drag(drag);
    }

    /// Ends a slider drag: one undo step, unless it left everything as it was.
    fn finish_sound_drag(&mut self, drag: Option<Edit>) {
        if let Some(undo) = drag {
            let mut probe = self.session.project().clone();
            let unchanged =
                dawsome_engine::edit::apply(&mut probe, undo.clone()).is_ok_and(|applied| {
                    dawsome_engine::edit::changed_nothing(&undo, &applied.inverse)
                });
            if !unchanged {
                self.session.record(undo);
            }
        }
    }

    /// Ends the note `open` holds on its map.
    fn end_map_note(&mut self, open: &Instrument) {
        if let Some((key, _)) = open.held {
            let result = self.session.send(Transport::LiveNoteOff { key });
            self.report(result);
        }
    }

    /// What the instrument window's own window shows: the window while it is open, its ground
    /// alone in the moment between its closing and its window going.
    pub(super) fn instrument_window_view(&self) -> Element<'_, Message> {
        match self.instrument_shown() {
            Some(shown) => window::view(shown),
            None => container(space())
                .width(Fill)
                .height(Fill)
                .style(style::panel)
                .into(),
        }
    }

    /// What the instrument window shows of its channel, laid out for the window's width; none
    /// in the moment between its channel going and the window closing.
    fn instrument_shown(&self) -> Option<window::View<'_>> {
        let open = self.instrument.as_ref()?;
        let project = self.session.project();
        let channel = project.channels.get(&open.channel)?;
        let inserts = project
            .mixer
            .ids()
            .filter_map(|id| {
                let insert = project.mixer.get(id)?;
                Some(window::InsertChoice {
                    id: Some(id),
                    name: insert.name.clone(),
                })
            })
            .chain([window::InsertChoice {
                id: None,
                name: "Mixer insert\u{2026}".to_owned(),
            }])
            .collect();
        Some(window::View {
            channel,
            input: self.input_view(open.channel),
            midi_out: self.midi_out_view(open.channel),
            maker: &open.maker,
            inserts,
            map: &open.map,
            zones: &open.zones,
            held: open.held,
            more: open.more,
            compared: open.compared.is_some(),
            gate: shown_gate(channel, open.own_gate),
            hint: self.instrument_hint(channel.kind),
            width: self.instrument_layout_width(),
            fine: self.modifiers.shift(),
            caches: &self.instrument_caches,
        })
    }

    /// The width the instrument window's content is laid out for: the window's own, as it
    /// reported it (under its least size too, where the strip scrolls), else, before it says,
    /// the width it opens at.
    fn instrument_layout_width(&self) -> f32 {
        self.instrument_width
            .unwrap_or_else(|| self.instrument_place.size(PopOut::Instrument).width)
    }

    /// The instrument window's own hint line: what its control under the cursor is and does,
    /// as the main window's hint bar names its own, while hints show; else how to play it.
    pub(super) fn instrument_hint(&self, kind: ChannelKind) -> String {
        match &self.hover {
            Some(hover @ Hover::Sound(_)) if self.shows_hints() => {
                hint::words(hover, &self.hint_context())
            }
            _ => window::help(kind, &self.keys),
        }
    }

    /// The instrument window's title: its channel's name and its sound's (a set's, a sample's
    /// file, a plugin's), or its kind where that is the channel's name already.
    pub(super) fn instrument_title(&self) -> String {
        let Some(channel) = self
            .instrument
            .as_ref()
            .and_then(|open| self.session.project().channels.get(&open.channel))
        else {
            return String::new();
        };
        let sound = self.sound_name(channel);
        let second = if sound.is_empty() || sound.eq_ignore_ascii_case(&channel.name) {
            window::kind_words(channel).to_owned()
        } else {
            sound
        };
        format!("{} \u{b7} {second}", channel.name)
    }

    /// The name of what `channel` plays: its set's as the Library lists it (else its file's),
    /// its sample's file, its plugin's, or its input device's.
    pub(super) fn sound_name(&self, channel: &Channel) -> String {
        let stem = |path: &std::path::Path| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        match channel.kind {
            ChannelKind::Instrument => {
                let file = self.set_file(channel);
                let sets = &self.browse.library.sets;
                sets.shipped
                    .iter()
                    .chain(&sets.mine)
                    .find(|set| set.sfz == file)
                    .map_or_else(|| stem(&channel.instrument), |set| set.name.clone())
            }
            ChannelKind::Sampler | ChannelKind::Audio => stem(&channel.sample),
            ChannelKind::Plugin => channel
                .processor
                .as_ref()
                .and_then(|processor| processor.name.clone())
                .unwrap_or_default(),
            ChannelKind::Input => match channel.input.as_ref() {
                Some(input) if input.device.is_empty() => "System default input".to_owned(),
                Some(input) => input.device.clone(),
                None => String::new(),
            },
        }
    }

    /// The app's pop-out windows follow their state and the app's focus: the instrument
    /// window's and the Settings window's own windows open while they are open and close once
    /// they are not ([`App::sync_instrument_window`], [`App::sync_settings_window`]); they and
    /// our plugin windows stay above every window while one of the app's windows has the
    /// keyboard and are stacked as any window once another program has it ([`pop_out_level`]).
    /// Runs after every message, and once at start for a window `--open-instrument` or
    /// `--panels settings` opened.
    pub fn sync_windows(&mut self) -> Task<Message> {
        let above = self.focused_window.is_some();
        let restack = above != self.windows_above;
        if restack {
            self.windows_above = above;
            self.keep_plugin_windows_above(above);
        }
        Task::batch([
            self.sync_instrument_window(restack),
            self.sync_settings_window(restack),
        ])
    }

    /// Opens the instrument window's own window while the instrument window is open and closes
    /// it once it is not, keeping its place for the next opening; asks it to the app's level
    /// again when `restack`.
    fn sync_instrument_window(&mut self, restack: bool) -> Task<Message> {
        let above = self.windows_above;
        match (self.instrument.is_some(), self.instrument_window) {
            (true, None) => {
                let settings = desktop::pop_out_settings(
                    PopOut::Instrument,
                    self.instrument_place.size(PopOut::Instrument),
                    self.instrument_place.position(),
                    pop_out_level(above),
                );
                let (id, opened) = iced::window::open(settings);
                self.instrument_window = Some(id);
                opened.discard()
            }
            (false, Some(id)) => {
                self.instrument_window = None;
                self.instrument_width = None;
                self.remember_instrument_place();
                iced::window::close(id)
            }
            (true, Some(id)) if restack => iced::window::set_level(id, pop_out_level(above)),
            _ => Task::none(),
        }
    }

    /// The instrument window opened, was resized or moved: its place is kept.
    pub(super) fn instrument_window_changed(&mut self, change: WindowChange) {
        let place = &mut self.instrument_place;
        match change {
            WindowChange::Opened { size, position } => {
                self.instrument_width = Some(size.width);
                *place = crate::settings::WindowPlace::of(size, position.or(place.position()));
            }
            WindowChange::Resized(size) => {
                self.instrument_width = Some(size.width);
                *place = crate::settings::WindowPlace::of(size, place.position());
            }
            WindowChange::Moved(at) => {
                *place = crate::settings::WindowPlace::of(place.size(PopOut::Instrument), Some(at));
            }
            WindowChange::Rescaled(_) | WindowChange::Focus(_) => {}
        }
    }

    /// Keeps the instrument window's place in the settings for its next opening.
    fn remember_instrument_place(&mut self) {
        let place = Some(self.instrument_place);
        if self.settings.instrument_window != place {
            self.settings.instrument_window = place;
            self.save_settings();
        }
    }

    /// The channel the window is open on: the typing keyboard and the MIDI ports no link
    /// routes play it while it is open and selected ([`App::live_target`]).
    pub(super) fn instrument_on(&self) -> Option<ChannelId> {
        self.instrument.as_ref().map(|open| open.channel)
    }

    /// The gate the window lights, for a test.
    #[cfg(test)]
    pub(super) fn instrument_gate(&self) -> Option<Gate> {
        let open = self.instrument.as_ref()?;
        let channel = self.session.project().channels.get(&open.channel)?;
        Some(shown_gate(channel, open.own_gate))
    }
}

/// What a note's end does on `channel` as it plays: its own gate, else its sound's (`own`), else
/// its kind's.
fn shown_gate(channel: &Channel, own: Option<Gate>) -> Gate {
    channel
        .sound
        .gate
        .or(own)
        .unwrap_or_else(|| Gate::of(channel.kind))
}

/// The edit `control` at `value` makes on `channel`, whose settings are `sound`: its volume or
/// pan, or its sound settings with that one changed.
fn control_edit(channel: ChannelId, sound: Sound, control: Control, value: f32) -> Edit {
    match control {
        Control::Volume => Edit::SetChannelVolume { channel, value },
        Control::Pan => Edit::SetChannelPan { channel, value },
        _ => Edit::SetChannelSound {
            channel,
            sound: window::with_value(sound, control, value),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use dawsome_engine::project::{Gate, Project};

    use super::super::tests::app_hosted;
    use super::*;
    use crate::document::Host;
    use crate::project_file;
    use crate::ui::instrument_window::InstrumentMessage as M;

    const F1: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/f1_four_on_floor.json"
    );

    fn send(app: &mut App, message: M) {
        let _ = app.update(Message::Instrument(message));
    }

    fn sound(app: &App) -> Sound {
        app.session.project().channels[&ChannelId(1)].sound
    }

    // F1's kick through its menu's "Instrument window…": the gate switched to Hold, a drag of
    // the attack (two moves, then the release) and a reset of the drag's control are one undo
    // step each; Compare stores the settings, then switches between them and the ones after.
    #[test]
    fn the_window_edits_a_channels_sound_one_undo_step_a_gesture() {
        let path = Path::new(F1);
        let project: Project = project_file::open(path).expect("opens").project;
        let (mut app, _player) = app_hosted(project, path, Host::offline());
        let _ = app.update(Message::OpenInstrument(ChannelId(1)));
        assert_eq!(app.instrument_on(), Some(ChannelId(1)));
        send(&mut app, M::Gate(Gate::Hold));
        assert_eq!(sound(&app).gate, Some(Gate::Hold));
        send(&mut app, M::Set(Control::Attack, 0.1));
        send(&mut app, M::Set(Control::Attack, 0.25));
        send(&mut app, M::Released);
        assert_eq!(sound(&app).attack, 0.25);
        assert_eq!(app.session.history().undo_len(), 2, "the drag is one step");
        send(&mut app, M::Store);
        send(&mut app, M::Set(Control::Attack, 0.5));
        send(&mut app, M::Released);
        send(&mut app, M::Compare);
        assert_eq!(sound(&app).attack, 0.25, "A again");
        send(&mut app, M::Compare);
        assert_eq!(sound(&app).attack, 0.5, "and back");
        send(&mut app, M::Reset(Control::Attack));
        assert_eq!(sound(&app).attack, 0.0);
        let _ = app.update(Message::Undo);
        assert_eq!(sound(&app).attack, 0.5);
        // The two switches, the second drag, the first drag and the gate.
        for _ in 0..5 {
            let _ = app.update(Message::Undo);
        }
        assert_eq!(sound(&app), Sound::default(), "every step undone");
        let _ = app.update(Message::ClearSelection);
        assert_eq!(app.instrument_on(), None, "Esc closes it");
    }

    // The PITCH group's bend range: a drag to 12 is one undo step, the project file then names
    // "bend_range":12 last in the channel's sound (and keeps format version 5, a sound setting's),
    // a reset is ±2 again and writes nothing; the next compile's plan bends live notes by it.
    #[test]
    fn the_bend_range_is_a_sound_setting_of_one_undo_step() {
        let path = Path::new(F1);
        let project: Project = project_file::open(path).expect("opens").project;
        let (mut app, _player) = app_hosted(project, path, Host::offline());
        let _ = app.update(Message::OpenInstrument(ChannelId(1)));
        send(&mut app, M::Set(Control::BendRange, 7.0));
        send(&mut app, M::Set(Control::BendRange, 12.0));
        send(&mut app, M::Released);
        assert_eq!(sound(&app).bend_range, 12);
        assert_eq!(app.session.history().undo_len(), 1, "the drag is one step");
        let json = app.session.project().to_json().expect("writes");
        assert!(json.starts_with(r#"{"format_version":5,"#), "{json}");
        assert!(
            json.contains(r#""resonance":0.0,"bend_range":12}"#),
            "{json}"
        );
        send(&mut app, M::Reset(Control::BendRange));
        assert_eq!(sound(&app), Sound::default());
        let _ = app.update(Message::Undo);
        assert_eq!(sound(&app).bend_range, 12);
        let _ = app.update(Message::Undo);
        assert_eq!(sound(&app), Sound::default());
    }

    // A press on the map plays its key at the velocity its height gave, through the live path,
    // on the window's channel; the release ends it; closing gives the selection back.
    #[test]
    fn the_map_plays_the_windows_channel_live() {
        use dawsome_engine::transport::Control as Live;
        let path = Path::new(F1);
        let mut project: Project = project_file::open(path).expect("opens").project;
        let second = Channel::sampler("snare", "impulse.wav");
        project.channels.insert(ChannelId(2), second);
        project.next_channel_id = ChannelId(3);
        let (mut app, _player) = app_hosted(project, path, Host::offline());
        assert_eq!(app.selected_channel, Some(ChannelId(1)));
        let _ = app.update(Message::OpenInstrument(ChannelId(2)));
        assert_eq!(app.selected_channel, Some(ChannelId(2)), "its keys play it");
        send(
            &mut app,
            M::Pressed {
                key: 64,
                velocity: 96,
            },
        );
        send(&mut app, M::Lifted);
        let sent: Vec<&Live> = app
            .session
            .controls_sent()
            .iter()
            .filter(|control| {
                matches!(
                    control,
                    Live::ChannelNoteOn { .. } | Live::LiveNoteOff { .. }
                )
            })
            .collect();
        assert_eq!(
            sent,
            [
                &Live::ChannelNoteOn {
                    channel: ChannelId(2),
                    key: 64,
                    velocity: 0.75
                },
                &Live::LiveNoteOff { key: 64 }
            ]
        );
        send(&mut app, M::Close);
        assert_eq!(app.selected_channel, Some(ChannelId(1)), "given back");
    }

    // The window follows the selection, so the typing keyboard plays the
    // selected channel, which is the window's. Before, it played the window's channel
    // whatever the rack selected since. A press on the kick's name let go of off it (no
    // click) selects the kick and leaves the window on the snare: typing plays the kick, the
    // selected one. The click on the kick takes the window there, and typing plays it through
    // the window's route. Each key-up ends its note; once the window closes, typing plays the
    // selected channel.
    #[test]
    fn typed_notes_play_the_windows_channel_while_it_is_open() {
        use dawsome_engine::transport::Control as Live;
        use iced::keyboard::key::Code;
        let path = Path::new(F1);
        let mut project: Project = project_file::open(path).expect("opens").project;
        project
            .channels
            .insert(ChannelId(2), Channel::sampler("snare", "impulse.wav"));
        project.next_channel_id = ChannelId(3);
        let (mut app, _player) = app_hosted(project, path, Host::offline());
        let _ = app.update(Message::OpenInstrument(ChannelId(2)));
        let _ = app.update(Message::ChannelSelected(ChannelId(1)));
        assert_eq!(app.selected_channel, Some(ChannelId(1)));
        assert_eq!(
            app.instrument_on(),
            Some(ChannelId(2)),
            "a press alone moves no window"
        );
        super::super::tests::legacy_keyboard(&mut app);
        let _ = app.update(Message::ToggleTyping);
        let z = |app: &mut App, pressed: bool| {
            let _ = app.update(Message::TypingKey {
                code: Code::KeyZ,
                pressed,
                repeat: false,
            });
        };
        let typed = |app: &App| -> Vec<Live> {
            app.session
                .controls_sent()
                .iter()
                .filter(|control| {
                    matches!(
                        control,
                        Live::ChannelNoteOn { .. }
                            | Live::LiveNoteOn { .. }
                            | Live::LiveNoteOff { .. }
                    )
                })
                .cloned()
                .collect()
        };
        z(&mut app, true);
        z(&mut app, false);
        let velocity = crate::app::typing::VELOCITY;
        assert_eq!(
            typed(&app),
            [
                Live::LiveNoteOn { key: 48, velocity },
                Live::LiveNoteOff { key: 48 },
            ],
            "the selected channel, not the window's"
        );
        let _ = app.update(Message::ChannelOpened(ChannelId(1)));
        assert_eq!(
            app.instrument_on(),
            Some(ChannelId(1)),
            "the double press moves it"
        );
        z(&mut app, true);
        z(&mut app, false);
        assert_eq!(
            typed(&app)[2..],
            [
                Live::ChannelNoteOn {
                    channel: ChannelId(1),
                    key: 48,
                    velocity,
                },
                Live::LiveNoteOff { key: 48 },
            ]
        );
        send(&mut app, M::Close);
        z(&mut app, true);
        assert_eq!(
            typed(&app)[4..],
            [Live::LiveNoteOn { key: 48, velocity }],
            "closed: the selected channel again"
        );
    }

    // A set whose zones are all loop_mode=one_shot (p13_shot) shows
    // One-shot lit, as it plays; Hold then holds it (stored, since it is not the set's own) and
    // One-shot gives the defaults back. F1's sampler is one-shot by itself, as before.
    #[test]
    fn the_window_lights_the_gate_a_one_shot_set_plays() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/p13_shot.json");
        let project: Project = project_file::open(&path).expect("opens").project;
        let (mut app, _player) = app_hosted(project, &path, Host::offline());
        let _ = app.update(Message::OpenInstrument(ChannelId(1)));
        assert_eq!(app.instrument_gate(), Some(Gate::OneShot));
        send(&mut app, M::Gate(Gate::Hold));
        assert_eq!(sound(&app).gate, Some(Gate::Hold));
        assert_eq!(app.instrument_gate(), Some(Gate::Hold));
        send(&mut app, M::Gate(Gate::OneShot));
        assert_eq!(sound(&app), Sound::default(), "the set's own again");
        let (mut f1, _player) = {
            let path = Path::new(F1);
            let project: Project = project_file::open(path).expect("opens").project;
            app_hosted(project, path, Host::offline())
        };
        let _ = f1.update(Message::OpenInstrument(ChannelId(1)));
        assert_eq!(f1.instrument_gate(), Some(Gate::OneShot));
    }

    // File → New replaces the document under the window, which closes
    // with it (it was left open on the new document's first channel, and A/B wrote the old
    // document's settings into it).
    #[test]
    fn the_window_closes_when_the_document_is_replaced() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/p13_envelope.json");
        let project: Project = project_file::open(&path).expect("opens").project;
        let (mut app, _player) = app_hosted(project, &path, Host::offline());
        let _ = app.update(Message::OpenInstrument(ChannelId(1)));
        send(&mut app, M::Store);
        let _ = app.update(Message::File(crate::app::FileMessage::New));
        assert_eq!(app.instrument_on(), None, "closed with its document");
        send(&mut app, M::Compare);
        assert_eq!(
            sound(&app),
            Sound::default(),
            "nothing written into the new one"
        );
        assert_eq!(app.session.history().undo_len(), 0);
    }

    // An undo that takes the window's channel away closes the window as Esc does: the key held
    // on its map ends, and the selection goes back to the channel selected before.
    #[test]
    fn the_window_closes_through_its_close_when_an_undo_takes_its_channel() {
        use dawsome_engine::transport::Control as Live;
        let path = Path::new(F1);
        let project: Project = project_file::open(path).expect("opens").project;
        let (mut app, _player) = app_hosted(project, path, Host::offline());
        let _ = app.update(Message::Channel(crate::app::ChannelCommand::Clone(
            ChannelId(1),
        )));
        let clone = ChannelId(2);
        let _ = app.update(Message::ChannelSelected(ChannelId(1)));
        let _ = app.update(Message::OpenInstrument(clone));
        send(
            &mut app,
            M::Pressed {
                key: 60,
                velocity: 100,
            },
        );
        let _ = app.update(Message::Undo);
        assert_eq!(app.instrument_on(), None);
        let offs = app
            .session
            .controls_sent()
            .iter()
            .filter(|control| matches!(control, Live::LiveNoteOff { key: 60 }))
            .count();
        assert_eq!(offs, 1, "the held key ends");
        assert_eq!(app.selected_channel, Some(ChannelId(1)), "given back");
    }

    // The map plays the window's channel whatever is selected: with the
    // window on "snare" (2) and the kick (1) selected in the rack after it opened, a press
    // starts a live note on 2.
    #[test]
    fn the_map_plays_its_channel_after_another_is_selected() {
        let path = Path::new(F1);
        let mut project: Project = project_file::open(path).expect("opens").project;
        project
            .channels
            .insert(ChannelId(2), Channel::sampler("snare", "impulse.wav"));
        project.next_channel_id = ChannelId(3);
        let (mut app, mut player) = app_hosted(project, path, Host::offline());
        let _ = app.update(Message::OpenInstrument(ChannelId(2)));
        let _ = app.update(Message::ChannelSelected(ChannelId(1)));
        assert_eq!(
            (app.instrument_on(), app.selected_channel),
            (Some(ChannelId(2)), Some(ChannelId(1)))
        );
        for _ in 0..16 {
            player.process(&mut [[0.0; 2]; 64]);
        }
        let _ = app.session.take_events();
        send(
            &mut app,
            M::Pressed {
                key: 60,
                velocity: 127,
            },
        );
        player.process(&mut [[0.0; 2]; 64]);
        let started: Vec<ChannelId> = app
            .session
            .take_events()
            .into_iter()
            .filter_map(|event| match event {
                dawsome_engine::realtime::EngineEvent::LiveNoteStarted { channel, .. } => {
                    Some(channel)
                }
                _ => None,
            })
            .collect();
        assert_eq!(started, [ChannelId(2)]);
    }

    /// What the rack's canvas does with `event` at rack point `at`: the message it sends.
    fn rack_event(
        app: &App,
        state: &mut crate::ui::rack::RackState,
        event: iced::widget::canvas::Event,
        at: iced::Point,
    ) -> Option<Message> {
        use iced::widget::canvas::Program as _;
        let project = app.session.project();
        let size = crate::ui::layout::rack_canvas_size(project.channels.len(), 16);
        let bounds = iced::Rectangle::new(iced::Point::ORIGIN, size);
        let cursor = iced::mouse::Cursor::Available(at);
        let action = app.channel_rack().update(state, &event, bounds, cursor)?;
        action.into_inner().0
    }

    /// A press and a release of `button` on rack row `row`'s name, through the rack's canvas,
    /// each message handled as the window handles it.
    fn click_name(app: &mut App, row: usize, button: iced::mouse::Button) {
        use iced::mouse::Event as Mouse;
        use iced::widget::canvas::Event;
        let at = crate::ui::layout::name_button_rect(row).center();
        let mut state = crate::ui::rack::RackState::default();
        for event in [Mouse::ButtonPressed(button), Mouse::ButtonReleased(button)] {
            if let Some(message) = rack_event(app, &mut state, Event::Mouse(event), at) {
                let _ = app.update(message);
            }
        }
    }

    /// Two left presses on `row`'s name within the double-press time, through the rack's own
    /// press path (one `RackState` across both, as a real double-click is).
    fn double_click_name(app: &mut App, row: usize) {
        use iced::mouse::{Button, Event as Mouse};
        use iced::widget::canvas::Event;
        let at = crate::ui::layout::name_button_rect(row).center();
        let mut state = crate::ui::rack::RackState::default();
        for _ in 0..2 {
            for event in [
                Mouse::ButtonPressed(Button::Left),
                Mouse::ButtonReleased(Button::Left),
            ] {
                if let Some(message) = rack_event(app, &mut state, Event::Mouse(event), at) {
                    let _ = app.update(message);
                }
            }
        }
    }

    // P3 through the rack's press path: a click on the snare's name selects it and opens no
    // window (asked for in use, 2026-10-27); a double-click opens its instrument window, a
    // window of its own; a click on the kick's selects it and leaves the window on the snare;
    // a double-click on the kick's retargets that window; another double-click on the kick
    // leaves it as it was (its stored A kept); a right press still opens the channel's menu,
    // and leaves the window where it is. Closing the window keeps the channel last clicked
    // selected.
    #[test]
    fn a_double_click_on_a_channels_name_opens_its_window_and_a_click_only_selects() {
        use iced::mouse::Button;
        let (mut app, _player) = two_channel_app(Host::offline());
        app.set_main_window(iced::window::Id::unique());
        click_name(&mut app, 1, Button::Left);
        assert_eq!(app.selected_channel, Some(ChannelId(2)));
        assert_eq!(app.instrument_on(), None, "a click opens no window");
        double_click_name(&mut app, 1);
        assert_eq!(app.instrument_on(), Some(ChannelId(2)));
        let id = app.instrument_window.expect("a window of its own");
        click_name(&mut app, 0, Button::Left);
        assert_eq!(
            (app.selected_channel, app.instrument_on()),
            (Some(ChannelId(1)), Some(ChannelId(2))),
            "a click selects and leaves the window"
        );
        double_click_name(&mut app, 0);
        assert_eq!(
            (
                app.selected_channel,
                app.instrument_on(),
                app.instrument_window
            ),
            (Some(ChannelId(1)), Some(ChannelId(1)), Some(id)),
            "retargeted"
        );
        send(&mut app, M::Store);
        double_click_name(&mut app, 0);
        send(&mut app, M::Set(Control::Attack, 0.5));
        send(&mut app, M::Released);
        send(&mut app, M::Compare);
        assert_eq!(
            sound(&app).attack,
            0.0,
            "A, stored before the second click, kept"
        );
        click_name(&mut app, 1, Button::Right);
        assert!(
            matches!(app.sampler_menu, Some((ChannelId(2), _))),
            "a right press still opens the menu"
        );
        assert_eq!(app.instrument_on(), Some(ChannelId(1)));
        let _ = app.update(Message::Escape(id));
        assert_eq!(app.instrument_on(), None);
        assert_eq!(
            app.selected_channel,
            Some(ChannelId(1)),
            "the channel clicked"
        );
    }

    // P4: the window shows its channel's sound again when that is replaced, and again when the
    // replacement is undone: F1's kick (a sampler: no zones) given the test set Pad shows the
    // set's zones, and the undo the sampler's none; the window stays open on the channel.
    #[test]
    fn the_window_follows_its_channels_sound_when_it_is_replaced() {
        let path = Path::new(F1);
        let project: Project = project_file::open(path).expect("opens").project;
        let (mut app, _player) = app_hosted(project, path, Host::offline());
        let _ = app.update(Message::OpenInstrument(ChannelId(1)));
        let zones = |app: &App| app.instrument.as_ref().map(|open| open.zones.len());
        assert_eq!(zones(&app), Some(0));
        let pad = SoundSource::instrument("instruments/pad/pad.sfz");
        app.replace_sound(ChannelId(1), pad, "Pad");
        assert_eq!(app.error, None);
        assert_eq!(
            app.session.project().channels[&ChannelId(1)].kind,
            ChannelKind::Instrument
        );
        assert!(
            zones(&app).is_some_and(|zones| zones > 0),
            "the set's zones"
        );
        assert_eq!(app.instrument_on(), Some(ChannelId(1)));
        let _ = app.update(Message::Undo);
        assert_eq!(zones(&app), Some(0), "the sampler again");
        assert_eq!(app.instrument_on(), Some(ChannelId(1)));
    }

    // An open window follows every selection the
    // app makes, so it shows the channel the keys play: a click on a name, a clone, a sound
    // let go of on another channel's header, a plugin the picker adds, and the first channel
    // an undo falls back to when it takes the selected one away (selected here by a press let
    // go of off its name, which selects and leaves the window where it is). Closing the window
    // then leaves the selection where it went.
    #[test]
    fn the_window_follows_every_selection_the_app_makes() {
        use crate::ui::picker::{PickerMessage, PickerTarget};
        let catalogue =
            dawsome_host_clap::Catalogue::load(&Path::new(F1).with_file_name("p7_catalogue.json"))
                .expect("loads");
        let host = Host {
            clap: dawsome_host_clap::ClapHost::new(catalogue),
            ..Host::offline()
        };
        let (mut app, _player) = two_channel_app(host);
        let shown = |app: &App| (app.selected_channel, app.instrument_on());
        let both = |id| (Some(ChannelId(id)), Some(ChannelId(id)));
        double_click_name(&mut app, 1);
        assert_eq!(shown(&app), both(2), "a double-click");
        let _ = app.update(Message::Channel(crate::app::ChannelCommand::Clone(
            ChannelId(1),
        )));
        assert_eq!(shown(&app), both(3), "a clone");
        app.replace_sound(
            ChannelId(2),
            SoundSource::sampler("p9_tone440.wav"),
            "p9_tone440",
        );
        assert_eq!(shown(&app), both(2), "a replaced sound");
        let _ = app.update(Message::OpenPicker(PickerTarget::Instrument));
        let _ = app.update(Message::Picker(PickerMessage::Submit));
        assert_eq!(shown(&app), both(4), "the picker's plugin");
        double_click_name(&mut app, 1);
        let _ = app.update(Message::ChannelSelected(ChannelId(4)));
        assert_eq!(
            shown(&app),
            (Some(ChannelId(4)), Some(ChannelId(2))),
            "a press alone"
        );
        let _ = app.update(Message::Undo);
        assert_eq!(shown(&app), both(1), "the undo's fallback");
        let id = app.instrument_window.expect("open");
        let _ = app.update(Message::Escape(id));
        assert_eq!(shown(&app), (Some(ChannelId(1)), None), "closing keeps it");
    }

    // Undo pressed while one of the window's
    // sliders is held ends the drag first, as it ends the rack's and the mixer's: the drag is
    // the newest step, so the undo takes it back and Redo gives it again; the release after
    // records nothing. Before, the undo took back the step before the drag (here a replaced
    // sound, whose undo reopened the window and let go of the drag) and the drag's step was
    // lost.
    #[test]
    fn undo_during_a_slider_drag_ends_the_drag_first() {
        let path = Path::new(F1);
        let project: Project = project_file::open(path).expect("opens").project;
        let (mut app, _player) = app_hosted(project, path, Host::offline());
        let _ = app.update(Message::OpenInstrument(ChannelId(1)));
        let pad = SoundSource::instrument("instruments/pad/pad.sfz");
        app.replace_sound(ChannelId(1), pad, "Pad");
        let steps = |app: &App| {
            let history = app.session.history();
            (history.undo_len(), history.redo_len())
        };
        assert_eq!(steps(&app), (1, 0));
        send(&mut app, M::Set(Control::Attack, 0.5));
        let _ = app.update(Message::Undo);
        let kind = |app: &App| app.session.project().channels[&ChannelId(1)].kind;
        assert_eq!(kind(&app), ChannelKind::Instrument, "the replace is kept");
        assert_eq!(sound(&app).attack, 0.0, "the drag undone");
        assert_eq!(steps(&app), (1, 1));
        send(&mut app, M::Released);
        assert_eq!(steps(&app), (1, 1), "the release records nothing");
        let _ = app.update(Message::Redo);
        assert_eq!(sound(&app).attack, 0.5, "the drag again");
    }

    // A click on an audio channel's name opens its
    // window, whose foot line says what is true there: its clip plays its file and notes do
    // nothing (the keyboard plays nothing on it), not that the computer keyboard plays it.
    #[test]
    fn an_audio_channels_window_says_notes_do_nothing_there() {
        let path = Path::new(F1).with_file_name("p8_ui.json");
        let project: Project = project_file::open(&path).expect("opens").project;
        let (mut app, _player) = app_hosted(project, &path, Host::offline());
        let _ = app.update(Message::ChannelSelected(ChannelId(1)));
        let _ = app.update(Message::ChannelOpened(ChannelId(1)));
        assert_eq!(app.instrument_on(), Some(ChannelId(1)));
        let shown = app.instrument_shown().expect("shown");
        assert_eq!(shown.channel.kind, ChannelKind::Audio);
        assert_eq!(
            shown.hint,
            "An audio channel plays its clip; notes do nothing here."
        );
    }

    // Esc in the main window closes a menu open over
    // a panel first (drawn on the window's top layer, it is above everything), then the
    // instrument window: a channel's menu, a knob's automation menu and a playlist track's
    // menu, each over the open window, go one Esc each, the window with the next.
    #[test]
    fn esc_closes_a_panel_menu_before_the_instrument_window() {
        use crate::ui::arranger::PlaylistMessage;
        use crate::ui::rack::Knob;
        let (mut app, _player) = two_channel_app(Host::offline());
        let main = iced::window::Id::unique();
        app.set_main_window(main);
        let _ = app.update(Message::ChannelSelected(ChannelId(2)));
        let _ = app.update(Message::ChannelOpened(ChannelId(2)));
        let at = iced::Point::new(83.0, 60.0);
        let _ = app.update(Message::Media(crate::app::MediaMessage::SamplerMenu {
            channel: ChannelId(2),
            at,
        }));
        assert!(app.sampler_menu.is_some());
        let _ = app.update(Message::Escape(main));
        assert_eq!(app.sampler_menu, None, "the channel's menu first");
        assert_eq!(app.instrument_on(), Some(ChannelId(2)));
        let _ = app.update(Message::KnobMenu {
            channel: ChannelId(2),
            knob: Knob::Volume,
            at,
        });
        assert!(app.automate.is_some());
        let _ = app.update(Message::Escape(main));
        assert!(app.automate.is_none(), "the automation menu first");
        assert_eq!(app.instrument_on(), Some(ChannelId(2)));
        let _ = app.update(Message::TogglePanel(crate::ui::panels::Panel::Playlist));
        let name = iced::Point::new(40.0, 20.0);
        let _ = app.update(Message::Playlist(PlaylistMessage::TracksPressed {
            at: name,
            button: iced::mouse::Button::Right,
            double: false,
        }));
        assert!(app.arranger.track_menu.is_some(), "a track's menu");
        let _ = app.update(Message::Escape(main));
        assert!(app.arranger.track_menu.is_none(), "the track's menu first");
        assert_eq!(app.instrument_on(), Some(ChannelId(2)));
        let _ = app.update(Message::Escape(main));
        assert_eq!(app.instrument_on(), None, "then the window");
    }

    /// F1 with a second sampler, "snare", beside its kick.
    fn two_channel_app(host: Host) -> (App, dawsome_engine::realtime::Player) {
        let path = Path::new(F1);
        let mut project: Project = project_file::open(path).expect("opens").project;
        project
            .channels
            .insert(ChannelId(2), Channel::sampler("snare", "impulse.wav"));
        project.next_channel_id = ChannelId(3);
        app_hosted(project, path, host)
    }

    // v2: the instrument window is a window of its own, one at a time. Opening it opens one
    // window titled "<channel> · <sound>"; another channel's right-click retargets that window
    // (its title follows); Esc in it, its X and the system closing it each close it, and the
    // selection goes back. Esc in the main window acts there first: with the picker open it
    // closes the picker and leaves the instrument window open.
    #[test]
    fn the_instrument_window_is_one_window_of_its_own() {
        let (mut app, _player) = two_channel_app(Host::offline());
        let main = iced::window::Id::unique();
        app.set_main_window(main);
        let _ = app.update(Message::OpenInstrument(ChannelId(1)));
        let id = app.instrument_window.expect("a window of its own");
        assert_eq!(app.window_title(id), "kick \u{b7} impulse");
        assert_eq!(app.window_title(main), app.title());
        let _ = app.update(Message::OpenInstrument(ChannelId(2)));
        assert_eq!(
            app.instrument_window,
            Some(id),
            "retargeted, not a second window"
        );
        assert_eq!(app.window_title(id), "snare \u{b7} impulse");
        let _ = app.update(Message::Escape(id));
        assert_eq!((app.instrument_on(), app.instrument_window), (None, None));
        assert_eq!(app.selected_channel, Some(ChannelId(1)), "given back");

        let _ = app.update(Message::OpenInstrument(ChannelId(2)));
        let id = app.instrument_window.expect("open again");
        let _ = app.update(Message::OpenPicker(
            crate::ui::picker::PickerTarget::AddInstrument,
        ));
        let _ = app.update(Message::Escape(main));
        assert!(
            app.picker.is_none(),
            "Esc in the main window closes its picker"
        );
        assert_eq!(
            app.instrument_window,
            Some(id),
            "and leaves the window open"
        );
        let _ = app.update(Message::WindowCloseRequested(id));
        assert_eq!((app.instrument_on(), app.instrument_window), (None, None));
        assert!(!app.exiting(), "its X closes it alone");
        assert!(app.files.modal.is_none());

        let _ = app.update(Message::OpenInstrument(ChannelId(2)));
        let id = app.instrument_window.expect("open again");
        let _ = app.update(Message::WindowClosed(id));
        assert_eq!((app.instrument_on(), app.instrument_window), (None, None));
        assert_eq!(app.selected_channel, Some(ChannelId(1)));
        // A close request from a window already closing is no request to close the app.
        let _ = app.update(Message::WindowCloseRequested(id));
        assert!(!app.exiting());
        let _ = app.update(Message::WindowCloseRequested(main));
        assert!(
            app.exiting(),
            "the main window's X closes the app (nothing unsaved)"
        );
    }

    // The instrument window stays above the main window while the app has
    // the keyboard and is stacked as any window once another program has it. Opened while the
    // main window has the keyboard it asks to be above; focus moving between the two windows
    // keeps it so (the main window losing the keyboard ends typed notes, as ever); another
    // program taking the keyboard makes it Normal, and the main window taking it back makes it
    // above again. The main window's X closes the app with it; the main window gone without
    // asking (the system closed it) closes the app too.
    #[test]
    fn the_instrument_window_stays_above_the_main_window_while_the_app_has_the_keyboard() {
        use iced::window::Level;
        let (mut app, _player) = two_channel_app(Host::offline());
        let main = iced::window::Id::unique();
        app.set_main_window(main);
        let focus = |app: &mut App, id, focused| {
            let _ = app.update(Message::Windowed(id, WindowChange::Focus(focused)));
        };
        assert_eq!(pop_out_level(true), Level::AlwaysOnTop);
        assert_eq!(pop_out_level(false), Level::Normal);
        focus(&mut app, main, true);
        assert!(app.windows_above);
        let _ = app.update(Message::OpenInstrument(ChannelId(1)));
        let id = app.instrument_window.expect("open");
        // The main window gives the keyboard to the instrument window: still above.
        focus(&mut app, main, false);
        focus(&mut app, id, true);
        assert!(app.windows_above);
        // Another program takes it.
        focus(&mut app, id, false);
        assert!(!app.windows_above, "it does not float over other programs");
        assert_eq!(app.focused_window, None);
        focus(&mut app, main, true);
        assert!(app.windows_above);
        assert_eq!(app.focused_window, Some(main));
        // A late "lost" of a window that no longer has it changes nothing.
        focus(&mut app, id, false);
        assert!(app.windows_above);

        let _ = app.update(Message::WindowCloseRequested(main));
        assert!(
            app.exiting(),
            "the main window's X closes the app, its windows too"
        );

        let (mut app, _player) = two_channel_app(Host::offline());
        let main = iced::window::Id::unique();
        app.set_main_window(main);
        let _ = app.update(Message::OpenInstrument(ChannelId(1)));
        let _ = app.update(Message::WindowClosed(main));
        assert!(app.exiting(), "the main window gone closes the app");
    }

    // v2: its size and place are kept in the settings when it closes and when the app closes
    // with it open, and it opens there again, in a later start too. The main window's size is
    // its own.
    #[test]
    fn the_instrument_windows_size_and_place_are_kept_in_the_settings() {
        use crate::settings::{Settings, WindowPlace};
        use iced::{Point, Size};
        let dir =
            std::env::temp_dir().join(format!("dawsome-instrument-place-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("dawsome/settings.json");
        let mut host = Host::offline();
        host.settings_file = Some(file.clone());
        host.saves_settings = true;
        let (mut app, _player) = two_channel_app(host.clone());
        let main = iced::window::Id::unique();
        app.set_main_window(main);
        let _ = app.update(Message::OpenInstrument(ChannelId(1)));
        let id = app.instrument_window.expect("open");
        let changed = |app: &mut App, id, change| {
            let _ = app.update(Message::Windowed(id, change));
        };
        changed(&mut app, id, WindowChange::Resized(Size::new(600.0, 450.0)));
        changed(&mut app, id, WindowChange::Moved(Point::new(30.0, 40.0)));
        changed(
            &mut app,
            main,
            WindowChange::Resized(Size::new(1100.0, 700.0)),
        );
        assert_eq!(app.window_size, Size::new(1100.0, 700.0));
        let _ = app.update(Message::Escape(id));
        let saved = Settings::load(Some(&file)).instrument_window;
        assert_eq!(
            saved,
            Some(WindowPlace::of(
                Size::new(600.0, 450.0),
                Some(Point::new(30.0, 40.0))
            ))
        );
        let _ = app.update(Message::OpenInstrument(ChannelId(2)));
        let id = app.instrument_window.expect("open again");
        changed(&mut app, id, WindowChange::Resized(Size::new(700.0, 500.0)));
        let _ = app.update(Message::WindowCloseRequested(main));
        assert!(app.exiting());
        let saved = Settings::load(Some(&file)).instrument_window;
        assert_eq!(
            saved.map(|place| place.size(PopOut::Instrument)),
            Some(Size::new(700.0, 500.0))
        );
        let (later, _player) = two_channel_app(host);
        assert_eq!(
            later.instrument_place.size(PopOut::Instrument),
            Size::new(700.0, 500.0)
        );
        assert_eq!(
            later.instrument_place.position(),
            Some(Point::new(30.0, 40.0))
        );
        std::fs::remove_dir_all(&dir).expect("removed");
    }

    // v2: the keys of either window reach the same listeners, so with the instrument window
    // having the keys the typing keyboard plays its channel, as with the main window's.
    #[test]
    fn typed_notes_play_from_either_window() {
        use dawsome_engine::transport::Control as Live;
        use iced::keyboard::{self, key, Key, Modifiers};
        use iced::{event, Event};
        let (mut app, _player) = two_channel_app(Host::offline());
        let _ = app.update(Message::OpenInstrument(ChannelId(2)));
        super::super::tests::legacy_keyboard(&mut app);
        let _ = app.update(Message::ToggleTyping);
        let id = app.instrument_window.expect("open");
        for window in [id, iced::window::Id::unique()] {
            for pressed in [true, false] {
                let key = Key::Character("z".into());
                let physical_key = key::Physical::Code(key::Code::KeyZ);
                let event = if pressed {
                    keyboard::Event::KeyPressed {
                        key: key.clone(),
                        modified_key: key,
                        physical_key,
                        location: keyboard::Location::Standard,
                        modifiers: Modifiers::empty(),
                        text: None,
                        repeat: false,
                    }
                } else {
                    keyboard::Event::KeyReleased {
                        key: key.clone(),
                        modified_key: key,
                        physical_key,
                        location: keyboard::Location::Standard,
                        modifiers: Modifiers::empty(),
                    }
                };
                let message = super::super::key_input::key_event(
                    Event::Keyboard(event),
                    event::Status::Ignored,
                    window,
                )
                .expect("the typing keyboard's");
                let _ = app.update(message);
            }
        }
        let velocity = crate::app::typing::VELOCITY;
        let played: Vec<&Live> = app
            .session
            .controls_sent()
            .iter()
            .filter(|control| matches!(control, Live::ChannelNoteOn { .. }))
            .collect();
        let on = Live::ChannelNoteOn {
            channel: ChannelId(2),
            key: 48,
            velocity,
        };
        assert_eq!(played, [&on, &on]);
    }

    // v2: the instrument window names its own controls on the line at its foot (while hints
    // show), and the main window's hint bar names the main window's alone; with no control
    // under the cursor the line says how to play it.
    #[test]
    fn the_instrument_window_has_its_own_hint_line() {
        use crate::ui::hint::{HoverMessage, SoundPart};
        let (mut app, _player) = two_channel_app(Host::offline());
        app.hints_flag = Some(true);
        // The computer keyboard switched off, as it was by default when this was written: on,
        // its line fills the main bar while nothing is under the cursor.
        let _ = app.update(Message::ToggleTyping);
        let _ = app.update(Message::OpenInstrument(ChannelId(1)));
        let attack = Hover::Sound(SoundPart::Control(Control::Attack));
        let _ = app.update(Message::Hover(HoverMessage::Over(attack.clone())));
        assert_eq!(app.hint_words(), "", "not on the main window's bar");
        assert!(
            app.instrument_hint(ChannelKind::Sampler)
                .starts_with("Attack \u{b7} "),
            "{}",
            app.instrument_hint(ChannelKind::Sampler)
        );
        let _ = app.update(Message::Hover(HoverMessage::Left(attack)));
        assert_eq!(
            app.instrument_hint(ChannelKind::Sampler),
            window::help(ChannelKind::Sampler, &crate::keys::Keys::default())
        );
        app.hints_flag = Some(false);
        let gate = Hover::Sound(SoundPart::Gate);
        let _ = app.update(Message::Hover(HoverMessage::Over(gate)));
        assert_eq!(
            app.instrument_hint(ChannelKind::Sampler),
            window::help(ChannelKind::Sampler, &crate::keys::Keys::default()),
            "hints off: how to play it"
        );
    }

    // v2: the LEVEL group's output chooser routes the channel to the insert picked, one undo
    // step, as Ctrl+L does.
    #[test]
    fn the_output_chooser_routes_the_channel() {
        use dawsome_engine::project::InsertId;
        let (mut app, _player) = two_channel_app(Host::offline());
        let _ = app.update(Message::OpenInstrument(ChannelId(2)));
        let insert = |app: &App| app.session.project().channels[&ChannelId(2)].insert;
        assert_eq!(insert(&app), InsertId::MASTER);
        send(&mut app, M::Route(InsertId(3)));
        assert_eq!(insert(&app), InsertId(3));
        assert_eq!(app.session.history().undo_len(), 1);
        let _ = app.update(Message::Undo);
        assert_eq!(insert(&app), InsertId::MASTER);
    }

    // The output chooser lists the inserts, then "Mixer insert…", which plays the channel into
    // an insert of its own as the channel menu's row does: channel 2 (on the master; channel 1 on
    // the master too, so every insert is free) into Insert 1, the mixer opened there, one undo
    // step; undone, the document as it was.
    #[test]
    fn the_output_choosers_mixer_insert_gives_the_channel_a_strip() {
        use dawsome_engine::project::InsertId;
        let (mut app, _player) = two_channel_app(Host::offline());
        let _ = app.update(Message::OpenInstrument(ChannelId(2)));
        let choices = app.instrument_shown().expect("shown").inserts;
        let names: Vec<&str> = choices.iter().map(|choice| choice.name.as_str()).collect();
        assert_eq!(names[..3], ["Master", "Insert 1", "Insert 2"]);
        assert_eq!(
            choices
                .last()
                .map(|choice| (choice.id, choice.name.as_str())),
            Some((None, "Mixer insert\u{2026}"))
        );
        assert_eq!(choices.len(), 18, "the master, 16 inserts and the row");
        let before = app.session.project().clone();
        assert_eq!(
            app.session.project().channels[&ChannelId(1)].insert,
            InsertId::MASTER
        );
        send(&mut app, M::MixerInsert);
        assert_eq!(
            app.session.project().channels[&ChannelId(2)].insert,
            InsertId(1)
        );
        assert!(app.panels.is_open(crate::ui::panels::Panel::Mixer));
        assert_eq!(app.mixer.selected, InsertId(1));
        assert_eq!(app.session.history().undo_len(), 1);
        let _ = app.update(Message::Undo);
        assert_eq!(app.session.project(), &before);
    }

    // Undoing an A/B switch keeps the stored A. Store A (attack 0.25),
    // change it (0.5), A/B plays A, Undo takes the switch back (0.5): A/B then plays A again,
    // and once more the settings it switched from.
    #[test]
    fn undoing_a_switch_keeps_the_stored_a() {
        let path = Path::new(F1);
        let project: Project = project_file::open(path).expect("opens").project;
        let (mut app, _player) = app_hosted(project, path, Host::offline());
        let _ = app.update(Message::OpenInstrument(ChannelId(1)));
        send(&mut app, M::Set(Control::Attack, 0.25));
        send(&mut app, M::Released);
        send(&mut app, M::Store);
        send(&mut app, M::Set(Control::Attack, 0.5));
        send(&mut app, M::Released);
        send(&mut app, M::Compare);
        assert_eq!(sound(&app).attack, 0.25, "A");
        let _ = app.update(Message::Undo);
        assert_eq!(sound(&app).attack, 0.5, "the switch undone");
        send(&mut app, M::Compare);
        assert_eq!(sound(&app).attack, 0.25, "A is kept");
        send(&mut app, M::Compare);
        assert_eq!(sound(&app).attack, 0.5, "and B");
    }

    // A desktop file dropped on the instrument
    // window does nothing; on the main window it lands as before (an audio clip).
    #[test]
    fn a_file_dropped_on_the_instrument_window_does_nothing() {
        let (mut app, _player) = two_channel_app(Host::offline());
        let _ = app.update(Message::OpenInstrument(ChannelId(2)));
        let id = app.instrument_window.expect("open");
        let tone = Path::new(F1).with_file_name("p9_tone440.wav");
        let dropped = |window| {
            super::super::rescale_message(
                iced::Event::Window(iced::window::Event::FileDropped(tone.clone())),
                iced::event::Status::Ignored,
                window,
            )
            .expect("a drop")
        };
        let before = app.session.project().clone();
        let _ = app.update(dropped(id));
        assert_eq!(
            app.session.project(),
            &before,
            "nothing on the instrument window"
        );
        assert_eq!(app.session.history().undo_len(), 0);
        let _ = app.update(dropped(iced::window::Id::unique()));
        assert_eq!(
            app.session.history().undo_len(),
            1,
            "the main window takes it"
        );
    }

    // The window's content is laid out for the
    // width the window reports, also under its least size (a tiling window manager may make it
    // 380 px wide), so its strip scrolls; only what is kept for the next opening is bounded.
    // The helper alone was tested once: the view the window builds is
    // laid out for 380 px too.
    #[test]
    fn the_window_lays_out_for_the_width_it_reports() {
        let (mut app, _player) = two_channel_app(Host::offline());
        let _ = app.update(Message::OpenInstrument(ChannelId(2)));
        let id = app.instrument_window.expect("open");
        let small = iced::Size::new(380.0, 300.0);
        let _ = app.update(Message::Windowed(id, WindowChange::Resized(small)));
        assert_eq!(app.instrument_layout_width(), 380.0);
        let laid_out = |app: &App| app.instrument_shown().map(|shown| shown.width);
        assert_eq!(laid_out(&app), Some(380.0), "the view is laid out for it");
        let _ = app.update(Message::Windowed(
            id,
            WindowChange::Moved(iced::Point::new(5.0, 6.0)),
        ));
        assert_eq!(app.instrument_layout_width(), 380.0, "a move keeps it");
        assert_eq!(laid_out(&app), Some(380.0));
        assert_eq!(
            app.instrument_place.size(PopOut::Instrument),
            crate::ui::instrument_window::WINDOW_SIZE,
            "what opens next time is bounded"
        );
    }

    // Esc in the instrument window closes it
    // whatever the main window has up, a dialog too, as its X does; the dialog stays.
    #[test]
    fn esc_in_the_instrument_window_closes_it_under_a_dialog() {
        let (mut app, _player) = two_channel_app(Host::offline());
        let _ = app.update(Message::OpenInstrument(ChannelId(2)));
        let id = app.instrument_window.expect("open");
        let _ = app.update(Message::StepClicked {
            channel: ChannelId(1),
            step: 1,
        });
        let _ = app.update(Message::File(crate::app::FileMessage::New));
        assert!(app.files.modal.is_some(), "the unsaved question is up");
        let _ = app.update(Message::Escape(id));
        assert_eq!(
            app.instrument_on(),
            None,
            "Esc closed the instrument window"
        );
        assert!(app.files.modal.is_some(), "and left the question up");
    }
}
