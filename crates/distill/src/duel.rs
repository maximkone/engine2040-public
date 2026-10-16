//! Arena duel as distill's teacher and as the student's ring.
//!
//! Consumer: the `distill_duel` generator records teacher rows with
//! [`teacher_episode`] and the strength gate plays [`play_bout`]; both go
//! through the public arena controller seam ([`engine_arena::run_controlled`])
//! and never touch sim state. Owner: this module owns the percept-lane table
//! ([`DUEL_LANE_STEPS`]), the input-to-intent label rule
//! ([`intent_class`]), the intent-to-input actuator ([`class_input`]) and the
//! bout verdict ([`BoutOutcome`]). Units: positions are the controller view's
//! millimetres, lanes are `i8` net inputs, classes are the scripted utility
//! order (attack, block, dodge, advance, retreat), ticks are fixed 60 Hz
//! steps. Determinism: the arena run is deterministic per seed; lanes go
//! through the one quantization edge ([`engine_nn::quantize_lanes`]); student
//! inference is integer-only ([`engine_nn::infer_into`]) with no allocation
//! per tick; the actuator is integer-only. Errors: an arena refusal maps to
//! [`DuelError`]; nothing here panics.
//!
//! ## The actuator (why intents, not raw inputs)
//!
//! The arena moves a fighter by its wish in world axes (`mx` east, `mz`
//! north). The depth brain emits its wish as if it were forward/strafe in
//! the facing frame (`Advance` is `mx = 100`), so in the legacy run both
//! brains walk east into the wall and never meet (measured: 1800 ticks, no
//! hit, separation stays above 16 m). Distilling those raw inputs would teach
//! a wall-hugger. So the student learns the brain's *intent* (its buttons
//! and wish sign, [`intent_class`]) and every intent leaves through one
//! world-frame actuator ([`class_input`]): advance and attack step toward the
//! foe, retreat away, dodge sideways.
//!
//! ## Teacher (gogame 6.2: training data from the strong player)
//!
//! The arena's own depth brain decides every slot. [`TeacherDriver`] claims
//! both slots: on each tick it reads the brain-proposed input, records
//! `(lanes, intent)` on perception ticks (`tick % AI_PERIOD_TICKS == 0`, the
//! ticks the brain re-scores) and re-emits the intent through the actuator,
//! so the duel actually closes and the rows cover the fight, not the wall.
//!
//! ## Student (gogame 14.2: the small net plays, search stays offline)
//!
//! [`StudentController`] claims one slot. On the same perception clock it
//! quantizes the view lanes, runs the `.e2nn` net into caller-owned scratch,
//! takes the first-max intent and holds it until the next perception tick;
//! the actuator turns it into an input every tick. Facing comes from the
//! brain-proposed row entry (face-the-foe), so the net decides what to do,
//! never where to look.

use core::f32::consts::PI;

use engine_ai::{
    FEATURE_DIM, FEATURE_FULL_DIST_M, FEATURE_FULL_GROUND_M, FEATURE_FULL_HEIGHT_M,
    INTENT_OUT_DIM,
};
use engine_arena::controller::{SlotInput, SlotTape};
use engine_arena::{
    run_controlled, ArenaConfig, ControlView, SlotController, SlotView, AI_PERIOD_TICKS,
};
use engine_input::fighter::{ATTACK, BLOCK, DODGE};
use engine_input::Buttons;
use engine_nn::{infer_into, quantize_lanes, Model, Scratch};

/// Lanes per duel percept: the net input dim (`engine_ai::FEATURE_DIM`).
pub const DUEL_LANES: usize = FEATURE_DIM;
/// Intent classes: the net output dim (`engine_ai::INTENT_OUT_DIM`).
pub const DUEL_CLASSES: usize = INTENT_OUT_DIM;
/// Duel arena half-extent in metres (the depth brain's default wall band).
pub const ARENA_HALF_EXTENT_M: f32 = 20.0;

