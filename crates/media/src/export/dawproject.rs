//! DAWproject export: the open project format of Bitwig and PreSonus (read by Bitwig Studio,
//! Studio One and Cubase), a zip of project.xml, metadata.xml, the audio clips' files under
//! `audio/` and each CLAP instrument's saved state under `plugins/`. The documents follow the
//! format's 1.0 schema (vendored in tests/fixtures/dawproject/); the rules and their known
//! answers are in tests/fixtures/gen_p18_expected.py.
//!
//! One Track per rack channel (content notes, or audio for an audio or input channel), each
//! with a Channel (volume as linear gain, pan normalized, mute) into a master track. The
//! Arrangement's Lanes run in beats, a Lanes per channel that has clips: a pattern clip becomes
//! a Notes clip on each channel it has notes for, unrolled as playback plays it
//! ([`dawsome_engine::song`]); an audio clip becomes a Warps clip, its file in seconds warped
//! onto beats at the tempo (onto its stretched length when stretched), its offset as playStart.
//! A clip that does not play (muted, or its track muted) is carried with enable="false".
//!
//! Why a track per channel rather than per playlist track: a Dawsome pattern plays notes for
//! many channels, and a DAWproject track has one instrument; only per channel do the notes reach
//! the sound they were written for.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, BufWriter};
use std::path::{Path, PathBuf};

use base64::Engine as _;
use dawsome_engine::project::{
    ChannelId, ChannelKind, Clip, ClipId, ClipItem, InsertId, Mixer, ProcessorRef, Project,
    StretchMode, MAX_VOLUME,
};
use dawsome_engine::song::{self, PlayedNote};
use dawsome_engine::time;

use super::file_name_part;
use super::zip::ZipWriter;
use crate::{decode, paths};

/// Every XML document of the package starts with this line.
const DECLARATION: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
/// The master's Channel, which every track's Channel plays into.
const MASTER: &str = "master";

/// An audio channel's file, as the package carries it.
enum AudioFile {
    /// Read: its name in the package, where it is, and what its header says.
    Packed {
        name: String,
        path: PathBuf,
        channels: usize,
        sample_rate: u32,
        frames: u64,
    },
    /// Not readable: why. Its clips are left out.
    Unreadable { reason: String },
}

/// What the package holds besides its two documents.
struct Contents {
    /// The file of each audio channel an audio clip plays.
    audio: BTreeMap<ChannelId, AudioFile>,
    /// Each CLAP instrument's saved state: its name in the package and its bytes, or `None`
    /// when the stored text is not base64 (the plugin then goes at its defaults).
    states: BTreeMap<ChannelId, Option<(String, Vec<u8>)>>,
}

/// Writes `project` (its sound paths relative to `base`) as a DAWproject file at `out`, titled
/// `title` in its metadata; returns what did not carry over, a line each. The file is written
/// beside `out` first and moved there whole, so a failed export leaves no half a package.
pub fn write(project: &Project, base: &Path, title: &str, out: &Path) -> io::Result<Vec<String>> {
    let contents = contents(project, base);
    let xml = project_xml(project, &contents);
    let lines = lines(project, &contents);
    let mut partial = out.as_os_str().to_owned();
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    let written = write_package(&partial, &xml, &metadata_xml(title), &contents)
        .and_then(|()| fs::rename(&partial, out));
    if let Err(error) = written {
        let _ = fs::remove_file(&partial);
        return Err(error);
    }
    Ok(lines)
}

/// project.xml, metadata.xml, the audio files by their names in the package, then the plugin
/// states in rack order.
fn write_package(path: &Path, xml: &str, metadata: &str, contents: &Contents) -> io::Result<()> {
    let mut zip = ZipWriter::new(BufWriter::new(File::create(path)?));
    zip.add("project.xml", xml.as_bytes())?;
    zip.add("metadata.xml", metadata.as_bytes())?;
    let audio: BTreeMap<&str, &Path> = contents
        .audio
        .values()
        .filter_map(|file| match file {
            AudioFile::Packed { name, path, .. } => Some((name.as_str(), path.as_path())),
            AudioFile::Unreadable { .. } => None,
        })
        .collect();
    for (name, path) in audio {
        zip.add(name, &fs::read(path)?)?;
    }
    for (name, bytes) in contents.states.values().flatten() {
        zip.add(name, bytes)?;
    }
    let file = zip
        .finish()?
        .into_inner()
        .map_err(|error| error.into_error())?;
    file.sync_all()
}

