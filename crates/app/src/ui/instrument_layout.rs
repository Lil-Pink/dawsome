//! The instrument window's controls, laid out like a mixer: groups left to right, each a
//! column of its own under a thin header naming it (SOUND, PITCH, ENVELOPE, FILTER, LEVEL),
//! holding knobs, switches (the mixer's lights), faders and a readout (the bend range, a value
//! dragged as a knob is) at fixed places. Pure geometry:
//! where each control lies, what is under a point, and where a drag leaves a control along its
//! sweep. A channel's kind decides which groups and controls it has, by the same rule the
//! window always used ([`Control::shown_for`]).

use dawsome_engine::project::{ChannelKind, Gate};
use iced::{Point, Rectangle, Size};

use crate::ui::instrument_window::{Control, Flag};

/// The strip's height, and its groups' header band.
pub const STRIP_HEIGHT: f32 = 176.0;
pub const HEADER_HEIGHT: f32 = 16.0;
/// The least and the most room between two groups: a wider window spreads them up to the most.
const LEAST_GAP: f32 = 6.0;
const MOST_GAP: f32 = 24.0;
/// A knob across, the cell it sits in (its value tip above it, its label under it), and the
/// pixels of drag from one stop to the other: the rack's knobs'.
pub const KNOB_SIZE: f32 = 28.0;
const KNOB_CELL: Size = Size::new(52.0, 58.0);
pub const KNOB_SWEEP: f32 = 200.0;
/// The tip's height: the mixer's.
pub const TIP_HEIGHT: f32 = crate::ui::mixer::TIP_HEIGHT;
/// A fader's track width, its cap (a small fader's and the tall one's), and how far its hit
/// area reaches beside and past the track.
pub const TRACK_WIDTH: f32 = 6.0;
pub const CAP_HEIGHT: f32 = 10.0;
const CAP_WIDTH: f32 = 24.0;
const TALL_CAP_WIDTH: f32 = 28.0;
const FADER_REACH: f32 = 20.0;
/// A small fader's track: its top and its travel. The tall one's.
const SMALL_TOP: f32 = 28.0;
const SMALL_TRAVEL: f32 = 72.0;
const TALL_TOP: f32 = 26.0;
const TALL_TRAVEL: f32 = 104.0;
/// A switch's light (the mixer's), and the width of its hit area with its label.
pub const LIGHT_RADIUS: f32 = 5.0;
const LIGHT_WIDTH: f32 = 68.0;
/// How much slower a control moves with Shift held: the mixer's.
pub const FINE: f32 = 0.1;
/// Pixels a detented knob holds its center over.
pub const DETENT: f32 = 8.0;
/// The output chooser: its label's width and its row's top and height.
pub const CHOOSER_LABEL: f32 = 24.0;
const CHOOSER_TOP: f32 = 150.0;
pub const CHOOSER_HEIGHT: f32 = 20.0;
/// The PITCH group's width, and its bend range readout's row: the output chooser's.
const PITCH_WIDTH: f32 = 70.0;
const READOUT_INSET: f32 = 3.0;

/// A group of the strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Sound,
    Pitch,
    Envelope,
    Filter,
    Level,
}

impl Group {
    /// The name its header shows.
    pub fn title(self) -> &'static str {
        match self {
            Group::Sound => "SOUND",
            Group::Pitch => "PITCH",
            Group::Envelope => "ENVELOPE",
            Group::Filter => "FILTER",
            Group::Level => "LEVEL",
        }
    }
}

/// A control of the strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// One of the gate switch's two positions.
    Gate(Gate),
    Flag(Flag),
    Control(Control),
    /// The output insert's chooser (a widget over the strip, not drawn by it).
    Insert,
}

/// How a control is drawn, where.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Look {
    Knob {
        center: Point,
    },
    /// A vertical fader on `track`; the tall one is the volume's.
    Fader {
        track: Rectangle,
        tall: bool,
    },
    /// A switch's light, its label right of it.
    Light {
        center: Point,
    },
    /// The output chooser's room.
    Chooser,
    /// A value written with its label in one line on a well, dragged as a knob is.
    Readout,
}

