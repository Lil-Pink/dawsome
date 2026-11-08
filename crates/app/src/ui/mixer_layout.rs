//! Every position in the mixer, in logical pixels. The mixer canvas draws from these values, its
//! hit test reads them, and the screenshot probe samples the points they give, so the drawing
//! and its checks cannot drift apart.
//!
//! Mixer coordinates start at the desk's top-left corner: the canvas under the panel's 6 px
//! top edge ([`HANDLE_HEIGHT`], the divider over it while a panel is above it). Alone in the
//! window the panel sits under the toolbar, so window y = mixer y + 72 + 6 ([`in_window`]).
//!
//! The desk is as tall as the height the panel stack shows the mixer, less its top edge
//! ([`desk_height`]), and never under [`LEAST_DESK`]: the stack never shows the mixer shorter
//! than [`LEAST_HEIGHT`], so every strip and a line of the effects' blocks show whole, never
//! scrolled out of the panel (they were: the desk was laid out from the kept height while the
//! stack showed less). Alone, the mixer fills the area under the toolbar. The canvas
//! reports the height it got, which every hit test uses. Its strips' fader track takes what the
//! height leaves ([`fader_bottom`]): 50 px at the least desk, 200 px at most, from a 379 px desk
//! (the controls keep their sizes).
//!
//! The desk, top to bottom, mixer y: the header (0..24) with the channel routing control at
//! x 560..790; then the strips (from 24) left of the effects area, which takes the rest of the
//! width right of the last strip, and at least [`EFFECTS_MIN_WIDTH`] (x 668..1000 in a 1000 px
//! window, where the strips scroll under it).
//!
//! A strip is 60 px wide: the master at x 4, insert k at x 12 + 62k, all moved left by the
//! scroll. Within a strip at x (mixer y, the least desk, 229 px): body 26..227, name 28..44,
//! mute light centered at (x+12, 51), pan knob 28 px from (x+16, 58), fader track x+12..x+18
//! over 96..146 with the cap centered on y = 146 − 50·fader, meters x+36..x+42 and x+45..x+51
//! over 96..146, the master's clip light at (x+44, 90), route button x+16..x+44 over 155..179,
//! send knob 24 px from (x+18, 183), label 209..225. A taller desk moves the fader's foot, the
//! route button, the send knob, the label and the body's foot down together (the fader growing
//! to 200 px), and past that the label and the body's foot alone. Why so close together: for
//! a mixer half as tall; the strip was 304 px with a 100 px fader and roomier gaps, and every
//! part stays, whole.
//!
//! The effects area (mixer y): from 28, 4 px under the header (from 52 while 24..48 held the
//! selected insert's name, and while that row stood empty after the header's routing control,
//! centred, named the insert alone), the selected insert's effects as
//! blocks in chain order, left to right in lines as wide as the area, each line under the one
//! before ([`chain_rects`]); after the last block, while the insert has room, the add tile. A
//! block is its head (the effect's light, its name, the arrows moving it a place earlier or
//! later and its remove cross, 22 px) over its knobs, at most five to a row, two rows for more
//! ([`knob_grid`]). The wheel over the area scrolls the lines when they are taller than it; an
//! effect just added shows by whole lines, from the first block while it can
//! ([`SHOW_ADDED`]). A block dragged by its name takes a neighbour's place once the pointer
//! passes that neighbour's middle ([`reorder_place`]).

use dawsome_engine::project::{InsertId, EFFECT_SLOTS};
use iced::{Point, Rectangle, Size};

use crate::ui::panels::Panel;

use crate::ui::knob_block::{
    self, block_size, head_rect, knob_at, knob_cell, BLOCK_FOOT, BLOCK_HEAD, BLOCK_INSET,
    KNOB_ROW_HEIGHT, KNOB_SIZE, KNOB_WIDTH, MOST_KNOBS_IN_A_ROW,
};

/// The mixer panel's height until the divider over it is dragged: its least, 235 px, every
/// strip whole with a 50 px fader and a line of the effects' blocks. Why its least (310 before,
/// and 400 and 504 before that, each made smaller in turn): it had been dragged down to 182 px,
/// as short as the divider let it go. Half of 310, the height aimed at, cannot hold a whole
/// strip, so the strip was made compact instead ([`LEAST_DESK`]); the
/// 85 px it gave up go to the playlist and the piano roll
/// ([`crate::ui::panels::ROLL_SHARE`]). The divider makes it taller. Kept in `settings.json`
/// as `mixer.height` when dragged off it.
pub const DEFAULT_HEIGHT: f32 = LEAST_HEIGHT;

/// The panel's top edge: the divider over it ([`crate::ui::panels::DIVIDER_BAND`] is this
/// band; dragged up the mixer grows, down it shrinks), drawn plain while it is alone.
pub const HANDLE_HEIGHT: f32 = 6.0;

/// The desk's least height: a strip with a 50 px fader track and the controls under it
/// (96 + 50 + 83 = 229; 219 until the route button grew to 24 px). The effects area beside the strips scrolls its own lines.
pub const LEAST_DESK: f32 = FADER_TOP + LEAST_FADER_SPAN + UNDER_FADER;

/// The mixer panel's least height, its top edge over the least desk: 6 + 229 = 235 px. The
/// panel stack never shows it shorter ([`crate::ui::panels::min_height`]).
pub const LEAST_HEIGHT: f32 = HANDLE_HEIGHT + LEAST_DESK;

/// The fader track's length on the least desk: 50 px, 66 dB of meter over it (was 100 px).
const LEAST_FADER_SPAN: f32 = 50.0;

// The least desk holds a whole line of the tallest block (two knob rows, the EQ's ten knobs:
// 22 + 2 × 58 + 4 = 142) between the blocks' first line (28) and the strips' foot (2 px over
// the desk's): 28 + 142 = 170 ≤ 227. So the strips set the least height, not the blocks: a
// shorter knob row would take nothing off it.
const _: () =
    assert!(BLOCKS_TOP + BLOCK_HEAD + 2.0 * KNOB_ROW_HEIGHT + BLOCK_FOOT <= LEAST_DESK - BODY_FOOT);

/// The desk under the top edge of a mixer panel `height` tall.
pub fn desk_height(height: f32) -> f32 {
    (height - HANDLE_HEIGHT).max(LEAST_DESK)
}

pub const HEADER_HEIGHT: f32 = 24.0;

/// Where the header's title starts: the strips' side padding, as the other headers'.
pub const TITLE_X: f32 = 12.0;

/// The routing control in the header: the "−" button, the label, the "+" button, 230 px in all,
/// the panel's title gap past "Mixer" ([`route_left`]). The buttons are 24 px wide (were 20)
/// and the header's 20 tall inside it: a 24 px circle round each stays inside the 24 px header,
/// WCAG 2.2's spacing exception.
pub const ROUTE_ARROW_WIDTH: f32 = 24.0;
/// The routing control's width.
const ROUTE_SPAN: f32 = 230.0;

pub const STRIP_WIDTH: f32 = 60.0;
pub const MASTER_LEFT: f32 = 4.0;
/// Insert k's left edge is `INSERTS_LEFT + STRIP_PITCH·k`.
pub const INSERTS_LEFT: f32 = 12.0;
pub const STRIP_PITCH: f32 = 62.0;

/// Mixer y of the strip body's top, 2 px under the header; its foot is [`BODY_FOOT`] above
/// the desk's ([`body_bottom`]).
pub const BODY_TOP: f32 = 26.0;
/// The desk under the strip bodies' foot.
const BODY_FOOT: f32 = 2.0;
pub const SELECTED_OUTLINE: f32 = 2.0;

pub const NAME_TOP: f32 = 28.0;
pub const NAME_HEIGHT: f32 = 16.0;

pub const LED_OFFSET: f32 = 12.0;
pub const LED_Y: f32 = 51.0;
pub const LED_RADIUS: f32 = 5.0;
/// The light takes clicks this far either side of its center.
pub const LED_HIT_HALF: f32 = 8.0;

pub const PAN_OFFSET: f32 = 16.0;
pub const PAN_TOP: f32 = 58.0;
pub const PAN_SIZE: f32 = 28.0;

/// The fader track: from fader 1 at the top to 0 at its foot, at most 200 px tall
/// ([`fader_bottom`]).
pub const FADER_TRACK_OFFSET: f32 = 12.0;
pub const FADER_TRACK_WIDTH: f32 = 6.0;
/// Its top: the cap at fader 1 (5 px over it) 5 px under the pan knob's foot (86).
pub const FADER_TOP: f32 = 96.0;
pub const MAX_FADER_SPAN: f32 = 200.0;
/// What a strip needs under its fader's foot: the route button 9 px down (4 px under the cap
/// at fader 0) and 24 tall (WCAG 2.2's target size; 14 before), 4 px, the send knob (24), 2 px, the
/// label (16), the body's 2 px and the desk's [`BODY_FOOT`] under it:
/// 9 + 24 + 4 + 24 + 2 + 16 + 2 + 2 = 83.
const UNDER_FADER: f32 = 83.0;
pub const CAP_OFFSET: f32 = 1.0;
pub const CAP_WIDTH: f32 = 28.0;
pub const CAP_HEIGHT: f32 = 10.0;
pub const CAP_LINE: f32 = 2.0;