/// Fixed scale table, `i8` steps per lane unit, in `engine_ai::LANE_*` order:
/// dx, dz (m over the 16 m ground scale), dy (m over 8 m), separation (m over
/// 16 m), foe attacking (1.0 when attacking), own yaw (radians over pi). The
/// one table every caller of the duel net quantizes with.
pub const DUEL_LANE_STEPS: [f32; DUEL_LANES] = [
    127.0 / FEATURE_FULL_GROUND_M,
    127.0 / FEATURE_FULL_GROUND_M,
    127.0 / FEATURE_FULL_HEIGHT_M,
    127.0 / FEATURE_FULL_DIST_M,
    127.0,
    127.0 / PI,
];

/// Intent class ids in the scripted utility order.
pub const CLASS_ATTACK: u8 = 0;
/// Block class.
pub const CLASS_BLOCK: u8 = 1;
/// Dodge class.
pub const CLASS_DODGE: u8 = 2;
/// Advance class (also the label for the brain's far-foe reposition).
pub const CLASS_ADVANCE: u8 = 3;
/// Retreat class.
pub const CLASS_RETREAT: u8 = 4;

/// Arena refusals while running a bout or a teacher episode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DuelError {
    /// The duel config failed arena validation (ticks out of range).
    BadConfig(engine_arena::ArenaError),
    /// The arena rejected the run (empty checkpoint list).
    RunRejected,
    /// The student net is not `DUEL_LANES` in, `DUEL_CLASSES` out.
    BadNetShape {
        /// Net input dim.
        input: usize,
        /// Net output dim.
        output: usize,
    },
}

impl core::fmt::Display for DuelError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DuelError::BadConfig(e) => write!(f, "duel config refused: {e:?}"),
            DuelError::RunRejected => f.write_str("arena rejected the duel run"),
            DuelError::BadNetShape { input, output } => write!(
                f,
                "net is {input} -> {output}, want {DUEL_LANES} -> {DUEL_CLASSES}"
            ),
        }
    }
}

impl std::error::Error for DuelError {}

/// Nearest other slot by squared millimetre distance; ties keep the lower
/// index after `slot + 1`, mirroring the arena percept's foe pick.
fn foe_of(slots: &[SlotView], slot: usize) -> usize {
    let n = slots.len();
    let me = slots[slot];
    let mut foe = (slot + 1) % n;
    let mut best = i64::MAX;
    for (index, other) in slots.iter().enumerate() {
        if index == slot {
            continue;
        }
        let dx = i64::from(other.x_mm) - i64::from(me.x_mm);
        let dz = i64::from(other.z_mm) - i64::from(me.z_mm);
        let d2 = dx * dx + dz * dz;
        if d2 < best {
            best = d2;
            foe = index;
        }
    }
    foe
}

/// Foe-minus-self ground delta in millimetres for `slot` (zero when the
/// view has no foe).
pub fn foe_delta_mm(slots: &[SlotView], slot: usize) -> (i64, i64) {
    if slot >= slots.len() || slots.len() < 2 {
        return (0, 0);
    }
    let me = slots[slot];
    let foe = slots[foe_of(slots, slot)];
    (
        i64::from(foe.x_mm) - i64::from(me.x_mm),
        i64::from(foe.z_mm) - i64::from(me.z_mm),
    )
}

/// Millimetres as metres at centimetre resolution (the lanes step every
/// ~126 mm, so the dropped millimetres move a lane by at most one rounding
/// tie); saturates past +/-327 m.
fn mm_to_m(mm: i64) -> f32 {
    let cm = (mm / 10).clamp(-32_767, 32_767);
    f32::from(i16::try_from(cm).unwrap_or(0)) / 100.0
}

