//! The modal dialogs: unsaved changes, recovering an autosave, the import report and the
//! export settings. Each one sits over the dimmed window and takes every click while shown.

use std::path::PathBuf;
use std::sync::OnceLock;

use dawsome_media::{BitDepth, Dither};
use iced::alignment::Horizontal;
use iced::widget::{
    button, column, container, mouse_area, opaque, pick_list, row, rule, scrollable, slider, space,
    text, text_input, Id,
};
use iced::{Center, Element, Fill, Padding, Point};

use crate::app::{
    Choice, ExportKind, ExportMessage, FileMessage, MediaMessage, Message, PatternMessage,
    PlayMode, RecordMessage, SignatureMessage, TakeKind,
};
use crate::keys::{Action, Keys};
use crate::play_to::RecordSource;
use crate::ui::fonts::size;
use crate::ui::hint::{hover_area, EditRow, FileRow, Hover, MenuRow, RecordRow};
use crate::ui::layout::TOOLBAR_HEIGHT;
use crate::ui::panels::Panel;
use crate::ui::roll::QuantizeForm;
use crate::ui::style;
use crate::ui::text_fit::widest;

/// The width every dialog box takes.
const DIALOG_WIDTH: f32 = 460.0;

/// How tall the import report's list grows before it scrolls.
const REPORT_HEIGHT: f32 = 300.0;

fn file(message: FileMessage) -> Message {
    Message::File(message)
}

fn action<'a>(label: &'a str, message: Option<Message>) -> Element<'a, Message> {
    button(text(label).size(size::BODY))
        .padding([6, 14])
        .style(style::tool_button(false))
        .on_press_maybe(message)
        .into()
}

/// A row of buttons on the right.
fn actions<'a>(buttons: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    row![space().width(Fill)]
        .extend(buttons)
        .spacing(8)
        .align_y(Center)
        .into()
}

/// `content` in a dialog box over the dimmed window.
pub fn overlay<'a>(content: Element<'a, Message>) -> Element<'a, Message> {
    opaque(container(boxed(content)).center(Fill).style(style::scrim))
}

/// As [`overlay`], without dimming the window: for a dialog whose effect shows behind it, as
/// the Quantize dialog's preview shows in the piano roll. Clicks still stop at it.
pub fn overlay_undimmed<'a>(content: Element<'a, Message>) -> Element<'a, Message> {
    opaque(container(boxed(content)).center(Fill))
}

/// A dialog's box: its width, margin and face.
fn boxed<'a>(content: Element<'a, Message>) -> Element<'a, Message> {
    container(content)
        .width(DIALOG_WIDTH)
        .padding(18)
        .style(style::dialog)
        .into()
}

/// "Save changes to <name>?" with Save, Don't Save and Cancel.
pub fn unsaved(name: &str) -> Element<'_, Message> {
    let choice = |label, choice| action(label, Some(file(FileMessage::Unsaved(choice))));
    column![
        text(format!("Save changes to {name}?")).size(size::TITLE),
        text("Changes not saved are lost.")
            .size(size::LABEL)
            .color(style::palette().text_secondary),
        actions(vec![
            choice("Save", Choice::Save),
            choice("Don't Save", Choice::DontSave),
            choice("Cancel", Choice::Cancel),
        ]),
    ]
    .spacing(14)
    .into()
}

/// "Recover unsaved changes from <age>?" with Open autosave, and Open saved file (or Discard,
/// when the document has no file).
pub fn recover(age: &str, has_file: bool) -> Element<'_, Message> {
    let other = if has_file {
        "Open saved file"
    } else {
        "Discard"
    };
    column![
        text(format!("Recover unsaved changes from {age}?")).size(size::TITLE),
        actions(vec![
            action("Open autosave", Some(file(FileMessage::Recover(true)))),
            action(other, Some(file(FileMessage::Recover(false)))),
        ]),
    ]
    .spacing(14)
    .into()
}

