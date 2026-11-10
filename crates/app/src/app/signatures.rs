//! The time signature menu: where a pattern's own signature (from the pattern name's menu or
//! the rack header's readout) and a change of signature along the song (from a right press on
//! the playlist ruler's numbers) are chosen, and the typed box its "Other…" opens.

use dawsome_engine::edit::Edit;
use dawsome_engine::project::PatternId;
use dawsome_engine::time::{Ticks, TimeSignature};
use iced::widget::operation;
use iced::widget::Id;
use iced::{Point, Task};

use super::{App, Message};
use crate::ui::dialog::{SignatureMenuView, SignatureRow};
use crate::ui::layout::TOOLBAR_HEIGHT;
use crate::ui::playlist_layout::{HEADER_HEIGHT, TRACK_HEADER_WIDTH};

/// The id of the menu's typed box.
pub const SIGNATURE_MENU_INPUT: &str = "signature-menu-input";

/// The signatures the list offers after the first row: the ones songs are most often in.
pub const CHOICES: [(u8, u8); 6] = [(3, 4), (6, 8), (5, 4), (7, 8), (12, 8), (2, 4)];

/// What a signature chosen from the menu sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureTarget {
    /// Pattern `0`'s own signature.
    Pattern(PatternId),
    /// The song's signature from bar line `0` on: a change there (or, at tick 0, the
    /// project's own).
    Change(Ticks),
}

/// The menu, while it is open.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct SignatureMenu {
    pub target: SignatureTarget,
    /// Where it opens, window coordinates.
    pub at: Point,
    /// An existing change's menu first shows its value, Change… and Remove; Change… then
    /// shows the list a new change and a pattern open on.
    pub list: bool,
    /// The typed box "Other…" opened, and what is typed in it so far.
    pub typed: Option<String>,
}

/// What the menu is told.
#[derive(Debug, Clone, PartialEq)]
pub enum SignatureMessage {
    /// The pattern name menu's "Time signature…" or the rack header's readout: the list for
    /// the selected pattern.
    ForPattern,
    /// A right press on the playlist ruler's numbers at the bar line `at`, ruler point `place`.
    ForChange {
        at: Ticks,
        place: Point,
    },
    /// A row: the pattern's own signature, or with `None` the project's again; a change's.
    Chosen(Option<TimeSignature>),
    /// An existing change's "Change…": the list.
    Change,
    /// An existing change's "Remove".
    Remove,
    /// "Other…": the typed box.
    Other,
    Typed(String),
    /// Enter in the typed box.
    Submitted,
    /// A press beside the menu, or Esc.
    Close,
}

impl App {
    pub(super) fn signature_message(&mut self, message: SignatureMessage) -> Task<Message> {
        match message {
            SignatureMessage::ForPattern => {
                self.patterns.menu = false;
                if let Some(pattern) = self.pattern {
                    self.signature_menu = Some(SignatureMenu {
                        target: SignatureTarget::Pattern(pattern),
                        at: Point::new(self.toolbar().spread().pattern_name_x(), TOOLBAR_HEIGHT),
                        list: true,
                        typed: None,
                    });
                }
            }
            SignatureMessage::ForChange { at, place } => {
                let panel = self.arranger.panel();
                let existing = self.change_at(at).is_some();
                self.signature_menu = Some(SignatureMenu {
                    target: SignatureTarget::Change(at),
                    at: Point::new(
                        panel.x + place.x.max(TRACK_HEADER_WIDTH),
                        panel.y + HEADER_HEIGHT + place.y,
                    ),
                    list: !existing,
                    typed: None,
                });
            }
            SignatureMessage::Chosen(signature) => {
                if let Some(menu) = self.signature_menu.take() {
                    self.choose_signature(menu.target, signature);
                }
            }
            SignatureMessage::Change => {
                if let Some(menu) = &mut self.signature_menu {
                    menu.list = true;
                }
            }
            SignatureMessage::Remove => {
                if let Some(SignatureMenu {
                    target: SignatureTarget::Change(at),
                    ..
                }) = self.signature_menu.take()
                {
                    self.commit(Edit::RemoveSignatureChange { at });
                }
            }
            SignatureMessage::Other => {
                if let Some(menu) = &mut self.signature_menu {
                    menu.list = true;
                    menu.typed = Some(String::new());
                    let input = Id::new(SIGNATURE_MENU_INPUT);
                    return operation::focus(input);
                }
            }
            SignatureMessage::Typed(typed) => {
                if let Some(text) = self
                    .signature_menu
                    .as_mut()
                    .and_then(|menu| menu.typed.as_mut())
                {
                    *text = typed;
                }
            }
            SignatureMessage::Submitted => {
                let Some(menu) = &self.signature_menu else {
                    return Task::none();
                };
                let typed = menu.typed.clone().unwrap_or_default();
                match typed.parse::<TimeSignature>() {
                    Ok(signature) => {
                        let target = menu.target;
                        self.signature_menu = None;
                        self.choose_signature(target, Some(signature));
                    }
                    Err(reason) => self.notice = Some(reason.to_string()),
                }
            }
            SignatureMessage::Close => self.signature_menu = None,
        }
        Task::none()
    }

