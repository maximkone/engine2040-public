//! combat: frame-data combat. Timing as data (hitstop, cancel/parry windows, trauma). Never tick-coupled.

#![warn(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

/// Anatomy graph plus tissue and armor tables for deterministic cuts:
/// joint parents, per-segment Q16 toughness, per-joint armor coverage and
/// the Rome gladiator row. Std-only, no heap, integer math only.
pub mod anatomy;
/// Match-step wiring over the exchange and sweep resolvers: the arena
/// call-site shape, integer ticks, caller buffers, no heap. Std-only.
pub mod arena;
/// Third-person melee camera framing from two ground positions: boom behind
/// the attacker's shoulder, silhouette readability, in-frame check. Pure
/// integers, no heap, no state. Presentation only.
pub mod camera_framing;
/// Ludus Magnus gladiator rows as `Tables` presets: integer permille only.
/// Caller: `crate::fight::class_tables`.
pub mod classes;
/// Shared string counter across the three stances: one cursor advanced by
/// every attack and by movement, with a per-stance move count. Integer
/// indices, no heap, `Copy`. Std-only.
pub mod combo;
/// Deterministic blade-vs-limb sever resolver: swept edge vs joint
/// capsules in integers, edge sharpness and wear, one 16-byte cut per joint
/// per swing. Std-only, no heap.
pub mod cut;
/// Combat-depth cores over the base fight table core: hit resolver,
/// stamina, poise and combo chains. Integer ticks, no heap. Std-only.
pub mod depth;
/// Duel timing patterns as data: hit-stop, cancel and reaction windows.
/// Integer ticks at sixty hertz, no heap, never tick-coupled. Std-only.
pub mod design;
/// Parry/riposte + clash over the base fight table core: timed answers,
/// integer ticks, versioned data. Std-only.
pub mod exchange;
/// Pichlmair feel-survey schemas: tuning/juicing/streamlining rows.
pub mod feel;
/// Feel effects registered against fight triggers (Game Feel Flow pattern):
/// one slot store to register, fixed firing order, no heap. Presentation only.
pub mod feel_flow;
/// Impact-feel instrumentation over the feel rows: 20-scenario
/// hit-stop + camera ablation and the balance-survey entry. Harness-only.
pub mod feel_instrument;
pub mod fight;
/// Integer port of the deepnight/gamefeel presentation feel: camera shake,
/// bump, hit-stop and slow-mo easing. Presentation only, no heap.
pub mod gamefeel;
pub mod gore;
/// Hit simulation: one blade state against every severable joint over many
/// clean hits, as a seeded distribution (the door's `hit.simulate`). Std-only.
pub mod hitsim;
/// Weapon weight in the fight (FEEL1): a weapon row's own startup, recovery,
/// damage and stagger, parsed from `content/design/weapons.rows`. Std-only.
pub mod heft;
/// Per-kind hit-stop freeze rows: light taps, heavy lands, parry answers
/// longest. V2 kind-to-ticks map; V1 keeps its single row. Std-only.
pub mod hitstop_kind;
/// Life after a cut or a heavy wound: stump states, venous and arterial
/// bleeding, shock, faint, surrender and exsanguination as versioned
/// tables. Integer, no heap. Std-only.
pub mod limbloss;
/// Nerve meter beside the feel survey: u8 resolve/break/dread/anchor levels.
pub mod nerve;
pub mod projectile;
pub mod reaction;
/// Trauma-curve shake stage: squared-trauma magnitude on the gamefeel wave
/// pair, as permille of the shake cap. Pure integers, presentation only.
pub mod shake_stage;
/// Stance clash matrix as a `4x4` table: Fast/Balanced/Heavy/Air, win-draw-
/// lose, single-sourced so players read one matrix. No heap, `Copy`.
pub mod stance;
/// Continuous swept detection: segment-vs-capsule, shield cones,
/// reach and facing decide. Std-only.
pub mod sweep;
pub mod tables;
/// Designer rows text loader: `content/design/combat.rows` into `SwapInput`.
/// Std-only, cold path; game wiring lands in EG29.
pub mod tables_text;

pub use anatomy::{
    validate as validate_anatomy, AnatomyError, AnatomyTable, JointId, TissueRow,
    ANATOMY_VERSION_V1, ARMOR_ALL, ARMOR_GALEA, ARMOR_GREAVE, ARMOR_LORICA, ARMOR_MANICA,
    ARMOR_SCUTUM, BONE_Q16, FLESH_Q16, JOINT_TOTAL, MAX_TOUGHNESS_Q16, NO_PARENT,
    SEVERABLE_JOINT_COUNT, SKIN_Q16,
};

