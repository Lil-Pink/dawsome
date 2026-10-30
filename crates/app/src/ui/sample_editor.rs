//! The sample editor: one channel's sound in a window of its own beside the main one, as the
//! instrument window is, to shape it without changing its file. Top to bottom: the channel's
//! name over its file, length, rate and sides (how many clips play it, how many edits it has,
//! and Revert); the tool row (Play and Stop, Fit, Zoom to selection, Snap to zero, Undo and
//! Redo, Save as new file… and Close at the right); the op bar (Crop, Delete, Silence, Fade
//! in, Fade out, the Gain readout dragged up or down, Normalize, Reverse); the waveform under
//! its ruler (the window of the clip it was opened from bracketed there), a lane a side, the
//! selection shaded, the cursor a line and Play's position
//! another, values past full scale in the warning color; its scroll bar; the foot line, what
//! is under the cursor or how to use it. The geometry is [`crate::ui::sample_view`]'s, the
//! sound and the gestures the app's (`app/sample_editor.rs`).

use std::path::PathBuf;

use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_media::Peaks;
use iced::widget::canvas::{self, Event, Frame, Geometry, Path, Stroke};
use iced::widget::scrollable::{Direction, Scrollbar as ScrollerBar};
use iced::widget::{
    button, canvas as canvas_widget, column, container, mouse_area, row, scrollable, space, text,
};
use iced::{mouse, Center, Color, Element, Fill, Point, Rectangle, Renderer, Size, Theme};

use crate::app::Message;
use crate::ui::hint::{hover_area, hover_change, Hover};
use crate::ui::instrument_window::{HINT_HEIGHT, HINT_SIZE, PADDING};
use crate::ui::sample_view::{self, Drawn, Selection, View as WaveView, RULER_HEIGHT};
use crate::ui::scroll::{BarMessage, Scrollbar, BAR_HEIGHT};
use crate::ui::style;
use crate::ui::text_fit;

/// The window's first size, and the least it can be made.
pub const WINDOW_SIZE: Size = Size::new(900.0, 440.0);
pub const WINDOW_MIN: Size = Size::new(560.0, 320.0);

/// The heading's band, and each row of buttons'.
pub const HEADING_HEIGHT: f32 = 36.0;
pub const ROW_HEIGHT: f32 = 28.0;
/// The room above and below the waveform and its scroll bar.
pub const WAVE_PADDING: f32 = 4.0;
/// The words on the buttons and in the heading.
const BUTTON_SIZE: f32 = 12.0;
const NAME_SIZE: f32 = 15.0;
const ABOUT_SIZE: f32 = 11.0;
/// The Gain readout's box.
const GAIN_WIDTH: f32 = 78.0;
/// The ruler's words.
const MARK_SIZE: f32 = 10.0;
/// A dot on a frame, zoomed in far.
const DOT_RADIUS: f32 = 2.0;
/// The bracket of the clip the editor was opened from, on the ruler: its line's height and
/// its ends' (from the ruler's foot).
pub const BRACKET_LINE: f32 = 2.0;
pub const BRACKET_END: f32 = 8.0;

/// What the heading says, in the warning color, of a sound whose file is not the one its
/// edits were made on (another length now, or the project at another rate).
pub const FILE_CHANGED: &str = "The file changed since these edits; they still apply, by frame";

/// The waveform's width in a window `width` across: the window less its padding at both sides.
pub fn waveform_width(width: f32) -> f32 {
    (width - 2.0 * PADDING).max(1.0)
}

/// Where the waveform (its ruler at its top) lies in a window `size` big, as the window's
/// column lays it out: under the heading and the two rows of buttons, over its scroll bar and
/// the foot line.
#[cfg(test)]
pub fn waveform_area(size: Size) -> Rectangle {
    let top = HEADING_HEIGHT + 2.0 * ROW_HEIGHT + WAVE_PADDING;
    let height = (size.height - top - WAVE_PADDING - BAR_HEIGHT - HINT_HEIGHT).max(0.0);
    Rectangle::new(
        Point::new(PADDING, top),
        Size::new(waveform_width(size.width), height),
    )
}

