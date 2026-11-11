//! Every genre's table, read once from genres.json beside this file: the one place a genre's
//! numbers are tuned, which tests/fixtures/gen_p36_expected.py reads too, so the known answers
//! follow a change without the number being written twice. The file is built into the program
//! (`include_str!`) and parsed the first time a genre is asked for; every number is checked as
//! it is read, so a mistyped field or a value out of range fails the tests with its place named
//! rather than writing a Jam no one meant.
//!
//! Readings the numbers were set by where the design's table is loose (the generator's
//! docstring states the same):
//! - a velocity without a stated value takes the voice's default base (kick 0.90, snare 0.85,
//!   clap 0.75, closed hat 0.60 on the beats and 0.45 between, open hat 0.50, perc 0.55, chords
//!   0.60, bass 0.80), chance hits too; a stated one is a base, jittered, unless fixed;
//! - ghosts are fixed at 0.35, and only lo-fi has the last-bar snare fill;
//! - boom bap's closed hat is lo-fi's whole closed hat; no other genre drops hats;
//! - trap's roll draws once after the bar's 16 hats: four hats half a step after 12 to 15;
//! - drill's triplet feel draws first: the even hats but 2 and 10, plus 3 and 11;
//! - house's chord on step 0 draws before the stabs (step order);
//! - afrobeats' bass on step 7 lasts 3 steps and its fifth on 14 lasts 2.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

use super::{
    Count, Genre, GenreTable, Hit, MelodyTable, Part, PartKind, Pitch, Variant, Velocity, When,
    STEPS_PER_BAR,
};
use crate::time::{MilliBpm, SWING_RANGE};

/// The tables, as the program is built with them.
const FILE: &str = include_str!("genres.json");

/// How far below its base a velocity may fall when a part does not say.
const DEFAULT_JITTER: u8 = 16;

/// The tables in [`Genre::ALL`]'s order.
pub(super) fn tables() -> &'static [GenreTable] {
    static TABLES: OnceLock<Vec<GenreTable>> = OnceLock::new();
    TABLES.get_or_init(|| {
        // The tests parse this same text, so a build that passes them cannot fail here.
        parse(FILE).unwrap_or_else(|problem| panic!("genres.json: {problem}"))
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TablesFile {
    /// What the file is for and how to tune it, for whoever opens it.
    #[serde(rename = "_about")]
    _about: String,
    genres: BTreeMap<String, GenreFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GenreFile {
    name: String,
    tempo_bpm: u32,
    tempo_range: [u32; 2],
    swing_tenths: u16,
    sevenths: bool,
    minor_progressions: Vec<[u8; 4]>,
    major_progressions: Vec<[u8; 4]>,
    kick: Vec<VariantFile>,
    snare: Vec<VariantFile>,
    clap: Vec<VariantFile>,
    closed_hat: Vec<VariantFile>,
    open_hat: Vec<VariantFile>,
    perc: Vec<VariantFile>,
    chords: Vec<VariantFile>,
    bass: Vec<VariantFile>,
    melody: Vec<MelodyTable>,
}

/// One way a voice, the chords or the bass may go: its note and its parts.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VariantFile {
    #[serde(rename = "_note")]
    note: String,
    parts: Vec<PartFile>,
}

/// A part as the file writes it: `hits`, `either` with `then` and `otherwise`, or `last_bar`,
/// and its `jitter` when not the default.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PartFile {
    hits: Option<Vec<HitFile>>,
    either: Option<u8>,
    then: Option<Vec<HitFile>>,
    otherwise: Option<Vec<HitFile>>,
    last_bar: Option<Vec<HitFile>>,
    jitter: Option<u8>,
}

/// A hit as the file writes it: its step, then what it leaves out of the defaults.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HitFile {
    step: u8,
    #[serde(default)]
    half: bool,
    #[serde(default)]
    length: u8,
    #[serde(default)]
    pitch: Pitch,
    chance: Option<u8>,
    unless: Option<u8>,
    base: Option<u8>,
    exact: Option<u8>,
}

/// The tables of `text`, every genre of [`Genre::ALL`] and no other, each number checked;
/// what is wrong, and where, when they are not.
pub(super) fn parse(text: &str) -> Result<Vec<GenreTable>, String> {
    let file: TablesFile = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let mut genres = file.genres;
    let tables = Genre::ALL
        .into_iter()
        .map(|genre| {
            let read = genres
                .remove(genre.id())
                .ok_or_else(|| format!("no genre \"{}\"", genre.id()))?;
            genre_table(read).map_err(|problem| format!("{}: {problem}", genre.id()))
        })
        .collect::<Result<Vec<_>, String>>()?;
    if let Some(extra) = genres.keys().next() {
        return Err(format!("\"{extra}\" is not a genre this build has"));
    }
    Ok(tables)
}

