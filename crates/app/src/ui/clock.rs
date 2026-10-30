//! The clock the canvases read to spot a double-press: the wall clock, except under a test
//! that holds it ([`hold`]). Why: a test drives two clicks through the real widgets, and on a
//! loaded machine (the full suite beside a build) the second came more than the double-press
//! time after the first, so the roll's double-click test drew a note instead of deleting one
//! (gate, 2026-10-25). Held, the clicks a test sends are as close as it says they are.

use std::time::Instant;

#[cfg(test)]
thread_local! {
    static HELD: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
}

/// Now, by the wall clock; under a test holding the clock, the instant it holds.
pub fn now() -> Instant {
    #[cfg(test)]
    if let Some(held) = HELD.with(std::cell::Cell::get) {
        return held;
    }
    Instant::now()
}

/// Holds the clock at `at` for this thread (`None` lets it run again).
#[cfg(test)]
pub fn hold(at: Option<Instant>) {
    HELD.with(|held| held.set(at));
}
