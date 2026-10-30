//! The CLAP host on the main thread: it creates plugin instances for the compiler, keeps them
//! (the instance stays on this thread; only its audio half moves to the player), answers the
//! app's questions about their parameters and state, shows their windows, runs their timers,
//! file descriptors and callbacks, and lets an instance go once its processor has left the plan
//! (or its session has ended) and its audio half has come back.

use std::cell::{Cell, Ref, RefCell};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use clack_extensions::audio_ports::{AudioPortInfoBuffer, PluginAudioPorts};
use clack_extensions::gui::{GuiConfiguration, GuiSize, PluginGui};
use clack_extensions::note_ports::{NoteDialects, NotePortInfoBuffer, PluginNotePorts};
use clack_extensions::params::{ParamInfoBuffer, ParamInfoFlags, PluginParams};
#[cfg(unix)]
use clack_extensions::posix_fd::{FdFlags, PluginPosixFd};
use clack_extensions::state::PluginState;
use clack_extensions::timer::PluginTimer;
use clack_host::prelude::{
    HostInfo, PluginAudioConfiguration, PluginEntry, PluginInstance, PluginInstanceError,
};
use clack_host::utils::ClapId;
use dawsome_engine::processor::{
    Captured, Editor, HostError, HostNotice, NoticeKind, PluginParam, Processor, ProcessorHost,
};
use dawsome_engine::project::{ProcessorOwner, ProcessorRef};
use dawsome_engine::realtime::BLOCK_FRAMES;

use crate::catalogue::{Catalogue, FileRecord, PluginFormat};
use crate::handlers::{DawsomeHost, MainState, SharedState, Signals, IS_MAIN};
use crate::processor::{ClapProcessor, Dialect, Layout, Role};
use crate::sizing::{Answer, Sizing};
use crate::window::{self, as_parent, Display, WindowEvent, WindowId};

/// The id prefix of CLAP plugins in a project.
pub const FORMAT: &str = "clap";

/// The most channels one audio port of a hosted plugin may have.
const MAX_PORT_CHANNELS: u32 = 32;

/// The CLAP host. Cloning gives another handle to the same host, so the compiler's registry and
/// the app can both hold it; all of it stays on the thread that made it.
#[derive(Clone)]
pub struct ClapHost {
    inner: Rc<Inner>,
    /// Whose instances these are: those created through this handle and its clones, which go
    /// when all of them are gone (see [`ClapHost::session`]).
    scope: Rc<()>,
}

struct Inner {
    catalogue: RefCell<Catalogue>,
    info: HostInfo,
    /// Loaded plugin files, so each is loaded once however many instances it has.
    entries: RefCell<BTreeMap<PathBuf, PluginEntry>>,
    instances: RefCell<Vec<Instance>>,
    /// The connection plugin windows use, made when the first one opens.
    display: RefCell<Option<Display>>,
    /// What went wrong outside a tick, for the next one to report.
    pending: RefCell<Vec<HostNotice>>,
    created: Cell<usize>,
    /// Rung by every instance's requests ([`Signals::raise`]).
    doorbell: Arc<AtomicBool>,
    /// Our plugin windows are kept above other windows ([`ProcessorHost::keep_windows_above`]).
    above: Cell<bool>,
}

/// A plugin's extensions the host side uses.
#[derive(Clone, Copy, Default)]
struct Extensions {
    params: Option<PluginParams>,
    state: Option<PluginState>,
    gui: Option<PluginGui>,
    timer: Option<PluginTimer>,
    #[cfg(unix)]
    fd: Option<PluginPosixFd>,
    audio_ports: Option<PluginAudioPorts>,
    note_ports: Option<PluginNotePorts>,
}

/// An open plugin interface.
#[derive(Debug, Clone, Copy)]
struct EditorState {
    kind: Editor,
    /// Our window it is embedded in; none for a floating one.
    window: Option<WindowId>,
    /// Our window's size around an embedded one: the size kept, and its settling.
    sizing: Option<Sizing>,
}

/// A live plugin instance.
struct Instance {
    owner: ProcessorOwner,
    plugin: PluginInstance<DawsomeHost>,
    signals: Arc<Signals>,
    /// The audio half holds the other handle; once only this one is left, the audio half has
    /// been dropped, and the instance can go if it is also released or its session is over.
    alive: Arc<()>,
    /// The session whose compiler created it; once that is gone, so is the document that had
    /// the plugin.
    scope: Weak<()>,
    /// Its processor left the plan.
    released: bool,
    extensions: Extensions,
    /// The state text the document holds for it, so a capture followed by its edit reloads
    /// nothing.
    synced: Option<String>,
    /// Its own state right after it was created without a saved one: what clearing the
    /// document's state (undoing the first capture) loads back.
    initial: Option<Vec<u8>>,
    editor: Option<EditorState>,
    /// The newest instance for its `owner`, the one questions are answered for. An older one is
    /// on its way out (its `owner` was removed and added again, or it is being recreated).
    current: bool,
}

