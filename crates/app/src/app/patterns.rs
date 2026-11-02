//! Patterns in the window: the toolbar's selector, whose next arrow makes a new pattern past
//! the last and whose name opens a menu to rename or delete the pattern, and the rack header's
//! bar count and swing for the selected pattern. The selection always names a pattern that
//! exists.

use dawsome_engine::edit::{takes, Edit, EditError};
use dawsome_engine::plan;
use dawsome_engine::project::{ClipId, PatternId, PatternKind};
use dawsome_engine::time::{STRAIGHT_SWING, SWING_RANGE};
use iced::widget::{operation, Id};
use iced::Task;

use super::recording::STOP_RECORDING_FIRST;
use super::{App, Message, PlayMode};
use crate::session::SessionError;
use crate::ui::panels::Panel;
use crate::ui::rack::RackHeader;
use crate::ui::step_grid::MAX_STEP_BARS;

/// The text box a pattern's name is edited in.
pub const RENAME_INPUT: &str = "pattern-rename";

/// One step of the swing knob, a wheel notch or 4 px of drag, in tenths of a per cent: 1 %.
/// Shift makes it a tenth of that ([`App::moved_swing`]).
pub const SWING_STEP: i32 = 10;

/// What the pattern controls report.
#[derive(Debug, Clone)]
pub enum PatternMessage {
    /// The toolbar's next arrow: the next pattern, or past the last a new one.
    Next,
    /// A right press on the pattern's name: its menu opens, or closes.
    ToggleMenu,
    /// A press beside the open menu.
    CloseMenu,
    /// The menu's Rename: the name becomes a text box.
    Rename,
    RenameEdited(String),
    /// Enter in the text box.
    RenameSubmitted,
    /// The menu's Delete: the pattern and every clip of it, one undo step.
    Delete,
    /// The rack header's "−" (−1) and "+" (1): the selected pattern a bar shorter or longer.
    Bars(i32),
    /// The wheel on the rack header's swing: the selected pattern's swing moved this many
    /// tenths of a per cent ([`SWING_STEP`] a notch), a tenth as far with Shift held.
    SwingBy(i32),
    /// A double press on the swing: the selected pattern plays straight.
    SwingReset,
}

/// The pattern name's menu and rename in the window.
#[derive(Debug, Default, PartialEq)]
pub(super) struct PatternUi {
    pub menu: bool,
    /// The pattern being renamed and the text typed so far.
    pub rename: Option<(PatternId, String)>,
}

/// The kind a play mode makes: patterns in pattern mode, takes in song mode.
pub(super) fn kind_for(mode: PlayMode) -> PatternKind {
    match mode {
        PlayMode::Pattern => PatternKind::Pattern,
        PlayMode::Song => PatternKind::Take,
    }
}

/// Whether `name` is a take's name for `channel`: "<channel> take N".
fn is_take_of(name: &str, channel: &str) -> bool {
    name.strip_prefix(channel)
        .and_then(|rest| rest.strip_prefix(" take "))
        .is_some_and(|number| number.parse::<u32>().is_ok())
}

impl PatternUi {
    /// Closes the menu or ends a rename; returns whether one was open.
    pub fn escape(&mut self) -> bool {
        let open = self.menu || self.rename.is_some();
        *self = Self::default();
        open
    }
}

impl App {
    pub(super) fn pattern_message(&mut self, message: PatternMessage) -> Task<Message> {
        match message {
            PatternMessage::Next => {
                self.keep_take_for_another_pattern();
                match self.neighboring_patterns().1 {
                    Some(next) => self.select_pattern(next),
                    None => self.add_pattern(),
                }
            }
            PatternMessage::ToggleMenu => self.patterns.menu = !self.patterns.menu,
            PatternMessage::CloseMenu => self.patterns.menu = false,
            PatternMessage::Rename => {
                self.patterns.menu = false;
                let shown = self
                    .pattern
                    .and_then(|id| Some((id, self.session.project().patterns.get(&id)?)));
                if let Some((id, shown)) = shown {
                    self.patterns.rename = Some((id, shown.name.clone()));
                    let input = Id::new(RENAME_INPUT);
                    return operation::focus(input.clone()).chain(operation::select_all(input));
                }
            }
            PatternMessage::RenameEdited(text) => {
                if let Some((_, typed)) = &mut self.patterns.rename {
                    *typed = text;
                }
            }
            PatternMessage::RenameSubmitted => self.finish_rename(),
            PatternMessage::Delete => {
                self.patterns.menu = false;
                // Why refused under a take of notes: the take's undo step, saved at Stop,
                // names the pattern it records into, and no undo could apply it once that
                // pattern is gone.
                if self.recording.taking_notes() {
                    self.notice = Some(STOP_RECORDING_FIRST.to_owned());
                } else {
                    self.delete_pattern();
                }
            }
            PatternMessage::Bars(by) => {
                self.focus = Panel::Rack;
                self.nudge_bars(by);
            }
            PatternMessage::SwingBy(by) => {
                self.focus = Panel::Rack;
                self.set_swing(|app, swing| app.moved_swing(swing, by.into()));
            }
            PatternMessage::SwingReset => {
                self.focus = Panel::Rack;
                self.set_swing(|_, _| STRAIGHT_SWING);
            }
        }
        Task::none()
    }

