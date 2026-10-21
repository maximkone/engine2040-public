//! world.query: the Bevy Remote Protocol filter shape over the door document.
//! Owner: endpoint. Consumer: agents trained on BRP, which already know
//! `with`, `without`, `components`, `has` and `strict`.
//! Read-only: the document, journal, budget and ledger never move.
//! Determinism: hits follow the document's id-sorted table, so the same
//! document and args give byte-identical output. Errors are typed and name
//! the object and the component kind.

use std::borrow::Cow;
use std::collections::BTreeMap;

use engine_scene::component::{Component, ComponentKind};
use rmcp::handler::server::router::tool::{SyncTool, ToolBase};
use rmcp::model::ToolAnnotations;
use rmcp::schemars::{self, JsonSchema};
use rmcp::serde::{Deserialize, Serialize};

use super::mcp::{McpServer, McpToolError};

/// Door tool name.
pub const TOOL_WORLD_QUERY: &str = "world.query";
/// Default hit cap.
pub const WORLD_QUERY_DEFAULT_LIMIT: usize = 64;
/// Hard hit cap.
pub const WORLD_QUERY_MAX_HITS: usize = 256;

/// The `data` half of a BRP `world.query`: what to return.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(crate = "rmcp::serde")]
pub struct QueryData {
    /// Components a hit must carry and that are returned.
    #[serde(default)]
    pub components: Vec<String>,
    /// Components returned when present, never required.
    #[serde(default)]
    pub option: Vec<String>,
    /// Components reported as a boolean per hit.
    #[serde(default)]
    pub has: Vec<String>,
}

/// The `filter` half of a BRP `world.query`: which objects to visit.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(crate = "rmcp::serde")]
pub struct QueryFilter {
    /// Component names every hit must carry.
    #[serde(default)]
    pub with: Vec<String>,
    /// Component names a hit must not carry.
    #[serde(default)]
    pub without: Vec<String>,
}

/// BRP `world.query` params: nested `data` and `filter`, plus strict and limit.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(crate = "rmcp::serde")]
pub struct WorldQueryArgs {
    /// What to return.
    #[serde(default)]
    pub data: QueryData,
    /// Which objects to visit.
    #[serde(default)]
    pub filter: QueryFilter,
    /// Refuse the whole answer when a hit lacks a named `data.components`
    /// kind, instead of skipping it.
    #[serde(default)]
    pub strict: bool,
    /// Hit cap; clamped to [`WORLD_QUERY_MAX_HITS`].
    #[serde(default)]
    pub limit: Option<u32>,
}

/// One object that passed the filter.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(crate = "rmcp::serde")]
pub struct WorldHit {
    /// Node id text, as the document keys it.
    pub id: String,
    /// Component kinds present on the node, name order.
    pub components: Vec<String>,
    /// One boolean per requested `has` kind.
    pub has: BTreeMap<String, bool>,
}

/// Answer shape.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(crate = "rmcp::serde")]
pub struct WorldQueryOutput {
    /// Document revision the answer was read at.
    pub revision: u64,
    /// Hits returned.
    pub count: u32,
    /// True when the cap stopped the scan.
    pub truncated: bool,
    /// Hits, id order.
    pub hits: Vec<WorldHit>,
}

fn kinds(field: &'static str, names: &[String]) -> Result<Vec<ComponentKind>, McpToolError> {
    names
        .iter()
        .map(|name| {
            ComponentKind::from_name(name).ok_or_else(|| {
                McpToolError::new(
                    "unknown_component",
                    format!(
                        "{field} names component `{name}`, which is not a kind; legal: {}",
                        ComponentKind::name_list()
                    ),
                )
                .with_data(rmcp::serde_json::json!({
                    "field": field,
                    "component": name,
                    "legal": ComponentKind::name_list(),
                }))
            })
        })
        .collect()
}

