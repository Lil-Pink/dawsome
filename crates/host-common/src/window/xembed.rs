//! The embedder's side of XEmbed 0.5 (freedesktop.org, "Message Specifications" and "Embedding
//! life cycle"), as plain state over window ids: which children of our embedding windows are
//! clients, whether each is mapped, whether our window has the focus, and the requests each event
//! calls for. The X connection's side (reading the events, sending the requests) is `x11.rs`'s;
//! nothing here touches the server, so the rules are tested without one.
//!
//! A child that appears in an embedding window (created there or reparented into it) is watched
//! and its `_XEMBED_INFO` read. With one it is embedded: told so (EMBEDDED_NOTIFY, our window,
//! version 0), mapped while its MAPPED flag is set and unmapped when it is cleared, and, when our
//! window has the focus, activated and focused. A child without `_XEMBED_INFO` is left alone (it
//! maps itself, as a CLAP plugin's interface does). A child is never sized: under XEmbed that is
//! its own business, and the format sizes its view.

use std::collections::BTreeMap;

/// XEmbed's message opcodes the embedder sends.
pub(super) const EMBEDDED_NOTIFY: u32 = 0;
pub(super) const WINDOW_ACTIVATE: u32 = 1;
pub(super) const WINDOW_DEACTIVATE: u32 = 2;
pub(super) const FOCUS_IN: u32 = 4;
pub(super) const FOCUS_OUT: u32 = 5;
/// FOCUS_IN's detail: the focus stays where the client had it.
pub(super) const FOCUS_CURRENT: u32 = 0;
/// `_XEMBED_INFO`'s flag asking to be mapped.
pub(super) const MAPPED: u32 = 1;
/// The X11 focus details that are no change of our window's own focus: the focus moving to a
/// window inside ours (Inferior, 2), and the pointer's window (Pointer, 5).
const NOTIFY_INFERIOR: u8 = 2;
const NOTIFY_POINTER: u8 = 5;

/// An `_XEMBED` message's five words: the time (CurrentTime, 0), the opcode, its detail and its
/// two data words.
pub(super) fn message(opcode: u32, detail: u32, data1: u32) -> [u32; 5] {
    [0, opcode, detail, data1, 0]
}

/// What the connection does for the embedder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    /// Hear the child's property changes and its structure, then read its `_XEMBED_INFO` and
    /// hand it to [`Embedders::info`].
    Watch(u32),
    /// Send the child an `_XEMBED` message of these words.
    Send(u32, [u32; 5]),
    Map(u32),
    Unmap(u32),
}

/// One child of an embedding window.
#[derive(Debug, Clone, Copy, Default)]
struct Client {
    /// It has `_XEMBED_INFO` and was told it is embedded.
    embedded: bool,
    /// We mapped it.
    mapped: bool,
}

#[derive(Debug, Default)]
struct Embedder {
    /// Our window has the keyboard focus.
    focused: bool,
    clients: BTreeMap<u32, Client>,
}

/// Every embedding window of one connection.
#[derive(Debug, Default)]
pub(super) struct Embedders {
    windows: BTreeMap<u32, Embedder>,
}

impl Embedders {
    /// `window` embeds its children from now.
    pub(super) fn add(&mut self, window: u32) {
        self.windows.entry(window).or_default();
    }

    /// `window` is gone, and its clients with it.
    pub(super) fn remove(&mut self, window: u32) {
        self.windows.remove(&window);
    }

    pub(super) fn has(&self, window: u32) -> bool {
        self.windows.contains_key(&window)
    }

    /// Whether `child` is a child of one of our embedding windows.
    pub(super) fn knows(&self, child: u32) -> bool {
        self.windows
            .values()
            .any(|embedder| embedder.clients.contains_key(&child))
    }

    /// `child` appeared in `parent` (created there, or reparented into it): watched, once.
    pub(super) fn appeared(&mut self, parent: u32, child: u32) -> Vec<Action> {
        let Some(embedder) = self.windows.get_mut(&parent) else {
            return Vec::new();
        };
        if embedder.clients.contains_key(&child) {
            return Vec::new();
        }
        embedder.clients.insert(child, Client::default());
        vec![Action::Watch(child)]
    }

    /// `child` is gone, or was reparented out of our window.
    pub(super) fn left(&mut self, child: u32) {
        for embedder in self.windows.values_mut() {
            embedder.clients.remove(&child);
        }
    }

