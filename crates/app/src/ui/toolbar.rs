//! The toolbar along the top of the window, in groups a thin line apart: the transport (Play,
//! Stop, Record with the input meter under it, beside it what a take records, Notes over Audio,
//! over Capture, then the metronome) | the tempo | the pattern (its
//! arrows and name) | the play mode (Pat over Song, then Loop) | the play position | the
//! computer keyboard (its labeled switch, the octave's + and − with its name between) | the
//! panel buttons | the notices, on one short line, and Retry while audio is off | File, Edit and
//! Help at the right end (File first since 2026-10-27: Edit was first).
//!
//! Every group's width is fixed by the toolbar itself (each button sized to its label measured,
//! the readouts and the name box set; the notices' line its text, cut to [`NOTICE_MOST`]) at the
//! toolbar's scale ([`Sizes`]): 1 in a window with no more room than everything the bar can show
//! at once needs, more in a wider one, up to [`SCALE_MOST`], each control's width, padding, icon
//! and text growing with it (the text only as far as its row holds) while the heights stay. What
//! the window has to spare past that, the gaps between groups share equally, the line in each
//! gap's middle, until the controls are at their most; from there the groups up to the panel
//! buttons stand to the left a gap of [`PACKED_EXTRA`] apart and the rest goes to the notices'
//! cell before File, Edit and Help ([`Spread`]). Every group is [`BUTTON_HEIGHT`] tall and centered in the bar, and so
//! are the lines between them, so each group fills the cell its lines bound: the readouts, the
//! pattern's arrows and name (one piece in one frame), Pat over Song (a two-row switch) beside
//! Loop, the keyboard switch on one line beside the octave's column, the panel buttons' two
//! rows, the notices' line and File, Edit and Help. So where each control lies is a function of the window's width and of
//! what shows beside the fixed groups ([`Shape`]), which the pixel probes, the menus and the
//! first-run note read. When the window is too narrow, the panel buttons fold away first, as
//! before; then the labels shorten ([`Tier::Short`]: the keyboard switch says only On or Off,
//! the pattern's name box narrows).

use std::borrow::Cow;

use dawsome_engine::project::{PatternId, TEMPO_RANGE};
use dawsome_engine::time::{self, MilliBpm, Ticks};
use iced::alignment::Horizontal;
use iced::widget::text::Wrapping;
use iced::widget::{
    button, canvas, column, container, mouse_area, row, text, text_input, tooltip, Id,
};
use iced::{mouse, Center, Color, Element, Fill, Font, Point, Rectangle, Renderer, Size, Theme};

use crate::app::{
    FileMessage, Message, PatternMessage, PlayMode, RecordMessage, TakeKind, PATTERN_RENAME_INPUT,
};
use crate::keys::{Action, Keys};
use crate::ui::hint::{hover_area, with_tip, Hover, Tool};
use crate::ui::layout::{TOOLBAR_HEIGHT, TOOLBAR_SIDE_PADDING};
use crate::ui::mixer::MINUS_6_DB;
use crate::ui::mixer_layout;
use crate::ui::panels::{Panel, Panels};
use crate::ui::style;
use crate::ui::text_fit::{fit, text_width};

/// The least space either side of the line between two of the toolbar's groups, and the line.
/// Why this tight: at 16 px text and 42 px transport buttons everything fitted 1000 px with the
/// panel buttons and a notice's room only so. Kept when the controls became 0.9 of their size
/// (asked 2026-10-27: "about 10 % smaller both ways, spacing kept"). A wider window gives every gap the same share of
/// what it has to spare ([`Spread`]). 2 px (was 3), so the 1000 px window
/// keeps the panel buttons beside a notice's room once the keyboard switch stood on one line and
/// the name's box held "Pattern 12" (28 px more); only a window that narrow shows the least gap.
/// The Help button right of File moved every fold by its 47.584 px, so at 1000 px
/// the panel buttons now fold beside a notice (see the fold test).
const BAR_SPACING: f32 = 2.0;
/// The line between two groups: as tall as the groups, so each group fills the cell its lines
/// bound: [`BUTTON_HEIGHT`] (was 44 px beside 52 px buttons and 60 px stacks, which read as
/// short boxes floating in tall cells).
const DIVIDER: Size = Size::new(1.0, BUTTON_HEIGHT);
/// The least gap between two groups: the space, the line, the space.
const GAP: f32 = 2.0 * BAR_SPACING + DIVIDER.width;
/// The line along the toolbar's foot, inside its height.
const EDGE: f32 = 1.0;
// The groups end above the line, centered in the bar.
const _: () = assert!((TOOLBAR_HEIGHT + BUTTON_HEIGHT) / 2.0 <= TOOLBAR_HEIGHT - EDGE);

/// The height of every group of the toolbar: its buttons, readouts, two-row switch, columns and
/// notices' line, centered in the bar, and of the lines between the groups: 46 px in the 64 px
/// bar, 9 px over and under (was 52 in 72, and 40 in 56 before that; the bar was asked
/// about 10 % smaller both ways: 46.8 would leave the panel grid's rows half a pixel), one height
/// for every group (the panel grid, Pat over Song and the octave column were 60 px), so they
/// read as one piece.
const BUTTON_HEIGHT: f32 = 46.0;
/// A row of the two-row groups (the panel buttons): two of them a [`PANEL_SPACING`] apart fill
/// [`BUTTON_HEIGHT`].
const HALF_ROW: f32 = (BUTTON_HEIGHT - PANEL_SPACING) / 2.0;

/// The most the toolbar's controls grow ([`Sizes::scale`]) in a wide window. Why 1.6: the bar's
/// height is fixed, so only widths, paddings, icons and text grow, and its short rows set the
/// limit: a row of the panel grid (25 px then) held a 19 px icon (12 × 1.6) with 3 px over and
/// under it, and a larger one would touch its button's edges; the transport's 42 × 52 px buttons
/// became 67 × 52 (38 × 46 → 61 × 46 at 0.9 of the size), wider than tall but not yet a flat
/// strip (2.0 read as wide pills, and with the text and
/// icons stopped at their rows' most it only padded them). Past it the groups stand to the left
/// and the notices' cell takes what the window has to spare ([`PACKED_EXTRA`]). Toolbar scale
/// (2026-10-24): at a full-screen window of 2259 px each group sat small in the middle
/// of a cell three or four times its width.
const SCALE_MOST: f32 = 1.6;
/// How many gaps the bar has with nothing but its groups (the transport to the panel buttons,
/// then File, Edit and Help: eight groups).
const PLAIN_GAPS: f32 = 7.0;
/// Each gap's width past [`GAP`] once the controls are at their most and the window has more to
/// spare than that: the share the bar with nothing but its groups gives each gap at the width
/// where the controls first reach their most (a notice of its most's room, [`NOTICE_MOST`], and
/// its gap, shared by the seven gaps in even pixels: 40 px, so 45 a gap). Past that the groups
/// up to the panel buttons keep this gap and stand to the left, and the gap before File, Edit and
/// Help takes the rest: the notices' cell, the notices left in it beside the panel buttons. Why:
/// a request made at full screen (2259 px): "push everything to the right of the
/// play/pause/record UI to the left, other than Edit and File, so that it fills the empty
/// spaces"; every group sat in the middle of a cell 123 px wider than itself. This gap, the one
/// the bar has where the cap is first reached (about 1722 px), keeps the bar's look there.
const PACKED_EXTRA: f32 = ((NOTICE_MOST + GAP) / PLAIN_GAPS / 2.0) as u32 as f32 * 2.0;
const _: () = assert!(PACKED_EXTRA == 40.0);
/// The scale goes up in twentieths ([`SCALE_PER_ONE`]), [`SCALE_STEPS`] of them to
/// [`SCALE_MOST`]: few enough that finding it measures little, fine enough that a window made
/// wider grows the controls a pixel or two at a time.
const SCALE_PER_ONE: u32 = 20;
const SCALE_STEPS: u32 = 12;
const _: () = assert!((SCALE_PER_ONE + SCALE_STEPS) as f32 / SCALE_PER_ONE as f32 == SCALE_MOST);

/// The transport's buttons: 38 px wide, [`BUTTON_HEIGHT`] tall, with 16 px glyphs, 2 px apart
/// (were 42 and 18 px: 0.9 of them, rounded, as every size of the bar). Why not squares: 4 × 10
/// px more left no room for the panel buttons at 1000 px.
const TRANSPORT_BUTTON: f32 = 38.0;
const TRANSPORT_ICON: f32 = 16.0;
const TRANSPORT_SPACING: f32 = 2.0;

/// The pattern's arrows: 13 px glyphs in 20 px wide buttons (were 14 in 22), joined to the
/// name's box between them into one piece (12 px glyphs 2 px apart from the box read as tiny
/// buttons beside it).
const ARROW_ICON: f32 = 13.0;
const ARROW_WIDTH: f32 = 20.0;

/// The edge of a piece made of parts (the pattern's arrows and its name, Pat over Song, Notes
/// over Audio): one line of [`style::BUTTON_EDGE`] round the piece and between its parts, where
/// each part drew its own edge before, which made 2 px seams between three boxes (on
/// 2026-10-27 the pattern's piece was called the ugliest part of the bar).
const FRAME: f32 = 1.0;
/// A side of a two-row switch in a group's height (Pat over Song, N over A): the height inside
/// the frame, halved.
const SWITCH_SIDE: f32 = (BUTTON_HEIGHT - 2.0 * FRAME) / 2.0;
/// A side of the switch over Capture, Notes over Audio, in its two rows.
const TAKE_SIDE_HEIGHT: f32 = TAKE_ROW - FRAME;

/// The readouts: digits of 16 px (were 18); the tempo holds "120.000", the position "10:01:00",
/// with 4 px either side. A readout grows as one piece with its digits, which stop at
/// [`READOUT_MOST`].
const READOUT_SIZE: f32 = 16.0;
const READOUT_MOST: f32 = 23.0;
const READOUT_PADDING: f32 = 4.0;
const TEMPO_WIDTH: f32 = 76.0;
const POSITION_WIDTH: f32 = 86.0;

/// The input meter under the Record button (was 60 × 6).
const INPUT_METER: Size = Size::new(54.0, 5.0);

/// The column right of Record (Recording v2): Notes over Audio, one switch saying what a take
/// records, then Capture, in rows of [`TAKE_ROW`] and [`CAPTURE_ROW`] the groups' height in all,
/// their words at [`TAKE_TEXT`]. Why always on the bar: a tick left in Record's menu, out of
/// sight, once turned note takes into audio takes for an evening. Why this small: 54 px was what
/// the 1000 px window had left beside the panel buttons; narrower ([`Column::Letters`]) the switch
/// says N and A in two rows and Capture folds into the Notes menu (and Ctrl+Shift+R). Rows of 14,
/// 14 and 16 px, words of 11 px (were 16, 16, 18 and 12).
const TAKE_ROW: f32 = 14.0;
const CAPTURE_ROW: f32 = BUTTON_HEIGHT - 2.0 * TAKE_ROW - PART_SPACING;
const TAKE_TEXT: f32 = 11.0;
/// The most [`TAKE_TEXT`] grows to: its 14 px rows hold no larger.
const TAKE_MOST: f32 = 12.0;
const TAKE_SIDE: f32 = 4.0;
/// The letter Record shows under its dot as [`Column::Badge`].
const BADGE_TEXT: f32 = 10.0;
const BADGE_MOST: f32 = 13.0;
const NOTES: &str = "Notes";
const AUDIO: &str = "Audio";
const CAPTURE: &str = "Capture";

/// The notices' text size: 14 px, the labels' (was 16, and 12 before that).
const NOTICE_SIZE: f32 = 14.0;
/// The most the notices' line takes: about 40 characters at 14 px (36 at the 16 px it was
/// set for). A longer line is cut with an ellipsis; the hint bar (and a tip) say it whole. Why a
/// bound: across a wide window the notices otherwise took the room the groups spread into. Why
/// not 0.9 of it with the rest: it is the room the bar keeps for what goes wrong, not a control,
/// and the gaps' share where the controls reach their most comes from it ([`PACKED_EXTRA`]),
/// which were asked kept.
pub const NOTICE_MOST: f32 = 288.0;
/// What the notices' line puts between two notices.
const NOTICE_JOIN: &str = " \u{b7} ";
/// The panel buttons' tips' text size, the other tips' (`hint::with_tip`).
const TIP_SIZE: f32 = 12.0;
/// Retry's text size (was the tips' 12), and the most it grows to (was 16).
const RETRY_SIZE: f32 = 11.0;
const SMALL_MOST: f32 = 14.0;

/// The button that tries the output device again.
const RETRY: &str = "Retry";

/// The File and Edit menus' buttons' labels (File first, then Edit, then Help: asked so on
/// 2026-10-27), the Help window's button's right of them (asked for there), and
/// the space between two of them.
const FILE: &str = "File";
const EDIT: &str = "Edit";
const MENU_SPACING: f32 = 4.0;

/// The text buttons' (Pat, Song, Loop, the keyboard switch, File, Edit, Help) text size and side
/// padding: 14 px text (was 16, and 15 before that) and 5 px sides (were 6), which pays for the
/// bigger text's width.
const LABEL_SIZE: f32 = 14.0;
const LABEL_SIDE: f32 = 5.0;
/// The most [`LABEL_SIZE`] grows to in the 46 px rows, and Pat's and Song's in their 23 px ones
/// (were 22 and 20 in 52 and 26 px).
const LABEL_MOST: f32 = 20.0;
const MODE_MOST: f32 = 18.0;

/// The keyboard switch's glyph (drawn at 18 × 12 and scaled up), and the space between it and
/// its words, on one line: "Keyboard on" or "Keyboard off" (they once wrapped onto
/// two lines, "Keyboard" over "on"). 19 × 13 and 5 px (were 21 × 14 and 6).
const KEYBOARD_GLYPH: Size = Size::new(19.0, 13.0);
const GLYPH_GAP: f32 = 5.0;
/// The keyboard glyph's drawing size, which [`KEYBOARD_GLYPH`] scales.
const KEYBOARD_DRAWN: f32 = 18.0;

/// The octave column: + over the octave's name over −, [`BUTTON_HEIGHT`] in all (was
/// 60 px, with a 24 px name): 23 px wide, rows of 16, 14 and 16 px (were 26 wide, 18, 16, 18).
const OCTAVE_WIDTH: f32 = 23.0;
const OCTAVE_BUTTON: f32 = 16.0;
const OCTAVE_NAME: f32 = BUTTON_HEIGHT - 2.0 * OCTAVE_BUTTON;
const OCTAVE_SIZE: f32 = 14.0;
const OCTAVE_NAME_SIZE: f32 = 11.0;
/// The most + and − grow to in their 16 px rows, and the name in its 14 px one.
const OCTAVE_MOST: f32 = 16.0;
const OCTAVE_NAME_MOST: f32 = 13.0;