/// Float percept lanes for `slot` in `engine_ai::LANE_*` order, from the
/// controller view (the arena's flat floor gives `dy = 0`). `None` when the
/// slot is out of range or the view has no foe.
pub fn duel_lanes(slots: &[SlotView], slot: usize) -> Option<[f32; DUEL_LANES]> {
    if slot >= slots.len() || slots.len() < 2 {
        return None;
    }
    let (dx_mm, dz_mm) = foe_delta_mm(slots, slot);
    let (dx, dz) = (mm_to_m(dx_mm), mm_to_m(dz_mm));
    let dist = (dx * dx + dz * dz).sqrt();
    let attacking = if slots[foe_of(slots, slot)].attacking {
        1.0
    } else {
        0.0
    };
    let yaw = f32::from(slots[slot].facing_cdeg) * (PI / 18_000.0);
    Some([dx, dz, 0.0, dist, attacking, yaw])
}

/// Quantized net input for `slot`: [`duel_lanes`] through the one
/// quantization edge with [`DUEL_LANE_STEPS`]. All-zero when the slot has
/// no percept. No allocation.
pub fn duel_features(slots: &[SlotView], slot: usize) -> [i8; DUEL_LANES] {
    let mut out = [0i8; DUEL_LANES];
    if let Some(lanes) = duel_lanes(slots, slot) {
        quantize_lanes(&lanes, &DUEL_LANE_STEPS, &mut out);
    }
    out
}

/// Teacher label for one brain input: buttons first (attack, block, dodge),
/// then the wish sign (backward is retreat, anything else advance; the
/// brain's far-foe reposition strafes forward, so it labels as advance).
pub fn intent_class(input: &SlotInput) -> u8 {
    let buttons = input.buttons.0;
    if buttons & ATTACK != 0 {
        CLASS_ATTACK
    } else if buttons & BLOCK != 0 {
        CLASS_BLOCK
    } else if buttons & DODGE != 0 {
        CLASS_DODGE
    } else if input.mx < 0 {
        CLASS_RETREAT
    } else {
        CLASS_ADVANCE
    }
}

/// World-frame wish of length `percent` along `(dx, dz)`, integer only;
/// zero when the delta is zero.
fn wish_toward(dx: i64, dz: i64, percent: i64) -> (i8, i8) {
    let len_sq = u64::try_from(dx * dx + dz * dz).unwrap_or(u64::MAX);
    let len = i64::try_from(len_sq.isqrt()).unwrap_or(i64::MAX);
    if len == 0 {
        return (0, 0);
    }
    let axis = |d: i64| i8::try_from((d * percent) / len).unwrap_or(0);
    (axis(dx), axis(dz))
}

/// The one actuator: an intent class plus the foe delta to a world-frame
/// input. Advance walks at full wish toward the foe, attack steps in at 30
/// with the attack button, retreat backs off at 80, block stands with the
/// block button, dodge sidesteps at 100 (side alternates every perception
/// window) with the dodge button. Keeps `proposed`'s tick, fighter and facing.
/// Unknown classes advance.
pub fn class_input(class: u8, tick: u32, proposed: SlotInput, to_foe: (i64, i64)) -> SlotInput {
    let (dx, dz) = to_foe;
    let ((mx, mz), buttons) = match class {
        CLASS_ATTACK => (wish_toward(dx, dz, 30), Buttons(ATTACK)),
        CLASS_BLOCK => ((0, 0), Buttons(BLOCK)),
        CLASS_DODGE => {
            let window = tick / AI_PERIOD_TICKS;
            let (px, pz) = if window.is_multiple_of(2) {
                (-dz, dx)
            } else {
                (dz, -dx)
            };
            (wish_toward(px, pz, 100), Buttons(DODGE))
        }
        CLASS_RETREAT => (wish_toward(-dx, -dz, 80), Buttons::NONE),
        _ => (wish_toward(dx, dz, 100), Buttons::NONE),
    };
    SlotInput {
        mx,
        mz,
        buttons,
        ..proposed
    }
}

