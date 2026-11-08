//! Every position in the Settings window's General page and header, in logical pixels from the
//! header's top-left corner: the drawing, the hit test and the screenshot probe all read these,
//! so they cannot drift apart. With the page unscrolled that corner is the Settings window's
//! (0, 0) (Settings was a panel under the toolbar before, its corner the main
//! window's (0, 72)).
//!
//! The header strip is 24 px. The General page's sections ([`Section`]): "MIDI input", its 24 px
//! rows (were 22; 24 px so each row is a press's whole target): "All ports", then one
//! per port (a 14 px checkbox, then the name), then the line saying no port was found when none
//! was, the counts of what was dropped when anything was, and what went wrong opening a port;
//! then the "Typing keyboard" section with its octave stepper; then the "Hints" section with
//! its one checkbox; then the "Display" section with its zoom stepper (− 100 % +, the octave's
//! look, its value a little wider); then the "Audio output" section ([`AudioLayout`]): a row
//! per device it offers ("System default" first), each with a round mark, then the line saying
//! what plays and, when the list failed, why; then the "Audio input" section, laid out as the
//! output's; then the "Input latency" section ([`LatencyLayout`]): the devices a take of the
//! line in uses, the figure kept for them, and a row of buttons (Measure, a millisecond less
//! and more, Clear). The sections stand in columns across the page ([`Columns`]): three on a
//! page 1200 px wide or more (the MIDI input to the display; the audio output; the audio input
//! and the input latency), two from 800 px (the MIDI input to the audio output; the audio input
//! and the input latency), one below that, in the order above; "Reset to defaults" under them
//! all, under the first column. Why columns: in a wide window the one column left most of the
//! page empty and the devices' rows ran down out of sight. The header holds the
//! page tabs, General (all of the above), Keys ([`crate::ui::keys_page`]) and Theme
//! ([`crate::ui::theme_page`]: the themes' keys and the accent, under Display until they stood
//! between the zoom and the audio devices); it stays at the window's top while a page scrolls
//! under it.

use iced::{Point, Rectangle, Size};

use crate::ui::color_wheel::WheelPart;
use crate::ui::palette::ThemeId;
use crate::ui::theme_page::ColorTarget;

pub const HEADER_HEIGHT: f32 = 24.0;
/// Space above a section's title on the Keys and Theme pages, and above the General page's
/// first title in each column.
pub const SECTION_GAP: f32 = 8.0;
/// The General page's space between two sections in a column, from one's last row to the
/// next one's title, and above "Reset to defaults". Why more than [`SECTION_GAP`] (8, which it
/// was): with the titles at 16 px the sections stand 28 px apart, so each reads as its own.
pub const GENERAL_GAP: f32 = 28.0;
pub const SECTION_TITLE_HEIGHT: f32 = 20.0;
pub const ROW_HEIGHT: f32 = 24.0;
/// The left margin of titles and checkboxes.
pub const INSET: f32 = 12.0;
pub const CHECKBOX: f32 = 14.0;
/// Where a row's text starts: after the checkbox and a gap.
pub const LABEL_X: f32 = INSET + CHECKBOX + 8.0;
/// How far right a row's text box reaches on the Theme page.
pub const TEXT_WIDTH: f32 = 360.0;
/// The octave stepper: its label, the two buttons and the octave's name between them (the
/// buttons from the column's margin: 110 in the first column).
pub const STEPPER_X: f32 = 110.0;
/// A stepper's − and + (the octave's, the zoom's, the input latency's), and the latency row's
/// Measure and Clear beside them: the row's whole 24 px. Why: WCAG 2.2's 24 px target, which
/// the row allows (they were 22 × 18 under its spacing exception).
pub const STEPPER_BUTTON: Size = Size::new(24.0, ROW_HEIGHT);
/// The height of the Settings pages' boxes and text keys that keep 18 px, centred in their 24 px
/// rows: "Reset to defaults", and the Theme page's menu, hex box and own-colour key.
pub const BOX_HEIGHT: f32 = 18.0;
pub const STEPPER_NAME_WIDTH: f32 = 36.0;
/// The zoom stepper's value between its buttons: "200 %" with room either side.
pub const ZOOM_VALUE_WIDTH: f32 = 48.0;
/// The Settings window's width at its first opening (and a [`Layout`]'s by default).
pub const FIRST_WIDTH: f32 = 720.0;
/// The General page takes one column more for each 400 px of its width, up to three.
pub const COLUMN_STEP: f32 = 400.0;
/// The most a column's inside is wide (its titles' and rows' room from its margin), and the
/// space between two columns.
pub const COLUMN_MOST: f32 = 520.0;
pub const COLUMN_GAP: f32 = 32.0;

/// The General page's sections, in the order one column shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Midi,
    Typing,
    Hints,
    Display,
    Output,
    Input,
    Latency,
}

impl Section {
    pub const ALL: [Section; 7] = [
        Section::Midi,
        Section::Typing,
        Section::Hints,
        Section::Display,
        Section::Output,
        Section::Input,
        Section::Latency,
    ];

    /// The column it stands in on a page of `count` columns: three put the audio output in the
    /// second and the audio input with the input latency in the third, two the audio input
    /// with the latency in the second, one holds them all. Why the latency with the input: it
    /// is the input's (a take of the line in), so it stays under it whichever column is
    /// shorter.
    pub fn column(self, count: usize) -> usize {
        match (count, self) {
            (3, Section::Output) => 1,
            (3, Section::Input | Section::Latency) => 2,
            (2, Section::Input | Section::Latency) => 1,
            _ => 0,
        }
    }
}

