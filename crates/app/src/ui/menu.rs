//! The menus a press opens over a panel: a channel's, a knob's and a mixer control's automation
//! menu, a playlist track's, an automation segment's curve menu, a mixer slot's and a Library
//! row's. Each is drawn on the window's top layer, above every panel, and kept inside the
//! window. Why: drawn inside its panel, a menu was cut off by the panel's edge and covered by
//! the panel below (the last rack row's menu under the playlist).
//!
//! [`floating`] puts a menu there. In the view it stands where the press was, taking no room
//! and drawing nothing; the window draws it as an overlay (iced's own layer over the whole
//! window, the one a pick list's menu uses), at the press's place in the window whatever the
//! panel's scroll, placed by [`place_below`]: down and right of the press, flipped up when it
//! would pass the window's foot (the hint bar's top while the bar shows, so a menu never covers
//! the line where its own rows' hints are written) unless that would take it above the area it
//! was opened in, moved left when it would pass its right edge.

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{Tree, Widget};
use iced::advanced::{mouse, overlay, renderer, Clipboard, Shell};
use iced::alignment::Vertical;
use iced::widget::{button, container, space, text, Button, Column};
use iced::{Element, Event, Fill, Length, Point, Rectangle, Renderer, Size, Theme, Vector};

use crate::app::Message;
use crate::ui::style;

/// A menu row's height: every row of every panel menu is this tall, so a menu's size is its
/// rows'.
pub const ROW_HEIGHT: f32 = 26.0;
/// The rows' text.
pub const TEXT_SIZE: f32 = 13.0;
/// The menu's ground around its rows.
pub const PADDING: f32 = 4.0;

/// Where a menu `size` big opened at window point `anchor` goes in a window `window` big: its
/// top-left corner at the anchor; above it instead when it would pass the window's foot (and at
/// the window's top when there is no room above either); left of it as far as needed when it
/// would pass the window's right edge, never left of the window. The tests' and the shot
/// probes' figures: the window's own rule, [`place_below`] with no area above to keep clear of.
#[cfg(test)]
pub fn place(anchor: Point, size: Size, window: Size) -> Point {
    place_below(anchor, size, window, 0.0)
}

/// [`place`], with the menu kept below window y `top`: flipped up only when it then stays below
/// `top`, else as low as it fits above the window's foot (over the press), and at `top` when it
/// is taller than that room. The overlay passes the top of the area the menu was opened in (a
/// panel's rows, under its header). Why: a channel's menu of eight rows opened on the last rack
/// row of a 400 px window, flipped up past the rack's rows, showed the rack's header and the
/// toolbar drawn over its first rows.
pub fn place_below(anchor: Point, size: Size, window: Size, top: f32) -> Point {
    let y = if anchor.y + size.height <= window.height {
        anchor.y
    } else if anchor.y - size.height >= top {
        anchor.y - size.height
    } else {
        (window.height - size.height).max(top)
    };
    let x = anchor.x.min(window.width - size.width).max(0.0);
    Point::new(x, y)
}

/// A menu row: `label`, sending `message` when pressed, dim and pressing nothing without one.
pub fn row<'a>(
    label: impl text::IntoFragment<'a>,
    message: Option<Message>,
) -> Button<'a, Message> {
    let dimmed = message.is_none();
    let label = container(text(label).size(TEXT_SIZE))
        .height(Fill)
        .align_y(Vertical::Center);
    button(label)
        .width(Fill)
        .height(ROW_HEIGHT)
        .padding([0, 10])
        .style(style::menu_item(dimmed))
        .on_press_maybe(message)
}

/// A menu row that is a switch: `label`, a tick at its right end while `on` (as the Record
/// menu's switches show theirs), sending `message` when pressed.
pub fn check_row<'a>(label: &'a str, on: bool, message: Message) -> Button<'a, Message> {
    let tick = if on { "\u{2713}" } else { "" };
    let content = iced::widget::row![
        text(label).size(TEXT_SIZE),
        space().width(Fill),
        text(tick).size(TEXT_SIZE),
    ]
    .height(Fill)
    .align_y(Vertical::Center);
    button(content)
        .width(Fill)
        .height(ROW_HEIGHT)
        .padding([0, 10])
        .style(style::menu_item(false))
        .on_press(message)
}

/// `rows` as a menu `width` wide: on the File menu's ground, its rows a [`ROW_HEIGHT`] each.
pub fn panel<'a>(rows: Column<'a, Message>, width: f32) -> Element<'a, Message> {
    container(rows)
        .width(width + 2.0 * PADDING)
        .padding(PADDING)
        .style(style::dialog)
        .into()
}

/// `menu` drawn on the window's top layer from where this stands in the view (see the module's
/// words), keeping clear of the window's last `foot` pixels (the hint bar while it shows): put
/// it where the press was, as a `pin` at the press's point.
pub fn floating<'a>(menu: impl Into<Element<'a, Message>>, foot: f32) -> Element<'a, Message> {
    Element::new(Floating {
        menu: menu.into(),
        foot,
    })
}

