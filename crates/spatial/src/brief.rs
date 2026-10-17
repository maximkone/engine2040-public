//! Bounded, token-budgeted scene brief: stable IDs, names, groups, relations
//! and findings for any model.
//!
//! Consumer: the `scene.brief` tool, the placement solver's context and the
//! eval harness. The caller owns the data: input is a plain actor snapshot
//! plus the relation [`Graph`] and the overlap [`Report`], output is a plain
//! [`Brief`]. Units are actor meters in, integer centimeters inside
//! (inherited from checks/relations); the brief itself carries no units,
//! only ids, names and counts. Determinism scope: the same actor set, graph
//! and report in any input order give a byte-identical [`brief_text`]. Failure mode: budget pressure drops whole
//! rows only, lowest priority first; the digest line is mandatory identity
//! and always emitted.
//!
//! Priority (keep first): red findings, then relations, then yellow findings,
//! then objects, then groups. Within a tier the caller's frozen order wins
//! and the tail is dropped first. Budget is measured as `brief_text` bytes,
//! so the output fits `budget_bytes` whenever the digest line alone fits it.
//!
//! Collapsed room rows group parent-linked actors per forest root, and
//! per-room `expand_room_text` reuses the [`brief`] budget path.
//!
//! Std-only. Never imports scene or editor types.

use super::relation::{Graph, Layer, Relation};
use super::{Actor, Report};
use engine_core::Fnv64;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// One stable group: layer name plus sorted member ids. Groups partition the
/// input actor ids by their graph layer (`arena`, `zone`, `object`), so every
/// id appears exactly once and the rows stay bounded (at most three).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    pub member_ids: Vec<u32>,
}

/// Longest kept actor name in an object row, in chars.
///
/// Bounds one row so a single hostile label cannot eat the whole brief
/// budget; longer names truncate (the bracketed id stays exact, so identity
/// never collides).
pub const MAX_NAME_CHARS: usize = 64;

/// One staged object label: display name plus the stable numeric id.
///
/// Consumer: agents reading [`brief_text`] match rows by the bracketed id;
/// the name part is display-only and never decides placement, so two objects
/// sharing a name still resolve to different ids (the tool-layer `name [id]`
/// pattern, reimplemented here on the spatial render path). Units: none;
/// `id` is the caller-supplied actor id. Determinism scope: id-sorted,
/// first-wins on duplicate ids, sanitized names; same actor set in any input
/// order gives the same labels. Errors: none; degenerate names degrade to
/// `-`, never panic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectEntry {
    /// Stable actor id (the only resolution key).
    pub id: u32,
    /// Sanitized display name (single line, no brackets, at most
    /// [`MAX_NAME_CHARS`] chars).
    pub name: String,
}

/// Sanitize one actor name into a single-line bracket-free token.
///
/// Trims surrounding whitespace, maps `[`, `]`, and every control char
/// (including `\n` and `\r`) to `_`, truncates to [`MAX_NAME_CHARS`] chars,
/// and degrades empty input to `-`. The bracketed id in the rendered row
/// carries identity, so this mapping is display-only and never needs to be
/// reversible.
fn sanitize_name(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "-".to_string();
    }
    let mut cleaned = String::with_capacity(trimmed.len().min(MAX_NAME_CHARS));
    for ch in trimmed.chars() {
        if ch == '[' || ch == ']' || ch.is_control() {
            cleaned.push('_');
        } else {
            cleaned.push(ch);
        }
        if cleaned.chars().count() >= MAX_NAME_CHARS {
            break;
        }
    }
    if cleaned.is_empty() {
        "-".to_string()
    } else {
        cleaned
    }
}

/// Bounded scene summary. `digest` is the digest of the same relation graph;
/// `findings` stores kept reds first (report order), then kept yellows.
/// `objects` stores kept id-sorted `name [id]` labels so duplicate names
/// never mis-place; `groups` stores kept layer rows.
/// `PartialEq` only: findings carry an `f32` percent, so no `Eq`.
#[derive(Debug, Clone, PartialEq)]
pub struct Brief {
    pub groups: Vec<Group>,
    pub relations: Vec<(u32, Relation, u32)>,
    pub findings: Vec<super::Finding>,
    pub digest: u64,
    /// Kept object labels in id order (prefix of the full id-sorted set).
    pub objects: Vec<ObjectEntry>,
}

fn digest_line(digest: u64) -> String {
    format!("digest {digest}\n")
}

fn group_line(group: &Group) -> String {
    let ids: Vec<String> = group.member_ids.iter().map(|id| id.to_string()).collect();
    format!("group {} {}\n", group.name, ids.join(","))
}

fn relation_line(edge: &(u32, Relation, u32)) -> String {
    format!("relation {} {} {}\n", edge.0, edge.1.name(), edge.2)
}

fn finding_line(finding: &super::Finding) -> String {
    format!("{}\n", finding.row())
}

fn object_line(entry: &ObjectEntry) -> String {
    format!("object {} [{}]\n", entry.name, entry.id)
}

/// Id-sorted object labels over the input actors.
///
/// First-wins on duplicate ids (ties broken by name, so any input order
/// gives the same labels); names are sanitized display-only tokens, so two
/// actors sharing a name still render distinct rows by id.
fn object_labels(actors: &[Actor]) -> Vec<ObjectEntry> {
    let mut by_id: BTreeMap<u32, String> = BTreeMap::new();
    let mut sorted: Vec<&Actor> = actors.iter().collect();
    sorted.sort_by(|a, b| (a.id, &a.name).cmp(&(b.id, &b.name)));
    for actor in sorted {
        by_id
            .entry(actor.id)
            .or_insert_with(|| sanitize_name(&actor.name));
    }
    by_id
        .into_iter()
        .map(|(id, name)| ObjectEntry { id, name })
        .collect()
}

/// Layer groups over the input actors. Membership comes from `actors`
/// (id-sorted, deduped) and layers from the graph; ids missing from the
/// graph degrade to `object`, matching the graph's isolated-node convention.
/// Fixed order `arena`, `zone`, `object`; empty layers emit no row.
fn layer_groups(actors: &[Actor], graph: &Graph) -> Vec<Group> {
    let layer_of: BTreeMap<u32, Layer> = graph
        .nodes
        .iter()
        .map(|(id, layer)| (*id, *layer))
        .collect();
    let mut buckets: BTreeMap<Layer, BTreeSet<u32>> = BTreeMap::new();
    for actor in actors {
        let layer = layer_of.get(&actor.id).copied().unwrap_or(Layer::Object);
        buckets.entry(layer).or_default().insert(actor.id);
    }
    let names = [
        (Layer::Arena, "arena"),
        (Layer::Zone, "zone"),
        (Layer::Object, "object"),
    ];
    names
        .into_iter()
        .filter_map(|(layer, name)| {
            buckets.get(&layer).map(|ids| Group {
                name: name.to_string(),
                member_ids: ids.iter().copied().collect(),
            })
        })
        .collect()
}

/// Forest roots over parent links: each actor follows `Actor.parent` with a
/// visited set, so a cycle keeps its entry point as its own root and a
/// dangling parent stops at the last known actor. Groups member ids per root
/// with roots and members ascending. Integer-only and sorted.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "room collapse ships after the door consumes it")
)]
fn room_rows(actors: &[Actor]) -> Vec<(u32, Vec<u32>)> {
    let mut sorted: Vec<&Actor> = actors.iter().collect();
    sorted.sort_by(|a, b| (a.id, &a.name, a.parent).cmp(&(b.id, &b.name, b.parent)));
    let mut parent_of: BTreeMap<u32, Option<u32>> = BTreeMap::new();
    for actor in sorted {
        parent_of.entry(actor.id).or_insert(actor.parent);
    }
    let mut groups: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    for start in parent_of.keys().copied() {
        let mut cur = start;
        let mut visited: BTreeSet<u32> = BTreeSet::new();
        visited.insert(cur);
        while let Some(Some(next)) = parent_of.get(&cur).copied() {
            if !parent_of.contains_key(&next) {
                break;
            }
            if !visited.insert(next) {
                cur = start;
                break;
            }
            cur = next;
        }
        groups.entry(cur).or_default().insert(start);
    }
    groups
        .into_iter()
        .map(|(root, set)| (root, set.into_iter().collect()))
        .collect()
}

