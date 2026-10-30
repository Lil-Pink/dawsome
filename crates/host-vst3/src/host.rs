//! The VST3 host on the main thread: it creates plugin instances for the compiler (only from
//! modules a scan child loaded unchanged), keeps them (the parts stay on this thread; only the
//! audio processor moves to the player), answers the app's questions about their parameters and
//! state, serves their run loops, carries values between the processor and the controller, and
//! lets an instance go once its processor has left the plan (or its session has ended) and its
//! audio half has come back. The shape is the CLAP host's (`dawsome_host_clap::ClapHost`): one
//! host cloned into each session's registry, instances scoped by session.
//!
//! A plugin's own editor opens in a window of ours (`view.rs`), from every door a CLAP plugin's
//! window has; one without one, or a desktop without X11, gets the app's parameter window. The
//! window's events are read and the editor's frame's run loop served by the tick, which runs
//! while an editor is open; the editor closes before its plugin's parts on every way out.
//!
//! What a plugin asks of its host with `restartComponent`: `kReloadComponent` and `kIoChanged`
//! are a [`NoticeKind::Restart`] (the app captures and creates it again); `kParamTitlesChanged`
//! (its parameters are read again) and `kMidiCCAssignmentChanged` (its MIDI mapping is) are a
//! [`NoticeKind::Params`] (a panel showing them is built again); `kParamValuesChanged` is
//! [`NoticeKind::Dirty`]; `kLatencyChanged` reads its latency again; the rest are not acted on.
//! What it asks before it is activated is forgotten: activation reads it all afresh.

use std::cell::{Cell, Ref, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::mem::ManuallyDrop;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use dawsome_engine::processor::{
    Captured, Editor, HostError, HostNotice, NoticeKind, PluginParam, Processor, ProcessorHost,
};
use dawsome_engine::project::{ProcessorOwner, ProcessorRef};
use dawsome_host_common::catalogue::{Catalogue, FileRecord, PluginFormat};
use dawsome_host_common::window::{Display, WindowEvent, WindowId, UNAVAILABLE};
use vst3::Steinberg::kResultOk;
use vst3::Steinberg::Vst::{IEditControllerTrait, ParameterInfo, String128};

use crate::com::frame;
use crate::com::handler::{Signals, EDITS};
use crate::com::{mark_main_thread, read_utf16};
use crate::constants::{
    HIDDEN, IO_CHANGED, LATENCY_CHANGED, MAPPING_CHANGED, READ_ONLY, RELOAD, TITLES_CHANGED,
    VALUES_CHANGED,
};
use crate::mapping::{self, Controls};
use crate::module::Module;
use crate::parts::{Parts, Role};
use crate::preset::{self, Preset};
use crate::processor::{normalized, Making, Value, Vst3Processor, VALUES};
use crate::uid;
use crate::view::{self, OpenView};

/// The id prefix of VST3 plugins in a project.
pub const FORMAT: &str = "vst3";

/// The VST3 host. Cloning gives another handle to the same host, so the compiler's registry and
/// the app can both hold it; all of it stays on the thread that made it.
#[derive(Clone)]
pub struct Vst3Host {
    inner: Rc<Inner>,
    /// Whose instances these are: those created through this handle and its clones, which go
    /// when all of them are gone (see [`Vst3Host::session`]).
    scope: Rc<()>,
}

struct Inner {
    catalogue: RefCell<Catalogue>,
    /// Loaded modules, so each is loaded once however many instances it has.
    modules: RefCell<BTreeMap<PathBuf, Arc<Module>>>,
    instances: RefCell<Vec<Rc<Instance>>>,
    /// What went wrong outside a tick, for the next one to report.
    pending: RefCell<Vec<HostNotice>>,
    created: Cell<usize>,
    /// Rung by every instance's requests, the audio thread's values included.
    doorbell: Arc<AtomicBool>,
    /// The connection plugin windows use, made when the first editor opens.
    display: RefCell<Option<Display>>,
    /// Our plugin windows are kept above other windows ([`ProcessorHost::keep_windows_above`]).
    above: Cell<bool>,
}

/// An open editor, as tests and diagnostics see it: the window of ours it sits in, and what it
/// registered with its frame's run loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenEditor {
    pub window: WindowId,
    pub timers: usize,
    pub fds: usize,
}

/// One parameter as the host keeps it: its id, its steps (0 for a continuous one) and whether
/// the plugin alone sets it.
#[derive(Debug, Clone, Copy)]
struct ParamShape {
    id: u32,
    steps: i32,
    read_only: bool,
}