/// The two meters, left then right, over the fader's height.
pub const METER_OFFSETS: [f32; 2] = [36.0, 45.0];
pub const METER_WIDTH: f32 = 6.0;
/// The meter shows −60 dB at the bottom to +6 dB at the top.
pub const METER_FLOOR_DB: f32 = -60.0;
pub const METER_RANGE_DB: f32 = 66.0;

pub const CLIP_OFFSET: f32 = 44.0;
/// The master's clip light: under the pan knob, 3 px over the meters' top.
pub const CLIP_Y: f32 = 90.0;
pub const CLIP_RADIUS: f32 = 3.0;

/// The route button and the send knob, this far under the fader's foot.
pub const ROUTE_OFFSET: f32 = 16.0;
const ROUTE_UNDER_FADER: f32 = 9.0;
pub const ROUTE_WIDTH: f32 = 28.0;
pub const ROUTE_HEIGHT: f32 = 24.0;

pub const SEND_OFFSET: f32 = 18.0;
const SEND_UNDER_FADER: f32 = 37.0;
pub const SEND_SIZE: f32 = 24.0;

/// The insert's number, 2 px above the body's foot.
pub const LABEL_HEIGHT: f32 = 16.0;

/// The blocks' first line, mixer y: 4 px under the header, the gap they had under the 24 px
/// row the selected insert's name held, level with the strips' names ([`NAME_TOP`]). Why not
/// 52, where they stood: the name went (the header's routing control names the insert) and its
/// row stayed an empty band over the blocks until they moved up into it.
pub const BLOCKS_TOP: f32 = HEADER_HEIGHT + 4.0;
/// The blocks' margin from the area's left edge.
pub const EFFECTS_INSET: f32 = 6.0;
/// The area's right margin, where the marks of lines scrolled out of view show.
pub const MARKS_WIDTH: f32 = 14.0;
// The effects' knobs are the size of a strip's pan knob.
const _: () = assert!(KNOB_SIZE == PAN_SIZE);
/// The remove cross, at the head's right end, and the two arrows left of it: each a target this
/// wide across the head's height, the glyph drawn in its middle at the size it always had. Why
/// 24: WCAG 2.2's target size, met by the spacing exception (centres 24 apart, the head 22
/// tall); the cross was 16 px and the arrows 14 before.
pub const HEAD_TARGET: f32 = 24.0;
/// The effects area's least width: the widest block (five columns) and the margins.
pub const EFFECTS_MIN_WIDTH: f32 =
    EFFECTS_INSET + MOST_KNOBS_IN_A_ROW as f32 * KNOB_WIDTH + 2.0 * BLOCK_INSET + MARKS_WIDTH;
/// Pixels the wheel scrolls the effects' lines per notch.
pub const EFFECTS_SCROLL_PER_NOTCH: f32 = 48.0;

/// A block's or the add tile's menu opens under its head, this far in from its left edge.
pub const MENU_INSET: f32 = 6.0;
/// The menus' rows' width: the add menu's longest row, "Compressor (Pressure4)", and a margin.
pub const MENU_WIDTH: f32 = 180.0;

/// Pixels the wheel scrolls the strips per notch: one strip.
pub const SCROLL_PER_NOTCH: f32 = STRIP_PITCH;

/// The effects area's left edge, where the strips end, in a panel `width` wide whose highest
/// insert is `last`: right of the last strip while the area keeps [`EFFECTS_MIN_WIDTH`] there,
/// else that width from the right edge, the strips scrolling under it.
pub fn effects_left(width: f32, last: InsertId) -> f32 {
    let strips = strip_left(last, 0.0) + STRIP_WIDTH + MASTER_LEFT;
    strips.min(width - EFFECTS_MIN_WIDTH).max(0.0)
}

/// Strip `id`'s left edge, mixer x, with the strips scrolled `scroll` px.
pub fn strip_left(id: InsertId, scroll: f32) -> f32 {
    let unscrolled = if id.is_master() {
        MASTER_LEFT
    } else {
        INSERTS_LEFT + STRIP_PITCH * f32::from(id.0)
    };
    unscrolled - scroll
}

/// How far the strips of a mixer with `last` as its highest insert can scroll in a panel
/// `width` wide: until the last strip's right edge reaches the effects area.
pub fn max_scroll(last: InsertId, width: f32) -> f32 {
    let right = strip_left(last, 0.0) + STRIP_WIDTH + MASTER_LEFT;
    (right - effects_left(width, last)).max(0.0)
}

/// The fader track's foot, mixer y, in a desk `desk` tall: what the controls under it leave,
/// at most [`MAX_FADER_SPAN`] under [`FADER_TOP`]. (146 at the least desk, the default; 296,
/// the most, from a 379 px desk on.)
pub fn fader_bottom(desk: f32) -> f32 {
    (desk.max(LEAST_DESK) - UNDER_FADER).min(FADER_TOP + MAX_FADER_SPAN)
}

/// The fader track's length in a desk `desk` tall.
pub fn fader_span(desk: f32) -> f32 {
    fader_bottom(desk) - FADER_TOP
}

/// The strip body's foot, mixer y, in a desk `desk` tall.
pub fn body_bottom(desk: f32) -> f32 {
    desk.max(LEAST_DESK) - BODY_FOOT
}

/// The insert number's top, mixer y, in a desk `desk` tall: 2 px over the body's foot.
pub fn label_top(desk: f32) -> f32 {
    body_bottom(desk) - LABEL_HEIGHT - 2.0
}

/// The mixer y the fader cap is centered on at position `fader`, in a desk `desk` tall.
pub fn cap_center_y(fader: f32, desk: f32) -> f32 {
    fader_bottom(desk) - fader_span(desk) * fader.clamp(0.0, 1.0)
}

/// The cap of strip at `left` with fader `fader`, in a desk `desk` tall.
pub fn cap_rect(left: f32, fader: f32, desk: f32) -> Rectangle {
    let center = cap_center_y(fader, desk);
    Rectangle::new(
        Point::new(left + CAP_OFFSET, center - CAP_HEIGHT / 2.0),
        Size::new(CAP_WIDTH, CAP_HEIGHT),
    )
}

/// The area a press on strip `left`'s fader lands in: the cap's width over the whole track.
pub fn fader_rect(left: f32, desk: f32) -> Rectangle {
    Rectangle::new(
        Point::new(left + CAP_OFFSET, FADER_TOP - CAP_HEIGHT / 2.0),
        Size::new(CAP_WIDTH, fader_span(desk) + CAP_HEIGHT),
    )
}

pub fn led_center(left: f32) -> Point {
    Point::new(left + LED_OFFSET, LED_Y)
}

pub fn pan_rect(left: f32) -> Rectangle {
    Rectangle::new(
        Point::new(left + PAN_OFFSET, PAN_TOP),
        Size::new(PAN_SIZE, PAN_SIZE),
    )
}

pub fn route_rect(left: f32, desk: f32) -> Rectangle {
    Rectangle::new(
        Point::new(left + ROUTE_OFFSET, fader_bottom(desk) + ROUTE_UNDER_FADER),
        Size::new(ROUTE_WIDTH, ROUTE_HEIGHT),
    )
}

pub fn send_rect(left: f32, desk: f32) -> Rectangle {
    Rectangle::new(
        Point::new(left + SEND_OFFSET, fader_bottom(desk) + SEND_UNDER_FADER),
        Size::new(SEND_SIZE, SEND_SIZE),
    )
}

pub fn name_rect(left: f32) -> Rectangle {
    Rectangle::new(
        Point::new(left, NAME_TOP),
        Size::new(STRIP_WIDTH, NAME_HEIGHT),
    )
}

pub fn body_rect(left: f32, desk: f32) -> Rectangle {
    Rectangle::new(
        Point::new(left, BODY_TOP),
        Size::new(STRIP_WIDTH, body_bottom(desk) - BODY_TOP),
    )
}

/// Meter `side`'s column of strip `left`, beside the fader track.
pub fn meter_rect(left: f32, side: usize, desk: f32) -> Rectangle {
    Rectangle::new(
        Point::new(left + METER_OFFSETS[side], FADER_TOP),
        Size::new(METER_WIDTH, fader_span(desk)),
    )
}

/// How much of a meter fills for a peak of `level` (linear): (dB + 60)/66, 0 at −60 dB and
/// below, 1 at +6 dB and above.
pub fn meter_fraction(level: f32) -> f32 {
    if level <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * level.log10();
    ((db - METER_FLOOR_DB) / METER_RANGE_DB).clamp(0.0, 1.0)
}