/// What the window reports.
#[derive(Debug, Clone, PartialEq)]
pub enum SampleEditorMessage {
    /// A left press on the waveform at this x (from its left edge).
    Pressed(f32),
    /// The press held moved to this x.
    Dragged(f32),
    /// The press was let go of at this x.
    Released(f32),
    /// The wheel over the waveform at this x: lines (or pixels) across and up.
    Wheel {
        x: f32,
        across: f32,
        up: f32,
    },
    /// The scroll bar under the waveform.
    Bar(BarMessage),
    Play,
    Stop,
    /// Space in the window: Play, or Stop while it plays.
    PlayStop,
    /// Ctrl+A in the window, or a double-click: the whole sound selected.
    SelectAll,
    /// The whole sound across the window.
    Fit,
    ZoomToSelection,
    /// Snap to zero, switched.
    Snap,
    Op(EditorOp),
    /// A press on the Gain readout: a drag up or down sets the gain, its release applies it.
    GainPressed,
    /// The Gain readout's drag at this window y.
    GainMoved(f32),
    GainReleased,
    /// Revert: the sound as its file is.
    Revert,
    /// Save as new file…
    SaveAs,
    Close,
    /// Select frames `[from, to)` (a scripted run's `--sample-editor-select`; the doors that
    /// open the window on a clip's part).
    Select {
        from: u64,
        to: u64,
    },
    /// Time to move Play's position and take a sound made in the background.
    Tick,
    /// Save as new file…'s write ended, on a worker.
    Saved(SaveDone),
    /// Play's copy of a long part was made, on a worker, for the copy `token` names (none when
    /// the worker stopped first).
    Copied {
        token: u64,
        part: Option<Part>,
    },
}

/// How Save as new file…'s write went: the file, the folder it is in, and what went wrong.
#[derive(Debug, Clone, PartialEq)]
pub struct SaveDone {
    pub target: PathBuf,
    pub dir: PathBuf,
    pub result: Result<(), String>,
}

/// A copy of a part of the sound, for Play.
#[derive(Debug, Clone)]
pub struct Part(pub SampleBuffer);

impl PartialEq for Part {
    fn eq(&self, other: &Self) -> bool {
        self.0.channels() == other.0.channels() && self.0.samples() == other.0.samples()
    }
}

/// Save as new file…'s words: "Saving…" while it writes.
pub fn save_words(saving: bool) -> &'static str {
    if saving {
        "Saving\u{2026}"
    } else {
        "Save as new file\u{2026}"
    }
}

/// An op of the op bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorOp {
    Crop,
    Delete,
    Silence,
    FadeIn,
    FadeOut,
    Normalize,
    Reverse,
}

impl EditorOp {
    /// Every op the bar holds, in its order.
    pub const ALL: [EditorOp; 7] = [
        EditorOp::Crop,
        EditorOp::Delete,
        EditorOp::Silence,
        EditorOp::FadeIn,
        EditorOp::FadeOut,
        EditorOp::Normalize,
        EditorOp::Reverse,
    ];

    /// Its button's words.
    pub fn label(self) -> &'static str {
        match self {
            EditorOp::Crop => "Crop",
            EditorOp::Delete => "Delete",
            EditorOp::Silence => "Silence",
            EditorOp::FadeIn => "Fade in",
            EditorOp::FadeOut => "Fade out",
            EditorOp::Normalize => "Normalize",
            EditorOp::Reverse => "Reverse",
        }
    }

    /// Whether it needs a selection: a crop or a delete of the whole sound means nothing.
    pub fn needs_selection(self) -> bool {
        matches!(self, EditorOp::Crop | EditorOp::Delete)
    }
}

/// A control of the window, as the foot line names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorPart {
    Waveform,
    Play,
    Stop,
    Fit,
    ZoomToSelection,
    Snap,
    Op(EditorOp),
    Gain,
    Undo,
    Redo,
    Revert,
    SaveAs,
    Close,
}

