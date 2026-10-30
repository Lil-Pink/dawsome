//! The plugin picker: a centered overlay listing the catalogue's instruments or effects of both
//! formats (a plugin installed as CLAP and VST3 once, its CLAP build:
//! `Catalogue::offered`), each row naming its format, with a filter box, the files of either
//! format that could not be loaded below them, and Rescan. Choosing a plugin
//! (double-click, or Enter for the selected or first one) adds it: a plugin channel, or the
//! effect in the mixer slot the picker was opened for. Esc closes it.
//!
//! The rack's "+ Instrument…" opens the same picker over every instrument there is, in
//! sections: the instrument sets, the starter kit, the plugin instruments (with the files that
//! could not be loaded, as the plugin picker lists them) and "Sample file…", which asks for a
//! sound file of the user's as "+ Sampler…" did. One filter box filters them all.
//!
//! The list is drawn on a canvas from the constants here, so the screenshot probe can find its
//! rows. Window coordinates of the list's top-left corner: the picker is centered in the window,
//! so ((W − 560)/2 + 14, (H − 420)/2 + 84); at the default 1000 by 560 that is (234, 154).

use std::time::{Duration, Instant};

use dawsome_engine::project::InsertId;
use dawsome_host_clap::catalogue::{FileRecord, Offered, PluginFormat, PluginKind, PluginRecord};
use dawsome_host_clap::Catalogue;
use iced::alignment::Vertical;
use iced::mouse;
use iced::widget::canvas::{self, Frame, Geometry, Text};
use iced::widget::{
    button, canvas as canvas_widget, column, container, opaque, row, space, text, text_input,
};
use iced::{Center, Color, Element, Fill, Point, Rectangle, Renderer, Size, Theme};

use crate::app::Message;
use crate::ui::hint::{hover_area, hover_change, Hover, PickerPart};
use crate::ui::library::{InstrumentSet, Sets};
use crate::ui::style;

pub const PICKER_WIDTH: f32 = 560.0;
pub const PICKER_HEIGHT: f32 = 420.0;
const PADDING: f32 = 14.0;
const SPACING: f32 = 8.0;
const TITLE_HEIGHT: f32 = 24.0;
const FILTER_HEIGHT: f32 = 30.0;
const FOOTER_HEIGHT: f32 = 30.0;
/// The list's top-left corner inside the picker.
#[cfg(test)]
pub const LIST_LEFT: f32 = PADDING;
pub const LIST_TOP: f32 = PADDING + TITLE_HEIGHT + SPACING + FILTER_HEIGHT + SPACING;
pub const LIST_WIDTH: f32 = PICKER_WIDTH - 2.0 * PADDING;
pub const LIST_HEIGHT: f32 = PICKER_HEIGHT - LIST_TOP - SPACING - FOOTER_HEIGHT - PADDING;
pub const ROW_HEIGHT: f32 = 26.0;

// The headless check's five lines (three instruments, the heading, a failed file) fit unscrolled.
const _: () = assert!(LIST_HEIGHT >= 5.0 * ROW_HEIGHT);
/// Where each column's text starts in a row.
const NAME_X: f32 = 10.0;
const VENDOR_X: f32 = 270.0;
pub const FORMAT_X: f32 = 400.0;
pub const KIND_X: f32 = 456.0;
pub const TEXT_SIZE: f32 = 12.0;
/// The two tones plugin rows alternate between: a probe counts them as bands.
pub const ROW_TONES: [Color; 2] = [
    Color::from_rgb8(0x30, 0x34, 0x3A),
    Color::from_rgb8(0x38, 0x3C, 0x43),
];
/// Two presses on one row this close together add its plugin.
const DOUBLE_PRESS: Duration = Duration::from_millis(400);

/// The id the filter box is focused by.
pub const FILTER_INPUT: &str = "plugin-picker-filter";

/// What a chosen plugin becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerTarget {
    /// A new plugin channel at the end of the rack.
    Instrument,
    /// The effect in this empty slot.
    Effect { insert: InsertId, slot: u8 },
    /// "+ Instrument…": a new channel at the end of the rack playing an instrument set, a
    /// starter kit sound, a plugin instrument or a sound file.
    AddInstrument,
}

