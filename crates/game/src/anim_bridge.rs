//! anim_bridge: sim state to clip cues with a per-slot crossfade.
//!
//! Consumer: the fighter look reads each slot's
//! [`ClipPick`] plus [`FighterSlot::fade`] in the look fold path
//! (`crates/game/src/fighter_look_acts.rs`) and mirrors the fade count onto
//! its slot. The pose no longer blends the two clips while the fade runs: a
//! clip switch rides on inertial offsets instead
//! (`crates/game/src/fighter_look_inertia.rs`).
//! Data owner: this module owns the act pick and the fade driver; clip files
//! stay with the look, fade math with `engine-anim`. Units: meters per tick
//! in, clip indices plus integer ticks plus Q8 rate out. Determinism: the
//! pick is integers only — same views plus same cues give the same pick
//! twice; speeds and phases stay presentation-side and never enter the tick
//! hash. Errors: [`BridgeError`]; refusals change nothing.

use engine_anim::crossfade::SlotFade;
use engine_anim::state_machine::RATE_Q8_ONE;
use engine_audio::FightCue;
use engine_core::Fnv64;

use crate::FighterView;

/// Clip cue in the look's clip order: idle, run, three swing variants, down.
/// The file order lives with the look; this enum pins the shared indices so a
/// reorder there fails the index test below instead of miscuing silently.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BridgeClip {
    /// Guard stance, looping.
    Idle,
    /// Moving across the sand, looping, speed-scaled.
    Run,
    /// First sword swing.
    Attack0,
    /// Second sword swing.
    Attack1,
    /// Third sword swing.
    Attack2,
    /// Knocked down, held on the last frame.
    Down,
}

impl BridgeClip {
    /// Index in the look's clip order.
    #[must_use]
    pub fn index(self) -> u8 {
        match self {
            Self::Idle => 0,
            Self::Run => 1,
            Self::Attack0 => 2,
            Self::Attack1 => 3,
            Self::Attack2 => 4,
            Self::Down => 5,
        }
    }
}

/// Caller-owned speed tuning: the planar speed above which a fighter reads
/// as running, and the reference speed where run plays at its authored rate.
/// Values live with the look; this struct only carries them, validated, so a
/// retune never forks into a second constant here.
#[derive(Clone, Copy, Debug)]
pub struct BridgeTuning {
    /// Planar meters per tick above which the pick flips to run.
    pub run_speed_m_per_tick: f32,
    /// Planar meters per tick where the run rate reads 1x.
    pub run_ref_m_per_tick: f32,
}

impl BridgeTuning {
    /// Build validated tuning from the caller's speed constants.
    ///
    /// Caller: `crates/game/src/fighter_look_acts.rs` (the fold path drives one
    /// bridge slot per seen fighter). Units: meters per tick in. Determinism:
    /// integers only downstream; this only carries the thresholds. Errors:
    /// [`BridgeError::BadTuning`] naming the offending field; refusals build
    /// nothing.
    pub fn new(run_speed_m_per_tick: f32, run_ref_m_per_tick: f32) -> Result<Self, BridgeError> {
        let tuning = Self {
            run_speed_m_per_tick,
            run_ref_m_per_tick,
        };
        tuning.validate()?;
        Ok(tuning)
    }

    /// Refuse a non-positive or non-finite threshold; the caller keeps its
    /// previous tuning then. Errors: [`BridgeError::BadTuning`] naming the
    /// offending field.
    pub fn validate(&self) -> Result<(), BridgeError> {
        if !self.run_speed_m_per_tick.is_finite() || self.run_speed_m_per_tick <= 0.0 {
            return Err(BridgeError::BadTuning("run_speed_m_per_tick"));
        }
        if !self.run_ref_m_per_tick.is_finite() || self.run_ref_m_per_tick <= 0.0 {
            return Err(BridgeError::BadTuning("run_ref_m_per_tick"));
        }
        Ok(())
    }
}

/// What refused a bridge call. Refusals change nothing.
#[derive(Clone, PartialEq, Debug)]
pub enum BridgeError {
    /// Tuning field failed [`BridgeTuning::validate`]; field name carried.
    BadTuning(&'static str),
    /// View slot differs from the driven slot; culprit carried.
    BadSlot(u8),
}

impl core::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadTuning(field) => write!(f, "anim bridge: tuning {field} is not usable"),
            Self::BadSlot(slot) => write!(f, "anim bridge: view slot {slot} is not this slot"),
        }
    }
}

impl std::error::Error for BridgeError {}

