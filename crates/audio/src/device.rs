//! device: default cpal output pulling stereo frames from a [`FrameSource`].
//!
//! [`DeviceOutput::open_default`] opens the default output device, builds a
//! stream in the device's preferred sample format and moves the source into
//! the audio callback, which owns it outright: no lock, no allocation (the
//! callback renders through one scratch buffer made before the stream
//! starts). The game talks to an [`AudioEngine`](crate::AudioEngine) source
//! through its lock-free feed. All open failures are typed [`DeviceError`]
//! (no device, no config, unsupported format, open/play backend errors) and
//! never panic; headless CI simply observes `Err` and moves on. Backend
//! stream errors are counted ([`DeviceOutput::stream_errors`]), never
//! swallowed.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::{clip_sample, AudioEngine, CueBank, FeedTx, FrameSource};

/// Cap for backend error strings carried in [`DeviceError`].
pub const MAX_BACKEND_MSG: usize = 256;
/// Frames the callback renders per [`FrameSource`] call; larger device
/// buffers are filled in chunks of this size.
pub const CALLBACK_CHUNK_FRAMES: usize = 1_024;

/// Typed device-open failure; every variant is a clean rejection, never a
/// panic. `Open`/`Play` carry a length-capped backend message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeviceError {
    /// No default output device on this host.
    NoDevice,
    /// Device reports no usable output config.
    NoConfig,
    /// Device's preferred sample format is not one this stub builds
    /// (f32/i16/u16); carries the format name.
    UnsupportedFormat {
        /// Backend sample-format name.
        format: String,
    },
    /// Stream construction failed; carries a capped backend message.
    Open(String),
    /// Constructed stream refused to play; carries a capped backend message.
    Play(String),
}

impl std::fmt::Display for DeviceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoDevice => write!(f, "device: no default output device"),
            Self::NoConfig => write!(f, "device: no supported output config"),
            Self::UnsupportedFormat { format } => {
                write!(f, "device: unsupported sample format {format}")
            }
            Self::Open(msg) => write!(f, "device: open failed: {msg}"),
            Self::Play(msg) => write!(f, "device: play failed: {msg}"),
        }
    }
}

impl std::error::Error for DeviceError {}

/// Deterministic `f32` (`[-1, 1]`, NaN-free) to device-sample mapping for
/// the formats this stub builds. Hand-rolled so the mapping is explicit and
/// unit-tested; `as` casts saturate, the clamp states the range.
trait ToDeviceSample {
    /// Map one clipped mix sample to the device format.
    fn from_mix(v: f32) -> Self;
}

impl ToDeviceSample for f32 {
    fn from_mix(v: f32) -> Self {
        v
    }
}

impl ToDeviceSample for i16 {
    fn from_mix(v: f32) -> Self {
        (v * 32_767.0)
            .round()
            .clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16
    }
}

impl ToDeviceSample for u16 {
    fn from_mix(v: f32) -> Self {
        ((v * 0.5 + 0.5) * 65_535.0).round().clamp(0.0, 65_535.0) as u16
    }
}

/// Bound a backend message so errors stay small and printable.
fn cap_message(msg: &str) -> String {
    let mut out = msg.chars().take(MAX_BACKEND_MSG).collect::<String>();
    if out.len() < msg.len() {
        out.push_str("...");
    }
    out
}

/// Map one mixer stereo frame onto output `channel` of a `channels`-wide
/// device frame: channel 0 is left, 1 is right, anything else (including mono
/// devices) folds to the mid. Pure helper so the mapping is unit-testable
/// without hardware.
fn map_channels(left: f32, right: f32, channel: usize, channels: usize) -> f32 {
    if channels <= 1 {
        return (left + right) * 0.5;
    }
    match channel {
        0 => left,
        1 => right,
        _ => (left + right) * 0.5,
    }
}

/// Fill one device buffer from `source`: render stereo into `scratch` a
/// chunk at a time, then spread each stereo frame over `channels` device
/// channels. A trailing partial device frame is still filled. Allocates
/// nothing, so it is safe on the audio thread.
fn fill_device_buffer<T: ToDeviceSample>(
    source: &mut impl FrameSource,
    scratch: &mut [f32],
    data: &mut [T],
    channels: usize,
) {
    let chunk_frames = scratch.len() / 2;
    if chunk_frames == 0 {
        for slot in data.iter_mut() {
            *slot = T::from_mix(0.0);
        }
        return;
    }
    for chunk in data.chunks_mut(chunk_frames * channels) {
        let block = &mut scratch[..chunk.len().div_ceil(channels) * 2];
        source.render_stereo_into(block);
        for (frame, pair) in chunk.chunks_mut(channels).zip(block.chunks_exact(2)) {
            for (c, slot) in frame.iter_mut().enumerate() {
                *slot = T::from_mix(clip_sample(map_channels(pair[0], pair[1], c, channels)));
            }
        }
    }
}

