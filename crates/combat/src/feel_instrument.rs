//! feel_instrument: impact-feel instrumentation over the feel rows.
//! Owner: engine-combat. Consumer: feel tuning (balance survey) — `run_suite`
//! grades hit-stop + camera coordination across 20 scripted scenarios, and
//! `pq2_balance_for` reuses the feel balance survey. Units: ticks at 60 Hz,
//! damage/trauma in permille, latency in ms. Determinism: integer math and
//! fighter-ID order only; same preset plus same base gives the same reports.
//! Errors: [`InstrumentError`](crate::feel_instrument::InstrumentError) for bad versions/scenarios/rows. Budget: the
//! 20-scenario survey is a headless harness (never the tick); per-scenario
//! cost is O(ticks * events) time into caller-owned buffers.

use crate::feel::{balance_over_200, BalanceReport, FeelError, FeelPreset};
use crate::fight::{step_with_tables_into, FightEvent, FightState, Tables};
use engine_core::checked::saturating_u32_to_u16;
use engine_input::fighter::{ATTACK, BLOCK, DODGE};
use engine_input::{Buttons, ParticipantId, ParticipantInput};

/// Combat scenarios in the suite: 5 situations x 4 hit-stop/camera rigs.
pub const SCENARIO_COUNT: usize = 20;
/// Same count as `u8` for id arithmetic.
pub const SCENARIO_COUNT_U8: u8 = 20;
/// Impact-feedback features in the engine-side inventory (ideas only: an
/// engine inventory inspired by Lin et al. 2208.06155v3 section 4, not a
/// copy of the paper's list).
pub const FEATURE_COUNT: usize = 19;
/// Fixed tick rate the latency model assumes; the rate's one owner is
/// `engine_core::TICK_HZ` and its checked narrow form is
/// `engine_core::TICK_HZ_U32`, which this constant takes.
pub const TICKS_PER_SEC: u32 = engine_core::TICK_HZ_U32;
/// p95 input-to-effect ceiling in milliseconds (packet contract).
pub const P95_LATENCY_MS: u32 = 50;
/// Minimum coordinated-vs-baseline impact gain in permille (+10%).
pub const MIN_COORD_GAIN_PERMILLE: u32 = 100;
/// Ticks simulated per scenario bout (covers startup, impact and freeze).
pub const SUITE_TICKS: u32 = 16;
/// Combat situations scripted per rig.
pub const SITUATION_COUNT: u8 = 5;
/// Hit-stop/camera rigs scripted per situation.
pub const RIG_COUNT: u8 = 4;
/// Rig index of the deterministic hit-stop baseline (hit-stop, no camera).
pub const RIG_BASELINE: u8 = 1;
/// Rig index of hit-stop + camera coordination.
pub const RIG_COORDINATED: u8 = 3;
/// Sound delay a rig without coherence reports (worst case, incoherent).
pub const INCOHERENT_SOUND_DELAY_TICKS: u32 = 12;

// ---- feature inventory ----------------------------------------------------

/// One impact-feedback feature in the engine-side inventory.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ImpactFeature {
    HitStop,
    SoundCoherence,
    CameraControl,
    CameraShake,
    HitFlash,
    HitSpark,
    ImpactSound,
    DamageNumber,
    Knockback,
    HitReact,
    Stagger,
    Knockdown,
    BlockChip,
    ParryWindow,
    DodgeIframe,
    InputBuffer,
    CancelGrace,
    TraumaDecay,
    FrameFreeze,
}

impl ImpactFeature {
    /// All 19 features in index order.
    pub const ALL: [Self; FEATURE_COUNT] = [
        Self::HitStop,
        Self::SoundCoherence,
        Self::CameraControl,
        Self::CameraShake,
        Self::HitFlash,
        Self::HitSpark,
        Self::ImpactSound,
        Self::DamageNumber,
        Self::Knockback,
        Self::HitReact,
        Self::Stagger,
        Self::Knockdown,
        Self::BlockChip,
        Self::ParryWindow,
        Self::DodgeIframe,
        Self::InputBuffer,
        Self::CancelGrace,
        Self::TraumaDecay,
        Self::FrameFreeze,
    ];

