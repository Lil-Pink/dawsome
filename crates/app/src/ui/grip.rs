//! A drag handle that resizes the panel part beside it: a strip a few pixels tall with a grip
//! drawn in its middle and the vertical resize cursor, reporting the press, each move while
//! held and the release in window y (the strip itself moves while the part resizes), and a
//! double press when it resets something. The piano roll's velocity lane and the dividers
//! between the stacked panels use it (the mixer's top edge is one of those).

use std::time::{Duration, Instant};

use iced::widget::canvas::{self, Event, Frame, Geometry};
use iced::{mouse, Point, Rectangle, Renderer, Size, Theme};

use crate::app::Message;
use crate::ui::hint::{hover_change, Hover};
use crate::ui::style;

/// The grip drawn in the handle's middle: its width and the two lines' gap.
const GRIP_WIDTH: f32 = 24.0;
const GRIP_GAP: f32 = 2.0;

/// Two presses at most this far apart in time and space are a double press, as on the mixer.
/// Why the distance too: a line dragged and grabbed again at once has moved.
const DOUBLE_PRESS_TIME: Duration = Duration::from_millis(400);
const DOUBLE_PRESS_DISTANCE: f32 = 6.0;

/// A drag handle and what it reports. `P` makes the press's message from the window y.
pub struct Grip<P> {
    /// Whether the handle is held, so drags and the release are reported anywhere.
    pub held: bool,
    /// A press on it, at window y.
    pub pressed: P,
    /// The cursor at window y while it is held.
    pub dragged: fn(f32) -> Message,
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

impl<P: Fn(f32) -> Message> canvas::Program<Message> for Grip<P> {
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
                Some(canvas::Action::publish((self.pressed)(at.y)).and_capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) if self.held => {
                Some(canvas::Action::publish((self.dragged)(position.y)))
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
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), style::STRIP);
        let grip = if self.held {
            style::ACCENT
        } else {
            style::TEXT_DIM
        };
        let left = (bounds.width - GRIP_WIDTH) / 2.0;
        let middle = bounds.height / 2.0;
        for y in [middle - GRIP_GAP, middle + GRIP_GAP - 1.0] {
            frame.fill_rectangle(Point::new(left, y), Size::new(GRIP_WIDTH, 1.0), grip);
        }
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
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), style::STRIP);
        vec![frame.into_geometry()]
    }
}