/// The import report: its summary line, then each heading with its lines.
pub fn report<'a>(
    summary: &'a str,
    groups: &'a [(&'static str, Vec<String>)],
) -> Element<'a, Message> {
    let groups = groups.iter().map(|(title, lines)| {
        let lines = lines
            .iter()
            .map(|line| text(line.as_str()).size(size::LABEL).into());
        column![
            text(*title).size(size::BODY).color(style::accent()),
            column(lines).spacing(2)
        ]
        .spacing(4)
        .into()
    });
    column![
        text(summary).size(size::TITLE),
        scrollable(column(groups).spacing(12).padding(Padding::ZERO.right(12)))
            .height(REPORT_HEIGHT),
        actions(vec![action("OK", Some(file(FileMessage::CloseReport)))]),
    ]
    .spacing(14)
    .into()
}

/// What the export dialog shows.
pub struct ExportForm<'a> {
    pub mp3: bool,
    /// Exporting stems: the playlist's tracks, a file each besides the master's. The song
    /// plays, its whole bars, so neither what plays nor the bars are chosen.
    pub stems: Option<usize>,
    pub bars: &'a str,
    pub depth: BitDepth,
    /// Whether a 16-bit file is dithered (WAV and stems).
    pub dither: Dither,
    pub rate: u32,
    /// What plays: the pattern on screen or the song.
    pub plays: PlayMode,
    /// The pattern on screen's name, when there is one.
    pub pattern: Option<&'a str>,
    /// While exporting: the share done.
    pub progress: Option<f32>,
    pub error: Option<&'a str>,
}

fn field<'a>(label: &'a str, value: Element<'a, Message>) -> Element<'a, Message> {
    row![text(label).size(size::BODY).width(110), value]
        .spacing(8)
        .align_y(Center)
        .into()
}

/// What plays: "Pattern" (lit when chosen, and off without a pattern on screen) and "Song",
/// then the chosen pattern's name, dim.
fn plays<'a>(form: &ExportForm<'a>, running: bool) -> Element<'a, Message> {
    let send = |message| file(FileMessage::ExportForm(message));
    let choice = |label, plays: PlayMode, offered: bool| {
        button(text(label).size(size::LABEL))
            .padding([5, 8])
            .style(style::tool_button(plays == form.plays))
            .on_press_maybe((offered && !running).then_some(send(ExportMessage::Plays(plays))))
    };
    let mut choices = row![
        choice("Pattern", PlayMode::Pattern, form.pattern.is_some()),
        choice("Song", PlayMode::Song, true),
    ]
    .spacing(2)
    .align_y(Center);
    if let (PlayMode::Pattern, Some(name)) = (form.plays, form.pattern) {
        choices = choices.push(space().width(6));
        choices = choices.push(
            text(name)
                .size(size::LABEL)
                .color(style::palette().text_secondary),
        );
    }
    choices.into()
}

/// Dither: "TPDF" (lit when on) and "Off", offered at 16 bits, the only depth dithered; at
/// another the choices rest and say so.
fn dither<'a>(form: &ExportForm<'a>, running: bool) -> Element<'a, Message> {
    let send = |message| file(FileMessage::ExportForm(message));
    let sixteen = form.depth == BitDepth::Int16;
    let choice = |label, dither: Dither| {
        button(text(label).size(size::LABEL))
            .padding([5, 8])
            .style(style::tool_button(sixteen && dither == form.dither))
            .on_press_maybe((sixteen && !running).then_some(send(ExportMessage::Dither(dither))))
    };
    let mut choices = row![choice("TPDF", Dither::Tpdf), choice("Off", Dither::Off)]
        .spacing(2)
        .align_y(Center);
    if !sixteen {
        choices = choices.push(space().width(6));
        choices = choices.push(
            text("16-bit only")
                .size(size::LABEL)
                .color(style::palette().text_secondary),
        );
    }
    choices.into()
}

