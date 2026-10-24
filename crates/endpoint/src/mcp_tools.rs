//! rmcp tool definitions: one `SyncTool` per contract tool, all sync so
//! tests need no executor, beside the `mcp_dto` shapes they schematize.
//!
//! Structure-only split from `mcp.rs`: no logic moved, no behaviour changed.
//! Every item below is re-exported from `crate::mcp`, so existing paths such
//! as `crate::mcp::SceneReadTool` keep working.

use std::borrow::Cow;
use std::sync::Arc;

use rmcp::handler::server::router::tool::{SyncTool, ToolBase};
use rmcp::model::ToolAnnotations;
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};
use rmcp::serde_json::Value;

use super::{
    write_annotations, McpServer, McpToolError, SceneReadArgs, SceneReadOutput, TxApplyArgs,
    TxApplyOutput, TxPreviewArgs, TxPreviewOutput, TxUndoArgs, TxUndoOutput, TOOL_GAME_BUILD,
    TOOL_SCENE_READ, TOOL_SIM_BUILD_ENCOUNTER, TOOL_TX_APPLY, TOOL_TX_PREVIEW, TOOL_TX_UNDO,
};

// ---------------------------------------------------------------------------
// rmcp tools: one SyncTool per contract tool, all sync so tests need no
// executor. Schemas generate from the DTOs above via schemars.
// ---------------------------------------------------------------------------

/// `scene.read` tool: bounded snapshot, read-only.
pub struct SceneReadTool;

impl ToolBase for SceneReadTool {
    type Parameter = SceneReadArgs;
    type Output = SceneReadOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_SCENE_READ.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Bounded scene snapshot: document revision plus stable-id rows \
             (id, parent, local pose). Paged by cursor/limit; read-only, no side effects."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        crate::mcp::read_only_annotations()
    }
}

impl SyncTool<McpServer> for SceneReadTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        // LG10: a bounded snapshot shares the session state, so eight readers
        // sit inside the call at once while a write waits for all of them.
        service.dispatch_read(TOOL_SCENE_READ, &param, |state| state.scene_read(&param))
    }
}

/// `tx.preview` tool: dry-run diff, read-only.
pub struct TxPreviewTool;

impl ToolBase for TxPreviewTool {
    type Parameter = TxPreviewArgs;
    type Output = TxPreviewOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_TX_PREVIEW.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Dry-run a command batch: validate and diff with no side effects. \
             Returns effects, the resulting revision and a single-use approval for tx.apply."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        crate::mcp::read_only_annotations()
    }
}

impl SyncTool<McpServer> for TxPreviewTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        // A dry-run diff reads the document and stages nothing: shared span.
        service.dispatch_read(TOOL_TX_PREVIEW, &param, |state| state.tx_preview(&param))
    }
}

/// `tx.apply` tool: atomic capability-gated apply.
pub struct TxApplyTool;

impl ToolBase for TxApplyTool {
    type Parameter = TxApplyArgs;
    type Output = TxApplyOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_TX_APPLY.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Atomically apply a previewed batch through the capability gate. \
             Requires the tx.preview approval; idempotency-keyed and revision-checked. \
             Denied, stale or duplicate calls write nothing."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        write_annotations(true)
    }
}

impl SyncTool<McpServer> for TxApplyTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        service.dispatch_screened(TOOL_TX_APPLY, &param, |state| state.tx_apply(&param))
    }
}

/// `tx.undo` tool: revert the newest batch as a new revision.
pub struct TxUndoTool;

impl ToolBase for TxUndoTool {
    type Parameter = TxUndoArgs;
    type Output = TxUndoOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_TX_UNDO.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Undo the newest journaled batch as a new revision. \
             A given undo_token must match the newest batch key."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        write_annotations(false)
    }
}

impl SyncTool<McpServer> for TxUndoTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        service.dispatch_screened(TOOL_TX_UNDO, &param, |state| state.tx_undo(&param))
    }
}

