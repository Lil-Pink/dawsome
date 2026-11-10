//! Plugin windows on Linux: top-level X11 windows of ours, on a connection of their own (through
//! XWayland on a Wayland desktop). A window opened as an embedder ([`Display::open_embedder`],
//! VST3's) also hears its children, and is the XEmbed embedder of a child that asks to be one
//! (`xembed.rs` holds the rules); a plain one (CLAP's) hears only itself, as before.
//!
//! Every plugin window hears its keys and its focus. X11 reports a key to the focus window, or
//! to the window under the pointer when that lies inside the focus window, and from there to the
//! first window up the tree that some client asked to hear keys on: so our window hears a key
//! only while no window of the plugin's on the way takes keys itself. The test interface's
//! window takes none, so its keys reach ours; Surge XT's (JUCE) takes them and takes the focus
//! when clicked, so with the pointer over it or its window focused the key event is the
//! plugin's alone. So while any plugin window is open the connection also hears every key's raw
//! event (XInput 2, on the root window), and passes one on while the X focus is in a window of
//! ours or a window inside it, as that window's own FocusIn and FocusOut events tell. One key
//! comes once: from its key event when that reached our window, else from its raw event
//! ([`KeyState`]).

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use x11rb::connection::Connection;
use x11rb::errors::ConnectionError;
use x11rb::properties::WmSizeHints;
use x11rb::protocol::xinput::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ClientMessageEvent, ConfigureNotifyEvent,
    ConfigureWindowAux, ConnectionExt as _, CreateWindowAux, EventMask, FocusInEvent,
    FocusOutEvent, KeyButMask, KeyPressEvent, Mapping, NotifyDetail, NotifyMode, PropMode,
    Property as State, WindowClass,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::x11_utils::Serialize as _;

use super::keys::typed_of_keysym;
use super::xembed::{Action, Embedders};
use super::{clamp_side, class, Typed, WindowEvent};

/// What a desktop without X11 shows instead of a plugin window.
pub const UNAVAILABLE: &str = "Plugin windows need X11 (XWayland)";

/// An X11 window id.
pub type WindowId = u32;

/// Where the pointer is over `window` (any client's, the app's own main window for one), asked
/// of the X server now on a connection of its own: its pixels from the window's top-left corner,
/// which lie outside it when the pointer does. `None` without a display, for a window that is
/// gone, or with the pointer on another screen. Why a connection per call: it is asked once a
/// file is dropped, and the toolkit's own connection is not ours to use.
pub fn pointer_in(window: WindowId) -> Option<(i32, i32)> {
    let (conn, _) = RustConnection::connect(None).ok()?;
    let pointer = conn.query_pointer(window).ok()?.reply().ok()?;
    pointer
        .same_screen
        .then_some((i32::from(pointer.win_x), i32::from(pointer.win_y)))
}

struct Atoms {
    wm_protocols: u32,
    wm_delete_window: u32,
    net_wm_name: u32,
    utf8_string: u32,
    net_wm_state: u32,
    net_wm_state_above: u32,
}

/// The atoms XEmbed's messages and property go by, interned when the first embedder opens.
#[derive(Debug, Clone, Copy)]
struct XembedAtoms {
    xembed: u32,
    info: u32,
}

/// What every plugin window hears of itself: its size and its going (and its closing, which a
/// client message brings unasked), the keys that reach it and its focus. Why the keys: the app's
/// typing keyboard plays while a plugin window is in front; why the focus: a key held as the
/// focus leaves is let go of where we never hear it, so its note is ended then.
fn plugin_window() -> EventMask {
    EventMask::STRUCTURE_NOTIFY
        | EventMask::KEY_PRESS
        | EventMask::KEY_RELEASE
        | EventMask::FOCUS_CHANGE
}
/// What an embedder hears besides: its children appearing, going and moving, and its focus.
fn embedder() -> EventMask {
    EventMask::SUBSTRUCTURE_NOTIFY | EventMask::FOCUS_CHANGE
}

/// What an embedder hears of a child it watches: its properties (`_XEMBED_INFO`) and its
/// structure.
fn watched_child() -> EventMask {
    EventMask::PROPERTY_CHANGE | EventMask::STRUCTURE_NOTIFY
}

/// The connection plugin windows live on.
pub struct Display {
    conn: Arc<RustConnection>,
    ours: Arc<Ours>,
    /// What the key and focus events heard so far leave to remember.
    keys: RefCell<KeyState>,
    black: u32,
    atoms: Atoms,
    xembed: Cell<Option<XembedAtoms>>,
    embedders: RefCell<Embedders>,
    /// The X server's keyboard mapping, read when [`Display::typed`] first asks and again after
    /// the server says it changed.
    keymap: RefCell<Option<Keymap>>,
}

/// The keysyms of every keycode: `per` of them for each from `first` on.
struct Keymap {
    first: u8,
    per: u8,
    keysyms: Vec<u32>,
}

/// One window of ours, for code that holds no display (a plugin's frame, called by the plugin,
/// and the view it closes as it goes): its size and its closing. It shares the display's
/// connection, which x11rb locks for itself, so it can be held anywhere.
#[derive(Debug, Clone)]
pub struct WindowHandle {
    conn: Arc<RustConnection>,
    ours: Arc<Ours>,
    window: WindowId,
}

impl WindowHandle {
    pub fn id(&self) -> WindowId {
        self.window
    }

    /// Gives the window the size its plugin asked for.
    pub fn resize(&self, width: u32, height: u32) {
        configure_size(&self.conn, self.window, width, height);
    }

    /// Gives the window a size it keeps: its least and its most size that size, then the size.
    pub fn fix(&self, width: u32, height: u32) {
        let size = (clamp_side(width), clamp_side(height));
        // A window that is already gone has nothing to keep.
        let _ = fixed_hints(size).send(&self.conn, self.window);
        configure_size(&self.conn, self.window, width, height);
    }

    pub fn close(&self) {
        let _ = self.conn.destroy_window(self.window);
        self.ours.gone(&self.conn, self.window);
        let _ = self.conn.flush();
    }
}

/// The plugin windows open on a connection, shared with their handles (a handle closes its
/// window too). While any is open the connection hears the raw key events, when the server has
/// them; with none open it stops, so no event waits unread while nothing polls the connection
/// (the hosts tick only while a window or other work is open).
#[derive(Debug)]
struct Ours {
    root: WindowId,
    raw_keys: bool,
    windows: Mutex<Vec<WindowId>>,
}

impl Ours {
    /// `window` opened; whether it is the only one open.
    fn opened(&self, conn: &RustConnection, window: WindowId) -> bool {
        let mut windows = self.windows.lock().unwrap_or_else(PoisonError::into_inner);
        let first = windows.is_empty();
        if first && self.raw_keys {
            hear_raw_keys(conn, self.root, true);
        }
        windows.push(window);
        first
    }

    /// `window` closed or gone, told once or more.
    fn gone(&self, conn: &RustConnection, window: WindowId) {
        let mut windows = self.windows.lock().unwrap_or_else(PoisonError::into_inner);
        let had = windows.len();
        windows.retain(|&open| open != window);
        if windows.is_empty() && had > 0 && self.raw_keys {
            hear_raw_keys(conn, self.root, false);
        }
    }
}

