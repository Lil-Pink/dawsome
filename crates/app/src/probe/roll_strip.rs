//! The piano roll's strip as the window lays it out, read from the layout dumps of p3_roll's
//! shots with the roll docked at 700 px (`--dump-layout`): the channel's name, then
//! [`TITLE_GAP`] on its controls in groups ([`STRIP_GROUP_GAP`] apart, their controls
//! [`STRIP_SPACING`] apart), folded by the strip's law, the Scale picker [`scale_width`] wide,
//! Notes after Fold, the roll's Play and Loop after Notes, and the pop-out target at the right
//! end. Positions come from the dump, the gaps from [`crate::ui::piano_layout`] and
//! [`crate::ui::panels`]. Then the roll's player playing (`--play-roll`): its Stop lit, the
//! toolbar's Pat and Song both unlit, and the playhead in the roll and on the rack's steps at
//! one tick.

use std::path::Path;

use serde_json::Value;

use dawsome_engine::time::Ticks;
use iced::Point;

use super::tests::{read_ui_shot, rgb8};
use super::{near, TOLERANCE};
use crate::ui::layout::{self, TOOLBAR_HEIGHT};
use crate::ui::note_gesture::Snap;
use crate::ui::panels::TITLE_GAP;
use crate::ui::piano_layout::{
    self, View, MENU_ARROW, MENU_ARROW_GAP, PLAYER_GAP, PLAYER_SIDE, PLAY_BUTTON, RULER_HEIGHT,
    STRIP_GROUP_GAP, STRIP_HEIGHT, STRIP_PADDING, STRIP_SPACING,
};
use crate::ui::piano_roll::{scale_width, StripFold};
use crate::ui::pop_out;
use crate::ui::style;
use crate::ui::text_fit::text_width;

/// A box's left edge and width along the strip, in the window's logical pixels.
#[derive(Debug, Clone, Copy)]
struct Span {
    left: f32,
    width: f32,
}

impl Span {
    fn right(self) -> f32 {
        self.left + self.width
    }

    fn of(entry: &Value) -> Self {
        let number = |key: &str| {
            entry[key]
                .as_f64()
                .unwrap_or_else(|| panic!("{key} in {entry}")) as f32
        };
        Span {
            left: number("x"),
            width: number("width"),
        }
    }
}

/// A layout dump as `--dump-layout` writes it, and where the roll's strip starts in it: under
/// the toolbar in the main window, at the top of the roll's own window.
struct Dump {
    json: Value,
    strip_top: f32,
}

impl Dump {
    /// The main window's dump `DAWSOME_UI_DUMP` names.
    fn read() -> Self {
        Self::read_with_strip_at(TOOLBAR_HEIGHT)
    }

    /// The dump `DAWSOME_UI_DUMP` names, the strip from window y `strip_top`.
    fn read_with_strip_at(strip_top: f32) -> Self {
        let path = std::env::var_os("DAWSOME_UI_DUMP")
            .expect("DAWSOME_UI_DUMP names the layout dump to read");
        let text = std::fs::read_to_string(Path::new(&path))
            .unwrap_or_else(|e| panic!("read {}: {e}", Path::new(&path).display()));
        Dump {
            json: serde_json::from_str(&text).expect("the dump is JSON"),
            strip_top,
        }
    }

    /// The strip's band, window y.
    fn strip(&self) -> std::ops::Range<f32> {
        self.strip_top..self.strip_top + STRIP_HEIGHT
    }

    fn entries(&self, kind: &str) -> &[Value] {
        self.json[kind]
            .as_array()
            .unwrap_or_else(|| panic!("the dump's {kind}"))
    }

    fn window_width(&self) -> f32 {
        self.json["window"]["width"]
            .as_f64()
            .expect("the window's width") as f32
    }

    /// The text run in the strip starting with `start`: its path in the walk and its box.
    fn text(&self, start: &str) -> (String, Span) {
        self.text_in(start, self.strip())
    }

    /// The text run starting with `start` whose top lies in `band` (window y): its path in
    /// the walk and its box.
    fn text_in(&self, start: &str, band: std::ops::Range<f32>) -> (String, Span) {
        self.text_where(start, band, |text| text.starts_with(start))
    }

