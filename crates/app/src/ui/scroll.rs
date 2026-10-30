//! One scroll model for the views that scroll along time: the piano roll, the playlist and the
//! rack's steps. A [`Span`] is how far the content reaches, how much of it the view shows and
//! where the view starts, in the view's own units (ticks for the timelines, pixels for the
//! rack's steps), so the rule is one and each view keeps its own mapping to pixels. The bar's
//! thumb is drawn from a span and a press or a drag on the bar maps back to an offset;
//! [`follow`] is the playhead's page jump; [`edge_step`] the scroll a drag held past a view's
//! edge gives on each tick. [`Scrollbar`] is the bar, one canvas program in the window's own
//! style: a thin trough and a thumb.

use iced::widget::canvas::{self, Event, Frame, Geometry, Path};
use iced::{mouse, Point, Rectangle, Renderer, Size, Theme};

use crate::app::Message;
use crate::ui::hint::{hover_change, Hover};
use crate::ui::style;

/// The bar's height: thin, and still easy to grab.
pub const BAR_HEIGHT: f32 = 10.0;
/// The thumb's inset from the trough's top and bottom.
pub const THUMB_INSET: f32 = 2.0;
/// The thumb is never shorter than this, so a long song keeps a thumb to grab.
pub const MIN_THUMB: f32 = 24.0;
/// A wheel notch over the bar moves the view by this share of it.
const WHEEL_SHARE: f64 = 0.125;
/// The thumb's corners.
const THUMB_RADIUS: f32 = 3.0;

/// One axis of a scrolling view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    /// Where the content ends, counted from 0.
    pub content: f64,
    /// Where the view starts.
    pub offset: f64,
    /// How much the view shows.
    pub view: f64,
}

impl Span {
    /// How far the bar reaches: the content's end, or the view's end when the view is scrolled
    /// past it (so the thumb then sits at the bar's end).
    pub fn extent(&self) -> f64 {
        self.content.max(self.offset + self.view)
    }

    /// The furthest the view can start inside the bar's reach.
    pub fn last_offset(&self) -> f64 {
        (self.extent() - self.view).max(0.0)
    }

    /// The thumb in a trough `length` pixels long: where it starts and how long it is. Its
    /// length is the view's share of the extent (at least [`MIN_THUMB`], at most the trough);
    /// its start is the offset's share of the extent left over, along the trough left over.
    pub fn thumb(&self, length: f32) -> (f32, f32) {
        let length = length.max(0.0);
        let extent = self.extent();
        if extent <= 0.0 || !extent.is_finite() {
            return (0.0, length);
        }
        let share = (self.view / extent).clamp(0.0, 1.0);
        let size = (share * f64::from(length)) as f32;
        let size = size.clamp(MIN_THUMB.min(length), length);
        let travel = length - size;
        let room = self.last_offset();
        let start = if room > 0.0 {
            (self.offset / room).clamp(0.0, 1.0) as f32 * travel
        } else {
            0.0
        };
        (start, size)
    }

    /// The offset whose thumb starts at `start` pixels along a trough `length` long: the
    /// inverse of [`Span::thumb`], kept to the bar's reach.
    pub fn offset_at(&self, start: f32, length: f32) -> f64 {
        let (_, size) = self.thumb(length);
        let travel = f64::from(length.max(0.0) - size);
        if travel <= 0.0 {
            return 0.0;
        }
        f64::from(start).clamp(0.0, travel) / travel * self.last_offset()
    }

    /// The offset a press in the trough beside the thumb gives: a page (the view's width)
    /// towards the press, kept to the bar's reach.
    pub fn paged(&self, forward: bool) -> f64 {
        self.moved(if forward { self.view } else { -self.view })
    }

    /// The offset a wheel notch over the bar gives: an eighth of the view, forward or back.
    pub fn wheeled(&self, forward: bool) -> f64 {
        let by = self.view * WHEEL_SHARE;
        self.moved(if forward { by } else { -by })
    }

    fn moved(&self, by: f64) -> f64 {
        (self.offset + by).clamp(0.0, self.last_offset())
    }
}

/// Where a view `view` long that starts at `offset` jumps to keep `at` (the playhead) in
/// sight: nowhere while `at` is in view; else to the start of the `unit` (a bar) holding `at`,
/// so the lines stay on the bars, or to `at` itself when a unit is wider than the view. A page
/// jump, not a scroll that moves every frame: the view stays put while the playhead crosses it.
pub fn follow(offset: f64, view: f64, at: f64, unit: f64) -> Option<f64> {
    if view <= 0.0 || (offset..offset + view).contains(&at) {
        return None;
    }
    let page = if unit > 0.0 {
        (at / unit).floor() * unit
    } else {
        at
    };
    let start = if at - page < view { page } else { at };
    Some(start.max(0.0))
}

