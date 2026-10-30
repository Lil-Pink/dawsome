//! Clip link and unlink in the window: the playlist header's link control and the clip menu's rows
//! give a clip its own copy of the pattern it shares, or give it back to the pattern its copy was
//! made from, the engine building each clip's edit (`edit::copies`); one click is one undo step
//! however many clips it changes. The roll follows the clip it edits: after either edit, or its
//! undo, the pattern box shows what that clip plays, so the rack's steps and the roll edit the same
//! pattern.

use std::collections::BTreeSet;

use dawsome_engine::edit::copies::{self, Link};
use dawsome_engine::edit::{self, Edit, EditError};
use dawsome_engine::project::{Clip, ClipId, PatternId, Project};

use super::recording::STOP_RECORDING_FIRST;
use super::App;
use crate::session::SessionError;

/// A clip given its own copy of its pattern, or linked back: what the clip menu's rows and the
/// header's link control send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipLink {
    /// Make unique: the clip plays a copy of its pattern from now on.
    Unlink(ClipId),
    /// The clip plays the pattern its copy was made from again.
    LinkBack(ClipId),
    /// The header's link control: what it shows done to the selected clips as they are when
    /// the click lands ([`selection_link`]).
    Selected,
}

/// How the selected pattern clips share their patterns: what the playlist header's link control
/// shows, and the clips a click on it changes, in clip order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionLink {
    /// Some of them play a pattern other clips play too (inside the selection or out of it): a
    /// click gives each of `clips` its own copy. `places`: how many clips play the first one's
    /// pattern, for the words of a single clip.
    Linked { clips: Vec<ClipId>, places: usize },
    /// None of them is shared and some play a copy alone: a click links each of `clips` back.
    /// `original`: what the first one's copy was made from, for the words of a single clip.
    Unlinked {
        clips: Vec<ClipId>,
        original: PatternId,
    },
}

impl SelectionLink {
    /// The clips a click changes.
    pub fn clips(&self) -> &[ClipId] {
        match self {
            SelectionLink::Linked { clips, .. } | SelectionLink::Unlinked { clips, .. } => clips,
        }
    }
}

/// What the header's link control shows for `selection`: linked while any selected pattern clip
/// shares its pattern (linked wins, as it did in the roll: the control warns before an edit),
/// else unlinked while any plays a copy alone; `None` hides it. Audio and automation clips, and
/// a pattern clip alone on a pattern that is no copy, have nothing to link or unlink: they are
/// left out, of what it shows and of what a click does.
pub fn selection_link(project: &Project, selection: &BTreeSet<ClipId>) -> Option<SelectionLink> {
    // `copies::link_of`'s rule with the clips counted once, not once a selected clip: a
    // selection of every clip of a long song stays one pass over the playlist.
    let counts = copies::clip_counts(project);
    let mut shared = Vec::new();
    let mut copies = Vec::new();
    let mut first_places = None;
    let mut first_original = None;
    for &clip in selection {
        let Some(pattern) = project.playlist.clips.get(&clip).and_then(Clip::pattern) else {
            continue;
        };
        let places = counts.get(&pattern).copied().unwrap_or(0);
        if places > 1 {
            first_places.get_or_insert(places);
            shared.push(clip);
        } else if let Some(original) = copies::original_of(project, clip) {
            first_original.get_or_insert(original);
            copies.push(clip);
        }
    }
    if let Some(places) = first_places {
        return Some(SelectionLink::Linked {
            clips: shared,
            places,
        });
    }
    first_original.map(|original| SelectionLink::Unlinked {
        clips: copies,
        original,
    })
}

/// The edit a click on the header's link control makes for `link`, one transaction: each clip's
/// edit built on the document the ones before it leave (so the copies are "(copy)", "(copy 2)"…
/// and take the next pattern IDs in turn). A shared clip whose pattern no other clip plays by
/// its turn (the selection held every clip of that pattern, and the others took copies) keeps
/// it: it is its own already, and a copy of it would leave the pattern played by none.
fn selection_edit(project: &Project, link: &SelectionLink) -> Result<Edit, EditError> {
    let mut after = project.clone();
    let mut edits = Vec::new();
    for &clip in link.clips() {
        let edit = match link {
            SelectionLink::Linked { .. } => {
                if !matches!(copies::link_of(&after, clip), Some(Link::Linked { .. })) {
                    continue;
                }
                copies::unlink(&after, clip)?.edit
            }
            SelectionLink::Unlinked { .. } => copies::link_back(&after, clip)?.edit,
        };
        edit::apply(&mut after, edit.clone())?;
        // Flat, so the step is named by its edits as one clip's is (`edit_names`), and one
        // clip's edit is the engine's own.
        match edit {
            Edit::Transaction(parts) => edits.extend(parts),
            edit => edits.push(edit),
        }
    }
    if edits.len() == 1 {
        return Ok(edits.remove(0));
    }
    Ok(Edit::Transaction(edits))
}

