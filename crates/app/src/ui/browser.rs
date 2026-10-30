//! The Library panel, a column down the window's left: a header, the search box in its well, and
//! the Library's rows (see [`crate::ui::library`]): three sections, each a header with a count
//! over its rows, the sample folders' tree among them. One canvas draws the rows and reports
//! presses on them; while a press is held it reports every move and the release wherever the
//! cursor is, so a sound, a plugin or a MIDI device can be dragged onto the other panels. Over a
//! row that can be dragged, a tip under the cursor says where it goes. The menu under a row
//! (Remove, Unlink) is a [`crate::ui::menu`] menu on the window's top layer.

use std::path::Path;

use dawsome_host_clap::catalogue::{Offered, PluginFormat};
use iced::alignment::Vertical;
use iced::widget::canvas::{self, Cache, Event, Frame, Geometry, Path as Shape, Stroke, Text};
use iced::widget::{column, container, pin, stack, text_input, Id};
use iced::{mouse, Color, Element, Fill, Point, Rectangle, Renderer, Size, Theme};

use crate::app::Message;
use crate::sets::size_text;
use crate::starter_kit;
use crate::ui::browser_model::{Browser, Playing, Row, RowKind};
use crate::ui::hint::{hover_change, Hover, LibraryPart};
use crate::ui::library::{InstrumentSet, Item, Port, Section, EXTERNAL_INPUT};
use crate::ui::menu;
use crate::ui::style;
use crate::ui::text_fit::{fit_width, text_width};

/// The panel's width.
pub const WIDTH: f32 = 300.0;
/// The header strip, from the panel's top.
pub const HEADER_HEIGHT: f32 = 24.0;
/// The search box's well.
pub const WELL: Rectangle = Rectangle {
    x: 6.0,
    y: 26.0,
    width: 288.0,
    height: 20.0,
};
/// Where the first row starts, and each row's height.
pub const ROWS_TOP: f32 = 52.0;
pub const ROW_HEIGHT: f32 = 22.0;
/// How far each level is indented, and where level 0's glyph starts.
pub const INDENT: f32 = 14.0;
pub const ROW_LEFT: f32 = 8.0;
/// From a row's glyph to its name.
const NAME_OFFSET: f32 = 12.0;
const NAME_SIZE: f32 = 12.0;
/// The size of the dim labels: group names, makers and hints.
const LABEL_SIZE: f32 = 11.0;
/// An average glyph of the name font, to cut names that would run past the panel.
const CHAR_WIDTH: f32 = 6.6;
/// The previewing row's left bar.
const PREVIEW_BAR: f32 = 2.0;
/// The mark at the right end of the instrument row the Library previews, which its keys play.
pub const PLAYING_MARK: &str = "playing";
/// The menu under a row (Remove under a folder of the user's, Unlink under a linked device):
/// its rows' width, and where it opens, this far in from the panel's left edge.
pub const MENU_WIDTH: f32 = 96.0;
const MENU_X: f32 = 40.0;
/// Rows the wheel scrolls per notch.
const ROWS_PER_NOTCH: i64 = 3;
/// A MIDI device's tick, and where its glyph and name start.
pub const TICK_SIZE: f32 = 12.0;
const PORT_GLYPH_X: f32 = ROW_LEFT + TICK_SIZE + 6.0;
pub const PORT_NAME_X: f32 = PORT_GLYPH_X + 14.0;
/// Space kept clear at a row's right end.
const RIGHT_MARGIN: f32 = 8.0;
/// Space inside the Rescan button, left and right of its words.
const BUTTON_PADDING: f32 = 8.0;
/// The tip under the cursor: its height, and how far below the cursor it starts.
const TIP_HEIGHT: f32 = 18.0;
const TIP_DROP: f32 = 16.0;

/// What a drag of a sound or a plugin does, and of a MIDI device.
pub const SOUND_TIP: &str = "Drag onto the rack or a track";
pub const DEVICE_TIP: &str = "Drag onto a channel in the rack";

/// The search box.
pub const SEARCH_INPUT: &str = "browser-search";

/// What the panel reports.
#[derive(Debug, Clone, PartialEq)]
pub enum BrowserMessage {
    /// A press on the Samples tree's row `row` (an index into the tree's rows, as
    /// [`Browser::rows`] gives them), the cursor at window point `at`.
    Pressed {
        row: usize,
        button: mouse::Button,
        at: Point,
    },
    /// A press on a Library row outside the Samples tree, the cursor at window point `at`.
    Picked {
        pick: Pick,
        button: mouse::Button,
        at: Point,
    },
    /// A press on a section's header: the section opens or closes.
    Section(Section),
    /// The cursor moved to window point `at` while a press is held.
    Moved(Point),
    /// The held press was let go.
    Released,
    /// The wheel over the rows: rows to scroll, up negative.
    Scrolled(i64),
    /// The search box holds this now.
    Search(String),
    /// The Remove menu's one row.
    RemoveChosen,
    /// The Unlink menu's one row.
    UnlinkChosen,
    /// Up, Down, Left, Right or Enter while the browser has the keys.
    Key(Nav),
}

/// What a press outside the Samples tree landed on.
#[derive(Debug, Clone, PartialEq)]
pub enum Pick {
    /// The starter kit's sound at this index.
    Sound(usize),
    /// An instrument set.
    Set(InstrumentSet),
    Plugin(Offered),
    /// Rescan, under the plugins.
    Rescan,
    /// "External input": an input channel.
    ExternalInput,
    /// A MIDI device's row, by its port's name.
    Device(String),
    /// A MIDI device's tick: whether its messages reach the app.
    Tick(String),
    /// A group's label inside Instruments, by its fold key ([`crate::ui::library::fold_key`]):
    /// it folds or opens.
    Group(String),
}

/// The keys that walk the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Up,
    Down,
    /// Closes the selected folder.
    Left,
    /// Opens the selected folder.
    Right,
    /// Previews the selected file, or opens or closes the selected folder (or the group whose
    /// label was pressed).
    Enter,
}

/// Which menu is open under a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuKind {
    Remove,
    Unlink,
}

impl MenuKind {
    fn label(self) -> &'static str {
        match self {
            MenuKind::Remove => "Remove",
            MenuKind::Unlink => "Unlink",
        }
    }

    fn chosen(self) -> BrowserMessage {
        match self {
            MenuKind::Remove => BrowserMessage::RemoveChosen,
            MenuKind::Unlink => BrowserMessage::UnlinkChosen,
        }
    }
}

