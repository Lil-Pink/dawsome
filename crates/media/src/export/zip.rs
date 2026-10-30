//! A zip archive of stored (not compressed) entries, as a DAWproject file packs its documents,
//! audio and plugin states: each entry's local header and bytes, then the central directory and
//! its end record. Why stored: every zip reader takes it, the writer stays small, and audio
//! gains little from deflate. Without ZIP64 an archive holds at most 65535 entries and 4 GiB;
//! past that the writer refuses rather than write a file no reader opens.

use std::io::{self, Write};

/// CRC-32 as zip uses it (IEEE 802.3: reflected, polynomial 0xEDB88320), a byte at a time from
/// a table made at compile time.
const CRC_TABLE: [u32; 256] = crc_table();

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut index = 0;
    while index < 256 {
        let mut crc = index as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[index] = crc;
        index += 1;
    }
    table
}

/// The CRC-32 of `bytes`.
pub(crate) fn crc32(bytes: &[u8]) -> u32 {
    !bytes.iter().fold(!0u32, |crc, &byte| {
        CRC_TABLE[usize::from((crc as u8) ^ byte)] ^ (crc >> 8)
    })
}

/// 1980-01-01 00:00, the earliest time a zip entry can carry. Every entry carries it, so one
/// project always packs into the same bytes.
const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = (1 << 5) | 1;
/// Flag bit 11: the entry's name is UTF-8.
const UTF8_NAME: u16 = 1 << 11;
/// Version 2.0, the first that reads folders in names; what the entries need and are made by.
const VERSION: u16 = 20;
const LOCAL_HEADER: u32 = 0x0403_4b50;
const CENTRAL_HEADER: u32 = 0x0201_4b50;
const END_OF_CENTRAL: u32 = 0x0605_4b50;

fn too_big() -> io::Error {
    io::Error::other(
        "the package would pass 4 GiB or 65535 files, more than a zip without ZIP64 holds",
    )
}

/// Writes a zip of stored entries to `out`, entry by entry; [`ZipWriter::finish`] writes the
/// central directory.
pub(crate) struct ZipWriter<W: Write> {
    out: W,
    /// Bytes written so far: where the next entry's local header starts.
    offset: u64,
    /// The central directory's records, written at the end.
    central: Vec<u8>,
    entries: u16,
}

impl<W: Write> ZipWriter<W> {
    pub(crate) fn new(out: W) -> Self {
        Self {
            out,
            offset: 0,
            central: Vec::new(),
            entries: 0,
        }
    }

    /// Adds `bytes` as the stored entry `name` (`/` between folders, as zip writes them).
    pub(crate) fn add(&mut self, name: &str, bytes: &[u8]) -> io::Result<()> {
        let size = u32::try_from(bytes.len()).map_err(|_| too_big())?;
        let offset = u32::try_from(self.offset).map_err(|_| too_big())?;
        let name_len = u16::try_from(name.len()).map_err(|_| {
            io::Error::other(format!("a file name in the package is too long: {name}"))
        })?;
        let entries = self.entries.checked_add(1).ok_or_else(too_big)?;
        let crc = crc32(bytes);
        let mut local = Vec::with_capacity(30 + name.len());
        local.extend(LOCAL_HEADER.to_le_bytes());
        for field in [VERSION, UTF8_NAME, 0, DOS_TIME, DOS_DATE] {
            local.extend(field.to_le_bytes());
        }
        for field in [crc, size, size] {
            local.extend(field.to_le_bytes());
        }
        local.extend(name_len.to_le_bytes());
        local.extend(0u16.to_le_bytes());
        local.extend(name.as_bytes());
        let end = self
            .offset
            .checked_add(local.len() as u64 + u64::from(size))
            .filter(|&end| end <= u64::from(u32::MAX))
            .ok_or_else(too_big)?;
        self.out.write_all(&local)?;
        self.out.write_all(bytes)?;
        let central = &mut self.central;
        central.extend(CENTRAL_HEADER.to_le_bytes());
        for field in [VERSION, VERSION, UTF8_NAME, 0, DOS_TIME, DOS_DATE] {
            central.extend(field.to_le_bytes());
        }
        for field in [crc, size, size] {
            central.extend(field.to_le_bytes());
        }
        // Name length; extra field, comment, disk number, internal attributes: none.
        for field in [name_len, 0, 0, 0, 0] {
            central.extend(field.to_le_bytes());
        }
        // External attributes: none; then where the local header starts.
        for field in [0, offset] {
            central.extend(field.to_le_bytes());
        }
        central.extend(name.as_bytes());
        self.offset = end;
        self.entries = entries;
        Ok(())
    }

    /// Writes the central directory and its end record, and hands back the output.
    pub(crate) fn finish(mut self) -> io::Result<W> {
        let start = u32::try_from(self.offset).map_err(|_| too_big())?;
        let size = u32::try_from(self.central.len()).map_err(|_| too_big())?;
        start.checked_add(size).ok_or_else(too_big)?;
        self.out.write_all(&self.central)?;
        let mut end = Vec::with_capacity(22);
        end.extend(END_OF_CENTRAL.to_le_bytes());
        // This disk, the disk the directory starts on, its entries here and in all.
        for field in [0, 0, self.entries, self.entries] {
            end.extend(field.to_le_bytes());
        }
        for field in [size, start] {
            end.extend(field.to_le_bytes());
        }
        // No comment.
        end.extend(0u16.to_le_bytes());
        self.out.write_all(&end)?;
        self.out.flush()?;
        Ok(self.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The CRC-32 check value every description of the algorithm gives: "123456789" ->
    // 0xCBF43926; the empty input -> 0.
    #[test]
    fn crc32_gives_the_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    // One entry "a.txt" holding "hi", by hand from the format: a 30-byte local header (stored,
    // UTF-8 name, 1980-01-01, CRC of "hi" 0xD8932AAC, sizes 2), the name, the bytes; the
    // central record (46 bytes and the name) at 37; the end record naming 1 entry, 51 bytes of
    // directory from offset 37.
    #[test]
    fn one_stored_entry_is_laid_out_as_the_format_says() {
        let mut zip = ZipWriter::new(Vec::new());
        zip.add("a.txt", b"hi").expect("added");
        let bytes = zip.finish().expect("finished");
        let le16 = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
        let le32 = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().expect("4"));
        assert_eq!(bytes.len(), 30 + 5 + 2 + 46 + 5 + 22);
        assert_eq!(le32(0), LOCAL_HEADER);
        assert_eq!((le16(4), le16(6), le16(8)), (20, 0x0800, 0));
        assert_eq!((le16(10), le16(12)), (0, 0x0021));
        assert_eq!(le32(14), 0xD893_2AAC);
        assert_eq!((le32(18), le32(22)), (2, 2));
        assert_eq!((le16(26), le16(28)), (5, 0));
        assert_eq!(&bytes[30..37], b"a.txthi");
        assert_eq!(le32(37), CENTRAL_HEADER);
        assert_eq!(le32(37 + 42), 0, "the local header's offset");
        assert_eq!(&bytes[37 + 46..37 + 51], b"a.txt");
        let end = 37 + 51;
        assert_eq!(le32(end), END_OF_CENTRAL);
        assert_eq!((le16(end + 8), le16(end + 10)), (1, 1));
        assert_eq!((le32(end + 12), le32(end + 16)), (51, 37));
    }
}