/// Pure filter over one component table. Exposed for tests and for any
/// caller that holds a table without a server.
pub fn query(
    components: &BTreeMap<String, BTreeMap<ComponentKind, Component>>,
    revision: u64,
    args: &WorldQueryArgs,
) -> Result<WorldQueryOutput, McpToolError> {
    let with = kinds("with", &args.filter.with)?;
    let without = kinds("without", &args.filter.without)?;
    let want = kinds("components", &args.data.components)?;
    let option = kinds("option", &args.data.option)?;
    let has = kinds("has", &args.data.has)?;
    let requested: Vec<ComponentKind> = want.iter().chain(option.iter()).copied().collect();
    let limit = args
        .limit
        .map(|n| usize::try_from(n).unwrap_or(usize::MAX))
        .unwrap_or(WORLD_QUERY_DEFAULT_LIMIT)
        .min(WORLD_QUERY_MAX_HITS);

    let mut hits = Vec::new();
    let mut truncated = false;
    for (id, table) in components {
        if !with.iter().all(|kind| table.contains_key(kind)) {
            continue;
        }
        if without.iter().any(|kind| table.contains_key(kind)) {
            continue;
        }
        if args.strict {
            if let Some(missing) = want.iter().find(|kind| !table.contains_key(*kind)) {
                return Err(McpToolError::new(
                    "missing_component",
                    format!(
                        "object `{id}` has no `{}` component (strict)",
                        missing.name()
                    ),
                )
                .with_data(rmcp::serde_json::json!({
                    "id": id,
                    "component": missing.name(),
                })));
            }
        }
        if hits.len() == limit {
            truncated = true;
            break;
        }
        hits.push(WorldHit {
            id: id.clone(),
            components: if requested.is_empty() {
                table.keys().map(|kind| kind.name().to_owned()).collect()
            } else {
                requested
                    .iter()
                    .filter(|kind| table.contains_key(*kind))
                    .map(|kind| kind.name().to_owned())
                    .collect()
            },
            has: has
                .iter()
                .map(|kind| (kind.name().to_owned(), table.contains_key(kind)))
                .collect(),
        });
    }
    Ok(WorldQueryOutput {
        revision,
        count: u32::try_from(hits.len()).unwrap_or(u32::MAX),
        truncated,
        hits,
    })
}

impl super::mcp::ServerState {
    /// Answer one `world.query` over the staged document.
    pub fn world_query(&self, args: &WorldQueryArgs) -> Result<WorldQueryOutput, McpToolError> {
        query(&self.doc.components, self.doc.revision, args)
    }
}

/// The door tool: BRP `world.query` over the staged document.
pub struct WorldQueryTool;

impl ToolBase for WorldQueryTool {
    type Parameter = WorldQueryArgs;
    type Output = WorldQueryOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_WORLD_QUERY.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Bevy Remote Protocol filter over the scene: nested data and filter \
             (components, option, has, with, without) plus strict and limit. \
             Read-only; returns ids and component names."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        crate::mcp::read_only_annotations()
    }
}

