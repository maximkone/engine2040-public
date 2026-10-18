//! `play` pack: `play.step_until` until one hp falls, plus the rival-door
//! verbs `input.inject` (hold one named action) and `state.digest` (hash one
//! bout prefix), and `play.sweep` (seeded bots play N bouts, one envelope);
//! see `play_ext.rs`, `rival.rs` and `sweep.rs`.
//! Consumer: MCP door after `api.load`. Owner: `engine-arena` sim; DTOs here.
//! Units: ticks, permille, meters, `u64` digests (decimal strings). Same args
//! twice seal same bytes; no clock. Errors: typed with recovery. DTO docs off.
use super::bad_args;
use super::{PackDef, PackError, VerbSpec};
use crate::mcp::{McpServer, McpToolError};
use crate::play::play_session::{dec_u64, BoutSpec, PlaySlot};
use crate::play::{build_tape, PlayInputDto, MAX_PLAY_INPUTS, MAX_PLAY_TICKS};
use crate::tool_capabilities::CapabilityCard;
use engine_arena::MAX_PARTICIPANTS;
use rmcp::schemars::JsonSchema;
// `self` for schemars derive with `serde(with)`.
use rmcp::serde::{self, Deserialize, Serialize};
/// Pack name.
pub const PACK: &str = "play";
/// Verb: run until one hp falls.
pub const TOOL_PLAY_STEP_UNTIL: &str = "play.step_until";

#[path = "play_ext.rs"]
mod play_ext;
#[path = "rival.rs"]
mod rival;
#[path = "sweep.rs"]
mod sweep;

pub use play_ext::{
    InputInjectArgs, InputInjectOutput, InputInjectTool, StateDigestArgs, StateDigestOutput,
    StateDigestTool, TOOL_INPUT_INJECT, TOOL_STATE_DIGEST,
};
pub use rival::{rival_for, verb_for_rival, RIVAL_PARITY};
pub use sweep::{PlaySweepArgs, PlaySweepOutput, PlaySweepTool, TOOL_PLAY_SWEEP};

