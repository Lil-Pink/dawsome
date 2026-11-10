//! The toolbar along the top of the window, in groups a thin line apart: the transport (Play,
//! Stop, Record with the input meter under it, beside it what a take records, Notes over Audio,
//! over Capture, then the metronome) | the tempo and the time signature | the pattern (its
//! arrows and name) | the play mode (Pat over Song, then Loop) | the play position | the
//! computer keyboard (its labeled switch, the octave's + and − with its name between) | the
//! notices, on one short line, and Retry while audio is off | the panel buttons | File, Edit and
//! Help at the right end (File first). Why the panel buttons beside the menus: the window's
//! controls stand together at the right end, the notices' room between them and the music's
//! controls.
//!
//! Every group's width is fixed by the toolbar itself (each button sized to its label measured,
//! the readouts and the name box set; the notices' line its text, cut to [`NOTICE_MOST`]), the
//! same at every window width, as every other panel's sizes are: the app's zoom (Settings →
//! Zoom, Ctrl+= and Ctrl+-) grows the whole window, the toolbar with it. What the window has to
//! spare, the gaps between groups share equally, the line in each gap's middle, up to
//! [`PACKED_EXTRA`] each; past that the groups up to the keyboard stand to the left that gap
//! apart, the panel buttons and the menus stand at the right end that gap apart, and the rest
//! goes to the notices' cell between them ([`Spread`]). Why one
//! size: the controls once grew with the window's width (up to 1.6 times, from about 1244 px),
//! and that width is in logical pixels, so a 4K screen at 2× and a 1080p one at 1× drew the same
//! bar while a laptop at 125 % drew one unlike either; the ramp was retired for the zoom.
//! Every group is [`BUTTON_HEIGHT`] tall and centered in the bar, and so
//! are the lines between them, so each group fills the cell its lines bound: the readouts, the
//! pattern's arrows and name (one piece in one frame), Pat over Song (a two-row switch) beside
//! Loop, the keyboard switch on one line beside the octave's column, the panel buttons' two
//! rows, the notices' line and File, Edit and Help. So where each control lies is a function of the window's width and of
//! what shows beside the fixed groups ([`Shape`]), which the pixel probes, the menus and the
//! first-run note read. When the window is too narrow, the panel buttons fold away first, as
//! before; then the labels shorten ([`Tier::Short`]: the keyboard switch says only On or Off,
//! the pattern's name box narrows).

use std::borrow::Cow;
use std::sync::OnceLock;

use dawsome_engine::project::{PatternId, TEMPO_RANGE};
use dawsome_engine::time::{self, MeterMap, MilliBpm, Ticks, TimeSignature};
use iced::alignment::Horizontal;
use iced::widget::text::Wrapping;
use iced::widget::{
    button, canvas, column, container, mouse_area, row, text, text_input, tooltip, Id,
};
use iced::{mouse, Bottom, Center, Color, Element, Fill, Point, Rectangle, Renderer, Size, Theme};

use crate::app::{
    FileMessage, Message, PatternMessage, PlayMode, RecordMessage, TakeKind, PATTERN_RENAME_INPUT,
};
use crate::keys::{Action, Keys};
use crate::ui::fonts::size;
use crate::ui::hint::{hover_area, with_tip, Hover, Tool};
use crate::ui::layout::{TOOLBAR_HEIGHT, TOOLBAR_SIDE_PADDING};
use crate::ui::mixer::MINUS_6_DB;
use crate::ui::mixer_layout;
use crate::ui::panels::{Panel, Panels};
use crate::ui::style;
use crate::ui::text_fit::{fit, text_width};
use crate::zoom::Zoom;

/// The least space either side of the line between two of the toolbar's groups, and the line.
/// Why this tight: at 16 px text and 42 px transport buttons everything fitted 1000 px with the
/// panel buttons and a notice's room only so. Kept when the controls became 0.9 of their size
/// (about 10 % smaller both ways, the spacing kept). A wider window gives every gap the same
/// share of what it has to spare ([`Spread`]). 2 px (was 3), so the 1000 px window
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

/// How many gaps the bar has with nothing but its groups (the transport to the keyboard, the
/// panel buttons, then File, Edit and Help: eight groups).
const PLAIN_GAPS: f32 = 7.0;
/// The most each gap takes past [`GAP`]: with more to spare than that, the groups up to the
/// keyboard keep this gap and stand to the left, the panel buttons and File, Edit and Help keep
/// it and stand at the right end, and the gap between takes the rest: the notices' cell, the
/// notices left in it beside the keyboard (from about 1300 px).
/// Why: a request made at full screen (2259 px): "push everything to the right of the
/// play/pause/record UI to the left, other than Edit and File, so that it fills the empty
/// spaces"; every group sat in the middle of a cell 123 px wider than itself. Why 40: a notice
/// of its most's room ([`NOTICE_MOST`]) and its gap shared by the seven gaps in even pixels, the
/// gap the bar had where its controls stopped growing with the width (about 1722 px), whose look
/// was asked kept.
const PACKED_EXTRA: f32 = ((NOTICE_MOST + GAP) / PLAIN_GAPS / 2.0) as u32 as f32 * 2.0;
const _: () = assert!(PACKED_EXTRA == 40.0);

/// The transport's buttons: 38 px wide, [`BUTTON_HEIGHT`] tall, with 16 px glyphs, 2 px apart
/// (were 42 and 18 px: 0.9 of them, rounded, as every size of the bar). Why not squares: 4 × 10
/// px more left no room for the panel buttons at 1000 px.
const TRANSPORT_BUTTON: f32 = 38.0;
const TRANSPORT_ICON: f32 = 16.0;
const TRANSPORT_SPACING: f32 = 2.0;

/// The pattern's arrows: 13 px glyphs in 24 px wide buttons (were 14 in 22, then 20 until the
/// 24 px targets), joined to the name's box between them into one piece (12 px glyphs 2 px apart
/// from the box read as tiny buttons beside it).
const ARROW_ICON: f32 = 13.0;
const ARROW_WIDTH: f32 = 24.0;

/// The edge of a piece made of parts (the pattern's arrows and its name, Pat over Song, Notes
/// over Audio): one line of `button_edge` ([`crate::ui::palette::Palette::button_edge`]) round the piece and between its parts, where
/// each part drew its own edge before, which made 2 px seams between three boxes (the
/// pattern's piece looked the worst of the bar).
const FRAME: f32 = 1.0;
/// A side of a two-row switch in a group's height (Pat over Song, N over A): the height inside
/// the frame, halved.
const SWITCH_SIDE: f32 = (BUTTON_HEIGHT - 2.0 * FRAME) / 2.0;

/// The readouts: digits of the type scale's title size, 16 px (were 18), 4 px either side, each
/// as wide as its widest value ([`readout_width`]): the tempo's shape, 10 to 522 BPM with three
/// decimals (it shows one when the others are zero, [`format_tempo`], but a tempo typed to the
/// thousandth shows three), and the position's, bar:step:tick up to bar 99 (two step digits
/// hold the 96 steps of 24 quarters).
const READOUT_PADDING: f32 = 4.0;
const TEMPO_SHAPE: &str = "000.000";
const POSITION_SHAPE: &str = "00:00:00";
/// The time signature's readout beside the tempo: half its footprint, its digits at the type
/// scale's numbers size (11 px, not the readouts' 16), as wide as "24/16" in that size's widest
/// digits and the readouts' padding either side, 24 px tall (the least a control is pressed at),
/// its bottom on the tempo's bottom, so the two read as one row.
/// Why smaller: at the readouts' size it read as a second tempo, and took 58 px of the bar.
const SIGNATURE_SHAPE: &str = "00/00";
const SIGNATURE_HEIGHT: f32 = 24.0;

/// The input meter under the Record button (was 60 × 6).
const INPUT_METER: Size = Size::new(54.0, 5.0);

/// The column right of Record (Recording v2): Notes over Audio, one switch saying what a take
/// records, then Capture, three rows of one face, [`TAKE_FACE`], their words at the type scale's
/// label size. Why always on the bar: a tick left in Record's menu, out of sight, once turned
/// note takes into audio takes for an evening. Why this small: 54 px was what the 1000 px window
/// had left beside the panel buttons; narrower ([`Column::Letters`]) the switch says N and A in
/// two rows and Capture folds into the Notes menu (and Ctrl+Shift+R).
///
/// The rows, in the groups' 46 px (y 9 to 55 in the 64 px bar): the switch's frame, its line
/// round Notes and Audio and between them, 1 + 13 + 1 + 13 + 1 = 29 (Notes' face y 10 to 23,
/// the line 23 to 24, Audio's 24 to 37), [`PART_SPACING`] 2, then Capture, its own edge round
/// the same 13 px face, 1 + 13 + 1 = 15 (y 40 to 55, its face 41 to 54): 29 + 2 + 15 = 46. The
/// words, 12 px, stand on a line the face's 13 px tall. Why: the
/// toolkit's own line, 1.3 em = 15.6 px, was taller than the 13 px face, so the words' box was
/// cut to the face and the words drawn from its top, their foot 0.69 px over the row under it,
/// and the lit Notes, with no line between, read as drawn over Audio. On a line no taller than
/// the face, centred in it, the baseline lies 6.5 + (0.969 − 0.241) / 2 · 12 = 10.865 px down
/// (cosmic-text centres the font's ascent and descent in the line; Inter's typo metrics, in em):
/// Notes' and Audio's ink 1.85 to 11.01 px of the 13, clear of either edge by 1.8 px. Capture's
/// p reaches 0.204 em (2.45 px) under its baseline, so its words take the button's top 13 px (2 px
/// of padding under them): ink 2.02 to 13.31 of the 15 px button, inside its face (1 to 14) by
/// 1.02 above and 0.69 below (1.2 device px at the 4K screen's 1.7). Were rows of 14, 14 and 16
/// px with no line between Notes and Audio (faces 13, 13 and 14), words of 12 px (16, 16, 18 and
/// 12 before, then 11 until the type scale).
const TAKE_FACE: f32 = 13.0;
const CAPTURE_ROW: f32 = TAKE_FACE + 2.0 * FRAME;
const _: () = assert!(2.0 * TAKE_FACE + 3.0 * FRAME + PART_SPACING + CAPTURE_ROW == BUTTON_HEIGHT);
const TAKE_SIDE: f32 = 4.0;
const NOTES: &str = "Notes";
const AUDIO: &str = "Audio";
const CAPTURE: &str = "Capture";

/// The most the notices' line takes: about 43 characters at the labels' 13 px (36 at the 16 px
/// it was set for, 40 at 14). A longer line is cut with an ellipsis; the hint bar (and a tip) say it whole. Why a
/// bound: across a wide window the notices otherwise took the room the groups spread into. Why
/// not 0.9 of it with the rest: it is the room the bar keeps for what goes wrong, not a control,
/// and the gaps' most share comes from it ([`PACKED_EXTRA`]), which was asked kept.
pub const NOTICE_MOST: f32 = 288.0;
/// What the notices' line puts between two notices.
const NOTICE_JOIN: &str = " \u{b7} ";

/// The button that tries the output device again.
const RETRY: &str = "Retry";

/// The File and Edit menus' buttons' labels (File first, then Edit, then Help), the Help
/// window's button's right of them, and the space between two of them.
const FILE: &str = "File";
const EDIT: &str = "Edit";
const MENU_SPACING: f32 = 4.0;

/// The text buttons' (Pat, Song, Loop, the keyboard switch, File, Edit, Help) side padding: 5 px
/// (were 6) either side of their words, the type scale's body size (were 16, 15, then 14 until
/// the type scale), as the notices' and the octave's + and −.
const LABEL_SIDE: f32 = 5.0;

/// The keyboard switch's glyph (drawn at 18 × 12 and scaled up), and the space between it and
/// its words, on one line: "Keyboard on" or "Keyboard off" (they once wrapped onto
/// two lines, "Keyboard" over "on"). 19 × 13 and 5 px (were 21 × 14 and 6).
const KEYBOARD_GLYPH: Size = Size::new(19.0, 13.0);
const GLYPH_GAP: f32 = 5.0;
/// The keyboard glyph's drawing size, which [`KEYBOARD_GLYPH`] scales.
const KEYBOARD_DRAWN: f32 = 18.0;

/// The octave column: + over the octave's name over −, [`BUTTON_HEIGHT`] in all (was
/// 60 px, with a 24 px name): 24 px wide, + and − 10 px tall, 2 px from the name between them,
/// 22 px, which holds the name at the titles' 16 px (were rows of 16, 14 and 16 with the name
/// at 11; 26 wide, 18, 16, 18, then 23 wide until the 24 px targets). Why: the name is what the column says
/// and read too small beside the buttons; 10 px is the least a + or − is pressed
/// at (24 × 10: the column's width kept, so each is still 24 across). Two rows of 24 and the
/// name between are more than the groups' 46 px, so + and − take WCAG 2.2's spacing exception:
/// their centres 36 px apart and a 24 px circle round each touching no other control.
const OCTAVE_WIDTH: f32 = 24.0;
const OCTAVE_BUTTON: f32 = 10.0;
const OCTAVE_GAP: f32 = 2.0;
const OCTAVE_NAME: f32 = BUTTON_HEIGHT - 2.0 * OCTAVE_BUTTON - 2.0 * OCTAVE_GAP;
/// The octave's + and −, drawn in a square this big centred in their buttons ([`sign_bars`]),
/// as wide as the type's "+" at the body size (5.9 px) and a little more, and their bars'
/// thickness. Why drawn, not text: the font's + and − sit on its maths axis, 1.3 px under the
/// middle of their 16 px boxes, so the − read low in its button and the column off centre.
const OCTAVE_GLYPH: f32 = 7.0;
const SIGN_BAR: f32 = 1.25;