/// First-max argmax over logits (strict `>`, the scripted tie order).
fn first_max(logits: &[i32]) -> u8 {
    let mut best = 0usize;
    for (i, v) in logits.iter().enumerate() {
        if *v > logits[best] {
            best = i;
        }
    }
    u8::try_from(best).unwrap_or(CLASS_ADVANCE)
}

/// First tick of the perception window holding `tick`.
fn perception_tick(tick: u32) -> u32 {
    tick - tick % AI_PERIOD_TICKS
}

/// Re-emit the brain's proposed intent for `slot` through the actuator.
fn actuate_brain(view: &ControlView<'_>, row: &mut [SlotInput], slot: usize) -> u8 {
    let class = intent_class(&row[slot]);
    row[slot] = class_input(
        class,
        perception_tick(view.tick),
        row[slot],
        foe_delta_mm(view.slots, slot),
    );
    class
}

/// One recorded teacher decision: quantized lanes plus the intent class.
pub type TeacherRow = ([i8; DUEL_LANES], u8);

/// Teacher tap: claims every slot, records `(lanes, brain intent)` on each
/// perception tick and re-emits the brain's intents through the actuator.
#[derive(Clone, Debug, Default)]
pub struct TeacherDriver {
    rows: Vec<TeacherRow>,
}

impl TeacherDriver {
    /// Empty driver.
    pub fn new() -> Self {
        Self::default()
    }

    /// Recorded rows in (tick, slot) order.
    pub fn rows(&self) -> &[TeacherRow] {
        &self.rows
    }
}

impl SlotController for TeacherDriver {
    fn controls(&self, _slot: u8) -> bool {
        true
    }

    fn control(&mut self, view: &ControlView<'_>, row: &mut [SlotInput]) {
        let record = view.tick.is_multiple_of(AI_PERIOD_TICKS);
        for slot in 0..row.len().min(view.slots.len()) {
            let features = duel_features(view.slots, slot);
            let class = actuate_brain(view, row, slot);
            if record {
                self.rows.push((features, class));
            }
        }
    }
}

/// Student net in one slot: on each perception tick quantize, infer into
/// caller-owned scratch, argmax; hold the class in between; the actuator
/// emits it every tick.
#[derive(Debug)]
pub struct StudentController<'m> {
    model: &'m Model,
    slot: u8,
    scratch: Scratch,
    logits: [i32; DUEL_CLASSES],
    held: Option<u8>,
}

impl<'m> StudentController<'m> {
    /// Student for `slot`; refuses a net that is not 6 -> 5.
    pub fn new(model: &'m Model, slot: u8) -> Result<Self, DuelError> {
        if model.input_dim() != DUEL_LANES || model.output_dim() != DUEL_CLASSES {
            return Err(DuelError::BadNetShape {
                input: model.input_dim(),
                output: model.output_dim(),
            });
        }
        Ok(Self {
            model,
            slot,
            scratch: Scratch::for_model(model),
            logits: [0; DUEL_CLASSES],
            held: None,
        })
    }

    /// Decide the class for this view: fresh on perception ticks, else held.
    fn class_for(&mut self, view: &ControlView<'_>) -> u8 {
        if let (Some(class), false) = (self.held, view.tick.is_multiple_of(AI_PERIOD_TICKS)) {
            return class;
        }
        let features = duel_features(view.slots, usize::from(self.slot));
        let class = match infer_into(self.model, &features, &mut self.scratch, &mut self.logits) {
            Ok(()) => first_max(&self.logits),
            // Shapes are checked in `new`, so this arm is unreachable; advance.
            Err(_) => CLASS_ADVANCE,
        };
        self.held = Some(class);
        class
    }
}

impl SlotController for StudentController<'_> {
    fn controls(&self, slot: u8) -> bool {
        slot == self.slot
    }

    fn control(&mut self, view: &ControlView<'_>, row: &mut [SlotInput]) {
        let slot = usize::from(self.slot);
        if slot >= row.len() {
            return;
        }
        let class = self.class_for(view);
        row[slot] = class_input(
            class,
            perception_tick(view.tick),
            row[slot],
            foe_delta_mm(view.slots, slot),
        );
    }
}