pub use combo::{
    combo_table_for_version, ComboCursor, ComboError, ComboTable, Stance, COMBO_V1,
    COMBO_VERSION_V1, MAX_STRING, STANCE_COUNT,
};
pub use cut::{
    chance_q16, cut_for_version, cut_one, digest_cuts, effective_q16, resolve_swing, roll_q16,
    saturate_q16, try_cut_one, try_resolve_swing, validate_cut_tables, validate_swing,
    validate_target, BladeSwing, Cut, CutError, CutMiss, CutOutcome, CutTableSet, CutTables,
    JointTarget, CUT_BLADE_HALF_MAX_MM, CUT_JOINT_RADIUS_MAX_MM, CUT_POS_BOUND_MM, CUT_Q16,
    CUT_SLACK_MM_V1, CUT_SWING_LEN_MM_V1, CUT_V1, CUT_VERSION_V1, MAX_CUTS_PER_SWING,
    MAX_ENERGY_Q16, MAX_SHARPNESS_Q16, MAX_TARGETS_PER_SWING, MAX_WEAR_Q16, SEVER_SPAN_Q16_V1,
};
pub use fight::{
    step, step_into, step_with_tables, step_with_tables_into, try_step, try_step_into,
    try_step_v2_with_cues_into, try_step_with_tables, try_step_with_tables_into, validate_inputs,
    AttackStarted, CombatError, FightEvent, FightState, FighterState, Tables,
    MAX_ATTACK_STARTED_PER_TICK, MAX_EVENTS_PER_TICK, TABLES_V1, TABLES_V2,
};
pub use gore::{
    decal_pos_cm, gore_for_events, gore_for_events_with_tables, severity_for_damage, spray_pos_cm,
    GoreEvent, GoreKind, JITTER_CM, SLOT_SPACING_CM, SPRAY_HEIGHT_CM,
};
pub use stance::{
    stance_matrix_for_version, ClashExt, ClashVerdict, StanceError, StanceMatrix, STANCE_MATRIX_V1,
    STANCE_MATRIX_VERSION_V1,
};
pub use tables::{
    step_for_version, tables_for_version, try_step_for_version, validate_tables, MoveTableSet,
    SwapInput, TABLES_VERSION_V1,
};

// The one counting allocator of this test binary: zero-allocation tests read
// it through `engine_core::alloc_probe::measure`.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOC: engine_core::alloc_probe::CountingAlloc =
    engine_core::alloc_probe::CountingAlloc;

/// One move's timing row: startup/active/recovery frames plus the
/// cancel-into list. All strings are borrowed statics, so rows hold no heap
/// and tick code can keep them freely. Deterministic: plain integer compares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MoveData {
    /// Move name, e.g. `"jab"`.
    pub name: &'static str,
    /// Startup frames before the active window.
    pub startup: u32,
    /// Active frames.
    pub active: u32,
    /// Recovery frames.
    pub recovery: u32,
    /// Moves this one may cancel into, matched by name.
    pub cancel: &'static [&'static str],
}

impl MoveData {
    pub fn total(&self) -> u32 {
        self.startup + self.active + self.recovery
    }
    pub fn can_cancel(&self, into_move: &str, frame: u32) -> bool {
        frame >= self.startup && self.cancel.contains(&into_move)
    }
}

/// Camera trauma in permille, `0..=1000` (0 steady, 1000 full shake).
/// Presentation-only: shake offsets never enter hashed state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trauma(u16);

impl Trauma {
    /// Builds trauma from permille, clamping past `1000` to full shake.
    pub fn from_permille(permille: u16) -> Self {
        Self(permille.min(1000))
    }

    /// Trauma as a `0.0..=1.0` amplitude scale.
    pub fn amplitude(self) -> f32 {
        f32::from(self.0) / 1000.0
    }
}

/// Trauma shake pure fn: seeded, no alloc. Amplitude decays quadratically;
/// same seed plus tick always gives the same offsets on every PC for one
/// build (presentation-only; the offsets never reach the state hash).
pub fn trauma_shake(seed: u64, trauma: Trauma, tick: u64) -> (f32, f32) {
    let t = trauma.amplitude();
    if t <= 0.0 {
        return (0.0, 0.0);
    }
    // Seeded noise from the shared SplitMix64 step (no rand crate); the two
    // halves of one mixed word give the two axes, so one tick is one sample.
    let z = engine_core::splitmix64(seed.wrapping_add(tick)) | 1;
    let max = engine_core::checked::f32_from_u64(u64::from(u32::MAX));
    let low = u64::from(u32::try_from(z & u64::from(u32::MAX)).unwrap_or(u32::MAX));
    let (a, b) = (
        engine_core::checked::f32_from_u64(z >> 32) / max * 2.0 - 1.0,
        engine_core::checked::f32_from_u64(low) / max * 2.0 - 1.0,
    );
    let amp = t * t; // trauma^2 response
    (a * amp, b * amp)
}

/// Shipped jab row: test-pinned presentation bridge; no production caller
/// (verified 2026-10-16).
pub const MOVE_JAB: MoveData = MoveData {
    name: "jab",
    startup: 4,
    active: 2,
    recovery: 8,
    cancel: &["cross"],
};

/// Shipped cross row: cancel target of the jab. Test-pinned presentation
/// bridge; no production caller (verified 2026-10-16).
pub const MOVE_CROSS: MoveData = MoveData {
    name: "cross",
    startup: 6,
    active: 3,
    recovery: 12,
    cancel: &["uppercut"],
};

