//! Brief to level: text brief to relation plan, SP5 placement, cell layout
//! with nav checks, then preview, apply and undo. Consumer is the `e2040-mcp`
//! door (`level.build` via `BuildLevelTool`; the one-line router hook is named
//! by `hook_name`). Owner is `engine-endpoint`; units meters in, integer cm
//! inside. Budget: at most 24 props, 16 wants each, 3 repairs, a 13x13 nav
//! grid; cost is simulated integers only, never wall clock. Errors are typed
//! `LevelError`; over-budget builds refuse with conflicts plus findings.
//! Determinism: same brief text plus seed gives the same plan, placements,
//! report and digests. Every stage shares the AB7 `brief.rs` envelope
//! (`BriefEnvelope`): this file consumes it as its input type and spends its
//! repair budget, keeping only a thin `envelope()` convert at the old call
//! site for the AB4/AB5 callers.
//!
//! ARC12 edit path: the landing edit is a relation diff. A brief becomes
//! relation requests, SP5 solves them against the shell plus the live
//! document, and the solve's spine writes (positions, yaw, scale) are what a
//! caller lands at a tick boundary. The applied relation delta
//! (`engine_spatial::diff`) is the payload a caller reads and reviews; the raw
//! coordinate table the report carries stays as the ablation arm
//! (`ablation_numbers`) and is never the edit. A solve that cannot place
//! refuses with the SP5 conflict set (`LevelRefusal::conflict_set`).

#[cfg(feature = "mcp")]
mod imp {
    use std::borrow::Cow;
    use std::collections::{BTreeMap, BTreeSet, VecDeque};

    use engine_core::{hash::FNV64_OFFSET, hash_bytes, Fnv64};
    use engine_editor::capability::{self, approve};
    use engine_editor::commands::{self, Batch, Op, ProvenanceStamp};
    use engine_scene::ids::ObjectId;
    use engine_scene::transform::Transform;
    use engine_spatial::{
        Actor, Finding, Intent, NavView, ObjectRef, Relation, Target, Tile, Wall, Want,
    };

    use rmcp::handler::server::router::tool::{SyncTool, ToolBase};
    use rmcp::model::ToolAnnotations;
    use rmcp::schemars::JsonSchema;
    use rmcp::serde::{Deserialize, Serialize};

    use super::super::brief::{BriefEnvelope, BriefStage};
    use super::super::build_game::{GameError, StageRecord};
    use super::super::mcp::{
        batch_error, cap_error, granted_ops, McpServer, McpToolError, ServerState,
    };
    use crate::canon::valid_name;
    use crate::spatial_glue::{conflict_texts, quat_of_yaw, yaw_of_quat, ParsedWant};

    mod build_level_session;
    pub use build_level_session::*;

    /// Door tool name for brief-to-level builds.
    pub const TOOL_BUILD_LEVEL: &str = "level.build";
    /// Stage tag carried by the envelope and every refusal.
    pub const STAGE: &str = "level";
    /// Next chain stage consuming `BuildReport` as its input.
    pub const NEXT_STAGE: &str = "playable-slice";
    /// Repair budget: at most this many repair rounds, then refuse.
    pub const MAX_REPAIRS: u8 = 3;
    /// Brief bound in chars.
    pub const BRIEF_MAX_CHARS: usize = 4096;
    /// Brief bound in lines.
    pub const BRIEF_MAX_LINES: usize = 64;
    /// One brief line bound in chars.
    pub const LINE_MAX_CHARS: usize = 256;
    /// Level bound in props.
    pub const PROPS_MAX: usize = 24;
    /// Intent bound in wants per prop (mirrors `scene.place`).
    pub const WANTS_MAX: usize = 16;
    /// Actor-name bound in chars (mirrors the spatial grammar check).
    pub const NAME_MAX: usize = crate::canon::NAME_MAX;
    /// Solve passes over unsolved props (at most one prop settles per pass).
    pub const PASSES_MAX: usize = PROPS_MAX;
    /// Nav tile pitch in cm (mirrors the spatial nav bake).
    pub const NAV_TILE_CM: i32 = 100;
    /// Nav grid half-extent in tiles; covers the room plus one outer ring.
    pub const NAV_HALF: i32 = 6;
    /// First shell actor id; shell ids never collide with prop or live ids.
    pub const SHELL_BASE: u32 = 900_000;
    /// First live-document actor id used while solving against a live scene.
    pub const LIVE_BASE: u32 = 700_000;
    /// Cell layout magic for `cell_bytes`.
    pub const CELL_MAGIC: [u8; 4] = *b"ABL1";
    /// Cell layout version for `cell_bytes`.
    pub const CELL_VERSION: u8 = 1;

    /// Prop catalog: kind name plus `(sx, sy, sz)` meters. Furniture-scale so
    /// props rest grounded and never seal the nav grid; walls come from the
    /// room shell, never the brief.
    pub const KINDS: &[(&str, (f32, f32, f32))] = &[
        ("altar", (1.5, 1.0, 1.0)),
        ("banner", (0.9, 2.0, 0.1)),
        ("barrel", (0.6, 0.9, 0.6)),
        ("bench", (1.8, 0.5, 0.5)),
        ("brazier", (0.6, 1.0, 0.6)),
        ("crate", (0.8, 0.8, 0.8)),
        ("pillar", (0.5, 3.0, 0.5)),
        ("post", (0.3, 2.2, 0.3)),
        ("stand", (1.2, 0.8, 1.2)),
        ("torch", (0.3, 1.6, 0.3)),
    ];

    /// Legal relation words: the SP3 vocabulary the SP5 solver understands.
    /// `next_to` and `facing` place on the grounded plane; `inside`,
    /// `on_top_of` and `supports` pass through to the solver and refuse
    /// honestly when the grounded-plane verify cannot hold them.
    pub const RELATIONS: &[&str] = &["next_to", "facing", "inside", "on_top_of", "supports"];