/// The pattern name's box at [`Tier::Short`]: a name longer than the box shrinks, then is cut
/// with an ellipsis: 47 px (was 52, and 56 before that), so the short labels still fit 880 px
/// with Retry. At [`Tier::Full`] the box holds [`PATTERN_NAME_WHOLE`] at the labels' size and
/// [`PATTERN_NAME_HELD`] at [`NAME_LEAST`] ([`Sizes::pattern_name_width`]).
const PATTERN_NAME_SHORT: f32 = 47.0;
/// The name the full box holds at the labels' size: "Pattern 12" (the box was 84 px, which
/// held "Pattern 1" and cut the tenth pattern's name).
const PATTERN_NAME_WHOLE: &str = "Pattern 12";
/// A name of 24 characters, which the full box holds whole at [`NAME_LEAST`]: a longer name
/// than the labels' size holds shrinks a pixel at a time down to it, then is cut with an
/// ellipsis (the hint bar names it whole). Why 24: the name was asked to fit always
/// (2026-10-27); the box held 10 characters ("Pattern 12") and cut every name past them.
const PATTERN_NAME_HELD: &str = "Chorus bassline (take 2)";
/// The least the pattern's name shrinks to: 10 px with nothing to spare, the column's words'
/// size, up to 14 at the most scale.
const NAME_LEAST: f32 = 10.0;
const NAME_LEAST_MOST: f32 = 14.0;
const PATTERN_NAME_PADDING: f32 = 5.0;
/// The space between the parts of a group that are not one piece: Pat over Song and Loop, the
/// keyboard switch and the octave's column.
const PART_SPACING: f32 = 2.0;

/// The panel buttons: a grid of three by two, each an icon of [`PANEL_ICON`] with
/// [`PANEL_PADDING`] around it, [`PANEL_SPACING`] apart. Why a grid of small icons rather than a
/// row of named buttons: two rows of three fit the toolbar's height in 64 px, and each has a tip
/// naming its panel and keys: 12 px icons in 20 × 28 px buttons (were 10 px in 20 × 22), the
/// grid as wide as before; then 20 × 25 px, the two rows
/// [`BUTTON_HEIGHT`] together (was 58 px); now 11 px icons in 19 × 22 px buttons, a 61 px grid,
/// at 0.9 of the size; the icon a whole pixel from the top (5 px over it, 6 under), so its edges
/// stay sharp.
const PANEL_ICON: f32 = 11.0;
/// The most [`PANEL_ICON`] grows to: 3 px over and under it in its 22 px row (was 19 in 25).
const PANEL_ICON_MOST: f32 = 16.0;
/// The icons' drawing size, which [`PANEL_ICON`] scales.
const PANEL_ICON_DRAWN: f32 = 10.0;
const PANEL_PADDING: iced::Padding = iced::Padding {
    top: 5.0,
    right: 4.0,
    bottom: 6.0,
    left: 4.0,
};
const PANEL_SPACING: f32 = 2.0;
const PANEL_COLUMNS: usize = 3;
const PANEL_BUTTON_HEIGHT: f32 = PANEL_ICON + PANEL_PADDING.top + PANEL_PADDING.bottom;
const _: () = assert!(PANEL_BUTTON_HEIGHT == HALF_ROW);
const _: () = assert!(PANEL_BUTTON_HEIGHT - PANEL_ICON_MOST >= 6.0);

// The octave's column and the two rows of buttons are the groups' height.
const _: () = assert!(2.0 * PANEL_BUTTON_HEIGHT + PANEL_SPACING == BUTTON_HEIGHT);
const _: () = assert!(OCTAVE_NAME > 0.0 && BUTTON_HEIGHT < TOOLBAR_HEIGHT);

/// The least room the notices keep beside the panel buttons while there are notices: the panel
/// buttons fold away before a notice would have less. Why the notices first: they say what went
/// wrong, which is shown nowhere else, while every panel keeps its keys (1–6, F5–F10).
const NOTICE_ROOM: f32 = 48.0;

/// How the column beside Record shows: its words, Notes over Audio over Capture, or in a window
/// too narrow for them only the switch, N over A (Capture then in the Notes menu and on
/// Ctrl+Shift+R). It goes to letters after the panel buttons fold and before the labels shorten.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    Words,
    Letters,
    /// No column: Record says N or A itself.
    Badge,
}

/// How much the toolbar's labels say: all of them, or, in a window too narrow for that, the
/// keyboard switch only On or Off and the pattern's name in a narrower box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Full,
    Short,
}

/// A group of the toolbar, left to right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Transport,
    Tempo,
    Pattern,
    Mode,
    Position,
    Keyboard,
    /// The panel buttons, while there is room for them.
    Panels,
    /// The notices' line, then Retry, while either shows.
    Status,
    /// File, Edit and Help.
    Menus,
}

/// What decides where the toolbar's groups lie besides the labels: the window's width, Retry
/// (audio is off), the notices' line's own width (its text cut to [`NOTICE_MOST`]; none without
/// notices) and the input meter under Record.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape {
    pub width: f32,
    pub retry: bool,
    pub notices: Option<f32>,
    pub input_meter: bool,
}

/// Where the toolbar's groups lie for a [`Shape`]: the toolbar's scale and every size it gives
/// ([`Sizes`]), the labels' tier, whether the panel buttons show, the notices' line's width, and
/// each group with its width, left to right. The gaps
/// between groups are [`GAP`] each and share what the window has to spare equally, in even
/// whole pixels, so the line in each gap's middle has the same space on both sides (the last
/// one before File, Edit and Help takes what the rounding leaves, so Help ends at the right
/// padding).
/// Even (whole pixels before, which left one side a pixel wider). With the
/// controls at their most and a share over [`PACKED_EXTRA`] the bar is packed: every gap is
/// [`PACKED_EXTRA`] past [`GAP`] but the last, which takes the rest; its line stands beside File
/// while the notices or Retry show (their cell then runs from the panel buttons' line to it),
/// else beside the panel buttons, so every line keeps to its group.
#[derive(Debug, Clone, PartialEq)]
pub struct Spread {
    sizes: Sizes,
    pub tier: Tier,
    /// How the column beside Record shows.
    pub column: Column,
    pub panels: bool,
    /// The notices' line's width: its own, or the room left when that is less; 0 without.
    pub notice_width: f32,
    groups: Vec<(Group, f32)>,
    /// Each gap's width past [`GAP`], and the last gap's.
    extra: f32,
    last_extra: f32,
    /// The groups stand to the left, the last gap taking what the window has to spare.
    packed: bool,
}

impl Spread {
    pub fn of(shape: Shape) -> Self {
        let sizes = Sizes::at(scale_for(shape.width));
        let least = |(tier, column), panels, notice| {
            least_width(&groups(
                tier,
                column,
                panels,
                notice,
                shape.retry,
                shape.input_meter,
                &sizes,
            ))
        };
        // While there are notices, the full labels and then the panel buttons take only what
        // leaves the notices their least room. The column beside Record gives way first: its
        // words, then its letters, for the badge on Record (no width of its own), before the
        // panel buttons fold (with the badge every other group lies where it lay
        // before Recording v2, so the old widths and folds hold); then the labels shorten.
        let reserve = shape.notices.map(|natural| natural.min(NOTICE_ROOM));
        let looks = [
            (Tier::Full, Column::Words, true),
            (Tier::Full, Column::Letters, true),
            (Tier::Full, Column::Badge, true),
            (Tier::Full, Column::Badge, false),
            (Tier::Short, Column::Badge, true),
        ];
        let (tier, column, panels) = looks
            .into_iter()
            .find(|&(tier, column, panels)| least((tier, column), panels, reserve) <= shape.width)
            .unwrap_or((Tier::Short, Column::Badge, false));
        let look = (tier, column);
        let notice = shape.notices.map(|natural| {
            let room = shape.width - least(look, panels, Some(0.0));
            natural.min(room.max(0.0))
        });
        let groups = groups(
            tier,
            column,
            panels,
            notice,
            shape.retry,
            shape.input_meter,
            &sizes,
        );
        let spare = (shape.width - least_width(&groups)).max(0.0);
        let gaps = groups.len().saturating_sub(1).max(1) as f32;
        let share = (spare / gaps / 2.0).floor() * 2.0;
        let packed = sizes.scale == SCALE_MOST && share > PACKED_EXTRA;
        let extra = if packed { PACKED_EXTRA } else { share };
        Self {
            sizes,
            tier,
            column,
            panels,
            notice_width: notice.unwrap_or(0.0),
            extra,
            last_extra: spare - extra * (gaps - 1.0),
            packed,
            groups,
        }
    }

    /// The groups, left to right, with their widths.
    pub fn groups(&self) -> &[(Group, f32)] {
        &self.groups
    }

    /// Every size the toolbar's scale gives.
    pub fn sizes(&self) -> Sizes {
        self.sizes
    }

    /// The width of the gap after group number `index` past [`GAP`].
    fn extra_after(&self, index: usize) -> f32 {
        if index + 2 == self.groups.len() {
            self.last_extra
        } else {
            self.extra
        }
    }

    /// How far past group number `index` the line in the gap after it starts: in the gap's
    /// middle, on a whole pixel (the last gap's as near as its rest allows). Packed, the last
    /// gap's line stands its gap's half from File when the notices' cell is before it, else its
    /// half from the group before.
    fn line_after(&self, index: usize) -> f32 {
        let last = index + 2 == self.groups.len();
        let half = BAR_SPACING + (PACKED_EXTRA / 2.0).floor();
        match self.groups.get(index) {
            Some((Group::Status, _)) if self.packed && last => {
                GAP + self.last_extra - DIVIDER.width - half
            }
            _ if self.packed && last => half,
            _ => BAR_SPACING + (self.extra_after(index) / 2.0).floor(),
        }
    }

    /// Where `group` starts across the toolbar, while it shows.
    pub fn left(&self, group: Group) -> Option<f32> {
        let mut x = TOOLBAR_SIDE_PADDING;
        for (index, &(shown, width)) in self.groups.iter().enumerate() {
            if shown == group {
                return Some(x);
            }
            x += width + GAP + self.extra_after(index);
        }
        None
    }

    /// Where `group` lies across the toolbar, while it shows.
    #[cfg(test)]
    pub fn span(&self, group: Group) -> Option<std::ops::Range<f32>> {
        let width = self.groups.iter().find(|(shown, _)| *shown == group)?.1;
        let left = self.left(group)?;
        Some(left..left + width)
    }

    /// Where the line in each gap lies across the toolbar, as the view places it: a whole
    /// number of pixels past the group before it, as many as before the group after it (the
    /// last gap's as near as its rest allows).
    #[cfg(test)]
    pub(crate) fn divider_xs(&self) -> Vec<f32> {
        (0..self.groups.len().saturating_sub(1))
            .map(|index| {
                let (group, width) = self.groups[index];
                let left = self.left(group).expect("shown");
                left + width + self.line_after(index)
            })
            .collect()
    }

    /// Where the pattern name's box starts across the toolbar, where its menu opens: past the
    /// pattern's previous arrow, which it is joined to.
    pub fn pattern_name_x(&self) -> f32 {
        self.left(Group::Pattern).unwrap_or(0.0) + self.sizes.arrow_width
    }

    /// Where the computer keyboard's switch lies in the window. The first-run note points at it.
    pub fn keyboard_switch(&self) -> Rectangle {
        let x = self.left(Group::Keyboard).unwrap_or(0.0);
        Rectangle::new(
            Point::new(x, (TOOLBAR_HEIGHT - BUTTON_HEIGHT) / 2.0),
            Size::new(self.sizes.keyboard_switch_width(self.tier), BUTTON_HEIGHT),
        )
    }

    /// Where the Record button starts across the toolbar, where its menu opens: past the side
    /// padding, Play and Stop.
    pub fn record_x(&self) -> f32 {
        TOOLBAR_SIDE_PADDING + 2.0 * (self.sizes.transport_button + TRANSPORT_SPACING)
    }

    /// Where the Record button lies in the window. The first-run note on recording points at it.
    pub fn record_button(&self) -> Rectangle {
        Rectangle::new(
            Point::new(self.record_x(), (TOOLBAR_HEIGHT - BUTTON_HEIGHT) / 2.0),
            Size::new(self.sizes.transport_button, BUTTON_HEIGHT),
        )
    }

    /// Where the column right of Record starts across the toolbar, where its menus open.
    pub fn take_column_x(&self) -> f32 {
        self.record_x() + self.sizes.transport_button + TRANSPORT_SPACING
    }

    /// How far the Edit menu's right edge lies from the window's: under the Edit button's right
    /// edge, Help's width and the menus' spacing left of the toolbar's right padding.
    pub fn edit_menu_right(&self) -> f32 {
        TOOLBAR_SIDE_PADDING + self.sizes.label_button_width(help_label(self.tier)) + MENU_SPACING
    }

    /// How far the File menu's right edge lies from the window's: under the File button's right
    /// edge, Edit's and Help's widths and two of the menus' spacings left of the toolbar's right
    /// padding.
    pub fn file_menu_right(&self) -> f32 {
        self.edit_menu_right() + self.sizes.label_button_width(EDIT) + MENU_SPACING
    }
}

/// The toolbar's scale in a window `width` wide: the largest twentieth up to [`SCALE_MOST`] at
/// which the full labels, the words beside Record, the panel buttons and notices of their most
/// ([`NOTICE_MOST`]) fit with the least gaps; 1 where that does not fit even at 1. Why notices of
/// their most, not what shows: the scale then follows the width alone, so no notice coming (a
/// take's "Recording notes 0:04") changes a control's size or moves Record from under the
/// pointer, and a notice shows whole; Retry and the input meter (audio off, an armed input) take
/// their room from the notice's, which keeps at least its least. Every window up to where the
/// scale starts to grow (about 1290 px) lays the bar out as before.
fn scale_for(width: f32) -> f32 {
    let scale = |step: u32| (SCALE_PER_ONE + step) as f32 / SCALE_PER_ONE as f32;
    let fits = |step: u32| {
        let sizes = Sizes::at(scale(step));
        let all = groups(
            Tier::Full,
            Column::Words,
            true,
            Some(NOTICE_MOST),
            false,
            false,
            &sizes,
        );
        least_width(&all) <= width
    };
    // Why these two first: a window at either end (most windows) costs one or two measures of
    // the bar, as the view lays it out on every update.
    if !fits(0) {
        return 1.0;
    }
    if fits(SCALE_STEPS) {
        return SCALE_MOST;
    }
    // The widths only grow with the scale: the last step that fits, by halves (`low` fits,
    // `high` does not).
    let (mut low, mut high) = (0, SCALE_STEPS);
    while high - low > 1 {
        let middle = (low + high) / 2;
        if fits(middle) {
            low = middle;
        } else {
            high = middle;
        }
    }
    scale(low)
}

/// Every size of the toolbar's controls that grows with its scale: lengths in whole pixels
/// (the base length times the scale, rounded), text sizes too, each up to what its row holds;
/// the readouts grow as one piece with their digits. At a scale of 1 each is its constant's
/// value, so a window that has nothing to spare lays out as before the scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sizes {
    /// How much the controls grow: 1 to [`SCALE_MOST`].
    pub scale: f32,
    transport_button: f32,
    transport_icon: f32,
    input_meter: f32,
    arrow_width: f32,
    arrow_icon: f32,
    readout_text: f32,
    readout_padding: f32,
    tempo_width: f32,
    position_width: f32,
    take_text: f32,
    take_side: f32,
    badge_text: f32,
    label_text: f32,
    label_side: f32,
    mode_text: f32,
    retry_text: f32,
    keyboard_glyph: Size,
    glyph_gap: f32,
    octave_width: f32,
    octave_text: f32,
    octave_name_text: f32,
    pattern_name_padding: f32,
    pattern_name_short: f32,
    /// The least the pattern's name shrinks to ([`NAME_LEAST`]).
    name_least_text: f32,
    /// The panel buttons' icon's side.
    pub panel_icon: f32,
    panel_padding: iced::Padding,
}

