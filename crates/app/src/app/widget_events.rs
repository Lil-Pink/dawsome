//! Events delivered to the main window (or a pop-out window, [`Window::pop_out`]) as iced
//! delivers them, for tests: the view built and laid out at the window's size on the CPU
//! renderer, each event passed down its widgets, the messages they publish handled, then the
//! event passed to the key listener with the status the widgets left (taken or not). Why: iced drops synthetic events sent to a real window, so a test
//! that sends the message a widget would publish never sees what the widget takes and what it
//! lets through (a text box takes a key's press, not its release). Overlays (tooltips, menus)
//! are delivered to only by [`Window::click_overlay`], and where they are laid out can be read
//! ([`Window::overlays`]); the mouse listeners of a drag are not delivered to. The view can be
//! drawn offscreen too ([`Window::snapshot`]), at any width.

use std::future::Future;
use std::path::PathBuf;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use iced::advanced::renderer::Headless;
use iced::advanced::widget::{operation, Operation, Tree};
use iced::advanced::{clipboard, layout, overlay, Layout, Shell};
use iced::keyboard::{self, key, Key, Location, Modifiers};
use iced::widget::Id;
use iced::{
    event, mouse, window, Element, Event, Font, Pixels, Point, Rectangle, Size, Theme, Vector,
};

use super::{key_input, App, Message};

/// The main window's widgets, kept between events as the window keeps them: which text box has
/// the keys, what it has selected.
pub(super) struct Window {
    id: window::Id,
    tree: Tree,
    renderer: iced::Renderer,
    /// The canvases' clock during a click: one tick later each click, so two clicks a test
    /// sends are a double-press however long the machine takes between them.
    clock: std::time::Instant,
    /// The pop-out window these widgets are, and its size; `None`: the main window, at the
    /// app's window size.
    pop_out: Option<(window::Id, Size)>,
}

impl Window {
    pub fn new(app: &App) -> Self {
        let renderer = ready(<iced::Renderer as Headless>::new(
            Font::DEFAULT,
            Pixels(16.0),
            Some("tiny-skia"),
        ))
        .expect("the CPU renderer");
        Self {
            id: window::Id::unique(),
            tree: Tree::new(app.view().as_widget()),
            renderer,
            clock: std::time::Instant::now(),
            pop_out: None,
        }
    }

    /// The widgets of the app's pop-out window `id` (the parameter window, ...), `size` big.
    pub fn pop_out(app: &App, id: window::Id, size: Size) -> Self {
        let mut window = Self::new(app);
        window.id = id;
        window.tree = Tree::new(app.view_window(id).as_widget());
        window.pop_out = Some((id, size));
        window
    }

