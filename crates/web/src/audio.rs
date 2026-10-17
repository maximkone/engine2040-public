//! Audio on the web (WEB7): when the WebAudio context may start, what the page says about it,
//! and the blocks the sink pulls from the engine's `FrameSource`. All of it is pure so it runs and
//! tests natively; `browser_audio` (wasm only) owns the `AudioContext`.
//!
//! Browsers keep an audio context `suspended` until the page has had a user gesture, and may
//! suspend it again later (an interruption, a tab switch). So the context is not even created
//! before the first gesture, and [`AudioGate`] is the small state machine that decides when to
//! create it and when to ask it to resume. The page line is `audio: state=<suspended|running>`.

use engine_audio::FrameSource;

/// What the browser reports for `AudioContext.state`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CtxState {
    /// No sound is rendered; waiting for a gesture or an interruption to end.
    Suspended,
    /// Rendering.
    Running,
    /// Safari: the OS took the output (a call, the lock screen). Treated like `Suspended`.
    Interrupted,
    /// The context was closed and cannot be used again.
    Closed,
}

impl CtxState {
    /// Parse the string `AudioContext.state` gives; `None` for a word this build does not know.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "suspended" => Some(Self::Suspended),
            "running" => Some(Self::Running),
            "interrupted" => Some(Self::Interrupted),
            "closed" => Some(Self::Closed),
            _ => None,
        }
    }
}

/// Where the gate is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// No gesture yet: there is no context.
    AwaitingGesture,
    /// A context exists and `resume()` was asked for; the browser has not said `running`.
    Starting,
    /// The browser says `running`.
    Running,
    /// The context was running and was suspended, or `resume()` was refused.
    Suspended,
}

/// What the browser glue must do now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    /// Nothing.
    Nothing,
    /// Make the `AudioContext`, connect the sink and call `resume()`. Only ever after a gesture.
    CreateAndResume,
    /// Call `resume()` on the existing context (it is inside a gesture now).
    Resume,
}

/// The resume state machine. Inputs are the user gestures the page sees and the state words the
/// browser reports; outputs are [`Act`]s and the page label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioGate {
    phase: Phase,
    gestures: u32,
}

impl Default for AudioGate {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioGate {
    /// No gesture seen, no context.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            phase: Phase::AwaitingGesture,
            gestures: 0,
        }
    }

    /// The phase now.
    #[must_use]
    pub const fn phase(&self) -> Phase {
        self.phase
    }

    /// User gestures seen so far.
    #[must_use]
    pub const fn gestures(&self) -> u32 {
        self.gestures
    }

    /// True once a context exists (it never exists before the first gesture).
    #[must_use]
    pub const fn has_context(&self) -> bool {
        !matches!(self.phase, Phase::AwaitingGesture)
    }

    /// A user gesture arrived. The first one creates the context; later ones retry `resume()`
    /// while the context is not running, so a refused resume heals on the next click.
    pub fn on_gesture(&mut self) -> Act {
        self.gestures = self.gestures.saturating_add(1);
        match self.phase {
            Phase::AwaitingGesture => {
                self.phase = Phase::Starting;
                Act::CreateAndResume
            }
            Phase::Starting | Phase::Suspended => {
                self.phase = Phase::Starting;
                Act::Resume
            }
            Phase::Running => Act::Nothing,
        }
    }

    /// The browser reported `AudioContext.state` (a `statechange`, or a poll of it).
    pub fn on_state(&mut self, state: CtxState) {
        self.phase = match (self.phase, state) {
            // A report with no context cannot happen; ignore it rather than invent one.
            (Phase::AwaitingGesture, _) => Phase::AwaitingGesture,
            (_, CtxState::Running) => Phase::Running,
            (_, CtxState::Closed) => Phase::AwaitingGesture,
            (Phase::Running, CtxState::Suspended | CtxState::Interrupted) => Phase::Suspended,
            (phase, CtxState::Suspended | CtxState::Interrupted) => phase,
        };
    }

    /// `resume()` was rejected or threw. Back to suspended; the next gesture asks again.
    pub fn on_resume_failed(&mut self) {
        if self.phase == Phase::Starting {
            self.phase = Phase::Suspended;
        }
    }

    /// The word the page prints: `running` only when the browser said so, else `suspended`
    /// (no context yet, resume pending, or suspended again).
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self.phase {
            Phase::Running => "running",
            Phase::AwaitingGesture | Phase::Starting | Phase::Suspended => "suspended",
        }
    }

    /// The page line: `audio: state=<suspended|running>`.
    #[must_use]
    pub fn line(&self) -> String {
        format!("audio: state={}", self.label())
    }
}

