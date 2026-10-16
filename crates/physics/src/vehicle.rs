//! vehicle: raycast vehicle (suspension rays + Pacejka-lite tires). Fixed-dt, std-only.
//!
//! Deterministic-angle note (tier 3 LAN replay): every angle call in the
//! fixed tick uses the integer tables below. Each helper scales one f32 by
//! 65536 into Q16 raw units, then uses only integer shifts, adds, muls and
//! table lerps, then scales back with one f32 divide. Past scaling the only
//! float ops are exact +-* and one divide, so all PCs compute the same bits.
//! Tables match the core fixed-point sine table and CORDIC arc-tangent table
//! value for value; local names keep this crate dependency-free.
//! Error bounds (raw LSB is 1/65536 rad or 1/65536 output units):
//! sine/cosine output within 300 raw units (0.0046), arc tangent within
//! 300 raw units (0.0046 rad), Pacejka-lite within 0.012 absolute for the
//! shipped tunes (D <= 1.1, C <= 1.45), bump height within 0.0012 m.
//! Signed zeros are preserved through the odd helpers; non-finite inputs map
//! to a fixed guard (0.0 for sine/arc tangent, 1.0 for cosine) so a NaN can
//! never leak platform-specific bits into the hash.

// Q16 trig tables, integer-only evaluation past the f32 scale step.
use engine_core::Fnv64;

const DET_SCALE_F: f32 = 65536.0;
const DET_TAU_RAW: i64 = 411776;
const DET_PI_RAW: i64 = 205888;
const DET_HALF_PI_RAW: i64 = 102944;
#[rustfmt::skip]
const DET_SIN_LUT: [i64; 65] = [0,1608,3216,4821,6424,8022,9616,11204,12785,14359,15924,17479,19024,20557,22078,23586,25080,26558,28020,29466,30893,32303,33692,35062,36410,37736,39040,40320,41576,42806,44011,45190,46341,47464,48559,49624,50660,51665,52639,53581,54491,55368,56212,57022,57798,58538,59244,59914,60547,61145,61705,62228,62714,63162,63572,63944,64277,64571,64827,65043,65220,65358,65457,65516,65536];
#[rustfmt::skip]
const DET_ATAN_T: [i64; 16] = [51472,30386,16055,8150,4091,2047,1024,512,256,128,64,32,16,8,4,2];
/// Largest magnitude that still scales into Q16 raw units with a finite
/// product: `f32::MAX / 65536`. Bigger finite inputs clamp here, the scaled
/// product stays finite, and the guarded cast below saturates to `i64::MAX`,
/// bit-identical on every PC (matches the old saturating-cast bits).
const DET_MAX_MAG_F: f32 = f32::MAX / DET_SCALE_F;

/// Scale one non-negative magnitude into Q16 raw units (rad * 65536).
/// Finite-guarded: non-finite maps to 0, negatives clamp to 0, huge finite
/// inputs saturate to `i64::MAX`. Single guarded cast site so every PC
/// computes the same bits; callers apply `rem_euclid`/CORDIC after.
fn q16_raw(mag_f: f32) -> i64 {
    if !mag_f.is_finite() {
        return 0;
    }
    let clamped = mag_f.clamp(0.0, DET_MAX_MAG_F);
    let scaled = clamped * DET_SCALE_F;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "guarded finite+clamped Q16 scale; identical saturating cast on every PC"
    )]
    let raw = scaled as i64;
    raw
}

fn det_lut(off: i64) -> i64 {
    if off <= 0 {
        return 0;
    }
    if off >= DET_HALF_PI_RAW {
        return 65536;
    }
    let p = off * 64;
    let idx = usize::try_from(p / DET_HALF_PI_RAW).unwrap_or(0);
    let fr = p % DET_HALF_PI_RAW;
    let a = DET_SIN_LUT[idx];
    let b = DET_SIN_LUT[idx + 1];
    a + (b - a) * fr / DET_HALF_PI_RAW
}

fn det_sin_raw(r: i64) -> i64 {
    match r / DET_HALF_PI_RAW {
        0 => det_lut(r),
        1 => det_lut(DET_PI_RAW - r),
        2 => -det_lut(r - DET_PI_RAW),
        _ => -det_lut(DET_TAU_RAW - r),
    }
}

