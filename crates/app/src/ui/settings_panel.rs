//! The Settings window's General page (F10, 6): which MIDI input ports play, what the inputs
//! dropped, the typing keyboard's octave, the hint bar, the zoom, the audio output device, and
//! the audio input device a new input channel plays; and the header over the three pages, its
//! tabs.
//! One canvas
//! draws the page from [`settings_layout`] positions, under the header that stays while it
//! scrolls, and reports presses in the page's coordinates (the header's top-left corner is
//! their origin, so the layout's positions are the window's with the page unscrolled); the app
//! turns them into changes. The Theme page reports its own the same way
//! ([`crate::ui::theme_page`]). A window of its own (it was a panel stacked in the main window,
//! which could not scroll to its lower options).

use iced::alignment::{Horizontal, Vertical};
use iced::widget::canvas::{self, Event, Frame, Geometry, Text};
use iced::{mouse, Color, Element, Fill, Point, Rectangle, Renderer, Size, Theme, Vector};

use crate::app::Message;
use crate::audio::OutputChoice;
use crate::settings::MidiInputs;
use crate::ui::color_wheel::WheelPart;
use crate::ui::fonts::size;
use crate::ui::hint::Hover;
use crate::ui::settings_layout::{
    tab, tab_at, AudioLayout, Hit, LatencyLayout, Layout, SettingsPage, HEADER_HEIGHT, INSET,
};
use crate::ui::theme_page::ColorTarget;

/// The Settings window's size at its first opening: the Keys page's table across and the
/// General page with a few ports and devices down, in one column (under 800 px: two columns
/// from there, three from 1200, [`crate::ui::settings_layout::Columns`]).
pub const WINDOW_SIZE: Size = Size::new(crate::ui::settings_layout::FIRST_WIDTH, 560.0);
/// The least the window is made: the header's tabs and the first group (the MIDI input's
/// title, "All ports" and a port or the line saying none was found) always show above the foot
/// line; the rest scrolls.
pub const WINDOW_MIN: Size = Size::new(420.0, 240.0);

/// The Settings window's share of the main window's width and height ([`window_size_for`]).
pub const MAIN_SHARE: (f32, f32) = (0.47, 0.67);

/// The Settings window's size when it opens over a main window `main` big (logical pixels
/// both): its share of each side ([`MAIN_SHARE`]), rounded to a whole pixel, never under
/// [`WINDOW_MIN`]. The window keeps it: the user does not resize it. Why: asked a window that
/// sits over the main one as the user had placed it (906 × 698 over 1916 × 1047), the same
/// share whatever the screen, its page laid out for that width ([`crate::ui::settings_layout::
/// Columns`]); a window larger than its monitor is made to fit as every pop-out is.
pub fn window_size_for(main: Size) -> Size {
    Size::new(
        (main.width * MAIN_SHARE.0).round().max(WINDOW_MIN.width),
        (main.height * MAIN_SHARE.1).round().max(WINDOW_MIN.height),
    )
}

/// A point of the General or the Theme page's canvas, which starts under the header, in the
/// page's coordinates (the header's top-left corner the origin), which the layouts give.
pub(crate) fn page_point(at: Point) -> Point {
    Point::new(at.x, at.y + HEADER_HEIGHT)
}
use crate::ui::style;
use crate::ui::text_fit::fit_width;

/// What the panel reports.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingsMessage {
    /// A left press at this point of the window (its header's tabs, the General page or the
    /// Theme page).
    Pressed(Point),
    /// A left press at this point of the Keys page, under the header.
    KeysPressed(Point),
    /// The cursor moved to this point of the Theme page while a press on this part of the
    /// colour wheel is held, wherever the cursor is.
    WheelDragged(WheelPart, Point),
    /// The press on the colour wheel let go of.
    WheelReleased,
    /// The colour's hex box holds this text, as typed.
    HexTyped(String),
    /// Enter in the colour's hex box.
    HexEntered,
    /// The Theme page's menu chose what the colour wheel edits.
    ColorTargetChosen(ColorTarget),
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
    /// The zoom every window draws at.
    pub zoom: crate::zoom::Zoom,
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
    /// "Reset to defaults" was pressed once and waits for the second press: it reads "Press
    /// again to reset".
    pub reset_armed: bool,
    /// The page's width (the Settings window's), which decides its columns.
    pub width: f32,
}

