//! The app's side of plugins: the picker that adds them, their windows and the parameter
//! window's list (its window: `param_window.rs`), live parameter drags, the state captures that
//! make plugin tweaks undoable, the main-thread pump, and the background scan.
//!
//! A plugin's state lives in the document as the host captured it. A gesture (a drag in the
//! parameter window, or one the plugin reports from its own window) ends in one capture, taken
//! at the next plugin tick so the audio thread has applied the last value, and stored as one
//! `SetProcessorState` edit when the bytes changed. States over 4 MiB go into the document only
//! before it is written (saved, autosaved, exported), with no undo step; a change to one still
//! marks the document unsaved at once.

use std::fs;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use dawsome_engine::edit::Edit;
use dawsome_engine::processor::{Editor, HostError, HostNotice, NoticeKind, ProcessorHost};
use dawsome_engine::project::{ProcessorOwner, ProcessorRef};
use dawsome_engine::transport::{Control, ParamTarget};
use dawsome_host_clap::catalogue::{self, Offered, PluginKind};
use dawsome_host_clap::Catalogue;
#[cfg(test)]
use dawsome_host_clap::FORMAT;
use iced::futures::{stream, Stream};
use iced::widget::{operation, Id};
use iced::{Element, Task};
use tokio::time::MissedTickBehavior;

use super::{App, DragTarget, MediaMessage, Message, PLUGIN_PERIOD};
use crate::headless::Script;
use crate::session::Session;
use crate::ui::param_window::{Heading, ParamPanel, SLIDER_WIDTH};
use crate::ui::picker::{self as picker_view, Picker, PickerMessage, PickerTarget, Press};

/// The largest state captured at the end of every gesture; bigger ones wait for a save.
const UNDOABLE_STATE_BYTES: usize = 4 << 20;

/// How long after a gesture ends its capture waits: long enough for the audio thread to have
/// applied its last value (a callback is about 11 ms at 512 frames and 48 kHz).
const CAPTURE_DELAY: Duration = Duration::from_millis(50);

/// A plugin parameter's slider being dragged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct PluginDrag {
    pub(super) owner: ProcessorOwner,
    pub(super) id: u32,
    min: f32,
    max: f32,
    stepped: bool,
}

