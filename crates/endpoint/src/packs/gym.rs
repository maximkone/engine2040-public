//! `gym` pack: headless gym verbs over the deterministic arena.
//!
//! Consumer: AI training loops through the MCP door (`sim.reset`, then
//! `sim.step`, then `sim.observe`). Data owner for simulation is
//! `engine-arena`; this module owns only the verb DTOs, validation and the
//! replay digests. Units: meters for extents and positions, radians for yaw,
//! fixed ticks, permille for health, stamina points, `u64` digests and seeds
//! (decimal strings on the wire, a number accepted on input). Determinism:
//! the same seed plus tape twice seals the same digest, the same numbers and
//! the same events; no wall clock, no threads. Errors are typed
//! `McpToolError` codes naming the legal values. Budget: reset is one spawn
//! plus one FNV-1a fold, step simulates at most 3600 ticks (service path,
//! never the fixed tick).
//!
//! The session holds one live bout (`PlaySlot`, shared with `play.step`):
//! `sim.step` names its whole tape prefix, so its answer stays a pure
//! function of its arguments, but a longer prefix of the tape the bout
//! already consumed only simulates the new ticks. `sim.reset` opens a fresh
//! bout at tick 0 and `sim.observe` reads whichever bout was stepped last.

use engine_arena::{initial_state, ArenaConfig, MAX_PARTICIPANTS};
use engine_core::hash::Fnv64;
use rmcp::schemars::JsonSchema;
// `self` puts `serde` itself in scope: the schemars derive names it when a
// field has both `default` and a `serde(with)` module (see `dec_u64`).
use rmcp::serde::{self, Deserialize, Serialize};

use super::{PackDef, PackError, VerbSpec};
use crate::mcp::{McpServer, McpToolError};
use crate::play::play_session::{
    dec_u64, ActorNumbers, BoutEvent, BoutSpec, PlaySlot, EVENT_DEFAULT, EVENT_LOG_CAP,
};
use crate::play::{build_tape, PlayInputDto, MAX_PLAY_INPUTS, MAX_PLAY_TICKS};
use crate::tool_capabilities::{CapabilityCard, CheckKind};

#[path = "gym_observe.rs"]
mod gym_observe;

pub use gym_observe::{SimObserveArgs, SimObserveOutput, SimObserveTool, TOOL_SIM_OBSERVE};

/// Pack name.
pub const PACK: &str = "gym";
/// Verb: reset the headless spar to its tick-zero spawn.
pub const TOOL_SIM_RESET: &str = "sim.reset";
/// Verb: step the headless spar through a tape to its digest.
pub const TOOL_SIM_STEP: &str = "sim.step";

/// Args for `sim.reset`: seed, headcount and extent, all optional with the
/// same defaults as `play.step` (seed 0, 2 fighters, 20 m).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SimResetArgs {
    /// Match seed (a decimal string or a number); the same seed plus config
    /// replays the same digest.
    #[serde(default, with = "dec_u64::opt")]
    #[schemars(with = "Option<String>")]
    pub seed: Option<u64>,
    /// Participant count, 2..=16.
    #[serde(default, alias = "fighters")]
    pub participants: Option<u8>,
    /// Arena half-extent in meters, 5.0..=100.0.
    #[serde(default)]
    pub half_extent_m: Option<f32>,
}

/// One fighter at tick zero: spawn pose plus full health and the RL readout.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SimResetParticipant {
    /// Participant slot, `0..participants`.
    pub id: u8,
    /// Feet position `[x, y, z]` in meters.
    pub pos: [f32; 3],
    /// Facing yaw in radians.
    pub yaw: f32,
    /// Health in permille, always 1000 at reset.
    pub hp: u16,
    /// Immediate survival reward, always `1.0` at reset.
    #[serde(default)]
    pub reward: f32,
    /// Legal discrete actions `[attack, block, dodge]`, all `true` at reset.
    #[serde(default)]
    pub legal_mask: [bool; engine_arena::gym::GYM_NUM_ACTIONS],
    /// Planar velocity `[vx, vy, vz]` in meters per second, zero at reset.
    #[serde(default)]
    pub velocity: [f32; 3],
}

