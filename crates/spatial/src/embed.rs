//! Tiny offline embedding model for scene queries (spike, 2026-10-19).
//!
//! Consumer: an offline scene-query helper. Nothing here is called from a
//! simulation step, so no vector ever reaches the tick or a digest.
//!
//! Model: there is no weights file, no download and no API key. The "weights"
//! are the pinned [`VOCAB`] table (one slot per known word) plus a hashed
//! subword table, both derived from these source bytes by feature hashing, so a
//! checkout cannot lose a checkpoint and `git ls-files` never lists one.
//! [`model_bytes`] bounds the whole model against the row's 50 MB ceiling.
//! Killed with numbers if the model bytes or the measured query time go over
//! budget; the `embed_` tests print both.
//!
//! Determinism: lowercase ASCII words, FNV-1a feature hashing and integer
//! cosine only. No float, no clock, no `HashMap`; the same text and labels give
//! the same hits in any label input order. Units: none cross this module.
//!
//! Budget: one query is O(nodes * [`DIM`]) integer multiply-adds plus one
//! embed of the query text; the `embed_` tests time it against
//! [`QUERY_BUDGET_US`].

use crate::intmath::isqrt;
use engine_core::Fnv64;

/// Feature slots in one embedding vector. Known words take slots `0..VOCAB`
/// and every hashed feature takes `VOCAB.len()..DIM`.
pub const DIM: usize = 128;
/// Bytes in one subword feature; a shorter word is its own subword.
pub const SUBWORD: usize = 4;
/// Fixed-point scale of a normalized vector: this integer is `1.0`.
pub const VEC_SCALE: i32 = 1 << 12;
/// Weight of one word that is in [`VOCAB`].
pub const KNOWN_WEIGHT: i32 = 6;
/// Weight of one word that is not in [`VOCAB`] (subwords still match it).
pub const UNKNOWN_WEIGHT: i32 = 2;
/// Weight of one subword feature.
pub const SUBWORD_WEIGHT: i32 = 2;
/// Mean query time the spike must stay under, in microseconds.
pub const QUERY_BUDGET_US: u64 = 1000;
/// Largest model the row allows, in bytes.
pub const MODEL_MAX_BYTES: usize = 50 * 1024 * 1024;

/// The pinned scene vocabulary: lowercase ASCII words the model knows by name.
/// Keep it sorted: the byte order is what [`VOCAB_HASH`] gates, and a word here
/// takes its own slot while every other token falls back to hashed subwords.
pub const VOCAB: &[&str] = &[
    "above", "arch", "banner", "barrel", "behind", "bench", "between", "box", "cloth", "corner",
    "cover", "crate", "debris", "desk", "east", "field", "floor", "gate", "inside", "iron",
    "lantern", "middle", "monument", "north", "pillar", "room", "row", "sand", "shrine", "side",
    "south", "stands", "statue", "stone", "table", "torch", "tunnel", "under", "wall", "west",
    "wood",
];

/// FNV-1a over the canonical [`VOCAB`] bytes (domain tag, then each word with a
/// little-endian `u64` length), pinned so an edit to the table fails the gate
/// until a reviewer re-pins it.
pub const VOCAB_HASH: u64 = 0x5185_acc6_7898_57c0;

/// Domain tag of the vocab gate; it separates this digest from every other
/// FNV use in the crate.
const VOCAB_TAG: &[u8] = b"e2embedvocab1";
/// Domain tag of a hashed word feature.
const WORD_TAG: &[u8] = b"w";
/// Domain tag of a hashed subword feature.
const GRAM_TAG: &[u8] = b"c";
/// Hash bit that picks a hashed feature's sign.
const SIGN_BIT: u32 = 32;
/// Lowest hash bit used to pick a hashed slot (the high bits mix best).
const SLOT_SHIFT: u32 = 57;

/// Canonical digest of [`VOCAB`]; compare it with [`VOCAB_HASH`].
pub fn vocab_hash() -> u64 {
    let mut hash = Fnv64::new();
    hash.write(VOCAB_TAG);
    for word in VOCAB {
        hash.write_len(word.len());
        hash.write(word.as_bytes());
    }
    hash.finish()
}

/// Bytes the whole model occupies: the vocab text plus one `i32` per slot. The
/// subword table is hashed on the fly, so it costs no bytes.
pub fn model_bytes() -> usize {
    let words: usize = VOCAB.iter().map(|word| word.len() + 1).sum();
    words + DIM * core::mem::size_of::<i32>()
}