/// XI2.h's XIAllMasterDevices: the master keyboard's raw events, one for each key. Every slave
/// device (each keyboard, and XTEST's) sends its own as well, which XIAllDevices would add.
const ALL_MASTER_DEVICES: xinput::DeviceId = 1;

/// Starts (`on`) or stops the raw key events of the master keyboard on `root`, where the X
/// server sends raw events.
fn hear_raw_keys(conn: &RustConnection, root: WindowId, on: bool) {
    let mask = if on {
        xinput::XIEventMask::RAW_KEY_PRESS | xinput::XIEventMask::RAW_KEY_RELEASE
    } else {
        xinput::XIEventMask::from(0u32)
    };
    let select = xinput::EventMask {
        deviceid: ALL_MASTER_DEVICES,
        mask: vec![mask],
    };
    // A server that refuses it leaves the keys to their key events, as before.
    let _ = conn.xinput_xi_select_events(root, &[select]);
    let _ = conn.flush();
}

/// Whether the X server sends raw key events: XInput 2, asked for as 2.1, from which version on
/// they come whatever another client has grabbed (2.0 sends them too, but not during a grab).
/// Else why not.
fn raw_keys_heard(conn: &RustConnection) -> Result<(), String> {
    let reply = conn
        .xinput_xi_query_version(2, 1)
        .map_err(|error| error.to_string())?
        .reply()
        .map_err(|error| error.to_string())?;
    if reply.major_version < 2 {
        return Err(format!(
            "XInput {}.{}",
            reply.major_version, reply.minor_version
        ));
    }
    Ok(())
}

/// Asks for `window` to be `width` by `height`.
fn configure_size(conn: &RustConnection, window: WindowId, width: u32, height: u32) {
    // A window that is already gone has nothing to resize.
    let _ = conn.configure_window(window, &size_request(width, height));
    let _ = conn.flush();
}

/// The ConfigureWindow values that ask for a size of `width` by `height`, each side held to what
/// a window takes: the width and the height alone (value-mask 4 | 8), never a place, a border or
/// a stacking. Every resize of a plugin window is this request: a plugin's own, the answer to a
/// user's resize, and the same answer asked again while the window manager has not given it.
fn size_request(width: u32, height: u32) -> ConfigureWindowAux {
    ConfigureWindowAux::new()
        .width(u32::from(clamp_side(width)))
        .height(u32::from(clamp_side(height)))
}

impl Display {
    /// Connects to `$DISPLAY`; fails with the reason to show when there is no X11.
    pub fn connect() -> Result<Self, String> {
        if std::env::var_os("DISPLAY").is_none() {
            return Err(UNAVAILABLE.into());
        }
        let (conn, screen) =
            RustConnection::connect(None).map_err(|error| format!("{UNAVAILABLE}: {error}"))?;
        let (root, black) = {
            let screen = &conn.setup().roots[screen];
            (screen.root, screen.black_pixel)
        };
        let intern = |name: &[u8]| -> Result<u32, String> {
            Ok(conn
                .intern_atom(false, name)
                .map_err(|error| error.to_string())?
                .reply()
                .map_err(|error| error.to_string())?
                .atom)
        };
        let atoms = Atoms {
            wm_protocols: intern(b"WM_PROTOCOLS")?,
            wm_delete_window: intern(b"WM_DELETE_WINDOW")?,
            net_wm_name: intern(b"_NET_WM_NAME")?,
            utf8_string: intern(b"UTF8_STRING")?,
            net_wm_state: intern(b"_NET_WM_STATE")?,
            net_wm_state_above: intern(b"_NET_WM_STATE_ABOVE")?,
        };
        let raw_keys = match raw_keys_heard(&conn) {
            Ok(()) => true,
            Err(why) => {
                // Said once, for the record and the tests; keys then come as before.
                eprintln!(
                    "plugin windows: no raw key events on this X server ({why}): a key reaches \
                     the typing keyboard only when it comes to our window itself"
                );
                false
            }
        };
        Ok(Self {
            conn: Arc::new(conn),
            ours: Arc::new(Ours {
                root,
                raw_keys,
                windows: Mutex::new(Vec::new()),
            }),
            keys: RefCell::new(KeyState::default()),
            black,
            atoms,
            xembed: Cell::new(None),
            embedders: RefCell::new(Embedders::default()),
            keymap: RefCell::new(None),
        })
    }

    /// Opens a window titled `title` of `width` by `height` pixels. A window that is not
    /// `resizable` asks the window manager to keep that size.
    pub fn open(
        &self,
        title: &str,
        width: u32,
        height: u32,
        resizable: bool,
    ) -> Result<WindowId, String> {
        self.try_open(title, width, height, resizable, plugin_window())
            .map_err(|error| format!("cannot open the plugin window: {error}"))
    }

    /// [`open`](Self::open), and the window is the XEmbed embedder of its children: a child
    /// with `_XEMBED_INFO` is told it is embedded, mapped as it asks and given our focus.
    pub fn open_embedder(
        &self,
        title: &str,
        width: u32,
        height: u32,
        resizable: bool,
    ) -> Result<WindowId, String> {
        let opened = self.xembed_atoms().and_then(|_| {
            self.try_open(
                title,
                width,
                height,
                resizable,
                plugin_window() | embedder(),
            )
        });
        let window = opened.map_err(|error| format!("cannot open the plugin window: {error}"))?;
        self.embedders.borrow_mut().add(window);
        Ok(window)
    }

    /// `window`, for holding apart from the display.
    pub fn handle(&self, window: WindowId) -> WindowHandle {
        WindowHandle {
            conn: Arc::clone(&self.conn),
            ours: Arc::clone(&self.ours),
            window,
        }
    }

    /// XEmbed's atoms, interned once.
    fn xembed_atoms(&self) -> Result<XembedAtoms, Box<dyn std::error::Error>> {
        if let Some(atoms) = self.xembed.get() {
            return Ok(atoms);
        }
        let atoms = XembedAtoms {
            xembed: self.conn.intern_atom(false, b"_XEMBED")?.reply()?.atom,
            info: self.conn.intern_atom(false, b"_XEMBED_INFO")?.reply()?.atom,
        };
        self.xembed.set(Some(atoms));
        Ok(atoms)
    }

