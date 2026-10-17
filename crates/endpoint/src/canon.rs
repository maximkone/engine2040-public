//! canon: variant dedup for generated briefs.
//! Owner `engine-endpoint`; consumer the review pipeline (`dedup`).
//! Units chars and seeds in, canonical keys out. Deterministic scope:
//! same raw text gives the same key on every machine (FNV-1a, BTree
//! order, integer SplitMix64). Errors are typed [`CanonError`].
//! Budget: one set of at most `MAX_VARIANTS` dedups in well under 60 s;
//! per-variant work is linear in its chars with no steady allocation
//! beyond the output key.

use std::collections::{BTreeMap, BTreeSet};

use engine_core::{hash::GOLDEN_GAMMA, hash_bytes, splitmix64};

/// Longest raw variant in chars; the shared brief bound (single owner:
/// [`super::brief::BRIEF_MAX_CHARS`]).
pub use super::brief::BRIEF_MAX_CHARS as MAX_CHARS;
/// Most variants one `dedup` call accepts; keeps memory bounded.
pub const MAX_VARIANTS: usize = 1024;
/// Rewrite rules; the engine applies them to saturation then extracts.
pub const RULES: &[&str] = &[
    "trim",
    "drop-empty",
    "drop-comment",
    "lowercase-kind",
    "lowercase-relation",
    "space-to-underscore",
    "lowercase-wall",
    "normalize-point",
    "normalize-range",
    "sort-props",
    "sort-wants",
    "dedup-props",
    "dedup-wants",
    "default-spawn",
    "collapse-whitespace",
    "lowercase-encounter",
];
/// Rule count; the experiment caps this at 30 with no dependency.
pub const RULE_COUNT: usize = RULES.len();
/// Largest rule budget the experiment allows.
pub const RULE_CAP: usize = 30;
/// Reduction bar: 300 permille is 30 percent fewer variants.
pub const REDUCTION_MIN_PERMILLE: u16 = 300;
/// Seeded sets behind the contract.
pub const SEEDED_SETS: u64 = 30;
/// Wall-clock budget for the 30 sets, in seconds.
pub const TIME_BUDGET_SECS: u64 = 60;

/// What stopped canonicalization. Messages name the legal values.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CanonError {
    /// No usable text after trimming.
    Empty,
    /// Raw text longer than `MAX_CHARS` chars.
    TooLong { len: usize },
    /// More than `MAX_VARIANTS` variants in one call.
    TooMany { count: usize },
}

impl CanonError {
    /// Machine code for this failure.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Empty => "empty_variant",
            Self::TooLong { .. } => "variant_too_long",
            Self::TooMany { .. } => "too_many_variants",
        }
    }
}

impl core::fmt::Display for CanonError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => write!(f, "empty_variant: submit 1..={MAX_CHARS} chars"),
            Self::TooLong { len } => write!(
                f,
                "variant_too_long: variant holds {len} chars; at most {MAX_CHARS}"
            ),
            Self::TooMany { count } => write!(
                f,
                "too_many_variants: call holds {count} variants; at most {MAX_VARIANTS}"
            ),
        }
    }
}

impl std::error::Error for CanonError {}

/// Catalog kinds the level grammar accepts; never merged across kinds.
const KINDS: &[&str] = &[
    "altar", "banner", "barrel", "bench", "brazier", "crate", "pillar", "post", "stand", "torch",
];

fn is_kind(word: &str) -> bool {
    KINDS.contains(&word)
}

/// Actor-name bound in chars (mirrors the spatial grammar check).
pub(crate) const NAME_MAX: usize = 64;

/// Spatial-grammar names: `[a-z0-9_]`, 1..=`NAME_MAX` chars, no leading digit.
pub(crate) fn valid_name(name: &str) -> bool {
    if name.is_empty() || name.len() > NAME_MAX {
        return false;
    }
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() || c == '_' => {}
        _ => return false,
    }
    name.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Normalize one relation word; unknown words pass through untouched so
/// distinct relations never merge by accident.
fn norm_relation(word: &str) -> String {
    let lower = word.to_ascii_lowercase();
    match lower.as_str() {
        "next_to" | "next-to" => "next_to".to_owned(),
        "next" => "next_to".to_owned(),
        "facing" => "facing".to_owned(),
        "inside" => "inside".to_owned(),
        "on_top_of" | "on-top-of" => "on_top_of".to_owned(),
        "supports" => "supports".to_owned(),
        _ => lower,
    }
}

