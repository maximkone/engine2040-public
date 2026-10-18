//! `scene.next`: the one next call a weak model should make.
//!
//! Consumer: any MCP client that does not know which scene verb to call
//! after the last one; the `read_repeat` refusal of the scene reads points
//! here. Data owner: the staged document, read only. The answer is a pure
//! function of the object count, the spatial verdict and finding total, the
//! revision, the brief digest and the worst actor, so the same scene gives
//! the same bytes. Units: counts, document revision. Errors: none beyond the
//! reads it makes (a poisoned lock). Budget: the reply stays under 600 bytes.

use std::borrow::Cow;

use rmcp::handler::server::router::tool::{SyncTool, ToolBase};
use rmcp::model::ToolAnnotations;
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use super::mcp::{McpServer, McpToolError, ServerState};
use super::scene_tools::{snapshot, SceneSpatialArgs};

/// Door tool name for the next-call planner.
pub const TOOL_SCENE_NEXT: &str = "scene.next";
/// Scenes with at least this many objects read compactly.
pub const LARGE_SCENE: usize = 100;
/// Findings up to this many read as a few.
const FEW_FINDINGS: u64 = 2;

/// Arguments of `scene.next`: none.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "serde reads the wire shape `{}`; a unit struct reads null"
)]
pub struct SceneNextArgs {}

/// One door call to make.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct NextCall {
    /// Verb to call.
    pub verb: String,
    /// Its arguments as JSON text.
    pub args: String,
}

/// Output of `scene.next`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SceneNextOutput {
    /// Where the scene stands: empty, clean, warn, red_few or red_many.
    pub stage: String,
    /// The one call to make now.
    pub next_call: NextCall,
    /// What to read after it to confirm it worked.
    pub verify: String,
    /// When to stop.
    pub stop_conditions: Vec<String>,
    /// What not to do.
    pub do_not: Vec<String>,
}

/// What the plan reads from the scene.
pub struct NextInputs<'a> {
    /// Staged objects.
    pub objects: usize,
    /// Spatial verdict: `clean`, `warn` or `red`.
    pub verdict: &'a str,
    /// Findings across all pages.
    pub total: u64,
    /// Document revision.
    pub revision: u64,
    /// Brief digest.
    pub digest: u64,
    /// Label of the actor with the most findings, if any.
    pub worst: Option<&'a str>,
}