/// The header across the Settings window: its title and the page tabs, the shown page's lit.
fn draw_header(frame: &mut Frame, page: SettingsPage, width: f32) {
    frame.fill_rectangle(
        Point::ORIGIN,
        Size::new(width, HEADER_HEIGHT),
        style::palette().raised,
    );
    text(
        frame,
        "Settings",
        Point::new(INSET, HEADER_HEIGHT / 2.0),
        style::palette().text,
        size::LABEL,
    );
    for shown in SettingsPage::ALL {
        let rect = tab(shown);
        let color = if shown == page {
            frame.fill_rectangle(rect.position(), rect.size(), style::palette().button);
            style::palette().text
        } else {
            style::palette().text_secondary
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

    /// The page's height from the header's top: down to its last row ("Reset to defaults")
    /// and a section's gap under it.
    pub fn content_height(&self) -> f32 {
        self.layout().page_bottom()
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
            notes.push((problem.to_owned(), style::palette().text_secondary));
        }
        notes
    }

    /// Where everything is, for what the panel lists now and its width.
    pub fn layout(&self) -> Layout {
        Layout {
            ports: self.ports.len(),
            notes: self.notes().len(),
            outputs: self.outputs.len(),
            output_lines: self.output_lines().len(),
            inputs: self.inputs_offered.len(),
            input_lines: usize::from(self.inputs_problem.is_some()),
            width: self.width,
        }
    }

    /// Where the audio section's parts are.
    pub fn audio_layout(&self) -> AudioLayout {
        self.layout().audio()
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

    /// Where the audio input section's parts are.
    pub fn input_layout(&self) -> AudioLayout {
        self.layout().input()
    }

    /// Where the input latency section's parts are, and "Reset to defaults".
    pub fn latency_layout(&self) -> LatencyLayout {
        self.layout().latency()
    }

    /// The input latency section: its title, the devices, the figure or "Unknown: takes land
    /// late", and its buttons (Measure reads "Measuring" while one runs); then, under it, the
    /// page's last row, "Reset to defaults" ("Press again to reset" once pressed).
    fn draw_latency(&self, frame: &mut Frame) {
        let layout = self.latency_layout();
        let title = layout.title();
        text(
            frame,
            "Input latency (takes of the line in)",
            Point::new(title.x, title.center_y()),
            style::palette().text,
            size::TITLE,
        );
        let (figure, color) = match &self.latency {
            Some(words) => (words.as_str(), style::palette().text),
            None => ("Unknown: takes land late", style::WARNING),
        };
        for (line, words, color) in [
            (
                0,
                self.latency_devices.as_str(),
                style::palette().text_secondary,
            ),
            (1, figure, color),
        ] {
            let rect = layout.line(line);
            if let Some(words) = fit_width(words, rect.width, size::LABEL) {
                text(
                    frame,
                    &words,
                    Point::new(rect.x, rect.center_y()),
                    color,
                    size::LABEL,
                );
            }
        }
        let measure = if self.measuring {
            "Measuring"
        } else {
            "Measure"
        };
        let reset = if self.reset_armed {
            "Press again to reset"
        } else {
            "Reset to defaults"
        };
        for (rect, words) in [
            (layout.measure(), measure),
            (layout.down(), "\u{2212}"),
            (layout.up(), "+"),
            (layout.clear(), "Clear"),
            (layout.reset(), reset),
        ] {
            frame.fill_rectangle(rect.position(), rect.size(), style::palette().button);
            centered(frame, words, rect.center(), style::palette().text);
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
            style::palette().text,
            size::TITLE,
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
            if let Some(words) = fit_width(problem, rect.width, size::LABEL) {
                let at = Point::new(rect.x, rect.center_y());
                text(
                    frame,
                    &words,
                    at,
                    style::palette().text_secondary,
                    size::LABEL,
                );
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
            style::palette().text,
            size::TITLE,
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
            if let Some(words) = fit_width(words, rect.width, size::LABEL) {
                let at = Point::new(rect.x, rect.center_y());
                text(
                    frame,
                    &words,
                    at,
                    style::palette().text_secondary,
                    size::LABEL,
                );
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
                style::palette().text,
                size::TITLE,
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
                style::palette().text_secondary,
                size::LABEL,
            );
        }
        for (line, (words, color)) in self.notes().into_iter().enumerate() {
            let rect = layout.note(line);
            text(
                frame,
                &words,
                Point::new(rect.x, rect.center_y()),
                color,
                size::LABEL,
            );
        }
        title(frame, layout.typing_title(), "Typing keyboard");
        let row = layout.octave_row();
        text(
            frame,
            "Octave",
            Point::new(row.x, row.center_y()),
            style::palette().text,
            size::LABEL,
        );
        for (rect, glyph) in [
            (layout.octave_down(), "\u{2212}"),
            (layout.octave_up(), "+"),
        ] {
            frame.fill_rectangle(rect.position(), rect.size(), style::palette().button);
            centered(frame, glyph, rect.center(), style::palette().text);
        }
        centered(
            frame,
            &self.octave,
            layout.octave_name().center(),
            style::palette().text,
        );
        title(frame, layout.hints_title(), "Hints");
        let checkbox = layout.hints_checkbox();
        let fill = if self.hints {
            style::accent()
        } else {
            style::palette().knob_ring
        };
        frame.fill_rectangle(checkbox.position(), checkbox.size(), fill);
        let row = layout.hints_row();
        text(
            frame,
            "Show hints at the window's foot",
            Point::new(row.x + crate::ui::settings_layout::LABEL_X, row.center_y()),
            style::palette().text,
            size::LABEL,
        );
        title(frame, layout.display_title(), "Display");
        let row = layout.zoom_row();
        text(
            frame,
            "Zoom",
            Point::new(row.x, row.center_y()),
            style::palette().text,
            size::LABEL,
        );
        for (rect, glyph) in [(layout.zoom_down(), "\u{2212}"), (layout.zoom_up(), "+")] {
            frame.fill_rectangle(rect.position(), rect.size(), style::palette().button);
            centered(frame, glyph, rect.center(), style::palette().text);
        }
        centered(
            frame,
            &self.zoom.to_string(),
            layout.zoom_value().center(),
            style::palette().text,
        );
        self.draw_audio(frame);
        self.draw_inputs(frame);
        self.draw_latency(frame);
    }

    /// A row of the MIDI section: its checkbox, ticked when `on`, and its words.
    fn draw_row(&self, frame: &mut Frame, layout: &Layout, row: usize, words: &str, on: bool) {
        let checkbox = layout.checkbox(row);
        let fill = if on {
            style::accent()
        } else {
            style::palette().knob_ring
        };
        frame.fill_rectangle(checkbox.position(), checkbox.size(), fill);
        let label = layout.label(row);
        // Why cut as well as clipped: the software renderer draws a clipped frame's text whole.
        if let Some(words) = fit_width(words, label.width, size::LABEL) {
            clipped_text(frame, label, &words, style::palette().text);
        }
    }
}

/// Row `index` of an audio section: its round mark in an unticked checkbox's tone, a dot in it in
/// the accent when `chosen` (as a ticked checkbox is; it was the text's grey, hard to see on the
/// mark's grey), then `words`, dim when the device is `missing`.
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
        style::palette().knob_ring,
    );
    if chosen {
        frame.fill(
            &canvas::Path::circle(mark.center(), radius / 2.0),
            style::accent(),
        );
    }
    let label = layout.label(index);
    let color = if missing {
        style::palette().text_disabled
    } else {
        style::palette().text
    };
    if let Some(words) = fit_width(words, label.width, size::LABEL) {
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
        text(frame, words, at, color, size::LABEL);
    });
}

