//! The one-time copy of per-user files from the folders the app kept them in under its former
//! name. A current folder that does not exist yet is made from the former folder beside it,
//! with only the files worth keeping. The former folders are never changed or removed, so
//! whatever still names a file in them (a recording, a recent project) keeps working.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use dawsome_host_clap::catalogue;

use crate::{project_file, settings};

/// The name the per-user folders had under the former name, beside the current ones.
const FORMER_FOLDER: &str = "orchard";

/// The per-user files worth carrying over, at their current paths: the settings, the recent
/// files, the browser's folders, the plugin catalogue (so the first start does not scan again)
/// and the demo copy, which the user may have changed. Caches that rebuild themselves,
/// autosaves and recordings stay in the former folders.
pub fn kept() -> Vec<PathBuf> {
    [
        settings::settings_file(),
        project_file::recent_file(),
        project_file::browser_file(),
        catalogue::default_path(),
        project_file::demo_dir(),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// For each folder holding paths of `kept` that does not exist yet, copies those of the paths
/// the former folder beside it has. A folder is made whole or not at all; the folders that
/// failed come back with why, and the next start tries them again.
pub fn carry_over(kept: &[PathBuf]) -> Vec<(PathBuf, io::Error)> {
    let mut folders: BTreeMap<&Path, Vec<&OsStr>> = BTreeMap::new();
    for path in kept {
        if let (Some(folder), Some(name)) = (path.parent(), path.file_name()) {
            folders.entry(folder).or_default().push(name);
        }
    }
    folders
        .into_iter()
        .filter_map(|(folder, names)| {
            carry_folder(folder, &names)
                .err()
                .map(|error| (folder.to_path_buf(), error))
        })
        .collect()
}

/// Makes `current` from the former folder beside it, when `current` is missing and the former
/// folder holds any of `names`.
fn carry_folder(current: &Path, names: &[&OsStr]) -> io::Result<()> {
    let former = current.with_file_name(FORMER_FOLDER);
    if fs::symlink_metadata(current).is_ok() || !former.is_dir() {
        return Ok(());
    }
    let present: Vec<&OsStr> = names
        .iter()
        .copied()
        .filter(|name| fs::symlink_metadata(former.join(name)).is_ok())
        .collect();
    if present.is_empty() {
        return Ok(());
    }
    // Built under a hidden name and renamed into place: a copy that fails halfway leaves no
    // half-filled folder, which would stop the next start from trying again.
    let mut staging = OsString::from(".");
    staging.push(current.file_name().unwrap_or_default());
    staging.push(format!(".carrying-{}", std::process::id()));
    let staging = current.with_file_name(staging);
    let carried = fs::create_dir(&staging)
        .and_then(|()| {
            present
                .iter()
                .try_for_each(|name| copy_entry(&former.join(name), &staging.join(name)))
        })
        .and_then(|()| fs::rename(&staging, current));
    if carried.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    carried
}

/// Copies the file or folder at `from` to `to`, a folder with everything in it. A link is
/// copied as the file it points to; a link to a folder, and anything that is neither a file nor
/// a folder, is left out, so the copy always ends and never waits on a pipe.
fn copy_entry(from: &Path, to: &Path) -> io::Result<()> {
    let kind = fs::symlink_metadata(from)?.file_type();
    if kind.is_dir() {
        fs::create_dir(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_entry(&entry.path(), &to.join(entry.file_name()))?;
        }
    } else if kind.is_file() || (kind.is_symlink() && from.is_file()) {
        fs::copy(from, to)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh base directory standing in for the XDG base directories.
    fn temp_base(test: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("dawsome-former-folders-{}", std::process::id()))
            .join(test);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().expect("parent")).expect("folders");
        fs::write(path, text).expect("written");
    }

    /// Every file under `dir` with its text, by path relative to `dir`.
    fn files(dir: &Path) -> BTreeMap<PathBuf, String> {
        fn walk(root: &Path, dir: &Path, found: &mut BTreeMap<PathBuf, String>) {
            for entry in fs::read_dir(dir).expect("readable") {
                let path = entry.expect("entry").path();
                if path.is_dir() {
                    walk(root, &path, found);
                } else {
                    let text = fs::read_to_string(&path).expect("text");
                    let relative = path.strip_prefix(root).expect("inside").to_path_buf();
                    found.insert(relative, text);
                }
            }
        }
        let mut found = BTreeMap::new();
        walk(dir, dir, &mut found);
        found
    }

    fn listing(entries: &[(&str, &str)]) -> BTreeMap<PathBuf, String> {
        entries
            .iter()
            .map(|(path, text)| (PathBuf::from(path), (*text).to_owned()))
            .collect()
    }

    /// The paths `kept` gives with the three bases under `base`.
    fn kept_under(base: &Path) -> Vec<PathBuf> {
        let config = base.join("config/dawsome");
        vec![
            config.join("settings.json"),
            config.join("recent.json"),
            config.join("browser.json"),
            base.join("cache/dawsome/clap-catalogue.json"),
            base.join("data/dawsome/demo"),
        ]
    }

    /// The former folder of base directory `kind` (config, cache, data, state) under `base`.
    fn former(base: &Path, kind: &str) -> PathBuf {
        base.join(kind).join(FORMER_FOLDER)
    }

    /// Former folders holding every kept file plus files that stay behind.
    fn former_folders(base: &Path) {
        for name in ["settings", "recent", "browser", "window"] {
            write(
                &former(base, "config").join(format!("{name}.json")),
                &format!("{{\"{name}\":1}}"),
            );
        }
        let (cache, data) = (former(base, "cache"), former(base, "data"));
        write(&cache.join("clap-catalogue.json"), "plugins");
        write(&cache.join("stretch/a.wav"), "stretched");
        write(&data.join("demo/demo.json"), "demo");
        write(&data.join("demo/sounds/kick.wav"), "kick");
        write(&data.join("recordings/take.wav"), "take");
        let autosave = former(base, "state").join("autosave/untitled.autosave.json");
        write(&autosave, "draft");
    }

    #[test]
    fn the_kept_files_and_the_demo_are_copied_and_the_former_folders_stay() {
        let base = temp_base("carried");
        former_folders(&base);
        let before = files(&base);
        assert!(carry_over(&kept_under(&base)).is_empty());
        assert_eq!(
            files(&base.join("config/dawsome")),
            listing(&[
                ("browser.json", r#"{"browser":1}"#),
                ("recent.json", r#"{"recent":1}"#),
                ("settings.json", r#"{"settings":1}"#),
            ])
        );
        assert_eq!(
            files(&base.join("cache/dawsome")),
            listing(&[("clap-catalogue.json", "plugins")])
        );
        assert_eq!(
            files(&base.join("data/dawsome")),
            listing(&[("demo/demo.json", "demo"), ("demo/sounds/kick.wav", "kick")])
        );
        let mut after = files(&base);
        after.retain(|path, _| !path.iter().any(|part| part == "dawsome"));
        assert_eq!(after, before, "the former folders are as they were");
        assert!(!base.join("state/dawsome").exists(), "autosaves stay");
    }

    #[test]
    fn a_folder_that_exists_is_left_as_it_is() {
        let base = temp_base("existing");
        former_folders(&base);
        write(&base.join("config/dawsome/settings.json"), "mine");
        assert!(carry_over(&kept_under(&base)).is_empty());
        assert_eq!(
            files(&base.join("config/dawsome")),
            listing(&[("settings.json", "mine")])
        );
        assert!(base.join("cache/dawsome/clap-catalogue.json").exists());
        assert!(base.join("data/dawsome/demo/demo.json").exists());
    }

    #[test]
    fn without_a_former_folder_nothing_is_made() {
        let base = temp_base("fresh");
        fs::create_dir_all(base.join("config")).expect("folders");
        write(&former(&base, "cache").join("stretch/a.wav"), "stretched");
        assert!(carry_over(&kept_under(&base)).is_empty());
        assert!(!base.join("config/dawsome").exists());
        assert!(
            !base.join("cache/dawsome").exists(),
            "a former folder holding nothing kept carries nothing"
        );
    }

    // Linux only: the copy is made to fail by a file without read permission, a mode bit
    // Windows files do not have.
    #[cfg(unix)]
    #[test]
    fn a_failed_copy_leaves_no_folder_behind() {
        use std::os::unix::fs::PermissionsExt;
        let base = temp_base("failed");
        former_folders(&base);
        let locked = former(&base, "config").join("recent.json");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("locked");
        if fs::File::open(&locked).is_ok() {
            // A privileged user reads it anyway; there is no failure to observe.
            return;
        }
        let failed = carry_over(&kept_under(&base));
        assert_eq!(failed.len(), 1, "{failed:?}");
        assert_eq!(failed[0].0, base.join("config/dawsome"));
        let left: Vec<_> = fs::read_dir(base.join("config"))
            .expect("readable")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        assert_eq!(
            left,
            [FORMER_FOLDER],
            "no current folder and no staging folder"
        );
        assert!(base.join("data/dawsome/demo/demo.json").exists());
    }

    // Linux only: making links takes a privilege on Windows that tests do not have.
    #[cfg(unix)]
    #[test]
    fn a_link_is_copied_as_its_file_and_a_link_to_a_folder_is_left_out() {
        let base = temp_base("links");
        write(&base.join("elsewhere/kit/kick.wav"), "kick");
        let demo = former(&base, "data").join("demo");
        write(&demo.join("demo.json"), "demo");
        std::os::unix::fs::symlink(base.join("elsewhere/kit/kick.wav"), demo.join("kick.wav"))
            .expect("link to a file");
        std::os::unix::fs::symlink(base.join("elsewhere/kit"), demo.join("kit"))
            .expect("link to a folder");
        assert!(carry_over(&kept_under(&base)).is_empty());
        let copied = base.join("data/dawsome/demo");
        assert_eq!(
            files(&copied),
            listing(&[("demo.json", "demo"), ("kick.wav", "kick")])
        );
        let kick = fs::symlink_metadata(copied.join("kick.wav")).expect("copied");
        assert!(!kick.file_type().is_symlink(), "a copy, not a link");
    }

    #[test]
    fn every_kept_path_is_in_a_folder_named_after_the_app() {
        for path in kept() {
            let folder = path.parent().and_then(Path::file_name);
            assert_eq!(folder, Some(OsStr::new("dawsome")), "{}", path.display());
        }
    }
}
