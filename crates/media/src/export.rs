//! Interchange export: the song as one MIDI file, stems (a WAV file a playlist track, and the
//! master), DAWproject ([`dawproject`]), and the report each export writes beside its file,
//! naming what did not carry over. The command line and the app share it; the rules and their
//! known answers are in tests/fixtures/gen_p18_expected.py.

pub mod dawproject;
mod zip;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use dawsome_engine::project::{ChannelKind, LoopRange, Project};
use dawsome_engine::smf::{self, SmfError, SongFile};
use dawsome_engine::time::{self, Ticks};

/// What one export wrote and what it could not carry: shown after the export and written beside
/// what it wrote ([`Report::path`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The file, or the folder of files, the export wrote.
    pub exported: PathBuf,
    /// What did not carry over, a line each.
    pub lines: Vec<String>,
}

impl Report {
    /// "Exported <path>", then "Not carried over:" and each line two spaces in (or "Everything
    /// carried over."), each ended by a newline.
    pub fn text(&self) -> String {
        let mut text = format!("Exported {}\n", self.exported.display());
        if self.lines.is_empty() {
            text.push_str("Everything carried over.\n");
        } else {
            text.push_str("Not carried over:\n");
            for line in &self.lines {
                text.push_str("  ");
                text.push_str(line);
                text.push('\n');
            }
        }
        text
    }

    /// Where the report is written: `<exported>.report.txt`, beside what the export wrote.
    pub fn path(&self) -> PathBuf {
        let mut name = self.exported.clone().into_os_string();
        name.push(".report.txt");
        PathBuf::from(name)
    }

    /// Writes [`Report::text`] to [`Report::path`].
    pub fn write(&self) -> io::Result<PathBuf> {
        let path = self.path();
        fs::write(&path, self.text())?;
        Ok(path)
    }
}

/// "1 note", "2 notes".
fn count(count: u32, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// The first line of every MIDI report: what the format holds at all.
const MIDI_HOLDS: &str =
    "A MIDI file holds notes and the tempo: the sounds, the mixer and automation stay in Dawsome";

/// The song as one MIDI file ([`smf::export_song`]), and its report's lines.
pub fn song_midi(project: &Project) -> Result<(Vec<u8>, Vec<String>), SmfError> {
    let file = smf::export_song(project)?;
    let lines = midi_lines(project, &file);
    Ok((file.bytes, lines))
}

/// What the song's MIDI file leaves out: what the format holds, then each channel's (rack
/// order), then what the notes lost on the way.
fn midi_lines(project: &Project, file: &SongFile) -> Vec<String> {
    let mut lines = vec![MIDI_HOLDS.to_owned()];
    for (id, channel) in &project.channels {
        let name = &channel.name;
        match channel.kind {
            ChannelKind::Audio => lines.push(format!(
                "{name}: its audio clips are not in a MIDI file; its track is empty"
            )),
            ChannelKind::Input => lines.push(format!(
                "{name}: a live input is not in a MIDI file; its track is empty"
            )),
            ChannelKind::Sampler | ChannelKind::Plugin | ChannelKind::Instrument => {
                if channel.mute {
                    lines.push(format!("{name} is muted here; its notes are in the file"));
                }
            }
        }
        if let Some(&left) = file.not_notes.get(id) {
            lines.push(format!("{name}: {} left out", count(left, "note", "notes")));
        }
    }
    if file.notes == 0 {
        lines.push("No clip plays a note: the file holds the tempo alone".to_owned());
    }
    if file.muted_clips > 0 {
        let clips = count(file.muted_clips, "muted clip", "muted clips");
        lines.push(format!("{clips} left out"));
    }
    if file.shortened > 0 {
        let notes = count(
            file.shortened,
            "overlapping note shortened",
            "overlapping notes shortened",
        );
        lines.push(format!("{notes} where the same key starts again"));
    }
    if file.dropped > 0 {
        let notes = count(file.dropped, "note left out", "notes left out");
        lines.push(format!(
            "{notes} where another starts on the same tick and key"
        ));
    }
    lines
}

/// A stems export: a file a playlist track, then the master, each a project to render in song
/// mode for `bars` played bars.
#[derive(Debug, Clone, PartialEq)]
pub struct Stems {
    /// The song's end rounded up to a bar, as the song's export renders it unless told
    /// otherwise.
    pub bars: u32,
    pub files: Vec<StemFile>,
}

/// One file of a stems export: its name in the stems folder and what plays in it.
#[derive(Debug, Clone, PartialEq)]
pub struct StemFile {
    pub name: String,
    pub project: Project,
}

/// The name of the master's file in the stems folder.
pub const MASTER_STEM: &str = "master.wav";

/// The folder a stems export of the project named `name` writes into, inside `dir`.
pub fn stems_folder(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name} stems"))
}

