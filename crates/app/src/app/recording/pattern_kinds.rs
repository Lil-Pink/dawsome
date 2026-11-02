//! Patterns and takes. The rules: in song mode the right arrow past the last pattern makes a
//! new take; switching into song mode with a blank pattern selected makes it a blank take in
//! place, and switching back makes a blank take a pattern; a pattern or take that holds notes
//! stays as it is, where it is in the menu, and a take started after the switch makes the new
//! pattern or take. Before them, the right arrow made a pattern in song mode, and Record then
//! made a take beside it and left the blank pattern behind. Also pinned: a key struck in the
//! count-in and held into bar 1 starts the take, and the Library previews nothing while a take
//! of notes runs. Expected values are these rules and p17_song's numbers (44.1 kHz, 120 BPM,
//! PPQ 96, a 384-tick bar, a one-bar count-in of 88 200 frames), never the code's output.

use std::path::Path;

use dawsome_engine::project::{ChannelId, PatternId, PatternKind};

use super::super::patterns::PatternMessage;
use super::super::{App, Message, PlayMode};
use super::library_preview_take::{
    armed_in_song_mode, calls, l_key, notes, take_of_l, COUNT_IN_CALLS, L_KEY,
};
use crate::app::library_preview::PREVIEW_AFTER_STOP;
use crate::session::PREVIEW_CHANNEL;

/// A pattern as the tests see it: its name, its kind and its notes as (channel, key).
type Shown = (String, PatternKind, Vec<(u32, u8)>);

/// Pattern `id` as [`Shown`], or none when it is gone.
fn pattern(app: &App, id: u32) -> Option<Shown> {
    let found = app.session.project().patterns.get(&PatternId(id))?;
    let notes = notes(app, PatternId(id)).unwrap_or_default();
    Some((found.name.clone(), found.kind, notes))
}

fn blank_take(name: &str) -> Option<Shown> {
    Some((name.to_owned(), PatternKind::Take, Vec::new()))
}

fn next(app: &mut App) {
    let _ = app.update(Message::Pattern(PatternMessage::Next));
}

// Song mode: the right arrow past Pattern 1 makes blank take 2, "Kick take 1" after the selected
// channel, and Record records into it: no new pattern, the take one undo step that leaves the
// blank take as the arrow made it.
#[test]
fn the_right_arrow_in_song_mode_makes_a_take_that_record_fills() {
    let (mut app, mut player) = armed_in_song_mode("arrow-take");
    next(&mut app);
    assert_eq!(app.pattern, Some(PatternId(2)));
    assert_eq!(pattern(&app, 2), blank_take("Kick take 1"));
    let recorded = take_of_l(&mut app, &mut player);
    assert_eq!(recorded, PatternId(2), "into the blank take");
    assert_eq!(
        app.session.project().patterns.len(),
        2,
        "no pattern beside it"
    );
    assert_eq!(
        pattern(&app, 2),
        Some((
            "Kick take 1".to_owned(),
            PatternKind::Take,
            vec![(1, L_KEY)]
        ))
    );
    let _ = app.update(Message::Undo);
    assert_eq!(pattern(&app, 2), blank_take("Kick take 1"), "one undo step");
}

// A blank take made with no channel selected is "Take 2"; recorded with Kick selected, it is
// named after Kick, and the rename is part of the take.
#[test]
fn a_blank_take_recorded_is_named_after_the_channel() {
    let (mut app, mut player) = armed_in_song_mode("take-name");
    app.selected_channel = None;
    next(&mut app);
    assert_eq!(pattern(&app, 2), blank_take("Take 2"));
    app.selected_channel = Some(ChannelId(1));
    take_of_l(&mut app, &mut player);
    assert_eq!(
        pattern(&app, 2),
        Some((
            "Kick take 1".to_owned(),
            PatternKind::Take,
            vec![(1, L_KEY)]
        ))
    );
    let _ = app.update(Message::Undo);
    assert_eq!(pattern(&app, 2), blank_take("Take 2"));
}

// Pattern mode with a take that holds notes selected: the take stays as it is, and Record makes
// "Pattern 3" and records into it, one undo step; a take that plays no note leaves the document
// as it was and the take selected.
#[test]
fn record_in_pattern_mode_over_a_recorded_take_makes_a_pattern() {
    let (mut app, mut player) = armed_in_song_mode("take-to-pattern");
    next(&mut app);
    take_of_l(&mut app, &mut player);
    let _ = app.update(Message::ModeSelected(PlayMode::Pattern));
    assert_eq!(
        app.pattern,
        Some(PatternId(2)),
        "the take stays where it is"
    );
    let recorded_take = Some((
        "Kick take 1".to_owned(),
        PatternKind::Take,
        vec![(1, L_KEY)],
    ));
    assert_eq!(pattern(&app, 2), recorded_take);

    let before = app.session.project().clone();
    let _ = app.update(Message::Play);
    assert_eq!(
        app.pattern,
        Some(PatternId(3)),
        "the take's new pattern plays"
    );
    calls(&mut app, &mut player, COUNT_IN_CALLS + 10);
    let _ = app.update(Message::Stop);
    calls(&mut app, &mut player, 2);
    assert_eq!(
        app.session.project(),
        &before,
        "nothing played, nothing kept"
    );
    assert_eq!(app.pattern, Some(PatternId(2)));

    let recorded = take_of_l(&mut app, &mut player);
    assert_eq!(recorded, PatternId(3));
    assert_eq!(
        pattern(&app, 3),
        Some((
            "Pattern 3".to_owned(),
            PatternKind::Pattern,
            vec![(1, L_KEY)]
        ))
    );
    assert_eq!(pattern(&app, 2), recorded_take, "the take is untouched");
    let _ = app.update(Message::Undo);
    assert_eq!(
        pattern(&app, 3),
        None,
        "one undo step takes the new pattern out"
    );
    assert_eq!(app.pattern, Some(PatternId(2)));
}