/// ASCII words of `text`, lowercased, split on every non-alphanumeric byte.
fn words(text: &str) -> Vec<String> {
    text.to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Every `SUBWORD`-wide byte slice of an ASCII word, or the whole word when it
/// is shorter. Tokens are ASCII by construction, so byte slices are safe.
fn subwords(word: &str) -> Vec<&[u8]> {
    let bytes = word.as_bytes();
    if bytes.len() < SUBWORD {
        return vec![bytes];
    }
    bytes.windows(SUBWORD).collect()
}

/// Hashing modulus for a feature slot: the room [`VOCAB`] leaves.
fn hashed_span() -> u64 {
    u64::try_from(DIM.saturating_sub(VOCAB.len()).max(1)).unwrap_or(1)
}

/// Slot and sign of one hashed feature, derived from `tag` and `bytes` alone.
fn hashed_feature(tag: &[u8], bytes: &[u8]) -> (usize, i32) {
    let mut hash = Fnv64::new();
    hash.write(tag);
    hash.write(bytes);
    let digest = hash.finish();
    let offset = usize::try_from((digest >> SLOT_SHIFT) % hashed_span()).unwrap_or(0);
    let sign = if (digest >> SIGN_BIT) & 1 == 0 { 1 } else { -1 };
    (VOCAB.len() + offset, sign)
}

/// Add `sign * weight` to one slot; a slot past [`DIM`] is dropped.
fn add_feature(vector: &mut [i32], slot: usize, sign: i32, weight: i32) {
    if let Some(cell) = vector.get_mut(slot) {
        *cell += sign * weight;
    }
}

/// Scale a vector to [`VEC_SCALE`] length in fixed point; a zero vector stays
/// zero. Rounds to nearest so a repeated text keeps its bytes.
fn normalize(vector: &mut [i32]) {
    let sumsq: i64 = vector
        .iter()
        .map(|value| i64::from(*value) * i64::from(*value))
        .sum();
    let norm = isqrt(sumsq);
    if norm == 0 {
        return;
    }
    let scale = i64::from(VEC_SCALE);
    for value in vector.iter_mut() {
        let scaled = (i64::from(*value) * scale + norm / 2) / norm;
        *value = i32::try_from(scaled).unwrap_or(0);
    }
}

/// Embed one text into a normalized integer vector of [`DIM`] slots. A word in
/// [`VOCAB`] takes its own slot; any other word and every subword are feature
/// hashed, which is what lets `crates` reach `crate_a` and `bannerz` reach
/// `banner_west` where an exact word match cannot.
pub fn embed(text: &str) -> Vec<i32> {
    let mut vector = vec![0i32; DIM];
    for word in words(text) {
        match VOCAB.iter().position(|known| *known == word.as_str()) {
            Some(slot) => add_feature(&mut vector, slot, 1, KNOWN_WEIGHT),
            None => {
                let (slot, sign) = hashed_feature(WORD_TAG, word.as_bytes());
                add_feature(&mut vector, slot, sign, UNKNOWN_WEIGHT);
            }
        }
        for gram in subwords(&word) {
            let (slot, sign) = hashed_feature(GRAM_TAG, gram);
            add_feature(&mut vector, slot, sign, SUBWORD_WEIGHT);
        }
    }
    normalize(&mut vector);
    vector
}

/// Cosine of two normalized vectors in permille (`1000` is identical);
/// vectors are compared slot by slot up to the shorter one.
pub fn cosine_permille(a: &[i32], b: &[i32]) -> i32 {
    let dot: i64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| i64::from(*x) * i64::from(*y))
        .sum();
    let denom = i64::from(VEC_SCALE) * i64::from(VEC_SCALE);
    i32::try_from(dot * 1000 / denom).unwrap_or(0)
}

/// One query answer: a node id and the cosine against its label in permille.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    /// Node the label belongs to.
    pub id: u32,
    /// Cosine with the query vector in permille; higher is closer.
    pub score_permille: i32,
}

/// Text index over scene node labels: one integer vector per node.
#[derive(Debug, Clone, Default)]
pub struct SceneIndex {
    rows: Vec<(u32, Vec<i32>)>,
}

