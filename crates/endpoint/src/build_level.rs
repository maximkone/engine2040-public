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
    pub const NAME_MAX: usize = 64;
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

    /// Spatial-grammar names: `[a-z0-9_]`, 1..=`NAME_MAX` chars, no leading
    /// digit. Brief names are validated here so `run()` never flags them.
    fn valid_name(name: &str) -> bool {
        if name.is_empty() || name.len() > NAME_MAX {
            return false;
        }
        let mut chars = name.chars();
        let first = chars.next().unwrap_or('0');
        if !first.is_ascii_lowercase() && first != '_' {
            return false;
        }
        name.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
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

    enum RawTarget {
        Wall(Wall),
        Point(i32, i32),
        Object(String),
    }

    struct RawWant {
        relation: Relation,
        target: RawTarget,
        range: Option<(i32, i32)>,
        source: String,
    }

    struct ParsedBrief {
        kinds: BTreeMap<String, String>,
        wants: BTreeMap<String, Vec<RawWant>>,
        spawn: Option<String>,
    }

    fn bad_line(line_no: usize, detail: &str) -> LevelError {
        LevelError::BadLine {
            line: line_no,
            reason: format!(
                "{detail}; grammar is `prop <kind> <name>`, \
                 `want <name> <relation> <target> [<lo>,<hi>]?` or `spawn <name>` with \
                 target `wall:<north|south|east|west>`, `object:<name>` or \
                 `point:<x_cm>,<z_cm>`"
            ),
        }
    }

    fn parse_relation_word(word: &str) -> Option<Relation> {
        match word {
            "next_to" => Some(Relation::NextTo),
            "facing" => Some(Relation::Facing),
            "inside" => Some(Relation::Inside),
            "on_top_of" => Some(Relation::OnTopOf),
            "supports" => Some(Relation::Supports),
            _ => None,
        }
    }

    fn parse_wall_word(word: &str) -> Option<Wall> {
        match word {
            "north" => Some(Wall::North),
            "south" => Some(Wall::South),
            "east" => Some(Wall::East),
            "west" => Some(Wall::West),
            _ => None,
        }
    }

    fn parse_int_word(word: &str, line_no: usize, what: &str) -> Result<i32, LevelError> {
        word.parse::<i32>().map_err(|_| {
            bad_line(
                line_no,
                &format!("bad {what} `{word}` (expected an integer)"),
            )
        })
    }

    /// Brief grammar, one directive per line, `#` comments ignored. Prop and
    /// want order never matters: props sort by name, wants keep line order per
    /// prop, so reordered brief lines plan identically.
    fn parse_brief(text: &str) -> Result<ParsedBrief, LevelError> {
        let mut kinds: BTreeMap<String, String> = BTreeMap::new();
        let mut wants: BTreeMap<String, Vec<RawWant>> = BTreeMap::new();
        let mut spawn: Option<String> = None;
        let mut lines = 0usize;
        for (index, raw) in text.lines().enumerate() {
            let line_no = index + 1;
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            lines += 1;
            if lines > BRIEF_MAX_LINES {
                return Err(bad_line(
                    line_no,
                    &format!("more than {BRIEF_MAX_LINES} directives"),
                ));
            }
            if line.len() > LINE_MAX_CHARS {
                return Err(bad_line(
                    line_no,
                    &format!("longer than {LINE_MAX_CHARS} chars"),
                ));
            }
            let mut parts = line.split_whitespace();
            let head = parts.next().unwrap_or("");
            match head {
                "prop" => {
                    let kind = parts
                        .next()
                        .ok_or_else(|| bad_line(line_no, "prop needs `<kind> <name>`"))?;
                    let name = parts
                        .next()
                        .ok_or_else(|| bad_line(line_no, "prop needs `<kind> <name>`"))?;
                    if parts.next().is_some() {
                        return Err(bad_line(line_no, "prop takes exactly `<kind> <name>`"));
                    }
                    if kind_size(kind).is_none() {
                        return Err(LevelError::UnknownKind {
                            kind: kind.to_owned(),
                        });
                    }
                    if !valid_name(name) {
                        return Err(bad_line(
                            line_no,
                            &format!(
                                "bad name `{name}` (lowercase letters, digits and `_`, \
                                 no leading digit, at most {NAME_MAX} chars)"
                            ),
                        ));
                    }
                    if kinds.contains_key(name) {
                        return Err(LevelError::DuplicateName {
                            name: name.to_owned(),
                        });
                    }
                    kinds.insert(name.to_owned(), kind.to_owned());
                }
                "want" => {
                    let name = parts
                        .next()
                        .ok_or_else(|| bad_line(line_no, "want needs a prop name"))?;
                    let relation_word = parts
                        .next()
                        .ok_or_else(|| bad_line(line_no, "want needs a relation"))?;
                    let target_word = parts
                        .next()
                        .ok_or_else(|| bad_line(line_no, "want needs a target"))?;
                    let range_word = parts.next();
                    if parts.next().is_some() {
                        return Err(bad_line(line_no, "want takes at most one range"));
                    }
                    if !valid_name(name) {
                        return Err(bad_line(line_no, &format!("bad name `{name}`")));
                    }
                    let relation = parse_relation_word(relation_word).ok_or_else(|| {
                        bad_line(
                            line_no,
                            &format!(
                                "unknown relation `{relation_word}`; submit one of {}",
                                RELATIONS.join(", ")
                            ),
                        )
                    })?;
                    let target = match target_word.split_once(':') {
                        Some(("wall", word)) => {
                            RawTarget::Wall(parse_wall_word(word).ok_or_else(|| {
                                bad_line(line_no, &format!("unknown wall `{word}`"))
                            })?)
                        }
                        Some(("object", word)) => {
                            if !valid_name(word) {
                                return Err(bad_line(
                                    line_no,
                                    &format!("bad object name `{word}`"),
                                ));
                            }
                            RawTarget::Object(word.to_owned())
                        }
                        Some(("point", coords)) => {
                            let (x_word, z_word) = coords
                                .split_once(',')
                                .ok_or_else(|| bad_line(line_no, "point needs `<x_cm>,<z_cm>`"))?;
                            RawTarget::Point(
                                parse_int_word(x_word, line_no, "x_cm")?,
                                parse_int_word(z_word, line_no, "z_cm")?,
                            )
                        }
                        _ => {
                            return Err(bad_line(
                                line_no,
                                "target needs a `wall:|object:|point:` kind",
                            ));
                        }
                    };
                    let range = match range_word {
                        None => None,
                        Some(word) => {
                            let inner = word
                                .strip_prefix('[')
                                .and_then(|text| text.strip_suffix(']'))
                                .ok_or_else(|| bad_line(line_no, "range needs `[<lo>,<hi>]`"))?;
                            let (lo_word, hi_word) = inner
                                .split_once(',')
                                .ok_or_else(|| bad_line(line_no, "range needs `[<lo>,<hi>]`"))?;
                            let lo = parse_int_word(lo_word, line_no, "lo")?;
                            let hi = parse_int_word(hi_word, line_no, "hi")?;
                            if lo < 0 || hi < lo {
                                return Err(bad_line(
                                    line_no,
                                    &format!("bad range `[{lo},{hi}]` (need 0 <= lo <= hi)"),
                                ));
                            }
                            Some((lo, hi))
                        }
                    };
                    wants.entry(name.to_owned()).or_default().push(RawWant {
                        relation,
                        target,
                        range,
                        source: line.to_owned(),
                    });
                }
                "spawn" => {
                    let name = parts
                        .next()
                        .ok_or_else(|| bad_line(line_no, "spawn needs a prop name"))?;
                    if parts.next().is_some() {
                        return Err(bad_line(line_no, "spawn takes exactly one name"));
                    }
                    if !valid_name(name) {
                        return Err(bad_line(line_no, &format!("bad name `{name}`")));
                    }
                    if spawn.is_some() {
                        return Err(bad_line(line_no, "spawn takes exactly one prop"));
                    }
                    spawn = Some(name.to_owned());
                }
                other => {
                    return Err(bad_line(
                        line_no,
                        &format!("unknown directive `{other}` (submit prop, want or spawn)"),
                    ));
                }
            }
        }
        if kinds.len() > PROPS_MAX {
            return Err(LevelError::TooManyProps { count: kinds.len() });
        }
        for name in wants.keys() {
            if !kinds.contains_key(name) {
                return Err(LevelError::UnknownName { name: name.clone() });
            }
        }
        if let Some(name) = &spawn {
            if !kinds.contains_key(name) {
                return Err(LevelError::UnknownSpawn { name: name.clone() });
            }
        }
        if kinds.is_empty() {
            return Err(LevelError::EmptyBrief);
        }
        Ok(ParsedBrief {
            kinds,
            wants,
            spawn,
        })
    }

    /// One planned prop: stable id, catalog size and solver wants with
    /// object targets resolved to actor ids.
    #[derive(Debug, Clone, PartialEq)]
    pub struct PlannedProp {
        /// Prop name from the brief.
        pub name: String,
        /// Catalog kind from the brief.
        pub kind: String,
        /// Stable actor id (name-sorted 1-based).
        pub id: u32,
        /// Solver wants in brief line order.
        pub wants: Vec<Want>,
        /// Source want lines in the same order (conflicts echo these).
        pub sources: Vec<String>,
    }

    /// Relation plan: every prop plus the nav spawn. Name-sorted, so brief
    /// line order never moves the plan.
    #[derive(Debug, Clone, PartialEq)]
    pub struct RelationPlan {
        /// Props in name order.
        pub props: Vec<PlannedProp>,
        /// Actor id the nav routes start from.
        pub spawn_id: u32,
    }

    /// Typed constraint category: every compiled want lands in exactly one
    /// per-category vector before placement solves. Counts are deterministic.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub enum ConstraintCategory {
        /// Prop declarations (`prop <kind> <name>`).
        Declare,
        /// Relation targets without ranges.
        Relate,
        /// Distance bounds, one per relate entry.
        Bound,
        /// Nav spawn (`spawn <name>`, default first prop).
        Spawn,
    }

    impl ConstraintCategory {
        /// Stable tag for notes and diagnostics.
        pub fn as_str(self) -> &'static str {
            match self {
                Self::Declare => "declare",
                Self::Relate => "relate",
                Self::Bound => "bound",
                Self::Spawn => "spawn",
            }
        }
    }

    /// One declared prop: catalog kind plus its stable actor id.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct DeclareConstraint {
        /// Prop name from the brief.
        pub name: String,
        /// Catalog kind from the brief.
        pub kind: String,
        /// Stable actor id (name-sorted 1-based).
        pub id: u32,
    }

    /// One relation want without its range: typed target, source line kept
    /// so conflicts echo the brief.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RelateConstraint {
        /// Owning prop actor id.
        pub prop: u32,
        /// Relation word.
        pub relation: Relation,
        /// Resolved target (object names become ids here).
        pub target: Target,
        /// Source brief line.
        pub source: String,
    }

    /// One range bound for a relate entry: explicit or solver-defaulted.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct BoundConstraint {
        /// Owning prop actor id.
        pub prop: u32,
        /// Index into the prop's relate entries.
        pub want_idx: usize,
        /// Inclusive lower bound in cm.
        pub lo: i32,
        /// Inclusive upper bound in cm.
        pub hi: i32,
        /// True when the brief gave no range and the solver default applies.
        pub defaulted: bool,
    }

    /// Semantic want bucket: every compiled relate lands in exactly one
    /// bucket before the placement solver runs, so pre-solve validation and
    /// repair budget account per bucket. Pure tag over relation plus target;
    /// the same relate names the same bucket on every machine, and the
    /// solver never sees this tag. Repair budget is shared: at most
    /// `MAX_REPAIRS` mutating repairs across all four buckets, so no single
    /// bucket can exceed it.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub enum WantCategory {
        /// Grounded anchors: `next_to` a wall or a room point.
        Placement,
        /// Prop separation: `next_to` another prop; spacing feeds nav
        /// clearance.
        SeparationNav,
        /// Stacking and containment: `inside`, `on_top_of`, `supports`.
        SupportSurface,
        /// Approach and reach: `facing` any target, plus the spawn directive
        /// (counted here by `want_histogram`); these feed route reachability.
        SpawnReach,
    }

    impl WantCategory {
        /// Stable tag for notes and diagnostics.
        pub fn as_str(self) -> &'static str {
            match self {
                Self::Placement => "placement",
                Self::SeparationNav => "separation_nav",
                Self::SupportSurface => "support_surface",
                Self::SpawnReach => "spawn_reach",
            }
        }

        /// Histogram slot for this bucket, in
        /// `[Placement, SeparationNav, SupportSurface, SpawnReach]` order.
        /// Named conversion so histogram indexing never casts.
        pub fn index(self) -> usize {
            match self {
                Self::Placement => 0,
                Self::SeparationNav => 1,
                Self::SupportSurface => 2,
                Self::SpawnReach => 3,
            }
        }
    }

    /// Tag one relate by relation plus target. Total: every relation word
    /// the brief grammar accepts lands in exactly one bucket. Deterministic
    /// and allocation-free.
    pub fn categorize_want(relation: Relation, target: Target) -> WantCategory {
        match (relation, target) {
            (Relation::Inside | Relation::OnTopOf | Relation::Supports, _) => {
                WantCategory::SupportSurface
            }
            (Relation::Facing, _) => WantCategory::SpawnReach,
            (Relation::NextTo, Target::Object(_)) => WantCategory::SeparationNav,
            (Relation::NextTo, _) => WantCategory::Placement,
        }
    }

    /// Constraint-first compile of one brief: typed per-category constraints
    /// before placement solves. Bounded diagnose-and-repair loop with greedy
    /// fixes only (no optimizer): dedup, room clamp and self-target drop,
    /// at most `MAX_REPAIRS` mutating repairs. Deterministic: same brief
    /// text gives the same categories, order and notes.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct CompiledBrief {
        /// Declares in prop-id order.
        pub declares: Vec<DeclareConstraint>,
        /// Relates grouped by prop: walls, points, objects, facing last.
        pub relates: Vec<RelateConstraint>,
        /// One bound per relate entry, same order.
        pub bounds: Vec<BoundConstraint>,
        /// Actor id the nav routes start from.
        pub spawn_id: u32,
        /// Mutating compile repairs consumed (at most `MAX_REPAIRS`).
        pub compile_repairs: u8,
        /// One note per normalization or repair, in apply order.
        pub notes: Vec<String>,
    }

    impl CompiledBrief {
        /// Count constraints in one category (spawn counts one).
        pub fn category_count(&self, category: ConstraintCategory) -> usize {
            match category {
                ConstraintCategory::Declare => self.declares.len(),
                ConstraintCategory::Relate => self.relates.len(),
                ConstraintCategory::Bound => self.bounds.len(),
                ConstraintCategory::Spawn => 1,
            }
        }

        /// Total constraints across all four categories.
        pub fn constraint_count(&self) -> usize {
            self.declares.len() + self.relates.len() + self.bounds.len() + 1
        }

        /// Semantic bucket per relate entry, in relate order.
        pub fn want_categories(&self) -> Vec<WantCategory> {
            self.relates
                .iter()
                .map(|relate| categorize_want(relate.relation, relate.target))
                .collect()
        }

        /// Histogram over the four semantic buckets in
        /// `[Placement, SeparationNav, SupportSurface, SpawnReach]` order;
        /// the spawn directive counts one in `SpawnReach`. Deterministic:
        /// the same compiled brief gives the same counts on every machine.
        pub fn want_histogram(&self) -> [usize; 4] {
            let mut counts = [0usize; 4];
            for relate in &self.relates {
                counts[categorize_want(relate.relation, relate.target).index()] += 1;
            }
            counts[WantCategory::SpawnReach.index()] += 1;
            counts
        }

        /// Pre-solve validation over the compiled form: relates and bounds
        /// stay paired, every relate and bound names a declared prop, and
        /// the spawn names a declared prop. Runs after compile, before the
        /// first placement solve; a broken compile refuses here and places
        /// nothing.
        pub fn validate(&self) -> Result<(), LevelError> {
            if self.relates.len() != self.bounds.len() {
                return Err(LevelError::BadLine {
                    line: 0,
                    reason: format!(
                        "compiled {} relates against {} bounds; refusing before solve",
                        self.relates.len(),
                        self.bounds.len()
                    ),
                });
            }
            let declared: BTreeSet<u32> = self.declares.iter().map(|declare| declare.id).collect();
            for relate in &self.relates {
                if !declared.contains(&relate.prop) {
                    return Err(LevelError::BadLine {
                        line: 0,
                        reason: format!(
                            "relate names undeclared prop {}; refusing before solve",
                            relate.prop
                        ),
                    });
                }
            }
            for bound in &self.bounds {
                if !declared.contains(&bound.prop) {
                    return Err(LevelError::BadLine {
                        line: 0,
                        reason: format!(
                            "bound names undeclared prop {}; refusing before solve",
                            bound.prop
                        ),
                    });
                }
            }
            if !declared.contains(&self.spawn_id) {
                return Err(LevelError::BadLine {
                    line: 0,
                    reason: format!(
                        "spawn id {} names no declared prop; refusing before solve",
                        self.spawn_id
                    ),
                });
            }
            Ok(())
        }
    }

    /// Room half-extent in cm for point clamps (the 10 m square shell).
    const ROOM_HALF_CM: i32 = 500;

    fn target_rank(target: &Target) -> u8 {
        match target {
            Target::Wall(_) => 0,
            Target::Point { .. } => 1,
            Target::Object(_) => 2,
        }
    }

    fn clamp_room(value: i32) -> i32 {
        value.clamp(-ROOM_HALF_CM, ROOM_HALF_CM)
    }

    /// One compiled want entry: relation, resolved target, optional range
    /// in cm, and the source brief line conflicts echo.
    type WantEntry = (Relation, Target, Option<(i32, i32)>, String);
    /// Dedup key for one want entry: the entry without its source line.
    type WantKey = (Relation, Target, Option<(i32, i32)>);

    struct PropDraft {
        name: String,
        kind: String,
        id: u32,
        entries: Vec<WantEntry>,
    }

    fn draft_entries(
        parsed: &ParsedBrief,
        name: &str,
        index_of: &BTreeMap<&str, u32>,
    ) -> Result<Vec<WantEntry>, LevelError> {
        let raw_wants: &[RawWant] = parsed.wants.get(name).map_or(&[], Vec::as_slice);
        if raw_wants.len() > WANTS_MAX {
            return Err(LevelError::BadLine {
                line: 0,
                reason: format!(
                    "prop `{name}` holds {} wants; at most {WANTS_MAX}",
                    raw_wants.len()
                ),
            });
        }
        let mut out = Vec::with_capacity(raw_wants.len());
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
            out.push((raw.relation, target, raw.range, raw.source.clone()));
        }
        Ok(out)
    }

    /// Greedy compile repair over one prop's entries: dedup exact repeats
    /// (normalization, uncounted), then bounded self-target drop and room
    /// clamp (counted, at most what is left of `MAX_REPAIRS`). Order is
    /// preserved here; the caller sorts after repair.
    fn repair_entries(
        prop: &str,
        prop_id: u32,
        entries: Vec<WantEntry>,
        repairs: &mut u8,
        notes: &mut Vec<String>,
    ) -> Vec<WantEntry> {
        let mut seen: Vec<WantKey> = Vec::new();
        let mut unique = Vec::with_capacity(entries.len());
        for entry in entries {
            let (relation, target, range, source) = &entry;
            let key = (*relation, *target, *range);
            if seen.contains(&key) {
                notes.push(format!("dedup {prop} {source}"));
            } else {
                seen.push(key);
                unique.push(entry);
            }
        }
        let mut out = Vec::with_capacity(unique.len());
        for (relation, target, range, source) in unique {
            if matches!(target, Target::Object(id) if id == prop_id) && *repairs < MAX_REPAIRS {
                *repairs += 1;
                notes.push(format!("drop-self {prop} {source}"));
                continue;
            }
            if let Target::Point { x_cm, z_cm } = target {
                let (cx, cz) = (clamp_room(x_cm), clamp_room(z_cm));
                if (cx, cz) != (x_cm, z_cm) && *repairs < MAX_REPAIRS {
                    *repairs += 1;
                    notes.push(format!("clamp-point {prop} [{x_cm},{z_cm}]"));
                    out.push((
                        relation,
                        Target::Point { x_cm: cx, z_cm: cz },
                        range,
                        source,
                    ));
                    continue;
                }
            }
            out.push((relation, target, range, source));
        }
        out
    }

    /// Stable want order per prop: walls, points, objects, facing last.
    /// Line order survives inside each rank, so reordered brief lines still
    /// compile identically and the solver sees the cheapest constraints
    /// first. Pure order, never a counted repair.
    fn order_entries(entries: Vec<WantEntry>) -> Vec<WantEntry> {
        let mut ranked: Vec<(u8, u8, usize)> = entries
            .iter()
            .enumerate()
            .map(|(index, (relation, target, _, _))| {
                let facing = u8::from(*relation == Relation::Facing);
                (facing, target_rank(target), index)
            })
            .collect();
        // Index keys are unique, so the order is total and the unstable
        // sort repeats byte-identically on every machine.
        ranked.sort_unstable();
        ranked
            .into_iter()
            .map(|(_, _, index)| entries[index].clone())
            .collect()
    }

    /// Compile the envelope text into typed per-category constraints.
    /// Grammar failures refuse exactly like `plan_brief` did; no actor is
    /// placed here and no provider is consulted.
    pub fn compile_brief(env: &BriefEnvelope) -> Result<CompiledBrief, LevelError> {
        let parsed = parse_brief(env.text())?;
        let mut names: Vec<&String> = parsed.kinds.keys().collect();
        names.sort();
        let index_of: BTreeMap<&str, u32> = names
            .iter()
            .enumerate()
            .map(|(index, name)| (name.as_str(), u32::try_from(index + 1).unwrap_or(u32::MAX)))
            .collect();
        let mut drafts = Vec::with_capacity(names.len());
        for name in &names {
            let id = index_of[name.as_str()];
            if id == u32::MAX {
                return Err(LevelError::TooManyProps { count: names.len() });
            }
            let entries = draft_entries(&parsed, name, &index_of)?;
            drafts.push(PropDraft {
                name: (*name).clone(),
                kind: parsed.kinds[*name].clone(),
                id,
                entries,
            });
        }
        let spawn_name: &str = match &parsed.spawn {
            Some(name) => name,
            None => names[0],
        };
        let spawn_id = index_of[spawn_name];
        let mut repairs = 0u8;
        let mut notes = Vec::new();
        let mut declares = Vec::with_capacity(drafts.len());
        let mut relates = Vec::new();
        let mut bounds = Vec::new();
        for draft in &drafts {
            declares.push(DeclareConstraint {
                name: draft.name.clone(),
                kind: draft.kind.clone(),
                id: draft.id,
            });
        }
        for draft in drafts {
            let fixed = repair_entries(
                &draft.name,
                draft.id,
                draft.entries,
                &mut repairs,
                &mut notes,
            );
            let ordered = order_entries(fixed);
            for (want_idx, (relation, target, range, source)) in ordered.into_iter().enumerate() {
                let (lo, hi, defaulted) = match range {
                    Some((lo, hi)) => (lo, hi, false),
                    None => (0, 0, true),
                };
                relates.push(RelateConstraint {
                    prop: draft.id,
                    relation,
                    target,
                    source,
                });
                bounds.push(BoundConstraint {
                    prop: draft.id,
                    want_idx,
                    lo,
                    hi,
                    defaulted,
                });
            }
        }
        Ok(CompiledBrief {
            declares,
            relates,
            bounds,
            spawn_id,
            compile_repairs: repairs,
            notes,
        })
    }

    /// Lower compiled constraints to the solver plan. Pure reshape:
    /// props in id order, wants in compiled order, defaulted bounds become
    /// `None` so the solver applies its per-target defaults.
    pub fn plan_from_compiled(compiled: &CompiledBrief) -> RelationPlan {
        let kinds: BTreeMap<u32, (&str, &str)> = compiled
            .declares
            .iter()
            .map(|declare| (declare.id, (declare.name.as_str(), declare.kind.as_str())))
            .collect();
        let mut ids: Vec<u32> = kinds.keys().copied().collect();
        ids.sort_unstable();
        let mut props = Vec::with_capacity(ids.len());
        for id in ids {
            let (name, kind) = kinds[&id];
            let mut wants = Vec::new();
            let mut sources = Vec::new();
            for (relate, bound) in compiled.relates.iter().zip(compiled.bounds.iter()) {
                if relate.prop != id {
                    continue;
                }
                wants.push(Want {
                    relation: relate.relation,
                    target: relate.target,
                    distance_cm: if bound.defaulted {
                        None
                    } else {
                        Some((bound.lo, bound.hi))
                    },
                });
                sources.push(relate.source.clone());
            }
            props.push(PlannedProp {
                name: name.to_owned(),
                kind: kind.to_owned(),
                id,
                wants,
                sources,
            });
        }
        RelationPlan {
            props,
            spawn_id: compiled.spawn_id,
        }
    }

    /// Parse the envelope text into a relation plan via the constraint-first
    /// compile. Grammar failures refuse with the line and the legal values;
    /// the compiled form validates before lowering, so a broken compile
    /// refuses before the first solve. No actor is placed here.
    pub fn plan_brief(env: &BriefEnvelope) -> Result<RelationPlan, LevelError> {
        let compiled = compile_brief(env)?;
        compiled.validate()?;
        Ok(plan_from_compiled(&compiled))
    }

    /// Room shell: four thin grounded walls forming a 10 m square, the SP5
    /// room convention. Non-physical so the nav gate skips them while the
    /// overlap gate still covers them; corners touch at most, never overlap.
    pub fn room_shell() -> Vec<Actor> {
        let wall = |id: u32, name: &str, x: f32, z: f32, sx: f32, sz: f32| Actor {
            id,
            name: name.into(),
            x,
            y: 1.0,
            z,
            sx,
            sy: 2.0,
            sz,
            parent: None,
            physical: false,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        };
        vec![
            wall(SHELL_BASE, "wall_north", 0.0, -5.0, 9.8, 0.2),
            wall(SHELL_BASE + 1, "wall_south", 0.0, 5.0, 9.8, 0.2),
            wall(SHELL_BASE + 2, "wall_west", -5.0, 0.0, 0.2, 10.0),
            wall(SHELL_BASE + 3, "wall_east", 5.0, 0.0, 0.2, 10.0),
        ]
    }

    fn is_shell(id: u32) -> bool {
        id >= SHELL_BASE
    }

    fn stage_actor(plan: &PlannedProp, slot: usize) -> Actor {
        let (_, sy, _) = kind_size(&plan.kind).unwrap_or((1.0, 1.0, 1.0));
        let (sx, _, sz) = kind_size(&plan.kind).unwrap_or((1.0, 1.0, 1.0));
        Actor {
            id: plan.id,
            name: plan.name.clone(),
            x: -4.0 + (slot % 6) as f32 * 1.6,
            y: sy * 0.5,
            z: 3.2 - (slot / 6) as f32 * 1.6,
            sx,
            sy,
            sz,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        }
    }

    fn actor_with(actor: &Actor, solved: &engine_spatial::TransformOut) -> Actor {
        let mut next = actor.clone();
        next.x = solved.x;
        next.y = solved.y;
        next.z = solved.z;
        next.yaw = solved.yaw;
        next
    }

    /// Solve every planned prop against the shell plus the current positions.
    /// Passes run in name order until every prop settles or no pass makes
    /// progress; returns the actor list, the solve-call count and the still
    /// unsolved ids in ascending order.
    fn solve_plan(
        plan: &RelationPlan,
        fixed: &[Actor],
        solve_calls: &mut u32,
    ) -> (Vec<Actor>, BTreeSet<u32>) {
        let mut actors: Vec<Actor> = room_shell();
        actors.extend_from_slice(fixed);
        let staged: Vec<Actor> = plan
            .props
            .iter()
            .enumerate()
            .map(|(slot, prop)| stage_actor(prop, slot))
            .collect();
        actors.extend(staged);
        let mut solved: BTreeSet<u32> = BTreeSet::new();
        for _ in 0..PASSES_MAX {
            let mut progress = false;
            for prop in &plan.props {
                if solved.contains(&prop.id) {
                    continue;
                }
                let intent = Intent {
                    object: ObjectRef::Id(prop.id),
                    wants: prop.wants.clone(),
                };
                *solve_calls += 1;
                if let Ok(out) = engine_spatial::solve(&intent, &actors) {
                    if let Some(first) = out.first() {
                        if let Some(slot) = actors.iter().position(|a| a.id == prop.id) {
                            actors[slot] = actor_with(&actors[slot], first);
                        }
                        solved.insert(prop.id);
                        progress = true;
                    }
                }
            }
            if solved.len() == plan.props.len() || !progress {
                break;
            }
        }
        let unsolved: BTreeSet<u32> = plan
            .props
            .iter()
            .map(|prop| prop.id)
            .filter(|id| !solved.contains(id))
            .collect();
        (actors, unsolved)
    }

    fn finding_line(finding: &Finding) -> String {
        let peer = if finding.b == u32::MAX {
            "-".to_owned()
        } else {
            finding.b.to_string()
        };
        format!(
            "finding {} {} {} {} {}",
            finding.check, finding.a, peer, finding.severity, finding.pct
        )
    }

    fn tile_of(x_cm: i32, z_cm: i32) -> (i32, i32) {
        (x_cm.div_euclid(NAV_TILE_CM), z_cm.div_euclid(NAV_TILE_CM))
    }

    /// Integer-cm rect `(min_x, max_x, min_z, max_z)`.
    type Rect4 = (i32, i32, i32, i32);

    /// Shell wall rects plus the room interior (the union bounds).
    /// Deterministic, integer only.
    fn shell_shapes(actors: &[Actor]) -> (Vec<Rect4>, Rect4) {
        let mut rects = Vec::new();
        for actor in actors {
            if !is_shell(actor.id) {
                continue;
            }
            let hx = cm(actor.sx).abs() / 2;
            let hz = cm(actor.sz).abs() / 2;
            rects.push((
                cm(actor.x) - hx,
                cm(actor.x) + hx,
                cm(actor.z) - hz,
                cm(actor.z) + hz,
            ));
        }
        let mut interior = (i32::MAX, i32::MIN, i32::MAX, i32::MIN);
        for (min_x, max_x, min_z, max_z) in &rects {
            interior.0 = interior.0.min(*min_x);
            interior.1 = interior.1.max(*max_x);
            interior.2 = interior.2.min(*min_z);
            interior.3 = interior.3.max(*max_z);
        }
        (rects, interior)
    }

    fn inside(px: i32, pz: i32, rect: &Rect4) -> bool {
        rect.0 <= px && px <= rect.1 && rect.2 <= pz && pz <= rect.3
    }

    /// Bake the cell layout plus its nav view: one tile per square meter over
    /// the room plus an outer ring. A tile is walkable when its center stands
    /// on the room floor, clear of every shell wall; wall-adjacent placements
    /// (the normal SP5 case) stay walkable while the bands still bound the
    /// room. Props stay walkable by design (furniture-scale; crowd steering
    /// around them is the NV2 crowd layer, never this bake). This feeds the
    /// same `NavView` shape the NV1 mesh bake produces; the WD1 cell streamer
    /// owns async publish, and `cell_bytes` below is the bake input it reads.
    /// Wall bands block movement edges in `routes`, never tiles here, so the
    /// shared `navConnect` check verifies floor coverage while the route
    /// proof below verifies real reachability.
    pub fn bake_nav(actors: &[Actor], spawn_id: u32) -> NavView {
        let (shells, interior) = shell_shapes(actors);
        let mut tiles: BTreeMap<(i32, i32), Tile> = BTreeMap::new();
        for tx in -NAV_HALF..=NAV_HALF {
            for tz in -NAV_HALF..=NAV_HALF {
                let (cx, cz) = (
                    tx * NAV_TILE_CM + NAV_TILE_CM / 2,
                    tz * NAV_TILE_CM + NAV_TILE_CM / 2,
                );
                let in_room =
                    interior.0 <= cx && cx <= interior.1 && interior.2 <= cz && cz <= interior.3;
                let clear = !shells.iter().any(|rect| inside(cx, cz, rect));
                tiles.insert(
                    (tx, tz),
                    Tile {
                        walk: in_room && clear,
                    },
                );
            }
        }
        NavView {
            tiles,
            links: Vec::new(),
            flow: BTreeMap::new(),
            doors: BTreeMap::new(),
            spawn_id,
        }
    }

    /// One navigable route: BFS tiles from the spawn tile to a prop tile.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RouteInfo {
        /// Prop actor id.
        pub id: u32,
        /// Prop name from the brief.
        pub name: String,
        /// True when the prop tile is reachable from the spawn tile.
        pub reachable: bool,
        /// BFS distance in tiles (`u32::MAX` when unreachable).
        pub tiles: u32,
    }

    /// BFS routes from the spawn tile over walkable tiles. A step between two
    /// tiles is blocked when the shared edge midpoint stands inside a shell
    /// wall band, so thin walls seal routes even though tiles stay floor
    /// tiles. A missing or unwalkable spawn marks every route unreachable;
    /// distances are exact.
    pub fn routes(nav: &NavView, actors: &[Actor]) -> Vec<RouteInfo> {
        let id_of: BTreeMap<u32, &Actor> = actors.iter().map(|a| (a.id, a)).collect();
        let spawn_tile = id_of.get(&nav.spawn_id).map(|a| tile_of(cm(a.x), cm(a.z)));
        let (shells, _) = shell_shapes(actors);
        let walkable = |tile: &(i32, i32)| nav.tiles.get(tile).map(|t| t.walk).unwrap_or(false);
        let edge_open = |from: &(i32, i32), to: &(i32, i32)| {
            let mid = (
                from.0 * NAV_TILE_CM + (to.0 - from.0) * NAV_TILE_CM / 2 + NAV_TILE_CM / 2,
                from.1 * NAV_TILE_CM + (to.1 - from.1) * NAV_TILE_CM / 2 + NAV_TILE_CM / 2,
            );
            !shells.iter().any(|rect| inside(mid.0, mid.1, rect))
        };
        let mut dist: BTreeMap<(i32, i32), u32> = BTreeMap::new();
        if let Some(start) = spawn_tile {
            if walkable(&start) {
                let mut queue = VecDeque::from([start]);
                dist.insert(start, 0);
                while let Some(tile) = queue.pop_front() {
                    let here = dist[&tile];
                    for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                        let next = (tile.0 + dx, tile.1 + dz);
                        if dist.contains_key(&next) {
                            continue;
                        }
                        if !walkable(&next) || !edge_open(&tile, &next) {
                            continue;
                        }
                        dist.insert(next, here + 1);
                        queue.push_back(next);
                    }
                }
            }
        }
        let mut infos: Vec<RouteInfo> = actors
            .iter()
            .filter(|a| !is_shell(a.id) && a.id < LIVE_BASE)
            .map(|a| {
                let tile = tile_of(cm(a.x), cm(a.z));
                match dist.get(&tile).copied() {
                    Some(tiles) => RouteInfo {
                        id: a.id,
                        name: a.name.clone(),
                        reachable: true,
                        tiles,
                    },
                    None => RouteInfo {
                        id: a.id,
                        name: a.name.clone(),
                        reachable: false,
                        tiles: u32::MAX,
                    },
                }
            })
            .collect();
        infos.sort_by_key(|info| info.id);
        infos
    }

    struct GateReport {
        findings: Vec<Finding>,
        unsolved: BTreeSet<u32>,
        nav: NavView,
        routes: Vec<RouteInfo>,
    }

    /// Full gate: the 14 offline checks plus `navConnect`, `doorClear` and
    /// `flowPath` over the baked view, plus per-prop BFS routes. `accessFront`
    /// stays out: yaw is solver-chosen, and facing gates belong to encounter
    /// staging, never level layout.
    fn gates(actors: &[Actor], spawn_id: u32, unsolved: &BTreeSet<u32>) -> GateReport {
        let nav = bake_nav(actors, spawn_id);
        let mut findings: Vec<Finding> = engine_spatial::run(actors).findings;
        if let Ok(nav_report) = engine_spatial::run_spatial(
            actors,
            &["navConnect", "doorClear", "flowPath"],
            Some(&nav),
        ) {
            findings.extend(nav_report.findings);
        }
        let routes = routes(&nav, actors);
        GateReport {
            findings,
            unsolved: unsolved.clone(),
            nav,
            routes,
        }
    }

    fn gates_clean(report: &GateReport) -> bool {
        report.findings.is_empty()
            && report.unsolved.is_empty()
            && report.routes.iter().all(|route| route.reachable)
    }

    /// First prop id behind a gate failure: the earliest finding touching a
    /// planned prop, else the first unreachable prop, else the first unsolved
    /// prop. Live-document and shell actors are never culprits: the repair
    /// loop moves brief props only.
    fn culprit(report: &GateReport, plan: &RelationPlan) -> Option<u32> {
        let owned: BTreeSet<u32> = plan.props.iter().map(|prop| prop.id).collect();
        for finding in &report.findings {
            if owned.contains(&finding.a) {
                return Some(finding.a);
            }
            if finding.b != u32::MAX && owned.contains(&finding.b) {
                return Some(finding.b);
            }
        }
        if let Some(route) = report.routes.iter().find(|route| !route.reachable) {
            return Some(route.id);
        }
        report.unsolved.iter().copied().next()
    }

    fn resolve_single(
        actors: &mut [Actor],
        prop_id: u32,
        wants: &[Want],
        solve_calls: &mut u32,
    ) -> bool {
        let intent = Intent {
            object: ObjectRef::Id(prop_id),
            wants: wants.to_vec(),
        };
        *solve_calls += 1;
        match engine_spatial::solve(&intent, actors) {
            Ok(out) => {
                if let (Some(first), Some(slot)) =
                    (out.first(), actors.iter().position(|a| a.id == prop_id))
                {
                    actors[slot] = actor_with(&actors[slot], first);
                    return true;
                }
                false
            }
            Err(_) => false,
        }
    }

    /// One repair round over the culprit prop. Round 0 drops its last want,
    /// round 1 resets every range to the solver default, round 2 relocates it
    /// to the first clean cell; the last prop is never removed, so a level
    /// keeps at least one prop and a stuck single-prop brief refuses instead
    /// of voiding itself. Every applied round counts, even a failed re-solve.
    /// A re-solved culprit leaves the `unsolved` set.
    fn repair_round(
        plan: &mut RelationPlan,
        actors: &mut [Actor],
        round: u8,
        culprit: u32,
        unsolved: &mut BTreeSet<u32>,
        solve_calls: &mut u32,
    ) -> bool {
        let multi = plan.props.len() > 1;
        let Some(entry) = plan.props.iter_mut().find(|prop| prop.id == culprit) else {
            return false;
        };
        match round {
            0 => {
                if entry.wants.is_empty() {
                    return false;
                }
                entry.wants.pop();
                entry.sources.pop();
                let wants = entry.wants.clone();
                if resolve_single(actors, culprit, &wants, solve_calls) {
                    unsolved.remove(&culprit);
                }
                true
            }
            1 => {
                for want in &mut entry.wants {
                    want.distance_cm = None;
                }
                let wants = entry.wants.clone();
                if resolve_single(actors, culprit, &wants, solve_calls) {
                    unsolved.remove(&culprit);
                }
                true
            }
            _ => {
                if !multi {
                    return true;
                }
                entry.wants.clear();
                entry.sources.clear();
                if resolve_single(actors, culprit, &[], solve_calls) {
                    unsolved.remove(&culprit);
                }
                true
            }
        }
    }

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
        let (mut actors, mut unsolved) = solve_plan(&plan, fixed, &mut solve_calls);
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
        let mut findings: Vec<String> = report.findings.iter().map(finding_line).collect();
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
        BuildOutcome::Refused(LevelRefusal {
            brief_id: env.brief_id(),
            stage: env.stage().as_str(),
            repairs_used,
            conflicts,
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

    /// Y yaw extracted from a world quaternion.
    fn yaw_of_quat(q: [f32; 4]) -> f32 {
        let [x, y, z, w] = q;
        (2.0 * (w * y + x * z)).atan2(1.0 - 2.0 * (y * y + z * z))
    }

    /// Pure-Y rotation quaternion for a solved yaw.
    fn quat_of_yaw(yaw: f32) -> [f32; 4] {
        let half = 0.5 * yaw;
        [0.0, half.sin(), 0.0, half.cos()]
    }

    /// Sorted-id snapshot of the live document as fixed solver actors: every
    /// node with a finite world position becomes one unit cube, like the
    /// `scene.place` mapping. Live actors constrain the solve but are never
    /// moved and never culprits.
    fn snapshot_live(doc: &engine_editor::commands::Document) -> Vec<Actor> {
        let mut order: Vec<ObjectId> = Vec::new();
        let mut stack = doc.hierarchy.roots();
        while let Some(id) = stack.pop() {
            order.push(id);
            stack.extend(doc.hierarchy.children(id));
        }
        order.sort_by_key(|id| id.to_string());
        let mut actors = Vec::new();
        let mut next_id = LIVE_BASE;
        for id in order {
            if next_id >= SHELL_BASE {
                break;
            }
            let Some(world) = doc.hierarchy.world(id) else {
                continue;
            };
            if !world.position.iter().all(|lane| lane.is_finite()) {
                continue;
            }
            let yaw = if world.rotation.iter().all(|lane| lane.is_finite()) {
                yaw_of_quat(world.rotation)
            } else {
                0.0
            };
            actors.push(Actor {
                id: next_id,
                name: format!("live_{next_id:06}"),
                x: world.position[0],
                y: world.position[1],
                z: world.position[2],
                sx: 1.0,
                sy: 1.0,
                sz: 1.0,
                parent: None,
                physical: true,
                vx: 0.0,
                vz: 0.0,
                yaw,
            });
            next_id += 1;
        }
        actors
    }

    fn bad_brief(error: LevelError) -> McpToolError {
        McpToolError::new("bad_brief", error.to_string())
    }

    /// Door args for `level.build`.
    #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
    #[serde(crate = "rmcp::serde")]
    #[schemars(crate = "rmcp::schemars")]
    pub struct BuildLevelArgs {
        /// Brief text in the `prop`/`want`/`spawn` grammar.
        pub brief: String,
        /// Build seed naming this attempt.
        pub seed: u64,
        /// Must equal the document revision, else nothing runs.
        pub expected_revision: u64,
        /// Retry identity for the landed batch.
        pub idempotency_key: u64,
    }

    /// One landed prop with its live document id.
    #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
    #[serde(crate = "rmcp::serde")]
    #[schemars(crate = "rmcp::schemars")]
    pub struct PlacedDto {
        /// Live document id (UUID text).
        pub id: String,
        /// Prop name from the brief.
        pub name: String,
        /// Position meters.
        pub x: f32,
        /// Grounded height meters.
        pub y: f32,
        /// Position meters.
        pub z: f32,
        /// Facing yaw radians.
        pub yaw: f32,
    }

    /// Door output for `level.build`: solved placements plus the AB4 input
    /// line, or the refusal with conflicts and findings. Undo rides the
    /// existing `tx.undo` tool with `undo_token`.
    #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
    #[serde(crate = "rmcp::serde")]
    #[schemars(crate = "rmcp::schemars")]
    pub struct BuildLevelOutput {
        /// Document revision after the call (unchanged on refusal).
        pub revision: u64,
        /// True when the level placed.
        pub solved: bool,
        /// Landed props with live ids.
        pub placements: Vec<PlacedDto>,
        /// Gate finding lines (empty on a pass).
        pub findings: Vec<String>,
        /// Culprit want source lines on a refusal.
        pub conflicts: Vec<String>,
        /// Why the build refused, empty on a pass.
        pub reason: String,
        /// Repair rounds consumed.
        pub repairs_used: u8,
        /// `"pass"` or `"refused"`.
        pub verdict: String,
        /// AB4 input line on a pass, empty on a refusal.
        pub next_ab4: String,
        /// Simulated cost units.
        pub cost_units: u64,
        /// Landed batch key, zero on a refusal.
        pub key: u64,
        /// Stable undo identity for `tx.undo`, zero on a refusal.
        pub undo_token: u64,
        /// Shared LW6 envelope record line (`BriefEnvelope::record_line`).
        /// Every door write carries it so the audit traces end to end.
        #[serde(default)]
        #[schemars(default)]
        pub record_line: String,
    }

    /// Run the level chain against the live document: snapshot the scene as
    /// fixed actors, solve, gate, then land inserts through validate plus the
    /// CAP1-gated apply exactly like `scene.place`. Denied, stale and refused
    /// calls write nothing.
    fn build_level_on_state(
        state: &mut ServerState,
        args: &BuildLevelArgs,
    ) -> Result<BuildLevelOutput, McpToolError> {
        if args.expected_revision != state.doc.revision {
            return Err(McpToolError::new(
                "stale_revision",
                format!(
                    "document is at revision {}, intent expects {}; \
                     re-read scene.brief, then build again",
                    state.doc.revision, args.expected_revision
                ),
            ));
        }
        let env = envelope(&args.brief, args.seed).map_err(bad_brief)?;
        let plan = plan_brief(&env).map_err(bad_brief)?;
        let live = snapshot_live(&state.doc);
        match build_on(&plan, &env, &live) {
            BuildOutcome::Refused(refusal) => Ok(BuildLevelOutput {
                revision: state.doc.revision,
                solved: false,
                placements: Vec::new(),
                findings: refusal.findings,
                conflicts: refusal.conflicts,
                reason: refusal.reason,
                repairs_used: refusal.repairs_used,
                verdict: "refused".to_owned(),
                next_ab4: String::new(),
                cost_units: 0,
                key: 0,
                undo_token: 0,
                record_line: env.record_line(),
            }),
            BuildOutcome::Placed(level) => {
                let sizes: BTreeMap<u32, (f32, f32, f32)> = level
                    .actors
                    .iter()
                    .map(|actor| (actor.id, (actor.sx, actor.sy, actor.sz)))
                    .collect();
                let mut ids = Vec::with_capacity(level.report.placements.len());
                let mut ops = Vec::with_capacity(level.report.placements.len());
                for placement in &level.report.placements {
                    let id = ObjectId::new();
                    let (sx, sy, sz) = sizes.get(&placement.id).copied().unwrap_or((1.0, 1.0, 1.0));
                    ids.push((id, placement.name.clone()));
                    ops.push(Op::Insert {
                        id,
                        parent: ObjectId::NIL,
                        local: Transform::new(
                            [placement.x, placement.y, placement.z],
                            quat_of_yaw(placement.yaw),
                            [sx, sy, sz],
                        ),
                    });
                }
                let batch = Batch {
                    ops,
                    base_revision: state.doc.revision,
                    idempotency_key: args.idempotency_key,
                    provenance: Some(ProvenanceStamp::script("level.build")),
                };
                let _preview =
                    commands::validate(&state.doc, &batch).map_err(|error| batch_error(&error))?;
                let approval = approve(
                    state.session,
                    &batch,
                    state.doc.revision,
                    state.doc.revision,
                );
                let granted = granted_ops(&state.cap);
                let applied = capability::apply_gated(
                    &mut state.doc,
                    &mut state.journal,
                    &batch,
                    &mut state.cap,
                    &approval,
                    &mut state.ledger,
                )
                .map_err(|error| cap_error(&error, &granted))?;
                // A landed level is a watch event: move the session cache.
                state.refresh_watch();
                let placements: Vec<PlacedDto> = ids
                    .iter()
                    .zip(level.report.placements.iter())
                    .map(|((id, name), placement)| PlacedDto {
                        id: id.to_string(),
                        name: name.clone(),
                        x: placement.x,
                        y: placement.y,
                        z: placement.z,
                        yaw: placement.yaw,
                    })
                    .collect();
                Ok(BuildLevelOutput {
                    revision: applied.revision,
                    solved: true,
                    placements,
                    findings: Vec::new(),
                    conflicts: Vec::new(),
                    reason: String::new(),
                    repairs_used: level.report.repairs_used,
                    verdict: "pass".to_owned(),
                    next_ab4: level.report.next_ab4.clone(),
                    cost_units: level.report.cost.units,
                    key: applied.key,
                    undo_token: applied.key,
                    record_line: env.record_line(),
                })
            }
        }
    }

    /// `level.build` door tool: brief to placed level through the command
    /// gate, dispatched like every sibling so the taxonomy fronts the name
    /// and the audit seals one record per call. Undo rides `tx.undo`.
    pub struct BuildLevelTool;

    impl ToolBase for BuildLevelTool {
        type Parameter = BuildLevelArgs;
        type Output = BuildLevelOutput;
        type Error = McpToolError;

        fn name() -> Cow<'static, str> {
            TOOL_BUILD_LEVEL.into()
        }

        fn description() -> Option<Cow<'static, str>> {
            Some(
                "Build a level from a text brief: relation plan, SP5 placement and cell \
                 layout with nav checks, applied through the command gate; conflicts and \
                 findings on refusal."
                    .into(),
            )
        }

        fn annotations() -> Option<ToolAnnotations> {
            let mut annotations = ToolAnnotations::default();
            annotations.read_only_hint = Some(false);
            annotations.idempotent_hint = Some(true);
            annotations.open_world_hint = Some(false);
            Some(annotations)
        }
    }

    impl SyncTool<McpServer> for BuildLevelTool {
        fn invoke(
            service: &McpServer,
            param: Self::Parameter,
        ) -> Result<Self::Output, Self::Error> {
            service.dispatch(TOOL_BUILD_LEVEL, |state| {
                build_level_on_state(state, &param)
            })
        }
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
