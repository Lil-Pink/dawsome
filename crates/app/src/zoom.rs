//! The app's zoom: one percentage the user sets (Settings → Display → Zoom, Ctrl+= or Ctrl+Plus,
//! Ctrl+- and Ctrl+0), multiplied on top of the system's own display scale in every window, the
//! main one and the pop-out windows alike (iced's scale factor, `main.rs`). The layout's numbers
//! stay in logical pixels and every test runs at 100 %: the toolkit scales the text, the
//! spacing, the borders, the canvases and the screenshots. Why one setting on top of the
//! system's: that is what other audio apps offer (Ableton's Zoom Display, Surge XT's zoom), and
//! the system's scale alone left a 4K screen and a laptop at 125 % unlike each other with no way
//! to match them.
//!
//! A zoom is one of the stops: every 10 % from 50 to 200, and the quarters between them (75, 125
//! and 175). Why the quarters: 125 % and 175 % are display scales Windows offers, and a zoom to
//! set beside such a desktop must reach them, which 10 % steps alone never do.

/// A step the zoom's keys and its stepper in Settings take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// The next stop up (Ctrl+=, Ctrl+Plus, the stepper's +).
    Larger,
    /// The next stop down (Ctrl+-, the stepper's −).
    Smaller,
    /// Back to 100 % (Ctrl+0, a double-click on the stepper's number).
    Reset,
}

/// A zoom, as a whole percentage on one of the stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Zoom(u16);

impl Default for Zoom {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl Zoom {
    /// No zoom: the system's scale alone.
    pub const DEFAULT: Zoom = Zoom(100);
    /// The least and the most.
    pub const LEAST: Zoom = Zoom(50);
    pub const MOST: Zoom = Zoom(200);

    /// Whether `percent` is a stop.
    const fn is_stop(percent: u16) -> bool {
        percent >= Self::LEAST.0
            && percent <= Self::MOST.0
            && (percent.is_multiple_of(10) || percent.is_multiple_of(25))
    }

    /// The zoom of `percent` when it is a stop; none otherwise (a `--zoom` of 123 is refused).
    pub fn exact(percent: u16) -> Option<Zoom> {
        Self::is_stop(percent).then_some(Zoom(percent))
    }

    /// The zoom a kept percentage reads as: held to 50..=200, then the nearest stop, the larger
    /// of two as near (115 reads as 120). A file edited by hand, or a later build's finer stop,
    /// so still opens near what it asked for.
    pub fn nearest(percent: u16) -> Zoom {
        let held = percent.clamp(Self::LEAST.0, Self::MOST.0);
        let below = (Self::LEAST.0..=held)
            .rev()
            .find(|&stop| Self::is_stop(stop));
        let above = (held..=Self::MOST.0).find(|&stop| Self::is_stop(stop));
        match (below, above) {
            (Some(below), Some(above)) if held - below < above - held => Zoom(below),
            (_, Some(above)) => Zoom(above),
            (Some(below), None) => Zoom(below),
            (None, None) => Self::DEFAULT,
        }
    }

    /// The next stop up, or the most.
    pub fn larger(self) -> Zoom {
        (self.0 + 1..=Self::MOST.0)
            .find(|&stop| Self::is_stop(stop))
            .map_or(Self::MOST, Zoom)
    }

    /// The next stop down, or the least.
    pub fn smaller(self) -> Zoom {
        (Self::LEAST.0..self.0)
            .rev()
            .find(|&stop| Self::is_stop(stop))
            .map_or(Self::LEAST, Zoom)
    }

    /// The zoom `step` leads to.
    pub fn stepped(self, step: Step) -> Zoom {
        match step {
            Step::Larger => self.larger(),
            Step::Smaller => self.smaller(),
            Step::Reset => Self::DEFAULT,
        }
    }

    /// The whole percentage: 125.
    pub fn percent(self) -> u16 {
        self.0
    }