impl PickerTarget {
    pub fn kind(self) -> PluginKind {
        match self {
            PickerTarget::Instrument | PickerTarget::AddInstrument => PluginKind::Instrument,
            PickerTarget::Effect { .. } => PluginKind::Effect,
        }
    }
}

/// The "+ Instrument…" picker's sections, top to bottom.
pub const SETS_SECTION: &str = "Instrument sets";
pub const KIT_SECTION: &str = "Starter kit";
pub const PLUGINS_SECTION: &str = "Plugins";

/// The row that asks for a sound file of the user's.
pub const SAMPLE_FILE: &str = "Sample file\u{2026}";

/// What the picker reports.
#[derive(Debug, Clone, PartialEq)]
pub enum PickerMessage {
    FilterEdited(String),
    /// Enter in the filter box: add the selected plugin, or the first one listed.
    Submit,
    /// A press on the list, at a point in its own coordinates.
    Pressed(Point),
    Scrolled(f32),
    Rescan,
    Close,
}

/// One drawn line of the list. Lines own their text, so the list does not hold the catalogue
/// borrowed while it is drawn.
#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    Plugin(Offered),
    /// "Could not load (n)".
    Failed(usize),
    /// A file that could not be loaded, as "<file name>: <reason>".
    FailedFile(String),
    /// A section's title over its rows, in the "+ Instrument…" picker.
    Section(&'static str),
    /// An instrument set.
    Set(InstrumentSet),
    /// The starter kit's sound at `index`, called `name`.
    Kit {
        index: usize,
        name: String,
    },
    /// "Sample file…": a sound file the user picks.
    SampleFile,
}

impl Line {
    /// The line's text as the dump lists it: a plugin's, set's or sound's name ("<name> (VST3)"
    /// for a VST3 plugin), a heading or a section's title, or a file's name and why it failed.
    pub fn label(&self) -> String {
        match self {
            Line::Plugin(offered) => match offered.format {
                PluginFormat::Clap => offered.plugin.name.clone(),
                PluginFormat::Vst3 => format!("{} (VST3)", offered.plugin.name),
            },
            Line::Failed(count) => format!("Could not load ({count})"),
            Line::FailedFile(label) => label.clone(),
            Line::Section(title) => (*title).to_owned(),
            Line::Set(set) => set.name.clone(),
            Line::Kit { name, .. } => name.clone(),
            Line::SampleFile => SAMPLE_FILE.to_owned(),
        }
    }

    /// What the hint bar calls a line of its kind.
    pub fn part(&self) -> PickerPart {
        match self {
            Line::Plugin(_) => PickerPart::Plugin,
            Line::Set(_) => PickerPart::Set,
            Line::Kit { .. } => PickerPart::KitSound,
            Line::SampleFile => PickerPart::SampleFile,
            Line::Section(_) => PickerPart::Title,
            Line::Failed(_) | Line::FailedFile(_) => PickerPart::Failed,
        }
    }

    /// Whether a press on it can choose it: a plugin, a set, a kit sound or "Sample file…".
    pub fn choosable(&self) -> bool {
        matches!(
            self,
            Line::Plugin(_) | Line::Set(_) | Line::Kit { .. } | Line::SampleFile
        )
    }
}

