//! The dividers between the stacked panels: a press on one holds it, the cursor moves the line
//! ([`panels::dragged`]), the release keeps the heights it left in the settings; a double-click
//! puts the two panels back ([`panels::reset`]). The heights live in the settings alone
//! ([`crate::settings::Settings::kept_heights`]), which the stack reads every time it is laid
//! out ([`App::panels_view`]).

use iced::widget::canvas;
use iced::{Element, Fill};

use super::{App, Message};
use crate::ui::grip::{Edge, Grip};
use crate::ui::hint::Hover;
use crate::ui::mixer_layout;
use crate::ui::panels::{self, Kept, Panel, DIVIDER_BAND};

/// What a divider reports, each by the panel it is over (the one under the line).
#[derive(Debug, Clone, PartialEq)]
pub enum DividerMessage {
    /// A press on the divider over `lower`, at window y.
    Pressed {
        lower: Panel,
        y: f32,
    },
    /// The cursor at window y while a divider is held.
    Dragged(f32),
    Released,
    /// A double-click on the divider over this panel.
    Reset(Panel),
}

/// The divider held: the one over `lower`, pressed at window y `from` while `kept` was kept.
/// Why the heights at the press: each move is measured from there, so the line stays under the
/// cursor whatever the moves before it clamped.
#[derive(Debug, Clone, Copy)]
pub(super) struct DividerDrag {
    lower: Panel,
    from: f32,
    kept: Kept,
}

impl App {
    pub(super) fn divider_message(&mut self, message: DividerMessage) {
        match message {
            DividerMessage::Pressed { lower, y } => {
                self.divider = Some(DividerDrag {
                    lower,
                    from: y,
                    kept: self.settings.kept_heights(),
                });
            }
            DividerMessage::Dragged(y) => {
                let Some(drag) = self.divider else {
                    return;
                };
                let stacked: Vec<Panel> = self.panels.stacked().collect();
                // A panel opened or closed while it was held: that line is gone.
                let Some(at) = divider_at(&stacked, drag.lower) else {
                    self.divider = None;
                    return;
                };
                let kept = panels::dragged(
                    &stacked,
                    self.rack_content(),
                    &drag.kept,
                    self.panels_height(),
                    at,
                    y - drag.from,
                );
                self.keep_heights(&kept);
            }
            DividerMessage::Released => {
                if self.divider.take().is_some() {
                    self.save_settings();
                }
            }
            DividerMessage::Reset(lower) => {
                self.divider = None;
                let stacked: Vec<Panel> = self.panels.stacked().collect();
                if let Some(at) = divider_at(&stacked, lower) {
                    let kept = panels::reset(&stacked, &self.settings.kept_heights(), at);
                    self.keep_heights(&kept);
                    self.save_settings();
                }
            }
        }
    }

    /// Keeps `kept` in the settings, the one place the heights live, and lays the mixer's
    /// desk out from its height at once.
    fn keep_heights(&mut self, kept: &Kept) {
        self.settings.set_kept_heights(kept);
        let desk = mixer_layout::desk_height(kept.mixer_height());
        self.mixer.set_desk_height(desk);
    }

    /// The band along the top of `panel`, stacked under `above` if a panel is: the divider
    /// that drags the line between them, or the mixer's plain top edge while it is alone (its
    /// desk is laid out under it); none over the top panel otherwise.
    pub(super) fn divider_view(
        &self,
        above: Option<Panel>,
        panel: Panel,
    ) -> Option<Element<'_, Message>> {
        let Some(upper) = above else {
            return (panel == Panel::Mixer).then(|| {
                canvas::Canvas::new(Edge)
                    .width(Fill)
                    .height(DIVIDER_BAND)
                    .into()
            });
        };
        let held = self.divider.is_some_and(|drag| drag.lower == panel);
        let grip = Grip {
            held,
            pressed: move |y| Message::Divider(DividerMessage::Pressed { lower: panel, y }),
            dragged: |y| Message::Divider(DividerMessage::Dragged(y)),
            released: Message::Divider(DividerMessage::Released),
            reset: Some(Message::Divider(DividerMessage::Reset(panel))),
            hover: Hover::Divider {
                upper,
                lower: panel,
            },
        };
        Some(
            canvas::Canvas::new(grip)
                .width(Fill)
                .height(DIVIDER_BAND)
                .into(),
        )
    }
}