impl std::fmt::Debug for ClapHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClapHost")
            .field("instances", &self.instances())
            .field("created", &self.created())
            .finish()
    }
}

impl ClapHost {
    /// A host that finds plugins in `catalogue`. Its thread becomes the plugins' main thread.
    pub fn new(catalogue: Catalogue) -> Self {
        IS_MAIN.with(|flag| flag.set(true));
        let info = HostInfo::new("Dawsome", "Dawsome", "", env!("CARGO_PKG_VERSION"))
            .unwrap_or_else(|_| unreachable!("these strings hold no NUL byte"));
        Self {
            inner: Rc::new(Inner {
                catalogue: RefCell::new(catalogue),
                info,
                entries: RefCell::new(BTreeMap::new()),
                instances: RefCell::new(Vec::new()),
                display: RefCell::new(None),
                pending: RefCell::new(Vec::new()),
                created: Cell::new(0),
                doorbell: Arc::default(),
                above: Cell::new(false),
            }),
            scope: Rc::new(()),
        }
    }

    /// Another handle to this host, for one session's compiler: the instances created through
    /// it (and its clones) go once all of them are gone and their audio halves have come back,
    /// as the document that had them is gone. Why a scope of its own: a new document's session
    /// is made while the old one still lives, with the same channel and slot ids.
    pub fn session(&self) -> Self {
        Self {
            inner: Rc::clone(&self.inner),
            scope: Rc::new(()),
        }
    }

