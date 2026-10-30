//! Our window's size around a plugin's embedded interface: the size kept (what our window and the
//! interface were last given), how a user's resize and a plugin's own request change it, and the
//! settling that asks our window again for the size kept while the window manager has not given
//! it (host-common's [`Settle`], which VST3's frame runs too). Plain state: the host does what an
//! answer says, so the rules are tested without a window (known answers `gen_p29_expected.py`,
//! its "clap" key).

use std::time::Instant;

use dawsome_host_common::window::{clamp_side, Settle};

/// A size held to what a window takes: 1 to 32767 pixels a side.
fn held((width, height): (u32, u32)) -> (u32, u32) {
    (u32::from(clamp_side(width)), u32::from(clamp_side(height)))
}

/// What a user's resize of our window leads to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Answer {
    /// Nothing: the size kept (our own resize's echo), or an interface that keeps its size.
    Nothing,
    /// Our window back to this size, the one kept; the interface has it already.
    PutBack((u32, u32)),
    /// The interface takes this size (`set_size`), kept from now; our window too when `window`
    /// (the window manager gave it another).
    Take { size: (u32, u32), window: bool },
}

/// The size of one open interface's window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Sizing {
    kept: (u32, u32),
    resizable: bool,
    settle: Settle,
}

impl Sizing {
    /// Our window opened at `size` at `now`, around an interface that says it can be resized
    /// (`resizable`) or not.
    pub(crate) fn new(size: (u32, u32), resizable: bool, now: Instant) -> Self {
        let size = held(size);
        Self {
            kept: size,
            resizable,
            settle: Settle::new(size, now),
        }
    }

    /// The size kept.
    #[cfg(test)]
    pub(crate) fn kept(&self) -> (u32, u32) {
        self.kept
    }

    pub(crate) fn resizable(&self) -> bool {
        self.resizable
    }

    /// The plugin asked at `now` for `size` (`request_resize`): the size our window is asked for,
    /// kept from now; `None` when it is the size kept already.
    pub(crate) fn plugin_asked(&mut self, size: (u32, u32), now: Instant) -> Option<(u32, u32)> {
        let size = held(size);
        if size == self.kept {
            return None;
        }
        self.kept = size;
        self.settle.asked(now);
        Some(size)
    }

    /// The window manager told us at `now` that our window is `size` (the last of a tick's, as
    /// the user resizes it); `adjust` is the interface's `adjust_size`, `None` when it will not
    /// say (the size asked is then taken). The size kept, or an interface that keeps its size,
    /// is nothing; a size held to the one kept puts our window back alone; another is the
    /// interface's, and our window's when the window manager gave another.
    pub(crate) fn user_resized(
        &mut self,
        size: (u32, u32),
        now: Instant,
        adjust: impl FnOnce((u32, u32)) -> Option<(u32, u32)>,
    ) -> Answer {
        self.settle.told(size, now);
        if !self.resizable || size == self.kept {
            return Answer::Nothing;
        }
        let answer = adjust(size).map_or(size, held);
        if answer == self.kept {
            self.settle.asked(now);
            return Answer::PutBack(answer);
        }
        self.kept = answer;
        let window = answer != size;
        if window {
            self.settle.asked(now);
        }
        Answer::Take {
            size: answer,
            window,
        }
    }