/// Owned snapshot of one output device's default config (cpal enumerate
/// shape: `format` is the sample-format name). Pure data, so the pick
/// rule below orders snapshots without touching hardware.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeInfo {
    /// Device channel count of the default config.
    pub channels: u16,
    /// Device sample rate of the default config, in Hz.
    pub sample_rate: u32,
    /// Backend sample-format name.
    pub format: String,
}

/// Rank one backend format name (`f32`, then `i16`, then `u16` beat anything
/// exotic), so an unbuildable default never wins over a buildable one.
fn probe_format_rank(format: &str) -> u8 {
    match format {
        "f32" => 0,
        "i16" => 1,
        "u16" => 2,
        _ => 3,
    }
}

/// Best-effort census of every output device's default config: failed
/// queries are skipped, the vec caps at 16, and headless hosts yield an
/// empty vec. Never panics; [`DeviceOutput::open_default`] stays the opener.
pub fn probe_outputs() -> Vec<ProbeInfo> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let mut out = Vec::new();
    let devices = match cpal::default_host().output_devices() {
        Ok(devices) => devices,
        Err(_) => return out,
    };
    for device in devices.take(16) {
        let Ok(supported) = device.default_output_config() else {
            continue;
        };
        let config = supported.config();
        out.push(ProbeInfo {
            channels: config.channels,
            sample_rate: config.sample_rate,
            format: supported.sample_format().to_string(),
        });
    }
    out
}

/// Index pick over `cands`: stereo first, then format rank (`f32`/`i16`/`u16`),
/// then 48 kHz; ties keep the first, empty is a typed refusal (`None`).
/// Integer-plus-rank math only, so it is unit-testable without hardware.
pub fn pick_probe_config(cands: &[ProbeInfo]) -> Option<usize> {
    cands
        .iter()
        .enumerate()
        .min_by_key(|(_, info)| {
            (
                u8::from(info.channels != 2),
                probe_format_rank(&info.format),
                u8::from(info.sample_rate != 48_000),
            )
        })
        .map(|(index, _)| index)
}

/// Live output stream handle. Dropping it stops device playback and drops
/// the source with it (an [`AudioEngine`](crate::AudioEngine)'s feed then
/// reports closed, so the game can rebuild the pair and reopen).
pub struct DeviceOutput {
    _stream: cpal::Stream,
    channels: u16,
    sample_rate: u32,
    errors: Arc<AtomicU64>,
}

impl DeviceOutput {
    /// Sample rate of the default output device, so the cue bank can be
    /// built at the rate the stream will run at (the mixer plays one sample
    /// per output frame).
    ///
    /// # Errors
    ///
    /// [`DeviceError::NoDevice`] or [`DeviceError::NoConfig`] on hosts
    /// without a usable output.
    pub fn default_rate() -> Result<u32, DeviceError> {
        use cpal::traits::{DeviceTrait, HostTrait};
        let device = cpal::default_host()
            .default_output_device()
            .ok_or(DeviceError::NoDevice)?;
        let supported = device
            .default_output_config()
            .map_err(|_| DeviceError::NoConfig)?;
        Ok(supported.config().sample_rate)
    }

