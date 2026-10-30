//! Decoding a sound file of any supported format (WAV, AIFF and AIFC, FLAC, Ogg Vorbis, MP3)
//! into interleaved 32-bit float frames at the file's own rate.

use std::fs::File;
use std::io;
use std::ops::Range;
use std::path::Path;

use symphonia::core::audio::{AudioBuffer, AudioBufferRef, Signal};
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader, Track};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::sample::Sample;
use thiserror::Error;

/// A decoded file: `channels` interleaved values per frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Decoded {
    pub sample_rate: u32,
    pub channels: usize,
    pub samples: Vec<f32>,
}

impl Decoded {
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1)
    }
}

/// Why a file could not be decoded.
#[derive(Debug, Error)]
pub enum DecodeError {
    #[error(transparent)]
    Open(#[from] io::Error),
    #[error("{0}")]
    Format(#[from] SymphoniaError),
    #[error("it holds no audio track")]
    NoTrack,
    #[error("it gives no sample rate")]
    NoSampleRate,
    #[error("its channel count changes part way")]
    ChannelsChanged,
    #[error("its header gives no length")]
    NoLength,
    #[error("it does not time its packets in frames")]
    NotInFrames,
}

/// What a sound file holds, read from its header without decoding it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileInfo {
    pub sample_rate: u32,
    pub channels: usize,
    /// Frames at the file's own rate; `None` when the header does not say (some MP3 files).
    pub frames: Option<u64>,
}

/// The format reader of the file at `path`, its header read. A WAV whose fmt chunk is a size
/// its spec does not give is read through a view with that chunk cut to its first 16 bytes
/// ([`crate::wav_fmt`]).
pub(crate) fn format_reader(path: &Path) -> Result<Box<dyn FormatReader>, DecodeError> {
    let source = MediaSourceStream::new(
        crate::wav_fmt::readable(File::open(path)?)?,
        Default::default(),
    );
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|extension| extension.to_str()) {
        hint.with_extension(extension);
    }
    let probed = symphonia::default::get_probe().format(
        &hint,
        source,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;
    Ok(probed.format)
}

/// The first track of `format` that holds audio.
pub(crate) fn audio_track(format: &dyn FormatReader) -> Result<&Track, DecodeError> {
    format
        .tracks()
        .iter()
        .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or(DecodeError::NoTrack)
}

/// Reads the header of the file at `path`: its rate, channel count and length.
pub fn probe(path: &Path) -> Result<FileInfo, DecodeError> {
    let format = format_reader(path)?;
    let params = &audio_track(&*format)?.codec_params;
    Ok(FileInfo {
        sample_rate: params.sample_rate.ok_or(DecodeError::NoSampleRate)?,
        channels: params.channels.map_or(1, |layout| layout.count()).max(1),
        frames: params.n_frames,
    })
}

/// Decodes the first audio track of the file at `path`.
///
/// Integer PCM is scaled by 2^-(bits-1), so a full-scale value maps to ±1.0; floats are kept as
/// they are. A packet the decoder reports as corrupt is skipped, as one bad frame should not
/// cost the whole file.
pub fn decode(path: &Path) -> Result<Decoded, DecodeError> {
    let mut format = format_reader(path)?;
    let track = audio_track(&*format)?;
    let track_id = track.id;
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or(DecodeError::NoSampleRate)?;
    let mut channels = track.codec_params.channels.map(|layout| layout.count());
    let mut decoder =
        symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
    let mut samples = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            // How every format reader reports the end of the file.
            Err(SymphoniaError::IoError(error)) if error.kind() == io::ErrorKind::UnexpectedEof => {
                break
            }
            Err(error) => return Err(error.into()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let buffer = match decoder.decode(&packet) {
            Ok(buffer) => buffer,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(error) => return Err(error.into()),
        };
        let count = buffer.spec().channels.count();
        if *channels.get_or_insert(count) != count {
            return Err(DecodeError::ChannelsChanged);
        }
        append(&buffer, &mut samples);
    }
    Ok(Decoded {
        sample_rate,
        channels: channels.unwrap_or(1).max(1),
        samples,
    })
}

/// 2^-n, exact in f32 for the depths used here.
fn scale(bits: i32) -> f32 {
    0.5f32.powi(bits)
}

/// Appends `buffer` interleaved, as f32.
fn append(buffer: &AudioBufferRef<'_>, out: &mut Vec<f32>) {
    out.reserve(buffer.frames() * buffer.spec().channels.count());
    each_value(buffer, 0..buffer.frames(), |_, value| out.push(value));
}

/// Gives `put` the channel and the f32 value of every sample of `frames` of `buffer`, frame by
/// frame, by the rules [`decode`] states. The one place a decoded sample becomes an f32, so a
/// file streamed from disk plays the values of the same file decoded into memory.
pub(crate) fn each_value(
    buffer: &AudioBufferRef<'_>,
    frames: Range<usize>,
    put: impl FnMut(usize, f32),
) {
    match buffer {
        AudioBufferRef::F32(buffer) => visit(buffer, frames, put, |value| value),
        AudioBufferRef::F64(buffer) => visit(buffer, frames, put, |value| value as f32),
        AudioBufferRef::S8(buffer) => visit(buffer, frames, put, |v| f32::from(v) * scale(7)),
        AudioBufferRef::S16(buffer) => visit(buffer, frames, put, |v| f32::from(v) * scale(15)),
        AudioBufferRef::S24(buffer) => {
            visit(buffer, frames, put, |v| v.inner() as f32 * scale(23));
        }
        AudioBufferRef::S32(buffer) => visit(buffer, frames, put, |v| v as f32 * scale(31)),
        // Unsigned PCM is centered on half its range.
        AudioBufferRef::U8(buffer) => {
            visit(buffer, frames, put, |v| {
                (i32::from(v) - 0x80) as f32 * scale(7)
            });
        }
        AudioBufferRef::U16(buffer) => {
            visit(buffer, frames, put, |v| {
                (i32::from(v) - 0x8000) as f32 * scale(15)
            });
        }
        AudioBufferRef::U24(buffer) => visit(buffer, frames, put, |v| {
            (v.inner() as i32 - 0x80_0000) as f32 * scale(23)
        }),
        AudioBufferRef::U32(buffer) => visit(buffer, frames, put, |v| {
            (i64::from(v) - 0x8000_0000) as f32 * scale(31)
        }),
    }
}

fn visit<S: Sample>(
    buffer: &AudioBuffer<S>,
    frames: Range<usize>,
    mut put: impl FnMut(usize, f32),
    convert: impl Fn(S) -> f32,
) {
    let channels = buffer.spec().channels.count();
    for frame in frames {
        for channel in 0..channels {
            put(channel, convert(buffer.chan(channel)[frame]));
        }
    }
}

#[cfg(test)]
mod tests {
    use hound::{SampleFormat, WavSpec, WavWriter};

