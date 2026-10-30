//! The piano roll's key edits: the arrows move the selected notes a semitone, an
//! octave (Shift) or the snap along time; Ctrl+C, Ctrl+X and Ctrl+V copy, cut and paste them.
//! Each edit is one undo step. The clipboard holds the notes last copied or cut, whatever
//! pattern and channel they came from, and a paste puts them on the channel the roll shows:
//! at the playhead while the song plays, else where the grid was last clicked (moved on past
//! each paste, so pasting again lays the copies end to end).

use dawsome_engine::time::Ticks;

use super::App;
use crate::ui::note_gesture::{snap_down, Target};
use crate::ui::note_ops::Quantize;
use crate::ui::roll::{NoteEdit, QuantizeMessage, Roll};

impl App {
    /// Carries out `edit` on the piano roll's notes, while it is open.
    pub(super) fn note_edit(&mut self, edit: NoteEdit) {
        if !self.panels.piano_roll {
            return;
        }
        self.roll.cancel_gesture();
        let Some(target) = Target::of(self.session.project(), self.pattern, self.selected_channel)
        else {
            return;
        };
        let commit = match edit {
            NoteEdit::Transpose(keys) => self.roll.transpose_selected(&target, keys),
            NoteEdit::Nudge(units) => self.roll.nudge_selected(&target, units),
            NoteEdit::Copy => {
                if let Some(clip) = self.roll.copy_selected(&target) {
                    self.clipboard = Some(clip);
                }
                None
            }
            NoteEdit::Cut => match self.roll.copy_selected(&target) {
                Some(clip) => {
                    self.clipboard = Some(clip);
                    self.roll.delete_selected(&target)
                }
                None => None,
            },
            NoteEdit::Paste => {
                let at = self.paste_tick(&target);
                match &self.clipboard {
                    Some(clip) => self.roll.paste(&target, clip, at),
                    None => None,
                }
            }
            NoteEdit::Delete => Roll::delete_selected(&self.roll, &target),
            NoteEdit::Duplicate => Roll::duplicate_selected(&self.roll, &target),
            NoteEdit::Quantize(_)
            | NoteEdit::Chop
            | NoteEdit::Glue
            | NoteEdit::Legato
            | NoteEdit::Reverse
            | NoteEdit::Invert
            | NoteEdit::Humanize => self.roll.operate(&target, edit),
        };
        if let Some(commit) = commit {
            self.commit_notes(commit);
        }
    }

    /// The Quantize dialog: opened with the last choice, changed, applied as one edit (and kept
    /// for the next opening), or put away.
    pub(super) fn quantize_message(&mut self, message: QuantizeMessage) {
        let form = &mut self.quantize_form;
        match message {
            QuantizeMessage::Open => {
                self.roll.menu = None;
                *form = Some(self.roll.quantize);
            }
            QuantizeMessage::Grid(grid) => form.iter_mut().for_each(|form| form.grid = grid),
            QuantizeMessage::Strength(value) => {
                form.iter_mut()
                    .for_each(|form| form.strength = value.min(100));
            }
            QuantizeMessage::Ends(ends) => form.iter_mut().for_each(|form| form.ends = ends),
            QuantizeMessage::Cancel => *form = None,
            QuantizeMessage::Apply => {
                let Some(chosen) = form.take() else {
                    return;
                };
                self.roll.quantize = chosen;
                let snap = self.roll.snap.unit(&self.roll.view);
                let how = Quantize {
                    grid: chosen.grid.ticks(snap, self.session.project().ppq),
                    strength: chosen.strength,
                    ends: chosen.ends,
                };
                self.note_edit(NoteEdit::Quantize(how));
            }
        }
    }

