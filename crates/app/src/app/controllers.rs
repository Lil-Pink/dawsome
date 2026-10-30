//! MIDI controllers in the window: the controller moves the MIDI inputs queue are taken on a
//! timer and move the parameters linked to them (see [`crate::links`]), and a right press on a
//! control offers "Link to controller…": the next controller moved is then linked to it, with
//! the MIDI channel it came on, as one undo step. A linked control is unlinked from the same
//! menu. Bursts of moves are timed by the wall clock at the project's rate, since a controller
//! moves parameters whether or not anything plays.

use std::time::{Duration, Instant};

use dawsome_engine::edit::Edit;
use dawsome_engine::project::{AutomationTarget, Controller, MAX_CONTROLLER};

use super::App;
use crate::midi::ControllerMove;
use crate::session::SessionError;

/// How often the controller moves waiting are taken while anything listens for them.
pub(super) const CONTROLLER_PERIOD: Duration = Duration::from_millis(16);

/// What the toolbar says while a control waits for its controller.
pub const LEARNING: &str = "Move a control on your MIDI device\u{2026} (Esc cancels)";

/// The window's side of controller links.
#[derive(Debug)]
pub(super) struct Controllers {
    /// The control waiting for the next controller moved, to be linked to it.
    pub learning: Option<AutomationTarget>,
    /// When the window started: the wall clock bursts of moves are timed by.
    started: Instant,
}

impl Default for Controllers {
    fn default() -> Self {
        Self {
            learning: None,
            started: Instant::now(),
        }
    }
}

impl App {
    /// The automation menu's link row: unlinks a linked control, or waits for the controller
    /// to link to it.
    pub(super) fn link_chosen(&mut self) {
        let Some(menu) = self.automate.take() else {
            return;
        };
        if self
            .session
            .project()
            .midi_links
            .get(&menu.target)
            .is_some()
        {
            self.commit(Edit::SetMidiLink {
                target: menu.target,
                link: None,
            });
            return;
        }
        self.controllers.learning = Some(menu.target);
        self.notice = Some(LEARNING.to_owned());
    }

    /// Esc while a control waits for its controller: it stops waiting. Returns whether one was.
    pub(super) fn cancel_learning(&mut self) -> bool {
        if self.controllers.learning.take().is_none() {
            return false;
        }
        self.notice = None;
        true
    }

    /// Takes the controller moves waiting: the first links a control waiting for one, the
    /// rest move what they are linked to; then the bursts that have been still end.
    pub(super) fn poll_controllers(&mut self) {
        let mut any = false;
        while let Some(moved) = self.controller_queue.pop() {
            self.controller_moved(moved);
            any = true;
        }
        if any {
            // The timer leaves the panels as drawn; what the moves changed is drawn anew.
            self.clear_panel_caches();
        }
        let now = self.controller_clock();
        let rate = self.session.project().sample_rate;
        let settled = self.links.settle(&mut self.session, now, rate);
        self.after_settle(settled);
    }

    fn controller_moved(&mut self, moved: ControllerMove) {
        if self.controllers.learning.is_some() && moved.cc <= MAX_CONTROLLER {
            let Some(target) = self.controllers.learning.take() else {
                return;
            };
            self.notice = None;
            let link = Controller {
                cc: moved.cc,
                channel: Some(moved.channel),
            };
            self.commit(Edit::SetMidiLink {
                target,
                link: Some(link),
            });
            return;
        }
        let now = self.controller_clock();
        let result = self.links.moved(&mut self.session, moved, now);
        self.after_edit(result);
    }

    /// Ends every burst of controller moves now: Stop, a step through the history, a save, a
    /// new document or the window closing.
    pub(super) fn settle_controllers(&mut self) {
        let settled = self.links.settle_all(&mut self.session);
        self.after_settle(settled);
    }

    /// Captures the state of each plugin a burst ended on, and shows what failed.
    fn after_settle(
        &mut self,
        settled: Result<Vec<dawsome_engine::project::ProcessorOwner>, SessionError>,
    ) {
        match settled {
            Ok(plugins) => {
                for owner in plugins {
                    self.capture_soon(owner);
                }
            }
            Err(error) => self.report(Err(error)),
        }
    }

    /// Frames of the project's rate since the window started, by the wall clock.
    fn controller_clock(&self) -> u64 {
        let seconds = self.controllers.started.elapsed().as_secs_f64();
        (seconds * f64::from(self.session.project().sample_rate)) as u64
    }

    /// Whether controller moves are listened for: a control waits for one, a burst waits to
    /// end, or moves wait in the queue. Why the queue: a move's one wake can be a tick a dialog
    /// drops, and then only the timer takes the move once the dialog closes.
    pub(super) fn controllers_busy(&self) -> bool {
        self.controllers.learning.is_some()
            || !self.links.is_still()
            || !self.controller_queue.is_empty()
    }

    /// The automation menu's link row for `target`.
    pub(super) fn link_row_label(&self, target: &AutomationTarget) -> &'static str {
        if self.session.project().midi_links.get(target).is_some() {
            "Unlink controller"
        } else {
            "Link to controller\u{2026}"
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use dawsome_engine::project::ChannelId;

    use super::super::{FileMessage, Message, Panel};
    use super::*;
    use crate::midi::Router;
    use crate::ui::rack::Knob;

    const F1: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/f1_four_on_floor.json"
    );

