//! inertialize: foot-lock + angular limiter. Per-foot pin + slide meter, capped joint turn rate.

fn san(v: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}
fn san3(v: [f32; 3]) -> [f32; 3] {
    [san(v[0]), san(v[1]), san(v[2])]
}
fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (dx, dy, dz) = (
        san(a[0]) - san(b[0]),
        san(a[1]) - san(b[1]),
        san(a[2]) - san(b[2]),
    );
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// ALS contact test: planted when low and slow.
pub fn contact(height_y: f32, speed: f32, h_max: f32, v_max: f32) -> bool {
    if !height_y.is_finite() || !speed.is_finite() || !h_max.is_finite() || !v_max.is_finite() {
        return false;
    }
    height_y <= h_max && speed <= v_max
}

/// Foot separation distance (slide ruler).
pub fn slide_dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    dist(a, b)
}

/// Per-foot lock: pins output while `contact`, meters slide. Breaks past break_r.
#[derive(Clone, Debug)]
pub struct FootLock {
    /// Pinned world pos while planted.
    pub locked: Option<[f32; 3]>,
    /// Accumulated slide (m). Accept gate: < 0.02.
    pub slide: f32,
    /// Re-lock radius (m). Beyond = foot lifted/moved, follow.
    pub break_r: f32,
}

impl FootLock {
    /// New lock with break radius (<=0 or NaN -> 0.15m).
    pub fn new(break_r: f32) -> Self {
        Self {
            locked: None,
            slide: 0.0,
            break_r: if break_r.is_finite() && break_r > 0.0 {
                break_r
            } else {
                0.15
            },
        }
    }
    /// True while pinned.
    pub fn is_locked(&self) -> bool {
        self.locked.is_some()
    }
    /// Total slide since lock (m).
    pub fn slide(&self) -> f32 {
        self.slide
    }
    /// Clear pin and meter.
    pub fn reset(&mut self) {
        self.locked = None;
        self.slide = 0.0;
    }
    /// Step: contact=true pins (output=locked), false releases (output=cur).
    pub fn update(&mut self, cur: [f32; 3], contact: bool) -> [f32; 3] {
        let c = san3(cur);
        if !contact {
            self.locked = None;
            return c;
        }
        match self.locked {
            None => {
                self.locked = Some(c);
                c
            }
            Some(l) => {
                let d = dist(c, l);
                if d > self.break_r {
                    self.slide += d;
                    self.locked = Some(c);
                    c
                } else {
                    l
                }
            }
        }
    }
}

/// Identity quat for limiter resets.
const IDENT_Q: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
/// Default angular limit in deg/s: 180 deg in 10 ticks at 60 Hz.
pub const DEFAULT_ANG_LIMIT_DEG_PER_SEC: f32 = 1080.0;
/// Fixed anim dt in seconds for the 60 Hz clock.
/// Single owner is `engine_core::DT` (identical bits to `1.0 / 60.0`).
const TICK_DT_S: f32 = engine_core::DT;

fn qdot(a: [f32; 4], b: [f32; 4]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]
}

fn qclean(q: [f32; 4]) -> [f32; 4] {
    if !q[0].is_finite() || !q[1].is_finite() || !q[2].is_finite() || !q[3].is_finite() {
        return IDENT_Q;
    }
    let n2 = q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3];
    if !n2.is_finite() || n2 <= 0.0 {
        return IDENT_Q;
    }
    let inv = 1.0 / n2.sqrt();
    [q[0] * inv, q[1] * inv, q[2] * inv, q[3] * inv]
}

/// asin(y) for y in 0..=0.75, Taylor to y^11; error under 0.03 deg.
/// Polynomial only: platform transcendentals round differently across
/// CPUs, so sim code never calls them (workspace determinism gate; the
/// same Taylor shape as the yaw step in `readable.rs`).
fn asin_unit(y: f32) -> f32 {
    let c = y.clamp(0.0, 0.75);
    let c2 = c * c;
    c * (1.0
        + c2 * (1.0 / 6.0
            + c2 * (3.0 / 40.0 + c2 * (5.0 / 112.0 + c2 * (35.0 / 1152.0 + c2 * 63.0 / 2816.0)))))
}

