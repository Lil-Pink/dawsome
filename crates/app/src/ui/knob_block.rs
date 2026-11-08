//! A block of knobs, as the mixer shows each effect and the instrument window each of the
//! synth's headings: a head ([`BLOCK_HEAD`]) over rows of knobs, at most five to a row and two
//! rows for more ([`knob_grid`]). Each knob has its label over it and its value under it; a
//! choice (a whole parameter whose values have names: the filter's type, the synth's waves) is
//! a button holding its value's name, which a click steps on ([`next_choice`]). Blocks lie left
//! to right in lines as wide as their area, each line under the one before ([`flow`]).
//!
//! The mixer ([`crate::ui::mixer_layout`]) and the synth's window
//! ([`crate::ui::synth_panel`]) both lay out, draw and hit their blocks from here, so the two
//! answer alike: the same knob, value text, choice button and automated and linked dots, found
//! under the cursor by the same rectangles they are drawn in. The mixer draws them flat, the
//! synth's window as a console's modules ([`Finish`]).

use dawsome_engine::processor::{ParamInfo, ProcessorFactory};
use iced::widget::canvas::Frame;
use iced::{Point, Rectangle, Size};

use crate::ui::console::{self, Finish};
use crate::ui::fonts::size;
use crate::ui::mixer::centered;
use crate::ui::mixer_state::{effect_param_text, is_choice};
use crate::ui::rack::{draw_automated_dot, draw_linked_dot};
use crate::ui::style;
use crate::ui::text_fit::fit_width;

/// The space between two blocks, across and down.
pub const BLOCK_GAP: f32 = 6.0;
/// A block's head: an effect's light, its name, its arrows and its remove cross; a heading's
/// name.
pub const BLOCK_HEAD: f32 = 22.0;
/// The knobs' margin inside a block, either side, and the block's foot under its last row.
pub const BLOCK_INSET: f32 = 6.0;
pub const BLOCK_FOOT: f32 = 4.0;
/// One knob's place in a block: its label, the knob and its value under it.
pub const KNOB_WIDTH: f32 = 60.0;
/// The room a knob's name and value take in its column: all but a pixel either side, so two
/// neighbours' words stay 2 px apart. Why not the 2 px either side it had: at the type scale's
/// dense size (11 px, was 10) "Resonance", 57.8 px in the bundled font, would be cut.
pub const KNOB_TEXT_ROOM: f32 = KNOB_WIDTH - 2.0;
pub const KNOB_ROW_HEIGHT: f32 = 58.0;
/// A knob across: a mixer strip's pan knob's size.
pub const KNOB_SIZE: f32 = 28.0;
/// The knob's top, its label's and its value's middles, down from the top of its place.
const KNOB_TOP: f32 = 14.0;
const KNOB_LABEL_Y: f32 = 7.0;
const KNOB_VALUE_Y: f32 = 50.0;
/// A choice's button (the filter's type, the phaser's stages): across its place, at the
/// knob's middle.
pub const CHOICE_HEIGHT: f32 = 18.0;
const CHOICE_INSET: f32 = 4.0;
/// The most knobs a block's row holds, and the fewest columns a block has (room in its head for
/// a name such as "Stereo width" beside the arrows).
pub const MOST_KNOBS_IN_A_ROW: usize = 5;
pub const LEAST_KNOB_COLUMNS: usize = 3;
/// How far a console block's well reaches past its knobs' places, either side and above.
const WELL_MARGIN: f32 = 2.0;

/// How a block lays out `count` knobs: (columns, rows). One row of up to five; more go in two
/// rows, the first as long as the second or one longer (the EQ's ten: five and five, the
/// phaser's six: three and three); never under three columns. A block with none (a plugin
/// with no parameters, one this build does not know) keeps one row for its words.
pub fn knob_grid(count: usize) -> (usize, usize) {
    let rows = count.div_ceil(MOST_KNOBS_IN_A_ROW).max(1);
    (count.div_ceil(rows).max(LEAST_KNOB_COLUMNS), rows)
}