    /// The text run whose words `matches`, its top in `band` (window y; `what` names it in a
    /// failure): its path in the walk and its box.
    fn text_where(
        &self,
        what: &str,
        band: std::ops::Range<f32>,
        matches: impl Fn(&str) -> bool,
    ) -> (String, Span) {
        let run = self
            .entries("texts")
            .iter()
            .find(|run| {
                let y = run["y"].as_f64().unwrap_or(-1.0) as f32;
                band.contains(&y) && run["text"].as_str().is_some_and(&matches)
            })
            .unwrap_or_else(|| panic!("no text \"{what}\" in {band:?}"));
        let path = run["path"].as_str().expect("a run's path").to_owned();
        (path, Span::of(run))
    }

    /// The words of every text run in the strip.
    fn strip_texts(&self) -> impl Iterator<Item = &str> {
        let strip = self.strip();
        self.entries("texts")
            .iter()
            .filter(move |run| strip.contains(&(run["y"].as_f64().unwrap_or(-1.0) as f32)))
            .filter_map(|run| run["text"].as_str())
    }

    /// Whether a text run in the strip says `words` exactly.
    fn has_text(&self, words: &str) -> bool {
        self.strip_texts().any(|text| text == words)
    }

    /// The box the walk met at `path`.
    fn boxed(&self, path: &str) -> Span {
        let found = self
            .entries("boxes")
            .iter()
            .find(|entry| entry["path"].as_str() == Some(path))
            .unwrap_or_else(|| panic!("no box at {path}"));
        Span::of(found)
    }

    /// The button holding the text starting with `start`, and the row holding that button.
    fn button(&self, start: &str) -> (Span, Span) {
        let (path, _) = self.text(start);
        let button = parent(&path);
        (self.boxed(button), self.boxed(parent(button)))
    }

    /// The box holding the text run at `path` and that box's top and height (window y).
    fn holder(&self, path: &str) -> (Span, f32, f32) {
        let found = self
            .entries("boxes")
            .iter()
            .find(|entry| entry["path"].as_str() == Some(parent(path)))
            .unwrap_or_else(|| panic!("no box holds {path}"));
        let number = |key: &str| found[key].as_f64().expect(key) as f32;
        (Span::of(found), number("y"), number("height"))
    }
}

/// The path of what holds the widget at `path`.
fn parent(path: &str) -> &str {
    path.rsplit_once('.').map_or("", |(parent, _)| parent)
}

