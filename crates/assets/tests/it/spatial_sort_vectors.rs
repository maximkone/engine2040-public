// Attribution: idea port of zeux/meshoptimizer MIT https://github.com/zeux/meshoptimizer spatialorder.cpp morton sort; fresh Rust, no donor bytes.
//! Spatial-sort pre-pass vectors: morton remap + tiny Tipsify-after; deterministic integer math only.
fn ex(v: u32) -> u32 {
    let mut r = 0;
    for i in 0..8u32 {
        r |= ((v >> i) & 1) << (2 * i);
    }
    r
}
fn mo(x: u32, y: u32) -> u32 {
    ex(x) | (ex(y) << 1)
}
fn lru(idx: &[u32], cap: usize) -> u64 {
    let mut c: Vec<u32> = Vec::new();
    let mut m = 0;
    for v in idx {
        if let Some(p) = c.iter().position(|a| a == v) {
            c.remove(p);
            c.push(*v);
        } else {
            m += 1;
            if c.len() >= cap {
                c.remove(0);
            }
            c.push(*v);
        }
    }
    m
}
fn ratio(idx: &[u32], cap: usize) -> f32 {
    lru(idx, cap) as f32 / idx.len() as f32
}
fn grid() -> (Vec<u32>, Vec<u32>, Vec<[u32; 3]>) {
    let (w, h): (u32, u32) = (8, 8);
    let mut px = Vec::new();
    let mut py = Vec::new();
    for y in 0..h {
        for x in 0..w {
            px.push(x);
            py.push(y);
        }
    }
    let mut t = Vec::new();
    for y in 0..(h - 1) {
        for x in 0..(w - 1) {
            let a = y * w + x;
            let b = a + 1;
            let c = a + w;
            let d = c + 1;
            t.push([a, b, c]);
            t.push([b, d, c]);
        }
    }
    (px, py, t)
}
fn key(t: [u32; 3], px: &[u32], py: &[u32]) -> u32 {
    mo(
        (px[t[0] as usize] + px[t[1] as usize] + px[t[2] as usize]) / 3,
        (py[t[0] as usize] + py[t[1] as usize] + py[t[2] as usize]) / 3,
    )
}
fn morton(tris: &[[u32; 3]], px: &[u32], py: &[u32]) -> Vec<[u32; 3]> {
    let mut v = tris.to_vec();
    v.sort_by_key(|t| (key(*t, px, py), *t));
    v
}
fn tipsify(order: &[[u32; 3]]) -> Vec<[u32; 3]> {
    let mut rem = order.to_vec();
    let mut out = Vec::with_capacity(rem.len());
    let mut cur: Option<[u32; 3]> = None;
    while !rem.is_empty() {
        let mut p = 0;
        if let Some(c) = cur {
            for (i, t) in rem.iter().enumerate() {
                if t.contains(&c[0]) || t.contains(&c[1]) || t.contains(&c[2]) {
                    p = i;
                    break;
                }
            }
        }
        cur = Some(rem.remove(p));
        out.push(cur.expect("nonempty"));
    }
    out
}
fn flat(tris: &[[u32; 3]]) -> Vec<u32> {
    tris.iter().flatten().copied().collect()
}
fn bad(tris: &[[u32; 3]]) -> Vec<[u32; 3]> {
    let n = tris.len();
    let s = 31;
    let mut o = Vec::with_capacity(n);
    for i in 0..n {
        o.push(tris[(i * s) % n]);
    }
    o
}
#[test]
fn morton_keys_known() {
    assert_eq!(mo(0, 0), 0);
    assert_eq!(mo(1, 0), 1);
    assert_eq!(mo(0, 1), 2);
    assert_eq!(mo(1, 1), 3);
    assert_eq!(mo(2, 0), 4);
    assert_eq!(mo(0, 2), 8);
}
#[test]
fn locality_down_5pp() {
    const C: usize = 16;
    let (px, py, tris) = grid();
    let b = bad(&tris);
    let m = morton(&b, &px, &py);
    let t = tipsify(&m);
    let br = ratio(&flat(&b), C);
    let ar = ratio(&flat(&t), C);
    println!("miss {br:.3} -> {ar:.3}");
    assert!(ar + 0.05 <= br, "need 5pp drop");
}
#[test]
fn deterministic_x2() {
    let (px, py, tris) = grid();
    let b = bad(&tris);
    let a = tipsify(&morton(&b, &px, &py));
    let c = tipsify(&morton(&b, &px, &py));
    assert_eq!(a, c);
    assert_eq!(flat(&a).len(), tris.len() * 3);
}