/// The pattern name's box at [`Tier::Short`]: a name longer than the box shrinks, then is cut
/// with an ellipsis: 47 px (was 52, and 56 before that), so the short labels still fit 880 px
/// with Retry. At [`Tier::Full`] the box holds [`PATTERN_NAME_WHOLE`] at the labels' size and
/// [`PATTERN_NAME_HELD`] at [`NAME_LEAST`] ([`pattern_name_width`]).
const PATTERN_NAME_SHORT: f32 = 47.0;
/// The name the full box holds at the labels' size: "Pattern 12" (the box was 84 px, which
/// held "Pattern 1" and cut the tenth pattern's name).
const PATTERN_NAME_WHOLE: &str = "Pattern 12";
/// A name of 24 characters, which the full box holds whole at [`NAME_LEAST`]: a longer name
/// than the labels' size holds shrinks a pixel at a time down to it, then is cut with an
/// ellipsis (the hint bar names it whole). Why 24: a name must always fit, and the box held 10
/// characters ("Pattern 12") and cut every name past them.
const PATTERN_NAME_HELD: &str = "Chorus bassline (take 2)";
/// The least the pattern's name shrinks to: the type scale's smallest step for words, 11 px
/// (was 10).
const NAME_LEAST: f32 = size::DENSE;
const PATTERN_NAME_PADDING: f32 = 5.0;
/// The space between the parts of a group that are not one piece: Pat over Song and Loop, the
/// keyboard switch and the octave's column.
const PART_SPACING: f32 = 2.0;

/// The panel buttons: a grid of three by two, each an icon of [`PANEL_ICON`] with
/// [`PANEL_PADDING`] around it, [`PANEL_SPACING`] apart. Why a grid of small icons rather than a
/// row of named buttons: two rows of three fit the toolbar's height in 64 px, and each has a tip
/// naming its panel and keys: 12 px icons in 20 × 28 px buttons (were 10 px in 20 × 22), the
/// grid as wide as before; then 20 × 25 px, the two rows
/// [`BUTTON_HEIGHT`] together (was 58 px); then 11 px icons in 19 × 22 px buttons, a 61 px grid,
/// at 0.9 of the size; now (for 24 px targets) 25 × 22, an 81 px grid: 25 wide so the 11 px icon
/// sits a whole pixel from either side (7 px), its edges sharp, as it sits a whole pixel from the
/// top (5 px over it, 6 under). Why 22 tall and not 24: two rows of 24 are more than the groups'
/// 46 px, so the rows take WCAG 2.2's spacing exception, their centres 24 px apart (27 across) and
/// a 24 px circle round each touching no other's.
const PANEL_ICON: f32 = 11.0;
/// The icons' drawing size, which [`PANEL_ICON`] scales.
const PANEL_ICON_DRAWN: f32 = 10.0;
const PANEL_PADDING: iced::Padding = iced::Padding {
    top: 5.0,
    right: 7.0,
    bottom: 6.0,
    left: 7.0,
};
const PANEL_SPACING: f32 = 2.0;
const PANEL_COLUMNS: usize = 3;
const PANEL_BUTTON_HEIGHT: f32 = PANEL_ICON + PANEL_PADDING.top + PANEL_PADDING.bottom;
const _: () = assert!(PANEL_BUTTON_HEIGHT == HALF_ROW);
/// A panel button's width: its icon and the padding either side; and the grid's, three of them
/// [`PANEL_SPACING`] apart.
const PANEL_BUTTON_WIDTH: f32 = PANEL_ICON + PANEL_PADDING.left + PANEL_PADDING.right;
const PANELS_WIDTH: f32 =
    PANEL_COLUMNS as f32 * PANEL_BUTTON_WIDTH + (PANEL_COLUMNS - 1) as f32 * PANEL_SPACING;

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
    /// The notices' line, then Retry, while either shows.
    Status,
    /// The panel buttons, while there is room for them.
    Panels,
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

/// Where the toolbar's groups lie for a [`Shape`]: the labels' tier, whether the panel buttons
/// show, the notices' line's width, and each group with its width, left to right. The gaps
/// between groups are [`GAP`] each and share what the window has to spare equally, in even
/// whole pixels, so the line in each gap's middle has the same space on both sides (the one
/// before the right end's groups, the panel buttons and the menus, takes what the rounding
/// leaves, so Help ends at the right padding: [`Spread::flex`]).
/// Even (whole pixels before, which left one side a pixel wider). With a share over
/// [`PACKED_EXTRA`] the bar is packed: every gap is [`PACKED_EXTRA`] past [`GAP`] but that one,
/// which takes the rest; its line stands beside the panel buttons (File with them folded) while
/// the notices or Retry show (their cell then runs from the keyboard's line to it), else beside
/// the keyboard, so every line keeps to its group.
#[derive(Debug, Clone, PartialEq)]
pub struct Spread {
    pub tier: Tier,
    /// How the column beside Record shows.
    pub column: Column,
    pub panels: bool,
    /// The notices' line's width: its own, or the room left when that is less; 0 without.
    pub notice_width: f32,
    groups: Vec<(Group, f32)>,
    /// Each gap's width past [`GAP`], and the flex gap's ([`Spread::flex`]).
    extra: f32,
    last_extra: f32,
    /// The groups stand to the left and to the right, the flex gap taking what the window has
    /// to spare.
    packed: bool,
}