/// Join split "next to" / "on top of" pairs before per-word mapping.
fn join_spaced_relations(line: &str) -> String {
    let mut out = line.to_owned();
    // Longest first so "on top of" wins before any shorter piece.
    for (from, to) in [("on top of", "on_top_of"), ("next to", "next_to")] {
        out = out.replace(from, to);
    }
    out
}

fn norm_wall(word: &str) -> String {
    word.to_ascii_lowercase()
}

/// Strip spaces around ':' ',' '[' ']' in point and range targets.
fn norm_punct(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    for c in word.chars() {
        match c {
            ' ' | '\t' => {}
            _ => out.push(c),
        }
    }
    out
}

fn looks_like_level(text: &str) -> bool {
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let head = t.split_whitespace().next().unwrap_or("");
        let low = head.to_ascii_lowercase();
        if low == "prop" || low == "want" || low == "spawn" {
            return true;
        }
    }
    false
}

/// Parsed prop-set behind the rewrite saturation.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
struct PropSet {
    kinds: BTreeMap<String, String>,
    wants: BTreeMap<String, BTreeSet<String>>,
    spawn: Option<String>,
}

/// Parse one level-like variant into a `PropSet`. Unknown directives are
/// kept as opaque sorted lines so they never silently merge.
fn parse_level(text: &str) -> Result<(PropSet, Vec<String>), CanonError> {
    let mut set = PropSet::default();
    let mut opaque: Vec<String> = Vec::new();
    let mut seen_any = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let head = parts.next().unwrap_or("").to_ascii_lowercase();
        match head.as_str() {
            "prop" => {
                let kind_raw = parts.next().unwrap_or("");
                let name_raw = parts.next().unwrap_or("");
                if parts.next().is_some() || kind_raw.is_empty() || name_raw.is_empty() {
                    opaque.push(line.to_ascii_lowercase());
                    continue;
                }
                let kind = kind_raw.to_ascii_lowercase();
                let name = name_raw.to_ascii_lowercase();
                if !is_kind(&kind) || !valid_name(&name) {
                    opaque.push(line.to_ascii_lowercase());
                    continue;
                }
                // First writer wins; dedup-props rule keeps one entry.
                set.kinds.entry(name).or_insert(kind);
                seen_any = true;
            }
            "want" => {
                let name_raw = parts.next().unwrap_or("");
                let rel_raw = parts.next().unwrap_or("");
                let tgt_raw = parts.next().unwrap_or("");
                let rest: Vec<String> = parts.map(|s| s.to_owned()).collect();
                if name_raw.is_empty() || rel_raw.is_empty() || tgt_raw.is_empty() {
                    opaque.push(line.to_ascii_lowercase());
                    continue;
                }
                // Rejoin then map spaced relations ("next to" across words).
                let tail = format!("{rel_raw} {tgt_raw} {}", rest.join(" "));
                let joined = join_spaced_relations(&tail.to_ascii_lowercase());
                let mut words = joined.split_whitespace();
                let rel_word = words.next().unwrap_or("");
                let tgt_word = words.next().unwrap_or("");
                let extra: Vec<String> = words.map(|s| s.to_owned()).collect();
                let rel = norm_relation(rel_word);
                let tgt = if tgt_word.starts_with("wall:") {
                    let wall = tgt_word.trim_start_matches("wall:");
                    format!("wall:{}", norm_wall(wall))
                } else if tgt_word.starts_with("object:") {
                    format!("object:{}", tgt_word.trim_start_matches("object:"))
                } else if tgt_word.starts_with("point:") {
                    format!(
                        "point:{}",
                        norm_punct(tgt_word.trim_start_matches("point:"))
                    )
                } else {
                    norm_punct(tgt_word)
                };
                let mut key = format!("{rel} {tgt}");
                for e in extra {
                    if e.starts_with('[') {
                        key.push(' ');
                        key.push_str(&norm_punct(&e));
                    } else if e.is_empty() {
                        continue;
                    } else {
                        key.push(' ');
                        key.push_str(&norm_punct(&e));
                    }
                }
                let name = name_raw.to_ascii_lowercase();
                set.wants.entry(name).or_default().insert(key);
                seen_any = true;
            }
            "spawn" => {
                let name_raw = parts.next().unwrap_or("");
                if name_raw.is_empty() || parts.next().is_some() {
                    opaque.push(line.to_ascii_lowercase());
                    continue;
                }
                let name = name_raw.to_ascii_lowercase();
                if set.spawn.is_none() {
                    set.spawn = Some(name);
                }
                seen_any = true;
            }
            _ => {
                opaque.push(line.to_ascii_lowercase());
            }
        }
    }
    if !seen_any && opaque.is_empty() {
        return Err(CanonError::Empty);
    }
    Ok((set, opaque))
}