impl PluginDrag {
    /// How far along the parameter's range `value` is, from 0 to 1.
    pub(super) fn fraction(self, value: f32) -> f32 {
        let span = self.max - self.min;
        if span > 0.0 {
            ((value - self.min) / span).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// The value `rise` pixels along the slider from `start`, `fine` times as fast: the whole
    /// range over the slider's width, kept inside it, whole for a stepped parameter.
    pub(super) fn moved(self, start: f32, rise: f32, fine: f32) -> f32 {
        let value = start + rise / SLIDER_WIDTH * (self.max - self.min) * fine;
        let value = value.clamp(self.min, self.max);
        if self.stepped {
            value.round()
        } else {
            value
        }
    }
}

/// The session's CLAP host, when it has one.
#[cfg(test)]
fn clap(session: &Session) -> Option<&dyn ProcessorHost> {
    session.registry().host(&format!("{FORMAT}:"))
}

/// What `session`'s document holds for `owner`'s plugin: a plugin channel's or a slot's (the
/// Library's preview's for its channel).
fn reference_of(session: &Session, owner: ProcessorOwner) -> Option<&ProcessorRef> {
    let project = session.project();
    let reference = match owner {
        // The Library's preview is no channel of the document; its plugin is the preview's.
        ProcessorOwner::Channel(crate::session::PREVIEW_CHANNEL) => {
            session.preview()?.processor.as_ref()?
        }
        ProcessorOwner::Channel(id) => project.channels.get(&id)?.processor.as_ref()?,
        ProcessorOwner::Effect(id) => &project.mixer.effect(id)?.1.processor,
    };
    reference.format().is_some().then_some(reference)
}

/// The host `owner`'s plugin plays in: the one of the format its processor id names.
pub(super) fn host_of(session: &Session, owner: ProcessorOwner) -> Option<&dyn ProcessorHost> {
    session.registry().host(&reference_of(session, owner)?.id)
}

impl App {
    /// The plugins' start: a background scan when the catalogue is stale, and for a scripted run
    /// the picker opened and its rows written out.
    pub(super) fn start_plugins(&mut self, script: &Script, failed: &AtomicBool) -> Task<Message> {
        if let Some(setup) = &self.host.plugins {
            let stale = catalogue::is_stale_for(&setup.cache, &setup.dirs, setup.formats);
            if self.host.scans && stale {
                self.start_scan(false);
            }
        }
        let Some(kind) = script.open_picker else {
            return Task::none();
        };
        let target = match kind {
            PluginKind::Instrument => PickerTarget::Instrument,
            PluginKind::Effect => PickerTarget::Effect {
                insert: self.mixer.selected,
                slot: 0,
            },
        };
        let task = self.open_picker(target);
        if let Some(path) = &script.dump_picker {
            let lines = self.picker_lines();
            if let Err(error) = fs::write(path, picker_view::dump(&lines)) {
                eprintln!(
                    "dawsome-app: cannot write the picker's rows to {}: {error}",
                    path.display()
                );
                failed.store(true, Ordering::Relaxed);
            }
        }
        task
    }

    /// The window's extra flags at start: the "+ Instrument…" picker opened, and its rows
    /// written to `dump` once the instrument sets are found (the look on disk runs in the
    /// background, so the rows are written when it lands).
    pub fn start_extras(
        &mut self,
        extras: &crate::args::Extras,
        dump: Option<&Path>,
    ) -> Task<Message> {
        self.hints_flag = extras.hints;
        if let Some(id) = extras.open_instrument {
            self.open_instrument(dawsome_engine::project::ChannelId(id));
        }
        if !extras.open_adder {
            return Task::none();
        }
        self.adder_dump = dump.map(Path::to_path_buf);
        let task = self.open_picker(PickerTarget::AddInstrument);
        if self.sets_found {
            return Task::batch([task, self.dump_adder()]);
        }
        task
    }

    /// Shows what a scripted run's `--show` asks for over the window at start.
    pub fn show_at_start(&mut self, shown: crate::args::Shown) {
        match shown {
            crate::args::Shown::Export => {
                self.open_export(super::export::ExportKind::Wav);
            }
            crate::args::Shown::EditMenu => self.edit_menu = true,
            crate::args::Shown::Help(section) => {
                self.help.open = true;
                let _ = self.help_message(crate::ui::help_window::HelpMessage::Show(section));
            }
        }
    }

    /// Writes the "+ Instrument…" picker's rows to the file a scripted run named, once; a write
    /// that fails fails the run.
    pub(super) fn dump_adder(&mut self) -> Task<Message> {
        let Some(path) = self.adder_dump.take() else {
            return Task::none();
        };
        let lines = self.picker_lines();
        match fs::write(&path, picker_view::dump(&lines)) {
            Ok(()) => Task::none(),
            Err(error) => {
                let reason = format!(
                    "cannot write the picker's rows to {}: {error}",
                    path.display()
                );
                self.headless.fail(&reason).map(Message::Headless)
            }
        }
    }

    /// The window is open: opens the plugin window a scripted run asked for. Why not at boot: a
    /// plugin interface may make its own GL context current on this thread, and the toolkit,
    /// which creates its renderer when the window opens, then fails to choose a GPU (measured
    /// with an embedded third-party plugin: a panic in EGL's make_current, then a segfault).
    pub(super) fn open_scripted_plugin(&mut self) -> Task<Message> {
        let Some(owner) = self.scripted_plugin.take() else {
            return Task::none();
        };
        match self.open_plugin(owner) {
            Some(editor) => {
                println!("opened {owner:?}: {editor:?}");
                self.dump_params()
            }
            None => self
                .headless
                .fail(&format!("{owner:?} has no plugin to open"))
                .map(Message::Headless),
        }
    }

    /// Before the stream stops (the app closes, or another document replaces this one): the
    /// player suspends its plugins on the audio thread, waited for up to 200 ms.
    pub(super) fn release_plugins(&mut self) {
        let hosting = self
            .session
            .registry()
            .hosts()
            .any(ProcessorHost::has_instances);
        if hosting && self.audio.is_some() {
            self.session.suspend_processors(Duration::from_millis(200));
        }
    }

    /// Whether the plugins need the window at all: an instance is alive, a scan runs or a
    /// capture waits.
    pub(super) fn plugins_busy(&self) -> bool {
        self.scan.is_some()
            || !self.captures.is_empty()
            || self
                .session
                .registry()
                .hosts()
                .any(ProcessorHost::has_instances)
    }

    /// Whether the plugin pump runs on its timer: a scan runs, a capture waits, a host has work
    /// that comes due by the clock (a plugin's timer, an open plugin window), or an export's
    /// plugins, made and served here, are not done yet ([`App::serve_render_hosts`]). Why not
    /// under a dialog: no tick reaches the app then, and the export dialog's poll serves them.
    pub(super) fn plugins_due(&self) -> bool {
        self.scan.is_some()
            || !self.captures.is_empty()
            || (!self.render_hosts.is_empty() && self.files.modal.is_none())
            || self
                .session
                .registry()
                .hosts()
                .any(ProcessorHost::needs_ticks)
    }

    /// The doorbells of the hosts with instances alive, which the window watches while
    /// nothing comes due by the clock.
    pub(super) fn plugin_doorbells(&self) -> Option<Doorbells> {
        let bells: Vec<Arc<AtomicBool>> = self
            .session
            .registry()
            .hosts()
            .filter(|host| host.has_instances())
            .filter_map(ProcessorHost::doorbell)
            .collect();
        (!bells.is_empty()).then_some(Doorbells(bells))
    }

    /// What the document holds for `owner`'s plugin: a plugin channel's or a slot's.
    pub(super) fn plugin_reference(&self, owner: ProcessorOwner) -> Option<&ProcessorRef> {
        reference_of(&self.session, owner)
    }

    /// "<channel> — <plugin>" or "<insert> slot <n> — <plugin>", for `owner`'s plugin.
    fn plugin_title(&self, owner: ProcessorOwner) -> Option<String> {
        let plugin = self.plugin_reference(owner)?.display_name();
        Some(format!("{} \u{2014} {plugin}", self.plugin_place(owner)?))
    }

    /// Where `owner`'s plugin plays: its channel's name, or "<insert> slot <n>".
    fn plugin_place(&self, owner: ProcessorOwner) -> Option<String> {
        let project = self.session.project();
        Some(match owner {
            ProcessorOwner::Channel(id) => project.channels.get(&id)?.name.clone(),
            ProcessorOwner::Effect(id) => {
                let (insert, effect) = project.mixer.effect(id)?;
                let name = &project.mixer.get(insert)?.name;
                format!("{name} slot {}", effect.slot + 1)
            }
        })
    }

    /// What `owner`'s plugin is, as the parameter window says it under its place: its name, "VST3"
    /// for a VST3 one, and its maker as the catalogue names it ("Surge XT · VST3 · by Surge
    /// Synth Team"), each part only when known.
    fn plugin_about(&self, owner: ProcessorOwner) -> Option<String> {
        let reference = self.plugin_reference(owner)?;
        let mut parts = vec![reference.display_name().to_owned()];
        let format = catalogue::PluginFormat::ALL
            .iter()
            .copied()
            .find(|format| reference.format() == Some(format.name()));
        if format == Some(catalogue::PluginFormat::Vst3) {
            parts.push("VST3".to_owned());
        }
        let id = reference.id.split_once(':').map(|(_, id)| id);
        let catalogue = self.host.catalogue();
        let maker = format
            .zip(id)
            .and_then(|(format, id)| catalogue.find(format, id))
            .map(|(_, plugin)| plugin.vendor.trim())
            .filter(|maker| !maker.is_empty());
        if let Some(maker) = maker {
            parts.push(format!("by {maker}"));
        }
        Some(parts.join(" \u{b7} "))
    }

    /// Shows what creating plugins had to say, such as a missing plugin, in the toolbar.
    pub(super) fn show_plugin_notices(&mut self) {
        let notices = self.session.take_plugin_notices();
        if !notices.is_empty() {
            self.show_creation_notice(notices.join("; "));
        }
    }

    /// Shows `notice`, about plugins that could not be created, in the toolbar.
    fn show_creation_notice(&mut self, notice: String) {
        self.plugin_notice = Some(notice.clone());
        self.notice = Some(notice);
    }

    /// Says in the toolbar when plugin channels had to drop notes since the last check (more
    /// at once than the player or the plugin has room for), naming the plugins that did.
    pub(super) fn show_dropped_notes(&mut self) {
        let project = self.session.project();
        let mut total = self.session.dropped_notes();
        let mut names = Vec::new();
        for (&id, channel) in &project.channels {
            let Some(reference) = channel.processor.as_ref() else {
                continue;
            };
            let dropped = self.host.dropped_notes(ProcessorOwner::Channel(id));
            if dropped > 0 {
                total += dropped;
                names.push(reference.display_name().to_owned());
            }
        }
        if total <= self.dropped_notes {
            return;
        }
        self.dropped_notes = total;
        let mut notice = format!("{total} notes dropped (too many at once)");
        if !names.is_empty() {
            notice.push_str(": ");
            notice.push_str(&names.join(", "));
        }
        self.notice = Some(notice);
    }

    /// Lets go of a panel, captures and big states whose plugin is gone from the document, and
    /// builds the panel again when another plugin took its plugin's place (one given to its
    /// channel, or the undo of that): a panel lists the rows of the plugin it was built for.
    pub(super) fn forget_gone_plugins(&mut self) {
        if let Some(owner) = self.panel.as_ref().map(|panel| panel.owner) {
            let plugin = self.plugin_reference(owner).map(|plugin| plugin.id.clone());
            match plugin {
                None => self.panel = None,
                Some(id) if Some(&id) != self.panel_plugin.as_ref() => self.open_panel(owner),
                Some(_) => {}
            }
        }
        let gone: Vec<ProcessorOwner> = self
            .captures
            .keys()
            .chain(&self.big_states)
            .copied()
            .filter(|&owner| self.plugin_reference(owner).is_none())
            .collect();
        for owner in gone {
            self.captures.remove(&owner);
            self.big_states.remove(&owner);
        }
    }

    pub(super) fn open_picker(&mut self, target: PickerTarget) -> Task<Message> {
        self.picker = Some(Picker::new(target));
        operation::focus(Id::new(picker_view::FILTER_INPUT))
    }

    /// The picker's lines, from the catalogue as the host has it now.
    pub(super) fn picker_lines(&self) -> Vec<picker_view::Line> {
        let Some(picker) = &self.picker else {
            return Vec::new();
        };
        let catalogue = self.host.catalogue();
        self.lines_of(picker, &catalogue)
    }

    /// What `picker` lists from `catalogue`: its plugins, or for "+ Instrument…" the sets the
    /// Library found, the starter kit, the plugin instruments and "Sample file…".
    fn lines_of(&self, picker: &Picker, catalogue: &Catalogue) -> Vec<picker_view::Line> {
        match picker.target {
            PickerTarget::AddInstrument => picker_view::add_lines(
                &self.browse.library.sets,
                &crate::starter_kit::names(),
                catalogue,
                &picker.filter,
            ),
            target => picker_view::lines(catalogue, target.kind(), &picker.filter),
        }
    }

    pub(super) fn picker_layer<'a>(
        &'a self,
        picker: &'a Picker,
        catalogue: &Catalogue,
    ) -> Element<'a, Message> {
        let lines = self.lines_of(picker, catalogue);
        picker_view::view(picker, lines, self.scan.is_some())
    }

    pub(super) fn picker_message(&mut self, message: PickerMessage) -> Task<Message> {
        let lines = self.picker_lines();
        let choosable: Vec<picker_view::Line> = picker_view::choosable(&lines)
            .into_iter()
            .cloned()
            .collect();
        let Some(picker) = &mut self.picker else {
            return Task::none();
        };
        let chosen = match message {
            PickerMessage::FilterEdited(filter) => {
                picker.set_filter(filter);
                None
            }
            PickerMessage::Submit => Some(picker.selected.unwrap_or(0)),
            PickerMessage::Pressed(at) => match picker.press_on(at, &lines, Instant::now()) {
                Some(Press::Choose(row)) => Some(row),
                Some(Press::Select(_)) | None => None,
            },
            PickerMessage::Scrolled(rise) => {
                picker.scroll_by(rise, lines.len());
                None
            }
            PickerMessage::Rescan => {
                self.start_scan(true);
                None
            }
            PickerMessage::Close => {
                self.picker = None;
                None
            }
        };
        let target = self.picker.as_ref().map(|picker| picker.target);
        let (Some(row), Some(target)) = (chosen, target) else {
            return Task::none();
        };
        match choosable.get(row) {
            Some(picker_view::Line::Plugin(plugin)) => self.add_plugin(target, plugin),
            Some(picker_view::Line::Set(set)) => self.add_set(&set.sfz, &set.name),
            Some(picker_view::Line::Kit { index, .. }) => self.add_kit_sound(*index),
            Some(picker_view::Line::SampleFile) => {
                self.picker = None;
                return self.media_message(MediaMessage::AddSampler);
            }
            _ => {}
        }
        Task::none()
    }

