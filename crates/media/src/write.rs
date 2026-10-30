//! Writing a stereo render to disk: WAV at 16 or 24 bits or 32-bit float, or MP3. A 16-bit WAV
//! is dithered unless told not to ([`Dither`]).

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use hound::{SampleFormat, WavSpec, WavWriter};
use mp3lame_encoder::{
    max_required_buffer_size, Bitrate, Builder, Encoder, FlushNoGap, InterleavedPcm, Mode, Quality,
};
use thiserror::Error;

/// The sample rates the MP3 encoder writes at without resampling.
pub const MP3_RATES: [u32; 3] = [32_000, 44_100, 48_000];

/// The most bytes the MP3 encoder's final flush writes.
const FLUSH_BYTES: usize = 7200;

/// Largest WAV data chunk written, leaving room for the header. Why check before writing: WAV
/// sizes are 32-bit fields, and the writer does not check them.
const MAX_WAV_DATA_BYTES: u64 = u32::MAX as u64 - 4096;

/// How the samples of a WAV file are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitDepth {
    Int16,
    Int24,
    Float32,
}

impl BitDepth {
    pub const ALL: [BitDepth; 3] = [BitDepth::Int16, BitDepth::Int24, BitDepth::Float32];

    /// The command line's names: `16`, `24`, `32f`.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|depth| depth.name() == name)
    }

    pub fn name(self) -> &'static str {
        match self {
            BitDepth::Int16 => "16",
            BitDepth::Int24 => "24",
            BitDepth::Float32 => "32f",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            BitDepth::Int16 => "16-bit",
            BitDepth::Int24 => "24-bit",
            BitDepth::Float32 => "32-bit float",
        }
    }

    fn bits(self) -> u16 {
        match self {
            BitDepth::Int16 => 16,
            BitDepth::Int24 => 24,
            BitDepth::Float32 => 32,
        }
    }
}

/// Whether a 16-bit WAV is dithered. Only 16 bits are: 24-bit and 32-bit float WAVs and MP3
/// are written as they always were, whichever is asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dither {
    /// TPDF dither: before a value is rounded to 16 bits, r1 + r2 − 1 LSB is added, r1 and r2
    /// the next two values in [0, 1) of the file's own generator ([`DITHER_SEED`]), so the
    /// rounding error is noise of one LSB at most either way that never follows the signal (a
    /// quiet fade then ends in a soft hiss, not in a buzz of the steps it is cut to), silence
    /// included. No noise shaping.
    #[default]
    Tpdf,
    /// Plain rounding, halves away from zero: the bytes every 16-bit file had before dither.
    Off,
}

/// Where each file's dither generator starts: x' = 1664525·x + 1013904223 (mod 2^32), each
/// value in [0, 1) its top 24 bits over 2^24. Why one fixed seed and not a random one: the same
/// project then renders the same bytes on every run and machine, and tests and the Python
/// generator (tests/fixtures/gen_p25_expected.py) state them.
pub const DITHER_SEED: u32 = 0x0D17_5EED;

/// How a render is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// WAV at its depth; the dither is heard only at 16 bits.
    Wav(BitDepth, Dither),
    /// Constant 320 kbps, encoder quality 2, stereo.
    Mp3,
}

impl Encoding {
    /// MP3 for a path ending in `.mp3` (any case), else WAV at `depth` with `dither`.
    pub fn for_path(path: &Path, depth: BitDepth, dither: Dither) -> Self {
        let mp3 = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("mp3"));
        if mp3 {
            Encoding::Mp3
        } else {
            Encoding::Wav(depth, dither)
        }
    }

    /// The most frames one file holds: a WAV file's data size is a 32-bit field; an MP3 file has
    /// no such field.
    pub fn max_frames(self) -> u64 {
        match self {
            Encoding::Wav(depth, _) => MAX_WAV_DATA_BYTES / (2 * u64::from(depth.bits() / 8)),
            Encoding::Mp3 => u64::MAX,
        }
    }
}

/// A file's TPDF dither ([`Dither::Tpdf`]): its generator's state, from [`DITHER_SEED`].
#[derive(Debug, Clone)]
pub struct Tpdf {
    state: u32,
}

impl Tpdf {
    fn new() -> Self {
        Self { state: DITHER_SEED }
    }

    /// The generator's next value in [0, 1), exact in f64.
    fn uniform(&mut self) -> f64 {
        self.state = self
            .state
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        f64::from(self.state >> 8) / f64::from(1u32 << 24)
    }

    /// The next value's noise in LSB: r1 + r2 − 1, triangular over (−1, 1). Drawn for every value
    /// written, whatever it is, so the noise depends on the value's place in the file alone.
    fn noise(&mut self) -> f64 {
        let first = self.uniform();
        let second = self.uniform();
        first + second - 1.0
    }
}

