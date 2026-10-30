//! The piano roll's note preview: drawing a note, pressing one, moving one onto another key and
//! pressing the key column each play that key on the selected channel through the live path, as
//! the typing keyboard does, for a short while. A switch in the roll's strip turns it off, and
//! the settings keep the choice.

use std::time::{Duration, Instant};

use dawsome_engine::edit::STEP_VELOCITY;
use dawsome_engine::project::MAX_KEY;
use dawsome_engine::transport::Control;
use iced::mouse;

use super::App;
use crate::ui::note_gesture::Target;
use crate::ui::piano_layout;
use crate::ui::roll::RollMessage;

/// How long a previewed note sounds before its note-off.
pub(super) const NOTE_LENGTH: Duration = Duration::from_millis(300);

/// How often the note sounding is looked at for its end.
pub(super) const PERIOD: Duration = Duration::from_millis(50);

/// The note the piano roll is previewing: its key, velocity and when it ends.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Sounding {
    key: u8,
    velocity: f32,
    until: Instant,
}

impl App {
    /// The key and velocity `message` previews, before the roll takes it: a plain left press
    /// on a note plays the note; on the key column or on empty grid (where it draws a note)
    /// the row's key at a step's velocity; a moved note crossing onto another key plays that
    /// key. Nothing while the preview is off, for any other message, or for a press the roll
    /// ignores (one during a held gesture, Ctrl's selection and box).
    pub(super) fn note_to_preview(
        &self,
        message: &RollMessage,
        target: Option<&Target>,
    ) -> Option<(u8, f32)> {
        if !self.settings.preview_notes() {
            return None;
        }
        let target = target?;
        let view = &self.roll.view;
        let key_at = |y: f32| {
            u8::try_from(view.key_at(y))
                .ok()
                .filter(|&key| key <= MAX_KEY)
        };
        match message {
            // A press on a note or a key plays it; a press on empty grid plays nothing until it
            // is let go of in place, which draws the note (a drag from it is a box).
            RollMessage::GridPressed {
                at,
                button: mouse::Button::Left,
                double: false,
            } if !self.modifiers.command() && !self.roll.grid_held() => {
                let under = piano_layout::note_under(view, target.channel_notes(), at.x, at.y);
                match under {
                    Some((note, _)) => Some((note.key, note.velocity)),
                    // The key column plays its key.
                    None if at.x < piano_layout::KEY_COLUMN_WIDTH => {
                        key_at(at.y).map(|key| (key, STEP_VELOCITY))
                    }
                    None => None,
                }
            }
            RollMessage::GridReleased(_) => {
                let at = self.roll.pending_draw()?;
                key_at(at.y).map(|key| (key, STEP_VELOCITY))
            }
            RollMessage::GridDragged(at) if self.roll.moving() => {
                let key = key_at(at.y)?;
                let sounding = self.note_preview?;
                (key != sounding.key).then_some((key, sounding.velocity))
            }
            _ => None,
        }
    }

    /// Plays `key` at `velocity` on the selected channel for [`NOTE_LENGTH`], ending the note
    /// previewed before.
    pub(super) fn preview_note(&mut self, key: u8, velocity: f32) {
        self.end_note_preview();
        let result = self.session.send(Control::LiveNoteOn { key, velocity });
        if result.is_ok() {
            // A preview is not playing: Capture leaves it out.
            self.capture.preview(key);
            self.note_preview = Some(Sounding {
                key,
                velocity,
                until: Instant::now() + NOTE_LENGTH,
            });
        }
        self.report(result);
    }

    /// Ends the note previewed, if one sounds.
    pub(super) fn end_note_preview(&mut self) {
        if let Some(sounding) = self.note_preview.take() {
            let result = self
                .session
                .send(Control::LiveNoteOff { key: sounding.key });
            self.report(result);
        }
    }

    /// The preview timer: the note previewed ends once its time is up.
    pub(super) fn note_preview_tick(&mut self) {
        if self
            .note_preview
            .is_some_and(|sounding| Instant::now() >= sounding.until)
        {
            self.end_note_preview();
        }
    }

    /// The strip's switch: the preview on or off, kept in the settings.
    pub(super) fn toggle_note_preview(&mut self) {
        let on = !self.settings.preview_notes();
        self.settings.set_preview_notes(on);
        if !on {
            self.end_note_preview();
        }
        self.save_settings();
    }
}
