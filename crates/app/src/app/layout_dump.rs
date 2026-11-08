//! `--dump-layout`: where a window's text and boxes lie as its screenshot is taken, in the
//! window's logical pixels, as JSON for `tests/scripts/layout_bounds.py`, which fails on text
//! wider than its box, two text runs overlapping, or a box outside the window. Why a check of the
//! layout and not of pixels: at a zoom other than 100 % no pixel is pinned, but each of those
//! faults is one at any zoom.
//!
//! The window's view is laid out again here on the CPU renderer at the shot's logical size, as
//! the window lays it out, and walked with the toolkit's own operation (each widget reports its
//! parts in its own layout, so nothing is guessed about how a widget's tree and layout pair up):
//! - a text widget is a text run: its box (its laid-out bounds) and its measured width (its
//!   paragraph's widest line, which the box does not bound), read from the paragraph its tree
//!   keeps for that text;
//! - every container the toolkit reports (a container, a row, a column, a button, a stack, a
//!   tooltip's anchor) is a box: its rect, its path in the walk, and whether it lies in a
//!   scrollable, which may hold it past the window to scroll it into view.
//!
//! What it cannot see, said here so nobody reads more into a pass: a canvas's own text and its
//! hit regions (the rack's rows, the mixer's strips, the Settings page) are drawn and tested by
//! the canvas itself, and the layout modules' tests and the pixel probes hold those; the toolkit
//! reports no widget's kind, so two boxes overlapping is no fault here (a stack's layers overlap
//! by design): two text runs overlapping is; and a tooltip or a menu not open at the shot.

use std::future::Future;
use std::path::Path;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use iced::advanced::renderer::Headless;
use iced::advanced::widget::{operation::Scrollable, tree, Id, Operation, Tree};
use iced::advanced::{layout, Layout};
use iced::{Element, Pixels, Rectangle, Renderer, Size, Vector};
use serde_json::{json, Value};

use super::Message;
use crate::ui::fonts;
use crate::zoom::Zoom;

/// The paragraph a text widget keeps in its tree.
type TextState =
    iced::advanced::widget::text::State<<Renderer as iced::advanced::text::Renderer>::Paragraph>;

/// The dump of `view`, laid out `size` big (logical pixels) at `zoom`, its shot taken at
/// `scale_factor` device pixels per logical pixel.
pub(super) fn dump(view: Element<'_, Message>, size: Size, zoom: Zoom, scale_factor: f32) -> Value {
    fonts::load();
    let renderer = ready(<Renderer as Headless>::new(
        fonts::UI,
        Pixels(16.0),
        Some("tiny-skia"),
    ))
    .expect("the CPU renderer");
    let mut view = view;
    let mut tree = Tree::new(view.as_widget());
    let node =
        view.as_widget_mut()
            .layout(&mut tree, &renderer, &layout::Limits::new(Size::ZERO, size));
    let mut paragraphs = Vec::new();
    collect_paragraphs(&tree, &mut paragraphs);
    let mut walk = Walk::default();
    view.as_widget_mut()
        .operate(&mut tree, Layout::new(&node), &renderer, &mut walk);
    let texts: Vec<Value> = walk
        .texts
        .into_iter()
        .map(|run| {
            let measured = measured(&mut paragraphs, &run.text, run.bounds);
            let mut entry = rect(run.bounds);
            entry["path"] = json!(run.path);
            entry["text"] = json!(run.text);
            entry["measured"] = json!(measured);
            entry["scrolled"] = json!(run.scrolled);
            entry
        })
        .collect();
    json!({
        "window": {"width": size.width, "height": size.height},
        "zoom": zoom.percent(),
        "scale_factor": scale_factor,
        "texts": texts,
        "boxes": walk.boxes,
    })
}

/// Writes [`dump`] to `path`, pretty-printed.
pub(super) fn write(
    path: &Path,
    view: Element<'_, Message>,
    size: Size,
    zoom: Zoom,
    scale_factor: f32,
) -> std::io::Result<()> {
    let value = dump(view, size, zoom, scale_factor);
    let text = serde_json::to_string_pretty(&value).map_err(std::io::Error::other)?;
    std::fs::write(path, text + "\n")
}