/// Why a render could not be written.
#[derive(Debug, Error)]
pub enum WriteError {
    #[error(transparent)]
    Wav(#[from] hound::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("MP3 export plays at 32000, 44100 or 48000 Hz; the project is {rate} Hz")]
    Mp3Rate { rate: u32 },
    #[error("the MP3 encoder failed: {0}")]
    Mp3(String),
    #[error("cannot convert the recording to the project's rate: {0}")]
    Resample(String),
}

/// A render file being written, frame by frame.
pub enum Sink {
    Wav {
        writer: WavWriter<BufWriter<File>>,
        depth: BitDepth,
        /// The dither's generator, for a dithered 16-bit file.
        dither: Option<Tpdf>,
    },
    Mp3 {
        encoder: Box<Encoder>,
        file: BufWriter<File>,
        /// Interleaved input and encoded output, reused between blocks.
        pcm: Vec<f32>,
        encoded: Vec<u8>,
    },
}

impl Sink {
    /// Creates (or truncates) the file at `path`.
    pub fn create(path: &Path, sample_rate: u32, encoding: Encoding) -> Result<Self, WriteError> {
        match encoding {
            Encoding::Wav(depth, dither) => {
                let spec = WavSpec {
                    channels: 2,
                    sample_rate,
                    bits_per_sample: depth.bits(),
                    sample_format: match depth {
                        BitDepth::Float32 => SampleFormat::Float,
                        BitDepth::Int16 | BitDepth::Int24 => SampleFormat::Int,
                    },
                };
                Ok(Sink::Wav {
                    writer: WavWriter::create(path, spec)?,
                    depth,
                    dither: (depth == BitDepth::Int16 && dither == Dither::Tpdf).then(Tpdf::new),
                })
            }
            Encoding::Mp3 => {
                if !MP3_RATES.contains(&sample_rate) {
                    return Err(WriteError::Mp3Rate { rate: sample_rate });
                }
                let failed =
                    |error: mp3lame_encoder::BuildError| WriteError::Mp3(error.to_string());
                let mut builder = Builder::new()
                    .ok_or_else(|| WriteError::Mp3("the encoder could not start".into()))?;
                builder.set_num_channels(2).map_err(failed)?;
                builder.set_sample_rate(sample_rate).map_err(failed)?;
                builder.set_brate(Bitrate::Kbps320).map_err(failed)?;
                builder.set_quality(Quality::NearBest).map_err(failed)?;
                builder.set_mode(Mode::Stereo).map_err(failed)?;
                let encoder = Box::new(builder.build().map_err(failed)?);
                Ok(Sink::Mp3 {
                    encoder,
                    file: BufWriter::new(File::create(path)?),
                    pcm: Vec::new(),
                    encoded: Vec::new(),
                })
            }
        }
    }

    /// Appends `frames`, left sample first.
    pub fn write(&mut self, frames: &[[f32; 2]]) -> Result<(), WriteError> {
        match self {
            Sink::Wav {
                writer,
                depth,
                dither,
            } => {
                for &sample in frames.as_flattened() {
                    match depth {
                        BitDepth::Float32 => writer.write_sample(sample)?,
                        BitDepth::Int16 | BitDepth::Int24 => {
                            let noise = dither.as_mut().map_or(0.0, Tpdf::noise);
                            writer.write_sample(rounded(sample, depth.bits(), noise))?;
                        }
                    }
                }
                Ok(())
            }
            Sink::Mp3 {
                encoder,
                file,
                pcm,
                encoded,
            } => {
                // The encoder's float input spans ±1.0; louder samples are clipped here, as an
                // integer WAV clips them.
                pcm.clear();
                pcm.extend(frames.as_flattened().iter().map(|s| s.clamp(-1.0, 1.0)));
                // Why reserve: the encoder writes into the spare capacity, and it reads a size
                // of 0 as "no limit", so an empty buffer would be written past its end.
                encoded.clear();
                encoded.reserve(max_required_buffer_size(frames.len()));
                encoder
                    .encode_to_vec(InterleavedPcm(pcm.as_slice()), encoded)
                    .map_err(|error| WriteError::Mp3(error.to_string()))?;
                file.write_all(encoded)?;
                Ok(())
            }
        }
    }