/// A control in its place: what it is, how it is drawn, where a press finds it, its label
/// and where that is centered (a light's label starts there).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    pub part: Part,
    pub look: Look,
    pub hit: Rectangle,
    pub label: &'static str,
    pub label_at: Point,
}

impl Placed {
    /// How many pixels of drag sweep it from one end to the other: a knob's turn, a fader's
    /// travel (its cap follows the cursor).
    pub fn sweep(&self) -> f32 {
        match self.look {
            Look::Fader { track, .. } => track.height,
            Look::Knob { .. } | Look::Light { .. } | Look::Chooser | Look::Readout => KNOB_SWEEP,
        }
    }

    /// Where its value tip shows: over a knob, its cell's top; over a fader, above its cap at
    /// `position` (0..1 along its travel), no higher than the group's header.
    pub fn tip(&self, position: f32) -> Rectangle {
        let width = KNOB_CELL.width;
        match self.look {
            Look::Knob { center } => Rectangle::new(
                Point::new(
                    center.x - width / 2.0,
                    center.y - KNOB_SIZE / 2.0 - TIP_HEIGHT - 2.0,
                ),
                Size::new(width, TIP_HEIGHT),
            ),
            Look::Fader { track, tall } => {
                let cap = cap_rect(track, position, tall);
                Rectangle::new(
                    Point::new(
                        track.center_x() - width / 2.0,
                        (cap.y - TIP_HEIGHT - 2.0).max(HEADER_HEIGHT),
                    ),
                    Size::new(width, TIP_HEIGHT),
                )
            }
            Look::Light { .. } | Look::Chooser | Look::Readout => self.hit,
        }
    }
}

/// A group in its place, its controls with it.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub group: Group,
    pub rect: Rectangle,
    pub parts: Vec<Placed>,
}

/// The strip for one channel: its groups in their places and its width.
#[derive(Debug, Clone, PartialEq)]
pub struct Strip {
    pub groups: Vec<Placement>,
    pub width: f32,
}

impl Strip {
    /// The strip of a channel of `kind`, its groups spread over `width` where it is wider than
    /// the least they need (never narrower than that: the window scrolls it).
    pub fn of(kind: ChannelKind, width: f32) -> Self {
        let groups = groups_of(kind);
        let least = least_width(kind);
        let gap = match groups.len() {
            0 | 1 => LEAST_GAP,
            many => ((width - (least - LEAST_GAP * (many - 1) as f32)) / (many - 1) as f32)
                .clamp(LEAST_GAP, MOST_GAP),
        };
        let mut placed = Vec::with_capacity(groups.len());
        let mut x = 0.0;
        for group in groups {
            let (group_width, parts) = contents(group, kind, x);
            placed.push(Placement {
                group,
                rect: Rectangle::new(Point::new(x, 0.0), Size::new(group_width, STRIP_HEIGHT)),
                parts,
            });
            x += group_width + gap;
        }
        Self {
            groups: placed,
            width: width.max(least),
        }
    }

    /// Every control, group by group, left to right.
    pub fn parts(&self) -> impl Iterator<Item = &Placed> {
        self.groups.iter().flat_map(|group| &group.parts)
    }

    /// The control a press at `at` finds, the output chooser left out (its widget takes its
    /// own presses).
    pub fn hit(&self, at: Point) -> Option<&Placed> {
        self.parts()
            .filter(|placed| placed.part != Part::Insert)
            .find(|placed| placed.hit.contains(at))
    }

    /// Where `part` lies, when the strip has it.
    pub fn find(&self, part: Part) -> Option<&Placed> {
        self.parts().find(|placed| placed.part == part)
    }
}

/// The groups a channel of `kind` has, left to right: a plugin's, an audio channel's or an
/// input channel's sound is its own, so it has its level alone.
pub fn groups_of(kind: ChannelKind) -> Vec<Group> {
    match kind {
        ChannelKind::Sampler | ChannelKind::Instrument => vec![
            Group::Sound,
            Group::Pitch,
            Group::Envelope,
            Group::Filter,
            Group::Level,
        ],
        ChannelKind::Plugin | ChannelKind::Audio | ChannelKind::Input => vec![Group::Level],
    }
}