/// Integer cue for one slot: what to play, since when, how fast. The sampler
/// converts the phase to seconds; no float appears here.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ClipPick {
    /// Current clip cue.
    pub clip: BridgeClip,
    /// Tick the current cue started on.
    pub act_start: u32,
    /// Ticks since `act_start` (saturating).
    pub phase_ticks: u32,
    /// Playback rate in Q8 units (256 reads 1x); only run scales.
    pub rate_q8: i64,
}

/// Presentation state for one fighter slot: the integer pick plus the fade
/// driver. The float positions below feed speed only and never enter the
/// pick digest. `Clone` lets the look checkpoint one tick of bridge state for
/// a rollback rewind (EG39).
#[derive(Clone)]
pub struct FighterSlot {
    slot: u8,
    pick: ClipPick,
    pos_m: [f32; 3],
    prev_pos_m: [f32; 3],
    seen: bool,
    swings: u8,
    fade: SlotFade,
    /// First tick the slot's clip clock runs free again (exclusive end of a
    /// hitstop hold). The look sets it from the tick's impact pair; the hold
    /// freezes clip phases only, never sim state. Zero means never held.
    hold_until: u32,
}

impl FighterSlot {
    /// Idle slot that has never seen its fighter.
    pub fn new(slot: u8) -> Self {
        Self {
            slot,
            pick: ClipPick {
                clip: BridgeClip::Idle,
                act_start: 0,
                phase_ticks: 0,
                rate_q8: RATE_Q8_ONE,
            },
            pos_m: [0.0; 3],
            prev_pos_m: [0.0; 3],
            seen: false,
            swings: 0,
            fade: SlotFade::idle(),
            hold_until: 0,
        }
    }

    /// Driven slot id.
    #[must_use]
    pub fn slot(&self) -> u8 {
        self.slot
    }

    /// Current integer cue.
    #[must_use]
    pub fn pick(&self) -> ClipPick {
        self.pick
    }

    /// Leaving clip plus fade position for the sampler.
    #[must_use]
    pub fn fade(&self) -> SlotFade {
        self.fade
    }

    /// Atomic look read: the current cue plus the fade driver together.
    ///
    /// Caller: `crates/game/src/fighter_look_acts.rs` (`fold_acts` drives one
    /// bridge slot per seen fighter, then samples both clips while the fade
    /// runs). Same bytes as `pick` plus `fade` read together, so the look
    /// never mixes a new pick with an old fade.
    #[must_use]
    pub fn look_pick(&self) -> (ClipPick, SlotFade) {
        (self.pick, self.fade)
    }

    /// First tick the slot runs free again (exclusive). The look records the
    /// impact pair's expiry here; `fold_acts` freezes exactly that pair.
    #[must_use]
    #[cfg(feature = "app")]
    pub(crate) fn hold_until(&self) -> u32 {
        self.hold_until
    }

    /// Record a hold through `until` (exclusive). Keeps the farthest expiry,
    /// so a new impact extends the freeze but never shortens it.
    #[cfg(any(feature = "app", test))]
    pub(crate) fn hold_through(&mut self, until: u32) {
        self.hold_until = self.hold_until.max(until);
    }

    /// March the pick clock with the tick while the act stays held: the swing
    /// window survives the freeze, so the resume tick still maps the held
    /// clip instead of an expired swing. The phase stays put while held; the
    /// start marches so the resume tick recomputes the same phase.
    /// Presentation only.
    #[cfg(any(feature = "app", test))]
    pub(crate) fn march_hold(&mut self, elapsed: u32) {
        self.pick.act_start = self.pick.act_start.saturating_add(elapsed);
    }

