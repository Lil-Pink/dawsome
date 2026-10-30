//! A VST3 module whose entry aborts the process as it loads (SIGABRT), standing in for a plugin
//! that crashes its host. The scanner loads each module in a child process, so this kills the
//! child only and the catalogue records the module as failed.

use std::ffi::c_void;

#[cfg(target_os = "linux")]
#[no_mangle]
extern "system" fn ModuleEntry(_library_handle: *mut c_void) -> bool {
    std::process::abort()
}

#[cfg(target_os = "linux")]
#[no_mangle]
extern "system" fn ModuleExit() -> bool {
    true
}

#[cfg(windows)]
#[no_mangle]
extern "system" fn InitDll() -> bool {
    std::process::abort()
}

#[no_mangle]
extern "system" fn GetPluginFactory() -> *mut c_void {
    std::process::abort()
}