impl SceneIndex {
    /// Index every `(node id, label)` pair, embedding each label once.
    pub fn build(labels: &[(u32, &str)]) -> Self {
        Self {
            rows: labels
                .iter()
                .map(|(id, label)| (*id, embed(label)))
                .collect(),
        }
    }

    /// Nodes the index holds.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// True when no node is indexed.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The `k` best nodes for `text`: score descending, then id ascending, so
    /// the answer never depends on the order the labels arrived in.
    pub fn query(&self, text: &str, k: usize) -> Vec<Hit> {
        let query = embed(text);
        let mut hits: Vec<Hit> = self
            .rows
            .iter()
            .map(|(id, vector)| Hit {
                id: *id,
                score_permille: cosine_permille(&query, vector),
            })
            .collect();
        hits.sort_by(|a, b| {
            b.score_permille
                .cmp(&a.score_permille)
                .then(a.id.cmp(&b.id))
        });
        hits.truncate(k);
        hits
    }
}

/// Lexical baseline for the spike: nodes ordered by the number of distinct
/// exact words they share with `text`, id ascending on ties. Nodes with no
/// shared word are dropped. This is the number the embedding has to beat on
/// real asset names; it is a comparison arm, never an answer path.
pub fn lexical_rank(labels: &[(u32, &str)], text: &str) -> Vec<u32> {
    let want = words(text);
    let mut rows: Vec<(usize, u32)> = Vec::new();
    for (id, label) in labels {
        let mut label_words = words(label);
        label_words.sort_unstable();
        label_words.dedup();
        let shared = label_words
            .iter()
            .filter(|word| want.contains(word))
            .count();
        if shared > 0 {
            rows.push((shared, *id));
        }
    }
    rows.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    rows.into_iter().map(|(_, id)| id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real kit node labels, read from the arena placement the engine ships.
    /// The embedding quality number below is measured on these names, not on
    /// names invented for the test.
    fn kit_nodes() -> Vec<(u32, String)> {
        let json = include_str!("../../../content/arena/placement.json");
        let mut nodes = Vec::new();
        let mut id: Option<u32> = None;
        for line in json.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("\"id\":") {
                id = rest.trim().trim_end_matches(',').parse().ok();
            } else if let Some(rest) = trimmed.strip_prefix("\"name\": \"") {
                if let (Some(id), Some(name)) = (id, rest.split('"').next()) {
                    nodes.push((id, name.to_owned()));
                }
            }
        }
        nodes
    }

    /// Plain-English queries over the kit, each with the node a human would
    /// name. The variants (plurals, a typo) are the ones exact words miss.
    const QUERIES: [(&str, &str); 14] = [
        ("a torch on the west side", "torch_west"),
        ("torches in the east", "torch_east"),
        ("the north wall", "wall_north"),
        ("walls to the south", "wall_south"),
        ("the gate tunnel", "gate_tunnel"),
        ("a stack of crates", "crate_a"),
        ("the gates", "gate_tunnel"),
        ("bannerz in the west", "banner_west"),
        ("banners in the east", "banner_east"),
        ("the stands", "stands"),
        ("sand floor", "sand_floor"),
        ("a field of debris", "debris_field"),
        ("the floorr", "sand_floor"),
        ("debris scattered about", "debris_field"),
    ];

    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "the spike prints measured wall time in-test only; no clock reaches the model, the tick or any hash"
    )]
    #[test]
    fn embed_answers_real_kit_queries_offline_inside_the_ms_budget() {
        use std::time::Instant;
        let owned = kit_nodes();
        let labels: Vec<(u32, &str)> = owned
            .iter()
            .map(|(id, name)| (*id, name.as_str()))
            .collect();
        assert!(!labels.is_empty(), "no kit nodes read from the placement");
        let index = SceneIndex::build(&labels);
        assert_eq!(index.len(), labels.len());

        let rounds = 200u128;
        let mut embed_hit = 0u32;
        let mut lexical_hit = 0u32;
        let mut total_us = 0u128;
        for (text, gold) in QUERIES {
            let gold_id = labels
                .iter()
                .find(|(_, name)| *name == gold)
                .map(|(id, _)| *id)
                .unwrap_or_else(|| panic!("the kit lost the node {gold}"));
            let hits = index.query(text, 3);
            assert_eq!(hits.len(), 3usize.min(labels.len()));
            if hits[0].id == gold_id {
                embed_hit += 1;
            }
            if lexical_rank(&labels, text).first() == Some(&gold_id) {
                lexical_hit += 1;
            }
            let started = Instant::now();
            for _ in 0..rounds {
                std::hint::black_box(index.query(text, 3));
            }
            total_us += started.elapsed().as_micros();
            println!(
                "embed query: {text:?} -> {} score {} (want {gold_id})",
                hits[0].id, hits[0].score_permille
            );
        }
        let count = u32::try_from(QUERIES.len()).unwrap_or(1);
        let embed_permille = embed_hit * 1000 / count;
        let lexical_permille = lexical_hit * 1000 / count;
        let per_query_us = total_us / (u128::from(count) * rounds);
        println!(
            "embed answers: {embed_hit}/{count} top-1 ({embed_permille} permille) on real kit names, \
             exact-word baseline {lexical_permille} permille, {per_query_us} us/query, \
             model {} bytes",
            model_bytes()
        );
        assert_eq!(embed_permille, 1000, "every gold query is answered top-1");
        assert!(
            lexical_permille < embed_permille,
            "the embedding has to add recall over exact words"
        );
        assert!(
            per_query_us < u128::from(QUERY_BUDGET_US),
            "{per_query_us} us/query is over the {} us budget",
            QUERY_BUDGET_US
        );
    }

    #[test]
    fn embed_vocab_hash_gate_holds() {
        assert_eq!(
            vocab_hash(),
            VOCAB_HASH,
            "the vocab table changed: review it and re-pin VOCAB_HASH"
        );
        assert!(
            VOCAB.len() < DIM,
            "the vocab has to leave room for hashed slots"
        );
        for pair in VOCAB.windows(2) {
            assert!(
                pair[0] < pair[1],
                "the vocab table is kept sorted: {pair:?}"
            );
        }
        for word in VOCAB {
            assert!(!word.is_empty(), "an empty vocab word is a wildcard");
            assert!(
                word.bytes().all(|b| b.is_ascii_lowercase()),
                "{word} is not lowercase ASCII"
            );
        }
        for word in ["torch", "wall", "crate", "debris", "floor", "gate"] {
            assert!(VOCAB.contains(&word), "the kit's own word is wired: {word}");
        }
        println!(
            "embed vocab: {} words, gate 0x{VOCAB_HASH:016x}",
            VOCAB.len()
        );
    }

    #[test]
    fn embed_model_is_code_only_and_under_the_size_ceiling() {
        let bytes = model_bytes();
        assert!(bytes > 0, "an empty model is not a model");
        assert!(
            bytes < MODEL_MAX_BYTES,
            "{bytes} bytes is over the {MODEL_MAX_BYTES} byte ceiling"
        );
        // No blob is read at run time: the model is code, so no weights file
        // can appear in `git ls-files`.
        let needle = ["include", "_bytes!"].concat();
        let source = include_str!("embed.rs");
        assert!(
            !source.contains(&needle),
            "the model must not load a weights blob"
        );
        println!("embed model: {bytes} bytes in code, ceiling {MODEL_MAX_BYTES} bytes");
    }

    #[test]
    fn embed_query_is_order_invariant_and_repeats() {
        let labels = [
            (3u32, "wall_north"),
            (1u32, "sand_floor"),
            (5u32, "gate_tunnel"),
        ];
        let index = SceneIndex::build(&labels);
        let mut flipped = labels;
        flipped.reverse();
        let twin = SceneIndex::build(&flipped);
        let first = index.query("north wall", 3);
        assert_eq!(first, twin.query("north wall", 3));
        assert_eq!(first[0].id, 3, "closest label wins");
        assert_eq!(embed("north wall"), embed("north wall"));

        let vector = embed("gate tunnel");
        assert!(
            cosine_permille(&vector, &vector) >= 990,
            "a vector is close to itself"
        );
        assert!(vector.iter().any(|slot| *slot != 0));

        let empty = SceneIndex::build(&[]);
        assert!(empty.is_empty());
        assert!(empty.query("anything", 3).is_empty());

        let blank = index.query("", 3);
        assert_eq!(blank.len(), 3, "an empty query ranks all nodes");
        assert_eq!(blank[0].id, 1, "ties fall back to ascending id");
        assert!(index.query("north wall", 0).is_empty());
        assert!(embed("").iter().all(|slot| *slot == 0));
    }
}