/// What plays, the bars, the rate, the bit depth and dither (WAV); Export… and Cancel, or the
/// progress.
pub fn export<'a>(form: ExportForm<'a>) -> Element<'a, Message> {
    let send = |message| file(FileMessage::ExportForm(message));
    let title = match (form.stems, form.mp3) {
        (Some(_), _) => "Export stems",
        (None, true) => "Export MP3",
        (None, false) => "Export WAV",
    };
    let running = form.progress.is_some();
    let bars = text_input("1", form.bars)
        .size(size::BODY)
        .width(80)
        .on_input_maybe((!running).then_some(move |value| send(ExportMessage::Bars(value))));
    let format: Element<'a, Message> = if form.mp3 {
        text("320 kbps stereo").size(size::BODY).into()
    } else {
        let depths = BitDepth::ALL.into_iter().map(|depth| {
            button(text(depth.label()).size(size::LABEL))
                .padding([5, 8])
                .style(style::tool_button(depth == form.depth))
                .on_press_maybe((!running).then_some(send(ExportMessage::Depth(depth))))
                .into()
        });
        row(depths).spacing(2).into()
    };
    let mut content = column![text(title).size(size::TITLE)].spacing(10);
    content = match form.stems {
        Some(tracks) => {
            let plural = if tracks == 1 { "" } else { "s" };
            let files = format!("{tracks} track{plural} and the master, a WAV file each");
            content
                .push(field("Files", text(files).size(size::BODY).into()))
                .push(field(
                    "Length",
                    text(format!("the song, {} bars", form.bars))
                        .size(size::BODY)
                        .into(),
                ))
        }
        None => content
            .push(field("Plays", plays(&form, running)))
            .push(field("Bars", bars.into())),
    };
    content = content
        .push(field(
            "Sample rate",
            text(format!("{} Hz", form.rate)).size(size::BODY).into(),
        ))
        .push(field(if form.mp3 { "Format" } else { "Bit depth" }, format));
    if !form.mp3 {
        content = content.push(field("Dither", dither(&form, running)));
    }
    if let Some(error) = form.error {
        content = content.push(text(error).size(size::LABEL).color(style::WARNING));
    }
    let buttons = match form.progress {
        Some(done) => row![
            text(format!("Exporting… {:.0}%", done * 100.0)).size(size::BODY),
            space().width(Fill),
            action("Cancel", Some(send(ExportMessage::Cancel))),
        ]
        .align_y(Center)
        .into(),
        None => actions(vec![
            action("Export…", Some(send(ExportMessage::Start))),
            action("Cancel", Some(send(ExportMessage::Cancel))),
        ]),
    };
    content.push(buttons).into()
}

/// The Quantize dialog: the grid (the snap or a note value), the strength, the swing (with a
/// Triplet button, lit at 66.7 %), whether the ends move too, how many notes its preview
/// merges (in the warning color when any do), Quantize and Cancel. Each control says what it
/// is in the hint bar.
/// The Strength readout's words for `strength` percent.
fn strength_words(strength: u8) -> String {
    format!("{strength} %")
}

/// The room the Strength readout keeps: its widest value's (0 to 100 % in fives), so the row
/// stays put as the slider moves (the type scale's digits are proportional).
fn strength_width() -> f32 {
    static WIDTH: OnceLock<f32> = OnceLock::new();
    *WIDTH.get_or_init(|| widest((0..=100).step_by(5).map(strength_words), size::BODY))
}