    /// Stable inventory name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::HitStop => "hit_stop",
            Self::SoundCoherence => "sound_coherence",
            Self::CameraControl => "camera_control",
            Self::CameraShake => "camera_shake",
            Self::HitFlash => "hit_flash",
            Self::HitSpark => "hit_spark",
            Self::ImpactSound => "impact_sound",
            Self::DamageNumber => "damage_number",
            Self::Knockback => "knockback",
            Self::HitReact => "hit_react",
            Self::Stagger => "stagger",
            Self::Knockdown => "knockdown",
            Self::BlockChip => "block_chip",
            Self::ParryWindow => "parry_window",
            Self::DodgeIframe => "dodge_iframe",
            Self::InputBuffer => "input_buffer",
            Self::CancelGrace => "cancel_grace",
            Self::TraumaDecay => "trauma_decay",
            Self::FrameFreeze => "frame_freeze",
        }
    }

    /// Index in [`ImpactFeature::ALL`].
    pub const fn index(self) -> u8 {
        match self {
            Self::HitStop => 0,
            Self::SoundCoherence => 1,
            Self::CameraControl => 2,
            Self::CameraShake => 3,
            Self::HitFlash => 4,
            Self::HitSpark => 5,
            Self::ImpactSound => 6,
            Self::DamageNumber => 7,
            Self::Knockback => 8,
            Self::HitReact => 9,
            Self::Stagger => 10,
            Self::Knockdown => 11,
            Self::BlockChip => 12,
            Self::ParryWindow => 13,
            Self::DodgeIframe => 14,
            Self::InputBuffer => 15,
            Self::CancelGrace => 16,
            Self::TraumaDecay => 17,
            Self::FrameFreeze => 18,
        }
    }

    /// Resolve an inventory index; `None` outside `0..19`.
    pub const fn from_index(index: u8) -> Option<Self> {
        match index {
            0 => Some(Self::HitStop),
            1 => Some(Self::SoundCoherence),
            2 => Some(Self::CameraControl),
            3 => Some(Self::CameraShake),
            4 => Some(Self::HitFlash),
            5 => Some(Self::HitSpark),
            6 => Some(Self::ImpactSound),
            7 => Some(Self::DamageNumber),
            8 => Some(Self::Knockback),
            9 => Some(Self::HitReact),
            10 => Some(Self::Stagger),
            11 => Some(Self::Knockdown),
            12 => Some(Self::BlockChip),
            13 => Some(Self::ParryWindow),
            14 => Some(Self::DodgeIframe),
            15 => Some(Self::InputBuffer),
            16 => Some(Self::CancelGrace),
            17 => Some(Self::TraumaDecay),
            18 => Some(Self::FrameFreeze),
            _ => None,
        }
    }

    /// True for the three features the paper finds strongest (hit-stop,
    /// sound coherence, camera control); only these vary across rigs.
    pub const fn is_core(self) -> bool {
        matches!(
            self,
            Self::HitStop | Self::SoundCoherence | Self::CameraControl
        )
    }
}

// ---- scenarios ------------------------------------------------------------

/// One scripted combat situation per rig.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Situation {
    Hit,
    Blocked,
    Parried,
    Dodged,
    Knockdown,
}

impl Situation {
    /// Index in scenario-id order (`id % 5`).
    pub const fn index(self) -> u8 {
        match self {
            Self::Hit => 0,
            Self::Blocked => 1,
            Self::Parried => 2,
            Self::Dodged => 3,
            Self::Knockdown => 4,
        }
    }

    /// Resolve a situation index; `None` outside `0..5`.
    pub const fn from_index(index: u8) -> Option<Self> {
        match index {
            0 => Some(Self::Hit),
            1 => Some(Self::Blocked),
            2 => Some(Self::Parried),
            3 => Some(Self::Dodged),
            4 => Some(Self::Knockdown),
            _ => None,
        }
    }

    /// Stable situation name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Hit => "hit",
            Self::Blocked => "blocked",
            Self::Parried => "parried",
            Self::Dodged => "dodged",
            Self::Knockdown => "knockdown",
        }
    }
}

/// One ablation rig: hit-stop and camera/sound toggled independently.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rig {
    /// The sim freezes on impact (affects timing, never forks winners).
    pub hitstop: bool,
    /// Presentation adds a camera kick (never read back by the sim).
    pub camera: bool,
    /// Presentation plays coherent impact audio (never read by the sim).
    pub sound: bool,
}

impl Rig {
    /// No instrumentation at all.
    pub const NONE: Self = Self {
        hitstop: false,
        camera: false,
        sound: false,
    };
    /// Deterministic hit-stop baseline: freeze, no camera coordination.
    pub const BASELINE: Self = Self {
        hitstop: true,
        camera: false,
        sound: false,
    };
    /// Camera and sound only, no freeze.
    pub const CAMERA: Self = Self {
        hitstop: false,
        camera: true,
        sound: true,
    };
    /// Hit-stop plus camera/sound coordination.
    pub const COORDINATED: Self = Self {
        hitstop: true,
        camera: true,
        sound: true,
    };

    /// Resolve a rig index (`id / 5`); `None` outside `0..4`.
    pub const fn for_index(index: u8) -> Option<Self> {
        match index {
            0 => Some(Self::NONE),
            1 => Some(Self::BASELINE),
            2 => Some(Self::CAMERA),
            3 => Some(Self::COORDINATED),
            _ => None,
        }
    }

    /// Core features this rig instruments (0..=3).
    pub const fn core_covered_count(self) -> u32 {
        (if self.hitstop { 1 } else { 0 })
            + (if self.sound { 1 } else { 0 })
            + (if self.camera { 1 } else { 0 })
    }