/// Saturation + extraction over the parsed set: sorts by construction
/// (BTree), dedups by construction (Set/entry), defaults the spawn to
/// the first prop. Repeated until the key stops changing.
pub fn canon_key(raw: &str) -> Result<String, CanonError> {
    let len = raw.chars().count();
    if len > MAX_CHARS {
        return Err(CanonError::TooLong { len });
    }
    if raw.split_whitespace().next().is_none() {
        return Err(CanonError::Empty);
    }
    if !looks_like_level(raw) {
        return canon_encounter(raw);
    }
    let (mut set, mut opaque) = parse_level(raw)?;
    // Saturation to fixpoint: BTree order plus the spawn default settle in
    // one pass; the bounded loop keeps termination if rules grow.
    for _ in 0..RULE_CAP {
        opaque.sort();
        opaque.dedup();
        let before = set.spawn.clone();
        if set.spawn.is_none() {
            if let Some(first) = set.kinds.keys().next().cloned() {
                set.spawn = Some(first);
            }
        }
        if set.spawn == before {
            break;
        }
    }
    let mut out = String::new();
    for (name, kind) in &set.kinds {
        out.push_str(&format!("prop {kind} {name}\n"));
    }
    for (name, wants) in &set.wants {
        for w in wants {
            out.push_str(&format!("want {name} {w}\n"));
        }
    }
    if let Some(spawn) = &set.spawn {
        out.push_str(&format!("spawn {spawn}\n"));
    }
    for line in &opaque {
        out.push_str(line);
        out.push('\n');
    }
    if out.is_empty() {
        return Err(CanonError::Empty);
    }
    Ok(out)
}

/// Encounter path: lowercase, collapse whitespace runs, keep word order.
/// Goal words (`duel`, `rivalry`, `cauldron`) are never mapped to each
/// other, so distinct keypoints never merge.
fn canon_encounter(raw: &str) -> Result<String, CanonError> {
    let mut words: Vec<String> = Vec::new();
    for token in raw.split_whitespace() {
        let clean: String = token
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect::<String>()
            .to_ascii_lowercase();
        if clean.is_empty() {
            continue;
        }
        words.push(clean);
    }
    if words.is_empty() {
        return Err(CanonError::Empty);
    }
    Ok(words.join(" "))
}

/// FNV-1a over the canonical key; same raw gives the same hash.
pub fn canon_hash(raw: &str) -> Result<u64, CanonError> {
    Ok(hash_bytes(canon_key(raw)?.as_bytes()))
}

/// Sort-only ablation: trims and sorts lines with no synonym, case or
/// spawn rules. Discriminates the full engine from naive sorting.
pub fn canon_key_sort_only(raw: &str) -> Result<String, CanonError> {
    let len = raw.chars().count();
    if len > MAX_CHARS {
        return Err(CanonError::TooLong { len });
    }
    let mut lines: Vec<String> = Vec::new();
    for line in raw.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        lines.push(t.to_owned());
    }
    if lines.is_empty() {
        return Err(CanonError::Empty);
    }
    lines.sort();
    lines.dedup();
    Ok(lines.join("\n") + "\n")
}

/// Dedup outcome: baseline distinct raws vs candidate distinct keys.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DedupReport {
    /// Distinct raw strings.
    pub baseline: usize,
    /// Distinct canonical keys.
    pub candidate: usize,
    /// Reduction in permille: `(baseline - candidate) * 1000 / baseline`.
    pub reduction_permille: u16,
    /// True when `reduction_permille >= REDUCTION_MIN_PERMILLE`.
    pub meets_bar: bool,
}

fn report_for(baseline: usize, candidate: usize) -> DedupReport {
    let reduction_permille = if baseline == 0 {
        0
    } else {
        (((baseline.saturating_sub(candidate)) as u64 * 1000) / (baseline as u64)).min(1000) as u16
    };
    DedupReport {
        baseline,
        candidate,
        reduction_permille,
        meets_bar: reduction_permille >= REDUCTION_MIN_PERMILLE,
    }
}

/// Review-pipeline entry: dedup one variant list via the full engine.
pub fn dedup(variants: &[String]) -> Result<DedupReport, CanonError> {
    if variants.len() > MAX_VARIANTS {
        return Err(CanonError::TooMany {
            count: variants.len(),
        });
    }
    let mut raws: BTreeSet<String> = BTreeSet::new();
    let mut keys: BTreeSet<String> = BTreeSet::new();
    for raw in variants {
        raws.insert(raw.clone());
        keys.insert(canon_key(raw)?);
    }
    Ok(report_for(raws.len(), keys.len()))
}