    /// Adds `plugin` as `target` says, one undo step, and closes the picker. A new instrument
    /// is selected as a click on its name selects it, live notes included.
    pub(super) fn add_plugin(&mut self, target: PickerTarget, plugin: &Offered) {
        let processor = plugin_processor(plugin);
        let project = self.session.project();
        let edit = match target {
            PickerTarget::Instrument | PickerTarget::AddInstrument => Edit::AddPluginChannel {
                id: project.next_channel_id,
                name: plugin.plugin.name.clone(),
                processor,
            },
            PickerTarget::Effect { insert, slot } => Edit::AddEffect {
                insert,
                slot,
                id: project.mixer.next_effect_id,
                processor,
            },
        };
        let added = match &edit {
            Edit::AddPluginChannel { id, .. } => ProcessorOwner::Channel(*id),
            Edit::AddEffect { id, .. } => ProcessorOwner::Effect(*id),
            _ => return,
        };
        self.picker = None;
        self.commit(edit);
        if self.error.is_some() {
            return;
        }
        match added {
            ProcessorOwner::Channel(id) => self.select_channel(id),
            ProcessorOwner::Effect(id) => self.mixer.selected_effect = Some(id),
        }
    }

    /// Opens `owner`'s plugin window; the parameter window when the plugin has none it can show
    /// here, or when windows cannot open (the reason goes to the toolbar). Returns what opened,
    /// `None` when `owner` plays no plugin or it is missing.
    pub(super) fn open_plugin(&mut self, owner: ProcessorOwner) -> Option<Editor> {
        let title = self.plugin_title(owner)?;
        let name = self.plugin_reference(owner)?.display_name().to_owned();
        let opened = match host_of(&self.session, owner) {
            Some(host) => host.open_editor(owner, &title),
            None => Err(HostError::Missing),
        };
        match opened {
            Ok(editor @ (Editor::Embedded | Editor::Floating)) => Some(editor),
            Ok(Editor::Generic) => {
                self.open_panel(owner);
                Some(Editor::Generic)
            }
            Err(HostError::Failed(reason)) => {
                self.notice = Some(reason);
                self.open_panel(owner);
                Some(Editor::Generic)
            }
            Err(HostError::Missing) => {
                self.show_creation_notice(format!("Missing plugin: {name}"));
                None
            }
        }
    }

    /// Keeps our plugin windows above every window (`above`) or stacked as any window, in
    /// every host.
    pub(super) fn keep_plugin_windows_above(&self, above: bool) {
        for host in self.session.registry().hosts() {
            host.keep_windows_above(above);
        }
    }

    /// Opens the parameter window on `owner`'s plugin (on another plugin open already, it
    /// shows this one's list from its top).
    pub(super) fn open_panel(&mut self, owner: ProcessorOwner) {
        let (Some(title), Some(place), Some(about)) = (
            self.plugin_title(owner),
            self.plugin_place(owner),
            self.plugin_about(owner),
        ) else {
            return;
        };
        let Some(host) = host_of(&self.session, owner) else {
            return;
        };
        let rate = self.session.project().sample_rate;
        let heading = Heading {
            title,
            place,
            about,
        };
        self.panel = Some(ParamPanel::new(owner, heading, host, rate));
        self.panel_plugin = self.plugin_reference(owner).map(|plugin| plugin.id.clone());
        self.params.rebuilt();
    }

    /// Builds the list open on `owner`'s plugin again, scrolled as it was: the plugin's
    /// parameters' names, flags or MIDI mapping changed.
    fn rebuild_panel(&mut self, owner: ProcessorOwner) {
        let Some(scroll) = self
            .panel
            .as_ref()
            .filter(|panel| panel.owner == owner)
            .map(|panel| panel.scroll)
        else {
            return;
        };
        self.open_panel(owner);
        if let Some(panel) = &mut self.panel {
            panel.scroll = scroll;
        }
    }

    /// Reads the values of the parameter window's rows again: those it builds now, near the
    /// view.
    pub(super) fn refresh_panel(&mut self) {
        let height = self.param_window_size().height;
        if let Some(panel) = &mut self.panel {
            if let Some(host) = host_of(&self.session, panel.owner) {
                let shown = panel.shown(height);
                panel.refresh_items(host, shown);
            }
        }
    }

    /// The drag of parameter `index` of `owner`'s plugin, counting only the parameters that
    /// are not hidden (the panels' rows); `None` for a read-only one.
    pub(super) fn plugin_drag(&self, owner: ProcessorOwner, index: usize) -> Option<PluginDrag> {
        let host = host_of(&self.session, owner)?;
        let param = host
            .params(owner)
            .into_iter()
            .filter(|param| !param.hidden)
            .nth(index)?;
        (!param.read_only).then_some(PluginDrag {
            owner,
            id: param.id,
            min: param.min as f32,
            max: param.max as f32,
            stepped: param.stepped,
        })
    }

    /// The dragged parameter's value now: the last one sent while dragging, else the plugin's.
    pub(super) fn plugin_value(&self, drag: PluginDrag) -> Option<f32> {
        if let Some((owner, id, value)) = self.plugin_sent {
            if owner == drag.owner && id == drag.id {
                return Some(value);
            }
        }
        host_of(&self.session, drag.owner)?
            .value(drag.owner, drag.id)
            .map(|value| value as f32)
    }

    /// Sends `value` for the dragged parameter over the control ring, and shows it in the panel at
    /// once.
    pub(super) fn set_plugin_param(&mut self, drag: PluginDrag, value: f32) {
        let target = ParamTarget::Plugin {
            owner: drag.owner,
            id: drag.id,
        };
        let result = self.session.send(Control::Param(target, value));
        self.report(result);
        self.plugin_sent = Some((drag.owner, drag.id, value));
        if let Some(panel) = &mut self.panel {
            if panel.owner == drag.owner {
                if let Some(index) = panel.params.iter().position(|param| param.id == drag.id) {
                    panel.values[index] = f64::from(value);
                }
            }
        }
    }

    /// Captures `owner`'s state at the first plugin tick after [`CAPTURE_DELAY`].
    pub(super) fn capture_soon(&mut self, owner: ProcessorOwner) {
        self.captures.insert(owner, Instant::now() + CAPTURE_DELAY);
    }

    /// Captures `owner`'s state into the document when it differs from what the document
    /// holds: one undo step when `undoable` (a state too big for that waits for the save),
    /// none when saving.
    fn capture_now(&mut self, owner: ProcessorOwner, undoable: bool) {
        let Some(stored) = self.plugin_reference(owner).map(|r| r.state.clone()) else {
            return;
        };
        let Some(host) = host_of(&self.session, owner) else {
            return;
        };
        let captured = match host.capture_state(owner) {
            Ok(captured) => captured,
            Err(HostError::Missing) => return,
            Err(HostError::Failed(reason)) => {
                self.notice = Some(format!("Plugin failed: {reason}"));
                return;
            }
        };
        if stored.as_deref() == Some(captured.state.as_str()) {
            return;
        }
        if undoable && captured.bytes > UNDOABLE_STATE_BYTES {
            // Unsaved now, though the state reaches the document only when it is written: the
            // tweak is the user's change, and closing must ask before losing it.
            self.big_states.insert(owner);
            self.session.mark_unsaved();
            return;
        }
        let edit = Edit::SetProcessorState {
            owner,
            state: Some(captured.state),
        };
        let result = if undoable {
            self.session.apply(edit)
        } else {
            self.session.apply_live(edit).map(drop)
        };
        self.after_edit(result);
    }

    /// Before the document is written (saved, autosaved, exported): the states too big to
    /// capture with each change go into it, with no undo step. Returns whether any waited.
    pub(super) fn capture_big_states(&mut self) -> bool {
        let waiting = std::mem::take(&mut self.big_states);
        let any = !waiting.is_empty();
        for owner in waiting {
            self.capture_now(owner, false);
        }
        any
    }

    /// The plugins' main-thread work (a running export's too), what it reports, the captures
    /// that are due, and a finished scan.
    pub(super) fn plugin_tick(&mut self) {
        self.serve_render_hosts();
        let notices: Vec<HostNotice> = self
            .session
            .registry()
            .hosts()
            .flat_map(ProcessorHost::tick)
            .collect();
        for HostNotice { owner, kind } in notices {
            match kind {
                NoticeKind::Dirty => {
                    self.captures.entry(owner).or_insert_with(Instant::now);
                }
                NoticeKind::Closed => {}
                NoticeKind::Restart => {
                    self.capture_now(owner, true);
                    let result = self.session.recreate(owner);
                    self.after_edit(result);
                }
                NoticeKind::Params => self.rebuild_panel(owner),
                NoticeKind::Failed(reason) => {
                    let name = self
                        .plugin_reference(owner)
                        .map_or_else(String::new, |r| r.display_name().to_owned());
                    self.notice = Some(format!("Plugin failed: {name}: {reason}"));
                }
            }
        }
        // A capture waits while its plugin's slider is still being dragged.
        let dragging = match &self.drag {
            Some(drag) => match drag.target {
                DragTarget::Plugin(plugin) => Some(plugin.owner),
                _ => None,
            },
            None => None,
        };
        let now = Instant::now();
        let due: Vec<ProcessorOwner> = self
            .captures
            .iter()
            .filter(|&(&owner, &when)| when <= now && Some(owner) != dragging)
            .map(|(&owner, _)| owner)
            .collect();
        for owner in due {
            self.captures.remove(&owner);
            self.capture_now(owner, true);
        }
        self.poll_scan();
    }