// The strip of p3_roll docked at 700 px: the roll beside the Library (300 px) in a 1000 x 560
// window, the channel "lead", no velocity drag, no take; once with no scale, once with Minor
// pentatonic chosen (`--roll-scale minor-pentatonic`; its root's chooser shows, at C). Laid out
// by the strip's constants and folded by its law (crate::ui::piano_roll::StripFold, its table
// derived by hand in `the_strip_folds_by_its_width`): with no scale it needs 733.93 px unfolded,
// 657.61 with the Len readout folded, so the Len readout goes (into the snap picker's hint) and
// nothing else; with the scale and its root 790.48 and 714.16, 647.41 with Preview and Follow
// as 24 px glyphs too, so those fold as well, and the Scale chooser keeps its 62 px.
// From the roll's left edge at x 300: the name from the 12 px margin, "lead" 27.99 (to 339.99),
// then, the title gap (48) on, the groups 20 px apart, two of the 10 px spacing that parts the
// controls inside a group. The snap picker alone, as wide as its widest entry by the app's text
// measure, "None" 30.32, plus the toolkit's arrow (as wide as the text is tall, 12 px) and
// padding (6 a side and 6 before the arrow): 60.32, at 387.99..448.31. With no scale:
// Preview (45.64 + 16) at 468.31..529.95 and Follow (37.11 + 16) a spacing on, 539.95..593.06;
// the Scale group at 613.06, its chooser 62 px, Fold (24.53 + 16) a spacing after it, at
// 685.06..725.59; Notes ("Notes" 33.49, the drawn arrow's 8 px square 4 after it, + 16: 61.49)
// at 745.59..807.09; the roll's player at 827.09..911.61, 84.52 wide: 5 px of plain room either
// side round Play, 26, and Loop, 28.52 + 16, 4 px apart: Play at 832.09..858.09, Loop at
// 862.09..906.61, both 20 tall at window y 68..88, the strip's middle (64 + 4). With the scale:
// Preview and Follow 24 px glyph buttons at 468.31..492.31 and 502.31..526.31; the Scale group
// at 546.31, the chooser 62 px, the root's chooser ("G#", the widest root, 16.55 + 30) a spacing
// on at 618.31..664.86 and Fold at 674.86..715.39; Notes at 735.39..796.89; the player at
// 816.89..901.41, Loop to 896.41. Either way the pop-out grip's 24 px target, docked, is the
// strip's last box, x 1000 − 12 − 24 = 964..988, at least a spacing right of the player's room.
// (Pins moved: Notes's arrow was a "▾" in its label, which Inter has not, drawn by the system's
// face, 7.911 px on this Linux, 0.71 px narrower than the drawn one, so every pin from Notes on
// stood that much left (Loop to 905.90 and 895.70); a box drew
// round Play and Loop in that room (its edge and inset the 5 px) and went the same day, nothing
// moving; before it Play and Loop stood a spacing apart, Loop to 906.90 and 896.70; the probe
// read the roll alone at
// 1000 px, unfolded, its controls to 683.22 (the Snap group stood at 87.99, "Len" a spacing
// after the picker), until the strip took its fold law; before that the name stood a group's
// gap, 20, before the Snap group, and Notes at the strip's right end before the button, at 954;
// then the title gap and Notes beside Fold; "Snap" stood before its picker, 28.875 px and a
// spacing, until the picker stood alone; the Snap group stood the title gap after the name, and
// the Scale picker 125.99 px wide, until the roll's player came first; Play and Loop stood first,
// the title gap after the name, until they moved right of Notes.)
//
// The roll popped out at its own window's least width, 608 px (--pop-out piano-roll, its place
// kept 608 wide), its strip from the window's top and left: in its own window it has no grip,
// so it needs 556.86 with no scale, the length and Preview and Follow folded (623.61 > 608), and
// 599.41 with the scale, the Scale chooser folded too (613.41 > 608). From x 0: the name to
// 39.99, the snap picker 87.99..148.31, the glyphs 168.31..192.31 and 202.31..226.31, the Scale
// group at 246.31. With no scale the chooser 62 px, Fold 318.31..358.84, Notes 378.84..440.33,
// the player 460.33..544.86 (Loop to 539.86); with the scale the chooser folded to 48 px, the
// root's 304.31..350.86, Fold 360.86..401.39, Notes 421.39..482.89, the player 502.89..587.41
// (Loop to 582.41), 8.59 px short of the right margin (608 − 12 = 596). Then with the channel
// named "Bass 808 low" (a
// copy of p3_roll renamed, the stage's own): whole it needs 660.50, so the name is cut to its
// least, "Bas…" (36.51; "Bass…", 43.90, is wider than the 36.58 the controls leave): the snap
// picker 96.51..156.83 (12 + 36.51 + 48 on), every control 36.51 − 27.99 = 8.52 px right of
// where it stands beside "lead", the player 511.41..595.93 (Loop to 590.93), 0.07 px short of
// the right margin. No 24 px box right of the player, and Notes's word whole. (Pins moved: the
// window's least width was 600, until a cut name's least showed it could not hold "Bas…", then
// 604, until the player's box took 4 px more; the pins here stood 0.71 px left, by this
// system's "▾", until Notes's arrow was drawn.)
struct Case {
    /// The channel's name as the strip shows it.
    name: &'static str,
    /// The roll's left edge in the window.
    left: f32,
    /// Where the strip's band starts, window y.
    top: f32,
    /// Minor pentatonic chosen: its root's chooser shows.
    scale: bool,
    /// How far the strip folds, by the hand table above.
    fold: StripFold,
    /// Where Loop ends, by hand.
    end: f32,
    /// In the stack: the pop-out grip ends the strip.
    docked: bool,
}

