//! lipsync: offline phoneme bake. Bake-only; runtime inference refused loud.
use crate::viseme::{self, Shape, VisemeClip};
use std::fmt;
/// Fixed crossfade into next cue (smooth, deterministic).
pub const BLEND_S: f32 = 0.08;
/// Neutral tail after last cue so baked clip ends at zeros.
pub const TAIL_S: f32 = 0.12;
/// Default bake grid (30 Hz).
pub const DEFAULT_DT: f32 = 1.0 / 30.0;
/// Key caps (alloc bound).
pub const MAX_KEYS: usize = 65536;
/// TSV size cap (1 MiB).
pub const MAX_TSV_BYTES: usize = 1 << 20;
/// One cue: hold shape on [t0,t1]; impulse (t0==t1) holds until next cue.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cue {
    pub t0: f32,
    pub t1: f32,
    pub shape: Shape,
}
/// Bake failure (loud, no silent fallback).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BakeError {
    Empty,
    TooLarge,
    BadLine,
    BadTime,
    BadShape,
    BadDt,
    TooManyKeys,
    RuntimeRefused,
}
impl BakeError {
    /// Stable message.
    pub fn as_str(self) -> &'static str {
        match self {
            BakeError::Empty => "empty",
            BakeError::TooLarge => "too-large",
            BakeError::BadLine => "bad-line",
            BakeError::BadTime => "bad-time",
            BakeError::BadShape => "bad-shape",
            BakeError::BadDt => "bad-dt",
            BakeError::TooManyKeys => "too-many-keys",
            BakeError::RuntimeRefused => "runtime-refused",
        }
    }
}
impl fmt::Display for BakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
/// Parse phoneme TSV: `t shape` (hold) or `t0 t1 shape`; `#` comments + blanks skipped.
/// Times finite, >=0, t1>=t0; shape in A-H,X. Sorted by (t0,t1).
pub fn parse_tsv(text: &str) -> Result<Vec<Cue>, BakeError> {
    if text.len() > MAX_TSV_BYTES {
        return Err(BakeError::TooLarge);
    }
    let mut out = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let p: Vec<&str> = l.split_whitespace().collect();
        let (a, b, sc) = match p.len() {
            2 => (p[0], p[0], p[1]),
            3 => (p[0], p[1], p[2]),
            _ => return Err(BakeError::BadLine),
        };
        let t0: f32 = a.parse().map_err(|_| BakeError::BadTime)?;
        let t1: f32 = b.parse().map_err(|_| BakeError::BadTime)?;
        if !t0.is_finite() || !t1.is_finite() || t0 < 0.0 || t1 < 0.0 || t1 < t0 {
            return Err(BakeError::BadTime);
        }
        if sc.len() != 1 {
            return Err(BakeError::BadShape);
        }
        let sh = viseme::shape_from_char(sc.chars().next().unwrap()).ok_or(BakeError::BadShape)?;
        out.push(Cue { t0, t1, shape: sh });
        if out.len() > MAX_KEYS {
            return Err(BakeError::TooManyKeys);
        }
    }
    if out.is_empty() {
        return Err(BakeError::Empty);
    }
    out.sort_by(|a, b| a.t0.total_cmp(&b.t0).then(a.t1.total_cmp(&b.t1)));
    Ok(out)
}
fn lerp52(a: [f32; viseme::N], b: [f32; viseme::N], f: f32) -> [f32; viseme::N] {
    let mut o = [0.0f32; viseme::N];
    for i in 0..viseme::N {
        o[i] = a[i] + (b[i] - a[i]) * f;
    }
    o
}
fn proto_at(cs: &[Cue], t: f32) -> [f32; viseme::N] {
    let z = [0.0f32; viseme::N];
    if cs.is_empty() || t.is_nan() {
        return z;
    }
    if t < cs[0].t0 {
        if t >= cs[0].t0 - BLEND_S {
            let f = ((t - (cs[0].t0 - BLEND_S)) / BLEND_S).clamp(0.0, 1.0);
            return lerp52(z, viseme::prototype(cs[0].shape), f);
        }
        return z;
    }
    let (mut lo, mut hi) = (0usize, cs.len());
    while hi - lo > 1 {
        let m = (lo + hi) / 2;
        if cs[m].t0 <= t {
            lo = m;
        } else {
            hi = m;
        }
    }
    let cur = cs[lo];
    if t >= cur.t0 && t <= cur.t1 {
        return viseme::prototype(cur.shape);
    }
    if let Some(nxt) = cs.get(lo + 1) {
        if t >= nxt.t0 - BLEND_S && t < nxt.t0 {
            let f = ((t - (nxt.t0 - BLEND_S)) / BLEND_S).clamp(0.0, 1.0);
            return lerp52(
                viseme::prototype(cur.shape),
                viseme::prototype(nxt.shape),
                f,
            );
        }
        return viseme::prototype(cur.shape);
    }
    let end = cur.t1.max(cur.t0);
    if t <= end {
        return viseme::prototype(cur.shape);
    }
    if t <= end + BLEND_S {
        return lerp52(
            viseme::prototype(cur.shape),
            z,
            ((t - end) / BLEND_S).clamp(0.0, 1.0),
        );
    }
    z
}
/// Bake cues to uniform VisemeClip at dt (0.005..=0.25). Deterministic: sorted copy, i*dt grid.
pub fn bake(cues: &[Cue], dt: f32) -> Result<VisemeClip, BakeError> {
    if cues.is_empty() {
        return Err(BakeError::Empty);
    }
    if !dt.is_finite() || !(0.005..=0.25).contains(&dt) {
        return Err(BakeError::BadDt);
    }
    if cues.len() > MAX_KEYS {
        return Err(BakeError::TooManyKeys);
    }
    let mut end = 0.0f32;
    for c in cues {
        end = end.max(c.t1.max(c.t0));
    }
    end += TAIL_S;
    let n = (end / dt).floor() as usize + 1;
    if n == 0 || n > MAX_KEYS {
        return Err(BakeError::TooManyKeys);
    }
    let mut s = cues.to_vec();
    s.sort_by(|a, b| a.t0.total_cmp(&b.t0).then(a.t1.total_cmp(&b.t1)));
    let mut times = Vec::with_capacity(n);
    let mut weights = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 * dt;
        times.push(t);
        weights.push(proto_at(&s, t));
    }
    Ok(VisemeClip { times, weights })
}
/// Parse + bake in one call.
pub fn bake_tsv(text: &str, dt: f32) -> Result<VisemeClip, BakeError> {
    bake(&parse_tsv(text)?, dt)
}
/// Bake SHA (FNV-1a64 of baked tracks).
pub fn bake_hash(c: &VisemeClip) -> u64 {
    viseme::clip_hash(c)
}
/// Bake-only gate: audio/MFCC/TTS inference refused loud (never infer at runtime).
pub fn infer_from_audio(_bytes: &[u8]) -> Result<VisemeClip, BakeError> {
    Err(BakeError::RuntimeRefused)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_shapes_comments() {
        let cs = parse_tsv("# rhubarb-pattern\n0.00\tA\n0.20 B\n0.40\t0.60\tC\n").unwrap();
        assert_eq!(cs.len(), 3);
        assert_eq!(cs[0].shape, Shape::A);
        assert_eq!((cs[2].t0, cs[2].t1), (0.4, 0.6));
        let u = parse_tsv("0.50 X\n0.00 a\n").unwrap();
        assert_eq!(u[0].shape, Shape::A);
    }
    #[test]
    fn parse_errors() {
        assert_eq!(parse_tsv(""), Err(BakeError::Empty));
        assert_eq!(parse_tsv("0.0 Z"), Err(BakeError::BadShape));
        assert_eq!(parse_tsv("oops A"), Err(BakeError::BadTime));
        assert_eq!(parse_tsv("-1.0 A"), Err(BakeError::BadTime));
        assert_eq!(parse_tsv("0.5 0.1 A"), Err(BakeError::BadTime));
        assert_eq!(parse_tsv("0.0"), Err(BakeError::BadLine));
        assert_eq!(bake(&[], DEFAULT_DT), Err(BakeError::Empty));
        assert_eq!(
            bake(
                &[Cue {
                    t0: 0.0,
                    t1: 0.0,
                    shape: Shape::A
                }],
                0.0
            ),
            Err(BakeError::BadDt)
        );
    }
    #[test]
    fn bake_sha_x2_avalanche() {
        let t = "0.00\tA\n0.20\tB\n0.40\tX\n";
        let (a, b) = (
            bake_tsv(t, DEFAULT_DT).unwrap(),
            bake_tsv(t, DEFAULT_DT).unwrap(),
        );
        assert_eq!(bake_hash(&a), bake_hash(&b));
        assert_eq!(a.times.len(), b.times.len());
        let c = bake_tsv("0.00\tA\n0.20\tC\n0.40\tX\n", DEFAULT_DT).unwrap();
        assert_ne!(bake_hash(&a), bake_hash(&c));
    }
    #[test]
    fn bake_holds_decays() {
        let c = bake_tsv("0.10\tA\n0.50\tB\n", DEFAULT_DT).unwrap();
        assert_eq!(c.times.len(), c.weights.len());
        for i in 1..c.times.len() {
            assert!(c.times[i] > c.times[i - 1]);
        }
        let w = viseme::sample(&c, 0.10);
        let p = viseme::prototype(Shape::A);
        for (wi, pi) in w.iter().zip(p.iter()) {
            assert!((wi - pi).abs() < 1e-6);
        }
        assert_eq!(viseme::sample(&c, 99.0), [0.0f32; viseme::N]);
    }
    #[test]
    fn runtime_refused_loud() {
        assert_eq!(infer_from_audio(&[1, 2, 3]), Err(BakeError::RuntimeRefused));
        assert_eq!(BakeError::RuntimeRefused.as_str(), "runtime-refused");
    }
}
