//! Where a project's sound files are: relative to the project file, absolute, or shipped (in
//! the per-user data folder or among the instrument sets installed with the program, named so
//! the project opens on any machine that has them), and keeping those paths pointing at the
//! same files when the project file moves to another directory.

use std::path::{Component, Path, PathBuf};

use dawsome_engine::project::Project;

/// What a project writes before the path of a file inside [`shipped_root`]:
/// `dawsome:instruments/piano/piano.sfz`, `dawsome:starter-kit/Kick.wav`.
///
/// Why a reference rather than the file's path: the data folder is a different absolute path
/// for every user and system, so a project naming a shipped sound by its path would open
/// silent anywhere else. Why this form: it is a plain relative path to the project file
/// format (stored and rebased as any other), a colon cannot start a real relative path on
/// Windows, and it keeps its bytes on every platform when written with `/`.
pub const SHIPPED_PREFIX: &str = "dawsome:";

/// The per-user data folder: `$DAWSOME_DATA_HOME` when that is an absolute path, else the
/// platform's (`$XDG_DATA_HOME` or `~/.local/share` on Linux, `%LOCALAPPDATA%` on Windows). The
/// app keeps its per-user data there too.
pub fn data_dir() -> Option<PathBuf> {
    std::env::var_os("DAWSOME_DATA_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(dirs::data_local_dir)
}

/// `dawsome/` in the per-user data folder: where the starter kit and the instrument sets the
/// user installs live, and the first place a [`SHIPPED_PREFIX`] path is read from.
pub fn shipped_root() -> Option<PathBuf> {
    data_dir().map(|dir| dir.join("dawsome"))
}

/// The folder of instrument sets, in [`shipped_root`] and in a reference: a reference
/// `dawsome:instruments/<set>/...` names a set the user's folder or the install holds.
pub const INSTRUMENTS: &str = "instruments";

/// The variable naming the installed instrument sets' folder in place of the one found beside
/// the program: for a run from the build folder, which has none beside it.
pub const SHIPPED_INSTRUMENTS_VAR: &str = "DAWSOME_SHIPPED_INSTRUMENTS";

/// The instrument sets installed with the program: `$DAWSOME_SHIPPED_INSTRUMENTS` when that is
/// an absolute path, else the folder [`installed_beside`] finds for this program.
pub fn installed_instruments() -> Option<PathBuf> {
    std::env::var_os(SHIPPED_INSTRUMENTS_VAR)
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| installed_beside(&std::env::current_exe().ok()?))
}

/// The installed instrument sets of the program at `program`: `instruments/` beside it (a
/// Windows install and the release zips), else, on other systems, `share/dawsome/instruments`
/// under the prefix it is installed in (`<prefix>/bin/dawsome`, as scripts/install.sh puts
/// it); `None` when neither is a folder.
pub fn installed_beside(program: &Path) -> Option<PathBuf> {
    let dir = program.parent()?;
    let beside = dir.join(INSTRUMENTS);
    if beside.is_dir() {
        return Some(beside);
    }
    if cfg!(windows) {
        return None;
    }
    let shared = dir.parent()?.join("share/dawsome").join(INSTRUMENTS);
    shared.is_dir().then_some(shared)
}

/// Where shipped references are read from: the user's folder, then the installed sets.
///
/// Why two: the sets an install ships stay in the install (copying 70 MB of them into every
/// user's data folder would gain nothing), while the user's own downloaded or edited sets live
/// in their data folder; a project names either the same way, so it opens on any install.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShippedRoots {
    /// `dawsome/` in the per-user data folder ([`shipped_root`]).
    pub user: Option<PathBuf>,
    /// The instrument sets installed with the program ([`installed_instruments`]).
    pub installed: Option<PathBuf>,
}

impl ShippedRoots {
    /// This user's folder and this program's installed sets.
    pub fn find() -> Self {
        Self {
            user: shipped_root(),
            installed: installed_instruments(),
        }
    }

    /// The file a reference names by `part` (what follows [`SHIPPED_PREFIX`]): one under
    /// `instruments/<set>/` in the first root holding a folder `<set>`, the user's
    /// `instruments/` before the installed sets, so a set of the user's shadows an installed
    /// one of the same folder name whole; anything else in the user's folder. `None` when
    /// `part` is not [`plain`] names, or no root could hold it.
    pub fn file(&self, part: &str) -> Option<PathBuf> {
        if !plain(part) {
            return None;
        }
        let user = self.user.as_ref().map(|root| root.join(part));
        let Some(rest) = part.strip_prefix("instruments/") else {
            return user;
        };
        let set = rest.split('/').next().unwrap_or(rest);
        let users = self
            .user
            .as_ref()
            .is_some_and(|root| root.join(INSTRUMENTS).join(set).is_dir());
        match &self.installed {
            Some(installed) if !users && installed.join(set).is_dir() => Some(installed.join(rest)),
            _ => user,
        }
    }

