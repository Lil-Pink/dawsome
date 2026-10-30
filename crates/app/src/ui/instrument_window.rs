//! The instrument window: a window of its own beside the main one, showing one channel's sound
//! and its sound settings, laid out like a mixer. It stays open while the main window is used;
//! Esc, its Close or its X closes it, and a line at its foot names the control under the
//! cursor. Top to bottom: the channel's name and kind; the strip of controls in groups (SOUND,
//! PITCH, ENVELOPE, FILTER, LEVEL: knobs, lights and faders, placed by
//! [`crate::ui::instrument_layout`], with the output insert's chooser); the sound map across the
//! window (a pad per drum, or a keyboard with the zones' range shaded and their root keys
//! dotted, where the height of a press sets its velocity); a thin footer with Store A, A/B, the
//! roots, the zones' list, Edit sample… (a sampler's or an audio channel's file) and Close. A plugin channel shows its level and opens its own window.
//! An input channel shows its level, then its input: the device and its channels (two pick
//! lists, the devices listed again by Refresh), the input's meter and how far behind it is heard
//! (or why its device did not open).

use dawsome_engine::project::{
    Channel, ChannelKind, Gate, InputChannels, InsertId, MidiOut, Sound, ATTACK_RANGE, BEND_RANGE,
    CENT_RANGE, LATENCY_RANGE, MIDI_OUT_CHANNELS, PAN_RANGE, PROGRAM_RANGE, RELEASE_RANGE,
    RESONANCE_RANGE, SEMITONE_RANGE, START_RANGE, VELOCITY_RANGE,
};
use std::cmp::Reverse;
use std::time::{Duration, Instant};

use iced::keyboard;
use iced::widget::canvas::{self, Event, Frame, Geometry, Path};
use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{
    button, canvas as canvas_widget, column, container, pick_list, pin, row, scrollable, slider,
    space, stack, text,
};
use iced::{mouse, Center, Element, Fill, Point, Rectangle, Renderer, Size, Theme};

use crate::app::Message;
use crate::sets::ZoneInfo;
use crate::ui::hint::{hover_area, hover_change, Hover, SoundPart};
use crate::ui::instrument_layout::{
    cap_rect, dragged, readout_at, Look, Part, Placed, Strip, HEADER_HEIGHT, KNOB_SIZE,
    LIGHT_RADIUS, STRIP_HEIGHT,
};
use crate::ui::key_names::key_name;
use crate::ui::mixer::{centered, draw_tip, label};
use crate::ui::mixer_layout::CAP_LINE;
use crate::ui::mixer_state::{fader_text, pan_text, percent_text};
use crate::ui::rack::draw_knob;
use crate::ui::style;

/// The instrument window's first size, and the least it can be made.
pub const WINDOW_SIZE: Size = Size::new(560.0, 420.0);
pub const WINDOW_MIN: Size = Size::new(420.0, 300.0);

/// The window's padding.
pub const PADDING: f32 = 12.0;
/// The width the sound map's keys and pads are laid out on (drawn stretched across the
/// window), and its height.
pub const MAP_WIDTH: f32 = 416.0;
pub const MAP_HEIGHT: f32 = 110.0;
/// A pad, the space between two, and how many share a row.
pub const PAD: Size = Size::new(52.0, 38.0);
pub const PAD_GAP: f32 = 6.0;
const PADS_PER_ROW: usize = 7;
/// A white key's width; black keys are narrower and shorter.
pub const KEY_WIDTH: f32 = 14.0;
const BLACK_WIDTH: f32 = 9.0;
const BLACK_DEPTH: f32 = 0.6;
/// The lowest cutoff the filter's slider reaches, and how many decades it sweeps.
const CUTOFF_FLOOR: f32 = 20.0;
const CUTOFF_DECADES: f32 = 3.0;

/// What the window reports.
#[derive(Debug, Clone, PartialEq)]
pub enum InstrumentMessage {
    Close,
    Gate(Gate),
    /// A slider moved to this value (in the control's own units).
    Set(Control, f32),
    /// A slider was let go of: the drag is one undo step.
    Released,
    /// A double-click on a control's row: its default back, one undo step.
    Reset(Control),
    Toggle(Flag),
    /// A press on the sound map: this key at this velocity (1..=127).
    Pressed {
        key: u8,
        velocity: u8,
    },
    /// The press on the sound map was let go of.
    Lifted,
    /// The output chooser: the channel plays into this insert.
    Route(InsertId),
    /// The output chooser's "Mixer insert…": an insert of the channel's own, and the mixer
    /// there, as the channel menu's row.
    MixerInsert,
    /// Store A: keep the settings as they are, for Compare.
    Store,
    /// Compare: switch between the settings stored and the ones now.
    Compare,
    /// The zones' list, folded or not.
    More,
    /// A plugin channel's own window.
    OpenPlugin,
    /// Edit sample…: the sample editor on the channel's sound.
    EditSample,
    /// An input channel's device chosen, by name (empty: the system's default input).
    InputDevice(String),
    /// An input channel's channels chosen.
    InputChannels(InputChannels),
    /// The input devices listed again.
    RefreshInputs,
    /// An input channel's MIDI out changed: one undo step (a latency drag: one on release).
    MidiOut(MidiOutChange),
}

/// A change to an input channel's MIDI out from its window's MIDI OUT group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MidiOutChange {
    /// The port by its name as listed, or none: no MIDI out.
    Port(Option<String>),
    /// The MIDI channel, 1 to 16.
    Channel(u8),
    /// The program, 0 to 127 (shown 1 to 128), or none.
    Program(Option<u8>),
    /// The latency in milliseconds, as the slider moves.
    Latency(i32),
    Echo(bool),
}

/// A slider of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Control {
    Volume,
    Pan,
    Semitones,
    Cents,
    Velocity,
    Attack,
    Release,
    Start,
    Cutoff,
    Resonance,
    /// How far a device's pitch bend reaches on the channel's live notes, in semitones.
    BendRange,
}

impl Control {
    /// Every control, as the tests walk them (the window places them by its layout).
    #[cfg(test)]
    pub const ALL: [Control; 11] = [
        Control::Volume,
        Control::Pan,
        Control::Semitones,
        Control::Cents,
        Control::Velocity,
        Control::Attack,
        Control::Release,
        Control::Start,
        Control::Cutoff,
        Control::Resonance,
        Control::BendRange,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Control::Volume => "Volume",
            Control::Pan => "Pan",
            Control::Semitones => "Tune",
            Control::Cents => "Fine tune",
            Control::Velocity => "Velocity \u{2192} volume",
            Control::Attack => "Attack",
            Control::Release => "Release",
            Control::Start => "Start",
            Control::Cutoff => "Cutoff",
            Control::Resonance => "Resonance",
            Control::BendRange => "Bend range",
        }
    }

    /// The value it shows for `channel`.
    pub fn value(self, channel: &Channel) -> f32 {
        let sound = &channel.sound;
        match self {
            Control::Volume => channel.volume,
            Control::Pan => channel.pan,
            Control::Semitones => f32::from(sound.semitones),
            Control::Cents => f32::from(sound.cents),
            Control::Velocity => sound.velocity,
            Control::Attack => sound.attack,
            Control::Release => sound.release,
            Control::Start => sound.start,
            Control::Cutoff => sound.cutoff,
            Control::Resonance => sound.resonance,
            Control::BendRange => f32::from(sound.bend_range),
        }
    }

    /// Its short label on the window's strip; the hint line names it whole ([`label`]).
    ///
    /// [`label`]: Control::label
    pub fn short(self) -> &'static str {
        match self {
            Control::Volume => "Volume",
            Control::Pan => "Pan",
            Control::Semitones => "Tune",
            Control::Cents => "Fine",
            Control::Velocity => "Velocity",
            Control::Attack => "Attack",
            Control::Release => "Release",
            Control::Start => "Start",
            Control::Cutoff => "Cutoff",
            Control::Resonance => "Reso",
            Control::BendRange => "Bend",
        }
    }

    /// Where `value` sits along the control's sweep (a knob's turn, a fader's travel), 0..1:
    /// the volume's fader by the cube law over its range (unity at 63 % of the travel), the
    /// attack's and the release's by the square law (the first fifth of the travel holds the
    /// shortest 4 %), the cutoff over three decades, the rest straight across their ranges.
    pub fn position(self, value: f32) -> f32 {
        let (low, high) = self.ends();
        let along = ((value - low) / (high - low)).clamp(0.0, 1.0);
        match self {
            Control::Volume => along.cbrt(),
            Control::Attack | Control::Release => along.sqrt(),
            Control::Cutoff => cutoff_position(value),
            _ => along,
        }
    }

    /// The value at `position` (0..1) along the control's sweep, as the setting takes it (in
    /// range, whole for the tune's semitones and cents): [`position`](Control::position)'s
    /// inverse.
    pub fn value_at(self, position: f32) -> f32 {
        let at = position.clamp(0.0, 1.0);
        let (low, high) = self.ends();
        let across = |along: f32| low + along * (high - low);
        match self {
            Control::Volume => across(at * at * at),
            Control::Attack | Control::Release => across(at * at),
            Control::Cutoff => cutoff_of(at),
            Control::Semitones | Control::Cents | Control::BendRange => across(at).round(),
            _ => across(at),
        }
    }

    /// The ends of its range, in its own units.
    fn ends(self) -> (f32, f32) {
        let whole = |range: std::ops::RangeInclusive<i8>| {
            (f32::from(*range.start()), f32::from(*range.end()))
        };
        let ends = |range: std::ops::RangeInclusive<f32>| (*range.start(), *range.end());
        match self {
            Control::Volume => (0.0, dawsome_engine::project::MAX_VOLUME),
            Control::Pan => ends(PAN_RANGE),
            Control::Semitones => whole(SEMITONE_RANGE),
            Control::Cents => whole(CENT_RANGE),
            Control::Velocity => ends(VELOCITY_RANGE),
            Control::Attack => ends(ATTACK_RANGE),
            Control::Release => ends(RELEASE_RANGE),
            Control::Start => ends(START_RANGE),
            Control::Cutoff => ends(dawsome_engine::project::CUTOFF_RANGE),
            Control::Resonance => ends(RESONANCE_RANGE),
            Control::BendRange => (f32::from(*BEND_RANGE.start()), f32::from(*BEND_RANGE.end())),
        }
    }

    /// Its value as the window writes it.
    pub fn text(self, value: f32) -> String {
        let seconds = |value: f32| {
            if value < 1.0 {
                format!("{:.0} ms", value * 1000.0)
            } else {
                format!("{value:.2} s")
            }
        };
        match self {
            // In decibels, as the mixer's faders say it.
            Control::Volume => fader_text(value),
            Control::Velocity | Control::Start | Control::Resonance => percent_text(value),
            Control::Pan => pan_text(value),
            Control::Semitones => format!("{value:+.0} st"),
            Control::Cents => format!("{value:+.0} ct"),
            Control::Attack | Control::Release => seconds(value),
            Control::Cutoff if value >= 1000.0 => format!("{:.1} kHz", value / 1000.0),
            Control::Cutoff => format!("{value:.0} Hz"),
            Control::BendRange => format!("\u{b1}{value:.0} st"),
        }
    }

    /// Whether a channel of `kind` shows it: a plugin, audio or input channel its volume and
    /// pan; an instrument set every control but the start.
    pub fn shown_for(self, kind: ChannelKind) -> bool {
        match kind {
            ChannelKind::Plugin | ChannelKind::Audio | ChannelKind::Input => {
                matches!(self, Control::Volume | Control::Pan)
            }
            ChannelKind::Instrument => self != Control::Start,
            ChannelKind::Sampler => true,
        }
    }
}

