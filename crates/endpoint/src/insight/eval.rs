//! Gold-set scoring and with/without-tools A/B for Engine Insight.
//!
//! Owner: API1. Units: tokens are chars/4 estimates, time in receipts.
//! Determinism: same corpus gives the same report; ordering is path-stable.
//! Errors: [`InsightError`] for malformed gold rows (missing fields).

#[cfg(test)]
use super::index::{Index, InsightError};
#[cfg(test)]
use super::search::{self};

/// One gold row: an agent-style query plus the path that must rank top-3.
/// Test-only: the gold fixture never ships in lib artifacts.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoldQuery {
    /// Agent-style question, written from board rows.
    pub query: String,
    /// Canonical path expected in the top 3.
    pub expect: String,
}

/// Gold report: hits, total, per-mille rate and the misses.
/// Test-only: scored by the gold tests below.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoldReport {
    /// Queries with `expect` inside the top 3.
    pub hits: usize,
    /// Queries scored.
    pub total: usize,
    /// `1000 * hits / total`.
    pub rate_milli: u32,
    /// Queries that missed, for the receipt.
    pub misses: Vec<String>,
}

/// The 50-query gold set. Lives at `fixtures/insight-gold.json` so agents
/// and harnesses read the same rows the tests score. Test-only: the fixture
/// text never embeds in lib artifacts.
#[cfg(test)]
pub const GOLD_JSON: &str = include_str!("../../../../fixtures/insight-gold.json");

/// Parse the gold JSON with a small std-only scanner (the endpoint
/// headless graph carries no serde): reads `"q"`/`"expect"` pairs.
/// Test-only.
#[cfg(test)]
pub fn parse_gold(json: &str) -> Result<Vec<GoldQuery>, InsightError> {
    let mut out = Vec::new();
    let mut rest = json;
    while let Some(q_at) = rest.find("\"q\"") {
        let after_q = &rest[q_at + 3..];
        let (query, query_used) = take_string(after_q).ok_or_else(|| InsightError::Walk {
            path: "fixtures/insight-gold.json".to_owned(),
            reason: "gold row is missing its query string".to_owned(),
        })?;
        let after_expect = after_q[query_used..].find("\"expect\"").and_then(|at| {
            let tail = &after_q[query_used + at + 8..];
            take_string(tail).map(|(value, used)| (value, query_used + at + 8 + used))
        });
        let Some((expect, used)) = after_expect else {
            return Err(InsightError::Walk {
                path: "fixtures/insight-gold.json".to_owned(),
                reason: "gold row is missing its expect path".to_owned(),
            });
        };
        out.push(GoldQuery { query, expect });
        rest = &after_q[used..];
    }
    if out.is_empty() {
        return Err(InsightError::Walk {
            path: "fixtures/insight-gold.json".to_owned(),
            reason: "gold set holds no queries".to_owned(),
        });
    }
    Ok(out)
}

/// Read one JSON string literal after a key: returns the value plus the
/// bytes consumed (handles `\"` and `\\` escapes). Test-only.
#[cfg(test)]
fn take_string(text: &str) -> Option<(String, usize)> {
    let open = text.find('"')? + 1;
    let mut value = String::new();
    let mut escaped = false;
    for (at, ch) in text[open..].char_indices() {
        if escaped {
            value.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            return Some((value, open + at + 1));
        } else {
            value.push(ch);
        }
    }
    None
}

/// Score gold queries against the index: `expect` must land in the top 3.
/// Test-only.
#[cfg(test)]
pub fn run_gold(index: &Index, queries: &[GoldQuery]) -> GoldReport {
    let mut hits = 0;
    let mut misses = Vec::new();
    for query in queries {
        let top = search::search(index, &query.query, 3).unwrap_or_default();
        if top.iter().any(|hit| hit.path == query.expect) {
            hits += 1;
        } else {
            misses.push(format!("{} -> {}", query.query, query.expect));
        }
    }
    GoldReport {
        hits,
        total: queries.len(),
        rate_milli: if queries.is_empty() {
            0
        } else {
            (1000 * hits / queries.len()) as u32
        },
        misses,
    }
}

/// One A/B packet: a board-row task with a known target item. Test-only.
#[cfg(test)]
pub struct PacketTask {
    /// Board row, e.g. `MCP7`.
    pub id: &'static str,
    /// What the agent must find.
    pub goal: &'static str,
    /// Canonical path that answers the goal.
    pub target: &'static str,
}

