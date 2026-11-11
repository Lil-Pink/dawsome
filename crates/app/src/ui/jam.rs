//! Jam's views: "Start a Jam" on the channel rack's header ([`start_button`]), the prompt over
//! the main window that asks which instruments play and which part each fills ([`prompt`]), the
//! Jam window ([`JamPage`]) and Settings' Music page, the project's key ([`music_page`]). The
//! state and what each message does are the app's (`app/jam.rs`).
//!
//! The Jam window opens at a fixed size ([`WINDOW_SIZE`]), cut to what a small screen leaves
//! but no smaller than [`WINDOW_MIN`] ([`fitted_size`]), its channel table scrolling.

use std::fmt;

use dawsome_engine::jam::{Genre, KeySignature, PitchClass, Role, Scale};
use dawsome_engine::project::ChannelId;
use dawsome_engine::time::SWING_RANGE;
use iced::alignment::Horizontal;
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::widget::{
    button, checkbox, column, container, pick_list, row, scrollable, slider, space, text,
    text_input,
};
use iced::{
    mouse, Background, Border, Center, Element, Fill, Point, Rectangle, Renderer, Size, Theme,
};

use crate::app::Message;
use crate::ui::fonts::size;
use crate::ui::hint::with_tip;
use crate::ui::rack::swing_text;
use crate::ui::style;
use crate::ui::text_fit::fit_width;

/// What the Jam's views report.
#[derive(Debug, Clone, PartialEq)]
pub enum JamMessage {
    /// The rack header's "Start a Jam": the prompt over the main window.
    Start,
    /// A prompt row's tick: the channel plays in the Jam, or stays out.
    Join(ChannelId, bool),
    /// A prompt row's role menu.
    RoleChosen(ChannelId, RoleName),
    /// The prompt's Next: the Jam window opens on the channels ticked.
    Next,
    /// The prompt's Cancel (and Esc): it closes, nothing written.
    Cancel,
    Genre(GenreChoice),
    /// The Tempo row's Auto: the genre's tempo, or the one typed.
    AutoTempo(bool),
    TempoEdited(String),
    Root(RootChoice),
    Scale(ScaleChoice),
    Bars(u32),
    Swing(u16),
    /// The big dice: every channel not locked rolled again.
    Roll,
    /// A channel's small dice: that channel alone rolled again.
    RollChannel(ChannelId),
    /// A channel's Lock: it keeps its notes across rolls, or not.
    Lock(ChannelId),
    /// The seed field typed in: the next roll takes that seed, to write a roll again.
    SeedEdited(String),
    /// The die beside the seed field: a fresh seed from the clock in the field, for the next
    /// roll; nothing rolled.
    NewSeed,
    /// A row's instrument menu: the row's part moves from the first channel to the second, heard
    /// at once.
    Switch(ChannelId, ChannelId),
    /// A row's "×": the channel leaves the Jam, its part dropped at once.
    Remove(ChannelId),
    /// "+ Add instrument…": the channel joins the Jam, its part written by the next roll.
    Add(ChannelId),
    /// A row's role menu: the part the channel plays from the next roll on.
    RowRole(ChannelId, RoleName),
    /// The pattern rolled is kept: the next roll writes a new one.
    Accept,
    /// A clip of the Jam pattern on the playlist, at the song position.
    SendToPlaylist,
    Close,
    /// Settings' Music page: the project's key root, or no key.
    ProjectRoot(KeyRoot),
    ProjectScale(ScaleChoice),
}

fn send(message: JamMessage) -> Message {
    Message::Jam(message)
}

/// A role as the prompt's menu offers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleName {
    Drums,
    Bass,
    Chords,
    Melody,
}

impl RoleName {
    pub const ALL: [RoleName; 4] = [
        RoleName::Drums,
        RoleName::Bass,
        RoleName::Chords,
        RoleName::Melody,
    ];

    pub fn of(role: Role) -> Self {
        match role {
            Role::Drums(_) => RoleName::Drums,
            Role::Bass => RoleName::Bass,
            Role::Chords => RoleName::Chords,
            Role::Melody => RoleName::Melody,
        }
    }
}

