//! particles: seeded CPU mirror of GPU FX at 60Hz. Fixed emitter order, hashed.
use crate::particles_gpu::throttle_spawn;
pub const FIXED_DT: f32 = 1.0 / 60.0;
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        // splitmix init (NOT seed|1: consecutive seeds must diverge —
        // 2040|1 == 2041|1 collapsed two streams into one).
        let mut z = seed.wrapping_add(0x9e3779b97f4a7c15);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        Self(z ^ (z >> 31) | 1)
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 32) as u32 as f32 / 4_294_967_300.0
    }
}
/// 1D keyframed curve over age01 in [0,1]. Linear interp, clamped ends.
#[derive(Clone, Debug, Default)]
pub struct Curve {
    pub keys: Vec<(f32, f32)>,
}
impl Curve {
    pub fn new(keys: Vec<(f32, f32)>) -> Self {
        Self { keys }
    }
    pub fn constant(v: f32) -> Self {
        Self {
            keys: vec![(0.0, v), (1.0, v)],
        }
    }
    pub fn fade() -> Self {
        Self {
            keys: vec![(0.0, 1.0), (0.7, 1.0), (1.0, 0.0)],
        }
    }
    pub fn grow() -> Self {
        Self {
            keys: vec![(0.0, 0.4), (0.3, 1.0), (1.0, 1.6)],
        }
    }
    pub fn sample(&self, t: f32) -> f32 {
        if self.keys.is_empty() {
            return 1.0;
        }
        if self.keys.len() == 1 {
            return self.keys[0].1;
        }
        let x = t.clamp(0.0, 1.0);
        if x <= self.keys[0].0 {
            return self.keys[0].1;
        }
        for w in self.keys.windows(2) {
            let (t0, v0) = w[0];
            let (t1, v1) = w[1];
            if x <= t1 {
                let d = t1 - t0;
                if d <= 1e-6 {
                    return v1;
                }
                let a = (x - t0) / d;
                return v0 + (v1 - v0) * a;
            }
        }
        self.keys.last().unwrap().1
    }
}
/// Flipbook atlas: cols x rows frames, Hanabi UV-index / Wicked atlas anim.
#[derive(Clone, Copy, Debug)]
pub struct Flipbook {
    pub cols: u8,
    pub rows: u8,
}
impl Flipbook {
    pub fn frames(self) -> usize {
        (self.cols.max(1) as usize) * (self.rows.max(1) as usize)
    }
    pub fn index(self, age01: f32, looping: bool) -> u16 {
        let f = self.frames();
        let mut i = (age01.clamp(0.0, 1.0) * f as f32) as usize;
        if looping {
            i %= f;
        } else if i >= f {
            i = f - 1;
        }
        i as u16
    }
}
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
/// Emitter node: spawner (rate+burst) + init (cone/life/size) + update params
/// (gravity/drag/curves/color/flipbook). All plain data, seeded at runtime.
#[derive(Clone, Debug)]
pub struct EmitterDesc {
    pub origin: [f32; 3],
    pub extent: [f32; 3],
    pub dir: [f32; 3],
    pub spread: f32,
    pub rate: f32,
    pub burst: u32,
    pub life: [f32; 2],
    pub speed: [f32; 2],
    pub gravity: f32,
    pub drag: f32,
    pub size: [f32; 2],
    pub size_over_life: Curve,
    pub alpha_over_life: Curve,
    pub c0: [f32; 4],
    pub c1: [f32; 4],
    pub atlas: [u8; 2],
    pub loop_flip: bool,
}
impl Default for EmitterDesc {
    fn default() -> Self {
        Self {
            origin: [0.0, 0.0, 0.0],
            extent: [0.5, 0.1, 0.5],
            dir: [0.0, 1.0, 0.0],
            spread: 0.35,
            rate: 240.0,
            burst: 32,
            life: [0.6, 1.4],
            speed: [2.0, 6.0],
            gravity: 4.5,
            drag: 0.6,
            size: [0.08, 0.22],
            size_over_life: Curve::grow(),
            alpha_over_life: Curve::fade(),
            c0: [1.0, 0.8, 0.4, 1.0],
            c1: [0.9, 0.2, 0.1, 0.0],
            atlas: [4, 4],
            loop_flip: false,
        }
    }
}
impl EmitterDesc {
    pub fn sparks() -> Self {
        Self {
            origin: [0.0, 0.5, 0.0],
            extent: [0.1, 0.1, 0.1],
            dir: [0.0, 1.0, 0.0],
            spread: 1.0,
            rate: 120.0,
            burst: 16,
            life: [0.4, 1.0],
            speed: [3.0, 9.0],
            gravity: 9.0,
            drag: 0.2,
            size: [0.03, 0.08],
            size_over_life: Curve::constant(1.0),
            alpha_over_life: Curve::fade(),
            c0: [1.0, 0.95, 0.7, 1.0],
            c1: [1.0, 0.4, 0.1, 0.0],
            atlas: [8, 2],
            loop_flip: true,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Particle {
    pub pos: [f32; 3],
    pub vel: [f32; 3],
    pub age: f32,
    pub life: f32,
    pub size0: f32,
    pub size: f32,
    pub alpha: f32,
    pub col: [f32; 4],
    pub flip: u16,
    pub emit: u8,
}
/// Effect graph: N emitters sharing one pool. update() evaluates nodes in fixed
/// order: 1 spawner 2 init(cone) 3 force+drag 4 integrate 5 curves/color 6 flipbook 7 age/cull.
pub struct ParticleEffect {
    descs: Vec<EmitterDesc>,
    parts: Vec<Particle>,
    rng: Rng,
    accs: Vec<f32>,
    time: f32,
    tick: u64,
    spawned: u64,
    max: usize,
}
impl ParticleEffect {
    pub fn new(seed: u64, descs: Vec<EmitterDesc>, max_particles: usize) -> Self {
        let n = descs.len().max(1);
        Self {
            descs,
            parts: Vec::new(),
            rng: Rng::new(seed),
            accs: vec![0.0; n],
            time: 0.0,
            tick: 0,
            spawned: 0,
            max: max_particles.clamp(1, 65536),
        }
    }
    pub fn demo(seed: u64) -> Self {
        Self::new(
            seed,
            vec![EmitterDesc::default(), EmitterDesc::sparks()],
            4096,
        )
    }
    pub fn tick(&mut self) {
        self.update(FIXED_DT);
    }
    pub fn alive(&self) -> usize {
        self.parts.len()
    }
    pub fn spawned_total(&self) -> u64 {
        self.spawned
    }
    pub fn time(&self) -> f32 {
        self.time
    }
    pub fn tick_count(&self) -> u64 {
        self.tick
    }
    pub fn particles(&self) -> &[Particle] {
        &self.parts
    }
    pub fn update(&mut self, dt: f32) {
        if self.descs.is_empty() {
            self.time += dt.max(0.0);
            self.tick += 1;
            return;
        }
        let dt = dt.min(0.1);
        if dt <= 0.0 {
            return;
        }
        for ei in 0..self.descs.len() {
            let (burst, rate) = {
                let d = &self.descs[ei];
                (if self.tick == 0 { d.burst } else { 0 }, d.rate.max(0.0))
            };
            for _ in 0..burst {
                self.spawn(ei as u8);
            }
            let n = throttle_spawn(&mut self.accs[ei], rate, dt);
            for _ in 0..n {
                self.spawn(ei as u8);
            }
        }
        for i in 0..self.parts.len() {
            let ei = (self.parts[i].emit as usize).min(self.descs.len() - 1);
            let (g, dr) = (self.descs[ei].gravity, self.descs[ei].drag);
            {
                let p = &mut self.parts[i];
                p.age += dt;
                p.vel[1] -= g * dt;
                let k = 1.0 / (1.0 + dr * dt);
                p.vel[0] *= k;
                p.vel[1] *= k;
                p.vel[2] *= k;
                p.pos[0] += p.vel[0] * dt;
                p.pos[1] += p.vel[1] * dt;
                p.pos[2] += p.vel[2] * dt;
            }
            let a01 = (self.parts[i].age / self.parts[i].life).clamp(0.0, 1.0);
            let d = &self.descs[ei];
            let (sm, am, c0, c1, atl, lp, s0) = (
                d.size_over_life.sample(a01),
                d.alpha_over_life.sample(a01),
                d.c0,
                d.c1,
                d.atlas,
                d.loop_flip,
                self.parts[i].size0,
            );
            let p = &mut self.parts[i];
            p.size = s0 * sm;
            let la = lerp(c0[3], c1[3], a01) * am;
            p.alpha = la;
            p.col = [
                lerp(c0[0], c1[0], a01),
                lerp(c0[1], c1[1], a01),
                lerp(c0[2], c1[2], a01),
                la,
            ];
            p.flip = Flipbook {
                cols: atl[0],
                rows: atl[1],
            }
            .index(a01, lp);
        }
        self.parts.retain(|p| p.age < p.life);
        self.time += dt;
        self.tick += 1;
    }
    fn spawn(&mut self, emit: u8) {
        if self.parts.len() >= self.max {
            return;
        }
        let di = emit as usize;
        if di >= self.descs.len() {
            return;
        }
        let (og, ex, dir, sp, lf, sd, sz) = {
            let d = &self.descs[di];
            (d.origin, d.extent, d.dir, d.spread, d.life, d.speed, d.size)
        };
        let (jx, jy, jz) = (
            self.rng.next_f32() * 2.0 - 1.0,
            self.rng.next_f32() * 2.0 - 1.0,
            self.rng.next_f32() * 2.0 - 1.0,
        );
        let (dx, dy, dz) = (
            self.rng.next_f32() * 2.0 - 1.0,
            self.rng.next_f32() * 2.0 - 1.0,
            self.rng.next_f32() * 2.0 - 1.0,
        );
        let (us, ul, uz) = (
            self.rng.next_f32(),
            self.rng.next_f32(),
            self.rng.next_f32(),
        );
        let mut vx = dir[0] + dx * sp;
        let mut vy = dir[1] + dy * sp;
        let mut vz = dir[2] + dz * sp;
        let mut l = (vx * vx + vy * vy + vz * vz).sqrt();
        if l.is_nan() || l <= 1e-6 {
            vx = 0.0;
            vy = 1.0;
            vz = 0.0;
            l = 1.0;
        }
        let spv = sd[0] + (sd[1] - sd[0]) * us;
        let life = (lf[0] + (lf[1] - lf[0]) * ul).max(1e-3);
        let size0 = sz[0] + (sz[1] - sz[0]) * uz;
        let d = &self.descs[di];
        let size = size0 * d.size_over_life.sample(0.0);
        let a = lerp(d.c0[3], d.c1[3], 0.0) * d.alpha_over_life.sample(0.0);
        self.parts.push(Particle {
            pos: [og[0] + jx * ex[0], og[1] + jy * ex[1], og[2] + jz * ex[2]],
            vel: [vx / l * spv, vy / l * spv, vz / l * spv],
            age: 0.0,
            life,
            size0,
            size,
            alpha: a,
            col: [d.c0[0], d.c0[1], d.c0[2], a],
            flip: Flipbook {
                cols: d.atlas[0],
                rows: d.atlas[1],
            }
            .index(0.0, d.loop_flip),
            emit,
        });
        self.spawned += 1;
    }
    pub fn state_hash(&self) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        let mut mix = |b: u8| {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        };
        for b in self
            .parts
            .len()
            .to_le_bytes()
            .iter()
            .chain(self.tick.to_le_bytes().iter())
        {
            mix(*b);
        }
        for p in &self.parts {
            for w in [
                p.pos[0].to_bits(),
                p.pos[1].to_bits(),
                p.pos[2].to_bits(),
                p.vel[0].to_bits(),
                p.vel[1].to_bits(),
                p.vel[2].to_bits(),
                p.size.to_bits(),
                p.alpha.to_bits(),
                p.age.to_bits(),
            ] {
                for b in w.to_le_bytes() {
                    mix(b);
                }
            }
            for b in p.flip.to_le_bytes() {
                mix(b);
            }
        }
        h
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn curve_clamp_lerp() {
        let c = Curve::new(vec![(0.0, 0.0), (1.0, 10.0)]);
        assert_eq!(
            (c.sample(-1.0), c.sample(0.5), c.sample(2.0)),
            (0.0, 5.0, 10.0)
        );
        assert_eq!(Curve::new(vec![]).sample(0.3), 1.0);
    }
    #[test]
    fn flipbook_once_loops() {
        let f = Flipbook { cols: 4, rows: 4 };
        assert_eq!(
            (
                f.frames(),
                f.index(0.0, false),
                f.index(0.99, false),
                f.index(1.0, false)
            ),
            (16, 0, 15, 15)
        );
        assert_eq!(f.index(1.0, true), 0);
    }
    #[test]
    fn seed_x2_same_hash_60fps() {
        let run = |seed| {
            let mut e = ParticleEffect::demo(seed);
            let mut v = Vec::new();
            for _ in 0..120 {
                e.tick();
                v.push(e.state_hash());
            }
            v
        };
        let (a, b) = (run(2040), run(2040));
        assert_eq!(a, b);
        assert_ne!(a[0], *a.last().unwrap());
    }
    #[test]
    fn capacity_and_finite() {
        let mut e = ParticleEffect::demo(7);
        for _ in 0..300 {
            e.tick();
        }
        assert!(e.alive() <= 4096);
        assert!(e
            .particles()
            .iter()
            .all(|p| p.pos[0].is_finite() && p.size.is_finite() && p.flip < 16));
    }
}
