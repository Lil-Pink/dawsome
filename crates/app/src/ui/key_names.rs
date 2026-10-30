//! A key's name, the one way the window names a MIDI key everywhere: the piano roll's key
//! column, the typing keyboard's octave, the instrument window's keys and roots, the hint bar.
//! MIDI key 60 is "C5" (the octave is the key divided by 12), so key 0 is "C0" and key 127
//! "G10". Why this numbering and not 60 = C4: the piano roll and the typing keyboard already
//! named key 60 so, as the workflow the app follows does; the sets' names carry no note.

/// The name of MIDI key `key`: its pitch class, sharps as "#", then its octave, key 60 "C5".
pub fn key_name(key: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!("{}{}", NAMES[usize::from(key % 12)], key / 12)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Written by hand from the rule key 60 = C5: twelve keys an octave, C first.
    #[test]
    fn keys_are_named_with_60_as_c5() {
        let table = [
            (0, "C0"),
            (1, "C#0"),
            (11, "B0"),
            (12, "C1"),
            (21, "A1"),
            (24, "C2"),
            (48, "C4"),
            (59, "B4"),
            (60, "C5"),
            (61, "C#5"),
            (64, "E5"),
            (69, "A5"),
            (72, "C6"),
            (120, "C10"),
            (127, "G10"),
        ];
        for (key, name) in table {
            assert_eq!(key_name(key), name, "key {key}");
        }
    }
}