/// The cutoff a position `at` (0..1) of its slider gives: 20 Hz × 1000^at.
pub fn cutoff_of(at: f32) -> f32 {
    (CUTOFF_FLOOR * 10f32.powf(CUTOFF_DECADES * at)).clamp(CUTOFF_FLOOR, 20_000.0)
}

/// The slider position of `cutoff`.
pub fn cutoff_position(cutoff: f32) -> f32 {
    ((cutoff / CUTOFF_FLOOR).log10() / CUTOFF_DECADES).clamp(0.0, 1.0)
}

/// A switch of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flag {
    Reverse,
    Filter,
}

/// One pad of a drum map: the key it plays and its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pad {
    pub key: u8,
    pub name: String,
}

/// What the sound map shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Map {
    /// One pad per drum.
    Pads(Vec<Pad>),
    /// A keyboard from `low` (a C) over as many white keys as fit, the zones' keys shaded and
    /// their roots dotted.
    Keys {
        low: u8,
        zones: Vec<(u8, u8)>,
        roots: Vec<u8>,
    },
}

/// `pads` (in key order) with the first kick, snare and hat moved to the front, in that order,
/// as a beat maker looks for them; the rest keep their order. A pad's kind is read from its
/// sound's name: "kick" or a name starting "bd", "snare" or starting "sn_", "hat" or "hh".
fn kick_snare_hat_first(mut pads: Vec<Pad>) -> Vec<Pad> {
    let kinds: [fn(&str) -> bool; 3] = [
        |name| name.contains("kick") || name.starts_with("bd"),
        |name| name.contains("snare") || name.starts_with("sn_"),
        |name| name.contains("hat") || name.contains("hh"),
    ];
    let mut front = Vec::new();
    for kind in kinds {
        if let Some(at) = pads.iter().position(|pad| kind(&pad.name.to_lowercase())) {
            front.push(pads.remove(at));
        }
    }
    front.extend(pads);
    front
}

/// The map for a channel of `kind` called `name` playing `zones` (a set's; none for a
/// sampler): pads when every zone is one key (a drum set) or the sampler plays one of the
/// starter kit's drums (`drum`), else a keyboard around the zones (a sampler's: around its root,
/// key 60).
pub fn map_of(kind: ChannelKind, name: &str, zones: &[ZoneInfo], drum: bool) -> Map {
    if kind == ChannelKind::Sampler {
        if drum {
            return Map::Pads(vec![Pad {
                key: 60,
                name: name.to_owned(),
            }]);
        }
        return keyboard(&[(0, 127)], &[60]);
    }
    let one_key = !zones.is_empty() && zones.iter().all(|zone| zone.keys.0 == zone.keys.1);
    if one_key {
        let mut pads: Vec<Pad> = Vec::new();
        for zone in zones {
            if !pads.iter().any(|pad| pad.key == zone.keys.0) {
                pads.push(Pad {
                    key: zone.keys.0,
                    name: stem(&zone.sample),
                });
            }
        }
        pads.sort_by_key(|pad| pad.key);
        return Map::Pads(kick_snare_hat_first(pads));
    }
    let ranges: Vec<(u8, u8)> = zones.iter().map(|zone| zone.keys).collect();
    let roots: Vec<u8> = zones.iter().map(|zone| zone.root).collect();
    keyboard(&ranges, &roots)
}

/// The line under a keyboard naming the keys its sounds were made at, the dots on it: "Root C5",
/// "Roots C4, C5, C6", or with more than four just how many; none for pads.
pub fn roots_line(map: &Map) -> Option<String> {
    let Map::Keys { roots, .. } = map else {
        return None;
    };
    match roots.as_slice() {
        [] => None,
        [root] => Some(format!("Root {}", key_name(*root))),
        few if few.len() <= 4 => {
            let names: Vec<String> = few.iter().map(|&root| key_name(root)).collect();
            Some(format!("Roots {}", names.join(", ")))
        }
        many => Some(format!("{} roots, dotted on the keys", many.len())),
    }
}

/// A keyboard over `ranges` with `roots` dotted: as many white keys as the map has room for,
/// from the C whose keyboard shows the most of the zones' keys; of those equally good, the one
/// nearest the keyboard centered on the middle of the zones' roots. Why not that center alone: a
/// set with more velocity layers in its low notes counts their roots once a layer, and its
/// keyboard sat an octave low (Electric Guitar: 24-72, its keys 73-86 off the map).
fn keyboard(ranges: &[(u8, u8)], roots: &[u8]) -> Map {
    let middle = if roots.is_empty() {
        60
    } else {
        let sum: u32 = roots.iter().map(|&root| u32::from(root)).sum();
        u8::try_from(sum / roots.len() as u32).unwrap_or(60)
    };
    let span = white_keys();
    // Half the white keys below the middle, at 7 white keys an octave, from a C.
    let octaves_below = (span / 2).div_ceil(7);
    let centered = (middle / 12).saturating_sub(u8::try_from(octaves_below).unwrap_or(0)) * 12;
    let played = |key: u8| ranges.iter().any(|&(lo, hi)| (lo..=hi).contains(&key));
    let shown = |low: u8| {
        key_rects(low)
            .into_iter()
            .filter(|&(key, _)| played(key))
            .count()
    };
    let low = (0..=127u8)
        .step_by(12)
        .max_by_key(|&low| (shown(low), Reverse(low.abs_diff(centered))))
        .unwrap_or(centered);
    let mut roots = roots.to_vec();
    roots.sort_unstable();
    roots.dedup();
    Map::Keys {
        low,
        zones: ranges.to_vec(),
        roots,
    }
}

/// How many white keys fit across the map.
pub fn white_keys() -> usize {
    (MAP_WIDTH / KEY_WIDTH) as usize
}

/// A sound file's name without its folder and extension.
fn stem(sample: &str) -> String {
    std::path::Path::new(sample).file_stem().map_or_else(
        || sample.to_owned(),
        |stem| stem.to_string_lossy().into_owned(),
    )
}

/// The velocity a press at height `y` of a map `height` tall plays: soft at the top, loud at
/// the bottom, 1..=127.
pub fn velocity_at(y: f32, height: f32) -> u8 {
    let fraction = (y / height.max(1.0)).clamp(0.0, 1.0);
    (1.0 + fraction * 126.0).round() as u8
}

/// The map's height: a keyboard's, or every row of pads.
pub fn map_height(map: &Map) -> f32 {
    match map {
        Map::Pads(pads) => {
            let rows = pads.len().div_ceil(PADS_PER_ROW).max(1) as f32;
            (rows * (PAD.height + PAD_GAP) - PAD_GAP).max(PAD.height)
        }
        Map::Keys { .. } => MAP_HEIGHT,
    }
}

/// Where pad `index` of a pad map lies, map coordinates.
pub fn pad_rect(index: usize) -> Rectangle {
    let (row, column) = (index / PADS_PER_ROW, index % PADS_PER_ROW);
    Rectangle::new(
        Point::new(
            column as f32 * (PAD.width + PAD_GAP),
            row as f32 * (PAD.height + PAD_GAP),
        ),
        PAD,
    )
}

/// Whether `key` is a black key.
fn is_black(key: u8) -> bool {
    matches!(key % 12, 1 | 3 | 6 | 8 | 10)
}

/// The white keys of a keyboard from `low` and where each lies, map coordinates; then the black
/// keys over them.
pub fn key_rects(low: u8) -> Vec<(u8, Rectangle)> {
    let mut whites = Vec::new();
    let mut blacks = Vec::new();
    let mut white = 0usize;
    let mut key = low;
    while white < white_keys() {
        let x = white as f32 * KEY_WIDTH;
        if is_black(key) {
            let rect = Rectangle::new(
                Point::new(x - BLACK_WIDTH / 2.0, 0.0),
                Size::new(BLACK_WIDTH, MAP_HEIGHT * BLACK_DEPTH),
            );
            blacks.push((key, rect));
        } else {
            whites.push((
                key,
                Rectangle::new(Point::new(x, 0.0), Size::new(KEY_WIDTH, MAP_HEIGHT)),
            ));
            white += 1;
        }
        match key.checked_add(1) {
            Some(next) if next <= 127 => key = next,
            _ => break,
        }
    }
    whites.extend(blacks);
    whites
}

/// The key under map point `at`: black keys over white ones.
pub fn key_at(map: &Map, at: Point) -> Option<u8> {
    key_and_rect_at(map, at).map(|(key, _)| key)
}

/// The key a press at map point `at` plays and its velocity: the press's height within that
/// key or pad, soft at its top and loud at its foot, so every key and every row of pads
/// reaches 1..=127.
pub fn press_at(map: &Map, at: Point) -> Option<(u8, u8)> {
    let (key, rect) = key_and_rect_at(map, at)?;
    Some((key, velocity_at(at.y - rect.y, rect.height)))
}

