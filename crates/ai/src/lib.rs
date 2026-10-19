//! ai: embedded seam (brief -> patch -> run). Typed, no strings; AI never touches sim memory.
//! Data owner: this seam owns [`Brief`] plus [`Op`] application over the
//! archetype [`World`](engine_ecs::archetype::World); ticks run through
//! [`Schedule::run_arena_tick`](engine_ecs::schedule::Schedule::run_arena_tick)
//! with per-phase scratch (the arena tick path, see [`engine_ecs::WORLD_FORK_DECISION`]).

use engine_core::hash::FNV64_PRIME;
use engine_ecs::archetype::{Position, Velocity, World};
use engine_physics::{Body, Space};

// Unit tests prove zero-allocation paths with the shared counting probe.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOC: engine_core::alloc_probe::CountingAlloc =
    engine_core::alloc_probe::CountingAlloc;

mod cursor;
pub mod utility;
pub use utility::{pick, pick_gated, response_curve, Action, Gate, Gated, Scorer};
pub mod blackboard;
pub use blackboard::{goap_plan, Blackboard, TaskStatus};
pub mod behavior;
pub use behavior::{tick, Behavior, Status};
/// Deterministic bouts between two trees; records tapes and reports.
pub mod bout;
/// Versioned integer trees for bout bots; the bout harness ticks these.
pub mod bt;
pub mod director;
/// Duel enemy tiers T1-T3: shipped rows, the win bar and the skill score.
pub mod tier;
pub use director::{pick_spawn, Budget, SpawnRow};
pub mod curves;
pub use curves::{
    validate_config, validate_curves, AiConfig, AiCurveError, AiCurves, BIAS_MAX, BIAS_NEUTRAL,
    CURVE_LANES, DEFAULT_AGGRESSION, DEFAULT_CAUTION, DEFAULT_CURVES_VERSION,
    DEFAULT_ENGAGE_RANGE_CM, DEFAULT_MORALE, ENGAGE_RANGE_MAX_CM, ENGAGE_RANGE_MIN_CM,
};
/// Designer rows text for [`AiConfig`]: parse plus row errors. The scripted
/// [`Brain`](crate::gladiator::Brain) builds from rows via `with_rows_text`.
pub mod curves_text;
pub mod gladiator;
pub mod goalstack;
/// GOAP planner: A* over a 64-fact integer world state.
pub mod goap;
/// One shared scanner for designer `key=value` rows: comment strip plus field
/// walk, so depth tier rows and ai curves rows refuse the same shapes.
pub mod rows;
pub use gladiator::{
    compare, compare_curves, features_from, pick_intent, Brain, Comparison, Intent, MarchPolicy,
    NetPolicy, NetPolicyError, Percept, Policy, TickPolicy, FEATURE_ATTACK_ON, FEATURE_DIM,
    FEATURE_FULL_DIST_M, FEATURE_FULL_GROUND_M, FEATURE_FULL_HEIGHT_M, INTENT_OUT_DIM, LANE_DIST,
    LANE_DX, LANE_DY, LANE_DZ, LANE_FOE_ATTACK, LANE_YAW,
};
pub mod melee;
pub use melee::{FoeView, MeleeView, StrikePhase, NEAR_FOES, WAIT_RING_MM};
pub mod personality;
pub mod reaction_delay;
pub use personality::Personality;
pub mod guard;
pub use guard::{ExecutionGuard, Guard, InputGuard, TopicalGuard, Validator, Verdict};
pub mod legion;
pub mod memory;
pub use memory::{
    MemoryStore, Recall, RecallHit, Salience, StoredMemory, MAX_NPCS, MAX_RECALL,
    MAX_STORED_PER_NPC, MEMORY_BYTES_VERSION,
};
pub mod mind;
pub use mind::{
    backend_rows, validate_proposal as validate_mind_proposal, ActionDraft, ActionKind, ActionLog,
    ActionSource, AdapterOutcome, AskOutcome, AskStatus, BackendRow, Goal, GoalState, LateOutcome,
    LocalAdapter, MemoryEvent, MemoryKind, Mind, MindAction, MindAdapter, MindContext, MindError,
    MindRequest, MindRuntime, MockAdapter, NpcId, Privacy, Proposal as MindProposal, Relation,
    RemoteAdapter, ScriptStep, CTX_GOALS, CTX_MEMORIES, LOCAL_LATENCY_MS, MAX_ACTIONS_PER_ASK,
    MAX_EVENTS, MAX_GOALS, MAX_RELATIONS, MAX_TEXT, MOCK_LATENCY_MS, REMOTE_COST,
    REMOTE_LATENCY_MS,
};
pub mod depth;
pub use depth::{
    crowd_volume, morale_step, spawn_due, stamina_cost, stamina_gate_bias, tension_row,
    validate_tension_row, ArenaPolicy, DepthBrain, DepthError, FeintState, Ladder, LadderProgress,
    Phase, Role, Squad, SquadN, TensionRow, DRIFT_MAX, DRIFT_MIN, LADDER_RUNGS,
    MAX_ATTACKERS_PER_TARGET, MAX_RUNG, SPAWN_MAX_TICKS, SPAWN_MIN_TICKS, STAMINA_COLLAPSE_AT,
    STAMINA_MAX, STAMINA_MIN, TENSION_LADDER, VOLUME_MAX_PERMILLE, VOLUME_MIN_PERMILLE,
};

