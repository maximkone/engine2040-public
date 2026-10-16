//! Insight MCP tools: `api.search`, `api.describe`, `code.callers`,
//! `asset.refs`. Read-only over the [`Index`](super::Index) held by the
//! session; field docs stay off the DTOs to protect the tool-list budget.

use std::borrow::Cow;

use rmcp::handler::server::router::tool::{SyncTool, ToolBase};
use rmcp::model::ToolAnnotations;
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use super::graph::{asset_refs, callers_of};
use super::index::{error_code, InsightError};
use super::search::{self, SEARCH_DEFAULT_LIMIT};
use crate::mcp::{McpServer, McpToolError, ServerState};

/// Tool: BM25 search over the engine index.
pub const TOOL_API_SEARCH: &str = "api.search";
/// Tool: full entry for one indexed path.
pub const TOOL_API_DESCRIBE: &str = "api.describe";
/// Tool: reverse lookup of mentioning items.
pub const TOOL_CODE_CALLERS: &str = "code.callers";
/// Tool: code and scenes touching one asset.
pub const TOOL_ASSET_REFS: &str = "asset.refs";

/// Search request: query words plus an optional hit limit.
/// Units are query chars in, hits out. Errors are typed `empty_query` and
/// `bad_limit` with the legal values. Budget is at most 32 hits per call.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ApiSearchArgs {
    pub query: String,
    #[serde(default)]
    pub limit: Option<u32>,
}

/// One ranked hit: indexed path, kind label, score and one-line brief.
/// Units are chars of brief text; score orders hits, higher is better.
/// No errors: empty results return no rows. Budget is the caller limit.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ApiHitDto {
    pub path: String,
    pub kind: String,
    pub score: f64,
    pub brief: String,
}

/// Search answer: ranked hits plus the total count.
/// Units are hits and bytes of brief text. No errors: an empty index
/// returns no hits. Budget is at most 32 hits per call.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ApiSearchOutput {
    pub hits: Vec<ApiHitDto>,
    pub total: u64,
}

/// Describe request: the canonical indexed path to look up.
/// Units are path chars in, one entry out. Errors are typed `unknown_path`
/// with a search-first recovery. Budget is one entry per call.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ApiDescribeArgs {
    pub path: String,
}

/// Full index entry: signature, docs, units, file owner and callers.
/// Units are chars of signature and docs. No errors once looked up; unknown
/// paths fail at the request. Budget is one entry per call.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ApiDescribeOutput {
    pub path: String,
    pub kind: String,
    pub signature: String,
    pub docs: String,
    pub units: String,
    pub file: String,
    pub owner: String,
    pub callers: Vec<String>,
}

/// Callers request: the canonical indexed path to reverse-lookup.
/// Units are path chars in, caller paths out. Errors are typed
/// `unknown_path`. Budget is one lookup per call.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct CodeCallersArgs {
    pub path: String,
}

/// Callers answer: the queried path plus every mentioning item.
/// Units are paths and counts. No errors once looked up. Budget is one
/// caller list per call.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct CodeCallersOutput {
    pub path: String,
    pub callers: Vec<String>,
    pub total: u64,
}

/// Asset refs request: the asset stem to find in code and scenes.
/// Units are asset path chars in, referencing paths out. Errors are typed
/// for unknown stems. Budget is one lookup per call.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct AssetRefsArgs {
    pub asset: String,
}

/// Asset refs answer: the queried asset plus every touching path.
/// Units are paths and counts. No errors once looked up. Budget is one
/// reference list per call.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct AssetRefsOutput {
    pub asset: String,
    pub refs: Vec<String>,
    pub total: u64,
}

/// Map an index failure to a tool error with the legal values attached.
fn tool_error(error: &InsightError) -> McpToolError {
    McpToolError::new(error_code(error), error.to_string())
}

impl ServerState {
    /// Ranked index search: blank queries and limits outside 1..=32 fail
    /// typed; an empty index simply returns no hits.
    pub fn api_search(&self, args: &ApiSearchArgs) -> Result<ApiSearchOutput, McpToolError> {
        let limit = args.limit.unwrap_or(SEARCH_DEFAULT_LIMIT);
        let hits = search::search(&self.insight, &args.query, limit)
            .map_err(|error| tool_error(&error))?;
        Ok(ApiSearchOutput {
            total: hits.len() as u64,
            hits: hits
                .into_iter()
                .map(|hit| ApiHitDto {
                    path: hit.path,
                    kind: hit.kind.to_owned(),
                    score: hit.score,
                    brief: hit.brief,
                })
                .collect(),
        })
    }

