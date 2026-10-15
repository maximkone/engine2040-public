// big23 golden: fused BLAS single-pass outputs locked byte-for-byte.
// Any change to digest, AABB, byte layout or error kind fails here.
use engine_render::accel::{
    build_input_bytes, win1_frame_instances, win1_ring_slots, AccelError, Blas, ProxyMesh, Tlas,
};

fn quad() -> ([[f32; 3]; 4], [u32; 6]) {
    (
        [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
        [0, 1, 2, 0, 2, 3],
    )
}

fn tetra() -> ([[f32; 3]; 4], [u32; 12]) {
    (
        [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ],
        [0, 1, 2, 0, 1, 3, 0, 2, 3, 1, 2, 3],
    )
}

#[test]
fn quad_blas_digest_aabb_bytes_locked() {
    let (pos, idx) = quad();
    let b = Blas::build(
        7,
        &ProxyMesh {
            positions: &pos,
            indices: &idx,
        },
    )
    .unwrap();
    assert_eq!(b.digest(), 2772257768545957698);
    assert_eq!(b.aabb(), ([0.0, 0.0, 0.0], [1.0, 1.0, 0.0]));
    assert_eq!(
        (b.tri_count(), b.degenerate_tris(), b.vert_count()),
        (2, 0, 4)
    );
    let bytes = build_input_bytes(&b);
    assert_eq!(bytes.len(), 92);
    assert_eq!(
        &bytes[..16],
        &[7, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 6, 0, 0, 0]
    );
    // Same proxy twice: same bytes, same digest, same order.
    let b2 = Blas::build(
        7,
        &ProxyMesh {
            positions: &pos,
            indices: &idx,
        },
    )
    .unwrap();
    assert_eq!(build_input_bytes(&b2), bytes);
    assert_eq!(b2.digest(), b.digest());
}

#[test]
fn tetra_blas_digest_aabb_locked() {
    let (pos, idx) = tetra();
    let b = Blas::build(
        3,
        &ProxyMesh {
            positions: &pos,
            indices: &idx,
        },
    )
    .unwrap();
    assert_eq!(b.digest(), 12985591553188110815);
    assert_eq!(b.aabb(), ([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]));
    assert_eq!(build_input_bytes(&b).len(), 116);
}

#[test]
fn digest_order_locked_id_matters() {
    let (pos, idx) = quad();
    let a = Blas::build(
        7,
        &ProxyMesh {
            positions: &pos,
            indices: &idx,
        },
    )
    .unwrap();
    let b = Blas::build(
        8,
        &ProxyMesh {
            positions: &pos,
            indices: &idx,
        },
    )
    .unwrap();
    assert_ne!(a.digest(), b.digest());
}

#[test]
fn partly_degenerate_count_locked() {
    let pos = [
        [0.0f32, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 0.0, 0.0],
    ];
    let idx = [0u32, 1, 2, 0, 0, 0];
    let b = Blas::build(
        7,
        &ProxyMesh {
            positions: &pos,
            indices: &idx,
        },
    )
    .unwrap();
    assert_eq!(b.degenerate_tris(), 1);
    assert_eq!(b.tri_count(), 2);
}

#[test]
fn refit_digest_aabb_locked() {
    let (pos, idx) = quad();
    let mut b = Blas::build(
        7,
        &ProxyMesh {
            positions: &pos,
            indices: &idx,
        },
    )
    .unwrap();
    let moved = [
        [0.0f32, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [2.0, 2.0, 0.0],
        [0.0, 2.0, 0.0],
    ];
    let rc = b.refit_positions(&moved).unwrap();
    assert_eq!((rc.refits, rc.verts, rc.tris), (1, 4, 2));
    assert_eq!(b.digest(), 13664110420262747298);
    assert_eq!(b.aabb(), ([0.0, 0.0, 0.0], [2.0, 2.0, 0.0]));
    assert_eq!(b.refits(), 1);
}

#[test]
fn win1_tlas_digest_sorted_locked() {
    let inst = win1_frame_instances([1, 2, 3, 4, 5, 6, 7, 8], win1_ring_slots());
    let t = Tlas::build(&inst).unwrap();
    assert_eq!(t.digest(), 10454273896396192510);
    assert_eq!((t.generation(), t.instances().len()), (0, 9));
    // Submit order never decides bytes: reversed input digests identically.
    let mut rev = inst.to_vec();
    rev.reverse();
    assert_eq!(Tlas::build(&rev).unwrap().digest(), t.digest());
}

#[test]
fn error_kinds_locked() {
    // Empty proxy: Invalid.
    let empty: [[f32; 3]; 0] = [];
    let idx = [0u32, 1, 2];
    assert!(matches!(
        Blas::build(
            0,
            &ProxyMesh {
                positions: &empty,
                indices: &idx
            }
        ),
        Err(AccelError::Invalid(_))
    ));
    // Non-triangle-multiple indices: Invalid.
    let (pos, _) = quad();
    let bad = [0u32, 1];
    assert!(matches!(
        Blas::build(
            0,
            &ProxyMesh {
                positions: &pos,
                indices: &bad
            }
        ),
        Err(AccelError::Invalid(_))
    ));
    // All zero-area: Degenerate.
    let flat = [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
    let fidx = [0u32, 1, 2];
    assert!(matches!(
        Blas::build(
            0,
            &ProxyMesh {
                positions: &flat,
                indices: &fidx
            }
        ),
        Err(AccelError::Degenerate(_))
    ));
    // Topology change on refit: NeedsRebuild.
    let (pos, idx) = quad();
    let mut b = Blas::build(
        0,
        &ProxyMesh {
            positions: &pos,
            indices: &idx,
        },
    )
    .unwrap();
    let short = [[0.0f32, 0.0, 0.0]];
    assert!(matches!(
        b.refit_positions(&short),
        Err(AccelError::NeedsRebuild { built: 4, got: 1 })
    ));
    // Base tier off the ray path: loud fallback.
    assert!(matches!(
        engine_render::accel::require_rt(engine_render::tier::Tier::Base),
        Err(AccelError::TierFallback { .. })
    ));
    // Zero-mask TLAS instance: Invalid.
    let mut inst = win1_frame_instances([1, 2, 3, 4, 5, 6, 7, 8], win1_ring_slots());
    inst[0].mask = 0;
    assert!(matches!(Tlas::build(&inst), Err(AccelError::Invalid(_))));
}