pub mod alert;
pub mod apply;
pub mod dialogue;
pub mod eqs;
mod eqs_slice;
pub mod hearing;
pub mod provenance;
/// Reflection cadence and capped-surprise importance.
pub mod reflect;

#[derive(Debug)]
pub struct Brief {
    pub tick: u64,
    pub entities: usize,
    pub bodies: usize,
    pub hash: u64,
}

/// Snapshot the archetype world plus physics space into a comparable brief.
/// Units: ticks, entity counts, FNV-1a64 hashes. Determinism: slot order
/// only; no wall clock. Errors: none (read-only over live state).
pub fn brief(tick: u64, world: &World, space: &Space) -> Brief {
    Brief {
        tick,
        entities: world.count(),
        bodies: space.bodies.len(),
        hash: world.state_hash() ^ space.state_hash().wrapping_mul(FNV64_PRIME),
    }
}

#[derive(Debug)]
pub enum Op {
    Spawn { x: f32, y: f32, vx: f32, vy: f32 },
    Kick { id: usize, vx: f32, vy: f32 },
}

/// Apply ops onto the archetype world plus physics space.
/// Unknown ids are skipped, never panic — AI sends garbage. Kicks retarget
/// physics bodies only (the legacy seam never mirrored them into ECS
/// velocities, so the brief coverage is unchanged).
/// Units: meters, meters/second. Determinism: op order; no wall clock.
/// Errors: none (invalid ids are skipped, never fatal).
pub fn patch(world: &mut World, space: &mut Space, ops: &[Op]) -> usize {
    let mut applied = 0;
    for op in ops {
        match *op {
            Op::Spawn { x, y, vx, vy } => {
                world.spawn(Some(Position { x, y }), Some(Velocity { x: vx, y: vy }));
                space.add(Body {
                    x,
                    y,
                    vx,
                    vy,
                    hx: 0.5,
                    hy: 0.5,
                });
                applied += 1;
            }
            Op::Kick { id, vx, vy } => {
                if id < space.bodies.len() {
                    space.bodies[id].vx = vx;
                    space.bodies[id].vy = vy;
                    applied += 1;
                }
            }
        }
    }
    applied
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_ecs::archetype::{Position, Velocity, World};
    use engine_ecs::schedule::{Phase, Schedule};
    use engine_ecs::PhaseArena;
    use engine_physics::{Body, Space};

    fn seeded_pair() -> (World, Space) {
        let mut world = World::new();
        world.reserve(16);
        let mut space = Space::new();
        for (x, y) in [(1.0f32, 2.0), (3.0, 4.0), (5.0, 6.0)] {
            world.spawn(Some(Position { x, y }), Some(Velocity { x: 0.5, y: -0.5 }));
            space.add(Body {
                x,
                y,
                vx: 0.5,
                vy: -0.5,
                hx: 0.5,
                hy: 0.5,
            });
        }
        (world, space)
    }

    #[test]
    fn same_seed_same_brief_and_patch() {
        let (w1, s1) = seeded_pair();
        let (w2, s2) = seeded_pair();
        let b1 = brief(7, &w1, &s1);
        let b2 = brief(7, &w2, &s2);
        assert_eq!(
            (b1.tick, b1.entities, b1.bodies, b1.hash),
            (b2.tick, b2.entities, b2.bodies, b2.hash)
        );
        let (mut w1, mut s1) = (w1, s1);
        let (mut w2, mut s2) = (w2, s2);
        let ops = vec![
            Op::Spawn {
                x: 10.0,
                y: 20.0,
                vx: 1.0,
                vy: -1.0,
            },
            Op::Kick {
                id: 0,
                vx: 7.0,
                vy: 8.0,
            },
        ];
        assert_eq!(
            (patch(&mut w1, &mut s1, &ops), patch(&mut w2, &mut s2, &ops)),
            (2, 2)
        );
        let b1 = brief(8, &w1, &s1);
        let b2 = brief(8, &w2, &s2);
        assert_eq!(
            (b1.entities, b1.bodies, b1.hash),
            (b2.entities, b2.bodies, b2.hash)
        );
    }

    #[test]
    fn unknown_op_id_skipped_never_panics() {
        let mut world = World::new();
        let mut space = Space::new();
        let applied = patch(
            &mut world,
            &mut space,
            &[
                Op::Kick {
                    id: 0,
                    vx: 1.0,
                    vy: 1.0,
                },
                Op::Kick {
                    id: 999,
                    vx: 1.0,
                    vy: 1.0,
                },
                Op::Kick {
                    id: usize::MAX,
                    vx: 1.0,
                    vy: 1.0,
                },
            ],
        );
        assert_eq!((applied, world.count()), (0, 0));
        assert!(space.bodies.is_empty());
        let (mut world, mut space) = seeded_pair();
        assert_eq!(
            patch(
                &mut world,
                &mut space,
                &[Op::Kick {
                    id: 3,
                    vx: 9.0,
                    vy: 9.0
                }]
            ),
            0
        );
        assert_eq!(
            patch(
                &mut world,
                &mut space,
                &[Op::Kick {
                    id: 1,
                    vx: 3.25,
                    vy: -4.5
                }]
            ),
            1
        );
        assert_eq!((space.bodies[1].vx, space.bodies[1].vy), (3.25, -4.5));
    }

    #[test]
    fn hash_sensitive_to_state_change() {
        let (mut world, mut space) = seeded_pair();
        let h0 = brief(0, &world, &space).hash;
        assert_eq!(
            patch(
                &mut world,
                &mut space,
                &[Op::Spawn {
                    x: 42.0,
                    y: 43.0,
                    vx: 0.0,
                    vy: 0.0
                }]
            ),
            1
        );
        let b1 = brief(0, &world, &space);
        assert_eq!((b1.entities, b1.bodies), (4, 4));
        assert_ne!(b1.hash, h0);
        assert_eq!(
            patch(
                &mut world,
                &mut space,
                &[Op::Kick {
                    id: 0,
                    vx: -11.0,
                    vy: 12.0
                }]
            ),
            1
        );
        let b2 = brief(0, &world, &space);
        assert_ne!(b2.hash, b1.hash);
    }

    #[test]
    fn patched_world_ticks_through_run_arena_tick() {
        // The migrated seam ticks through the arena path, not the legacy
        // direct integrate. The checkpoint equals the direct hash, and
        // movement actually moves the brief.
        let (mut world, mut space) = seeded_pair();
        world.reserve(16);
        let mut arena = PhaseArena::with_capacity(16, 512);
        arena.reserve(16, 512);
        let mut schedule: Schedule<World> = Schedule::new();
        schedule.add_system(Phase::Update, "integrate", |world, ctx| {
            world.integrate(ctx.dt);
        });
        let before = brief(0, &world, &space);
        let checkpoint = schedule
            .run_arena_tick(&mut world, &mut arena, 1.0 / 60.0)
            .expect("presized arena fits the 3-entity tick");
        space.step(1.0 / 60.0);
        assert_eq!(checkpoint, world.state_hash());
        let after = brief(1, &world, &space);
        assert_eq!((after.entities, after.bodies), (3, 3));
        assert_ne!(after.hash, before.hash);
        // A refused tick keeps its staged state: an empty scratch cannot
        // even census the three live entities.
        let mut tiny = PhaseArena::new();
        let mut refusing: Schedule<World> = Schedule::new();
        refusing.add_system(Phase::Update, "integrate", |world, ctx| {
            world.integrate(ctx.dt);
        });
        assert!(matches!(
            refusing.run_arena_tick(&mut world, &mut tiny, 1.0 / 60.0),
            Err(engine_ecs::ArenaError::Overflow { .. })
        ));
    }
}