/// Who the student fights in a bout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opponent {
    /// The depth brain's intents through the same actuator as the student:
    /// the teacher itself, the like-for-like distillation check.
    BrainIntents,
    /// The arena's depth brain exactly as the legacy match runs it (raw
    /// inputs, facing-frame wish read as world axes).
    ArenaBrain,
}

/// One bout from one side's point of view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoutOutcome {
    /// The side knocked the foe out first, or led on health at time.
    Win,
    /// The foe knocked the side out first, or led on health at time.
    Loss,
    /// Same-tick double knockout, or equal health at time.
    Draw,
}

/// Tracks health through the run: first knockout, else health at time.
#[derive(Clone, Copy, Debug, Default)]
struct Judge {
    hp: [u16; 2],
    /// First tick a knockout was seen: `(slot 0 down, slot 1 down)`.
    knockout: Option<(bool, bool)>,
}

impl Judge {
    fn watch(&mut self, slots: &[SlotView]) {
        if slots.len() < 2 {
            return;
        }
        self.hp = [slots[0].hp_permille, slots[1].hp_permille];
        if self.knockout.is_none() && (self.hp[0] == 0 || self.hp[1] == 0) {
            self.knockout = Some((self.hp[0] == 0, self.hp[1] == 0));
        }
    }

    fn outcome(&self, side: usize) -> BoutOutcome {
        let (side_hp, foe_hp, side_down, foe_down) = match (side, self.knockout) {
            (0, Some((a, b))) => (0, 0, a, b),
            (_, Some((a, b))) => (0, 0, b, a),
            (0, None) => (self.hp[0], self.hp[1], false, false),
            (_, None) => (self.hp[1], self.hp[0], false, false),
        };
        match (side_down, foe_down) {
            (false, true) => BoutOutcome::Win,
            (true, false) => BoutOutcome::Loss,
            (true, true) => BoutOutcome::Draw,
            (false, false) if side_hp > foe_hp => BoutOutcome::Win,
            (false, false) if side_hp < foe_hp => BoutOutcome::Loss,
            (false, false) => BoutOutcome::Draw,
        }
    }
}

/// Student plus opponent plus judge: what one strength-gate bout runs.
struct Ring<'m> {
    student: StudentController<'m>,
    opponent: Opponent,
    judge: Judge,
}

impl SlotController for Ring<'_> {
    fn controls(&self, slot: u8) -> bool {
        self.student.controls(slot) || self.opponent == Opponent::BrainIntents
    }

    fn control(&mut self, view: &ControlView<'_>, row: &mut [SlotInput]) {
        self.judge.watch(view.slots);
        if self.opponent == Opponent::BrainIntents {
            for slot in 0..row.len().min(view.slots.len()) {
                if !self.student.controls(u8::try_from(slot).unwrap_or(u8::MAX)) {
                    actuate_brain(view, row, slot);
                }
            }
        }
        self.student.control(view, row);
    }
}

/// Validated two-fighter duel config.
fn duel_config(seed: u64, ticks: u32) -> Result<ArenaConfig, DuelError> {
    ArenaConfig::try_new(seed, ticks, 2, ARENA_HALF_EXTENT_M).map_err(DuelError::BadConfig)
}

/// Play one bout: the student net in `student_slot` (0 or 1) against
/// `opponent` in the other slot, `ticks` fixed steps on `seed`. Returns the
/// student's outcome. Deterministic per `(net, seed, slot, ticks, opponent)`.
pub fn play_bout(
    model: &Model,
    seed: u64,
    student_slot: u8,
    ticks: u32,
    opponent: Opponent,
) -> Result<BoutOutcome, DuelError> {
    let config = duel_config(seed, ticks)?;
    let slot = student_slot.min(1);
    let mut ring = Ring {
        student: StudentController::new(model, slot)?,
        opponent,
        judge: Judge::default(),
    };
    if run_controlled(&config, &SlotTape(Vec::new()), &mut ring).is_empty() {
        return Err(DuelError::RunRejected);
    }
    Ok(ring.judge.outcome(usize::from(slot)))
}