/// acos(d) for d in 0..=1 via 2*asin(sqrt((1-d)/2)); same error bar.
fn acos_unit(d: f32) -> f32 {
    2.0 * asin_unit(((1.0 - d.clamp(0.0, 1.0)) * 0.5).sqrt())
}

/// sin(x) for x in 0..=PI: fold to [0, PI/2], Taylor to x^9; error ~1e-5.
fn sin_unit(x: f32) -> f32 {
    const HPI: f32 = std::f32::consts::FRAC_PI_2;
    const PI: f32 = std::f32::consts::PI;
    let c = x.clamp(0.0, PI);
    let r = if c > HPI { PI - c } else { c };
    let r2 = r * r;
    r * (1.0 + r2 * (-1.0 / 6.0 + r2 * (1.0 / 120.0 + r2 * (-1.0 / 5040.0 + r2 / 362880.0))))
}

/// Shortest-arc angle between two unit quats in degrees.
fn qangle_deg(a: [f32; 4], b: [f32; 4]) -> f32 {
    let mut d = qdot(a, b);
    if !d.is_finite() {
        return 180.0;
    }
    if d < 0.0 {
        d = -d;
    }
    if d > 1.0 {
        d = 1.0;
    }
    2.0 * acos_unit(d) * 180.0 / std::f32::consts::PI
}

/// Shortest-arc slerp between unit quats; `t` in 0..=1.
fn qslerp(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let tt = if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mut bb = b;
    let mut d = qdot(a, b);
    if !d.is_finite() {
        return a;
    }
    if d < 0.0 {
        d = -d;
        bb = [-b[0], -b[1], -b[2], -b[3]];
    }
    if d > 0.9995 {
        let out = [
            a[0] + (bb[0] - a[0]) * tt,
            a[1] + (bb[1] - a[1]) * tt,
            a[2] + (bb[2] - a[2]) * tt,
            a[3] + (bb[3] - a[3]) * tt,
        ];
        return qclean(out);
    }
    if d > 1.0 {
        d = 1.0;
    }
    let ang = acos_unit(d);
    if !ang.is_finite() || ang < 1e-6 {
        return a;
    }
    let s = sin_unit(ang);
    if !s.is_finite() || s.abs() < 1e-6 {
        return qclean([
            a[0] + (bb[0] - a[0]) * tt,
            a[1] + (bb[1] - a[1]) * tt,
            a[2] + (bb[2] - a[2]) * tt,
            a[3] + (bb[3] - a[3]) * tt,
        ]);
    }
    let w0 = sin_unit((1.0 - tt) * ang) / s;
    let w1 = sin_unit(tt * ang) / s;
    qclean([
        a[0] * w0 + bb[0] * w1,
        a[1] * w0 + bb[1] * w1,
        a[2] * w0 + bb[2] * w1,
        a[3] * w0 + bb[3] * w1,
    ])
}

/// Angular-speed limiter for joint rotations: caps per-tick change.
///
/// Owner: `engine-anim`; consumer: `crate::space::SpacePlayer` plus the game look.
/// Units: degrees per second, seconds for `dt`, unit quats.
/// Determinism: shortest-arc slerp with finite guards; same inputs give same bytes x2.
/// Errors: never fails; non-finite quats become identity, bad limits become the default.
#[derive(Clone, Debug)]
pub struct AngularLimiter {
    last: Vec<[f32; 4]>,
    limit_deg_per_sec: f32,
}

impl Default for AngularLimiter {
    fn default() -> Self {
        Self {
            last: Vec::new(),
            limit_deg_per_sec: DEFAULT_ANG_LIMIT_DEG_PER_SEC,
        }
    }
}

