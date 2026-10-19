//! gladiator: scripted duel policy emitting [`FighterInput`].
//!
//! Consumer: `arena` fills [`Percept`] from snapshots and feeds one
//! [`Brain`] per fighter. Data owner: [`Brain`] owns its perception clock,
//! last intent and far-streak only; match state stays in the arena.
//! Units: metres for `dist_m`/positions, ticks are zero-based fixed steps
//! (60 Hz assumed, so 180 ticks = 3 s). Determinism scope: same [`Percept`]
//! sequence through same-config [`Brain`]s gives byte-identical
//! [`FighterInput`]s. Errors: inputs are sanitized (NaN/inf clamped), never
//! panics; every emitted input passes [`FighterInput::validate`].
//!
//! Why not reuse `utility::pick` / `behavior::tick`: those score a [`Brief`]
//! (entity/body counts) with `f32` scorers, while this policy scores a
//! positional [`Percept`] with integer math per the no-transcendental
//! contract. The tie-break convention is mirrored: strict `>` so the first
//! index wins ties. Integer math only (`+ - * /`, comparisons, integer
//! division); no `sin`/`cos`/`atan2`/`sqrt`/`exp`/`powf` anywhere.
//!
//! Any brain drives the tick through [`TickPolicy`]: the arena composes its
//! Rome biases (phase/feint/morale/role) around this core, and
//! [`MarchPolicy`] proves a non-gladiator script runs the same seam.
//!
//! [`Brief`]: crate::Brief
//! [`FighterInput`]: engine_input::FighterInput
//! [`FighterInput::validate`]: engine_input::FighterInput::validate

use crate::apply::{apply_due, apply_due_into, AppliedBatch};
use crate::curves::{AiConfig, AiCurves};
use crate::curves_text::AiRowsError;
use crate::melee::MeleeView;
use crate::mind::{ActionLog, MindRequest};
use engine_input::fighter::{ATTACK, BLOCK, DODGE, FACING_RANGE};
use engine_input::{Buttons, FighterId, FighterInput};

/// Arena-filled observation. Plain data; the arena maps snapshots into it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Percept {
    /// Own position in metres (x, y, z).
    pub self_pos: [f32; 3],
    /// Own yaw in radians (used only as a facing fallback).
    pub self_yaw: f32,
    /// Foe position in metres (x, y, z).
    pub foe_pos: [f32; 3],
    /// True while the foe is attacking.
    pub foe_attacking: bool,
    /// Separation in metres.
    pub dist_m: f32,
    /// Zero-based fixed tick.
    pub tick: u32,
    /// Quantized close-quarters view (own vitals, the nearest foes, the
    /// foe's swing phase). [`MeleeView::EMPTY`] on every legacy percept,
    /// which adds no bias.
    pub melee: MeleeView,
}

impl Percept {
    /// Build a percept; fields are sanitized on use, never here. The melee
    /// view starts empty: attach one with [`Percept::with_melee`].
    pub const fn new(
        self_pos: [f32; 3],
        self_yaw: f32,
        foe_pos: [f32; 3],
        foe_attacking: bool,
        dist_m: f32,
        tick: u32,
    ) -> Self {
        Self {
            self_pos,
            self_yaw,
            foe_pos,
            foe_attacking,
            dist_m,
            tick,
            melee: MeleeView::EMPTY,
        }
    }

    /// This percept with a quantized close-quarters view attached.
    #[must_use]
    pub const fn with_melee(mut self, melee: MeleeView) -> Self {
        self.melee = melee;
        self
    }
}

/// Duel intent, in utility-table order (ties keep the earlier index).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    /// Close-range strike.
    Attack,
    /// Hold guard toward the foe.
    Block,
    /// Sidestep the incoming attack.
    Dodge,
    /// Close the distance.
    Advance,
    /// Give ground while facing the foe.
    Retreat,
    /// Wander/reposition strafe (anti-stuck when the foe is far).
    Reposition,
}

/// Generic integer-utility core: argmax over the five intent lanes in
/// [`Intent`] table order (attack, block, dodge, advance, retreat). Strict
/// `>` keeps the earlier index on ties, exactly like [`Brain::decide`].
/// Pure, total and allocation-free: the same scores always pick the same
/// intent. Budget: five comparisons, zero allocation.
pub fn pick_intent(scores: [i32; 5]) -> Intent {
    const ORDER: [Intent; INTENT_OUT_DIM] = [
        Intent::Attack,
        Intent::Block,
        Intent::Dodge,
        Intent::Advance,
        Intent::Retreat,
    ];
    let mut best = 0usize;
    for i in 1..scores.len() {
        if scores[i] > scores[best] {
            best = i;
        }
    }
    ORDER[best]
}

/// Base 0..1 lane scores from integer signals; attack is an AND of range and stamina. Units: scores 0..1. Errors: none. Deterministic.
fn thinker_base(dist_mm: i32, foe_attacking: bool, stamina: i32) -> [f32; 5] {
    let range_attack = if dist_mm <= Brain::ATTACK_MM {
        0.8
    } else if dist_mm <= Brain::CLOSE_MM {
        0.5
    } else if dist_mm <= Brain::FAR_MM {
        0.2
    } else {
        0.05
    };
    let stamina_attack = if stamina >= 400 {
        1.0
    } else if stamina >= 150 {
        0.6
    } else {
        0.2
    };
    let attack = crate::utility::all_or_nothing(&[range_attack, stamina_attack], 0.3);
    let block = if foe_attacking {
        if dist_mm <= Brain::CLOSE_MM {
            0.65
        } else {
            0.3
        }
    } else if dist_mm <= Brain::ATTACK_MM {
        0.2
    } else {
        0.08
    };
    let urgency = if foe_attacking { 1.0 } else { 0.3 };
    let reach = if foe_attacking {
        if dist_mm <= Brain::ATTACK_MM {
            0.75
        } else if dist_mm <= Brain::CLOSE_MM {
            0.45
        } else {
            0.15
        }
    } else {
        0.2
    };
    let dodge = crate::utility::product_of(&[urgency, reach]);
    let dist_pull = if dist_mm > Brain::FAR_MM {
        0.6
    } else if dist_mm > Brain::CLOSE_MM {
        0.4
    } else if dist_mm > Brain::ATTACK_MM {
        0.25
    } else {
        0.05
    };
    let stamina_push = if stamina < 150 { 0.1 } else { 0.0 };
    let advance = crate::utility::sum_of(&[dist_pull, stamina_push]);
    let retreat = if foe_attacking {
        if dist_mm <= Brain::ATTACK_MM {
            0.5
        } else {
            0.25
        }
    } else if dist_mm <= Brain::ATTACK_MM {
        0.2
    } else {
        0.1
    };
    [attack, block, dodge, advance, retreat]
}

/// Signed bias lane to a small f32 delta. Units: score delta. Errors: none. Deterministic.
fn bias_to_delta(bias: i32) -> f32 {
    let clamped = bias.clamp(-1000, 1000);
    if clamped >= 0 {
        let as_u16 = u16::try_from(clamped).unwrap_or(0);
        f32::from(as_u16) / 4000.0
    } else {
        let pos = clamped.saturating_neg();
        let as_u16 = u16::try_from(pos).unwrap_or(0);
        0.0 - f32::from(as_u16) / 4000.0
    }
}

/// Nerve-weighted lane scores in the `0..=1` utility band from integer signals:
/// the base table shaped by `weights`, then the depth bias as a small `f32`
/// delta. Pure and total; no allocation.
fn thinker_scores(
    dist_mm: i32,
    foe_attacking: bool,
    stamina: i32,
    weights: &crate::personality::Weights,
    bias: [i32; 5],
) -> [f32; 5] {
    let weighted = weights.apply(thinker_base(dist_mm, foe_attacking, stamina));
    [
        crate::utility::clamp01(weighted[0] + bias_to_delta(bias[0])),
        crate::utility::clamp01(weighted[1] + bias_to_delta(bias[1])),
        crate::utility::clamp01(weighted[2] + bias_to_delta(bias[2])),
        crate::utility::clamp01(weighted[3] + bias_to_delta(bias[3])),
        crate::utility::clamp01(weighted[4] + bias_to_delta(bias[4])),
    ]
}

/// Thinker-table intent from integer signals plus nerve weights and depth bias. Scores never enter any digest. Units: intent. Errors: none. Deterministic.
/// Caller: Brain::score_intent tick path.
///
/// The gated choice table ([`crate::utility::Thinker`]) is a cold-build item:
/// its two `Vec`s are built once, not per decision. The tick reads the same
/// five scored rows straight through [`crate::utility::pick_gated`], so a
/// decide allocates nothing. `Highest` with a zero threshold picks the argmax
/// with strict `>` (the earlier lane keeps a tie), exactly what the table
/// returned; the `_` arm is the table's otherwise intent.
pub fn thinker_intent(
    dist_mm: i32,
    foe_attacking: bool,
    stamina: i32,
    weights: &crate::personality::Weights,
    bias: [i32; 5],
) -> Intent {
    let scored = thinker_scores(dist_mm, foe_attacking, stamina, weights, bias);
    struct Fixed(f32);
    impl crate::utility::Scorer for Fixed {
        fn score(&self, _: &crate::Brief) -> f32 {
            self.0
        }
    }
    let (a, b, c, d, e) = (
        Fixed(scored[0]),
        Fixed(scored[1]),
        Fixed(scored[2]),
        Fixed(scored[3]),
        Fixed(scored[4]),
    );
    let rows: [&dyn crate::utility::Scorer; INTENT_OUT_DIM] = [&a, &b, &c, &d, &e];
    let brief = crate::Brief {
        tick: 0,
        entities: 0,
        bodies: 0,
        hash: 0,
    };
    match crate::utility::pick_gated(&rows, &brief, 0.0, crate::utility::Gate::Highest) {
        crate::utility::Gated::Pick(0) => Intent::Attack,
        crate::utility::Gated::Pick(1) => Intent::Block,
        crate::utility::Gated::Pick(2) => Intent::Dodge,
        crate::utility::Gated::Pick(3) => Intent::Advance,
        crate::utility::Gated::Pick(4) => Intent::Retreat,
        _ => Intent::Advance,
    }
}

/// Any brain on the tick: one [`Percept`] in, one validated
/// [`FighterInput`] out, deterministically. The arena holds one boxed or
/// inline policy per fighter and calls this seam without knowing which
/// brain runs behind it (scripted [`Brain`], net [`NetPolicy`], composed
/// [`Policy`], or a fixture's own script such as [`MarchPolicy`]).
/// Static dispatch only: no `dyn` per entity per tick.
///
/// [`FighterInput`]: engine_input::FighterInput
pub trait TickPolicy {
    /// Decide one [`FighterInput`] for `percept`. Always deterministic and
    /// always passes [`FighterInput::validate`].
    ///
    /// [`FighterInput::validate`]: engine_input::FighterInput::validate
    fn decide(&mut self, percept: &Percept) -> FighterInput;
    /// Last decided intent (initially [`Intent::Advance`]).
    fn intent(&self) -> Intent;
}

/// Gladiator policy: perception clock + integer utility + morale scalar.
///
/// `period_ticks` is the perception period (`decide` re-scores only when
/// `tick - last_eval >= period`; between evaluations it repeats the last
/// intent with the fresh tick). `morale` is `0..=1000` (500 neutral);
/// high morale shifts scores toward [`Intent::Attack`]/[`Intent::Advance`],
/// low morale toward [`Intent::Retreat`]/[`Intent::Block`]/[`Intent::Dodge`].
///
/// Replacement tuning arrives as one immutable [`AiConfig`] snapshot cloned
/// at match build: the aggression/caution lanes bias the same integer
/// scores and `engage_range_cm` replaces the strike threshold. The arena
/// owns the snapshot lifetime; matches never write curves back.
#[derive(Clone, Debug)]
pub struct Brain {
    fighter: FighterId,
    period_ticks: u32,
    morale: i32,
    last_eval_tick: Option<u32>,
    last_input: Option<FighterInput>,
    last_intent: Intent,
    far_streak: u32,
    config: AiConfig,
}

impl Brain {
    /// Ticks of consecutive far percepts (at 60 Hz) before reposition fires.
    pub const FAR_STREAK_TICKS: u32 = 180;
    /// Separation above which a percept counts as far (millimetres).
    pub const FAR_MM: i32 = 8_000;
    /// Strike range (millimetres).
    pub const ATTACK_MM: i32 = 2_500;
    /// Close range (millimetres).
    pub const CLOSE_MM: i32 = 4_000;
    /// Neutral morale.
    pub const MORALE_NEUTRAL: i32 = 500;
    /// Morale bounds.
    pub const MORALE_MIN: i32 = 0;
    /// Morale bounds.
    pub const MORALE_MAX: i32 = 1_000;

