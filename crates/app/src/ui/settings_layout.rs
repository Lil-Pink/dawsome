//! Every position in the Settings window's General page and header, in logical pixels from the
//! header's top-left corner: the drawing, the hit test and the screenshot probe all read these,
//! so they cannot drift apart. With the page unscrolled that corner is the Settings window's
//! (0, 0) (Settings was a panel under the toolbar before, its corner the main
//! window's (0, 72)).
//!
//! The header strip is 24 px. The "MIDI input" section's title follows, then 22 px rows:
//! "All ports", then one per port (a 12 px checkbox, then the name), then the line saying no
//! port was found when none was, the counts of what was dropped when anything was, and what
//! went wrong opening a port; then the "Typing keyboard" section with its octave stepper; then
//! the "Hints" section with its one checkbox; then the "Audio output" section ([`AudioLayout`]):
//! a row per device it offers ("System default" first), each with a round mark, then the line
//! saying what plays and, when the list failed, why; then the "Audio input" section, laid out
//! as the output's; then the "Input latency" section ([`LatencyLayout`]): the devices a take of
//! the line in uses, the figure kept for them, and a row of buttons (Measure, a millisecond less
//! and more, Clear). The header holds the page tabs, General (all of the above) and Keys
//! ([`crate::ui::keys_page`]); it stays at the window's top while a page scrolls under it.

use iced::{Point, Rectangle, Size};

pub const HEADER_HEIGHT: f32 = 24.0;
/// Space above a section's title.
pub const SECTION_GAP: f32 = 8.0;
pub const SECTION_TITLE_HEIGHT: f32 = 20.0;
pub const ROW_HEIGHT: f32 = 22.0;
/// The left margin of titles and checkboxes.
pub const INSET: f32 = 12.0;
pub const CHECKBOX: f32 = 12.0;
/// Where a row's text starts: after the checkbox and a gap.
pub const LABEL_X: f32 = INSET + CHECKBOX + 8.0;
/// How far right a row's text box reaches.
pub const TEXT_WIDTH: f32 = 360.0;
/// The octave stepper: its label, the two buttons and the octave's name between them.
pub const STEPPER_X: f32 = 110.0;
pub const STEPPER_BUTTON: Size = Size::new(22.0, 18.0);
pub const STEPPER_NAME_WIDTH: f32 = 36.0;

pub const TITLE_SIZE: f32 = 13.0;
pub const TEXT_SIZE: f32 = 12.0;

/// What the panel lists, which decides where its rows are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// MIDI input ports listed.
    pub ports: usize,
    /// Lines of counts and problems under the ports (dropped messages, lost recorded notes,
    /// a port that would not open).
    pub notes: usize,
}

/// A page of the settings panel, chosen by the tabs in its header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsPage {
    /// MIDI inputs, the typing keyboard's octave, the hints, the audio output.
    #[default]
    General,
    /// The shortcuts and the note layout.
    Keys,
}

impl SettingsPage {
    pub const ALL: [SettingsPage; 2] = [SettingsPage::General, SettingsPage::Keys];

    pub fn title(self) -> &'static str {
        match self {
            SettingsPage::General => "General",
            SettingsPage::Keys => "Keys",
        }
    }

    /// Its name on the command line (`--settings-page keys`).
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|page| page.title().eq_ignore_ascii_case(name))
    }
}

/// Where the header's first tab starts, and each tab's width (General, Keys).
const TAB_X: f32 = 90.0;
const TAB_WIDTHS: [f32; 2] = [64.0, 44.0];
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
    /// A page's tab in the header (which [`tab_at`] finds, not [`Layout::hit`]).
    Page(SettingsPage),
    /// The input latency's buttons (which [`LatencyLayout::hit`] finds).
    Measure,
    LatencyDown,
    LatencyUp,
    LatencyClear,
}

impl Layout {
    /// The "MIDI input" section's title.
    pub fn midi_title(&self) -> Rectangle {
        title_at(HEADER_HEIGHT + SECTION_GAP)
    }

    /// Row `row` of the MIDI section: 0 is "All ports", then the ports.
    pub fn row(&self, row: usize) -> Rectangle {
        let top = self.midi_title().y + SECTION_TITLE_HEIGHT + ROW_HEIGHT * row as f32;
        Rectangle::new(
            Point::new(0.0, top),
            Size::new(LABEL_X + TEXT_WIDTH, ROW_HEIGHT),
        )
    }