fn the_strip_folds(case: Case) {
    use crate::ui::piano_layout::{GLYPH_SWITCH, SCALE_FOLDED_WIDTH};
    use crate::ui::scale::Root;
    use crate::ui::short_pick_list::natural_width;
    let Case {
        name: shown,
        left: roll,
        top,
        scale,
        fold: folded,
        end,
        docked,
    } = case;
    let dump = Dump::read_with_strip_at(top);
    let label = |words: &str| text_width(words, crate::ui::fonts::size::LABEL);
    let (_, name) = dump.text(shown);
    // The player's row (Play, Loop, its plain room either side) among the groups.
    let (looping, player) = dump.button("Loop");
    let (loop_path, _) = dump.text("Loop");
    let play = dump.boxed(&format!("{}.0", parent(parent(&loop_path))));
    let (fold, scale_group) = dump.button("Fold");
    // Notes's word and its drawn arrow stand in a row, inside its button.
    let (notes_path, notes_words) = dump.text("Notes");
    let notes = dump.boxed(parent(parent(&notes_path)));
    let (fold_path, _) = dump.text("Fold");
    // The groups' row holds the Scale group, which holds Fold's button.
    let groups = parent(parent(parent(&fold_path)));
    let snap_group = dump.boxed(&format!("{groups}.0"));
    let switches = dump.boxed(&format!("{groups}.1"));
    let switch = |place: usize| dump.boxed(&format!("{groups}.1.{place}"));
    let (preview, follow) = (switch(0), switch(1));
    let snap_picker = natural_width(&Snap::ALL, crate::ui::fonts::size::LABEL);
    let root = natural_width(&Root::ALL, crate::ui::fonts::size::LABEL);
    let words: Vec<&str> = dump.strip_texts().collect();
    println!(
        "a scale {scale}, docked {docked}: name {name:?}; snap group {snap_group:?}; switches \
         {switches:?} (Preview {preview:?}, Follow {follow:?}); scale group {scale_group:?} \
         (Fold {fold:?}); Notes {notes:?}; player {player:?} (Play {play:?}, \
         Loop {looping:?}); the strip's \
         words {words:?}"
    );
    let at = |found: f32, expected: f32, what: &str| {
        assert!(
            (found - expected).abs() < 0.01,
            "{what}: at {found}, the constants put it at {expected}"
        );
    };
    assert!(folded >= StripFold::Length, "every case folds the length");
    assert!(
        words
            .iter()
            .all(|text| !text.starts_with("Len ") && *text != "Snap"),
        "the Len readout folded: {words:?}"
    );
    at(name.left, roll + STRIP_PADDING, "the name");
    at(
        name.width,
        text_width(shown, crate::ui::fonts::size::PANEL_TITLE),
        "the name as the strip cuts it",
    );
    at(snap_group.left, name.right() + TITLE_GAP, "the snap picker");
    at(snap_group.width, snap_picker, "the snap picker alone");
    at(
        switches.left,
        snap_group.right() + STRIP_GROUP_GAP,
        "Preview and Follow",
    );
    let glyphs = folded >= StripFold::Glyphs;
    let switch_width = |words: &str| {
        if glyphs {
            GLYPH_SWITCH.width
        } else {
            label(words) + 16.0
        }
    };
    at(preview.left, switches.left, "Preview first in them");
    at(preview.width, switch_width("Preview"), "Preview's width");
    at(
        follow.left,
        preview.right() + STRIP_SPACING,
        "Follow beside Preview",
    );
    at(follow.width, switch_width("Follow"), "Follow's width");
    at(follow.right(), switches.right(), "Follow last in them");
    assert_eq!(
        (dump.has_text("Preview"), dump.has_text("Follow")),
        (!glyphs, !glyphs),
        "their words until they fold to glyphs"
    );
    at(
        scale_group.left,
        switches.right() + STRIP_GROUP_GAP,
        "the Scale group",
    );
    let chooser = if folded >= StripFold::Scale {
        SCALE_FOLDED_WIDTH
    } else {
        scale_width()
    };
    let before_fold = if scale {
        chooser + STRIP_SPACING + root
    } else {
        chooser
    };
    at(
        fold.left,
        scale_group.left + before_fold + STRIP_SPACING,
        "Fold after the chooser (and the root's)",
    );
    at(fold.right(), scale_group.right(), "Fold last in it");
    at(
        notes.left,
        fold.right() + STRIP_GROUP_GAP,
        "Notes a group's gap after Fold",
    );
    at(notes_words.width, label("Notes"), "Notes's word whole");
    at(
        notes.width,
        label("Notes") + MENU_ARROW_GAP + MENU_ARROW + 16.0,
        "Notes's button: its word, the drawn arrow a gap after it, the padding",
    );
    at(
        player.left,
        notes.right() + STRIP_GROUP_GAP,
        "the player a group's gap after Notes",
    );
    at(
        play.left,
        player.left + PLAYER_SIDE,
        "Play its side's room in",
    );
    at(play.width, PLAY_BUTTON.width, "Play's width");
    at(
        looping.left,
        play.right() + PLAYER_GAP,
        "Loop a player's gap after Play",
    );
    at(
        looping.right(),
        player.right() - PLAYER_SIDE,
        "Loop its side's room from the player's end",
    );
    let (_, play_top, play_height) = dump.holder(&format!("{}.0.0", parent(parent(&loop_path))));
    let (_, loop_top, loop_height) = dump.holder(&loop_path);
    at(play_height, PLAY_BUTTON.height, "Play 20 tall");
    at(loop_height, PLAY_BUTTON.height, "Loop as tall as Play");
    at(loop_top, play_top, "Loop level with Play");
    at(
        play_top,
        top + (STRIP_HEIGHT - PLAY_BUTTON.height) / 2.0,
        "the pair in the strip's middle",
    );
    at(looping.right(), end, "Loop's end");
    let button = pop_out::SIZE;
    let strip = dump.strip();
    let grip = dump
        .entries("boxes")
        .iter()
        .filter(|entry| strip.contains(&(entry["y"].as_f64().unwrap_or(-1.0) as f32)))
        .map(Span::of)
        .find(|span| (span.width - button).abs() < 0.01 && span.left > player.right());
    if !docked {
        assert!(grip.is_none(), "no pop-out grip in the roll's own window");
        assert!(
            player.right() <= dump.window_width() - STRIP_PADDING,
            "the player and its room inside the right margin"
        );
        return;
    }
    let grip = grip.expect("the pop-out target right of Loop");
    at(
        grip.left,
        dump.window_width() - STRIP_PADDING - button,
        "the pop-out target at the right end",
    );
    assert!(
        grip.left - player.right() >= STRIP_SPACING - 0.01,
        "the player and its room at least a spacing from the pop-out target"
    );
}

