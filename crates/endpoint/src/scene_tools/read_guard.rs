//! SPF12 scene-read guard: the `include_spatial` summary type and the
//! per-session identical-read counter the scene read tools call.

use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use super::super::mcp::{McpToolError, ServerState};
use super::{TOOL_SCENE_BRIEF, TOOL_SCENE_SPATIAL};

/// Spatial summary riding a `scene.brief` reply (`include_spatial`).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct BriefSpatial {
    /// `clean`, `warn` or `red`: the verdict of a default `scene.spatial` call.
    pub verdict: String,
    /// Findings of that call, across all pages.
    pub total: u64,
    /// Hash of that call's finding set.
    pub hash: u64,
    /// Findings the brief text left out for want of budget.
    pub focus_gap: u64,
}

/// `skip_serializing_if` helper: a first call carries no repeat count.
pub(crate) fn no_repeat(count: &u32) -> bool {
    *count == 0
}

impl ServerState {
    /// SPF12: count one scene read and refuse the fifth identical one (same
    /// arguments, same document revision) in a row inside 30 s with `read_repeat`, which names the verb to try next.
    /// Returns the repeat count for calls two to four, zero for a first call.
    pub(crate) fn note_read<A: Serialize>(
        &mut self,
        tool: &str,
        args: &A,
    ) -> Result<u32, McpToolError> {
        let mut key = engine_core::Fnv64::new();
        key.write(tool.as_bytes());
        key.write(
            rmcp::serde_json::to_string(args)
                .unwrap_or_default()
                .as_bytes(),
        );
        key.write(&self.doc.revision.to_le_bytes());
        match self.reads.note(key.finish()) {
            crate::guards::ReadRepeat::Fresh => Ok(0),
            crate::guards::ReadRepeat::Repeat(count) => Ok(count),
            crate::guards::ReadRepeat::Refuse(count) => {
                let next = match tool {
                    TOOL_SCENE_BRIEF => TOOL_SCENE_SPATIAL,
                    _ => "scene.next",
                };
                Err(McpToolError::new(
                    "read_repeat",
                    format!(
                        "{tool} was called {count} times in a row with the same arguments \
                         and the scene has not changed; call {next} instead, or change an argument"
                    ),
                ))
            }
        }
    }
}
