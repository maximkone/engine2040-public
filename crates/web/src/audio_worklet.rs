//! A5: the page's audio sink is an `AudioWorklet` fed by the engine's own mixer.
//!
//! The page owns one [`AudioEngine`] (the same mixer the native device drains) and renders fixed
//! blocks with it into a ring of [`WORKLET_BLOCK_FRAMES`]-frame stereo blocks. A tiny worklet
//! processor (`crates/web/audio-worklet.js`, shipped inside the wasm bundle as a blob module)
//! pulls those blocks in the audio thread and returns each spent block to the page for a refill.
//! The ring travels by batched `postMessage`, not `SharedArrayBuffer`: a shared ring needs
//! cross-origin isolation and an atomics wasm build, and the page deliberately runs on a plain
//! host.
//!
//! This module is pure: the report the page prints, the underrun bookkeeping, the scripted fight
//! the headless check plays, and the counters it compares. The DOM half lives in `browser`
//! (wasm32 only). The receipt keeps the two required fields exact:
//!
//! - worklet: `audio: state=running underruns=<n>`
//! - fallback: `audio: state=running worklet=fallen-back reason="<err>" underruns=<n>`

use std::sync::Arc;

use engine_audio::combat_cues::{cue_for_event, footstep, CuePlay, Surface};
use engine_audio::{AudioEngine, CueBank, CueError, FeedTx, FightCue, SpatialMixer, FEED_CAPACITY};

/// Frames in one ring block. 1024 at 48 kHz is about 21 ms: big enough that the message rate
/// stays near 50 a second, small enough that a cue starts within a block or two of playout.
pub const WORKLET_BLOCK_FRAMES: u32 = 1_024;

/// Blocks the ring holds when full (about 128 ms at 48 kHz). The page renders every block it has
/// a free buffer for, so steady state is a full ring.
pub const RING_BLOCKS: usize = 6;

/// Queued blocks below which the processor asks the page for more (`RING_BLOCKS / 2`).
pub const RING_LOW_WATER: u32 = 3;

/// The name the worklet registers and the page constructs.
pub const PROCESSOR: &str = "engine2040-sink";

/// Variant seed, the arena's own. The same script picks the same takes.
pub const AUDIO_SEED: u64 = 2_040;

/// How a `reason="..."` field is quoted: one line, no embedded double quote, at most this many
/// characters (then `...`).
pub const REASON_CHARS: usize = 160;

/// The module a `?worklet=broken` page asks for: no such file, so the browser refuses it and the
/// page takes the real fallback path with the browser's own error text. Relative to the page, so
/// it fails the same way on any loopback port.
pub const BROKEN_MODULE: &str = "./audio-worklet-missing.js";

/// Which module load the page attempts on its first gesture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorkletMode {
    /// Load the processor shipped in the wasm bundle (blob URL); fall back on any refusal.
    #[default]
    Auto,
    /// Skip the worklet and play the mixer through the script node (a forced, deliberate fallback).
    Off,
    /// Ask for [`BROKEN_MODULE`], so the browser's own 404 refusal is printed.
    Broken,
}

/// Read the `?worklet=` flag from a search string. Anything else is [`WorkletMode::Auto`], so a
/// typo never changes how the page plays.
#[must_use]
pub fn worklet_mode(search: &str) -> WorkletMode {
    for pair in search.trim_start_matches('?').split('&') {
        if let Some(value) = pair.strip_prefix("worklet=") {
            return match value {
                "off" => WorkletMode::Off,
                "broken" => WorkletMode::Broken,
                _ => WorkletMode::Auto,
            };
        }
    }
    WorkletMode::Auto
}

/// A sample rate the browser reported, in whole hertz. Browsers report whole hertz; the decimal
/// text parses exactly, so the value never needs a lossy `as` cast. Anything unusable (a NaN or a
/// rate too low for the cue table) is the browser's usual 48 kHz.
#[must_use]
pub fn rate_hz(reported: f32) -> u32 {
    if !reported.is_finite() || reported < 8_000.0 {
        return 48_000;
    }
    reported.to_string().parse().unwrap_or(48_000)
}