impl App {
    /// Unlinks or links back a clip, or the selected clips, one undo step; refused under a take
    /// of notes, whose undo step names the pattern it records into (a link back can delete that
    /// pattern).
    pub(super) fn clip_link(&mut self, link: ClipLink) {
        if self.recording.taking_notes() {
            self.notice = Some(STOP_RECORDING_FIRST.to_owned());
            return;
        }
        let project = self.session.project();
        let edit = match link {
            ClipLink::Unlink(clip) => copies::unlink(project, clip).map(|unlink| unlink.edit),
            ClipLink::LinkBack(clip) => copies::link_back(project, clip).map(|back| back.edit),
            ClipLink::Selected => match self.selection_link() {
                Some(selected) => selection_edit(project, &selected),
                None => return,
            },
        };
        match edit {
            Ok(edit) => self.commit(edit),
            Err(error) => self.report(Err(SessionError::from(error))),
        }
    }

    /// The playlist header's link control: how the selected clips share their patterns, `None`
    /// while it is hidden.
    pub(super) fn selection_link(&self) -> Option<SelectionLink> {
        selection_link(self.session.project(), &self.arranger.selection)
    }

    /// After every change: the roll forgets a clip that is gone or whose pattern the pattern
    /// box no longer shows, and follows one whose pattern changed under it (unlinked, linked
    /// back, undone), the pattern box with it.
    pub(super) fn keep_roll_clip(&mut self) -> Result<(), SessionError> {
        let Some(pattern) = self.roll.keep_clip(self.session.project(), self.pattern) else {
            return Ok(());
        };
        // The copy holds the same notes, so the selection and the view stay; notes that are not
        // there are dropped by the refresh that follows.
        self.pattern = Some(pattern);
        self.roll.cancel_gesture();
        self.session.set_mode(self.plan_mode())
    }
}

#[cfg(test)]
mod tests {
    //! The scenario found in use, with the control in the playlist's header, on
    //! tests/fixtures/p20_five_clips.json, "Pattern 1" on five one-bar clips side by side on
    //! Track 1 (a step on tick 0, a note on tick 96 key 64), through the real widgets where a
    //! press is the point: the clicks that select clips, the double-click that opens one, the
    //! header's chain right of Fit, the clip menu. Expected documents and names are read off the
    //! fixture by hand.

    use std::collections::BTreeSet;
    use std::path::Path;

    use dawsome_engine::edit::copies::{self, Link};
    use dawsome_engine::project::{ChannelId, ClipId, PatternId};
    use dawsome_engine::realtime::Player;
    use iced::keyboard::Modifiers;
    use iced::{mouse, Point, Size};

    use super::super::files::FileMessage;
    use super::super::tests::app_on;
    use super::super::widget_events::Window;
    use super::super::{App, Message, Panel, PatternMessage};
    use super::{selection_link, ClipLink, SelectionLink};
    use crate::ui::hint::{ClipRow, Hover, HoverMessage, PlaylistTool, RollPart};
    use crate::ui::layout::TOOLBAR_HEIGHT;
    use crate::ui::piano_layout::STRIP_HEIGHT;
    use crate::ui::playlist::{clip_rows, link_badge};
    use crate::ui::playlist_layout::{self, HEADER_HEIGHT, RULER_HEIGHT};
    use crate::ui::roll::NoteEdit;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    fn fixture() -> std::path::PathBuf {
        Path::new(FIXTURES).join("p20_five_clips.json")
    }

    /// The five clips with the playlist alone under the toolbar of a 1000 x 560 window, the hint
    /// bar off, so tracks point (x, y) is window point (x, y + toolbar + header + ruler).
    fn five() -> (App, Player) {
        let (mut app, player) = app_on(&fixture());
        let _ = app.update(Message::TogglePanel(Panel::Playlist));
        let _ = app.update(Message::TogglePanel(Panel::Rack));
        let _ = app.update(Message::WindowResized(Size::new(1000.0, 560.0)));
        app.hints_flag = Some(false);
        assert!(app.panels.playlist && !app.panels.rack && !app.panels.piano_roll);
        let mut window = Window::new(&app);
        window.mouse(
            &mut app,
            mouse::Event::CursorLeft,
            mouse::Cursor::Unavailable,
        );
        (app, player)
    }

