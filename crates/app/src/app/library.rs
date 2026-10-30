//! The app's side of the Library panel beyond its Samples tree (which is in [`super::browse`]):
//! the rows it lists, presses on the starter kit's sounds, the instrument sets, the plugin
//! instruments and the MIDI devices, and what each becomes where a drag of it is let go. A
//! sound plays and drops as any sound file does. A plugin instrument dropped on the rack is a
//! new plugin channel, as the picker adds one; dropped on a playlist lane it is that and a clip
//! of the rack's pattern there, the new channel selected. An instrument set drops the same way,
//! as an instrument channel. A sound, a set or a plugin dropped on a channel's header gives
//! that channel the sound in its place. A MIDI device dropped on a rack row makes that channel
//! play the device's notes; its row's menu unlinks it, and its tick is the Settings panel's.

use std::path::{Path, PathBuf};

use dawsome_engine::edit::Edit;
use dawsome_engine::project::{stable_port_name, Channel, ChannelId, ChannelKind, SoundSource};
use dawsome_engine::time::Ticks;
use dawsome_host_clap::catalogue::{Offered, PluginKind};
use std::time::{Duration, Instant};

use iced::{mouse, Element, Point, Task};

use super::browse::{Held, MediaMessage, Press, RowMenu};
use super::files::blocking;
use super::{App, Message};
use crate::sets;
use crate::starter_kit;
use crate::ui::browser::{self, BrowserMessage, BrowserPanel, MenuKind, Pick};
use crate::ui::clip_gesture;
use crate::ui::layout::TOOLBAR_HEIGHT;
use crate::ui::library::{Contents, Item};
use crate::ui::panels::Panel;
use crate::ui::picker::PickerTarget;

/// Two presses on one instrument row this close together add it to the rack.
const DOUBLE_PRESS: Duration = Duration::from_millis(400);

/// How long a set's preview note plays before it is cut.
const SET_PREVIEW: Duration = Duration::from_millis(1500);

impl App {
    /// The Library's rows as they are now.
    pub(super) fn library_items(&self) -> Vec<Item> {
        self.items_over(self.browse.model.rows().len())
    }

    /// The Library's rows over a Samples tree of `tree` rows.
    fn items_over(&self, tree: usize) -> Vec<Item> {
        let catalogue = self.host.catalogue();
        let project = self.session.project();
        let links: Vec<(String, String)> = project
            .channels
            .values()
            .filter(|channel| channel.takes_midi_input())
            .filter_map(|channel| Some((channel.midi_input.clone()?, channel.name.clone())))
            .collect();
        let kit = starter_kit::names();
        let model = &self.browse.model;
        self.browse.library.items(&Contents {
            search: &model.search,
            kit: &kit,
            plugins: catalogue.offered(PluginKind::Instrument),
            tree,
            folders: model.roots.len(),
            ports: self.library_ports(&links),
        })
    }

    /// The Library panel as it is now.
    pub(super) fn library_panel(&self) -> BrowserPanel<'_> {
        let rows = self.browse.model.rows();
        let items = self.items_over(rows.len());
        let top = self.browse.library.top;
        // A menu whose row is scrolled out of view shows nothing, as the row does: the menu is
        // drawn on the window's top layer, which no panel's edge cuts.
        let shown = top..top + browser::visible_rows(self.panels_height());
        let menu = self.browse.menu.as_ref().and_then(|menu| {
            let (row, kind) = match menu {
                RowMenu::Remove(tree) => (
                    items.iter().position(|item| *item == Item::Tree(*tree))?,
                    MenuKind::Remove,
                ),
                RowMenu::Unlink(port) => (
                    items.iter().position(
                        |item| matches!(item, Item::Port(shown) if shown.name == *port),
                    )?,
                    MenuKind::Unlink,
                ),
            };
            shown.contains(&row).then_some((row, kind))
        });
        BrowserPanel {
            browser: &self.browse.model,
            rows,
            items,
            top,
            kit: self.host.starter_kit.as_deref(),
            scanning: self.scan.is_some(),
            held: self.browse.press.is_some(),
            menu,
            menu_foot: self.hint_bar_height(),
            folded: &self.browse.library.folded,
            cache: &self.browse.cache,
        }
    }

    pub(super) fn browser_view(&self) -> Element<'_, Message> {
        self.library_panel().view()
    }

    /// Opens the menu of the Library row of MIDI device `port`, as a right press on that row
    /// does: Unlink, when a channel plays the device. `--library-menu`, for a screenshot of the
    /// menu. Whether it opened.
    pub fn open_library_menu(&mut self, port: &str) -> bool {
        let items = self.library_items();
        let row = items
            .iter()
            .position(|item| matches!(item, Item::Port(shown) if shown.name == port));
        let Some(rect) = row.and_then(|row| browser::row_rect(row, self.browse.library.top)) else {
            return false;
        };
        let pressed = BrowserMessage::Picked {
            pick: Pick::Device(port.to_owned()),
            button: mouse::Button::Right,
            at: Point::new(rect.center_x(), rect.center_y() + TOOLBAR_HEIGHT),
        };
        let _ = self.update(Message::Browser(pressed));
        self.browse.menu.is_some()
    }

    /// A press on a Library row outside the Samples tree, the cursor at window point `at`: a
    /// sound previews (and may be dragged), a plugin or a device may be dragged, Rescan scans,
    /// a tick lets a device's messages in or keeps them out, and the right button on a linked
    /// device offers Unlink.
    pub(super) fn picked(&mut self, pick: Pick, button: mouse::Button, at: Point) -> Task<Message> {
        self.focus = Panel::Browser;
        self.browse.menu = None;
        let left = button == mouse::Button::Left;
        let instrument = matches!(
            pick,
            Pick::Sound(_) | Pick::Set(_) | Pick::Plugin(_) | Pick::ExternalInput
        );
        if left && instrument {
            // A press soon after a click (a press let go of without a drag) on the same row adds
            // it to the rack; any other press selects it for Enter and previews it, and may
            // start a drag. Why after a click and not after any press: a drag let go of over
            // nothing and tried again at once must stay a drag.
            let now = Instant::now();
            let double = self.browse.clicked.take().is_some_and(|(when, clicked)| {
                clicked == pick && now.duration_since(when) < DOUBLE_PRESS
            });
            if double {
                self.browse.chosen = None;
                self.stop_preview();
                self.add_picked(&pick);
                return Task::none();
            }
            self.browse.chosen = Some(pick.clone());
            self.browse.pressing = Some(pick.clone());
            // Its keys and the devices no link routes play it from now on; the external input
            // has no sound of its own to preview, so a preview playing ends.
            let result = if pick == Pick::ExternalInput {
                self.end_library_preview()
            } else {
                self.preview_in_library(&pick)
            };
            self.report(result);
        } else {
            // Any other press in the Library ends the preview.
            let result = self.end_library_preview();
            self.report(result);
        }
        match pick {
            Pick::Sound(index) if left => {
                if let Some((file, name)) = self.kit_file(index) {
                    self.toggle_preview(&file);
                    self.browse.press = Some(Press::new(Held::File(file), name, at));
                }
            }
            Pick::Plugin(plugin) if left => {
                let name = plugin.plugin.name.clone();
                self.browse.press = Some(Press::new(Held::Plugin(plugin), name, at));
            }
            Pick::Set(set) if left => {
                self.preview_set(&set.sfz);
                let held = Held::Instrument {
                    sfz: set.sfz,
                    name: set.name.clone(),
                };
                self.browse.press = Some(Press::new(held, set.name, at));
            }
            Pick::ExternalInput if left => {
                let name = crate::ui::library::EXTERNAL_INPUT.to_owned();
                self.browse.press = Some(Press::new(Held::Input, name, at));
            }
            Pick::Rescan => self.start_scan(true),
            // A press on a group's label folds or opens it, and keeps it for Enter, Left and
            // Right.
            Pick::Group(key) if left => {
                self.fold_group(&key, None);
                self.browse.chosen = Some(Pick::Group(key));
            }
            Pick::Device(port) if left => {
                self.browse.press = Some(Press::new(Held::Device(port.clone()), port, at));
            }
            Pick::Device(port) => {
                if self.session.project().channel_for_port(&port).is_some() {
                    self.browse.menu = Some(RowMenu::Unlink(port));
                }
            }
            Pick::Tick(port) => {
                self.settings.midi_inputs.toggle(&port, self.ports.names());
                self.inputs_changed();
            }
            // The right button on a kit sound, a set or the external input: it goes on the rack
            // and its instrument window opens, the window being a channel's.
            Pick::Sound(_) | Pick::Set(_) | Pick::ExternalInput => {
                let id = self.session.project().next_channel_id;
                self.add_picked(&pick);
                if self.session.project().channels.contains_key(&id) {
                    self.open_instrument(id);
                }
            }
            Pick::Plugin(_) | Pick::Group(_) => {}
        }
        Task::none()
    }

    /// Previews the set whose main file is `sfz` by the sound of its zone at middle C: one note
    /// at that zone's root, the file as it is, cut after [`SET_PREVIEW`]; a second press stops
    /// it as for a sound file. Nothing when the set cannot be read.
    fn preview_set(&mut self, sfz: &Path) {
        let folders = self.browse.model.user_folders().folders;
        let Some(file) = sets::root_sound(sfz, &folders) else {
            return;
        };
        self.toggle_preview(&file);
        if self.browse.model.previewing.is_some() {
            self.browse.preview_stop = Some(Instant::now() + SET_PREVIEW);
        }
    }

    /// Enter while the Library has the keys: the instrument row last pressed is added to the
    /// rack; with none, the Samples tree's key acts as before. Returns whether a row was added.
    pub(super) fn add_chosen(&mut self) -> bool {
        let Some(pick) = self.browse.chosen.take() else {
            return false;
        };
        self.stop_preview();
        self.add_picked(&pick);
        true
    }

    /// The starter kit's sound `index`: its file, after writing the kit into its folder where
    /// it is missing, and its name; `None`, with a notice, when it cannot be written.
    pub(super) fn kit_file(&mut self, index: usize) -> Option<(PathBuf, String)> {
        let sound = starter_kit::SOUNDS.get(index)?;
        let Some(dir) = self.host.starter_kit.clone() else {
            self.notice = Some("There is no folder to put the starter kit in".to_owned());
            return None;
        };
        if let Err(error) = starter_kit::install(&dir) {
            self.notice = Some(format!("The starter kit was not written: {error}"));
            return None;
        }
        Some((sound.path_in(&dir), sound.name.to_owned()))
    }

    /// A plugin instrument let go of: on a playlist lane, a new plugin channel and a clip of
    /// the rack's pattern at the floor-snapped tick there, one undo step, the new channel
    /// selected; on a channel's header, that channel's sound; elsewhere on the rack, a new
    /// plugin channel as the picker adds one; anywhere else, nothing.
    pub(super) fn drop_plugin(&mut self, plugin: &Offered) {
        if let (true, Some(channel)) = (self.browse.rack_hover, self.browse.drop_header) {
            let source = SoundSource::plugin(super::plugins::plugin_processor(plugin));
            self.replace_sound(channel, source, &plugin.plugin.name);
            return;
        }
        let project = self.session.project();
        if let Some((track, tick)) = self.arranger.drop_target(project) {
            let id = project.next_channel_id;
            let add = Edit::AddPluginChannel {
                id,
                name: plugin.plugin.name.clone(),
                processor: super::plugins::plugin_processor(plugin),
            };
            // The drop target is snapped already: placed at its tick exactly.
            let place = self.pattern.and_then(|pattern| {
                let index = clip_gesture::index_of(project, track)?;
                clip_gesture::place(project, pattern, index, tick, Ticks(1))
            });
            let edit = match place {
                Some(place) => Edit::Transaction(vec![add, place]),
                None => add,
            };
            self.commit(edit);
            if self.error.is_none() {
                self.select_channel(id);
            }
        } else if self.browse.rack_hover {
            self.add_plugin(PickerTarget::Instrument, plugin);
        }
    }

    /// An instrument set let go of: on a playlist lane, a new instrument channel and a clip of
    /// the rack's pattern at the floor-snapped tick there, one undo step; on a channel's header,
    /// that channel's sound; elsewhere on the rack, a new instrument channel; either way the
    /// channel selected. Anywhere else, nothing.
    pub(super) fn drop_instrument(&mut self, sfz: &Path, name: &str) {
        if let (true, Some(channel)) = (self.browse.rack_hover, self.browse.drop_header) {
            let source = SoundSource::instrument(self.sample_path(sfz));
            self.replace_sound(channel, source, name);
            return;
        }
        let (id, mut edits) = self.new_instrument_channel(sfz, name);
        let project = self.session.project();
        if let Some((track, tick)) = self.arranger.drop_target(project) {
            // The drop target is snapped already: placed at its tick exactly.
            let place = self.pattern.and_then(|pattern| {
                let index = clip_gesture::index_of(project, track)?;
                clip_gesture::place(project, pattern, index, tick, Ticks(1))
            });
            edits.extend(place);
        } else if !self.browse.rack_hover {
            return;
        }
        self.commit(one_step(edits));
        if self.error.is_none() {
            self.select_channel(id);
        }
    }

    /// Gives `channel` the sound `source`, whose name is `name`, one undo step, and selects it:
    /// what a Library sound, set or plugin let go of on its header does. It keeps its notes,
    /// routing, mute, volume, pan, MIDI input and the sound settings the new kind takes; it takes
    /// `name` unless the user renamed it ([`App::unrenamed_name`]), which keeps the user's. The
    /// sound it plays already changes nothing (no step, and a plugin keeps its state, which a
    /// fresh instance would reset); the channel is selected all the same.
    pub(super) fn replace_sound(&mut self, channel: ChannelId, source: SoundSource, name: &str) {
        let Some(shown) = self.session.project().channels.get(&channel) else {
            return;
        };
        if shown.source().same_sound(&source) {
            self.select_channel(channel);
            return;
        }
        let name = if shown.name == self.unrenamed_name(shown) {
            name.to_owned()
        } else {
            shown.name.clone()
        };
        let sound = shown.sound.kept_for(source.kind);
        self.commit(Edit::ReplaceChannelSound {
            channel,
            name,
            source: Box::new(source),
            sound,
        });
        if self.error.is_none() {
            self.select_channel(channel);
        }
    }

    /// The name `channel` has while nothing renamed it: what its sound names it when it is
    /// added (its sound's name, as the instrument window names it; "Sampler" for a sampler
    /// without one, as a new document's). Why compared against this rather than a flag kept
    /// in the project: channels renamed before such a flag existed would read as not renamed.
    pub(super) fn unrenamed_name(&self, channel: &Channel) -> String {
        let empty = channel.kind == ChannelKind::Sampler && channel.sample.as_os_str().is_empty();
        if empty {
            return crate::project_file::EMPTY_SAMPLER.to_owned();
        }
        self.sound_name(channel)
    }

    /// The new channel's id and the edits adding the set whose main file is `sfz` as an
    /// instrument channel called `name` at the end of the rack, with the sound settings the set
    /// is added with ([`sets::added_sound`]: the 808 basses cutting themselves), for one step.
    fn new_instrument_channel(&self, sfz: &Path, name: &str) -> (ChannelId, Vec<Edit>) {
        let id = self.session.project().next_channel_id;
        let mut edits = vec![Edit::AddInstrumentChannel {
            id,
            name: name.to_owned(),
            instrument: self.sample_path(sfz),
        }];
        let sound = sets::added_sound(sfz);
        if !sound.is_default() {
            edits.push(Edit::SetChannelSound { channel: id, sound });
        }
        (id, edits)
    }

    /// Adds the set whose main file is `sfz` to the end of the rack as an instrument channel
    /// called `name`, one undo step, and selects it: what a double press or Enter on its
    /// Library row and the "+ Instrument…" picker do.
    pub(super) fn add_set(&mut self, sfz: &Path, name: &str) {
        let (id, edits) = self.new_instrument_channel(sfz, name);
        self.add_and_select(id, one_step(edits));
    }

    /// Adds the starter kit's sound `index` to the end of the rack as a sampler channel named
    /// after it, writing the kit into its folder where it is missing, one undo step, and selects
    /// it; with no folder for the kit, says so.
    pub(super) fn add_kit_sound(&mut self, index: usize) {
        let Some((file, name)) = self.kit_file(index) else {
            return;
        };
        let id = self.session.project().next_channel_id;
        let add = Edit::AddSamplerChannel {
            id,
            name,
            sample: self.sample_path(&file),
        };
        self.add_and_select(id, add);
    }

    /// Commits `add`, which adds channel `id`, and selects that channel once it is there.
    fn add_and_select(&mut self, id: ChannelId, add: Edit) {
        self.picker = None;
        self.commit(add);
        if self.error.is_none() {
            self.select_channel(id);
        }
    }

    /// Adds what a Library row holds to the end of the rack: a kit sound, a set or a plugin
    /// instrument; anything else, nothing.
    pub(super) fn add_picked(&mut self, pick: &Pick) {
        match pick {
            Pick::Sound(index) => self.add_kit_sound(*index),
            Pick::Set(set) => self.add_set(&set.sfz, &set.name),
            Pick::Plugin(plugin) => self.add_plugin(PickerTarget::Instrument, plugin),
            Pick::ExternalInput => {
                self.add_input_channel();
            }
            Pick::Rescan | Pick::Device(_) | Pick::Tick(_) | Pick::Group(_) => {}
        }
    }

    /// Folds the Library's group `key` names (`Some(true)`), opens it (`Some(false)`), or does
    /// the other of what it is (`None`); the folds are saved, and the list keeps its rows in
    /// view.
    pub(super) fn fold_group(&mut self, key: &str, fold: Option<bool>) {
        match fold {
            Some(fold) => self.browse.library.fold(key, fold),
            None => self.browse.library.toggle_group(key),
        }
        self.browse.cache.clear();
        self.settings
            .set_library_folded(&self.browse.library.folded);
        self.save_settings();
        self.scroll_library(0);
    }

    /// Looks for the instrument sets again, in the background: the shipped ones (the data
    /// folder's, then the installed ones), with the shipped folders of sounds, and those in the
    /// folders added to the Samples section.
    pub(super) fn scan_sets(&mut self) -> Task<Message> {
        let shipped: Vec<PathBuf> = [&self.host.instruments, &self.host.installed_instruments]
            .into_iter()
            .flatten()
            .cloned()
            .collect();
        let folders = self.browse.model.user_folders().folders;
        let work = move || {
            let found = sets::scan_roots(&shipped, &folders);
            let kinds = sets::kinds(&found.sets.shipped);
            (found, kinds)
        };
        if !self.host.background {
            let (found, kinds) = work();
            return self.media_message(MediaMessage::SetsFound(found, kinds));
        }
        Task::perform(blocking(work), |found| match found {
            Some((found, kinds)) => Message::Media(MediaMessage::SetsFound(found, kinds)),
            // The look stopped; the sets listed stay as they were.
            None => Message::Media(MediaMessage::Tick),
        })
    }

    /// A MIDI device let go of over a rack row: that channel plays its notes from now on (a
    /// channel playing it before lets it go), one undo step. The link names the port without
    /// the numbers the system appends on Linux, which change from one session to the next. A
    /// device whose messages were kept out is let in, since a link to it would otherwise play
    /// nothing.
    pub(super) fn drop_device(&mut self, port: &str) {
        let Some(channel) = self.browse.drop_channel else {
            return;
        };
        self.commit(Edit::SetChannelMidiInput {
            channel,
            port: Some(stable_port_name(port).to_owned()),
        });
        if self.error.is_none() && !self.settings.midi_inputs.plays(port) {
            self.settings.midi_inputs.toggle(port, self.ports.names());
            self.inputs_changed();
        }
    }

    /// The device row's Unlink: the channel playing `port` lets it go, one undo step.
    pub(super) fn unlink_device(&mut self, port: &str) {
        if let Some(channel) = self.session.project().channel_for_port(port) {
            self.commit(Edit::SetChannelMidiInput {
                channel,
                port: None,
            });
        }
    }
}