    /// Fold one tick of sim state into the pick: advance the fade clock,
    /// then fold position, cues and hp. A clip change stashes the leaving
    /// cue and restarts the fade, so the switch frame still shows the old
    /// clip whole; a return to the fading clip cancels the stale fade
    /// instead (Bevy retrigger rule), except around `Down`: a death swap
    /// always blends, so a revival blends from the held last frame. Order
    /// inside is clock, events, pick; the sampler blends after. Errors: [`BridgeError`] on bad tuning or a
    /// foreign view slot; refusals change nothing.
    pub fn observe_tick(
        &mut self,
        tick: u32,
        view: &FighterView,
        hp_permille: u16,
        cues: &[FightCue],
        tuning: &BridgeTuning,
        swing_ticks: u32,
    ) -> Result<ClipPick, BridgeError> {
        tuning.validate()?;
        if view.slot != self.slot {
            return Err(BridgeError::BadSlot(view.slot));
        }
        // A held slot's crossfade waits with its clip clock, so the blend
        // weight on the first free tick is the one the hit froze.
        if tick >= self.hold_until {
            self.fade.tick();
        }
        self.fold_position(view);
        let speed = planar_speed(self.pos_m, self.prev_pos_m);
        let down = hp_permille == 0;
        let striking = !down && strikes_this_tick(self.slot, cues);
        let next = if down {
            BridgeClip::Down
        } else if self.pick.clip == BridgeClip::Down {
            BridgeClip::Idle
        } else if striking {
            let variant = match self.swings % 3 {
                0 => BridgeClip::Attack0,
                1 => BridgeClip::Attack1,
                _ => BridgeClip::Attack2,
            };
            self.swings = self.swings.wrapping_add(1);
            variant
        } else if matches!(
            self.pick.clip,
            BridgeClip::Attack0 | BridgeClip::Attack1 | BridgeClip::Attack2
        ) && tick.saturating_sub(self.pick.act_start) < swing_ticks
        {
            self.pick.clip
        } else if speed > tuning.run_speed_m_per_tick {
            BridgeClip::Run
        } else {
            BridgeClip::Idle
        };
        if next != self.pick.clip {
            // A death swap always blends: leaving or entering `Down` never
            // cancels, so a revival blends from the held last frame instead
            // of cutting (Bevy A-B-A only applies off the death clip).
            let down_swap = next == BridgeClip::Down || self.pick.clip == BridgeClip::Down;
            if down_swap || !retarget_cancel(&mut self.fade, next.index()) {
                self.fade
                    .retarget(self.pick.clip.index(), self.pick.phase_ticks);
            }
            self.pick.clip = next;
            self.pick.act_start = tick;
            self.pick.phase_ticks = 0;
        } else {
            self.pick.phase_ticks = tick.saturating_sub(self.pick.act_start);
        }
        self.pick.rate_q8 = if next == BridgeClip::Run {
            rate_q8_for_speed(speed, tuning.run_ref_m_per_tick)
        } else {
            RATE_Q8_ONE
        };
        Ok(self.pick)
    }

    /// Shift the seen position pair; a non-finite view is skipped, so the
    /// speed reads zero that tick instead of poisoning the pick.
    fn fold_position(&mut self, view: &FighterView) {
        if !view.pos_m.iter().all(|v| v.is_finite()) {
            return;
        }
        if self.seen {
            self.prev_pos_m = self.pos_m;
        } else {
            self.prev_pos_m = view.pos_m;
            self.seen = true;
        }
        self.pos_m = view.pos_m;
    }

    /// FNV over the slot id, the whole integer pick, the swing counter, the
    /// hold expiry and the fade digest: the replay proof. Float positions are
    /// excluded by construction, so the digest never depends on presentation
    /// rounding. The hold expiry enters because it drives the hitstop freeze;
    /// the same pick with a different freeze must not digest the same.
    #[must_use]
    pub fn digest(&self) -> u64 {
        let mut h = Fnv64::new();
        h.write_u8(self.slot);
        h.write_u8(self.pick.clip.index());
        h.write_u32(self.pick.act_start);
        h.write_u32(self.pick.phase_ticks);
        h.write_i64(self.pick.rate_q8);
        h.write_u8(self.swings);
        h.write_u8(u8::from(self.seen));
        h.write_u32(self.hold_until);
        h.write_u64(self.fade.digest());
        h.finish()
    }
}

/// Bevy retrigger rule over the integer fade: when the next clip is the
/// fading one, cancel the stale fade instead of restashing it, so a quick
/// A-B-A return shows the current clip whole. Death swaps never cancel:
/// leaving or entering `Down` always blends (see `observe_tick`). PORT(bevy@0.19.1
/// `AnimationTransitions::play`): the donor keeps `current_weight` from the
/// old weight with `weight_decline_per_sec` at `1/duration`, splits remaining
/// weight greedy newest-first, and expires weight-zero clips; here the
/// integer `fade_left` going to zero is the expiry, and the presentation
/// weight follows from `fade_weight` (a zero total cuts at once). Caller:
/// [`FighterSlot::observe_tick`], checked before it stashes the leaving cue.
/// Integer state only; true when the fade was cancelled.
fn retarget_cancel(fade: &mut SlotFade, next_clip: u8) -> bool {
    if fade.blending() && fade.prev_clip == next_clip {
        fade.fade_left = 0;
        true
    } else {
        false
    }
}

/// True when any cue starts a swing for `slot`: any landed, soaked, parried,
/// dodged or finishing blow names its attacker. Mirrors the look's cue fold.
fn strikes_this_tick(slot: u8, cues: &[FightCue]) -> bool {
    cues.iter().any(|cue| {
        matches!(
            *cue,
            FightCue::Hit { attacker, .. }
                | FightCue::Blocked { attacker, .. }
                | FightCue::Parried { attacker, .. }
                | FightCue::Dodged { attacker, .. }
                | FightCue::Knockdown { attacker, .. }
                if attacker == slot
        )
    })
}