    /// The catalogue plugins are found in.
    pub fn catalogue(&self) -> Ref<'_, Catalogue> {
        self.inner.catalogue.borrow()
    }

    /// Replaces the catalogue (after a rescan). Running instances are not touched.
    pub fn set_catalogue(&self, catalogue: Catalogue) {
        *self.inner.catalogue.borrow_mut() = catalogue;
    }

    /// How many instances this host has created.
    pub fn created(&self) -> usize {
        self.inner.created.get()
    }

    /// How many instances are alive now, the ones on their way out included.
    pub fn instances(&self) -> usize {
        self.inner.instances.borrow().len()
    }

    /// Notes `owner`'s plugin was not sent because too many of its notes were sounding.
    pub fn dropped_notes(&self, owner: ProcessorOwner) -> u64 {
        self.inner
            .instances
            .borrow()
            .iter()
            .filter(|instance| instance.owner == owner)
            .map(|instance| instance.signals.dropped_notes.load(Ordering::Relaxed))
            .sum()
    }

    /// Our window `owner`'s interface is embedded in, while it is open (the window tests look at
    /// it on a connection of their own).
    pub fn editor_window(&self, owner: ProcessorOwner) -> Option<WindowId> {
        self.inner
            .instances
            .borrow()
            .iter()
            .rev()
            .find(|instance| instance.owner == owner && instance.current)
            .and_then(|instance| instance.editor)
            .and_then(|editor| editor.window)
    }

    /// The plugin file `record` names, loaded once, and only as the scan saw it.
    fn entry(&self, record: &FileRecord) -> Result<PluginEntry, HostError> {
        let file = &record.file;
        if let Some(entry) = self.inner.entries.borrow().get(file) {
            return Ok(entry.clone());
        }
        // Why compared first: a file replaced since its scan has not been loaded by a scan
        // child, and loading it here would let it crash the whole app.
        if !record.is_current() {
            return Err(HostError::Failed(
                "its file changed after the last plugin scan".into(),
            ));
        }
        // SAFETY: loading a plugin file runs its code. The file is one a scan child loaded
        // without crashing and is unchanged since (same size and modification time); that is as
        // far as a host can vouch for a plugin it runs in process.
        let entry = unsafe { PluginEntry::load(file) }.map_err(|error| {
            HostError::Failed(format!("cannot load {}: {error}", file.display()))
        })?;
        self.inner
            .entries
            .borrow_mut()
            .insert(file.to_owned(), entry.clone());
        Ok(entry)
    }

    fn create_instance(
        &self,
        owner: ProcessorOwner,
        reference: &ProcessorRef,
        sample_rate: u32,
    ) -> Result<Box<dyn Processor>, HostError> {
        let id = reference
            .id
            .strip_prefix("clap:")
            .ok_or(HostError::Missing)?;
        let record = {
            let catalogue = self.inner.catalogue.borrow();
            catalogue
                .find(PluginFormat::Clap, id)
                .map(|(file, _)| file.clone())
        }
        .ok_or(HostError::Missing)?;
        let entry = self.entry(&record)?;
        let saved = reference
            .state
            .as_deref()
            .map(|text| {
                BASE64
                    .decode(text)
                    .map_err(|_| HostError::Failed("its saved state is not base64".into()))
            })
            .transpose()?;
        let plugin_id = CString::new(id).map_err(|_| HostError::Missing)?;
        let signals = Arc::new(Signals::ringing(Arc::clone(&self.inner.doorbell)));
        let shared_signals = Arc::clone(&signals);
        let mut plugin = PluginInstance::<DawsomeHost>::new(
            |_| SharedState {
                signals: shared_signals,
            },
            |shared| MainState::new(shared),
            &entry,
            &plugin_id,
            &self.inner.info,
        )
        .map_err(|error| HostError::Failed(format!("would not start: {error}")))?;
        let extensions = query_extensions(&plugin);
        let initial = match (&saved, extensions.state) {
            (None, Some(state)) => save_state(&mut plugin, state).ok(),
            _ => None,
        };
        if let Some(bytes) = &saved {
            let state = extensions
                .state
                .ok_or_else(|| HostError::Failed("it keeps no state".into()))?;
            state
                .load(&plugin.plugin_handle(), &mut bytes.as_slice())
                .map_err(|_| HostError::Failed("it refused its saved state".into()))?;
            forget_own_load(&signals);
        }
        let role = match owner {
            ProcessorOwner::Channel(_) => Role::Instrument,
            ProcessorOwner::Effect(_) => Role::Effect,
        };
        let layout = layout(&mut plugin, &extensions, role)?;
        let audio = plugin
            .activate(
                |_, _| (),
                PluginAudioConfiguration {
                    sample_rate: f64::from(sample_rate),
                    min_frames_count: 1,
                    max_frames_count: BLOCK_FRAMES as u32,
                },
            )
            .map_err(|error| HostError::Failed(format!("would not activate: {error}")))?;
        let alive = Arc::new(());
        let processor = ClapProcessor::new(
            audio.into(),
            layout,
            Arc::clone(&signals),
            Arc::clone(&alive),
        );
        let mut instances = self.inner.instances.borrow_mut();
        for older in instances.iter_mut().filter(|other| other.owner == owner) {
            older.current = false;
        }
        instances.push(Instance {
            owner,
            plugin,
            signals,
            alive,
            scope: Rc::downgrade(&self.scope),
            released: false,
            extensions,
            synced: reference.state.clone(),
            initial,
            editor: None,
            current: true,
        });
        self.inner.created.set(self.inner.created.get() + 1);
        Ok(Box::new(processor))
    }

    /// Runs `work` on `owner`'s current instance.
    fn with_instance<T>(
        &self,
        owner: ProcessorOwner,
        work: impl FnOnce(&mut Instance) -> T,
    ) -> Option<T> {
        let mut instances = self.inner.instances.borrow_mut();
        instances
            .iter_mut()
            .rev()
            .find(|instance| instance.owner == owner && instance.current)
            .map(work)
    }

    /// The window connection, made on first use.
    fn display(&self) -> Result<Ref<'_, Display>, HostError> {
        if self.inner.display.borrow().is_none() {
            let display = Display::connect().map_err(HostError::Failed)?;
            *self.inner.display.borrow_mut() = Some(display);
        }
        Ref::filter_map(self.inner.display.borrow(), Option::as_ref)
            .map_err(|_| HostError::Failed(window::UNAVAILABLE.into()))
    }

    /// Takes `instance`'s interface down, then its window.
    fn close(&self, instance: &mut Instance) {
        instance.close_editor(self.inner.display.borrow().as_ref());
    }

    /// Opens `instance`'s interface: embedded in a window of ours when the plugin can, else
    /// floating, else not at all.
    fn open(&self, instance: &mut Instance, title: &str) -> Result<Editor, HostError> {
        let Some(gui) = instance.extensions.gui else {
            return Ok(Editor::Generic);
        };
        let display = self.display()?;
        let handle = instance.plugin.plugin_handle();
        let embedded = GuiConfiguration {
            api_type: window::API,
            is_floating: false,
        };
        if gui.is_api_supported(&handle, embedded) && gui.create(&handle, embedded).is_ok() {
            let size = gui.get_size(&handle).unwrap_or(GuiSize {
                width: 640,
                height: 480,
            });
            let resizable = gui.can_resize(&handle);
            match display.open(title, size.width, size.height, resizable) {
                Ok(window) => {
                    // SAFETY: `window` is a live window of ours; it is destroyed only after the
                    // plugin's interface is (see `close`).
                    let parented = unsafe { gui.set_parent(&handle, as_parent(window)) };
                    if parented.is_ok() && gui.show(&handle).is_ok() {
                        if self.inner.above.get() {
                            display.set_above(window, true);
                        }
                        instance.editor = Some(EditorState {
                            kind: Editor::Embedded,
                            window: Some(window),
                            sizing: Some(Sizing::new(
                                (size.width, size.height),
                                resizable,
                                Instant::now(),
                            )),
                        });
                        return Ok(Editor::Embedded);
                    }
                    gui.destroy(&handle);
                    display.close(window);
                }
                Err(_) => gui.destroy(&handle),
            }
        }
        let floating = GuiConfiguration {
            api_type: window::API,
            is_floating: true,
        };
        if gui.is_api_supported(&handle, floating) && gui.create(&handle, floating).is_ok() {
            if let Ok(title) = CString::new(title) {
                gui.suggest_title(&handle, &title);
            }
            if gui.show(&handle).is_ok() {
                instance.editor = Some(EditorState {
                    kind: Editor::Floating,
                    window: None,
                    sizing: None,
                });
                return Ok(Editor::Floating);
            }
            gui.destroy(&handle);
        }
        Ok(Editor::Generic)
    }

    /// The window manager told us our window `window` around `instance`'s embedded interface is
    /// `size` (the user resized it): the interface's `adjust_size` holds it, and our window and
    /// the interface take what [`Sizing::user_resized`] answers.
    fn user_resized(
        &self,
        instance: &mut Instance,
        window: WindowId,
        size: (u32, u32),
        now: Instant,
    ) {
        let (Some(gui), Some(editor)) = (instance.extensions.gui, instance.editor.as_mut()) else {
            return;
        };
        let Some(sizing) = editor.sizing.as_mut() else {
            return;
        };
        let handle = instance.plugin.plugin_handle();
        let answer = sizing.user_resized(size, now, |(width, height)| {
            gui.adjust_size(&handle, GuiSize { width, height })
                .map(|held| (held.width, held.height))
        });
        let display = self.inner.display.borrow();
        let Some(display) = display.as_ref() else {
            return;
        };
        match answer {
            Answer::Nothing => {}
            Answer::PutBack((width, height)) => display.resize(window, width, height),
            Answer::Take {
                size: (width, height),
                window: resize,
            } => {
                if resize {
                    display.resize(window, width, height);
                }
                let _ = gui.set_size(&handle, GuiSize { width, height });
            }
        }
    }

    /// `instance`'s plugin asked for `size` (`request_resize`): our window takes it unless it is
    /// the size kept, a fixed interface's with its least and most size that size (its hints were
    /// the old size's).
    fn plugin_asked(&self, instance: &mut Instance, size: (u32, u32), now: Instant) {
        let Some(editor) = instance.editor.as_mut() else {
            return;
        };
        let (Some(window), Some(sizing)) = (editor.window, editor.sizing.as_mut()) else {
            return;
        };
        let Some((width, height)) = sizing.plugin_asked(size, now) else {
            return;
        };
        if let Some(display) = self.inner.display.borrow().as_ref() {
            if sizing.resizable() {
                display.resize(window, width, height);
            } else {
                display.handle(window).fix(width, height);
            }
        }
    }

    /// Asks `instance`'s window again for the size kept when the window manager left it at
    /// another and has been quiet a while ([`Sizing::due`]).
    fn settle(&self, instance: &mut Instance, now: Instant) {
        let Some(editor) = instance.editor.as_mut() else {
            return;
        };
        let (Some(window), Some(sizing)) = (editor.window, editor.sizing.as_mut()) else {
            return;
        };
        if let (Some((width, height)), Some(display)) =
            (sizing.due(now), self.inner.display.borrow().as_ref())
        {
            display.resize(window, width, height);
        }
    }
}