impl fmt::Display for RoleName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            RoleName::Drums => "Drums",
            RoleName::Bass => "Bass",
            RoleName::Chords => "Chords",
            RoleName::Melody => "Melody",
        })
    }
}

/// A genre as the Genre menu lists it, by its shown name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenreChoice(pub Genre);

impl GenreChoice {
    pub fn all() -> [GenreChoice; Genre::ALL.len()] {
        Genre::ALL.map(GenreChoice)
    }
}

impl fmt::Display for GenreChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.name())
    }
}

/// A scale as the Scale menus list it, by its shown name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScaleChoice(pub Scale);

impl ScaleChoice {
    pub fn all() -> [ScaleChoice; Scale::ALL.len()] {
        Scale::ALL.map(ScaleChoice)
    }
}

impl fmt::Display for ScaleChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.name())
    }
}

/// A key's root as the Jam window's Key menu lists it: C to B, sharps for the black keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootChoice(pub PitchClass);

impl RootChoice {
    pub fn all() -> Vec<RootChoice> {
        (0..12)
            .filter_map(PitchClass::new)
            .map(RootChoice)
            .collect()
    }
}

impl fmt::Display for RootChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.name())
    }
}

/// Settings' Key menu: no key, or a root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyRoot {
    None,
    Root(PitchClass),
}

impl KeyRoot {
    pub fn all() -> Vec<KeyRoot> {
        std::iter::once(KeyRoot::None)
            .chain((0..12).filter_map(PitchClass::new).map(KeyRoot::Root))
            .collect()
    }
}

impl fmt::Display for KeyRoot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyRoot::None => f.write_str("No key"),
            KeyRoot::Root(root) => f.write_str(root.name()),
        }
    }
}

/// "Start a Jam" for the rack's header, flat as the header's other controls
/// ([`style::header_button`]): greyed with the reason as its tip while `refused` says why a Jam
/// cannot be written, else pressable, its tip what it does. Its words are centred in it and the
/// header's row centres it on the strip, so they stand on the line of "Swing" and "Bars".
pub fn start_button<'a>(refused: Option<String>) -> Element<'a, Message> {
    let words = text("Start a Jam")
        .size(size::LABEL)
        .height(Fill)
        .align_y(Center);
    let face = button(words)
        .padding([0, 8])
        .height(START_HEIGHT)
        .style(style::header_button)
        .on_press_maybe(refused.is_none().then_some(send(JamMessage::Start)));
    let tip =
        refused.unwrap_or_else(|| "Writes a new pattern for the instruments on the rack".into());
    with_tip(face, tip)
}

/// The height of "Start a Jam": the rack header's controls', 2 px clear of the strip's top and
/// bottom.
pub const START_HEIGHT: f32 = crate::ui::layout::RACK_HEADER_HEIGHT - 4.0;

/// The prompt's hint on the melody's part: a held sound plays it longer (`Articulation`).
pub const MELODY_HINT: &str =
    "Leads, plucks and bells sound best on the melody; keys, strings and pads are held longer.";

/// One of the rack's channels as the prompt lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct PromptRow {
    pub channel: ChannelId,
    pub name: String,
    /// Whether the channel plays notes: one that does not (an audio or input channel) is listed
    /// dim and cannot join.
    pub plays_notes: bool,
    /// The part it plays: the one found for it, or the one picked; none: "pick a role".
    pub role: Option<Role>,
    pub joined: bool,
}

/// The side of a tick box, in the prompt and the Jam window.
const CHECK_BOX: f32 = 14.0;
/// The role menu's width in the prompt: "pick a role" and its arrow.
const ROLE_MENU_WIDTH: f32 = 120.0;
/// How tall the prompt's list grows before it scrolls.
const PROMPT_LIST_MOST: f32 = 320.0;
/// The prompt's room for a channel's name: the dialog's inside less the tick box, its gap, the
/// role menu and the space before it.
const PROMPT_NAME_ROOM: f32 = 424.0 - CHECK_BOX - 8.0 - ROLE_MENU_WIDTH - 8.0;

fn action<'a>(label: &'a str, message: Option<Message>) -> Element<'a, Message> {
    // 10 px a side: Send to playlist, Accept and Close fit the window's 276 px row with room.
    button(text(label).size(size::BODY))
        .padding([6, 10])
        .style(style::tool_button(false))
        .on_press_maybe(message)
        .into()
}