    /// The change of signature at tick `at`, if the song has one there.
    fn change_at(&self, at: Ticks) -> Option<TimeSignature> {
        self.session
            .project()
            .signature_changes
            .iter()
            .find(|change| change.at == at)
            .map(|change| change.signature)
    }

    /// Gives `target` `signature` (a pattern `None`: the project's), one undo step, when that
    /// changes it.
    fn choose_signature(&mut self, target: SignatureTarget, signature: Option<TimeSignature>) {
        let project = self.session.project();
        let edit = match target {
            SignatureTarget::Pattern(pattern) => project
                .patterns
                .get(&pattern)
                .filter(|shown| shown.time_signature != signature)
                .map(|_| Edit::SetPatternTimeSignature { pattern, signature }),
            // The first bar's meter is the project's own signature.
            SignatureTarget::Change(Ticks(0)) => signature
                .filter(|&signature| signature != project.time_signature)
                .map(|signature| Edit::SetTimeSignature { signature }),
            SignatureTarget::Change(at) => signature
                .filter(|&signature| self.change_at(at) != Some(signature))
                .map(|signature| Edit::SetSignatureChange { at, signature }),
        };
        if let Some(edit) = edit {
            self.commit(edit);
        }
    }

    /// The open menu as it shows: its rows, the current one ticked, and the typed box.
    pub(super) fn signature_menu_view(&self) -> Option<SignatureMenuView> {
        let menu = self.signature_menu.as_ref()?;
        let project = self.session.project();
        let pick = |signature| Message::Signature(SignatureMessage::Chosen(signature));
        let mut rows = Vec::new();
        match menu.target {
            SignatureTarget::Change(at) if !menu.list => {
                let shown = self
                    .change_at(at)
                    .map_or_else(String::new, |sig| sig.to_string());
                let bar = project.meter_map().bar_at(at).bar + 1;
                rows.push(SignatureRow::note(format!("{shown} from bar {bar}")));
                rows.push(SignatureRow::act(
                    "Change\u{2026}".to_owned(),
                    Message::Signature(SignatureMessage::Change),
                    false,
                ));
                rows.push(SignatureRow::act(
                    "Remove".to_owned(),
                    Message::Signature(SignatureMessage::Remove),
                    false,
                ));
            }
            target => {
                let now = match target {
                    SignatureTarget::Pattern(pattern) => project
                        .patterns
                        .get(&pattern)
                        .and_then(|shown| shown.time_signature),
                    SignatureTarget::Change(Ticks(0)) => Some(project.time_signature),
                    SignatureTarget::Change(at) => self.change_at(at),
                };
                let mut listed = Vec::new();
                match target {
                    SignatureTarget::Pattern(_) => rows.push(SignatureRow::act(
                        format!("Project\u{2019}s ({})", project.time_signature),
                        pick(None),
                        now.is_none(),
                    )),
                    SignatureTarget::Change(_) => listed.push(project.time_signature),
                }
                for (beats, unit) in CHOICES {
                    if let Some(signature) = TimeSignature::new(beats, unit)
                        .filter(|signature| !listed.contains(signature))
                    {
                        listed.push(signature);
                    }
                }
                for signature in listed {
                    rows.push(SignatureRow::act(
                        signature.to_string(),
                        pick(Some(signature)),
                        now == Some(signature),
                    ));
                }
                rows.push(SignatureRow::act(
                    "Other\u{2026}".to_owned(),
                    Message::Signature(SignatureMessage::Other),
                    false,
                ));
            }
        }
        Some(SignatureMenuView {
            at: menu.at,
            rows,
            typed: menu.typed.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use dawsome_engine::project::{ClipId, PatternId};
    use dawsome_engine::time::{Ticks, TimeSignature};
    use iced::{mouse, Point};

    use super::super::tests::app_on;
    use super::*;
    use crate::ui::arranger::PlaylistMessage;
    use crate::ui::hint::ClipRow;
    use crate::ui::playlist::{clip_rows, signature_differs};

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    fn app(name: &str) -> App {
        app_on(&Path::new(FIXTURES).join(name)).0
    }

    fn send(app: &mut App, message: SignatureMessage) {
        let _ = app.update(Message::Signature(message));
    }

    fn sig(beats: u8, unit: u8) -> TimeSignature {
        TimeSignature::new(beats, unit).expect("a signature")
    }

    fn changes(app: &App) -> Vec<(i64, String)> {
        app.session
            .project()
            .signature_changes
            .iter()
            .map(|change| (change.at.0, change.signature.to_string()))
            .collect()
    }

    /// A press on the playlist ruler's number band at tick `tick` (64 px a 4/4 bar, unscrolled:
    /// x = 120 + tick / 6), 8 px down.
    fn ruler(app: &mut App, tick: i64, button: mouse::Button) {
        let at = Point::new(120.0 + tick as f32 / 6.0 + 2.0, 8.0);
        let _ = app.update(Message::Playlist(PlaylistMessage::RulerPressed {
            at,
            button,
        }));
    }

    // The pattern name's menu opens the list for the selected pattern: "Project's (4/4)"
    // ticked, then 3/4, 6/8, 5/4, 7/8, 12/8, 2/4 and Other…. 3/4 gives pattern 1 its own (one
    // undo step "Pattern time signature"; the list closes); Other… typed "7/8" sets that; a
    // typed 3/2 is refused with the ranges and the box stays; the project's row gives it back.
    #[test]
    fn a_pattern_gets_its_own_signature_from_its_menu() {
        let mut app = app("p35_edit_b.json");
        app.select_pattern(PatternId(1));
        send(&mut app, SignatureMessage::ForPattern);
        let view = app.signature_menu_view().expect("open");
        assert_eq!(view.rows.len(), 8, "the project's, six signatures, Other…");
        send(&mut app, SignatureMessage::Chosen(Some(sig(3, 4))));
        assert!(app.signature_menu.is_none());
        let own = |app: &App| app.session.project().patterns[&PatternId(1)].time_signature;
        assert_eq!(own(&app), Some(sig(3, 4)));
        assert_eq!(app.session.project().patterns[&PatternId(1)].length_bars, 2);
        assert_eq!(
            app.session.history().undo_name(),
            Some("Pattern time signature")
        );
        assert_eq!(app.rack_header().signature, Some(sig(3, 4)));
        send(&mut app, SignatureMessage::ForPattern);
        send(&mut app, SignatureMessage::Other);
        send(&mut app, SignatureMessage::Typed("3/2".to_owned()));
        send(&mut app, SignatureMessage::Submitted);
        assert!(app.signature_menu.is_some(), "a wrong one keeps the box");
        assert!(app
            .notice
            .as_deref()
            .is_some_and(|notice| notice.contains("1 to 24 beats over 4, 8 or 16")));
        send(&mut app, SignatureMessage::Typed("7/8".to_owned()));
        send(&mut app, SignatureMessage::Submitted);
        assert_eq!(own(&app), Some(sig(7, 8)));
        send(&mut app, SignatureMessage::ForPattern);
        send(&mut app, SignatureMessage::Chosen(None));
        assert_eq!(own(&app), None);
        assert_eq!(app.rack_header().signature, None);
        let _ = app.update(Message::Undo);
        assert_eq!(own(&app), Some(sig(7, 8)));
    }

    // Known answers C 6 through the ruler: a right press on its numbers at tick 800 opens the
    // list for the bar line at or before it, 768; 3/4 adds the change there. A right press on
    // its flag shows "3/4 from bar 3", Change… and Remove; Remove takes it out; one undo puts it
    // back. In bar 1 the list sets the project's own signature.
    #[test]
    fn the_ruler_adds_changes_and_removes_them() {
        let mut app = app("p35_edit_c.json");
        ruler(&mut app, 800, mouse::Button::Right);
        let menu = app.signature_menu.clone().expect("open");
        assert_eq!(menu.target, SignatureTarget::Change(Ticks(768)));
        assert!(menu.list, "a new change opens on the list");
        send(&mut app, SignatureMessage::Chosen(Some(sig(3, 4))));
        assert_eq!(changes(&app), [(768, "3/4".to_owned())]);
        assert_eq!(
            app.session.history().undo_name(),
            Some("Time signature change")
        );
        ruler(&mut app, 768, mouse::Button::Right);
        let menu = app.signature_menu.clone().expect("open");
        assert!(!menu.list, "an existing change's menu");
        let view = app.signature_menu_view().expect("shown");
        assert_eq!(view.rows.len(), 3);
        send(&mut app, SignatureMessage::Change);
        send(&mut app, SignatureMessage::Chosen(Some(sig(6, 8))));
        assert_eq!(changes(&app), [(768, "6/8".to_owned())]);
        ruler(&mut app, 768, mouse::Button::Right);
        send(&mut app, SignatureMessage::Remove);
        assert!(changes(&app).is_empty());
        let _ = app.update(Message::Undo);
        assert_eq!(changes(&app), [(768, "6/8".to_owned())]);
        ruler(&mut app, 100, mouse::Button::Right);
        send(&mut app, SignatureMessage::Chosen(Some(sig(5, 4))));
        assert_eq!(app.session.project().time_signature, sig(5, 4));
    }

    // Known answers C 6 through the ruler: p35_meter's 3/4 flag at 768 dragged to tick 1300
    // lands on the bar line at or before it in the song without it (4/4: 1152), as the move's
    // edit puts it, and the later changes settle: 1344 up to 1440, 1920 down to 1728.
    #[test]
    fn a_flag_dragged_moves_its_change_and_the_later_ones_settle() {
        let mut app = app("p35_meter.json");
        ruler(&mut app, 768, mouse::Button::Left);
        let to = Point::new(120.0 + 1300.0 / 6.0, 8.0);
        let _ = app.update(Message::Playlist(PlaylistMessage::RulerDragged(to)));
        assert_eq!(
            app.arranger.moving_change(),
            Some((Ticks(768), Ticks(1152)))
        );
        let _ = app.update(Message::Playlist(PlaylistMessage::RulerReleased(to)));
        assert_eq!(
            changes(&app),
            [
                (1152, "3/4".to_owned()),
                (1440, "6/8".to_owned()),
                (1728, "7/8".to_owned())
            ]
        );
        assert_eq!(app.song_marker, Ticks(0), "a flag's drag is no seek");
    }

    // Known answers B 4: p35_pattern_34's clip 1 plays pattern 2 (3/4) where the song is 4/4:
    // its title says "Pattern 2 · 3/4" and its menu offers "Use the time signature here", which
    // gives pattern 2 the song's 4/4 (every clip of it follows); clip 2 (pattern 1, 4/4) offers
    // nothing of the kind.
    #[test]
    fn a_clip_in_another_meter_says_so_and_takes_the_songs() {
        let mut app = app("p35_pattern_34.json");
        let project = app.session.project();
        let meter = project.meter_map();
        assert_eq!(
            signature_differs(project, &meter, ClipId(1)),
            Some(sig(3, 4))
        );
        assert_eq!(signature_differs(project, &meter, ClipId(2)), None);
        let rows = clip_rows(project, ClipId(1));
        assert!(rows.contains(&(
            ClipRow::UseSignature,
            "Use the time signature here".to_owned(),
            true
        )));
        assert!(!clip_rows(project, ClipId(2))
            .iter()
            .any(|(row, ..)| *row == ClipRow::UseSignature));
        app.arranger.clip_menu = Some((ClipId(1), Point::ORIGIN));
        let _ = app.update(Message::Playlist(PlaylistMessage::ClipMenu(
            ClipRow::UseSignature,
        )));
        assert_eq!(
            app.session.project().patterns[&PatternId(2)].time_signature,
            Some(TimeSignature::COMMON)
        );
        let project = app.session.project();
        assert_eq!(
            signature_differs(project, &project.meter_map(), ClipId(1)),
            None
        );
    }
}