fn query_extensions(plugin: &PluginInstance<DawsomeHost>) -> Extensions {
    let shared = plugin.plugin_shared_handle();
    Extensions {
        params: shared.get_extension(),
        state: shared.get_extension(),
        gui: shared.get_extension(),
        timer: shared.get_extension(),
        #[cfg(unix)]
        fd: shared.get_extension(),
        audio_ports: shared.get_extension(),
        note_ports: shared.get_extension(),
    }
}

/// Clears the dirty flag a state load the host made itself raised. Why: a plugin answers a load
/// with a parameter rescan, which marks it dirty, and capturing that would put the plugin's own
/// reading of the document's state back into the document as a change the user never made.
fn forget_own_load(signals: &Signals) {
    Signals::take(&signals.dirty);
}

fn save_state(
    plugin: &mut PluginInstance<DawsomeHost>,
    state: PluginState,
) -> Result<Vec<u8>, HostError> {
    let mut bytes = Vec::new();
    state
        .save(&plugin.plugin_handle(), &mut bytes)
        .map_err(|_| HostError::Failed("it would not save its state".into()))?;
    Ok(bytes)
}

/// The ports the audio half sizes its buffers for, and how the plugin takes notes. A plugin
/// whose main output is neither mono nor stereo, or an effect without a mono or stereo main
/// input, cannot be hosted.
fn layout(
    plugin: &mut PluginInstance<DawsomeHost>,
    extensions: &Extensions,
    role: Role,
) -> Result<Layout, HostError> {
    let handle = plugin.plugin_handle();
    let ports = |is_input| -> Result<Vec<u32>, HostError> {
        let Some(audio) = extensions.audio_ports else {
            return Ok(Vec::new());
        };
        (0..audio.count(&handle, is_input))
            .map(|index| {
                let mut buffer = AudioPortInfoBuffer::new();
                let channels = audio
                    .get(&handle, index, is_input, &mut buffer)
                    .map_or(0, |port| port.channel_count);
                if channels > MAX_PORT_CHANNELS {
                    return Err(HostError::Failed("it has too many audio channels".into()));
                }
                Ok(channels)
            })
            .collect()
    };
    let (inputs, outputs) = (ports(true)?, ports(false)?);
    let mono_or_stereo = |ports: &[u32]| matches!(ports.first(), Some(1 | 2));
    if !mono_or_stereo(&outputs) {
        return Err(HostError::Failed(
            "its main output is not mono or stereo".into(),
        ));
    }
    if role == Role::Effect && !mono_or_stereo(&inputs) {
        return Err(HostError::Failed("it has no mono or stereo input".into()));
    }
    let supported = match (role, extensions.note_ports) {
        (Role::Instrument, Some(notes)) if notes.count(&handle, true) > 0 => {
            let mut buffer = NotePortInfoBuffer::new();
            notes
                .get(&handle, 0, true, &mut buffer)
                .map_or(NoteDialects::empty(), |port| port.supported_dialects)
        }
        _ => NoteDialects::empty(),
    };
    let dialect = if supported.contains(NoteDialects::CLAP) {
        Dialect::Clap
    } else if supported.contains(NoteDialects::MIDI) {
        Dialect::Midi
    } else {
        Dialect::None
    };
    Ok(Layout {
        role,
        dialect,
        takes_midi: supported.contains(NoteDialects::MIDI),
        inputs,
        outputs,
        max_frames: BLOCK_FRAMES,
        params: extensions.params,
    })
}