/// The key under map point `at` and where its key or pad lies: black keys over white ones.
fn key_and_rect_at(map: &Map, at: Point) -> Option<(u8, Rectangle)> {
    match map {
        Map::Pads(pads) => (0..pads.len())
            .map(|index| (pads[index].key, pad_rect(index)))
            .find(|(_, rect)| rect.contains(at)),
        Map::Keys { low, .. } => key_rects(*low)
            .into_iter()
            .rev()
            .find(|(_, rect)| rect.contains(at)),
    }
}

/// The sound map: pads or keys, laid out on a map [`MAP_WIDTH`] across and drawn stretched so
/// they fill the window across (keys and pads wider, their text not); a press sends the key and
/// the velocity its height gives, the release that it was let go of; the key held lights.
struct SoundMap<'a> {
    map: &'a Map,
    held: Option<(u8, u8)>,
    cache: &'a canvas::Cache,
}

/// The window's drawings kept from one frame to the next: the strip's groups and controls, and
/// the sound map. The app clears them whenever what they show may have changed (every message
/// but the timers'). Why: iced draws every window after every message, so while the transport
/// plays the window redraws at the playhead's rate though nothing in it moves.
pub struct Caches {
    pub strip: canvas::Cache,
    pub map: canvas::Cache,
}

impl Caches {
    pub fn new() -> Self {
        Self {
            strip: canvas::Cache::new(),
            map: canvas::Cache::new(),
        }
    }

    pub fn clear(&self) {
        self.strip.clear();
        self.map.clear();
    }
}

/// The width a map's keys or pads take in its own layout: as many white keys as fit, or a
/// whole row of pads.
pub fn span(map: &Map) -> f32 {
    match map {
        Map::Pads(_) => PADS_PER_ROW as f32 * (PAD.width + PAD_GAP) - PAD_GAP,
        Map::Keys { .. } => white_keys() as f32 * KEY_WIDTH,
    }
}

/// How much wider than its own layout `map` is drawn to fill `width`.
fn stretch(width: f32, map: &Map) -> f32 {
    width.max(1.0) / span(map)
}

/// `rect`, of the map's own layout, as drawn stretched by `x`.
fn stretched(rect: Rectangle, x: f32) -> Rectangle {
    Rectangle::new(
        Point::new(rect.x * x, rect.y),
        Size::new(rect.width * x, rect.height),
    )
}

/// The point of `map`'s own layout under `cursor`, the map drawn in `bounds`.
fn on_map(map: &Map, cursor: mouse::Cursor, bounds: Rectangle) -> Option<Point> {
    let at = cursor.position_in(bounds)?;
    Some(Point::new(at.x / stretch(bounds.width, map), at.y))
}

impl canvas::Program<Message> for SoundMap<'_> {
    /// What the cursor was over last.
    type State = Option<Hover>;

    fn update(
        &self,
        hovered: &mut Option<Hover>,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let send = |message| canvas::Action::publish(Message::Instrument(message));
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let (key, velocity) = press_at(self.map, on_map(self.map, cursor, bounds)?)?;
                Some(send(InstrumentMessage::Pressed { key, velocity }).and_capture())
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                if self.held.is_some() =>
            {
                Some(send(InstrumentMessage::Lifted))
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let now = on_map(self.map, cursor, bounds)
                    .and_then(|at| key_at(self.map, at))
                    .map(|key| Hover::Sound(SoundPart::Key(key)));
                hover_change(hovered, now)
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &Option<Hover>,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let geometry = self.cache.draw(renderer, bounds.size(), |frame| {
            self.draw_map(frame, bounds);
        });
        vec![geometry]
    }

    fn mouse_interaction(
        &self,
        _state: &Option<Hover>,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        match on_map(self.map, cursor, bounds).and_then(|at| key_at(self.map, at)) {
            Some(_) => mouse::Interaction::Pointer,
            None => mouse::Interaction::default(),
        }
    }
}

impl SoundMap<'_> {
    fn draw_map(&self, frame: &mut Frame, bounds: Rectangle) {
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), style::RACK_WELL);
        let held = self.held.map(|(key, _)| key);
        let x = stretch(bounds.width, self.map);
        match self.map {
            Map::Pads(pads) => {
                for (index, pad) in pads.iter().enumerate() {
                    let rect = stretched(pad_rect(index), x);
                    let lit = held == Some(pad.key);
                    let fill = if lit { style::ACCENT } else { style::BUTTON };
                    frame.fill_rectangle(rect.position(), rect.size(), fill);
                    let name = crate::ui::text_fit::fit_width(&pad.name, rect.width - 8.0, 11.0)
                        .map(std::borrow::Cow::into_owned)
                        .unwrap_or_default();
                    label(
                        frame,
                        &name,
                        Point::new(rect.x + 4.0, rect.y + 11.0),
                        style::TEXT,
                        11.0,
                    );
                    let key = key_name(pad.key);
                    let at = Point::new(rect.x + 4.0, rect.y + rect.height - 10.0);
                    label(frame, &key, at, style::TEXT_DIM, 10.0);
                }
            }
            Map::Keys { low, zones, roots } => {
                let inside = |key: u8| zones.iter().any(|&(lo, hi)| (lo..=hi).contains(&key));
                for (key, rect) in key_rects(*low) {
                    let rect = stretched(rect, x);
                    let color = match (held == Some(key), is_black(key), inside(key)) {
                        (true, _, _) => style::ACCENT,
                        (false, true, true) => style::KEY_BLACK,
                        (false, true, false) => style::KNOB_RING,
                        (false, false, true) => style::KEY_WHITE,
                        (false, false, false) => style::TEXT_DIM,
                    };
                    frame.fill_rectangle(rect.position(), rect.size(), color);
                    if !is_black(key) {
                        frame.fill_rectangle(
                            Point::new(rect.x + rect.width - 1.0, rect.y),
                            Size::new(1.0, rect.height),
                            style::KEY_EDGE,
                        );
                    }
                    if roots.contains(&key) {
                        let center = Point::new(rect.center_x(), rect.y + rect.height - 8.0);
                        frame.fill(&Path::circle(center, 3.0), style::ACCENT);
                    }
                    if key % 12 == 0 && !is_black(key) {
                        let at = Point::new(rect.x + 2.0, rect.y + rect.height - 20.0);
                        label(frame, &key_name(key), at, style::KEY_LABEL, 9.0);
                    }
                }
            }
        }
        if let Some((key, velocity)) = self.held {
            let words = format!("{} \u{b7} velocity {velocity}", key_name(key));
            let at = Point::new(bounds.width - 110.0, 10.0);
            label(frame, &words, at, style::ACCENT, 11.0);
        }
    }
}

/// The hint line's word for a control of the strip.
pub fn hover_of(part: Part) -> Hover {
    Hover::Sound(match part {
        Part::Gate(_) => SoundPart::Gate,
        Part::Flag(flag) => SoundPart::Flag(flag),
        Part::Control(control) => SoundPart::Control(control),
        Part::Insert => SoundPart::Insert,
    })
}

/// Two presses on one knob or fader this close together reset it, as the mixer's do.
const DOUBLE_PRESS: Duration = Duration::from_millis(400);

/// The text size of the strip's labels, readouts and group names.
const LABEL_SIZE: f32 = 10.0;

/// The strip of controls, like a mixer's: draws its groups, knobs, lights and faders, the
/// value tip over the control under the cursor or held, and turns presses and drags into the
/// window's messages. A knob or fader moves up with the cursor (down lowers it), a tenth as
/// fast with Shift, from where it was; a double-click resets it.
struct Controls<'a> {
    channel: &'a Channel,
    /// The strip's drawing without the tip, kept between frames.
    cache: &'a canvas::Cache,
    /// What a note's end does as the channel plays now: the gate switch's lit position.
    gate: Gate,
    strip: Strip,
    /// Whether Shift was held as the window last heard: a drag starting now moves fine.
    fine: bool,
}

/// A knob or fader held down: where along its sweep it started, the cursor's height then,
/// how many pixels its sweep takes and whether it moves fine.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Held {
    control: Control,
    start: f32,
    anchor: f32,
    sweep: f32,
    fine: bool,
}

/// What the strip remembers between events.
#[derive(Debug, Default)]
struct ControlsState {
    /// What the cursor was over last, as the hint line names it.
    hover: Option<Hover>,
    held: Option<Held>,
    /// The last press on a knob or fader and when, for a double-click.
    last_press: Option<(Instant, Control)>,
}

impl Controls<'_> {
    fn value(&self, control: Control) -> f32 {
        control.value(self.channel)
    }

    /// Whether a switch's light is lit.
    fn lit(&self, part: Part) -> bool {
        match part {
            Part::Gate(gate) => self.gate == gate,
            Part::Flag(Flag::Reverse) => self.channel.sound.reverse,
            Part::Flag(Flag::Filter) => self.channel.sound.filter,
            Part::Control(_) | Part::Insert => false,
        }
    }

    /// Whether `control` acts now: the cutoff and resonance only with the filter on.
    fn acts(&self, control: Control) -> bool {
        !matches!(control, Control::Cutoff | Control::Resonance) || self.channel.sound.filter
    }

    fn draw_part(&self, frame: &mut Frame, placed: &Placed) {
        let label_color = match placed.part {
            Part::Control(control) if !self.acts(control) => style::TEXT_DIM,
            _ => style::TEXT,
        };
        match (placed.look, placed.part) {
            (Look::Knob { center }, Part::Control(control)) => {
                draw_knob(
                    frame,
                    center,
                    KNOB_SIZE,
                    control.position(self.value(control)),
                );
                centered(
                    frame,
                    placed.label,
                    placed.label_at,
                    label_color,
                    LABEL_SIZE,
                );
            }
            (Look::Fader { track, tall }, Part::Control(control)) => {
                let value = self.value(control);
                frame.fill_rectangle(track.position(), track.size(), style::FADER_TRACK);
                let cap = cap_rect(track, control.position(value), tall);
                frame.fill_rectangle(cap.position(), cap.size(), style::FADER_CAP);
                frame.fill_rectangle(
                    Point::new(cap.x, cap.center_y() - CAP_LINE / 2.0),
                    Size::new(cap.width, CAP_LINE),
                    style::ACCENT,
                );
                if !tall {
                    let readout = control.text(value);
                    centered(frame, &readout, readout_at(track), style::TEXT, LABEL_SIZE);
                }
                centered(
                    frame,
                    placed.label,
                    placed.label_at,
                    label_color,
                    LABEL_SIZE,
                );
            }
            (Look::Light { center }, part) => {
                let color = if self.lit(part) {
                    style::TRACK_LED_ON
                } else {
                    style::LED_OFF
                };
                frame.fill(&Path::circle(center, LIGHT_RADIUS), color);
                label(
                    frame,
                    placed.label,
                    placed.label_at,
                    style::TEXT,
                    LABEL_SIZE,
                );
            }
            (Look::Chooser, _) => {
                centered(
                    frame,
                    placed.label,
                    placed.label_at,
                    style::TEXT,
                    LABEL_SIZE,
                );
            }
            // Its label and its value in one line on a well, which a drag moves as a knob's.
            (Look::Readout, Part::Control(control)) => {
                frame.fill_rectangle(placed.hit.position(), placed.hit.size(), style::FADER_TRACK);
                let words = format!("{} {}", placed.label, control.text(self.value(control)));
                centered(frame, &words, placed.label_at, label_color, LABEL_SIZE);
            }
            (Look::Knob { .. } | Look::Fader { .. } | Look::Readout, _) => {}
        }
    }
}