impl Sizes {
    /// The sizes at `scale`.
    pub fn at(scale: f32) -> Self {
        let grown = |base: f32| (base * scale).round();
        let text = |base: f32, most: f32| (base * scale).min(most).round();
        let readout_text = text(READOUT_SIZE, READOUT_MOST);
        let readout = readout_text / READOUT_SIZE;
        let panel_icon = grown(PANEL_ICON).min(PANEL_ICON_MOST);
        // The icon a whole pixel from its button's top, the odd pixel under it.
        let over = ((PANEL_BUTTON_HEIGHT - panel_icon) / 2.0).floor();
        let panel_side = grown(PANEL_PADDING.left);
        Self {
            scale,
            transport_button: grown(TRANSPORT_BUTTON),
            transport_icon: grown(TRANSPORT_ICON),
            input_meter: grown(INPUT_METER.width),
            arrow_width: grown(ARROW_WIDTH),
            arrow_icon: grown(ARROW_ICON),
            readout_text,
            readout_padding: (READOUT_PADDING * readout).round(),
            tempo_width: (TEMPO_WIDTH * readout).ceil(),
            position_width: (POSITION_WIDTH * readout).ceil(),
            take_text: text(TAKE_TEXT, TAKE_MOST),
            take_side: grown(TAKE_SIDE),
            badge_text: text(BADGE_TEXT, BADGE_MOST),
            label_text: text(LABEL_SIZE, LABEL_MOST),
            label_side: grown(LABEL_SIDE),
            mode_text: text(LABEL_SIZE, MODE_MOST),
            retry_text: text(RETRY_SIZE, SMALL_MOST),
            keyboard_glyph: Size::new(grown(KEYBOARD_GLYPH.width), grown(KEYBOARD_GLYPH.height)),
            glyph_gap: grown(GLYPH_GAP),
            octave_width: grown(OCTAVE_WIDTH),
            octave_text: text(OCTAVE_SIZE, OCTAVE_MOST),
            octave_name_text: text(OCTAVE_NAME_SIZE, OCTAVE_NAME_MOST),
            pattern_name_padding: grown(PATTERN_NAME_PADDING),
            pattern_name_short: grown(PATTERN_NAME_SHORT),
            name_least_text: text(NAME_LEAST, NAME_LEAST_MOST),
            panel_icon,
            panel_padding: iced::Padding {
                top: over,
                right: panel_side,
                bottom: PANEL_BUTTON_HEIGHT - panel_icon - over,
                left: panel_side,
            },
        }
    }

    /// The width of the transport: Play, Stop, Record (the input meter's width while it shows
    /// under it), the column beside it and the metronome.
    fn transport_width(&self, input_meter: bool, column: Column) -> f32 {
        let record = if input_meter {
            self.input_meter
        } else {
            self.transport_button
        };
        let beside = match column {
            Column::Badge => 0.0,
            Column::Words | Column::Letters => self.take_column_width(column) + TRANSPORT_SPACING,
        };
        3.0 * self.transport_button + record + beside + 3.0 * TRANSPORT_SPACING
    }

    /// The column right of Record: as wide as its widest word and the room either side, in whole
    /// pixels; as [`Column::Letters`] its letters'; none as [`Column::Badge`].
    fn take_column_width(&self, column: Column) -> f32 {
        let words: &[&str] = match column {
            Column::Words => &[NOTES, AUDIO, CAPTURE],
            Column::Letters => &["N", "A"],
            Column::Badge => return 0.0,
        };
        let widest = words
            .iter()
            .map(|word| text_width(word, self.take_text))
            .fold(0.0, f32::max);
        (widest + 2.0 * self.take_side).ceil()
    }

    fn pattern_width(&self, tier: Tier) -> f32 {
        2.0 * self.arrow_width + self.pattern_name_width(tier)
    }

    /// The pattern name's box, its frame's line either side: at [`Tier::Full`] the wider of
    /// [`PATTERN_NAME_WHOLE`] at the labels' size and [`PATTERN_NAME_HELD`] at the least, measured,
    /// and its padding, in whole pixels; at [`Tier::Short`] [`PATTERN_NAME_SHORT`].
    fn pattern_name_width(&self, tier: Tier) -> f32 {
        match tier {
            Tier::Full => {
                let whole = text_width(PATTERN_NAME_WHOLE, self.label_text);
                let held = text_width(PATTERN_NAME_HELD, self.name_least_text);
                (whole.max(held) + 2.0 * self.pattern_name_padding + 2.0 * FRAME).ceil()
            }
            Tier::Short => self.pattern_name_short,
        }
    }

    /// The room the pattern's name has in its box: the box less its frame's lines and padding.
    fn pattern_name_room(&self, tier: Tier) -> f32 {
        self.pattern_name_width(tier) - 2.0 * FRAME - 2.0 * self.pattern_name_padding
    }

    /// The play mode's Pat over Song: as wide as the wider label.
    fn mode_pair_width(&self) -> f32 {
        let width = |label| text_width(label, self.mode_text) + 2.0 * self.label_side;
        width("Song").max(width("Pat"))
    }

    fn mode_width(&self) -> f32 {
        self.mode_pair_width() + PART_SPACING + self.label_button_width("Loop")
    }

    /// The keyboard switch's width: its glyph and its words on or off, the wider, so on and off
    /// are one width.
    fn keyboard_switch_width(&self, tier: Tier) -> f32 {
        let words = [false, true]
            .into_iter()
            .map(|on| text_width(keyboard_words(on, tier), self.label_text))
            .fold(0.0, f32::max);
        self.keyboard_glyph.width + self.glyph_gap + words + 2.0 * self.label_side
    }

    fn keyboard_width(&self, tier: Tier) -> f32 {
        self.keyboard_switch_width(tier) + PART_SPACING + self.octave_width
    }

    fn retry_width(&self) -> f32 {
        text_width(RETRY, self.retry_text) + 2.0 * self.label_side
    }

    fn menus_width(&self, tier: Tier) -> f32 {
        self.label_button_width(FILE)
            + MENU_SPACING
            + self.label_button_width(EDIT)
            + MENU_SPACING
            + self.label_button_width(help_label(tier))
    }

    /// A panel button's width: its icon and the padding either side.
    fn panel_button_width(&self) -> f32 {
        self.panel_icon + self.panel_padding.left + self.panel_padding.right
    }

    /// The panel grid's width.
    fn panels_width(&self) -> f32 {
        PANEL_COLUMNS as f32 * self.panel_button_width()
            + (PANEL_COLUMNS - 1) as f32 * PANEL_SPACING
    }

    /// A text button's width: its label at the labels' size with their side padding on both
    /// sides.
    fn label_button_width(&self, label: &str) -> f32 {
        text_width(label, self.label_text) + 2.0 * self.label_side
    }

    /// The vertical padding that centers a text box of `size` text in a group's height.
    fn text_box_padding(size: f32) -> f32 {
        ((BUTTON_HEIGHT - 1.3 * size) / 2.0).floor()
    }
}

/// The groups that show, left to right, with their widths at `sizes`: the labels at `tier`, the
/// panel buttons when `panels`, the notices' line `notice` wide while there are notices, Retry.
fn groups(
    tier: Tier,
    column: Column,
    panels: bool,
    notice: Option<f32>,
    retry: bool,
    input_meter: bool,
    sizes: &Sizes,
) -> Vec<(Group, f32)> {
    let mut groups = vec![
        (Group::Transport, sizes.transport_width(input_meter, column)),
        (Group::Tempo, sizes.tempo_width),
        (Group::Pattern, sizes.pattern_width(tier)),
        (Group::Mode, sizes.mode_width()),
        (Group::Position, sizes.position_width),
        (Group::Keyboard, sizes.keyboard_width(tier)),
    ];
    if panels {
        groups.push((Group::Panels, sizes.panels_width()));
    }
    let status = match (notice, retry) {
        (Some(notice), true) => Some(notice + BAR_SPACING + sizes.retry_width()),
        (Some(notice), false) => Some(notice),
        (None, true) => Some(sizes.retry_width()),
        (None, false) => None,
    };
    if let Some(status) = status {
        groups.push((Group::Status, status));
    }
    groups.push((Group::Menus, sizes.menus_width(tier)));
    groups
}

/// The width `groups` take with the least gaps between them and the side padding.
fn least_width(groups: &[(Group, f32)]) -> f32 {
    let widths: f32 = groups.iter().map(|(_, width)| width).sum();
    2.0 * TOOLBAR_SIDE_PADDING + widths + groups.len().saturating_sub(1) as f32 * GAP
}

/// The Record button: lit when armed, blinking between lit and off during the count-in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecordLook {
    pub armed: bool,
    /// The count-in plays and this is a dark beat of the blink.
    pub blink_off: bool,
    /// The input's level, while the Input source is armed.
    pub input_level: Option<f32>,
    /// What a take records: the switch beside Record lights it, and Record's tip says it.
    pub kind: TakeKind,
}

/// The computer keyboard's switch, lit while it plays notes, and its octave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypingLook {
    pub on: bool,
    /// The octave its first key plays, "C5".
    pub octave: String,
    /// Whether the octave can go down, and up.
    pub lower: bool,
    pub higher: bool,
    /// What the switch's tip says: which key plays which note at which velocity.
    pub tip: String,
}

/// What the toolbar shows.
pub struct Toolbar<'a> {
    /// The window's width, which decides the labels' [`Tier`].
    pub width: f32,
    pub playing: bool,
    pub tempo: MilliBpm,
    pub pattern_name: &'a str,
    pub previous_pattern: Option<PatternId>,
    /// The text of the pattern's name while it is being renamed.
    pub pattern_rename: Option<&'a str>,
    pub mode: PlayMode,
    /// The loop range's state: on, off, or `None` when there is no range.
    pub loop_on: Option<bool>,
    pub position: Ticks,
    pub ppq: u16,
    /// Why the last action failed.
    pub error: Option<&'a str>,
    /// What is wrong with live audio, such as it being off. Shown last, beside any error.
    pub audio_problem: Option<&'a str>,
    /// Audio is off and can be tried again: a Retry button stands before File.
    pub retry_audio: bool,
    /// What the last file or sound action has to tell, such as a missing sample.
    pub notice: Option<&'a str>,
    /// While a take runs: "Recording notes 0:04", the notices' line's first words.
    pub recording: Option<String>,
    /// Whether the File menu is open, so its button shows lit.
    pub menu_open: bool,
    /// Whether the Edit menu is open, so its button shows lit.
    pub edit_menu_open: bool,
    /// Whether the Help window is open, so its button shows lit.
    pub help_open: bool,
    /// Whether the metronome is on, so its button shows lit.
    pub metronome: bool,
    /// The text typed in the tempo's box while it is open (a click on the tempo opens it).
    pub tempo_edit: Option<&'a str>,
    pub record: RecordLook,
    /// Audio clip blocks that played silence because their stream was not ready.
    pub underruns: u64,
    pub typing: TypingLook,
    /// The panels open, whose buttons show lit.
    pub panels: Panels,
    /// The shortcuts, whose keys the tips name.
    pub keys: &'a Keys,
}

impl<'a> Toolbar<'a> {
    /// The notices as one line, a dot between two: what the last file or sound action has to
    /// tell, why the last action failed, what is wrong with live audio, the underruns; none
    /// while there is nothing to tell. How audio runs (resampling) is not one: Settings' audio
    /// output says it, and the hint bar over the position.
    pub fn notice_text(&self) -> Option<String> {
        let underruns = (self.underruns > 0).then(|| format!("Underruns: {}", self.underruns));
        let parts: Vec<&str> = [
            self.recording.as_deref(),
            self.notice,
            self.error,
            self.audio_problem,
        ]
        .into_iter()
        .flatten()
        .chain(underruns.as_deref())
        .collect();
        (!parts.is_empty()).then(|| parts.join(NOTICE_JOIN))
    }

    /// What decides where this toolbar's groups lie.
    pub fn shape(&self) -> Shape {
        Shape {
            width: self.width,
            retry: self.retry_audio,
            notices: self.notice_text().map(|text| notice_width(&text)),
            input_meter: self.record.input_level.is_some(),
        }
    }

    /// Where this toolbar's groups lie.
    pub fn spread(&self) -> Spread {
        Spread::of(self.shape())
    }