/// `poll(2)`'s events for a plugin's file descriptor flags, and back.
#[cfg(unix)]
fn poll_events(flags: FdFlags) -> i16 {
    let mut events = 0;
    if flags.contains(FdFlags::READ) {
        events |= libc::POLLIN;
    }
    if flags.contains(FdFlags::WRITE) {
        events |= libc::POLLOUT;
    }
    events
}

#[cfg(unix)]
fn fd_flags(revents: i16) -> FdFlags {
    let mut flags = FdFlags::empty();
    if revents & libc::POLLIN != 0 {
        flags |= FdFlags::READ;
    }
    if revents & libc::POLLOUT != 0 {
        flags |= FdFlags::WRITE;
    }
    if revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
        flags |= FdFlags::ERROR;
    }
    flags
}

impl Instance {
    /// Whether it has main-thread work that a tick must take without a new request: its window
    /// is open, it registered a timer or a file descriptor, it is on its way out (released or
    /// replaced, or its session over; it goes once its audio half is back), or a request it
    /// made has not been served yet.
    fn needs_ticks(&self) -> bool {
        self.editor.is_some()
            || !self.current
            || self.scope.strong_count() == 0
            || self.signals.pending()
            || self.plugin.access_handler(|main| main.waits_on_the_clock())
    }

