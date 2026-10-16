//! BM25 search over the Insight index, plain Rust.
//!
//! Owner: API1. Weights mirror forge `FlaxApiFts.cs`: name 10, path 5,
//! docs 1. Determinism: ties break on path text, never hash order.
//! Errors: [`InsightError`] for blank queries and bad limits.

use super::index::{Index, InsightError};

/// BM25 saturation and length-norm constants.
pub const BM25_K1: f64 = 1.2;
/// BM25 length-norm constant.
pub const BM25_B: f64 = 0.75;
/// Name field weight (forge `FlaxApiFts.cs` parity).
pub const WEIGHT_NAME: f64 = 10.0;
/// Path field weight.
pub const WEIGHT_PATH: f64 = 5.0;
/// Docs field weight.
pub const WEIGHT_DOCS: f64 = 1.0;
/// Default hit count for `api.search`.
pub const SEARCH_DEFAULT_LIMIT: u32 = 8;
/// Hard hit cap for `api.search`.
pub const SEARCH_MAX_LIMIT: u32 = 32;

/// One ranked hit: path, kind label, score and a one-line brief.
#[derive(Debug, Clone)]
pub struct SearchHit {
    /// Canonical item path.
    pub path: String,
    /// Kind label (`function`, `tool`, ...).
    pub kind: &'static str,
    /// BM25 score (higher is better; ordering is what matters).
    pub score: f64,
    /// One-line brief for tool lists.
    pub brief: String,
}

/// Lowercase alphanumeric runs plus their camel-case humps (`ObjectId`
/// gives `objectid`, `object` and `id`). Single chars are dropped as noise.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut run = String::new();
    let mut flush = |run: &mut String| {
        if run.chars().count() < 2 {
            run.clear();
            return;
        }
        let full = run.to_lowercase();
        tokens.push(full);
        // Humps after a lower-to-upper step (`Object|Id`).
        let mut hump = String::new();
        let mut chars = run.chars().peekable();
        while let Some(ch) = chars.next() {
            hump.push(ch);
            let hump_ends = match chars.peek() {
                Some(next) => next.is_uppercase() && ch.is_lowercase(),
                None => true,
            };
            if hump_ends {
                let lower = hump.to_lowercase();
                if lower.chars().count() >= 2 && Some(&lower) != tokens.last() {
                    tokens.push(lower);
                }
                hump.clear();
            }
        }
        run.clear();
    };
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            run.push(ch);
        } else if !run.is_empty() {
            flush(&mut run);
        }
    }
    if !run.is_empty() {
        flush(&mut run);
    }
    tokens.retain(|token| token.chars().count() >= 2);
    tokens
}

/// Count term frequencies, sorted by term for stable iteration.
pub(crate) fn count_terms(tokens: &[String]) -> Vec<(String, u32)> {
    let mut sorted: Vec<&str> = tokens.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    let mut out: Vec<(String, u32)> = Vec::new();
    for term in sorted {
        if let Some(last) = out.last_mut() {
            if last.0 == term {
                last.1 += 1;
                continue;
            }
        }
        out.push((term.to_owned(), 1));
    }
    out
}

/// Total token count in a frequency table.
pub(crate) fn total(table: &[(String, u32)]) -> u64 {
    table.iter().map(|(_, count)| u64::from(*count)).sum()
}

fn term_in(table: &[(String, u32)], term: &str) -> u32 {
    match table.binary_search_by(|(name, _)| name.as_str().cmp(term)) {
        Ok(at) => table[at].1,
        Err(_) => 0,
    }
}

/// Rank the index for `query`, returning at most `limit` hits.
/// Empty queries and limits outside 1..=32 are typed errors.
pub fn search(index: &Index, query: &str, limit: u32) -> Result<Vec<SearchHit>, InsightError> {
    if limit == 0 || limit > SEARCH_MAX_LIMIT {
        return Err(InsightError::BadLimit { got: limit });
    }
    let query_tf = count_terms(&tokenize(query));
    if query_tf.is_empty() {
        return Err(InsightError::EmptyQuery);
    }
    let doc_count = index.len() as f64;
    let avg = index.avg_len().max(1.0);
    let mut scored: Vec<(f64, &super::index::ApiItem)> = Vec::new();
    for item in index.items() {
        let mut score = 0.0;
        for (term, _) in &query_tf {
            let df = index.doc_freq(term) as f64;
            let idf = ((doc_count - df + 0.5) / (df + 0.5) + 1.0).ln();
            let tf = WEIGHT_NAME * f64::from(term_in(&item.name_tf, term))
                + WEIGHT_PATH * f64::from(term_in(&item.path_tf, term))
                + WEIGHT_DOCS * f64::from(term_in(&item.docs_tf, term));
            if tf <= 0.0 {
                continue;
            }
            let len = item.weighted_len as f64;
            let norm = tf * (BM25_K1 + 1.0) / (tf + BM25_K1 * (1.0 - BM25_B + BM25_B * len / avg));
            score += idf * norm;
        }
        if score > 0.0 {
            scored.push((score, item));
        }
    }
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.path.cmp(&b.1.path))
    });
    Ok(scored
        .into_iter()
        .take(limit as usize)
        .map(|(score, item)| SearchHit {
            path: item.path.clone(),
            kind: item.kind.as_str(),
            score,
            brief: item.brief(120),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight::eval::seed_corpus;

    #[test]
    fn tokenizes_camel_snake_and_dots() {
        let tokens = tokenize("ObjectId::from_u64 tx.preview");
        assert!(tokens.contains(&"objectid".to_owned()));
        assert!(tokens.contains(&"object".to_owned()));
        assert!(tokens.contains(&"preview".to_owned()));
        assert!(tokens.contains(&"tx".to_owned()));
        assert!(!tokens.iter().any(|token| token.len() < 2));
    }

    #[test]
    fn exact_name_beats_path_and_docs() {
        let index = seed_corpus();
        let hits = search(&index, "ObjectId", 3).expect("name searches");
        assert_eq!(hits[0].path, "engine-scene::ids::ObjectId");
    }

    #[test]
    fn docs_keyword_finds_its_item() {
        let index = seed_corpus();
        let hits = search(&index, "windlass", 3).expect("docs search");
        assert!(!hits.is_empty());
        assert!(hits[0].path.contains("Approval"));
    }

    #[test]
    fn blank_query_and_bad_limit_are_typed() {
        let index = seed_corpus();
        assert_eq!(
            search(&index, "   ", 8).expect_err("blank must fail"),
            InsightError::EmptyQuery
        );
        assert_eq!(
            search(&index, "scene", 0).expect_err("zero must fail"),
            InsightError::BadLimit { got: 0 }
        );
        assert_eq!(
            search(&index, "scene", 33).expect_err("over must fail"),
            InsightError::BadLimit { got: 33 }
        );
    }

    #[test]
    fn ranking_is_path_stable_on_ties() {
        let index = seed_corpus();
        let first = search(&index, "scene", 8).expect("scene search");
        let second = search(&index, "scene", 8).expect("scene rerun");
        let paths: Vec<&str> = first.iter().map(|hit| hit.path.as_str()).collect();
        let rerun: Vec<&str> = second.iter().map(|hit| hit.path.as_str()).collect();
        assert_eq!(paths, rerun, "same query twice gives same order");
    }
}
