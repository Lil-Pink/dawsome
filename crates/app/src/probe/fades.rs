//! Screenshot probes of an audio clip's fades on the playlist: a selected clip shows its fade
//! handles, discs in the accent, and every faded clip shades the part of its body above each
//! fade's line; a clip nobody points at or selected shows no handle. Positions come from the
//! playlist's layout constants with the hand-worked figures beside them; colours from
//! [`crate::ui::style`].

use super::tests::{read_ui_shot, rgb8};
use super::{near, Screenshot, TOLERANCE};
use crate::ui::layout::TOOLBAR_HEIGHT;
use crate::ui::playlist_layout::{
    FADE_HANDLE, HANDLE_RING, HEADER_HEIGHT, RULER_HEIGHT, SELECTED_OUTLINE, TITLE_HEIGHT,
};
use crate::ui::style;

// ui-check's p38_faded_wide.json (p38_both's clip moved to ticks 192..960 at full gain) with the
// playlist alone in a 1000 x 400 window at 64 px a bar: x = 120 + tick / 6, so the clip spans
// x 152..280, 192000 frames. Its fade in, 48000 frames, reaches 128 · 48000 / 192000 = 32 px, to
// x 184; its fade out, 36000, reaches 24 px, from x 256. Eight tracks fit 32 px lanes: track 1's
// from y 116 (under the 64 px toolbar, the header and the ruler), its body y 126..148, its
// waveform's centre line y 137. The handles hang 5 px under the body's top, centred at
// (184, 131) and (256, 131).
const CLIP: (f32, f32) = (152.0, 280.0);
const FADE_ENDS: (f32, f32) = (184.0, 256.0);
const WEDGE_PIXELS: [(f32, f32); 2] = [
    // x 160 on the fade in's line is y 148 − 22 · 8 / 32 = 142.5, under y 129; the flat sound's
    // waveform there (its 0.5 at a quarter of the fade) is 1.4 px either side of y 137.
    (160.0, 129.0),
    // x 272 on the fade out's is y 126 + 22 · 16 / 24 = 140.67, under y 129; past the sound's
    // end (80 ticks, x 165.3) the body holds only its centre line.
    (272.0, 129.0),
];
/// Mid-clip, past both fades and the sound: the body's own colour.
const PLAIN_PIXEL: (f32, f32) = (216.0, 129.0);
/// How much darker than the body each channel of the shade is at least: black at 45 % takes
/// 11 to 23 off the body's channels however the renderer blends (gamma or linear).
const SHADE_LEAST: u8 = 8;

fn lane_top() -> f32 {
    TOOLBAR_HEIGHT + HEADER_HEIGHT + RULER_HEIGHT
}

fn pixel_at(shot: &Screenshot, x: f32, y: f32) -> [u8; 3] {
    let physical = |logical: f32| (logical * shot.scale).round() as usize;
    let at = (physical(y) * shot.width as usize + physical(x)) * 4;
    [shot.rgba[at], shot.rgba[at + 1], shot.rgba[at + 2]]
}

/// The shade over each fade: the wedge pixels darker than the body by [`SHADE_LEAST`] or more
/// in every channel, the plain one the body itself.
fn assert_shaded(shot: &Screenshot) {
    let body = rgb8(style::palette().audio_clip_body);
    let plain = pixel_at(shot, PLAIN_PIXEL.0, PLAIN_PIXEL.1);
    assert!(
        near(plain, body, TOLERANCE),
        "the body at {PLAIN_PIXEL:?}: {plain:?}"
    );
    for (x, y) in WEDGE_PIXELS {
        let shaded = pixel_at(shot, x, y);
        assert!(
            shaded
                .iter()
                .zip(body)
                .all(|(&shade, body)| shade.saturating_add(SHADE_LEAST) <= body),
            "({x}, {y}) is shaded: {shaded:?} against the body's {body:?}"
        );
    }
}

#[test]
#[ignore = "needs a screenshot of p38_faded_wide with its clip selected: scripts/ui-check.sh takes it and runs this"]
fn h1_a_selected_faded_clip_shows_its_handles_and_shades_its_fades() {
    let shot = read_ui_shot();
    let centre_y = lane_top() + TITLE_HEIGHT + FADE_HANDLE / 2.0 + HANDLE_RING;
    assert_eq!(centre_y, 131.0);
    let accent = rgb8(style::accent());
    for x in [FADE_ENDS.0, FADE_ENDS.1] {
        // The disc's middle, 8 px across: its centre and a pixel either side.
        for dx in [-1.0, 0.0, 1.0] {
            let pixel = pixel_at(&shot, x + dx, centre_y);
            assert!(
                near(pixel, accent, TOLERANCE),
                "the handle at ({}, {centre_y}) is the accent: {pixel:?}",
                x + dx
            );
        }
    }
    assert_shaded(&shot);
}

#[test]
#[ignore = "needs a screenshot of p38_faded_wide with nothing selected: scripts/ui-check.sh takes it and runs this"]
fn h2_a_faded_clip_at_rest_shows_no_handles_and_keeps_its_shade() {
    let shot = read_ui_shot();
    let accent = rgb8(style::accent());
    let physical = |logical: f32| (logical * shot.scale).round() as usize;
    // The whole clip, title strip and body, but the selection outline's room at its sides.
    let (left, right) = (CLIP.0 + SELECTED_OUTLINE, CLIP.1 - SELECTED_OUTLINE);
    let lit: Vec<(usize, usize)> = (physical(lane_top())..physical(lane_top() + 32.0))
        .flat_map(|row| (physical(left)..physical(right)).map(move |column| (column, row)))
        .filter(|&(column, row)| {
            let at = (row * shot.width as usize + column) * 4;
            near(
                [shot.rgba[at], shot.rgba[at + 1], shot.rgba[at + 2]],
                accent,
                TOLERANCE,
            )
        })
        .collect();
    assert!(
        lit.is_empty(),
        "no handle on a clip at rest: accent at {lit:?}"
    );
    assert_shaded(&shot);
}