/// Whether a `keydown` of this `KeyboardEvent.code` counts as a user activation. Browsers do not
/// count Escape or a lone modifier, so the context must not be started from them.
#[must_use]
pub fn keydown_is_gesture(code: &str) -> bool {
    !matches!(
        code,
        "Escape"
            | "ShiftLeft"
            | "ShiftRight"
            | "ControlLeft"
            | "ControlRight"
            | "AltLeft"
            | "AltRight"
            | "MetaLeft"
            | "MetaRight"
            | "OSLeft"
            | "OSRight"
            | "CapsLock"
            | "ContextMenu"
    )
}

/// Frames per block the sink asks for (`ScriptProcessorNode` buffer size: a power of two from
/// 256 to 16384). 2048 at 48 kHz is about 43 ms: steady on a weak machine, fine for cues.
pub const BLOCK_FRAMES: u32 = 2_048;

/// Pull `left.len()` frames from `source` and split them into the two channel buffers of a
/// WebAudio output block. `scratch` is the interleaved buffer, kept between calls so the audio
/// callback allocates only when the block size grows. Samples are clipped to -1..1 and NaN is
/// silence, as in the native device path.
pub fn render_planar(
    source: &mut dyn FrameSource,
    scratch: &mut Vec<f32>,
    left: &mut [f32],
    right: &mut [f32],
) {
    let frames = left.len().min(right.len());
    scratch.clear();
    scratch.resize(frames * 2, 0.0);
    source.render_stereo_into(scratch);
    for (i, pair) in scratch.chunks_exact(2).enumerate() {
        left[i] = engine_audio::clip_sample(pair[0]);
        right[i] = engine_audio::clip_sample(pair[1]);
    }
}

/// A short soft two-note chime, rendered on the first block after the context starts. It is
/// the proof that sound comes out of the page; the fight cues replace it as the source.
#[derive(Clone, Debug)]
pub struct Chime {
    sample_rate: f32,
    frame: u32,
    total: u32,
}

impl Chime {
    /// Peak level: quiet on purpose (a page that starts sound by itself must not startle).
    pub const LEVEL: f32 = 0.08;

    /// A chime for a context running at `sample_rate` Hz.
    #[must_use]
    pub fn new(sample_rate: f32) -> Self {
        let rate = if sample_rate.is_finite() && sample_rate >= 8_000.0 {
            sample_rate
        } else {
            48_000.0
        };
        // 0.25 s; the cast is of a value in 2_000..=200_000 range for any sane rate.
        let total = (rate * 0.25) as u32;
        Self {
            sample_rate: rate,
            frame: 0,
            total,
        }
    }

    /// True once every frame has been rendered; later blocks are silence.
    #[must_use]
    pub fn done(&self) -> bool {
        self.frame >= self.total
    }
}