/// A/B outcome: baseline (paged dump under a token budget) versus tools
/// (`api.search` then `api.describe`). Tokens are chars/4. Test-only.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbReport {
    /// Baseline tokens across all tasks.
    pub base_tokens: usize,
    /// Tool-path tokens across all tasks.
    pub tool_tokens: usize,
    /// Baseline tool calls (8-item page reads per task).
    pub base_calls: usize,
    /// Tool-path calls (search plus describe per task).
    pub tool_calls: usize,
    /// Baseline tasks solved (target inside the budget window).
    pub base_correct: usize,
    /// Tool-path tasks solved (target in the top 3).
    pub tool_correct: usize,
    /// Tasks compared.
    pub tasks: usize,
}

/// Compare paged dumps against search-then-describe on 10 board tasks.
/// The baseline reads path-sorted 8-item pages until it solves the task or
/// `budget_tokens` runs out; solved tasks charge only the tokens read, not
/// the full budget. The tool path spends one search plus one describe task.
/// Test-only.
#[cfg(test)]
pub fn ab_compare(index: &Index, tasks: &[PacketTask], budget_tokens: usize) -> AbReport {
    let mut report = AbReport {
        base_tokens: 0,
        tool_tokens: 0,
        base_calls: 0,
        tool_calls: 0,
        base_correct: 0,
        tool_correct: 0,
        tasks: tasks.len(),
    };
    let dump: Vec<String> = index
        .items()
        .iter()
        .map(|item| format!("{} {} {}", item.path, item.signature, item.docs))
        .collect();
    for task in tasks {
        // Baseline: page the dump, stopping at the solving page so solved
        // tasks charge only the tokens read; unsolved tasks run to budget.
        let mut seen = 0;
        let mut solved = false;
        let mut pages = 0;
        for chunk in dump.chunks(8) {
            pages += 1;
            let chars: usize = chunk.iter().map(String::len).sum();
            seen += chars / 4;
            if chunk.iter().any(|row| row.starts_with(task.target)) {
                if seen <= budget_tokens {
                    solved = true;
                }
                break;
            }
            if seen >= budget_tokens {
                break;
            }
        }
        report.base_tokens += seen.min(budget_tokens);
        report.base_calls += pages;
        report.base_correct += usize::from(solved);
        // Tool path: one search plus one describe of the top hit.
        let hits = search::search(index, task.goal, 3).unwrap_or_default();
        let solved_tool = hits.iter().any(|hit| hit.path == task.target);
        let mut chars = task.goal.len();
        for hit in &hits {
            chars += hit.path.len() + hit.brief.len();
        }
        if let Some(top) = hits.first() {
            if let Some(item) = index.get(&top.path) {
                chars += item.signature.len() + item.docs.len();
            }
        }
        report.tool_tokens += chars / 4;
        report.tool_calls += 2;
        report.tool_correct += usize::from(solved_tool);
    }
    report
}