/// The files and states `project` packs: each audio channel's file that a clip plays (one
/// entry for a file two channels play, names kept apart when two files share one), each CLAP
/// instrument's state.
fn contents(project: &Project, base: &Path) -> Contents {
    let played: BTreeSet<ChannelId> = project
        .playlist
        .clips
        .values()
        .filter_map(Clip::audio_channel)
        .collect();
    // By the file's resolved path, so a file two channels play is packed once.
    let mut packed: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut audio = BTreeMap::new();
    for id in played {
        let Some(channel) = project.channels.get(&id) else {
            continue;
        };
        let path = paths::sound_file(base, &channel.sample);
        let file = match read_header(&path) {
            Ok((channels, sample_rate, frames)) => {
                let key = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                let taken: BTreeSet<&str> = packed.values().map(String::as_str).collect();
                let name = match packed.get(&key) {
                    Some(name) => name.clone(),
                    None => package_name(&path, &taken),
                };
                packed.insert(key, name.clone());
                AudioFile::Packed {
                    name,
                    path,
                    channels,
                    sample_rate,
                    frames,
                }
            }
            Err(reason) => AudioFile::Unreadable { reason },
        };
        audio.insert(id, file);
    }
    let mut states = BTreeMap::new();
    for &id in project.channels.keys() {
        let Some(text) =
            clap_instrument(project, id).and_then(|processor| processor.state.as_ref())
        else {
            continue;
        };
        let bytes = base64::engine::general_purpose::STANDARD.decode(text).ok();
        let name = format!("plugins/channel{}.clap-preset", id.0);
        states.insert(id, bytes.map(|bytes| (name, bytes)));
    }
    Contents { audio, states }
}

/// The CLAP plugin channel `id` plays, if it is a plugin channel playing one.
fn clap_instrument(project: &Project, id: ChannelId) -> Option<&ProcessorRef> {
    let channel = project.channels.get(&id)?;
    let processor = channel.processor.as_ref()?;
    (channel.kind == ChannelKind::Plugin && processor.format() == Some("clap")).then_some(processor)
}

/// A sound file's channels, rate and frames, from its header (decoding it only when the header
/// gives no length), or why it cannot be packed. A file of no frames is refused: it plays
/// nothing, and its warp would map no seconds onto no beats, a slope a reader cannot work out.
fn read_header(path: &Path) -> Result<(usize, u32, u64), String> {
    let info = decode::probe(path).map_err(|error| error.to_string())?;
    let frames = match info.frames {
        Some(frames) => frames,
        None => decode::decode(path)
            .map_err(|error| error.to_string())?
            .frames() as u64,
    };
    if frames == 0 {
        return Err("it holds no audio".to_owned());
    }
    Ok((info.channels, info.sample_rate, frames))
}

/// `audio/<file name>` for the file at `path`, its name made safe, with " (2)", " (3)" … before
/// the extension while `taken` holds it.
fn package_name(path: &Path, taken: &BTreeSet<&str>) -> String {
    let file = path
        .file_name()
        .map(|name| file_name_part(&name.to_string_lossy()))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "audio".to_owned());
    let (stem, extension) = match file.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem.to_owned(), format!(".{extension}")),
        _ => (file.clone(), String::new()),
    };
    let mut name = format!("audio/{file}");
    let mut copy = 1;
    while taken.contains(name.as_str()) {
        copy += 1;
        name = format!("audio/{stem} ({copy}){extension}");
    }
    name
}

/// An XML document, an element a line, two spaces deeper per level.
struct Xml {
    text: String,
    depth: usize,
}

impl Xml {
    fn new() -> Self {
        Self {
            text: DECLARATION.to_owned(),
            depth: 0,
        }
    }

    fn line(&mut self, body: &str) {
        for _ in 0..self.depth {
            self.text.push_str("  ");
        }
        self.text.push_str(body);
        self.text.push('\n');
    }

    /// `<name a="v" …`, without its end.
    fn tag(name: &str, attributes: &[(&str, String)]) -> String {
        let mut tag = format!("<{name}");
        for (key, value) in attributes {
            tag.push_str(&format!(" {key}=\"{}\"", escape(value)));
        }
        tag
    }