/// How high a meter `span` tall fills for a peak of `level`: 200·(dB + 60)/66 px at 200 px.
pub fn meter_height(level: f32, span: f32) -> f32 {
    span * meter_fraction(level)
}

/// The rectangle the effects' lines show in, on `desk`: the area under its title, inside its
/// margins, down to the strips' foot.
pub fn blocks_view(desk: Desk) -> Rectangle {
    let left = effects_left(desk.size.width, desk.last) + EFFECTS_INSET;
    let right = desk.size.width - MARKS_WIDTH;
    Rectangle::new(
        Point::new(left, BLOCKS_TOP),
        Size::new(
            (right - left).max(0.0),
            (body_bottom(desk.size.height) - BLOCKS_TOP).max(0.0),
        ),
    )
}

/// Whether an insert with `count` effects shows the add tile: while it has a free slot.
pub fn has_add_tile(count: usize) -> bool {
    count < usize::from(EFFECT_SLOTS)
}

/// The part of the add tile at `tile` that shows in `view`, drawn and pressed: the tile down to
/// the view's foot while at least a block's head of it shows there. Why the tile and no block
/// may show cut: on the least desk a line under a whole one is cut by its last 9 px or more
/// ([`added_scroll`]), and the tile is only an outline round its words, cut by no clip (the
/// software renderer draws a clipped frame's shapes and text whole). Never cut at the top: it
/// ends the last line, whose foot the lines never scroll above the view's.
pub fn shown_tile(tile: Rectangle, view: Rectangle) -> Option<Rectangle> {
    let part = tile.intersection(&view)?;
    (part.y == tile.y && part.height >= BLOCK_HEAD).then_some(part)
}

/// Where the blocks of `chain` (each effect's knob count, in chain order) and then the add
/// tile sit, unscrolled, from the top left of `view`, in lines ([`knob_block::flow`]). With
/// the bottom of the last line.
fn flow(chain: &[usize], view: Rectangle) -> (Vec<Rectangle>, f32) {
    let tile = has_add_tile(chain.len()).then_some(0);
    knob_block::flow(chain.iter().copied().chain(tile).map(block_size), view)
}

/// How far the effects' lines of `chain` can scroll on `desk`: until the last line's foot meets
/// the view's.
pub fn max_effects_scroll(chain: &[usize], desk: Desk) -> f32 {
    let view = blocks_view(desk);
    let (_, bottom) = flow(chain, view);
    (bottom - (view.y + view.height)).max(0.0)
}

/// The effects' scroll on `desk` as it shows: its own, kept inside what `chain` can scroll (a
/// chain that got shorter leaves less to scroll); [`SHOW_ADDED`] the one that shows the last
/// block.
pub fn effects_scroll(chain: &[usize], desk: Desk) -> f32 {
    let wanted = if desk.effects_scroll == SHOW_ADDED {
        added_scroll(chain, desk)
    } else {
        desk.effects_scroll
    };
    wanted.clamp(0.0, max_effects_scroll(chain, desk))
}

/// The effects' scroll an add leaves ([`crate::ui::mixer_state::MixerState::show_added`]):
/// shown as [`added_scroll`] gives it, until the wheel or another insert sets one.
pub const SHOW_ADDED: f32 = f32::INFINITY;

/// The scroll that shows the last block of `chain` on `desk`, the effect just added: by whole
/// lines, the fewest scrolled out above the view that show its head and the add tile's words
/// ([`shown_tile`]), else its head alone; none while they show from the top, so the first
/// block stays there. As far as the lines go when no line's top shows its head (its line is
/// the last and the one over it is two knob rows tall). Why whole lines, not the lines' end:
/// the least desk's view (165 px then, 175 once the route button was 24 px tall) held less
/// than two lines (84 + 6 + 84), so the end cut the
/// line over the last to its foot, and a block's head and knobs are drawn only while whole:
/// three effects left the first two as bare strips. Its 199 px since the blocks moved up to 28
/// hold two lines of one knob row, still not a two-row block's line (142) and another.
fn added_scroll(chain: &[usize], desk: Desk) -> f32 {
    let view = blocks_view(desk);
    let (rects, bottom) = flow(chain, view);
    let most = (bottom - (view.y + view.height)).max(0.0);
    let Some(&added) = chain
        .len()
        .checked_sub(1)
        .and_then(|place| rects.get(place))
    else {
        return 0.0;
    };
    let tile = rects.get(chain.len()).copied();
    let up = |rect: Rectangle, scroll: f32| Rectangle {
        y: rect.y - scroll,
        ..rect
    };
    let head_shows = |scroll: f32| {
        let head = Rectangle::new(
            up(added, scroll).position(),
            Size::new(added.width, BLOCK_HEAD),
        );
        head.intersection(&view) == Some(head)
    };
    let tile_shows =
        |scroll: f32| tile.is_none_or(|tile| shown_tile(up(tile, scroll), view).is_some());
    // Each line's top as a scroll, the first line's 0 (a line's blocks share its top).
    let mut tops: Vec<f32> = rects
        .iter()
        .map(|rect| rect.y - view.y)
        .filter(|&top| top <= most)
        .collect();
    tops.dedup();
    let first = |shows: &dyn Fn(f32) -> bool| tops.iter().copied().find(|&top| shows(top));
    first(&|top| head_shows(top) && tile_shows(top))
        .or_else(|| first(&head_shows))
        .unwrap_or(most)
}

/// Where the blocks of `chain` and then the add tile (while there is one) are on `desk`,
/// scrolled.
pub fn chain_rects(chain: &[usize], desk: Desk) -> Vec<Rectangle> {
    let scroll = effects_scroll(chain, desk);
    let (rects, _) = flow(chain, blocks_view(desk));
    rects
        .into_iter()
        .map(|rect| Rectangle {
            y: rect.y - scroll,
            ..rect
        })
        .collect()
}

/// The light of the block at `block`: where a strip's light sits in its strip, across the head.
pub fn effect_led_center(block: Rectangle) -> Point {
    Point::new(block.x + LED_OFFSET, block.y + BLOCK_HEAD / 2.0)
}

/// The [`HEAD_TARGET`]-wide target of the block at `block`'s head `places` targets in from its
/// right end, across the head's height.
fn head_target(block: Rectangle, places: f32) -> Rectangle {
    Rectangle::new(
        Point::new(block.x + block.width - places * HEAD_TARGET, block.y),
        Size::new(HEAD_TARGET, BLOCK_HEAD),
    )
}

/// The remove cross of the block at `block`, at its head's right end.
pub fn remove_rect(block: Rectangle) -> Rectangle {
    head_target(block, 1.0)
}

/// The arrow moving the block at `block` a place later in the chain (›), left of its remove
/// cross, and the one moving it a place earlier (‹), left of that.
pub fn later_rect(block: Rectangle) -> Rectangle {
    head_target(block, 2.0)
}

pub fn earlier_rect(block: Rectangle) -> Rectangle {
    head_target(block, 3.0)
}

/// The name of the block at `block`: its head between the light and the arrows, where a drag
/// starts.
pub fn effect_name_rect(block: Rectangle) -> Rectangle {
    let left = block.x + 2.0 * LED_OFFSET;
    let right = earlier_rect(block).x - 2.0;
    Rectangle::new(
        Point::new(left, block.y),
        Size::new((right - left).max(0.0), BLOCK_HEAD),
    )
}

/// Where the block dragged at place `place` of `chain` (each block's knob count, in the order
/// shown) goes for the pointer at `at` on `desk`: once the pointer is past the middle of the
/// block after it, it takes that block's place, and on while it is past the next one's; or the
/// same toward the chain's start. A point is past a block's middle when it is below that
/// block's line, or on its line right of its middle; before it when above its line, or on it
/// left of its middle.
pub fn reorder_place(chain: &[usize], desk: Desk, place: usize, at: Point) -> usize {
    let mut chain = chain.to_vec();
    let mut place = place.min(chain.len().saturating_sub(1));
    let start = place;
    // Each way in turn, never back within one move: a swap changes the widths on its line, and
    // stepping back over the block just passed would flicker between the two.
    while place + 1 < chain.len() && past_middle(&chain_rects(&chain, desk), place + 1, at) {
        chain.swap(place, place + 1);
        place += 1;
    }
    if place != start {
        return place;
    }
    while place > 0 && before_middle(&chain_rects(&chain, desk), place - 1, at) {
        chain.swap(place, place - 1);
        place -= 1;
    }
    place
}

/// The foot of block `index`'s line among `rects`: the lowest foot of the blocks starting at
/// its top.
fn line_foot(rects: &[Rectangle], index: usize) -> f32 {
    let top = rects[index].y;
    rects
        .iter()
        .filter(|rect| rect.y == top)
        .map(|rect| rect.y + rect.height)
        .fold(top, f32::max)
}