impl AngularLimiter {
    /// New limiter for `joints` bones with `limit_deg_per_sec`.
    /// Non-finite or non-positive limits become the default.
    pub fn new(joints: usize, limit_deg_per_sec: f32) -> Self {
        let limit = if limit_deg_per_sec.is_finite() && limit_deg_per_sec > 0.0 {
            limit_deg_per_sec
        } else {
            DEFAULT_ANG_LIMIT_DEG_PER_SEC
        };
        Self {
            last: vec![IDENT_Q; joints],
            limit_deg_per_sec: limit,
        }
    }
    /// Joint count.
    pub fn len(&self) -> usize {
        self.last.len()
    }
    /// True when empty.
    pub fn is_empty(&self) -> bool {
        self.last.is_empty()
    }
    /// Limit in deg/s.
    pub fn limit_deg_per_sec(&self) -> f32 {
        self.limit_deg_per_sec
    }
    /// Size for `joints` bones and reset to identity; reuses storage when the shape matches.
    pub fn prepare(&mut self, joints: usize) {
        if self.last.len() != joints {
            self.last.clear();
            self.last.reserve(joints);
            self.last.resize(joints, IDENT_Q);
        } else {
            self.last.fill(IDENT_Q);
        }
    }
    /// Reset all lanes to identity.
    pub fn reset(&mut self) {
        self.last.fill(IDENT_Q);
    }
    /// Limit `wanted` toward the stored pose by `limit * dt`; writes `out` and stores it.
    /// Steady-state calls allocate nothing; slices in, slices out.
    pub fn step(&mut self, wanted: &[[f32; 4]], dt: f32, out: &mut [[f32; 4]]) {
        let m = self.last.len().min(wanted.len()).min(out.len());
        let limit = if self.limit_deg_per_sec.is_finite() && self.limit_deg_per_sec > 0.0 {
            self.limit_deg_per_sec
        } else {
            DEFAULT_ANG_LIMIT_DEG_PER_SEC
        };
        let dt_c = if dt.is_finite() && dt > 0.0 {
            dt
        } else {
            TICK_DT_S
        };
        let max_step = limit * dt_c;
        for i in 0..m {
            let w = qclean(wanted[i]);
            let cur = qclean(self.last[i]);
            let ang = qangle_deg(cur, w);
            if !ang.is_finite() || ang <= max_step || ang < 1e-6 {
                out[i] = w;
                self.last[i] = w;
            } else {
                let t = (max_step / ang).clamp(0.0, 1.0);
                let next = qslerp(cur, w, t);
                out[i] = next;
                self.last[i] = next;
            }
        }
    }
    /// Limit `io` in place by `limit * dt`; stores the result.
    /// Steady-state calls allocate nothing.
    pub fn limit_in_place(&mut self, io: &mut [[f32; 4]], dt: f32) {
        let m = self.last.len().min(io.len());
        let limit = if self.limit_deg_per_sec.is_finite() && self.limit_deg_per_sec > 0.0 {
            self.limit_deg_per_sec
        } else {
            DEFAULT_ANG_LIMIT_DEG_PER_SEC
        };
        let dt_c = if dt.is_finite() && dt > 0.0 {
            dt
        } else {
            TICK_DT_S
        };
        let max_step = limit * dt_c;
        for (last, slot) in self.last.iter_mut().zip(io.iter_mut()).take(m) {
            let w = qclean(*slot);
            let cur = qclean(*last);
            let ang = qangle_deg(cur, w);
            if !ang.is_finite() || ang <= max_step || ang < 1e-6 {
                *slot = w;
                *last = w;
            } else {
                let t = (max_step / ang).clamp(0.0, 1.0);
                let next = qslerp(cur, w, t);
                *slot = next;
                *last = next;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn footlock_slide_under_2cm() {
        let mut fl = FootLock::new(0.15);
        let l = [1.0, 0.02, 0.0];
        assert_eq!(fl.update(l, true), l);
        assert!(fl.is_locked());
        for i in 0..60 {
            let j = [1.0 + 0.001 * ((i % 2) as f32 * 2.0 - 1.0), 0.02, 0.0];
            let o = fl.update(j, true);
            assert_eq!(o, l);
        }
        assert!(fl.slide() < 0.02, "slide={}", fl.slide());
        assert!(slide_dist(fl.update(l, true), l) < 0.02);
        let free = fl.update([5.0, 1.0, 0.0], false);
        assert_eq!(free, [5.0, 1.0, 0.0]);
        assert!(!fl.is_locked());
    }
    #[test]
    fn footlock_break_and_contact() {
        assert!(contact(0.02, 0.3, 0.05, 1.0));
        assert!(!contact(0.5, 0.1, 0.05, 1.0));
        assert!(!contact(0.02, f32::NAN, 0.05, 1.0));
        let mut fl = FootLock::new(0.15);
        fl.update([0.0, 0.0, 0.0], true);
        let moved = fl.update([1.0, 0.0, 0.0], true);
        assert_eq!(moved, [1.0, 0.0, 0.0]);
        assert!(fl.slide() > 0.0);
    }
    #[test]
    fn ang_limit_takes_ten_ticks_for_180_deg_at_1080_degs() {
        const ID: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
        const HALF_Y: [f32; 4] = [0.0, 1.0, 0.0, 0.0];
        let dt = 1.0 / 60.0;
        let mut lim = AngularLimiter::new(1, 1080.0);
        assert_eq!(lim.len(), 1);
        assert!(!lim.is_empty());
        assert!((lim.limit_deg_per_sec() - 1080.0).abs() < 1e-6);
        let want = [HALF_Y];
        let mut out = [ID];
        lim.step(&want, dt, &mut out);
        let first = qangle_deg(ID, out[0]);
        assert!(
            (17.0..=19.0).contains(&first),
            "first step {first} deg, want 18 +-1"
        );
        for _ in 0..9 {
            lim.step(&want, dt, &mut out);
        }
        let done = qangle_deg(out[0], HALF_Y);
        assert!(done <= 1.0, "after 10 ticks off {done} deg");
        // Twins replay byte-equal; extra hit still capped.
        let mut a = AngularLimiter::new(1, 1080.0);
        let mut b = AngularLimiter::new(1, 1080.0);
        let mut oa = [ID];
        let mut ob = [ID];
        for _ in 0..10 {
            a.step(&want, dt, &mut oa);
            b.step(&want, dt, &mut ob);
        }
        assert_eq!(oa, ob);
        // Failure guards: bad limit defaults, NaN wanted becomes identity.
        assert!((AngularLimiter::new(1, f32::NAN).limit_deg_per_sec() - 1080.0).abs() < 1e-6);
        assert!((AngularLimiter::new(1, 0.0).limit_deg_per_sec() - 1080.0).abs() < 1e-6);
        let mut bad = AngularLimiter::new(1, 1080.0);
        let mut bout = [ID];
        bad.step(&[[f32::NAN, 0.0, 0.0, 0.0]], dt, &mut bout);
        assert_eq!(bout[0], ID);
        bad.prepare(1);
        assert_eq!(bad.len(), 1);
        bad.reset();
        let mut io = [HALF_Y];
        bad.limit_in_place(&mut io, dt);
        assert!(qangle_deg(ID, io[0]) <= 19.0);
    }
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test-only libm oracle measures polynomial helpers; production keeps sin_unit/acos_unit"
    )]
    fn poly_trig_matches_libm_oracle() {
        use std::f32::consts::PI;
        // Oracle uses platform libm; allowed here: tests sit after
        // #[cfg(test)], which the workspace determinism scan skips.
        // Production must stay on the polynomial helpers below.
        for deg in [
            0.0, 0.5, 1.0, 5.0, 17.0, 18.0, 19.0, 45.0, 90.0, 135.0, 179.0, 180.0,
        ] {
            let rad = deg * PI / 180.0;
            let d = rad.cos().abs().clamp(0.0, 1.0);
            // Shortest arc: qangle folds past 90 deg via |dot|.
            let want = if deg > 90.0 { 180.0 - deg } else { deg };
            let got_deg = acos_unit(d) * 180.0 / PI;
            assert!(
                (got_deg - want).abs() < 0.1,
                "acos {deg} deg: got {got_deg} want {want}"
            );
            let (got_sin, want_sin) = (sin_unit(rad), rad.sin());
            assert!(
                (got_sin - want_sin).abs() < 1e-3,
                "sin {deg} deg: got {got_sin} want {want_sin}"
            );
        }
    }
}
