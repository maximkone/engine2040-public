//! vfetch vectors: vertex-cache miss-ratio + order parity vectors.
//! Owner: assets cook (offline index order). Units: misses per index ref.
//! Deterministic: LRU over index order only, no clock, no hash order.
//! Errors: none (pure oracles); live cross-checks run under `import`.
//! Attribution: idea-only after zeux/meshoptimizer (MIT); fresh write here,
//! no vendored code; behaviour cross-checked vs mesh_cook.rs:607,665,913.

/// Cache size the vectors target, in entries (mirrors the cook default).
const VECTORS_CACHE_SIZE: usize = 32;

/// Count LRU misses of one index buffer (fresh walk, no shared code).
fn lru_misses(indices: &[u32], cache_size: usize) -> u64 {
    let mut cache: Vec<u32> = Vec::new();
    let mut misses: u64 = 0;
    for index in indices {
        let mut hit: Option<usize> = None;
        for (slot, cached) in cache.iter().enumerate() {
            if cached == index {
                hit = Some(slot);
                break;
            }
        }
        match hit {
            Some(slot) => {
                cache.remove(slot);
                cache.push(*index);
            }
            None => {
                misses += 1;
                if cache.len() >= cache_size && cache_size > 0 {
                    cache.remove(0);
                }
                if cache_size > 0 {
                    cache.push(*index);
                }
            }
        }
    }
    misses
}

/// Miss ratio `0.0..=1.0` in misses per index reference.
fn miss_ratio(indices: &[u32], cache_size: usize) -> f32 {
    if indices.is_empty() {
        return 0.0;
    }
    if cache_size == 0 {
        return 1.0;
    }
    let misses = lru_misses(indices, cache_size);
    let total_len = u16::try_from(indices.len()).expect("vector fits u16");
    let miss_len = u16::try_from(misses).expect("misses fit u16");
    f32::from(miss_len) / f32::from(total_len)
}

/// Deterministic Tipsify-school greedy order (idea-only, fresh walk):
/// most-recent cached vertex first, lowest triangle id on ties, then
/// lowest live id. Index loops only, no hash iteration, no clock.
fn greedy_order(corners: &[[u32; 3]], cache_size: usize) -> Vec<u32> {
    let size = cache_size.max(1);
    let mut live = vec![true; corners.len()];
    let mut cache: Vec<u32> = Vec::new();
    let mut stamp: Vec<u64> = Vec::new();
    let mut order: Vec<u32> = Vec::with_capacity(corners.len());
    let touch = |vertex: u32, tick: u64, cache: &mut Vec<u32>, stamp: &mut Vec<u64>| {
        let id = usize::try_from(vertex).expect("vertex fits usize");
        if stamp.len() <= id {
            stamp.resize(id + 1, 0);
        }
        stamp[id] = tick;
        let mut hit: Option<usize> = None;
        for (slot, cached) in cache.iter().enumerate() {
            if *cached == vertex {
                hit = Some(slot);
                break;
            }
        }
        match hit {
            Some(slot) => {
                cache.remove(slot);
                cache.push(vertex);
            }
            None => {
                if cache.len() >= size {
                    let mut victim = 0usize;
                    let mut oldest = u64::MAX;
                    for (slot, cached) in cache.iter().enumerate() {
                        let cid = usize::try_from(*cached).expect("vertex fits usize");
                        let age = stamp.get(cid).copied().unwrap_or(0);
                        if age < oldest {
                            oldest = age;
                            victim = slot;
                        }
                    }
                    cache.remove(victim);
                }
                cache.push(vertex);
            }
        }
    };
    let mut clock: u64 = 0;
    let mut remaining = corners.len();
    while remaining > 0 {
        let mut members = cache.clone();
        members.sort_unstable();
        members.sort_by(|a, b| {
            let aid = usize::try_from(*a).expect("vertex fits usize");
            let bid = usize::try_from(*b).expect("vertex fits usize");
            let old = stamp.get(aid).copied().unwrap_or(0);
            let young = stamp.get(bid).copied().unwrap_or(0);
            young.cmp(&old).then_with(|| a.cmp(b))
        });
        let mut picked: Option<usize> = None;
        for member in &members {
            let mut best: Option<usize> = None;
            for (id, tri) in corners.iter().enumerate() {
                if !live[id] {
                    continue;
                }
                if tri.contains(member) {
                    best = Some(id);
                    break;
                }
            }
            if best.is_some() {
                picked = best;
                break;
            }
        }
        if picked.is_none() {
            for (id, alive) in live.iter().enumerate() {
                if *alive {
                    picked = Some(id);
                    break;
                }
            }
        }
        let Some(id) = picked else {
            break;
        };
        live[id] = false;
        remaining -= 1;
        let tri_id = u32::try_from(id).expect("triangle fits u32");
        order.push(tri_id);
        for corner in corners[id] {
            clock += 1;
            touch(corner, clock, &mut cache, &mut stamp);
        }
    }
    order
}