    /// New policy for `fighter`; `period_ticks == 0` means every tick.
    /// Loads the designer rows file `content/design/ai.rows` at compile
    /// time via [`parse_ai_config`](crate::curves_text::parse_ai_config)
    /// and falls back to the scripted curve defaults when rows refuse.
    /// Cold only (match build), never tick; same file gives same brain.
    pub fn new(fighter: FighterId, period_ticks: u32) -> Self {
        let text = include_str!("../../../content/design/ai.rows");
        Self::with_rows_text(fighter, period_ticks, text).unwrap_or_else(|_| {
            Self::with_config(fighter, period_ticks, AiConfig::scripted_defaults())
        })
    }

    /// Policy for `fighter` from an immutable [`AiConfig`] snapshot (the
    /// arena clones its match config here; live tuning builds a new snapshot
    /// instead of mutating a running one). Invalid curves or morale fall back
    /// to scripted defaults; valid configs start at `morale_default`.
    /// `period_ticks == 0` means every tick. Never panics.
    pub fn with_config(fighter: FighterId, period_ticks: u32, config: AiConfig) -> Self {
        let valid = config.effective();
        Self {
            fighter,
            period_ticks: period_ticks.max(1),
            morale: valid.morale_default,
            last_eval_tick: None,
            last_input: None,
            last_intent: Intent::Advance,
            far_streak: 0,
            config: valid,
        }
    }

    /// Policy for `fighter` from designer rows text (see
    /// [`parse_ai_config`](crate::curves_text::parse_ai_config)): the same
    /// snapshot [`with_config`](Self::with_config) would build from, so a
    /// bad line is refused with its 1-based [`AiRowsError`] line number
    /// instead of falling back to scripted defaults. `period_ticks == 0`
    /// means every tick. Never panics; integer math only, so the same text
    /// gives the same brain.
    pub fn with_rows_text(
        fighter: FighterId,
        period_ticks: u32,
        text: &str,
    ) -> Result<Self, AiRowsError> {
        let config = crate::curves_text::parse_ai_config(text)?;
        Ok(Self::with_config(fighter, period_ticks, config))
    }

    /// Make the next [`Brain::decide_with_bias`] re-score regardless of the
    /// perception clock (an interrupt: the caller saw something the clock
    /// would sit on). The far streak and held input are untouched.
    pub(crate) fn rearm(&mut self) {
        self.last_eval_tick = None;
    }

    /// Immutable configuration snapshot this brain decides from. Borrowed,
    /// never mutated: the arena clones it at match build.
    pub const fn config(&self) -> &AiConfig {
        &self.config
    }

    /// Current morale (`0..=1000`).
    pub const fn morale(&self) -> i32 {
        self.morale
    }

    /// Set morale, clamped to `0..=1000`. Returns the clamped value.
    pub fn set_morale(&mut self, morale: i32) -> i32 {
        self.morale = morale.clamp(Self::MORALE_MIN, Self::MORALE_MAX);
        self.morale
    }

    /// Last decided intent (initially [`Intent::Advance`]).
    pub const fn intent(&self) -> Intent {
        self.last_intent
    }

    /// Decide one [`FighterInput`] for `percept`.
    ///
    /// Re-scores on perception ticks only; between them the last intent is
    /// repeated with `tick` refreshed. Always deterministic and always
    /// passes [`FighterInput::validate`].
    ///
    /// [`FighterInput::validate`]: engine_input::FighterInput::validate
    pub fn decide(&mut self, percept: &Percept) -> FighterInput {
        self.decide_with_bias(percept, [0; 5])
    }

    /// Decide one [`FighterInput`] with an additive depth bias in utility
    /// order (attack, block, dodge, advance, retreat), applied with
    /// saturating adds after the far-streak anti-stuck check, so gladiator depth
    /// composes this core without forking it. Strict `>` keeps the earlier
    /// index on ties, exactly like [`Brain::decide`].
    ///
    /// [`FighterInput::validate`]: engine_input::FighterInput::validate
    pub fn decide_with_bias(&mut self, percept: &Percept, bias: [i32; 5]) -> FighterInput {
        let dist_mm = sanitize_dist_mm(percept.dist_m);
        if dist_mm > Self::FAR_MM {
            self.far_streak = self.far_streak.saturating_add(1);
        } else {
            self.far_streak = 0;
        }
        let due = match self.last_eval_tick {
            None => true,
            Some(last) => {
                // Out-of-order ticks re-evaluate; otherwise wait `period`.
                percept.tick < last || percept.tick.wrapping_sub(last) >= self.period_ticks
            }
        };
        if !due {
            if let Some(last) = self.last_input {
                let mut held = last;
                held.tick = percept.tick;
                return held;
            }
            // No history yet (unreachable after first eval); fall through.
        }
        let intent = self.score_intent(percept, dist_mm, bias);
        let facing = self.face_percept(percept);
        let input = emit(self.fighter, percept.tick, intent, facing);
        self.last_eval_tick = Some(percept.tick);
        self.last_input = Some(input);
        self.last_intent = intent;
        input
    }

    /// Integer utility choice plus a saturating depth bias. Strict `>`
    /// keeps the earlier index on ties.
    fn score_intent(&self, percept: &Percept, dist_mm: i32, bias: [i32; 5]) -> Intent {
        if self.far_streak >= Self::FAR_STREAK_TICKS {
            return Intent::Reposition;
        }
        let base = utility_scores_cfg(
            dist_mm,
            percept.foe_attacking,
            self.morale,
            &self.config.curves,
        );
        let scores = [
            base[0].saturating_add(bias[0]),
            base[1].saturating_add(bias[1]),
            base[2].saturating_add(bias[2]),
            base[3].saturating_add(bias[3]),
            base[4].saturating_add(bias[4]),
        ];
        if self.morale == Self::MORALE_NEUTRAL {
            pick_intent(scores)
        } else {
            let clamped = self.morale.clamp(Self::MORALE_MIN, Self::MORALE_MAX);
            let seen = u16::try_from(clamped).unwrap_or(500);
            let nerve = crate::personality::Personality::new(0, 1_200, seen);
            let weights = nerve.weights();
            thinker_intent(dist_mm, percept.foe_attacking, self.morale, &weights, bias)
        }
    }

    /// Facing toward the foe in centidegrees, fallback to own yaw/last.
    fn face_percept(&self, percept: &Percept) -> i16 {
        let (dx_mm, dz_mm) = delta_mm(percept);
        if let Some(face) = face_toward_cdeg(dx_mm, dz_mm) {
            return face;
        }
        let yaw_face = yaw_to_cdeg(sanitize_finite(percept.self_yaw, 0.0));
        if yaw_face != i16::MIN {
            return yaw_face;
        }
        self.last_input.map_or(0, |input| input.facing)
    }
}

impl Brain {
    /// NPC brain tick: drain due mind actions, then decide.
    ///
    /// Drain runs [`apply_due_into`](crate::apply::apply_due_into): actions
    /// due at `tick` leave `log` and sort into `batch` under the `requests`
    /// windows. Say/Gesture land in `batch.presented` for the presentation
    /// layer only (never steering); Approach/Leave/Idle land in
    /// `batch.intended`. Out-of-window stamps refuse into `batch.refused`
    /// with their cause (deadline overruns as timeout, inverted windows as
    /// past-deadline); nothing half-applies. The caller reuses `batch` to
    /// avoid per-tick allocation; steering itself is O(1).
    ///
    /// Steer runs [`resolve_drained`](Brain::resolve_drained): the latest
    /// intended movement (acceptance order) overrides the scored intent for
    /// this tick, emitted with the shared facing helper so the output
    /// always validates. With no intent the scored [`Brain::decide`] runs
    /// unchanged.
    pub fn decide_drained_into(
        &mut self,
        percept: &Percept,
        log: &mut ActionLog,
        tick: u64,
        requests: &[MindRequest],
        batch: &mut AppliedBatch,
    ) -> FighterInput {
        apply_due_into(log, tick, requests, batch);
        self.resolve_drained(percept, batch)
    }

    /// Allocating NPC brain tick: drains with
    /// [`apply_due`](crate::apply::apply_due), then decides exactly like
    /// [`Brain::decide_drained_into`]. Prefer the `_into` form on the hot
    /// tick with a reused batch.
    pub fn decide_drained(
        &mut self,
        percept: &Percept,
        log: &mut ActionLog,
        tick: u64,
        requests: &[MindRequest],
    ) -> (FighterInput, AppliedBatch) {
        let batch = apply_due(log, tick, requests);
        let input = self.resolve_drained(percept, &batch);
        (input, batch)
    }

    /// Steer from a drained batch, else fall back to the scored choice.
    /// Steered ticks mirror the far-streak accounting `decide` performs, so
    /// the anti-stuck clock counts exactly once per tick on every path.
    fn resolve_drained(&mut self, percept: &Percept, batch: &AppliedBatch) -> FighterInput {
        if let Some(intent) = batch.intended.last() {
            let dist_mm = sanitize_dist_mm(percept.dist_m);
            if dist_mm > Self::FAR_MM {
                self.far_streak = self.far_streak.saturating_add(1);
            } else {
                self.far_streak = 0;
            }
            let steer = intent.brain_intent();
            let facing = self.face_percept(percept);
            let input = emit(self.fighter, percept.tick, steer, facing);
            self.last_eval_tick = Some(percept.tick);
            self.last_input = Some(input);
            self.last_intent = steer;
            return input;
        }
        self.decide(percept)
    }
}

/// Quantized percept feature count: the only accepted net input dim.
/// [`features_from`] always emits this many `i8` lanes; [`NetPolicy::load`]
/// rejects any other input dim so the net and the percept stay in agreement.
pub const FEATURE_DIM: usize = 6;
/// Net output lanes decoded into intents: the only accepted net output dim.
/// Lane order matches the scripted utility table (attack, block, dodge,
/// advance, retreat); [`NetPolicy::load`] rejects any other output dim.
pub const INTENT_OUT_DIM: usize = 5;
/// Feature lane: foe-minus-self x in metres over [`FEATURE_FULL_GROUND_M`].
pub const LANE_DX: usize = 0;
/// Feature lane: foe-minus-self z in metres over [`FEATURE_FULL_GROUND_M`].
pub const LANE_DZ: usize = 1;
/// Feature lane: foe-minus-self y in metres over [`FEATURE_FULL_HEIGHT_M`].
pub const LANE_DY: usize = 2;
/// Feature lane: separation in metres over [`FEATURE_FULL_DIST_M`].
pub const LANE_DIST: usize = 3;
/// Feature lane: [`FEATURE_ATTACK_ON`] while the foe attacks, else 0.
pub const LANE_FOE_ATTACK: usize = 4;
/// Feature lane: own yaw in radians over pi.
pub const LANE_YAW: usize = 5;
/// Full scale (m) for the dx/dz lanes; wider gaps saturate at +/-127.
pub const FEATURE_FULL_GROUND_M: f32 = 16.0;
/// Full scale (m) for the dy lane; wider gaps saturate at +/-127.
pub const FEATURE_FULL_HEIGHT_M: f32 = 8.0;
/// Full scale (m) for the dist lane; wider gaps saturate at 127.
pub const FEATURE_FULL_DIST_M: f32 = 16.0;
/// Foe-attacking lane value while the foe is attacking (0 otherwise).
pub const FEATURE_ATTACK_ON: i8 = 127;

/// Symmetric quantization to `i8`: `value / full` clamped to `[-1, 1]`
/// times 127, truncated toward zero. Non-finite values and non-positive
/// scales map to 0; float-to-int `as` saturates by language rule.
fn quant_lane(value: f32, full: f32) -> i8 {
    if !value.is_finite() || !full.is_finite() || full <= 0.0 {
        return 0;
    }
    // `value` is finite and `full` is finite positive: the quotient is finite.
    let clamped = (value / full).clamp(-1.0, 1.0);
    (clamped * 127.0) as i8
}

