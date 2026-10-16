//! trace: geometric room acoustics beside the frozen mix.
//!
//! Owner: audio lane; consumer: [`crate::device::record_audio`] offline capture.
//! Units: metres, seconds, Hz for cutoffs; gains linear 0..1. Determinism:
//! fixed Fibonacci rays plus a fixed hash scatter, so the same scene gives
//! the same numbers on one machine (presentation only, never hashed).
//! Errors: none (inputs clamp); budget: offline only, never the tick.
//! PORT(AizenKai1001/lunaris@56cc7e30:crates/lunaris-audio/src/trace.rs).

/// Point or direction in metres.
pub type V3 = [f32; 3];
/// Speed of sound in air, metres per second.
pub const SPEED_OF_SOUND: f32 = 343.0;
/// A ray missing within this range left the scene through an opening.
pub const OPEN_DISTANCE: f32 = 60.0;
/// Wall thickness where `transmission` applies, in metres.
pub const REFERENCE_THICKNESS: f32 = 0.2;
/// Echo taps kept per voice.
pub const MAX_TAPS: usize = 16;
/// Longest echo kept, in seconds.
pub const MAX_ECHO: f32 = 0.26;
/// BVH leaf size.
const LEAF_SIZE: usize = 4;
/// Echo merge edges in milliseconds (donor TAP_EDGES_MS shape).
const TAP_EDGES_MS: [f32; 12] = [
    0.0, 5.0, 10.0, 15.0, 20.0, 30.0, 40.0, 60.0, 80.0, 120.0, 180.0, 250.0,
];

/// Closed shape an occluder is made of.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// Axis-aligned box.
    Box { center: V3, half: V3 },
    /// Sphere.
    Sphere { center: V3, radius: f32 },
}

/// One occluder: shape plus surface shares.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Occluder {
    /// Geometry.
    pub shape: Shape,
    /// Share of energy a reflection loses, 0..1.
    pub absorption: f32,
    /// Amplitude through a [`REFERENCE_THICKNESS`] wall, 0..1.
    pub transmission: f32,
}

/// Nearest-surface answer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// Index into the scene build list.
    pub index: usize,
    /// Distance along the ray in metres.
    pub t: f32,
    /// Unit normal on the side the ray came from.
    pub normal: V3,
}

struct Node {
    min: V3,
    max: V3,
    left: usize,
    count: usize,
}

/// Ray-traceable occluder set over a small BVH.
pub struct Scene {
    occluders: Vec<Occluder>,
    order: Vec<usize>,
    nodes: Vec<Node>,
}

fn dir_dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn shape_bounds(shape: &Shape) -> (V3, V3) {
    match *shape {
        Shape::Box { center, half } => (
            [
                center[0] - half[0],
                center[1] - half[1],
                center[2] - half[2],
            ],
            [
                center[0] + half[0],
                center[1] + half[1],
                center[2] + half[2],
            ],
        ),
        Shape::Sphere { center, radius } => (
            [center[0] - radius, center[1] - radius, center[2] - radius],
            [center[0] + radius, center[1] + radius, center[2] + radius],
        ),
    }
}