pub fn quantize<'a>(
    form: &QuantizeForm,
    merges: Option<crate::ui::roll::QuantizeMerges>,
) -> Element<'a, Message> {
    use dawsome_engine::time::{SWING_RANGE, TRIPLET_SWING};

    use crate::ui::hint::QuantizePart;
    use crate::ui::rack::swing_text;
    use crate::ui::roll::{QuantizeGrid, QuantizeMessage};
    let send = Message::Quantize;
    let grid = pick_list(QuantizeGrid::ALL, Some(form.grid), move |grid| {
        send(QuantizeMessage::Grid(grid))
    })
    .text_size(size::BODY)
    .padding([4, 8]);
    let strength = row![
        slider(0..=100u8, form.strength, move |value| send(
            QuantizeMessage::Strength(value)
        ))
        .step(5u8)
        .width(200),
        container(text(strength_words(form.strength)).size(size::BODY))
            .width(strength_width())
            .align_x(Horizontal::Right),
    ]
    .spacing(10)
    .align_y(Center);
    // The readout has a width of its own, so the Triplet button stays put as the digits change.
    let swing = row![
        slider(SWING_RANGE, form.swing_tenths, move |value| send(
            QuantizeMessage::Swing(value)
        ))
        .step(1u16)
        .width(200),
        text(swing_text(form.swing_tenths))
            .size(size::BODY)
            .width(44)
            .align_x(Horizontal::Right),
        button(text("Triplet").size(size::LABEL))
            .padding([5, 8])
            .style(style::tool_button(form.swing_tenths == TRIPLET_SWING))
            .on_press(send(QuantizeMessage::Swing(TRIPLET_SWING))),
    ]
    .spacing(10)
    .align_y(Center);
    let choice = |label: &'a str, lit: bool, ends: bool| {
        button(text(label).size(size::LABEL))
            .padding([5, 8])
            .style(style::tool_button(lit))
            .on_press(send(QuantizeMessage::Ends(ends)))
    };
    let moves = row![
        choice("Starts", !form.ends, false),
        choice("Starts + ends", form.ends, true),
    ]
    .spacing(2);
    let merges = merges.map(|merges| {
        let color = if merges.merged == 0 {
            style::palette().text_secondary
        } else {
            style::WARNING
        };
        text(merges.to_string()).size(size::LABEL).color(color)
    });
    column![
        text("Quantize").size(size::TITLE),
        field(
            "Grid",
            hover_area(grid, Hover::Quantize(QuantizePart::Grid))
        ),
        field(
            "Strength",
            hover_area(strength, Hover::Quantize(QuantizePart::Strength))
        ),
        field(
            "Swing",
            hover_area(swing, Hover::Quantize(QuantizePart::Swing))
        ),
        field(
            "Moves",
            hover_area(moves, Hover::Quantize(QuantizePart::Ends))
        ),
    ]
    .push(merges)
    .push(actions(vec![
        hover_area(
            action("Quantize", Some(send(QuantizeMessage::Apply))),
            Hover::Quantize(QuantizePart::Apply),
        ),
        action("Cancel", Some(send(QuantizeMessage::Cancel))),
    ]))
    .spacing(10)
    .into()
}

/// What the File menu lists.
pub struct FileMenu<'k> {
    /// Recent documents, newest first, and whether each still exists.
    pub recent: Vec<(PathBuf, bool)>,
    /// Why MP3 export is not offered, if it is not.
    pub mp3_unavailable: Option<&'static str>,
    /// The shortcuts, whose keys the rows name.
    pub keys: &'k Keys,
    /// The window's height: a menu taller than the room under the toolbar scrolls.
    pub window_height: f32,
    /// How far the menu's right edge lies from the window's: under the File button's
    /// ([`crate::ui::toolbar::Spread::file_menu_right`]; Edit and Help stand right of File).
    pub right: f32,
}

/// The room under the toolbar a menu dropping from it has in a window `height` tall, a margin
/// clear of the window's foot.
fn room_under_toolbar(height: f32) -> f32 {
    (height - (TOOLBAR_HEIGHT - 6.0) - MENU_MARGIN).max(0.0)
}

/// The room a menu leaves under it at the window's foot.
const MENU_MARGIN: f32 = 8.0;

fn entry<'a>(
    label: String,
    keys: impl text::IntoFragment<'a>,
    message: Option<Message>,
    dimmed: bool,
) -> Element<'a, Message> {
    button(
        row![
            text(label).size(size::BODY),
            space().width(Fill),
            text(keys)
                .size(size::LABEL)
                .color(style::palette().text_secondary)
        ]
        .spacing(16),
    )
    .width(Fill)
    .padding([5, 10])
    .style(style::menu_item(dimmed))
    .on_press_maybe(message)
    .into()
}

