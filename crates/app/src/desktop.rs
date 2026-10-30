//! What the desktop knows the app by: the application id its desktop entry goes by (or the one
//! `DAWSOME_APP_ID` names), and the window icon, carried in the binary; and the settings its
//! windows open with, the main one's and the pop-out windows' beside it ([`PopOut`]).

use std::io::Cursor;

use iced::window::{self, Icon};
use iced::{Point, Size};
use thiserror::Error;

/// The window's application id, and its class on X11: the name of the desktop entry
/// (`assets/dawsome.desktop`), so a Linux desktop shows the entry's name and icon for the window.
#[cfg(target_os = "linux")]
pub const APP_ID: &str = "dawsome";

/// The environment variable that, set, is the window's application id instead of [`APP_ID`].
/// Why: the scripted runs that open a window (`scripts/ui-check.sh`, `scripts/measure.sh`) set
/// `dawsome-test`, so a window rule of the desktop can place their windows apart and keep them
/// from taking focus, while the app itself stays `dawsome`.
#[cfg(target_os = "linux")]
pub const APP_ID_VARIABLE: &str = "DAWSOME_APP_ID";

/// The 64-pixel picture of the icon set, the size the window icon is made from.
const ICON_64: &[u8] = include_bytes!("../../../assets/icon/dawsome-64.png");

/// Why the window icon could not be made.
#[derive(Debug, Error)]
pub enum IconError {
    #[error("the icon is not a PNG: {0}")]
    Decode(#[from] png::DecodingError),
    #[error("the icon is {0:?} at {1:?} bits, not 8-bit RGBA")]
    Format(png::ColorType, png::BitDepth),
    #[error("the window takes no such icon: {0}")]
    Window(#[from] window::icon::Error),
}

/// The window settings the app opens with: `size`, the application id and, unless it cannot
/// be made, the icon.
pub fn window_settings(size: Size) -> window::Settings {
    let icon = window_icon()
        .map_err(|error| eprintln!("dawsome-app: no window icon: {error}"))
        .ok();
    window::Settings {
        size,
        icon,
        platform_specific: platform_specific(),
        ..window::Settings::default()
    }
}

/// The app's windows beside the main one, each an OS window the app opens and closes to match
/// its state: the instrument window, the Settings window, the Help window, the parameter
/// window and the sample editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopOut {
    Instrument,
    Settings,
    Help,
    /// The parameter window: one plugin's parameters.
    Params,
    /// The sample editor: one channel's sound.
    SampleEditor,
}

impl PopOut {
    /// Its size at its first opening.
    pub fn first_size(self) -> Size {
        match self {
            PopOut::Instrument => crate::ui::instrument_window::WINDOW_SIZE,
            PopOut::Settings => crate::ui::settings_panel::WINDOW_SIZE,
            PopOut::Help => crate::ui::help_window::WINDOW_SIZE,
            PopOut::Params => crate::ui::param_window::WINDOW_SIZE,
            PopOut::SampleEditor => crate::ui::sample_editor::WINDOW_SIZE,
        }
    }

