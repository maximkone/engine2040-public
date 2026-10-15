//! `play` pack extension: `input.inject` and `state.digest`.
//!
//! Consumer: drivers shaped like other engines' doors (`godot_input` holds a
//! button, `godot_runtime_state` reads the hash; see `rival.rs`). Data owner
//! for simulation is `engine-arena`; this file owns only the verb DTOs,
//! validation and the stateless report cores. Units: ticks, permille,
//! `u64` digests (decimal strings). Same args twice seal the same bytes; no
//! clock, no session writes. Errors: typed with recovery. DTO docs off.
use rmcp::schemars::JsonSchema;
// `self` for schemars derive with `serde(with)`.
use rmcp::serde::{self, Deserialize, Serialize};

use crate::mcp::{McpServer, McpToolError};
use crate::packs::{PackDef, PackError, VerbSpec};
use crate::play::play_session::{dec_u64, BoutSpec, PlaySlot};
use crate::play::{build_tape, PlayInputDto, MAX_PLAY_INPUTS, MAX_PLAY_TICKS};
use crate::tool_capabilities::CapabilityCard;
use engine_arena::MAX_PARTICIPANTS;

/// Verb: hold one named action for N ticks.
pub const TOOL_INPUT_INJECT: &str = "input.inject";
/// Verb: hash of a bout prefix.
pub const TOOL_STATE_DIGEST: &str = "state.digest";
/// Button bit behind the `attack` action.
const BUTTON_ATTACK: u8 = 0x01;
/// Button bit behind the `block` action.
const BUTTON_BLOCK: u8 = 0x02;
/// Button bit behind the `dodge` action.
const BUTTON_DODGE: u8 = 0x04;

/// Register the two extension verbs on a play pack under construction.
pub fn add_verbs(pack: &mut PackDef) -> Result<(), PackError> {
    pack.add(VerbSpec::of::<InputInjectTool>(
        TOOL_INPUT_INJECT,
        super::PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<StateDigestTool>(
        TOOL_STATE_DIGEST,
        super::PACK,
        CapabilityCard::Observe,
    )?);
    Ok(())
}

/// Request: seed, bout, fighter, one named action and how long to hold it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct InputInjectArgs {
    #[serde(default, with = "dec_u64::opt")]
    #[schemars(with = "Option<String>")]
    pub seed: Option<u64>,
    #[serde(default, alias = "fighters")]
    pub participants: Option<u8>,
    #[serde(default)]
    pub half_extent_m: Option<f32>,
    #[serde(default)]
    pub fighter: Option<u8>,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub ticks: Option<u32>,
    #[serde(default)]
    pub start_tick: Option<u32>,
}

/// Answer: tick count, digest and the injected fighter's hp.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct InputInjectOutput {
    pub tick: u32,
    #[serde(with = "dec_u64")]
    #[schemars(with = "String")]
    pub digest: u64,
    pub participants: u8,
    pub fighter: u8,
    pub action: String,
    pub ticks: u32,
    pub fighter_hp: u16,
}

/// Request: seed, bout, tape and tick count.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct StateDigestArgs {
    #[serde(default, with = "dec_u64::opt")]
    #[schemars(with = "Option<String>")]
    pub seed: Option<u64>,
    #[serde(default, alias = "fighters")]
    pub participants: Option<u8>,
    #[serde(default)]
    pub half_extent_m: Option<f32>,
    #[serde(default)]
    pub ticks: Option<u32>,
    #[serde(default)]
    pub inputs: Option<Vec<PlayInputDto>>,
}

/// Answer: tick count, digest and headcount.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct StateDigestOutput {
    pub tick: u32,
    #[serde(with = "dec_u64")]
    #[schemars(with = "String")]
    pub digest: u64,
    pub participants: u8,
}

fn bad_args(message: String) -> McpToolError {
    McpToolError::new("bad_args", message)
}