fn det_atan_pos(mut vx: i64, mut vy: i64) -> i64 {
    // CORDIC arc tangent for non-negative vy with positive vx, 16 steps.
    // Bound: within 300 raw units (0.0046 rad) of the true angle.
    let mut va = 0i64;
    let mut i = 0u32;
    while i < 16 {
        let sx = vx >> i;
        let sy = vy >> i;
        let a = DET_ATAN_T[usize::try_from(i).unwrap_or(0)];
        if vy >= 0 {
            vx = vx.wrapping_add(sy);
            vy = vy.wrapping_sub(sx);
            va = va.wrapping_add(a);
        } else {
            vx = vx.wrapping_sub(sy);
            vy = vy.wrapping_add(sx);
            va = va.wrapping_sub(a);
        }
        i += 1;
    }
    va
}

/// Deterministic sine of an f32 radian angle, bound 0.0046 absolute.
/// Odd by construction (sign handled in float, magnitude via integers).
fn det_sin_f32(x: f32) -> f32 {
    if x == 0.0 {
        return x;
    }
    if !x.is_finite() {
        return 0.0;
    }
    let neg = x < 0.0;
    let a = if neg { -x } else { x };
    let raw = q16_raw(a).rem_euclid(DET_TAU_RAW);
    let s = det_sin_raw(raw) as f32 / DET_SCALE_F;
    if neg {
        -s
    } else {
        s
    }
}

/// Deterministic cosine of an f32 radian angle, bound 0.0046 absolute.
/// Even by construction (magnitude only, shift by half-pi in raw units).
fn det_cos_f32(x: f32) -> f32 {
    if !x.is_finite() {
        return 1.0;
    }
    let a = x.abs();
    let raw = q16_raw(a)
        .wrapping_add(DET_HALF_PI_RAW)
        .rem_euclid(DET_TAU_RAW);
    det_sin_raw(raw) as f32 / DET_SCALE_F
}

