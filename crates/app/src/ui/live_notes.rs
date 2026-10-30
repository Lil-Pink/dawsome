//! The notes being played live (the typing keyboard, a MIDI port, the instrument window's keys,
//! the roll's own preview), as the piano roll shows them on the channel they play: its key lit
//! in the key column, and a translucent bar growing from the tick the note started at while the
//! song runs, or a short mark in a thin strip at the timeline's left edge while it does not.
//! Read from the same events the player posts for recording ([`EngineEvent::LiveNoteStarted`]
//! and [`EngineEvent::LiveNoteEnded`]), so what the roll shows and what a take records cannot
//! disagree. A note on a channel that plays no sound (an external input) shows all the same.

use std::collections::BTreeMap;

use dawsome_engine::project::ChannelId;
use dawsome_engine::realtime::EngineEvent;
use dawsome_engine::time::Ticks;
use iced::{Point, Rectangle, Size};

use crate::ui::piano_layout::{View, KEY_COLUMN_WIDTH, ROW_HEIGHT};

/// The thin strip at the timeline's left edge where a note held while the song is stopped is
/// marked.
pub const LIVE_STRIP_WIDTH: f32 = 6.0;
/// The least width a live bar is drawn: a note just started still shows.
pub const LEAST_BAR: f32 = 2.0;

/// A note held live: its velocity, and the song tick it started at (none when the song's
/// timeline was not running: stopped, or the count-in).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Held {
    pub velocity: f32,
    pub tick: Option<Ticks>,
}

/// The notes held live now, by channel and key.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LiveNotes {
    held: BTreeMap<(ChannelId, u8), Held>,
}

impl LiveNotes {
    /// Takes in what the player posted; returns whether a note started or ended.
    pub fn hear(&mut self, events: &[EngineEvent]) -> bool {
        let mut changed = false;
        for event in events {
            match *event {
                EngineEvent::LiveNoteStarted {
                    channel,
                    key,
                    velocity,
                    tick,
                    ..
                } => {
                    self.held.insert((channel, key), Held { velocity, tick });
                    changed = true;
                }
                EngineEvent::LiveNoteEnded { channel, key, .. } => {
                    changed |= self.held.remove(&(channel, key)).is_some();
                }
                _ => {}
            }
        }
        changed
    }

    /// No note is held.
    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    /// Forgets every note, as a new document's player starts with none held.
    pub fn clear(&mut self) {
        self.held.clear();
    }

    /// `channel`'s notes held, by key, lowest first.
    pub fn of(&self, channel: ChannelId) -> Vec<(u8, Held)> {
        self.held
            .range((channel, 0)..=(channel, u8::MAX))
            .map(|(&(_, key), &held)| (key, held))
            .collect()
    }
}

/// The key column's lit key `key`, grid coordinates: the key's face, less its bottom pixel.
pub fn lit_key(view: &View, key: u8) -> Rectangle {
    Rectangle::new(
        Point::new(0.0, view.row_top(key)),
        Size::new(KEY_COLUMN_WIDTH, ROW_HEIGHT - 1.0),
    )
}