    /// `child`'s `_XEMBED_INFO` as read (`None`: it has none): the first one embeds it; a later
    /// one maps or unmaps it as its MAPPED flag says.
    pub(super) fn info(&mut self, child: u32, info: Option<&[u32]>) -> Vec<Action> {
        let Some(flags) = info.and_then(|info| info.get(1).copied()) else {
            return Vec::new();
        };
        let Some((&window, embedder)) = self
            .windows
            .iter_mut()
            .find(|(_, embedder)| embedder.clients.contains_key(&child))
        else {
            return Vec::new();
        };
        let focused = embedder.focused;
        let Some(client) = embedder.clients.get_mut(&child) else {
            return Vec::new();
        };
        let mut actions = Vec::new();
        let newly = !client.embedded;
        if newly {
            client.embedded = true;
            actions.push(Action::Send(child, message(EMBEDDED_NOTIFY, 0, window)));
        }
        let wanted = flags & MAPPED != 0;
        if wanted != client.mapped {
            client.mapped = wanted;
            actions.push(if wanted {
                Action::Map(child)
            } else {
                Action::Unmap(child)
            });
        }
        if newly && focused {
            actions.push(Action::Send(child, message(WINDOW_ACTIVATE, 0, 0)));
            actions.push(Action::Send(child, message(FOCUS_IN, FOCUS_CURRENT, 0)));
        }
        actions
    }