    /// The window point in the middle of `clip`'s body, read off the playlist's own layout.
    fn on_clip(app: &App, clip: ClipId) -> Point {
        let project = app.session.project();
        let shown = &project.playlist.clips[&clip];
        let index = crate::ui::clip_gesture::index_of(project, shown.track).expect("its track");
        let grid = &app.arranger.grid;
        let top = grid.row_top_of(index).expect("its row is shown");
        let rect = playlist_layout::clip_rect(grid, shown, top);
        let below = TOOLBAR_HEIGHT + HEADER_HEIGHT + RULER_HEIGHT;
        Point::new(rect.center_x(), rect.y + rect.height * 0.75 + below)
    }

    /// The pattern each clip plays, clips 1 up.
    fn played(app: &App) -> Vec<u32> {
        let clips = &app.session.project().playlist.clips;
        clips
            .values()
            .filter_map(|clip| clip.pattern())
            .map(|p| p.0)
            .collect()
    }

    /// The patterns as (id, name).
    fn patterns(app: &App) -> Vec<(u32, String)> {
        let patterns = &app.session.project().patterns;
        patterns
            .iter()
            .map(|(id, p)| (id.0, p.name.clone()))
            .collect()
    }

    /// Channel 1's notes in `pattern` as (tick, key).
    fn notes(app: &App, pattern: u32) -> Vec<(i64, u8)> {
        let shown = &app.session.project().patterns[&PatternId(pattern)];
        let notes = shown.notes.iter().filter(|n| n.channel == ChannelId(1));
        notes.map(|n| (n.start.0, n.key)).collect()
    }

    /// The "×N" each clip shows, clips 1 up.
    fn badges(app: &App) -> Vec<Option<String>> {
        let project = app.session.project();
        let counts = copies::clip_counts(project);
        let clips = project
            .playlist
            .clips
            .values()
            .filter_map(|clip| clip.pattern());
        clips.map(|p| link_badge(counts[&p])).collect()
    }

    /// The clip the roll edits and how it shares its pattern, while the pattern box shows that
    /// clip's pattern: what the roll's strip showed its chain for before the control moved to
    /// the playlist's header.
    fn roll_edits(app: &App) -> Option<(ClipId, Link)> {
        let clip = app.roll.clip?;
        let project = app.session.project();
        let plays = project.playlist.clips.get(&clip)?.pattern()?;
        if Some(plays) != app.pattern {
            return None;
        }
        Some((clip, copies::link_of(project, clip)?))
    }

    /// The hovers met along the line at window height `y`, swept 2 px apart from x `from`
    /// towards x `to`, each with the first point it was met at, in the order met.
    fn swept(
        app: &mut App,
        window: &mut Window,
        y: f32,
        from: f32,
        to: f32,
    ) -> Vec<(Hover, Point)> {
        let mut met: Vec<(Hover, Point)> = Vec::new();
        let step = if to > from { 2.0 } else { -2.0 };
        let mut x = from;
        while (to - x) * step > 0.0 {
            let at = Point::new(x, y);
            let event = mouse::Event::CursorMoved { position: at };
            for message in window.mouse(app, event, mouse::Cursor::Available(at)) {
                if let Message::Hover(HoverMessage::Over(hover)) = message {
                    if met.last().map(|(last, _)| last) != Some(&hover) {
                        met.push((hover, at));
                    }
                }
            }
            x += step;
        }
        let event = mouse::Event::CursorLeft;
        window.mouse(app, event, mouse::Cursor::Unavailable);
        met
    }

    /// The playlist's header (the top panel), swept left to right along its middle: the link
    /// control's point, when it shows; it stands directly right of Fit, no control between.
    fn header_chain(app: &mut App, window: &mut Window) -> Option<Point> {
        let y = TOOLBAR_HEIGHT + HEADER_HEIGHT / 2.0;
        let width = app.window_size.width;
        let met = swept(app, window, y, 1.0, width);
        let fit = Hover::PlaylistTool(PlaylistTool::Fit);
        let link = Hover::PlaylistTool(PlaylistTool::Link);
        let place = |hover: &Hover| met.iter().position(|(met, _)| met == hover);
        let fit_at = place(&fit).expect("the header's Fit");
        let link_at = place(&link)?;
        assert_eq!(link_at, fit_at + 1, "directly right of Fit: {met:?}");
        Some(met[link_at].1)
    }