/// A drag held up to this many pixels past a view's edge moves the view at [`EDGE_SLOW`]; from
/// [`EDGE_FAR`] past it, at [`EDGE_FAST`]; in between, the rate grows in step with the distance.
pub const EDGE_NEAR: f32 = 12.0;
pub const EDGE_FAR: f32 = 120.0;
/// Pixels the view moves on each tick with the pointer just past its edge, and at the most.
/// At the window's 33 ms ticks: about 120 px a second, two bars at the playlist's default zoom,
/// and about 1200 px a second.
pub const EDGE_SLOW: f32 = 4.0;
pub const EDGE_FAST: f32 = 40.0;

/// Pixels a view from `start` to `end` (along its axis) moves on one tick while a held drag has
/// the pointer at `at`: none while the pointer is inside it, its edges included; past an edge,
/// towards that edge (negative before `start`), [`EDGE_SLOW`] up to [`EDGE_NEAR`] past it, then
/// more with the distance, up to [`EDGE_FAST`] from [`EDGE_FAR`] past. Why it grows: the slow
/// rate brings a drag a little further at a time, the fast one crosses a song minutes long in a
/// few seconds.
pub fn edge_step(at: f32, start: f32, end: f32) -> f32 {
    let (past, towards) = if at < start {
        (start - at, -1.0)
    } else if at > end {
        (at - end, 1.0)
    } else {
        return 0.0;
    };
    let further = ((past - EDGE_NEAR) / (EDGE_FAR - EDGE_NEAR)).clamp(0.0, 1.0);
    towards * (EDGE_SLOW + (EDGE_FAST - EDGE_SLOW) * further)
}

/// What a wheel event of `x` across and `y` up or down moves: whether it is across, and by how
/// much (`x` across, else `y`). Across when it moves more across than up or down. Why not only
/// when `y` is 0: a trackpad's swipe across carries a little up or down in nearly every event,
/// which then scrolled the keys or the tracks instead of time.
pub fn wheel(x: f32, y: f32) -> (bool, f32) {
    if x.abs() > y.abs() {
        (true, x)
    } else {
        (false, y)
    }
}

/// What the bar asks its view for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BarMessage {
    /// Show the view from `offset`: a page, a wheel notch, or the thumb dragged, `held` while
    /// it is (so a playhead that follows waits until it is let go of).
    To { offset: f64, held: bool },
    /// The thumb was let go of.
    LetGo,
}

/// A horizontal scroll bar over `span`, asking through `message`. The trough is the canvas's
/// whole width.
pub struct Scrollbar {
    pub span: Span,
    pub message: fn(BarMessage) -> Message,
}

/// The bar's memory between events: where on the thumb a held press took it, with the span
/// as it was then (a drag maps by the bar it started on), and what the cursor was over.
#[derive(Debug, Default)]
pub struct BarState {
    grab: Option<(f32, Span)>,
    hover: Option<Hover>,
}

impl Scrollbar {
    /// The span the bar shows: its view's, but while its thumb is held the reach and the view
    /// as they were at the press, around the view's offset now, so the thumb drawn stays under
    /// the cursor (the drag maps by the same span). Why: a view scrolled past the content's
    /// end shrinks the bar's reach as it comes back inside it, and a thumb drawn from that ran
    /// ahead of the cursor; the reach shrinks once the thumb is let go of.
    pub fn shown(&self, state: &BarState) -> Span {
        match state.grab {
            Some((_, grabbed)) => Span {
                content: grabbed.extent(),
                offset: self.span.offset,
                view: grabbed.view,
            },
            None => self.span,
        }
    }
}

impl canvas::Program<Message> for Scrollbar {
    type State = BarState;