impl SyncTool<McpServer> for WorldQueryTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        service.dispatch(TOOL_WORLD_QUERY, |state| state.world_query(&param))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(
        rows: &[(&str, &[(ComponentKind, &str)])],
    ) -> BTreeMap<String, BTreeMap<ComponentKind, Component>> {
        let mut out = BTreeMap::new();
        for (id, kinds) in rows {
            let mut node = BTreeMap::new();
            for (kind, payload) in *kinds {
                let value = Component::from_json(*kind, payload).expect("fixture json");
                node.insert(*kind, value);
            }
            out.insert((*id).to_owned(), node);
        }
        out
    }

    const MESH: &str = r#"{"asset":"crate.gltf","cast_shadows":true}"#;
    const LIGHT: &str = r#"{"kind":"point","color":[1,0,0],"intensity":2,"range":5}"#;
    const COLLIDER: &str = r#"{"shape":"box","half":[0.5,0.5,0.5],"sensor":false}"#;

    fn scene() -> BTreeMap<String, BTreeMap<ComponentKind, Component>> {
        table(&[
            (
                "crate",
                &[
                    (ComponentKind::Mesh, MESH),
                    (ComponentKind::Collider, COLLIDER),
                ],
            ),
            ("torch", &[(ComponentKind::Light, LIGHT)]),
            ("statue", &[(ComponentKind::Mesh, MESH)]),
        ])
    }

    fn args(with: &[&str], without: &[&str], components: &[&str], strict: bool) -> WorldQueryArgs {
        WorldQueryArgs {
            data: QueryData {
                components: components.iter().map(|s| (*s).to_owned()).collect(),
                option: Vec::new(),
                has: vec!["light".to_owned()],
            },
            filter: QueryFilter {
                with: with.iter().map(|s| (*s).to_owned()).collect(),
                without: without.iter().map(|s| (*s).to_owned()).collect(),
            },
            strict,
            limit: None,
        }
    }

    #[test]
    fn with_filters_to_nodes_carrying_every_named_kind() {
        let out = query(&scene(), 7, &args(&["mesh", "collider"], &[], &[], false)).unwrap();
        assert_eq!(out.revision, 7);
        assert_eq!(out.hits.len(), 1);
        assert_eq!(out.hits[0].id, "crate");
        assert_eq!(out.hits[0].components, vec!["mesh", "collider"]);
        assert!(!out.hits[0].has["light"]);
    }

    #[test]
    fn without_excludes_the_named_kind() {
        let out = query(&scene(), 0, &args(&[], &["light"], &[], false)).unwrap();
        let ids: Vec<&str> = out.hits.iter().map(|hit| hit.id.as_str()).collect();
        assert_eq!(ids, vec!["crate", "statue"]);
        assert_eq!(out.count, 2);
    }

    #[test]
    fn strict_refuses_missing_component_and_names_it() {
        let err = query(&scene(), 0, &args(&["mesh"], &[], &["collider"], true)).unwrap_err();
        assert_eq!(err.code(), "missing_component");
        assert!(
            err.to_string().contains("statue"),
            "names the object: {err}"
        );
        assert!(
            err.to_string().contains("collider"),
            "names the kind: {err}"
        );
    }

    #[test]
    fn bevy_shaped_nested_request_runs_unchanged() {
        let json = r#"{"data":{"components":["mesh"],"option":[],"has":["light"]},
            "filter":{"with":["mesh"],"without":["collider","light"]},"strict":true,"limit":8}"#;
        let shaped: WorldQueryArgs = rmcp::serde_json::from_str(json).expect("brp json");
        let out = query(&scene(), 3, &shaped).unwrap();
        assert_eq!(out.revision, 3);
        assert_eq!(out.hits.len(), 1);
        assert_eq!(out.hits[0].id, "statue");
        assert_eq!(out.hits[0].components, vec!["mesh"]);
        assert!(!out.hits[0].has["light"]);
    }

    #[test]
    fn unknown_component_is_refused_with_the_legal_list() {
        let err = query(&scene(), 0, &args(&["banana"], &[], &[], false)).unwrap_err();
        assert_eq!(err.code(), "unknown_component");
        let message = err.to_string();
        assert!(message.contains("banana"), "{message}");
        assert!(message.contains("mesh"), "{message}");
    }

    #[test]
    fn limit_truncates_and_the_same_call_gives_the_same_bytes() {
        let mut a = args(&[], &[], &[], false);
        a.limit = Some(1);
        let first = query(&scene(), 0, &a).unwrap();
        let second = query(&scene(), 0, &a).unwrap();
        assert_eq!(first.count, 1);
        assert!(first.truncated, "the cap stopped the scan");
        let one = rmcp::serde_json::to_string(&first).unwrap();
        let two = rmcp::serde_json::to_string(&second).unwrap();
        assert_eq!(one, two);
    }
}