fn check_bout(
    participants: Option<u8>,
    half_extent_m: Option<f32>,
) -> Result<(u8, f32), McpToolError> {
    let participants = participants.unwrap_or(2);
    let half = half_extent_m.unwrap_or(20.0);
    if !(2..=MAX_PARTICIPANTS).contains(&participants) {
        return Err(McpToolError::new(
            "bad_fighters",
            format!(
                "participants {participants} is outside 2..={MAX_PARTICIPANTS}; re-run in range"
            ),
        ));
    }
    if !half.is_finite() || !(5.0..=100.0).contains(&half) {
        return Err(McpToolError::new(
            "bad_extent",
            format!("half_extent_m {half} is outside 5.0..=100.0; re-run in range"),
        ));
    }
    Ok((participants, half))
}

/// Named action to button bits; unknown names refuse with the legal values.
fn action_buttons(action: &str) -> Result<u8, McpToolError> {
    match action {
        "attack" => Ok(BUTTON_ATTACK),
        "block" => Ok(BUTTON_BLOCK),
        "dodge" => Ok(BUTTON_DODGE),
        _ => Err(bad_args(format!(
            "action `{action}` is unknown; submit attack, block or dodge; re-run"
        ))),
    }
}

/// Hold one named action for N ticks over a fresh bout; same slot path as
/// `sim.step`, so the digest equals the same tape stepped directly.
pub fn inject_report(args: &InputInjectArgs) -> Result<InputInjectOutput, McpToolError> {
    let seed = args.seed.unwrap_or(0);
    let (participants, half) = check_bout(args.participants, args.half_extent_m)?;
    let fighter = args.fighter.unwrap_or(0);
    if fighter >= participants {
        return Err(McpToolError::new(
            "bad_fighters",
            format!("fighter {fighter} is outside 0..{participants}; re-run with a live slot"),
        ));
    }
    let action = args.action.clone().ok_or_else(|| {
        bad_args("action missing; submit attack, block or dodge; re-run".to_owned())
    })?;
    let buttons = action_buttons(&action)?;
    let ticks = args.ticks.ok_or_else(|| {
        McpToolError::new(
            "bad_ticks",
            format!("ticks missing; submit 1..={MAX_PLAY_TICKS}; re-run with a count"),
        )
    })?;
    if ticks == 0 || ticks > MAX_PLAY_TICKS {
        return Err(McpToolError::new(
            "bad_ticks",
            format!("ticks {ticks} is outside 1..={MAX_PLAY_TICKS}; re-run in range"),
        ));
    }
    let start = args.start_tick.unwrap_or(0);
    let end = start.checked_add(ticks).ok_or_else(|| {
        McpToolError::new(
            "bad_ticks",
            format!("start_tick {start} plus {ticks} ticks overflows; re-run inside 1..={MAX_PLAY_TICKS}"),
        )
    })?;
    if end > MAX_PLAY_TICKS {
        return Err(McpToolError::new(
            "bad_ticks",
            format!("end tick {end} is past {MAX_PLAY_TICKS}; re-run inside the bound"),
        ));
    }
    let inputs: Vec<PlayInputDto> = (start..end)
        .map(|tick| PlayInputDto {
            tick,
            fighter,
            mx: 0,
            mz: 0,
            facing: 0,
            buttons,
        })
        .collect();
    if inputs.len() > MAX_PLAY_INPUTS {
        return Err(McpToolError::new(
            "too_many_inputs",
            format!(
                "inputs holds {} entries; at most {MAX_PLAY_INPUTS}; re-run shorter",
                inputs.len()
            ),
        ));
    }
    let tape = build_tape(&inputs, end, participants)?;
    let spec = BoutSpec::new(seed, participants, half);
    let mut slot = PlaySlot::default();
    let view = slot.advance_to(spec, &tape, end)?.observe(0);
    let hp = view
        .fighters
        .iter()
        .find(|f| f.id == fighter)
        .map_or(1000u16, |f| f.hp);
    Ok(InputInjectOutput {
        tick: end,
        digest: view.digest,
        participants,
        fighter,
        action,
        ticks,
        fighter_hp: hp,
    })
}

