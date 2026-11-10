//! The piano roll panel: a strip of controls over the ruler (the bar numbers), the note grid
//! (with its key column), its scroll bar and the velocity lane. The canvases draw from
//! [`piano_layout`] positions and the [`Roll`] state, and only report the cursor in their own
//! coordinates; the app turns that into edits. The ruler, the grid and the lane keep their
//! numbers, rows, lines, keys and notes in a cached layer, cleared by the app whenever what
//! they show changes, and the grid draws the playhead and the select box over it every frame.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use dawsome_engine::project::{ChannelId, Note, PatternId, Project};
use dawsome_engine::time::Ticks;
use iced::alignment::{Horizontal, Vertical};
use iced::keyboard::Modifiers;
use iced::widget::canvas::{self, Cache, Event, Frame, Geometry, Path, Stroke, Text};
use iced::widget::{button, column, container, pick_list, pin, responsive, row, stack, text};
use iced::{mouse, Center, Color, Element, Fill, Point, Rectangle, Renderer, Size, Theme};

use crate::app::Message;
use crate::ui::fonts::size;
use crate::ui::grid_rules::{
    self, device_line, first_at_or_after, NUMBER_BAND_HEIGHT, NUMBER_TICK_HEIGHT,
    STRONG_COMPANION_ALPHA,
};
use crate::ui::hint::{hover_area, hover_change, Hover, MenuRow, NoteRow, Pointer, RollPart};
use crate::ui::key_names::key_name;
use crate::ui::live_notes::{self, LiveNotes};
use crate::ui::menu;
use crate::ui::note_gesture::{key_of, velocity_steps, NoteKey, Snap, Target};
use crate::ui::palette::Rgb;
use crate::ui::panels::{Panel, TITLE_GAP};
use crate::ui::piano_layout::{
    self, View, GLYPH_SWITCH, GLYPH_SWITCH_ICON, KEY_COLUMN_WIDTH, LABEL_RIGHT, LANE_HEIGHT,
    MENU_ARROW, MENU_ARROW_GAP, NOTE_BORDER, PLAYER_GAP, PLAYER_SIDE, PLAYHEAD_WIDTH, PLAY_BUTTON,
    PLAY_ICON, ROW_HEIGHT, RULER_HEIGHT, SCALE_FOLDED_WIDTH, SELECTED_BORDER, STRIP_GROUP_GAP,
    STRIP_HEIGHT, STRIP_PADDING, STRIP_SPACING,
};
use crate::ui::pop_out;
use crate::ui::roll::{
    NoteEdit, QuantizeForm, QuantizeMessage, Roll, RollMessage, MENU_FROM_RIGHT,
};
use crate::ui::scale::{Root, Scale, ScaleMode};
use crate::ui::scroll::{self, Scrollbar};
use crate::ui::short_pick_list::{self, natural_width, short_pick_list};
use crate::ui::style;
use crate::ui::text_fit::{self, text_width};
use crate::ui::toolbar::{format_length, glyph_switch, menu_arrow, play_stop_button, Glyph};

/// Keys whose rows are black keys on a piano, by their remainder from 12.
const BLACK_KEYS: [u8; 5] = [1, 3, 6, 8, 10];

/// Two left presses at most this far apart in time and space are a double-click.
const DOUBLE_PRESS_TIME: Duration = Duration::from_millis(400);
const DOUBLE_PRESS_DISTANCE: f32 = 6.0;

/// The grid canvas's memory between events: where the cursor was last seen in a drag, what it
/// is over, and the last left press, to spot a double-click.
#[derive(Debug, Default)]
pub struct GridState {
    pointer: Pointer,
    previous_press: Option<(Point, Instant)>,
}

/// The piano roll's player as its strip and the hints show it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RollPlayer {
    /// The transport plays the roll's channel of its pattern: its Play shows Stop, lit.
    pub playing: bool,
    /// Its Loop: the pattern loops; off, it plays once.
    pub looping: bool,
    /// A take runs: its Play is off, recording being the toolbar's.
    pub recording: bool,
}

/// What the piano roll shows: the selected channel's notes in the selected pattern.
pub struct PianoRoll<'a> {
    pub project: &'a Project,
    pub pattern: Option<PatternId>,
    pub channel: Option<ChannelId>,
    pub roll: &'a Roll,
    /// The keys held now, for the snap a drag previews with (Alt: single ticks).
    pub modifiers: Modifiers,
    /// The tick being played, `None` while stopped.
    pub playhead: Option<Ticks>,
    /// The song tick where the pattern's tick 0 lies for the playhead and the live notes: a
    /// song take's start while it records into the pattern shown, else 0 (the song's tick
    /// within the pattern as it loops).
    pub origin: Ticks,
    pub ruler_cache: &'a Cache,
    pub grid_cache: &'a Cache,
    pub lane_cache: &'a Cache,
    /// The window's device pixels per logical pixel, which the lines are laid on.
    pub scale_factor: f32,
    /// Whether drawn, moved and clicked notes play: the strip's Preview switch.
    pub preview: bool,
    /// Whether the view jumps a page as the playhead leaves it: the strip's Follow switch.
    pub follow: bool,
    /// Whether the clipboard holds notes, so the menu's Paste acts.
    pub clipboard: bool,
    /// The notes played live now, on every channel.
    pub live: &'a LiveNotes,
    /// Whether a take of notes runs: the strip's "Rec notes" light.
    pub recording_notes: bool,
    /// The window's last pixels a menu keeps clear of (the hint bar while it shows).
    pub menu_foot: f32,
    /// The Quantize dialog's choice while it is open: the grid previews it.
    pub quantize: Option<QuantizeForm>,
    /// Whether the roll is in the stack, where its strip ends with the pop-out button
    /// ([`crate::ui::pop_out`]); in its own window it is not.
    pub docked: bool,
    /// The colour of the user's own for the channel's notes and their velocity stems
    /// (Settings → Theme → Piano roll notes); none: the channel's own colour.
    pub note_color: Option<Rgb>,
    /// The strip's Play / Stop and Loop.
    pub player: RollPlayer,
}

/// The notes menu's width: its longest row, "Chop at snap", and a margin.
pub const NOTE_MENU_WIDTH: f32 = MENU_FROM_RIGHT - 2.0 * menu::PADDING - 12.0;

/// The message a notes menu row sends; none when it would do nothing (Cut, Copy and Delete
/// with nothing selected, Paste with an empty clipboard).
pub fn note_row_message(row: NoteRow, selected: bool, clipboard: bool) -> Option<Message> {
    let edit = |edit| Some(Message::NoteEdit(edit));
    match row {
        NoteRow::Cut if selected => edit(NoteEdit::Cut),
        NoteRow::Copy if selected => edit(NoteEdit::Copy),
        NoteRow::Paste if clipboard => edit(NoteEdit::Paste),
        NoteRow::Delete if selected => edit(NoteEdit::Delete),
        NoteRow::Cut | NoteRow::Copy | NoteRow::Paste | NoteRow::Delete => None,
        NoteRow::Duplicate => edit(NoteEdit::Duplicate),
        NoteRow::Quantize => Some(Message::Quantize(QuantizeMessage::Open)),
        NoteRow::Chop => edit(NoteEdit::Chop),
        NoteRow::Glue => edit(NoteEdit::Glue),
        NoteRow::Legato => edit(NoteEdit::Legato),
        NoteRow::Reverse => edit(NoteEdit::Reverse),
        NoteRow::Invert => edit(NoteEdit::Invert),
        NoteRow::Humanize => edit(NoteEdit::Humanize),
    }
}

/// `channel`'s live notes as the grid draws them: each key with the tick it started at, and the
/// playhead their bars grow to, both counted from `origin` (where the pattern's tick 0 lies).
pub(crate) fn live_from(
    live: &LiveNotes,
    channel: ChannelId,
    playhead: Option<Ticks>,
    origin: Ticks,
) -> (Vec<(u8, Option<Ticks>)>, Option<Ticks>) {
    let from_origin = |tick: Ticks| Ticks(tick.0 - origin.0);
    let keys = live
        .of(channel)
        .into_iter()
        .map(|(key, held)| (key, held.tick.map(from_origin)))
        .collect();
    (keys, playhead.map(from_origin))
}

/// A note's start, length and key, as tests compare what the grid draws.
#[cfg(test)]
pub(crate) type Placed = (i64, i64, u8);

/// A note as the canvases draw it.
#[derive(Debug, Clone)]
struct Shown {
    note: Note,
    selected: bool,
}

/// The velocity readout's words while a drag sets `steps`.
fn velocity_words(steps: u8) -> String {
    format!("Vel {steps}")
}

/// The Scale chooser's width: half what a chooser of its longest entry ("Minor pentatonic")
/// takes, whole pixels down, so the strip has room for the roll's Play and Loop; its face cuts
/// a longer name with an ellipsis, its menu shows each whole.
pub fn scale_width() -> f32 {
    static WIDTH: OnceLock<f32> = OnceLock::new();
    *WIDTH.get_or_init(|| {
        (short_pick_list::natural_width(&ScaleMode::ALL, size::LABEL) / 2.0).floor()
    })
}

/// The room the velocity readout keeps: its widest value's (0 to 127), so the Rec light after
/// it stays put as a drag sets the velocity (the type scale's digits are proportional).
fn velocity_width() -> f32 {
    static WIDTH: OnceLock<f32> = OnceLock::new();
    *WIDTH.get_or_init(|| crate::ui::text_fit::widest((0..=127).map(velocity_words), size::LABEL))
}