    /// The least the window is made.
    pub fn least_size(self) -> Size {
        match self {
            PopOut::Instrument => crate::ui::instrument_window::WINDOW_MIN,
            PopOut::Settings => crate::ui::settings_panel::WINDOW_MIN,
            PopOut::Help => crate::ui::help_window::WINDOW_MIN,
            PopOut::Params => crate::ui::param_window::WINDOW_MIN,
            PopOut::SampleEditor => crate::ui::sample_editor::WINDOW_MIN,
        }
    }
}

/// A pop-out window's settings: as the main window's (the application id, the icon), at
/// `size` and `position` (none: where the system puts it), at `level`, never smaller than its
/// least. Its close button asks the app, which closes it (the instrument window hands the
/// selection back).
pub fn pop_out_settings(
    window: PopOut,
    size: Size,
    position: Option<Point>,
    level: window::Level,
) -> window::Settings {
    window::Settings {
        min_size: Some(window.least_size()),
        position: position.map_or(window::Position::Default, window::Position::Specific),
        exit_on_close_request: false,
        level,
        ..window_settings(size)
    }
}

#[cfg(target_os = "linux")]
fn platform_specific() -> window::settings::PlatformSpecific {
    window::settings::PlatformSpecific {
        application_id: app_id(),
        ..Default::default()
    }
}

/// The application id the app's windows go by ([`APP_ID_VARIABLE`]'s, else [`APP_ID`]), which its
/// plugin windows carry as their X11 class too.
#[cfg(target_os = "linux")]
pub fn app_id() -> String {
    application_id(std::env::var(APP_ID_VARIABLE).ok())
}

/// The application id: `set` (the value of [`APP_ID_VARIABLE`]) unless it is missing or blank,
/// else [`APP_ID`].
#[cfg(target_os = "linux")]
fn application_id(set: Option<String>) -> String {
    set.filter(|id| !id.trim().is_empty())
        .unwrap_or_else(|| APP_ID.to_owned())
}

/// The environment variable that chooses the window system the app's windows open on, on Linux:
/// `x11` or `wayland` (any case). Unset, a desktop run under a Wayland session that also has an
/// X11 display (XWayland) opens them on X11 ([`window_system`]).
#[cfg(target_os = "linux")]
pub const WINDOW_SYSTEM_VARIABLE: &str = "DAWSOME_WINDOW_SYSTEM";

/// Where the app's windows open on Linux: as the toolkit picks (Wayland when the session has it,
/// else X11), or on X11 through the session's X11 display.
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowSystem {
    Toolkit,
    X11,
}

/// The window system for a run: `asked` ([`WINDOW_SYSTEM_VARIABLE`]'s value) when it names one;
/// else X11 for a desktop run where the session has both a Wayland and an X11 display, and the
/// toolkit's own choice for a `scripted` one; X11 only where there is an X11 display. A value
/// that names neither is said back, and the run goes on as though unset.
///
/// Why X11 under Wayland: a Wayland compositor gives an app no say in how its windows stack
/// (winit 0.30's Wayland `set_window_level` is empty, and iced 0.14 offers a new window no xdg
/// parent: its settings have none, winit's `with_parent_window` is X11's and Windows', and the
/// xdg_toplevel, the one handle a parent is set on, never leaves iced's windows), so the
/// instrument, Settings, Help and parameter windows went under the main window on a click
/// (found 2026-10-27 on KDE Plasma's Wayland session). On X11 the pop-out windows keep
/// `_NET_WM_STATE_ABOVE` while the app has the keyboard, as the plugin windows (X11 always)
/// did, and all of them share one server. Measured on that desktop: on Wayland the instrument
/// window went under the main window once it was activated; on X11 it stayed above (KWin's
/// above layer), and on the 4K screen at scale 1.7 the window the compositor shows is the app's
/// own buffer pixel for pixel (KDE's "legacy X11 apps scale themselves", the app at Xft.dpi's
/// 1.698; on a scale-1 screen the compositor shrinks that buffer to the window's size, which
/// is as large as on Wayland, its text a little softer). Why the toolkit's choice for a scripted
/// run: its shots and probes stay as they were in every session (under X11 every window draws
/// at Xft.dpi's scale, the test windows' scale-1 side screen too, so a shot's pixels would
/// follow the desktop's scale setting).
#[cfg(target_os = "linux")]
pub fn window_system(
    asked: Option<&str>,
    wayland: bool,
    x11: bool,
    scripted: bool,
) -> (WindowSystem, Option<String>) {
    let asked = asked.map(str::trim).filter(|asked| !asked.is_empty());
    let named = asked.map(str::to_ascii_lowercase);
    let (wanted, said) = match named.as_deref() {
        Some("x11") => (Some(WindowSystem::X11), None),
        Some("wayland") => (Some(WindowSystem::Toolkit), None),
        Some(_) => (
            None,
            Some(format!(
                "{WINDOW_SYSTEM_VARIABLE}={}: x11 or wayland; the windows open as though unset",
                asked.unwrap_or_default()
            )),
        ),
        None => (None, None),
    };
    let chosen = match wanted {
        Some(WindowSystem::X11) if x11 => WindowSystem::X11,
        Some(_) => WindowSystem::Toolkit,
        None if wayland && x11 && !scripted => WindowSystem::X11,
        None => WindowSystem::Toolkit,
    };
    (chosen, said)
}

/// Opens the app's windows on the window system [`window_system`] gives, before the toolkit
/// reads its environment: on X11, the Wayland display's variables leave the process's own
/// environment (winit 0.30 reads only `WAYLAND_DISPLAY`, `WAYLAND_SOCKET` and `DISPLAY`;
/// `WINIT_UNIX_BACKEND` went in 0.29), so the plugin scan's children inherit X11 alone too. Call
/// it while the program has one thread: a change to the environment races any other thread
/// that reads it.
#[cfg(target_os = "linux")]
pub fn open_windows_on_chosen_system(scripted: bool) {
    let present = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
    let wayland = present("WAYLAND_DISPLAY") || present("WAYLAND_SOCKET");
    let asked = std::env::var(WINDOW_SYSTEM_VARIABLE).ok();
    let (chosen, said) = window_system(asked.as_deref(), wayland, present("DISPLAY"), scripted);
    if let Some(said) = said {
        eprintln!("dawsome-app: {said}");
    }
    if chosen == WindowSystem::X11 {
        std::env::remove_var("WAYLAND_DISPLAY");
        std::env::remove_var("WAYLAND_SOCKET");
    }
}

/// Windows finds a window's taskbar entry by its program, not by an id.
#[cfg(not(target_os = "linux"))]
fn platform_specific() -> window::settings::PlatformSpecific {
    window::settings::PlatformSpecific::default()
}

/// The window icon, from the 64-pixel picture.
fn window_icon() -> Result<Icon, IconError> {
    let (rgba, width, height) = decode(ICON_64)?;
    let icon = window::icon::from_rgba(rgba, width, height).map_err(window::icon::Error::from)?;
    Ok(icon)
}

/// An 8-bit RGBA PNG as its pixels, width and height.
fn decode(png: &[u8]) -> Result<(Vec<u8>, u32, u32), IconError> {
    let mut reader = png::Decoder::new(Cursor::new(png)).read_info()?;
    match reader.output_color_type() {
        (png::ColorType::Rgba, png::BitDepth::Eight) => {}
        (color, depth) => return Err(IconError::Format(color, depth)),
    }
    let mut rgba = vec![0; reader.output_buffer_size().unwrap_or_default()];
    let frame = reader.next_frame(&mut rgba)?;
    rgba.truncate(frame.buffer_size());
    Ok((rgba, frame.width, frame.height))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Where the windows open, the table worked out from the rule (2026-10-27):
    // a desktop run under a Wayland session with XWayland opens on X11, a scripted one as the
    // toolkit picks; an X11 session or a Wayland one without X11 as the toolkit picks; the
    // variable's x11 (any case, spaces round it) on X11 where there is an X11 display, scripted
    // or not, its wayland as the toolkit picks; an empty one as unset; another value said back
    // and taken as unset.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_windows_open_on_x11_under_wayland_unless_asked_or_scripted() {
        use WindowSystem::{Toolkit, X11};
        let table = [
            (None, true, true, false, X11),
            (None, true, true, true, Toolkit),
            (None, true, false, false, Toolkit),
            (None, false, true, false, Toolkit),
            (None, false, true, true, Toolkit),
            (Some("wayland"), true, true, false, Toolkit),
            (Some("Wayland"), true, true, true, Toolkit),
            (Some("x11"), true, true, false, X11),
            (Some(" X11 "), true, true, true, X11),
            (Some("x11"), true, false, false, Toolkit),
            (Some("x11"), false, true, true, X11),
            (Some(""), true, true, false, X11),
            (Some("   "), true, true, true, Toolkit),
        ];
        for (asked, wayland, x11, scripted, want) in table {
            let (chosen, said) = window_system(asked, wayland, x11, scripted);
            assert_eq!(chosen, want, "{asked:?} {wayland} {x11} {scripted}");
            assert_eq!(said, None, "{asked:?}");
        }
        let (chosen, said) = window_system(Some("mir"), true, true, false);
        assert_eq!(chosen, X11, "taken as unset");
        assert_eq!(
            said.as_deref(),
            Some("DAWSOME_WINDOW_SYSTEM=mir: x11 or wayland; the windows open as though unset")
        );
    }

