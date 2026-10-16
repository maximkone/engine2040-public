//! kcc: kinematic controller (sweep+slide, slope limit, coyote/buffer, snap). Fixed-dt, std-only.
use engine_core::Fnv64;
#[derive(Clone, Copy, Debug, PartialEq)]
/// 3D vector, y-up.
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
    /// length squared.
    pub fn len2(self) -> f32 {
        self.dot(self)
    }
    /// length.
    pub fn len(self) -> f32 {
        self.len2().sqrt()
    }
    /// normalized (zero if tiny).
    pub fn norm(self) -> V3 {
        let l = self.len();
        if l < 1e-9 {
            V3::zero()
        } else {
            self.scale(1.0 / l)
        }
    }
}
#[derive(Clone, Copy, Debug)]
/// static axis-aligned box collider.
pub struct Aabb {
    pub min: V3,
    pub max: V3,
}
impl Aabb {
    /// new box.
    pub fn new(mn: V3, mx: V3) -> Self {
        Self { min: mn, max: mx }
    }
}
#[derive(Clone, Copy, Debug)]
/// static plane collider: n.dot(p)==d.
pub struct Plane {
    pub n: V3,
    pub d: f32,
}
impl Plane {
    /// plane from normal + point.
    pub fn from_np(n: V3, p: V3) -> Self {
        Self {
            n: n.norm(),
            d: n.norm().dot(p),
        }
    }
    /// horizontal ground at height y.
    pub fn ground(y: f32) -> Self {
        Self { n: V3::up(), d: y }
    }
}
#[derive(Clone, Debug, Default)]
/// static collision world (boxes + planes, fixed order).
pub struct World {
    pub boxes: Vec<Aabb>,
    pub planes: Vec<Plane>,
}
impl World {
    /// empty world.
    pub fn new() -> Self {
        Self {
            boxes: vec![],
            planes: vec![],
        }
    }
    /// push box.
    pub fn push_box(&mut self, b: Aabb) {
        self.boxes.push(b);
    }
    /// push plane.
    pub fn push_plane(&mut self, p: Plane) {
        self.planes.push(p);
    }
    /// push ground plane.
    pub fn ground(&mut self, y: f32) {
        self.planes.push(Plane::ground(y));
    }
}
#[derive(Clone, Copy, Debug)]
/// KCC tuning. All times seconds, dt fixed (1/60).
pub struct KccConfig {
    /// capsule radius (m).
    pub radius: f32,
    /// contact skin (m).
    pub skin: f32,
    /// max slide iterations.
    pub max_slides: u32,
    /// cos(max walkable slope).
    pub slope_cos: f32,
    /// ground snap distance.
    pub step_down: f32,
    /// gravity (+down).
    pub gravity: f32,
    /// jump takeoff velocity.
    pub jump_v: f32,
    /// coyote time.
    pub coyote: f32,
    /// jump buffer time.
    pub buffer: f32,
    /// horizontal accel grounded.
    pub accel_ground: f32,
    /// horizontal accel airborne.
    pub accel_air: f32,
}
impl Default for KccConfig {
    /// default tune: r=.35 slope=45deg g=22 jump=8 coyote=.12 buffer=.15.
    fn default() -> Self {
        Self {
            radius: 0.35,
            skin: 0.02,
            max_slides: 4,
            slope_cos: std::f32::consts::FRAC_1_SQRT_2,
            step_down: 0.35,
            gravity: 22.0,
            jump_v: 8.0,
            coyote: 0.12,
            buffer: 0.15,
            accel_ground: 60.0,
            accel_air: 20.0,
        }
    }
}
#[derive(Clone, Copy, Debug)]
/// per-tick input: wish encodes horizontal velocity (y ignored).
pub struct KccInput {
    pub wish: V3,
    pub jump_pressed: bool,
}
impl KccInput {
    /// new input.
    pub fn new(wish: V3, jump_pressed: bool) -> Self {
        Self { wish, jump_pressed }
    }
}
#[derive(Clone, Copy, Debug)]
/// character state: pos = feet.
pub struct KccState {
    pub pos: V3,
    pub vel: V3,
    pub grounded: bool,
    pub coyote_t: f32,
    pub buffer_t: f32,
}
impl KccState {
    /// new state at feet pos.
    pub fn new(pos: V3) -> Self {
        Self {
            pos,
            vel: V3::zero(),
            grounded: false,
            coyote_t: 0.0,
            buffer_t: 0.0,
        }
    }
}
/// FNV-1a hash of KCC state (bytes via the shared owner).
pub fn state_hash(s: &KccState) -> u64 {
    let mut h = Fnv64::new();
    h.write_u32(s.pos.x.to_bits());
    h.write_u32(s.pos.y.to_bits());
    h.write_u32(s.pos.z.to_bits());
    h.write_u32(s.vel.x.to_bits());
    h.write_u32(s.vel.y.to_bits());
    h.write_u32(s.vel.z.to_bits());
    h.write_u32(s.coyote_t.to_bits());
    h.write_u32(s.buffer_t.to_bits());
    h.write_bool(s.grounded);
    h.finish()
}
/// Normal of the box face nearest to `o`, for a ray that starts inside the
/// box. Ties go to the earlier face in -X, +X, -Y, +Y, -Z, +Z order.
fn nearest_face_normal(o: V3, mn: V3, mx: V3) -> V3 {
    let (dx1, dx2) = (o.x - mn.x, mx.x - o.x);
    let (dy1, dy2) = (o.y - mn.y, mx.y - o.y);
    let (dz1, dz2) = (o.z - mn.z, mx.z - o.z);
    let mut m = dx1;
    let mut n = V3::new(-1.0, 0.0, 0.0);
    if dx2 < m {
        m = dx2;
        n = V3::new(1.0, 0.0, 0.0);
    }
    if dy1 < m {
        m = dy1;
        n = V3::new(0.0, -1.0, 0.0);
    }
    if dy2 < m {
        m = dy2;
        n = V3::new(0.0, 1.0, 0.0);
    }
    if dz1 < m {
        m = dz1;
        n = V3::new(0.0, 0.0, -1.0);
    }
    if dz2 < m {
        n = V3::new(0.0, 0.0, 1.0);
    }
    let _ = m;
    n
}