/// Fixed 74-item corpus behind the gold set, the A/B and the graph tests:
/// 8 engine files plus the 4 Insight tools, one asset and one scene ref.
/// Synthetic retrieval fixture: every item carries one unique rare marker
/// word (Turnstile, Lodestone, ...) so gold queries discriminate by
/// construction. Real-workspace recall is covered separately by the
/// `real_workspace_spot_check_*` test below.
#[cfg(test)]
pub(crate) fn seed_corpus() -> Index {
    use super::index::FileInput;
    const IDS: &str = "\
/// Stable object identity for authored nodes. Turnstile ledgers key on it, never on a content hash.
pub struct ObjectId(u64);
/// Nil sentinel for root parents. Lodestone null marker for empty links.
pub const NIL_ID: u64 = 0;
/// Revision-stamped snapshot handle. Waybill tags each sealed generation.
pub struct SnapshotId(u64);
/// Mint a fresh identity outside the tick. Ferryman hands out ids in order.
pub fn alloc_id() -> ObjectId { ObjectId(0) }
/// Bad UUID text on the wire. Harbormaster rejects malformed ids.
pub enum ParseError { BadText }
/// Parse UUID text into an ObjectId. Hadron splits the lanes.
pub fn from_text(text: &str) -> ObjectId { ObjectId(0) }
/// True for the nil sentinel. Cartwheel spins on empty checks.
pub fn is_nil(id: ObjectId) -> bool { true }
/// FNV digest of an ObjectId for journal keys, never a security hash. Bellows pump the lanes.
pub fn id_hash(id: ObjectId) -> u64 { 0 }
";
    const SNAPSHOT: &str = "\
/// Collect every ObjectId in id-text order. Lantern walk over roots.
pub fn snapshot_ids() -> Vec<ObjectId> { Vec::new() }
/// One sealed page of rows. Dossier pins revision plus rows.
pub struct Snapshot { pub revision: u64 }
/// Slice one page of rows by cursor. Parchment cuts the page.
pub fn page_rows(cursor: u32) -> u32 { cursor }
/// Cursor past the end or a vanished node. Tripwire fires on bad pages.
pub enum SnapshotError { BadCursor }
/// Codec version for sealed pages. Signet stamps the envelope.
pub const SNAPSHOT_VERSION: u32 = 1;
/// Blank snapshot at revision zero. Fallow field with no rows.
pub fn empty_snapshot() -> Snapshot { Snapshot { revision: 0 } }
/// Root versus delta page kinds. Crossroads picks the shape.
pub enum SnapshotKind { Root, Delta }
/// Rows in one page. Abacus beads per cursor.
pub fn row_count() -> u64 { 0 }
";
    const CAPABILITY: &str = "\
/// Minted by preview, spent by apply. Windlass binds session, revision, hash and expiry.
pub struct Approval { pub revision: u64 }
/// Mint an Approval for one exact batch. Notary seals the canonical hash.
pub fn approve() -> Approval { Approval { revision: 0 } }
/// Session token with op mask and scope. Turnkey guards every apply, holding the Approval key.
pub struct Capability { pub session: u64 }
/// Atomic gated apply through the journal. Portcullis drops on forged approvals.
pub fn apply_gated() -> u64 { 0 }
/// Forged, stale, expired or reused approvals. Barrister names each refusal.
pub enum CapError { Forged }
/// Insert, move, edit and delete bits. Lattice of four grants.
pub struct OpMask(u8);
/// Spent-key ledger against replays. Tally book of used hashes.
pub struct ApprovalLedger { pub spent: u64 }
/// Ops left in this session. Cistern level before refusal.
pub fn remaining() -> u32 { 0 }
";
    const MCP_DOOR: &str = "\
/// Stdio door over one document. Gatehouse serves a single client.
pub struct McpServer { pub session: u64 }
/// Bounded snapshot tool, read-only. Spyglass pages stable-id rows.
pub struct SceneReadTool;
/// Dry-run diff with no side effects. Windmill turns without grinding.
pub struct TxPreviewTool;
/// Atomic capability-gated apply. Drawbridge lands one revision.
pub struct TxApplyTool;
/// Agree to 2026-07-28, refuse the rest. Bouncer checks protocol dates.
pub fn negotiate_version() -> bool { true }
/// Convert wire ops to command ops. Cooper binds the batch staves.
pub fn build_batch() -> u64 { 0 }
/// Snapshot tool name. Signpost for scene reads.
pub const TOOL_SCENE_READ: &str = \"scene.read\";
/// Machine code plus legal values. Towncrier never sends an empty pass.
pub struct McpToolError { pub code: u32 }
";
    // NOTE (test-honesty): the `golden_brief_text_arena_fixture() -> bool`
    // line inside BRIEF is synthetic retrieval-fixture text for the
    // `#[cfg(test)]` seed corpus only — sample data, never a proof source
    // (nothing asserts its return value; the real brief gate is elsewhere).
    const BRIEF: &str = "\
/// Bounded brief text plus digest. Cartographer sketches groups and relations.
pub fn scene_brief() -> u64 { 0 }
/// Max brief bytes per call. Saddlebag limit for summaries.
pub const BRIEF_BUDGET: u32 = 4096;
/// Revision, digest, text and id table. Waypoint packet for agents.
pub struct BriefOutput { pub revision: u64 }
/// Order-invariant digest of actors. Millstone grinds positions to one hash.
pub fn digest_scene() -> u64 { 0 }
/// Drop whole rows past the budget. Shears cut rows, never mid-row.
pub fn truncate_rows() -> u64 { 0 }
/// Supports, on-top-of, inside, next-to, facing. Compass of five relations.
pub enum RelationKind { Supports }
/// Golden brief over the arena fixture. Lodestar pins the expected text.
pub fn golden_brief_text_arena_fixture() -> bool { true }
/// Synthetic prop names like prop_0007. Nametag keeps grammar checks quiet.
pub fn actor_name() -> u64 { 0 }
";
    const MATCH_MODE: &str = "\
/// Headless bout on a fixed tick. Bullring hosts two fighters.
pub fn run_match() -> u64 { 0 }
/// Rounds, ticks and seeds for one bout. Rulebook of the arena.
pub struct MatchConfig { pub rounds: u32 }
/// Ticks before both sides count as frozen. Frostbite threshold rejects degenerate bouts.
pub const STUCK_LIMIT: u32 = 600;
/// Save and compare match state. Bookmark for fork honesty.
pub fn checkpoint() -> u64 { 0 }
";
    const LOADER: &str = "\
/// Load the walk cycle. Treadmill loops content/locomotion/walk.anim on disk.
pub fn load_walk() -> u64 { 0 }
/// Cooked vertex and index blobs. Kiln bakes source plus settings.
pub struct CookedMesh { pub bytes: u64 }
/// Source plus settings plus importer version. Foundry pours the artifact digest.
pub fn cook_model() -> u64 { 0 }
/// Dependency graph edges for invalidation. Spiderweb tracks imports.
pub struct ManifestEntry { pub path: u64 }
";
    const MOCK: &str = "\
/// Scripted answers for evals. Understudy never calls the wire.
pub struct MockProvider;
/// Prompt context plus budgets and deadlines. Purse carries the request.
pub struct AiRequest { pub budget: u32 }
/// Proposal plus usage rows. Ledger of cost and latency.
pub struct AiResult { pub cost: u64 }
/// Mock, local or remote backends. Stable of three adapters.
pub enum BackendKind { Mock }
/// Destination and data-class gates. Gatekeeper binds privacy mode.
pub struct SessionPolicy { pub privacy: u32 }
";
    let files = [
        FileInput {
            path: "crates/scene/src/ids.rs",
            krate: "engine-scene",
            text: IDS,
        },
        FileInput {
            path: "crates/scene/src/snapshot.rs",
            krate: "engine-scene",
            text: SNAPSHOT,
        },
        FileInput {
            path: "crates/editor/src/capability.rs",
            krate: "engine-editor",
            text: CAPABILITY,
        },
        FileInput {
            path: "crates/endpoint/src/mcp_door.rs",
            krate: "engine-endpoint",
            text: MCP_DOOR,
        },
        FileInput {
            path: "crates/spatial/src/brief.rs",
            krate: "engine-spatial",
            text: BRIEF,
        },
        FileInput {
            path: "crates/arena/src/match_mode.rs",
            krate: "engine-arena",
            text: MATCH_MODE,
        },
        FileInput {
            path: "crates/assets/src/loader.rs",
            krate: "engine-assets",
            text: LOADER,
        },
        FileInput {
            path: "crates/provider/src/mock.rs",
            krate: "engine-provider",
            text: MOCK,
        },
    ];
    let mut index = Index::build(&files);
    index.add_tool(
        "api.search",
        "api.search(query, limit?) -> hits",
        "Search the engine index with BM25. Lodestone finder for agents.",
        "API1",
    );
    index.add_tool(
        "api.describe",
        "api.describe(path) -> item",
        "Full signature, docs, units, owner and callers for one path. Cartouche reads one entry.",
        "API1",
    );
    index.add_tool(
        "code.callers",
        "code.callers(path) -> callers",
        "Reverse lookup: which items mention a path. Bloodhound trails callers through the graph.",
        "API1",
    );
    index.add_tool(
        "asset.refs",
        "asset.refs(asset) -> refs",
        "Which code and scenes touch one asset. Census of asset references.",
        "API1",
    );
    index.add_tool(
        "scene.brief",
        "scene.brief(budget?) -> brief",
        "Bounded brief text plus digest. Eyepiece summary for agents.",
        "MCP3",
    );
    index.add_tool(
        "scene.place",
        "scene.place(intent) -> placement",
        "Relation intent through the command gate. Dockmaster berths props.",
        "MCP3",
    );
    index.add_tool(
        "tx.preview",
        "tx.preview(batch) -> diff plus approval",
        "Dry-run a command batch with no side effects. Rehearsal before the single-use approval.",
        "MCP1",
    );
    index.add_tool(
        "tx.apply",
        "tx.apply(batch, approval) -> receipt",
        "Atomically apply a previewed batch. Gavel lands one revision.",
        "MCP1",
    );
    index.add_asset("content/locomotion/walk.anim", "anim", 4096);
    index.add_scene_ref(
        "550e8400-e29b-41d4-a716-446655440000",
        "brazier-01",
        "fixtures/scene-v1.json",
    );
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ten board-row packets for the A/B: goals read like agent questions.
    pub(crate) fn ab_tasks() -> Vec<PacketTask> {
        vec![
            PacketTask {
                id: "MCP7",
                goal: "gatehouse stdio door serves one client McpServer",
                target: "engine-endpoint::mcp_door::McpServer",
            },
            PacketTask {
                id: "MCP1",
                goal: "windlass binds session revision hash expiry Approval",
                target: "engine-editor::capability::Approval",
            },
            PacketTask {
                id: "SP4",
                goal: "cartographer sketches groups relations scene_brief",
                target: "engine-spatial::brief::scene_brief",
            },
            PacketTask {
                id: "GM1",
                goal: "bullring hosts two fighters run_match headless",
                target: "engine-arena::match_mode::run_match",
            },
            PacketTask {
                id: "AS1",
                goal: "treadmill loops the walk cycle load_walk",
                target: "engine-assets::loader::load_walk",
            },
            PacketTask {
                id: "AX1",
                goal: "understudy never calls the wire MockProvider",
                target: "engine-provider::mock::MockProvider",
            },
            PacketTask {
                id: "SC1",
                goal: "turnstile ledgers key on stable identity ObjectId",
                target: "engine-scene::ids::ObjectId",
            },
            PacketTask {
                id: "EV1",
                goal: "bloodhound trails callers through the graph code",
                target: "code.callers",
            },
            PacketTask {
                id: "MCP3",
                goal: "waypoint packet revision digest text BriefOutput",
                target: "engine-spatial::brief::BriefOutput",
            },
            PacketTask {
                id: "ED7",
                goal: "portcullis drops on forged approvals apply_gated",
                target: "engine-editor::capability::apply_gated",
            },
        ]
    }

    #[test]
    fn gold_fixture_scores_at_least_ninety_percent_top3() {
        let index = seed_corpus();
        let queries = parse_gold(GOLD_JSON).expect("gold fixture parses");
        assert_eq!(queries.len(), 50, "the gold set holds exactly 50 queries");
        // Every expect path must exist, or the fixture drifted from the index.
        for query in &queries {
            assert!(
                index.get(&query.expect).is_some(),
                "gold expect `{}` is not indexed",
                query.expect
            );
        }
        let report = run_gold(&index, &queries);
        eprintln!(
            "insight gold: {}/{} top-3 ({} permille)",
            report.hits, report.total, report.rate_milli
        );
        assert!(
            report.misses.is_empty(),
            "gold misses (need >=90%): {:?}",
            report.misses
        );
        assert!(report.rate_milli >= 900, "hit rate {}", report.rate_milli);
    }

    #[test]
    fn tools_win_tokens_calls_and_correctness() {
        // Unity Insight cut tokens 53% and time 52% cost-only; this A/B
        // scores correctness too, over 10 board-row packets.
        let index = seed_corpus();
        let tasks = ab_tasks();
        let report = ab_compare(&index, &tasks, 800);
        eprintln!(
            "insight A/B: base {} tokens / {} calls / {}/{} correct vs tools {} tokens / {} calls / {}/{} correct",
            report.base_tokens,
            report.base_calls,
            report.base_correct,
            report.tasks,
            report.tool_tokens,
            report.tool_calls,
            report.tool_correct,
            report.tasks
        );
        assert!(
            report.tool_tokens < report.base_tokens,
            "tools use fewer tokens"
        );
        assert!(
            report.tool_calls < report.base_calls,
            "tools use fewer calls"
        );
        assert!(
            report.tool_correct > report.base_correct,
            "tools solve more"
        );
        assert_eq!(report.tool_correct, report.tasks, "tools solve all 10");
    }

    #[test]
    fn gold_parser_rejects_empty_and_broken_rows() {
        assert!(parse_gold("{\"queries\": []}").is_err());
        assert!(parse_gold("{\"queries\": [{\"q\": \"x\"}]}").is_err());
        let one = parse_gold("{\"queries\": [{\"q\": \"a b\", \"expect\": \"c::D\"}]}")
            .expect("one row parses");
        assert_eq!(
            one,
            vec![GoldQuery {
                query: "a b".to_owned(),
                expect: "c::D".to_owned()
            }]
        );
    }

    #[test]
    fn real_workspace_spot_check_finds_scene_identity() {
        // Honesty spot-check on the live tree (no synthetic markers): the
        // real `crates/` walk must surface `engine-scene::ids::ObjectId`.
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..");
        let mut index = Index::new();
        index
            .refresh_from_dir(&root.join("crates"))
            .expect("workspace walks");
        let item = index
            .get("engine-scene::ids::ObjectId")
            .expect("real workspace indexes the scene identity");
        assert_eq!(item.owner, "SC1");
        assert_eq!(item.file, "crates/scene/src/ids.rs");
        let hits = search::search(&index, "ObjectId", 8).expect("real search runs");
        assert!(
            hits.iter()
                .any(|hit| hit.path == "engine-scene::ids::ObjectId"),
            "ObjectId ranks in the top 8 on the real tree, got {:?}",
            hits.iter().map(|hit| &hit.path).collect::<Vec<_>>()
        );
    }
}