/// Where row `row` (counted from the list's first row) is drawn, when the list scrolled to
/// `top` shows it at all.
pub fn row_rect(row: usize, top: usize) -> Option<Rectangle> {
    let shown = row.checked_sub(top)?;
    Some(Rectangle::new(
        Point::new(0.0, ROWS_TOP + shown as f32 * ROW_HEIGHT),
        Size::new(WIDTH, ROW_HEIGHT),
    ))
}

/// How many rows fit in a panel `height` tall.
pub fn visible_rows(height: f32) -> usize {
    ((height - ROWS_TOP) / ROW_HEIGHT).floor().max(0.0) as usize
}

/// Where the menu under row `row` opens (panel coordinates), when the list scrolled to `top`
/// shows that row at all: under the row, [`MENU_X`] in.
pub fn menu_anchor(row: usize, top: usize) -> Option<Point> {
    let rect = row_rect(row, top)?;
    Some(Point::new(MENU_X, rect.y + rect.height))
}

/// The tick of the MIDI device whose row is `rect`.
pub fn tick_rect(rect: Rectangle) -> Rectangle {
    Rectangle::new(
        Point::new(ROW_LEFT, rect.center_y() - TICK_SIZE / 2.0),
        Size::new(TICK_SIZE, TICK_SIZE),
    )
}

/// What the panel shows.
pub struct BrowserPanel<'a> {
    pub browser: &'a Browser,
    /// The Samples tree's rows, as [`Browser::rows`] gives them.
    pub rows: Vec<Row>,
    /// The Library's rows, the tree's among them.
    pub items: Vec<Item>,
    /// The first row shown: how far the list is scrolled.
    pub top: usize,
    /// Where the starter kit's sounds are kept, to light the one previewing.
    pub kit: Option<&'a Path>,
    /// A plugin scan runs.
    pub scanning: bool,
    /// A press is held on a row: moves and the release are reported anywhere.
    pub held: bool,
    /// The menu open under a row: the row (counted as `items`) and which menu.
    pub menu: Option<(usize, MenuKind)>,
    /// The groups folded, by fold key: their labels' triangles point right (not while
    /// searching, when every match shows).
    pub folded: &'a std::collections::BTreeSet<String>,
    /// The window's last pixels the menu under a row keeps clear of: the hint bar's.
    pub menu_foot: f32,
    pub cache: &'a Cache,
}

/// The panel's state between events: whether a tip was drawn, so moving off it draws again,
/// and what the cursor was over.
#[derive(Debug, Default)]
pub struct PanelState {
    tip: bool,
    hover: Option<Hover>,
}