/// A channel's name cut to `room` at `size`, with an ellipsis when it is longer.
fn fitted(name: &str, room: f32, size: f32) -> String {
    fit_width(name, room, size).map_or_else(String::new, std::borrow::Cow::into_owned)
}

/// The prompt: the rack's channels, each with its tick and its part (a channel with no part
/// found is off, its tick dim until a part is picked, and says "pick a role"), then Next (once a
/// channel ticked has a part) and Cancel.
pub fn prompt(rows: &[PromptRow]) -> Element<'_, Message> {
    let lines = rows.iter().map(|shown| {
        let channel = shown.channel;
        let tick = checkbox(shown.joined)
            .label(fitted(&shown.name, PROMPT_NAME_ROOM, size::BODY))
            .size(CHECK_BOX)
            .spacing(8)
            .text_size(size::BODY)
            .width(Fill)
            .style(style::check_box)
            .on_toggle_maybe(
                (shown.plays_notes && shown.role.is_some())
                    .then_some(move |on| send(JamMessage::Join(channel, on))),
            );
        let role: Element<'_, Message> = if shown.plays_notes {
            pick_list(RoleName::ALL, shown.role.map(RoleName::of), move |name| {
                send(JamMessage::RoleChosen(channel, name))
            })
            .placeholder("pick a role")
            .text_size(size::LABEL)
            .padding([4, 8])
            .width(ROLE_MENU_WIDTH)
            .style(style::chooser)
            .menu_style(style::chooser_menu)
            .into()
        } else {
            container(
                text("plays no notes")
                    .size(size::LABEL)
                    .color(style::palette().text_disabled),
            )
            .width(ROLE_MENU_WIDTH)
            .into()
        };
        row![tick, role].spacing(8).align_y(Center).into()
    });
    let list = container(scrollable(column(lines).spacing(4))).max_height(PROMPT_LIST_MOST);
    let ready = rows.iter().any(|row| row.joined && row.role.is_some());
    column![
        text("Start a Jam").size(size::TITLE),
        text("Tick the instruments that play, and the part each one fills.")
            .size(size::LABEL)
            .color(style::palette().text_secondary),
        list,
        text(MELODY_HINT)
            .size(size::LABEL)
            .color(style::palette().text_secondary),
        row![
            space().width(Fill),
            action("Next", ready.then_some(send(JamMessage::Next))),
            action("Cancel", Some(send(JamMessage::Cancel))),
        ]
        .spacing(8)
        .align_y(Center),
    ]
    .spacing(10)
    .into()
}

/// The Jam window's size at a zoom of 100 %: the settings in two columns, the dice, the channel
/// table with room for about eight rows, and the buttons. Fixed: the user does not resize it.
/// Why: the window wanted measured 1018 x 952 device pixels on the desktop at its 1.70 scale,
/// and a window that stays so reads the same every time it opens.
pub const WINDOW_SIZE: Size = Size::new(600.0, 560.0);
/// The least it is made on a small screen ([`fitted_size`]): the settings and a table row or two,
/// the table scrolling.
pub const WINDOW_MIN: Size = Size::new(420.0, 400.0);

/// Room a screen keeps from the Jam window, in the system's logical pixels: the window's title
/// bar and a desktop panel, about 30 and 44 on KDE Plasma. Why a margin: the toolkit tells the
/// app its monitor's size, not the room the desktop leaves on it.
pub const SCREEN_ROOM: f32 = 80.0;

/// The Jam window's size on a monitor of `monitor` (in the window's own pixels, at `zoom`):
/// [`WINDOW_SIZE`], each side cut to what the monitor leaves ([`SCREEN_ROOM`] off its height)
/// but no smaller than [`WINDOW_MIN`]; [`WINDOW_SIZE`] when the monitor is not known.
pub fn fitted_size(monitor: Option<Size>, zoom: f32) -> Size {
    let Some(monitor) = monitor else {
        return WINDOW_SIZE;
    };
    let width = WINDOW_SIZE.width.min(monitor.width);
    let height = WINDOW_SIZE.height.min(monitor.height - SCREEN_ROOM / zoom);
    Size::new(width.max(WINDOW_MIN.width), height.max(WINDOW_MIN.height))
}