/// Where in `stacked` the panel the divider over `lower` sits on is: none when it is closed or
/// at the top, with no divider over it.
fn divider_at(stacked: &[Panel], lower: Panel) -> Option<usize> {
    stacked
        .iter()
        .position(|&panel| panel == lower)
        .filter(|&at| at > 0)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use dawsome_engine::realtime::Player;
    use iced::{mouse, Point, Size};

    use super::super::tests::app_on;
    use super::super::widget_events::Window;
    use super::super::{App, Message, Panel};
    use super::DividerMessage;
    use crate::ui::layout::TOOLBAR_HEIGHT;
    use crate::ui::mixer_layout::{self as mix, DEFAULT_HEIGHT};
    use crate::ui::panels::{stack_heights, Kept};

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    /// The heights the stack shows now, top to bottom.
    fn shown(app: &App) -> Vec<f32> {
        let stacked: Vec<Panel> = app.panels.stacked().collect();
        let kept = app.settings.kept_heights();
        stack_heights(&stacked, app.rack_content(), &kept, app.panels_height())
    }

    /// p3_roll with the playlist, the piano roll and the mixer open (no rack) in a 1000 × 1000
    /// window, no hint bar: 928 px under the toolbar.
    fn three_panels() -> (App, Player) {
        let (mut app, player) = app_on(&Path::new(FIXTURES).join("p3_roll.json"));
        // The window first, so the mixer opens with room and closes nothing.
        let _ = app.update(Message::WindowResized(Size::new(1000.0, 1000.0)));
        for panel in [Panel::Rack, Panel::Playlist, Panel::PianoRoll, Panel::Mixer] {
            let _ = app.update(Message::TogglePanel(panel));
        }
        app.hints_flag = Some(false);
        let stacked: Vec<Panel> = app.panels.stacked().collect();
        assert_eq!(stacked, [Panel::Playlist, Panel::PianoRoll, Panel::Mixer]);
        (app, player)
    }

    /// Whether `published` holds the divider message `message`.
    fn said(published: &[Message], message: &DividerMessage) -> bool {
        published
            .iter()
            .any(|published| matches!(published, Message::Divider(said) if said == message))
    }

    /// A point on the band at window y `top` (the line's), half way down it.
    fn on_band(top: f32) -> Point {
        Point::new(500.0, top + 3.0)
    }

    // Through the widgets, as the window delivers a drag (the same known answers as panels'
    // tests): the playlist 222, the roll 314, the mixer 400. The band over the roll starts at
    // y 64 + 222 = 286; dragged 50 px down the playlist is 272 and the roll 264, kept in the
    // settings, its band held while the cursor moves (lit) and let go of after. The band over
    // the mixer then starts at 64 + 272 + 264 = 600: dragged 20 px up the roll is 244 and the
    // mixer 420, its desk laid out from that at once. Its line now at 580, a double-click on
    // it puts both back, 222, 314, 400. (Pins moved: the playlist 8 px taller under the 64 px
    // toolbar than under the 72 px one, 214 and 264.)
    #[test]
    fn the_lines_between_the_playlist_the_roll_and_the_mixer_drag_and_reset() {
        let (mut app, _player) = three_panels();
        assert_eq!(shown(&app), [222.0, 314.0, 400.0]);
        let mut window = Window::new(&app);
        let roll_line = TOOLBAR_HEIGHT + 222.0;
        assert_eq!(roll_line, 286.0);
        let from = on_band(roll_line);
        let published = window.drag(&mut app, from, Point::new(from.x, from.y + 50.0));
        assert!(
            said(
                &published,
                &DividerMessage::Pressed {
                    lower: Panel::PianoRoll,
                    y: from.y
                }
            ),
            "{published:?}"
        );
        assert!(said(&published, &DividerMessage::Released), "{published:?}");
        assert!(app.divider.is_none(), "let go of");
        assert_eq!(shown(&app), [272.0, 264.0, 400.0]);
        assert_eq!(
            app.settings.kept_heights(),
            Kept {
                piano_roll: Some(264.0),
                ..Kept::default()
            }
        );
        let mixer_line = TOOLBAR_HEIGHT + 272.0 + 264.0;
        assert_eq!(mixer_line, 600.0);
        let from = on_band(mixer_line);
        window.drag(&mut app, from, Point::new(from.x, from.y - 20.0));
        assert_eq!(shown(&app), [272.0, 244.0, 420.0]);
        assert_eq!(app.settings.kept_heights().mixer, Some(420.0));
        assert_eq!(app.mixer.size().height, mix::desk_height(420.0));
        let at = on_band(TOOLBAR_HEIGHT + 272.0 + 244.0);
        window.click(&mut app, at);
        let published = window.click(&mut app, at);
        let reset = DividerMessage::Reset(Panel::Mixer);
        assert!(said(&published, &reset), "{published:?}");
        assert_eq!(shown(&app), [222.0, 314.0, 400.0]);
        assert_eq!(
            app.settings.kept_heights(),
            Kept {
                piano_roll: None,
                mixer: None,
                ..Kept::default()
            }
        );
        assert_eq!(app.mixer.size().height, mix::desk_height(DEFAULT_HEIGHT));
    }

    // The mixer's handle, now the divider over the mixer: p5_gain with the rack
    // and the mixer in the 1000 × 560 window, 496 px under the 64 px toolbar; the rack (the
    // flexible panel) shows 96 px and keeps at least its header and a row, 70. Dragged 40 px up
    // the mixer is 426 (the rack at its least), 200 px down 200, 300 down 180 (the mixer's
    // least) (pins moved: 488 px under the 72 px toolbar, the rack 88, the mixer 418 and the rack
    // 308 at the end); the desk never shorter than its least (304 px: the panel scrolls it). The
    // release keeps the height in the settings; a move after it does nothing. With the mixer
    // alone there is no divider: its top edge drags nothing.
    #[test]
    fn the_divider_over_the_mixer_sets_its_height_between_its_least_and_the_room() {
        let (mut app, _player) = app_on(&Path::new(FIXTURES).join("p5_gain.json"));
        let _ = app.update(Message::TogglePanel(Panel::Mixer));
        app.hints_flag = Some(false);
        assert!(app.panels.rack && app.panels.mixer);
        assert_eq!(shown(&app), [96.0, 400.0]);
        let divider = |app: &mut App, message| {
            let _ = app.update(Message::Divider(message));
        };
        let pressed = DividerMessage::Pressed {
            lower: Panel::Mixer,
            y: 163.0,
        };
        divider(&mut app, pressed);
        assert!(app.divider.is_some());
        let mixer = |app: &App| app.settings.kept_heights().mixer_height();
        divider(&mut app, DividerMessage::Dragged(123.0));
        assert_eq!(mixer(&app), 426.0);
        divider(&mut app, DividerMessage::Dragged(363.0));
        assert_eq!(mixer(&app), 200.0);
        assert_eq!(app.mixer.size().height, mix::LEAST_DESK);
        divider(&mut app, DividerMessage::Dragged(463.0));
        assert_eq!(mixer(&app), 180.0);
        divider(&mut app, DividerMessage::Released);
        assert!(app.divider.is_none());
        assert_eq!(shown(&app), [316.0, 180.0]);
        divider(&mut app, DividerMessage::Dragged(100.0));
        assert_eq!(mixer(&app), 180.0, "a move after the release does nothing");
        // The mixer alone: its top edge is no divider, so a press there reports nothing.
        let _ = app.update(Message::TogglePanel(Panel::Rack));
        let mut window = Window::new(&app);
        let published = window.click(&mut app, on_band(TOOLBAR_HEIGHT));
        assert!(
            !published
                .iter()
                .any(|message| matches!(message, Message::Divider(_))),
            "{published:?}"
        );
    }

    // The mixer's top edge moved out of its own view into the stack's: alone it still fills
    // the window, its desk the 482 px under the toolbar and its 6 px edge (560 − 72 − 6; the
    // ui-check's mixer probes read it), and under the rack the 394 px of its 400 less its band
    // (the ui-check run before the fix found the lone desk left at 394, the window's foot empty).
    // The desk reports the size it was laid out at on an event that carries no input.
    #[test]
    fn the_mixer_desk_fills_the_panel_under_its_top_edge() {
        let (mut app, _player) = app_on(&Path::new(FIXTURES).join("p5_gain.json"));
        let _ = app.update(Message::TogglePanel(Panel::Mixer));
        app.hints_flag = Some(false);
        let laid_out = |app: &mut App| {
            let mut window = Window::new(app);
            window.mouse(app, mouse::Event::CursorLeft, mouse::Cursor::Unavailable);
            app.mixer.size().height
        };
        assert_eq!(laid_out(&mut app), mix::desk_height(DEFAULT_HEIGHT));
        let _ = app.update(Message::TogglePanel(Panel::Rack));
        assert!(!app.panels.rack && app.panels.mixer);
        assert_eq!(
            laid_out(&mut app),
            560.0 - TOOLBAR_HEIGHT - mix::HANDLE_HEIGHT
        );
    }

    // A panel closed while a divider is held re-pairs the lines: the line over the roll held,
    // the playlist closed leaves the roll at the top, with no line over it, so the next move
    // ends the drag and changes nothing.
    #[test]
    fn a_divider_whose_line_is_gone_drags_nothing() {
        let (mut app, _player) = three_panels();
        let _ = app.update(Message::Divider(DividerMessage::Pressed {
            lower: Panel::PianoRoll,
            y: 289.0,
        }));
        let _ = app.update(Message::TogglePanel(Panel::Playlist));
        let before = app.settings.kept_heights();
        let _ = app.update(Message::Divider(DividerMessage::Dragged(339.0)));
        assert!(
            app.divider.is_none(),
            "the roll is at the top: no line over it"
        );
        assert_eq!(app.settings.kept_heights(), before);
    }
}