/// `sim.reset` result: tick zero, the spawn digest and the fighters.
///
/// `check` names the capability check that ran (`readonly`); `digest` is the
/// value read back, so the answer carries both without a second call.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SimResetOutput {
    /// Ticks simulated: always 0 at reset.
    pub tick: u32,
    /// Spawn digest: FNV-1a over seed, config and every spawn pose plus
    /// health, so the same reset twice seals the same hash. A decimal string
    /// on the wire.
    #[serde(with = "dec_u64")]
    #[schemars(with = "String")]
    pub digest: u64,
    /// Participant count that was spawned.
    pub participants: u8,
    /// Capability check that ran (`readonly`).
    pub check: String,
    /// Participant snapshots in slot order.
    pub fighters: Vec<SimResetParticipant>,
}

/// Pure reset core: validate, spawn, fold the digest. Writes nothing, so the
/// scene revision never moves and `tx.undo` stays empty behind it.
fn reset_core(args: &SimResetArgs) -> Result<SimResetOutput, McpToolError> {
    let seed = args.seed.unwrap_or(0);
    let participants = args.participants.unwrap_or(2);
    let half = args.half_extent_m.unwrap_or(20.0);
    if !(2..=MAX_PARTICIPANTS).contains(&participants) {
        return Err(McpToolError::new(
            "bad_fighters",
            format!(
                "participants {participants} is outside 2..={MAX_PARTICIPANTS}; \
                 re-run with a headcount in that range"
            ),
        ));
    }
    if !half.is_finite() || !(5.0..=100.0).contains(&half) {
        return Err(McpToolError::new(
            "bad_extent",
            format!("half_extent_m {half} is outside 5.0..=100.0 meters; re-run in range"),
        ));
    }
    let states = match initial_state(seed, participants, half) {
        Some(states) => states,
        None => {
            return Err(McpToolError::new(
                "bad_config",
                format!(
                    "spawn for seed {seed} with {participants} fighters at {half}m holds \
                     nothing; ranges already check 2..=16 and 5.0..=100.0m, so re-run \
                     in range and report the seed"
                ),
            ));
        }
    };
    let mut hasher = Fnv64::new();
    hasher.write_u64(seed);
    hasher.write_u8(participants);
    hasher.write_f32(half);
    let mut fighters = Vec::with_capacity(states.len());
    for state in &states {
        hasher.write_u8(state.id.0);
        for lane in state.pos {
            hasher.write_f32(lane);
        }
        hasher.write_f32(state.yaw);
        hasher.write_u16(state.hp);
        fighters.push(SimResetParticipant {
            id: state.id.0,
            pos: state.pos,
            yaw: state.yaw,
            hp: state.hp,
            reward: state.reward,
            legal_mask: state.legal_mask,
            velocity: state.velocity,
        });
    }
    Ok(SimResetOutput {
        tick: 0,
        digest: hasher.finish(),
        participants,
        check: CheckKind::ReadOnly.name().to_owned(),
        fighters,
    })
}

/// `sim.reset` handler: the pure core under the audited dispatch seam (one
/// audit record, no document write, no journal entry), then a fresh live bout
/// at tick 0 so `sim.observe` reads the spawn right after a reset.
fn sim_reset(service: &McpServer, args: &SimResetArgs) -> Result<SimResetOutput, McpToolError> {
    service.dispatch(TOOL_SIM_RESET, |state| {
        let out = reset_core(args)?;
        let half = args.half_extent_m.unwrap_or(20.0);
        let spec = BoutSpec::new(args.seed.unwrap_or(0), out.participants, half);
        state.play.reset(spec)?;
        Ok(out)
    })
}

crate::e2_tool!(
    /// `sim.reset` tool: tick-zero spawn plus its replay digest. Read-only.
    SimResetTool,
    name = TOOL_SIM_RESET,
    description = "Reset the headless spar to its tick-zero spawn: seed, headcount and extent in, spawn digest plus fighters out. Same inputs replay the same digest; writes nothing.",
    readonly = true,
    args = SimResetArgs,
    output = SimResetOutput,
    invoke = sim_reset
);

/// Args for `sim.step`: seed, headcount, extent, tick count and tape.
///
/// Same defaults as `sim.reset` (seed 0, 2 fighters, 20 m); `ticks` is the
/// 1-indexed count N of the `0..N` prefix (1..=3600, the MCP bound); `inputs`
/// carry 0-indexed tick ids inside `0..ticks`, sorted, no duplicates.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SimStepArgs {
    /// Match seed (a decimal string or a number); the same seed plus tape
    /// replays the same digest.
    #[serde(default, with = "dec_u64::opt")]
    #[schemars(with = "Option<String>")]
    pub seed: Option<u64>,
    /// Participant count, 2..=16.
    #[serde(default, alias = "fighters")]
    pub participants: Option<u8>,
    /// Arena half-extent in meters, 5.0..=100.0.
    #[serde(default)]
    pub half_extent_m: Option<f32>,
    /// Ticks to simulate as the 1-indexed count N (1..=3600).
    #[serde(default)]
    pub ticks: Option<u32>,
    /// Tape inputs with 0-indexed tick ids inside `0..ticks`.
    #[serde(default)]
    pub inputs: Option<Vec<PlayInputDto>>,
    /// How many of the newest combat events to return, 0..=64 (default 16).
    #[serde(default)]
    pub events: Option<u32>,
}

