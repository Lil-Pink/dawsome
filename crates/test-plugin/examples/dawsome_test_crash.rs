//! A plugin file whose entry aborts the process as it loads (SIGABRT), standing in for a plugin
//! that crashes its host. The scanner loads each file in a child process, so this kills the
//! child only and the catalogue records the file as failed.

use std::ffi::CStr;

use clack_plugin::entry::prelude::*;
use clack_plugin::prelude::*;

pub struct AbortingEntry;

impl Entry for AbortingEntry {
    fn new(_bundle_path: Option<&CStr>) -> Result<Self, EntryLoadError> {
        std::process::abort()
    }

    fn declare_factories<'a>(&'a self, _builder: &mut EntryFactories<'a>) {}
}

clack_export_entry!(AbortingEntry);