/// How the General page splits its width: how many columns, and each one's inside.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Columns {
    pub count: usize,
    /// A column's inside: from its margin ([`Columns::x`]) to its end.
    pub inner: f32,
}

impl Columns {
    /// For a page `width` wide: three from 3 × [`COLUMN_STEP`] (1200 px), two from 2 ×
    /// (800), one below; the width less the margins either side and the gaps between shared
    /// evenly, each at most [`COLUMN_MOST`] (the columns then start from the left).
    pub fn of(width: f32) -> Self {
        let count = (1..=3)
            .rev()
            .find(|&count| width >= count as f32 * COLUMN_STEP)
            .unwrap_or(1);
        let gaps = COLUMN_GAP * (count - 1) as f32;
        let inner = ((width - 2.0 * INSET - gaps) / count as f32).clamp(0.0, COLUMN_MOST);
        Columns { count, inner }
    }

    /// Where column `column`'s titles and boxes start: the margin, then a column and a gap for
    /// each before it.
    pub fn x(&self, column: usize) -> f32 {
        INSET + column as f32 * (self.inner + COLUMN_GAP)
    }
}

/// What the General page lists and how wide it is, which decide where its parts are.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    /// MIDI input ports listed.
    pub ports: usize,
    /// Lines of counts and problems under the ports (dropped messages, lost recorded notes,
    /// a port that would not open).
    pub notes: usize,
    /// Output devices offered, and the lines under them (what plays, why the list is short).
    pub outputs: usize,
    pub output_lines: usize,
    /// Input devices offered, and the line under them when the list is short.
    pub inputs: usize,
    pub input_lines: usize,
    /// The page's width: the Settings window's.
    pub width: f32,
}

impl Default for Layout {
    /// Nothing listed, at the window's first width.
    fn default() -> Self {
        Layout {
            ports: 0,
            notes: 0,
            outputs: 0,
            output_lines: 0,
            inputs: 0,
            input_lines: 0,
            width: FIRST_WIDTH,
        }
    }
}

/// A page of the settings panel, chosen by the tabs in its header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsPage {
    /// MIDI inputs, the typing keyboard's octave, the hints, the audio output.
    #[default]
    General,
    /// The shortcuts and the note layout.
    Keys,
    /// The theme and the accent.
    Theme,
}

impl SettingsPage {
    pub const ALL: [SettingsPage; 3] = [
        SettingsPage::General,
        SettingsPage::Keys,
        SettingsPage::Theme,
    ];

    pub fn title(self) -> &'static str {
        match self {
            SettingsPage::General => "General",
            SettingsPage::Keys => "Keys",
            SettingsPage::Theme => "Theme",
        }
    }

    /// Its name on the command line (`--settings-page keys`).
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|page| page.title().eq_ignore_ascii_case(name))
    }
}

/// Where the header's first tab starts, and each tab's width (General, Keys, Theme): its title
/// with room either side, 8 px or so.
const TAB_X: f32 = 90.0;
const TAB_WIDTHS: [f32; SettingsPage::ALL.len()] = [64.0, 44.0, 56.0];
const TAB_GAP: f32 = 4.0;

/// The tab of `page` in the header.
pub fn tab(page: SettingsPage) -> Rectangle {
    let index = SettingsPage::ALL
        .iter()
        .position(|&shown| shown == page)
        .unwrap_or(0);
    let x = TAB_X
        + TAB_WIDTHS[..index]
            .iter()
            .map(|width| width + TAB_GAP)
            .sum::<f32>();
    Rectangle::new(
        Point::new(x, 2.0),
        Size::new(TAB_WIDTHS[index], HEADER_HEIGHT - 4.0),
    )
}

/// The tab under `point`, panel coordinates.
pub fn tab_at(point: Point) -> Option<SettingsPage> {
    SettingsPage::ALL
        .into_iter()
        .find(|&page| tab(page).contains(point))
}

/// What a press in the panel lands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    AllPorts,
    /// Port number `index` of the list.
    Port(usize),
    OctaveDown,
    OctaveUp,
    /// The hint bar's checkbox.
    ShowHints,
    /// The zoom's stepper: a stop smaller, its value (a double-click sets 100 %), a stop larger.
    ZoomDown,
    ZoomValue,
    ZoomUp,
    /// A theme's key on the Theme page (which [`crate::ui::theme_page::hit`] finds, as it does
    /// the colours' parts under it).
    Theme(ThemeId),
    /// The menu choosing what the colour wheel edits, a widget of its own that takes its
    /// presses (named here for the hint line).
    ColorChoice,
    /// The colour wheel editing the target: its ring (the hue) or its square (saturation and
    /// value).
    Wheel(ColorTarget, WheelPart),
    /// The target's hex box, a text box of its own that takes its presses (named here for the
    /// hint line).
    Hex(ColorTarget),
    /// The key giving the target its own colour again: the theme's accent ("Theme's own"), each
    /// channel's colour for the notes ("Channel's own").
    OwnColor(ColorTarget),
    /// A page's tab in the header (which [`tab_at`] finds, not [`Layout::hit`]).
    Page(SettingsPage),
    /// The input latency's buttons (which [`LatencyLayout::hit`] finds).
    Measure,
    LatencyDown,
    LatencyUp,
    LatencyClear,
    /// "Reset to defaults" at the General page's foot (which [`Layout::reset`] places).
    ResetDefaults,
}

