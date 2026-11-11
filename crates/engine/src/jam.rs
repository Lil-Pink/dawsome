//! Jam: a pattern written for the instruments on the rack, from a genre's table, the project's
//! key and a seed. Pure: the one source of chance is [`JamRng`], so a seed gives the same pattern
//! on every machine, and tests/fixtures/gen_p36_expected.py works the same patterns out on its
//! own, draw for draw.
//!
//! The draw order, which the oracle follows:
//! - the progression first: `below(count)` over the key family's progressions, even when no
//!   channel plays harmony; then the tempo, `low + below(high - low + 1)` over the genre's range;
//!   then one variant of each part in [`PARTS`] order, `below(n)` over its variants (no draw for
//!   a part with one), every part's whether or not a channel plays it;
//! - then each channel in the request's order (rack order), each bar left to right;
//! - drums: the voices kick, snare, clap, closed hat, open hat, perc, a voice the channel's map
//!   lacks drawing nothing; each voice's [`Part`]s in order, each [`Hit`] in its list's order:
//!   its chance or its drop, then (when it plays) its velocity jitter;
//! - chords and bass: their parts per bar, as a drum voice's;
//! - melody per bar: the count, the rest-light chance, the starts (a repeat drawn again), the
//!   late steps in start order; then per note in time order its pitch move (none for the
//!   channel's first note) and its velocity.
//!
//! No draw is made for a value the table fixes. Every velocity is held to 0.30..1.00 after its
//! jitter: some genres' bases jitter below 0.30 otherwise.

mod genres;

use std::collections::BTreeMap;
use std::str::FromStr;

use crate::project::{ChannelId, Note, Project, DEFAULT_KEY};
pub use crate::project::{KeyError, KeySignature, PitchClass, Scale};
use crate::time::{MilliBpm, Ticks};

/// Steps a bar: a Jam writes 4/4 only.
pub const STEPS_PER_BAR: u8 = 16;

/// A Jam's PPQ is a multiple of this: a hat roll's half step is then whole ticks (at PPQ 4 it
/// would be 0, and the roll would replace the hits it doubles).
pub const PPQ_MULTIPLE: u16 = 8;

/// The lowest and highest velocity a Jam writes, in hundredths.
pub const VELOCITY_RANGE: std::ops::RangeInclusive<u8> = 30..=100;

/// A melody note's velocity: 0.55 plus `jitter(20)` hundredths.
const MELODY_VELOCITY: u8 = 55;
const MELODY_VELOCITY_SPREAD: u32 = 20;

/// The bands notes are placed in: a chord's root, the bass; a chord note above `CHORD_TOP`
/// drops an octave. The melody's band is each genre's ([`MelodyTable::low`]).
const CHORD_ROOT_LOW: u8 = 48;
const CHORD_TOP: u8 = 71;
const BASS_LOW: u8 = 36;

/// The lowest key the bass's octave down goes to; above it the root stays.
const BASS_FLOOR: u8 = 24;

/// The golden ratio's 64 bits: splitmix64's step and the genres' salts.
const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;

/// splitmix64's mix of `x`: one input bit changes about half the output's. Why a Jam's seed goes
/// through it: xorshift64* started from seeds 1, 2, 3 starts from nearly the same bits, and its
/// first draws (the progression, the tempo, the variants) came out alike.
pub fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(GOLDEN);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The one source of chance in a Jam: xorshift64*, its upper 32 bits per draw.
#[derive(Debug, Clone)]
pub struct JamRng {
    state: u64,
}

impl JamRng {
    /// xorshift64* from `state` as it is; 0, where xorshift would stay, starts from [`GOLDEN`].
    pub fn new(state: u64) -> Self {
        let state = if state == 0 { GOLDEN } else { state };
        Self { state }
    }

    /// The generator a Jam of `genre` from `seed` draws from: its state
    /// `splitmix64(seed ^ salt)`, the salt the genre's place in [`Genre::ALL`] from 1 times
    /// [`GOLDEN`], so one seed rolls unlike patterns in two genres.
    pub fn for_jam(genre: Genre, seed: u64) -> Self {
        let place = Genre::ALL
            .iter()
            .position(|&each| each == genre)
            .unwrap_or(0) as u64;
        Self::new(splitmix64(seed ^ (place + 1).wrapping_mul(GOLDEN)))
    }

    pub fn next_u32(&mut self) -> u32 {
        let mut s = self.state;
        s ^= s >> 12;
        s ^= s << 25;
        s ^= s >> 27;
        self.state = s;
        // The upper half of the product: its low bits are the weaker.
        (s.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32) as u32
    }

    /// A number in `0..n`; `n` is at least 1.
    pub fn below(&mut self, n: u32) -> u32 {
        self.next_u32() % n
    }

    /// True `percent` times in 100.
    pub fn chance(&mut self, percent: u8) -> bool {
        self.below(100) < u32::from(percent)
    }

    /// A number in `0..range`, which the caller scales.
    pub fn jitter(&mut self, range: u32) -> u32 {
        self.below(range)
    }
}

/// A drum voice a channel may play.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Voice {
    Kick,
    Snare,
    Clap,
    ClosedHat,
    OpenHat,
    Perc,
}

impl Voice {
    /// The order the voices draw in, each bar.
    pub const ALL: [Voice; 6] = [
        Voice::Kick,
        Voice::Snare,
        Voice::Clap,
        Voice::ClosedHat,
        Voice::OpenHat,
        Voice::Perc,
    ];

    /// The word the command line takes.
    pub const fn id(self) -> &'static str {
        match self {
            Voice::Kick => "kick",
            Voice::Snare => "snare",
            Voice::Clap => "clap",
            Voice::ClosedHat => "chat",
            Voice::OpenHat => "ohat",
            Voice::Perc => "perc",
        }
    }

    pub fn from_id(word: &str) -> Option<Voice> {
        Voice::ALL.into_iter().find(|voice| voice.id() == word)
    }
}

/// The key each drum voice plays on a channel; `None`: the channel has no such voice.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct DrumMap {
    pub kick: Option<u8>,
    pub snare: Option<u8>,
    pub clap: Option<u8>,
    pub closed_hat: Option<u8>,
    pub open_hat: Option<u8>,
    pub perc: Option<u8>,
}

impl DrumMap {
    /// General MIDI's drum keys; perc is the cross-stick.
    pub const GM: DrumMap = DrumMap {
        kick: Some(36),
        snare: Some(38),
        clap: Some(39),
        closed_hat: Some(42),
        open_hat: Some(46),
        perc: Some(37),
    };

    /// A channel that is one voice, a sample played at its own pitch.
    pub fn single(voice: Voice) -> Self {
        DrumMap::default().with(voice, Some(DEFAULT_KEY))
    }

    pub fn key(&self, voice: Voice) -> Option<u8> {
        match voice {
            Voice::Kick => self.kick,
            Voice::Snare => self.snare,
            Voice::Clap => self.clap,
            Voice::ClosedHat => self.closed_hat,
            Voice::OpenHat => self.open_hat,
            Voice::Perc => self.perc,
        }
    }

    /// This map with `voice` on `key` (or without it, for `None`).
    pub fn with(mut self, voice: Voice, key: Option<u8>) -> Self {
        let slot = match voice {
            Voice::Kick => &mut self.kick,
            Voice::Snare => &mut self.snare,
            Voice::Clap => &mut self.clap,
            Voice::ClosedHat => &mut self.closed_hat,
            Voice::OpenHat => &mut self.open_hat,
            Voice::Perc => &mut self.perc,
        };
        *slot = key;
        self
    }
}

/// What a channel plays in a Jam.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Drums(DrumMap),
    Bass,
    Chords,
    Melody,
}

impl Role {
    /// The word the command line takes and prints.
    pub const fn id(&self) -> &'static str {
        match self {
            Role::Drums(_) => "drums",
            Role::Bass => "bass",
            Role::Chords => "chords",
            Role::Melody => "melody",
        }
    }
}

/// A genre: which table a Jam is written from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Genre {
    LoFi,
    BoomBap,
    Trap,
    Drill,
    House,
    Phonk,
    Afrobeats,
}