/// `sim.build_encounter` args: brief text plus seed.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SimBuildEncounterArgs {
    pub brief: String,
    pub seed: u64,
}

/// `sim.build_encounter` result: the landed plan or the refusal set.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SimBuildEncounterOutput {
    pub solved: bool,
    pub plan_id: u64,
    pub rung: u8,
    pub fork_id: u64,
    pub repairs_used: u8,
    pub reason: String,
    pub conflicts: Vec<String>,
    // D19: the checks that judged this build: the bucket first, then one
    // check per line. A plain comment on purpose: a doc comment here would
    // land in the tool list.
    #[serde(default)]
    pub verify: String,
}

/// `sim.build_encounter` handler: brief text to validated encounter plan
/// under the deterministic oracle. Pure and read-only: a refusal rides the
/// output (`solved: false`), only a malformed brief errors.
fn sim_build_encounter(
    service: &McpServer,
    args: &SimBuildEncounterArgs,
) -> Result<SimBuildEncounterOutput, McpToolError> {
    service.dispatch_screened(TOOL_SIM_BUILD_ENCOUNTER, args, |_| {
        let brief = crate::build_encounter::EncounterBrief::new(&args.brief, args.seed).map_err(
            |error| {
                McpToolError::new(
                    "bad_brief",
                    format!(
                        "brief is refused ({}); submit 1..={} chars of brief text",
                        error,
                        crate::build_encounter::BRIEF_MAX_CHARS,
                    ),
                )
            },
        )?;
        let oracle = crate::build_encounter::DeterministicOracle;
        match crate::build_encounter::build_encounter(&brief, &oracle) {
            Ok(built) => {
                // D19: the landed plan answers with the checks that judged it.
                let checks = vec![
                    crate::verify_tasks::check("sim.solved", TOOL_SIM_BUILD_ENCOUNTER, true, ""),
                    crate::verify_tasks::check(
                        "sim.plan_id",
                        TOOL_SIM_BUILD_ENCOUNTER,
                        built.plan_id != 0,
                        "the plan id is zero",
                    ),
                    crate::verify_tasks::check("sim.conflicts", TOOL_SIM_BUILD_ENCOUNTER, true, ""),
                ];
                Ok(SimBuildEncounterOutput {
                    solved: true,
                    plan_id: built.plan_id,
                    rung: built.rung,
                    fork_id: built.fork_id,
                    repairs_used: u8::try_from(built.repairs_used.len()).unwrap_or(u8::MAX),
                    reason: String::new(),
                    conflicts: Vec::new(),
                    verify: crate::verify_tasks::VerifyBlock::of(TOOL_SIM_BUILD_ENCOUNTER, &checks)
                        .wire_report(),
                })
            }
            Err(refusal) => {
                // D19: a refusal answers with the checks that judged it too.
                let checks = vec![
                    crate::verify_tasks::check(
                        "sim.solved",
                        TOOL_SIM_BUILD_ENCOUNTER,
                        false,
                        refusal.to_string(),
                    ),
                    crate::verify_tasks::check(
                        "sim.conflicts",
                        TOOL_SIM_BUILD_ENCOUNTER,
                        refusal.conflicts.is_empty(),
                        format!("{} conflicts", refusal.conflicts.len()),
                    ),
                ];
                Ok(SimBuildEncounterOutput {
                    solved: false,
                    plan_id: 0,
                    rung: 0,
                    fork_id: 0,
                    repairs_used: u8::try_from(refusal.repairs_used.len()).unwrap_or(u8::MAX),
                    reason: refusal.to_string(),
                    conflicts: refusal.conflicts,
                    verify: crate::verify_tasks::VerifyBlock::of(TOOL_SIM_BUILD_ENCOUNTER, &checks)
                        .wire_report(),
                })
            }
        }
    })
}

crate::e2_tool!(
    /// `sim.build_encounter` tool: brief text to validated encounter plan. Read-only.
    SimBuildEncounterTool,
    name = TOOL_SIM_BUILD_ENCOUNTER,
    description = crate::build_encounter::MCP_TOOL_SUMMARY,
    readonly = true,
    args = SimBuildEncounterArgs,
    output = SimBuildEncounterOutput,
    invoke = sim_build_encounter
);