fn failed_label(file: &FileRecord) -> String {
    let name = file.file.file_name().map_or_else(
        || file.file.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    match &file.reason {
        Some(reason) => format!("{name}: {reason}"),
        None => name,
    }
}

/// The lines the picker shows for `kind` with `filter` typed: the matching plugins of both
/// formats as the catalogue offers them (by name, then format, then id), then the files of
/// either format that failed, by path, under their heading.
pub fn lines(catalogue: &Catalogue, kind: PluginKind, filter: &str) -> Vec<Line> {
    let filter = filter.trim().to_lowercase();
    let mut lines: Vec<Line> = catalogue
        .offered(kind)
        .into_iter()
        .filter(|offered| plugin_matches(&offered.plugin, &filter))
        .map(Line::Plugin)
        .collect();
    let failed: Vec<&FileRecord> = catalogue.failed_all();
    if !failed.is_empty() {
        lines.push(Line::Failed(failed.len()));
        lines.extend(
            failed
                .into_iter()
                .map(|file| Line::FailedFile(failed_label(file))),
        );
    }
    lines
}

/// What the "+ Instrument…" picker lists with `filter` typed, in sections: the instrument sets
/// (`sets`, shipped then the user's own) whose name or maker matches, the starter kit's sounds
/// (`kit`, by name) that match, then the plugin instruments as [`lines`] lists them (failed files
/// under them, while a plugin matches), each section with its title and only when it has a
/// row; last "Sample file…",
/// always there, so a filter matching nothing still leaves a way to add a sound.
pub fn add_lines(sets: &Sets, kit: &[&str], catalogue: &Catalogue, filter: &str) -> Vec<Line> {
    let query = filter.trim().to_lowercase();
    let mut lines = Vec::new();
    let matching_sets: Vec<Line> = sets
        .shipped
        .iter()
        .chain(&sets.mine)
        .filter(|set| {
            set.name.to_lowercase().contains(&query) || set.maker.to_lowercase().contains(&query)
        })
        .cloned()
        .map(Line::Set)
        .collect();
    let sounds: Vec<Line> = kit
        .iter()
        .enumerate()
        .filter(|(_, name)| name.to_lowercase().contains(&query))
        .map(|(index, name)| Line::Kit {
            index,
            name: (*name).to_owned(),
        })
        .collect();
    // The plugin picker's lines, the failed files under the plugins; with no plugin matching,
    // none of them.
    let mut plugins = self::lines(catalogue, PluginKind::Instrument, filter);
    if !plugins.iter().any(|line| matches!(line, Line::Plugin(_))) {
        plugins.clear();
    }
    for (title, rows) in [
        (SETS_SECTION, matching_sets),
        (KIT_SECTION, sounds),
        (PLUGINS_SECTION, plugins),
    ] {
        if !rows.is_empty() {
            lines.push(Line::Section(title));
            lines.extend(rows);
        }
    }
    lines.push(Line::SampleFile);
    lines
}

/// The lines a press can choose, in order: what the picker's row indices name.
pub fn choosable(lines: &[Line]) -> Vec<&Line> {
    lines.iter().filter(|line| line.choosable()).collect()
}

/// Whether `plugin`'s name or maker holds `lowercase`, a filter already in lower case; every
/// plugin matches an empty one. The picker's filter and the Library's search both go by it.
pub fn plugin_matches(plugin: &PluginRecord, lowercase: &str) -> bool {
    lowercase.is_empty()
        || plugin.name.to_lowercase().contains(lowercase)
        || plugin.vendor.to_lowercase().contains(lowercase)
}

/// The lines' labels, one a line, in the order drawn: the headless dump.
pub fn dump(lines: &[Line]) -> String {
    lines.iter().map(|line| line.label() + "\n").collect()
}

/// The picker between frames.
#[derive(Debug, Clone, PartialEq)]
pub struct Picker {
    pub target: PickerTarget,
    pub filter: String,
    /// The row selected, an index into the lines a press can choose ([`choosable`]).
    pub selected: Option<usize>,
    /// How far the list is scrolled down, in pixels.
    pub scroll: f32,
    last_press: Option<(Instant, usize)>,
}

/// What a press on the list asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Press {
    Select(usize),
    /// A second press on the same row: add its plugin.
    Choose(usize),
}

impl Picker {
    pub fn new(target: PickerTarget) -> Self {
        Self {
            target,
            filter: String::new(),
            selected: None,
            scroll: 0.0,
            last_press: None,
        }
    }

    /// A press at `at` (list coordinates) on `lines` at time `now`, its row counted among the
    /// lines a press can choose; `None` past the list, beside it, or on any other line (a
    /// section's title, a failed file).
    pub fn press_on(&mut self, at: Point, lines: &[Line], now: Instant) -> Option<Press> {
        let row = ((at.y + self.scroll) / ROW_HEIGHT).floor();
        if !(0.0..lines.len() as f32).contains(&row) || !(0.0..LIST_WIDTH).contains(&at.x) {
            return None;
        }
        let row = row as usize;
        if !lines[row].choosable() {
            return None;
        }
        let index = lines[..row].iter().filter(|line| line.choosable()).count();
        Some(self.press_row(index, now))
    }