/// Permute triangle corners into emit order.
fn permute_corners(corners: &[[u32; 3]], order: &[u32]) -> Vec<[u32; 3]> {
    let mut out = Vec::with_capacity(corners.len());
    for tri in order {
        let id = usize::try_from(*tri).expect("triangle fits usize");
        out.push(corners[id]);
    }
    out
}

/// Cooked payload bytes for a fully-referenced mesh: attributes + indices.
fn payload_bytes(vertex_count: usize, index_count: usize) -> usize {
    vertex_count
        .saturating_mul(12 + 12 + 8 + 16)
        .saturating_add(index_count.saturating_mul(4))
}

/// Area of one triangle in meters squared (double for stable sums).
fn tri_area(triangle: [[f32; 3]; 3]) -> f64 {
    let ax = f64::from(triangle[1][0] - triangle[0][0]);
    let ay = f64::from(triangle[1][1] - triangle[0][1]);
    let az = f64::from(triangle[1][2] - triangle[0][2]);
    let bx = f64::from(triangle[2][0] - triangle[0][0]);
    let by = f64::from(triangle[2][1] - triangle[0][1]);
    let bz = f64::from(triangle[2][2] - triangle[0][2]);
    let cx = ay * bz - az * by;
    let cy = az * bx - ax * bz;
    let cz = ax * by - ay * bx;
    (cx * cx + cy * cy + cz * cz).sqrt() * 0.5
}

/// Surface area over corners resolved through positions.
fn surface_area(positions: &[[f32; 3]], corners: &[[u32; 3]]) -> f64 {
    let mut sum = 0.0f64;
    for tri in corners {
        let mut pts = [[0.0f32; 3]; 3];
        for (slot, corner) in tri.iter().enumerate() {
            let id = usize::try_from(*corner).expect("vertex fits usize");
            pts[slot] = positions[id];
        }
        sum += tri_area(pts);
    }
    sum
}

/// Sorted triangle sets match (same surface combinatorics, any order).
fn sorted_tris(corners: &[[u32; 3]]) -> Vec<[u32; 3]> {
    let mut out: Vec<[u32; 3]> = corners
        .iter()
        .map(|tri| {
            let mut sorted = *tri;
            sorted.sort_unstable();
            sorted
        })
        .collect();
    out.sort_unstable();
    out
}

#[test]
fn cold_single_triangle_scores_one() {
    assert_eq!(miss_ratio(&[0, 1, 2], VECTORS_CACHE_SIZE), 1.0);
    assert_eq!(lru_misses(&[0, 1, 2], VECTORS_CACHE_SIZE), 3);
}

#[test]
fn repeated_triangle_halves_miss_ratio() {
    let cold = miss_ratio(&[0, 1, 2], VECTORS_CACHE_SIZE);
    let repeat = miss_ratio(&[0, 1, 2, 0, 1, 2], VECTORS_CACHE_SIZE);
    assert_eq!(cold, 1.0);
    assert_eq!(repeat, 0.5);
    assert!(repeat < cold, "repeat must reuse the cache");
}