    /// Starts a plugin scan in the background, unless one runs or this run scans nothing.
    pub(super) fn start_scan(&mut self, force: bool) {
        if self.scan.is_some() || !self.host.scans {
            return;
        }
        let Some(setup) = self.host.plugins.clone() else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(setup.scan(force));
        });
        self.scan = Some(receiver);
    }

    /// Takes a finished scan's catalogue: the picker lists it from now on, and the plugins of
    /// the document that could not be created (missing, or their file changed since the last
    /// scan) are tried again with it; the others stay as they were created.
    fn poll_scan(&mut self) {
        let Some(receiver) = &self.scan else {
            return;
        };
        match receiver.try_recv() {
            Ok(scanned) => {
                self.host.set_catalogue(scanned.catalogue);
                self.scan = None;
                // The Library lists the catalogue's instruments.
                self.browse.cache.clear();
                match self.session.retry_plugins() {
                    // The notice the failed plugins left gives way to what is still failing.
                    // Why only that one: a notice put up since (a controller asked for, a
                    // missing sample) is newer news than the scan, and what still fails was
                    // told when it first failed.
                    Ok(true) => {
                        let notices = self.session.take_plugin_notices();
                        if self.plugin_notice.is_some() && self.notice == self.plugin_notice {
                            self.notice = (!notices.is_empty()).then(|| notices.join("; "));
                            self.plugin_notice.clone_from(&self.notice);
                        }
                    }
                    Ok(false) => {}
                    Err(error) => self.report(Err(error)),
                }
                if let Some(unsaved) = scanned.unsaved {
                    self.error = Some(unsaved.to_string());
                }
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => self.scan = None,
        }
    }
}

/// What the document holds for `plugin` when it is added: its id in its format
/// (`"<format>:<id>"`), its name and its default state.
pub(super) fn plugin_processor(plugin: &Offered) -> ProcessorRef {
    ProcessorRef::plugin(plugin.processor_id(), &plugin.plugin.name)
}

/// The doorbells of the plugin hosts, watched off the window's thread: the window is woken
/// for the plugins only when one rang. A watch is itself and its clones (it hashes its bells),
/// so it runs on from one update to the next.
#[derive(Debug, Clone)]
pub(super) struct Doorbells(Vec<Arc<AtomicBool>>);

impl Hash for Doorbells {
    fn hash<H: Hasher>(&self, state: &mut H) {
        for bell in &self.0 {
            Arc::as_ptr(bell).hash(state);
        }
    }
}

impl Doorbells {
    /// An item whenever a bell rang since the last look, each look clearing every bell. It
    /// looks every [`PLUGIN_PERIOD`], so a plugin's request waits as long as it did when the
    /// window itself ran the pump on that timer; a look costs a load per bell, a redraw none.
    pub(super) fn rung(&self) -> impl Stream<Item = ()> + Send + 'static {
        stream::unfold(
            (self.0.clone(), None),
            |(bells, interval): (Vec<Arc<AtomicBool>>, Option<tokio::time::Interval>)| async move {
                // Made on the executor's first poll, where the timer driver is.
                let mut interval = interval.unwrap_or_else(|| {
                    let mut interval = tokio::time::interval(PLUGIN_PERIOD);
                    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
                    interval
                });
                loop {
                    interval.tick().await;
                    let rung = bells.iter().fold(false, |rung, bell| {
                        bell.swap(false, Ordering::AcqRel) | rung
                    });
                    if rung {
                        return Some(((), (bells, Some(interval))));
                    }
                }
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plugin_slider_sweeps_its_range_over_its_width_and_steps_snap() {
        let drag = PluginDrag {
            owner: ProcessorOwner::Channel(dawsome_engine::project::ChannelId(1)),
            id: 0,
            min: 0.0,
            max: 1.0,
            stepped: false,
        };
        assert_eq!(drag.moved(0.5, 12.0, 1.0), 0.6);
        assert_eq!(drag.moved(0.5, 1000.0, 1.0), 1.0);
        assert_eq!(drag.moved(0.5, 12.0, 0.1), 0.51);
        let stepped = PluginDrag {
            min: 0.0,
            max: 8.0,
            stepped: true,
            ..drag
        };
        assert_eq!(stepped.moved(2.0, 20.0, 1.0), 3.0);
    }
}

#[cfg(test)]
pub(super) mod app_tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    use dawsome_engine::project::{
        AutomationTarget, ChannelId, ChannelKind, EffectId, InsertId, Note, PatternId, Project,
    };
    use dawsome_engine::realtime::Player;
    use dawsome_engine::time::{MilliBpm, Ticks};
    use dawsome_host_clap::catalogue::{FileRecord, FileStatus};
    use dawsome_host_clap::{scan, ClapHost};

    use super::*;
    use crate::app::files::Modal;
    use crate::app::{Choice, ExportKind, ExportMessage, FileMessage, PlayMode, Start};
    use crate::audio;
    use crate::document::{AudioChoice, Document, Host};
    use crate::headless;
    use crate::project_file;
    use crate::ui::hint::ChannelRow;
    use crate::ui::layout;
    use crate::ui::mixer_layout;
    use crate::ui::mixer_state::{Menu, MixerMessage, ADD_MENU};
    use crate::ui::panels::{Panel, Panels};
    use crate::ui::param_window::ParamMessage;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");
    const SINE: ProcessorOwner = ProcessorOwner::Channel(ChannelId(1));

    /// The test plugin file scripts/build-test-plugins.sh puts in the target dir.
    pub(in crate::app) fn test_plugin_file() -> PathBuf {
        built_plugin("test-plugins", "dawsome_test_plugin.clap")
    }

    /// The test plugin variants' file (the big-state, failing and timer sines).
    pub(in crate::app) fn variants_file() -> PathBuf {
        built_plugin("test-plugin-variants", "dawsome_test_variants.clap")
    }

    /// `<target>/<folder>/<name>`. Unit tests have no CARGO_TARGET_TMPDIR, so the target dir is
    /// found from this test binary's own place, `<target>/debug/deps/<binary>`.
    fn built_plugin(folder: &str, name: &str) -> PathBuf {
        let exe = std::env::current_exe().expect("the test binary has a path");
        let target = exe
            .ancestors()
            .nth(3)
            .expect("the binary is inside the target dir");
        let file = target.join(folder).join(name);
        assert!(
            file.is_file(),
            "no {}: run scripts/build-test-plugins.sh (scripts/check.sh does)",
            file.display()
        );
        file
    }

    /// An app hosting the test plugins, with no device and nothing written outside.
    fn app_on(fixture: &str, catalogue: Catalogue) -> (App, Player) {
        let host = Host {
            clap: ClapHost::new(catalogue),
            ..Host::offline()
        };
        app_hosted(&Path::new(FIXTURES).join(fixture), host)
    }

    /// An app on the project at `path`, with no device, touching outside what `host` allows.
    pub(in crate::app) fn app_hosted(path: &Path, host: Host) -> (App, Player) {
        let opened = project_file::open(path).expect("fixture opens");
        let mode = Session::mode_for(&opened.project, false);
        let (session, player) =
            Session::open_with(opened.project, opened.samples_dir, mode, host.registry())
                .expect("compiles");
        let start = Start {
            document: Document::saved_at(path),
        };
        let (app, _) = App::boot(
            session,
            Err(audio::StartError::NoDevice),
            Panels::default(),
            layout::DEFAULT_WINDOW_SIZE,
            false,
            headless::Script::default(),
            Arc::new(AtomicBool::new(false)),
            host,
            start,
        );
        (app, player)
    }

    fn test_catalogue() -> Catalogue {
        catalogue_of(test_plugin_file())
    }

    /// A right press on `channel`'s rack name (its menu opens where the press was) and a click
    /// on the menu's `row`, through the main window's top layer: what the row sends.
    pub(in crate::app) fn channel_menu_row(
        app: &mut App,
        channel: ChannelId,
        row: ChannelRow,
    ) -> Message {
        use crate::app::widget_events::Window;
        use crate::ui::menu;
        app.open_channel_menu(channel);
        let rows = app.channel_menu_rows();
        let index = rows
            .iter()
            .position(|(shown, _)| *shown == row)
            .unwrap_or_else(|| panic!("{row:?} is not in the menu: {rows:?}"));
        let mut window = Window::new(app);
        let menus = window.overlays(app);
        let [shown] = menus[..] else {
            panic!("one menu on the window's top layer: {menus:?}");
        };
        let at = iced::Point::new(
            shown.center_x(),
            shown.y + menu::PADDING + menu::ROW_HEIGHT * (index as f32 + 0.5),
        );
        let mut published = window.click_overlay(app, at);
        assert_eq!(published.len(), 1, "one row: {published:?}");
        published.pop().expect("a message")
    }

    /// `channel`'s menu's "Parameters…", clicked: the parameter window opens on its plugin.
    fn parameters_from_menu(app: &mut App, channel: ChannelId) {
        let message = channel_menu_row(app, channel, ChannelRow::Parameters);
        let _ = app.update(message);
        assert_eq!(app.sampler_menu, None, "closed by its row");
    }

    /// A catalogue of `file`, described in this process (the scan child's work).
    pub(in crate::app) fn catalogue_of(file: PathBuf) -> Catalogue {
        let plugins = scan::describe_file(&file, || {}).expect("the test plugins load");
        let (size, mtime) = catalogue::stamp(&file).expect("the test plugin file is there");
        Catalogue {
            files: vec![FileRecord {
                file,
                format: catalogue::PluginFormat::Clap,
                size,
                mtime,
                status: FileStatus::Ok,
                reason: None,
                plugins,
            }],
            ..Catalogue::default()
        }
    }

    /// A fresh directory for one test's files.
    fn temp_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("dawsome-app-plugins-{}", std::process::id()))
            .join(test);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn state(app: &App, owner: ProcessorOwner) -> Option<String> {
        app.plugin_reference(owner).and_then(|r| r.state.clone())
    }