/// The play pack, loaded on demand.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "run a bout prefix until one hp falls or a cap, hold one named action, hash one prefix",
        false,
    );
    pack.add(VerbSpec::of::<PlayStepUntilTool>(
        TOOL_PLAY_STEP_UNTIL,
        PACK,
        CapabilityCard::Observe,
    )?);
    play_ext::add_verbs(&mut pack)?;
    sweep::add_verb(&mut pack)?;
    Ok(pack)
}
/// Request: seed, bout, tape, cap and watched hp threshold.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PlayStepUntilArgs {
    #[serde(default, with = "dec_u64::opt")]
    #[schemars(with = "Option<String>")]
    pub seed: Option<u64>,
    #[serde(default, alias = "fighters")]
    pub participants: Option<u8>,
    #[serde(default)]
    pub half_extent_m: Option<f32>,
    #[serde(default)]
    pub inputs: Option<Vec<PlayInputDto>>,
    #[serde(default)]
    pub tick_cap: Option<u32>,
    #[serde(default)]
    pub fighter: Option<u8>,
    #[serde(default)]
    pub hp_below: Option<u16>,
}
/// Answer: first tripping tick, digest and hp.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PlayStepUntilOutput {
    pub tick: u32,
    pub stopped: bool,
    #[serde(with = "dec_u64")]
    #[schemars(with = "String")]
    pub digest: u64,
    pub participants: u8,
    pub fighter_hp: u16,
}
/// Run until watched hp falls or cap; same slot path as `sim.step`.
pub fn step_until_report(args: &PlayStepUntilArgs) -> Result<PlayStepUntilOutput, McpToolError> {
    let seed = args.seed.unwrap_or(0);
    let participants = args.participants.unwrap_or(2);
    let half = args.half_extent_m.unwrap_or(20.0);
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
    let cap = args.tick_cap.ok_or_else(|| {
        McpToolError::new(
            "bad_ticks",
            format!("tick_cap missing; submit 1..={MAX_PLAY_TICKS}; re-run with a cap"),
        )
    })?;
    if cap == 0 || cap > MAX_PLAY_TICKS {
        return Err(McpToolError::new(
            "bad_ticks",
            format!("tick_cap {cap} is outside 1..={MAX_PLAY_TICKS}; re-run in range"),
        ));
    }
    let slot_id = args.fighter.ok_or_else(|| {
        McpToolError::new(
            "bad_fighters",
            format!("fighter missing; submit 0..{participants}; re-run with a slot"),
        )
    })?;
    if slot_id >= participants {
        return Err(McpToolError::new(
            "bad_fighters",
            format!("fighter {slot_id} is outside 0..{participants}; re-run with a live slot"),
        ));
    }
    let below = args
        .hp_below
        .ok_or_else(|| bad_args("hp_below missing; submit 0..=1000; re-run".to_owned()))?;
    if below > 1000 {
        return Err(bad_args(format!(
            "hp_below {below} is outside 0..=1000; re-run in range"
        )));
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
    let spec = BoutSpec::new(seed, participants, half);
    let mut last = (cap, 0u64, 1000u16);
    for now in 1..=cap {
        let prefix: Vec<PlayInputDto> = inputs
            .iter()
            .filter(|dto| dto.tick < now)
            .cloned()
            .collect();
        let tape = build_tape(&prefix, now, participants)?;
        let mut slot = PlaySlot::default();
        let view = slot.advance_to(spec, &tape, now)?.observe(0);
        let hp = view
            .fighters
            .iter()
            .find(|f| f.id == slot_id)
            .map_or(1000u16, |f| f.hp);
        last = (now, view.digest, hp);
        if hp < below {
            return Ok(PlayStepUntilOutput {
                tick: now,
                stopped: true,
                digest: view.digest,
                participants,
                fighter_hp: hp,
            });
        }
    }
    Ok(PlayStepUntilOutput {
        tick: last.0,
        stopped: false,
        digest: last.1,
        participants,
        fighter_hp: last.2,
    })
}
fn play_step_until(
    service: &McpServer,
    args: &PlayStepUntilArgs,
) -> Result<PlayStepUntilOutput, McpToolError> {
    service.dispatch(TOOL_PLAY_STEP_UNTIL, |_state| step_until_report(args))
}
crate::e2_tool!(
    /// `play.step_until` tool: run until one hp falls. Read-only.
    PlayStepUntilTool,
    name = TOOL_PLAY_STEP_UNTIL,
    description = "Run a bout prefix until one hp falls below hp_below or tick_cap: seed, bout, tape, cap and slot in; first tick, digest and hp out. Same inputs replay same tick. Read-only.",
    readonly = true,
    args = PlayStepUntilArgs,
    output = PlayStepUntilOutput,
    invoke = play_step_until
);
#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight::mcp_tools::{ApiDescribeArgs, ApiDescribeTool};
    use crate::packs::PackRegistry;
    use engine_editor::capability::Capability;
    use rmcp::handler::server::router::tool::SyncTool;
    fn server() -> McpServer {
        McpServer::new(7, Capability::unrestricted(7))
    }
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
    fn until_args(cap: u32) -> PlayStepUntilArgs {
        PlayStepUntilArgs {
            seed: Some(0),
            participants: Some(2),
            half_extent_m: Some(20.0),
            inputs: Some(script()),
            tick_cap: Some(cap),
            fighter: Some(1),
            hp_below: Some(900),
        }
    }
    #[test]
    fn play_step_until_hp_below_900_stops_on_the_first_such_tick() {
        let server = server();
        let first = PlayStepUntilTool::invoke(&server, until_args(60)).expect("serves");
        assert!(first.stopped, "seed 0 hurts below 900: {first:?}");
        assert!((1..=60).contains(&first.tick));
        assert!(first.fighter_hp < 900);
        assert_ne!(first.digest, 0);
        assert_eq!(
            first,
            PlayStepUntilTool::invoke(&server, until_args(60)).expect("replays")
        );
        let early = step_until_report(&until_args(first.tick - 1)).expect("early serves");
        assert!(!early.stopped, "before still holds: {early:?}");
    }
    #[test]
    fn play_step_until_is_listed_read_only_and_refuses_bad_input() {
        let verb = PackRegistry::shipped()
            .verb(TOOL_PLAY_STEP_UNTIL)
            .expect("registers");
        assert_eq!(verb.pack(), PACK);
        assert_eq!(verb.card(), CapabilityCard::Observe);
        assert!(verb.readonly());
        let described = ApiDescribeTool::invoke(
            &server(),
            ApiDescribeArgs {
                path: TOOL_PLAY_STEP_UNTIL.to_owned(),
            },
        )
        .expect("describe lists it");
        assert_eq!(described.path, TOOL_PLAY_STEP_UNTIL);
        assert!(described.docs.contains("hp"));
        for bad in [
            PlayStepUntilArgs {
                tick_cap: Some(0),
                ..until_args(60)
            },
            PlayStepUntilArgs {
                fighter: Some(5),
                ..until_args(60)
            },
            PlayStepUntilArgs {
                hp_below: Some(1001),
                ..until_args(60)
            },
        ] {
            assert!(step_until_report(&bad)
                .expect_err("refuses")
                .message
                .contains("re-run"));
        }
        assert_eq!(
            step_until_report(&PlayStepUntilArgs {
                tick_cap: None,
                ..until_args(60)
            })
            .expect_err("missing")
            .code,
            "bad_ticks"
        );
    }
}