    fn try_open(
        &self,
        title: &str,
        width: u32,
        height: u32,
        resizable: bool,
        mask: EventMask,
    ) -> Result<WindowId, Box<dyn std::error::Error>> {
        let (w, h) = (clamp_side(width), clamp_side(height));
        let window = self.conn.generate_id()?;
        self.conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            self.ours.root,
            0,
            0,
            w,
            h,
            0,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &window_aux(self.black, mask),
        )?;
        let class = class();
        for property in properties(title, (w, h), resizable, class.as_deref(), &self.atoms) {
            property.send(&self.conn, window)?;
        }
        self.conn.map_window(window)?;
        // A round trip, not a flush: the plugin's own connection parents its child to this window
        // as soon as it is told, and X keeps order within one connection only (Dexed's editor
        // failed so).
        self.conn.sync()?;
        if self.ours.opened(&self.conn, window) {
            // Nothing heard while no window was open counts: no key down, no focus of ours.
            *self.keys.borrow_mut() = KeyState::default();
        }
        Ok(window)
    }

    /// Gives `window` the size a plugin asked for.
    pub fn resize(&self, window: WindowId, width: u32, height: u32) {
        configure_size(&self.conn, window, width, height);
    }

    pub fn close(&self, window: WindowId) {
        let _ = self.conn.destroy_window(window);
        self.ours.gone(&self.conn, window);
        self.keys.borrow_mut().gone(window);
        let _ = self.conn.flush();
    }

    /// Asks the window manager to keep `window` above other windows (`above`), or not: the
    /// EWMH request a client sends the root window for a mapped window's `_NET_WM_STATE_ABOVE`.
    pub fn set_above(&self, window: WindowId, above: bool) {
        let event = above_request(
            window,
            above,
            self.atoms.net_wm_state,
            self.atoms.net_wm_state_above,
        );
        let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
        // A window that is already gone has nothing to stack.
        let _ = self.conn.send_event(false, self.ours.root, mask, event);
        let _ = self.conn.flush();
    }

    /// What happened to our windows since the last call.
    pub fn events(&self) -> Vec<WindowEvent> {
        // All that came, first: a key's events are read with their neighbours (a repeat is a
        // release and a press together; a raw key's twin follows it).
        let mut heard = Vec::new();
        while let Ok(Some(event)) = self.conn.poll_for_event() {
            heard.push(event);
        }
        // Asked of the server once in a batch, and only for a raw press that would pass.
        let mut asked = None;
        let mut held = || *asked.get_or_insert_with(|| self.held());
        let events = window_events(
            &heard,
            self.atoms.wm_delete_window,
            &mut self.keys.borrow_mut(),
            &mut held,
        );
        for event in &heard {
            match event {
                Event::MappingNotify(changed) if changed.request == Mapping::KEYBOARD => {
                    self.keymap.borrow_mut().take();
                }
                Event::DestroyNotify(gone) if gone.event == gone.window => {
                    self.ours.gone(&self.conn, gone.window);
                }
                _ => {}
            }
            self.embed(event);
        }
        events
    }

    /// The modifiers and buttons held now (QueryPointer's mask: the core state), for a raw key,
    /// which carries none. Nothing held when the server cannot say.
    fn held(&self) -> KeyButMask {
        self.conn
            .query_pointer(self.ours.root)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map_or(KeyButMask::from(0u16), |pointer| pointer.mask)
    }

    /// What the key of `scancode` (a keycode less 8, as [`WindowEvent::Key`] has it) types on
    /// the X server's keyboard mapping with no modifier held: its first keysym's. `None` when
    /// it types nothing this knows or the server cannot say.
    pub fn typed(&self, scancode: u32) -> Option<Typed> {
        let keycode = u8::try_from(scancode.checked_add(8)?).ok()?;
        if self.keymap.borrow().is_none() {
            let keymap = self.read_keymap()?;
            *self.keymap.borrow_mut() = Some(keymap);
        }
        let keymap = self.keymap.borrow();
        let keymap = keymap.as_ref()?;
        let at = usize::from(keycode.checked_sub(keymap.first)?) * usize::from(keymap.per);
        typed_of_keysym(*keymap.keysyms.get(at)?)
    }

    /// The server's keyboard mapping, every keycode's keysyms.
    fn read_keymap(&self) -> Option<Keymap> {
        let setup = self.conn.setup();
        let (first, last) = (setup.min_keycode, setup.max_keycode);
        let count = last.checked_sub(first)?.checked_add(1)?;
        let reply = self
            .conn
            .get_keyboard_mapping(first, count)
            .ok()?
            .reply()
            .ok()?;
        Some(Keymap {
            first,
            per: reply.keysyms_per_keycode,
            keysyms: reply.keysyms,
        })
    }

    /// What `event` means for our embedders' children, done.
    fn embed(&self, event: &Event) {
        let Some(atoms) = self.xembed.get() else {
            // No embedder was ever opened on this connection.
            return;
        };
        let actions = match event {
            Event::CreateNotify(created) => self
                .embedders
                .borrow_mut()
                .appeared(created.parent, created.window),
            Event::ReparentNotify(moved) => {
                let mut embedders = self.embedders.borrow_mut();
                if embedders.has(moved.parent) {
                    embedders.appeared(moved.parent, moved.window)
                } else {
                    embedders.left(moved.window);
                    Vec::new()
                }
            }
            Event::DestroyNotify(gone) => {
                let mut embedders = self.embedders.borrow_mut();
                if embedders.has(gone.window) {
                    embedders.remove(gone.window);
                } else {
                    embedders.left(gone.window);
                }
                Vec::new()
            }
            Event::PropertyNotify(changed)
                if changed.atom == atoms.info && self.embedders.borrow().knows(changed.window) =>
            {
                let info = if changed.state == State::DELETE {
                    None
                } else {
                    self.info_of(changed.window, atoms.info)
                };
                self.embedders
                    .borrow_mut()
                    .info(changed.window, info.as_deref())
            }
            Event::FocusIn(focus) => {
                self.embedders
                    .borrow_mut()
                    .focus(focus.event, true, u8::from(focus.detail))
            }
            Event::FocusOut(focus) => {
                self.embedders
                    .borrow_mut()
                    .focus(focus.event, false, u8::from(focus.detail))
            }
            _ => Vec::new(),
        };
        self.act(actions, atoms);
    }

    /// Does what the embedders ask, and what that leads to (a child watched is read).
    fn act(&self, actions: Vec<Action>, atoms: XembedAtoms) {
        if actions.is_empty() {
            return;
        }
        let mut queue = VecDeque::from(actions);
        while let Some(action) = queue.pop_front() {
            // A child that is already gone answers with an error event, which `events` drops.
            match action {
                Action::Watch(child) => {
                    let aux = ChangeWindowAttributesAux::new().event_mask(watched_child());
                    let _ = self.conn.change_window_attributes(child, &aux);
                    let info = self.info_of(child, atoms.info);
                    let next = self.embedders.borrow_mut().info(child, info.as_deref());
                    queue.extend(next);
                }
                Action::Send(child, words) => {
                    let message = ClientMessageEvent::new(32, child, atoms.xembed, words);
                    let _ = self
                        .conn
                        .send_event(false, child, EventMask::NO_EVENT, message);
                }
                Action::Map(child) => {
                    let _ = self.conn.map_window(child);
                }
                Action::Unmap(child) => {
                    let _ = self.conn.unmap_window(child);
                }
            }
        }
        let _ = self.conn.flush();
    }

    /// `child`'s `_XEMBED_INFO` words, `None` when it has none (or is gone).
    fn info_of(&self, child: WindowId, atom: u32) -> Option<Vec<u32>> {
        let reply = self
            .conn
            .get_property(false, child, atom, AtomEnum::ANY, 0, 2)
            .ok()?
            .reply()
            .ok()?;
        let words = reply.value32()?.collect();
        Some(words)
    }
}

