//! decode: bounded vorbis/WAV decode behind the `device` feature.
//!
//! [`decode_wav`] (hound) and [`decode_vorbis`] (lewton) turn byte blobs into
//! [`Pcm`](crate::Pcm) for [`crate::Mixer::play`]. Every entrypoint is typed,
//! size-bounded and panic-free: oversize input, truncated streams and garbage
//! bytes all return [`DecodeError`], never panic. Vorbis has no in-repo
//! encoder, so its positive path is covered by rejection tests here plus a
//! real-file check when O7 content lands; the WAV roundtrip below proves the
//! shared [`Pcm`](crate::Pcm) plumbing end to end.

use std::io::Cursor;
use std::path::Path;

use crate::cue::{CueBank, CueError, LoadReport};
use crate::Pcm;

/// Inputs larger than this are rejected before parsing (8 MiB covers minutes
/// of compressed game audio; larger blobs belong in the asset cooker).
pub const MAX_DECODE_BYTES: usize = 8 * 1024 * 1024;
/// Decoded frame cap per call (~83 s of mono 48 kHz); longer clips must
/// stream instead of decoding whole.
pub const MAX_PCM_FRAMES: usize = 4_000_000;
/// Channel counts accepted by both decoders (mono/stereo game audio).
pub const MAX_CHANNELS: u16 = 2;

/// Pure pack-vs-stream decision for a decoded [`Pcm`](crate::Pcm):
/// forwards `pcm.frames()`, `instances` and `pcm.samples.len() * 4`
/// (f32 bytes) to [`crate::cue::should_stream`]. No new decode path.
pub fn pcm_should_stream(pcm: &crate::Pcm, instances: usize) -> bool {
    crate::cue::should_stream(pcm.frames(), instances, pcm.samples.len().saturating_mul(4))
}