fn past_middle(rects: &[Rectangle], index: usize, at: Point) -> bool {
    let rect = rects[index];
    at.y >= line_foot(rects, index) || (at.y >= rect.y && at.x > rect.center_x())
}

fn before_middle(rects: &[Rectangle], index: usize, at: Point) -> bool {
    let rect = rects[index];
    at.y < rect.y || (at.y < line_foot(rects, index) && at.x < rect.center_x())
}

/// Where the menu of the block (or add tile) at `block` opens: under its head, [`MENU_INSET`]
/// in. The menu is drawn on the window's top layer from there ([`crate::ui::menu`]).
pub fn menu_anchor(block: Rectangle) -> Point {
    Point::new(block.x + MENU_INSET, block.y + BLOCK_HEAD)
}

/// Where the routing control starts on a desk `width` wide: centred in the header, (width −
/// [`ROUTE_SPAN`]) / 2, the title at the left and the pop-out button at the right as before.
/// Why centred: it names the selected insert and where it goes, the mixer's one line of state,
/// and the effects area's title that named the insert again went; it stood the
/// title gap past the title before, and at x 560 before that. Never closer to the title than
/// that gap ([`Panel::after_title`]: 12 + 37.112 for "Mixer" at 14 px + 48 = 97.112, a desk
/// under 424.224 px); on a desk too narrow for it and the pop-out button ([`pop_out_rect`])
/// with the strips' side padding between them (under 375 px), further left: its right end then
/// stands that padding left of the button. Why: it ran under the button there.
pub fn route_left(width: f32) -> f32 {
    let room = pop_out_rect(width).x - crate::ui::pop_out::STRIP_RIGHT - ROUTE_SPAN;
    ((width - ROUTE_SPAN) / 2.0)
        .max(Panel::Mixer.after_title(TITLE_X))
        .min(room)
}

pub fn route_down_rect(width: f32) -> Rectangle {
    header_rect(route_left(width), ROUTE_ARROW_WIDTH)
}

pub fn route_up_rect(width: f32) -> Rectangle {
    let right = route_left(width) + ROUTE_SPAN;
    header_rect(right - ROUTE_ARROW_WIDTH, ROUTE_ARROW_WIDTH)
}

pub fn route_label_rect(width: f32) -> Rectangle {
    let left = route_left(width) + ROUTE_ARROW_WIDTH;
    header_rect(left, ROUTE_SPAN - 2.0 * ROUTE_ARROW_WIDTH)
}

/// The pop-out button on a desk `width` wide: its 24 px square at the header's right end, the
/// strips' side padding from the edge ([`crate::ui::pop_out`]). The desk draws it while the
/// mixer is docked; a press on it pops the mixer out (nothing in the mixer's own window).
pub fn pop_out_rect(width: f32) -> Rectangle {
    let size = crate::ui::pop_out::SIZE;
    Rectangle::new(
        Point::new(width - crate::ui::pop_out::STRIP_RIGHT - size, 0.0),
        Size::new(size, size),
    )
}

fn header_rect(x: f32, width: f32) -> Rectangle {
    Rectangle::new(Point::new(x, 2.0), Size::new(width, HEADER_HEIGHT - 4.0))
}

/// A mixer point in the window, when the mixer is the only panel: under the toolbar and the
/// panel's top edge.
#[cfg(test)]
pub fn in_window(point: Point) -> Point {
    Point::new(
        point.x,
        point.y + crate::ui::layout::TOOLBAR_HEIGHT + HANDLE_HEIGHT,
    )
}

/// What a press at a mixer point lands on, ignoring menus (the state checks those first). An
/// effect is named by its place in the selected insert's chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// The pop-out button at the header's right end ([`pop_out_rect`]).
    PopOut,
    RouteDown,
    RouteUp,
    RouteLabel,
    Led(InsertId),
    ClipLight,
    Pan(InsertId),
    Fader(InsertId),
    Route(InsertId),
    Send(InsertId),
    Name(InsertId),
    Strip(InsertId),
    /// The effect's light: on or off.
    EffectLed(usize),
    /// The effect's name, where a drag moves it along the chain.
    EffectName(usize),
    RemoveEffect(usize),
    /// The arrow moving the effect a place earlier (none on the first), or later (none on the
    /// last).
    MoveEarlier(usize),
    MoveLater(usize),
    /// Knob `knob` of the effect at `block` (a choice's button too).
    Knob {
        block: usize,
        knob: usize,
    },
    /// Elsewhere on an effect's block.
    Block(usize),
    AddEffect,
}

/// What a press needs to know of the desk beyond the point: its size, how far its strips are
/// scrolled, the highest insert, and how far the effects' lines are scrolled.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Desk {
    pub size: Size,
    pub scroll: f32,
    pub last: InsertId,
    /// As the wheel left it, or [`SHOW_ADDED`]; shown inside what the chain can scroll
    /// ([`effects_scroll`]).
    pub effects_scroll: f32,
}

/// The control under `point` on `desk`, whose selected insert's effects have the knob counts
/// `chain`. Strips under the effects area are not hit, nor the parts of blocks scrolled out of
/// the view.
pub fn hit_test(point: Point, desk: Desk, chain: &[usize]) -> Option<Hit> {
    let Desk {
        size, scroll, last, ..
    } = desk;
    let (width, height) = (size.width, size.height);
    if point.y < HEADER_HEIGHT {
        return [
            (pop_out_rect(width), Hit::PopOut),
            (route_down_rect(width), Hit::RouteDown),
            (route_up_rect(width), Hit::RouteUp),
            (route_label_rect(width), Hit::RouteLabel),
        ]
        .into_iter()
        .find(|(rect, _)| rect.contains(point))
        .map(|(_, hit)| hit);
    }
    if point.x >= effects_left(width, last) {
        return effects_hit(point, desk, chain);
    }
    let strip = (0..=last.0).map(InsertId).find(|&id| {
        let left = strip_left(id, scroll);
        (left..left + STRIP_WIDTH).contains(&point.x)
    })?;
    let left = strip_left(strip, scroll);
    if !body_rect(left, height).contains(point) {
        return None;
    }
    if led_center(left).distance(point) <= LED_HIT_HALF {
        return Some(Hit::Led(strip));
    }
    let clip = Point::new(left + CLIP_OFFSET, CLIP_Y);
    if strip.is_master() && clip.distance(point) <= CLIP_RADIUS + 3.0 {
        return Some(Hit::ClipLight);
    }
    let controls = [
        (pan_rect(left), Hit::Pan(strip)),
        (fader_rect(left, height), Hit::Fader(strip)),
        (route_rect(left, height), Hit::Route(strip)),
        (send_rect(left, height), Hit::Send(strip)),
        (name_rect(left), Hit::Name(strip)),
    ];
    let control = controls
        .into_iter()
        .find(|(rect, _)| rect.contains(point))
        .map(|(_, hit)| hit);
    Some(control.unwrap_or(Hit::Strip(strip)))
}