fn rect(bounds: Rectangle) -> Value {
    json!({"x": bounds.x, "y": bounds.y, "width": bounds.width, "height": bounds.height})
}

/// A text widget the walk met.
struct Run {
    path: String,
    text: String,
    bounds: Rectangle,
    scrolled: bool,
}

/// The walk over the laid-out view: each text run and box with its path (the place among its
/// siblings at each depth) and whether a scrollable holds it.
#[derive(Default)]
struct Walk {
    texts: Vec<Run>,
    boxes: Vec<Value>,
    /// The place at each depth so far, and whether that depth is inside a scrollable.
    levels: Vec<(usize, bool)>,
    /// The widget just met was a scrollable: what it traverses next is its content.
    scrollable_next: bool,
}

impl Walk {
    /// The next widget at this depth: its path, and whether it lies in a scrollable.
    fn next(&mut self) -> (String, bool) {
        if self.levels.is_empty() {
            self.levels.push((0, false));
        }
        let scrolled = self.levels.last().is_some_and(|&(_, scrolled)| scrolled);
        if let Some((count, _)) = self.levels.last_mut() {
            *count += 1;
        }
        let path = self
            .levels
            .iter()
            .map(|(count, _)| (count - 1).to_string())
            .collect::<Vec<_>>()
            .join(".");
        (path, scrolled)
    }
}

impl Operation for Walk {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        let inside = self.levels.last().is_some_and(|&(_, scrolled)| scrolled);
        let scrolled = inside || std::mem::take(&mut self.scrollable_next);
        self.levels.push((0, scrolled));
        operate(self);
        self.levels.pop();
    }

    fn container(&mut self, _id: Option<&Id>, bounds: Rectangle) {
        self.scrollable_next = false;
        let (path, scrolled) = self.next();
        let mut entry = rect(bounds);
        entry["path"] = json!(path);
        entry["scrolled"] = json!(scrolled);
        self.boxes.push(entry);
    }

    fn scrollable(
        &mut self,
        _id: Option<&Id>,
        _bounds: Rectangle,
        _content_bounds: Rectangle,
        _translation: Vector,
        _state: &mut dyn Scrollable,
    ) {
        let _ = self.next();
        self.scrollable_next = true;
    }

    fn text(&mut self, _id: Option<&Id>, bounds: Rectangle, text: &str) {
        self.scrollable_next = false;
        let (path, scrolled) = self.next();
        if !text.trim().is_empty() {
            self.texts.push(Run {
                path,
                text: text.to_owned(),
                bounds,
                scrolled,
            });
        }
    }
}

/// A text widget's paragraph as its tree keeps it after the layout: its text and its widest
/// line.
struct Paragraph {
    text: String,
    width: f32,
    used: bool,
}

/// Every text widget's paragraph in `tree`, in the tree's order.
fn collect_paragraphs(tree: &Tree, found: &mut Vec<Paragraph>) {
    if tree.tag == tree::Tag::of::<TextState>() {
        if let tree::State::Some(state) = &tree.state {
            if let Some(paragraph) = state.downcast_ref::<TextState>() {
                found.push(Paragraph {
                    text: paragraph.content().to_owned(),
                    width: paragraph.min_width(),
                    used: false,
                });
            }
        }
    }
    for child in &tree.children {
        collect_paragraphs(child, found);
    }
}

/// The measured width of the text run `text` laid out at `bounds`: its paragraph's, the first
/// of that text not taken yet. Why in order: the tree keeps the paragraphs in the order the
/// walk meets the runs (a widget's children in its order both ways), so the second "−" of the
/// window is the second "−" paragraph, though they are at two sizes. None found (a widget
/// keeping its paragraph otherwise) is the box's own width.
fn measured(paragraphs: &mut [Paragraph], text: &str, bounds: Rectangle) -> f32 {
    let next = paragraphs
        .iter_mut()
        .find(|paragraph| !paragraph.used && paragraph.text == text);
    match next {
        Some(paragraph) => {
            paragraph.used = true;
            paragraph.width
        }
        None => bounds.width,
    }
}