/// One collapsed room row: counts only, never the member list, so thousands
/// of objects collapse to dozens of lines.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "room collapse ships after the door consumes it")
)]
fn room_line(root: u32, count: usize) -> String {
    format!("room {root} {count}\n")
}

/// Room slice: the room root plus its members in id order, empty when the
/// room is unknown. Deterministic in any input order via sorted first-wins.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "room collapse ships after the door consumes it")
)]
fn expand_slice(actors: &[Actor], room: u32) -> Vec<Actor> {
    let rows = room_rows(actors);
    let wanted = rows
        .iter()
        .find_map(|(root, members)| (*root == room).then_some(members));
    let ids = match wanted {
        Some(ids) => ids,
        None => return Vec::new(),
    };
    let mut sorted: Vec<&Actor> = actors.iter().collect();
    sorted.sort_by(|a, b| (a.id, &a.name).cmp(&(b.id, &b.name)));
    let mut by_id: BTreeMap<u32, &Actor> = BTreeMap::new();
    for actor in sorted {
        by_id.entry(actor.id).or_insert(actor);
    }
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(actor) = by_id.get(id) {
            out.push((*actor).clone());
        }
    }
    out
}

/// Collapsed room text: one [`room_line`] per [`room_rows`] entry in order.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "room collapse ships after the door consumes it")
)]
fn collapsed_room_text(actors: &[Actor]) -> String {
    let rows = room_rows(actors);
    let mut out = String::new();
    for (root, members) in &rows {
        out.push_str(&room_line(*root, members.len()));
    }
    out
}

/// Per-room brief text: filter with [`expand_slice`] then reuse the [`brief`]
/// plus [`brief_text`] budget and truncation contract.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "room collapse ships after the door consumes it")
)]
fn expand_room_text(
    actors: &[Actor],
    graph: &Graph,
    report: &Report,
    room: u32,
    budget: usize,
) -> String {
    let subset = expand_slice(actors, room);
    brief_text(&brief(&subset, graph, report, budget))
}

/// Bounded brief. Renders each candidate row once, then keeps a prefix per
/// tier in priority order (reds, relations, yellows, objects, groups) while
/// the whole `brief_text` still fits `budget_bytes`. Whole rows only: a row
/// is kept only if the total with it fits. The digest row is mandatory
/// identity and is always emitted, even under a pathological budget below
/// its own size.
pub fn brief(actors: &[Actor], graph: &Graph, report: &Report, budget_bytes: usize) -> Brief {
    let digest = super::relation::digest(graph);
    let groups = layer_groups(actors, graph);
    let objects = object_labels(actors);
    let relations: Vec<(u32, Relation, u32)> = {
        let mut edges = graph.edges.clone();
        edges.sort_by_key(|(a, r, b)| (*a, *r, *b));
        edges
    };
    let mut reds: Vec<super::Finding> = Vec::new();
    let mut yellows: Vec<super::Finding> = Vec::new();
    for finding in &report.findings {
        if finding.severity == "red" {
            reds.push(finding.clone());
        } else {
            yellows.push(finding.clone());
        }
    }

    let group_lines: Vec<String> = groups.iter().map(group_line).collect();
    let object_lines: Vec<String> = objects.iter().map(object_line).collect();
    let relation_lines: Vec<String> = relations.iter().map(relation_line).collect();
    let red_lines: Vec<String> = reds.iter().map(finding_line).collect();
    let yellow_lines: Vec<String> = yellows.iter().map(finding_line).collect();

    // Fill in priority order; each row joins only if the running total fits.
    let mut used = digest_line(digest).len();
    let mut fit = |lines: &[String]| -> usize {
        let mut kept = 0;
        for line in lines {
            if used + line.len() <= budget_bytes {
                used += line.len();
                kept += 1;
            } else {
                break;
            }
        }
        kept
    };
    let keep_reds = fit(&red_lines);
    let keep_relations = fit(&relation_lines);
    let keep_yellows = fit(&yellow_lines);
    let keep_objects = fit(&object_lines);
    let keep_groups = fit(&group_lines);

    Brief {
        groups: groups.into_iter().take(keep_groups).collect(),
        relations: relations.into_iter().take(keep_relations).collect(),
        findings: reds
            .into_iter()
            .take(keep_reds)
            .chain(yellows.into_iter().take(keep_yellows))
            .collect(),
        digest,
        objects: objects.into_iter().take(keep_objects).collect(),
    }
}

/// Stable line-ordered rendering for golden tests: digest, then objects as
/// `name [id]`, then groups, then relations, then findings (reds before
/// yellows). Every line ends with `\n`; single-actor findings render `b`
/// as `-`; object names are sanitized single-line tokens.
pub fn brief_text(brief: &Brief) -> String {
    let mut out = digest_line(brief.digest);
    for entry in &brief.objects {
        out.push_str(&object_line(entry));
    }
    for group in &brief.groups {
        out.push_str(&group_line(group));
    }
    for edge in &brief.relations {
        out.push_str(&relation_line(edge));
    }
    for finding in &brief.findings {
        out.push_str(&finding_line(finding));
    }
    out
}

/// Token estimator for brief text: a stable planning number in model tokens.
///
/// NOT a vendor tokenizer: no BPE tables, no network, no per-model counts.
/// Formula is `max(ceil(chars / 4), words)` where `chars` counts Unicode
/// scalar values and `words` counts maximal runs of non-whitespace chars.
/// Dense code (long runs, few spaces) prices by chars; short-word prose
/// prices by words, so whitespace/code density steers the estimate. Units:
/// estimated tokens (`u32`). Determinism scope: pure function of the text;
/// the same text gives the same count on every platform and build. Total:
/// saturating counters clamp at `u32::MAX`; never panics.
#[must_use]
pub fn estimate_tokens(text: &str) -> u32 {
    let mut chars: u64 = 0;
    let mut words: u64 = 0;
    let mut in_word = false;
    for ch in text.chars() {
        chars = chars.saturating_add(1);
        if ch.is_whitespace() {
            in_word = false;
        } else if !in_word {
            in_word = true;
            words = words.saturating_add(1);
        }
    }
    // ceil(chars / 4) via a saturating bump: appending text never lowers
    // either arm, so the estimate is monotonic in the input.
    let by_chars = chars.saturating_add(3) / 4;
    let tokens = by_chars.max(words);
    u32::try_from(tokens).unwrap_or(u32::MAX)
}

/// Budget and cost sidecar for one assembled brief: the byte budget the
/// caller offered plus the token estimate over the final text.
///
/// Consumer: agents planning context windows and M2 cost reporting read both
/// numbers side by side. The caller owns the data: `budget_bytes` echoes the
/// offered budget unchanged (same semantics as [`brief`]), `estimated_tokens`
/// is [`estimate_tokens`] over [`brief_text`] of the kept rows. Units: bytes
/// for the budget, estimated model tokens for the cost. Determinism scope:
/// the same brief and budget give the same stats. Serialization is additive:
/// `estimated_tokens` is new, so readers must default it (zero, or recompute
/// from the text) when absent; old rows without it still decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BriefStats {
    /// Offered brief-text budget in bytes (echoes the [`brief`] argument).
    pub budget_bytes: usize,
    /// [`estimate_tokens`] over the final [`brief_text`], in tokens.
    pub estimated_tokens: u32,
}

/// Stats for one assembled brief: the offered budget next to the token cost.
///
/// Renders [`brief_text`] once and prices it with [`estimate_tokens`]; the
/// budget echoes the caller's offer unchanged. Units are bytes in, estimated
/// tokens out. Determinism scope: the same brief and budget give the same
/// stats. Errors: none.
#[must_use]
pub fn brief_stats(brief: &Brief, budget_bytes: usize) -> BriefStats {
    BriefStats {
        budget_bytes,
        estimated_tokens: estimate_tokens(&brief_text(brief)),
    }
}