impl Layout {
    /// The page's columns at its width.
    pub fn columns(&self) -> Columns {
        Columns::of(self.width)
    }

    /// The rows `section` holds under its title.
    fn rows(&self, section: Section) -> usize {
        match section {
            Section::Midi => 1 + self.ports.max(1) + self.notes,
            Section::Typing | Section::Hints | Section::Display => 1,
            Section::Output => self.outputs + self.output_lines,
            Section::Input => self.inputs + self.input_lines,
            Section::Latency => 3,
        }
    }

    /// `section`'s height: its title and its rows.
    fn height(&self, section: Section) -> f32 {
        SECTION_TITLE_HEIGHT + ROW_HEIGHT * self.rows(section) as f32
    }

    /// Where `section`'s title starts down the page: the first of its column a section's gap
    /// under the header, each other [`GENERAL_GAP`] under the one before it in its column.
    pub fn top(&self, section: Section) -> f32 {
        let count = self.columns().count;
        let column = section.column(count);
        Section::ALL
            .into_iter()
            .take_while(|&before| before != section)
            .filter(|before| before.column(count) == column)
            .fold(HEADER_HEIGHT + SECTION_GAP, |top, before| {
                top + self.height(before) + GENERAL_GAP
            })
    }

    /// Where `section`'s title and boxes start across the page: its column's margin.
    pub fn left(&self, section: Section) -> f32 {
        let columns = self.columns();
        columns.x(section.column(columns.count))
    }

    /// `section`'s title, across its column.
    pub fn title(&self, section: Section) -> Rectangle {
        Rectangle::new(
            Point::new(self.left(section), self.top(section)),
            Size::new(self.columns().inner, SECTION_TITLE_HEIGHT),
        )
    }

    /// Row `row` under `section`'s title: from the column's left margin to its end, a press's
    /// whole target.
    fn row_of(&self, section: Section, row: usize) -> Rectangle {
        let top = self.top(section) + SECTION_TITLE_HEIGHT + ROW_HEIGHT * row as f32;
        Rectangle::new(
            Point::new(self.left(section) - INSET, top),
            Size::new(INSET + self.columns().inner, ROW_HEIGHT),
        )
    }

    /// Where a row's words sit: past its box and the gap after it, to the column's end.
    fn words_of(&self, row: Rectangle) -> Rectangle {
        let x = row.x + LABEL_X;
        Rectangle::new(
            Point::new(x, row.y),
            Size::new(row.x + row.width - x, ROW_HEIGHT),
        )
    }

    /// The box at a row's start (a checkbox, a device's round mark), centred in its height.
    fn box_of(row: Rectangle) -> Rectangle {
        Rectangle::new(
            Point::new(row.x + INSET, row.y + (ROW_HEIGHT - CHECKBOX) / 2.0),
            Size::new(CHECKBOX, CHECKBOX),
        )
    }

    /// The "MIDI input" section's title.
    pub fn midi_title(&self) -> Rectangle {
        self.title(Section::Midi)
    }

    /// Row `row` of the MIDI section: 0 is "All ports", then the ports.
    pub fn row(&self, row: usize) -> Rectangle {
        self.row_of(Section::Midi, row)
    }

    /// The checkbox of `row`.
    pub fn checkbox(&self, row: usize) -> Rectangle {
        Self::box_of(self.row(row))
    }

    /// Where `row`'s text sits.
    pub fn label(&self, row: usize) -> Rectangle {
        self.words_of(self.row(row))
    }

    /// The line saying no port was found, when none was.
    pub fn empty_message(&self) -> Option<Rectangle> {
        (self.ports == 0).then(|| self.label(1))
    }

    /// Note line `line` under the ports (and under the empty line, when shown).
    pub fn note(&self, line: usize) -> Rectangle {
        let after = 1 + self.ports.max(1);
        self.label(after + line)
    }

    /// The "Typing keyboard" section's title.
    pub fn typing_title(&self) -> Rectangle {
        self.title(Section::Typing)
    }

    /// The octave stepper's row: its "Octave" label at the row's start.
    pub fn octave_row(&self) -> Rectangle {
        self.words_of(self.row_of(Section::Typing, 0))
    }

    pub fn octave_down(&self) -> Rectangle {
        self.stepper_button(Section::Typing, 0.0)
    }

    /// Where the octave's name sits, between the buttons.
    pub fn octave_name(&self) -> Rectangle {
        self.stepper_value(Section::Typing, STEPPER_NAME_WIDTH)
    }

    pub fn octave_up(&self) -> Rectangle {
        self.stepper_button(Section::Typing, STEPPER_BUTTON.width + STEPPER_NAME_WIDTH)
    }

    /// The "Hints" section's title, under the typing keyboard's.
    pub fn hints_title(&self) -> Rectangle {
        self.title(Section::Hints)
    }

    /// The hint bar's row: its checkbox, then its words.
    pub fn hints_row(&self) -> Rectangle {
        self.row_of(Section::Hints, 0)
    }

    pub fn hints_checkbox(&self) -> Rectangle {
        Self::box_of(self.hints_row())
    }

    /// The "Display" section's title, under the hints'.
    pub fn display_title(&self) -> Rectangle {
        self.title(Section::Display)
    }

