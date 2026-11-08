//! A pick list narrower than its longest entry: its face shows the chosen entry cut with an
//! ellipsis to fit ([`text_fit`]), and its open menu stays as wide as the longest entry needs,
//! so every entry reads whole where it is chosen. Why: the piano roll's strip had no room for
//! its Scale chooser at the width of "Minor pentatonic" beside the roll's Play and Loop, and a
//! name is read in the menu; the face only reminds.
//!
//! The toolkit's pick list opens its menu exactly as wide as the list, and draws its face with
//! the same text as the menu's rows. So each entry is wrapped ([`Entry`]) to give the face a cut
//! text of its own, and the list is wrapped ([`WideMenu`]) to lay its menu out against a box of
//! the list's place and the menu's width; everything else is the toolkit's pick list unchanged.

use std::fmt;

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{tree, Operation, Tree, Widget};
use iced::advanced::{mouse, overlay, renderer, Clipboard, Shell};
use iced::widget::pick_list;
use iced::{Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector};

use crate::app::Message;
use crate::ui::text_fit;

/// The pick list's room around its text, a side: what the strip's pickers have.
const PADDING_X: f32 = 6.0;
const PADDING_Y: f32 = 2.0;

/// The width a pick list of `options` at text `size` takes when left to size itself: its
/// longest entry, the arrow (as wide as the text is tall) and the padding (a side's, either
/// side, and one more before the arrow).
pub fn natural_width<T: ToString>(options: &[T], size: f32) -> f32 {
    let longest = options
        .iter()
        .map(|option| text_fit::text_width(&option.to_string(), size))
        .fold(0.0, f32::max);
    longest + size + 3.0 * PADDING_X
}

/// A pick list of `options` `width` wide at text `size`, `selected` on its face cut to fit,
/// sending `on_select` of the entry chosen; its menu as wide as [`natural_width`].
pub fn short_pick_list<'a, T>(
    options: &[T],
    selected: Option<T>,
    on_select: impl Fn(T) -> Message + 'a,
    width: f32,
    size: f32,
) -> Element<'a, Message>
where
    T: Clone + PartialEq + ToString + 'a,
{
    // The face's room: the list's width less its padding and the arrow's share.
    let room = width - size - 3.0 * PADDING_X;
    let face = selected.map(|value| {
        let whole = value.to_string();
        let cut = text_fit::fit_width(&whole, room, size).map(|cut| cut.into_owned());
        Entry {
            value,
            face: Some(cut.unwrap_or_default()),
        }
    });
    let entries: Vec<Entry<T>> = options
        .iter()
        .map(|value| Entry {
            value: value.clone(),
            face: None,
        })
        .collect();
    let list = pick_list(entries, face, move |entry: Entry<T>| on_select(entry.value))
        .width(width)
        .text_size(size)
        .padding([PADDING_Y, PADDING_X]);
    Element::new(WideMenu {
        list: list.into(),
        menu_width: natural_width(options, size),
        menu: layout::Node::default(),
    })
}

/// An entry of the list: its value, and on the face the text it shows there.
#[derive(Debug, Clone)]
struct Entry<T> {
    value: T,
    /// Set on the selected entry the face shows: its text cut to fit.
    face: Option<String>,
}

/// The same entry whatever its face shows: the menu marks the selected entry by it.
impl<T: PartialEq> PartialEq for Entry<T> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<T: ToString> fmt::Display for Entry<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.face {
            Some(face) => f.write_str(face),
            None => f.write_str(&self.value.to_string()),
        }
    }
}

/// The pick list, its menu laid out `menu_width` wide from the list's left edge.
struct WideMenu<'a> {
    list: Element<'a, Message>,
    menu_width: f32,
    /// The box the open menu is laid out against: the list's place, `menu_width` wide. Kept
    /// here because the menu's layout borrows it for as long as the menu is open.
    menu: layout::Node,
}

impl Widget<Message, Theme, Renderer> for WideMenu<'_> {
    fn size(&self) -> Size<Length> {
        self.list.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.list.as_widget().size_hint()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.list.as_widget_mut().layout(tree, renderer, limits)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.list
            .as_widget()
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }

    fn tag(&self) -> tree::Tag {
        self.list.as_widget().tag()
    }

    fn state(&self) -> tree::State {
        self.list.as_widget().state()
    }

    fn children(&self) -> Vec<Tree> {
        self.list.as_widget().children()
    }

    fn diff(&self, tree: &mut Tree) {
        self.list.as_widget().diff(tree);
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.list
            .as_widget_mut()
            .operate(tree, layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.list.as_widget_mut().update(
            tree, event, layout, cursor, renderer, clipboard, shell, viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.list
            .as_widget()
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let bounds = layout.bounds();
        let Self {
            list,
            menu_width,
            menu,
        } = self;
        *menu = layout::Node::new(Size::new(bounds.width.max(*menu_width), bounds.height));
        let wide = Layout::with_offset(Vector::new(bounds.x, bounds.y), menu);
        list.as_widget_mut()
            .overlay(tree, wide, renderer, viewport, translation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::fonts::size;

    // The face holds the chosen entry whole when it fits and cut to fit with an ellipsis when
    // not; the menu's rows are the entries whole, and the selected one is found among them
    // whatever its face shows.
    #[test]
    fn the_face_is_cut_and_the_entries_stay_whole() {
        let options = ["Scale off", "Minor pentatonic"];
        let width = natural_width(&options, size::LABEL) / 2.0;
        let room = width - size::LABEL - 3.0 * PADDING_X;
        let cut =
            text_fit::fit_width("Minor pentatonic", room, size::LABEL).expect("a start of it fits");
        assert!(cut.ends_with('\u{2026}') && cut.starts_with("Min"), "{cut}");
        assert!(text_fit::text_width(&cut, size::LABEL) <= room);
        let face = Entry {
            value: "Minor pentatonic",
            face: Some(cut.into_owned()),
        };
        let row = Entry {
            value: "Minor pentatonic",
            face: None,
        };
        assert_eq!(row.to_string(), "Minor pentatonic");
        assert_eq!(face, row, "the face's entry is the menu's row");
    }
}