/// Review-pipeline dedup bind: collapse near-duplicate brief variants
/// through the frozen `dedup` engine before review, so one canonical key
/// is reviewed once. Single owner for `build_level`, `build_encounter`
/// and `build_character`; offline authoring only, never in the tick.
pub fn review_variants(variants: &[String]) -> Result<DedupReport, CanonError> {
    dedup(variants)
}

/// Collapse variants for review: keep the first raw text per canonical key
/// in first-seen order, so reviewers see one representative per key. Single
/// owner for the three build stages. Errors reuse the frozen [`CanonError`]
/// codes (`empty_variant`, `variant_too_long`, `too_many_variants`).
pub fn collapse_for_review(variants: &[String]) -> Result<Vec<String>, CanonError> {
    if variants.len() > MAX_VARIANTS {
        return Err(CanonError::TooMany {
            count: variants.len(),
        });
    }
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for raw in variants {
        let key = canon_key(raw)?;
        if seen.insert(key) {
            out.push(raw.clone());
        }
    }
    Ok(out)
}

/// Ablation entry: same input through sort-only canonicalization.
pub fn dedup_sort_only(variants: &[String]) -> Result<DedupReport, CanonError> {
    if variants.len() > MAX_VARIANTS {
        return Err(CanonError::TooMany {
            count: variants.len(),
        });
    }
    let mut raws: BTreeSet<String> = BTreeSet::new();
    let mut keys: BTreeSet<String> = BTreeSet::new();
    for raw in variants {
        raws.insert(raw.clone());
        keys.insert(canon_key_sort_only(raw)?);
    }
    Ok(report_for(raws.len(), keys.len()))
}

/// One seeded set: 8 superficial variants of one prop scene. Permuted
/// line order, casing, whitespace, synonyms, comments and duplicate
/// lines all name one canonical key by construction.
pub fn seeded_set(seed: u64) -> Vec<String> {
    let mut rng = seed.wrapping_add(0xab6u64);
    let draw_a = splitmix64(rng);
    rng = rng.wrapping_add(GOLDEN_GAMMA);
    let a = (draw_a % 3) as usize;
    let (k1, k2) = match a {
        0 => ("crate", "barrel"),
        1 => ("pillar", "torch"),
        _ => ("bench", "brazier"),
    };
    let draw_n1 = splitmix64(rng);
    rng = rng.wrapping_add(GOLDEN_GAMMA);
    let n1 = format!("prop_{:02}", draw_n1 % 90 + 10);
    let draw_n2 = splitmix64(rng);
    let n2 = format!("prop_{:02}", draw_n2 % 90 + 11);
    let (n1, n2) = if n1 == n2 {
        (n1, "prop_99".to_owned())
    } else {
        (n1, n2)
    };
    let base = format!("prop {k1} {n1}\nprop {k2} {n2}\nwant {n1} next_to wall:north\nspawn {n1}");
    let upper = base.to_ascii_uppercase();
    let spaced = base.replace('\n', "  \n  ").replace(' ', "   ");
    let reordered =
        format!("spawn {n1}\nwant {n1} next_to wall:north\nprop {k2} {n2}\nprop {k1} {n1}");
    let synonym =
        format!("prop {k1} {n1}\nprop {k2} {n2}\nwant {n1} next to wall:North\nspawn {n1}");
    let commented = format!("# seed {seed}\n{base}\n# end");
    let duplicated = format!("{base}\n{base}");
    let padded = format!("\n\n  {base}  \n\n");
    vec![
        base, upper, spaced, reordered, synonym, commented, duplicated, padded,
    ]
}