    /// Open the probed-best output device and start pulling stereo frames
    /// from `source`, which the audio callback now owns. The census from
    /// [`probe_outputs`] picks the device through [`pick_probe_config`]
    /// (stereo, then buildable format, then 48 kHz); a missing or stale
    /// pick falls back to the default device. Returns [`DeviceError`]
    /// on headless machines instead of panicking. Never panics.
    pub fn open_default<S: FrameSource>(source: S) -> Result<Self, DeviceError> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let host = cpal::default_host();
        let mut probed: Option<cpal::Device> = None;
        if let Some(picked) = pick_probe_config(&probe_outputs()) {
            if let Ok(mut devices) = host.output_devices() {
                probed = devices.nth(picked);
            }
        }
        let device = probed
            .or_else(|| host.default_output_device())
            .ok_or(DeviceError::NoDevice)?;
        let supported = device
            .default_output_config()
            .map_err(|_| DeviceError::NoConfig)?;
        let format = supported.sample_format();
        let config = supported.config();
        if config.channels == 0 {
            return Err(DeviceError::NoConfig);
        }
        let errors = Arc::new(AtomicU64::new(0));
        let stream = match format {
            cpal::SampleFormat::F32 => {
                build_stream::<f32, S>(&device, &config, source, Arc::clone(&errors))
            }
            cpal::SampleFormat::I16 => {
                build_stream::<i16, S>(&device, &config, source, Arc::clone(&errors))
            }
            cpal::SampleFormat::U16 => {
                build_stream::<u16, S>(&device, &config, source, Arc::clone(&errors))
            }
            other => Err(DeviceError::UnsupportedFormat {
                format: other.to_string(),
            }),
        }?;
        stream
            .play()
            .map_err(|e| DeviceError::Play(cap_message(&e.to_string())))?;
        Ok(Self {
            _stream: stream,
            channels: config.channels,
            sample_rate: config.sample_rate,
            errors,
        })
    }

    /// Device channel count backing this stream.
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Device sample rate backing this stream.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Backend stream errors seen so far (device lost, xrun reports). A
    /// rising count means the stream needs reopening.
    pub fn stream_errors(&self) -> u64 {
        self.errors.load(Ordering::Relaxed)
    }
}

/// Where an engine's frames go: a live device stream, or the caller's own
/// offline loop when there is no device (muted, headless, or the open failed).
pub enum AudioSink {
    /// A live stream owns the engine and pulls frames itself.
    Device(DeviceOutput),
    /// The caller owns the engine and renders it with [`Self::pump_offline`].
    Offline(AudioEngine),
}

impl AudioSink {
    /// Build the engine and its game-side [`FeedTx`], then try to hand the
    /// engine to the default device unless `muted`. A failed open falls back
    /// to an offline engine over the same bank and returns the typed error,
    /// so the caller can print its notice; the returned feed always matches
    /// the returned sink. `sample_rate` must match the bank's clips (pick it
    /// with [`DeviceOutput::default_rate`] before loading them).
    pub fn open(
        bank: &Arc<CueBank>,
        capacity: usize,
        sample_rate: u32,
        seed: u64,
        muted: bool,
    ) -> (FeedTx, Self, Option<DeviceError>) {
        let mut refusal = None;
        if !muted {
            let (tx, engine) =
                AudioEngine::with_feed(Arc::clone(bank), capacity, sample_rate, seed);
            match DeviceOutput::open_default(engine) {
                Ok(output) => return (tx, Self::Device(output), None),
                Err(err) => refusal = Some(err),
            }
        }
        let (tx, engine) = AudioEngine::with_feed(Arc::clone(bank), capacity, sample_rate, seed);
        (tx, Self::Offline(engine), refusal)
    }

    /// True when a device stream is playing.
    pub fn is_live(&self) -> bool {
        matches!(self, Self::Device(_))
    }

    /// Render one buffer of an offline engine into `scratch` (interleaved
    /// stereo, the result is discarded) so its voices and feed keep moving at
    /// the sim's pace; a live stream pumps itself and this does nothing.
    pub fn pump_offline(&mut self, scratch: &mut [f32]) {
        if let Self::Offline(engine) = self {
            engine.render_stereo_into(scratch);
        }
    }

    /// Backend stream errors of the live device, `0` offline.
    pub fn stream_errors(&self) -> u64 {
        match self {
            Self::Device(output) => output.stream_errors(),
            Self::Offline(_) => 0,
        }
    }
}

/// Build one typed output stream whose callback owns `source`. The scratch
/// buffer is made here, before the stream runs, so the callback itself never
/// allocates; backend errors only bump `errors`.
fn build_stream<T, S>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut source: S,
    errors: Arc<AtomicU64>,
) -> Result<cpal::Stream, DeviceError>
where
    T: cpal::SizedSample + ToDeviceSample,
    S: FrameSource,
{
    use cpal::traits::DeviceTrait;
    let channels = usize::from(config.channels.max(1));
    let mut scratch = vec![0.0f32; CALLBACK_CHUNK_FRAMES * 2];
    device
        .build_output_stream(
            *config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                fill_device_buffer(&mut source, &mut scratch, data, channels);
            },
            move |_| {
                errors.fetch_add(1, Ordering::Relaxed);
            },
            None,
        )
        .map_err(|e| DeviceError::Open(cap_message(&e.to_string())))
}

/// Offline loudness report over interleaved stereo, donor `levels` shape.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Levels {
    /// Largest absolute sample, 0..1.
    pub peak: f32,
    /// RMS of the left channel.
    pub rms_left: f32,
    /// RMS of the right channel.
    pub rms_right: f32,
    /// Samples at full scale.
    pub clipped: u64,
}