impl EditorPart {
    /// Every control, for the hint table's test.
    #[cfg(test)]
    pub fn all() -> Vec<EditorPart> {
        let mut parts = vec![
            EditorPart::Waveform,
            EditorPart::Play,
            EditorPart::Stop,
            EditorPart::Fit,
            EditorPart::ZoomToSelection,
            EditorPart::Snap,
            EditorPart::Gain,
            EditorPart::Undo,
            EditorPart::Redo,
            EditorPart::Revert,
            EditorPart::SaveAs,
            EditorPart::Close,
        ];
        parts.extend(EditorOp::ALL.map(EditorPart::Op));
        parts
    }
}

/// The sound the waveform draws: its frames side by side, how many sides, its peaks.
#[derive(Debug, Clone, Copy)]
pub struct Wave<'a> {
    pub samples: &'a [f32],
    pub sides: u8,
    pub peaks: &'a Peaks,
}

/// The drawing the window keeps between frames: the waveform and its ruler (the selection, the
/// cursor and Play's position are drawn over it each frame).
#[derive(Debug, Default)]
pub struct Caches {
    pub wave: canvas::Cache,
}

/// What the window shows.
pub struct View<'a> {
    /// The channel's name, and its file's.
    pub name: &'a str,
    pub file: String,
    /// "1.088 s · 44100 Hz · mono", or why the file cannot be read.
    pub about: String,
    /// How many clips play the sound, and how many edits it has.
    pub clips: usize,
    pub edits: usize,
    /// Whether its file changed since its edits were made ([`FILE_CHANGED`]).
    pub stale: bool,
    pub wave: Option<Wave<'a>>,
    pub view: WaveView,
    /// The window of the clip the editor was opened from, in frames of the sound.
    pub bracket: Option<Selection>,
    pub selection: Option<Selection>,
    pub cursor: u64,
    /// Where Play is in the sound, while it plays.
    pub playhead: Option<u64>,
    pub rate: u32,
    pub snap: bool,
    /// The Gain readout's value, in hundredths of a dB.
    pub gain: i32,
    /// Whether the ops can run: the sound shown is the channel's as it is now, and no save or
    /// copy is under way.
    pub ready: bool,
    /// Whether a save or Play's copy is under way (the buttons dim), and whether it is a save.
    pub busy: bool,
    pub saving: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub size: Size,
    pub hint: String,
    pub caches: &'a Caches,
}