    fn kick_volume_menu(app: &mut App) {
        let _ = app.update(Message::KnobMenu {
            channel: ChannelId(1),
            knob: Knob::Volume,
            at: iced::Point::new(182.0, 30.0),
        });
        assert_eq!(
            app.automate.as_ref().map(|menu| menu.panel),
            Some(Panel::Rack)
        );
    }

    // Learn: the menu's row waits for a controller; the next one moved (cc 7 on MIDI channel
    // 3, status 0xB2) is linked to the kick's volume with its channel, as exactly one undo
    // step that leaves the volume as it was; the row then unlinks, and one undo of the link
    // takes it away again.
    #[test]
    fn the_next_controller_moved_links_to_the_control_in_one_undo_step() {
        let (mut app, _player) = super::super::tests::app_on(Path::new(F1));
        let target = AutomationTarget::ChannelVolume(ChannelId(1));
        let device = Router::new(app.session.midi_inbox(), app.controller_queue.clone());
        kick_volume_menu(&mut app);
        assert_eq!(app.link_row_label(&target), "Link to controller\u{2026}");
        let _ = app.update(Message::LinkChosen);
        assert_eq!(app.notice.as_deref(), Some(LEARNING));
        let steps = app.session.history().undo_len();
        device.deliver(&[0xB2, 7, 100]);
        let _ = app.update(Message::ControllerTick);
        let linked = Controller {
            cc: 7,
            channel: Some(2),
        };
        assert_eq!(app.session.project().midi_links.get(&target), Some(linked));
        assert_eq!(app.session.history().undo_len(), steps + 1);
        assert_eq!(
            app.session.project().channels[&ChannelId(1)].volume,
            1.0,
            "the learning move moves nothing"
        );
        assert_eq!(app.notice, None);
        kick_volume_menu(&mut app);
        assert_eq!(app.link_row_label(&target), "Unlink controller");
        let _ = app.update(Message::ClearSelection);
        let _ = app.update(Message::Undo);
        assert!(app.session.project().midi_links.is_empty());
    }

    // A dialog lets no controller tick through, so a move made while one is up waits in the
    // queue; that move's one wake was the tick the dialog dropped. Once the dialog closes, the
    // controller timer runs for the move still waiting, and its next tick moves the linked
    // kick volume to the top of its 0..4 (cc 7 = 127).
    #[test]
    fn a_move_made_while_a_dialog_is_up_is_applied_once_it_closes() {
        let (mut app, _player) = super::super::tests::app_on(Path::new(F1));
        let device = Router::new(app.session.midi_inbox(), app.controller_queue.clone());
        kick_volume_menu(&mut app);
        let _ = app.update(Message::LinkChosen);
        device.deliver(&[0xB0, 7, 1]);
        let _ = app.update(Message::ControllerTick);
        let _ = app.update(Message::File(FileMessage::New));
        assert!(
            app.files.modal.is_some(),
            "the link left changes unsaved: a dialog asks"
        );
        device.deliver(&[0xB0, 7, 127]);
        let _ = app.update(Message::ControllerTick);
        let volume = |app: &App| app.session.project().channels[&ChannelId(1)].volume;
        assert_eq!(volume(&app), 1.0, "the dialog holds the move back");
        let _ = app.update(Message::ClearSelection);
        assert!(app.files.modal.is_none(), "Esc cancels the dialog");
        assert!(
            app.controllers_busy(),
            "the timer runs for the move waiting"
        );
        let _ = app.update(Message::ControllerTick);
        assert_eq!(volume(&app), 4.0);
    }

    // A linked controller moves its control live (cc 7 = 127 is the top of the volume's 0..4)
    // and Stop makes the burst one undo step; Esc stops a control waiting for a controller.
    #[test]
    fn a_linked_controller_moves_its_control_and_esc_cancels_learning() {
        let (mut app, _player) = super::super::tests::app_on(Path::new(F1));
        let device = Router::new(app.session.midi_inbox(), app.controller_queue.clone());
        kick_volume_menu(&mut app);
        let _ = app.update(Message::LinkChosen);
        device.deliver(&[0xB0, 7, 1]);
        let _ = app.update(Message::ControllerTick);
        let steps = app.session.history().undo_len();
        device.deliver(&[0xB0, 7, 127]);
        device.deliver(&[0xB1, 7, 0]);
        let _ = app.update(Message::ControllerTick);
        let volume = |app: &App| app.session.project().channels[&ChannelId(1)].volume;
        assert_eq!(volume(&app), 4.0, "MIDI channel 1 is not the link's 0");
        assert!(app.controllers_busy(), "the burst waits to end");
        let _ = app.update(Message::Stop);
        assert_eq!(app.session.history().undo_len(), steps + 1);
        assert!(!app.controllers_busy());
        kick_volume_menu(&mut app);
        let _ = app.update(Message::ClearSelection);
        let pan = AutomationTarget::ChannelPan(ChannelId(1));
        let _ = app.update(Message::KnobMenu {
            channel: ChannelId(1),
            knob: Knob::Pan,
            at: iced::Point::new(148.0, 30.0),
        });
        let _ = app.update(Message::LinkChosen);
        assert_eq!(app.controllers.learning, Some(pan));
        let _ = app.update(Message::ClearSelection);
        assert_eq!(app.controllers.learning, None);
        assert_eq!(app.notice, None);
    }
}
