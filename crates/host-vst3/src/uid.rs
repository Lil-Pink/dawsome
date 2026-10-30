//! A class id as text: the 32 uppercase hex digits `FUID::toString` prints, the text a
//! `moduleinfo.json` file and a `.vstpreset` header carry, and what a project names a VST3 plugin
//! by (`vst3:<text>`). On Windows a class id's first three fields are kept in memory as a GUID
//! keeps them (its first word and the next two halves in little-endian order) and printed as the
//! integers they are; elsewhere the 16 bytes are printed as they lie. One `INLINE_UID` therefore
//! prints the same text on both platforms, and a project made on one finds its plugins on the
//! other (the SDK's `uid.h`; Dexed's Linux and Windows `moduleinfo.json` agree, plan 1.6).

use vst3::Steinberg::TUID;

/// The text of class id `cid`.
pub fn text(cid: &TUID) -> String {
    let bytes = cid.map(|byte| byte as u8);
    let mut digits = String::with_capacity(32);
    for byte in in_text_order(bytes) {
        digits.push_str(&format!("{byte:02X}"));
    }
    digits
}

/// The class id whose text is `text` (32 hex digits, either case); `None` for anything else.
pub fn parse(text: &str) -> Option<TUID> {
    if text.len() != 32 || !text.is_ascii() {
        return None;
    }
    let mut bytes = [0u8; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(in_text_order(bytes).map(|byte| byte as std::ffi::c_char))
}

/// The bytes of a class id in the order its text prints them, or back: on Windows the first
/// field's 4 bytes and the next two fields' 2 bytes each are reversed (the same swap both ways);
/// elsewhere they are as they lie.
fn in_text_order(mut bytes: [u8; 16]) -> [u8; 16] {
    if cfg!(windows) {
        bytes[0..4].reverse();
        bytes[4..6].reverse();
        bytes[6..8].reverse();
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    // 13. The SDK's INLINE_UID of the sine's four words prints the ASCII of its 16-byte name in
    // hex, on Linux and on Windows alike (vst3's `uid` lays the words out as each platform's
    // INLINE_UID does), and the text parses back to the same id.
    #[test]
    fn a_class_id_prints_the_same_text_on_every_platform() {
        let sine = vst3::uid(0x4461_7773, 0x6F6D_6554, 0x6573_7453, 0x696E_6531);
        assert_eq!(text(&sine), "446177736F6D655465737453696E6531");
        assert_eq!(parse("446177736F6D655465737453696E6531"), Some(sine));
        assert_eq!(
            parse("446177736f6d655465737453696e6531"),
            Some(sine),
            "lower case"
        );
        if cfg!(windows) {
            assert_eq!(
                &sine[..4],
                &[0x73, 0x77, 0x61, 0x44].map(|b| b as std::ffi::c_char)
            );
        } else {
            assert_eq!(
                &sine[..4],
                &[0x44, 0x61, 0x77, 0x73].map(|b| b as std::ffi::c_char)
            );
        }
        assert_eq!(parse("446177736F6D655465737453696E65"), None, "short");
        assert_eq!(parse("446177736F6D655465737453696E65ZZ"), None, "not hex");
    }
}
