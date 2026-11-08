//! Dawsome Synth's instrument window, shot alone: its headings as a console's modules, placed
//! as a DJ's booth, its strip's LEVEL and its presets as modules under the decks.

use dawsome_engine::dsp::synth;
use dawsome_engine::processor::Registry;
use iced::{Point, Size, Vector};

use super::tests::{read_ui_shot, rgb8};
use super::{near, TOLERANCE};
use crate::ui::console::{self, chassis_rect};
use crate::ui::instrument_window::{HINT_HEIGHT, PADDING, ROW_GAP, TITLE_HEIGHT};
use crate::ui::knob_block::{choice_rect, grid_cell, well_rect};
use crate::ui::synth_panel::{booth, PRESETS_HEADING};

/// The window's first size.
const WINDOW: Size = Size::new(924.0, 516.0);

/// The chassis's surface at window height `y`, its two colors mixed along its height as the
/// GPU renderer mixes a linear gradient's stops: eased, by `smoothstep` (iced_wgpu's gradient
/// shader), so the top and the foot hold their colors longer than a straight mix would. Its
/// place is the window over the foot line (the rows reckoned at their round heights; the real
/// ones are a little shorter, which moves the mix by less than a tenth of a level).
fn chassis_at(y: f32) -> [u8; 3] {
    let chassis = chassis_rect(Size::new(WINDOW.width, WINDOW.height - HINT_HEIGHT));
    let straight = ((y - chassis.y) / chassis.height).clamp(0.0, 1.0);
    let along = straight * straight * (3.0 - 2.0 * straight);
    let shades = console::Shades::now();
    let (top, foot) = (rgb8(shades.chassis_top), rgb8(shades.chassis_foot));
    std::array::from_fn(|channel| {
        let (top, foot) = (f32::from(top[channel]), f32::from(foot[channel]));
        (top + (foot - top) * along).round() as u8
    })
}

// p33_sine's synth in its instrument window, opened at start and shot alone: the window at its
// first size, 924 × 516; its booth from (12, 40), under the title (20) and an 8 px gap, as wide
// as the room, so not moved to centre it; each of the eight blocks where `synth_panel::booth`
// places it, each a raised module: its face lit at its top (the module's top color 3 px down,
// right of its heading and left of its screw) and its knobs' well sunk in it (the well's color
// 3 px inside its bottom right corner); the presets' module under the right deck the same, its
// well's corner clear of its controls (they stand at its top); the chassis between the left
// deck and the mixer, at the window's height there; the LEVEL's module under the left deck,
// where bare chassis showed before, its face lit at its top right and its well sunk at
// (40, 238 + 100) of the booth (the strip's well runs from 2 px inside the face, under its
// 16 px header; its controls stand in its middle, from x 100); Osc 1's wave a raised key, its
// face left of its word.
#[test]
#[ignore = "needs a screenshot of p33_sine's instrument window (--open-instrument channel:1 --screenshot-window instrument): scripts/ui-check.sh takes one and runs this"]
fn s4_the_synth_window_shows_its_headings_as_blocks() {
    let shot = read_ui_shot();
    let size = (
        shot.width as f32 / shot.scale,
        shot.height as f32 / shot.scale,
    );
    println!("the window is {size:?}");
    assert_eq!(size, (WINDOW.width, WINDOW.height), "its first size");
    let registry = Registry::native();
    let factory = registry.get(synth::ID).expect("registered");
    let origin = Vector::new(PADDING, PADDING + TITLE_HEIGHT + ROW_GAP);
    assert_eq!(origin, Vector::new(12.0, 40.0));
    let booth = booth(factory);
    assert_eq!(booth.blocks.len(), 8);
    let shades = console::Shades::now();
    let headings = factory.groups().iter().map(|(name, _)| *name);
    let modules = headings
        .zip(booth.blocks.iter().map(|block| block.rect))
        .chain([(PRESETS_HEADING, booth.presets)]);
    for (name, rect) in modules {
        let lit = Point::new(rect.x + rect.width - 12.0, rect.y + 3.0);
        let face = shot.pixel(lit + origin);
        let well = well_rect(rect);
        let inside = Point::new(well.x + well.width - 3.0, well.y + well.height - 3.0);
        let sunk = shot.pixel(inside + origin);
        println!("the module of {name} at {rect:?}: face {face:02X?}, well {sunk:02X?}");
        assert!(
            near(face, rgb8(shades.face_top), TOLERANCE),
            "the module of {name}: its face, lit at its top"
        );
        assert!(
            near(sunk, rgb8(shades.well), TOLERANCE),
            "the module of {name}: its well"
        );
    }
    let between = Point::new(318.0, 70.0) + origin;
    let seen_between = shot.pixel(between);
    let chassis_between = chassis_at(between.y);
    println!(
        "between the left deck and the mixer {seen_between:02X?} (the chassis \
         {chassis_between:02X?})"
    );
    assert!(
        near(seen_between, chassis_between, TOLERANCE),
        "the chassis between a deck and the mixer"
    );
    let level = booth.level;
    assert_eq!(
        (level.x, level.y, level.width, level.height),
        (0.0, 238.0, 312.0, 176.0)
    );
    let lit = shot.pixel(Point::new(level.x + level.width - 12.0, level.y + 3.0) + origin);
    let sunk = shot.pixel(Point::new(level.x + 40.0, level.y + 100.0) + origin);
    println!("the LEVEL's module under the left deck: face {lit:02X?}, well {sunk:02X?}");
    assert!(
        near(lit, rgb8(shades.face_top), TOLERANCE),
        "the LEVEL's module, lit at its top, under the left deck"
    );
    assert!(
        near(sunk, rgb8(shades.well), TOLERANCE),
        "the LEVEL's well where the chassis showed under the left deck"
    );
    let osc1 = &booth.blocks[0];
    let wave = choice_rect(grid_cell(osc1.rect, osc1.columns, 0));
    let key = shot.pixel(Point::new(wave.x + 3.0, wave.center_y()) + origin);
    println!("Osc 1's wave key {key:02X?}");
    assert!(
        near(key, rgb8(shades.key), TOLERANCE),
        "a choice's raised key"
    );
}