/// The window's margin, the space between its blocks and between its settings' rows, the two
/// settings columns' gap, a settings row's height, the dice's, the status line's.
const PADDING: f32 = 12.0;
const GAP: f32 = 8.0;
const COLUMN_GAP: f32 = 16.0;
const ROW_HEIGHT: f32 = 28.0;
const ROLL_HEIGHT: f32 = 36.0;
const STATUS_HEIGHT: f32 = 18.0;
/// The settings stand in two columns from this width of the window, else in one (a screen
/// narrower than the window's width).
const TWO_COLUMNS: f32 = 560.0;

/// The seed field's id, which a test focuses to type a seed.
pub const SEED_FIELD: &str = "jam-seed";

/// The labels' column ("Genre", "Tempo", ...).
const LABEL_WIDTH: f32 = 72.0;
/// The Tempo box, the Key's root menu, the Seed field and the dice beside it.
const TEMPO_WIDTH: f32 = 64.0;
const ROOT_WIDTH: f32 = 62.0;
const SEED_WIDTH: f32 = 96.0;
const DICE_WIDTH: f32 = 28.0;

/// The channel table: its header's height, a row's, the space inside its panel, a row's
/// instrument menu, its role menu, Lock, the die and "×", the space between them, and the add
/// menu's width.
const HEADER_HEIGHT: f32 = 22.0;
const TABLE_ROW: f32 = 30.0;
const TABLE_PADDING: f32 = 6.0;
const INSTRUMENT_WIDTH: f32 = 170.0;
const ROLE_WIDTH: f32 = 104.0;
const LOCK_WIDTH: f32 = 48.0;
const REMOVE_WIDTH: f32 = 26.0;
const CELL_GAP: f32 = 6.0;
const ADD_WIDTH: f32 = 230.0;
/// Room at the table rows' right for the scroll bar, which floats over the rows when they
/// scroll (iced's is 10 px): the header keeps it too, so the columns line up.
const SCROLL_ROOM: f32 = 10.0;

/// What a row shows while its part waits for a roll: added since the last roll, or its role
/// changed.
pub const ROLL_TO_HEAR: &str = "roll to hear";

/// A rack channel as the Jam window's instrument menus list it, by its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelChoice {
    pub channel: ChannelId,
    pub name: String,
}

impl fmt::Display for ChannelChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

/// A rack channel not in the Jam as "+ Add instrument…" lists it: its name and the part it
/// would play.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddChoice {
    pub channel: ChannelId,
    pub name: String,
    pub role: RoleName,
}

impl fmt::Display for AddChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} \u{b7} {}", self.name, self.role)
    }
}

/// One channel as the Jam window lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct JamRow {
    pub channel: ChannelId,
    pub name: String,
    pub role: Role,
    pub locked: bool,
    /// Its part waits for the next roll: it joined since the last roll, or its role changed.
    pub pending: bool,
}

/// What the Jam window shows.
pub struct JamPage<'a> {
    pub genre: Genre,
    pub auto_tempo: bool,
    /// The tempo shown: the genre's while Auto is on, else as typed.
    pub tempo: String,
    pub key: KeySignature,
    pub bars: u32,
    pub swing: u16,
    pub channels: &'a [JamRow],
    /// The rack's channels that play notes and are not in the Jam: what a row's instrument menu
    /// switches to, and what "+ Add instrument…" adds (with the part each would play).
    pub free: Vec<ChannelChoice>,
    pub addable: Vec<AddChoice>,
    /// The seed field: the last roll's seed (which writes the same pattern again, as `dawsome
    /// jam --seed` does), or one typed or drawn for the next roll.
    pub seed: &'a str,
    /// The line under the dice: why the last roll was refused (in the warning colour), else
    /// what a roll does now.
    pub status: String,
    pub refused: bool,
    /// Whether a pattern was rolled that Accept keeps.
    pub rolled: bool,
    /// The window's width, which decides the settings' columns.
    pub width: f32,
}

