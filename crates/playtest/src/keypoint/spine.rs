//! Spine track of the keypoint verifier (ARC8, ARC11).
//!
//! A `"track":"spine"` spec plays its whole script from tick 0 through the
//! spine step (`engine_arena::spine_step`) and asserts over the spine columns
//! the step leaves behind: vitals (health, stamina, state bits) and the
//! transform. The spine has no fork, so setup and window inputs merge into
//! one tape and the prefix is simply the first part of the run; every
//! `(tick, slot)` is scripted, so brains are never consulted.
//!
//! State bits are the `Vitals` byte of the spine: 1 attacking, 2 blocking,
//! 4 dodging, 8 down, 16 out. `SPINE_RULES` maps each rule the spine's
//! `Rules` and `Motion` systems own to the specs that exercise it.

use engine_arena::spine_step::SpineMatch;
use engine_arena::world::{MatchWorld, Obstacle};
use engine_core::checked::{saturating_f64_to_u64, saturating_i64_to_i32};

use super::*;

/// One rule of the spine's `Rules` or `Motion` system and the specs that
/// pin it. The `Inputs`, `Ai` and `Extract` systems are plumbing: the
/// parity harness (`engine_arena::parity`) holds them to the legacy step.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SpineRule {
    /// Stable rule id, `system.rule`.
    pub id: &'static str,
    /// Spine system that owns the rule: `Rules` or `Motion`.
    pub system: &'static str,
    /// Spec ids that exercise the rule; at least one must pass for the rule
    /// to count as covered.
    pub specs: &'static [&'static str],
}

/// The rule-to-spec map ARC11 holds at 100 percent: every rule the spine's
/// `Rules` and `Motion` systems own has a passing spine spec.
pub const SPINE_RULES: [SpineRule; 11] = [
    SpineRule {
        id: "rules.hit",
        system: "Rules",
        specs: &["spine_undefended_hit", "spine_react_then_counter"],
    },
    SpineRule {
        id: "rules.block-chip",
        system: "Rules",
        specs: &["spine_late_block_chips"],
    },
    SpineRule {
        id: "rules.parry",
        system: "Rules",
        specs: &["spine_parry_first_edge", "spine_parry_last_edge"],
    },
    SpineRule {
        id: "rules.dodge-iframes",
        system: "Rules",
        specs: &["spine_dodge_negates"],
    },
    SpineRule {
        id: "rules.hit-stop",
        system: "Rules",
        specs: &["spine_hitstop_swallows_press"],
    },
    SpineRule {
        id: "rules.react-lock",
        system: "Rules",
        specs: &["spine_react_then_counter"],
    },
    SpineRule {
        id: "rules.stamina",
        system: "Rules",
        specs: &["spine_rules_stamina_regen"],
    },
    SpineRule {
        id: "rules.knockout",
        system: "Rules",
        specs: &["spine_kill_confirm"],
    },
    SpineRule {
        id: "motion.arena-clamp",
        system: "Motion",
        specs: &["spine_wall_clamp"],
    },
    SpineRule {
        id: "motion.held-pose",
        system: "Motion",
        specs: &["spine_out_holds_pose"],
    },
    SpineRule {
        id: "motion.obstacle",
        system: "Motion",
        specs: &["spine_rules_wall_blocks"],
    },
];

/// One slot's spine columns after a step.
#[derive(Clone, Copy)]
struct Frame {
    hp: u16,
    stamina: u16,
    state: u8,
    pos: [f32; 2],
}

/// Every slot's columns right now.
fn frame_of(sim: &SpineMatch) -> Vec<Frame> {
    let spine = sim.spine();
    spine
        .vitals()
        .iter()
        .zip(spine.transforms())
        .map(|(vitals, transform)| Frame {
            hp: vitals.hp_permille,
            stamina: vitals.stamina,
            state: vitals.state,
            pos: [transform.pos[0], transform.pos[2]],
        })
        .collect()
}

/// Whole millimeters of a meter value (same-machine scope, like the capture
/// raster).
fn mm(meters: f32) -> i32 {
    let magnitude = saturating_f64_to_u64((f64::from(meters) * 1000.0).abs().round());
    let signed = i64::try_from(magnitude).unwrap_or(i64::MAX);
    saturating_i64_to_i32(if meters < 0.0 { -signed } else { signed })
}