    /// True while this rig instruments `feature`. The three core features
    /// follow the rig toggles; the rest ride the shared fight rows in every
    /// rig, so ablation isolates exactly hit-stop, coherence and camera.
    pub const fn covers(self, feature: ImpactFeature) -> bool {
        match feature {
            ImpactFeature::HitStop | ImpactFeature::FrameFreeze => self.hitstop,
            ImpactFeature::SoundCoherence | ImpactFeature::ImpactSound => self.sound,
            ImpactFeature::CameraControl
            | ImpactFeature::CameraShake
            | ImpactFeature::TraumaDecay => self.camera,
            _ => true,
        }
    }
}

/// One of the 20 suite scenarios: a situation under a rig.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Scenario {
    /// Suite id, `0..20`.
    pub id: u8,
    /// Scripted combat situation (`id % 5`).
    pub situation: Situation,
    /// Ablation rig (`id / 5`).
    pub rig: Rig,
    /// Rig index (`id / 5`).
    pub rig_index: u8,
}

impl Scenario {
    /// Resolve a suite id; `None` outside `0..20`.
    pub const fn for_index(id: u8) -> Option<Self> {
        if id >= SCENARIO_COUNT_U8 {
            return None;
        }
        let situation = match Situation::from_index(id % SITUATION_COUNT) {
            Some(situation) => situation,
            None => return None,
        };
        let rig = match Rig::for_index(id / SITUATION_COUNT) {
            Some(rig) => rig,
            None => return None,
        };
        Some(Self {
            id,
            situation,
            rig,
            rig_index: id / SITUATION_COUNT,
        })
    }
}

/// Bounded instrumentation failures; invalid user data never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InstrumentError {
    /// Suite id outside `0..20`, culprit carried.
    BadScenario(u8),
    /// Version 0 is reserved for legacy untagged replays, never valid anew.
    BadVersion(u32),
    /// One tunable row is outside its sane range; first failure wins.
    BadValue {
        /// Table field name, e.g. `"damage_permille"`.
        field: &'static str,
        /// Culprit value widened to `u32`.
        value: u32,
    },
    /// The caller report buffer is too small; `need` is always 20.
    BufferTooSmall {
        /// Reports required.
        need: usize,
        /// Reports provided.
        have: usize,
    },
}

/// One measured scenario: sim facts plus presentation-side impact.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ScenarioReport {
    /// Suite id, `0..20`.
    pub scenario_id: u8,
    /// Scripted situation.
    pub situation: Situation,
    /// Ablation rig.
    pub rig: Rig,
    /// Rig index (`id / 5`).
    pub rig_index: u8,
    /// Fight events observed across the bout.
    pub event_count: u16,
    /// Landed damage in permille (hits plus block chip).
    pub damage_total: u16,
    /// Sim hit-stop ticks on impact (0 when the rig disables the freeze).
    pub hitstop_ticks: u32,
    /// Presentation camera kick in permille (0 without camera or impact).
    pub camera_kick_permille: u16,
    /// Presentation sound delay in ticks (worst case without coherence).
    pub sound_delay_ticks: u32,
    /// Impact score in permille-scale points (integer proxy, ideas only).
    pub impact: u32,
    /// Added input-to-effect latency in ms for a fresh press: the ticks the
    /// fight tick adds before the swing starts (0 under the shipped tick).
    /// The intake window is a forgiveness budget for a locked-swing press,
    /// never a delay, so this reading is not the buffer row in ms.
    pub latency_ms: u32,
}

impl ScenarioReport {
    /// Quiet placeholder for stack arrays filled by [`run_suite_into`].
    pub const ZERO: Self = Self {
        scenario_id: 0,
        situation: Situation::Hit,
        rig: Rig::NONE,
        rig_index: 0,
        event_count: 0,
        damage_total: 0,
        hitstop_ticks: 0,
        camera_kick_permille: 0,
        sound_delay_ticks: 0,
        impact: 0,
        latency_ms: 0,
    };
}

/// Suite summary over the 20 reports: baseline vs coordinated means.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SuiteSummary {
    /// Mean impact of the 5 baseline (hit-stop, no camera) scenarios.
    pub baseline_avg: u32,
    /// Mean impact of the 5 coordinated scenarios.
    pub coordinated_avg: u32,
    /// Coordinated gain over baseline in permille (`0` on an empty base).
    pub gain_permille: u32,
    /// p95 input latency across the 20 scenarios in ms.
    pub p95_latency_ms: u32,
    /// Scenarios measured (always 20).
    pub scenarios: u8,
}

// ---- impact model (integer only, ideas only) -------------------------------