    pub fn view(self) -> Element<'a, Message> {
        let spread = self.spread();
        let tier = spread.tier;
        let s = spread.sizes();
        let transport = row![
            tool(
                transport_button(Glyph::Play, self.playing, Some(Message::Play), s),
                Tool::Play
            ),
            tool(
                transport_button(Glyph::Stop, false, Some(Message::Stop), s),
                Tool::Stop
            ),
            tool(
                with_tip(
                    record_button(self.record, spread.column == Column::Badge, s),
                    record_tip(self.keys, self.record.kind)
                ),
                Tool::Record
            ),
        ]
        .spacing(TRANSPORT_SPACING)
        .align_y(Center);
        let transport = match take_column(self.record.kind, spread.column, self.keys, s) {
            Some(column) => transport.push(column),
            None => transport,
        };
        let transport = row![
            transport,
            tool(
                with_tip(
                    transport_button(
                        Glyph::Metronome,
                        self.metronome,
                        Some(Message::ToggleMetronome),
                        s
                    ),
                    "Metronome"
                ),
                Tool::Metronome
            ),
        ]
        .spacing(TRANSPORT_SPACING)
        .align_y(Center);
        // One piece in one frame: the previous arrow, the name on the readouts' dark glass, the
        // next arrow, the frame's line between each.
        let pattern = framed(
            row![
                tool(
                    arrow_button(
                        Glyph::Previous,
                        self.previous_pattern.map(Message::PatternSelected),
                        iced::border::left(1.0),
                        s,
                    ),
                    Tool::PreviousPattern
                ),
                tool(
                    pattern_name(self.pattern_name, self.pattern_rename, tier, s),
                    Tool::PatternName
                ),
                // Past the last pattern it makes a new one.
                tool(
                    arrow_button(
                        Glyph::Next,
                        Some(Message::Pattern(PatternMessage::Next)),
                        iced::border::right(1.0),
                        s,
                    ),
                    Tool::NextPattern
                ),
            ]
            .spacing(FRAME)
            .align_y(Center),
        );
        let looping = label_button(
            "Loop",
            self.loop_on == Some(true),
            self.loop_on.map(|_| Message::ToggleLoop),
            s,
        );
        let looping = with_tip(looping, loop_tip(self.loop_on));
        let side = s.mode_pair_width() - 2.0 * FRAME;
        // Pat over Song: one switch of two rows in one frame, the mode playing lit.
        let mode = row![
            framed(column![
                tool(
                    mode_button(
                        "Pat",
                        PlayMode::Pattern,
                        self.mode,
                        [side, s.mode_text],
                        iced::border::top(1.0)
                    ),
                    Tool::PatternMode
                ),
                tool(
                    mode_button(
                        "Song",
                        PlayMode::Song,
                        self.mode,
                        [side, s.mode_text],
                        iced::border::bottom(1.0)
                    ),
                    Tool::SongMode
                ),
            ]),
            tool(looping, Tool::Loop),
        ]
        .spacing(PART_SPACING)
        .align_y(Center);
        // Drag up or down, or scroll, to change the tempo; Shift for 0.001 steps; a click
        // opens a box to type it in.
        let tempo: Element<'a, Message> = match self.tempo_edit {
            // In a box the readout's height, so the group keeps it while the tempo is typed.
            Some(typed) => container(
                text_input("120", typed)
                    .id(Id::new(crate::app::TEMPO_INPUT))
                    .on_input(Message::TempoEdited)
                    .on_submit(Message::TempoSubmitted)
                    .font(Font::MONOSPACE)
                    .size(s.readout_text)
                    .padding([Sizes::text_box_padding(s.readout_text), s.readout_padding])
                    .width(s.tempo_width),
            )
            .height(BUTTON_HEIGHT)
            .center_y(BUTTON_HEIGHT)
            .into(),
            None => with_tip(
                mouse_area(readout(format_tempo(self.tempo), s.tempo_width, s))
                    .on_press(Message::TempoPressed)
                    .on_scroll(Message::TempoScrolled)
                    .interaction(mouse::Interaction::ResizingVertically),
                "Tempo \u{b7} drag or scroll; click to type it",
            ),
        };
        let tempo = tool(tempo, Tool::Tempo);
        // Why measured to fit: the toolbar is one line, and a notice longer than its share
        // wrapped into lines the toolbar has no height for, and showed nothing at all.
        let notices = self
            .notice_text()
            .map(|text| notice_line(text, spread.notice_width));
        // Why its own item beside the notices' line: inside a room fitted to the notices, the
        // button took its width from that room, and below 1000 px it overflowed its box, drew
        // over File or was left out with the notices.
        let retry = self.retry_audio.then(|| {
            let face = button(
                container(text(RETRY).size(s.retry_text))
                    .center_y(Fill)
                    .center_x(Fill),
            )
            .width(s.retry_width())
            .height(BUTTON_HEIGHT)
            .padding(0)
            .style(style::tool_button(false))
            .on_press(Message::RetryAudio);
            tool(face, Tool::Retry)
        });
        let menus = row![
            tool(
                label_button(
                    FILE,
                    self.menu_open,
                    Some(Message::File(FileMessage::ToggleMenu)),
                    s
                ),
                Tool::File
            ),
            tool(
                label_button(EDIT, self.edit_menu_open, Some(Message::ToggleEditMenu), s),
                Tool::Edit
            ),
            tool(
                label_button(
                    help_label(tier),
                    self.help_open,
                    Some(Message::ToggleHelp),
                    s
                ),
                Tool::Help
            ),
        ]
        .spacing(MENU_SPACING);
        let typing = typing_keyboard(self.typing, tier, s);
        let position = tool(
            readout(
                format_position(self.position, self.ppq),
                s.position_width,
                s,
            ),
            Tool::Position,
        );
        let status: Option<Element<'a, Message>> = match (notices, retry) {
            (Some(notices), Some(retry)) => Some(
                row![notices, retry]
                    .spacing(BAR_SPACING)
                    .align_y(Center)
                    .into(),
            ),
            (Some(only), None) | (None, Some(only)) => Some(only),
            (None, None) => None,
        };
        let mut parts = [
            Some(transport.into()),
            Some(tempo),
            Some(pattern),
            Some(mode.into()),
            Some(position),
            Some(typing),
            spread
                .panels
                .then(|| panel_buttons(self.panels, self.keys, s)),
            status,
            Some(menus.into()),
        ]
        .into_iter()
        .flatten();
        let mut content = row![].align_y(Center);
        let count = spread.groups().len();
        for index in 0..count {
            if let Some(part) = parts.next() {
                content = content.push(part);
            }
            if index + 1 == count {
                break;
            }
            // The line in the gap's middle, on a whole pixel past its group (packed, the last
            // gap's beside File or its group); the last gap's right side takes what is left, so
            // Help ends at the right padding.
            let extra = spread.extra_after(index);
            let left = spread.line_after(index);
            content = content
                .push(iced::widget::space().width(left))
                .push(divider());
            content = if index + 2 == count {
                content.push(iced::widget::space().width(Fill))
            } else {
                content.push(iced::widget::space().width(GAP - DIVIDER.width - left + extra))
            };
        }
        let bar = container(content)
            .padding([0.0, TOOLBAR_SIDE_PADDING])
            .width(Fill)
            .height(TOOLBAR_HEIGHT)
            .align_y(Center)
            .style(style::panel);
        // A line along the toolbar's last pixel row, over its ground (nothing
        // of the groups reaches it), so the bar ends where the panels' title rows begin.
        let foot = column![
            iced::widget::space().height(Fill),
            container(iced::widget::space())
                .width(Fill)
                .height(EDGE)
                .style(style::divider),
        ];
        iced::widget::stack![bar, foot].into()
    }
}

/// The thin line between two groups.
fn divider<'a>() -> Element<'a, Message> {
    container(iced::widget::space())
        .width(DIVIDER.width)
        .height(DIVIDER.height)
        .style(style::divider)
        .into()
}

/// The Record button's tip: what it records now and how it arms, "Record notes · R arms it;
/// the switch beside it or right-click chooses notes or audio".
fn record_tip(keys: &Keys, kind: TakeKind) -> String {
    let what = match kind {
        TakeKind::Notes => "Record notes",
        TakeKind::Audio => "Record audio",
    };
    format!(
        "{what} \u{b7} {} arms it; the switch beside it or right-click chooses notes or audio",
        keys.first(Action::ArmRecord)
    )
}

/// The column right of Record: Notes over Audio, the side lit what a take records (a press
/// picks it, a right press opens its menu), then Capture; as [`Column::Letters`] the switch
/// alone, its sides N and A; none as [`Column::Badge`], where Record says it.
fn take_column<'a>(
    kind: TakeKind,
    column: Column,
    keys: &Keys,
    s: Sizes,
) -> Option<Element<'a, Message>> {
    let width = s.take_column_width(column);
    let (notes, audio, row) = match column {
        Column::Words => (NOTES, AUDIO, TAKE_SIDE_HEIGHT),
        Column::Letters => ("N", "A", SWITCH_SIDE),
        Column::Badge => return None,
    };
    let side = |label: &'a str, side: TakeKind, radius, tip: &'static str, which| {
        let face = button(
            container(text(label).size(s.take_text).wrapping(Wrapping::None))
                .center_x(Fill)
                .center_y(Fill),
        )
        .width(width - 2.0 * FRAME)
        .height(row)
        .padding(0)
        .style(style::switch_side(kind == side, radius))
        .on_press(Message::Record(RecordMessage::Kind(side)));
        let face = mouse_area(face).on_right_press(Message::Record(RecordMessage::Menu(side)));
        tool(with_tip(face, tip), which)
    };
    let switch = framed(column![
        side(
            notes,
            TakeKind::Notes,
            iced::border::top(1.0),
            "Notes \u{b7} Record takes the notes played into the piano roll; right-click: quantize, count-in, Capture",
            Tool::TakeNotes
        ),
        side(
            audio,
            TakeKind::Audio,
            iced::border::bottom(1.0),
            "Audio \u{b7} Record takes what you hear (or the line in) into a sound file; right-click: which",
            Tool::TakeAudio
        ),
    ]);
    if column == Column::Letters {
        return Some(switch);
    }
    let capture = button(
        container(text(CAPTURE).size(s.take_text).wrapping(Wrapping::None))
            .center_x(Fill)
            .center_y(Fill),
    )
    .width(width)
    .height(CAPTURE_ROW)
    .padding(0)
    .style(style::tool_button(false))
    .on_press(Message::Record(RecordMessage::Capture));
    let tip = format!(
        "Capture \u{b7} {} turns the notes you played in the last 30 s into notes",
        keys.first(Action::Capture)
    );
    Some(
        column![switch, tool(with_tip(capture, tip), Tool::Capture)]
            .spacing(PART_SPACING)
            .into(),
    )
}

/// The Loop button's tip: how to get a loop range while there is none, else what it does.
fn loop_tip(loop_on: Option<bool>) -> &'static str {
    match loop_on {
        None => "Drag in the ruler to set a loop range",
        Some(_) => "Loop \u{b7} switches the loop range on or off",
    }
}

/// `content`, saying when the cursor comes onto it and leaves it, for the hint bar.
fn tool<'a>(content: impl Into<Element<'a, Message>>, which: Tool) -> Element<'a, Message> {
    hover_area(content, Hover::Tool(which))
}

/// The pattern's name, which a right press opens the menu of, or the text box it is renamed in:
/// inside the pattern's frame, on the readouts' dark glass, centered.
fn pattern_name<'a>(
    name: &'a str,
    rename: Option<&'a str>,
    tier: Tier,
    s: Sizes,
) -> Element<'a, Message> {
    let width = s.pattern_name_width(tier) - 2.0 * FRAME;
    let height = BUTTON_HEIGHT - 2.0 * FRAME;
    match rename {
        Some(typed) => container(
            text_input("", typed)
                .id(Id::new(PATTERN_RENAME_INPUT))
                .on_input(|typed| Message::Pattern(PatternMessage::RenameEdited(typed)))
                .on_submit(Message::Pattern(PatternMessage::RenameSubmitted))
                .size(s.label_text)
                .padding([
                    Sizes::text_box_padding(s.label_text) - FRAME,
                    s.pattern_name_padding,
                ])
                .width(width),
        )
        .height(height)
        .center_y(height)
        .into(),
        None => {
            // One line, shrunk to fit, then cut with an ellipsis: a long name once wrapped out of
            // the toolbar, then was cut past ten characters.
            let (shown, size) = shown_pattern_name(name, tier, s);
            mouse_area(
                container(
                    text(shown)
                        .size(size)
                        .color(style::LCD_TEXT)
                        .wrapping(Wrapping::None),
                )
                .width(width)
                .height(height)
                .center_x(width)
                .center_y(height)
                .padding([0.0, s.pattern_name_padding])
                .style(style::name_display),
            )
            .on_right_press(Message::Pattern(PatternMessage::ToggleMenu))
            .into()
        }
    }
}

/// The pattern's name as its box shows it, and its text size: whole at the labels' size when it
/// fits; else whole at the largest whole size down to the least that fits ([`NAME_LEAST`]); else
/// cut with an ellipsis at the least (the hint bar names it whole). Any name, of any length, so
/// fits the box.
fn shown_pattern_name(name: &str, tier: Tier, s: Sizes) -> (String, f32) {
    let room = s.pattern_name_room(tier);
    let (most, least) = (s.label_text, s.name_least_text.min(s.label_text));
    let natural = text_width(name, most);
    if natural <= room {
        return (name.to_owned(), most);
    }
    // The text's width grows with its size about in proportion: start at the size that would
    // just fit and step down a pixel while it does not.
    let mut size = (most * room / natural).floor().clamp(least, most);
    while size > least && text_width(name, size) > room {
        size -= 1.0;
    }
    if text_width(name, size) <= room {
        return (name.to_owned(), size);
    }
    let cut =
        crate::ui::text_fit::fit_width(name, room, least).map_or_else(String::new, Cow::into_owned);
    (cut, least)
}

/// The keyboard switch's words, on one line: "Keyboard on" or, short, "On".
fn keyboard_words(on: bool, tier: Tier) -> &'static str {
    match (tier, on) {
        (Tier::Full, true) => "Keyboard on",
        (Tier::Full, false) => "Keyboard off",
        (Tier::Short, true) => "On",
        (Tier::Short, false) => "Off",
    }
}

/// The Help button's label: "Help", or in a window too narrow for the full labels "?" (its hint
/// still says Help), so the narrowest toolbar keeps the notices their least room.
fn help_label(tier: Tier) -> &'static str {
    match tier {
        Tier::Full => "Help",
        Tier::Short => "?",
    }
}

/// The six panel buttons in their grid, each lit while its panel is open, its tip naming the
/// panel and its two keys.
fn panel_buttons<'a>(panels: Panels, keys: &Keys, s: Sizes) -> Element<'a, Message> {
    let button_of = |panel: Panel| -> Element<'a, Message> {
        let open = panels.is_open(panel);
        let color = if open { style::ACCENT } else { style::TEXT };
        let face = button(
            canvas(PanelIcon { panel, color })
                .width(s.panel_icon)
                .height(s.panel_icon),
        )
        .width(s.panel_button_width())
        .height(PANEL_BUTTON_HEIGHT)
        .padding(s.panel_padding)
        .style(style::tool_button(open))
        .on_press(Message::TogglePanel(panel));
        let tip = container(text(panel_tip(panel, keys)).size(TIP_SIZE))
            .padding([4, 8])
            .style(style::panel);
        tool(
            tooltip(face, tip, tooltip::Position::Bottom),
            Tool::Panel(panel),
        )
    };
    let rows = Panel::ALL.chunks(PANEL_COLUMNS).map(|panels| {
        row(panels.iter().map(|&panel| button_of(panel)))
            .spacing(PANEL_SPACING)
            .into()
    });
    column(rows).spacing(PANEL_SPACING).into()
}

/// A panel button's tip: "Playlist · 2 or F5", its keys as bound.
pub fn panel_tip(panel: Panel, keys: &Keys) -> String {
    format!(
        "{} \u{b7} {}",
        panel.title(),
        keys.all(Action::Panel(panel), " or ")
    )
}

/// The width the notices' line `text` takes on its own: its text, cut to [`NOTICE_MOST`].
fn notice_width(text: &str) -> f32 {
    fit(text, NOTICE_MOST, &line_width).map_or(0.0, |fitted| line_width(&fitted.text))
}

/// The notices' line, `width` wide: `text` on one line, cut with an ellipsis when longer (its
/// tip then holds it whole, as the hint bar does over it), in the warnings' color. A press
/// opens the Settings window, where how audio runs is said.
fn notice_line<'a>(text_all: String, width: f32) -> Element<'a, Message> {
    let fitted = fit(&text_all, width, &line_width);
    let cut = fitted.as_ref().is_none_or(|fitted| fitted.cut);
    let shown = fitted.map_or_else(String::new, |fitted| fitted.text.into_owned());
    let line = container(
        text(shown)
            .size(NOTICE_SIZE)
            .color(style::WARNING)
            .wrapping(Wrapping::None),
    )
    .width(width)
    .height(BUTTON_HEIGHT)
    .align_y(Center);
    let pressed = mouse_area(line)
        .on_press(Message::NoticePressed)
        .interaction(mouse::Interaction::Pointer);
    let line: Element<'a, Message> = if cut {
        with_tip(pressed, text_all)
    } else {
        pressed.into()
    };
    tool(line, Tool::Notices)
}

/// A text button a group's height and as wide as its label at `s`: lit when `lit`, dim with
/// nothing to do.
fn label_button<'a>(
    label: &'a str,
    lit: bool,
    on_press: Option<Message>,
    s: Sizes,
) -> Element<'a, Message> {
    button(
        container(text(label).size(s.label_text).wrapping(Wrapping::None))
            .center_x(Fill)
            .center_y(Fill),
    )
    .width(s.label_button_width(label))
    .height(BUTTON_HEIGHT)
    .padding(0)
    .style(style::tool_button(lit))
    .on_press_maybe(on_press)
    .into()
}

/// The toolbar's layout in a scripted shot `width` wide, as the pixel probes read it: with audio
/// off (`audio_off`), Retry and notices longer than any room it has; else neither, no input
/// meter.
#[cfg(test)]
pub(crate) fn shot_spread(width: f32, audio_off: bool) -> Spread {
    Spread::of(Shape {
        width,
        retry: audio_off,
        notices: audio_off.then_some(NOTICE_MOST),
        input_meter: false,
    })
}