/// The menu's place in the view: no room, no drawing, its menu an overlay.
struct Floating<'a> {
    menu: Element<'a, Message>,
    /// The window's last pixels the menu keeps clear of.
    foot: f32,
}

impl Widget<Message, Theme, Renderer> for Floating<'_> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Shrink, Length::Shrink)
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        _limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(Size::ZERO)
    }

    fn draw(
        &self,
        _tree: &Tree,
        _renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.menu)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.menu));
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        _renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let tree = tree.children.first_mut()?;
        Some(overlay::Element::new(Box::new(Overlay {
            menu: &mut self.menu,
            tree,
            anchor: layout.position() + translation,
            // The visible part of what the menu stands in (a scrollable's, the panel's rows),
            // in window coordinates.
            top: viewport.y.max(0.0),
            foot: self.foot,
        })))
    }
}

/// The menu on the window's top layer, opened at window point `anchor`, keeping below window y
/// `top` (the area it was opened in) and clear of the window's last `foot` pixels.
struct Overlay<'a, 'b> {
    menu: &'b mut Element<'a, Message>,
    tree: &'b mut Tree,
    anchor: Point,
    top: f32,
    foot: f32,
}

impl overlay::Overlay<Message, Theme, Renderer> for Overlay<'_, '_> {
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        let room = Size::new(bounds.width, (bounds.height - self.foot).max(0.0));
        let limits = layout::Limits::new(Size::ZERO, room);
        let node = self
            .menu
            .as_widget_mut()
            .layout(self.tree, renderer, &limits);
        let at = place_below(self.anchor, node.size(), room, self.top);
        node.move_to(at)
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        self.menu.as_widget().draw(
            self.tree,
            renderer,
            theme,
            style,
            layout,
            cursor,
            &layout.bounds(),
        );
    }

    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) {
        self.menu.as_widget_mut().update(
            self.tree,
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            &layout.bounds(),
        );
    }

    /// Over the menu, never none, so what lies under it takes no cursor: a press on the menu's
    /// edge between its rows stays the menu's.
    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        if !cursor.is_over(layout.bounds()) {
            return mouse::Interaction::None;
        }
        self.menu
            .as_widget()
            .mouse_interaction(self.tree, layout, cursor, &layout.bounds(), renderer)
            .max(mouse::Interaction::Idle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // By hand, in a 1000 x 560 window: a 168 x 138 menu opened at (83, 254) fits down and right
    // of the press; opened at (83, 500) its foot would be at 638, so it goes above, from 362;
    // opened at (900, 254) it would end at 1068, so it moves left to start at 832. Taller than
    // the room above and below (a 140 px window), it starts at the window's top; wider than the
    // window, at its left.
    #[test]
    fn a_menu_stays_inside_the_window() {
        let window = Size::new(1000.0, 560.0);
        let menu = Size::new(168.0, 138.0);
        assert_eq!(
            place(Point::new(83.0, 254.0), menu, window),
            Point::new(83.0, 254.0)
        );
        assert_eq!(
            place(Point::new(83.0, 500.0), menu, window),
            Point::new(83.0, 362.0)
        );
        assert_eq!(
            place(Point::new(900.0, 254.0), menu, window),
            Point::new(832.0, 254.0)
        );
        assert_eq!(
            place(Point::new(900.0, 500.0), menu, window),
            Point::new(832.0, 362.0),
            "both"
        );
        assert_eq!(
            place(Point::new(10.0, 60.0), menu, Size::new(100.0, 140.0)),
            Point::new(0.0, 2.0)
        );
        assert_eq!(
            place(Point::new(83.0, 422.0), menu, window).y,
            422.0,
            "just fits"
        );
    }

    // By hand, the channel menu of eight rows (168 x 216) opened on p5_gain's last rack row at
    // (83, 262) in a 1000 x 400 window over its 22 px hint bar (room to y 378), the rack's rows
    // from y 88: down it would end at 478, up it would start at 46, above the rows; so as low as
    // it fits, 378 − 216 = 162. Below a top of 0 it flips to 46 as before; a top of 40 still
    // lets it; taller than the room under the top (a 300 px menu), at the top.
    #[test]
    fn a_menu_flips_up_only_as_far_as_the_area_it_opened_in() {
        let room = Size::new(1000.0, 378.0);
        let menu = Size::new(168.0, 216.0);
        let at = Point::new(83.0, 262.0);
        assert_eq!(place_below(at, menu, room, 88.0), Point::new(83.0, 162.0));
        assert_eq!(place_below(at, menu, room, 0.0), Point::new(83.0, 46.0));
        assert_eq!(place(at, menu, room), Point::new(83.0, 46.0));
        assert_eq!(place_below(at, menu, room, 40.0), Point::new(83.0, 46.0));
        assert_eq!(
            place_below(at, Size::new(168.0, 300.0), room, 88.0),
            Point::new(83.0, 88.0)
        );
    }
}
