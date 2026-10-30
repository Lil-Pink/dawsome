//! A VST3 module loaded into this process: its binary (a bundle's for this platform, or a
//! single-file module), its entry called, its factory and the classes it offers. A module stays
//! loaded while anything made from it lives (the host's map, each instance, each audio half hold
//! it); it goes only after the last: the factory released, `ModuleExit` (`ExitDll` on Windows)
//! called, then the library closed. Why so late: unloading a module whose objects or threads
//! still run crashes (the SDK's `module_linux.cpp` and `module_win32.cpp` unload in that order,
//! and only on the host's own say).

use std::ffi::c_void;
use std::path::{Path, PathBuf};

use dawsome_host_common::catalogue::PluginFormat;
use vst3::Steinberg::Vst::IComponent;
use vst3::Steinberg::{
    kResultOk, IPluginFactory, IPluginFactory2, IPluginFactory2Trait, IPluginFactory3,
    IPluginFactory3Trait, IPluginFactoryTrait, PClassInfo, PClassInfo2, PClassInfoW, PFactoryInfo,
    TUID,
};
use vst3::{ComPtr, ComWrapper, Interface};

use crate::com::context::{self, HostContext};
use crate::com::{read_utf16, read_utf8};
use crate::constants::UNICODE;

/// The category of the classes a host plays (`kVstAudioEffectClass`, a C macro in the SDK).
const AUDIO_MODULE_CLASS: &str = "Audio Module Class";

/// One class a module offers, as its factory describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClassInfo {
    pub cid: TUID,
    pub name: String,
    pub vendor: String,
    pub version: String,
    /// "Instrument|Synth", "Fx|Delay": what kind of plugin it is.
    pub sub_categories: String,
}

type Exit = unsafe extern "system" fn() -> bool;

pub(crate) struct Module {
    /// Released before the exit is called. While it is `Some` the factory is live: this holds
    /// its reference, and `Drop` lets it go before the exit and the library close.
    factory: Option<ComPtr<IPluginFactory>>,
    /// The context the factory was given (`setHostContext`): its run loop is the module's.
    context: ComWrapper<HostContext>,
    exit: Option<Exit>,
    /// Closed last.
    library: Option<libloading::Library>,
    vendor: String,
    unicode: bool,
    path: PathBuf,
}

impl std::fmt::Debug for Module {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Module").field("path", &self.path).finish()
    }
}

impl Module {
    /// Loads the module at `path` (a bundle or a single-file module) and calls its entry.
    ///
    /// # Safety
    /// Loading a module runs its code: the caller must have reason to trust it, as a scan child
    /// does (that is what it is for) or the host does for a module a scan child loaded
    /// unchanged.
    pub(crate) unsafe fn load(path: &Path) -> Result<Self, String> {
        let binary = PluginFormat::Vst3.binary(path);
        // SAFETY: the caller's.
        let (library, exit) = unsafe { open(&binary)? };
        let raw = {
            // SAFETY: `GetPluginFactory` has this signature (the SDK's `ipluginbase.h`: no
            // arguments, an `IPluginFactory*` back); `get` is used only inside this scope, while
            // `library` is open.
            let get: libloading::Symbol<'_, unsafe extern "system" fn() -> *mut IPluginFactory> =
                unsafe { library.get(b"GetPluginFactory\0") }
                    .map_err(|_| "it has no GetPluginFactory".to_owned())?;
            // SAFETY: the module's entry has run (`open`), as the format requires first.
            unsafe { get() }
        };
        // SAFETY: the factory comes with one reference for its caller (`ipluginbase.h`), which
        // the `ComPtr` takes over; null gives `None`.
        let factory = unsafe { ComPtr::from_raw(raw) };
        let mut module = Self {
            factory: None,
            context: HostContext::new(),
            exit,
            library: Some(library),
            vendor: String::new(),
            unicode: false,
            path: path.to_owned(),
        };
        let factory = factory.ok_or_else(|| "it offers no plugins".to_owned())?;
        // SAFETY: `PFactoryInfo` is plain C data (integers and char arrays), for which all zero
        // bytes are a value.
        let mut info: PFactoryInfo = unsafe { std::mem::zeroed() };
        // SAFETY: `factory` holds the reference `GetPluginFactory` gave; `info` is ours to fill.
        if unsafe { factory.getFactoryInfo(&mut info) } == kResultOk {
            module.vendor = read_utf8(&info.vendor);
            module.unicode = info.flags & UNICODE != 0;
        }
        if let Some(factory) = factory.cast::<IPluginFactory3>() {
            // SAFETY: `factory` is a second reference to the live factory (`cast`); the context
            // is our object, which `module` keeps longer than the factory and which the
            // factory may keep a reference to.
            unsafe { factory.setHostContext(context::as_unknown(&module.context)) };
        }
        module.factory = Some(factory);
        Ok(module)
    }