    /// The checkbox of `row`.
    pub fn checkbox(&self, row: usize) -> Rectangle {
        let rect = self.row(row);
        Rectangle::new(
            Point::new(INSET, rect.y + (ROW_HEIGHT - CHECKBOX) / 2.0),
            Size::new(CHECKBOX, CHECKBOX),
        )
    }

    /// Where `row`'s text sits.
    pub fn label(&self, row: usize) -> Rectangle {
        let rect = self.row(row);
        Rectangle::new(
            Point::new(LABEL_X, rect.y),
            Size::new(TEXT_WIDTH, ROW_HEIGHT),
        )
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
        let after = 1 + self.ports.max(1) + self.notes;
        title_at(self.row(after).y + SECTION_GAP)
    }

    /// The octave stepper's row: its "Octave" label at the row's start.
    pub fn octave_row(&self) -> Rectangle {
        let title = self.typing_title();
        Rectangle::new(
            Point::new(LABEL_X, title.y + SECTION_TITLE_HEIGHT),
            Size::new(TEXT_WIDTH, ROW_HEIGHT),
        )
    }

    pub fn octave_down(&self) -> Rectangle {
        self.stepper_button(STEPPER_X)
    }

    /// Where the octave's name sits, between the buttons.
    pub fn octave_name(&self) -> Rectangle {
        let row = self.octave_row();
        Rectangle::new(
            Point::new(STEPPER_X + STEPPER_BUTTON.width, row.y),
            Size::new(STEPPER_NAME_WIDTH, ROW_HEIGHT),
        )
    }

    pub fn octave_up(&self) -> Rectangle {
        self.stepper_button(STEPPER_X + STEPPER_BUTTON.width + STEPPER_NAME_WIDTH)
    }

    /// The "Hints" section's title, under the typing keyboard's.
    pub fn hints_title(&self) -> Rectangle {
        let row = self.octave_row();
        title_at(row.y + ROW_HEIGHT + SECTION_GAP)
    }

    /// The hint bar's row: its checkbox, then its words.
    pub fn hints_row(&self) -> Rectangle {
        let title = self.hints_title();
        Rectangle::new(
            Point::new(0.0, title.y + SECTION_TITLE_HEIGHT),
            Size::new(LABEL_X + TEXT_WIDTH, ROW_HEIGHT),
        )
    }

    pub fn hints_checkbox(&self) -> Rectangle {
        let row = self.hints_row();
        Rectangle::new(
            Point::new(INSET, row.y + (ROW_HEIGHT - CHECKBOX) / 2.0),
            Size::new(CHECKBOX, CHECKBOX),
        )
    }

    /// The "Audio output" section under the hints, offering `choices` devices.
    pub fn audio(&self, choices: usize) -> AudioLayout {
        AudioLayout {
            top: self.hints_row().y + ROW_HEIGHT + SECTION_GAP,
            choices,
        }
    }

    fn stepper_button(&self, x: f32) -> Rectangle {
        let row = self.octave_row();
        Rectangle::new(
            Point::new(x, row.y + (ROW_HEIGHT - STEPPER_BUTTON.height) / 2.0),
            STEPPER_BUTTON,
        )
    }