/// Hash one bout prefix over a fresh bout; the digest replays `sim.step` and
/// `play.run` on the same seed, extent, participants and tape.
pub fn digest_report(args: &StateDigestArgs) -> Result<StateDigestOutput, McpToolError> {
    let seed = args.seed.unwrap_or(0);
    let (participants, half) = check_bout(args.participants, args.half_extent_m)?;
    let ticks = args.ticks.ok_or_else(|| {
        McpToolError::new(
            "bad_ticks",
            format!("ticks missing; submit 1..={MAX_PLAY_TICKS}; re-run with a count"),
        )
    })?;
    if ticks == 0 || ticks > MAX_PLAY_TICKS {
        return Err(McpToolError::new(
            "bad_ticks",
            format!("ticks {ticks} is outside 1..={MAX_PLAY_TICKS}; re-run in range"),
        ));
    }
    let inputs = args.inputs.clone().unwrap_or_default();
    if inputs.len() > MAX_PLAY_INPUTS {
        return Err(McpToolError::new(
            "too_many_inputs",
            format!(
                "inputs holds {} entries; at most {MAX_PLAY_INPUTS}; re-run shorter",
                inputs.len()
            ),
        ));
    }
    let tape = build_tape(&inputs, ticks, participants)?;
    let spec = BoutSpec::new(seed, participants, half);
    let mut slot = PlaySlot::default();
    let view = slot.advance_to(spec, &tape, ticks)?.observe(0);
    Ok(StateDigestOutput {
        tick: ticks,
        digest: view.digest,
        participants,
    })
}

fn input_inject(
    service: &McpServer,
    args: &InputInjectArgs,
) -> Result<InputInjectOutput, McpToolError> {
    service.dispatch(TOOL_INPUT_INJECT, |_state| inject_report(args))
}

fn state_digest(
    service: &McpServer,
    args: &StateDigestArgs,
) -> Result<StateDigestOutput, McpToolError> {
    service.dispatch(TOOL_STATE_DIGEST, |_state| digest_report(args))
}

crate::e2_tool!(
    /// `input.inject` tool: hold one named action for N ticks. Read-only.
    InputInjectTool,
    name = TOOL_INPUT_INJECT,
    description = "Hold one named action (attack, block, dodge) for N ticks from a start tick: seed, bout, slot, action and count in; tick, digest and hp out. Same inputs replay the same digest. Read-only.",
    readonly = true,
    args = InputInjectArgs,
    output = InputInjectOutput,
    invoke = input_inject
);