/// The window's content, filling its window.
pub fn view(shown: View<'_>) -> Element<'_, Message> {
    let editor = |message| Message::SampleEditor(message);
    let room = (shown.size.width - 2.0 * PADDING).max(0.0);
    let mut about = format!("{} \u{b7} {}", shown.file, shown.about);
    if shown.clips > 1 {
        about.push_str(&format!(" \u{b7} plays in {} clips", shown.clips));
    }
    if shown.edits > 0 {
        let edits = if shown.edits == 1 { "edit" } else { "edits" };
        about.push_str(&format!(" \u{b7} {} {edits}", shown.edits));
    }
    let mut heading = row![
        text(shown.name.to_owned())
            .size(NAME_SIZE)
            .wrapping(text::Wrapping::None),
        text(about)
            .size(ABOUT_SIZE)
            .color(style::TEXT_DIM)
            .wrapping(text::Wrapping::None),
    ]
    .spacing(10)
    .align_y(Center);
    if shown.stale {
        heading = heading.push(
            text(FILE_CHANGED)
                .size(ABOUT_SIZE)
                .color(style::WARNING)
                .wrapping(text::Wrapping::None),
        );
    }
    heading = heading.push(space().width(Fill));
    if shown.edits > 0 {
        heading = heading.push(tool(
            "Revert",
            EditorPart::Revert,
            false,
            shown.ready.then(|| editor(SampleEditorMessage::Revert)),
        ));
    }
    let heading = container(heading)
        .padding([0.0, PADDING])
        .width(Fill)
        .height(HEADING_HEIGHT)
        .align_y(Center)
        .clip(true);
    let selected = shown.selection.is_some();
    let tools = row![
        tool(
            "Play",
            EditorPart::Play,
            shown.playhead.is_some(),
            (shown.wave.is_some() && !shown.busy).then(|| editor(SampleEditorMessage::Play)),
        ),
        tool(
            "Stop",
            EditorPart::Stop,
            false,
            shown
                .playhead
                .is_some()
                .then(|| editor(SampleEditorMessage::Stop)),
        ),
        space().width(8),
        tool(
            "Fit",
            EditorPart::Fit,
            false,
            Some(editor(SampleEditorMessage::Fit))
        ),
        tool(
            "Zoom to selection",
            EditorPart::ZoomToSelection,
            false,
            selected.then(|| editor(SampleEditorMessage::ZoomToSelection)),
        ),
        space().width(8),
        tool(
            "Snap to zero",
            EditorPart::Snap,
            shown.snap,
            Some(editor(SampleEditorMessage::Snap)),
        ),
        space().width(8),
        tool(
            "Undo",
            EditorPart::Undo,
            false,
            (shown.can_undo && !shown.busy).then_some(Message::Undo)
        ),
        tool(
            "Redo",
            EditorPart::Redo,
            false,
            (shown.can_redo && !shown.busy).then_some(Message::Redo)
        ),
        space().width(Fill),
        tool(
            save_words(shown.saving),
            EditorPart::SaveAs,
            false,
            shown.ready.then(|| editor(SampleEditorMessage::SaveAs)),
        ),
        tool(
            "Close",
            EditorPart::Close,
            false,
            Some(editor(SampleEditorMessage::Close))
        ),
    ]
    .spacing(4)
    .align_y(Center);
    let mut ops = row![].spacing(4).align_y(Center);
    for op in EditorOp::ALL {
        let runs = shown.ready && (selected || !op.needs_selection());
        ops = ops.push(tool(
            op.label(),
            EditorPart::Op(op),
            false,
            runs.then(|| editor(SampleEditorMessage::Op(op))),
        ));
        if op == EditorOp::FadeOut {
            ops = ops.push(gain_readout(shown.gain, shown.ready));
        }
    }
    let waveform = canvas_widget(Waveform {
        wave: shown.wave,
        view: shown.view,
        bracket: shown.bracket,
        selection: shown.selection,
        cursor: shown.cursor,
        playhead: shown.playhead,
        rate: shown.rate,
        cache: &shown.caches.wave,
    })
    .width(Fill)
    .height(Fill);
    let bar = canvas_widget(Scrollbar {
        span: shown.view.span(),
        message: |message| Message::SampleEditor(SampleEditorMessage::Bar(message)),
    })
    .width(Fill)
    .height(BAR_HEIGHT);
    let foot = text_fit::fit_width(&shown.hint, room, HINT_SIZE)
        .map(std::borrow::Cow::into_owned)
        .unwrap_or_default();
    let foot = container(
        text(foot)
            .size(HINT_SIZE)
            .color(style::TEXT_DIM)
            .wrapping(text::Wrapping::None),
    )
    .padding([0.0, PADDING])
    .width(Fill)
    .height(HINT_HEIGHT)
    .align_y(Center)
    .style(style::window);
    let band = |content: Element<'static, Message>| {
        container(
            scrollable(content)
                .direction(Direction::Horizontal(
                    ScrollerBar::new().width(3).scroller_width(3),
                ))
                .width(Fill),
        )
        .padding([0.0, PADDING])
        .width(Fill)
        .height(ROW_HEIGHT)
        .align_y(Center)
    };
    let body = column![
        heading,
        band(tools.into()),
        band(ops.into()),
        container(column![waveform, bar])
            .padding([WAVE_PADDING, PADDING])
            .width(Fill)
            .height(Fill),
        foot
    ];
    container(body)
        .width(Fill)
        .height(Fill)
        .style(style::panel)
        .into()
}

/// A button of the tool row or the op bar: lit when `lit`, dim without a message.
fn tool(
    words: &'static str,
    part: EditorPart,
    lit: bool,
    message: Option<Message>,
) -> Element<'static, Message> {
    let button = button(text(words).size(BUTTON_SIZE))
        .padding([2, 8])
        .style(style::tool_button(lit))
        .on_press_maybe(message);
    hover_area(button, Hover::SampleEditor(part))
}

