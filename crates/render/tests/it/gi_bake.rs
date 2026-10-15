//! gi_bake: rebake-identical + PALETTE match + lookup-only runtime gates.
use engine_render::gi::{
    palette_entry, sh_basis, sh_irradiance, tonemap_u8, BakeScene, DirLight, GiRuntime, Lightmap,
    PointLight, ProbeGrid, SkyAmbient, SurfaceSample, PALETTE, PALETTE_LEN, RUNTIME_OPS_PER_SAMPLE,
};

fn sky() -> SkyAmbient {
    SkyAmbient {
        sky: [0.55, 0.62, 0.72],
        ground: [0.22, 0.18, 0.15],
    }
}
fn samples4() -> Vec<SurfaceSample> {
    let mut v = Vec::new();
    for y in 0..4 {
        for x in 0..4 {
            v.push(SurfaceSample {
                pos: [x as f32 * 0.5, 0.0, y as f32 * 0.5],
                normal: [0.06 * (x as f32 - 1.5), 1.0, -0.04 * (y as f32 - 1.5)],
                albedo: [0.75 - 0.05 * x as f32, 0.7, 0.62 + 0.05 * y as f32],
                occlusion: 1.0 - 0.08 * ((x + y) % 3) as f32,
            });
        }
    }
    v
}
fn bake_all() -> (Lightmap, ProbeGrid) {
    let d = vec![DirLight {
        dir: [0.3, 1.0, 0.4],
        color: [1.2, 1.1, 1.0],
    }];
    let p = vec![PointLight {
        pos: [0.75, 1.5, 0.75],
        color: [2.0, 1.2, 0.8],
        inv_range_sq: 1.0 / 9.0,
    }];
    let s = samples4();
    let sc = BakeScene {
        dirs: &d,
        points: &p,
        sky: sky(),
    };
    let m = Lightmap::bake(4, 4, &s, &sc);
    let g = ProbeGrid::bake(2, 2, 2, [0.0, 0.0, 0.0], [1.5, 1.0, 1.5], &sc);
    (m, g)
}
/// independent nearest search (gate re-derives the argmin, not via bake).
fn nearest_ref(c: [u8; 3]) -> u8 {
    let (mut bi, mut bd) = (0u8, u32::MAX);
    for (i, p) in PALETTE.iter().enumerate() {
        let d = (c[0].abs_diff(p[0]) as u32).pow(2)
            + (c[1].abs_diff(p[1]) as u32).pow(2)
            + (c[2].abs_diff(p[2]) as u32).pow(2);
        if d < bd {
            bd = d;
            bi = i as u8;
        }
    }
    bi
}
fn finite(v: [f32; 3]) -> bool {
    v.iter().all(|x| x.is_finite())
}

#[test]
fn rebake_identical() {
    let (m1, g1) = bake_all();
    let (m2, g2) = bake_all();
    assert_eq!(m1.tex, m2.tex);
    assert_eq!(m1.idx, m2.idx);
    assert_eq!(m1.digest(), m2.digest());
    assert_eq!(g1.sh, g2.sh);
    assert_eq!(g1.digest(), g2.digest());
    assert_eq!(g1.probe_count(), 8);
}

#[test]
fn palette_match_gate() {
    let (m, _) = bake_all();
    assert!(m.is_palette_exact());
    for (i, t) in m.tex.iter().enumerate() {
        assert_eq!(m.idx[i], nearest_ref(tonemap_u8(*t)), "texel {}", i);
    }
    for y in 0..4 {
        for x in 0..4 {
            let u = x as f32 / 3.0;
            let v = y as f32 / 3.0;
            let c = m.sample_nn(u, v);
            assert_eq!(c, PALETTE[m.idx[y * 4 + x] as usize]);
            assert!(PALETTE.contains(&c));
            assert_eq!(palette_entry(m.idx[y * 4 + x]), c);
        }
    }
}

#[test]
fn runtime_lookup_sane() {
    let (m, g) = bake_all();
    assert_eq!(m.sample_nn(-1.0, 2.0), m.sample_nn(0.0, 1.0));
    let f = m.sample_nn_f32(0.5, 0.5);
    assert!(f.iter().all(|&x| (0.0..=1.0).contains(&x)));
    let rt = GiRuntime {
        map: &m,
        probes: &g,
    };
    for i in 0..5000 {
        let u = (i % 64) as f32 / 63.0;
        let v = ((i / 64) % 64) as f32 / 63.0;
        assert!(PALETTE.contains(&rt.light(u, v)));
        let pos = [u * 1.5, 0.5, v * 1.5];
        assert!(finite(rt.irradiance(pos, [0.0, 1.0, 0.0])));
    }
    assert_eq!(g.sample_trilerp(g.probe_pos(0, 0, 0)), g.sh[0]);
    assert_eq!(g.sample_trilerp(g.probe_pos(9, 9, 9)), g.sh[7]);
    assert_eq!(
        g.sample_trilerp([1e6, -1e6, 0.0]),
        g.sample_trilerp(g.probe_pos(1, 0, 0))
    );
    let d = vec![];
    let p = vec![];
    let sc = BakeScene {
        dirs: &d,
        points: &p,
        sky: sky(),
    };
    let empty = Lightmap::bake(0, 0, &[], &sc);
    assert_eq!(empty.sample_nn(0.5, 0.5), PALETTE[0]);
    assert!(empty.is_palette_exact());
}

#[test]
fn bleed_deterministic_and_paletted() {
    let (mut a, _) = bake_all();
    let (mut b, _) = bake_all();
    let plain = a.digest();
    a.blur_bleed(2);
    b.blur_bleed(2);
    assert_eq!(a.digest(), b.digest());
    assert_ne!(a.digest(), plain);
    assert!(a.is_palette_exact());
    for (i, t) in a.tex.iter().enumerate() {
        assert_eq!(a.idx[i], nearest_ref(tonemap_u8(*t)));
    }
}

#[test]
fn sh_basis_and_budget_consts() {
    assert_eq!((PALETTE_LEN, PALETTE.len()), (16, 16));
    assert_eq!(RUNTIME_OPS_PER_SAMPLE, 64);
    let b = sh_basis([0.0, 1.0, 0.0]);
    assert!((b[0] - 0.282095).abs() < 1e-6);
    assert!((b[1] + 0.488603).abs() < 1e-6);
    let zero = [[0.0f32; 3]; 9];
    assert_eq!(sh_irradiance(&zero, [0.0, 1.0, 0.0]), [0.0, 0.0, 0.0]);
    assert!(finite(sh_irradiance(&zero, [0.0, 0.0, 0.0])));
    for (i, p) in PALETTE.iter().enumerate() {
        assert_eq!(palette_entry(i as u8), *p);
    }
}