/// The least width the strip of a channel of `kind` takes: its groups at the least gap.
pub fn least_width(kind: ChannelKind) -> f32 {
    let groups = groups_of(kind);
    let widths: f32 = groups
        .iter()
        .map(|&group| contents(group, kind, 0.0).0)
        .sum();
    widths + LEAST_GAP * groups.len().saturating_sub(1) as f32
}

/// `group`'s width and its controls for a channel of `kind`, its left edge at `x`.
fn contents(group: Group, kind: ChannelKind, x: f32) -> (f32, Vec<Placed>) {
    let shown = |control: Control| control.shown_for(kind);
    match group {
        Group::Sound => {
            let mut parts = vec![
                light(Part::Gate(Gate::OneShot), x + 12.0, 34.0, "One-shot"),
                light(Part::Gate(Gate::Hold), x + 12.0, 52.0, "Hold"),
            ];
            // Reverse and the start are a sampler's: a set's zones keep their own.
            if !shown(Control::Start) {
                return (80.0, parts);
            }
            parts.push(light(Part::Flag(Flag::Reverse), x + 12.0, 84.0, "Reverse"));
            parts.push(fader(Control::Start, x + 98.0, false));
            (122.0, parts)
        }
        Group::Pitch => {
            let middle = x + PITCH_WIDTH / 2.0;
            let bend = Rectangle::new(
                Point::new(x + READOUT_INSET, CHOOSER_TOP),
                Size::new(PITCH_WIDTH - 2.0 * READOUT_INSET, CHOOSER_HEIGHT),
            );
            (
                PITCH_WIDTH,
                vec![
                    knob(Control::Semitones, middle, 18.0),
                    knob(Control::Cents, middle, 82.0),
                    Placed {
                        part: Part::Control(Control::BendRange),
                        look: Look::Readout,
                        hit: bend,
                        label: Control::BendRange.short(),
                        label_at: bend.center(),
                    },
                ],
            )
        }
        Group::Envelope => (
            96.0,
            vec![
                fader(Control::Attack, x + 24.0, false),
                fader(Control::Release, x + 72.0, false),
            ],
        ),
        Group::Filter => (
            112.0,
            vec![
                light(Part::Flag(Flag::Filter), x + 12.0, 34.0, "On"),
                knob(Control::Cutoff, x + 30.0, 50.0),
                knob(Control::Resonance, x + 84.0, 50.0),
            ],
        ),
        Group::Level => {
            let mut parts = Vec::new();
            // The pan in the second cell under the velocity, or in the first without it.
            let pan_top = if shown(Control::Velocity) {
                parts.push(knob(Control::Velocity, x + 30.0, 18.0));
                82.0
            } else {
                18.0
            };
            parts.push(knob(Control::Pan, x + 30.0, pan_top));
            parts.push(fader(Control::Volume, x + 86.0, true));
            let chooser = Rectangle::new(
                Point::new(x + 4.0 + CHOOSER_LABEL, CHOOSER_TOP),
                Size::new(104.0 - CHOOSER_LABEL, CHOOSER_HEIGHT),
            );
            parts.push(Placed {
                part: Part::Insert,
                look: Look::Chooser,
                hit: chooser,
                label: "Out",
                label_at: Point::new(x + 4.0 + CHOOSER_LABEL / 2.0, chooser.center_y()),
            });
            (112.0, parts)
        }
    }
}

/// `control` as a knob in the cell centered across at `x` from `top`.
fn knob(control: Control, x: f32, top: f32) -> Placed {
    let center = Point::new(x, top + TIP_HEIGHT + 2.0 + KNOB_SIZE / 2.0);
    Placed {
        part: Part::Control(control),
        look: Look::Knob { center },
        hit: Rectangle::new(
            Point::new(x - KNOB_CELL.width / 2.0, top + TIP_HEIGHT),
            Size::new(KNOB_CELL.width, KNOB_CELL.height - TIP_HEIGHT),
        ),
        label: control.short(),
        label_at: Point::new(x, top + KNOB_CELL.height - 4.0),
    }
}