/// The value of `future`, which needs no waiting (the CPU renderer is made at once).
pub(super) fn ready<T>(future: impl Future<Output = T>) -> T {
    match pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("the renderer waited"),
    }
}

impl super::App {
    /// `--dump-layout`: the shot window's layout written as `shot` is taken (nothing without the
    /// flag), laid out at the shot's logical size. Why at the shot: the layout is then the one
    /// in the picture.
    pub(super) fn dump_layout(&self, shot: &iced::window::Screenshot) -> Result<(), String> {
        let Some(path) = self.headless.dump_layout() else {
            return Ok(());
        };
        let scale = shot.scale_factor;
        let size = Size::new(
            shot.size.width as f32 / scale,
            shot.size.height as f32 / scale,
        );
        let view = match self.shot_window_id() {
            Some(id) => self.view_window(id),
            None => self.view(),
        };
        write(path, view, size, self.zoom(), scale)
            .map_err(|error| format!("cannot write the layout to {}: {error}", path.display()))?;
        println!("wrote {}", path.display());
        Ok(())
    }

    /// The window the screenshot is of, while it is open.
    fn shot_window_id(&self) -> Option<iced::window::Id> {
        use crate::headless::ShotWindow;
        match self.headless.shot_window() {
            ShotWindow::Main => self.main_window,
            ShotWindow::Instrument => self.instrument_window,
            ShotWindow::Settings => self.settings_window,
            ShotWindow::Help => self.help.window,
            ShotWindow::Params => self.params.window,
            ShotWindow::SampleEditor => self.editor.window,
            ShotWindow::Panel(panel) => self.panel_windows.window(panel),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::app::tests::app_on;

    const F1: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/f1_four_on_floor.json"
    );

    // F1's main window at 1000 x 560: the dump holds the toolbar's labels where the toolbar's
    // spread lays them out (File's run inside File's button, its measured width the label's
    // own, `text_width` at 14 px), the window's size and zoom, and boxes, none outside the
    // window but what scrolls.
    #[test]
    fn the_main_windows_dump_holds_the_toolbars_labels_where_it_lays_them_out() {
        use crate::ui::toolbar::{retry_edit_file_and_help, Group};
        let (mut app, _player) = app_on(Path::new(F1));
        let size = Size::new(1000.0, 560.0);
        let _ = app.update(Message::WindowResized(size));
        let dump = dump(app.view(), size, Zoom::DEFAULT, 1.0);
        assert_eq!(dump["window"], json!({"width": 1000.0, "height": 560.0}));
        assert_eq!(dump["zoom"], 100);
        let texts = dump["texts"].as_array().expect("texts");
        let file = texts
            .iter()
            .find(|run| run["text"] == "File")
            .expect("File's label");
        let spread = app.toolbar().spread();
        let [_, _, file_button, _] = retry_edit_file_and_help(&spread);
        let x = file["x"].as_f64().expect("x") as f32;
        assert!(file_button.contains(&x), "{file} in {file_button:?}");
        let measured = file["measured"].as_f64().expect("measured") as f32;
        let label = crate::ui::text_fit::text_width("File", crate::ui::fonts::size::BODY);
        assert!(
            (measured - label).abs() < 1e-3,
            "{measured} against {label}"
        );
        assert!(spread.left(Group::Menus).is_some());
        let boxes = dump["boxes"].as_array().expect("boxes");
        assert!(boxes.len() > 20, "{}", boxes.len());
        for found in boxes.iter().filter(|found| found["scrolled"] == false) {
            let right = found["x"].as_f64().expect("x") + found["width"].as_f64().expect("w");
            assert!(right <= 1000.0 + 1e-3, "{found}");
        }
    }
}