/// The rows every group of the toolbar and every line between two spans: the groups' height
/// centered in the bar (y 9..55; 10..62 in the 72 px bar). As the pixel probes look for them.
#[cfg(test)]
pub(crate) fn group_rows() -> std::ops::Range<f32> {
    let top = (TOOLBAR_HEIGHT - BUTTON_HEIGHT) / 2.0;
    top..top + BUTTON_HEIGHT
}

/// A panel button's icon and its width with nothing to spare (scale 1) and at the most scale:
/// 11 and 19 px, 16 and 28 (worked by hand in the sizes' test). As the probe of the icons wants
/// them.
#[cfg(test)]
pub(crate) fn panel_icon_sizes() -> [(f32, f32); 2] {
    [Sizes::at(1.0), Sizes::at(SCALE_MOST)].map(|s| (s.panel_icon, s.panel_button_width()))
}

/// Where Retry, Edit, File and Help lie across the toolbar `spread` lays out: their left and
/// right edges, as the pixel probe looks for them. Help stands against the right padding, Edit
/// the menus' spacing before it and File the spacing before Edit, at the group's start; Retry
/// ends its group, before the last gap.
#[cfg(test)]
pub(crate) fn retry_edit_file_and_help(spread: &Spread) -> [std::ops::Range<f32>; 4] {
    let s = spread.sizes();
    let menus = spread
        .span(Group::Menus)
        .expect("File, Edit and Help always show");
    let help_start = menus.end - s.label_button_width(help_label(spread.tier));
    let edit_end = help_start - MENU_SPACING;
    let edit_start = edit_end - s.label_button_width(EDIT);
    let file_end = edit_start - MENU_SPACING;
    let retry_end = spread
        .span(Group::Status)
        .map_or(menus.start, |status| status.end);
    [
        retry_end - s.retry_width()..retry_end,
        edit_start..edit_end,
        menus.start..file_end,
        help_start..menus.end,
    ]
}

/// Where each panel button lies in the toolbar `spread` lays out, while they show: the grid
/// [`BUTTON_HEIGHT`] tall, centered on the toolbar's height. As the pixel probe looks for them.
#[cfg(test)]
pub(crate) fn panel_button_rects(spread: &Spread) -> Vec<(Panel, Rectangle)> {
    let Some(left) = spread.left(Group::Panels) else {
        return Vec::new();
    };
    let top = (TOOLBAR_HEIGHT - BUTTON_HEIGHT) / 2.0;
    let width = spread.sizes.panel_button_width();
    Panel::ALL
        .into_iter()
        .enumerate()
        .map(|(index, panel)| {
            let (column, row) = (index % PANEL_COLUMNS, index / PANEL_COLUMNS);
            let at = Point::new(
                left + column as f32 * (width + PANEL_SPACING),
                top + row as f32 * (PANEL_BUTTON_HEIGHT + PANEL_SPACING),
            );
            let size = Size::new(width, PANEL_BUTTON_HEIGHT);
            (panel, Rectangle::new(at, size))
        })
        .collect()
}

/// The width `content` takes on one line as a notice: its text size, the window's font.
fn line_width(content: &str) -> f32 {
    text_width(content, NOTICE_SIZE)
}

/// Tempo with three decimals: 120000 milli-BPM reads "120.000".
pub fn format_tempo(tempo: MilliBpm) -> String {
    format!("{}.{:03}", tempo.0 / 1000, tempo.0 % 1000)
}

/// A typed tempo: BPM with up to three decimals after a point or a comma, as milli-BPM inside
/// the range a project takes; else why not, in words for the toolbar's notice.
pub fn parse_tempo(typed: &str) -> Result<MilliBpm, String> {
    let typed = typed.trim().replace(',', ".");
    let (whole, fraction) = typed.split_once('.').unwrap_or((typed.as_str(), ""));
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    let pointed = typed.contains('.');
    let shaped = digits(whole) && (!pointed || digits(fraction)) && fraction.len() <= 3;
    let milli = shaped
        .then(|| {
            let whole: u32 = whole.parse().ok()?;
            let fraction: u32 = format!("{fraction:0<3}").parse().ok()?;
            whole.checked_mul(1000)?.checked_add(fraction)
        })
        .flatten()
        .ok_or_else(|| "Type a tempo in BPM, such as 120 or 87.5".to_owned())?;
    if !TEMPO_RANGE.contains(&milli) {
        return Err(format!(
            "Tempo goes from {} to {} BPM",
            TEMPO_RANGE.start() / 1000,
            TEMPO_RANGE.end() / 1000
        ));
    }
    Ok(MilliBpm(milli))
}

/// `bar:step:tick`, counted from 1:01:00 at the start of the pattern.
fn format_position(position: Ticks, ppq: u16) -> String {
    let (bars, steps, ticks) = bars_steps_ticks(position, ppq);
    format!("{}:{:02}:{ticks:02}", bars + 1, steps + 1)
}

/// A length as `bars:steps:ticks`, counted from 0:00:00: a step is 0:01:00.
pub fn format_length(length: Ticks, ppq: u16) -> String {
    let (bars, steps, ticks) = bars_steps_ticks(length, ppq);
    format!("{bars}:{steps:02}:{ticks:02}")
}

/// Whole bars, then whole steps, then ticks in a span from tick 0 (negative counts as 0).
fn bars_steps_ticks(span: Ticks, ppq: u16) -> (i64, i64, i64) {
    let bar_ticks = time::bars_to_ticks(1, ppq).0;
    let step_ticks = time::step_to_ticks(1, ppq).0;
    let ticks = span.0.max(0);
    (
        ticks / bar_ticks,
        ticks % bar_ticks / step_ticks,
        ticks % step_ticks,
    )
}

/// A recessed display with right-aligned digits.
fn readout<'a>(content: String, width: f32, s: Sizes) -> Element<'a, Message> {
    container(
        text(content)
            .font(Font::MONOSPACE)
            .size(s.readout_text)
            .color(style::LCD_TEXT),
    )
    .width(width)
    .height(BUTTON_HEIGHT)
    .padding([0.0, s.readout_padding])
    .align_x(Horizontal::Right)
    .center_y(BUTTON_HEIGHT)
    .style(style::lcd)
    .into()
}

/// The computer keyboard: its switch (a keyboard glyph and "Keyboard on" on one line, lit while
/// it plays notes; a press switches it), then + over the octave's name over −, the switch's
/// height.
fn typing_keyboard<'a>(look: TypingLook, tier: Tier, s: Sizes) -> Element<'a, Message> {
    let glyph_color = if look.on { style::ACCENT } else { style::TEXT };
    let words = text(keyboard_words(look.on, tier))
        .size(s.label_text)
        .wrapping(Wrapping::None);
    let face = row![
        canvas(Icon {
            glyph: Glyph::Keyboard,
            color: glyph_color,
        })
        .width(s.keyboard_glyph.width)
        .height(s.keyboard_glyph.height),
        words,
    ]
    .spacing(s.glyph_gap)
    .align_y(Center);
    let switch = button(container(face).center_x(Fill).center_y(Fill))
        .width(s.keyboard_switch_width(tier))
        .height(BUTTON_HEIGHT)
        .padding(0)
        .style(style::tool_button(look.on))
        .on_press(Message::ToggleTyping);
    let switch = with_tip(switch, look.tip);
    let octave = |label, octaves: i8, enabled: bool| {
        button(
            container(text(label).size(s.octave_text))
                .center_x(Fill)
                .center_y(Fill),
        )
        .width(s.octave_width)
        .height(OCTAVE_BUTTON)
        .padding(0)
        .style(style::tool_button(false))
        .on_press_maybe(enabled.then_some(Message::TypingOctave(octaves)))
    };
    let name = container(
        text(look.octave)
            .size(s.octave_name_text)
            .color(style::TEXT_DIM)
            .wrapping(Wrapping::None),
    )
    .width(s.octave_width)
    .height(OCTAVE_NAME)
    .center_x(s.octave_width)
    .center_y(OCTAVE_NAME);
    let column = column![
        tool(octave("+", 1, look.higher), Tool::OctaveUp),
        name,
        tool(octave("\u{2212}", -1, look.lower), Tool::OctaveDown),
    ];
    row![tool(switch, Tool::Keys), column]
        .spacing(PART_SPACING)
        .align_y(Center)
        .into()
}

/// Pat or Song, a side of the play mode's switch `width` wide with `size` text, inside its
/// frame: `radius` rounds the switch's outer corners inside the frame's.
fn mode_button<'a>(
    label: &'a str,
    mode: PlayMode,
    current: PlayMode,
    [width, size]: [f32; 2],
    radius: iced::border::Radius,
) -> Element<'a, Message> {
    button(
        container(text(label).size(size).wrapping(Wrapping::None))
            .center_x(Fill)
            .center_y(Fill),
    )
    .width(width)
    .height(SWITCH_SIDE)
    .padding(0)
    .style(style::switch_side(mode == current, radius))
    .on_press(Message::ModeSelected(mode))
    .into()
}

/// The round Record button (a press arms, a right press opens its menu), with the input meter
/// under it while the Input source is armed; with `badge` (no room for the switch beside it) it
/// says under its dot what a take records, N or A.
fn record_button<'a>(look: RecordLook, badge: bool, s: Sizes) -> Element<'a, Message> {
    let color = if look.armed && !look.blink_off {
        style::RECORD_ARMED
    } else {
        style::RECORD_OFF
    };
    let dot = canvas(Icon {
        glyph: Glyph::Record,
        color,
    })
    .width(s.transport_icon)
    .height(s.transport_icon);
    let content: Element<'a, Message> = if badge {
        let letter = match look.kind {
            TakeKind::Notes => "N",
            TakeKind::Audio => "A",
        };
        column![dot, text(letter).size(s.badge_text).color(style::TEXT_DIM)]
            .spacing(1)
            .align_x(Center)
            .into()
    } else {
        dot.into()
    };
    let face = button(container(content).center_x(Fill).center_y(Fill))
        .width(s.transport_button)
        .height(BUTTON_HEIGHT)
        .padding(0)
        .style(style::tool_button(look.armed))
        .on_press(Message::Record(RecordMessage::ToggleArm));
    let face = mouse_area(face).on_right_press(Message::Record(RecordMessage::ToggleMenu));
    match look.input_level {
        Some(level) => column![
            face,
            canvas(InputMeter { level })
                .width(s.input_meter)
                .height(INPUT_METER.height)
        ]
        .spacing(2)
        .align_x(Center)
        .into(),
        None => face.into(),
    }
}

/// The input meter: filled from the left to the level on the mixer meters' decibel scale,
/// green to −6 dB, yellow to 0 dB, red above.
struct InputMeter {
    level: f32,
}

impl canvas::Program<Message> for InputMeter {
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
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), style::METER_GROUND);
        let along = |level: f32| mixer_layout::meter_fraction(level) * bounds.width;
        let filled = along(self.level);
        let bands = [
            (0.0, along(MINUS_6_DB), style::METER_GREEN),
            (along(MINUS_6_DB), along(1.0), style::METER_YELLOW),
            (along(1.0), bounds.width, style::METER_RED),
        ];
        for (from, to, color) in bands {
            let end = filled.min(to);
            if end > from {
                frame.fill_rectangle(
                    Point::new(from, 0.0),
                    Size::new(end - from, bounds.height),
                    color,
                );
            }
        }
        vec![frame.into_geometry()]
    }
}

/// A transport button: 38 × 46 px at a scale of 1, showing `glyph`, accent when `lit`, dimmed
/// when it has nothing to do.
fn transport_button<'a>(
    glyph: Glyph,
    lit: bool,
    on_press: Option<Message>,
    s: Sizes,
) -> Element<'a, Message> {
    glyph_face(glyph, lit, on_press, s.transport_icon)
        .width(s.transport_button)
        .height(BUTTON_HEIGHT)
        .style(style::tool_button(lit))
        .into()
}

/// One of the pattern's arrows: a 13 px glyph in a 20 px wide part of the pattern's piece at a
/// scale of 1, inside its frame, `radius` rounding the piece's outer corners inside the frame's.
fn arrow_button<'a>(
    glyph: Glyph,
    on_press: Option<Message>,
    radius: iced::border::Radius,
    s: Sizes,
) -> Element<'a, Message> {
    glyph_face(glyph, false, on_press, s.arrow_icon)
        .width(s.arrow_width - FRAME)
        .height(BUTTON_HEIGHT - 2.0 * FRAME)
        .style(style::piece_part(radius))
        .into()
}

/// A button showing `glyph` `side` px square in its middle: the accent when `lit`, dim with
/// nothing to do.
fn glyph_face<'a>(
    glyph: Glyph,
    lit: bool,
    on_press: Option<Message>,
    side: f32,
) -> iced::widget::Button<'a, Message> {
    let color = match (lit, on_press.is_some()) {
        (true, _) => style::ACCENT,
        (false, true) => style::TEXT,
        (false, false) => style::TEXT_DIM,
    };
    button(
        container(canvas(Icon { glyph, color }).width(side).height(side))
            .center_x(Fill)
            .center_y(Fill),
    )
    .padding(0)
    .on_press_maybe(on_press)
}

/// `parts` in one frame: a line of the buttons' edge round them, and between them where they
/// stand [`FRAME`] apart, so they read as one control.
fn framed<'a>(parts: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(parts)
        .padding(FRAME)
        .style(style::piece_frame)
        .into()
}

#[derive(Debug, Clone, Copy)]
enum Glyph {
    Play,
    Stop,
    Record,
    /// A metronome: a tall triangle, its arm swung right.
    Metronome,
    Previous,
    Next,
    /// A computer keyboard: an outline with two rows of keys and a space bar.
    Keyboard,
}

/// A toolbar glyph drawn as filled and stroked shapes in its canvas's bounds, so no icon font
/// is needed.
struct Icon {
    glyph: Glyph,
    color: Color,
}

impl canvas::Program<Message> for Icon {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        // The transport's icons show in every window, so they tell `--stats` which renderer
        // draws it.
        crate::window_stats::drawn_with(renderer);
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let (w, s) = (bounds.width, bounds.height);
        let line = |width| {
            canvas::Stroke::default()
                .with_color(self.color)
                .with_width(width)
        };
        let shape = match self.glyph {
            Glyph::Play => triangle([(2.0, 0.5), (s - 0.5, s / 2.0), (2.0, s - 0.5)]),
            Glyph::Stop => {
                canvas::Path::rectangle(Point::new(1.5, 1.5), Size::new(s - 3.0, s - 3.0))
            }
            Glyph::Record => canvas::Path::circle(Point::new(s / 2.0, s / 2.0), s / 2.0 - 0.5),
            Glyph::Metronome => {
                let body = triangle([(s / 2.0, 0.75), (s - 1.5, s - 0.75), (1.5, s - 0.75)]);
                frame.stroke(&body, line(1.75));
                let arm =
                    canvas::Path::line(Point::new(s / 2.0, s - 3.0), Point::new(s - 1.0, 1.5));
                frame.stroke(&arm, line(1.75));
                return vec![frame.into_geometry()];
            }
            Glyph::Previous => triangle([(s - 2.0, 0.5), (2.0, s / 2.0), (s - 2.0, s - 0.5)]),
            Glyph::Next => triangle([(2.0, 0.5), (s - 2.0, s / 2.0), (2.0, s - 0.5)]),
            Glyph::Keyboard => {
                // Drawn at 18 × 12 and scaled to the canvas.
                let scale = w / KEYBOARD_DRAWN;
                frame.scale(scale);
                let (w, s) = (w / scale, s / scale);
                let outline = canvas::Path::rounded_rectangle(
                    Point::new(0.75, 0.75),
                    Size::new(w - 1.5, s - 1.5),
                    2.0.into(),
                );
                frame.stroke(&outline, line(1.5));
                for row in 0..2 {
                    for key in 0..5 {
                        let at = Point::new(3.0 + key as f32 * 2.6, 3.0 + row as f32 * 2.5);
                        frame.fill_rectangle(at, Size::new(1.6, 1.5), self.color);
                    }
                }
                frame.fill_rectangle(
                    Point::new(5.0, s - 4.0),
                    Size::new(w - 10.0, 1.5),
                    self.color,
                );
                return vec![frame.into_geometry()];
            }
        };
        frame.fill(&shape, self.color);
        vec![frame.into_geometry()]
    }
}