/// The files of a stems export of `project`: for each playlist track in order, "NN name.wav"
/// (NN its place from 01, as many digits as the last needs and at least two; the name as
/// [`file_name_part`] makes it safe) playing the song with every pattern and audio clip of the
/// other tracks muted, then [`MASTER_STEM`], the song itself. Automation clips play in every
/// file (they shape the sound a track's clips make), a track's own mute is kept (its file is
/// silence), so every file is that track's part of the master. `None` for a song with no clip.
///
/// Every file plays from tick 0 to the song's end rounded up to a bar, through a loop over
/// exactly that range: the project's own loop range would play part of the song again, and the
/// song's end would wrap to its start inside the last bar; the render ends where the loop
/// would wrap, so nothing plays twice and the last notes ring on to the bar's end.
pub fn stems(project: &Project) -> Option<Stems> {
    let end = project.playlist.end().0;
    let bar = time::bars_to_ticks(1, project.ppq).0.max(1);
    if end <= 0 {
        return None;
    }
    // Both are above 0, so this is the end rounded up to a bar.
    let bars = u32::try_from((end + bar - 1) / bar).ok()?;
    let mut song = project.clone();
    song.playlist.loop_range = Some(LoopRange {
        start: Ticks(0),
        end: time::bars_to_ticks(bars, project.ppq),
        on: true,
    });
    let tracks = &project.playlist.tracks;
    let width = tracks.len().to_string().len().max(2);
    let mut files: Vec<StemFile> = tracks
        .iter()
        .enumerate()
        .map(|(place, (&id, track))| {
            let mut solo = song.clone();
            for clip in solo.playlist.clips.values_mut() {
                if clip.track != id && clip.automation().is_none() {
                    clip.muted = true;
                }
            }
            let number = format!("{:0width$}", place + 1);
            let part = file_name_part(&track.name);
            let name = if part.is_empty() {
                format!("{number}.wav")
            } else {
                format!("{number} {part}.wav")
            };
            StemFile {
                name,
                project: solo,
            }
        })
        .collect();
    files.push(StemFile {
        name: MASTER_STEM.to_owned(),
        project: song,
    });
    Some(Stems { bars, files })
}

/// `name` as part of a file name on every system the program runs on: `/ \ : * ? " < > |`
/// and control characters become `_`, and trailing spaces and dots go (Windows drops them).
pub fn file_name_part(name: &str) -> String {
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    safe.trim_end_matches([' ', '.']).to_owned()
}

/// What a stems export leaves out: the loop range, and what a render hears as silence.
pub fn stems_lines(project: &Project) -> Vec<String> {
    let mut lines = Vec::new();
    if project.playlist.loop_range.is_some_and(|range| range.on) {
        lines.push(
            "The loop range is left out: every file plays the song once from its start".to_owned(),
        );
    }
    for track in project.playlist.tracks.values().filter(|track| track.mute) {
        lines.push(format!("{} is muted, so its file is silence", track.name));
    }
    for channel in project.channels.values() {
        if channel.kind == ChannelKind::Input {
            lines.push(format!(
                "{} plays a live input, which a render hears as silence",
                channel.name
            ));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    // By hand: the characters no file system takes become _, trailing dots and spaces go.
    #[test]
    fn a_track_name_becomes_a_safe_file_name() {
        assert_eq!(file_name_part("Hats/Top"), "Hats_Top");
        assert_eq!(file_name_part("a\\b:c*d?e\"f<g>h|i"), "a_b_c_d_e_f_g_h_i");
        assert_eq!(file_name_part("tab\there"), "tab_here");
        assert_eq!(file_name_part("Lead. . "), "Lead");
        assert_eq!(file_name_part("Ünïcode ♪"), "Ünïcode ♪");
        assert_eq!(file_name_part("..."), "");
    }

    // By hand: the text names the export, then each line two spaces in; with no line it says
    // so; the report sits beside the export with ".report.txt" added to its whole name.
    #[test]
    fn a_report_names_the_export_and_each_line() {
        let report = Report {
            exported: PathBuf::from("/songs/a.mid"),
            lines: vec!["one".into(), "two".into()],
        };
        assert_eq!(
            report.text(),
            "Exported /songs/a.mid\nNot carried over:\n  one\n  two\n"
        );
        assert_eq!(report.path(), PathBuf::from("/songs/a.mid.report.txt"));
        let empty = Report {
            exported: PathBuf::from("b stems"),
            lines: Vec::new(),
        };
        assert_eq!(empty.text(), "Exported b stems\nEverything carried over.\n");
        assert_eq!(empty.path(), PathBuf::from("b stems.report.txt"));
    }
}