/// Planar meters covered since the previous tick; non-finite input reads as
/// still standing, never as a sprint.
fn planar_speed(pos_m: [f32; 3], prev_pos_m: [f32; 3]) -> f32 {
    let dx = pos_m[0] - prev_pos_m[0];
    let dz = pos_m[2] - prev_pos_m[2];
    let speed = (dx * dx + dz * dz).sqrt();
    if speed.is_finite() {
        speed
    } else {
        0.0
    }
}

/// Run rate in Q8 units from planar speed: speed over the reference reads
/// 1x, clamped to half through double rate. Non-finite speeds hold 1x.
/// Single owner for the Q8 one is the anim state machine; the bounds derive
/// from it, never from a mirrored literal.
fn rate_q8_for_speed(speed_m_per_tick: f32, ref_m_per_tick: f32) -> i64 {
    if !speed_m_per_tick.is_finite() {
        return RATE_Q8_ONE;
    }
    let one = f32::from(u16::try_from(RATE_Q8_ONE).unwrap_or(256));
    let scaled = (speed_m_per_tick / ref_m_per_tick).clamp(0.5, 2.0) * one;
    // Scaled sits in 128..=512 after the clamp; the decode below keeps that
    // float-to-int step exact without a cast.
    q8_from_rounded(scaled.round())
}