impl FrameSource for Chime {
    fn render_stereo_into(&mut self, out: &mut [f32]) {
        for pair in out.chunks_exact_mut(2) {
            let s = if self.frame < self.total {
                let t = self.frame as f32 / self.sample_rate;
                let fade = 1.0 - self.frame as f32 / self.total as f32;
                // A fifth apart: 660 Hz and 990 Hz.
                let wave = (t * 660.0 * std::f32::consts::TAU).sin()
                    + 0.5 * (t * 990.0 * std::f32::consts::TAU).sin();
                self.frame += 1;
                wave * fade * fade * Self::LEVEL / 1.5
            } else {
                0.0
            };
            pair[0] = s;
            pair[1] = s;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_exists_before_a_gesture_and_the_page_says_suspended() {
        let g = AudioGate::new();
        assert_eq!(g.phase(), Phase::AwaitingGesture);
        assert!(!g.has_context());
        assert_eq!(g.line(), "audio: state=suspended");
        assert_eq!(g.gestures(), 0);
    }

    #[test]
    fn the_first_gesture_creates_the_context_and_running_is_the_browsers_word() {
        let mut g = AudioGate::new();
        assert_eq!(g.on_gesture(), Act::CreateAndResume);
        assert!(g.has_context());
        // resume() was asked for but the browser has not said so: still suspended on the page.
        assert_eq!(g.label(), "suspended");
        g.on_state(CtxState::Running);
        assert_eq!(g.phase(), Phase::Running);
        assert_eq!(g.line(), "audio: state=running");
        assert_eq!(g.on_gesture(), Act::Nothing, "already running");
        assert_eq!(g.gestures(), 2);
    }

    #[test]
    fn a_refused_resume_heals_on_the_next_gesture() {
        let mut g = AudioGate::new();
        assert_eq!(g.on_gesture(), Act::CreateAndResume);
        g.on_resume_failed();
        assert_eq!(g.phase(), Phase::Suspended);
        assert_eq!(g.on_gesture(), Act::Resume, "no second context");
        g.on_state(CtxState::Running);
        assert_eq!(g.label(), "running");
    }

    #[test]
    fn an_interruption_suspends_and_the_next_gesture_resumes() {
        let mut g = AudioGate::new();
        g.on_gesture();
        g.on_state(CtxState::Running);
        g.on_state(CtxState::Interrupted);
        assert_eq!(g.phase(), Phase::Suspended);
        assert_eq!(g.label(), "suspended");
        assert_eq!(g.on_gesture(), Act::Resume);
        g.on_state(CtxState::Suspended);
        assert_eq!(g.phase(), Phase::Starting, "still waiting for the browser");
        g.on_state(CtxState::Running);
        assert_eq!(g.phase(), Phase::Running);
    }

    #[test]
    fn a_suspended_report_while_starting_does_not_undo_the_start() {
        let mut g = AudioGate::new();
        g.on_gesture();
        g.on_state(CtxState::Suspended);
        assert_eq!(g.phase(), Phase::Starting);
        g.on_resume_failed();
        g.on_resume_failed();
        assert_eq!(g.phase(), Phase::Suspended);
    }

    #[test]
    fn a_closed_context_is_replaced_by_the_next_gesture() {
        let mut g = AudioGate::new();
        g.on_gesture();
        g.on_state(CtxState::Running);
        g.on_state(CtxState::Closed);
        assert!(!g.has_context());
        assert_eq!(g.label(), "suspended");
        assert_eq!(g.on_gesture(), Act::CreateAndResume);
    }

    #[test]
    fn a_state_report_with_no_context_is_ignored() {
        let mut g = AudioGate::new();
        g.on_state(CtxState::Running);
        assert_eq!(g.phase(), Phase::AwaitingGesture);
        assert_eq!(g.label(), "suspended");
        g.on_resume_failed();
        assert_eq!(g.phase(), Phase::AwaitingGesture);
    }

    #[test]
    fn state_words_parse() {
        for (word, state) in [
            ("suspended", CtxState::Suspended),
            ("running", CtxState::Running),
            ("interrupted", CtxState::Interrupted),
            ("closed", CtxState::Closed),
        ] {
            assert_eq!(CtxState::parse(word), Some(state));
        }
        assert_eq!(CtxState::parse("Running"), None);
        assert_eq!(CtxState::parse(""), None);
    }

    #[test]
    fn escape_and_lone_modifiers_are_not_gestures() {
        for code in ["Escape", "ShiftLeft", "ControlRight", "AltLeft", "MetaLeft"] {
            assert!(!keydown_is_gesture(code), "{code}");
        }
        for code in ["KeyW", "Space", "Enter", "ArrowUp", "KeyF"] {
            assert!(keydown_is_gesture(code), "{code}");
        }
    }

    #[test]
    fn the_chime_is_quiet_audible_then_silent_and_split_into_two_channels() {
        let mut chime = Chime::new(48_000.0);
        let mut scratch = Vec::new();
        let (mut l, mut r) = (vec![9.0_f32; 2_048], vec![9.0_f32; 2_048]);
        render_planar(&mut chime, &mut scratch, &mut l, &mut r);
        assert_eq!(l, r, "mono chime, both channels equal");
        let peak = l.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.01 && peak <= Chime::LEVEL + 1e-6, "peak {peak}");
        assert!(!chime.done());
        // 0.25 s at 48 kHz is 12000 frames; after enough blocks it is over and silent.
        for _ in 0..6 {
            render_planar(&mut chime, &mut scratch, &mut l, &mut r);
        }
        assert!(chime.done());
        assert!(l.iter().chain(r.iter()).all(|s| *s == 0.0));
        // Same input, same samples.
        let (mut a, mut b) = (Chime::new(48_000.0), Chime::new(48_000.0));
        let (mut ba, mut bb) = (vec![0.0_f32; 64], vec![0.0_f32; 64]);
        a.render_stereo_into(&mut ba);
        b.render_stereo_into(&mut bb);
        assert_eq!(ba, bb);
    }

    #[test]
    fn render_planar_clips_and_silences_nan() {
        struct Loud;
        impl FrameSource for Loud {
            fn render_stereo_into(&mut self, out: &mut [f32]) {
                for (i, pair) in out.chunks_exact_mut(2).enumerate() {
                    pair[0] = if i == 0 { 4.0 } else { f32::NAN };
                    pair[1] = -4.0;
                }
            }
        }
        let mut scratch = Vec::new();
        let (mut l, mut r) = ([0.0_f32; 3], [0.0_f32; 3]);
        render_planar(&mut Loud, &mut scratch, &mut l, &mut r);
        assert_eq!(l, [1.0, 0.0, 0.0]);
        assert_eq!(r, [-1.0, -1.0, -1.0]);
        // Uneven channel buffers: the shorter one sets the block.
        let (mut l2, mut r2) = ([7.0_f32; 4], [7.0_f32; 2]);
        render_planar(&mut Loud, &mut scratch, &mut l2, &mut r2);
        assert_eq!(
            l2[2], 7.0,
            "frames past the shorter channel are not touched"
        );
    }

    #[test]
    fn a_bad_rate_falls_back_to_48k() {
        assert_eq!(Chime::new(f32::NAN).total, 12_000);
        assert_eq!(Chime::new(100.0).total, 12_000);
        assert_eq!(Chime::new(44_100.0).total, 11_025);
    }
}