    // The scenario with the control in the playlist's header: with no clip
    // selected the header has no chain; the third of five "Pattern 1" clips double-clicked is
    // selected and opens the roll on it, and the header shows the chain right of Fit (5
    // places); a click unlinks it: "Pattern 1 (copy)" with the same notes, clip 3 on it, the
    // pattern box on the copy (the roll edits clip 3), the chain crossed; the other clips show
    // "×4", clip 3 nothing. A note edited in the roll and a step pressed in the rack land in
    // the copy alone. A second click links it back (the copy gone, the box on Pattern 1, "×5");
    // Undo brings the copy back with both edits and the box follows it; three more undos give
    // the file back.
    #[test]
    fn unlink_edit_link_back_and_undo_through_the_header() {
        let (mut app, _player) = five();
        let mut window = Window::new(&app);
        assert_eq!(app.selection_link(), None);
        assert_eq!(
            header_chain(&mut app, &mut window),
            None,
            "nothing selected"
        );
        let third = on_clip(&app, ClipId(3));
        window.click(&mut app, third);
        window.click(&mut app, third);
        assert!(
            app.panels.piano_roll && app.panels.playlist,
            "the roll opened"
        );
        assert_eq!(app.arranger.selection, BTreeSet::from([ClipId(3)]));
        assert_eq!(app.roll.clip, Some(ClipId(3)));
        assert_eq!(app.pattern, Some(PatternId(1)));
        assert_eq!(app.selected_channel, Some(ChannelId(1)));
        assert_eq!(
            app.selection_link(),
            Some(SelectionLink::Linked {
                clips: vec![ClipId(3)],
                places: 5
            })
        );
        assert_eq!(
            roll_edits(&app),
            Some((ClipId(3), Link::Linked { clips: 5 }))
        );
        assert_eq!(badges(&app), vec![Some("\u{d7}5".to_owned()); 5]);

        let at = header_chain(&mut app, &mut window).expect("the chain, a clip selected");
        window.click(&mut app, at);
        assert_eq!(
            patterns(&app),
            [
                (1, "Pattern 1".to_owned()),
                (2, "Pattern 1 (copy)".to_owned())
            ]
        );
        assert_eq!(played(&app), [1, 1, 2, 1, 1]);
        assert_eq!(notes(&app, 2), notes(&app, 1), "the same notes");
        assert_eq!(app.pattern, Some(PatternId(2)), "the pattern box follows");
        let unlinked = Link::Unlinked {
            original: PatternId(1),
        };
        assert_eq!(roll_edits(&app), Some((ClipId(3), unlinked)));
        assert_eq!(
            app.selection_link(),
            Some(SelectionLink::Unlinked {
                clips: vec![ClipId(3)],
                original: PatternId(1)
            }),
            "the red X"
        );
        assert_eq!(app.session.history().undo_name(), Some("Unlink clip"));
        let four = Some("\u{d7}4".to_owned());
        assert_eq!(
            badges(&app),
            [four.clone(), four.clone(), None, four.clone(), four]
        );

        let _ = app.update(Message::SelectAllNotes);
        let _ = app.update(Message::RollKey(NoteEdit::Transpose(3)));
        let _ = app.update(Message::StepClicked {
            channel: ChannelId(1),
            step: 8,
        });
        assert_eq!(notes(&app, 2), [(0, 63), (96, 67), (192, 60)], "the copy");
        assert_eq!(notes(&app, 1), [(0, 60), (96, 64)], "Pattern 1 as it was");

        let at = header_chain(&mut app, &mut window).expect("the crossed chain");
        window.click(&mut app, at);
        assert_eq!(patterns(&app), [(1, "Pattern 1".to_owned())]);
        assert_eq!(played(&app), [1; 5]);
        assert_eq!(app.pattern, Some(PatternId(1)));
        assert_eq!(
            roll_edits(&app),
            Some((ClipId(3), Link::Linked { clips: 5 }))
        );
        assert_eq!(app.session.history().undo_name(), Some("Link clip back"));

        let _ = app.update(Message::Undo);
        assert_eq!(played(&app), [1, 1, 2, 1, 1]);
        assert_eq!(
            notes(&app, 2),
            [(0, 63), (96, 67), (192, 60)],
            "with its edits"
        );
        assert_eq!(app.pattern, Some(PatternId(2)), "the box follows the clip");
        assert_eq!(roll_edits(&app), Some((ClipId(3), unlinked)));
        for _ in 0..3 {
            let _ = app.update(Message::Undo);
        }
        let file = std::fs::read_to_string(fixture()).expect("reads");
        let document = app.session.project().to_json().expect("serializes");
        assert_eq!(document, file.trim_end(), "the file as it was");
        assert_eq!(app.pattern, Some(PatternId(1)));
        assert_eq!(app.error, None);
    }