    /// Our `window` gained (`focused`) or lost the focus, X11's focus `detail` saying how: every
    /// embedded client is activated and focused, or unfocused and deactivated. The focus moving
    /// inside our window, or to the pointer's window, is no change.
    pub(super) fn focus(&mut self, window: u32, focused: bool, detail: u8) -> Vec<Action> {
        if detail == NOTIFY_INFERIOR || detail == NOTIFY_POINTER {
            return Vec::new();
        }
        let Some(embedder) = self.windows.get_mut(&window) else {
            return Vec::new();
        };
        embedder.focused = focused;
        let words = if focused {
            [
                message(WINDOW_ACTIVATE, 0, 0),
                message(FOCUS_IN, FOCUS_CURRENT, 0),
            ]
        } else {
            [message(FOCUS_OUT, 0, 0), message(WINDOW_DEACTIVATE, 0, 0)]
        };
        embedder
            .clients
            .iter()
            .filter(|(_, client)| client.embedded)
            .flat_map(|(&child, _)| words.map(|words| Action::Send(child, words)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// gen_p29_expected.py's answers.
    fn p29() -> serde_json::Value {
        serde_json::from_str(include_str!("../../../../tests/fixtures/p29_expected.json"))
            .expect("p29_expected.json parses")
    }

    fn words(value: &serde_json::Value) -> [u32; 5] {
        let list: Vec<u32> = value
            .as_array()
            .expect("words")
            .iter()
            .map(|word| word.as_u64().expect("a word") as u32)
            .collect();
        list.try_into().expect("five words")
    }

    // W0. The XEmbed messages' words: time 0, the opcode, the detail, data1, data2 (EMBEDDED_NOTIFY
    // names our window, version 0; FOCUS_IN's detail is CURRENT).
    #[test]
    fn the_xembed_words_are_the_protocols() {
        let p29 = p29();
        let xembed = &p29["xembed"];
        let ours = xembed["our_window"].as_u64().expect("an id") as u32;
        assert_eq!(
            message(EMBEDDED_NOTIFY, 0, ours),
            words(&xembed["embedded_notify"])
        );
        assert_eq!(
            message(WINDOW_ACTIVATE, 0, 0),
            words(&xembed["window_activate"])
        );
        assert_eq!(
            message(WINDOW_DEACTIVATE, 0, 0),
            words(&xembed["window_deactivate"])
        );
        assert_eq!(
            message(FOCUS_IN, FOCUS_CURRENT, 0),
            words(&xembed["focus_in"])
        );
        assert_eq!(message(FOCUS_OUT, 0, 0), words(&xembed["focus_out"]));
    }

    const OURS: u32 = 100;
    const CHILD: u32 = 200;

    // A child created in our window is watched once; with MAPPED in its info it is told it is
    // embedded and mapped; a later info clearing the flag unmaps it, setting it maps it again; a
    // child with no info is left alone; one that leaves is forgotten.
    #[test]
    fn a_child_with_xembed_info_is_embedded_and_follows_its_flag() {
        let mut embedders = Embedders::default();
        assert!(
            embedders.appeared(OURS, CHILD).is_empty(),
            "not an embedder yet"
        );
        embedders.add(OURS);
        assert_eq!(embedders.appeared(OURS, CHILD), [Action::Watch(CHILD)]);
        assert!(embedders.appeared(OURS, CHILD).is_empty(), "watched once");
        assert!(
            embedders.info(CHILD, None).is_empty(),
            "no info: left alone"
        );
        assert_eq!(
            embedders.info(CHILD, Some(&[0, MAPPED])),
            [
                Action::Send(CHILD, [0, EMBEDDED_NOTIFY, 0, OURS, 0]),
                Action::Map(CHILD)
            ]
        );
        assert!(
            embedders.info(CHILD, Some(&[0, MAPPED])).is_empty(),
            "nothing new"
        );
        assert_eq!(embedders.info(CHILD, Some(&[0, 0])), [Action::Unmap(CHILD)]);
        assert_eq!(
            embedders.info(CHILD, Some(&[0, MAPPED])),
            [Action::Map(CHILD)]
        );
        embedders.left(CHILD);
        assert!(!embedders.knows(CHILD));
        assert!(embedders.info(CHILD, Some(&[0, 0])).is_empty(), "forgotten");
    }

    // An info read before the child set it (none) and the one its PropertyNotify brings later
    // embed it then; a child unmapped at first is embedded and not mapped.
    #[test]
    fn a_child_read_too_early_is_embedded_by_its_later_info() {
        let mut embedders = Embedders::default();
        embedders.add(OURS);
        embedders.appeared(OURS, CHILD);
        assert!(embedders.info(CHILD, Some(&[])).is_empty(), "no flags word");
        assert_eq!(
            embedders.info(CHILD, Some(&[0, 0])),
            [Action::Send(CHILD, [0, EMBEDDED_NOTIFY, 0, OURS, 0])]
        );
    }

    // W7's rules: our window's focus in (detail Nonlinear, 3) activates and focuses every embedded
    // client, its focus out unfocuses and deactivates them; the focus moving inside our window
    // (Inferior) or to the pointer's (Pointer) is nothing; a child embedded while ours has the
    // focus is activated and focused after it is told it is embedded and mapped.
    #[test]
    fn our_focus_reaches_every_embedded_client() {
        let p29 = p29();
        let focus = &p29["focus"];
        let detail = |case: &str| focus[case]["detail"].as_u64().expect("detail") as u8;
        let mut embedders = Embedders::default();
        embedders.add(OURS);
        embedders.appeared(OURS, CHILD);
        embedders.info(CHILD, Some(&[0, MAPPED]));
        embedders.appeared(OURS, CHILD + 1);
        let activate = [0, WINDOW_ACTIVATE, 0, 0, 0];
        let focus_in = [0, FOCUS_IN, 0, 0, 0];
        assert_eq!(
            embedders.focus(OURS, true, detail("in")),
            [Action::Send(CHILD, activate), Action::Send(CHILD, focus_in)],
            "the child without info gets nothing"
        );
        assert!(embedders
            .focus(OURS, false, detail("out_inferior"))
            .is_empty());
        assert!(embedders.focus(OURS, true, detail("in_pointer")).is_empty());
        assert!(
            embedders.focus(OURS + 1, true, detail("in")).is_empty(),
            "not ours"
        );
        assert_eq!(
            embedders.info(CHILD + 1, Some(&[0, MAPPED])),
            [
                Action::Send(CHILD + 1, [0, EMBEDDED_NOTIFY, 0, OURS, 0]),
                Action::Map(CHILD + 1),
                Action::Send(CHILD + 1, activate),
                Action::Send(CHILD + 1, focus_in),
            ]
        );
        assert_eq!(
            embedders.focus(OURS, false, detail("out")),
            [
                Action::Send(CHILD, [0, FOCUS_OUT, 0, 0, 0]),
                Action::Send(CHILD, [0, WINDOW_DEACTIVATE, 0, 0, 0]),
                Action::Send(CHILD + 1, [0, FOCUS_OUT, 0, 0, 0]),
                Action::Send(CHILD + 1, [0, WINDOW_DEACTIVATE, 0, 0, 0]),
            ]
        );
        embedders.remove(OURS);
        assert!(!embedders.has(OURS) && !embedders.knows(CHILD));
    }
}