/// The strip's words, which its fold law measures as the view lays them out. Each character
/// is one the bundled font has (`strip_glyphs` checks), so no width depends on the system's
/// fonts; the Notes menu's arrow is drawn ([`MENU_ARROW`]).
const PREVIEW: &str = "Preview";
const FOLLOW: &str = "Follow";
const FOLD: &str = "Fold";
const NOTES: &str = "Notes";
const LOOP: &str = "Loop";
const REC_NOTES: &str = "\u{25cf} Rec notes";
/// A text button's padding in the strip, (top and bottom, either side).
const BUTTON_PADDING: [f32; 2] = [2.0, 8.0];

/// The width a text button of the strip's takes: its words and its padding either side.
fn text_button_width(words: &str) -> f32 {
    text_width(words, size::LABEL) + 2.0 * BUTTON_PADDING[1]
}

/// The Notes menu button's width: its word, the drawn arrow [`MENU_ARROW_GAP`] after it, and
/// the text buttons' padding either side.
fn notes_button_width() -> f32 {
    text_width(NOTES, size::LABEL) + MENU_ARROW_GAP + MENU_ARROW + 2.0 * BUTTON_PADDING[1]
}

/// The draw length's readout, "Len 0:01:00", after the snap picker until the strip folds it.
fn length_words(length: &str) -> String {
    format!("Len {length}")
}

/// How far the piano roll's strip folds to fit its width, each fold keeping those before it
/// ([`StripFold::of`]). Why folds and not fewer controls: as the toolbar's labels do, the strip
/// says less where the roll is narrow and keeps every control; Play and
/// Loop, the snap picker, Scale and Fold, Notes and the pop-out grip never go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StripFold {
    /// Nothing folded.
    Open,
    /// The Len readout hidden: the snap picker's hint says the length ("Snap: Line · draws
    /// 0:01:00").
    Length,
    /// Preview and Follow 24 px glyph buttons too ([`GLYPH_SWITCH`]: a speaker, an arrow).
    Glyphs,
    /// The Scale chooser's face [`SCALE_FOLDED_WIDTH`] wide too.
    Scale,
    /// The channel's name cut with "…" too, to the room the folded controls leave, never
    /// shorter than its first [`NAME_LEAST_CHARS`] characters and "…" ([`StripShape::shown_name`]);
    /// the name's hint says it whole.
    Name,
}

impl StripFold {
    /// The order the strip folds in, nothing folded first.
    pub const ALL: [StripFold; 5] = [
        StripFold::Open,
        StripFold::Length,
        StripFold::Glyphs,
        StripFold::Scale,
        StripFold::Name,
    ];

    /// The least fold at which the strip fits `shape`'s width; every fold when none does (the
    /// name at its least, the strip's last controls then run past its end).
    pub fn of(shape: &StripShape) -> Self {
        Self::ALL
            .into_iter()
            .find(|&fold| shape.needs(fold) <= shape.width)
            .unwrap_or(StripFold::Name)
    }
}

/// The fewest of its characters a cut channel name keeps, before its "…". Why a floor: a name
/// cut to one or two letters says no more than none, and the hint says it whole.
pub const NAME_LEAST_CHARS: usize = 3;

/// What decides how far the strip folds ([`StripFold::of`]): its width and what it shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StripShape<'s> {
    /// The strip's width: the roll's, docked in the stack or in its own window.
    pub width: f32,
    /// The channel's name, the strip's title.
    pub name: &'s str,
    /// The draw length as the Len readout says it, "0:01:00".
    pub length: &'s str,
    /// A scale is chosen: its root's chooser shows beside it.
    pub scale: bool,
    /// The velocity readout shows: a drag in the lane sets one.
    pub velocity: bool,
    /// The "Rec notes" light shows: a take of notes runs.
    pub recording: bool,
    /// The roll is in the stack: the pop-out grip ends the strip.
    pub docked: bool,
}

impl<'s> StripShape<'s> {
    /// The width the strip takes folded to `fold`: its margins; the name and [`TITLE_GAP`];
    /// the groups [`STRIP_GROUP_GAP`] apart, their controls [`STRIP_SPACING`] apart; docked, a
    /// spacing and the pop-out grip. Each control as wide as the view lays it out: a text by
    /// the app's measure, a text button with its padding, a pick list by
    /// [`natural_width`] (the toolkit sizes one so), the rest by their constants. Folded to the
    /// name, the name at its least ([`least_name`]).
    pub fn needs(&self, fold: StripFold) -> f32 {
        let name = if fold < StripFold::Name {
            Cow::Borrowed(self.name)
        } else {
            least_name(self.name)
        };
        self.beside_name(fold) + text_width(&name, size::PANEL_TITLE)
    }

    /// The channel's name as the strip shows it at `fold`: whole until the strip folds it, then
    /// its longest start with "…" that fits the room the folded controls leave
    /// ([`text_fit::fit`]), never shorter than [`least_name`].
    pub fn shown_name(&self, fold: StripFold) -> Cow<'s, str> {
        if fold < StripFold::Name {
            return Cow::Borrowed(self.name);
        }
        let room = self.width - self.beside_name(fold);
        let measure = |text: &str| text_width(text, size::PANEL_TITLE);
        match text_fit::fit(self.name, room, &measure) {
            Some(fitted) if !fitted.cut || kept_chars(&fitted.text) >= NAME_LEAST_CHARS => {
                fitted.text
            }
            _ => least_name(self.name),
        }
    }

    /// The width of everything in the strip but the channel's name, folded to `fold`.
    fn beside_name(&self, fold: StripFold) -> f32 {
        let apart = |widths: &[f32], gap: f32| {
            widths.iter().sum::<f32>() + gap * widths.len().saturating_sub(1) as f32
        };
        let mut snap = vec![natural_width(&Snap::ALL, size::LABEL)];
        if fold < StripFold::Length {
            snap.push(text_width(&length_words(self.length), size::LABEL));
        }
        let switch = |words| {
            if fold < StripFold::Glyphs {
                text_button_width(words)
            } else {
                GLYPH_SWITCH.width
            }
        };
        let mut scale = vec![scale_face(fold)];
        if self.scale {
            scale.push(natural_width(&Root::ALL, size::LABEL));
        }
        scale.push(text_button_width(FOLD));
        let mut groups = vec![
            apart(&snap, STRIP_SPACING),
            apart(&[switch(PREVIEW), switch(FOLLOW)], STRIP_SPACING),
            apart(&scale, STRIP_SPACING),
            notes_button_width(),
            player_width(),
        ];
        if self.velocity {
            groups.push(velocity_width());
        }
        if self.recording {
            groups.push(text_width(REC_NOTES, size::LABEL));
        }
        let grip = if self.docked {
            STRIP_SPACING + pop_out::SIZE
        } else {
            0.0
        };
        2.0 * STRIP_PADDING + TITLE_GAP + apart(&groups, STRIP_GROUP_GAP) + grip
    }
}

/// `name` at its shortest in the strip: its first [`NAME_LEAST_CHARS`] characters and "…", or
/// the name whole when that is no wider ("lead" is narrower than "lea…").
fn least_name(name: &str) -> Cow<'_, str> {
    let Some((end, _)) = name.char_indices().nth(NAME_LEAST_CHARS) else {
        return Cow::Borrowed(name);
    };
    let cut = format!("{}\u{2026}", name[..end].trim_end());
    let width = |text: &str| text_width(text, size::PANEL_TITLE);
    if width(name) <= width(&cut) {
        Cow::Borrowed(name)
    } else {
        Cow::Owned(cut)
    }
}

/// The characters a cut name keeps before its "…".
fn kept_chars(cut: &str) -> usize {
    cut.trim_end_matches('\u{2026}').chars().count()
}

/// The player's width: Play and Loop [`PLAYER_GAP`] apart, [`PLAYER_SIDE`] either side.
fn player_width() -> f32 {
    2.0 * PLAYER_SIDE + PLAY_BUTTON.width + PLAYER_GAP + text_button_width(LOOP)
}

/// The Scale chooser's width at `fold`: [`scale_width`], or [`SCALE_FOLDED_WIDTH`] once folded.
fn scale_face(fold: StripFold) -> f32 {
    if fold < StripFold::Scale {
        scale_width()
    } else {
        SCALE_FOLDED_WIDTH
    }
}

/// What the piano roll's strip shows, laid out anew for the width the roll gives it
/// ([`StripFold`]).
#[derive(Debug, Clone)]
struct Strip<'a> {
    /// The channel the roll edits, and its name.
    channel: ChannelId,
    name: &'a str,
    snap: Snap,
    /// The draw length, as the Len readout says it.
    length: String,
    preview: bool,
    follow: bool,
    scale: Scale,
    /// The roll's Fold: rows only for the keys that hold notes (and the scale's).
    rows_folded: bool,
    menu_open: bool,
    player: RollPlayer,
    /// The velocity a drag in the lane sets, while one does.
    velocity: Option<u8>,
    recording: bool,
    docked: bool,
}