/// A window's own new size: from a ConfigureNotify of the window itself (`event == window`),
/// never from a child's, which a window that hears its children (SUBSTRUCTURE_NOTIFY) is told of
/// as well. Why it matters: a plugin's interface sized by the plugin would otherwise read as the
/// user resizing our window.
fn resized(configure: &ConfigureNotifyEvent) -> Option<WindowEvent> {
    (configure.event == configure.window).then_some(WindowEvent::Resized {
        window: configure.window,
        width: u32::from(configure.width),
        height: u32::from(configure.height),
    })
}

/// The modifiers whose chords are the plugin's shortcuts: Ctrl, Mod1 (Alt) and Mod4 (the logo
/// key), as X servers map them.
fn chord() -> KeyButMask {
    KeyButMask::CONTROL | KeyButMask::MOD1 | KeyButMask::MOD4
}

/// Whether a key event passes on with `held` (the modifiers and buttons down): a press unless
/// one of the chord's modifiers is held (the plugin's shortcut), a release always, so a note
/// started by a key never outlives it.
fn passes(pressed: bool, held: KeyButMask) -> bool {
    !pressed || !held.intersects(chord())
}

/// What the events of one batch heard on the connection, in the order the server sent them,
/// say happened to our windows; `held` asks for the modifiers held now. Only the keys and the
/// focus are remembered (`keys`); everything the connection itself must do about an event is
/// [`Display::events`]'s.
fn window_events(
    heard: &[Event],
    wm_delete_window: u32,
    keys: &mut KeyState,
    held: &mut dyn FnMut() -> KeyButMask,
) -> Vec<WindowEvent> {
    let mut events = Vec::new();
    for (index, event) in heard.iter().enumerate() {
        let after = &heard[index + 1..];
        match event {
            Event::ClientMessage(message) if message.data.as_data32()[0] == wm_delete_window => {
                events.push(WindowEvent::Closed(message.window));
            }
            Event::ConfigureNotify(configure) => events.extend(resized(configure)),
            Event::KeyPress(core) | Event::KeyRelease(core) => {
                let pressed = matches!(event, Event::KeyPress(_));
                if !keys.twin_passed(core, pressed) {
                    let before = index.checked_sub(1).and_then(|before| heard.get(before));
                    events.extend(key(event, before, after.first()));
                }
            }
            Event::XinputRawKeyPress(raw) => events.extend(keys.raw(raw, true, after, held)),
            Event::XinputRawKeyRelease(raw) => events.extend(keys.raw(raw, false, after, held)),
            Event::FocusIn(focus) => keys.focus_in(focus),
            Event::FocusOut(focus) => {
                events.extend(keys_lost(focus));
                keys.focus_out(focus);
            }
            Event::DestroyNotify(gone) if gone.event == gone.window => keys.gone(gone.window),
            _ => {}
        }
    }
    events
}

/// What the key and focus events heard on a connection leave to remember from one batch to the
/// next.
///
/// Which event a key comes from, one per key: X sends a key's raw event (to the root window,
/// for every key, the master keyboard's) before its key event (to the window it is reported
/// to), both with the server's time of the key. A key event that came to our window is the
/// key (`plugin_focused` false: the plugin's windows took it not); its raw twin, the same
/// keycode, way and time, is then dropped. A raw event with no such twin passes, `plugin_focused`
/// true (a window of the plugin's took the key event), while the X focus is in one of our
/// windows or a window inside it. Its twin may still come in a later batch (the two arrive
/// together in practice), and is dropped then. A key event a client sent (XSendEvent) has no
/// raw event and is always the key.
#[derive(Debug, Default)]
struct KeyState {
    /// The window of ours the keyboard focus is in (it, or a window inside it), as its own
    /// FocusIn and FocusOut events tell, at the place in the events the batch has reached. Why
    /// from the events and not asked of the server: they come in the same order as the keys, so
    /// the focus is the one each key had, where the server's answer would be the focus by the
    /// time the batch is read (a window manager may have moved it meanwhile).
    focus: Option<WindowId>,
    /// The keycodes down, as their raw events tell.
    down: Vec<u8>,
    /// Raw keys passed on whose key event may still come: keycode, time, pressed. The newest
    /// [`KeyState::PASSED`].
    passed: VecDeque<(u32, u32, bool)>,
}

impl KeyState {
    /// How many raw keys passed on are kept for their twin: far more than come between a raw
    /// event and its twin.
    const PASSED: usize = 16;

    /// Our window's FocusIn: the focus is in it after any but the pointer's (detail Pointer) and
    /// one while another client holds the keyboard (mode WhileGrabbed), whose keys are that
    /// client's until its FocusIn with mode Ungrab.
    fn focus_in(&mut self, focus: &FocusInEvent) {
        if focus.detail != NotifyDetail::POINTER && focus.mode != NotifyMode::WHILE_GRABBED {
            self.focus = Some(focus.event);
        }
    }

    /// Our window's FocusOut: the focus left it and everything inside it, unless it only moved
    /// inside or is the pointer's ([`keys_lost`]'s rule).
    fn focus_out(&mut self, focus: &FocusOutEvent) {
        if keys_lost(focus).is_some() && self.focus == Some(focus.event) {
            self.focus = None;
        }
    }

    /// Our window closed or gone.
    fn gone(&mut self, window: WindowId) {
        if self.focus == Some(window) {
            self.focus = None;
        }
    }

    /// Whether `core`, a key event that came to our window (`pressed`, or a release), is the
    /// twin of a raw key passed on already: then it is that key, which came already.
    fn twin_passed(&mut self, core: &KeyPressEvent, pressed: bool) -> bool {
        if sent(core) {
            return false;
        }
        let twin = (u32::from(core.detail), core.time, pressed);
        let Some(at) = self.passed.iter().position(|&passed| passed == twin) else {
            return false;
        };
        self.passed.remove(at);
        true
    }

    /// What a raw key event (`pressed`, or a release) says, with the events heard `after` it in
    /// its batch: a key of the window of ours the focus is in, or nothing. A press of a key
    /// already down is a repeat, and a release with a press of the same key at the same time
    /// right after it (the shape X gives a held key's repeat) is nothing: both as the key
    /// events have it. The source says the server's autorepeat sends no raw events at all
    /// (xkbAccessX.c hands its repeats straight to event processing); both shapes are kept
    /// apart all the same, as no note may start again on a held key.
    fn raw(
        &mut self,
        raw: &xinput::RawKeyPressEvent,
        pressed: bool,
        after: &[Event],
        held: &mut dyn FnMut() -> KeyButMask,
    ) -> Option<WindowEvent> {
        let keycode = u8::try_from(raw.detail).ok()?;
        let was_down = self.down.contains(&keycode);
        if pressed {
            if !was_down {
                self.down.push(keycode);
            }
        } else {
            let next_raw = after.iter().find_map(|event| match event {
                Event::XinputRawKeyPress(next) => Some((true, next)),
                Event::XinputRawKeyRelease(next) => Some((false, next)),
                _ => None,
            });
            if matches!(next_raw, Some((true, next)) if next.detail == raw.detail && next.time == raw.time)
            {
                return None;
            }
            self.down.retain(|&down| down != keycode);
        }
        let window = self.focus?;
        let twin_follows = after.iter().any(|event| match event {
            Event::KeyPress(core) | Event::KeyRelease(core) => {
                matches!(event, Event::KeyPress(_)) == pressed
                    && !sent(core)
                    && u32::from(core.detail) == raw.detail
                    && core.time == raw.time
            }
            _ => false,
        });
        // Only a press asks what is held: a release always passes.
        if twin_follows || (pressed && !passes(true, held())) {
            return None;
        }
        if self.passed.len() == Self::PASSED {
            self.passed.pop_front();
        }
        self.passed.push_back((raw.detail, raw.time, pressed));
        Some(WindowEvent::Key {
            window,
            scancode: raw.detail.saturating_sub(8),
            pressed,
            repeat: pressed && was_down,
            plugin_focused: true,
        })
    }
}