#[test]
#[ignore = "needs the layout dump of p3_roll with --panels browser,piano-roll (--dump-layout): scripts/ui-check.sh writes one and runs this"]
fn p3_the_rolls_strip_at_700_folds_its_length() {
    the_strip_folds(Case {
        name: "lead",
        left: crate::ui::browser::WIDTH,
        top: TOOLBAR_HEIGHT,
        scale: false,
        fold: StripFold::Length,
        end: 906.61,
        docked: true,
    });
}

#[test]
#[ignore = "needs the layout dump of p3_roll with --panels browser,piano-roll --roll-scale minor-pentatonic (--dump-layout): scripts/ui-check.sh writes one and runs this"]
fn p3_the_rolls_strip_at_700_with_a_scale_folds_preview_and_follow_too() {
    the_strip_folds(Case {
        name: "lead",
        left: crate::ui::browser::WIDTH,
        top: TOOLBAR_HEIGHT,
        scale: true,
        fold: StripFold::Glyphs,
        end: 896.41,
        docked: true,
    });
}

#[test]
#[ignore = "needs the layout dump of p3_roll's own window at its least width (--pop-out piano-roll --screenshot-window piano-roll --dump-layout): scripts/ui-check.sh writes one and runs this"]
fn p3_the_rolls_own_window_at_its_least_folds_its_length_and_switches() {
    the_strip_folds(Case {
        name: "lead",
        left: 0.0,
        top: 0.0,
        scale: false,
        fold: StripFold::Glyphs,
        end: 539.86,
        docked: false,
    });
}

#[test]
#[ignore = "needs the layout dump of p3_roll's own window at its least width with --roll-scale minor-pentatonic (--dump-layout): scripts/ui-check.sh writes one and runs this"]
fn p3_the_rolls_own_window_at_its_least_with_a_scale_shows_every_control() {
    the_strip_folds(Case {
        name: "lead",
        left: 0.0,
        top: 0.0,
        scale: true,
        fold: StripFold::Scale,
        end: 582.41,
        docked: false,
    });
}