impl<'a> Strip<'a> {
    /// The channel's name, the strip's title, then [`TITLE_GAP`] on the controls in groups
    /// [`STRIP_GROUP_GAP`] apart: the snap choice (its picker alone, no "Snap" before it: its
    /// entries say what it is, and the hint names it) and the draw length; the
    /// Preview and Follow switches; the scale (its root while one is chosen; the chooser
    /// [`scale_width`] wide) and Fold; Notes; the roll's player, Play / Stop and Loop (right of
    /// Notes: the editing controls first, then the one that hears them). After
    /// them the velocity while a lane drag sets one and the "Rec notes" light, and, docked, the
    /// pop-out button at the right end. Folded as `width` needs ([`StripFold`]): the length into
    /// the snap picker's hint, Preview and Follow to glyphs, the Scale chooser to
    /// [`SCALE_FOLDED_WIDTH`], then the name cut short (its hint says it whole). Why Notes beside Fold (it stood at the right end): the menu is a
    /// control of the roll's like the others, and the strip's end is the pop-out's alone.
    fn view(&self, width: f32) -> Element<'a, Message> {
        let shape = StripShape {
            width,
            name: self.name,
            length: &self.length,
            scale: self.scale.is_on(),
            velocity: self.velocity.is_some(),
            recording: self.recording,
            docked: self.docked,
        };
        let fold = StripFold::of(&shape);
        let name = text(shape.shown_name(fold).into_owned()).size(size::PANEL_TITLE);
        let name = hover_area(
            name,
            Hover::Roll(RollPart::Name {
                channel: self.channel,
            }),
        );
        let group = |controls: Vec<Element<'a, Message>>| {
            row(controls).spacing(STRIP_SPACING).align_y(Center)
        };
        let text_button = |words: &'static str, lit: bool, message: Message| {
            button(text(words).size(size::LABEL))
                .padding(BUTTON_PADDING)
                .style(style::tool_button(lit))
                .on_press(message)
        };
        // Play stays a Stop while the roll plays; it is off while a take runs.
        let player = self.player;
        let play = play_stop_button(
            player.playing,
            (player.playing || !player.recording).then_some(Message::RollPlayStop),
            PLAY_BUTTON,
            PLAY_ICON,
        );
        // As tall as Play, its word in the middle, so the pair shares one middle line.
        let looping = button(
            text(LOOP)
                .size(size::LABEL)
                .height(Fill)
                .align_y(Vertical::Center),
        )
        .padding(BUTTON_PADDING)
        .height(PLAY_BUTTON.height)
        .style(style::tool_button(player.looping))
        .on_press(Message::ToggleRollLoop);
        let player = row![
            hover_area(play, Hover::Roll(RollPart::Play)),
            hover_area(looping, Hover::Roll(RollPart::Loop)),
        ]
        .spacing(PLAYER_GAP)
        .padding([0.0, PLAYER_SIDE])
        .align_y(Center);
        let snap = pick_list(Snap::ALL, Some(self.snap), |snap| {
            Message::Roll(RollMessage::SnapSelected(snap))
        })
        .text_size(size::LABEL)
        .padding([2, 6]);
        let mut snap_and_length = vec![hover_area(snap, Hover::Roll(RollPart::Snap))];
        if fold < StripFold::Length {
            snap_and_length.push(text(length_words(&self.length)).size(size::LABEL).into());
        }
        let switch = |words, glyph, on, message: Message, part| {
            let face: Element<'a, Message> = if fold < StripFold::Glyphs {
                text_button(words, on, message).into()
            } else {
                glyph_switch(glyph, on, message, GLYPH_SWITCH, GLYPH_SWITCH_ICON)
            };
            hover_area(face, Hover::Roll(part))
        };
        let switches = group(vec![
            switch(
                PREVIEW,
                Glyph::Speaker,
                self.preview,
                Message::ToggleNotePreview,
                RollPart::Preview,
            ),
            switch(
                FOLLOW,
                Glyph::Follow,
                self.follow,
                Message::ToggleFollow(Panel::PianoRoll),
                RollPart::Follow,
            ),
        ]);
        // A fixed width, so the strip does not move as the scale changes.
        let scale = short_pick_list(
            &ScaleMode::ALL,
            Some(self.scale.mode),
            |mode| Message::Roll(RollMessage::ScaleSelected(mode)),
            scale_face(fold),
            size::LABEL,
        );
        let mut scale_and_fold = vec![hover_area(scale, Hover::Roll(RollPart::Scale))];
        if self.scale.is_on() {
            let root = pick_list(Root::ALL, Some(self.scale.root), |root| {
                Message::Roll(RollMessage::RootSelected(root))
            })
            .text_size(size::LABEL)
            .padding([2, 6]);
            scale_and_fold.push(hover_area(root, Hover::Roll(RollPart::ScaleRoot)));
        }
        let rows_folded = text_button(
            FOLD,
            self.rows_folded,
            Message::Roll(RollMessage::ToggleFold),
        );
        scale_and_fold.push(hover_area(rows_folded, Hover::Roll(RollPart::Fold)));
        let notes = button(
            row![
                text(NOTES).size(size::LABEL),
                menu_arrow(self.menu_open, MENU_ARROW)
            ]
            .spacing(MENU_ARROW_GAP)
            .align_y(Center),
        )
        .padding(BUTTON_PADDING)
        .style(style::tool_button(self.menu_open))
        .on_press(Message::Roll(RollMessage::MenuButton));
        let mut groups = row![
            group(snap_and_length),
            switches,
            group(scale_and_fold),
            hover_area(notes, Hover::Roll(RollPart::NotesMenu)),
            player,
        ]
        .spacing(STRIP_GROUP_GAP)
        .align_y(Center);
        if let Some(steps) = self.velocity {
            let readout = text(velocity_words(steps))
                .size(size::LABEL)
                .color(style::accent());
            groups = groups.push(
                container(readout)
                    .width(velocity_width())
                    .align_x(Horizontal::Right),
            );
        }
        if self.recording {
            let light = text(REC_NOTES).size(size::LABEL).color(style::RECORD_ARMED);
            groups = groups.push(hover_area(light, Hover::Roll(RollPart::RecNotes)));
        }
        // The title and the controls take the room the pop-out button leaves, so the button
        // stands at the strip's right end, at least a spacing from the last control.
        let mut content = row![row![name, groups]
            .spacing(TITLE_GAP)
            .align_y(Center)
            .width(Fill)]
        .spacing(STRIP_SPACING)
        .align_y(Center);
        if self.docked {
            content = content.push(pop_out::button(Panel::PianoRoll));
        }
        container(content)
            .padding([0.0, STRIP_PADDING])
            .width(Fill)
            .height(STRIP_HEIGHT)
            .align_y(Center)
            .style(style::strip)
            .into()
    }
}