impl canvas::Program<Message> for Controls<'_> {
    type State = ControlsState;

    fn update(
        &self,
        state: &mut ControlsState,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let send = |message| canvas::Action::publish(Message::Instrument(message));
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let placed = self.strip.hit(cursor.position_in(bounds)?)?;
                let control = match placed.part {
                    Part::Gate(gate) => {
                        return Some(send(InstrumentMessage::Gate(gate)).and_capture());
                    }
                    Part::Flag(flag) => {
                        return Some(send(InstrumentMessage::Toggle(flag)).and_capture());
                    }
                    Part::Control(control) => control,
                    Part::Insert => return None,
                };
                let now = crate::ui::clock::now();
                let double = state.last_press.is_some_and(|(when, last)| {
                    last == control && now.duration_since(when) < DOUBLE_PRESS
                });
                if double {
                    state.last_press = None;
                    state.held = None;
                    return Some(send(InstrumentMessage::Reset(control)).and_capture());
                }
                state.last_press = Some((now, control));
                state.held = Some(Held {
                    control,
                    start: control.position(self.value(control)),
                    anchor: cursor.position()?.y,
                    sweep: placed.sweep(),
                    fine: self.fine,
                });
                Some(canvas::Action::request_redraw().and_capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let Some(held) = state.held {
                    // The cursor, not the event's position: a scrolled body moves the cursor
                    // its content sees, as it did for the anchor, and off the strip's
                    // scrollable the cursor levitates but still says where it is.
                    let y = cursor.land().position()?.y;
                    // Detented at no transposition, so the tune finds its center by feel.
                    let detent = held.control == Control::Semitones;
                    let at = dragged(held.start, held.anchor - y, held.sweep, held.fine, detent);
                    let value = held.control.value_at(at);
                    if value == self.value(held.control) {
                        return None;
                    }
                    return Some(send(InstrumentMessage::Set(held.control, value)));
                }
                let now = cursor
                    .position_in(bounds)
                    .and_then(|at| self.strip.hit(at))
                    .map(|placed| hover_of(placed.part));
                hover_change(&mut state.hover, now)
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                if state.held.is_some() =>
            {
                state.held = None;
                Some(send(InstrumentMessage::Released))
            }
            // Shift pressed or let go of half way through: the drag goes on from where it is,
            // at the new speed, rather than jumping.
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                let anchor = cursor.land().position()?.y;
                let held = state.held.as_mut()?;
                held.start = held.control.position(held.control.value(self.channel));
                held.anchor = anchor;
                held.fine = modifiers.shift();
                None
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        state: &ControlsState,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let strip = self.cache.draw(renderer, bounds.size(), |frame| {
            self.draw_strip(frame);
        });
        let mut frame = Frame::new(renderer, bounds.size());
        // The control held down, else the one under the cursor, says its value as the
        // mixer's controls do: drawn each frame, over the strip kept.
        let hovered = match state.hover {
            Some(Hover::Sound(SoundPart::Control(control))) => Some(control),
            _ => None,
        };
        let tipped = state.held.map(|held| held.control).or(hovered);
        // A readout says its value itself.
        let tipped = tipped.and_then(|control| {
            let placed = self.strip.find(Part::Control(control))?;
            (placed.look != Look::Readout).then_some(control)
        });
        if let Some((control, placed)) =
            tipped.and_then(|control| Some((control, self.strip.find(Part::Control(control))?)))
        {
            let value = self.value(control);
            draw_tip(
                &mut frame,
                placed.tip(control.position(value)),
                &control.text(value),
            );
        }
        vec![strip, frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &ControlsState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.held.is_some() {
            return mouse::Interaction::Grabbing;
        }
        match cursor.position_in(bounds).and_then(|at| self.strip.hit(at)) {
            Some(_) => mouse::Interaction::Pointer,
            None => mouse::Interaction::default(),
        }
    }
}

impl Controls<'_> {
    fn draw_strip(&self, frame: &mut Frame) {
        for group in &self.strip.groups {
            let rect = group.rect;
            frame.fill_rectangle(rect.position(), rect.size(), style::STRIP);
            frame.fill_rectangle(
                rect.position(),
                Size::new(rect.width, HEADER_HEIGHT),
                style::STRIP_SELECTED,
            );
            let title = Point::new(rect.center_x(), HEADER_HEIGHT / 2.0);
            centered(frame, group.group.title(), title, style::TEXT, LABEL_SIZE);
            for placed in &group.parts {
                self.draw_part(frame, placed);
            }
        }
    }
}

/// An insert the output chooser offers, by its name; `id` none for its last row, "Mixer
/// insert…" (an insert of the channel's own, the first free one, as the channel menu's row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsertChoice {
    pub id: Option<InsertId>,
    pub name: String,
}

impl std::fmt::Display for InsertChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

/// An input device the input channel's device picker offers: its name (empty for the
/// system's default) and the words it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceChoice {
    pub name: String,
    pub words: String,
}

impl std::fmt::Display for DeviceChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.words)
    }
}

/// The device channels the input channel's channel picker offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelsChoice(pub InputChannels);

impl std::fmt::Display for ChannelsChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::live_input::channels_words(self.0))
    }
}

/// A port the MIDI OUT group's Port picker offers: its name as listed (none: no MIDI out) and
/// the words it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortChoice {
    pub name: Option<String>,
    pub words: String,
}

impl std::fmt::Display for PortChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.words)
    }
}

/// A program the Program picker offers: none ("Off"), or a program change's byte, shown one
/// higher, as instruments count them (1 to 128).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgramChoice(pub Option<u8>);

impl ProgramChoice {
    /// "Off", then every program.
    pub fn all() -> Vec<Self> {
        std::iter::once(Self(None))
            .chain(PROGRAM_RANGE.map(|program| Self(Some(program))))
            .collect()
    }
}

impl std::fmt::Display for ProgramChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Some(program) => write!(f, "{}", u16::from(program) + 1),
            None => f.write_str("Off"),
        }
    }
}

/// What an input channel's window shows of its MIDI out.
#[derive(Debug, Clone, PartialEq)]
pub struct MidiOutView {
    /// The ports offered: none first, the listed ones (the system's loopback last), the one
    /// chosen among them.
    pub ports: Vec<PortChoice>,
    pub port: PortChoice,
    /// Its settings, when it has a port.
    pub out: Option<MidiOut>,
    /// What the MIDI-out thread counted (sent late, refused, not taken), when anything; and
    /// whether that is a warning.
    pub counts: Option<(String, bool)>,
}

/// What an input channel's window shows of its input.
#[derive(Debug, Clone, PartialEq)]
pub struct InputView {
    /// The devices offered, the system's default first, the one chosen among them.
    pub devices: Vec<DeviceChoice>,
    pub device: DeviceChoice,
    /// The channels offered on the chosen device, and the ones chosen.
    pub channels: Vec<ChannelsChoice>,
    pub picked: ChannelsChoice,
    /// The input's level, 0 to 1 and over.
    pub level: f32,
    /// How far behind it is heard, or why its device did not open.
    pub status: String,
    /// Its device did not open: the status is a warning.
    pub refused: bool,
}

/// What the window shows for one channel.
pub struct View<'a> {
    pub channel: &'a Channel,
    /// An input channel's input; `None` for every other kind.
    pub input: Option<InputView>,
    /// An input channel's MIDI out; `None` for every other kind.
    pub midi_out: Option<MidiOutView>,
    /// Who made its set, when it says.
    pub maker: &'a str,
    /// The inserts its output chooser offers, in the mixer's order, then "Mixer insert…".
    pub inserts: Vec<InsertChoice>,
    pub map: &'a Map,
    pub zones: &'a [ZoneInfo],
    pub held: Option<(u8, u8)>,
    pub more: bool,
    /// Whether settings are stored for Compare.
    pub compared: bool,
    /// What a note's end does as the channel plays now: its own gate, else its sound's.
    pub gate: Gate,
    /// The line at the window's foot: the control under the cursor, or how to play it.
    pub hint: String,
    /// The window's width, as it last said.
    pub width: f32,
    /// Whether Shift is held: a drag starting now moves fine.
    pub fine: bool,
    /// The drawings kept between frames.
    pub caches: &'a Caches,
}

/// The size of the instrument window's foot line, and its height.
pub const HINT_SIZE: f32 = 11.0;
pub const HINT_HEIGHT: f32 = 22.0;