/// `entry` as row `row` of a menu, saying when the cursor comes onto it and leaves, for the hint
/// bar.
fn menu_entry<'a>(
    row: MenuRow,
    label: String,
    keys: impl text::IntoFragment<'a>,
    message: Option<Message>,
    dimmed: bool,
) -> Element<'a, Message> {
    hover_area(entry(label, keys, message, dimmed), Hover::Menu(row))
}

/// The File menu under the toolbar's File button, over a backdrop that closes it when clicked:
/// its rows in [`FileRow::ALL`]'s order, the recent files where `Recent` stands, a rule before
/// the recent files, Save and the exports.
pub fn file_menu<'a>(menu: FileMenu) -> Element<'a, Message> {
    let key = |action| menu.keys.first(action);
    let mp3 = menu
        .mp3_unavailable
        .is_none()
        .then_some(file(FileMessage::Export(ExportKind::Mp3)));
    let mut list = column![].spacing(2);
    for row in FileRow::ALL {
        let fixed = |label: &str, keys: String, message: Message| {
            menu_entry(
                MenuRow::File(row),
                label.to_owned(),
                keys,
                Some(message),
                false,
            )
        };
        let shown = match row {
            FileRow::New => fixed("New", key(Action::New), file(FileMessage::New)),
            FileRow::NewEmpty => fixed(
                "New empty project",
                String::new(),
                file(FileMessage::NewEmpty),
            ),
            FileRow::Open => fixed("Open\u{2026}", key(Action::Open), file(FileMessage::Open)),
            FileRow::OpenDemo => fixed("Open demo", String::new(), file(FileMessage::OpenDemo)),
            FileRow::ImportAudio => fixed(
                "Import audio\u{2026}",
                String::new(),
                Message::Media(MediaMessage::ImportAudio),
            ),
            FileRow::ImportMidi => fixed(
                "Import MIDI\u{2026}",
                String::new(),
                file(FileMessage::ImportMidi),
            ),
            FileRow::Recent => {
                list = list.push(rule::horizontal(1)).push(
                    text("Open recent")
                        .size(size::LABEL)
                        .color(style::palette().text_secondary),
                );
                if menu.recent.is_empty() {
                    list = list.push(entry("No recent files".into(), "", None, true));
                }
                for (path, exists) in &menu.recent {
                    let name = path.file_name().map_or_else(
                        || path.display().to_string(),
                        |name| name.to_string_lossy().into_owned(),
                    );
                    let open = file(FileMessage::OpenRecent(path.clone()));
                    list = list.push(menu_entry(
                        MenuRow::File(row),
                        name,
                        "",
                        Some(open),
                        !*exists,
                    ));
                }
                continue;
            }
            FileRow::Save => {
                list = list.push(rule::horizontal(1));
                fixed("Save", key(Action::Save), file(FileMessage::Save))
            }
            FileRow::SaveAs => fixed(
                "Save As\u{2026}",
                key(Action::SaveAs),
                file(FileMessage::SaveAs),
            ),
            FileRow::ExportWav => {
                list = list.push(rule::horizontal(1));
                fixed(
                    "Export WAV\u{2026}",
                    String::new(),
                    file(FileMessage::Export(ExportKind::Wav)),
                )
            }
            FileRow::ExportMp3 => menu_entry(
                MenuRow::File(row),
                "Export MP3\u{2026}".into(),
                menu.mp3_unavailable.unwrap_or(""),
                mp3.clone(),
                false,
            ),
            FileRow::ExportMidi => fixed(
                "Export pattern as MIDI\u{2026}",
                String::new(),
                file(FileMessage::ExportMidi),
            ),
            FileRow::ExportSongMidi => fixed(
                "Export song as MIDI\u{2026}",
                String::new(),
                file(FileMessage::ExportSongMidi),
            ),
            FileRow::ExportStems => fixed(
                "Export stems\u{2026}",
                String::new(),
                file(FileMessage::Export(ExportKind::Stems)),
            ),
            FileRow::ExportDawproject => fixed(
                "Export DAWproject\u{2026}",
                String::new(),
                file(FileMessage::ExportDawproject),
            ),
        };
        list = list.push(shown);
    }
    // Ten recent files make it taller than a small window: then it scrolls inside the room
    // under the toolbar, rather than running past the window's foot.
    let list = scrollable(list).direction(scrollable::Direction::Vertical(
        scrollable::Scrollbar::new().width(4).scroller_width(4),
    ));
    let panel = container(list)
        .width(300)
        .max_height(room_under_toolbar(menu.window_height))
        .padding(6)
        .style(style::dialog);
    let placed = container(panel)
        .width(Fill)
        .height(Fill)
        .align_x(Horizontal::Right)
        .padding(Padding::ZERO.top(TOOLBAR_HEIGHT - 6.0).right(menu.right));
    iced::widget::stack![
        mouse_area(container(space()).width(Fill).height(Fill))
            .on_press(file(FileMessage::CloseMenu)),
        placed,
    ]
    .into()
}