impl Genre {
    pub const ALL: [Genre; 7] = [
        Genre::LoFi,
        Genre::BoomBap,
        Genre::Trap,
        Genre::Drill,
        Genre::House,
        Genre::Phonk,
        Genre::Afrobeats,
    ];

    /// The genre's numbers, read from genres.json the first time any genre's are asked for.
    pub fn table(self) -> &'static GenreTable {
        let index = Genre::ALL
            .iter()
            .position(|&genre| genre == self)
            .unwrap_or(0);
        &genres::tables()[index]
    }

    /// The word the command line takes.
    pub const fn id(self) -> &'static str {
        match self {
            Genre::LoFi => "lofi",
            Genre::BoomBap => "boombap",
            Genre::Trap => "trap",
            Genre::Drill => "drill",
            Genre::House => "house",
            Genre::Phonk => "phonk",
            Genre::Afrobeats => "afrobeats",
        }
    }

    /// The name shown to the user.
    pub fn name(self) -> &'static str {
        &self.table().name
    }
}

/// Why a genre's word was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("'{0}' is not a genre: lofi, boombap, trap, drill, house (or edm), phonk or afrobeats")]
pub struct UnknownGenre(pub String);

impl FromStr for Genre {
    type Err = UnknownGenre;

    /// [`Genre::id`], in either case; "edm" is house, as the menu shows it.
    fn from_str(text: &str) -> Result<Self, UnknownGenre> {
        let word = text.to_ascii_lowercase();
        if word == "edm" {
            return Ok(Genre::House);
        }
        Genre::ALL
            .into_iter()
            .find(|genre| genre.id() == word)
            .ok_or_else(|| UnknownGenre(text.to_owned()))
    }
}

/// A genre's numbers: tempo, swing, harmony, and for every voice and part the hits it may play.
/// Data only, read from genres.json (the one place they are tuned): [`write`] reads every genre
/// through the same code.
#[derive(Debug)]
pub struct GenreTable {
    /// The name shown to the user.
    pub name: String,
    /// The tempo a Jam sets with auto tempo on.
    pub tempo: MilliBpm,
    /// The tempos that suit the genre, in whole BPM, for the window's tempo box.
    pub tempo_range: std::ops::RangeInclusive<u32>,
    pub swing_tenths: u16,
    /// Chords with their seventh.
    pub sevenths: bool,
    /// Progressions (scale degrees from 1, a chord a bar) for a minor-family key.
    pub minor_progressions: Vec<[u8; 4]>,
    /// And for a major-family key.
    pub major_progressions: Vec<[u8; 4]>,
    /// Each part's variants, a roll playing one of each.
    pub kick: Vec<Variant>,
    pub snare: Vec<Variant>,
    pub clap: Vec<Variant>,
    pub closed_hat: Vec<Variant>,
    pub open_hat: Vec<Variant>,
    pub perc: Vec<Variant>,
    pub chords: Vec<Variant>,
    pub bass: Vec<Variant>,
    pub melody: Vec<MelodyTable>,
}

/// The parts a roll picks a variant of, in the order it draws them: [`JamPattern::variants`]'s.
pub const PARTS: [&str; 9] = [
    "kick",
    "snare",
    "clap",
    "closed_hat",
    "open_hat",
    "perc",
    "chords",
    "bass",
    "melody",
];

/// One way a drum voice, the chords or the bass may go in a genre.
#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    /// How it goes, in plain words.
    pub note: String,
    /// Drawn in order each bar.
    pub parts: Vec<Part>,
}

impl GenreTable {
    pub fn voice(&self, voice: Voice) -> &[Variant] {
        match voice {
            Voice::Kick => &self.kick,
            Voice::Snare => &self.snare,
            Voice::Clap => &self.clap,
            Voice::ClosedHat => &self.closed_hat,
            Voice::OpenHat => &self.open_hat,
            Voice::Perc => &self.perc,
        }
    }

    /// How many variants each part has, in [`PARTS`] order.
    pub fn variant_counts(&self) -> [usize; 9] {
        [
            self.kick.len(),
            self.snare.len(),
            self.clap.len(),
            self.closed_hat.len(),
            self.open_hat.len(),
            self.perc.len(),
            self.chords.len(),
            self.bass.len(),
            self.melody.len(),
        ]
    }

    /// The progressions a key of `scale` draws from.
    pub fn progressions(&self, scale: Scale) -> &[[u8; 4]] {
        if scale.is_minor_family() {
            &self.minor_progressions
        } else {
            &self.major_progressions
        }
    }
}

/// One part of a voice (or of the chords or the bass) in a bar, and how far below their base
/// its velocities may fall.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub kind: PartKind,
    /// A jittered velocity is its base less `jitter(this)` hundredths: 16 unless the file
    /// says, at least 1.
    pub jitter: u8,
}

/// Which hits a part plays.
#[derive(Debug, Clone, PartialEq)]
pub enum PartKind {
    /// Hits drawn one by one.
    Hits(Vec<Hit>),
    /// One chance draw for the bar: on a hit the first list, else the second.
    Either {
        chance: u8,
        then: Vec<Hit>,
        otherwise: Vec<Hit>,
    },
    /// Hits on the pattern's last bar only.
    LastBar(Vec<Hit>),
}

/// A hit a part may play in a bar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub step: u8,
    /// Half a step after `step` (a 32nd): a hat roll's.
    pub half: bool,
    /// In steps; 0 for a drum hit.
    pub length: u8,
    /// Which note a bass hit plays; chords and drums ignore it.
    pub pitch: Pitch,
    pub when: When,
    pub velocity: Velocity,
}

/// Whether a hit plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    /// Always, with no draw.
    Always,
    /// When `chance(p)`.
    Chance(u8),
    /// Unless `chance(p)`: a hit dropped now and then.
    Unless(u8),
}

/// A hit's velocity, in hundredths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Velocity {
    /// The base less the part's jitter draw.
    Jitter(u8),
    /// This velocity, with no draw.
    Exact(u8),
}

/// Which note of the bar's chord a bass hit plays.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pitch {
    /// The root, in 36..=47.
    #[default]
    Root,
    /// The root's pitch class plus 7, in 36..=47.
    Fifth,
    OctaveUp,
    /// An octave down when that is 24 or more, else the root.
    OctaveDown,
    /// The root plus the step to the chord scale's next degree: a passing note.
    NextDegree,
}

/// A melody's numbers.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MelodyTable {
    /// How it goes, in plain words.
    #[serde(rename = "_note")]
    pub note: String,
    pub count: Count,
    /// The steps a note may start on.
    pub starts: Vec<u8>,
    /// The chance each note starts a step late.
    #[serde(default)]
    pub late: Option<u8>,
    /// The band the melody walks in, `low..=high`, an octave or more wide: 60 to 72 (C4 to C5)
    /// unless the file says.
    #[serde(default = "melody_low")]
    pub low: u8,
    #[serde(default = "melody_high")]
    pub high: u8,
    /// The most steps a note lasts: it holds to the next note's start, or to the bar's end for
    /// the last, at most this.
    pub max_length: u8,
}

fn melody_low() -> u8 {
    60
}

fn melody_high() -> u8 {
    72
}

/// How many notes a melody's bar has.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Count {
    /// `least + below(spread)` notes on starts picked at random, then on `rest_light`'s chance
    /// at most its `cap`.
    Drawn {
        least: u8,
        spread: u8,
        #[serde(default)]
        rest_light: Option<RestLight>,
    },
    /// One on every start, with no draw.
    Every,
}

/// A chance that a melody's bar plays fewer notes.
#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestLight {
    pub chance: u8,
    pub cap: u8,
}

/// What [`write`] is asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JamRequest {
    pub genre: Genre,
    pub key: KeySignature,
    /// 4 or 8 in the app and the command line; the progression repeats every 4.
    pub bars: u32,
    pub seed: u64,
    /// The project's ticks per quarter: a step is `ppq / 4` ticks, a half step `ppq / 8`. The
    /// edit takes only a multiple of 8 ([`PPQ_MULTIPLE`]), so a half step is whole ticks.
    pub ppq: u16,
    /// The channels that play, each with its role and its instrument's keys, in rack order.
    pub channels: Vec<JamChannel>,
}