fn percent(value: u8, what: &str) -> Result<u8, String> {
    if value <= 100 {
        Ok(value)
    } else {
        Err(format!("{what} {value} is past 100"))
    }
}

fn genre_table(read: GenreFile) -> Result<GenreTable, String> {
    let [low, high] = read.tempo_range;
    if low > high || !(low..=high).contains(&read.tempo_bpm) {
        return Err(format!(
            "tempo {} must lie in a tempo range low to high, not {low}..{high}",
            read.tempo_bpm
        ));
    }
    if !SWING_RANGE.contains(&read.swing_tenths) {
        return Err(format!(
            "swing_tenths {} is outside 500..=750",
            read.swing_tenths
        ));
    }
    for (scale, progressions) in [
        ("minor", &read.minor_progressions),
        ("major", &read.major_progressions),
    ] {
        if progressions.is_empty() || progressions.iter().flatten().any(|d| !(1..=7).contains(d)) {
            return Err(format!(
                "{scale}_progressions must be one or more lists of degrees 1 to 7"
            ));
        }
    }
    let variants = |name: &str, read: Vec<VariantFile>| -> Result<Vec<Variant>, String> {
        if read.is_empty() {
            return Err(format!(
                "{name} has no variant: give it one, with no parts to play nothing"
            ));
        }
        read.into_iter()
            .enumerate()
            .map(|(v, variant)| {
                let parts = variant
                    .parts
                    .into_iter()
                    .enumerate()
                    .map(|(i, part)| {
                        part_of(part).map_err(|problem| {
                            format!("{name} variant {} part {}: {problem}", v + 1, i + 1)
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                Ok(Variant {
                    note: variant.note,
                    parts,
                })
            })
            .collect()
    };
    if read.melody.is_empty() {
        return Err("melody has no variant".to_owned());
    }
    for (v, melody) in read.melody.iter().enumerate() {
        check_melody(melody).map_err(|problem| format!("melody variant {}: {problem}", v + 1))?;
    }
    Ok(GenreTable {
        name: read.name,
        tempo: MilliBpm(read.tempo_bpm * 1000),
        tempo_range: low..=high,
        swing_tenths: read.swing_tenths,
        sevenths: read.sevenths,
        minor_progressions: read.minor_progressions,
        major_progressions: read.major_progressions,
        kick: variants("kick", read.kick)?,
        snare: variants("snare", read.snare)?,
        clap: variants("clap", read.clap)?,
        closed_hat: variants("closed_hat", read.closed_hat)?,
        open_hat: variants("open_hat", read.open_hat)?,
        perc: variants("perc", read.perc)?,
        chords: variants("chords", read.chords)?,
        bass: variants("bass", read.bass)?,
        melody: read.melody,
    })
}

/// What is wrong with a melody table, if anything.
fn check_melody(melody: &MelodyTable) -> Result<(), String> {
    if melody.starts.is_empty() || melody.starts.iter().any(|&start| start >= STEPS_PER_BAR) {
        return Err("starts must be one or more steps 0 to 15".to_owned());
    }
    if let Count::Drawn {
        spread, rest_light, ..
    } = &melody.count
    {
        if *spread == 0 {
            return Err("spread must be at least 1".to_owned());
        }
        if let Some(rest) = rest_light {
            percent(rest.chance, "rest_light chance")?;
        }
    }
    if let Some(late) = melody.late {
        percent(late, "late")?;
    }
    // Why an octave at least: a note walks out of the band by a pool step and folds back by
    // octaves, which lands only when the band holds a whole octave.
    if melody.high > 127 || u32::from(melody.low) + 11 > u32::from(melody.high) {
        return Err(format!(
            "band {}..{} must be an octave or more wide, at most 127",
            melody.low, melody.high
        ));
    }
    if !(1..=STEPS_PER_BAR).contains(&melody.max_length) {
        return Err(format!(
            "max_length {} is outside 1..=16",
            melody.max_length
        ));
    }
    Ok(())
}

fn part_of(read: PartFile) -> Result<Part, String> {
    let hits = |read: Vec<HitFile>| -> Result<Vec<Hit>, String> {
        read.into_iter()
            .enumerate()
            .map(|(i, hit)| hit_of(hit).map_err(|problem| format!("hit {}: {problem}", i + 1)))
            .collect()
    };
    let kind = match read {
        PartFile {
            hits: Some(listed),
            either: None,
            then: None,
            otherwise: None,
            last_bar: None,
            ..
        } => PartKind::Hits(hits(listed)?),
        PartFile {
            hits: None,
            either: Some(chance),
            then: Some(then),
            otherwise: Some(otherwise),
            last_bar: None,
            ..
        } => PartKind::Either {
            chance: percent(chance, "either")?,
            then: hits(then)?,
            otherwise: hits(otherwise)?,
        },
        PartFile {
            hits: None,
            either: None,
            then: None,
            otherwise: None,
            last_bar: Some(listed),
            ..
        } => PartKind::LastBar(hits(listed)?),
        _ => {
            return Err(
                "a part is one of hits, either (with then and otherwise) or last_bar".to_owned(),
            )
        }
    };
    let jitter = read.jitter.unwrap_or(DEFAULT_JITTER);
    // Why not 0: the draw is `below(jitter)`, and nothing is below 0.
    if !(1..=100).contains(&jitter) {
        return Err(format!("jitter {jitter} is outside 1..=100"));
    }
    Ok(Part { kind, jitter })
}

fn hit_of(read: HitFile) -> Result<Hit, String> {
    if read.step >= STEPS_PER_BAR {
        return Err(format!("step {} is past 15", read.step));
    }
    if u32::from(read.step) + u32::from(read.length) > u32::from(STEPS_PER_BAR) {
        return Err(format!(
            "step {} and length {} run past the bar",
            read.step, read.length
        ));
    }
    let when = match (read.chance, read.unless) {
        (None, None) => When::Always,
        (Some(p), None) => When::Chance(percent(p, "chance")?),
        (None, Some(p)) => When::Unless(percent(p, "unless")?),
        (Some(_), Some(_)) => return Err("a hit has chance or unless, not both".to_owned()),
    };
    let velocity = match (read.base, read.exact) {
        (Some(v), None) => Velocity::Jitter(percent(v, "base")?),
        (None, Some(v)) => Velocity::Exact(percent(v, "exact")?),
        _ => return Err("a hit has base or exact, one of them".to_owned()),
    };
    Ok(Hit {
        step: read.step,
        half: read.half,
        length: read.length,
        pitch: read.pitch,
        when,
        velocity,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every hit of every variant of a table, with where it is.
    fn hits(table: &GenreTable) -> Vec<(&'static str, Hit)> {
        let parts: [(&'static str, &Vec<Variant>); 8] = [
            ("kick", &table.kick),
            ("snare", &table.snare),
            ("clap", &table.clap),
            ("closed_hat", &table.closed_hat),
            ("open_hat", &table.open_hat),
            ("perc", &table.perc),
            ("chords", &table.chords),
            ("bass", &table.bass),
        ];
        parts
            .into_iter()
            .flat_map(|(name, variants)| {
                variants
                    .iter()
                    .flat_map(|variant| &variant.parts)
                    .map(move |part| (name, part))
            })
            .flat_map(|(name, part)| {
                let listed: Vec<Hit> = match &part.kind {
                    PartKind::Hits(hits) | PartKind::LastBar(hits) => hits.clone(),
                    PartKind::Either {
                        then, otherwise, ..
                    } => then.iter().chain(otherwise).copied().collect(),
                };
                listed.into_iter().map(move |hit| (name, hit))
            })
            .collect()
    }

    // The file as built: every genre there, in the menu's order, each with a tempo range that
    // holds its tempo, a swing the knob can set, every velocity 0 to 100 and every step 0 to 15,
    // and a jitter of at least 1 on every part.
    #[test]
    fn the_file_holds_every_genre_with_sane_numbers() {
        let tables = parse(FILE).expect("genres.json parses");
        assert_eq!(tables.len(), Genre::ALL.len());
        let names: Vec<&str> = tables.iter().map(|table| table.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Lo-fi",
                "Boom bap",
                "Trap",
                "Drill",
                "EDM / house",
                "Phonk",
                "Afrobeats"
            ]
        );
        for table in &tables {
            let range = &table.tempo_range;
            assert!(
                !range.is_empty() && range.contains(&(table.tempo.0 / 1000)),
                "{}",
                table.name
            );
            assert!(SWING_RANGE.contains(&table.swing_tenths), "{}", table.name);
            for (part, hit) in hits(table) {
                let velocity = match hit.velocity {
                    Velocity::Jitter(v) | Velocity::Exact(v) => v,
                };
                assert!(velocity <= 100, "{} {part}: {hit:?}", table.name);
                assert!(hit.step < STEPS_PER_BAR, "{} {part}: {hit:?}", table.name);
            }
            for melody in &table.melody {
                assert!(melody.starts.iter().all(|&start| start < STEPS_PER_BAR));
                assert!(
                    melody.low < melody.high && melody.high <= 127,
                    "{}",
                    table.name
                );
                assert!(
                    (1..=STEPS_PER_BAR).contains(&melody.max_length),
                    "{}",
                    table.name
                );
                assert!(!melody.note.is_empty(), "{}", table.name);
            }
            // Every part has a variant at least, each saying how it goes; the chords, the bass
            // and the melody two or more; six progressions a mode.
            let counts = {
                let mut counts = Vec::new();
                for variants in [
                    &table.kick,
                    &table.snare,
                    &table.clap,
                    &table.closed_hat,
                    &table.open_hat,
                    &table.perc,
                    &table.chords,
                    &table.bass,
                ] {
                    assert!(variants.iter().all(|variant| !variant.note.is_empty()));
                    counts.push(variants.len());
                }
                counts.push(table.melody.len());
                counts
            };
            assert_eq!(counts, table.variant_counts(), "{}", table.name);
            assert!(counts.iter().all(|&count| count >= 1), "{}", table.name);
            assert!(
                counts[6..].iter().all(|&count| count >= 2),
                "{}",
                table.name
            );
            assert_eq!(table.minor_progressions.len(), 6, "{}", table.name);
            assert_eq!(table.major_progressions.len(), 6, "{}", table.name);
        }
        // Lo-fi's: three kicks, two snares, no clap, three closed hats, two open hats, no
        // percussion, two chord rhythms, three bass shapes, three melodies.
        assert_eq!(tables[0].variant_counts(), [3, 2, 1, 3, 2, 1, 2, 3, 3]);
    }

    // A tuning mistake is refused with its place named: a mistyped field, a step past the bar,
    // a hit with two velocities, a jitter of 0, a melody band narrower than an octave or past
    // key 127, a note longer than a bar, a part with no variant, a variant with no note, a genre
    // missing.
    #[test]
    fn a_mistyped_table_is_refused_with_its_place() {
        let broken = |from: &str, to: &str| {
            let text = FILE.replacen(from, to, 1);
            assert_ne!(text, FILE, "{from} is in the file");
            parse(&text).expect_err("refused")
        };
        let error = broken(r#""chance": 40"#, r#""chanse": 40"#);
        assert!(error.contains("chanse"), "{error}");
        let error = broken(
            r#"{"step": 13, "chance": 25"#,
            r#"{"step": 16, "chance": 25"#,
        );
        assert_eq!(
            error,
            "lofi: kick variant 1 part 1: hit 4: step 16 is past 15"
        );
        let error = broken(r#""base": 90}"#, r#""base": 90, "exact": 30}"#);
        assert!(error.contains("base or exact"), "{error}");
        let error = broken(r#"{"hits": ["#, r#"{"jitter": 0, "hits": ["#);
        assert!(error.contains("jitter 0"), "{error}");
        let error = broken(
            r#""low": 60, "high": 72, "max_length": 12"#,
            r#""low": 60, "high": 66, "max_length": 12"#,
        );
        assert_eq!(
            error,
            "lofi: melody variant 1: band 60..66 must be an octave or more wide, at most 127"
        );
        let error = broken(
            r#""low": 60, "high": 72, "max_length": 12"#,
            r#""low": 120, "high": 132, "max_length": 12"#,
        );
        assert_eq!(
            error,
            "lofi: melody variant 1: band 120..132 must be an octave or more wide, at most 127"
        );
        let error = broken(r#""max_length": 12"#, r#""max_length": 17"#);
        assert_eq!(
            error,
            "lofi: melody variant 1: max_length 17 is outside 1..=16"
        );
        let error = broken(
            r#""clap": [
        {"_note": "no clap", "parts": []}
      ]"#,
            r#""clap": []"#,
        );
        assert!(error.starts_with("lofi: clap has no variant"), "{error}");
        let error = broken(r#"{"_note": "no clap", "parts": []}"#, r#"{"parts": []}"#);
        assert!(error.contains("_note"), "{error}");
        let error = broken(r#""afrobeats": {"#, r#""afrobeat": {"#);
        assert!(error.contains("no genre \"afrobeats\""), "{error}");
    }
}