/// Impact proxy for one bout: landed damage plus knockdown/parry bonuses,
/// a hit-stop term, and camera plus coherence bonuses that fire only on a
/// real impact (damage, knockdown or parry), so whiffs never score.
/// Integer only, saturating; presentation-side, never stepped.
pub const fn impact_for(
    damage: u16,
    knockdowns: u32,
    parries: u32,
    hitstop_ticks: u32,
    camera: bool,
    sound: bool,
) -> u32 {
    let mut score = (damage as u32)
        .saturating_add(knockdowns.saturating_mul(400))
        .saturating_add(parries.saturating_mul(200));
    score = score.saturating_add(hitstop_ticks.saturating_mul(20));
    let landed = damage > 0 || knockdowns > 0 || parries > 0;
    if camera && landed {
        score = score.saturating_add(200 + damage as u32 * 300 / 1000);
    }
    if sound && landed {
        score = score.saturating_add(150);
    }
    score
}

/// Presentation camera kick: trauma peak scaled by damage, half peak on a
/// damageless parry, quiet otherwise. Integer only, never read by the sim.
pub const fn camera_kick_for(damage: u16, parries: u32, trauma_peak: u16, camera: bool) -> u16 {
    if !camera {
        return 0;
    }
    if damage > 0 {
        let capped = if damage > 1000 {
            1000u32
        } else {
            damage as u32
        };
        return saturating_u32_to_u16(trauma_peak as u32 * capped / 1000);
    }
    if parries > 0 {
        return trauma_peak / 2;
    }
    0
}

/// Input-to-effect latency in ms added to a fresh press under a preset: the
/// tick's own number ([`FeelPreset::pq1_latency_ms`]), never the preset's
/// buffer row in ms. Consumer: the per-scenario report (`measure_one`) and,
/// through it, the p95 gate.
pub const fn input_latency_ms(preset: FeelPreset) -> u32 {
    preset.pq1_latency_ms()
}

// ---- suite -----------------------------------------------------------------

fn map_feel(error: FeelError) -> InstrumentError {
    match error {
        FeelError::BadVersion(version) => InstrumentError::BadVersion(version),
        FeelError::BadValue { field, value } => InstrumentError::BadValue { field, value },
        FeelError::BufferFull => InstrumentError::BadValue {
            field: "stream_buffer",
            value: u32::MAX,
        },
    }
}

fn fighter_input(fighter: u8, buttons: u8, tick: u32) -> Result<ParticipantInput, InstrumentError> {
    match ParticipantId::new(fighter) {
        Some(id) => Ok(ParticipantInput {
            tick,
            fighter: id,
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons(buttons),
        }),
        None => Err(InstrumentError::BadValue {
            field: "fighter",
            value: u32::from(fighter),
        }),
    }
}

/// Scripted buttons for one situation at one bout tick: the attacker opens
/// at tick 0, and the defender answers per situation. Impact lands at tick
/// 4 under the shipped startup, so a block held from 0 chips (age 4 past
/// the window), a block pressed at 3 parries (age 1), and a dodge at 1
/// i-frames through it (age 3 of 6).
const fn script_buttons(situation: Situation, tick: u32) -> (u8, u8) {
    let attacker = if tick == 0 { ATTACK } else { 0 };
    let defender = match situation {
        Situation::Hit | Situation::Knockdown => 0,
        Situation::Blocked => BLOCK,
        Situation::Parried => {
            if tick >= 3 && tick <= 6 {
                BLOCK
            } else {
                0
            }
        }
        Situation::Dodged => {
            if tick == 1 {
                DODGE
            } else {
                0
            }
        }
    };
    (attacker, defender)
}

/// Event totals folded over one scripted bout.
struct BoutTally {
    event_count: u32,
    damage_sum: u32,
    hits: u32,
    parries: u32,
    knockdowns: u32,
}

/// Step the scripted two-fighter bout for `situation` over [`SUITE_TICKS`]
/// ticks under `tables` and tally its events. Owns a scratch event buffer, so
/// it belongs to the headless harness, never the tick.
fn run_bout(situation: Situation, tables: &Tables) -> Result<BoutTally, InstrumentError> {
    let mut state = match FightState::new(2) {
        Ok(state) => state,
        Err(_) => {
            return Err(InstrumentError::BadValue {
                field: "fighter_count",
                value: 2,
            });
        }
    };
    if situation == Situation::Knockdown {
        state.fighters[1].hp_permille = tables.attack_damage.max(1);
    }
    let mut events = Vec::new();
    let mut tally = BoutTally {
        event_count: 0,
        damage_sum: 0,
        hits: 0,
        parries: 0,
        knockdowns: 0,
    };
    let mut tick = 0;
    while tick < SUITE_TICKS {
        let (attacker_buttons, defender_buttons) = script_buttons(situation, tick);
        let inputs = [
            fighter_input(0, attacker_buttons, tick)?,
            fighter_input(1, defender_buttons, tick)?,
        ];
        step_with_tables_into(&inputs, &mut state, tables, &mut events);
        let mut i = 0;
        while i < events.len() {
            tally.event_count += 1;
            match events[i] {
                FightEvent::Hit { damage, .. } => {
                    tally.hits += 1;
                    tally.damage_sum = tally.damage_sum.saturating_add(u32::from(damage));
                }
                FightEvent::Blocked { chip, .. } => {
                    tally.damage_sum = tally.damage_sum.saturating_add(u32::from(chip));
                }
                FightEvent::Parried { .. } => {
                    tally.parries += 1;
                }
                FightEvent::Dodged { .. } => {}
                FightEvent::StaminaOut { .. } => {}
                FightEvent::Knockdown { .. } => {
                    tally.knockdowns += 1;
                }
            }
            i += 1;
        }
        tick += 1;
    }
    Ok(tally)
}