/// The size of a block of `count` knobs; the mixer's add tile is the size of a block of none.
pub fn block_size(count: usize) -> Size {
    let (columns, rows) = knob_grid(count);
    grid_size(columns, rows)
}

/// The size of a block whose knobs lie in `rows` rows of `columns` places.
pub fn grid_size(columns: usize, rows: usize) -> Size {
    Size::new(
        columns as f32 * KNOB_WIDTH + 2.0 * BLOCK_INSET,
        BLOCK_HEAD + rows as f32 * KNOB_ROW_HEIGHT + BLOCK_FOOT,
    )
}

/// Where blocks of `sizes`, in order, sit from the top left of `view`: left to right, a block
/// that would cross the right edge starting the next line (unless it is the line's first), each
/// line as tall as its tallest block, [`BLOCK_GAP`] between blocks and between lines. With the
/// bottom of the last line.
pub fn flow(sizes: impl IntoIterator<Item = Size>, view: Rectangle) -> (Vec<Rectangle>, f32) {
    let sizes = sizes.into_iter();
    let mut rects = Vec::with_capacity(sizes.size_hint().0);
    let (mut x, mut y, mut line) = (view.x, view.y, 0.0f32);
    for size in sizes {
        if x > view.x && x + size.width > view.x + view.width {
            x = view.x;
            y += line + BLOCK_GAP;
            line = 0.0;
        }
        rects.push(Rectangle::new(Point::new(x, y), size));
        x += size.width + BLOCK_GAP;
        line = line.max(size.height);
    }
    (rects, y + line)
}

/// The head of the block at `block`: across its top, [`BLOCK_HEAD`] tall.
pub fn head_rect(block: Rectangle) -> Rectangle {
    Rectangle::new(block.position(), Size::new(block.width, BLOCK_HEAD))
}

/// The place of knob `index` of the `count` in the block at `block`: row by row, left to
/// right.
pub fn knob_cell(block: Rectangle, count: usize, index: usize) -> Rectangle {
    grid_cell(block, knob_grid(count).0, index)
}

/// The place of knob `index` in the block at `block` whose rows hold `columns` knobs each: row
/// by row, left to right.
pub fn grid_cell(block: Rectangle, columns: usize, index: usize) -> Rectangle {
    let columns = columns.max(1);
    let (row, column) = (index / columns, index % columns);
    Rectangle::new(
        Point::new(
            block.x + BLOCK_INSET + column as f32 * KNOB_WIDTH,
            block.y + BLOCK_HEAD + row as f32 * KNOB_ROW_HEIGHT,
        ),
        Size::new(KNOB_WIDTH, KNOB_ROW_HEIGHT),
    )
}

/// The knob of the `count` in the block at `block` whose place holds `point`.
pub fn knob_at(block: Rectangle, count: usize, point: Point) -> Option<usize> {
    grid_knob_at(block, knob_grid(count).0, count, point)
}

/// The knob of the `count` in the block at `block`, its rows `columns` knobs long, whose place
/// holds `point`.
pub fn grid_knob_at(block: Rectangle, columns: usize, count: usize, point: Point) -> Option<usize> {
    (0..count).find(|&knob| grid_cell(block, columns, knob).contains(point))
}

/// The knob in its place `cell`, centered across it under its label.
pub fn knob_rect(cell: Rectangle) -> Rectangle {
    Rectangle::new(
        Point::new(cell.center_x() - KNOB_SIZE / 2.0, cell.y + KNOB_TOP),
        Size::new(KNOB_SIZE, KNOB_SIZE),
    )
}

/// A choice's button in its place `cell`: across it, at the knob's middle.
pub fn choice_rect(cell: Rectangle) -> Rectangle {
    let middle = knob_rect(cell).center_y();
    Rectangle::new(
        Point::new(cell.x + CHOICE_INSET, middle - CHOICE_HEIGHT / 2.0),
        Size::new(KNOB_WIDTH - 2.0 * CHOICE_INSET, CHOICE_HEIGHT),
    )
}

/// The middles of a knob's label, over it, and of its value, under it, in its place `cell`.
pub fn knob_label_center(cell: Rectangle) -> Point {
    Point::new(cell.center_x(), cell.y + KNOB_LABEL_Y)
}