    pub(crate) fn context(&self) -> &HostContext {
        &self.context
    }

    /// The audio classes the module offers, in its factory's order: the "Audio Module Class"
    /// ones alone (a controller class or JUCE's "Plugin Compatibility Class" is none).
    pub(crate) fn classes(&self) -> Vec<ClassInfo> {
        let Some(factory) = &self.factory else {
            return Vec::new();
        };
        let factory2 = factory.cast::<IPluginFactory2>();
        let factory3 = factory.cast::<IPluginFactory3>().filter(|_| self.unicode);
        // SAFETY: the factory is live while `self.factory` holds it (see the field).
        let count = unsafe { factory.countClasses() };
        let mut classes = Vec::new();
        for index in 0..count {
            // SAFETY: `PClassInfo` is plain C data, for which all zero bytes are a value.
            let mut basic: PClassInfo = unsafe { std::mem::zeroed() };
            // SAFETY: the factory is live (as for `countClasses`); `basic` is ours to fill.
            if unsafe { factory.getClassInfo(index, &mut basic) } != kResultOk
                || read_utf8(&basic.category) != AUDIO_MODULE_CLASS
            {
                continue;
            }
            let mut class = ClassInfo {
                cid: basic.cid,
                name: read_utf8(&basic.name),
                vendor: self.vendor.clone(),
                version: String::new(),
                sub_categories: String::new(),
            };
            if let Some(factory2) = &factory2 {
                // SAFETY: `PClassInfo2` is plain C data, for which all zero bytes are a value.
                let mut info: PClassInfo2 = unsafe { std::mem::zeroed() };
                // SAFETY: `factory2` is a second reference to the live factory (`cast`); `info`
                // is ours to fill.
                if unsafe { factory2.getClassInfo2(index, &mut info) } == kResultOk {
                    class.name = read_utf8(&info.name);
                    class.sub_categories = read_utf8(&info.subCategories);
                    class.version = read_utf8(&info.version);
                    let vendor = read_utf8(&info.vendor);
                    if !vendor.is_empty() {
                        class.vendor = vendor;
                    }
                }
            }
            if let Some(factory3) = &factory3 {
                // SAFETY: `PClassInfoW` is plain C data, for which all zero bytes are a value.
                let mut info: PClassInfoW = unsafe { std::mem::zeroed() };
                // SAFETY: as for `factory2`.
                if unsafe { factory3.getClassInfoUnicode(index, &mut info) } == kResultOk {
                    class.name = read_utf16(&info.name);
                    class.sub_categories = read_utf8(&info.subCategories);
                    class.version = read_utf16(&info.version);
                    let vendor = read_utf16(&info.vendor);
                    if !vendor.is_empty() {
                        class.vendor = vendor;
                    }
                }
            }
            classes.push(class);
        }
        classes
    }

    /// A new object of class `cid` answering interface `I`; `None` when the factory makes none.
    pub(crate) fn create<I: Interface>(&self, cid: &TUID) -> Option<ComPtr<I>> {
        let factory = self.factory.as_ref()?;
        let mut made: *mut c_void = std::ptr::null_mut();
        // SAFETY: the factory is live while `self.factory` holds it; the ids are 16 bytes as the
        // interface reads them, and `made` gets a pointer with a reference for us, or stays null.
        let result =
            unsafe { factory.createInstance(cid.as_ptr(), I::IID.as_ptr().cast(), &mut made) };
        if result != kResultOk {
            return None;
        }
        // SAFETY: what the factory made answers `I`, with our reference.
        unsafe { ComPtr::from_raw(made.cast()) }
    }