    /// Whether the name menu offers Delete: not for the only pattern, and not under a take of
    /// notes, where Delete is refused.
    pub(super) fn offers_delete(&self) -> bool {
        self.session.project().patterns.len() > 1 && !self.recording.taking_notes()
    }

    /// Selects pattern `id`, which pattern mode then plays.
    pub(super) fn select_pattern(&mut self, id: PatternId) {
        if self.session.project().patterns.contains_key(&id) {
            self.pattern = Some(id);
            self.roll.forget_notes();
            self.send_mode();
        }
    }

    /// What the player plays: the selected pattern in pattern mode, else the song.
    pub(super) fn plan_mode(&self) -> plan::PlayMode {
        match (self.mode, self.pattern) {
            (PlayMode::Pattern, Some(pattern)) => plan::PlayMode::Pattern(pattern),
            _ => plan::PlayMode::Song,
        }
    }

    /// Keeps the selection on a pattern that exists: when an undo or a delete took the selected
    /// one away, the one before it is selected, else the first, and pattern mode plays it.
    pub(super) fn keep_pattern(&mut self) -> Result<(), SessionError> {
        let patterns = &self.session.project().patterns;
        let kept = match self.pattern {
            Some(id) if patterns.contains_key(&id) => return Ok(()),
            Some(id) => patterns
                .range(..id)
                .next_back()
                .or_else(|| patterns.iter().next())
                .map(|(&kept, _)| kept),
            None => patterns.keys().next().copied(),
        };
        if kept == self.pattern {
            return Ok(());
        }
        self.pattern = kept;
        self.roll.forget_notes();
        self.session.set_mode(self.plan_mode())
    }

    /// Adds a blank one-bar pattern past the last, of the kind the play mode makes (a take in
    /// song mode), and selects it: one undo step.
    fn add_pattern(&mut self) {
        match self.blank_pattern(kind_for(self.mode)) {
            Ok((edit, id)) => {
                self.commit(edit);
                self.select_pattern(id);
            }
            Err(error) => self.error = Some(SessionError::from(error).to_string()),
        }
    }

    /// The edit that adds a blank `kind` of one bar past the last, and its ID: a pattern
    /// "Pattern N" (N its ID), or a take named by [`new_take_name`](Self::new_take_name).
    pub(super) fn blank_pattern(&self, kind: PatternKind) -> Result<(Edit, PatternId), EditError> {
        let project = self.session.project();
        let id = project
            .next_pattern_id()
            .ok_or(EditError::PatternIdsExhausted)?;
        match kind {
            PatternKind::Pattern => {
                let add = Edit::AddPattern {
                    id,
                    name: format!("Pattern {}", id.0),
                    bars: 1,
                    copied_from: None,
                };
                Ok((add, id))
            }
            PatternKind::Take => takes::take_pattern(project, self.new_take_name(id)),
        }
    }

    /// A take's name for pattern `id`: after the selected channel, "Kick take 2" with the first
    /// number no pattern is named with ([`takes::take_name`]), else "Take N" (N its ID).
    fn new_take_name(&self, id: PatternId) -> String {
        let project = self.session.project();
        self.selected_channel
            .and_then(|channel| project.channels.get(&channel))
            .map_or_else(
                || format!("Take {}", id.0),
                |channel| takes::take_name(project, &channel.name),
            )
    }

