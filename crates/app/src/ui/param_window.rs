//! The parameter window: one plugin's parameters as a list (each one's name, a slider, the
//! plugin's own text for its value), in a window of its own beside the main one. It opens from a
//! plugin channel's menu (Parameters…), a mixer slot's menu, for a plugin with no interface of its
//! own, and when plugin windows cannot open. It looks as the instrument window does: the place
//! and the plugin over the list, the plugin's own groups titled as the instrument window's MIDI
//! OUT group is, a footer with Close, and a foot line. Hidden parameters are left out (a VST3
//! plugin's that its MIDI mapping names too); read-only ones are shown dimmed and do not move. A
//! plugin that delays its sound says so in the footer.
//!
//! The list is iced's own scrollable, so the wheel and its scroller move it as any list's; it
//! builds only the rows near the view, spacers standing for the rest, so 775 rows scroll as
//! lightly as 2. Why the sliders are boxes under a mouse area rather than iced's slider: that one
//! jumps to the press, has no fine drag and resets on Ctrl+click. A press here starts the app's
//! own drag (the whole range over [`SLIDER_WIDTH`] pixels, Shift for fine, one undo step when
//! let go of), and a second press within [`DOUBLE_PRESS`] sets the default.

use std::ops::Range;
use std::time::{Duration, Instant};

use dawsome_engine::processor::{PluginParam, ProcessorHost};
use dawsome_engine::project::ProcessorOwner;
use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{
    button, column, container, keyed_column, mouse_area, row, scrollable, space, stack, text,
};
use iced::{Center, Color, Element, Fill, Padding, Point, Size, Theme};

use crate::app::Message;
use crate::ui::instrument_window::{HINT_HEIGHT, HINT_SIZE, MIDI_OUT_RULE, PADDING};
use crate::ui::style;
use crate::ui::text_fit;

/// The window's first size (the list shows about 16 rows), and the least it can be made.
pub const WINDOW_SIZE: Size = Size::new(560.0, 480.0);
pub const WINDOW_MIN: Size = Size::new(420.0, 240.0);

/// The id the list's scrollable goes by, so it can be scrolled from the app.
pub const LIST: &str = "param-list";
/// Every item of the list, a row or a group's title, is this tall.
pub const ROW_HEIGHT: f32 = 24.0;
/// The names' column: a longer name is cut with an ellipsis.
pub const NAME_WIDTH: f32 = 200.0;
/// A slider's width, and the pixels of drag that sweep its whole range (gen_p28's drag; the
/// mixer slot panel's sliders are as wide).
pub const SLIDER_WIDTH: f32 = 120.0;
pub const SLIDER_HEIGHT: f32 = 12.0;
/// Between the name, the slider and the value.
pub const GAP: f32 = 10.0;
/// Where the sliders start across the window.
pub const SLIDER_X: f32 = PADDING + NAME_WIDTH + GAP;
const TEXT_SIZE: f32 = 12.0;
/// A group's title, as the instrument window's MIDI OUT title.
const GROUP_SIZE: f32 = 10.0;
/// The heading: the place as the instrument window's channel name, the plugin beside it dim;
/// its band's height, the list starting under it.
const PLACE_SIZE: f32 = 15.0;
const ABOUT_SIZE: f32 = 11.0;
pub const HEADING_HEIGHT: f32 = 40.0;
/// The list's scroller, as wide as the in-window panel's was.
const SCROLLER: f32 = 6.0;
/// Rows built past the view at each end, so a fast wheel never shows an empty strip before the
/// list catches up with it.
const SPARE_ROWS: usize = 8;
/// Two presses on one slider this close together set it to its default.
const DOUBLE_PRESS: Duration = Duration::from_millis(400);
/// The title of a run of parameters in no group of the plugin's, after one in a group.
const OTHER: &str = "Other";

/// What a plugin's latency is said as: "delays its sound by N frames (M ms); not compensated",
/// M in tenths of a millisecond, rounded half up in integers ((20000·N + rate) div (2·rate)).
pub fn latency_text(frames: u32, rate: u32) -> String {
    let rate = u64::from(rate.max(1));
    let tenths = (20_000 * u64::from(frames) + rate) / (2 * rate);
    format!(
        "delays its sound by {frames} frames ({}.{} ms); not compensated",
        tenths / 10,
        tenths % 10
    )
}