    /// A press on choosable row `row` at time `now`: it is selected, and a second press on it
    /// soon after chooses it.
    fn press_row(&mut self, row: usize, now: Instant) -> Press {
        let again = self
            .last_press
            .is_some_and(|(when, last)| last == row && now.duration_since(when) < DOUBLE_PRESS);
        self.selected = Some(row);
        if again {
            self.last_press = None;
            Press::Choose(row)
        } else {
            self.last_press = Some((now, row));
            Press::Select(row)
        }
    }

    /// Scrolls by `rise` notches (up is positive), within the `lines` there are.
    pub fn scroll_by(&mut self, rise: f32, lines: usize) {
        let content = lines as f32 * ROW_HEIGHT;
        let max = (content - LIST_HEIGHT).max(0.0);
        self.scroll = (self.scroll - rise * ROW_HEIGHT).clamp(0.0, max);
    }

    /// A new filter: the selection starts over.
    pub fn set_filter(&mut self, filter: String) {
        self.filter = filter;
        self.selected = None;
        self.scroll = 0.0;
    }
}

/// What the hint bar names at list point `at` of `lines` scrolled `scroll` down: the kind of
/// the line under it.
pub fn hover_at(lines: &[Line], scroll: f32, at: Point) -> Option<Hover> {
    let row = ((at.y + scroll) / ROW_HEIGHT).floor();
    if row < 0.0 || !(0.0..LIST_WIDTH).contains(&at.x) {
        return None;
    }
    lines
        .get(row as usize)
        .map(|line| Hover::Picker(line.part()))
}

/// The list canvas.
struct List {
    lines: Vec<Line>,
    selected: Option<usize>,
    scroll: f32,
}

impl canvas::Program<Message> for List {
    /// What the cursor was over last.
    type State = Option<Hover>;

    fn update(
        &self,
        hovered: &mut Option<Hover>,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if let canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) = event {
            let now = cursor
                .position_in(bounds)
                .and_then(|at| hover_at(&self.lines, self.scroll, at));
            return hover_change(hovered, now);
        }
        let at = cursor.position_in(bounds)?;
        let message = match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                PickerMessage::Pressed(at)
            }
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let rise = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y,
                    mouse::ScrollDelta::Pixels { y, .. } => y / ROW_HEIGHT,
                };
                PickerMessage::Scrolled(rise)
            }
            _ => return None,
        };
        Some(canvas::Action::publish(Message::Picker(message)).and_capture())
    }

    fn draw(
        &self,
        _state: &Option<Hover>,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), style::RACK_WELL);
        // The choosable rows alternate tones and are counted as the selection counts them.
        let mut choosable_row = 0;
        for (index, line) in self.lines.iter().enumerate() {
            let top = index as f32 * ROW_HEIGHT - self.scroll;
            let shown = top + ROW_HEIGHT >= 0.0 && top <= bounds.height;
            if shown {
                let middle = top + ROW_HEIGHT / 2.0;
                if line.choosable() {
                    self.draw_band(&mut frame, choosable_row, top, bounds.width);
                }
                for (x, content, color) in columns(line) {
                    label(&mut frame, &content, Point::new(x, middle), color);
                }
            }
            if line.choosable() {
                choosable_row += 1;
            }
        }
        vec![frame.into_geometry()]
    }
}

impl List {
    /// The band under choosable row `row`, whose top is at `top`: its tone, and the accent's
    /// outline while it is selected.
    fn draw_band(&self, frame: &mut Frame, row: usize, top: f32, width: f32) {
        frame.fill_rectangle(
            Point::new(0.0, top),
            Size::new(width, ROW_HEIGHT),
            ROW_TONES[row % 2],
        );
        if self.selected == Some(row) {
            frame.stroke_rectangle(
                Point::new(0.5, top + 0.5),
                Size::new(width - 1.0, ROW_HEIGHT - 1.0),
                canvas::Stroke::default()
                    .with_color(style::ACCENT)
                    .with_width(1.0),
            );
        }
    }
}