    #[test]
    fn the_window_icon_decodes_to_64_by_64_pixels() {
        let (rgba, width, height) = decode(ICON_64).expect("decodes");
        assert_eq!((width, height), (64, 64));
        assert_eq!(rgba.len(), 64 * 64 * 4);
        let alphas: Vec<u8> = rgba.chunks_exact(4).map(|pixel| pixel[3]).collect();
        assert!(alphas.contains(&255), "it draws something opaque");
        assert!(alphas.contains(&0), "its corners show the desktop through");
        assert!(window_icon().is_ok());
        assert!(window_settings(Size::new(800.0, 600.0)).icon.is_some());
    }

    // The instrument window is the app's too: the same icon and (on Linux) application id as
    // the main window, at the size and place given, never smaller than 420 x 300, and its close
    // button left to the app. With no place given the system places it. The
    // Settings window too, never smaller than 420 x 240.
    #[test]
    fn the_instrument_window_is_the_apps_at_its_place_and_least_size() {
        let main = window_settings(Size::new(800.0, 600.0));
        let placed = pop_out_settings(
            PopOut::Instrument,
            Size::new(560.0, 420.0),
            Some(Point::new(40.0, 30.0)),
            window::Level::AlwaysOnTop,
        );
        assert_eq!(placed.size, Size::new(560.0, 420.0));
        assert_eq!(placed.min_size, Some(Size::new(420.0, 300.0)));
        assert!(matches!(
            placed.position,
            window::Position::Specific(at) if at == Point::new(40.0, 30.0)
        ));
        assert!(!placed.exit_on_close_request);
        assert!(placed.icon.is_some());
        assert_eq!(placed.platform_specific, main.platform_specific);
        assert_eq!(placed.level, window::Level::AlwaysOnTop);
        let unplaced = pop_out_settings(
            PopOut::Instrument,
            Size::new(600.0, 500.0),
            None,
            window::Level::Normal,
        );
        assert!(matches!(unplaced.position, window::Position::Default));
        assert_eq!(unplaced.level, window::Level::Normal);
        let settings = pop_out_settings(
            PopOut::Settings,
            PopOut::Settings.first_size(),
            None,
            window::Level::AlwaysOnTop,
        );
        assert_eq!(settings.size, Size::new(720.0, 560.0));
        assert_eq!(settings.min_size, Some(Size::new(420.0, 240.0)));
        assert!(!settings.exit_on_close_request);
        assert_eq!(settings.platform_specific, main.platform_specific);
    }