/// What the window reports.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamMessage {
    /// A left press on the slider of row `index` (among the parameters shown).
    Pressed(usize),
    /// A right press on row `index`: the automation menu of its parameter.
    RightPressed(usize),
    /// The list scrolled: its rows are this many pixels up.
    Scrolled(f32),
    /// Its Close.
    Close,
}

/// A press on a slider: row `index`'s; `double` for a second press there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SliderPress {
    pub index: usize,
    pub double: bool,
}

/// An item of the list, one [`ROW_HEIGHT`] tall.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// The title over a run of one of the plugin's groups: its path's parts a slash apart.
    Group(String),
    /// The row of parameter `index` (among the parameters shown).
    Row(usize),
}

/// The list's items for `params`, in the plugin's order: a title where a run of one of the
/// plugin's groups begins (its path's parts a slash apart: "Filter / Envelope" for
/// "Filter/Envelope", "Macros" for Surge XT's "/Macros/"), and [`OTHER`] over a run in none after
/// one in a group; the parameters before any group's, and all of a plugin that groups none, under
/// no title.
pub fn items(params: &[PluginParam]) -> Vec<Item> {
    let mut items = Vec::with_capacity(params.len());
    let mut run: Option<&str> = None;
    let mut grouped = false;
    for (index, param) in params.iter().enumerate() {
        let group = param.group.as_str();
        if run != Some(group) {
            let parts: Vec<&str> = group
                .split('/')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .collect();
            if !parts.is_empty() {
                items.push(Item::Group(parts.join(" / ")));
                grouped = true;
            } else if grouped {
                items.push(Item::Group(OTHER.to_owned()));
            }
            run = Some(group);
        }
        items.push(Item::Row(index));
    }
    items
}

/// The items built of `count`, with the list scrolled `scroll` pixels up in a window `height`
/// tall: those in view and [`SPARE_ROWS`] more at each end. The window's height stands for the
/// list's, which is less, so the view is always covered.
pub fn shown(count: usize, scroll: f32, height: f32) -> Range<usize> {
    let first = (scroll.max(0.0) / ROW_HEIGHT).floor() as usize;
    let last = ((scroll.max(0.0) + height.max(0.0)) / ROW_HEIGHT).ceil() as usize;
    let end = last.saturating_add(SPARE_ROWS).min(count);
    first.saturating_sub(SPARE_ROWS).min(end)..end
}

/// What heads the window.
#[derive(Debug, Clone, PartialEq)]
pub struct Heading {
    /// The window's title, the one the plugin's own window carries: "<channel or slot> —
    /// <plugin>".
    pub title: String,
    /// The channel's name, or the slot's place ("Insert 1 slot 2").
    pub place: String,
    /// What the plugin is: "<plugin> · by <maker>".
    pub about: String,
}

/// One plugin's parameters as the window shows them.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamPanel {
    pub owner: ProcessorOwner,
    /// "<channel or slot> — <plugin>": the window's title.
    pub title: String,
    /// The heading's two parts: the place, and what the plugin is.
    pub place: String,
    pub about: String,
    /// The parameters shown: every one that is not hidden, in the plugin's order.
    pub params: Vec<PluginParam>,
    /// Each one's value and text, as last read.
    pub values: Vec<f64>,
    pub texts: Vec<String>,
    /// Each one's name as its row shows it: cut with an ellipsis to [`NAME_WIDTH`].
    pub labels: Vec<String>,
    /// The list: the rows and their groups' titles.
    pub items: Vec<Item>,
    /// How far the list is scrolled up, in pixels, as it last said.
    pub scroll: f32,
    /// The line in its footer: how late the plugin says its sound comes, while it says it does.
    pub foot: Option<String>,
    /// The rate the plugin plays at, which its latency is said in milliseconds at.
    pub rate: u32,
    last_press: Option<(Instant, usize)>,
}