/// Quantize one [`Percept`] into [`FEATURE_DIM`] `i8` lanes in `LANE_*`
/// order. Pure, total and bit-exact: arena-scale coords clamp, non-finite
/// or negative separations degrade to 0, and the tick never enters the
/// lanes (it only drives the deterministic sidestep in [`emit`]).
pub fn features_from(percept: &Percept) -> [i8; FEATURE_DIM] {
    let dx = sanitize_coord(percept.foe_pos[0]) - sanitize_coord(percept.self_pos[0]);
    let dz = sanitize_coord(percept.foe_pos[2]) - sanitize_coord(percept.self_pos[2]);
    let dy = sanitize_coord(percept.foe_pos[1]) - sanitize_coord(percept.self_pos[1]);
    let dist = if percept.dist_m.is_finite() && percept.dist_m > 0.0 {
        percept.dist_m
    } else {
        0.0
    };
    [
        quant_lane(dx, FEATURE_FULL_GROUND_M),
        quant_lane(dz, FEATURE_FULL_GROUND_M),
        quant_lane(dy, FEATURE_FULL_HEIGHT_M),
        quant_lane(dist, FEATURE_FULL_DIST_M),
        if percept.foe_attacking {
            FEATURE_ATTACK_ON
        } else {
            0
        },
        quant_lane(
            sanitize_finite(percept.self_yaw, 0.0),
            core::f32::consts::PI,
        ),
    ]
}

/// Argmax over the 5 intent lanes in scripted utility-table order
/// (attack, block, dodge, advance, retreat). Strict `>` keeps the earlier
/// index on ties, exactly like [`Brain::score_intent`]. Missing lanes read
/// as `i32::MIN` so short slices stay total (loaded nets always carry 5).
fn argmax_intent(logits: &[i32]) -> Intent {
    const ORDER: [Intent; INTENT_OUT_DIM] = [
        Intent::Attack,
        Intent::Block,
        Intent::Dodge,
        Intent::Advance,
        Intent::Retreat,
    ];
    let mut best = 0usize;
    for i in 1..INTENT_OUT_DIM {
        let cur = logits.get(i).copied().unwrap_or(i32::MIN);
        let top = logits.get(best).copied().unwrap_or(i32::MIN);
        if cur > top {
            best = i;
        }
    }
    ORDER[best]
}

/// Net-load failures: malformed bytes name the net-runtime cause, wrong dims name
/// expected vs got. Never panics, never blocks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NetPolicyError {
    /// Bytes are not a valid `.e2nn` file; net-runtime cause carried.
    Nn(engine_nn::NnErr),
    /// Net input dim differs from [`FEATURE_DIM`]; expected + got carried.
    BadInputDim {
        /// Required input dim ([`FEATURE_DIM`]).
        expected: usize,
        /// Net's declared input dim.
        got: usize,
    },
    /// Net output dim differs from [`INTENT_OUT_DIM`]; expected + got carried.
    BadOutputDim {
        /// Required output dim ([`INTENT_OUT_DIM`]).
        expected: usize,
        /// Net's declared output dim.
        got: usize,
    },
}

impl core::fmt::Display for NetPolicyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            NetPolicyError::Nn(cause) => write!(f, "net policy: {cause}"),
            NetPolicyError::BadInputDim { expected, got } => write!(
                f,
                "net policy: input dim {got} differs from features {expected}"
            ),
            NetPolicyError::BadOutputDim { expected, got } => write!(
                f,
                "net policy: output dim {got} differs from intents {expected}"
            ),
        }
    }
}

impl std::error::Error for NetPolicyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            NetPolicyError::Nn(cause) => Some(cause),
            _ => None,
        }
    }
}

impl From<engine_nn::NnErr> for NetPolicyError {
    fn from(cause: engine_nn::NnErr) -> Self {
        NetPolicyError::Nn(cause)
    }
}

/// Net-driven duel policy: quantized percept features through an
/// integer net, argmax over the 5 scored intents with the scripted
/// tie-break order, then the same [`emit`] mapping the scripted [`Brain`]
/// uses (so a `FighterInput` from either policy always validates).
///
/// Data owner: this handle owns the validated model plus the last intent
/// and facing for introspection and the coincident-position fallback; match
/// state stays in the arena. Determinism: integer-only net inference plus
/// the shared integer facing/emit helpers; same bytes plus same percepts
/// give byte-identical inputs. Differences from [`Brain`] by design: the
/// net re-scores every tick (no perception clock) and covers the 5 scored
/// intents only (no far-streak [`Intent::Reposition`]); compare fairly with
/// a period-1 scripted brain. Weights are hand-built in tests; no weights
/// ship with the crate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetPolicy {
    fighter: FighterId,
    model: engine_nn::Model,
    last_intent: Intent,
    last_facing: i16,
}

impl NetPolicy {
    /// Load and validate an `.e2nn` net for `fighter`: the net runtime parses the bytes,
    /// then the dims are checked against [`FEATURE_DIM`]/[`INTENT_OUT_DIM`].
    /// Same bytes twice give equal handles. Malformed bytes or wrong dims
    /// return [`NetPolicyError`], never panic.
    pub fn load(fighter: FighterId, bytes: &[u8]) -> Result<Self, NetPolicyError> {
        let model = engine_nn::load(bytes)?;
        if model.input_dim() != FEATURE_DIM {
            return Err(NetPolicyError::BadInputDim {
                expected: FEATURE_DIM,
                got: model.input_dim(),
            });
        }
        if model.output_dim() != INTENT_OUT_DIM {
            return Err(NetPolicyError::BadOutputDim {
                expected: INTENT_OUT_DIM,
                got: model.output_dim(),
            });
        }
        Ok(Self {
            fighter,
            model,
            last_intent: Intent::Advance,
            last_facing: 0,
        })
    }

    /// Bound fighter slot.
    pub const fn fighter(&self) -> FighterId {
        self.fighter
    }

    /// Last decided intent (initially [`Intent::Advance`]).
    pub const fn intent(&self) -> Intent {
        self.last_intent
    }

    /// Score one percept without updating history: quantize, infer, argmax.
    pub fn intent_for(&self, percept: &Percept) -> Intent {
        argmax_intent(&engine_nn::infer(&self.model, &features_from(percept)))
    }

    /// Facing toward the foe in centidegrees, fallback to own yaw then the
    /// last facing (the scripted [`Brain`] falls back to its last input
    /// instead; both agree whenever the fighters do not coincide).
    fn facing_for(&self, percept: &Percept) -> i16 {
        facing_toward(percept, self.last_facing)
    }

    /// Decide one [`FighterInput`] for `percept`. Always deterministic and
    /// always passes [`FighterInput::validate`].
    ///
    /// [`FighterInput::validate`]: engine_input::FighterInput::validate
    pub fn decide(&mut self, percept: &Percept) -> FighterInput {
        let intent = self.intent_for(percept);
        let input = emit(self.fighter, percept.tick, intent, self.facing_for(percept));
        self.last_intent = intent;
        self.last_facing = input.facing;
        input
    }
}

/// Shipped duel policy: scripted [`Brain`] or net-driven [`NetPolicy`].
/// The net variant exists only after [`NetPolicy::load`] validated the
/// bytes; construction from untrusted bytes goes through
/// [`Policy::load_or_scripted`], which falls back to scripted on
/// absent/invalid nets instead of panicking. Consumer: the duel tournament
/// picks the shipped policy with this enum.
#[derive(Clone, Debug)]
pub enum Policy {
    /// Scripted integer-utility brain.
    Scripted(Brain),
    /// Net-driven brain.
    Net(NetPolicy),
}

impl Policy {
    /// Build the net policy for `fighter` from `bytes`, falling back to a
    /// scripted brain (period 1, neutral morale) when `bytes` is `None` or
    /// fails validation. Never panics.
    pub fn load_or_scripted(fighter: FighterId, bytes: Option<&[u8]>) -> Self {
        match bytes {
            Some(raw) => match NetPolicy::load(fighter, raw) {
                Ok(net) => Policy::Net(net),
                Err(_) => Policy::Scripted(Brain::new(fighter, 1)),
            },
            None => Policy::Scripted(Brain::new(fighter, 1)),
        }
    }

    /// True for the net-driven variant.
    pub const fn is_net(&self) -> bool {
        match self {
            Policy::Scripted(_) => false,
            Policy::Net(_) => true,
        }
    }

    /// Last decided intent (initially [`Intent::Advance`] on both variants).
    pub const fn intent(&self) -> Intent {
        match self {
            Policy::Scripted(brain) => brain.intent(),
            Policy::Net(net) => net.intent(),
        }
    }

    /// Decide one [`FighterInput`] for `percept`. Always deterministic and
    /// always passes [`FighterInput::validate`].
    ///
    /// [`FighterInput::validate`]: engine_input::FighterInput::validate
    pub fn decide(&mut self, percept: &Percept) -> FighterInput {
        match self {
            Policy::Scripted(brain) => brain.decide(percept),
            Policy::Net(net) => net.decide(percept),
        }
    }
}

impl TickPolicy for Brain {
    fn decide(&mut self, percept: &Percept) -> FighterInput {
        Brain::decide(self, percept)
    }

    fn intent(&self) -> Intent {
        Brain::intent(self)
    }
}

impl TickPolicy for NetPolicy {
    fn decide(&mut self, percept: &Percept) -> FighterInput {
        NetPolicy::decide(self, percept)
    }

    fn intent(&self) -> Intent {
        NetPolicy::intent(self)
    }
}

impl TickPolicy for Policy {
    fn decide(&mut self, percept: &Percept) -> FighterInput {
        Policy::decide(self, percept)
    }

    fn intent(&self) -> Intent {
        Policy::intent(self)
    }
}

/// Non-gladiator scripted policy: marches at the foe every tick. No
/// utility scores, no morale, no curves, no phases, no feints — the
/// fixture-genre shape (a different brain on the same tick). Owns only its
/// fighter slot plus the last facing for the coincident-position fallback;
/// match state stays in the arena. Determinism: the shared integer
/// facing/emit helpers; same percepts give byte-identical inputs. Budget:
/// a dozen integer ops with zero allocation.
#[derive(Clone, Debug)]
pub struct MarchPolicy {
    fighter: FighterId,
    last_facing: i16,
}

impl MarchPolicy {
    /// New march policy for `fighter`. Never panics.
    pub const fn new(fighter: FighterId) -> Self {
        Self {
            fighter,
            last_facing: 0,
        }
    }

    /// Bound fighter slot.
    pub const fn fighter(&self) -> FighterId {
        self.fighter
    }

    /// Facing toward the foe in centidegrees, fallback to own yaw then the
    /// last facing (the same fallback chain [`NetPolicy`] uses, so every
    /// policy agrees whenever the fighters do not coincide).
    fn facing_for(&self, percept: &Percept) -> i16 {
        facing_toward(percept, self.last_facing)
    }

    /// Decide one [`FighterInput`] for `percept`: always
    /// [`Intent::Advance`], faced at the foe. Always deterministic and
    /// always passes [`FighterInput::validate`].
    ///
    /// [`FighterInput::validate`]: engine_input::FighterInput::validate
    pub fn decide(&mut self, percept: &Percept) -> FighterInput {
        let input = emit(
            self.fighter,
            percept.tick,
            Intent::Advance,
            self.facing_for(percept),
        );
        self.last_facing = input.facing;
        input
    }

    /// Last decided facing is [`Intent::Advance`] by construction (the
    /// initial report before the first decide).
    pub const fn intent(&self) -> Intent {
        Intent::Advance
    }
}

impl TickPolicy for MarchPolicy {
    fn decide(&mut self, percept: &Percept) -> FighterInput {
        MarchPolicy::decide(self, percept)
    }

    fn intent(&self) -> Intent {
        MarchPolicy::intent(self)
    }
}

/// Fork-comparison outcome: same percept script through two policies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Comparison {
    /// Percepts compared.
    pub total: usize,
    /// Percepts where both policies emitted the same [`FighterInput`].
    pub agree: usize,
    /// `agree * 1000 / total` (1000 when the script is empty).
    pub agree_rate_permille: u32,
    /// First script index where the policies diverged, if any.
    pub first_divergent_index: Option<usize>,
}

/// Run the same percept script through both policies and report agreement.
/// Deterministic: same policies plus same script give the same report.
/// An empty script agrees vacuously (rate 1000, no divergence).
pub fn compare(a: &mut Policy, b: &mut Policy, script: &[Percept]) -> Comparison {
    let mut agree = 0usize;
    let mut first_divergent_index: Option<usize> = None;
    for (index, percept) in script.iter().enumerate() {
        if a.decide(percept) == b.decide(percept) {
            agree += 1;
        } else if first_divergent_index.is_none() {
            first_divergent_index = Some(index);
        }
    }
    let agree_rate_permille = if script.is_empty() {
        1000
    } else {
        (agree * 1000 / script.len()) as u32
    };
    Comparison {
        total: script.len(),
        agree,
        agree_rate_permille,
        first_divergent_index,
    }
}