/// The Gain readout: "Gain", then the gain a drag up or down has set, on the readouts' glass.
fn gain_readout(gain: i32, ready: bool) -> Element<'static, Message> {
    let value = gain_text(gain);
    let color = if ready {
        style::LCD_TEXT
    } else {
        style::TEXT_DIM
    };
    let glass = container(text(value).size(BUTTON_SIZE).color(color))
        .width(GAIN_WIDTH)
        .padding([2, 6])
        .align_x(Center)
        .style(|_theme: &Theme| container::Style {
            background: Some(style::LCD.into()),
            border: iced::Border {
                color: style::BUTTON_EDGE,
                width: 1.0,
                radius: 2.0.into(),
            },
            ..container::Style::default()
        });
    let readout = row![text("Gain").size(BUTTON_SIZE).color(style::TEXT_DIM), glass]
        .spacing(4)
        .align_y(Center);
    let area = mouse_area(readout).interaction(mouse::Interaction::ResizingVertically);
    let area = if ready {
        area.on_press(Message::SampleEditor(SampleEditorMessage::GainPressed))
    } else {
        area
    };
    hover_area(area, Hover::SampleEditor(EditorPart::Gain))
}

/// A gain in hundredths of a dB as the readout shows it: "+3.0 dB", "−0.6 dB", "0.0 dB".
pub fn gain_text(centi_db: i32) -> String {
    let tenths = (centi_db.abs() + 5) / 10;
    let sign = match centi_db.signum() {
        _ if tenths == 0 => "",
        1 => "+",
        _ => "\u{2212}",
    };
    format!("{sign}{}.{} dB", tenths / 10, tenths % 10)
}

/// The waveform under its ruler, and what a press, a drag and the wheel over it say.
struct Waveform<'a> {
    wave: Option<Wave<'a>>,
    view: WaveView,
    bracket: Option<Selection>,
    selection: Option<Selection>,
    cursor: u64,
    playhead: Option<u64>,
    rate: u32,
    cache: &'a canvas::Cache,
}

/// Between events: whether a press is held, where the cursor was last seen, what it is over.
#[derive(Debug, Default)]
struct WaveState {
    held: bool,
    last: Option<Point>,
    hover: Option<Hover>,
}