/// What the Edit menu offers.
pub struct EditMenu<'k> {
    /// Undo's entry, "Undo Delete channel" (or "Undo" for a step without a name), while there
    /// is a step to undo; none when there is not.
    pub undo: Option<String>,
    /// Redo's entry, as Undo's.
    pub redo: Option<String>,
    /// Something is selected in the panel with the keys, so Delete and Duplicate act.
    pub selected: bool,
    /// The piano roll shows a channel's notes, so Select all notes acts.
    pub notes: bool,
    /// The shortcuts, whose keys the rows name.
    pub keys: &'k Keys,
    /// How far the menu's right edge lies from the window's: under the Edit button's
    /// ([`crate::ui::toolbar::Spread::edit_menu_right`]).
    pub right: f32,
}

/// The Edit menu under the toolbar's Edit button, over a backdrop that closes it when clicked:
/// Undo and Redo naming their steps, Delete, Duplicate and Select all notes, each with its key
/// and dim when it would do nothing, then Settings, so the window is found from a menu too.
pub fn edit_menu<'a>(menu: EditMenu) -> Element<'a, Message> {
    let mut list = column![].spacing(2);
    for row in EditRow::ALL {
        let key = |action| menu.keys.first(action);
        let (label, keys, message, acts) = match row {
            EditRow::Undo => (
                menu.undo.clone().unwrap_or_else(|| "Undo".to_owned()),
                key(Action::Undo),
                Message::Undo,
                menu.undo.is_some(),
            ),
            EditRow::Redo => (
                menu.redo.clone().unwrap_or_else(|| "Redo".to_owned()),
                key(Action::Redo),
                Message::Redo,
                menu.redo.is_some(),
            ),
            EditRow::Delete => (
                "Delete".to_owned(),
                key(Action::Delete),
                Message::Delete,
                menu.selected,
            ),
            EditRow::Duplicate => (
                "Duplicate".to_owned(),
                key(Action::Duplicate),
                Message::Duplicate,
                menu.selected,
            ),
            EditRow::SelectAll => (
                "Select all notes".to_owned(),
                key(Action::SelectAll),
                Message::SelectAllNotes,
                menu.notes,
            ),
            EditRow::Settings => (
                "Settings\u{2026}".to_owned(),
                key(Action::Panel(Panel::Settings)),
                Message::TogglePanel(Panel::Settings),
                true,
            ),
        };
        if row == EditRow::Delete || row == EditRow::Settings {
            list = list.push(rule::horizontal(1));
        }
        list = list.push(menu_entry(
            MenuRow::Edit(row),
            label,
            keys,
            acts.then_some(message),
            !acts,
        ));
    }
    let panel = container(list).width(260).padding(6).style(style::dialog);
    let placed = container(panel)
        .width(Fill)
        .height(Fill)
        .align_x(Horizontal::Right)
        .padding(Padding::ZERO.top(TOOLBAR_HEIGHT - 6.0).right(menu.right));
    iced::widget::stack![
        mouse_area(container(space()).width(Fill).height(Fill)).on_press(Message::CloseEditMenu),
        placed,
    ]
    .into()
}