impl<'a> PianoRoll<'a> {
    pub fn view(self) -> Element<'a, Message> {
        let Some(target) = Target::of(self.project, self.pattern, self.channel) else {
            return container(
                text("No channel selected")
                    .size(size::BODY)
                    .color(style::palette().text_secondary),
            )
            .center(Fill)
            .style(style::rack_well)
            .into();
        };
        let rack_row = |id: ChannelId| self.project.channels.keys().position(|&c| c == id);
        // The channel's notes, the stems and what a drag or the Quantize dialog shows of them
        // take the user's colour when there is one; the other channels' ghosts keep theirs, so
        // they still say whose they are.
        let color = self.note_color.map_or_else(
            || style::channel_color(rack_row(target.channel).unwrap_or(0)),
            Rgb::color,
        );
        let ghosts = target
            .notes
            .iter()
            .filter(|note| note.channel != target.channel)
            .map(|note| {
                let color = style::channel_color(rack_row(note.channel).unwrap_or(0));
                (note.clone(), color)
            })
            .collect();
        let (shown, before) = self.shown_notes(&target);
        let end = target.end();
        let (live, song_tick) = live_from(self.live, target.channel, self.playhead, self.origin);
        let ruler = canvas::Canvas::new(Ruler {
            view: self.roll.view,
            scale_factor: self.scale_factor,
            cache: self.ruler_cache,
        })
        .width(Fill)
        .height(RULER_HEIGHT);
        let grid = canvas::Canvas::new(Grid {
            view: self.roll.view,
            end,
            shown: shown.clone(),
            ghosts,
            before,
            color,
            playhead: self
                .playhead
                .map(|tick| Ticks((tick.0 - self.origin.0).rem_euclid(end.0.max(1)))),
            select_box: self.roll.select_box(),
            live,
            song_tick,
            scale: self.roll.scale,
            held: self.roll.grid_held(),
            known_size: self.roll.grid_size(),
            scale_factor: self.scale_factor,
            cache: self.grid_cache,
        })
        .width(Fill)
        .height(Fill);
        let mut layers = vec![Element::from(grid)];
        layers.extend(self.notes_menu());
        let grid = stack(layers).width(Fill).height(Fill);
        let lane = canvas::Canvas::new(Lane {
            view: self.roll.view,
            shown,
            color,
            held: self.roll.lane_held(),
            ramp: self.roll.ramp_line(),
            scale_factor: self.scale_factor,
            cache: self.lane_cache,
        })
        .width(Fill)
        .height(LANE_HEIGHT);
        // The rail the panel's whole width, under the key column too, as the playlist's: the
        // thumb starts at the panel's left edge at the pattern's start.
        let bar = canvas::Canvas::new(Scrollbar {
            span: self.roll.span(Some(&target)),
            message: |message| Message::Roll(RollMessage::Bar(message)),
        })
        .width(Fill)
        .height(scroll::BAR_HEIGHT);
        let name = self
            .project
            .channels
            .get(&target.channel)
            .map_or("", |channel| channel.name.as_str());
        let strip = Strip {
            channel: target.channel,
            name,
            snap: self.roll.snap,
            length: format_length(self.roll.draw_length(), target.signature, self.project.ppq),
            preview: self.preview,
            follow: self.follow,
            scale: self.roll.scale,
            rows_folded: self.roll.fold,
            menu_open: self.roll.menu.is_some(),
            player: self.player,
            velocity: self.roll.dragged_velocity(),
            recording: self.recording_notes,
            docked: self.docked,
        };
        let strip = responsive(move |size| strip.view(size.width)).height(STRIP_HEIGHT);
        column![strip, ruler, grid, bar, lane].into()
    }

    /// The notes menu where it was opened, on the window's top layer: a row per
    /// [`NoteRow::ALL`].
    fn notes_menu(&self) -> Option<Element<'a, Message>> {
        let at = self.roll.menu?;
        let selected = !self.roll.selection.is_empty();
        let rows = NoteRow::ALL.map(|row| {
            let message = note_row_message(row, selected, self.clipboard);
            hover_area(
                menu::row(row.label(), message),
                Hover::Menu(MenuRow::Notes(row)),
            )
        });
        let panel = menu::panel(column(rows), NOTE_MENU_WIDTH);
        Some(
            pin(menu::floating(panel, self.menu_foot))
                .x(at.x)
                .y(at.y)
                .into(),
        )
    }

    /// The channel's notes as they would be if the held drag were released now, or the open
    /// Quantize dialog applied; and, for the dialog, the notes it takes out, where they are now.
    fn shown_notes(&self, target: &Target) -> (Vec<Shown>, Vec<Note>) {
        let quantizing = || {
            let (change, selection) = self.roll.quantize_preview(target, self.quantize?)?;
            Some((change, selection, true))
        };
        let preview = self
            .roll
            .preview(target, self.modifiers)
            .map(|(change, selection)| (change, selection, false))
            .or_else(quantizing);
        let Some((change, selection, quantize)) = preview else {
            let shown = target
                .channel_notes()
                .map(|note| Shown {
                    selected: self.roll.selection.contains(&key_of(note)),
                    note: note.clone(),
                })
                .collect();
            return (shown, Vec::new());
        };
        let removed: BTreeSet<NoteKey> = change.remove.iter().map(key_of).collect();
        let mut notes: Vec<Note> = target
            .channel_notes()
            .filter(|note| !removed.contains(&key_of(note)))
            .cloned()
            .chain(change.add)
            .collect();
        notes.sort_by_key(|note| (note.start, note.key));
        let before = if quantize { change.remove } else { Vec::new() };
        let shown = notes
            .into_iter()
            .map(|note| Shown {
                selected: selection.contains(&key_of(&note)),
                note,
            })
            .collect();
        (shown, before)
    }

    /// The channel's notes as the grid draws them and, while the Quantize dialog is open, the
    /// notes it moves where they were: each as (start, length, key), by start then key.
    #[cfg(test)]
    pub(crate) fn drawn(&self) -> (Vec<Placed>, Vec<Placed>) {
        let Some(target) = Target::of(self.project, self.pattern, self.channel) else {
            return (Vec::new(), Vec::new());
        };
        let (shown, before) = self.shown_notes(&target);
        let place = |note: &Note| (note.start.0, note.length.0, note.key);
        let mut before: Vec<_> = before.iter().map(place).collect();
        before.sort_by_key(|&(start, _, key)| (start, key));
        (
            shown.iter().map(|shown| place(&shown.note)).collect(),
            before,
        )
    }
}

fn is_black(key: u8) -> bool {
    BLACK_KEYS.contains(&(key % 12))
}

fn row_color(key: u8) -> Color {
    if is_black(key) {
        style::palette().roll_row_black
    } else {
        style::palette().roll_row_white
    }
}

/// `key`'s row color with `scale` chosen: darker outside it.
fn scaled_row(key: u8, scale: Scale) -> Color {
    let row = row_color(key);
    if scale.holds(key) {
        row
    } else {
        style::out_of_scale(row)
    }
}

/// What `key`'s row is filled with: its color with `scale` chosen ([`scaled_row`]), lighter in
/// a `shaded` bar ([`style::shaded`]), and that dimmed `past_end` of the pattern
/// ([`style::dimmed`]), as the lines there are, so the bars past the end are the pattern's at
/// 0.6, their shading too: quieter than the pattern, not busier.
fn row_fill(key: u8, scale: Scale, past_end: bool, shaded: bool) -> Color {
    let row = scaled_row(key, scale);
    let row = if shaded { style::shaded(row) } else { row };
    if past_end {
        style::dimmed(row)
    } else {
        row
    }
}

/// The keys whose rows a grid `height` tall holds whole, top row first: rows n = 0..floor(h/12)
/// ([`piano_layout::visible_rows`]), from y = 12n. The ground under them shows no key.
fn visible_keys(view: &View, height: f32) -> impl Iterator<Item = u8> + '_ {
    let rows = u16::from(piano_layout::visible_rows(height));
    (0..rows).filter_map(move |row| {
        u8::try_from(view.key_at(f32::from(row) * ROW_HEIGHT))
            .ok()
            .filter(|&key| key <= dawsome_engine::project::MAX_KEY)
    })
}

/// Fills `rect` with `edge`, then all but a `border`-wide rim of it with `fill`.
fn fill_bordered(frame: &mut Frame, rect: Rectangle, border: f32, edge: Color, fill: Color) {
    frame.fill_rectangle(rect.position(), rect.size(), edge);
    if rect.width > 2.0 * border && rect.height > 2.0 * border {
        frame.fill_rectangle(
            Point::new(rect.x + border, rect.y + border),
            Size::new(rect.width - 2.0 * border, rect.height - 2.0 * border),
            fill,
        );
    }
}

/// The note grid with its key column.
struct Grid<'a> {
    view: View,
    /// The pattern's end: the rows and lines past it are dimmed.
    end: Ticks,
    shown: Vec<Shown>,
    /// The other channels' notes in the pattern, with their rack colors.
    ghosts: Vec<(Note, Color)>,
    /// The notes the open Quantize dialog moves, where they are in the document: drawn as
    /// faint ghosts of the channel's color with a firmer rim, under where they go.
    before: Vec<Note>,
    color: Color,
    playhead: Option<Ticks>,
    select_box: Option<(Point, Point)>,
    /// The shown channel's notes played live: each key and the song tick it started at.
    live: Vec<(u8, Option<Ticks>)>,
    /// The song's tick while it plays, where live bars grow to.
    song_tick: Option<Ticks>,
    /// The scale whose outside keys shade darker.
    scale: Scale,
    /// Whether a move, resize or box select is held, so drags and the release are reported
    /// wherever the cursor is.
    held: bool,
    /// The size the app last heard of; a different size is reported.
    known_size: Size,
    /// The window's device pixels per logical pixel, which the lines are laid on.
    scale_factor: f32,
    cache: &'a Cache,
}