    fn update(
        &self,
        state: &mut BarState,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let send = |message| canvas::Action::publish((self.message)(message));
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let at = cursor.position_in(bounds)?;
                let (start, size) = self.span.thumb(bounds.width);
                let offset = if (start..start + size).contains(&at.x) {
                    state.grab = Some((at.x - start, self.span));
                    return Some(
                        send(BarMessage::To {
                            offset: self.span.offset,
                            held: true,
                        })
                        .and_capture(),
                    );
                } else {
                    self.span.paged(at.x >= start + size)
                };
                Some(
                    send(BarMessage::To {
                        offset,
                        held: false,
                    })
                    .and_capture(),
                )
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) if state.grab.is_some() => {
                let (grab, span) = state.grab?;
                let offset = span.offset_at(position.x - bounds.x - grab, bounds.width);
                Some(send(BarMessage::To { offset, held: true }))
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                if state.grab.is_some() =>
            {
                state.grab = None;
                Some(send(BarMessage::LetGo))
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                cursor.position_in(bounds)?;
                let (mouse::ScrollDelta::Lines { x, y } | mouse::ScrollDelta::Pixels { x, y }) =
                    *delta;
                // Down or right is later, as the timelines' Shift+wheel has it.
                let notch = if y != 0.0 { y } else { x };
                (notch != 0.0).then(|| {
                    let offset = self.span.wheeled(notch < 0.0);
                    send(BarMessage::To {
                        offset,
                        held: false,
                    })
                    .and_capture()
                })
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let now = cursor.position_in(bounds).map(|_| Hover::ScrollBar);
                hover_change(&mut state.hover, now)
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        state: &BarState,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), style::RACK_WELL);
        let (start, size) = self.shown(state).thumb(bounds.width);
        let over = cursor
            .position_in(bounds)
            .is_some_and(|at| (start..start + size).contains(&at.x));
        let color = if state.grab.is_some() || over {
            THUMB_HOT
        } else {
            THUMB
        };
        let thumb = Path::rounded_rectangle(
            Point::new(start, THUMB_INSET),
            Size::new(size, (bounds.height - 2.0 * THUMB_INSET).max(0.0)),
            THUMB_RADIUS.into(),
        );
        frame.fill(&thumb, color);
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &BarState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.grab.is_some() {
            return mouse::Interaction::Grabbing;
        }
        match cursor.position_in(bounds) {
            Some(_) => mouse::Interaction::Pointer,
            None => mouse::Interaction::default(),
        }
    }
}

/// The thumb, and the thumb under the cursor or held.
pub const THUMB: iced::Color = style::KNOB_RING;
pub const THUMB_HOT: iced::Color = style::TEXT_DIM;

#[cfg(test)]
mod tests {
    use super::*;

    fn span(content: f64, offset: f64, view: f64) -> Span {
        Span {
            content,
            offset,
            view,
        }
    }

    // By hand: a view of 250 of 1000 units in a 400 px trough is a 100 px thumb (a quarter);
    // its 300 px of travel stand for the 750 units the view can move, so offset 375 is 150 px
    // along and 750 the end. Scrolled past the content (offset 1000) the bar reaches 1250: an
    // 80 px thumb at the end of its 320 px of travel.
    #[test]
    fn the_thumb_is_the_views_share_where_the_view_is() {
        assert_eq!(span(1000.0, 0.0, 250.0).thumb(400.0), (0.0, 100.0));
        assert_eq!(span(1000.0, 375.0, 250.0).thumb(400.0), (150.0, 100.0));
        assert_eq!(span(1000.0, 750.0, 250.0).thumb(400.0), (300.0, 100.0));
        let past = span(1000.0, 1000.0, 250.0);
        assert_eq!(past.extent(), 1250.0);
        assert_eq!(past.thumb(400.0), (320.0, 80.0));
        // A long song keeps a thumb to grab; content that fits fills the trough.
        assert_eq!(span(100_000.0, 0.0, 100.0).thumb(400.0), (0.0, MIN_THUMB));
        assert_eq!(span(200.0, 0.0, 250.0).thumb(400.0), (0.0, 400.0));
        assert_eq!(span(0.0, 0.0, 0.0).thumb(400.0), (0.0, 400.0));
    }

    // The thumb dragged maps back: 150 px along is offset 375 again; past either end, the end.
    // A trough press beside the thumb pages a view's width towards it, a wheel notch an eighth
    // of it, both kept to the bar's reach.
    #[test]
    fn a_drag_a_page_and_a_notch_map_back_to_offsets() {
        let at = span(1000.0, 375.0, 250.0);
        assert_eq!(at.offset_at(150.0, 400.0), 375.0);
        assert_eq!(at.offset_at(-20.0, 400.0), 0.0);
        assert_eq!(at.offset_at(900.0, 400.0), 750.0);
        assert_eq!(at.paged(true), 625.0);
        assert_eq!(at.paged(false), 125.0);
        assert_eq!(span(1000.0, 600.0, 250.0).paged(true), 750.0);
        assert_eq!(span(1000.0, 100.0, 250.0).paged(false), 0.0);
        assert_eq!(at.wheeled(true), 375.0 + 31.25);
        assert_eq!(at.wheeled(false), 375.0 - 31.25);
        assert_eq!(
            span(200.0, 0.0, 250.0).offset_at(10.0, 400.0),
            0.0,
            "nothing to move"
        );
    }