/// Fork-compare two AI configurations before a replacement ships: the same
/// percept script runs through a scripted brain per config (period 1, so the
/// perception clock cannot mask a curve change) and the policy [`compare`]
/// harness reports the agreement rate. Invalid configs fall back to scripted
/// defaults inside [`Brain::with_config`], so comparing garbage against the
/// defaults agrees everywhere. Deterministic: same configs plus same script
/// give the same report.
pub fn compare_curves(a: &AiConfig, b: &AiConfig, script: &[Percept]) -> Comparison {
    let mut left = Policy::Scripted(Brain::with_config(FighterId(0), 1, *a));
    let mut right = Policy::Scripted(Brain::with_config(FighterId(0), 1, *b));
    compare(&mut left, &mut right, script)
}

/// Vectorized 2P zero-sum neutral-play self-play harness: batch bouts of two
/// duel policies to move win-rate against a heuristic baseline.
///
/// Shape: the duel tournament (consumer) builds one policy pair per bout and
/// calls [`self_play_batch`] with slices in and a [`SelfPlayReport`] out.
/// Ticks run lockstep across the batch (outer ticks, inner pairs), so the
/// batch itself is the vector width. Geometry is neutral every tick (mirrored
/// positions, one shared separation, neutral yaws), and scoring is zero-sum
/// per bout (a win for A is a loss for B). Determinism: same initial
/// policies plus same [`SelfPlayConfig`] give the same report; the harness
/// reads no clock, so throughput is the caller's job through
/// [`SelfPlayReport::steps_per_sec`]. Budget: three small per-bout vecs sized
/// by the batch (harness only); the per-tick policy step itself never
/// allocates.
///
/// Attribution: training shape idea-only from como-research/FootsiesGym
/// (GPL-3.0-only, https://github.com/como-research/FootsiesGym) — idea-only,
/// reimplemented fresh in our style; no donor code is pasted here.
///
/// Batch width cap: 256 bouts keep the harness report small and fast.
pub const MAX_SELF_PLAY_BOUTS: usize = 256;
/// Bout length cap: 3_600 ticks cover a 60 s bout at 60 Hz.
pub const MAX_SELF_PLAY_TICKS: u32 = 3_600;

/// Self-play configuration failures: out-of-range batch shapes only.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelfPlayError {
    /// `bouts == 0`; at least one bout is required.
    EmptyBouts,
    /// `bouts` exceeds [`MAX_SELF_PLAY_BOUTS`]; culprit carried.
    TooManyBouts {
        /// Requested bout count.
        got: usize,
    },
    /// `ticks_per_bout == 0`; at least one tick is required.
    EmptyTicks,
    /// `ticks_per_bout` exceeds [`MAX_SELF_PLAY_TICKS`]; culprit carried.
    TooManyTicks {
        /// Requested tick count.
        got: u32,
    },
}

impl core::fmt::Display for SelfPlayError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SelfPlayError::EmptyBouts => write!(f, "self-play: need at least one bout"),
            SelfPlayError::TooManyBouts { got } => {
                write!(f, "self-play: {got} bouts exceed {MAX_SELF_PLAY_BOUTS}")
            }
            SelfPlayError::EmptyTicks => write!(f, "self-play: need at least one tick"),
            SelfPlayError::TooManyTicks { got } => {
                write!(f, "self-play: {got} ticks exceed {MAX_SELF_PLAY_TICKS}")
            }
        }
    }
}

impl std::error::Error for SelfPlayError {}

/// Batch self-play shape: bout count, bout length and replay seed.
///
/// `bouts` is `1..=MAX_SELF_PLAY_BOUTS`, `ticks_per_bout` is
/// `1..=MAX_SELF_PLAY_TICKS` (60 Hz fixed steps). Out-of-range shapes fail
/// with [`SelfPlayError`]; construction never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SelfPlayConfig {
    /// Bouts in the batch.
    pub bouts: usize,
    /// Ticks per bout.
    pub ticks_per_bout: u32,
    /// Seed mixed into the neutral geometry; same seed replays identically.
    pub seed: u64,
}

impl SelfPlayConfig {
    /// Checked batch shape; out-of-range shapes return [`SelfPlayError`].
    /// Never panics.
    pub const fn new(bouts: usize, ticks_per_bout: u32, seed: u64) -> Result<Self, SelfPlayError> {
        if bouts == 0 {
            return Err(SelfPlayError::EmptyBouts);
        }
        if bouts > MAX_SELF_PLAY_BOUTS {
            return Err(SelfPlayError::TooManyBouts { got: bouts });
        }
        if ticks_per_bout == 0 {
            return Err(SelfPlayError::EmptyTicks);
        }
        if ticks_per_bout > MAX_SELF_PLAY_TICKS {
            return Err(SelfPlayError::TooManyTicks {
                got: ticks_per_bout,
            });
        }
        Ok(Self {
            bouts,
            ticks_per_bout,
            seed,
        })
    }
}

/// Zero-sum bout outcome: a win for one side is a loss for the other.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BoutOutcome {
    /// Side A scored more clean hits.
    WinA,
    /// Side B scored more clean hits.
    WinB,
    /// Equal hits (including scoreless).
    Draw,
}

/// Batch self-play report: zero-sum wins plus the evaluated step count.
///
/// `wins_a + wins_b + draws == bouts`, and `win_rate_a_permille` is
/// `wins_a * 1000 / bouts` (a sweep reads 1000, all draws read 0).
/// `eval_steps` counts both policy steps per tick. Every field is a pure
/// function of the pairs and the config, so equal inputs give equal reports.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SelfPlayReport {
    /// Bouts actually run (`pairs.len()`).
    pub bouts: usize,
    /// Ticks run per bout.
    pub ticks_per_bout: u32,
    /// Bouts won by side A.
    pub wins_a: usize,
    /// Bouts won by side B.
    pub wins_b: usize,
    /// Drawn bouts.
    pub draws: usize,
    /// `wins_a * 1000 / bouts` (0 when the batch is empty).
    pub win_rate_a_permille: u32,
    /// Policy steps evaluated (`bouts * ticks * 2`).
    pub eval_steps: u64,
}

impl SelfPlayReport {
    /// Evaluated steps per second over a wall time the caller measured around
    /// [`self_play_batch`]. Integer math, saturating; a zero `elapsed` (too
    /// fast to time) reports `eval_steps`. Throughput only, never an outcome.
    #[must_use]
    pub fn steps_per_sec(&self, elapsed: std::time::Duration) -> u64 {
        let nanos = elapsed.as_nanos();
        if nanos == 0 {
            return self.eval_steps;
        }
        let rate = u128::from(self.eval_steps).saturating_mul(1_000_000_000) / nanos;
        u64::try_from(rate).unwrap_or(u64::MAX)
    }
}

/// Neutral mirrored percepts for one `(bout, tick)`: symmetric positions
/// about the origin, one shared separation, neutral yaws. Separation cycles
/// 1.0..13.0 m from the seeded integer mix (no RNG state, no trig), so
/// neither side is favoured and the same `(seed, bout, tick)` replays
/// identically. `foe_a`/`foe_b` carry the rival last attack flag.
fn neutral_pair(bout: u64, tick: u32, foe_a: bool, foe_b: bool, seed: u64) -> (Percept, Percept) {
    let mixed = engine_core::splitmix64(
        seed.wrapping_add(bout.wrapping_mul(engine_core::hash::GOLDEN_GAMMA))
            .wrapping_add(u64::from(tick).wrapping_mul(engine_core::hash::MIX64_MUL_A)),
    );
    // `mixed % 120` is 0..120, so separation stays in 1.0..13.0 m.
    let lane = (mixed % 120) as f32;
    let dist_m = 1.0 + lane * 0.1;
    let half = dist_m * 0.5;
    (
        Percept::new(
            [-half, 0.0, 0.0],
            0.0,
            [half, 0.0, 0.0],
            foe_a,
            dist_m,
            tick,
        ),
        Percept::new(
            [half, 0.0, 0.0],
            0.0,
            [-half, 0.0, 0.0],
            foe_b,
            dist_m,
            tick,
        ),
    )
}

/// A strike lands only in range against an unguarded rival: the attacker
/// presses attack while separated by at most [`Brain::ATTACK_MM`] and the
/// defender neither blocks nor dodges. Integer range check only.
fn clean_hit(attacks: bool, defender: Buttons, dist_mm: i32) -> bool {
    attacks
        && dist_mm <= Brain::ATTACK_MM
        && defender != Buttons(BLOCK)
        && defender != Buttons(DODGE)
}

/// One deterministic 2P bout: mirrored neutral percepts both sides, rival
/// attack flags fed back each tick, clean hits tallied, most hits wins.
/// Same policies plus same `(bout, ticks, seed)` give the same outcome.
pub fn self_play_bout<A: TickPolicy, B: TickPolicy>(
    a: &mut A,
    b: &mut B,
    bout: u64,
    ticks: u32,
    seed: u64,
) -> BoutOutcome {
    let mut hits_a = 0u32;
    let mut hits_b = 0u32;
    let (mut foe_a, mut foe_b) = (false, false);
    for tick in 0..ticks {
        let (pa, pb) = neutral_pair(bout, tick, foe_a, foe_b, seed);
        let dist_mm = sanitize_dist_mm(pa.dist_m);
        let out_a = a.decide(&pa);
        let out_b = b.decide(&pb);
        let atk_a = out_a.buttons == Buttons(ATTACK);
        let atk_b = out_b.buttons == Buttons(ATTACK);
        if clean_hit(atk_a, out_b.buttons, dist_mm) {
            hits_a = hits_a.saturating_add(1);
        }
        if clean_hit(atk_b, out_a.buttons, dist_mm) {
            hits_b = hits_b.saturating_add(1);
        }
        foe_a = atk_b;
        foe_b = atk_a;
    }
    if hits_a > hits_b {
        BoutOutcome::WinA
    } else if hits_b > hits_a {
        BoutOutcome::WinB
    } else {
        BoutOutcome::Draw
    }
}

/// Batch self-play across bout pairs: ticks run lockstep over the batch and
/// each bout keeps its own hit and foe state, so the batch is the vector
/// width (slices in, one report out). `config.ticks_per_bout` and
/// `config.seed` drive every bout; the report counts `pairs.len()` bouts.
/// Same pairs plus same config give the same report. Harness allocates its
/// three per-bout vecs only; it reads no clock.
pub fn self_play_batch<A: TickPolicy, B: TickPolicy>(
    pairs: &mut [(A, B)],
    config: &SelfPlayConfig,
) -> SelfPlayReport {
    let mut hits_a = vec![0u32; pairs.len()];
    let mut hits_b = vec![0u32; pairs.len()];
    let mut foe_a = vec![false; pairs.len()];
    let mut foe_b = vec![false; pairs.len()];
    for tick in 0..config.ticks_per_bout {
        for (index, (a, b)) in pairs.iter_mut().enumerate() {
            let bout = u64::try_from(index).unwrap_or(u64::MAX);
            let (pa, pb) = neutral_pair(bout, tick, foe_a[index], foe_b[index], config.seed);
            let dist_mm = sanitize_dist_mm(pa.dist_m);
            let out_a = a.decide(&pa);
            let out_b = b.decide(&pb);
            let atk_a = out_a.buttons == Buttons(ATTACK);
            let atk_b = out_b.buttons == Buttons(ATTACK);
            if clean_hit(atk_a, out_b.buttons, dist_mm) {
                hits_a[index] = hits_a[index].saturating_add(1);
            }
            if clean_hit(atk_b, out_a.buttons, dist_mm) {
                hits_b[index] = hits_b[index].saturating_add(1);
            }
            foe_a[index] = atk_b;
            foe_b[index] = atk_a;
        }
    }
    let (mut wins_a, mut wins_b, mut draws) = (0usize, 0usize, 0usize);
    for index in 0..pairs.len() {
        if hits_a[index] > hits_b[index] {
            wins_a += 1;
        } else if hits_b[index] > hits_a[index] {
            wins_b += 1;
        } else {
            draws += 1;
        }
    }
    let bouts = pairs.len();
    // Checked narrow: reachable permille is 0..=1000 so the `TryFrom`
    // always succeeds; `MAX` only fires on an unreachable overflow.
    let win_rate_a_permille =
        u32::try_from((wins_a * 1000).checked_div(bouts).unwrap_or(0)).unwrap_or(u32::MAX);
    let eval_steps = (bouts as u64)
        .saturating_mul(u64::from(config.ticks_per_bout))
        .saturating_mul(2);
    SelfPlayReport {
        bouts,
        ticks_per_bout: config.ticks_per_bout,
        wins_a,
        wins_b,
        draws,
        win_rate_a_permille,
        eval_steps,
    }
}