fn ray_aabb(o: V3, d: V3, mn: V3, mx: V3) -> Option<(f32, V3)> {
    if o.x >= mn.x && o.x <= mx.x && o.y >= mn.y && o.y <= mx.y && o.z >= mn.z && o.z <= mx.z {
        return Some((0.0, nearest_face_normal(o, mn, mx)));
    }
    let mut t0 = 0.0f32;
    let mut t1 = 1.0f32;
    let mut n = V3::zero();
    let mut ok = false;
    if d.x.abs() < 1e-9 {
        if o.x < mn.x || o.x > mx.x {
            return None;
        }
    } else {
        let iv = 1.0 / d.x;
        let mut ta = (mn.x - o.x) * iv;
        let mut tb = (mx.x - o.x) * iv;
        let mut nn = V3::new(-1.0, 0.0, 0.0);
        if ta > tb {
            std::mem::swap(&mut ta, &mut tb);
            nn = V3::new(1.0, 0.0, 0.0);
        }
        if ta > t0 {
            t0 = ta;
            n = nn;
            ok = true;
        }
        t1 = t1.min(tb);
        if t0 > t1 {
            return None;
        }
    }
    if d.y.abs() < 1e-9 {
        if o.y < mn.y || o.y > mx.y {
            return None;
        }
    } else {
        let iv = 1.0 / d.y;
        let mut ta = (mn.y - o.y) * iv;
        let mut tb = (mx.y - o.y) * iv;
        let mut nn = V3::new(0.0, -1.0, 0.0);
        if ta > tb {
            std::mem::swap(&mut ta, &mut tb);
            nn = V3::new(0.0, 1.0, 0.0);
        }
        if ta > t0 {
            t0 = ta;
            n = nn;
            ok = true;
        }
        t1 = t1.min(tb);
        if t0 > t1 {
            return None;
        }
    }
    if d.z.abs() < 1e-9 {
        if o.z < mn.z || o.z > mx.z {
            return None;
        }
    } else {
        let iv = 1.0 / d.z;
        let mut ta = (mn.z - o.z) * iv;
        let mut tb = (mx.z - o.z) * iv;
        let mut nn = V3::new(0.0, 0.0, -1.0);
        if ta > tb {
            std::mem::swap(&mut ta, &mut tb);
            nn = V3::new(0.0, 0.0, 1.0);
        }
        if ta > t0 {
            t0 = ta;
            n = nn;
            ok = true;
        }
        t1 = t1.min(tb);
        if t0 > t1 {
            return None;
        }
    }
    if !ok {
        return None;
    }
    if t0 > 1.0 || t1 < 0.0 {
        return None;
    }
    Some((t0.clamp(0.0, 1.0), n))
}
fn sweep_world(w: &World, c: V3, d: V3, r: f32) -> Option<(f32, V3)> {
    let mut bt = 2.0f32;
    let mut bn = V3::up();
    let mut hit = false;
    for b in &w.boxes {
        let mn = V3::new(b.min.x - r, b.min.y - r, b.min.z - r);
        let mx = V3::new(b.max.x + r, b.max.y + r, b.max.z + r);
        if let Some((t, n)) = ray_aabb(c, d, mn, mx) {
            if t < bt {
                bt = t;
                bn = n;
                hit = true;
            }
        }
    }
    for p in &w.planes {
        let dn = d.dot(p.n);
        if dn >= 0.0 {
            continue;
        }
        let dist = c.dot(p.n) - p.d - r;
        if dist < 0.0 {
            if 0.0 < bt {
                bt = 0.0;
                bn = p.n;
                hit = true;
            }
            continue;
        }
        let t = -dist / dn;
        if (0.0..=1.0).contains(&t) && t < bt {
            bt = t;
            bn = p.n;
            hit = true;
        }
    }
    if hit && bt <= 1.0 {
        Some((bt, bn))
    } else {
        None
    }
}
/// Highest low-ledge top under the intended horizontal move, when it sits
/// within step reach above the feet. Max over candidates, so slice order
/// never decides. Boxes only; planes are ground, never stairs.
fn fps_step_top(w: &World, c: V3, rem: V3, r: f32, max_rise: f32) -> Option<f32> {
    let feet = c.y - r;
    let (px, pz) = (c.x + rem.x, c.z + rem.z);
    let mut best: Option<f32> = None;
    for b in &w.boxes {
        if !(px > b.min.x - r && px < b.max.x + r && pz > b.min.z - r && pz < b.max.z + r) {
            continue;
        }
        let top = b.max.y;
        if top > feet && top <= feet + max_rise && best.is_none_or(|v| top > v) {
            best = Some(top);
        }
    }
    best
}

