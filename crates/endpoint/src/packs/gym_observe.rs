//! `sim.observe`: read the session's live bout as numbers.
//!
//! Consumer: AI training loops and weak-model drivers through the MCP door
//! (`sim.reset` or `sim.step`, then `sim.observe`). Data owner for
//! simulation stays `engine-arena`; this file owns the verb DTOs, the typed
//! refusals and the round-trip proofs. Units: meters, radians, permille,
//! stamina points, fixed ticks. Hashes and ids (`digest`, `seed`) go out as
//! decimal strings, so a client that parses JSON numbers as f64 keeps them
//! exact. Determinism: the same seed plus tape twice seals the same bytes;
//! no wall clock, no threads. Errors: `no_live_bout` before any reset or
//! step, `bad_events` past the ring; both name the legal recovery.
//! Budget: one fighter row per participant plus at most 64 events; the read
//! never steps the sim.

use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use super::event_window;
use crate::mcp::{McpServer, McpToolError};
use crate::play::play_session::{ActorNumbers, BoutEvent, PlaySlot};
use crate::tool_capabilities::CheckKind;

/// Verb: read the live bout's numbers without stepping it.
pub const TOOL_SIM_OBSERVE: &str = "sim.observe";

/// Args for `sim.observe`: how many newest events to return.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SimObserveArgs {
    /// How many of the newest combat events to return, 0..=64 (default 16).
    #[serde(default)]
    pub events: Option<u32>,
}

/// `sim.observe` result: the live bout after its last simulated tick.
///
/// `tick` is the 1-indexed count of ticks simulated, so every event `tick`
/// is below it and a hit's hp change shows from the observation after it.
/// `digest` and `seed` are decimal strings of `u64` values (exact in any
/// JSON client); `digest` is `"0"` before the first tick.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SimObserveOutput {
    /// Ticks simulated so far: the 1-indexed count.
    pub tick: u32,
    /// Match seed of the live bout, as a decimal string.
    pub seed: String,
    /// State digest (checkpoint `tick - 1` hash), as a decimal string.
    pub digest: String,
    /// Participant count of the live bout.
    pub participants: u8,
    /// Arena half-extent of the live bout in meters.
    pub half_extent_m: f32,
    /// Capability check that ran (`readonly`).
    pub check: String,
    /// Participant numbers in slot order: hp, stamina, phase, position, yaw.
    pub fighters: Vec<ActorNumbers>,
    /// The newest combat events, oldest first, each with its tick.
    pub events: Vec<BoutEvent>,
}

/// Pure observe core: read the held bout. Steps nothing and writes nothing.
fn observe_core(slot: &PlaySlot, args: &SimObserveArgs) -> Result<SimObserveOutput, McpToolError> {
    let window = event_window(args.events)?;
    let bout = slot.current().ok_or_else(|| {
        McpToolError::new(
            "no_live_bout",
            "no live bout in this session; call sim.reset or sim.step first, then re-run sim.observe",
        )
    })?;
    let spec = bout.spec();
    let view = bout.observe(window);
    Ok(SimObserveOutput {
        tick: view.tick,
        seed: spec.seed.to_string(),
        digest: view.digest.to_string(),
        participants: spec.participants,
        half_extent_m: spec.half_extent_m(),
        check: CheckKind::ReadOnly.name().to_owned(),
        fighters: view.fighters,
        events: view.events,
    })
}

/// `sim.observe` handler: the pure core under the audited dispatch seam (one
/// audit record, no document write, no journal entry).
fn sim_observe(
    service: &McpServer,
    args: &SimObserveArgs,
) -> Result<SimObserveOutput, McpToolError> {
    service.dispatch(TOOL_SIM_OBSERVE, |state| observe_core(&state.play, args))
}