impl Spread {
    pub fn of(shape: Shape) -> Self {
        let least = |(tier, column), panels, notice| {
            least_width(&groups(
                tier,
                column,
                panels,
                notice,
                shape.retry,
                shape.input_meter,
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
        let groups = groups(tier, column, panels, notice, shape.retry, shape.input_meter);
        let spare = (shape.width - least_width(&groups)).max(0.0);
        let gaps = groups.len().saturating_sub(1).max(1) as f32;
        let share = (spare / gaps / 2.0).floor() * 2.0;
        let packed = share > PACKED_EXTRA;
        let extra = if packed { PACKED_EXTRA } else { share };
        Self {
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

    /// The gap that takes what the shares leave (all the window spares past them, packed): the
    /// one before the right end's groups, the panel buttons while they show and File, Edit and
    /// Help, so those stand at the right padding together; counted as the group before it.
    pub(crate) fn flex(&self) -> usize {
        let right = self
            .groups
            .iter()
            .position(|(group, _)| matches!(group, Group::Panels | Group::Menus))
            .unwrap_or(self.groups.len().saturating_sub(1));
        right.saturating_sub(1)
    }

    /// The width of the gap after group number `index` past [`GAP`].
    fn extra_after(&self, index: usize) -> f32 {
        if index == self.flex() {
            self.last_extra
        } else {
            self.extra
        }
    }

    /// How far past group number `index` the line in the gap after it starts: in the gap's
    /// middle, on a whole pixel (the flex gap's as near as its rest allows). Packed, the flex
    /// gap's line stands its gap's half from the group after it (the panel buttons, or File)
    /// when the notices' cell is before it, else its half from the group before.
    fn line_after(&self, index: usize) -> f32 {
        let flex = index == self.flex();
        let half = BAR_SPACING + (PACKED_EXTRA / 2.0).floor();
        match self.groups.get(index) {
            Some((Group::Status, _)) if self.packed && flex => {
                GAP + self.last_extra - DIVIDER.width - half
            }
            _ if self.packed && flex => half,
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

    /// Where the line in each gap lies across the toolbar: a whole number of pixels past the
    /// group before it, as many as before the group after it (the last gap's as near as its rest
    /// allows). Every gap holds one: the group law reads, every group ends at a line. (The
    /// keyboard's held none while it stood alone across the empty stretch; with the notices'
    /// cell kept beside the panel buttons it is the keyboard's again.)
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
        self.left(Group::Pattern).unwrap_or(0.0) + ARROW_WIDTH
    }

    /// Where the computer keyboard's switch lies in the window. The first-run note points at it.
    pub fn keyboard_switch(&self) -> Rectangle {
        let x = self.left(Group::Keyboard).unwrap_or(0.0);
        Rectangle::new(
            Point::new(x, (TOOLBAR_HEIGHT - BUTTON_HEIGHT) / 2.0),
            Size::new(keyboard_switch_width(self.tier), BUTTON_HEIGHT),
        )
    }

    /// Where the Record button starts across the toolbar, where its menu opens: past the side
    /// padding, Play and Stop.
    pub fn record_x(&self) -> f32 {
        TOOLBAR_SIDE_PADDING + 2.0 * (TRANSPORT_BUTTON + TRANSPORT_SPACING)
    }

    /// Where the Record button lies in the window. The first-run note on recording points at it.
    pub fn record_button(&self) -> Rectangle {
        Rectangle::new(
            Point::new(self.record_x(), (TOOLBAR_HEIGHT - BUTTON_HEIGHT) / 2.0),
            Size::new(TRANSPORT_BUTTON, BUTTON_HEIGHT),
        )
    }

    /// Where the column right of Record starts across the toolbar, where its menus open.
    pub fn take_column_x(&self) -> f32 {
        self.record_x() + TRANSPORT_BUTTON + TRANSPORT_SPACING
    }

    /// How far the Edit menu's right edge lies from the window's: under the Edit button's right
    /// edge, Help's width and the menus' spacing left of the toolbar's right padding.
    pub fn edit_menu_right(&self) -> f32 {
        TOOLBAR_SIDE_PADDING + help_width(self.tier) + MENU_SPACING
    }

    /// How far the File menu's right edge lies from the window's: under the File button's right
    /// edge, Edit's and Help's widths and two of the menus' spacings left of the toolbar's right
    /// padding.
    pub fn file_menu_right(&self) -> f32 {
        self.edit_menu_right() + menu_button_width() + MENU_SPACING
    }
}

/// The width a readout of `shape` takes: the shape with every digit the font's widest, at
/// `text_size` (the readouts' 16 px, the time signature's 11), and their padding either side, in whole pixels, so every value of the shape
/// fits in it. Why the widest digit: the app's font has proportional digits (its "1" about two
/// thirds of its "4"), iced 0.14 cannot ask a font for its tabular ones, and a changed copy of
/// the font was ruled out (one more font to license, ship and lock, for looks alone), so a
/// readout reserves its widest value and its digits stand centred in it ([`readout`]), a "1" no
/// longer as wide as a "4". ASCII text is shaped without kerning, so no value of the shape is
/// wider than that.
fn readout_width(shape: &str, text_size: f32) -> f32 {
    let widest = ('0'..='9').fold(('0', 0.0), |(widest, most), digit| {
        let width = text_width(digit.encode_utf8(&mut [0; 4]), text_size);
        if width > most {
            (digit, width)
        } else {
            (widest, most)
        }
    });
    let form: String = shape
        .chars()
        .map(|c| if c.is_ascii_digit() { widest.0 } else { c })
        .collect();
    (text_width(&form, text_size) + 2.0 * READOUT_PADDING).ceil()
}

/// The tempo's, the time signature's and the position's readouts' widths, measured once: the
/// font is the app's own and never changes while it runs, and the bar is laid out several times
/// a frame.
fn readout_widths() -> [f32; 3] {
    static WIDTHS: OnceLock<[f32; 3]> = OnceLock::new();
    *WIDTHS.get_or_init(|| {
        [
            readout_width(TEMPO_SHAPE, size::TITLE),
            readout_width(SIGNATURE_SHAPE, size::DENSE),
            readout_width(POSITION_SHAPE, size::TITLE),
        ]
    })
}

fn tempo_width() -> f32 {
    readout_widths()[0]
}

fn signature_width() -> f32 {
    readout_widths()[1]
}

fn position_width() -> f32 {
    readout_widths()[2]
}

/// The tempo's group: its readout, then the time signature's beside it, [`PART_SPACING`] apart.
fn tempo_group_width() -> f32 {
    tempo_width() + PART_SPACING + signature_width()
}

/// The width of the transport: Play, Stop, Record (the input meter's width while it shows under
/// it), the column beside it and the metronome.
fn transport_width(input_meter: bool, column: Column) -> f32 {
    let record = if input_meter {
        INPUT_METER.width
    } else {
        TRANSPORT_BUTTON
    };
    let beside = match column {
        Column::Badge => 0.0,
        Column::Words | Column::Letters => take_column_width(column) + TRANSPORT_SPACING,
    };
    3.0 * TRANSPORT_BUTTON + record + beside + 3.0 * TRANSPORT_SPACING
}

/// The column right of Record: as wide as its widest word and the room either side, in whole
/// pixels; as [`Column::Letters`] its letters'; none as [`Column::Badge`].
fn take_column_width(column: Column) -> f32 {
    let words: &[&str] = match column {
        Column::Words => &[NOTES, AUDIO, CAPTURE],
        Column::Letters => &["N", "A"],
        Column::Badge => return 0.0,
    };
    let widest = words
        .iter()
        .map(|word| text_width(word, size::LABEL))
        .fold(0.0, f32::max);
    (widest + 2.0 * TAKE_SIDE).ceil()
}

fn pattern_width(tier: Tier) -> f32 {
    2.0 * ARROW_WIDTH + pattern_name_width(tier)
}

/// The pattern name's box, its frame's line either side: at [`Tier::Full`] the wider of
/// [`PATTERN_NAME_WHOLE`] at the labels' size and [`PATTERN_NAME_HELD`] at the least, measured,
/// and its padding, in whole pixels; at [`Tier::Short`] [`PATTERN_NAME_SHORT`].
fn pattern_name_width(tier: Tier) -> f32 {
    match tier {
        Tier::Full => {
            let whole = text_width(PATTERN_NAME_WHOLE, size::BODY);
            let held = text_width(PATTERN_NAME_HELD, NAME_LEAST);
            (whole.max(held) + 2.0 * PATTERN_NAME_PADDING + 2.0 * FRAME).ceil()
        }
        Tier::Short => PATTERN_NAME_SHORT,
    }
}

/// The room the pattern's name has in its box: the box less its frame's lines and padding.
fn pattern_name_room(tier: Tier) -> f32 {
    pattern_name_width(tier) - 2.0 * FRAME - 2.0 * PATTERN_NAME_PADDING
}

/// The play mode's Pat over Song: as wide as the wider label.
fn mode_pair_width() -> f32 {
    let width = |label| text_width(label, size::BODY) + 2.0 * LABEL_SIDE;
    width("Song").max(width("Pat"))
}

fn mode_width() -> f32 {
    mode_pair_width() + PART_SPACING + label_button_width("Loop")
}

/// The keyboard switch's width: its glyph and its words on or off, the wider, so on and off are
/// one width.
fn keyboard_switch_width(tier: Tier) -> f32 {
    let words = [false, true]
        .into_iter()
        .map(|on| text_width(keyboard_words(on, tier), size::BODY))
        .fold(0.0, f32::max);
    KEYBOARD_GLYPH.width + GLYPH_GAP + words + 2.0 * LABEL_SIDE
}

fn keyboard_width(tier: Tier) -> f32 {
    keyboard_switch_width(tier) + PART_SPACING + OCTAVE_WIDTH
}

fn retry_width() -> f32 {
    text_width(RETRY, size::LABEL) + 2.0 * LABEL_SIDE
}

fn menus_width(tier: Tier) -> f32 {
    menu_button_width() + MENU_SPACING + menu_button_width() + MENU_SPACING + help_width(tier)
}

/// A text button's width: its label at the labels' size with their side padding on both sides.
fn label_button_width(label: &str) -> f32 {
    text_width(label, size::BODY) + 2.0 * LABEL_SIDE
}

/// Help this much wider than a text button of its label ([`label_button_width`]), in two
/// steps each rounded up to a whole pixel ([`widened_width`]): 5 %, then 10 % more. Why: the
/// menus were made a little wider, then another 10 % wider than they stood (Help 41 px to 46),
/// so the second step widens what the first gave, not the text button.
const MENU_WIDEN: [f32; 2] = [1.05, 1.10];

/// A text button's width for `label` times each of [`MENU_WIDEN`] in turn, rounded up to a
/// whole pixel after each (a 40 px button is 42, then 47).
fn widened_width(label: &str) -> f32 {
    MENU_WIDEN
        .iter()
        .fold(label_button_width(label), |width, widen| {
            (width * widen).ceil()
        })
}

/// File's, Edit's and Help's one width: Help's label widened ([`widened_width`]), 46 px. Why:
/// the three as wide as Help read as one row of equal buttons; Help, the widest label, still
/// holds its text.
fn menu_button_width() -> f32 {
    widened_width(help_label(Tier::Full))
}

/// Help's width at `tier`: the menus' one width, or at the short labels "?" widened.
fn help_width(tier: Tier) -> f32 {
    match tier {
        Tier::Full => menu_button_width(),
        Tier::Short => widened_width(help_label(tier)),
    }
}

/// The vertical padding that centers a text box of `size` text in a group's height.
fn text_box_padding(size: f32) -> f32 {
    ((BUTTON_HEIGHT - 1.3 * size) / 2.0).floor()
}

/// The groups that show, left to right, with their widths: the labels at `tier`, the panel
/// buttons when `panels`, the notices' line `notice` wide while there are notices, Retry.
fn groups(
    tier: Tier,
    column: Column,
    panels: bool,
    notice: Option<f32>,
    retry: bool,
    input_meter: bool,
) -> Vec<(Group, f32)> {
    let mut groups = vec![
        (Group::Transport, transport_width(input_meter, column)),
        (Group::Tempo, tempo_group_width()),
        (Group::Pattern, pattern_width(tier)),
        (Group::Mode, mode_width()),
        (Group::Position, position_width()),
        (Group::Keyboard, keyboard_width(tier)),
    ];
    let status = match (notice, retry) {
        (Some(notice), true) => Some(notice + BAR_SPACING + retry_width()),
        (Some(notice), false) => Some(notice),
        (None, true) => Some(retry_width()),
        // Nothing to say: the cell stays, empty, while the panel buttons show, so they stand
        // between two lines (the keyboard's and their own) as they do beside a notice. Its gap
        // and line cost GAP, which moves the folds that keep them.
        (None, false) => panels.then_some(0.0),
    };
    if let Some(status) = status {
        groups.push((Group::Status, status));
    }
    if panels {
        groups.push((Group::Panels, PANELS_WIDTH));
    }
    groups.push((Group::Menus, menus_width(tier)));
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
    /// The time signature shown right of the tempo: the song's at the song position (the
    /// project's own, from tick 0, but while the song plays past a change).
    pub time_signature: TimeSignature,
    /// The meter the position counts bars of: the song's map, or the pattern's own signature
    /// while a pattern plays alone.
    pub meter: MeterMap,
    pub pattern_name: &'a str,
    pub previous_pattern: Option<PatternId>,
    /// The text of the pattern's name while it is being renamed.
    pub pattern_rename: Option<&'a str>,
    /// The play mode the switch shows lit; none while the piano roll's player plays.
    pub mode: Option<PlayMode>,
    /// The loop range's state: on, off, or `None` when there is no range.
    pub loop_on: Option<bool>,
    pub position: Ticks,
    /// Why the last action failed.
    pub error: Option<&'a str>,
    /// What is wrong with live audio, such as it being off. Shown last, beside any error.
    pub audio_problem: Option<&'a str>,
    /// Audio is off and can be tried again: a Retry button stands before File.
    pub retry_audio: bool,
    /// What the last file or sound action has to tell, such as a missing sample.
    pub notice: Option<&'a str>,
    /// The zoom just set, for two seconds: the notices' line starts "Zoom 125 %".
    pub zoomed: Option<Zoom>,
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
    /// The text typed in the time signature's box while it is open (a click on it opens it).
    pub signature_edit: Option<&'a str>,
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
    /// The notices as one line, a dot between two: the zoom just set, what the last file or
    /// sound action has to tell, why the last action failed, what is wrong with live audio, the
    /// underruns; none while there is nothing to tell. How audio runs (resampling) is not one:
    /// Settings' audio output says it, and the hint bar over the position.
    pub fn notice_text(&self) -> Option<String> {
        let underruns = (self.underruns > 0).then(|| format!("Underruns: {}", self.underruns));
        let zoomed = self.zoomed.map(|zoom| format!("Zoom {zoom}"));
        let parts: Vec<&str> = [
            zoomed.as_deref(),
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
        let transport = row![
            tool(
                transport_button(Glyph::Play, self.playing, Some(Message::Play)),
                Tool::Play
            ),
            tool(
                transport_button(Glyph::Stop, false, Some(Message::Stop)),
                Tool::Stop
            ),
            tool(
                with_tip(
                    record_button(self.record, spread.column == Column::Badge),
                    record_tip(self.keys, self.record.kind)
                ),
                Tool::Record
            ),
        ]
        .spacing(TRANSPORT_SPACING)
        .align_y(Center);
        let transport = match take_column(self.record.kind, spread.column, self.keys) {
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
                    ),
                    Tool::PreviousPattern
                ),
                tool(
                    pattern_name(self.pattern_name, self.pattern_rename, tier),
                    Tool::PatternName
                ),
                // Past the last pattern it makes a new one.
                tool(
                    arrow_button(
                        Glyph::Next,
                        Some(Message::Pattern(PatternMessage::Next)),
                        iced::border::right(1.0),
                    ),
                    Tool::NextPattern
                ),
            ]
            .spacing(FRAME)
            .align_y(Center),
        );
        let looping = label_button(
            "Loop",
            label_button_width("Loop"),
            self.loop_on == Some(true),
            self.loop_on.map(|_| Message::ToggleLoop),
        );
        let looping = with_tip(looping, loop_tip(self.loop_on));
        let side = mode_pair_width() - 2.0 * FRAME;
        // Pat over Song: one switch of two rows in one frame, the mode playing lit.
        let mode = row![
            framed(column![
                tool(
                    mode_button(
                        "Pat",
                        PlayMode::Pattern,
                        self.mode,
                        [side, size::BODY],
                        iced::border::top(1.0)
                    ),
                    Tool::PatternMode
                ),
                tool(
                    mode_button(
                        "Song",
                        PlayMode::Song,
                        self.mode,
                        [side, size::BODY],
                        iced::border::bottom(1.0)
                    ),
                    Tool::SongMode
                ),
            ]),
            tool(looping, Tool::Loop),
        ]
        .spacing(PART_SPACING)
        .align_y(Center);
        // Drag left or right, or scroll, to change the tempo; Shift for 0.001 steps; a click
        // opens a box to type it in.
        let tempo: Element<'a, Message> = match self.tempo_edit {
            // In a box the readout's height, so the group keeps it while the tempo is typed.
            Some(typed) => container(
                text_input("120", typed)
                    .id(Id::new(crate::app::TEMPO_INPUT))
                    .on_input(Message::TempoEdited)
                    .on_submit(Message::TempoSubmitted)
                    .size(size::TITLE)
                    .padding([text_box_padding(size::TITLE), READOUT_PADDING])
                    .width(tempo_width()),
            )
            .height(BUTTON_HEIGHT)
            .center_y(BUTTON_HEIGHT)
            .into(),
            None => with_tip(
                mouse_area(readout(format_tempo(self.tempo), tempo_width()))
                    .on_press(Message::TempoPressed)
                    .on_scroll(Message::TempoScrolled)
                    .interaction(mouse::Interaction::ResizingHorizontally),
                "Tempo \u{b7} drag or scroll; click to type it",
            ),
        };
        let tempo = tool(tempo, Tool::Tempo);
        // Drag up or down, or scroll, to change the beats a bar; Shift for the beat's note; a
        // click opens a box to type it in.
        let signature: Element<'a, Message> = match self.signature_edit {
            Some(typed) => container(
                text_input("4/4", typed)
                    .id(Id::new(crate::app::SIGNATURE_INPUT))
                    .on_input(Message::SignatureEdited)
                    .on_submit(Message::SignatureSubmitted)
                    .size(size::DENSE)
                    .padding([
                        ((SIGNATURE_HEIGHT - 1.3 * size::DENSE) / 2.0).floor(),
                        READOUT_PADDING,
                    ])
                    .width(signature_width()),
            )
            .height(SIGNATURE_HEIGHT)
            .center_y(SIGNATURE_HEIGHT)
            .into(),
            None => with_tip(
                mouse_area(readout_box(
                    self.time_signature.to_string(),
                    signature_width(),
                    size::DENSE,
                    SIGNATURE_HEIGHT,
                ))
                .on_press(Message::SignaturePressed)
                .on_scroll(Message::SignatureScrolled)
                .interaction(mouse::Interaction::ResizingVertically),
                "Time signature \u{b7} drag or scroll; Shift for the note; click to type it",
            ),
        };
        // The signature's bottom on the tempo's: the tempo box's top + its height − the
        // readout's height, 9 + 46 − 24 = y 31 in the 64 px bar (was 9 + (46 − 24) / 2 = 20,
        // centred), so it ends at y 55 with the tempo. The row is the tempo's height, so the
        // row's bottom is the tempo's.
        let tempo: Element<'a, Message> = row![tempo, tool(signature, Tool::TimeSignature)]
            .spacing(PART_SPACING)
            .align_y(Bottom)
            .into();
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
                container(text(RETRY).size(size::LABEL))
                    .center_y(Fill)
                    .center_x(Fill),
            )
            .width(retry_width())
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
                    menu_button_width(),
                    self.menu_open,
                    Some(Message::File(FileMessage::ToggleMenu)),
                ),
                Tool::File
            ),
            tool(
                label_button(
                    EDIT,
                    menu_button_width(),
                    self.edit_menu_open,
                    Some(Message::ToggleEditMenu),
                ),
                Tool::Edit
            ),
            tool(
                label_button(
                    help_label(tier),
                    help_width(tier),
                    self.help_open,
                    Some(Message::ToggleHelp),
                ),
                Tool::Help
            ),
        ]
        .spacing(MENU_SPACING);
        let typing = typing_keyboard(self.typing, tier);
        let position = tool(
            readout(
                format_position(self.position, &self.meter),
                position_width(),
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
            // The empty cell the spread keeps beside the panel buttons ([`groups`]).
            (None, None) => spread
                .panels
                .then(|| iced::widget::space().width(0.0).into()),
        };
        let mut parts = [
            Some(transport.into()),
            Some(tempo),
            Some(pattern),
            Some(mode.into()),
            Some(position),
            Some(typing),
            status,
            spread.panels.then(|| panel_buttons(self.panels, self.keys)),
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
            // The line in the gap's middle, on a whole pixel past its group (packed, the flex
            // gap's beside the group after it or before it); the flex gap's right side takes what
            // is left, so Help ends at the right padding.
            let extra = spread.extra_after(index);
            let flex = index == spread.flex();
            let left = spread.line_after(index);
            content = content
                .push(iced::widget::space().width(left))
                .push(divider());
            content = if flex {
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
fn take_column<'a>(kind: TakeKind, column: Column, keys: &Keys) -> Option<Element<'a, Message>> {
    let width = take_column_width(column);
    // Words: the frame's line between Notes and Audio (rows of 13 px, [`TAKE_FACE`]); letters:
    // none, two sides of 22 px filling the frame as Pat over Song do.
    let (notes, audio, row, line) = match column {
        Column::Words => (NOTES, AUDIO, TAKE_FACE, FRAME),
        Column::Letters => ("N", "A", SWITCH_SIDE, 0.0),
        Column::Badge => return None,
    };
    let side = |label: &'a str, side: TakeKind, radius, tip: &'static str, which| {
        // On a line the side's height, never taller than its face ([`TAKE_FACE`]'s why).
        let words = text(label)
            .size(size::LABEL)
            .line_height(iced::Pixels(row))
            .wrapping(Wrapping::None);
        let face = button(container(words).center_x(Fill).center_y(Fill))
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
    ]
    .spacing(line));
    if column == Column::Letters {
        return Some(switch);
    }
    // In the button's top 13 px, 2 px of padding under the words: its p clear of its edge
    // ([`TAKE_FACE`]'s why).
    let words = text(CAPTURE)
        .size(size::LABEL)
        .line_height(iced::Pixels(TAKE_FACE))
        .wrapping(Wrapping::None);
    let capture = button(
        container(words)
            .padding(iced::Padding::ZERO.bottom(2.0 * FRAME))
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
fn pattern_name<'a>(name: &'a str, rename: Option<&'a str>, tier: Tier) -> Element<'a, Message> {
    let width = pattern_name_width(tier) - 2.0 * FRAME;
    let height = BUTTON_HEIGHT - 2.0 * FRAME;
    match rename {
        Some(typed) => container(
            text_input("", typed)
                .id(Id::new(PATTERN_RENAME_INPUT))
                .on_input(|typed| Message::Pattern(PatternMessage::RenameEdited(typed)))
                .on_submit(Message::Pattern(PatternMessage::RenameSubmitted))
                .size(size::BODY)
                .padding([text_box_padding(size::BODY) - FRAME, PATTERN_NAME_PADDING])
                .width(width),
        )
        .height(height)
        .center_y(height)
        .into(),
        None => {
            // One line, shrunk to fit, then cut with an ellipsis: a long name once wrapped out of
            // the toolbar, then was cut past ten characters.
            let (shown, size) = shown_pattern_name(name, tier);
            mouse_area(
                container(
                    text(shown)
                        .size(size)
                        .color(style::palette().lcd_text)
                        .wrapping(Wrapping::None),
                )
                .width(width)
                .height(height)
                .center_x(width)
                .center_y(height)
                .padding([0.0, PATTERN_NAME_PADDING])
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
fn shown_pattern_name(name: &str, tier: Tier) -> (String, f32) {
    let room = pattern_name_room(tier);
    let (most, least) = (size::BODY, NAME_LEAST);
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
fn panel_buttons<'a>(panels: Panels, keys: &Keys) -> Element<'a, Message> {
    let button_of = |panel: Panel| -> Element<'a, Message> {
        let open = panels.is_open(panel);
        let color = if open {
            style::accent()
        } else {
            style::palette().text
        };
        let face = button(
            canvas(PanelIcon { panel, color })
                .width(PANEL_ICON)
                .height(PANEL_ICON),
        )
        .width(PANEL_BUTTON_WIDTH)
        .height(PANEL_BUTTON_HEIGHT)
        .padding(PANEL_PADDING)
        .style(style::tool_button(open))
        .on_press(Message::TogglePanel(panel));
        let tip = container(text(panel_tip(panel, keys)).size(size::LABEL))
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
            .size(size::BODY)
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

/// A text button a group's height and `width` wide, its label centred: lit when `lit`, dim with
/// nothing to do.
fn label_button<'a>(
    label: &'a str,
    width: f32,
    lit: bool,
    on_press: Option<Message>,
) -> Element<'a, Message> {
    button(
        container(text(label).size(size::BODY).wrapping(Wrapping::None))
            .center_x(Fill)
            .center_y(Fill),
    )
    .width(width)
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

/// A panel button's icon and its width: 11 and 19 px at any window width. As the probe of the
/// icons wants them.
#[cfg(test)]
pub(crate) fn panel_icon_size() -> (f32, f32) {
    (PANEL_ICON, PANEL_BUTTON_WIDTH)
}

/// Where Retry, Edit, File and Help lie across the toolbar `spread` lays out: their left and
/// right edges, as the pixel probe looks for them. Help stands against the right padding, Edit
/// the menus' spacing before it and File the spacing before Edit, at the group's start; Retry
/// ends its group, the notices' cell before the panel buttons (File, while they are folded).
#[cfg(test)]
pub(crate) fn retry_edit_file_and_help(spread: &Spread) -> [std::ops::Range<f32>; 4] {
    let menus = spread
        .span(Group::Menus)
        .expect("File, Edit and Help always show");
    let help_start = menus.end - help_width(spread.tier);
    let edit_end = help_start - MENU_SPACING;
    let edit_start = edit_end - menu_button_width();
    let file_end = edit_start - MENU_SPACING;
    let retry_end = spread
        .span(Group::Status)
        .map_or(menus.start, |status| status.end);
    [
        retry_end - retry_width()..retry_end,
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
    let width = PANEL_BUTTON_WIDTH;
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
    text_width(content, size::BODY)
}

/// Tempo in BPM with one decimal, and the second and third only when they are not zero:
/// 120000 milli-BPM reads "120.0", 120500 "120.5", 120125 "120.125". Why not always three
/// (it read "120.000"): the usual tempo is whole or a half, and two zeros after it are noise;
/// a tempo set to the thousandth still reads whole.
pub fn format_tempo(tempo: MilliBpm) -> String {
    let thousandths = format!("{:03}", tempo.0 % 1000);
    let decimals = thousandths.trim_end_matches('0');
    let decimals = if decimals.is_empty() { "0" } else { decimals };
    format!("{}.{decimals}", tempo.0 / 1000)
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

/// `bar:step:tick`, counted from 1:01:00 at the start, in the bars of `meter` (numbered on
/// across its changes; 12 steps a bar in 3/4 and 6/8, 16 in 4/4).
pub fn format_position(position: Ticks, meter: &MeterMap) -> String {
    let position = Ticks(position.0.max(0));
    let bar = meter.bar_at(position);
    let step_ticks = time::step_to_ticks(1, meter.ppq()).0.max(1);
    let into = position.0 - bar.start.0;
    format!(
        "{}:{:02}:{:02}",
        bar.bar + 1,
        into / step_ticks + 1,
        into % step_ticks
    )
}

/// A length as `bars:steps:ticks` in bars of `signature`, counted from 0:00:00: a step is
/// 0:01:00.
pub fn format_length(length: Ticks, signature: TimeSignature, ppq: u16) -> String {
    let (bars, steps, ticks) = bars_steps_ticks(length, signature, ppq);
    format!("{bars}:{steps:02}:{ticks:02}")
}

/// Whole bars of `signature`, then whole steps, then ticks in a span from tick 0 (negative
/// counts as 0).
fn bars_steps_ticks(span: Ticks, signature: TimeSignature, ppq: u16) -> (i64, i64, i64) {
    let bar_ticks = signature.bar_ticks(ppq).0.max(1);
    let step_ticks = time::step_to_ticks(1, ppq).0.max(1);
    let ticks = span.0.max(0);
    (
        ticks / bar_ticks,
        ticks % bar_ticks / step_ticks,
        ticks % step_ticks,
    )
}

/// A recessed display, `width` wide ([`readout_width`]), its digits centred in it. Why centred
/// (they stood right-aligned): the box is as wide as its widest value, so a usual one, "1:01:00"
/// or "120.0", sat with most of the room on its left and read off-centre. The box
/// keeps its width, so it never jitters; only the digits move within it as they change.
fn readout<'a>(content: String, width: f32) -> Element<'a, Message> {
    readout_box(content, width, size::TITLE, BUTTON_HEIGHT)
}

/// A readout of `text_size` digits, `width` by `height`, its digits centred: the glass the
/// readouts share ([`readout`]; the time signature's smaller one, [`SIGNATURE_HEIGHT`]).
fn readout_box<'a>(
    content: String,
    width: f32,
    text_size: f32,
    height: f32,
) -> Element<'a, Message> {
    container(
        text(content)
            .size(text_size)
            .color(style::palette().lcd_text),
    )
    .width(width)
    .height(height)
    .padding([0.0, READOUT_PADDING])
    .align_x(Horizontal::Center)
    .center_y(height)
    .style(style::lcd)
    .into()
}

/// The computer keyboard: its switch (a keyboard glyph and "Keyboard on" on one line, lit while
/// it plays notes; a press switches it), then + over the octave's name over −, the switch's
/// height.
fn typing_keyboard<'a>(look: TypingLook, tier: Tier) -> Element<'a, Message> {
    let glyph_color = if look.on {
        style::accent()
    } else {
        style::palette().text
    };
    let words = text(keyboard_words(look.on, tier))
        .size(size::BODY)
        .wrapping(Wrapping::None);
    let face = row![
        canvas(Icon {
            glyph: Glyph::Keyboard,
            color: glyph_color,
        })
        .width(KEYBOARD_GLYPH.width)
        .height(KEYBOARD_GLYPH.height),
        words,
    ]
    .spacing(GLYPH_GAP)
    .align_y(Center);
    let switch = button(container(face).center_x(Fill).center_y(Fill))
        .width(keyboard_switch_width(tier))
        .height(BUTTON_HEIGHT)
        .padding(0)
        .style(style::tool_button(look.on))
        .on_press(Message::ToggleTyping);
    let switch = with_tip(switch, look.tip);
    let octave = |glyph, octaves: i8, enabled: bool| {
        let on_press = enabled.then_some(Message::TypingOctave(octaves));
        glyph_face(glyph, false, on_press, OCTAVE_GLYPH)
            .width(OCTAVE_WIDTH)
            .height(OCTAVE_BUTTON)
            .style(style::tool_button(false))
    };
    let name = container(
        text(look.octave)
            .size(size::OCTAVE_NAME)
            .color(style::palette().text_secondary)
            .wrapping(Wrapping::None),
    )
    .width(OCTAVE_WIDTH)
    .height(OCTAVE_NAME)
    .center_x(OCTAVE_WIDTH)
    .center_y(OCTAVE_NAME);
    let column = column![
        tool(octave(Glyph::Plus, 1, look.higher), Tool::OctaveUp),
        name,
        tool(octave(Glyph::Minus, -1, look.lower), Tool::OctaveDown),
    ]
    .spacing(OCTAVE_GAP);
    row![tool(switch, Tool::Keys), column]
        .spacing(PART_SPACING)
        .align_y(Center)
        .into()
}

/// Pat or Song, a side of the play mode's switch `width` wide with `size` text, inside its
/// frame, lit when it is `current`: `radius` rounds the switch's outer corners inside the
/// frame's.
fn mode_button<'a>(
    label: &'a str,
    mode: PlayMode,
    current: Option<PlayMode>,
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
    .style(style::switch_side(current == Some(mode), radius))
    .on_press(Message::ModeSelected(mode))
    .into()
}

/// The round Record button (a press arms, a right press opens its menu), with the input meter
/// under it while the Input source is armed; with `badge` (no room for the switch beside it) it
/// says under its dot what a take records, N or A.
fn record_button<'a>(look: RecordLook, badge: bool) -> Element<'a, Message> {
    let color = if look.armed && !look.blink_off {
        style::RECORD_ARMED
    } else {
        style::RECORD_OFF
    };
    let dot = canvas(Icon {
        glyph: Glyph::Record,
        color,
    })
    .width(TRANSPORT_ICON)
    .height(TRANSPORT_ICON);
    let content: Element<'a, Message> = if badge {
        let letter = match look.kind {
            TakeKind::Notes => "N",
            TakeKind::Audio => "A",
        };
        column![
            dot,
            text(letter)
                .size(size::DENSE)
                .color(style::palette().text_secondary)
        ]
        .spacing(1)
        .align_x(Center)
        .into()
    } else {
        dot.into()
    };
    let face = button(container(content).center_x(Fill).center_y(Fill))
        .width(TRANSPORT_BUTTON)
        .height(BUTTON_HEIGHT)
        .padding(0)
        .style(style::tool_button(look.armed))
        .on_press(Message::Record(RecordMessage::ToggleArm));
    let face = mouse_area(face).on_right_press(Message::Record(RecordMessage::ToggleMenu));
    match look.input_level {
        Some(level) => column![
            face,
            canvas(InputMeter { level })
                .width(INPUT_METER.width)
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
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), style::palette().meter_ground);
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

/// A transport button: 38 × 46 px, showing `glyph`, accent when `lit`, dimmed when it has
/// nothing to do.
fn transport_button<'a>(
    glyph: Glyph,
    lit: bool,
    on_press: Option<Message>,
) -> Element<'a, Message> {
    glyph_face(glyph, lit, on_press, TRANSPORT_ICON)
        .width(TRANSPORT_BUTTON)
        .height(BUTTON_HEIGHT)
        .style(style::tool_button(lit))
        .into()
}

/// The piano roll strip's Play / Stop, `size` big with an `icon` px glyph: the transport's
/// Play, or its Stop lit while the roll plays; dim with nothing to do (a take runs).
pub fn play_stop_button<'a>(
    playing: bool,
    on_press: Option<Message>,
    size: Size,
    icon: f32,
) -> Element<'a, Message> {
    let glyph = if playing { Glyph::Stop } else { Glyph::Play };
    glyph_face(glyph, playing, on_press, icon)
        .width(size.width)
        .height(size.height)
        .style(style::tool_button(playing))
        .into()
}

/// The piano roll strip's Preview or Follow folded to a glyph ([`Glyph::Speaker`],
/// [`Glyph::Follow`]), `size` big with an `icon` px glyph: the accent while `on`, as the
/// switch's words are, and the switch's own button look.
pub fn glyph_switch<'a>(
    glyph: Glyph,
    on: bool,
    on_press: Message,
    size: Size,
    icon: f32,
) -> Element<'a, Message> {
    glyph_face(glyph, on, Some(on_press), icon)
        .width(size.width)
        .height(size.height)
        .style(style::tool_button(on))
        .into()
}

/// A menu button's arrow ([`Glyph::MenuArrow`]) in a `side` px square, in the colour a
/// [`style::tool_button`]'s words take: the accent while `lit` (its menu open), else the
/// text's. The piano roll strip's Notes draws it after its word.
pub fn menu_arrow<'a>(lit: bool, side: f32) -> Element<'a, Message> {
    let color = if lit {
        style::accent()
    } else {
        style::palette().text
    };
    canvas(Icon {
        glyph: Glyph::MenuArrow,
        color,
    })
    .width(side)
    .height(side)
    .into()
}

/// One of the pattern's arrows: a 13 px glyph in a 20 px wide part of the pattern's piece,
/// inside its frame, `radius` rounding the piece's outer corners inside the frame's.
fn arrow_button<'a>(
    glyph: Glyph,
    on_press: Option<Message>,
    radius: iced::border::Radius,
) -> Element<'a, Message> {
    glyph_face(glyph, false, on_press, ARROW_ICON)
        .width(ARROW_WIDTH - FRAME)
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
        (true, _) => style::accent(),
        (false, true) => style::palette().text,
        (false, false) => style::palette().text_disabled,
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
pub enum Glyph {
    Play,
    Stop,
    Record,
    /// A metronome: a tall triangle, its arm swung right.
    Metronome,
    Previous,
    Next,
    /// A computer keyboard: an outline with two rows of keys and a space bar.
    Keyboard,
    /// The octave's + and −: bars across the middle of the glyph's square ([`sign_bars`]).
    Plus,
    Minus,
    /// The piano roll's Preview folded: a speaker, its box and cone filled, one sound wave.
    Speaker,
    /// The piano roll's Follow folded: an arrow pointing on along time.
    Follow,
    /// A menu button's arrow: a small triangle pointing down, 1 px in from either side of its
    /// square and 3.5 px tall about its middle (6 × 3.5 in the strip's 8 px square).
    MenuArrow,
}

/// The bars of the octave's `glyph` (+ or −) in a square `side` wide: one across its middle,
/// and for + one down it, each [`SIGN_BAR`] thick and centred on the square's middle, so the
/// sign stands in the middle of its button whatever the font's axis.
fn sign_bars(glyph: Glyph, side: f32) -> Vec<Rectangle> {
    let middle = (side - SIGN_BAR) / 2.0;
    let across = Rectangle::new(Point::new(0.0, middle), Size::new(side, SIGN_BAR));
    let down = Rectangle::new(Point::new(middle, 0.0), Size::new(SIGN_BAR, side));
    match glyph {
        Glyph::Plus => vec![across, down],
        Glyph::Minus => vec![across],
        _ => Vec::new(),
    }
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
            Glyph::Plus | Glyph::Minus => {
                for bar in sign_bars(self.glyph, s) {
                    frame.fill_rectangle(bar.position(), bar.size(), self.color);
                }
                return vec![frame.into_geometry()];
            }
            Glyph::Speaker => {
                let cone = canvas::Path::new(|path| {
                    path.move_to(Point::new(0.5, 0.35 * s));
                    path.line_to(Point::new(0.25 * s, 0.35 * s));
                    path.line_to(Point::new(0.55 * s, 0.1 * s));
                    path.line_to(Point::new(0.55 * s, 0.9 * s));
                    path.line_to(Point::new(0.25 * s, 0.65 * s));
                    path.line_to(Point::new(0.5, 0.65 * s));
                    path.close();
                });
                frame.fill(&cone, self.color);
                let wave = canvas::Path::new(|path| {
                    path.arc(canvas::path::Arc {
                        center: Point::new(0.55 * s, s / 2.0),
                        radius: 0.35 * s,
                        start_angle: iced::Radians(-0.9),
                        end_angle: iced::Radians(0.9),
                    });
                });
                frame.stroke(&wave, line(1.25));
                return vec![frame.into_geometry()];
            }
            Glyph::Follow => {
                let shaft =
                    canvas::Path::line(Point::new(0.5, s / 2.0), Point::new(s - 4.0, s / 2.0));
                frame.stroke(&shaft, line(1.5));
                let head = triangle([(s - 5.5, 0.15 * s), (s - 0.5, s / 2.0), (s - 5.5, 0.85 * s)]);
                frame.fill(&head, self.color);
                return vec![frame.into_geometry()];
            }
            Glyph::MenuArrow => triangle([
                (1.0, s / 2.0 - 1.75),
                (w - 1.0, s / 2.0 - 1.75),
                (w / 2.0, s / 2.0 + 1.75),
            ]),
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

    /// The known answers `tests/fixtures/gen_u1_expected.py` works out from the font's own
    /// tables: the readouts' widths, the column beside Record.
    fn known() -> serde_json::Value {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/u1_font.json"
        )))
        .expect("u1_font.json")
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

    // Pins moved (were 252 and 286): the pattern menu opens under the name's box: 8 px
    // of side padding, the transport (four 38 px buttons 2 px apart: 158, or 174 with the 54 px
    // input meter under Record), 2 + a 1 px line + 2, the 75 px tempo (Inter's widest value,
    // gen_u1_expected.py; 76 in the monospace face before, the pins 272 and 288), 2 + 1 + 2, the
    // 20 px previous arrow: x 271, or 287 with the meter. (Pins moved: 296 and 316 with the
    // 40 px buttons.) So in a window with nothing to spare; a wider one moves it right by the
    // two gaps' shares. Pins moved (were 304 and
    // 322): the arrow is joined to the box (no 2 px between them), and the least space beside
    // each line is 2 px (was 3). Recording v2, pins moved (were 298 and 316): the 54 px column
    // beside Record (Notes over Audio over Capture) and its 2 px. Pins moved at 0.9 of the size
    // (were 298 and 316, 354 and 372 with 42 px buttons, the 60 px
    // meter, the 84 px tempo, the 22 px arrow and the 54 px column): the 50 px column, Inter's
    // "Capture" at 11 px and 4 px either side, rounded up (gen_u1_expected.py). With the 24 px
    // targets, pins moved (were 271 and 287, 323 and 339): the previous arrow 24 px (was 20), 275
    // and 291; the column's words at the type scale's 12 px, "Capture" 45.386 px and 4 px either
    // side, 54 (gen_u1_expected.py), so with them the name lies 54 + 2 further right, 331 and
    // 347. With the time signature right of the tempo, pins moved (were 275 and 291, 331 and
    // 347): the tempo's group is the 75 px tempo, 2 px (PART_SPACING) and the 41 px signature
    // ("44/44" at the numbers size, 11 px: (4 · 1323 + 738) · 11 / 2048 = 32.388, and 4 px
    // either side, rounded up; gen_u1_expected.py), 118 px, 43 more, so the name lies at 275 +
    // 43 = 318 and 291 + 43 = 334, with the column's words 331 + 43 = 374 and 347 + 43 = 390.
    #[test]
    fn the_pattern_name_starts_past_the_transport_tempo_and_arrow() {
        assert_eq!(known()["take_column_words"], 54);
        assert_eq!(take_column_width(Column::Words), 54.0);
        // With the badge on Record (no column) the name's box lies where it lay
        // before Recording v2.
        assert_eq!(known()["readout"]["signature_width"], 41);
        assert_eq!(signature_width(), 41.0);
        assert_eq!(tempo_group_width(), 75.0 + 2.0 + 41.0);
        for (input_meter, x) in [(false, 318.0), (true, 334.0)] {
            let least = least_width(&groups(
                Tier::Full,
                Column::Badge,
                true,
                None,
                false,
                input_meter,
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
        for (input_meter, x) in [(false, 374.0), (true, 390.0)] {
            let least = least_width(&groups(
                Tier::Full,
                Column::Words,
                true,
                None,
                false,
                input_meter,
            ));
            let shape = Shape {
                width: least,
                retry: false,
                notices: None,
                input_meter,
            };
            assert_eq!(Spread::of(shape).pattern_name_x(), x);
            // 321 px to spare: 40 a gap over the eight gaps (in even pixels; a pixel over 320, as
            // a sum of measured widths in f32 can come out a hair under its value, and 319.99997
            // px is 38 a gap), the name two gaps' shares right. Pins moved (was least + 700 px, x +
            // 200, before the toolbar scale; 281 over seven gaps until the notices' cell kept its
            // gap beside the panel buttons): with 700 to spare, more than 40 a gap,
            // the groups stand left 40 px past the least gap apart, so the name lies where the
            // 321 px left it.
            for spared in [321.0, 700.0] {
                let wider = Spread::of(Shape {
                    width: least + spared,
                    ..shape
                });
                assert_eq!(wider.packed, spared > 321.0, "{spared} px to spare");
                assert_eq!(
                    wider.pattern_name_x(),
                    x + 2.0 * 40.0,
                    "{spared} px to spare"
                );
            }
        }
    }

    // Across a wider window every group keeps its width and the gaps
    // between them share what the window has to spare equally, in whole pixels; the flex gap,
    // before the panel buttons (or File, with them folded), takes what the rounding leaves, so
    // the first group starts at the side padding and Help ends at the other. A notice shorter
    // than the room keeps its own width. In even pixels, so the line in a gap's middle has as
    // much space on its left as on its right (the flex gap's rest is under two pixels a gap).
    // With a share over [`PACKED_EXTRA`] the gaps are that ([`PACKED_EXTRA`] past the least)
    // and the flex gap takes the rest, its line beside the panel buttons while the notices or
    // Retry show, else beside the keyboard. (Pins moved: the last gap, before File, was the
    // flex one while the panel buttons stood after the keyboard.)
    #[test]
    fn the_groups_spread_across_the_toolbar() {
        // The widths of windows in use (1000, 1280, a full screen's 2259) and wider, an armed
        // input's meter on some.
        for (width, notices, retry, input_meter) in [
            (1920.0, None, false, false),
            (1920.0, Some(200.0), true, false),
            (2560.0, Some(NOTICE_MOST), false, false),
            (1000.0, None, false, false),
            (1333.0, Some(97.5), false, false),
            (880.0, None, true, false),
            (1280.0, None, false, true),
            (1400.0, None, false, false),
            // With the time signature's 43 px the bar's least width with the column's words,
            // the panel buttons and no notice is 1105.292 px, so a notice of its most (288) keeps
            // its own width from 1105.292 + 288 = 1393.292 px, 1400 among them.
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
            let flex = spread.flex();
            let right = spread.groups()[flex + 1].0;
            assert!(
                matches!(right, Group::Panels | Group::Menus),
                "{width} px: the flex gap before {right:?}"
            );
            let last = &gaps[flex];
            let rest: Vec<f32> = gaps
                .iter()
                .enumerate()
                .filter(|&(index, _)| index != flex)
                .map(|(_, &gap)| gap)
                .collect();
            println!("{width} px: gaps {gaps:?}, packed {}", spread.packed);
            assert!(
                rest.iter().all(|gap| (gap - rest[0]).abs() < 1e-3),
                "{gaps:?}"
            );
            assert_eq!((rest[0] - GAP) % 2.0, 0.0, "even pixels: {gaps:?}");
            if spread.packed {
                assert_eq!(rest[0], GAP + PACKED_EXTRA, "{width} px: {gaps:?}");
                assert!(*last > rest[0], "{gaps:?}");
                let line = spread.divider_xs()[flex];
                let (before, after) = (line - spans[flex].end, spans[flex + 1].start - line - 1.0);
                // To a thousandth of a pixel: f32 holds no finer at 4510 px.
                let half = BAR_SPACING + PACKED_EXTRA / 2.0;
                if spread.groups()[flex].0 == Group::Status {
                    assert!(
                        (after - half).abs() < 1e-3,
                        "{width} px: the line beside the panel buttons"
                    );
                } else {
                    assert!(
                        (before - half).abs() < 1e-3,
                        "{width} px: the line beside its group"
                    );
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
                .filter(|&(index, _)| index != flex)
            {
                let (before, after) =
                    (line - spans[index].end, spans[index + 1].start - line - 1.0);
                // To a thousandth of a pixel, as the gaps above: the groups' places are sums of
                // measured widths in f32 (at 1400 px, a scale step lower with
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
        // Packed exactly where the gaps' even share would pass 40 px, with or without a notice
        // of its most; 40 a gap at 1920 px.
        for width in [1000.0, 1280.0, 1400.0, 1920.0, 2259.0, 4510.0] {
            for notices in [false, true] {
                let spread = spread_at(width, false, notices);
                let spare = width - least_width(spread.groups());
                let gaps = (spread.groups().len() - 1) as f32;
                let share = (spare / gaps / 2.0).floor() * 2.0;
                assert_eq!(spread.packed, share > PACKED_EXTRA, "{width} px, {notices}");
            }
        }
        assert!(spread_at(1920.0, false, false).packed);
        assert_eq!(spread_at(1920.0, false, false).extra, 40.0);
    }

    // At full screen (2259 px) everything right of the transport packs to the left but Edit
    // and File, so that it fills the empty spaces, and the panel buttons stand beside File,
    // Edit and Help. The
    // bar with nothing but its groups (eight, and the notices' cell kept empty beside the panel
    // buttons: nine cells, eight gaps) packs from the first width where the gaps' even share
    // passes 40 px: 42 a gap, 8 × 42 = 336 px past its least width (worked from the rule, not
    // measured: 335.9 px share 40 and stay spread), 1062.292 + 336 = 1398.292 px (1057.292 +
    // 7 × 42 = 1351.292 before the empty cell's gap)
    // (printed; 1316.366 before File, Edit and Help grew 6.926 px, 1323.292 before they grew
    // another 10 %, 34 + 35 + 41 = 110 px to 38 + 39 + 46 = 123, 13 px more, 1336.292 before
    // File and Edit took Help's width, 46 + 46 + 46 = 138, 15 px more). There, and wider, every
    // group from the transport to the keyboard, and the line after each but the keyboard's, stays
    // exactly where a share of 40 laid it (1920, 2259, 2560, 4510 px), 45 px apart; the panel
    // buttons stand 45 px left of File, the line between them 22 px from the panel buttons, and
    // File, Edit and Help at the right edge; the keyboard's line 22 px past it, and the panel
    // buttons' left line 22 + 1 px before them, the empty cell between. A notice and Retry stand 45 px past the
    // keyboard (the notice's text first, left in its
    // cell), the line before the panel buttons 22 px from them, the cell between taking what the
    // window has to spare. (Pins moved: the panel buttons stood 45 px past the keyboard, the
    // notices' cell between them and File.)
    #[test]
    fn past_the_most_the_groups_stand_left_and_the_notices_cell_takes_the_rest() {
        let plain = least_width(&groups(Tier::Full, Column::Words, true, None, false, false));
        let first = plain + 8.0 * 42.0;
        println!("the bar packs from {first} px ({plain} px least)");
        // A thousandth over: the widths are sums of measured widths in f32, and 294 px past one
        // can come out a hair under 294.
        assert!(spread_at(first + 1e-3, false, false).packed);
        assert!(!spread_at(first - 0.1, false, false).packed);
        // Spread with a share of 40 each, the flex gap the rest: where the packed groups stand.
        let reached = spread_at(plain + 8.0 * 40.0 + 2.0, false, false);
        assert!(!reached.packed);
        assert_eq!(reached.extra, 40.0);
        assert!(
            (reached.last_extra - 42.0).abs() < 1e-3,
            "{}",
            reached.last_extra
        );
        let packed_groups = |spread: &Spread| -> Vec<(Group, f32)> {
            spread
                .groups()
                .iter()
                .take_while(|(group, _)| {
                    !matches!(group, Group::Status | Group::Panels | Group::Menus)
                })
                .map(|&(group, _)| (group, spread.left(group).expect("shown")))
                .collect()
        };
        let lefts = packed_groups(&reached);
        assert_eq!(lefts.len(), 6, "the transport to the keyboard");
        let lines = reached.divider_xs();
        let keyboard_end = reached.span(Group::Keyboard).expect("shown").end;
        let half = BAR_SPACING + PACKED_EXTRA / 2.0;
        for width in [first.ceil() + 2.0, 1920.0, 2259.0, 2560.0, 4510.0] {
            let plain = spread_at(width, false, false);
            assert!(plain.packed, "{width} px");
            assert_eq!(packed_groups(&plain), lefts, "{width} px");
            assert_eq!(plain.divider_xs()[..5], lines[..5], "{width} px");
            assert_eq!(
                plain.divider_xs()[5],
                keyboard_end + half,
                "{width} px: the keyboard's line keeps to it"
            );
            // To a thousandth of a pixel: the groups' places are sums of measured widths in f32
            // (at 0.9 of the size, 2251.0002 against 2251 at 2259 px).
            let menus = plain.span(Group::Menus).expect("shown");
            let panels = plain.span(Group::Panels).expect("shown");
            assert!(
                (menus.end - (width - TOOLBAR_SIDE_PADDING)).abs() < 1e-3,
                "{width} px: {menus:?}"
            );
            assert!(
                (panels.end + GAP + PACKED_EXTRA - menus.start).abs() < 1e-3,
                "{width} px: the panel buttons a gap left of File"
            );
            assert!(
                (plain.divider_xs()[6] + 1.0 + half - panels.start).abs() < 1e-3,
                "{width} px: the line before the panel buttons, beside them"
            );
            assert!(
                (plain.divider_xs()[7] - panels.end - half).abs() < 1e-3,
                "{width} px: the line after the panel buttons"
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
                if width < 1900.0 {
                    continue;
                }
                assert!(busy.packed, "{width} px, {retry} {notices:?}");
                assert_eq!(packed_groups(&busy), lefts, "{width} px");
                let status = busy.span(Group::Status).expect("the notices or Retry");
                assert_eq!(
                    status.start,
                    keyboard_end + GAP + PACKED_EXTRA,
                    "{width} px"
                );
                let busy_lines = busy.divider_xs();
                assert_eq!(busy_lines[5], keyboard_end + half, "{width} px");
                let panels = busy.span(Group::Panels).expect("shown");
                assert!(
                    (busy_lines[6] + 1.0 + half - panels.start).abs() < 1e-3,
                    "{width} px: the line beside the panel buttons"
                );
                let menus = busy.span(Group::Menus).expect("shown");
                assert!(
                    (panels.end + GAP + PACKED_EXTRA - menus.start).abs() < 1e-3,
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
            "2259 px: {:?}, lines {:?}, the cell after the keyboard {} px",
            packed_groups(&owner),
            owner.divider_xs(),
            owner.last_extra + GAP
        );
    }

    // At 1000 px everything shows with the full labels, the panel buttons too; below that
    // the column beside Record gives way first (its words, then its letters, for the badge on
    // Record), then the panel buttons fold, then the labels shorten, and at 880 px with Retry the
    // short labels leave Retry, File, Edit and Help ("?") whole and a notice its least room.
    // Recording v2 and the Help button (47.584 px) had pushed every fold past 1000 px (the panel
    // buttons folded beside a notice there, the full labels with Retry and a notice held only
    // from 1016.5 px); at 0.9 of the size they lay under it again, the panel buttons beside a
    // notice's least room with 2 px to spare. The 24 px targets (the panel grid 79 px, was 61;
    // the pattern's arrows 24, were 20; the octave's column 24, was 23), the pattern's box
    // holding its 24 characters at 11 px (was 10) and the column's words at 12 (were 11), less
    // the labels at 13 (were 14), put the panel buttons beside a notice's least room past it
    // again (about 1019 px), so at 1000 px they fold beside a notice and show without one. File,
    // Edit and Help each 5 % wider (118 px, were 111.074) move every fold 6.926 px wider, as
    // printed: the column's words beside Record from 1029.292 px (1022.366), beside a notice
    // from 1082.292 (1075.366); the panel buttons beside a notice from 1026.292 (1019.366); the
    // full labels with Retry and a notice from 984.198 (977.272); the order of the groups moves
    // no fold, a least width being their sum. File, Edit and Help each another 10 % wider, each
    // rounded up (34 → 38, 35 → 39, 41 → 46: the group 118 → 131 px) move every fold 13 px
    // wider, as printed: the column's words beside Record from 1042.292 px, beside a notice
    // from 1095.292; the panel buttons beside a notice from 1039.292; the full labels with
    // Retry and a notice from 997.198, still under 1000. File and Edit as wide as Help (38 and 39
    // px to 46 each: the group 131 → 146 px) move every fold 15 px wider, as printed: the
    // column's words beside Record from 1057.292 px, beside a notice from 1110.292; the panel
    // buttons beside a notice from 1054.292, with nothing to say from 1001.292, so at 1000 px
    // they fold (1.292 px short); the full labels with Retry and a notice from 1012.198, so at
    // 1000 px those shorten. Each fold is worked out here from the least widths of the looks on
    // either side of it (printed), so the pins follow the sizes; the narrow pin holds every
    // place.
    // The time signature right of the tempo (the tempo's group 75 + 2 + 41 = 118 px, 43 more)
    // moves every fold 43 px wider, as printed and by hand from the groups' widths (transport
    // 158 with the badge, 214 with the column's words; pattern 187, short 95; Pat over Song and
    // Loop 84.689; position 80; keyboard 140.603, short 79.563; panel buttons 79; File, Edit and
    // Help 146, short 120; Retry 39.906; 8 px padding either side, 5 px a gap): the full labels,
    // the badge, no panel buttons, nothing to say: 16 + 158 + 118 + 187 + 84.689 + 80 + 140.603
    // + 146 + 6 · 5 = 960.292 px; the panel buttons with their empty cell, two groups and two
    // gaps more, from 960.292 + 79 + 10 = 1049.292 (1006.292 before), beside a notice's least
    // room from 1049.292 + 48 = 1097.292 (1054.292); the column's words, 56 px more, from
    // 1105.292 (1062.292), beside a notice from 1153.292 (1110.292); the full labels without
    // the panel buttons beside a notice from 960.292 + 48 + 5 = 1013.292 (970.292), so at 1000
    // px with a notice the labels now shorten, the panel buttons showing (the short labels with
    // them beside a notice: 16 + 158 + 118 + 95 + 84.689 + 80 + 79.563 + 48 + 79 + 120 + 8 · 5 =
    // 918.253 px); with Retry and a notice from 960.292 + 48 + 2 + 39.906 + 5 = 1055.198
    // (1012.198). At 880 px with Retry and a notice the short labels without the panel buttons
    // take 960.292 − 92 − 61.039 − 26 = 781.253 px with no status, and the status (Retry, 2 px
    // and the notice) and its gap leave the notice 880 − 781.253 − 5 − 2 − 39.906 = 51.841 px,
    // over its least room (48 px).
    #[test]
    fn everything_fits_1000_px_and_the_labels_shorten_below() {
        let least = |tier, column, panels, notice: Option<f32>, retry| {
            least_width(&groups(tier, column, panels, notice, retry, false))
        };
        let room = Some(NOTICE_ROOM);
        let words_quiet = least(Tier::Full, Column::Words, true, None, false);
        let words_beside = least(Tier::Full, Column::Words, true, room, false);
        let badge_beside = least(Tier::Full, Column::Badge, true, room, false);
        let badge_quiet = least(Tier::Full, Column::Badge, true, None, false);
        let full_with_retry = least(Tier::Full, Column::Badge, false, room, true);
        println!(
            "the words beside Record from {words_quiet} px, beside a notice from {words_beside}; \
             the panel buttons beside a notice from {badge_beside}, with nothing to say from \
             {badge_quiet}; the full labels with Retry and a notice from {full_with_retry}"
        );
        let quiet = spread_at(1000.0, false, false);
        assert_eq!((quiet.tier, quiet.panels), (Tier::Full, false));
        assert!(badge_quiet > 1000.0, "{badge_quiet}");
        assert!(spread_at(badge_quiet, false, false).panels);
        assert!(!spread_at(badge_quiet - 0.1, false, false).panels);
        let busy = spread_at(1000.0, false, true);
        assert_eq!((busy.tier, busy.panels), (Tier::Short, true));
        assert!(busy.notice_width >= NOTICE_ROOM);
        assert!(badge_beside > 1000.0, "{badge_beside}");
        let full_beside = least(Tier::Full, Column::Badge, false, room, false);
        assert!((full_beside - 1013.292).abs() < 1e-3, "{full_beside}");
        assert_eq!(spread_at(full_beside, false, true).tier, Tier::Full);
        assert_eq!(spread_at(full_beside - 0.1, false, true).tier, Tier::Short);
        for (fold, want) in [
            (badge_quiet, 1049.292),
            (badge_beside, 1097.292),
            (words_quiet, 1105.292),
            (words_beside, 1153.292),
            (full_with_retry, 1055.198),
        ] {
            assert!((fold - want).abs() < 1e-3, "{fold} against {want} by hand");
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
        assert!(take_column_width(Column::Letters) < 24.0);
        // With Retry and a notice the full labels hold down to 1055.198 px (1012.198 before the
        // time signature), past 1000, the column given up for the badge first.
        assert!(full_with_retry > 1000.0, "{full_with_retry}");
        assert_eq!(spread_at(1000.0, true, true).tier, Tier::Short);
        let audio_off = spread_at(full_with_retry, true, true);
        assert_eq!(
            (audio_off.tier, audio_off.column),
            (Tier::Full, Column::Badge),
            "the column gives up its words before the labels shorten"
        );
        assert_eq!(
            spread_at(full_with_retry - 0.1, true, true).tier,
            Tier::Short
        );
        let short = spread_at(880.0, true, true);
        assert_eq!((short.tier, short.panels), (Tier::Short, false));
        assert!(short.notice_width >= NOTICE_ROOM, "{}", short.notice_width);
        assert!(
            (short.notice_width - 51.841).abs() < 1e-3,
            "{}",
            short.notice_width
        );
        // Beside a notice at the narrowest that keeps the panel buttons (1097.292 px; was
        // 1054.292 before the time signature).
        let wide = spread_at(badge_beside, false, true);
        let switch = wide.keyboard_switch();
        assert!(switch.x + switch.width < wide.left(Group::Panels).expect("panels"));
        assert_eq!(switch.height, BUTTON_HEIGHT);
        assert!(keyboard_switch_width(Tier::Short) < keyboard_switch_width(Tier::Full));
    }

    // Every gap holds a line, and the panel buttons stand between two: at 1920 px (packed, no
    // notice) the notices' cell stays, empty, beside them, so nine cells and eight lines; the
    // keyboard's line stands its gap's packed half from the keyboard, the panel buttons' left
    // line the same half (2 + 31 = 33 px: BAR_SPACING and half of PACKED_EXTRA) before them, and
    // their right line the shared half before File. At 1000 px with Retry and a notice the cell
    // holds them, the same lines either side. With the panel buttons folded no empty
    // cell: the keyboard's line and File's are one gap's.
    #[test]
    fn the_panel_buttons_stand_between_two_lines() {
        let half = BAR_SPACING + (PACKED_EXTRA / 2.0).floor();
        for (width, retry, notices) in [(1920.0, false, false), (1000.0, true, true)] {
            let spread = spread_at(width, retry, notices);
            let groups = spread.groups();
            let index = |wanted: Group| {
                groups
                    .iter()
                    .position(|&(group, _)| group == wanted)
                    .expect("shown")
            };
            assert_eq!(index(Group::Status) + 1, index(Group::Panels), "{width} px");
            let lines = spread.divider_xs();
            assert_eq!(lines.len(), groups.len() - 1, "{width} px");
            let panels = spread.left(Group::Panels).expect("shown");
            let before = lines[index(Group::Status)];
            assert!(
                before < panels && before > panels - GAP - half,
                "{width} px"
            );
            if spread.packed {
                assert_eq!(panels - (before + DIVIDER.width), half);
            }
        }
        let wide = spread_at(1920.0, false, false);
        assert_eq!(wide.groups().len(), 9);
        assert_eq!(
            wide.groups()[6],
            (Group::Status, 0.0),
            "the empty cell before the panel buttons"
        );
        let keyboard = wide.left(Group::Keyboard).expect("shown") + wide.groups()[5].1;
        assert_eq!(wide.divider_xs()[5], keyboard + half);
        let folded = groups(Tier::Short, Column::Badge, false, None, false, false);
        assert!(
            folded.iter().all(|&(group, _)| group != Group::Status),
            "no empty cell while the panel buttons are folded"
        );
    }

    // File, Edit and Help by hand, from Inter's advances at the body size (13 px; the generator
    // gen_u1_expected.py's rule): Help's label and 5 px either side, times 1.05 rounded up, times
    // 1.10 rounded up: 28.349 + 10 = 38.349 → 40.266 → 41 → 45.1 → 46, and File and Edit as
    // wide (were 38 and 39; their labels, 21.550 and 23.175 px, sit well inside). "?" (the short
    // labels) 16.646 → 17.478 → 18 → 19.8 → 20. The group with the menus' two 4 px spacings:
    // 46 + 4 + 46 + 4 + 46 = 146 px at the full labels (was 131), 46 + 4 + 46 + 4 + 20 = 120 at
    // the short (was 105).
    #[test]
    fn file_edit_and_help_are_as_wide_as_help() {
        assert_eq!(
            (
                menu_button_width(),
                help_width(Tier::Full),
                help_width(Tier::Short)
            ),
            (46.0, 46.0, 20.0)
        );
        assert_eq!(
            (menus_width(Tier::Full), menus_width(Tier::Short)),
            (146.0, 120.0)
        );
    }

    // With the Edit button beside File, a toolbar 880 px wide still leaves the notices room
    // for the audio notice cut with an ellipsis beside Retry, as the p8 probe looks for; File
    // first, Edit the menus' spacing right of it and Help
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
        // Help at the right padding (File was), "?" at the short labels; Edit
        // the menus' spacing left of it (File was, before the swap).
        assert_eq!(help.end, 872.0);
        assert_eq!(edit.end + MENU_SPACING, help.start);
        // Pins moved: the two 4 px apart (were a group's 7), Retry a line and its
        // spacing (2 + 1 + 2; 3 + 1 + 3 before) left of the first, File now.
        assert_eq!(file.end + MENU_SPACING, edit.start);
        assert!(
            (retry.end + GAP - file.start).abs() < 1e-3,
            "{retry:?} {file:?}"
        );
        assert!((880.0 - spread.edit_menu_right() - edit.end).abs() < 1e-3);
        assert!((880.0 - spread.file_menu_right() - file.end).abs() < 1e-3);
    }

    // While there is no loop range, the Loop button's tip says how to make one.
    #[test]
    fn the_loop_buttons_tip_says_how_to_set_a_range() {
        assert_eq!(loop_tip(None), "Drag in the ruler to set a loop range");
        assert_eq!(loop_tip(Some(false)), loop_tip(Some(true)));
        assert!(loop_tip(Some(true)).starts_with("Loop"));
    }

    // A pattern's name always fits its box: one
    // that fits at the labels' size shows so ("Pattern 1", "Pattern 12": the tenth pattern's
    // name); a longer one shrinks a pixel at a time, whole, down to the least size (a name of 24
    // characters is whole at a size between the two, and a pixel larger would not fit); a longer
    // one still is cut with an ellipsis at the least size, its first words kept. At the full
    // labels; and at the short ones, where the box is 47 px.
    #[test]
    fn a_long_pattern_name_shrinks_then_is_cut_with_an_ellipsis() {
        for name in ["Pattern 1", "Pattern 12"] {
            assert_eq!(
                shown_pattern_name(name, Tier::Full),
                (name.to_owned(), size::BODY),
                "{name}"
            );
        }
        let room = pattern_name_room(Tier::Full);
        let (held, size) = shown_pattern_name(PATTERN_NAME_HELD, Tier::Full);
        assert_eq!(held, PATTERN_NAME_HELD, "24 characters whole");
        assert_eq!(PATTERN_NAME_HELD.chars().count(), 24);
        assert!((NAME_LEAST..size::BODY).contains(&size), "{size}");
        assert!(text_width(PATTERN_NAME_HELD, size + 1.0) > room, "{size}");
        let (long, size) =
            shown_pattern_name("A very long name for the second verse pattern", Tier::Full);
        assert_eq!(size, NAME_LEAST);
        assert!(
            long.ends_with('\u{2026}') && long.starts_with("A very long"),
            "{long}"
        );
        assert!(text_width(&long, size) <= room);
        // Pin moved: the full box held "Pattern 12" at the labels' size (79 px at 0.9 of the
        // size), now 24 characters at 10 px.
        println!("the full box: {} px", pattern_name_width(Tier::Full));
        let (short, size) = shown_pattern_name("Pattern 1", Tier::Short);
        assert!(text_width(&short, size) <= pattern_name_room(Tier::Short));
    }

    // Any name, of any length, fits its box whole or cut, at a size between the least and the
    // labels': every start of a long name of mixed words, from none to 80 characters, at the full
    // labels and the short ones.
    #[test]
    fn a_pattern_name_of_any_length_fits_its_box() {
        let words = "Chorus bassline (take 2), the bridge's WIDE pad and an ending that goes on";
        for tier in [Tier::Full, Tier::Short] {
            let room = pattern_name_room(tier);
            for length in 0..=80 {
                let name: String = words.chars().cycle().take(length).collect();
                let (shown, size) = shown_pattern_name(&name, tier);
                assert!(
                    text_width(&shown, size) <= room,
                    "{name:?} {tier:?}: {shown:?} at {size}"
                );
                assert!((NAME_LEAST..=size::BODY).contains(&size), "{size}");
                if !shown.ends_with('\u{2026}') {
                    assert_eq!(shown, name);
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

    // Known answers by hand: the whole BPM, a point, then the thousandths as three digits with
    // the zeros at their end dropped, one digit kept: 120000 is 120 and 000, "120.0"; 120500
    // is 120 and 500, "120.5"; 120125, "120.125"; 99999 is 99 and 999, "99.999"; 120050 is 120
    // and 050, "120.05"; 140001, "140.001"; the range's ends 10000 and 522000, "10.0" and
    // "522.0". (Pin moved: always three decimals, "120.000".)
    #[test]
    fn tempo_reads_with_one_decimal_and_more_only_when_set() {
        let table = [
            (120_000, "120.0"),
            (120_500, "120.5"),
            (120_125, "120.125"),
            (99_999, "99.999"),
            (120_050, "120.05"),
            (174_500, "174.5"),
            (140_001, "140.001"),
            (10_000, "10.0"),
            (522_000, "522.0"),
        ];
        for (milli, shown) in table {
            assert_eq!(format_tempo(MilliBpm(milli)), shown, "{milli}");
            assert_eq!(
                parse_tempo(shown),
                Ok(MilliBpm(milli)),
                "{shown} typed back"
            );
        }
    }

    // The readouts reserve their widest value, worked from Inter's own advances
    // (gen_u1_expected.py): the widest digit is "4" (1323 units of 2048), so the tempo's
    // "000.000" is as wide as "444.444", 16 × (6 × 1323 + 590) / 2048 = 66.625 px, and 4 px
    // either side, rounded up: 75 px (76 in the monospace face before); the position's
    // "00:00:00" as "44:44:44", 16 × (6 × 1323 + 2 × 590) / 2048 = 71.234 px: 80 (86 before).
    // Every value, the range's ends and the widest the shapes hold, fits inside the padding.
    #[test]
    fn a_readout_reserves_its_widest_value() {
        let readout = &known()["readout"];
        assert_eq!(known()["widest_digit"], "4");
        let wanted = |key: &str| readout[key].as_f64().expect("a width") as f32;
        assert_eq!(
            (tempo_width(), position_width()),
            (wanted("tempo_width"), wanted("position_width"))
        );
        assert_eq!((tempo_width(), position_width()), (75.0, 80.0));
        let tempos = [
            *TEMPO_RANGE.start(),
            111_111,
            174_505,
            444_444,
            *TEMPO_RANGE.end(),
        ]
        .map(|milli| format_tempo(MilliBpm(milli)));
        let room = |width: f32| width - 2.0 * READOUT_PADDING;
        for value in tempos
            .iter()
            .map(String::as_str)
            .chain(["999.999", "888.888"])
        {
            let width = text_width(value, size::TITLE);
            assert!(width <= room(tempo_width()), "{value}: {width}");
        }
        for value in ["1:01:00", "10:01:00", "44:16:23", "99:16:23", "88:88:88"] {
            let width = text_width(value, size::TITLE);
            assert!(width <= room(position_width()), "{value}: {width}");
        }
    }

    // At ppq 96 a bar is 384 ticks and a step 24: tick 434 is bar 2, step 3 (48 ticks in),
    // tick 2 of that step.
    #[test]
    fn position_reads_bar_step_tick_from_one() {
        assert_eq!(format_position(Ticks(0), &common()), "1:01:00");
        assert_eq!(format_position(Ticks(120), &common()), "1:06:00");
        assert_eq!(format_position(Ticks(383), &common()), "1:16:23");
        assert_eq!(format_position(Ticks(434), &common()), "2:03:02");
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

    // The panel buttons (79 px: three of 25 px, 2 px apart; 61 before, 64 before
    // 0.9 of the size) show when the room left has them and, while there are
    // notices, a gap of 5 px (7 before) and 48 px more for those; else they
    // fold away and the notices
    // take it all.
    #[test]
    fn the_panel_buttons_fold_away_before_the_notices_lose_their_room() {
        assert_eq!(PANELS_WIDTH, 79.0);
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
            ))
        };
        let with = least(Some(NOTICE_ROOM));
        // The notices' cell and its gap are there with nothing to say too (empty, while the
        // panel buttons show), so a notice costs its room alone. (Pin moved: its
        // room and a gap, 53 px.)
        assert_eq!(with - least(None), NOTICE_ROOM);
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
        // At 1051 px: with the time signature's 43 px the panel buttons show with nothing to say
        // from 1049.292 px (1006.292 before), 1.708 px short of it.
        let rects = panel_button_rects(&spread_at(1051.0, false, false));
        assert_eq!(rects.len(), 6);
        // Two rows of three, the groups' 46 px centered on the 64 px toolbar: 9..31 and 33..55
        // (pins moved: 7..35 and 37..65 in 58 px before, 5..27 and 29..51 on the 56 px
        // toolbar, and 10..35 and 37..62 on the 72 px one), 25 px buttons 2 px apart (19 before
        // the 24 px targets): their centres 27 px apart across and 24 down, WCAG 2.2's spacing
        // exception for the 22 px rows.
        assert_eq!((rects[0].1.y, rects[3].1.y), (9.0, 33.0));
        assert_eq!(rects[0].1.height, 22.0);
        assert_eq!(rects[3].1.y + rects[3].1.height, 55.0);
        assert_eq!(rects[1].1.x - rects[0].1.x, 27.0);
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

    // Narrow windows lay the toolbar out to the pixel as before, every tier, fold and notice's
    // room (880 to 1200 px; with and without notices, Retry and the input meter). The pin holds
    // what the layout gave at 8247957, before the toolbar scale, written by the same line; it is
    // the "as before", not a value worked out here. The scale retired, the pin held byte for
    // byte: these widths were all at a scale of 1 already. Then rewritten by the same line with
    // the bundled Inter, its diff read group by group: no shape's look changed (tier,
    // column, panel buttons), the transport stayed 158 px, and only the measured groups moved:
    // the tempo 76 → 75 and the position 86 → 80 (their widest values in Inter, no longer the
    // monospace face's), the pattern 164 → 168, Pat over Song and Loop 88.3 → 89.5, the keyboard
    // 143.8 → 145.8 (short 79.6 → 80.1), File, Edit and Help 115.9 → 116.7 (short 91.5 → 93.3)
    // and the notices' cell by a pixel either way (Retry 3.5 px wider).
    // Pin rewritten by the same line with the Help button right of File: before
    // that, every one of the 60 lines of 8247957's pin came out of the new layout to the pixel
    // at its width plus Help's room (its label and the menus' spacing: 47.584 px at the full
    // labels, "?" 22.264 px at the short ones), Help's place left out, so Help moved every fold
    // by its own room and nothing else. Pin rewritten by the same line at 0.9 of the size
    // (every size of the bar 0.9 of what it was, the gaps kept), its lines
    // checked apart from the layout: each group starts where the one before ends plus a gap of
    // 5 px and its share, Help ends at the right padding, and every group and line spans y 9..55.
    // Rewritten again with File first: against the line
    // before, each line alike to the Edit menu's place, File starting where Edit started and
    // Edit ending where File ended, each as wide as before, Retry and Help where they were. And
    // again with the pattern's box holding 24 characters (164 px at the full labels, 45 more):
    // against the line before, each line whose look stayed has the pattern 45 px wider at the
    // full labels and every other group as wide; and against the pin at d5f9f02 (before 0.9),
    // no window of the 60 shows less than it did (33 show more: the column's words or letters,
    // the panel buttons or the full labels). Rewritten by the same line with the type scale and
    // the 24 px targets, its diff read group by group: the tempo (75) and the position (80) as
    // they were; the transport with the badge 158 (174 with the meter) as it was, with the
    // column's words 214 and 230 (were 210 and 226: "Capture" at 12 px, 54 with its sides, was
    // 50) and with its letters 194 (was 193); the pattern 187 at the full labels (was 168: its
    // two arrows 24 px, were 20, and its box holding the 24 characters at 11 px, was 10) and 95
    // at the short (was 87: the arrows alone); Pat over Song and Loop 84.689 (was 89.512: the
    // labels at 13 px, were 14); the keyboard 140.603 (was 145.803: its words at 13 px, the
    // octave's column 24 px, was 23) and 79.563 at the short labels (was 80.068); the panel
    // buttons 79 (was 61: 25 px buttons, were 19); File, Edit and Help 111.074 (was 116.695)
    // and 89.372 at the short labels (was 93.323); the notices' cell with what is left. 45 of
    // the 60 windows keep their look; 15 give a step more to the wider bar: at 1000 and 1051 px
    // the column's words give way to its letters (5) and its letters to the badge (3), and the
    // panel buttons fold beside Retry or a notice (5: 1000 px with a notice, 1000 with Retry,
    // 1051 with Retry and a notice); at 880 px the panel buttons fold beside Retry and a notice
    // (2), and with nothing to say the bar takes the short labels with the panel buttons where
    // it took the full labels without them (1).
    // Rewritten by the same line with the panel buttons moved beside File, Edit and Help (the
    // notices' cell before them, after the keyboard) and File, Edit and Help each 5 % wider,
    // rounded up (31.550, 33.175 and 38.349 px, "?" 16.646 at the short labels, to 34, 35, 41
    // and 18: the group 111.074 → 118 at the full labels, 89.372 → 95 at the short), and
    // checked line by line apart from the layout: from the old pin's measured widths, with the
    // menus' new widths and the new order, the look each line takes (the first of the looks
    // whose least width, 16 + the widths + 5 a gap, fits beside a notice's least room), the
    // notices' width, every group's width and place (8, then each width, 5 and the gap's even
    // share, the gap before the panel buttons, or File, taking the rest) and every line's
    // place (2 and half its gap's share past its group) come out of the rule as the new pin
    // has them, all 60 to a thousandth of a pixel. 56 of the 60 keep their look; 4 give a step
    // more to the 6.926 px wider menus: at 930 px with Retry the short labels with the panel
    // buttons where the full labels without them were; at 1000 px with Retry, a notice and the
    // meter (2) the same; at 1051 px with Retry and the meter the column's letters give way to
    // the badge.
    // Rewritten by the same line with File, Edit and Help each another 10 % wider, each rounded
    // up (38, 39 and 46 px, "?" 20: the group 131 px at the full labels, 105 at the short), and
    // checked line by line apart from the layout, as before: every group's width read off the
    // old pin but the menus', the menus' worked out by hand, then the look, the notices' width,
    // every group's place and line, the gaps' shares and every place read off them come out of
    // the rule as the new pin has them, all 60 to a thousandth of a pixel. 52 of the 60 keep
    // their look; 8 give a step more to the 13 px wider menus (10 at the short labels): at 1000
    // px with nothing to say the column's letters give way to the badge, and with the meter the
    // panel buttons fold; at 1051 px with the meter the column's words give way to its letters,
    // with Retry or a notice its letters to the badge (3), and with a notice and the meter the
    // panel buttons fold (2).
    // Rewritten by the same line with File and Edit as wide as Help (46 px each, "?" 20: the
    // group 146 px at the full labels, 120 at the short, 15 px more at both), checked line by
    // line as before, all 60 to a thousandth of a pixel. 47 keep their look; 13 give a step
    // more: at 880 px with the meter and Retry or a notice the panel buttons fold at the short
    // labels (3); at 930 px with the meter the full labels shorten (1), with the meter, Retry
    // and a notice the panel buttons fold (2); at 1000 px with nothing to say the panel buttons
    // fold (1), with Retry and a notice the labels shorten (2); at 1051 px with nothing to say
    // the column's words give way to its letters (1), with Retry and the meter the panel buttons
    // fold (1), with a notice the panel buttons fold (2).
    // Rewritten by the same line with the time signature right of the tempo (the tempo's group
    // 75 + 2 + 41 = 118 px, 43 more at both tiers), checked line by line apart from the layout:
    // every group's width read off the old pin but the tempo's, the tempo's worked out by hand
    // (118), then the look (the first of the looks in order whose least width the window holds,
    // the notices keeping their least room), the notices' width, every group's place and line,
    // the gaps' even shares and the flex gap's rest, the pattern's name and the keyboard switch,
    // recomputed from the rule by a script of its own and compared with the new pin, all 60 to a
    // thousandth of a pixel. 36 keep their look; 24 give a step more to the 43 px: at 880 px the
    // panel buttons fold with the meter, with Retry, with a notice (4); at 930 px with nothing to
    // say the full labels shorten, the panel buttons showing (1), with a notice and the meter or
    // Retry the panel buttons fold (4); at 1000 px with Retry or a notice the labels shorten, the
    // panel buttons showing (6); at 1051 px with nothing to say the column's letters give way to
    // the badge (1; with the meter the panel buttons fold too, 1), with Retry the panel buttons
    // fold (1), with Retry and a notice the labels shorten (4); at 1200 px with the meter, Retry
    // and a notice the column's words give way to its letters (2).
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

    // The octave's column, by hand: the groups are 46 px tall, centred in the 64 px bar from
    // (64 − 46) / 2 = 9; + is 10 px (y 9..19), 2 px, the octave's name 22 (21..43), 2 px, − 10
    // (45..55), so the name's middle, 9 + 10 + 2 + 22 / 2 = 32, is the bar's middle, 64 / 2,
    // and the keyboard switch's, 9 + 46 / 2. Each sign is drawn in a 7 px square centred in its
    // 24 × 10 button, (10 − 7) / 2 = 1.5 down, its bars 1.25 px thick centred on the square's
    // middle, 3.5: so each sign's middle is its button's, y 9 + 1.5 + 3.5 = 14 and 45 + 1.5 +
    // 3.5 = 50, 18 px either side of the name's middle. The name at its own 15 px (5 % under the
    // titles' 16) fits the 24 px column: its widest name measures 20.65 px in the app's font
    // (the keyboard's octaves run C2 to C8), and 22 px of row hold a 15 px line (19.5), centred
    // as before: only the glyphs are smaller; the row and its middle (32) stay. Spacing (WCAG 2.2's exception for the
    // 10 px tall buttons): their centres (12, 14) and (12, 50) in the column are 36 px apart, a
    // 24 px circle round each reaches 12 px, and the switch's right edge is 12 + 2 px (the
    // parts' spacing) left of their centres, the switch's own middle 73.54 px from them at the
    // full labels ((114.603 / 2 + 2 + 12, 18)) and 44.58 at the short ones (53.563 / 2 + 14,
    // 18). (Pins moved: + and − 16 px tall, the name 14 at 11 px, sign middles 17 and 47; the
    // name at the titles' 16 px after that.)
    #[test]
    fn the_octave_column_is_centred_on_the_switch() {
        let top = (TOOLBAR_HEIGHT - BUTTON_HEIGHT) / 2.0;
        assert_eq!(top, 9.0);
        assert_eq!((OCTAVE_BUTTON, OCTAVE_GAP, OCTAVE_NAME), (10.0, 2.0, 22.0));
        let name_top = top + OCTAVE_BUTTON + OCTAVE_GAP;
        let name_middle = name_top + OCTAVE_NAME / 2.0;
        assert_eq!(name_middle, TOOLBAR_HEIGHT / 2.0, "the bar's middle");
        assert_eq!(
            name_middle,
            top + BUTTON_HEIGHT / 2.0,
            "the switch's middle"
        );
        let plus = sign_bars(Glyph::Plus, OCTAVE_GLYPH);
        assert_eq!(
            plus,
            [
                Rectangle::new(Point::new(0.0, 2.875), Size::new(7.0, 1.25)),
                Rectangle::new(Point::new(2.875, 0.0), Size::new(1.25, 7.0)),
            ]
        );
        assert_eq!(sign_bars(Glyph::Minus, OCTAVE_GLYPH), plus[..1]);
        let inset = (OCTAVE_BUTTON - OCTAVE_GLYPH) / 2.0;
        let minus_top = name_top + OCTAVE_NAME + OCTAVE_GAP;
        let plus_middle = top + inset + plus[0].center_y();
        let minus_middle = minus_top + inset + plus[0].center_y();
        assert_eq!((plus_middle, minus_middle), (14.0, 50.0));
        assert_eq!(
            minus_top + OCTAVE_BUTTON,
            top + BUTTON_HEIGHT,
            "the group's foot"
        );
        assert_eq!(name_middle - plus_middle, minus_middle - name_middle);
        assert_eq!(plus[1].center_x(), OCTAVE_GLYPH / 2.0, "+ centred across");
        let widest = ["C2", "C3", "C4", "C5", "C6", "C7", "C8", "C0"]
            .map(|name| text_width(name, size::OCTAVE_NAME))
            .into_iter()
            .fold(0.0, f32::max);
        assert_eq!(size::OCTAVE_NAME, (size::TITLE * 0.95).round());
        assert!(
            widest <= OCTAVE_WIDTH && size::OCTAVE_NAME * 1.3 <= OCTAVE_NAME,
            "{widest}"
        );
        // The buttons' spacing: 36 px between their centres; the switch's right edge past a
        // 24 px circle round each; the switch's middle from theirs, at either labels' length.
        assert_eq!(minus_middle - plus_middle, 36.0);
        const { assert!(OCTAVE_WIDTH / 2.0 + PART_SPACING > 12.0) };
        let reach = |tier| {
            let dx = keyboard_switch_width(tier) / 2.0 + PART_SPACING + OCTAVE_WIDTH / 2.0;
            (dx * dx + 18.0 * 18.0).sqrt()
        };
        assert!(
            (reach(Tier::Full) - 73.54).abs() < 0.01,
            "{}",
            reach(Tier::Full)
        );
        assert!(
            (reach(Tier::Short) - 44.58).abs() < 0.01,
            "{}",
            reach(Tier::Short)
        );
    }

    /// tests/fixtures/gen_p34_expected.py's known answers (p34_expected.json).
    fn p34() -> serde_json::Value {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/p34_expected.json"
        )))
        .expect("p34_expected.json")
    }

    // Known answers 10 of gen_p34_expected.py: the position and a length count bars of the time
    // signature and its steps a bar: 300 in 3/4 and 6/8 is "2:01:12" (bar 2 from 288, 12 ticks
    // in), 372 in 6/8 "2:04:12", one 6/8 bar "1:00:00" long, the last tick of a 24/4 bar
    // "1:96:23" (two digits hold its 96 steps), 359 in 15/16 "1:15:23".
    #[test]
    fn the_readouts_count_bars_of_the_time_signature() {
        for case in p34()["readouts"].as_array().expect("readouts") {
            let sig = &case["signature"];
            let signature = TimeSignature::new(
                sig[0].as_u64().expect("beats") as u8,
                sig[1].as_u64().expect("unit") as u8,
            )
            .expect("a signature");
            let tick = Ticks(case["tick"].as_i64().expect("a tick"));
            assert_eq!(
                format_position(tick, &MeterMap::single(signature, 96)),
                case["position"].as_str().expect("a position"),
                "{signature} at {tick:?}"
            );
            assert_eq!(
                format_length(tick, signature, 96),
                case["length"].as_str().expect("a length"),
                "{signature}, {tick:?} long"
            );
        }
    }

    /// A song of one 4/4 at 96 ppq.
    fn common() -> MeterMap {
        MeterMap::single(dawsome_engine::time::TimeSignature::COMMON, 96)
    }

    // Known answers B 4 and C 6 of gen_p35_expected.py: the position counts the bars of the
    // meter that plays. Pattern 2 of p35_pattern_34 alone (its own 3/4): tick 300 is "2:01:12";
    // the song (4/4): "1:13:12". Along p35_meter's changes the bar comes from the map: 1100 is
    // "4:02:20" (44 into bar 4, 3/4 from 1056), 1700 "6:03:20", 2300 "8:02:20".
    #[test]
    fn the_position_counts_the_bars_of_the_meter_that_plays() {
        let p35: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/p35_expected.json"
        )))
        .expect("p35_expected.json");
        let load = |name: &str| {
            let text = std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../tests/fixtures")
                    .join(name),
            )
            .expect("a p35 fixture");
            dawsome_engine::project::Project::load_json(&text).expect("loads")
        };
        let b = load(p35["b"]["project"].as_str().expect("a file"));
        for case in p35["b"]["readouts"].as_array().expect("readouts") {
            let meter = match case["mode"].as_str() {
                Some("song") => b.meter_map(),
                _ => MeterMap::single(
                    b.pattern_signature(dawsome_engine::project::PatternId(2)),
                    96,
                ),
            };
            let tick = Ticks(case["tick"].as_i64().expect("a tick"));
            assert_eq!(
                format_position(tick, &meter),
                case["position"].as_str().expect("a position"),
                "{case}"
            );
        }
        let meter = load(p35["c"]["project"].as_str().expect("a file")).meter_map();
        for case in p35["c"]["readouts"].as_array().expect("readouts") {
            let tick = Ticks(case[0].as_i64().expect("a tick"));
            assert_eq!(
                format_position(tick, &meter),
                case[1].as_str().expect("a position"),
                "{case}"
            );
        }
    }

    #[test]
    fn lengths_read_bar_step_tick_from_zero() {
        assert_eq!(
            format_length(Ticks(24), dawsome_engine::time::TimeSignature::COMMON, 96),
            "0:01:00"
        );
        assert_eq!(
            format_length(Ticks(96), dawsome_engine::time::TimeSignature::COMMON, 96),
            "0:04:00"
        );
        assert_eq!(
            format_length(Ticks(434), dawsome_engine::time::TimeSignature::COMMON, 96),
            "1:02:02"
        );
        assert_eq!(
            format_length(Ticks(1), dawsome_engine::time::TimeSignature::COMMON, 96),
            "0:00:01"
        );
    }
}