/// True when the stepped capsule center clears every expanded box.
/// Strict bounds, so resting exactly on a top counts as free.
fn fps_headroom_free(w: &World, c: V3, r: f32) -> bool {
    for b in &w.boxes {
        if c.x > b.min.x - r
            && c.x < b.max.x + r
            && c.y > b.min.y - r
            && c.y < b.max.y + r
            && c.z > b.min.z - r
            && c.z < b.max.z + r
        {
            return false;
        }
    }
    true
}
/// advance KCC one fixed step (call at fixed dt).
pub fn step(cfg: &KccConfig, w: &World, s: &mut KccState, inp: &KccInput, dt: f32) {
    if s.grounded {
        s.coyote_t = cfg.coyote;
    } else {
        s.coyote_t = (s.coyote_t - dt).max(0.0);
    }
    if inp.jump_pressed {
        s.buffer_t = cfg.buffer;
    } else {
        s.buffer_t = (s.buffer_t - dt).max(0.0);
    }
    let acc = if s.grounded {
        cfg.accel_ground
    } else {
        cfg.accel_air
    };
    let mx = acc * dt;
    s.vel.x += (inp.wish.x - s.vel.x).clamp(-mx, mx);
    s.vel.z += (inp.wish.z - s.vel.z).clamp(-mx, mx);
    s.vel.y -= cfg.gravity * dt;
    if s.buffer_t > 0.0 && (s.grounded || s.coyote_t > 0.0) {
        s.vel.y = cfg.jump_v;
        s.grounded = false;
        s.coyote_t = 0.0;
        s.buffer_t = 0.0;
    }
    let r = cfg.radius + cfg.skin;
    let mut c = V3::new(s.pos.x, s.pos.y + r, s.pos.z);
    let mut delta = V3::new(s.vel.x * dt, s.vel.y * dt, s.vel.z * dt);
    let mut g = false;
    for _ in 0..cfg.max_slides {
        if delta.len2() < 1e-12 {
            break;
        }
        if let Some((t, n)) = sweep_world(w, c, delta, r) {
            // FPS collide: low ledges step up, tall walls slide. Step-up runs
            // first on wall faces while falling or grounded, so stairs never
            // read as walls; ceilings and jumps keep the slide path. The probe
            // uses the full intended horizontal move (past the face), because
            // the remainder stops at contact and never reaches the tread.
            if n.y < cfg.slope_cos
                && s.vel.y <= 0.01
                && delta.x * delta.x + delta.z * delta.z > 1e-12
            {
                if let Some(top) = fps_step_top(w, c, delta, r, cfg.step_down) {
                    let stepped = V3::new(c.x + delta.x, top + r, c.z + delta.z);
                    if fps_headroom_free(w, stepped, r) {
                        c = stepped;
                        g = true;
                        s.vel.y = 0.0;
                        delta = V3::zero();
                        continue;
                    }
                }
            }
            c = c.add(delta.scale(t)).add(n.scale(cfg.skin * 0.5));
            let vn = s.vel.dot(n);
            if vn < 0.0 {
                s.vel = s.vel.sub(n.scale(vn));
            }
            if n.y >= cfg.slope_cos {
                g = true;
            } else if n.y < -0.3 && s.vel.y > 0.0 {
                s.vel.y = 0.0;
            }
            let rem = delta.scale(1.0 - t);
            delta = rem.sub(n.scale(rem.dot(n)));
            if delta.len2() < 1e-10 {
                break;
            }
        } else {
            c = c.add(delta);
            break;
        }
    }
    if !g && s.vel.y <= 0.01 {
        let probe = V3::new(0.0, -(cfg.step_down + cfg.skin), 0.0);
        if let Some((t, n)) = sweep_world(w, c, probe, r) {
            if n.y >= cfg.slope_cos && t <= 1.0 {
                c = c.add(probe.scale(t));
                g = true;
                s.vel.y = 0.0;
            }
        }
    }
    if g && s.vel.y < 0.0 {
        s.vel.y = 0.0;
    }
    s.pos = V3::new(c.x, c.y - r, c.z);
    s.grounded = g;
}
#[cfg(test)]
mod tests {
    use super::*;
    const DT: f32 = 1.0 / 60.0;
    fn course() -> World {
        let mut w = World::new();
        w.push_box(Aabb::new(V3::new(-6.0, -1.0, -2.0), V3::new(2.0, 0.0, 2.0)));
        w.push_box(Aabb::new(V3::new(4.0, -1.0, -2.0), V3::new(12.0, 0.0, 2.0)));
        w.push_box(Aabb::new(V3::new(-1.0, 0.0, 1.0), V3::new(8.0, 2.0, 1.5)));
        w
    }
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test oracle builds inputs with libm trig; results are never hashed"
    )]
    fn slope_walkable_vs_wall() {
        let cfg = KccConfig::default();
        let s20: f32 = 20.0f32.to_radians();
        let mut w = World::new();
        w.push_plane(Plane::from_np(
            V3::new(-s20.sin(), s20.cos(), 0.0),
            V3::zero(),
        ));
        let mut s = KccState::new(V3::new(-2.0, 2.0, 0.0));
        for _ in 0..180 {
            step(
                &cfg,
                &w,
                &mut s,
                &KccInput::new(V3::new(3.0, 0.0, 0.0), false),
                DT,
            );
        }
        assert!(s.grounded, "ramp should ground");
        assert!(s.pos.x > -1.0, "climb {:?}", s.pos);
        assert!(s.pos.y > -0.6, "{:?}", s.pos);
        let mut w2 = World::new();
        w2.ground(0.0);
        w2.push_plane(Plane {
            n: V3::new(-1.0, 0.0, 0.0),
            d: -3.0,
        });
        let mut s2 = KccState::new(V3::new(0.0, 0.05, 0.0));
        s2.grounded = true;
        for _ in 0..120 {
            step(
                &cfg,
                &w2,
                &mut s2,
                &KccInput::new(V3::new(5.0, 0.0, 0.0), false),
                DT,
            );
        }
        assert!(s2.pos.x < 3.0 - 0.35 - 0.02 - 0.01, "blocked {:?}", s2.pos);
        assert!(s2.grounded);
    }
    #[test]
    fn coyote_crosses_gap_no_jump_falls() {
        let cfg = KccConfig::default();
        let w = course();
        let run = |jump_tick: i32| {
            let mut s = KccState::new(V3::new(-4.0, 0.0, 0.0));
            s.grounded = true;
            // Record any tick on the box2 deck (past it lies void).
            let mut crossed = false;
            for t in 0..240 {
                let j = t == jump_tick;
                step(
                    &cfg,
                    &w,
                    &mut s,
                    &KccInput::new(V3::new(6.0, 0.0, 0.0), j),
                    DT,
                );
                crossed = crossed || (s.pos.x > 4.5 && s.pos.y > -0.15);
            }
            (s, crossed)
        };
        let (ok, crossed) = run(62);
        assert!(crossed, "coyote must cross the gap, ended {:?}", ok.pos);
        let (bad, bad_crossed) = run(-1);
        assert!(!bad_crossed, "no jump must not cross, ended {:?}", bad.pos);
    }
    #[test]
    fn buffer_fires_late_press() {
        let cfg = KccConfig::default();
        let mut w = World::new();
        w.ground(0.0);
        let run = |jt: i32| {
            let mut s = KccState::new(V3::new(0.0, 2.0, 0.0));
            let mut y30 = 0.0;
            for t in 0..60 {
                step(&cfg, &w, &mut s, &KccInput::new(V3::zero(), t == jt), DT);
                if t == 30 {
                    y30 = s.pos.y;
                }
            }
            y30
        };
        assert!(run(18) > 0.25, "buffered jump should be airborne at t30");
        assert!(run(-1) < 0.1, "no press stays grounded");
    }
    #[test]
    fn slide_along_side_wall() {
        // Shared course misses the wall; this one hits the face head-on and must slide.
        let cfg = KccConfig::default();
        let mut w = World::new();
        w.ground(0.0);
        w.push_box(Aabb::new(V3::new(-2.0, 0.0, 1.0), V3::new(12.0, 2.0, 2.0)));
        let mut s = KccState::new(V3::new(0.0, 0.05, -1.0));
        s.grounded = true;
        for _ in 0..120 {
            step(
                &cfg,
                &w,
                &mut s,
                &KccInput::new(V3::new(4.0, 0.0, 4.0), false),
                DT,
            );
        }
        assert!(s.pos.x > 0.0, "progress {:?}", s.pos);
        assert!(s.pos.z < 0.7, "slide clamp {:?}", s.pos);
        assert!(s.pos.y.abs() < 0.5, "stays grounded {:?}", s.pos);
    }
    #[test]
    fn course_arena_hash_x2() {
        let cfg = KccConfig::default();
        let w = course();
        let run = || {
            let mut s = KccState::new(V3::new(-4.0, 0.0, -1.0));
            s.grounded = true;
            for t in 0..400 {
                let j = t == 62;
                step(
                    &cfg,
                    &w,
                    &mut s,
                    &KccInput::new(V3::new(6.0, 0.0, 0.0), j),
                    DT,
                );
            }
            state_hash(&s)
        };
        assert_eq!(run(), run());
        let mut s = KccState::new(V3::new(-4.0, 0.0, -1.0));
        s.grounded = true;
        for t in 0..400 {
            step(
                &cfg,
                &w,
                &mut s,
                &KccInput::new(V3::new(6.0, 0.0, 0.0), t == 62),
                DT,
            );
        }
        assert!(s.pos.x > 8.0, "goal {:?}", s.pos);
    }

    #[test]
    fn hash_matches_shared_fnv_bytes() {
        use engine_core::hash::{FNV64_OFFSET, FNV64_PRIME};
        fn legacy(s: &KccState) -> u64 {
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
            m(s.coyote_t.to_bits());
            m(s.buffer_t.to_bits());
            h ^= u64::from(s.grounded);
            h = h.wrapping_mul(FNV64_PRIME);
            h
        }
        let mut s = KccState::new(V3::new(-4.0, 0.0, -1.0));
        s.grounded = true;
        assert_eq!(state_hash(&s), legacy(&s));
        s.vel = V3::new(f32::NAN, -0.0, f32::INFINITY);
        s.grounded = false;
        assert_eq!(state_hash(&s), legacy(&s));
    }
    #[test]
    fn fps_steps_onto_low_ledge() {
        // FPS collide: a 0.25 m stair (under the 0.35 m snap/step lane)
        // steps up instead of blocking; a 0.6 m wall still blocks.
        let cfg = KccConfig::default();
        let mut w = World::new();
        w.ground(0.0);
        w.push_box(Aabb::new(V3::new(2.0, 0.0, -2.0), V3::new(4.0, 0.25, 2.0)));
        let mut s = KccState::new(V3::new(0.0, 0.05, 0.0));
        s.grounded = true;
        let mut peak_y = 0.0f32;
        for _ in 0..180 {
            step(
                &cfg,
                &w,
                &mut s,
                &KccInput::new(V3::new(4.0, 0.0, 0.0), false),
                DT,
            );
            peak_y = peak_y.max(s.pos.y);
        }
        assert!(
            s.pos.x > 4.5,
            "low stair must step up and cross, ended {s:?}"
        );
        assert!(
            (peak_y - 0.25).abs() < 0.08,
            "must ride the stair top mid-run, peak={peak_y}, ended {s:?}"
        );
        assert!(s.grounded, "stair walk ends grounded, {s:?}");
        let mut tall = World::new();
        tall.ground(0.0);
        tall.push_box(Aabb::new(V3::new(2.0, 0.0, -2.0), V3::new(4.0, 0.6, 2.0)));
        let mut t = KccState::new(V3::new(0.0, 0.05, 0.0));
        t.grounded = true;
        for _ in 0..180 {
            step(
                &cfg,
                &tall,
                &mut t,
                &KccInput::new(V3::new(4.0, 0.0, 0.0), false),
                DT,
            );
        }
        assert!(t.pos.x < 2.0, "0.6 m wall must block, ended {t:?}");
    }
    #[test]
    fn fps_fast_move_does_not_tunnel_thin_wall() {
        // No tunneling: a full-speed sprint tick into a 0.1 m wall stops
        // at the face (swept segment cast), never past it.
        let cfg = KccConfig::default();
        let mut w = World::new();
        w.ground(0.0);
        w.push_box(Aabb::new(V3::new(3.0, 0.0, -3.0), V3::new(3.1, 2.0, 3.0)));
        let mut s = KccState::new(V3::new(0.0, 0.05, 0.0));
        s.grounded = true;
        for _ in 0..120 {
            step(
                &cfg,
                &w,
                &mut s,
                &KccInput::new(V3::new(8.0, 0.0, 0.0), false),
                DT,
            );
        }
        let face = 3.0 - cfg.radius - cfg.skin;
        assert!(
            s.pos.x <= face + 0.02,
            "must not tunnel past the thin wall, ended {s:?}"
        );
        assert!(s.pos.x > 1.0, "must reach the wall, ended {s:?}");
        assert!(s.pos.y.abs() < 0.5, "stays grounded, {s:?}");
    }
    #[test]
    fn fps_f32_matches_f64_oracle_free_space() {
        // Deterministic fixed-dt: free-space walk matches an f64 oracle
        // within float tolerance, and the same run hashes x2.
        let cfg = KccConfig::default();
        let mut w = World::new();
        w.ground(0.0);
        let run = || {
            let mut s = KccState::new(V3::new(0.0, 0.05, 0.0));
            s.grounded = true;
            for _ in 0..120 {
                step(
                    &cfg,
                    &w,
                    &mut s,
                    &KccInput::new(V3::new(4.0, 0.0, 0.0), false),
                    DT,
                );
            }
            s
        };
        let (a, b) = (run(), run());
        assert_eq!(state_hash(&a), state_hash(&b), "replay must hash x2");
        assert_eq!(a.pos.x, b.pos.x);
        // f64 oracle: same accel approach plus fixed-dt integration.
        let (mut vx, mut x) = (0.0f64, 0.0f64);
        let (dt, acc, wish) = (f64::from(DT), f64::from(cfg.accel_ground), 4.0f64);
        for _ in 0..120 {
            let mx = acc * dt;
            vx += (wish - vx).clamp(-mx, mx);
            x += vx * dt;
        }
        let dx = (f64::from(a.pos.x) - x).abs();
        assert!(
            dx < 0.05,
            "f32 runtime must track the f64 oracle, dx={dx}, x={}, oracle={x}",
            a.pos.x,
        );
    }
}
