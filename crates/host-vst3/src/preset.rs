//! A VST3 plugin's state as the project keeps it: the SDK's `.vstpreset` file
//! (`vstpresetfile.cpp`), which holds the component's state and the controller's in one file,
//! the file DAWproject stores a VST3 plugin's state as. Little-endian throughout:
//!
//! - header: `b"VST3"`, int32 version 1, the class id as 32 ASCII characters
//!   ([`crate::uid::text`]), int64 the offset of the chunk list;
//! - the chunks' data, the component's (`Comp`) and then the controller's (`Cont`);
//! - the chunk list: `b"List"`, int32 the count, then for each chunk its id, int64 offset and
//!   int64 size.

/// What a preset file holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Preset<'a> {
    /// The class id's text, as the header names it.
    pub class: &'a str,
    pub component: &'a [u8],
    /// The controller's state; `None` when the file has no `Cont` chunk.
    pub controller: Option<&'a [u8]>,
}

const MAGIC: &[u8; 4] = b"VST3";
const VERSION: i32 = 1;
const HEADER: usize = 4 + 4 + 32 + 8;
const LIST: &[u8; 4] = b"List";
const COMPONENT: &[u8; 4] = b"Comp";
const CONTROLLER: &[u8; 4] = b"Cont";
/// The most chunks a list may name: a file naming more is not one the SDK writes.
const MOST_CHUNKS: usize = 64;

/// The preset file of class `class` (its id's text) holding `component`'s state and, when
/// there is a controller, `controller`'s.
pub(crate) fn write(class: &str, component: &[u8], controller: Option<&[u8]>) -> Vec<u8> {
    let chunks: Vec<(&[u8; 4], &[u8])> = std::iter::once((COMPONENT, component))
        .chain(controller.map(|state| (CONTROLLER, state)))
        .collect();
    let data: usize = chunks.iter().map(|(_, bytes)| bytes.len()).sum();
    let list_at = HEADER + data;
    let mut file = Vec::with_capacity(list_at + 8 + chunks.len() * 20);
    file.extend_from_slice(MAGIC);
    file.extend_from_slice(&VERSION.to_le_bytes());
    file.extend_from_slice(class.as_bytes());
    file.extend_from_slice(&(list_at as i64).to_le_bytes());
    for (_, bytes) in &chunks {
        file.extend_from_slice(bytes);
    }
    file.extend_from_slice(LIST);
    file.extend_from_slice(&(chunks.len() as i32).to_le_bytes());
    let mut offset = HEADER;
    for (id, bytes) in &chunks {
        file.extend_from_slice(*id);
        file.extend_from_slice(&(offset as i64).to_le_bytes());
        file.extend_from_slice(&(bytes.len() as i64).to_le_bytes());
        offset += bytes.len();
    }
    file
}

/// The preset `file` holds; `None` when it is not a preset file with a component chunk, every
/// offset and size inside it.
pub(crate) fn parse(file: &[u8]) -> Option<Preset<'_>> {
    if file.len() < HEADER || &file[..4] != MAGIC {
        return None;
    }
    let class = std::str::from_utf8(&file[8..40]).ok()?;
    if !class.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let list_at = usize::try_from(read_i64(file, 40)?).ok()?;
    if file.get(list_at..list_at.checked_add(4)?)? != LIST {
        return None;
    }
    let count = usize::try_from(read_i32(file, list_at + 4)?).ok()?;
    if count > MOST_CHUNKS {
        return None;
    }
    let (mut component, mut controller) = (None, None);
    for index in 0..count {
        let entry = list_at + 8 + index * 20;
        let id = file.get(entry..entry + 4)?;
        let offset = usize::try_from(read_i64(file, entry + 4)?).ok()?;
        let size = usize::try_from(read_i64(file, entry + 12)?).ok()?;
        let chunk = file.get(offset..offset.checked_add(size)?)?;
        if id == COMPONENT {
            component.get_or_insert(chunk);
        } else if id == CONTROLLER {
            controller.get_or_insert(chunk);
        }
    }
    Some(Preset {
        class,
        component: component?,
        controller,
    })
}

fn read_i64(file: &[u8], at: usize) -> Option<i64> {
    Some(i64::from_le_bytes(file.get(at..at + 8)?.try_into().ok()?))
}

fn read_i32(file: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_le_bytes(file.get(at..at + 4)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine as _;

    use super::*;

    const SINE: &str = "446177736F6D655465737453696E6531";

    fn component(gain: f64) -> Vec<u8> {
        let mut bytes = b"OTP1".to_vec();
        bytes.extend_from_slice(&gain.to_le_bytes());
        bytes
    }

    // 4. The sine's gain-0.5 preset is the 116 bytes the spec gives (by hand from
    // vstpresetfile.cpp's layout): header, the 12 bytes of the component and the 8 of the
    // controller, the list of two; it reads back as it was written.
    #[test]
    fn the_gain_half_preset_is_the_specs_116_bytes() {
        let controller = *b"OTC1\0\0\0\0";
        let file = write(SINE, &component(0.5), Some(&controller));
        assert_eq!(file.len(), 116);
        assert_eq!(
            BASE64.encode(&file),
            "VlNUMwEAAAA0NDYxNzc3MzZGNkQ2NTU0NjU3Mzc0NTM2OTZFNjUzMUQAAAAAAAAAT1RQMQAAAAAAAOA/T1RDMQAAAABMaXN0AgAAAENvbXAwAAAAAAAAAAwAAAAAAAAAQ29udDwAAAAAAAAACAAAAAAAAAA="
        );
        let read = parse(&file).expect("a preset");
        assert_eq!(
            read,
            Preset {
                class: SINE,
                component: &component(0.5),
                controller: Some(&controller),
            }
        );
    }

    // A preset without a controller chunk reads as the component alone; anything but a preset
    // reads as none: a CLAP state, a list that points past the end, a chunk that runs past it,
    // no component chunk.
    #[test]
    fn what_is_no_preset_is_refused() {
        let alone = write(SINE, &component(0.25), None);
        assert_eq!(alone.len(), 88);
        assert_eq!(parse(&alone).expect("a preset").controller, None);
        assert_eq!(parse(&component(0.5)), None, "a CLAP state");
        let mut far = alone.clone();
        far[40..48].copy_from_slice(&(1i64 << 40).to_le_bytes());
        assert_eq!(parse(&far), None, "a list past the end");
        let mut long = alone.clone();
        let size_at = alone.len() - 8;
        long[size_at..].copy_from_slice(&1000i64.to_le_bytes());
        assert_eq!(parse(&long), None, "a chunk past the end");
        let mut renamed = alone.clone();
        let id_at = alone.len() - 20;
        renamed[id_at..id_at + 4].copy_from_slice(b"Info");
        assert_eq!(parse(&renamed), None, "no component chunk");
        assert_eq!(parse(&[]), None);
    }
}