    /// What the window shows now.
    fn view<'a>(&self, app: &'a App) -> Element<'a, Message> {
        match self.pop_out {
            Some((id, _)) => app.view_window(id),
            None => app.view(),
        }
    }

    /// The window's size.
    fn size(&self, app: &App) -> Size {
        self.pop_out.map_or(app.window_size, |(_, size)| size)
    }

    /// The wheel turned `lines` lines (up the list for a negative count, as a wheel's turn
    /// toward the user is) with the cursor at `at`, handled: the messages the widgets published.
    pub fn wheel(&mut self, app: &mut App, at: Point, lines: f32) -> Vec<Message> {
        let delta = mouse::ScrollDelta::Lines { x: 0.0, y: lines };
        self.mouse(
            app,
            mouse::Event::WheelScrolled { delta },
            mouse::Cursor::Available(at),
        )
    }

    /// Gives the text box `id` the keys with its text selected, as the window does when it
    /// opens one.
    pub fn focus(&mut self, app: &App, id: &'static str) {
        self.operate(app, &mut operation::focusable::focus(Id::new(id)));
        self.operate(app, &mut operation::text_input::select_all(Id::new(id)));
    }

    /// Types `keys` one by one, each a character and the key it lies on, pressed and let go of;
    /// Shift is held for a capital.
    pub fn type_keys(&mut self, app: &mut App, keys: &[(char, key::Code)]) {
        for &(typed, code) in keys {
            let shift = typed.is_uppercase();
            let modifiers = if shift {
                Modifiers::SHIFT
            } else {
                Modifiers::empty()
            };
            if shift {
                self.modifiers(app, modifiers);
            }
            let plain = Key::Character(typed.to_lowercase().collect::<String>().into());
            let shown = Key::Character(typed.to_string().into());
            self.key(app, plain, shown, code, modifiers, Some(typed));
            if shift {
                self.modifiers(app, Modifiers::empty());
            }
        }
    }

    /// Presses and lets go of the named key `named`, which lies at `code`.
    pub fn tap(&mut self, app: &mut App, named: key::Named, code: key::Code) {
        let key = Key::Named(named);
        self.key(app, key.clone(), key, code, Modifiers::empty(), None);
    }

    /// A press and release of the left button at `at`, each handled: the messages the widgets
    /// published.
    pub fn click(&mut self, app: &mut App, at: Point) -> Vec<Message> {
        let cursor = mouse::Cursor::Available(at);
        let mut published = Vec::new();
        self.clock += std::time::Duration::from_millis(1);
        crate::ui::clock::hold(Some(self.clock));
        for event in [mouse::Event::ButtonPressed, mouse::Event::ButtonReleased] {
            let event = Event::Mouse(event(mouse::Button::Left));
            let (messages, _) = self.deliver(app, &event, cursor);
            for message in messages {
                published.push(message.clone());
                let _ = app.update(message);
            }
        }
        crate::ui::clock::hold(None);
        published
    }

    /// A left-button drag from `from` to `to` (the cursor seen half way and at `to`), each
    /// event handled: the messages the widgets published.
    pub fn drag(&mut self, app: &mut App, from: Point, to: Point) -> Vec<Message> {
        let half = Point::new((from.x + to.x) / 2.0, (from.y + to.y) / 2.0);
        let events = [
            (from, mouse::Event::ButtonPressed(mouse::Button::Left)),
            (half, mouse::Event::CursorMoved { position: half }),
            (to, mouse::Event::CursorMoved { position: to }),
            (to, mouse::Event::ButtonReleased(mouse::Button::Left)),
        ];
        let mut published = Vec::new();
        for (at, event) in events {
            let (messages, _) =
                self.deliver(app, &Event::Mouse(event), mouse::Cursor::Available(at));
            for message in messages {
                published.push(message.clone());
                let _ = app.update(message);
            }
        }
        published
    }

    /// A mouse event delivered with the cursor at `cursor`, each message published handled:
    /// the messages.
    pub fn mouse(
        &mut self,
        app: &mut App,
        event: mouse::Event,
        cursor: mouse::Cursor,
    ) -> Vec<Message> {
        let (messages, _) = self.deliver(app, &Event::Mouse(event), cursor);
        for message in &messages {
            let _ = app.update(message.clone());
        }
        messages
    }

    /// `files` dropped from the desktop, the window's cursor `cursor` (unavailable when the
    /// window has not seen it), as the window delivers them: one `FileDropped` a file, each
    /// passed down the widgets, then to the window's listener with the status they left (a drop
    /// no widget took is the window's), every message handled. The messages, in order.
    pub fn drop_files(
        &mut self,
        app: &mut App,
        files: &[PathBuf],
        cursor: mouse::Cursor,
    ) -> Vec<Message> {
        let mut published = Vec::new();
        for file in files {
            let event = Event::Window(window::Event::FileDropped(file.clone()));
            let (messages, status) = self.deliver(app, &event, cursor);
            let listened = super::rescale_message(event, status, self.id);
            for message in messages.into_iter().chain(listened) {
                published.push(message.clone());
                let _ = app.update(message);
            }
        }
        published
    }

    /// The named key `named` at `code` pressed and let go of with nothing held, through the
    /// widgets first.
    pub fn press_named(&mut self, app: &mut App, named: key::Named, code: key::Code) {
        self.tap(app, named, code);
    }

    /// The modifier keys held from now on, as the window reports them.
    pub fn hold(&mut self, app: &mut App, modifiers: Modifiers) {
        self.modifiers(app, modifiers);
    }

    fn modifiers(&mut self, app: &mut App, modifiers: Modifiers) {
        self.send(
            app,
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)),
        );
    }

    /// The key at `code` pressed and let go of with `modifiers` held: `plain` is what it types
    /// without them, `shown` with them, `text` what it writes.
    fn key(
        &mut self,
        app: &mut App,
        plain: Key,
        shown: Key,
        code: key::Code,
        modifiers: Modifiers,
        text: Option<char>,
    ) {
        let physical_key = key::Physical::Code(code);
        let location = Location::Standard;
        self.send(
            app,
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: plain.clone(),
                modified_key: shown.clone(),
                physical_key,
                location,
                modifiers,
                text: text.map(|typed| typed.to_string().into()),
                repeat: false,
            }),
        );
        self.send(
            app,
            Event::Keyboard(keyboard::Event::KeyReleased {
                key: plain,
                modified_key: shown,
                physical_key,
                location,
                modifiers,
            }),
        );
    }

    /// `event` as the window delivers it: the widgets' messages first, then the key listener's,
    /// told whether a widget took it.
    fn send(&mut self, app: &mut App, event: Event) {
        let (published, status) = self.deliver(app, &event, mouse::Cursor::Unavailable);
        for message in published {
            let _ = app.update(message);
        }
        if let Some(message) = key_input::key_event(event, status, self.id) {
            let _ = app.update(message);
        }
    }

    /// `event` passed down the widgets of the view `app` shows now, the cursor at `cursor`: the
    /// messages they published, and whether one took it.
    fn deliver(
        &mut self,
        app: &App,
        event: &Event,
        cursor: mouse::Cursor,
    ) -> (Vec<Message>, event::Status) {
        let mut view = self.view(app);
        self.tree.diff(view.as_widget());
        let size = self.size(app);
        let node = view.as_widget_mut().layout(
            &mut self.tree,
            &self.renderer,
            &layout::Limits::new(Size::ZERO, size),
        );
        let mut published = Vec::new();
        let mut shell = Shell::new(&mut published);
        view.as_widget_mut().update(
            &mut self.tree,
            event,
            Layout::new(&node),
            cursor,
            &self.renderer,
            &mut clipboard::Null,
            &mut shell,
            &Rectangle::with_size(size),
        );
        let status = shell.event_status();
        drop(shell);
        (published, status)
    }

    /// Where the window's overlays (the menus over its panels) are laid out now, in window
    /// coordinates, one rectangle each. Why: a menu on iced's overlay layer is placed by its
    /// own layout, which the view's layout does not hold.
    pub fn overlays(&mut self, app: &App) -> Vec<Rectangle> {
        let window = Rectangle::with_size(self.size(app));
        self.on_overlays(app, |_, laid_out, _| {
            let mut found = Vec::new();
            placed(laid_out, window, &mut found);
            found
        })
        .unwrap_or_default()
    }

    /// A press and release of the left button at `at` passed to the window's overlays (the
    /// menus over its panels), as iced passes them a click before the widgets under them: the
    /// messages they published, not handled.
    pub fn click_overlay(&mut self, app: &App, at: Point) -> Vec<Message> {
        let cursor = mouse::Cursor::Available(at);
        let mut published = Vec::new();
        for event in [mouse::Event::ButtonPressed, mouse::Event::ButtonReleased] {
            let event = Event::Mouse(event(mouse::Button::Left));
            self.on_overlays(app, |overlay, laid_out, renderer| {
                let mut shell = Shell::new(&mut published);
                overlay.update(
                    &event,
                    Layout::new(laid_out),
                    cursor,
                    renderer,
                    &mut clipboard::Null,
                    &mut shell,
                );
            });
        }
        published
    }

    /// `act` on the overlays of the view `app` shows now, laid out at the window's size (with
    /// their layout and the renderer); `None` when it shows none.
    fn on_overlays<T>(
        &mut self,
        app: &App,
        act: impl FnOnce(
            &mut dyn overlay::Overlay<Message, Theme, iced::Renderer>,
            &layout::Node,
            &iced::Renderer,
        ) -> T,
    ) -> Option<T> {
        let mut view = self.view(app);
        self.tree.diff(view.as_widget());
        let size = self.size(app);
        let node = view.as_widget_mut().layout(
            &mut self.tree,
            &self.renderer,
            &layout::Limits::new(Size::ZERO, size),
        );
        let mut overlay = view.as_widget_mut().overlay(
            &mut self.tree,
            Layout::new(&node),
            &self.renderer,
            &Rectangle::with_size(size),
            Vector::ZERO,
        )?;
        let overlay = overlay.as_overlay_mut();
        let laid_out = overlay.layout(&self.renderer, size);
        Some(act(overlay, &laid_out, &self.renderer))
    }

    /// The window as the CPU renderer draws it now, at its size and a scale factor of 1, as a
    /// screenshot holds it (the overlays left out). Why: a window wider than any monitor here
    /// (the toolbar at 2560 or 4510 px) cannot be shot by the app, which fits its window to
    /// its monitor; the same view drawn offscreen can.
    pub fn snapshot(&mut self, app: &App) -> window::Screenshot {
        use iced::advanced::renderer::Style;
        let mut view = self.view(app);
        self.tree.diff(view.as_widget());
        let size = self.size(app);
        let node = view.as_widget_mut().layout(
            &mut self.tree,
            &self.renderer,
            &layout::Limits::new(Size::ZERO, size),
        );
        let theme = Theme::Dark;
        let palette = theme.palette();
        view.as_widget().draw(
            &self.tree,
            &mut self.renderer,
            &theme,
            &Style {
                text_color: palette.text,
            },
            Layout::new(&node),
            mouse::Cursor::Unavailable,
            &Rectangle::with_size(size),
        );
        let physical = Size::new(size.width.ceil() as u32, size.height.ceil() as u32);
        let rgba = self.renderer.screenshot(physical, 1.0, palette.background);
        window::Screenshot::new(rgba, physical, 1.0)
    }

    fn operate(&mut self, app: &App, operation: &mut dyn Operation) {
        let mut view = self.view(app);
        self.tree.diff(view.as_widget());
        let size = self.size(app);
        let node = view.as_widget_mut().layout(
            &mut self.tree,
            &self.renderer,
            &layout::Limits::new(Size::ZERO, size),
        );
        view.as_widget_mut().operate(
            &mut self.tree,
            Layout::new(&node),
            &self.renderer,
            operation,
        );
    }
}

/// The overlays laid out in `node`, into `found`: iced groups a container's overlays in a node
/// as big as the window, so a node of any other size is one overlay.
fn placed(node: &layout::Node, window: Rectangle, found: &mut Vec<Rectangle>) {
    if node.bounds() == window && !node.children().is_empty() {
        for child in node.children() {
            placed(child, window, found);
        }
    } else {
        found.push(node.bounds());
    }
}

/// The value of `future`, which needs no waiting (the CPU renderer is made at once).
fn ready<T>(future: impl Future<Output = T>) -> T {
    match pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("the renderer waited"),
    }
}
