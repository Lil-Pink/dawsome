//! A drag handle that resizes the panel part beside it: a strip a few pixels tall with a grip
//! drawn in its middle and the vertical resize cursor, reporting the press, each move while
//! held and the release at window points (the strip itself moves while the part resizes), and
//! a double press when it resets something. The dividers between the stacked panels use it
//! (the mixer's top edge is one of those); a divider dragged
//! sideways is armed to pop its panel out, and its whole strip lights. The Library's right edge
//! is the family's deeper line, a groove with a lit lip that drags nothing ([`Seam`]).

use std::time::{Duration, Instant};

use iced::widget::canvas::{self, Event, Frame, Geometry};
use iced::{mouse, Color, Point, Rectangle, Renderer, Size, Theme};

use crate::app::Message;
use crate::ui::hint::{hover_change, Hover};
use crate::ui::style;

/// The grip drawn in the handle's middle: its width and the two lines' gap.
const GRIP_WIDTH: f32 = 24.0;
const GRIP_GAP: f32 = 2.0;

/// A grip's two lines, `width` wide and centred on `middle`: each 1 px tall, one ending a pixel
/// above `middle`'s row and one starting a pixel under it, so 2 px of ground part them. The
/// dividers' grip is so, and the pop-out button's mini grip ([`crate::ui::pop_out`]), the same
/// motif: grab here to move the panel.
pub fn lines(middle: Point, width: f32) -> [Rectangle; 2] {
    let left = middle.x - width / 2.0;
    [middle.y - GRIP_GAP, middle.y + GRIP_GAP - 1.0]
        .map(|y| Rectangle::new(Point::new(left, y), Size::new(width, 1.0)))
}

/// A grip's [`lines`] in `frame`, in `color`.
pub fn draw_lines(frame: &mut Frame, middle: Point, width: f32, color: Color) {
    for line in lines(middle, width) {
        frame.fill_rectangle(line.position(), line.size(), color);
    }
}

/// Two presses at most this far apart in time and space are a double press, as on the mixer.
/// Why the distance too: a line dragged and grabbed again at once has moved.
const DOUBLE_PRESS_TIME: Duration = Duration::from_millis(400);
const DOUBLE_PRESS_DISTANCE: f32 = 6.0;

/// A drag handle and what it reports. `P` makes the press's message from the window point.
pub struct Grip<P> {
    /// Whether the handle is held, so drags and the release are reported anywhere.
    pub held: bool,
    /// Whether the release would do something other than resize (a divider dragged sideways
    /// pops its panel out): the whole strip lights, not only its grip.
    pub armed: bool,
    /// A press on it, at a window point.
    pub pressed: P,
    /// The cursor at a window point while it is held.
    pub dragged: fn(Point) -> Message,
    pub released: Message,
    /// What a second press soon after the first reports instead of a press, if anything.
    pub reset: Option<Message>,
    /// What the hint bar names while the cursor is over it.
    pub hover: Hover,
}

/// What a handle remembers: what the hint bar names, and when and where it was last pressed.
#[derive(Default)]
pub struct GripState {
    hover: Option<Hover>,
    pressed: Option<(Instant, Point)>,
}

impl<P: Fn(Point) -> Message> canvas::Program<Message> for Grip<P> {
    type State = GripState;

    fn update(
        &self,
        state: &mut GripState,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                cursor.position_in(bounds)?;
                let at = cursor.position()?;
                let now = crate::ui::clock::now();
                let double = state.pressed.is_some_and(|(when, from)| {
                    now.duration_since(when) <= DOUBLE_PRESS_TIME
                        && from.distance(at) <= DOUBLE_PRESS_DISTANCE
                });
                if let (true, Some(reset)) = (double, &self.reset) {
                    state.pressed = None;
                    return Some(canvas::Action::publish(reset.clone()).and_capture());
                }
                state.pressed = Some((now, at));
                Some(canvas::Action::publish((self.pressed)(at)).and_capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) if self.held => {
                Some(canvas::Action::publish((self.dragged)(*position)))
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if self.held => {
                Some(canvas::Action::publish(self.released.clone()))
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let now = cursor.position_in(bounds).map(|_| self.hover.clone());
                hover_change(&mut state.hover, now)
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &GripState,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let ground = if self.armed {
            style::accent()
        } else {
            style::palette().strip
        };
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), ground);
        let grip = if self.armed {
            style::palette().text
        } else if self.held {
            style::accent()
        } else {
            style::palette().text_secondary
        };
        let middle = Point::new(bounds.width / 2.0, bounds.height / 2.0);
        draw_lines(&mut frame, middle, GRIP_WIDTH, grip);
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _state: &GripState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if self.held || cursor.is_over(bounds) {
            mouse::Interaction::ResizingVertically
        } else {
            mouse::Interaction::default()
        }
    }
}

/// The seam down the Library's right edge, between it and the stacked panels: a groove half
/// as wide as a divider's band is tall, in the shaded edge's colour (darker than every panel's
/// ground), its far wall lit as the synth console lights a sunken well's lips, so it reads cut
/// into the window. Why deeper than the dividers: they part the panels of one stack, while the
/// seam parts the Library from all four, which read as one with it across a plain edge.
pub struct Seam;

/// The seam's width: half a divider's band ([`crate::ui::panels::DIVIDER_BAND`]), chosen on
/// sight over the full band.
pub const SEAM_WIDTH: f32 = crate::ui::panels::DIVIDER_BAND / 2.0;

impl canvas::Program<Message> for Seam {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), style::palette().seam);
        frame.fill_rectangle(
            Point::new(bounds.width - 1.0, 0.0),
            Size::new(1.0, bounds.height),
            style::palette().line_strong,
        );
        vec![frame.into_geometry()]
    }
}

/// The mixer's top edge while no panel is above it: the divider's band, drawn plain and
/// dragging nothing, because the mixer's desk is laid out under it
/// ([`crate::ui::mixer_layout::HANDLE_HEIGHT`]).
pub struct Edge;

impl canvas::Program<Message> for Edge {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), style::palette().strip);
        vec![frame.into_geometry()]
    }
}