    /// The zoom stepper's row: its "Zoom" label at the row's start.
    pub fn zoom_row(&self) -> Rectangle {
        self.words_of(self.row_of(Section::Display, 0))
    }

    pub fn zoom_down(&self) -> Rectangle {
        self.stepper_button(Section::Display, 0.0)
    }

    /// Where the zoom's value sits, between the buttons.
    pub fn zoom_value(&self) -> Rectangle {
        self.stepper_value(Section::Display, ZOOM_VALUE_WIDTH)
    }

    pub fn zoom_up(&self) -> Rectangle {
        self.stepper_button(Section::Display, STEPPER_BUTTON.width + ZOOM_VALUE_WIDTH)
    }

    /// Where a stepper's buttons start in `section`'s row: [`STEPPER_X`] from the page's edge
    /// in the first column, as far from its own margin in another.
    fn stepper_x(&self, section: Section) -> f32 {
        self.left(section) - INSET + STEPPER_X
    }

    /// A stepper's button `offset` past where its buttons start, centred in its row's height.
    fn stepper_button(&self, section: Section, offset: f32) -> Rectangle {
        let row = self.row_of(section, 0);
        Rectangle::new(
            Point::new(
                self.stepper_x(section) + offset,
                row.y + (ROW_HEIGHT - STEPPER_BUTTON.height) / 2.0,
            ),
            STEPPER_BUTTON,
        )
    }

    /// A stepper's value, `width` wide, between its two buttons.
    fn stepper_value(&self, section: Section, width: f32) -> Rectangle {
        let row = self.row_of(section, 0);
        Rectangle::new(
            Point::new(self.stepper_x(section) + STEPPER_BUTTON.width, row.y),
            Size::new(width, ROW_HEIGHT),
        )
    }

    /// The "Audio output" section, offering [`Layout::outputs`] devices.
    pub fn audio(&self) -> AudioLayout {
        self.devices(Section::Output, self.outputs)
    }

    /// The "Audio input" section, laid out as the output's.
    pub fn input(&self) -> AudioLayout {
        self.devices(Section::Input, self.inputs)
    }

    fn devices(&self, section: Section, choices: usize) -> AudioLayout {
        AudioLayout {
            first: self.row_of(section, 0),
            title: self.title(section),
            choices,
        }
    }

    /// The "Input latency" section, under the audio input's, and "Reset to defaults" under the
    /// page's columns.
    pub fn latency(&self) -> LatencyLayout {
        let words = self.words_of(self.row_of(Section::Latency, 0));
        LatencyLayout {
            title: self.title(Section::Latency),
            words,
            reset: self.reset(),
        }
    }

    /// Where the columns end: under the lowest one's last section.
    fn columns_bottom(&self) -> f32 {
        Section::ALL
            .into_iter()
            .map(|section| self.top(section) + self.height(section))
            .fold(0.0, f32::max)
    }

    /// "Reset to defaults", the General page's last thing: [`GENERAL_GAP`] under the lowest
    /// column, in a row across the page, at the first column's margin, [`BOX_HEIGHT`] tall and
    /// centred in its row.
    pub fn reset(&self) -> Rectangle {
        let top = self.columns_bottom() + GENERAL_GAP;
        Rectangle::new(
            Point::new(self.columns().x(0), top + (ROW_HEIGHT - BOX_HEIGHT) / 2.0),
            Size::new(RESET_WIDTH, BOX_HEIGHT),
        )
    }

    /// Where the General page ends: under the reset button's row, and a section's gap.
    pub fn page_bottom(&self) -> f32 {
        self.columns_bottom() + GENERAL_GAP + ROW_HEIGHT + SECTION_GAP
    }

    /// What `point` lands on: a row of the MIDI section (its checkbox or its name), a
    /// stepper's button or value, or the hints' row.
    pub fn hit(&self, point: Point) -> Option<Hit> {
        let steppers = [
            (self.octave_down(), Hit::OctaveDown),
            (self.octave_up(), Hit::OctaveUp),
            (self.zoom_down(), Hit::ZoomDown),
            (self.zoom_value(), Hit::ZoomValue),
            (self.zoom_up(), Hit::ZoomUp),
        ];
        if let Some(hit) = steppers
            .into_iter()
            .find_map(|(rect, hit)| rect.contains(point).then_some(hit))
        {
            return Some(hit);
        }
        if self.hints_row().contains(point) {
            return Some(Hit::ShowHints);
        }
        (0..=self.ports)
            .find(|&row| self.row(row).contains(point))
            .map(|row| match row {
                0 => Hit::AllPorts,
                port => Hit::Port(port - 1),
            })
    }
}

/// Where an audio device section's parts are (the output's or the input's).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioLayout {
    /// Its title, and the row of its first choice (each next one a row lower).
    title: Rectangle,
    first: Rectangle,
    /// The devices offered, "System default" first.
    pub choices: usize,
}

impl AudioLayout {
    pub fn title(&self) -> Rectangle {
        self.title
    }

    /// The row of choice `index`: its mark, then the device's name.
    pub fn row(&self, index: usize) -> Rectangle {
        self.first + iced::Vector::new(0.0, ROW_HEIGHT * index as f32)
    }

    /// The round mark of choice `index`, filled when it is the one chosen.
    pub fn mark(&self, index: usize) -> Rectangle {
        Layout::box_of(self.row(index))
    }