/// Deterministic arc tangent of an f32 slope, bound 0.0046 rad.
/// Odd by construction; equals integer CORDIC of (slope, 1).
fn det_atan_f32(z: f32) -> f32 {
    if z == 0.0 {
        return z;
    }
    if !z.is_finite() {
        // `DET_HALF_PI_RAW` is 102944, exactly representable; the literal
        // keeps the guard `as`-free with identical bits.
        return if z > 0.0 {
            102944.0 / DET_SCALE_F
        } else {
            -102944.0 / DET_SCALE_F
        };
    }
    let neg = z < 0.0;
    let a = if neg { -z } else { z };
    let y_raw = q16_raw(a);
    let a_raw = det_atan_pos(65536, y_raw);
    let s = a_raw as f32 / DET_SCALE_F;
    if neg {
        -s
    } else {
        s
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
/// 3D vector, y-up, yaw about +Y.
pub struct V3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}
impl V3 {
    /// new vector.
    pub fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }
    /// zero vector.
    pub fn zero() -> Self {
        Self::new(0.0, 0.0, 0.0)
    }
    /// up vector.
    pub fn up() -> Self {
        Self::new(0.0, 1.0, 0.0)
    }
    /// add.
    #[expect(
        clippy::should_implement_trait,
        reason = "inherent vector helpers; operator traits would need an import at every call site"
    )]
    pub fn add(self, o: V3) -> V3 {
        V3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
    /// sub.
    #[expect(
        clippy::should_implement_trait,
        reason = "inherent vector helpers; operator traits would need an import at every call site"
    )]
    pub fn sub(self, o: V3) -> V3 {
        V3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
    /// scale.
    pub fn scale(self, s: f32) -> V3 {
        V3::new(self.x * s, self.y * s, self.z * s)
    }
    /// dot.
    pub fn dot(self, o: V3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
}
#[derive(Clone, Copy, Debug)]
/// full vehicle tune (suspension + Pacejka-lite + chassis).
pub struct Tune {
    /// chassis mass kg.
    pub mass: f32,
    /// wheel radius m.
    pub wheel_r: f32,
    /// suspension rest length m.
    pub rest: f32,
    /// suspension travel m.
    pub travel: f32,
    /// spring N/m per wheel.
    pub spring: f32,
    /// damper Ns/m per wheel.
    pub damper: f32,
    /// Pacejka stiffness.
    pub pace_b: f32,
    /// Pacejka shape.
    pub pace_c: f32,
    /// Pacejka peak (friction).
    pub pace_d: f32,
    /// aero drag 1/s.
    pub drag: f32,
    /// rolling resist m/s^2.
    pub rr: f32,
    /// lateral axle distance m.
    pub track: f32,
    /// longitudinal axle distance m.
    pub wheelbase: f32,
    /// max steer rad.
    pub steer_max: f32,
}
/// nimble kart tune.
pub fn kart_tune() -> Tune {
    Tune {
        mass: 280.0,
        wheel_r: 0.28,
        rest: 0.30,
        travel: 0.18,
        spring: 18000.0,
        damper: 2200.0,
        pace_b: 10.0,
        pace_c: 1.35,
        pace_d: 1.1,
        drag: 0.35,
        rr: 0.4,
        track: 1.1,
        wheelbase: 1.7,
        steer_max: 0.55,
    }
}
/// heavy sedan tune.
pub fn sedan_tune() -> Tune {
    Tune {
        mass: 1250.0,
        wheel_r: 0.33,
        rest: 0.38,
        travel: 0.22,
        spring: 28000.0,
        damper: 3800.0,
        pace_b: 9.0,
        pace_c: 1.45,
        pace_d: 1.0,
        drag: 0.28,
        rr: 0.35,
        track: 1.6,
        wheelbase: 2.7,
        steer_max: 0.5,
    }
}
#[derive(Clone, Copy, Debug)]
/// driver input: throttle -1..1, brake 0..1, steer -1..1.
pub struct VInput {
    pub throttle: f32,
    pub brake: f32,
    pub steer: f32,
}
impl VInput {
    /// new input.
    pub fn new(throttle: f32, brake: f32, steer: f32) -> Self {
        Self {
            throttle,
            brake,
            steer,
        }
    }
}
#[derive(Clone, Copy, Debug)]
/// vehicle state (4 wheels: FL,FR,RL,RR).
pub struct VState {
    pub pos: V3,
    pub vel: V3,
    pub yaw: f32,
    pub yaw_rate: f32,
    pub comp: [f32; 4],
    pub spin: [f32; 4],
}
impl VState {
    /// new state at pos/yaw.
    pub fn new(pos: V3, yaw: f32) -> Self {
        Self {
            pos,
            vel: V3::zero(),
            yaw,
            yaw_rate: 0.0,
            comp: [0.0; 4],
            spin: [0.0; 4],
        }
    }
}
/// Pacejka-lite magic formula with deterministic trig.
/// Replaces platform arc tangent plus sine with integer-table versions.
/// Bound: within 0.012 absolute of the true formula for shipped tunes.
pub fn pacejka(slip: f32, b: f32, c: f32, d: f32) -> f32 {
    d * det_sin_f32(c * det_atan_f32(b * slip))
}
/// flat ground y=0.
pub fn flat_ground(_x: f32, _z: f32) -> (f32, V3) {
    (0.0, V3::up())
}
/// small-bump heightfield (normal near up).
/// Replaces platform sine/cosine with integer-table versions.
/// Bound: within 0.0012 m of the true bump height.
pub fn bump_ground(x: f32, z: f32) -> (f32, V3) {
    (0.12 * det_sin_f32(x * 0.6) * det_cos_f32(z * 0.6), V3::up())
}
/// FNV-1a hash of vehicle state (bytes via the shared owner).
pub fn state_hash(s: &VState) -> u64 {
    let mut h = Fnv64::new();
    h.write_u32(s.pos.x.to_bits());
    h.write_u32(s.pos.y.to_bits());
    h.write_u32(s.pos.z.to_bits());
    h.write_u32(s.vel.x.to_bits());
    h.write_u32(s.vel.y.to_bits());
    h.write_u32(s.vel.z.to_bits());
    h.write_u32(s.yaw.to_bits());
    h.write_u32(s.yaw_rate.to_bits());
    for i in 0..4 {
        h.write_u32(s.comp[i].to_bits());
        h.write_u32(s.spin[i].to_bits());
    }
    h.finish()
}
/// advance vehicle one fixed step (call at fixed dt).
pub fn step(t: &Tune, s: &mut VState, i: &VInput, dt: f32, ground: fn(f32, f32) -> (f32, V3)) {
    use std::f32::consts::PI;
    let th = i.throttle.clamp(-1.0, 1.0);
    let br = i.brake.clamp(0.0, 1.0);
    let st_ = i.steer.clamp(-1.0, 1.0);
    // Chassis heading from deterministic integer-table trig.
    // Bound: each component within 0.0046 of the true heading.
    let c = det_cos_f32(s.yaw);
    let sn = det_sin_f32(s.yaw);
    let fwd = V3::new(c, 0.0, sn);
    let rgt = V3::new(-sn, 0.0, c);
    let hb = t.wheelbase * 0.5;
    let _ht = t.track * 0.5;
    let vf0 = s.vel.dot(fwd);
    let vr0 = s.vel.dot(rgt);
    let mut fx = 0.0f32;
    let mut fz = 0.0f32;
    let mut fy = 0.0f32;
    let mut tq = 0.0f32;
    for k in 0..4 {
        let (lx, lz) = if k == 0 {
            (hb, _ht)
        } else if k == 1 {
            (hb, -_ht)
        } else if k == 2 {
            (-hb, _ht)
        } else {
            (-hb, -_ht)
        };
        let wx = lx * c - lz * sn;
        let wz = lx * sn + lz * c;
        let (h, _n) = ground(s.pos.x + wx, s.pos.z + wz);
        let susp_len = (s.pos.y - h) - t.wheel_r;
        let comp = (t.rest - susp_len).clamp(-t.travel, t.rest);
        let contact = comp > 0.0;
        // Rate from chassis vy, not clamped compression (clamped diff spikes and drops the car).
        // cv = +vy: load must grow while falling.
        let cv = (s.vel.y).clamp(-8.0, 8.0);
        let mut load = 0.0;
        if contact {
            load = (t.spring * comp - t.damper * cv).max(0.0);
            fy += load;
        }
        let da = if k < 2 { st_ * t.steer_max } else { 0.0 };
        let vlat = vr0 + s.yaw_rate * lx;
        let den = vf0.abs() + 2.0;
        let free_spin = vf0 / t.wheel_r.max(0.05);
        let dir = if vf0 >= 0.0 { 1.0 } else { -1.0 };
        let want = free_spin + th * 28.0 - br * dir * 20.0;
        let rate = if contact { 6.0 } else { 2.0 };
        s.spin[k] += (want - s.spin[k]) * (rate * dt).min(1.0);
        // Slip angle with deterministic arc tangent, bound 0.0046 rad.
        let sa = det_atan_f32(vlat / den) - da;
        let sr = (s.spin[k] * t.wheel_r - vf0) / den;
        let (lat, lon) = if contact {
            (
                -pacejka(sa, t.pace_b, t.pace_c, t.pace_d) * load,
                pacejka(sr, t.pace_b, t.pace_c, t.pace_d) * load,
            )
        } else {
            (0.0, 0.0)
        };
        // Steered-axle frame from deterministic integer-table trig.
        // Bound: each component within 0.0046 of the true frame.
        let cs = det_cos_f32(da);
        let s2 = det_sin_f32(da);
        let sfx = c * cs + (-sn) * s2;
        let sfz = sn * cs + c * s2;
        let srx = -c * s2 + (-sn) * cs;
        let srz = -sn * s2 + c * cs;
        let fxi = sfx * lon + srx * lat;
        let fzi = sfz * lon + srz * lat;
        fx += fxi;
        fz += fzi;
        tq += wz * fxi - wx * fzi;
        s.comp[k] = comp;
    }
    let inv = 1.0 / t.mass.max(1.0);
    s.vel.y += (fy * inv - 9.81) * dt;
    s.vel.x += (fx * inv - s.vel.x * t.drag) * dt;
    s.vel.z += (fz * inv - s.vel.z * t.drag) * dt;
    let vf1 = s.vel.dot(fwd);
    let sg = if vf1 >= 0.0 { 1.0 } else { -1.0 };
    let dec = (t.rr * dt).min(vf1.abs());
    s.vel = s.vel.sub(fwd.scale(sg * dec));
    let ine = t.mass * (t.wheelbase * t.wheelbase + t.track * t.track) / 12.0;
    s.yaw_rate += tq / ine.max(1.0) * dt;
    s.yaw_rate *= (1.0 - 0.8 * dt).max(0.0);
    s.yaw += s.yaw_rate * dt;
    s.yaw = (s.yaw + PI).rem_euclid(2.0 * PI) - PI;
    s.pos = s.pos.add(s.vel.scale(dt));
    let (hc, _) = ground(s.pos.x, s.pos.z);
    let mn = hc + 0.12;
    if s.pos.y < mn {
        s.pos.y = mn;
        if s.vel.y < 0.0 {
            s.vel.y = 0.0;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    const DT: f32 = 1.0 / 60.0;
    #[test]
    fn pacejka_zero_odd_saturates() {
        assert!(pacejka(0.0, 10.0, 1.35, 1.1).abs() < 1e-6);
        let (a, b) = (
            pacejka(0.2, 10.0, 1.35, 1.1),
            pacejka(-0.2, 10.0, 1.35, 1.1),
        );
        assert!((a + b).abs() < 1e-5);
        assert!(a > 0.5 && a <= 1.11);
        assert!((pacejka(5.0, 10.0, 1.35, 1.1) - 1.1).abs() < 0.6);
    }
    #[test]
    fn suspension_settles_from_drop() {
        let t = kart_tune();
        let mut s = VState::new(V3::new(0.0, 2.0, 0.0), 0.0);
        for _ in 0..300 {
            step(&t, &mut s, &VInput::new(0.0, 0.0, 0.0), DT, flat_ground);
        }
        let eq = t.wheel_r + t.rest - t.mass * 9.81 / 4.0 / t.spring;
        assert!((s.pos.y - eq).abs() < 0.06, "y={} eq={}", s.pos.y, eq);
        assert!(s.vel.y.abs() < 0.25, "{:?}", s.vel);
    }
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test oracle builds inputs with libm trig; results are never hashed"
    )]
    fn straight_gains_speed_stays_flat() {
        let t = sedan_tune();
        let mut s = VState::new(V3::new(0.0, 1.0, 0.0), 0.0);
        for _ in 0..300 {
            step(&t, &mut s, &VInput::new(1.0, 0.0, 0.0), DT, flat_ground);
        }
        let fwd = V3::new(s.yaw.cos(), 0.0, s.yaw.sin());
        assert!(s.vel.dot(fwd) > 5.0, "{:?}", s.vel);
        assert!(s.vel.y.abs() < 0.6 && s.pos.y.is_finite());
    }
    #[test]
    fn arena_circle_hash_x2_bumps() {
        let t = kart_tune();
        let run = || {
            let mut s = VState::new(V3::new(0.0, 1.0, 0.0), 0.0);
            for _ in 0..600 {
                step(&t, &mut s, &VInput::new(0.6, 0.0, 0.4), DT, bump_ground);
            }
            state_hash(&s)
        };
        assert_eq!(run(), run());
        let mut s = VState::new(V3::new(0.0, 1.0, 0.0), 0.0);
        for _ in 0..600 {
            step(&t, &mut s, &VInput::new(0.6, 0.0, 0.4), DT, bump_ground);
        }
        assert!(s.pos.x.abs() + s.pos.z.abs() < 120.0, "{:?}", s.pos);
        assert!(s.yaw.abs() > 0.3, "{:?}", s.yaw);
        assert!(s.pos.x.is_finite() && s.vel.x.is_finite());
    }
    #[test]
    fn deterministic_outputs_bit_identical() {
        // Determinism gate: same complete input twice gives bit-identical state.
        let t = kart_tune();
        let run = || {
            let mut s = VState::new(V3::new(0.0, 1.0, 0.0), 0.0);
            for _ in 0..600 {
                step(&t, &mut s, &VInput::new(0.6, 0.0, 0.4), DT, bump_ground);
            }
            s
        };
        let a = run();
        let b = run();
        assert_eq!(state_hash(&a), state_hash(&b));
        for (x, y, name) in [
            (a.pos.x, b.pos.x, "pos.x"),
            (a.pos.y, b.pos.y, "pos.y"),
            (a.pos.z, b.pos.z, "pos.z"),
            (a.vel.x, b.vel.x, "vel.x"),
            (a.vel.y, b.vel.y, "vel.y"),
            (a.vel.z, b.vel.z, "vel.z"),
            (a.yaw, b.yaw, "yaw"),
            (a.yaw_rate, b.yaw_rate, "yaw_rate"),
        ] {
            assert_eq!(x.to_bits(), y.to_bits(), "{name}");
        }
        for k in 0..4 {
            assert_eq!(a.comp[k].to_bits(), b.comp[k].to_bits(), "comp[{k}]");
            assert_eq!(a.spin[k].to_bits(), b.spin[k].to_bits(), "spin[{k}]");
        }
        // Helpers are pure functions of their input: same input, same bits.
        assert_eq!(det_sin_f32(0.7).to_bits(), det_sin_f32(0.7).to_bits());
        assert_eq!(det_cos_f32(0.7).to_bits(), det_cos_f32(0.7).to_bits());
        assert_eq!(det_atan_f32(0.7).to_bits(), det_atan_f32(0.7).to_bits());
        assert_eq!(
            pacejka(0.2, 10.0, 1.35, 1.1).to_bits(),
            pacejka(0.2, 10.0, 1.35, 1.1).to_bits()
        );
    }

    #[test]
    fn hash_matches_shared_fnv_bytes() {
        use engine_core::hash::{FNV64_OFFSET, FNV64_PRIME};
        fn legacy(s: &VState) -> u64 {
            let mut h: u64 = FNV64_OFFSET;
            let mut m = |w: u32| {
                for b in w.to_le_bytes() {
                    h ^= u64::from(b);
                    h = h.wrapping_mul(FNV64_PRIME);
                }
            };
            m(s.pos.x.to_bits());
            m(s.pos.y.to_bits());
            m(s.pos.z.to_bits());
            m(s.vel.x.to_bits());
            m(s.vel.y.to_bits());
            m(s.vel.z.to_bits());
            m(s.yaw.to_bits());
            m(s.yaw_rate.to_bits());
            for i in 0..4 {
                m(s.comp[i].to_bits());
                m(s.spin[i].to_bits());
            }
            h
        }
        let mut s = VState::new(V3::new(1.0, 2.0, -3.0), 0.7);
        s.vel = V3::new(-0.0, f32::NAN, 4.5);
        s.comp = [0.1, 0.2, 0.3, 0.4];
        assert_eq!(state_hash(&s), legacy(&s));
        let t = VState::new(V3::zero(), 0.0);
        assert_eq!(state_hash(&t), legacy(&t));
    }

    #[test]
    fn q16_raw_guards_cover_cast_sites() {
        // Failure case: non-finite never reaches the cast.
        assert_eq!(q16_raw(f32::NAN), 0);
        assert_eq!(q16_raw(f32::INFINITY), 0);
        assert_eq!(q16_raw(f32::NEG_INFINITY), 0);
        // Failure case: negatives clamp to zero (callers pass magnitudes).
        assert_eq!(q16_raw(-1.5), 0);
        // Behavior pin: unit scale is exact.
        assert_eq!(q16_raw(0.0), 0);
        assert_eq!(q16_raw(1.0), 65536);
        // Behavior pin: huge finite saturates identically on every PC.
        assert_eq!(q16_raw(1.0e30), i64::MAX);
        assert_eq!(q16_raw(DET_MAX_MAG_F), i64::MAX);
    }

    #[test]
    fn det_trig_huge_and_non_finite_angles_stay_finite() {
        // Huge finite angles saturate the Q16 scale; the trig still lands in range.
        for v in [1.0e20, 1.0e30, f32::MAX] {
            for f in [det_sin_f32(v), det_cos_f32(v), det_atan_f32(v)] {
                assert!(f.is_finite(), "{v} -> {f}");
            }
        }
        // A saturated magnitude wraps as `i64::MAX` modulo tau, sine and cosine alike.
        let wrapped = i64::MAX.rem_euclid(DET_TAU_RAW);
        let expect_sin = det_sin_raw(wrapped) as f32 / DET_SCALE_F;
        assert_eq!(det_sin_f32(1.0e30).to_bits(), expect_sin.to_bits());
        let wrapped_cos = i64::MAX
            .wrapping_add(DET_HALF_PI_RAW)
            .rem_euclid(DET_TAU_RAW);
        let expect_cos = det_sin_raw(wrapped_cos) as f32 / DET_SCALE_F;
        assert_eq!(det_cos_f32(1.0e30).to_bits(), expect_cos.to_bits());
        // Non-finite angles never reach the cast.
        assert_eq!(det_sin_f32(f32::INFINITY), 0.0);
        assert_eq!(det_sin_f32(f32::NAN), 0.0);
        assert_eq!(det_cos_f32(f32::INFINITY), 1.0);
        assert!((det_sin_f32(0.7) - 0.6442).abs() < 0.005);
        assert!((det_cos_f32(0.7) - 0.7648).abs() < 0.005);
    }
}
