//! Standard MIDI files from the File menu: "Import MIDI…" reads a file into the selected
//! pattern and channel (the channel's notes there replaced, the pattern grown to hold them) as
//! one undo step, and says what it left out; "Export pattern as MIDI…" writes the selected
//! channel's notes in the selected pattern. Both go through the engine's reader and writer,
//! as the command line's `smf-import` and `smf-export` do.

use std::path::{Path, PathBuf};

use dawsome_engine::smf;
use iced::Task;

use super::files::{pick_save, with_extension};
use super::{App, FileMessage, Message};
use crate::project_file;

impl App {
    /// File → Import MIDI…: the desktop's picker for a .mid file.
    pub(super) fn import_midi(&mut self) -> Task<Message> {
        if !self.host.background {
            return Task::none();
        }
        let dir = self.document_dir();
        Task::perform(pick_midi(dir), |path| {
            Message::File(FileMessage::MidiPicked(path))
        })
    }

    /// Reads the picked file into the selected pattern and channel.
    pub(super) fn midi_picked(&mut self, path: &Path) {
        let (Some(pattern), Some(channel)) = (self.pattern, self.selected_channel) else {
            self.notice = Some("Import MIDI needs a pattern and a channel".to_owned());
            return;
        };
        let read = std::fs::read(path).map_err(|error| error.to_string());
        let imported = read.and_then(|bytes| {
            smf::import_into(self.session.project(), pattern, channel, &bytes)
                .map_err(|error| error.to_string())
        });
        match imported {
            Ok(import) => {
                self.commit(import.edit);
                if self.error.is_none() {
                    self.notice = Some(import.report.summary());
                }
            }
            Err(error) => {
                self.notice = Some(format!("Cannot import {}: {error}", path.display()));
            }
        }
    }

    /// File → Export pattern as MIDI…: the desktop's Save dialog, suggesting the pattern's name.
    pub(super) fn export_midi(&mut self) -> Task<Message> {
        if !self.host.background {
            return Task::none();
        }
        let project = self.session.project();
        let name = self
            .pattern
            .and_then(|id| project.patterns.get(&id))
            .map_or_else(|| "Pattern".to_owned(), |pattern| pattern.name.clone());
        let dialog = pick_save(
            "Export pattern as MIDI",
            self.document_dir(),
            format!("{name}.mid"),
            ("MIDI file (*.mid)", "mid"),
        );
        Task::perform(dialog, |path| {
            Message::File(FileMessage::MidiExportPicked(path))
        })
    }

    /// Writes the selected channel's notes in the selected pattern to `path` (`.mid` added
    /// when it has no extension), atomically.
    pub(super) fn midi_export_picked(&mut self, path: &Path) {
        let (Some(pattern), Some(channel)) = (self.pattern, self.selected_channel) else {
            self.notice = Some("Export MIDI needs a pattern and a channel".to_owned());
            return;
        };
        let path = with_extension(path, "mid");
        let written = smf::export(self.session.project(), pattern, channel)
            .map_err(|error| error.to_string())
            .and_then(|bytes| {
                project_file::write_atomically(&path, &bytes).map_err(|error| error.to_string())
            });
        self.notice = Some(match written {
            Ok(()) => format!("Exported {}", path.display()),
            Err(error) => format!("Cannot export {}: {error}", path.display()),
        });
    }
}

