//! Edited sounds: a sampler's or an audio channel's file played through its edit list
//! ([`dawsome_engine::dsp::edits`]), made ahead of playback as stretched clips are: on the UI
//! thread when it is one op past a short sound already held, else on a worker live and before
//! an offline render starts. A short result stays in memory; a long one is written as a 32-bit
//! float WAV at the project's rate into the cache and streamed from there, so what plays is
//! exactly what the list gives. The file itself is never written.
//!
//! The cache file is named by everything that makes it what it is (the file, its size and time,
//! a digest of its two ends, the rate and the list), so deleting the cache is always safe; the
//! folder is kept under [`CACHE_CAP`] bytes, oldest first, since every edit of a long sound
//! writes a whole file.

use std::collections::HashSet;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use dawsome_engine::dsp::edits;
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::project::SoundEdits;
use sha2::{Digest, Sha256};

use crate::decode::probe;
use crate::sample::{load_channel_sample, SampleNotice};
use crate::sounds::write_float_wav;

/// What every cache name and digest starts with: a change to how an op is applied bumps it.
pub const CACHE_VERSION: &str = "edits-v1";

/// The most the edited cache keeps, in bytes: 4 GB. Why a cap: each edit of a ten-minute
/// stereo sound writes 230 MB, 46 GB over 200 edits.
pub const CACHE_CAP: u64 = 4 << 30;

/// How much of each end of a sound's file its ends' digest reads: 64 KiB.
pub const END_BYTES: u64 = 64 << 10;