crate::e2_tool!(
    /// `sim.observe` tool: the live bout's per-fighter numbers and newest events out. Read-only.
    SimObserveTool,
    name = TOOL_SIM_OBSERVE,
    description = "Observe the live bout without stepping it: tick, digest and seed as decimal strings, per-fighter hp, stamina, phase, position and yaw, and the newest hit/block/parry/dodge/kill events with their ticks. Run sim.reset or sim.step first. Writes nothing.",
    readonly = true,
    args = SimObserveArgs,
    output = SimObserveOutput,
    invoke = sim_observe
);

#[cfg(test)]
mod tests {
    use crate::packs::gym::SimResetParticipant;
    use crate::play::{build_tape, PlayInputDto};
    use engine_arena::{observe_state, run, ArenaConfig, FighterTape};

    const SEED: u64 = 2040;
    const TICKS: u32 = 8;
    const FIGHTERS: u8 = 2;
    const HALF_M: f32 = 20.0;

    fn config() -> ArenaConfig {
        ArenaConfig::try_new(SEED, TICKS, FIGHTERS, HALF_M)
            .expect("seed 2040 with 2 fighters validates")
    }

    fn idle_tape() -> FighterTape {
        build_tape(&[], TICKS, FIGHTERS).expect("empty tape validates")
    }

    fn taped_inputs() -> Vec<PlayInputDto> {
        vec![PlayInputDto {
            tick: 0,
            fighter: 0,
            mx: 100,
            mz: 0,
            facing: 0,
            buttons: 0x01,
        }]
    }

    #[test]
    fn observe_round_trip_matches_run_digest() {
        let config = config();
        let tape = idle_tape();
        let first = observe_state(&config, &tape).expect("valid observe serves");
        let second = observe_state(&config, &tape).expect("observe replays");
        assert_eq!(first, second, "same seed plus tape seals the same snapshot");
        assert_eq!(first.tick, TICKS);
        assert_eq!(first.fighters.len(), usize::from(FIGHTERS));
        // The digest equals the `run` final hash for the same config plus
        // tape, so gym replays and `play.run` receipts agree on one value.
        let points = run(&config, &tape);
        assert_eq!(points.len(), TICKS as usize, "8 ticks seal 8 checkpoints");
        let last = points.last().expect("8 ticks seal a final hash");
        assert_eq!(
            first.digest, last.hash,
            "observe digest and run final hash agree"
        );
        assert_ne!(first.digest, 0, "the digest carries the stepped state");
        // One taped input must move the digest; another seed must too.
        let taped = build_tape(&taped_inputs(), TICKS, FIGHTERS).expect("one input validates");
        let stepped = observe_state(&config, &taped).expect("taped observe serves");
        assert_eq!(stepped.tick, TICKS);
        assert_eq!(stepped.fighters.len(), usize::from(FIGHTERS));
        assert_ne!(
            first.digest, stepped.digest,
            "idle and taped prefixes must diverge"
        );
        let other = ArenaConfig::try_new(7, TICKS, FIGHTERS, HALF_M).expect("other seed validates");
        let foreign = observe_state(&other, &tape).expect("other seed serves");
        assert_ne!(
            first.digest, foreign.digest,
            "seed 2040 and seed 7 must diverge"
        );
        // Every observed fighter is sane: slot order, meters, permille.
        for (slot, fighter) in first.fighters.iter().enumerate() {
            assert_eq!(fighter.id.0, slot as u8, "fighters read back in slot order");
            assert!(fighter.hp <= 1000, "health stays in 0..=1000 permille");
            assert!(fighter.stamina <= 100, "stamina stays in 0..=100 points");
            assert!(
                fighter.pos.iter().all(|lane| lane.is_finite()),
                "positions stay finite meters"
            );
            assert!(fighter.yaw.is_finite(), "yaw stays finite radians");
        }
    }