#[test]
fn empty_and_zero_cache_edges() {
    assert_eq!(miss_ratio(&[], VECTORS_CACHE_SIZE), 0.0);
    assert_eq!(miss_ratio(&[0, 1, 2], 0), 1.0);
    assert_eq!(miss_ratio(&[7, 7, 7, 7], VECTORS_CACHE_SIZE), 0.25);
}

#[test]
fn greedy_order_is_deterministic_permutation() {
    let corners = vec![[0, 1, 2], [2, 1, 3], [2, 3, 4], [0, 2, 4], [5, 6, 7]];
    let first = greedy_order(&corners, VECTORS_CACHE_SIZE);
    let second = greedy_order(&corners, VECTORS_CACHE_SIZE);
    assert_eq!(first, second, "same mesh orders the same way twice");
    let mut ranked = first.clone();
    ranked.sort_unstable();
    let expect: Vec<u32> = (0..5).collect();
    assert_eq!(ranked, expect, "order must permute every triangle");
}

#[test]
fn reorder_keeps_bytes_and_surface() {
    let positions = vec![
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.5, 1.5, 0.0],
    ];
    let corners = vec![[0, 1, 2], [0, 2, 3], [3, 2, 4]];
    let flat: Vec<u32> = corners.iter().flatten().copied().collect();
    let before_ratio = miss_ratio(&flat, VECTORS_CACHE_SIZE);
    let order = greedy_order(&corners, VECTORS_CACHE_SIZE);
    let moved = permute_corners(&corners, &order);
    let moved_flat: Vec<u32> = moved.iter().flatten().copied().collect();
    assert_eq!(
        payload_bytes(positions.len(), flat.len()),
        payload_bytes(positions.len(), moved_flat.len())
    );
    assert_eq!(sorted_tris(&corners), sorted_tris(&moved));
    assert_eq!(
        surface_area(&positions, &corners),
        surface_area(&positions, &moved)
    );
    assert!(
        miss_ratio(&moved_flat, VECTORS_CACHE_SIZE) <= before_ratio,
        "greedy order must not regress misses"
    );
}

/// Live cross-checks against the real cook (run under `--features import`).
#[cfg(feature = "import")]
mod live {
    use engine_assets::mesh_cook::{
        lod_bytes, optimize_vertex_cache, vertex_cache_miss_ratio, CookedMesh, VERTEX_CACHE_SIZE,
    };

    #[test]
    fn cache_size_and_vectors_match_cook() {
        assert_eq!(super::VECTORS_CACHE_SIZE, VERTEX_CACHE_SIZE);
        assert_eq!(vertex_cache_miss_ratio(&[0, 1, 2], VERTEX_CACHE_SIZE), 1.0);
        assert_eq!(
            vertex_cache_miss_ratio(&[0, 1, 2, 0, 1, 2], VERTEX_CACHE_SIZE),
            0.5
        );
        assert_eq!(super::miss_ratio(&[0, 1, 2], 32), 1.0);
        assert_eq!(super::miss_ratio(&[0, 1, 2, 0, 1, 2], 32), 0.5);
    }

    fn quad_mesh() -> CookedMesh {
        CookedMesh {
            positions: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            uv0: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            tangents: vec![[1.0, 0.0, 0.0, 1.0]; 4],
            indices: vec![0, 1, 2, 0, 2, 3],
        }
    }

    #[test]
    fn optimize_is_deterministic_and_keeps_bytes() {
        let mesh = quad_mesh();
        let first = optimize_vertex_cache(&mesh);
        let second = optimize_vertex_cache(&mesh);
        assert_eq!(
            first.indices, second.indices,
            "cook orders deterministically"
        );
        assert_eq!(lod_bytes(&mesh), lod_bytes(&first), "bytes stay equal");
        assert!(
            vertex_cache_miss_ratio(&first.indices, VERTEX_CACHE_SIZE)
                <= vertex_cache_miss_ratio(&mesh.indices, VERTEX_CACHE_SIZE),
            "cook never regresses the miss ratio"
        );
    }
}