/// Peak, per-ear RMS and clipped count of interleaved stereo `samples`.
pub fn levels(samples: &[f32]) -> Levels {
    let mut peak = 0.0f32;
    let (mut sl, mut sr) = (0.0f32, 0.0f32);
    let mut clipped = 0u64;
    for (i, &s) in samples.iter().enumerate() {
        peak = peak.max(s.abs());
        if s.abs() >= 0.9999 {
            clipped += 1;
        }
        if i % 2 == 0 {
            sl += s * s;
        } else {
            sr += s * s;
        }
    }
    let frames = crate::f32_of(u32::try_from(samples.len() / 2).unwrap_or(1)).max(1.0);
    Levels { peak, rms_left: (sl / frames).sqrt(), rms_right: (sr / frames).sqrt(), clipped }
}

impl FrameSource for crate::Mixer {
    fn render_stereo_into(&mut self, out: &mut [f32]) {
        crate::Mixer::render_stereo_into(self, out);
    }
}

/// Render the current mix offline to 16-bit PCM WAV bytes plus loudness.
///
/// Caller: offline export only (tests plus a future arena capture path);
/// the live cpal callback is untouched. Applies the traced room (direct
/// gain plus first echoes scaled by closedness) so the bytes match what the
/// room measures. PORT(lunaris@56cc7e30:lib.rs `levels`, wav write, device render).
pub fn record_audio<S: FrameSource>(
    source: &mut S,
    sample_rate: u32,
    frames: usize,
    scene: &crate::trace::Scene,
    listener: crate::trace::V3,
    source_pos: crate::trace::V3,
) -> (Vec<u8>, Levels) {
    let mut buf = vec![0.0f32; frames.saturating_mul(2)];
    source.render_stereo_into(&mut buf);
    let room = crate::trace::analyse_room(scene, listener, 64, 16);
    let ac = crate::trace::trace_source(scene, listener, source_pos, 32, 0.1);
    let settings = crate::trace::RoomSettings { rt60: room.rt60, damping: 0.5, wet: room.closedness, predelay: 0.0 };
    let wet = settings.wet.clamp(0.0, 1.0);
    let dg = ac.direct_gain.clamp(0.0, 1.0);
    for s in buf.iter_mut() {
        *s = clip_sample(*s * dg);
    }
    let rate_f = crate::f32_of(sample_rate.max(1));
    for tap in ac.taps.iter() {
        let want = tap.delay * rate_f;
        let mut off = 0usize;
        while crate::f32_of(u32::try_from(off).unwrap_or(u32::MAX)) < want && off < buf.len() {
            off += 1;
        }
        let g = (tap.gain * wet).clamp(0.0, 1.0);
        let mut k = 0usize;
        while off + k * 2 + 1 < buf.len() {
            let at = k * 2;
            if at + 1 < buf.len() {
                buf[off + k * 2] = clip_sample(buf[off + k * 2] + buf[at] * g * 0.5);
                buf[off + k * 2 + 1] = clip_sample(buf[off + k * 2 + 1] + buf[at + 1] * g * 0.5);
            }
            k += 1;
            if k >= buf.len() {
                break;
            }
        }
    }
    let report = levels(&buf);
    (encode_wav16(&buf, sample_rate), report)
}