#[test]
#[ignore = "needs the layout dump of a copy of p3_roll with its channel named Bass 808 low, in its own window at its least width with --roll-scale minor-pentatonic (--dump-layout): scripts/ui-check.sh writes one and runs this"]
fn p3_the_rolls_own_window_at_its_least_cuts_a_long_name_to_fit() {
    the_strip_folds(Case {
        name: "Bas\u{2026}",
        left: 0.0,
        top: 0.0,
        scale: true,
        fold: StripFold::Name,
        end: 590.93,
        docked: false,
    });
}

// Spec 3.4: rp_keys_drums with the rack and the roll at 1000 x 700, the roll's player playing
// (`--play-roll`, `--null-output`), shot mid-pass. The toolbar's Pat and Song both on the unlit
// switch's ground (the lit side's is the LCD's); the roll's Play (left of the strip's Loop) lit,
// the Stop square in the accent at its middle (Play's triangle leaves that pixel clear only while stopped); the
// playhead an accent line in the roll's grid at x = 48 + tick · 16 / 24 (16 px a step, the
// view at the pattern's start) and an accent edge on the rack's steps (each step's top 2 px)
// of the 16th that holds that tick: the one playhead, a pattern's tick, as pattern mode draws
// it. Past the start: the shot is taken a second into the bar's two.
#[test]
#[ignore = "needs a shot of rp_keys_drums with --play-roll and its layout dump: scripts/ui-check.sh takes them and runs this"]
fn rp_the_roll_player_shows_its_playhead_in_the_roll_and_the_rack_at_one_tick() {
    let shot = read_ui_shot();
    let dump = Dump::read();
    let accent = |pixel| near(pixel, rgb8(style::accent()), TOLERANCE);
    for side in ["Pat", "Song"] {
        // Exactly the side's word: "Pattern 1", the pattern's name, starts with "Pat" too.
        let (path, _) = dump.text_where(side, 0.0..TOOLBAR_HEIGHT, |text| text == side);
        let (button, top, height) = dump.holder(&path);
        let ground = shot.pixel(Point::new(button.left + 2.0, top + height / 2.0));
        println!("{side}'s ground {ground:02X?}");
        assert!(
            near(ground, rgb8(style::palette().button), TOLERANCE),
            "{side} unlit"
        );
    }
    let (name_path, _) = dump.text_in("keys", TOOLBAR_HEIGHT..700.0);
    let (_, strip_top, strip_height) = dump.holder(&name_path);
    let strip_middle = strip_top + strip_height / 2.0;
    // Play is a spacing left of the strip's Loop (the toolbar's Loop lies above the band).
    let (loop_path, _) = dump.text_in("Loop", strip_top..strip_top + strip_height);
    let (looping, _, _) = dump.holder(&loop_path);
    let play = Point::new(
        looping.left - PLAYER_GAP - PLAY_BUTTON.width / 2.0,
        strip_middle,
    );
    assert!(accent(shot.pixel(play)), "the roll's Stop lit at {play:?}");
    let grid_y = strip_top + STRIP_HEIGHT + RULER_HEIGHT + 40.0;
    let line = (48..1000)
        .map(|x| x as f32)
        .find(|&x| accent(shot.pixel(Point::new(x, grid_y))))
        .expect("the playhead in the roll's grid");
    let view = View {
        top: 72,
        scroll: Ticks(0),
        zoom: piano_layout::ZOOM.default,
        step: Ticks(24),
        signature: dawsome_engine::time::TimeSignature::COMMON,
        rows: piano_layout::Rows::ALL,
    };
    // The ticks whose line starts in the pixel column the line was found in.
    let ticks: Vec<i64> = (0..384)
        .filter(|&tick| view.x_of(Ticks(tick)).floor() == line)
        .collect();
    let marked: Vec<u32> = (0..16)
        .filter(|&step| {
            let rect = layout::step_rect(0, step, 4);
            let edge = Point::new(rect.center_x(), rect.y + layout::PLAYHEAD_EDGE / 2.0);
            accent(shot.pixel(layout::rack_in_window(edge)))
        })
        .collect();
    println!("the roll's line at x {line} (ticks {ticks:?}); the rack's marked steps {marked:?}");
    assert_eq!(marked.len(), 1, "one step marked");
    let step = i64::from(marked[0]);
    assert!(step > 0, "past the pass's start");
    assert!(
        ticks.iter().any(|tick| tick / 24 == step),
        "the roll's playhead in the rack's 16th"
    );
}