    /// What `point` lands on: a row of the MIDI section (its checkbox or its name), or a
    /// stepper button.
    pub fn hit(&self, point: Point) -> Option<Hit> {
        if self.octave_down().contains(point) {
            return Some(Hit::OctaveDown);
        }
        if self.octave_up().contains(point) {
            return Some(Hit::OctaveUp);
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

/// Where the "Audio output" section's parts are. Why a layout of its own beside [`Layout`]:
/// tests build that one field by field.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioLayout {
    /// The section's top: its title's.
    top: f32,
    /// The devices offered, "System default" first.
    pub choices: usize,
}

impl AudioLayout {
    pub fn title(&self) -> Rectangle {
        title_at(self.top)
    }

    /// The row of choice `index`: its mark, then the device's name.
    pub fn row(&self, index: usize) -> Rectangle {
        let top = self.top + SECTION_TITLE_HEIGHT + ROW_HEIGHT * index as f32;
        Rectangle::new(
            Point::new(0.0, top),
            Size::new(LABEL_X + TEXT_WIDTH, ROW_HEIGHT),
        )
    }

    /// The round mark of choice `index`, filled when it is the one chosen.
    pub fn mark(&self, index: usize) -> Rectangle {
        let row = self.row(index);
        Rectangle::new(
            Point::new(INSET, row.y + (ROW_HEIGHT - CHECKBOX) / 2.0),
            Size::new(CHECKBOX, CHECKBOX),
        )
    }

    pub fn label(&self, index: usize) -> Rectangle {
        let row = self.row(index);
        Rectangle::new(
            Point::new(LABEL_X, row.y),
            Size::new(TEXT_WIDTH, ROW_HEIGHT),
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

    /// A section of `choices` laid out as this one, under it and its `lines` lines: the
    /// "Audio input" section under "Audio output".
    pub fn below(&self, lines: usize, choices: usize) -> AudioLayout {
        AudioLayout {
            top: self.line(lines).y + SECTION_GAP,
            choices,
        }
    }

    /// The "Input latency" section under this one and its `lines` lines.
    pub fn latency_below(&self, lines: usize) -> LatencyLayout {
        LatencyLayout {
            top: self.line(lines).y + SECTION_GAP,
        }
    }
}

/// The latency row's buttons: Measure, then a millisecond less and more, then Clear.
const MEASURE_WIDTH: f32 = 72.0;
const CLEAR_WIDTH: f32 = 48.0;
const BUTTON_GAP: f32 = 8.0;

/// Where the "Input latency" section's parts are: its title, the line naming the devices, the
/// line with the figure, and the row of buttons.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatencyLayout {
    top: f32,
}

impl LatencyLayout {
    pub fn title(&self) -> Rectangle {
        title_at(self.top)
    }

    /// Line 0 names the devices, line 1 holds the figure (or "Unknown"), line 2 the buttons.
    pub fn line(&self, line: usize) -> Rectangle {
        Rectangle::new(
            Point::new(
                LABEL_X,
                self.top + SECTION_TITLE_HEIGHT + ROW_HEIGHT * line as f32,
            ),
            Size::new(TEXT_WIDTH, ROW_HEIGHT),
        )
    }

    fn button(&self, x: f32, width: f32) -> Rectangle {
        let row = self.line(2);
        Rectangle::new(
            Point::new(x, row.y + (ROW_HEIGHT - STEPPER_BUTTON.height) / 2.0),
            Size::new(width, STEPPER_BUTTON.height),
        )
    }

    pub fn measure(&self) -> Rectangle {
        self.button(LABEL_X, MEASURE_WIDTH)
    }

    pub fn down(&self) -> Rectangle {
        self.button(LABEL_X + MEASURE_WIDTH + BUTTON_GAP, STEPPER_BUTTON.width)
    }

    pub fn up(&self) -> Rectangle {
        let down = self.down();
        self.button(down.x + down.width, STEPPER_BUTTON.width)
    }

    pub fn clear(&self) -> Rectangle {
        let up = self.up();
        self.button(up.x + up.width + BUTTON_GAP, CLEAR_WIDTH)
    }

    /// Where the section ends: under its row of buttons.
    pub fn bottom(&self) -> f32 {
        self.line(2).y + ROW_HEIGHT
    }

    /// The button a press at `point` lands on.
    pub fn hit(&self, point: Point) -> Option<Hit> {
        [
            (self.measure(), Hit::Measure),
            (self.down(), Hit::LatencyDown),
            (self.up(), Hit::LatencyUp),
            (self.clear(), Hit::LatencyClear),
        ]
        .into_iter()
        .find_map(|(rect, hit)| rect.contains(point).then_some(hit))
    }
}

fn title_at(top: f32) -> Rectangle {
    Rectangle::new(
        Point::new(INSET, top),
        Size::new(TEXT_WIDTH, SECTION_TITLE_HEIGHT),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // With two ports: the title at y 32..52, "All ports" 52..74, the ports 74..96 and
    // 96..118, the typing section's title at 126, its stepper row 146..168.
    #[test]
    fn rows_follow_the_header_title_and_each_other() {
        let layout = Layout { ports: 2, notes: 0 };
        assert_eq!(layout.midi_title().y, 32.0);
        assert_eq!(
            [layout.row(0).y, layout.row(1).y, layout.row(2).y],
            [52.0, 74.0, 96.0]
        );
        assert_eq!(
            layout.checkbox(1),
            Rectangle::new(Point::new(12.0, 79.0), Size::new(12.0, 12.0))
        );
        assert_eq!(layout.empty_message(), None);
        assert_eq!(layout.typing_title().y, 126.0);
        assert_eq!(layout.octave_row().y, 146.0);
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

    // The hints section follows the stepper row (146..168): its title at 176, its checkbox row
    // 196..218, the box centered in it.
    #[test]
    fn the_hints_checkbox_follows_the_typing_keyboard() {
        let layout = Layout { ports: 2, notes: 0 };
        assert_eq!(layout.hints_title().y, 176.0);
        assert_eq!(layout.hints_row().y, 196.0);
        assert_eq!(layout.hints_checkbox().y, 201.0);
        assert_eq!(layout.hit(Point::new(18.0, 207.0)), Some(Hit::ShowHints));
        assert_eq!(layout.hit(Point::new(18.0, 186.0)), None, "the title");
    }

    // B8: the audio section follows the hints row (196..218): its title at 226, its choices'
    // rows from 246, 22 px each, the status line after the last; a press on a row is that
    // choice, one on the title or the status line none.
    #[test]
    fn the_audio_section_follows_the_hints() {
        let audio = Layout { ports: 2, notes: 0 }.audio(3);
        assert_eq!(audio.title().y, 226.0);
        assert_eq!([audio.row(0).y, audio.row(2).y], [246.0, 290.0]);
        assert_eq!(audio.mark(1).y, 273.0);
        assert_eq!(audio.line(0).y, 312.0);
        assert_eq!(audio.hit(Point::new(18.0, 250.0)), Some(0));
        assert_eq!(audio.hit(Point::new(300.0, 300.0)), Some(2));
        assert_eq!(audio.hit(Point::new(18.0, 236.0)), None, "the title");
        assert_eq!(audio.hit(Point::new(18.0, 320.0)), None, "the status line");
    }

    // The latency section follows the input section's last line (its title at 226, two
    // choices from 246, the line after them at 290): its title at 298, the devices' line 318,
    // the figure's 340, the buttons' row 362..384, each button 18 px tall centered in it (y 364);
    // Measure 32..104, − 112..134, + 134..156, Clear 164..212; a press between them is none.
    #[test]
    fn the_latency_section_follows_the_audio_input() {
        let latency = AudioLayout {
            top: 226.0,
            choices: 2,
        }
        .latency_below(0);
        assert_eq!(latency.title().y, 298.0);
        assert_eq!([latency.line(0).y, latency.line(1).y], [318.0, 340.0]);
        assert_eq!(
            latency.measure(),
            Rectangle::new(Point::new(32.0, 364.0), Size::new(72.0, 18.0))
        );
        let xs = (latency.down().x, latency.up().x, latency.clear().x);
        assert_eq!(xs, (112.0, 134.0, 164.0));
        assert_eq!(latency.bottom(), 384.0);
        assert_eq!(latency.hit(Point::new(40.0, 370.0)), Some(Hit::Measure));
        assert_eq!(
            latency.hit(Point::new(120.0, 370.0)),
            Some(Hit::LatencyDown)
        );
        assert_eq!(latency.hit(Point::new(150.0, 370.0)), Some(Hit::LatencyUp));
        assert_eq!(
            latency.hit(Point::new(200.0, 370.0)),
            Some(Hit::LatencyClear)
        );
        assert_eq!(latency.hit(Point::new(108.0, 370.0)), None, "between");
        assert_eq!(latency.hit(Point::new(40.0, 345.0)), None, "the figure");
    }

    // 6d K3: the header's tabs, General then Keys, right of the title, inside the header; a
    // press on the title is no tab.
    #[test]
    fn the_tabs_sit_in_the_header() {
        let general = tab(SettingsPage::General);
        let keys = tab(SettingsPage::Keys);
        assert_eq!((general.x, general.width), (90.0, 64.0));
        assert_eq!((keys.x, keys.width), (158.0, 44.0));
        assert!(keys.y + keys.height <= HEADER_HEIGHT);
        assert_eq!(tab_at(keys.center()), Some(SettingsPage::Keys));
        assert_eq!(tab_at(Point::new(20.0, 12.0)), None);
        assert_eq!(SettingsPage::from_name("keys"), Some(SettingsPage::Keys));
        assert_eq!(SettingsPage::from_name("audio"), None);
    }

    // No port: the empty line takes the first port row's place, and the notes come after it.
    #[test]
    fn with_no_port_the_empty_line_takes_the_first_row() {
        let layout = Layout { ports: 0, notes: 1 };
        assert_eq!(layout.empty_message().map(|rect| rect.y), Some(74.0));
        assert_eq!(layout.note(0).y, 96.0);
        assert_eq!(layout.typing_title().y, 126.0);
        assert_eq!(layout.hit(Point::new(18.0, 80.0)), None, "the empty line");
    }
}