/// A panel button's icon, drawn as filled shapes in a [`PANEL_ICON`] square: the Library a
/// list, the rack two rows of steps, the piano roll a key column and notes, the playlist clips
/// on lanes, the mixer three faders, the settings a cog.
struct PanelIcon {
    panel: Panel,
    color: Color,
}

impl canvas::Program<Message> for PanelIcon {
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
        // Drawn in a 10 px square and scaled to the canvas.
        frame.scale(bounds.width / PANEL_ICON_DRAWN);
        let boxes: &[(f32, f32, f32, f32)] = match self.panel {
            Panel::Browser => &[
                (0.0, 0.5, 2.0, 2.0),
                (3.0, 0.5, 7.0, 2.0),
                (0.0, 4.0, 2.0, 2.0),
                (3.0, 4.0, 7.0, 2.0),
                (0.0, 7.5, 2.0, 2.0),
                (3.0, 7.5, 7.0, 2.0),
            ],
            Panel::Rack => &[
                (0.0, 1.0, 2.0, 3.0),
                (2.7, 1.0, 2.0, 3.0),
                (5.3, 1.0, 2.0, 3.0),
                (8.0, 1.0, 2.0, 3.0),
                (0.0, 6.0, 2.0, 3.0),
                (2.7, 6.0, 2.0, 3.0),
                (5.3, 6.0, 2.0, 3.0),
                (8.0, 6.0, 2.0, 3.0),
            ],
            Panel::PianoRoll => &[
                (0.0, 0.0, 2.5, 10.0),
                (4.0, 1.0, 3.0, 2.0),
                (6.0, 4.0, 4.0, 2.0),
                (4.0, 7.5, 5.0, 2.0),
            ],
            Panel::Playlist => &[
                (0.0, 0.5, 4.0, 2.5),
                (5.0, 0.5, 5.0, 2.5),
                (2.0, 3.75, 6.0, 2.5),
                (0.0, 7.0, 3.0, 2.5),
                (4.0, 7.0, 6.0, 2.5),
            ],
            Panel::Mixer => &[
                (1.0, 0.0, 1.0, 10.0),
                (0.0, 6.0, 3.0, 2.0),
                (4.5, 0.0, 1.0, 10.0),
                (3.5, 2.0, 3.0, 2.0),
                (8.0, 0.0, 1.0, 10.0),
                (7.0, 4.0, 3.0, 2.0),
            ],
            Panel::Settings => &[
                (4.0, 0.0, 2.0, 2.0),
                (4.0, 8.0, 2.0, 2.0),
                (0.0, 4.0, 2.0, 2.0),
                (8.0, 4.0, 2.0, 2.0),
            ],
        };
        for &(x, y, width, height) in boxes {
            frame.fill_rectangle(Point::new(x, y), Size::new(width, height), self.color);
        }
        if self.panel == Panel::Settings {
            let ring = canvas::Path::circle(Point::new(5.0, 5.0), 3.0);
            frame.stroke(
                &ring,
                canvas::Stroke::default()
                    .with_color(self.color)
                    .with_width(2.0),
            );
        }
        vec![frame.into_geometry()]
    }
}