    /// What it multiplies the system's scale by: 1.25.
    pub fn factor(self) -> f32 {
        f32::from(self.0) / 100.0
    }
}

/// As the stepper and the notice say it: "125 %".
impl std::fmt::Display for Zoom {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{} %", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Known answers from the stops' rule (every 10 % from 50 to 200 and the quarters, 75, 125,
    // 175), worked by hand: nineteen stops in all.
    const STOPS: [u16; 19] = [
        50, 60, 70, 75, 80, 90, 100, 110, 120, 125, 130, 140, 150, 160, 170, 175, 180, 190, 200,
    ];

    #[test]
    fn the_stops_are_every_ten_and_the_quarters_from_50_to_200() {
        let found: Vec<u16> = (0..=u16::MAX)
            .filter_map(Zoom::exact)
            .map(Zoom::percent)
            .collect();
        assert_eq!(found, STOPS);
        assert_eq!(Zoom::default(), Zoom(100));
        assert_eq!(Zoom(125).to_string(), "125 %");
        assert_eq!(Zoom::exact(125).map(Zoom::factor), Some(1.25));
        assert_eq!(Zoom::exact(50).map(Zoom::factor), Some(0.5));
        assert_eq!(Zoom::exact(200).map(Zoom::factor), Some(2.0));
        for refused in [0, 45, 105, 115, 123, 205, 250, 1000] {
            assert_eq!(Zoom::exact(refused), None, "{refused}");
        }
    }

    // A step up or down goes to the next stop, and holds at the ends: from 100 up 110, 120, 125,
    // 130; from 80 down 75, 70; at 200 up stays 200, at 50 down stays 50.
    #[test]
    fn a_step_goes_to_the_next_stop_and_holds_at_the_ends() {
        let ups = [
            (100, 110),
            (110, 120),
            (120, 125),
            (125, 130),
            (170, 175),
            (175, 180),
        ];
        for (from, to) in ups {
            assert_eq!(Zoom(from).larger(), Zoom(to), "{from} up");
            assert_eq!(Zoom(to).smaller(), Zoom(from), "{to} down");
        }
        assert_eq!(Zoom(130).stepped(Step::Reset), Zoom(100));
        assert_eq!(Zoom(130).stepped(Step::Larger), Zoom(140));
        assert_eq!(Zoom(130).stepped(Step::Smaller), Zoom(125));
        assert_eq!(Zoom(80).smaller(), Zoom(75));
        assert_eq!(Zoom(75).smaller(), Zoom(70));
        assert_eq!(Zoom(200).larger(), Zoom(200));
        assert_eq!(Zoom(50).smaller(), Zoom(50));
        // Every stop up and back down again, the ends held.
        for pair in STOPS.windows(2) {
            assert_eq!(Zoom(pair[0]).larger(), Zoom(pair[1]));
            assert_eq!(Zoom(pair[1]).smaller(), Zoom(pair[0]));
        }
    }

    // A kept percentage reads as the nearest stop after it is held to 50..=200, the larger of two
    // as near: 0 and 35 read 50, 1000 reads 200; 122 reads 120 (2 from it, 3 from 125), 123 and
    // 127 read 125, 128 reads 130; 115 is as near 110 as 120 and reads 120; 72 reads 70, 73 75.
    #[test]
    fn a_kept_percentage_reads_as_the_nearest_stop() {
        let table = [
            (0, 50),
            (35, 50),
            (50, 50),
            (72, 70),
            (73, 75),
            (100, 100),
            (115, 120),
            (122, 120),
            (123, 125),
            (127, 125),
            (128, 130),
            (178, 180),
            (199, 200),
            (201, 200),
            (1000, 200),
            (u16::MAX, 200),
        ];
        for (kept, read) in table {
            assert_eq!(Zoom::nearest(kept), Zoom(read), "{kept}");
        }
        for stop in STOPS {
            assert_eq!(Zoom::nearest(stop), Zoom(stop));
        }
    }
}