/// A channel a Jam writes for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JamChannel {
    pub channel: ChannelId,
    pub role: Role,
    /// The lowest and highest key its instrument plays, inclusive; `None`: any key. A chords,
    /// bass or melody part is fitted to it after its draws ([`fit`]); drums never move.
    pub range: Option<(u8, u8)>,
    /// Whether its instrument rings out or is held: a held one's melody notes last to the next.
    pub articulation: Articulation,
}

/// How an instrument plays a note: struck and dying away, or held while the key is down.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Articulation {
    /// Leads, plucks, bells, a bass, a drum: the melody as the genre writes it, rests and short
    /// notes and all.
    #[default]
    Plucked,
    /// Keys, pads, strings: a melody note held to the next start in its bar, or to the bar's
    /// end, past the genre's longest note; one drawn shorter than 2 steps as drawn. Why: on a
    /// sound that holds, the genre's rests and short notes stop and start; on one that dies
    /// away they are the groove.
    Sustained,
}

impl From<(ChannelId, Role)> for JamChannel {
    /// A plucked channel whose instrument plays any key.
    fn from((channel, role): (ChannelId, Role)) -> Self {
        JamChannel {
            channel,
            role,
            range: None,
            articulation: Articulation::Plucked,
        }
    }
}

/// What a Jam wrote: every channel's notes, and what the pattern and the project take from the
/// genre.
#[derive(Debug, Clone, PartialEq)]
pub struct JamPattern {
    /// The ticks per quarter its notes were written at; the edit refuses it in a project of
    /// another.
    pub ppq: u16,
    pub bars: u32,
    /// The scale degrees of the bars' chords, a chord a bar.
    pub progression: [u8; 4],
    pub swing_tenths: u16,
    /// The tempo drawn inside the genre's range, which the edit sets when asked.
    pub tempo: MilliBpm,
    /// The variant each part played, in [`PARTS`] order, counted from 0. A part a lock kept
    /// played the variant of the roll it was kept from.
    pub variants: [u8; 9],
    /// The melody's longest note as its table draws it, in steps: what a melody moved from a
    /// held instrument to a plucked one is cut back to ([`JamPattern::moved`]).
    pub melody_longest: u8,
    /// Each channel's notes, in the request's order.
    pub parts: Vec<JamPart>,
}

/// One channel's part of a Jam.
#[derive(Debug, Clone, PartialEq)]
pub struct JamPart {
    pub channel: ChannelId,
    pub role: Role,
    /// The notes as drawn for the channel's articulation, before [`fit`] moved them into its
    /// range: what a move to another channel fits again ([`JamPattern::moved`]). A drum part's
    /// are its notes.
    pub drawn: Vec<JamNote>,
    /// Sorted by (tick, key), one note a tick and key.
    pub notes: Vec<JamNote>,
    /// The notes [`fit`] left out: no octave of their pitch class lies in the channel's range.
    pub dropped: u32,
}

/// A note a Jam wrote; a drum hit is one of length 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JamNote {
    pub tick: Ticks,
    pub length: Ticks,
    pub key: u8,
    /// In hundredths: whole hundredths keep the oracle's numbers and the engine's the same.
    pub velocity: u8,
}

impl JamPattern {
    /// The pattern's notes as the project holds them: every part's, sorted by (tick, channel,
    /// key).
    pub fn notes(&self) -> Vec<Note> {
        let mut notes: Vec<Note> = self
            .parts
            .iter()
            .flat_map(|part| {
                part.notes.iter().map(|note| Note {
                    channel: part.channel,
                    start: note.tick,
                    length: note.length,
                    velocity: f32::from(note.velocity) / 100.0,
                    key: note.key,
                })
            })
            .collect();
        notes.sort_by_key(Note::order_key);
        notes
    }

    /// This pattern with the parts of the channels in `keep` taken from `previous` (a lock, or
    /// a roll of one channel alone): each such channel's notes and role are `previous`'s,
    /// repeated from its start when `previous` is shorter and cut at this pattern's end; a
    /// channel `previous` has no part for keeps this pattern's. Substitution only: no draw is
    /// made, so the other channels stay what their seed wrote.
    pub fn keeping(mut self, previous: &JamPattern, keep: &[ChannelId]) -> JamPattern {
        let step = i64::from(self.ppq / 4);
        let bar = i64::from(STEPS_PER_BAR) * step;
        let end = i64::from(self.bars) * bar;
        let was = i64::from(previous.bars) * bar;
        for part in &mut self.parts {
            if !keep.contains(&part.channel) {
                continue;
            }
            let Some(kept) = previous
                .parts
                .iter()
                .find(|kept| kept.channel == part.channel)
            else {
                continue;
            };
            part.role = kept.role;
            part.notes = repeated(&kept.notes, was, end);
            part.drawn = repeated(&kept.drawn, was, end);
            part.dropped = kept.dropped;
        }
        self
    }

    /// This pattern with `from`'s part played by the channel `to` instead (a row of the Jam
    /// window switched to another instrument): the part takes `to`'s channel and role, a drum
    /// part each hit on the key `to`'s drum map gives its voice (a voice the map lacks is
    /// dropped), and a pitched part its notes as drawn, a melody's lengths held or cut back for
    /// `to`'s articulation as [`write`] lengthens them, then fitted to `to`'s range ([`fit`]).
    /// Lengths and octaves only: no draw is made, so a part moved to a channel is the part the
    /// same seed writes for it. A pattern without `from`'s part is returned as it is.
    pub fn moved(mut self, from: ChannelId, to: &JamChannel) -> JamPattern {
        let step = i64::from(self.ppq / 4);
        let longest = i64::from(self.melody_longest) * step;
        let Some(part) = self.parts.iter_mut().find(|part| part.channel == from) else {
            return self;
        };
        let (drawn, notes, dropped) = match (part.role, to.role) {
            (Role::Drums(was), Role::Drums(now)) => {
                let mut hits = BTreeMap::new();
                let mut dropped = 0;
                for note in &part.drawn {
                    let voice = Voice::ALL
                        .into_iter()
                        .find(|&voice| was.key(voice) == Some(note.key));
                    match voice.and_then(|voice| now.key(voice)) {
                        Some(key) => {
                            hits.insert((note.tick, key), JamNote { key, ..*note });
                        }
                        None => dropped += 1,
                    }
                }
                let hits: Vec<JamNote> = hits.into_values().collect();
                (hits.clone(), hits, dropped)
            }
            // A drum part as another role, or the other way: the app never asks it; the keys
            // stay as they are.
            (Role::Drums(_), _) | (_, Role::Drums(_)) => {
                (part.drawn.clone(), part.notes.clone(), part.dropped)
            }
            (_, role) => {
                let drawn = if role == Role::Melody {
                    articulated(&part.drawn, to.articulation, step, longest)
                } else {
                    part.drawn.clone()
                };
                let (notes, dropped) = fit(&drawn, to.range);
                (drawn, notes, dropped)
            }
        };
        part.channel = to.channel;
        part.role = to.role;
        part.drawn = drawn;
        part.notes = notes;
        part.dropped = dropped;
        self
    }
}

/// `notes` from a pattern `was` ticks long, repeated from its start to fill `end` ticks and cut
/// there.
fn repeated(notes: &[JamNote], was: i64, end: i64) -> Vec<JamNote> {
    let mut out = Vec::new();
    let mut offset = 0;
    while was > 0 && offset < end {
        out.extend(
            notes
                .iter()
                .map(|note| JamNote {
                    tick: Ticks(note.tick.0 + offset),
                    ..*note
                })
                .filter(|note| note.tick.0 < end),
        );
        offset += was;
    }
    out
}