impl Grid<'_> {
    fn draw_body(&self, frame: &mut Frame) {
        let size = frame.size();
        let view = &self.view;
        let keys = || visible_keys(view, size.height);
        // Under the whole rows, the roll's ground: the scroll bar's trough and the ruler's.
        let rows_bottom = piano_layout::rows_bottom(size.height);
        frame.fill_rectangle(
            Point::new(0.0, rows_bottom),
            Size::new(size.width, size.height - rows_bottom),
            style::palette().bg,
        );
        // A note whose row starts in the ground or under it is not drawn (rows are whole
        // multiples of 12, so a row starting above it ends by it).
        let in_rows = |note: &Note| view.row_top(note.key) < rows_bottom;
        let shows = |note: &Note| view.rows.shows(note.key) && in_rows(note);
        let timeline_end = size.width.max(KEY_COLUMN_WIDTH);
        let end_x = view.x_of(self.end).clamp(KEY_COLUMN_WIDTH, timeline_end);
        // The shaded bars' edges on the device pixels their bar lines start on, so the lines
        // cover them and no blended pixel shows beside a line.
        let on_line = |x: f32| device_line(x, self.scale_factor).0;
        let shaded: Vec<(f32, f32)> = piano_layout::shaded_spans(view, size.width)
            .map(|(left, right)| (on_line(left), on_line(right)))
            .collect();
        for key in keys() {
            let y = view.row_top(key);
            let mut fill = |left: f32, right: f32, past_end: bool, shade: bool| {
                if right > left {
                    let color = row_fill(key, self.scale, past_end, shade);
                    frame.fill_rectangle(
                        Point::new(left, y),
                        Size::new(right - left, ROW_HEIGHT),
                        color,
                    );
                }
            };
            fill(KEY_COLUMN_WIDTH, end_x, false, false);
            fill(end_x, timeline_end, true, false);
            for &(left, right) in &shaded {
                fill(left, right.min(end_x), false, true);
                fill(left.max(end_x), right, true, true);
            }
        }
        // The lines run down the whole rows, not the ground under them.
        self.draw_lines(frame, Size::new(size.width, rows_bottom));
        let ghost_row = |note: &Note| {
            let row = scaled_row(note.key, self.scale);
            if note.start >= self.end {
                style::dimmed(row)
            } else {
                row
            }
        };
        for (note, color) in &self.ghosts {
            // Another channel's note on a key the fold leaves out, or under the whole rows, has
            // no row to show in.
            if !shows(note) {
                continue;
            }
            let rect = view.note_rect(note);
            let fill = style::ghost(*color, ghost_row(note));
            frame.fill_rectangle(rect.position(), rect.size(), fill);
        }
        for note in &self.before {
            if !shows(note) {
                continue;
            }
            let row = ghost_row(note);
            let rim = style::ghost_rim(self.color, row);
            let fill = style::faint_ghost(self.color, row);
            fill_bordered(frame, view.note_rect(note), NOTE_BORDER, rim, fill);
        }
        for shown in self.shown.iter().filter(|shown| in_rows(&shown.note)) {
            let (edge, border) = if shown.selected {
                (style::accent(), SELECTED_BORDER)
            } else {
                (style::palette().button_edge, NOTE_BORDER)
            };
            fill_bordered(frame, view.note_rect(&shown.note), border, edge, self.color);
        }
        // The key column goes on top, covering notes scrolled to the left of the timeline, drawn
        // as a keyboard: every row a white key's face, a black key's row black over the left of
        // it ([`piano_layout::black_key_width`]) with the white showing beside it. A row's faces
        // shade as its key does with a scale chosen: the row is the key a press plays.
        let black_width = piano_layout::black_key_width(KEY_COLUMN_WIDTH);
        for key in keys() {
            let y = view.row_top(key);
            let face = |color| {
                if self.scale.holds(key) {
                    color
                } else {
                    style::out_of_scale(color)
                }
            };
            frame.fill_rectangle(
                Point::new(0.0, y),
                Size::new(KEY_COLUMN_WIDTH, ROW_HEIGHT),
                face(style::palette().key_white),
            );
            if is_black(key) {
                frame.fill_rectangle(
                    Point::new(0.0, y),
                    Size::new(black_width, ROW_HEIGHT),
                    face(style::palette().key_black),
                );
            }
            frame.fill_rectangle(
                Point::new(0.0, y + ROW_HEIGHT - 1.0),
                Size::new(KEY_COLUMN_WIDTH, 1.0),
                style::palette().key_edge,
            );
            if key % 12 == 0 {
                frame.fill_text(Text {
                    content: key_name(key),
                    position: Point::new(LABEL_RIGHT, y + ROW_HEIGHT / 2.0),
                    color: style::palette().key_label,
                    size: size::DENSE.into(),
                    align_x: Horizontal::Right.into(),
                    align_y: Vertical::Center,
                    ..Text::default()
                });
            }
        }
    }

    /// What the hint bar names at point `at` of the grid `height` tall: the key column's key, a
    /// note (its key, velocity and whether on its right edge) or the empty grid's row.
    fn hover_at(&self, at: Point, height: f32) -> Option<Hover> {
        let notes = self.shown.iter().map(|shown| &shown.note);
        grid_hover(&self.view, notes, at, height)
    }

    /// The lines through the grid ([`grid_lines`]), 1 px on whole device pixels, `size.height`
    /// tall.
    fn draw_lines(&self, frame: &mut Frame, size: Size) {
        let view = &self.view;
        for line in grid_lines(view, self.end, view.tick_at(size.width)) {
            let (left, width) = device_line(view.x_of(line.tick), self.scale_factor);
            frame.fill_rectangle(
                Point::new(left, 0.0),
                Size::new(width, size.height),
                line.color,
            );
            if let Some(companion) = line.companion {
                // On the device pixels right after the line's: snapped again, the sum could
                // floor back onto the line's last pixel.
                frame.fill_rectangle(
                    Point::new(left + width, 0.0),
                    Size::new(width, size.height),
                    companion,
                );
            }
        }
    }
}

/// A line through the grid: the tick it lies on, its color, and for a strong bar line the
/// fainter one drawn right of it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct GridLine {
    tick: Ticks,
    color: Color,
    companion: Option<Color>,
}

/// A grid line's color: `base` at `alpha`, through [`style::dimmed`] `past_end` of the pattern
/// as the rows there are, so a line keeps its strength against the row it lies on. Why: drawn
/// undimmed over the dimmed rows, the lines past the end stood out more than the pattern's own.
fn line_color(base: Color, alpha: f32, past_end: bool) -> Color {
    let base = if past_end { style::dimmed(base) } else { base };
    Color { a: alpha, ..base }
}

/// The lines through a grid showing `view` up to tick `last`, left to right within each kind,
/// by the rules the playlist's lanes draw theirs ([`grid_rules`]): half steps and steps in
/// `line_soft` and beats in `grid_beat`, each at the strength how far apart they are gives it
/// ([`View::line_levels`]; none while it is 0) and none where a coarser line lies, then a line at
/// every bar in `bar_line`, the one starting every four bars ([`grid_rules::strong`]) with a
/// fainter one right of it. Every line after the pattern's `end` is dimmed ([`line_color`]);
/// the line on the end itself closes the pattern and is not.
fn grid_lines(view: &View, end: Ticks, last: Ticks) -> Vec<GridLine> {
    let palette = style::palette();
    let bar = view.bar().0.max(1);
    let levels = view.line_levels();
    let bases = [palette.line_soft, palette.line_soft, palette.grid_beat];
    let mut lines = Vec::new();
    for (index, (&(unit, alpha), base)) in levels.iter().zip(bases).enumerate() {
        if alpha <= 0.0 {
            continue;
        }
        let unit = unit.0.max(1);
        // The next coarser level's lines lie on these ticks too and are drawn by it.
        let coarser = levels
            .get(index + 1)
            .map_or(bar, |&(next, _)| next.0.max(1));
        let mut tick = first_at_or_after(view.scroll.0, unit);
        while tick <= last.0 {
            if tick % coarser != 0 {
                lines.push(GridLine {
                    tick: Ticks(tick),
                    color: line_color(base, alpha, tick > end.0),
                    companion: None,
                });
            }
            tick += unit;
        }
    }
    let mut tick = first_at_or_after(view.scroll.0, bar);
    while tick <= last.0 {
        let past_end = tick > end.0;
        lines.push(GridLine {
            tick: Ticks(tick),
            color: line_color(palette.bar_line, 1.0, past_end),
            companion: grid_rules::strong(tick / bar)
                .then(|| line_color(palette.bar_line, STRONG_COMPANION_ALPHA, past_end)),
        });
        tick += bar;
    }
    lines
}

/// What the hint bar names at grid point `at` of a grid `grid_height` tall showing `view` with
/// the channel's `notes`: the key column's key, a note (its key, velocity and whether on its
/// right edge) or the empty grid's row; nothing above or below the keyboard, nor in the ground
/// under the whole rows ([`View::key_in_rows`]).
pub fn grid_hover<'n>(
    view: &View,
    notes: impl DoubleEndedIterator<Item = &'n Note>,
    at: Point,
    grid_height: f32,
) -> Option<Hover> {
    let key = view.key_in_rows(at.y, grid_height)?;
    if at.x < KEY_COLUMN_WIDTH {
        return Some(Hover::Roll(RollPart::Keys { key }));
    }
    let part = match piano_layout::note_under(view, notes, at.x, at.y) {
        Some((note, part)) => RollPart::Note {
            key: note.key,
            velocity: velocity_steps(note.velocity),
            edge: part == piano_layout::NotePart::RightEdge,
        },
        None => RollPart::Empty { key },
    };
    Some(Hover::Roll(part))
}