impl ParamPanel {
    /// The list for `owner`'s plugin, headed by `heading`, its values read from `host`; the
    /// plugin plays at `rate`.
    pub fn new(
        owner: ProcessorOwner,
        heading: Heading,
        host: &dyn ProcessorHost,
        rate: u32,
    ) -> Self {
        let params: Vec<PluginParam> = host
            .params(owner)
            .into_iter()
            .filter(|param| !param.hidden)
            .collect();
        let labels = params
            .iter()
            .map(|param| {
                text_fit::fit_width(&param.name, NAME_WIDTH, TEXT_SIZE)
                    .map(std::borrow::Cow::into_owned)
                    .unwrap_or_default()
            })
            .collect();
        let mut panel = Self {
            owner,
            title: heading.title,
            place: heading.place,
            about: heading.about,
            values: params.iter().map(|param| param.default).collect(),
            texts: vec![String::new(); params.len()],
            labels,
            items: items(&params),
            params,
            scroll: 0.0,
            foot: None,
            rate,
            last_press: None,
        };
        panel.refresh(host);
        panel
    }

    /// Reads every value and its text again, and the plugin's latency.
    pub fn refresh(&mut self, host: &dyn ProcessorHost) {
        self.refresh_items(host, 0..self.items.len());
    }

    /// Reads the plugin's latency again, and the values and texts of the rows among `items`
    /// (positions in [`ParamPanel::items`]): the window's timer reads those it builds, the rest
    /// as they come into view. Why not all: at 30 reads a second, Surge XT's 775 texts cost the
    /// window more than everything it draws.
    pub fn refresh_items(&mut self, host: &dyn ProcessorHost, items: Range<usize>) {
        self.foot = host
            .latency(self.owner)
            .filter(|&frames| frames > 0)
            .map(|frames| capitalized(&latency_text(frames, self.rate)));
        let end = items.end.min(self.items.len());
        for item in &self.items[items.start.min(end)..end] {
            let &Item::Row(index) = item else {
                continue;
            };
            let (Some(param), Some(value), Some(text)) = (
                self.params.get(index),
                self.values.get_mut(index),
                self.texts.get_mut(index),
            ) else {
                continue;
            };
            if let Some(now) = host.value(self.owner, param.id) {
                *value = now;
            }
            *text = host
                .value_text(self.owner, param.id, *value)
                .unwrap_or_else(|| format!("{value:.3}"));
        }
    }

    /// The items the window builds when `height` tall, as [`shown`] gives them.
    pub fn shown(&self, height: f32) -> Range<usize> {
        shown(self.items.len(), self.scroll, height)
    }

    /// A press on row `index`'s slider at `now`; `None` for a read-only parameter or a row that
    /// is not there.
    pub fn press(&mut self, index: usize, now: Instant) -> Option<SliderPress> {
        let param = self.params.get(index)?;
        if param.read_only {
            return None;
        }
        let double = self
            .last_press
            .is_some_and(|(when, last)| last == index && now.duration_since(when) < DOUBLE_PRESS);
        self.last_press = (!double).then_some((now, index));
        Some(SliderPress { index, double })
    }

    /// The list as a scripted run writes it (`--dump-params`): the window's title, then a line
    /// per row, its name, a tab and the plugin's text for its value.
    pub fn dump(&self) -> String {
        let mut dump = format!("{}\n", self.title);
        for (param, text) in self.params.iter().zip(&self.texts) {
            dump.push_str(&format!("{}\t{text}\n", param.name));
        }
        dump
    }

    /// Where row `index`'s automation menu opens: under its slider's left end, in the list's
    /// coordinates.
    pub fn menu_at(&self, index: usize) -> Option<Point> {
        let item = self
            .items
            .iter()
            .position(|item| *item == Item::Row(index))?;
        Some(Point::new(SLIDER_X, (item + 1) as f32 * ROW_HEIGHT))
    }
}

/// `text` with its first letter a capital.
fn capitalized(text: &str) -> String {
    let mut letters = text.chars();
    letters
        .next()
        .map(|first| first.to_uppercase().chain(letters).collect())
        .unwrap_or_default()
}

/// What the window shows.
pub struct View<'a> {
    pub panel: &'a ParamPanel,
    /// The automation menu of a row, while one is open (pinned in the list's coordinates).
    pub menu: Option<Element<'a, Message>>,
    /// The window's size, as it last said: the list builds the rows its height shows, and the
    /// foot line is cut to its width.
    pub size: Size,
    /// The foot line.
    pub hint: String,
}