/// A melody's `notes` (one a start, sorted) lengthened for `articulation` as [`write`]
/// lengthens them, from either: each note's room is the gap to the next start in its bar, or to
/// the bar's end; held, a note drawn 2 steps or longer lasts its whole room; plucked, a note
/// lasts its room up to the genre's `longest` (ticks). Why from either: a held note's drawn
/// length was the room up to `longest` (a note shorter than its room was drawn under 2 steps
/// and is as drawn either way).
fn articulated(
    notes: &[JamNote],
    articulation: Articulation,
    step: i64,
    longest: i64,
) -> Vec<JamNote> {
    let bar = i64::from(STEPS_PER_BAR) * step;
    notes
        .iter()
        .enumerate()
        .map(|(i, note)| {
            let bar_end = (note.tick.0.div_euclid(bar) + 1) * bar;
            let next = notes
                .get(i + 1)
                .map_or(bar_end, |next| next.tick.0.min(bar_end));
            let room = next - note.tick.0;
            let length = note.length.0;
            let length = match articulation {
                Articulation::Sustained if length >= 2 * step => room,
                Articulation::Sustained => length,
                Articulation::Plucked if length >= room => room.min(longest),
                Articulation::Plucked => length,
            };
            JamNote {
                length: Ticks(length),
                ..*note
            }
        })
        .collect()
}

/// The progression [`write`] picks for `genre` in `scale` from `seed`: the first draw of every
/// Jam. Why apart: a roll that keeps a channel's chords, bass or melody can look for a seed that
/// picks the progression they were written over.
pub fn progression(genre: Genre, scale: Scale, seed: u64) -> [u8; 4] {
    let progressions = genre.table().progressions(scale);
    progressions[JamRng::for_jam(genre, seed).below(progressions.len() as u32) as usize]
}

/// The name of the pattern a Jam adds to `project`: "Jam N", the lowest N no pattern has.
pub fn pattern_name(project: &Project) -> String {
    (1u32..)
        .map(|n| format!("Jam {n}"))
        .find(|name| {
            project
                .patterns
                .values()
                .all(|pattern| pattern.name != *name)
        })
        .unwrap_or_else(|| "Jam".to_owned())
}

/// Semitones from the key's root to 1-based degree `degree` of `steps`; a degree past the
/// scale's length is an octave up.
fn degree_offset(steps: &[u8], degree: u8) -> i32 {
    let index = usize::from(degree.saturating_sub(1));
    i32::from(steps[index % steps.len()]) + 12 * (index / steps.len()) as i32
}

/// The pitch class of the root of the chord on `degree`.
fn chord_root_class(key: KeySignature, degree: u8) -> u8 {
    let steps = key.scale.chord_scale().steps();
    ((i32::from(key.root.get()) + degree_offset(steps, degree)).rem_euclid(12)) as u8
}

/// The chord on `degree` of `key`, low to high: stacked thirds of the chord scale, its root in
/// 48..=59, every note above 71 an octave down.
pub fn chord(key: KeySignature, degree: u8, sevenths: bool) -> Vec<u8> {
    let steps = key.scale.chord_scale().steps();
    let low = i32::from(CHORD_ROOT_LOW + chord_root_class(key, degree));
    let stacked: &[u8] = if sevenths { &[0, 2, 4, 6] } else { &[0, 2, 4] };
    let mut notes: Vec<u8> = stacked
        .iter()
        .map(|&third| {
            let note = low + degree_offset(steps, degree + third) - degree_offset(steps, degree);
            let note = if note > i32::from(CHORD_TOP) {
                note - 12
            } else {
                note
            };
            note as u8
        })
        .collect();
    notes.sort_unstable();
    notes
}

/// The note a bass hit of `pitch` plays under the chord on `degree`.
pub fn bass_note(key: KeySignature, degree: u8, pitch: Pitch) -> u8 {
    let class = chord_root_class(key, degree);
    let root = BASS_LOW + class;
    match pitch {
        Pitch::Root => root,
        Pitch::Fifth => BASS_LOW + (class + 7) % 12,
        Pitch::OctaveUp => root + 12,
        Pitch::OctaveDown if root >= BASS_FLOOR + 12 => root - 12,
        Pitch::OctaveDown => root,
        Pitch::NextDegree => {
            let steps = key.scale.chord_scale().steps();
            let up = degree_offset(steps, degree + 1) - degree_offset(steps, degree);
            (i32::from(root) + up) as u8
        }
    }
}

/// The pitch classes a melody moves through, low to high: the key family's pentatonic on the
/// key's root, without the notes the key's scale lacks (harmonic minor has no flat seventh).
pub fn melody_pool(key: KeySignature) -> Vec<u8> {
    let pentatonic = if key.scale.is_minor_family() {
        Scale::PentatonicMinor
    } else {
        Scale::PentatonicMajor
    };
    let mut pool: Vec<u8> = pentatonic
        .steps()
        .iter()
        .map(|&step| (key.root.get() + step) % 12)
        .filter(|&class| key.contains(class))
        .collect();
    pool.sort_unstable();
    pool
}

/// Where `key` stands on the pool's ladder: every pool note of every octave, counted from 0.
fn ladder_index(pool: &[u8], key: u8) -> i32 {
    let place = pool
        .iter()
        .position(|&class| class == key % 12)
        .unwrap_or(0);
    i32::from(key / 12) * pool.len() as i32 + place as i32
}

fn ladder_note(pool: &[u8], index: i32) -> i32 {
    let len = pool.len() as i32;
    12 * index.div_euclid(len) + i32::from(pool[index.rem_euclid(len) as usize])
}

/// `note` moved by octaves into the melody's band `low..=high`, an octave or more wide (the
/// genre file is held to that), so the moves always land.
fn fold_into_band(mut note: i32, low: u8, high: u8) -> u8 {
    while note > i32::from(high) {
        note -= 12;
    }
    while note < i32::from(low) {
        note += 12;
    }
    note as u8
}

/// The pool note in the band `low..=high` nearest `target`, the lower of two as near.
fn nearest_in_pool(pool: &[u8], target: u8, low: u8, high: u8) -> u8 {
    (low..=high)
        .filter(|key| pool.contains(&(key % 12)))
        .min_by_key(|&key| (key.abs_diff(target), key))
        .unwrap_or(target)
}

/// Draws the hits `parts` play in one bar, in order, and hands each to `play` with its velocity.
fn play_parts(rng: &mut JamRng, parts: &[Part], last_bar: bool, mut play: impl FnMut(&Hit, u8)) {
    for part in parts {
        let listed = match &part.kind {
            PartKind::Hits(hits) => hits,
            PartKind::Either {
                chance,
                then,
                otherwise,
            } => {
                if rng.chance(*chance) {
                    then
                } else {
                    otherwise
                }
            }
            PartKind::LastBar(hits) if last_bar => hits,
            PartKind::LastBar(_) => continue,
        };
        for hit in listed.iter() {
            let plays = match hit.when {
                When::Always => true,
                When::Chance(p) => rng.chance(p),
                When::Unless(p) => !rng.chance(p),
            };
            if !plays {
                continue;
            }
            let velocity = match hit.velocity {
                Velocity::Exact(v) => v,
                // The draw is below the part's jitter, a u8, so it fits one.
                Velocity::Jitter(base) => {
                    base.saturating_sub(rng.jitter(u32::from(part.jitter)) as u8)
                }
            };
            play(
                hit,
                velocity.clamp(*VELOCITY_RANGE.start(), *VELOCITY_RANGE.end()),
            );
        }
    }
}

/// A channel's notes as they are written: a later one on a tick and key replaces the earlier.
#[derive(Default)]
struct Written(BTreeMap<(Ticks, u8), JamNote>);

impl Written {
    fn put(&mut self, tick: Ticks, length: Ticks, key: u8, velocity: u8) {
        let note = JamNote {
            tick,
            length,
            key,
            velocity,
        };
        self.0.insert((tick, key), note);
    }
}

/// The grid at a project's ticks per quarter.
#[derive(Clone, Copy)]
struct Grid {
    step: i64,
}

impl Grid {
    fn bar_start(self, bar: u32) -> i64 {
        i64::from(bar) * i64::from(STEPS_PER_BAR) * self.step
    }

    fn tick(self, bar: u32, hit: &Hit) -> Ticks {
        let half = if hit.half { self.step / 2 } else { 0 };
        Ticks(self.bar_start(bar) + i64::from(hit.step) * self.step + half)
    }