/// What `point` in the effects area lands on: a part of a block, the add tile, or nothing. A
/// part counts only while it shows whole in the view, as it is drawn; the rest of a block
/// scrolled half out of view is the block.
fn effects_hit(point: Point, desk: Desk, chain: &[usize]) -> Option<Hit> {
    let view = blocks_view(desk);
    if !view.contains(point) {
        return None;
    }
    let rects = chain_rects(chain, desk);
    let block = rects.iter().position(|rect| rect.contains(point))?;
    let rect = rects[block];
    let whole = |part: Rectangle| part.intersection(&view) == Some(part);
    let Some(&count) = chain.get(block) else {
        return shown_tile(rect, view).map(|_| Hit::AddEffect);
    };
    let head = head_rect(rect);
    if !head.contains(point) {
        return Some(match knob_at(rect, count, point) {
            Some(knob) if whole(knob_cell(rect, count, knob)) => Hit::Knob { block, knob },
            _ => Hit::Block(block),
        });
    }
    if !whole(head) {
        return Some(Hit::Block(block));
    }
    if effect_led_center(rect).distance(point) <= LED_HIT_HALF {
        return Some(Hit::EffectLed(block));
    }
    if remove_rect(rect).contains(point) {
        return Some(Hit::RemoveEffect(block));
    }
    if block + 1 < chain.len() && later_rect(rect).contains(point) {
        return Some(Hit::MoveLater(block));
    }
    if block > 0 && earlier_rect(rect).contains(point) {
        return Some(Hit::MoveEarlier(block));
    }
    if effect_name_rect(rect).contains(point) {
        return Some(Hit::EffectName(block));
    }
    Some(Hit::Block(block))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::knob_block::{
        choice_rect, knob_grid, knob_label_center, knob_rect, knob_value_center,
    };

    const WIDTH: f32 = 1000.0;
    /// The desk the design was drawn at: the old 504 px panel, which had no handle.
    const DESIGN: f32 = 504.0;

    /// The design's window point (logical, the mixer alone under the design's 56 px toolbar) as
    /// a mixer point. The toolbar is 72 px and the panel has a 6 px handle
    /// over the desk now, so these tests keep the design's points and say where they are.
    fn window(x: f32, y: f32) -> Point {
        Point::new(x, y - 56.0)
    }

    /// A mixer point as the design's window point.
    fn design(point: Point) -> Point {
        Point::new(point.x, point.y + 56.0)
    }

    fn desk(height: f32, last: InsertId) -> Desk {
        Desk {
            size: Size::new(WIDTH, height),
            scroll: 0.0,
            last,
            effects_scroll: 0.0,
        }
    }

    /// An insert holding an effect of nine knobs (two rows, five and four), the delay (four)
    /// and the filter (three).
    const CHAIN: [usize; 3] = [9, 4, 3];

    // The strips on the design's 504 px desk, written out as the design's window points (under
    // its 56 px toolbar): master at x 4, insert k at 12 + 62k. The compact strip
    // puts the fader's top at mixer y 96 and, the desk being past 379 px, its track at its most,
    // 200 px, its foot at 296: the cap of a fader at 0.8 is centered on 296 − 160 = 136, window
    // 192, at 1.0 on 96, window 152; the meters start at 96 too (window 152), the route button
    // 9 px under the foot (305, window 361), the send knob 37 px under it (333, window 389: the
    // route button 24 px tall, 14 before, when it was 323, window 379); the
    // label 2 px over the body's foot, which is 2 px over the desk's: 484 and 502. (Were 210,
    // 170, 384, 404, 170, 480 and 500 with the roomier strip and its top at 114.)
    #[test]
    fn strips_and_their_parts_sit_where_the_design_puts_them() {
        assert_eq!(strip_left(InsertId::MASTER, 0.0), 4.0);
        assert_eq!(strip_left(InsertId(1), 0.0), 74.0);
        assert_eq!(strip_left(InsertId(3), 0.0), 198.0);
        assert_eq!(strip_left(InsertId(3), 62.0), 136.0);
        assert_eq!(design(Point::new(0.0, cap_center_y(0.8, DESIGN))).y, 192.0);
        assert_eq!(design(Point::new(0.0, cap_center_y(1.0, DESIGN))).y, 152.0);
        let cap = cap_rect(4.0, 0.8, DESIGN);
        assert_eq!((cap.x, cap.width, cap.height), (5.0, 28.0, 10.0));
        assert_eq!(
            design(route_rect(4.0, DESIGN).position()),
            Point::new(20.0, 361.0)
        );
        assert_eq!(
            design(send_rect(4.0, DESIGN).position()),
            Point::new(22.0, 389.0)
        );
        assert_eq!(
            design(meter_rect(4.0, 1, DESIGN).position()),
            Point::new(49.0, 152.0)
        );
        assert_eq!((label_top(DESIGN), body_bottom(DESIGN)), (484.0, 502.0));
    }

    // By hand: the effects area's least width is a five-column block (5·60 + 2·6 = 312) with
    // the 6 px left margin and the 14 px of marks: 332. Sixteen inserts end at 12 + 62·16 + 60 +
    // 4 = 1068, so at 1000 px the area takes the last 332 (from 668) and at 1920 px everything
    // right of the strips (from 1068); two inserts end at 200, so the area starts there.
    #[test]
    fn the_effects_area_takes_the_width_the_strips_leave() {
        assert_eq!(EFFECTS_MIN_WIDTH, 332.0);
        assert_eq!(effects_left(WIDTH, InsertId(16)), 668.0);
        assert_eq!(effects_left(1920.0, InsertId(16)), 1068.0);
        assert_eq!(effects_left(WIDTH, InsertId(2)), 200.0);
        assert_eq!(
            effects_left(200.0, InsertId(16)),
            0.0,
            "never left of the panel"
        );
    }

    // By hand: up to five knobs in one row, more in two (the first row one longer when they do
    // not split evenly), never under three columns; a block is 60 px a column and 12 px of
    // margins wide, 22 px of head, 58 px a row and 4 px of foot tall. Nine knobs (as the EQ's
    // ten) take 312 × 142, the stereo width (one) and the add tile (none) 192 × 84.
    #[test]
    fn a_block_holds_five_knobs_a_row_and_two_rows_at_most() {
        let grids: Vec<(usize, usize)> = [0, 1, 2, 3, 4, 5, 6, 7, 9, 10]
            .into_iter()
            .map(knob_grid)
            .collect();
        assert_eq!(
            grids,
            [
                (3, 1),
                (3, 1),
                (3, 1),
                (3, 1),
                (4, 1),
                (5, 1),
                (3, 2),
                (4, 2),
                (5, 2),
                (5, 2)
            ]
        );
        assert_eq!(block_size(9), Size::new(312.0, 142.0));
        assert_eq!(block_size(1), Size::new(192.0, 84.0));
        assert_eq!(block_size(0), block_size(1));
    }

    // By hand, at 1000 × 504 with sixteen inserts: the view is x 674..986 (668 + 6, 1000 − 14),
    // y 28..502 (4 px under the 24 px header; the strips' foot, 2 px over the desk's). The
    // nine-knob block (312 × 142) fills the first line; the delay (252 × 84) starts the next
    // at 28 + 142 + 6 = 176, the filter (192 × 84) does not fit beside it (674 + 252 + 6 + 192
    // = 1124) and starts the third at 176 + 84 + 6 = 266, and the add tile (192 × 84) the fourth
    // at 356: the last line ends at 440, inside the view, so nothing scrolls. At 1920 px (the
    // view x 1074..1906) the three blocks share the first line and the tile starts the second,
    // at 176. A full insert shows no tile. On a 394 px desk (a 400 px panel: the view's foot at
    // 392) the lines scroll 440 − 392 = 48 px, the tile then at 356 − 48 = 308. (Pins moved:
    // with the compact strip, the strips' foot 2 px over the desk's, was 4: the view to 502, was
    // 500, and 74 px of scroll; with the blocks up from 52 to 28, every line 24 px higher and 24
    // px less to scroll, 72.)
    #[test]
    fn blocks_flow_left_to_right_in_lines_as_wide_as_the_area() {
        let at = |x, y, size: Size| Rectangle::new(Point::new(x, y), size);
        let (eq, delay, filter, tile) =
            (block_size(9), block_size(4), block_size(3), block_size(0));
        let design = desk(DESIGN, InsertId(16));
        assert_eq!(
            blocks_view(design),
            at(674.0, 28.0, Size::new(312.0, 474.0))
        );
        assert_eq!(
            chain_rects(&CHAIN, design),
            [
                at(674.0, 28.0, eq),
                at(674.0, 176.0, delay),
                at(674.0, 266.0, filter),
                at(674.0, 356.0, tile)
            ]
        );
        assert_eq!(max_effects_scroll(&CHAIN, design), 0.0);
        let wide = Desk {
            size: Size::new(1920.0, DESIGN),
            ..design
        };
        assert_eq!(
            chain_rects(&CHAIN, wide),
            [
                at(1074.0, 28.0, eq),
                at(1392.0, 28.0, delay),
                at(1650.0, 28.0, filter),
                at(1074.0, 176.0, tile)
            ]
        );
        assert_eq!(chain_rects(&[2; 10], wide).len(), 10, "no tile when full");
        let taller = desk(desk_height(400.0), InsertId(16));
        assert_eq!(max_effects_scroll(&CHAIN, taller), 48.0);
        let scrolled = Desk {
            effects_scroll: 500.0,
            ..taller
        };
        assert_eq!(
            effects_scroll(&CHAIN, scrolled),
            48.0,
            "no further than the end"
        );
        assert_eq!(chain_rects(&CHAIN, scrolled)[3].y, 308.0);
    }

    // By hand, an effect just added ([`SHOW_ADDED`]) on the least desk (229 since the route
    // button is 24 px tall: the view 28..227, 199 px) with sixteen inserts: at 1000 px the view
    // is x 674..986, one block a line; at 1620 x 1074..1606 and at 1920 x 1074..1906. A line's
    // top is the scroll that puts it at 28; a line's top past the lines' end is no stop.
    // - 1620, delay, reverb, EQ (4, 4, 10 knobs): the first line 28..112, the EQ (312 x 142) and
    //   the tile at 118; the EQ's head (118..140) and the whole tile (to 202) show from the top:
    //   0 (the lines' end, 260 - 227 = 33, would cut the first line to its last 51 px).
    // - 1000, the delay: the tile at 118..202 shows whole from the top: 0.
    // - 1000, delay and reverb: from the top the reverb (118..202) shows whole and the tile at
    //   208 only 19 px, under a head's 22; the reverb's line (90) is past the end, 292 - 227 =
    //   65, so no line shows both and the first that shows the head stays: 0.
    // - 1000, three EQs (148 apart, at 28, 176 and 324): the third's head shows from the
    //   second's line (148, at 176..198), but the tile then lies at 472 - 148 = 324, out of
    //   view; from its own line (296) the tile (472 - 296 = 176) shows 51 px, over a head's 22:
    //   296, the tile drawn and pressed (the end, 556 - 227 = 329, would cut the EQ's head).
    // - 1920, two EQs then the delay beside the tile at 176: from the top the delay's head
    //   (176..198) and the tile's top 51 px show: 0.
    // - 1000, ten blocks of two knobs (192 x 84, no tile, at 28 + 90k): the last at 838 shows
    //   its head from its line's top, 810, which is past the end (922 - 227 = 695), and from no
    //   line's top before it (630's leaves it at 208, its head to 230): the end, 695, where it
    //   shows at 143.
    // - The 394 px desk (28..392), three delays: the tile at 298 shows whole, to 382: 0.
    // (Pins moved with the blocks up from 52 to 28: the view 175 px tall, the lines' ends 57,
    // 89, 353 and 719, the tile's 27 px; on the 219 px desk before the route button grew the
    // reverb's line showed both and the two EQs' case stopped at the end, 67.)
    #[test]
    fn an_added_effect_shows_by_whole_lines_from_the_first_block() {
        let least = Desk {
            effects_scroll: SHOW_ADDED,
            ..desk(LEAST_DESK, InsertId(16))
        };
        let wide = |width| Desk {
            size: Size::new(width, LEAST_DESK),
            ..least
        };
        assert_eq!(effects_scroll(&[4, 4, 10], wide(1620.0)), 0.0);
        assert_eq!(chain_rects(&[4, 4, 10], wide(1620.0))[0].y, 28.0);
        assert_eq!(effects_scroll(&[4], least), 0.0);
        assert_eq!(effects_scroll(&[4, 4], least), 0.0);
        assert_eq!(effects_scroll(&[10, 10, 10], least), 296.0);
        assert_eq!(effects_scroll(&[10, 10, 4], wide(1920.0)), 0.0);
        assert_eq!(effects_scroll(&[2; 10], least), 695.0);
        assert_eq!(effects_scroll(&[], least), 0.0);
        let taller = Desk {
            effects_scroll: SHOW_ADDED,
            ..desk(desk_height(400.0), InsertId(16))
        };
        assert_eq!(effects_scroll(&[4, 4, 4], taller), 0.0);
        // The wheel takes it from there: a scroll of 138 (a notch past 90) holds at the end, 65.
        assert_eq!(
            effects_scroll(
                &[4, 4],
                Desk {
                    effects_scroll: 138.0,
                    ..least
                }
            ),
            65.0,
            "no further than the end"
        );
    }

    // By hand, the add tile (192 x 84) cut by the view's foot (28..227 on the least desk): under
    // the delay at 1000 px it is at (674, 118) and shows whole (to 202); at 152 it shows its top
    // 75 px, drawn and pressed there; under three EQs scrolled to 296 it is at 176 and shows
    // 51 px; at 200, 27 px, over a block head's 22, drawn and pressed; at 210, 17 px, under a
    // head's 22: neither drawn nor pressed. Whole at the view's top, it shows whole. (Pins
    // moved with the blocks up from 52 to 28: under the delay at 142, under the EQs at 200.)
    #[test]
    fn the_add_tile_shows_down_to_the_views_foot_while_a_head_of_it_shows() {
        let least = desk(LEAST_DESK, InsertId(16));
        let view = blocks_view(least);
        let tile = |y| Rectangle::new(Point::new(674.0, y), Size::new(192.0, 84.0));
        assert_eq!(shown_tile(tile(118.0), view), Some(tile(118.0)));
        let cut = |y, height| Rectangle::new(Point::new(674.0, y), Size::new(192.0, height));
        assert_eq!(shown_tile(tile(152.0), view), Some(cut(152.0, 75.0)));
        assert_eq!(shown_tile(tile(176.0), view), Some(cut(176.0, 51.0)));
        assert_eq!(shown_tile(tile(200.0), view), Some(cut(200.0, 27.0)));
        assert_eq!(shown_tile(tile(210.0), view), None);
        assert_eq!(shown_tile(tile(28.0), view), Some(tile(28.0)));
        assert_eq!(
            hit_test(Point::new(770.0, 200.0), least, &[4]),
            Some(Hit::AddEffect)
        );
        let scrolled = Desk {
            effects_scroll: SHOW_ADDED,
            ..least
        };
        assert_eq!(chain_rects(&[10, 10, 10], scrolled)[3].y, 176.0);
        assert_eq!(
            hit_test(Point::new(770.0, 210.0), scrolled, &[10, 10, 10]),
            Some(Hit::AddEffect)
        );
    }

    // By hand, the nine-knob block at (674, 28), 312 wide (the first block, at the view's top):
    // its light at (674 + 12, 28 + 11); its remove cross a 24 px target across the head's 22 px
    // at its right end (986 − 24 = 962..986), its arrows' two 24 px targets left of it
    // (› 938..962, ‹ 914..938), each glyph drawn in its target's middle (the cross at 974, as
    // before, the arrows at 950 and 926); its name between the light and the arrows (698..912,
    // 2 px short of ‹); its first knob's place 6 px in under the head (680, 50), the knob 28 px
    // centered across it from 14 px down (696, 64), its label's middle 7 px down and its value's
    // 50; its seventh knob is the second of the second row (740, 108); a choice's button is 4 px
    // in from its place's sides, 18 px tall, on the knob's middle (69..87). (Pins moved: the
    // block stood at 52, every y 24 lower; the cross was 16 px square at 966..982, 3 px down, the
    // arrows 14 px wide 2 px left of it (› 950..964, ‹ 936..950), the name to 934.)
    #[test]
    fn a_blocks_parts_sit_where_its_head_and_rows_put_them() {
        let block = Rectangle::new(Point::new(674.0, 28.0), block_size(9));
        assert_eq!(effect_led_center(block), Point::new(686.0, 39.0));
        assert_eq!(
            remove_rect(block),
            Rectangle::new(Point::new(962.0, 28.0), Size::new(24.0, 22.0))
        );
        assert_eq!(remove_rect(block).center(), Point::new(974.0, 39.0));
        assert_eq!(
            later_rect(block),
            Rectangle::new(Point::new(938.0, 28.0), Size::new(24.0, 22.0))
        );
        assert_eq!(
            earlier_rect(block),
            Rectangle::new(Point::new(914.0, 28.0), Size::new(24.0, 22.0))
        );
        assert_eq!(
            effect_name_rect(block),
            Rectangle::new(Point::new(698.0, 28.0), Size::new(214.0, 22.0))
        );
        let first = knob_cell(block, 9, 0);
        assert_eq!(
            first,
            Rectangle::new(Point::new(680.0, 50.0), Size::new(60.0, 58.0))
        );
        assert_eq!(knob_rect(first).position(), Point::new(696.0, 64.0));
        assert_eq!(knob_label_center(first), Point::new(710.0, 57.0));
        assert_eq!(knob_value_center(first), Point::new(710.0, 100.0));
        assert_eq!(knob_cell(block, 9, 6).position(), Point::new(740.0, 108.0));
        assert_eq!(
            choice_rect(first),
            Rectangle::new(Point::new(684.0, 69.0), Size::new(52.0, 18.0))
        );
        assert_eq!(menu_anchor(block), Point::new(680.0, 50.0));
    }

    // By hand, the compact strip: the least desk is 96 (the fader's top) + 50 (its
    // track) + 83 (under it: the route button 9 px down and 24 tall, 4 px, the send knob 24, 2 px,
    // the label 16, the body's 2 px and the desk's 2) = 229 px, the default panel's (235 = 6 +
    // 229), and no panel is laid out shorter. On it the fader's foot is at 146, the route button
    // at 155..179, the send knob at 183..207, the label at 209..225 and the body's foot at 227.
    // A 300 px desk gives the track 300 − 83 − 96 = 121 px, the controls moving down with its
    // foot (217: the route button at 226). A 400 px panel's 394 px desk is past the 379 px where
    // the track reaches its most, 200 px (96..296): the route button stays at 305 and the send
    // knob at 333..357, and the label (374..390) and the body's foot (392) keep 2 px over the
    // desk's. (Before: a 304 px least desk with a 100 px track from 114, the label 4 px over a
    // body 4 px over the desk's; then 219 px with a 14 px route button, until the 24 px
    // targets.)
    #[test]
    fn a_shorter_desk_shortens_the_fader_and_keeps_the_controls() {
        assert_eq!(LEAST_DESK, 229.0);
        assert_eq!((LEAST_HEIGHT, DEFAULT_HEIGHT), (235.0, 235.0));
        assert_eq!(desk_height(DEFAULT_HEIGHT), LEAST_DESK);
        assert_eq!(desk_height(180.0), LEAST_DESK);
        assert_eq!(
            (fader_bottom(LEAST_DESK), fader_span(LEAST_DESK)),
            (146.0, 50.0)
        );
        assert_eq!(cap_center_y(1.0, LEAST_DESK), FADER_TOP);
        assert_eq!(cap_center_y(0.0, LEAST_DESK), 146.0);
        let route = route_rect(4.0, LEAST_DESK);
        assert_eq!((route.y, route.y + route.height), (155.0, 179.0));
        let send = send_rect(4.0, LEAST_DESK);
        assert_eq!((send.y, send.y + send.height), (183.0, 207.0));
        assert_eq!(
            (label_top(LEAST_DESK), body_bottom(LEAST_DESK)),
            (209.0, 227.0)
        );
        assert_eq!((fader_bottom(300.0), fader_span(300.0)), (217.0, 121.0));
        assert_eq!(route_rect(4.0, 300.0).y, 226.0);
        let taller = desk_height(400.0);
        assert_eq!(taller, 394.0);
        assert_eq!((fader_bottom(taller), fader_span(taller)), (296.0, 200.0));
        assert_eq!(route_rect(4.0, taller).y, 305.0);
        let send = send_rect(4.0, taller);
        assert_eq!((send.y, send.y + send.height), (333.0, 357.0));
        assert_eq!((label_top(taller), body_bottom(taller)), (374.0, 392.0));
        assert_eq!(fader_span(379.0), 200.0, "its most from 379 px");
        assert_eq!(fader_span(378.0), 199.0);
        assert_eq!(fader_span(700.0), 200.0);
    }

    // The compact strip keeps every part whole inside its body, one under the other with no
    // overlap, on the least desk and on a tall one, for the master (its clip light) and an
    // insert: the name (28..44) over the mute light (46..56; its solo ring, 6.75 px round it,
    // 44.25..57.75), over the pan knob (58..86), over the clip light (87..93, beside the knob's
    // round foot), over the meters' top (96); the fader's press area (the cap's travel, 5 px past
    // either end of the track: 91..151 on the least desk) under the knob's foot and over the
    // route button (155..179), over the send knob (183..207), over the label (209..225), over
    // the body's foot (227), over the desk's (229). The meters are as tall as the track: 50 px.
    #[test]
    fn the_compact_strips_parts_lie_whole_inside_it_one_under_the_other() {
        let inside = |part: Rectangle, body: Rectangle| {
            part.x >= body.x
                && part.x + part.width <= body.x + body.width
                && part.y >= body.y
                && part.y + part.height <= body.y + body.height
        };
        let around = |center: Point, radius: f32| {
            Rectangle::new(
                Point::new(center.x - radius, center.y - radius),
                Size::new(2.0 * radius, 2.0 * radius),
            )
        };
        for desk in [LEAST_DESK, 300.0, DESIGN] {
            for id in [InsertId::MASTER, InsertId(3)] {
                let left = strip_left(id, 0.0);
                let body = body_rect(left, desk);
                let label = Rectangle::new(
                    Point::new(left, label_top(desk)),
                    Size::new(STRIP_WIDTH, LABEL_HEIGHT),
                );
                let clip = around(Point::new(left + CLIP_OFFSET, CLIP_Y), CLIP_RADIUS);
                let solo = around(led_center(left), LED_RADIUS + 1.75);
                // Top to bottom down the strip's middle column (the light, the knob, the fader,
                // the route button, the send knob, the label).
                let column = [
                    name_rect(left),
                    solo,
                    pan_rect(left),
                    fader_rect(left, desk),
                    route_rect(left, desk),
                    send_rect(left, desk),
                    label,
                ];
                for part in column {
                    assert!(inside(part, body), "{part:?} outside {body:?} at {desk}");
                }
                for pair in column.windows(2) {
                    let (upper, lower) = (pair[0], pair[1]);
                    assert!(
                        upper.y + upper.height <= lower.y,
                        "{upper:?} runs into {lower:?} at {desk}"
                    );
                }
                for side in 0..2 {
                    let meter = meter_rect(left, side, desk);
                    assert!(inside(meter, body), "{meter:?} at {desk}");
                    assert_eq!(meter.height, fader_span(desk));
                }
                assert!(inside(clip, body));
                let meters_top = meter_rect(left, 0, desk).y;
                assert!(
                    clip.y + clip.height < meters_top,
                    "the clip light over the meters"
                );
                // The clip light beside the knob's round foot: its center 14 px right of the
                // knob's and 18 under it, further than the two radii.
                let knob = pan_rect(left).center();
                assert!(knob.distance(clip.center()) > PAN_SIZE / 2.0 + CLIP_RADIUS);
                assert_eq!(body.y + body.height, desk - BODY_FOOT);
            }
        }
        assert_eq!(body_rect(4.0, LEAST_DESK).y, 26.0);
        assert_eq!(fader_rect(4.0, LEAST_DESK).y, 91.0);
        assert_eq!(fader_rect(4.0, LEAST_DESK).height, 60.0);
        assert_eq!(meter_rect(4.0, 0, LEAST_DESK).height, 50.0);
    }

    #[test]
    fn meters_fill_from_minus_60_to_plus_6_db() {
        let span = 200.0;
        assert_eq!(meter_height(0.0, span), 0.0);
        assert_eq!(
            meter_height(0.000_1, span),
            0.0,
            "−80 dB is below the floor"
        );
        assert!((meter_height(1.0, span) - 200.0 * 60.0 / 66.0).abs() < 1e-4);
        assert_eq!(meter_height(100.0, span), 200.0, "+40 dB pins at the top");
        assert!((meter_height(0.5, span) - 200.0 * (60.0 - 6.0206) / 66.0).abs() < 0.01);
        assert_eq!(meter_fraction(100.0), 1.0);
        assert!((meter_height(1.0, 100.0) - 100.0 * 60.0 / 66.0).abs() < 1e-4);
    }

    #[test]
    fn presses_land_on_the_control_drawn_there() {
        let last = InsertId(16);
        let at = |x, y| hit_test(window(x, y), desk(DESIGN, last), &CHAIN);
        // The pop-out button on the 1000 px desk: x 1000 − 12 − 24 = 964..988, the header's
        // 24 px (window y 56..80 under the design's toolbar).
        assert_eq!(
            pop_out_rect(1000.0),
            Rectangle::new(Point::new(964.0, 0.0), Size::new(24.0, 24.0))
        );
        assert_eq!(at(970.0, 68.0), Some(Hit::PopOut));
        assert_eq!(at(963.0, 68.0), None, "left of it");
        assert_eq!(at(989.0, 68.0), None, "in the padding right of it");
        // The routing control, 230 px, centred in the header: (width − 230) / 2, by hand x 385
        // on the 1000 px desk (its "−" 385..409, its label to 591, its "+" to 615), 845 on a
        // 1920 px one, 235 on the 700 px one of a 1000 px window with the Library open (to 465,
        // the button at 664). Never nearer the title than its gap: "Mixer" from 12, 37.112 px at
        // 14 px (Inter's advances, gen_u1_expected.py's rule: 14 × 5429 / 2048), then 48, so
        // 97.112 on a desk under 97.112 · 2 + 230 = 424.224 px (375.2: its centre 72.6, held at
        // 97.112). On a desk under 97.112 + 230 + 12 + 24 + 12 = 375.112 px it ends 12 px left of
        // the button: at 300 px, 264 − 12 − 230 = 22. (Pins moved: it stood at 97.112..327.112,
        // the title gap past the title, on every desk with room; 61.112 with the 12 px gap;
        // 560..790 before a title gap.)
        let near = |found: f32, wanted: f32| (found - wanted).abs() < 1e-3;
        for (width, left) in [
            (1000.0, 385.0),
            (1920.0, 845.0),
            (700.0, 235.0),
            (424.224, 97.112),
            (375.2, 97.112),
            (300.0, 22.0),
        ] {
            assert!(
                near(route_left(width), left),
                "{width}: {}",
                route_left(width)
            );
        }
        assert_eq!(pop_out_rect(700.0).x, 664.0);
        let up = route_up_rect(700.0);
        assert!(near(up.x + up.width, 465.0));
        assert_eq!(at(390.0, 68.0), Some(Hit::RouteDown));
        assert_eq!(at(600.0, 68.0), Some(Hit::RouteUp));
        assert_eq!(at(500.0, 68.0), Some(Hit::RouteLabel));
        assert_eq!(at(40.0, 68.0), None, "the title");
        assert_eq!(at(100.0, 68.0), None, "where the control stood");
        assert_eq!(at(380.0, 68.0), None, "left of it");
        assert_eq!(at(620.0, 68.0), None, "right of it");
        // Insert 3 at x 198, the design's window y (mixer y + 56) of the compact strip on the
        // 504 px desk: light at (210, 107), pan 214..242 × 114..142, fader 199..227 over 147..357
        // (the cap's travel), route button 214..242 × 361..375, send knob 216..240 × 379..403,
        // name 88..104; the master's clip light at (48, 146). (Pins moved with the
        // compact strip: were 116, 126..154, 370 on the fader, 391 the route, 415 the send, 162.)
        assert_eq!(at(210.0, 107.0), Some(Hit::Led(InsertId(3))));
        assert_eq!(at(228.0, 128.0), Some(Hit::Pan(InsertId(3))));
        assert_eq!(at(213.0, 210.0), Some(Hit::Fader(InsertId(3))));
        assert_eq!(at(213.0, 350.0), Some(Hit::Fader(InsertId(3))));
        assert_eq!(at(228.0, 368.0), Some(Hit::Route(InsertId(3))));
        assert_eq!(at(228.0, 391.0), Some(Hit::Send(InsertId(3))));
        assert_eq!(at(228.0, 96.0), Some(Hit::Name(InsertId(3))));
        assert_eq!(at(240.0, 500.0), Some(Hit::Strip(InsertId(3))));
        assert_eq!(at(48.0, 146.0), Some(Hit::ClipLight));
        // The gap between strips.
        assert_eq!(at(259.0, 300.0), None);
        // Insert 10 (x 632..692) runs under the effects area from x 668: left of it the strip,
        // its name row (window y 96, mixer 40) past it the first block's head (28..50), 6 px left
        // of its light's middle (686, 39), inside the light's 8. (Pin moved: with the blocks at
        // 52 the name row past x 668 was the empty band over them, which took no presses.)
        assert_eq!(at(666.0, 300.0), Some(Hit::Strip(InsertId(10))));
        assert_eq!(at(666.0, 96.0), Some(Hit::Name(InsertId(10))));
        assert_eq!(at(680.0, 96.0), Some(Hit::EffectLed(0)));
        // The blocks (mixer y, as `blocks_flow_left_to_right_in_lines_as_wide_as_the_area`
        // places them: the nine-knob block at (674, 28), the delay at (674, 176), the tile at
        // (674, 356)): the nine-knob block's light, remove cross (its 24 px target to the block's
        // right end, 962..986: 963 and 985 are the cross, as 974 is), name, first knob and
        // seventh; its second row's empty fifth place; the delay's first knob; the gap right of
        // the delay; the tile. (Pins moved 24 px up with the blocks, from 52.)
        let at = |x, y| hit_test(Point::new(x, y), desk(DESIGN, last), &CHAIN);
        assert_eq!(at(686.0, 39.0), Some(Hit::EffectLed(0)));
        for x in [963.0, 974.0, 985.0] {
            assert_eq!(at(x, 39.0), Some(Hit::RemoveEffect(0)), "{x}");
        }
        assert_eq!(at(800.0, 39.0), Some(Hit::EffectName(0)));
        // The nine-knob block's arrows: › (938..962) moves it later; the first has no ‹, its
        // place (914..938) the block's. The delay's (926 its right end: its cross 902..926,
        // › 878..902, ‹ 854..878) both, each pressed at its middle and at its ends' pixels.
        assert_eq!(at(950.0, 39.0), Some(Hit::MoveLater(0)));
        assert_eq!(at(926.0, 39.0), Some(Hit::Block(0)));
        for x in [855.0, 866.0, 877.0] {
            assert_eq!(at(x, 187.0), Some(Hit::MoveEarlier(1)), "{x}");
        }
        for x in [879.0, 890.0, 901.0] {
            assert_eq!(at(x, 187.0), Some(Hit::MoveLater(1)), "{x}");
        }
        assert_eq!(at(914.0, 187.0), Some(Hit::RemoveEffect(1)));
        assert_eq!(at(710.0, 76.0), Some(Hit::Knob { block: 0, knob: 0 }));
        assert_eq!(at(770.0, 136.0), Some(Hit::Knob { block: 0, knob: 6 }));
        assert_eq!(at(950.0, 136.0), Some(Hit::Block(0)));
        assert_eq!(at(710.0, 226.0), Some(Hit::Knob { block: 1, knob: 0 }));
        assert_eq!(at(930.0, 226.0), None);
        assert_eq!(at(700.0, 396.0), Some(Hit::AddEffect));
        assert_eq!(at(990.0, 76.0), None, "the marks' margin");
    }

    // By hand, the chain at 1000 × 504: the nine-knob block (674, 28; its middle at x 830, its line to 170),
    // the delay (674, 176; middle 800, line to 260), the filter (674, 266; middle 770, line to
    // 350). That block dragged to the delay's line left of its middle stays first; right of it, it
    // takes the delay's place (the delay first now, at 28, and it on the line from 118,
    // above the filter, whose line the pointer is not on). Dragged onto the filter's line left of
    // its middle it passes the delay (whose line the pointer is below) and stops; right of it it
    // passes both. The filter dragged up to the first line left of the first block's middle comes
    // first. (Pins moved 24 px up with the blocks, from 52.)
    #[test]
    fn a_dragged_block_takes_a_neighbours_place_past_its_middle() {
        let design = desk(DESIGN, InsertId(16));
        let to = |place, x, y| reorder_place(&CHAIN, design, place, Point::new(x, y));
        assert_eq!(to(0, 700.0, 226.0), 0);
        assert_eq!(to(0, 810.0, 226.0), 1);
        assert_eq!(to(0, 700.0, 306.0), 1);
        assert_eq!(to(0, 780.0, 306.0), 2);
        assert_eq!(to(2, 700.0, 36.0), 0);
        assert_eq!(to(1, 750.0, 216.0), 1, "on its own place");
    }

    // On a 394 px desk (a 400 px panel) the fader's track is at its most, 96..296: the route
    // button at mixer y 305..319, the send knob at 323..347, the fader's press area to 301;
    // unscrolled, the filter's first knob's place is at 266 + 22 = 288..346. The effects' lines
    // scrolled to their end (440 − 392 = 48 px: the view's foot at 392): the add tile at
    // 356 − 48 = 308..392, the first block's head (28 − 48 = −20..2) out of view, its first row
    // (2..60) not whole in the view from 28 (a press there is the block's) and its second row at
    // 60..118; a scroll past the end shows as the end; above the view (y 26, 2 px under the
    // header) nothing. (Pins moved: with the compact strip the route button was at 318..332, the
    // send knob at 338..362, the scroll's end 74; with the blocks up from 52 to 28 the end 72,
    // the filter's knob at 312..370, and "above the view" was y 45.) On the least desk, 219 px:
    // insert 3's fader foot at 146, its press area to 151, the route button at 155..169 and the
    // send knob at 173..197.
    #[test]
    fn presses_on_a_shorter_desk_find_its_controls_and_scrolled_blocks() {
        let last = InsertId(16);
        let taller = desk_height(400.0);
        let at = |x, y, effects_scroll| {
            let desk = Desk {
                effects_scroll,
                ..desk(taller, last)
            };
            hit_test(Point::new(x, y), desk, &CHAIN)
        };
        assert_eq!(at(228.0, 312.0, 0.0), Some(Hit::Route(InsertId(3))));
        assert_eq!(at(228.0, 335.0, 0.0), Some(Hit::Send(InsertId(3))));
        assert_eq!(at(213.0, 290.0, 0.0), Some(Hit::Fader(InsertId(3))));
        assert_eq!(at(700.0, 340.0, 0.0), Some(Hit::Knob { block: 2, knob: 0 }));
        assert_eq!(at(700.0, 350.0, 48.0), Some(Hit::AddEffect));
        assert_eq!(at(700.0, 350.0, 500.0), Some(Hit::AddEffect));
        assert_eq!(at(710.0, 55.0, 48.0), Some(Hit::Block(0)));
        assert_eq!(at(710.0, 80.0, 48.0), Some(Hit::Knob { block: 0, knob: 5 }));
        assert_eq!(at(686.0, 26.0, 48.0), None, "above the view");
        let least = |x, y| hit_test(Point::new(x, y), desk(LEAST_DESK, last), &CHAIN);
        assert_eq!(least(213.0, 150.0), Some(Hit::Fader(InsertId(3))));
        assert_eq!(least(228.0, 162.0), Some(Hit::Route(InsertId(3))));
        assert_eq!(least(228.0, 185.0), Some(Hit::Send(InsertId(3))));
        assert_eq!(
            least(228.0, 207.0),
            Some(Hit::Strip(InsertId(3))),
            "the label"
        );
    }

    #[test]
    fn the_strips_scroll_until_the_last_one_meets_the_effects_area() {
        // Insert 16's right edge is 12 + 62·16 + 60 + 4 = 1068; the effects area starts at 668.
        assert_eq!(max_scroll(InsertId(16), WIDTH), 400.0);
        assert_eq!(max_scroll(InsertId(2), WIDTH), 0.0);
    }
}
