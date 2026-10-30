//! A WAV file whose fmt chunk is a size its format's spec does not give, read as its first 16
//! bytes. Symphonia's WAV reader takes 16, 18 or 40 bytes for PCM and IEEE float and refuses
//! the rest ("malformed fmt_ieee chunk"); libsndfile reads them, and libraries ship them
//! (FreePats' vowel choir writes its float WAVs with 20: WAVEFORMAT's 16, cbSize 0 and two zero
//! bytes). Such a file is read through a view of it: the same bytes with that chunk cut to its
//! 16 (format, channels, rate, byte rate, block align, bits) and the RIFF size less what was
//! cut. Only the header is copied; the rest is read from the file as asked.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};

use symphonia::core::io::MediaSource;

/// The largest fmt chunk read to judge it: WAVE_FORMAT_EXTENSIBLE's 40, and room past it.
const MOST_FMT: u32 = 64;

/// How many chunks before the fmt chunk are passed over looking for it, at most.
const MOST_CHUNKS: usize = 64;

/// The WAVEFORMAT part every fmt chunk begins with.
const WAVEFORMAT: u32 = 16;

/// `file` as Symphonia reads it: through a view whose fmt chunk is its first 16 bytes when the
/// file is a WAV whose fmt chunk is PCM or IEEE float, of a size other than 16, 18 and 40, and
/// agrees with itself (block align = channels × bits / 8, byte rate = rate × block align,
/// cbSize inside the chunk); else `file` itself, from its start.
pub(crate) fn readable(mut file: File) -> io::Result<Box<dyn MediaSource>> {
    let cut = find_cut(&mut file)?;
    file.seek(SeekFrom::Start(0))?;
    Ok(match cut {
        Some(cut) => Box::new(Spliced::new(file, cut)?),
        None => Box::new(file),
    })
}

/// Where a fmt chunk to cut lies: the offset of its header, and its size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Cut {
    at: u64,
    size: u32,
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// The fmt chunk of `file` to cut, when there is one; `None` for any other file, which is read
/// as it is (Symphonia then reads it or says why not).
fn find_cut(file: &mut File) -> io::Result<Option<Cut>> {
    let mut riff = [0u8; 12];
    if read_all(file, &mut riff)? < riff.len() || &riff[..4] != b"RIFF" || &riff[8..] != b"WAVE" {
        return Ok(None);
    }
    let mut at = riff.len() as u64;
    for _ in 0..MOST_CHUNKS {
        let mut header = [0u8; 8];
        if read_all(file, &mut header)? < header.len() {
            return Ok(None);
        }
        let size = u32_at(&header, 4);
        match &header[..4] {
            b"fmt " => return Ok(cuttable(file, size)?.then_some(Cut { at, size })),
            b"data" => return Ok(None),
            _ => {
                let skip = u64::from(size) + u64::from(size & 1);
                file.seek(SeekFrom::Current(skip as i64))?;
                at += 8 + skip;
            }
        }
    }
    Ok(None)
}

/// Whether the fmt chunk of `size` bytes `file` is at the start of is one to cut.
fn cuttable(file: &mut File, size: u32) -> io::Result<bool> {
    if matches!(size, 16 | 18 | 40) || !(18..=MOST_FMT).contains(&size) {
        return Ok(false);
    }
    let mut fmt = vec![0u8; size as usize];
    if read_all(file, &mut fmt)? < fmt.len() {
        return Ok(false);
    }
    let (tag, channels, rate) = (u16_at(&fmt, 0), u16_at(&fmt, 2), u32_at(&fmt, 4));
    let (byte_rate, align, bits) = (u32_at(&fmt, 8), u16_at(&fmt, 12), u16_at(&fmt, 14));
    let extension = u32::from(u16_at(&fmt, 16));
    let agrees = channels > 0
        && bits > 0
        && bits % 8 == 0
        && u32::from(align) == u32::from(channels) * u32::from(bits) / 8
        && u64::from(byte_rate) == u64::from(rate) * u64::from(align)
        && 18 + extension <= size;
    Ok(matches!(tag, 1 | 3) && agrees)
}

/// Reads into `buf` until it is full or the file ends; how much it read.
fn read_all(file: &mut File, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match file.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}

/// The file with its fmt chunk cut: `head` (its bytes up to the chunk's 16th, the RIFF and the
/// chunk's sizes put right), then the file from `tail_at` (the chunk's end, its pad byte
/// passed) to its end.
struct Spliced {
    file: File,
    head: Vec<u8>,
    tail_at: u64,
    len: u64,
    /// Where the view reads next, and where the file is.
    position: u64,
    file_at: Option<u64>,
}

impl Spliced {
    fn new(mut file: File, cut: Cut) -> io::Result<Self> {
        let file_len = file.metadata()?.len();
        let kept = cut.at + 8 + u64::from(WAVEFORMAT);
        let tail_at = cut.at + 8 + u64::from(cut.size) + u64::from(cut.size & 1);
        let malformed = || io::Error::new(io::ErrorKind::InvalidData, "a fmt chunk past the file");
        if tail_at > file_len {
            return Err(malformed());
        }
        let mut head = vec![0u8; kept as usize];
        file.seek(SeekFrom::Start(0))?;
        if read_all(&mut file, &mut head)? < head.len() {
            return Err(malformed());
        }
        let removed = tail_at - kept;
        let riff = u64::from(u32_at(&head, 4)).saturating_sub(removed) as u32;
        head[4..8].copy_from_slice(&riff.to_le_bytes());
        let size_at = cut.at as usize + 4;
        head[size_at..size_at + 4].copy_from_slice(&WAVEFORMAT.to_le_bytes());
        Ok(Self {
            len: kept + (file_len - tail_at),
            file,
            head,
            tail_at,
            position: 0,
            file_at: None,
        })
    }
}