/// `content` in a settings row with `label` before it, its label's column [`LABEL_WIDTH`].
fn labelled<'a>(label: &'a str, content: Element<'a, Message>) -> Element<'a, Message> {
    container(
        row![
            text(label)
                .size(size::LABEL)
                .color(style::palette().text_secondary)
                .width(LABEL_WIDTH),
            content
        ]
        .spacing(8)
        .align_y(Center),
    )
    .height(ROW_HEIGHT)
    .align_y(Center)
    .into()
}

/// A small die button: `message` on a press, `tip` under it.
fn die_button<'a>(message: Message, tip: &'a str) -> Element<'a, Message> {
    let face = button(
        container(canvas::Canvas::new(Dice).width(12).height(12))
            .center_x(Fill)
            .center_y(Fill),
    )
    .width(DICE_WIDTH)
    .height(22)
    .padding(0)
    .style(style::tool_button(false))
    .on_press(message);
    with_tip(face, tip)
}

/// The channel table's panel: the raised band in the buttons' edge.
fn table_panel(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(style::palette().raised)),
        border: Border {
            color: style::palette().button_edge,
            width: 1.0,
            radius: 2.0.into(),
        },
        ..container::Style::default()
    }
}

/// Row `index` of the table: the two row tones in turn.
fn row_tone(index: usize) -> impl Fn(&Theme) -> container::Style {
    move |_theme| container::Style {
        background: Some(Background::Color(style::palette().row_tones[index % 2])),
        ..container::Style::default()
    }
}

