//! terrain_bake: bake SHA x2, layout/splat/holes, navConnect.
use engine_world::{
    terrain::{
        bake_hash, bake_terrain, blocked, parse_terrain, terrain_hash, SLOPE_DEFAULT, TVERSION,
    },
    world_hash,
};
fn baked(seed: u64, size: u16) -> Vec<u8> {
    bake_terrain(seed, size).expect("size within MAX_BAKE_SIZE")
}
#[test]
fn bake_sha_x2() {
    let (s, n) = (0x12345678u64, 32u16);
    let (a, b) = (baked(s, n), baked(s, n));
    assert_eq!(a, b);
    assert_eq!(world_hash(&a), world_hash(&b));
    assert_eq!(Ok(terrain_hash(&a)), bake_hash(s, n));
    assert_ne!(baked(s, n), baked(s ^ 1, n));
    assert_ne!(baked(s, n), baked(s, 16));
}
#[test]
fn layout_splat_holes() {
    let (s, n) = (7u64, 16u16);
    let b = baked(s, n);
    let nn = n as usize;
    assert_eq!([b[0], b[1], b[2], b[3]], *b"W204");
    assert_eq!(b[4], TVERSION);
    assert_eq!(u64::from_le_bytes(b[5..13].try_into().unwrap()), s);
    assert_eq!(u16::from_le_bytes(b[13..15].try_into().unwrap()), n);
    assert_eq!(b.len(), 15 + nn * nn + (nn * nn).div_ceil(8) + nn * nn * 3);
    let t = parse_terrain(&b).expect("parse");
    assert_eq!((t.seed, t.size), (s, n));
    for w in &t.splat {
        assert_eq!(w[0] as u16 + w[1] as u16 + w[2] as u16, 255);
    }
    for x in 0..nn {
        assert!(!t.holes[x] && !t.holes[(nn - 1) * nn + x]);
        assert!(!t.holes[x * nn] && !t.holes[x * nn + nn - 1]);
    }
    assert!(parse_terrain(&b[..b.len() - 1]).is_none());
    let mut bad = b.clone();
    bad[4] ^= 0xFF;
    assert!(parse_terrain(&bad).is_none());
}
/// 4-dir flood fill from corner to corner. With no-corner-cutting, 4-dir
/// reachability equals 8-dir reachability: a diagonal step's two orthogonal
/// neighbors are free, so it expands to two 4-dir steps. engine-nav owns paths.
fn connected(bl: &[bool], n: usize) -> bool {
    if bl[0] || bl[n * n - 1] {
        return false;
    }
    let mut seen = vec![false; n * n];
    let mut stack = vec![0usize];
    seen[0] = true;
    while let Some(c) = stack.pop() {
        if c == n * n - 1 {
            return true;
        }
        let (cx, cy) = (c % n, c / n);
        for (dx, dy) in [(1i32, 0), (-1, 0), (0, 1), (0, -1)] {
            let (nx, ny) = (cx as i32 + dx, cy as i32 + dy);
            if nx < 0 || ny < 0 || nx >= n as i32 || ny >= n as i32 {
                continue;
            }
            let ni = ny as usize * n + nx as usize;
            if !bl[ni] && !seen[ni] {
                seen[ni] = true;
                stack.push(ni);
            }
        }
    }
    seen[n * n - 1]
}
#[test]
fn nav_connect() {
    for (s, n) in [(1u64, 16u16), (2, 24), (3, 32)] {
        let t = parse_terrain(&baked(s, n)).unwrap();
        let nn = n as usize;
        let bl = blocked(&t, SLOPE_DEFAULT);
        assert!(!bl[0] && !bl[nn * nn - 1]);
        assert!(connected(&bl, nn), "navConnect {s} {n}");
        assert_eq!(connected(&bl, nn), connected(&bl, nn));
    }
}