    fn open(&mut self, name: &str, attributes: &[(&str, String)]) {
        let tag = Self::tag(name, attributes);
        self.line(&format!("{tag}>"));
        self.depth += 1;
    }

    fn empty(&mut self, name: &str, attributes: &[(&str, String)]) {
        let tag = Self::tag(name, attributes);
        self.line(&format!("{tag}/>"));
    }

    fn close(&mut self, name: &str) {
        self.depth -= 1;
        self.line(&format!("</{name}>"));
    }

    fn text_element(&mut self, name: &str, text: &str) {
        self.line(&format!("<{name}>{}</{name}>", escape(text)));
    }
}

/// `text` as XML text or a double-quoted attribute value: the five markup characters as
/// entities, tab and line ends as character references (an attribute would read them as
/// spaces), and the characters XML 1.0 does not allow at all as U+FFFD.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\u{0}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}' => out.push('\u{fffd}'),
            c => out.push(c),
        }
    }
    out
}

/// The shortest decimal that reads back as `value`, with ".0" when it is whole.
fn decimal(value: f64) -> String {
    let text = format!("{value}");
    if text.contains('.') || !value.is_finite() {
        text
    } else {
        text + ".0"
    }
}

/// [`decimal`] for a value stored as an f32: its own shortest decimal.
fn decimal32(value: f32) -> String {
    let text = format!("{value}");
    if text.contains('.') || !value.is_finite() {
        text
    } else {
        text + ".0"
    }
}

/// `numerator / denominator` as a double, as [`decimal`] writes it.
fn ratio(numerator: u128, denominator: u128) -> String {
    decimal(numerator as f64 / denominator.max(1) as f64)
}

/// `ticks` in beats.
fn beats(ticks: i64, ppq: u16) -> String {
    decimal(ticks as f64 / f64::from(ppq.max(1)))
}

fn yes_no(value: bool) -> String {
    (if value { "true" } else { "false" }).to_owned()
}

fn metadata_xml(title: &str) -> String {
    let mut xml = Xml::new();
    xml.open("MetaData", &[]);
    xml.text_element("Title", title);
    xml.close("MetaData");
    xml.text
}

/// A channel's clips in the arrangement, in timeline order.
enum Part<'a> {
    Notes {
        clip: &'a Clip,
        notes: Vec<PlayedNote>,
    },
    Audio {
        clip: &'a Clip,
        file: &'a AudioFile,
    },
}

/// The clips in timeline order (start, then clip ID), each as the parts of the channels it
/// plays: a pattern clip's notes by channel (channels whose sound is notes), an audio clip on
/// its channel when its file was read.
fn parts<'a>(project: &'a Project, contents: &'a Contents) -> BTreeMap<ChannelId, Vec<Part<'a>>> {
    let mut clips: Vec<(&ClipId, &Clip)> = project.playlist.clips.iter().collect();
    clips.sort_by_key(|(id, clip)| (clip.start, **id));
    let mut parts: BTreeMap<ChannelId, Vec<Part<'a>>> = BTreeMap::new();
    for (_, clip) in clips {
        match clip.item {
            ClipItem::Pattern { .. } => {
                let mut by_channel: BTreeMap<ChannelId, Vec<PlayedNote>> = BTreeMap::new();
                for note in song::clip_notes(project, clip) {
                    let plays = project
                        .channels
                        .get(&note.channel)
                        .is_some_and(|channel| channel.plays_notes());
                    if plays {
                        by_channel.entry(note.channel).or_default().push(note);
                    }
                }
                for (channel, notes) in by_channel {
                    parts
                        .entry(channel)
                        .or_default()
                        .push(Part::Notes { clip, notes });
                }
            }
            ClipItem::Audio { channel, .. } => {
                if let Some(file @ AudioFile::Packed { .. }) = contents.audio.get(&channel) {
                    parts
                        .entry(channel)
                        .or_default()
                        .push(Part::Audio { clip, file });
                }
            }
            ClipItem::Automation { .. } => {}
        }
    }
    parts
}