// Pattern mode with a blank take selected (chosen after the switch): Record makes it a pattern
// in place, "Pattern 2", and records into it; one undo step gives the blank take back.
#[test]
fn record_in_pattern_mode_on_a_blank_take_makes_it_a_pattern() {
    let (mut app, mut player) = armed_in_song_mode("blank-take");
    next(&mut app);
    let _ = app.update(Message::PatternSelected(PatternId(1)));
    let _ = app.update(Message::ModeSelected(PlayMode::Pattern));
    let _ = app.update(Message::PatternSelected(PatternId(2)));
    assert_eq!(pattern(&app, 2), blank_take("Kick take 1"));
    let recorded = take_of_l(&mut app, &mut player);
    assert_eq!(recorded, PatternId(2));
    assert_eq!(app.session.project().patterns.len(), 2);
    assert_eq!(
        pattern(&app, 2),
        Some((
            "Pattern 2".to_owned(),
            PatternKind::Pattern,
            vec![(1, L_KEY)]
        ))
    );
    let _ = app.update(Message::Undo);
    assert_eq!(pattern(&app, 2), blank_take("Kick take 1"));
}

// Switching modes with a blank pattern selected makes it the other kind in place, one undo
// step each; one with notes stays as it is.
#[test]
fn switching_modes_flips_a_blank_pattern_and_leaves_a_recorded_one() {
    let (mut app, _player) = armed_in_song_mode("switch");
    next(&mut app);
    let _ = app.update(Message::ModeSelected(PlayMode::Pattern));
    assert_eq!(app.pattern, Some(PatternId(2)));
    let blank_pattern = Some(("Pattern 2".to_owned(), PatternKind::Pattern, Vec::new()));
    assert_eq!(pattern(&app, 2), blank_pattern);
    let _ = app.update(Message::Undo);
    assert_eq!(pattern(&app, 2), blank_take("Kick take 1"), "one undo step");
    let _ = app.update(Message::Redo);
    assert_eq!(pattern(&app, 2), blank_pattern);
    let _ = app.update(Message::ModeSelected(PlayMode::Song));
    assert_eq!(pattern(&app, 2), blank_take("Kick take 1"));

    let _ = app.update(Message::PatternSelected(PatternId(1)));
    let recorded = pattern(&app, 1);
    let _ = app.update(Message::ModeSelected(PlayMode::Pattern));
    let _ = app.update(Message::ModeSelected(PlayMode::Song));
    assert_eq!(
        pattern(&app, 1),
        recorded,
        "Pattern 1 holds a kick: it stays"
    );
    assert_eq!(
        recorded.map(|(_, kind, _)| kind),
        Some(PatternKind::Pattern)
    );
}

// L struck 10 calls before the count-in ends and let go of 10 calls into bar 1 starts the take:
// at tick 0, as long as it was held into the take. By hand: bar 1 begins at frame 88 200; the
// release is heard at the first frame of call 184, 183 · 512 = 93 696, so 5 496 frames in; the
// playhead's tick there is the last t with floor(t · 60 · 44 100 · 1000 / (120 000 · 96)) =
// floor(229.6875 t) at or before 5 496: t = 23 (5 282; 24 gives 5 512).
#[test]
fn a_key_struck_in_the_count_in_and_held_into_bar_1_starts_the_take() {
    let (mut app, mut player) = armed_in_song_mode("count-in");
    let _ = app.update(Message::Play);
    let take = app.pattern.expect("the pattern box shows the take");
    calls(&mut app, &mut player, COUNT_IN_CALLS - 10);
    l_key(&mut app, true);
    calls(&mut app, &mut player, 20);
    l_key(&mut app, false);
    calls(&mut app, &mut player, 10);
    let _ = app.update(Message::Stop);
    calls(&mut app, &mut player, 2);
    let landed: Vec<(i64, i64, u8)> = app.session.project().patterns[&take]
        .notes
        .iter()
        .map(|note| (note.start.0, note.length.0, note.key))
        .collect();
    assert_eq!(landed, [(0, 23, L_KEY)]);
}

// A Library row clicked while a take of notes runs previews nothing, and says it waits for
// Stop; the keys go on playing Kick, which the take records. After Stop the click previews.
#[test]
fn the_library_previews_nothing_while_a_take_of_notes_runs() {
    let (mut app, mut player) = armed_in_song_mode("preview-waits");
    let _ = app.update(Message::Play);
    let take = app.pattern.expect("the pattern box shows the take");
    calls(&mut app, &mut player, COUNT_IN_CALLS + 10);
    app.preview_set_at_start(Path::new("sets/pad/pad.sfz"));
    assert_eq!(app.live_target(), None, "the keys stay on Kick");
    assert!(app.browse.model.playing.is_none(), "no row is marked");
    assert_eq!(app.notice.as_deref(), Some(PREVIEW_AFTER_STOP));
    l_key(&mut app, true);
    calls(&mut app, &mut player, 10);
    l_key(&mut app, false);
    calls(&mut app, &mut player, 10);
    let _ = app.update(Message::Stop);
    calls(&mut app, &mut player, 2);
    assert_eq!(notes(&app, take), Some(vec![(1, L_KEY)]));
    app.preview_set_at_start(Path::new("sets/pad/pad.sfz"));
    assert_eq!(
        app.live_target(),
        Some(PREVIEW_CHANNEL),
        "after Stop it previews"
    );
}