    fn steps(self, count: u8) -> Ticks {
        Ticks(i64::from(count) * self.step)
    }
}

/// Writes the Jam `request` asks for. Every draw comes from one [`JamRng`] seeded with the
/// request's seed, in the order the module documents.
pub fn write(request: &JamRequest) -> JamPattern {
    let table = request.genre.table();
    let mut rng = JamRng::for_jam(request.genre, request.seed);
    let progressions = table.progressions(request.key.scale);
    let progression = progressions[rng.below(progressions.len() as u32) as usize];
    let (low, high) = (*table.tempo_range.start(), *table.tempo_range.end());
    let tempo = MilliBpm((low + rng.below(high - low + 1)) * 1000);
    let mut variants = [0u8; 9];
    for (picked, count) in variants.iter_mut().zip(table.variant_counts()) {
        if count > 1 {
            // The file holds a few variants a part, far fewer than 256.
            *picked = rng.below(count as u32) as u8;
        }
    }
    let variant = |part: usize, of: &'static [Variant]| &of[usize::from(variants[part])].parts;
    let grid = Grid {
        step: i64::from(request.ppq / 4),
    };
    let mut parts = Vec::with_capacity(request.channels.len());
    for &JamChannel {
        channel,
        role,
        range,
        articulation,
    } in &request.channels
    {
        let mut written = Written::default();
        let mut melody = Melody {
            pool: melody_pool(request.key),
            previous: None,
            held: articulation == Articulation::Sustained,
        };
        for bar in 0..request.bars {
            let degree = progression[bar as usize % progression.len()];
            let last_bar = bar + 1 == request.bars;
            match role {
                Role::Drums(map) => {
                    for (index, voice) in Voice::ALL.into_iter().enumerate() {
                        let Some(key) = map.key(voice) else { continue };
                        let parts = variant(index, table.voice(voice));
                        play_parts(&mut rng, parts, last_bar, |hit, v| {
                            written.put(grid.tick(bar, hit), Ticks(0), key, v);
                        });
                    }
                }
                Role::Chords => {
                    let stack = chord(request.key, degree, table.sevenths);
                    play_parts(&mut rng, variant(6, &table.chords), last_bar, |hit, v| {
                        for &key in &stack {
                            written.put(grid.tick(bar, hit), grid.steps(hit.length), key, v);
                        }
                    });
                }
                Role::Bass => {
                    play_parts(&mut rng, variant(7, &table.bass), last_bar, |hit, v| {
                        let key = bass_note(request.key, degree, hit.pitch);
                        written.put(grid.tick(bar, hit), grid.steps(hit.length), key, v);
                    });
                }
                Role::Melody => melody.bar(
                    &mut rng,
                    &table.melody[usize::from(variants[8])],
                    chord_root_class(request.key, degree),
                    grid,
                    bar,
                    &mut written,
                ),
            }
        }
        let drawn: Vec<JamNote> = written.0.into_values().collect();
        // Fitted after every draw, so a range changes no draw: the channels after this one,
        // and this one without a range, are what the seed writes either way.
        let (notes, dropped) = match role {
            Role::Drums(_) => (drawn.clone(), 0),
            _ => fit(&drawn, range),
        };
        parts.push(JamPart {
            channel,
            role,
            drawn,
            notes,
            dropped,
        });
    }
    JamPattern {
        ppq: request.ppq,
        bars: request.bars,
        progression,
        swing_tenths: table.swing_tenths,
        tempo,
        variants,
        melody_longest: table.melody[usize::from(variants[8])].max_length,
        parts,
    }
}

/// The octave offsets a whole part is tried at, in this order.
const OCTAVE_OFFSETS: [i32; 5] = [0, -12, 12, -24, 24];

/// `notes`, one pitched part sorted by (tick, key), fitted to `range` (its lowest and highest
/// key, inclusive), and how many were left out. Two steps: the whole part moves by the octave
/// offset that puts the most notes inside (the first such of 0, -12, +12, -24, +24), so it
/// keeps its shape; then each note still outside moves the fewest whole octaves toward the
/// range, and one that cannot reach it so (a range narrower than an octave without its pitch
/// class) is left out. `None` leaves the part as it is. Why: an instrument plays nothing on a
/// key it has no zone for, and a note it cannot play is better gone than silent.
pub fn fit(notes: &[JamNote], range: Option<(u8, u8)>) -> (Vec<JamNote>, u32) {
    let Some((low, high)) = range else {
        return (notes.to_vec(), 0);
    };
    let (low, high) = (i32::from(low), i32::from(high));
    let inside = |key: i32| (low..=high).contains(&key);
    let count = |offset: i32| {
        notes
            .iter()
            .filter(|note| inside(i32::from(note.key) + offset))
            .count()
    };
    let mut best = OCTAVE_OFFSETS[0];
    for offset in OCTAVE_OFFSETS {
        if count(offset) > count(best) {
            best = offset;
        }
    }
    // Keyed as `Written` is: two notes that land on one tick and key keep the later.
    let mut fitted = BTreeMap::new();
    let mut dropped = 0;
    for note in notes {
        let mut key = i32::from(note.key) + best;
        if key < low {
            key += 12 * ((low - key + 11) / 12);
        } else if key > high {
            key -= 12 * ((key - high + 11) / 12);
        }
        if inside(key) {
            // Inside a range of keys, so 0 to 127.
            let key = key as u8;
            fitted.insert((note.tick, key), JamNote { key, ..*note });
        } else {
            dropped += 1;
        }
    }
    (fitted.into_values().collect(), dropped)
}

/// A melody channel as it is written bar by bar.
struct Melody {
    /// [`melody_pool`] of the key.
    pool: Vec<u8>,
    /// The channel's note before, `None` before its first.
    previous: Option<u8>,
    /// Its notes held to the next start ([`Articulation::Sustained`]).
    held: bool,
}