/// Where note `key`, held from song tick `started`, is drawn in the grid of a pattern
/// `pattern_end` ticks long showing `view`. While the song plays (`playhead`, the song's tick)
/// and the note started on the timeline, a bar from where it started in the pattern to where
/// the playhead is now, in two pieces when the pattern wrapped between (to the end, then from
/// the start), one over the whole pattern once held that long; otherwise a mark in the live
/// strip. Nothing left of the timeline is drawn.
pub fn bars(
    view: &View,
    pattern_end: Ticks,
    playhead: Option<Ticks>,
    key: u8,
    started: Option<Ticks>,
) -> Vec<Rectangle> {
    let top = view.row_top(key);
    let height = ROW_HEIGHT - 1.0;
    let (Some(now), Some(started)) = (playhead, started) else {
        return vec![Rectangle::new(
            Point::new(KEY_COLUMN_WIDTH, top),
            Size::new(LIVE_STRIP_WIDTH, height),
        )];
    };
    let length = pattern_end.0.max(1);
    // Held a whole pattern or more: it has sounded over all of it.
    if now.0 - started.0 >= length {
        let left = view.x_of(Ticks(0)).max(KEY_COLUMN_WIDTH);
        let right = view.x_of(Ticks(length)).max(left + LEAST_BAR);
        return vec![Rectangle::new(
            Point::new(left, top),
            Size::new(right - left, height),
        )];
    }
    let (from, to) = (started.0.rem_euclid(length), now.0.rem_euclid(length));
    let spans = if to >= from {
        vec![(from, to)]
    } else {
        vec![(from, length), (0, to)]
    };
    spans
        .into_iter()
        .filter_map(|(from, to)| {
            let left = view.x_of(Ticks(from)).max(KEY_COLUMN_WIDTH);
            let right = view.x_of(Ticks(to)).max(left + LEAST_BAR);
            (right > KEY_COLUMN_WIDTH)
                .then(|| Rectangle::new(Point::new(left, top), Size::new(right - left, height)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::piano_layout::DEFAULT_ZOOM;

    /// p3_roll's view: top key 69, 16 px a step of 24 ticks (1.5 ticks a pixel), no scroll.
    fn view() -> View {
        View {
            top: 69,
            scroll: Ticks(0),
            zoom: DEFAULT_ZOOM,
            step: Ticks(24),
            rows: crate::ui::piano_layout::Rows::ALL,
        }
    }

    fn started(channel: u32, key: u8, tick: Option<i64>) -> EngineEvent {
        EngineEvent::LiveNoteStarted {
            channel: ChannelId(channel),
            key,
            velocity: 0.5,
            tick: tick.map(Ticks),
            frame: 0,
        }
    }

    fn ended(channel: u32, key: u8) -> EngineEvent {
        EngineEvent::LiveNoteEnded {
            channel: ChannelId(channel),
            key,
            tick: None,
            frame: 0,
        }
    }

    // A note started, another on another channel, the first ended: the set follows; an end
    // of a note not held changes nothing.
    #[test]
    fn the_live_set_follows_the_players_events() {
        let mut live = LiveNotes::default();
        assert!(live.hear(&[started(1, 60, None), started(2, 64, Some(30))]));
        assert_eq!(
            live.of(ChannelId(1)),
            [(
                60,
                Held {
                    velocity: 0.5,
                    tick: None
                }
            )]
        );
        assert_eq!(live.of(ChannelId(2))[0].1.tick, Some(Ticks(30)));
        assert!(live.hear(&[ended(1, 60)]));
        assert!(live.of(ChannelId(1)).is_empty());
        assert!(!live.hear(&[ended(1, 61)]), "nothing held there");
        live.clear();
        assert!(live.is_empty());
    }

    // By hand at 1.5 ticks a pixel, key 60's row y 108..119: held from tick 96 with the
    // playhead at 180, the bar spans x 112..168; from 360 with the playhead wrapped to 30 in a
    // 384-tick pattern, 360..384 (x 288..304) and 0..30 (x 48..68); just started (96, 96), 2 px
    // wide; stopped, or started in the count-in, a 6 px mark at x 48. The lit key is the key
    // column's 48 px.
    // A note held a whole pattern or more (100 → 500 and 100 → 900 in
    // a 384-tick pattern) is one bar over the whole pattern, x 48..304, not the short piece
    // since it last came round; held one tick less than the pattern (100 → 483) it is still
    // two pieces, 100..384 and 0..99.
    #[test]
    fn a_note_held_a_whole_pattern_fills_it() {
        let view = view();
        let end = Ticks(384);
        let rect =
            |x: f32, width: f32| Rectangle::new(Point::new(x, 108.0), Size::new(width, 11.0));
        for now in [500, 900] {
            assert_eq!(
                bars(&view, end, Some(Ticks(now)), 60, Some(Ticks(100))),
                [rect(48.0, 256.0)],
                "held to {now}"
            );
        }
        let short = bars(&view, end, Some(Ticks(483)), 60, Some(Ticks(100)));
        assert_eq!(short.len(), 2, "{short:?}");
        assert_eq!(short[1], rect(48.0, 66.0));
    }

    #[test]
    fn live_bars_grow_from_their_tick_or_mark_the_strip() {
        let view = view();
        let end = Ticks(384);
        let rect =
            |x: f32, width: f32| Rectangle::new(Point::new(x, 108.0), Size::new(width, 11.0));
        assert_eq!(
            bars(&view, end, Some(Ticks(180)), 60, Some(Ticks(96))),
            [rect(112.0, 56.0)]
        );
        assert_eq!(
            bars(&view, end, Some(Ticks(384 + 30)), 60, Some(Ticks(360))),
            [rect(288.0, 16.0), rect(48.0, 20.0)]
        );
        assert_eq!(
            bars(&view, end, Some(Ticks(96)), 60, Some(Ticks(96))),
            [rect(112.0, 2.0)]
        );
        assert_eq!(
            bars(&view, end, None, 60, Some(Ticks(96))),
            [rect(48.0, 6.0)]
        );
        assert_eq!(
            bars(&view, end, Some(Ticks(10)), 60, None),
            [rect(48.0, 6.0)]
        );
        assert_eq!(
            lit_key(&view, 60),
            Rectangle::new(Point::new(0.0, 108.0), Size::new(48.0, 11.0))
        );
    }
}
