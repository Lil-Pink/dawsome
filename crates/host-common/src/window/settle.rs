//! Whether one of our plugin windows has the size its host keeps for it, as the window manager
//! last told us: the settling both formats' hosts run after a user's resize (VST3's frame, CLAP's
//! editor state). Plain state over the clock, no window system, so it is the same on every
//! platform and its rules are tested by hand.

use std::time::{Duration, Instant};

/// How long our window, left by the window manager at another size than the one kept, waits after
/// the last size it was told of or asked for before it asks for the kept size again.
pub const SETTLE_AFTER: Duration = Duration::from_millis(250);
/// How many times it asks again while the window manager keeps telling one size: forty quiet
/// waits, ten seconds, so a window manager that never gives the size is not asked forever.
pub const SETTLE_TRIES: u32 = 40;

/// Whether our window has the size kept: the size the window manager last told us it has, when
/// it told us or we asked last, and how often we asked again since it told us that size. Why: a
/// window manager may leave a client's resize unapplied while the user is still resizing the
/// window, and tell it nothing after (likely KWin's way, inferred from windows seen in use: an
/// editor that holds its size, JS80P's VST3 by its aspect, was left at that size inside a taller
/// window with black under it; Surge XT's CLAP interface, dragged wider, with black to its
/// right), so the answer is asked again once the window manager has been quiet a while, until it
/// tells us the size kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settle {
    seen: (u32, u32),
    last: Instant,
    tries: u32,
}

impl Settle {
    /// A window opened at `size` at `now`.
    pub fn new(size: (u32, u32), now: Instant) -> Self {
        Self {
            seen: size,
            last: now,
            tries: 0,
        }
    }

    /// The window manager told us at `now` that our window is `size`.
    pub fn told(&mut self, size: (u32, u32), now: Instant) {
        if size != self.seen {
            self.tries = 0;
        }
        self.seen = size;
        self.last = now;
    }

    /// We asked for a size at `now`: the window manager's answer is waited for.
    pub fn asked(&mut self, now: Instant) {
        self.last = now;
    }

    /// The size to ask our window for again at `now`: `kept`, while the window manager last told
    /// us another, has said nothing for [`SETTLE_AFTER`] since it told us or we asked, and fewer
    /// than [`SETTLE_TRIES`] asks were made since it told us that size.
    pub fn due(&mut self, kept: (u32, u32), now: Instant) -> Option<(u32, u32)> {
        let quiet = now.saturating_duration_since(self.last) >= SETTLE_AFTER;
        if self.seen == kept || self.tries >= SETTLE_TRIES || !quiet {
            return None;
        }
        self.tries += 1;
        self.last = now;
        Some(kept)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    // The settling, by hand, on JS80P's case (a window seen in use, 2026-10-27: kept 996x576, an
    // editor that holds a taller window to that size by its aspect). The user drags the window to
    // 996x700, then 996x900, each answered with the kept size (put back at 100 and 200 ms) and
    // each answer lost in the drag; the drag ends with nothing more told. The window manager
    // quiet for 250 ms after the last ask, the kept size is asked again at 450 ms, not before
    // (ticks at 216 and 449 ms ask nothing); once it tells us the kept size (460 ms) nothing more
    // is asked.
    #[test]
    fn a_size_lost_in_the_users_drag_is_asked_again_once_the_window_manager_is_quiet() {
        let kept = (996, 576);
        let start = Instant::now();
        let mut settle = Settle::new(kept, start);
        assert_eq!(settle.due(kept, at(start, 5_000)), None, "opened at it");
        let mut settle = Settle::new(kept, start);
        settle.told((996, 700), at(start, 100));
        settle.asked(at(start, 100));
        settle.told((996, 900), at(start, 200));
        settle.asked(at(start, 200));
        assert_eq!(settle.due(kept, at(start, 216)), None, "16 ms quiet");
        assert_eq!(settle.due(kept, at(start, 449)), None, "249 ms quiet");
        assert_eq!(settle.due(kept, at(start, 450)), Some(kept), "250 ms quiet");
        assert_eq!(settle.due(kept, at(start, 451)), None, "just asked");
        settle.told(kept, at(start, 460));
        assert_eq!(settle.due(kept, at(start, 800)), None, "it has the size");
        assert_eq!(settle.due(kept, at(start, 60_000)), None);
    }

    // A drag held still (or a window manager that never gives the size): asked every 250 ms, 40
    // times (250 ms to 10 s), then no more while it tells the same size; a size it tells anew
    // (the user moved on) starts the count again. The editor's own resize waits as long for the
    // window manager's answer before asking again.
    #[test]
    fn a_size_never_given_is_asked_forty_times_then_left() {
        let kept = (996, 576);
        let start = Instant::now();
        let mut settle = Settle::new(kept, start);
        settle.told((996, 900), start);
        settle.asked(start);
        let asked: Vec<u64> = (1..=60u64)
            .map(|n| n * 250)
            .filter(|&ms| settle.due(kept, at(start, ms)).is_some())
            .collect();
        assert_eq!(asked, (1..=40u64).map(|n| n * 250).collect::<Vec<_>>());
        assert_eq!(SETTLE_TRIES, 40);
        settle.told((996, 900), at(start, 15_100));
        assert_eq!(
            settle.due(kept, at(start, 15_350)),
            None,
            "the same size again"
        );
        settle.told((996, 901), at(start, 15_400));
        assert_eq!(settle.due(kept, at(start, 15_649)), None);
        assert_eq!(
            settle.due(kept, at(start, 15_650)),
            Some(kept),
            "a new size: counted anew"
        );

        let resized = (1000, 600);
        let mut settle = Settle::new(kept, start);
        settle.asked(at(start, 5_000));
        assert_eq!(
            settle.due(resized, at(start, 5_100)),
            None,
            "its answer is waited for"
        );
        assert_eq!(
            settle.due(resized, at(start, 5_250)),
            Some(resized),
            "none came"
        );
        let mut settle = Settle::new(kept, start);
        settle.asked(at(start, 5_000));
        settle.told(resized, at(start, 5_020));
        assert_eq!(settle.due(resized, at(start, 9_000)), None, "it came");
    }
}