    #[test]
    fn observe_inputs_and_fighter_dtos_json_round_trip() {
        // The tape inputs survive the door JSON there and back.
        let inputs = taped_inputs();
        let json = rmcp::serde_json::to_value(&inputs).expect("inputs serialize");
        let back: Vec<PlayInputDto> =
            rmcp::serde_json::from_value(json).expect("inputs deserialize");
        assert_eq!(inputs, back);
        // The observation DTO mirrors the snapshot fighter and survives the
        // door JSON too, so what the sim saw is what the client reads.
        let snapshot = observe_state(&config(), &idle_tape()).expect("observe serves");
        let dtos: Vec<SimResetParticipant> = snapshot
            .fighters
            .iter()
            .map(|fighter| SimResetParticipant {
                id: fighter.id.0,
                pos: fighter.pos,
                yaw: fighter.yaw,
                hp: fighter.hp,
                reward: fighter.reward,
                legal_mask: fighter.legal_mask,
                velocity: fighter.velocity,
            })
            .collect();
        assert_eq!(dtos.len(), usize::from(FIGHTERS));
        let json = rmcp::serde_json::to_value(&dtos).expect("fighters serialize");
        let back: Vec<SimResetParticipant> =
            rmcp::serde_json::from_value(json).expect("fighters deserialize");
        assert_eq!(dtos, back);
    }

    #[test]
    fn observe_refuses_bad_input_with_none_and_typed_codes() {
        let tape = idle_tape();
        // Bad configs read back as `None`, never partial.
        for bad in [
            ArenaConfig {
                seed: SEED,
                ticks: 0,
                fighters: FIGHTERS,
                half_extent_m: HALF_M,
                table_version: 1,
            },
            ArenaConfig {
                seed: SEED,
                ticks: TICKS,
                fighters: 1,
                half_extent_m: HALF_M,
                table_version: 1,
            },
            ArenaConfig {
                seed: SEED,
                ticks: TICKS,
                fighters: FIGHTERS,
                half_extent_m: f32::NAN,
                table_version: 1,
            },
        ] {
            assert!(
                observe_state(&bad, &tape).is_none(),
                "bad config holds nothing"
            );
        }
        // Bad tape rows refuse typed, naming the legal values.
        let past_end = build_tape(
            &[PlayInputDto {
                tick: TICKS,
                ..taped_inputs()[0].clone()
            }],
            TICKS,
            FIGHTERS,
        )
        .expect_err("tick past the end refuses");
        assert_eq!(past_end.code, "invalid_tape");
        assert!(past_end.message.contains("past the match end"));
        let past_slots = build_tape(
            &[PlayInputDto {
                fighter: FIGHTERS,
                ..taped_inputs()[0].clone()
            }],
            TICKS,
            FIGHTERS,
        )
        .expect_err("fighter past the slots refuses");
        assert_eq!(past_slots.code, "invalid_tape");
        assert!(past_slots.message.contains("outside 0.."));
    }

    // ---- door tests: the verbs through the session --------------------------

    use super::{SimObserveArgs, SimObserveOutput, SimObserveTool, TOOL_SIM_OBSERVE};
    use crate::mcp::{default_tool_names, tool_names, McpServer};
    use crate::packs::gym::{
        SimResetArgs, SimResetOutput, SimResetTool, SimStepArgs, SimStepOutput, SimStepTool, PACK,
    };
    use crate::packs::{ApiLoadArgs, ApiLoadTool, PackRegistry};
    use crate::tool_capabilities::CapabilityCard;
    use engine_editor::capability::Capability;
    use rmcp::handler::server::router::tool::SyncTool;
    use rmcp::serde_json::Value;

    /// Seed 2 is a bout where the closing fighter's swing and the brain's
    /// answers land several hits inside 60 ticks.
    const BOUT_SEED: u64 = 2;

    fn server() -> McpServer {
        McpServer::new(7, Capability::unrestricted(7))
    }

