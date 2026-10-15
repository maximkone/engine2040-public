//! Callers graph and asset references over indexed file bodies.
//!
//! Owner: API1. Units: counts only. Determinism: outputs are path-sorted;
//! matching is substring with identifier boundaries, never hash order.
//! Errors: [`InsightError::UnknownPath`] for paths the index lacks.

use super::index::{Index, InsightError};

/// True when `needle` occurs in `hay` with identifier boundaries
/// (no alphanumeric or `_` touching either side). Names under 4 chars
/// never match: they drown the graph in noise (`new`, `id`, `run`).
pub fn mentions(hay: &str, needle: &str) -> bool {
    if needle.len() < 4 {
        return false;
    }
    let bytes = hay.as_bytes();
    let find = needle.as_bytes();
    let mut at = 0;
    while at + find.len() <= bytes.len() {
        if &bytes[at..at + find.len()] == find {
            let left = at == 0 || !is_ident(bytes[at - 1]);
            let end = at + find.len();
            let right = end >= bytes.len() || !is_ident(bytes[end]);
            if left && right {
                return true;
            }
        }
        at += 1;
    }
    false
}

fn is_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Items whose file mentions `path`'s item name: the `code.callers`
/// answer. The item itself is excluded; results are path-sorted.
/// Short names (<4 chars) yield an empty graph rather than noise.
pub fn callers_of(index: &Index, path: &str) -> Result<Vec<String>, InsightError> {
    let target = index.get(path).ok_or_else(|| InsightError::UnknownPath {
        path: path.to_owned(),
    })?;
    let name = target.name.clone();
    if name.len() < 4 {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for item in index.items() {
        if item.path == path {
            continue;
        }
        if let Some(body) = index.body_of(&item.file) {
            if mentions(body, &name) {
                out.push(item.path.clone());
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// Items whose file mentions an asset's file stem: the `asset.refs`
/// answer. Unknown assets are typed errors; unreferenced assets give an
/// empty list, never a guess.
pub fn asset_refs(index: &Index, asset: &str) -> Result<Vec<String>, InsightError> {
    if index.get(asset).is_none() {
        return Err(InsightError::UnknownPath {
            path: asset.to_owned(),
        });
    }
    let stem = asset
        .rsplit('/')
        .next()
        .unwrap_or(asset)
        .split('.')
        .next()
        .unwrap_or(asset);
    if stem.len() < 4 {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for item in index.items() {
        if item.path == asset {
            continue;
        }
        if let Some(body) = index.body_of(&item.file) {
            if mentions(body, stem) {
                out.push(item.path.clone());
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight::eval::seed_corpus;

    #[test]
    fn word_boundaries_reject_substrings_and_short_names() {
        assert!(mentions("use crate::scene::Brief;", "Brief"));
        assert!(!mentions("BriefingNotes", "Brief"));
        assert!(!mentions("use x;", "run"));
        assert!(!mentions("anything at all", "id"));
    }

    #[test]
    fn callers_finds_mentioning_items() {
        let index = seed_corpus();
        let callers = callers_of(&index, "engine-scene::ids::ObjectId").expect("callers");
        assert!(
            callers.contains(&"engine-scene::snapshot::snapshot_ids".to_owned()),
            "snapshot mentions ObjectId, got {callers:?}"
        );
        assert!(!callers.contains(&"engine-scene::ids::ObjectId".to_owned()));
    }

    #[test]
    fn unknown_paths_are_typed() {
        let index = seed_corpus();
        assert_eq!(
            callers_of(&index, "missing::Thing").expect_err("must fail"),
            InsightError::UnknownPath {
                path: "missing::Thing".to_owned()
            }
        );
        assert_eq!(
            asset_refs(&index, "content/missing.anim").expect_err("must fail"),
            InsightError::UnknownPath {
                path: "content/missing.anim".to_owned()
            }
        );
    }

    #[test]
    fn asset_refs_finds_loading_code() {
        let index = seed_corpus();
        let refs = asset_refs(&index, "content/locomotion/walk.anim").expect("refs");
        assert!(
            refs.contains(&"engine-assets::loader::load_walk".to_owned()),
            "loader mentions walk, got {refs:?}"
        );
    }
}