/// The window's content, filling its window: the place and the plugin over the list, which
/// scrolls; the footer with the plugin's latency and Close; the foot line.
pub fn view(shown: View<'_>) -> Element<'_, Message> {
    let View {
        panel,
        menu,
        size,
        hint,
    } = shown;
    // On one line, cut by the window's edge in a window too narrow for both.
    let heading = row![
        text(panel.place.clone())
            .size(PLACE_SIZE)
            .wrapping(text::Wrapping::None),
        text(panel.about.clone())
            .size(ABOUT_SIZE)
            .color(style::TEXT_DIM)
            .wrapping(text::Wrapping::None),
    ]
    .spacing(10)
    .align_y(Center);
    let heading = container(heading)
        .padding([0.0, PADDING])
        .width(Fill)
        .height(HEADING_HEIGHT)
        .align_y(Center)
        .clip(true);
    let body: Element<'_, Message> = if panel.params.is_empty() {
        container(
            text("This plugin shows no parameters.")
                .size(TEXT_SIZE)
                .color(style::TEXT_DIM),
        )
        .padding([0.0, PADDING])
        .height(Fill)
        .into()
    } else {
        let mut layers = vec![list(panel, size.height)];
        layers.extend(menu);
        scrollable(stack(layers))
            .id(LIST)
            .on_scroll(|viewport| {
                Message::Params(ParamMessage::Scrolled(viewport.absolute_offset().y))
            })
            .direction(Direction::Vertical(
                Scrollbar::new().width(SCROLLER).scroller_width(SCROLLER),
            ))
            .width(Fill)
            .height(Fill)
            .into()
    };
    let room = (size.width - 2.0 * PADDING).max(0.0);
    let foot_line = text_fit::fit_width(&hint, room, HINT_SIZE)
        .map(std::borrow::Cow::into_owned)
        .unwrap_or_default();
    let foot_line = container(
        text(foot_line)
            .size(HINT_SIZE)
            .color(style::TEXT_DIM)
            .wrapping(text::Wrapping::None),
    )
    .padding([0.0, PADDING])
    .width(Fill)
    .height(HINT_HEIGHT)
    .align_y(Center)
    .style(style::window);
    container(column![heading, body, footer(panel), foot_line])
        .width(Fill)
        .height(Fill)
        .style(style::panel)
        .into()
}

/// The list's items near the view of a window `height` tall, spacers above and below standing
/// for the rest, so it is as tall as all of them.
fn list(panel: &ParamPanel, height: f32) -> Element<'_, Message> {
    let count = panel.items.len();
    let range = panel.shown(height);
    let built = panel.items[range.clone()]
        .iter()
        .enumerate()
        .map(|(offset, item)| (range.start + offset, item_view(panel, item)));
    let above = range.start as f32 * ROW_HEIGHT;
    let below = (count - range.end) as f32 * ROW_HEIGHT;
    container(column![
        space().height(above),
        keyed_column(built),
        space().height(below)
    ])
    .padding(Padding::ZERO.left(PADDING).right(PADDING + SCROLLER))
    .width(Fill)
    .into()
}

fn item_view<'a>(panel: &'a ParamPanel, item: &Item) -> Element<'a, Message> {
    match item {
        Item::Group(group) => group_title(group),
        Item::Row(index) => param_row(panel, *index),
    }
}

/// A group's title over its rows, as the instrument window titles its MIDI OUT group: the
/// group's name small, in capitals and dim, an accent rule after it.
fn group_title(group: &str) -> Element<'static, Message> {
    let words = text(group.to_uppercase())
        .size(GROUP_SIZE)
        .color(style::TEXT_DIM)
        .wrapping(text::Wrapping::None);
    let rule = container(space())
        .width(Fill)
        .height(MIDI_OUT_RULE)
        .style(ground(style::ACCENT));
    container(row![words, rule].spacing(6).align_y(Center))
        .height(ROW_HEIGHT)
        .align_y(Center)
        .into()
}