    /// The size to ask our window for again at `now`: the one kept, while the window manager
    /// last told us another and has been quiet a while ([`Settle::due`]); an interface that keeps
    /// its size leaves its window to its hints.
    pub(crate) fn due(&mut self, now: Instant) -> Option<(u32, u32)> {
        if !self.resizable {
            return None;
        }
        self.settle.due(self.kept, now)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::Value;

    use super::*;

    fn p29() -> Value {
        serde_json::from_str(include_str!("../../../tests/fixtures/p29_expected.json"))
            .expect("p29_expected.json parses")
    }

    fn size(value: &Value) -> (u32, u32) {
        (
            value[0].as_u64().expect("a width") as u32,
            value[1].as_u64().expect("a height") as u32,
        )
    }

    fn at(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    /// The test interface's `adjust_size` as gen_p29 lists its answers.
    fn adjust(p29: &Value) -> impl Fn((u32, u32)) -> Option<(u32, u32)> + '_ {
        move |asked| {
            p29["clap"]["adjust"]
                .as_array()
                .expect("answers")
                .iter()
                .find(|pair| size(&pair[0]) == asked)
                .map(|pair| size(&pair[1]))
        }
    }

    /// The lines the test interface logs for `answer` to a resize to `asked`, `held` its
    /// `adjust_size` answer.
    fn logged(asked: (u32, u32), answer: Answer, held: Option<(u32, u32)>) -> Vec<String> {
        let text = |(w, h): (u32, u32)| format!("{w}x{h}");
        match (answer, held) {
            (Answer::Nothing, _) => Vec::new(),
            (Answer::PutBack(_), Some(held)) => {
                vec![format!("adjust_size {} -> {}", text(asked), text(held))]
            }
            (Answer::Take { size, .. }, Some(held)) => vec![
                format!("adjust_size {} -> {}", text(asked), text(held)),
                format!("set_size {}", text(size)),
            ],
            (answer, None) => panic!("{answer:?} without adjust_size"),
        }
    }

    // gen_p29's user resizes, each on the window the one before left (the resizable interface's,
    // then the fixed one's): the answer's calls are the log's, our window ends at its size
    // (Surge XT's case seen in use first: 600x300, wider than the interface's aspect, puts our
    // window back to 400x300 and tells the interface nothing), and our own resize's echo is
    // nothing.
    #[test]
    fn a_users_resize_is_held_by_the_interface_and_our_window_follows() {
        let p29 = p29();
        let adjust = adjust(&p29);
        let start = Instant::now();
        let first = size(&p29["clap"]["size"]);
        for list in ["user_resizes", "real_resizes"] {
            let mut sizings = [
                Sizing::new(first, true, start),
                Sizing::new(first, false, start),
            ];
            let mut windows = [first, first];
            for case in p29["clap"][list].as_array().expect("cases") {
                let fixed = case["fixed"].as_bool().expect("fixed");
                let sizing = &mut sizings[usize::from(fixed)];
                let asked = size(&case["size"]);
                let answer = sizing.user_resized(asked, start, &adjust);
                let window = &mut windows[usize::from(fixed)];
                match answer {
                    Answer::Nothing => {}
                    Answer::PutBack(size) => *window = size,
                    Answer::Take { size, window: true } => *window = size,
                    Answer::Take {
                        size,
                        window: false,
                    } => {
                        assert_eq!(size, asked, "taken as the window manager gave it");
                        *window = size;
                    }
                }
                let log: Vec<String> = case["log"]
                    .as_array()
                    .expect("a log")
                    .iter()
                    .map(|line| line.as_str().expect("a line").to_owned())
                    .collect();
                assert_eq!(logged(asked, answer, adjust(asked)), log, "{list} {case}");
                assert_eq!(*window, size(&case["window"]), "{list} {case}");
                assert_eq!(sizing.kept(), *window, "{list} {case}");
                assert_eq!(
                    sizing.user_resized(*window, start, |_| panic!("asked again")),
                    Answer::Nothing,
                    "the echo of {case}"
                );
            }
        }
    }

    // A plugin's own request: our window takes it unless it is the size kept, held to what a
    // window takes; a fixed interface's too (its hints follow, the host's part).
    #[test]
    fn a_plugins_request_resizes_our_window_unless_it_is_kept() {
        let p29 = p29();
        let start = Instant::now();
        let mut sizing = Sizing::new(size(&p29["clap"]["size"]), true, start);
        let mut window = sizing.kept();
        for step in p29["clap"]["plugin_requests"].as_array().expect("steps") {
            let asked = size(&step["asked"]);
            if let Some(size) = sizing.plugin_asked(asked, start) {
                window = size;
            }
            assert_eq!(window, size(&step["window"]), "{step}");
        }
        assert_eq!(sizing.plugin_asked(window, start), None, "kept");
        // Never reached from a plugin (`request_resize` refuses a zero side); pinned so the
        // rule stays harmless if it ever is.
        assert_eq!(sizing.plugin_asked((0, 0), start), Some((1, 1)));
        let fixed = &p29["clap"]["fixed"];
        let mut sizing = Sizing::new(size(&p29["clap"]["size"]), false, start);
        assert_eq!(
            sizing.plugin_asked(size(&fixed["asked"]), start),
            Some(size(&fixed["window"]))
        );
        assert!(!sizing.resizable());
    }

    // The settling on Surge XT's case seen in use (2026-10-28: kept 400x300, an interface that
    // holds its aspect). The user drags our window to 600x300, then 700x300, each answered by
    // putting it back (at 100 and 200 ms) and each answer lost in the drag; the drag ends with
    // nothing more told. 250 ms quiet after the last ask, 400x300 is asked again at 450 ms, not
    // before (ticks at 216 and 449 ms ask nothing); once the window manager tells us 400x300
    // (460 ms) nothing more is asked. A fixed interface's window is never asked again: its
    // hints hold it.
    #[test]
    fn a_size_lost_in_the_users_drag_is_asked_again_once_the_window_manager_is_quiet() {
        let start = Instant::now();
        let kept = (400, 300);
        let hold = |_| Some(kept);
        let mut sizing = Sizing::new(kept, true, start);
        assert_eq!(sizing.due(at(start, 5_000)), None, "opened at it");
        assert_eq!(
            sizing.user_resized((600, 300), at(start, 100), hold),
            Answer::PutBack(kept)
        );
        assert_eq!(
            sizing.user_resized((700, 300), at(start, 200), hold),
            Answer::PutBack(kept)
        );
        assert_eq!(sizing.due(at(start, 216)), None, "16 ms quiet");
        assert_eq!(sizing.due(at(start, 449)), None, "249 ms quiet");
        assert_eq!(sizing.due(at(start, 450)), Some(kept), "250 ms quiet");
        assert_eq!(sizing.due(at(start, 451)), None, "just asked");
        assert_eq!(
            sizing.user_resized(kept, at(start, 460), |_| panic!("asked")),
            Answer::Nothing
        );
        assert_eq!(sizing.due(at(start, 800)), None, "it has the size");

        // A size taken whose window the window manager did not resize: asked again too.
        let mut sizing = Sizing::new(kept, true, start);
        assert_eq!(
            sizing.user_resized((900, 900), at(start, 100), |_| Some((900, 675))),
            Answer::Take {
                size: (900, 675),
                window: true
            }
        );
        assert_eq!(sizing.due(at(start, 349)), None);
        assert_eq!(sizing.due(at(start, 350)), Some((900, 675)));

        let mut fixed = Sizing::new(kept, false, start);
        assert_eq!(
            fixed.user_resized((600, 300), at(start, 100), |_| panic!("asked")),
            Answer::Nothing
        );
        assert_eq!(fixed.due(at(start, 10_000)), None, "left to its hints");
    }
}