/// Record one teacher episode: the depth brain in both slots, its intents
/// actuated, on `seed` for `ticks` steps; `(lanes, intent)` for both slots on
/// every perception tick, so the row count is
/// [`teacher_rows_per_episode`]`(ticks)` for every seed.
pub fn teacher_episode(seed: u64, ticks: u32) -> Result<Vec<TeacherRow>, DuelError> {
    let config = duel_config(seed, ticks)?;
    let mut tap = TeacherDriver::new();
    if run_controlled(&config, &SlotTape(Vec::new()), &mut tap).is_empty() {
        return Err(DuelError::RunRejected);
    }
    Ok(tap.rows)
}

/// Rows a [`teacher_episode`] of `ticks` steps records.
pub fn teacher_rows_per_episode(ticks: u32) -> usize {
    usize::try_from(ticks.div_ceil(AI_PERIOD_TICKS))
        .unwrap_or(usize::MAX)
        .saturating_mul(2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_input::FighterId as SlotId;

    fn view(x_mm: i32, z_mm: i32, facing_cdeg: i16, attacking: bool) -> SlotView {
        SlotView {
            alive: true,
            x_mm,
            z_mm,
            facing_cdeg,
            hp_permille: 1000,
            attacking,
            ..SlotView::default()
        }
    }

    fn input(mx: i8, buttons: u8) -> SlotInput {
        SlotInput {
            tick: 6,
            fighter: SlotId(1),
            mx,
            mz: 0,
            facing: 4500,
            buttons: Buttons(buttons),
        }
    }

    #[test]
    fn lane_table_pins_the_ai_feature_scales() {
        assert_eq!(DUEL_LANES, 6);
        assert_eq!(DUEL_CLASSES, 5);
        assert_eq!(DUEL_LANE_STEPS[engine_ai::LANE_DX], 7.9375);
        assert_eq!(DUEL_LANE_STEPS[engine_ai::LANE_DZ], 7.9375);
        assert_eq!(DUEL_LANE_STEPS[engine_ai::LANE_DY], 15.875);
        assert_eq!(DUEL_LANE_STEPS[engine_ai::LANE_DIST], 7.9375);
        assert_eq!(
            i32::from(engine_nn::quantize_lane(
                1.0,
                DUEL_LANE_STEPS[engine_ai::LANE_FOE_ATTACK]
            )),
            i32::from(engine_ai::FEATURE_ATTACK_ON)
        );
        assert_eq!(
            engine_nn::quantize_lane(PI, DUEL_LANE_STEPS[engine_ai::LANE_YAW]),
            127
        );
    }

    #[test]
    fn features_quantize_the_view_through_the_one_edge() {
        // Foe 3 m east, 4 m north, attacking; own facing 90 degrees.
        let slots = [view(1000, -2000, 9000, false), view(4000, 2000, 0, true)];
        let lanes = duel_lanes(&slots, 0).expect("two slots");
        assert_eq!(lanes, [3.0, 4.0, 0.0, 5.0, 1.0, 9000.0 * (PI / 18_000.0)]);
        // 3 * 7.9375 = 23.8 -> 24; 4 * 7.9375 = 31.75 -> 32; 5 -> 39.69 -> 40.
        let f = duel_features(&slots, 0);
        assert_eq!(&f[..5], &[24, 32, 0, 40, 127]);
        assert_eq!(f[5], engine_nn::quantize_lane(lanes[5], 127.0 / PI));
        // Mirror view: the foe sees the negated deltas, not attacking.
        let g = duel_features(&slots, 1);
        assert_eq!(&g[..5], &[-24, -32, 0, 40, 0]);
        assert_eq!(duel_lanes(&slots, 2), None);
        assert_eq!(duel_features(&slots[..1], 0), [0; DUEL_LANES]);
        assert_eq!(foe_delta_mm(&slots, 0), (3000, 4000));
        assert_eq!(foe_delta_mm(&slots[..1], 0), (0, 0));
    }

    #[test]
    fn labels_follow_buttons_then_wish_sign() {
        assert_eq!(intent_class(&input(30, ATTACK)), CLASS_ATTACK);
        assert_eq!(intent_class(&input(0, BLOCK)), CLASS_BLOCK);
        assert_eq!(intent_class(&input(0, DODGE)), CLASS_DODGE);
        assert_eq!(intent_class(&input(100, 0)), CLASS_ADVANCE);
        assert_eq!(intent_class(&input(60, 0)), CLASS_ADVANCE);
        assert_eq!(intent_class(&input(-80, 0)), CLASS_RETREAT);
    }

    #[test]
    fn actuator_steps_in_world_axes_toward_the_foe() {
        // Foe 3 m east, 4 m north.
        let to_foe = (3000, 4000);
        let adv = class_input(CLASS_ADVANCE, 12, input(0, 0), to_foe);
        assert_eq!((adv.mx, adv.mz, adv.buttons), (60, 80, Buttons::NONE));
        let atk = class_input(CLASS_ATTACK, 12, input(0, 0), to_foe);
        assert_eq!((atk.mx, atk.mz, atk.buttons), (18, 24, Buttons(ATTACK)));
        let ret = class_input(CLASS_RETREAT, 12, input(0, 0), to_foe);
        assert_eq!((ret.mx, ret.mz), (-48, -64));
        let blk = class_input(CLASS_BLOCK, 12, input(0, 0), to_foe);
        assert_eq!((blk.mx, blk.mz, blk.buttons), (0, 0, Buttons(BLOCK)));
        let even = class_input(CLASS_DODGE, 12, input(0, 0), to_foe);
        let odd = class_input(CLASS_DODGE, 18, input(0, 0), to_foe);
        assert_eq!((even.mx, even.mz), (-80, 60));
        assert_eq!((odd.mx, odd.mz), (80, -60));
        // Coincident fighters: no wish, buttons still apply.
        let still = class_input(CLASS_ATTACK, 0, input(0, 0), (0, 0));
        assert_eq!((still.mx, still.mz, still.buttons), (0, 0, Buttons(ATTACK)));
        // Every class validates; button classes round-trip the label rule.
        for class in 0..u8::try_from(DUEL_CLASSES).unwrap_or(0) {
            let mapped = class_input(class, 12, input(0, 0), (0, 5000));
            if class <= CLASS_DODGE {
                assert_eq!(intent_class(&mapped), class);
            }
            assert_eq!(mapped.facing, 4500, "facing passes through");
            assert_eq!(mapped.fighter, SlotId(1));
            mapped.validate().expect("mapped input validates");
        }
    }

    #[test]
    fn judge_scores_knockouts_then_health() {
        let mut judge = Judge::default();
        let mut slots = [view(0, 0, 0, false), view(1000, 0, 0, false)];
        slots[1].hp_permille = 400;
        judge.watch(&slots);
        assert_eq!(judge.outcome(0), BoutOutcome::Win);
        assert_eq!(judge.outcome(1), BoutOutcome::Loss);
        slots[0].hp_permille = 0;
        judge.watch(&slots);
        assert_eq!(judge.outcome(0), BoutOutcome::Loss);
        // The first knockout sticks even if health changes later.
        slots[1].hp_permille = 0;
        judge.watch(&slots);
        assert_eq!(judge.outcome(1), BoutOutcome::Win);
        let mut even = Judge::default();
        even.watch(&[view(0, 0, 0, false), view(1, 0, 0, false)]);
        assert_eq!(even.outcome(0), BoutOutcome::Draw);
        let mut double = Judge::default();
        let mut both = [view(0, 0, 0, false), view(1, 0, 0, false)];
        both[0].hp_permille = 0;
        both[1].hp_permille = 0;
        double.watch(&both);
        assert_eq!(double.outcome(0), BoutOutcome::Draw);
    }

    #[test]
    fn teacher_episode_is_fixed_length_deterministic_and_fights() {
        let a = teacher_episode(7, 1200).expect("duel runs");
        assert_eq!(a.len(), teacher_rows_per_episode(1200));
        assert_eq!(a.len(), 400);
        let b = teacher_episode(7, 1200).expect("duel runs again");
        assert_eq!(a, b);
        assert!(a.iter().all(|(_, class)| usize::from(*class) < DUEL_CLASSES));
        // The actuated duel closes: some rows sit inside 4 m and carry a
        // combat intent (attack, block or dodge).
        let close = a.iter().filter(|(lanes, _)| lanes[engine_ai::LANE_DIST] < 32).count();
        let combat = a.iter().filter(|(_, class)| *class <= CLASS_DODGE).count();
        assert!(close > 0 && combat > 0, "close {close}, combat {combat}");
        assert_eq!(
            teacher_episode(7, 0),
            Err(DuelError::BadConfig(engine_arena::ArenaError::BadTicks(0)))
        );
    }

    #[test]
    fn student_refuses_wrong_shapes() {
        let mut b = Vec::new();
        b.extend_from_slice(&engine_nn::MAGIC);
        b.push(engine_nn::VERSION);
        b.push(1);
        b.extend_from_slice(&2u16.to_le_bytes());
        b.push(engine_nn::TAG_DENSE);
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1i32.to_le_bytes());
        b.push(0);
        b.push(engine_nn::ACT_NONE);
        b.extend_from_slice(&0i32.to_le_bytes());
        b.extend_from_slice(&[1, 1]);
        let narrow = engine_nn::load(&b).expect("2 -> 1 net loads");
        let shape = DuelError::BadNetShape {
            input: 2,
            output: 1,
        };
        assert_eq!(StudentController::new(&narrow, 0).err(), Some(shape));
        assert_eq!(
            play_bout(&narrow, 1, 0, 60, Opponent::ArenaBrain),
            Err(shape)
        );
    }

    /// The committed distilled net (`distill_duel` defaults; regenerate or
    /// verify with `cargo run -p engine-distill --bin distill_duel -- --check`).
    const FIXTURE_NET: &[u8] = include_bytes!("../../../fixtures/nets/depth_student.e2nn");

    #[test]
    fn committed_net_is_pinned_and_bind_ready() {
        let model = engine_nn::load(FIXTURE_NET).expect("fixture net loads");
        assert_eq!(model.digest(), 0x6ace_b82b_2a85_6f5c, "regenerate with distill_duel");
        assert_eq!(model.layers().len(), 2);
        assert_eq!((model.input_dim(), model.output_dim()), (DUEL_LANES, DUEL_CLASSES));
        assert!(FIXTURE_NET.len() < 1024, "tiny net: {} B", FIXTURE_NET.len());
        // The arena-side net policy accepts the exact bytes (6 -> 5).
        engine_ai::NetPolicy::load(SlotId(0), FIXTURE_NET).expect("NetPolicy loads the net");
        // Two short bouts per side against the teacher replay bit-exactly.
        let first = crate::strength_gate(FIXTURE_NET, Opponent::BrainIntents, 0xF1, 4, 600)
            .expect("gate runs");
        let second = crate::strength_gate(FIXTURE_NET, Opponent::BrainIntents, 0xF1, 4, 600)
            .expect("gate runs again");
        assert_eq!(first, second);
    }
}