/// The frames of state `tick` (after `tick` steps); empty past the run.
fn at(frames: &[Vec<Frame>], tick: u32) -> &[Frame] {
    usize::try_from(tick)
        .ok()
        .and_then(|index| frames.get(index))
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// Run the merged tape through the spine for `prefix + window` steps and
/// check the spec over the recorded columns.
pub(super) fn run(
    spec: &KeypointSpec,
    config: &ArenaConfig,
    tape: &FighterTape,
    prefix: u32,
    window: u32,
) -> Result<Evidence, KeypointError> {
    let refused = || KeypointError::ArenaRefused(spec.id.clone());
    let walls = spec
        .walls
        .iter()
        .map(|w| Obstacle::aligned([w[0], w[1]], [w[2], w[3]]))
        .collect();
    let world = MatchWorld::try_new(walls).map_err(|_| refused())?;
    let mut sim = SpineMatch::build(config, tape, world).ok_or_else(refused)?;
    let end = prefix + window;
    let mut frames = vec![frame_of(&sim)];
    while sim.tick() < end {
        sim.step().ok_or_else(refused)?;
        frames.push(frame_of(&sim));
    }
    let failures = check(spec, prefix, window, &frames);
    let settle = frames.last().map(Vec::as_slice).unwrap_or_default();
    let dots: Vec<(u8, [f32; 3], u16)> = (0u8..)
        .zip(settle)
        .map(|(slot, frame)| (slot, [frame.pos[0], 0.0, frame.pos[1]], frame.hp))
        .collect();
    let mut digester = spec_digester(spec);
    for frame in frames.iter().flatten() {
        digester.u16(frame.hp);
        digester.u16(frame.stamina);
        digester.u8(frame.state);
        digester.bytes(&mm(frame.pos[0]).to_le_bytes());
        digester.bytes(&mm(frame.pos[1]).to_le_bytes());
    }
    Ok(Evidence {
        failures,
        pixels: capture_rgba(&dots, spec.half_extent_m, CAPTURE_EDGE),
        digester,
    })
}

/// `expected … observed …` lines for every assert the columns do not meet.
/// Window-relative tick `t` is the step from state `prefix + t` to
/// `prefix + t + 1`, the same clock the fork track's recorder uses.
fn check(spec: &KeypointSpec, prefix: u32, window: u32, frames: &[Vec<Frame>]) -> Vec<String> {
    let asserts = &spec.asserts;
    let settle = at(frames, prefix + window);
    let mut failures = Vec::new();
    let mut compare =
        |what: &str, slot: u8, expected: String, observed: Option<String>| match observed {
            Some(seen) if seen == expected => {}
            Some(seen) => failures.push(format!(
                "{what} fighter {slot}: expected {expected} observed {seen}"
            )),
            None => failures.push(format!("{what} fighter {slot}: no column")),
        };
    for &(slot, want) in &asserts.hp {
        let seen = settle.get(usize::from(slot)).map(|f| f.hp.to_string());
        compare("hp", slot, want.to_string(), seen);
    }
    for &(slot, want) in &asserts.state {
        let seen = settle.get(usize::from(slot)).map(|f| f.state.to_string());
        compare("state", slot, want.to_string(), seen);
    }
    for &(slot, want) in &asserts.stamina {
        let seen = settle.get(usize::from(slot)).map(|f| f.stamina.to_string());
        compare("stamina", slot, want.to_string(), seen);
    }
    for &(slot, want) in &asserts.pos_mm {
        let seen = settle
            .get(usize::from(slot))
            .map(|f| format!("{:?}", [mm(f.pos[0]), mm(f.pos[1])]));
        compare("pos_mm", slot, format!("{want:?}"), seen);
    }
    let mut damage_ticks = Vec::new();
    let mut first_kill = None;
    for tick in 0..window {
        let (before, after) = (at(frames, prefix + tick), at(frames, prefix + tick + 1));
        let pairs = || before.iter().zip(after);
        if pairs().any(|(b, a)| a.hp < b.hp) {
            damage_ticks.push(tick);
        }
        if first_kill.is_none() && pairs().any(|(b, a)| b.hp > 0 && a.hp == 0) {
            first_kill = Some(tick + 1);
        }
    }
    if let Some(expected) = asserts.killed {
        let killed = first_kill.is_some();
        if killed != expected {
            failures.push(format!("killed: expected {expected} observed {killed}"));
        }
    }
    if let Some(expected) = &asserts.damage_ticks {
        if &damage_ticks != expected {
            failures.push(format!(
                "damage_ticks: expected {expected:?} observed {damage_ticks:?}"
            ));
        }
    }
    if let Some(expected) = asserts.ticks_to_kill {
        if first_kill != expected {
            failures.push(format!(
                "ticks_to_kill: expected {expected:?} observed {first_kill:?}"
            ));
        }
    }
    failures.sort();
    failures
}