/// `control` as a fader centered across at `x`: a small one with its readout and label under
/// it, or the tall one with its label.
fn fader(control: Control, x: f32, tall: bool) -> Placed {
    let (top, travel) = if tall {
        (TALL_TOP, TALL_TRAVEL)
    } else {
        (SMALL_TOP, SMALL_TRAVEL)
    };
    let track = Rectangle::new(
        Point::new(x - TRACK_WIDTH / 2.0, top),
        Size::new(TRACK_WIDTH, travel),
    );
    let under = if tall { 12.0 } else { 26.0 };
    Placed {
        part: Part::Control(control),
        look: Look::Fader { track, tall },
        hit: Rectangle::new(
            Point::new(x - FADER_REACH, top - CAP_HEIGHT / 2.0 - 1.0),
            Size::new(2.0 * FADER_REACH, travel + CAP_HEIGHT + 2.0),
        ),
        label: control.short(),
        label_at: Point::new(x, top + travel + under),
    }
}

/// Where a small fader on `track` writes its value: under its track, over its label.
pub fn readout_at(track: Rectangle) -> Point {
    Point::new(track.center_x(), track.y + track.height + 12.0)
}

/// A switch's light at (`x`, `y`) with `label` right of it.
fn light(part: Part, x: f32, y: f32, label: &'static str) -> Placed {
    Placed {
        part,
        look: Look::Light {
            center: Point::new(x, y),
        },
        hit: Rectangle::new(Point::new(x - 8.0, y - 8.0), Size::new(LIGHT_WIDTH, 16.0)),
        label,
        label_at: Point::new(x + LIGHT_RADIUS + 5.0, y),
    }
}

/// A fader's cap on `track` at `position` (0 at its foot, 1 at its top).
pub fn cap_rect(track: Rectangle, position: f32, tall: bool) -> Rectangle {
    let width = if tall { TALL_CAP_WIDTH } else { CAP_WIDTH };
    let center = track.y + track.height * (1.0 - position.clamp(0.0, 1.0));
    Rectangle::new(
        Point::new(track.center_x() - width / 2.0, center - CAP_HEIGHT / 2.0),
        Size::new(width, CAP_HEIGHT),
    )
}

