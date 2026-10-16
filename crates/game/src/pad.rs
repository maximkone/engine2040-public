//! Gamepad sticks and buttons through one shared bindings table.
//!
//! Owner: this module owns the stick snapshot plus the merge into the
//! caller's pressed list; the input crate owns the bindings table, the
//! deadzone shape and the validated fighter command. Units: stick
//! deflection is a finite 32-bit float in `-1.0..=1.0` per axis, wishes
//! are `-100..=100` percent of top speed, deadzones are wish units,
//! ticks are zero-based fixed steps. Determinism: the same snapshot plus
//! the same table gives the same command twice; iteration is index order,
//! never hash order, with no wall clock. Errors: bad fighter slots fail
//! through the shared fighter error; non-finite deflection rests at zero.

use engine_input::fighter::WISH_RANGE;
use engine_input::{
    apply_deadzone, Bindings, FighterError, FighterId, FighterInput, PhysicalButton,
};

/// One gamepad snapshot as plain data: no device handles, no threads.
///
/// The window layer fills one of these per fixed tick from whatever pad
/// backend it uses; this module only reads the snapshot. The button bits
/// cover every pad code the shared button type can name. Sticks rest at
/// zero deflection; non-finite readings also rest at zero once quantized.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PadFrame {
    /// Left stick sideways deflection, `-1.0` left to `1.0` right.
    pub lx: f32,
    /// Left stick forward deflection, `-1.0` back to `1.0` forward.
    pub ly: f32,
    /// Held pad buttons as bits: bit `i` means pad code `i` is down.
    pub buttons: [u64; 4],
}

impl PadFrame {
    /// Empty snapshot: sticks centered, no buttons held.
    ///
    /// Units: none. Ownership: caller-owned value. Threading: `Send + Sync`,
    /// safe on any thread. Determinism: bitwise equal snapshots compare equal.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            lx: 0.0,
            ly: 0.0,
            buttons: [0, 0, 0, 0],
        }
    }

    /// Snapshot from raw readings: stick deflection plus the held pad codes.
    ///
    /// Units: sticks in `-1.0..=1.0` (values outside clamp at quantize time),
    /// codes are pad button indices. Ownership: caller-owned value.
    /// Threading: `Send + Sync`. Determinism: the same inputs give the same
    /// bits twice; duplicate codes collapse to one bit. Errors: none; every
    /// input maps to some snapshot, never panics.
    #[must_use]
    pub fn snapshot(lx: f32, ly: f32, pressed: &[u8]) -> Self {
        let mut buttons = [0u64, 0u64, 0u64, 0u64];
        for code in pressed {
            let word = usize::from(*code / 64);
            let bit = *code % 64;
            // The word index always fits the four-word table by construction.
            if let Some(slot) = buttons.get_mut(word) {
                *slot |= 1u64 << bit;
            }
        }
        Self { lx, ly, buttons }
    }

    /// True when pad code `index` is held in this snapshot.
    ///
    /// Units: button index. Ownership: borrows the snapshot. Threading:
    /// `Send + Sync`. Determinism: pure bit read.
    #[must_use]
    pub fn is_pressed(self, index: u8) -> bool {
        let word = usize::from(index / 64);
        let bit = index % 64;
        self.buttons
            .get(word)
            .is_some_and(|slot| slot & (1u64 << bit) != 0)
    }

    /// Stick wish for this snapshot: sideways becomes right wish, forward
    /// becomes forward wish, quantized then passed through `deadzone`.
    ///
    /// Units: wishes in `-100..=100`, deadzone in wish units. Ownership:
    /// borrows the snapshot. Threading: `Send + Sync`. Determinism: pure
    /// over the two deflections plus the deadzone; non-finite deflection
    /// rests at zero.
    #[must_use]
    pub fn stick_wish(self, deadzone: u8) -> (i8, i8) {
        stick_wish(self.lx, self.ly, deadzone)
    }

    /// Append held pad buttons to the caller's pressed list in index order,
    /// skipping codes already present so one button never fires twice.
    ///
    /// Units: button indices. Ownership: borrows the snapshot, extends the
    /// caller's buffer which keeps its capacity. Threading: caller thread
    /// only, no locks. Determinism: index order `0..=255`; the same snapshot
    /// plus the same incoming list appends the same codes twice. Budget:
    /// at most one scan of the button table; no heap use beyond the
    /// caller's buffer growth.
    pub fn append_pressed(self, out: &mut Vec<PhysicalButton>) {
        for index in 0..=u8::MAX {
            if self.is_pressed(index) {
                let button = PhysicalButton::Pad(index);
                if !out.contains(&button) {
                    out.push(button);
                }
            }
        }
    }
}