/// A live plugin instance.
struct Instance {
    owner: ProcessorOwner,
    /// Closed and dropped with the instance, only once its audio half is gone (see `Drop`).
    parts: ManuallyDrop<Parts>,
    /// The audio half holds the other handle.
    alive: Arc<()>,
    scope: Weak<()>,
    /// Its processor left the plan.
    released: Cell<bool>,
    /// The newest instance for its `owner`, the one questions are answered for.
    current: Cell<bool>,
    /// The state text the document holds for it, so a capture followed by its edit reloads
    /// nothing.
    synced: RefCell<Option<String>>,
    /// Its own state right after it was created without a saved one: what clearing the
    /// document's state (undoing the first capture) loads back.
    initial: RefCell<Option<Vec<u8>>>,
    values: RefCell<rtrb::Consumer<Value>>,
    /// As its controller described them last (at its making, then at `kParamTitlesChanged`);
    /// the audio half's table of steps stays as it was made (a changed step count is a reload).
    params: RefCell<Vec<ParamShape>>,
    /// The parameters its MIDI mapping names: the MIDI input's, left out of the panels.
    mapped: RefCell<BTreeSet<u32>>,
    /// The parameters the audio half sends the bend and the wheel to.
    controls: Arc<Controls>,
    latency: Cell<u32>,
    /// Its own editor, while open. Never borrowed across a call into the plugin.
    editor: RefCell<Option<OpenView>>,
}

impl Instance {
    /// Our window its open editor sits in.
    fn editor_window(&self) -> Option<WindowId> {
        self.editor.borrow().as_ref().map(OpenView::window)
    }

    fn shape(&self, id: u32) -> Option<ParamShape> {
        self.params
            .borrow()
            .iter()
            .find(|param| param.id == id)
            .copied()
    }

    /// Reads its MIDI mapping again, for the audio half and the panels.
    fn read_mapping(&self) {
        let mapping = mapping::read(&self.parts);
        self.controls.set(&mapping);
        *self.mapped.borrow_mut() = mapping.mapped;
    }

    /// Hands the values the audio half sent back to the controller, so it (and the host's
    /// `value`) follows automation and the control ring; a value the plugin changed itself on
    /// a parameter it does not alone set marks its state changed, outside a gesture.
    fn hand_values_on(&self) {
        let values: Vec<Value> = {
            let mut consumer = self.values.borrow_mut();
            std::iter::from_fn(|| consumer.pop().ok()).collect()
        };
        let Some(controller) = &self.parts.controller else {
            return;
        };
        let signals = &self.parts.signals;
        for value in values {
            // SAFETY: the controller is live; called on the main thread, with no borrow held.
            unsafe { controller.setParamNormalized(value.id, value.value) };
            let writable = self.shape(value.id).is_some_and(|param| !param.read_only);
            if value.from_plugin && writable && signals.gestures.load(Ordering::Acquire) <= 0 {
                signals.raise(&signals.dirty);
            }
        }
    }

    /// Whether it has main-thread work that a tick must take without a new request: its editor
    /// is open (its window's events are read, its frame's run loop served by a tick), it is on its
    /// way out, a request it made has not been served, values wait for its controller, or its run
    /// loop has a registration.
    fn needs_ticks(&self) -> bool {
        self.editor.borrow().is_some()
            || !self.current.get()
            || self.scope.strong_count() == 0
            || self.parts.signals.pending()
            || !self.values.borrow().is_empty()
            || self.parts.context.run_loop().is_waiting()
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        // Its editor first, on every way out (released, deleted, restarted, its session or the
        // host gone): removed and let go while its parts are whole, before they are deactivated
        // and terminated.
        drop(self.editor.get_mut().take());
        // Its parts only with its audio half gone: a processor still playing elsewhere (the host dropped
        // under a running player) keeps its parts and module for good rather than have them
        // terminated or unloaded under it.
        if Arc::strong_count(&self.alive) == 1 {
            // SAFETY: dropped once, here, and never used again.
            unsafe { ManuallyDrop::drop(&mut self.parts) };
        }
    }
}

impl std::fmt::Debug for Vst3Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vst3Host")
            .field("instances", &self.instances())
            .field("created", &self.created())
            .finish()
    }
}