    /// A background scan that found `catalogue`, finished and waiting for the next plugin tick.
    fn scan_found(app: &mut App, catalogue: Catalogue) {
        let (sender, receiver) = mpsc::channel();
        sender
            .send(scan::Scanned {
                catalogue,
                report: scan::ScanReport {
                    scanned: Vec::new(),
                    unchanged: 1,
                },
                unsaved: None,
            })
            .expect("sends");
        app.scan = Some(receiver);
    }

    /// The captures waiting, made due now, then a plugin tick.
    fn tick_captures(app: &mut App) {
        for when in app.captures.values_mut() {
            *when = Instant::now();
        }
        let _ = app.update(Message::PluginTick);
    }

    // The test sine has no window of its own, so a second press on its name opens the parameter
    // window; dragging Gain from 0.5 a quarter of the slider to the left gives 0.25, heard
    // through the control ring, and its end is one undo step holding the captured state
    // 'T1RQMQAAAAAAANA/'; undo loads 'T1RQMQAAAAAAAOA/' back into the plugin.
    #[test]
    fn a_panel_drag_is_heard_live_and_undoes_as_one_state_change() {
        let (mut app, mut player) = app_on("p7_sine.json", test_catalogue());
        let _ = app.update(Message::ChannelOpened(ChannelId(1)));
        let panel = app.panel.as_ref().expect("the parameter window opens");
        assert_eq!(panel.title, "Dawsome Test Sine \u{2014} Dawsome Test Sine");
        assert_eq!(panel.params.len(), 1);
        assert_eq!(panel.params[0].name, "Gain");
        assert_eq!((panel.values[0], panel.texts[0].as_str()), (0.5, "0.50"));
        let _ = app.update(Message::Params(ParamMessage::Pressed(0)));
        let _ = app.update(Message::DragMoved(500.0));
        let _ = app.update(Message::DragMoved(470.0));
        // Stopped, the player hands the value to the plugin from its idle call.
        player.process(&mut [[0.0; 2]; 64]);
        let _ = app.update(Message::DragEnded);
        assert!(app.captures.contains_key(&SINE), "captured at a later tick");
        tick_captures(&mut app);
        assert_eq!(state(&app, SINE).as_deref(), Some("T1RQMQAAAAAAANA/"));
        assert_eq!(app.session.history().undo_len(), 1, "one undo step");
        let _ = app.update(Message::Undo);
        assert_eq!(state(&app, SINE).as_deref(), Some("T1RQMQAAAAAAAOA/"));
        let host = clap(&app.session).expect("a host");
        assert_eq!(
            host.value(SINE, 0),
            Some(0.5),
            "the plugin has the old value"
        );
        let fixture =
            std::fs::read_to_string(Path::new(FIXTURES).join("p7_sine.json")).expect("reads");
        assert_eq!(app.session.project().to_json().expect("json"), fixture);
    }

    // Opening a project changes nothing in it: the test sine clamps the saved gain 1.5
    // ('T1RQMQAAAAAAAPg/') to 1.0 as the host loads it and asks for a parameter rescan, but that
    // load is the host's own, so no capture follows: no undo step, no unsaved changes, and the
    // document holds the bytes it was read from.
    #[test]
    fn opening_a_project_does_not_capture_the_states_the_host_loaded() {
        let dir = temp_dir("host_loads");
        let text = std::fs::read_to_string(Path::new(FIXTURES).join("p7_sine.json"))
            .expect("reads")
            .replace("T1RQMQAAAAAAAOA/", "T1RQMQAAAAAAAPg/");
        let path = dir.join("p7_sine_loud.json");
        std::fs::write(&path, &text).expect("writes");
        let (mut app, _player) = app_on(path.to_str().expect("utf-8"), test_catalogue());
        let _ = app.update(Message::PluginTick);
        tick_captures(&mut app);
        assert_eq!(app.session.history().undo_len(), 0);
        assert!(!app.session.is_dirty());
        assert_eq!(app.session.project().to_json().expect("json"), text);
    }

    // A stream that fails takes no plugin with it: its player is parked, so the sine's
    // instance stays, its panel opens and its state can still be captured.
    #[test]
    fn a_failed_stream_parks_its_player_and_the_plugins_stay() {
        let (mut app, player) = app_on("p7_sine.json", test_catalogue());
        app.audio = Some(audio::Output::holding(player));
        let output = app.audio.as_ref().expect("an output");
        output.report(cpal::ErrorKind::DeviceNotAvailable);
        let _ = app.update(Message::AudioCheck);
        assert!(app.audio.is_none(), "the failed stream is gone");
        let _ = app.update(Message::PluginTick);
        assert_eq!(app.open_plugin(SINE), Some(Editor::Generic));
        let host = clap(&app.session).expect("a host");
        assert_eq!(
            host.capture_state(SINE).map(|captured| captured.state),
            Ok("T1RQMQAAAAAAAOA/".to_owned())
        );
    }

    // The catalogue's record of the test plugin file says another size than the file has (the
    // file changed after its scan), so the sine is refused and its channel is silent. When the
    // background rescan's catalogue comes in, the refused plugin is created from the state the
    // document holds, without the project being opened again: its panel opens, it sounds, and
    // the document is as it was read.
    #[test]
    fn a_rescan_creates_the_plugin_a_changed_file_kept_out() {
        let mut stale = test_catalogue();
        stale.files[0].size += 1;
        let (mut app, mut player) = app_on("p7_sine.json", stale);
        assert_eq!(
            app.notice.as_deref(),
            Some("Plugin failed: Dawsome Test Sine: its file changed after the last plugin scan")
        );
        assert_eq!(app.open_plugin(SINE), None, "no instance");
        scan_found(&mut app, test_catalogue());
        assert!(app.plugins_due(), "the scan's end waits for a tick");
        let _ = app.update(Message::PluginTick);
        assert!(app.scan.is_none(), "the scan is taken");
        assert_eq!(app.notice, None, "nothing fails any more");
        assert_eq!(app.open_plugin(SINE), Some(Editor::Generic));
        let host = clap(&app.session).expect("a host");
        assert_eq!(
            host.capture_state(SINE).map(|captured| captured.state),
            Ok("T1RQMQAAAAAAAOA/".to_owned())
        );
        let _ = app.update(Message::Play);
        let mut out = vec![[0.0f32; 2]; 12_288];
        player.process(&mut out);
        // Python (gen_p7_expected.py): the sine starts at frame 6000, gain 0.5.
        assert_eq!(out[5999], [0.0; 2]);
        assert_eq!(out[6000][0], 0.5);
        let fixture =
            std::fs::read_to_string(Path::new(FIXTURES).join("p7_sine.json")).expect("reads");
        assert_eq!(app.session.project().to_json().expect("json"), fixture);
        assert_eq!(app.session.history().undo_len(), 0);
    }