fn shape_interval(shape: &Shape, o: V3, d: V3) -> Option<(f32, f32, V3)> {
    match *shape {
        Shape::Box { center, half } => {
            let mut t0 = f32::NEG_INFINITY;
            let mut t1 = f32::INFINITY;
            let mut n0: V3 = [0.0; 3];
            for a in 0..3 {
                let rel = o[a] - center[a];
                if d[a].abs() < 1e-9 {
                    if rel.abs() > half[a] {
                        return None;
                    }
                    continue;
                }
                let mut ta = (-half[a] - rel) / d[a];
                let mut tb = (half[a] - rel) / d[a];
                let mut na: V3 = [0.0; 3];
                na[a] = -1.0;
                if ta > tb {
                    std::mem::swap(&mut ta, &mut tb);
                    na[a] = 1.0;
                }
                if ta > t0 {
                    t0 = ta;
                    n0 = na;
                }
                t1 = t1.min(tb);
                if t0 > t1 {
                    return None;
                }
            }
            if t1 < 0.0 || !t0.is_finite() {
                return None;
            }
            Some((t0, t1, n0))
        }
        Shape::Sphere { center, radius } => {
            let m = [o[0] - center[0], o[1] - center[1], o[2] - center[2]];
            let b = dir_dot(m, d);
            let c = dir_dot(m, m) - radius * radius;
            let disc = b * b - c;
            if disc < 0.0 {
                return None;
            }
            let s = disc.sqrt();
            let (t0, t1) = (-b - s, -b + s);
            if t1 < 0.0 {
                return None;
            }
            let l = (dir_dot(m, m) + t0 * (2.0 * b + t0)).max(0.0).sqrt();
            let n = if l > 1e-12 {
                [
                    (m[0] + d[0] * t0) / l,
                    (m[1] + d[1] * t0) / l,
                    (m[2] + d[2] * t0) / l,
                ]
            } else {
                [0.0, 1.0, 0.0]
            };
            Some((t0, t1, n))
        }
    }
}

/// Evenly spread direction over the sphere; same on every machine.
pub fn fibonacci_direction(i: usize, n: usize) -> V3 {
    let fi = crate::f32_of(u32::try_from(i).unwrap_or(0));
    let count = crate::f32_of(u32::try_from(n).unwrap_or(1)).max(1.0);
    let y = 1.0 - 2.0 * (fi + 0.5) / count;
    let r = (1.0 - y * y).max(0.0).sqrt();
    let phi = fi * 2.399_963_2;
    [r * phi.cos(), y, r * phi.sin()]
}

fn hash(mut x: u64) -> u64 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    x ^ (x >> 33)
}

fn unit_random(seed: u64) -> f32 {
    u32::try_from(hash(seed) >> 40).map_or(0.0, crate::f32_of) / 16_777_216.0
}