    /// Closes its interface, if open: the plugin's side first, then the window it sat in.
    fn close_editor(&mut self, display: Option<&Display>) {
        let Some(editor) = self.editor.take() else {
            return;
        };
        if let Some(gui) = self.extensions.gui {
            gui.destroy(&self.plugin.plugin_handle());
        }
        if let (Some(window), Some(display)) = (editor.window, display) {
            display.close(window);
        }
    }

    /// Runs the main-thread work the plugin is due: its callback, its timers, its file
    /// descriptors.
    fn serve(&mut self, now: Instant) {
        if Signals::take(&self.signals.callback) {
            self.plugin.call_on_main_thread_callback();
        }
        if let Some(timer) = self.extensions.timer {
            let due = self.plugin.access_handler(|main| main.due_timers(now));
            for id in due {
                timer.on_timer(&self.plugin.plugin_handle(), id);
            }
        }
        #[cfg(unix)]
        self.serve_fds();
    }

    /// Tells the plugin which of its file descriptors are ready.
    #[cfg(unix)]
    fn serve_fds(&mut self) {
        if let Some(fd) = self.extensions.fd {
            let registered = self.plugin.access_handler(|main| main.fds());
            if registered.is_empty() {
                return;
            }
            let mut polled: Vec<libc::pollfd> = registered
                .iter()
                .map(|&(fd, flags)| libc::pollfd {
                    fd,
                    events: poll_events(flags),
                    revents: 0,
                })
                .collect();
            let count = libc::nfds_t::try_from(polled.len()).unwrap_or(libc::nfds_t::MAX);
            // SAFETY: `polled` is a live array of `count` pollfd records, and a zero timeout
            // returns at once.
            let ready = unsafe { libc::poll(polled.as_mut_ptr(), count, 0) };
            if ready <= 0 {
                return;
            }
            for entry in polled.iter().filter(|entry| entry.revents != 0) {
                fd.on_fd(
                    &self.plugin.plugin_handle(),
                    entry.fd,
                    fd_flags(entry.revents),
                );
            }
        }
    }
}

impl Drop for Inner {
    // The host goes (the app quits) with plugin interfaces still open: each is closed before
    // its plugin is destroyed, as on every other way out. Why: a plugin destroyed with its
    // interface open can leave the interface's threads running, and unloading its file then
    // takes their code away (a third-party plugin with its interface open crashed the app on
    // quit with SIGSEGV).
    fn drop(&mut self) {
        let display = self.display.get_mut().as_ref();
        for instance in self.instances.get_mut() {
            instance.close_editor(display);
        }
    }
}