impl<'a> BrowserPanel<'a> {
    pub fn view(self) -> Element<'a, Message> {
        let search = text_input("Search", &self.browser.search)
            .id(Id::new(SEARCH_INPUT))
            .on_input(|text| Message::Browser(BrowserMessage::Search(text)))
            .size(NAME_SIZE)
            .padding([2, 6])
            .width(WELL.width)
            .style(style::search_box);
        let menu = self.row_menu();
        let rows = canvas::Canvas::new(self).width(Fill).height(Fill);
        let mut layers: Vec<Element<'a, Message>> =
            vec![rows.into(), pin(search).x(WELL.x).y(WELL.y).into()];
        layers.extend(menu);
        container(stack(layers)).width(WIDTH).height(Fill).into()
    }

    /// The menu open under a row, on the window's top layer: its one row, Remove or Unlink.
    fn row_menu(&self) -> Option<Element<'a, Message>> {
        let (row, kind) = self.menu?;
        let at = menu_anchor(row, self.top)?;
        let rows = column![menu::row(
            kind.label(),
            Some(Message::Browser(kind.chosen()))
        )];
        let floating = menu::floating(menu::panel(rows, MENU_WIDTH), self.menu_foot);
        Some(pin(floating).x(at.x).y(at.y).into())
    }

    /// A group's label at `x`: its triangle, pointing down while the group shows its rows and
    /// right while it is folded, then its name.
    fn draw_group(&self, frame: &mut Frame, item: &Item, name: &str, x: f32, center: f32) {
        disclosure(frame, x, center, self.group_open(item));
        let name_x = x + NAME_OFFSET;
        fitted(frame, name, name_x, center, style::TEXT_DIM, LABEL_SIZE);
    }

    /// Whether group label `item` shows its rows: not folded, or a search runs.
    fn group_open(&self, item: &Item) -> bool {
        !self.browser.search.is_empty()
            || crate::ui::library::fold_key(item).is_none_or(|key| !self.folded.contains(&key))
    }

    fn draw_body(&self, frame: &mut Frame) {
        let size = frame.size();
        frame.fill_rectangle(Point::ORIGIN, size, style::PANEL);
        frame.fill_rectangle(
            Point::ORIGIN,
            Size::new(size.width, HEADER_HEIGHT),
            style::HEADER,
        );
        frame.fill_text(Text {
            content: "Library".to_owned(),
            position: Point::new(ROW_LEFT, HEADER_HEIGHT / 2.0),
            color: style::TEXT,
            size: NAME_SIZE.into(),
            align_y: Vertical::Center,
            ..Text::default()
        });
        frame.fill_rectangle(WELL.position(), WELL.size(), style::RACK_WELL);
        let shown = self
            .items
            .iter()
            .enumerate()
            .skip(self.top)
            .take(visible_rows(size.height));
        for (index, item) in shown {
            let Some(rect) = row_rect(index, self.top) else {
                continue;
            };
            self.draw_item(frame, item, rect);
        }
    }

    fn draw_item(&self, frame: &mut Frame, item: &Item, rect: Rectangle) {
        let center = rect.center_y();
        match item {
            Item::Header {
                section,
                count,
                open,
            } => draw_header(frame, rect, section.title(), *count, *open),
            Item::Group(name) => self.draw_group(frame, item, name, ROW_LEFT, center),
            Item::Maker(name) => self.draw_group(frame, item, name, ROW_LEFT + INDENT, center),
            Item::Kind(kind) => {
                self.draw_group(frame, item, kind.title(), ROW_LEFT + INDENT, center);
            }
            Item::Hint(line) => {
                let x = ROW_LEFT + INDENT;
                fitted(frame, line, x, center, style::TEXT_DIM, LABEL_SIZE);
            }
            Item::Rescan => {
                let words = if self.scanning {
                    "Scanning\u{2026}"
                } else {
                    "Rescan"
                };
                // A button, as the rack's own add buttons are: an action, not a hint line.
                let button = rescan_rect(rect, words);
                frame.fill_rectangle(button.position(), button.size(), style::BUTTON);
                let at = Point::new(button.x + BUTTON_PADDING, center);
                label(frame, words, at, style::TEXT_DIM, NAME_SIZE);
            }
            Item::Sound(index) => {
                let Some(sound) = starter_kit::SOUNDS.get(*index) else {
                    return;
                };
                let path = self.kit.map(|dir| sound.path_in(dir));
                if path.is_some() && self.browser.previewing == path {
                    draw_previewing(frame, rect);
                }
                let x = ROW_LEFT + INDENT;
                wave_glyph(frame, x, center);
                let right = self.mark_playing(frame, item, rect);
                fitted_to(frame, sound.name, x + NAME_OFFSET, right, center);
            }
            Item::Set(set) => {
                let x = ROW_LEFT + INDENT;
                set_glyph(frame, x, center);
                let right = self.mark_playing(frame, item, rect);
                fitted_to(frame, &set.name, x + NAME_OFFSET, right, center);
            }
            Item::Plugin(plugin) => {
                let x = ROW_LEFT + 2.0 * INDENT;
                plug_glyph(frame, x, center);
                let mut right = self.mark_playing(frame, item, rect);
                if plugin.format == PluginFormat::Vst3 {
                    right = mark_format(frame, plugin.format, right, center);
                }
                fitted_to(frame, &plugin.plugin.name, x + NAME_OFFSET, right, center);
            }
            Item::ExternalInput => {
                let x = ROW_LEFT + INDENT;
                jack_glyph(frame, x, center);
                let right = WIDTH - RIGHT_MARGIN;
                fitted_to(frame, EXTERNAL_INPUT, x + NAME_OFFSET, right, center);
            }
            Item::Tree(index) => {
                let Some(row) = self.rows.get(*index) else {
                    return;
                };
                let previewing = self.browser.previewing.as_ref() == Some(&row.path)
                    && row.kind == RowKind::File;
                if previewing {
                    draw_previewing(frame, rect);
                } else if self.browser.selected == Some(*index) {
                    frame.fill_rectangle(rect.position(), rect.size(), style::BROWSER_SELECTED);
                }
                draw_row(frame, row, rect);
            }
            Item::Port(port) => draw_port(frame, port, rect),
        }
    }

    /// Writes the mark at the right end of `item`'s row `rect` when the Library previews it;
    /// returns where the row's name must stop.
    fn mark_playing(&self, frame: &mut Frame, item: &Item, rect: Rectangle) -> f32 {
        let right = WIDTH - RIGHT_MARGIN;
        if !self
            .browser
            .playing
            .as_ref()
            .is_some_and(|playing| plays(playing, item))
        {
            return right;
        }
        let x = right - text_width(PLAYING_MARK, LABEL_SIZE);
        let at = Point::new(x, rect.center_y());
        label(frame, PLAYING_MARK, at, style::ACCENT, LABEL_SIZE);
        x - NAME_OFFSET / 2.0
    }

    /// The row under panel point `at`, counted from the list's first row.
    fn row_at(&self, at: Point, height: f32) -> Option<usize> {
        if at.y < ROWS_TOP {
            return None;
        }
        let shown = ((at.y - ROWS_TOP) / ROW_HEIGHT).floor() as usize;
        let row = self.top + shown;
        (shown < visible_rows(height) && row < self.items.len()).then_some(row)
    }

    /// What a press at panel point `at` with `button` asks for, the cursor at window point
    /// `window`.
    fn press(
        &self,
        at: Point,
        window: Point,
        button: mouse::Button,
        height: f32,
    ) -> Option<BrowserMessage> {
        let row = self.row_at(at, height)?;
        let left = button == mouse::Button::Left;
        let picked = |pick| BrowserMessage::Picked {
            pick,
            button,
            at: window,
        };
        Some(match self.items.get(row)? {
            Item::Header { section, .. } if left => BrowserMessage::Section(*section),
            Item::Tree(index) => BrowserMessage::Pressed {
                row: *index,
                button,
                at: window,
            },
            Item::Sound(index) => picked(Pick::Sound(*index)),
            Item::Set(set) => picked(Pick::Set(set.clone())),
            Item::Plugin(plugin) => picked(Pick::Plugin(plugin.clone())),
            Item::ExternalInput => picked(Pick::ExternalInput),
            Item::Rescan if left => picked(Pick::Rescan),
            Item::Group(_) | Item::Kind(_) | Item::Maker(_) if left => {
                picked(Pick::Group(crate::ui::library::fold_key(&self.items[row])?))
            }
            Item::Port(port) => {
                let rect = row_rect(row, self.top)?;
                let on_tick = tick_rect(rect).contains(at) && !port.unplugged && left;
                picked(if on_tick {
                    Pick::Tick(port.name.clone())
                } else {
                    Pick::Device(port.name.clone())
                })
            }
            _ => return None,
        })
    }

    /// What the hint bar names at panel point `at`: the row's kind and name; nothing on an
    /// empty state's line.
    fn hover_at(&self, at: Point, height: f32) -> Option<Hover> {
        let row = self.row_at(at, height)?;
        let item = self.items.get(row)?;
        let on_tick = row_rect(row, self.top).is_some_and(|rect| tick_rect(rect).contains(at));
        hover_of_item(item, &self.rows, on_tick)
    }

    /// The tip's words for the row under panel point `at`: what dragging it does, and for an
    /// instrument set the memory its sounds take.
    fn tip_words(&self, at: Point, height: f32) -> Option<String> {
        let tip = self.tip_at(at, height)?;
        let row = self.row_at(at, height)?;
        Some(match self.items.get(row) {
            Some(Item::Set(InstrumentSet {
                bytes: Some(bytes), ..
            })) => format!("{tip} \u{b7} {}", size_text(*bytes)),
            _ => tip.to_owned(),
        })
    }

    /// The tip's lines for the row under panel point `at`: [`tip_words`](Self::tip_words),
    /// and for an instrument set that names who made it, a line saying so.
    fn tip_lines(&self, at: Point, height: f32) -> Vec<String> {
        let Some(words) = self.tip_words(at, height) else {
            return Vec::new();
        };
        let maker = match self.row_at(at, height).and_then(|row| self.items.get(row)) {
            Some(Item::Set(set)) if !set.maker.is_empty() => Some(format!("By {}", set.maker)),
            _ => None,
        };
        std::iter::once(words).chain(maker).collect()
    }

    /// The tip for the row under panel point `at`: what dragging it does.
    fn tip_at(&self, at: Point, height: f32) -> Option<&'static str> {
        let row = self.row_at(at, height)?;
        match self.items.get(row)? {
            Item::Sound(_) | Item::Set(_) | Item::Plugin(_) | Item::ExternalInput => {
                Some(SOUND_TIP)
            }
            Item::Tree(index) => {
                (self.rows.get(*index)?.kind == RowKind::File).then_some(SOUND_TIP)
            }
            Item::Port(port) if !port.unplugged => Some(DEVICE_TIP),
            _ => None,
        }
    }
}