/// The desktop's file picker for Import MIDI…, MIDI files only.
async fn pick_midi(dir: Option<PathBuf>) -> Option<PathBuf> {
    let mut dialog = rfd::AsyncFileDialog::new()
        .set_title("Import MIDI")
        .add_filter("MIDI files (*.mid, *.midi)", &["mid", "midi"]);
    if let Some(dir) = dir {
        dialog = dialog.set_directory(dir);
    }
    dialog
        .pick_file()
        .await
        .map(|file| file.path().to_path_buf())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use dawsome_engine::project::PatternId;

    use super::super::tests::app_on;
    use super::*;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    // Known answer (1) from p9_expected.json through the menu: p9_import.mid replaces the one
    // step of p9_import_base's channel 1 with four notes and grows the pattern to 2 bars, one
    // undo step; the notice is the report's line, which also names the file's 4/4 (its meta
    // event FF 58 04 04 02 18 08).
    #[test]
    fn import_midi_replaces_the_channels_notes_in_one_undo_step() {
        let (mut app, _player) = app_on(&Path::new(FIXTURES).join("p9_import_base.json"));
        let before = app.session.project().clone();
        let _ = app.update(Message::File(FileMessage::MidiPicked(Some(
            Path::new(FIXTURES).join("p9_import.mid"),
        ))));
        let pattern = &app.session.project().patterns[&PatternId(1)];
        let notes: Vec<(i64, i64, u8, f32)> = pattern
            .notes
            .iter()
            .map(|note| (note.start.0, note.length.0, note.key, note.velocity))
            .collect();
        assert_eq!(
            notes,
            [
                (0, 96, 60, 0.78125),
                (15, 96, 64, 0.5),
                (192, 24, 36, 0.992_187_5),
                (260, 220, 67, 0.625)
            ]
        );
        assert_eq!(pattern.length_bars, 2);
        assert_eq!(
            app.notice.as_deref(),
            Some(
                "4 notes imported; 1 overlapping note dropped, 1 note closed at track end, 3 \
                 other events ignored, tempo 120 BPM not applied, time signature 4/4 not \
                 applied"
            )
        );
        let _ = app.update(Message::Undo);
        assert_eq!(app.session.project(), &before);
    }

    // gen_p9_expected.py's known answer (2): the imported pattern exports as 85 bytes.
    #[test]
    fn export_midi_writes_the_selected_channels_notes() {
        let (mut app, _player) = app_on(&Path::new(FIXTURES).join("p9_import_base.json"));
        let _ = app.update(Message::File(FileMessage::MidiPicked(Some(
            Path::new(FIXTURES).join("p9_import.mid"),
        ))));
        let dir = std::env::temp_dir().join(format!("dawsome-midi-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let _ = app.update(Message::File(FileMessage::MidiExportPicked(Some(
            dir.join("pattern"),
        ))));
        let written = std::fs::read(dir.join("pattern.mid")).expect("written");
        let hex: String = written.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(
            hex,
            "4d546864000000060000000100604d54726b0000003f00ff510307a12000ff58040402180800ff03\
             095061747465726e203100903c640f404051803c400f40405190247f188024402c904350815c804340\
             00ff2f00"
        );
        std::fs::remove_dir_all(&dir).expect("removed");
    }

    #[test]
    fn a_file_that_is_no_midi_file_is_named_and_changes_nothing() {
        let (mut app, _player) = app_on(&Path::new(FIXTURES).join("p9_import_base.json"));
        let before = app.session.project().clone();
        let _ = app.update(Message::File(FileMessage::MidiPicked(Some(
            Path::new(FIXTURES).join("p9_import_base.json"),
        ))));
        assert_eq!(app.session.project(), &before);
        let notice = app.notice.clone().unwrap_or_default();
        assert!(notice.starts_with("Cannot import"), "{notice}");
    }

    // 37 bytes at PPQ 1: one note 0x0FFFFFFF quarters long, 67,108,864 bars. The import is
    // refused, the notice says why, and the pattern keeps its length.
    #[test]
    fn a_file_past_the_longest_song_is_refused_with_the_reason() {
        let (mut app, _player) = app_on(&Path::new(FIXTURES).join("p9_import_base.json"));
        let before = app.session.project().clone();
        let dir = std::env::temp_dir().join(format!("dawsome-midi-long-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("long.mid");
        let mut bytes = b"MThd\0\0\0\x06\0\0\0\x01\0\x01MTrk\0\0\0\x0f".to_vec();
        bytes.extend_from_slice(&[0x00, 0x90, 0x3C, 0x40, 0xFF, 0xFF, 0xFF, 0x7F]);
        bytes.extend_from_slice(&[0x80, 0x3C, 0x40, 0x00, 0xFF, 0x2F, 0x00]);
        std::fs::write(&file, &bytes).expect("written");
        let _ = app.update(Message::File(FileMessage::MidiPicked(Some(file.clone()))));
        assert_eq!(app.session.project(), &before);
        assert_eq!(
            app.notice.as_deref(),
            Some(
                format!(
                    "Cannot import {}: a note lies past bar 100000, the end of the longest song",
                    file.display()
                )
                .as_str()
            )
        );
        std::fs::remove_dir_all(&dir).expect("removed");
    }
}