    // Two clips selected (clip 2 clicked, clip 4 Shift+clicked): the header's chain unlinks
    // both in one click, "Pattern 1 (copy)" for clip 2 and "Pattern 1 (copy 2)" for clip 4,
    // each with Pattern 1's notes, one undo step ("Unlink clips") that gives the file back and
    // a redo that brings both again. Then both play copies alone: the red X links both back,
    // the copies gone, one undo step ("Link clips back") again.
    #[test]
    fn two_selected_clips_unlink_and_link_back_in_one_step_each() {
        let (mut app, _player) = five();
        let mut window = Window::new(&app);
        let (second, fourth) = (on_clip(&app, ClipId(2)), on_clip(&app, ClipId(4)));
        window.click(&mut app, second);
        window.hold(&mut app, Modifiers::SHIFT);
        window.click(&mut app, fourth);
        window.hold(&mut app, Modifiers::empty());
        assert_eq!(
            app.arranger.selection,
            BTreeSet::from([ClipId(2), ClipId(4)])
        );
        assert_eq!(played(&app), [1; 5], "selecting changed nothing");
        assert_eq!(
            app.selection_link(),
            Some(SelectionLink::Linked {
                clips: vec![ClipId(2), ClipId(4)],
                places: 5
            })
        );

        let at = header_chain(&mut app, &mut window).expect("the chain");
        window.click(&mut app, at);
        assert_eq!(
            patterns(&app),
            [
                (1, "Pattern 1".to_owned()),
                (2, "Pattern 1 (copy)".to_owned()),
                (3, "Pattern 1 (copy 2)".to_owned())
            ]
        );
        assert_eq!(played(&app), [1, 2, 1, 3, 1]);
        assert_eq!(notes(&app, 2), [(0, 60), (96, 64)]);
        assert_eq!(notes(&app, 3), [(0, 60), (96, 64)]);
        assert_eq!(app.session.history().undo_name(), Some("Unlink clips"));
        let after = app.session.project().to_json().expect("serializes");
        let _ = app.update(Message::Undo);
        let file = std::fs::read_to_string(fixture()).expect("reads");
        let document = app.session.project().to_json().expect("serializes");
        assert_eq!(document, file.trim_end(), "one undo gives the file back");
        let _ = app.update(Message::Redo);
        let redone = app.session.project().to_json().expect("serializes");
        assert_eq!(redone, after, "one redo brings both copies");

        assert_eq!(
            app.selection_link(),
            Some(SelectionLink::Unlinked {
                clips: vec![ClipId(2), ClipId(4)],
                original: PatternId(1)
            })
        );
        let at = header_chain(&mut app, &mut window).expect("the crossed chain");
        window.click(&mut app, at);
        assert_eq!(patterns(&app), [(1, "Pattern 1".to_owned())]);
        assert_eq!(played(&app), [1; 5]);
        assert_eq!(app.session.history().undo_name(), Some("Link clips back"));
        let _ = app.update(Message::Undo);
        assert_eq!(played(&app), [1, 2, 1, 3, 1], "one undo");
        assert_eq!(app.error, None);
    }