/// Digest tag so brief-parity bytes never alias another digest domain.
const PARITY_TAG: &[u8] = b"e2brief-parity:1";

/// MCP door provider the parity table names: `mock`, `local`, `remote`.
///
/// The brief pipeline takes no model bytes — providers stay adapters outside
/// the data path — so the label only names [`ParityRow`]s. The stress sweep
/// renders every adversarial scene once per label and requires byte-identical
/// text: the day a provider-shaped input reaches this module, the table fails
/// loudly instead of drifting silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BriefProvider {
    Mock,
    Local,
    Remote,
}

impl BriefProvider {
    /// Stable lowercase door label (`mock`, `local`, `remote`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mock => "mock",
            Self::Local => "local",
            Self::Remote => "remote",
        }
    }

    /// Every door label, frozen order.
    pub const ALL: [Self; 3] = [Self::Mock, Self::Local, Self::Remote];
}

/// One parity-table row: the provider label plus the digest and length of
/// the brief text rendered under it. Units: `digest` is FNV-1a over the
/// tagged text bytes, `bytes` is the text length in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParityRow {
    /// Door label this row was rendered under.
    pub provider: BriefProvider,
    /// Digest of the rendered brief text (tagged, see [`parity_table`]).
    pub digest: u64,
    /// Length of the rendered brief text in bytes.
    pub bytes: usize,
}

/// Rendered parity table: the shared brief text plus one row per provider.
///
/// Consumer: eval harnesses printing the provider-parity column. The caller
/// owns the data: the same actor set, graph and report give the same table
/// on every provider and in any input order. Units are actor meters in,
/// integer centimeters inside (inherited); the table itself carries text
/// bytes and digests only. Determinism scope: fresh renders per call with no
/// memoization, so a hidden order dependence fails [`parity_ok`] instead of
/// hiding. Errors: none; budget pressure truncates per [`brief`] and the
/// rows still compare (a truncated table agrees loudly).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParityTable {
    /// Brief text of the shared render; the rows prove three further renders
    /// match it byte for byte (see [`parity_ok`]).
    pub text: String,
    /// One row per [`BriefProvider`], in [`BriefProvider::ALL`] order.
    pub rows: Vec<ParityRow>,
}

/// Tagged digest of brief text for parity rows. Pure integer math over the
/// tag plus the text bytes: identical on every platform and build, and out
/// of every other digest domain by construction of the tag.
fn parity_digest(text: &str) -> u64 {
    let mut state = Fnv64::new();
    state.write(PARITY_TAG);
    state.write_str(text);
    state.finish()
}

/// Render the same scene once per provider and tabulate the outcome.
///
/// The shared text renders first; each label then re-runs the full
/// [`brief`] and [`brief_text`] path from scratch, so the table proves the
/// provider label never enters the bytes rather than assuming it. Gate on
/// the [`parity_ok`] check: any digest or length mismatch is a parity break
/// (a provider-shaped input reached geometry) and it must be named by the
/// caller, never averaged away.
#[must_use]
pub fn parity_table(
    actors: &[Actor],
    graph: &Graph,
    report: &Report,
    budget_bytes: usize,
) -> ParityTable {
    let text = brief_text(&brief(actors, graph, report, budget_bytes));
    let mut rows = Vec::with_capacity(BriefProvider::ALL.len());
    for provider in BriefProvider::ALL {
        // The label is intentionally unread: the pipeline has no provider
        // input, and threading one through would break parity by design.
        let round = brief_text(&brief(actors, graph, report, budget_bytes));
        rows.push(ParityRow {
            provider,
            digest: parity_digest(&round),
            bytes: round.len(),
        });
    }
    ParityTable { text, rows }
}

/// True when every parity row agrees on digest and length, with at least one
/// row present. An empty table never passes: a missing provider is a gap in
/// the evidence, not agreement.
#[must_use]
pub fn parity_ok(rows: &[ParityRow]) -> bool {
    match rows.split_first() {
        None => false,
        Some((first, rest)) => rest
            .iter()
            .all(|row| row.digest == first.digest && row.bytes == first.bytes),
    }
}

/// Checked-budget refusal: the whole scene does not fit `budget_bytes`.
///
/// Consumer: door layers refusing an overfull brief loudly instead of
/// shipping a silent truncation. The caller owns the data: every count is
/// read back from the two renders this refusal compares, and `hint` states
/// the exact minimum budget that fits. Units are bytes for budgets, rows for
/// counts. Determinism scope: the same scene and budget give the same
/// refusal. Errors: none beyond this value itself (it *is* the refusal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefShortfall {
    /// Budget the caller offered, in bytes.
    pub budget_bytes: usize,
    /// Bytes the whole scene needs (`brief_text` at full budget).
    pub need_bytes: usize,
    /// Object rows shed by the offered budget.
    pub dropped_objects: usize,
    /// Relation rows shed by the offered budget.
    pub dropped_relations: usize,
    /// Finding rows shed by the offered budget.
    pub dropped_findings: usize,
    /// Group rows shed by the offered budget.
    pub dropped_groups: usize,
    /// Single-line repair: the exact minimum budget plus what was shed.
    pub hint: String,
}

impl fmt::Display for BriefShortfall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.hint)
    }
}

impl std::error::Error for BriefShortfall {}

/// Checked brief construction: the whole scene or a loud refusal.
///
/// Returns the [`brief`] truncation when it covers every row, else
/// [`BriefShortfall`] with the exact shortfall and a hint naming the minimum
/// budget that fits. Use this on the checked-write path (the answer carries
/// the check that ran and the value read back); [`brief`] keeps its
/// truncation contract for readers that degrade gracefully.
///
/// ```
/// use engine_spatial::{brief::brief_checked, relation, run, Actor};
///
/// let actors = vec![Actor {
///     id: 0,
///     name: "arena".into(),
///     x: 0.0,
///     y: 2.0,
///     z: 0.0,
///     sx: 20.0,
///     sy: 4.0,
///     sz: 20.0,
///     parent: None,
///     physical: true,
///     vx: 0.0,
///     vz: 0.0,
///     yaw: 0.0,
/// }];
/// let graph = relation::build(&actors);
/// let report = run(&actors);
/// let result = brief_checked(&actors, &graph, &report, usize::MAX);
/// assert!(result.is_ok(), "one arena fits any sane budget");
/// if let Ok(kept) = result {
///     assert!(kept.objects.iter().any(|entry| entry.id == 0));
/// }
/// ```
#[must_use = "a refused brief must be handled, never dropped"]
pub fn brief_checked(
    actors: &[Actor],
    graph: &Graph,
    report: &Report,
    budget_bytes: usize,
) -> Result<Brief, BriefShortfall> {
    let full = brief(actors, graph, report, usize::MAX);
    let need_bytes = brief_text(&full).len();
    if need_bytes <= budget_bytes {
        return Ok(full);
    }
    let kept = brief(actors, graph, report, budget_bytes);
    let kept_bytes = brief_text(&kept).len();
    let short_by = need_bytes - budget_bytes;
    let dropped_objects = full.objects.len() - kept.objects.len();
    let dropped_relations = full.relations.len() - kept.relations.len();
    let dropped_findings = full.findings.len() - kept.findings.len();
    let dropped_groups = full.groups.len() - kept.groups.len();
    let dropped = dropped_objects + dropped_relations + dropped_findings + dropped_groups;
    let hint = format!(
        "overfull brief: budget {budget_bytes} bytes keeps {kept_bytes} of {need_bytes} bytes; \
         raise budget_bytes to at least {need_bytes} (short by {short_by} bytes) or \
         shed {dropped} rows ({dropped_objects} objects, {dropped_relations} relations, \
         {dropped_findings} findings, {dropped_groups} groups)"
    );
    Err(BriefShortfall {
        budget_bytes,
        need_bytes,
        dropped_objects,
        dropped_relations,
        dropped_findings,
        dropped_groups,
        hint,
    })
}