/// A JavaScript count as a Rust count. `Reflect::get` hands every number over as `f64`; the
/// processor only posts whole non-negative counts, so the decimal text parses exactly and no
/// lossy `as` cast is needed. A negative or non-finite value is 0.
#[must_use]
pub fn count_u64(value: f64) -> u64 {
    if !value.is_finite() || value < 0.0 {
        return 0;
    }
    value.to_string().parse().unwrap_or(0)
}

/// Which sink plays the page's sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Path {
    /// The `AudioWorklet` processor pulled the mixer's blocks from the ring.
    Worklet,
    /// The worklet module did not load (or the page asked it not to): the deprecated
    /// `ScriptProcessorNode` plays the same mixer instead.
    Script,
    /// Neither: there is no sound, and the report says why.
    None,
}

/// What the page prints about the audio sink: the path taken, why it fell back, and the
/// consumer-side counters. The underrun total is monotonic: [`Self::observe`] keeps the larger
/// count, so an out-of-order message never lowers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SinkReport {
    path: Path,
    reason: Option<String>,
    underruns: u64,
    blocks: u64,
}

impl SinkReport {
    /// Nothing is built yet (before the first gesture, or while the module loads).
    #[must_use]
    pub const fn pending() -> Self {
        Self {
            path: Path::None,
            reason: None,
            underruns: 0,
            blocks: 0,
        }
    }

    /// The worklet loaded and pulls the ring.
    #[must_use]
    pub const fn worklet() -> Self {
        Self {
            path: Path::Worklet,
            reason: None,
            underruns: 0,
            blocks: 0,
        }
    }

    /// The worklet module was refused; the script node plays the same mixer.
    #[must_use]
    pub fn fallen_back(reason: impl Into<String>) -> Self {
        Self {
            path: Path::Script,
            reason: Some(reason.into()),
            underruns: 0,
            blocks: 0,
        }
    }

    /// No sink at all (no context, no bank, no node).
    #[must_use]
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            path: Path::None,
            reason: Some(reason.into()),
            underruns: 0,
            blocks: 0,
        }
    }

    /// The path this report names.
    #[must_use]
    pub const fn path(&self) -> Path {
        self.path
    }

    /// The consumer's underrun total.
    #[must_use]
    pub const fn underruns(&self) -> u64 {
        self.underruns
    }

    /// Blocks the page has rendered and posted to the worklet.
    #[must_use]
    pub const fn blocks(&self) -> u64 {
        self.blocks
    }

    /// The reason the fallback was taken; `None` on the worklet path.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    /// Take a consumer's underrun total. Returns true when the printed total changed.
    pub fn observe(&mut self, underruns: u64) -> bool {
        let was = self.underruns;
        self.underruns = self.underruns.max(underruns);
        self.underruns != was
    }

    /// One block went to the consumer.
    pub fn sent(&mut self) {
        self.blocks = self.blocks.saturating_add(1);
    }

    /// The `#audio` line. `state` is the browser's own word ([`crate::audio::AudioGate::label`]).
    /// The worklet path is the exact `audio: state=<s> underruns=<n>`; a fallback keeps both
    /// required fields and adds `worklet=` and the quoted reason.
    #[must_use]
    pub fn line(&self, state: &str) -> String {
        match self.path {
            Path::Worklet => format!("audio: state={state} underruns={}", self.underruns),
            Path::Script => format!(
                "audio: state={state} worklet=fallen-back reason=\"{}\" underruns={}",
                quote_reason(self.reason.as_deref().unwrap_or("unknown")),
                self.underruns
            ),
            Path::None => {
                if self.reason.is_none() {
                    // Nothing tried yet: keep the pre-existing suspended line shape.
                    format!("audio: state={state}")
                } else {
                    format!(
                        "audio: state={state} worklet=none reason=\"{}\" underruns={}",
                        quote_reason(self.reason.as_deref().unwrap_or("unknown")),
                        self.underruns
                    )
                }
            }
        }
    }

    /// The `#audioworklet` line: which sink plays, how many blocks it took, the counters.
    #[must_use]
    pub fn path_line(&self) -> String {
        match self.path {
            Path::Worklet => format!(
                "audio: path=worklet blocks={} underruns={}",
                self.blocks, self.underruns
            ),
            Path::Script => format!(
                "audio: path=script reason=\"{}\" underruns={}",
                quote_reason(self.reason.as_deref().unwrap_or("unknown")),
                self.underruns
            ),
            Path::None => match self.reason.as_deref() {
                Some(reason) => format!(
                    "audio: path=none reason=\"{}\" underruns={}",
                    quote_reason(reason),
                    self.underruns
                ),
                None => "audio: path=none".to_string(),
            },
        }
    }
}