    /// Fighter 0 closes and swings at tick 0, then swings again at tick 30.
    fn script() -> Vec<PlayInputDto> {
        vec![
            PlayInputDto {
                tick: 0,
                fighter: 0,
                mx: 100,
                mz: 0,
                facing: 0,
                buttons: 0x01,
            },
            PlayInputDto {
                tick: 30,
                fighter: 0,
                mx: 60,
                mz: 0,
                facing: 0,
                buttons: 0x01,
            },
        ]
    }

    /// The step to `ticks`: the whole script prefix that fits inside it.
    fn step_args(ticks: u32) -> SimStepArgs {
        SimStepArgs {
            seed: Some(BOUT_SEED),
            participants: Some(2),
            half_extent_m: Some(20.0),
            ticks: Some(ticks),
            inputs: Some(
                script()
                    .into_iter()
                    .filter(|input| input.tick < ticks)
                    .collect(),
            ),
            events: Some(64),
        }
    }

    fn step(server: &McpServer, ticks: u32) -> SimStepOutput {
        SimStepTool::invoke(server, step_args(ticks)).expect("step serves")
    }

    fn observe(server: &McpServer) -> SimObserveOutput {
        SimObserveTool::invoke(server, SimObserveArgs { events: Some(64) }).expect("observe serves")
    }

    fn hp_of(out: &SimObserveOutput, slot: u8) -> u16 {
        out.fighters
            .iter()
            .find(|fighter| fighter.id == slot)
            .expect("slot is in the bout")
            .hp
    }

    #[test]
    fn observe_sees_a_hit_event_then_the_victims_hp_falls() {
        let server = server();
        step(&server, 60);
        let seen = observe(&server);
        assert_eq!(seen.tick, 60);
        assert_eq!(seen.fighters.len(), 2);
        let hit = seen
            .events
            .iter()
            .find(|event| event.kind == "hit")
            .expect("a hit lands inside sixty ticks")
            .clone();
        assert!(
            hit.tick > 0 && hit.tick < 60,
            "event ticks sit inside the bout"
        );
        assert!(hit.amount > 0, "a hit carries damage");
        assert!(
            hp_of(&seen, hit.victim) < 1000,
            "the victim ends the bout hurt"
        );
        // Replay to the tick before the hit resolves, then one tick past it.
        step(&server, hit.tick);
        let before = observe(&server);
        assert!(
            !before.events.iter().any(|event| *event == hit),
            "the hit has not happened yet at tick {}",
            hit.tick
        );
        step(&server, hit.tick + 1);
        let after = observe(&server);
        assert!(
            after.events.iter().any(|event| *event == hit),
            "the hit shows in the very next observation"
        );
        assert_eq!(
            hp_of(&before, hit.victim) - hp_of(&after, hit.victim),
            hit.amount,
            "hp falls by exactly the damage the event names"
        );
        assert!(hp_of(&seen, hit.victim) <= hp_of(&after, hit.victim));
    }

    #[test]
    fn observe_replay_twice_is_byte_identical() {
        let run_once = |seed: u64| {
            let server = server();
            SimStepTool::invoke(
                &server,
                SimStepArgs {
                    seed: Some(seed),
                    ..step_args(60)
                },
            )
            .expect("step serves");
            rmcp::serde_json::to_string(&observe(&server)).expect("observe serializes")
        };
        let first = run_once(BOUT_SEED);
        assert_eq!(first, run_once(BOUT_SEED), "two sessions, one byte stream");
        assert_ne!(first, run_once(BOUT_SEED + 1), "another seed must differ");
        // One session replaying the same bout after a reset says the same.
        let server = server();
        step(&server, 60);
        let live = rmcp::serde_json::to_string(&observe(&server)).expect("observe serializes");
        SimResetTool::invoke(
            &server,
            SimResetArgs {
                seed: Some(BOUT_SEED),
                participants: Some(2),
                half_extent_m: Some(20.0),
            },
        )
        .expect("reset serves");
        step(&server, 60);
        let replay = rmcp::serde_json::to_string(&observe(&server)).expect("observe serializes");
        assert_eq!(live, replay, "reset plus the same inputs replays the bytes");
        assert_eq!(live, first);
    }