    // A notice that came after the one the refused sine left is not the scan's to replace: with
    // the sine's volume waiting for its controller, the rescan creates the sine and the
    // toolbar still asks for the controller.
    #[test]
    fn a_rescan_leaves_a_notice_newer_than_the_plugins_alone() {
        let mut stale = test_catalogue();
        stale.files[0].size += 1;
        let (mut app, _player) = app_on("p7_sine.json", stale);
        let _ = app.update(Message::KnobMenu {
            channel: ChannelId(1),
            knob: crate::ui::rack::Knob::Volume,
            at: iced::Point::new(182.0, 30.0),
        });
        let _ = app.update(Message::LinkChosen);
        let learning = Some(super::super::controllers::LEARNING);
        assert_eq!(app.notice.as_deref(), learning);
        scan_found(&mut app, test_catalogue());
        let _ = app.update(Message::PluginTick);
        assert_eq!(
            app.open_plugin(SINE),
            Some(Editor::Generic),
            "the sine is created"
        );
        assert_eq!(app.notice.as_deref(), learning, "the prompt stays");
    }

    // A plugin's parameters are automatable by its own ids, from its panel: a right press on the
    // sine's Gain opens the automation menu, whose row adds a clip of channel 1's param 0 holding
    // the gain as it is (0.5, of 0..1). Its points moved to 0.25, as dragging them does, it sets
    // the gain in song mode from play start, and the sine plays at it (values from
    // gen_p7_expected.py, 3b).
    #[test]
    fn a_plugin_parameter_is_automated_from_its_panel_and_plays_so() {
        let (mut app, mut player) = app_on("p7_sine_song.json", test_catalogue());
        parameters_from_menu(&mut app, ChannelId(1));
        let _ = app.update(Message::Params(ParamMessage::RightPressed(0)));
        let _ = app.update(Message::AutomateChosen);
        let (&id, automation) = app
            .session
            .project()
            .automation
            .list
            .iter()
            .next()
            .expect("an automation was added");
        assert_eq!(
            automation.target,
            AutomationTarget::ChannelParam {
                channel: ChannelId(1),
                param_id: 0
            }
        );
        let points: Vec<(i64, f32)> = automation
            .points
            .iter()
            .map(|point| (point.tick.0, point.value))
            .collect();
        assert_eq!(points, [(0, 0.5), (1536, 0.5)]);
        for tick in [0, 1536] {
            app.commit(Edit::MoveAutomationPoint {
                automation: id,
                from_tick: Ticks(tick),
                tick: Ticks(tick),
                value: 0.25,
            });
        }
        let _ = app.update(Message::ModeSelected(PlayMode::Song));
        let _ = app.update(Message::Play);
        // In device buffers: the player takes one queued plan per call, as from a device.
        let mut out = vec![[0.0f32; 2]; 12_288];
        for buffer in out.chunks_mut(512) {
            player.process(buffer);
        }
        assert_eq!(out[5999], [0.0; 2]);
        for (frame, want) in [
            (6000, 0.25),
            (6001, 0.249_585_45),
            (6002, 0.248_343_18),
            (6003, 0.246_277_33),
            (7200, 0.25),
        ] {
            assert_eq!(out[frame][0], want, "frame {frame}");
        }
    }

    // A plugin effect's rows in the mixer's slot panel are automatable the same way: a right
    // press on the test gain's row offers its automation by the plugin's id for it.
    #[test]
    fn a_plugin_effects_row_in_the_slot_panel_offers_its_automation() {
        let (mut app, _player) = app_on("p7_fx.json", test_catalogue());
        let _ = app.update(Message::TogglePanel(Panel::Mixer));
        let width = app.mixer.size().width;
        let press = |app: &mut App, at, button| {
            let message = MixerMessage::Pressed {
                at,
                button,
                double: false,
            };
            let _ = app.update(Message::Mixer(message));
        };
        let project = app.session.project().clone();
        app.mixer.select(InsertId(1), &project);
        press(
            &mut app,
            mixer_layout::slot_rect(0, width).center(),
            iced::mouse::Button::Left,
        );
        let row = mixer_layout::slider_rect(0, width).center();
        press(&mut app, row, iced::mouse::Button::Right);
        let menu = app.automate.as_ref().expect("the automation menu opens");
        assert_eq!(
            menu.target,
            AutomationTarget::EffectPlugin {
                effect: EffectId(1),
                param_id: 0
            }
        );
        assert_eq!(menu.value, 0.5, "the gain as it is, of 0..1");
    }

    /// An output device that is not there twice, then is: each try hands the player back.
    fn there_the_third_time(player: Player) -> Result<audio::Output, audio::StartFailure> {
        static TRIES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        if TRIES.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 2 {
            return Err(audio::StartFailure {
                error: audio::StartError::NoDevice,
                player: Some(Box::new(player)),
            });
        }
        Ok(audio::Output::holding(player))
    }

    // With no device at start the player is parked; each retry (the 5 s timer's, or the
    // notice's button) tries the device again, and the third finds it: the parked player
    // moves to the stream with its plugin as it was, no instance made anew, and plays the
    // sine there.
    #[test]
    fn a_retry_moves_the_parked_player_to_the_device_once_it_is_there() {
        let host = Host {
            clap: ClapHost::new(test_catalogue()),
            audio: AudioChoice::Fake(there_the_third_time),
            ..Host::offline()
        };
        let (mut app, player) = app_hosted(&Path::new(FIXTURES).join("p7_sine.json"), host);
        app.session.park(player);
        let created = app.host.clap.created();
        for _ in 0..2 {
            let _ = app.update(Message::RetryAudio);
            assert!(app.audio.is_none(), "no device yet");
            assert!(app.session.is_parked(), "the player is parked again");
            assert_eq!(
                app.audio_problem.as_deref(),
                Some("Audio off: no audio output device")
            );
        }
        let _ = app.update(Message::RetryAudio);
        assert!(app.audio.is_some(), "the third try opens the device");
        assert!(!app.session.is_parked());
        assert_eq!(app.audio_problem, None);
        assert_eq!(app.host.clap.created(), created, "the same instance");
        let host = clap(&app.session).expect("a host");
        assert_eq!(
            host.capture_state(SINE).map(|captured| captured.state),
            Ok("T1RQMQAAAAAAAOA/".to_owned())
        );
        assert_eq!(app.open_plugin(SINE), Some(Editor::Generic));
        let _ = app.update(Message::Play);
        let mut player = app
            .audio
            .take()
            .and_then(audio::Output::stop)
            .expect("the stream hands its player back");
        let mut out = vec![[0.0f32; 2]; 12_288];
        player.process(&mut out);
        // Python (gen_p7_expected.py): the sine starts at frame 6000, gain 0.5.
        assert_eq!(out[5999], [0.0; 2]);
        assert_eq!(out[6000][0], 0.5);
    }

    // 8 kHz at 522 BPM put ticks 0 to 5 in the first block: 600 long notes start there, the
    // sine's table of sounding notes holds 512, and the health timer names the plugin that had
    // to drop the other 88.
    #[test]
    fn a_flood_of_notes_names_the_plugin_that_dropped_them() {
        let text = std::fs::read_to_string(Path::new(FIXTURES).join("p7_sine.json"));
        let mut project = Project::load_json(&text.expect("reads")).expect("fixture loads");
        project.sample_rate = 8_000;
        project.tempo = MilliBpm(522_000);
        let pattern = project.patterns.get_mut(&PatternId(1)).expect("pattern 1");
        pattern.notes = (0..6)
            .flat_map(|tick| {
                (0..100).map(move |key| Note {
                    channel: ChannelId(1),
                    start: Ticks(tick),
                    length: Ticks(300),
                    velocity: 1.0,
                    key,
                })
            })
            .collect();
        let path = temp_dir("note_flood").join("flood.json");
        std::fs::write(&path, project.to_json().expect("json")).expect("writes");
        let (mut app, mut player) = app_on(path.to_str().expect("utf-8"), test_catalogue());
        let _ = app.update(Message::Play);
        player.process(&mut [[0.0; 2]; 512]);
        let _ = app.update(Message::AudioCheck);
        assert_eq!(
            app.notice.as_deref(),
            Some("88 notes dropped (too many at once): Dawsome Test Sine")
        );
    }

    /// p7_sine with the big-state variant in place of the sine, at its default gain 1.0, in
    /// `dir`; and a host of the variants, autosaving into `dir`.
    fn big_state_project(dir: &Path) -> (PathBuf, Host) {
        let text = std::fs::read_to_string(Path::new(FIXTURES).join("p7_sine.json"))
            .expect("reads")
            .replace(
                r#""id":"clap:org.dawsome.test.sine","name":"Dawsome Test Sine","state":"T1RQMQAAAAAAAOA/""#,
                r#""id":"clap:org.dawsome.test.big","name":"Dawsome Test Big State""#,
            );
        assert!(text.contains("test.big"), "the fixture names the sine");
        let path = dir.join("big.json");
        std::fs::write(&path, text).expect("writes");
        let host = Host {
            clap: ClapHost::new(catalogue_of(variants_file())),
            untitled_autosave: Some(dir.join("untitled.autosave.json")),
            ..Host::offline()
        };
        (path, host)
    }