/// Integer utility scores in [`Intent`] table order
/// (attack, block, dodge, advance, retreat) under one curve set.
///
/// Base bands match the scripted table, except the strike threshold is the
/// curve-set engage range instead of [`Brain::ATTACK_MM`]. Morale resolves
/// through the aggression/caution lanes (`-500..=500` bias each around a
/// neutral 500): aggression drives attack/advance and drags retreat, caution
/// steadies guard/dodge/retreat. With the scripted defaults this reproduces
/// the scripted scores up to lane quantization. Integer division only.
fn utility_scores_cfg(
    dist_mm: i32,
    foe_attacking: bool,
    morale: i32,
    curves: &AiCurves,
) -> [i32; 5] {
    let aggression_bias = curves.aggression_bias_at(morale);
    let caution_bias = curves.caution_bias_at(morale);
    let engage_mm = curves.engage_range_mm();

    let mut attack = if dist_mm <= engage_mm {
        700
    } else if dist_mm <= Brain::CLOSE_MM {
        500
    } else if dist_mm <= Brain::FAR_MM {
        200
    } else {
        50
    };
    if foe_attacking {
        attack -= 150;
    }
    attack += aggression_bias / 2;

    let mut block = if foe_attacking {
        if dist_mm <= Brain::CLOSE_MM {
            650
        } else {
            300
        }
    } else if dist_mm <= engage_mm {
        200
    } else {
        80
    };
    block += caution_bias / 4 - aggression_bias / 4;

    let mut dodge = if foe_attacking {
        if dist_mm <= engage_mm {
            750
        } else if dist_mm <= Brain::CLOSE_MM {
            450
        } else {
            150
        }
    } else {
        60
    };
    dodge += caution_bias / 4 - aggression_bias / 4;

    let mut advance = if dist_mm > Brain::FAR_MM {
        600
    } else if dist_mm > Brain::CLOSE_MM {
        400
    } else if dist_mm > engage_mm {
        250
    } else {
        50
    };
    advance += aggression_bias / 4;

    let mut retreat = if foe_attacking {
        if dist_mm <= engage_mm {
            500
        } else {
            250
        }
    } else if dist_mm <= engage_mm {
        200
    } else {
        100
    };
    retreat += caution_bias / 4 - aggression_bias / 2;

    [attack, block, dodge, advance, retreat]
}

/// Map an intent to a validated [`FighterInput`].
fn emit(fighter: FighterId, tick: u32, intent: Intent, facing: i16) -> FighterInput {
    let (mx, mz, buttons) = match intent {
        Intent::Attack => (30, 0, Buttons(ATTACK)),
        Intent::Block => (0, 0, Buttons(BLOCK)),
        Intent::Dodge => {
            // Deterministic sidestep: alternate with the perception tick.
            let side = if tick.is_multiple_of(2) { 100 } else { -100 };
            (0, side, Buttons(DODGE))
        }
        Intent::Advance => (100, 0, Buttons::NONE),
        Intent::Retreat => (-80, 0, Buttons::NONE),
        // Deterministic wander: strafe flips every 30 ticks so the
        // fighter keeps moving (anti-stuck) without any RNG.
        Intent::Reposition => {
            let side = if (tick / 30).is_multiple_of(2) {
                80
            } else {
                -80
            };
            (60, side, Buttons::NONE)
        }
    };
    FighterInput {
        tick,
        fighter,
        mx,
        mz,
        facing: facing.clamp(-FACING_RANGE, FACING_RANGE),
        buttons,
    }
}

/// Facing toward the foe in centidegrees; falls back to own yaw, then to
/// `last_facing` when the fighters coincide and the yaw is unusable. The one
/// fallback chain every net and march policy shares.
fn facing_toward(percept: &Percept, last_facing: i16) -> i16 {
    let (dx_mm, dz_mm) = delta_mm(percept);
    if let Some(face) = face_toward_cdeg(dx_mm, dz_mm) {
        return face;
    }
    let yaw_face = yaw_to_cdeg(sanitize_finite(percept.self_yaw, 0.0));
    if yaw_face != i16::MIN {
        return yaw_face;
    }
    last_facing
}

/// Millimetre separation from `dist_m`; NaN/inf/negative fall back to mid-range.
pub(crate) fn sanitize_dist_mm(dist_m: f32) -> i32 {
    if !dist_m.is_finite() || dist_m < 0.0 {
        return 10_000;
    }
    let clamped = if dist_m > 60.0 { 60.0 } else { dist_m };
    // `clamped` is finite and bounded: truncating cast cannot wrap.
    (clamped * 1000.0) as i32
}

/// Replace NaN/inf with `default`.
fn sanitize_finite(x: f32, default: f32) -> f32 {
    if x.is_finite() {
        x
    } else {
        default
    }
}

/// Clamp one coordinate to arena scale, then keep it finite.
fn sanitize_coord(x: f32) -> f32 {
    // Input is finite here (NaN/inf replaced above), so `clamp` cannot panic.
    sanitize_finite(x, 0.0).clamp(-1_000.0, 1_000.0)
}

/// Foe-minus-self delta in millimetres on the ground plane (x, z).
fn delta_mm(percept: &Percept) -> (i64, i64) {
    let dx = sanitize_coord(percept.foe_pos[0]) - sanitize_coord(percept.self_pos[0]);
    let dz = sanitize_coord(percept.foe_pos[2]) - sanitize_coord(percept.self_pos[2]);
    ((dx * 1000.0) as i64, (dz * 1000.0) as i64)
}

/// Integer facing toward `(dx_mm, dz_mm)` in centidegrees.
///
/// Convention: 0 = +Z (north), +9000 = +X (east), +/-18000 = south.
/// Piecewise-linear pseudo-angle from the dominant axis; `None` when the
/// fighters coincide. Integer division only, no transcendental calls.
fn face_toward_cdeg(dx_mm: i64, dz_mm: i64) -> Option<i16> {
    if dx_mm == 0 && dz_mm == 0 {
        return None;
    }
    let ax = dx_mm.abs();
    let az = dz_mm.abs();
    // `denom > 0` because at least one axis is nonzero.
    let denom = ax.saturating_add(az).max(1);
    // Angle from the Z axis, 0..=9000, linear within the octant.
    let from_z = if az >= ax {
        (ax.saturating_mul(9_000) / denom).clamp(0, 9_000)
    } else {
        9_000 - (az.saturating_mul(9_000) / denom).clamp(0, 9_000)
    };
    let cdeg: i64 = match (dx_mm >= 0, dz_mm > 0) {
        (true, true) => from_z,            // NE: 0..9000
        (true, false) => 18_000 - from_z,  // SE: 9000..18000
        (false, false) => from_z - 18_000, // SW: -18000..-9000
        (false, true) => -from_z,          // NW: -9000..0
    };
    Some(cdeg.clamp(-18_000, 18_000) as i16)
}