/// What the pattern name's menu offers.
pub struct PatternMenu {
    /// Where the pattern's name starts across the toolbar: the menu opens under it.
    pub x: f32,
    /// Delete can delete: more than one pattern, and no take of notes under way.
    pub can_delete: bool,
}

/// The pattern name's menu: Rename, Delete (dim when it cannot delete) and Time signature…
/// (the pattern's own, [`signature_menu`]). A press anywhere else closes it.
pub fn pattern_menu<'a>(menu: PatternMenu) -> Element<'a, Message> {
    let pattern = |message| Message::Pattern(message);
    let list = column![
        entry(
            "Rename".into(),
            "",
            Some(pattern(PatternMessage::Rename)),
            false
        ),
        entry(
            "Delete".into(),
            "",
            menu.can_delete.then(|| pattern(PatternMessage::Delete)),
            !menu.can_delete
        ),
        entry(
            "Time signature\u{2026}".into(),
            "",
            Some(Message::Signature(SignatureMessage::ForPattern)),
            false
        ),
    ]
    .spacing(2);
    let panel = container(list).width(160).padding(6).style(style::dialog);
    let placed = container(panel)
        .width(Fill)
        .height(Fill)
        .padding(Padding::ZERO.top(TOOLBAR_HEIGHT - 6.0).left(menu.x));
    iced::widget::stack![
        mouse_area(container(space()).width(Fill).height(Fill))
            .on_press(pattern(PatternMessage::CloseMenu)),
        placed,
    ]
    .into()
}

/// A row of the time signature menu: a choice (ticked when it is the one in force), or a note
/// that names what the menu is about.
pub struct SignatureRow {
    label: String,
    message: Option<Message>,
    ticked: bool,
}

impl SignatureRow {
    /// A row a press acts on.
    pub fn act(label: String, message: Message, ticked: bool) -> Self {
        Self {
            label,
            message: Some(message),
            ticked,
        }
    }

    /// A row that only says something.
    pub fn note(label: String) -> Self {
        Self {
            label,
            message: None,
            ticked: false,
        }
    }
}

/// What the time signature menu shows.
pub struct SignatureMenuView {
    /// Where it opens, window coordinates.
    pub at: Point,
    pub rows: Vec<SignatureRow>,
    /// The typed box's text, while "Other…" has opened it.
    pub typed: Option<String>,
}

/// The time signature menu: its rows, the one in force ticked, and under them the typed box
/// "Other…" opens (Enter applies, a wrong one says why in the hint bar). A press anywhere else
/// closes it.
pub fn signature_menu<'a>(menu: SignatureMenuView) -> Element<'a, Message> {
    let mut list = column![].spacing(2);
    for row in menu.rows {
        list = list.push(match row.message {
            Some(message) => entry(
                row.label,
                if row.ticked { "\u{2713}" } else { "" },
                Some(message),
                false,
            ),
            None => container(
                text(row.label)
                    .size(size::LABEL)
                    .color(style::palette().text_secondary),
            )
            .padding([4, 10])
            .into(),
        });
    }
    if let Some(typed) = menu.typed {
        list = list.push(
            text_input("3/4, 6/8 \u{2026}", &typed)
                .id(Id::new(crate::app::SIGNATURE_MENU_INPUT))
                .on_input(|typed| Message::Signature(SignatureMessage::Typed(typed)))
                .on_submit(Message::Signature(SignatureMessage::Submitted))
                .size(size::BODY)
                .padding([4, 8]),
        );
    }
    let panel = container(list).width(170).padding(6).style(style::dialog);
    let placed = container(panel)
        .width(Fill)
        .height(Fill)
        .padding(Padding::ZERO.top(menu.at.y).left(menu.at.x));
    iced::widget::stack![
        mouse_area(container(space()).width(Fill).height(Fill))
            .on_press(Message::Signature(SignatureMessage::Close)),
        placed,
    ]
    .into()
}