/// Row `index`: the name, the slider (its value's place along the range filled), the plugin's
/// text for the value; a press on the slider drags it, a right press on the row opens its
/// automation menu. A read-only one is dim and takes neither.
fn param_row(panel: &ParamPanel, index: usize) -> Element<'_, Message> {
    let param = &panel.params[index];
    let color = if param.read_only {
        style::TEXT_DIM
    } else {
        style::TEXT
    };
    let words = |content: &str| {
        text(content.to_owned())
            .size(TEXT_SIZE)
            .color(color)
            .wrapping(text::Wrapping::None)
    };
    let label = panel.labels.get(index).map_or("", String::as_str);
    let name = container(words(label)).width(NAME_WIDTH);
    let value = panel.values.get(index).copied().unwrap_or(param.default);
    let span = param.max - param.min;
    let fraction = if span > 0.0 {
        ((value - param.min) / span).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    let fill = if param.read_only {
        style::BUTTON
    } else {
        style::SLIDER_FILL
    };
    let filled = container(space())
        .width(SLIDER_WIDTH * fraction)
        .height(Fill)
        .style(ground(fill));
    let track = container(filled)
        .width(SLIDER_WIDTH)
        .height(SLIDER_HEIGHT)
        .style(ground(style::FADER_TRACK));
    // The row's height is the slider's to press, as the in-window panel's reach around it was.
    let slider = container(track).height(ROW_HEIGHT).align_y(Center);
    let shown = panel.texts.get(index).map_or("", String::as_str);
    let value = container(words(shown)).width(Fill).clip(true);
    if param.read_only {
        return row![name, slider, value]
            .spacing(GAP)
            .height(ROW_HEIGHT)
            .align_y(Center)
            .into();
    }
    let slider = mouse_area(slider).on_press(Message::Params(ParamMessage::Pressed(index)));
    let line = row![name, slider, value]
        .spacing(GAP)
        .height(ROW_HEIGHT)
        .align_y(Center);
    mouse_area(line)
        .on_right_press(Message::Params(ParamMessage::RightPressed(index)))
        .into()
}

/// The footer: the plugin's latency (while it says it has one), then Close at the right, as the
/// instrument window's footer ends in its Close.
fn footer(panel: &ParamPanel) -> Element<'_, Message> {
    let latency = panel.foot.as_deref().unwrap_or_default();
    let close = button(text("Close").size(12))
        .padding([2, 8])
        .style(style::tool_button(false))
        .on_press(Message::Params(ParamMessage::Close));
    container(
        row![
            text(latency.to_owned())
                .size(ABOUT_SIZE)
                .color(style::TEXT_DIM)
                .wrapping(text::Wrapping::None),
            space().width(Fill),
            close
        ]
        .spacing(4)
        .align_y(Center),
    )
    .padding([6.0, PADDING])
    .width(Fill)
    .into()
}