    #[test]
    fn observe_digest_and_seed_are_decimal_strings() {
        let server = server();
        let stepped = step(&server, 60);
        let json = rmcp::serde_json::to_value(observe(&server)).expect("observe serializes");
        let digest = json["digest"].as_str().expect("digest is a JSON string");
        assert_eq!(
            digest.parse::<u64>().expect("digest is decimal"),
            stepped.digest,
            "the string carries the exact step digest"
        );
        assert_eq!(json["seed"].as_str(), Some("2"));
        assert!(json["tick"].is_u64(), "ticks stay plain numbers");
    }

    #[test]
    fn every_gym_digest_is_a_decimal_string_and_seed_takes_both_forms() {
        let server = server();
        let reset = SimResetTool::invoke(
            &server,
            SimResetArgs {
                seed: Some(BOUT_SEED),
                participants: Some(2),
                half_extent_m: Some(20.0),
            },
        )
        .expect("reset serves");
        let stepped = step(&server, 60);
        assert!(
            stepped.digest > 1 << 53,
            "the proof needs a digest an f64 cannot hold"
        );
        for (name, digest, json) in [
            (
                "sim.reset",
                reset.digest,
                rmcp::serde_json::to_value(&reset).expect("reset serializes"),
            ),
            (
                "sim.step",
                stepped.digest,
                rmcp::serde_json::to_value(&stepped).expect("step serializes"),
            ),
        ] {
            let text = json["digest"].as_str().expect("digest is a JSON string");
            assert_eq!(text.parse::<u64>().expect("decimal"), digest, "{name}");
            let back = rmcp::serde_json::from_value::<Value>(json.clone()).expect("value");
            assert_eq!(back["digest"], json["digest"], "{name} round trips");
        }
        assert_ne!(
            stepped.digest as f64 as u64, stepped.digest,
            "as a JSON number an f64 client would have rounded it"
        );
        let back: SimStepOutput =
            rmcp::serde_json::from_value(rmcp::serde_json::to_value(&stepped).expect("json"))
                .expect("the string form reads back");
        assert_eq!(back, stepped);
        // The advertised schemas say string for the digest and the seed.
        let schema = |value: Value| value["properties"].clone();
        let step_out = schema(
            rmcp::serde_json::to_value(rmcp::schemars::schema_for!(SimStepOutput))
                .expect("schema serializes"),
        );
        assert_eq!(step_out["digest"]["type"], "string");
        let reset_out = schema(
            rmcp::serde_json::to_value(rmcp::schemars::schema_for!(SimResetOutput))
                .expect("schema serializes"),
        );
        assert_eq!(reset_out["digest"]["type"], "string");
        for seed in [
            schema(
                rmcp::serde_json::to_value(rmcp::schemars::schema_for!(SimStepArgs))
                    .expect("schema serializes"),
            ),
            schema(
                rmcp::serde_json::to_value(rmcp::schemars::schema_for!(SimResetArgs))
                    .expect("schema serializes"),
            ),
        ] {
            let kind = seed["seed"]["type"].to_string();
            assert!(
                kind.contains("string") && !kind.contains("integer"),
                "{kind}"
            );
        }
        // Input takes a decimal string, a number or nothing.
        let parse = |text: &str| rmcp::serde_json::from_str::<SimStepArgs>(text);
        assert_eq!(parse(r#"{"ticks":4}"#).expect("no seed").seed, None);
        assert_eq!(parse(r#"{"seed":2040}"#).expect("number").seed, Some(2040));
        assert_eq!(
            parse(r#"{"seed":"18446744073709551615"}"#)
                .expect("string")
                .seed,
            Some(u64::MAX)
        );
        let reset_args = rmcp::serde_json::from_str::<SimResetArgs>(r#"{"seed":"2"}"#)
            .expect("reset takes a string seed");
        assert_eq!(reset_args.seed, Some(2));
        let refused = parse(r#"{"seed":"two"}"#).expect_err("a bad seed refuses");
        assert!(refused.to_string().contains("decimal string"), "{refused}");
    }

    #[test]
    fn observe_wire_shape_is_pinned() {
        let server = server();
        step(&server, 5);
        let json = rmcp::serde_json::to_value(observe(&server)).expect("observe serializes");
        let keys = |value: &Value| -> Vec<String> {
            value
                .as_object()
                .expect("an object")
                .keys()
                .cloned()
                .collect()
        };
        assert_eq!(
            keys(&json),
            [
                "check",
                "digest",
                "events",
                "fighters",
                "half_extent_m",
                "participants",
                "seed",
                "tick"
            ]
        );
        assert_eq!(
            keys(&json["fighters"][0]),
            [
                "hp",
                "id",
                "legal_mask",
                "phase",
                "pos",
                "reward",
                "stamina",
                "velocity",
                "yaw"
            ]
        );
        assert!(json["fighters"][1]["phase"].is_string());
        assert!(json["fighters"][0]["reward"].is_number());
        assert!(json["fighters"][0]["legal_mask"].is_array());
        assert!(json["fighters"][0]["velocity"].is_array());
        // A longer bout carries events: pin their keys too.
        step(&server, 60);
        let json = rmcp::serde_json::to_value(observe(&server)).expect("observe serializes");
        assert_eq!(
            keys(&json["events"][0]),
            ["amount", "attacker", "kind", "tick", "victim"]
        );
    }

    #[test]
    fn tool_list_carries_sim_observe_with_schemas() {
        assert!(tool_names().contains(&TOOL_SIM_OBSERVE));
        assert!(
            !default_tool_names().contains(&TOOL_SIM_OBSERVE),
            "gym waits for api.load; the default list does not grow"
        );
        let registry = PackRegistry::shipped();
        let spec = registry.verb(TOOL_SIM_OBSERVE).expect("verb registers");
        assert_eq!(spec.pack(), PACK);
        assert_ne!(spec.card(), CapabilityCard::Deny);
        let server = server();
        assert!(
            !server
                .tools()
                .iter()
                .any(|tool| tool.name == TOOL_SIM_OBSERVE),
            "unloaded, the verb stays out of the list"
        );
        ApiLoadTool::invoke(
            &server,
            ApiLoadArgs {
                pack: PACK.to_owned(),
                unload: None,
            },
        )
        .expect("gym loads");
        let tool = server
            .tools()
            .into_iter()
            .find(|tool| tool.name == TOOL_SIM_OBSERVE)
            .expect("sim.observe is in the tool list once gym loads");
        let input = Value::Object((*tool.input_schema).clone());
        assert!(
            input["properties"].get("events").is_some(),
            "the input schema names the events window: {input}"
        );
        let output = Value::Object(
            (**tool
                .output_schema
                .as_ref()
                .expect("sim.observe advertises an output schema"))
            .clone(),
        );
        for field in [
            "tick",
            "seed",
            "digest",
            "participants",
            "fighters",
            "events",
        ] {
            assert!(
                output["properties"].get(field).is_some(),
                "output schema names `{field}`: {output}"
            );
        }
        assert_eq!(
            output["properties"]["digest"]["type"], "string",
            "hashes advertise as strings"
        );
    }

    #[test]
    fn observe_refuses_without_a_bout_and_past_the_ring() {
        let server = server();
        let empty = SimObserveTool::invoke(&server, SimObserveArgs::default())
            .expect_err("nothing was stepped yet");
        assert_eq!(empty.code, "no_live_bout");
        assert!(empty.message.contains("sim.step"), "{}", empty.message);
        step(&server, 60);
        let wide = SimObserveTool::invoke(&server, SimObserveArgs { events: Some(65) })
            .expect_err("a window past the ring refuses");
        assert_eq!(wide.code, "bad_events");
        assert!(wide.message.contains("0..=64"), "{}", wide.message);
        // The window picks the newest events, oldest first.
        let all = observe(&server).events;
        assert!(all.len() >= 2, "the bout fought");
        let none = SimObserveTool::invoke(&server, SimObserveArgs { events: Some(0) })
            .expect("zero events serves");
        assert!(none.events.is_empty());
        let two = SimObserveTool::invoke(&server, SimObserveArgs { events: Some(2) })
            .expect("two events serve");
        assert_eq!(two.events, all[all.len() - 2..]);
        let default = SimObserveTool::invoke(&server, SimObserveArgs::default())
            .expect("the default window serves");
        assert_eq!(default.events.len(), all.len().min(16));
    }

    #[test]
    fn reset_opens_a_bout_that_observe_reads_at_tick_zero() {
        let server = server();
        let reset = SimResetTool::invoke(
            &server,
            SimResetArgs {
                seed: Some(BOUT_SEED),
                participants: Some(2),
                half_extent_m: Some(20.0),
            },
        )
        .expect("reset serves");
        let seen = observe(&server);
        assert_eq!(seen.tick, 0);
        assert_eq!(seen.digest, "0", "no digest before the first tick");
        assert_eq!(seen.seed, "2");
        assert_eq!(seen.participants, 2);
        assert!(seen.events.is_empty());
        for (fighter, spawn) in seen.fighters.iter().zip(&reset.fighters) {
            assert_eq!(fighter.id, spawn.id);
            assert_eq!(fighter.pos, spawn.pos);
            assert_eq!(fighter.yaw, spawn.yaw);
            assert_eq!(fighter.hp, 1000);
            assert_eq!(fighter.stamina, 100);
            assert_eq!(fighter.phase, "idle");
        }
        // The first step continues the reset bout instead of opening another.
        step(&server, 10);
        assert_eq!(
            server.lock().expect("test lock holds").play.steps,
            10,
            "reset plus ten ticks costs ten sim steps"
        );
    }

    #[test]
    fn a_longer_prefix_steps_only_the_new_ticks() {
        let server = server();
        let mut costs = Vec::new();
        for ticks in [20, 40, 60, 60, 30] {
            let held = step(&server, ticks);
            costs.push(server.lock().expect("test lock holds").play.steps);
            // Held or rebuilt, the answer is what a from-zero run gives.
            assert_eq!(held, step(&self::server(), ticks), "ticks {ticks}");
            assert_eq!(held.tick, ticks);
        }
        // Only the held server's counter is read: 20, +20, +20, +0, then a
        // rewind to 30 rebuilds from tick 0.
        assert_eq!(costs, [20, 40, 60, 60, 90]);
        assert_eq!(
            observe(&server).tick,
            30,
            "the rewound bout is the held one"
        );
    }

    #[test]
    fn a_changed_early_input_rebuilds_and_matches_a_fresh_run() {
        let server = server();
        step(&server, 40);
        let mut changed = step_args(50);
        changed.inputs = Some(vec![PlayInputDto {
            tick: 3,
            fighter: 1,
            mx: -100,
            mz: 0,
            facing: 18_000,
            buttons: 0x02,
        }]);
        let rebuilt = SimStepTool::invoke(&server, changed.clone()).expect("changed tape serves");
        let fresh = SimStepTool::invoke(&self::server(), changed).expect("fresh run serves");
        assert_eq!(rebuilt, fresh, "a held bout never leaks into a new tape");
        assert_eq!(
            server.lock().expect("test lock holds").play.steps,
            40 + 50,
            "the changed prefix rebuilt from tick zero"
        );
        assert_ne!(rebuilt, step(&self::server(), 50), "the change matters");
    }
}