/// Whether a client sent `event` (XSendEvent: the protocol's top bit of its code), rather than
/// the server for a key.
fn sent(event: &KeyPressEvent) -> bool {
    event.response_type & 0x80 != 0
}

/// What a key event of ours (`event`: a KeyPress or KeyRelease) says, with the events heard just
/// `before` and `after` it on the connection. X11 repeats a key held down as a release and a
/// press of the same keycode at the same time, one after the other: that release is nothing, and
/// that press a repeat. A press with Ctrl, Alt or the logo key held is nothing (the plugin's
/// shortcut); a release always is a release.
fn key(event: &Event, before: Option<&Event>, after: Option<&Event>) -> Option<WindowEvent> {
    let paired = |release: &KeyPressEvent, press: &KeyPressEvent| {
        release.detail == press.detail && release.time == press.time && release.event == press.event
    };
    let (key, pressed, repeat) = match event {
        Event::KeyPress(press) => {
            if !passes(true, press.state) {
                return None;
            }
            let repeat =
                matches!(before, Some(Event::KeyRelease(release)) if paired(release, press));
            (press, true, repeat)
        }
        Event::KeyRelease(release) => {
            if matches!(after, Some(Event::KeyPress(press)) if paired(release, press)) {
                return None;
            }
            (release, false, false)
        }
        _ => return None,
    };
    Some(WindowEvent::Key {
        window: key.event,
        scancode: u32::from(key.detail).saturating_sub(8),
        pressed,
        repeat,
        plugin_focused: false,
    })
}

/// A FocusOut of our window that takes the focus out of it and of everything inside it: every
/// one but the focus moving to a window inside ours (Inferior) and the pointer's (Pointer).
fn keys_lost(focus: &FocusOutEvent) -> Option<WindowEvent> {
    let inside = matches!(focus.detail, NotifyDetail::INFERIOR | NotifyDetail::POINTER);
    (!inside).then_some(WindowEvent::KeysLost(focus.event))
}

/// What a plugin window is made with: a black ground, and the events `mask` names reported.
fn window_aux(black: u32, mask: EventMask) -> CreateWindowAux {
    CreateWindowAux::new()
        .background_pixel(black)
        .event_mask(mask)
}

/// One property a window is given as it opens: its name, its type and its value.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Property {
    name: u32,
    kind: u32,
    value: PropertyValue,
}

/// A property's value: 8-bit units or 32-bit words.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PropertyValue {
    Bytes(Vec<u8>),
    Words(Vec<u32>),
}

impl Property {
    fn send(&self, conn: &RustConnection, window: WindowId) -> Result<(), ConnectionError> {
        match &self.value {
            PropertyValue::Bytes(bytes) => {
                conn.change_property8(PropMode::REPLACE, window, self.name, self.kind, bytes)?;
            }
            PropertyValue::Words(words) => {
                conn.change_property32(PropMode::REPLACE, window, self.name, self.kind, words)?;
            }
        }
        Ok(())
    }
}

/// The properties a window titled `title` of `size` is given before it is mapped: its title for
/// the window manager in both encodings, the close button's protocol, for a window that is not
/// `resizable` a least and a most size of `size`, and the X11 class `class` when there is one
/// (instance and class name both, ICCCM 4.1.2.5), last.
fn properties(
    title: &str,
    size: (u16, u16),
    resizable: bool,
    class: Option<&str>,
    atoms: &Atoms,
) -> Vec<Property> {
    let mut list = vec![
        Property {
            name: AtomEnum::WM_NAME.into(),
            kind: AtomEnum::STRING.into(),
            value: PropertyValue::Bytes(title.as_bytes().to_vec()),
        },
        Property {
            name: atoms.net_wm_name,
            kind: atoms.utf8_string,
            value: PropertyValue::Bytes(title.as_bytes().to_vec()),
        },
        Property {
            name: atoms.wm_protocols,
            kind: AtomEnum::ATOM.into(),
            value: PropertyValue::Words(vec![atoms.wm_delete_window]),
        },
    ];
    if !resizable {
        list.push(fixed_hints(size));
    }
    if let Some(class) = class {
        list.push(Property {
            name: AtomEnum::WM_CLASS.into(),
            kind: AtomEnum::STRING.into(),
            value: PropertyValue::Bytes(format!("{class}\0{class}\0").into_bytes()),
        });
    }
    list
}

/// `WM_NORMAL_HINTS` keeping a window at `size`: its least and its most size.
fn fixed_hints((w, h): (u16, u16)) -> Property {
    let mut hints = WmSizeHints::new();
    hints.min_size = Some((i32::from(w), i32::from(h)));
    hints.max_size = Some((i32::from(w), i32::from(h)));
    let words = hints
        .serialize()
        .chunks_exact(4)
        .map(|word| u32::from_ne_bytes([word[0], word[1], word[2], word[3]]))
        .collect();
    Property {
        name: AtomEnum::WM_NORMAL_HINTS.into(),
        kind: AtomEnum::WM_SIZE_HINTS.into(),
        value: PropertyValue::Words(words),
    }
}