/// What a line shows, column by column: where each text starts, the text and its color. A
/// plugin its name, maker, format ("CLAP" or "VST3") and kind; a set its name and maker; a kit
/// sound its name (the
/// section's title says what they are); a section's title in the text color; a heading or a
/// failed file dim.
fn columns(line: &Line) -> Vec<(f32, String, Color)> {
    match line {
        Line::Plugin(offered) => {
            let plugin = &offered.plugin;
            let kind = match plugin.kind {
                PluginKind::Instrument => "Instrument",
                PluginKind::Effect => "Effect",
            };
            vec![
                (NAME_X, plugin.name.clone(), style::TEXT),
                (VENDOR_X, plugin.vendor.clone(), style::TEXT_DIM),
                (
                    FORMAT_X,
                    format_name(offered.format).to_owned(),
                    style::TEXT_DIM,
                ),
                (KIND_X, kind.to_owned(), style::TEXT_DIM),
            ]
        }
        Line::Set(set) => vec![
            (NAME_X, set.name.clone(), style::TEXT),
            (VENDOR_X, set.maker.clone(), style::TEXT_DIM),
        ],
        Line::Kit { name, .. } => vec![(NAME_X, name.clone(), style::TEXT)],
        Line::SampleFile => vec![
            (NAME_X, SAMPLE_FILE.to_owned(), style::TEXT),
            (
                VENDOR_X,
                "a sound file of yours".to_owned(),
                style::TEXT_DIM,
            ),
        ],
        Line::Section(title) => vec![(NAME_X, (*title).to_owned(), style::TEXT)],
        Line::Failed(_) | Line::FailedFile(_) => vec![(NAME_X, line.label(), style::TEXT_DIM)],
    }
}

/// A format's name as the lists write it.
pub fn format_name(format: PluginFormat) -> &'static str {
    match format {
        PluginFormat::Clap => "CLAP",
        PluginFormat::Vst3 => "VST3",
    }
}

fn label(frame: &mut Frame, content: &str, at: Point, color: Color) {
    frame.fill_text(Text {
        content: content.to_owned(),
        position: at,
        color,
        size: TEXT_SIZE.into(),
        align_y: Vertical::Center,
        ..Text::default()
    });
}

fn picker(message: PickerMessage) -> Message {
    Message::Picker(message)
}

/// The picker over the dimmed window, listing `lines`.
pub fn view<'a>(state: &'a Picker, lines: Vec<Line>, scanning: bool) -> Element<'a, Message> {
    let title = match state.target {
        PickerTarget::Instrument => "Add a plugin instrument",
        PickerTarget::Effect { .. } => "Add a plugin effect",
        PickerTarget::AddInstrument => "Add an instrument",
    };
    let hint = match state.target {
        PickerTarget::AddInstrument => "Filter by name or maker",
        PickerTarget::Instrument | PickerTarget::Effect { .. } => "Filter by name or vendor",
    };
    let list = List {
        lines,
        selected: state.selected,
        scroll: state.scroll,
    };
    let control =
        |part: PickerPart, shown: Element<'a, Message>| hover_area(shown, Hover::Picker(part));
    let filter = text_input(hint, &state.filter)
        .id(FILTER_INPUT)
        .on_input(|text| picker(PickerMessage::FilterEdited(text)))
        .on_submit(picker(PickerMessage::Submit))
        .size(13)
        .padding([5, 8]);
    let filter = control(PickerPart::Filter, filter.into());
    let status = if scanning { "Scanning\u{2026}" } else { "" };
    let rescan = button(text("Rescan").size(12))
        .padding([5, 12])
        .style(style::tool_button(false))
        .on_press_maybe((!scanning).then(|| picker(PickerMessage::Rescan)));
    let close = button(text("Close").size(12))
        .padding([5, 12])
        .style(style::tool_button(false))
        .on_press(picker(PickerMessage::Close));
    let footer = row![
        text(status).size(12).color(style::TEXT_DIM),
        space().width(Fill),
        control(PickerPart::Rescan, rescan.into()),
        control(PickerPart::Close, close.into()),
    ]
    .spacing(8)
    .align_y(Center);
    let content = column![
        container(text(title).size(15)).height(TITLE_HEIGHT),
        container(filter).height(FILTER_HEIGHT),
        canvas_widget(list).width(LIST_WIDTH).height(LIST_HEIGHT),
        container(footer).height(FOOTER_HEIGHT),
    ]
    .spacing(SPACING);
    let dialog = container(content)
        .width(PICKER_WIDTH)
        .height(PICKER_HEIGHT)
        .padding(PADDING)
        .style(style::dialog);
    opaque(container(dialog).center(Fill).style(style::scrim))
}