    /// Completes the file; without this it is malformed (WAV) or misses its last frames (MP3).
    pub fn finish(self) -> Result<(), WriteError> {
        match self {
            Sink::Wav { writer, .. } => Ok(writer.finalize()?),
            Sink::Mp3 {
                mut encoder,
                mut file,
                mut encoded,
                ..
            } => {
                encoded.clear();
                encoded.reserve(FLUSH_BYTES);
                encoder
                    .flush_to_vec::<FlushNoGap>(&mut encoded)
                    .map_err(|error| WriteError::Mp3(error.to_string()))?;
                file.write_all(&encoded)?;
                file.flush()?;
                Ok(())
            }
        }
    }
}

/// A float sample as a `bits`-bit integer: round(x · 2^(bits-1)), halves away from zero,
/// clamped to the integer range; no dither. NaN becomes 0. The tests' plain-rounding oracle;
/// the writers go through [`rounded`] (dither being a noise of 0 when off).
#[cfg(test)]
fn quantize(sample: f32, bits: u16) -> i32 {
    rounded(sample, bits, 0.0)
}

/// [`quantize`] with `noise` LSB added before the rounding (0 adds nothing: x·2^(bits−1) + 0 is
/// x·2^(bits−1) to the bit).
fn rounded(sample: f32, bits: u16, noise: f64) -> i32 {
    let full = f64::from(1u32 << (bits - 1));
    (f64::from(sample) * full + noise)
        .round()
        .clamp(-full, full - 1.0) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    // Python: round-half-away of x · 2^15 and x · 2^23, clamped.
    #[test]
    fn floats_quantize_by_rounding_and_clamping() {
        let cases = [
            (1.0, 32_767, 8_388_607),
            (0.5, 16_384, 4_194_304),
            (-1.0, -32_768, -8_388_608),
            (16.75, 32_767, 8_388_607),
            (-0.25, -8_192, -2_097_152),
            // Half a 16-bit step rounds away from zero; 0.49 of one rounds to 0.
            (0.5 / 32_768.0, 1, 128),
            (-0.5 / 32_768.0, -1, -128),
            (0.49 / 32_768.0, 0, 125),
        ];
        for (sample, sixteen, twenty_four) in cases {
            assert_eq!(quantize(sample, 16), sixteen, "{sample} at 16 bits");
            assert_eq!(quantize(sample, 24), twenty_four, "{sample} at 24 bits");
        }
        assert_eq!(quantize(f32::NAN, 16), 0);
    }

    #[test]
    fn depths_have_names_and_the_extension_picks_mp3() {
        assert_eq!(BitDepth::parse("24"), Some(BitDepth::Int24));
        assert_eq!(BitDepth::parse("32f"), Some(BitDepth::Float32));
        assert_eq!(BitDepth::parse("32"), None);
        assert_eq!(
            Encoding::for_path(Path::new("song.MP3"), BitDepth::Int16, Dither::Tpdf),
            Encoding::Mp3
        );
        assert_eq!(
            Encoding::for_path(Path::new("song.wav"), BitDepth::Int16, Dither::Off),
            Encoding::Wav(BitDepth::Int16, Dither::Off)
        );
        // 2^32 - 1 - 4096 bytes of 32-bit float stereo.
        assert_eq!(
            Encoding::Wav(BitDepth::Float32, Dither::Tpdf).max_frames(),
            536_870_399
        );
    }

    // By hand: the generator's first state is 1664525·0x0D175EED + 1013904223 mod 2^32 =
    // 0x7B963368 (its top 24 bits 0x7B9633 over 2^24), the second 0x60B2FFA7; so the first
    // noise is 0x7B9633/2^24 + 0x60B2FF/2^24 − 1. Every value is in (−1, 1) and a quarter of
    // them round silence to ±1 (P(|r1 + r2 − 1| ≥ 1/2) = 1/4).
    #[test]
    fn the_dither_is_the_lcg_runs_triangle() {
        let mut tpdf = Tpdf::new();
        let first = tpdf.noise();
        let by_hand = f64::from(0x007B_9633_u32) / 16_777_216.0
            + f64::from(0x0060_B2FF_u32) / 16_777_216.0
            - 1.0;
        assert_eq!(first, by_hand);
        let noise: Vec<f64> = (0..400_000).map(|_| tpdf.noise()).collect();
        assert!(noise.iter().all(|n| (-1.0..1.0).contains(n)));
        let heard = noise.iter().filter(|&&n| rounded(0.0, 16, n) != 0).count();
        assert!((heard as f64 / 400_000.0 - 0.25).abs() < 0.005, "{heard}");
        // Plain rounding is the dither's with no noise, to the bit.
        for sample in [0.5 / 32_768.0, -1.0, 0.123_456_7, 2.0] {
            assert_eq!(rounded(sample, 16, 0.0), quantize(sample, 16));
        }
    }

    // The first block meets an empty output buffer; one that the encoder took as unlimited
    // would crash here.
    #[test]
    fn an_mp3_written_in_blocks_decodes_to_stereo_at_its_rate() {
        let path = std::env::temp_dir().join(format!("dawsome-blocks-{}.mp3", std::process::id()));
        let mut sink = Sink::create(&path, 48_000, Encoding::Mp3).expect("creates");
        let frames = vec![[0.25f32, -0.25]; 512];
        for _ in 0..20 {
            sink.write(&frames).expect("encodes");
        }
        sink.finish().expect("flushes");
        let decoded = crate::decode::decode(&path).expect("decodes");
        assert_eq!((decoded.sample_rate, decoded.channels), (48_000, 2));
        assert!(decoded.frames() >= 20 * 512, "{} frames", decoded.frames());
    }

    #[test]
    fn mp3_refuses_a_rate_it_cannot_write() {
        let path = std::env::temp_dir().join(format!("dawsome-{}.mp3", std::process::id()));
        let result = Sink::create(&path, 22_050, Encoding::Mp3);
        assert!(matches!(result, Err(WriteError::Mp3Rate { rate: 22_050 })));
    }
}