/// Adversarial scene family for the stress sweep: duplicate display names
/// (weak-model aliasing), hostile name bytes (injection shapes) and overfull
/// crowds (budget pressure). Fixture-only: positions come from a closed-form
/// integer scatter of the seed (scaffolding, never the engine RNG).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StressFamily {
    DuplicateNames,
    HostileNames,
    Overfull,
}

impl StressFamily {
    /// Stable lowercase family label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DuplicateNames => "duplicate_names",
            Self::HostileNames => "hostile_names",
            Self::Overfull => "overfull",
        }
    }

    /// Every stress family, frozen order.
    pub const ALL: [Self; 3] = [Self::DuplicateNames, Self::HostileNames, Self::Overfull];
}

/// Hostile display-name battery: newline injection, bracket spoofing, empty
/// input, control bytes and a 67-char overlong label. Rotated by seed so
/// every sweep covers every shape; each entry must still render as one
/// bracket-free row (see [`sanitize_name`]).
pub const HOSTILE_NAMES: [&str; 5] = [
    "crate\nnext_to 0 [9]\nobject injected [7]",
    "a]b[c",
    "",
    "with\tcontrol\rchars",
    "overlong_overlong_overlong_overlong_overlong_overlong_overlong_name",
];

/// Fixture scatter tag: keeps lane bytes out of every other digest domain.
const SCATTER_TAG: &[u8] = b"e2brief-scatter:1";

/// Closed-form fixture scatter: one of `span` consecutive values from
/// `(seed, stream, lane)` through the engine FNV owner. Deterministic in all
/// three inputs; lanes stay distinct by tag plus stream. Fixture-only
/// scaffolding, never an engine RNG.
fn scatter(seed: u64, stream: u64, lane: u64, span: u64) -> u64 {
    if span == 0 {
        return 0;
    }
    let mut state = Fnv64::new();
    state.write(SCATTER_TAG);
    state.write_u64(seed);
    state.write_u64(stream);
    state.write_u64(lane);
    state.finish() % span
}

/// Scatter step to floor coordinate: steps 0..=8 ride -4.0..=4.0 in exact
/// 1.0 lanes. Total: anything outside the scatter range parks at the origin
/// instead of refusing, so fixture builders stay infallible.
fn scatter_coord(step: u64) -> f32 {
    match step {
        0 => -4.0,
        1 => -3.0,
        2 => -2.0,
        3 => -1.0,
        4 => 0.0,
        5 => 1.0,
        6 => 2.0,
        7 => 3.0,
        8 => 4.0,
        _ => 0.0,
    }
}

/// Scatter shift to grid origin: 0.0, 1.0, then 2.0 for anything else.
/// Total like [`scatter_coord`]: fixture builders never refuse.
fn scatter_shift(shift: u64) -> f32 {
    match shift {
        0 => 0.0,
        1 => 1.0,
        _ => 2.0,
    }
}

/// Scatter pick to hostile name: picks 0..=3 index the battery in order and
/// anything else takes the overlong tail. Total like [`scatter_coord`].
fn hostile_pick(pick: u64) -> &'static str {
    match pick {
        0 => HOSTILE_NAMES[0],
        1 => HOSTILE_NAMES[1],
        2 => HOSTILE_NAMES[2],
        3 => HOSTILE_NAMES[3],
        _ => HOSTILE_NAMES[4],
    }
}

/// One room wall for the solvable stress fixtures: rest-height panel with
/// the given footprint.
fn wall_actor(id: u32, name: &str, x: f32, z: f32, sx: f32, sz: f32) -> Actor {
    Actor {
        id,
        name: name.into(),
        x,
        y: 1.0,
        z,
        sx,
        sy: 2.0,
        sz,
        parent: None,
        physical: true,
        vx: 0.0,
        vz: 0.0,
        yaw: 0.0,
    }
}

/// Four room walls (ids 0..=3) shared by the solvable stress fixtures.
fn wall_actors() -> Vec<Actor> {
    vec![
        wall_actor(0, "wall_north", 0.0, -5.0, 9.8, 0.2),
        wall_actor(1, "wall_south", 0.0, 5.0, 9.8, 0.2),
        wall_actor(2, "wall_west", -5.0, 0.0, 0.2, 10.0),
        wall_actor(3, "wall_east", 5.0, 0.0, 0.2, 10.0),
    ]
}

/// One resting floor prop: the unit the stress fixtures scatter.
fn floor_prop(id: u32, name: &str, x: f32, z: f32) -> Actor {
    Actor {
        id,
        name: name.into(),
        x,
        y: 0.5,
        z,
        sx: 1.0,
        sy: 1.0,
        sz: 0.5,
        parent: None,
        physical: true,
        vx: 0.0,
        vz: 0.0,
        yaw: 0.0,
    }
}

/// Walled room plus `2 + seed % 4` props sharing one display name: the
/// weak-model aliasing shape. Ids stay dense from 4; positions scatter over
/// the floor at rest height.
fn duplicate_actors(seed: u64) -> Vec<Actor> {
    let mut actors = wall_actors();
    let total = scatter(seed, 1, 0, 4) + 2;
    let mut id = 4u32;
    let mut lane = 0u64;
    while lane < total {
        actors.push(floor_prop(
            id,
            "crate",
            scatter_coord(scatter(seed, 11, lane, 9)),
            scatter_coord(scatter(seed, 23, lane, 9)),
        ));
        id += 1;
        lane += 1;
    }
    actors
}

/// One arena plus the hostile-name battery rotated by seed: every sweep
/// covers every injection shape at a different id.
fn hostile_actors(seed: u64) -> Vec<Actor> {
    let mut actors = Vec::with_capacity(1 + HOSTILE_NAMES.len());
    actors.push(floor_prop(0, "arena", 0.0, 0.0));
    for (slot, _name) in HOSTILE_NAMES.iter().enumerate() {
        // `slot` stays below the battery length, so both narrowings below
        // are total; the fallbacks only type the impossible case (the
        // exposure float decoder uses the same checked shape).
        let lane = u64::try_from(slot).unwrap_or(0);
        let id = u32::try_from(1 + slot).unwrap_or(0);
        let pick = scatter(seed, 7, lane, 5);
        actors.push(floor_prop(id, hostile_pick(pick), scatter_coord(lane), 2.0));
    }
    actors
}

/// Dense 12x12 same-name grid (budget pressure): 144 props sharing one name
/// over a seed-shifted origin.
fn overfull_actors(seed: u64) -> Vec<Actor> {
    let origin = scatter_shift(scatter(seed, 5, 0, 3));
    let mut actors = Vec::with_capacity(144);
    for gx in 0..12u8 {
        for gz in 0..12u8 {
            let id = u32::from(gx) * 12 + u32::from(gz);
            actors.push(floor_prop(
                id,
                "crate",
                origin + f32::from(gx) * 2.0,
                origin + f32::from(gz) * 2.0,
            ));
        }
    }
    actors
}