    /// Full entry for one path: signature, docs, units, file owner and
    /// callers. Unknown paths fail typed with a search-first recovery.
    /// Rival-door aliases (`godot_game_time` etc.) resolve to their
    /// play-pack verb first, so another engine's door names still land.
    pub fn api_describe(&self, args: &ApiDescribeArgs) -> Result<ApiDescribeOutput, McpToolError> {
        // LF16 rival-door parity: resolve through the play-pack table before
        // the index lookup. Unknown rivals fall through to the typed refusal.
        let lookup: &str =
            crate::packs::play::verb_for_rival(&args.path).unwrap_or(args.path.as_str());
        let item = self.insight.get(lookup).ok_or_else(|| {
            tool_error(&InsightError::UnknownPath {
                path: args.path.clone(),
            })
        })?;
        let callers = callers_of(&self.insight, lookup).map_err(|error| tool_error(&error))?;
        Ok(ApiDescribeOutput {
            path: item.path.clone(),
            kind: item.kind.as_str().to_owned(),
            signature: item.signature.clone(),
            docs: item.docs.clone(),
            units: item.units.clone(),
            file: item.file.clone(),
            owner: item.owner.clone(),
            callers,
        })
    }

    /// Reverse lookup: items whose file mentions the path's name.
    pub fn code_callers(&self, args: &CodeCallersArgs) -> Result<CodeCallersOutput, McpToolError> {
        let callers = callers_of(&self.insight, &args.path).map_err(|error| tool_error(&error))?;
        Ok(CodeCallersOutput {
            path: args.path.clone(),
            total: callers.len() as u64,
            callers,
        })
    }

    /// Code and scenes touching one asset stem.
    pub fn asset_refs(&self, args: &AssetRefsArgs) -> Result<AssetRefsOutput, McpToolError> {
        let refs = asset_refs(&self.insight, &args.asset).map_err(|error| tool_error(&error))?;
        Ok(AssetRefsOutput {
            asset: args.asset.clone(),
            total: refs.len() as u64,
            refs,
        })
    }
}

/// Read-only annotations shared by all four Insight tools.
fn readonly_annotations() -> Option<ToolAnnotations> {
    let mut annotations = ToolAnnotations::default();
    annotations.read_only_hint = Some(true);
    annotations.open_world_hint = Some(false);
    Some(annotations)
}

/// `api.search` tool: ranked index search, read-only.
pub struct ApiSearchTool;

impl ToolBase for ApiSearchTool {
    type Parameter = ApiSearchArgs;
    type Output = ApiSearchOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_API_SEARCH.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some("Search the engine index (API, tools, assets, scenes) with BM25. Read-only.".into())
    }

    fn annotations() -> Option<ToolAnnotations> {
        readonly_annotations()
    }
}

impl SyncTool<McpServer> for ApiSearchTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        let payload = rmcp::serde_json::to_string(&param).map_err(|error| {
            McpToolError::new("bad_payload", format!("payload encode failed: {error}"))
        })?;
        service.dispatch_with_payload(TOOL_API_SEARCH, &payload, |state| state.api_search(&param))
    }
}

/// `api.describe` tool: one full index entry, read-only.
pub struct ApiDescribeTool;

impl ToolBase for ApiDescribeTool {
    type Parameter = ApiDescribeArgs;
    type Output = ApiDescribeOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_API_DESCRIBE.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some("Describe one indexed path: signature, docs, units, owner, callers. Read-only.".into())
    }

    fn annotations() -> Option<ToolAnnotations> {
        readonly_annotations()
    }
}

impl SyncTool<McpServer> for ApiDescribeTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        let payload = rmcp::serde_json::to_string(&param).map_err(|error| {
            McpToolError::new("bad_payload", format!("payload encode failed: {error}"))
        })?;
        service.dispatch_with_payload(TOOL_API_DESCRIBE, &payload, |state| {
            state.api_describe(&param)
        })
    }
}

/// `code.callers` tool: reverse reference lookup, read-only.
pub struct CodeCallersTool;

impl ToolBase for CodeCallersTool {
    type Parameter = CodeCallersArgs;
    type Output = CodeCallersOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_CODE_CALLERS.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some("List items mentioning one path: the reverse callers graph. Read-only.".into())
    }

    fn annotations() -> Option<ToolAnnotations> {
        readonly_annotations()
    }
}

impl SyncTool<McpServer> for CodeCallersTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        let payload = rmcp::serde_json::to_string(&param).map_err(|error| {
            McpToolError::new("bad_payload", format!("payload encode failed: {error}"))
        })?;
        service.dispatch_with_payload(TOOL_CODE_CALLERS, &payload, |state| {
            state.code_callers(&param)
        })
    }
}

/// `asset.refs` tool: asset reference lookup, read-only.
pub struct AssetRefsTool;

impl ToolBase for AssetRefsTool {
    type Parameter = AssetRefsArgs;
    type Output = AssetRefsOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_ASSET_REFS.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some("List code and scenes touching one asset. Read-only.".into())
    }

    fn annotations() -> Option<ToolAnnotations> {
        readonly_annotations()
    }
}