impl canvas::Program<Message> for Waveform<'_> {
    type State = WaveState;

    fn update(
        &self,
        state: &mut WaveState,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let send = |message| canvas::Action::publish(Message::SampleEditor(message));
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let at = cursor.position_in(bounds)?;
                state.held = true;
                state.last = Some(at);
                Some(send(SampleEditorMessage::Pressed(at.x)).and_capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) if state.held => {
                let at = Point::new(position.x - bounds.x, position.y - bounds.y);
                state.last = Some(at);
                Some(send(SampleEditorMessage::Dragged(at.x)))
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if state.held => {
                state.held = false;
                let at = cursor.position_in(bounds).or(state.last)?;
                Some(send(SampleEditorMessage::Released(at.x)))
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let at = cursor.position_in(bounds)?;
                let (mouse::ScrollDelta::Lines { x, y } | mouse::ScrollDelta::Pixels { x, y }) =
                    *delta;
                Some(
                    send(SampleEditorMessage::Wheel {
                        x: at.x,
                        across: x,
                        up: y,
                    })
                    .and_capture(),
                )
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let now = cursor
                    .position_in(bounds)
                    .map(|_| Hover::SampleEditor(EditorPart::Waveform));
                hover_change(&mut state.hover, now)
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &WaveState,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let view = WaveView {
            width: bounds.width,
            ..self.view
        };
        let wave = self.cache.draw(renderer, bounds.size(), |frame| {
            draw_wave(frame, self.wave, &view, self.rate);
        });
        let mut over = Frame::new(renderer, bounds.size());
        let lanes_top = RULER_HEIGHT;
        let lanes_height = (bounds.height - RULER_HEIGHT).max(0.0);
        if let Some(selection) = self.selection {
            // Lit over the wave, its edges in the accent where they are in view.
            let (from, to) = (
                view.x_of(selection.from as f64),
                view.x_of(selection.to as f64),
            );
            let (left, right) = (from.clamp(0.0, bounds.width), to.clamp(0.0, bounds.width));
            over.fill_rectangle(
                Point::new(left, lanes_top),
                Size::new((right - left).max(1.0), lanes_height),
                Color {
                    a: 0.14,
                    ..style::TEXT
                },
            );
            for edge in [from, to] {
                if (0.0..=bounds.width).contains(&edge) {
                    over.fill_rectangle(
                        Point::new(edge.round() - 0.5, lanes_top),
                        Size::new(1.0, lanes_height),
                        style::ACCENT,
                    );
                }
            }
        }
        let line = |frame: &mut Frame, at: u64, color: Color| {
            let x = view.x_of(at as f64);
            if (0.0..=bounds.width).contains(&x) {
                frame.fill_rectangle(
                    Point::new(x.round() - 0.5, lanes_top),
                    Size::new(1.0, lanes_height),
                    color,
                );
            }
        };
        line(&mut over, self.cursor, style::TEXT);
        if let Some(at) = self.playhead {
            line(&mut over, at, style::ACCENT);
        }
        if let Some(clip) = self.bracket {
            draw_bracket(&mut over, &view, clip, bounds.width);
        }
        vec![wave, over.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _state: &WaveState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        match cursor.position_in(bounds) {
            Some(_) => mouse::Interaction::Text,
            None => mouse::Interaction::default(),
        }
    }
}

/// The clip's window on the ruler, in the clip's color: a line along the ruler's foot across
/// the part in view, an end rising from it where the window starts or ends in view.
fn draw_bracket(frame: &mut Frame, view: &WaveView, clip: Selection, width: f32) {
    let (from, to) = (view.x_of(clip.from as f64), view.x_of(clip.to as f64));
    let (left, right) = (from.clamp(0.0, width), to.clamp(0.0, width));
    if right <= 0.0 || left >= width {
        return;
    }
    frame.fill_rectangle(
        Point::new(left.round(), RULER_HEIGHT - BRACKET_LINE),
        Size::new((right.round() - left.round()).max(1.0), BRACKET_LINE),
        style::AUDIO_WAVE,
    );
    for edge in [from, to] {
        if (0.0..=width).contains(&edge) {
            // The end at the window's last frame stands inside it, as the selection's does.
            let x = if edge == to {
                edge.round() - 1.0
            } else {
                edge.round()
            };
            frame.fill_rectangle(
                Point::new(x, RULER_HEIGHT - BRACKET_END),
                Size::new(1.0, BRACKET_END),
                style::AUDIO_WAVE,
            );
        }
    }
}

/// The ruler, each side's lane and its wave: from the peaks, the frames' columns, a line
/// through the frames or that line and a dot on each, as the zoom says; a column or a point
/// past full scale in the warning color.
fn draw_wave(frame: &mut Frame, wave: Option<Wave<'_>>, view: &WaveView, rate: u32) {
    let size = frame.size();
    frame.fill_rectangle(Point::ORIGIN, size, style::RACK_WELL);
    frame.fill_rectangle(
        Point::ORIGIN,
        Size::new(size.width, RULER_HEIGHT),
        style::HEADER,
    );
    for (x, at) in view.marks(rate) {
        frame.fill_rectangle(
            Point::new(x.round(), RULER_HEIGHT - 5.0),
            Size::new(1.0, 5.0),
            style::TEXT_DIM,
        );
        frame.fill_text(canvas::Text {
            content: sample_view::time_text(at, rate),
            position: Point::new(x.round() + 3.0, 2.0),
            color: style::TEXT_DIM,
            size: MARK_SIZE.into(),
            ..canvas::Text::default()
        });
    }
    let Some(wave) = wave else {
        return;
    };
    let sides = usize::from(wave.sides.clamp(1, 2));
    let lanes = sample_view::lanes(size.height, wave.sides);
    let frames = (wave.samples.len() / sides) as u64;
    let value = |index: u64, side: usize| {
        usize::try_from(index)
            .ok()
            .and_then(|index| wave.samples.get(index * sides + side))
            .copied()
            .unwrap_or(0.0)
    };
    let columns = size.width.max(0.0).ceil() as u32;
    for (side, &(top, height)) in lanes.iter().enumerate() {
        frame.fill_rectangle(
            Point::new(0.0, top),
            Size::new(size.width, height),
            style::AUDIO_BODY,
        );
        frame.fill_rectangle(
            Point::new(0.0, top + height / 2.0),
            Size::new(size.width, 1.0),
            style::AUDIO_CENTER,
        );
        let y = |value: f32| sample_view::value_y(top, height, value).clamp(top, top + height);
        let over = |value: f32| value.abs() > 1.0;
        match view.drawn() {
            Drawn::Peaks | Drawn::Columns => {
                let mut fine = canvas::path::Builder::new();
                let mut hot = canvas::path::Builder::new();
                for column in 0..columns {
                    if view.at(column as f32) >= frames as f64 {
                        break;
                    }
                    let (from, to) = view.column(column);
                    let span = if view.drawn() == Drawn::Peaks {
                        wave.peaks.span(side, from, to)
                    } else {
                        (from..to).map(|index| value(index, side)).fold(
                            None,
                            |span: Option<(f32, f32)>, v| {
                                Some(span.map_or((v, v), |(lo, hi)| (lo.min(v), hi.max(v))))
                            },
                        )
                    };
                    let Some((lo, hi)) = span else {
                        continue;
                    };
                    let (top_y, bottom_y) = (y(hi), y(lo));
                    let path = if over(lo) || over(hi) {
                        &mut hot
                    } else {
                        &mut fine
                    };
                    path.rectangle(
                        Point::new(column as f32, top_y),
                        Size::new(1.0, (bottom_y - top_y).max(1.0)),
                    );
                }
                frame.fill(&fine.build(), style::AUDIO_WAVE);
                frame.fill(&hot.build(), style::WARNING);
            }
            Drawn::Lines | Drawn::Dots => {
                let (from, to) = view.frames_shown();
                let point = |index: u64| {
                    Point::new(
                        view.x_of(index as f64 + 0.5),
                        sample_view::value_y(top, height, value(index, side)),
                    )
                };
                let mut line = canvas::path::Builder::new();
                for index in from..to {
                    if index == from {
                        line.move_to(point(index));
                    } else {
                        line.line_to(point(index));
                    }
                }
                frame.stroke(
                    &line.build(),
                    Stroke::default()
                        .with_color(style::AUDIO_WAVE)
                        .with_width(1.0),
                );
                if view.drawn() == Drawn::Dots {
                    for index in from..to {
                        let color = if over(value(index, side)) {
                            style::WARNING
                        } else {
                            style::AUDIO_WAVE
                        };
                        frame.fill(&Path::circle(point(index), DOT_RADIUS), color);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The readout in tenths of a dB, rounded half away from zero: 600 is +6.0 dB, -300 is
    // −3.0 dB (a true minus), 5 rounds to +0.1, 4 to 0.0 without a sign, -2400 is −24.0.
    #[test]
    fn the_gain_readout_shows_tenths_of_a_db() {
        assert_eq!(gain_text(600), "+6.0 dB");
        assert_eq!(gain_text(-300), "\u{2212}3.0 dB");
        assert_eq!(gain_text(5), "+0.1 dB");
        assert_eq!(gain_text(4), "0.0 dB");
        assert_eq!(gain_text(0), "0.0 dB");
        assert_eq!(gain_text(-2400), "\u{2212}24.0 dB");
        assert_eq!(waveform_width(WINDOW_SIZE.width), 876.0);
        assert_eq!(save_words(true), "Saving\u{2026}");
        assert_eq!(save_words(false), "Save as new file\u{2026}");
    }
}