/// How to play a channel of `kind` from its window, its foot line while no control is under the
/// cursor.
pub fn help(kind: ChannelKind, keys: &crate::keys::Keys) -> String {
    let switch = keys.first(crate::keys::Action::Keyboard);
    match kind {
        ChannelKind::Plugin => format!(
            "The computer keyboard plays it ({switch} switches it); the plugin's own window has the rest."
        ),
        // Live notes (the keys, the map, a device) sound nothing on it: its clips on the
        // playlist play its file.
        ChannelKind::Audio => "An audio channel plays its clip; notes do nothing here.".to_owned(),
        // Its sound is its input, heard whether the song plays or not.
        ChannelKind::Input => {
            "An input channel plays its audio input live; notes do nothing here.".to_owned()
        }
        ChannelKind::Sampler | ChannelKind::Instrument => format!(
            "Press a pad or key; lower is louder. The computer keyboard plays it too ({switch} switches it)."
        ),
    }
}

/// Whether `channel`'s window offers Edit sample…: a sampler or an audio channel with a file.
pub fn edits_a_file(channel: &Channel) -> bool {
    channel.plays_a_file() && !channel.sample.as_os_str().is_empty()
}

/// Whether `channel` plays a VST3 plugin.
pub fn plays_vst3(channel: &Channel) -> bool {
    channel.kind == ChannelKind::Plugin
        && channel
            .processor
            .as_ref()
            .and_then(|processor| processor.format())
            == Some("vst3")
}

/// What a channel's window calls its kind: "VST3 plugin instrument" for one playing a VST3
/// plugin, else its kind's name.
pub fn kind_words(channel: &Channel) -> &'static str {
    if plays_vst3(channel) {
        "VST3 plugin instrument"
    } else {
        kind_name(channel.kind)
    }
}

/// The window's content, filling its window: the channel's name over its strip of controls
/// (scrolled across when the window is narrower than the strip), the sound map across the
/// window (a plugin channel: its own window's button), Store A, A/B and the zones in a thin
/// footer, the zones' list when unfolded, and the foot line.
pub fn view(shown: View<'_>) -> Element<'_, Message> {
    let instrument = |message| Message::Instrument(message);
    let channel = shown.channel;
    let kind = channel.kind;
    let room = (shown.width - 2.0 * PADDING).max(0.0);
    let about = if shown.maker.is_empty() {
        kind_words(channel).to_owned()
    } else {
        format!("{} \u{b7} by {}", kind_words(channel), shown.maker)
    };
    let title = row![
        text(channel.name.clone()).size(15),
        text(about).size(11).color(style::TEXT_DIM),
    ]
    .spacing(10)
    .align_y(Center);
    let mut body = column![title, strip_view(&shown, room)].spacing(8);
    if let Some(input) = shown.input.clone() {
        body = body.push(input_section(input));
        if let Some(out) = shown.midi_out.clone() {
            body = body.push(midi_out_section(out));
        }
    } else if kind == ChannelKind::Plugin {
        let open = button(text("Open plugin window").size(12))
            .padding([4, 10])
            .style(style::tool_button(false))
            .on_press(instrument(InstrumentMessage::OpenPlugin));
        body = body.push(hover_area(open, Hover::Sound(SoundPart::OpenPlugin)));
    } else {
        let map = canvas_widget(SoundMap {
            map: shown.map,
            held: shown.held,
            cache: &shown.caches.map,
        })
        .width(Fill)
        .height(map_height(shown.map));
        body = body.push(map);
    }
    body = body.push(footer(&shown));
    if kind == ChannelKind::Instrument && shown.more {
        let lines = shown.zones.iter().map(|zone| {
            text(format!(
                "{}\u{2013}{}  root {}  velocity {}\u{2013}{}  {}",
                key_name(zone.keys.0),
                key_name(zone.keys.1),
                key_name(zone.root),
                zone.velocities.0,
                zone.velocities.1,
                zone.sample
            ))
            .size(11)
            .color(style::TEXT_DIM)
            .into()
        });
        body = body.push(column(lines).spacing(2));
    }
    // Cut with an ellipsis to the window's width, as the main window's hint bar is.
    let hint = crate::ui::text_fit::fit_width(&shown.hint, room, HINT_SIZE)
        .map(std::borrow::Cow::into_owned)
        .unwrap_or_default();
    panel(body, hint)
}

/// The strip of controls over `room` across, the output chooser over its place.
fn strip_view<'a>(shown: &View<'a>, room: f32) -> Element<'a, Message> {
    let strip = Strip::of(shown.channel.kind, room);
    let width = strip.width;
    let chooser = strip.find(Part::Insert).map(|placed| placed.hit);
    let controls = canvas_widget(Controls {
        channel: shown.channel,
        cache: &shown.caches.strip,
        gate: shown.gate,
        strip,
        fine: shown.fine,
    })
    .width(width)
    .height(STRIP_HEIGHT);
    let mut layers: Vec<Element<'a, Message>> = vec![controls.into()];
    if let Some(rect) = chooser {
        let selected = shown
            .inserts
            .iter()
            .find(|choice| choice.id == Some(shown.channel.insert))
            .cloned();
        let list = pick_list(shown.inserts.clone(), selected, |choice: InsertChoice| {
            Message::Instrument(match choice.id {
                Some(insert) => InstrumentMessage::Route(insert),
                None => InstrumentMessage::MixerInsert,
            })
        })
        .text_size(LABEL_SIZE + 1.0)
        .padding([2, 4])
        .width(rect.width)
        .style(style::chooser)
        .menu_style(style::chooser_menu);
        let list = hover_area(list, Hover::Sound(SoundPart::Insert));
        layers.push(pin(list).x(rect.x).y(rect.y).into());
    }
    let strip = stack(layers).width(width).height(STRIP_HEIGHT);
    if width > room {
        scrollable(strip)
            .direction(Direction::Horizontal(
                Scrollbar::new().width(4).scroller_width(4),
            ))
            .width(Fill)
            .into()
    } else {
        strip.into()
    }
}

/// The thin footer: Store A and A/B with the keys the sounds were made at, then the zones'
/// list's button (a set's), Edit sample… (a channel playing a file of its own) and Close.
fn footer<'a>(shown: &View<'a>) -> Element<'a, Message> {
    let instrument = |message| Message::Instrument(message);
    let kind = shown.channel.kind;
    let mut footer = row![].spacing(4).align_y(Center);
    // A plugin's and an input's sound has no settings of the window's to store.
    if !matches!(kind, ChannelKind::Plugin | ChannelKind::Input) {
        let store = button(text("Store A").size(12))
            .padding([2, 8])
            .style(style::tool_button(false))
            .on_press(instrument(InstrumentMessage::Store));
        let compare = button(text("A/B").size(12))
            .padding([2, 8])
            .style(style::tool_button(shown.compared))
            .on_press_maybe(
                shown
                    .compared
                    .then(|| instrument(InstrumentMessage::Compare)),
            );
        let compare = row![store, compare].spacing(2);
        footer = footer.push(hover_area(compare, Hover::Sound(SoundPart::Compare)));
        if let Some(roots) = roots_line(shown.map) {
            footer = footer
                .push(space().width(4))
                .push(text(roots).size(11).color(style::TEXT_DIM));
        }
    }
    footer = footer.push(space().width(Fill));
    if kind == ChannelKind::Instrument {
        let more = button(
            text(if shown.more {
                "Zones \u{25be}"
            } else {
                "Zones \u{25b8}"
            })
            .size(12),
        )
        .padding([2, 8])
        .style(style::tool_button(shown.more))
        .on_press(instrument(InstrumentMessage::More));
        footer = footer.push(hover_area(more, Hover::Sound(SoundPart::More)));
    }
    if edits_a_file(shown.channel) {
        let edit = button(text("Edit sample\u{2026}").size(12))
            .padding([2, 8])
            .style(style::tool_button(false))
            .on_press(instrument(InstrumentMessage::EditSample));
        footer = footer.push(hover_area(edit, Hover::Sound(SoundPart::EditSample)));
    }
    let close = button(text("Close").size(12))
        .padding([2, 8])
        .style(style::tool_button(false))
        .on_press(instrument(InstrumentMessage::Close));
    footer
        .push(hover_area(close, Hover::Sound(SoundPart::Close)))
        .into()
}

/// The width of an input row's label, and the size of its words.
pub const INPUT_LABEL: f32 = 64.0;
const INPUT_SIZE: f32 = 12.0;
/// The input meter's size.
const INPUT_METER: Size = Size::new(220.0, 8.0);

/// An input channel's input: its device (with Refresh), its channels, its meter, and how far
/// behind it is heard or why its device did not open.
fn input_section<'a>(input: InputView) -> Element<'a, Message> {
    let instrument = |message| Message::Instrument(message);
    let label = |words: &'static str| {
        text(words)
            .size(INPUT_SIZE)
            .color(style::TEXT_DIM)
            .width(INPUT_LABEL)
    };
    let devices = pick_list(
        input.devices,
        Some(input.device),
        move |choice: DeviceChoice| instrument(InstrumentMessage::InputDevice(choice.name)),
    )
    .text_size(INPUT_SIZE)
    .padding([2, 6])
    .width(Fill)
    .style(style::chooser)
    .menu_style(style::chooser_menu);
    let refresh = button(text("Refresh").size(INPUT_SIZE))
        .padding([2, 8])
        .style(style::tool_button(false))
        .on_press(instrument(InstrumentMessage::RefreshInputs));
    let device_row = row![
        label("Device"),
        hover_area(devices, Hover::Sound(SoundPart::InputDevice)),
        hover_area(refresh, Hover::Sound(SoundPart::InputRefresh)),
    ]
    .spacing(6)
    .align_y(Center);
    let channels = pick_list(
        input.channels,
        Some(input.picked),
        move |choice: ChannelsChoice| instrument(InstrumentMessage::InputChannels(choice.0)),
    )
    .text_size(INPUT_SIZE)
    .padding([2, 6])
    .width(160)
    .style(style::chooser)
    .menu_style(style::chooser_menu);
    let channels_row = row![
        label("Channels"),
        hover_area(channels, Hover::Sound(SoundPart::InputChannels)),
    ]
    .spacing(6)
    .align_y(Center);
    let meter = canvas_widget(InputMeter { level: input.level })
        .width(INPUT_METER.width)
        .height(INPUT_METER.height);
    let level_row = row![
        label("Level"),
        hover_area(meter, Hover::Sound(SoundPart::InputLevel)),
    ]
    .spacing(6)
    .align_y(Center);
    let color = if input.refused {
        style::WARNING
    } else {
        style::TEXT_DIM
    };
    let status = text(input.status).size(11).color(color);
    column![device_row, channels_row, level_row, status]
        .spacing(6)
        .into()
}