fn project_xml(project: &Project, contents: &Contents) -> String {
    let mut xml = Xml::new();
    xml.open("Project", &[("version", "1.0".to_owned())]);
    xml.empty(
        "Application",
        &[
            ("name", "Dawsome".to_owned()),
            ("version", env!("CARGO_PKG_VERSION").to_owned()),
        ],
    );
    xml.open("Transport", &[]);
    xml.empty(
        "Tempo",
        &[
            ("unit", "bpm".to_owned()),
            ("value", ratio(u128::from(project.tempo.0), 1000)),
        ],
    );
    // Dawsome's bars are four beats.
    xml.empty(
        "TimeSignature",
        &[
            ("numerator", "4".to_owned()),
            ("denominator", "4".to_owned()),
        ],
    );
    xml.close("Transport");
    xml.open("Structure", &[]);
    for (&id, channel) in &project.channels {
        let content = match channel.kind {
            ChannelKind::Audio | ChannelKind::Input => "audio",
            ChannelKind::Sampler | ChannelKind::Plugin | ChannelKind::Instrument => "notes",
        };
        xml.open(
            "Track",
            &[
                ("contentType", content.to_owned()),
                ("id", format!("track{}", id.0)),
                ("name", channel.name.clone()),
            ],
        );
        xml.open(
            "Channel",
            &[
                ("audioChannels", "2".to_owned()),
                ("destination", MASTER.to_owned()),
                ("role", "regular".to_owned()),
                ("solo", "false".to_owned()),
                ("id", format!("channel{}", id.0)),
            ],
        );
        if let Some(processor) = clap_instrument(project, id) {
            let plugin_id = processor.id.split_once(':').map_or("", |(_, id)| id);
            let device = [
                ("deviceID", plugin_id.to_owned()),
                ("deviceName", processor.display_name().to_owned()),
                ("deviceRole", "instrument".to_owned()),
            ];
            xml.open("Devices", &[]);
            match contents.states.get(&id) {
                Some(Some((name, _))) => {
                    xml.open("ClapPlugin", &device);
                    xml.empty("State", &[("path", name.clone())]);
                    xml.close("ClapPlugin");
                }
                _ => xml.empty("ClapPlugin", &device),
            }
            xml.close("Devices");
        }
        mixer_controls(
            &mut xml,
            channel.mute,
            (channel.pan + 1.0) / 2.0,
            channel.volume,
        );
        xml.close("Channel");
        xml.close("Track");
    }
    xml.open(
        "Track",
        &[
            ("contentType", "audio notes".to_owned()),
            ("id", "master-track".to_owned()),
            ("name", "Master".to_owned()),
        ],
    );
    xml.open(
        "Channel",
        &[
            ("audioChannels", "2".to_owned()),
            ("role", "master".to_owned()),
            ("solo", "false".to_owned()),
            ("id", MASTER.to_owned()),
        ],
    );
    mixer_controls(&mut xml, false, 0.5, 1.0);
    xml.close("Channel");
    xml.close("Track");
    xml.close("Structure");
    xml.open("Arrangement", &[]);
    let parts = parts(project, contents);
    if parts.is_empty() {
        xml.empty("Lanes", &[("timeUnit", "beats".to_owned())]);
    } else {
        xml.open("Lanes", &[("timeUnit", "beats".to_owned())]);
        for (id, channel_parts) in &parts {
            let Some(channel) = project.channels.get(id) else {
                continue;
            };
            xml.open("Lanes", &[("track", format!("track{}", id.0))]);
            xml.open("Clips", &[]);
            for part in channel_parts {
                match part {
                    Part::Notes { clip, notes } => notes_clip(&mut xml, project, clip, notes),
                    Part::Audio { clip, file } => {
                        audio_clip(&mut xml, project, clip, file, &channel.name);
                    }
                }
            }
            xml.close("Clips");
            xml.close("Lanes");
        }
        xml.close("Lanes");
    }
    xml.close("Arrangement");
    xml.close("Project");
    xml.text
}

/// A Channel's mute, pan (normalized, 0.5 the center) and volume (linear gain).
fn mixer_controls(xml: &mut Xml, mute: bool, pan: f32, volume: f32) {
    xml.empty("Mute", &[("value", yes_no(mute))]);
    xml.empty(
        "Pan",
        &[
            ("unit", "normalized".to_owned()),
            ("value", decimal32(pan)),
            ("min", "0.0".to_owned()),
            ("max", "1.0".to_owned()),
        ],
    );
    xml.empty(
        "Volume",
        &[
            ("unit", "linear".to_owned()),
            ("value", decimal32(volume)),
            ("min", "0.0".to_owned()),
            ("max", decimal32(MAX_VOLUME)),
        ],
    );
}