    // What the control shows for a selection, worked by hand on the five clips: nothing for
    // none; a mixed selection shows the chain (linked wins) and its click unlinks the shared
    // clip alone, the copy staying as it is; every clip of a pattern selected, the click leaves
    // the last on the pattern (it is its own once the others have copies), and the red X then
    // links the four copies back. Audio and automation clips are left out (p8_ui's two, with
    // two "Pattern 1" clips placed beside them), and a selection of only those hides it.
    #[test]
    fn the_selection_decides_what_the_control_shows_and_does() {
        let (mut app, _player) = five();
        let select = |app: &mut App, clips: &[u32]| {
            app.arranger.selection = clips.iter().map(|&id| ClipId(id)).collect();
        };
        let click = |app: &mut App| {
            let _ = app.update(Message::ClipLink(ClipLink::Selected));
        };
        select(&mut app, &[]);
        assert_eq!(app.selection_link(), None);
        click(&mut app);
        assert_eq!(app.session.history().undo_name(), None, "nothing to do");

        let _ = app.update(Message::ClipLink(ClipLink::Unlink(ClipId(3))));
        assert_eq!(played(&app), [1, 1, 2, 1, 1]);
        select(&mut app, &[1, 3]);
        assert_eq!(
            app.selection_link(),
            Some(SelectionLink::Linked {
                clips: vec![ClipId(1)],
                places: 4
            }),
            "linked wins"
        );
        click(&mut app);
        assert_eq!(played(&app), [3, 1, 2, 1, 1], "clip 3 keeps its copy");
        assert_eq!(patterns(&app)[2], (3, "Pattern 1 (copy 2)".to_owned()));
        assert_eq!(
            app.selection_link(),
            Some(SelectionLink::Unlinked {
                clips: vec![ClipId(1), ClipId(3)],
                original: PatternId(1)
            })
        );
        let _ = app.update(Message::Undo);
        let _ = app.update(Message::Undo);
        assert_eq!(played(&app), [1; 5]);

        select(&mut app, &[1, 2, 3, 4, 5]);
        click(&mut app);
        assert_eq!(played(&app), [2, 3, 4, 5, 1], "the last keeps Pattern 1");
        assert_eq!(
            patterns(&app),
            [
                (1, "Pattern 1".to_owned()),
                (2, "Pattern 1 (copy)".to_owned()),
                (3, "Pattern 1 (copy 2)".to_owned()),
                (4, "Pattern 1 (copy 3)".to_owned()),
                (5, "Pattern 1 (copy 4)".to_owned())
            ]
        );
        assert_eq!(badges(&app), [None, None, None, None, None]);
        assert_eq!(
            app.selection_link(),
            Some(SelectionLink::Unlinked {
                clips: vec![ClipId(1), ClipId(2), ClipId(3), ClipId(4)],
                original: PatternId(1)
            }),
            "clip 5, alone on a pattern that is no copy, is left out"
        );
        click(&mut app);
        assert_eq!(played(&app), [1; 5]);
        assert_eq!(patterns(&app), [(1, "Pattern 1".to_owned())]);
        let _ = app.update(Message::Undo);
        assert_eq!(played(&app), [2, 3, 4, 5, 1], "one step");

        let (mut mixed, _player) = app_on(&Path::new(FIXTURES).join("p8_ui.json"));
        let placed = |start| {
            dawsome_engine::project::Clip::of_pattern(
                PatternId(1),
                dawsome_engine::project::TrackId(1),
                dawsome_engine::time::Ticks(start),
                dawsome_engine::time::Ticks(384),
            )
        };
        mixed.commit(dawsome_engine::edit::Edit::PlaceClips {
            first: ClipId(3),
            clips: vec![placed(768), placed(1152)],
        });
        let project = mixed.session.project();
        let audio = project.playlist.clips[&ClipId(1)].pattern();
        assert_eq!(audio, None, "clip 1 is the audio clip");
        let only = BTreeSet::from([ClipId(1), ClipId(2)]);
        assert_eq!(selection_link(project, &only), None, "audio and automation");
        let with = BTreeSet::from([ClipId(1), ClipId(2), ClipId(4)]);
        assert_eq!(
            selection_link(project, &with),
            Some(SelectionLink::Linked {
                clips: vec![ClipId(4)],
                places: 2
            })
        );
        mixed.arranger.selection = with;
        let _ = mixed.update(Message::ClipLink(ClipLink::Selected));
        let clips = &mixed.session.project().playlist.clips;
        let plays: Vec<_> = clips.values().map(|clip| clip.pattern()).collect();
        assert_eq!(plays, [None, None, Some(PatternId(1)), Some(PatternId(2))]);
    }

    // P2: the roll opened on a shared clip (double-clicked) has no chain in its strip any more:
    // swept leftwards from the strip's right end, the first control is Notes ▾ and the next
    // the Fold switch, nothing between them. The roll still edits the clip.
    #[test]
    fn the_roll_strip_ends_with_notes_again() {
        let (mut app, _player) = five();
        let mut window = Window::new(&app);
        let third = on_clip(&app, ClipId(3));
        window.click(&mut app, third);
        window.click(&mut app, third);
        assert_eq!(
            roll_edits(&app),
            Some((ClipId(3), Link::Linked { clips: 5 }))
        );
        let _ = app.update(Message::TogglePanel(Panel::Playlist));
        assert!(app.panels.piano_roll && !app.panels.playlist && !app.panels.rack);
        let y = TOOLBAR_HEIGHT + STRIP_HEIGHT / 2.0;
        let width = app.window_size.width;
        let met = swept(&mut app, &mut window, y, width - 1.0, 0.0);
        let first: Vec<&Hover> = met.iter().take(2).map(|(hover, _)| hover).collect();
        assert_eq!(
            first,
            [
                &Hover::Roll(RollPart::NotesMenu),
                &Hover::Roll(RollPart::Fold)
            ],
            "the strip's right end: {met:?}"
        );
    }