/// Thirty seeded sets flattened for the contract harness.
pub fn thirty_sets() -> Vec<String> {
    let mut out = Vec::with_capacity(240);
    for s in 0..SEEDED_SETS {
        out.extend(seeded_set(2040 + s));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn rules_stay_bounded_without_dependencies() {
        let count = RULES.len();
        assert!(count <= RULE_CAP, "rules {count} over cap {RULE_CAP}");
        assert_eq!(RULE_COUNT, count);
        assert!(!RULES.is_empty());
    }

    #[test]
    fn canonicalization_is_deterministic() {
        let raw = "prop crate alpha\nwant alpha next_to wall:north\nspawn alpha";
        let first = canon_key(raw).expect("fixture lands");
        let again = canon_key(raw).expect("replay lands");
        assert_eq!(first, again);
        assert_eq!(
            canon_hash(raw).expect("hash"),
            canon_hash(raw).expect("rehash")
        );
    }

    #[test]
    fn superficial_variants_share_one_key() {
        let set = seeded_set(2040);
        assert_eq!(set.len(), 8);
        let mut keys = BTreeSet::new();
        for raw in &set {
            keys.insert(canon_key(raw).expect("seeded variant lands"));
        }
        assert_eq!(keys.len(), 1, "one scene is one key: {keys:?}");
    }

    #[test]
    fn thirty_seeded_sets_shrink_by_thirty_percent() {
        let start = Instant::now();
        let all = thirty_sets();
        assert_eq!(all.len(), 240);
        let full = dedup(&all).expect("dedup lands");
        assert!(
            full.meets_bar,
            "need >=30% reduction, got {}/{} ({} permille)",
            full.candidate, full.baseline, full.reduction_permille
        );
        assert!(
            full.reduction_permille >= REDUCTION_MIN_PERMILLE,
            "reduction {} below bar {}",
            full.reduction_permille,
            REDUCTION_MIN_PERMILLE
        );
        assert!(
            start.elapsed().as_secs() < TIME_BUDGET_SECS,
            "over the 60 s budget"
        );
    }

    #[test]
    fn ablation_sort_only_misses_synonym_merges() {
        // Discriminating ablation: full engine collapses each seeded set to
        // one key while sort-only keeps casing/synonym splits apart.
        let set = seeded_set(77);
        let full_keys: BTreeSet<String> = set
            .iter()
            .map(|r| canon_key(r).expect("full lands"))
            .collect();
        let sort_keys: BTreeSet<String> = set
            .iter()
            .map(|r| canon_key_sort_only(r).expect("sort lands"))
            .collect();
        assert_eq!(full_keys.len(), 1);
        assert!(
            sort_keys.len() > full_keys.len(),
            "ablation must keep splits: sort {sort_keys:?}"
        );
        let all = thirty_sets();
        let full = dedup(&all).expect("full lands");
        let ablated = dedup_sort_only(&all).expect("ablation lands");
        assert!(
            full.candidate < ablated.candidate,
            "full {} must beat sort-only {}",
            full.candidate,
            ablated.candidate
        );
    }

    #[test]
    fn no_false_merges_on_keypoint_distinct_pairs() {
        // VK1 shape: distinct gameplay meanings never share a key. Each pair
        // below differs in exactly one semantic lane (kind, relation, goal,
        // target) and must stay split.
        let pairs: &[(&str, &str)] = &[
            ("prop crate a\nspawn a", "prop barrel a\nspawn a"),
            (
                "prop crate a\nwant a next_to wall:north\nspawn a",
                "prop crate a\nwant a facing wall:north\nspawn a",
            ),
            (
                "prop crate a\nwant a next_to wall:north\nspawn a",
                "prop crate a\nwant a next_to wall:south\nspawn a",
            ),
            ("a quiet duel at dawn", "a brutal cauldron final"),
            ("a rivalry grudge match", "a quiet duel at dawn"),
            ("prop crate a\nspawn a", "prop crate b\nspawn b"),
            (
                "prop crate a\nprop barrel b\nspawn a",
                "prop crate a\nspawn a",
            ),
            (
                "prop crate a\nwant a next_to wall:north\nspawn a",
                "prop crate a\nwant a inside wall:north\nspawn a",
            ),
        ];
        for (left, right) in pairs {
            let lk = canon_key(left).expect("left lands");
            let rk = canon_key(right).expect("right lands");
            assert_ne!(lk, rk, "false merge: {left:?} vs {right:?}");
        }
        // Thirty distinct keypoint-like scenes stay thirty keys.
        let mut distinct: Vec<String> = Vec::new();
        for i in 0..30u64 {
            distinct.push(format!("prop crate key_{i:02}\nspawn key_{i:02}"));
        }
        let report = dedup(&distinct).expect("distinct dedup lands");
        assert_eq!(report.candidate, 30, "distinct scenes never merge");
        assert_eq!(report.reduction_permille, 0);
    }

    #[test]
    fn bad_inputs_refuse_with_codes() {
        assert_eq!(canon_key("   ").expect_err("blank").code(), "empty_variant");
        let long = "x".repeat(MAX_CHARS + 1);
        assert_eq!(
            canon_key(&long).expect_err("overlong").code(),
            "variant_too_long"
        );
        let many = vec!["prop crate a\nspawn a".to_owned(); MAX_VARIANTS + 1];
        assert_eq!(
            dedup(&many).expect_err("too many").code(),
            "too_many_variants"
        );
    }
}