impl<'a> JamPage<'a> {
    pub fn view(self) -> Element<'a, Message> {
        let settings = self.settings();
        let roll = button(
            container(
                row![
                    canvas::Canvas::new(Dice).width(18).height(18),
                    text("Roll").size(size::TITLE),
                ]
                .spacing(10)
                .align_y(Center),
            )
            .center_x(Fill)
            .height(Fill)
            .align_y(Center),
        )
        .width(Fill)
        .height(ROLL_HEIGHT)
        .padding([0, 12])
        .style(style::tool_button(false))
        .on_press(send(JamMessage::Roll));
        let status_color = if self.refused {
            style::WARNING
        } else {
            style::palette().text_secondary
        };
        let room = self.width - 2.0 * PADDING;
        let status = container(
            text(fitted(&self.status, room, size::LABEL))
                .size(size::LABEL)
                .color(status_color)
                .wrapping(text::Wrapping::None),
        )
        .height(STATUS_HEIGHT)
        .align_y(Center);
        let buttons = container(
            row![
                space().width(Fill),
                action(
                    "Send to playlist",
                    self.rolled.then_some(send(JamMessage::SendToPlaylist)),
                ),
                action("Accept", self.rolled.then_some(send(JamMessage::Accept))),
                action("Close", Some(send(JamMessage::Close))),
            ]
            .spacing(8)
            .align_y(Center),
        )
        .height(ROW_HEIGHT + 4.0)
        .align_y(Center);
        let table = self.table();
        let page = column![settings, column![roll, status].spacing(4), table, buttons]
            .spacing(GAP)
            .height(Fill);
        container(page)
            .padding(PADDING)
            .width(Fill)
            .height(Fill)
            .style(style::panel)
            .into()
    }

    /// The settings: Genre, Tempo and Bars on the left, Key, Swing and Seed on the right; in one
    /// column under [`TWO_COLUMNS`].
    fn settings(&self) -> Element<'a, Message> {
        let genre = pick_list(GenreChoice::all(), Some(GenreChoice(self.genre)), |genre| {
            send(JamMessage::Genre(genre))
        })
        .text_size(size::LABEL)
        .padding([4, 8])
        .width(Fill)
        .style(style::chooser)
        .menu_style(style::chooser_menu);
        let tempo_box = text_input("", &self.tempo)
            .size(size::LABEL)
            .width(TEMPO_WIDTH)
            .padding([3, 6])
            .style(style::hex_box)
            .on_input_maybe(
                (!self.auto_tempo).then_some(|typed| send(JamMessage::TempoEdited(typed))),
            );
        let auto = checkbox(self.auto_tempo)
            .label("Auto")
            .size(CHECK_BOX)
            .spacing(6)
            .text_size(size::LABEL)
            .style(style::check_box)
            .on_toggle(|on| send(JamMessage::AutoTempo(on)));
        let tempo = row![tempo_box, text("BPM").size(size::LABEL), auto]
            .spacing(8)
            .align_y(Center);
        let bars = row([4u32, 8].map(|bars| {
            button(text(bars.to_string()).size(size::LABEL))
                .padding([4, 10])
                .style(style::tool_button(bars == self.bars))
                .on_press(send(JamMessage::Bars(bars)))
                .into()
        }))
        .spacing(2);
        let root = pick_list(RootChoice::all(), Some(RootChoice(self.key.root)), |root| {
            send(JamMessage::Root(root))
        })
        .text_size(size::LABEL)
        .padding([4, 8])
        .width(ROOT_WIDTH)
        .style(style::chooser)
        .menu_style(style::chooser_menu);
        let scale = pick_list(
            ScaleChoice::all(),
            Some(ScaleChoice(self.key.scale)),
            |scale| send(JamMessage::Scale(scale)),
        )
        .text_size(size::LABEL)
        .padding([4, 8])
        .width(Fill)
        .style(style::chooser)
        .menu_style(style::chooser_menu);
        let key = row![root, scale].spacing(6).align_y(Center);
        let swing = row![
            slider(SWING_RANGE, self.swing, |tenths| send(JamMessage::Swing(
                tenths
            )))
            .step(1u16)
            .width(Fill),
            text(swing_text(self.swing))
                .size(size::LABEL)
                .width(44)
                .align_x(Horizontal::Right),
        ]
        .spacing(8)
        .align_y(Center);
        let seed_field = text_input("", self.seed)
            .id(SEED_FIELD)
            .size(size::LABEL)
            .width(SEED_WIDTH)
            .padding([3, 6])
            .style(style::hex_box)
            .on_input(|typed| send(JamMessage::SeedEdited(typed)));
        let seed = row![
            seed_field,
            die_button(
                send(JamMessage::NewSeed),
                "Draws a new seed for the next roll",
            ),
        ]
        .spacing(6)
        .align_y(Center);
        let left = column![
            labelled("Genre", genre.into()),
            labelled("Tempo", tempo.into()),
            labelled("Bars", bars.into()),
        ]
        .spacing(GAP);
        let right = column![
            labelled("Key", key.into()),
            labelled("Swing", swing.into()),
            labelled("Seed", seed.into()),
        ]
        .spacing(GAP);
        if self.width >= TWO_COLUMNS {
            row![left.width(Fill), right.width(Fill)]
                .spacing(COLUMN_GAP)
                .into()
        } else {
            column![left, right].spacing(GAP).into()
        }
    }

    /// The channel table in its raised panel: the header, a row per channel in the rack's order
    /// on the two row tones, and "+ Add instrument…" last; the rows scroll under the header.
    fn table(&self) -> Element<'a, Message> {
        let head = |words: &'a str| {
            text(words)
                .size(size::LABEL)
                .color(style::palette().text_secondary)
        };
        // The first cell spans what a row's instrument and hint take.
        let header = container(
            row![
                head("Instrument").width(Fill),
                head("Role").width(ROLE_WIDTH),
                head("Lock").width(LOCK_WIDTH).align_x(Horizontal::Center),
                head("Roll").width(DICE_WIDTH).align_x(Horizontal::Center),
                space().width(REMOVE_WIDTH),
            ]
            .spacing(CELL_GAP)
            .align_y(Center),
        )
        .padding(iced::Padding::from([0.0, TABLE_PADDING]).right(TABLE_PADDING + SCROLL_ROOM))
        .height(HEADER_HEIGHT)
        .align_y(Center);
        let mut rows: Vec<Element<'a, Message>> = self
            .channels
            .iter()
            .enumerate()
            .map(|(index, shown)| self.channel_row(index, shown))
            .collect();
        rows.push(self.add_row());
        let list = scrollable(column(rows).width(Fill))
            .height(Fill)
            .width(Fill);
        container(column![header, list].width(Fill).height(Fill))
            .padding(1)
            .width(Fill)
            .height(Fill)
            .style(table_panel)
            .into()
    }

    /// A channel's row: its instrument (a menu of it and the rack's channels not in the Jam,
    /// which switches the row's instrument), "roll to hear" while its part waits for a roll, its
    /// role's menu, Lock, its own die and "×", which takes it out of the Jam.
    fn channel_row(&self, index: usize, shown: &'a JamRow) -> Element<'a, Message> {
        let channel = shown.channel;
        let mut choices = vec![ChannelChoice {
            channel,
            name: shown.name.clone(),
        }];
        choices.extend(self.free.iter().cloned());
        let current = choices[0].clone();
        let instrument = pick_list(choices, Some(current), move |to: ChannelChoice| {
            send(JamMessage::Switch(channel, to.channel))
        })
        .text_size(size::LABEL)
        .padding([4, 8])
        .width(INSTRUMENT_WIDTH)
        .style(style::chooser)
        .menu_style(style::chooser_menu);
        let hint = text(if shown.pending { ROLL_TO_HEAR } else { "" })
            .size(size::LABEL)
            .color(style::palette().text_disabled)
            .wrapping(text::Wrapping::None)
            .width(Fill);
        let role = pick_list(RoleName::ALL, Some(RoleName::of(shown.role)), move |name| {
            send(JamMessage::RowRole(channel, name))
        })
        .text_size(size::LABEL)
        .padding([4, 8])
        .width(ROLE_WIDTH)
        .style(style::chooser)
        .menu_style(style::chooser_menu);
        let lock = button(
            text("Lock")
                .size(size::LABEL)
                .width(Fill)
                .align_x(Horizontal::Center),
        )
        .width(LOCK_WIDTH)
        .padding([4, 4])
        .style(style::tool_button(shown.locked))
        .on_press(send(JamMessage::Lock(channel)));
        let remove = button(
            text("\u{d7}")
                .size(size::BODY)
                .width(Fill)
                .align_x(Horizontal::Center),
        )
        .width(REMOVE_WIDTH)
        .padding([2, 0])
        .style(style::tool_button(false))
        .on_press(send(JamMessage::Remove(channel)));
        container(
            row![
                row![instrument, hint]
                    .spacing(CELL_GAP)
                    .align_y(Center)
                    .width(Fill),
                role,
                with_tip(lock, "Keeps this channel's notes when you roll"),
                die_button(
                    send(JamMessage::RollChannel(channel)),
                    "Rolls this channel alone"
                ),
                with_tip(remove, "Takes this channel out of the Jam"),
            ]
            .spacing(CELL_GAP)
            .align_y(Center),
        )
        .padding(iced::Padding::from([0.0, TABLE_PADDING]).right(TABLE_PADDING + SCROLL_ROOM))
        .height(TABLE_ROW)
        .align_y(Center)
        .style(row_tone(index))
        .into()
    }

    /// The table's last row: "+ Add instrument…", a menu of the rack's channels not in the Jam
    /// with the part each would play; a dim line when every one is in.
    fn add_row(&self) -> Element<'a, Message> {
        let content: Element<'a, Message> = if self.addable.is_empty() {
            text("Every instrument on the rack is in the Jam")
                .size(size::LABEL)
                .color(style::palette().text_disabled)
                .into()
        } else {
            pick_list(self.addable.clone(), None::<AddChoice>, |add: AddChoice| {
                send(JamMessage::Add(add.channel))
            })
            .placeholder("+ Add instrument\u{2026}")
            .text_size(size::LABEL)
            .padding([4, 8])
            .width(ADD_WIDTH)
            .style(style::chooser)
            .menu_style(style::chooser_menu)
            .into()
        };
        container(content)
            .padding(iced::Padding::from([0.0, TABLE_PADDING]).right(TABLE_PADDING + SCROLL_ROOM))
            .height(TABLE_ROW + 4.0)
            .align_y(Center)
            .into()
    }
}