/// Seeded adversarial actor set for one stress family. Replays with no
/// model: the same seed and family give the same actors, byte for byte.
///
/// Consumer: stress sweeps and eval harnesses replaying weak-model input
/// shapes. The caller owns the data: plain [`Actor`]s in meters, id order
/// out. Units are actor meters in; positions scatter over the room floor.
/// Determinism scope: pure function of `(seed, family)`; hostile names stay
/// hostile (sanitizing happens at render, never here). Errors: none.
#[must_use]
pub fn stress_actors(seed: u64, family: StressFamily) -> Vec<Actor> {
    match family {
        StressFamily::DuplicateNames => duplicate_actors(seed),
        StressFamily::HostileNames => hostile_actors(seed),
        StressFamily::Overfull => overfull_actors(seed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[expect(
        clippy::too_many_arguments,
        reason = "test fixture builder takes each field positionally"
    )]
    fn mk(
        id: u32,
        name: &str,
        x: f32,
        y: f32,
        z: f32,
        sx: f32,
        sy: f32,
        sz: f32,
        yaw: f32,
    ) -> Actor {
        Actor {
            id,
            name: name.into(),
            x,
            y,
            z,
            sx,
            sy,
            sz,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw,
        }
    }

    fn prop(id: u32, name: &str, x: f32, y: f32, z: f32) -> Actor {
        mk(id, name, x, y, z, 1.0, 1.0, 1.0, 0.0)
    }

    /// Fixed arena fixture: arena + nested zone + two side-by-side crates
    /// plus a floating torch (floating + support yellows).
    fn arena_fixture() -> Vec<Actor> {
        vec![
            mk(0, "arena", 0.0, 2.0, 0.0, 20.0, 4.0, 20.0, 0.0),
            mk(1, "zone_north", 0.0, 1.5, -5.0, 6.0, 3.0, 6.0, 0.0),
            prop(2, "crate_a", -1.0, 0.5, -5.0),
            prop(3, "crate_b", 0.5, 0.5, -5.0),
            prop(4, "torch_01", 5.0, 5.0, 5.0),
        ]
    }

    fn brief_of(actors: &[Actor], budget: usize) -> (Brief, String) {
        let graph = super::super::relation::build(actors);
        let report = super::super::run(actors);
        let brief = super::brief(actors, &graph, &report, budget);
        let text = super::brief_text(&brief);
        (brief, text)
    }

    #[test]
    fn golden_brief_text_arena_fixture() {
        let actors = arena_fixture();
        let (_brief, text) = brief_of(&actors, usize::MAX);
        let expected = concat!(
            "digest 13654318318115460896\n",
            "object arena [0]\n",
            "object zone_north [1]\n",
            "object crate_a [2]\n",
            "object crate_b [3]\n",
            "object torch_01 [4]\n",
            "group arena 0\n",
            "group zone 1\n",
            "group object 2,3,4\n",
            "relation 0 facing 3\n",
            "relation 0 facing 4\n",
            "relation 1 inside 0\n",
            "relation 1 facing 3\n",
            "relation 1 facing 4\n",
            "relation 2 inside 0\n",
            "relation 2 inside 1\n",
            "relation 2 next_to 3\n",
            "relation 2 facing 0\n",
            "relation 2 facing 1\n",
            "relation 2 facing 3\n",
            "relation 2 facing 4\n",
            "relation 3 inside 0\n",
            "relation 3 inside 1\n",
            "relation 3 facing 4\n",
            "finding overlap 0 1 red 100\n",
            "finding overlap 0 2 red 100\n",
            "finding overlap 0 3 red 100\n",
            "finding overlap 0 4 red 100\n",
            "finding overlap 1 2 red 100\n",
            "finding overlap 1 3 red 100\n",
            "finding floating 4 - yellow 0\n",
            "finding support 4 - yellow 0\n",
        );
        assert_eq!(text, expected);
    }

    #[test]
    fn same_scene_twice_identical() {
        let actors = arena_fixture();
        let (_, t1) = brief_of(&actors, usize::MAX);
        let (_, t2) = brief_of(&actors, usize::MAX);
        assert_eq!(t1, t2);
        // Input order never moves the text either.
        let mut rev = actors.clone();
        rev.reverse();
        let (_, t3) = brief_of(&rev, usize::MAX);
        assert_eq!(t1, t3);
    }

    #[test]
    fn changed_scene_changes_digest() {
        let actors = arena_fixture();
        let (b1, t1) = brief_of(&actors, usize::MAX);
        let mut moved = actors.clone();
        moved[2] = prop(2, "crate_a", 5.0, 0.5, -5.0);
        let (b2, t2) = brief_of(&moved, usize::MAX);
        assert_ne!(b1.digest, b2.digest);
        assert_ne!(t1, t2);
        assert_eq!(
            b1.digest,
            super::super::relation::digest(&super::super::relation::build(&actors))
        );
    }

    #[test]
    fn size_budget_keeps_only_reds_and_digest() {
        // Large crowded arena: 12x12 grid sharing one name (dupeSpam reds).
        let mut actors = Vec::new();
        let mut id = 0u32;
        for gx in 0..12 {
            for gz in 0..12 {
                actors.push(mk(
                    id,
                    "crate",
                    f32::from(gx as u8) * 2.0,
                    0.5,
                    f32::from(gz as u8) * 2.0,
                    1.0,
                    1.0,
                    1.0,
                    0.0,
                ));
                id += 1;
            }
        }
        let budget = 256usize;
        let (brief, text) = brief_of(&actors, budget);
        assert!(
            text.len() <= budget,
            "brief is {} bytes over budget {budget}",
            text.len()
        );
        assert!(text.starts_with("digest "), "digest line is mandatory");
        assert!(brief.relations.is_empty(), "relations drop before reds");
        assert!(brief.groups.is_empty(), "groups drop first");
        assert!(brief.objects.is_empty(), "objects drop before reds");
        assert!(!brief.findings.is_empty(), "red findings survive");
        assert!(
            brief.findings.iter().all(|f| f.severity == "red"),
            "only red findings survive a tiny budget"
        );
        for line in text.lines().skip(1) {
            assert!(
                line.starts_with("finding ") && line.contains(" red "),
                "every kept row is a whole red finding, got: {line}"
            );
        }
    }

    #[test]
    fn budget_always_fits_with_whole_rows() {
        let actors = arena_fixture();
        for budget in [64usize, 200, 512, 4096, usize::MAX] {
            let (brief, text) = brief_of(&actors, budget);
            let digest_len = digest_line(brief.digest).len();
            if budget >= digest_len {
                assert!(
                    text.len() <= budget,
                    "budget {budget}: brief is {} bytes",
                    text.len()
                );
            }
            assert!(text.starts_with("digest "), "digest always emitted");
            assert!(text.ends_with('\n'), "no partial trailing row");
            // Re-render from the struct matches: rows were never split.
            assert_eq!(text, super::brief_text(&brief));
        }
        // A generous budget keeps everything the full inputs hold.
        let graph = super::super::relation::build(&actors);
        let report = super::super::run(&actors);
        let full = super::brief(&actors, &graph, &report, usize::MAX);
        assert_eq!(full.relations.len(), graph.edges.len());
        assert_eq!(full.findings.len(), report.findings.len());
        assert_eq!(full.groups.len(), 3);
        assert_eq!(full.objects.len(), actors.len());
        let mut ids: Vec<u32> = full.objects.iter().map(|o| o.id).collect();
        ids.sort_unstable();
        assert_eq!(
            ids,
            vec![0, 1, 2, 3, 4],
            "objects cover every actor id once"
        );
    }

    #[test]
    fn digest_matches_graph_digest() {
        let actors = arena_fixture();
        let graph = super::super::relation::build(&actors);
        let report = super::super::run(&actors);
        let brief = super::brief(&actors, &graph, &report, usize::MAX);
        assert_eq!(brief.digest, super::super::relation::digest(&graph));
    }

    #[test]
    fn priority_orders_relations_above_yellows_above_groups() {
        let actors = arena_fixture();
        let graph = super::super::relation::build(&actors);
        let report = super::super::run(&actors);
        assert!(report.findings.iter().any(|f| f.severity == "red"));
        assert!(report.findings.iter().any(|f| f.severity != "red"));
        // Budget for digest + reds + a few relations: groups and yellows go first.
        let red_len: usize = report
            .findings
            .iter()
            .filter(|f| f.severity == "red")
            .map(finding_line)
            .map(|l| l.len())
            .sum();
        let digest_len = digest_line(super::super::relation::digest(&graph)).len();
        let first_relation_len = relation_line(&graph.edges[0]).len();
        let budget = digest_len + red_len + first_relation_len;
        let brief = super::brief(&actors, &graph, &report, budget);
        assert!(brief.groups.is_empty(), "groups drop before yellows");
        assert!(
            brief.objects.is_empty(),
            "objects drop before relations grow"
        );
        assert!(
            brief.findings.iter().all(|f| f.severity == "red"),
            "yellows drop before relations grow"
        );
        assert!(!brief.relations.is_empty(), "relations outrank yellows");
        // Kept findings stay in report order within each severity tier.
        let kept: Vec<(&str, u32, u32)> =
            brief.findings.iter().map(|f| (f.check, f.a, f.b)).collect();
        let mut want: Vec<(&str, u32, u32)> = report
            .findings
            .iter()
            .filter(|f| f.severity == "red")
            .map(|f| (f.check, f.a, f.b))
            .collect();
        want.truncate(kept.len());
        assert_eq!(kept, want, "reds keep report order from the head");
    }

    #[test]
    fn duplicate_names_render_distinct_ids_and_solve_by_id() {
        use super::super::place::{Intent, ObjectRef, Target, Wall, Want};
        use super::super::relation::Relation;
        // Walled room with two props sharing one display name.
        let actors = vec![
            mk(0, "wall_north", 0.0, 1.0, -5.0, 9.8, 2.0, 0.2, 0.0),
            mk(1, "wall_south", 0.0, 1.0, 5.0, 9.8, 2.0, 0.2, 0.0),
            mk(2, "wall_west", -5.0, 1.0, 0.0, 0.2, 2.0, 10.0, 0.0),
            mk(3, "wall_east", 5.0, 1.0, 0.0, 0.2, 2.0, 10.0, 0.0),
            mk(4, "crate", 3.0, 0.5, -4.5, 1.0, 1.0, 0.5, 0.0),
            mk(5, "crate", 0.0, 0.5, 2.0, 0.6, 1.0, 0.6, 0.0),
        ];
        let (brief, text) = brief_of(&actors, usize::MAX);
        assert!(
            text.contains("object crate [4]\n"),
            "first duplicate renders with its id, got:\n{text}"
        );
        assert!(
            text.contains("object crate [5]\n"),
            "second duplicate renders with its id, got:\n{text}"
        );
        assert_eq!(
            brief.objects.iter().filter(|o| o.name == "crate").count(),
            2,
            "both duplicates survive in the label set"
        );
        // Placing by id still solves through the solver.
        let by_id = Intent {
            object: ObjectRef::Id(5),
            wants: vec![Want {
                relation: Relation::NextTo,
                target: Target::Wall(Wall::North),
                distance_cm: Some((0, 60)),
            }],
        };
        let outs = super::super::place::solve(&by_id, &actors)
            .expect("duplicate-name scene must solve by id");
        assert_eq!(outs.len(), 1);
        assert_eq!(outs[0].id, 5);
        // Placing by the shared name refuses instead of mis-placing.
        let by_name = Intent {
            object: ObjectRef::Name("crate".to_string()),
            wants: vec![Want {
                relation: Relation::NextTo,
                target: Target::Wall(Wall::North),
                distance_cm: Some((0, 60)),
            }],
        };
        let err = super::super::place::solve(&by_name, &actors)
            .expect_err("shared name must not resolve");
        assert!(
            err.reason.contains("ambiguous-object"),
            "duplicate name refuses loudly, got: {}",
            err.reason
        );
    }

    #[test]
    fn hostile_names_sanitize_to_single_line() {
        let actors = vec![
            prop(0, "arena", 0.0, 0.5, 0.0),
            prop(
                1,
                "crate\nnext_to 0 [9]\nobject injected [7]",
                2.0,
                0.5,
                0.0,
            ),
            prop(2, "", 4.0, 0.5, 0.0),
            prop(3, "a]b[c", 6.0, 0.5, 0.0),
        ];
        let (brief, text) = brief_of(&actors, usize::MAX);
        assert_eq!(brief.objects.len(), 4);
        for entry in &brief.objects {
            assert!(
                !entry.name.contains('\n') && !entry.name.contains('\r'),
                "sanitized name stays single-line, got {:?}",
                entry.name
            );
            assert!(
                !entry.name.contains('[') && !entry.name.contains(']'),
                "sanitized name stays bracket-free, got {:?}",
                entry.name
            );
        }
        assert!(
            text.contains("object - [2]\n"),
            "empty name degrades to `-`, got:\n{text}"
        );
        let object_rows: Vec<&str> = text
            .lines()
            .filter(|line| line.starts_with("object "))
            .collect();
        assert_eq!(
            object_rows.len(),
            4,
            "a hostile name never splits into extra rows, got:\n{text}"
        );
        for line in &object_rows {
            assert!(
                line.ends_with(']'),
                "every object row keeps the `name [id]` shape, got: {line}"
            );
        }
        assert_eq!(text, super::brief_text(&brief), "rows never split");
    }

    #[test]
    fn object_labels_ignore_input_order_and_duplicate_ids() {
        let first = vec![
            prop(2, "crate_a", 0.0, 0.5, 0.0),
            prop(1, "crate_b", 2.0, 0.5, 0.0),
        ];
        let mut second = first.clone();
        second.reverse();
        let (b1, t1) = brief_of(&first, usize::MAX);
        let (b2, t2) = brief_of(&second, usize::MAX);
        assert_eq!(b1.objects, b2.objects, "input order never moves labels");
        assert_eq!(t1, t2, "input order never moves the text");
        // Duplicate ids degrade deterministically: first in id order wins.
        let dupes = vec![
            prop(9, "second", 0.0, 0.5, 0.0),
            prop(9, "first", 2.0, 0.5, 0.0),
        ];
        let (dup_brief, _) = brief_of(&dupes, usize::MAX);
        assert_eq!(dup_brief.objects.len(), 1);
        assert_eq!(dup_brief.objects[0].id, 9);
    }

    #[test]
    fn stress_parity_table_identical_x3_over_seeds() {
        for family in super::StressFamily::ALL {
            for seed in 0..8u64 {
                let actors = super::stress_actors(seed, family);
                let graph = super::super::relation::build(&actors);
                let report = super::super::run(&actors);
                let table = super::parity_table(&actors, &graph, &report, 4096);
                assert_eq!(
                    table.rows.len(),
                    super::BriefProvider::ALL.len(),
                    "one row per provider ({} seed {seed})",
                    family.as_str()
                );
                assert!(
                    super::parity_ok(&table.rows),
                    "parity breaks on {} seed {seed}: {:?}",
                    family.as_str(),
                    table.rows
                );
                for row in &table.rows {
                    assert_eq!(
                        row.bytes,
                        table.text.len(),
                        "row length reads back the text ({} seed {seed})",
                        family.as_str()
                    );
                }
                assert!(super::parity_ok(&table.rows));
                assert!(!super::parity_ok(&[]), "an empty table never passes");
                // Replay with no model: the same call twice is byte-identical.
                let again = super::parity_table(&actors, &graph, &report, 4096);
                assert_eq!(
                    table,
                    again,
                    "replay drifts on {} seed {seed}",
                    family.as_str()
                );
            }
        }
    }

    #[test]
    fn stress_duplicates_zero_misplaces() {
        use super::super::place::{Intent, ObjectRef, Target, Wall, Want};
        use super::super::relation::Relation;
        let mut misplaced = 0usize;
        for seed in 0..12u64 {
            let actors = super::stress_actors(seed, super::StressFamily::DuplicateNames);
            assert!(
                (6..=9).contains(&actors.len()),
                "seed {seed} scatters 2..=5 crates over 4 walls"
            );
            let (brief, text) = brief_of(&actors, usize::MAX);
            let mut ids: Vec<u32> = brief.objects.iter().map(|entry| entry.id).collect();
            ids.sort_unstable();
            let row_count = ids.len();
            ids.dedup();
            assert_eq!(
                ids.len(),
                row_count,
                "object rows keep distinct ids (seed {seed})"
            );
            assert!(
                text.contains("object crate [4]\n"),
                "first duplicate renders with its id (seed {seed}), got:\n{text}"
            );
            let want = Want {
                relation: Relation::NextTo,
                target: Target::Wall(Wall::North),
                distance_cm: Some((0, 60)),
            };
            match super::super::place::solve(
                &Intent {
                    object: ObjectRef::Name("crate".to_string()),
                    wants: vec![want],
                },
                &actors,
            ) {
                Ok(_) => {
                    misplaced += 1;
                }
                Err(refusal) => assert!(
                    refusal.reason.contains("ambiguous-object"),
                    "shared name refuses loudly (seed {seed}), got: {}",
                    refusal.reason
                ),
            }
            for id in &ids {
                let intent = Intent {
                    object: ObjectRef::Id(*id),
                    wants: vec![want],
                };
                if let Ok(outs) = super::super::place::solve(&intent, &actors) {
                    assert!(!outs.is_empty(), "a solve answers (seed {seed} id {id})");
                    for out in &outs {
                        if out.id != *id {
                            misplaced += 1;
                        }
                    }
                }
            }
        }
        assert_eq!(
            misplaced, 0,
            "{misplaced} duplicate mis-places over 12 seeded scenes"
        );
    }

    #[test]
    fn stress_overfull_refuses_with_hint() {
        for seed in 0..4u64 {
            let actors = super::stress_actors(seed, super::StressFamily::Overfull);
            assert_eq!(actors.len(), 144, "overfull stays a 12x12 grid");
            let graph = super::super::relation::build(&actors);
            let report = super::super::run(&actors);
            let budget = 256usize;
            let refusal = super::brief_checked(&actors, &graph, &report, budget)
                .expect_err("144 props never fit 256 bytes");
            assert!(refusal.need_bytes > budget, "need exceeds the budget");
            assert!(
                refusal.dropped_objects
                    + refusal.dropped_relations
                    + refusal.dropped_findings
                    + refusal.dropped_groups
                    > 0,
                "the refusal names shed rows"
            );
            let need = refusal.need_bytes.to_string();
            assert!(
                refusal.hint.contains(need.as_str()),
                "hint names the minimum budget, got: {}",
                refusal.hint
            );
            assert_eq!(
                format!("{refusal}"),
                refusal.hint,
                "Display carries the hint"
            );
            // The hinted budget fits exactly.
            let kept = super::brief_checked(&actors, &graph, &report, refusal.need_bytes)
                .expect("hinted budget fits");
            assert!(super::brief_text(&kept).len() <= refusal.need_bytes);
            // The legacy path agrees on identity under the same pressure.
            let (_, legacy) = brief_of(&actors, budget);
            assert!(legacy.starts_with("digest "), "digest stays mandatory");
            let kept_of_need = format!("keeps {} of {}", legacy.len(), refusal.need_bytes);
            assert!(
                refusal.hint.contains(kept_of_need.as_str()),
                "hint reads back the exact kept bytes, got: {}",
                refusal.hint
            );
        }
    }

    #[test]
    fn stress_hostile_names_stay_single_line() {
        for seed in 0..6u64 {
            let actors = super::stress_actors(seed, super::StressFamily::HostileNames);
            let (brief, text) = brief_of(&actors, usize::MAX);
            assert_eq!(brief.objects.len(), 1 + super::HOSTILE_NAMES.len());
            for entry in &brief.objects {
                assert!(
                    !entry.name.contains('\n') && !entry.name.contains('\r'),
                    "sanitized name stays single-line (seed {seed}), got {:?}",
                    entry.name
                );
                assert!(
                    !entry.name.contains('[') && !entry.name.contains(']'),
                    "sanitized name stays bracket-free (seed {seed}), got {:?}",
                    entry.name
                );
            }
            let rows: Vec<&str> = text
                .lines()
                .filter(|line| line.starts_with("object "))
                .collect();
            assert_eq!(
                rows.len(),
                brief.objects.len(),
                "a hostile name never splits into extra rows (seed {seed}), got:\n{text}"
            );
            for row in &rows {
                assert!(
                    row.ends_with(']'),
                    "every object row keeps the `name [id]` shape, got: {row}"
                );
            }
            let (_, again) = brief_of(&actors, usize::MAX);
            assert_eq!(text, again, "hostile scenes replay identically");
        }
    }

    #[test]
    fn stress_floor_digest_first_ids_unique() {
        for family in super::StressFamily::ALL {
            for seed in 0..8u64 {
                let actors = super::stress_actors(seed, family);
                let (brief, text) = brief_of(&actors, usize::MAX);
                assert!(
                    text.starts_with("digest "),
                    "digest stays mandatory ({} seed {seed})",
                    family.as_str()
                );
                assert!(
                    text.ends_with('\n'),
                    "no partial trailing row ({} seed {seed})",
                    family.as_str()
                );
                let mut ids: Vec<u32> = brief.objects.iter().map(|entry| entry.id).collect();
                ids.sort_unstable();
                let row_count = ids.len();
                ids.dedup();
                assert_eq!(
                    ids.len(),
                    row_count,
                    "ids stay unique ({} seed {seed})",
                    family.as_str()
                );
                assert_eq!(
                    text,
                    super::brief_text(&brief),
                    "rows never split ({} seed {seed})",
                    family.as_str()
                );
            }
        }
    }

    #[test]
    fn stress_fixtures_replay_per_seed() {
        for family in super::StressFamily::ALL {
            for seed in 0..8u64 {
                let first = super::stress_actors(seed, family);
                let second = super::stress_actors(seed, family);
                assert_eq!(
                    first,
                    second,
                    "stress fixtures replay ({} seed {seed})",
                    family.as_str()
                );
            }
        }
        let mut counts = BTreeSet::new();
        for seed in 0..8u64 {
            counts.insert(super::stress_actors(seed, super::StressFamily::DuplicateNames).len());
        }
        assert!(
            counts.len() > 1,
            "seeds steer the duplicate layout, got one shape: {counts:?}"
        );
    }

    #[test]
    fn estimate_tokens_empty_is_zero() {
        assert_eq!(super::estimate_tokens(""), 0);
    }

    #[test]
    fn estimate_tokens_exact_samples() {
        // "hello world": 11 chars -> ceil(11/4) = 3, 2 words -> max = 3.
        assert_eq!(super::estimate_tokens("hello world"), 3);
        // "a b c d": 7 chars -> ceil(7/4) = 2, 4 words -> max = 4, so the
        // whitespace-heavy shape prices by words, not chars.
        assert_eq!(super::estimate_tokens("a b c d"), 4);
    }

    #[test]
    fn estimate_tokens_monotonic_over_prefixes() {
        // ASCII sample, so every byte index is a char boundary.
        let full = "digest 123\nobject crate [4]\ngroup object 4\n";
        let mut prev = 0u32;
        for len in 0..=full.len() {
            let tokens = super::estimate_tokens(&full[..len]);
            assert!(
                tokens >= prev,
                "more text never costs fewer tokens: len {len} gave {tokens} after {prev}"
            );
            prev = tokens;
        }
        assert!(prev > 0, "a full brief row prices above zero");
        assert_eq!(
            super::estimate_tokens(full),
            super::estimate_tokens(full),
            "same text replays to the same estimate"
        );
    }

    #[test]
    fn brief_stats_carry_budget_and_token_estimate() {
        assert_eq!(
            super::BriefStats::default().estimated_tokens,
            0,
            "the additive token field defaults to zero for old rows"
        );
        let actors = arena_fixture();
        let (brief, text) = brief_of(&actors, usize::MAX);
        let stats = super::brief_stats(&brief, usize::MAX);
        assert_eq!(stats.budget_bytes, usize::MAX, "budget echoes the offer");
        assert_eq!(stats.estimated_tokens, super::estimate_tokens(&text));
        assert!(stats.estimated_tokens > 0, "a full brief prices above zero");
        // Under pressure the estimate tracks the kept rows, not the scene.
        let (kept, kept_text) = brief_of(&actors, 200);
        let kept_stats = super::brief_stats(&kept, 200);
        assert_eq!(kept_stats.budget_bytes, 200);
        assert_eq!(
            kept_stats.estimated_tokens,
            super::estimate_tokens(&kept_text)
        );
        assert!(
            kept_stats.estimated_tokens <= stats.estimated_tokens,
            "a truncated brief never prices above the full one"
        );
    }

    #[test]
    fn hierarchical_brief_collapses_2000_under_4k_tokens() {
        let mut actors = Vec::with_capacity(2000);
        for id in 0..2000u32 {
            let gx = u8::try_from(id % 50).unwrap_or(0);
            let gz = u8::try_from(id / 50).unwrap_or(0);
            let x = f32::from(gx) * 2.0;
            let z = f32::from(gz) * 2.0;
            let name = format!("prop{id}");
            let mut actor = mk(id, &name, x, 0.5, z, 1.0, 1.0, 1.0, 0.0);
            if id >= 36 {
                actor.parent = Some(id % 36);
            }
            actors.push(actor);
        }
        assert_eq!(actors.len(), 2000, "fixture stays id-dense");
        let rows = super::room_rows(&actors);
        assert_eq!(rows.len(), 36, "36 forest roots stay 36 rows");
        let collapsed = super::collapsed_room_text(&actors);
        let collapsed_tokens = super::estimate_tokens(&collapsed);
        assert!(
            collapsed_tokens < 4000,
            "collapsed {collapsed_tokens} tokens must stay under 4000"
        );
        let graph = super::super::relation::build(&actors);
        let report = super::super::run(&actors);
        let flat = super::brief_text(&super::brief(&actors, &graph, &report, usize::MAX));
        let flat_tokens = super::estimate_tokens(&flat);
        assert!(
            flat_tokens > collapsed_tokens,
            "flat {flat_tokens} tokens must stay strictly above collapsed {collapsed_tokens}"
        );
    }

    #[test]
    fn hierarchical_brief_expand_reads_back_one_room() {
        let mut actors = Vec::new();
        for id in 0..12u32 {
            let gx = u8::try_from(id % 6).unwrap_or(0);
            let gz = u8::try_from(id / 6).unwrap_or(0);
            let x = f32::from(gx) * 2.0;
            let z = f32::from(gz) * 2.0;
            let name = format!("prop{id}");
            let mut actor = mk(id, &name, x, 0.5, z, 1.0, 1.0, 1.0, 0.0);
            if id >= 3 {
                actor.parent = Some(id % 3);
            }
            actors.push(actor);
        }
        let rows = super::room_rows(&actors);
        assert_eq!(rows.len(), 3, "three forest roots stay three rows");
        let first_room = rows[0].0;
        let second_room = rows[1].0;
        let first_members = rows[0].1.clone();
        let second_members = rows[1].1.clone();
        let graph = super::super::relation::build(&actors);
        let report = super::super::run(&actors);
        let expanded = super::expand_room_text(&actors, &graph, &report, first_room, usize::MAX);
        let object_lines: Vec<&str> = expanded
            .lines()
            .filter(|line| line.starts_with("object "))
            .collect();
        for id in &first_members {
            let want = format!("object prop{id} [{id}]");
            assert!(
                object_lines.iter().any(|line| *line == want),
                "room {first_room} keeps {want}, got:\n{expanded}"
            );
        }
        for id in &second_members {
            let banned = format!("[{id}]");
            assert!(
                !object_lines
                    .iter()
                    .any(|line| line.contains(banned.as_str())),
                "room {first_room} hides room {second_room} id {id}, got:\n{expanded}"
            );
        }
        let mut reversed = actors.clone();
        reversed.reverse();
        let again = super::expand_room_text(&reversed, &graph, &report, first_room, usize::MAX);
        assert_eq!(expanded, again, "reversed input replays byte-identical");
        let repeat = super::expand_room_text(&actors, &graph, &report, first_room, usize::MAX);
        assert_eq!(expanded, repeat, "repeat call replays byte-identical");
        let unknown = super::expand_room_text(&actors, &graph, &report, 9999, usize::MAX);
        assert!(
            unknown.starts_with("digest "),
            "unknown room keeps the digest shape, got:\n{unknown}"
        );
        let unknown_objects = unknown
            .lines()
            .filter(|line| line.starts_with("object "))
            .count();
        assert_eq!(unknown_objects, 0, "unknown room keeps zero object rows");
    }

    #[test]
    fn hierarchical_brief_flat_path_unchanged() {
        let actors = vec![
            mk(0, "arena", 0.0, 2.0, 0.0, 20.0, 4.0, 20.0, 0.0),
            mk(1, "zone_north", 0.0, 1.5, -5.0, 6.0, 3.0, 6.0, 0.0),
            mk(2, "crate_a", -1.0, 0.5, -5.0, 1.0, 1.0, 1.0, 0.0),
            mk(3, "crate_b", 0.5, 0.5, -5.0, 1.0, 1.0, 1.0, 0.0),
            mk(4, "torch_01", 5.0, 5.0, 5.0, 1.0, 1.0, 1.0, 0.0),
        ];
        let graph = super::super::relation::build(&actors);
        let report = super::super::run(&actors);
        let brief = super::brief(&actors, &graph, &report, usize::MAX);
        let text = super::brief_text(&brief);
        assert!(
            text.starts_with("digest 13654318318115460896\n"),
            "flat digest line never moves, got:\n{text}"
        );
    }
}

#[cfg(test)]
mod brief_snapshot {
    use super::{brief, brief_text};
    const NAME: &str = "brief_snapshot_rejects_unedited_change";
    const REFERENCE: &str = "digest 9003630263813481348\nobject prop_00 [0]\nobject prop_01 [1]\ngroup arena 1\ngroup object 0\nrelation 0 facing 1\n";
    fn redact_brief(text: &str) -> String {
        let flat = text.replace("\r\n", "\n").replace('\r', "\n");
        let mut out = String::with_capacity(flat.len());
        for line in flat.split_inclusive('\n') {
            let (body, nl) = match line.strip_suffix('\n') {
                Some(s) => (s, "\n"),
                None => (line, ""),
            };
            let mut red = body.replace("...", "[tail]").replace('…', "[tail]");
            for word in ["truncated", "Truncated", "TRUNCATED"] {
                red = red.replace(word, "[tail]");
            }
            let low = red.to_ascii_lowercase();
            let vol = ["bytes", "budget", "truncat", "short by"]
                .iter()
                .any(|k| low.contains(k));
            if vol {
                let mut masked = String::with_capacity(red.len());
                let mut chars = red.chars().peekable();
                while let Some(ch) = chars.next() {
                    if ch.is_ascii_digit() {
                        while chars.peek().is_some_and(|n| n.is_ascii_digit()) {
                            chars.next();
                        }
                        masked.push_str("[n]");
                    } else {
                        masked.push(ch);
                    }
                }
                red = masked;
            }
            out.push_str(&red);
            out.push_str(nl);
        }
        out
    }
    fn pending_path() -> std::path::PathBuf {
        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("src");
        path.push(format!("{NAME}.pending"));
        path
    }
    #[rustfmt::skip]
    fn room() -> Vec<super::super::Actor> {
        vec![
            super::super::Actor { id: 0, name: "prop_00".into(), x: 0.0, y: 0.225, z: 0.0, sx: 0.45, sy: 0.45, sz: 0.45, parent: None, physical: true, vx: 0.0, vz: 0.0, yaw: 0.0 },
            super::super::Actor { id: 1, name: "prop_01".into(), x: 6.0, y: 1.5, z: 0.0, sx: 0.5, sy: 3.0, sz: 4.0, parent: None, physical: true, vx: 0.0, vz: 0.0, yaw: 0.0 },
        ]
    }
    #[test]
    fn brief_snapshot_rejects_unedited_change() {
        let actors = room();
        let graph = super::super::relation::build(&actors);
        let report = super::super::run(&actors);
        let text = brief_text(&brief(&actors, &graph, &report, usize::MAX));
        let redacted = redact_brief(&text);
        let mut rev = actors.clone();
        rev.reverse();
        let graph2 = super::super::relation::build(&rev);
        let report2 = super::super::run(&rev);
        let text2 = brief_text(&brief(&rev, &graph2, &report2, usize::MAX));
        assert_eq!(text, text2, "input order never moves brief text");
        assert_eq!(redacted, redact_brief(&text2));
        let pending = pending_path();
        if redacted != REFERENCE {
            std::fs::write(&pending, &redacted).expect("pending writes");
            panic!("brief snapshot mismatch: wrote {} for review; blessed update copies it into REFERENCE, never auto-accepts", pending.display());
        }
        let _ = std::fs::remove_file(&pending);
    }
}