/// Measure one suite scenario: step a scripted bout under the preset rows
/// (hit-stop zeroed when the rig disables the freeze) and fold sim facts
/// plus presentation-side impact into one report. Headless harness only:
/// it owns a scratch event buffer, never the tick.
pub fn measure_one(
    index: u8,
    preset: &FeelPreset,
    base: &Tables,
) -> Result<ScenarioReport, InstrumentError> {
    let scenario = match Scenario::for_index(index) {
        Some(scenario) => scenario,
        None => return Err(InstrumentError::BadScenario(index)),
    };
    preset.validate().map_err(map_feel)?;
    let mut tables = preset.tables(*base);
    if !scenario.rig.hitstop {
        tables.hitstop_ticks = 0;
    }
    let BoutTally {
        event_count,
        damage_sum,
        hits,
        parries,
        knockdowns,
    } = run_bout(scenario.situation, &tables)?;
    let damage_total = if damage_sum > u32::from(u16::MAX) {
        u16::MAX
    } else {
        u16::try_from(damage_sum).unwrap_or(u16::MAX)
    };
    let hitstop_ticks = if scenario.rig.hitstop && (hits > 0 || parries > 0) {
        tables.hitstop_ticks
    } else {
        0
    };
    let camera_kick = camera_kick_for(
        damage_total,
        parries,
        preset.juicing.trauma_permille,
        scenario.rig.camera,
    );
    let sound_delay = if scenario.rig.sound {
        preset.juicing.sound_delay_ticks
    } else {
        INCOHERENT_SOUND_DELAY_TICKS
    };
    Ok(ScenarioReport {
        scenario_id: scenario.id,
        situation: scenario.situation,
        rig: scenario.rig,
        rig_index: scenario.rig_index,
        event_count: if event_count > u32::from(u16::MAX) {
            u16::MAX
        } else {
            u16::try_from(event_count).unwrap_or(u16::MAX)
        },
        damage_total,
        hitstop_ticks,
        camera_kick_permille: camera_kick,
        sound_delay_ticks: sound_delay,
        impact: impact_for(
            damage_total,
            knockdowns,
            parries,
            hitstop_ticks,
            scenario.rig.camera,
            scenario.rig.sound,
        ),
        latency_ms: input_latency_ms(*preset),
    })
}

/// Measure all 20 scenarios into a caller buffer (no heap for reports; the
/// per-bout scratch buffers are harness-only). Deterministic: same preset
/// plus same base always fills the same reports in id order.
pub fn run_suite_into(
    preset: &FeelPreset,
    base: &Tables,
    out: &mut [ScenarioReport],
) -> Result<(), InstrumentError> {
    if out.len() < SCENARIO_COUNT {
        return Err(InstrumentError::BufferTooSmall {
            need: SCENARIO_COUNT,
            have: out.len(),
        });
    }
    let mut id = 0u8;
    while id < SCENARIO_COUNT_U8 {
        out[usize::from(id)] = measure_one(id, preset, base)?;
        id += 1;
    }
    Ok(())
}

/// Measure all 20 scenarios into a stack array. Same reports as
/// [`run_suite_into`]; the array keeps the harness off the heap.
pub fn run_suite(
    preset: &FeelPreset,
    base: &Tables,
) -> Result<[ScenarioReport; SCENARIO_COUNT], InstrumentError> {
    let mut reports = [ScenarioReport::ZERO; SCENARIO_COUNT];
    run_suite_into(preset, base, &mut reports)?;
    Ok(reports)
}

/// p95 input latency across the suite in ms: ascending insertion sort over
/// the 20 report latencies, 19th of 20 (index 18). Integer only.
pub fn p95_latency_ms(reports: &[ScenarioReport; SCENARIO_COUNT]) -> u32 {
    let mut latencies = [0u32; SCENARIO_COUNT];
    let mut i = 0;
    while i < SCENARIO_COUNT {
        latencies[i] = reports[i].latency_ms;
        i += 1;
    }
    let mut i = 1;
    while i < SCENARIO_COUNT {
        let key = latencies[i];
        let mut j = i;
        while j > 0 && latencies[j - 1] > key {
            latencies[j] = latencies[j - 1];
            j -= 1;
        }
        latencies[j] = key;
        i += 1;
    }
    latencies[SCENARIO_COUNT - 2]
}