impl Vst3Host {
    /// A host that finds plugins in `catalogue`. Its thread becomes the plugins' main thread.
    pub fn new(catalogue: Catalogue) -> Self {
        mark_main_thread();
        Self {
            inner: Rc::new(Inner {
                catalogue: RefCell::new(catalogue),
                modules: RefCell::new(BTreeMap::new()),
                instances: RefCell::new(Vec::new()),
                pending: RefCell::new(Vec::new()),
                created: Cell::new(0),
                doorbell: Arc::default(),
                display: RefCell::new(None),
                above: Cell::new(false),
            }),
            scope: Rc::new(()),
        }
    }

    /// Another handle to this host, for one session's compiler: the instances created through
    /// it (and its clones) go once all of them are gone and their audio halves have come back.
    pub fn session(&self) -> Self {
        Self {
            inner: Rc::clone(&self.inner),
            scope: Rc::new(()),
        }
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

    /// How many modules are loaded.
    pub fn modules(&self) -> usize {
        self.inner.modules.borrow().len()
    }

    /// Notes `owner`'s plugin was not sent because too many of its notes were sounding.
    pub fn dropped_notes(&self, owner: ProcessorOwner) -> u64 {
        self.inner
            .instances
            .borrow()
            .iter()
            .filter(|instance| instance.owner == owner)
            .map(|instance| instance.parts.signals.dropped_notes.load(Ordering::Relaxed))
            .sum()
    }

    /// `owner`'s open editor: its window and what it registered with its frame's run loop.
    pub fn editor(&self, owner: ProcessorOwner) -> Option<OpenEditor> {
        let instance = self.current(owner)?;
        let editor = instance.editor.borrow();
        let editor = editor.as_ref()?;
        let (timers, fds) = editor.frame().run_loop().registered();
        Some(OpenEditor {
            window: editor.window(),
            timers,
            fds,
        })
    }

    /// The window connection, made on first use.
    fn display(&self) -> Result<Ref<'_, Display>, HostError> {
        if self.inner.display.borrow().is_none() {
            let display = Display::connect().map_err(HostError::Failed)?;
            *self.inner.display.borrow_mut() = Some(display);
        }
        Ref::filter_map(self.inner.display.borrow(), Option::as_ref)
            .map_err(|_| HostError::Failed(UNAVAILABLE.into()))
    }

    /// `owner`'s current instance, handed out so no borrow of the table is held while the
    /// plugin is called.
    fn current(&self, owner: ProcessorOwner) -> Option<Rc<Instance>> {
        self.inner
            .instances
            .borrow()
            .iter()
            .rev()
            .find(|instance| instance.owner == owner && instance.current.get())
            .cloned()
    }

    /// The module `record` names, loaded once, and only as the scan saw it.
    fn module(&self, record: &FileRecord) -> Result<Arc<Module>, HostError> {
        if let Some(module) = self.inner.modules.borrow().get(&record.file) {
            return Ok(Arc::clone(module));
        }
        // Why compared first: a module replaced since its scan has not been loaded by a scan
        // child, and loading it here would let it crash the whole program.
        if !record.is_current() {
            return Err(HostError::Failed(
                "its file changed after the last plugin scan".into(),
            ));
        }
        // SAFETY: loading a module runs its code. The module is one a scan child loaded without
        // crashing and is unchanged since (its binary's size and modification time); that is
        // as far as a host can vouch for a plugin it runs in process.
        let module = Arc::new(unsafe { Module::load(&record.file) }.map_err(HostError::Failed)?);
        self.inner
            .modules
            .borrow_mut()
            .insert(record.file.clone(), Arc::clone(&module));
        Ok(module)
    }