fn triangle(corners: [(f32, f32); 3]) -> canvas::Path {
    canvas::Path::new(|path| {
        let [first, second, third] = corners.map(|(x, y)| Point::new(x, y));
        path.move_to(first);
        path.line_to(second);
        path.line_to(third);
        path.close();
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sizes at a scale of 1: a window with nothing to spare.
    fn narrow() -> Sizes {
        Sizes::at(1.0)
    }

    /// The toolbar's layout `width` wide, Retry shown when `retry`, with a notice longer than
    /// any room (cut to [`NOTICE_MOST`]) when `notices`, no input meter.
    fn spread_at(width: f32, retry: bool, notices: bool) -> Spread {
        Spread::of(Shape {
            width,
            retry,
            notices: notices.then_some(NOTICE_MOST),
            input_meter: false,
        })
    }

    // 6d K5, pins moved (were 252 and 286): the pattern menu opens under the name's box: 8 px
    // of side padding, the transport (four 38 px buttons 2 px apart: 158, or 174 with the 54 px
    // input meter under Record), 2 + a 1 px line + 2, the 76 px tempo, 2 + 1 + 2, the 20 px
    // previous arrow: x 272, or 288 with the meter. (Pins moved: 296 and 316 with the
    // 40 px buttons.) So in a window with nothing to spare; a wider one moves it right by the
    // two gaps' shares. Pins moved (were 304 and
    // 322): the arrow is joined to the box (no 2 px between them), and the least space beside
    // each line is 2 px (was 3). Recording v2, pins moved (were 298 and 316): the 54 px column
    // beside Record (Notes over Audio over Capture) and its 2 px. Pins moved at 0.9 of the size
    // (asked 2026-10-27; were 298 and 316, 354 and 372 with 42 px buttons, the 60 px
    // meter, the 84 px tempo, the 22 px arrow and the 54 px column): the 50 px column.
    #[test]
    fn the_pattern_name_starts_past_the_transport_tempo_and_arrow() {
        assert_eq!(narrow().take_column_width(Column::Words), 50.0);
        // With the badge on Record (no column) the name's box lies where it lay
        // before Recording v2.
        for (input_meter, x) in [(false, 272.0), (true, 288.0)] {
            let least = least_width(&groups(
                Tier::Full,
                Column::Badge,
                true,
                None,
                false,
                input_meter,
                &narrow(),
            ));
            let spread = Spread::of(Shape {
                width: least,
                retry: false,
                notices: None,
                input_meter,
            });
            assert_eq!(spread.column, Column::Badge);
            assert_eq!(spread.pattern_name_x(), x);
        }
        for (input_meter, x) in [(false, 324.0), (true, 340.0)] {
            let least = least_width(&groups(
                Tier::Full,
                Column::Words,
                true,
                None,
                false,
                input_meter,
                &narrow(),
            ));
            let shape = Shape {
                width: least,
                retry: false,
                notices: None,
                input_meter,
            };
            assert_eq!(Spread::of(shape).pattern_name_x(), x);
            // Toolbar scale, pin moved (was least + 700 px, x + 200): a window wider than
            // everything the bar can show at once grows the controls before the gaps, so the
            // gaps alone take what is spared only below that, here 281 px (40 a gap, in even
            // pixels; a pixel over 280, as a sum of measured widths in f32 can come out a hair
            // under its value, and 279.99997 px is 38 a gap).
            let wider = Spread::of(Shape {
                width: least + 281.0,
                ..shape
            });
            assert_eq!(wider.sizes().scale, 1.0);
            assert_eq!(wider.pattern_name_x(), x + 2.0 * 40.0);
            let grown = Spread::of(Shape {
                width: least + 700.0,
                ..shape
            });
            assert!(grown.sizes().scale > 1.0, "{:?}", grown.sizes());
        }
    }

    // Across a wider window every group keeps its width and the gaps
    // between them share what the window has to spare equally, in whole pixels; the last gap,
    // before File, Edit and Help, takes what the rounding leaves, so the first group starts at the
    // side padding and File ends at the other. A notice shorter than the room keeps its own
    // width. In even pixels, so the line in a gap's middle has as much space
    // on its left as on its right (the last gap's rest is under two pixels a gap).
    // With the controls at their most and more to spare than the gap they reach it with, the
    // gaps are that gap ([`PACKED_EXTRA`] past the least) and the last takes the rest, its line
    // beside File while the notices or Retry show, else beside the panel buttons.
    #[test]
    fn the_groups_spread_across_the_toolbar() {
        // Toolbar scale: the widths of windows in use (1000, 1280, a full screen's 2259)
        // and wider, the controls grown at most of them, an armed input's meter on some.
        for (width, notices, retry, input_meter) in [
            (1920.0, None, false, false),
            (1920.0, Some(200.0), true, false),
            (2560.0, Some(NOTICE_MOST), false, false),
            (1000.0, None, false, false),
            (1333.0, Some(97.5), false, false),
            (880.0, None, true, false),
            (1280.0, None, false, true),
            (1400.0, None, false, false),
            (1400.0, Some(NOTICE_MOST), false, false),
            (1400.0, Some(150.0), true, true),
            (2259.0, None, false, false),
            (2259.0, Some(150.0), false, true),
            (4510.0, None, false, false),
            (4510.0, Some(NOTICE_MOST), true, true),
        ] {
            let spread = Spread::of(Shape {
                width,
                retry,
                notices,
                input_meter,
            });
            let spans: Vec<std::ops::Range<f32>> = spread
                .groups()
                .iter()
                .map(|&(group, group_width)| {
                    let span = spread.span(group).expect("shown");
                    let kept = span.end - span.start;
                    assert!((kept - group_width).abs() < 1e-3, "{group:?}: {kept}");
                    span
                })
                .collect();
            assert_eq!(spans[0].start, TOOLBAR_SIDE_PADDING);
            let end = spans.last().expect("File, Edit and Help").end;
            assert!(
                (end - (width - TOOLBAR_SIDE_PADDING)).abs() < 1e-3,
                "{width}: {end}"
            );
            let gaps: Vec<f32> = spans
                .windows(2)
                .map(|pair| pair[1].start - pair[0].end)
                .collect();
            let (last, rest) = gaps.split_last().expect("gaps");
            println!("{width} px: gaps {gaps:?}, packed {}", spread.packed);
            assert!(
                rest.iter().all(|gap| (gap - rest[0]).abs() < 1e-3),
                "{gaps:?}"
            );
            assert_eq!((rest[0] - GAP) % 2.0, 0.0, "even pixels: {gaps:?}");
            if spread.packed {
                assert_eq!(spread.sizes().scale, SCALE_MOST, "{width} px");
                assert_eq!(rest[0], GAP + PACKED_EXTRA, "{width} px: {gaps:?}");
                assert!(*last > rest[0], "{gaps:?}");
                let lines = spread.divider_xs();
                let line = lines[lines.len() - 1];
                let count = spans.len();
                let (before, after) = (
                    line - spans[count - 2].end,
                    spans[count - 1].start - line - 1.0,
                );
                let half = BAR_SPACING + PACKED_EXTRA / 2.0;
                if spread.groups()[count - 2].0 == Group::Status {
                    assert_eq!(after, half, "{width} px: the line beside File");
                } else {
                    assert_eq!(before, half, "{width} px: the line beside its group");
                }
            } else {
                assert!(
                    *last >= rest[0] && *last - rest[0] < 2.0 * gaps.len() as f32,
                    "{gaps:?}"
                );
            }
            for (index, line) in spread
                .divider_xs()
                .into_iter()
                .enumerate()
                .take(gaps.len() - 1)
            {
                let (before, after) =
                    (line - spans[index].end, spans[index + 1].start - line - 1.0);
                // To a thousandth of a pixel, as the gaps above: the groups' places are sums of
                // measured widths in f32 (help guide H1: at 1400 px, a scale step lower with
                // the Help button, one side came out 22.999939 px against 23).
                assert!(
                    (before - after).abs() < 1e-3,
                    "the line after group {index}: {before} {after} {gaps:?}"
                );
            }
            if let Some(natural) = notices {
                assert_eq!(
                    spread.notice_width, natural,
                    "{width}: the notice's own width"
                );
            }
        }
        // 1920 px with nothing but the groups: more room to spare than the gaps' share where the
        // controls reach their most, so the groups stand left, 45 px apart. Below the most, and
        // with less to spare than that (a notice of its most at 1920 px), the gaps share it.
        for (width, notices, packed) in [
            (1920.0, false, true),
            (2259.0, false, true),
            (4510.0, true, true),
            (1920.0, true, false),
            (1400.0, false, false),
        ] {
            assert_eq!(
                spread_at(width, false, notices).packed,
                packed,
                "{width} px"
            );
        }
        assert_eq!(spread_at(1920.0, false, false).extra, 40.0);
    }

    // A request made at full screen (2259 px: "push everything to the right of the
    // play/pause/record UI to the left, other than Edit and File, so that it fills the empty
    // spaces"). Where the controls first reach their most (everything the bar can show at once
    // at 1.6, about 1659 px at 0.9 of the size; 1722 before), the bar with nothing but its groups shares a notice's room and its
    // gap, 288 + 5 px, between its seven gaps in even pixels: 40 px each, the last 53, and that
    // look stays. Wider, every group from the transport to the panel buttons, and the line after
    // each, stays exactly where it lay there (1920, 2259, 2560, 4510 px), 45 px
    // apart, and File, Edit and Help stand at the right edge; a notice and Retry stand 45 px past
    // the panel buttons (the notice's text first, left in its cell), the line before File 22 px from
    // it, the cell between taking what the window has to spare.
    #[test]
    fn past_the_most_the_groups_stand_left_and_the_notices_cell_takes_the_rest() {
        let first = least_width(&groups(
            Tier::Full,
            Column::Words,
            true,
            Some(NOTICE_MOST),
            false,
            false,
            &Sizes::at(SCALE_MOST),
        ));
        let reached = spread_at(first, false, false);
        assert_eq!(reached.sizes().scale, SCALE_MOST);
        assert!(spread_at(first - 1.0, false, false).sizes().scale < SCALE_MOST);
        println!("the most first at {first} px: {reached:?}");
        assert!(!reached.packed);
        assert_eq!((reached.extra, reached.last_extra), (40.0, 53.0));
        let packed_groups = |spread: &Spread| -> Vec<(Group, f32)> {
            spread
                .groups()
                .iter()
                .take_while(|(group, _)| !matches!(group, Group::Status | Group::Menus))
                .map(|&(group, _)| (group, spread.left(group).expect("shown")))
                .collect()
        };
        let lefts = packed_groups(&reached);
        assert_eq!(lefts.len(), 7, "the transport to the panel buttons");
        let lines = reached.divider_xs();
        let panels_end = reached.span(Group::Panels).expect("shown").end;
        let half = BAR_SPACING + PACKED_EXTRA / 2.0;
        for width in [first.ceil() + 2.0, 1920.0, 2259.0, 2560.0, 4510.0] {
            let plain = spread_at(width, false, false);
            assert!(plain.packed, "{width} px");
            assert_eq!(packed_groups(&plain), lefts, "{width} px");
            assert_eq!(plain.divider_xs()[..6], lines[..6], "{width} px");
            assert_eq!(
                plain.divider_xs()[6],
                panels_end + half,
                "{width} px: the panel buttons' line keeps to them"
            );
            // To a thousandth of a pixel: the groups' places are sums of measured widths in f32
            // (at 0.9 of the size, 2251.0002 against 2251 at 2259 px).
            let menus = plain.span(Group::Menus).expect("shown");
            assert!(
                (menus.end - (width - TOOLBAR_SIDE_PADDING)).abs() < 1e-3,
                "{width} px: {menus:?}"
            );
            for (retry, notices) in [
                (false, Some(150.0)),
                (true, Some(NOTICE_MOST)),
                (true, None),
            ] {
                let busy = Spread::of(Shape {
                    width,
                    retry,
                    notices,
                    input_meter: false,
                });
                // Narrower, a notice or Retry leaves less than that gap's share to spare.
                if width < 2000.0 {
                    continue;
                }
                assert!(busy.packed, "{width} px, {retry} {notices:?}");
                assert_eq!(packed_groups(&busy), lefts, "{width} px");
                let status = busy.span(Group::Status).expect("the notices or Retry");
                assert_eq!(status.start, panels_end + GAP + PACKED_EXTRA, "{width} px");
                let busy_lines = busy.divider_xs();
                assert_eq!(busy_lines[6], panels_end + half, "{width} px");
                let menus = busy.span(Group::Menus).expect("shown");
                assert!(
                    (busy_lines[7] + 1.0 + half - menus.start).abs() < 1e-3,
                    "{width} px"
                );
                assert!(
                    (menus.end - (width - TOOLBAR_SIDE_PADDING)).abs() < 1e-3,
                    "{width} px: {menus:?}"
                );
                if let Some(natural) = notices {
                    assert_eq!(busy.notice_width, natural.min(NOTICE_MOST), "{width} px");
                }
            }
        }
        // A full screen's 2259 px, for the record.
        let owner = spread_at(2259.0, false, false);
        println!(
            "2259 px: {:?}, lines {:?}, the cell before File {} px",
            packed_groups(&owner),
            owner.divider_xs(),
            owner.last_extra + GAP
        );
    }

    // 6d K5: at 1000 px everything shows with the full labels, the panel buttons too, beside a
    // notice's least room; below that the column beside Record gives way first (its words, then its
    // letters, for the badge on Record), then the panel buttons fold, then the labels shorten, and
    // at 880 px with Retry the short labels leave Retry, File, Edit and Help ("?") whole and a
    // notice its least room. Recording v2 and the Help button (47.584 px) had pushed every fold
    // past 1000 px (the panel buttons folded beside a notice there, the full labels with Retry and
    // a notice held only from 1016.5 px); at 0.9 of the size (asked 2026-10-27) they lie
    // under it again. Each fold is worked out here from the least widths of the looks on either
    // side of it (printed), so the pins follow the sizes; the narrow pin holds every place.
    #[test]
    fn everything_fits_1000_px_and_the_labels_shorten_below() {
        let least = |tier, column, panels, notice: Option<f32>, retry| {
            least_width(&groups(
                tier,
                column,
                panels,
                notice,
                retry,
                false,
                &narrow(),
            ))
        };
        let room = Some(NOTICE_ROOM);
        let words_quiet = least(Tier::Full, Column::Words, true, None, false);
        let words_beside = least(Tier::Full, Column::Words, true, room, false);
        let badge_beside = least(Tier::Full, Column::Badge, true, room, false);
        let full_with_retry = least(Tier::Full, Column::Badge, false, room, true);
        println!(
            "the words beside Record from {words_quiet} px, beside a notice from {words_beside}; \
             the panel buttons beside a notice from {badge_beside}; the full labels with Retry and \
             a notice from {full_with_retry}"
        );
        for notices in [false, true] {
            let spread = spread_at(1000.0, false, notices);
            assert_eq!(
                (spread.tier, spread.panels),
                (Tier::Full, true),
                "{notices}"
            );
            assert!(!notices || spread.notice_width >= NOTICE_ROOM);
        }
        assert_eq!(spread_at(words_quiet, false, false).column, Column::Words);
        assert_ne!(
            spread_at(words_quiet - 0.1, false, false).column,
            Column::Words
        );
        assert_eq!(spread_at(words_beside, false, true).column, Column::Words);
        let given_way = spread_at(words_beside - 0.1, false, true);
        assert_ne!(given_way.column, Column::Words);
        assert!(
            given_way.panels,
            "the column gives way before the panel buttons fold"
        );
        let beside = spread_at(badge_beside, false, true);
        assert_eq!(
            (beside.tier, beside.column, beside.panels),
            (Tier::Full, Column::Badge, true)
        );
        assert!(!spread_at(badge_beside - 0.1, false, true).panels);
        assert!(narrow().take_column_width(Column::Letters) < 24.0);
        // With Retry and a notice the full labels hold down to under 1000 px, the column given up
        // for the badge first.
        assert!(full_with_retry < 1000.0, "{full_with_retry}");
        let audio_off = spread_at(1000.0, true, true);
        assert_eq!(
            (audio_off.tier, audio_off.column),
            (Tier::Full, Column::Badge),
            "the column gives up its words before the labels shorten"
        );
        assert_eq!(spread_at(full_with_retry, true, true).tier, Tier::Full);
        assert_eq!(
            spread_at(full_with_retry - 0.1, true, true).tier,
            Tier::Short
        );
        let short = spread_at(880.0, true, true);
        assert_eq!(short.tier, Tier::Short);
        assert!(short.notice_width >= NOTICE_ROOM, "{}", short.notice_width);
        let wide = spread_at(1051.0, false, true);
        let switch = wide.keyboard_switch();
        assert!(switch.x + switch.width < wide.left(Group::Panels).expect("panels"));
        assert_eq!(switch.height, BUTTON_HEIGHT);
        assert!(
            narrow().keyboard_switch_width(Tier::Short)
                < narrow().keyboard_switch_width(Tier::Full)
        );
    }

    // B4: with the Edit button beside File, a toolbar 880 px wide still leaves the notices room
    // for the audio notice cut with an ellipsis beside Retry, as the p8 probe looks for; File
    // first (asked 2026-10-27: Edit was), Edit the menus' spacing right of it and Help
    // the spacing right of Edit, Retry a line and its spacing left of File (no room to spare
    // there), the Edit menu's right edge under Edit's and the File menu's under File's.
    #[test]
    fn the_edit_button_leaves_the_notices_room_at_880_px() {
        let spread = spread_at(880.0, true, true);
        let room = spread.notice_width;
        let fitted = fit("Audio off: no audio output device", room, &line_width);
        println!("the notices' room at 880 px: {room} px, showing {fitted:?}");
        assert!(fitted.is_some(), "{room} px hold no notice");
        let [retry, edit, file, help] = retry_edit_file_and_help(&spread);
        // Help guide H1: Help at the right padding (File was), "?" at the short labels; Edit
        // the menus' spacing left of it (File was, before the swap).
        assert_eq!(help.end, 872.0);
        assert_eq!(edit.end + MENU_SPACING, help.start);
        // 6d K5, pins moved: the two 4 px apart (were a group's 7), Retry a line and its
        // spacing (2 + 1 + 2; 3 + 1 + 3 before) left of the first, File now.
        assert_eq!(file.end + MENU_SPACING, edit.start);
        assert!(
            (retry.end + GAP - file.start).abs() < 1e-3,
            "{retry:?} {file:?}"
        );
        assert!((880.0 - spread.edit_menu_right() - edit.end).abs() < 1e-3);
        assert!((880.0 - spread.file_menu_right() - file.end).abs() < 1e-3);
    }

    // B7: while there is no loop range, the Loop button's tip says how to make one.
    #[test]
    fn the_loop_buttons_tip_says_how_to_set_a_range() {
        assert_eq!(loop_tip(None), "Drag in the ruler to set a loop range");
        assert_eq!(loop_tip(Some(false)), loop_tip(Some(true)));
        assert!(loop_tip(Some(true)).starts_with("Loop"));
    }

    // A pattern's name fits its box (asked 2026-10-27: "the name must always fit"): one
    // that fits at the labels' size shows so ("Pattern 1", "Pattern 12": the tenth pattern's
    // name); a longer one shrinks a pixel at a time, whole, down to the least size (a name of 24
    // characters is whole at a size between the two, and a pixel larger would not fit); a longer
    // one still is cut with an ellipsis at the least size, its first words kept. At a scale of 1
    // and at the most, at the full labels; and at the short ones, where the box is 47 px.
    #[test]
    fn a_long_pattern_name_shrinks_then_is_cut_with_an_ellipsis() {
        for s in [narrow(), Sizes::at(SCALE_MOST)] {
            for name in ["Pattern 1", "Pattern 12"] {
                assert_eq!(
                    shown_pattern_name(name, Tier::Full, s),
                    (name.to_owned(), s.label_text),
                    "{name} at {}",
                    s.scale
                );
            }
            let room = s.pattern_name_room(Tier::Full);
            let (held, size) = shown_pattern_name(PATTERN_NAME_HELD, Tier::Full, s);
            assert_eq!(
                held, PATTERN_NAME_HELD,
                "24 characters whole at {}",
                s.scale
            );
            assert_eq!(PATTERN_NAME_HELD.chars().count(), 24);
            assert!(size >= s.name_least_text && size < s.label_text, "{size}");
            assert!(text_width(PATTERN_NAME_HELD, size + 1.0) > room, "{size}");
            let (long, size) = shown_pattern_name(
                "A very long name for the second verse pattern",
                Tier::Full,
                s,
            );
            assert_eq!(size, s.name_least_text);
            assert!(
                long.ends_with('\u{2026}') && long.starts_with("A very long"),
                "{long}"
            );
            assert!(text_width(&long, size) <= room);
        }
        assert_eq!(
            (
                narrow().name_least_text,
                Sizes::at(SCALE_MOST).name_least_text
            ),
            (10.0, 14.0)
        );
        // Pin moved: the full box held "Pattern 12" at the labels' size (79 px at 0.9 of the
        // size), now 24 characters at 10 px.
        println!(
            "the full box: {} px at 1, {} at the most",
            narrow().pattern_name_width(Tier::Full),
            Sizes::at(SCALE_MOST).pattern_name_width(Tier::Full)
        );
        let (short, size) = shown_pattern_name("Pattern 1", Tier::Short, narrow());
        assert!(text_width(&short, size) <= narrow().pattern_name_room(Tier::Short));
    }

    // Any name, of any length, fits its box whole or cut, at a size between the least and the
    // labels': every start of a long name of mixed words, from none to 80 characters, at a scale
    // of 1 and at the most, at the full labels and the short ones.
    #[test]
    fn a_pattern_name_of_any_length_fits_its_box() {
        let words = "Chorus bassline (take 2), the bridge's WIDE pad and an ending that goes on";
        for s in [narrow(), Sizes::at(1.3), Sizes::at(SCALE_MOST)] {
            for tier in [Tier::Full, Tier::Short] {
                let room = s.pattern_name_room(tier);
                for length in 0..=80 {
                    let name: String = words.chars().cycle().take(length).collect();
                    let (shown, size) = shown_pattern_name(&name, tier, s);
                    assert!(
                        text_width(&shown, size) <= room,
                        "{name:?} at {} {tier:?}: {shown:?} at {size}",
                        s.scale
                    );
                    assert!(
                        (s.name_least_text.min(s.label_text)..=s.label_text).contains(&size),
                        "{size}"
                    );
                    if !shown.ends_with('\u{2026}') {
                        assert_eq!(shown, name);
                    }
                }
            }
        }
    }

    // C3: a typed tempo in BPM, with up to three decimals after a point or a comma, becomes
    // milli-BPM, inside 10 to 522 BPM; anything else is refused with why.
    #[test]
    fn a_typed_tempo_reads_as_milli_bpm_in_range() {
        let table = [
            ("87", Ok(87_000)),
            (" 87.5 ", Ok(87_500)),
            ("87,25", Ok(87_250)),
            ("120.000", Ok(120_000)),
            ("10", Ok(10_000)),
            ("522", Ok(522_000)),
            ("174.505", Ok(174_505)),
        ];
        for (typed, want) in table {
            assert_eq!(parse_tempo(typed).map(|tempo| tempo.0), want, "{typed:?}");
        }
        for typed in [
            "", "abc", "9.999", "522.001", "1000", "87.1234", "-90", "8 7", ".5",
        ] {
            let refused = parse_tempo(typed);
            assert!(refused.is_err(), "{typed:?} gave {refused:?}");
        }
        assert_eq!(
            parse_tempo("600"),
            Err("Tempo goes from 10 to 522 BPM".to_owned())
        );
    }

    #[test]
    fn tempo_reads_with_three_decimals() {
        assert_eq!(format_tempo(MilliBpm(120_000)), "120.000");
        assert_eq!(format_tempo(MilliBpm(174_500)), "174.500");
        assert_eq!(format_tempo(MilliBpm(140_001)), "140.001");
        assert_eq!(format_tempo(MilliBpm(10_000)), "10.000");
        assert_eq!(format_tempo(MilliBpm(522_000)), "522.000");
    }

    // At ppq 96 a bar is 384 ticks and a step 24: tick 434 is bar 2, step 3 (48 ticks in),
    // tick 2 of that step.
    #[test]
    fn position_reads_bar_step_tick_from_one() {
        assert_eq!(format_position(Ticks(0), 96), "1:01:00");
        assert_eq!(format_position(Ticks(120), 96), "1:06:00");
        assert_eq!(format_position(Ticks(383), 96), "1:16:23");
        assert_eq!(format_position(Ticks(434), 96), "2:03:02");
    }

    // The notices are one line of at most NOTICE_MOST (about 38
    // characters at 14 px; 36 at 16 px before 0.9 of the size): a longer one is cut there with an ellipsis, a short one keeps its
    // width. A cut notice fits the width it was cut for, and one more character would not.
    #[test]
    fn a_notice_is_cut_to_one_short_line() {
        let text = "sample samples/an-ambient-pad.wav cannot be read; its channel plays silence";
        assert!(
            line_width(text) > NOTICE_MOST,
            "the notice is longer than the most"
        );
        let width = notice_width(text);
        let cut = fit(text, NOTICE_MOST, &line_width).expect("shown");
        let kept = cut.text.trim_end_matches('\u{2026}');
        println!(
            "{width} px: {:?} ({} characters)",
            cut.text,
            cut.text.chars().count()
        );
        assert!(cut.cut && text.starts_with(kept));
        assert!(width <= NOTICE_MOST && width == line_width(&cut.text));
        assert!(
            (30..=42).contains(&cut.text.chars().count()),
            "about 38 characters"
        );
        let one_more: String = text.chars().take(kept.chars().count() + 1).collect();
        assert!(line_width(&format!("{one_more}\u{2026}")) > NOTICE_MOST);
        assert_eq!(notice_width("Underruns: 3"), line_width("Underruns: 3"));
    }

    // The panel buttons (61 px; 64 before 0.9 of the size) show when the room left has them and, while there are
    // notices, a gap of 5 px (7 before) and 48 px more for those; else they
    // fold away and the notices
    // take it all.
    #[test]
    fn the_panel_buttons_fold_away_before_the_notices_lose_their_room() {
        assert_eq!(narrow().panels_width(), 61.0);
        // The panel buttons fold where they folded before Recording v2, the
        // column beside Record having given way to the badge on Record first.
        let least = |notice| {
            least_width(&groups(
                Tier::Full,
                Column::Badge,
                true,
                notice,
                false,
                false,
                &narrow(),
            ))
        };
        let with = least(Some(NOTICE_ROOM));
        assert_eq!(with - least(None), GAP + NOTICE_ROOM);
        assert!(spread_at(with, false, true).panels);
        assert!(!spread_at(with - 0.1, false, true).panels);
        assert!(spread_at(least(None), false, false).panels);
        assert!(!spread_at(least(None) - 0.1, false, false).panels);
    }

    // Each tip names its panel and the digit and function key that toggle it, the keys the
    // window's shortcuts give it (the playlist's digit was 3, the piano
    // roll's 2).
    #[test]
    fn each_panel_buttons_tip_names_its_panel_and_keys() {
        let keys = Keys::default();
        assert_eq!(panel_tip(Panel::Playlist, &keys), "Playlist \u{b7} 2 or F5");
        assert_eq!(
            panel_tip(Panel::PianoRoll, &keys),
            "Piano roll \u{b7} 3 or F7"
        );
        assert_eq!(panel_tip(Panel::Browser, &keys), "Library \u{b7} 5 or F8");
        assert_eq!(
            panel_tip(Panel::Settings, &keys),
            "Settings \u{b7} 6 or F10"
        );
        let rects = panel_button_rects(&spread_at(1000.0, false, false));
        assert_eq!(rects.len(), 6);
        // Two rows of three, the groups' 46 px centered on the 64 px toolbar: 9..31 and 33..55
        // (pins moved: 7..35 and 37..65 in 58 px before, 5..27 and 29..51 on the 56 px
        // toolbar, and 10..35 and 37..62 on the 72 px one), 19 px buttons 2 px apart.
        assert_eq!((rects[0].1.y, rects[3].1.y), (9.0, 33.0));
        assert_eq!(rects[0].1.height, 22.0);
        assert_eq!(rects[3].1.y + rects[3].1.height, 55.0);
        assert_eq!(rects[1].1.x - rects[0].1.x, 21.0);
    }

    /// One line of where the toolbar lays everything out for `shape`, as
    /// tests/fixtures/toolbar_narrow_layout.txt holds it: the look, the notices' width, the
    /// groups, their places and lines, the gaps' shares, and every place read off them (the
    /// name's box, the keyboard switch, Record, the column beside it, the Edit menu, the panel
    /// buttons, Retry, Edit, File and Help, as `retry_edit_file_and_help` names them).
    fn layout_line(shape: Shape) -> String {
        let spread = Spread::of(shape);
        let lefts: Vec<_> = spread
            .groups()
            .iter()
            .map(|&(group, _)| spread.left(group))
            .collect();
        format!(
            "{shape:?} => {:?} {:?} {} {} {:?} {:?} {:?} {} {} {} {:?} {} {} {:?} {:?} {:?} {:?}\n",
            spread.tier,
            spread.column,
            spread.panels,
            spread.notice_width,
            spread.groups(),
            lefts,
            spread.divider_xs(),
            spread.extra,
            spread.last_extra,
            spread.pattern_name_x(),
            spread.keyboard_switch(),
            spread.record_x(),
            spread.take_column_x(),
            spread.record_button(),
            spread.edit_menu_right(),
            panel_button_rects(&spread),
            retry_edit_file_and_help(&spread),
        )
    }

    // Toolbar scale: a window up to where the controls start to grow lays the toolbar out to
    // the pixel as before, every tier, fold and notice's room (880 to 1200 px; with and without
    // notices, Retry and the input meter). The pin holds what the layout gave at 8247957, before
    // the scale, written by the same line; it is the "as before", not a value worked out here.
    // Help guide H1, pin rewritten by the same line with the Help button right of File: before
    // that, every one of the 60 lines of 8247957's pin came out of the new layout to the pixel
    // at its width plus Help's room (its label and the menus' spacing: 47.584 px at the full
    // labels, "?" 22.264 px at the short ones), Help's place left out, so Help moved every fold
    // by its own room and nothing else. Pin rewritten by the same line at 0.9 of the size (asked
    // 2026-10-27: every size of the bar 0.9 of what it was, the gaps kept), its lines
    // checked apart from the layout: each group starts where the one before ends plus a gap of
    // 5 px and its share, Help ends at the right padding, and every group and line spans y 9..55.
    // Rewritten again with File first (asked 2026-10-27): against the line
    // before, each line alike to the Edit menu's place, File starting where Edit started and
    // Edit ending where File ended, each as wide as before, Retry and Help where they were. And
    // again with the pattern's box holding 24 characters (164 px at the full labels, 45 more):
    // against the line before, each line whose look stayed has the pattern 45 px wider at the
    // full labels and every other group as wide; and against the pin at d5f9f02 (before 0.9),
    // no window of the 60 shows less than it did (33 show more: the column's words or letters,
    // the panel buttons or the full labels).
    #[test]
    fn narrow_windows_lay_the_bar_out_as_before() {
        let pinned = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/toolbar_narrow_layout.txt"
        ));
        let mut now = String::new();
        for width in [880.0, 930.0, 1000.0, 1051.0, 1200.0] {
            for notices in [None, Some(97.5), Some(NOTICE_MOST)] {
                for retry in [false, true] {
                    for input_meter in [false, true] {
                        let shape = Shape {
                            width,
                            retry,
                            notices,
                            input_meter,
                        };
                        assert_eq!(Spread::of(shape).sizes(), narrow(), "{shape:?}");
                        now.push_str(&layout_line(shape));
                    }
                }
            }
        }
        assert_eq!(pinned.lines().count(), 60);
        for (line, (was, is)) in pinned.lines().zip(now.lines()).enumerate() {
            assert_eq!(was, is, "line {}", line + 1);
        }
    }

    // Toolbar scale: the scale is 1 while the window is no wider than everything the bar can
    // show at once needs at 1 (the full labels, the words beside Record, the panel buttons, a
    // notice cut to its most, 288 px: about 1244 px at 0.9 of the size; 1290 before, when a first
    // start's 1280 px window kept the bar at 1), then grows in twentieths with the width (1400 px:
    // a little), up to 1.6, which a 1920 px window already has, and so do a full screen's
    // 2259 px, 2560 and 4510. It never shrinks as the window widens, and it is the window's width
    // alone that sets it: the same with notices, Retry or the input meter, so Record and every
    // other control keep their place and size when one comes.
    #[test]
    fn the_scale_follows_the_window_width_alone() {
        let scale = |width| spread_at(width, false, false).sizes().scale;
        for width in [880.0, 1000.0, 1200.0] {
            assert_eq!(scale(width), 1.0, "{width} px");
        }
        let all_at = |scale: f32| {
            least_width(&groups(
                Tier::Full,
                Column::Words,
                true,
                Some(NOTICE_MOST),
                false,
                false,
                &Sizes::at(scale),
            ))
        };
        let starts = all_at(1.0);
        println!(
            "everything at a scale of 1: {starts} px; at 1.6: {} px",
            all_at(1.6)
        );
        assert!((1201.0..1400.0).contains(&starts), "{starts}");
        assert_eq!(scale(starts), 1.0);
        assert_eq!(scale(all_at(1.05)), 1.05);
        let middle = scale(1400.0);
        assert!(middle > 1.0 && middle < SCALE_MOST, "1400 px: {middle}");
        assert!(all_at(middle) <= 1400.0 && all_at(middle + 0.05) > 1400.0);
        assert!(all_at(1.6) <= 1920.0, "{}", all_at(1.6));
        for width in [1920.0, 2259.0, 2560.0, 4510.0] {
            assert_eq!(scale(width), SCALE_MOST, "{width} px");
        }
        let mut last = 1.0;
        // Every 19 px from 800 to 4970 (the scale's twelve steps lie about 35 px of window apart
        // on average, 1244 to 1659 px): few enough layouts that the test stays light beside the
        // others, which click by the wall clock.
        for step in 0..220 {
            let width = 800.0 + 19.0 * step as f32;
            let now = scale(width);
            assert!(now >= last, "{width} px: {now} after {last}");
            last = now;
            let quiet = spread_at(width, false, false);
            for (retry, notices, input_meter) in [
                (true, None, false),
                (false, Some(40.0), false),
                (true, Some(NOTICE_MOST), true),
            ] {
                let busy = Spread::of(Shape {
                    width,
                    retry,
                    notices,
                    input_meter,
                });
                assert_eq!(busy.sizes(), quiet.sizes(), "{width} px");
                assert_eq!(busy.record_button(), quiet.record_button(), "{width} px");
            }
        }
    }

    // Toolbar scale, known answers worked by hand: at 1.6 each length is its base times 1.6,
    // rounded (the transport's buttons 38 → 61 px and glyphs 16 → 26, the input meter 54 → 86,
    // the arrows 20 → 32 and their glyphs 13 → 21, the octave column 23 → 37, the keyboard glyph
    // 19 × 13 → 30 × 21, its gap 5 → 8, the labels' sides 5 → 8, the column's sides 4 → 6, the
    // name's padding 5 → 8, the panel buttons' sides 4 → 6); each text grows as far as its row
    // holds it (the labels 14 → 20 px, Pat and Song 14 → 18 in their 23 px rows, Notes, Audio
    // and Capture 11 → 12 in their 14 px ones, Retry 11 → 14, the octave's + and − 14 → 16 and
    // its name 11 → 13, Record's badge 10 → 13); the panel icons 11 → 16 px (17.6, held to what
    // leaves 3 px over and under in their 22 px rows), so a panel button is 16 + 2 × 6 = 28 px
    // wide and the grid 3 × 28 + 2 × 2 = 88; the readouts' digits 16 → 23 px and the readouts
    // with them, 76 × 23 / 16 = 109.25 → 110 px and 86 × 23 / 16 = 123.6 → 124, 4 × 23 / 16 =
    // 5.75 → 6 px either side. At 1 every size is its constant. (At 0.9 of the size, asked
    // 2026-10-27: were 42 → 67, 18 → 29, 60 → 96, 22 → 35, 14 → 22, 26 → 42, 21 × 14 →
    // 34 × 22, 6 → 10, 6 → 10, 16 → 22, 16 → 20, 12 → 14, 12 → 16, 15 → 18, 12 → 14, 11 → 14,
    // 12 → 19 in 25 px rows, 31, 97, 18 → 26, 122, 139.)
    #[test]
    fn each_size_grows_with_the_scale_as_far_as_its_row_holds() {
        let most = Sizes::at(SCALE_MOST);
        let lengths = [
            ("transport button", most.transport_button, 61.0),
            ("transport glyph", most.transport_icon, 26.0),
            ("input meter", most.input_meter, 86.0),
            ("arrow", most.arrow_width, 32.0),
            ("arrow glyph", most.arrow_icon, 21.0),
            ("octave column", most.octave_width, 37.0),
            ("keyboard glyph width", most.keyboard_glyph.width, 30.0),
            ("keyboard glyph height", most.keyboard_glyph.height, 21.0),
            ("glyph gap", most.glyph_gap, 8.0),
            ("label side", most.label_side, 8.0),
            ("take side", most.take_side, 6.0),
            ("name padding", most.pattern_name_padding, 8.0),
            ("label text", most.label_text, 20.0),
            ("mode text", most.mode_text, 18.0),
            ("take text", most.take_text, 12.0),
            ("retry text", most.retry_text, 14.0),
            ("octave text", most.octave_text, 16.0),
            ("octave name text", most.octave_name_text, 13.0),
            ("badge text", most.badge_text, 13.0),
            ("panel icon", most.panel_icon, 16.0),
            ("panel button", most.panel_button_width(), 28.0),
            ("panel grid", most.panels_width(), 88.0),
            ("readout text", most.readout_text, 23.0),
            ("tempo", most.tempo_width, 110.0),
            ("position", most.position_width, 124.0),
            ("readout padding", most.readout_padding, 6.0),
        ];
        for (what, is, want) in lengths {
            assert_eq!(is, want, "{what} at {SCALE_MOST}");
        }
        assert_eq!(
            most.panel_padding,
            iced::Padding {
                top: 3.0,
                right: 6.0,
                bottom: 3.0,
                left: 6.0
            }
        );
        let one = narrow();
        let bases = [
            (one.transport_button, TRANSPORT_BUTTON),
            (one.transport_icon, TRANSPORT_ICON),
            (one.arrow_width, ARROW_WIDTH),
            (one.label_text, LABEL_SIZE),
            (one.mode_text, LABEL_SIZE),
            (one.take_text, TAKE_TEXT),
            (one.retry_text, RETRY_SIZE),
            (one.readout_text, READOUT_SIZE),
            (one.tempo_width, TEMPO_WIDTH),
            (one.position_width, POSITION_WIDTH),
            (one.panel_icon, PANEL_ICON),
            (one.octave_width, OCTAVE_WIDTH),
        ];
        for (index, (is, base)) in bases.into_iter().enumerate() {
            assert_eq!(is, base, "size {index} at 1");
        }
        assert_eq!(one.panel_padding, PANEL_PADDING);
        // The grown words still fit their boxes: "Pattern 12" whole in the name's box, the
        // keyboard switch's words beside its glyph, the readouts' digits inside their sides.
        assert_eq!(
            shown_pattern_name("Pattern 12", Tier::Full, most).0,
            "Pattern 12"
        );
        let digits = |content: &str| {
            let renderer = iced::advanced::text::Text {
                content,
                bounds: Size::INFINITE,
                size: most.readout_text.into(),
                line_height: Default::default(),
                font: Font::MONOSPACE,
                align_x: Default::default(),
                align_y: iced::alignment::Vertical::Top,
                shaping: Default::default(),
                wrapping: Wrapping::None,
            };
            use iced::advanced::text::Paragraph as _;
            <Renderer as iced::advanced::text::Renderer>::Paragraph::with_text(renderer).min_width()
        };
        for (content, width) in [
            ("120.000", most.tempo_width),
            ("10:01:00", most.position_width),
        ] {
            let room = width - 2.0 * most.readout_padding;
            assert!(
                digits(content) <= room,
                "{content}: {} in {room}",
                digits(content)
            );
        }
    }

    // Toolbar scale: once the controls grow (from about 1290 px) the bar keeps its whole look,
    // the full labels, the words beside Record and the panel buttons, whatever else shows (a
    // notice, Retry, the input meter); a notice keeps at least its least room and, with neither
    // Retry nor the meter, its own width up to the most.
    #[test]
    fn a_grown_bar_keeps_its_look_and_the_notices_their_room() {
        let mut grown = 0;
        for step in 0..100 {
            let width = 1300.0 + 33.0 * step as f32;
            for retry in [false, true] {
                for input_meter in [false, true] {
                    for notices in [None, Some(40.0), Some(NOTICE_MOST)] {
                        let spread = Spread::of(Shape {
                            width,
                            retry,
                            notices,
                            input_meter,
                        });
                        if spread.sizes().scale == 1.0 {
                            continue;
                        }
                        grown += 1;
                        let look = (spread.tier, spread.column, spread.panels);
                        assert_eq!(look, (Tier::Full, Column::Words, true), "{width} px");
                        if let Some(natural) = notices {
                            let least = natural.min(NOTICE_ROOM);
                            assert!(spread.notice_width >= least, "{width} px");
                            if !retry && !input_meter {
                                assert_eq!(spread.notice_width, natural, "{width} px");
                            }
                        }
                    }
                }
            }
        }
        assert!(grown > 1000, "{grown}");
    }

    #[test]
    fn lengths_read_bar_step_tick_from_zero() {
        assert_eq!(format_length(Ticks(24), 96), "0:01:00");
        assert_eq!(format_length(Ticks(96), 96), "0:04:00");
        assert_eq!(format_length(Ticks(434), 96), "1:02:02");
        assert_eq!(format_length(Ticks(1), 96), "0:00:01");
    }
}