    /// Opens channel 1's panel and drags its Gain a quarter of the slider down, heard by the
    /// stopped player, then lets go and runs the capture.
    fn drag_gain_a_quarter_down(app: &mut App, player: &mut Player) {
        let _ = app.update(Message::ChannelOpened(ChannelId(1)));
        let _ = app.update(Message::Params(ParamMessage::Pressed(0)));
        let _ = app.update(Message::DragMoved(500.0));
        let _ = app.update(Message::DragMoved(470.0));
        player.process(&mut [[0.0; 2]; 64]);
        let _ = app.update(Message::DragEnded);
        tick_captures(app);
    }

    // A tweak to a state too big for an undo step (the variant's 5 MiB, from gain 1.0 to 0.75)
    // still leaves the document with unsaved changes, so closing asks first, and the autosave
    // holds the tweak: its state starts with 'OTP1' and 0.75 ('T1RQMQAAAAAAAOg/') and is
    // 6990508 base64 characters long (5 MiB, from Python).
    #[test]
    fn a_big_state_tweak_is_unsaved_and_autosaved() {
        let dir = temp_dir("big_state_unsaved");
        let (path, host) = big_state_project(&dir);
        let (mut app, mut player) = app_hosted(&path, host);
        drag_gain_a_quarter_down(&mut app, &mut player);
        assert_eq!(
            app.session.history().undo_len(),
            0,
            "too big for an undo step"
        );
        assert!(app.session.is_dirty(), "the tweak is an unsaved change");
        let _ = app.update(Message::File(FileMessage::CloseRequested));
        assert!(app.shows_unsaved_prompt());
        let _ = app.update(Message::File(FileMessage::Unsaved(Choice::Cancel)));
        let _ = app.update(Message::File(FileMessage::AutosaveTick));
        let autosave = project_file::autosave_beside(&path);
        let text = std::fs::read_to_string(&autosave).expect("autosaved");
        let saved: serde_json::Value = serde_json::from_str(&text).expect("JSON");
        let state = saved["channels"][0]["processor"]["state"]
            .as_str()
            .expect("the captured state");
        assert!(state.starts_with("T1RQMQAAAAAAAOg/"), "{}", &state[..24]);
        assert_eq!(state.len(), 6_990_508);
    }

    // An export renders the document with a big state's tweak in it: the state is captured
    // first, so the variant sounds at 0.75 on the note's first frame (6000), not at 1.0.
    #[test]
    fn an_export_captures_a_big_state_first() {
        let dir = temp_dir("big_state_export");
        let (path, mut host) = big_state_project(&dir);
        let cache = dir.join("catalogue.json");
        catalogue_of(variants_file()).save(&cache).expect("saves");
        host.plugins = Some(dawsome_host_clap::PluginSetup {
            dirs: vec![dir.clone()],
            cache,
            child: scan::ChildCommand {
                program: PathBuf::from("false"),
                args: Vec::new(),
            },
            formats: catalogue::PluginFormat::CLAP,
        });
        let (mut app, mut player) = app_hosted(&path, host);
        drag_gain_a_quarter_down(&mut app, &mut player);
        let export = |message| Message::File(FileMessage::ExportForm(message));
        let _ = app.update(Message::File(FileMessage::Export(ExportKind::Wav)));
        let _ = app.update(export(ExportMessage::Depth(
            dawsome_media::BitDepth::Float32,
        )));
        let _ = app.update(export(ExportMessage::Start));
        let _ = app.update(export(ExportMessage::Picked(Some(dir.join("big")))));
        let mut reader = hound::WavReader::open(dir.join("big.wav")).expect("exported");
        let left: Vec<f32> = reader
            .samples::<f32>()
            .step_by(2)
            .collect::<Result<_, _>>()
            .expect("samples");
        assert_eq!((left[5999], left[6000]), (0.0, 0.75));
    }

    // An export whose plugin fails part way (the fail variant, from its fifth block on) says so
    // once, by name, beside "Exported".
    #[test]
    fn an_export_names_a_plugin_that_failed_once() {
        let dir = temp_dir("fail_export");
        let text = std::fs::read_to_string(Path::new(FIXTURES).join("p7_sine.json"))
            .expect("reads")
            .replace(
                r#""id":"clap:org.dawsome.test.sine","name":"Dawsome Test Sine""#,
                r#""id":"clap:org.dawsome.test.fail","name":"Dawsome Test Fail""#,
            );
        let path = dir.join("fail.json");
        std::fs::write(&path, text).expect("writes");
        let cache = dir.join("catalogue.json");
        catalogue_of(variants_file()).save(&cache).expect("saves");
        let host = Host {
            clap: ClapHost::new(catalogue_of(variants_file())),
            plugins: Some(dawsome_host_clap::PluginSetup {
                dirs: vec![dir.clone()],
                cache,
                child: scan::ChildCommand {
                    program: PathBuf::from("false"),
                    args: Vec::new(),
                },
                formats: catalogue::PluginFormat::CLAP,
            }),
            ..Host::offline()
        };
        let (mut app, _player) = app_hosted(&path, host);
        let export = |message| Message::File(FileMessage::ExportForm(message));
        let _ = app.update(Message::File(FileMessage::Export(ExportKind::Wav)));
        let _ = app.update(export(ExportMessage::Bars("20".into())));
        let _ = app.update(export(ExportMessage::Start));
        let _ = app.update(export(ExportMessage::Picked(Some(dir.join("fail")))));
        let notice = app.notice.as_deref().unwrap_or_default();
        assert!(notice.starts_with("Exported "), "{notice}");
        let named = "Plugin failed: Dawsome Test Fail: it failed while processing";
        assert_eq!(notice.matches(named).count(), 1, "{notice}");
    }