/// The clip's place: its start and length in beats.
fn clip_attributes(project: &Project, clip: &Clip) -> Vec<(&'static str, String)> {
    vec![
        ("time", beats(clip.start.0, project.ppq)),
        ("duration", beats(clip.length.0, project.ppq)),
    ]
}

fn notes_clip(xml: &mut Xml, project: &Project, clip: &Clip, notes: &[PlayedNote]) {
    let mut attributes = clip_attributes(project, clip);
    if !project.clip_plays(clip) {
        attributes.push(("enable", "false".to_owned()));
    }
    let name = clip
        .pattern()
        .and_then(|id| project.patterns.get(&id))
        .map_or_else(String::new, |pattern| pattern.name.clone());
    attributes.push(("name", name));
    xml.open("Clip", &attributes);
    xml.open("Notes", &[]);
    let mut notes = notes.to_vec();
    notes.sort_by_key(|note| (note.start, note.key));
    for note in notes {
        xml.empty(
            "Note",
            &[
                ("time", beats(note.start.0 - clip.start.0, project.ppq)),
                ("duration", beats(note.end.0 - note.start.0, project.ppq)),
                ("channel", "0".to_owned()),
                ("key", note.key.to_string()),
                ("vel", decimal32(note.velocity)),
            ],
        );
    }
    xml.close("Notes");
    xml.close("Clip");
}

fn audio_clip(xml: &mut Xml, project: &Project, clip: &Clip, file: &AudioFile, name: &str) {
    let (
        ClipItem::Audio {
            frame_offset,
            stretch,
            ..
        },
        AudioFile::Packed {
            name: packed,
            channels,
            sample_rate,
            frames,
            ..
        },
    ) = (clip.item, file)
    else {
        return;
    };
    // Frames at the project's rate (the offset, a stretched length) in beats:
    // frames · milli-BPM / (rate · 60000).
    let tempo = u128::from(project.tempo.0);
    let rate = u128::from(project.sample_rate);
    let in_beats = |frames: u64| ratio(u128::from(frames) * tempo, rate * 60_000);
    let seconds = ratio(u128::from(*frames), u128::from(*sample_rate));
    let warped = match stretch {
        Some(stretch) => in_beats(stretch.frames),
        None => ratio(
            u128::from(*frames) * tempo,
            u128::from(*sample_rate) * 60_000,
        ),
    };
    let mut attributes = clip_attributes(project, clip);
    attributes.push(("playStart", in_beats(frame_offset)));
    if !project.clip_plays(clip) {
        attributes.push(("enable", "false".to_owned()));
    }
    attributes.push(("name", name.to_owned()));
    xml.open("Clip", &attributes);
    xml.open(
        "Warps",
        &[
            ("contentTimeUnit", "seconds".to_owned()),
            ("timeUnit", "beats".to_owned()),
        ],
    );
    xml.open(
        "Audio",
        &[
            ("channels", channels.to_string()),
            ("duration", seconds.clone()),
            ("sampleRate", sample_rate.to_string()),
            ("timeUnit", "seconds".to_owned()),
        ],
    );
    xml.empty("File", &[("path", packed.clone())]);
    xml.close("Audio");
    xml.empty(
        "Warp",
        &[
            ("time", "0.0".to_owned()),
            ("contentTime", "0.0".to_owned()),
        ],
    );
    xml.empty("Warp", &[("time", warped), ("contentTime", seconds)]);
    xml.close("Warps");
    xml.close("Clip");
}