/// What the hint bar names for Library row `item` (`rows` the Samples tree's, `on_tick` when
/// the cursor is on a device's tick): its kind and name; nothing on an empty state's line.
pub fn hover_of_item(item: &Item, rows: &[Row], on_tick: bool) -> Option<Hover> {
    let part = match item {
        Item::Header { section, .. } => LibraryPart::Header(*section),
        Item::Sound(index) => {
            LibraryPart::Sound(crate::starter_kit::SOUNDS.get(*index)?.name.to_owned())
        }
        Item::Set(set) => LibraryPart::Set(set.name.clone()),
        Item::Plugin(plugin) => LibraryPart::Plugin {
            name: plugin.plugin.name.clone(),
            note: plugin_note(plugin),
        },
        Item::ExternalInput => LibraryPart::ExternalInput,
        Item::Rescan => LibraryPart::Rescan,
        Item::Tree(index) => {
            let shown = rows.get(*index)?;
            match shown.kind {
                RowKind::Root { .. } | RowKind::Folder { .. } => LibraryPart::Folder,
                RowKind::File => LibraryPart::File(shown.name.clone()),
                RowKind::AddFolder => LibraryPart::AddFolder,
            }
        }
        // Without its light, which blinks under a still cursor and would re-say the hint.
        Item::Port(port) if on_tick && !port.unplugged => LibraryPart::Tick(Port {
            lit: false,
            ..port.clone()
        }),
        Item::Port(port) => LibraryPart::Device(Port {
            lit: false,
            ..port.clone()
        }),
        Item::Group(name) => LibraryPart::Group((*name).to_owned()),
        Item::Kind(kind) => LibraryPart::Group(kind.title().to_owned()),
        Item::Maker(name) => LibraryPart::Group(name.clone()),
        Item::Hint(_) => return None,
    };
    Some(Hover::Library(part))
}

/// A section's header: a band across the panel, its disclosure triangle, and its title with
/// the count.
fn draw_header(frame: &mut Frame, rect: Rectangle, title: &str, count: usize, open: bool) {
    let band = Rectangle::new(
        Point::new(rect.x, rect.y + 1.0),
        Size::new(rect.width, rect.height - 2.0),
    );
    frame.fill_rectangle(band.position(), band.size(), style::BUTTON);
    disclosure(frame, ROW_LEFT, rect.center_y(), open);
    let words = format!("{title} \u{b7} {count}");
    let x = ROW_LEFT + NAME_OFFSET;
    fitted(
        frame,
        &words,
        x,
        rect.center_y(),
        style::BROWSER_ROOT,
        NAME_SIZE,
    );
}

/// A MIDI device: its tick, its keyboard glyph (its light: filled green while it sends) and
/// name, and where its notes go after an arrow at the row's right end: the channel linked to
/// it, else (while it plays) the selected one, in the warning color when that plays nothing;
/// dim with "(unplugged)" while it is not plugged in, dim as a whole for the system's loopback.
fn draw_port(frame: &mut Frame, port: &Port, rect: Rectangle) {
    let center = rect.center_y();
    let tick = tick_rect(rect);
    if port.unplugged {
        frame.stroke_rectangle(
            tick.position(),
            tick.size(),
            Stroke::default().with_color(style::KNOB_RING),
        );
    } else {
        let fill = if port.plays {
            style::ACCENT
        } else {
            style::KNOB_RING
        };
        frame.fill_rectangle(tick.position(), tick.size(), fill);
    }
    if port.lit {
        frame.fill_rectangle(
            Point::new(PORT_GLYPH_X + 0.5, center - 3.5),
            Size::new(10.0, 7.0),
            style::TRACK_LED_ON,
        );
    }
    keyboard_glyph(frame, PORT_GLYPH_X, center);
    let mut right = rect.width - RIGHT_MARGIN;
    if let Some(link) = port.arrow() {
        // At most half the row, so the device's own name keeps room.
        let room = (right - PORT_NAME_X) / 2.0;
        if let Some(link) = fit_width(&link, room, NAME_SIZE) {
            let width = text_width(&link, NAME_SIZE);
            // Lit for a link of the document's own: a channel linked to the device, or a MIDI
            // out sending to it while nothing else is said.
            let linked = port.channel.is_some() || (port.goes.is_none() && port.out.is_some());
            let color = if port.silent.is_some() {
                style::WARNING
            } else if port.unplugged || port.loopback || !linked {
                style::TEXT_DIM
            } else {
                style::ACCENT
            };
            label(
                frame,
                &link,
                Point::new(right - width, center),
                color,
                NAME_SIZE,
            );
            right -= width + 6.0;
        }
    }
    let (name, color) = if port.unplugged {
        (format!("{} (unplugged)", port.name), style::TEXT_DIM)
    } else if port.loopback {
        (port.name.clone(), style::TEXT_DIM)
    } else {
        (port.name.clone(), style::TEXT)
    };
    if let Some(name) = fit_width(&name, right - PORT_NAME_X, NAME_SIZE) {
        label(
            frame,
            &name,
            Point::new(PORT_NAME_X, center),
            color,
            NAME_SIZE,
        );
    }
}