    // The page jump, by hand with bars of 384 ticks and a view of 1536 (four bars): the
    // playhead in view moves nothing; at 1536 (the view's end) the view jumps to start there;
    // at 2000 it starts at the bar holding it, 1920; a playhead that wrapped back to 100 takes
    // the view to 0. With a bar wider than the view (view 100), the view starts at the playhead.
    #[test]
    fn the_view_jumps_a_page_when_the_playhead_leaves_it() {
        assert_eq!(follow(0.0, 1536.0, 1535.0, 384.0), None);
        assert_eq!(follow(0.0, 1536.0, 0.0, 384.0), None);
        assert_eq!(follow(0.0, 1536.0, 1536.0, 384.0), Some(1536.0));
        assert_eq!(follow(0.0, 1536.0, 2000.0, 384.0), Some(1920.0));
        assert_eq!(follow(1920.0, 1536.0, 100.0, 384.0), Some(0.0));
        assert_eq!(follow(0.0, 100.0, 500.0, 384.0), Some(500.0));
        assert_eq!(follow(0.0, 0.0, 500.0, 384.0), None, "no view, no jump");
    }

    // A 600 px trough over content
    // 2688 with a view of 1320 scrolled past the end to 5000 (the bar reaching 6320). The thumb
    // held at its middle and the cursor moved 60 px left at a time, the view taking each offset
    // the bar asks for: the point held stays under the cursor (it ran up to 175 px ahead, the
    // thumb drawn from a reach shrinking as the view came back inside the content). Let go
    // of, the bar shows the view's own span again.
    #[test]
    fn a_held_thumb_stays_under_the_cursor() {
        use canvas::Program as _;
        let width = 600.0;
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(width, BAR_HEIGHT));
        let mut view = span(2688.0, 5000.0, 1320.0);
        let bar = |span| Scrollbar {
            span,
            message: Message::RackBar,
        };
        let mut state = BarState::default();
        let (start, size) = view.thumb(width);
        let held = start + size / 2.0;
        let at = |x| mouse::Cursor::Available(Point::new(x, 5.0));
        let press = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        assert!(bar(view)
            .update(&mut state, &press, bounds, at(held))
            .is_some());
        for step in 1..=6 {
            let x = held - 60.0 * step as f32;
            let moved = Event::Mouse(mouse::Event::CursorMoved {
                position: Point::new(x, 5.0),
            });
            let asked = bar(view)
                .update(&mut state, &moved, bounds, at(x))
                .and_then(|action| action.into_inner().0);
            let Some(Message::RackBar(BarMessage::To { offset, held: true })) = asked else {
                panic!("a held drag: {asked:?}");
            };
            view.offset = offset;
            let (start, size) = bar(view).shown(&state).thumb(width);
            let drawn = start + size / 2.0;
            assert!(
                (drawn - x).abs() < 0.01,
                "step {step}: drawn at {drawn}, cursor at {x}"
            );
        }
        let release = Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left));
        let _ = bar(view).update(&mut state, &release, bounds, at(held));
        assert_eq!(bar(view).shown(&state), view, "let go of");
    }

    // By hand, a view from x 120 to 1000: inside it, its edges included, nothing moves; up to
    // 12 px past either edge, 4 px a tick towards it; 66 px past, half way from 12 to 120,
    // 4 + 36 / 2 = 22 px; from 120 px past, 40 px however far.
    #[test]
    fn a_drag_past_an_edge_moves_the_view_faster_the_further_it_is() {
        let step = |at| edge_step(at, 120.0, 1000.0);
        for inside in [120.0, 500.0, 1000.0] {
            assert_eq!(step(inside), 0.0, "inside, at {inside}");
        }
        assert_eq!(step(1000.5), 4.0);
        assert_eq!(step(1012.0), 4.0);
        assert_eq!(step(119.0), -4.0);
        assert_eq!(step(108.0), -4.0);
        assert_eq!(step(1066.0), 22.0);
        assert_eq!(step(54.0), -22.0);
        assert_eq!(step(1120.0), 40.0);
        assert_eq!(step(3000.0), 40.0);
        assert_eq!(step(0.0), -40.0);
        assert_eq!(step(-800.0), -40.0);
        assert_eq!(step(f32::NAN), 0.0, "no pointer, no step");
    }

    // A wheel event moves across when it moves
    // more across than up or down, as a trackpad's swipe across does with a little up or down
    // in nearly every event; it moved across only with none at all.
    #[test]
    fn a_swipe_mostly_across_counts_as_across() {
        assert_eq!(wheel(-3.0, 0.4), (true, -3.0));
        assert_eq!(wheel(-3.0, 0.0), (true, -3.0));
        assert_eq!(wheel(0.4, -3.0), (false, -3.0));
        assert_eq!(
            wheel(2.0, 2.0),
            (false, 2.0),
            "as much both ways: up or down"
        );
        assert_eq!(wheel(0.0, 0.0), (false, 0.0));
    }
}
