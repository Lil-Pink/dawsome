//! The dividers between the stacked panels: a press on one holds it, the cursor moves the line
//! ([`panels::dragged`]), the release keeps the heights it left in the settings; a double-click
//! puts the two panels back ([`panels::reset`]). The heights live in the settings alone
//! ([`crate::settings::Settings::kept_heights`]), which the stack reads every time it is laid
//! out ([`App::panels_view`]).
//!
//! Dragged sideways instead ([`POP_OUT_REACH`] px, more than it moved up or down), the divider
//! is armed: the line goes back to where it was pressed, the strip lights, a window's outline
//! follows the cursor and the hint bar says what the release does; let go there, the panel
//! under the line pops out of the stack into a window of its own, placed at the cursor
//! (`App::pop_out`). Back within the reach, the line follows the cursor again. Esc while the
//! divider is held puts the line back and ends the drag.

use iced::widget::{canvas, column, container, pin, text};
use iced::{Element, Fill, Point, Vector};

use super::{App, Message};
use crate::ui::grip::{Edge, Grip};
use crate::ui::hint::Hover;
use crate::ui::mixer_layout;
use crate::ui::panels::{self, Kept, Panel, DIVIDER_BAND};
use crate::ui::style;

/// How far sideways a held divider is dragged before its release pops the panel under it out,
/// while it moved less up or down: past the few pixels a hand drifts while resizing, and well
/// under a panel's width.
pub const POP_OUT_REACH: f32 = 40.0;

/// The outline that follows the cursor while a divider is armed: a small window's.
const OUTLINE_SIZE: iced::Size = iced::Size::new(220.0, 120.0);

/// What a divider reports, each by the panel it is over (the one under the line).
#[derive(Debug, Clone, PartialEq)]
pub enum DividerMessage {
    /// A press on the divider over `lower`, at a window point.
    Pressed {
        lower: Panel,
        at: Point,
    },
    /// The cursor at a window point while a divider is held.
    Dragged(Point),
    Released,
    /// A double-click on the divider over this panel.
    Reset(Panel),
}

/// The divider held: the one over `lower`, pressed at window point `from` while `kept` was
/// kept, the cursor now at `at`. Why the heights at the press: each move is measured from
/// there, so the line stays under the cursor whatever the moves before it clamped, and Esc or
/// an armed drag puts them back.
#[derive(Debug, Clone, Copy)]
pub(super) struct DividerDrag {
    lower: Panel,
    from: Point,
    kept: Kept,
    at: Point,
}

impl DividerDrag {
    /// Whether the release pops the panel out: the cursor [`POP_OUT_REACH`] or more sideways
    /// from the press, and less than that far up or down.
    pub(super) fn armed(&self) -> bool {
        pops_out(self.at - self.from)
    }
}

/// Whether a divider dragged `moved` from its press is armed to pop its panel out.
fn pops_out(moved: Vector) -> bool {
    moved.x.abs() >= POP_OUT_REACH && moved.y.abs() < moved.x.abs()
}