impl Melody {
    /// One bar of the melody over a chord whose root is `chord_root` (a pitch class).
    fn bar(
        &mut self,
        rng: &mut JamRng,
        table: &MelodyTable,
        chord_root: u8,
        grid: Grid,
        bar: u32,
        written: &mut Written,
    ) {
        let mut starts: Vec<u8> = match table.count {
            Count::Drawn {
                least,
                spread,
                rest_light,
            } => {
                let mut count = u32::from(least) + rng.below(u32::from(spread));
                if let Some(rest) = rest_light {
                    if rng.chance(rest.chance) {
                        count = count.min(u32::from(rest.cap));
                    }
                }
                let count = (count as usize).min(table.starts.len());
                let mut picked = Vec::with_capacity(count);
                while picked.len() < count {
                    let start = table.starts[rng.below(table.starts.len() as u32) as usize];
                    if !picked.contains(&start) {
                        picked.push(start);
                    }
                }
                picked.sort_unstable();
                picked
            }
            Count::Every => table.starts.to_vec(),
        };
        if let Some(late) = table.late {
            for start in &mut starts {
                if rng.chance(late) {
                    *start += 1;
                }
            }
        }
        for (i, &start) in starts.iter().enumerate() {
            let end = starts.get(i + 1).copied().unwrap_or(STEPS_PER_BAR);
            // A late start never reaches the next one in any table; saturating keeps a table
            // that did from panicking.
            let gap = end.saturating_sub(start);
            let drawn = table.max_length.min(gap);
            // Held: to the next start or the bar's end; lengths only, so no draw changes.
            let length = if self.held && drawn >= 2 { gap } else { drawn };
            let (low, high) = (table.low, table.high);
            let note = match self.previous {
                None => {
                    // The chord's root in the band's lowest octave: the key from `low` up with
                    // its pitch class.
                    let root = low + (chord_root + 12 - low % 12) % 12;
                    nearest_in_pool(&self.pool, root, low, high)
                }
                Some(before) => {
                    let moved = ladder_index(&self.pool, before) + rng.below(5) as i32 - 2;
                    fold_into_band(ladder_note(&self.pool, moved), low, high)
                }
            };
            let velocity = MELODY_VELOCITY + rng.jitter(MELODY_VELOCITY_SPREAD) as u8;
            let tick = Ticks(grid.bar_start(bar) + i64::from(start) * grid.step);
            written.put(tick, grid.steps(length), note, velocity);
            self.previous = Some(note);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(root: u8, scale: Scale) -> KeySignature {
        KeySignature {
            root: PitchClass::new(root).expect("a pitch class"),
            scale,
        }
    }

    // The generator's first ten values for seed 1 in lo-fi and in boom bap, and splitmix64 of
    // 0, 1 and the golden constant, as gen_p36_expected.py works them out (Python's big integers;
    // splitmix64(0) is SplitMix64's published first output, 0xE220A8397B1DCDAF).
    #[test]
    fn the_rng_gives_the_oracles_first_ten_values() {
        let first_ten = |genre| {
            let mut rng = JamRng::for_jam(genre, 1);
            (0..10).map(|_| rng.next_u32()).collect::<Vec<_>>()
        };
        assert_eq!(
            first_ten(Genre::LoFi),
            [
                2218154741, 3429946819, 3628769943, 1714714897, 4065165237, 2745818865, 531721687,
                257011544, 3852032980, 586721125
            ]
        );
        assert_eq!(
            first_ten(Genre::BoomBap),
            [
                1324867962, 3064551515, 75187461, 3069065949, 2991280273, 1704438455, 1323046890,
                577075559, 4275900461, 2040719538
            ]
        );
        assert_eq!(
            [splitmix64(0), splitmix64(1), splitmix64(GOLDEN)],
            [
                16294208416658607535,
                10451216379200822465,
                7960286522194355700
            ]
        );
        assert_eq!(splitmix64(0), 0xE220_A839_7B1D_CDAF);
    }

    // Chords by hand: A minor's i7 is A3 C4 E4 G4; C major's V7 G3 B3 D4 F4; E harmonic minor's
    // V7 B3 D#4 F#4 A4 (the raised seventh); Bb pentatonic major's IV comes from Bb major, Eb G
    // Bb; A minor's VII triad G3 B3 D4 wraps the degrees past 7.
    #[test]
    fn chords_stack_thirds_of_the_chord_scale() {
        assert_eq!(chord(key(9, Scale::Minor), 1, true), [57, 60, 64, 67]);
        assert_eq!(chord(key(0, Scale::Major), 5, true), [55, 59, 62, 65]);
        assert_eq!(
            chord(key(4, Scale::HarmonicMinor), 5, true),
            [59, 63, 66, 69]
        );
        assert_eq!(
            chord(key(10, Scale::PentatonicMajor), 4, false),
            [51, 55, 58]
        );
        assert_eq!(chord(key(9, Scale::Minor), 7, false), [55, 59, 62]);
    }

    // Every root, scale and degree: the chord's root in 48..=59, no note above 71, every note in
    // the chord scale; the bass root in 36..=47 and its fifth too; the melody pool inside the key.
    #[test]
    fn voicings_stay_in_their_bands() {
        for root in 0..12 {
            for scale in Scale::ALL {
                let k = key(root, scale);
                let chords_in = KeySignature {
                    scale: scale.chord_scale(),
                    ..k
                };
                for degree in 1..=7 {
                    for sevenths in [false, true] {
                        let notes = chord(k, degree, sevenths);
                        assert!((48..=59).contains(&notes[0]), "{k} {degree}: {notes:?}");
                        assert!(notes.iter().all(|&n| n <= 71 && chords_in.contains(n)));
                        assert_eq!(notes.len(), if sevenths { 4 } else { 3 });
                    }
                    let root_note = bass_note(k, degree, Pitch::Root);
                    assert!((36..=47).contains(&root_note));
                    assert_eq!(root_note % 12, chord(k, degree, false)[0] % 12);
                    assert!((36..=47).contains(&bass_note(k, degree, Pitch::Fifth)));
                    assert_eq!(bass_note(k, degree, Pitch::OctaveDown) + 12, root_note);
                }
                let pool = melody_pool(k);
                assert!(pool.len() >= 4 && pool.iter().all(|&class| k.contains(class)));
            }
        }
    }

    // Every genre in every scale, seeds 0 to 20, 4 and 8 bars: the pins gen_p36_expected.py
    // holds its own cases to, over every table.
    #[test]
    fn every_genre_keeps_the_pins() {
        let channels = [
            (ChannelId(1), Role::Drums(DrumMap::GM)),
            (ChannelId(2), Role::Bass),
            (ChannelId(3), Role::Chords),
            (ChannelId(4), Role::Melody),
        ];
        for genre in Genre::ALL {
            for scale in Scale::ALL {
                for (seed, bars) in (0..=20).zip([4, 8].into_iter().cycle()) {
                    let k = key((seed % 12) as u8, scale);
                    let request = JamRequest {
                        genre,
                        key: k,
                        bars,
                        seed,
                        ppq: 96,
                        channels: channels.iter().copied().map(JamChannel::from).collect(),
                    };
                    let jam = write(&request);
                    let end = Ticks(i64::from(bars) * 384);
                    let chord_key = KeySignature {
                        scale: scale.chord_scale(),
                        ..k
                    };
                    for part in &jam.parts {
                        assert!(!part.notes.is_empty(), "{genre:?} {k} {seed}");
                        for note in &part.notes {
                            assert!(note.tick < end && Ticks(note.tick.0 + note.length.0) <= end);
                            assert!(VELOCITY_RANGE.contains(&note.velocity));
                            match part.role {
                                Role::Melody => {
                                    let picked = usize::from(jam.variants[8]);
                                    let melody = &genre.table().melody[picked];
                                    assert!(k.contains(note.key));
                                    assert!((melody.low..=melody.high).contains(&note.key));
                                    assert!(note.length.0 <= i64::from(melody.max_length) * 24);
                                }
                                Role::Chords => assert!(chord_key.contains(note.key)),
                                Role::Bass => {
                                    let bar = (note.tick.0 / 384) as usize;
                                    let degree = jam.progression[bar % 4];
                                    let root = bass_note(k, degree, Pitch::Root) % 12;
                                    let passing = bass_note(k, degree, Pitch::NextDegree);
                                    assert!(
                                        note.key % 12 == root
                                            || note.key % 12 == (root + 7) % 12
                                            || note.key == passing,
                                        "{genre:?} {k} {seed}: {note:?}"
                                    );
                                }
                                Role::Drums(_) => assert_eq!(note.length, Ticks(0)),
                            }
                        }
                    }
                }
            }
        }
    }

    /// A part of `notes` given as (tick, length, key, velocity).
    fn part(channel: u32, role: Role, notes: &[(i64, i64, u8, u8)]) -> JamPart {
        let notes: Vec<JamNote> = notes
            .iter()
            .map(|&(tick, length, key, velocity)| JamNote {
                tick: Ticks(tick),
                length: Ticks(length),
                key,
                velocity,
            })
            .collect();
        JamPart {
            channel: ChannelId(channel),
            role,
            drawn: notes.clone(),
            dropped: 0,
            notes,
        }
    }

    fn pattern(bars: u32, parts: Vec<JamPart>) -> JamPattern {
        JamPattern {
            ppq: 96,
            bars,
            progression: [1, 6, 3, 7],
            swing_tenths: 580,
            tempo: MilliBpm(78_000),
            variants: [0; 9],
            melody_longest: 12,
            parts,
        }
    }

    // Keeping by hand, at PPQ 96 (a bar 384 ticks): a 4-bar roll's drums (36 at 0, 38 at 192)
    // kept in an 8-bar one play twice, the second time 1536 ticks on (4 bars), with their own
    // velocities; the bass, not kept, stays the new roll's; the melody, kept but absent from the
    // old roll, stays the new roll's. An 8-bar roll's note at 1600 kept in a 4-bar one (ending at
    // 1536) is cut, the one at 300 stays. A role kept is the old roll's too.
    #[test]
    fn keeping_puts_back_the_kept_channels_notes() {
        let drums = Role::Drums(DrumMap::GM);
        let before = pattern(
            4,
            vec![
                part(1, drums, &[(0, 0, 36, 90), (192, 0, 38, 85)]),
                part(2, Role::Bass, &[(0, 144, 45, 80)]),
            ],
        );
        let rolled = pattern(
            8,
            vec![
                part(1, drums, &[(0, 0, 36, 70)]),
                part(2, Role::Bass, &[(96, 48, 40, 60)]),
                part(3, Role::Melody, &[(0, 96, 76, 55)]),
            ],
        );
        let kept = rolled
            .clone()
            .keeping(&before, &[ChannelId(1), ChannelId(3)]);
        assert_eq!(
            kept.parts[0],
            part(
                1,
                drums,
                &[
                    (0, 0, 36, 90),
                    (192, 0, 38, 85),
                    (1536, 0, 36, 90),
                    (1728, 0, 38, 85)
                ]
            )
        );
        assert_eq!(kept.parts[1..], rolled.parts[1..]);
        assert_eq!(rolled.clone().keeping(&before, &[]), rolled);

        let long = pattern(
            8,
            vec![part(
                4,
                Role::Chords,
                &[(300, 96, 60, 60), (1600, 96, 62, 60)],
            )],
        );
        let short = pattern(4, vec![part(4, Role::Melody, &[(0, 96, 80, 55)])]);
        let kept = short.keeping(&long, &[ChannelId(4)]);
        assert_eq!(kept.parts, [part(4, Role::Chords, &[(300, 96, 60, 60)])]);
    }

    // Moving a part by hand, at PPQ 96 (a step 24 ticks): General MIDI drums (kick 36 at 0,
    // snare 38 at 96, clap 39 at 96, closed hat 42 at 48, open hat 46 at 336, cross-stick 37 at
    // 144) moved to channel 9 on the kit's electronic map (kick 28, snare 38, closed hat 56,
    // open hat 46, no clap or perc) keep their ticks and velocities on 28, 38, 56 and 46, the
    // clap and the cross-stick dropped (2). A house melody (its longest note 4 steps) of three
    // notes at steps 0, 8 and 15, plucked (4, 4 and 1 step: each its room up to 4), moved to a
    // held channel lasts 8, 7 and 1 steps (its room, the last drawn under 2 steps as drawn), and
    // moved back to a plucked one 4, 4 and 1 again; moved to a channel of 60..=71 its keys 74,
    // 76, 79 go down an octave to 62, 64, 67. A part the pattern lacks moves nothing.
    #[test]
    fn moving_a_part_maps_its_drums_and_fits_its_lengths_and_octaves() {
        let gm = Role::Drums(DrumMap::GM);
        let electronic = DrumMap {
            kick: Some(28),
            snare: Some(38),
            clap: None,
            closed_hat: Some(56),
            open_hat: Some(46),
            perc: None,
        };
        let drums = pattern(
            4,
            vec![part(
                1,
                gm,
                &[
                    (0, 0, 36, 90),
                    (48, 0, 42, 60),
                    (96, 0, 38, 85),
                    (96, 0, 39, 75),
                    (144, 0, 37, 55),
                    (336, 0, 46, 50),
                ],
            )],
        );
        let to = JamChannel {
            channel: ChannelId(9),
            role: Role::Drums(electronic),
            range: None,
            articulation: Articulation::Plucked,
        };
        let moved = drums.clone().moved(ChannelId(1), &to);
        let mut want = part(
            9,
            Role::Drums(electronic),
            &[
                (0, 0, 28, 90),
                (48, 0, 56, 60),
                (96, 0, 38, 85),
                (336, 0, 46, 50),
            ],
        );
        want.dropped = 2;
        assert_eq!(moved.parts, [want]);
        assert_eq!(
            drums.clone().moved(ChannelId(5), &to),
            drums,
            "no such part"
        );

        let mut melody = pattern(
            1,
            vec![part(
                4,
                Role::Melody,
                &[(0, 96, 74, 55), (192, 96, 76, 60), (360, 24, 79, 65)],
            )],
        );
        melody.melody_longest = 4;
        let held = JamChannel {
            channel: ChannelId(5),
            role: Role::Melody,
            range: None,
            articulation: Articulation::Sustained,
        };
        let lengths = |pattern: &JamPattern| -> Vec<i64> {
            pattern.parts[0]
                .notes
                .iter()
                .map(|note| note.length.0)
                .collect()
        };
        let sustained = melody.clone().moved(ChannelId(4), &held);
        assert_eq!(sustained.parts[0].channel, ChannelId(5));
        assert_eq!(lengths(&sustained), [192, 168, 24]);
        let plucked = JamChannel {
            channel: ChannelId(6),
            articulation: Articulation::Plucked,
            range: Some((60, 71)),
            ..held
        };
        let back = sustained.moved(ChannelId(5), &plucked);
        assert_eq!(lengths(&back), [96, 96, 24]);
        let keys: Vec<u8> = back.parts[0].notes.iter().map(|note| note.key).collect();
        assert_eq!(keys, [62, 64, 67]);
        assert_eq!(
            back.parts[0].drawn, melody.parts[0].drawn,
            "drawn as before the fit"
        );
    }

    // A part moved is the part the same seed writes for the channel it moved to: every genre,
    // scale and seed 0 to 11, the bass moved from a channel of 26..=45 to one of any key, the
    // chords from any key to 55..=70, and the melody from a plucked channel of 72..=96 to a held
    // one of 60..=72, and back. Why it holds: a range and an articulation change no draw
    // (`write` fits after its draws and lengthens without drawing), so the moved part's draws
    // are the written one's.
    #[test]
    fn a_moved_part_is_what_the_seed_writes_for_its_new_channel() {
        let channel = |id: u32, role, range, articulation| JamChannel {
            channel: ChannelId(id),
            role,
            range,
            articulation,
        };
        let plucked = Articulation::Plucked;
        let held = Articulation::Sustained;
        let first = [
            channel(1, Role::Drums(DrumMap::GM), None, plucked),
            channel(2, Role::Bass, Some((26, 45)), plucked),
            channel(3, Role::Chords, None, plucked),
            channel(4, Role::Melody, Some((72, 96)), plucked),
        ];
        let second = [
            first[0],
            channel(2, Role::Bass, None, plucked),
            channel(3, Role::Chords, Some((55, 70)), plucked),
            channel(4, Role::Melody, Some((60, 72)), held),
        ];
        for genre in Genre::ALL {
            for scale in Scale::ALL {
                for seed in 0..12 {
                    let request = |channels: &[JamChannel; 4]| JamRequest {
                        genre,
                        key: key((seed % 12) as u8, scale),
                        bars: 4,
                        seed,
                        ppq: 96,
                        channels: channels.to_vec(),
                    };
                    let (one, two) = (write(&request(&first)), write(&request(&second)));
                    let mut moved = one.clone();
                    for to in &second[1..] {
                        moved = moved.moved(to.channel, to);
                    }
                    assert_eq!(moved, two, "{genre:?} {scale:?} {seed}");
                    let mut back = two;
                    for to in &first[1..] {
                        back = back.moved(to.channel, to);
                    }
                    assert_eq!(back, one, "{genre:?} {scale:?} {seed}, back");
                }
            }
        }
    }

    // The progression a seed picks is the one `write` picks, in every genre and scale.
    #[test]
    fn the_progression_is_the_one_write_picks() {
        for genre in Genre::ALL {
            for scale in Scale::ALL {
                for seed in 0..40 {
                    let request = JamRequest {
                        genre,
                        key: key(9, scale),
                        bars: 4,
                        seed,
                        ppq: 96,
                        channels: Vec::new(),
                    };
                    assert_eq!(
                        progression(genre, scale, seed),
                        write(&request).progression,
                        "{genre:?} {scale:?} {seed}"
                    );
                }
            }
        }
    }

    // A map with one key twice (gen_p36_expected.py's house_1_shared_key: kick and snare on 36,
    // closed hat 42): the later voice's hit on a tick replaces the earlier's, so the channel
    // never holds two notes on one tick and key. The oracle's velocities there (seed 1 picks
    // the snare's second variant, base 0.80, over the kick's four on the floor, base 0.95): 0.75
    // on step 4 of bar 1 and 0.67 on step 12, the snare's both.
    #[test]
    fn one_key_twice_keeps_the_later_note() {
        let map = DrumMap::default()
            .with(Voice::Kick, Some(36))
            .with(Voice::Snare, Some(36))
            .with(Voice::ClosedHat, Some(42));
        let request = JamRequest {
            genre: Genre::House,
            key: key(9, Scale::Minor),
            bars: 4,
            seed: 1,
            ppq: 96,
            channels: vec![(ChannelId(1), Role::Drums(map)).into()],
        };
        let jam = write(&request);
        let notes = &jam.parts[0].notes;
        let mut keys: Vec<(Ticks, u8)> = notes.iter().map(|n| (n.tick, n.key)).collect();
        keys.dedup();
        assert_eq!(keys.len(), notes.len());
        let velocity_at = |tick| {
            notes
                .iter()
                .find(|n| n.tick == Ticks(tick) && n.key == 36)
                .map(|n| n.velocity)
        };
        assert_eq!(velocity_at(96), Some(75));
        assert_eq!(velocity_at(288), Some(67));
        assert_eq!(notes.len(), 35);
    }

    /// Notes given as (tick, key), each a step long at velocity 0.60.
    fn notes(at: &[(i64, u8)]) -> Vec<JamNote> {
        at.iter()
            .map(|&(tick, key)| JamNote {
                tick: Ticks(tick),
                length: Ticks(24),
                key,
                velocity: 60,
            })
            .collect()
    }

    fn keys(notes: &[JamNote]) -> Vec<(i64, u8)> {
        notes.iter().map(|note| (note.tick.0, note.key)).collect()
    }

    // The fit's steps by hand. The whole part: 80 and 84 above 60..=76 go down an octave
    // together (offset 0 has none inside, -12 both). Per note: a part mostly inside stays at
    // offset 0 and its one outsider moves alone, the fewest octaves (90 into 48..=60: three down,
    // 54; 30 into 60..=76: three up, 66). The drop: 38 into 40..=46 reaches 50 an octave up,
    // past 46, so no octave of D lies in the range. Offsets tie: the first in the order wins
    // (60 fits 48..=72 at 0 and at +12; 0 stays). None leaves the part as it is.
    #[test]
    fn fit_moves_the_part_then_each_note_and_drops_what_cannot_fit() {
        assert_eq!(
            fit(&notes(&[(0, 80), (24, 84)]), Some((60, 76))),
            (notes(&[(0, 68), (24, 72)]), 0)
        );
        assert_eq!(
            fit(&notes(&[(0, 50), (24, 90)]), Some((48, 60))),
            (notes(&[(0, 50), (24, 54)]), 0)
        );
        assert_eq!(
            fit(&notes(&[(0, 30)]), Some((60, 76))),
            (notes(&[(0, 66)]), 0)
        );
        let (kept, dropped) = fit(&notes(&[(0, 38), (24, 41)]), Some((40, 46)));
        assert_eq!((keys(&kept), dropped), (vec![(24, 41)], 1));
        assert_eq!(
            fit(&notes(&[(0, 60)]), Some((48, 72))),
            (notes(&[(0, 60)]), 0)
        );
        let part = notes(&[(0, 20), (24, 100), (48, 64)]);
        assert_eq!(fit(&part, None), (part.clone(), 0));
        // Moved notes keep (tick, key) order: 72 and 50 on one tick, fitted to 48..=60, are
        // 60 and 50, written low to high.
        let (moved, _) = fit(&notes(&[(0, 50), (0, 72)]), Some((48, 60)));
        assert_eq!(keys(&moved), [(0, 50), (0, 60)]);
    }

    // Through `write`: a range on the drums moves nothing; a range on the bass fits only the
    // bass, every note inside it, and leaves every other channel as the seed writes it without
    // ranges (fitting draws nothing).
    #[test]
    fn write_fits_pitched_parts_and_leaves_drums_and_draws_alone() {
        let band = |ranges: [Option<(u8, u8)>; 4]| {
            let roles = [
                Role::Drums(DrumMap::GM),
                Role::Bass,
                Role::Chords,
                Role::Melody,
            ];
            write(&JamRequest {
                genre: Genre::LoFi,
                key: key(9, Scale::Minor),
                bars: 4,
                seed: 8,
                ppq: 96,
                channels: (1..=4)
                    .zip(roles)
                    .zip(ranges)
                    .map(|((id, role), range)| JamChannel {
                        channel: ChannelId(id),
                        role,
                        range,
                        articulation: Articulation::Plucked,
                    })
                    .collect(),
            })
        };
        let plain = band([None; 4]);
        let drums_ranged = band([Some((60, 61)), None, None, None]);
        assert_eq!(drums_ranged, plain, "drums never move");
        let bass_ranged = band([None, Some((26, 45)), None, None]);
        for i in [0, 2, 3] {
            assert_eq!(bass_ranged.parts[i], plain.parts[i]);
        }
        let bass = &bass_ranged.parts[1];
        assert!(bass.notes.iter().all(|note| (26..=45).contains(&note.key)));
        assert_eq!(
            bass.notes.len() as u32 + bass.dropped,
            plain.parts[1].notes.len() as u32
        );
    }

    // A sustained melody against the same seed plucked, every genre, seeds 1 to 30: the same
    // notes on the same ticks, keys and velocities; each now lasting to the next start in its bar
    // (or the bar's end) when drawn 2 steps or longer, as drawn when shorter; every other channel
    // the same; and some note longer somewhere (trap's 8-step cap holds fewer to the next).
    #[test]
    fn a_sustained_melody_holds_to_the_next_start_and_nothing_else_moves() {
        let mut lengthened = 0;
        for genre in Genre::ALL {
            for seed in 1..=30 {
                let band = |articulation| {
                    let melody = JamChannel {
                        articulation,
                        ..(ChannelId(4), Role::Melody).into()
                    };
                    write(&JamRequest {
                        genre,
                        key: key(9, Scale::Minor),
                        bars: 4,
                        seed,
                        ppq: 96,
                        channels: vec![
                            (ChannelId(1), Role::Drums(DrumMap::GM)).into(),
                            (ChannelId(2), Role::Bass).into(),
                            melody,
                        ],
                    })
                };
                let (plucked, held) = (band(Articulation::Plucked), band(Articulation::Sustained));
                assert_eq!(plucked.parts[..2], held.parts[..2]);
                let (was, now) = (&plucked.parts[2].notes, &held.parts[2].notes);
                assert_eq!(was.len(), now.len());
                for (i, (drawn, note)) in was.iter().zip(now).enumerate() {
                    assert_eq!(
                        (drawn.tick, drawn.key, drawn.velocity),
                        (note.tick, note.key, note.velocity)
                    );
                    let bar_end = (note.tick.0 / 384 + 1) * 384;
                    let next = now[i + 1..]
                        .iter()
                        .map(|later| later.tick.0)
                        .find(|&tick| tick < bar_end)
                        .unwrap_or(bar_end);
                    let want = if drawn.length.0 >= 48 {
                        next - note.tick.0
                    } else {
                        drawn.length.0
                    };
                    assert_eq!(note.length.0, want, "{genre:?} {seed}: {note:?}");
                    lengthened += usize::from(note.length > drawn.length);
                }
            }
        }
        assert!(lengthened > 0);
    }

    #[test]
    fn genres_and_voices_read_their_words() {
        for genre in Genre::ALL {
            assert_eq!(genre.id().parse::<Genre>(), Ok(genre));
        }
        assert_eq!("EDM".parse::<Genre>(), Ok(Genre::House));
        assert_eq!(Genre::House.name(), "EDM / house");
        assert!("polka".parse::<Genre>().is_err());
        for voice in Voice::ALL {
            assert_eq!(Voice::from_id(voice.id()), Some(voice));
        }
        assert_eq!(DrumMap::single(Voice::Snare).snare, Some(DEFAULT_KEY));
        assert_eq!(DrumMap::single(Voice::Snare).kick, None);
    }
}