    /// One-line router hook for the orchestrator: register `BuildLevelTool`
    /// on the twenty-tool router in `mcp.rs` and bump `tool_names` plus the
    /// tool-list budget test. Returned as data so the hook is nameable from
    /// tests without touching the router in this packet.
    pub fn hook_name() -> &'static str {
        "mcp.rs ToolRouter::with_sync_tool::<BuildLevelTool>() + tool_names 20->21"
    }

    fn cm(value: f32) -> i32 {
        (value * 100.0).round() as i32
    }

    fn kind_size(kind: &str) -> Option<(f32, f32, f32)> {
        KINDS
            .iter()
            .find(|entry| entry.0 == kind)
            .map(|entry| entry.1)
    }

    /// What stopped a brief, a plan or a session call. Every message names
    /// the legal values; failures before apply write nothing.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum LevelError {
        /// Brief holds no usable line.
        EmptyBrief,
        /// Brief longer than `BRIEF_MAX_CHARS` chars.
        BriefTooLong { len: usize },
        /// One brief line breaks the grammar.
        BadLine { line: usize, reason: String },
        /// Unknown prop kind.
        UnknownKind { kind: String },
        /// Two props share one name.
        DuplicateName { name: String },
        /// More than `PROPS_MAX` props.
        TooManyProps { count: usize },
        /// A want names a prop the brief never declares.
        UnknownName { name: String },
        /// Spawn names a prop the brief never declares.
        UnknownSpawn { name: String },
        /// Apply against a moved revision; nothing was staged or applied.
        StaleRevision { expected: u64, got: u64 },
        /// Key reuse with a different payload; nothing was applied.
        DuplicateKey { key: u64 },
        /// Key belongs to an undone batch; submit a fresh key to reapply.
        UndoneKey { key: u64 },
        /// Undo with no applied batch on the journal.
        NothingToUndo,
        /// Token names another batch than the newest one.
        UndoMismatch { expected: u64, got: u64 },
    }

    impl LevelError {
        /// Machine code for this failure.
        pub fn code(&self) -> &'static str {
            match self {
                Self::EmptyBrief => "empty_brief",
                Self::BriefTooLong { .. } => "brief_too_long",
                Self::BadLine { .. } => "bad_line",
                Self::UnknownKind { .. } => "unknown_kind",
                Self::DuplicateName { .. } => "duplicate_name",
                Self::TooManyProps { .. } => "too_many_props",
                Self::UnknownName { .. } => "unknown_name",
                Self::UnknownSpawn { .. } => "unknown_spawn",
                Self::StaleRevision { .. } => "stale_revision",
                Self::DuplicateKey { .. } => "duplicate_key",
                Self::UndoneKey { .. } => "undone_key",
                Self::NothingToUndo => "nothing_to_undo",
                Self::UndoMismatch { .. } => "undo_mismatch",
            }
        }
    }

    impl std::fmt::Display for LevelError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::EmptyBrief => write!(
                    f,
                    "empty_brief: the brief holds no prop line; \
                     submit `prop <kind> <name>` with kind one of altar, banner, barrel, \
                     bench, brazier, crate, pillar, post, stand, torch"
                ),
                Self::BriefTooLong { len } => write!(
                    f,
                    "brief_too_long: brief holds {len} chars; at most {BRIEF_MAX_CHARS}"
                ),
                Self::BadLine { line, reason } => write!(f, "bad_line {line}: {reason}"),
                Self::UnknownKind { kind } => write!(
                    f,
                    "unknown_kind `{kind}`: submit one of altar, banner, barrel, bench, \
                     brazier, crate, pillar, post, stand, torch"
                ),
                Self::DuplicateName { name } => write!(
                    f,
                    "duplicate_name `{name}`: every prop name must be unique in one brief"
                ),
                Self::TooManyProps { count } => write!(
                    f,
                    "too_many_props: brief holds {count} props; at most {PROPS_MAX}"
                ),
                Self::UnknownName { name } => write!(
                    f,
                    "unknown_name `{name}`: a want names a prop the brief never declares; \
                     declare it with `prop <kind> <name>` first"
                ),
                Self::UnknownSpawn { name } => write!(
                    f,
                    "unknown_spawn `{name}`: spawn names a prop the brief never declares"
                ),
                Self::StaleRevision { expected, got } => write!(
                    f,
                    "stale_revision: session is at revision {expected}, batch expects {got}; \
                     re-read preview, then apply again"
                ),
                Self::DuplicateKey { key } => write!(
                    f,
                    "duplicate_key: idempotency key {key} was already used with a different \
                     payload; submit a fresh key"
                ),
                Self::UndoneKey { key } => write!(
                    f,
                    "undone_key: idempotency key {key} belongs to an undone batch; \
                     submit a fresh key to reapply"
                ),
                Self::NothingToUndo => {
                    write!(f, "nothing_to_undo: the journal holds no applied batches")
                }
                Self::UndoMismatch { expected, got } => write!(
                    f,
                    "undo_mismatch: newest undoable batch is key {expected}, not token {got}; \
                     omit undo_token to undo the newest batch"
                ),
            }
        }
    }

    impl std::error::Error for LevelError {}

    /// Thin convert at the old call site: validate a brief into the shared
    /// AB7 envelope under the level budget. Empty and oversize briefs refuse
    /// with the legal values; grammar errors surface at plan time.
    pub fn envelope(text: &str, seed: u64) -> Result<BriefEnvelope, LevelError> {
        BriefEnvelope::new(text, seed, BriefStage::Level).map_err(|error| match error {
            super::super::brief::BriefError::Empty => LevelError::EmptyBrief,
            super::super::brief::BriefError::TooLong { len } => LevelError::BriefTooLong { len },
            other => LevelError::BadLine {
                line: 0,
                reason: format!("shared envelope refused: {other}"),
            },
        })
    }

    /// Review-pipeline dedup binds (AB1): single owner in
    /// [`super::super::canon`]; re-exported here so `build_level`
    /// keeps its path with zero behavior change. Offline authoring only,
    /// never in the tick.
    pub use super::super::canon::{collapse_for_review, review_variants};

    mod parse;
    use parse::*;
    mod compile;
    pub use compile::*;
    mod nav;
    pub use nav::*;
    mod door;
    pub use door::*;

    /// One solved placement in meters, yaw radians.
    #[derive(Debug, Clone, PartialEq)]
    pub struct Placement {
        /// Prop name from the brief.
        pub name: String,
        /// Actor id.
        pub id: u32,
        /// Position meters.
        pub x: f32,
        /// Grounded height meters.
        pub y: f32,
        /// Position meters.
        pub z: f32,
        /// Facing yaw radians.
        pub yaw: f32,
    }

    /// Simulated build cost: integer counts only, never wall clock, so the
    /// report stays bit-identical across machines.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct CostInfo {
        /// SP5 solve calls behind this build.
        pub solve_calls: u32,
        /// Nav tiles baked.
        pub nav_tiles: u32,
        /// Repair rounds consumed.
        pub repair_rounds: u8,
        /// `solves*1000 + tiles*10 + rounds*500 + props*100`.
        pub units: u64,
    }

    /// Level cell layout: the WD1 bake input. The WD1 cell streamer owns
    /// async load, unload and frame-boundary publish; this layout is the
    /// deterministic bytes it reads, row-major walk bits plus a digest.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct CellLayout {
        /// Min tile corner in tile units.
        pub origin: (i32, i32),
        /// Grid dims in tiles `(x, z)`.
        pub dims: (u32, u32),
        /// Row-major walk bits, `dims.0 * dims.1` long.
        pub walk: Vec<bool>,
        /// FNV over `cell_bytes`.
        pub digest: u64,
    }

    impl CellLayout {
        /// Canonical bytes: magic, version, origin, dims, packed walk bits.
        pub fn cell_bytes(&self) -> Vec<u8> {
            let mut out = Vec::new();
            out.extend_from_slice(&CELL_MAGIC);
            out.push(CELL_VERSION);
            for lane in [self.origin.0, self.origin.1] {
                out.extend_from_slice(&lane.to_le_bytes());
            }
            for lane in [self.dims.0, self.dims.1] {
                out.extend_from_slice(&lane.to_le_bytes());
            }
            for chunk in self.walk.chunks(8) {
                let mut byte = 0u8;
                for (bit, walk) in chunk.iter().enumerate() {
                    if *walk {
                        byte |= 1 << bit;
                    }
                }
                out.push(byte);
            }
            out
        }
    }

    fn cell_layout(nav: &NavView) -> CellLayout {
        let origin = (-NAV_HALF, -NAV_HALF);
        let side = u32::try_from(2 * NAV_HALF + 1).unwrap_or(0);
        let mut walk = Vec::with_capacity((side * side) as usize);
        for tz in -NAV_HALF..=NAV_HALF {
            for tx in -NAV_HALF..=NAV_HALF {
                walk.push(
                    nav.tiles
                        .get(&(tx, tz))
                        .map(|tile| tile.walk)
                        .unwrap_or(false),
                );
            }
        }
        let layout = CellLayout {
            origin,
            dims: (side, side),
            walk,
            digest: 0,
        };
        let digest = hash_bytes(&layout.cell_bytes());
        CellLayout { digest, ..layout }
    }

    /// Passing build report: placements, routes, cost and the AB4 input line.
    /// This report (plus `cell_bytes`) is the next stage's input: the
    /// playable-slice chain consumes the level digest, the seed and the
    /// placement table verbatim.
    #[derive(Debug, Clone, PartialEq)]
    pub struct BuildReport {
        /// Envelope brief id.
        pub brief_id: u64,
        /// Build seed.
        pub seed: u64,
        /// Props placed.
        pub props_placed: u32,
        /// Placements in prop-id order.
        pub placements: Vec<Placement>,
        /// Routes in prop-id order with BFS tile distances.
        pub routes: Vec<RouteInfo>,
        /// Repair rounds consumed (at most `MAX_REPAIRS`).
        pub repairs_used: u8,
        /// Always `"pass"` here; refusals ride `BuildOutcome::Refused`.
        pub verdict: String,
        /// FNV over the placement table.
        pub level_digest: u64,
        /// FNV over the cell bytes.
        pub cell_digest: u64,
        /// Simulated cost.
        pub cost: CostInfo,
        /// AB4 input line: digest, seed, prop count, route span, repairs.
        pub next_ab4: String,
    }

    /// Refusal: the brief id, the stage, the consumed budget, the conflicts
    /// and the findings. AB7 SEAM: this is the refusal shape the shared
    /// envelope carries once AB7 lands.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct LevelRefusal {
        /// Envelope brief id.
        pub brief_id: u64,
        /// Chain stage, always `STAGE` here.
        pub stage: &'static str,
        /// Repair rounds consumed, always `MAX_REPAIRS` here.
        pub repairs_used: u8,
        /// Culprit want source lines (or the stuck prop line).
        pub conflicts: Vec<String>,
        /// SP5 conflict-set evidence (ARC12): per still-unsolved prop in id
        /// order, the solver's minimal unmet-want subset mapped back to the
        /// exact brief lines the caller wrote. Empty when the refusal came
        /// from gate findings alone (no solve failed) or the solver returned
        /// no conflict set. A caller fixes placement from these first.
        pub conflict_set: Vec<String>,
        /// Gate finding lines plus solver conflict lines.
        pub findings: Vec<String>,
        /// Why the build refused, with the fix embedded.
        pub reason: String,
        /// Legal values for the next attempt.
        pub legal: String,
    }

    /// Build outcome: a placed level with its report, or a refusal.
    #[derive(Debug, Clone, PartialEq)]
    pub enum BuildOutcome {
        /// Level placed: actors plus the passing report.
        Placed(BuiltLevel),
        /// Budget exhausted: nothing was placed.
        Refused(LevelRefusal),
    }

    /// Placed level: the full actor list (shell plus props) and the report.
    #[derive(Debug, Clone, PartialEq)]
    pub struct BuiltLevel {
        /// Shell plus placed props.
        pub actors: Vec<Actor>,
        /// Passing report.
        pub report: BuildReport,
    }

    fn placements_of(plan: &RelationPlan, actors: &[Actor]) -> Vec<Placement> {
        let mut out: Vec<Placement> = plan
            .props
            .iter()
            .filter_map(|prop| {
                actors.iter().find(|a| a.id == prop.id).map(|a| Placement {
                    name: prop.name.clone(),
                    id: prop.id,
                    x: a.x,
                    y: a.y,
                    z: a.z,
                    yaw: a.yaw,
                })
            })
            .collect();
        out.sort_by_key(|placement| placement.id);
        out
    }

    fn level_digest(placements: &[Placement]) -> u64 {
        let mut hasher = Fnv64::new();
        for placement in placements {
            hasher.write(placement.name.as_bytes());
            hasher.write(&placement.id.to_le_bytes());
            for lane in [placement.x, placement.y, placement.z, placement.yaw] {
                hasher.write(&lane.to_bits().to_le_bytes());
            }
        }
        hasher.finish()
    }

    /// Run the full chain: plan, multi-pass solve, then at most
    /// `MAX_REPAIRS` repair rounds driven by gate findings. `fixed` actors
    /// (the live document on the door path) constrain the solve but are never
    /// moved and never culprits. Returns the placed level or the refusal;
    /// writes nothing by itself.
    pub fn build(env: &BriefEnvelope) -> Result<BuildOutcome, LevelError> {
        Ok(build_on(&plan_brief(env)?, env, &[]))
    }

    fn build_on(plan: &RelationPlan, env: &BriefEnvelope, fixed: &[Actor]) -> BuildOutcome {
        let mut plan = plan.clone();
        let mut solve_calls = 0u32;
        let (mut actors, mut unsolved, mut conflicts_by_prop) =
            solve_plan(&plan, fixed, &mut solve_calls);
        let mut repairs_used = 0u8;
        let mut conflicts: Vec<String> = Vec::new();
        for round in 0..env.repair_budget() {
            let report = gates(&actors, plan.spawn_id, &unsolved);
            if gates_clean(&report) {
                return placed(env, &plan, &actors, &report, repairs_used, solve_calls);
            }
            let Some(id) = culprit(&report, &plan) else {
                break;
            };
            if !repair_round(
                &mut plan,
                &mut actors,
                round,
                id,
                &mut unsolved,
                &mut conflicts_by_prop,
                &mut solve_calls,
            ) {
                break;
            }
            repairs_used += 1;
            conflicts = plan
                .props
                .iter()
                .find(|prop| prop.id == id)
                .map(|prop| {
                    if prop.sources.is_empty() {
                        vec![format!("prop {} {}", prop.kind, prop.name)]
                    } else {
                        prop.sources.clone()
                    }
                })
                .unwrap_or_default();
        }
        let report = gates(&actors, plan.spawn_id, &unsolved);
        if gates_clean(&report) {
            return placed(env, &plan, &actors, &report, repairs_used, solve_calls);
        }
        let mut findings: Vec<String> = report.findings.iter().map(Finding::row).collect();
        for id in &report.unsolved {
            if let Some(prop) = plan.props.iter().find(|prop| &prop.id == id) {
                findings.push(format!("conflict unsolved {} - yellow 0", prop.name));
            }
        }
        for route in report.routes.iter().filter(|route| !route.reachable) {
            findings.push(format!("conflict unreachable {} - yellow 0", route.name));
        }
        findings.sort();
        let budget = env.repair_budget();
        let conflict_set = conflict_set_texts(&plan, &conflicts_by_prop, &report.unsolved);
        BuildOutcome::Refused(LevelRefusal {
            brief_id: env.brief_id(),
            stage: env.stage().as_str(),
            repairs_used,
            conflicts,
            conflict_set,
            findings,
            reason: format!(
                "no clean level after {repairs_used} repair round(s); \
                 loosen ranges, spread props or shrink the brief"
            ),
            legal: format!(
                "at most {budget} repairs per brief; wants keep SP5 grammar with \
                 0 <= lo <= hi; the room is a 10 m square"
            ),
        })
    }

    /// SP5 conflict-set evidence for a refusal: per still-unsolved prop in id
    /// order, the solver's minimal unmet-want subset mapped back to the exact
    /// brief lines the caller wrote. Empty when no solve failed (gate
    /// findings alone) or the solver returned no conflict set.
    fn conflict_set_texts(
        plan: &RelationPlan,
        conflicts: &BTreeMap<u32, engine_spatial::ConflictSet>,
        unsolved: &BTreeSet<u32>,
    ) -> Vec<String> {
        let mut out = Vec::new();
        for id in unsolved {
            let (Some(set), Some(prop)) = (
                conflicts.get(id),
                plan.props.iter().find(|prop| prop.id == *id),
            ) else {
                continue;
            };
            let parsed: Vec<ParsedWant> = prop
                .wants
                .iter()
                .cloned()
                .zip(prop.sources.iter().cloned())
                .map(|(want, source)| ParsedWant { want, source })
                .collect();
            out.extend(conflict_texts(&parsed, &set.unmet));
        }
        out
    }

    fn placed(
        env: &BriefEnvelope,
        plan: &RelationPlan,
        actors: &[Actor],
        report: &GateReport,
        repairs_used: u8,
        solve_calls: u32,
    ) -> BuildOutcome {
        let placements = placements_of(plan, actors);
        let level_digest = level_digest(&placements);
        let layout = cell_layout(&report.nav);
        let cell_digest = layout.digest;
        let nav_tiles = layout.walk.len() as u32;
        let props_placed = placements.len() as u32;
        let min_route = report
            .routes
            .iter()
            .map(|route| route.tiles)
            .min()
            .unwrap_or(0);
        let max_route = report
            .routes
            .iter()
            .map(|route| route.tiles)
            .max()
            .unwrap_or(0);
        let units = u64::from(solve_calls) * 1000
            + u64::from(nav_tiles) * 10
            + u64::from(repairs_used) * 500
            + u64::from(props_placed) * 100;
        let next_ab4 = format!(
            "{} input: level {level_digest:016x} seed {} props {props_placed} \
             routes {min_route}-{max_route} tiles repairs {repairs_used}",
            NEXT_STAGE,
            env.seed()
        );
        BuildOutcome::Placed(BuiltLevel {
            actors: actors.to_vec(),
            report: BuildReport {
                brief_id: env.brief_id(),
                seed: env.seed(),
                props_placed,
                placements,
                routes: report.routes.clone(),
                repairs_used,
                verdict: "pass".to_owned(),
                level_digest,
                cell_digest,
                cost: CostInfo {
                    solve_calls,
                    nav_tiles,
                    repair_rounds: repairs_used,
                    units,
                },
                next_ab4,
            },
        })
    }

    // Per-stage brief record: this stage records its inputs/outputs per brief
    // so one brief replays byte-identically twice with providers off. Owner is
    // `engine-endpoint`; the full-chain replay consumes `StageRecord` verbatim
    // (one format, never a second). Units are brief chars, seeds and FNV-1a
    // digests. Budget is one deterministic build plus O(n) digest folds, offline
    // authoring only, never in the tick. Errors ride `GameError`: refusals name
    // the stage plus the fix and write nothing; providers are never consulted.

    /// Replay digest domain; matches the full-chain glue so a stage record
    /// stamped here compares byte-identically there.
    const LEVEL_REPLAY_DOMAIN: u64 = 0x0ab4_9a1c_e009_0001;
    /// Level input salt; matches the full-chain glue Level salt.
    const LEVEL_INPUT_SALT: u64 = 0x1e00;

    /// FNV over the canonical stage inputs (stage tag, brief id, seed, text).
    /// Same inputs name the same digest on every machine.
    fn level_input_digest(brief_id: u64, seed: u64, text: &str) -> u64 {
        let mut hasher = Fnv64::with_state(FNV64_OFFSET ^ (LEVEL_REPLAY_DOMAIN ^ LEVEL_INPUT_SALT));
        hasher.write(BriefStage::Level.as_str().as_bytes());
        let mut digest = hasher.finish();
        hasher = Fnv64::with_state(FNV64_OFFSET ^ digest);
        hasher.write(&brief_id.to_le_bytes());
        digest = hasher.finish();
        hasher = Fnv64::with_state(FNV64_OFFSET ^ digest);
        hasher.write(&seed.to_le_bytes());
        digest = hasher.finish();
        hasher = Fnv64::with_state(FNV64_OFFSET ^ digest);
        hasher.write(text.as_bytes());
        hasher.finish()
    }

    /// FNV over the canonical stage outputs (the level plus cell digests).
    fn level_output_digest(level_digest: u64, cell_digest: u64) -> u64 {
        let mut hasher = Fnv64::with_state(FNV64_OFFSET ^ LEVEL_REPLAY_DOMAIN);
        hasher.write(&level_digest.to_le_bytes());
        let digest = hasher.finish();
        hasher = Fnv64::with_state(FNV64_OFFSET ^ digest);
        hasher.write(&cell_digest.to_le_bytes());
        hasher.finish()
    }

    // Relation-diff edit path (ARC12). Owner `engine-endpoint`, glue only:
    // the relation algebra and the delta are `engine-spatial`, the placement
    // is the SP5 solver, and the write list is the spine pose convention.
    // Units: meters and radians for a pose, line and byte counts for the
    // payload. A write is recorded for a tick boundary, never applied
    // mid-tick, the same discipline provider and script effects follow. The
    // raw coordinate table the parent report carries is kept as the ablation
    // arm only; the edit a caller reads and reviews is the relation delta.

    /// One spine transform write: the pose one placed prop takes when the
    /// edit lands on a tick boundary. The write list is recorded, not
    /// applied in place, so a consumer lands it at the boundary the way
    /// provider and script effects land.
    #[derive(Debug, Clone, PartialEq)]
    pub struct SpineWrite {
        /// Prop actor id (the spine slot's source id, name-sorted).
        pub id: u32,
        /// Prop name from the brief.
        pub name: String,
        /// Feet position meters `[x, y, z]`.
        pub pos: [f32; 3],
        /// Facing yaw radians.
        pub yaw: f32,
        /// Authored box scale `[sx, sy, sz]` meters: the authored document op
        /// needs it, the spine pose does not.
        pub scale: [f32; 3],
    }

    /// Relation-diff edit: the relation requests a brief carries, the
    /// relation delta the SP5 solve made on the level graph, and the spine
    /// writes it produced, plus the parent report of the same solve.
    #[derive(Debug, Clone, PartialEq)]
    pub struct RelationEdit {
        /// Envelope brief id.
        pub brief_id: u64,
        /// Build seed.
        pub seed: u64,
        /// Relation requests in prop-id then want order (`+ relation ...`):
        /// the diff the brief asks for, before any solve.
        pub requests: Vec<String>,
        /// Applied relation delta lines (`engine_spatial::describe` order).
        pub changes: Vec<String>,
        /// Bounded counts line (`engine_spatial::summarize`).
        pub summary: String,
        /// Spine writes in prop-id order, one per placed prop.
        pub writes: Vec<SpineWrite>,
        /// Passing report of the same solve (the raw-coordinate ablation arm).
        pub report: BuildReport,
    }

    /// Edit outcome: a relation-diff edit with its writes, or the refusal
    /// carrying the SP5 conflict set.
    #[derive(Debug, Clone, PartialEq)]
    pub enum EditOutcome {
        /// Solved: the writes are ready to land at a tick boundary.
        Edited(RelationEdit),
        /// Budget exhausted: the refusal with conflicts and findings.
        Refused(LevelRefusal),
    }

    /// Ablation outcome: the numbers for a placing brief, or the refusal.
    #[derive(Debug, Clone, PartialEq)]
    pub enum AblationOutcome {
        /// Numbers for one placed brief.
        Numbers(AblationNumbers),
        /// The brief refused; the ablation has nothing to weigh.
        Refused(LevelRefusal),
    }

    /// Relation-diff edit payload against the raw coordinate table for one
    /// brief. `requests` and `changes` count the relation lines a caller
    /// reads; `raw_lanes`/`raw_bytes` count the coordinate table the parent
    /// report carries. `digest_equal` proves the edit's spine writes
    /// reproduce the coordinate digest bit for bit, so the smaller payload
    /// is lossless. Both arms run the same plan and the same solve.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct AblationNumbers {
        /// Envelope brief id.
        pub brief_id: u64,
        /// Props placed.
        pub props: u32,
        /// Relation request lines.
        pub requests: u32,
        /// Applied relation delta lines.
        pub changes: u32,
        /// Coordinate lanes the raw table carries (4 per prop).
        pub raw_lanes: u32,
        /// Raw table bytes (one `f32` per lane).
        pub raw_bytes: u64,
        /// Relation payload bytes (requests plus changes, newline included).
        pub edit_bytes: u64,
        /// SP5 solve calls.
        pub solves: u32,
        /// Routes reachable, always equal to `props` on a placing brief.
        pub reachable: u32,
        /// True when the edit's writes carry the exact coordinate digest.
        pub digest_equal: bool,
    }

    /// One `+ relation <prop> <relation> <target>` request line per want, in
    /// prop-id then want order: the relation diff a brief asks for before any
    /// solve. Pure text; no digest is taken over it.
    fn request_lines(plan: &RelationPlan) -> Vec<String> {
        let mut out = Vec::new();
        for prop in &plan.props {
            for want in &prop.wants {
                out.push(format!(
                    "+ relation {} {} {}",
                    prop.name,
                    want.relation.name(),
                    target_label(want.target, plan)
                ));
            }
        }
        out
    }

    /// Stable target text shared by request lines: `wall:<side>`,
    /// `object:<prop name>` or `point:<x_cm>,<z_cm>`.
    fn target_label(target: Target, plan: &RelationPlan) -> String {
        match target {
            Target::Wall(wall) => wall_label(wall),
            Target::Point { x_cm, z_cm } => format!("point:{x_cm},{z_cm}"),
            Target::Object(id) => plan.props.iter().find(|prop| prop.id == id).map_or_else(
                || format!("object:{id}"),
                |prop| format!("object:{}", prop.name),
            ),
        }
    }

    /// Wall side text for request lines. One owner for the side match:
    /// [`crate::intents::wall_word`] (this used to duplicate its body).
    fn wall_label(wall: Wall) -> String {
        format!("wall:{}", crate::intents::wall_word(wall))
    }

    /// Relation delta between two actor sets (`engine_spatial` order: added
    /// and removed edges by `(a, relation, b)`, then layer relabels by id).
    /// The same two actor sets give the same delta in any input order.
    fn relation_delta(before: &[Actor], after: &[Actor]) -> engine_spatial::SemanticDiff {
        engine_spatial::diff_graph(
            &engine_spatial::build(before),
            &engine_spatial::build(after),
        )
    }

    /// Spine writes for every placed prop, in prop-id order. A prop without a
    /// live actor is skipped (it was not placed).
    fn writes_of(plan: &RelationPlan, actors: &[Actor]) -> Vec<SpineWrite> {
        let mut out: Vec<SpineWrite> = plan
            .props
            .iter()
            .filter_map(|prop| {
                actors
                    .iter()
                    .find(|actor| actor.id == prop.id)
                    .map(|actor| SpineWrite {
                        id: prop.id,
                        name: prop.name.clone(),
                        pos: [actor.x, actor.y, actor.z],
                        yaw: actor.yaw,
                        scale: [actor.sx, actor.sy, actor.sz],
                    })
            })
            .collect();
        out.sort_by_key(|write| write.id);
        out
    }

    /// Relation-diff edit path: plan the brief into relation requests, solve
    /// with SP5 against `fixed` (the live-document actors), then report the
    /// applied relation delta plus the spine writes. Writes nothing; no
    /// provider is consulted.
    pub fn relation_edit_on(
        env: &BriefEnvelope,
        fixed: &[Actor],
    ) -> Result<EditOutcome, LevelError> {
        let plan = plan_brief(env)?;
        Ok(edit_from(&plan, env, fixed))
    }

    /// Relation-diff edit over the room shell alone (no live document).
    pub fn relation_edit(env: &BriefEnvelope) -> Result<EditOutcome, LevelError> {
        relation_edit_on(env, &[])
    }

    fn edit_from(plan: &RelationPlan, env: &BriefEnvelope, fixed: &[Actor]) -> EditOutcome {
        let requests = request_lines(plan);
        match build_on(plan, env, fixed) {
            BuildOutcome::Refused(refusal) => EditOutcome::Refused(refusal),
            BuildOutcome::Placed(level) => {
                let mut before = room_shell();
                before.extend_from_slice(fixed);
                let delta = relation_delta(&before, &level.actors);
                EditOutcome::Edited(RelationEdit {
                    brief_id: level.report.brief_id,
                    seed: level.report.seed,
                    requests,
                    changes: engine_spatial::describe(&delta),
                    summary: engine_spatial::summarize(&delta),
                    writes: writes_of(plan, &level.actors),
                    report: level.report,
                })
            }
        }
    }

    /// Ablation numbers for one brief: the relation-diff payload a caller
    /// reads against the raw coordinate table the parent report carries, plus
    /// the digest-equality proof that the smaller payload is lossless. Both
    /// arms run the same plan and the same solve.
    pub fn ablation_numbers(env: &BriefEnvelope) -> Result<AblationOutcome, LevelError> {
        let edit = match relation_edit(env)? {
            EditOutcome::Edited(edit) => edit,
            EditOutcome::Refused(refusal) => return Ok(AblationOutcome::Refused(refusal)),
        };
        let placements: Vec<Placement> = edit
            .writes
            .iter()
            .map(|write| Placement {
                name: write.name.clone(),
                id: write.id,
                x: write.pos[0],
                y: write.pos[1],
                z: write.pos[2],
                yaw: write.yaw,
            })
            .collect();
        let raw_lanes = u32::try_from(placements.len())
            .ok()
            .and_then(|count| count.checked_mul(4))
            .unwrap_or(u32::MAX);
        let edit_bytes = edit
            .requests
            .iter()
            .chain(edit.changes.iter())
            .map(|line| u64::try_from(line.len() + 1).unwrap_or(u64::MAX))
            .sum();
        Ok(AblationOutcome::Numbers(AblationNumbers {
            brief_id: edit.brief_id,
            props: edit.report.props_placed,
            requests: u32::try_from(edit.requests.len()).unwrap_or(u32::MAX),
            changes: u32::try_from(edit.changes.len()).unwrap_or(u32::MAX),
            raw_lanes,
            raw_bytes: u64::from(raw_lanes) * 4,
            edit_bytes,
            solves: edit.report.cost.solve_calls,
            reachable: u32::try_from(
                edit.report
                    .routes
                    .iter()
                    .filter(|route| route.reachable)
                    .count(),
            )
            .unwrap_or(u32::MAX),
            digest_equal: level_digest(&placements) == edit.report.level_digest,
        }))
    }

    /// Record one brief: envelope, plan, solve and gate, then stamp the shared
    /// envelope line plus the FNV digests of the canonical inputs and outputs
    /// in the single stage-record format. Providers are never consulted; a
    /// refused brief refuses the record with the stage named.
    pub fn record_level(text: &str, seed: u64) -> Result<StageRecord, GameError> {
        let env = envelope(text, seed).map_err(|error| {
            GameError::new("level_refused", format!("AB1 envelope refused: {error}"))
        })?;
        let outcome = build(&env).map_err(|error| {
            GameError::new("level_refused", format!("AB1 plan refused: {error}"))
        })?;
        match outcome {
            BuildOutcome::Refused(refusal) => Err(GameError::new(
                "level_refused",
                format!(
                    "AB1 refused after {} repair(s): {}; conflicts: [{}]",
                    refusal.repairs_used,
                    refusal.reason,
                    refusal.conflicts.join(", ")
                ),
            )),
            BuildOutcome::Placed(level) => {
                let brief_id = env.brief_id();
                Ok(StageRecord {
                    stage: BriefStage::Level,
                    brief_id,
                    seed,
                    input_digest: level_input_digest(brief_id, seed, text),
                    output_digest: level_output_digest(
                        level.report.level_digest,
                        level.report.cell_digest,
                    ),
                    record_line: env.record_line(),
                })
            }
        }
    }

    /// Replay one recorded brief providers-off: rebuild from the brief inputs
    /// (deterministic solve only; no model is ever called), re-record, and
    /// require the canonical bytes back byte-identically with matching
    /// digests. A foreign stage is a loud `bad_record`; any divergence is a
    /// loud `replay_mismatch`.
    pub fn replay_level_offline(
        record: &StageRecord,
        text: &str,
        seed: u64,
    ) -> Result<StageRecord, GameError> {
        if record.stage != BriefStage::Level {
            return Err(GameError::bad_record(format!(
                "stage record is `{}`; submit `{}`",
                record.stage.as_str(),
                BriefStage::Level.as_str()
            )));
        }
        let fresh = record_level(text, seed)?;
        if fresh.encode() != record.encode() || fresh.digest() != record.digest() {
            return Err(GameError::replay_mismatch(
                record.output_digest,
                fresh.output_digest,
            ));
        }
        Ok(fresh)
    }

    /// Find the Level stage record; an absent stage is a loud `missing_record`
    /// instead of a model call. Runs before any rebuild, so a gap fails
    /// without touching a provider.
    pub fn find_level_record(records: &[StageRecord]) -> Result<&StageRecord, GameError> {
        records
            .iter()
            .find(|record| record.stage == BriefStage::Level)
            .ok_or_else(|| GameError::missing_record(BriefStage::Level.as_str()))
    }

    /// Replay one stored Level record providers-off: the missing-record check
    /// runs before any rebuild, then the bytes must come back byte-identically.
    pub fn replay_stored_level(
        records: &[StageRecord],
        text: &str,
        seed: u64,
    ) -> Result<StageRecord, GameError> {
        let record = find_level_record(records)?;
        replay_level_offline(record, text, seed)
    }

    /// Digest over an actor list in id order. Undo-clean means this digest
    /// matches across the preview/apply/undo roundtrip.
    pub fn actors_digest(actors: &[Actor]) -> u64 {
        let mut order: Vec<&Actor> = actors.iter().collect();
        order.sort_by_key(|actor| actor.id);
        let mut hasher = Fnv64::new();
        for actor in order {
            hasher.write(&actor.id.to_le_bytes());
            hasher.write(actor.name.as_bytes());
            for lane in [
                actor.x, actor.y, actor.z, actor.sx, actor.sy, actor.sz, actor.yaw,
            ] {
                hasher.write(&lane.to_bits().to_le_bytes());
            }
        }
        hasher.finish()
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::mcp::{SceneReadArgs, SceneReadTool, TxUndoArgs, TxUndoTool};
        use engine_editor::capability::Capability;

        const BRIEF_BRAZIER: &str = "prop brazier brazier_01\n\
             want brazier_01 next_to wall:north [0,60]\n\
             want brazier_01 next_to wall:west [0,60]";
        const BRIEF_CRATES: &str = "prop crate crate_01\n\
             want crate_01 next_to point:200,-300 [0,50]\n\
             prop crate crate_02\n\
             want crate_02 next_to object:crate_01 [50,150]";
        const BRIEF_PILLARS: &str = "prop pillar pillar_01\n\
             want pillar_01 next_to wall:east [0,50]\n\
             prop pillar pillar_02\n\
             want pillar_02 next_to object:pillar_01 [150,250]\n\
             want pillar_02 facing object:pillar_01";
        const BRIEF_TORCHES: &str = "prop torch torch_01\n\
             want torch_01 next_to wall:south [0,50]\n\
             prop torch torch_02\n\
             want torch_02 next_to wall:south [0,50]\n\
             want torch_02 next_to object:torch_01 [200,300]";
        const BRIEF_ALTAR: &str = "prop altar altar_01\n\
             want altar_01 next_to point:0,0 [0,80]\n\
             want altar_01 facing wall:north";
        const BRIEF_MARKET: &str = "prop barrel barrel_01\n\
             prop stand stand_01\n\
             want stand_01 next_to object:barrel_01 [30,120]\n\
             prop banner banner_01\n\
             want banner_01 next_to wall:west [0,40]\n\
             want banner_01 facing object:stand_01";
        const BRIEF_POSTS: &str = "prop post post_01\n\
             want post_01 next_to wall:north [100,200]\n\
             prop post post_02\n\
             want post_02 next_to wall:north [100,200]\n\
             want post_02 next_to object:post_01 [150,250]";
        const BRIEF_BENCH: &str = "prop bench bench_01\n\
             want bench_01 next_to wall:east [0,60]\n\
             want bench_01 facing wall:west";
        const BRIEF_YARD: &str = "prop crate crate_01\n\
             want crate_01 next_to point:-200,200 [0,60]\n\
             prop barrel barrel_01\n\
             want barrel_01 next_to object:crate_01 [40,140]\n\
             prop torch torch_01\n\
             want torch_01 next_to wall:south [0,50]\n\
             prop brazier brazier_01\n\
             want brazier_01 next_to wall:north [0,60]";
        const BRIEF_SPAWN: &str = "prop stand stand_01\n\
             want stand_01 next_to point:100,100 [0,80]\n\
             spawn stand_01\n\
             prop brazier brazier_01\n\
             want brazier_01 facing object:stand_01\n\
             want brazier_01 next_to wall:north [0,80]";

        const TEN_BRIEFS: [&str; 10] = [
            BRIEF_BRAZIER,
            BRIEF_CRATES,
            BRIEF_PILLARS,
            BRIEF_TORCHES,
            BRIEF_ALTAR,
            BRIEF_MARKET,
            BRIEF_POSTS,
            BRIEF_BENCH,
            BRIEF_YARD,
            BRIEF_SPAWN,
        ];

        fn shell_digest() -> u64 {
            actors_digest(&room_shell())
        }

        fn placed_outcome(text: &str, seed: u64) -> BuiltLevel {
            let env = envelope(text, seed).expect("brief envelopes");
            match build(&env).expect("brief builds") {
                BuildOutcome::Placed(level) => level,
                BuildOutcome::Refused(refusal) => {
                    panic!("brief must place, refused: {}", refusal.reason)
                }
            }
        }

        #[test]
        fn envelope_validates_and_stays_stable() {
            let first = envelope(BRIEF_BRAZIER, 7).expect("brief envelopes");
            let second = envelope(BRIEF_BRAZIER, 7).expect("brief envelopes again");
            assert_eq!(first, second, "same text plus seed, same envelope");
            assert_eq!(first.stage(), BriefStage::Level);
            assert_eq!(first.stage().as_str(), STAGE);
            assert_eq!(first.repair_budget(), MAX_REPAIRS);
            let direct =
                BriefEnvelope::new(BRIEF_BRAZIER, 7, BriefStage::Level).expect("shared envelope");
            assert_eq!(first, direct, "thin convert names the shared envelope");
            let other = envelope(BRIEF_BRAZIER, 8).expect("other seed envelopes");
            assert_ne!(first.brief_id(), other.brief_id(), "seed names the build");
            assert!(envelope("   \n  ", 1)
                .expect_err("blank must fail")
                .to_string()
                .contains("prop"));
            let long = "x".repeat(BRIEF_MAX_CHARS + 1);
            let refused = envelope(&long, 1).expect_err("oversize must fail");
            assert_eq!(refused.code(), "brief_too_long");
        }

        #[test]
        fn plan_sorts_names_and_honors_spawn() {
            let env = envelope(BRIEF_YARD, 3).expect("brief envelopes");
            let plan = plan_brief(&env).expect("brief plans");
            let names: Vec<&str> = plan.props.iter().map(|prop| prop.name.as_str()).collect();
            let mut sorted = names.clone();
            sorted.sort_unstable();
            assert_eq!(names, sorted, "plan order is name order");
            assert_eq!(plan.props.len(), 4);
            let spawn = envelope(BRIEF_SPAWN, 3).expect("spawn envelopes");
            let spawn_plan = plan_brief(&spawn).expect("spawn plans");
            let stand = spawn_plan
                .props
                .iter()
                .find(|prop| prop.name == "stand_01")
                .expect("stand plans");
            assert_eq!(spawn_plan.spawn_id, stand.id, "explicit spawn wins");
            let default = plan_brief(&env).expect("default plans");
            let first = plan
                .props
                .iter()
                .map(|prop| prop.id)
                .min()
                .expect("props exist");
            assert_eq!(default.spawn_id, first, "default spawn is the first prop");
        }

        #[test]
        fn bad_briefs_name_the_grammar() {
            let cases = [
                ("want lonely next_to wall:north", "unknown_name"),
                ("prop crate crate_01\nspawn ghost_01", "unknown_spawn"),
                ("prop dragon dragon_01", "unknown_kind"),
                (
                    "prop crate crate_01\nprop barrel crate_01",
                    "duplicate_name",
                ),
                ("prop crate 1st_prop", "bad_line"),
                ("prop crate BadName", "bad_line"),
                ("frobnicate now", "bad_line"),
                ("prop crate", "bad_line"),
                ("want crate_01 teleport wall:north", "bad_line"),
                ("want crate_01 next_to wall:up", "bad_line"),
                ("want crate_01 next_to point:10", "bad_line"),
                ("want crate_01 next_to wall:north [60,10]", "bad_line"),
                ("want crate_01 next_to wall:north [-5,10]", "bad_line"),
                (
                    "prop crate crate_01\nprop crate crate_01x\nspawn a\nspawn b",
                    "bad_line",
                ),
            ];
            for (text, code) in cases {
                let env = envelope(text, 11).expect("envelopes even bad grammar");
                let refused = plan_brief(&env).expect_err("bad brief must fail");
                assert_eq!(refused.code(), code, "brief `{text}`");
                assert!(!refused.to_string().is_empty());
            }
            let crowded: String = (0..PROPS_MAX + 1)
                .map(|i| format!("prop crate prop_{i:02}\n"))
                .collect();
            let env = envelope(&crowded, 11).expect("crowded envelopes");
            assert_eq!(
                plan_brief(&env).expect_err("crowded must fail").code(),
                "too_many_props"
            );
        }

        #[test]
        fn compile_emits_typed_categories_deterministically() {
            let env = envelope(BRIEF_YARD, 3).expect("brief envelopes");
            let first = compile_brief(&env).expect("brief compiles");
            let second = compile_brief(&env).expect("brief compiles again");
            assert_eq!(first, second, "same text, same constraints");
            assert_eq!(first.declares.len(), 4, "one declare per prop");
            assert_eq!(
                first.relates.len(),
                first.bounds.len(),
                "one bound per relate"
            );
            assert!(!first.relates.is_empty(), "yard carries wants");
            assert_eq!(first.compile_repairs, 0, "clean brief needs no repair");
            assert!(first.notes.is_empty(), "clean brief notes nothing");
            assert_eq!(
                first.category_count(ConstraintCategory::Declare),
                4,
                "declare count"
            );
            assert_eq!(
                first.category_count(ConstraintCategory::Spawn),
                1,
                "spawn counts one"
            );
            assert_eq!(
                first.constraint_count(),
                first.declares.len() + first.relates.len() + first.bounds.len() + 1,
                "total sums the four categories"
            );
            for declare in &first.declares {
                let mut facing_seen = false;
                for relate in first.relates.iter().filter(|r| r.prop == declare.id) {
                    if relate.relation == Relation::Facing {
                        facing_seen = true;
                    } else {
                        assert!(!facing_seen, "facing sorts last for {}", declare.name);
                    }
                }
            }
            let via_compiled = plan_from_compiled(&first);
            let via_plan = plan_brief(&env).expect("brief plans");
            assert_eq!(via_compiled, via_plan, "plan lowers the compile");
            let reordered = "want stand_01 next_to object:barrel_01 [30,120]\n\
                 prop banner banner_01\n\
                 prop barrel barrel_01\n\
                 want banner_01 facing object:stand_01\n\
                 prop stand stand_01\n\
                 want banner_01 next_to wall:west [0,40]";
            let reorder_env = envelope(reordered, 42).expect("reorder envelopes");
            let market_env = envelope(BRIEF_MARKET, 42).expect("market envelopes");
            assert_eq!(
                compile_brief(&reorder_env).expect("reorder compiles"),
                compile_brief(&market_env).expect("market compiles"),
                "line order never moves the compile"
            );
        }

        #[test]
        fn compile_repairs_stay_bounded_and_named() {
            let text = "prop crate ego\n\
                 want ego next_to object:ego [50,150]\n\
                 want ego next_to object:ego [50,150]\n\
                 prop torch torch_01\n\
                 want torch_01 next_to wall:south [0,50]";
            let env = envelope(text, 5).expect("ego envelopes");
            let compiled = compile_brief(&env).expect("ego compiles");
            assert!(
                compiled.compile_repairs >= 1 && compiled.compile_repairs <= MAX_REPAIRS,
                "one mutating repair, got {}",
                compiled.compile_repairs
            );
            assert!(
                compiled.notes.iter().any(|note| note.starts_with("dedup")),
                "dedup noted: {:?}",
                compiled.notes
            );
            assert!(
                compiled
                    .notes
                    .iter()
                    .any(|note| note.starts_with("drop-self")),
                "self-drop noted: {:?}",
                compiled.notes
            );
            assert!(
                compiled
                    .relates
                    .iter()
                    .all(|r| r.target != Target::Object(r.prop)),
                "no relate targets its own prop"
            );
            let far = "prop crate far\n\
                 want far next_to point:900,-900 [0,50]\n\
                 prop torch torch_01\n\
                 want torch_01 next_to wall:south [0,50]";
            let far_env = envelope(far, 6).expect("far envelopes");
            let far_compiled = compile_brief(&far_env).expect("far compiles");
            assert_eq!(far_compiled.compile_repairs, 1, "one clamp repair");
            assert!(
                far_compiled
                    .notes
                    .iter()
                    .any(|note| note.starts_with("clamp-point")),
                "clamp noted: {:?}",
                far_compiled.notes
            );
            for relate in &far_compiled.relates {
                if let Target::Point { x_cm, z_cm } = relate.target {
                    assert!(
                        (-ROOM_HALF_CM..=ROOM_HALF_CM).contains(&x_cm)
                            && (-ROOM_HALF_CM..=ROOM_HALF_CM).contains(&z_cm),
                        "clamped inside the room: [{x_cm},{z_cm}]"
                    );
                }
            }
        }

        #[test]
        fn semantic_tags_cover_every_relation_word() {
            assert_eq!(
                categorize_want(Relation::NextTo, Target::Wall(Wall::North)),
                WantCategory::Placement,
                "wall anchors place"
            );
            assert_eq!(
                categorize_want(Relation::NextTo, Target::Point { x_cm: 0, z_cm: 0 }),
                WantCategory::Placement,
                "point anchors place"
            );
            assert_eq!(
                categorize_want(Relation::NextTo, Target::Object(7)),
                WantCategory::SeparationNav,
                "prop spacing feeds nav clearance"
            );
            assert_eq!(
                categorize_want(Relation::Facing, Target::Wall(Wall::North)),
                WantCategory::SpawnReach,
                "approach orientation feeds reach"
            );
            assert_eq!(
                categorize_want(Relation::Facing, Target::Object(7)),
                WantCategory::SpawnReach,
                "facing a prop still feeds reach"
            );
            for relation in [Relation::Inside, Relation::OnTopOf, Relation::Supports] {
                assert_eq!(
                    categorize_want(relation, Target::Object(7)),
                    WantCategory::SupportSurface,
                    "stacking tags support"
                );
            }
        }

        #[test]
        fn semantic_buckets_partition_every_brief_before_solve() {
            for (index, text) in TEN_BRIEFS.iter().enumerate() {
                let seed = 2000 + index as u64;
                let env = envelope(text, seed).expect("brief envelopes");
                let compiled = compile_brief(&env).expect("brief compiles");
                compiled.validate().expect("compiled validates pre-solve");
                let again = compile_brief(&env).expect("brief compiles again");
                assert_eq!(
                    compiled.want_histogram(),
                    again.want_histogram(),
                    "brief {index}: same text, same buckets"
                );
                assert_eq!(
                    compiled.want_categories().len(),
                    compiled.relates.len(),
                    "brief {index}: one bucket per relate"
                );
                assert_eq!(
                    compiled.want_histogram().iter().sum::<usize>(),
                    compiled.relates.len() + 1,
                    "brief {index}: buckets plus spawn cover every relate"
                );
                assert!(
                    compiled.compile_repairs <= MAX_REPAIRS,
                    "brief {index}: repairs stay in budget"
                );
            }
            let crates_env = envelope(BRIEF_CRATES, 9).expect("crates envelopes");
            let crates_compiled = compile_brief(&crates_env).expect("crates compiles");
            assert!(
                crates_compiled
                    .want_categories()
                    .contains(&WantCategory::Placement),
                "crates point anchor places: {:?}",
                crates_compiled.want_categories()
            );
            assert!(
                crates_compiled
                    .want_categories()
                    .contains(&WantCategory::SeparationNav),
                "crates prop gap separates: {:?}",
                crates_compiled.want_categories()
            );
            let pillars_env = envelope(BRIEF_PILLARS, 9).expect("pillars envelopes");
            let pillars_compiled = compile_brief(&pillars_env).expect("pillars compiles");
            assert!(
                pillars_compiled
                    .want_categories()
                    .contains(&WantCategory::SpawnReach),
                "pillars facing reaches: {:?}",
                pillars_compiled.want_categories()
            );
        }

        #[test]
        fn four_bucket_brief_repairs_inside_budget() {
            let text = "prop crate ego\n\
                 want ego next_to object:ego [50,150]\n\
                 want ego next_to point:900,900 [0,50]\n\
                 prop torch torch_01\n\
                 want torch_01 facing object:ego\n\
                 want torch_01 supports object:ego\n\
                 want torch_01 next_to object:ego [30,120]\n\
                 want torch_01 next_to wall:south [0,50]";
            let env = envelope(text, 77).expect("dirty envelopes");
            let compiled = compile_brief(&env).expect("dirty compiles");
            compiled.validate().expect("dirty validates pre-solve");
            let histogram = compiled.want_histogram();
            for (bucket, slot) in [
                (WantCategory::Placement, 0),
                (WantCategory::SeparationNav, 1),
                (WantCategory::SupportSurface, 2),
                (WantCategory::SpawnReach, 3),
            ] {
                assert!(
                    histogram[slot] >= 1,
                    "bucket {} holds a relate: {histogram:?}",
                    bucket.as_str()
                );
            }
            assert!(
                compiled.compile_repairs >= 1 && compiled.compile_repairs <= MAX_REPAIRS,
                "self-drop plus clamp repair inside budget, got {}",
                compiled.compile_repairs
            );
        }

        #[test]
        fn broken_compile_refuses_before_the_first_solve() {
            let env = envelope(BRIEF_BRAZIER, 3).expect("brief envelopes");
            let mut compiled = compile_brief(&env).expect("brief compiles");
            compiled.bounds.pop();
            let refused = compiled
                .validate()
                .expect_err("unpaired bounds must refuse");
            assert_eq!(refused.code(), "bad_line");
            assert!(
                refused.to_string().contains("before solve"),
                "names the pre-solve refusal: {refused}"
            );
        }

        /// Raw planner for the A/B: the pre-compile shape (line order, no
        /// dedup, clamp or self-drop) so the lift test measures like for
        /// like against `compile_brief` plus `plan_from_compiled`.
        fn raw_plan(env: &BriefEnvelope) -> Result<RelationPlan, LevelError> {
            let parsed = parse_brief(env.text())?;
            let mut names: Vec<&String> = parsed.kinds.keys().collect();
            names.sort();
            let index_of: BTreeMap<&str, u32> = names
                .iter()
                .enumerate()
                .map(|(index, name)| (name.as_str(), u32::try_from(index + 1).unwrap_or(u32::MAX)))
                .collect();
            let mut props = Vec::with_capacity(names.len());
            for name in &names {
                let id = index_of[name.as_str()];
                if id == u32::MAX {
                    return Err(LevelError::TooManyProps { count: names.len() });
                }
                let raw_wants: &[RawWant] = parsed.wants.get(*name).map_or(&[], Vec::as_slice);
                if raw_wants.len() > WANTS_MAX {
                    return Err(LevelError::BadLine {
                        line: 0,
                        reason: format!(
                            "prop `{name}` holds {} wants; at most {WANTS_MAX}",
                            raw_wants.len()
                        ),
                    });
                }
                let mut wants = Vec::with_capacity(raw_wants.len());
                let mut sources = Vec::with_capacity(raw_wants.len());
                for raw in raw_wants {
                    let target = match &raw.target {
                        RawTarget::Wall(wall) => Target::Wall(*wall),
                        RawTarget::Point(x, z) => Target::Point { x_cm: *x, z_cm: *z },
                        RawTarget::Object(other) => {
                            let Some(id) = index_of.get(other.as_str()).copied() else {
                                return Err(LevelError::UnknownName {
                                    name: other.clone(),
                                });
                            };
                            Target::Object(id)
                        }
                    };
                    wants.push(Want {
                        relation: raw.relation,
                        target,
                        distance_cm: raw.range,
                    });
                    sources.push(raw.source.clone());
                }
                props.push(PlannedProp {
                    name: (*name).clone(),
                    kind: parsed.kinds[*name].clone(),
                    id,
                    wants,
                    sources,
                });
            }
            let spawn_name: &str = match &parsed.spawn {
                Some(name) => name,
                None => names[0],
            };
            let spawn_id = index_of[spawn_name];
            Ok(RelationPlan { props, spawn_id })
        }

        fn outcome_repairs(env: &BriefEnvelope, plan: &RelationPlan) -> Option<u8> {
            match build_on(plan, env, &[]) {
                BuildOutcome::Placed(level) => Some(level.report.repairs_used),
                BuildOutcome::Refused(_) => None,
            }
        }

        #[test]
        fn constraint_first_lifts_first_pass_on_ab1_briefs() {
            const AB1: [&str; 10] = [
                "prop crate ego\n\
                 want ego next_to object:ego [50,150]\n\
                 prop torch torch_01\n\
                 want torch_01 next_to wall:south [0,50]",
                "prop crate ego\n\
                 want ego facing object:ego\n\
                 prop torch torch_01\n\
                 want torch_01 next_to wall:south [0,50]",
                "prop stand hub\n\
                 want hub next_to object:hub [30,120]\n\
                 prop barrel barrel_01\n\
                 want barrel_01 next_to wall:west [0,40]",
                "prop crate ego\n\
                 want ego next_to object:ego [50,150]\n\
                 want ego next_to object:ego [50,150]\n\
                 prop torch torch_01\n\
                 want torch_01 next_to wall:south [0,50]",
                "prop crate far\n\
                 want far next_to point:900,900 [0,50]\n\
                 want far next_to object:far [50,150]\n\
                 prop torch torch_01\n\
                 want torch_01 next_to wall:south [0,50]",
                "prop crate far\n\
                 want far next_to point:900,-900 [0,50]\n\
                 prop torch torch_01\n\
                 want torch_01 next_to wall:south [0,50]",
                BRIEF_BRAZIER,
                BRIEF_BENCH,
                BRIEF_CRATES,
                BRIEF_ALTAR,
            ];
            let mut raw_clean = 0u32;
            let mut compiled_clean = 0u32;
            for (index, text) in AB1.iter().enumerate() {
                let seed = 500 + index as u64;
                let env = envelope(text, seed).expect("AB1 envelopes");
                let raw = raw_plan(&env).expect("AB1 raw plans");
                let compiled = plan_from_compiled(&compile_brief(&env).expect("AB1 compiles"));
                let raw_repairs = outcome_repairs(&env, &raw);
                let compiled_repairs = outcome_repairs(&env, &compiled);
                for repairs in [raw_repairs, compiled_repairs].into_iter().flatten() {
                    assert!(
                        repairs <= MAX_REPAIRS,
                        "brief {index} stays in budget: {repairs}"
                    );
                }
                if raw_repairs == Some(0) {
                    raw_clean += 1;
                }
                if compiled_repairs == Some(0) {
                    compiled_clean += 1;
                }
            }
            assert!(
                compiled_clean >= raw_clean + 3,
                "need +30% clean-first-pass: raw {raw_clean}/10, compiled {compiled_clean}/10"
            );
        }

        #[test]
        fn dry_run_diff_equals_applied_diff_on_ten_briefs() {
            for (index, text) in TEN_BRIEFS.iter().enumerate() {
                let seed = 1000 + index as u64;
                let mut session = fresh_session();
                let preview = session.preview(text, seed).expect("brief previews");
                let applied = match session
                    .apply(text, seed, 0, 700 + index as u64)
                    .expect("brief applies")
                {
                    ApplyOutcome::Placed(applied) => applied,
                    ApplyOutcome::Refused { refusal, .. } => {
                        panic!("brief {index} refused: {}", refusal.reason)
                    }
                };
                assert_eq!(preview.verdict, "pass", "brief {index}");
                assert_eq!(
                    preview.placements, applied.report.placements,
                    "brief {index} placements match"
                );
                assert_eq!(
                    preview.level_digest, applied.report.level_digest,
                    "brief {index} digest"
                );
                assert_eq!(
                    preview.next_ab4, applied.report.next_ab4,
                    "brief {index} next stage"
                );
                assert_eq!(
                    preview.resulting_revision, applied.revision,
                    "brief {index} revision"
                );
                assert_eq!(
                    preview.repairs_used, applied.report.repairs_used,
                    "brief {index} repairs"
                );
            }
        }

        #[test]
        fn ten_briefs_place_clean_navigable_and_undo_clean() {
            for (index, text) in TEN_BRIEFS.iter().enumerate() {
                let seed = 1000 + index as u64;
                let mut session = fresh_session();
                let before = session.current_digest();
                assert_eq!(before, shell_digest(), "fresh session holds the shell");
                let preview = session.preview(text, seed).expect("brief previews");
                assert_eq!(preview.verdict, "pass", "brief {index} previews clean");
                assert!(preview.findings.is_empty(), "brief {index}");
                assert_eq!(preview.resulting_revision, 1);
                let outcome = session
                    .apply(text, seed, 0, 900 + index as u64)
                    .expect("brief applies");
                let applied = match outcome {
                    ApplyOutcome::Placed(applied) => applied,
                    ApplyOutcome::Refused { refusal, .. } => {
                        panic!("brief {index} refused: {}", refusal.reason)
                    }
                };
                assert_eq!(session.revision(), 1);
                assert!(applied.report.repairs_used <= MAX_REPAIRS, "brief {index}");
                assert_eq!(applied.report.verdict, "pass");
                assert!(!applied.report.placements.is_empty(), "brief {index}");
                assert!(
                    applied.report.routes.iter().all(|route| route.reachable),
                    "brief {index} routes every prop"
                );
                assert!(
                    applied
                        .report
                        .routes
                        .iter()
                        .all(|route| route.tiles < u32::MAX),
                    "brief {index} records distances"
                );
                assert!(
                    applied.report.next_ab4.contains(NEXT_STAGE),
                    "brief {index} feeds the next stage"
                );
                assert_eq!(
                    applied.report.cost.repair_rounds,
                    applied.report.repairs_used
                );
                let undone = session.undo(None).expect("level undoes");
                assert_eq!(undone.revision, 2);
                assert_eq!(
                    session.current_digest(),
                    before,
                    "brief {index} undoes cleanly"
                );
            }
        }

        /// Spatial-gold fixture directory, resolved from the crate root.
        fn spatial_gold_dir() -> std::path::PathBuf {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/spatial-gold")
        }

        /// The ARC12 brief fixture rows: `id<TAB>seed<TAB>brief`, with a
        /// literal `\n` in the text standing for a brief newline. The column
        /// header is the first non-comment line (the tasks.tsv convention) and
        /// is checked before its rows parse.
        fn arc12_brief_rows() -> Vec<(String, u64, String)> {
            let path = spatial_gold_dir().join("arc12_briefs.tsv");
            let text = std::fs::read_to_string(&path).expect("arc12 briefs fixture reads");
            let mut lines = text
                .lines()
                .filter(|line| !line.is_empty() && !line.starts_with('#'));
            assert_eq!(
                lines.next(),
                Some("id\tseed\tbrief"),
                "arc12 briefs fixture keeps its column header"
            );
            let mut rows = Vec::new();
            for line in lines {
                let mut parts = line.split('\t');
                let id = parts.next().unwrap_or_default().trim().to_owned();
                let seed_text = parts.next().unwrap_or_default().trim();
                let seed = seed_text.parse::<u64>().unwrap_or_else(|error| {
                    panic!("arc12 brief row seed `{seed_text}` in {line:?}: {error}")
                });
                let brief = parts.next().unwrap_or_default().replace("\\n", "\n");
                assert_eq!(
                    parts.count(),
                    0,
                    "arc12 brief row has three columns: {line:?}"
                );
                assert!(!id.is_empty() && !brief.is_empty(), "row fields: {line:?}");
                rows.push((id, seed, brief));
            }
            rows
        }

        /// One `arc12_ablation.tsv` row: the payload counts for one brief.
        #[derive(Debug, Clone, PartialEq, Eq)]
        struct AblationRow {
            id: String,
            props: u32,
            requests: u32,
            changes: u32,
            raw_lanes: u32,
            raw_bytes: u64,
            edit_bytes: u64,
            solves: u32,
            reachable: u32,
            digest_equal: bool,
        }

        impl AblationRow {
            fn tsv(&self) -> String {
                format!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    self.id,
                    self.props,
                    self.requests,
                    self.changes,
                    self.raw_lanes,
                    self.raw_bytes,
                    self.edit_bytes,
                    self.solves,
                    self.reachable,
                    u8::from(self.digest_equal)
                )
            }

            fn of(id: &str, numbers: &AblationNumbers) -> Self {
                Self {
                    id: id.to_owned(),
                    props: numbers.props,
                    requests: numbers.requests,
                    changes: numbers.changes,
                    raw_lanes: numbers.raw_lanes,
                    raw_bytes: numbers.raw_bytes,
                    edit_bytes: numbers.edit_bytes,
                    solves: numbers.solves,
                    reachable: numbers.reachable,
                    digest_equal: numbers.digest_equal,
                }
            }
        }

        /// Read the checked-in ablation fixture rows.
        fn arc12_ablation_rows() -> Vec<AblationRow> {
            let path = spatial_gold_dir().join("arc12_ablation.tsv");
            let text = std::fs::read_to_string(&path).expect("arc12 ablation fixture reads");
            text.lines()
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .map(|line| {
                    let cols: Vec<&str> = line.split('\t').collect();
                    assert_eq!(cols.len(), 10, "ablation row has ten columns: {line}");
                    let num = |index: usize| -> u64 {
                        cols[index].parse::<u64>().expect("numeric column")
                    };
                    AblationRow {
                        id: cols[0].to_owned(),
                        props: u32::try_from(num(1)).expect("props fit"),
                        requests: u32::try_from(num(2)).expect("requests fit"),
                        changes: u32::try_from(num(3)).expect("changes fit"),
                        raw_lanes: u32::try_from(num(4)).expect("lanes fit"),
                        raw_bytes: num(5),
                        edit_bytes: num(6),
                        solves: u32::try_from(num(7)).expect("solves fit"),
                        reachable: u32::try_from(num(8)).expect("reachable fits"),
                        digest_equal: num(9) == 1,
                    }
                })
                .collect()
        }

        /// Recompute the ablation numbers for every fixture brief.
        fn arc12_ablation_for_fixture() -> Vec<(String, AblationNumbers)> {
            arc12_brief_rows()
                .iter()
                .map(|(id, seed, brief)| {
                    let env = envelope(brief, *seed).expect("fixture brief envelopes");
                    match ablation_numbers(&env).expect("fixture brief plans") {
                        AblationOutcome::Numbers(numbers) => (id.clone(), numbers),
                        AblationOutcome::Refused(refusal) => {
                            panic!("fixture brief {id} refused: {}", refusal.reason)
                        }
                    }
                })
                .collect()
        }

        #[test]
        fn arc12_fixture_briefs_are_the_ten_ab1_briefs() {
            let rows = arc12_brief_rows();
            assert_eq!(rows.len(), 10, "one fixture brief per AB1 brief");
            let texts: Vec<&str> = rows.iter().map(|(_, _, brief)| brief.as_str()).collect();
            assert_eq!(texts, TEN_BRIEFS, "the fixture is the gold AB1 brief set");
        }

        #[test]
        fn arc12_relation_edit_places_ten_briefs_with_zero_findings() {
            for (id, seed, brief) in arc12_brief_rows() {
                let env = envelope(&brief, seed).expect("fixture brief envelopes");
                let edit = match relation_edit(&env).expect("fixture brief plans") {
                    EditOutcome::Edited(edit) => edit,
                    EditOutcome::Refused(refusal) => {
                        panic!("brief {id} refused: {}", refusal.reason)
                    }
                };
                assert_eq!(edit.report.verdict, "pass", "brief {id}");
                assert!(edit.report.props_placed > 0, "brief {id} places props");
                assert_eq!(
                    edit.writes.len(),
                    edit.report.placements.len(),
                    "brief {id} writes one pose per placement"
                );
                assert!(!edit.requests.is_empty(), "brief {id} asks for relations");
                assert!(
                    edit.requests
                        .iter()
                        .all(|line| line.starts_with("+ relation ")),
                    "brief {id} requests are relation lines"
                );
                assert!(!edit.changes.is_empty(), "brief {id} changes relations");
                assert!(
                    edit.summary.starts_with("diff +"),
                    "brief {id}: {}",
                    edit.summary
                );
                assert_eq!(
                    edit.report.routes.len(),
                    edit.writes.len(),
                    "brief {id} routes every prop"
                );
                assert!(
                    edit.report.routes.iter().all(|route| route.reachable),
                    "brief {id} every route reachable"
                );
                // The writes carry the exact poses the report solved.
                for (write, placement) in edit.writes.iter().zip(edit.report.placements.iter()) {
                    assert_eq!((write.id, &write.name), (placement.id, &placement.name));
                    assert_eq!(
                        [write.pos[0], write.pos[1], write.pos[2]],
                        [placement.x, placement.y, placement.z]
                    );
                    assert_eq!(write.yaw, placement.yaw);
                }
            }
        }

        #[test]
        fn arc12_ten_briefs_land_clean_and_undo_twice() {
            for (id, seed, brief) in arc12_brief_rows() {
                let mut session = fresh_session();
                let shell = session.current_digest();
                let mut key = 4000 + seed;
                for round in 0..2u64 {
                    let revision = session.revision();
                    let preview = session.preview(&brief, seed).expect("brief previews");
                    assert_eq!(preview.verdict, "pass", "brief {id} round {round}");
                    assert!(preview.findings.is_empty(), "brief {id} zero findings");
                    assert!(!preview.placements.is_empty(), "brief {id}");
                    assert!(
                        !preview.relation_diff.is_empty(),
                        "brief {id} previews its relation edit"
                    );
                    assert!(preview.conflict_set.is_empty(), "brief {id} places");
                    let outcome = session
                        .apply(&brief, seed, revision, key)
                        .expect("brief applies");
                    let applied = match outcome {
                        ApplyOutcome::Placed(applied) => applied,
                        ApplyOutcome::Refused { refusal, .. } => {
                            panic!("brief {id} refused: {}", refusal.reason)
                        }
                    };
                    assert_eq!(applied.revision, revision + 1, "brief {id}");
                    assert_eq!(
                        applied.relation_diff, preview.relation_diff,
                        "brief {id} preview equals the landed edit"
                    );
                    assert_eq!(applied.report.placements, preview.placements);
                    assert_eq!(
                        applied.report.routes.len(),
                        applied.report.placements.len(),
                        "brief {id} routes every prop"
                    );
                    assert!(
                        applied.report.routes.iter().all(|route| route.reachable),
                        "brief {id} 100% reachable"
                    );
                    let undone = session.undo(Some(key)).expect("level undoes");
                    assert_eq!(undone.undone_key, key, "brief {id}");
                    assert_eq!(
                        session.current_digest(),
                        shell,
                        "brief {id} round {round} undoes clean"
                    );
                    key += 1;
                }
            }
        }

        #[test]
        fn arc12_door_reports_the_relation_diff_and_refuses_with_conflicts() {
            // The door's staged gate mirrors the spatial floating check on
            // unit 1 m cubes (`capability::snapshot_hierarchy`), so a prop
            // taller than 1 m reads as floating and is refused before it
            // lands. The door accept here rides a 1 m brazier; prop sizes
            // are the level build's business, gated by its own checks.
            let server = McpServer::new(11, Capability::unrestricted(11));
            let mut key = 700u64;
            // A build lands one revision, its undo publishes the next.
            let mut revision = 0u64;
            for round in 0..2u64 {
                let out = BuildLevelTool::invoke(
                    &server,
                    BuildLevelArgs {
                        brief: BRIEF_BRAZIER.to_owned(),
                        seed: 3000,
                        expected_revision: revision,
                        idempotency_key: key,
                    },
                )
                .unwrap_or_else(|error| panic!("round {round}: {}", error.message));
                assert!(out.solved, "round {round}: {}", out.reason);
                assert_eq!(out.verdict, "pass", "round {round}");
                assert!(
                    !out.relation_diff.is_empty(),
                    "round {round} reports its relation edit"
                );
                assert!(
                    out.relation_diff
                        .iter()
                        .all(|line| line.starts_with("- relation ")
                            || line.starts_with("+ relation ")
                            || line.starts_with("= ")),
                    "round {round} rows are relation changes: {:?}",
                    out.relation_diff
                );
                assert_eq!(
                    out.revision,
                    revision + 1,
                    "round {round} lands one revision"
                );
                key += 1;
                let undone = TxUndoTool::invoke(&server, TxUndoArgs { undo_token: None })
                    .unwrap_or_else(|error| panic!("round {round} undo: {}", error.message));
                assert_eq!(undone.undone_key, out.key, "round {round} undo key");
                assert_eq!(undone.revision, revision + 2, "round {round} undo revision");
                // Clean: nothing of this brief is left after the undo.
                let page =
                    SceneReadTool::invoke(&server, SceneReadArgs::default()).expect("scene reads");
                assert_eq!(page.total, 0, "round {round} undo is clean");
                revision += 2;
            }
            // A stuck brief refuses and the door names the SP5 conflict set.
            let stuck_brief = "prop crate doomed\n\
                 want doomed next_to wall:north [0,10]\n\
                 want doomed next_to wall:south [0,10]\n\
                 want doomed next_to wall:east [0,10]\n\
                 want doomed next_to wall:west [0,10]";
            let stuck = BuildLevelTool::invoke(
                &server,
                BuildLevelArgs {
                    brief: stuck_brief.to_owned(),
                    seed: 31,
                    expected_revision: revision,
                    idempotency_key: 900,
                },
            )
            .expect("door answers refusals");
            assert!(!stuck.solved);
            assert_eq!(stuck.verdict, "refused");
            assert!(!stuck.findings.is_empty(), "gate findings ride the refusal");
            assert!(
                stuck.conflicts.len() >= 2,
                "the SP5 conflict set is named: {:?}",
                stuck.conflicts
            );
            for line in &stuck.conflicts {
                assert!(
                    stuck_brief.lines().any(|want| want == line),
                    "every conflict echoes a brief want line, got: {line}"
                );
            }
            assert!(stuck.relation_diff.is_empty(), "a refusal reports no edit");
            assert_eq!(stuck.revision, revision, "refusals write nothing");
        }

        #[test]
        fn arc12_conflict_set_names_the_unmet_wants() {
            // Four walls cannot be met at once on one grounded prop.
            let text = "prop crate doomed\n\
                 want doomed next_to wall:north [0,10]\n\
                 want doomed next_to wall:south [0,10]\n\
                 want doomed next_to wall:east [0,10]\n\
                 want doomed next_to wall:west [0,10]";
            let env = envelope(text, 23).expect("stuck brief envelopes");
            let refusal = match relation_edit(&env).expect("stuck brief plans") {
                EditOutcome::Refused(refusal) => refusal,
                EditOutcome::Edited(edit) => panic!("impossible brief placed: {:?}", edit.writes),
            };
            assert!(!refusal.findings.is_empty(), "refusal carries findings");
            assert_eq!(
                refusal.conflict_set.len(),
                2,
                "SP5 keeps the minimal subset"
            );
            for line in &refusal.conflict_set {
                assert!(
                    text.lines().any(|want| want == line),
                    "conflict set echoes a brief line, got: {line}"
                );
            }
        }

        #[test]
        fn arc12_relation_edit_is_deterministic_and_name_sorted() {
            let env = envelope(BRIEF_MARKET, 42).expect("brief envelopes");
            let once = match relation_edit(&env).expect("brief plans") {
                EditOutcome::Edited(edit) => edit,
                EditOutcome::Refused(refusal) => panic!("market refused: {}", refusal.reason),
            };
            let twice = match relation_edit(&env).expect("brief plans again") {
                EditOutcome::Edited(edit) => edit,
                EditOutcome::Refused(refusal) => panic!("market refused: {}", refusal.reason),
            };
            assert_eq!(once, twice, "same brief gives the same edit twice");
            assert!(!once.changes.is_empty(), "the edit changed relations");
            assert_eq!(once.summary, twice.summary);
            // The requests follow the plan (name order), so reordered brief
            // lines ask for the same relations in the same order.
            let reordered = "want banner_01 next_to wall:west [0,40]\n\
                 prop banner banner_01\n\
                 prop stand stand_01\n\
                 prop barrel barrel_01\n\
                 want banner_01 facing object:stand_01\n\
                 want stand_01 next_to object:barrel_01 [30,120]";
            let other = envelope(reordered, 42).expect("reordered envelopes");
            let from_lines = match relation_edit(&other).expect("reordered plans") {
                EditOutcome::Edited(edit) => edit,
                EditOutcome::Refused(refusal) => panic!("reordered refused: {}", refusal.reason),
            };
            assert_eq!(
                once.requests, from_lines.requests,
                "brief line order never moves the request diff"
            );
            assert_eq!(
                once.report.level_digest, from_lines.report.level_digest,
                "and the level is the same level"
            );
        }

        #[test]
        fn arc12_ablation_matches_the_fixture_and_is_lossless() {
            let computed: Vec<AblationRow> = arc12_ablation_for_fixture()
                .iter()
                .map(|(id, numbers)| AblationRow::of(id, numbers))
                .collect();
            for row in &computed {
                eprintln!("arc12-ablation-row\t{}", row.tsv());
            }
            let expected = arc12_ablation_rows();
            assert_eq!(
                computed, expected,
                "the ablation fixture drifted from the live numbers"
            );
            let rows = arc12_brief_rows();
            let numbers = arc12_ablation_for_fixture();
            for ((id, _, brief), (_, n)) in rows.iter().zip(numbers.iter()) {
                // Lossless: the relation edit's spine writes carry the exact
                // coordinate digest of the same solve.
                assert!(n.digest_equal, "brief {id} edit carries the poses");
                assert_eq!(n.reachable, n.props, "brief {id} every route reachable");
                assert_eq!(n.raw_lanes, n.props * 4, "brief {id} four lanes per prop");
                assert_eq!(n.solves, n.props, "brief {id} one SP5 solve per prop");
                // The request diff is exactly the brief's relation wishes.
                let wants = brief
                    .lines()
                    .filter(|line| line.starts_with("want "))
                    .count();
                assert_eq!(
                    n.requests,
                    u32::try_from(wants).expect("wants fit"),
                    "brief {id} one request line per want"
                );
                assert!(n.changes > 0, "brief {id} the solve changed relations");
            }
        }

        #[test]
        fn same_brief_twice_is_identical() {
            let first = placed_outcome(BRIEF_MARKET, 42);
            let second = placed_outcome(BRIEF_MARKET, 42);
            assert_eq!(first.report.placements, second.report.placements);
            assert_eq!(first.report.level_digest, second.report.level_digest);
            assert_eq!(first.report.cell_digest, second.report.cell_digest);
            assert_eq!(first.report.cost, second.report.cost);
            assert_eq!(actors_digest(&first.actors), actors_digest(&second.actors));
            let reordered = "want stand_01 next_to object:barrel_01 [30,120]\n\
                 prop banner banner_01\n\
                 prop barrel barrel_01\n\
                 want banner_01 facing object:stand_01\n\
                 prop stand stand_01\n\
                 want banner_01 next_to wall:west [0,40]";
            let third = placed_outcome(reordered, 42);
            assert_eq!(
                first.report.level_digest, third.report.level_digest,
                "brief line order never moves the level"
            );
        }

        #[test]
        fn giant_object_measures_props_against_the_room_shell() {
            // Catalog props are 0.3 to 3 m; the fixed shell is a 10 m room.
            // Two props of different size must not make the taller one a
            // "giant" of a layout that is only the two of them (SPF5 reads
            // the span of the other solid boxes, and the shell is
            // non-physical), yet an oversized live box must still flag.
            let env =
                envelope("prop crate crate_01\nprop torch torch_01", 4).expect("brief envelopes");
            let plan = plan_brief(&env).expect("brief plans");
            let mut actors = room_shell();
            actors.extend(
                plan.props
                    .iter()
                    .enumerate()
                    .map(|(slot, prop)| stage_actor(prop, slot)),
            );
            let giants = |actors: &[Actor]| -> Vec<u32> {
                gates(actors, plan.spawn_id, &BTreeSet::new())
                    .findings
                    .iter()
                    .filter(|finding| finding.check == "giantObject")
                    .map(|finding| finding.a)
                    .collect()
            };
            assert_eq!(
                giants(&actors),
                Vec::<u32>::new(),
                "a 1.6 m torch beside a 0.8 m crate in the 10 m room is no giant"
            );
            let mut slab = stage_actor(&plan.props[0], 0);
            slab.id = LIVE_BASE;
            slab.name = "slab".to_owned();
            (slab.x, slab.y, slab.z) = (0.0, 1.0, -8.0);
            (slab.sx, slab.sy, slab.sz) = (30.0, 2.0, 0.5);
            actors.push(slab);
            assert_eq!(
                giants(&actors),
                vec![LIVE_BASE],
                "a 30 m slab against the 10 m room is still a giant"
            );
        }

        #[test]
        fn repair_drops_the_last_want_and_places() {
            // Tight [0,10] wall gaps are infeasible against the 20 cm shell
            // walls, so round 0 (drop the south want) still fails and round 1
            // (reset ranges to the solver defaults) places the prop.
            let text = "prop crate boxy\n\
                 want boxy next_to wall:north [0,10]\n\
                 want boxy next_to wall:south [0,10]\n\
                 prop torch torch_01\n\
                 want torch_01 next_to wall:south [0,50]";
            let level = placed_outcome(text, 5);
            assert_eq!(level.report.repairs_used, 2, "two repair rounds fire");
            assert_eq!(level.report.verdict, "pass");
            assert_eq!(level.report.props_placed, 2);
            assert!(level.report.routes.iter().all(|route| route.reachable));
            // Loose ranges repair in one round: each wall want holds alone,
            // so dropping the south want places the prop immediately.
            let loose = "prop crate roomy\n\
                 want roomy next_to wall:north [0,60]\n\
                 want roomy next_to wall:south [0,60]";
            let quick = placed_outcome(loose, 6);
            assert_eq!(quick.report.repairs_used, 1, "one repair round fires");
            assert_eq!(quick.report.verdict, "pass");
        }

        #[test]
        fn impossible_brief_refuses_after_three_repairs() {
            let text = "prop crate doomed\n\
                 want doomed next_to wall:north [0,10]\n\
                 want doomed next_to wall:south [0,10]\n\
                 want doomed next_to wall:east [0,10]";
            let env = envelope(text, 9).expect("brief envelopes");
            let refusal = match build(&env).expect("build runs") {
                BuildOutcome::Placed(level) => {
                    panic!("impossible brief placed: {:?}", level.report.placements)
                }
                BuildOutcome::Refused(refusal) => refusal,
            };
            assert_eq!(refusal.repairs_used, MAX_REPAIRS, "budget exhausts");
            assert_eq!(refusal.stage, STAGE);
            assert!(!refusal.findings.is_empty(), "refusal carries findings");
            assert_eq!(refusal.conflicts.len(), 2, "conflict keeps the live wants");
            assert!(refusal.reason.contains("repair"), "got: {}", refusal.reason);
            assert!(!refusal.legal.is_empty());
            let mut session = fresh_session();
            match session.apply(text, 9, 0, 4242).expect("apply runs") {
                ApplyOutcome::Placed(_) => panic!("refused builds never apply"),
                ApplyOutcome::Refused { refusal, revision } => {
                    assert_eq!(revision, 0, "refusals write nothing");
                    assert_eq!(refusal.repairs_used, MAX_REPAIRS);
                }
            }
            assert_eq!(session.revision(), 0);
            assert_eq!(
                session.undo(None).expect_err("nothing applied").code(),
                "nothing_to_undo"
            );
        }

        #[test]
        fn session_keys_stale_and_undo_rules() {
            let mut session = fresh_session();
            let stale = session
                .apply(BRIEF_BRAZIER, 1, 7, 100)
                .expect_err("stale must fail");
            assert_eq!(stale.code(), "stale_revision");
            assert_eq!(session.revision(), 0);
            let first = match session
                .apply(BRIEF_BRAZIER, 1, 0, 100)
                .expect("first applies")
            {
                ApplyOutcome::Placed(applied) => applied,
                ApplyOutcome::Refused { .. } => panic!("brazier must place"),
            };
            assert_eq!((first.revision, first.key, first.undo_token), (1, 100, 100));
            let replay = match session
                .apply(BRIEF_BRAZIER, 1, 1, 100)
                .expect("replay runs")
            {
                ApplyOutcome::Placed(applied) => applied,
                ApplyOutcome::Refused { .. } => panic!("replay must place"),
            };
            assert_eq!(replay, first, "same key plus payload replays");
            assert_eq!(session.revision(), 1, "replays publish no revision");
            let clash = session
                .apply(BRIEF_CRATES, 1, 1, 100)
                .expect_err("key clash must fail");
            assert_eq!(clash.code(), "duplicate_key");
            let wrong = session.undo(Some(999)).expect_err("wrong token must fail");
            assert_eq!(wrong.code(), "undo_mismatch");
            let undone = session.undo(Some(100)).expect("undo lands");
            assert_eq!((undone.revision, undone.undone_key), (2, 100));
            assert_eq!(session.current_digest(), shell_digest());
            let gone = session
                .apply(BRIEF_BRAZIER, 1, 2, 100)
                .expect_err("undone key must fail");
            assert_eq!(gone.code(), "undone_key");
            let fresh = match session
                .apply(BRIEF_BRAZIER, 1, 2, 101)
                .expect("fresh key applies")
            {
                ApplyOutcome::Placed(applied) => applied,
                ApplyOutcome::Refused { .. } => panic!("brazier must place again"),
            };
            assert_eq!(fresh.revision, 3);
        }

        #[test]
        fn nav_bake_and_cell_bytes_are_deterministic() {
            let level = placed_outcome(BRIEF_YARD, 77);
            let nav = bake_nav(&level.actors, 1);
            assert_eq!(nav.tiles.len(), 169, "13x13 bake");
            assert_eq!(nav.spawn_id, 1);
            let layout = cell_layout(&nav);
            assert_eq!(layout.dims, (13, 13));
            assert_eq!(layout.walk.len(), 169);
            let bytes = layout.cell_bytes();
            assert_eq!(&bytes[0..4], &CELL_MAGIC);
            assert_eq!(bytes[4], CELL_VERSION);
            assert_eq!(layout.digest, {
                let mut hash: u64 = 0xcbf29ce484222325;
                for byte in &bytes {
                    hash ^= u64::from(*byte);
                    hash = hash.wrapping_mul(0x100000001b3);
                }
                hash
            });
            assert_eq!(level.report.cell_digest, layout.digest);
            let again = bake_nav(&placed_outcome(BRIEF_YARD, 77).actors, 1);
            assert_eq!(cell_layout(&again), layout, "bake repeats exactly");
        }

        #[test]
        fn sealed_prop_routes_unreachable() {
            let mut actors = room_shell();
            let ring = |id: u32, name: &str, x: f32, z: f32| Actor {
                id,
                name: name.into(),
                x,
                y: 1.0,
                z,
                sx: 9.8,
                sy: 2.0,
                sz: 0.2,
                parent: None,
                physical: false,
                vx: 0.0,
                vz: 0.0,
                yaw: 0.0,
            };
            actors.push(ring(SHELL_BASE + 10, "wall_ring_n", 0.0, -1.0));
            actors.push(ring(SHELL_BASE + 11, "wall_ring_s", 0.0, 1.0));
            actors.push(Actor {
                id: 1,
                name: "sealed_01".into(),
                x: 0.0,
                y: 0.5,
                z: 0.0,
                sx: 1.0,
                sy: 1.0,
                sz: 1.0,
                parent: None,
                physical: true,
                vx: 0.0,
                vz: 0.0,
                yaw: 0.0,
            });
            actors.push(Actor {
                id: 2,
                name: "free_01".into(),
                x: 3.0,
                y: 0.5,
                z: 3.0,
                sx: 1.0,
                sy: 1.0,
                sz: 1.0,
                parent: None,
                physical: true,
                vx: 0.0,
                vz: 0.0,
                yaw: 0.0,
            });
            let nav = bake_nav(&actors, 2);
            let route_list = routes(&nav, &actors);
            let sealed = route_list
                .iter()
                .find(|route| route.id == 1)
                .expect("sealed routes");
            let free = route_list
                .iter()
                .find(|route| route.id == 2)
                .expect("free routes");
            assert!(!sealed.reachable, "the ring seals the spawn-side route");
            assert!(free.reachable, "open ground stays reachable");
            assert_eq!(free.tiles, 0, "spawn prop routes zero tiles");
        }

        #[test]
        fn door_build_applies_and_undoes() {
            let server = McpServer::new(7, Capability::unrestricted(7));
            let out = BuildLevelTool::invoke(
                &server,
                BuildLevelArgs {
                    brief: BRIEF_BRAZIER.to_owned(),
                    seed: 21,
                    expected_revision: 0,
                    idempotency_key: 501,
                },
            )
            .expect("door builds");
            assert!(out.solved, "door places: {}", out.reason);
            assert_eq!(out.placements.len(), 1);
            assert_eq!(out.placements[0].name, "brazier_01");
            assert_eq!(out.verdict, "pass");
            assert!(out.next_ab4.contains(NEXT_STAGE));
            assert_eq!((out.revision, out.key, out.undo_token), (1, 501, 501));
            let page = SceneReadTool::invoke(
                &server,
                SceneReadArgs {
                    limit: Some(8),
                    cursor: None,
                },
            )
            .expect("scene reads");
            assert_eq!(page.total, 1, "one live node landed");
            let undone =
                TxUndoTool::invoke(&server, TxUndoArgs { undo_token: None }).expect("door undoes");
            assert_eq!(undone.revision, 2);
            let back = SceneReadTool::invoke(&server, SceneReadArgs::default()).expect("reads");
            assert_eq!(back.total, 0, "undo removes the landed node");
            let stuck = BuildLevelTool::invoke(
                &server,
                BuildLevelArgs {
                    brief: "prop crate doomed\nwant doomed next_to wall:north [0,10]\n\
                         want doomed next_to wall:south [0,10]\n\
                         want doomed next_to wall:east [0,10]"
                        .to_owned(),
                    seed: 22,
                    expected_revision: 2,
                    idempotency_key: 502,
                },
            )
            .expect("door answers refusals");
            assert!(!stuck.solved);
            assert_eq!(stuck.verdict, "refused");
            assert!(!stuck.findings.is_empty());
            assert_eq!(stuck.revision, 2, "refusals write nothing");
            let stale = BuildLevelTool::invoke(
                &server,
                BuildLevelArgs {
                    brief: BRIEF_BRAZIER.to_owned(),
                    seed: 21,
                    expected_revision: 0,
                    idempotency_key: 503,
                },
            )
            .expect_err("stale builds fail");
            assert_eq!(stale.code, "stale_revision");
        }

        #[test]
        fn hook_names_the_door_registration() {
            assert_eq!(TOOL_BUILD_LEVEL, "level.build");
            assert!(hook_name().contains("BuildLevelTool"));
            assert!(hook_name().contains("tool_names"));
            assert_eq!(STAGE, "level");
            assert_eq!(MAX_REPAIRS, 3);
        }

        #[test]
        fn ten_recorded_briefs_replay_byte_identically_twice_offline() {
            for (index, text) in TEN_BRIEFS.iter().enumerate() {
                let seed = 1000 + index as u64;
                let live = record_level(text, seed)
                    .unwrap_or_else(|error| panic!("brief {index} must record: {error}"));
                assert_eq!(live.stage, BriefStage::Level);
                assert_ne!(live.input_digest, 0, "brief {index} input real");
                assert_ne!(live.output_digest, 0, "brief {index} output real");
                let first = replay_level_offline(&live, text, seed)
                    .unwrap_or_else(|error| panic!("brief {index} must replay once: {error}"));
                let second = replay_level_offline(&live, text, seed)
                    .unwrap_or_else(|error| panic!("brief {index} must replay twice: {error}"));
                assert_eq!(live.encode(), first.encode(), "brief {index} replays once");
                assert_eq!(
                    live.encode(),
                    second.encode(),
                    "brief {index} replays twice"
                );
                assert_eq!(live.digest(), first.digest(), "brief {index} digest");
                assert_eq!(live.digest(), second.digest(), "brief {index} digest x2");
                assert_eq!(
                    live.input_digest, first.input_digest,
                    "brief {index} live and offline inputs match"
                );
                assert_eq!(
                    live.output_digest, first.output_digest,
                    "brief {index} live and offline digests match"
                );
                assert_eq!(
                    live.record_line, first.record_line,
                    "brief {index} stamps one record line"
                );
                let stored = [live.clone()];
                let via_stored = replay_stored_level(&stored, text, seed)
                    .unwrap_or_else(|error| panic!("brief {index} must replay stored: {error}"));
                assert_eq!(
                    live.encode(),
                    via_stored.encode(),
                    "brief {index} stored replays"
                );
                let decoded = StageRecord::decode(&live.encode()).expect("record decodes");
                assert_eq!(live, decoded, "brief {index} round-trips");
            }
        }

        #[test]
        fn missing_level_record_fails_loudly_without_a_model() {
            let live = record_level(TEN_BRIEFS[0], 1000).expect("brief records");
            let error = find_level_record(&[]).expect_err("empty store must refuse");
            assert_eq!(error.code, "missing_record");
            assert!(
                error.to_string().contains("level"),
                "names the stage: {error}"
            );
            assert!(
                error.to_string().contains("instead of calling a model"),
                "never falls back: {error}"
            );
            let error = replay_stored_level(&[], TEN_BRIEFS[0], 1000)
                .expect_err("empty replay must refuse");
            assert_eq!(error.code, "missing_record");
            let mut foreign = live.clone();
            foreign.stage = BriefStage::Character;
            let error =
                find_level_record(&[foreign.clone()]).expect_err("foreign store must refuse");
            assert_eq!(error.code, "missing_record");
            let error = replay_level_offline(&foreign, TEN_BRIEFS[0], 1000)
                .expect_err("foreign record must refuse");
            assert_eq!(error.code, "bad_record");
        }

        #[test]
        fn corrupt_level_record_bytes_refuse_as_bad_record() {
            let live = record_level(TEN_BRIEFS[2], 1002).expect("brief records");
            let bytes = live.encode();
            for at in [0usize, 1, 5, bytes.len() / 2, bytes.len() - 1] {
                let error = StageRecord::decode(&bytes[..at]).expect_err("cut must refuse");
                assert_eq!(error.code, "bad_record", "cut at {at}");
            }
            assert_eq!(
                StageRecord::decode(&[])
                    .expect_err("empty must refuse")
                    .code,
                "bad_record"
            );
            let mut trailed = bytes.clone();
            trailed.push(0x00);
            assert_eq!(
                StageRecord::decode(&trailed)
                    .expect_err("trailing must refuse")
                    .code,
                "bad_record"
            );
            let mut versioned = bytes.clone();
            versioned[0] ^= 0x7f;
            assert_eq!(
                StageRecord::decode(&versioned)
                    .expect_err("version must refuse")
                    .code,
                "bad_record"
            );
        }

        #[test]
        fn level_replay_detects_tampered_outputs_and_inputs() {
            let text = TEN_BRIEFS[3];
            let seed = 1003;
            let live = record_level(text, seed).expect("brief records");
            let mut tampered = live.clone();
            tampered.output_digest ^= 0x01;
            let error = replay_level_offline(&tampered, text, seed)
                .expect_err("tampered output must diverge");
            assert_eq!(error.code, "replay_mismatch");
            let mut tampered_input = live.clone();
            tampered_input.input_digest ^= 0x01;
            let error = replay_level_offline(&tampered_input, text, seed)
                .expect_err("tampered input must diverge");
            assert_eq!(error.code, "replay_mismatch");
            let error = replay_level_offline(&live, TEN_BRIEFS[4], 1004)
                .expect_err("swapped text must diverge");
            assert_eq!(error.code, "replay_mismatch");
            let error =
                record_level("prop dragon dragon_01", 77).expect_err("unknown kind must refuse");
            assert_eq!(error.code, "level_refused");
            assert!(error.to_string().contains("AB1"), "stage named: {error}");
        }

        #[test]
        fn level_records_share_the_single_full_chain_stage_format() {
            use crate::build_game::{
                build_slice, fixture_briefs, record_chain, REPLAY_STAGES, STAGE_RECORD_VERSION,
            };
            let live = record_level(TEN_BRIEFS[0], 1000).expect("brief records");
            assert_eq!(live.encode()[0], STAGE_RECORD_VERSION);
            assert!(REPLAY_STAGES.contains(&BriefStage::Level));
            for index in [0usize, 1] {
                let fixture = &fixture_briefs()[index];
                let slice = build_slice(
                    fixture.text,
                    fixture.seed,
                    fixture.variant,
                    fixture.bloodied,
                )
                .unwrap_or_else(|error| panic!("slice {index} must package: {error}"));
                let chain = record_chain(
                    fixture.text,
                    fixture.seed,
                    fixture.variant,
                    fixture.bloodied,
                    &slice,
                )
                .expect("chain records");
                let staged = chain.find_stage(BriefStage::Level).expect("level staged");
                let direct = record_level(fixture.text, fixture.seed).expect("direct records");
                assert_eq!(*staged, direct, "slice {index} shares the stage record");
                assert_eq!(staged.encode(), direct.encode());
            }
        }

        #[test]
        fn review_pipeline_collapses_seeded_sets_by_thirty_percent() {
            let all = crate::canon::thirty_sets();
            assert_eq!(all.len(), 240);
            let report = review_variants(&all).expect("review dedup lands");
            assert!(
                report.meets_bar,
                "need >=30% reduction, got {}/{} ({} permille)",
                report.candidate, report.baseline, report.reduction_permille
            );
            assert!(report.reduction_permille >= crate::canon::REDUCTION_MIN_PERMILLE);
            let collapsed = collapse_for_review(&all).expect("collapse lands");
            assert_eq!(collapsed.len(), report.candidate, "one per key");
            let again = collapse_for_review(&all).expect("replay lands");
            assert_eq!(collapsed, again);
        }

        #[test]
        fn review_pipeline_keeps_keypoint_distinct_scenes_split() {
            let mut distinct: Vec<String> = Vec::new();
            for i in 0..30u64 {
                distinct.push(format!("prop crate key_{i:02}\nspawn key_{i:02}"));
            }
            let report = review_variants(&distinct).expect("distinct dedup lands");
            assert_eq!(report.candidate, 30, "distinct scenes never merge");
            assert_eq!(report.reduction_permille, 0);
            let collapsed = collapse_for_review(&distinct).expect("collapse lands");
            assert_eq!(collapsed.len(), 30);
            let pair = [
                "prop crate a\nspawn a".to_owned(),
                "prop crate a\nwant a facing wall:north\nspawn a".to_owned(),
            ];
            let split = review_variants(&pair).expect("pair lands");
            assert_eq!((split.baseline, split.candidate), (2, 2));
            assert_eq!(collapse_for_review(&pair).expect("pair collapse").len(), 2);
        }

        #[test]
        fn review_pipeline_refuses_bad_inputs_with_codes() {
            assert_eq!(
                review_variants(&["   ".to_owned()])
                    .expect_err("blank")
                    .code(),
                "empty_variant"
            );
            assert_eq!(
                collapse_for_review(&["   ".to_owned()])
                    .expect_err("blank collapse")
                    .code(),
                "empty_variant"
            );
            let long = "x".repeat(crate::canon::MAX_CHARS + 1);
            assert_eq!(
                review_variants(std::slice::from_ref(&long))
                    .expect_err("overlong")
                    .code(),
                "variant_too_long"
            );
            assert_eq!(
                collapse_for_review(&[long])
                    .expect_err("overlong collapse")
                    .code(),
                "variant_too_long"
            );
            let many = vec!["prop crate a\nspawn a".to_owned(); crate::canon::MAX_VARIANTS + 1];
            assert_eq!(
                review_variants(&many).expect_err("too many").code(),
                "too_many_variants"
            );
            assert_eq!(
                collapse_for_review(&many)
                    .expect_err("too many collapse")
                    .code(),
                "too_many_variants"
            );
        }
    }
}

#[cfg(feature = "mcp")]
pub use imp::*;