    fn create_instance(
        &self,
        owner: ProcessorOwner,
        reference: &ProcessorRef,
        sample_rate: u32,
    ) -> Result<Box<dyn Processor>, HostError> {
        let class = reference
            .id
            .strip_prefix("vst3:")
            .ok_or(HostError::Missing)?;
        let cid = uid::parse(class).ok_or(HostError::Missing)?;
        let record = {
            let catalogue = self.inner.catalogue.borrow();
            catalogue
                .find(PluginFormat::Vst3, &uid::text(&cid))
                .map(|(file, _)| file.clone())
        }
        .ok_or(HostError::Missing)?;
        let module = self.module(&record)?;
        let saved = reference
            .state
            .as_deref()
            .map(|text| {
                BASE64
                    .decode(text)
                    .map_err(|_| HostError::Failed("its saved state is not base64".into()))
            })
            .transpose()?;
        let preset = saved.as_deref().map(preset_of(&cid)).transpose()?;
        let signals = Arc::new(Signals::ringing(Arc::clone(&self.inner.doorbell)));
        let (edits_in, edits_out) = rtrb::RingBuffer::new(EDITS);
        let parts = Parts::open(&module, &cid, Arc::clone(&signals), edits_in)
            .map_err(HostError::Failed)?;
        let initial = match &preset {
            Some(preset) => {
                parts.load(preset).map_err(HostError::Failed)?;
                None
            }
            None => {
                parts.share_component_state();
                parts.capture().ok()
            }
        };
        signals.forget_before_activation();
        let role = match owner {
            ProcessorOwner::Channel(_) => Role::Instrument,
            ProcessorOwner::Effect(_) => Role::Effect,
        };
        let layout = parts
            .activate(role, sample_rate)
            .map_err(HostError::Failed)?;
        let params = param_shapes(&parts);
        let mapping = mapping::read(&parts);
        let controls = Arc::new(Controls::new(&mapping));
        let mut steps: Vec<(u32, i32)> = params
            .iter()
            .filter(|param| param.steps > 0)
            .map(|param| (param.id, param.steps))
            .collect();
        steps.sort_unstable();
        let (values_in, values_out) = rtrb::RingBuffer::new(VALUES);
        let alive = Arc::new(());
        let latency = layout.latency;
        let audio = parts
            .audio
            .clone()
            .ok_or_else(|| HostError::Failed("its main output is not mono or stereo".into()))?;
        let processor = Vst3Processor::new(Making {
            audio,
            module,
            layout,
            signals,
            processing: Arc::clone(&parts.processing),
            steps,
            edits: edits_out,
            values: values_in,
            alive: Arc::clone(&alive),
            controls: Arc::clone(&controls),
        });
        let instance = Rc::new(Instance {
            owner,
            parts: ManuallyDrop::new(parts),
            alive,
            scope: Rc::downgrade(&self.scope),
            released: Cell::new(false),
            current: Cell::new(true),
            synced: RefCell::new(reference.state.clone()),
            initial: RefCell::new(initial),
            values: RefCell::new(values_out),
            params: RefCell::new(params),
            mapped: RefCell::new(mapping.mapped),
            controls,
            latency: Cell::new(latency),
            editor: RefCell::new(None),
        });
        let mut instances = self.inner.instances.borrow_mut();
        for older in instances.iter().filter(|other| other.owner == owner) {
            older.current.set(false);
        }
        instances.push(instance);
        self.inner.created.set(self.inner.created.get() + 1);
        Ok(Box::new(processor))
    }
}

/// The preset in `bytes`, which must be class `cid`'s.
fn preset_of(cid: &vst3::Steinberg::TUID) -> impl Fn(&[u8]) -> Result<Preset<'_>, HostError> + '_ {
    move |bytes| {
        let preset = preset::parse(bytes)
            .ok_or_else(|| HostError::Failed("its saved state is not a VST3 preset".into()))?;
        if uid::parse(preset.class).as_ref() != Some(cid) {
            return Err(HostError::Failed(
                "its saved state is for another plugin".into(),
            ));
        }
        Ok(preset)
    }
}

/// Every parameter's id, steps and read-only flag, as its controller describes it.
fn param_shapes(parts: &Parts) -> Vec<ParamShape> {
    param_infos(parts)
        .into_iter()
        .map(|info| ParamShape {
            id: info.id,
            steps: info.stepCount.max(0),
            read_only: info.flags & READ_ONLY != 0,
        })
        .collect()
}

fn param_infos(parts: &Parts) -> Vec<ParameterInfo> {
    let Some(controller) = &parts.controller else {
        return Vec::new();
    };
    // SAFETY: the controller is live while `parts` holds it; called on the main thread.
    let count = unsafe { controller.getParameterCount() };
    (0..count)
        .filter_map(|index| {
            // SAFETY: `ParameterInfo` is plain C data (numbers and UTF-16 arrays), for which all
            // zero bytes are a value.
            let mut info: ParameterInfo = unsafe { std::mem::zeroed() };
            // SAFETY: as for the count; `info` is ours to fill.
            let found = unsafe { controller.getParameterInfo(index, &mut info) };
            (found == kResultOk).then_some(info)
        })
        .collect()
}