/// `game.build` args: brief text, seed, kit variant and bloodied flag.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct GameBuildArgs {
    pub brief: String,
    pub seed: u64,
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(default)]
    pub bloodied: Option<bool>,
}

/// `game.build` result: the packaged slice proof or the refusal reason.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct GameBuildOutput {
    pub solved: bool,
    pub play_digest: u64,
    pub replay_digest: u64,
    pub ticks_played: u32,
    pub package_bytes: u32,
    pub verdict: String,
    pub reason: String,
    // D19: the checks that judged this build: the bucket first, then one
    // check per line. A plain comment on purpose: a doc comment here would
    // land in the tool list.
    #[serde(default)]
    pub verify: String,
}

/// `game.build` handler: brief text to packaged playable slice (level,
/// character, encounter, five-minute play plus exact replay). Pure and
/// read-only: a refusal rides the output (`solved: false`), only a malformed
/// brief or unknown variant errors.
fn game_build(service: &McpServer, args: &GameBuildArgs) -> Result<GameBuildOutput, McpToolError> {
    service.dispatch_screened(TOOL_GAME_BUILD, args, |_| {
        let variant_name = args.variant.as_deref().unwrap_or("hero");
        let variant = crate::build_character::VariantId::parse(variant_name).map_err(|error| {
            McpToolError::new(
                "bad_variant",
                format!(
                    "variant `{variant_name}` is refused ({error}); re-run with one of \
                     [hero, opp_male_light, opp_female_medium, opp_female_light]",
                ),
            )
        })?;
        let slice = crate::build_game::build_slice(
            &args.brief,
            args.seed,
            variant,
            args.bloodied.unwrap_or(false),
        )
        .map_err(|error| {
            McpToolError::new(
                error.code,
                format!("slice refused ({error}); fix the brief and re-run"),
            )
        })?;
        // D19: the packaged slice answers with the checks that judged it.
        let checks = vec![
            crate::verify_tasks::check("game.solved", TOOL_GAME_BUILD, true, ""),
            crate::verify_tasks::check(
                "game.replay_digest",
                TOOL_GAME_BUILD,
                slice.eval.replay_digest != 0,
                "the replay digest is zero",
            ),
            crate::verify_tasks::check(
                "game.ticks",
                TOOL_GAME_BUILD,
                slice.eval.ticks_played > 0,
                "no tick was played",
            ),
        ];
        Ok(GameBuildOutput {
            solved: true,
            play_digest: slice.play_digest,
            replay_digest: slice.eval.replay_digest,
            ticks_played: slice.eval.ticks_played,
            package_bytes: slice.eval.package_bytes,
            verdict: slice.eval.verdict.clone(),
            reason: String::new(),
            verify: crate::verify_tasks::VerifyBlock::of(TOOL_GAME_BUILD, &checks).wire_report(),
        })
    })
}

crate::e2_tool!(
    /// `game.build` tool: brief text to packaged playable slice. Read-only.
    GameBuildTool,
    name = TOOL_GAME_BUILD,
    description = crate::build_game::TOOL_SUMMARY,
    readonly = true,
    args = GameBuildArgs,
    output = GameBuildOutput,
    invoke = game_build
);

// ---------------------------------------------------------------------------
// Server: negotiated protocol version, core router plus packs, stdio runner.
// ---------------------------------------------------------------------------

