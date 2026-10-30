//! A crash leaves a file: a panic anywhere in the program writes its message, where it was
//! raised and a backtrace to `dawsome/crashes/<seconds since the epoch>.txt` in the state
//! folder, then the program ends as it did before (the panic still prints to stderr). Why: a
//! crash in a desktop launch leaves no terminal to read, and a report from a tester with no
//! evidence cannot be reproduced (2026-10-27: an app that ended while a plugin's parameters
//! were scrolled left nothing behind).

use std::fmt::Write as _;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Installs the hook, in front of the one installed before it (which still prints).
pub fn install_hook() {
    let before = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let text = report(
            info,
            &std::backtrace::Backtrace::force_capture().to_string(),
        );
        if let Some(path) =
            crate::project_file::crash_dir().and_then(|dir| write_report(&dir, &text))
        {
            eprintln!("dawsome-app: crash report written to {}", path.display());
        }
        before(info);
    }));
}

/// The report's text: the version, the panic's message and place, and the backtrace.
fn report(info: &PanicHookInfo<'_>, backtrace: &str) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "dawsome-app {} crashed", env!("CARGO_PKG_VERSION"));
    let message = info
        .payload()
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(no message)".to_owned());
    let _ = writeln!(text, "message: {message}");
    match info.location() {
        Some(at) => {
            let _ = writeln!(text, "at: {}:{}:{}", at.file(), at.line(), at.column());
        }
        None => {
            let _ = writeln!(text, "at: unknown");
        }
    }
    let _ = writeln!(text, "\nbacktrace:\n{backtrace}");
    text
}

/// Writes `text` to a new file in `dir`, named by the second; none when it cannot be written.
fn write_report(dir: &Path, text: &str) -> Option<PathBuf> {
    std::fs::create_dir_all(dir).ok()?;
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("{seconds}.txt"));
    std::fs::write(&path, text).ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The report names the version, the message and the place, then the backtrace it was
    // given; a panic raised in a thread of this test gives the hook a real PanicHookInfo.
    #[test]
    fn a_report_names_the_message_the_place_and_the_backtrace() {
        let (sender, receiver) = std::sync::mpsc::channel::<String>();
        let before = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = sender.send(report(info, "frame 0: here"));
        }));
        let thread = std::thread::spawn(|| panic!("a known message {}", 7));
        let _ = thread.join();
        std::panic::set_hook(before);
        let text = receiver.recv().expect("the hook ran");
        assert!(
            text.starts_with(&format!(
                "dawsome-app {} crashed\n",
                env!("CARGO_PKG_VERSION")
            )),
            "{text}"
        );
        assert!(text.contains("message: a known message 7\n"), "{text}");
        // As the compiler spells this file's path (with `\` on Windows), as the panic does.
        assert!(text.contains(&format!("at: {}:", file!())), "{text}");
        assert!(text.ends_with("backtrace:\nframe 0: here\n"), "{text}");
    }

    // The file lands in the folder it is given (made if missing), named by the second; a
    // folder that cannot be made gives none.
    #[test]
    fn a_report_is_written_to_the_crash_folder_named_by_the_second() {
        let root = std::env::temp_dir().join(format!("dawsome-crash-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("dawsome/crashes");
        let path = write_report(&dir, "the text").expect("written");
        assert_eq!(path.parent(), Some(dir.as_path()));
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "the text");
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        assert!(
            name.ends_with(".txt") && name.trim_end_matches(".txt").parse::<u64>().is_ok(),
            "{name}"
        );
        let _ = std::fs::remove_dir_all(&root);
        let file_not_dir =
            std::env::temp_dir().join(format!("dawsome-crash-file-{}", std::process::id()));
        std::fs::write(&file_not_dir, "x").expect("a file");
        assert!(write_report(&file_not_dir.join("under-a-file"), "t").is_none());
        let _ = std::fs::remove_file(&file_not_dir);
    }
}
