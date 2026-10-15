//! P2 feel latency over the live combat step (sim side only).
//!
//! Scripts seeded presses through `try_step_v2_with_cues_into` and records the
//! tick each effect leaves the sim: the press cue, hit-stop start, the impact
//! sound event and the camera kick. Checks: the impact trio shares one tick,
//! the first-effect p95 is at most 3 ticks (50 ms at 60 Hz), and a seeded
//! 4-tick delay makes that check fail. The present leg (event to frame) needs
//! a GPU and stays in `engine_perf::latency`. Fixed seed 2040, integer ticks,
//! no wall clock, nothing written back to the sim.

use engine_combat::feel::JUICE_TRAUMA_PEAK_PERMILLE;
use engine_combat::feel_instrument::camera_kick_for;
use engine_combat::fight::{
    try_step_v2_with_cues_into, AttackStarted, FightEvent, FightState, StrikePose, Tables,
    MAX_ATTACK_STARTED_PER_TICK, MAX_EVENTS_PER_TICK, TABLES_V1, TABLES_V2,
};
use engine_combat::sweep::{SweepParticipant, WeaponSweep};
use engine_core::splitmix64;
use engine_input::fighter::{ATTACK, BLOCK, DODGE};
use engine_input::{Buttons, FighterId, FighterInput};

/// Fixed seed for the scripted input set (the P2 rows name seed 2040).
const SEED: u64 = 2040;
/// Scripted bouts per table set; the P2 bar asks for at least 200.
const INPUTS: u64 = 240;
/// Ticks per scripted bout: idle lead-in, startup, impact and freeze.
const HORIZON: u32 = 40;
/// Idle ticks before the first press, so a held block cannot cover it.
const LEAD_IN: u32 = 8;
/// The press lands on `LEAD_IN + (seed % PRESS_SPREAD)`.
const PRESS_SPREAD: u64 = 8;
/// Sim clock in Hz.
const TICK_HZ: u32 = 60;
/// P2 bar: input to first simulated effect p95 at most 50 ms.
const GATE_MS: u32 = 50;
/// The same bar in whole ticks: 3 ticks at 60 Hz.
const GATE_TICKS: u32 = GATE_MS * TICK_HZ / 1000;
/// Delay the seeded fault adds to every emitted effect, in ticks.
const SEEDED_FAULT_TICKS: u32 = 4;

/// Scripted defence per bout; the attacker always presses attack once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Victim idle: a landed hit.
    Hit,
    /// Victim blocks 3 ticks before impact: a parry.
    Parry,
    /// Victim holds block from the start: a chip block.
    Block,
    /// Victim dodges 2 ticks before impact: an evasion.
    Dodge,
}

const KINDS: [Kind; 4] = [Kind::Hit, Kind::Parry, Kind::Block, Kind::Dodge];

/// What one scripted bout emitted, in ticks of the bout clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Sample {
    kind: Kind,
    /// Tick the attack button was pressed (the input tick).
    press: u32,
    /// First press cue for slot 0 (readable-timing rows only).
    press_cue: Option<u32>,
    /// First impact-class event (the sound cue source).
    impact: Option<u32>,
    /// The impact event itself, for the outcome check.
    outcome: Option<FightEvent>,
    /// First tick hit-stop rose from zero on a Hit or Parried.
    hitstop: Option<u32>,
    /// First tick with a non-zero camera kick.
    camera: Option<u32>,
}

impl Sample {
    /// Earliest emitted effect of any kind. The press cue counts here, but
    /// `crates/audio/src/crowd.rs` maps it to `Reaction::Quiet` (no sound), so
    /// it is a sim tell, not a perceivable effect; see the OPEN test below.
    fn first_effect(&self) -> Option<u32> {
        [self.press_cue, self.impact, self.hitstop, self.camera]
            .into_iter()
            .flatten()
            .min()
    }
}

/// Hurt capsule and blade for one slot, as in the live duel's strike pose.
fn pose(slot: u8, x: i32, z: i32, facing: (i32, i32), team: u8) -> StrikePose {
    let (sx, sz) = (facing.0.signum(), facing.1.signum());
    StrikePose {
        body: SweepParticipant {
            id: slot,
            pos_mm: [x, 0, z],
            facing_x: facing.0,
            facing_z: facing.1,
            hurt_half_height_mm: 800,
            hurt_radius_mm: 150,
            shield_half_deg: 60,
            shield_up: false,
        },
        blade: WeaponSweep {
            attacker: slot,
            p0_mm: [x + sx * 200, 0, z + sz * 200],
            p1_mm: [x + sx * 1500, 0, z + sz * 1500],
            blade_half_mm: 10,
        },
        team,
    }
}

fn input(fighter: u8, buttons: u8, tick: u32) -> FighterInput {
    FighterInput {
        tick,
        fighter: FighterId::new(fighter).expect("slot 0 or 1"),
        mx: 0,
        mz: 0,
        facing: 0,
        buttons: Buttons(buttons),
    }
}