    /// The edit that makes blank pattern `id` a `kind` in place, named as that kind is: a
    /// pattern "Pattern N" (N its ID); a take after the selected channel ([`new_take_name`]),
    /// unless it is a take named for that channel already, or a take and no channel is
    /// selected. `None` when it is that kind with that name already (or is gone).
    ///
    /// [`new_take_name`]: Self::new_take_name
    pub(super) fn reshape(&self, id: PatternId, kind: PatternKind) -> Option<Edit> {
        let project = self.session.project();
        let pattern = project.patterns.get(&id)?;
        let channel = self
            .selected_channel
            .and_then(|channel| project.channels.get(&channel));
        let keeps_name = match (kind, channel) {
            (PatternKind::Pattern, _) => false,
            (PatternKind::Take, Some(channel)) => is_take_of(&pattern.name, &channel.name),
            (PatternKind::Take, None) => pattern.kind == PatternKind::Take,
        };
        let name = match kind {
            _ if keeps_name => pattern.name.clone(),
            PatternKind::Pattern => format!("Pattern {}", id.0),
            PatternKind::Take => self.new_take_name(id),
        };
        let mut edits = Vec::new();
        if pattern.kind != kind {
            edits.push(Edit::SetPatternKind { pattern: id, kind });
        }
        if pattern.name != name {
            edits.push(Edit::RenamePattern { id, name });
        }
        (!edits.is_empty()).then_some(Edit::Transaction(edits))
    }

    /// The play mode switched: a blank pattern selected becomes, in place, the kind the new mode
    /// makes, so the right arrow and Record go on from it; one with notes stays as it is. One undo
    /// step.
    pub(super) fn reshape_selected_for_mode(&mut self) {
        let kind = kind_for(self.mode);
        let Some(id) = self.pattern else {
            return;
        };
        let project = self.session.project();
        let blank = project
            .patterns
            .get(&id)
            .is_some_and(|pattern| pattern.notes.is_empty() && pattern.kind != kind);
        if let Some(edit) = self.reshape(id, kind).filter(|_| blank) {
            self.commit(edit);
        }
    }

    /// Renames the pattern being renamed to the text typed, trimmed; blank text keeps the name.
    fn finish_rename(&mut self) {
        let Some((id, typed)) = self.patterns.rename.take() else {
            return;
        };
        let name = typed.trim();
        if !name.is_empty() {
            self.commit(Edit::RenamePattern {
                id,
                name: name.to_owned(),
            });
        }
    }

    /// Deletes the selected pattern, and before it every clip that plays it, as one undo step.
    fn delete_pattern(&mut self) {
        let Some(id) = self.pattern else {
            return;
        };
        let clips: Vec<ClipId> = self
            .session
            .project()
            .playlist
            .clips
            .iter()
            .filter(|(_, clip)| clip.pattern() == Some(id))
            .map(|(&clip, _)| clip)
            .collect();
        let delete = Edit::DeletePattern { id };
        let edit = if clips.is_empty() {
            delete
        } else {
            Edit::Transaction(vec![Edit::DeleteClips { ids: clips }, delete])
        };
        self.commit(edit);
    }

    /// Makes the selected pattern `by` bars longer, when [`Self::nudged_bars`] allows it.
    fn nudge_bars(&mut self, by: i32) {
        if let Some((pattern, bars)) = self.nudged_bars(by) {
            self.commit(Edit::SetPatternBars { pattern, bars });
        }
    }

    /// The selected pattern and its length `by` bars longer, when that keeps it from 1 to
    /// [`MAX_STEP_BARS`] bars: a pattern the piano roll grew past that can still get shorter,
    /// never longer.
    fn nudged_bars(&self, by: i32) -> Option<(PatternId, u32)> {
        let pattern = self.pattern?;
        let shown = self.session.project().patterns.get(&pattern)?;
        let bars = i64::from(shown.length_bars) + i64::from(by);
        let allowed = if by > 0 {
            bars <= i64::from(MAX_STEP_BARS)
        } else {
            bars >= 1
        };
        if !allowed {
            return None;
        }
        Some((pattern, u32::try_from(bars).ok()?))
    }

    /// The selected pattern and its swing, in tenths of a per cent.
    fn selected_swing(&self) -> Option<(PatternId, u16)> {
        let pattern = self.pattern?;
        let shown = self.session.project().patterns.get(&pattern)?;
        Some((pattern, shown.swing_tenths))
    }

    /// Sets the selected pattern's swing to what `to` makes of it, one undo step, when that
    /// changes it: a press at a stop takes no step.
    fn set_swing(&mut self, to: impl FnOnce(&Self, u16) -> u16) {
        let Some((pattern, swing)) = self.selected_swing() else {
            return;
        };
        let swing_tenths = to(self, swing);
        if swing_tenths != swing {
            self.commit(Edit::SetPatternSwing {
                pattern,
                swing_tenths,
            });
        }
    }