/// Quote a failure in one short line: the first line of the text, whitespace collapsed, double
/// quotes made single (the field is quoted), control characters dropped, cut at [`REASON_CHARS`].
#[must_use]
pub fn quote_reason(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(REASON_CHARS));
    let mut last_space = false;
    for ch in text.chars().take(REASON_CHARS) {
        let ch = match ch {
            '"' => '\'',
            '\n' | '\r' | '\t' => ' ',
            c if c.is_control() => continue,
            c => c,
        };
        if ch == ' ' {
            if last_space || out.is_empty() {
                continue;
            }
            last_space = true;
        } else {
            last_space = false;
        }
        out.push(ch);
    }
    if text.chars().count() > REASON_CHARS {
        out.push_str("...");
    }
    let trimmed = out.trim_end().to_string();
    if trimmed.is_empty() {
        "unknown".to_string()
    } else {
        trimmed
    }
}

// ---- the mixer the page plays through ----

/// Build the page's mixer: the arena cue table synthesized at the context's sample rate, behind
/// the same feed the native device drains. The page queues cue plays; each render applies them.
///
/// # Errors
///
/// A [`engine_audio::cue::CueError`] when the shipped cue table does not validate (never).
pub fn build_mixer(sample_rate: u32) -> Result<(FeedTx, AudioEngine), CueError> {
    // Below a telephone rate the synthesized takes would be unusable; the browser never asks.
    let rate = if sample_rate >= 8_000 {
        sample_rate
    } else {
        48_000
    };
    let bank = Arc::new(CueBank::placeholder(rate)?);
    Ok(AudioEngine::with_feed(
        bank,
        FEED_CAPACITY,
        rate,
        AUDIO_SEED,
    ))
}

/// The scripted fight: one cue of each kind the arena's match can raise, in the order a short
/// exchange produces them. `StaminaOut` is silent by design and yields no play.
#[must_use]
pub fn cue_script() -> [FightCue; 6] {
    [
        FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 120,
        },
        FightCue::Blocked {
            attacker: 1,
            victim: 0,
            chip: 24,
        },
        FightCue::Parried {
            attacker: 0,
            victim: 1,
        },
        FightCue::Dodged {
            attacker: 1,
            victim: 0,
        },
        FightCue::Knockdown {
            attacker: 0,
            victim: 1,
        },
        FightCue::StaminaOut { fighter: 0 },
    ]
}

/// Where the scripted blow lands: the victim about three metres from the listener, inside the
/// cue falloff's full-gain radius, so nothing is culled for distance.
pub const CUE_ATTACKER_CM: [i32; 3] = [120, 0, 0];
/// See [`CUE_ATTACKER_CM`].
pub const CUE_VICTIM_CM: [i32; 3] = [340, 0, 120];

/// The cue plays the scripted fight asks for, in order (a silent event yields none).
#[must_use]
pub fn cue_plays() -> Vec<CuePlay> {
    let mut plays: Vec<CuePlay> = cue_script()
        .into_iter()
        .filter_map(|event| cue_for_event(event, CUE_ATTACKER_CM, CUE_VICTIM_CM))
        .collect();
    plays.push(footstep(Surface::Sand, [200, 0, 60]));
    plays
}

/// Queue the scripted fight on the mixer's feed. Returns how many plays were accepted: the
/// number of cues a correct sink must start.
pub fn queue_cue_script(tx: &mut FeedTx) -> u64 {
    let mut expected = 0;
    for (tick, play) in cue_plays().into_iter().enumerate() {
        let tick = u32::try_from(tick).unwrap_or(0);
        if tx.play(tick, play.name, play.gain, play.pos_cm).is_ok() {
            expected += 1;
        }
    }
    expected
}

/// The mixer's voice counters at one instant, so a proof can compare a delta around a script.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MixerSnapshot {
    /// Cue voices started.
    pub started: u64,
    /// Cue plays refused.
    pub rejected: u64,
    /// Plays culled past their falloff max.
    pub culled: u64,
}