/// The `_NET_WM_STATE` client message that adds (`above`) or removes `_NET_WM_STATE_ABOVE` on
/// `window`: action 1 adds, 0 removes, one property, and source 1, a normal application.
fn above_request(window: WindowId, above: bool, state: u32, above_atom: u32) -> ClientMessageEvent {
    ClientMessageEvent::new(32, window, state, [u32::from(above), above_atom, 0, 1, 0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use x11rb::protocol::xproto::ConfigureWindowRequest;

    // The EWMH request for keeping a window above: format 32, the window, _NET_WM_STATE, and
    // [action, the property, none, source 1, 0] with action 1 to add and 0 to remove.
    #[test]
    fn the_above_request_adds_and_removes_the_state() {
        let added = above_request(7, true, 100, 200);
        assert_eq!((added.format, added.window, added.type_), (32, 7, 100));
        assert_eq!(added.data.as_data32(), [1, 200, 0, 1, 0]);
        assert_eq!(
            above_request(7, false, 100, 200).data.as_data32(),
            [0, 200, 0, 1, 0]
        );
    }

    // The requests a plugin window opens with, pinned to the protocol's numbers: a black ground,
    // and reported its own structure (STRUCTURE_NOTIFY, 1 << 17), the keys that reach it
    // (KEY_PRESS 1 << 0, KEY_RELEASE 1 << 1) and its focus (FOCUS_CHANGE, 1 << 21); nothing else
    // of the window's attributes.
    #[test]
    fn a_window_is_made_black_and_hears_its_structure_keys_and_focus() {
        let CreateWindowAux {
            background_pixmap,
            background_pixel,
            border_pixmap,
            border_pixel,
            bit_gravity,
            win_gravity,
            backing_store,
            backing_planes,
            backing_pixel,
            override_redirect,
            save_under,
            event_mask,
            do_not_propogate_mask,
            colormap,
            cursor,
        } = window_aux(7, plugin_window());
        assert_eq!(background_pixel, Some(7));
        assert_eq!(event_mask.map(u32::from), Some(0x0022_0003));
        assert!(background_pixmap.is_none() && border_pixmap.is_none() && border_pixel.is_none());
        assert!(bit_gravity.is_none() && win_gravity.is_none() && backing_store.is_none());
        assert!(backing_planes.is_none() && backing_pixel.is_none() && override_redirect.is_none());
        assert!(save_under.is_none() && do_not_propogate_mask.is_none());
        assert!(colormap.is_none() && cursor.is_none());
    }

    fn configure(event: WindowId, window: WindowId) -> ConfigureNotifyEvent {
        ConfigureNotifyEvent {
            response_type: x11rb::protocol::xproto::CONFIGURE_NOTIFY_EVENT,
            sequence: 0,
            event,
            window,
            above_sibling: 0,
            x: 0,
            y: 0,
            width: 300,
            height: 200,
            border_width: 0,
            override_redirect: false,
        }
    }

    // A window's own ConfigureNotify is its new size; a child's (the plugin's interface, which a
    // window hearing its children is told of) is not ours to act on.
    #[test]
    fn only_a_windows_own_configure_is_its_new_size() {
        assert_eq!(
            resized(&configure(7, 7)),
            Some(WindowEvent::Resized {
                window: 7,
                width: 300,
                height: 200
            })
        );
        assert_eq!(resized(&configure(7, 8)), None, "its child's");
    }

    // An embedder's window adds its children's structure (SUBSTRUCTURE_NOTIFY, 1 << 19) and
    // its focus (FOCUS_CHANGE, 1 << 21) to every plugin window's mask; a child it watches is
    // heard for its properties (PROPERTY_CHANGE, 1 << 22) and structure (gen_p29's pins).
    #[test]
    fn an_embedder_hears_its_children_and_its_focus() {
        let p29: serde_json::Value =
            serde_json::from_str(include_str!("../../../../tests/fixtures/p29_expected.json"))
                .expect("p29_expected.json parses");
        let pin = |name: &str| p29["pins"][name].as_u64().expect("a mask") as u32;
        assert_eq!(u32::from(plugin_window()), pin("plugin_window_mask"));
        assert_eq!(
            u32::from(plugin_window() | embedder()),
            pin("embedder_mask")
        );
        assert_eq!(u32::from(watched_child()), pin("watched_child_mask"));
        let made = window_aux(7, plugin_window() | embedder());
        assert_eq!(made.event_mask.map(u32::from), Some(pin("embedder_mask")));
        assert_eq!(made.background_pixel, Some(7));
        let class = p29["pins"]["class"].as_str().expect("a class");
        let wm_class: Vec<u8> = p29["pins"]["wm_class"]
            .as_array()
            .expect("bytes")
            .iter()
            .map(|byte| byte.as_u64().expect("a byte") as u8)
            .collect();
        let with = properties("Synth", (400, 300), true, Some(class), &atoms());
        assert_eq!(
            with.last().map(|property| &property.value),
            Some(&PropertyValue::Bytes(wm_class))
        );
        let hints = p29["fixed_hints"]["before"].as_array().expect("words");
        let hints: Vec<u32> = hints
            .iter()
            .map(|w| w.as_u64().expect("a word") as u32)
            .collect();
        assert_eq!(fixed_hints((400, 300)).value, PropertyValue::Words(hints));
    }

    // The request every resize of a plugin window sends (a plugin's own, the answer to a user's
    // resize, and that answer asked again as it settles), byte for byte as gen_p29 writes it from
    // the X11 protocol: ConfigureWindow (opcode 12), five words long, the window, value-mask
    // width | height (4 | 8), then the two sides; each side held to 1..32767.
    #[test]
    fn a_resize_asks_for_the_width_and_the_height_alone() {
        let p29: serde_json::Value =
            serde_json::from_str(include_str!("../../../../tests/fixtures/p29_expected.json"))
                .expect("p29_expected.json parses");
        let pin = &p29["clap"]["configure_request"];
        let window = pin["window"].as_u64().expect("a window") as u32;
        for case in pin["cases"].as_array().expect("cases") {
            let side = |index: usize| case["asked"][index].as_u64().expect("a side") as u32;
            let request = ConfigureWindowRequest {
                window,
                value_list: std::borrow::Cow::Owned(size_request(side(0), side(1))),
            };
            let (parts, fds) = request.serialize();
            assert!(fds.is_empty());
            let bytes: Vec<u8> = parts.iter().flat_map(|part| part.iter().copied()).collect();
            let want: Vec<u8> = case["bytes"]
                .as_array()
                .expect("bytes")
                .iter()
                .map(|byte| byte.as_u64().expect("a byte") as u8)
                .collect();
            assert_eq!(bytes, want, "{case}");
        }
    }

    fn atoms() -> Atoms {
        Atoms {
            wm_protocols: 101,
            wm_delete_window: 102,
            net_wm_name: 103,
            utf8_string: 104,
            net_wm_state: 105,
            net_wm_state_above: 106,
        }
    }

    // WM_NAME (39) as STRING (31) and _NET_WM_NAME as UTF8_STRING, both the title's bytes;
    // WM_PROTOCOLS as ATOM (4) holding WM_DELETE_WINDOW. A window that keeps its size adds
    // WM_NORMAL_HINTS (40) as WM_SIZE_HINTS (41): flags PMinSize | PMaxSize (48), the least
    // and the most size its own, every other word 0 (ICCCM 4.1.2.3's 18 words).
    #[test]
    fn a_window_is_titled_closes_by_protocol_and_a_fixed_one_keeps_its_size() {
        let title = "Synth \u{2014} Dexed";
        let named = vec![
            Property {
                name: 39,
                kind: 31,
                value: PropertyValue::Bytes(title.as_bytes().to_vec()),
            },
            Property {
                name: 103,
                kind: 104,
                value: PropertyValue::Bytes(title.as_bytes().to_vec()),
            },
            Property {
                name: 101,
                kind: 4,
                value: PropertyValue::Words(vec![102]),
            },
        ];
        assert_eq!(properties(title, (640, 480), true, None, &atoms()), named);
        let mut fixed = named;
        fixed.push(Property {
            name: 40,
            kind: 41,
            value: PropertyValue::Words(vec![
                48, 0, 0, 0, 0, 640, 480, 640, 480, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ]),
        });
        assert_eq!(properties(title, (640, 480), false, None, &atoms()), fixed);
    }

    // With a class given, WM_CLASS (67) as STRING (31) holds "<id>\0<id>\0", after every
    // property a window had before (those unchanged).
    #[test]
    fn a_window_carries_the_class_given() {
        let with = properties("Synth", (640, 480), false, Some("dawsome-test"), &atoms());
        let without = properties("Synth", (640, 480), false, None, &atoms());
        assert_eq!(with[..without.len()], without[..]);
        assert_eq!(
            with[without.len()..],
            [Property {
                name: 67,
                kind: 31,
                value: PropertyValue::Bytes(b"dawsome-test\0dawsome-test\0".to_vec()),
            }]
        );
    }

    /// A key event of keycode `keycode` on window 7 at `time`, with `state` held.
    fn key_event(pressed: bool, keycode: u8, time: u32, state: KeyButMask) -> Event {
        let event = KeyPressEvent {
            response_type: if pressed {
                x11rb::protocol::xproto::KEY_PRESS_EVENT
            } else {
                x11rb::protocol::xproto::KEY_RELEASE_EVENT
            },
            detail: keycode,
            sequence: 0,
            time,
            root: 1,
            event: 7,
            child: 0,
            root_x: 0,
            root_y: 0,
            event_x: 0,
            event_y: 0,
            state,
            same_screen: true,
        };
        if pressed {
            Event::KeyPress(event)
        } else {
            Event::KeyRelease(event)
        }
    }

    fn passed(scancode: u32, pressed: bool, repeat: bool) -> Option<WindowEvent> {
        Some(WindowEvent::Key {
            window: 7,
            scancode,
            pressed,
            repeat,
            plugin_focused: false,
        })
    }

    // Keycode 38 is evdev's KEY_A (30) plus 8, as every X server on evdev numbers it: its
    // scancode for winit is 30. A press and a release alone pass as they are; X11's repeat of a
    // held key, a release and a press at the same time, is no release and a repeated press; a
    // release and a press at different times are both kept (the key let go of and pressed
    // again). A press with Ctrl (4), Alt (Mod1, 8) or the logo key (Mod4, 64) held is the
    // plugin's shortcut, nothing; with Shift (1) or NumLock (Mod2, 16) it is a press; a release
    // with Ctrl held is still a release.
    #[test]
    fn a_key_passes_on_with_its_repeat_and_a_chords_press_does_not() {
        let none = KeyButMask::from(0u16);
        let (down, up) = (
            key_event(true, 38, 100, none),
            key_event(false, 38, 100, none),
        );
        assert_eq!(key(&down, None, None), passed(30, true, false));
        assert_eq!(key(&up, None, None), passed(30, false, false));
        let again = key_event(true, 38, 140, none);
        let repeated_up = key_event(false, 38, 140, none);
        assert_eq!(
            key(&repeated_up, Some(&down), Some(&again)),
            None,
            "a repeat's release"
        );
        assert_eq!(
            key(&again, Some(&repeated_up), None),
            passed(30, true, true)
        );
        let later = key_event(true, 38, 900, none);
        assert_eq!(key(&up, None, Some(&later)), passed(30, false, false));
        assert_eq!(key(&later, Some(&up), None), passed(30, true, false));
        let other = key_event(true, 39, 100, none);
        assert_eq!(
            key(&up, None, Some(&other)),
            passed(30, false, false),
            "another key"
        );
        for chord in [4u16, 8, 64, 4 | 1] {
            let held = KeyButMask::from(chord);
            assert_eq!(
                key(&key_event(true, 38, 5, held), None, None),
                None,
                "{chord}"
            );
            assert_eq!(
                key(&key_event(false, 38, 6, held), None, None),
                passed(30, false, false),
                "{chord}"
            );
        }
        for plain in [1u16, 16, 2] {
            let held = KeyButMask::from(plain);
            let press = key_event(true, 38, 5, held);
            assert_eq!(key(&press, None, None), passed(30, true, false), "{plain}");
        }
    }

    // Our window's FocusOut takes the keys away unless the focus only moved inside it (detail
    // Inferior, 2) or it is the pointer's (Pointer, 5): Ancestor 0, Virtual 1, Nonlinear 3 and
    // NonlinearVirtual 4 all leave it (the protocol's NotifyDetail numbers).
    #[test]
    fn the_keys_are_lost_unless_the_focus_stays_inside() {
        let out = |detail: u8| FocusOutEvent {
            response_type: x11rb::protocol::xproto::FOCUS_OUT_EVENT,
            detail: NotifyDetail::from(detail),
            sequence: 0,
            event: 7,
            mode: x11rb::protocol::xproto::NotifyMode::NORMAL,
        };
        for detail in [0, 1, 3, 4] {
            assert_eq!(
                keys_lost(&out(detail)),
                Some(WindowEvent::KeysLost(7)),
                "{detail}"
            );
        }
        for detail in [2, 5] {
            assert_eq!(keys_lost(&out(detail)), None, "{detail}");
        }
    }

    // A press passes unless Ctrl (ControlMask 4), Alt (Mod1Mask 8) or the logo key (Mod4Mask
    // 64) is held, alone or with others; Shift (1), Lock (2), NumLock (Mod2Mask 16), Mod3 (32),
    // AltGr as X servers usually map it (Mod5Mask 128) and a mouse button (Button1Mask 256)
    // make no chord. A release passes with anything held (xproto.h's masks).
    #[test]
    fn a_press_passes_unless_a_chords_modifier_is_held() {
        let table: [(u16, bool); 13] = [
            (0, true),
            (1, true),
            (2, true),
            (16, true),
            (32, true),
            (128, true),
            (256, true),
            (4, false),
            (8, false),
            (64, false),
            (4 | 1, false),
            (8 | 16, false),
            (64 | 128, false),
        ];
        for (mask, press) in table {
            assert_eq!(passes(true, KeyButMask::from(mask)), press, "{mask:#x}");
            assert!(
                passes(false, KeyButMask::from(mask)),
                "a release, {mask:#x}"
            );
        }
    }

    /// A raw key event of the master keyboard (device 3) from XTEST's keyboard (9), as this
    /// desk's Xwayland 24.1.14 sent them on 2026-11-10 (the XInput extension's opcode 131).
    fn raw_event(pressed: bool, keycode: u32, time: u32) -> Event {
        let event = xinput::RawKeyPressEvent {
            response_type: x11rb::protocol::xproto::GE_GENERIC_EVENT,
            extension: 131,
            event_type: if pressed {
                xinput::RAW_KEY_PRESS_EVENT
            } else {
                xinput::RAW_KEY_RELEASE_EVENT
            },
            deviceid: 3,
            time,
            detail: keycode,
            sourceid: 9,
            ..Default::default()
        };
        if pressed {
            Event::XinputRawKeyPress(event)
        } else {
            Event::XinputRawKeyRelease(event)
        }
    }

    /// Our window 7's FocusIn (`into`) or FocusOut of `detail` and `mode` (NotifyDetail's and
    /// NotifyMode's numbers).
    fn focus_event(into: bool, detail: u8, mode: u8) -> Event {
        let event = FocusInEvent {
            response_type: if into {
                x11rb::protocol::xproto::FOCUS_IN_EVENT
            } else {
                x11rb::protocol::xproto::FOCUS_OUT_EVENT
            },
            detail: NotifyDetail::from(detail),
            sequence: 0,
            event: 7,
            mode: NotifyMode::from(mode),
        };
        if into {
            Event::FocusIn(event)
        } else {
            Event::FocusOut(event)
        }
    }

    fn none() -> KeyButMask {
        KeyButMask::from(0u16)
    }

    /// What a batch says, nothing held.
    fn batch(keys: &mut KeyState, heard: &[Event]) -> Vec<WindowEvent> {
        window_events(heard, 102, keys, &mut none)
    }

    fn raw_key(scancode: u32, pressed: bool, repeat: bool) -> WindowEvent {
        WindowEvent::Key {
            window: 7,
            scancode,
            pressed,
            repeat,
            plugin_focused: true,
        }
    }

    fn core_key(scancode: u32, pressed: bool) -> WindowEvent {
        WindowEvent::Key {
            window: 7,
            scancode,
            pressed,
            repeat: false,
            plugin_focused: false,
        }
    }

    // The order this desk's X server sent a key in while our window had the focus (2026-11-10,
    // Right, keycode 114, one press and release at time 30519221): the raw press, then the key
    // event to our window, then the same for the release. The GUI test interface's shape (its
    // window takes no keys): each key event came to ours and is the key, plugin_focused false;
    // the raw twins say nothing. The focus leaving (Nonlinear 3) ends the keys. Surge XT's shape
    // (its window took the key events; the focus inside ours, NonlinearVirtual 4): the raw
    // events are the keys, plugin_focused true. Scancode 106 is evdev's KEY_RIGHT.
    #[test]
    fn a_key_comes_once_from_its_key_event_or_else_from_its_raw_event() {
        let time = 30_519_221;
        let mut keys = KeyState::default();
        let heard = [
            focus_event(true, 3, 0),
            raw_event(true, 114, time),
            key_event(true, 114, time, none()),
            raw_event(false, 114, time),
            key_event(false, 114, time, none()),
            focus_event(false, 3, 0),
        ];
        assert_eq!(
            batch(&mut keys, &heard),
            [
                core_key(106, true),
                core_key(106, false),
                WindowEvent::KeysLost(7)
            ]
        );
        let mut keys = KeyState::default();
        let heard = [
            focus_event(true, 4, 0),
            raw_event(true, 114, time),
            raw_event(false, 114, time),
        ];
        assert_eq!(
            batch(&mut keys, &heard),
            [raw_key(106, true, false), raw_key(106, false, false)]
        );
        // The focus elsewhere (no FocusIn of ours): a raw key is another window's.
        let mut keys = KeyState::default();
        let heard = [raw_event(true, 38, 5), raw_event(false, 38, 6)];
        assert_eq!(batch(&mut keys, &heard), []);
    }

    // A raw key and its twin read in two batches: the raw key passes in the first, its twin is
    // dropped in the second; the release, whose twin follows it in its own batch, comes from
    // its key event. One press, one release.
    #[test]
    fn a_twin_in_the_next_batch_is_dropped() {
        let mut keys = KeyState::default();
        let first = [focus_event(true, 3, 0), raw_event(true, 38, 900)];
        assert_eq!(batch(&mut keys, &first), [raw_key(30, true, false)]);
        let second = [
            key_event(true, 38, 900, none()),
            raw_event(false, 38, 950),
            key_event(false, 38, 950, none()),
        ];
        assert_eq!(batch(&mut keys, &second), [core_key(30, false)]);
        // A key event a client sent is never a twin: it has no raw event.
        let mut keys = KeyState::default();
        assert_eq!(
            batch(
                &mut keys,
                &[focus_event(true, 4, 0), raw_event(true, 38, 100)]
            ),
            [raw_key(30, true, false)]
        );
        let Event::KeyPress(mut sent_key) = key_event(true, 38, 100, none()) else {
            unreachable!()
        };
        sent_key.response_type |= 0x80;
        assert_eq!(
            batch(&mut keys, &[Event::KeyPress(sent_key)]),
            [core_key(30, true)],
            "a sent key"
        );
    }

    // Where the focus is, by our window's own events: in it after a FocusIn of Ancestor 0,
    // Virtual 1, Inferior 2, Nonlinear 3 or NonlinearVirtual 4, mode Normal 0 or Ungrab 2; not
    // after one of Pointer 5, or of mode WhileGrabbed 3 (another client holds the keyboard).
    // A FocusOut of Inferior 2 or Pointer 5 leaves it in; any other, or the window gone, takes
    // it out.
    #[test]
    fn the_focus_is_ours_as_our_windows_focus_events_tell() {
        let heard_after = |events: &[Event]| {
            let mut keys = KeyState::default();
            let mut heard = events.to_vec();
            heard.push(raw_event(true, 38, 1));
            batch(&mut keys, &heard)
        };
        for (detail, mode) in [(0, 0), (1, 0), (2, 0), (3, 0), (4, 0), (3, 2)] {
            assert_eq!(
                heard_after(&[focus_event(true, detail, mode)]),
                [raw_key(30, true, false)],
                "FocusIn {detail} {mode}"
            );
        }
        for (detail, mode) in [(5, 0), (3, 3)] {
            assert_eq!(
                heard_after(&[focus_event(true, detail, mode)]),
                [],
                "FocusIn {detail} {mode}"
            );
        }
        for detail in [2, 5] {
            assert_eq!(
                heard_after(&[focus_event(true, 3, 0), focus_event(false, detail, 0)]),
                [raw_key(30, true, false)],
                "FocusOut {detail}"
            );
        }
        for detail in [0, 1, 3, 4] {
            let heard = heard_after(&[focus_event(true, 3, 0), focus_event(false, detail, 0)]);
            assert_eq!(heard, [WindowEvent::KeysLost(7)], "FocusOut {detail}");
        }
        let mut keys = KeyState::default();
        batch(&mut keys, &[focus_event(true, 4, 0)]);
        keys.gone(7);
        assert_eq!(batch(&mut keys, &[raw_event(true, 38, 1)]), [], "gone");
    }

    // With Ctrl held (ControlMask, as QueryPointer tells), a raw press is the plugin's shortcut
    // and its release still passes; the modifiers are asked once in a batch, and only for a
    // press that would pass.
    #[test]
    fn a_raw_chords_press_does_not_pass_and_its_release_does() {
        let mut keys = KeyState::default();
        let mut asked = 0;
        let mut held = || {
            asked += 1;
            KeyButMask::CONTROL
        };
        let heard = [
            focus_event(true, 4, 0),
            raw_event(true, 38, 10),
            raw_event(false, 38, 20),
        ];
        assert_eq!(
            window_events(&heard, 102, &mut keys, &mut held),
            [raw_key(30, false, false)]
        );
        assert_eq!(asked, 1);
    }

    // A raw press of a key already down is a repeat (a device that repeats with presses alone);
    // a release and a press of the same key at the same time (X11's repeat shape) is no
    // release and a repeat; a key let go of and pressed again later is a press.
    #[test]
    fn a_held_raw_key_repeats_and_never_starts_again() {
        let mut keys = KeyState::default();
        let heard = [
            focus_event(true, 4, 0),
            raw_event(true, 38, 100),
            raw_event(true, 38, 600),
            raw_event(false, 38, 640),
            raw_event(true, 38, 640),
            raw_event(false, 38, 700),
            raw_event(true, 38, 900),
        ];
        assert_eq!(
            batch(&mut keys, &heard),
            [
                raw_key(30, true, false),
                raw_key(30, true, true),
                raw_key(30, true, true),
                raw_key(30, false, false),
                raw_key(30, true, false),
            ]
        );
    }
}