impl canvas::Program<Message> for Grid<'_> {
    type State = GridState;

    fn update(
        &self,
        state: &mut GridState,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let roll = |message| canvas::Action::publish(Message::Roll(message));
        let local = |position: Point| Point::new(position.x - bounds.x, position.y - bounds.y);
        let input = match event {
            Event::Mouse(mouse::Event::ButtonPressed(
                button @ (mouse::Button::Left | mouse::Button::Right),
            )) => cursor.position_in(bounds).map(|at| {
                state.pointer.last = Some(at);
                let left = *button == mouse::Button::Left;
                let now = crate::ui::clock::now();
                let double = left
                    && state.previous_press.is_some_and(|(previous, time)| {
                        now.duration_since(time) <= DOUBLE_PRESS_TIME
                            && previous.distance(at) <= DOUBLE_PRESS_DISTANCE
                    });
                // A double-click is not the first of another one.
                state.previous_press = (left && !double).then_some((at, now));
                roll(RollMessage::GridPressed {
                    at,
                    button: *button,
                    double,
                })
                .and_capture()
            }),
            Event::Mouse(mouse::Event::CursorMoved { position }) if self.held => {
                let at = local(*position);
                state.pointer.last = Some(at);
                Some(roll(RollMessage::GridDragged(at)))
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let now = cursor
                    .position_in(bounds)
                    .and_then(|at| self.hover_at(at, bounds.height));
                hover_change(&mut state.pointer.hover, now)
            }
            // The window says only this when the cursor leaves it: the hover goes, so the roll
            // does not keep the keys it gives.
            Event::Mouse(mouse::Event::CursorLeft) => hover_change(&mut state.pointer.hover, None),
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if self.held => cursor
                .position()
                .map(local)
                .or(state.pointer.last)
                .map(|at| roll(RollMessage::GridReleased(at))),
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => cursor
                .position_in(bounds)
                .map(|at| roll(RollMessage::GridScrolled { at, delta: *delta }).and_capture()),
            _ => None,
        };
        // A new size is reported on the next event that carries no input, so no press is lost.
        input.or_else(|| {
            (bounds.size() != self.known_size)
                .then(|| roll(RollMessage::GridResized(bounds.size())))
        })
    }

    fn draw(
        &self,
        _state: &GridState,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let body = self
            .cache
            .draw(renderer, bounds.size(), |frame| self.draw_body(frame));
        let mut overlay = Frame::new(renderer, bounds.size());
        let rows_bottom = piano_layout::rows_bottom(bounds.height);
        if let Some(tick) = self.playhead {
            let x = self.view.x_of(tick);
            if x >= KEY_COLUMN_WIDTH {
                overlay.fill_rectangle(
                    Point::new(x, 0.0),
                    Size::new(PLAYHEAD_WIDTH, rows_bottom),
                    style::accent(),
                );
            }
        }
        // Live notes: the key lit in the key column, and the bar (or the strip's mark).
        let live = Color {
            a: 0.55,
            ..style::accent()
        };
        for &(key, started) in &self.live {
            if !self.view.shows_row(key, bounds.height) {
                continue;
            }
            let lit = live_notes::lit_key(&self.view, key);
            overlay.fill_rectangle(lit.position(), lit.size(), style::accent());
            for bar in live_notes::bars(&self.view, self.end, self.song_tick, key, started) {
                overlay.fill_rectangle(bar.position(), bar.size(), live);
            }
        }
        if let Some((from, to)) = self.select_box {
            let corner = Point::new(from.x.min(to.x), from.y.min(to.y));
            let size = Size::new((from.x - to.x).abs(), (from.y - to.y).abs());
            overlay.stroke(
                &Path::rectangle(corner, size),
                Stroke::default()
                    .with_color(style::accent())
                    .with_width(1.0),
            );
        }
        vec![body, overlay.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _state: &GridState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if self.held {
            return mouse::Interaction::Grabbing;
        }
        let Some(at) = cursor.position_in(bounds) else {
            return mouse::Interaction::default();
        };
        if at.x < KEY_COLUMN_WIDTH || at.y >= piano_layout::rows_bottom(bounds.height) {
            return mouse::Interaction::default();
        }
        let notes = self.shown.iter().map(|shown| &shown.note);
        match piano_layout::note_under(&self.view, notes, at.x, at.y) {
            Some((_, piano_layout::NotePart::RightEdge)) => {
                mouse::Interaction::ResizingHorizontally
            }
            Some(_) => mouse::Interaction::Grab,
            None => mouse::Interaction::Crosshair,
        }
    }
}

/// The ruler over the grid: the bar numbers, as the playlist's ruler writes them, over the
/// bars' lines, and a tick at each other beat at the band's foot ([`beat_tick_color`]); the
/// wheel over it does what it does over the grid.
struct Ruler<'a> {
    view: View,
    /// The window's device pixels per logical pixel, which the numbers' ticks are laid on.
    scale_factor: f32,
    cache: &'a Cache,
}

impl canvas::Program<Message> for Ruler<'_> {
    type State = ();

    fn update(
        &self,
        _state: &mut (),
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let Event::Mouse(mouse::Event::WheelScrolled { delta }) = event else {
            return None;
        };
        let at = cursor.position_in(bounds)?;
        let message = RollMessage::RulerScrolled { at, delta: *delta };
        Some(canvas::Action::publish(Message::Roll(message)).and_capture())
    }

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let numbers = self.cache.draw(renderer, bounds.size(), |frame| {
            frame.fill_rectangle(Point::ORIGIN, frame.size(), style::palette().bg);
            if let Some(color) = beat_tick_color(&self.view) {
                for x in piano_layout::visible_beats(&self.view, frame.width()) {
                    let (left, width) = device_line(x, self.scale_factor);
                    frame.fill_rectangle(
                        Point::new(left, NUMBER_BAND_HEIGHT - NUMBER_TICK_HEIGHT),
                        Size::new(width, NUMBER_TICK_HEIGHT),
                        color,
                    );
                }
            }
            let bars: Vec<(i64, f32)> =
                piano_layout::visible_bars(&self.view, frame.width()).collect();
            let px_per_bar = self.view.px_per_bar();
            grid_rules::draw_bar_numbers(frame, &bars, px_per_bar, self.scale_factor);
        });
        vec![numbers]
    }
}

/// The color of the ruler's beat ticks at `view`'s zoom: the grid's beat lines' at their
/// strength ([`View::beat_alpha`]), so the ticks fade as the lines do; none while those are not
/// drawn.
fn beat_tick_color(view: &View) -> Option<Color> {
    let alpha = view.beat_alpha();
    (alpha > 0.0).then(|| Color {
        a: alpha,
        ..style::palette().grid_beat
    })
}

/// The velocity lane: a stem per note of the channel, as tall as its velocity, on the meters'
/// dark ground between two lit edges ([`lane_edges`]).
struct Lane<'a> {
    view: View,
    shown: Vec<Shown>,
    color: Color,
    /// Whether a velocity drag is held, so drags and the release are reported anywhere.
    held: bool,
    /// The ramp being drawn (Alt+drag), end to end.
    ramp: Option<(Point, Point)>,
    /// The window's device pixels per logical pixel, which the edges are laid on.
    scale_factor: f32,
    cache: &'a Cache,
}

/// The velocity lane's two edges for a lane whose top is at window y `top`, each as (y, height)
/// in lane coordinates: one device pixel row per whole scale step (as [`device_line`] is wide),
/// on whole device pixels inside the lane, the top edge on its first rows and the foot edge on
/// its last, over the stems. Why edges: the lane's dark ground met the grid's scroll bar and the
/// hint bar under it without a line, so a short stem standing on the hint bar read as cut. Why
/// over the lane's own rows: no position, stem or press moves. Why on device pixels: a logical
/// pixel at a fractional scale straddles two rows, which the renderer blends into the grounds
/// beside it.
fn lane_edges(top: f32, scale_factor: f32) -> [(f32, f32); 2] {
    let scale = if scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    // Within a thousandth of a device row counts as on it: 812.8 logical px times 1.25 is
    // 1015.99998 in single precision, the 1016th row's edge.
    const ON_ROW: f32 = 1e-3;
    let rows = scale.round().max(1.0);
    let first = (top * scale - ON_ROW).ceil();
    let last = ((top + LANE_HEIGHT) * scale + ON_ROW).floor() - rows;
    [first, last].map(|row| (row / scale - top, rows / scale))
}

impl canvas::Program<Message> for Lane<'_> {
    type State = Pointer;

    fn update(
        &self,
        state: &mut Pointer,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let roll = |message| canvas::Action::publish(Message::Roll(message));
        let local = |position: Point| Point::new(position.x - bounds.x, position.y - bounds.y);
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                cursor.position_in(bounds).map(|at| {
                    state.last = Some(at);
                    roll(RollMessage::LanePressed(at)).and_capture()
                })
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) if self.held => {
                let at = local(*position);
                state.last = Some(at);
                Some(roll(RollMessage::LaneDragged(at)))
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let notes = self.shown.iter().map(|shown| &shown.note);
                let now = cursor
                    .position_in(bounds)
                    .and_then(|at| piano_layout::stem_under(&self.view, notes, at.x))
                    .map(|_| Hover::Roll(RollPart::Stem));
                hover_change(&mut state.hover, now)
            }
            Event::Mouse(mouse::Event::CursorLeft) => hover_change(&mut state.hover, None),
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if self.held => cursor
                .position()
                .map(local)
                .or(state.last)
                .map(|at| roll(RollMessage::LaneReleased(at))),
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &Pointer,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let lane = self.cache.draw(renderer, bounds.size(), |frame| {
            frame.fill_rectangle(Point::ORIGIN, frame.size(), style::palette().meter_ground);
            // In the notes' order, by start then key: the stems of a chord stand in one
            // column, the higher key's drawn last, over the others.
            for shown in &self.shown {
                let steps = velocity_steps(shown.note.velocity);
                // A note wholly left of the timeline has no stem; one scrolled half out of view
                // has its stem at the timeline's left edge.
                let Some(stem) = self.view.stem_rect(&shown.note, steps) else {
                    continue;
                };
                let color = if shown.selected {
                    style::accent()
                } else {
                    self.color
                };
                frame.fill_rectangle(stem.position(), stem.size(), color);
            }
        });
        // Drawn every frame, not cached: they follow the lane's place in the window, which a
        // resize can move while its size stays.
        let mut over = Frame::new(renderer, bounds.size());
        for (y, height) in lane_edges(bounds.y, self.scale_factor) {
            over.fill_rectangle(
                Point::new(0.0, y),
                Size::new(bounds.width, height),
                style::palette().line_strong,
            );
        }
        if let Some((from, to)) = self.ramp {
            over.stroke(
                &Path::line(from, to),
                Stroke::default()
                    .with_color(style::accent())
                    .with_width(2.0),
            );
        }
        vec![lane, over.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _state: &Pointer,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if self.held || cursor.is_over(bounds) {
            mouse::Interaction::ResizingVertically
        } else {
            mouse::Interaction::default()
        }
    }
}

#[cfg(test)]
mod strip_glyphs;

#[cfg(test)]
mod tests {
    use super::*;