    /// The reference naming `file` when it lies in a root: its path from the user's folder, or
    /// `instruments/` and its path from the installed sets.
    pub fn reference(&self, file: &Path) -> Option<PathBuf> {
        let user = self
            .user
            .as_deref()
            .and_then(|root| shipped_reference(file, root));
        user.or_else(|| {
            let root = self.installed.as_deref()?;
            let part = shipped_reference(file, root)?;
            let part = shipped_part(&part)?;
            Some(PathBuf::from(format!(
                "{SHIPPED_PREFIX}{INSTRUMENTS}/{part}"
            )))
        })
    }
}

/// The path after [`SHIPPED_PREFIX`] when `stored` is a shipped reference.
pub fn shipped_part(stored: &Path) -> Option<&str> {
    stored.to_str()?.strip_prefix(SHIPPED_PREFIX)
}

/// The file `stored` names in a project whose relative paths start at `dir`: a shipped
/// reference as `roots` find it ([`ShippedRoots::file`]; unreadable when no root holds it or it
/// is not written as [`plain`] names), any other path joined to `dir` (an absolute one as it
/// is).
pub fn sound_file_from(dir: &Path, stored: &Path, roots: &ShippedRoots) -> PathBuf {
    shipped_part(stored)
        .and_then(|part| roots.file(part))
        .unwrap_or_else(|| dir.join(stored))
}

/// [`sound_file_from`] with `shipped` as the user's folder and no installed sets.
pub fn sound_file_in(dir: &Path, stored: &Path, shipped: Option<&Path>) -> PathBuf {
    let roots = ShippedRoots {
        user: shipped.map(Path::to_path_buf),
        installed: None,
    };
    sound_file_from(dir, stored, &roots)
}