fn cosine_direction(n: V3, seed: u64) -> V3 {
    let (u1, u2) = (unit_random(seed), unit_random(seed ^ 0x9e37_79b9));
    let r = u1.sqrt();
    let phi = core::f32::consts::TAU * u2;
    let (x, y, z) = (r * phi.cos(), r * phi.sin(), (1.0 - u1).max(0.0).sqrt());
    let helper: V3 = if n[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let t = [
        helper[1] * n[2] - helper[2] * n[1],
        helper[2] * n[0] - helper[0] * n[2],
        helper[0] * n[1] - helper[1] * n[0],
    ];
    let tl = dir_dot(t, t).sqrt().max(1e-12);
    let tu = [t[0] / tl, t[1] / tl, t[2] / tl];
    let b = [
        n[1] * tu[2] - n[2] * tu[1],
        n[2] * tu[0] - n[0] * tu[2],
        n[0] * tu[1] - n[1] * tu[0],
    ];
    let v = [
        tu[0] * x + b[0] * y + n[0] * z,
        tu[1] * x + b[1] * y + n[1] * z,
        tu[2] * x + b[2] * y + n[2] * z,
    ];
    let l = dir_dot(v, v).sqrt().max(1e-12);
    [v[0] / l, v[1] / l, v[2] / l]
}

impl Scene {
    /// Build the BVH over `occluders`.
    pub fn new(occluders: Vec<Occluder>) -> Self {
        let mut scene = Self {
            order: (0..occluders.len()).collect(),
            nodes: Vec::new(),
            occluders,
        };
        if !scene.occluders.is_empty() {
            let n = scene.occluders.len();
            scene.nodes.push(Node {
                min: [0.0; 3],
                max: [0.0; 3],
                left: 0,
                count: 0,
            });
            scene.build_into(0, 0, n);
        }
        scene
    }

    /// Occluder count.
    pub fn len(&self) -> usize {
        self.occluders.len()
    }

    /// True when there is nothing to hit.
    pub fn is_empty(&self) -> bool {
        self.occluders.is_empty()
    }

    fn range_bounds(&self, from: usize, to: usize) -> (V3, V3) {
        let mut lo: V3 = [f32::INFINITY; 3];
        let mut hi: V3 = [f32::NEG_INFINITY; 3];
        for &i in &self.order[from..to] {
            let (a, b) = shape_bounds(&self.occluders[i].shape);
            for k in 0..3 {
                lo[k] = lo[k].min(a[k]);
                hi[k] = hi[k].max(b[k]);
            }
        }
        (lo, hi)
    }

    fn build_into(&mut self, slot: usize, from: usize, to: usize) {
        let (min, max) = self.range_bounds(from, to);
        if to - from <= LEAF_SIZE {
            self.nodes[slot] = Node {
                min,
                max,
                left: from,
                count: to - from,
            };
            return;
        }
        let mut clo: V3 = [f32::INFINITY; 3];
        let mut chi: V3 = [f32::NEG_INFINITY; 3];
        for &i in &self.order[from..to] {
            let (a, b) = shape_bounds(&self.occluders[i].shape);
            for k in 0..3 {
                let c = 0.5 * (a[k] + b[k]);
                clo[k] = clo[k].min(c);
                chi[k] = chi[k].max(c);
            }
        }
        let mut axis = 0;
        let mut span = chi[0] - clo[0];
        for k in 1..3 {
            if chi[k] - clo[k] > span {
                span = chi[k] - clo[k];
                axis = k;
            }
        }
        let occ = &self.occluders;
        self.order[from..to].sort_by(|&a, &b| {
            let (aa, ab) = shape_bounds(&occ[a].shape);
            let (ba, bb) = shape_bounds(&occ[b].shape);
            (aa[axis] + ab[axis])
                .total_cmp(&(ba[axis] + bb[axis]))
                .then_with(|| a.cmp(&b))
        });
        let mid = from + (to - from) / 2;
        let left = self.nodes.len();
        self.nodes.push(Node {
            min,
            max,
            left: 0,
            count: 0,
        });
        self.nodes.push(Node {
            min,
            max,
            left: 0,
            count: 0,
        });
        self.build_into(left, from, mid);
        self.build_into(left + 1, mid, to);
        self.nodes[slot] = Node {
            min,
            max,
            left,
            count: 0,
        };
    }

    fn hit_box(min: V3, max: V3, o: V3, inv: V3, far: f32) -> Option<f32> {
        let mut t0 = 0.0f32;
        let mut t1 = far;
        for a in 0..3 {
            let mut ta = (min[a] - o[a]) * inv[a];
            let mut tb = (max[a] - o[a]) * inv[a];
            if ta > tb {
                std::mem::swap(&mut ta, &mut tb);
            }
            t0 = t0.max(ta);
            t1 = t1.min(tb);
            if t0 > t1 {
                return None;
            }
        }
        Some(t0)
    }

    /// Nearest surface within `far` metres, if any.
    pub fn nearest(&self, o: V3, d: V3, far: f32) -> Option<Hit> {
        if self.nodes.is_empty() {
            return None;
        }
        let inv: V3 = [
            1.0 / if d[0].abs() < 1e-20 { 1e-20 } else { d[0] },
            1.0 / if d[1].abs() < 1e-20 { 1e-20 } else { d[1] },
            1.0 / if d[2].abs() < 1e-20 { 1e-20 } else { d[2] },
        ];
        let mut best: Option<Hit> = None;
        let mut stack = [0usize; 96];
        let mut sp = 1usize;
        stack[0] = 0;
        while sp > 0 {
            sp -= 1;
            let ni = stack[sp];
            let node = &self.nodes[ni];
            let limit = best.map_or(far, |h: Hit| h.t);
            if Self::hit_box(node.min, node.max, o, inv, limit).is_none() {
                continue;
            }
            if node.count > 0 {
                for &oi in &self.order[node.left..node.left + node.count] {
                    if let Some((t0, _, n)) = shape_interval(&self.occluders[oi].shape, o, d) {
                        if t0 > 1e-4 && t0 <= limit && best.is_none_or(|h: Hit| t0 < h.t) {
                            best = Some(Hit {
                                index: oi,
                                t: t0,
                                normal: n,
                            });
                        }
                    }
                }
            } else if sp + 2 <= stack.len() {
                stack[sp] = node.left;
                stack[sp + 1] = node.left + 1;
                sp += 2;
            }
        }
        best
    }

    /// Amplitude surviving the segment `a` to `b` through walls.
    pub fn transmittance(&self, a: V3, b: V3) -> f32 {
        let seg = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let len = dir_dot(seg, seg).sqrt();
        if len < 1e-5 {
            return 1.0;
        }
        let d = [seg[0] / len, seg[1] / len, seg[2] / len];
        let mut t = 1.0f32;
        for occ in &self.occluders {
            if let Some((t0, t1, _)) = shape_interval(&occ.shape, a, d) {
                let (enter, leave) = (t0.max(0.0), t1.min(len));
                if leave - enter >= 0.0 && enter < len - 0.02 && leave > 0.02 && t1 >= t0 {
                    let chord = (leave - enter).max(REFERENCE_THICKNESS);
                    t *= occ
                        .transmission
                        .clamp(0.0, 1.0)
                        .powf(chord / REFERENCE_THICKNESS);
                }
            }
        }
        t
    }
}

/// What the room does to sound around the listener.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Room {
    /// Seconds to fall 60 dB, 0 in the open.
    pub rt60: f32,
    /// Share of rays hitting within [`OPEN_DISTANCE`], 1 is enclosed.
    pub closedness: f32,
}