pub fn knob_value_center(cell: Rectangle) -> Point {
    Point::new(cell.center_x(), cell.y + KNOB_VALUE_Y)
}

/// The value a press on the button of the choice `info` at `now` steps to: the next, or after
/// the last the first.
pub fn next_choice(info: &ParamInfo, now: f32) -> f32 {
    if now + 1.0 > info.max {
        info.min
    } else {
        now + 1.0
    }
}

/// Whether `rect` lies wholly inside `view`.
pub fn shows_whole(view: Rectangle, rect: Rectangle) -> bool {
    rect.x >= view.x
        && rect.y >= view.y
        && rect.x + rect.width <= view.x + view.width
        && rect.y + rect.height <= view.y + view.height
}

/// How far along `min..max` `value` is, from 0 to 1.
pub fn fraction(value: f32, min: f32, max: f32) -> f32 {
    let span = max - min;
    if span > 0.0 {
        ((value - min) / span).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// The ground of the block at `block`, as far as it shows in `view`: flat, the strips' color;
/// on the console a raised module with its knobs in a sunken well under its head, drawn whole
/// (the synth lays its blocks inside its canvas, which would clip one that were not).
pub fn draw_ground(frame: &mut Frame, block: Rectangle, view: Rectangle, finish: Finish) {
    let Some(ground) = block.intersection(&view) else {
        return;
    };
    match finish {
        Finish::Flat => {
            frame.fill_rectangle(ground.position(), ground.size(), style::palette().strip)
        }
        Finish::Console => {
            console::draw_module(frame, block);
            console::draw_well(frame, well_rect(block));
        }
    }
}

/// The well a console block's knobs sit in: across their places and [`WELL_MARGIN`] past them
/// either side and above, down to a rim that margin and a pixel more over its module's foot
/// ([`console::DROP`] over the block's, the shadow's room); the values' words end higher still.
pub fn well_rect(block: Rectangle) -> Rectangle {
    let (left, top) = (
        block.x + BLOCK_INSET - WELL_MARGIN,
        block.y + BLOCK_HEAD - WELL_MARGIN,
    );
    let right = block.x + block.width - BLOCK_INSET + WELL_MARGIN;
    let bottom = block.y + block.height - console::DROP - WELL_MARGIN - 1.0;
    Rectangle::new(Point::new(left, top), Size::new(right - left, bottom - top))
}

/// Parameter `index` of `factory`'s processor at `value`, in a project at `sample_rate`, in
/// its place `cell`: its knob with the value in its unit under it, or a choice's button holding
/// the value's name; its automated and linked `dots`; in its `finish`.
pub fn draw_param(
    frame: &mut Frame,
    cell: Rectangle,
    factory: &dyn ProcessorFactory,
    (index, value): (usize, f32),
    sample_rate: u32,
    dots: (bool, bool),
    finish: Finish,
) {
    let Some(info) = factory.params().get(index) else {
        return;
    };
    let text = effect_param_text(factory, index, value, sample_rate).unwrap_or_default();
    let knob = (!is_choice(factory, index)).then(|| fraction(value, info.min, info.max));
    let texts = (factory.knob_label(index), text.as_str());
    draw_knob_cell(frame, cell, texts, knob, dots, finish);
}

/// One knob's place `cell`: its `name` over it, then the knob at `knob` (a fraction of its
/// range) with its `value` under it, or with no fraction a choice's button holding the value's
/// name; its automated and linked `dots` at the knob's corner, as a strip's knobs have them; in
/// its `finish`.
pub fn draw_knob_cell(
    frame: &mut Frame,
    cell: Rectangle,
    (name, value): (&str, &str),
    knob: Option<f32>,
    (automated, linked): (bool, bool),
    finish: Finish,
) {
    let room = KNOB_TEXT_ROOM;
    if let Some(shown) = fit_width(name, room, size::DENSE) {
        let at = knob_label_center(cell);
        centered(
            frame,
            &shown,
            at,
            style::palette().text_secondary,
            size::DENSE,
        );
    }
    let center = knob_rect(cell).center();
    match knob {
        Some(fraction) => {
            finish.draw_knob(frame, center, KNOB_SIZE, fraction);
            if let Some(shown) = fit_width(value, room, size::DENSE) {
                let at = knob_value_center(cell);
                centered(frame, &shown, at, style::palette().text, size::DENSE);
            }
        }
        None => {
            let button = choice_rect(cell);
            finish.draw_key(frame, button);
            if let Some(shown) = fit_width(value, button.width - 4.0, size::DENSE) {
                centered(
                    frame,
                    &shown,
                    button.center(),
                    style::palette().text,
                    size::DENSE,
                );
            }
        }
    }
    if automated {
        draw_automated_dot(frame, center, KNOB_SIZE);
    }
    if linked {
        draw_linked_dot(frame, center, KNOB_SIZE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Blocks flow left to right and wrap before the right edge, each line as tall as its
    // tallest, by hand: in a view 300 wide from (10, 20), to x 310, blocks 192, 100 and 120 wide
    // (142, 84 and 84 tall): the first two share a line (10..202, then 208..308), the third
    // would end at 314 + 120 = 434 and starts the next line at y 20 + 142 + 6 = 168, whose foot
    // is 168 + 84 = 252. A block wider than the view keeps a line of its own rather than none.
    #[test]
    fn blocks_wrap_into_lines_as_tall_as_their_tallest() {
        let view = Rectangle::new(Point::new(10.0, 20.0), Size::new(300.0, 500.0));
        let sizes = [
            Size::new(192.0, 142.0),
            Size::new(100.0, 84.0),
            Size::new(120.0, 84.0),
        ];
        let (rects, foot) = flow(sizes, view);
        let corners: Vec<(f32, f32)> = rects.iter().map(|rect| (rect.x, rect.y)).collect();
        assert_eq!(corners, [(10.0, 20.0), (208.0, 20.0), (10.0, 168.0)]);
        assert_eq!(foot, 252.0);
        let (wide, foot) = flow([Size::new(400.0, 84.0), Size::new(50.0, 84.0)], view);
        assert_eq!((wide[0].x, wide[0].y, wide[1].y), (10.0, 20.0, 110.0));
        assert_eq!(foot, 194.0);
        let (none, foot) = flow([], view);
        assert!(none.is_empty());
        assert_eq!(foot, 20.0, "no line, no height");
    }

    // A press finds the knob whose place holds it, row by row: in a block of 7 at (0, 0)
    // (4 columns, 2 rows), (6 + 60·2 + 1, 22 + 58 + 1) is the seventh knob's place (row 1,
    // column 2); the head and the margins are none.
    #[test]
    fn a_press_finds_the_knob_whose_place_holds_it() {
        let block = Rectangle::new(Point::ORIGIN, block_size(7));
        assert_eq!(knob_grid(7), (4, 2));
        assert_eq!(knob_at(block, 7, Point::new(127.0, 81.0)), Some(6));
        assert_eq!(knob_at(block, 7, Point::new(7.0, 23.0)), Some(0));
        assert_eq!(knob_at(block, 7, Point::new(30.0, 10.0)), None, "the head");
        assert_eq!(knob_at(block, 7, Point::new(3.0, 50.0)), None, "the margin");
        assert_eq!(
            knob_at(block, 7, Point::new(200.0, 100.0)),
            None,
            "the eighth place, past the last knob"
        );
    }

    // A choice steps through its values and after the last comes back to the first: the
    // synth's waves (0..3).
    #[test]
    fn a_choice_steps_on_and_round() {
        let waves = ParamInfo {
            key: "osc1_wave",
            label: "Osc 1 wave",
            min: 0.0,
            max: 3.0,
            default: 2.0,
            unit: "",
            whole: true,
            taper: dawsome_engine::processor::Taper::Linear,
        };
        assert_eq!(next_choice(&waves, 0.0), 1.0);
        assert_eq!(next_choice(&waves, 2.0), 3.0);
        assert_eq!(next_choice(&waves, 3.0), 0.0);
    }
}