    pub fn label(&self, index: usize) -> Rectangle {
        let row = self.row(index);
        let x = row.x + LABEL_X;
        Rectangle::new(
            Point::new(x, row.y),
            Size::new(row.x + row.width - x, ROW_HEIGHT),
        )
    }

    /// Line `line` under the choices: what plays, then why the list is short.
    pub fn line(&self, line: usize) -> Rectangle {
        self.label(self.choices + line)
    }

    /// The choice a press at `point` lands on.
    pub fn hit(&self, point: Point) -> Option<usize> {
        (0..self.choices).find(|&index| self.row(index).contains(point))
    }
}

/// The latency row's buttons: Measure, then a millisecond less and more, then Clear.
const MEASURE_WIDTH: f32 = 72.0;
const CLEAR_WIDTH: f32 = 48.0;
const BUTTON_GAP: f32 = 8.0;
/// "Reset to defaults" at the General page's foot: as wide as its longer words, "Press again to
/// reset" (112.69 px at 12 px), with a button's 8 px either side, rounded up to 4 px: 132.
pub const RESET_WIDTH: f32 = 132.0;

/// Where the "Input latency" section's parts are: its title, the line naming the devices, the
/// line with the figure, and the row of buttons; and "Reset to defaults", whose press its hit
/// test finds too.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatencyLayout {
    title: Rectangle,
    /// Where its first line's words sit; each next line a row lower.
    words: Rectangle,
    reset: Rectangle,
}

impl LatencyLayout {
    pub fn title(&self) -> Rectangle {
        self.title
    }

    /// Line 0 names the devices, line 1 holds the figure (or "Unknown"), line 2 the buttons.
    pub fn line(&self, line: usize) -> Rectangle {
        self.words + iced::Vector::new(0.0, ROW_HEIGHT * line as f32)
    }

    fn button(&self, x: f32, width: f32) -> Rectangle {
        let row = self.line(2);
        Rectangle::new(
            Point::new(x, row.y + (ROW_HEIGHT - STEPPER_BUTTON.height) / 2.0),
            Size::new(width, STEPPER_BUTTON.height),
        )
    }

    pub fn measure(&self) -> Rectangle {
        self.button(self.words.x, MEASURE_WIDTH)
    }

    pub fn down(&self) -> Rectangle {
        let measure = self.measure();
        self.button(measure.x + measure.width + BUTTON_GAP, STEPPER_BUTTON.width)
    }

    pub fn up(&self) -> Rectangle {
        let down = self.down();
        self.button(down.x + down.width, STEPPER_BUTTON.width)
    }

    pub fn clear(&self) -> Rectangle {
        let up = self.up();
        self.button(up.x + up.width + BUTTON_GAP, CLEAR_WIDTH)
    }

    /// "Reset to defaults" under the page's columns ([`Layout::reset`]).
    pub fn reset(&self) -> Rectangle {
        self.reset
    }

    /// The button a press at `point` lands on: the section's, or the reset.
    pub fn hit(&self, point: Point) -> Option<Hit> {
        [
            (self.measure(), Hit::Measure),
            (self.down(), Hit::LatencyDown),
            (self.up(), Hit::LatencyUp),
            (self.clear(), Hit::LatencyClear),
            (self.reset(), Hit::ResetDefaults),
        ]
        .into_iter()
        .find_map(|(rect, hit)| rect.contains(point).then_some(hit))
    }
}