impl App {
    pub(super) fn divider_message(&mut self, message: DividerMessage) {
        match message {
            DividerMessage::Pressed { lower, at } => {
                self.divider = Some(DividerDrag {
                    lower,
                    from: at,
                    kept: self.settings.kept_heights(),
                    at,
                });
            }
            DividerMessage::Dragged(at) => {
                let Some(mut drag) = self.divider else {
                    return;
                };
                let stacked: Vec<Panel> = self.panels.stacked().collect();
                // A panel opened or closed while it was held: that line is gone.
                let Some(line) = divider_at(&stacked, drag.lower) else {
                    self.divider = None;
                    return;
                };
                drag.at = at;
                self.divider = Some(drag);
                if drag.armed() {
                    self.keep_heights(&drag.kept);
                    return;
                }
                let kept = panels::dragged(
                    &stacked,
                    self.rack_content(),
                    &drag.kept,
                    self.panels_height(),
                    line,
                    at.y - drag.from.y,
                );
                self.keep_heights(&kept);
            }
            DividerMessage::Released => {
                let Some(drag) = self.divider.take() else {
                    return;
                };
                if drag.armed() {
                    self.pop_out(drag.lower, drag.at);
                }
                self.save_settings();
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

    /// Esc while a divider is held: the heights as they were at its press, and the drag over.
    /// Whether one was held.
    pub(super) fn cancel_divider(&mut self) -> bool {
        let Some(drag) = self.divider.take() else {
            return false;
        };
        self.keep_heights(&drag.kept);
        true
    }

    /// The divider held and armed to pop its panel out, if one is: the panel.
    pub(super) fn popping(&self) -> Option<Panel> {
        self.divider
            .filter(DividerDrag::armed)
            .map(|drag| drag.lower)
    }

    /// The outline of the window the panel would pop out into, at the cursor while a divider is
    /// armed: the panel's name and what letting go does, in an accent frame. Why at the cursor:
    /// the window opens there.
    pub(super) fn pop_out_outline(&self) -> Option<Element<'_, Message>> {
        let drag = self.divider.filter(DividerDrag::armed)?;
        let words = column![
            text(drag.lower.title()).size(15).color(style::TEXT),
            text("Release: its own window")
                .size(12)
                .color(style::TEXT_DIM),
        ]
        .spacing(4)
        .align_x(iced::Center);
        let outline = container(words)
            .center_x(OUTLINE_SIZE.width)
            .center_y(OUTLINE_SIZE.height)
            .style(style::pop_out_outline);
        Some(pin(outline).x(drag.at.x).y(drag.at.y).into())
    }

    /// Keeps `kept` in the settings, the one place the heights live, and lays the mixer's
    /// desk out at once from the height the stack now shows it.
    pub(super) fn keep_heights(&mut self, kept: &Kept) {
        self.settings.set_kept_heights(kept);
        let desk = mixer_layout::desk_height(self.mixer_panel_height());
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
        let held = self.divider.filter(|drag| drag.lower == panel);
        let grip = Grip {
            held: held.is_some(),
            armed: held.is_some_and(|drag| drag.armed()),
            pressed: move |at| Message::Divider(DividerMessage::Pressed { lower: panel, at }),
            dragged: |at| Message::Divider(DividerMessage::Dragged(at)),
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
    use iced::keyboard::key::{Code, Named};
    use iced::{mouse, window, Point, Size};

    use super::super::tests::app_on;
    use super::super::widget_events::Window;
    use super::super::{App, Message, Panel, WindowChange};
    use super::DividerMessage;
    use crate::settings::WindowPlace;
    use crate::ui::layout::TOOLBAR_HEIGHT;
    use crate::ui::mixer_layout::{self as mix, DEFAULT_HEIGHT};
    use crate::ui::panels::{stack_heights, Kept, Popped};

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
    // tests): the playlist 355, the roll 356, the mixer 225. The band over the roll starts at
    // y 64 + 355 = 419; dragged 50 px down the playlist is 405 and the roll 306, kept in the
    // settings, its band held while the cursor moves (lit) and let go of after. The band over
    // the mixer then starts at 64 + 405 + 306 = 775: dragged 20 px up the roll is 286 and the
    // mixer 245, its desk laid out from that at once (239). Its line now at 755, a double-click
    // on it puts both back, 355, 356, 225, the desk its least, 219. (Pins moved: the playlist
    // 8 px taller under the 64 px toolbar than under the 72 px one, 214 and 264; the
    // mixer's default 400 → 310: were 222, 286, 272, 600, 420, 580; then 310 → 225 with its
    // compact strip, the playlist 43 px taller and the roll 42: were 312, 314, 310, 376, 362,
    // 264, 690, 244, 330, 324, 670, 304.)
    #[test]
    fn the_lines_between_the_playlist_the_roll_and_the_mixer_drag_and_reset() {
        let (mut app, _player) = three_panels();
        assert_eq!(shown(&app), [355.0, 356.0, 225.0]);
        let mut window = Window::new(&app);
        let roll_line = TOOLBAR_HEIGHT + 355.0;
        assert_eq!(roll_line, 419.0);
        let from = on_band(roll_line);
        let published = window.drag(&mut app, from, Point::new(from.x, from.y + 50.0));
        assert!(
            said(
                &published,
                &DividerMessage::Pressed {
                    lower: Panel::PianoRoll,
                    at: from
                }
            ),
            "{published:?}"
        );
        assert!(said(&published, &DividerMessage::Released), "{published:?}");
        assert!(app.divider.is_none(), "let go of");
        assert_eq!(shown(&app), [405.0, 306.0, 225.0]);
        assert_eq!(
            app.settings.kept_heights(),
            Kept {
                piano_roll: Some(306.0),
                ..Kept::default()
            }
        );
        let mixer_line = TOOLBAR_HEIGHT + 405.0 + 306.0;
        assert_eq!(mixer_line, 775.0);
        let from = on_band(mixer_line);
        window.drag(&mut app, from, Point::new(from.x, from.y - 20.0));
        assert_eq!(shown(&app), [405.0, 286.0, 245.0]);
        assert_eq!(app.settings.kept_heights().mixer, Some(245.0));
        assert_eq!(app.mixer.size().height, mix::desk_height(245.0));
        assert_eq!(app.mixer.size().height, 239.0);
        let at = on_band(TOOLBAR_HEIGHT + 405.0 + 286.0);
        window.click(&mut app, at);
        let published = window.click(&mut app, at);
        let reset = DividerMessage::Reset(Panel::Mixer);
        assert!(said(&published, &reset), "{published:?}");
        assert_eq!(shown(&app), [355.0, 356.0, 225.0]);
        assert_eq!(
            app.settings.kept_heights(),
            Kept {
                piano_roll: None,
                mixer: None,
                ..Kept::default()
            }
        );
        assert_eq!(app.mixer.size().height, mix::desk_height(DEFAULT_HEIGHT));
        assert_eq!(app.mixer.size().height, mix::LEAST_DESK);
    }

    // The mixer's handle, now the divider over the mixer: p5_gain with the rack
    // and the mixer in the 1000 × 560 window, 496 px under the 64 px toolbar; the rack (the
    // flexible panel) shows 271 px and keeps at least its header and a row, 70. Pressed on the
    // line (y 64 + 271 + 3 = 338): dragged 60 px up the mixer is 285, 220 up 426 (the rack at
    // its least), 100 down 225 (the mixer's least, every strip whole: its desk the least desk,
    // 219) (pins moved: 488 px under the 72 px toolbar, the rack 88, the mixer 418 and the rack
    // 308 at the end; the mixer's default 400 → 310 and its least 180 → 310: the
    // rack showed 96, 40 up gave 426, 200 down 200 and 300 down 180, its desk scrolled; then
    // 310 → 225 with its compact strip: the rack showed 186, pressed at 253, 60 up gave 370, 120
    // up 426, 100 down 310). The release keeps the height in the settings; a move after it does
    // nothing. With the mixer alone there is no divider: its top edge drags nothing.
    #[test]
    fn the_divider_over_the_mixer_sets_its_height_between_its_least_and_the_room() {
        let (mut app, _player) = app_on(&Path::new(FIXTURES).join("p5_gain.json"));
        let _ = app.update(Message::TogglePanel(Panel::Mixer));
        app.hints_flag = Some(false);
        assert!(app.panels.rack && app.panels.mixer);
        assert_eq!(shown(&app), [271.0, 225.0]);
        let divider = |app: &mut App, message| {
            let _ = app.update(Message::Divider(message));
        };
        let pressed = DividerMessage::Pressed {
            lower: Panel::Mixer,
            at: Point::new(500.0, 338.0),
        };
        divider(&mut app, pressed);
        assert!(app.divider.is_some());
        let mixer = |app: &App| app.settings.kept_heights().mixer_height();
        divider(&mut app, DividerMessage::Dragged(Point::new(500.0, 278.0)));
        assert_eq!(mixer(&app), 285.0);
        assert_eq!(app.mixer.size().height, mix::desk_height(285.0));
        divider(&mut app, DividerMessage::Dragged(Point::new(500.0, 118.0)));
        assert_eq!(mixer(&app), 426.0);
        divider(&mut app, DividerMessage::Dragged(Point::new(500.0, 438.0)));
        assert_eq!(mixer(&app), 225.0);
        assert_eq!(app.mixer.size().height, mix::LEAST_DESK);
        divider(&mut app, DividerMessage::Released);
        assert!(app.divider.is_none());
        assert_eq!(shown(&app), [271.0, 225.0]);
        divider(&mut app, DividerMessage::Dragged(Point::new(500.0, 100.0)));
        assert_eq!(mixer(&app), 225.0, "a move after the release does nothing");
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
    // the window, its desk the 490 px under the toolbar and its 6 px edge (560 − 64 − 6; the
    // ui-check's mixer probes read it), and under the rack the 219 px of its 225 less its band
    // (the ui-check run before the fix found the lone desk left at 394, the window's foot empty;
    // 394 of 400 under the rack at first, then 304 of 310). The desk reports the size
    // it was laid out at on an event that carries no input.
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

    /// The add menu of insert 12 (no effects: its add tile is its first block), opened as a
    /// press on the tile opens it, laid out on the window's top layer: where it is, and that
    /// each of its fourteen rows, clicked there, sends its place in the menu.
    fn add_menu_of_insert_12(app: &mut App, window: &mut Window) -> iced::Rectangle {
        use dawsome_engine::project::InsertId;

        use crate::ui::mixer_state::MixerMessage;
        assert!(app.open_slot_menu(InsertId(12), 0));
        let menus = window.overlays(app);
        let [menu] = menus[..] else {
            panic!("one menu on the window's top layer: {menus:?}");
        };
        for row in 0..14 {
            let at = Point::new(menu.center_x(), menu.y + 4.0 + 26.0 * (row as f32 + 0.5));
            let published = window.click_overlay(app, at);
            assert!(
                matches!(
                    published.as_slice(),
                    [Message::Mixer(MixerMessage::MenuChosen(chosen))] if *chosen == row
                ),
                "row {row}: {published:?}"
            );
        }
        menu
    }

    // A layout that cut the mixer off, through the window's own view: p5_gain (rack content 24 +
    // 8 + 6 × 38 = 260) with the rack, the playlist, the piano roll and the mixer in 1600 × 900
    // with the hint bar, 814 px under the toolbar. By hand: the least heights, 70 + 180 + 230 +
    // 225 = 705, fit with more than the mixer's 85 to spare, so the playlist keeps its floor,
    // 180 + 43 = 223, and the rack (260), the roll (356: an octave, its 42 beside the mixer, its
    // band; its floor 230 + 42 = 272) and the mixer (225) each keep 24 / 274 of what they have
    // above their floor: 86.642, 279.358, 225, none under its least. In whole pixels (86, 223,
    // 279) the mixer starts at 64 + 588 = 652 and takes the 226 left: its desk, under
    // its 6 px edge at 658, is laid out 219 tall and stretched to the 220 above the hint bar's
    // top (878), its strips' foot at 658 + 218 = 876. Insert 12's add tile sits at the effects
    // area's (1068 + 6, 52), so its menu, 188 × 372, opens at window (1080, 658 + 52 + 22) =
    // (1080, 732): down it would end at 1104, up start at 360, above the mixer, and the 220 px
    // under the mixer's top hold no 372, so it ends on the hint bar's top, from 878 − 372 =
    // 506, every row inside the window. (Before the mixer fit: the mixer showed 248.6 px of a
    // 394 px desk, the strips cut under the fader, and the menu ran from 635 to 1007, its last
    // four rows past the window. With the 310 px mixer: 86.642, 237.358, its desk from 573, 305
    // tall, the strips' foot at 874, the menu at 647, then 506.) With the mixer's divider having
    // left it 400 px, a height kept, the floors are the least heights and the playlist keeps
    // 180; the three keep 109 / 491 of what they have above their least: the mixer
    // 263.849, the rack 112.179, the roll 257.971; in whole pixels (112, 180, 257) it starts at
    // 613 and takes the 265 left, its desk laid out from what it shows, 259 under its edge, not
    // from the 400 kept (a 394 px desk scrolled in a shorter panel was the strips cut off).
    #[test]
    fn four_panels_at_1600_by_900_show_the_mixers_strips_and_add_menu_whole() {
        let (mut app, _player) = app_on(&Path::new(FIXTURES).join("p5_gain.json"));
        let _ = app.update(Message::WindowResized(Size::new(1600.0, 900.0)));
        app.hints_flag = Some(true);
        for panel in [Panel::Playlist, Panel::PianoRoll, Panel::Mixer] {
            let _ = app.update(Message::TogglePanel(panel));
        }
        let stacked: Vec<Panel> = app.panels.stacked().collect();
        assert_eq!(stacked, Panel::STACKED, "nothing closed to make room");
        assert_eq!(app.panels_height(), 814.0);
        assert_eq!(app.rack_content(), 260.0);
        let heights = shown(&app);
        let share = 24.0 / 274.0;
        let expected = [70.0 + 190.0 * share, 223.0, 272.0 + 84.0 * share, 225.0];
        for (height, expected) in heights.iter().zip(expected) {
            assert!((height - expected).abs() < 1e-3, "{heights:?}");
        }
        for (height, least) in heights.iter().zip([70.0, 180.0, 230.0, 225.0]) {
            assert!(*height >= least, "{heights:?}");
        }
        let mut window = Window::new(&app);
        window.mouse(
            &mut app,
            mouse::Event::CursorLeft,
            mouse::Cursor::Unavailable,
        );
        assert_eq!(app.mixer.size().height, 220.0, "the desk as laid out");
        let desk_top = TOOLBAR_HEIGHT + 86.0 + 223.0 + 279.0 + mix::HANDLE_HEIGHT;
        assert_eq!(desk_top, 658.0);
        let strips_foot = desk_top + mix::body_bottom(app.mixer.size().height);
        assert_eq!(strips_foot, 876.0);
        assert!(strips_foot <= 900.0 - crate::ui::layout::HINT_BAR_HEIGHT);
        let menu = add_menu_of_insert_12(&mut app, &mut window);
        assert_eq!(
            menu,
            iced::Rectangle::new(Point::new(1080.0, 506.0), Size::new(188.0, 372.0))
        );
        app.mixer.menu = None;
        app.settings.set_kept_heights(&Kept {
            mixer: Some(400.0),
            ..Kept::default()
        });
        let heights = shown(&app);
        let share = 109.0 / 491.0;
        let expected = [
            70.0 + 190.0 * share,
            180.0,
            230.0 + 126.0 * share,
            225.0 + 175.0 * share,
        ];
        for (height, expected) in heights.iter().zip(expected) {
            assert!((height - expected).abs() < 1e-3, "{heights:?}");
        }
        let mut window = Window::new(&app);
        window.mouse(
            &mut app,
            mouse::Event::CursorLeft,
            mouse::Cursor::Unavailable,
        );
        assert_eq!(app.mixer.size().height, 259.0, "the desk the mixer shows");
    }

    // A window made too short for the four panels' least heights: p5_gain's four panels opened
    // in 1000 × 1000, then the window made 1000 × 600 with the hint bar, 514 px under the
    // toolbar for 705 px of least heights. By hand: each panel shows at its least (70, 180, 230,
    // 225) and the stack scrolls; the mixer, from 64 + 480 = 544, has its 219 px desk laid out
    // whole. The wheel over the line above the playlist (y 134..140, where no panel takes the
    // wheel) scrolls the stack to its end, 705 − 514 = 191 px: the mixer from 353, its desk
    // from 359 to 578, the hint bar's top. Insert 12's add tile at the effects area's (668 + 6,
    // 52) opens its menu at (680, 359 + 74) = (680, 433): down it would end at 805, up start at
    // 61, above the mixer, and the 219 px under the mixer's top hold no 372, so it ends on the
    // hint bar's top, from 578 − 372 = 206, every row inside the window. (Before: the four were
    // squeezed into the 514 px, the mixer 118.9 px over a 394 px desk; with the 310 px mixer
    // the stack scrolled 276 px and the menu opened at 348, then 206.)
    #[test]
    fn a_window_too_short_scrolls_the_stack_and_keeps_the_add_menu_inside() {
        let (mut app, _player) = app_on(&Path::new(FIXTURES).join("p5_gain.json"));
        let _ = app.update(Message::WindowResized(Size::new(1000.0, 1000.0)));
        for panel in [Panel::Playlist, Panel::PianoRoll, Panel::Mixer] {
            let _ = app.update(Message::TogglePanel(panel));
        }
        app.hints_flag = Some(true);
        let _ = app.update(Message::WindowResized(Size::new(1000.0, 600.0)));
        let stacked: Vec<Panel> = app.panels.stacked().collect();
        assert_eq!(stacked, Panel::STACKED, "a resize closes nothing");
        assert_eq!(app.panels_height(), 514.0);
        assert!(!crate::ui::panels::fits(
            &stacked,
            app.rack_content(),
            514.0
        ));
        assert_eq!(shown(&app), [70.0, 180.0, 230.0, 225.0]);
        let mut window = Window::new(&app);
        window.mouse(
            &mut app,
            mouse::Event::CursorLeft,
            mouse::Cursor::Unavailable,
        );
        assert_eq!(app.mixer.size().height, mix::LEAST_DESK);
        window.wheel(
            &mut app,
            Point::new(500.0, TOOLBAR_HEIGHT + 70.0 + 3.0),
            -10.0,
        );
        let menu = add_menu_of_insert_12(&mut app, &mut window);
        assert_eq!(
            menu,
            iced::Rectangle::new(Point::new(680.0, 206.0), Size::new(188.0, 372.0))
        );
        assert!(menu.y >= 0.0 && menu.y + menu.height <= 600.0 - 22.0);
    }

    // A panel closed while a divider is held re-pairs the lines: the line over the roll held,
    // the playlist closed leaves the roll at the top, with no line over it, so the next move
    // ends the drag and changes nothing.
    #[test]
    fn a_divider_whose_line_is_gone_drags_nothing() {
        let (mut app, _player) = three_panels();
        let _ = app.update(Message::Divider(DividerMessage::Pressed {
            lower: Panel::PianoRoll,
            at: Point::new(500.0, 289.0),
        }));
        let _ = app.update(Message::TogglePanel(Panel::Playlist));
        let before = app.settings.kept_heights();
        let _ = app.update(Message::Divider(DividerMessage::Dragged(Point::new(
            500.0, 339.0,
        ))));
        assert!(
            app.divider.is_none(),
            "the roll is at the top: no line over it"
        );
        assert_eq!(app.settings.kept_heights(), before);
    }

    /// The line over the roll in [`three_panels`] (64 + 355 = 419), pressed half way down its
    /// band.
    const ROLL_LINE: Point = Point::new(500.0, 422.0);

    /// A press, then each of `moves`, delivered through the main window's widgets as the window
    /// delivers them; the button still held.
    fn press_and_move(app: &mut App, window: &mut Window, from: Point, moves: &[Point]) {
        let at = mouse::Cursor::Available(from);
        window.mouse(app, mouse::Event::ButtonPressed(mouse::Button::Left), at);
        for &to in moves {
            let cursor = mouse::Cursor::Available(to);
            window.mouse(app, mouse::Event::CursorMoved { position: to }, cursor);
        }
    }

    /// The left button let go of at `at`, through the widgets: the messages they published.
    fn release(app: &mut App, window: &mut Window, at: Point) -> Vec<Message> {
        let cursor = mouse::Cursor::Available(at);
        window.mouse(
            app,
            mouse::Event::ButtonReleased(mouse::Button::Left),
            cursor,
        )
    }

    // Held and moved 30 px down with no sideways move, the divider resizes as before: the
    // playlist 355 + 30 = 385, the roll 356 − 30 = 326 (kept), the mixer 225; nothing pops.
    #[test]
    fn a_divider_dragged_up_or_down_resizes_and_pops_nothing() {
        let (mut app, _player) = three_panels();
        let mut window = Window::new(&app);
        let to = Point::new(ROLL_LINE.x, ROLL_LINE.y + 30.0);
        window.drag(&mut app, ROLL_LINE, to);
        assert_eq!(shown(&app), [385.0, 326.0, 225.0]);
        assert_eq!(app.settings.kept_heights().piano_roll, Some(326.0));
        assert_eq!(app.panels.popped, Popped::NONE);
        assert_eq!(app.panel_windows.window(Panel::PianoRoll), None);
    }

    // Held and moved 45 px sideways (past the 40, more than its 0 up or down), the divider is
    // armed: the line stays where it was pressed (355, 356, 225), its strip lit, the outline at
    // the cursor, the hint bar saying what letting go does. Let go of there, the roll pops out:
    // still open, out of the stack, a window of its own titled as the others are, which the app
    // does not take for the main one; the playlist and the mixer share the 936 px as though
    // the roll were closed. By hand: their least 180 + 225 = 405 fit; the playlist is flexible
    // and, beside the mixer without the roll, its floor holds all of the 85 px the mixer gave
    // up (180 + 85 = 265); the mixer keeps its 225; the playlist takes the 711 left. The
    // window opens as the stack showed the roll, 1000 × 356, its top-left corner where the
    // divider was let go of on the screen: the main window at (100, 50) and the cursor at
    // (545, 422) in it, (645, 472). Nothing kept moved.
    #[test]
    fn a_divider_dragged_sideways_pops_the_panel_under_it_out() {
        let (mut app, _player) = three_panels();
        let main = window::Id::unique();
        app.set_main_window(main);
        let _ = app.update(Message::Windowed(
            main,
            WindowChange::Moved(Point::new(100.0, 50.0)),
        ));
        let mut window = Window::new(&app);
        let to = Point::new(ROLL_LINE.x + 45.0, ROLL_LINE.y);
        press_and_move(&mut app, &mut window, ROLL_LINE, &[to]);
        assert_eq!(app.popping(), Some(Panel::PianoRoll));
        assert_eq!(shown(&app), [355.0, 356.0, 225.0]);
        assert!(app.pop_out_outline().is_some());
        assert!(
            app.hint_words()
                .starts_with("Piano roll \u{b7} release to pop the panel out"),
            "{}",
            app.hint_words()
        );
        let published = release(&mut app, &mut window, to);
        assert!(said(&published, &DividerMessage::Released), "{published:?}");
        assert!(app.divider.is_none());
        assert!(app.panels.piano_roll && app.panels.popped.has(Panel::PianoRoll));
        let stacked: Vec<Panel> = app.panels.stacked().collect();
        assert_eq!(stacked, [Panel::Playlist, Panel::Mixer]);
        assert_eq!(shown(&app), [711.0, 225.0]);
        let id = app
            .panel_windows
            .window(Panel::PianoRoll)
            .expect("its window");
        assert!(!app.is_main_window(id));
        assert_eq!(app.window_title(id), "Piano roll - Dawsome");
        assert_eq!(
            app.panel_windows.place(Panel::PianoRoll),
            Some(WindowPlace::of(
                Size::new(1000.0, 356.0),
                Some(Point::new(645.0, 472.0))
            ))
        );
        assert_eq!(app.focus, Panel::PianoRoll);
        assert_eq!(app.settings.kept_heights(), Kept::default());
        assert!(
            app.pop_out_outline().is_none(),
            "the outline goes with the drag"
        );
    }

    // 45 px sideways and 30 down (30 < 45) is armed, the line at its press; back to 10 px
    // sideways and 20 down (20 > 10) is not, and the line follows the cursor again: the
    // playlist 375, the roll 336; let go of there it resizes (the roll's 336 kept) and pops
    // nothing.
    #[test]
    fn a_divider_brought_back_within_the_reach_resizes_again() {
        let (mut app, _player) = three_panels();
        let mut window = Window::new(&app);
        let armed = Point::new(ROLL_LINE.x + 45.0, ROLL_LINE.y + 30.0);
        press_and_move(&mut app, &mut window, ROLL_LINE, &[armed]);
        assert_eq!(app.popping(), Some(Panel::PianoRoll));
        assert_eq!(shown(&app), [355.0, 356.0, 225.0]);
        let back = Point::new(ROLL_LINE.x + 10.0, ROLL_LINE.y + 20.0);
        let cursor = mouse::Cursor::Available(back);
        window.mouse(
            &mut app,
            mouse::Event::CursorMoved { position: back },
            cursor,
        );
        assert_eq!(app.popping(), None);
        assert_eq!(shown(&app), [375.0, 336.0, 225.0]);
        release(&mut app, &mut window, back);
        assert_eq!(app.panels.popped, Popped::NONE);
        assert_eq!(app.settings.kept_heights().piano_roll, Some(336.0));
        assert_eq!(app.panel_windows.window(Panel::PianoRoll), None);
    }

    // Esc while a divider is held puts the line back where it was pressed and ends the drag:
    // moved 50 px down (405, 306), Esc gives 355, 356, 225 with nothing kept, and letting go
    // after does nothing. Armed, Esc likewise: letting go pops nothing.
    #[test]
    fn esc_while_a_divider_is_held_puts_it_back() {
        let (mut app, _player) = three_panels();
        let mut window = Window::new(&app);
        let down = Point::new(ROLL_LINE.x, ROLL_LINE.y + 50.0);
        press_and_move(&mut app, &mut window, ROLL_LINE, &[down]);
        assert_eq!(shown(&app), [405.0, 306.0, 225.0]);
        window.tap(&mut app, Named::Escape, Code::Escape);
        assert!(app.divider.is_none());
        assert_eq!(shown(&app), [355.0, 356.0, 225.0]);
        assert_eq!(app.settings.kept_heights(), Kept::default());
        let published = release(&mut app, &mut window, down);
        assert!(!published
            .iter()
            .any(|message| matches!(message, Message::Divider(_))));
        assert_eq!(shown(&app), [355.0, 356.0, 225.0]);
        // Pressed again 100 px along the line: at the same point soon after, the press would
        // be the second of a double-click, which resets the line.
        let again = Point::new(ROLL_LINE.x + 100.0, ROLL_LINE.y);
        let sideways = Point::new(again.x + 60.0, again.y);
        press_and_move(&mut app, &mut window, again, &[sideways]);
        assert_eq!(app.popping(), Some(Panel::PianoRoll));
        window.tap(&mut app, Named::Escape, Code::Escape);
        release(&mut app, &mut window, sideways);
        assert_eq!(app.panels.popped, Popped::NONE);
        assert_eq!(app.panel_windows.window(Panel::PianoRoll), None);
        assert_eq!(shown(&app), [355.0, 356.0, 225.0]);
    }

    // p5_gain's four panels at 1600 × 900 (86.642, 223, 279.358, 225; the roll's
    // line at 64 + 86 + 223 = 373), the roll popped by its divider dragged 60 px sideways (the
    // drag's half way, 30 px, still resizes: by nothing). The other three share the 814 px by
    // the least-height rule as though the roll were closed. By hand: least 70 + 180 + 225 = 475
    // fit; nothing kept, so the playlist's floor holds the mixer's 85 (no roll beside it), 265;
    // the rack its content, 260 (under half of 814), the mixer 225, the playlist the 329 left.
    // Its window opens as the stack showed it, 1600 × 279 (whole pixels), and nothing is kept.
    #[test]
    fn three_panels_share_the_stack_once_the_fourth_pops_out() {
        let (mut app, _player) = app_on(&Path::new(FIXTURES).join("p5_gain.json"));
        let _ = app.update(Message::WindowResized(Size::new(1600.0, 900.0)));
        app.hints_flag = Some(true);
        for panel in [Panel::Playlist, Panel::PianoRoll, Panel::Mixer] {
            let _ = app.update(Message::TogglePanel(panel));
        }
        assert_eq!(app.panels.stacked().count(), 4);
        let mut window = Window::new(&app);
        let from = on_band(TOOLBAR_HEIGHT + 86.0 + 223.0);
        window.drag(&mut app, from, Point::new(from.x + 60.0, from.y));
        let stacked: Vec<Panel> = app.panels.stacked().collect();
        assert_eq!(stacked, [Panel::Rack, Panel::Playlist, Panel::Mixer]);
        assert_eq!(shown(&app), [260.0, 329.0, 225.0]);
        assert_eq!(app.settings.kept_heights(), Kept::default());
        assert_eq!(
            app.panel_windows.place(Panel::PianoRoll),
            Some(WindowPlace::of(Size::new(1600.0, 279.0), None))
        );
        assert_eq!(app.mixer.size().height, mix::desk_height(225.0));
    }
}