    /// `swing` moved `tenths` tenths of a per cent, a tenth as far while Shift is held (0.1 %
    /// for 1 %), kept inside [`SWING_RANGE`]. The one rule the swing's wheel and drag go by.
    pub(super) fn moved_swing(&self, swing: u16, tenths: i64) -> u16 {
        let tenths = if self.modifiers.shift() {
            tenths / i64::from(SWING_STEP)
        } else {
            tenths
        };
        let (low, high) = (*SWING_RANGE.start(), *SWING_RANGE.end());
        let moved = i64::from(swing)
            .saturating_add(tenths)
            .clamp(i64::from(low), i64::from(high));
        // The clamp keeps it inside the u16 swing range.
        u16::try_from(moved).unwrap_or(low)
    }

    /// The rack's header for the selected pattern: its bar count, whether "−" and "+" can
    /// change it, by the rule the presses go through, and its swing.
    pub(super) fn rack_header(&self) -> RackHeader {
        let bars = self
            .pattern
            .and_then(|id| self.session.project().patterns.get(&id))
            .map(|shown| shown.length_bars);
        RackHeader {
            bars,
            shorter: self.nudged_bars(-1).is_some(),
            longer: self.nudged_bars(1).is_some(),
            swing: self.selected_swing().map(|(_, swing)| swing),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use dawsome_engine::plan::PlayMode as PlanMode;
    use dawsome_engine::project::{ClipId, PatternId};

    use super::super::tests::app_on;
    use super::*;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    fn fixture_app(name: &str) -> App {
        app_on(&Path::new(FIXTURES).join(name)).0
    }

    fn send(app: &mut App, message: PatternMessage) {
        let _ = app.update(Message::Pattern(message));
    }

    /// The patterns as (id, name, bars, notes).
    fn patterns(app: &App) -> Vec<(u32, String, u32, usize)> {
        app.session
            .project()
            .patterns
            .iter()
            .map(|(id, pattern)| {
                let name = pattern.name.clone();
                (id.0, name, pattern.length_bars, pattern.notes.len())
            })
            .collect()
    }

    // The toolbar's next arrow selects the next pattern, and past the last makes a new empty
    // one, "Pattern 4" of one bar, which it selects and pattern mode plays: one undo step,
    // whose undo selects the pattern before it again.
    #[test]
    fn next_past_the_last_pattern_makes_a_new_one() {
        let mut app = fixture_app("p2_three_patterns.json");
        send(&mut app, PatternMessage::Next);
        send(&mut app, PatternMessage::Next);
        assert_eq!(app.pattern, Some(PatternId(3)));
        assert_eq!(patterns(&app).len(), 3, "walking adds nothing");
        send(&mut app, PatternMessage::Next);
        assert_eq!(
            patterns(&app)[3],
            (4, "Pattern 4".to_owned(), 1, 0),
            "past the last"
        );
        assert_eq!(app.pattern, Some(PatternId(4)));
        assert_eq!(app.session.mode(), PlanMode::Pattern(PatternId(4)));
        let _ = app.update(Message::Undo);
        assert_eq!(patterns(&app).len(), 3);
        assert_eq!(app.pattern, Some(PatternId(3)));
        assert_eq!(app.session.mode(), PlanMode::Pattern(PatternId(3)));
        let _ = app.update(Message::Redo);
        assert_eq!(patterns(&app).len(), 4);
    }

    // Its twin in song mode: walking leaves blank Pattern 3 a pattern, and past the last the arrow
    // makes a blank take of one bar named after the selected channel, "kick take 1", which it
    // selects while the song stays what plays; one undo step. With no channel selected the take is
    // "Take 4" (its ID).
    #[test]
    fn next_past_the_last_pattern_in_song_mode_makes_a_new_take() {
        let mut app = fixture_app("p2_three_patterns.json");
        let _ = app.update(Message::ModeSelected(PlayMode::Song));
        send(&mut app, PatternMessage::Next);
        send(&mut app, PatternMessage::Next);
        assert_eq!(app.pattern, Some(PatternId(3)));
        let kind = |app: &App, id: u32| app.session.project().patterns[&PatternId(id)].kind;
        assert_eq!(
            kind(&app, 3),
            PatternKind::Pattern,
            "walking changes nothing"
        );
        send(&mut app, PatternMessage::Next);
        assert_eq!(
            patterns(&app)[3],
            (4, "kick take 1".to_owned(), 1, 0),
            "past the last"
        );
        assert_eq!(kind(&app, 4), PatternKind::Take);
        assert_eq!(app.pattern, Some(PatternId(4)));
        assert_eq!(app.session.mode(), PlanMode::Song);
        let _ = app.update(Message::Undo);
        assert_eq!(patterns(&app).len(), 3);
        assert_eq!(app.pattern, Some(PatternId(3)));

        app.selected_channel = None;
        send(&mut app, PatternMessage::Next);
        assert_eq!(patterns(&app)[3], (4, "Take 4".to_owned(), 1, 0));
        assert_eq!(kind(&app, 4), PatternKind::Take);
    }

    // A right press on the pattern's name opens its menu; Rename turns the name into a text box
    // and Enter renames the pattern (one undo step), Esc leaves it as it was.
    #[test]
    fn the_name_menu_renames_the_pattern() {
        let mut app = fixture_app("p2_three_patterns.json");
        send(&mut app, PatternMessage::ToggleMenu);
        assert!(app.patterns.menu);
        let _ = app.update(Message::ClearSelection);
        assert!(!app.patterns.menu, "Esc closes the menu");
        send(&mut app, PatternMessage::ToggleMenu);
        send(&mut app, PatternMessage::Rename);
        assert!(!app.patterns.menu);
        assert_eq!(
            app.patterns.rename,
            Some((PatternId(1), "Pattern 1".to_owned()))
        );
        send(
            &mut app,
            PatternMessage::RenameEdited("  Verse ".to_owned()),
        );
        send(&mut app, PatternMessage::RenameSubmitted);
        assert_eq!(app.patterns.rename, None);
        assert_eq!(patterns(&app)[0].1, "Verse", "trimmed");
        let _ = app.update(Message::Undo);
        assert_eq!(patterns(&app)[0].1, "Pattern 1");

        send(&mut app, PatternMessage::Rename);
        send(&mut app, PatternMessage::RenameEdited("Chorus".to_owned()));
        let _ = app.update(Message::ClearSelection);
        assert_eq!(app.patterns.rename, None);
        assert_eq!(patterns(&app)[0].1, "Pattern 1", "Esc renames nothing");
        send(&mut app, PatternMessage::Rename);
        send(&mut app, PatternMessage::RenameEdited("   ".to_owned()));
        send(&mut app, PatternMessage::RenameSubmitted);
        assert_eq!(patterns(&app)[0].1, "Pattern 1", "a blank name is no name");

        // A message that closes the menus, here a step pressed, ends a rename as Esc does.
        send(&mut app, PatternMessage::Rename);
        send(&mut app, PatternMessage::RenameEdited("Bridge".to_owned()));
        let _ = app.update(Message::StepClicked {
            channel: dawsome_engine::project::ChannelId(1),
            step: 3,
        });
        assert_eq!(app.patterns.rename, None);
        send(&mut app, PatternMessage::RenameSubmitted);
        assert_eq!(patterns(&app)[0].1, "Pattern 1");
    }

    // Delete takes the pattern and every clip of it in one undo step and selects the pattern
    // before it, or the first; the only pattern is refused, with the reason in the toolbar.
    #[test]
    fn the_name_menu_deletes_the_pattern_with_its_clips() {
        let mut app = fixture_app("p4_two_clips.json");
        let before = app.session.project().clone();
        send(&mut app, PatternMessage::ToggleMenu);
        send(&mut app, PatternMessage::Delete);
        assert!(!app.patterns.menu);
        let ids: Vec<u32> = patterns(&app).iter().map(|pattern| pattern.0).collect();
        assert_eq!(ids, [2, 3]);
        assert!(app.session.project().playlist.clips.is_empty());
        assert_eq!(app.pattern, Some(PatternId(2)), "none before 1: the first");
        assert_eq!(app.session.mode(), PlanMode::Pattern(PatternId(2)));
        let _ = app.update(Message::Undo);
        assert_eq!(app.session.project(), &before, "one undo brings both back");
        let clips: Vec<ClipId> = app
            .session
            .project()
            .playlist
            .clips
            .keys()
            .copied()
            .collect();
        assert_eq!(clips, [ClipId(1), ClipId(2)]);

        let _ = app.update(Message::PatternSelected(PatternId(3)));
        send(&mut app, PatternMessage::Delete);
        assert_eq!(app.pattern, Some(PatternId(2)), "the one before");

        let mut only = fixture_app("f1_four_on_floor.json");
        let before = only.session.project().clone();
        send(&mut only, PatternMessage::Delete);
        assert_eq!(only.session.project(), &before);
        assert_eq!(
            only.error.as_deref(),
            Some("edit refused: the only pattern cannot be deleted")
        );
    }
}
