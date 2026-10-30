//! The Settings window's General page (F10, 6): which MIDI input ports play, what the inputs
//! dropped, the typing keyboard's octave, the hint bar, the audio output device, and the audio
//! input device a new input channel plays; and the header over both pages, its tabs. One canvas
//! draws the page from [`settings_layout`] positions, under the header that stays while it
//! scrolls, and reports presses in the page's coordinates (the header's top-left corner is
//! their origin, so the layout's positions are the window's with the page unscrolled); the app
//! turns them into changes. A window of its own (it was a panel stacked in
//! the main window, which could not scroll to its lower options).

use iced::alignment::{Horizontal, Vertical};
use iced::widget::canvas::{self, Event, Frame, Geometry, Text};
use iced::{mouse, Color, Element, Fill, Point, Rectangle, Renderer, Size, Theme, Vector};

use crate::app::Message;
use crate::audio::OutputChoice;
use crate::settings::MidiInputs;
use crate::ui::settings_layout::{
    tab, tab_at, AudioLayout, Hit, LatencyLayout, Layout, SettingsPage, HEADER_HEIGHT, INSET,
    SECTION_GAP, TEXT_SIZE, TITLE_SIZE,
};

/// The Settings window's size at its first opening: the Keys page's table across and the
/// General page with a few ports and devices down.
pub const WINDOW_SIZE: Size = Size::new(720.0, 560.0);
/// The least the window is made: the header's tabs and the first group (the MIDI input's
/// title, "All ports" and a port or the line saying none was found) always show above the foot
/// line; the rest scrolls.
pub const WINDOW_MIN: Size = Size::new(420.0, 240.0);

/// A point of the General page's canvas, which starts under the header, in the page's
/// coordinates (the header's top-left corner the origin), which the layout gives.
fn page_point(at: Point) -> Point {
    Point::new(at.x, at.y + HEADER_HEIGHT)
}
use crate::ui::style;
use crate::ui::text_fit::fit_width;

/// What the panel reports.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SettingsMessage {
    /// A left press at this point of the panel (its header's tabs, or the General page).
    Pressed(Point),
    /// A left press at this point of the Keys page, under the header.
    KeysPressed(Point),
}

/// What the settings panel shows.
pub struct SettingsPanel<'a> {
    pub ports: &'a [String],
    pub inputs: &'a MidiInputs,
    /// MIDI messages dropped because a queue was full, and live notes a recording lost.
    pub dropped: u64,
    pub lost: u64,
    /// What went wrong listing or opening the ports.
    pub problem: Option<&'a str>,
    /// The typing keyboard's octave, "C4".
    pub octave: String,
    /// Whether the hint bar shows (a scripted run hides it whatever the settings say).
    pub hints: bool,
    /// The output devices offered, "System default" first, the chosen one marked.
    pub outputs: Vec<OutputChoice>,
    /// What plays now, how it is resampled when it is ("Resampling 44100 → 48000 Hz"), why the
    /// device chosen last did not open, and why the device list is short when it is.
    pub playing: String,
    pub resampling: Option<String>,
    pub refused: Option<String>,
    pub outputs_problem: Option<String>,
    /// The input devices a new input channel may play, "System default input" first, the
    /// chosen one marked, and why the list is short when it is.
    pub inputs_offered: Vec<OutputChoice>,
    pub inputs_problem: Option<String>,
    /// The devices a take of the line in uses now, as the latency section names them; the
    /// figure kept for them ("25.7 ms (measured)"), none when unknown; whether a measurement
    /// runs.
    pub latency_devices: String,
    pub latency: Option<String>,
    pub measuring: bool,
}

/// The header across the Settings window: its title and the page tabs, the shown page's lit.
fn draw_header(frame: &mut Frame, page: SettingsPage, width: f32) {
    frame.fill_rectangle(
        Point::ORIGIN,
        Size::new(width, HEADER_HEIGHT),
        style::HEADER,
    );
    text(
        frame,
        "Settings",
        Point::new(INSET, HEADER_HEIGHT / 2.0),
        style::TEXT,
        TEXT_SIZE,
    );
    for shown in SettingsPage::ALL {
        let rect = tab(shown);
        let color = if shown == page {
            frame.fill_rectangle(rect.position(), rect.size(), style::BUTTON);
            style::TEXT
        } else {
            style::TEXT_DIM
        };
        centered(frame, shown.title(), rect.center(), color);
    }
}

/// The header alone, over the page shown (which scrolls under it).
pub struct SettingsHeader {
    pub page: SettingsPage,
}

impl SettingsHeader {
    pub fn view<'a>(self) -> Element<'a, Message> {
        canvas::Canvas::new(self)
            .width(Fill)
            .height(HEADER_HEIGHT)
            .into()
    }
}

impl canvas::Program<Message> for SettingsHeader {
    type State = Option<crate::ui::hint::Hover>;