impl MixerSnapshot {
    /// Read the counters from the live mixer.
    #[must_use]
    pub fn read(mixer: &SpatialMixer) -> Self {
        let stats = mixer.mixer_stats();
        Self {
            started: stats.started,
            rejected: stats.rejected,
            culled: mixer.culled(),
        }
    }

    /// The proof of `expected` cues between this snapshot and `after`.
    #[must_use]
    pub fn proof(self, expected: u64, after: Self) -> CueProof {
        CueProof {
            expected,
            played: after.started.saturating_sub(self.started),
            culled: after.culled.saturating_sub(self.culled),
            rejected: after.rejected.saturating_sub(self.rejected),
        }
    }
}

/// What the scripted fight did, from the mixer's own counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CueProof {
    /// Cues the script queued.
    pub expected: u64,
    /// Cues the mixer started a voice for.
    pub played: u64,
    /// Plays culled past their falloff max.
    pub culled: u64,
    /// Plays the mixer refused (a full group, nothing weaker to steal).
    pub rejected: u64,
}

impl CueProof {
    /// Cues that never sounded.
    #[must_use]
    pub const fn missing(&self) -> u64 {
        self.expected.saturating_sub(self.played)
    }

    /// True when every expected cue started and none was culled or refused.
    #[must_use]
    pub const fn ok(&self) -> bool {
        self.missing() == 0 && self.culled == 0 && self.rejected == 0
    }

    /// The page line: `audio: cues expected=<n> played=<n> culled=<n> rejected=<n>`.
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "audio: cues expected={} played={} culled={} rejected={}",
            self.expected, self.played, self.culled, self.rejected
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_audio::FrameSource;

    const RATE: u32 = 8_000;

    #[test]
    fn the_worklet_line_is_the_exact_receipt_with_its_underrun_total() {
        let mut report = SinkReport::worklet();
        assert_eq!(report.line("running"), "audio: state=running underruns=0");
        report.observe(3);
        assert_eq!(report.line("running"), "audio: state=running underruns=3");
        assert_eq!(
            report.line("suspended"),
            "audio: state=suspended underruns=3"
        );
        assert_eq!(
            report.path_line(),
            "audio: path=worklet blocks=0 underruns=3"
        );
        report.sent();
        report.sent();
        assert_eq!(
            report.path_line(),
            "audio: path=worklet blocks=2 underruns=3"
        );
    }

    #[test]
    fn a_fallback_keeps_both_required_fields_and_quotes_the_reason() {
        let mut report = SinkReport::fallen_back("AbortError: the module was refused");
        assert_eq!(
            report.line("running"),
            "audio: state=running worklet=fallen-back reason=\"AbortError: the module was refused\" underruns=0"
        );
        assert_eq!(
            report.path_line(),
            "audio: path=script reason=\"AbortError: the module was refused\" underruns=0"
        );
        // The script node cannot starve: it never has a queue to run dry. The total stays 0.
        report.observe(9);
        assert_eq!(report.underruns(), 9, "a consumer total is always taken");
    }

    #[test]
    fn no_sink_says_none_and_the_reason() {
        assert_eq!(
            SinkReport::pending().line("suspended"),
            "audio: state=suspended"
        );
        let mut report = SinkReport::unavailable("the context threw");
        assert_eq!(
            report.line("suspended"),
            "audio: state=suspended worklet=none reason=\"the context threw\" underruns=0"
        );
        report.observe(1);
        assert_eq!(
            report.path_line(),
            "audio: path=none reason=\"the context threw\" underruns=1"
        );
    }

    #[test]
    fn an_underrun_total_only_grows_and_reports_its_own_change() {
        let mut report = SinkReport::worklet();
        assert!(report.observe(4));
        assert!(!report.observe(4), "the same total is not a change");
        assert!(!report.observe(2), "a stale message does not lower it");
        assert_eq!(report.underruns(), 4);
        assert!(report.observe(5));
        assert_eq!(report.underruns(), 5);
    }

    #[test]
    fn a_reason_is_one_short_line_without_a_quoting_quote() {
        assert_eq!(
            quote_reason("AbortError: \"x\"\n  and a line"),
            "AbortError: 'x' and a line"
        );
        assert_eq!(quote_reason("   "), "unknown");
        assert_eq!(quote_reason("a\tb\u{7}c"), "a bc");
        let long = "x".repeat(REASON_CHARS + 20);
        let cut = quote_reason(&long);
        assert_eq!(cut.chars().count(), REASON_CHARS + 3);
        assert!(cut.ends_with("..."));
        // No newline or quote ever lands inside the field.
        let messy = quote_reason(&format!("\"{}\"\r\n", "y\n".repeat(400)));
        assert!(!messy.contains('\n') && !messy.contains('\r') && !messy.contains('"'));
    }

