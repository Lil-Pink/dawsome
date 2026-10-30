//! Plugin windows on Linux: top-level X11 windows of ours, on a connection of their own (through
//! XWayland on a Wayland desktop). A window opened as an embedder ([`Display::open_embedder`],
//! VST3's) also hears its children and its focus, and is the XEmbed embedder of a child that asks
//! to be one (`xembed.rs` holds the rules); a plain one (CLAP's) hears only itself, as before.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::sync::Arc;

use x11rb::connection::Connection;
use x11rb::errors::ConnectionError;
use x11rb::properties::WmSizeHints;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ClientMessageEvent, ConfigureNotifyEvent,
    ConfigureWindowAux, ConnectionExt as _, CreateWindowAux, EventMask, PropMode,
    Property as State, WindowClass,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::x11_utils::Serialize as _;

use super::xembed::{Action, Embedders};
use super::{clamp_side, class, WindowEvent};

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
/// client message brings unasked).
const PLUGIN_WINDOW: EventMask = EventMask::STRUCTURE_NOTIFY;
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
    root: WindowId,
    black: u32,
    atoms: Atoms,
    xembed: Cell<Option<XembedAtoms>>,
    embedders: RefCell<Embedders>,
}

/// One window of ours, for code that holds no display (a plugin's frame, called by the plugin,
/// and the view it closes as it goes): its size and its closing. It shares the display's
/// connection, which x11rb locks for itself, so it can be held anywhere.
#[derive(Debug, Clone)]
pub struct WindowHandle {
    conn: Arc<RustConnection>,
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
        let _ = self.conn.flush();
    }
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
        Ok(Self {
            conn: Arc::new(conn),
            root,
            black,
            atoms,
            xembed: Cell::new(None),
            embedders: RefCell::new(Embedders::default()),
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
        self.try_open(title, width, height, resizable, PLUGIN_WINDOW)
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
            self.try_open(title, width, height, resizable, PLUGIN_WINDOW | embedder())
        });
        let window = opened.map_err(|error| format!("cannot open the plugin window: {error}"))?;
        self.embedders.borrow_mut().add(window);
        Ok(window)
    }

    /// `window`, for holding apart from the display.
    pub fn handle(&self, window: WindowId) -> WindowHandle {
        WindowHandle {
            conn: Arc::clone(&self.conn),
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
            self.root,
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
        self.conn.flush()?;
        Ok(window)
    }

    /// Gives `window` the size a plugin asked for.
    pub fn resize(&self, window: WindowId, width: u32, height: u32) {
        configure_size(&self.conn, window, width, height);
    }

    pub fn close(&self, window: WindowId) {
        let _ = self.conn.destroy_window(window);
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
        let _ = self.conn.send_event(false, self.root, mask, event);
        let _ = self.conn.flush();
    }

    /// What happened to our windows since the last call.
    pub fn events(&self) -> Vec<WindowEvent> {
        let mut events = Vec::new();
        while let Ok(Some(event)) = self.conn.poll_for_event() {
            match event {
                Event::ClientMessage(message)
                    if message.data.as_data32()[0] == self.atoms.wm_delete_window =>
                {
                    events.push(WindowEvent::Closed(message.window));
                }
                Event::ConfigureNotify(configure) => events.extend(resized(&configure)),
                event => self.embed(&event),
            }
        }
        events
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

    // The requests a plugin window opens with, pinned to the protocol's numbers (CLAP's windows
    // as they were before the window code moved to host-common): a black ground and its own
    // structure reported (STRUCTURE_NOTIFY, 1 << 17), nothing else of the window's attributes.
    #[test]
    fn a_window_is_made_black_and_hears_its_own_structure() {
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
        } = window_aux(7, PLUGIN_WINDOW);
        assert_eq!(background_pixel, Some(7));
        assert_eq!(event_mask.map(u32::from), Some(0x0002_0000));
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

    // W0. An embedder's window adds its children's structure (SUBSTRUCTURE_NOTIFY, 1 << 19) and
    // its focus (FOCUS_CHANGE, 1 << 21) to every plugin window's mask; a child it watches is
    // heard for its properties (PROPERTY_CHANGE, 1 << 22) and structure (gen_p29's pins).
    #[test]
    fn an_embedder_hears_its_children_and_its_focus() {
        let p29: serde_json::Value =
            serde_json::from_str(include_str!("../../../../tests/fixtures/p29_expected.json"))
                .expect("p29_expected.json parses");
        let pin = |name: &str| p29["pins"][name].as_u64().expect("a mask") as u32;
        assert_eq!(u32::from(PLUGIN_WINDOW), pin("plugin_window_mask"));
        assert_eq!(u32::from(PLUGIN_WINDOW | embedder()), pin("embedder_mask"));
        assert_eq!(u32::from(watched_child()), pin("watched_child_mask"));
        let made = window_aux(7, PLUGIN_WINDOW | embedder());
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
}
