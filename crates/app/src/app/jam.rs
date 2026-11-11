//! Jam in the app: "Start a Jam" on the channel rack's header opens the prompt over the main
//! window, which lists the rack's channels with the part each fills (found from what the
//! channel plays, [`found_role`]); Next opens the Jam window, a pop-out placed clear of the rack
//! and the playlist (over the Library's column, under X11; where the window manager puts it
//! under Wayland), which rolls patterns.
//!
//! A roll is one undoable edit ([`Edit::Jam`]): its first writes a new pattern "Jam N" and
//! selects it, each roll after that replaces that same pattern until Accept, and the roll after
//! Accept writes the next new one. A roll plays nothing by itself. Each takes a fresh seed from
//! the clock, shown in the window's seed field: a seed typed there is the one the next roll takes,
//! so a roll can be written again (as `dawsome jam --seed N` writes it). A scripted run or a test
//! pins the first seed ([`Jam::next_seed`]) and the rolls after it count on from it, so it writes
//! the same rolls every time. A locked channel keeps its notes:
//! the whole Jam is written from the new seed, then the locked channels' parts are put back from
//! the roll before ([`JamPattern::keeping`]); a channel's own dice keeps every other channel the
//! same way. Why a seed is passed over while a kept part is chords, a bass or a melody: the new
//! seed must pick the progression the kept part was written over, or the parts clash.
//!
//! The project's key, which a Jam writes in, is also set on Settings' Music page
//! ([`App::project_key_message`]); the piano roll veils the keys outside it.

use std::time::{SystemTime, UNIX_EPOCH};

use dawsome_engine::edit::{takes, Edit};
use dawsome_engine::jam::{
    self, Articulation, DrumMap, Genre, JamChannel, JamPattern, JamRequest, Role, Voice,
};
use dawsome_engine::project::{Channel, ChannelId, ChannelKind, PatternId};
use dawsome_engine::time::{MeterUnit, Ticks, SWING_RANGE};
use iced::{window, Element, Point, Size, Task};

use super::instrument::pop_out_level;
use super::{App, Message, WindowChange};
use crate::desktop::{self, PopOut};
use crate::ui::jam::{
    AddChoice, ChannelChoice, JamMessage, JamPage, JamRow, KeyRoot, PromptRow, RoleName,
};
use crate::ui::layout::TOOLBAR_HEIGHT;
use crate::ui::library::Kind;
use crate::ui::toolbar::{format_tempo, parse_tempo};

/// How many seeds after the next a roll that keeps harmony looks through for one that picks the
/// kept part's progression; past them it takes the next as it comes. Each genre has two or three
/// progressions a family, so one in three seeds or so picks a given one.
const PROGRESSION_SEARCH: u64 = 64;

/// How many seeds the clock hands out, 1 to this: few enough digits to type a roll's seed again.
const CLOCK_SEEDS: u64 = 1_000_000;

/// A fresh seed: the clock's nanoseconds mixed by splitmix64 (so two rolls a moment apart are
/// far apart), 1 to [`CLOCK_SEEDS`].
fn clock_seed() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos() as u64);
    jam::splitmix64(nanos) % CLOCK_SEEDS + 1
}

/// Room left above the Jam window for its frame's title bar, in the system's logical pixels.
/// Why: the system places a window by its frame's corner (winit's outer position, which is also
/// what it says of the main window), so a window put at the toolbar's foot would lay its title
/// bar over the toolbar's lower half (Play, the tempo). 36 clears a KDE Plasma title bar (about 30
/// at any scale) and leaves a few pixels of the Library showing above it; an undecorated main
/// window shows those 36 too.
const TITLE_BAR_ROOM: f32 = 36.0;

/// The key a Jam window opens in when the project has none: A minor, the key lo-fi's own pins
/// are written in.
const FIRST_KEY: jam::KeySignature = jam::KeySignature {
    root: match jam::PitchClass::new(9) {
        Some(root) => root,
        None => jam::PitchClass::C,
    },
    scale: jam::Scale::Minor,
};

/// The Jam's state in the app.
pub(super) struct Jam {
    /// The prompt over the main window, while it shows: the rack's channels.
    pub prompt: Option<Vec<PromptRow>>,
    /// The Jam window's session, while it is open.
    pub session: Option<JamSession>,
    /// Its own window while it is open or opening.
    pub window: Option<window::Id>,
    /// Where it opens: its size and position, worked out as Next opens it.
    place: (Size, Option<Point>),
    /// Whether its window was last asked to stay above every window.
    above: bool,
    /// The seed the next roll takes when it is not 0, the rolls after it counting on (a
    /// scripted run's, a test's); 0, every roll takes a fresh one from the clock.
    pub next_seed: u64,
}

impl Jam {
    pub fn new() -> Self {
        Self {
            prompt: None,
            session: None,
            window: None,
            place: (crate::ui::jam::WINDOW_SIZE, None),
            above: false,
            next_seed: 0,
        }
    }
}

/// What the Jam window rolls with.
pub(super) struct JamSession {
    pub channels: Vec<JamRow>,
    pub genre: Genre,
    /// Whether a roll sets the genre's tempo; else the one typed.
    pub auto_tempo: bool,
    pub tempo: String,
    pub key: jam::KeySignature,
    pub bars: u32,
    pub swing: u16,
    /// The pattern the rolls replace until Accept; none: the next roll writes a new one.
    pub pattern: Option<PatternId>,
    /// The last roll, whose parts a lock or a channel's own dice keeps.
    pub last: Option<JamPattern>,
    /// The last roll's seed.
    pub seed: Option<u64>,
    /// The seed field: the last roll's seed, or what was typed there.
    pub seed_text: String,
    /// Whether the seed field was typed in since the last roll: the next roll takes its seed.
    pub seed_typed: bool,
    /// The tempo the last roll drew while Auto was on, shown until the genre or Auto changes.
    pub rolled_tempo: Option<dawsome_engine::time::MilliBpm>,
    /// Why the last roll was refused.
    pub refused: Option<String>,
}

/// The part `channel` fills in a Jam by what it plays, if one is found: a drum set's channel
/// drums on `drums` (its set's keys), a bass's the bass, keys', pads', guitars' and strings'
/// the chords, a synth's or mallets' the melody (Dawsome Synth's too); a sampler whose sound (or
/// else whose name) is a starter-kit drum's one voice of drums, on the key it plays its sound
/// as recorded at: Kick… the kick, Snare, Clap, Hat closed, Hat open, and Tom…, Rim, Cowbell and
/// Crash the perc. `kind` is the kind of an instrument channel's set.
pub fn found_role(channel: &Channel, kind: Option<Kind>, drums: DrumMap) -> Option<Role> {
    match channel.kind {
        ChannelKind::Instrument => match kind? {
            Kind::Drums => Some(Role::Drums(drums)),
            Kind::Bass => Some(Role::Bass),
            Kind::Keys | Kind::Pads | Kind::Guitar | Kind::Strings => Some(Role::Chords),
            Kind::Synth | Kind::Mallets => Some(Role::Melody),
            Kind::Other => None,
        },
        ChannelKind::Synth => Some(Role::Melody),
        ChannelKind::Sampler => {
            let stem = channel
                .sample
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default();
            kit_voice(stem)
                .or_else(|| kit_voice(&channel.name))
                .map(|voice| Role::Drums(DrumMap::single(voice)))
        }
        _ => None,
    }
}

/// How `channel`'s instrument plays a melody: held for a set of keys (an organ set is one), pads
/// or strings; plucked for everything else, a sound the app knows nothing of included.
pub fn articulation_of(channel: &Channel, kind: Option<Kind>) -> Articulation {
    match (channel.kind, kind) {
        (ChannelKind::Instrument, Some(Kind::Keys | Kind::Pads | Kind::Strings)) => {
            Articulation::Sustained
        }
        _ => Articulation::Plucked,
    }
}

/// The drum voice a starter-kit sound's name `name` plays: "Kick 808" and "Kick 909" the kick,
/// the hats by their names, the toms, the rim, the cowbell and the crash the perc.
fn kit_voice(name: &str) -> Option<Voice> {
    match name {
        "Snare" => Some(Voice::Snare),
        "Clap" => Some(Voice::Clap),
        "Hat closed" => Some(Voice::ClosedHat),
        "Hat open" => Some(Voice::OpenHat),
        "Rim" | "Cowbell" | "Crash" => Some(Voice::Perc),
        _ if name.starts_with("Kick") => Some(Voice::Kick),
        _ if name.starts_with("Tom") => Some(Voice::Perc),
        _ => None,
    }
}

