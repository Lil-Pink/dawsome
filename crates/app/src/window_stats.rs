//! What the window has done, for `--stats`: how many messages the app took (each one rebuilds
//! the view and asks for a frame), how many frames it drew, and which renderer drew them. Two
//! counters; counting a frame makes no message. Frames are counted only once [`count_frames`]
//! is called, because the listener that counts them is woken by every window event.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use iced::{event, window, Event, Renderer, Subscription};

static UPDATES: AtomicU64 = AtomicU64::new(0);
static FRAMES: AtomicU64 = AtomicU64::new(0);
static COUNTING_FRAMES: AtomicBool = AtomicBool::new(false);
/// The renderer that drew the first frame. Why worth printing: the toolkit falls back from the
/// GPU's renderer to a software one without a word when the GPU's cannot start, and the two
/// differ in cost and in how they clip.
static RENDERER: OnceLock<&'static str> = OnceLock::new();

/// Records which renderer draws the window: "wgpu" on the GPU, "tiny-skia" in software. A
/// canvas every window shows calls it as it draws.
pub fn drawn_with(renderer: &Renderer) {
    RENDERER.get_or_init(|| match renderer {
        Renderer::Primary(_) => "wgpu",
        Renderer::Secondary(_) => "tiny-skia",
    });
}

/// The renderer that drew the window, once it drew a frame.
pub fn renderer() -> Option<&'static str> {
    RENDERER.get().copied()
}

/// From now on, [`frames`] counts the frames drawn.
pub fn count_frames() {
    COUNTING_FRAMES.store(true, Ordering::Relaxed);
}

/// The subscription that counts frames, once [`count_frames`] was called.
pub fn frames<Message: 'static + Send>() -> Subscription<Message> {
    if COUNTING_FRAMES.load(Ordering::Relaxed) {
        event::listen_raw(count_frame)
    } else {
        Subscription::none()
    }
}

/// Counts one message taken by the app.
pub fn updated() {
    UPDATES.fetch_add(1, Ordering::Relaxed);
}

/// Counts a drawn frame; it never makes a message.
fn count_frame<Message>(
    event: Event,
    _status: event::Status,
    _window: window::Id,
) -> Option<Message> {
    if let Event::Window(window::Event::RedrawRequested(_)) = event {
        FRAMES.fetch_add(1, Ordering::Relaxed);
    }
    None
}

/// The counts so far, over a window open for `open`.
pub struct Summary {
    pub updates: u64,
    pub frames: u64,
    pub open: Duration,
}

impl Summary {
    pub fn now(open: Duration) -> Self {
        Self {
            updates: UPDATES.load(Ordering::Relaxed),
            frames: FRAMES.load(Ordering::Relaxed),
            open,
        }
    }
}

impl fmt::Display for Summary {
    /// "window: 750 updates (30.0/s), 752 frames drawn (30.1/s) in 25.0 s".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let seconds = self.open.as_secs_f64().max(f64::EPSILON);
        write!(
            f,
            "window: {} updates ({:.1}/s), {} frames drawn ({:.1}/s) in {:.1} s",
            self.updates,
            self.updates as f64 / seconds,
            self.frames,
            self.frames as f64 / seconds,
            self.open.as_secs_f64(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_summary_gives_counts_and_rates() {
        let summary = Summary {
            updates: 750,
            frames: 753,
            open: Duration::from_secs(25),
        };
        assert_eq!(
            summary.to_string(),
            "window: 750 updates (30.0/s), 753 frames drawn (30.1/s) in 25.0 s"
        );
    }
}