/// The first 32 hex digits of the SHA-256 of `bytes`.
fn hex_of(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The first 32 hex digits of the SHA-256 of `text`.
fn hex(text: &str) -> String {
    hex_of(text.as_bytes())
}

/// The digest of the file at `file`'s two ends: the first 32 hex digits of the SHA-256 of its
/// length (8 bytes, little-endian), its first [`END_BYTES`] and its last [`END_BYTES`] (the
/// whole file when it is shorter than both); `-` when it cannot be read. Why: a file replaced
/// by one of the same size and time (`cp -p`, `rsync -t`, `touch -r`) is told apart by what it
/// holds, without reading it whole (ten minutes of stereo are 230 MB). Its cost, per edit job
/// and per stretch of an edited sound: two reads of 64 KiB and their hash, about 65 µs from the
/// page cache in a release build. A change only inside a file's middle,
/// its ends, size and time the same, is not seen.
pub fn ends_digest(file: &Path) -> String {
    read_ends(file).map_or_else(|_| "-".to_owned(), |bytes| hex_of(&bytes))
}

/// The bytes [`ends_digest`] hashes.
fn read_ends(file: &Path) -> io::Result<Vec<u8>> {
    let mut opened = fs::File::open(file)?;
    let length = opened.metadata()?.len();
    let mut bytes = length.to_le_bytes().to_vec();
    (&mut opened).take(END_BYTES).read_to_end(&mut bytes)?;
    let head = bytes.len() as u64 - 8;
    opened.seek(SeekFrom::Start(length.saturating_sub(END_BYTES).max(head)))?;
    opened.take(END_BYTES).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// The edited sound's digest, for the names of what is made from it (a stretch): the first 32
/// hex digits of the SHA-256 of `edits-v1|<ends digest of file>|<the list as the project file
/// writes it>`. Why the ends: what is made from it must change with the file as the edited
/// sound does.
pub fn digest(file: &Path, edits: &SoundEdits) -> String {
    hex(&format!(
        "{CACHE_VERSION}|{}|{}",
        ends_digest(file),
        edits.key_text()
    ))
}

/// The cache file's name for the file at `file` played through `edits` at `rate`: the first 32
/// hex digits of the SHA-256 of
/// `edits-v1|<canonical path>|<size>|<mtime ns>|<ends digest>|<rate>|<list>`.
pub fn cache_name(file: &Path, rate: u32, edits: &SoundEdits) -> String {
    let path = fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
    let metadata = fs::metadata(&path).ok();
    let size = metadata.as_ref().map_or(0, fs::Metadata::len);
    let modified = metadata
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_nanos());
    let key = format!(
        "{CACHE_VERSION}|{}|{size}|{modified}|{}|{rate}|{}",
        path.display(),
        ends_digest(&path),
        edits.key_text()
    );
    format!("{}.wav", hex(&key))
}

/// An edited sound ready to play: in memory, or (long, for a clip) in the cache, to stream.
#[derive(Debug, Clone)]
pub enum Edited {
    Memory(SampleBuffer),
    Cached(PathBuf),
}

impl Edited {
    /// The sound's frames: those in memory, or the cached file's, decoded (at the project's rate
    /// already, so exactly the frames written).
    pub fn buffer(&self, rate: u32) -> (SampleBuffer, Vec<SampleNotice>) {
        match self {
            Edited::Memory(buffer) => (buffer.clone(), Vec::new()),
            Edited::Cached(file) => {
                let loaded = load_channel_sample(Path::new(""), file, rate);
                (loaded.buffer, loaded.notices)
            }
        }
    }
}

/// What making an edited sound gave, and what loading its file had to say.
#[derive(Debug, Clone)]
pub struct EditOutcome {
    pub result: Edited,
    pub notices: Vec<SampleNotice>,
    /// Whether it wrote a file into the cache.
    pub wrote: bool,
}

/// One edited sound to make: the file (absolute, or relative to the working directory) at the
/// project's `rate`, through `edits`; the cache folder (none: memory only); the frames above
/// which the result is written to the cache (and, when it `streams`, played from there); and
/// what the first ops already gave (`base`, with how many), so only the rest are applied.
#[derive(Debug, Clone)]
pub struct EditJob {
    pub file: PathBuf,
    pub rate: u32,
    pub edits: SoundEdits,
    pub cache: Option<PathBuf>,
    pub stream_threshold: u64,
    /// The asker streams a long result from its cached file (an audio clip); else (a sampler)
    /// it is handed back in memory.
    pub streams: bool,
    pub base: Option<(Edited, usize)>,
}

impl EditJob {
    /// Finds the edited sound in the cache, or makes it: the file decoded (or the base taken)
    /// and the list's ops applied in order; a long result written into the cache by way of a
    /// temporary file, so a half-written one is never read.
    pub fn run(&self) -> EditOutcome {
        let cached = self
            .cache
            .as_ref()
            .map(|dir| dir.join(cache_name(&self.file, self.rate, &self.edits)));
        if let Some(path) = cached.as_ref().filter(|path| path.is_file()) {
            let long = probe(path)
                .ok()
                .and_then(|info| info.frames)
                .is_some_and(|frames| frames > self.stream_threshold);
            let result = if self.streams && long {
                Edited::Cached(path.clone())
            } else {
                Edited::Memory(load_channel_sample(Path::new(""), path, self.rate).buffer)
            };
            return EditOutcome {
                result,
                notices: Vec::new(),
                wrote: false,
            };
        }
        let (source, done, notices) = match &self.base {
            Some((base, done)) => {
                let (buffer, notices) = base.buffer(self.rate);
                (buffer, *done, notices)
            }
            None => {
                let loaded = load_channel_sample(Path::new(""), &self.file, self.rate);
                (loaded.buffer, 0, loaded.notices)
            }
        };
        let ops = self.edits.ops.get(done..).unwrap_or_default();
        let buffer = edits::apply_ops(&source, ops);
        // A failed write leaves no file, so the next ask makes it again.
        let written = cached
            .filter(|_| buffer.frames() > self.stream_threshold)
            .filter(|path| write_float_wav(&buffer, self.rate, path).is_ok());
        let wrote = written.is_some();
        let result = match written {
            Some(path) if self.streams => Edited::Cached(path),
            _ => Edited::Memory(buffer),
        };
        EditOutcome {
            result,
            notices,
            wrote,
        }
    }
}

/// Takes the cache folder `dir` down to `cap` bytes, removing its oldest files first (by their
/// time), never one in `keep` (what plays now, or is kept to play while a newer list is made).
/// Returns the files removed.
pub fn trim(dir: &Path, cap: u64, keep: &HashSet<PathBuf>) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(SystemTime, u64, PathBuf)> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "wav"))
        .filter_map(|path| {
            let metadata = fs::metadata(&path).ok()?;
            let time = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            Some((time, metadata.len(), path))
        })
        .collect();
    let mut total: u64 = files.iter().map(|(_, size, _)| size).sum();
    files.sort();
    let mut removed = Vec::new();
    for (_, size, path) in files {
        if total <= cap {
            break;
        }
        if keep.contains(&path) || fs::remove_file(&path).is_err() {
            continue;
        }
        total -= size;
        removed.push(path);
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawsome_engine::project::SoundOp;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("dawsome-edited-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("a temp folder");
        dir
    }

    // A long result is written once and found again by its name, which the file's identity and
    // the list make: another list, another name. A short one stays in memory. The base's ops are
    // not applied twice.
    #[test]
    fn a_long_result_is_cached_by_the_file_and_the_list() {
        let dir = temp_dir("cache");
        let file = dir.join("ramp.wav");
        let ramp = SampleBuffer::mono((0..1000).map(|k| k as f32 / 1024.0).collect());
        write_float_wav(&ramp, 44_100, &file).expect("writes");
        let cache = dir.join("edited");
        let gain = SoundOp::Gain {
            from: 0,
            to: 1000,
            centi_db: -600,
        };
        let edits = SoundEdits {
            ops: vec![SoundOp::Reverse { from: 0, to: 1000 }, gain],
            source: None,
        };
        let job = EditJob {
            file: file.clone(),
            rate: 44_100,
            edits: edits.clone(),
            cache: Some(cache.clone()),
            stream_threshold: 500,
            streams: true,
            base: None,
        };
        let made = job.run();
        assert!(made.wrote);
        let Edited::Cached(path) = &made.result else {
            panic!("a long clip streams from the cache");
        };
        assert_eq!(path, &cache.join(cache_name(&file, 44_100, &edits)));
        let wanted = edits::apply(&ramp, &edits);
        let (cached, _) = made.result.buffer(44_100);
        assert_eq!(cached.samples(), wanted.samples());
        let again = job.run();
        assert!(!again.wrote, "found, not made again");
        let sampler = EditJob {
            streams: false,
            ..job.clone()
        }
        .run();
        let Edited::Memory(buffer) = sampler.result else {
            panic!("a sampler is handed its sound in memory");
        };
        assert_eq!(buffer.samples(), wanted.samples());
        let other = SoundEdits {
            ops: vec![SoundOp::Reverse { from: 0, to: 999 }],
            source: None,
        };
        assert_ne!(
            cache_name(&file, 44_100, &other),
            cache_name(&file, 44_100, &edits)
        );
        assert_ne!(digest(&file, &other), digest(&file, &edits));
        // From a base that already holds the reverse, only the gain is applied.
        let reversed = edits::apply_ops(&ramp, &edits.ops[..1]);
        let short = EditJob {
            cache: None,
            stream_threshold: 5000,
            base: Some((Edited::Memory(reversed), 1)),
            ..job
        }
        .run();
        let Edited::Memory(buffer) = short.result else {
            panic!("short: in memory");
        };
        assert_eq!(buffer.samples(), wanted.samples());
        let _ = fs::remove_dir_all(&dir);
    }

    // The cap: the oldest files go first until the folder fits, never one kept.
    #[test]
    fn the_cache_is_trimmed_oldest_first_keeping_what_plays() {
        let dir = temp_dir("trim");
        let mut paths = Vec::new();
        for (index, age) in [(0, 300), (1, 200), (2, 100), (3, 0)] {
            let path = dir.join(format!("{index}.wav"));
            fs::write(&path, vec![0u8; 1000]).expect("writes");
            let time = SystemTime::now() - std::time::Duration::from_secs(age);
            fs::File::options()
                .write(true)
                .open(&path)
                .and_then(|file| file.set_modified(time))
                .expect("dates");
            paths.push(path);
        }
        fs::write(dir.join("notes.txt"), "not a sound").expect("writes");
        let keep: HashSet<PathBuf> = [paths[0].clone()].into();
        let removed = trim(&dir, 2500, &keep);
        assert_eq!(removed, [paths[1].clone(), paths[2].clone()]);
        assert!(paths[0].is_file() && paths[3].is_file() && dir.join("notes.txt").is_file());
        assert!(trim(&dir, 2500, &keep).is_empty(), "fits now");
        let _ = fs::remove_dir_all(&dir);
    }

    // The name follows what the file holds at its ends, not only its size and time: a byte
    // changed in its first or last 64 KiB, the size and time put back, gives another name (and
    // another digest for what is made from it); one changed only in its middle does not (the
    // limit README states). A file shorter than both ends is read whole.
    #[test]
    fn a_file_replaced_at_the_same_size_and_time_is_told_by_its_ends() {
        let dir = temp_dir("ends");
        let edits = SoundEdits {
            ops: vec![SoundOp::Reverse { from: 0, to: 10 }],
            source: None,
        };
        let file = dir.join("long.wav");
        let length = 3 * END_BYTES as usize;
        fs::write(&file, vec![7u8; length]).expect("writes");
        let time = fs::metadata(&file)
            .and_then(|metadata| metadata.modified())
            .expect("a time");
        let changed_at = |at: usize| {
            let mut bytes = vec![7u8; length];
            bytes[at] = 8;
            fs::write(&file, bytes).expect("writes");
            fs::File::options()
                .write(true)
                .open(&file)
                .and_then(|opened| opened.set_modified(time))
                .expect("dated");
            (cache_name(&file, 44_100, &edits), digest(&file, &edits))
        };
        let first = changed_at(length / 2);
        assert_eq!(changed_at(length / 2 + 1), first, "the middle is not read");
        assert_ne!(changed_at(0), first, "the first byte");
        assert_ne!(changed_at(length - 1), first, "the last byte");
        let short = dir.join("short.wav");
        fs::write(&short, vec![7u8; 1000]).expect("writes");
        let whole = ends_digest(&short);
        let mut bytes = vec![7u8; 1000];
        bytes[500] = 8;
        fs::write(&short, bytes).expect("writes");
        assert_ne!(ends_digest(&short), whole, "a short file is read whole");
        assert_eq!(ends_digest(&dir.join("none.wav")), "-");
        let _ = fs::remove_dir_all(&dir);
    }
}