    // The strip's fold law by hand, from the font's own tables (gen_u1_expected.py's advances:
    // each text's width at 12 px, the name's at 14) and the strip's constants, for p3_roll's
    // "lead" with the draw length 0:01:00, docked:
    //   margins 12 + 12, "lead" 27.986, the title gap 48, the grip a spacing and 24: 133.986
    //   the snap picker, "None" 30.322 + the arrow 12 + padding 18 = 60.322, then a spacing and
    //     "Len 0:01:00" 66.322: 136.645 (the length folded: 60.322)
    //   Preview 45.645 + 16 and Follow 37.107 + 16, a spacing apart: 124.752 (as glyphs:
    //     24 + 10 + 24 = 58)
    //   the Scale chooser 62 (folded: 48), a spacing, Fold 24.533 + 16: 112.533 (98.533); a
    //     scale chosen adds its root's chooser, "G#" 16.553 + 30, and a spacing: 169.086
    //     (155.086)
    //   Notes, "Notes" 33.492 (u1_font.json's notes_12px), the drawn arrow's 8 px square 4
    //     after it, + 16: 61.492
    //   the player: 5 of plain room either side round Play 26 and Loop 28.523 + 16, 4 apart:
    //     10 + 26 + 4 + 44.523 = 84.523 (the room a box round them took the day it stood there;
    //     before it Play and Loop stood a spacing, 10, apart: 80.523, 4 less)
    //   four group gaps: 80
    // So the strip needs, folded to nothing / the length / the glyphs / the scale / the name too
    // ("lead" is its own least: "lea…", 31.51, is wider, so the name fold needs what the scale
    // fold does):
    //   no scale        733.93, 657.61, 590.86, 576.86, 576.86
    //   a scale + root  790.48, 714.16, 647.41, 633.41, 633.41
    // and folds, the least fold that fits:
    //   | strip | no scale | scale + root |
    //   | 1000  | Open     | Open         |
    //   | 700   | Length   | Glyphs       |  733.93 > 700 >= 657.61; 714.16 > 700 >= 647.41
    //   | 640   | Glyphs   | Scale        |  657.61 > 640 >= 590.86; 647.41 > 640 >= 633.41
    // In its own window (no grip: 34 less) it needs 699.93, 623.61, 556.86, 542.86, 542.86 with
    // no scale and 756.48, 680.16, 613.41, 599.41, 599.41 with one: at its least width, 608
    // (crate::desktop::ROLL_WINDOW_MIN_WIDTH), the length and the glyphs fold with no scale
    // (623.61 > 608 >= 556.86) and the scale too with one (613.41 > 608 >= 599.41); at the
    // panels' 480, which it had, no fold fits and the strip is every fold. A longer name, in
    // its own window with a scale and its root, every control folded: they take 599.41 − 27.99
    // = 571.42 beside the name, which at 600 leaves 28.58 for it and at 608 36.58; the name is
    // cut to its longest start with "…" that fits, at 14 px, never under its first three
    // characters and "…". A text holding "…" is shaped with the font's pair kerning (plain
    // ASCII is not), and Inter's GPOS kerns K before i by +32 of its 2048 units, 0.22 px at 14
    // (no pair in "Bas…" is kerned):
    //   "Kick 909" (58.61; folded to the scale 630.04): "Kick…" 40.58 + 0.22 fits neither, "Kic…"
    //     32.89 + 0.22 = 33.11 fits 608 and not 600, its least either way: 604.54 in all, 4.54
    //     past 600, 3.46 to spare at 608
    //   "Bass 808 low" (89.07; 660.50): "Bas…" 36.51, its least, fits 608 (0.07 to spare) but
    //     not 600, where it runs 607.93, 7.93 past (a shorter start fits only under the floor)
    // so the window's least width is 608, 607.93 up to a whole pixel (604 before the player
    // took 4 px more). A velocity readout or the Rec light adds its width and a group's gap.
    // Every width here is the bundled font's: the strip's words hold no character Inter lacks
    // (strip_glyphs.rs checks them against its cmap), so the table is the same on every system.
    // (Pins moved 0.714 px wider: the Notes label was "Notes ▾", whose "▾" Inter has not, so
    // the system's face drew it, 7.911 px on this Linux, and the pins took that width; Windows'
    // face drew it over 0.78 px wider and "Bass 808 low" overran 608 there.)
    #[test]
    fn the_strip_folds_by_its_width() {
        let shape = |width, scale, docked| StripShape {
            width,
            name: "lead",
            length: "0:01:00",
            scale,
            velocity: false,
            recording: false,
            docked,
        };
        assert!(
            (notes_button_width() - (33.492_188 + 4.0 + 8.0 + 16.0)).abs() < 0.001,
            "Notes is {} wide",
            notes_button_width()
        );
        let needs =
            |scale, docked| StripFold::ALL.map(|fold| shape(0.0, scale, docked).needs(fold));
        let near = |found: [f32; 5], wanted: [f32; 5]| {
            found
                .iter()
                .zip(wanted)
                .all(|(found, wanted)| (found - wanted).abs() < 0.01)
        };
        let (plain, scaled) = (needs(false, true), needs(true, true));
        assert!(
            near(plain, [733.93, 657.61, 590.86, 576.86, 576.86]),
            "{plain:?}"
        );
        assert!(
            near(scaled, [790.48, 714.16, 647.41, 633.41, 633.41]),
            "{scaled:?}"
        );
        use StripFold::*;
        for (width, without, with) in [
            (1000.0, Open, Open),
            (700.0, Length, Glyphs),
            (640.0, Glyphs, Scale),
        ] {
            assert_eq!(
                StripFold::of(&shape(width, false, true)),
                without,
                "{width}"
            );
            assert_eq!(
                StripFold::of(&shape(width, true, true)),
                with,
                "{width}, a scale"
            );
        }
        let (plain, scaled) = (needs(false, false), needs(true, false));
        assert!(
            near(plain, [699.93, 623.61, 556.86, 542.86, 542.86]),
            "{plain:?}"
        );
        assert!(
            near(scaled, [756.48, 680.16, 613.41, 599.41, 599.41]),
            "{scaled:?}"
        );
        let least = crate::desktop::ROLL_WINDOW_MIN_WIDTH;
        assert_eq!(least, 608.0);
        assert_eq!(StripFold::of(&shape(least, false, false)), Glyphs, "608");
        assert_eq!(
            StripFold::of(&shape(least, true, false)),
            Scale,
            "608, a scale"
        );
        for scale in [false, true] {
            let at_480 = shape(480.0, scale, false);
            assert_eq!(StripFold::of(&at_480), Name, "480");
            assert_eq!(at_480.shown_name(Name), "lead", "lead is its own least");
        }
        let named = |name, width| StripShape {
            name,
            ..shape(width, true, false)
        };
        for (name, width, shown, total) in [
            ("Kick 909", 600.0, "Kic\u{2026}", 604.54),
            ("Kick 909", 608.0, "Kic\u{2026}", 604.54),
            ("Bass 808 low", 600.0, "Bas\u{2026}", 607.93),
            ("Bass 808 low", 608.0, "Bas\u{2026}", 607.93),
        ] {
            let case = format!("{name} at {width}");
            let named = named(name, width);
            assert!(named.needs(Scale) > width, "{case}: whole, it does not fit");
            assert_eq!(StripFold::of(&named), Name, "{case}");
            let cut = named.shown_name(Name);
            assert_eq!(cut, shown, "{case}");
            let strip = named.needs(Name);
            assert!((strip - total).abs() < 0.01, "{case}: {strip}");
            assert_eq!(strip <= width, total <= width, "{case}: fits");
        }
        assert!(
            named("Bass 808 low", least).needs(Name) <= least,
            "608 holds it"
        );
        assert!(
            named("Bass 808 low", least - 1.0).needs(Name) > least - 1.0,
            "607 does not: 608 is the least whole pixel"
        );
        let lit = StripShape {
            velocity: true,
            recording: true,
            ..shape(0.0, false, true)
        };
        let more = velocity_width() + text_width(REC_NOTES, size::LABEL) + 2.0 * STRIP_GROUP_GAP;
        assert!((lit.needs(Open) - shape(0.0, false, true).needs(Open) - more).abs() < 0.001);
    }

    #[test]
    fn black_keys_follow_the_piano() {
        let black: Vec<u8> = (60..72).filter(|&key| is_black(key)).collect();
        assert_eq!(black, [61, 63, 66, 68, 70]);
        assert_eq!(row_color(60), style::palette().roll_row_white);
        assert_eq!(row_color(61), style::palette().roll_row_black);
    }

    // A6: with C major chosen, C (60) keeps its row color and C# (61) shades darker (0.7 of
    // its color over black); with no scale nothing shades.
    #[test]
    fn keys_outside_the_scale_shade_darker() {
        use crate::ui::scale::{Root, Scale, ScaleMode};
        let c_major = Scale {
            root: Root(0),
            mode: ScaleMode::Major,
        };
        assert_eq!(scaled_row(60, c_major), style::palette().roll_row_white);
        assert_eq!(
            scaled_row(61, c_major),
            style::out_of_scale(style::palette().roll_row_black)
        );
        assert_eq!(
            scaled_row(61, Scale::default()),
            style::palette().roll_row_black
        );
        assert_ne!(
            style::out_of_scale(style::palette().roll_row_black),
            style::palette().roll_row_black
        );
    }