/// A die's face showing five, in the text's colour: the Jam's dice, drawn rather than a glyph
/// (the bundled font has none).
struct Dice;

impl canvas::Program<Message> for Dice {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let side = bounds.width.min(bounds.height) - 1.0;
        let color = style::palette().text;
        let face = Path::rounded_rectangle(
            Point::new(0.5, 0.5),
            Size::new(side, side),
            (side * 0.2).into(),
        );
        frame.stroke(&face, Stroke::default().with_color(color).with_width(1.0));
        let pip = side * 0.09;
        for (x, y) in [
            (0.28, 0.28),
            (0.72, 0.28),
            (0.5, 0.5),
            (0.28, 0.72),
            (0.72, 0.72),
        ] {
            frame.fill(
                &Path::circle(Point::new(0.5 + side * x, 0.5 + side * y), pip),
                color,
            );
        }
        vec![frame.into_geometry()]
    }
}

/// Settings' Music page: the project's key, its root ("No key" for none) and its scale, and
/// what it is for.
pub fn music_page<'a>(key: Option<KeySignature>) -> Element<'a, Message> {
    let root = key.map_or(KeyRoot::None, |key| KeyRoot::Root(key.root));
    let roots = pick_list(KeyRoot::all(), Some(root), |root| {
        send(JamMessage::ProjectRoot(root))
    })
    .text_size(size::LABEL)
    .padding([4, 8])
    .width(96)
    .style(style::chooser)
    .menu_style(style::chooser_menu);
    let scale: Element<'a, Message> = match key {
        Some(key) => pick_list(ScaleChoice::all(), Some(ScaleChoice(key.scale)), |scale| {
            send(JamMessage::ProjectScale(scale))
        })
        .text_size(size::LABEL)
        .padding([4, 8])
        .width(160)
        .style(style::chooser)
        .menu_style(style::chooser_menu)
        .into(),
        None => space().width(160).into(),
    };
    let about = "The project's key: a Jam writes its chords, bass and melody in it, and the \
                 piano roll shades the keys outside it.";
    container(
        column![
            text("Key").size(size::TITLE),
            row![roots, scale].spacing(8).align_y(Center),
            text(about)
                .size(size::LABEL)
                .color(style::palette().text_secondary),
        ]
        .spacing(10)
        .max_width(420),
    )
    .padding([8.0, crate::ui::settings_layout::INSET])
    .width(Fill)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    // The window's size on a monitor, by hand (the monitor in the window's own pixels): none
    // known, or one of 1920 x 1080, 600 x 560; 1366 x 600 leaves 600 − 80 = 520 high; 1024 x 450
    // leaves 370, held to the least 400; 500 x 700 is 500 wide; 380 x 300 the least, 420 x 400;
    // at a zoom of 2 the room's 80 is 40 of the window's pixels, so 1280 x 720 leaves 680, past
    // 560.
    #[test]
    fn the_window_keeps_its_size_and_shrinks_to_a_small_screen() {
        let on = |width, height, zoom| fitted_size(Some(Size::new(width, height)), zoom);
        assert_eq!(fitted_size(None, 1.0), Size::new(600.0, 560.0));
        assert_eq!(on(1920.0, 1080.0, 1.0), Size::new(600.0, 560.0));
        assert_eq!(on(1366.0, 600.0, 1.0), Size::new(600.0, 520.0));
        assert_eq!(on(1024.0, 450.0, 1.0), Size::new(600.0, 400.0));
        assert_eq!(on(500.0, 700.0, 1.0), Size::new(500.0, 560.0));
        assert_eq!(on(380.0, 300.0, 1.0), WINDOW_MIN);
        assert_eq!(on(1280.0, 720.0, 2.0), WINDOW_SIZE);
    }

    // The menus' words: the genres' shown names, "No key" before the roots, the scales'.
    #[test]
    fn the_menus_say_the_engines_names() {
        let genres: Vec<String> = GenreChoice::all().iter().map(ToString::to_string).collect();
        assert_eq!(
            genres,
            [
                "Lo-fi",
                "Boom bap",
                "Trap",
                "Drill",
                "EDM / house",
                "Phonk",
                "Afrobeats"
            ]
        );
        let roots: Vec<String> = KeyRoot::all().iter().map(ToString::to_string).collect();
        assert_eq!(roots.len(), 13);
        assert_eq!((roots[0].as_str(), roots[2].as_str()), ("No key", "C#"));
        assert_eq!(
            ScaleChoice(Scale::HarmonicMinor).to_string(),
            "Harmonic minor"
        );
    }
}