/// What a Record menu shows ticked: Notes' or Audio's.
pub struct RecordMenu<'k> {
    /// The menu shown: Notes' (quantize, count-in) or Audio's (what it records, count-in).
    pub kind: TakeKind,
    /// What a take records now, the switch's side.
    pub recording: TakeKind,
    /// What an audio take records.
    pub source: RecordSource,
    /// The input channel the line in records from (the selected one, open), by name; none
    /// records the system's default input.
    pub input: Option<String>,
    pub quantize: bool,
    pub count_in: bool,
    /// The shortcuts, whose keys the rows name.
    pub keys: &'k Keys,
    /// Where the column right of Record starts across the toolbar: the menu opens under it
    /// ([`crate::ui::toolbar::Spread::take_column_x`]).
    pub x: f32,
}

/// A Record menu, on the overlay layer: Notes' ("Record notes", "Quantize notes to snap") or
/// Audio's ("Record audio", "What you hear (master)", "Line in …"), then the count-in. A press
/// anywhere else closes it.
pub fn record_menu<'a>(menu: RecordMenu) -> Element<'a, Message> {
    let record = |message| Some(Message::Record(message));
    let mark = |on: bool| if on { "\u{2713}" } else { "" };
    // Both menus start with what a take records, so a narrow window, whose Record says N or A
    // with no switch beside it, still picks it from either.
    let mut list = column![
        menu_entry(
            MenuRow::Record(RecordRow::Notes),
            "Record notes (into the piano roll)".into(),
            mark(menu.recording == TakeKind::Notes),
            record(RecordMessage::Kind(TakeKind::Notes)),
            false,
        ),
        menu_entry(
            MenuRow::Record(RecordRow::Audio),
            "Record audio (into a sound file)".into(),
            mark(menu.recording == TakeKind::Audio),
            record(RecordMessage::Kind(TakeKind::Audio)),
            false,
        ),
        rule::horizontal(1),
    ]
    .spacing(2);
    match menu.kind {
        TakeKind::Notes => {
            list = list.push(menu_entry(
                MenuRow::Record(RecordRow::Quantize),
                "Quantize notes to snap".into(),
                mark(menu.quantize),
                record(RecordMessage::ToggleQuantize),
                false,
            ));
            list = list.push(menu_entry(
                MenuRow::Record(RecordRow::Capture),
                "Capture the last 30 s played".into(),
                menu.keys.first(Action::Capture),
                record(RecordMessage::Capture),
                false,
            ));
        }
        TakeKind::Audio => {
            let audio = menu.recording == TakeKind::Audio;
            list = list.push(menu_entry(
                MenuRow::Record(RecordRow::Master),
                "What you hear (master)".into(),
                mark(audio && menu.source == RecordSource::Master),
                record(RecordMessage::Source(RecordSource::Master)),
                false,
            ));
            list = list.push(menu_entry(
                MenuRow::Record(RecordRow::LineIn),
                menu.input.as_ref().map_or_else(
                    || "Line in (default input)".to_owned(),
                    |name| format!("Line in: {name}"),
                ),
                mark(audio && menu.source == RecordSource::Input),
                record(RecordMessage::Source(RecordSource::Input)),
                false,
            ));
        }
    }
    let list = list.push(rule::horizontal(1)).push(menu_entry(
        MenuRow::Record(RecordRow::CountIn),
        format!("Count-in {}", mark(menu.count_in)),
        menu.keys.first(Action::CountIn),
        record(RecordMessage::ToggleCountIn),
        false,
    ));
    let panel = container(list).width(260).padding(6).style(style::dialog);
    let placed = container(panel)
        .width(Fill)
        .height(Fill)
        .padding(Padding::ZERO.top(TOOLBAR_HEIGHT - 6.0).left(menu.x));
    iced::widget::stack![
        mouse_area(container(space()).width(Fill).height(Fill))
            .on_press(Message::Record(RecordMessage::CloseMenu)),
        placed,
    ]
    .into()
}