/// The Rescan button on the row `rect`, holding `words`.
fn rescan_rect(rect: Rectangle, words: &str) -> Rectangle {
    let width = text_width(words, NAME_SIZE) + 2.0 * BUTTON_PADDING;
    Rectangle::new(
        Point::new(ROW_LEFT + INDENT, rect.y + 2.0),
        Size::new(width, rect.height - 4.0),
    )
}

fn draw_previewing(frame: &mut Frame, rect: Rectangle) {
    frame.fill_rectangle(rect.position(), rect.size(), style::BROWSER_PREVIEWING);
    frame.fill_rectangle(
        rect.position(),
        Size::new(PREVIEW_BAR, rect.height),
        style::ACCENT,
    );
}

/// Whether `item` is the instrument row `playing` names.
pub fn plays(playing: &Playing, item: &Item) -> bool {
    match (playing, item) {
        (Playing::KitSound(index), Item::Sound(row)) => index == row,
        (Playing::Set(sfz), Item::Set(set)) => *sfz == set.sfz,
        (Playing::Plugin(id), Item::Plugin(plugin)) => *id == plugin.processor_id(),
        _ => false,
    }
}

/// Writes `format`'s name dim at the right of a plugin row, ending at `right`; returns where the
/// row's name must stop.
fn mark_format(frame: &mut Frame, format: PluginFormat, right: f32, center: f32) -> f32 {
    let words = crate::ui::picker::format_name(format);
    let x = right - text_width(words, LABEL_SIZE);
    label(
        frame,
        words,
        Point::new(x, center),
        style::TEXT_DIM,
        LABEL_SIZE,
    );
    x - NAME_OFFSET / 2.0
}

/// What a plugin row's hint says of its format: "VST3" for a VST3 plugin, "also installed as
/// VST3" for a CLAP one whose VST3 build the list leaves out, nothing for any other.
pub fn plugin_note(plugin: &Offered) -> Option<&'static str> {
    match (plugin.format, plugin.twin) {
        (PluginFormat::Vst3, _) => Some("VST3"),
        (PluginFormat::Clap, true) => Some("also installed as VST3"),
        (PluginFormat::Clap, false) => None,
    }
}

/// A row's name from `x`, cut with an ellipsis where it would pass `right`.
fn fitted_to(frame: &mut Frame, words: &str, x: f32, right: f32, center: f32) {
    if let Some(words) = fit_width(words, right - x, NAME_SIZE) {
        label(frame, &words, Point::new(x, center), style::TEXT, NAME_SIZE);
    }
}

/// `words` from `x`, cut with an ellipsis to the panel's right margin.
fn fitted(frame: &mut Frame, words: &str, x: f32, center: f32, color: Color, size: f32) {
    if let Some(words) = fit_width(words, WIDTH - RIGHT_MARGIN - x, size) {
        label(frame, &words, Point::new(x, center), color, size);
    }
}

fn label(frame: &mut Frame, words: &str, at: Point, color: Color, size: f32) {
    frame.fill_text(Text {
        content: words.to_owned(),
        position: at,
        color,
        size: size.into(),
        align_y: Vertical::Center,
        ..Text::default()
    });
}

/// One row of the Samples tree: its disclosure triangle or wave glyph, then its name (and a
/// search match's folder, dimmed).
fn draw_row(frame: &mut Frame, row: &Row, rect: Rectangle) {
    let x = ROW_LEFT + row.depth as f32 * INDENT;
    let center = rect.center_y();
    match row.kind {
        RowKind::Root { open, .. } | RowKind::Folder { open } => disclosure(frame, x, center, open),
        RowKind::File => wave_glyph(frame, x, center),
        RowKind::AddFolder => {}
    }
    let (name_x, color) = match row.kind {
        RowKind::Root { .. } => (x + NAME_OFFSET, style::BROWSER_ROOT),
        RowKind::AddFolder => (x, style::TEXT_DIM),
        RowKind::Folder { .. } | RowKind::File => (x + NAME_OFFSET, style::TEXT),
    };
    let room = ((WIDTH - name_x - 4.0) / CHAR_WIDTH).max(1.0) as usize;
    let name = cut(&row.name, room);
    let used = name.chars().count();
    frame.fill_text(Text {
        content: name,
        position: Point::new(name_x, center),
        color,
        size: NAME_SIZE.into(),
        align_y: Vertical::Center,
        ..Text::default()
    });
    if let Some(parent) = &row.parent {
        let room = room.saturating_sub(used + 2);
        if room > 3 {
            frame.fill_text(Text {
                content: cut(parent, room),
                position: Point::new(name_x + (used + 2) as f32 * CHAR_WIDTH, center),
                color: style::TEXT_DIM,
                size: (NAME_SIZE - 1.0).into(),
                align_y: Vertical::Center,
                ..Text::default()
            });
        }
    }
}

/// `text` cut to `room` characters, the last one an ellipsis when it had to be cut.
fn cut(text: &str, room: usize) -> String {
    if text.chars().count() <= room {
        return text.to_owned();
    }
    let mut kept: String = text.chars().take(room.saturating_sub(1)).collect();
    kept.push('\u{2026}');
    kept
}

/// A folder's or a section's triangle at `x`: pointing down when open, right when closed.
fn disclosure(frame: &mut Frame, x: f32, center: f32, open: bool) {
    let corners = if open {
        [
            (x, center - 2.5),
            (x + 8.0, center - 2.5),
            (x + 4.0, center + 3.5),
        ]
    } else {
        [
            (x + 1.0, center - 4.0),
            (x + 7.0, center),
            (x + 1.0, center + 4.0),
        ]
    };
    frame.fill(&triangle(corners), style::TEXT_DIM);
}

fn glyph_stroke() -> Stroke<'static> {
    Stroke::default()
        .with_color(style::TEXT_DIM)
        .with_width(1.0)
}

/// A sound: one cycle of a wave.
fn wave_glyph(frame: &mut Frame, x: f32, center: f32) {
    let wave = Shape::new(|path| {
        path.move_to(Point::new(x, center));
        for step in 1..=8 {
            let t = step as f32 / 8.0;
            let y = center - 3.5 * (t * std::f32::consts::TAU).sin();
            path.line_to(Point::new(x + 9.0 * t, y));
        }
    });
    frame.stroke(&wave, glyph_stroke());
}