/// Where a drag `rise` pixels up (down is negative) from `start` (0..1 along the sweep) leaves
/// a control swept by `pixels`, a tenth as fast when `fine` (Shift); a `detent` control holds
/// its center over [`DETENT`] pixels of the drag first.
pub fn dragged(start: f32, rise: f32, pixels: f32, fine: bool, detent: bool) -> f32 {
    let pixels = if fine { pixels / FINE } else { pixels };
    if !detent {
        return (start + rise / pixels).clamp(0.0, 1.0);
    }
    // Along a track `pixels + DETENT` long whose center position holds over DETENT of it.
    let half = pixels / 2.0;
    let along = match start {
        start if start < 0.5 => start * pixels,
        start if start > 0.5 => start * pixels + DETENT,
        _ => half + DETENT / 2.0,
    };
    let at = (along + rise).clamp(0.0, pixels + DETENT);
    if at < half {
        at / pixels
    } else if at > half + DETENT {
        (at - DETENT) / pixels
    } else {
        0.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(kind: ChannelKind, group: Group) -> Vec<Part> {
        let strip = Strip::of(kind, 600.0);
        strip
            .groups
            .iter()
            .find(|placed| placed.group == group)
            .map(|placed| placed.parts.iter().map(|part| part.part).collect())
            .unwrap_or_default()
    }

    // v2 spec: SOUND One-shot/Hold, Reverse and Start (samplers); PITCH Tune and Fine, and the
    // bend range (pedal, bend and wheel spec); ENVELOPE Attack and Release; FILTER On, Cutoff,
    // Resonance; LEVEL Velocity, Pan, Volume and the output insert, left to right. A set has no
    // Reverse or Start; a plugin channel its level alone, without the velocity.
    #[test]
    fn each_kind_has_the_groups_and_controls_the_spec_names() {
        use Control as C;
        let sampler = ChannelKind::Sampler;
        let groups: Vec<Group> = Strip::of(sampler, 600.0)
            .groups
            .iter()
            .map(|placed| placed.group)
            .collect();
        assert_eq!(
            groups,
            [
                Group::Sound,
                Group::Pitch,
                Group::Envelope,
                Group::Filter,
                Group::Level
            ]
        );
        assert_eq!(
            parts(sampler, Group::Sound),
            [
                Part::Gate(Gate::OneShot),
                Part::Gate(Gate::Hold),
                Part::Flag(Flag::Reverse),
                Part::Control(C::Start)
            ]
        );
        assert_eq!(
            parts(sampler, Group::Pitch),
            [
                Part::Control(C::Semitones),
                Part::Control(C::Cents),
                Part::Control(C::BendRange)
            ]
        );
        assert_eq!(
            parts(sampler, Group::Envelope),
            [Part::Control(C::Attack), Part::Control(C::Release)]
        );
        assert_eq!(
            parts(sampler, Group::Filter),
            [
                Part::Flag(Flag::Filter),
                Part::Control(C::Cutoff),
                Part::Control(C::Resonance)
            ]
        );
        assert_eq!(
            parts(sampler, Group::Level),
            [
                Part::Control(C::Velocity),
                Part::Control(C::Pan),
                Part::Control(C::Volume),
                Part::Insert
            ]
        );
        assert_eq!(
            parts(ChannelKind::Instrument, Group::Sound),
            [Part::Gate(Gate::OneShot), Part::Gate(Gate::Hold)]
        );
        let plugin = Strip::of(ChannelKind::Plugin, 600.0);
        assert_eq!(plugin.groups.len(), 1);
        assert_eq!(
            parts(ChannelKind::Plugin, Group::Level),
            [
                Part::Control(C::Pan),
                Part::Control(C::Volume),
                Part::Insert
            ]
        );
        // Every control a kind shows is on its strip, once.
        for kind in [
            ChannelKind::Sampler,
            ChannelKind::Instrument,
            ChannelKind::Plugin,
        ] {
            let strip = Strip::of(kind, 600.0);
            for control in Control::ALL {
                let found = strip
                    .parts()
                    .filter(|placed| placed.part == Part::Control(control))
                    .count();
                assert_eq!(
                    found,
                    usize::from(control.shown_for(kind)),
                    "{kind:?} {control:?}"
                );
            }
        }
    }

    // Asked for in use, 2026-10-26 ("I don't want UI that is saying 'Cut itself'"): no light of any
    // kind's window is the switch (it is the rack channel menu's "One note at a time" now). A set's
    // SOUND group is its gate pair alone, and a sampler's Reverse is back on its row under them, at
    // y 84 under One-shot.
    #[test]
    fn the_sound_group_has_no_cut_itself_light() {
        let center = |strip: &Strip, part| match strip.find(part).expect("placed").look {
            Look::Light { center } => center,
            look => panic!("{part:?} is a {look:?}"),
        };
        for kind in [
            ChannelKind::Sampler,
            ChannelKind::Instrument,
            ChannelKind::Plugin,
        ] {
            let labels: Vec<&str> = Strip::of(kind, 600.0)
                .parts()
                .map(|placed| placed.label)
                .collect();
            assert!(
                labels
                    .iter()
                    .all(|label| *label != "Cut itself" && *label != "One note at a time"),
                "{kind:?}: {labels:?}"
            );
        }
        let sampler = Strip::of(ChannelKind::Sampler, 600.0);
        let one_shot = center(&sampler, Part::Gate(Gate::OneShot));
        assert_eq!(
            center(&sampler, Part::Flag(Flag::Reverse)),
            Point::new(one_shot.x, 84.0)
        );
        let set = Strip::of(ChannelKind::Instrument, 600.0);
        let sound = set
            .groups
            .iter()
            .find(|placed| placed.group == Group::Sound)
            .expect("a set has a SOUND group");
        assert_eq!(sound.parts.len(), 2, "One-shot and Hold");
    }

    // The knobs are the pitch's, the filter's, the velocity and the pan; the start, the attack
    // and the release small faders; the volume the tall one.
    #[test]
    fn knobs_small_faders_and_the_tall_one() {
        let strip = Strip::of(ChannelKind::Sampler, 600.0);
        let look = |control| strip.find(Part::Control(control)).expect("placed").look;
        for control in [
            Control::Semitones,
            Control::Cents,
            Control::Cutoff,
            Control::Resonance,
            Control::Velocity,
            Control::Pan,
        ] {
            assert!(matches!(look(control), Look::Knob { .. }), "{control:?}");
        }
        for control in [Control::Start, Control::Attack, Control::Release] {
            assert!(
                matches!(look(control), Look::Fader { tall: false, .. }),
                "{control:?}"
            );
        }
        assert!(matches!(
            look(Control::Volume),
            Look::Fader { tall: true, .. }
        ));
        assert_eq!(look(Control::BendRange), Look::Readout);
    }

    // A press at each control's middle finds it; no two controls' areas overlap; every control
    // lies inside its group, under its header, and inside the strip; between them nothing is
    // found.
    #[test]
    fn a_press_finds_the_control_under_it_and_nothing_between() {
        for kind in [
            ChannelKind::Sampler,
            ChannelKind::Instrument,
            ChannelKind::Plugin,
        ] {
            let strip = Strip::of(kind, 600.0);
            for group in &strip.groups {
                for placed in &group.parts {
                    let hit = placed.hit;
                    assert!(
                        hit.x >= group.rect.x
                            && hit.x + hit.width <= group.rect.x + group.rect.width
                            && hit.y >= HEADER_HEIGHT
                            && hit.y + hit.height <= STRIP_HEIGHT,
                        "{kind:?} {:?} {hit:?} in {:?}",
                        placed.part,
                        group.rect
                    );
                    if placed.part != Part::Insert {
                        assert_eq!(
                            strip.hit(hit.center()).map(|found| found.part),
                            Some(placed.part)
                        );
                    }
                    for other in strip.parts().filter(|other| other.part != placed.part) {
                        assert!(
                            hit.intersection(&other.hit).is_none(),
                            "{:?} and {:?} overlap",
                            placed.part,
                            other.part
                        );
                    }
                }
            }
            assert_eq!(strip.hit(Point::new(1.0, 1.0)), None, "the header");
            assert_eq!(strip.hit(Point::new(1.0, STRIP_HEIGHT - 2.0)), None);
        }
        let strip = Strip::of(ChannelKind::Sampler, 600.0);
        let chooser = strip.find(Part::Insert).expect("placed").hit;
        assert_eq!(
            strip.hit(chooser.center()),
            None,
            "the chooser's widget takes it"
        );
    }

    // The sampler's strip fits the window's first size (560 less its 12 px each side); a wider
    // window spreads the groups up to 24 px apart, a narrower one keeps them 6 px apart and
    // the window scrolls the strip. PITCH is 70 px since the bend range's readout (60 before):
    // the strip is then exactly the window's first size.
    #[test]
    fn the_groups_spread_over_a_wider_window_and_keep_their_least_width() {
        let least = least_width(ChannelKind::Sampler);
        assert_eq!(least, 122.0 + 70.0 + 96.0 + 112.0 + 112.0 + 4.0 * 6.0);
        assert!(least <= 560.0 - 24.0);
        let lefts = |width| -> Vec<f32> {
            Strip::of(ChannelKind::Sampler, width)
                .groups
                .iter()
                .map(|group| group.rect.x)
                .collect()
        };
        assert_eq!(lefts(least), [0.0, 128.0, 204.0, 306.0, 424.0]);
        let wide = Strip::of(ChannelKind::Sampler, least + 40.0);
        assert_eq!(wide.width, least + 40.0);
        assert_eq!(
            lefts(least + 40.0),
            [0.0, 138.0, 224.0, 336.0, 464.0],
            "16 px apart"
        );
        assert_eq!(lefts(2000.0)[1], 122.0 + 24.0, "at most 24 px apart");
        let narrow = Strip::of(ChannelKind::Sampler, 396.0);
        assert_eq!(narrow.width, least, "it scrolls");
        assert_eq!(lefts(396.0), lefts(least));
        assert_eq!(least_width(ChannelKind::Plugin), 112.0);
    }

    // Known answers, by hand (python3 -I): a knob sweeps its range over 200 px, so the pan
    // from the center 50 px up is 0.75 of its sweep (50% R), and with Shift 0.525; a fader's
    // cap follows the cursor over its travel (72 px: 36 px up from the foot is half way); the
    // tune knob holds its center over 8 px: 4 px up or down stays at 0.5, 5 px is 0.505 and
    // 9 px 0.525 (the first that rounds to +1 semitone; 8 px is 0.52, still 0).
    #[test]
    fn a_drag_moves_a_control_along_its_sweep() {
        let close = |a: f32, b: f32| (a - b).abs() < 1e-6;
        assert!(close(dragged(0.5, 50.0, KNOB_SWEEP, false, false), 0.75));
        assert!(close(dragged(0.5, 50.0, KNOB_SWEEP, true, false), 0.525));
        assert!(close(dragged(0.0, 36.0, SMALL_TRAVEL, false, false), 0.5));
        assert_eq!(dragged(0.9, 100.0, KNOB_SWEEP, false, false), 1.0, "a stop");
        assert_eq!(dragged(0.1, -100.0, KNOB_SWEEP, false, false), 0.0);
        for rise in [3.0, 4.0, -4.0] {
            assert_eq!(dragged(0.5, rise, KNOB_SWEEP, false, true), 0.5, "{rise}");
        }
        assert!(close(dragged(0.5, 5.0, KNOB_SWEEP, false, true), 0.505));
        assert!(close(dragged(0.5, -5.0, KNOB_SWEEP, false, true), 0.495));
        assert!(close(dragged(0.5, 8.0, KNOB_SWEEP, false, true), 0.52));
        assert!(close(dragged(0.5, 9.0, KNOB_SWEEP, false, true), 0.525));
        assert!(close(dragged(0.5, -9.0, KNOB_SWEEP, false, true), 0.475));
        // Past the center the detent is passed on the way back too.
        assert!(close(dragged(0.6, -24.0, KNOB_SWEEP, false, true), 0.5));
        assert!(close(dragged(0.6, -29.0, KNOB_SWEEP, false, true), 0.495));
        assert_eq!(dragged(0.0, 1000.0, KNOB_SWEEP, false, true), 1.0);
    }

    // A fader's cap sits at its position along the track, its foot 0 and its top 1; its tip
    // sits above the cap, never over the group's header; a knob's tip over the knob.
    #[test]
    fn caps_and_tips_sit_where_their_values_are() {
        let strip = Strip::of(ChannelKind::Sampler, 600.0);
        let attack = strip.find(Part::Control(Control::Attack)).expect("placed");
        let Look::Fader { track, tall } = attack.look else {
            panic!("a fader")
        };
        assert_eq!(
            cap_rect(track, 0.0, tall).center_y(),
            track.y + track.height
        );
        assert_eq!(cap_rect(track, 1.0, tall).center_y(), track.y);
        assert_eq!(cap_rect(track, 0.5, tall).center_y(), track.y + 36.0);
        assert_eq!(attack.sweep(), 72.0);
        assert_eq!(attack.tip(1.0).y, HEADER_HEIGHT, "not over the header");
        assert!(attack.tip(0.0).y + TIP_HEIGHT <= cap_rect(track, 0.0, tall).y);
        let tune = strip
            .find(Part::Control(Control::Semitones))
            .expect("placed");
        let Look::Knob { center } = tune.look else {
            panic!("a knob")
        };
        assert_eq!(tune.sweep(), KNOB_SWEEP);
        let tip = tune.tip(0.5);
        assert!(tip.y >= HEADER_HEIGHT && tip.y + TIP_HEIGHT <= center.y - KNOB_SIZE / 2.0);
    }
}