/// Summarize a measured suite: baseline vs coordinated means, permille
/// gain (`0` on an empty base), p95 latency and the scenario count.
pub fn summarize(reports: &[ScenarioReport; SCENARIO_COUNT]) -> SuiteSummary {
    let mut baseline_sum = 0u32;
    let mut coordinated_sum = 0u32;
    let mut i = 0;
    while i < SCENARIO_COUNT {
        if reports[i].rig_index == RIG_BASELINE {
            baseline_sum = baseline_sum.saturating_add(reports[i].impact);
        }
        if reports[i].rig_index == RIG_COORDINATED {
            coordinated_sum = coordinated_sum.saturating_add(reports[i].impact);
        }
        i += 1;
    }
    let baseline_avg = baseline_sum / u32::from(SITUATION_COUNT);
    let coordinated_avg = coordinated_sum / u32::from(SITUATION_COUNT);
    let gain = coordinated_avg
        .saturating_sub(baseline_avg)
        .saturating_mul(1000)
        .checked_div(baseline_avg)
        .unwrap_or(0);
    SuiteSummary {
        baseline_avg,
        coordinated_avg,
        gain_permille: gain,
        p95_latency_ms: p95_latency_ms(reports),
        scenarios: SCENARIO_COUNT_U8,
    }
}

/// Packet gates: p95 fits 50 ms and coordination gains at least +10%.
/// Latency is checked first, deterministically.
pub fn check_gates(summary: &SuiteSummary) -> Result<(), InstrumentError> {
    if summary.p95_latency_ms > P95_LATENCY_MS {
        return Err(InstrumentError::BadValue {
            field: "p95_latency_ms",
            value: summary.p95_latency_ms,
        });
    }
    if summary.gain_permille < MIN_COORD_GAIN_PERMILLE {
        return Err(InstrumentError::BadValue {
            field: "gain_permille",
            value: summary.gain_permille,
        });
    }
    Ok(())
}