/// An audio input: a jack's sleeve and tip, its cable going off left.
fn jack_glyph(frame: &mut Frame, x: f32, center: f32) {
    let jack = Shape::new(|path| {
        path.move_to(Point::new(x, center));
        path.line_to(Point::new(x + 3.0, center));
        path.rectangle(Point::new(x + 3.0, center - 2.0), Size::new(4.0, 4.0));
        path.move_to(Point::new(x + 7.0, center));
        path.line_to(Point::new(x + 10.0, center));
    });
    frame.stroke(&jack, glyph_stroke());
}

/// A plugin: a plug's body with its two prongs above and its cable below.
fn plug_glyph(frame: &mut Frame, x: f32, center: f32) {
    let plug = Shape::new(|path| {
        path.rectangle(Point::new(x + 1.5, center - 1.5), Size::new(6.0, 4.0));
        for prong in [x + 3.0, x + 6.0] {
            path.move_to(Point::new(prong, center - 1.5));
            path.line_to(Point::new(prong, center - 4.5));
        }
        path.move_to(Point::new(x + 4.5, center + 2.5));
        path.line_to(Point::new(x + 4.5, center + 5.0));
    });
    frame.stroke(&plug, glyph_stroke());
}

/// An instrument set: two waves, one over the other, its layers.
fn set_glyph(frame: &mut Frame, x: f32, center: f32) {
    let waves = Shape::new(|path| {
        for (offset, height) in [(-2.5, 2.0), (2.5, 2.0)] {
            path.move_to(Point::new(x, center + offset));
            for step in 1..=8 {
                let t = step as f32 / 8.0;
                let y = center + offset - height * (t * std::f32::consts::TAU).sin();
                path.line_to(Point::new(x + 9.0 * t, y));
            }
        }
    });
    frame.stroke(&waves, glyph_stroke());
}

/// A MIDI device: a keyboard's outline and its keys.
fn keyboard_glyph(frame: &mut Frame, x: f32, center: f32) {
    let keys = Shape::new(|path| {
        path.rectangle(Point::new(x + 0.5, center - 3.5), Size::new(10.0, 7.0));
        for key in [x + 3.5, x + 5.5, x + 7.5] {
            path.move_to(Point::new(key, center - 3.5));
            path.line_to(Point::new(key, center));
        }
    });
    frame.stroke(&keys, glyph_stroke());
}

fn triangle(corners: [(f32, f32); 3]) -> Shape {
    Shape::new(|path| {
        let [first, second, third] = corners.map(|(x, y)| Point::new(x, y));
        path.move_to(first);
        path.line_to(second);
        path.line_to(third);
        path.close();
    })
}

/// Where the one-line tip `words` for the cursor at panel point `at` goes:
/// [`tip_rect_lines`] of one line.
#[cfg(test)]
fn tip_rect(words: &str, at: Point, bounds: Size) -> Rectangle {
    tip_rect_lines(&[words.to_owned()], at, bounds)
}

/// Where the tip of `lines` for the cursor at panel point `at` goes: under it, as wide as its
/// widest line and a line's height each, kept inside the panel's sides, and above it when there
/// is no room below.
fn tip_rect_lines(lines: &[String], at: Point, bounds: Size) -> Rectangle {
    let widest = lines
        .iter()
        .map(|line| text_width(line, LABEL_SIZE))
        .fold(0.0, f32::max);
    let width = (widest + 12.0).min(bounds.width - 8.0);
    let height = TIP_HEIGHT * lines.len() as f32;
    let x = (at.x - 10.0).clamp(4.0, (bounds.width - width - 4.0).max(4.0));
    let below = at.y + TIP_DROP;
    let y = if below + height <= bounds.height {
        below
    } else {
        at.y - TIP_DROP - height
    };
    Rectangle::new(Point::new(x, y), Size::new(width, height))
}