crate::e2_tool!(
    /// `state.digest` tool: hash one bout prefix. Read-only.
    StateDigestTool,
    name = TOOL_STATE_DIGEST,
    description = "Hash a bout prefix: seed, bout, tape and tick count in; tick, digest and headcount out. Equals the sim.step digest and the play.run final hash on the same prefix. Read-only.",
    readonly = true,
    args = StateDigestArgs,
    output = StateDigestOutput,
    invoke = state_digest
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight::mcp_tools::{ApiDescribeArgs, ApiDescribeTool};
    use crate::packs::gym::{SimStepArgs, SimStepTool};
    use crate::packs::PackRegistry;
    use engine_editor::capability::Capability;
    use rmcp::handler::server::router::tool::SyncTool;

    fn server() -> McpServer {
        McpServer::new(7, Capability::unrestricted(7))
    }

    fn inject_args() -> InputInjectArgs {
        InputInjectArgs {
            seed: Some(0),
            participants: Some(2),
            half_extent_m: Some(20.0),
            fighter: Some(0),
            action: Some("attack".to_owned()),
            ticks: Some(3),
            start_tick: None,
        }
    }

    fn digest_args() -> StateDigestArgs {
        StateDigestArgs {
            seed: Some(0),
            participants: Some(2),
            half_extent_m: Some(20.0),
            ticks: Some(3),
            inputs: Some(vec![
                PlayInputDto {
                    tick: 0,
                    fighter: 0,
                    mx: 0,
                    mz: 0,
                    facing: 0,
                    buttons: BUTTON_ATTACK,
                },
                PlayInputDto {
                    tick: 1,
                    fighter: 0,
                    mx: 0,
                    mz: 0,
                    facing: 0,
                    buttons: BUTTON_ATTACK,
                },
                PlayInputDto {
                    tick: 2,
                    fighter: 0,
                    mx: 0,
                    mz: 0,
                    facing: 0,
                    buttons: BUTTON_ATTACK,
                },
            ]),
        }
    }

    #[test]
    fn inject_attack_3_ticks_moves_like_the_same_fighter_input() {
        let server = server();
        let injected = InputInjectTool::invoke(&server, inject_args()).expect("serves");
        assert_eq!(injected.tick, 3);
        assert_eq!(injected.participants, 2);
        assert_eq!(injected.fighter, 0);
        assert_eq!(injected.action, "attack");
        assert_eq!(injected.ticks, 3);
        assert_ne!(injected.digest, 0);
        assert_eq!(
            injected,
            InputInjectTool::invoke(&server, inject_args()).expect("replays")
        );
        // The same tape stepped directly seals the same digest: the inject
        // is exactly three attack `FighterInput`s for fighter 0.
        let stepped = SimStepTool::invoke(
            &server,
            SimStepArgs {
                seed: Some(0),
                participants: Some(2),
                half_extent_m: Some(20.0),
                ticks: Some(3),
                inputs: digest_args().inputs,
                events: None,
            },
        )
        .expect("sim.step serves");
        assert_eq!(injected.digest, stepped.digest);
        assert_eq!(injected.fighter_hp, stepped.fighters[0].hp);
    }

    #[test]
    fn digest_equals_the_replayed_hash() {
        let server = server();
        let first = StateDigestTool::invoke(&server, digest_args()).expect("serves");
        assert_eq!(first.tick, 3);
        assert_eq!(first.participants, 2);
        assert_ne!(first.digest, 0);
        assert_eq!(
            first,
            StateDigestTool::invoke(&server, digest_args()).expect("replays")
        );
        // Same prefix through `sim.step` seals the same hash.
        let stepped = SimStepTool::invoke(
            &server,
            SimStepArgs {
                seed: Some(0),
                participants: Some(2),
                half_extent_m: Some(20.0),
                ticks: Some(3),
                inputs: digest_args().inputs,
                events: None,
            },
        )
        .expect("sim.step serves");
        assert_eq!(first.digest, stepped.digest);
        // The registry path answers the same bytes as the direct tool path.
        let via_registry = PackRegistry::shipped()
            .verb(TOOL_STATE_DIGEST)
            .expect("verb registers")
            .invoke(
                &server,
                rmcp::serde_json::to_value(digest_args()).expect("args serialize"),
            )
            .expect("registry dispatches");
        let direct = rmcp::serde_json::to_value(first).expect("output serializes");
        assert_eq!(via_registry, direct);
    }

    #[test]
    fn inject_and_digest_are_listed_read_only_and_refuse_bad_input() {
        for (verb, card) in [
            (TOOL_INPUT_INJECT, CapabilityCard::Observe),
            (TOOL_STATE_DIGEST, CapabilityCard::Observe),
        ] {
            let spec = PackRegistry::shipped().verb(verb).expect("registers");
            assert_eq!(spec.pack(), crate::packs::play::PACK);
            assert_eq!(spec.card(), card);
            assert!(spec.readonly());
            let described = ApiDescribeTool::invoke(
                &server(),
                ApiDescribeArgs {
                    path: verb.to_owned(),
                },
            )
            .expect("describe lists it");
            assert_eq!(described.path, verb);
        }
        for bad in [
            InputInjectArgs {
                action: Some("fireball".to_owned()),
                ..inject_args()
            },
            InputInjectArgs {
                ticks: Some(0),
                ..inject_args()
            },
            InputInjectArgs {
                fighter: Some(5),
                ..inject_args()
            },
            InputInjectArgs {
                ticks: None,
                ..inject_args()
            },
        ] {
            assert!(inject_report(&bad)
                .expect_err("refuses")
                .message
                .contains("re-run"));
        }
        assert_eq!(
            inject_report(&InputInjectArgs {
                action: None,
                ..inject_args()
            })
            .expect_err("missing action")
            .code,
            "bad_args"
        );
        for bad in [
            StateDigestArgs {
                ticks: Some(0),
                ..digest_args()
            },
            StateDigestArgs {
                ticks: None,
                ..digest_args()
            },
        ] {
            assert!(digest_report(&bad)
                .expect_err("refuses")
                .message
                .contains("re-run"));
        }
    }
}