    // A row in a shaded bar is 5 lighter a channel than it is beside it, whatever color the row
    // has, by hand: the white-key row #34373D is #393C42 there, the black-key row #2D3035
    // #32353A, and C# with C major chosen (#2D3035 at 0.7 over black, #202225) #25272A;
    // unshaded, the white row keeps its color. Past the pattern's end the shaded row is dimmed
    // after it is shaded, its shading at 0.6 too, (6x + 5) div 10 a channel: the white row
    // #393C42 is #222428 there (beside it #1F2125: 3 lighter, not 5), the black row #32353A
    // #1E2023. (Pin moved: the dimmed row was shaded after, #24262A, so the bars past the end
    // kept the pattern's full shading on their darker rows.)
    #[test]
    fn rows_in_a_shaded_bar_are_lighter_past_the_end_by_the_dim_factor() {
        use crate::ui::scale::{Root, ScaleMode};
        let none = Scale::default();
        let c_major = Scale {
            root: Root(0),
            mode: ScaleMode::Major,
        };
        let rgb = |color: Color| color.into_rgba8();
        assert_eq!(
            row_fill(60, none, false, false),
            style::palette().roll_row_white
        );
        assert_eq!(
            rgb(row_fill(60, none, false, true)),
            [0x39, 0x3C, 0x42, 0xFF]
        );
        assert_eq!(
            rgb(row_fill(61, none, false, true)),
            [0x32, 0x35, 0x3A, 0xFF]
        );
        assert_eq!(
            rgb(row_fill(60, none, true, true)),
            [0x22, 0x24, 0x28, 0xFF]
        );
        assert_eq!(
            rgb(row_fill(60, none, true, false)),
            [0x1F, 0x21, 0x25, 0xFF]
        );
        assert_eq!(
            rgb(row_fill(61, none, true, true)),
            [0x1E, 0x20, 0x23, 0xFF]
        );
        assert_eq!(
            rgb(row_fill(61, c_major, false, true)),
            [0x25, 0x27, 0x2A, 0xFF]
        );
    }

    // The dim factor on a line's color by hand, (6x + 5) div 10 a channel as the rows past the
    // end have it, its strength kept: the bar line #626873 is #3B3E45 past the end, the beat
    // line #4A4F57 #2C2F34, the half-step line #3E4248 #25282B at its 3/14; inside the pattern
    // each keeps its color.
    #[test]
    fn a_line_past_the_end_is_dimmed_as_the_rows_are() {
        let rgb = |color: Color| {
            let [r, g, b, _] = color.into_rgba8();
            [r, g, b]
        };
        let palette = style::palette();
        assert_eq!(
            rgb(line_color(palette.bar_line, 1.0, true)),
            [0x3B, 0x3E, 0x45]
        );
        assert_eq!(
            rgb(line_color(palette.grid_beat, 1.0, true)),
            [0x2C, 0x2F, 0x34]
        );
        let half = line_color(palette.line_soft, 3.0 / 14.0, true);
        assert_eq!((rgb(half), half.a), ([0x25, 0x28, 0x2B], 3.0 / 14.0));
        assert_eq!(line_color(palette.bar_line, 1.0, false), palette.bar_line);
        let inside = line_color(palette.line_soft, 3.0 / 14.0, false);
        assert_eq!((rgb(inside), inside.a), ([0x3E, 0x42, 0x48], 3.0 / 14.0));
    }

    // p3_roll's lines (a one-bar pattern, 16 px a step) up to tick 2000, by hand: the half step
    // at 12 at 3/14 and the beat at 96 whole, undimmed; the bar line on the end (384) closes
    // the pattern undimmed, bar 2's and no strong one; past the end the half step at 396, the
    // beat at 480 and bar 3's line (768) dimmed; bar 1's (0) and bar 5's (1536) lines are
    // strong, a companion at 0.45 right of each, bar 5's dimmed with it.
    #[test]
    fn the_grids_lines_past_the_end_are_dimmed_and_the_end_line_not() {
        let palette = style::palette();
        let view = View {
            top: 69,
            scroll: Ticks(0),
            zoom: piano_layout::ZOOM.default,
            step: Ticks(24),
            signature: dawsome_engine::time::TimeSignature::COMMON,
            rows: piano_layout::Rows::ALL,
        };
        let lines = grid_lines(&view, Ticks(384), Ticks(2000));
        let at = |tick| {
            let mut on = lines.iter().filter(|line| line.tick == Ticks(tick));
            let line = on.next().unwrap_or_else(|| panic!("a line at {tick}"));
            assert!(on.next().is_none(), "one line at {tick}: the coarsest");
            (line.color, line.companion)
        };
        let faded = |color: Color, alpha: f32| Color { a: alpha, ..color };
        let dim = |color: Color, alpha: f32| faded(style::dimmed(color), alpha);
        let companion = faded(palette.bar_line, STRONG_COMPANION_ALPHA);
        assert_eq!(at(12), (faded(palette.line_soft, 3.0 / 14.0), None));
        assert_eq!(at(96), (palette.grid_beat, None));
        assert_eq!(at(0), (palette.bar_line, Some(companion)));
        assert_eq!(at(384), (palette.bar_line, None), "the end's own line");
        assert_eq!(at(396), (dim(palette.line_soft, 3.0 / 14.0), None));
        assert_eq!(at(480), (dim(palette.grid_beat, 1.0), None));
        assert_eq!(at(768), (dim(palette.bar_line, 1.0), None));
        assert_eq!(
            at(1536),
            (
                dim(palette.bar_line, 1.0),
                Some(dim(palette.bar_line, STRONG_COMPANION_ALPHA))
            )
        );
    }

    // The ruler's beat ticks in the beat lines' color at their strength: 11/14 at 4 px a step
    // (16 px a beat), whole at 16.
    #[test]
    fn the_rulers_beat_ticks_take_the_beat_lines_color() {
        let view = |zoom| View {
            top: 69,
            scroll: Ticks(0),
            zoom,
            step: Ticks(24),
            signature: dawsome_engine::time::TimeSignature::COMMON,
            rows: piano_layout::Rows::ALL,
        };
        let far = beat_tick_color(&view(4.0)).expect("drawn at the farthest zoom");
        assert_eq!(
            Color { a: 1.0, ..far },
            style::palette().grid_beat,
            "the beat lines' color"
        );
        assert!((far.a - 11.0 / 14.0).abs() < 1e-6);
        assert_eq!(
            beat_tick_color(&view(16.0)),
            Some(style::palette().grid_beat)
        );
    }

    // The lane's edges by hand, (y, height) in lane coordinates. At scale 1 from y 528: rows 0
    // and 31, a pixel each. At 1.25 from 780.8 (device row 976): one device row, 0.8 px, at 0
    // and at 31.2 (device row 1015, the last before 1016). At 1.25 from 396.2 (device 495.25):
    // the first whole row is 496, 0.6 into the lane; the lane ends at device 535.25, so the last
    // whole row is 534, 31.0 in. At 1.7 from 100: two device rows, 2/1.7 = 1.1765 px, from 0
    // and from 222/1.7 − 100 = 30.5882 (the lane ends at device 224.4).
    #[test]
    fn the_lanes_edges_lie_on_whole_device_rows_inside_it() {
        let close = |(y, h): (f32, f32), (ey, eh): (f32, f32)| {
            (y - ey).abs() < 1e-3 && (h - eh).abs() < 1e-3
        };
        let at = |top: f32, scale: f32, expected: [(f32, f32); 2]| {
            let edges = lane_edges(top, scale);
            assert!(
                close(edges[0], expected[0]) && close(edges[1], expected[1]),
                "from {top} at {scale}: {edges:?}"
            );
        };
        at(528.0, 1.0, [(0.0, 1.0), (31.0, 1.0)]);
        at(780.8, 1.25, [(0.0, 0.8), (31.2, 0.8)]);
        at(396.2, 1.25, [(0.6, 0.8), (31.0, 0.8)]);
        at(100.0, 1.7, [(0.0, 1.176_5), (30.588_2, 1.176_5)]);
        at(528.0, 0.0, [(0.0, 1.0), (31.0, 1.0)]);
    }

    // 340 px is 28⅓ rows: the 28 whole rows show, keys 69 down to 42 (key 42's row y
    // 324..336), and the third of key 41's row under them is ground. (Pin moved: key 41's row
    // was drawn, cut to its top 4 px, until the grid showed whole rows only.) 336 px is exactly
    // 28 rows, 335 px 27 (key 43 last).
    #[test]
    fn the_grid_draws_every_row_it_shows() {
        let view = View {
            top: 69,
            scroll: Ticks(0),
            zoom: piano_layout::ZOOM.default,
            step: Ticks(24),
            signature: dawsome_engine::time::TimeSignature::COMMON,
            rows: crate::ui::piano_layout::Rows::ALL,
        };
        let keys: Vec<u8> = visible_keys(&view, 340.0).collect();
        assert_eq!(keys.first(), Some(&69));
        assert_eq!(keys.last(), Some(&42));
        assert_eq!(keys.len(), 28);
        assert_eq!(
            visible_keys(&view, 336.0).last(),
            Some(42),
            "exactly 28 rows"
        );
        assert_eq!(visible_keys(&view, 335.0).last(), Some(43));
        assert_eq!(visible_keys(&view, 11.0).count(), 0, "under a row");
        let low = View { top: 10, ..view };
        assert_eq!(visible_keys(&low, 340.0).last(), Some(0));
    }
}