/// Measure the room with fixed rays plus mirror and matte bounces.
pub fn analyse_room(scene: &Scene, listener: V3, rays: usize, bounces: usize) -> Room {
    if scene.is_empty() || rays == 0 || bounces == 0 {
        return Room {
            rt60: 0.0,
            closedness: 0.0,
        };
    }
    let mut events: Vec<(f32, f32)> = Vec::new();
    let mut closed = 0usize;
    for i in 0..rays {
        let mut d = fibonacci_direction(i, rays);
        let mut o = listener;
        let mut e = 1.0f32;
        let mut travelled = 0.0f32;
        for b in 0..bounces {
            let Some(h) = scene.nearest(o, d, OPEN_DISTANCE) else {
                break;
            };
            travelled += h.t;
            if b == 0 {
                closed += 1;
            }
            events.push((travelled / SPEED_OF_SOUND, e));
            let occ = &scene.occluders[h.index];
            e *= 1.0 - occ.absorption.clamp(0.0, 1.0);
            if e < 0.02 {
                break;
            }
            let p = [o[0] + d[0] * h.t, o[1] + d[1] * h.t, o[2] + d[2] * h.t];
            let n = if dir_dot(h.normal, d) > 0.0 {
                [-h.normal[0], -h.normal[1], -h.normal[2]]
            } else {
                h.normal
            };
            o = [p[0] + n[0] * 0.01, p[1] + n[1] * 0.01, p[2] + n[2] * 0.01];
            let roll = hash(u64::try_from(i).unwrap_or(0) << 8 | u64::try_from(b).unwrap_or(0));
            d = if roll.is_multiple_of(3) {
                cosine_direction(n, roll)
            } else {
                let k = 2.0 * dir_dot(d, n);
                let v = [d[0] - n[0] * k, d[1] - n[1] * k, d[2] - n[2] * k];
                let l = dir_dot(v, v).sqrt().max(1e-12);
                [v[0] / l, v[1] / l, v[2] / l]
            };
        }
    }
    let rays_f = crate::f32_of(u32::try_from(rays).unwrap_or(1)).max(1.0);
    let closed_f = crate::f32_of(u32::try_from(closed).unwrap_or(0));
    let mut room = Room {
        rt60: 0.0,
        closedness: (closed_f / rays_f).clamp(0.0, 1.0),
    };
    if events.is_empty() {
        return room;
    }
    events.sort_by(|a, b| a.0.total_cmp(&b.0));
    let total: f32 = events.iter().map(|(_, e)| *e).sum();
    if total <= 0.0 {
        return room;
    }
    let at = |level: f32| {
        let mut tail = total;
        let mut prev = (0.0f32, total);
        for &(t, e) in &events {
            tail -= e;
            let db = 10.0 * (tail / total).max(1e-12).log10();
            if db <= level {
                let a = 10.0 * (prev.1 / total).max(1e-12).log10();
                let w = if a > db {
                    (a - level) / (a - db).max(1e-9)
                } else {
                    0.0
                };
                return Some(prev.0 + (t - prev.0) * w);
            }
            prev = (t, tail);
        }
        None
    };
    if let (Some(a), Some(b)) = (at(-1.0), at(-5.0)) {
        room.rt60 = ((b - a).max(0.0) * 60.0 / 4.0).clamp(0.0, 6.0);
    }
    room
}