/// The plan for a scene state: one stage, one call. Pure.
pub fn plan(input: &NextInputs<'_>) -> SceneNextOutput {
    let large = input.objects >= LARGE_SCENE;
    let (stage, verb, args) = if input.objects == 0 {
        (
            "empty",
            "api.describe",
            r#"{"path":"tx.preview"}"#.to_owned(),
        )
    } else if input.verdict == "clean" {
        (
            "clean",
            "play.run",
            format!(
                r#"{{"action":"start","seed":2040,"ticks":60,"fighters":2,"half_extent_m":20.0,"scene_rev":{}}}"#,
                input.revision
            ),
        )
    } else if input.verdict != "red" {
        let format = if large { r#","format":"ids""# } else { "" };
        ("warn", "scene.spatial", format!(r#"{{"limit":8{format}}}"#))
    } else if input.total > FEW_FINDINGS {
        (
            "red_many",
            "scene.spatial",
            r#"{"matched_only":true,"format":"ids"}"#.to_owned(),
        )
    } else {
        match input.worst {
            Some(label) if !large => (
                "red_few",
                "scene.query",
                format!(r#"{{"kind":"near","anchor":"{label}","radius_cm":300}}"#),
            ),
            _ => (
                "red_few",
                "scene.spatial",
                r#"{"matched_only":true,"limit":5}"#.to_owned(),
            ),
        }
    };
    SceneNextOutput {
        stage: stage.to_owned(),
        next_call: NextCall {
            verb: verb.to_owned(),
            args,
        },
        verify: format!(
            "scene.brief: digest {} at revision {}; it must move or findings must drop",
            input.digest, input.revision
        ),
        stop_conditions: vec![
            "scene.spatial verdict is clean".to_owned(),
            "revision and digest unchanged after the call".to_owned(),
        ],
        do_not: vec![
            "do not raise a threshold to hide a finding".to_owned(),
            "do not repeat the same read".to_owned(),
        ],
    }
}

impl ServerState {
    /// Plan the next call from the staged scene. Read-only.
    pub fn scene_next(&self) -> Result<SceneNextOutput, McpToolError> {
        let spatial = self.scene_spatial(&SceneSpatialArgs {
            format: Some("count".to_owned()),
            ..Default::default()
        })?;
        let objects = snapshot(&self.doc).ids.len();
        let digest = self
            .scene_brief(&Default::default())
            .map(|brief| brief.digest)?;
        let worst = spatial.worst_actors.first().map(|actor| actor.id.as_str());
        Ok(plan(&NextInputs {
            objects,
            verdict: &spatial.verdict,
            total: spatial.total,
            revision: self.doc.revision,
            digest,
            worst,
        }))
    }
}

/// MCP tool registration for `scene.next` (read-only planner).
pub struct SceneNextTool;

impl ToolBase for SceneNextTool {
    type Parameter = SceneNextArgs;
    type Output = SceneNextOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_SCENE_NEXT.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Names the one next call for this scene, how to verify it and when to stop. Read-only."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        crate::mcp::read_only_annotations()
    }
}

impl SyncTool<McpServer> for SceneNextTool {
    fn invoke(service: &McpServer, _param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        service.dispatch(TOOL_SCENE_NEXT, |state| state.scene_next())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(objects: usize, verdict: &'static str, total: u64) -> NextInputs<'static> {
        NextInputs {
            objects,
            verdict,
            total,
            revision: 12,
            digest: 9_876_543_210,
            worst: Some("crate_1 [00000000-0000-0000-0000-000000000001]"),
        }
    }

    /// Twelve scene states give exactly one next call each, the same bytes
    /// twice, a reply under 600 bytes, and the standing do-not line.
    #[test]
    fn scene_next_twelve_states_one_call_each() {
        let states = [
            inputs(0, "clean", 0),
            inputs(5, "clean", 0),
            inputs(500, "clean", 0),
            inputs(5, "warn", 3),
            inputs(500, "warn", 40),
            inputs(5, "red", 1),
            inputs(5, "red", 2),
            inputs(500, "red", 2),
            inputs(5, "red", 3),
            inputs(500, "red", 400),
            inputs(99, "red", 1),
            inputs(100, "warn", 1),
        ];
        let mut stages = std::collections::BTreeSet::new();
        for state in &states {
            let out = plan(state);
            assert_eq!(out, plan(state), "same state, same plan");
            let wire = rmcp::serde_json::to_string(&out).expect("plan serializes");
            assert!(wire.len() <= 600, "{} bytes: {wire}", wire.len());
            assert!(!out.next_call.verb.is_empty());
            rmcp::serde_json::from_str::<rmcp::serde_json::Value>(&out.next_call.args)
                .expect("next call args are JSON");
            assert!(out
                .do_not
                .contains(&"do not raise a threshold to hide a finding".to_owned()));
            assert!(out.verify.contains("9876543210"));
            stages.insert(out.stage);
        }
        assert_eq!(stages.len(), 5, "empty clean warn red_few red_many");
    }

    /// The door verb plans from the live scene: an empty scene starts with
    /// the transaction schema, and the verb is listed in the scene pack.
    #[test]
    fn scene_next_door_reads_the_live_scene() {
        use engine_editor::capability::Capability;
        let server = McpServer::new(7, Capability::unrestricted(7));
        let out = SceneNextTool::invoke(&server, SceneNextArgs {}).expect("plans");
        assert_eq!(out.stage, "empty");
        assert_eq!(out.next_call.verb, "api.describe");
        assert!(crate::mcp::tool_names().contains(&TOOL_SCENE_NEXT));
    }
}