    // The roll edits a clip only when opened from one: the key opens it on the pattern alone
    // (no clip), and opening a file lands it on its first clip (Track 1 first) while the roll
    // is open; the clip is forgotten when the pattern is switched by hand, when the clip is
    // deleted (and stays forgotten after the delete's undo), and when the key opens the roll.
    #[test]
    fn the_roll_edits_a_clip_only_when_opened_from_one() {
        let (mut app, _player) = app_on(&fixture());
        let _ = app.update(Message::TogglePanel(Panel::PianoRoll));
        assert_eq!(app.roll.clip, None, "opened by its key");
        assert_eq!(roll_edits(&app), None);
        let _ = app.update(Message::File(FileMessage::OpenPicked(Some(fixture()))));
        assert!(app.panels.piano_roll);
        assert_eq!(app.roll.clip, Some(ClipId(1)), "the landing's clip");
        assert_eq!(
            roll_edits(&app),
            Some((ClipId(1), Link::Linked { clips: 5 }))
        );
        let _ = app.update(Message::TogglePanel(Panel::PianoRoll));
        let _ = app.update(Message::TogglePanel(Panel::PianoRoll));
        assert_eq!(app.roll.clip, None, "the key opened it again");

        let _ = app.update(Message::File(FileMessage::OpenPicked(Some(fixture()))));
        let _ = app.update(Message::ClipLink(ClipLink::Unlink(ClipId(1))));
        assert_eq!(app.pattern, Some(PatternId(2)), "the box followed the clip");
        let _ = app.update(Message::PatternSelected(PatternId(1)));
        assert_eq!(app.roll.clip, None, "switched by hand");
        assert_eq!(roll_edits(&app), None);

        let _ = app.update(Message::File(FileMessage::OpenPicked(Some(fixture()))));
        assert_eq!(app.roll.clip, Some(ClipId(1)));
        let _ = app.update(Message::Pattern(PatternMessage::Next));
        assert_eq!(app.roll.clip, None, "the next arrow");

        let _ = app.update(Message::File(FileMessage::OpenPicked(Some(fixture()))));
        app.commit(dawsome_engine::edit::Edit::DeleteClips {
            ids: vec![ClipId(1)],
        });
        assert_eq!(app.roll.clip, None, "the clip deleted");
        let _ = app.update(Message::Undo);
        assert_eq!(app.roll.clip, None, "and its delete undone");
    }

    // The clip menu, Ctrl+right-click on a pattern clip through the real widgets: on clip 3 of
    // five it offers "Unlink (make unique)", whose row unlinks it (the roll edits no clip, so
    // the pattern box stays); on clip 3 then "Link back to Pattern 1", whose row links it back;
    // each one undo step. A plain right press still deletes a clip. A copy other clips play too
    // offers both rows; a clip alone on a pattern that is no copy, Unlink dimmed.
    #[test]
    fn the_clip_menu_unlinks_and_links_back() {
        let (mut app, _player) = five();
        let mut window = Window::new(&app);
        let third = on_clip(&app, ClipId(3));
        let right = |window: &mut Window, app: &mut App, at: Point| {
            for event in [mouse::Event::ButtonPressed, mouse::Event::ButtonReleased] {
                let event = event(mouse::Button::Right);
                window.mouse(app, event, mouse::Cursor::Available(at));
            }
        };
        let choose = |window: &mut Window, app: &mut App, row: usize| {
            let menus = window.overlays(app);
            assert_eq!(menus.len(), 1, "the clip menu");
            let menu = menus[0];
            let y = menu.y
                + crate::ui::menu::PADDING
                + crate::ui::menu::ROW_HEIGHT * (row as f32 + 0.5);
            let at = Point::new(menu.center_x(), y);
            for message in window.click_overlay(app, at) {
                let _ = app.update(message);
            }
        };
        window.hold(&mut app, Modifiers::CTRL);
        right(&mut window, &mut app, third);
        assert_eq!(app.arranger.clip_menu.map(|(id, _)| id), Some(ClipId(3)));
        let rows = clip_rows(app.session.project(), ClipId(3));
        assert_eq!(
            rows,
            [(ClipRow::Unlink, "Unlink (make unique)".to_owned(), true)]
        );
        choose(&mut window, &mut app, 0);
        assert_eq!(app.arranger.clip_menu, None, "the row closes it");
        assert_eq!(played(&app), [1, 1, 2, 1, 1]);
        assert_eq!(app.pattern, Some(PatternId(1)), "the box stays");
        assert_eq!(app.session.history().undo_name(), Some("Unlink clip"));

        right(&mut window, &mut app, third);
        let rows = clip_rows(app.session.project(), ClipId(3));
        let back = (ClipRow::LinkBack, "Link back to Pattern 1".to_owned(), true);
        assert_eq!(rows, std::slice::from_ref(&back));
        choose(&mut window, &mut app, 0);
        assert_eq!(played(&app), [1; 5]);
        assert_eq!(patterns(&app), [(1, "Pattern 1".to_owned())]);
        let _ = app.update(Message::Undo);
        assert_eq!(played(&app), [1, 1, 2, 1, 1], "one undo");

        window.hold(&mut app, Modifiers::empty());
        let first = on_clip(&app, ClipId(1));
        right(&mut window, &mut app, first);
        assert_eq!(app.arranger.clip_menu, None);
        assert_eq!(played(&app), [1, 2, 1, 1], "clip 1 deleted");
        let _ = app.update(Message::Undo);

        let sixth = dawsome_engine::project::Clip::of_pattern(
            PatternId(2),
            dawsome_engine::project::TrackId(1),
            dawsome_engine::time::Ticks(1920),
            dawsome_engine::time::Ticks(384),
        );
        app.commit(dawsome_engine::edit::Edit::PlaceClips {
            first: ClipId(6),
            clips: vec![sixth],
        });
        let rows = clip_rows(app.session.project(), ClipId(3));
        let unlink = (ClipRow::Unlink, "Unlink (make unique)".to_owned(), true);
        assert_eq!(rows, [unlink, back]);
        let (alone, _player) = app_on(&Path::new(FIXTURES).join("p17_song.json"));
        let rows = clip_rows(alone.session.project(), ClipId(1));
        assert_eq!(
            rows,
            [(ClipRow::Unlink, "Unlink (make unique)".to_owned(), false)]
        );
    }

