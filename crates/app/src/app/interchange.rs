//! File → Export song as MIDI… and Export DAWproject…: the desktop's Save dialog, then the media
//! crate's writers, as the command line's `export-midi` and `export-dawproject` run them. Every
//! interchange export (stems too, from the export dialog) ends on its report: a dialog naming
//! what did not carry over, which Esc or OK closes, and the same text written beside the export
//! (`<file>.report.txt`).

use std::path::Path;

use dawsome_media::export::{self, Report};
use iced::Task;

use super::files::{blocking, pick_save, with_extension, FileMessage, Modal};
use super::{App, Message};
use crate::project_file;

impl App {
    /// File → Export song as MIDI…: the desktop's Save dialog, suggesting the document's name.
    pub(super) fn export_song_midi(&mut self) -> Task<Message> {
        if !self.host.background {
            return Task::none();
        }
        let dialog = pick_save(
            "Export song as MIDI",
            self.document_dir(),
            format!("{}.mid", self.document_stem()),
            ("MIDI file (*.mid)", "mid"),
        );
        Task::perform(dialog, |path| {
            Message::File(FileMessage::SongMidiPicked(path))
        })
    }

    /// Writes the song as one MIDI file to `path` (`.mid` added when it has no extension),
    /// atomically, then shows its report.
    pub(super) fn song_midi_picked(&mut self, path: &Path) {
        let path = with_extension(path, "mid");
        if let Some(refused) = self.refusal(&path) {
            self.notice = Some(refused);
            return;
        }
        let written = export::song_midi(self.session.project())
            .map_err(|error| error.to_string())
            .and_then(|(bytes, lines)| {
                project_file::write_atomically(&path, &bytes)
                    .map(|()| lines)
                    .map_err(|error| error.to_string())
            });
        match written {
            Ok(lines) => self.show_export_report(&Report {
                exported: path,
                lines,
            }),
            Err(error) => {
                self.notice = Some(format!("Cannot export {}: {error}", path.display()));
            }
        }
    }

    /// File → Export DAWproject…: the desktop's Save dialog, suggesting the document's name.
    pub(super) fn export_dawproject(&mut self) -> Task<Message> {
        if !self.host.background {
            return Task::none();
        }
        let dialog = pick_save(
            "Export DAWproject",
            self.document_dir(),
            format!("{}.dawproject", self.document_stem()),
            ("DAWproject (*.dawproject)", "dawproject"),
        );
        Task::perform(dialog, |path| {
            Message::File(FileMessage::DawprojectPicked(path))
        })
    }

    /// Packs the document as a DAWproject file at `path` (`.dawproject` added when it has no
    /// extension) on a thread of its own, since it copies the audio clips' files (here, when
    /// nothing runs in the background); its report shows when it is written.
    pub(super) fn dawproject_picked(&mut self, path: &Path) -> Task<Message> {
        let path = with_extension(path, "dawproject");
        if let Some(refused) = self.refusal(&path) {
            self.notice = Some(refused);
            return Task::none();
        }
        // The plugins' states as they are now, tweaks too big for an undo step included.
        self.capture_big_states();
        let project = self.session.project().clone();
        let base = self.session.samples_dir().to_path_buf();
        let title = self.document_stem();
        let work = move || {
            export::dawproject::write(&project, &base, &title, &path)
                .map(|lines| Report {
                    exported: path.clone(),
                    lines,
                })
                .map_err(|error| format!("Cannot export {}: {error}", path.display()))
        };
        if !self.host.background {
            self.dawproject_written(work());
            return Task::none();
        }
        Task::perform(blocking(work), |result| {
            let result = result.unwrap_or_else(|| Err("The DAWproject export stopped".to_owned()));
            Message::File(FileMessage::DawprojectWritten(result))
        })
    }

    /// A DAWproject export is written: its report, or why it failed.
    pub(super) fn dawproject_written(&mut self, result: Result<Report, String>) {
        match result {
            Ok(report) => self.show_export_report(&report),
            Err(error) => self.notice = Some(error),
        }
    }