impl ProcessorHost for ClapHost {
    fn format(&self) -> &'static str {
        FORMAT
    }

    fn create(
        &self,
        owner: ProcessorOwner,
        reference: &ProcessorRef,
        sample_rate: u32,
    ) -> Result<Box<dyn Processor>, HostError> {
        self.create_instance(owner, reference, sample_rate)
    }

    fn release(&self, owner: ProcessorOwner) {
        let scope = Rc::as_ptr(&self.scope);
        for instance in self
            .inner
            .instances
            .borrow_mut()
            .iter_mut()
            .filter(|instance| instance.owner == owner && instance.scope.as_ptr() == scope)
        {
            instance.released = true;
            instance.current = false;
        }
    }

    fn sync_state(&self, owner: ProcessorOwner, reference: &ProcessorRef) {
        let failed = self.with_instance(owner, |instance| {
            if instance.synced == reference.state {
                return None;
            }
            let bytes = match &reference.state {
                Some(text) => BASE64.decode(text).ok()?,
                None => instance.initial.clone()?,
            };
            let state = instance.extensions.state?;
            let loaded = state.load(&instance.plugin.plugin_handle(), &mut bytes.as_slice());
            forget_own_load(&instance.signals);
            instance.synced = reference.state.clone();
            loaded.is_err().then_some(())
        });
        if failed.flatten().is_some() {
            self.inner.pending.borrow_mut().push(HostNotice {
                owner,
                kind: NoticeKind::Failed("it refused the state it was given".into()),
            });
        }
    }

    fn params(&self, owner: ProcessorOwner) -> Vec<PluginParam> {
        self.with_instance(owner, |instance| {
            let Some(params) = instance.extensions.params else {
                return Vec::new();
            };
            let handle = instance.plugin.plugin_handle();
            (0..params.count(&handle))
                .filter_map(|index| {
                    let mut buffer = ParamInfoBuffer::new();
                    let info = params.get_info(&handle, index, &mut buffer)?;
                    Some(PluginParam {
                        id: info.id.get(),
                        name: String::from_utf8_lossy(info.name).into_owned(),
                        group: String::from_utf8_lossy(info.module).into_owned(),
                        min: info.min_value,
                        max: info.max_value,
                        default: info.default_value,
                        stepped: info.flags.contains(ParamInfoFlags::IS_STEPPED),
                        hidden: info.flags.contains(ParamInfoFlags::IS_HIDDEN),
                        read_only: info.flags.contains(ParamInfoFlags::IS_READONLY),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
    }

    fn value(&self, owner: ProcessorOwner, id: u32) -> Option<f64> {
        self.with_instance(owner, |instance| {
            let params = instance.extensions.params?;
            params.get_value(&instance.plugin.plugin_handle(), ClapId::from_raw(id)?)
        })
        .flatten()
    }

    fn value_text(&self, owner: ProcessorOwner, id: u32, value: f64) -> Option<String> {
        self.with_instance(owner, |instance| {
            let params = instance.extensions.params?;
            let mut buffer = [0u8; 256];
            let text = params
                .value_to_text(
                    &instance.plugin.plugin_handle(),
                    ClapId::from_raw(id)?,
                    value,
                    &mut buffer,
                )
                .ok()?;
            Some(String::from_utf8_lossy(text).into_owned())
        })
        .flatten()
    }

    fn capture_state(&self, owner: ProcessorOwner) -> Result<Captured, HostError> {
        self.with_instance(owner, |instance| {
            let state = instance
                .extensions
                .state
                .ok_or_else(|| HostError::Failed("it keeps no state".into()))?;
            let bytes = save_state(&mut instance.plugin, state)?;
            let text = BASE64.encode(&bytes);
            instance.synced = Some(text.clone());
            Ok(Captured {
                state: text,
                bytes: bytes.len(),
            })
        })
        .unwrap_or(Err(HostError::Missing))
    }

    fn keep_windows_above(&self, above: bool) {
        if self.inner.above.replace(above) == above {
            return;
        }
        let display = self.inner.display.borrow();
        let Some(display) = display.as_ref() else {
            return;
        };
        for instance in self.inner.instances.borrow().iter() {
            if let Some(window) = instance.editor.and_then(|editor| editor.window) {
                display.set_above(window, above);
            }
        }
    }

    fn open_editor(&self, owner: ProcessorOwner, title: &str) -> Result<Editor, HostError> {
        let mut instances = self.inner.instances.borrow_mut();
        let instance = instances
            .iter_mut()
            .rev()
            .find(|instance| instance.owner == owner && instance.current)
            .ok_or(HostError::Missing)?;
        if let Some(editor) = instance.editor {
            return Ok(editor.kind);
        }
        self.open(instance, title)
    }

    fn tick(&self) -> Vec<HostNotice> {
        let mut notices = std::mem::take(&mut *self.inner.pending.borrow_mut());
        let events = self
            .inner
            .display
            .borrow()
            .as_ref()
            .map(Display::events)
            .unwrap_or_default();
        let mut instances = self.inner.instances.borrow_mut();
        // A window's last size in this batch is the one it has: the earlier ones (a window
        // manager tells some twice, a drag many) would each ask its interface again.
        let last_size = |index: usize, window: WindowId| {
            !events[index + 1..].iter().any(|later| {
                matches!(later, WindowEvent::Resized { window: other, .. } if *other == window)
            })
        };
        for (index, &event) in events.iter().enumerate() {
            let window = event.window();
            if matches!(event, WindowEvent::Resized { .. }) && !last_size(index, window) {
                continue;
            }
            let Some(instance) = instances
                .iter_mut()
                .find(|instance| instance.editor.is_some_and(|e| e.window == Some(window)))
            else {
                continue;
            };
            match event {
                WindowEvent::Closed(_) => {
                    self.close(instance);
                    notices.push(HostNotice {
                        owner: instance.owner,
                        kind: NoticeKind::Closed,
                    });
                }
                WindowEvent::Resized { width, height, .. } => {
                    self.user_resized(instance, window, (width, height), Instant::now());
                }
            }
        }
        let now = Instant::now();
        for instance in instances.iter_mut() {
            instance.serve(now);
            let resize = instance.signals.resize.swap(0, Ordering::AcqRel);
            if resize != 0 {
                let size = GuiSize::unpack_from_u64(resize);
                self.plugin_asked(instance, (size.width, size.height), now);
            }
            // A size the window manager left our window at after a user's resize is asked again
            // once it is quiet (the settling VST3's frame runs too).
            self.settle(instance, now);
            if Signals::take(&instance.signals.closed) {
                self.close(instance);
                if instance.current {
                    notices.push(HostNotice {
                        owner: instance.owner,
                        kind: NoticeKind::Closed,
                    });
                }
            }
            if !instance.current {
                continue;
            }
            let owner = instance.owner;
            let flags = [
                (&instance.signals.dirty, NoticeKind::Dirty),
                (&instance.signals.restart, NoticeKind::Restart),
                (
                    &instance.signals.failed,
                    NoticeKind::Failed("it failed while processing".into()),
                ),
            ];
            for (flag, kind) in flags {
                if Signals::take(flag) {
                    notices.push(HostNotice { owner, kind });
                }
            }
        }
        // Instances released, or whose session is over, once their audio half has been
        // dropped: interface first, then deactivation, then the instance itself, all here on the
        // main thread.
        let mut gone = Vec::new();
        let mut index = 0;
        while index < instances.len() {
            let instance = &instances[index];
            let over = instance.released || instance.scope.strong_count() == 0;
            if !over || Arc::strong_count(&instance.alive) > 1 {
                index += 1;
                continue;
            }
            self.close(&mut instances[index]);
            match instances[index].plugin.try_deactivate() {
                Ok(()) | Err(PluginInstanceError::DeactivatedPlugin) => {
                    gone.push(instances.remove(index));
                }
                // The audio half is still being dropped elsewhere: next tick.
                Err(_) => index += 1,
            }
        }
        drop(instances);
        // Destroyed outside the borrow, in case a plugin calls back while it goes.
        drop(gone);
        notices
    }

    fn has_instances(&self) -> bool {
        !self.inner.instances.borrow().is_empty()
    }

    fn needs_ticks(&self) -> bool {
        !self.inner.pending.borrow().is_empty()
            || self
                .inner
                .instances
                .borrow()
                .iter()
                .any(Instance::needs_ticks)
    }

    fn doorbell(&self) -> Option<Arc<AtomicBool>> {
        Some(Arc::clone(&self.inner.doorbell))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Linux only: file descriptors are a POSIX extension, offered to plugins there alone.
    #[cfg(unix)]
    #[test]
    fn poll_flags_map_both_ways() {
        assert_eq!(
            poll_events(FdFlags::READ | FdFlags::WRITE),
            libc::POLLIN | libc::POLLOUT
        );
        assert_eq!(poll_events(FdFlags::ERROR), 0, "errors are always reported");
        assert_eq!(fd_flags(libc::POLLIN), FdFlags::READ);
        assert_eq!(fd_flags(libc::POLLHUP), FdFlags::ERROR);
    }

    // An open plugin interface keeps the pump on the clock: its window's events (closed,
    // resized) are read by a tick, not rung. No test opens a real window (that needs a
    // desktop), so the test sine's instance is given one as `open` records it.
    #[test]
    fn an_open_interface_needs_ticks() {
        use crate::catalogue::FileStatus;

        let file = crate::test_plugins::test_plugins().join("dawsome_test_plugin.clap");
        let plugins = crate::scan::describe_file(&file, || {}).expect("the test plugins load");
        let (size, mtime) = crate::catalogue::stamp(&file).expect("the file is there");
        let host = ClapHost::new(Catalogue {
            files: vec![FileRecord {
                file,
                format: PluginFormat::Clap,
                size,
                mtime,
                status: FileStatus::Ok,
                reason: None,
                plugins,
            }],
            ..Catalogue::default()
        });
        let owner = ProcessorOwner::Channel(dawsome_engine::project::ChannelId(1));
        let reference = ProcessorRef::plugin("clap:org.dawsome.test.sine", "Dawsome Test Sine");
        let _audio_half = host.create(owner, &reference, 48_000).expect("creates");
        assert!(host.tick().is_empty());
        assert!(!host.needs_ticks(), "a quiet sine");
        host.inner.instances.borrow_mut()[0].editor = Some(EditorState {
            kind: Editor::Floating,
            window: None,
            sizing: None,
        });
        assert!(host.needs_ticks(), "its interface is open");
    }

    #[test]
    fn an_id_the_catalogue_does_not_list_is_missing() {
        let host = ClapHost::new(Catalogue::default());
        let reference = ProcessorRef::plugin("clap:org.example.nothing", "Nothing");
        let owner = ProcessorOwner::Channel(dawsome_engine::project::ChannelId(1));
        assert!(matches!(
            host.create(owner, &reference, 48_000),
            Err(HostError::Missing)
        ));
        let native = ProcessorRef::new("dawsome.delay");
        assert!(matches!(
            host.create(owner, &native, 48_000),
            Err(HostError::Missing)
        ));
        assert!(!host.has_instances());
        assert!(host.tick().is_empty());
    }
}