/// The MIDI OUT group's title rule's height, and the latency's words' width.
pub const MIDI_OUT_RULE: f32 = 2.0;
const LATENCY_WORDS: f32 = 48.0;
/// The Channel and Program pickers' widths.
pub const MIDI_CHANNEL_WIDTH: f32 = 60.0;
const PROGRAM_WIDTH: f32 = 80.0;

/// An input channel's MIDI out: the group's title over its accent rule, the Port picker, and
/// with a port the MIDI channel and program, the latency slider (one undo step a drag) and Echo.
/// The pickers line up with the input's, after the same column of labels.
fn midi_out_section<'a>(shown: MidiOutView) -> Element<'a, Message> {
    let instrument = |change| Message::Instrument(InstrumentMessage::MidiOut(change));
    let label = |words: &'static str| {
        text(words)
            .size(INPUT_SIZE)
            .color(style::TEXT_DIM)
            .width(INPUT_LABEL)
    };
    let words = |words: &'static str| text(words).size(INPUT_SIZE).color(style::TEXT_DIM);
    let rule = container(space())
        .width(Fill)
        .height(MIDI_OUT_RULE)
        .style(|_theme: &Theme| container::Style {
            background: Some(iced::Background::Color(style::ACCENT)),
            ..container::Style::default()
        });
    let title = row![text("MIDI OUT").size(10).color(style::TEXT_DIM), rule]
        .spacing(6)
        .align_y(Center);
    let ports = pick_list(shown.ports, Some(shown.port), move |choice: PortChoice| {
        instrument(MidiOutChange::Port(choice.name))
    })
    .text_size(INPUT_SIZE)
    .padding([2, 6])
    .width(Fill)
    .style(style::chooser)
    .menu_style(style::chooser_menu);
    let port_row = row![
        label("Port"),
        hover_area(ports, Hover::Sound(SoundPart::MidiOutPort)),
    ]
    .spacing(6)
    .align_y(Center);
    let mut group = column![title, port_row].spacing(6);
    if let Some(out) = shown.out {
        let channels: Vec<u8> = MIDI_OUT_CHANNELS.collect();
        let channel = pick_list(channels, Some(out.channel), move |channel| {
            instrument(MidiOutChange::Channel(channel))
        })
        .text_size(INPUT_SIZE)
        .padding([2, 6])
        .width(MIDI_CHANNEL_WIDTH)
        .style(style::chooser)
        .menu_style(style::chooser_menu);
        let program = pick_list(
            ProgramChoice::all(),
            Some(ProgramChoice(out.program)),
            move |choice: ProgramChoice| instrument(MidiOutChange::Program(choice.0)),
        )
        .text_size(INPUT_SIZE)
        .padding([2, 6])
        .width(PROGRAM_WIDTH)
        .style(style::chooser)
        .menu_style(style::chooser_menu);
        let channel_row = row![
            label("Channel"),
            hover_area(channel, Hover::Sound(SoundPart::MidiOutChannel)),
            space().width(6),
            words("Program"),
            hover_area(program, Hover::Sound(SoundPart::MidiOutProgram)),
        ]
        .spacing(6)
        .align_y(Center);
        let latency = slider(LATENCY_RANGE, out.latency_ms, move |ms| {
            instrument(MidiOutChange::Latency(ms))
        })
        .on_release(Message::Instrument(InstrumentMessage::Released))
        .width(Fill);
        let echo = button(text("Echo").size(INPUT_SIZE))
            .padding([2, 8])
            .style(style::tool_button(out.echo))
            .on_press(instrument(MidiOutChange::Echo(!out.echo)));
        let latency_row = row![
            label("Latency"),
            hover_area(latency, Hover::Sound(SoundPart::MidiOutLatency)),
            text(format!("{} ms", out.latency_ms))
                .size(INPUT_SIZE)
                .width(LATENCY_WORDS),
            hover_area(echo, Hover::Sound(SoundPart::MidiOutEcho)),
        ]
        .spacing(6)
        .align_y(Center);
        group = group.push(channel_row).push(latency_row);
    }
    if let Some((counts, warning)) = shown.counts {
        let color = if warning {
            style::WARNING
        } else {
            style::TEXT_DIM
        };
        group = group.push(text(counts).size(11).color(color));
    }
    group.into()
}

/// The input's meter: a bar on a decibel scale from −60 dB to full scale, green to −6 dB,
/// yellow to 0, red past it.
struct InputMeter {
    level: f32,
}

/// The decibels the input meter's left end stands for.
const METER_FLOOR_DB: f32 = -60.0;

/// Where `level` falls along the input meter, 0 to 1.
pub fn meter_position(level: f32) -> f32 {
    if level <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * level.log10();
    ((db - METER_FLOOR_DB) / -METER_FLOOR_DB).clamp(0.0, 1.0)
}

impl<Message> canvas::Program<Message> for InputMeter {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), style::METER_GROUND);
        let width = bounds.width * meter_position(self.level);
        let color = if self.level > 1.0 {
            style::METER_RED
        } else if self.level > 0.5 {
            style::METER_YELLOW
        } else {
            style::METER_GREEN
        };
        frame.fill_rectangle(Point::ORIGIN, Size::new(width, bounds.height), color);
        vec![frame.into_geometry()]
    }
}

/// The kind's name under the channel's.
pub fn kind_name(kind: ChannelKind) -> &'static str {
    match kind {
        ChannelKind::Sampler => "Sampler",
        ChannelKind::Instrument => "Instrument set",
        ChannelKind::Plugin => "Plugin instrument",
        ChannelKind::Audio => "Audio",
        ChannelKind::Input => "External input",
    }
}