/// Quantize one stick axis to wish units: finite deflection scales by one
/// hundred and rounds, non-finite deflection rests at zero.
///
/// Units: input in `-1.0..=1.0` (outside values clamp), output in
/// `-100..=100`. Ownership: pure value function. Threading: `Send + Sync`.
/// Determinism: the same float gives the same byte twice; the float total
/// order decides rounding, never platform state. Errors: none; every float
/// maps to some wish, never panics.
#[must_use]
pub fn quantize_axis(value: f32) -> i8 {
    if !value.is_finite() {
        return 0;
    }
    let clamped = value.clamp(-1.0, 1.0);
    let scaled = (clamped * 100.0).round();
    // The rounded scale is an exact integer float in range, so it equals
    // exactly one wish converted back; the search converts integers to
    // floats (exact here) instead of floats to integers.
    for candidate in -100i8..=100i8 {
        if f32::from(candidate) == scaled {
            return candidate;
        }
    }
    0
}

/// Stick deflection to wish: sideways maps to right wish, forward maps to
/// forward wish, quantized then snapped through `deadzone` by the shared
/// deadzone owner.
///
/// Units: deflection in `-1.0..=1.0` per axis, wishes in `-100..=100`,
/// deadzone in wish units. Ownership: pure value function. Threading:
/// `Send + Sync`. Determinism: pure over the two floats plus the deadzone.
/// Errors: none; every pair maps to some wish, never panics.
#[must_use]
pub fn stick_wish(lx: f32, ly: f32, deadzone: u8) -> (i8, i8) {
    let mz = quantize_axis(lx);
    let mx = quantize_axis(ly);
    apply_deadzone(mx, mz, deadzone)
}

/// Add a button wish and a stick wish with saturation to the wish range.
///
/// Units: wishes in `-100..=100` per axis. Ownership: pure value function.
/// Threading: `Send + Sync`. Determinism: pure integer math; opposed full
/// deflections clamp rather than wrap. Errors: none; every pair maps to a
/// wish in range, never panics.
#[must_use]
pub fn combine_wish(button: (i8, i8), stick: (i8, i8)) -> (i8, i8) {
    let mx = i16::from(button.0) + i16::from(stick.0);
    let mz = i16::from(button.1) + i16::from(stick.1);
    let lo = i16::from(-WISH_RANGE);
    let hi = i16::from(WISH_RANGE);
    let mx = mx.clamp(lo, hi);
    let mz = mz.clamp(lo, hi);
    // Both sums clamp to the wish range above, so the fallible step always
    // succeeds; the fallback only satisfies the total function contract.
    let mx = i8::try_from(mx).unwrap_or(0);
    let mz = i8::try_from(mz).unwrap_or(0);
    (mx, mz)
}

/// Merge keyboard and pad buttons into one pressed list with no duplicates.
///
/// The caller holds `out` across ticks: clear it, copy the keyboard slice in
/// order, then append pad codes in index order skipping repeats. There is a
/// single input state per tick, never a pad table beside the keyboard table.
///
/// Units: button codes. Ownership: borrows both inputs, reuses the caller's
/// buffer which keeps its capacity. Threading: caller thread only, no locks.
/// Determinism: keyboard order then pad index order; the same inputs merge
/// to the same list twice. Budget: one pass over each input; no heap use
/// beyond the caller's buffer growth.
pub fn merge_pressed(keyboard: &[PhysicalButton], pad: &PadFrame, out: &mut Vec<PhysicalButton>) {
    out.clear();
    out.extend_from_slice(keyboard);
    pad.append_pressed(out);
}