/// One echo tap: delay after direct, gain at one metre, arrival dir.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tap {
    /// Seconds after the direct sound.
    pub delay: f32,
    /// Linear gain, 1 is as loud as direct at one metre.
    pub gain: f32,
    /// Unit arrival direction from the listener.
    pub dir: V3,
}

/// What the path does to one voice (donor mixer Acoustics shape).
#[derive(Clone, Debug, PartialEq)]
pub struct Acoustics {
    /// Direct amplitude surviving, 0..1.
    pub direct_gain: f32,
    /// Direct low-pass in Hz, 20000 is none.
    pub direct_cutoff: f32,
    /// Share sent to the room tail, 0..1.
    pub send: f32,
    /// Merged first echoes, at most [`MAX_TAPS`].
    pub taps: Vec<Tap>,
}

/// Room tail settings for the reverberator (donor mixer RoomSettings shape).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoomSettings {
    /// Tail length in seconds.
    pub rt60: f32,
    /// High-frequency loss, 0..1.
    pub damping: f32,
    /// Tail level beside direct at one metre.
    pub wet: f32,
    /// Tail delay in seconds.
    pub predelay: f32,
}

/// Trace one source to the listener: direct plus merged first echoes.
pub fn trace_source(
    scene: &Scene,
    listener: V3,
    source: V3,
    rays: usize,
    radius: f32,
) -> Acoustics {
    let to = [
        source[0] - listener[0],
        source[1] - listener[1],
        source[2] - listener[2],
    ];
    let distance = dir_dot(to, to).sqrt();
    let r = radius.max(0.0);
    let aim: [V3; 7] = [
        source,
        [source[0] + r, source[1], source[2]],
        [source[0] - r, source[1], source[2]],
        [source[0], source[1] + r, source[2]],
        [source[0], source[1] - r, source[2]],
        [source[0], source[1], source[2] + r],
        [source[0], source[1], source[2] - r],
    ];
    let count = if r > 0.0 { 7 } else { 1 };
    let mut sum = 0.0f32;
    for target in aim.iter().take(count) {
        sum += scene.transmittance(listener, *target);
    }
    let direct = sum / crate::f32_of(u32::try_from(count).unwrap_or(1)).max(1.0);
    let mut out = Acoustics {
        direct_gain: direct.clamp(0.0, 1.0),
        direct_cutoff: 20_000.0,
        send: 0.5,
        taps: Vec::new(),
    };
    if scene.is_empty() || rays == 0 {
        return out;
    }
    let mut buckets = [(0.0f32, 0.0f32, [0.0f32; 3]); 12];
    let direct_delay = distance / SPEED_OF_SOUND;
    let rays_f = crate::f32_of(u32::try_from(rays).unwrap_or(1)).max(1.0);
    for i in 0..rays {
        let first = fibonacci_direction(i, rays);
        let mut d = first;
        let mut o = listener;
        let (mut len, mut rho) = (0.0f32, 1.0f32);
        for _ in 0..2 {
            let Some(h) = scene.nearest(o, d, OPEN_DISTANCE) else {
                break;
            };
            len += h.t;
            let p = [o[0] + d[0] * h.t, o[1] + d[1] * h.t, o[2] + d[2] * h.t];
            let n = if dir_dot(h.normal, d) > 0.0 {
                [-h.normal[0], -h.normal[1], -h.normal[2]]
            } else {
                h.normal
            };
            rho *= 1.0 - scene.occluders[h.index].absorption.clamp(0.0, 1.0);
            let q = [p[0] + n[0] * 0.02, p[1] + n[1] * 0.02, p[2] + n[2] * 0.02];
            let ts = [source[0] - q[0], source[1] - q[1], source[2] - q[2]];
            let ds = dir_dot(ts, ts).sqrt();
            if ds > 1e-3 {
                let cos_s = (ts[0] * n[0] + ts[1] * n[1] + ts[2] * n[2]) / ds;
                if cos_s > 0.02 {
                    let t = scene.transmittance(q, source);
                    if t > 0.01 {
                        let gain = 2.0 * (rho * cos_s / rays_f).sqrt() / ds.max(0.5) * t;
                        let delay = ((len + ds) / SPEED_OF_SOUND - direct_delay).max(0.001);
                        if delay <= MAX_ECHO {
                            let ms = delay * 1000.0;
                            let mut k = 0;
                            for (e, edge) in TAP_EDGES_MS.iter().enumerate() {
                                if ms >= *edge {
                                    k = e;
                                }
                            }
                            let pw = gain * gain;
                            buckets[k].0 += pw;
                            buckets[k].1 += pw * delay;
                            buckets[k].2 = [
                                buckets[k].2[0] + first[0] * pw,
                                buckets[k].2[1] + first[1] * pw,
                                buckets[k].2[2] + first[2] * pw,
                            ];
                        }
                    }
                }
            }
            let k = 2.0 * dir_dot(d, n);
            let v = [d[0] - n[0] * k, d[1] - n[1] * k, d[2] - n[2] * k];
            let l = dir_dot(v, v).sqrt().max(1e-12);
            d = [v[0] / l, v[1] / l, v[2] / l];
            o = q;
        }
    }
    for (power, delay_sum, dir_sum) in buckets {
        if power > 1e-12 {
            let inv = 1.0 / power;
            out.taps.push(Tap {
                delay: delay_sum * inv,
                gain: power.sqrt(),
                dir: [dir_sum[0] * inv, dir_sum[1] * inv, dir_sum[2] * inv],
            });
        }
    }
    if out.taps.len() > MAX_TAPS {
        out.taps.truncate(MAX_TAPS);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stone_room() -> Vec<Occluder> {
        let (hw, hh, hl, t) = (4.0f32, 2.5, 2.0, 0.1);
        let wall = |c: V3, h: V3| Occluder {
            shape: Shape::Box { center: c, half: h },
            absorption: 0.1,
            transmission: 0.0,
        };
        vec![
            wall([0.0, -hh - t, 0.0], [hw + 0.2, t, hl + 0.2]),
            wall([0.0, hh + t, 0.0], [hw + 0.2, t, hl + 0.2]),
            wall([-hw - t, 0.0, 0.0], [t, hh, hl]),
            wall([hw + t, 0.0, 0.0], [t, hh, hl]),
            wall([0.0, 0.0, -hl - t], [hw, hh, t]),
            wall([0.0, 0.0, hl + t], [hw, hh, t]),
        ]
    }

    #[test]
    fn acoustic_room_rt60_within_10pct() {
        let scene = Scene::new(stone_room());
        let room = analyse_room(&scene, [0.0, 0.0, 0.0], 128, 32);
        let volume = 8.0 * 5.0 * 4.0;
        let area = 2.0 * (8.0 * 5.0 + 8.0 * 4.0 + 5.0 * 4.0);
        let target = 0.1611 * volume / (-area * (1.0 - 0.1f32).ln());
        let err = (room.rt60 - target).abs() / target;
        assert!(err <= 0.10, "rt60={} target={target}", room.rt60);
        assert!(room.closedness > 0.9, "closed={}", room.closedness);
        let ac = trace_source(&scene, [0.0, 0.0, 0.0], [2.0, 0.0, 0.0], 64, 0.1);
        assert!(ac.direct_gain > 0.9, "direct={}", ac.direct_gain);
        assert!(ac.taps.len() <= MAX_TAPS, "taps={}", ac.taps.len());
        assert!(ac.taps.iter().all(|t| t.delay <= MAX_ECHO), "echo overlong");
        let open = analyse_room(&Scene::new(Vec::new()), [0.0, 0.0, 0.0], 32, 8);
        assert_eq!(open.rt60, 0.0, "open air does not ring");
    }

    #[test]
    fn slab_swap_keeps_entry_before_exit() {
        let shape = Shape::Box {
            center: [0.0, 0.0, 0.0],
            half: [1.0, 1.0, 1.0],
        };
        for (o, d) in [
            ([-5.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
            ([5.0, 0.0, 0.0], [-1.0, 0.0, 0.0]),
            ([0.0, -5.0, 0.0], [0.0, 1.0, 0.0]),
        ] {
            let Some((t0, t1, _)) = shape_interval(&shape, o, d) else {
                panic!("ray from {o:?} must enter the box");
            };
            assert!(t0 <= t1, "entry {t0} after exit {t1}");
            assert!((t0 - 4.0).abs() < 1e-4, "entry {t0} != 4");
            assert!((t1 - 6.0).abs() < 1e-4, "exit {t1} != 6");
        }
        assert!(
            shape_interval(&shape, [0.0, 0.0, 5.0], [0.0, 0.0, 1.0]).is_none(),
            "ray leaving the box must miss"
        );
    }

    #[test]
    fn hash_branch_parity_mod3_vs_is_multiple_of() {
        assert!(
            (0u64..4096).all(|roll| (roll % 3 == 0) == roll.is_multiple_of(3)),
            "mod-3 branch and is_multiple_of must agree on every hash slice"
        );
    }

    #[test]
    fn aim_sampling_uses_one_target_for_point_source() {
        let scene = Scene::new(Vec::new());
        for radius in [0.0f32, 0.5] {
            let ac = trace_source(&scene, [0.0, 0.0, 0.0], [2.0, 0.0, 0.0], 8, radius);
            assert_eq!(
                ac.direct_gain, 1.0,
                "empty scene transmits fully (radius={radius})"
            );
        }
    }

    #[test]
    fn trace_uses_swap_not_manual_tmp() {
        assert!(
            !include_str!("trace.rs").contains("let tmp = ta;"),
            "slab swap must use std::mem::swap so clippy -D warnings stays green"
        );
    }

    #[test]
    fn trace_uses_is_multiple_of_not_mod() {
        assert!(
            !include_str!("trace.rs").contains("roll % 3"),
            "hash branch must use is_multiple_of so clippy -D warnings stays green"
        );
    }

    #[test]
    fn trace_iterates_aim_not_index_loop() {
        assert!(
            !include_str!("trace.rs").contains("for k in 0..count"),
            "aim sampling must iterate the array so clippy -D warnings stays green"
        );
    }
}