impl Read for Spliced {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let head = self.head.len() as u64;
        if self.position < head {
            let from = self.position as usize;
            let count = (self.head.len() - from).min(buf.len());
            buf[..count].copy_from_slice(&self.head[from..from + count]);
            self.position += count as u64;
            return Ok(count);
        }
        let at = self.tail_at + (self.position - head);
        if self.file_at != Some(at) {
            self.file.seek(SeekFrom::Start(at))?;
        }
        let read = self.file.read(buf)?;
        self.position += read as u64;
        self.file_at = Some(at + read as u64);
        Ok(read)
    }
}

impl Seek for Spliced {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let position = match to {
            SeekFrom::Start(at) => Some(at),
            SeekFrom::End(by) => self.len.checked_add_signed(by),
            SeekFrom::Current(by) => self.position.checked_add_signed(by),
        };
        let position = position.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "a seek before the file's start",
            )
        })?;
        self.position = position;
        Ok(position)
    }
}

impl MediaSource for Spliced {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dawsome-wav-fmt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join(name);
        std::fs::write(&path, bytes).expect("writes");
        path
    }

    /// A mono 32-bit float WAV at 8 kHz: a LIST chunk of `list` bytes (none for 0), the fmt
    /// chunk of `fmt_size` bytes (16, then cbSize `extension` and zeros), two frames.
    fn wav(list: usize, fmt_size: usize, extension: u16, align: u16) -> Vec<u8> {
        let mut fmt = Vec::new();
        for field in [3u16, 1] {
            fmt.extend(field.to_le_bytes());
        }
        fmt.extend(8000u32.to_le_bytes());
        fmt.extend((8000 * u32::from(align)).to_le_bytes());
        fmt.extend(align.to_le_bytes());
        fmt.extend(32u16.to_le_bytes());
        if fmt_size >= 18 {
            fmt.extend(extension.to_le_bytes());
            fmt.resize(fmt_size, 0);
        }
        let mut body = b"WAVE".to_vec();
        if list > 0 {
            body.extend(b"LIST");
            body.extend((list as u32).to_le_bytes());
            body.extend(vec![7u8; list + (list & 1)]);
        }
        body.extend(b"fmt ");
        body.extend((fmt.len() as u32).to_le_bytes());
        body.extend(&fmt);
        if fmt.len() % 2 == 1 {
            body.push(0);
        }
        body.extend(b"data");
        body.extend(8u32.to_le_bytes());
        body.extend(0.5f32.to_le_bytes());
        body.extend((-0.25f32).to_le_bytes());
        let mut file = b"RIFF".to_vec();
        file.extend((body.len() as u32).to_le_bytes());
        file.extend(body);
        file
    }

    fn view(bytes: &[u8], name: &str) -> (Option<Cut>, Vec<u8>) {
        let path = temp_file(name, bytes);
        let cut = find_cut(&mut File::open(&path).expect("opens")).expect("reads");
        let mut out = Vec::new();
        readable(File::open(&path).expect("opens"))
            .expect("a view")
            .read_to_end(&mut out)
            .expect("reads");
        (cut, out)
    }

    // By the spec: a 20-byte fmt chunk (after a LIST chunk of 3 bytes and its pad) is cut to
    // its 16, the RIFF size 4 less, and the view is the 16-byte file byte for byte; one of 21
    // drops its pad byte too. 16, 18 and 40 are left to Symphonia, as is a chunk whose block
    // align disagrees with its channels and bits, or whose cbSize runs past it.
    #[test]
    fn a_fmt_chunk_of_another_size_is_seen_as_its_first_16_bytes() {
        let plain = wav(3, 16, 0, 4);
        for (size, name) in [(20, "twenty.wav"), (21, "odd.wav")] {
            let (cut, out) = view(&wav(3, size, 0, 4), name);
            assert_eq!(
                cut,
                Some(Cut {
                    at: 12 + 8 + 4,
                    size: size as u32
                })
            );
            assert_eq!(out, plain, "{name}");
        }
        for (bytes, name) in [
            (wav(0, 16, 0, 4), "16.wav"),
            (wav(0, 18, 0, 4), "18.wav"),
            (wav(0, 40, 22, 4), "40.wav"),
            (wav(0, 20, 0, 8), "align.wav"),
            (wav(0, 20, 3, 4), "extension.wav"),
        ] {
            let (cut, out) = view(&bytes, name);
            assert_eq!((cut, out), (None, bytes), "{name}");
        }
        let (cut, out) = view(b"not a wav", "text.wav");
        assert_eq!((cut, out.as_slice()), (None, &b"not a wav"[..]));
    }

    // The view seeks as a file does: from its start, its end and where it is, in the head and
    // past it; and refuses a seek before its start.
    #[test]
    fn the_view_seeks_like_the_file_it_stands_for() {
        let plain = wav(0, 16, 0, 4);
        let path = temp_file("seek.wav", &wav(0, 20, 0, 4));
        let mut source = readable(File::open(&path).expect("opens")).expect("a view");
        assert_eq!(source.byte_len(), Some(plain.len() as u64));
        let mut byte = [0u8; 3];
        for (to, at) in [
            (SeekFrom::Start(10), 10),
            (SeekFrom::End(-3), plain.len() - 3),
            (SeekFrom::Current(-30), plain.len() - 30),
        ] {
            assert_eq!(source.seek(to).expect("seeks"), at as u64);
            source.read_exact(&mut byte).expect("reads");
            assert_eq!(byte, plain[at..at + 3]);
        }
        assert!(source.seek(SeekFrom::Current(-1000)).is_err());
    }
}
