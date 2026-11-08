//! The pop-out button at the right end of a stacked panel's header strip: the channel rack's,
//! the playlist's, the piano roll's and the mixer's (the Library stays docked). A press puts the
//! panel in a window of its own where that window last was, or at its first place, as a divider
//! dragged sideways does; the drag stays. Why a button too: a panel alone in the stack has no
//! divider over it, so it could not pop out at all. In the panel's own window the spot shows
//! nothing: that window's way back is its close button (and the panel's key or toolbar button
//! while it has the keyboard), none of them in the strip. The mixer draws its header on its
//! desk's canvas, so it draws the glyph there too and hit-tests it with its other controls
//! ([`crate::ui::mixer_layout::pop_out_rect`]); the others take this widget.
//!
//! A mini grip and no face: the dividers' grip ([`crate::ui::grip`]) at 6 px, two 1 px lines 2 px
//! apart, in the disabled text's tone, lit to the text's under the pointer, centred in a 24 px
//! target with no frame or fill. Why the grip: it is what a user already drags to pop a panel
//! out, so the button reads as the same handle, "grab here"; a window with an arrow leaving it
//! read as another app's icon. It was that glyph, 12 px of 1.5 px strokes, and
//! before that in the secondary text's tone on a face that lit under the pointer.

use iced::widget::canvas::{self, Event, Frame, Geometry};
use iced::widget::{container, mouse_area};
use iced::{mouse, Color, Element, Point, Rectangle, Renderer, Theme};

use crate::app::Message;
use crate::ui::hint::{hover_area, Hover};
use crate::ui::panels::Panel;
use crate::ui::style;

/// The button's side: a 24 px target, as every press takes.
pub const SIZE: f32 = 24.0;
/// The mini grip's width, centred in the button: a quarter of the dividers' 24.
const GRIP: f32 = 6.0;
/// The space between the button and the strip's right edge where the strip has no padding of
/// its own (the rack's and the mixer's headers, drawn on canvases): the playlist header's and
/// the piano roll strip's side padding, 12 px, so the four buttons stand alike.
pub const STRIP_RIGHT: f32 = 12.0;

/// The button for `panel`: the mini grip, dim, lit under the pointer, in its 24 px target, its hint
/// naming the panel ([`hint`]); no tip, the hint bar says it.
pub fn button<'a>(panel: Panel) -> Element<'a, Message> {
    // In a container of the target's size, which a layout dump reports as a box.
    let target = container(canvas::Canvas::new(Target).width(SIZE).height(SIZE))
        .width(SIZE)
        .height(SIZE);
    let face = mouse_area(target)
        .on_press(Message::PopOutPanel(panel))
        .interaction(mouse::Interaction::Pointer);
    hover_area(face, Hover::PopOut(panel))
}

/// The glyph's colour: the disabled text's tone, the text's while the pointer is `over` it.
pub fn glyph_color(over: bool) -> Color {
    if over {
        style::palette().text
    } else {
        style::palette().text_disabled
    }
}

/// What `panel`'s button does, as the hint bar says it after its name, "Pop out": "opens the
/// playlist in its own window".
pub fn hint(panel: Panel) -> String {
    let name = match panel {
        Panel::Rack => "channel rack",
        Panel::Playlist => "playlist",
        Panel::PianoRoll => "piano roll",
        Panel::Mixer => "mixer",
        Panel::Browser => "Library",
        Panel::Settings => "Settings",
    };
    format!("opens the {name} in its own window")
}

/// The button's 24 px target: the mini grip centred in it, lit while the pointer is over it.
struct Target;

impl canvas::Program<Message> for Target {
    /// Whether the pointer was over the target when last seen.
    type State = bool;

    fn update(
        &self,
        over: &mut bool,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let Event::Mouse(mouse::Event::CursorMoved { .. } | mouse::Event::CursorLeft) = event
        else {
            return None;
        };
        let now = cursor.is_over(bounds);
        (now != *over).then(|| {
            *over = now;
            canvas::Action::request_redraw()
        })
    }

    fn draw(
        &self,
        over: &bool,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let color = glyph_color(*over);
        draw_glyph(&mut frame, Point::ORIGIN, color);
        vec![frame.into_geometry()]
    }
}

/// The mini grip in `frame` in `color`, centred in the 24 px button whose top-left corner is at
/// `at` ([`crate::ui::grip::draw_lines`]): the button's own canvas draws it so, and the mixer's
/// desk, which draws its header itself, at the button's place there
/// ([`crate::ui::mixer_layout::pop_out_rect`]).
pub fn draw_glyph(frame: &mut Frame, at: Point, color: Color) {
    crate::ui::grip::draw_lines(frame, middle(at), GRIP, color);
}

/// The mini grip's ink in a button whose top-left corner is `at`: the grip's two lines, `GRIP`
/// wide, around the button's middle ([`crate::ui::grip::lines`]).
#[cfg(test)]
pub fn glyph_lines(at: Point) -> [Rectangle; 2] {
    crate::ui::grip::lines(middle(at), GRIP)
}

/// The middle of the button whose top-left corner is `at`.
fn middle(at: Point) -> Point {
    Point::new(at.x + SIZE / 2.0, at.y + SIZE / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Each stacked panel's hint says the button opens it in its own window, the panel named as
    // the manual names it in running text.
    #[test]
    fn each_buttons_hint_names_its_panel() {
        assert_eq!(
            hint(Panel::Rack),
            "opens the channel rack in its own window"
        );
        assert_eq!(
            hint(Panel::Playlist),
            "opens the playlist in its own window"
        );
        assert_eq!(
            hint(Panel::PianoRoll),
            "opens the piano roll in its own window"
        );
        assert_eq!(hint(Panel::Mixer), "opens the mixer in its own window");
    }

    // The mini grip by hand in a button at (964, 0): its middle (976, 12), two lines 6 px wide
    // from x 973 to 979, one on row 10 and one on row 13 (the dividers' rule: a pixel above
    // and under the middle row), rows 11 and 12 between them the ground: 2 × 6 = 12 pixels of
    // ink at a scale of 1, a quarter of a divider grip's (2 × 24).
    #[test]
    fn the_mini_grip_is_two_six_pixel_lines_two_apart() {
        let [top, bottom] = glyph_lines(Point::new(964.0, 0.0));
        assert_eq!(
            top,
            Rectangle::new(Point::new(973.0, 10.0), iced::Size::new(6.0, 1.0))
        );
        assert_eq!(
            bottom,
            Rectangle::new(Point::new(973.0, 13.0), iced::Size::new(6.0, 1.0))
        );
        assert_eq!(
            bottom.y - (top.y + top.height),
            2.0,
            "2 px of ground between"
        );
        let ink: f32 = [top, bottom]
            .iter()
            .map(|line| line.width * line.height)
            .sum();
        assert_eq!(ink, 12.0);
    }
}