fn encode_wav16(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let data_bytes = u32::try_from(samples.len().saturating_mul(2)).unwrap_or(u32::MAX);
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&data_bytes.saturating_add(36).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&sample_rate.max(1).to_le_bytes());
    out.extend_from_slice(&sample_rate.max(1).saturating_mul(4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes.to_le_bytes());
    for &s in samples {
        out.extend_from_slice(&i16::from_mix(clip_sample(s)).to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_audio_writes_wav_with_loudness_report() {
        use crate::trace::{Occluder, Scene, Shape};
        let wall = |c: [f32; 3], h: [f32; 3]| Occluder {
            shape: Shape::Box { center: c, half: h },
            absorption: 0.1,
            transmission: 0.0,
        };
        let scene = Scene::new(vec![
            wall([0.0, -2.6, 0.0], [4.2, 0.1, 2.2]),
            wall([0.0, 2.6, 0.0], [4.2, 0.1, 2.2]),
            wall([-4.1, 0.0, 0.0], [0.1, 2.5, 2.0]),
            wall([4.1, 0.0, 0.0], [0.1, 2.5, 2.0]),
            wall([0.0, 0.0, -2.1], [4.0, 2.5, 0.1]),
            wall([0.0, 0.0, 2.1], [4.0, 2.5, 0.1]),
        ]);
        let run = || {
            let mut mixer = crate::Mixer::new(48_000);
            let pcm = crate::Pcm::new(48_000, 1, vec![0.4; 4_800]);
            mixer.play(&pcm, 1.0);
            record_audio(&mut mixer, 48_000, 4_800, &scene, [0.0, 0.0, 0.0], [2.0, 0.0, 0.0])
        };
        let (a_bytes, a_rep) = run();
        let (b_bytes, b_rep) = run();
        assert_eq!(a_bytes, b_bytes, "offline bytes are deterministic");
        assert_eq!(a_rep, b_rep, "report repeats");
        assert!(a_rep.peak <= 1.0 && a_rep.peak > 0.0, "peak={}", a_rep.peak);
        assert!(a_rep.rms_left.is_finite() && a_rep.rms_right.is_finite(), "rms finite");
        let pcm = crate::decode_wav(&a_bytes).expect("wav parses as 16-bit PCM");
        assert_eq!((pcm.sample_rate, pcm.channels), (48_000, 2));
        assert_eq!(pcm.frames(), 4_800);
    }

    #[test]
    fn channel_mapping_folds_without_hardware() {
        let approx =
            |got: f32, want: f32| assert!((got - want).abs() < 1e-6, "got={got} want={want}");
        approx(map_channels(0.6, -0.2, 0, 2), 0.6);
        approx(map_channels(0.6, -0.2, 1, 2), -0.2);
        approx(map_channels(0.6, -0.2, 0, 1), 0.2);
        approx(map_channels(0.6, -0.2, 4, 6), 0.2);
        approx(map_channels(0.6, -0.2, 0, 0), 0.2);
    }

    fn probe_candidate(channels: u16, sample_rate: u32, format: &str) -> ProbeInfo {
        ProbeInfo {
            channels,
            sample_rate,
            format: format.to_string(),
        }
    }

    #[test]
    fn probe_pick_prefers_stereo_48k_f32() {
        // Accept list: mono 44100 i16, stereo 44100 i16, stereo 48000 f32/u16.
        let cands = vec![
            probe_candidate(1, 44_100, "i16"),
            probe_candidate(2, 44_100, "i16"),
            probe_candidate(2, 48_000, "f32"),
            probe_candidate(2, 48_000, "u16"),
        ];
        assert_eq!(pick_probe_config(&cands), Some(2));
        let dropped = vec![cands[0].clone(), cands[1].clone(), cands[3].clone()];
        assert_eq!(pick_probe_config(&dropped), Some(1));
        assert_eq!(pick_probe_config(&[]), None);
    }

    #[test]
    fn probe_pick_mono_falls_back_and_ties_keep_first() {
        // No stereo: 48k f32 still beats 44.1k i16 on a mono-only host.
        let mono = vec![
            probe_candidate(1, 44_100, "i16"),
            probe_candidate(1, 48_000, "f32"),
        ];
        assert_eq!(pick_probe_config(&mono), Some(1));
        // Identical twins: the first keeps the seat.
        let twins = vec![
            probe_candidate(2, 48_000, "f32"),
            probe_candidate(2, 48_000, "f32"),
        ];
        assert_eq!(pick_probe_config(&twins), Some(0));
        // Exotic backend formats rank below the three this stub builds.
        let odd = vec![
            probe_candidate(2, 48_000, "dsdu16"),
            probe_candidate(2, 48_000, "f32"),
        ];
        assert_eq!(pick_probe_config(&odd), Some(1));
    }

    #[test]
    fn probe_pick_outputs_never_panics_headless() {
        // Headless CI has no output device: the census comes back a vec
        // (maybe empty) instead of panicking, and a pick over it is typed.
        let found = probe_outputs();
        assert!(found.len() <= 16, "census is capped, got {}", found.len());
        let _ = pick_probe_config(&found);
    }

    #[test]
    fn device_sample_mapping_exact() {
        assert_eq!(f32::from_mix(0.5), 0.5);
        assert_eq!(i16::from_mix(1.0), 32_767);
        assert_eq!(i16::from_mix(-1.0), -32_767);
        assert_eq!(i16::from_mix(0.0), 0);
        assert_eq!(u16::from_mix(0.0), 32_768);
        assert_eq!(u16::from_mix(1.0), 65_535);
        assert_eq!(u16::from_mix(-1.0), 0);
    }

    #[test]
    fn device_errors_are_typed_and_printable() {
        for e in [
            DeviceError::NoDevice,
            DeviceError::NoConfig,
            DeviceError::UnsupportedFormat {
                format: "dsdu8".to_string(),
            },
            DeviceError::Open("busy".to_string()),
            DeviceError::Play("gone".to_string()),
        ] {
            assert!(!e.to_string().is_empty(), "{e:?} must describe itself");
        }
        assert!(cap_message(&"x".repeat(1024)).len() <= MAX_BACKEND_MSG + 3);
    }

    /// Source that numbers its frames: left `n/100`, right `-n/100`.
    struct Counter {
        next: u16,
        calls: usize,
    }

    impl FrameSource for Counter {
        fn render_stereo_into(&mut self, out: &mut [f32]) {
            self.calls += 1;
            for pair in out.chunks_exact_mut(2) {
                let v = f32::from(self.next) / 100.0;
                pair[0] = v;
                pair[1] = -v;
                self.next += 1;
            }
        }
    }

    #[test]
    fn fill_renders_in_chunks_and_keeps_frame_order() {
        // Scratch holds 2 frames; the device wants 5 stereo frames.
        let mut source = Counter { next: 1, calls: 0 };
        let mut scratch = [0.0f32; 4];
        let mut data = [9.0f32; 10];
        fill_device_buffer(&mut source, &mut scratch, &mut data, 2);
        assert_eq!(source.calls, 3, "2 + 2 + 1 frames");
        let want: Vec<f32> = (1..=5u16)
            .flat_map(|n| [f32::from(n) / 100.0, -f32::from(n) / 100.0])
            .collect();
        assert_eq!(data.to_vec(), want);
    }

    #[test]
    fn fill_spreads_stereo_over_wide_and_mono_devices() {
        let mut scratch = [0.0f32; 8];
        let mut six = [9.0f32; 12];
        fill_device_buffer(
            &mut Counter { next: 10, calls: 0 },
            &mut scratch,
            &mut six,
            6,
        );
        let (l, r, mid) = (0.10, -0.10, 0.0);
        assert_eq!(six[..6].to_vec(), vec![l, r, mid, mid, mid, mid]);
        assert_eq!(six[6], 0.11);
        let mut mono = [9.0f32; 3];
        fill_device_buffer(
            &mut Counter { next: 10, calls: 0 },
            &mut scratch,
            &mut mono,
            1,
        );
        assert_eq!(mono, [0.0, 0.0, 0.0], "mono folds L and R to the mid");
    }

    #[test]
    fn fill_fills_a_trailing_partial_frame_and_converts_formats() {
        let mut scratch = [0.0f32; 8];
        let mut data = [0i16; 7];
        fill_device_buffer(
            &mut Counter { next: 50, calls: 0 },
            &mut scratch,
            &mut data,
            2,
        );
        assert_eq!(data[0], i16::from_mix(0.5));
        assert_eq!(data[1], i16::from_mix(-0.5));
        assert_eq!(
            data[6],
            i16::from_mix(0.53),
            "partial frame gets its left sample"
        );
        let mut mute = [7.0f32; 6];
        fill_device_buffer(&mut Counter { next: 1, calls: 0 }, &mut [], &mut mute, 2);
        assert_eq!(mute, [0.0; 6], "no scratch means silence, not a panic");
    }

    #[test]
    fn engine_in_the_callback_allocates_nothing() {
        let bank = Arc::new(CueBank::placeholder(48_000).expect("shipped table"));
        let (mut tx, mut engine) = AudioEngine::with_feed(bank, 64, 48_000, 11);
        let mut scratch = vec![0.0f32; CALLBACK_CHUNK_FRAMES * 2];
        let mut data = vec![0.0f32; 1_920 * 2];
        // Warm once (first voices, listener), then measure the steady state.
        tx.listener(0, [0, 300, 900], 0).expect("room");
        tx.play(0, crate::cue_names::HIT, 1.0, Some([100, 100, 0]))
            .expect("room");
        fill_device_buffer(&mut engine, &mut scratch, &mut data, 2);
        let ((), heap) = engine_core::alloc_probe::measure(|| {
            for t in 1..20 {
                tx.play(t, crate::cue_names::HIT, 1.0, Some([100, 100, 0]))
                    .ok();
                tx.play(t, crate::cue_names::CROWD, 0.5, None).ok();
                fill_device_buffer(&mut engine, &mut scratch, &mut data, 2);
            }
        });
        assert_eq!(heap.calls, 0, "device callback path: {} bytes", heap.bytes);
        assert!(data.iter().any(|&s| s != 0.0), "the engine was audible");
    }

    #[test]
    fn muted_sink_is_offline_and_keeps_the_engine_moving() {
        let bank = Arc::new(CueBank::placeholder(48_000).expect("shipped table"));
        let (mut tx, mut sink, refusal) = AudioSink::open(&bank, 16, 48_000, 5, true);
        assert!(refusal.is_none(), "muted is a choice, not a failure");
        assert!(!sink.is_live());
        assert_eq!(sink.stream_errors(), 0);
        tx.play(0, crate::cue_names::HIT, 1.0, None).expect("room");
        let mut scratch = vec![0.0f32; 512];
        sink.pump_offline(&mut scratch);
        assert!(
            scratch.iter().any(|&s| s != 0.0),
            "offline engine rendered the hit"
        );
        assert_eq!(tx.stats().applied, 1);
    }

    #[test]
    fn unmuted_sink_reports_why_it_fell_back() {
        // Either a device exists (live, no refusal) or it does not (offline
        // plus the typed reason); never a panic, and the feed always works.
        let bank = Arc::new(CueBank::placeholder(48_000).expect("shipped table"));
        let (mut tx, mut sink, refusal) = AudioSink::open(&bank, 16, 48_000, 5, false);
        assert_eq!(sink.is_live(), refusal.is_none());
        tx.listener(0, [0, 0, 0], 0).expect("feed matches the sink");
        sink.pump_offline(&mut [0.0f32; 8]);
    }

    #[test]
    fn open_attempt_never_panics_without_audio() {
        // No audio assertions: headless CI has no device, so either outcome
        // is fine; the gate is that open errors are typed, never panics.
        let bank = Arc::new(CueBank::placeholder(48_000).expect("shipped table"));
        let (_tx, engine) = AudioEngine::with_feed(bank, 16, 48_000, 1);
        let _ = DeviceOutput::open_default(engine);
        let _ = DeviceOutput::default_rate();
    }

    #[test]
    fn callback_drains_a_thousand_commands_under_contention() {
        // EG21 accept through the real callback helper: a game thread
        // outruns a 64-slot ring while `fill_device_buffer` drains it like
        // the device callback. Every refusal is retried and counted; none
        // is lost (the lossy `try_lock` hand-off this feed replaces went
        // silent here instead).
        use std::thread;
        use std::time::{Duration, Instant};

        use crate::FeedError;
        const COMMANDS: u32 = 1_000;
        const RATE: u32 = 48_000;
        let bank = Arc::new(CueBank::placeholder(RATE).expect("shipped table"));
        let (mut tx, mut engine) = AudioEngine::with_feed(bank, 64, RATE, 2040);
        tx.listener(0, [0, 0, 0], 0).expect("room");
        let producer = thread::spawn(move || {
            let mut refused = 0u64;
            for tick in 1..=COMMANDS {
                let t = i32::try_from(tick).unwrap_or(0);
                while let Err(err) = tx.play(tick, crate::cue_names::HIT, 1.0, Some([t, 100, 0])) {
                    assert_eq!(err, FeedError::Full, "audio side stayed open");
                    refused += 1;
                    thread::yield_now();
                }
            }
            (tx, refused)
        });
        let limit = Instant::now() + Duration::from_secs(30);
        let mut scratch = vec![0.0f32; CALLBACK_CHUNK_FRAMES * 2];
        let mut data = vec![0.0f32; 512 * 2];
        let mut heard = false;
        while engine.applied() < u64::from(COMMANDS) + 1 {
            fill_device_buffer(&mut engine, &mut scratch, &mut data, 2);
            heard |= data.iter().any(|&s| s != 0.0);
            assert!(
                Instant::now() < limit,
                "callback starved at {}",
                engine.applied()
            );
            thread::yield_now();
        }
        let (tx, refused) = producer.join().expect("producer ran");
        fill_device_buffer(&mut engine, &mut scratch, &mut data, 2);
        heard |= data.iter().any(|&s| s != 0.0);
        let stats = tx.stats();
        assert_eq!(stats.sent, u64::from(COMMANDS) + 1);
        assert_eq!(
            (stats.applied, stats.last_applied_tick),
            (stats.sent, COMMANDS)
        );
        assert_eq!(
            stats.dropped_full, refused,
            "refusals are counted, not lost"
        );
        assert!(heard, "hits were audible through the callback");
    }

    #[test]
    fn hit_cue_picks_deterministically_per_seed_through_the_callback() {
        // EG21 accept through the real callback helper: the hit cue is
        // cues-not-clips (3-4 takes) and the same seed replays the same
        // takes, one full voice per render so takes never overlap.
        const RATE: u32 = 48_000;
        const PLAYS: u32 = 24;
        let bank = Arc::new(CueBank::placeholder(RATE).expect("shipped table"));
        let hit = bank
            .cue(bank.id(crate::cue_names::HIT).expect("hit"))
            .expect("hit cue");
        let (takes, frames) = (hit.variants.len(), hit.variants[0].frames());
        assert!(
            (3..=4).contains(&takes),
            "hit carries 3-4 takes, got {takes}"
        );
        let run = |seed: u64| {
            let (mut tx, mut engine) = AudioEngine::with_feed(Arc::clone(&bank), 64, RATE, seed);
            let mut scratch = vec![0.0f32; CALLBACK_CHUNK_FRAMES * 2];
            let mut data = vec![0.0f32; frames * 2];
            let mut heard = Vec::new();
            for t in 0..PLAYS {
                tx.play(t, crate::cue_names::HIT, 1.0, None).expect("room");
                fill_device_buffer(&mut engine, &mut scratch, &mut data, 2);
                heard.push(data[0].to_bits());
            }
            heard
        };
        let a = run(2040);
        assert_eq!(a, run(2040), "same seed replays the same takes");
        assert_ne!(a, run(2041), "another seed picks another sequence");
        let distinct: std::collections::BTreeSet<u32> = a.iter().copied().collect();
        assert_eq!(
            distinct.len(),
            takes,
            "all {takes} takes heard in {PLAYS} plays"
        );
    }

    #[test]
    fn arena_real_bank_plays_hit_block_footstep_and_crowd_through_the_callback() {
        // EG21 real-sound-in-arena, audio-side proof: the window's three
        // placeholder voices (hit, footstep, crowd-bed) resolve to the real
        // `content/audio` bank here. Hit, block and footstep commands plus
        // the camera listener drain through `fill_device_buffer` (the exact
        // helper the device callback runs); the oracle is the feed stats,
        // not a sample count: sent == applied with dropped_full 0, and the
        // rendered frames are audible. No wall clock: a fixed fill count.
        use std::path::Path;

        const RATE: u32 = 48_000;
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/audio");
        let (bank, report) = crate::load_cue_bank(&root, RATE).expect("real clips decode");
        assert!(
            report.missing_files.is_empty(),
            "every listed clip loads: {:?}",
            report.missing_files
        );
        assert!(
            report.loaded_files > 0,
            "the bank is recorded, not synthesized"
        );
        let bank = Arc::new(bank);
        let takes = |name: &str| {
            bank.cue(bank.id(name).expect("cue"))
                .expect("id")
                .variants
                .len()
        };
        assert_eq!(
            takes(crate::cue_names::HIT),
            3,
            "hit carries its 3 recorded takes"
        );
        assert_eq!(
            takes(crate::cue_names::FOOTSTEP),
            4,
            "footstep carries its 4 recorded takes"
        );
        assert_eq!(
            takes(crate::cue_names::BLOCK),
            3,
            "block carries its 3 recorded takes"
        );

        let (mut tx, mut engine) = AudioEngine::with_feed(Arc::clone(&bank), 64, RATE, 2040);
        // The camera is the listener; impacts sound at the victim.
        tx.listener(0, [0, 300, 900], 0).expect("room");
        tx.play(1, crate::cue_names::HIT, 1.0, Some([100, 100, 0]))
            .expect("room");
        tx.play(2, crate::cue_names::BLOCK, 0.9, Some([100, 100, 0]))
            .expect("room");
        tx.play(3, crate::cue_names::FOOTSTEP, 0.8, Some([100, 100, 0]))
            .expect("room");
        tx.play(4, crate::cue_names::CROWD, 0.6, None)
            .expect("room");
        let mut scratch = vec![0.0f32; CALLBACK_CHUNK_FRAMES * 2];
        let mut data = vec![0.0f32; 512 * 2];
        let mut heard = false;
        for _ in 0..96 {
            fill_device_buffer(&mut engine, &mut scratch, &mut data, 2);
            heard |= data.iter().any(|&s| s != 0.0);
        }
        assert!(heard, "real clips were audible through the callback");
        let stats = tx.stats();
        assert_eq!(stats.sent, 5, "listener plus four cue plays");
        assert_eq!(
            (stats.applied, stats.last_applied_tick),
            (stats.sent, 4),
            "every queued command applied in order"
        );
        assert_eq!(stats.dropped_full, 0, "no command lost");
    }
}