/// Whole clamped Q8 rate (`128.0..=512.0`) to its integer step: bit decode
/// keeps every in-range integer exact without a cast, the same shape as the
/// wall-clock quantizer in `engine-core`. Out-of-range input reads as the
/// authored 1x rate, so a missed clamp never miscues the sampler.
fn q8_from_rounded(rounded: f32) -> i64 {
    if !rounded.is_finite() || !(128.0..=512.0).contains(&rounded) {
        return RATE_Q8_ONE;
    }
    let bits = rounded.to_bits();
    let mant = u64::from((bits & 0x007F_FFFF) | 0x0080_0000);
    let exp = (bits >> 23) & 0xFF;
    let shift = match 150u32.checked_sub(exp) {
        Some(shift) if shift <= 16 => shift,
        _ => return RATE_Q8_ONE,
    };
    i64::try_from(mant >> shift).unwrap_or(RATE_Q8_ONE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::alloc_probe::measure;

    // Look-owned speed values, passed in as tuning: the bridge never mirrors
    // them into its own constants.
    fn tuning() -> BridgeTuning {
        BridgeTuning {
            run_speed_m_per_tick: 0.012,
            run_ref_m_per_tick: 0.05,
        }
    }

    fn view(slot: u8, x: f32) -> FighterView {
        FighterView {
            slot,
            pos_m: [x, 0.0, 0.0],
            yaw: 0.0,
        }
    }

    fn hit(attacker: u8, victim: u8) -> FightCue {
        FightCue::Hit {
            attacker,
            victim,
            damage: 100,
        }
    }

    fn tick_march(
        slot: &mut FighterSlot,
        tick: u32,
        x: f32,
        hp: u16,
        cues: &[FightCue],
    ) -> ClipPick {
        slot.observe_tick(tick, &view(0, x), hp, cues, &tuning(), 12)
            .expect("valid march tick")
    }

    #[test]
    fn fighter_anim_bridge_same_tick_same_pick_x2() {
        let cues = [hit(0, 1)];
        let mut first = FighterSlot::new(0);
        let mut second = FighterSlot::new(0);
        for tick in 0..20u32 {
            let x = -1.0 + 0.10 * f32::from(u16::try_from(tick).unwrap_or(u16::MAX));
            let tick_cues: &[FightCue] = if tick == 5 { &cues } else { &[] };
            let a = tick_march(&mut first, tick, x, 1000, tick_cues);
            let b = tick_march(&mut second, tick, x, 1000, tick_cues);
            assert_eq!(a, b, "tick {tick}");
            assert_eq!(first.digest(), second.digest(), "tick {tick}");
        }
    }

    #[test]
    fn fighter_anim_bridge_idle_to_run_crossfades() {
        let mut slot = FighterSlot::new(0);
        assert_eq!(
            tick_march(&mut slot, 0, -1.0, 1000, &[]).clip,
            BridgeClip::Idle
        );
        // One fast step flips the pick and restarts the fade whole.
        let pick = tick_march(&mut slot, 1, -0.90, 1000, &[]);
        assert_eq!(pick.clip, BridgeClip::Run);
        assert_eq!(pick.act_start, 1);
        assert!(slot.fade().blending());
        assert_eq!(slot.fade().prev_clip, BridgeClip::Idle.index());
        // Four ticks later the new-clip weight reads 4/9, never one half:
        // nine ticks are odd, so no frame lands exactly halfway.
        for tick in 2..=5u32 {
            tick_march(
                &mut slot,
                tick,
                -0.90 + 0.10 * f32::from(u16::try_from(tick - 1).unwrap_or(u16::MAX)),
                1000,
                &[],
            );
        }
        assert_eq!(slot.fade().fade_left, 5);
        let weight = slot.fade().weight();
        assert!((weight - 4.0 / 9.0).abs() < 1e-6, "{weight}");
        assert_ne!(weight, 0.5);
    }

    #[test]
    fn fighter_anim_bridge_hold_pauses_the_fade_until_it_ends() {
        // LF10: the blend weight stands still through a hitstop hold and
        // resumes the first free tick; an unheld twin keeps blending.
        let run = |held: bool| {
            let mut slot = FighterSlot::new(0);
            tick_march(&mut slot, 0, -1.0, 1000, &[]);
            tick_march(&mut slot, 1, -0.90, 1000, &[]);
            assert!(slot.fade().blending(), "idle to run starts a fade");
            let total = slot.fade().fade_left;
            if held {
                slot.hold_through(5);
            }
            let mut lefts = Vec::new();
            for tick in 2..=5u32 {
                let x = -0.90 + 0.10 * f32::from(u16::try_from(tick - 1).unwrap_or(u16::MAX));
                tick_march(&mut slot, tick, x, 1000, &[]);
                lefts.push(slot.fade().fade_left);
            }
            (total, lefts)
        };
        let (total, held) = run(true);
        assert_eq!(&held[..3], &[total; 3], "ticks 2..=4 hold the blend");
        assert_eq!(held[3], total - 1, "tick 5 is free and the blend moves");
        let (_, free) = run(false);
        assert_eq!(free, vec![total - 1, total - 2, total - 3, total - 4]);
    }

    #[test]
    fn fighter_anim_bridge_run_rate_scales_with_speed() {
        let mut slow = FighterSlot::new(0);
        let mut fast = FighterSlot::new(0);
        for tick in 0..=6u32 {
            let t = f32::from(u16::try_from(tick).unwrap_or(u16::MAX));
            tick_march(&mut slow, tick, -1.0 + 0.025 * t, 1000, &[]);
            tick_march(&mut fast, tick, -1.0 + 0.10 * t, 1000, &[]);
        }
        assert_eq!(slow.pick().clip, BridgeClip::Run);
        assert_eq!(fast.pick().clip, BridgeClip::Run);
        // Analytic Q8 answers from the inputs: half rate and double rate.
        assert_eq!(slow.pick().rate_q8, 128);
        assert_eq!(fast.pick().rate_q8, 512);
        // A standing slot holds the authored rate.
        let mut still = FighterSlot::new(0);
        let pick = tick_march(&mut still, 0, -1.0, 1000, &[]);
        assert_eq!(pick.rate_q8, RATE_Q8_ONE);
        assert_eq!(rate_q8_for_speed(f32::NAN, 0.05), RATE_Q8_ONE);
        assert_eq!(rate_q8_for_speed(f32::INFINITY, 0.05), RATE_Q8_ONE);
    }

    #[test]
    fn fighter_anim_bridge_q8_decode_is_exact_without_cast() {
        // Every whole step the clamp can hand down decodes to itself, so the
        // cast-free path answers exactly like the old saturating step.
        for step in 128u16..=512u16 {
            assert_eq!(q8_from_rounded(f32::from(step)), i64::from(step));
        }
        assert_eq!(q8_from_rounded(0.0), RATE_Q8_ONE);
        assert_eq!(q8_from_rounded(600.0), RATE_Q8_ONE);
        assert_eq!(q8_from_rounded(f32::NAN), RATE_Q8_ONE);
        assert_eq!(q8_from_rounded(f32::INFINITY), RATE_Q8_ONE);
    }

    #[test]
    fn fighter_anim_bridge_strikes_cycle_attack_variants() {
        let mut slot = FighterSlot::new(0);
        tick_march(&mut slot, 0, -1.0, 1000, &[]);
        let variants = [
            BridgeClip::Attack0,
            BridgeClip::Attack1,
            BridgeClip::Attack2,
        ];
        for (round, want) in variants.iter().cycle().take(4).enumerate() {
            let tick = u32::try_from(round + 1).unwrap_or(u32::MAX);
            let pick = tick_march(&mut slot, tick, -1.0, 1000, &[hit(0, 1)]);
            assert_eq!(pick.clip, *want, "round {round}");
            assert!(slot.fade().blending(), "every swing restarts the fade");
        }
        // A rival's swing never starts ours: a fresh slot stays idle.
        let mut other = FighterSlot::new(1);
        let calm = other
            .observe_tick(0, &view(1, 1.0), 1000, &[hit(0, 1)], &tuning(), 12)
            .expect("valid rival tick");
        assert_eq!(calm.clip, BridgeClip::Idle);
    }

    #[test]
    fn fighter_anim_bridge_down_blocks_strikes_then_revives() {
        let mut slot = FighterSlot::new(0);
        tick_march(&mut slot, 0, -1.0, 1000, &[]);
        let down = tick_march(&mut slot, 1, -1.0, 0, &[]);
        assert_eq!(down.clip, BridgeClip::Down);
        assert_eq!(down.phase_ticks, 0);
        assert!(slot.fade().blending());
        // Strikes while down change nothing: the pick stays down, and the
        // swing counter does not advance (the next live swing opens at 0).
        let still = tick_march(&mut slot, 2, -1.0, 0, &[hit(0, 1)]);
        assert_eq!(still.clip, BridgeClip::Down);
        assert_eq!(still.phase_ticks, 1);
        // Back on its feet the slot idles through a fresh fade.
        let up = tick_march(&mut slot, 3, -1.0, 1000, &[]);
        assert_eq!(up.clip, BridgeClip::Idle);
        assert!(slot.fade().blending());
        assert_eq!(slot.fade().prev_clip, BridgeClip::Down.index());
        let reopen = tick_march(&mut slot, 4, -1.0, 1000, &[hit(0, 1)]);
        assert_eq!(reopen.clip, BridgeClip::Attack0);
    }

    #[test]
    fn fighter_anim_bridge_swing_holds_then_falls_back() {
        let mut slot = FighterSlot::new(0);
        tick_march(&mut slot, 0, -1.0, 1000, &[]);
        let first = tick_march(&mut slot, 1, -1.0, 1000, &[hit(0, 1)]);
        assert_eq!(first.clip, BridgeClip::Attack0);
        // Quiet ticks hold the swing for 12 ticks, then fall back to idle.
        for tick in 2..=12u32 {
            let pick = tick_march(&mut slot, tick, -1.0, 1000, &[]);
            assert_eq!(pick.clip, BridgeClip::Attack0, "tick {tick}");
        }
        let back = tick_march(&mut slot, 13, -1.0, 1000, &[]);
        assert_eq!(back.clip, BridgeClip::Idle);
        // A fresh strike mid-swing restarts with the next variant.
        tick_march(&mut slot, 14, -1.0, 1000, &[hit(0, 1)]);
        let restarted = tick_march(&mut slot, 15, -1.0, 1000, &[hit(0, 1)]);
        assert_eq!(restarted.clip, BridgeClip::Attack2);
        assert_eq!(restarted.act_start, 15);
    }

    #[test]
    fn fighter_anim_bridge_bad_input_is_refused() {
        let mut slot = FighterSlot::new(0);
        let before = slot.digest();
        let flat = BridgeTuning {
            run_speed_m_per_tick: 0.0,
            run_ref_m_per_tick: 0.05,
        };
        assert_eq!(
            slot.observe_tick(0, &view(0, 0.0), 1000, &[], &flat, 12),
            Err(BridgeError::BadTuning("run_speed_m_per_tick"))
        );
        let nan = BridgeTuning {
            run_speed_m_per_tick: f32::NAN,
            run_ref_m_per_tick: 0.05,
        };
        assert_eq!(
            slot.observe_tick(0, &view(0, 0.0), 1000, &[], &nan, 12),
            Err(BridgeError::BadTuning("run_speed_m_per_tick"))
        );
        let zero_ref = BridgeTuning {
            run_speed_m_per_tick: 0.012,
            run_ref_m_per_tick: -1.0,
        };
        assert_eq!(
            slot.observe_tick(0, &view(0, 0.0), 1000, &[], &zero_ref, 12),
            Err(BridgeError::BadTuning("run_ref_m_per_tick"))
        );
        assert_eq!(slot.digest(), before, "refusals change nothing");
        // A foreign view slot is refused and names the culprit.
        assert_eq!(
            slot.observe_tick(0, &view(3, 0.0), 1000, &[], &tuning(), 12),
            Err(BridgeError::BadSlot(3))
        );
        assert_eq!(slot.digest(), before);
        // A non-finite position is skipped: the slot stands still, stays idle.
        tick_march(&mut slot, 0, -1.0, 1000, &[]);
        let idle = slot
            .observe_tick(
                1,
                &FighterView {
                    slot: 0,
                    pos_m: [f32::NAN, 0.0, 0.0],
                    yaw: 0.0,
                },
                1000,
                &[],
                &tuning(),
                12,
            )
            .expect("skipped views still answer");
        assert_eq!(idle.clip, BridgeClip::Idle);
        assert!(BridgeError::BadSlot(9).to_string().contains('9'));
        assert!(BridgeError::BadTuning("run_ref_m_per_tick")
            .to_string()
            .contains("run_ref_m_per_tick"));
    }

    #[test]
    fn fighter_anim_bridge_steady_state_allocates_nothing() {
        let mut slot = FighterSlot::new(0);
        let strike = [hit(0, 1)];
        let ((), heap) = measure(|| {
            for tick in 0..120u32 {
                let x = -1.0 + 0.05 * f32::from(u16::try_from(tick % 7).unwrap_or(u16::MAX));
                let hp = if tick % 30 == 29 { 0 } else { 1000 };
                let cues: &[FightCue] = if tick % 5 == 1 { &strike } else { &[] };
                let _ = slot
                    .observe_tick(tick, &view(0, x), hp, cues, &tuning(), 12)
                    .expect("march stays valid");
                let _ = slot.pick();
                let _ = slot.fade();
                let _ = slot.digest();
            }
        });
        assert_eq!(heap.calls, 0, "bridge ticks must not allocate");
    }

    #[test]
    fn fighter_anim_bridge_bevy_retrigger_cancels_stale_fade() {
        // A-B-A inside the nine-tick window cancels the stale fade (Bevy
        // `AnimationTransitions::play` rule); a fresh A-C still blends, and
        // twin slots digest identically.
        let mut first = FighterSlot::new(0);
        let mut second = FighterSlot::new(0);
        assert_eq!(
            tick_march(&mut first, 0, -1.0, 1000, &[]).clip,
            BridgeClip::Idle
        );
        assert_eq!(
            tick_march(&mut second, 0, -1.0, 1000, &[]).clip,
            BridgeClip::Idle
        );
        // A-B: idle to run restarts the fade whole.
        assert_eq!(
            tick_march(&mut first, 1, -0.90, 1000, &[]).clip,
            BridgeClip::Run
        );
        assert_eq!(
            tick_march(&mut second, 1, -0.90, 1000, &[]).clip,
            BridgeClip::Run
        );
        assert!(first.fade().blending());
        assert_eq!(first.fade().prev_clip, BridgeClip::Idle.index());
        assert_eq!(first.digest(), second.digest());
        // B-A inside the window: return to the fading clip cancels it.
        assert_eq!(
            tick_march(&mut first, 2, -0.90, 1000, &[]).clip,
            BridgeClip::Idle
        );
        assert_eq!(
            tick_march(&mut second, 2, -0.90, 1000, &[]).clip,
            BridgeClip::Idle
        );
        assert!(
            !first.fade().blending(),
            "retrigger must cancel the stale fade"
        );
        assert_eq!(first.fade().weight(), 1.0);
        assert_eq!(first.fade().fade_left, 0);
        assert_eq!(first.digest(), second.digest());
        // Fresh A-C still blends: a strike opens Attack0 through a new fade.
        let strike = [hit(0, 1)];
        assert_eq!(
            tick_march(&mut first, 3, -0.90, 1000, &strike).clip,
            BridgeClip::Attack0
        );
        assert_eq!(
            tick_march(&mut second, 3, -0.90, 1000, &strike).clip,
            BridgeClip::Attack0
        );
        assert!(first.fade().blending(), "fresh switch must still blend");
        assert_eq!(first.digest(), second.digest());
        assert_eq!(first.pick(), second.pick());
    }

    #[test]
    fn fighter_anim_bridge_clip_indices_pin_the_look_order() {
        // Shared contract with the look's clip order (idle, run, three
        // swings, down): a reorder there must fail here, loudly.
        assert_eq!(BridgeClip::Idle.index(), 0);
        assert_eq!(BridgeClip::Run.index(), 1);
        assert_eq!(BridgeClip::Attack0.index(), 2);
        assert_eq!(BridgeClip::Attack1.index(), 3);
        assert_eq!(BridgeClip::Attack2.index(), 4);
        assert_eq!(BridgeClip::Down.index(), 5);
    }

    /// Replay one slot exactly as the look fold path drives it (the same
    /// views, hp and cues `fold_acts` folds): strikes, motion and quiet
    /// ticks over ticks `0..=13`.
    fn march_look_sequence(slot: &mut FighterSlot) {
        let strike = [hit(0, 1)];
        for tick in 0..=13u32 {
            let x = if tick < 3 {
                -1.0
            } else {
                -1.0 + 0.10 * f32::from(u16::try_from(tick).unwrap_or(u16::MAX))
            };
            let cues: &[FightCue] = if tick == 1 { &strike } else { &[] };
            tick_march(slot, tick, x, 1000, cues);
        }
    }

    #[test]
    fn fighter_anim_bridge_drives_look_pick() {
        // EG18: the look fold path reads each slot's `pick()` plus `fade()`
        // and samples both clips while the fade runs. Pin what it consumes
        // across idle, strike, swing hold and run, so a bridge change that
        // miscues the look fails here instead of on screen.
        let mut slot = FighterSlot::new(0);
        // Idle on first sighting: the look reads idle with no fade.
        let idle = tick_march(&mut slot, 0, -1.0, 1000, &[]);
        assert_eq!(idle.clip, BridgeClip::Idle);
        assert!(!slot.fade().blending());
        // A strike cue flips the look's pick to the first swing and restarts
        // the fade from the leaving idle clip, so the switch frame still
        // shows the old clip whole.
        let strike = [hit(0, 1)];
        let swung = tick_march(&mut slot, 1, -1.0, 1000, &strike);
        assert_eq!(swung.clip, BridgeClip::Attack0);
        assert_eq!(swung.act_start, 1);
        assert_eq!(swung.phase_ticks, 0);
        assert!(slot.fade().blending());
        assert_eq!(slot.fade().prev_clip, BridgeClip::Idle.index());
        // Quiet ticks hold the swing the look keeps sampling: same clip,
        // phase marching with the tick.
        let held = tick_march(&mut slot, 2, -1.0, 1000, &[]);
        assert_eq!(held.clip, BridgeClip::Attack0);
        assert_eq!(held.phase_ticks, 1);
        // Motion flips the pick to run once the swing window ends: the look
        // reads run with a fresh fade from the leaving swing. A fresh slot
        // replays the whole look sequence (idle, strike, motion) so the
        // swing counter matches the fold path's single drive.
        let mut driven = FighterSlot::new(0);
        march_look_sequence(&mut driven);
        let running = driven.pick();
        assert_eq!(running.clip, BridgeClip::Run);
        assert!(driven.fade().blending());
        assert_eq!(driven.fade().prev_clip, BridgeClip::Attack0.index());
        // A twin slot driven identically picks identically: the same ticks
        // give the look the same pick twice.
        let mut twin = FighterSlot::new(0);
        march_look_sequence(&mut twin);
        assert_eq!(twin.pick(), running);
        assert_eq!(twin.digest(), driven.digest());
    }

    #[test]
    fn fighter_anim_bridge_look_pick_is_atomic() {
        // EG18 caller wire: the fold path reads the cue plus the fade
        // together, so a new pick never pairs with an old fade.
        let mut slot = FighterSlot::new(0);
        let idle = tick_march(&mut slot, 0, -1.0, 1000, &[]);
        assert_eq!(idle, slot.look_pick().0);
        assert_eq!(slot.fade(), slot.look_pick().1);
        let strike = [hit(0, 1)];
        let swung = tick_march(&mut slot, 1, -1.0, 1000, &strike);
        let (pick, fade) = slot.look_pick();
        assert_eq!((pick, fade), (swung, slot.fade()));
        assert_eq!(pick.clip, BridgeClip::Attack0);
        assert!(fade.blending());
    }

    #[test]
    fn fighter_anim_bridge_hold_enters_digest() {
        // EG18 slice: the hold expiry drives the hitstop freeze, so the replay
        // proof must include it; otherwise a held slot and a free slot with the
        // same pick digest identically yet freeze differently.
        let mut held = FighterSlot::new(0);
        let mut free = FighterSlot::new(0);
        for tick in 0..=4u32 {
            tick_march(&mut held, tick, -1.0, 1000, &[]);
            tick_march(&mut free, tick, -1.0, 1000, &[]);
        }
        assert_eq!(held.digest(), free.digest(), "same ticks digest the same");
        held.hold_through(8);
        assert_ne!(
            held.digest(),
            free.digest(),
            "hold expiry must enter the digest"
        );
        let mut twin = FighterSlot::new(0);
        for tick in 0..=4u32 {
            tick_march(&mut twin, tick, -1.0, 1000, &[]);
        }
        twin.hold_through(8);
        assert_eq!(held.digest(), twin.digest(), "same hold digests the same");
    }

    #[test]
    fn fighter_anim_bridge_hold_freezes_phase() {
        // EG18 slice: the hitstop hold marches the start so the clip phase
        // stays put (the look promises the phase stays put while held).
        let mut slot = FighterSlot::new(0);
        for tick in 0..=8u32 {
            tick_march(&mut slot, tick, -1.0, 1000, &[]);
        }
        let frozen = slot.pick();
        slot.hold_through(12);
        slot.march_hold(2);
        assert_eq!(
            slot.pick().phase_ticks,
            frozen.phase_ticks,
            "hold keeps the phase put"
        );
        assert_eq!(
            slot.pick().act_start,
            frozen.act_start.saturating_add(2),
            "start marches so resume recomputes the same phase"
        );
    }
}