/// `sim.step` result: tick count, the replay digest, the numbers and the
/// newest events.
///
/// `check` names the capability check that ran (`readonly`); `digest` equals
/// the arena `run` final hash for the same config plus tape, so gym replays
/// and `play.run` receipts agree on the same prefix. `fighters` carries the
/// reset fields (`id`, `pos`, `yaw`, `hp`) plus `stamina` and `phase`;
/// `events` lists the newest hits, blocks and kills with their ticks.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SimStepOutput {
    /// Ticks simulated: the 1-indexed count N.
    pub tick: u32,
    /// State digest: the checkpoint `tick - 1` hash, a decimal string on the
    /// wire.
    #[serde(with = "dec_u64")]
    #[schemars(with = "String")]
    pub digest: u64,
    /// Participant count that was stepped.
    pub participants: u8,
    /// Capability check that ran (`readonly`).
    pub check: String,
    /// Participant numbers in slot order: hp, stamina, phase, position, yaw.
    pub fighters: Vec<ActorNumbers>,
    /// The newest combat events, oldest first, each with its tick.
    #[serde(default)]
    pub events: Vec<BoutEvent>,
}

/// Events to return for a request: the caller's count, else the default;
/// refused past the ring size, naming the legal range.
pub(crate) fn event_window(requested: Option<u32>) -> Result<usize, McpToolError> {
    let count = requested.map_or(EVENT_DEFAULT, |count| {
        usize::try_from(count).unwrap_or(usize::MAX)
    });
    if count > EVENT_LOG_CAP {
        return Err(McpToolError::new(
            "bad_events",
            format!(
                "events {count} is outside 0..={EVENT_LOG_CAP} (default {EVENT_DEFAULT}); \
                 re-run with a count in that range"
            ),
        ));
    }
    Ok(count)
}

/// Step core: validate, build the tape, bring the session's live bout to the
/// requested tick and read the numbers. Writes nothing to the scene, so the
/// scene revision never moves and `tx.undo` stays empty behind it.
fn step_core(slot: &mut PlaySlot, args: &SimStepArgs) -> Result<SimStepOutput, McpToolError> {
    let seed = args.seed.unwrap_or(0);
    let participants = args.participants.unwrap_or(2);
    let half = args.half_extent_m.unwrap_or(20.0);
    let ticks = args.ticks.ok_or_else(|| {
        McpToolError::new(
            "bad_ticks",
            format!(
                "ticks is missing; submit 1..={MAX_PLAY_TICKS} (MCP sim.step bound); re-run with a tick count"
            ),
        )
    })?;
    if !(2..=MAX_PARTICIPANTS).contains(&participants) {
        return Err(McpToolError::new(
            "bad_fighters",
            format!(
                "participants {participants} is outside 2..={MAX_PARTICIPANTS}; \
                 re-run with a headcount in that range"
            ),
        ));
    }
    if !half.is_finite() || !(5.0..=100.0).contains(&half) {
        return Err(McpToolError::new(
            "bad_extent",
            format!("half_extent_m {half} is outside 5.0..=100.0 meters; re-run in range"),
        ));
    }
    if ticks == 0 || ticks > MAX_PLAY_TICKS {
        return Err(McpToolError::new(
            "bad_ticks",
            format!(
                "ticks {ticks} is outside 1..={MAX_PLAY_TICKS} (MCP sim.step bound); re-run in range"
            ),
        ));
    }
    let window = event_window(args.events)?;
    let inputs = args.inputs.clone().unwrap_or_default();
    if inputs.len() > MAX_PLAY_INPUTS {
        return Err(McpToolError::new(
            "too_many_inputs",
            format!(
                "inputs holds {} entries; at most {MAX_PLAY_INPUTS} per step; re-run with a shorter tape",
                inputs.len()
            ),
        ));
    }
    let tape = build_tape(&inputs, ticks, participants)?;
    if ArenaConfig::try_new(seed, ticks, participants, half).is_err() {
        return Err(McpToolError::new(
            "bad_config",
            format!(
                "step for seed {seed} with {participants} fighters at {half}m for {ticks} ticks holds \
                 nothing; ranges already check 2..=16, 5.0..=100.0m and 1..={MAX_PLAY_TICKS}, so re-run \
                 in range and report the seed"
            ),
        ));
    }
    let spec = BoutSpec::new(seed, participants, half);
    let view = slot.advance_to(spec, &tape, ticks)?.observe(window);
    Ok(SimStepOutput {
        tick: view.tick,
        digest: view.digest,
        participants,
        check: CheckKind::ReadOnly.name().to_owned(),
        fighters: view.fighters,
        events: view.events,
    })
}