/// Yaw (radians) to centidegrees with integer wrap; `i16::MIN` on overflow.
///
/// Linear scale only (`18000/pi`); no trigonometry.
fn yaw_to_cdeg(yaw_rad: f32) -> i16 {
    const SCALE: f32 = 5_729.578; // 180*100/pi, f32 precision
    let raw = yaw_rad * SCALE;
    if !raw.is_finite() {
        return 0;
    }
    // `raw` is finite here, so `clamp` cannot panic.
    let clamped = raw.clamp(-9_000_000.0, 9_000_000.0);
    let raw_i = clamped as i32;
    let wrapped = ((raw_i + 18_000) % 36_000 + 36_000) % 36_000 - 18_000;
    if wrapped < i32::from(i16::MIN) || wrapped > i32::from(i16::MAX) {
        i16::MIN
    } else {
        wrapped as i16
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::personality::Weights;
    use std::time::Duration;

    fn fighter() -> FighterId {
        FighterId::new(0).expect("slot 0 is legal")
    }

    fn percept(tick: u32, dist_m: f32, foe_attacking: bool) -> Percept {
        Percept::new(
            [0.0, 0.0, 0.0],
            0.0,
            [dist_m, 0.0, 0.0],
            foe_attacking,
            dist_m,
            tick,
        )
    }

    /// Fixed 60 s script: positions/dist/attack derive from the tick index
    /// with integer math only, so the script itself is deterministic.
    fn script_percept(tick: u32) -> Percept {
        let lane = (tick % 7) as f32;
        let self_pos = [(tick % 13) as f32 * 0.5, 0.0, (tick % 11) as f32 * -0.25];
        let foe_pos = [(tick % 17) as f32 * 0.75 - 4.0, 0.0, lane - 3.0];
        // Pseudo-distance cycles near/far without any RNG or trig.
        let dist_m = 1.0 + ((tick * 37) % 120) as f32 * 0.1;
        Percept::new(
            self_pos,
            (tick % 5) as f32 * 0.3,
            foe_pos,
            tick.is_multiple_of(3),
            dist_m,
            tick,
        )
    }

    fn run_script(period: u32, morale: i32) -> Vec<FighterInput> {
        let mut brain = Brain::new(fighter(), period);
        brain.set_morale(morale);
        (0..3_600)
            .map(|tick| brain.decide(&script_percept(tick)))
            .collect()
    }

    #[test]
    fn scripted_60s_identical_twice() {
        let first = run_script(6, Brain::MORALE_NEUTRAL);
        let second = run_script(6, Brain::MORALE_NEUTRAL);
        assert_eq!(first.len(), 3_600);
        assert_eq!(first, second);
        for input in &first {
            input.validate().expect("every brain output validates");
        }
        // Sanity: the script exercises more than one intent.
        let mut kinds = first
            .iter()
            .map(|input| input.buttons.0)
            .collect::<Vec<_>>();
        kinds.sort_unstable();
        kinds.dedup();
        assert!(kinds.len() >= 2, "script should vary buttons: {kinds:?}");
    }

    #[test]
    fn aggression_differs_with_morale_extremes() {
        // Close and under attack: the brave fighter strikes, the
        // terrified one guards or evades instead.
        let probe = percept(0, 2.0, true);
        let mut brave = Brain::new(fighter(), 1);
        brave.set_morale(Brain::MORALE_MAX);
        let mut coward = Brain::new(fighter(), 1);
        coward.set_morale(Brain::MORALE_MIN);
        let (brave_out, coward_out) = (brave.decide(&probe), coward.decide(&probe));
        brave_out.validate().expect("brave output validates");
        coward_out.validate().expect("coward output validates");
        assert_eq!(brave_out.buttons, Buttons(ATTACK), "{brave_out:?}");
        assert_ne!(brave_out, coward_out, "morale must shift the choice");
        assert!(
            coward_out.buttons == Buttons(BLOCK)
                || coward_out.buttons == Buttons(DODGE)
                || coward_out.mx < 0,
            "coward defends or retreats: {coward_out:?}"
        );
    }

    /// The pre-change thinker path: the same five scored rows fed through the
    /// two-`Vec` [`crate::utility::Thinker`] table per call, kept as the oracle
    /// the flat read must equal on every fixture.
    fn thinker_intent_table(
        dist_mm: i32,
        foe_attacking: bool,
        stamina: i32,
        weights: &Weights,
        bias: [i32; 5],
    ) -> Intent {
        let scored = thinker_scores(dist_mm, foe_attacking, stamina, weights, bias);
        struct Fixed(f32);
        impl crate::utility::Scorer for Fixed {
            fn score(&self, _: &crate::Brief) -> f32 {
                self.0
            }
        }
        let (a, b, c, d, e) = (
            Fixed(scored[0]),
            Fixed(scored[1]),
            Fixed(scored[2]),
            Fixed(scored[3]),
            Fixed(scored[4]),
        );
        let table = crate::utility::Thinker::new(3)
            .when(&a, 0)
            .when(&b, 1)
            .when(&c, 2)
            .when(&d, 3)
            .when(&e, 4);
        let brief = crate::Brief {
            tick: 0,
            entities: 0,
            bodies: 0,
            hash: 0,
        };
        match table.decide(&brief, 0.0, crate::utility::Gate::Highest) {
            0 => Intent::Attack,
            1 => Intent::Block,
            2 => Intent::Dodge,
            3 => Intent::Advance,
            4 => Intent::Retreat,
            _ => Intent::Advance,
        }
    }

    /// Lane index for the sweep tally (`0..=5`); no cast.
    fn intent_tag(intent: Intent) -> usize {
        match intent {
            Intent::Attack => 0,
            Intent::Block => 1,
            Intent::Dodge => 2,
            Intent::Advance => 3,
            Intent::Retreat => 4,
            Intent::Reposition => 5,
        }
    }

    /// The distance, attack, stamina, nerve and bias sweep: the flat read
    /// answers exactly what the per-call table build answered on every fixture,
    /// and the sweep really does reach more than one lane.
    #[test]
    fn thinker_intent_matches_the_table_oracle_on_a_sweep() {
        let mut lanes = [0u32; 6];
        for dist_mm in [0, 1_000, 2_500, 4_000, 8_000, 9_000, 40_000] {
            for foe_attacking in [false, true] {
                for stamina in [0, 100, 400, 1_000] {
                    for weights in [Weights::bold(), Weights::neutral(), Weights::shy()] {
                        for bias in [[0; 5], [900, -900, 0, 0, 0], [-900, 0, 0, 600, 200]] {
                            let want = thinker_intent_table(
                                dist_mm,
                                foe_attacking,
                                stamina,
                                &weights,
                                bias,
                            );
                            let got =
                                thinker_intent(dist_mm, foe_attacking, stamina, &weights, bias);
                            assert_eq!(
                                got, want,
                                "flat read moved at dist {dist_mm}, attack {foe_attacking}"
                            );
                            lanes[intent_tag(got)] = lanes[intent_tag(got)].saturating_add(1);
                        }
                    }
                }
            }
        }
        let reached = lanes.iter().filter(|n| **n > 0).count();
        assert!(reached >= 2, "the sweep must reach real choices: {lanes:?}");
    }

    /// Timing receipt: p50 microseconds of one [`thinker_intent`] against the
    /// per-call table build it replaced, interleaved so the same machine load
    /// sits under both sides. Integer nanos only: no cast, no unwrap.
    /// Run: cargo test -p engine-ai --release --lib thinker_intent_paired_receipt -- --ignored --nocapture
    #[test]
    #[ignore = "timing: run with --ignored (release, one thread) for the receipt"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "timing test reads the wall clock; it never enters the tick"
    )]
    fn thinker_intent_paired_receipt() {
        const ITERS: usize = 400;
        let weights = Weights::bold();
        let bias = [0, -200, 0, 300, 0];
        let (dist_mm, foe_attacking, stamina) = (2_500, true, 800);
        assert_eq!(
            thinker_intent(dist_mm, foe_attacking, stamina, &weights, bias),
            thinker_intent_table(dist_mm, foe_attacking, stamina, &weights, bias),
            "the two timed paths disagree"
        );
        std::hint::black_box(thinker_intent_table(
            dist_mm,
            foe_attacking,
            stamina,
            &weights,
            bias,
        ));
        std::hint::black_box(thinker_intent(
            dist_mm,
            foe_attacking,
            stamina,
            &weights,
            bias,
        ));
        let mut table = Vec::with_capacity(ITERS);
        let mut flat = Vec::with_capacity(ITERS);
        for _ in 0..ITERS {
            let begin = std::time::Instant::now();
            let _ = std::hint::black_box(thinker_intent_table(
                dist_mm,
                foe_attacking,
                stamina,
                &weights,
                bias,
            ));
            table.push(begin.elapsed().as_nanos());
            let begin = std::time::Instant::now();
            let _ = std::hint::black_box(thinker_intent(
                dist_mm,
                foe_attacking,
                stamina,
                &weights,
                bias,
            ));
            flat.push(begin.elapsed().as_nanos());
        }
        table.sort_unstable();
        flat.sort_unstable();
        let (table_ns, flat_ns) = (table[ITERS / 2], flat[ITERS / 2]);
        eprintln!(
            "gladiator_thinker_intent table_p50_us={}.{:03} flat_p50_us={}.{:03} speedup_x100={} iters={ITERS}",
            table_ns / 1_000,
            table_ns % 1_000,
            flat_ns / 1_000,
            flat_ns % 1_000,
            table_ns.saturating_mul(100).checked_div(flat_ns).unwrap_or(0),
        );
        assert!(
            flat_ns < table_ns,
            "the flat read must not be the slower one: {flat_ns} ns vs {table_ns} ns"
        );
    }

    #[test]
    fn wander_fires_when_foe_far_for_3s() {
        let mut brain = Brain::new(fighter(), 1);
        // Early far ticks close distance (advance), not wander.
        let first = brain.decide(&percept(0, 20.0, false));
        assert_eq!(brain.intent(), Intent::Advance, "{first:?}");
        assert_ne!((first.mx, first.mz), (0, 0));
        // After 180 consecutive far ticks the anti-stuck intent fires.
        let mut last = first;
        for tick in 1..=Brain::FAR_STREAK_TICKS {
            last = brain.decide(&percept(tick, 20.0, false));
        }
        assert_eq!(brain.intent(), Intent::Reposition, "{last:?}");
        assert_ne!((last.mx, last.mz), (0, 0), "wander must move: {last:?}");
        last.validate().expect("wander output validates");
        // Coming close clears the streak back to scored intents.
        let closed = brain.decide(&percept(Brain::FAR_STREAK_TICKS + 1, 1.5, false));
        assert_ne!(brain.intent(), Intent::Reposition, "{closed:?}");
    }

    #[test]
    fn perception_clock_holds_intent_between_ticks() {
        let mut brain = Brain::new(fighter(), 6);
        // Tick 0: close and calm, scores an attack.
        let eval = brain.decide(&percept(0, 1.5, false));
        assert_eq!(brain.intent(), Intent::Attack, "{eval:?}");
        // Ticks 1..5 present the opposite situation, yet the held
        // intent (all fields but `tick`) must not change.
        for tick in 1..6 {
            let held = brain.decide(&percept(tick, 20.0, true));
            assert_eq!(held.tick, tick);
            assert_eq!(held.fighter, eval.fighter);
            assert_eq!(held.mx, eval.mx);
            assert_eq!(held.mz, eval.mz);
            assert_eq!(held.facing, eval.facing);
            assert_eq!(held.buttons, eval.buttons);
        }
        // Tick 6 is the next perception tick and re-scores far+attacking.
        let next = brain.decide(&percept(6, 20.0, true));
        assert_eq!(next.tick, 6);
        assert_ne!(
            (next.mx, next.mz, next.buttons),
            (eval.mx, eval.mz, eval.buttons),
            "perception tick 6 must re-score: {next:?} vs {eval:?}"
        );
    }

    #[test]
    fn facing_points_by_quadrant_and_nan_never_panics() {
        assert_eq!(face_toward_cdeg(1_000, 0), Some(9_000)); // east
        assert_eq!(face_toward_cdeg(0, 1_000), Some(0)); // north
        assert_eq!(face_toward_cdeg(0, -1_000), Some(18_000)); // south
        assert_eq!(face_toward_cdeg(-1_000, 0), Some(-9_000)); // west
        assert_eq!(face_toward_cdeg(1_000, 1_000), Some(4_500));
        assert_eq!(face_toward_cdeg(0, 0), None);
        // NaN/inf/negative percepts sanitize instead of panicking.
        let mut brain = Brain::new(fighter(), 1);
        for (tick, dist) in [f32::NAN, f32::INFINITY, -5.0, 1e30]
            .into_iter()
            .enumerate()
        {
            let nasty = Percept::new(
                [f32::NAN, 0.0, f32::INFINITY],
                f32::NAN,
                [f32::NEG_INFINITY, 0.0, f32::NAN],
                tick % 2 == 0,
                dist,
                tick as u32,
            );
            let out = brain.decide(&nasty);
            out.validate().expect("sanitized output validates");
        }
    }

    // --- net policy: hand-built `.e2nn` fixtures, never shipped weights ---

    fn w_u16_net(value: u16, out: &mut Vec<u8>) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn w_i32_net(value: i32, out: &mut Vec<u8>) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    /// Hand-built single-dense `.e2nn` net: `in_dim` inputs to `out_dim`
    /// outputs, identity requant, no activation. Built from the documented
    /// net byte layout; nothing here ships weights with the crate.
    fn dense_bytes(in_dim: u16, out_dim: u16, bias: &[i32], weights: &[i8]) -> Vec<u8> {
        assert_eq!(bias.len(), usize::from(out_dim));
        assert_eq!(weights.len(), usize::from(in_dim) * usize::from(out_dim));
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&engine_nn::MAGIC);
        bytes.push(engine_nn::VERSION);
        bytes.push(1);
        w_u16_net(in_dim, &mut bytes);
        bytes.push(engine_nn::TAG_DENSE);
        w_u16_net(out_dim, &mut bytes);
        w_i32_net(1, &mut bytes);
        bytes.push(0);
        bytes.push(engine_nn::ACT_NONE);
        for v in bias {
            w_i32_net(*v, &mut bytes);
        }
        bytes.extend_from_slice(&weights.iter().map(|w| *w as u8).collect::<Vec<_>>());
        bytes
    }

    /// 6-feature, 5-intent zero-weight net with per-intent biases.
    fn bias_net_bytes(bias: [i32; 5]) -> Vec<u8> {
        dense_bytes(
            FEATURE_DIM as u16,
            INTENT_OUT_DIM as u16,
            &bias,
            &[0i8; FEATURE_DIM * INTENT_OUT_DIM],
        )
    }

    /// Net whose Block lane (index 1) reads the foe-attack lane with weight
    /// 1 and is zero elsewhere: calm percepts tie at 0 (Attack keeps the
    /// tie), attacking percepts score Block 127.
    fn foe_gate_bytes() -> Vec<u8> {
        let mut weights = [0i8; FEATURE_DIM * INTENT_OUT_DIM];
        weights[FEATURE_DIM + LANE_FOE_ATTACK] = 1;
        dense_bytes(
            FEATURE_DIM as u16,
            INTENT_OUT_DIM as u16,
            &[0, 0, 0, 0, 0],
            &weights,
        )
    }

    #[test]
    fn percept_features_quantize_with_goldens() {
        // Foe 16 m east at our height, attacking, yaw 0: full-scale lanes.
        let east = Percept::new([0.0, 0.0, 0.0], 0.0, [16.0, 0.0, 0.0], true, 16.0, 0);
        assert_eq!(features_from(&east), [127, 0, 0, 127, FEATURE_ATTACK_ON, 0]);
        // Off-axis foe at rest, calm, facing half pi: sign and scale carry.
        let off = Percept::new(
            [0.0, 0.0, 0.0],
            core::f32::consts::FRAC_PI_2,
            [-8.0, 4.0, -4.0],
            false,
            2.0,
            1,
        );
        assert_eq!(features_from(&off), [-63, -31, 63, 15, 0, 63]);
        // Non-finite geometry degrades to 0 while the attack flag survives.
        let nasty = Percept::new(
            [f32::NAN, 0.0, f32::INFINITY],
            f32::NAN,
            [f32::NEG_INFINITY, 0.0, f32::NAN],
            true,
            f32::NAN,
            2,
        );
        assert_eq!(features_from(&nasty), [0, 0, 0, 0, FEATURE_ATTACK_ON, 0]);
        assert_eq!(features_from(&nasty).len(), FEATURE_DIM);
    }

    #[test]
    fn hand_built_net_golden_decisions() {
        // All-zero net: every logit ties at 0, the scripted order keeps Attack.
        let mut tied =
            NetPolicy::load(fighter(), &bias_net_bytes([0, 0, 0, 0, 0])).expect("zero net loads");
        let probe = percept(0, 1.5, false);
        assert_eq!(tied.intent_for(&probe), Intent::Attack);
        let out = tied.decide(&probe);
        out.validate().expect("net output validates");
        assert_eq!((out.mx, out.mz, out.buttons), (30, 0, Buttons(ATTACK)));
        assert_eq!(tied.intent(), Intent::Attack);
        // Bias-only Block net always guards, even when calm and close.
        let mut guard = NetPolicy::load(fighter(), &bias_net_bytes([0, 100, 0, 0, 0]))
            .expect("guard net loads");
        assert_eq!(guard.intent_for(&probe), Intent::Block);
        let held = guard.decide(&probe);
        assert_eq!((held.mx, held.mz, held.buttons), (0, 0, Buttons(BLOCK)));
        // Foe-gated net: calm strikes, attacking guards.
        let mut gate = NetPolicy::load(fighter(), &foe_gate_bytes()).expect("gate net loads");
        assert_eq!(gate.intent_for(&percept(1, 1.5, false)), Intent::Attack);
        assert_eq!(gate.intent_for(&percept(2, 1.5, true)), Intent::Block);
        // Facing follows the scripted convention: foe due +X reads +9000.
        let struck = gate.decide(&percept(3, 1.5, false));
        struck.validate().expect("gated output validates");
        assert_eq!(struck.facing, 9_000);
        assert_eq!(struck.buttons, Buttons(ATTACK));
    }

    #[test]
    fn net_driven_fight_replays_bit_exactly() {
        let bytes = foe_gate_bytes();
        let fight = || {
            let mut policy = Policy::load_or_scripted(fighter(), Some(&bytes));
            assert!(policy.is_net());
            (0..3_600)
                .map(|tick| policy.decide(&script_percept(tick)))
                .collect::<Vec<_>>()
        };
        let (first, second) = (fight(), fight());
        assert_eq!(first.len(), 3_600);
        assert_eq!(first, second);
        for input in &first {
            input.validate().expect("every net output validates");
        }
        // Reloaded bytes replay identically too.
        let mut reloaded = Policy::load_or_scripted(fighter(), Some(&bytes));
        let third = (0..3_600)
            .map(|tick| reloaded.decide(&script_percept(tick)))
            .collect::<Vec<_>>();
        assert_eq!(third, first);
        // The script exercises both sides of the gate.
        assert!(first.iter().any(|i| i.buttons == Buttons(ATTACK)));
        assert!(first.iter().any(|i| i.buttons == Buttons(BLOCK)));
    }

    #[test]
    fn fallback_engages_on_malformed_net() {
        // Absent net falls back to scripted and still validates.
        let mut absent = Policy::load_or_scripted(fighter(), None);
        assert!(!absent.is_net());
        absent
            .decide(&percept(0, 1.5, false))
            .validate()
            .expect("fallback output validates");
        // Corrupt bytes fall back too, never panic.
        let mut bad_magic = foe_gate_bytes();
        bad_magic[0] = b'Z';
        for bad in [Vec::new(), vec![0u8; 8], bad_magic] {
            let mut fallback = Policy::load_or_scripted(fighter(), Some(&bad));
            assert!(!fallback.is_net());
            fallback
                .decide(&percept(1, 1.5, true))
                .validate()
                .expect("fallback output validates");
        }
        // Typed errors name the cause: truncated bytes carry the net-runtime error.
        assert_eq!(
            NetPolicy::load(fighter(), &[]),
            Err(NetPolicyError::Nn(engine_nn::NnErr::Truncated))
        );
        assert_eq!(
            NetPolicy::load(fighter(), &[0u8; 8]),
            Err(NetPolicyError::Nn(engine_nn::NnErr::BadMagic))
        );
        // Valid nets with the wrong dims are rejected by lane.
        let narrow = dense_bytes(3, 5, &[0; 5], &[0; 15]);
        engine_nn::load(&narrow).expect("narrow net is valid NN1");
        assert_eq!(
            NetPolicy::load(fighter(), &narrow),
            Err(NetPolicyError::BadInputDim {
                expected: FEATURE_DIM,
                got: 3
            })
        );
        let short = dense_bytes(6, 3, &[0; 3], &[0; 18]);
        engine_nn::load(&short).expect("short net is valid NN1");
        assert_eq!(
            NetPolicy::load(fighter(), &short),
            Err(NetPolicyError::BadOutputDim {
                expected: INTENT_OUT_DIM,
                got: 3
            })
        );
    }

    #[test]
    fn comparison_names_first_divergence() {
        // Identical policies agree everywhere; empty scripts agree vacuously.
        let bytes = foe_gate_bytes();
        let script = [percept(0, 1.5, false), percept(1, 2.0, true)];
        let mut a = Policy::load_or_scripted(fighter(), Some(&bytes));
        let mut b = Policy::load_or_scripted(fighter(), Some(&bytes));
        assert_eq!(
            compare(&mut a, &mut b, &script),
            Comparison {
                total: 2,
                agree: 2,
                agree_rate_permille: 1000,
                first_divergent_index: None,
            }
        );
        assert_eq!(
            compare(&mut a, &mut b, &[]),
            Comparison {
                total: 0,
                agree: 0,
                agree_rate_permille: 1000,
                first_divergent_index: None,
            }
        );
        // Crafted disagreeing script: always-attack vs foe-gate agree on two
        // calm percepts, then diverge on the first attacking percept.
        let attack_bytes = bias_net_bytes([50, 0, 0, 0, 0]);
        let mut attack = Policy::load_or_scripted(fighter(), Some(&attack_bytes));
        let mut gate = Policy::load_or_scripted(fighter(), Some(&bytes));
        let script = [
            percept(0, 1.5, false),
            percept(1, 1.5, false),
            percept(2, 1.5, true),
            percept(3, 1.5, true),
            percept(4, 1.5, false),
        ];
        let report = compare(&mut attack, &mut gate, &script);
        assert_eq!(report.first_divergent_index, Some(2));
        assert_eq!(report.total, 5);
        assert_eq!(report.agree, 3);
        assert_eq!(report.agree_rate_permille, 600);
    }

    // --- replacement curves: validated tables, scripted fallback ---

    use crate::curves::{validate_config, validate_curves};
    use crate::curves::{AiConfig, AiCurveError, AiCurves, CURVE_LANES, DEFAULT_ENGAGE_RANGE_CM};

    /// Hot replacement: maximum aggression plus a long engage range, still
    /// monotone and in range, so it validates and strikes where the
    /// scripted defaults still advance.
    fn hot_curves() -> AiCurves {
        AiCurves {
            version: 2,
            aggression: [1_000; CURVE_LANES],
            caution: [500; CURVE_LANES],
            engage_range_cm: 600,
        }
    }

    fn bout_script(ticks: u32) -> Vec<Percept> {
        (0..ticks).map(script_percept).collect()
    }

    fn run_brain(brain: &mut Brain, script: &[Percept]) -> Vec<FighterInput> {
        script.iter().map(|p| brain.decide(p)).collect()
    }

    #[test]
    fn replacement_curves_validate_and_change_decisions() {
        let hot = hot_curves();
        validate_curves(&hot).expect("hot replacement validates");
        validate_config(&AiConfig::try_new(hot, Brain::MORALE_NEUTRAL).expect("hot config"))
            .expect("hot config validates");
        let script = bout_script(600);
        let first = run_brain(
            &mut Brain::with_config(fighter(), 1, AiConfig::new(hot, Brain::MORALE_NEUTRAL)),
            &script,
        );
        let second = run_brain(
            &mut Brain::with_config(fighter(), 1, AiConfig::new(hot, Brain::MORALE_NEUTRAL)),
            &script,
        );
        assert_eq!(first.len(), 600);
        assert_eq!(first, second, "same config replays identically");
        for input in &first {
            input.validate().expect("replacement output validates");
        }
        let baseline = run_brain(&mut Brain::new(fighter(), 1), &script);
        assert_ne!(
            first, baseline,
            "replacement must change decisions vs default"
        );
        assert!(
            first.iter().any(|i| i.buttons == Buttons(ATTACK)),
            "hot curves strike on the bout"
        );
        // The longer engage range commits at 5 m where the defaults advance.
        let probe = percept(7, 5.0, false);
        let mut hot_brain =
            Brain::with_config(fighter(), 1, AiConfig::new(hot, Brain::MORALE_NEUTRAL));
        let mut plain = Brain::new(fighter(), 1);
        assert_eq!(hot_brain.decide(&probe).buttons, Buttons(ATTACK));
        assert_ne!(hot_brain.decide(&probe), plain.decide(&probe));
        assert_eq!(hot_brain.config().curves, hot);
    }

    #[test]
    fn invalid_curves_rejected_per_rule() {
        let good = AiCurves::default_curves();
        let mut versionless = good;
        versionless.version = 0;
        assert_eq!(
            validate_curves(&versionless),
            Err(AiCurveError::ZeroVersion)
        );
        let mut wild = good;
        wild.aggression[3] = 1_001;
        assert_eq!(
            validate_curves(&wild),
            Err(AiCurveError::AggressionOutOfRange {
                index: 3,
                value: 1_001,
            })
        );
        let mut rash = good;
        rash.caution[6] = 9_999;
        assert_eq!(
            validate_curves(&rash),
            Err(AiCurveError::CautionOutOfRange {
                index: 6,
                value: 9_999,
            })
        );
        let mut dipped = good;
        dipped.aggression = [100, 200, 300, 400, 500, 499, 700, 800];
        assert_eq!(
            validate_curves(&dipped),
            Err(AiCurveError::AggressionNotMonotonic {
                index: 5,
                prev: 500,
                next: 499,
            })
        );
        let mut point_blank = good;
        point_blank.engage_range_cm = 0;
        assert_eq!(
            validate_curves(&point_blank),
            Err(AiCurveError::EngageRangeOutOfRange { value: 0 })
        );
        // Sane bounds still accept the table edges.
        let mut edges = good;
        edges.version = 2;
        edges.aggression = [0; CURVE_LANES];
        edges.caution = [1_000; CURVE_LANES];
        edges.engage_range_cm = DEFAULT_ENGAGE_RANGE_CM;
        validate_curves(&edges).expect("edge tables validate");
    }

    #[test]
    fn fallback_to_scripted_defaults_on_invalid_config() {
        let script = bout_script(300);
        let baseline = run_brain(&mut Brain::new(fighter(), 1), &script);
        // Every invalid shape falls back to the scripted defaults.
        let mut versionless = AiCurves::default_curves();
        versionless.version = 0;
        let mut dipped = AiCurves::default_curves();
        dipped.aggression = [900, 800, 700, 600, 500, 400, 300, 200];
        let mut point_blank = AiCurves::default_curves();
        point_blank.engage_range_cm = 0;
        for bad in [versionless, dipped, point_blank] {
            assert!(validate_curves(&bad).is_err());
            let config = AiConfig {
                curves: bad,
                morale_default: Brain::MORALE_NEUTRAL,
            };
            assert!(!config.is_valid());
            assert_eq!(config.effective(), AiConfig::scripted_defaults());
            let rerun = run_brain(&mut Brain::with_config(fighter(), 1, config), &script);
            assert_eq!(
                rerun, baseline,
                "invalid config falls back to scripted defaults"
            );
            for input in &rerun {
                input.validate().expect("fallback output validates");
            }
        }
        // A bad morale default falls back too, including the start morale.
        let stray = AiConfig {
            curves: AiCurves::default_curves(),
            morale_default: -50,
        };
        let brain = Brain::with_config(fighter(), 1, stray);
        assert_eq!(brain.morale(), Brain::MORALE_NEUTRAL);
        assert_eq!(*brain.config(), AiConfig::scripted_defaults());
    }

    #[test]
    fn comparison_harness_reports_agreement_rate() {
        let current = AiConfig::scripted_defaults();
        let replacement = AiConfig::new(hot_curves(), Brain::MORALE_NEUTRAL);
        let script = bout_script(240);
        let identical = compare_curves(&current, &current, &script);
        assert_eq!(identical.total, 240);
        assert_eq!(identical.agree, 240);
        assert_eq!(identical.agree_rate_permille, 1000);
        assert_eq!(identical.first_divergent_index, None);
        assert_eq!(
            compare_curves(&current, &current, &[]),
            Comparison {
                total: 0,
                agree: 0,
                agree_rate_permille: 1000,
                first_divergent_index: None,
            }
        );
        let report = compare_curves(&current, &replacement, &script);
        assert_eq!(report.total, script.len());
        assert!(
            report.agree < report.total,
            "hot curves must diverge: {report:?}"
        );
        assert_eq!(
            report.agree_rate_permille,
            (report.agree * 1000 / report.total) as u32
        );
        assert!(
            report.first_divergent_index.is_some(),
            "divergence names its tick: {report:?}"
        );
        // Same configs twice give the same report.
        assert_eq!(compare_curves(&current, &replacement, &script), report);
    }

    #[test]
    fn generic_pick_matches_scored_choices_and_breaks_ties_first() {
        // Ties keep the earliest lane, exactly like the scored core.
        assert_eq!(pick_intent([0, 0, 0, 0, 0]), Intent::Attack);
        assert_eq!(pick_intent([1, 1, 1, 1, 1]), Intent::Attack);
        assert_eq!(pick_intent([0, 9, 9, 0, 0]), Intent::Block);
        // One lane ahead wins outright.
        assert_eq!(pick_intent([0, 0, 750, 0, 0]), Intent::Dodge);
        assert_eq!(pick_intent([0, 0, 0, 600, 0]), Intent::Advance);
        assert_eq!(pick_intent([0, 0, 0, 0, 500]), Intent::Retreat);
        // The scored core agrees: far and calm advances through the seam.
        let mut brain = Brain::new(fighter(), 1);
        let out = brain.decide(&percept(0, 20.0, false));
        assert_eq!(brain.intent(), Intent::Advance, "{out:?}");
    }

    /// Drive any [`TickPolicy`] over the fixed 60 s script. Harness only
    /// (allocates its report); the tick itself never allocates.
    fn drive<P: TickPolicy>(policy: &mut P) -> Vec<FighterInput> {
        (0..3_600)
            .map(|tick| policy.decide(&script_percept(tick)))
            .collect()
    }

    #[test]
    fn non_gladiator_march_drives_the_same_tick() {
        // A non-gladiator script (no utility, no morale, no curves) runs
        // the same [`TickPolicy`] seam: 60 s identical twice, always valid.
        let (first, second) = (
            drive(&mut MarchPolicy::new(fighter())),
            drive(&mut MarchPolicy::new(fighter())),
        );
        assert_eq!(first.len(), 3_600);
        assert_eq!(first, second);
        for input in &first {
            input.validate().expect("every march output validates");
        }
        // It marches: every wish advances, faced east down the script lane.
        assert!(first.iter().all(|input| input.buttons == Buttons::NONE));
        assert!(first.iter().all(|input| input.mx == 100 && input.mz == 0));
        assert_eq!(MarchPolicy::new(fighter()).intent(), Intent::Advance);
        let probe = MarchPolicy::new(fighter()).fighter();
        assert_eq!(probe, fighter());
        // A different brain on the same script decides differently: the
        // seam carries any policy, never just the gladiator.
        let brain_first = run_script(1, Brain::MORALE_NEUTRAL);
        assert_ne!(first, brain_first, "march must differ from utility");
        // Every shipped policy drives the same seam over the same script.
        for bytes in [None, Some(foe_gate_bytes())] {
            let make = |f: FighterId| match &bytes {
                Some(raw) => Policy::load_or_scripted(f, Some(raw)),
                None => Policy::Scripted(Brain::new(f, 1)),
            };
            let mut first_run = make(fighter());
            let rerun: Vec<FighterInput> = (0..3_600)
                .map(|tick| TickPolicy::decide(&mut first_run, &script_percept(tick)))
                .collect();
            assert_eq!(rerun.len(), 3_600);
            for input in &rerun {
                input.validate().expect("every policy output validates");
            }
            let mut second_run = make(fighter());
            let again: Vec<FighterInput> = (0..3_600)
                .map(|tick| TickPolicy::decide(&mut second_run, &script_percept(tick)))
                .collect();
            assert_eq!(rerun, again, "same policy replays identically");
            let _ = TickPolicy::intent(&second_run);
        }
    }

    #[test]
    fn march_faces_the_foe_and_survives_nan() {
        let mut march = MarchPolicy::new(fighter());
        let struck = march.decide(&percept(3, 1.5, false));
        struck.validate().expect("march output validates");
        assert_eq!(struck.facing, 9_000);
        assert_eq!(march.intent(), Intent::Advance);
        // Coincident fighters fall back to yaw, then to the last facing.
        let mut coinc = MarchPolicy::new(fighter());
        let first = coinc.decide(&Percept::new(
            [1.0, 0.0, 2.0],
            0.0,
            [1.0, 0.0, 2.0],
            false,
            0.0,
            0,
        ));
        assert_eq!(first.facing, 0);
        for (tick, dist) in [f32::NAN, f32::INFINITY, -5.0].into_iter().enumerate() {
            let nasty = Percept::new(
                [f32::NAN, 0.0, f32::INFINITY],
                f32::NAN,
                [f32::NEG_INFINITY, 0.0, f32::NAN],
                false,
                dist,
                tick as u32,
            );
            let out = march.decide(&nasty);
            out.validate().expect("sanitized march output validates");
        }
    }

    // --- mind drain: the NPC brain tick drains via apply ---

    use crate::mind::{
        ActionKind, ActionLog, ActionSource, MindAction, MindError, MindRequest, NpcId, Privacy,
    };

    fn mind_npc() -> NpcId {
        NpcId(7)
    }

    fn mind_req(id: u64, issued: u64) -> MindRequest {
        MindRequest {
            id,
            npc: mind_npc(),
            issued_tick: issued,
            deadline_tick: issued + 4,
            budget_ms: 500,
            base_revision: 3,
            privacy: Privacy::RemoteAllowed,
        }
    }

    fn mind_action(kind: ActionKind, text: &str, stamp: u64, request: u64) -> MindAction {
        MindAction {
            tick_apply: stamp,
            npc: mind_npc(),
            kind,
            text: text.to_owned(),
            source: ActionSource::Scripted,
            request,
        }
    }

    #[test]
    fn drained_say_and_gesture_present_without_steering() {
        let ask = mind_req(1, 100);
        let mut log = ActionLog::new();
        log.push(mind_action(
            ActionKind::Say,
            "hold the gate, brothers.",
            100,
            1,
        ))
        .expect("records");
        log.push(mind_action(ActionKind::Gesture, "raises a fist.", 101, 1))
            .expect("records");
        let probe = percept(0, 1.5, false);
        let mut brain = Brain::new(fighter(), 1);
        let (out, batch) = brain.decide_drained(&probe, &mut log, 104, &[ask]);
        assert_eq!(batch.presented.len(), 2, "talk presents two lines");
        assert!(batch.intended.is_empty(), "talk never intends");
        assert!(batch.refused.is_empty(), "in-window stamps never refuse");
        assert!(log.is_empty(), "drain removes applied actions");
        out.validate().expect("drained output validates");
        assert_eq!(batch.presented[0].text, "hold the gate, brothers.");
        assert_eq!(batch.presented[1].kind, ActionKind::Gesture);
        // No steering happened: a mind-free brain decides identically.
        let mut plain = Brain::new(fighter(), 1);
        assert_eq!(out, plain.decide(&probe), "presentation must not steer");
    }

    #[test]
    fn drained_approach_and_leave_steer_the_brain() {
        // Close and calm scores Attack mind-free; the drained movement
        // overrides to Advance, Retreat and Block respectively.
        let probe = percept(0, 1.5, false);
        let mut plain = Brain::new(fighter(), 1);
        assert_eq!(plain.decide(&probe).buttons, Buttons(ATTACK));
        for (kind, target, intent, mx, buttons) in [
            (
                ActionKind::Approach,
                "the north gate",
                Intent::Advance,
                100,
                Buttons::NONE,
            ),
            (
                ActionKind::Leave,
                "the arena floor",
                Intent::Retreat,
                -80,
                Buttons::NONE,
            ),
            (
                ActionKind::Idle,
                "hold the line",
                Intent::Block,
                0,
                Buttons(BLOCK),
            ),
        ] {
            let ask = mind_req(2, 200);
            let mut log = ActionLog::new();
            log.push(mind_action(kind, target, 202, 2))
                .expect("records");
            let mut brain = Brain::new(fighter(), 1);
            let mut batch = AppliedBatch::new();
            let out = brain.decide_drained_into(&probe, &mut log, 204, &[ask], &mut batch);
            out.validate().expect("steered output validates");
            assert!(batch.presented.is_empty(), "{kind:?} never presents");
            assert_eq!(batch.intended.len(), 1);
            assert_eq!(batch.intended[0].target, target);
            assert!(batch.refused.is_empty());
            assert_eq!((out.mx, out.buttons), (mx, buttons), "{kind:?} steers");
            assert_eq!(brain.intent(), intent, "{kind:?} records its intent");
            assert!(log.is_empty(), "drain removes applied actions");
        }
    }

    #[test]
    fn drained_deadline_refusals_steer_nothing() {
        let ask = mind_req(3, 300);
        // Past-deadline stamps refuse as timeout, kept as history.
        let mut log = ActionLog::new();
        log.push(mind_action(ActionKind::Say, "too late.", 305, 3))
            .expect("records");
        log.push(mind_action(ActionKind::Approach, "the gate.", 306, 3))
            .expect("records");
        let probe = percept(0, 1.5, false);
        let mut brain = Brain::new(fighter(), 1);
        let (out, batch) = brain.decide_drained(&probe, &mut log, 1_000, &[ask]);
        assert!(batch.presented.is_empty(), "late talk never presents");
        assert!(batch.intended.is_empty(), "late moves never intend");
        assert_eq!(batch.refused.len(), 2);
        assert!(
            batch
                .refused
                .iter()
                .all(|refused| refused.cause == MindError::Timeout),
            "deadline overruns refuse as timeout: {:?}",
            batch.refused
        );
        assert!(log.is_empty(), "drain removes refused drains");
        out.validate().expect("refusal output validates");
        // Refusals steer nothing: a mind-free brain decides identically.
        let mut plain = Brain::new(fighter(), 1);
        assert_eq!(out, plain.decide(&probe), "refusals must not steer");
        // An inverted window refuses without reading the action.
        let broken = MindRequest {
            deadline_tick: 99,
            ..mind_req(4, 100)
        };
        let mut orphan = ActionLog::new();
        orphan
            .push(mind_action(ActionKind::Gesture, "waves.", 100, 4))
            .expect("records");
        let mut fallback = Brain::new(fighter(), 1);
        let (_, denied) = fallback.decide_drained(&probe, &mut orphan, 1_000, &[broken]);
        assert_eq!(denied.refused.len(), 1);
        assert_eq!(denied.refused[0].cause, MindError::PastDeadline);
        assert!(denied.presented.is_empty() && denied.intended.is_empty());
    }

    #[test]
    fn self_play_win_rate_vs_heuristic_baseline() {
        // Heuristic baseline: MarchPolicy always advances and never strikes,
        // so the scripted Brain (the candidate) must win every bout while the
        // harness reports eval throughput (steps/sec) for the win-rate work.
        let config = SelfPlayConfig::new(8, 240, 2040).expect("8x240 self-play fits");
        let foes = FighterId::new(1).expect("slot 1 is legal");
        let mut pairs: Vec<(Brain, MarchPolicy)> = (0..config.bouts)
            .map(|_| (Brain::new(fighter(), 1), MarchPolicy::new(foes)))
            .collect();
        let report = self_play_batch(&mut pairs, &config);
        assert_eq!(report.bouts, 8);
        assert_eq!(report.ticks_per_bout, 240);
        assert_eq!(report.wins_a + report.wins_b + report.draws, report.bouts);
        assert_eq!(
            report.win_rate_a_permille,
            (report.wins_a * 1000 / report.bouts) as u32
        );
        assert_eq!(report.eval_steps, 8 * 240 * 2);
        assert!(
            report.wins_a > report.wins_b,
            "scripted brain beats the march baseline: {report:?}"
        );
        // Throughput is caller-timed integer math over the step count.
        assert_eq!(report.steps_per_sec(Duration::from_secs(2)), 1_920);
        assert_eq!(report.steps_per_sec(Duration::from_millis(1)), 3_840_000);
        assert_eq!(report.steps_per_sec(Duration::ZERO), report.eval_steps);
        // Deterministic: fresh pairs replay the identical report.
        let mut rerun: Vec<(Brain, MarchPolicy)> = (0..config.bouts)
            .map(|_| (Brain::new(fighter(), 1), MarchPolicy::new(foes)))
            .collect();
        let again = self_play_batch(&mut rerun, &config);
        assert_eq!(again, report);
        // Single bouts replay too, and the shape rejects empty batches.
        let mut solo_a = Brain::new(fighter(), 1);
        let mut solo_b = MarchPolicy::new(foes);
        let first = self_play_bout(&mut solo_a, &mut solo_b, 3, 240, 2040);
        let mut solo_c = Brain::new(fighter(), 1);
        let mut solo_d = MarchPolicy::new(foes);
        assert_eq!(
            first,
            self_play_bout(&mut solo_c, &mut solo_d, 3, 240, 2040)
        );
        assert_eq!(
            SelfPlayConfig::new(0, 240, 0),
            Err(SelfPlayError::EmptyBouts)
        );
        assert_eq!(SelfPlayConfig::new(8, 0, 0), Err(SelfPlayError::EmptyTicks));
        assert_eq!(
            SelfPlayConfig::new(257, 240, 0),
            Err(SelfPlayError::TooManyBouts { got: 257 })
        );
        assert_eq!(
            SelfPlayConfig::new(8, 3_601, 0),
            Err(SelfPlayError::TooManyTicks { got: 3_601 })
        );
    }

    #[test]
    fn rows_text_builds_scripted_brain() {
        // Designer rows without a Rust edit build the scripted brain;
        // bad rows refuse instead of falling back silently.
        const V1: &str = "table ai version 1\nv1 aggression0=0 aggression1=125 aggression2=250 aggression3=375 aggression4=500 aggression5=625 aggression6=750 aggression7=1000 caution0=500 caution1=500 caution2=500 caution3=500 caution4=500 caution5=500 caution6=500 caution7=500 engage_range_cm=250 morale_default=500";
        let brain = Brain::with_rows_text(fighter(), 1, V1).expect("v1 rows build a brain");
        assert_eq!(*brain.config(), AiConfig::scripted_defaults());
        assert!(
            Brain::with_rows_text(fighter(), 1, "table ai version 0\nv0 aggression0=0").is_err()
        );
        // The live constructor loads the designer file, which matches defaults.
        assert_eq!(
            *Brain::new(fighter(), 1).config(),
            AiConfig::scripted_defaults()
        );
    }
}