    #[test]
    fn the_scripted_fight_is_the_arena_cue_set_plus_a_footstep() {
        let plays = cue_plays();
        let names: Vec<&str> = plays.iter().map(|p| p.name).collect();
        assert_eq!(
            names,
            vec!["hit", "block", "parry", "swing", "knockdown", "footstep"],
            "one cue per fight event kind, the silent one skipped"
        );
        // Six events, five audible: StaminaOut queues nothing.
        assert_eq!(cue_script().len(), 6);
        assert_eq!(plays.len(), 6);
    }

    #[test]
    fn every_scripted_cue_starts_a_voice_and_none_is_culled_or_refused() {
        let (mut tx, mut engine) = build_mixer(RATE).expect("the shipped bank is valid");
        let before = MixerSnapshot::read(engine.mixer());
        let expected = queue_cue_script(&mut tx);
        assert_eq!(expected, 6, "every scripted play fits the feed");
        // The render call is what applies the feed; drain is its exposed seam.
        assert_eq!(engine.drain(), 6);
        let proof = before.proof(expected, MixerSnapshot::read(engine.mixer()));
        assert_eq!(
            proof,
            CueProof {
                expected: 6,
                played: 6,
                culled: 0,
                rejected: 0
            }
        );
        assert!(proof.ok());
        assert_eq!(
            proof.line(),
            "audio: cues expected=6 played=6 culled=0 rejected=0"
        );
        // And the mix is audible, not just counted.
        let mut out = vec![0.0_f32; 4_000];
        engine.render_stereo_into(&mut out);
        let peak = out.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.001, "peak {peak}");
        // The mixer's own counters agree after the render too.
        let after = MixerSnapshot::read(engine.mixer());
        assert_eq!(after.started, 6);
        assert_eq!(after.culled, 0);
        assert_eq!(after.rejected, 0);
    }

    #[test]
    fn a_silent_event_queues_nothing_and_a_flat_play_has_no_position() {
        assert!(cue_for_event(
            FightCue::StaminaOut { fighter: 0 },
            CUE_ATTACKER_CM,
            CUE_VICTIM_CM
        )
        .is_none());
        let step = footstep(Surface::Sand, [1, 2, 3]);
        assert_eq!(step.pos_cm, Some([1, 2, 3]));
    }

    #[test]
    fn the_worklet_flag_only_reads_its_two_known_values() {
        assert_eq!(worklet_mode(""), WorkletMode::Auto);
        assert_eq!(worklet_mode("?worklet=off"), WorkletMode::Off);
        assert_eq!(worklet_mode("?a=1&worklet=broken&b=2"), WorkletMode::Broken);
        assert_eq!(worklet_mode("?worklet=nonsense"), WorkletMode::Auto);
        assert_eq!(worklet_mode("?worklet="), WorkletMode::Auto);
        assert_eq!(worklet_mode("?other=off"), WorkletMode::Auto);
        assert_eq!(WorkletMode::default(), WorkletMode::Auto);
    }

    #[test]
    fn a_reported_rate_becomes_whole_hertz_without_a_cast() {
        assert_eq!(rate_hz(44_100.0), 44_100);
        assert_eq!(rate_hz(48_000.0), 48_000);
        assert_eq!(rate_hz(22_050.0), 22_050);
        assert_eq!(rate_hz(f32::NAN), 48_000);
        assert_eq!(rate_hz(0.0), 48_000);
        assert_eq!(rate_hz(-1.0), 48_000);
    }

    #[test]
    fn a_javascript_count_reads_as_a_rust_count() {
        assert_eq!(count_u64(0.0), 0);
        assert_eq!(count_u64(7.0), 7);
        assert_eq!(count_u64(1_234_567.0), 1_234_567);
        assert_eq!(count_u64(-3.0), 0);
        assert_eq!(count_u64(f64::NAN), 0);
        assert_eq!(count_u64(2.5), 0, "the processor posts whole counts only");
    }
}