/// Shipped uppercut row: terminal move, cancels into nothing.
/// Test-pinned presentation bridge; no production caller (verified 2026-10-16).
pub const MOVE_UPPERCUT: MoveData = MoveData {
    name: "uppercut",
    startup: 9,
    active: 3,
    recovery: 16,
    cancel: &[],
};

/// Shipped timing table for cancel checks in tests: test-pinned presentation
/// bridge; no production caller (verified 2026-10-16).
/// Deterministic: plain row order, no heap.
pub const MOVES_V1: &[MoveData] = &[MOVE_JAB, MOVE_CROSS, MOVE_UPPERCUT];

/// Look up a shipped move row by name in tests. Pure, no heap, `O(rows)`.
/// Test-pinned presentation bridge; no production caller (verified 2026-10-16).
/// Returns `None` for an unknown name instead of panicking.
#[must_use]
pub fn move_row(name: &str) -> Option<MoveData> {
    let mut i = 0;
    while i < MOVES_V1.len() {
        if MOVES_V1[i].name == name {
            return Some(MOVES_V1[i]);
        }
        i += 1;
    }
    None
}

/// Shake offsets for a trauma permille at a tick: test-pinned presentation
/// bridge; no production caller (verified 2026-10-16). Same seed plus tick
/// plus permille always gives the same offsets on every PC for one build;
/// offsets never reach the state hash. No heap.
#[must_use]
pub fn shake_for_trauma(seed: u64, trauma_permille: u16, tick: u64) -> (f32, f32) {
    trauma_shake(seed, Trauma::from_permille(trauma_permille), tick)
}

/// Parry window check: frame within [start,end] inclusive (data, e.g. 2f/4f/6f/10f).
pub fn parry_open(frame: u32, start: u32, end: u32) -> bool {
    frame >= start && frame <= end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jab() -> MoveData {
        MoveData {
            name: "jab",
            startup: 4,
            active: 2,
            recovery: 8,
            cancel: &["cross"],
        }
    }

    #[test]
    fn cancel_windows_as_data() {
        let m = jab();
        assert_eq!(m.total(), 14);
        assert!(!m.can_cancel("cross", 2)); // startup, too early
        assert!(m.can_cancel("cross", 5));
        assert!(!m.can_cancel("uppercut", 5));
    }

    #[test]
    fn trauma_seeded() {
        assert_eq!(Trauma::from_permille(2000), Trauma::from_permille(1000));
        assert_eq!(Trauma::from_permille(500).amplitude(), 0.5);
        assert_eq!(trauma_shake(7, Trauma::from_permille(0), 99), (0.0, 0.0));
        let (a, b) = (
            trauma_shake(7, Trauma::from_permille(1000), 3),
            trauma_shake(7, Trauma::from_permille(1000), 3),
        );
        assert_eq!(a, b);
        assert_ne!(
            trauma_shake(7, Trauma::from_permille(1000), 3),
            trauma_shake(7, Trauma::from_permille(1000), 4)
        );
    }

    #[test]
    fn shipped_moves_feed_cancel_checks() {
        let jab = move_row("jab").expect("jab ships");
        assert_eq!(jab, MOVE_JAB);
        assert_eq!(jab.total(), 14);
        assert!(jab.can_cancel("cross", 5));
        let cross = move_row("cross").expect("cross ships");
        assert!(cross.can_cancel("uppercut", 6));
        assert_eq!(move_row("nope"), None);
        assert_eq!(MOVES_V1.len(), 3);
    }

    #[test]
    fn shake_bridge_matches_shake_and_clamps() {
        assert_eq!(shake_for_trauma(7, 0, 99), (0.0, 0.0));
        assert_eq!(
            shake_for_trauma(7, 1000, 3),
            trauma_shake(7, Trauma::from_permille(1000), 3)
        );
        assert_eq!(
            shake_for_trauma(7, 2000, 3),
            trauma_shake(7, Trauma::from_permille(1000), 3)
        );
    }

    #[test]
    fn parry_windows() {
        assert!(parry_open(2, 2, 4));
        assert!(parry_open(4, 2, 4));
        assert!(!parry_open(5, 2, 4));
        assert!(!parry_open(1, 2, 4));
    }

    #[test]
    fn attack_started_reachable_via_crate_root() {
        // EG19b: the press-tick cue entry is public via the crate root so the
        // arena game leg names it without reaching into `fight::`.
        let cue = crate::AttackStarted { attacker: 0 };
        assert_eq!(cue.attacker, 0);
        assert_eq!(
            crate::MAX_ATTACK_STARTED_PER_TICK,
            crate::fight::MAX_ATTACK_STARTED_PER_TICK
        );
        assert_eq!(crate::TABLES_V2, crate::fight::TABLES_V2);
        let _ = crate::try_step_v2_with_cues_into
            as fn(
                &[engine_input::FighterInput],
                &[crate::fight::StrikePose],
                &mut crate::FightState,
                &crate::Tables,
                &mut Vec<crate::FightEvent>,
                &mut Vec<crate::AttackStarted>,
            ) -> Result<(), crate::CombatError>;
    }
}