pub(crate) fn text(frame: &mut Frame, words: &str, at: Point, color: Color, size: f32) {
    frame.fill_text(Text {
        content: words.to_owned(),
        position: at,
        color,
        size: size.into(),
        align_y: Vertical::Center,
        ..Text::default()
    });
}

pub(crate) fn centered(frame: &mut Frame, words: &str, at: Point, color: Color) {
    frame.fill_text(Text {
        content: words.to_owned(),
        position: at,
        color,
        size: size::LABEL.into(),
        align_x: Horizontal::Center.into(),
        align_y: Vertical::Center,
        ..Text::default()
    });
}

impl SettingsPanel<'_> {
    /// What the hint bar names at `at`: a MIDI, typing, hints or zoom control, or an output
    /// or input device, or the input latency's buttons.
    fn hover_at(&self, at: Point) -> Option<Hover> {
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
    type State = Option<Hover>;

    fn update(
        &self,
        hovered: &mut Option<Hover>,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let at = page_point(cursor.position_in(bounds)?);
                Some(
                    canvas::Action::publish(Message::Settings(SettingsMessage::Pressed(at)))
                        .and_capture(),
                )
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
        _hovered: &Option<Hover>,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        // Why no cache: a handful of rectangles and lines, and the counts change as they come.
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), style::palette().bg);
        frame.translate(Vector::new(0.0, -HEADER_HEIGHT));
        self.draw_body(&mut frame);
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _hovered: &Option<Hover>,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        match cursor
            .position_in(bounds)
            .and_then(|at| self.hover_at(page_point(at)))
        {
            Some(_) => mouse::Interaction::Pointer,
            None => mouse::Interaction::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::settings_layout::Columns;

    // The Settings window's size by hand from the main window's: 0.47 and 0.67 of each side,
    // rounded to a whole pixel. 1920 × 1080: 902.4 → 902, 723.6 → 724, its page in two columns
    // (902 is 800 or more); 2560 × 1440: 1203.2 → 1203, 964.8 → 965, three (1200 or more);
    // 1280 × 800: 601.6 → 602, 536, one, above the least (420 × 240) so as it is; a main
    // window of 800 × 300: 376 and 201, under the least, so 420 × 240.
    #[test]
    fn the_settings_window_takes_its_share_of_the_main_window() {
        let table = [
            (Size::new(1920.0, 1080.0), Size::new(902.0, 724.0), 2),
            (Size::new(2560.0, 1440.0), Size::new(1203.0, 965.0), 3),
            (Size::new(1280.0, 800.0), Size::new(602.0, 536.0), 1),
            (Size::new(800.0, 300.0), Size::new(420.0, 240.0), 1),
        ];
        for (main, settings, columns) in table {
            let size = window_size_for(main);
            assert_eq!(size, settings, "{main:?}");
            assert_eq!(Columns::of(size.width).count, columns, "{main:?}");
        }
    }
}