/// The list's top-left corner in window coordinates, for a window of `size`: where the probe
/// looks.
#[cfg(test)]
pub fn list_origin(size: Size) -> Point {
    Point::new(
        (size.width - PICKER_WIDTH) / 2.0 + LIST_LEFT,
        (size.height - PICKER_HEIGHT) / 2.0 + LIST_TOP,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../../../tests/fixtures/p7_catalogue.json");

    fn catalogue() -> Catalogue {
        serde_json::from_str(FIXTURE).expect("parses")
    }

    #[test]
    fn instruments_are_listed_by_name_then_the_failed_files() {
        let catalogue = catalogue();
        let labels: Vec<String> = lines(&catalogue, PluginKind::Instrument, "")
            .iter()
            .map(Line::label)
            .collect();
        assert_eq!(
            labels,
            [
                "Analog Lead",
                "Bell Keys",
                "Dawsome Test Sine",
                "Could not load (1)",
                "broken.clap: crashed: signal 6"
            ]
        );
        let all = lines(&catalogue, PluginKind::Instrument, "");
        assert_eq!(dump(&all), labels.join("\n") + "\n");
        let effects = lines(&catalogue, PluginKind::Effect, "");
        assert_eq!(choosable(&effects).len(), 1);
        assert_eq!(effects[0].label(), "Dawsome Test Gain");
    }

    #[test]
    fn the_filter_matches_names_and_vendors_without_case() {
        let catalogue = catalogue();
        let names = |filter: &str| -> Vec<String> {
            choosable(&lines(&catalogue, PluginKind::Instrument, filter))
                .iter()
                .map(|line| line.label())
                .collect()
        };
        assert_eq!(names("  BELL "), ["Bell Keys"]);
        assert_eq!(names("example"), ["Analog Lead", "Bell Keys"]);
        assert!(names("nothing like it").is_empty());
    }

    #[test]
    fn a_second_press_on_a_row_chooses_it() {
        let mut picker = Picker::new(PickerTarget::Instrument);
        // Three plugins, then the failed files' heading.
        let lines = lines(&catalogue(), PluginKind::Instrument, "");
        let start = Instant::now();
        let row_1 = Point::new(40.0, ROW_HEIGHT * 1.5);
        assert_eq!(
            picker.press_on(row_1, &lines, start),
            Some(Press::Select(1))
        );
        assert_eq!(
            picker.press_on(row_1, &lines, start + Duration::from_millis(100)),
            Some(Press::Choose(1))
        );
        assert_eq!(
            picker.press_on(row_1, &lines, start + Duration::from_secs(2)),
            Some(Press::Select(1)),
            "too slow for a double press"
        );
        let below = Point::new(40.0, ROW_HEIGHT * 3.5);
        assert_eq!(
            picker.press_on(below, &lines, start),
            None,
            "past the plugin rows"
        );
        assert_eq!(picker.selected, Some(1));
    }

    fn set(name: &str, maker: &str) -> InstrumentSet {
        InstrumentSet {
            name: name.to_owned(),
            maker: maker.to_owned(),
            sfz: format!("{name}.sfz").into(),
            bytes: None,
        }
    }

    // "+ Instrument…" lists, in sections with their titles, the sets (shipped, then the user's
    // own), the kit's sounds and the plugin instruments with the failed files, then "Sample
    // file…"; a filter keeps the matching rows of each and drops a section left empty, never
    // "Sample file…".
    #[test]
    fn the_instrument_picker_lists_sets_kit_plugins_and_a_sample_file() {
        let catalogue = catalogue();
        let sets = Sets {
            shipped: vec![set("Upright Piano", "Dawsome"), set("Bass 808", "Dawsome")],
            mine: vec![set("My Keys", "")],
        };
        let kit = ["Kick 808", "Snare", "Clap"];
        let labels = |filter: &str| -> Vec<String> {
            add_lines(&sets, &kit, &catalogue, filter)
                .iter()
                .map(Line::label)
                .collect()
        };
        assert_eq!(
            labels(""),
            [
                "Instrument sets",
                "Upright Piano",
                "Bass 808",
                "My Keys",
                "Starter kit",
                "Kick 808",
                "Snare",
                "Clap",
                "Plugins",
                "Analog Lead",
                "Bell Keys",
                "Dawsome Test Sine",
                "Could not load (1)",
                "broken.clap: crashed: signal 6",
                "Sample file\u{2026}"
            ]
        );
        assert_eq!(
            labels("  KEY"),
            [
                "Instrument sets",
                "My Keys",
                "Plugins",
                "Bell Keys",
                "Could not load (1)",
                "broken.clap: crashed: signal 6",
                "Sample file\u{2026}"
            ]
        );
        assert_eq!(
            labels("sn"),
            ["Starter kit", "Snare", "Sample file\u{2026}"]
        );
        assert_eq!(
            labels("dawsome").first().map(String::as_str),
            Some("Instrument sets"),
            "sets match by maker"
        );
    }

    // In the sectioned list a press counts only the rows it can choose: the sets' title is
    // none, the first set is row 0, the kit's first sound after two sets and a title is row 2;
    // a second press chooses; the failed files and titles take no press.
    #[test]
    fn a_press_on_the_sectioned_list_counts_the_rows_it_can_choose() {
        let sets = Sets {
            shipped: vec![set("A", ""), set("B", "")],
            mine: Vec::new(),
        };
        let lines = add_lines(&sets, &["Kick"], &catalogue(), "");
        let mut picker = Picker::new(PickerTarget::AddInstrument);
        let start = Instant::now();
        let on = |row: f32| Point::new(40.0, ROW_HEIGHT * (row + 0.5));
        assert_eq!(picker.press_on(on(0.0), &lines, start), None, "a title");
        assert_eq!(
            picker.press_on(on(1.0), &lines, start),
            Some(Press::Select(0))
        );
        assert_eq!(
            picker.press_on(on(4.0), &lines, start),
            Some(Press::Select(2)),
            "Kick, after two sets and the kit's title"
        );
        assert_eq!(
            picker.press_on(on(4.0), &lines, start + Duration::from_millis(100)),
            Some(Press::Choose(2))
        );
        let failed = lines
            .iter()
            .position(|line| matches!(line, Line::FailedFile(_)))
            .expect("a failed file") as f32;
        assert_eq!(picker.press_on(on(failed), &lines, start), None);
        let chosen = choosable(&lines);
        assert_eq!(chosen[2].label(), "Kick");
        assert_eq!(
            chosen.last().map(|line| line.label()),
            Some(SAMPLE_FILE.to_owned())
        );
        // The plugin picker's rows press as before: its plugins come first.
        let plugin_lines = lines_for_plugins();
        let mut plugins = Picker::new(PickerTarget::Instrument);
        assert_eq!(
            plugins.press_on(on(1.0), &plugin_lines, start),
            Some(Press::Select(1))
        );
        assert_eq!(plugins.press_on(on(3.0), &plugin_lines, start), None);
    }

    fn lines_for_plugins() -> Vec<Line> {
        lines(&catalogue(), PluginKind::Instrument, "")
    }

    #[test]
    fn the_list_sits_where_the_probe_looks() {
        assert_eq!(
            list_origin(Size::new(1000.0, 560.0)),
            Point::new(234.0, 154.0)
        );
        assert_eq!(LIST_WIDTH, 532.0);
    }
}