/// Attacker presses attack on its press tick and nothing else.
fn attacker_buttons(t: u32, press: u32) -> u8 {
    if t == press {
        ATTACK
    } else {
        0
    }
}

/// Victim script per kind. Impact is `press + startup` under the rows in use;
/// the outcome check below confirms the script really hit its target.
fn victim_buttons(kind: Kind, press: u32, startup: u32, t: u32) -> u8 {
    let impact = press + startup;
    match kind {
        Kind::Hit => 0,
        Kind::Parry => {
            if t + 3 >= impact && t <= impact {
                BLOCK
            } else {
                0
            }
        }
        Kind::Block => BLOCK,
        Kind::Dodge => {
            if t + 2 == impact {
                DODGE
            } else {
                0
            }
        }
    }
}

/// Step one scripted bout and record the tick of each effect. `delay` is the
/// seeded fault: it moves every emitted effect later by that many ticks.
fn run_bout(kind: Kind, press: u32, tables: &Tables, delay: u32) -> Sample {
    let poses = [pose(0, 0, 0, (1000, 0), 0), pose(1, 1_000, 0, (-1_000, 0), 1)];
    let mut state = FightState::new(2).expect("two fighters");
    let startup = tables.attack_startup_ticks;
    let mut out: Vec<FightEvent> = Vec::with_capacity(MAX_EVENTS_PER_TICK);
    let mut cues: Vec<AttackStarted> = Vec::with_capacity(MAX_ATTACK_STARTED_PER_TICK);
    let mut sample = Sample {
        kind,
        press,
        press_cue: None,
        impact: None,
        outcome: None,
        hitstop: None,
        camera: None,
    };
    for t in 0..HORIZON {
        let at = t + delay;
        let inputs = [
            input(0, attacker_buttons(t, press), t),
            input(1, victim_buttons(kind, press, startup, t), t),
        ];
        let before = state.hitstop;
        try_step_v2_with_cues_into(&inputs, &poses, &mut state, tables, &mut out, &mut cues)
            .expect("scripted inputs are valid");
        if sample.press_cue.is_none() && cues.iter().any(|c| c.attacker == 0) {
            sample.press_cue = Some(at);
        }
        let mut kick = 0u16;
        for event in out.iter() {
            let (damage, parries) = match *event {
                FightEvent::Hit { damage, .. } => (damage, 0),
                FightEvent::Blocked { chip, .. } => (chip, 0),
                FightEvent::Parried { .. } => (0, 1),
                _ => (0, 0),
            };
            kick = kick.max(camera_kick_for(
                damage,
                parries,
                JUICE_TRAUMA_PEAK_PERMILLE,
                true,
            ));
        }
        if sample.impact.is_none() {
            if let Some(event) = out.iter().find(|e| {
                matches!(
                    e,
                    FightEvent::Hit { .. }
                        | FightEvent::Blocked { .. }
                        | FightEvent::Parried { .. }
                        | FightEvent::Dodged { .. }
                )
            }) {
                sample.impact = Some(at);
                sample.outcome = Some(*event);
            }
        }
        let freezes = out
            .iter()
            .any(|e| matches!(e, FightEvent::Hit { .. } | FightEvent::Parried { .. }));
        if sample.hitstop.is_none() && freezes && state.hitstop > before {
            sample.hitstop = Some(at);
        }
        if sample.camera.is_none() && kick > 0 {
            sample.camera = Some(at);
        }
    }
    sample
}

/// Run the seeded suite: `INPUTS` bouts, kind and press tick from `splitmix64`.
fn run_suite(tables: &Tables, delay: u32) -> Vec<Sample> {
    let mut samples = Vec::with_capacity(INPUTS as usize);
    for i in 0..INPUTS {
        let r = splitmix64(SEED.wrapping_add(i));
        let kind = KINDS[(r % 4) as usize];
        let press = LEAD_IN + ((r >> 8) % PRESS_SPREAD) as u32;
        samples.push(run_bout(kind, press, tables, delay));
    }
    samples
}

/// Nearest-rank p95 over the samples (1-based rank `ceil(0.95 n)`).
fn p95(values: &mut [u32]) -> u32 {
    values.sort_unstable();
    let rank = (values.len() * 95).div_ceil(100);
    values[rank - 1]
}

/// Input to first simulated effect, in ticks, one value per bout.
fn first_effect_ticks(samples: &[Sample]) -> Vec<u32> {
    samples
        .iter()
        .map(|s| s.first_effect().expect("every bout emits an effect") - s.press)
        .collect()
}

/// Input to impact-class event, in ticks, one value per bout.
fn impact_ticks(samples: &[Sample]) -> Vec<u32> {
    samples
        .iter()
        .map(|s| s.impact.expect("every bout has an impact") - s.press)
        .collect()
}

