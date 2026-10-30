//! A first-run note: a speech bubble under a toolbar control, an arrow from the bubble to it
//! and a ring around it, drawn in the window's own style over the panels. The ring and the
//! arrow take no events, so the control under the ring still takes its press; the bubble takes
//! every press on it, its words too, so nothing under it is pressed through it, and only its
//! buttons act. The places come from [`bubble`], which the screenshot probe reads too.

use iced::widget::{button, canvas, column, container, opaque, pin, row, stack, text};
use iced::{mouse, Element, Fill, Point, Rectangle, Renderer, Theme};

use crate::app::Message;
use crate::ui::style;

/// What the note's buttons say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalloutMessage {
    /// The window has drawn its first frame: the note waiting shows.
    Due,
    GotIt,
    /// Opens Settings on its Keys page.
    OpenKeys,
}

/// The bubble's width, and how far the ring stands off the control.
pub const BUBBLE_WIDTH: f32 = 340.0;
pub const RING_MARGIN: f32 = 4.0;
/// The space between the ring's foot and the bubble's top, which the arrow spans.
const ARROW_DROP: f32 = 12.0;
const ARROW_HALF_WIDTH: f32 = 9.0;
/// The bubble's least space from the window's edges.
const EDGE: f32 = 8.0;

/// The ring around `anchor`.
pub fn ring(anchor: Rectangle) -> Rectangle {
    anchor.expand(RING_MARGIN)
}

/// Where the bubble's top-left corner goes for a control at `anchor` in a window `window`
/// wide: under the ring, starting a little left of the control's middle, kept inside the
/// window.
pub fn bubble(anchor: Rectangle, window: f32) -> Point {
    let ring = ring(anchor);
    let x = (anchor.center_x() - 48.0)
        .min(window - BUBBLE_WIDTH - EDGE)
        .max(EDGE);
    Point::new(x, ring.y + ring.height + ARROW_DROP)
}

/// The note: `lines` in the bubble (the first bright, the rest dim), "Got it" (and "Open Keys
/// settings" when `keys` says) under them, pointing at `anchor` in a window `window` wide.
pub fn view<'a>(
    anchor: Rectangle,
    window: f32,
    lines: Vec<String>,
    keys: bool,
) -> Element<'a, Message> {
    let at = bubble(anchor, window);
    let mut words = column![].spacing(6);
    for (index, line) in lines.into_iter().enumerate() {
        let color = if index == 0 {
            style::TEXT
        } else {
            style::TEXT_DIM
        };
        words = words.push(text(line).size(13).color(color));
    }
    let choose = |label: &'a str, lit: bool, message: CalloutMessage| {
        button(text(label).size(13))
            .padding([5, 10])
            .style(style::tool_button(lit))
            .on_press(Message::Callout(message))
    };
    let mut buttons = row![choose("Got it", true, CalloutMessage::GotIt)].spacing(8);
    if keys {
        buttons = buttons.push(choose(
            "Open Keys settings",
            false,
            CalloutMessage::OpenKeys,
        ));
    }
    let body = container(column![words, buttons].spacing(10))
        .padding(12)
        .width(BUBBLE_WIDTH)
        .style(style::callout);
    let pointer = canvas(Pointer { anchor, at }).width(Fill).height(Fill);
    stack![pointer, pin(opaque(body)).x(at.x).y(at.y)].into()
}

/// The ring and the arrow, over the whole window and taking no events.
struct Pointer {
    anchor: Rectangle,
    /// The bubble's top-left corner.
    at: Point,
}

impl canvas::Program<Message> for Pointer {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let ring = ring(self.anchor);
        let line = canvas::Stroke::default()
            .with_color(style::ACCENT)
            .with_width(2.5);
        frame.stroke(
            &canvas::Path::rounded_rectangle(ring.position(), ring.size(), 6.0.into()),
            line,
        );
        let tip = Point::new(self.anchor.center_x(), ring.y + ring.height + 2.0);
        let arrow = canvas::Path::new(|path| {
            path.move_to(tip);
            path.line_to(Point::new(tip.x + ARROW_HALF_WIDTH, self.at.y + 1.0));
            path.line_to(Point::new(tip.x - ARROW_HALF_WIDTH, self.at.y + 1.0));
            path.close();
        });
        frame.fill(&arrow, style::ACCENT);
        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use iced::Size;

    use super::*;

    // Under a control at x 500..580, y 12..44 of a 1000 px window: the ring 4 px out, the
    // bubble 12 px under it from x 540 − 48 = 492; near the right edge it stays 8 px inside;
    // near the left, 8 px from it.
    #[test]
    fn the_bubble_hangs_under_the_ring_inside_the_window() {
        let anchor = Rectangle::new(Point::new(500.0, 12.0), Size::new(80.0, 32.0));
        assert_eq!(
            ring(anchor),
            Rectangle::new(Point::new(496.0, 8.0), Size::new(88.0, 40.0))
        );
        assert_eq!(bubble(anchor, 1000.0), Point::new(492.0, 60.0));
        assert_eq!(bubble(anchor, 700.0).x, 700.0 - BUBBLE_WIDTH - 8.0);
        let left = Rectangle::new(Point::new(10.0, 12.0), Size::new(40.0, 32.0));
        assert_eq!(bubble(left, 1000.0).x, 8.0);
    }
}
