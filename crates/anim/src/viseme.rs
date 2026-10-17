//! viseme: 52-weight sampler. Indices at runtime, names for tooling only.
use engine_core::Fnv64;
pub const N: usize = 52;
/// ARKit-52 names, canonical order (tooling only; sampler uses indices).
pub const NAMES: [&str; N] = [
    "browDownLeft",
    "browDownRight",
    "browInnerUp",
    "browOuterUpLeft",
    "browOuterUpRight",
    "cheekPuff",
    "cheekSquintLeft",
    "cheekSquintRight",
    "eyeBlinkLeft",
    "eyeBlinkRight",
    "eyeLookDownLeft",
    "eyeLookDownRight",
    "eyeLookInLeft",
    "eyeLookInRight",
    "eyeLookOutLeft",
    "eyeLookOutRight",
    "eyeLookUpLeft",
    "eyeLookUpRight",
    "eyeSquintLeft",
    "eyeSquintRight",
    "eyeWideLeft",
    "eyeWideRight",
    "jawForward",
    "jawLeft",
    "jawRight",
    "jawOpen",
    "mouthClose",
    "mouthDimpleLeft",
    "mouthDimpleRight",
    "mouthFrownLeft",
    "mouthFrownRight",
    "mouthFunnel",
    "mouthLeft",
    "mouthLowerDownLeft",
    "mouthLowerDownRight",
    "mouthPressLeft",
    "mouthPressRight",
    "mouthPucker",
    "mouthRight",
    "mouthRollLower",
    "mouthRollUpper",
    "mouthShrugLower",
    "mouthShrugUpper",
    "mouthSmileLeft",
    "mouthSmileRight",
    "mouthStretchLeft",
    "mouthStretchRight",
    "mouthUpperUpLeft",
    "mouthUpperUpRight",
    "noseSneerLeft",
    "noseSneerRight",
    "tongueOut",
];
/// Linear name->index (deterministic, no map).
pub fn index_of(n: &str) -> Option<usize> {
    let mut i = 0;
    while i < N {
        if NAMES[i] == n {
            return Some(i);
        }
        i += 1;
    }
    None
}
/// Rhubarb 9 mouth shapes (extended Preston Blair set).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    X,
}
/// Parse shape char, case-insensitive.
pub fn shape_from_char(c: char) -> Option<Shape> {
    match c.to_ascii_uppercase() {
        'A' => Some(Shape::A),
        'B' => Some(Shape::B),
        'C' => Some(Shape::C),
        'D' => Some(Shape::D),
        'E' => Some(Shape::E),
        'F' => Some(Shape::F),
        'G' => Some(Shape::G),
        'H' => Some(Shape::H),
        'X' => Some(Shape::X),
        _ => None,
    }
}
/// Shape to char.
pub fn shape_char(s: Shape) -> char {
    match s {
        Shape::A => 'A',
        Shape::B => 'B',
        Shape::C => 'C',
        Shape::D => 'D',
        Shape::E => 'E',
        Shape::F => 'F',
        Shape::G => 'G',
        Shape::H => 'H',
        Shape::X => 'X',
    }
}
/// Shape prototype weights in [0,1]; X = neutral zeros.
pub fn prototype(s: Shape) -> [f32; N] {
    let mut w = [0.0f32; N];
    match s {
        Shape::X => {}
        Shape::A => {
            w[25] = 1.0;
            w[31] = 0.35;
            w[45] = 0.25;
            w[46] = 0.25;
        }
        Shape::B => {
            w[26] = 1.0;
            w[35] = 0.6;
            w[36] = 0.6;
        }
        Shape::C => {
            w[45] = 0.85;
            w[46] = 0.85;
            w[25] = 0.25;
            w[43] = 0.3;
            w[44] = 0.3;
        }
        Shape::D => {
            w[37] = 1.0;
            w[31] = 0.6;
            w[25] = 0.35;
        }
        Shape::E => {
            w[25] = 0.55;
            w[33] = 0.7;
            w[34] = 0.7;
            w[45] = 0.35;
            w[46] = 0.35;
        }
        Shape::F => {
            w[47] = 0.8;
            w[48] = 0.8;
            w[33] = 0.45;
            w[34] = 0.45;
            w[25] = 0.3;
        }
        Shape::G => {
            w[51] = 0.9;
            w[25] = 0.4;
            w[31] = 0.25;
        }
        Shape::H => {
            w[43] = 1.0;
            w[44] = 1.0;
            w[27] = 0.5;
            w[28] = 0.5;
            w[25] = 0.3;
        }
    }
    w
}
/// Baked weight keys: parallel times+weights, times strictly increasing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VisemeClip {
    pub times: Vec<f32>,
    pub weights: Vec<[f32; N]>,
}
use super::clip::clamp01;
/// Sample weights at t: clamp, binary search, per-lane lerp. Endpoints exact; empty/mismatch/NaN -> zeros.
pub fn sample(c: &VisemeClip, t: f32) -> [f32; N] {
    let z = [0.0f32; N];
    if t.is_nan() {
        return z;
    }
    if c.times.len() != c.weights.len() || c.times.is_empty() {
        return z;
    }
    if t <= c.times[0] {
        return c.weights[0];
    }
    let last = c.times.len() - 1;
    if t >= c.times[last] {
        return c.weights[last];
    }
    let (mut lo, mut hi) = (0usize, last);
    while hi - lo > 1 {
        let m = (lo + hi) / 2;
        if c.times[m] <= t {
            lo = m;
        } else {
            hi = m;
        }
    }
    let sp = c.times[hi] - c.times[lo];
    let f = if sp <= 0.0 {
        0.0
    } else {
        clamp01((t - c.times[lo]) / sp)
    };
    let (a, b) = (c.weights[lo], c.weights[hi]);
    let mut o = [0.0f32; N];
    for i in 0..N {
        o[i] = a[i] + (b[i] - a[i]) * f;
    }
    o
}
/// FNV-1a64 over len + time/weight bits.
pub fn clip_hash(c: &VisemeClip) -> u64 {
    let mut h = Fnv64::new();
    h.write_len(c.times.len());
    for t in &c.times {
        h.write_f32(*t);
    }
    for w in &c.weights {
        for v in w {
            h.write_f32(*v);
        }
    }
    h.finish()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_52_unique() {
        assert_eq!(NAMES.len(), 52);
        for (i, &name) in NAMES.iter().enumerate() {
            assert_eq!(index_of(name), Some(i));
        }
        for (i, a) in NAMES.iter().enumerate() {
            for b in &NAMES[i + 1..] {
                assert_ne!(a, b);
            }
        }
        assert_eq!(index_of("nope"), None);
    }
    #[test]
    fn shapes_roundtrip_bounds() {
        for (s, ch) in [
            (Shape::A, 'A'),
            (Shape::B, 'B'),
            (Shape::C, 'C'),
            (Shape::D, 'D'),
            (Shape::E, 'E'),
            (Shape::F, 'F'),
            (Shape::G, 'G'),
            (Shape::H, 'H'),
            (Shape::X, 'X'),
        ] {
            assert_eq!(shape_from_char(ch), Some(s));
            assert_eq!(shape_from_char(ch.to_ascii_lowercase()), Some(s));
            assert_eq!(shape_char(s), ch);
            for v in prototype(s) {
                assert!((0.0..=1.0).contains(&v) && !v.is_nan());
            }
        }
        assert_eq!(prototype(Shape::X), [0.0f32; N]);
        assert_eq!(shape_from_char('Z'), None);
    }
    #[test]
    fn sample_endpoints_empty() {
        let c = VisemeClip {
            times: vec![0.0, 1.0],
            weights: vec![prototype(Shape::A), prototype(Shape::B)],
        };
        assert_eq!(sample(&c, -1.0), prototype(Shape::A));
        assert_eq!(sample(&c, 1.0), prototype(Shape::B));
        assert_eq!(sample(&c, 99.0), prototype(Shape::B));
        assert_eq!(sample(&VisemeClip::default(), 0.5), [0.0f32; N]);
        assert_eq!(sample(&c, f32::NAN), [0.0f32; N]);
    }
    #[test]
    fn sample_mid_lerps() {
        let c = VisemeClip {
            times: vec![0.0, 1.0],
            weights: vec![prototype(Shape::A), prototype(Shape::B)],
        };
        let (a, b, w) = (prototype(Shape::A), prototype(Shape::B), sample(&c, 0.5));
        for i in 0..N {
            assert!((w[i] - (a[i] + (b[i] - a[i]) * 0.5)).abs() < 1e-6);
        }
    }
    #[test]
    fn hash_det_avalanche() {
        let c = VisemeClip {
            times: vec![0.0, 0.5],
            weights: vec![prototype(Shape::A), prototype(Shape::X)],
        };
        assert_eq!(clip_hash(&c), clip_hash(&c.clone()));
        let mut d = c.clone();
        d.weights[0][25] += f32::EPSILON;
        assert_ne!(clip_hash(&c), clip_hash(&d));
    }
    #[test]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "timing test reads the wall clock; it never enters the tick"
    )]
    fn sample_p99_under_point1ms() {
        let c = VisemeClip {
            times: vec![0.0, 0.5, 1.0],
            weights: vec![
                prototype(Shape::A),
                prototype(Shape::B),
                prototype(Shape::X),
            ],
        };
        let n = 2000;
        let mut ds = Vec::with_capacity(n);
        for i in 0..n {
            let t = (i as f32 * 0.0007) % 1.0;
            let s = std::time::Instant::now();
            std::hint::black_box(sample(&c, t));
            ds.push(s.elapsed().as_nanos());
        }
        ds.sort_unstable();
        assert!(ds[n * 99 / 100] < 100_000, "p99={}", ds[n * 99 / 100]);
    }
    #[test]
    fn bake_drift() {
        // Sim bytes stay fixed while bake-only tables cook beside them.
        let sim = crate::Clip {
            tracks: vec![crate::Track {
                times: vec![0.0, 1.0],
                values: vec![[0.0, 0.0, 0.0], [2.0, 4.0, 6.0]],
            }],
        };
        let sim_hash = crate::clip_hash(&sim);
        let first =
            crate::lipsync::bake_tsv("0.00 A\n0.20 B\n0.40 X\n", crate::lipsync::DEFAULT_DT)
                .unwrap();
        let second =
            crate::lipsync::bake_tsv("0.00 A\n0.20 B\n0.40 X\n", crate::lipsync::DEFAULT_DT)
                .unwrap();
        assert_eq!(
            crate::lipsync::bake_hash(&first),
            crate::lipsync::bake_hash(&second)
        );
        assert_eq!(crate::clip_hash(&sim), sim_hash);
        // A different line cooks different bytes, sim bytes still fixed.
        let other =
            crate::lipsync::bake_tsv("0.00 F\n0.16 G\n0.80 X\n", crate::lipsync::DEFAULT_DT)
                .unwrap();
        assert_ne!(
            crate::lipsync::bake_hash(&first),
            crate::lipsync::bake_hash(&other)
        );
        assert_eq!(
            crate::clip_hash(&sim),
            sim_hash,
            "bake must not drift sim bytes"
        );
        assert_eq!(crate::clip_hash(&sim), crate::clip_hash(&sim));
        // Bad bakes refuse loudly and leave sim bytes alone.
        assert!(crate::lipsync::bake_tsv("", crate::lipsync::DEFAULT_DT).is_err());
        assert_eq!(crate::clip_hash(&sim), sim_hash);
        // Posted morph cost for the baked lines: worst key decides.
        let peak =
            crate::face::peak_active_count(&first).max(crate::face::peak_active_count(&other));
        assert!(peak >= 3, "speech must light several lanes, got {peak}");
        let full = crate::face::morph_cost_us(
            crate::face::TIER_B_HEAD_VERTS,
            u8::try_from(crate::face::MAX_GPU_MORPHS).unwrap(),
        );
        assert!(
            (full - 625.5).abs() < 1e-3,
            "full morph cost posts 625.5us, got {full}"
        );
        assert!(crate::face::morph_within_budget(
            crate::face::TIER_B_HEAD_VERTS,
            peak
        ));
        assert!(crate::face::morph_within_budget(
            crate::face::TIER_B_HEAD_VERTS,
            u8::try_from(crate::face::MAX_GPU_MORPHS).unwrap()
        ));
    }
}