/// The window's ground: `body`, scrolling when taller than the window, over the foot line
/// holding `hint`.
fn panel(body: iced::widget::Column<'_, Message>, hint: String) -> Element<'_, Message> {
    let content = container(body).padding(PADDING).width(Fill);
    let foot = container(
        text(hint)
            .size(HINT_SIZE)
            .color(style::TEXT_DIM)
            .wrapping(iced::widget::text::Wrapping::None),
    )
    .padding([0.0, PADDING])
    .width(Fill)
    .height(HINT_HEIGHT)
    .align_y(Center)
    .style(style::window);
    container(column![scrollable(content).height(Fill), foot])
        .width(Fill)
        .height(Fill)
        .style(style::panel)
        .into()
}

/// `sound` with `control` (a sound setting) at `value`, kept in range.
pub fn with_value(mut sound: Sound, control: Control, value: f32) -> Sound {
    let clamp = |range: std::ops::RangeInclusive<f32>| value.clamp(*range.start(), *range.end());
    match control {
        Control::Volume | Control::Pan => {}
        Control::Semitones => {
            sound.semitones = value.round().clamp(
                f32::from(*SEMITONE_RANGE.start()),
                f32::from(*SEMITONE_RANGE.end()),
            ) as i8;
        }
        Control::Cents => {
            sound.cents = value
                .round()
                .clamp(f32::from(*CENT_RANGE.start()), f32::from(*CENT_RANGE.end()))
                as i8;
        }
        Control::Velocity => sound.velocity = clamp(VELOCITY_RANGE),
        Control::Attack => sound.attack = clamp(ATTACK_RANGE),
        Control::Release => sound.release = clamp(RELEASE_RANGE),
        Control::Start => sound.start = clamp(START_RANGE),
        Control::Cutoff => sound.cutoff = clamp(dawsome_engine::project::CUTOFF_RANGE),
        Control::Resonance => sound.resonance = clamp(RESONANCE_RANGE),
        Control::BendRange => {
            sound.bend_range = value
                .round()
                .clamp(f32::from(*BEND_RANGE.start()), f32::from(*BEND_RANGE.end()))
                as u8;
        }
    }
    sound
}

/// `control`'s default value, as a reset gives it.
pub fn default_value(control: Control) -> f32 {
    let defaults = Channel::sampler("", "");
    control.value(&defaults)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone(keys: (u8, u8), root: u8, sample: &str) -> ZoneInfo {
        ZoneInfo {
            keys,
            velocities: (1, 127),
            root,
            sample: sample.to_owned(),
        }
    }

    // The cut A4 left: a drum set's pads start with its first kick, snare and hat (by their
    // sounds' names: "kick" or "bd…", "snare" or "sn_…", "hat" or "hh"), the rest after them in
    // key order; a set without them stays in key order.
    #[test]
    fn a_drum_sets_pads_start_with_a_kick_a_snare_and_a_hat() {
        let drums = [
            zone((36, 36), 36, "s/bd_808.flac"),
            zone((37, 37), 37, "s/bd_ada.flac"),
            zone((38, 38), 38, "s/sn_dub.flac"),
            zone((40, 40), 40, "s/mid_snare_center_vl5.flac"),
            zone((42, 42), 42, "s/hat_cab.flac"),
            zone((49, 49), 49, "s/crash.flac"),
        ];
        let Map::Pads(pads) = map_of(ChannelKind::Instrument, "Kit", &drums, false) else {
            panic!("pads")
        };
        let keys: Vec<u8> = pads.iter().map(|pad| pad.key).collect();
        assert_eq!(keys, [36, 38, 42, 37, 40, 49]);
        let toms = [
            zone((45, 45), 45, "t/tom_hi.wav"),
            zone((41, 41), 41, "t/tom_lo.wav"),
        ];
        let Map::Pads(pads) = map_of(ChannelKind::Instrument, "Toms", &toms, false) else {
            panic!("pads")
        };
        assert_eq!(pads.iter().map(|pad| pad.key).collect::<Vec<_>>(), [41, 45]);
    }

    // A set of one key per zone is pads, one per key in key order, named after the sounds; a
    // starter kit drum is one pad; a pitched set is a keyboard from a C around its roots; a
    // sampler of another sound a keyboard rooted at 60.
    #[test]
    fn drums_are_pads_and_pitched_sounds_a_keyboard() {
        let drums = [
            zone((38, 38), 38, "kit/snare.wav"),
            zone((36, 36), 36, "kit/kick.wav"),
        ];
        assert_eq!(
            map_of(ChannelKind::Instrument, "Kit", &drums, false),
            Map::Pads(vec![
                Pad {
                    key: 36,
                    name: "kick".to_owned()
                },
                Pad {
                    key: 38,
                    name: "snare".to_owned()
                },
            ])
        );
        assert_eq!(
            map_of(ChannelKind::Sampler, "Kick 909", &[], true),
            Map::Pads(vec![Pad {
                key: 60,
                name: "Kick 909".to_owned()
            }])
        );
        let piano = [zone((0, 54), 48, "a.wav"), zone((55, 127), 72, "b.wav")];
        let Map::Keys { low, roots, .. } = map_of(ChannelKind::Instrument, "Piano", &piano, false)
        else {
            panic!("a keyboard")
        };
        // Roots 48 and 72 center on 60; 29 white keys, 14 below: two octaves under C5.
        assert_eq!((low, roots), (36, vec![48, 72]));
        assert_eq!(white_keys(), 29);
        assert!(matches!(
            map_of(ChannelKind::Sampler, "Pluck", &[], false),
            Map::Keys { low: 36, .. }
        ));
    }

    /// A set's zones from its key splits, each `(lokey, hikey, root, layers)` listed once per
    /// velocity layer, as the set's .sfz lists its regions.
    fn layered(splits: &[(u8, u8, u8, usize)]) -> Vec<ZoneInfo> {
        splits
            .iter()
            .flat_map(|&(low, high, root, layers)| {
                (0..layers).map(move |_| zone((low, high), root, "s.flac"))
            })
            .collect()
    }

    /// The keys of `map`'s keyboard that a zone plays, and those that play nothing.
    fn played_and_silent(map: &Map, zones: &[ZoneInfo]) -> (usize, usize) {
        let Map::Keys { low, .. } = map else {
            panic!("a keyboard")
        };
        let keys: Vec<u8> = key_rects(*low).into_iter().map(|(key, _)| key).collect();
        let played = keys
            .iter()
            .filter(|&&key| zones.iter().any(|z| (z.keys.0..=z.keys.1).contains(&key)))
            .count();
        (played, keys.len() - played)
    }

    // The guitar sets (kit b22add6) in the instrument window: the keyboard shows 29 white keys,
    // 49 keys from a C, so it starts at the C that shows the most of the set's keys. Counted by
    // hand from each .sfz's splits: Electric Guitar plays 35-86, two layers up to key 65 and
    // one above; from 36 the keyboard shows 36-84, every key played, where the roots' average
    // (57, the low roots counted twice) put it at 24: 24-72, 11 silent keys and 73-86 off the
    // map. Bass Guitar plays 21-67: from 24, 44 keys played (24-67), from 12 only 40 (21-60).
    // Acoustic Guitar plays 36-88: from 36, 49 played (36-84), as before; from 48 only 41.
    // A set no keyboard holds whole keeps the keyboard centered on its roots (the piano test).
    #[test]
    fn a_sets_keyboard_shows_the_most_of_its_keys() {
        let electric = layered(&[
            (35, 38, 36, 2),
            (39, 40, 40, 2),
            (41, 42, 41, 2),
            (43, 46, 45, 2),
            (47, 49, 48, 2),
            (50, 51, 50, 2),
            (52, 53, 52, 2),
            (54, 56, 55, 2),
            (57, 59, 59, 2),
            (60, 62, 61, 2),
            (63, 65, 64, 2),
            (66, 69, 67, 1),
            (70, 71, 71, 1),
            (72, 72, 72, 1),
            (73, 75, 74, 1),
            (76, 78, 77, 1),
            (79, 81, 80, 1),
            (82, 83, 82, 1),
            (84, 86, 85, 1),
        ]);
        let bass: Vec<ZoneInfo> = layered(
            &(0..13)
                .map(|n| (21 + 3 * n + u8::from(n > 0), 24 + 3 * n, 23 + 3 * n, 4))
                .chain([(61, 67, 62, 4)])
                .collect::<Vec<_>>(),
        );
        let acoustic = layered(&[
            (36, 38, 37, 4),
            (39, 41, 40, 4),
            (42, 43, 42, 4),
            (44, 46, 45, 4),
            (47, 49, 48, 4),
            (50, 52, 51, 4),
            (53, 55, 54, 4),
            (56, 58, 57, 4),
            (59, 61, 60, 4),
            (62, 64, 63, 4),
            (65, 67, 66, 4),
            (68, 70, 69, 4),
            (71, 73, 72, 4),
            (74, 76, 75, 4),
            (77, 79, 78, 4),
            (80, 82, 81, 4),
            (83, 88, 84, 4),
        ]);
        assert_eq!((electric.len(), bass.len(), acoustic.len()), (30, 56, 68));
        assert_eq!(
            bass[16].keys,
            (34, 36),
            "the bass's splits as its .sfz has them"
        );
        for (name, zones, low, played, silent) in [
            ("Electric Guitar", &electric, 36, 49, 0),
            ("Bass Guitar", &bass, 24, 44, 5),
            ("Acoustic Guitar", &acoustic, 36, 49, 0),
        ] {
            let map = map_of(ChannelKind::Instrument, name, zones, false);
            let Map::Keys { low: shown, .. } = &map else {
                panic!("{name}: a keyboard")
            };
            assert_eq!(*shown, low, "{name}");
            assert_eq!(played_and_silent(&map, zones), (played, silent), "{name}");
        }
    }

    // B5: the keys the sounds were made at, named the one way (key 60 = C5): a sampler's root
    // 60, a set's roots, a set of many roots by their count; pads name no root.
    #[test]
    fn the_roots_line_names_the_keys_with_60_as_c5() {
        let keys = |roots: Vec<u8>| Map::Keys {
            low: 36,
            zones: vec![(0, 127)],
            roots,
        };
        let sampler = map_of(ChannelKind::Sampler, "Pluck", &[], false);
        assert_eq!(roots_line(&sampler).as_deref(), Some("Root C5"));
        assert_eq!(
            roots_line(&keys(vec![48, 60, 72])).as_deref(),
            Some("Roots C4, C5, C6")
        );
        assert_eq!(
            roots_line(&keys((40..50).collect())).as_deref(),
            Some("10 roots, dotted on the keys")
        );
        assert_eq!(roots_line(&Map::Pads(vec![])), None);
    }

    // The velocity is the press's height within the key or pad pressed, so
    // a black key and every row of pads reach 1..127: black key 37 plays 127 at its foot (it
    // reached 77 of the whole map's height), the top row's first pad 1 at its top and 127 at
    // its foot (59 at most before), the second row's first pad 1 at its top.
    #[test]
    fn a_press_plays_the_velocity_of_its_height_within_its_key() {
        let keys = Map::Keys {
            low: 36,
            zones: vec![(0, 127)],
            roots: vec![60],
        };
        let black = key_rects(36)
            .into_iter()
            .find(|(key, _)| *key == 37)
            .map(|(_, rect)| rect)
            .expect("key 37");
        let foot = Point::new(black.center_x(), black.y + black.height - 0.01);
        assert_eq!(press_at(&keys, foot), Some((37, 127)));
        let pads = Map::Pads(
            (0..12)
                .map(|key| Pad {
                    key: 36 + key,
                    name: format!("pad {key}"),
                })
                .collect(),
        );
        let first = pad_rect(0);
        assert_eq!(
            press_at(&pads, Point::new(first.center_x(), first.y)),
            Some((36, 1))
        );
        assert_eq!(
            press_at(
                &pads,
                Point::new(first.center_x(), first.y + first.height - 0.01)
            ),
            Some((36, 127))
        );
        let second_row = pad_rect(PADS_PER_ROW);
        assert_eq!(
            press_at(&pads, Point::new(second_row.center_x(), second_row.y)),
            Some((36 + PADS_PER_ROW as u8, 1))
        );
    }

    // Lower on a key is louder: the top plays 1, the bottom 127; a press on a black key beats
    // the white one under it; the pads' grid.
    #[test]
    fn a_press_plays_the_key_under_it_at_the_velocity_of_its_height() {
        assert_eq!(velocity_at(0.0, MAP_HEIGHT), 1);
        assert_eq!(velocity_at(MAP_HEIGHT, MAP_HEIGHT), 127);
        assert_eq!(velocity_at(MAP_HEIGHT / 2.0, MAP_HEIGHT), 64);
        let keys = Map::Keys {
            low: 36,
            zones: vec![(0, 127)],
            roots: vec![60],
        };
        assert_eq!(
            key_at(&keys, Point::new(KEY_WIDTH / 2.0, MAP_HEIGHT - 5.0)),
            Some(36)
        );
        assert_eq!(
            key_at(&keys, Point::new(KEY_WIDTH, 5.0)),
            Some(37),
            "a black key"
        );
        assert_eq!(
            key_at(&keys, Point::new(KEY_WIDTH * 1.5, MAP_HEIGHT - 5.0)),
            Some(38)
        );
        let pads = Map::Pads(vec![
            Pad {
                key: 36,
                name: String::new(),
            },
            Pad {
                key: 38,
                name: String::new(),
            },
        ]);
        assert_eq!(key_at(&pads, pad_rect(1).center()), Some(38));
        assert_eq!(
            key_at(&pads, Point::new(PAD.width + 1.0, 5.0)),
            None,
            "between pads"
        );
    }

    // Known answers, by hand (python3 -I): the volume's fader is the cube law over 0..4, so
    // unity sits at 0.62996 of its travel and half way plays 0.5; the attack's and the
    // release's the square law, so half way is 0.5 s and 1 s and 20 ms sits at a tenth; the
    // cutoff knob's middle is 20 Hz x 1000^0.5 = 632.4555 Hz; the tune is whole semitones
    // straight across -12..12 (0.52 of the turn is +0.48 st, still 0; 0.525 is +1). The
    // volume reads in dB as the mixer's faders do: half is -6.0 dB, the top +12.0 dB.
    #[test]
    fn the_controls_sweep_by_their_laws() {
        let close = |a: f32, b: f32| (a - b).abs() < 1e-4;
        assert!(close(Control::Volume.position(1.0), 0.62996));
        assert!(close(Control::Volume.value_at(0.5), 0.5));
        assert_eq!(Control::Volume.value_at(1.0), 4.0);
        assert!(close(Control::Attack.value_at(0.5), 0.5));
        assert!(close(Control::Release.value_at(0.5), 1.0));
        assert!(close(Control::Attack.position(0.02), 0.1));
        assert!(close(Control::Cutoff.value_at(0.5), 632.4555));
        assert!(close(Control::Cutoff.position(2000.0), 2.0 / 3.0));
        assert_eq!(Control::Semitones.value_at(0.5), 0.0);
        assert_eq!(Control::Semitones.value_at(0.0), -12.0);
        assert_eq!(Control::Semitones.value_at(1.0), 12.0);
        assert_eq!(Control::Semitones.value_at(0.52), 0.0);
        assert_eq!(Control::Semitones.value_at(0.525), 1.0);
        assert_eq!(Control::Pan.value_at(0.75), 0.5);
        for control in Control::ALL {
            for at in [0.0, 0.25, 0.5, 0.9, 1.0] {
                let value = control.value_at(at);
                assert!(
                    close(control.value_at(control.position(value)), value),
                    "{control:?} at {at}"
                );
            }
        }
        assert_eq!(Control::Volume.text(0.5), "\u{2212}6.0 dB");
        assert_eq!(Control::Volume.text(4.0), "+12.0 dB");
        assert_eq!(Control::Volume.text(1.0), "0.0 dB");
    }

    /// The strip's canvas taking `event` with the cursor at `at` (strip coordinates): the
    /// window's message it publishes, if any.
    fn strip_event(
        controls: &Controls<'_>,
        state: &mut ControlsState,
        event: Event,
        at: Point,
    ) -> Option<InstrumentMessage> {
        use canvas::Program;
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(controls.strip.width, STRIP_HEIGHT));
        let action = controls.update(state, &event, bounds, mouse::Cursor::Available(at))?;
        match action.into_inner().0? {
            Message::Instrument(message) => Some(message),
            other => panic!("not the window's: {other:?}"),
        }
    }

    // v2: the strip's knobs and faders are the mixer's: a press holds one, a move up raises it
    // from where it was (the pan from the center 50 px up: 0.5), with Shift a tenth as fast
    // from where it is then, the release ends the drag (one undo step), a second press soon
    // after resets it; a light's press switches it. A move that keeps the value sends nothing.
    #[test]
    fn the_strips_knobs_fade_switch_and_reset_as_the_mixers_do() {
        use crate::ui::instrument_layout::Part;
        let channel = Channel::sampler("kick", "kick.wav");
        let cache = canvas::Cache::new();
        let controls = Controls {
            channel: &channel,
            cache: &cache,
            gate: Gate::OneShot,
            strip: Strip::of(ChannelKind::Sampler, 536.0),
            fine: false,
        };
        let pan = controls
            .strip
            .find(Part::Control(Control::Pan))
            .expect("placed")
            .hit
            .center();
        let mut state = ControlsState::default();
        let press = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        // A move's own position is the window's, 300 px off what the strip's cursor says in a
        // body scrolled down 300 px: the drag goes by the cursor.
        let moved = |y: f32| {
            Event::Mouse(mouse::Event::CursorMoved {
                position: Point::new(pan.x, y + 300.0),
            })
        };
        let release = Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left));
        let instrument = Some;
        assert_eq!(strip_event(&controls, &mut state, press.clone(), pan), None);
        assert_eq!(
            strip_event(&controls, &mut state, moved(pan.y), pan),
            None,
            "the value it has"
        );
        let up = |y: f32| Point::new(pan.x, y);
        assert!(matches!(
            strip_event(&controls, &mut state, moved(pan.y - 50.0), up(pan.y - 50.0)),
            Some(InstrumentMessage::Set(Control::Pan, value)) if value == 0.5
        ));
        // Off the strip's scrollable its cursor levitates, and still says where it is.
        {
            use canvas::Program;
            let bounds =
                Rectangle::new(Point::ORIGIN, Size::new(controls.strip.width, STRIP_HEIGHT));
            let off = mouse::Cursor::Levitating(up(pan.y - 60.0));
            let action = controls
                .update(&mut state, &moved(pan.y - 60.0), bounds, off)
                .expect("a drag off the strip goes on");
            assert!(matches!(
                action.into_inner().0,
                Some(Message::Instrument(InstrumentMessage::Set(Control::Pan, value)))
                    if (value - 0.6).abs() < 1e-6
            ));
        }
        let shift = Event::Keyboard(keyboard::Event::ModifiersChanged(
            keyboard::Modifiers::SHIFT,
        ));
        let _ = strip_event(
            &controls,
            &mut state,
            shift,
            Point::new(pan.x, pan.y - 50.0),
        );
        // The channel still holds the center here, so fine goes on from it: 50 px is 0.05.
        assert!(matches!(
            strip_event(&controls, &mut state, moved(pan.y - 100.0), up(pan.y - 100.0)),
            Some(InstrumentMessage::Set(Control::Pan, value))
                if (value - 0.05).abs() < 1e-6
        ));
        assert_eq!(
            strip_event(&controls, &mut state, release.clone(), pan),
            instrument(InstrumentMessage::Released)
        );
        assert_eq!(
            strip_event(&controls, &mut state, press.clone(), pan),
            instrument(InstrumentMessage::Reset(Control::Pan)),
            "a double-click"
        );
        assert_eq!(strip_event(&controls, &mut state, release, pan), None);
        let hold = controls
            .strip
            .find(Part::Gate(Gate::Hold))
            .expect("placed")
            .hit
            .center();
        assert_eq!(
            strip_event(&controls, &mut state, press.clone(), hold),
            instrument(InstrumentMessage::Gate(Gate::Hold))
        );
        let reverse = controls
            .strip
            .find(Part::Flag(Flag::Reverse))
            .expect("placed")
            .hit
            .center();
        assert_eq!(
            strip_event(&controls, &mut state, press, reverse),
            instrument(InstrumentMessage::Toggle(Flag::Reverse))
        );
        // The attack's fader follows the cursor: 36 px up its 72 px travel is half way, 0.5 s.
        let attack = controls
            .strip
            .find(Part::Control(Control::Attack))
            .expect("placed");
        let at = attack.hit.center();
        let mut state = ControlsState::default();
        let press = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        let _ = strip_event(&controls, &mut state, press, at);
        let raised = Point::new(at.x, at.y - 36.0);
        let up = Event::Mouse(mouse::Event::CursorMoved { position: raised });
        assert!(matches!(
            strip_event(&controls, &mut state, up, raised),
            Some(InstrumentMessage::Set(Control::Attack, value))
                if (value - 0.5).abs() < 1e-6
        ));
    }

    // v2: the map fills the window across, its keys or pads stretched: 29 white keys take
    // 406 px of their own layout, so drawn 536 px across a press at x 535 is on the last white
    // key (four octaves above C3, key 84) and one at x 0 on the first (36); seven pads take
    // 400 px, so x 535 is the seventh pad and x 0 the first.
    #[test]
    fn the_map_fills_the_window_across() {
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(536.0, MAP_HEIGHT));
        let at = |x: f32| mouse::Cursor::Available(Point::new(x, 60.0));
        let keys = Map::Keys {
            low: 36,
            zones: vec![(0, 127)],
            roots: vec![60],
        };
        assert_eq!(span(&keys), 406.0);
        let key = |x| on_map(&keys, at(x), bounds).and_then(|point| key_at(&keys, point));
        assert_eq!(key(535.0), Some(84));
        assert_eq!(key(0.0), Some(36));
        let pads = Map::Pads(
            (0..7)
                .map(|index| Pad {
                    key: 36 + index,
                    name: String::new(),
                })
                .collect(),
        );
        assert_eq!(span(&pads), 400.0);
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(536.0, map_height(&pads)));
        let pad = |x| on_map(&pads, at(x), bounds).and_then(|point| key_at(&pads, point));
        assert_eq!(pad(535.0), None, "under the pads' row: below its 38 px");
        let at = |x: f32| mouse::Cursor::Available(Point::new(x, 20.0));
        let pad = |x| on_map(&pads, at(x), bounds).and_then(|point| key_at(&pads, point));
        assert_eq!(pad(535.0), Some(42));
        assert_eq!(pad(0.0), Some(36));
    }

    // The values as the window writes them, and the cutoff slider's three decades.
    #[test]
    fn values_read_in_their_units() {
        assert_eq!(Control::Attack.text(0.012), "12 ms");
        assert_eq!(Control::Release.text(1.5), "1.50 s");
        assert_eq!(Control::Semitones.text(-3.0), "-3 st");
        assert_eq!(Control::Cutoff.text(1200.0), "1.2 kHz");
        assert_eq!(Control::Cutoff.text(800.0), "800 Hz");
        assert_eq!(cutoff_of(0.0), 20.0);
        assert_eq!(cutoff_of(1.0), 20_000.0);
        assert!((cutoff_position(2000.0) - 2.0 / 3.0).abs() < 1e-6);
        let sound = with_value(Sound::default(), Control::Semitones, 30.0);
        assert_eq!(sound.semitones, 12, "kept in range");
        assert_eq!(default_value(Control::Velocity), 1.0);
        assert_eq!(default_value(Control::Cutoff), 20_000.0);
        assert!(!Control::Start.shown_for(ChannelKind::Instrument));
        assert!(!Control::Cents.shown_for(ChannelKind::Plugin));
        // The bend range: whole semitones 1..24, ±2 by default; a plugin bends by its own.
        assert_eq!(Control::BendRange.text(2.0), "\u{b1}2 st");
        assert_eq!(default_value(Control::BendRange), 2.0);
        assert_eq!(Control::BendRange.value_at(0.0), 1.0);
        assert_eq!(Control::BendRange.value_at(1.0), 24.0);
        let wide = with_value(Sound::default(), Control::BendRange, 30.0);
        assert_eq!(wide.bend_range, 24, "kept in range");
        assert!(Control::BendRange.shown_for(ChannelKind::Instrument));
        assert!(!Control::BendRange.shown_for(ChannelKind::Plugin));
    }
}
