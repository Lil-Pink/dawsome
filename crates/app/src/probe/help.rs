//! The Help window's screenshot probe (help guide H2, H4). Positions come from the window's own
//! layout constants (`ui/help_window.rs`), colors from [`crate::ui::style`].

use iced::Point;

use super::tests::{read_ui_shot, rgb8};
use super::{near, TOLERANCE};
use crate::ui::help_window::{CONTENTS_WIDTH, WINDOW_SIZE};
use crate::ui::layout::HINT_BAR_HEIGHT;
use crate::ui::style;

// H2, H4: the Help window opened at start (--show help), shot alone (--screenshot-window help)
// at its first size: the contents down its left on the panels' ground with the first entry lit
// (the button's ground behind "1. What Dawsome does", 12 px under the "Contents" line), the page
// right of it on the well's ground, the foot line on the panels' ground, and the first section's
// picture of the main window drawn on the page (most of the page's lower half is neither the
// well's ground nor the text: a missing picture would leave it the ground).
#[test]
#[ignore = "needs a screenshot of the Help window: scripts/ui-check.sh takes one and runs this"]
fn h1_the_help_window_shows_its_contents_its_page_and_a_picture() {
    let shot = read_ui_shot();
    assert_eq!(
        (
            shot.width as f32 / shot.scale,
            shot.height as f32 / shot.scale
        ),
        (WINDOW_SIZE.width, WINDOW_SIZE.height),
        "the Help window at its first size"
    );
    let (panel, well, lit) = (
        rgb8(style::PANEL),
        rgb8(style::RACK_WELL),
        rgb8(style::BUTTON),
    );
    let contents = shot.pixel(Point::new(CONTENTS_WIDTH / 2.0, WINDOW_SIZE.height - 60.0));
    let entry = shot.pixel(Point::new(CONTENTS_WIDTH - 16.0, 51.0));
    let page = shot.pixel(Point::new(WINDOW_SIZE.width - 40.0, 12.0));
    let foot = shot.pixel(Point::new(
        WINDOW_SIZE.width / 2.0,
        WINDOW_SIZE.height - HINT_BAR_HEIGHT / 4.0,
    ));
    println!(
        "contents {contents:02X?}, first entry {entry:02X?}, page {page:02X?}, foot {foot:02X?}"
    );
    assert!(near(contents, panel, TOLERANCE), "the contents' ground");
    assert!(near(entry, lit, TOLERANCE), "the first entry lit");
    assert!(near(page, well, TOLERANCE), "the page's ground");
    assert!(near(foot, panel, TOLERANCE), "the foot line");
    let lower = (WINDOW_SIZE.height / 2.0)..(WINDOW_SIZE.height - HINT_BAR_HEIGHT - 4.0);
    let across = (CONTENTS_WIDTH + 24.0)..(WINDOW_SIZE.width - 40.0);
    let area = ((across.end - across.start) * (lower.end - lower.start) * shot.scale * shot.scale)
        as usize;
    let drawn = shot.count_matching(across, lower, |pixel| !near(pixel, well, 6));
    println!("{drawn} of {area} pixels of the page's lower half are drawn on");
    assert!(drawn * 2 > area, "the first section's picture shows");
}