    // The control's words, as the spec words them, in its tip and the hint bar: one selected
    // clip in the roll's control's words, several counted.
    #[test]
    fn the_link_control_says_what_a_click_does() {
        let (mut app, _player) = app_on(&fixture());
        let project = app.session.project();
        let linked = SelectionLink::Linked {
            clips: vec![ClipId(3)],
            places: 5,
        };
        assert_eq!(
            crate::ui::hint::link_tip(project, &linked),
            "Linked: this pattern plays in 5 places; edits change all of them. Click to unlink this clip."
        );
        let two = SelectionLink::Linked {
            clips: vec![ClipId(2), ClipId(4)],
            places: 5,
        };
        assert_eq!(
            crate::ui::hint::link_tip(project, &two),
            "Linked: 2 selected clips share their patterns; edits change every clip playing them. Click to give each its own copy."
        );
        let _ = app.update(Message::ClipLink(ClipLink::Unlink(ClipId(3))));
        let project = app.session.project();
        let unlinked = SelectionLink::Unlinked {
            clips: vec![ClipId(3)],
            original: PatternId(1),
        };
        assert_eq!(
            crate::ui::hint::link_tip(project, &unlinked),
            "Unlinked: this clip has its own copy. Click to link it back to Pattern 1."
        );
        let both = SelectionLink::Unlinked {
            clips: vec![ClipId(2), ClipId(3)],
            original: PatternId(1),
        };
        assert_eq!(
            crate::ui::hint::link_tip(project, &both),
            "Unlinked: 2 selected clips have their own copies. Click to link each back to the pattern it was copied from."
        );
        let words = |app: &mut App, clips: &[u32]| {
            app.arranger.selection = clips.iter().map(|&id| ClipId(id)).collect();
            let context = app.hint_context();
            crate::ui::hint::words(&Hover::PlaylistTool(PlaylistTool::Link), &context)
        };
        assert_eq!(
            words(&mut app, &[3]),
            "Unlinked \u{b7} this clip has its own copy \u{b7} click to link it back to Pattern 1"
        );
        assert_eq!(
            words(&mut app, &[1]),
            "Linked \u{b7} this pattern plays in 4 places \u{b7} edits change all of them; click to unlink this clip"
        );
        assert_eq!(
            words(&mut app, &[1, 2]),
            "Linked \u{b7} 2 selected clips share their patterns \u{b7} edits change every clip \
             playing them; click to give each its own copy"
        );
        let _ = app.update(Message::ClipLink(ClipLink::Unlink(ClipId(1))));
        assert_eq!(
            words(&mut app, &[1, 3]),
            "Unlinked \u{b7} 2 selected clips have their own copies \u{b7} click to link each \
             back to the pattern it was copied from"
        );
    }
}
