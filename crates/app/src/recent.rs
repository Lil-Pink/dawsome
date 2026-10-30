//! The recently opened and saved documents: absolute paths, newest first, each once, at most
//! [`LIMIT`], kept in a JSON array that is rewritten atomically on every change.

use std::fs;
use std::path::{Path, PathBuf};

use crate::project_file::{self, SaveError};

/// How many documents the list keeps.
pub const LIMIT: usize = 10;

/// The list and the file it lives in; with no file (a scripted run) it is never saved.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Recent {
    file: Option<PathBuf>,
    entries: Vec<PathBuf>,
}

impl Recent {
    /// The list in `file`; empty when the file is missing or not a list of paths.
    pub fn load(file: Option<PathBuf>) -> Self {
        let entries = file
            .as_deref()
            .and_then(|file| fs::read_to_string(file).ok())
            .and_then(|text| serde_json::from_str::<Vec<PathBuf>>(&text).ok())
            .unwrap_or_default();
        let mut recent = Self {
            file,
            entries: Vec::new(),
        };
        for entry in entries.into_iter().rev() {
            recent.push_front(entry);
        }
        recent
    }

    pub fn entries(&self) -> &[PathBuf] {
        &self.entries
    }

    /// Puts `path` first, absolute, dropping an older entry for it and the oldest past the
    /// limit, and saves the list.
    pub fn add(&mut self, path: &Path) -> Result<(), SaveError> {
        self.push_front(std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()));
        self.save()
    }

    /// Takes `path` off the list and saves it.
    pub fn remove(&mut self, path: &Path) -> Result<(), SaveError> {
        self.entries.retain(|entry| entry != path);
        self.save()
    }

    fn push_front(&mut self, path: PathBuf) {
        self.entries.retain(|entry| *entry != path);
        self.entries.insert(0, path);
        self.entries.truncate(LIMIT);
    }

    fn save(&self) -> Result<(), SaveError> {
        let Some(file) = &self.file else {
            return Ok(());
        };
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir).map_err(|source| SaveError::Write {
                path: file.clone(),
                source,
            })?;
        }
        let text =
            serde_json::to_string_pretty(&self.entries).map_err(|source| SaveError::Serialize {
                path: file.clone(),
                source,
            })?;
        project_file::write_atomically(file, text.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An absolute folder on this platform, which no test touches.
    fn music() -> &'static Path {
        Path::new(if cfg!(windows) { r"C:\music" } else { "/music" })
    }

    fn file(test: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("dawsome-recent-{}", std::process::id()))
            .join(test);
        let _ = fs::remove_dir_all(&dir);
        dir.join("dawsome/recent.json")
    }

    #[test]
    fn entries_are_newest_first_once_each_and_at_most_ten() {
        let file = file("cap");
        let mut recent = Recent::load(Some(file.clone()));
        let song = |n: u32| music().join(format!("{n}.json"));
        for n in 0..12 {
            recent.add(&song(n)).expect("saves");
        }
        recent.add(&song(5)).expect("saves");
        assert_eq!(recent.entries(), [5, 11, 10, 9, 8, 7, 6, 4, 3, 2].map(song));
        // The file holds the same list, and loads back as it is.
        assert_eq!(Recent::load(Some(file.clone())), recent);
        recent.remove(&song(11)).expect("saves");
        assert_eq!(Recent::load(Some(file)).entries().len(), 9);
    }

    #[test]
    fn a_missing_or_broken_file_is_an_empty_list() {
        let missing = file("missing");
        assert!(Recent::load(Some(missing.clone())).entries().is_empty());
        fs::create_dir_all(missing.parent().expect("dir")).expect("dir");
        fs::write(&missing, b"{not a list").expect("writes");
        assert!(Recent::load(Some(missing)).entries().is_empty());
    }

    #[test]
    fn a_list_without_a_file_is_kept_in_memory_only() {
        let mut recent = Recent::load(None);
        let song = music().join("a.json");
        recent.add(&song).expect("nothing to write");
        assert_eq!(recent.entries(), [song]);
    }
}