/// A plain box of `color`.
fn ground(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_theme| container::Style {
        background: Some(color.into()),
        ..container::Style::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn param(id: u32, group: &str, read_only: bool) -> PluginParam {
        PluginParam {
            id,
            name: format!("P{id}"),
            group: group.to_owned(),
            min: 0.0,
            max: 1.0,
            default: 0.5,
            stepped: false,
            hidden: false,
            read_only,
        }
    }

    fn panel(params: Vec<PluginParam>) -> ParamPanel {
        let count = params.len();
        ParamPanel {
            owner: ProcessorOwner::Channel(dawsome_engine::project::ChannelId(1)),
            title: "Lead \u{2014} Synth".into(),
            place: "Lead".into(),
            about: "Synth".into(),
            values: vec![0.5; count],
            texts: vec![String::new(); count],
            labels: params.iter().map(|param| param.name.clone()).collect(),
            items: items(&params),
            params,
            scroll: 0.0,
            foot: None,
            rate: 48_000,
            last_press: None,
        }
    }

    // gen_p28_expected.py's latency texts: the milliseconds in tenths, rounded half up in
    // integers (12 frames at 48 kHz are 0.25 ms: "0.3 ms"; 441,000 at 44.1 kHz are 10 s).
    #[test]
    fn a_latency_is_said_in_frames_and_tenths_of_a_millisecond() {
        let expected: serde_json::Value =
            serde_json::from_str(include_str!("../../../../tests/fixtures/p28_expected.json"))
                .expect("p28_expected.json parses");
        let cases = expected["latency"]["texts"].as_array().expect("cases");
        assert!(!cases.is_empty());
        for case in cases {
            let frames = case[0].as_u64().expect("frames") as u32;
            let rate = case[1].as_u64().expect("rate") as u32;
            assert_eq!(
                latency_text(frames, rate),
                case[2].as_str().expect("a text")
            );
        }
        assert_eq!(
            capitalized(&latency_text(64, 44_100)),
            expected["latency"]["foot_before"].as_str().expect("a text")
        );
    }

    // The in-window panel's press test, by row now that each slider is its own widget: a press
    // on row 0's slider is one, a second within 400 ms a double press; a read-only row's slider
    // takes none, nor does a row that is not there. (Its Close is a button of its own now:
    // param_window's app tests close the window through it.)
    #[test]
    fn presses_find_the_sliders() {
        let mut panel = panel(vec![param(7, "", false), param(8, "", true)]);
        let now = Instant::now();
        assert_eq!(
            panel.press(0, now),
            Some(SliderPress {
                index: 0,
                double: false
            })
        );
        assert_eq!(
            panel.press(0, now + Duration::from_millis(100)),
            Some(SliderPress {
                index: 0,
                double: true
            })
        );
        assert_eq!(panel.press(1, now), None, "read-only");
        assert_eq!(panel.press(2, now), None, "no row 2");
        // After a double press the next is a first press again, and a press too late is too.
        assert_eq!(
            panel.press(0, now + Duration::from_millis(200)),
            Some(SliderPress {
                index: 0,
                double: false
            })
        );
        assert_eq!(
            panel.press(0, now + Duration::from_millis(700)),
            Some(SliderPress {
                index: 0,
                double: false
            })
        );
    }

    // By hand, 24 px items: 40 of them in a 300 px window from the top show 0 to 12.5 (13
    // items) and 8 more below, 0..21; scrolled 345 px up, items 14.375 to 26.875 show, so 6..35
    // are built (8 spare each side) with 144 px above and 120 px below; scrolled to the last
    // item's foot (960 − 300 = 690), 28.75 to 41.25, so 20..40, the last item built. Five items
    // in the window are all built.
    #[test]
    fn the_list_builds_the_items_near_the_view() {
        assert_eq!(shown(40, 0.0, 300.0), 0..21);
        assert_eq!(shown(40, 345.0, 300.0), 6..35);
        assert_eq!(shown(40, 690.0, 300.0), 20..40);
        assert_eq!(shown(5, 0.0, 300.0), 0..5);
        assert_eq!(shown(5, 900.0, 300.0), 5..5, "scrolled past the end");
        assert_eq!(shown(0, 0.0, 300.0), 0..0);
    }

    // A plugin's groups title their runs, their paths' parts a slash apart (Surge XT's CLAP
    // build names its first "/Macros/"); parameters before any group's have no title, and a run
    // in no group after a grouped one is "Other". A plugin that groups none is rows alone.
    // Row 3 is the sixth item (two titles before it): its menu opens under its slider, 6 × 24 px
    // down.
    #[test]
    fn the_plugins_groups_title_their_rows() {
        let grouped = [
            param(0, "", false),
            param(1, "Osc/1", false),
            param(2, "Osc/1", false),
            param(3, "/Filter/", false),
            param(4, "", false),
        ];
        assert_eq!(
            items(&grouped),
            [
                Item::Row(0),
                Item::Group("Osc / 1".into()),
                Item::Row(1),
                Item::Row(2),
                Item::Group("Filter".into()),
                Item::Row(3),
                Item::Group("Other".into()),
                Item::Row(4),
            ]
        );
        let plain = [param(0, "", false), param(1, "", true)];
        assert_eq!(items(&plain), [Item::Row(0), Item::Row(1)]);
        let panel = panel(grouped.to_vec());
        assert_eq!(
            panel.menu_at(3),
            Some(Point::new(12.0 + 200.0 + 10.0, 6.0 * ROW_HEIGHT))
        );
        assert_eq!(panel.menu_at(9), None);
    }
}
