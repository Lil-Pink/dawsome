//! What Settings says when a system MIDI port cannot be listed or opened, and which listed ports
//! are opened: the parts of the input path a Windows machine reaches that need no device.

use super::*;

// midir 0.11.1's WinMM backend (backend/winmm/mod.rs, `connect`) gives this error, word for word,
// when `midiInOpen` returns MMSYSERR_ALLOCATED: the port is open in another program, and WinMM
// gives each input port to one program at a time.
const WINMM_IN_USE: &str = "could not create Windows MM MIDI input port (MMSYSERR_ALLOCATED)";

#[test]
fn a_port_another_program_holds_says_so_and_what_to_do() {
    let line = open_problem("Chord Pad", WINMM_IN_USE);
    assert_eq!(
        line,
        "Chord Pad is in use by another program: close that program and it connects"
    );
    assert!(!line.contains("MMSYSERR"), "no system code for the user");
    // Any other refusal keeps midir's words after the port's name.
    assert_eq!(
        open_problem("Chord Pad", "could not create Windows MM MIDI input port"),
        "Cannot open MIDI input Chord Pad: could not create Windows MM MIDI input port"
    );
    assert_eq!(
        open_problem("Port A", "it is gone"),
        "Cannot open MIDI input Port A: it is gone"
    );
}

// Under Wine (GE-Proton 11-6), the released Windows build: winmm counted five MIDI inputs, midir's
// WinMM backend named none of them (each port's device-interface size query was refused, and the
// backend leaves such a port out without a word), and Settings would read "No MIDI input
// found" while devices were there. The count against the names says it instead.
#[test]
fn ports_the_system_counts_but_cannot_name_are_said() {
    assert_eq!(
        unread_ports(5, 0).as_deref(),
        Some("5 MIDI input ports could not be read from the system")
    );
    assert_eq!(
        unread_ports(1, 0).as_deref(),
        Some("1 MIDI input port could not be read from the system")
    );
    assert_eq!(
        unread_ports(4, 3).as_deref(),
        Some("1 MIDI input port could not be read from the system")
    );
    assert_eq!(unread_ports(3, 3), None, "every port named");
    assert_eq!(unread_ports(0, 0), None, "no device at all");
    // A port plugged in between the names and the count: more names than counted, nothing said.
    assert_eq!(unread_ports(2, 3), None);
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| (*name).to_owned()).collect()
}

// Two ports of one name (two of one device, or one device under a naming that does not number
// its twin) list the name twice. A port opens by its name, so opening both would reach the first
// port twice: WinMM refuses the second handle as in use (this program's own first one), which
// Settings would blame on another program; a system that shares ports would deliver each note
// twice. Each name opens once.
#[test]
fn a_name_listed_twice_is_opened_once() {
    let listed = names(&["Chord Pad", "Chord Pad", "Keys"]);
    assert_eq!(
        to_open(&listed, &[], |_| true),
        names(&["Chord Pad", "Keys"])
    );
    assert_eq!(
        to_open(&listed, &["Chord Pad"], |_| true),
        names(&["Keys"]),
        "an open port is not opened again"
    );
    assert_eq!(
        to_open(&listed, &[], |name| name == "Chord Pad"),
        names(&["Chord Pad"]),
        "only ports that play"
    );
    assert!(to_open(&listed, &["Chord Pad", "Keys"], |_| true).is_empty());
    assert!(to_open(&[], &[], |_| true).is_empty());
}
