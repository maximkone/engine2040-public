//! clip: baked quantized clips + SoA sampler, tick-fixed. Clamp + cursor + lerp, FNV traj hash.
use engine_core::{quat_mul, Fnv64};
/// Fixed tick step in seconds. Single owner of the value is `engine_core::DT`.
pub const TICK_DT: f32 = engine_core::DT;
#[derive(Clone, Debug)]
pub struct QTrack {
    pub times: Vec<f32>,
    pub keys: Vec<[u16; 3]>,
    pub min: [f32; 3],
    pub max: [f32; 3],
}
#[derive(Clone, Debug, Default)]
pub struct QClip {
    pub tracks: Vec<QTrack>,
    pub dur: f32,
}
#[derive(Clone, Debug)]
pub struct Cursor {
    pub pos: Vec<usize>,
}
impl Cursor {
    pub fn new(n: usize) -> Self {
        Self { pos: vec![0; n] }
    }
    pub fn reset(&mut self) {
        for p in self.pos.iter_mut() {
            *p = 0;
        }
    }
}
pub(crate) fn clamp01(t: f32) -> f32 {
    if t.is_nan() {
        0.0
    } else {
        t.clamp(0.0, 1.0)
    }
}
fn deq(q: [u16; 3], min: [f32; 3], max: [f32; 3]) -> [f32; 3] {
    [
        min[0] + (max[0] - min[0]) * (f32::from(q[0]) / 65535.0),
        min[1] + (max[1] - min[1]) * (f32::from(q[1]) / 65535.0),
        min[2] + (max[2] - min[2]) * (f32::from(q[2]) / 65535.0),
    ]
}
pub fn bake_track(times: Vec<f32>, values: Vec<[f32; 3]>) -> QTrack {
    if times.len() != values.len() || times.is_empty() {
        return QTrack {
            times: Vec::new(),
            keys: Vec::new(),
            min: [0.0; 3],
            max: [0.0; 3],
        };
    }
    let (mut mn, mut mx) = (values[0], values[0]);
    for v in &values[1..] {
        for i in 0..3 {
            if v[i] < mn[i] {
                mn[i] = v[i];
            }
            if v[i] > mx[i] {
                mx[i] = v[i];
            }
        }
    }
    let mut keys = Vec::with_capacity(values.len());
    for v in &values {
        let mut q = [0u16; 3];
        for i in 0..3 {
            let s = mx[i] - mn[i];
            q[i] = if s <= 0.0 {
                0
            } else {
                (clamp01((v[i] - mn[i]) / s) * 65535.0)
                    .round()
                    .clamp(0.0, 65535.0) as u16
            };
        }
        keys.push(q);
    }
    QTrack {
        times,
        keys,
        min: mn,
        max: mx,
    }
}
pub fn bake_clip(tracks: Vec<QTrack>) -> QClip {
    let mut dur = 0.0f32;
    for tr in &tracks {
        if let Some(&t) = tr.times.last() {
            if t > dur {
                dur = t;
            }
        }
    }
    QClip { tracks, dur }
}
fn interval(times: &[f32], hint: usize, t: f32) -> (usize, f32) {
    let last = times.len() - 1;
    let mut lo = hint.min(last);
    if times[lo] <= t {
        while lo < last && times[lo + 1] <= t {
            lo += 1;
        }
        if lo >= last {
            return (last, 0.0);
        }
    } else {
        let (mut a, mut b) = (0usize, last);
        while b - a > 1 {
            let m = (a + b) / 2;
            if times[m] <= t {
                a = m;
            } else {
                b = m;
            }
        }
        lo = a;
    }
    let (t0, t1) = (times[lo], times[lo + 1]);
    let s = t1 - t0;
    (lo, if s <= 0.0 { 0.0 } else { clamp01((t - t0) / s) })
}
pub fn sample_track(tr: &QTrack, t: f32, hint: &mut usize) -> [f32; 3] {
    if tr.times.len() != tr.keys.len() || tr.times.is_empty() {
        return [0.0; 3];
    }
    // NaN time maps to the first key, so a one-key track never reaches `interval`.
    let t = if t.is_nan() { tr.times[0] } else { t };
    let last = tr.times.len() - 1;
    if t <= tr.times[0] {
        *hint = 0;
        return deq(tr.keys[0], tr.min, tr.max);
    }
    if t >= tr.times[last] {
        *hint = last;
        return deq(tr.keys[last], tr.min, tr.max);
    }
    let (lo, f) = interval(&tr.times, *hint, t);
    *hint = lo;
    let a = deq(tr.keys[lo], tr.min, tr.max);
    let b = deq(tr.keys[lo + 1], tr.min, tr.max);
    [
        a[0] + (b[0] - a[0]) * f,
        a[1] + (b[1] - a[1]) * f,
        a[2] + (b[2] - a[2]) * f,
    ]
}
pub fn sample_all(c: &QClip, t: f32, cur: &mut Cursor, out: &mut [[f32; 3]]) {
    if cur.pos.len() != c.tracks.len() {
        cur.pos.resize(c.tracks.len(), 0);
    }
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = if i < c.tracks.len() {
            sample_track(&c.tracks[i], t, &mut cur.pos[i])
        } else {
            [0.0; 3]
        };
    }
}
pub fn qclip_hash(c: &QClip) -> u64 {
    let mut h = Fnv64::new();
    h.write_len(c.tracks.len());
    h.write_f32(c.dur);
    for tr in &c.tracks {
        h.write_len(tr.times.len());
        for v in tr.min.iter().chain(tr.max.iter()) {
            h.write_f32(*v);
        }
        for t in &tr.times {
            h.write_f32(*t);
        }
        for k in &tr.keys {
            for x in k {
                h.write_u16(*x);
            }
        }
    }
    h.finish()
}
pub fn traj_hash(c: &QClip, joint: usize, ticks: u32) -> u64 {
    let mut h = Fnv64::new();
    h.write_u32(ticks);
    h.write_u64(joint as u64);
    let mut cur = Cursor::new(c.tracks.len());
    for k in 0..ticks {
        let t = if c.dur <= 0.0 {
            0.0
        } else {
            (k as f32) * TICK_DT % c.dur
        };
        let p = if joint < c.tracks.len() {
            sample_track(&c.tracks[joint], t, &mut cur.pos[joint])
        } else {
            [0.0; 3]
        };
        for x in p {
            h.write_f32(x);
        }
    }
    h.finish()
}
pub fn slip(a: [f32; 3], b: [f32; 3], dt: f32) -> f32 {
    if dt <= 0.0 {
        return 0.0;
    }
    let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
    if !dx.is_finite() || !dz.is_finite() {
        return 0.0;
    }
    (dx * dx + dz * dz).sqrt() / dt
}
pub fn avg_slip(c: &QClip, foot: usize, ticks: u32) -> f32 {
    if ticks < 2 || foot >= c.tracks.len() {
        return 0.0;
    }
    let mut cur = Cursor::new(c.tracks.len());
    let mut p = sample_track(&c.tracks[foot], 0.0, &mut cur.pos[foot]);
    let mut acc = 0.0f32;
    for k in 1..ticks {
        let mut t = (k as f32) * TICK_DT;
        if t > c.dur {
            t = c.dur;
        }
        let q = sample_track(&c.tracks[foot], t, &mut cur.pos[foot]);
        acc += slip(p, q, TICK_DT);
        p = q;
    }
    acc / (ticks - 1) as f32
}