/// Build one validated fighter command from merged buttons plus stick wish.
///
/// Button wishes come from the shared table over `pressed`, the stick wish
/// adds on with saturation, facing stays pinned at zero like the keyboard
/// path, and the table validates the result. Rebinding any action row to a
/// pad code routes that pad button through this same call.
///
/// Units: wishes in `-100..=100`, facing in centidegrees (always zero here),
/// ticks are zero-based. Ownership: borrows the pressed slice and the table.
/// Threading: caller thread only, no locks. Determinism: pure over the
/// inputs plus the table; the same row gives the same command twice.
/// Errors: [`FighterError`] when the fighter slot or the combined wish is
/// out of range; the table refuses loudly, never clamps silently.
pub fn fighter_input_from_pad(
    tick: u32,
    fighter: FighterId,
    pressed: &[PhysicalButton],
    stick: (i8, i8),
    bindings: &Bindings,
) -> Result<FighterInput, FighterError> {
    let button = bindings.wish_from_pressed(pressed);
    let wish = combine_wish(button, stick);
    bindings.to_fighter_input(tick, fighter, pressed, wish, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::alloc_probe;
    use engine_input::{fighter::ATTACK, Bindings, BoundAction, Buttons};

    fn fighter() -> FighterId {
        FighterId::new(0).expect("slot zero is always valid")
    }

    #[test]
    fn axes_quantize_full_half_and_rest() {
        assert_eq!(quantize_axis(1.0), 100);
        assert_eq!(quantize_axis(-1.0), -100);
        assert_eq!(quantize_axis(0.0), 0);
        assert_eq!(quantize_axis(0.5), 50);
        assert_eq!(quantize_axis(-0.25), -25);
        // Outside deflection clamps instead of escaping the wish range.
        assert_eq!(quantize_axis(2.0), 100);
        assert_eq!(quantize_axis(-3.0), -100);
        // Non-finite readings rest at zero so no bad float reaches the hash.
        assert_eq!(quantize_axis(f32::NAN), 0);
        assert_eq!(quantize_axis(f32::INFINITY), 0);
        assert_eq!(quantize_axis(f32::NEG_INFINITY), 0);
    }

    #[test]
    fn deadzone_snaps_noise_but_keeps_drive() {
        let bindings = Bindings::new();
        let dead = bindings.deadzone();
        // Stick noise inside the deadzone rests.
        assert_eq!(PadFrame::snapshot(0.02, 0.01, &[]).stick_wish(dead), (0, 0));
        assert_eq!(stick_wish(0.0, 0.0, dead), (0, 0));
        // Full deflection drives full wish through the same deadzone.
        assert_eq!(stick_wish(1.0, 1.0, dead), (100, 100));
        assert_eq!(stick_wish(-1.0, 0.5, dead), (50, -100));
        // The frame helper agrees with the free function on the same inputs.
        let frame = PadFrame::snapshot(0.5, -0.5, &[]);
        assert_eq!(frame.stick_wish(dead), stick_wish(0.5, -0.5, dead));
    }

    #[test]
    fn rebound_pad_button_fires_and_old_key_stops() {
        let mut bindings = Bindings::new();
        let old = bindings.button(BoundAction::Attack);
        bindings.rebind(BoundAction::Attack, PhysicalButton::Pad(5));
        let pressed = [PhysicalButton::Pad(5)];
        let input = fighter_input_from_pad(0, fighter(), &pressed, (0, 0), &bindings)
            .expect("pad attack validates");
        assert_eq!(input.buttons, Buttons(ATTACK));
        // The old key no longer attacks once its row moves to the pad.
        let stale = fighter_input_from_pad(0, fighter(), &[old], (0, 0), &bindings)
            .expect("stale key validates");
        assert_eq!(stale.buttons, Buttons::NONE);
        // Every action row accepts a pad code, so a full fight fits on pads.
        for action in BoundAction::ALL {
            let code = u8::try_from(action.index())
                .expect("action table fits in a pad code")
                .saturating_add(10);
            bindings.rebind(action, PhysicalButton::Pad(code));
        }
        let config = engine_input::InputConfig::from_bindings(&bindings);
        let restored = config.to_bindings();
        assert_eq!(restored, bindings);
    }

    #[test]
    fn sticks_add_to_keys_with_saturation() {
        let bindings = Bindings::new();
        // Keyboard forward plus stick right gives a diagonal.
        let fwd = bindings.button(BoundAction::MoveForward);
        let right = bindings.button(BoundAction::MoveRight);
        let pressed = [fwd];
        let input = fighter_input_from_pad(7, fighter(), &pressed, (0, 100), &bindings)
            .expect("diagonal validates");
        assert_eq!((input.mx, input.mz), (100, 100));
        assert_eq!(input.tick, 7);
        // Opposed full drives clamp instead of wrapping.
        assert_eq!(combine_wish((100, 100), (100, 100)), (100, 100));
        assert_eq!(combine_wish((-100, -100), (-100, 100)), (-100, 0));
        assert_eq!(combine_wish((100, 0), (-100, 0)), (0, 0));
        // The unused move rows never set button bits.
        let moves = [fwd, right];
        assert_eq!(bindings.button_bits(&moves), Buttons::NONE);
    }

    #[test]
    fn merge_keeps_one_state_in_fixed_order() {
        let pad = PadFrame::snapshot(0.0, 0.0, &[3, 7, 3]);
        let keyboard = [PhysicalButton::Key(9), PhysicalButton::Pad(3)];
        let mut merged = Vec::with_capacity(8);
        merge_pressed(&keyboard, &pad, &mut merged);
        // Keyboard order first, then pad index order, each code once.
        assert_eq!(
            merged,
            vec![
                PhysicalButton::Key(9),
                PhysicalButton::Pad(3),
                PhysicalButton::Pad(7),
            ]
        );
        // The same inputs merge to the same list twice.
        let mut again = Vec::with_capacity(8);
        merge_pressed(&keyboard, &pad, &mut again);
        assert_eq!(merged, again);
        // Duplicate pad codes collapse in the snapshot itself.
        assert!(pad.is_pressed(3));
        assert!(pad.is_pressed(7));
        assert!(!pad.is_pressed(4));
        assert_eq!(PadFrame::empty(), PadFrame::snapshot(0.0, 0.0, &[]));
    }

    #[test]
    fn bad_fighter_is_refused_loudly() {
        let bindings = Bindings::new();
        let bad = FighterId(99);
        assert_eq!(
            fighter_input_from_pad(0, bad, &[], (0, 0), &bindings),
            Err(FighterError::UnknownFighter(99))
        );
    }

    #[test]
    fn pad_driven_session_advances_and_replays() {
        let config = crate::GameConfig {
            seed: 41,
            ticks: 64,
            fighters: 2,
            half_extent_m: 12.0,
        };
        let run_once = || {
            let mut app = crate::App::new(config).expect("test config validates");
            let mut bindings = Bindings::new();
            bindings.rebind(BoundAction::Attack, PhysicalButton::Pad(0));
            let mut pressed = Vec::with_capacity(8);
            let mut outcomes = Vec::with_capacity(64);
            for tick in 0..64u32 {
                // Circle on the stick, pad attack every fifth tick.
                let phase = tick % 4;
                let (lx, ly) = match phase {
                    0 => (1.0, 0.0),
                    1 => (0.0, 1.0),
                    2 => (-1.0, 0.0),
                    _ => (0.0, -1.0),
                };
                let pad = if tick % 5 == 0 {
                    PadFrame::snapshot(lx, ly, &[0])
                } else {
                    PadFrame::snapshot(lx, ly, &[])
                };
                merge_pressed(&[], &pad, &mut pressed);
                let stick = pad.stick_wish(bindings.deadzone());
                let input = fighter_input_from_pad(tick, fighter(), &pressed, stick, &bindings)
                    .expect("scripted pad input validates");
                outcomes.push(app.tick(input));
            }
            (app.checkpoints().to_vec(), outcomes)
        };
        let (first_checks, first_outcomes) = run_once();
        let (second_checks, second_outcomes) = run_once();
        assert_eq!(first_checks.len(), 64);
        assert_eq!(first_checks, second_checks);
        assert_eq!(first_outcomes, second_outcomes);
        assert!(first_outcomes.iter().all(|outcome| outcome.input_accepted));
        // The scripted bout moves rather than idling on one hash.
        let mut hashes: Vec<u64> = first_checks.iter().map(|check| check.hash).collect();
        hashes.sort_unstable();
        hashes.dedup();
        assert!(hashes.len() > 1, "pad bout never moved");
    }

    #[test]
    fn merged_path_allocates_nothing_at_steady_state() {
        // Calibration: the shared probe must see a real allocation first.
        let (_, probe) = alloc_probe::measure(|| std::hint::black_box(vec![0u8; 64]));
        assert!(probe.calls >= 1, "allocation probe is blind");

        let bindings = Bindings::new();
        let pad = PadFrame::snapshot(0.5, -0.25, &[1, 2]);
        let keyboard = [PhysicalButton::Key(3)];
        let mut pressed = Vec::with_capacity(16);
        merge_pressed(&keyboard, &pad, &mut pressed);
        let stick = pad.stick_wish(bindings.deadzone());
        // Warmup sizes the buffer; the measured pass must not grow it.
        let (input, steady) = alloc_probe::measure(|| {
            merge_pressed(&keyboard, &pad, &mut pressed);
            fighter_input_from_pad(0, fighter(), &pressed, stick, &bindings)
        });
        assert!(input.is_ok());
        assert_eq!(
            steady.calls, 0,
            "pad merge plus build must not allocate per tick"
        );
    }
}