    /// An app on p7_sine with the variant `id` (a `clap:` id) named `name` in place of the sine,
    /// written into `dir`, hosting the variants.
    fn variant_app(dir: &Path, id: &str, name: &str) -> (App, Player) {
        let text = std::fs::read_to_string(Path::new(FIXTURES).join("p7_sine.json"))
            .expect("reads")
            .replace(
                r#""id":"clap:org.dawsome.test.sine","name":"Dawsome Test Sine""#,
                &format!(r#""id":"{id}","name":"{name}""#),
            );
        assert!(text.contains(id), "the fixture names the sine");
        let path = dir.join("variant.json");
        std::fs::write(&path, text).expect("writes");
        let host = Host {
            clap: ClapHost::new(catalogue_of(variants_file())),
            ..Host::offline()
        };
        app_hosted(&path, host)
    }

    const FAILED: &str = "Plugin failed: Dawsome Test Fail: it failed while processing";

    // A loaded plugin with no timer and no window wakes the window only through its host's
    // doorbell, watched off the window's thread. The fail variant fails on the audio thread
    // from its fifth block: that rings the doorbell, the watch yields once (clearing it), and
    // the tick it brings names the failure.
    #[test]
    fn a_quiet_plugin_wakes_the_window_only_through_its_doorbell() {
        use iced::futures::StreamExt;

        let dir = temp_dir("doorbell");
        let (mut app, mut player) =
            variant_app(&dir, "clap:org.dawsome.test.fail", "Dawsome Test Fail");
        let _ = app.update(Message::PluginTick);
        assert!(!app.plugins_due(), "no timer, no window, nothing waiting");
        let bells = app.plugin_doorbells().expect("an instance is alive");
        for bell in &bells.0 {
            bell.store(false, Ordering::Relaxed);
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("a runtime");
        let mut rung = Box::pin(bells.rung());
        let mut next = || {
            runtime.block_on(async {
                tokio::time::timeout(Duration::from_millis(100), rung.next())
                    .await
                    .ok()
            })
        };
        assert_eq!(next(), None, "a quiet plugin never rings");
        let _ = app.update(Message::Play);
        for _ in 0..8 {
            player.process(&mut [[0.0; 2]; 512]);
        }
        assert_eq!(next(), Some(Some(())), "the failure rang");
        assert!(
            bells.0.iter().all(|bell| !bell.load(Ordering::Relaxed)),
            "the look cleared it"
        );
        assert_eq!(next(), None, "once");
        let _ = app.update(Message::PluginTick);
        assert_eq!(app.notice.as_deref(), Some(FAILED));
    }

    // A request whose tick a dialog kept out is still served once the dialog closes, without
    // the plugin asking again. The fail variant fails from its fifth block, which rings the
    // bell; the watch clears the bell as it looks, and the tick it sends arrives while "Save
    // changes?" is up, which only its answers get past. The failure still waits in the
    // plugin's flags, so the window's timer runs until a tick has taken it.
    #[test]
    fn a_request_a_dialog_kept_out_is_served_once_it_closes() {
        let dir = temp_dir("dialog_request");
        let (mut app, mut player) =
            variant_app(&dir, "clap:org.dawsome.test.fail", "Dawsome Test Fail");
        let _ = app.update(Message::PluginTick);
        assert!(!app.plugins_due(), "nothing waits yet");
        let _ = app.update(Message::Play);
        for _ in 0..6 {
            player.process(&mut [[0.0; 2]; 512]);
        }
        app.files.modal = Some(Modal::Unsaved);
        let bells = app.plugin_doorbells().expect("an instance is alive");
        let rung = bells.0.iter().fold(false, |rung, bell| {
            bell.swap(false, Ordering::AcqRel) | rung
        });
        assert!(rung, "the failure rang");
        let _ = app.update(Message::PluginTick);
        assert_eq!(app.notice, None, "the dialog kept the tick out");
        app.files.modal = None;
        assert!(app.plugins_due(), "the failure keeps the timer running");
        let _ = app.update(Message::PluginTick);
        assert_eq!(app.notice.as_deref(), Some(FAILED));
    }

    // A plugin that registered a timer keeps the window's plugin timer running: its timer comes
    // due by the clock, with nothing rung.
    #[test]
    fn a_plugin_timer_keeps_the_window_ticking() {
        let dir = temp_dir("plugin_timer");
        let (mut app, _player) =
            variant_app(&dir, "clap:org.dawsome.test.timer", "Dawsome Test Timer");
        let _ = app.update(Message::PluginTick);
        assert!(app.plugins_due(), "the timer comes due by the clock");
    }

    // A notice the host holds from outside a tick (the sine refusing a state it was given)
    // keeps the timer running until a tick shows it. "AAAA" is three zero bytes, not the
    // sine's "OTP1" state.
    #[test]
    fn a_notice_the_host_holds_keeps_the_window_ticking_until_shown() {
        let (mut app, _player) = app_on("p7_sine.json", test_catalogue());
        let _ = app.update(Message::PluginTick);
        assert!(!app.plugins_due(), "a quiet sine");
        app.commit(Edit::SetProcessorState {
            owner: SINE,
            state: Some("AAAA".to_owned()),
        });
        assert!(app.plugins_due(), "the refusal waits for a tick");
        let _ = app.update(Message::PluginTick);
        assert_eq!(
            app.notice.as_deref(),
            Some("Plugin failed: Dawsome Test Sine: it refused the state it was given")
        );
        assert!(!app.plugins_due(), "shown, nothing waits");
    }

    // A capture waiting for its time keeps the timer running until a tick has taken it.
    #[test]
    fn a_waiting_capture_keeps_the_window_ticking() {
        let (mut app, _player) = app_on("p7_sine.json", test_catalogue());
        let _ = app.update(Message::PluginTick);
        assert!(!app.plugins_due(), "a quiet sine");
        app.capture_soon(SINE);
        assert!(app.plugins_due(), "the capture waits for a tick");
        tick_captures(&mut app);
        assert!(!app.plugins_due(), "taken, nothing waits");
    }

    // A capture that finds the bytes unchanged stores nothing and records no step.
    #[test]
    fn an_unchanged_capture_records_nothing() {
        let (mut app, _player) = app_on("p7_sine.json", test_catalogue());
        app.capture_soon(SINE);
        tick_captures(&mut app);
        assert_eq!(app.session.history().undo_len(), 0);
        assert!(!app.session.is_dirty());
    }

    // The parameter panel open on a plugin channel
    // follows another plugin given to that channel (a plugin let go of on its header): its
    // title and rows are the new plugin's. It was built once and kept the old plugin's.
    #[test]
    fn the_parameter_panel_follows_a_plugin_replaced_under_it() {
        let mut catalogue = test_catalogue();
        catalogue.files.extend(catalogue_of(variants_file()).files);
        let timer = catalogue
            .offered(PluginKind::Instrument)
            .into_iter()
            .find(|plugin| plugin.plugin.name == "Dawsome Test Timer")
            .expect("listed");
        let (mut app, _player) = app_on("p7_sine.json", catalogue);
        parameters_from_menu(&mut app, ChannelId(1));
        let title = |app: &App| app.panel.as_ref().map(|panel| panel.title.clone());
        assert_eq!(
            title(&app).as_deref(),
            Some("Dawsome Test Sine \u{2014} Dawsome Test Sine")
        );
        let source = dawsome_engine::project::SoundSource::plugin(plugin_processor(&timer));
        app.replace_sound(ChannelId(1), source, &timer.plugin.name);
        assert_eq!(app.error, None);
        assert_eq!(
            title(&app).as_deref(),
            Some("Dawsome Test Timer \u{2014} Dawsome Test Timer")
        );
        let host = clap(&app.session).expect("the host");
        let rows: Vec<u32> = app
            .panel
            .as_ref()
            .map(|panel| panel.params.iter().map(|param| param.id).collect())
            .unwrap_or_default();
        let wanted: Vec<u32> = host
            .params(SINE)
            .into_iter()
            .filter(|param| !param.hidden)
            .map(|param| param.id)
            .collect();
        assert_eq!(rows, wanted, "the new plugin's rows");
    }

    // Rack → "+ Plugin instrument…" → the picker; Enter adds the first instrument listed as a
    // plugin channel named after it, selected, one undo step.
    #[test]
    fn the_picker_adds_a_plugin_channel() {
        let (mut app, _player) = app_on("p7_sine.json", test_catalogue());
        let _ = app.update(Message::OpenPicker(PickerTarget::Instrument));
        let lines = app.picker_lines();
        assert_eq!(
            lines
                .iter()
                .map(picker_view::Line::label)
                .collect::<Vec<_>>(),
            ["Dawsome Test Sine"]
        );
        let _ = app.update(Message::Picker(PickerMessage::Submit));
        assert!(app.picker.is_none(), "closed");
        let channel = &app.session.project().channels[&ChannelId(2)];
        assert_eq!(channel.name, "Dawsome Test Sine");
        assert_eq!(channel.kind, ChannelKind::Plugin);
        let processor = channel.processor.as_ref().expect("a plugin");
        assert_eq!(processor.id, "clap:org.dawsome.test.sine");
        assert_eq!(
            processor.state, None,
            "the plugin's defaults until a capture"
        );
        assert_eq!(app.selected_channel, Some(ChannelId(2)));
        let _ = app.update(Message::Undo);
        assert!(!app.session.project().channels.contains_key(&ChannelId(2)));
    }

    // The instrument the picker adds is selected as a click on its name selects it: live notes,
    // from a device and from the typing keyboard, play it.
    #[test]
    fn the_picker_hands_the_live_notes_to_the_channel_it_adds() {
        let (mut app, mut player) = app_on("p7_sine.json", test_catalogue());
        let _ = app.update(Message::OpenPicker(PickerTarget::Instrument));
        let _ = app.update(Message::Picker(PickerMessage::Submit));
        assert_eq!(app.selected_channel, Some(ChannelId(2)));
        assert_eq!(
            super::super::tests::live_note_channels(&mut app, &mut player),
            [ChannelId(2), ChannelId(2)]
        );
    }

    // Mixer → empty slot → its menu's last row → the picker for effects; choosing the test gain
    // puts it in that slot.
    #[test]
    fn the_slot_menus_last_row_picks_a_plugin_effect() {
        let (mut app, _player) = app_on("p7_sine.json", test_catalogue());
        let _ = app.update(Message::TogglePanel(Panel::Mixer));
        let width = app.mixer.size().width;
        let slot = mixer_layout::slot_rect(0, width).center();
        let press = |app: &mut App, at| {
            let message = MixerMessage::Pressed {
                at,
                button: iced::mouse::Button::Left,
                double: false,
            };
            let _ = app.update(Message::Mixer(message));
        };
        press(&mut app, slot);
        assert_eq!(app.mixer.menu, Some(Menu::Add { slot: 0 }));
        let chosen = MixerMessage::MenuChosen(ADD_MENU.len());
        let _ = app.update(Message::Mixer(chosen));
        assert_eq!(
            app.picker.as_ref().map(|picker| picker.target),
            Some(PickerTarget::Effect {
                insert: InsertId::MASTER,
                slot: 0
            })
        );
        let _ = app.update(Message::Picker(PickerMessage::Submit));
        let (insert, effect) = app
            .session
            .project()
            .mixer
            .effect(EffectId(1))
            .expect("added");
        assert_eq!(insert, InsertId::MASTER);
        assert_eq!(effect.processor.id, "clap:org.dawsome.test.gain");
        assert_eq!(effect.processor.display_name(), "Dawsome Test Gain");
    }

    // A project whose plugins are not installed opens, with a banner naming each missing plugin.
    #[test]
    fn missing_plugins_are_named_in_the_toolbar() {
        let (app, _player) = app_on("p7_missing.json", Catalogue::default());
        let notice = app.notice.as_deref().unwrap_or_default();
        assert!(
            notice.contains("Missing plugin: Missing Synth (clap:org.example.missing)"),
            "{notice}"
        );
        assert!(notice.contains("Missing FX"), "{notice}");
    }
}
