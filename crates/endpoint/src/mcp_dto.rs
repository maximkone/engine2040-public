//! MCP transport DTOs: serde for the wire, schemars for the advertised schemas.
//!
//! Consumer: `mcp` (re-exported as `crate::mcp::PoseDto` and siblings, so
//! every caller keeps its path). Data owner stays the editor command service;
//! this module owns only the wire shapes. Units follow `engine-scene`: meters
//! for positions. Determinism scope: none at this boundary.
//!
//! Structure-only split from `mcp.rs`: no logic moved. The one wire change is
//! that every `u64` hash, digest and id goes out as a decimal JSON string
//! (see `u64_text`), and input takes either form.

use rmcp::schemars::JsonSchema;
// `serde` itself is in scope too: schemars' `#[serde(default, with)]` expansion names `serde::Serialize`.
use rmcp::serde::{self, Deserialize, Serialize};
use rmcp::serde_json::Value;

use crate::receipts::ChangeList;
use crate::scene_query::diff::SemanticChange;

/// `#[serde(with = "u64_text")]` for a `u64` hash, digest or id. JS and LLM
/// clients parse JSON numbers as f64 and round anything above 2^53 (a real
/// run got `forged_approval` from one such rounding), so the wire carries the
/// exact decimal text. Input takes that string or the old JSON number.
mod u64_text {
    use rmcp::serde::de::{self, Visitor};
    use rmcp::serde::{Deserializer, Serializer};
    use std::fmt;

    /// Write the value as its decimal text.
    pub fn serialize<S: Serializer>(value: &u64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&value.to_string())
    }

    struct DecimalU64;

    impl Visitor<'_> for DecimalU64 {
        type Value = u64;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a u64 as a decimal string or a JSON number")
        }

        fn visit_u64<E: de::Error>(self, value: u64) -> Result<u64, E> {
            Ok(value)
        }

        fn visit_str<E: de::Error>(self, text: &str) -> Result<u64, E> {
            text.parse::<u64>()
                .map_err(|_| E::invalid_value(de::Unexpected::Str(text), &self))
        }
    }

    /// Read a `u64` from a decimal string or a JSON number.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
        d.deserialize_any(DecimalU64)
    }

    /// `Option<u64>` form for `#[serde(with = "u64_text::opt")]`: `null` and a
    /// missing field (with `#[serde(default)]`) are `None`.
    pub mod opt {
        use rmcp::serde::de::{self, Visitor};
        use rmcp::serde::{Deserializer, Serializer};
        use std::fmt;

        /// Write `Some` as the decimal string and `None` as null.
        pub fn serialize<S: Serializer>(value: &Option<u64>, s: S) -> Result<S::Ok, S::Error> {
            match value {
                Some(v) => s.serialize_some(&v.to_string()),
                None => s.serialize_none(),
            }
        }

        struct OptDecimal;

        impl<'de> Visitor<'de> for OptDecimal {
            type Value = Option<u64>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("null, or a u64 as a decimal string or a JSON number")
            }

            fn visit_none<E: de::Error>(self) -> Result<Option<u64>, E> {
                Ok(None)
            }

            fn visit_unit<E: de::Error>(self) -> Result<Option<u64>, E> {
                Ok(None)
            }

            fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Option<u64>, D::Error> {
                super::deserialize(d).map(Some)
            }
        }

        /// Read `null` or a `u64` (decimal string or JSON number).
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
            d.deserialize_option(OptDecimal)
        }
    }
}

// ---------------------------------------------------------------------------
// Transport DTOs: serde for the wire, schemars for the advertised schemas.
// Both derive through rmcp's re-exports, so no new dependency is needed.
// The shapes mirror `engine-editor::commands` one to one; the drift test
// asserts the generated schemas still match the command shapes.
// ---------------------------------------------------------------------------

/// Pose in meters (position), quaternion (rotation) and scale, mirroring
/// [`Transform`](engine_scene::transform::Transform).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PoseDto {
    /// Position in meters, parent frame.
    pub pos: [f32; 3],
    /// Rotation quaternion.
    pub rot: [f32; 4],
    /// Scale factors.
    pub scale: [f32; 3],
}