impl canvas::Program<Message> for BrowserPanel<'_> {
    type State = PanelState;

    fn update(
        &self,
        state: &mut PanelState,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let send = |message| canvas::Action::publish(Message::Browser(message));
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(
                button @ (mouse::Button::Left | mouse::Button::Right),
            )) => {
                let local = cursor.position_in(bounds)?;
                let at = cursor.position()?;
                let message = self.press(local, at, *button, bounds.height)?;
                Some(send(message).and_capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) if self.held => {
                Some(send(BrowserMessage::Moved(*position)))
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if self.held => {
                Some(send(BrowserMessage::Released))
            }
            // The tip follows the cursor over the rows, and goes when the cursor leaves them;
            // the hint bar names what is under it.
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let at = cursor.position_in(bounds);
                let tip = at.and_then(|at| self.tip_at(at, bounds.height)).is_some();
                let redraw = tip || state.tip;
                state.tip = tip;
                let now = at.and_then(|at| self.hover_at(at, bounds.height));
                hover_change(&mut state.hover, now)
                    .or_else(|| redraw.then(canvas::Action::request_redraw))
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                cursor.position_in(bounds)?;
                let notch = match delta {
                    mouse::ScrollDelta::Lines { y, .. } | mouse::ScrollDelta::Pixels { y, .. } => {
                        *y
                    }
                };
                (notch != 0.0).then(|| {
                    let rows = if notch > 0.0 {
                        -ROWS_PER_NOTCH
                    } else {
                        ROWS_PER_NOTCH
                    };
                    send(BrowserMessage::Scrolled(rows)).and_capture()
                })
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &PanelState,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let rows = self
            .cache
            .draw(renderer, bounds.size(), |frame| self.draw_body(frame));
        let mut tip = Frame::new(renderer, bounds.size());
        let shown = cursor
            .position_in(bounds)
            .filter(|_| !self.held)
            .map(|at| (at, self.tip_lines(at, bounds.height)))
            .filter(|(_, lines)| !lines.is_empty());
        if let Some((at, lines)) = shown {
            let rect = tip_rect_lines(&lines, at, bounds.size());
            tip.fill_rectangle(rect.position(), rect.size(), style::LCD);
            for (index, line) in lines.iter().enumerate() {
                if let Some(words) = fit_width(line, rect.width - 12.0, LABEL_SIZE) {
                    let y = rect.y + TIP_HEIGHT * (index as f32 + 0.5);
                    label(
                        &mut tip,
                        &words,
                        Point::new(rect.x + 6.0, y),
                        style::LCD_TEXT,
                        LABEL_SIZE,
                    );
                }
            }
        }
        vec![rows, tip.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _state: &PanelState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if self.held {
            return mouse::Interaction::Grabbing;
        }
        let item = cursor
            .position_in(bounds)
            .and_then(|at| self.row_at(at, bounds.height))
            .and_then(|row| self.items.get(row));
        match item {
            Some(Item::Hint(_)) | None => mouse::Interaction::default(),
            Some(_) => mouse::Interaction::Pointer,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // With the panel under the 64 px toolbar, row r's text box spans window y 122 + 22r .. 132 +
    // 22r, inside row r's rectangle (window y 116 + 22r .. 138 + 22r), and the well is window y
    // 90..110. (Pins moved 16 px down from the 56 px toolbar's, then 8 px up from the 72 px
    // one's.)
    #[test]
    fn rows_start_under_the_search_well() {
        let toolbar = crate::ui::layout::TOOLBAR_HEIGHT;
        for row in 0..3 {
            let rect = row_rect(row, 0).expect("shown");
            assert_eq!(rect.y + toolbar, 116.0 + 22.0 * row as f32);
            assert_eq!(rect.height, 22.0);
        }
        assert_eq!(
            (WELL.y + toolbar, WELL.y + WELL.height + toolbar),
            (90.0, 110.0)
        );
        assert_eq!((WELL.x, WELL.x + WELL.width), (6.0, 294.0));
        assert_eq!(row_rect(0, 1), None, "scrolled out of view");
        assert_eq!(visible_rows(504.0), 20);
    }

    #[test]
    fn long_names_are_cut_with_an_ellipsis() {
        assert_eq!(cut("kick.wav", 8), "kick.wav");
        assert_eq!(cut("a-very-long-name.wav", 6), "a-ver\u{2026}");
    }

    use crate::ui::browser_model::Row;
    use crate::ui::library::{DEVICE_HINT, PLUGIN_HINT};

    fn port(name: &str, unplugged: bool) -> Item {
        Item::Port(Port {
            name: name.to_owned(),
            unplugged,
            ..Port::default()
        })
    }

    // v2 addition: the Library marks the one row it previews: a kit sound by its index, a set
    // by its main file, a plugin by its id; no other row, and nothing of another kind.
    #[test]
    fn the_preview_marks_its_own_row_alone() {
        let set = |sfz: &str| {
            Item::Set(InstrumentSet {
                name: "Pad".to_owned(),
                maker: String::new(),
                sfz: sfz.into(),
                bytes: None,
            })
        };
        let pad = Playing::Set("sets/pad/pad.sfz".into());
        assert!(plays(&pad, &set("sets/pad/pad.sfz")));
        assert!(!plays(&pad, &set("sets/layers/layers.sfz")));
        assert!(!plays(&pad, &Item::Sound(0)));
        assert!(plays(&Playing::KitSound(2), &Item::Sound(2)));
        assert!(!plays(&Playing::KitSound(2), &Item::Sound(3)));
        let plugin = |id: &str| {
            Item::Plugin(Offered {
                format: PluginFormat::Clap,
                plugin: dawsome_host_clap::catalogue::PluginRecord {
                    id: id.to_owned(),
                    name: "Lead".to_owned(),
                    vendor: String::new(),
                    version: String::new(),
                    kind: dawsome_host_clap::catalogue::PluginKind::Instrument,
                    gui: dawsome_host_clap::catalogue::GuiSupport::None,
                    params: 0,
                },
                twin: false,
            })
        };
        // A plugin row plays by its whole processor id, its format included (a VST3
        // plugin of the same id text is another plugin).
        assert!(plays(
            &Playing::Plugin("clap:a.lead".to_owned()),
            &plugin("a.lead")
        ));
        assert!(!plays(
            &Playing::Plugin("clap:a.lead".to_owned()),
            &plugin("a.keys")
        ));
        assert!(!plays(
            &Playing::Plugin("vst3:a.lead".to_owned()),
            &plugin("a.lead")
        ));
        assert!(!plays(&Playing::KitSound(0), &Item::Rescan));
    }

    /// A panel of `items` over a tree of a folder and a file, scrolled to the top.
    fn with_panel(items: Vec<Item>, test: impl FnOnce(&BrowserPanel)) {
        let browser = Browser::default();
        let cache = Cache::new();
        let row = |name: &str, kind| Row {
            depth: 0,
            name: name.to_owned(),
            path: name.into(),
            kind,
            parent: None,
        };
        let panel = BrowserPanel {
            browser: &browser,
            rows: vec![
                row("drums", RowKind::Folder { open: false }),
                row("tone.wav", RowKind::File),
            ],
            items,
            top: 0,
            kit: None,
            scanning: false,
            held: false,
            menu: None,
            menu_foot: 0.0,
            folded: &std::collections::BTreeSet::new(),
            cache: &cache,
        };
        test(&panel);
    }

    /// The middle of row `row`, panel coordinates, and its x moved to `x`.
    fn on_row(row: usize, x: f32) -> Point {
        Point::new(x, row_rect(row, 0).expect("shown").center_y())
    }

    // A press on a header opens or closes its section (with the left button only); on the tree's
    // rows it names the tree's own row; on a device's tick it switches the device, on the rest
    // of the row it holds the device, and an unplugged device's tick only holds it; a group's
    // label folds it by its key (the left button only), hints take no press.
    #[test]
    fn presses_name_what_they_land_on() {
        let items = vec![
            Item::Header {
                section: Section::Devices,
                count: 2,
                open: true,
            },
            Item::Group("Plugins"),
            Item::Hint(DEVICE_HINT[0]),
            Item::Tree(1),
            port("Port A", false),
            port("Port C", true),
            Item::Rescan,
        ];
        with_panel(items, |panel| {
            let window = Point::new(1.0, 2.0);
            let left = mouse::Button::Left;
            let press = |row, x, button| panel.press(on_row(row, x), window, button, 500.0);
            assert_eq!(
                press(0, 100.0, left),
                Some(BrowserMessage::Section(Section::Devices))
            );
            assert_eq!(press(0, 100.0, mouse::Button::Right), None);
            assert_eq!(press(1, 100.0, mouse::Button::Right), None);
            assert_eq!(press(2, 100.0, left), None);
            assert_eq!(
                press(3, 100.0, left),
                Some(BrowserMessage::Pressed {
                    row: 1,
                    button: left,
                    at: window
                })
            );
            let tick = ROW_LEFT + TICK_SIZE / 2.0;
            let picked = |pick| {
                Some(BrowserMessage::Picked {
                    pick,
                    button: left,
                    at: window,
                })
            };
            assert_eq!(
                press(4, tick, left),
                picked(Pick::Tick("Port A".to_owned()))
            );
            assert_eq!(
                press(4, 150.0, left),
                picked(Pick::Device("Port A".to_owned()))
            );
            assert_eq!(
                press(5, tick, left),
                picked(Pick::Device("Port C".to_owned()))
            );
            assert_eq!(press(6, 100.0, left), picked(Pick::Rescan));
            assert_eq!(
                press(1, 100.0, left),
                picked(Pick::Group("Plugins".to_owned()))
            );
            assert_eq!(press(7, 100.0, left), None, "past the last row");
        });
    }

    // The tip says what a drag does: a sound, a plugin and a sample file go onto the rack or a
    // track, a plugged-in device onto a channel; a folder, an unplugged device and a header
    // have none.
    #[test]
    fn the_tip_says_where_a_drag_goes() {
        let plugin = Offered {
            format: PluginFormat::Clap,
            plugin: dawsome_host_clap::catalogue::PluginRecord {
                id: "org.example.lead".to_owned(),
                name: "Analog Lead".to_owned(),
                vendor: "Example".to_owned(),
                version: "1.0.0".to_owned(),
                kind: dawsome_host_clap::catalogue::PluginKind::Instrument,
                gui: dawsome_host_clap::catalogue::GuiSupport::None,
                params: 1,
            },
            twin: false,
        };
        let items = vec![
            Item::Sound(0),
            Item::Plugin(plugin),
            Item::Tree(0),
            Item::Tree(1),
            port("Port A", false),
            port("Port C", true),
            Item::Header {
                section: Section::Samples,
                count: 1,
                open: true,
            },
        ];
        with_panel(items, |panel| {
            let tips: Vec<Option<&str>> = (0..7)
                .map(|row| panel.tip_at(on_row(row, 100.0), 500.0))
                .collect();
            assert_eq!(
                tips,
                [
                    Some(SOUND_TIP),
                    Some(SOUND_TIP),
                    None,
                    Some(SOUND_TIP),
                    Some(DEVICE_TIP),
                    None,
                    None
                ]
            );
        });
        let tip = tip_rect(SOUND_TIP, Point::new(290.0, 480.0), Size::new(WIDTH, 500.0));
        assert!(
            tip.x >= 4.0 && tip.x + tip.width <= WIDTH - 4.0,
            "inside the panel"
        );
        assert!(
            tip.y + tip.height <= 500.0,
            "above the cursor at the bottom"
        );
    }

    // Each line of the empty states fits its row from where it is drawn, so none is cut.
    #[test]
    fn the_hint_lines_fit_their_rows() {
        let room = WIDTH - RIGHT_MARGIN - (ROW_LEFT + INDENT);
        for line in PLUGIN_HINT.iter().chain(DEVICE_HINT) {
            assert!(text_width(line, LABEL_SIZE) <= room, "{line}");
        }
    }

    // A set's row holds the set for a drag, and its tip says what a drag does and how much
    // memory the set takes; a set not measured has the plain tip.
    #[test]
    fn a_sets_row_is_held_and_its_tip_names_its_size() {
        let set = |bytes| InstrumentSet {
            name: "Piano".to_owned(),
            maker: String::new(),
            sfz: "piano.sfz".into(),
            bytes,
        };
        let items = vec![Item::Set(set(Some(142_300_000))), Item::Set(set(None))];
        with_panel(items, |panel| {
            let window = Point::new(1.0, 2.0);
            assert_eq!(
                panel.press(on_row(0, 100.0), window, mouse::Button::Left, 500.0),
                Some(BrowserMessage::Picked {
                    pick: Pick::Set(set(Some(142_300_000))),
                    button: mouse::Button::Left,
                    at: window,
                })
            );
            assert_eq!(
                panel.tip_words(on_row(0, 100.0), 500.0).as_deref(),
                Some("Drag onto the rack or a track \u{b7} 142.3 MB")
            );
            assert_eq!(
                panel.tip_words(on_row(1, 100.0), 500.0).as_deref(),
                Some(SOUND_TIP)
            );
        });
    }

    // A set's tip names its maker on a line of its own under what a drag does; a set nobody is
    // named for, and any other row, has the one line. Two lines make the tip twice as tall.
    #[test]
    fn a_sets_tip_names_its_maker() {
        let set = |maker: &str| {
            Item::Set(InstrumentSet {
                name: "Piano".to_owned(),
                maker: maker.to_owned(),
                sfz: "piano.sfz".into(),
                bytes: Some(2_000_000),
            })
        };
        let items = vec![set("FreePats"), set(""), Item::Sound(0)];
        with_panel(items, |panel| {
            let lines = |row| panel.tip_lines(on_row(row, 100.0), 500.0);
            assert_eq!(
                lines(0),
                [
                    "Drag onto the rack or a track \u{b7} 2.0 MB".to_owned(),
                    "By FreePats".to_owned()
                ]
            );
            assert_eq!(
                lines(1),
                ["Drag onto the rack or a track \u{b7} 2.0 MB".to_owned()]
            );
            assert_eq!(lines(2), [SOUND_TIP.to_owned()]);
        });
        let size = Size::new(WIDTH, 500.0);
        let one = tip_rect_lines(&["a".to_owned()], Point::new(100.0, 100.0), size);
        let two = tip_rect_lines(
            &["a".to_owned(), "b".to_owned()],
            Point::new(100.0, 100.0),
            size,
        );
        assert_eq!((one.height, two.height), (TIP_HEIGHT, 2.0 * TIP_HEIGHT));
        let low = tip_rect_lines(
            &["a".to_owned(), "b".to_owned()],
            Point::new(100.0, 490.0),
            size,
        );
        assert!(
            low.y + low.height <= 500.0,
            "above the cursor at the bottom"
        );
    }
}
