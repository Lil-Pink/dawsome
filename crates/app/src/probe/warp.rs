//! Screenshot probe of a selected audio clip's transient marks and fade handles: drawn by the
//! GPU renderer, which dropped the marks while they were drawn into a clipped frame. Positions
//! come from the playlist's layout constants with the hand-worked figures beside them; colours
//! from [`crate::ui::style`].

use super::tests::{read_ui_shot, rgb8};
use super::{near, TOLERANCE};
use crate::ui::layout::TOOLBAR_HEIGHT;
use crate::ui::playlist_layout::{
    FADE_HANDLE, HANDLE_RING, HEADER_HEIGHT, RULER_HEIGHT, TITLE_HEIGHT,
};
use crate::ui::style;

// p39_quantize with the playlist alone in a 1000 x 400 window, its clip (track 1, ticks
// 10..384) selected, at the default zoom of 64 px a bar: x = 120 + tick / 6, 250 frames a tick,
// so a song frame f is at x = 120 + f / 1500. Of its sound's eight hits the clip plays seven,
// at song frames 6700, 11100, 26999, 33000, 48100, 69100 and 74900 (2500 + each hit's frame):
// x 124.5, 127.4, 138.0, 142.0, 152.1, 166.1 and 169.9; the eighth, at 97500, is past the
// clip's end (96000). Selected, the clip shows its fade handles: with no fades, discs 8 px
// across in a 1 px ring hanging from the body's top just inside its corners, centred at x
// 121.67 + 5 = 126.67 and 184 − 5 = 179, 5 px down. Their rings cover x 121.67..131.67 and
// 174..184 of the body's top rows, so the first two hits' ticks lie under the fade in's handle
// (drawn over them since the handles show on a selected clip). The body's top row (under the
// 10 px title strip), between the handles, holds the other five ticks: five runs of the
// accent, two device pixels each; each handle's middle is the accent.
#[test]
#[ignore = "needs a screenshot of p39_quantize with its clip selected: scripts/ui-check.sh takes it and runs this"]
fn w1_a_selected_clip_marks_its_hits() {
    let shot = read_ui_shot();
    let physical = |logical: f32| (logical * shot.scale).round() as usize;
    let row = physical(TOOLBAR_HEIGHT + HEADER_HEIGHT + RULER_HEIGHT + TITLE_HEIGHT);
    let pixel = |column: usize| {
        let at = (row * shot.width as usize + column) * 4;
        [shot.rgba[at], shot.rgba[at + 1], shot.rgba[at + 2]]
    };
    let body = rgb8(style::palette().audio_clip_body);
    assert!(
        near(pixel(physical(160.0)), body, TOLERANCE),
        "row {row} is the clip's body: {:?} at x 160",
        pixel(physical(160.0))
    );
    let accent = rgb8(style::accent());
    let (left, right) = (120.0 + 10.0 / 6.0, 120.0 + 384.0 / 6.0);
    let ring = FADE_HANDLE / 2.0 + HANDLE_RING;
    let columns = physical(left + 2.0 * ring + 1.0)..physical(right - 2.0 * ring - 1.0);
    let lit: Vec<usize> = columns
        .filter(|&column| near(pixel(column), accent, TOLERANCE))
        .collect();
    let runs =
        lit.windows(2).filter(|pair| pair[1] != pair[0] + 1).count() + usize::from(!lit.is_empty());
    println!("row {row}: accent at {lit:?}, {runs} runs");
    assert_eq!(
        runs, 5,
        "a tick for each hit the clip plays clear of the handles"
    );
    assert_eq!(lit.len(), 5 * 2, "two device pixels a tick");
    let middle = TOOLBAR_HEIGHT + HEADER_HEIGHT + RULER_HEIGHT + TITLE_HEIGHT + ring;
    for x in [left + ring, right - ring] {
        let at = (physical(middle) * shot.width as usize + physical(x)) * 4;
        let handle = [shot.rgba[at], shot.rgba[at + 1], shot.rgba[at + 2]];
        assert!(
            near(handle, accent, TOLERANCE),
            "the fade handle at ({x}, {middle}) is the accent: {handle:?}"
        );
    }
}