/// Balance survey for a feel preset: validate, resolve the tuned rows over
/// `base`, and survey 200 seeded bouts. Same rows always agree. This is
/// the consumer wiring: feel tuning grades presets through this entry.
pub fn pq2_balance_for(
    preset: &FeelPreset,
    base: &Tables,
) -> Result<BalanceReport, InstrumentError> {
    preset.validate().map_err(map_feel)?;
    Ok(balance_over_200(&preset.tables(*base)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feel::{FeelPreset, JuicingParams, StreamliningParams, TuningParams, FEEL_V1};
    use crate::fight::TABLES_V1;

    fn v1_reports() -> [ScenarioReport; SCENARIO_COUNT] {
        run_suite(&FeelPreset::v1(), &TABLES_V1).expect("v1 suite measures")
    }

    #[test]
    fn inventory_has_19_features_with_3_core() {
        assert_eq!(ImpactFeature::ALL.len(), FEATURE_COUNT);
        assert_eq!(ImpactFeature::from_index(19), None);
        let mut core = 0u32;
        let mut i = 0u8;
        while i < u8::try_from(FEATURE_COUNT).unwrap_or(u8::MAX) {
            let feature = ImpactFeature::from_index(i).expect("inventory index resolves");
            assert_eq!(feature.index(), i);
            assert_eq!(ImpactFeature::from_index(feature.index()), Some(feature));
            if feature.is_core() {
                core += 1;
            }
            // Names are unique across the inventory.
            let mut j = 0u8;
            while j < i {
                let other = ImpactFeature::from_index(j).expect("inventory index resolves");
                assert_ne!(feature.name(), other.name(), "feature name repeats");
                j += 1;
            }
            i += 1;
        }
        assert_eq!(core, 3, "exactly hit-stop, coherence and camera are core");
        assert!(ImpactFeature::HitStop.is_core());
        assert!(ImpactFeature::SoundCoherence.is_core());
        assert!(ImpactFeature::CameraControl.is_core());
        assert!(!ImpactFeature::InputBuffer.is_core());
    }

    #[test]
    fn twenty_scenarios_cover_5_situations_x_4_rigs() {
        let mut id = 0u8;
        while id < SCENARIO_COUNT_U8 {
            let scenario = Scenario::for_index(id).expect("suite id resolves");
            assert_eq!(scenario.id, id);
            assert_eq!(
                scenario.situation,
                Situation::from_index(id % SITUATION_COUNT).unwrap()
            );
            assert_eq!(scenario.rig_index, id / SITUATION_COUNT);
            assert_eq!(
                scenario.situation.index() % SITUATION_COUNT,
                id % SITUATION_COUNT
            );
            id += 1;
        }
        assert_eq!(Scenario::for_index(SCENARIO_COUNT_U8), None);
        assert_eq!(Rig::for_index(RIG_COUNT), None);
        assert_eq!(Situation::from_index(SITUATION_COUNT), None);
        // Rigs toggle hit-stop and camera independently.
        assert_eq!(Rig::for_index(0), Some(Rig::NONE));
        assert_eq!(Rig::for_index(RIG_BASELINE), Some(Rig::BASELINE));
        assert_eq!(Rig::for_index(RIG_COORDINATED), Some(Rig::COORDINATED));
        assert!(Rig::COORDINATED.covers(ImpactFeature::HitStop));
        assert!(Rig::COORDINATED.covers(ImpactFeature::CameraControl));
        assert!(Rig::COORDINATED.covers(ImpactFeature::SoundCoherence));
        assert_eq!(Rig::COORDINATED.core_covered_count(), 3);
        assert_eq!(Rig::BASELINE.core_covered_count(), 1);
        assert!(!Rig::BASELINE.covers(ImpactFeature::CameraControl));
    }

    #[test]
    fn scripts_fire_their_named_event() {
        let preset = FeelPreset::v1();
        // Coordinated rig, one scenario per situation (ids 15..20).
        let hit = measure_one(15, &preset, &TABLES_V1).unwrap();
        assert!(
            hit.damage_total == preset.tuning.damage_permille,
            "hit lands full damage"
        );
        assert_eq!(hit.event_count, 1);

        let blocked = measure_one(16, &preset, &TABLES_V1).unwrap();
        assert_eq!(blocked.event_count, 1);
        assert!(blocked.damage_total > 0 && blocked.damage_total < hit.damage_total);

        let parried = measure_one(17, &preset, &TABLES_V1).unwrap();
        assert_eq!(parried.damage_total, 0, "parry soaks all damage");
        assert_eq!(
            parried.hitstop_ticks,
            preset.tables(TABLES_V1).hitstop_ticks
        );

        let dodged = measure_one(18, &preset, &TABLES_V1).unwrap();
        assert_eq!(dodged.damage_total, 0);
        assert_eq!(dodged.impact, 0, "whiffs never score, even coordinated");
        assert_eq!(dodged.event_count, 1);

        let knocked = measure_one(19, &preset, &TABLES_V1).unwrap();
        assert_eq!(knocked.event_count, 2, "hit plus knockdown");
        assert!(
            knocked.impact > hit.impact,
            "knockdown outranks a plain hit"
        );
    }

    #[test]
    fn suite_replays_identically() {
        let preset = FeelPreset::v1();
        let first = run_suite(&preset, &TABLES_V1).unwrap();
        let second = run_suite(&preset, &TABLES_V1).unwrap();
        assert_eq!(first, second);
        // Id order is the suite order.
        let mut id = 0u8;
        while id < SCENARIO_COUNT_U8 {
            assert_eq!(first[id as usize].scenario_id, id);
            id += 1;
        }
    }

    #[test]
    fn coordinated_beats_flat_by_10pct() {
        let reports = v1_reports();
        let summary = summarize(&reports);
        assert_eq!(summary.scenarios, SCENARIO_COUNT_U8);
        assert!(summary.coordinated_avg > summary.baseline_avg);
        assert!(
            summary.gain_permille >= MIN_COORD_GAIN_PERMILLE,
            "gain {} below +10%: {summary:?}",
            summary.gain_permille
        );
        assert_eq!(check_gates(&summary), Ok(()));
    }

    /// The p95 pin in the same unit as the PQ1 pin: the latency the tick adds
    /// to a fresh press, which is 0 ms for every preset. A wider buffer row
    /// buys forgiveness, never delay (it used to read 33 ms off a 2-tick row
    /// the tick never read). The 50 ms ceiling still refuses a reading over it.
    #[test]
    fn p95_latency_within_50ms() {
        let reports = v1_reports();
        assert_eq!(
            p95_latency_ms(&reports),
            0,
            "a fresh press swings on its press tick"
        );
        assert_eq!(check_gates(&summarize(&reports)), Ok(()));
        // The old failure leg, inverted: four buffer ticks on the row no
        // longer cost 66 ms, because the row is not a delay.
        let mut streamlining = StreamliningParams::v1();
        streamlining.input_buffer_ticks = 4;
        let wide = FeelPreset::new(2, TuningParams::v1(), JuicingParams::v1(), streamlining);
        assert_eq!(input_latency_ms(wide), 0);
        let wide_reports = run_suite(&wide, &TABLES_V1).unwrap();
        assert_eq!(p95_latency_ms(&wide_reports), 0);
        assert_eq!(check_gates(&summarize(&wide_reports)), Ok(()));
        // The ceiling keeps its teeth: a reading over 50 ms is refused.
        let over = SuiteSummary {
            p95_latency_ms: P95_LATENCY_MS + 1,
            ..summarize(&reports)
        };
        assert_eq!(
            check_gates(&over),
            Err(InstrumentError::BadValue {
                field: "p95_latency_ms",
                value: P95_LATENCY_MS + 1
            })
        );
    }

    #[test]
    fn camera_never_forks_the_sim() {
        let preset = FeelPreset::v1();
        // Same situation and freeze, camera off (5) vs on (15): the fight
        // agrees exactly, only the presentation impact differs.
        let flat = measure_one(5, &preset, &TABLES_V1).unwrap();
        let coordinated = measure_one(15, &preset, &TABLES_V1).unwrap();
        assert_eq!(flat.rig_index, RIG_BASELINE);
        assert_eq!(coordinated.rig_index, RIG_COORDINATED);
        assert_eq!(flat.damage_total, coordinated.damage_total);
        assert_eq!(flat.event_count, coordinated.event_count);
        assert_eq!(flat.camera_kick_permille, 0);
        assert!(coordinated.camera_kick_permille > 0);
        assert!(coordinated.impact > flat.impact);
        assert!(coordinated.sound_delay_ticks < flat.sound_delay_ticks);
    }

    #[test]
    fn hitstop_freeze_is_sim_visible() {
        let preset = FeelPreset::v1();
        // Same hit script, freeze off (0) vs on (5): same damage, and the
        // report shows the frozen ticks the sim actually held.
        let none = measure_one(0, &preset, &TABLES_V1).unwrap();
        let baseline = measure_one(5, &preset, &TABLES_V1).unwrap();
        assert_eq!(none.damage_total, baseline.damage_total);
        assert_eq!(none.hitstop_ticks, 0);
        assert_eq!(
            baseline.hitstop_ticks,
            preset.tables(TABLES_V1).hitstop_ticks
        );
    }

    #[test]
    fn validation_rejects_bad_rows() {
        let preset = FeelPreset::v1();
        assert_eq!(
            measure_one(SCENARIO_COUNT_U8, &preset, &TABLES_V1),
            Err(InstrumentError::BadScenario(SCENARIO_COUNT_U8))
        );
        let bad_version = FeelPreset::new(
            0,
            TuningParams::v1(),
            JuicingParams::v1(),
            StreamliningParams::v1(),
        );
        assert_eq!(
            measure_one(0, &bad_version, &TABLES_V1),
            Err(InstrumentError::BadVersion(0))
        );
        let mut tuning = TuningParams::v1();
        tuning.damage_permille = 0;
        let bad_row = FeelPreset::new(2, tuning, JuicingParams::v1(), StreamliningParams::v1());
        assert!(matches!(
            measure_one(0, &bad_row, &TABLES_V1),
            Err(InstrumentError::BadValue {
                field: "damage_permille",
                ..
            })
        ));
        assert_eq!(
            pq2_balance_for(&bad_version, &TABLES_V1),
            Err(InstrumentError::BadVersion(0))
        );
        // A short caller buffer is refused before measuring anything.
        let mut short = [ScenarioReport::ZERO; 3];
        assert_eq!(
            run_suite_into(&preset, &TABLES_V1, &mut short),
            Err(InstrumentError::BufferTooSmall {
                need: SCENARIO_COUNT,
                have: 3
            })
        );
        // The full-width buffer fills end to end in id order.
        let mut full = [ScenarioReport::ZERO; SCENARIO_COUNT];
        assert_eq!(run_suite_into(&preset, &TABLES_V1, &mut full), Ok(()));
        assert_eq!(full, v1_reports());
    }

    #[test]
    fn impact_model_spot_checks() {
        assert_eq!(impact_for(0, 0, 0, 0, false, false), 0);
        assert_eq!(impact_for(0, 0, 0, 0, true, true), 0, "whiffs stay quiet");
        let flat = impact_for(120, 0, 0, 4, false, false);
        let coordinated = impact_for(120, 0, 0, 4, true, true);
        assert_eq!(flat, 200);
        assert_eq!(coordinated, 586);
        assert_eq!(camera_kick_for(1000, 0, 600, true), 600);
        assert_eq!(camera_kick_for(500, 0, 600, true), 300);
        assert_eq!(
            camera_kick_for(0, 1, 600, true),
            300,
            "parry takes half peak"
        );
        assert_eq!(camera_kick_for(120, 0, 600, false), 0);
        assert_eq!(
            FEEL_V1.tables(TABLES_V1).hitstop_ticks,
            TABLES_V1.hitstop_ticks
        );
    }

    #[test]
    fn pq2_balance_holds_for_v1() {
        let preset = FeelPreset::v1();
        let report = pq2_balance_for(&preset, &TABLES_V1).expect("v1 surveys");
        assert_eq!(
            u32::from(report.wins_a) + u32::from(report.wins_b) + u32::from(report.draws),
            crate::feel::BALANCE_BOUTS
        );
        assert!(report.is_balanced(), "v1 feel rows stay in the PQ2 band");
        // Same rows survey identically twice.
        assert_eq!(report, pq2_balance_for(&preset, &TABLES_V1).unwrap());
    }
}