impl SyncTool<McpServer> for AssetRefsTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        let payload = rmcp::serde_json::to_string(&param).map_err(|error| {
            McpToolError::new("bad_payload", format!("payload encode failed: {error}"))
        })?;
        service.dispatch_with_payload(TOOL_ASSET_REFS, &payload, |state| state.asset_refs(&param))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_editor::capability::Capability;

    fn seeded_server() -> McpServer {
        let server = McpServer::new(7, Capability::unrestricted(7));
        let mut state = server.lock().expect("test lock holds");
        state.insight = super::super::eval::seed_corpus();
        drop(state);
        server
    }

    #[test]
    fn search_describe_callers_refs_roundtrip() {
        let server = seeded_server();
        let found = ApiSearchTool::invoke(
            &server,
            ApiSearchArgs {
                query: "ObjectId stable identity".to_owned(),
                limit: None,
            },
        )
        .expect("search works");
        assert!(!found.hits.is_empty());
        assert_eq!(found.hits[0].path, "engine-scene::ids::ObjectId");

        let entry = ApiDescribeTool::invoke(
            &server,
            ApiDescribeArgs {
                path: "engine-scene::ids::ObjectId".to_owned(),
            },
        )
        .expect("describe works");
        assert_eq!(entry.owner, "SC1");
        assert!(entry
            .callers
            .contains(&"engine-scene::snapshot::snapshot_ids".to_owned()));

        let callers = CodeCallersTool::invoke(
            &server,
            CodeCallersArgs {
                path: "engine-scene::ids::ObjectId".to_owned(),
            },
        )
        .expect("callers works");
        assert_eq!(callers.total as usize, callers.callers.len());

        let refs = AssetRefsTool::invoke(
            &server,
            AssetRefsArgs {
                asset: "content/locomotion/walk.anim".to_owned(),
            },
        )
        .expect("refs works");
        assert!(refs
            .refs
            .contains(&"engine-assets::loader::load_walk".to_owned()));
    }

    #[test]
    fn insight_failures_are_typed() {
        let server = seeded_server();
        let unknown = ApiDescribeTool::invoke(
            &server,
            ApiDescribeArgs {
                path: "missing::Thing".to_owned(),
            },
        )
        .expect_err("unknown path must fail");
        assert_eq!(unknown.code, "unknown_path");

        let blank = ApiSearchTool::invoke(
            &server,
            ApiSearchArgs {
                query: "  ".to_owned(),
                limit: None,
            },
        )
        .expect_err("blank query must fail");
        assert_eq!(blank.code, "empty_query");

        let over = ApiSearchTool::invoke(
            &server,
            ApiSearchArgs {
                query: "scene".to_owned(),
                limit: Some(99),
            },
        )
        .expect_err("over-limit must fail");
        assert_eq!(over.code, "bad_limit");
    }

    #[test]
    fn empty_index_answers_without_failing() {
        // A fresh server holds only the door's own pack and verb entries:
        // a word none of them carry misses without failing.
        let server = McpServer::new(7, Capability::unrestricted(7));
        let found = ApiSearchTool::invoke(
            &server,
            ApiSearchArgs {
                query: "quaternion".to_owned(),
                limit: None,
            },
        )
        .expect("sparse index searches");
        assert!(found.hits.is_empty());
        let missing = CodeCallersTool::invoke(
            &server,
            CodeCallersArgs {
                path: "scene::Gone".to_owned(),
            },
        )
        .expect_err("empty index has no paths");
        assert_eq!(missing.code, "unknown_path");
    }

    #[test]
    fn rival_alias_describe_resolves_to_play_pack_verb() {
        // LF16 rival-door parity: `godot_game_time` etc. read as their
        // play-pack verb, so a model holding another engine's door names
        // still lands on our verbs. Caller: `ServerState::api_describe`
        // (resolves via `crate::packs::play::verb_for_rival`).
        let server = McpServer::new(7, Capability::unrestricted(7));
        for (rival, ours) in [
            ("godot_game_time", "play.step_until"),
            ("godot_input", "input.inject"),
            ("godot_runtime_state", "state.digest"),
        ] {
            let described = ApiDescribeTool::invoke(
                &server,
                ApiDescribeArgs {
                    path: rival.to_owned(),
                },
            )
            .unwrap_or_else(|error| panic!("rival `{rival}` resolves to `{ours}`: {error}"));
            assert_eq!(described.path, ours, "rival `{rival}` reads as `{ours}`");
            assert!(!described.docs.is_empty(), "rival `{rival}` carries docs");
        }
        let unknown = ApiDescribeTool::invoke(
            &server,
            ApiDescribeArgs {
                path: "godot_unknown".to_owned(),
            },
        )
        .expect_err("unknown rival must fail");
        assert_eq!(unknown.code, "unknown_path");
    }
}
