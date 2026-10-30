//! Where plugins are looked for, where the catalogue is kept, and how a scan child starts: what
//! a program that hosts plugins sets up once.

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::catalogue::{self, Catalogue, PluginFormat};
use crate::scan::{self, ChildCommand, ScanRequest, Scanned};

/// Where plugins are looked for, which formats, where the catalogue is kept, and how a scan
/// child starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginSetup {
    pub dirs: Vec<PathBuf>,
    pub cache: PathBuf,
    pub child: ChildCommand,
    /// The formats its scans look for; the catalogue's records of others are kept.
    pub formats: &'static [PluginFormat],
}

impl PluginSetup {
    /// [`for_formats`](Self::for_formats) CLAP alone: the app's setup while it hosts CLAP alone.
    pub fn new(dirs: Vec<PathBuf>, cache: Option<PathBuf>, child: ChildCommand) -> Option<Self> {
        Self::for_formats(PluginFormat::CLAP, dirs, cache, child)
    }

    /// Plugins of `formats`, looked for in `dirs`, or in those formats' standard search paths
    /// when none are given ([`catalogue::search_paths_for`]), and the catalogue at `cache`, or
    /// in the user's cache dir. `None` when there is no cache dir to use.
    pub fn for_formats(
        formats: &'static [PluginFormat],
        dirs: Vec<PathBuf>,
        cache: Option<PathBuf>,
        child: ChildCommand,
    ) -> Option<Self> {
        let dirs = if dirs.is_empty() {
            catalogue::search_paths_for(formats)
        } else {
            dirs
        };
        Some(Self {
            dirs,
            cache: cache.or_else(catalogue::default_path)?,
            child,
            formats,
        })
    }

    /// A scan of the plugin directories into the catalogue; `force` loads unchanged files
    /// again too.
    pub fn scan(&self, force: bool) -> Scanned {
        scan::scan(&ScanRequest {
            dirs: self.dirs.clone(),
            formats: self.formats.to_vec(),
            cache: self.cache.clone(),
            force,
            timeout: scan::CHILD_TIMEOUT,
            cap: scan::CHILD_CAP,
            child: self.child.clone(),
        })
    }

    /// The catalogue to find `plugins` (format, id) in: the cached one, or, when it lacks one of
    /// them or one's file changed after it was scanned, one a scan makes (returned too, for
    /// what it has to report).
    pub fn catalogue_for_plugins(
        &self,
        plugins: &BTreeSet<(PluginFormat, String)>,
    ) -> (Catalogue, Option<Scanned>) {
        let cached = Catalogue::load(&self.cache).unwrap_or_default();
        let found_as_scanned = |(format, id): &(PluginFormat, String)| {
            cached
                .find(*format, id)
                .is_some_and(|(file, _)| file.is_current())
        };
        if plugins.iter().all(found_as_scanned) {
            return (cached, None);
        }
        let scanned = self.scan(false);
        (scanned.catalogue.clone(), Some(scanned))
    }
}