/// Typed decode failure; every variant is a clean rejection, never a panic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// No bytes to decode.
    Empty,
    /// Input exceeds [`MAX_DECODE_BYTES`]; carries the offered length.
    TooLarge {
        /// Offered byte length.
        bytes: usize,
    },
    /// Channel count outside `1..=2`; carries the offered count.
    UnsupportedChannels {
        /// Offered channel count.
        channels: u16,
    },
    /// Zero sample rate in the container header.
    UnsupportedRate,
    /// Decoded length exceeds [`MAX_PCM_FRAMES`]; carries the frame count.
    TooManyFrames {
        /// Decoded (or header-declared) frame count.
        frames: u64,
    },
    /// WAV header unparseable or an unsupported encoding/bit depth.
    InvalidWav,
    /// Stream ends mid-sample (truncated body or cut packet).
    Truncated,
    /// Ogg/Vorbis headers unparseable, not Vorbis at all, or zero audio
    /// packets after valid headers.
    InvalidVorbis,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "decode: empty input"),
            Self::TooLarge { bytes } => write!(f, "decode: input {bytes} bytes exceeds limit"),
            Self::UnsupportedChannels { channels } => {
                write!(f, "decode: {channels} channels not supported (1..=2)")
            }
            Self::UnsupportedRate => write!(f, "decode: zero sample rate"),
            Self::TooManyFrames { frames } => {
                write!(f, "decode: {frames} frames exceeds limit")
            }
            Self::InvalidWav => write!(f, "decode: invalid wav"),
            Self::Truncated => write!(f, "decode: truncated stream"),
            Self::InvalidVorbis => write!(f, "decode: invalid vorbis"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Decode WAV bytes (PCM integer 8/16/24/32-bit or 32-bit float, 1-2
/// channels) into [`Pcm`](crate::Pcm). Bounds: empty, `> MAX_DECODE_BYTES`,
/// bad channel counts, zero rates, `> MAX_PCM_FRAMES` and truncated bodies
/// are all [`DecodeError`]. Never panics.
pub fn decode_wav(bytes: &[u8]) -> Result<Pcm, DecodeError> {
    if bytes.is_empty() {
        return Err(DecodeError::Empty);
    }
    if bytes.len() > MAX_DECODE_BYTES {
        return Err(DecodeError::TooLarge { bytes: bytes.len() });
    }
    let mut reader =
        hound::WavReader::new(Cursor::new(bytes)).map_err(|_| DecodeError::InvalidWav)?;
    let spec = reader.spec();
    if spec.channels == 0 || spec.channels > MAX_CHANNELS {
        return Err(DecodeError::UnsupportedChannels {
            channels: spec.channels,
        });
    }
    if spec.sample_rate == 0 {
        return Err(DecodeError::UnsupportedRate);
    }
    let frames = u64::from(reader.duration());
    if frames > MAX_PCM_FRAMES as u64 {
        return Err(DecodeError::TooManyFrames { frames });
    }
    let channels = usize::from(spec.channels);
    let sample_cap = MAX_PCM_FRAMES.saturating_mul(usize::from(MAX_CHANNELS));
    let mut out = Vec::new();
    match spec.sample_format {
        hound::SampleFormat::Int => {
            let scale = match spec.bits_per_sample {
                8 => 128.0,
                16 => 32_768.0,
                24 => 8_388_608.0,
                32 => 2_147_483_600.0,
                _ => return Err(DecodeError::InvalidWav),
            };
            for s in reader.samples::<i32>() {
                let v = s.map_err(|_| DecodeError::Truncated)?;
                if out.len() >= sample_cap {
                    return Err(DecodeError::TooManyFrames {
                        frames: out.len() as u64 / channels.max(1) as u64,
                    });
                }
                out.push((v as f32) / scale);
            }
        }
        hound::SampleFormat::Float => {
            if spec.bits_per_sample != 32 {
                return Err(DecodeError::InvalidWav);
            }
            for s in reader.samples::<f32>() {
                let v = s.map_err(|_| DecodeError::Truncated)?;
                if out.len() >= sample_cap {
                    return Err(DecodeError::TooManyFrames {
                        frames: out.len() as u64 / channels.max(1) as u64,
                    });
                }
                out.push(v);
            }
        }
    }
    if out.is_empty() {
        return Err(DecodeError::Truncated);
    }
    Ok(Pcm::new(spec.sample_rate, spec.channels, out))
}

/// Decode Ogg Vorbis bytes (1-2 channels) into [`Pcm`](crate::Pcm).
/// Header failures map to [`DecodeError::InvalidVorbis`], mid-stream read
/// failures to [`DecodeError::Truncated`]; bounds mirror [`decode_wav`].
/// Never panics.
pub fn decode_vorbis(bytes: &[u8]) -> Result<Pcm, DecodeError> {
    if bytes.is_empty() {
        return Err(DecodeError::Empty);
    }
    if bytes.len() > MAX_DECODE_BYTES {
        return Err(DecodeError::TooLarge { bytes: bytes.len() });
    }
    let mut reader = lewton::inside_ogg::OggStreamReader::new(Cursor::new(bytes))
        .map_err(|_| DecodeError::InvalidVorbis)?;
    let channels = reader.ident_hdr.audio_channels;
    if channels == 0 || u16::from(channels) > MAX_CHANNELS {
        return Err(DecodeError::UnsupportedChannels {
            channels: u16::from(channels),
        });
    }
    let rate = reader.ident_hdr.audio_sample_rate;
    if rate == 0 {
        return Err(DecodeError::UnsupportedRate);
    }
    let ch = usize::from(channels);
    let mut out = Vec::new();
    let mut frames: u64 = 0;
    loop {
        match reader.read_dec_packet_itl() {
            Ok(None) => break,
            Ok(Some(packet)) => {
                let add = packet.len() as u64 / ch as u64;
                frames = frames.saturating_add(add);
                if frames > MAX_PCM_FRAMES as u64 {
                    return Err(DecodeError::TooManyFrames { frames });
                }
                out.reserve(packet.len());
                for s in packet {
                    out.push(f32::from(s) / 32_768.0);
                }
            }
            Err(_) => return Err(DecodeError::Truncated),
        }
    }
    if out.is_empty() {
        return Err(DecodeError::InvalidVorbis);
    }
    Ok(Pcm::new(rate, u16::from(channels), out))
}

/// Load the arena cue table from `root` (the `content/audio` directory):
/// every listed clip is read and decoded to `sample_rate` (OGG Vorbis with
/// [`decode_vorbis`], any other name with [`decode_wav`]), so the game plays
/// cues by name and the mixer shares each clip's samples. A file that is
/// missing or does not decode is listed in the [`LoadReport`] and its cue
/// keeps its other takes, or falls back to a synthesized voice when none
/// load; the bank is never left without a cue.
///
/// # Errors
///
/// A [`CueError`] when the assembled table is invalid (never for the shipped
/// table).
pub fn load_cue_bank(root: &Path, sample_rate: u32) -> Result<(CueBank, LoadReport), CueError> {
    CueBank::build(sample_rate, |file| {
        let bytes = std::fs::read(root.join(file)).ok()?;
        let vorbis = Path::new(file)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("ogg"));
        if vorbis {
            decode_vorbis(&bytes).ok()
        } else {
            decode_wav(&bytes).ok()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode `samples` as WAV in memory (test-only encoder via hound).
    fn encode_wav(spec: hound::WavSpec, samples: &[i32], float_samples: &[f32]) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut writer =
                hound::WavWriter::new(&mut cursor, spec).expect("in-test wav header encodes");
            if spec.sample_format == hound::SampleFormat::Float {
                for &s in float_samples {
                    writer
                        .write_sample(s)
                        .expect("in-test float sample encodes");
                }
            } else {
                for &s in samples {
                    writer.write_sample(s).expect("in-test int sample encodes");
                }
            }
            writer.finalize().expect("in-test wav finalizes");
        }
        cursor.into_inner()
    }

    #[test]
    fn wav_roundtrip_int16_mono() {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 8_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let wave: Vec<i32> = (0..800)
            .map(|i| {
                ((2.0 * std::f32::consts::PI * 440.0 * i as f32 / 8_000.0).sin() * 20_000.0) as i32
            })
            .collect();
        let bytes = encode_wav(spec, &wave, &[]);
        let pcm = decode_wav(&bytes).expect("in-test wav decodes");
        assert_eq!((pcm.sample_rate, pcm.channels), (8_000, 1));
        assert_eq!(pcm.frames(), wave.len());
        for (got, want) in pcm.samples.iter().zip(wave.iter()) {
            assert!(
                (got - *want as f32 / 32_768.0).abs() < 1e-6,
                "got={got} want={want}"
            );
        }
    }

    #[test]
    fn wav_roundtrip_float32_stereo() {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let pair = [0.5f32, -0.5, 1.0, -1.0, 0.0, 0.25, -0.25, 0.125];
        let bytes = encode_wav(spec, &[], &pair);
        let pcm = decode_wav(&bytes).expect("stereo float wav decodes");
        assert_eq!((pcm.sample_rate, pcm.channels), (48_000, 2));
        assert_eq!(pcm.frames(), pair.len() / 2);
        assert_eq!(pcm.samples, pair);
        assert_eq!(pcm.stereo_at(0), (0.5, -0.5));
    }

    #[test]
    fn oversize_decode_rejected() {
        let big = vec![0u8; MAX_DECODE_BYTES + 1];
        assert_eq!(
            decode_wav(&big),
            Err(DecodeError::TooLarge { bytes: big.len() })
        );
        assert_eq!(
            decode_vorbis(&big),
            Err(DecodeError::TooLarge { bytes: big.len() })
        );
        assert_eq!(decode_wav(&[]), Err(DecodeError::Empty));
        assert_eq!(decode_vorbis(&[]), Err(DecodeError::Empty));
    }

    #[test]
    fn truncated_and_garbage_decode_rejected() {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 8_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let full = encode_wav(spec, &[0, 1000, -1000, 32767, -32768], &[]);
        assert!(full.len() > 16);
        let cut = &full[..full.len() / 2];
        assert!(decode_wav(cut).is_err(), "halved wav body must fail");
        let garbage = b"not audio at all, just text bytes................";
        assert!(decode_wav(garbage).is_err());
        assert!(decode_vorbis(garbage).is_err());
        assert!(decode_vorbis(&full).is_err(), "wav bytes are not vorbis");
        assert!(!decode_wav(garbage).unwrap_err().to_string().is_empty());
    }

    #[test]
    fn unsupported_channels_rejected() {
        let spec = hound::WavSpec {
            channels: 3,
            sample_rate: 8_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let bytes = encode_wav(spec, &[0, 0, 0, 0, 0, 0], &[]);
        assert_eq!(
            decode_wav(&bytes),
            Err(DecodeError::UnsupportedChannels { channels: 3 })
        );
    }

    #[test]
    fn lying_header_frame_count_rejected() {
        // Minimal PCM header declaring more frames than MAX_PCM_FRAMES, then
        // no body: the pre-read bound must fire before any big allocation.
        let declared = MAX_PCM_FRAMES as u32 + 1;
        let mut hdr = Vec::new();
        hdr.extend_from_slice(b"RIFF");
        hdr.extend_from_slice(&36u32.to_le_bytes());
        hdr.extend_from_slice(b"WAVEfmt ");
        hdr.extend_from_slice(&16u32.to_le_bytes());
        hdr.extend_from_slice(&1u16.to_le_bytes());
        hdr.extend_from_slice(&1u16.to_le_bytes());
        hdr.extend_from_slice(&8_000u32.to_le_bytes());
        hdr.extend_from_slice(&16_000u32.to_le_bytes());
        hdr.extend_from_slice(&2u16.to_le_bytes());
        hdr.extend_from_slice(&16u16.to_le_bytes());
        hdr.extend_from_slice(b"data");
        hdr.extend_from_slice(&declared.saturating_mul(2).to_le_bytes());
        assert_eq!(
            decode_wav(&hdr),
            Err(DecodeError::TooManyFrames {
                frames: u64::from(declared)
            })
        );
    }

    /// A decoded clip is only honest when its shape holds: mono/stereo,
    /// nonzero rate, bounded frames, whole interleaved frames, finite audio.
    fn check_pcm_shape(pcm: &Pcm, what: &str) {
        assert!(
            (1..=MAX_CHANNELS).contains(&pcm.channels),
            "{what} decoded {pcm:?}: channels outside 1..={MAX_CHANNELS}"
        );
        assert_ne!(pcm.sample_rate, 0, "{what} decoded {pcm:?}: zero rate");
        assert!(
            pcm.frames() <= MAX_PCM_FRAMES,
            "{what} decoded {} frames past the {MAX_PCM_FRAMES} cap",
            pcm.frames()
        );
        assert!(
            !pcm.is_empty(),
            "{what} decoded an empty clip (decoders reject empties)"
        );
        assert_eq!(
            pcm.samples.len() % usize::from(pcm.channels),
            0,
            "{what} decoded a partial trailing frame"
        );
        assert!(
            pcm.samples.iter().all(|s| s.is_finite()),
            "{what} decoded non-finite audio"
        );
    }

    /// A rejection is only honest when it names a typed [`DecodeError`].
    fn check_typed_rejection(error: &DecodeError, what: &str) {
        assert!(
            matches!(
                error,
                DecodeError::Empty
                    | DecodeError::TooLarge { .. }
                    | DecodeError::UnsupportedChannels { .. }
                    | DecodeError::UnsupportedRate
                    | DecodeError::TooManyFrames { .. }
                    | DecodeError::InvalidWav
                    | DecodeError::Truncated
                    | DecodeError::InvalidVorbis
            ),
            "{what} rejected with an untyped error: {error:?}"
        );
        assert!(
            error.to_string().starts_with("decode: "),
            "{what} rejection carries no decode context: {error}"
        );
    }

    #[test]
    fn adversarial_bytes_never_panic() {
        // Byte-level mutations over a tiny wav plus pure noise: every input
        // must decode to a well-shaped clip or return a typed rejection,
        // never panic. Ok and Err alike are checked, so a wrong-accept (a
        // corrupt header decoding to garbage) or a wrong-error fails here.
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 8_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let base = encode_wav(spec, &[1, -2, 3, -4], &[]);
        // Pinned corpus: the untouched base is four mono frames at 8 kHz.
        let clean = decode_wav(&base).expect("pinned base wav decodes");
        assert_eq!(clean.sample_rate, 8_000, "pinned base rate");
        assert_eq!(clean.channels, 1, "pinned base channels");
        assert_eq!(clean.frames(), 4, "pinned base frames");
        assert_eq!(
            decode_wav(&[]),
            Err(DecodeError::Empty),
            "empty wav pins the Empty rejection"
        );
        assert_eq!(
            decode_vorbis(&[]),
            Err(DecodeError::Empty),
            "empty vorbis pins the Empty rejection"
        );
        for i in 0..base.len() {
            let mut mutated = base.clone();
            mutated[i] ^= 0xFF;
            let what = format!("wav mutation byte {i}");
            match decode_wav(&mutated) {
                Ok(pcm) => check_pcm_shape(&pcm, &what),
                Err(error) => check_typed_rejection(&error, &what),
            }
            let what = format!("vorbis mutation byte {i}");
            match decode_vorbis(&mutated) {
                Ok(pcm) => check_pcm_shape(&pcm, &what),
                Err(error) => check_typed_rejection(&error, &what),
            }
        }
        for len in [1usize, 2, 3, 4, 7, 12, 44, 45, 100] {
            let noise: Vec<u8> = (0..len).map(|i| (i.wrapping_mul(251) >> 1) as u8).collect();
            let what = format!("{len}-byte noise");
            match decode_wav(&noise) {
                Ok(pcm) => check_pcm_shape(&pcm, &what),
                Err(error) => check_typed_rejection(&error, &what),
            }
            match decode_vorbis(&noise) {
                Ok(pcm) => check_pcm_shape(&pcm, &what),
                Err(error) => check_typed_rejection(&error, &what),
            }
        }
    }

    fn content_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/audio")
    }

    /// Fresh scratch directory under the system temp dir.
    fn scratch_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("engine-audio-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir is creatable");
        dir
    }

    #[test]
    fn repo_clips_load_into_the_arena_cues() {
        use crate::cue::{names, ARENA_CUES};
        let (bank, report) = load_cue_bank(&content_root(), 48_000).expect("table builds");
        assert!(
            report.missing_files.is_empty(),
            "{:?}",
            report.missing_files
        );
        assert_eq!(report.loaded_files, 54);
        assert_eq!(
            report.synthesized,
            vec![
                names::IMPACT_BONE,
                names::STUMP,
                names::STRUCTURE_GROAN,
                names::RIPOSTE,
                names::SEVER,
                names::GORE,
                names::SWING,
                names::STINGER_KILL,
                names::STINGER_ROAR
            ]
        );
        let takes = |name: &str| {
            bank.cue(bank.id(name).expect("cue"))
                .expect("id")
                .variants
                .len()
        };
        assert_eq!(takes(names::HIT), 3);
        assert_eq!(takes(names::KNOCKDOWN), 4);
        assert_eq!(takes(names::FOOTSTEP), 4);
        assert_eq!(takes(names::PARRY), 4);
        assert_eq!(takes(names::BLOCK), 4);
        assert_eq!(takes(names::CROWD), 2);
        assert_eq!(takes(names::CROWD_CHEER), 2);
        for spec in &ARENA_CUES {
            let cue = bank.cue(bank.id(spec.name).expect("cue")).expect("id");
            // A take is resampled to the output rate, or, when it is longer
            // than the stream window (the crowd bed loops are 8-25 s), kept at
            // its own rate and streamed.
            assert!(cue.variants.iter().all(|c| {
                c.frames() > 0
                    && (c.sample_rate == 48_000 || crate::cue::should_stream(c.frames(), 1, 0))
            }));
        }
        let hit = bank.cue(bank.id(names::HIT).expect("hit")).expect("cue");
        assert_eq!(
            hit.variants[0].frames(),
            19_200,
            "flesh-small is 0.4 s at 48 kHz"
        );
        assert!(
            hit.variants[0].samples.iter().any(|&s| s.abs() > 0.05),
            "recorded, not silent"
        );
    }

    #[test]
    fn missing_root_synthesizes_every_cue() {
        let (bank, report) =
            load_cue_bank(Path::new("does/not/exist"), 48_000).expect("table builds");
        assert_eq!(report.loaded_files, 0);
        assert_eq!(report.missing_files.len(), 54);
        assert_eq!(report.synthesized.len(), crate::cue::ARENA_CUES.len());
        assert_eq!(bank.len(), crate::cue::ARENA_CUES.len());
    }

    /// Every clip the six new rows name exists on disk, decodes and reports
    /// its own rate and length (the "loud" landing, 2026-10-24): the parry
    /// and clash rows play the CC0 StarNinjas clashes, the block and armour
    /// rows the Kenney wood and plate thuds, the crowd bed and `crowd_cheer`
    /// the CC0 Freesound recordings. No audio device is opened.
    #[test]
    fn the_new_cc0_takes_decode_and_their_cues_pick_them() {
        use crate::cue::{names, ARENA_CUES};
        let root = content_root();
        let rows = [
            names::PARRY,
            names::CLASH,
            names::BLOCK,
            names::IMPACT_ARMOR,
            names::CROWD,
            names::CROWD_CHEER,
        ];
        let mut printed = 0usize;
        for spec in ARENA_CUES.iter().filter(|spec| rows.contains(&spec.name)) {
            assert!(!spec.files.is_empty(), "{} must name files", spec.name);
            for file in spec.files {
                let bytes = std::fs::read(root.join(file))
                    .unwrap_or_else(|error| panic!("{file}: not on disk: {error}"));
                let pcm = if file.ends_with(".ogg") {
                    decode_vorbis(&bytes)
                } else {
                    decode_wav(&bytes)
                }
                .unwrap_or_else(|error| panic!("{file}: does not decode: {error}"));
                let frames = u64::try_from(pcm.frames()).unwrap_or(0);
                let ms = frames * 1_000 / u64::from(pcm.sample_rate);
                println!(
                    "{file}: {} Hz, {} ch, {frames} frames, {ms} ms",
                    pcm.sample_rate, pcm.channels
                );
                assert!(pcm.sample_rate >= 8_000, "{file}: an audio rate");
                if spec.name == names::CROWD || spec.name == names::CROWD_CHEER {
                    assert_eq!(pcm.sample_rate, 22_050, "{file}: converted at 22.05 kHz");
                }
                assert!(pcm.frames() > 0, "{file}: silent clip");
                assert!(
                    pcm.samples.iter().any(|sample| sample.abs() > 0.01),
                    "{file}: recorded, not a silent buffer"
                );
                printed += 1;
            }
        }
        assert_eq!(printed, 18, "six rows, eighteen takes");
        // The bank the window builds from the real folder keeps every take of
        // these rows, so the cue picks the decoded file and never the synth.
        let (bank, report) = load_cue_bank(&root, 48_000).expect("table builds");
        assert!(
            report.missing_files.is_empty(),
            "{:?}",
            report.missing_files
        );
        for name in rows {
            assert!(
                !report.synthesized.contains(&name),
                "{name} fell back to the synth"
            );
            let cue = bank.cue(bank.id(name).expect("cue")).expect("id");
            // A short take is resampled to the output rate; a long bed (the
            // crowd loops are 8-25 s) keeps its own rate and streams, so only
            // the frames are pinned here.
            assert!(
                cue.variants.iter().all(|clip| clip.frames() > 0),
                "{name}: every take decoded"
            );
            assert!(
                !cue.variants.is_empty(),
                "{name}: the recorded takes replaced the synth"
            );
        }
    }

    #[test]
    fn undecodable_file_is_reported_and_other_takes_survive() {
        let root = scratch_dir("undecodable");
        std::fs::create_dir_all(root.join("impacts")).expect("dir");
        std::fs::write(
            root.join("impacts/flesh-small.wav"),
            b"not a wav at all, just text",
        )
        .expect("write garbage");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 8_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let wave: Vec<i32> = (0..800).map(|i| (i % 50) * 400).collect();
        std::fs::write(
            root.join("impacts/flesh-large.wav"),
            encode_wav(spec, &wave, &[]),
        )
        .expect("write wav");
        let (bank, report) = load_cue_bank(&root, 48_000).expect("table builds");
        let _ = std::fs::remove_dir_all(&root);
        assert!(report.missing_files.contains(&"impacts/flesh-small.wav"));
        assert_eq!(
            report.loaded_files, 2,
            "flesh-large feeds both hit and knockdown"
        );
        let hit = bank.cue(bank.id("hit").expect("hit")).expect("cue");
        assert_eq!(hit.variants.len(), 1, "only the good take is kept");
        assert_eq!(hit.variants[0].sample_rate, 48_000, "resampled on load");
        assert_eq!(
            hit.variants[0].frames(),
            4_800,
            "800 frames at 8 kHz become 4800"
        );
    }

    #[test]
    fn eg21_pack_long_bed_streams() {
        let pcm = Pcm::new(48_000, 1, vec![0.0; 200_000]);
        let first = pcm_should_stream(&pcm, 2);
        let second = pcm_should_stream(&pcm, 2);
        assert!(first, "long bed past the frame cap streams");
        assert_eq!(first, second, "same inputs give the same bool");
    }
}