/// Whether `part` is names alone: no `..`, `.`, root or drive. A shipped reference names a file
/// inside the shipped folder, so one written otherwise names nothing that can be read (before,
/// `dawsome:../x` and `dawsome:/abs/x` read files outside it: found
/// 2026-10-22).
fn plain(part: &str) -> bool {
    !part.is_empty()
        && Path::new(part)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

/// [`sound_file_from`] with this user's folder and this program's installed sets.
pub fn sound_file(dir: &Path, stored: &Path) -> PathBuf {
    sound_file_from(dir, stored, &ShippedRoots::find())
}

/// The shipped reference naming `file` when it lies inside `shipped`: [`SHIPPED_PREFIX`] and
/// its path from there, written with `/`.
pub fn shipped_reference(file: &Path, shipped: &Path) -> Option<PathBuf> {
    let file = resolved(file);
    let inside = file.strip_prefix(resolved(shipped)).ok()?;
    let parts = inside
        .components()
        .map(|part| match part {
            Component::Normal(part) => part.to_str(),
            _ => None,
        })
        .collect::<Option<Vec<&str>>>()?;
    (!parts.is_empty()).then(|| PathBuf::from(format!("{SHIPPED_PREFIX}{}", parts.join("/"))))
}

/// Every file `project`, its relative paths starting at `dir`, reads its sounds from: each
/// channel's sound file and instrument set. A render must never write over one of them.
pub fn input_files(project: &Project, dir: &Path) -> Vec<PathBuf> {
    project
        .channels
        .values()
        .flat_map(|channel| [&channel.sample, &channel.instrument])
        .filter(|path| !path.as_os_str().is_empty())
        .map(|path| sound_file(dir, path))
        .collect()
}

/// Rewrites each sound path of `project` (sound files and instrument sets), read relative to
/// `from`, so that it names the same file from `to`: relative when the file lies under `to`,
/// a shipped reference when it lies in the user's [`shipped_root`] or among the installed sets,
/// else absolute, with links and `..` resolved when the file exists. When `from` and `to` are
/// the same directory, only absolute paths change (an import's found samples saved beside
/// them; shipped files), so saving in place keeps every other byte; empty paths (channels
/// without a sound) and shipped references stay as they are.
pub fn rebase_samples(project: &mut Project, from: &Path, to: &Path) {
    rebase_samples_from(project, from, to, &ShippedRoots::find());
}

/// [`rebase_samples`] with `shipped` as the user's folder and no installed sets.
pub fn rebase_samples_in(project: &mut Project, from: &Path, to: &Path, shipped: Option<&Path>) {
    let roots = ShippedRoots {
        user: shipped.map(Path::to_path_buf),
        installed: None,
    };
    rebase_samples_from(project, from, to, &roots);
}

/// [`rebase_samples`] with `roots` as the folders shipped files are named from.
pub fn rebase_samples_from(project: &mut Project, from: &Path, to: &Path, roots: &ShippedRoots) {
    let (from_resolved, to_resolved) = (resolved(from), resolved(to));
    let in_place = from_resolved == to_resolved;
    for channel in project.channels.values_mut() {
        for stored in [&mut channel.sample, &mut channel.instrument] {
            if stored.as_os_str().is_empty()
                || shipped_part(stored).is_some()
                || (in_place && stored.is_relative())
            {
                continue;
            }
            let file = resolved(&from_resolved.join(&*stored));
            if let Ok(inside) = file.strip_prefix(&to_resolved) {
                *stored = inside.to_path_buf();
            } else if let Some(reference) = roots.reference(&file) {
                *stored = reference;
            } else if !in_place {
                *stored = file;
            }
        }
    }
}

/// `path` with links and `..` resolved when it exists, else made absolute as written.
fn resolved(path: &Path) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| absolute(path))
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use dawsome_engine::project::{Channel, ChannelId};
    use dawsome_engine::time::MilliBpm;

    use super::*;

    fn with_samples(samples: &[&str]) -> Project {
        let mut project = Project::new(44_100, MilliBpm(120_000), 96);
        for (id, sample) in (1..).zip(samples) {
            project
                .channels
                .insert(ChannelId(id), Channel::sampler(format!("ch{id}"), *sample));
        }
        project.next_channel_id = ChannelId(samples.len() as u32 + 1);
        project
    }

    fn samples(project: &Project) -> Vec<PathBuf> {
        project
            .channels
            .values()
            .map(|c| c.sample.clone())
            .collect()
    }

    #[test]
    fn paths_follow_the_file_into_another_directory() {
        let root = std::env::temp_dir().join(format!("dawsome-paths-{}", std::process::id()));
        let (old, new) = (root.join("old"), root.join("new"));
        fs::create_dir_all(old.join("kit")).expect("dirs");
        fs::create_dir_all(new.join("kit")).expect("dirs");
        fs::write(new.join("kit/snare.wav"), b"").expect("file");
        let snare = new.join("kit/snare.wav");
        let mut project = with_samples(&["kit/kick.wav", snare.to_str().expect("utf-8"), ""]);
        let before = project.clone();
        rebase_samples(&mut project, &old, &old.join("."));
        assert_eq!(project, before, "the same directory changes nothing");
        rebase_samples(&mut project, &old, &new);
        assert_eq!(
            samples(&project),
            [
                resolved(&old).join("kit/kick.wav"),
                PathBuf::from("kit/snare.wav"),
                PathBuf::new()
            ]
        );
    }

    // A relative path written with `/`, as every saved project writes it, still names its file
    // after a save into another folder, on every platform. Why the file is opened rather than
    // the path compared: a path the platform cannot open can still equal one built the same way.
    #[test]
    fn a_moved_relative_sample_still_names_its_file() {
        let root = std::env::temp_dir().join(format!("dawsome-paths-open-{}", std::process::id()));
        let (old, new) = (root.join("old"), root.join("new"));
        fs::create_dir_all(old.join("kit")).expect("dirs");
        fs::create_dir_all(&new).expect("dirs");
        fs::write(old.join("kit").join("kick.wav"), b"kick").expect("file");
        let mut project = with_samples(&["kit/kick.wav", "kit/../kit/kick.wav"]);
        rebase_samples(&mut project, &old, &new);
        for sample in samples(&project) {
            assert!(sample.is_absolute(), "{}", sample.display());
            assert_eq!(fs::read(&sample).expect("it opens"), b"kick");
        }
    }

    // An import holds absolute paths; saved beside the files they name, those become relative,
    // while relative paths keep their bytes and paths elsewhere stay as written.
    #[test]
    fn absolute_paths_under_the_same_directory_become_relative() {
        let root = std::env::temp_dir().join(format!("dawsome-paths-same-{}", std::process::id()));
        let dir = root.join("song");
        fs::create_dir_all(dir.join("kit")).expect("dirs");
        fs::write(dir.join("kit/snare.wav"), b"").expect("file");
        let snare = resolved(&dir.join("kit/snare.wav"));
        let elsewhere = root.join("other/hat.wav");
        let mut project = with_samples(&[
            snare.to_str().expect("utf-8"),
            "kit/../kit/kick.wav",
            elsewhere.to_str().expect("utf-8"),
            "",
        ]);
        rebase_samples(&mut project, &dir, &dir.join("."));
        assert_eq!(
            samples(&project),
            [
                PathBuf::from("kit/snare.wav"),
                PathBuf::from("kit/../kit/kick.wav"),
                elsewhere,
                PathBuf::new()
            ]
        );
    }

    // A shipped reference names its file in the data folder given, whatever the project's
    // folder; any other path is the project folder's, an absolute one as it is.
    #[test]
    fn a_shipped_reference_resolves_in_the_data_folder() {
        let (a, b) = (Path::new("/data/a/dawsome"), Path::new("/data/b/dawsome"));
        let stored = Path::new("dawsome:instruments/piano/piano.sfz");
        assert_eq!(
            sound_file_in(Path::new("/songs"), stored, Some(a)),
            a.join("instruments/piano/piano.sfz")
        );
        assert_eq!(
            sound_file_in(Path::new("/elsewhere"), stored, Some(b)),
            b.join("instruments/piano/piano.sfz")
        );
        let plain = Path::new("kit/kick.wav");
        assert_eq!(
            sound_file_in(Path::new("/songs"), plain, Some(a)),
            Path::new("/songs/kit/kick.wav")
        );
        assert_eq!(
            sound_file_in(Path::new("/songs"), Path::new("/abs/kick.wav"), Some(a)),
            Path::new("/abs/kick.wav")
        );
    }

    /// A fresh empty folder of this test's own.
    fn temp(test: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("dawsome-paths-roots-{}", std::process::id()))
            .join(test);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("dirs");
        dir
    }

    // A set's reference is read from the user's folder when it holds the set's folder, else
    // from the installed sets: the user's folder shadows an installed set of the same folder
    // name whole, even for a file only the installed one has. Anything not under
    // `instruments/` is the user's alone; a reference not written as names alone, nothing.
    #[test]
    fn a_reference_is_read_from_the_user_first_then_the_installed_sets() {
        let root = temp("roots");
        let (user, installed) = (root.join("data/dawsome"), root.join("share/instruments"));
        for dir in [
            user.join("instruments/strings"),
            installed.join("strings"),
            installed.join("piano"),
        ] {
            fs::create_dir_all(dir).expect("dirs");
        }
        let roots = ShippedRoots {
            user: Some(user.clone()),
            installed: Some(installed.clone()),
        };
        assert_eq!(
            roots.file("instruments/piano/piano.sfz"),
            Some(installed.join("piano/piano.sfz"))
        );
        assert_eq!(
            roots.file("instruments/strings/only-installed.wav"),
            Some(user.join("instruments/strings/only-installed.wav"))
        );
        assert_eq!(
            roots.file("instruments/organ/organ.sfz"),
            Some(user.join("instruments/organ/organ.sfz")),
            "held by neither: where the user's would be"
        );
        assert_eq!(
            roots.file("starter-kit/Snare.wav"),
            Some(user.join("starter-kit/Snare.wav"))
        );
        assert_eq!(roots.file("instruments/../../piano/piano.sfz"), None);
        let none = ShippedRoots::default();
        assert_eq!(none.file("instruments/piano/piano.sfz"), None);
        assert_eq!(
            sound_file_from(
                Path::new("/songs"),
                Path::new("dawsome:instruments/piano/piano.sfz"),
                &roots
            ),
            installed.join("piano/piano.sfz")
        );
    }

    // A file among the installed sets is named `dawsome:instruments/<its path there>`, one in
    // the user's folder by its path there; saved, a project names an installed set so.
    #[test]
    fn installed_sets_are_named_under_instruments() {
        let root = temp("references");
        let (user, installed) = (root.join("data/dawsome"), root.join("share/instruments"));
        fs::create_dir_all(installed.join("piano")).expect("dirs");
        fs::create_dir_all(user.join("starter-kit")).expect("dirs");
        let piano = installed.join("piano/piano.sfz");
        fs::write(&piano, b"").expect("file");
        let roots = ShippedRoots {
            user: Some(user.clone()),
            installed: Some(installed),
        };
        assert_eq!(
            roots.reference(&piano),
            Some(PathBuf::from("dawsome:instruments/piano/piano.sfz"))
        );
        assert_eq!(
            roots.reference(&user.join("starter-kit/Kick 808.wav")),
            Some(PathBuf::from("dawsome:starter-kit/Kick 808.wav"))
        );
        assert_eq!(roots.reference(&root.join("elsewhere.sfz")), None);
        let mut project = with_samples(&[]);
        project
            .channels
            .insert(ChannelId(1), Channel::instrument("Piano", &piano));
        project.next_channel_id = ChannelId(2);
        rebase_samples_from(&mut project, &root, &root.join("songs"), &roots);
        assert_eq!(
            project.channels[&ChannelId(1)].instrument,
            Path::new("dawsome:instruments/piano/piano.sfz")
        );
    }

    // The install's sets are found from the program's place: `instruments/` beside it first,
    // else (not on Windows) `share/dawsome/instruments` under the prefix of `<prefix>/bin/`.
    #[test]
    fn the_installed_sets_are_found_beside_the_program() {
        let root = temp("beside");
        let program = root.join("bin/dawsome");
        fs::create_dir_all(root.join("bin")).expect("dirs");
        assert_eq!(installed_beside(&program), None, "none installed");
        let shared = root.join("share/dawsome/instruments");
        fs::create_dir_all(&shared).expect("dirs");
        let found = installed_beside(&program);
        if cfg!(windows) {
            assert_eq!(found, None);
        } else {
            assert_eq!(found, Some(shared));
        }
        fs::create_dir_all(root.join("bin/instruments")).expect("dirs");
        assert_eq!(
            installed_beside(&program),
            Some(root.join("bin/instruments"))
        );
    }

    // Saved anywhere, a file in the data folder becomes its shipped reference (an absolute one
    // in place too), a reference stays, and an instrument set rebases like a sample.
    #[test]
    fn shipped_files_save_as_references_and_sets_rebase_like_samples() {
        let root =
            std::env::temp_dir().join(format!("dawsome-paths-shipped-{}", std::process::id()));
        let shipped = root.join("data/dawsome");
        let (old, new) = (root.join("old"), root.join("new"));
        for dir in [
            shipped.join("starter-kit"),
            shipped.join("instruments/pad"),
            old.join("sets"),
            new.clone(),
        ] {
            fs::create_dir_all(dir).expect("dirs");
        }
        fs::write(shipped.join("starter-kit/Kick.wav"), b"").expect("file");
        fs::write(shipped.join("instruments/pad/pad.sfz"), b"").expect("file");
        let kick = shipped.join("starter-kit/Kick.wav");
        let mut project = with_samples(&[
            kick.to_str().expect("utf-8"),
            "dawsome:starter-kit/Snare.wav",
        ]);
        let pad = shipped.join("instruments/pad/pad.sfz");
        project
            .channels
            .insert(ChannelId(3), Channel::instrument("pad", pad));
        project
            .channels
            .insert(ChannelId(4), Channel::instrument("own", "sets/own.sfz"));
        project.next_channel_id = ChannelId(5);
        rebase_samples_in(&mut project, &old, &old, Some(&shipped));
        let instruments = |project: &Project| -> Vec<PathBuf> {
            project
                .channels
                .values()
                .map(|c| c.instrument.clone())
                .collect()
        };
        assert_eq!(
            samples(&project)[..2],
            [
                PathBuf::from("dawsome:starter-kit/Kick.wav"),
                PathBuf::from("dawsome:starter-kit/Snare.wav")
            ]
        );
        assert_eq!(
            instruments(&project)[2..],
            [
                PathBuf::from("dawsome:instruments/pad/pad.sfz"),
                PathBuf::from("sets/own.sfz")
            ]
        );
        rebase_samples_in(&mut project, &old, &new, Some(&shipped));
        assert_eq!(
            samples(&project)[0],
            PathBuf::from("dawsome:starter-kit/Kick.wav")
        );
        assert_eq!(
            instruments(&project)[2],
            PathBuf::from("dawsome:instruments/pad/pad.sfz")
        );
        assert_eq!(
            instruments(&project)[3],
            resolved(&old).join("sets/own.sfz")
        );
    }
}