/// `edits` as one undo step: the edit alone, else a transaction of them in order.
fn one_step(mut edits: Vec<Edit>) -> Edit {
    if edits.len() == 1 {
        edits.remove(0)
    } else {
        Edit::Transaction(edits)
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use dawsome_engine::project::{ChannelId, ChannelKind, Project};
    use dawsome_engine::realtime::{EngineEvent, Player};
    use dawsome_host_clap::{Catalogue, ClapHost};
    use iced::widget::canvas::{self, Program as _};
    use iced::{Rectangle, Size};

    use super::super::tests::app_hosted;
    use super::super::widget_events::Window;
    use super::*;
    use crate::document::Host;
    use crate::midi::{ControllerQueue, MidiPorts, PortSource, Router};
    use crate::project_file;
    use crate::settings::MidiInputs;
    use crate::ui::arranger::PlaylistMessage;
    use crate::ui::browser::{self, BrowserMessage, Nav, PanelState};
    use crate::ui::layout::{self, TOOLBAR_HEIGHT};
    use crate::ui::library::{Port, Section};
    use crate::ui::rack::DeviceDrop;
    use crate::ui::settings_layout;
    use crate::ui::settings_panel::SettingsMessage;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    fn fixture(name: &str) -> PathBuf {
        Path::new(FIXTURES).join(name)
    }

    /// The figures tests/fixtures/gen_p10_expected.py wrote.
    fn p10_expected() -> serde_json::Value {
        let text = std::fs::read_to_string(fixture("p10_expected.json")).expect("p10_expected");
        serde_json::from_str(&text).expect("parses")
    }

    /// A known answer's document, as the generator wrote it.
    fn expected_document(name: &serde_json::Value) -> String {
        let name = name.as_str().expect("a file name");
        let text = std::fs::read_to_string(fixture(name)).expect("the expected document");
        text.trim_end().to_owned()
    }

    fn document(app: &App) -> String {
        app.session.project().to_json().expect("serializes")
    }

    fn temp_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("dawsome-app-library-{}", std::process::id()))
            .join(test);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// The app on the project at `path`, the playlist and the Library open.
    fn library_app(path: &Path, host: Host) -> (App, Player) {
        let project: Project = project_file::open(path).expect("opens").project;
        let (mut app, player) = app_hosted(project, path, host);
        // 1000 × 576: the room under the toolbar these tests' rows were placed in, 504 px, as
        // the 1000 × 560 window gave it under the 56 px toolbar (72 px now).
        let _ = app.update(Message::WindowResized(Size::new(1000.0, 576.0)));
        let _ = app.update(Message::TogglePanel(Panel::Playlist));
        let _ = app.update(Message::TogglePanel(Panel::Browser));
        (app, player)
    }

    /// tests/fixtures/p7_catalogue.json: instruments Analog Lead and Bell Keys by Example and
    /// Dawsome Test Sine by Dawsome.
    fn catalogue_host() -> Host {
        let catalogue = Catalogue::load(&fixture("p7_catalogue.json")).expect("loads");
        Host {
            clap: ClapHost::new(catalogue),
            ..Host::offline()
        }
    }

    /// What the Library's canvas sends for `event` with the cursor at window point `at`.
    fn library_event(app: &App, event: canvas::Event, at: Point) -> Option<Message> {
        let height = app.window_size.height - TOOLBAR_HEIGHT;
        let bounds = Rectangle::new(
            Point::new(0.0, TOOLBAR_HEIGHT),
            Size::new(browser::WIDTH, height),
        );
        let cursor = mouse::Cursor::Available(at);
        let panel = app.library_panel();
        let action = panel.update(&mut PanelState::default(), &event, bounds, cursor)?;
        action.into_inner().0
    }

    /// Where the menu under a Library row is laid out on the window's top layer, window
    /// coordinates: the one menu open.
    fn row_menu(app: &App) -> Rectangle {
        let menus = Window::new(app).overlays(app);
        let [menu] = menus[..] else {
            panic!("one menu on the window's top layer: {menus:?}");
        };
        menu
    }

    /// What the menus on the window's top layer send for a click at window point `at`.
    fn menu_click(app: &App, at: Point) -> Option<Message> {
        let mut published = Window::new(app).click_overlay(app, at);
        assert!(published.len() <= 1, "one message at most: {published:?}");
        published.pop()
    }

    fn send(app: &mut App, message: Option<Message>) {
        let _ = app.update(message.expect("the canvas sends a message"));
    }

    /// The window point at the middle of the Library row showing `item`.
    fn row_center(app: &App, item: &Item) -> Point {
        let row = app
            .library_items()
            .iter()
            .position(|shown| shown == item)
            .unwrap_or_else(|| panic!("{item:?} is listed"));
        let rect = browser::row_rect(row, app.browse.library.top).expect("in view");
        Point::new(rect.center_x(), rect.center_y() + TOOLBAR_HEIGHT)
    }

    fn press(app: &mut App, at: Point, button: mouse::Button) {
        let event = canvas::Event::Mouse(mouse::Event::ButtonPressed(button));
        let message = library_event(app, event, at);
        send(app, message);
    }

    /// A press on `item`'s row and a move to window point `to`, through the Library's canvas.
    fn hold_and_move(app: &mut App, item: &Item, to: Point) {
        let at = row_center(app, item);
        press(app, at, mouse::Button::Left);
        let moved = canvas::Event::Mouse(mouse::Event::CursorMoved { position: to });
        let message = library_event(app, moved, to);
        send(app, message);
    }

    /// The release of the press held, through the Library's canvas.
    fn release(app: &mut App, at: Point) {
        let event = canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left));
        let message = library_event(app, event, at);
        send(app, message);
    }

    /// Where a drop lands on track `index` (0 for track 1) at bar `bar`: a little past the
    /// bar's first tick, in the tracks canvas.
    fn lane_point(app: &App, index: usize, bar: u32) -> Point {
        let tick = Ticks(i64::from(bar - 1) * 384);
        let grid = &app.arranger.grid;
        Point::new(grid.x_of(tick) + 4.0, grid.row_top(index) + 10.0)
    }

    fn hover_lane(app: &mut App, at: Point) {
        let hover = PlaylistMessage::BrowserHover(Some(at));
        let _ = app.update(Message::Playlist(hover));
    }

    // Known answer (1a, gen_p10_expected.py): the kit's first sound, "Kick 808", pressed in the
    // starter kit, dragged onto track 2 at bar 3 and let go writes the kit into its folder and
    // gives exactly p10_after_kit.json: a new audio channel "Kick 808" and one clip of the whole
    // file at the project's rate from tick 768, 281 ticks long. One undo gives p10_base back.
    #[test]
    fn p10_the_kits_kick_dropped_on_track_2_at_bar_3_is_the_known_document() {
        let want = &p10_expected()["kit_drop"];
        let dir = temp_dir("kit_drop");
        for name in ["p10_base.json", "impulse.wav"] {
            std::fs::copy(fixture(name), dir.join(name)).expect("copies");
        }
        let host = Host {
            starter_kit: Some(dir.join("starter-kit")),
            ..Host::offline()
        };
        let (mut app, _player) = library_app(&dir.join("p10_base.json"), host);
        let before = document(&app);
        let file = dir
            .join("starter-kit")
            .join(want["file"].as_str().expect("the sound's file"));
        hold_and_move(&mut app, &Item::Sound(0), Point::new(500.0, 200.0));
        assert_eq!(
            app.browse.model.previewing,
            Some(file.clone()),
            "the press previews the sound"
        );
        let index = usize::try_from(want["track"].as_u64().expect("track") - 1).expect("fits");
        let bar = u32::try_from(want["bar"].as_u64().expect("bar")).expect("fits");
        let lane = lane_point(&app, index, bar);
        hover_lane(&mut app, lane);
        release(&mut app, Point::new(500.0, 200.0));
        assert_eq!(document(&app), expected_document(&want["document"]));
        assert!(file.is_file(), "the kit is written");
        assert_eq!(app.session.history().undo_len(), 1);
        let _ = app.update(Message::Undo);
        assert_eq!(document(&app), before);
    }

    // Known answer (1b): the plugin "Bell Keys" dragged onto track 1 at bar 2 gives exactly
    // p10_after_plugin.json: a new plugin channel and a clip of the rack's pattern from tick
    // 384, one bar long. The new channel is selected: the piano roll shows it and live notes
    // play it. One undo step takes both away.
    #[test]
    fn p10_bell_keys_dropped_on_track_1_at_bar_2_is_the_known_document() {
        let want = &p10_expected()["plugin_drop"];
        let (mut app, mut player) = library_app(&fixture("p10_base.json"), catalogue_host());
        let before = document(&app);
        let catalogue = Catalogue::load(&fixture("p7_catalogue.json")).expect("loads");
        let bells = catalogue
            .offered(PluginKind::Instrument)
            .into_iter()
            .find(|plugin| plugin.plugin.name == want["plugin"])
            .expect("listed");
        hold_and_move(&mut app, &Item::Plugin(bells), Point::new(500.0, 200.0));
        let index = usize::try_from(want["track"].as_u64().expect("track") - 1).expect("fits");
        let bar = u32::try_from(want["bar"].as_u64().expect("bar")).expect("fits");
        let lane = lane_point(&app, index, bar);
        hover_lane(&mut app, lane);
        release(&mut app, Point::new(500.0, 200.0));
        assert_eq!(document(&app), expected_document(&want["document"]));
        let selected = u32::try_from(want["selected_channel"].as_u64().expect("id")).expect("fits");
        assert_eq!(app.selected_channel, Some(ChannelId(selected)));
        // The player takes the new plans first, as the stream's calls do: one per call.
        for _ in 0..4 {
            player.process(&mut [[0.0; 2]; 64]);
        }
        let router = Router::new(app.session.midi_inbox(), ControllerQueue::default());
        router.deliver(&[0x90, 60, 64]);
        player.process(&mut [[0.0; 2]; 64]);
        assert!(
            matches!(
                app.session.take_events()[..],
                [EngineEvent::LiveNoteStarted { channel, .. }] if channel == ChannelId(selected)
            ),
            "live notes play the new channel"
        );
        assert_eq!(app.session.history().undo_len(), 1);
        let _ = app.update(Message::Undo);
        assert_eq!(document(&app), before);
    }

    // A plugin instrument let go of over the rack is a new plugin channel, as the picker adds
    // one: named after it, selected, no clip; let go of anywhere else it adds nothing.
    #[test]
    fn a_plugin_dropped_on_the_rack_is_a_plugin_channel_and_elsewhere_nothing() {
        let (mut app, _player) = library_app(&fixture("p10_base.json"), catalogue_host());
        let before = document(&app);
        let sine = app
            .library_items()
            .into_iter()
            .find(|item| matches!(item, Item::Plugin(plugin) if plugin.plugin.name == "Dawsome Test Sine"))
            .expect("listed");
        hold_and_move(&mut app, &sine, Point::new(600.0, 500.0));
        release(&mut app, Point::new(600.0, 500.0));
        assert_eq!(document(&app), before, "let go of over nothing");
        hold_and_move(&mut app, &sine, Point::new(400.0, 100.0));
        let _ = app.update(Message::Media(super::super::MediaMessage::RackHover(true)));
        release(&mut app, Point::new(400.0, 100.0));
        let project = app.session.project();
        let channel = &project.channels[&ChannelId(2)];
        assert_eq!(
            (channel.name.as_str(), channel.kind),
            ("Dawsome Test Sine", ChannelKind::Plugin)
        );
        assert!(project.playlist.clips.is_empty(), "no clip");
        assert_eq!(app.selected_channel, Some(ChannelId(2)));
    }

    // A plugin instrument let go of over the rack is selected as a click on its name selects
    // it, so live notes, from a device and from the typing keyboard, play it.
    #[test]
    fn a_plugin_dropped_on_the_rack_takes_the_live_notes() {
        let (mut app, mut player) = library_app(&fixture("p10_base.json"), catalogue_host());
        let sine = app
            .library_items()
            .into_iter()
            .find(|item| matches!(item, Item::Plugin(plugin) if plugin.plugin.name == "Dawsome Test Sine"))
            .expect("listed");
        hold_and_move(&mut app, &sine, Point::new(400.0, 100.0));
        let _ = app.update(Message::Media(super::super::MediaMessage::RackHover(true)));
        release(&mut app, Point::new(400.0, 100.0));
        assert_eq!(app.selected_channel, Some(ChannelId(2)));
        assert_eq!(
            super::super::tests::live_note_channels(&mut app, &mut player),
            [ChannelId(2), ChannelId(2)]
        );
    }

    // Undoing a drop that added a channel and selected it (a plugin or a set let go of on a
    // track) takes the channel away and selects the first one again, so live notes play it
    // rather than nothing.
    #[test]
    fn undoing_a_track_drop_hands_the_selection_back_to_the_first_channel() {
        let (mut app, mut player) = library_app(&fixture("p10_base.json"), catalogue_host());
        let bells = app
            .library_items()
            .into_iter()
            .find(|item| matches!(item, Item::Plugin(plugin) if plugin.plugin.name == "Bell Keys"))
            .expect("listed");
        let (mut set_app, mut set_player, _dir) = set_app("set_drop_undo");
        let pad = set_item(&set_app, "Pad");
        for (app, player, held) in [
            (&mut app, &mut player, bells),
            (&mut set_app, &mut set_player, pad),
        ] {
            hold_and_move(app, &held, Point::new(500.0, 200.0));
            let lane = lane_point(app, 0, 2);
            hover_lane(app, lane);
            release(app, Point::new(500.0, 200.0));
            assert_eq!(app.selected_channel, Some(ChannelId(2)), "{held:?}");
            assert_eq!(
                super::super::tests::live_note_channels(app, player),
                [ChannelId(2), ChannelId(2)],
                "{held:?}"
            );
            let _ = app.update(Message::Undo);
            assert_eq!(app.error, None, "{held:?}");
            assert_eq!(app.selected_channel, Some(ChannelId(1)), "{held:?}");
            assert_eq!(
                super::super::tests::live_note_channels(app, player),
                [ChannelId(1), ChannelId(1)],
                "{held:?}: live notes play the first channel"
            );
        }
    }

    /// p10_link ("kick" first, "keys" playing the linked port) with the given ports listed, the
    /// Library open with its Instruments closed, so the devices show in the window's 560 px;
    /// no port is ever opened.
    fn device_app(ports: &[&str]) -> (App, Player) {
        let (mut app, player) = library_app(&fixture("p10_link.json"), Host::offline());
        let names = ports.iter().map(|name| (*name).to_owned()).collect();
        app.ports = MidiPorts::new(PortSource::Given(names));
        let instruments = app.library_items()[0].clone();
        let at = row_center(&app, &instruments);
        press(&mut app, at, mouse::Button::Left);
        (app, player)
    }

    /// The Library's row of device `name`.
    fn device_row(app: &App, name: &str) -> crate::ui::library::Port {
        match device(app, name) {
            Item::Port(port) => port,
            other => panic!("{other:?} is no device"),
        }
    }

    // Each device row says where its notes go. p10_link ("kick" selected,
    // "keys" linked to "USB Keyboard MIDI 1") with the Oriole, Port B and the system loopback
    // listed and the Oriole ticked: the Oriole's notes go to the selected channel, "selected:
    // kick"; Port B, not ticked, says nothing; the loopback lists last. With no channel
    // selected, "nothing selected" until a port is let in, which selects the first channel with
    // a sound. Why a channel plays nothing is said for an audio channel and an empty sampler.
    #[test]
    fn a_device_row_says_where_its_notes_go() {
        let oriole = "Oriole:Oriole MIDI 1 28:0";
        let through = "Midi Through:Midi Through Port-0 14:0";
        let (mut app, _player) = device_app(&[through, oriole, "Port B"]);
        app.settings.midi_inputs.toggle(oriole, app.ports.names());
        app.inputs_changed();
        let row = device_row(&app, oriole);
        assert_eq!(
            (row.goes.as_deref(), row.silent.as_deref(), row.loopback),
            (Some("selected: kick"), None, false)
        );
        assert_eq!(device_row(&app, "Port B").goes, None, "not ticked");
        let ports: Vec<String> = app
            .library_items()
            .into_iter()
            .filter_map(|item| match item {
                Item::Port(port) => Some(port.name),
                _ => None,
            })
            .collect();
        assert_eq!(ports.last().map(String::as_str), Some(through));
        assert!(device_row(&app, through).loopback);
        let words = crate::ui::hint::words(
            &crate::ui::hint::Hover::Library(crate::ui::hint::LibraryPart::Device(row)),
            &app.hint_context(),
        );
        assert!(words.contains("\u{2192} selected: kick"), "{words}");

        app.selected_channel = None;
        assert_eq!(
            device_row(&app, oriole).goes.as_deref(),
            Some("nothing selected")
        );
        app.settings.midi_inputs.toggle("Port B", app.ports.names());
        app.inputs_changed();
        assert_eq!(
            app.selected_channel,
            Some(ChannelId(1)),
            "the first with a sound"
        );

        let audio = Channel::audio("Vocal take", "take.wav");
        let empty = Channel::sampler("Sampler", "");
        assert_eq!(
            super::super::inputs::silent_reason(&audio).as_deref(),
            Some("Vocal take is an audio channel: it plays its clips, not notes")
        );
        assert_eq!(
            super::super::inputs::silent_reason(&empty).as_deref(),
            Some("Sampler has no sound yet")
        );
        assert_eq!(
            super::super::inputs::silent_reason(&Channel::sampler("kick", "impulse.wav")),
            None
        );
    }

    fn device(app: &App, name: &str) -> Item {
        app.library_items()
            .into_iter()
            .find(|item| matches!(item, Item::Port(port) if port.name == name))
            .unwrap_or_else(|| panic!("{name} is listed"))
    }

    /// What the device drop target over the rack sends with the cursor on rack row `row`.
    fn over_row(app: &App, row: usize) -> Option<Message> {
        let project = app.session.project();
        let size = layout::rack_canvas_size(project.channels.len(), 16);
        let target = DeviceDrop {
            project,
            over: app.browse.drop_channel,
            rows: app.rack_rows(),
        };
        let at = Point::new(60.0, layout::row_top(row) + 10.0);
        let moved = canvas::Event::Mouse(mouse::Event::CursorMoved { position: at });
        let bounds = Rectangle::new(Point::ORIGIN, size);
        let action = target.update(&mut (), &moved, bounds, mouse::Cursor::Available(at))?;
        action.into_inner().0
    }

    // "Port B" dragged onto "kick"'s rack row: kick plays it from now on, one undo step, and
    // its messages, kept out until then, are let in. Dragged onto "keys", it moves there and
    // "keys" lets its first port go (one port per channel), so that row loses its arrow. The
    // linked row's right-button menu unlinks it.
    #[test]
    fn a_device_dropped_on_a_rack_row_plays_that_channel_and_its_menu_unlinks_it() {
        let linked = p10_expected()["link"]["port"]
            .as_str()
            .expect("port")
            .to_owned();
        let (mut app, _player) = device_app(&[&linked, "Port B"]);
        let before = document(&app);
        assert!(!app.settings.midi_inputs.plays("Port B"));
        let port_b = device(&app, "Port B");
        hold_and_move(&mut app, &port_b, Point::new(400.0, 100.0));
        let over = over_row(&app, 0);
        send(&mut app, over);
        assert_eq!(app.browse.drop_channel, Some(ChannelId(1)));
        assert!(over_row(&app, 0).is_none(), "said once");
        release(&mut app, Point::new(400.0, 100.0));
        let project = app.session.project();
        assert_eq!(project.channel_for_port("Port B"), Some(ChannelId(1)));
        assert_eq!(project.channel_for_port(&linked), Some(ChannelId(2)));
        assert!(app.settings.midi_inputs.plays("Port B"), "let in");
        assert_eq!(
            device(&app, "Port B"),
            Item::Port(Port {
                name: "Port B".to_owned(),
                plays: true,
                channel: Some("kick".to_owned()),
                ..Port::default()
            })
        );
        assert_eq!(app.session.history().undo_len(), 1);
        let port_b = device(&app, "Port B");
        hold_and_move(&mut app, &port_b, Point::new(400.0, 140.0));
        let over = over_row(&app, 1);
        send(&mut app, over);
        release(&mut app, Point::new(400.0, 140.0));
        let project = app.session.project();
        assert_eq!(project.channel_for_port("Port B"), Some(ChannelId(2)));
        assert_eq!(project.channel_for_port(&linked), None, "keys let it go");
        assert!(matches!(
            device(&app, &linked),
            Item::Port(Port { channel: None, .. })
        ));
        let at = row_center(&app, &device(&app, "Port B"));
        press(&mut app, at, mouse::Button::Right);
        let chosen = row_menu(&app).center();
        assert!(matches!(
            menu_click(&app, chosen),
            Some(Message::Browser(BrowserMessage::UnlinkChosen))
        ));
        let _ = app.update(Message::Browser(BrowserMessage::UnlinkChosen));
        assert_eq!(app.session.project().channel_for_port("Port B"), None);
        for _ in 0..3 {
            let _ = app.update(Message::Undo);
        }
        assert_eq!(document(&app), before);
    }

    // The menu under a row
    // is drawn on the window's top layer, so the panel's foot never cuts it. The linked device's
    // row made the Library's last whole row, the hint bar on: its Unlink menu, one row of 26 px
    // on 4 px of ground, 34 px tall, would pass the bar's top from the row's foot, so it flips
    // up and ends there; its row, clicked, unlinks. In a window one row shorter the row is out
    // of view, and so is its menu, still open.
    #[test]
    fn the_row_menu_shows_whole_at_the_panels_foot() {
        let linked = p10_expected()["link"]["port"]
            .as_str()
            .expect("port")
            .to_owned();
        let (mut app, _player) = device_app(&[&linked, "Port B"]);
        app.hints_flag = Some(true);
        let row = app
            .library_items()
            .iter()
            .position(|item| *item == device(&app, &linked))
            .expect("listed");
        // The window y where rows 0 to `row` end: the panel's foot, the hint bar under it.
        let foot = TOOLBAR_HEIGHT + browser::ROWS_TOP + browser::ROW_HEIGHT * (row + 1) as f32;
        let window = |height: f32| Message::WindowResized(Size::new(1000.0, height));
        let _ = app.update(window(foot + layout::HINT_BAR_HEIGHT));
        let at = row_center(&app, &device(&app, &linked));
        press(&mut app, at, mouse::Button::Right);
        let menu = row_menu(&app);
        assert_eq!(menu.height, 34.0);
        assert_eq!(
            menu.y + menu.height,
            foot,
            "flipped up, ending at the row's foot"
        );
        assert!(matches!(
            menu_click(&app, menu.center()),
            Some(Message::Browser(BrowserMessage::UnlinkChosen))
        ));
        let _ = app.update(window(foot + layout::HINT_BAR_HEIGHT - browser::ROW_HEIGHT));
        assert!(app.browse.menu.is_some());
        let menus = Window::new(&app).overlays(&app);
        assert!(
            menus.is_empty(),
            "no menu under a row out of view: {menus:?}"
        );
    }

    // A file whose audio channel "keys" holds the link to a port opens with that link left
    // out, since no edit could put it back. The port, listed, dragged onto "kick" links it
    // there, and Ctrl+Z gives the opened document back with no failure; before, the undo was
    // refused (it put the link back on the audio channel) and stayed refused.
    #[test]
    fn a_device_linked_after_a_link_was_left_out_undoes() {
        let linked = p10_expected()["link"]["port"]
            .as_str()
            .expect("port")
            .to_owned();
        let dir = temp_dir("left_out_undo");
        let text = std::fs::read_to_string(fixture("p10_link.json")).expect("reads");
        let audio = text.replace(
            r#""kind":"sampler","sample":"ramp4.wav""#,
            r#""kind":"audio","sample":"ramp4.wav""#,
        );
        assert_ne!(audio, text);
        let path = dir.join("p10_link.json");
        std::fs::write(&path, audio).expect("writes");
        std::fs::copy(fixture("impulse.wav"), dir.join("impulse.wav")).expect("copies");
        let (mut app, _player) = library_app(&path, Host::offline());
        app.ports = MidiPorts::new(PortSource::Given(vec![linked.clone()]));
        let instruments = app.library_items()[0].clone();
        let at = row_center(&app, &instruments);
        press(&mut app, at, mouse::Button::Left);
        let before = document(&app);
        let port = device(&app, &linked);
        hold_and_move(&mut app, &port, Point::new(400.0, 100.0));
        let over = over_row(&app, 0);
        send(&mut app, over);
        release(&mut app, Point::new(400.0, 100.0));
        assert_eq!(
            app.session.project().channel_for_port(&linked),
            Some(ChannelId(1))
        );
        let _ = app.update(Message::Undo);
        assert_eq!(app.error, None, "the undo applies");
        assert_eq!(document(&app), before);
        assert_eq!(app.session.history().undo_len(), 0);
    }

    // On Linux a port's name ends in the system's client and port numbers. A link saved under
    // former numbers shows on the renumbered port's row ("keys"), not as an unplugged device;
    // another device dropped on "kick" links it by its name without the numbers, and its tick,
    // switched on by the drop, is stored so too.
    #[test]
    fn a_devices_link_and_tick_are_kept_by_its_name_without_the_systems_numbers() {
        let dir = temp_dir("alsa_names");
        std::fs::copy(fixture("p10_link_alsa.json"), dir.join("p10.json")).expect("copies");
        std::fs::copy(fixture("impulse.wav"), dir.join("impulse.wav")).expect("copies");
        let (mut app, _player) = library_app(&dir.join("p10.json"), Host::offline());
        let keyboard = "USB Keyboard:USB Keyboard MIDI 1 36:0";
        let pads = "Pads:Pads MIDI 1 40:0";
        app.ports = MidiPorts::new(PortSource::Given(vec![
            keyboard.to_owned(),
            pads.to_owned(),
        ]));
        let instruments = app.library_items()[0].clone();
        let at = row_center(&app, &instruments);
        press(&mut app, at, mouse::Button::Left);
        assert!(matches!(
            device(&app, keyboard),
            Item::Port(Port { channel: Some(ref name), unplugged: false, .. }) if name == "keys"
        ));
        assert!(
            !app.library_items().iter().any(|item| matches!(
                item,
                Item::Port(Port {
                    unplugged: true,
                    ..
                })
            )),
            "no unplugged row"
        );
        let held = device(&app, pads);
        hold_and_move(&mut app, &held, Point::new(400.0, 100.0));
        let over = over_row(&app, 0);
        send(&mut app, over);
        release(&mut app, Point::new(400.0, 100.0));
        let project = app.session.project();
        assert_eq!(
            project.channels[&ChannelId(1)].midi_input.as_deref(),
            Some("Pads:Pads MIDI 1")
        );
        assert_eq!(project.channel_for_port(pads), Some(ChannelId(1)));
        assert_eq!(
            app.settings.midi_inputs,
            MidiInputs::Named(["Pads:Pads MIDI 1".to_owned()].into())
        );
        assert!(app.settings.midi_inputs.plays(pads));
    }

    // The Library's ticks are the Settings panel's MIDI input boxes: switched in one, shown in
    // the other.
    #[test]
    fn the_librarys_ticks_and_the_settings_panels_show_one_choice() {
        let (mut app, _player) = device_app(&["Port A", "Port B"]);
        let tick = |app: &App, name: &str| {
            let row = row_center(app, &device(app, name));
            Point::new(browser::ROW_LEFT + browser::TICK_SIZE / 2.0, row.y)
        };
        let at = tick(&app, "Port A");
        press(&mut app, at, mouse::Button::Left);
        assert_eq!(
            app.settings.midi_inputs,
            MidiInputs::Named(["Port A".to_owned()].into())
        );
        let layout = settings_layout::Layout { ports: 2, notes: 0 };
        let box_a = layout.checkbox(1).center();
        let _ = app.update(Message::Settings(SettingsMessage::Pressed(box_a)));
        assert!(matches!(
            device(&app, "Port A"),
            Item::Port(Port { plays: false, .. })
        ));
        let name = row_center(&app, &device(&app, "Port B"));
        press(&mut app, name, mouse::Button::Left);
        assert!(
            !app.settings.midi_inputs.plays("Port B"),
            "a press on the name only holds the device"
        );
    }

    // A press on a section's header closes it, its count staying; a second opens it again.
    #[test]
    fn a_sections_header_closes_and_opens_it() {
        let (mut app, _player) = library_app(&fixture("p10_base.json"), Host::offline());
        let header = app.library_items()[0].clone();
        assert!(matches!(
            header,
            Item::Header {
                section: Section::Instruments,
                count: 12,
                open: true
            }
        ));
        let at = row_center(&app, &header);
        press(&mut app, at, mouse::Button::Left);
        let items = app.library_items();
        assert!(matches!(
            items[..2],
            [
                Item::Header {
                    section: Section::Instruments,
                    count: 12,
                    open: false
                },
                Item::Header {
                    section: Section::Samples,
                    ..
                }
            ]
        ));
        let at = row_center(&app, &items[0]);
        press(&mut app, at, mouse::Button::Left);
        assert!(matches!(app.library_items()[1], Item::Group("Starter kit")));
    }

    // Walking the Samples tree with the arrow keys keeps the selected row in view, now that the
    // tree sits under the Instruments section: in a 416 px window 13 rows show, and the tree's
    // first row is list row 23 (12 kit sounds, 4 hint lines, Rescan and the external input
    // under its group come before it; the plugins' hint has said where VST3 plugins go since
    // VST3 reached the app, one line more).
    // (Pin moved: a 400 px window with the 56 px toolbar; 416 keeps the same
    // room under the 72 px one.)
    #[test]
    fn walking_the_tree_keeps_the_selected_row_in_view() {
        let host = Host {
            browser_root: Some(fixture("browser")),
            ..Host::offline()
        };
        let (mut app, _player) = library_app(&fixture("p10_base.json"), host);
        app.window_size = Size::new(1000.0, 416.0);
        let visible = browser::visible_rows(416.0 - TOOLBAR_HEIGHT);
        assert_eq!(visible, 13);
        let row_of = |app: &App, tree| {
            app.library_items()
                .iter()
                .position(|item| *item == Item::Tree(tree))
                .expect("listed")
        };
        // On Linux; Windows' plugin hint is a line longer (its folders take two lines), so
        // every row under it lies one further down.
        let shift = crate::ui::library::PLUGIN_HINT.len() - 4;
        assert_eq!(row_of(&app, 0), 23 + shift);
        for selected in 0..3 {
            let _ = app.update(Message::Browser(BrowserMessage::Key(Nav::Down)));
            assert_eq!(app.browse.model.selected, Some(selected));
            let row = row_of(&app, selected);
            let top = app.browse.library.top;
            assert!((top..top + visible).contains(&row), "row {row} from {top}");
        }
        assert_eq!(
            app.browse.library.top,
            13 + shift,
            "list row 25 is the last shown"
        );
        for _ in 0..3 {
            let _ = app.update(Message::Browser(BrowserMessage::Key(Nav::Up)));
        }
        assert_eq!(app.browse.model.selected, Some(0));
        assert_eq!(
            app.browse.library.top,
            13 + shift,
            "row 23 is still in view"
        );
    }

    // With no folder for the starter kit (a scripted run's), a press on a sound says so and
    // holds nothing.
    #[test]
    fn a_kit_sound_without_a_folder_to_go_in_says_so() {
        let (mut app, _player) = library_app(&fixture("p10_base.json"), Host::offline());
        let at = row_center(&app, &Item::Sound(1));
        press(&mut app, at, mouse::Button::Left);
        assert_eq!(
            app.notice.as_deref(),
            Some("There is no folder to put the starter kit in")
        );
        assert!(app.browse.press.is_none());
        assert_eq!(app.browse.model.previewing, None);
    }

    /// What the sound target over the rack sends with the cursor on rack point `at`.
    fn over_header(app: &App, at: Point) -> Option<Message> {
        let project = app.session.project();
        let size = layout::rack_canvas_size(project.channels.len(), 16);
        let target = crate::ui::rack::SoundDrop {
            project,
            over: app.browse.drop_header,
            rows: app.rack_rows(),
        };
        let moved = canvas::Event::Mouse(mouse::Event::CursorMoved { position: at });
        let bounds = Rectangle::new(Point::ORIGIN, size);
        let action = target.update(&mut (), &moved, bounds, mouse::Cursor::Available(at))?;
        action.into_inner().0
    }

    /// p15_base copied into a folder of its own with the pad set in its sets/ folder and a
    /// folder for the starter kit, the catalogue's plugins listed: the app on it, the playlist
    /// and the Library open.
    fn p15_app(test: &str) -> (App, Player, PathBuf) {
        p15_app_on("p15_base.json", test)
    }

    /// The fixture `base` set up as [`p15_app`] sets up p15_base.
    fn p15_app_on(base: &str, test: &str) -> (App, Player, PathBuf) {
        let dir = temp_dir(test);
        for name in [base, "impulse.wav"] {
            std::fs::copy(fixture(name), dir.join(name)).expect("copies");
        }
        copy_dir(&fixture("instruments/pad"), &dir.join("sets/pad"));
        let host = Host {
            instruments: Some(dir.join("sets")),
            starter_kit: Some(dir.join("starter-kit")),
            ..catalogue_host()
        };
        let (mut app, player) = library_app(&dir.join(base), host);
        // Tall enough for the Library to show the plugins under the kit and the set.
        let _ = app.update(Message::WindowResized(Size::new(1000.0, 900.0)));
        (app, player, dir)
    }

    /// The Library's row of what a p15 known answer says was dropped.
    fn dropped_item(app: &App, answer: &serde_json::Value) -> Item {
        match answer["dropped"].as_str().expect("what was dropped") {
            "set" => set_item(app, answer["name"].as_str().expect("its name")),
            "kit" => Item::Sound(0),
            "plugin" => app
                .library_items()
                .into_iter()
                .find(|item| matches!(item, Item::Plugin(plugin) if plugin.plugin.name == answer["name"]))
                .expect("listed"),
            other => panic!("{other}"),
        }
    }

    /// `item` held from the Library and let go of on channel `id`'s header.
    fn drop_on_header(app: &mut App, item: &Item, id: u32) {
        hold_and_move(app, item, Point::new(500.0, 200.0));
        let _ = app.update(Message::Media(super::super::MediaMessage::RackHover(true)));
        let header = layout::channel_header_rect(id as usize - 1).center();
        let over = over_header(app, header);
        send(app, over);
        assert_eq!(app.browse.drop_header, Some(ChannelId(id)));
        release(app, Point::new(500.0, 200.0));
    }

    // Through the Library's drop path: the tied
    // known answers (gen_p15_expected.py). A plugin let go of on a plugin channel's header, or a
    // set, gives exactly the case's document: the automations and controller links of the old
    // plugin's parameters go with it in the one step, whose undo gives the base back.
    #[test]
    fn p15_a_plugin_replaced_from_the_library_takes_its_parameters_ties() {
        let want = serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(fixture("p15_tied_expected.json")).expect("p15_tied"),
        )
        .expect("parses");
        for (case, answer) in want["cases"].as_object().expect("cases") {
            let (mut app, _player, _dir) =
                p15_app_on("p15_tied_base.json", &format!("p15_tied_{case}"));
            let before = document(&app);
            let held = dropped_item(&app, answer);
            let id = u32::try_from(answer["channel"].as_u64().expect("id")).expect("fits");
            drop_on_header(&mut app, &held, id);
            assert_eq!(
                document(&app),
                expected_document(&answer["document"]),
                "{case}"
            );
            assert_eq!(app.session.history().undo_len(), 1, "{case}");
            assert_eq!(app.session.history().undo_name(), Some("Replace sound"));
            let _ = app.update(Message::Undo);
            assert_eq!(document(&app), before, "{case}: undone");
        }
    }

    // The sound a channel plays let go of on its
    // own header changes nothing: no edit, no undo step, and a plugin keeps its state (a fresh
    // instance would have reset it). The pad set on "My pad", the plugin on "Bell Keys", and
    // the kit's first drum on the channel it was just given to. The channel is selected, as
    // every sound let go of on a header selects it.
    #[test]
    fn a_channels_own_sound_let_go_of_on_its_header_changes_nothing() {
        let (mut app, _player, _dir) = p15_app("p15_own_sound");
        let before = document(&app);
        let pad = set_item(&app, "Pad");
        drop_on_header(&mut app, &pad, 2);
        assert_eq!(app.selected_channel, Some(ChannelId(2)));
        let bells = app
            .library_items()
            .into_iter()
            .find(|item| matches!(item, Item::Plugin(plugin) if plugin.plugin.name == "Bell Keys"))
            .expect("listed");
        drop_on_header(&mut app, &bells, 4);
        assert_eq!(app.selected_channel, Some(ChannelId(4)));
        assert_eq!(document(&app), before, "the state AAAA kept");
        assert_eq!(app.session.history().undo_len(), 0);
        drop_on_header(&mut app, &Item::Sound(0), 2);
        let kit = document(&app);
        assert_eq!(
            app.session.history().undo_len(),
            1,
            "another sound replaces"
        );
        drop_on_header(&mut app, &Item::Sound(0), 2);
        assert_eq!(document(&app), kit);
        assert_eq!(app.session.history().undo_len(), 1, "the same one does not");
    }

    // P4 known answers (gen_p15_expected.py): a set, a kit sound and a plugin each held from the
    // Library, moved onto a channel's header (its target says which) and let go of there give
    // exactly the case's document: the channel keeps its id, notes, routing, mute, volume, pan
    // and MIDI input, takes the sound and the settings its kind takes, and its new sound's
    // name unless it was renamed. It is selected; one undo gives the base document back byte
    // for byte, a plugin's state too.
    #[test]
    fn p15_a_sound_let_go_of_on_a_header_replaces_the_channels_sound() {
        let want = serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(fixture("p15_expected.json")).expect("p15_expected"),
        )
        .expect("parses");
        for (case, answer) in want["cases"].as_object().expect("cases") {
            let (mut app, _player, _dir) = p15_app(&format!("p15_{case}"));
            let before = document(&app);
            let held = match answer["dropped"].as_str().expect("what was dropped") {
                "set" => set_item(&app, answer["name"].as_str().expect("its name")),
                "kit" => Item::Sound(0),
                "plugin" => app
                    .library_items()
                    .into_iter()
                    .find(|item| matches!(item, Item::Plugin(plugin) if plugin.plugin.name == answer["name"]))
                    .expect("listed"),
                other => panic!("{other}"),
            };
            let id = u32::try_from(answer["channel"].as_u64().expect("id")).expect("fits");
            hold_and_move(&mut app, &held, Point::new(500.0, 200.0));
            let _ = app.update(Message::Media(super::super::MediaMessage::RackHover(true)));
            let header = layout::channel_header_rect(id as usize - 1).center();
            let over = over_header(&app, header);
            send(&mut app, over);
            assert_eq!(app.browse.drop_header, Some(ChannelId(id)), "{case}");
            release(&mut app, Point::new(500.0, 200.0));
            assert_eq!(
                document(&app),
                expected_document(&answer["document"]),
                "{case}"
            );
            assert_eq!(app.selected_channel, Some(ChannelId(id)), "{case}");
            assert_eq!(app.browse.drop_header, None, "{case}: the drag is over");
            assert_eq!(app.session.history().undo_len(), 1, "{case}");
            assert_eq!(app.session.history().undo_name(), Some("Replace sound"));
            let _ = app.update(Message::Undo);
            assert_eq!(document(&app), before, "{case}: undone");
        }
    }

    // P4: let go of between two rows, or moved off a header onto the steps, a set adds a new
    // channel as before; over an audio channel's header (p8_ui's first row) there is no target.
    #[test]
    fn a_sound_let_go_of_off_the_headers_adds_a_channel_as_before() {
        let (mut app, _player, _dir) = p15_app("p15_between");
        let pad = set_item(&app, "Pad");
        hold_and_move(&mut app, &pad, Point::new(500.0, 200.0));
        let _ = app.update(Message::Media(super::super::MediaMessage::RackHover(true)));
        let over = over_header(&app, layout::channel_header_rect(0).center());
        send(&mut app, over);
        assert_eq!(app.browse.drop_header, Some(ChannelId(1)));
        let off = over_header(&app, layout::step_rect(0, 3).center());
        send(&mut app, off);
        assert_eq!(app.browse.drop_header, None);
        let between = Point::new(80.0, layout::row_top(1) - 4.0);
        assert!(over_header(&app, between).is_none());
        release(&mut app, Point::new(500.0, 200.0));
        let project = app.session.project();
        assert_eq!(project.channels.len(), 5, "a new channel");
        assert_eq!(project.channels[&ChannelId(5)].name, "Pad");
        assert_eq!(
            project.channels[&ChannelId(1)].kind,
            ChannelKind::Sampler,
            "kept"
        );
    }

    /// The figures tests/fixtures/gen_p11_expected.py wrote.
    fn p11_expected() -> serde_json::Value {
        let text = std::fs::read_to_string(fixture("p11_expected.json")).expect("p11_expected");
        serde_json::from_str(&text).expect("parses")
    }

    /// Copies the folder `from` into `to`, files and folders.
    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).expect("dir");
        for entry in std::fs::read_dir(from).expect("lists") {
            let entry = entry.expect("entry");
            let target = to.join(entry.file_name());
            if entry.file_type().expect("type").is_dir() {
                copy_dir(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).expect("copies");
            }
        }
    }

    /// The Library's row of the set named `name`.
    fn set_item(app: &App, name: &str) -> Item {
        app.library_items()
            .into_iter()
            .find(|item| matches!(item, Item::Set(set) if set.name == name))
            .unwrap_or_else(|| panic!("{name} is listed"))
    }

    /// p11_base copied into a folder of its own with the pad set in its sets/ folder, which
    /// the Library lists as the shipped sets: the app on it, the playlist and Library open.
    fn set_app(test: &str) -> (App, Player, PathBuf) {
        let dir = temp_dir(test);
        for name in ["p11_base.json", "impulse.wav"] {
            std::fs::copy(fixture(name), dir.join(name)).expect("copies");
        }
        copy_dir(&fixture("instruments/pad"), &dir.join("sets/pad"));
        let host = Host {
            instruments: Some(dir.join("sets")),
            ..Host::offline()
        };
        let (app, player) = library_app(&dir.join("p11_base.json"), host);
        (app, player, dir)
    }

    // Known answer (gen_p11_expected.py): the set "Pad" dragged onto track 1 at bar 2 gives
    // exactly p11_after_set.json: an instrument channel naming the set by its path from the
    // project and a clip of the rack's pattern from tick 384, one bar long, written as format
    // version 4. The channel is selected, so live notes play it, and it sounds. One undo takes
    // both away and gives the version 2 document back.
    #[test]
    fn p11_the_pad_set_dropped_on_track_1_at_bar_2_is_the_known_document() {
        let want = &p11_expected()["set_drop"];
        let (mut app, mut player, _dir) = set_app("set_drop");
        let before = document(&app);
        let pad = set_item(&app, want["set"].as_str().expect("set"));
        hold_and_move(&mut app, &pad, Point::new(500.0, 200.0));
        let index = usize::try_from(want["track"].as_u64().expect("track") - 1).expect("fits");
        let bar = u32::try_from(want["bar"].as_u64().expect("bar")).expect("fits");
        let lane = lane_point(&app, index, bar);
        hover_lane(&mut app, lane);
        release(&mut app, Point::new(500.0, 200.0));
        assert_eq!(document(&app), expected_document(&want["document"]));
        let selected = u32::try_from(want["selected_channel"].as_u64().expect("id")).expect("fits");
        assert_eq!(app.selected_channel, Some(ChannelId(selected)));
        for _ in 0..4 {
            player.process(&mut [[0.0; 2]; 64]);
        }
        let router = Router::new(app.session.midi_inbox(), ControllerQueue::default());
        router.deliver(&[0x90, 60, 127]);
        let mut out = [[0.0; 2]; 64];
        player.process(&mut out);
        assert!(
            matches!(
                app.session.take_events()[..],
                [EngineEvent::LiveNoteStarted { channel, .. }] if channel == ChannelId(selected)
            ),
            "live notes play the new channel"
        );
        assert!(out.iter().any(|frame| *frame != [0.0; 2]), "the set sounds");
        assert_eq!(app.session.history().undo_len(), 1);
        let _ = app.update(Message::Undo);
        assert_eq!(document(&app), before);
    }

    // A set let go of over the rack is a new instrument channel named after it, selected, with
    // no clip; let go of anywhere else it adds nothing.
    #[test]
    fn a_set_dropped_on_the_rack_is_an_instrument_channel_and_elsewhere_nothing() {
        let (mut app, _player, _dir) = set_app("set_rack");
        let before = document(&app);
        let pad = set_item(&app, "Pad");
        hold_and_move(&mut app, &pad, Point::new(600.0, 500.0));
        release(&mut app, Point::new(600.0, 500.0));
        assert_eq!(document(&app), before, "let go of over nothing");
        hold_and_move(&mut app, &pad, Point::new(400.0, 100.0));
        let _ = app.update(Message::Media(super::super::MediaMessage::RackHover(true)));
        release(&mut app, Point::new(400.0, 100.0));
        let project = app.session.project();
        let channel = &project.channels[&ChannelId(2)];
        assert_eq!(
            (channel.name.as_str(), channel.kind),
            ("Pad", ChannelKind::Instrument)
        );
        assert_eq!(channel.instrument, Path::new("sets/pad/pad.sfz"));
        assert!(project.playlist.clips.is_empty(), "no clip");
        assert_eq!(app.selected_channel, Some(ChannelId(2)));
    }

    // The shipped sets list under the kit, counted among the instruments: 12 kit sounds and the
    // four test sets make 16.
    #[test]
    fn the_shipped_sets_list_under_the_kit() {
        let host = Host {
            instruments: Some(fixture("instruments")),
            ..Host::offline()
        };
        let (app, _player) = library_app(&fixture("p10_base.json"), host);
        let items = app.library_items();
        assert!(matches!(
            items[0],
            Item::Header {
                section: Section::Instruments,
                count: 16,
                open: true
            }
        ));
        assert_eq!(items[14], Item::Group(crate::ui::library::SHIPPED_SETS));
        let names: Vec<&str> = items[15..19]
            .iter()
            .map(|item| match item {
                Item::Set(set) => set.name.as_str(),
                other => panic!("{other:?} is not a set"),
            })
            .collect();
        assert_eq!(names, ["Layers", "Macro", "Pad", "Round robin"]);
    }

    // The sets installed with the program list under the kit with the data folder's (here
    // none); a set of them dropped on the rack names its main file by its reference,
    // `dawsome:instruments/pad/pad.sfz`, so the project opens on any install; and a shipped
    // folder of sounds (set.json "kind": "samples") is a Samples root of its own, named by its
    // set.json, that cannot be removed.
    #[test]
    fn an_installed_set_drops_as_its_reference_and_a_sounds_folder_is_a_samples_root() {
        let dir = temp_dir("installed");
        let (song, installed) = (dir.join("song"), dir.join("installed"));
        std::fs::create_dir_all(&song).expect("dir");
        for name in ["p11_base.json", "impulse.wav"] {
            std::fs::copy(fixture(name), song.join(name)).expect("copies");
        }
        copy_dir(&fixture("instruments/pad"), &installed.join("pad"));
        let loops = installed.join("one-shots");
        std::fs::create_dir_all(loops.join("samples")).expect("dir");
        std::fs::copy(fixture("impulse.wav"), loops.join("samples/hit.wav")).expect("copies");
        std::fs::write(
            loops.join("set.json"),
            r#"{"name":"One-shots and Loops","kind":"samples"}"#,
        )
        .expect("written");
        let host = Host {
            installed_instruments: Some(installed.clone()),
            ..Host::offline()
        };
        let (mut app, _player) = library_app(&song.join("p11_base.json"), host);
        let pad = set_item(&app, "Pad");
        let Item::Set(set) = &pad else {
            panic!("{pad:?} is a set")
        };
        assert_eq!(set.maker, "Dawsome tests");
        hold_and_move(&mut app, &pad, Point::new(400.0, 100.0));
        let _ = app.update(Message::Media(super::super::MediaMessage::RackHover(true)));
        release(&mut app, Point::new(400.0, 100.0));
        let channel = &app.session.project().channels[&ChannelId(2)];
        assert_eq!(
            channel.instrument,
            Path::new("dawsome:instruments/pad/pad.sfz")
        );
        let roots: Vec<(String, PathBuf, bool)> = app
            .browse
            .model
            .rows()
            .into_iter()
            .filter_map(|row| match row.kind {
                crate::ui::browser_model::RowKind::Root { user, .. } => {
                    Some((row.name, row.path, user))
                }
                _ => None,
            })
            .collect();
        let shipped = (
            "One-shots and Loops".to_owned(),
            loops.join("samples"),
            false,
        );
        assert!(roots.contains(&shipped), "{roots:?}");
    }

    // A second press on an instrument channel's name shows its set's load report when the set
    // left something out (macro's two opcodes and a header, each once, where first met); a set
    // that played all of itself opens nothing.
    #[test]
    fn a_second_press_on_an_instrument_channel_shows_what_its_set_left_out() {
        let (mut app, _player) = library_app(&fixture("p11_macro.json"), Host::offline());
        let _ = app.update(Message::ChannelOpened(ChannelId(1)));
        let Some(super::super::files::Modal::Report { summary, groups }) = &app.files.modal else {
            panic!("the report is on screen");
        };
        assert_eq!(summary, "Macro: 2 sounds play; some of the set is left out");
        assert_eq!(
            groups,
            &[(
                "Not played by this reader",
                vec![
                    "xfin_lovel: once, first at macro.sfz:5".to_owned(),
                    "xfout_hivel: once, first at regions.sfz:3".to_owned(),
                    "<effect>: once, first at macro.sfz:7".to_owned(),
                ]
            )]
        );
        let (mut app, _player) = library_app(&fixture("p11_layers.json"), Host::offline());
        let _ = app.update(Message::ChannelOpened(ChannelId(1)));
        assert!(app.files.modal.is_none(), "nothing left out, nothing shown");
    }

    // While a set's sounds load in the background the title counts them; once in, it is the
    // document's name alone.
    #[test]
    fn the_title_counts_a_sets_sounds_while_they_load() {
        let host = Host {
            playback: dawsome_media::Playback {
                stream_threshold: None,
                waiting: dawsome_media::Waiting::Never,
                convert_dir: None,
                stretch_dir: None,
                edited_dir: None,
            },
            ..Host::offline()
        };
        let (mut app, _player) = library_app(&fixture("p11_layers.json"), host);
        assert_eq!(
            app.title(),
            "p11_layers - Dawsome (loading instruments: 0 of 7 sounds)"
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while app.title() != "p11_layers - Dawsome" {
            assert!(
                std::time::Instant::now() < deadline,
                "the set loads: {}",
                app.title()
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
            let _ = app.update(Message::Media(super::super::MediaMessage::Tick));
        }
        assert!(app
            .session
            .instrument_report(Path::new("instruments/layers/layers.sfz"))
            .is_some());
    }

    // A set whose sounds cannot all arrive between two of the window's media ticks still
    // finishes loading: 64 zones of two seconds each, the window ticking every 100 ms as it
    // does. Before, each compile while the set loaded let go of the sounds already in, so a set
    // loaded only when every file landed within one tick, and the kit's 66-file piano never did
    // (found 2026-10-22).
    #[test]
    fn a_set_too_big_for_one_tick_still_finishes_loading() {
        const ZONES: u8 = 64;
        let dir = temp_dir("big_set");
        let set = dir.join("big");
        std::fs::create_dir_all(&set).expect("dir");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut sfz = String::new();
        for key in 0..ZONES {
            let name = format!("z{key}.wav");
            let mut writer = hound::WavWriter::create(set.join(&name), spec).expect("creates");
            for frame in 0..88_200_i32 {
                let value = ((frame + i32::from(key)) % 200 - 100) * 100;
                writer
                    .write_sample(i16::try_from(value).expect("fits"))
                    .expect("writes");
            }
            writer.finalize().expect("finalizes");
            sfz.push_str(&format!("<region> sample={name} key={key}\n"));
        }
        std::fs::write(set.join("big.sfz"), sfz).expect("writes");
        let layers = std::fs::read_to_string(fixture("p11_layers.json")).expect("fixture");
        let project = layers.replace("instruments/layers/layers.sfz", "big/big.sfz");
        assert_ne!(project, layers);
        std::fs::write(dir.join("big.json"), project).expect("writes");
        let host = Host {
            playback: dawsome_media::Playback {
                stream_threshold: None,
                waiting: dawsome_media::Waiting::Never,
                convert_dir: None,
                stretch_dir: None,
                edited_dir: None,
            },
            ..Host::offline()
        };
        let (mut app, _player) = library_app(&dir.join("big.json"), host);
        assert_eq!(
            app.title(),
            format!("big - Dawsome (loading instruments: 0 of {ZONES} sounds)")
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while app.title() != "big - Dawsome" {
            assert!(
                std::time::Instant::now() < deadline,
                "the set loads: {}",
                app.title()
            );
            std::thread::sleep(super::super::browse::MEDIA_PERIOD);
            let _ = app.update(Message::Media(super::super::MediaMessage::Tick));
        }
        let report = app
            .session
            .instrument_report(Path::new("big/big.sfz"))
            .expect("reported");
        assert_eq!(report.zones, usize::from(ZONES));
    }

    /// p11_base in `<dir>/song/` and a library laid out as many are, `<dir>/lib/`: the set
    /// `Instruments/a.sfz` with one region over every key playing `sample` (as the .sfz writes
    /// it), the fixtures' impulse as `Samples/a.wav` and as `<dir>/outside.wav`; lib/ is a
    /// folder added to the Library (in the folders file it reads). The app on the song, the
    /// playlist and Library open, and the main file.
    fn library_set_app(test: &str, sample: &str) -> (App, Player, PathBuf) {
        let dir = temp_dir(test);
        let (song, lib) = (dir.join("song"), dir.join("lib"));
        std::fs::create_dir_all(&song).expect("dir");
        std::fs::create_dir_all(lib.join("Instruments")).expect("dir");
        std::fs::create_dir_all(lib.join("Samples")).expect("dir");
        for name in ["p11_base.json", "impulse.wav"] {
            std::fs::copy(fixture(name), song.join(name)).expect("copies");
        }
        for to in [lib.join("Samples/a.wav"), dir.join("outside.wav")] {
            std::fs::copy(fixture("impulse.wav"), to).expect("copies");
        }
        let main = lib.join("Instruments/a.sfz");
        std::fs::write(&main, format!("<region> sample={sample}\n")).expect("written");
        let folders = dir.join("browser.json");
        let listed = serde_json::json!({ "folders": [lib] });
        std::fs::write(&folders, listed.to_string()).expect("written");
        let host = Host {
            browser_folders: Some(folders),
            ..Host::offline()
        };
        let (app, player) = library_app(&song.join("p11_base.json"), host);
        (app, player, main)
    }

    /// `set` (a Library row) let go of over the rack, then a note struck on the channel it
    /// became: the new channel's set path and whether the note sounds.
    fn drop_and_strike(app: &mut App, player: &mut Player, set: &Item) -> (PathBuf, bool) {
        hold_and_move(app, set, Point::new(400.0, 100.0));
        let _ = app.update(Message::Media(super::super::MediaMessage::RackHover(true)));
        release(app, Point::new(400.0, 100.0));
        let channel = &app.session.project().channels[&ChannelId(2)];
        assert_eq!(channel.kind, ChannelKind::Instrument);
        let instrument = channel.instrument.clone();
        for _ in 0..4 {
            player.process(&mut [[0.0; 2]; 64]);
        }
        let router = Router::new(app.session.midi_inbox(), ControllerQueue::default());
        router.deliver(&[0x90, 60, 127]);
        let mut out = [[0.0; 2]; 64];
        player.process(&mut out);
        (instrument, out.iter().any(|frame| *frame != [0.0; 2]))
    }

    // A library laid out lib/Instruments/a.sfz + lib/Samples/a.wav, lib/
    // added to the Library. "a" lists under My instruments; dropped on the rack it is an
    // instrument channel whose one zone, ../Samples/a.wav, plays: a note sounds.
    #[test]
    fn a_library_set_in_a_folder_added_plays_its_sibling_sample_folder() {
        let (mut app, mut player, main) = library_set_app("library_set", "../Samples/a.wav");
        let set = set_item(&app, "a");
        assert!(matches!(&set, Item::Set(found) if found.sfz == main));
        let (instrument, sounds) = drop_and_strike(&mut app, &mut player, &set);
        let report = app
            .session
            .instrument_report(&instrument)
            .expect("reported");
        assert_eq!(report.zones, 1);
        assert!(report.is_complete(), "{report:?}");
        assert!(sounds, "the set sounds");
    }

    // The same set reaching past the folder added (../../outside.wav, a file beside lib/) plays
    // nothing: its one region is left out, and its report says why.
    #[test]
    fn a_library_set_reads_nothing_past_the_folder_added() {
        let (mut app, mut player, _main) = library_set_app("library_past", "../../outside.wav");
        let set = set_item(&app, "a");
        let (instrument, sounds) = drop_and_strike(&mut app, &mut player, &set);
        let report = app
            .session
            .instrument_report(&instrument)
            .expect("reported");
        assert_eq!(report.zones, 0);
        assert_eq!(
            report.problems,
            [
                "a.sfz:1: sample=../../outside.wav: outside the folder added to the Library; \
              the region is left out"
            ]
        );
        assert!(!sounds, "nothing sounds");
    }

    /// A click on `item`'s row: a press and its release, no drag.
    fn click(app: &mut App, item: &Item) {
        let at = row_center(app, item);
        press(app, at, mouse::Button::Left);
        release(app, at);
    }

    // A click on a set previews the sound of its middle C (Pad's ramp.wav), cut after a short
    // while; a second press on it soon after adds it to the rack as an instrument channel,
    // selected, one undo step, as a drag onto the rack would.
    #[test]
    fn a_double_click_on_a_set_adds_it_and_a_click_previews_its_root_note() {
        let (mut app, _player, dir) = set_app("set_double");
        let pad = set_item(&app, "Pad");
        click(&mut app, &pad);
        let previewing = app.browse.model.previewing.clone().expect("a preview");
        assert!(previewing.ends_with("sets/pad/ramp.wav"), "{previewing:?}");
        assert!(app.browse.preview_stop.is_some(), "cut short");
        assert_eq!(
            app.session.project().channels.len(),
            1,
            "a click adds nothing"
        );
        let at = row_center(&app, &pad);
        press(&mut app, at, mouse::Button::Left);
        let project = app.session.project();
        let channel = &project.channels[&ChannelId(2)];
        assert_eq!(
            (channel.name.as_str(), channel.kind),
            ("Pad", ChannelKind::Instrument)
        );
        assert_eq!(channel.instrument, Path::new("sets/pad/pad.sfz"));
        assert_eq!(app.selected_channel, Some(ChannelId(2)));
        assert_eq!(app.browse.model.previewing, None, "the preview stops");
        assert_eq!(app.session.history().undo_len(), 1);
        std::fs::remove_dir_all(&dir).expect("removed");
    }

    // Enter while the Library has the keys adds the instrument row pressed last: a plugin as
    // the picker adds it, a kit sound as a sampler channel named after it (the kit written into
    // its folder). Without such a row Enter walks the Samples tree as before.
    #[test]
    fn enter_adds_the_instrument_row_pressed_last() {
        let dir = temp_dir("enter_adds");
        let host = Host {
            starter_kit: Some(dir.join("starter-kit")),
            ..catalogue_host()
        };
        let (mut app, _player) = library_app(&fixture("p10_base.json"), host);
        let sine = app
            .library_items()
            .into_iter()
            .find(|item| matches!(item, Item::Plugin(plugin) if plugin.plugin.name == "Dawsome Test Sine"))
            .expect("listed");
        click(&mut app, &sine);
        let _ = app.update(Message::OpenSelectedChannel);
        let channel = &app.session.project().channels[&ChannelId(2)];
        assert_eq!(
            (channel.name.as_str(), channel.kind),
            ("Dawsome Test Sine", ChannelKind::Plugin)
        );
        click(&mut app, &Item::Sound(1));
        let _ = app.update(Message::OpenSelectedChannel);
        let channel = &app.session.project().channels[&ChannelId(3)];
        assert_eq!(
            (channel.name.as_str(), channel.kind),
            ("Kick 909", ChannelKind::Sampler)
        );
        assert!(dir.join("starter-kit/Kick 909.wav").is_file());
        assert_eq!(app.selected_channel, Some(ChannelId(3)));
        let channels = app.session.project().channels.len();
        let _ = app.update(Message::OpenSelectedChannel);
        assert_eq!(
            app.session.project().channels.len(),
            channels,
            "Enter again adds nothing more"
        );
        std::fs::remove_dir_all(&dir).expect("removed");
    }

    // 6d review 3: while the keyboard plays notes (as a first start has it), Enter through the
    // window's key path still adds the instrument row pressed last, the plugin, and the numpad's
    // Enter does the same for the kit sound; neither plays a note.
    #[test]
    fn enter_adds_the_row_pressed_while_the_keyboard_plays() {
        use dawsome_engine::transport::Control;
        use iced::keyboard::key::{Code, Named};
        use iced::keyboard::{Key, Modifiers};
        let dir = temp_dir("enter_while_playing");
        let host = Host {
            starter_kit: Some(dir.join("starter-kit")),
            ..catalogue_host()
        };
        let (mut app, _player) = library_app(&fixture("p10_base.json"), host);
        assert!(app.typing.on);
        let sine = app
            .library_items()
            .into_iter()
            .find(|item| matches!(item, Item::Plugin(plugin) if plugin.plugin.name == "Dawsome Test Sine"))
            .expect("listed");
        let sent = app.session.controls_sent().len();
        for (row, code, added, expected) in [
            (
                sine,
                Code::Enter,
                ChannelId(2),
                ("Dawsome Test Sine", ChannelKind::Plugin),
            ),
            (
                Item::Sound(1),
                Code::NumpadEnter,
                ChannelId(3),
                ("Kick 909", ChannelKind::Sampler),
            ),
        ] {
            click(&mut app, &row);
            for pressed in [true, false] {
                let _ = app.update(Message::Key(super::super::KeyInput {
                    window: iced::window::Id::unique(),
                    key: Key::Named(Named::Enter),
                    code: Some(code),
                    modifiers: Modifiers::empty(),
                    pressed,
                    repeat: false,
                }));
            }
            let channel = app.session.project().channels.get(&added);
            assert_eq!(
                channel.map(|channel| (channel.name.as_str(), channel.kind)),
                Some(expected),
                "{code:?}"
            );
        }
        let notes = app.session.controls_sent()[sent..]
            .iter()
            .filter(|control| {
                matches!(
                    control,
                    Control::LiveNoteOn { .. } | Control::ChannelNoteOn { .. }
                )
            })
            .count();
        assert_eq!(notes, 0, "Enter plays no note");
        std::fs::remove_dir_all(&dir).expect("removed");
    }

    // The rack's "+ Instrument…" opens one picker over the sets, the kit and the plugins; a
    // double press on a kit sound adds it, on a set adds the set, each selected.
    #[test]
    fn the_instrument_picker_adds_a_kit_sound_and_a_set() {
        use crate::ui::picker::{self, PickerMessage, PickerTarget, ROW_HEIGHT};
        let (mut app, _player, dir) = set_app("adder");
        let kit = dir.join("starter-kit");
        app.host.starter_kit = Some(kit.clone());
        let _ = app.update(Message::OpenPicker(PickerTarget::AddInstrument));
        let lines = app.picker_lines();
        let row_of = |label: &str| {
            lines
                .iter()
                .position(|line| line.label() == label)
                .unwrap_or_else(|| panic!("{label} is listed"))
        };
        assert_eq!(row_of(picker::SETS_SECTION), 0);
        assert_eq!(row_of("Pad"), 1);
        assert_eq!(row_of(picker::KIT_SECTION), 2);
        let twice = |app: &mut App, row: usize| {
            let at = Point::new(40.0, ROW_HEIGHT * (row as f32 + 0.5));
            for _ in 0..2 {
                let _ = app.update(Message::Picker(PickerMessage::Pressed(at)));
            }
        };
        twice(&mut app, row_of("Snare"));
        assert!(app.picker.is_none(), "closed");
        let channel = &app.session.project().channels[&ChannelId(2)];
        assert_eq!(
            (channel.name.as_str(), channel.kind),
            ("Snare", ChannelKind::Sampler)
        );
        assert!(kit.join("Snare.wav").is_file());
        let _ = app.update(Message::OpenPicker(PickerTarget::AddInstrument));
        twice(&mut app, 1);
        let channel = &app.session.project().channels[&ChannelId(3)];
        assert_eq!(
            (channel.name.as_str(), channel.kind),
            ("Pad", ChannelKind::Instrument)
        );
        assert_eq!(app.selected_channel, Some(ChannelId(3)));
        assert_eq!(app.session.history().undo_len(), 2);
        std::fs::remove_dir_all(&dir).expect("removed");
    }

    /// set_app with the layers set beside the pad set in its sets/ folder.
    fn two_set_app(test: &str) -> (App, Player, PathBuf) {
        let (app, player, dir) = set_app(test);
        copy_dir(&fixture("instruments/layers"), &dir.join("sets/layers"));
        let host = app.host.clone();
        drop(app);
        drop(player);
        let (app, player) = library_app(&dir.join("p11_base.json"), host);
        (app, player, dir)
    }

    /// The live notes the session sent the player, in order.
    fn live_notes(app: &App) -> Vec<dawsome_engine::transport::Control> {
        use dawsome_engine::transport::Control;
        app.session
            .controls_sent()
            .iter()
            .filter(|control| {
                matches!(
                    control,
                    Control::ChannelNoteOn { .. }
                        | Control::LiveNoteOn { .. }
                        | Control::LiveNoteOff { .. }
                        | Control::LiveNotesOff
                )
            })
            .cloned()
            .collect()
    }

    fn type_z(app: &mut App, pressed: bool) {
        let _ = app.update(Message::TypingKey {
            code: iced::keyboard::key::Code::KeyZ,
            pressed,
            repeat: false,
        });
    }

    // A click on a channel's name
    // opens its window; a set let go of on the rack then adds a channel, selected, and the
    // window follows it there, so the typing keyboard and a device no link routes play the
    // new channel, not the one clicked. Before, both played the clicked one.
    #[test]
    fn a_set_added_after_a_click_on_a_name_plays_from_the_keys_and_the_devices() {
        let (mut app, mut player, dir) = set_app("follow_add");
        let _ = app.update(Message::ChannelSelected(ChannelId(1)));
        let _ = app.update(Message::ChannelOpened(ChannelId(1)));
        assert_eq!(app.instrument_on(), Some(ChannelId(1)));
        let pad = set_item(&app, "Pad");
        hold_and_move(&mut app, &pad, Point::new(500.0, 200.0));
        let _ = app.update(Message::Media(super::super::MediaMessage::RackHover(true)));
        release(&mut app, Point::new(500.0, 200.0));
        let added = ChannelId(2);
        assert_eq!(app.session.project().channels[&added].name, "Pad");
        assert_eq!(app.selected_channel, Some(added));
        assert_eq!(app.instrument_on(), Some(added), "the window follows");
        let port = crate::midi::port_channel(app.session.project(), "Some Port", app.live_target());
        assert_eq!(port, Some(added), "a port no link routes");
        for _ in 0..16 {
            player.process(&mut [[0.0; 2]; 64]);
        }
        let _ = app.session.take_events();
        let device =
            Router::new(app.session.midi_inbox(), ControllerQueue::default()).for_port(false);
        device.route(port);
        device.deliver(&[0x90, 60, 100]);
        device.deliver(&[0x80, 60, 0]);
        if !app.typing.on {
            let _ = app.update(Message::ToggleTyping);
        }
        let first = app.typing.layout.first_key();
        for pressed in [true, false] {
            let _ = app.update(Message::TypingKey {
                code: first,
                pressed,
                repeat: false,
            });
        }
        player.process(&mut [[0.0; 2]; 64]);
        let started: Vec<ChannelId> = app
            .session
            .take_events()
            .into_iter()
            .filter_map(|event| match event {
                EngineEvent::LiveNoteStarted { channel, .. } => Some(channel),
                _ => None,
            })
            .collect();
        assert_eq!(
            started,
            [added, added],
            "the typing keyboard's and the device's"
        );
        std::fs::remove_dir_all(&dir).expect("removed");
    }

    // Cut itself: a set whose set.json says "cut_itself": true, as the 808 basses' do, is added
    // as a channel cutting itself, the add and the setting one undo step that takes both away,
    // and previewed so; the other sets at the defaults (Pad as shipped: the preview test below).
    #[test]
    fn a_set_that_cuts_itself_is_added_and_previewed_cutting_itself() {
        use dawsome_engine::project::Sound;
        let (mut app, _player, dir) = set_app("set_cuts");
        std::fs::write(
            dir.join("sets/pad/set.json"),
            r#"{"name":"Pad","maker":"Dawsome tests","licence":"CC0-1.0","cut_itself":true}"#,
        )
        .expect("writes");
        let pad = set_item(&app, "Pad");
        let Item::Set(set) = &pad else {
            panic!("a set")
        };
        let before = app.session.history().undo_len();
        app.add_set(&set.sfz, &set.name);
        let cuts = Sound {
            cut_itself: true,
            ..Sound::default()
        };
        let added = &app.session.project().channels[&ChannelId(2)];
        assert_eq!(added.sound, cuts);
        assert_eq!(added.volume, 0.5625, "as an add puts a channel in");
        assert_eq!(app.session.history().undo_len(), before + 1, "one step");
        let _ = app.update(Message::Undo);
        assert_eq!(
            app.session.project().channels.len(),
            1,
            "one undo takes it away"
        );
        click(&mut app, &pad);
        assert_eq!(
            app.session.preview().map(|channel| channel.sound),
            Some(cuts)
        );
        std::fs::remove_dir_all(&dir).expect("removed");
    }

    // v2 addition: a click on a Library set makes it the preview: a channel of the plans alone
    // (the set at the default sound settings and at the volume an add gives a channel, so it
    // sounds as the channel it would add) that the typing keyboard and the devices no link
    // routes play, its row marked, the hint bar saying so; the document, the rack and the
    // history hold nothing of it. Selecting a rack channel ends it and releases its notes: the
    // key held ends, and typing plays the selected channel again.
    #[test]
    fn a_clicked_set_plays_from_the_keys_until_a_rack_channel_is_selected() {
        use crate::session::PREVIEW_CHANNEL;
        use crate::ui::browser_model::Playing;
        use dawsome_engine::project::Channel;
        use dawsome_engine::transport::Control;
        let (mut app, _player, dir) = set_app("preview_keys");
        let before = document(&app);
        let pad = set_item(&app, "Pad");
        click(&mut app, &pad);
        let Item::Set(set) = &pad else {
            panic!("a set")
        };
        assert_eq!(
            app.browse.model.playing,
            Some(Playing::Set(set.sfz.clone()))
        );
        assert!(browser::plays(
            app.browse.model.playing.as_ref().expect("on"),
            &pad
        ));
        assert_eq!(
            app.session.preview(),
            Some(&Channel::instrument("Pad", "sets/pad/pad.sfz").as_added())
        );
        assert_eq!(app.session.preview().map(|c| c.volume), Some(0.5625));
        assert_eq!(app.live_target(), Some(PREVIEW_CHANNEL));
        assert_eq!(
            crate::midi::port_channel(app.session.project(), "Port A", app.live_target()),
            Some(PREVIEW_CHANNEL),
            "a device no link routes plays it"
        );
        assert_eq!(document(&app), before, "not in the document");
        assert_eq!(app.session.history().undo_len(), 0);
        // A known answer again (it was once compared with the function under test), the keys
        // being the defaults.
        assert_eq!(
            app.hint_words(),
            "Playing from the Library \u{b7} Ctrl+T or your keyboard \u{b7} double-click to add"
        );
        super::super::tests::legacy_keyboard(&mut app);
        let _ = app.update(Message::ToggleTyping);
        type_z(&mut app, true);
        let velocity = crate::app::typing::VELOCITY;
        assert_eq!(
            live_notes(&app).last(),
            Some(&Control::ChannelNoteOn {
                channel: PREVIEW_CHANNEL,
                key: 48,
                velocity
            })
        );
        let sent = live_notes(&app).len();
        let _ = app.update(Message::ChannelSelected(ChannelId(1)));
        assert_eq!(
            live_notes(&app)[sent..],
            [Control::LiveNoteOff { key: 48 }],
            "its note released"
        );
        assert_eq!(app.session.preview(), None);
        assert_eq!(app.browse.model.playing, None);
        assert_eq!(app.live_target(), None);
        type_z(&mut app, false);
        type_z(&mut app, true);
        assert_eq!(
            live_notes(&app).last(),
            Some(&Control::LiveNoteOn { key: 48, velocity }),
            "the selected channel again"
        );
        assert_eq!(document(&app), before);
        std::fs::remove_dir_all(&dir).expect("removed");
    }

    // v2 addition: a project saved while a set is previewed holds no extra channel: the file
    // written is the one written without the preview.
    #[test]
    fn a_project_saved_while_previewing_holds_no_extra_channel() {
        let (mut app, _player, dir) = set_app("preview_save");
        let path = dir.join("p11_base.json");
        let unpreviewed =
            project_file::json_for(app.session.project(), app.session.samples_dir(), &path)
                .expect("writes");
        let pad = set_item(&app, "Pad");
        click(&mut app, &pad);
        assert!(app.session.preview().is_some());
        let _ = app.update(Message::File(crate::app::FileMessage::Save));
        assert_eq!(app.error, None);
        let saved = std::fs::read_to_string(&path).expect("saved");
        assert_eq!(saved, unpreviewed);
        let channels = serde_json::from_str::<serde_json::Value>(&saved).expect("JSON")["channels"]
            .as_array()
            .expect("channels")
            .len();
        assert_eq!(channels, 1);
        assert!(app.session.preview().is_some(), "saving keeps the preview");
        std::fs::remove_dir_all(&dir).expect("removed");
    }

    // A click on a group's label inside Instruments folds it
    // ("Plugins": its makers, plugins and Rescan go), the folds are saved under
    // "library.folded", and Enter opens it again, Left folds it, Right opens it, while that
    // label holds the keys; Down walks the Samples tree as before. A window started on the same
    // settings opens with the group folded.
    #[test]
    fn a_groups_label_folds_it_by_click_and_keys_and_the_fold_is_kept() {
        let dir = temp_dir("group_folds");
        let file = dir.join("dawsome/settings.json");
        let host = Host {
            settings_file: Some(file.clone()),
            saves_settings: true,
            ..catalogue_host()
        };
        let (mut app, _player) = library_app(&fixture("p10_base.json"), host.clone());
        let plugins = Item::Group(crate::ui::library::PLUGINS);
        let rescan_shown = |app: &App| app.library_items().contains(&Item::Rescan);
        assert!(rescan_shown(&app));
        let at = row_center(&app, &plugins);
        press(&mut app, at, mouse::Button::Left);
        assert!(!rescan_shown(&app), "folded");
        assert!(
            !app.library_items()
                .iter()
                .any(|item| matches!(item, Item::Maker(_) | Item::Plugin(_))),
            "no maker or plugin under the folded label"
        );
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&file).expect("saved")).expect("JSON");
        assert_eq!(saved["library"], serde_json::json!({"folded": ["Plugins"]}));
        let key = |app: &mut App, nav| {
            let _ = app.update(Message::Browser(BrowserMessage::Key(nav)));
        };
        key(&mut app, Nav::Enter);
        assert!(rescan_shown(&app), "Enter opens it");
        key(&mut app, Nav::Left);
        assert!(!rescan_shown(&app), "Left folds it");
        key(&mut app, Nav::Left);
        assert!(!rescan_shown(&app), "Left keeps it folded");
        key(&mut app, Nav::Right);
        assert!(rescan_shown(&app), "Right opens it");
        key(&mut app, Nav::Left);
        key(&mut app, Nav::Down);
        assert_eq!(app.browse.chosen, None, "the arrows walk the tree again");
        key(&mut app, Nav::Enter);
        assert!(!rescan_shown(&app), "Enter no longer acts on the label");
        let (again, _player) = library_app(&fixture("p10_base.json"), host);
        assert!(!rescan_shown(&again), "folded on the next start");
        assert!(
            again.library_items().contains(&plugins),
            "its label still shows"
        );
        std::fs::remove_dir_all(&dir).expect("removed");
    }

    // v2 addition: a click on another set makes it the preview instead (one preview at a
    // time); a double-click adds the set and ends the preview; a press elsewhere in the Library
    // (a section's header) ends it; so does another document.
    #[test]
    fn the_preview_follows_the_last_set_clicked_and_ends_on_adding_or_elsewhere() {
        use crate::ui::browser_model::Playing;
        let (mut app, _player, dir) = two_set_app("preview_follows");
        let (pad, layers) = (set_item(&app, "Pad"), set_item(&app, "Layers"));
        click(&mut app, &pad);
        click(&mut app, &layers);
        let instrument = |app: &App| {
            app.session
                .preview()
                .map(|channel| channel.instrument.clone())
        };
        assert_eq!(
            instrument(&app),
            Some(PathBuf::from("sets/layers/layers.sfz"))
        );
        assert!(
            matches!(&app.browse.model.playing, Some(Playing::Set(sfz)) if sfz.ends_with("layers/layers.sfz"))
        );
        // A press soon after the click: the second of a double-click, which adds.
        let at = row_center(&app, &layers);
        press(&mut app, at, mouse::Button::Left);
        assert_eq!(app.session.project().channels.len(), 2, "added");
        assert_eq!(app.session.preview(), None, "and the preview ends");
        click(&mut app, &pad);
        assert!(app.session.preview().is_some());
        let _ = app.update(Message::Browser(BrowserMessage::Section(Section::Samples)));
        assert_eq!(app.session.preview(), None, "a press elsewhere ends it");
        // The other set: a click on the pad again this soon would be a double-click.
        click(&mut app, &layers);
        assert!(app.session.preview().is_some());
        let _ = app.update(Message::File(crate::app::FileMessage::NewEmpty));
        // The set added is unsaved: the question first.
        let _ = app.update(Message::File(crate::app::FileMessage::Unsaved(
            crate::app::Choice::DontSave,
        )));
        assert_eq!(app.session.project().channels.len(), 1, "the new document");
        assert_eq!(app.browse.model.playing, None, "a new document has none");
        assert_eq!(app.session.preview(), None);
        std::fs::remove_dir_all(&dir).expect("removed");
    }

    // v2 addition: a click on a plugin instrument previews it as a plugin channel of the plans
    // alone, created as a rack plugin's; another plugin's click replaces it, so a bounded
    // number of preview instances runs at a time, and selecting a rack channel ends it.
    // On the real test plugins (the catalogue
    // this was first written on names a file that is not there, so it created none): each
    // click creates one instance; right after a switch at most three are alive (the new one,
    // the old one in the plan the player still plays, one on its way out), and two once the
    // player has taken the new plan and the window has had its plugin tick.
    #[test]
    fn a_previewed_plugin_is_one_instance_replaced_by_the_next() {
        use super::super::plugins::app_tests::{
            app_hosted, catalogue_of, test_plugin_file, variants_file,
        };
        use crate::ui::browser_model::Playing;
        let mut catalogue = catalogue_of(test_plugin_file());
        catalogue.files.extend(catalogue_of(variants_file()).files);
        let host = Host {
            clap: ClapHost::new(catalogue),
            ..Host::offline()
        };
        let path = fixture("p10_base.json");
        // The session's registry is the host's, so the previews create real instances.
        let (mut app, mut player) = app_hosted(&path, host);
        // The room the Library had under the 56 px toolbar (see `library_app`).
        let _ = app.update(Message::WindowResized(Size::new(1000.0, 576.0)));
        let _ = app.update(Message::TogglePanel(Panel::Browser));
        let plugins: Vec<Item> = app
            .library_items()
            .into_iter()
            .filter(|item| matches!(item, Item::Plugin(_)))
            .collect();
        assert!(
            plugins.len() >= 3,
            "the test plugins' instruments: {plugins:?}"
        );
        let processor = |app: &App| {
            let channel = app.session.preview().expect("a preview");
            assert_eq!(channel.kind, ChannelKind::Plugin);
            channel.processor.as_ref().expect("a plugin").id.clone()
        };
        for (round, item) in plugins.iter().enumerate() {
            click(&mut app, item);
            let Item::Plugin(record) = item else {
                panic!("a plugin")
            };
            assert_eq!(
                processor(&app),
                super::super::plugins::plugin_processor(record).id
            );
            assert_eq!(
                app.browse.model.playing,
                Some(Playing::Plugin(record.processor_id()))
            );
            let (created, alive) = (app.host.clap.created(), app.host.clap.instances());
            println!("round {round}: created {created}, alive {alive} after the switch");
            assert_eq!(created, round + 1, "one instance a click");
            assert!(alive <= 3, "right after a switch: {alive}");
            for _ in 0..4 {
                player.process(&mut [[0.0; 2]; 64]);
            }
            let _ = app.update(Message::Tick);
            let _ = app.update(Message::PluginTick);
            let alive = app.host.clap.instances();
            println!("round {round}: alive {alive} once the player took the plan");
            assert!(alive <= 2, "once the player has the plan: {alive}");
        }
        assert_eq!(app.session.project().channels.len(), 1, "nothing added");
        let _ = app.update(Message::ChannelSelected(ChannelId(1)));
        assert_eq!(app.session.preview(), None);
    }

    // Opening the instrument window ends the
    // Library's preview even on the channel already selected, so the typing keyboard and the
    // ports no link routes play the window's channel, not the hidden preview.
    #[test]
    fn opening_the_window_on_the_selected_channel_ends_the_preview() {
        let (mut app, _player, dir) = set_app("preview_window_selected");
        let selected = app.selected_channel.expect("a channel");
        let pad = set_item(&app, "Pad");
        click(&mut app, &pad);
        assert!(app.browse.model.playing.is_some(), "previewing");
        let _ = app.update(Message::OpenInstrument(selected));
        assert_eq!(app.instrument_on(), Some(selected));
        assert_eq!(app.browse.model.playing, None, "the preview ended");
        assert_eq!(app.session.preview(), None);
        assert_eq!(app.live_target(), Some(selected));
        std::fs::remove_dir_all(&dir).expect("removed");
    }

    // A previewed plugin that cannot be created
    // says so at the click (the catalogue's plugin file is not there: its stand-in plays), not
    // after the next edit; and the preview channel's plugin is the one its notices name.
    #[test]
    fn a_previewed_plugin_says_at_once_that_it_is_missing_and_is_named() {
        let path = fixture("p10_base.json");
        let (mut app, _player) = library_app(&path, catalogue_host());
        let lead = app
            .library_items()
            .into_iter()
            .find(
                |item| matches!(item, Item::Plugin(plugin) if plugin.plugin.name == "Analog Lead"),
            )
            .expect("listed");
        click(&mut app, &lead);
        let notice = app.notice.clone().unwrap_or_default();
        assert!(
            notice.starts_with("Missing plugin: Analog Lead"),
            "at the click: {notice:?}"
        );
        let owner =
            dawsome_engine::project::ProcessorOwner::Channel(crate::session::PREVIEW_CHANNEL);
        assert_eq!(
            app.plugin_reference(owner)
                .map(|reference| reference.display_name()),
            Some("Analog Lead")
        );
    }
}