/// One schema-typed edit, mirroring [`Op`](engine_editor::commands::Op): `id`
/// is UUID text, `parent` is UUID text with the nil id meaning "no parent"
/// (a root).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde", tag = "op", rename_all = "snake_case")]
#[schemars(crate = "rmcp::schemars")]
pub enum OpDto {
    /// Add a fresh node under `parent` (nil for a root).
    Insert {
        /// Fresh id; must not be nil and must not exist yet.
        id: String,
        /// Parent node, or the nil id for a root.
        parent: String,
        /// Pose in meters relative to the parent.
        local: PoseDto,
    },
    /// Delete a node; children relink to its parent.
    Remove {
        /// Node to delete.
        id: String,
    },
    /// Replace a node's local pose.
    SetLocal {
        /// Node to move in its parent frame.
        id: String,
        /// New pose relative to the parent.
        local: PoseDto,
    },
    /// Move a node, keeping its world pose (nil detaches to a root).
    Reparent {
        /// Node to move.
        id: String,
        /// New parent, or the nil id for a root.
        new_parent: String,
    },
    /// Bind a material to a node (`None` clears).
    SetMaterial {
        /// Node to bind.
        id: String,
        /// Material name, `None` clears.
        material: Option<String>,
    },
    /// Set one typed component on a node (adds it or replaces that kind).
    SetComponent {
        /// Node carrying the component.
        id: String,
        /// Kind: mesh, collider, material, light, spawner, destructible, anatomy, rig.
        component: String,
        /// Fields of that kind's schema (`api.describe component:<kind>`).
        value: Value,
        /// Schema version of `value`; omitted means the current one.
        #[serde(default)]
        version: Option<u32>,
    },
    /// Remove one component kind from a node.
    RemoveComponent {
        /// Node carrying the component.
        id: String,
        /// Kind to remove.
        component: String,
    },
}

/// Approval minted by `tx.preview`, echoed back by `tx.apply`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ApprovalDto {
    /// Document revision the batch was prepared against.
    pub revision: u64,
    /// Canonical hash of the exact batch (decimal string on the wire).
    #[serde(with = "u64_text")]
    #[schemars(with = "String")]
    pub action_hash: u64,
    /// Last live revision (`current <= expiry_tick` passes).
    pub expiry_tick: u64,
}

/// Args for `scene.read`: both fields optional, defaults apply.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SceneReadArgs {
    /// Page size, 1..=128 (default 64).
    #[serde(default)]
    pub limit: Option<u32>,
    /// Page start as an object offset (default 0).
    #[serde(default)]
    pub cursor: Option<u32>,
}

/// Args for `tx.preview`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct TxPreviewArgs {
    /// Must equal the document revision, else nothing runs.
    pub expected_revision: u64,
    /// Retry identity, echoed back to `tx.apply`.
    #[serde(with = "u64_text")]
    #[schemars(with = "String")]
    pub idempotency_key: u64,
    /// Edits in order, 1..=256 entries.
    pub ops: Vec<OpDto>,
}

/// Args for `tx.apply`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct TxApplyArgs {
    /// Must equal the document revision, else nothing runs.
    pub expected_revision: u64,
    /// Same key as the previewed batch.
    #[serde(with = "u64_text")]
    #[schemars(with = "String")]
    pub idempotency_key: u64,
    /// Same ops as the previewed batch.
    pub ops: Vec<OpDto>,
    /// Approval minted by `tx.preview` for this exact batch.
    pub approval: ApprovalDto,
    /// `false` skips the spatial delta (default true).
    #[serde(default)]
    pub delta: Option<bool>,
}

/// Args for `tx.undo`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct TxUndoArgs {
    /// When given, must match the newest batch key (decimal string or number in).
    #[serde(default, with = "u64_text::opt")]
    #[schemars(with = "Option<String>")]
    pub undo_token: Option<u64>,
}

/// One snapshot row: stable id, parent link (null for roots), local pose.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SceneObjectDto {
    /// Stable object id (UUID text).
    pub id: String,
    /// Parent id (UUID text), null for roots.
    pub parent: Option<String>,
    /// Local pose in meters.
    pub local: PoseDto,
}

/// `scene.read` result: revision plus one bounded page.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SceneReadOutput {
    /// Document revision this page was read at.
    pub revision: u64,
    /// Objects in the document.
    pub total: u64,
    /// Offset this page started at.
    pub cursor: u64,
    /// Rows in this page.
    pub returned: u64,
    /// True when more rows follow.
    pub truncated: bool,
    /// Page rows in stable id order.
    pub objects: Vec<SceneObjectDto>,
}

/// `tx.preview` result: diff plus the single-use approval for `tx.apply`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct TxPreviewOutput {
    /// One line per op, in batch order.
    pub effects: Vec<String>,
    /// `base_revision + 1`; the revision an apply would publish.
    pub resulting_revision: u64,
    /// Canonical hash of the exact batch (decimal string on the wire).
    #[serde(with = "u64_text")]
    #[schemars(with = "String")]
    pub action_hash: u64,
    /// Single-use approval, bound to session/revision/hash/expiry.
    pub approval: ApprovalDto,
}