    /// A new component of class `cid`.
    pub(crate) fn component(&self, cid: &TUID) -> Option<ComPtr<IComponent>> {
        self.create(cid)
    }
}

impl Drop for Module {
    fn drop(&mut self) {
        // The module's own registrations go first, then the factory, then its exit, then the
        // library, the order the SDK's hosting code keeps.
        self.context.run_loop().close();
        self.factory.take();
        if let Some(exit) = self.exit {
            // SAFETY: the module's exit, called once, after every object of ours from it is gone
            // (each instance and audio half holds the module).
            unsafe { exit() };
        }
        self.library.take();
    }
}

/// Opens `binary` and calls its entry: on Linux `ModuleEntry(handle)`, which a module must
/// have, with `ModuleExit`; on Windows `InitDll`, which it may have, with `ExitDll`.
///
/// # Safety
/// As [`Module::load`]: opening `binary` runs its code. The exit handed back may be called
/// only while the library handed back is open.
#[cfg(unix)]
unsafe fn open(binary: &Path) -> Result<(libloading::Library, Option<Exit>), String> {
    use libloading::os::unix::{Library, RTLD_LAZY, RTLD_LOCAL};

    // SAFETY: the caller's (`Module::load`); RTLD_LOCAL keeps its symbols to itself, as the
    // SDK's module_linux.cpp loads one.
    let library = unsafe { Library::open(Some(binary), RTLD_LAZY | RTLD_LOCAL) }
        .map_err(|error| format!("cannot load {}: {error}", binary.display()))?;
    let handle = library.into_raw();
    // SAFETY: `handle` is the one just opened; the library takes it back.
    let library = unsafe { Library::from_raw(handle) };
    // SAFETY: the format's documented entry, `bool ModuleEntry(void*)` (the SDK's
    // module_linux.cpp); the copied pointer is called below, while `library` is open.
    let entry =
        unsafe { library.get::<unsafe extern "system" fn(*mut c_void) -> bool>(b"ModuleEntry\0") }
            .map(|symbol| *symbol)
            .map_err(|_| "it has no ModuleEntry".to_owned())?;
    // SAFETY: the format's documented exit, `bool ModuleExit()`; the copied pointer goes back
    // with the library, and `Module` calls it before closing that.
    let exit = unsafe { library.get::<Exit>(b"ModuleExit\0") }
        .map(|symbol| *symbol)
        .map_err(|_| "it has no ModuleExit".to_owned())?;
    // SAFETY: called once, with the module's own handle, before anything else of it.
    if !unsafe { entry(handle) } {
        return Err("its ModuleEntry failed".into());
    }
    Ok((library.into(), Some(exit)))
}

/// # Safety
/// As the Linux `open`.
#[cfg(windows)]
unsafe fn open(binary: &Path) -> Result<(libloading::Library, Option<Exit>), String> {
    // SAFETY: the caller's (`Module::load`). `LoadLibraryExW` with no flags, as `LoadLibraryW`
    // (the SDK's module_win32.cpp).
    let library = unsafe { libloading::Library::new(binary) }
        .map_err(|error| format!("cannot load {}: {error}", binary.display()))?;
    // SAFETY: the format's optional entry, `bool InitDll()` (the SDK's module_win32.cpp); the
    // copied pointer is called below, while `library` is open.
    let entry = unsafe { library.get::<unsafe extern "system" fn() -> bool>(b"InitDll\0") }
        .map(|symbol| *symbol)
        .ok();
    // SAFETY: the format's optional exit, `bool ExitDll()`; the copied pointer goes back with
    // the library, and `Module` calls it before closing that.
    let exit = unsafe { library.get::<Exit>(b"ExitDll\0") }
        .map(|symbol| *symbol)
        .ok();
    if let Some(entry) = entry {
        // SAFETY: called once, before anything else of it.
        if !unsafe { entry() } {
            return Err("its InitDll failed".into());
        }
    }
    Ok((library, exit))
}