/// The P2 check: first-effect p95 at or under the bar, else an error text.
fn gate_check(samples: &[Sample]) -> Result<u32, String> {
    let mut lat = first_effect_ticks(samples);
    let p = p95(&mut lat);
    if p <= GATE_TICKS {
        Ok(p)
    } else {
        Err(format!(
            "first-effect p95 {p} ticks ({} ms) is over the {GATE_TICKS}-tick ({GATE_MS} ms) bar",
            ticks_to_ms(p)
        ))
    }
}

fn ticks_to_ms(ticks: u32) -> u32 {
    ticks * 1000 / TICK_HZ
}

/// The impact trio (hit-stop, sound event, camera kick) on one tick, per kind.
fn same_tick(s: &Sample) -> bool {
    let Some(impact) = s.impact else {
        return false;
    };
    match s.kind {
        Kind::Hit | Kind::Parry => s.hitstop == Some(impact) && s.camera == Some(impact),
        Kind::Block => s.hitstop.is_none() && s.camera == Some(impact),
        Kind::Dodge => s.hitstop.is_none() && s.camera.is_none(),
    }
}

fn outcome_matches(s: &Sample) -> bool {
    matches!(
        (s.kind, s.outcome),
        (Kind::Hit, Some(FightEvent::Hit { .. }))
            | (Kind::Parry, Some(FightEvent::Parried { .. }))
            | (Kind::Block, Some(FightEvent::Blocked { .. }))
            | (Kind::Dodge, Some(FightEvent::Dodged { .. }))
    )
}

#[test]
fn p2_scripted_bouts_hit_their_scripted_outcome() {
    for tables in [&TABLES_V2, &TABLES_V1] {
        let samples = run_suite(tables, 0);
        assert!(samples.len() >= 200, "need at least 200 scripted inputs");
        let misses = samples.iter().filter(|s| !outcome_matches(s)).count();
        assert_eq!(misses, 0, "scripted defence missed its outcome");
    }
}

#[test]
fn p2_impact_trio_shares_one_tick() {
    for tables in [&TABLES_V2, &TABLES_V1] {
        let samples = run_suite(tables, 0);
        let bad = samples.iter().filter(|s| !same_tick(s)).count();
        assert_eq!(bad, 0, "hit-stop, sound and camera kick must share a tick");
    }
}

/// Checks the check: the press tell (quiet, see `Sample::first_effect`) is
/// inside the bar on readable rows, so the seeded-fault test below has a clean
/// baseline. This is NOT the P2 feel pass: the perceivable effects are late
/// (OPEN test below).
#[test]
fn p2_press_tell_p95_is_inside_the_bar_but_quiet() {
    let samples = run_suite(&TABLES_V2, 0);
    let presses = samples.iter().filter(|s| s.press_cue.is_some()).count();
    assert_eq!(presses, samples.len(), "every press cues on its press tick");
    let p = gate_check(&samples).expect("clean run must meet the bar");
    let mut impacts = impact_ticks(&samples);
    let impact_p95 = p95(&mut impacts);
    println!(
        "P2 sim v2: press-tell p95 {p} ticks ({} ms, quiet) over {} bouts; perceivable impact p95 {impact_p95} ticks ({} ms)",
        ticks_to_ms(p),
        samples.len(),
        ticks_to_ms(impact_p95)
    );
}

#[test]
fn p2_seeded_four_tick_delay_fails_the_check() {
    let samples = run_suite(&TABLES_V2, SEEDED_FAULT_TICKS);
    let err = gate_check(&samples).expect_err("a 4-tick delay must fail the bar");
    assert!(err.contains("over the 3-tick"), "unexpected message: {err}");
    println!("P2 seeded fault: {err}");
}

#[test]
fn p2_suite_replays_identically_twice() {
    let first = run_suite(&TABLES_V2, 0);
    let second = run_suite(&TABLES_V2, 0);
    assert_eq!(first, second);
}

/// OPEN, not a pass. Hit effects (hit-stop, sound, camera) leave the sim at
/// the startup row: 4 ticks (67 ms) on the shipped V1 rows and 15 ticks
/// (250 ms) on the readable V2 rows, both over the 3-tick bar. This pins the
/// measured value so any change shows up here; when the impact moves earlier,
/// update the pin with the gate.
#[test]
fn p2_open_impact_latency_is_pinned_over_the_bar() {
    let mut v2 = impact_ticks(&run_suite(&TABLES_V2, 0));
    let mut v1 = impact_ticks(&run_suite(&TABLES_V1, 0));
    let v2_p95 = p95(&mut v2);
    let v1_p95 = p95(&mut v1);
    println!(
        "P2 OPEN impact p95: V2 {v2_p95} ticks ({} ms), V1 {v1_p95} ticks ({} ms), bar {GATE_TICKS} ticks",
        ticks_to_ms(v2_p95),
        ticks_to_ms(v1_p95)
    );
    assert_eq!(v2_p95, TABLES_V2.attack_startup_ticks);
    assert_eq!(v1_p95, TABLES_V1.attack_startup_ticks);
    assert!(v2_p95 > GATE_TICKS && v1_p95 > GATE_TICKS);
}