/// Spatial findings a write changed, on the objects it touched.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SpatialDeltaDto {
    /// Touched object ids, at most 16.
    pub ids: Vec<String>,
    /// New findings, red first, at most 5.
    pub new_findings: Vec<String>,
    /// Findings cleared, at most 5.
    pub resolved: Vec<String>,
    /// Findings left out by the caps.
    pub more: u32,
}

/// `tx.apply` result: the landed receipt.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct TxApplyOutput {
    /// Document revision after the apply.
    pub revision: u64,
    /// One line per op, in batch order.
    pub effects: Vec<String>,
    /// The batch key this receipt answers.
    #[serde(with = "u64_text")]
    #[schemars(with = "String")]
    pub key: u64,
    /// Journal position hint (entries shift on eviction/undo/redo).
    pub journal_index: u64,
    /// Document content digest before the write: 64 lowercase hex chars.
    #[serde(default)]
    pub before_digest: String,
    /// Content digest of the applied state as staged, before it landed.
    #[serde(default)]
    pub staged_digest: String,
    /// Content digest read back after landing. Equal to `staged_digest` unless
    /// the store carried different bytes; the durable path refuses that with
    /// `landed_mismatch` instead of returning a receipt.
    #[serde(default)]
    pub after_digest: String,
    /// Stable undo identity: pass back to `tx.undo`.
    #[serde(with = "u64_text")]
    #[schemars(with = "String")]
    pub undo_token: u64,
    /// Bounded "what changed" summary: revisions, counts and clipped rows.
    #[serde(default)]
    pub changes: ChangeListDto,
    /// Spatial findings this write changed; absent with `delta: false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spatial_delta: Option<SpatialDeltaDto>,
}

/// Bounded "what changed" summary for one landed write, on the receipt.
///
/// Consumer: an agent reads which objects the call added, removed or moved
/// instead of re-reading the scene. The counts are the full totals; `rows`
/// keeps at most `MAX_CHANGES` of them and `truncated` says when it clipped,
/// so a count never lies.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ChangeListDto {
    /// Document revision before the change.
    pub revision_before: u64,
    /// Document revision after the change.
    pub revision_after: u64,
    /// Objects present after only.
    pub added: u32,
    /// Objects present before only.
    pub removed: u32,
    /// Objects on both sides whose world pose moved.
    pub moved: u32,
    /// One row per change, clipped to the cap.
    pub rows: Vec<String>,
    /// True when `rows` dropped entries; counts stay the full totals.
    pub truncated: bool,
    /// Document content digest on the before side: 64 lowercase hex chars.
    #[serde(default)]
    pub digest_before: String,
    /// Document content digest on the after side.
    #[serde(default)]
    pub digest_after: String,
    /// Touched object ids in id-text order, clipped to the cap.
    #[serde(default)]
    pub ids: Vec<String>,
    /// Full count of `ids` before the cap.
    #[serde(default)]
    pub ids_total: u32,
    /// Relation edges gained, a row per edge: `+ relation <a> <rel> <b>`.
    #[serde(default)]
    pub edges_added: Vec<String>,
    /// Full count of relation edges gained.
    #[serde(default)]
    pub edges_added_total: u32,
    /// Relation edges lost, a row per edge: `- relation <a> <rel> <b>`.
    #[serde(default)]
    pub edges_removed: Vec<String>,
    /// Full count of relation edges lost.
    #[serde(default)]
    pub edges_removed_total: u32,
}

/// Most touched ids one change list names; the totals stay exact.
pub const CHANGE_MAX_IDS: usize = 16;
/// Most relation edges per direction one change list names.
pub const CHANGE_MAX_EDGES: usize = 16;

/// Count for a wire field: saturates instead of wrapping.
fn clip_len(count: usize) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

impl ChangeListDto {
    /// Attach the semantic part of a change: touched object ids and the
    /// relation edges gained or lost, each clipped to its cap. The totals keep
    /// the full counts, so a clipped list never reads as complete.
    pub(crate) fn with_semantics(mut self, change: SemanticChange) -> Self {
        self.ids_total = clip_len(change.ids.len());
        self.ids = change.ids.into_iter().take(CHANGE_MAX_IDS).collect();
        self.edges_added_total = clip_len(change.edges_added.len());
        self.edges_added = change
            .edges_added
            .into_iter()
            .take(CHANGE_MAX_EDGES)
            .collect();
        self.edges_removed_total = clip_len(change.edges_removed.len());
        self.edges_removed = change
            .edges_removed
            .into_iter()
            .take(CHANGE_MAX_EDGES)
            .collect();
        self
    }
}

impl From<ChangeList> for ChangeListDto {
    fn from(list: ChangeList) -> Self {
        Self {
            revision_before: list.revision_before,
            revision_after: list.revision_after,
            added: list.added,
            removed: list.removed,
            moved: list.moved,
            rows: list.rows,
            truncated: list.truncated,
            ..Self::default()
        }
    }
}