    use super::*;

    fn temp_path(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dawsome-media-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir.join(name)
    }

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name)
    }

    // Every fixture sound decodes to exactly what the WAV reader the project used before gives.
    #[test]
    fn fixture_wavs_decode_bit_for_bit_like_the_wav_reader() {
        for name in [
            "impulse.wav",
            "ramp4.wav",
            "hold2000.wav",
            "sine1k_48k.wav",
            "sine3k_48k.wav",
        ] {
            let path = fixture(name);
            let decoded = decode(&path).expect("decodes");
            let mut reader = hound::WavReader::open(&path).expect("opens");
            let spec = reader.spec();
            let expected: Vec<f32> = reader
                .samples::<f32>()
                .collect::<Result<_, _>>()
                .expect("reads");
            assert_eq!(decoded.sample_rate, spec.sample_rate, "{name}");
            assert_eq!(decoded.channels, 1, "{name}");
            let same = decoded
                .samples
                .iter()
                .zip(&expected)
                .all(|(a, b)| a.to_bits() == b.to_bits());
            assert!(same && decoded.samples.len() == expected.len(), "{name}");
        }
    }

    // 16- and 24-bit integers scale by 2^-15 and 2^-23: 16384 is 0.5, -32768 is -1.0,
    // 4194304 is 0.5 at 24 bits. Stereo stays interleaved.
    #[test]
    fn integer_pcm_scales_to_plus_minus_one() {
        for (bits, values, expected) in [
            (
                16,
                vec![16_384, -32_768, 1],
                vec![0.5, -1.0, 1.0 / 32_768.0],
            ),
            (
                24,
                vec![4_194_304, -8_388_608, 1],
                vec![0.5, -1.0, 1.0 / 8_388_608.0],
            ),
        ] {
            let path = temp_path(&format!("int{bits}.wav"));
            let spec = WavSpec {
                channels: 1,
                sample_rate: 22_050,
                bits_per_sample: bits,
                sample_format: SampleFormat::Int,
            };
            let mut writer = WavWriter::create(&path, spec).expect("creates");
            for value in values {
                writer.write_sample(value).expect("writes");
            }
            writer.finalize().expect("finalizes");
            let decoded = decode(&path).expect("decodes");
            assert_eq!(decoded.sample_rate, 22_050);
            assert_eq!(decoded.samples, expected, "{bits} bits");
        }
        let path = temp_path("stereo.wav");
        let spec = WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 32,
            sample_format: SampleFormat::Float,
        };
        let mut writer = WavWriter::create(&path, spec).expect("creates");
        for value in [0.25f32, -0.5, 0.75, 1.0] {
            writer.write_sample(value).expect("writes");
        }
        writer.finalize().expect("finalizes");
        let decoded = decode(&path).expect("decodes");
        assert_eq!((decoded.channels, decoded.frames()), (2, 2));
        assert_eq!(decoded.samples, [0.25, -0.5, 0.75, 1.0]);
    }

    /// p11_expected.json, the instrument sets' known answers (gen_p11_expected.py).
    fn p11_expected() -> serde_json::Value {
        let text = std::fs::read_to_string(fixture("p11_expected.json")).expect("p11_expected");
        serde_json::from_str(&text).expect("parses")
    }

    // Known answers (gen_p11_expected.py's set "formats", its sounds written from their specs):
    // each decodes to its rate, channels and values, bit for bit, and a sound whose fmt chunk
    // has a size its spec does not give (20 bytes, float or PCM) decodes as its 16-byte twin.
    #[test]
    fn sounds_in_the_forms_real_libraries_write_decode_to_their_values() {
        let formats = &p11_expected()["formats"];
        let dir = fixture(formats["dir"].as_str().expect("dir"));
        for (name, want) in formats["sounds"].as_object().expect("sounds") {
            let decoded = decode(&dir.join(name)).unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(
                u64::from(decoded.sample_rate),
                want["rate"].as_u64().expect("rate"),
                "{name}"
            );
            assert_eq!(
                decoded.channels as u64,
                want["channels"].as_u64().expect("channels"),
                "{name}"
            );
            let values: Vec<u32> = want["values"]
                .as_array()
                .expect("values")
                .iter()
                .map(|v| (v.as_f64().expect("a value") as f32).to_bits())
                .collect();
            let got: Vec<u32> = decoded.samples.iter().map(|v| v.to_bits()).collect();
            assert_eq!(got, values, "{name}");
        }
        for pair in formats["twins"].as_array().expect("twins") {
            let [a, b] = [0, 1].map(|i| dir.join(pair[i].as_str().expect("a name")));
            assert_eq!(decode(&a).expect("decodes"), decode(&b).expect("decodes"));
        }
    }

    #[test]
    fn a_missing_or_foreign_file_is_an_error() {
        assert!(matches!(
            decode(Path::new("/nonexistent/x.wav")),
            Err(DecodeError::Open(_))
        ));
        let text = temp_path("not-audio.wav");
        std::fs::write(&text, b"plain text, not audio").expect("writes");
        assert!(decode(&text).is_err());
    }
}