/// The average horizontal speed of a foot over the ground, in meters a
/// second, read from the foot's **world** position.
///
/// [`avg_slip`] reads the foot's own track, which is its offset from its
/// parent: the foot's motion inside the leg, not its motion over the sand.
/// A foot slides when the whole body travels over a planted foot, so the
/// slip meter wants the world position. And the world position of a local
/// clip is the offsets composed down the chain with the joint rotations,
/// exactly the skinning convention: every joint's offset is turned by its
/// parent's world rotation before it is added.
///
/// `rotations` carries one track per joint; a clip with no rotation lanes
/// (a version-1 wire) reads as if every rotation were the identity. An
/// empty skeleton or an out-of-range foot reads 0.0.
pub fn avg_world_slip(
    c: &QClip,
    rotations: &[crate::pose::QuatTrack],
    skeleton: &crate::contract::Skeleton,
    foot: usize,
    ticks: u32,
) -> f32 {
    if ticks < 2 || foot >= c.tracks.len() {
        return 0.0;
    }
    // The chain from the root down to the foot, in parent order.
    let mut chain = Vec::new();
    let mut at = Some(foot);
    while let Some(joint) = at {
        if joint >= skeleton.joints.len() {
            return 0.0;
        }
        chain.push(joint);
        at = skeleton.joints[joint].parent;
    }
    chain.reverse();
    let world = |time: f32, hint: &mut Cursor| -> [f32; 3] {
        let mut pos = [0.0f32; 3];
        let mut rot = [0.0, 0.0, 0.0, 1.0];
        for joint in &chain {
            let local = sample_track(&c.tracks[*joint], time, &mut hint.pos[*joint]);
            let turned = crate::contract::rotate(&local, &rot);
            pos = [pos[0] + turned[0], pos[1] + turned[1], pos[2] + turned[2]];
            let local_rot = rotations
                .get(*joint)
                .and_then(|track| crate::pose::sample_quat_track(track, time))
                .unwrap_or([0.0, 0.0, 0.0, 1.0]);
            rot = quat_mul(rot, local_rot);
        }
        pos
    };
    let mut cur = Cursor::new(c.tracks.len());
    let mut p = world(0.0, &mut cur);
    let mut acc = 0.0f32;
    for k in 1..ticks {
        let mut t = k as f32 * TICK_DT;
        if t > c.dur {
            t = c.dur;
        }
        let q = world(t, &mut cur);
        acc += slip(p, q, TICK_DT);
        p = q;
    }
    acc / (ticks - 1) as f32
}
#[cfg(test)]
mod tests {
    use super::*;
    fn lin() -> QClip {
        bake_clip(vec![
            bake_track(vec![0.0, 1.0], vec![[0.0, 0.0, 0.0], [2.0, 4.0, 6.0]]),
            bake_track(vec![0.0, 1.0], vec![[1.0, 0.0, 0.0], [1.0, 0.0, 0.0]]),
        ])
    }
    #[test]
    fn endpoints_clamp() {
        let c = lin();
        let mut h = 0;
        let a = sample_track(&c.tracks[0], -1.0, &mut h);
        let mut h2 = 0;
        let b = sample_track(&c.tracks[0], 0.0, &mut h2);
        assert_eq!(a, b);
        for i in 0..3 {
            assert!((a[i] - [0.0, 0.0, 0.0][i]).abs() < 2e-3, "{a:?}");
        }
        let mut h3 = 0;
        let e = sample_track(&c.tracks[0], 99.0, &mut h3);
        for i in 0..3 {
            assert!((e[i] - [2.0, 4.0, 6.0][i]).abs() < 2e-3, "{e:?}");
        }
        let empty = QTrack {
            times: vec![],
            keys: vec![],
            min: [0.0; 3],
            max: [0.0; 3],
        };
        assert_eq!(sample_track(&empty, 0.5, &mut h3), [0.0; 3]);
    }
    #[test]
    fn nan_time_on_a_one_key_track_returns_key_zero() {
        let one = bake_track(vec![0.5], vec![[1.0, 2.0, 3.0]]);
        let mut hint = 0;
        let got = sample_track(&one, f32::NAN, &mut hint);
        assert_eq!(got, sample_track(&one, 0.0, &mut 0));
        assert_eq!(hint, 0);
        for i in 0..3 {
            assert!((got[i] - [1.0, 2.0, 3.0][i]).abs() < 2e-3, "{got:?}");
        }
        let c = lin();
        let mut h = 0;
        assert_eq!(
            sample_track(&c.tracks[0], f32::NAN, &mut h),
            sample_track(&c.tracks[0], 0.0, &mut 0)
        );
    }
    #[test]
    fn quant_err_mid() {
        let c = lin();
        let mut h = 0;
        let m = sample_track(&c.tracks[0], 0.5, &mut h);
        for i in 0..3 {
            assert!((m[i] - [1.0, 2.0, 3.0][i]).abs() < 2e-3, "{m:?}");
        }
    }
    #[test]
    fn world_slip_reads_the_body_travelling_over_a_planted_foot() {
        // The AM24 residual: a foot slides when the whole body travels over
        // a planted foot, so the meter wants the foot's world position. The
        // root track carries a 2 m/s travel, the foot's own track never
        // moves: avg_slip reads 0, avg_world_slip reads the 2 m/s.
        let skeleton = crate::contract::Skeleton {
            joints: vec![
                crate::contract::Joint {
                    name: "root".to_string(),
                    parent: None,
                },
                crate::contract::Joint {
                    name: "foot".to_string(),
                    parent: Some(0),
                },
            ],
        };
        let c = bake_clip(vec![
            bake_track(vec![0.0, 1.0], vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0]]),
            bake_track(vec![0.0, 1.0], vec![[1.0, 0.0, 0.0], [1.0, 0.0, 0.0]]),
        ]);
        assert_eq!(avg_slip(&c, 1, 60), 0.0, "the foot's own track never moves");
        let world = avg_world_slip(&c, &[], &skeleton, 1, 60);
        assert!(
            (world - 2.0).abs() < 0.05,
            "world slip reads the 2 m/s travel, got {world}"
        );
        assert_eq!(
            avg_world_slip(&c, &[], &skeleton, 9, 60),
            0.0,
            "a bad foot reads zero"
        );
        assert_eq!(
            avg_world_slip(&c, &[], &skeleton, 1, 1),
            0.0,
            "a one-tick window reads zero"
        );
    }
    #[test]
    fn cursor_eq_seek() {
        let c = lin();
        let mut fwd = Cursor::new(2);
        let mut o1 = [[0.0; 3]; 2];
        let mut o2 = [[0.0; 3]; 2];
        for k in 0..60 {
            let t = (k as f32) * TICK_DT;
            sample_all(&c, t, &mut fwd, &mut o1);
            let mut seek = Cursor::new(2);
            seek.pos[0] = usize::MAX / 2;
            seek.pos[1] = usize::MAX / 2;
            sample_all(&c, t, &mut seek, &mut o2);
            assert_eq!(o1, o2, "k={k} t={t}");
        }
    }
    #[test]
    fn traj_hash_x2() {
        let c = lin();
        assert_eq!(traj_hash(&c, 0, 600), traj_hash(&c, 0, 600));
        assert_eq!(qclip_hash(&c), qclip_hash(&c.clone()));
        let mut d = lin();
        d.tracks[0].keys[0][0] ^= 1;
        assert_ne!(traj_hash(&lin(), 0, 120), traj_hash(&d, 0, 120));
    }
    #[test]
    fn slip_locked_under_8cm() {
        let c = lin();
        assert!(avg_slip(&c, 1, 600) < 0.08, "slip={}", avg_slip(&c, 1, 600));
        assert!(slip([0.0, 0.0, 0.0], [0.01, 0.0, 0.0], TICK_DT) < 0.8);
    }
}