impl ProcessorHost for Vst3Host {
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
            .borrow()
            .iter()
            .filter(|instance| instance.owner == owner && instance.scope.as_ptr() == scope)
        {
            instance.released.set(true);
            instance.current.set(false);
        }
    }

    fn sync_state(&self, owner: ProcessorOwner, reference: &ProcessorRef) {
        let Some(instance) = self.current(owner) else {
            return;
        };
        if *instance.synced.borrow() == reference.state {
            return;
        }
        let bytes = match &reference.state {
            Some(text) => match BASE64.decode(text) {
                Ok(bytes) => bytes,
                Err(_) => return,
            },
            None => match instance.initial.borrow().clone() {
                Some(bytes) => bytes,
                None => return,
            },
        };
        let cid = instance.parts.cid;
        let loaded = preset_of(&cid)(&bytes)
            .map_err(|_| ())
            .and_then(|preset| instance.parts.load(&preset).map_err(|_| ()));
        instance.parts.signals.forget_own_load();
        *instance.synced.borrow_mut() = reference.state.clone();
        if loaded.is_err() {
            self.inner.pending.borrow_mut().push(HostNotice {
                owner,
                kind: NoticeKind::Failed("it refused the state it was given".into()),
            });
        }
    }

    fn params(&self, owner: ProcessorOwner) -> Vec<PluginParam> {
        let Some(instance) = self.current(owner) else {
            return Vec::new();
        };
        param_infos(&instance.parts)
            .into_iter()
            .map(|info| {
                let steps = info.stepCount.max(0);
                let max = if steps > 0 { f64::from(steps) } else { 1.0 };
                PluginParam {
                    id: info.id,
                    name: read_utf16(&info.title),
                    group: String::new(),
                    min: 0.0,
                    max,
                    default: if steps > 0 {
                        (info.defaultNormalizedValue * max).round()
                    } else {
                        info.defaultNormalizedValue
                    },
                    stepped: steps > 0,
                    // A parameter the MIDI mapping names belongs to the MIDI input, not the sound.
                    hidden: info.flags & HIDDEN != 0 || instance.mapped.borrow().contains(&info.id),
                    read_only: info.flags & READ_ONLY != 0,
                }
            })
            .collect()
    }

    fn value(&self, owner: ProcessorOwner, id: u32) -> Option<f64> {
        let instance = self.current(owner)?;
        let controller = instance.parts.controller.as_ref()?;
        let steps = instance.shape(id)?.steps;
        // SAFETY: the controller is live; called on the main thread.
        let normalized = unsafe { controller.getParamNormalized(id) };
        Some(if steps > 0 {
            normalized * f64::from(steps)
        } else {
            normalized
        })
    }

    fn value_text(&self, owner: ProcessorOwner, id: u32, value: f64) -> Option<String> {
        let instance = self.current(owner)?;
        let controller = instance.parts.controller.as_ref()?;
        let steps = instance.shape(id)?.steps;
        // The step the plugin hears for `value`, as the audio half sends it.
        let normalized = if steps > 0 {
            normalized(value, steps)
        } else {
            value
        };
        let mut text: String128 = [0; 128];
        // SAFETY: the controller is live; `text` is our out parameter of 128 units.
        (unsafe { controller.getParamStringByValue(id, normalized, &mut text) } == kResultOk)
            .then(|| read_utf16(&text))
    }

    fn capture_state(&self, owner: ProcessorOwner) -> Result<Captured, HostError> {
        let instance = self.current(owner).ok_or(HostError::Missing)?;
        let bytes = instance.parts.capture().map_err(HostError::Failed)?;
        let text = BASE64.encode(&bytes);
        *instance.synced.borrow_mut() = Some(text.clone());
        Ok(Captured {
            state: text,
            bytes: bytes.len(),
        })
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
            if let Some(window) = instance.editor_window() {
                display.set_above(window, above);
            }
        }
    }

    // The plugin's own editor in a window of ours; one an instance has open already stays as it
    // is. No editor (or one that will not take our window): the app's parameter window.
    fn open_editor(&self, owner: ProcessorOwner, title: &str) -> Result<Editor, HostError> {
        let instance = self.current(owner).ok_or(HostError::Missing)?;
        if instance.editor.borrow().is_some() {
            return Ok(Editor::Embedded);
        }
        let Some(controller) = instance.parts.controller.clone() else {
            return Ok(Editor::Generic);
        };
        let Some(opened) = view::open(&controller, title, || self.display())? else {
            return Ok(Editor::Generic);
        };
        if self.inner.above.get() {
            if let Some(display) = self.inner.display.borrow().as_ref() {
                display.set_above(opened.window(), true);
            }
        }
        *instance.editor.borrow_mut() = Some(opened);
        Ok(Editor::Embedded)
    }

    fn latency(&self, owner: ProcessorOwner) -> Option<u32> {
        self.current(owner).map(|instance| instance.latency.get())
    }

    fn tick(&self) -> Vec<HostNotice> {
        let mut notices = std::mem::take(&mut *self.inner.pending.borrow_mut());
        let instances: Vec<Rc<Instance>> = self.inner.instances.borrow().clone();
        let events = self
            .inner
            .display
            .borrow()
            .as_ref()
            .map(Display::events)
            .unwrap_or_default();
        // A window's last size in this batch is the one it has: the earlier ones (a window
        // manager tells some twice) would each ask its editor again.
        let last_size = |index: usize, window: WindowId| {
            !events[index + 1..].iter().any(|later| {
                matches!(later, WindowEvent::Resized { window: other, .. } if *other == window)
            })
        };
        for (index, &event) in events.iter().enumerate() {
            if matches!(event, WindowEvent::Resized { .. }) && !last_size(index, event.window()) {
                continue;
            }
            let Some(instance) = instances
                .iter()
                .find(|instance| instance.editor_window() == Some(event.window()))
            else {
                continue;
            };
            match event {
                WindowEvent::Closed(_) => {
                    // Taken out of its place first: the plugin is called as it closes.
                    let closed = instance.editor.borrow_mut().take();
                    drop(closed);
                    notices.push(HostNotice {
                        owner: instance.owner,
                        kind: NoticeKind::Closed,
                    });
                }
                WindowEvent::Resized { width, height, .. } => {
                    let parts = instance.editor.borrow().as_ref().and_then(OpenView::parts);
                    if let Some((view, frame)) = parts {
                        frame::user_resized(&view, &frame, (width, height), Instant::now());
                    }
                }
            }
        }
        let modules: Vec<Arc<Module>> = self.inner.modules.borrow().values().cloned().collect();
        let now = Instant::now();
        for module in &modules {
            module.context().run_loop().serve(now);
        }
        for instance in &instances {
            instance.parts.context.run_loop().serve(now);
            let frame = instance
                .editor
                .borrow()
                .as_ref()
                .map(|editor| editor.frame().clone());
            if let Some(frame) = frame {
                // A size the window manager left our window at after a user's resize is asked
                // again once it is quiet (S7's settling).
                frame.settle(now);
                frame.serve(now);
            }
            instance.hand_values_on();
            let signals = &instance.parts.signals;
            let restart = signals.restart.swap(0, Ordering::AcqRel);
            if restart & LATENCY_CHANGED != 0 {
                instance.latency.set(instance.parts.latency());
            }
            if !instance.current.get() {
                continue;
            }
            let owner = instance.owner;
            if restart & (RELOAD | IO_CHANGED) != 0 {
                notices.push(HostNotice {
                    owner,
                    kind: NoticeKind::Restart,
                });
            }
            if restart & MAPPING_CHANGED != 0 {
                instance.read_mapping();
            }
            if restart & TITLES_CHANGED != 0 {
                let params = param_shapes(&instance.parts);
                *instance.params.borrow_mut() = params;
            }
            if restart & (TITLES_CHANGED | MAPPING_CHANGED) != 0 {
                notices.push(HostNotice {
                    owner,
                    kind: NoticeKind::Params,
                });
            }
            if Signals::take(&signals.dirty) || restart & VALUES_CHANGED != 0 {
                notices.push(HostNotice {
                    owner,
                    kind: NoticeKind::Dirty,
                });
            }
            if Signals::take(&signals.failed) {
                notices.push(HostNotice {
                    owner,
                    kind: NoticeKind::Failed("it failed while processing".into()),
                });
            }
        }
        drop(instances);
        drop(modules);
        // Instances released, or whose session is over, once their audio half has been
        // dropped: closed and destroyed here on the main thread, outside every borrow, in case
        // a plugin calls back while it goes.
        let mut gone = Vec::new();
        self.inner.instances.borrow_mut().retain(|instance| {
            let over = instance.released.get() || instance.scope.strong_count() == 0;
            if over && Arc::strong_count(&instance.alive) == 1 {
                gone.push(Rc::clone(instance));
                false
            } else {
                true
            }
        });
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
                .any(|instance| instance.needs_ticks())
            || self
                .inner
                .modules
                .borrow()
                .values()
                .any(|module| module.context().run_loop().is_waiting())
    }

    fn doorbell(&self) -> Option<Arc<AtomicBool>> {
        Some(Arc::clone(&self.inner.doorbell))
    }
}