/// `sim.step` handler: the step core under the audited dispatch seam (one
/// audit record, no document write, no journal entry).
fn sim_step(service: &McpServer, args: &SimStepArgs) -> Result<SimStepOutput, McpToolError> {
    service.dispatch(TOOL_SIM_STEP, |state| step_core(&mut state.play, args))
}

crate::e2_tool!(
    /// `sim.step` tool: seed plus tape in, tick count, replay digest, per-fighter numbers and newest events out. Read-only.
    SimStepTool,
    name = TOOL_SIM_STEP,
    description = "Step the headless spar through a tape: seed, headcount, extent, tick count and inputs in; replay digest, per-fighter hp, stamina, phase, position, yaw and the newest hit/block/kill events out. A longer prefix of the same tape only simulates the new ticks. Same inputs replay the same digest; writes nothing.",
    readonly = true,
    args = SimStepArgs,
    output = SimStepOutput,
    invoke = sim_step
);

/// The gym pack: `sim.reset`, `sim.step` and `sim.observe`, loaded on demand.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "headless gym: reset the deterministic spar to tick zero, step it through a tape to a replay digest \
         with per-fighter hp, stamina, phase, position and the newest hit events, and observe the live bout",
        false,
    );
    pack.add(VerbSpec::of::<SimResetTool>(
        TOOL_SIM_RESET,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<SimStepTool>(
        TOOL_SIM_STEP,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<SimObserveTool>(
        TOOL_SIM_OBSERVE,
        PACK,
        CapabilityCard::Observe,
    )?);
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight::mcp_tools::{ApiSearchArgs, ApiSearchTool};
    use crate::mcp::tool_names;
    use crate::mcp::{TxUndoArgs, TxUndoTool};
    use crate::packs::{ApiLoadArgs, ApiLoadTool, PackRegistry};
    use engine_editor::capability::Capability;
    use rmcp::handler::server::router::tool::SyncTool;

    fn server() -> McpServer {
        McpServer::new(7, Capability::unrestricted(7))
    }

    fn reset_args() -> SimResetArgs {
        SimResetArgs {
            seed: Some(2040),
            participants: Some(2),
            half_extent_m: Some(20.0),
        }
    }

    #[test]
    fn sim_reset_replay_same_hash() {
        let server = server();
        let first = SimResetTool::invoke(&server, reset_args()).expect("reset serves");
        let second = SimResetTool::invoke(&server, reset_args()).expect("reset replays");
        assert_eq!(first, second, "same seed plus config seals the same reset");
        assert_eq!(first.tick, 0);
        assert_eq!(first.participants, 2);
        assert_eq!(first.check, CheckKind::ReadOnly.name());
        assert_eq!(first.fighters.len(), 2);
        assert!(
            first.fighters.iter().all(|fighter| fighter.hp == 1000),
            "tick zero spawns at full health"
        );
        assert_ne!(first.digest, 0, "the digest carries the spawn");
        // A different seed must move the digest, or the replay proves nothing.
        let other = SimResetTool::invoke(
            &server,
            SimResetArgs {
                seed: Some(7),
                ..reset_args()
            },
        )
        .expect("other seed serves");
        assert_ne!(
            first.digest, other.digest,
            "seed 2040 and seed 7 must diverge"
        );
        // The registry path answers the same bytes as the direct tool path.
        let registry = PackRegistry::shipped();
        let spec = registry.verb(TOOL_SIM_RESET).expect("verb registers");
        assert_eq!(spec.pack(), PACK);
        assert_ne!(spec.card(), CapabilityCard::Deny);
        let via_registry = spec
            .invoke(
                &server,
                rmcp::serde_json::to_value(reset_args()).expect("args serialize"),
            )
            .expect("registry dispatches");
        let direct = rmcp::serde_json::to_value(first).expect("output serializes");
        assert_eq!(via_registry, direct);
    }

    #[test]
    fn sim_reset_refuses_bad_input_with_legal_values() {
        let server = server();
        for participants in [1, MAX_PARTICIPANTS + 1] {
            let refused = SimResetTool::invoke(
                &server,
                SimResetArgs {
                    participants: Some(participants),
                    ..reset_args()
                },
            )
            .expect_err("headcount outside 2..=16 refuses");
            assert_eq!(refused.code, "bad_fighters");
            assert!(refused.message.contains("2..=16"), "{}", refused.message);
        }
        for half in [4.0, 101.0, f32::NAN, f32::INFINITY] {
            let refused = SimResetTool::invoke(
                &server,
                SimResetArgs {
                    half_extent_m: Some(half),
                    ..reset_args()
                },
            )
            .expect_err("extent outside 5.0..=100.0 refuses");
            assert_eq!(refused.code, "bad_extent");
            assert!(
                refused.message.contains("5.0..=100.0"),
                "{}",
                refused.message
            );
        }
    }

    #[test]
    fn sim_reset_hides_until_loaded_and_search_finds_it() {
        let server = server();
        assert!(
            !crate::mcp::default_tool_names().contains(&TOOL_SIM_RESET),
            "gym waits for api.load; the default list does not grow"
        );
        assert!(
            tool_names().contains(&TOOL_SIM_RESET),
            "the verb stays callable by exact name either way"
        );
        let found = ApiSearchTool::invoke(
            &server,
            ApiSearchArgs {
                query: "sim reset".to_owned(),
                limit: None,
            },
        )
        .expect("search serves");
        let hit = found
            .hits
            .iter()
            .find(|hit| hit.path == TOOL_SIM_RESET)
            .expect("api.search finds sim.reset");
        eprintln!(
            "api.search hit: path={} kind={} score={:.3} brief={}",
            hit.path, hit.kind, hit.score, hit.brief
        );
        let loaded = ApiLoadTool::invoke(
            &server,
            ApiLoadArgs {
                pack: PACK.to_owned(),
                unload: None,
            },
        )
        .expect("gym loads");
        assert!(loaded.verbs.contains(&TOOL_SIM_RESET.to_owned()));
        assert!(server
            .tools()
            .iter()
            .any(|tool| tool.name == TOOL_SIM_RESET));
        let unloaded = ApiLoadTool::invoke(
            &server,
            ApiLoadArgs {
                pack: PACK.to_owned(),
                unload: Some(true),
            },
        )
        .expect("gym unloads");
        assert!(!unloaded.loaded);
        assert!(
            !server
                .tools()
                .iter()
                .any(|tool| tool.name == TOOL_SIM_RESET),
            "unload hides the verb again"
        );
    }

    fn step_args() -> SimStepArgs {
        SimStepArgs {
            seed: Some(2040),
            participants: Some(2),
            half_extent_m: Some(20.0),
            ticks: Some(8),
            inputs: None,
            events: None,
        }
    }

    #[test]
    fn sim_step_replay_same_digest() {
        let server = server();
        let first = SimStepTool::invoke(&server, step_args()).expect("step serves");
        let second = SimStepTool::invoke(&server, step_args()).expect("step replays");
        assert_eq!(first, second, "same seed plus tape seals the same step");
        assert_eq!(first.tick, 8);
        assert_eq!(first.participants, 2);
        assert_eq!(first.check, CheckKind::ReadOnly.name());
        assert_eq!(first.fighters.len(), 2);
        assert_ne!(first.digest, 0, "the digest carries the stepped state");
        let other = SimStepTool::invoke(
            &server,
            SimStepArgs {
                seed: Some(7),
                ..step_args()
            },
        )
        .expect("other seed serves");
        assert_ne!(
            first.digest, other.digest,
            "seed 2040 and seed 7 must diverge"
        );
        let taped = SimStepTool::invoke(
            &server,
            SimStepArgs {
                inputs: Some(vec![crate::play::PlayInputDto {
                    tick: 0,
                    fighter: 0,
                    mx: 100,
                    mz: 0,
                    facing: 0,
                    buttons: 0x01,
                }]),
                ..step_args()
            },
        )
        .expect("taped step serves");
        assert_ne!(
            first.digest, taped.digest,
            "idle and taped prefixes must diverge"
        );
        let registry = PackRegistry::shipped();
        let spec = registry.verb(TOOL_SIM_STEP).expect("verb registers");
        assert_eq!(spec.pack(), PACK);
        assert_ne!(spec.card(), CapabilityCard::Deny);
        let via_registry = spec
            .invoke(
                &server,
                rmcp::serde_json::to_value(step_args()).expect("args serialize"),
            )
            .expect("registry dispatches");
        let direct = rmcp::serde_json::to_value(first).expect("output serializes");
        assert_eq!(via_registry, direct);
    }

    #[test]
    fn sim_step_refuses_bad_input_with_legal_values() {
        let server = server();
        for participants in [1, MAX_PARTICIPANTS + 1] {
            let refused = SimStepTool::invoke(
                &server,
                SimStepArgs {
                    participants: Some(participants),
                    ..step_args()
                },
            )
            .expect_err("headcount outside 2..=16 refuses");
            assert_eq!(refused.code, "bad_fighters");
            assert!(refused.message.contains("2..=16"), "{}", refused.message);
        }
        for half in [4.0, 101.0, f32::NAN, f32::INFINITY] {
            let refused = SimStepTool::invoke(
                &server,
                SimStepArgs {
                    half_extent_m: Some(half),
                    ..step_args()
                },
            )
            .expect_err("extent outside 5.0..=100.0 refuses");
            assert_eq!(refused.code, "bad_extent");
            assert!(
                refused.message.contains("5.0..=100.0"),
                "{}",
                refused.message
            );
        }
        for ticks in [0, crate::play::MAX_PLAY_TICKS + 1] {
            let refused = SimStepTool::invoke(
                &server,
                SimStepArgs {
                    ticks: Some(ticks),
                    ..step_args()
                },
            )
            .expect_err("ticks outside 1..=3600 refuses");
            assert_eq!(refused.code, "bad_ticks");
            assert!(refused.message.contains("1..="), "{}", refused.message);
        }
        let missing = SimStepTool::invoke(
            &server,
            SimStepArgs {
                ticks: None,
                ..step_args()
            },
        )
        .expect_err("missing ticks refuses");
        assert_eq!(missing.code, "bad_ticks");
        let past_end = SimStepTool::invoke(
            &server,
            SimStepArgs {
                inputs: Some(vec![crate::play::PlayInputDto {
                    tick: 8,
                    fighter: 0,
                    mx: 0,
                    mz: 0,
                    facing: 0,
                    buttons: 0,
                }]),
                ..step_args()
            },
        )
        .expect_err("tick past the end refuses");
        assert!(
            past_end.code == "invalid_tape"
                || past_end.code == "unsorted_tape"
                || past_end.code == "duplicate_input",
            "got {}: {}",
            past_end.code,
            past_end.message
        );
    }

    #[test]
    fn sim_step_hides_until_loaded_and_writes_nothing() {
        let server = server();
        assert!(
            !crate::mcp::default_tool_names().contains(&TOOL_SIM_STEP),
            "gym waits for api.load; the default list does not grow"
        );
        assert!(
            tool_names().contains(&TOOL_SIM_STEP),
            "the verb stays callable by exact name either way"
        );
        let found = ApiSearchTool::invoke(
            &server,
            ApiSearchArgs {
                query: "sim step".to_owned(),
                limit: None,
            },
        )
        .expect("search serves");
        assert!(
            found.hits.iter().any(|hit| hit.path == TOOL_SIM_STEP),
            "api.search finds sim.step"
        );
        let before = server.lock().expect("test lock holds").doc.revision;
        SimStepTool::invoke(&server, step_args()).expect("step serves");
        let after = server.lock().expect("test lock holds").doc.revision;
        assert_eq!(before, after, "a read-only step moves no revision");
        let undone = TxUndoTool::invoke(&server, TxUndoArgs { undo_token: None })
            .expect_err("nothing was written, so nothing undoes");
        assert_eq!(undone.code, "nothing_to_undo");
    }

    #[test]
    fn sim_reset_writes_nothing_so_undo_stays_empty() {
        let server = server();
        let before = server.lock().expect("test lock holds").doc.revision;
        SimResetTool::invoke(&server, reset_args()).expect("reset serves");
        let after = server.lock().expect("test lock holds").doc.revision;
        assert_eq!(before, after, "a read-only reset moves no revision");
        let undone = TxUndoTool::invoke(&server, TxUndoArgs { undo_token: None })
            .expect_err("nothing was written, so nothing undoes");
        assert_eq!(undone.code, "nothing_to_undo");
    }
}