/// `tx.undo` result: the undo receipt.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct TxUndoOutput {
    /// Document revision after the undo.
    pub revision: u64,
    /// Key of the batch that was undone.
    #[serde(with = "u64_text")]
    #[schemars(with = "String")]
    pub undone_key: u64,
    /// What the undo did.
    pub effects: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Above 2^53: a JSON number for this value rounds in any f64 client.
    const BIG: u64 = 18_446_744_073_709_551_557;

    #[test]
    fn hash_above_2_53_round_trips_as_exact_string() {
        let approval = ApprovalDto {
            revision: 3,
            action_hash: BIG,
            expiry_tick: 9,
        };
        let value = rmcp::serde_json::to_value(&approval).expect("approval serializes");
        assert_eq!(
            value["action_hash"],
            rmcp::serde_json::json!("18446744073709551557")
        );
        // The text a client reads, parsed back as plain JSON.
        let text = value.to_string();
        let parsed: Value = rmcp::serde_json::from_str(&text).expect("wire text parses");
        assert_eq!(
            parsed["action_hash"]
                .as_str()
                .and_then(|s| s.parse::<u64>().ok()),
            Some(BIG),
            "string survives JSON text exactly"
        );
        // The same digits as a JSON number, read as f64, do not survive.
        let as_f64: f64 = rmcp::serde_json::from_str(&BIG.to_string()).expect("f64 parse");
        assert_ne!(as_f64 as u64, BIG, "a number would round");
        let back: ApprovalDto = rmcp::serde_json::from_value(parsed).expect("string input");
        assert_eq!(back, approval);
    }

    #[test]
    fn numeric_input_still_deserializes() {
        let old: ApprovalDto =
            rmcp::serde_json::from_str(r#"{"revision":3,"action_hash":42,"expiry_tick":9}"#)
                .expect("old numeric approval");
        assert_eq!(old.action_hash, 42);
        let big_number: ApprovalDto = rmcp::serde_json::from_str(
            r#"{"revision":3,"action_hash":18446744073709551557,"expiry_tick":9}"#,
        )
        .expect("numeric hash is exact when parsed as u64");
        assert_eq!(big_number.action_hash, BIG);
        let undo: TxUndoArgs =
            rmcp::serde_json::from_str(r#"{"undo_token":4242}"#).expect("numeric undo token");
        assert_eq!(undo.undo_token, Some(4242));
        let undo_text: TxUndoArgs =
            rmcp::serde_json::from_str(r#"{"undo_token":"4242"}"#).expect("string undo token");
        assert_eq!(undo_text.undo_token, Some(4242));
        let undo_null: TxUndoArgs =
            rmcp::serde_json::from_str(r#"{"undo_token":null}"#).expect("null undo token");
        assert_eq!(undo_null.undo_token, None);
        let undo_missing: TxUndoArgs = rmcp::serde_json::from_str("{}").expect("no undo token");
        assert_eq!(undo_missing.undo_token, None);
        assert!(
            rmcp::serde_json::from_str::<ApprovalDto>(
                r#"{"revision":3,"action_hash":"forty-two","expiry_tick":9}"#,
            )
            .is_err(),
            "a non-numeric string is refused"
        );
    }

    #[test]
    fn outputs_carry_ids_as_strings() {
        let applied = TxApplyOutput {
            revision: 1,
            effects: Vec::new(),
            key: BIG,
            journal_index: 0,
            before_digest: "0".repeat(64),
            staged_digest: "1".repeat(64),
            after_digest: "1".repeat(64),
            undo_token: BIG,
            changes: ChangeListDto::default(),
            spatial_delta: None,
        };
        let value = rmcp::serde_json::to_value(&applied).expect("receipt serializes");
        assert_eq!(
            value["key"],
            rmcp::serde_json::json!("18446744073709551557")
        );
        assert_eq!(
            value["undo_token"],
            rmcp::serde_json::json!("18446744073709551557")
        );
        assert_eq!(
            value["revision"],
            rmcp::serde_json::json!(1),
            "counters stay numbers"
        );
        let undone = TxUndoOutput {
            revision: 2,
            undone_key: BIG,
            effects: Vec::new(),
        };
        let value = rmcp::serde_json::to_value(&undone).expect("undo serializes");
        assert_eq!(
            value["undone_key"],
            rmcp::serde_json::json!("18446744073709551557")
        );
    }

    #[test]
    fn advertised_schema_says_string_for_ids() {
        let schema = rmcp::serde_json::to_value(rmcp::schemars::schema_for!(ApprovalDto))
            .expect("schema serializes");
        assert_eq!(schema["properties"]["action_hash"]["type"], "string");
        assert_eq!(schema["properties"]["expiry_tick"]["type"], "integer");
    }
}