/// schemars emits `u64` fields as `{"type": "integer", "format": "uint64"}`;
/// some clients log `unknown format uint64 ignored` per tool. Rewrite that
/// emit to plain `integer` so serve.log stays quiet.
fn sanitize_schema_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            // AX15 surface trim (2026-10-17): every advertised schema repeated
            // the same 57-char `$schema` dialect key, 2622 chars on the loaded
            // list. MCP clients read tool schemas in the protocol's own
            // dialect, so the key carried no information a caller needs; the
            // door drops it and spends the room on verbs instead of
            // boilerplate. No field, verb or budget changes.
            map.remove("$schema");
            for (key, entry) in map.iter_mut() {
                if (key == "format" || key == "type")
                    && entry.as_str().is_some_and(|format| format == "uint64")
                {
                    *entry = Value::String("integer".to_owned());
                } else {
                    sanitize_schema_value(entry);
                }
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                sanitize_schema_value(item);
            }
        }
        _ => {}
    }
}

/// Trim one advertised tool's schemas to what a caller needs: the `uint64`
/// emit becomes plain `integer`, and the repeated `$schema` dialect key is
/// dropped (see [`sanitize_schema_value`]). Both shrink the ledger the
/// discovery budget counts, without changing one field.
pub(super) fn sanitize_tool_schema(tool: &mut rmcp::model::Tool) {
    let mut input = Value::Object((*tool.input_schema).clone());
    sanitize_schema_value(&mut input);
    if let Value::Object(map) = input {
        tool.input_schema = Arc::new(map);
    }
    if let Some(output) = tool.output_schema.as_mut() {
        let mut replacement = Value::Object((**output).clone());
        sanitize_schema_value(&mut replacement);
        if let Value::Object(map) = replacement {
            *output = Arc::new(map);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::Kind;
    use engine_editor::capability::Capability;

    #[test]
    fn verify_task_the_mcp_build_verbs_answer_their_reports() {
        let server = McpServer::new(97, Capability::unrestricted(97));
        let encounter = SimBuildEncounterTool::invoke(
            &server,
            SimBuildEncounterArgs {
                brief: "a training duel for the new recruits".to_owned(),
                seed: 2040,
            },
        )
        .expect("the encounter brief lands");
        assert!(encounter.solved, "{}", encounter.reason);
        assert!(
            encounter
                .verify
                .starts_with("instance=sim.build_encounter\nbucket=resolved\n"),
            "{}",
            encounter.verify
        );
        assert!(
            encounter.verify.contains("buckets:"),
            "{}",
            encounter.verify
        );

        let game = GameBuildTool::invoke(
            &server,
            GameBuildArgs {
                brief: "prop brazier slice_01\nwant slice_01 next_to wall:north [0,80]\n\
                        spawn slice_01\n# a quiet duel at dawn in the training yard"
                    .to_owned(),
                seed: 5000,
                variant: None,
                bloodied: None,
            },
        )
        .expect("the slice brief lands");
        assert!(game.solved, "{}", game.reason);
        assert!(
            game.verify
                .starts_with("instance=game.build\nbucket=resolved\n"),
            "{}",
            game.verify
        );
        assert!(game.verify.contains("buckets:"), "{}", game.verify);
    }

    #[test]
    fn a_refused_tool_call_reaches_the_client_as_a_record() {
        // DOOR6: a real tools/call refusal crosses the boundary as the typed
        // record, not only a sentence: kind, retryable, suggestions and next
        // ride the payload beside the code, message and data.
        let server = McpServer::new(97, Capability::unrestricted(97));
        let refused = server
            .dispatch("no.such_verb", |_| Ok::<(), McpToolError>(()))
            .expect_err("an unknown verb refuses");
        assert_eq!(refused.code, "unknown_tool");
        assert_eq!(refused.kind(), Kind::Input);
        assert!(!refused.retryable());
        let envelope = rmcp::ErrorData::from(refused);
        let payload = envelope
            .data
            .as_ref()
            .and_then(Value::as_object)
            .expect("the refusal payload is an object");
        assert_eq!(payload["code"], "unknown_tool");
        assert_eq!(payload["kind"], "input");
        assert_eq!(payload["retryable"], false);
        assert_eq!(
            payload["suggestions"][0],
            "api.search the name you meant; every reachable verb is indexed by name"
        );
        assert_eq!(payload["next"]["verb"], "api.search");
    }
}