    /// Where a paste lands in `target`'s pattern: the playhead while the song plays (where it
    /// falls in the pattern, moved down to the snap), else the tick last clicked in the grid.
    fn paste_tick(&self, target: &Target) -> Ticks {
        match self.playhead {
            Some(tick) => {
                let within = Ticks(tick.0.rem_euclid(target.end().0.max(1)));
                snap_down(within, self.roll.snap.unit(&self.roll.view))
            }
            None => self.roll.paste_at(target.pattern),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use dawsome_engine::project::{ChannelId, PatternId};
    use dawsome_engine::realtime::Player;
    use dawsome_engine::time::Ticks;
    use iced::keyboard::key::{Code, Named};
    use iced::keyboard::{Key, Modifiers};
    use iced::{mouse, window, Point, Size};

    use super::super::key_input::KeyInput;
    use super::super::tests::app_on;
    use super::super::widget_events::Window;
    use super::super::{App, Message, Panel, PatternMessage};
    use crate::keys::NoteLayout;
    use crate::ui::hint::{Hover, HoverMessage, RollPart};
    use crate::ui::layout::TOOLBAR_HEIGHT;
    use crate::ui::piano_layout::STRIP_HEIGHT;
    use crate::ui::roll::{NoteEdit, QuantizeGrid, QuantizeMessage, RollMessage};

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    /// A key as the window hears it: what it types, where it lies, the modifiers held.
    #[derive(Clone)]
    struct Press(Key, Code, Modifiers);

    fn named(named: Named, code: Code, modifiers: Modifiers) -> Press {
        Press(Key::Named(named), code, modifiers)
    }

    fn up() -> Press {
        named(Named::ArrowUp, Code::ArrowUp, Modifiers::empty())
    }
    fn down() -> Press {
        named(Named::ArrowDown, Code::ArrowDown, Modifiers::empty())
    }
    fn shift_up() -> Press {
        named(Named::ArrowUp, Code::ArrowUp, Modifiers::SHIFT)
    }
    fn shift_down() -> Press {
        named(Named::ArrowDown, Code::ArrowDown, Modifiers::SHIFT)
    }
    fn left() -> Press {
        named(Named::ArrowLeft, Code::ArrowLeft, Modifiers::empty())
    }
    fn right() -> Press {
        named(Named::ArrowRight, Code::ArrowRight, Modifiers::empty())
    }
    fn del() -> Press {
        named(Named::Delete, Code::Delete, Modifiers::empty())
    }
    fn ctrl(letter: &str, code: Code) -> Press {
        Press(Key::Character(letter.into()), code, Modifiers::CTRL)
    }

    /// The key pressed and let go of, through the window's own path.
    fn tap(app: &mut App, press: &Press) {
        let Press(key, code, modifiers) = press.clone();
        for pressed in [true, false] {
            let _ = app.update(Message::Key(KeyInput {
                window: window::Id::unique(),
                key: key.clone(),
                code: Some(code),
                modifiers,
                pressed,
                repeat: false,
            }));
        }
    }

    /// What the key's press asks for, without carrying it out.
    fn asked(app: &mut App, press: &Press) -> Option<Message> {
        let Press(key, code, modifiers) = press.clone();
        app.key_message(KeyInput {
            window: window::Id::unique(),
            key,
            code: Some(code),
            modifiers,
            pressed: true,
            repeat: false,
        })
    }

    /// p3_roll with the piano roll opened by its key: the rack and the roll, channel 1
    /// ("lead": A 0..48 on key 60, B 96..192 on 64; the kicks on channel 2), the roll with the
    /// keys, the computer keyboard playing the piano row.
    fn p3() -> (App, Player) {
        let (mut app, player) = app_on(&Path::new(FIXTURES).join("p3_roll.json"));
        let _ = app.update(Message::TogglePanel(Panel::PianoRoll));
        assert!(app.roll_has_keys() && app.typing.on);
        (app, player)
    }

    fn document(app: &App) -> String {
        app.session.project().to_json().expect("serializes")
    }

    fn fixture(name: &str) -> String {
        let text = std::fs::read_to_string(Path::new(FIXTURES).join(name)).expect("reads");
        text.trim_end().to_owned()
    }

    /// `channel`'s notes in `pattern`, as (start, length, key).
    fn notes_of(app: &App, pattern: u32, channel: u32) -> Vec<(i64, i64, u8)> {
        app.session.project().patterns[&PatternId(pattern)]
            .notes
            .iter()
            .filter(|note| note.channel == ChannelId(channel))
            .map(|note| (note.start.0, note.length.0, note.key))
            .collect()
    }

    fn lead(app: &App) -> Vec<(i64, i64, u8)> {
        notes_of(app, 1, 1)
    }

    /// The lead notes starting at `starts` selected.
    fn select(app: &mut App, starts: &[i64]) {
        let key = |start: i64| if start == 0 { 60 } else { 64 };
        app.roll.selection = starts
            .iter()
            .map(|&start| (ChannelId(1), Ticks(start), key(start)))
            .collect();
    }

    /// Each step undone gives the bytes from before back; each redone, the bytes after.
    fn undo_redo_round_trip(app: &mut App, before: &str) {
        let after = document(app);
        let steps = app.session.history().undo_len();
        for _ in 0..steps {
            let _ = app.update(Message::Undo);
        }
        assert_eq!(document(app), before, "undone");
        for _ in 0..steps {
            let _ = app.update(Message::Redo);
        }
        assert_eq!(document(app), after, "redone");
    }

    /// p3_roll with the lead notes starting at `selected` selected, after `keys`: the lead
    /// notes and the undo steps taken; each step undoes and redoes byte for byte.
    fn after_keys(selected: &[i64], keys: &[Press]) -> (Vec<(i64, i64, u8)>, usize) {
        let (mut app, _player) = p3();
        let before = document(&app);
        select(&mut app, selected);
        for press in keys {
            tap(&mut app, press);
        }
        let steps = app.session.history().undo_len();
        let notes = lead(&app);
        undo_redo_round_trip(&mut app, &before);
        (notes, steps)
    }

    const A: (i64, i64, u8) = (0, 48, 60);
    const B: (i64, i64, u8) = (96, 96, 64);

    // Up a semitone; twice for both; nothing selected, nothing moves. Saturated: B lifted to
    // 124 by five octaves, then three semitones to 127, and a fourth goes no higher.
    #[test]
    fn up_moves_the_selected_notes_a_semitone() {
        assert_eq!(after_keys(&[0], &[up()]), (vec![(0, 48, 61), B], 1));
        assert_eq!(
            after_keys(&[0, 96], &[up(), up()]),
            (vec![(0, 48, 62), (96, 96, 66)], 2)
        );
        assert_eq!(after_keys(&[], &[up()]), (vec![A, B], 0));
        let mut keys = vec![shift_up(); 5];
        keys.extend(vec![up(); 4]);
        assert_eq!(after_keys(&[96], &keys), (vec![A, (96, 96, 127)], 8));
    }

    // Down a semitone, for one and for both; saturated: A taken down to key 0 by five octaves
    // goes no lower; nothing selected, nothing moves.
    #[test]
    fn down_moves_the_selected_notes_a_semitone() {
        assert_eq!(after_keys(&[0], &[down()]), (vec![(0, 48, 59), B], 1));
        assert_eq!(
            after_keys(&[0, 96], &[down()]),
            (vec![(0, 48, 59), (96, 96, 63)], 1)
        );
        let mut keys = vec![shift_down(); 5];
        keys.push(down());
        assert_eq!(after_keys(&[0], &keys), (vec![(0, 48, 0), B], 5));
        assert_eq!(after_keys(&[], &[down()]), (vec![A, B], 0));
    }

    // Shift+Up an octave; both; saturated: a sixth octave from 124 stops at 127 (the group goes
    // as far as its highest note can).
    #[test]
    fn shift_up_moves_the_selected_notes_an_octave() {
        assert_eq!(after_keys(&[0], &[shift_up()]), (vec![(0, 48, 72), B], 1));
        assert_eq!(
            after_keys(&[0, 96], &[shift_up()]),
            (vec![(0, 48, 72), (96, 96, 76)], 1)
        );
        assert_eq!(
            after_keys(&[96], &vec![shift_up(); 6]),
            (vec![A, (96, 96, 127)], 6)
        );
        assert_eq!(after_keys(&[], &[shift_up()]), (vec![A, B], 0));
    }

    // Shift+Down an octave; both; saturated: a sixth octave from key 0 does nothing.
    #[test]
    fn shift_down_moves_the_selected_notes_an_octave() {
        assert_eq!(after_keys(&[0], &[shift_down()]), (vec![(0, 48, 48), B], 1));
        assert_eq!(
            after_keys(&[0, 96], &[shift_down()]),
            (vec![(0, 48, 48), (96, 96, 52)], 1)
        );
        assert_eq!(
            after_keys(&[0], &vec![shift_down(); 6]),
            (vec![(0, 48, 0), B], 5)
        );
    }

    // Left by the snap (a step, 24 ticks): B to 72, twice to 48; saturated: with A at tick 0
    // in the selection nothing moves; nothing selected, nothing moves.
    #[test]
    fn left_moves_the_selected_notes_earlier_by_the_snap() {
        assert_eq!(after_keys(&[96], &[left()]), (vec![A, (72, 96, 64)], 1));
        assert_eq!(
            after_keys(&[96], &[left(), left()]),
            (vec![A, (48, 96, 64)], 2)
        );
        assert_eq!(after_keys(&[0, 96], &[left()]), (vec![A, B], 0));
        assert_eq!(after_keys(&[], &[left()]), (vec![A, B], 0));
    }

    // Right by the snap: A to 24; both 24 and 120; saturated: B pushed eight steps ends on the
    // pattern's end (288..384), a ninth past it grows the pattern to 2 bars in the same step.
    #[test]
    fn right_moves_the_selected_notes_later_by_the_snap() {
        assert_eq!(after_keys(&[0], &[right()]), (vec![(24, 48, 60), B], 1));
        assert_eq!(
            after_keys(&[0, 96], &[right()]),
            (vec![(24, 48, 60), (120, 96, 64)], 1)
        );
        let (mut app, _player) = p3();
        select(&mut app, &[96]);
        for _ in 0..8 {
            tap(&mut app, &right());
        }
        assert_eq!(app.session.project().patterns[&PatternId(1)].length_bars, 1);
        tap(&mut app, &right());
        assert_eq!(lead(&app), [A, (312, 96, 64)]);
        assert_eq!(app.session.project().patterns[&PatternId(1)].length_bars, 2);
        assert_eq!(app.session.history().undo_len(), 9);
    }

    // Delete: A; both; nothing selected deletes nothing.
    #[test]
    fn delete_takes_the_selected_notes_out() {
        assert_eq!(after_keys(&[0], &[del()]), (vec![B], 1));
        assert_eq!(after_keys(&[0, 96], &[del()]), (vec![], 1));
        assert_eq!(after_keys(&[], &[del()]), (vec![A, B], 0));
    }

    // Ctrl+D, duplicate: gen_p3_expected.py's known answers for the selection copied
    // one extent later (p3_d_duplicate.json), and again past the pattern's end, which grows
    // (p3_d2_duplicate_again.json); with nothing selected, the channel's notes (the same
    // file); A alone, 48 ticks long, lands at 48.
    #[test]
    fn ctrl_d_duplicates_the_selection_after_its_end() {
        let d = || ctrl("d", Code::KeyD);
        let (mut app, _player) = p3();
        let before = document(&app);
        select(&mut app, &[0, 96]);
        tap(&mut app, &d());
        assert_eq!(document(&app), fixture("p3_d_duplicate.json"));
        tap(&mut app, &d());
        assert_eq!(document(&app), fixture("p3_d2_duplicate_again.json"));
        undo_redo_round_trip(&mut app, &before);
        let (mut app, _player) = p3();
        tap(&mut app, &d());
        assert_eq!(document(&app), fixture("p3_d_duplicate.json"));
        assert_eq!(after_keys(&[0], &[d()]), (vec![A, (48, 48, 60), B], 1));
    }

    // Ctrl+C then Ctrl+V: A copied, a click on B (its tick 108 down to the step, 96) says where,
    // and A's copy lands at 96; again, after it at 144. The clipboard outlives a switch of
    // pattern and of channel: both copied, pasted into a new pattern at 0 and then onto the
    // kick channel after them, at 192. While playing, a paste lands at the playhead (200 down
    // to the step, 192). Saturated: an empty clipboard pastes nothing, a copy of nothing keeps
    // the clipboard empty, and a paste at the playhead's 336 runs past the end and grows the
    // pattern.
    #[test]
    fn ctrl_c_and_ctrl_v_copy_and_paste_notes() {
        let (c, v) = (ctrl("c", Code::KeyC), ctrl("v", Code::KeyV));
        let (mut app, _player) = p3();
        let before = document(&app);
        select(&mut app, &[0]);
        tap(&mut app, &c);
        let click_b = |app: &mut App| {
            let at = Point::new(120.0, 66.0);
            let _ = app.update(Message::Roll(RollMessage::GridPressed {
                at,
                button: mouse::Button::Left,
                double: false,
            }));
            let _ = app.update(Message::Roll(RollMessage::GridReleased(at)));
        };
        click_b(&mut app);
        tap(&mut app, &v);
        assert_eq!(lead(&app), [A, (96, 48, 60), B]);
        tap(&mut app, &v);
        assert_eq!(lead(&app), [A, (96, 48, 60), B, (144, 48, 60)]);
        let pasted = BTreeSet::from([(ChannelId(1), Ticks(144), 60)]);
        assert_eq!(app.roll.selection, pasted, "the pasted notes are selected");
        undo_redo_round_trip(&mut app, &before);

        let (mut app, _player) = p3();
        select(&mut app, &[0, 96]);
        tap(&mut app, &c);
        let _ = app.update(Message::Pattern(PatternMessage::Next));
        assert_eq!(app.pattern, Some(PatternId(2)));
        tap(&mut app, &v);
        assert_eq!(notes_of(&app, 2, 1), [A, B]);
        app.select_channel(ChannelId(2));
        tap(&mut app, &v);
        assert_eq!(notes_of(&app, 2, 2), [(192, 48, 60), (288, 96, 64)]);

        let (mut app, _player) = p3();
        select(&mut app, &[0]);
        tap(&mut app, &c);
        app.playhead = Some(Ticks(200));
        tap(&mut app, &v);
        assert_eq!(lead(&app), [A, B, (192, 48, 60)]);

        let (mut app, _player) = p3();
        tap(&mut app, &v);
        tap(&mut app, &c);
        assert_eq!((app.clipboard.is_none(), lead(&app)), (true, vec![A, B]));
        select(&mut app, &[0, 96]);
        tap(&mut app, &c);
        app.playhead = Some(Ticks(336));
        tap(&mut app, &v);
        assert_eq!(lead(&app), [A, B, (336, 48, 60), (432, 96, 64)]);
        assert_eq!(app.session.project().patterns[&PatternId(1)].length_bars, 2);
    }

    // Ctrl+X: A cut and pasted back where it was (nothing clicked: tick 0) gives p3_roll's own
    // bytes in two steps; both cut leave the channel empty; nothing selected cuts nothing.
    #[test]
    fn ctrl_x_cuts_the_selected_notes_to_the_clipboard() {
        let (x, v) = (ctrl("x", Code::KeyX), ctrl("v", Code::KeyV));
        let (mut app, _player) = p3();
        select(&mut app, &[0]);
        tap(&mut app, &x);
        assert_eq!(lead(&app), [B]);
        assert_eq!(app.clipboard.as_ref().map(|clip| clip.len()), Some(1));
        tap(&mut app, &v);
        assert_eq!(document(&app), fixture("p3_roll.json"));
        assert_eq!(app.session.history().undo_len(), 2);
        assert_eq!(after_keys(&[0, 96], std::slice::from_ref(&x)), (vec![], 1));
        assert_eq!(after_keys(&[], &[x]), (vec![A, B], 0));
    }

    // Ctrl+A: the channel's notes, never another's; the kick channel's own; an empty pattern
    // selects nothing.
    #[test]
    fn ctrl_a_selects_every_note_of_the_channel() {
        let a = ctrl("a", Code::KeyA);
        let (mut app, _player) = p3();
        tap(&mut app, &a);
        let lead_keys =
            BTreeSet::from([(ChannelId(1), Ticks(0), 60), (ChannelId(1), Ticks(96), 64)]);
        assert_eq!(app.roll.selection, lead_keys);
        app.select_channel(ChannelId(2));
        let _ = app.update(Message::TogglePanel(Panel::PianoRoll));
        let _ = app.update(Message::TogglePanel(Panel::PianoRoll));
        tap(&mut app, &a);
        let kicks = BTreeSet::from([
            (ChannelId(2), Ticks(192), 60),
            (ChannelId(2), Ticks(288), 60),
        ]);
        assert_eq!(app.roll.selection, kicks);
        let _ = app.update(Message::Pattern(PatternMessage::Next));
        tap(&mut app, &a);
        assert!(app.roll.selection.is_empty());
    }

    // With the computer keyboard on, the arrows, Delete and Ctrl+D, C, X,
    // V and A edit the roll only while it has the keys: the cursor over it with notes selected,
    // or the roll the last panel pressed in. Otherwise they do what they did: the piano row
    // leaves the arrows to the Library's walk and Delete to the panel with the keys (none here),
    // the two rows swallow them; Ctrl chords never play notes. The keys around them keep their
    // work while the roll has the keys: Enter, the digits and function keys, and the piano
    // row's Backspace, Tab, Z, X, C and V.
    #[test]
    fn the_rolls_keys_edit_it_only_while_it_has_the_keys() {
        let roll_keys = [
            up(),
            del(),
            ctrl("d", Code::KeyD),
            ctrl("c", Code::KeyC),
            ctrl("x", Code::KeyX),
            ctrl("v", Code::KeyV),
            ctrl("a", Code::KeyA),
        ];
        for layout in [NoteLayout::Piano, NoteLayout::TwoRows] {
            let (mut app, _player) = p3();
            app.typing.layout = layout;
            app.clipboard = None;
            select(&mut app, &[0]);
            app.focus = Panel::Rack;
            assert!(!app.roll_has_keys(), "{layout:?}");
            let before = document(&app);
            for press in &roll_keys {
                tap(&mut app, press);
            }
            assert_eq!(document(&app), before, "{layout:?}: nothing edited");
            assert!(app.clipboard.is_none(), "{layout:?}: nothing copied");
            let arrow = asked(&mut app, &up());
            match layout {
                NoteLayout::Piano => assert!(
                    matches!(arrow, Some(Message::Browser(_))),
                    "the Library's walk: {arrow:?}"
                ),
                NoteLayout::TwoRows => assert!(
                    matches!(
                        arrow,
                        Some(Message::TypingKey {
                            code: Code::ArrowUp,
                            ..
                        })
                    ),
                    "swallowed: {arrow:?}"
                ),
            }
            // The cursor over the roll with nothing selected: still not the roll's.
            app.roll.selection.clear();
            let over = Hover::Roll(RollPart::Empty { key: 60 });
            let _ = app.update(Message::Hover(HoverMessage::Over(over.clone())));
            assert!(!app.roll_has_keys(), "{layout:?}");
            // Over the roll with A selected: Up moves it, Delete takes it out.
            select(&mut app, &[0]);
            assert!(app.roll_has_keys(), "{layout:?}");
            let asked_up = asked(&mut app, &up());
            assert!(
                matches!(asked_up, Some(Message::RollKey(NoteEdit::Transpose(1)))),
                "{layout:?}: {asked_up:?}"
            );
            tap(&mut app, &up());
            assert_eq!(lead(&app), [(0, 48, 61), B], "{layout:?}");
            tap(&mut app, &del());
            assert_eq!(lead(&app), [B], "{layout:?}");
            // The cursor gone, the roll the last pressed in: its keys again.
            let _ = app.update(Message::Hover(HoverMessage::Left(over)));
            app.focus = Panel::PianoRoll;
            tap(&mut app, &ctrl("a", Code::KeyA));
            tap(&mut app, &ctrl("c", Code::KeyC));
            assert_eq!(app.clipboard.as_ref().map(|clip| clip.len()), Some(1));
        }
        // The piano row's own keys and the window's keep their work while the roll has them.
        let (mut app, _player) = p3();
        select(&mut app, &[0]);
        let typed = |code| Some(code);
        for (press, code) in [
            (
                Press(Key::Character("a".into()), Code::KeyA, Modifiers::empty()),
                Code::KeyA,
            ),
            (
                named(Named::Backspace, Code::Backspace, Modifiers::empty()),
                Code::Backspace,
            ),
            (named(Named::Tab, Code::Tab, Modifiers::empty()), Code::Tab),
            (
                Press(Key::Character("z".into()), Code::KeyZ, Modifiers::empty()),
                Code::KeyZ,
            ),
            (
                Press(Key::Character("x".into()), Code::KeyX, Modifiers::empty()),
                Code::KeyX,
            ),
            (
                Press(Key::Character("c".into()), Code::KeyC, Modifiers::empty()),
                Code::KeyC,
            ),
            (
                Press(Key::Character("v".into()), Code::KeyV, Modifiers::empty()),
                Code::KeyV,
            ),
        ] {
            let message = asked(&mut app, &press);
            assert!(
                matches!(message, Some(Message::TypingKey { code: played, .. }) if typed(played) == typed(code)),
                "{code:?}: {message:?}"
            );
        }
        assert!(matches!(
            asked(
                &mut app,
                &named(Named::Enter, Code::Enter, Modifiers::empty())
            ),
            Some(Message::OpenSelectedChannel)
        ));
        assert!(matches!(
            asked(
                &mut app,
                &Press(Key::Character("1".into()), Code::Digit1, Modifiers::empty())
            ),
            Some(Message::TogglePanel(Panel::Rack))
        ));
        assert!(matches!(
            asked(&mut app, &named(Named::F5, Code::F5, Modifiers::empty())),
            Some(Message::TogglePanel(Panel::Playlist))
        ));
    }

    // ---------------------------------------------------------------------------------------
    // Gestures through the real widgets (app/widget_events.rs): the piano roll alone under the
    // toolbar of a 1000 x 560 window, the hint bar off, so grid point (x, y) is window point
    // (x, y + toolbar + strip). On p3_roll's roll (top key 69, 16 px a step): A spans x 48..80
    // on key 60's row (y 108..120), B x 112..176 on key 64's (y 60..72).
    // ---------------------------------------------------------------------------------------

    fn roll_alone() -> (App, Player) {
        let (mut app, player) = p3();
        let _ = app.update(Message::TogglePanel(Panel::Rack));
        let _ = app.update(Message::WindowResized(Size::new(1000.0, 560.0)));
        app.hints_flag = Some(false);
        assert!(app.panels.piano_roll && !app.panels.rack && !app.panels.browser);
        (app, player)
    }

    fn grid(x: f32, y: f32) -> Point {
        Point::new(x, y + TOOLBAR_HEIGHT + STRIP_HEIGHT)
    }

    fn keys_of(starts: &[i64]) -> BTreeSet<(ChannelId, Ticks, u8)> {
        starts
            .iter()
            .map(|&start| (ChannelId(1), Ticks(start), if start == 0 { 60 } else { 64 }))
            .collect()
    }

    // A drag from empty grid at (200, 50) to (60, 130) draws a box meeting A and B: both
    // selected, nothing drawn. A click on B (no drag) selects B alone. With Shift, a box over A
    // alone adds it to B.
    #[test]
    fn a_drag_from_empty_grid_selects_what_its_box_meets() {
        let (mut app, _player) = roll_alone();
        let before = document(&app);
        let mut window = Window::new(&app);
        window.drag(&mut app, grid(200.0, 50.0), grid(60.0, 130.0));
        assert_eq!(app.roll.selection, keys_of(&[0, 96]));
        assert_eq!(document(&app), before, "a box draws no note");
        window.click(&mut app, grid(120.0, 66.0));
        assert_eq!(app.roll.selection, keys_of(&[96]));
        window.hold(&mut app, Modifiers::SHIFT);
        window.drag(&mut app, grid(100.0, 100.0), grid(40.0, 125.0));
        window.hold(&mut app, Modifiers::empty());
        assert_eq!(app.roll.selection, keys_of(&[0, 96]), "Shift adds");
        assert_eq!(document(&app), before);
    }

    // A click selects A alone; Shift+click on B adds it, Shift+click on A takes it out.
    #[test]
    fn shift_click_adds_a_note_to_the_selection_or_takes_it_out() {
        let (mut app, _player) = roll_alone();
        let mut window = Window::new(&app);
        window.click(&mut app, grid(56.0, 114.0));
        assert_eq!(app.roll.selection, keys_of(&[0]));
        window.hold(&mut app, Modifiers::SHIFT);
        window.click(&mut app, grid(120.0, 66.0));
        assert_eq!(app.roll.selection, keys_of(&[0, 96]));
        window.click(&mut app, grid(56.0, 114.0));
        assert_eq!(app.roll.selection, keys_of(&[96]));
        window.hold(&mut app, Modifiers::empty());
        assert_eq!(
            app.session.history().undo_len(),
            0,
            "selecting edits nothing"
        );
    }

    // Both selected by a box, A dragged from x 56 (tick 12) to 80 (tick 48): 36 ticks, nearest
    // step 48, so A to 48 and B to 144 together, one undo step.
    #[test]
    fn a_drag_on_a_selected_note_moves_the_whole_selection() {
        let (mut app, _player) = roll_alone();
        let before = document(&app);
        let mut window = Window::new(&app);
        window.drag(&mut app, grid(200.0, 50.0), grid(60.0, 130.0));
        window.drag(&mut app, grid(56.0, 114.0), grid(80.0, 114.0));
        assert_eq!(lead(&app), [(48, 48, 60), (144, 96, 64)]);
        assert_eq!(app.session.history().undo_len(), 1);
        undo_redo_round_trip(&mut app, &before);
    }

    // Two quick clicks on A delete it, one step. Two quick clicks on empty grid at x 250 (tick
    // 303, down to 288) on key 60's row draw one note there, as long as A (the click on it
    // made its 48 ticks the draw length): the second click lands on the note the first drew
    // and leaves it.
    #[test]
    fn a_double_click_deletes_a_note_and_on_empty_grid_draws_one() {
        let (mut app, _player) = roll_alone();
        let before = document(&app);
        let mut window = Window::new(&app);
        window.click(&mut app, grid(56.0, 114.0));
        window.click(&mut app, grid(56.0, 114.0));
        assert_eq!(lead(&app), [B]);
        assert_eq!(app.session.history().undo_len(), 1);
        window.click(&mut app, grid(250.0, 114.0));
        window.click(&mut app, grid(250.0, 114.0));
        assert_eq!(lead(&app), [B, (288, 48, 60)]);
        assert_eq!(app.session.history().undo_len(), 2);
        undo_redo_round_trip(&mut app, &before);
    }

    // Gate 2026-10-25: on a loaded machine the two clicks above came more than the double-press
    // time apart by the wall clock, and the second drew a note instead of deleting one. The
    // test window holds the canvases' clock, so even half a second between the clicks is a
    // double-click.
    #[test]
    fn a_double_click_is_one_however_slowly_the_machine_sends_it() {
        let (mut app, _player) = roll_alone();
        let mut window = Window::new(&app);
        window.click(&mut app, grid(56.0, 114.0));
        std::thread::sleep(std::time::Duration::from_millis(500));
        window.click(&mut app, grid(56.0, 114.0));
        assert_eq!(lead(&app), [B], "deleted, not drawn beside");
        assert_eq!(app.session.history().undo_len(), 1);
    }

    // ---------------------------------------------------------------------------------------
    // The notes menu, the strip's Notes button and the Quantize dialog (A5). With nothing
    // selected an operation acts on every note of the channel; with a selection, on it.
    // ---------------------------------------------------------------------------------------

    fn menu(app: &mut App, edit: NoteEdit) {
        let _ = app.update(Message::NoteEdit(edit));
    }

    // Nothing selected: Reverse mirrors A and B in their span (B to 0..96, A to 144..192);
    // Invert swaps their keys; Legato makes A end where B starts (0..96). A selected alone:
    // Chop cuts it at the step (0..24, 24..48) and Glue joins it again, p3_roll's own bytes.
    #[test]
    fn the_menus_operations_act_on_the_selection_or_on_every_note() {
        let (mut app, _player) = p3();
        let before = document(&app);
        menu(&mut app, NoteEdit::Reverse);
        assert_eq!(lead(&app), [(0, 96, 64), (144, 48, 60)]);
        undo_redo_round_trip(&mut app, &before);
        let (mut app, _player) = p3();
        menu(&mut app, NoteEdit::Invert);
        assert_eq!(lead(&app), [(0, 48, 64), (96, 96, 60)]);
        undo_redo_round_trip(&mut app, &before);
        let (mut app, _player) = p3();
        menu(&mut app, NoteEdit::Legato);
        assert_eq!(lead(&app), [(0, 96, 60), B]);
        undo_redo_round_trip(&mut app, &before);
        let (mut app, _player) = p3();
        select(&mut app, &[0]);
        menu(&mut app, NoteEdit::Chop);
        assert_eq!(lead(&app), [(0, 24, 60), (24, 24, 60), B]);
        let pieces = BTreeSet::from([(ChannelId(1), Ticks(0), 60), (ChannelId(1), Ticks(24), 60)]);
        assert_eq!(app.roll.selection, pieces, "the pieces stay selected");
        menu(&mut app, NoteEdit::Glue);
        assert_eq!(document(&app), before, "glued back to p3_roll");
        assert_eq!(app.session.history().undo_len(), 2);
        undo_redo_round_trip(&mut app, &before);
    }

    // Humanize draws from its own count: the first run (seed 0) moves A −4 (stopped at 0) and
    // +2 velocity steps (100 → 102), B +1 (→ 97) and −4 (64 → 60), as the SplitMix64 oracle
    // in note_ops' tests gives; a second run (seed 1) differs.
    #[test]
    fn humanize_moves_notes_by_its_count() {
        let (mut app, _player) = p3();
        let before = document(&app);
        menu(&mut app, NoteEdit::Humanize);
        let velocities = |app: &App| -> Vec<(i64, f32)> {
            app.session.project().patterns[&PatternId(1)]
                .notes
                .iter()
                .filter(|note| note.channel == ChannelId(1))
                .map(|note| (note.start.0, note.velocity))
                .collect()
        };
        assert_eq!(velocities(&app), [(0, 102.0 / 128.0), (97, 60.0 / 128.0)]);
        let first = document(&app);
        menu(&mut app, NoteEdit::Humanize);
        assert_ne!(document(&app), first, "the next run draws other numbers");
        undo_redo_round_trip(&mut app, &before);
    }

    // The Quantize dialog: opened from the menu with the last choice (the snap, 100 %, starts),
    // set to 1/8 (48 ticks) at 50 %: a note of key 62 at 30 goes half way to 48, 39. Opened
    // again it remembers 1/8 and 50 %; at 1/4 and 100 % the note at 39 goes to 0. Cancel and
    // Esc close it unapplied.
    #[test]
    fn the_quantize_dialog_moves_notes_toward_the_grid_it_chooses() {
        use dawsome_engine::edit::Edit;
        use dawsome_engine::project::Note;
        let (mut app, _player) = p3();
        app.commit(Edit::ReplaceNotes {
            pattern: PatternId(1),
            remove: vec![],
            add: vec![Note {
                channel: ChannelId(1),
                start: Ticks(30),
                length: Ticks(24),
                velocity: 0.5,
                key: 62,
            }],
        });
        select(&mut app, &[]);
        app.roll.selection = BTreeSet::from([(ChannelId(1), Ticks(30), 62)]);
        let send = |app: &mut App, message| {
            let _ = app.update(Message::Quantize(message));
        };
        send(&mut app, QuantizeMessage::Open);
        assert_eq!(app.quantize_form, Some(app.roll.quantize));
        send(&mut app, QuantizeMessage::Grid(QuantizeGrid::Note(8)));
        send(&mut app, QuantizeMessage::Strength(50));
        send(&mut app, QuantizeMessage::Apply);
        assert_eq!(app.quantize_form, None);
        assert_eq!(lead(&app), [A, (39, 24, 62), B]);
        send(&mut app, QuantizeMessage::Open);
        let form = app.quantize_form.expect("open");
        assert_eq!(
            (form.grid, form.strength, form.ends),
            (QuantizeGrid::Note(8), 50, false)
        );
        send(&mut app, QuantizeMessage::Cancel);
        assert_eq!(app.quantize_form, None);
        send(&mut app, QuantizeMessage::Open);
        let _ = app.update(Message::ClearSelection);
        assert_eq!(app.quantize_form, None, "Esc closes it");
        send(&mut app, QuantizeMessage::Open);
        send(&mut app, QuantizeMessage::Grid(QuantizeGrid::Note(4)));
        send(&mut app, QuantizeMessage::Strength(100));
        send(&mut app, QuantizeMessage::Apply);
        assert_eq!(lead(&app), [A, (0, 24, 62), B]);
    }

    // The menu's rows through the real overlay: a right press on B opens the notes menu on the
    // window's top layer, inside the window; its rows are NoteRow::ALL's, a ROW_HEIGHT each,
    // and a click on Delete (the fifth) takes B out. Cut, Copy and Delete are dim with
    // nothing selected, Paste with nothing copied. The strip's Notes button opens the menu
    // under the strip's right end, and a second press closes it.
    #[test]
    fn the_notes_menu_opens_on_the_overlay_and_its_rows_act() {
        use crate::ui::hint::NoteRow;
        use crate::ui::menu;
        use crate::ui::piano_roll::note_row_message;
        let (mut app, _player) = roll_alone();
        let _ = app.update(Message::Roll(RollMessage::GridPressed {
            at: Point::new(120.0, 66.0),
            button: mouse::Button::Right,
            double: false,
        }));
        let mut window = Window::new(&app);
        let menus = window.overlays(&app);
        let [shown] = menus[..] else {
            panic!("one menu: {menus:?}");
        };
        let rows = NoteRow::ALL.len() as f32;
        assert_eq!(shown.height, rows * menu::ROW_HEIGHT + 2.0 * menu::PADDING);
        assert!(shown.x + shown.width <= 1000.0 && shown.y + shown.height <= 560.0);
        let delete = NoteRow::ALL
            .iter()
            .position(|&row| row == NoteRow::Delete)
            .expect("listed") as f32;
        let at = Point::new(
            shown.x + 20.0,
            shown.y + menu::PADDING + menu::ROW_HEIGHT * (delete + 0.5),
        );
        let published = window.click_overlay(&app, at);
        for message in published {
            let _ = app.update(message);
        }
        assert_eq!(lead(&app), [A]);
        assert_eq!(app.roll.menu, None);
        for row in [NoteRow::Cut, NoteRow::Copy, NoteRow::Delete, NoteRow::Paste] {
            assert!(note_row_message(row, false, false).is_none(), "{row:?}");
        }
        assert!(note_row_message(NoteRow::Paste, false, true).is_some());
        assert!(note_row_message(NoteRow::Legato, false, false).is_some());
        let _ = app.update(Message::Roll(RollMessage::MenuButton));
        let opened = app.roll.menu.expect("opened by the button");
        assert!(opened.y == 0.0 && opened.x > 700.0, "{opened:?}");
        let mut window = Window::new(&app);
        let [shown] = window.overlays(&app)[..] else {
            panic!("one menu");
        };
        assert!(shown.x + shown.width <= 1000.0, "{shown:?}");
        let _ = app.update(Message::Roll(RollMessage::MenuButton));
        assert_eq!(app.roll.menu, None, "pressed again, it closes");
    }

    /// Lane point (x, y) in the window: the lane (64 px) runs down to the window's foot.
    fn lane(x: f32, y: f32) -> Point {
        Point::new(x, 560.0 - crate::ui::piano_layout::LANE_HEIGHT + y)
    }

    fn lead_velocities(app: &App) -> Vec<f32> {
        app.session.project().patterns[&PatternId(1)]
            .notes
            .iter()
            .filter(|note| note.channel == ChannelId(1))
            .map(|note| note.velocity)
            .collect()
    }

    // Through the real lane: A's stem's middle is x 50, B's x 114; on the 64 px lane, y gives
    // round((64 − y)·2) steps. A drag from (50, 60) across to (114, 4) paints A 8 and B 120,
    // one step, both selected after. Both selected, a drag on A's stem from y 32 (64) to y 16
    // sets both to 96. Alt+drag from (50, 63.8) (0.4 of a step, rounded to 0; the lane's
    // last pixel row is y 63) to (114, 0) draws a ramp from 0 at A's tick to 128 at B's: A 0,
    // B 128.
    #[test]
    fn velocity_paints_across_stems_drags_a_selection_and_ramps_with_alt() {
        let (mut app, _player) = roll_alone();
        let before = document(&app);
        let mut window = Window::new(&app);
        window.drag(&mut app, lane(50.0, 60.0), lane(114.0, 4.0));
        assert_eq!(lead_velocities(&app), [8.0 / 128.0, 120.0 / 128.0]);
        assert_eq!(app.roll.selection, keys_of(&[0, 96]));
        assert_eq!(app.session.history().undo_len(), 1);
        window.drag(&mut app, lane(50.0, 32.0), lane(50.0, 16.0));
        assert_eq!(lead_velocities(&app), [0.75, 0.75]);
        window.hold(&mut app, Modifiers::ALT);
        window.drag(&mut app, lane(50.0, 63.8), lane(114.0, 0.0));
        window.hold(&mut app, Modifiers::empty());
        assert_eq!(lead_velocities(&app), [0.0, 1.0]);
        assert_eq!(app.session.history().undo_len(), 3);
        undo_redo_round_trip(&mut app, &before);
    }

    // ---------------------------------------------------------------------------------------
    // Live notes on the roll (A1), fed through the headless door as the recording tests feed
    // them: a MIDI device's bytes into the session's inbox, the player's calls, the window's
    // playhead timer reading what the player posted.
    // ---------------------------------------------------------------------------------------

    /// `calls` calls of the player, 512 frames each, the window's timer after each.
    fn run(app: &mut App, player: &mut Player, calls: usize) {
        for _ in 0..calls {
            player.process(&mut [[0.0; 2]; 512]);
            let _ = app.update(Message::Tick);
        }
    }

    /// The keys the roll lights on `channel`.
    fn lit(app: &App, channel: u32) -> Vec<u8> {
        app.live_notes
            .of(ChannelId(channel))
            .into_iter()
            .map(|(key, _)| key)
            .collect()
    }

    // Stopped: a device's D5 (62) held lights key 62 on the lead's roll and marks the live
    // strip (x 48, 6 px, key 62's row y 84..95); let go of, it goes. Playing p3_roll (44.1 kHz,
    // 120 BPM: 229.6875 samples a tick), a note heard from call 20 (sample 10240) started at
    // tick 44, so its bar starts at x 48 + 44/1.5 and grows to the playhead.
    #[test]
    fn live_notes_light_their_keys_and_grow_bars_on_the_roll() {
        use crate::midi::{ControllerQueue, Router};
        use crate::ui::live_notes::{bars, Held};
        use iced::Rectangle;
        let (mut app, mut player) = p3();
        let device = Router::new(app.session.midi_inbox(), ControllerQueue::default());
        device.deliver(&[0x90, 62, 100]);
        run(&mut app, &mut player, 1);
        assert_eq!(lit(&app, 1), [62]);
        let [(_, Held { tick, .. })] = app.live_notes.of(ChannelId(1))[..] else {
            panic!("one note held");
        };
        assert_eq!(tick, None, "the song is stopped");
        assert_eq!(
            bars(&app.roll.view, Ticks(384), app.playhead, 62, tick),
            [Rectangle::new(Point::new(48.0, 84.0), Size::new(6.0, 11.0))]
        );
        assert!(app.live_notes_due(), "the timer runs while a note is held");
        device.deliver(&[0x80, 62, 0]);
        run(&mut app, &mut player, 1);
        assert!(lit(&app, 1).is_empty());

        let _ = app.update(Message::Play);
        run(&mut app, &mut player, 20);
        device.deliver(&[0x90, 64, 90]);
        run(&mut app, &mut player, 10);
        let [(64, Held { tick, .. })] = app.live_notes.of(ChannelId(1))[..] else {
            panic!("E5 held: {:?}", app.live_notes);
        };
        assert_eq!(tick, Some(Ticks(44)));
        let playhead = app.playhead.expect("playing");
        let shown = bars(&app.roll.view, Ticks(384), Some(playhead), 64, tick);
        let [bar] = shown[..] else {
            panic!("one bar: {shown:?}");
        };
        assert_eq!(bar.x, 48.0 + 44.0 / 1.5);
        assert!(
            bar.x + bar.width > bar.x + 2.0,
            "it grew toward the playhead"
        );
        device.deliver(&[0x80, 64, 0]);
        run(&mut app, &mut player, 1);
        assert!(lit(&app, 1).is_empty());
        let _ = app.update(Message::Stop);
    }

    // A device's sustain pedal holds a note's bar on the roll past its note-off, until the pedal
    // lifts (the roll reads the player's note events, which end the note there); a pedal linked to
    // a control holds nothing.
    #[test]
    fn a_pedal_holds_a_notes_bar_until_it_lifts() {
        use crate::midi::{ControllerQueue, Router};
        let (mut app, mut player) = p3();
        let device = Router::new(app.session.midi_inbox(), ControllerQueue::default());
        device.deliver(&[0xB0, 64, 127]);
        device.deliver(&[0x90, 62, 100]);
        run(&mut app, &mut player, 1);
        device.deliver(&[0x80, 62, 0]);
        run(&mut app, &mut player, 3);
        assert_eq!(lit(&app, 1), [62], "held by the pedal");
        device.deliver(&[0xB0, 64, 0]);
        run(&mut app, &mut player, 1);
        assert!(lit(&app, 1).is_empty(), "the lift ends it");
        device.links(crate::midi::linked_controls(&{
            let mut project = app.session.project().clone();
            project.midi_links.insert(
                dawsome_engine::project::AutomationTarget::ChannelPan(ChannelId(1)),
                dawsome_engine::project::Controller {
                    cc: 64,
                    channel: None,
                },
            );
            project
        }));
        device.deliver(&[0xB0, 64, 127]);
        device.deliver(&[0x90, 62, 100]);
        run(&mut app, &mut player, 1);
        device.deliver(&[0x80, 62, 0]);
        run(&mut app, &mut player, 1);
        assert!(lit(&app, 1).is_empty(), "a linked pedal holds nothing");
    }

    // The typing keyboard's A (C5, 60) shows on the selected channel's roll the same way, and
    // a note played on an external input channel (p16_unity's Oriole, selected, which plays no
    // sound of its own) draws on its roll all the same.
    #[test]
    fn typed_notes_and_notes_on_an_input_channel_show_on_the_roll() {
        let (mut app, mut player) = p3();
        let a = Press(Key::Character("a".into()), Code::KeyA, Modifiers::empty());
        let Press(key, code, modifiers) = a;
        let _ = app.update(Message::Key(KeyInput {
            window: window::Id::unique(),
            key,
            code: Some(code),
            modifiers,
            pressed: true,
            repeat: false,
        }));
        assert!(app.live_notes_due(), "a key wakes the timer");
        run(&mut app, &mut player, 1);
        assert_eq!(lit(&app, 1), [60]);

        use crate::midi::{ControllerQueue, Router};
        let (mut app, mut player) = app_on(&Path::new(FIXTURES).join("p16_unity.json"));
        let _ = app.update(Message::TogglePanel(Panel::PianoRoll));
        assert_eq!(app.selected_channel, Some(ChannelId(1)));
        let device = Router::new(app.session.midi_inbox(), ControllerQueue::default());
        device.deliver(&[0x90, 67, 100]);
        run(&mut app, &mut player, 1);
        assert_eq!(lit(&app, 1), [67]);
    }

    // ---------------------------------------------------------------------------------------
    // The piano roll's edge cases.
    // ---------------------------------------------------------------------------------------

    /// p3_roll with the roll closed: a device holds MIDI key 50, taps key 60 `taps` times and
    /// lets 50 go, the player called after each message and, when `window` says, the window's
    /// activity message after each call; then the roll opens and the timer runs 3 times.
    fn taps_with_the_roll_closed(taps: usize, window: bool) -> (App, Player) {
        use crate::midi::{ControllerQueue, Router};
        let (mut app, mut player) = p3();
        let _ = app.update(Message::TogglePanel(Panel::PianoRoll));
        assert!(!app.panels.piano_roll);
        let device = Router::new(app.session.midi_inbox(), ControllerQueue::default());
        let mut send = |app: &mut App, bytes: [u8; 3]| {
            device.deliver(&bytes);
            player.process(&mut [[0.0; 2]; 512]);
            if window {
                let _ = app.update(Message::MidiActivity);
            }
        };
        send(&mut app, [0x90, 50, 100]);
        for _ in 0..taps {
            send(&mut app, [0x90, 60, 100]);
            send(&mut app, [0x80, 60, 0]);
        }
        send(&mut app, [0x80, 50, 0]);
        let _ = app.update(Message::TogglePanel(Panel::PianoRoll));
        assert!(app.panels.piano_roll);
        run(&mut app, &mut player, 3);
        (app, player)
    }

    // 401 events with the roll closed, more than the player's
    // 256-event ring holds, so the last ends are lost; before the fix keys 50 and 60 stayed
    // lit and the timer ran for good once the roll opened. A lost event clears the live set:
    // none lit, no timer. The control, 50 taps (102 events), is heard whole: none lit either.
    #[test]
    fn notes_lost_with_the_roll_closed_leave_no_key_lit() {
        for taps in [200, 50] {
            let (mut app, mut player) = taps_with_the_roll_closed(taps, false);
            assert!(lit(&app, 1).is_empty(), "{taps} taps: {:?}", lit(&app, 1));
            assert!(!app.live_notes_due(), "{taps} taps: the timer stops");
            run(&mut app, &mut player, 100);
            assert!(lit(&app, 1).is_empty() && !app.live_notes_due());
        }
        let (app, _player) = taps_with_the_roll_closed(200, false);
        assert!(app.session.lost_live_events() > 0, "the ring overflowed");
    }

    // The window's path: each port message reads what the player posted, whether
    // the roll is open or not, so 200 taps never overflow the ring and key 50, held, shows as
    // soon as its message comes.
    #[test]
    fn each_port_message_reads_the_players_notes() {
        use crate::midi::{ControllerQueue, Router};
        let (mut app, mut player) = p3();
        let _ = app.update(Message::TogglePanel(Panel::PianoRoll));
        let device = Router::new(app.session.midi_inbox(), ControllerQueue::default());
        device.deliver(&[0x90, 50, 100]);
        player.process(&mut [[0.0; 2]; 512]);
        let _ = app.update(Message::MidiActivity);
        assert_eq!(
            lit(&app, 1),
            [50],
            "read on the port's message, the roll closed"
        );
        let (mut app, _player) = taps_with_the_roll_closed(200, true);
        assert_eq!(app.session.lost_live_events(), 0, "drained as they came");
        assert!(lit(&app, 1).is_empty());
        assert!(
            app.live_notes_due(),
            "a port message just came: the timer runs on a while"
        );
        app.live_wake = app
            .live_wake
            .map(|when| when - std::time::Duration::from_millis(600));
        assert!(!app.live_notes_due(), "and then stops");
    }

    // The cursor over the roll's grid with A selected gives
    // the roll the keys though the Library was pressed last; the window's cursor leaving
    // (iced sends only CursorLeft) used to leave that hover behind, so Up still moved A. Now
    // the grid and the lane let go of their hover: Up is the Library's walk again.
    #[test]
    fn the_roll_lets_go_of_the_keys_when_the_cursor_leaves_the_window() {
        let (mut app, _player) = roll_alone();
        let mut window = Window::new(&app);
        for over in [grid(300.0, 30.0), lane(50.0, 30.0)] {
            window.mouse(
                &mut app,
                mouse::Event::CursorMoved { position: over },
                mouse::Cursor::Available(over),
            );
            assert!(
                matches!(app.hover, Some(Hover::Roll(_))),
                "{over:?}: {:?}",
                app.hover
            );
            select(&mut app, &[0]);
            app.focus = Panel::Browser;
            assert!(app.roll_has_keys());
            window.mouse(
                &mut app,
                mouse::Event::CursorLeft,
                mouse::Cursor::Unavailable,
            );
            assert_eq!(app.hover, None, "{over:?}");
            assert!(!app.roll_has_keys());
            let asked_up = asked(&mut app, &up());
            assert!(
                !matches!(asked_up, Some(Message::RollKey(_))),
                "{asked_up:?}"
            );
        }
    }

    // With a text box open (the tempo's, a channel's name being
    // typed) and the roll holding the keys, Up and Delete edited notes and closed the box
    // unapplied. Now no roll edit runs while a box is open, in either layout: the note stays,
    // the box stays open.
    #[test]
    fn an_open_text_box_keeps_the_rolls_keys_from_editing_notes() {
        for layout in [NoteLayout::TwoRows, NoteLayout::Piano] {
            for open_box in ["tempo", "rename"] {
                let (mut app, _player) = p3();
                app.typing.layout = layout;
                select(&mut app, &[0]);
                assert!(app.roll_has_keys());
                match open_box {
                    "tempo" => app.tempo_edit = Some("120".to_owned()),
                    _ => app.channel_rename = Some((ChannelId(1), "lea".to_owned())),
                }
                for press in [up(), del(), ctrl("d", Code::KeyD), ctrl("v", Code::KeyV)] {
                    let message = asked(&mut app, &press);
                    assert!(
                        message.as_ref().is_none_or(|message| !matches!(
                            message,
                            Message::RollKey(_) | Message::Delete | Message::Duplicate
                        )),
                        "{layout:?}, {open_box}: {message:?}"
                    );
                    tap(&mut app, &press);
                }
                assert_eq!(lead(&app), [A, B], "{layout:?}, {open_box}");
                let still_open = app.tempo_edit.is_some() || app.channel_rename.is_some();
                assert!(still_open, "{layout:?}, {open_box}: the box stays open");
            }
        }
    }

    // The Library's search box: typed into (it takes the letters itself), it lets
    // Up through, which then moved the selected note while the cursor was over the roll; now
    // it edits nothing until a press lands elsewhere (here on the roll's grid).
    #[test]
    fn typing_in_the_library_search_keeps_the_rolls_keys_from_editing_notes() {
        use crate::ui::browser::SEARCH_INPUT;
        let (mut app, _player) = roll_alone();
        let _ = app.update(Message::TogglePanel(Panel::Browser));
        let mut window = Window::new(&app);
        window.focus(&app, SEARCH_INPUT);
        window.type_keys(&mut app, &[('k', Code::KeyK)]);
        assert_eq!(app.browse.model.search, "k", "the box took the letter");
        select(&mut app, &[0]);
        let over = Hover::Roll(RollPart::Note {
            key: 60,
            velocity: 100,
            edge: false,
        });
        let _ = app.update(Message::Hover(HoverMessage::Over(over)));
        assert!(app.roll_has_keys());
        window.press_named(&mut app, Named::ArrowUp, Code::ArrowUp);
        assert_eq!(
            lead(&app),
            [A, B],
            "Up edits nothing while the search has the keys"
        );
        let _ = app.update(Message::Roll(RollMessage::GridPressed {
            at: Point::new(56.0, 114.0),
            button: mouse::Button::Left,
            double: false,
        }));
        let _ = app.update(Message::Roll(RollMessage::GridReleased(Point::new(
            56.0, 114.0,
        ))));
        tap(&mut app, &up());
        assert_eq!(
            lead(&app),
            [(0, 48, 61), B],
            "a press on the grid gave the roll its keys"
        );
    }

    // A paste lands where the grid was last clicked only in the pattern clicked
    // in. A click at tick 700 in a 2-bar pattern, then Ctrl+V on a 1-bar pattern: before, the
    // copy landed at 700, past the end, and grew the pattern; now it lands at the pattern's
    // start. Pasting again in that pattern lays the copies end to end as before.
    #[test]
    fn a_paste_after_a_pattern_change_lands_at_the_patterns_start() {
        use dawsome_engine::edit::Edit;
        let (mut app, _player) = p3();
        select(&mut app, &[0]);
        tap(&mut app, &ctrl("c", Code::KeyC));
        let _ = app.update(Message::Pattern(PatternMessage::Next));
        let two = app.pattern.expect("pattern 2");
        app.commit(Edit::SetPatternBars {
            pattern: two,
            bars: 2,
        });
        // Tick 700 is x 48 + 700/1.5 at the roll's zoom: the click lands on empty grid, which
        // draws a note there on release; that note is not the point, the click's tick is.
        let x = 48.0 + 700.0 / 1.5;
        let _ = app.update(Message::Roll(RollMessage::GridPressed {
            at: Point::new(x, 150.0),
            button: mouse::Button::Left,
            double: false,
        }));
        assert_eq!(app.roll.paste_at(two), Ticks(696), "700 down to the step");
        let _ = app.update(Message::Roll(RollMessage::GridReleased(Point::new(
            x, 150.0,
        ))));
        let _ = app.update(Message::PatternSelected(PatternId(1)));
        tap(&mut app, &ctrl("v", Code::KeyV));
        assert_eq!(app.session.project().patterns[&PatternId(1)].length_bars, 1);
        assert_eq!(lead(&app), [A, B], "A's copy on A itself: nothing changes");
        let _ = app.update(Message::PatternSelected(two));
        tap(&mut app, &ctrl("v", Code::KeyV));
        assert!(
            notes_of(&app, two.0, 1).contains(&(696, 48, 60)),
            "at the click in its pattern"
        );
    }

    /// The roll alone with the Library open, "k" typed into its search box through the real
    /// widgets, A selected and the roll the last panel pressed in.
    fn typed_into_the_search() -> (App, Player, Window) {
        use crate::ui::browser::SEARCH_INPUT;
        let (mut app, player) = roll_alone();
        let _ = app.update(Message::TogglePanel(Panel::Browser));
        let mut window = Window::new(&app);
        window.focus(&app, SEARCH_INPUT);
        window.type_keys(&mut app, &[('k', Code::KeyK)]);
        assert_eq!(app.browse.model.search, "k");
        select(&mut app, &[0]);
        app.focus = Panel::PianoRoll;
        (app, player, window)
    }

    // Once the search box lets go of the keys
    // (a press elsewhere, here the roll's Notes button twice; or Esc, which closes the
    // instrument window first), the roll's keys work again at once. Before, the flag kept by
    // a list of presses missed both and Up did nothing until a press on the grid.
    #[test]
    fn the_rolls_keys_come_back_once_the_search_box_lets_go() {
        let (mut app, _player, _window) = typed_into_the_search();
        for _ in 0..2 {
            let _ = app.update(Message::Roll(RollMessage::MenuButton));
        }
        tap(&mut app, &up());
        assert_eq!(lead(&app), [(0, 48, 61), B], "after the Notes button");

        let (mut app, _player, mut window) = typed_into_the_search();
        app.open_instrument(ChannelId(1));
        assert!(app.instrument_on().is_some());
        window.type_keys(&mut app, &[('i', Code::KeyI)]);
        let _ = app.update(Message::Escape(window::Id::unique()));
        assert!(app.instrument_on().is_none(), "Esc closed the window first");
        tap(&mut app, &up());
        assert_eq!(lead(&app), [(0, 48, 61), B], "after Esc");
    }

    // ... and while the search box has the keys (typed into, nothing done since but key
    // releases, hovers and timers), no key edits the roll: Up, Shift+Up, Down and Ctrl+D, the
    // ones the box lets through.
    #[test]
    fn typing_in_the_search_never_fires_the_rolls_keys() {
        let (mut app, mut player, mut window) = typed_into_the_search();
        let over = Hover::Roll(RollPart::Empty { key: 60 });
        let _ = app.update(Message::Hover(HoverMessage::Over(over)));
        run(&mut app, &mut player, 3);
        for (named, code, modifiers) in [
            (Named::ArrowUp, Code::ArrowUp, Modifiers::empty()),
            (Named::ArrowUp, Code::ArrowUp, Modifiers::SHIFT),
            (Named::ArrowDown, Code::ArrowDown, Modifiers::empty()),
        ] {
            tap(&mut app, &Press(Key::Named(named), code, modifiers));
        }
        tap(&mut app, &ctrl("d", Code::KeyD));
        window.type_keys(&mut app, &[('i', Code::KeyI)]);
        window.press_named(&mut app, Named::ArrowUp, Code::ArrowUp);
        assert_eq!(lead(&app), [A, B], "nothing edited");
        assert_eq!(app.browse.model.search, "ki");
    }
}