/// A section's title at `top`, from the left margin: the Keys and Theme pages' (the General
/// page's stand in its columns, [`Layout::title`]).
pub fn title_at(top: f32) -> Rectangle {
    Rectangle::new(
        Point::new(INSET, top),
        Size::new(TEXT_WIDTH, SECTION_TITLE_HEIGHT),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::fonts::size;

    /// Two ports, three output devices and the line saying what plays, two input devices, on a
    /// page `width` wide: what the tables below are worked out for.
    fn listed(width: f32) -> Layout {
        Layout {
            ports: 2,
            outputs: 3,
            output_lines: 1,
            inputs: 2,
            width,
            ..Layout::default()
        }
    }

    // With two ports, on the window's first 720 px (one column): the title at y 32..52, then
    // 24 px rows (WCAG 2.2's target size; 22 before): "All ports" 52..76, the ports 76..100 and 100..124,
    // a port's 14 px box 5 px into its row (81), the typing section's title at 124 + 28 = 152
    // (the sections 28 px apart; it was 132 at 8), its stepper row 172..196.
    #[test]
    fn rows_follow_the_header_title_and_each_other() {
        let layout = listed(720.0);
        assert_eq!(layout.midi_title().y, 32.0);
        assert_eq!(
            [layout.row(0).y, layout.row(1).y, layout.row(2).y],
            [52.0, 76.0, 100.0]
        );
        assert_eq!(
            layout.checkbox(1),
            Rectangle::new(Point::new(12.0, 81.0), Size::new(14.0, 14.0))
        );
        assert_eq!(layout.empty_message(), None);
        assert_eq!(layout.typing_title().y, 152.0);
        assert_eq!(layout.octave_row().y, 172.0);
        assert_eq!(layout.hit(Point::new(18.0, 85.0)), Some(Hit::Port(0)));
        assert_eq!(layout.hit(Point::new(200.0, 60.0)), Some(Hit::AllPorts));
        assert_eq!(layout.hit(Point::new(18.0, 110.0)), Some(Hit::Port(1)));
        assert_eq!(layout.hit(layout.octave_up().center()), Some(Hit::OctaveUp));
        assert_eq!(
            layout.hit(layout.octave_down().center()),
            Some(Hit::OctaveDown)
        );
        assert_eq!(layout.hit(Point::new(18.0, 40.0)), None, "the title");
    }

    // The hints section 28 px under the stepper row (172..196): its title at 224, its checkbox
    // row 244..268, the box centered in it (249).
    #[test]
    fn the_hints_checkbox_follows_the_typing_keyboard() {
        let layout = listed(720.0);
        assert_eq!(layout.hints_title().y, 224.0);
        assert_eq!(layout.hints_row().y, 244.0);
        assert_eq!(layout.hints_checkbox().y, 249.0);
        assert_eq!(layout.hit(Point::new(18.0, 247.0)), Some(Hit::ShowHints));
        assert_eq!(layout.hit(Point::new(18.0, 234.0)), None, "the title");
    }

    // The display section 28 px under the hints row (244..268): its title at 296, the
    // zoom's row 316..340, the stepper as the octave's (− at x 110..134, y 316..340: 24 px
    // square, the row's whole height) with its value 48 px wide between (134..182) and + after
    // it (182..206); a press on each at the row's middle (328) is its hit, and on the buttons'
    // top and foot rows (316, 339) too; one on the title or the label none. (Pins moved with
    // the 24 px targets: − was 22 × 18 at 110..132 by 319..337, the value 132..180, + from 180.)
    // The octave's stepper is the same shape in its row (172..196): − 110..134, its name 36 px
    // (134..170), + 170..194.
    #[test]
    fn the_display_section_follows_the_hints() {
        let layout = listed(720.0);
        assert_eq!(layout.display_title().y, 296.0);
        assert_eq!(layout.zoom_row().y, 316.0);
        assert_eq!(
            layout.zoom_down(),
            Rectangle::new(Point::new(110.0, 316.0), Size::new(24.0, 24.0))
        );
        assert_eq!(
            layout.zoom_value(),
            Rectangle::new(Point::new(134.0, 316.0), Size::new(48.0, 24.0))
        );
        assert_eq!(
            layout.zoom_up(),
            Rectangle::new(Point::new(182.0, 316.0), Size::new(24.0, 24.0))
        );
        assert_eq!(layout.hit(Point::new(120.0, 328.0)), Some(Hit::ZoomDown));
        assert_eq!(layout.hit(Point::new(150.0, 328.0)), Some(Hit::ZoomValue));
        assert_eq!(layout.hit(Point::new(190.0, 328.0)), Some(Hit::ZoomUp));
        for y in [316.0, 339.0] {
            assert_eq!(layout.hit(Point::new(120.0, y)), Some(Hit::ZoomDown), "{y}");
            assert_eq!(layout.hit(Point::new(200.0, y)), Some(Hit::ZoomUp), "{y}");
        }
        assert_eq!(layout.hit(Point::new(18.0, 306.0)), None, "the title");
        assert_eq!(layout.hit(Point::new(40.0, 328.0)), None, "the label");
        assert_eq!(
            layout.octave_down(),
            Rectangle::new(Point::new(110.0, 172.0), Size::new(24.0, 24.0))
        );
        assert_eq!(
            layout.octave_name(),
            Rectangle::new(Point::new(134.0, 172.0), Size::new(36.0, 24.0))
        );
        assert_eq!(
            layout.octave_up(),
            Rectangle::new(Point::new(170.0, 172.0), Size::new(24.0, 24.0))
        );
    }

    // Every zoom's value fits its box between the stepper's buttons with 4 px either side, in
    // the app's font ("200 %" the widest).
    #[test]
    fn every_zoom_fits_its_value_box() {
        for zoom in (0..=200).filter_map(crate::zoom::Zoom::exact) {
            let width = crate::ui::text_fit::text_width(&zoom.to_string(), size::LABEL);
            assert!(width + 8.0 <= ZOOM_VALUE_WIDTH, "{zoom}: {width}");
        }
    }

    // The audio section 28 px under the Display section's zoom row (316..340): its title
    // at 368, its choices' rows from 388, 24 px each (the third at 436, the second's mark 5 px
    // into its row at 417), the status line after the last at 460; a press on a row is that
    // choice, one on the title or the status line none. Nothing under the zoom row is a hit of
    // the page's own: the zoom row's foot under its − (120, 343) and the audio section's title
    // (272, 374). (Pins moved 190 px up when the theme's keys and the accent went to the Theme
    // page: were 466, 486, 530, 513, 552; then down with the 24 px rows, 288, 308, 356, 337,
    // 380; then 80 px with the sections 28 px apart.)
    #[test]
    fn the_audio_section_follows_the_display() {
        let layout = listed(720.0);
        let audio = layout.audio();
        assert_eq!(audio.title().y, 368.0);
        assert_eq!([audio.row(0).y, audio.row(2).y], [388.0, 436.0]);
        assert_eq!(audio.mark(1).y, 417.0);
        assert_eq!(audio.line(0).y, 460.0);
        assert_eq!(audio.hit(Point::new(18.0, 392.0)), Some(0));
        assert_eq!(audio.hit(Point::new(300.0, 445.0)), Some(2));
        assert_eq!(audio.hit(Point::new(18.0, 378.0)), None, "the title");
        assert_eq!(audio.hit(Point::new(18.0, 470.0)), None, "the status line");
        for at in [Point::new(120.0, 343.0), Point::new(272.0, 374.0)] {
            assert_eq!(layout.hit(at), None, "{at:?}");
        }
    }

    // The latency section under the audio input (one column at 720 px, two output devices and
    // the line saying what plays: the output's title at 368, its section to 368 + 20 + 3 · 24 =
    // 460; the input's title at 488, two devices, to 556): its title at 584, the devices' line
    // 604, the figure's 628, the buttons' row 652..676, each button the row's 24 px; from the
    // rows' text (x 12 + 14 + 8 = 34, the 14 px box's) Measure 34..106, 8 px, − 114..138, +
    // 138..162, 8 px, Clear 170..218; a press between them is none. "Reset to defaults" 28 px
    // under the lowest column (here the one, to 676): its row 704..728, the button 18 px tall
    // centred in it, 12..144 by 707..725, and the page to 728 + 8 = 736. (Pins moved with the
    // 24 px targets: the buttons were 18 px tall from 655, − and + 22 wide, + from 136, Clear
    // from 166.)
    #[test]
    fn the_latency_section_follows_the_audio_input() {
        let layout = Layout {
            outputs: 2,
            ..listed(720.0)
        };
        let latency = layout.latency();
        assert_eq!(latency.title().y, 584.0);
        assert_eq!([latency.line(0).y, latency.line(1).y], [604.0, 628.0]);
        assert_eq!(
            latency.measure(),
            Rectangle::new(Point::new(34.0, 652.0), Size::new(72.0, 24.0))
        );
        assert_eq!(
            [latency.down(), latency.up(), latency.clear()],
            [
                Rectangle::new(Point::new(114.0, 652.0), Size::new(24.0, 24.0)),
                Rectangle::new(Point::new(138.0, 652.0), Size::new(24.0, 24.0)),
                Rectangle::new(Point::new(170.0, 652.0), Size::new(48.0, 24.0)),
            ]
        );
        assert_eq!(latency.hit(Point::new(40.0, 664.0)), Some(Hit::Measure));
        assert_eq!(
            latency.hit(Point::new(120.0, 664.0)),
            Some(Hit::LatencyDown)
        );
        assert_eq!(latency.hit(Point::new(150.0, 664.0)), Some(Hit::LatencyUp));
        assert_eq!(
            latency.hit(Point::new(120.0, 653.0)),
            Some(Hit::LatencyDown)
        );
        assert_eq!(latency.hit(Point::new(150.0, 675.0)), Some(Hit::LatencyUp));
        assert_eq!(
            latency.hit(Point::new(200.0, 664.0)),
            Some(Hit::LatencyClear)
        );
        assert_eq!(latency.hit(Point::new(110.0, 664.0)), None, "between");
        assert_eq!(latency.hit(Point::new(40.0, 633.0)), None, "the figure");
        assert_eq!(
            latency.reset(),
            Rectangle::new(Point::new(12.0, 707.0), Size::new(132.0, 18.0))
        );
        assert_eq!(
            latency.hit(Point::new(20.0, 716.0)),
            Some(Hit::ResetDefaults)
        );
        assert_eq!(layout.page_bottom(), 736.0);
    }

    // The General page's columns, by hand, for two ports, three output devices and their line,
    // two input devices (`listed`). The sections: the MIDI input 20 + 3 · 24 = 92 px tall, the
    // typing keyboard, the hints and the display 44 each, the audio output 20 + 4 · 24 = 116,
    // the audio input 68, the input latency 92; each column's first title 8 px under the 24 px
    // header (32), each next 28 px under the one before. The columns: the page less 12 px
    // margins either side and 32 px between, shared evenly, at most 520 each, from x 12.
    // - 720 px (the window's first width, under 800): one column, 696 → 520 wide at x 12; the
    //   titles at 32, 152, 224, 296, 368, 512 and 608, their first rows 20 lower; the lowest
    //   column ends at 700, "Reset to defaults" 28 px under it (its row 728, the button at 731).
    // - 1000 px (800 to 1200): two, (1000 − 24 − 32) / 2 = 472 wide at x 12 and 516; the first
    //   the MIDI input to the audio output (32, 152, 224, 296, 368, to 484), the second the
    //   audio input (32, to 100) and the latency (128, to 220); the reset under 484 (row 512,
    //   button 515).
    // - 1400 px (1200 and wider): three, (1400 − 24 − 64) / 3 = 437.333 wide at x 12, 481.333
    //   and 950.667; the first the MIDI input to the display (to 340), the second the audio
    //   output (32, to 148), the third the audio input (32) and the latency (128, to 220); the
    //   reset under 340 (row 368, button 371).
    #[test]
    fn the_general_page_stands_in_one_two_or_three_columns() {
        use Section::*;
        let near = |found: f32, wanted: f32| (found - wanted).abs() < 1e-3;
        type Row = (f32, usize, [f32; 3], [(Section, f32, f32); 7], f32);
        let table: [Row; 3] = [
            (
                720.0,
                1,
                [12.0, 12.0, 12.0],
                [
                    (Midi, 12.0, 52.0),
                    (Typing, 12.0, 172.0),
                    (Hints, 12.0, 244.0),
                    (Display, 12.0, 316.0),
                    (Output, 12.0, 388.0),
                    (Input, 12.0, 532.0),
                    (Latency, 12.0, 628.0),
                ],
                731.0,
            ),
            (
                1000.0,
                2,
                [12.0, 516.0, 516.0],
                [
                    (Midi, 12.0, 52.0),
                    (Typing, 12.0, 172.0),
                    (Hints, 12.0, 244.0),
                    (Display, 12.0, 316.0),
                    (Output, 12.0, 388.0),
                    (Input, 516.0, 52.0),
                    (Latency, 516.0, 148.0),
                ],
                515.0,
            ),
            (
                1400.0,
                3,
                [12.0, 481.333, 950.667],
                [
                    (Midi, 12.0, 52.0),
                    (Typing, 12.0, 172.0),
                    (Hints, 12.0, 244.0),
                    (Display, 12.0, 316.0),
                    (Output, 481.333, 52.0),
                    (Input, 950.667, 52.0),
                    (Latency, 950.667, 148.0),
                ],
                371.0,
            ),
        ];
        for (width, count, xs, sections, reset_y) in table {
            let layout = listed(width);
            let columns = layout.columns();
            assert_eq!(columns.count, count, "{width} px");
            for (column, x) in xs.into_iter().take(count).enumerate() {
                assert!(near(columns.x(column), x), "{width} px, column {column}");
            }
            for (section, x, first_row) in sections {
                let row = layout.row_of(section, 0);
                assert!(
                    near(layout.left(section), x),
                    "{width} px: {section:?} at x"
                );
                assert_eq!(row.y, first_row, "{width} px: {section:?}'s first row");
                assert_eq!(layout.title(section).y, first_row - 20.0);
                assert!(near(row.x + row.width, x + columns.inner));
            }
            let reset = layout.reset();
            assert_eq!((reset.x, reset.y), (12.0, reset_y), "{width} px: the reset");
        }
        // Where a page takes a column more, and the most a column takes.
        let counts = [399.0, 420.0, 799.0, 800.0, 1199.0, 1200.0, 1920.0]
            .map(|width| Columns::of(width).count);
        assert_eq!(counts, [1, 1, 1, 2, 2, 3, 3]);
        assert_eq!(Columns::of(420.0).inner, 396.0);
        assert!(near(Columns::of(1200.0).inner, 370.667));
        let widest = Columns::of(1920.0);
        assert_eq!(widest.inner, 520.0);
        assert_eq!([widest.x(1), widest.x(2)], [564.0, 1116.0]);
        // At 1400 px each column's presses stay in it: the output's first device under its
        // title, the input's in the third column, the latency's Measure there too.
        let three = listed(1400.0);
        assert_eq!(three.audio().hit(Point::new(600.0, 60.0)), Some(0));
        assert_eq!(three.audio().hit(Point::new(300.0, 60.0)), None);
        assert_eq!(three.input().hit(Point::new(1000.0, 84.0)), Some(1));
        assert_eq!(
            three.hit(Point::new(600.0, 60.0)),
            None,
            "no MIDI row there"
        );
        assert!(near(three.latency().measure().x, 972.667));
        assert_eq!(three.page_bottom(), 400.0);
    }

    // The header's tabs, General, Keys, then Theme, right of the title, inside the
    // header, each 4 px after the one before: General 90..154, Keys 158..202 and Theme
    // 206..262, by x = 90 + the widths and gaps before it (64 + 4, then 44 + 4); a press on
    // each is its page, one on the title or in a gap none. Each title fits its tab with 7 px
    // either side in the app's font (Keys, the tightest, has 7.9).
    #[test]
    fn the_tabs_sit_in_the_header() {
        let spans = SettingsPage::ALL.map(|page| {
            let rect = tab(page);
            (rect.x, rect.x + rect.width)
        });
        assert_eq!(spans, [(90.0, 154.0), (158.0, 202.0), (206.0, 262.0)]);
        for page in SettingsPage::ALL {
            let rect = tab(page);
            assert_eq!((rect.y, rect.y + rect.height), (2.0, 22.0), "{page:?}");
            assert!(rect.y + rect.height <= HEADER_HEIGHT);
            assert_eq!(tab_at(rect.center()), Some(page));
            let words = crate::ui::text_fit::text_width(page.title(), size::LABEL);
            assert!(words + 14.0 <= rect.width, "{}: {words}", page.title());
        }
        assert_eq!(tab_at(Point::new(20.0, 12.0)), None, "the title");
        assert_eq!(
            tab_at(Point::new(204.0, 12.0)),
            None,
            "between Keys and Theme"
        );
        assert_eq!(SettingsPage::from_name("keys"), Some(SettingsPage::Keys));
        assert_eq!(SettingsPage::from_name("Theme"), Some(SettingsPage::Theme));
        assert_eq!(SettingsPage::from_name("audio"), None);
    }

    // No port: the empty line takes the first port row's place (52 + 24 = 76), and the note
    // comes after it (100); the typing section's title 28 px under that row, 124 + 28.
    #[test]
    fn with_no_port_the_empty_line_takes_the_first_row() {
        let layout = Layout {
            notes: 1,
            ..Layout::default()
        };
        assert_eq!(layout.empty_message().map(|rect| rect.y), Some(76.0));
        assert_eq!(layout.note(0).y, 100.0);
        assert_eq!(layout.typing_title().y, 152.0);
        assert_eq!(layout.hit(Point::new(18.0, 80.0)), None, "the empty line");
    }
}