/// The capitalised first letter of `words`, as a line starts.
fn sentence(words: &str) -> String {
    let mut chars = words.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

impl App {
    /// Why "Start a Jam" is greyed, in the engine's words, or none: a Jam writes 4/4 patterns
    /// whose half steps are whole ticks, for the channels on the rack.
    pub(super) fn jam_refusal(&self) -> Option<String> {
        let project = self.session.project();
        if !project.time_signature.is_common() {
            return Some(format!(
                "A Jam writes 4/4 patterns and this project is in {}",
                project.time_signature
            ));
        }
        if !project.ppq.is_multiple_of(jam::PPQ_MULTIPLE) {
            return Some(format!(
                "A Jam needs a PPQ that is a multiple of 8, and this project's is {}",
                project.ppq
            ));
        }
        if project.channels.is_empty() {
            return Some("The rack is empty: add an instrument to jam with".to_owned());
        }
        None
    }

    /// The keys `channel`'s instrument plays, the lowest and highest over its set's zones (what
    /// the instrument window lists), which a Jam fits its part into; `None` for any other
    /// channel (a sample plays every key at its own pitch, a plugin says nothing of its keys)
    /// and for a set with no zone read.
    pub(super) fn key_range(&self, channel: &Channel) -> Option<(u8, u8)> {
        let zones = self.zones_of(channel);
        let low = zones.iter().map(|zone| zone.keys.0).min()?;
        let high = zones.iter().map(|zone| zone.keys.1).max()?;
        Some((low, high))
    }

    /// The kind an instrument channel's set says it is, as the Library files it; none for any
    /// other channel.
    fn set_kind(&self, channel: &Channel) -> Option<Kind> {
        let set = (channel.kind == ChannelKind::Instrument).then(|| self.set_file(channel))?;
        crate::sets::kind_of_set(&set)
    }

    /// The part `channel` fills by what it plays ([`found_role`]), and the drum keys it plays as
    /// drums, picked or found: its set's, or a sampler's one voice (a perc for a sound the kit
    /// does not name).
    fn channel_roles(&self, channel: &Channel) -> (Option<Role>, DrumMap) {
        let set = (channel.kind == ChannelKind::Instrument).then(|| self.set_file(channel));
        let kind = set.as_deref().and_then(crate::sets::kind_of_set);
        let drums = match (&set, channel.kind) {
            (Some(set), _) => crate::sets::drum_map(set),
            (None, ChannelKind::Sampler) => DrumMap::single(Voice::Perc),
            (None, _) => DrumMap::GM,
        };
        let found = found_role(channel, kind, drums);
        let drums = match found {
            Some(Role::Drums(map)) => map,
            _ => drums,
        };
        (found, drums)
    }

    /// Something the Jam's views reported.
    pub(super) fn jam_message(&mut self, message: JamMessage) -> Task<Message> {
        match message {
            JamMessage::Start => self.open_jam_prompt(),
            JamMessage::Join(channel, on) => self.with_prompt_row(channel, |row| row.joined = on),
            JamMessage::RoleChosen(channel, name) => {
                let drums = self
                    .session
                    .project()
                    .channels
                    .get(&channel)
                    .map_or(DrumMap::GM, |shown| self.channel_roles(shown).1);
                let role = match name {
                    RoleName::Drums => Role::Drums(drums),
                    RoleName::Bass => Role::Bass,
                    RoleName::Chords => Role::Chords,
                    RoleName::Melody => Role::Melody,
                };
                // A part picked is a channel that plays.
                self.with_prompt_row(channel, |row| {
                    row.role = Some(role);
                    row.joined = true;
                });
            }
            JamMessage::Next => self.open_jam_window(),
            JamMessage::Cancel => self.jam.prompt = None,
            JamMessage::Close => self.close_jam_window(),
            JamMessage::ProjectRoot(root) => self.set_project_root(root),
            JamMessage::ProjectScale(scale) => {
                if let Some(key) = self.session.project().key {
                    self.commit(Edit::SetKey {
                        key: Some(jam::KeySignature {
                            scale: scale.0,
                            ..key
                        }),
                    });
                }
            }
            JamMessage::SendToPlaylist => self.send_jam_to_playlist(),
            JamMessage::Switch(from, to) => self.switch_jam_instrument(from, to),
            JamMessage::Remove(channel) => self.remove_from_jam(channel),
            JamMessage::Add(channel) => self.add_to_jam(channel),
            JamMessage::RowRole(channel, name) => self.change_jam_role(channel, name),
            JamMessage::Roll => self.roll_jam(None),
            JamMessage::RollChannel(channel) => self.roll_jam(Some(channel)),
            other => self.jam_setting(other),
        }
        Task::none()
    }

    /// One of the Jam window's settings changed: the genre (its swing, and its tempo while Auto
    /// is on), the tempo, the key, the bars, the swing, a lock; or Accept.
    fn jam_setting(&mut self, message: JamMessage) {
        let project_tempo = format_tempo(self.session.project().tempo);
        let Some(session) = self.jam.session.as_mut() else {
            return;
        };
        match message {
            JamMessage::SeedEdited(typed) => {
                session.seed_text = typed;
                session.seed_typed = true;
            }
            JamMessage::NewSeed => {
                session.seed_text = clock_seed().to_string();
                session.seed_typed = true;
            }
            JamMessage::Genre(genre) => {
                session.rolled_tempo = None;
                session.genre = genre.0;
                session.swing = genre.0.table().swing_tenths;
            }
            JamMessage::AutoTempo(on) => {
                session.rolled_tempo = None;
                session.auto_tempo = on;
                // Off, the box starts at the project's tempo, so a roll keeps it until another
                // is typed.
                if !on {
                    session.tempo = project_tempo;
                }
            }
            JamMessage::TempoEdited(typed) => session.tempo = typed,
            JamMessage::Root(root) => session.key.root = root.0,
            JamMessage::Scale(scale) => session.key.scale = scale.0,
            JamMessage::Bars(bars) => session.bars = bars,
            JamMessage::Swing(tenths) => {
                session.swing = tenths.clamp(*SWING_RANGE.start(), *SWING_RANGE.end());
            }
            JamMessage::Lock(channel) => {
                if let Some(row) = session
                    .channels
                    .iter_mut()
                    .find(|row| row.channel == channel)
                {
                    row.locked = !row.locked;
                }
            }
            JamMessage::Accept => {
                session.pattern = None;
                session.refused = None;
            }
            _ => {}
        }
    }

    fn with_prompt_row(&mut self, channel: ChannelId, change: impl FnOnce(&mut PromptRow)) {
        if let Some(row) = self
            .jam
            .prompt
            .as_mut()
            .and_then(|rows| rows.iter_mut().find(|row| row.channel == channel))
        {
            change(row);
        }
    }

    /// "Start a Jam": the prompt over the main window, the rack's channels in its order, each
    /// ticked when a part was found for it; nothing while a Jam cannot be written.
    fn open_jam_prompt(&mut self) {
        if self.jam_refusal().is_some() {
            return;
        }
        // The rack's order is the channels' (`ui::rack_rows`); the audio takes under Recordings
        // play no notes and are left out.
        let rows = self
            .session
            .project()
            .channels
            .iter()
            .filter(|(_, channel)| !crate::ui::rack_rows::is_take(channel))
            .map(|(&id, channel)| {
                let plays_notes = channel.plays_notes();
                let role = if plays_notes {
                    self.channel_roles(channel).0
                } else {
                    None
                };
                PromptRow {
                    channel: id,
                    name: channel.name.clone(),
                    plays_notes,
                    role,
                    joined: role.is_some(),
                }
            })
            .collect();
        self.jam.prompt = Some(rows);
    }

    /// The prompt's Next: the Jam window opens on the channels ticked with a part, in the rack's
    /// order, at the genre's settings and the project's key (A minor without one), placed over the
    /// Library's column.
    fn open_jam_window(&mut self) {
        let Some(rows) = self.jam.prompt.take() else {
            return;
        };
        let channels: Vec<JamRow> = rows
            .into_iter()
            .filter(|row| row.joined)
            .filter_map(|row| {
                Some(JamRow {
                    channel: row.channel,
                    name: row.name,
                    role: row.role?,
                    locked: false,
                    pending: false,
                })
            })
            .collect();
        if channels.is_empty() {
            return;
        }
        let genre = Genre::LoFi;
        let project = self.session.project();
        // Over the Library's column: under the toolbar and room for its title bar, at the main
        // window's left edge, where the system says where the main window is (X11, Windows); a
        // Wayland compositor places it. Its size is fixed; a small screen shrinks it once it is
        // open and its monitor is known ([`App::fit_jam_window`]).
        let zoom = self.zoom().factor();
        let position = self
            .main_position
            .map(|main| Point::new(main.x, main.y + TOOLBAR_HEIGHT * zoom + TITLE_BAR_ROOM));
        self.jam.place = (crate::ui::jam::WINDOW_SIZE, position);
        self.jam.session = Some(JamSession {
            channels,
            genre,
            auto_tempo: true,
            tempo: format_tempo(project.tempo),
            key: project.key.unwrap_or(FIRST_KEY),
            bars: 4,
            swing: genre.table().swing_tenths,
            pattern: None,
            last: None,
            seed: None,
            seed_text: String::new(),
            seed_typed: false,
            rolled_tempo: None,
            refused: None,
        });
    }

    /// Closes the Jam window, leaving what it rolled: its X, Esc in it, Close.
    pub(super) fn close_jam_window(&mut self) {
        self.jam.session = None;
    }

    /// Closes the prompt and the Jam window: their channels are another document's.
    pub(super) fn close_jam(&mut self) {
        self.jam.prompt = None;
        self.jam.session = None;
    }

    /// Esc in the main window while the prompt shows: it closes; `false` when none showed.
    pub(super) fn escape_jam_prompt(&mut self) -> bool {
        self.jam.prompt.take().is_some()
    }

    /// The next seed (a fresh one from the clock, or the pinned one), or for a roll that keeps a
    /// part of harmony over `progression` the first of the next [`PROGRESSION_SEARCH`] from it
    /// that picks it ([`jam::progression`]).
    fn take_seed(&mut self, genre: Genre, scale: jam::Scale, progression: Option<[u8; 4]>) -> u64 {
        let first = match self.jam.next_seed {
            0 => clock_seed(),
            pinned => pinned,
        };
        let seed = progression
            .and_then(|wanted| {
                (0..PROGRESSION_SEARCH)
                    .map(|step| first.wrapping_add(step))
                    .find(|&seed| jam::progression(genre, scale, seed) == wanted)
            })
            .unwrap_or(first);
        if self.jam.next_seed != 0 {
            // Pinned stays pinned: 0 would hand the next roll to the clock.
            self.jam.next_seed = seed.wrapping_add(1).max(1);
        }
        seed
    }

    /// "Send to playlist": one clip of the Jam pattern, as long as it, from the bar line at or
    /// before the song position (the playhead while the song plays, else where it plays from),
    /// on the first track free over the clip, else on a new track: one undo step.
    fn send_jam_to_playlist(&mut self) {
        let Some(pattern) = self
            .jam
            .session
            .as_ref()
            .and_then(|session| session.pattern)
        else {
            return;
        };
        let at = if self.plays_song() {
            self.playhead.unwrap_or(self.song_marker)
        } else {
            self.song_marker
        };
        let project = self.session.project();
        let start = project
            .meter_map()
            .snap_down(Ticks(at.0.max(0)), MeterUnit::Bar);
        if let Some(edit) = takes::pattern_clip(project, pattern, start) {
            self.commit(edit);
        }
    }

    /// A roll: of every channel but the locked ones, or of `only` that channel, as one undo
    /// step. The pattern rolled is selected; why it was refused, if it was, shows in the window.
    fn roll_jam(&mut self, only: Option<ChannelId>) {
        let project = self.session.project();
        let Some(session) = self.jam.session.as_ref() else {
            return;
        };
        let channels: Vec<JamChannel> = session
            .channels
            .iter()
            .filter_map(|row| {
                let channel = project.channels.get(&row.channel)?;
                Some(JamChannel {
                    channel: row.channel,
                    role: row.role,
                    range: self.key_range(channel),
                    articulation: articulation_of(channel, self.set_kind(channel)),
                })
            })
            .collect();
        let refused = if channels.is_empty() {
            Some("None of the Jam's channels is on the rack now".to_owned())
        } else {
            None
        };
        let tempo = match (session.auto_tempo, parse_tempo(&session.tempo)) {
            (true, _) => Ok(None),
            (false, Ok(tempo)) => Ok(Some(tempo)),
            (false, Err(reason)) => Err(reason),
        };
        let (refused, tempo) = match (refused, tempo) {
            (Some(refused), _) | (None, Err(refused)) => (Some(refused), None),
            (None, Ok(tempo)) => (None, tempo),
        };
        // A seed typed in the field is the one this roll takes, to write a roll again.
        let typed_seed = match (session.seed_typed, session.seed_text.trim().parse::<u64>()) {
            (false, _) => Ok(None),
            (true, Ok(seed)) => Ok(Some(seed)),
            (true, Err(_)) => Err("The seed is a whole number, 0 or more".to_owned()),
        };
        let (refused, typed_seed) = match (refused, typed_seed) {
            (Some(refused), _) | (None, Err(refused)) => (Some(refused), None),
            (None, Ok(seed)) => (None, seed),
        };
        if let Some(refused) = refused {
            if let Some(session) = self.jam.session.as_mut() {
                session.refused = Some(sentence(&refused));
            }
            return;
        }
        let keep: Vec<ChannelId> = match only {
            Some(rolled) => channels
                .iter()
                .map(|jammed| jammed.channel)
                .filter(|&channel| channel != rolled)
                .collect(),
            None => session
                .channels
                .iter()
                .filter(|row| row.locked)
                .map(|row| row.channel)
                .collect(),
        };
        let previous = session.last.clone().filter(|_| !keep.is_empty());
        let harmony = previous.as_ref().and_then(|previous| {
            previous
                .parts
                .iter()
                .any(|part| keep.contains(&part.channel) && !matches!(part.role, Role::Drums(_)))
                .then_some(previous.progression)
        });
        let (genre, key, bars, swing) = (session.genre, session.key, session.bars, session.swing);
        let replace = session
            .pattern
            .filter(|id| project.patterns.contains_key(id));
        let name = match replace.and_then(|id| project.patterns.get(&id)) {
            Some(pattern) => pattern.name.clone(),
            None => jam::pattern_name(project),
        };
        let ppq = project.ppq;
        let seed = match typed_seed {
            Some(seed) => seed,
            None => self.take_seed(genre, key.scale, harmony),
        };
        let request = JamRequest {
            genre,
            key,
            bars,
            seed,
            ppq,
            channels,
        };
        let mut pattern = jam::write(&request);
        if let Some(previous) = &previous {
            pattern = pattern.keeping(previous, &keep);
        }
        pattern.swing_tenths = swing;
        if let Some(tempo) = tempo {
            pattern.tempo = tempo;
        }
        let edit = Edit::Jam {
            pattern_name: name.clone(),
            replace,
            pattern: pattern.clone(),
            set_tempo: true,
            key,
        };
        self.finish_drag();
        match self.session.apply(edit) {
            Ok(()) => {
                let id = replace.or_else(|| {
                    let patterns = &self.session.project().patterns;
                    patterns
                        .iter()
                        .rev()
                        .find(|(_, shown)| shown.name == name)
                        .map(|(&id, _)| id)
                });
                if let Some(session) = self.jam.session.as_mut() {
                    // A part waiting for a roll is written by this one unless a lock or another
                    // channel's die kept its part from the roll before.
                    let kept_before = |channel: ChannelId| {
                        keep.contains(&channel)
                            && previous.as_ref().is_some_and(|previous| {
                                previous.parts.iter().any(|part| part.channel == channel)
                            })
                    };
                    for row in &mut session.channels {
                        if !kept_before(row.channel) {
                            row.pending = false;
                        }
                    }
                    session.pattern = id;
                    session.rolled_tempo = session.auto_tempo.then_some(pattern.tempo);
                    session.last = Some(pattern);
                    session.seed = Some(seed);
                    session.seed_text = seed.to_string();
                    session.seed_typed = false;
                    session.refused = None;
                }
                self.after_edit(Ok(()));
                if let Some(id) = id {
                    self.select_pattern(id);
                }
            }
            Err(error) => {
                if let Some(session) = self.jam.session.as_mut() {
                    session.refused = Some(sentence(&error.to_string()));
                }
            }
        }
    }

    /// The rack's channels that play notes and are not in the Jam, in the rack's order (the
    /// audio takes left out, as the prompt leaves them).
    fn jam_free_channels(&self) -> Vec<ChannelChoice> {
        let Some(session) = &self.jam.session else {
            return Vec::new();
        };
        self.session
            .project()
            .channels
            .iter()
            .filter(|(id, channel)| {
                channel.plays_notes()
                    && !crate::ui::rack_rows::is_take(channel)
                    && session.channels.iter().all(|row| row.channel != **id)
            })
            .map(|(&channel, shown)| ChannelChoice {
                channel,
                name: shown.name.clone(),
            })
            .collect()
    }

    /// The part `channel` plays when it joins the Jam window: the one found for it, else the
    /// melody (a sound the app knows nothing of, which its role menu then changes).
    fn joining_role(&self, channel: ChannelId) -> Role {
        self.session
            .project()
            .channels
            .get(&channel)
            .and_then(|shown| self.channel_roles(shown).0)
            .unwrap_or(Role::Melody)
    }

    /// `role` as `channel` plays it: drums on its own drum keys, any other part as it is.
    fn role_on(&self, channel: &Channel, role: Role) -> Role {
        match role {
            Role::Drums(_) => Role::Drums(self.channel_roles(channel).1),
            other => other,
        }
    }

    /// A row's instrument menu: the row's part moves from channel `from` to `to` (which must be
    /// on the rack and not in the Jam), fitted to `to`'s keys and articulation and on its drum
    /// keys ([`JamPattern::moved`]); the Jam pattern open to rolls is written again with it at
    /// once, one undo step, so the switch is heard. Before a roll, or after Accept, only the row
    /// changes.
    fn switch_jam_instrument(&mut self, from: ChannelId, to: ChannelId) {
        let project = self.session.project();
        let Some(session) = &self.jam.session else {
            return;
        };
        let Some(target) = project.channels.get(&to) else {
            return;
        };
        if from == to || session.channels.iter().any(|row| row.channel == to) {
            return;
        }
        let Some(row) = session.channels.iter().find(|row| row.channel == from) else {
            return;
        };
        let moved_to = JamChannel {
            channel: to,
            role: self.role_on(target, row.role),
            range: self.key_range(target),
            articulation: articulation_of(target, self.set_kind(target)),
        };
        let name = target.name.clone();
        let moved = session.last.clone().map(|last| last.moved(from, &moved_to));
        let Some(session) = self.jam.session.as_mut() else {
            return;
        };
        if let Some(row) = session.channels.iter_mut().find(|row| row.channel == from) {
            row.channel = to;
            row.name = name;
            row.role = moved_to.role;
        }
        // The rack's order, which the next roll writes the channels in.
        session.channels.sort_by_key(|row| row.channel);
        if let Some(moved) = moved {
            self.rewrite_jam(moved);
        }
    }

    /// A row's "×": the channel leaves the Jam, and its part leaves the Jam pattern open to
    /// rolls at once, one undo step.
    fn remove_from_jam(&mut self, channel: ChannelId) {
        let Some(session) = self.jam.session.as_mut() else {
            return;
        };
        session.channels.retain(|row| row.channel != channel);
        let Some(mut pattern) = session.last.clone() else {
            return;
        };
        if pattern.parts.iter().all(|part| part.channel != channel) {
            return;
        }
        pattern.parts.retain(|part| part.channel != channel);
        self.rewrite_jam(pattern);
    }

    /// "+ Add instrument…": `channel` joins the Jam with the part found for it, in the rack's
    /// order, waiting for the next roll to write it ("roll to hear").
    fn add_to_jam(&mut self, channel: ChannelId) {
        let role = self.joining_role(channel);
        let Some(shown) = self.session.project().channels.get(&channel) else {
            return;
        };
        let name = shown.name.clone();
        let Some(session) = self.jam.session.as_mut() else {
            return;
        };
        if session.channels.iter().any(|row| row.channel == channel) {
            return;
        }
        session.channels.push(JamRow {
            channel,
            name,
            role,
            locked: false,
            pending: true,
        });
        session.channels.sort_by_key(|row| row.channel);
    }

    /// A row's role menu: the part `channel` plays from the next roll on, which it waits for
    /// ("roll to hear"); drums on the channel's own drum keys.
    fn change_jam_role(&mut self, channel: ChannelId, name: RoleName) {
        let Some(shown) = self.session.project().channels.get(&channel) else {
            return;
        };
        let role = match name {
            RoleName::Drums => self.role_on(shown, Role::Drums(DrumMap::GM)),
            RoleName::Bass => Role::Bass,
            RoleName::Chords => Role::Chords,
            RoleName::Melody => Role::Melody,
        };
        let Some(row) = self.jam.session.as_mut().and_then(|session| {
            session
                .channels
                .iter_mut()
                .find(|row| row.channel == channel)
        }) else {
            return;
        };
        if row.role != role {
            row.role = role;
            row.pending = true;
        }
    }

    /// The Jam pattern open to rolls written again as `pattern` (a part moved or dropped), its
    /// swing, length, key and tempo as they are: one undo step. Without a pattern open to rolls
    /// (none rolled, or Accepted), `pattern` is only what the next roll's locks keep.
    fn rewrite_jam(&mut self, mut pattern: JamPattern) {
        let project = self.session.project();
        let Some(session) = &self.jam.session else {
            return;
        };
        let open = session
            .pattern
            .and_then(|id| Some((id, project.patterns.get(&id)?)));
        let Some((id, shown)) = open else {
            if let Some(session) = self.jam.session.as_mut() {
                session.last = Some(pattern);
            }
            return;
        };
        pattern.swing_tenths = shown.swing_tenths;
        let edit = Edit::Jam {
            pattern_name: shown.name.clone(),
            replace: Some(id),
            pattern: pattern.clone(),
            set_tempo: false,
            key: project.key.unwrap_or(session.key),
        };
        self.finish_drag();
        match self.session.apply(edit) {
            Ok(()) => {
                if let Some(session) = self.jam.session.as_mut() {
                    session.last = Some(pattern);
                    session.refused = None;
                }
                self.after_edit(Ok(()));
            }
            Err(error) => {
                if let Some(session) = self.jam.session.as_mut() {
                    session.refused = Some(sentence(&error.to_string()));
                }
            }
        }
    }

    /// The Jam window about to open at `level`: [`crate::ui::jam::WINDOW_SIZE`] at the zoom,
    /// over the Library's column ([`App::open_jam_window`]), and fixed: the user does not
    /// resize it (its least and most size the same).
    pub(super) fn jam_window_opening(&self, level: window::Level) -> window::Settings {
        let (size, position) = self.jam.place;
        let zoom = self.zoom();
        let fixed = Some(size * zoom.factor());
        window::Settings {
            resizable: false,
            min_size: fixed,
            max_size: fixed,
            ..desktop::pop_out_settings(PopOut::Jam, size, position, level, zoom)
        }
    }

    /// The Jam window opened, or the zoom changed, on a monitor of `monitor` (in the window's
    /// own pixels): it takes its fixed size at the zoom, cut to a small screen
    /// ([`crate::ui::jam::fitted_size`]), its least and most size with it so it stays fixed.
    pub(super) fn fit_jam_window(
        &mut self,
        id: window::Id,
        monitor: Option<Size>,
    ) -> Task<Message> {
        let size = crate::ui::jam::fitted_size(monitor, self.zoom().factor());
        self.jam.place.0 = size;
        // In this order: a least size over the most the window had would be refused.
        window::set_max_size(id, None)
            .chain(window::set_min_size(id, Some(size)))
            .chain(window::set_max_size(id, Some(size)))
            .chain(window::resize(id, size))
    }

    /// Settings' Key menu: the project's key set to `root` (its scale kept, Minor for a project
    /// that had none), or no key; one undo step.
    fn set_project_root(&mut self, root: KeyRoot) {
        let now = self.session.project().key;
        let key = match root {
            KeyRoot::None => None,
            KeyRoot::Root(root) => Some(jam::KeySignature {
                root,
                scale: now.map_or(jam::Scale::Minor, |key| key.scale),
            }),
        };
        if key != now {
            self.commit(Edit::SetKey { key });
        }
    }

    /// Opens the Jam window's own window while its session is open and closes it once it is
    /// not; asks it to the app's level again when the app's windows were last asked another.
    pub(super) fn sync_jam_window(&mut self) -> Task<Message> {
        let level = pop_out_level(self.windows_above);
        let restack = self.jam.above != self.windows_above;
        self.jam.above = self.windows_above;
        match (self.jam.session.is_some(), self.jam.window) {
            (true, None) => {
                let settings = self.jam_window_opening(level);
                let (id, opened) = window::open(settings);
                self.jam.window = Some(id);
                opened.discard()
            }
            (false, Some(id)) => {
                self.jam.window = None;
                window::close(id)
            }
            (true, Some(id)) if restack => window::set_level(id, level),
            _ => Task::none(),
        }
    }

    /// The Jam window's own events: its X, its closing and Esc in it close it; its size is
    /// kept while it is open.
    pub(super) fn jam_window_message(&mut self, message: &Message) {
        match message {
            Message::Windowed(_, WindowChange::Opened { size, .. })
            | Message::Windowed(_, WindowChange::Resized(size)) => {
                self.jam.place.0 = *size;
            }
            Message::WindowCloseRequested(_) | Message::Escape(_) => self.close_jam_window(),
            Message::WindowClosed(_) => {
                self.jam.window = None;
                self.close_jam_window();
            }
            _ => {}
        }
    }

    /// What the Jam window shows: its rows while it is open, its ground alone in the moment
    /// between its closing and its window going.
    pub(super) fn jam_window_view(&self) -> Element<'_, Message> {
        let Some(session) = &self.jam.session else {
            return iced::widget::container(iced::widget::space())
                .width(iced::Fill)
                .height(iced::Fill)
                .style(crate::ui::style::panel)
                .into();
        };
        let tempo = if session.auto_tempo {
            format_tempo(session.rolled_tempo.unwrap_or(session.genre.table().tempo))
        } else {
            session.tempo.clone()
        };
        let project = self.session.project();
        let rolled = session
            .pattern
            .and_then(|id| project.patterns.get(&id))
            .map(|pattern| pattern.name.as_str());
        let status = match (&session.refused, rolled) {
            (Some(refused), _) => refused.clone(),
            (None, Some(name)) => format!("{name}: a roll replaces it until you Accept"),
            (None, None) => format!(
                "A roll writes a new pattern, {}",
                jam::pattern_name(project)
            ),
        };
        let free = self.jam_free_channels();
        let addable = free
            .iter()
            .map(|choice| AddChoice {
                channel: choice.channel,
                name: choice.name.clone(),
                role: RoleName::of(self.joining_role(choice.channel)),
            })
            .collect();
        JamPage {
            genre: session.genre,
            auto_tempo: session.auto_tempo,
            tempo,
            key: session.key,
            bars: session.bars,
            swing: session.swing,
            channels: &session.channels,
            free,
            addable,
            seed: &session.seed_text,
            status,
            refused: session.refused.is_some(),
            rolled: rolled.is_some(),
            width: self.jam.place.0.width,
        }
        .view()
    }

    /// The Jam window's title, the main window's way: "Jam - Dawsome".
    pub(super) fn jam_title(&self) -> String {
        "Jam - Dawsome".to_owned()
    }

    /// The prompt over the main window, while it shows.
    pub(super) fn jam_prompt_view(&self) -> Option<Element<'_, Message>> {
        let rows = self.jam.prompt.as_deref()?;
        Some(crate::ui::dialog::overlay(crate::ui::jam::prompt(rows)))
    }

    /// `--show jam` and `--show jam-window`: the prompt over the main window at start, or the
    /// Jam window opened from it, its seeds counted from 1 so a shot is the same every run.
    pub fn show_jam_at_start(&mut self, window: bool) {
        self.jam.next_seed = 1;
        let _ = self.jam_message(JamMessage::Start);
        if window {
            let _ = self.jam_message(JamMessage::Next);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use dawsome_engine::jam::{DrumMap, KeySignature, PitchClass, Role, Scale, Voice};
    use dawsome_engine::project::{ChannelId, PatternId};
    use dawsome_engine::realtime::Player;
    use dawsome_engine::time::{MilliBpm, Ticks};
    use iced::{window, Element, Point, Size};

    use super::super::tests::app_on;
    use super::super::{layout_dump, widget_events, App, Message, WindowChange};
    use super::*;
    use crate::ui::jam::{JamMessage, RootChoice};

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    /// A folder of its own holding a project of every kind of channel a part is or is not found
    /// for, removed when dropped.
    struct Folder(PathBuf);

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// The project's channels, in the rack's order: an instrument channel on a set in each of
    /// the kit's folders drums-electronic, bass-synth, piano and mallets ("Beat", "Sub",
    /// "Piano", "Bells"), a sampler playing "Hat open.wav" named "Top", one playing "hit.wav"
    /// named "Snare", one playing "hit.wav" named "Vox", and an audio channel "Loop".
    const RACK: &str = r#"{"format_version":9,"sample_rate":48000,"tempo_mbpm":120000,"ppq":96,
"next_channel_id":9,"channels":[
{"id":1,"name":"Beat","kind":"instrument","instrument":"drums-electronic/drums-electronic.sfz","volume":1.0,"pan":0.0,"mute":false},
{"id":2,"name":"Sub","kind":"instrument","instrument":"bass-synth/bass-synth.sfz","volume":1.0,"pan":0.0,"mute":false},
{"id":3,"name":"Piano","kind":"instrument","instrument":"piano/piano.sfz","volume":1.0,"pan":0.0,"mute":false},
{"id":4,"name":"Bells","kind":"instrument","instrument":"mallets/mallets.sfz","volume":1.0,"pan":0.0,"mute":false},
{"id":5,"name":"Top","kind":"sampler","sample":"Hat open.wav","volume":1.0,"pan":0.0,"mute":false},
{"id":6,"name":"Snare","kind":"sampler","sample":"hit.wav","volume":1.0,"pan":0.0,"mute":false},
{"id":7,"name":"Vox","kind":"sampler","sample":"hit.wav","volume":1.0,"pan":0.0,"mute":false},
{"id":8,"name":"Loop","kind":"audio","sample":"hit.wav","volume":1.0,"pan":0.0,"mute":false}],
"patterns":[{"id":1,"name":"Pattern 1","length_bars":1,"notes":[]}]}"#;

    /// [`RACK`] written into a folder of its own named after `test`, its sets' sounds the Jam
    /// fixtures' hit, and an app on it.
    fn rack(test: &str) -> (Folder, App, Player) {
        let dir = std::env::temp_dir().join(format!("dawsome-jam-{}-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let hit = Path::new(FIXTURES).join("p36_sets/drums/hit.wav");
        for set in ["drums-electronic", "bass-synth", "piano", "mallets"] {
            let folder = dir.join(set);
            fs::create_dir_all(&folder).expect("a set folder");
            fs::copy(&hit, folder.join("hit.wav")).expect("its sound");
            fs::write(
                folder.join(format!("{set}.sfz")),
                "<region> sample=hit.wav key=60\n",
            )
            .expect("its .sfz");
        }
        for sound in ["Hat open.wav", "hit.wav"] {
            fs::copy(&hit, dir.join(sound)).expect("a sound");
        }
        let file = dir.join("rack.json");
        fs::write(&file, RACK).expect("the project");
        let (app, player) = app_on(&file);
        (Folder(dir), app, player)
    }

    /// The middle of the `nth` text run reading `words` in `view` laid out `size` big, and the
    /// box of the row holding it.
    fn find(view: Element<'_, Message>, size: Size, words: &str, nth: usize) -> (Point, Rectangle) {
        let dump = layout_dump::dump(view, size, crate::zoom::Zoom::DEFAULT, 1.0);
        let run = dump["texts"]
            .as_array()
            .expect("texts")
            .iter()
            .filter(|run| run["text"] == words)
            .nth(nth)
            .unwrap_or_else(|| panic!("no \"{words}\" ({nth}) in {dump}"));
        let rect = |entry: &serde_json::Value| {
            let at = |name: &str| entry[name].as_f64().expect("a number") as f32;
            Rectangle::new(
                Point::new(at("x"), at("y")),
                Size::new(at("width"), at("height")),
            )
        };
        let path = run["path"].as_str().expect("a path");
        let holder = path.rsplit_once('.').map_or(path, |(parent, _)| parent);
        let row = dump["boxes"]
            .as_array()
            .expect("boxes")
            .iter()
            .find(|entry| entry["path"].as_str() == Some(holder))
            .map_or_else(|| rect(run), rect);
        (rect(run).center(), row)
    }

    use iced::Rectangle;

    fn electronic() -> DrumMap {
        DrumMap {
            kick: Some(28),
            snare: Some(38),
            clap: None,
            closed_hat: Some(56),
            open_hat: Some(46),
            perc: None,
        }
    }

    fn rows(app: &App) -> Vec<(u32, Option<Role>, bool, bool)> {
        app.jam
            .prompt
            .as_ref()
            .expect("the prompt")
            .iter()
            .map(|row| (row.channel.0, row.role, row.plays_notes, row.joined))
            .collect()
    }

    // "Start a Jam" on the rack's header, clicked through the main window's widgets, opens the
    // prompt over the window with the rack's channels in its order and the part found for each
    // by the spec's rules: the electronic drum set drums on its kit map (kick 28, snare 38,
    // closed hat 56, open hat 46, no clap or perc), the synth bass the bass, the piano the
    // chords, the mallets the melody, "Hat open.wav" its one voice (the open hat, on key 60),
    // "Snare" by its name the snare; "Vox" none, off ("pick a role"); the audio channel plays no
    // notes, off. Its tick, clicked, takes Sub out; Vox's tick, clicked, does nothing while it
    // has no part; Vox's menu, opened and its Chords row clicked, gives it the chords and ticks
    // it; Next opens the Jam window on the six ticked, in
    // the rack's order, the prompt gone, its place over the Library's column: at the main
    // window's left edge, under the toolbar and 36 px for its title bar (100, 50 + 64 + 36), at
    // its fixed 600 x 560.
    #[test]
    fn the_prompt_finds_each_channels_part_and_next_opens_the_jam_window() {
        let (_folder, mut app, _player) = rack("prompt");
        let size = app.window_size;
        let mut widgets = widget_events::Window::new(&app);
        let (start, _) = find(app.view(), size, "Start a Jam", 0);
        let published = widgets.click(&mut app, start);
        assert!(
            published
                .iter()
                .any(|message| matches!(message, Message::Jam(JamMessage::Start))),
            "{published:?}"
        );
        let snare = Role::Drums(DrumMap::single(Voice::Snare));
        let open_hat = Role::Drums(DrumMap::single(Voice::OpenHat));
        assert_eq!(DrumMap::single(Voice::OpenHat).open_hat, Some(60));
        assert_eq!(
            rows(&app),
            [
                (1, Some(Role::Drums(electronic())), true, true),
                (2, Some(Role::Bass), true, true),
                (3, Some(Role::Chords), true, true),
                (4, Some(Role::Melody), true, true),
                (5, Some(open_hat), true, true),
                (6, Some(snare), true, true),
                (7, None, true, false),
                (8, None, false, false),
            ]
        );

        let (sub, _) = find(app.view(), size, "Sub", 0);
        let _ = widgets.click(&mut app, sub);
        assert_eq!(rows(&app)[1], (2, Some(Role::Bass), true, false), "Sub out");

        let (vox_tick, vox) = find(app.view(), size, "Vox", 0);
        let _ = widgets.click(&mut app, vox_tick);
        assert_eq!(rows(&app)[6], (7, None, true, false), "no part, no tick");
        let menu = Point::new(vox.x + vox.width - 60.0, vox.center_y());
        let _ = widgets.click(&mut app, menu);
        let overlays = widgets.overlays(&app);
        let list = *overlays.last().expect("the role menu open");
        let chords = Point::new(list.center_x(), list.y + list.height * 2.5 / 4.0);
        for message in widgets.choose_overlay(&app, chords) {
            let _ = app.update(message);
        }
        assert_eq!(
            rows(&app)[6],
            (7, Some(Role::Chords), true, true),
            "Vox plays chords"
        );

        app.main_position = Some(Point::new(100.0, 50.0));
        let (next, _) = find(app.view(), size, "Next", 0);
        let _ = widgets.click(&mut app, next);
        assert!(app.jam.prompt.is_none() && app.jam.window.is_some());
        let session = app.jam.session.as_ref().expect("the Jam window");
        let joined: Vec<(u32, Role)> = session
            .channels
            .iter()
            .map(|row| (row.channel.0, row.role))
            .collect();
        assert_eq!(
            joined,
            [
                (1, Role::Drums(electronic())),
                (3, Role::Chords),
                (4, Role::Melody),
                (5, open_hat),
                (6, snare),
                (7, Role::Chords),
            ]
        );
        assert_eq!(
            app.jam.place,
            (Size::new(600.0, 560.0), Some(Point::new(100.0, 150.0)))
        );
        let id = app.jam.window.expect("its window");
        assert_eq!(app.window_title(id), "Jam - Dawsome");
        let _ = app.update(Message::Escape(id));
        assert!(
            app.jam.session.is_none() && app.jam.window.is_none(),
            "Esc closes it"
        );
    }

    // "Start a Jam" is greyed with the engine's reason: a project in 3/4 (p36_waltz) "A Jam
    // writes 4/4 patterns and this project is in 3/4", an empty rack "The rack is empty: add an
    // instrument to jam with"; its message then opens nothing. Esc closes the
    // prompt, and Cancel does.
    #[test]
    fn start_is_refused_outside_4_4_and_on_an_empty_rack() {
        let (mut waltz, _player) = app_on(&Path::new(FIXTURES).join("p36_waltz.json"));
        assert_eq!(
            waltz.jam_refusal().as_deref(),
            Some("A Jam writes 4/4 patterns and this project is in 3/4")
        );
        let _ = waltz.update(Message::Jam(JamMessage::Start));
        assert!(waltz.jam.prompt.is_none());
        let dir = std::env::temp_dir().join(format!("dawsome-jam-{}-empty", std::process::id()));
        fs::create_dir_all(&dir).expect("a folder");
        let folder = Folder(dir);
        let file = folder.0.join("empty.json");
        let empty = r#"{"format_version":9,"sample_rate":48000,"tempo_mbpm":120000,"ppq":96,
"next_channel_id":1,"channels":[],"patterns":[{"id":1,"name":"Pattern 1","length_bars":1,
"notes":[]}]}"#;
        fs::write(&file, empty).expect("the project");
        let (empty, _player) = app_on(&file);
        assert!(empty.session.project().channels.is_empty());
        assert_eq!(
            empty.jam_refusal().as_deref(),
            Some("The rack is empty: add an instrument to jam with")
        );
        let (mut band, _player) = app_on(&Path::new(FIXTURES).join("p36_band.json"));
        assert_eq!(band.jam_refusal(), None);
        let _ = band.update(Message::Jam(JamMessage::Start));
        assert!(band.jam.prompt.is_some());
        let _ = band.update(Message::ClearSelection);
        assert!(band.jam.prompt.is_none(), "Esc closes it");
        let _ = band.update(Message::Jam(JamMessage::Start));
        let _ = band.update(Message::Jam(JamMessage::Cancel));
        assert!(band.jam.prompt.is_none(), "Cancel closes it");
    }

    /// p36_band (Drums, Bass, Keys, Lead) with every channel given the part its name says, and
    /// the Jam window open on them, seeds from `seed`: its window's id and widgets.
    fn band_window(seed: u64) -> (App, Player, window::Id, widget_events::Window) {
        band_window_on("p36_band.json", seed)
    }

    /// As [`band_window`], on the fixture project `file`, a band of the same four channels.
    fn band_window_on(file: &str, seed: u64) -> (App, Player, window::Id, widget_events::Window) {
        let (mut app, player) = app_on(&Path::new(FIXTURES).join(file));
        let _ = app.update(Message::Jam(JamMessage::Start));
        for (channel, role) in [
            (1, RoleName::Drums),
            (2, RoleName::Bass),
            (3, RoleName::Chords),
            (4, RoleName::Melody),
        ] {
            let _ = app.update(Message::Jam(JamMessage::RoleChosen(
                ChannelId(channel),
                role,
            )));
        }
        let _ = app.update(Message::Jam(JamMessage::Next));
        app.jam.next_seed = seed;
        let id = app.jam.window.expect("the Jam window");
        let size = app.jam.place.0;
        let opened = WindowChange::Opened {
            size,
            position: None,
        };
        let _ = app.update(Message::Windowed(id, opened));
        let widgets = widget_events::Window::pop_out(&app, id, size);
        (app, player, id, widgets)
    }

    /// A press on the Jam window's text `words` (the `nth` such), through its widgets.
    fn press(
        app: &mut App,
        id: window::Id,
        widgets: &mut widget_events::Window,
        words: &str,
        nth: usize,
    ) {
        let (at, _) = find(app.view_window(id), app.jam.place.0, words, nth);
        let _ = widgets.click(app, at);
    }

    /// The patterns named "Jam …", with their ids.
    fn jams(app: &App) -> Vec<(PatternId, String)> {
        app.session
            .project()
            .patterns
            .iter()
            .filter(|(_, pattern)| pattern.name.starts_with("Jam"))
            .map(|(&id, pattern)| (id, pattern.name.clone()))
            .collect()
    }

    /// Channel `channel`'s notes in pattern `id`.
    fn notes_of(app: &App, id: PatternId, channel: u32) -> Vec<dawsome_engine::project::Note> {
        app.session.project().patterns[&id]
            .notes
            .iter()
            .filter(|note| note.channel == ChannelId(channel))
            .cloned()
            .collect()
    }

    // "Send to playlist", through the Jam window's widgets: after a roll, with the song position
    // at tick 800 (inside bar 3), one clip of Jam 1 lands on the playlist from bar 3's line,
    // 768, on the first track (the band's playlist is empty), as long as the pattern (4 bars,
    // 1536 ticks); one undo takes back the clip alone, the Jam pattern and its notes kept.
    #[test]
    fn send_places_one_clip_of_the_jam_at_the_song_position_and_undoes() {
        let (mut app, _player, id, mut widgets) = band_window(7);
        press(&mut app, id, &mut widgets, "Roll", 0);
        let rolled = app.session.project().clone();
        assert!(rolled.playlist.clips.is_empty());
        app.song_marker = Ticks(800);
        press(&mut app, id, &mut widgets, "Send to playlist", 0);
        let project = app.session.project();
        let clips: Vec<_> = project.playlist.clips.values().collect();
        assert_eq!(clips.len(), 1);
        let first_track = *project.playlist.tracks.keys().next().expect("a track");
        assert_eq!(
            *clips[0],
            dawsome_engine::project::Clip::of_pattern(
                PatternId(2),
                first_track,
                Ticks(768),
                Ticks(1536)
            )
        );
        let _ = app.update(Message::Undo);
        assert_eq!(
            app.session.project().playlist,
            rolled.playlist,
            "one undo step"
        );
        assert_eq!(
            app.session.project().patterns[&PatternId(2)].notes,
            written(7).notes(),
            "the Jam kept"
        );
    }

    // The seed field, through the window's widgets: a seed typed there (12345) is the one the
    // next roll takes, its notes those `dawsome jam --seed 12345` writes; a roll with nothing
    // typed takes the next seed instead (7, the pinned one); 12345 typed again writes the first
    // roll's notes again, note for note.
    #[test]
    fn a_typed_seed_rolls_the_same_pattern_again() {
        use iced::keyboard::key::Code;
        let (mut app, _player, id, mut widgets) = band_window(7);
        let typed = [
            ('1', Code::Digit1),
            ('2', Code::Digit2),
            ('3', Code::Digit3),
            ('4', Code::Digit4),
            ('5', Code::Digit5),
        ];
        widgets.focus(&app, crate::ui::jam::SEED_FIELD);
        widgets.type_keys(&mut app, &typed);
        press(&mut app, id, &mut widgets, "Roll", 0);
        let seed = |app: &App| app.jam.session.as_ref().expect("open").seed;
        assert_eq!(seed(&app), Some(12345));
        let first = notes_of(&app, PatternId(2), 1);
        let all = app.session.project().patterns[&PatternId(2)].notes.clone();
        assert_eq!(all, written(12345).notes());
        press(&mut app, id, &mut widgets, "Roll", 0);
        assert_eq!(seed(&app), Some(7), "nothing typed: the next seed");
        assert_ne!(notes_of(&app, PatternId(2), 1), first);
        widgets.focus(&app, crate::ui::jam::SEED_FIELD);
        widgets.type_keys(&mut app, &typed);
        press(&mut app, id, &mut widgets, "Roll", 0);
        assert_eq!(seed(&app), Some(12345));
        assert_eq!(app.session.project().patterns[&PatternId(2)].notes, all);
    }

    // The Electric Bass's case, through the Jam window: p36_band_bass's Bass plays a set whose
    // one zone spans 26..=45, so its request's range is (26, 45) (the drum set's 24..=46, the
    // tone set's 0..=127, the synth's none). A roll of seed 8 writes the pattern
    // gen_p36_expected.py's lofi_8_electric_bass saves, note for note: the bass moved down an
    // octave, every note of it inside 26..=45, where its E's fifth on 47 would sound nothing.
    #[test]
    fn a_roll_fits_the_bass_into_its_sets_keys() {
        let (mut app, _player, id, mut widgets) = band_window_on("p36_band_bass.json", 10);
        let ranges: Vec<Option<(u8, u8)>> = (1..=4)
            .map(|channel| {
                let project = app.session.project();
                app.key_range(&project.channels[&ChannelId(channel)])
            })
            .collect();
        assert_eq!(
            ranges,
            [Some((24, 46)), Some((26, 45)), Some((0, 127)), None]
        );
        press(&mut app, id, &mut widgets, "Roll", 0);
        assert_eq!(app.jam.session.as_ref().expect("open").seed, Some(10));
        let bass = notes_of(&app, PatternId(2), 2);
        assert!(!bass.is_empty() && bass.iter().all(|note| (26..=45).contains(&note.key)));
        assert_eq!(jam_notes(&app), oracle_notes("lofi_10_electric_bass"));
    }

    /// A note as the known answers compare it: (channel, tick, length, key, velocity in
    /// hundredths).
    type Compared = (u64, i64, i64, u64, i64);

    /// The Jam pattern's notes gen_p36_expected.py saves for `case`.
    fn oracle_notes(case: &str) -> Vec<Compared> {
        let text = fs::read_to_string(Path::new(FIXTURES).join("p36_expected.json"))
            .expect("p36_expected.json");
        let want: serde_json::Value = serde_json::from_str(&text).expect("parses");
        want["cases"][case]["saved"]["pattern"]["notes"]
            .as_array()
            .expect("notes")
            .iter()
            .map(|note| {
                let number = |field: &str| note[field].as_i64().expect(field);
                let velocity = note["velocity"].as_f64().expect("velocity");
                (
                    note["channel"].as_u64().expect("channel"),
                    number("tick"),
                    number("length"),
                    note.get("key")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(60),
                    (velocity * 100.0).round() as i64,
                )
            })
            .collect()
    }

    /// Pattern 2's notes, the first Jam's, as the known answers compare them.
    fn jam_notes(app: &App) -> Vec<Compared> {
        app.session.project().patterns[&PatternId(2)]
            .notes
            .iter()
            .map(|note| {
                (
                    u64::from(note.channel.0),
                    note.start.0,
                    note.length.0,
                    u64::from(note.key),
                    (f64::from(note.velocity) * 100.0).round() as i64,
                )
            })
            .collect()
    }

    // Articulation from the instrument: p36_band's synth Lead is plucked; p36_band_strings'
    // Strings set (its set.json's category) is held. A trap roll of seed 1 with the strings on the
    // melody writes gen_p36_expected.py's trap_1_sustained, note for note: its first melody note
    // held 14 steps, to the next, past trap's longest 8.
    #[test]
    fn a_strings_melody_is_held_and_a_synth_one_plucked() {
        let (band, _band_player) = app_on(&Path::new(FIXTURES).join("p36_band.json"));
        let lead = &band.session.project().channels[&ChannelId(4)];
        assert_eq!(band.set_kind(lead), None);
        assert_eq!(
            articulation_of(lead, band.set_kind(lead)),
            Articulation::Plucked
        );
        let (mut app, _player, id, mut widgets) = band_window_on("p36_band_strings.json", 1);
        let strings = app.session.project().channels[&ChannelId(4)].clone();
        assert_eq!(app.set_kind(&strings), Some(Kind::Strings));
        assert_eq!(
            articulation_of(&strings, app.set_kind(&strings)),
            Articulation::Sustained
        );
        let trap = crate::ui::jam::GenreChoice(Genre::Trap);
        let _ = app.update(Message::Jam(JamMessage::Genre(trap)));
        press(&mut app, id, &mut widgets, "Roll", 0);
        assert_eq!(app.jam.session.as_ref().expect("open").seed, Some(1));
        assert_eq!(jam_notes(&app), oracle_notes("trap_1_sustained"));
        assert_ne!(
            jam_notes(&app),
            oracle_notes("trap_1"),
            "held, not as plucked"
        );
    }

    /// What `dawsome jam --seed <seed>` writes for p36_band's four channels in lo-fi, A minor, 4
    /// bars: the pattern a roll of that seed must hold.
    fn written(seed: u64) -> JamPattern {
        jam::write(&JamRequest {
            genre: Genre::LoFi,
            key: KeySignature {
                root: PitchClass::new(9).expect("A"),
                scale: Scale::Minor,
            },
            bars: 4,
            seed,
            ppq: 96,
            channels: vec![
                (ChannelId(1), Role::Drums(DrumMap::GM)).into(),
                (ChannelId(2), Role::Bass).into(),
                (ChannelId(3), Role::Chords).into(),
                (ChannelId(4), Role::Melody).into(),
            ],
        })
    }

    // The Jam window's dice (its Roll, through its widgets): the first roll adds "Jam 1" past
    // the project's one pattern, selected, with the notes `dawsome jam --seed 7` writes (lo-fi,
    // A minor, the project's key none, so A minor), the seed shown, the genre's tempo set (78
    // BPM, Auto on), the key written (A minor) and lo-fi's swing; a second roll replaces Jam 1
    // (seed 8's notes), still one Jam; one undo puts seed 7's back. Accept, then a roll, adds
    // "Jam 2", selected, Jam 1 kept as it was.
    #[test]
    fn rolls_write_jam_1_replace_it_and_accept_moves_on_to_jam_2() {
        let (mut app, _player, id, mut widgets) = band_window(7);
        assert_eq!(app.session.project().key, None);
        press(&mut app, id, &mut widgets, "Roll", 0);
        let first = jams(&app);
        assert_eq!(first, [(PatternId(2), "Jam 1".to_owned())]);
        assert_eq!(app.pattern, Some(PatternId(2)), "selected");
        let project = app.session.project();
        assert_eq!(project.patterns[&PatternId(2)].notes, written(7).notes());
        assert_eq!(project.patterns[&PatternId(2)].swing_tenths, 580);
        assert_eq!(project.tempo, written(7).tempo, "the tempo seed 7 draws");
        assert_eq!(
            project.key,
            Some(KeySignature {
                root: PitchClass::new(9).expect("A"),
                scale: Scale::Minor
            })
        );
        let session = app.jam.session.as_ref().expect("open");
        assert_eq!(session.seed, Some(7));
        assert_eq!(session.seed_text, "7", "the seed field shows it");

        press(&mut app, id, &mut widgets, "Roll", 0);
        assert_eq!(jams(&app), first, "the same Jam pattern");
        assert_eq!(
            app.session.project().patterns[&PatternId(2)].notes,
            written(8).notes()
        );
        let _ = app.update(Message::Undo);
        assert_eq!(
            app.session.project().patterns[&PatternId(2)].notes,
            written(7).notes(),
            "one undo step"
        );

        press(&mut app, id, &mut widgets, "Accept", 0);
        press(&mut app, id, &mut widgets, "Roll", 0);
        assert_eq!(
            jams(&app),
            [
                (PatternId(2), "Jam 1".to_owned()),
                (PatternId(3), "Jam 2".to_owned())
            ]
        );
        assert_eq!(app.pattern, Some(PatternId(3)));
        assert_eq!(
            app.session.project().patterns[&PatternId(2)].notes,
            written(7).notes(),
            "Jam 1 kept"
        );
    }

    // Locks, through the window's widgets: the drums locked (row 1's Lock) keep seed 7's notes
    // through a roll whose other parts are seed 8's; the bass locked too, the roll looks for a
    // seed whose progression is the one the bass was written over, and the bass and drums stay
    // seed 7's. A channel's own dice (the melody's, row 4's) rolls it alone: every other channel
    // keeps its notes, the melody is the new seed's.
    #[test]
    fn a_lock_keeps_a_channels_notes_and_its_dice_rolls_it_alone() {
        let (mut app, _player, id, mut widgets) = band_window(7);
        press(&mut app, id, &mut widgets, "Roll", 0);
        let jam = PatternId(2);
        let drums = notes_of(&app, jam, 1);
        // The header's "Lock" is the first; row 1's the second.
        press(&mut app, id, &mut widgets, "Lock", 1);
        assert!(app.jam.session.as_ref().expect("open").channels[0].locked);
        press(&mut app, id, &mut widgets, "Roll", 0);
        assert_eq!(app.jam.session.as_ref().expect("open").seed, Some(8));
        assert_eq!(notes_of(&app, jam, 1), drums, "the drums kept");
        let eight = written(8);
        let others = |pattern: &JamPattern, channel: u32| -> Vec<dawsome_engine::project::Note> {
            pattern
                .notes()
                .into_iter()
                .filter(|note| note.channel == ChannelId(channel))
                .collect()
        };
        for channel in [2, 3, 4] {
            assert_eq!(notes_of(&app, jam, channel), others(&eight, channel));
        }

        let bass = notes_of(&app, jam, 2);
        let progression = app
            .jam
            .session
            .as_ref()
            .expect("open")
            .last
            .as_ref()
            .expect("a roll")
            .progression;
        press(&mut app, id, &mut widgets, "Lock", 2);
        press(&mut app, id, &mut widgets, "Roll", 0);
        let seed = app
            .jam
            .session
            .as_ref()
            .expect("open")
            .seed
            .expect("rolled");
        assert!(seed > 8);
        assert_eq!(
            jam::progression(Genre::LoFi, Scale::Minor, seed),
            progression
        );
        assert_eq!(notes_of(&app, jam, 1), drums);
        assert_eq!(notes_of(&app, jam, 2), bass);
        assert_eq!(notes_of(&app, jam, 4), others(&written(seed), 4));

        let kept: Vec<_> = [1, 2, 3]
            .map(|channel| notes_of(&app, jam, channel))
            .to_vec();
        let _ = app.update(Message::Jam(JamMessage::RollChannel(ChannelId(4))));
        let alone = app
            .jam
            .session
            .as_ref()
            .expect("open")
            .seed
            .expect("rolled");
        assert_eq!(
            [1, 2, 3]
                .map(|channel| notes_of(&app, jam, channel))
                .to_vec(),
            kept
        );
        assert_eq!(notes_of(&app, jam, 4), others(&written(alone), 4));
    }

    // Auto off (its tick, through the widgets), the tempo typed as 100 sets the project's tempo
    // to 100 BPM on the roll; the key's root C (its menu's message) writes C minor; a tempo the
    // project cannot take ("0") is refused in the window, the project untouched.
    #[test]
    fn the_tempo_typed_and_the_key_chosen_are_written_by_the_roll() {
        let (mut app, _player, id, mut widgets) = band_window(3);
        press(&mut app, id, &mut widgets, "Auto", 0);
        assert!(!app.jam.session.as_ref().expect("open").auto_tempo);
        let _ = app.update(Message::Jam(JamMessage::TempoEdited("100".into())));
        let _ = app.update(Message::Jam(JamMessage::Root(RootChoice(PitchClass::C))));
        press(&mut app, id, &mut widgets, "Roll", 0);
        let project = app.session.project();
        assert_eq!(project.tempo, MilliBpm(100_000));
        assert_eq!(
            project.key,
            Some(KeySignature {
                root: PitchClass::C,
                scale: Scale::Minor
            })
        );
        let state = app.session.state();
        let _ = app.update(Message::Jam(JamMessage::TempoEdited("0".into())));
        press(&mut app, id, &mut widgets, "Roll", 0);
        assert_eq!(app.session.state(), state, "nothing written");
        assert!(app.jam.session.as_ref().expect("open").refused.is_some());
    }

    // Settings' Music page: the key menu's C on a project with none sets C minor, one undo
    // step; the scale menu's Dorian keeps the root; "No key" takes it off.
    #[test]
    fn the_music_page_sets_the_projects_key() {
        let (mut app, _player) = app_on(&Path::new(FIXTURES).join("p36_band.json"));
        let send = |app: &mut App, message| {
            let _ = app.update(Message::Jam(message));
        };
        send(
            &mut app,
            JamMessage::ProjectRoot(KeyRoot::Root(PitchClass::C)),
        );
        let c = |scale| {
            Some(KeySignature {
                root: PitchClass::C,
                scale,
            })
        };
        assert_eq!(app.session.project().key, c(Scale::Minor));
        send(
            &mut app,
            JamMessage::ProjectScale(crate::ui::jam::ScaleChoice(Scale::Dorian)),
        );
        assert_eq!(app.session.project().key, c(Scale::Dorian));
        send(&mut app, JamMessage::ProjectRoot(KeyRoot::None));
        assert_eq!(app.session.project().key, None);
        let _ = app.update(Message::Undo);
        assert_eq!(app.session.project().key, c(Scale::Dorian));
    }

    /// p36_band's four (Drums, Bass and Keys on the tone set, which plays any key, Lead on
    /// Dawsome Synth) and a fifth channel, "Electric", on the bass set (one zone, 26..=45),
    /// written into a folder of its own; the Jam window open on the four with every part as its
    /// name says, seeds from `seed`, "Electric" left out (its set names no kind, so no part is
    /// found for it).
    fn five_window(
        test: &str,
        seed: u64,
    ) -> (Folder, App, Player, window::Id, widget_events::Window) {
        let dir =
            std::env::temp_dir().join(format!("dawsome-jam-v2-{}-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("a folder");
        let set = |id: u32, name: &str, set: &str| {
            format!(
                r#"{{"id":{id},"name":"{name}","kind":"instrument","instrument":"{FIXTURES}/p36_sets/{set}","volume":1.0,"pan":0.0,"mute":false}}"#
            )
        };
        let lead = r#"{"id":4,"name":"Lead","kind":"synth","processor":{"id":"dawsome.synth","params":{}},"volume":1.0,"pan":0.0,"mute":false}"#;
        let project = format!(
            r#"{{"format_version":9,"sample_rate":48000,"tempo_mbpm":120000,"ppq":96,"next_channel_id":6,"channels":[{},{},{},{lead},{}],"patterns":[{{"id":1,"name":"Pattern 1","length_bars":1,"notes":[]}}]}}"#,
            set(1, "Drums", "drums/drums.sfz"),
            set(2, "Bass", "tone/tone.sfz"),
            set(3, "Keys", "tone/tone.sfz"),
            set(5, "Electric", "bass/bass.sfz"),
        );
        let file = dir.join("five.json");
        fs::write(&file, project).expect("the project");
        let folder = Folder(dir);
        let (mut app, player) = app_on(&file);
        let _ = app.update(Message::Jam(JamMessage::Start));
        for (channel, role) in [
            (1, RoleName::Drums),
            (2, RoleName::Bass),
            (3, RoleName::Chords),
            (4, RoleName::Melody),
        ] {
            let _ = app.update(Message::Jam(JamMessage::RoleChosen(
                ChannelId(channel),
                role,
            )));
        }
        let _ = app.update(Message::Jam(JamMessage::Next));
        app.jam.next_seed = seed;
        let id = app.jam.window.expect("the Jam window");
        let size = app.jam.place.0;
        let opened = WindowChange::Opened {
            size,
            position: None,
        };
        let _ = app.update(Message::Windowed(id, opened));
        let widgets = widget_events::Window::pop_out(&app, id, size);
        (folder, app, player, id, widgets)
    }

    /// The Jam window's rows: (channel, role, waiting for a roll).
    fn table(app: &App) -> Vec<(u32, RoleName, bool)> {
        app.jam
            .session
            .as_ref()
            .expect("open")
            .channels
            .iter()
            .map(|row| (row.channel.0, RoleName::of(row.role), row.pending))
            .collect()
    }

    /// Opens the menu in table row `row` (from 0) under the header's `column` text, `offset`
    /// right of the header row's left edge for "Instrument" (its text spans the row's
    /// instrument and hint), else at that text's middle; and picks its entry `entry` of
    /// `entries`, through the window's widgets.
    fn pick_in_row(
        app: &mut App,
        id: window::Id,
        widgets: &mut widget_events::Window,
        (column, offset): (&str, f32),
        row: usize,
        (entry, entries): (usize, usize),
    ) {
        let size = app.jam.place.0;
        let (middle, header) = find(app.view_window(id), size, column, 0);
        // The header's "Lock" is the first; row `row`'s comes after it.
        let (lock, _) = find(app.view_window(id), size, "Lock", row + 1);
        let x = if column == "Instrument" {
            header.x + offset
        } else {
            middle.x
        };
        let _ = widgets.click(app, Point::new(x, lock.y));
        let list = *widgets.overlays(app).last().expect("the menu open");
        let at = Point::new(
            list.center_x(),
            list.y + list.height * (entry as f32 + 0.5) / entries as f32,
        );
        for message in widgets.choose_overlay(app, at) {
            let _ = app.update(message);
        }
    }

    /// The "×" of table row `row` (from 0), through the window's widgets.
    fn remove_row(app: &mut App, id: window::Id, widgets: &mut widget_events::Window, row: usize) {
        press(app, id, widgets, "\u{d7}", row);
    }

    /// Whether the Jam window shows "roll to hear".
    fn shows_hint(app: &App, id: window::Id) -> bool {
        let dump = layout_dump::dump(
            app.view_window(id),
            app.jam.place.0,
            crate::zoom::Zoom::DEFAULT,
            1.0,
        );
        dump["texts"]
            .as_array()
            .expect("texts")
            .iter()
            .any(|run| run["text"] == crate::ui::jam::ROLL_TO_HEAR)
    }

    // A row's instrument switched (the Bass row's menu, its second entry, the one channel not
    // in the Jam: Electric, through the window's widgets): the bass part moves to Electric at
    // once, fitted to its set's keys 26..=45, as `JamPattern::moved` writes it from seed 7's roll
    // (the engine's known answers hold a moved part to what the seed writes for its new channel),
    // every key inside 26..=45; the Bass channel plays nothing; the drums, keys and lead are as
    // they were; the row is Electric's, in the rack's order (last); one undo puts the bass back.
    #[test]
    fn switching_a_rows_instrument_moves_its_part_and_fits_it_at_once() {
        let (_folder, mut app, _player, id, mut widgets) = five_window("switch", 7);
        press(&mut app, id, &mut widgets, "Roll", 0);
        let jam = PatternId(2);
        let before: Vec<_> = [1, 2, 3, 4]
            .map(|channel| notes_of(&app, jam, channel))
            .to_vec();
        let last = app
            .jam
            .session
            .as_ref()
            .expect("open")
            .last
            .clone()
            .expect("a roll");
        pick_in_row(&mut app, id, &mut widgets, ("Instrument", 85.0), 1, (1, 2));
        let electric = JamChannel {
            channel: ChannelId(5),
            role: Role::Bass,
            range: Some((26, 45)),
            articulation: Articulation::Plucked,
        };
        let want: Vec<_> = last
            .moved(ChannelId(2), &electric)
            .notes()
            .into_iter()
            .filter(|note| note.channel == ChannelId(5))
            .collect();
        let moved = notes_of(&app, jam, 5);
        assert!(!moved.is_empty() && moved.iter().all(|note| (26..=45).contains(&note.key)));
        assert_eq!(moved, want);
        assert!(notes_of(&app, jam, 2).is_empty());
        for (channel, was) in [(1, &before[0]), (3, &before[2]), (4, &before[3])] {
            assert_eq!(&notes_of(&app, jam, channel), was, "channel {channel}");
        }
        assert_eq!(
            table(&app),
            [
                (1, RoleName::Drums, false),
                (3, RoleName::Chords, false),
                (4, RoleName::Melody, false),
                (5, RoleName::Bass, false),
            ]
        );
        let _ = app.update(Message::Undo);
        assert_eq!(notes_of(&app, jam, 2), before[1], "one undo step");
        assert!(notes_of(&app, jam, 5).is_empty());
    }

    // A row's "×" (the Keys row's, through the window's widgets): its chords leave the Jam
    // pattern at once and the row the table; the others' notes stay; one undo puts them back.
    #[test]
    fn removing_a_row_drops_its_part_at_once() {
        let (_folder, mut app, _player, id, mut widgets) = five_window("remove", 7);
        press(&mut app, id, &mut widgets, "Roll", 0);
        let jam = PatternId(2);
        let before: Vec<_> = [1, 2, 3, 4]
            .map(|channel| notes_of(&app, jam, channel))
            .to_vec();
        assert!(!before[2].is_empty());
        remove_row(&mut app, id, &mut widgets, 2);
        assert!(notes_of(&app, jam, 3).is_empty(), "the chords gone");
        for (channel, was) in [(1, &before[0]), (2, &before[1]), (4, &before[3])] {
            assert_eq!(&notes_of(&app, jam, channel), was, "channel {channel}");
        }
        assert_eq!(
            table(&app),
            [
                (1, RoleName::Drums, false),
                (2, RoleName::Bass, false),
                (4, RoleName::Melody, false),
            ]
        );
        let _ = app.update(Message::Undo);
        assert_eq!(notes_of(&app, jam, 3), before[2], "one undo step");
    }

    // "+ Add instrument…" (its one entry, "Electric · Melody": no part is found for its set, so
    // it joins on the melody), through the window's widgets: a row joins in the rack's order
    // showing "roll to hear", and the Jam pattern is as it was; the next roll (seed 8) writes it,
    // the hint gone, its notes what seed 8 writes for the five in the rack's order (Electric's
    // melody fitted to 26..=45).
    #[test]
    fn an_added_instrument_waits_for_the_next_roll() {
        let (_folder, mut app, _player, id, mut widgets) = five_window("add", 7);
        press(&mut app, id, &mut widgets, "Roll", 0);
        let jam = PatternId(2);
        let before = app.session.project().patterns[&jam].notes.clone();
        assert!(!shows_hint(&app, id));
        let size = app.jam.place.0;
        let (_, header) = find(app.view_window(id), size, "Instrument", 0);
        let (last_lock, _) = find(app.view_window(id), size, "Lock", 4);
        let add = Point::new(header.x + 100.0, last_lock.y + 32.0);
        let _ = widgets.click(&mut app, add);
        let list = *widgets.overlays(&app).last().expect("the add menu open");
        for message in widgets.choose_overlay(&app, list.center()) {
            let _ = app.update(message);
        }
        assert_eq!(table(&app)[4], (5, RoleName::Melody, true));
        assert!(shows_hint(&app, id), "roll to hear");
        assert_eq!(
            app.session.project().patterns[&jam].notes,
            before,
            "nothing rolled"
        );
        press(&mut app, id, &mut widgets, "Roll", 0);
        assert!(table(&app).iter().all(|&(_, _, pending)| !pending));
        assert!(!shows_hint(&app, id));
        let want = jam::write(&JamRequest {
            genre: Genre::LoFi,
            key: KeySignature {
                root: PitchClass::new(9).expect("A"),
                scale: Scale::Minor,
            },
            bars: 4,
            seed: 8,
            ppq: 96,
            channels: vec![
                JamChannel {
                    range: Some((24, 46)),
                    ..(ChannelId(1), Role::Drums(DrumMap::GM)).into()
                },
                JamChannel {
                    range: Some((0, 127)),
                    ..(ChannelId(2), Role::Bass).into()
                },
                JamChannel {
                    range: Some((0, 127)),
                    ..(ChannelId(3), Role::Chords).into()
                },
                (ChannelId(4), Role::Melody).into(),
                JamChannel {
                    range: Some((26, 45)),
                    ..(ChannelId(5), Role::Melody).into()
                },
            ],
        });
        assert_eq!(app.session.project().patterns[&jam].notes, want.notes());
    }

    // A row's role changed (Keys' menu, its Melody entry, through the window's widgets): the
    // row says Melody and "roll to hear", the Jam pattern is as it was; the next roll (seed 8)
    // writes Keys a melody (as seed 8 writes the four with Keys on the melody).
    #[test]
    fn a_role_changed_waits_for_the_next_roll() {
        let (_folder, mut app, _player, id, mut widgets) = five_window("role", 7);
        press(&mut app, id, &mut widgets, "Roll", 0);
        let jam = PatternId(2);
        let before = app.session.project().patterns[&jam].notes.clone();
        pick_in_row(&mut app, id, &mut widgets, ("Role", 0.0), 2, (3, 4));
        assert_eq!(table(&app)[2], (3, RoleName::Melody, true));
        assert!(shows_hint(&app, id));
        assert_eq!(
            app.session.project().patterns[&jam].notes,
            before,
            "nothing rolled"
        );
        press(&mut app, id, &mut widgets, "Roll", 0);
        assert_eq!(table(&app)[2], (3, RoleName::Melody, false));
        let keys = notes_of(&app, jam, 3);
        let want: Vec<_> = jam::write(&JamRequest {
            genre: Genre::LoFi,
            key: KeySignature {
                root: PitchClass::new(9).expect("A"),
                scale: Scale::Minor,
            },
            bars: 4,
            seed: 8,
            ppq: 96,
            channels: vec![
                JamChannel {
                    range: Some((24, 46)),
                    ..(ChannelId(1), Role::Drums(DrumMap::GM)).into()
                },
                JamChannel {
                    range: Some((0, 127)),
                    ..(ChannelId(2), Role::Bass).into()
                },
                JamChannel {
                    range: Some((0, 127)),
                    ..(ChannelId(3), Role::Melody).into()
                },
                (ChannelId(4), Role::Melody).into(),
            ],
        })
        .notes()
        .into_iter()
        .filter(|note| note.channel == ChannelId(3))
        .collect();
        assert_eq!(keys, want);
    }

    // The window's size: it opens 600 x 560 at a zoom of 100 %, the user unable to resize it
    // (not resizable, its least and most size that size); a monitor of 1366 x 600 cuts it to
    // 600 x 520 (600 less the 80 the desktop keeps), least and most with it; one of 1920 x 1080
    // leaves it whole. The seed's die draws a seed into the field without rolling, and the next
    // roll takes it.
    #[test]
    fn the_window_is_fixed_and_shrinks_to_a_small_screen() {
        let (_folder, mut app, _player, id, _widgets) = five_window("size", 7);
        let opening = app.jam_window_opening(window::Level::Normal);
        assert_eq!(opening.size, Size::new(600.0, 560.0));
        assert!(!opening.resizable, "the user does not resize it");
        assert_eq!(opening.min_size, Some(Size::new(600.0, 560.0)));
        assert_eq!(opening.max_size, opening.min_size);
        let _ = app.update(Message::PopOutMonitor {
            window: id,
            size: Size::new(600.0, 560.0),
            monitor: Some(Size::new(1366.0, 600.0)),
        });
        assert_eq!(app.jam.place.0, Size::new(600.0, 520.0));
        let opening = app.jam_window_opening(window::Level::Normal);
        assert_eq!(opening.min_size, Some(Size::new(600.0, 520.0)));
        let _ = app.update(Message::PopOutMonitor {
            window: id,
            size: Size::new(600.0, 520.0),
            monitor: Some(Size::new(1920.0, 1080.0)),
        });
        assert_eq!(app.jam.place.0, Size::new(600.0, 560.0));

        let _ = app.update(Message::Jam(JamMessage::NewSeed));
        let session = app.jam.session.as_ref().expect("open");
        assert!(
            session.seed_typed && session.last.is_none(),
            "drawn, not rolled"
        );
        let drawn: u64 = session.seed_text.parse().expect("a seed");
        let _ = app.update(Message::Jam(JamMessage::Roll));
        assert_eq!(app.jam.session.as_ref().expect("open").seed, Some(drawn));
    }
}