    #[test]
    fn a_picture_that_is_not_rgba_is_refused() {
        let mut gray = Vec::new();
        let mut encoder = png::Encoder::new(&mut gray, 2, 2);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("header");
        writer.write_image_data(&[0, 64, 128, 255]).expect("pixels");
        writer.finish().expect("written");
        assert!(matches!(
            decode(&gray),
            Err(IconError::Format(png::ColorType::Grayscale, _))
        ));
        assert!(matches!(decode(b"not a png"), Err(IconError::Decode(_))));
    }

    // DAWSOME_APP_ID, when set, is the application id (and X11 class) instead of `dawsome`, so
    // a desktop can tell a scripted run's window from the app's; unset or blank, `dawsome`.
    #[cfg(target_os = "linux")]
    #[test]
    fn dawsome_app_id_names_the_window_when_set() {
        assert_eq!(APP_ID_VARIABLE, "DAWSOME_APP_ID");
        assert_eq!(
            application_id(Some("dawsome-test".to_owned())),
            "dawsome-test"
        );
        assert_eq!(application_id(None), "dawsome");
        assert_eq!(application_id(Some(String::new())), "dawsome");
        assert_eq!(application_id(Some("  ".to_owned())), "dawsome");
    }

    // Linux only: desktop entries and application ids are a Linux desktop's; Windows knows a
    // window by its program.
    #[cfg(target_os = "linux")]
    mod desktop_entry {
        use std::path::Path;

        use super::*;

        const ENTRY: &str = include_str!("../../../assets/dawsome.desktop");

        /// The value of `key` in the desktop entry.
        fn entry(key: &str) -> &'static str {
            ENTRY
                .lines()
                .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
                .unwrap_or_else(|| panic!("the desktop entry has no {key}"))
        }

        // The desktop matches a window to its entry by the application id (Wayland) or the
        // window class (X11), starts the entry's program, and finds the icon by its name.
        #[test]
        fn the_desktop_entry_names_this_program_its_window_class_and_its_icon() {
            let exec = entry("Exec");
            assert_eq!(exec.split_whitespace().next(), Some(env!("CARGO_BIN_NAME")));
            assert_eq!(entry("StartupWMClass"), APP_ID);
            let icons = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/icon");
            assert!(icons.join(format!("{}.svg", entry("Icon"))).is_file());
            assert_eq!(
                window_settings(Size::new(800.0, 600.0))
                    .platform_specific
                    .application_id,
                APP_ID
            );
        }

        // A double-click on a project in a file manager starts the app with it: the entry opens
        // the type the shipped MIME package declares, and the package gives that type to files
        // named *.dawsome (scripts/install-check.sh checks the installed pair too).
        #[test]
        fn the_desktop_entry_opens_the_project_type_the_mime_package_declares() {
            const PACKAGE: &str = include_str!("../../../assets/dawsome-mime.xml");
            assert_eq!(entry("MimeType"), "application/x-dawsome-project;");
            assert!(
                entry("Exec").ends_with(" %f"),
                "the file is passed to the program"
            );
            assert!(PACKAGE.contains(r#"<mime-type type="application/x-dawsome-project">"#));
            assert!(PACKAGE.contains(&format!(
                r#"<glob pattern="*.{}"/>"#,
                crate::project_file::EXTENSION
            )));
        }
    }
}