/// What the package does not carry: each channel's (rack order), each audio clip's (timeline
/// order), then the mixer, automation and the loop range.
fn lines(project: &Project, contents: &Contents) -> Vec<String> {
    let mut lines = Vec::new();
    let file_of = |path: &Path| {
        path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
    };
    let mut not_notes: BTreeMap<ChannelId, usize> = BTreeMap::new();
    for clip in project.playlist.clips.values() {
        for note in song::clip_notes(project, clip) {
            if project
                .channels
                .get(&note.channel)
                .is_some_and(|channel| !channel.plays_notes())
            {
                *not_notes.entry(note.channel).or_default() += 1;
            }
        }
    }
    for (id, channel) in &project.channels {
        let name = &channel.name;
        match channel.kind {
            ChannelKind::Sampler => lines.push(format!(
                "{name}: a sampler playing {}; DAWproject has no sampler, so its track has no instrument",
                file_of(&channel.sample)
            )),
            ChannelKind::Instrument => lines.push(format!(
                "{name}: the instrument set {}; DAWproject has no sampler, so its track has no instrument",
                file_of(&channel.instrument)
            )),
            ChannelKind::Plugin => {
                let plugin = channel
                    .processor
                    .as_ref()
                    .map_or(name.as_str(), |processor| processor.display_name());
                match (clap_instrument(project, *id), contents.states.get(id)) {
                    (Some(_), Some(Some(_))) => lines.push(format!(
                        "{name}: {plugin} goes as a CLAP plugin with its saved state; loading that state is up to the other program"
                    )),
                    (Some(_), Some(None)) => lines.push(format!(
                        "{name}: {plugin} goes as a CLAP plugin at its defaults: its saved state could not be read"
                    )),
                    (Some(_), None) => {}
                    (None, _) => lines.push(format!(
                        "{name}: {plugin} is not a CLAP plugin, so its track has no instrument"
                    )),
                }
            }
            ChannelKind::Input => lines.push(format!(
                "{name}: a live input has nothing to carry; its track is empty"
            )),
            ChannelKind::Audio => {
                if let Some(AudioFile::Unreadable { reason }) = contents.audio.get(id) {
                    lines.push(format!(
                        "{name}: {} could not be read ({reason}); its clips are left out",
                        file_of(&channel.sample)
                    ));
                }
            }
        }
        if channel.plays_notes() && !channel.sound.is_default() {
            lines.push(format!(
                "{name}: its sound settings (tuning, envelope, filter) stay in Dawsome"
            ));
        }
        // The file goes into the package as it is; the edited sound goes in a later version.
        if !channel.edits.is_empty() {
            lines.push(format!(
                "{name}'s sound edits (its file is exported as on disk)"
            ));
        }
        if let Some(count) = not_notes.get(id) {
            let notes = if *count == 1 { "note" } else { "notes" };
            lines.push(format!("{name}: {count} {notes} left out"));
        }
    }
    let mut clips: Vec<(&ClipId, &Clip)> = project.playlist.clips.iter().collect();
    clips.sort_by_key(|(id, clip)| (clip.start, **id));
    let bar = time::bars_to_ticks(1, project.ppq).0.max(1);
    for (_, clip) in clips {
        let ClipItem::Audio {
            channel,
            gain,
            stretch,
            ..
        } = clip.item
        else {
            continue;
        };
        if !matches!(contents.audio.get(&channel), Some(AudioFile::Packed { .. })) {
            continue;
        }
        let name = project
            .channels
            .get(&channel)
            .map_or("", |channel| channel.name.as_str());
        let at = clip.start.0.div_euclid(bar) + 1;
        if gain != 1.0 {
            lines.push(format!(
                "{name} clip at bar {at}: gain {gain} is not carried"
            ));
        }
        if let Some(stretch) = stretch {
            let how = match (stretch.mode, stretch.pitch) {
                (StretchMode::Resample, _) => "resample".to_owned(),
                (StretchMode::Stretch, 0) => "stretch".to_owned(),
                (StretchMode::Stretch, pitch) => format!("stretch (pitch {pitch:+})"),
            };
            lines.push(format!(
                "{name} clip at bar {at}: its {how} goes as a warp to its length; how it stretches is up to the other program"
            ));
        }
    }
    let routed = project
        .channels
        .values()
        .any(|channel| channel.insert != InsertId::MASTER);
    if routed || project.mixer != Mixer::default() {
        lines.push(
            "The mixer (inserts, sends, effects) stays in Dawsome: every track plays into the master"
                .to_owned(),
        );
    }
    let automations = project.automation.list.len();
    if automations > 0 {
        let what = if automations == 1 {
            "automation stays"
        } else {
            "automations stay"
        };
        lines.push(format!("{automations} {what} in Dawsome"));
    }
    if project.playlist.loop_range.is_some_and(|range| range.on) {
        lines.push("The loop range stays in Dawsome".to_owned());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    // By hand: markup characters become entities, tab and line ends character references,
    // a character XML forbids U+FFFD; others stay.
    #[test]
    fn text_is_escaped_for_xml() {
        assert_eq!(
            escape("a&b<c>d\"e'f\tg\nh\ri\u{1}j ♪"),
            "a&amp;b&lt;c&gt;d&quot;e&apos;f&#9;g&#10;h&#13;i\u{fffd}j ♪"
        );
    }

    // By hand: the shortest decimal of the double, ".0" on whole numbers.
    #[test]
    fn numbers_are_the_shortest_decimal() {
        assert_eq!(beats(384, 96), "4.0");
        assert_eq!(beats(24, 96), "0.25");
        assert_eq!(beats(1, 96), "0.010416666666666666");
        assert_eq!(ratio(93_217, 1000), "93.217");
        assert_eq!(decimal32(0.8), "0.8");
        assert_eq!(decimal32(1.0), "1.0");
        assert_eq!(decimal32((-0.25 + 1.0) / 2.0), "0.375");
    }

    // Two files named alike get names of their own; a name without a stem keeps its dot.
    #[test]
    fn package_names_stay_apart() {
        let none = BTreeSet::new();
        assert_eq!(
            package_name(Path::new("/a/kick.wav"), &none),
            "audio/kick.wav"
        );
        let taken = BTreeSet::from(["audio/kick.wav", "audio/kick (2).wav"]);
        assert_eq!(
            package_name(Path::new("/b/kick.wav"), &taken),
            "audio/kick (3).wav"
        );
        assert_eq!(package_name(Path::new("/c/.wav"), &none), "audio/.wav");
        assert_eq!(
            package_name(Path::new("/d/a:b.wav"), &none),
            "audio/a_b.wav"
        );
    }

    // A sound file of no frames plays nothing, and its warp would map no seconds onto no
    // beats (a reader working out the warp's slope divides by zero): its clips are left out
    // and the report says why.
    #[test]
    fn an_audio_file_without_frames_leaves_its_clips_out_and_says_so() {
        use dawsome_engine::project::{Channel, Clip, TrackId};
        use dawsome_engine::time::{MilliBpm, Ticks};
        let dir = std::env::temp_dir().join(format!("dawsome-media-empty-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 44_100,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        hound::WavWriter::create(dir.join("empty.wav"), spec)
            .expect("creates")
            .finalize()
            .expect("written");
        let mut project = Project::new(44_100, MilliBpm(120_000), 96);
        project
            .channels
            .insert(ChannelId(1), Channel::audio("empty", "empty.wav"));
        project.playlist.clips.insert(
            ClipId(1),
            Clip::of_audio(ChannelId(1), TrackId(1), Ticks(0), Ticks(384)),
        );
        let contents = contents(&project, &dir);
        let xml = project_xml(&project, &contents);
        assert!(!xml.contains("<Clip "), "{xml}");
        assert_eq!(
            lines(&project, &contents),
            ["empty: empty.wav could not be read (it holds no audio); its clips are left out"]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    // A file no decoder reads leaves its clips out and says why, and the rest still packs.
    #[test]
    fn an_unreadable_audio_file_leaves_its_clips_out_and_says_so() {
        use dawsome_engine::project::{Channel, Clip, TrackId};
        use dawsome_engine::time::{MilliBpm, Ticks};
        let mut project = Project::new(44_100, MilliBpm(120_000), 96);
        project
            .channels
            .insert(ChannelId(1), Channel::audio("gone", "no-such-file.wav"));
        project.playlist.clips.insert(
            ClipId(1),
            Clip::of_audio(ChannelId(1), TrackId(1), Ticks(0), Ticks(384)),
        );
        let contents = contents(&project, Path::new("/nonexistent-dawsome-test"));
        assert!(matches!(
            contents.audio.get(&ChannelId(1)),
            Some(AudioFile::Unreadable { .. })
        ));
        let xml = project_xml(&project, &contents);
        assert!(!xml.contains("<Clip "), "{xml}");
        assert!(xml.contains("<Lanes timeUnit=\"beats\"/>"), "{xml}");
        let lines = lines(&project, &contents);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].starts_with("gone: no-such-file.wav could not be read ("),
            "{lines:?}"
        );
        assert!(lines[0].ends_with("); its clips are left out"), "{lines:?}");
    }
}