    fn update(
        &self,
        hovered: &mut Option<crate::ui::hint::Hover>,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                cursor.position_in(bounds).map(|at| {
                    canvas::Action::publish(Message::Settings(SettingsMessage::Pressed(at)))
                        .and_capture()
                })
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let now = cursor
                    .position_in(bounds)
                    .and_then(tab_at)
                    .map(|page| crate::ui::hint::Hover::Setting(Hit::Page(page)));
                crate::ui::hint::hover_change(hovered, now)
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &Option<crate::ui::hint::Hover>,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        draw_header(&mut frame, self.page, bounds.width);
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _state: &Option<crate::ui::hint::Hover>,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if cursor.position_in(bounds).and_then(tab_at).is_some() {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}

/// The words of an input choice's row.
pub fn input_words(choice: &OutputChoice) -> String {
    match (&choice.name, choice.missing) {
        (None, _) => "System default input".to_owned(),
        (Some(name), false) => name.clone(),
        (Some(name), true) => format!("{name} (not plugged in)"),
    }
}

/// The words of an output choice's row.
pub fn output_words(choice: &OutputChoice) -> String {
    match (&choice.name, choice.missing) {
        (None, _) => "System default".to_owned(),
        (Some(name), false) => name.clone(),
        (Some(name), true) => format!("{name} (not plugged in)"),
    }
}

impl<'a> SettingsPanel<'a> {
    /// The General page under the header (which stays over it while it scrolls): a canvas as
    /// tall as its content.
    pub fn view(self) -> Element<'a, Message> {
        let height = self.content_height() - HEADER_HEIGHT;
        canvas::Canvas::new(self).width(Fill).height(height).into()
    }

    /// The page's height from the header's top: down to its last line (the input latency's
    /// buttons) and a section's gap under it.
    pub fn content_height(&self) -> f32 {
        self.latency_layout().bottom() + SECTION_GAP
    }

    /// The lines under the ports: the counts that are not 0, and the problem.
    fn notes(&self) -> Vec<(String, Color)> {
        let mut notes = Vec::new();
        if self.dropped > 0 {
            notes.push((
                format!("Dropped messages: {}", self.dropped),
                style::WARNING,
            ));
        }
        if self.lost > 0 {
            notes.push((
                format!("Lost recorded notes: {}", self.lost),
                style::WARNING,
            ));
        }
        if let Some(problem) = self.problem {
            notes.push((problem.to_owned(), style::TEXT_DIM));
        }
        notes
    }

    /// Where everything is, for what the panel lists now.
    pub fn layout(&self) -> Layout {
        Layout {
            ports: self.ports.len(),
            notes: self.notes().len(),
        }
    }

    /// Where the audio section's parts are.
    pub fn audio_layout(&self) -> AudioLayout {
        self.layout().audio(self.outputs.len())
    }

    /// The lines under the output devices: what plays, how it is resampled when it is, why the
    /// device chosen last did not open, then why the list is short.
    fn output_lines(&self) -> Vec<&str> {
        std::iter::once(self.playing.as_str())
            .chain(self.resampling.as_deref())
            .chain(self.refused.as_deref())
            .chain(self.outputs_problem.as_deref())
            .collect()
    }

    /// Where the audio input section's parts are: under the output section.
    pub fn input_layout(&self) -> AudioLayout {
        self.audio_layout()
            .below(self.output_lines().len(), self.inputs_offered.len())
    }

    /// Where the input latency section's parts are: under the audio input section and why its
    /// list is short.
    pub fn latency_layout(&self) -> LatencyLayout {
        let lines = usize::from(self.inputs_problem.is_some());
        self.input_layout().latency_below(lines)
    }

    /// The input latency section: its title, the devices, the figure or "Unknown: takes land
    /// late", and its buttons (Measure reads "Measuring" while one runs).
    fn draw_latency(&self, frame: &mut Frame) {
        let layout = self.latency_layout();
        let title = layout.title();
        text(
            frame,
            "Input latency (takes of the line in)",
            Point::new(title.x, title.center_y()),
            style::TEXT,
            TITLE_SIZE,
        );
        let (figure, color) = match &self.latency {
            Some(words) => (words.as_str(), style::TEXT),
            None => ("Unknown: takes land late", style::WARNING),
        };
        for (line, words, color) in [
            (0, self.latency_devices.as_str(), style::TEXT_DIM),
            (1, figure, color),
        ] {
            let rect = layout.line(line);
            if let Some(words) = fit_width(words, rect.width, TEXT_SIZE) {
                text(
                    frame,
                    &words,
                    Point::new(rect.x, rect.center_y()),
                    color,
                    TEXT_SIZE,
                );
            }
        }
        let measure = if self.measuring {
            "Measuring"
        } else {
            "Measure"
        };
        for (rect, words) in [
            (layout.measure(), measure),
            (layout.down(), "\u{2212}"),
            (layout.up(), "+"),
            (layout.clear(), "Clear"),
        ] {
            frame.fill_rectangle(rect.position(), rect.size(), style::BUTTON);
            centered(frame, words, rect.center(), style::TEXT);
        }
    }

    /// The audio input section: its title, a row per device as the output section has, and
    /// why the list is short.
    fn draw_inputs(&self, frame: &mut Frame) {
        let layout = self.input_layout();
        let title = layout.title();
        text(
            frame,
            "Audio input (new input channels)",
            Point::new(title.x, title.center_y()),
            style::TEXT,
            TITLE_SIZE,
        );
        for (index, choice) in self.inputs_offered.iter().enumerate() {
            draw_choice(
                frame,
                &layout,
                index,
                &input_words(choice),
                choice.chosen,
                choice.missing,
            );
        }
        if let Some(problem) = &self.inputs_problem {
            let rect = layout.line(0);
            if let Some(words) = fit_width(problem, rect.width, TEXT_SIZE) {
                let at = Point::new(rect.x, rect.center_y());
                text(frame, &words, at, style::TEXT_DIM, TEXT_SIZE);
            }
        }
    }

    /// The audio section: its title, a row per device with a round mark (a dot in it for the
    /// one chosen), what plays, and why the list is short. Why a dot in the text color and not
    /// the accent the ticked boxes take: the mark is a choice of one, not a switch, and the
    /// panel's screenshot probe holds that a scripted run ticks nothing in the accent.
    fn draw_audio(&self, frame: &mut Frame) {
        let audio = self.audio_layout();
        let title = audio.title();
        text(
            frame,
            "Audio output",
            Point::new(title.x, title.center_y()),
            style::TEXT,
            TITLE_SIZE,
        );
        for (index, choice) in self.outputs.iter().enumerate() {
            draw_choice(
                frame,
                &audio,
                index,
                &output_words(choice),
                choice.chosen,
                choice.missing,
            );
        }
        for (line, words) in self.output_lines().into_iter().enumerate() {
            let rect = audio.line(line);
            if let Some(words) = fit_width(words, rect.width, TEXT_SIZE) {
                let at = Point::new(rect.x, rect.center_y());
                text(frame, &words, at, style::TEXT_DIM, TEXT_SIZE);
            }
        }
    }

    /// The page's sections, in the page's coordinates.
    fn draw_body(&self, frame: &mut Frame) {
        let layout = self.layout();
        let title = |frame: &mut Frame, rect: Rectangle, words: &str| {
            text(
                frame,
                words,
                Point::new(rect.x, rect.center_y()),
                style::TEXT,
                TITLE_SIZE,
            );
        };
        title(frame, layout.midi_title(), "MIDI input");
        let all = matches!(self.inputs, MidiInputs::All);
        self.draw_row(frame, &layout, 0, "All ports", all);
        for (index, name) in self.ports.iter().enumerate() {
            let plays = self.inputs.plays(name);
            self.draw_row(frame, &layout, index + 1, name, plays);
        }
        if let Some(rect) = layout.empty_message() {
            let at = Point::new(rect.x, rect.center_y());
            text(
                frame,
                "No MIDI input ports found",
                at,
                style::TEXT_DIM,
                TEXT_SIZE,
            );
        }
        for (line, (words, color)) in self.notes().into_iter().enumerate() {
            let rect = layout.note(line);
            text(
                frame,
                &words,
                Point::new(rect.x, rect.center_y()),
                color,
                TEXT_SIZE,
            );
        }
        title(frame, layout.typing_title(), "Typing keyboard");
        let row = layout.octave_row();
        text(
            frame,
            "Octave",
            Point::new(row.x, row.center_y()),
            style::TEXT,
            TEXT_SIZE,
        );
        for (rect, glyph) in [
            (layout.octave_down(), "\u{2212}"),
            (layout.octave_up(), "+"),
        ] {
            frame.fill_rectangle(rect.position(), rect.size(), style::BUTTON);
            centered(frame, glyph, rect.center(), style::TEXT);
        }
        centered(
            frame,
            &self.octave,
            layout.octave_name().center(),
            style::TEXT,
        );
        title(frame, layout.hints_title(), "Hints");
        let checkbox = layout.hints_checkbox();
        let fill = if self.hints {
            style::ACCENT
        } else {
            style::KNOB_RING
        };
        frame.fill_rectangle(checkbox.position(), checkbox.size(), fill);
        let row = layout.hints_row();
        text(
            frame,
            "Show hints at the window's foot",
            Point::new(crate::ui::settings_layout::LABEL_X, row.center_y()),
            style::TEXT,
            TEXT_SIZE,
        );
        self.draw_audio(frame);
        self.draw_inputs(frame);
        self.draw_latency(frame);
    }

    /// A row of the MIDI section: its checkbox, ticked when `on`, and its words.
    fn draw_row(&self, frame: &mut Frame, layout: &Layout, row: usize, words: &str, on: bool) {
        let checkbox = layout.checkbox(row);
        let fill = if on { style::ACCENT } else { style::KNOB_RING };
        frame.fill_rectangle(checkbox.position(), checkbox.size(), fill);
        let label = layout.label(row);
        // Why cut as well as clipped: the software renderer draws a clipped frame's text whole.
        if let Some(words) = fit_width(words, label.width, TEXT_SIZE) {
            clipped_text(frame, label, &words, style::TEXT);
        }
    }
}

/// Row `index` of an audio section: its round mark (a dot in it when `chosen`), then `words`,
/// dim when the device is `missing`.
fn draw_choice(
    frame: &mut Frame,
    layout: &AudioLayout,
    index: usize,
    words: &str,
    chosen: bool,
    missing: bool,
) {
    let mark = layout.mark(index);
    let radius = mark.width / 2.0;
    frame.fill(
        &canvas::Path::circle(mark.center(), radius),
        style::KNOB_RING,
    );
    if chosen {
        frame.fill(
            &canvas::Path::circle(mark.center(), radius / 2.0),
            style::TEXT,
        );
    }
    let label = layout.label(index);
    let color = if missing {
        style::TEXT_DIM
    } else {
        style::TEXT
    };
    if let Some(words) = fit_width(words, label.width, TEXT_SIZE) {
        clipped_text(frame, label, &words, color);
    }
}

/// `words` from the left of `label` (page coordinates), clipped to it. Why shifted by hand: the
/// page is drawn translated up by the header's height (its canvas starts under the header), and
/// a clipped part is drawn in a frame of its own with no translation (iced 0.14's `with_clip`
/// drafts a fresh frame), where the row's words landed 24 px under their boxes.
fn clipped_text(frame: &mut Frame, label: Rectangle, words: &str, color: Color) {
    let up = Vector::new(0.0, -HEADER_HEIGHT);
    let label = label + up;
    frame.with_clip(label, |frame| {
        let at = Point::new(label.x, label.center_y());
        text(frame, words, at, color, TEXT_SIZE);
    });
}

fn text(frame: &mut Frame, words: &str, at: Point, color: Color, size: f32) {
    frame.fill_text(Text {
        content: words.to_owned(),
        position: at,
        color,
        size: size.into(),
        align_y: Vertical::Center,
        ..Text::default()
    });
}

fn centered(frame: &mut Frame, words: &str, at: Point, color: Color) {
    frame.fill_text(Text {
        content: words.to_owned(),
        position: at,
        color,
        size: TEXT_SIZE.into(),
        align_x: Horizontal::Center.into(),
        align_y: Vertical::Center,
        ..Text::default()
    });
}

impl SettingsPanel<'_> {
    /// What the hint bar names at `at`: a MIDI, typing or hints control, or an output device.
    fn hover_at(&self, at: Point) -> Option<crate::ui::hint::Hover> {
        use crate::ui::hint::Hover;
        if let Some(page) = tab_at(at) {
            return Some(Hover::Setting(Hit::Page(page)));
        }
        if let Some(index) = self.audio_layout().hit(at) {
            let choice = self.outputs.get(index)?;
            return Some(Hover::Output(output_words(choice)));
        }
        if let Some(index) = self.input_layout().hit(at) {
            let choice = self.inputs_offered.get(index)?;
            return Some(Hover::Input(input_words(choice)));
        }
        if let Some(hit) = self.latency_layout().hit(at) {
            return Some(Hover::Setting(hit));
        }
        self.layout().hit(at).map(Hover::Setting)
    }
}

impl canvas::Program<Message> for SettingsPanel<'_> {
    /// What the cursor was over last.
    type State = Option<crate::ui::hint::Hover>;

    fn update(
        &self,
        hovered: &mut Option<crate::ui::hint::Hover>,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                cursor.position_in(bounds).map(|at| {
                    let at = page_point(at);
                    canvas::Action::publish(Message::Settings(SettingsMessage::Pressed(at)))
                        .and_capture()
                })
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let now = cursor
                    .position_in(bounds)
                    .and_then(|at| self.hover_at(page_point(at)));
                crate::ui::hint::hover_change(hovered, now)
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &Option<crate::ui::hint::Hover>,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        // Why no cache: a handful of rectangles and lines, and the counts change as they come.
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), style::RACK_WELL);
        frame.translate(Vector::new(0.0, -HEADER_HEIGHT));
        self.draw_body(&mut frame);
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _state: &Option<crate::ui::hint::Hover>,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        let over = cursor
            .position_in(bounds)
            .and_then(|at| self.hover_at(page_point(at)));
        if over.is_some() {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}