    /// An export's report, written beside what it wrote and shown in a dialog; when another
    /// dialog is up (the export ran in the background meanwhile), a notice says where the
    /// report is instead, so no question on screen is lost.
    pub(super) fn show_export_report(&mut self, report: &Report) {
        let path = report.path();
        if let Err(error) = report.write() {
            self.notice = Some(format!(
                "Exported {}; cannot write the report {}: {error}",
                report.exported.display(),
                path.display()
            ));
        }
        if self.files.modal.is_some() {
            self.notice = Some(format!(
                "Exported {}; what did not carry over is in {}",
                report.exported.display(),
                path.display()
            ));
            return;
        }
        let groups = if report.lines.is_empty() {
            Vec::new()
        } else {
            vec![("Not carried over", report.lines.clone())]
        };
        self.files.modal = Some(Modal::Report {
            summary: format!("Exported {}", report.exported.display()),
            groups,
        });
    }

    /// Why an export may not write `out`: it is the document's file, or one of its sounds.
    fn refusal(&self, out: &Path) -> Option<String> {
        let project = self.session.project();
        let samples = dawsome_media::input_files(project, self.session.samples_dir());
        let inputs = self.document.path.clone().into_iter().chain(samples);
        dawsome_media::overwritten_input(out, inputs).map(|path| {
            format!(
                "Refusing to write {}: it is also an input file of this project",
                path.display()
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use dawsome_engine::project::Channel;
    use dawsome_media::BitDepth;
    use serde_json::Value;

    use super::super::tests::app_on;
    use super::*;
    use crate::app::{ExportKind, ExportMessage};

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    fn fixture(name: &str) -> PathBuf {
        Path::new(FIXTURES).join(name)
    }

    fn temp_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("dawsome-app-interchange-{}", std::process::id()))
            .join(test);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn expected() -> Value {
        let text = fs::read_to_string(fixture("p18_expected.json")).expect("p18_expected.json");
        serde_json::from_str(&text).expect("parses")
    }

    fn strings(value: &Value) -> Vec<String> {
        value
            .as_array()
            .expect("a list")
            .iter()
            .map(|line| line.as_str().expect("a string").to_owned())
            .collect()
    }

    fn send(app: &mut App, message: FileMessage) {
        let _ = app.update(Message::File(message));
    }

    /// The report on screen: its summary and its lines.
    fn report_shown(app: &App) -> (String, Vec<String>) {
        match &app.files.modal {
            Some(Modal::Report { summary, groups }) => {
                let lines = groups
                    .iter()
                    .flat_map(|(title, lines)| {
                        assert_eq!(*title, "Not carried over");
                        lines.clone()
                    })
                    .collect();
                (summary.clone(), lines)
            }
            _ => panic!("no report on screen"),
        }
    }

    /// The report beside `export`, after its first line, as the dialog lists it.
    fn report_beside(export: &Path) -> Vec<String> {
        let mut name = export.as_os_str().to_owned();
        name.push(".report.txt");
        let text = fs::read_to_string(&name).expect("the report beside the export");
        let mut lines = text.lines();
        assert_eq!(
            lines.next(),
            Some(format!("Exported {}", export.display()).as_str())
        );
        assert_eq!(lines.next(), Some("Not carried over:"));
        lines
            .map(|line| line.strip_prefix("  ").expect("two spaces in").to_owned())
            .collect()
    }

    // Known answer E1 through the menu: p18_song_repeat's song exports as the bytes written out
    // by hand in gen_p18_expected.py; its report is on screen and beside the file, and Esc
    // closes it.
    #[test]
    fn the_song_exports_as_midi_and_its_report_shows() {
        let dir = temp_dir("song_midi");
        let (mut app, _player) = app_on(&fixture("p18_song_repeat.json"));
        send(
            &mut app,
            FileMessage::SongMidiPicked(Some(dir.join("song"))),
        );
        let out = dir.join("song.mid");
        assert_eq!(
            fs::read(&out).expect("written"),
            fs::read(fixture("p18_song_repeat.mid")).expect("fixture")
        );
        let want = strings(&expected()["midi"]["repeat"]["report"]);
        let (summary, lines) = report_shown(&app);
        assert_eq!(summary, format!("Exported {}", out.display()));
        assert_eq!(lines, want);
        assert_eq!(report_beside(&out), want);
        let _ = app.update(Message::ClearSelection);
        assert!(app.files.modal.is_none(), "Esc closes the report");
    }

    // Known answer E3 through the menu: p18_daw_mix packs the hand-written project.xml (the
    // entries are stored, so its bytes stand in the package as they are), and the Library's
    // preview channel, which the plans play beside the document, is never in it.
    #[test]
    fn a_dawproject_export_holds_the_document_and_not_the_library_preview() {
        let dir = temp_dir("dawproject");
        let (mut app, _player) = app_on(&fixture("p18_daw_mix.json"));
        app.session
            .set_preview(Some(Channel::sampler("Preview kick", "impulse.wav")))
            .expect("the preview compiles");
        send(
            &mut app,
            FileMessage::DawprojectPicked(Some(dir.join("mix"))),
        );
        let out = dir.join("mix.dawproject");
        let package = fs::read(&out).expect("written");
        let document = fs::read(fixture("p18_daw_mix.project.xml")).expect("fixture");
        assert!(
            package
                .windows(document.len())
                .any(|window| window == document.as_slice()),
            "the package holds the hand-written project.xml"
        );
        let preview = b"Preview kick";
        assert!(!package.windows(preview.len()).any(|w| w == preview));
        let want = strings(&expected()["dawproject"]["mix"]["report"]);
        assert_eq!(report_shown(&app).1, want);
        assert_eq!(report_beside(&out), want);
    }

    // Known answer E2 through the export dialog: p18_stems at 16 bits into
    // "<dir>/p18_stems stems/": a file a playlist track and the master, every one the song's
    // 176400 frames; then the report, naming the loop left out and the muted track.
    #[test]
    fn stems_export_from_the_dialog_and_end_on_their_report() {
        let dir = temp_dir("stems");
        let (mut app, _player) = app_on(&fixture("p18_stems.json"));
        let form = |message| FileMessage::ExportForm(message);
        send(&mut app, FileMessage::Export(ExportKind::Stems));
        send(&mut app, form(ExportMessage::Depth(BitDepth::Int16)));
        send(&mut app, form(ExportMessage::Start));
        send(&mut app, form(ExportMessage::Picked(Some(dir.clone()))));
        let want = &expected()["stems"];
        let folder = dir.join(want["folder"].as_str().expect("folder"));
        let mut names: Vec<String> = fs::read_dir(&folder)
            .expect("the stems folder")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        let mut files: Vec<String> = want["files"]
            .as_array()
            .expect("files")
            .iter()
            .map(|file| file[0].as_str().expect("name").to_owned())
            .collect();
        files.sort();
        assert_eq!(names, files);
        for name in &names {
            let reader = hound::WavReader::open(folder.join(name)).expect("a WAV file");
            assert_eq!(reader.spec().bits_per_sample, 16, "{name}");
            assert_eq!(u64::from(reader.duration()), want["frames"], "{name}");
        }
        let lines = strings(&want["report"]);
        let (summary, shown) = report_shown(&app);
        assert_eq!(summary, format!("Exported {}", folder.display()));
        assert_eq!(shown, lines);
        assert_eq!(report_beside(&folder), lines);
        // Nine files of two bars: not left in the system's temporary folder.
        let _ = fs::remove_dir_all(&dir);
    }

    // A song without a clip has nothing to split: the menu says so and opens no dialog.
    #[test]
    fn stems_need_a_clip_on_the_playlist() {
        let (mut app, _player) = app_on(&fixture("p3_roll.json"));
        send(&mut app, FileMessage::Export(ExportKind::Stems));
        assert!(app.files.modal.is_none());
        assert_eq!(
            app.notice.as_deref(),
            Some("Export stems needs a song: the playlist has no clip")
        );
    }

    // An export never writes over the document's own file.
    #[test]
    fn an_export_over_the_document_is_refused() {
        let dir = temp_dir("refused");
        let project = dir.join("song.json");
        fs::copy(fixture("p18_song_repeat.json"), &project).expect("copy");
        let (mut app, _player) = app_on(&project);
        let before = fs::read(&project).expect("read");
        send(&mut app, FileMessage::SongMidiPicked(Some(project.clone())));
        assert_eq!(fs::read(&project).expect("read"), before);
        let notice = app.notice.clone().unwrap_or_default();
        assert!(notice.starts_with("Refusing to write"), "{notice}");
        assert!(app.files.modal.is_none());
    }
}
