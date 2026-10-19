//! spline: rational NURBS curves, arc-length tables and twist-free sweep
//! frames, for rails, roads and rope paths baked by content tools.
//! Owner: nav crate; callers build a [`Nurbs`], ask for points, tangents,
//! the nearest parameter and [`Frame`]s at even arc-length steps, and sweep a
//! 2D profile along them with [`sweep`].
//! Units: curve units in, curve units out; parameters live in the knot
//! domain. Determinism: only `+ - * /`, `sqrt` and comparisons on `f64`
//! (no sin, cos, pow or fused ops), fixed loop orders, no clock, no hash
//! order, so one curve and one request give the same bits on every machine.
//! Errors: typed [`SplineError`]; bad curves refuse at construction.
//! Budgets: a point or tangent is O(degree^2) on the stack; tables and
//! frames allocate their output only.
//! PORT(flax-mcp plugins/world Procedural/Core/Packs/Nurbs SplineMath and
//! ParallelTransport, VectorExtrusion/VectorExtruder, MIT, own code): Cox-de
//! Boor evaluation in homogeneous coordinates, analytic rational tangents,
//! arc-length lookup, double-reflection parallel transport and ear-clip caps,
//! reimplemented without any transcendental function.

use std::fmt;

/// Highest curve degree accepted (keeps evaluation on a fixed stack array).
pub const MAX_DEGREE: usize = 7;
/// Most control points one curve holds.
pub const MAX_POINTS: usize = 4096;
/// Most samples one table or frame list holds.
pub const MAX_SAMPLES: usize = 1 << 16;
/// Sample count of the arc-length table behind frames and nearest queries.
const TABLE_SAMPLES: usize = 512;

/// Typed spline failures.
#[derive(Clone, Debug, PartialEq)]
pub enum SplineError {
    /// Degree is zero or past [`MAX_DEGREE`].
    BadDegree(usize),
    /// Control point count is `<= degree` or past [`MAX_POINTS`].
    BadPointCount(usize),
    /// Weight lane length differs from the control point count.
    MismatchedWeights {
        /// Control points given.
        points: usize,
        /// Weights given.
        weights: usize,
    },
    /// A weight is not finite and positive.
    BadWeight(usize),
    /// A coordinate is not finite.
    NonFinite,
    /// Knot count is not `points + degree + 1`, or knots decrease, or the
    /// domain is empty.
    BadKnots,
    /// Sample count is under two or past [`MAX_SAMPLES`].
    BadSampleCount(usize),
    /// Profile has fewer than three points or zero area.
    BadProfile,
}

impl fmt::Display for SplineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadDegree(d) => write!(f, "spline: degree {d} outside 1..={MAX_DEGREE}"),
            Self::BadPointCount(n) => write!(f, "spline: {n} control points refused"),
            Self::MismatchedWeights { points, weights } => {
                write!(f, "spline: {points} points but {weights} weights")
            }
            Self::BadWeight(i) => write!(f, "spline: weight {i} not finite and positive"),
            Self::NonFinite => write!(f, "spline: non-finite coordinate"),
            Self::BadKnots => write!(f, "spline: knot vector refused"),
            Self::BadSampleCount(n) => write!(f, "spline: sample count {n} refused"),
            Self::BadProfile => write!(f, "spline: profile needs three points and area"),
        }
    }
}

impl std::error::Error for SplineError {}

type V3 = [f64; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn axpy(a: f64, x: V3, y: V3) -> V3 {
    [a * x[0] + y[0], a * x[1] + y[1], a * x[2] + y[2]]
}

fn unit(a: V3) -> V3 {
    let l = dot(a, a).sqrt();
    if l > 0.0 {
        [a[0] / l, a[1] / l, a[2] / l]
    } else {
        [0.0; 3]
    }
}

fn dist(a: V3, b: V3) -> f64 {
    let d = sub(a, b);
    dot(d, d).sqrt()
}

/// A rational B-spline curve in 3D.
#[derive(Clone, Debug, PartialEq)]
pub struct Nurbs {
    degree: usize,
    /// Homogeneous control points `[w x, w y, w z, w]`.
    ctrl: Vec<[f64; 4]>,
    knots: Vec<f64>,
}

impl Nurbs {
    /// Curve from control points, positive weights and a knot vector of
    /// `points + degree + 1` non-decreasing values with a non-empty domain.
    /// Errors: [`SplineError`].
    pub fn new(
        degree: usize,
        points: &[V3],
        weights: &[f64],
        knots: Vec<f64>,
    ) -> Result<Self, SplineError> {
        if degree == 0 || degree > MAX_DEGREE {
            return Err(SplineError::BadDegree(degree));
        }
        if points.len() <= degree || points.len() > MAX_POINTS {
            return Err(SplineError::BadPointCount(points.len()));
        }
        if weights.len() != points.len() {
            return Err(SplineError::MismatchedWeights {
                points: points.len(),
                weights: weights.len(),
            });
        }
        if let Some(i) = weights.iter().position(|w| !(w.is_finite() && *w > 0.0)) {
            return Err(SplineError::BadWeight(i));
        }
        if points.iter().flatten().any(|c| !c.is_finite()) {
            return Err(SplineError::NonFinite);
        }
        let n = points.len();
        let sorted = knots.windows(2).all(|k| k[0] <= k[1]);
        if knots.len() != n + degree + 1 || !sorted || knots.iter().any(|k| !k.is_finite()) {
            return Err(SplineError::BadKnots);
        }
        if knots[degree] >= knots[n] {
            return Err(SplineError::BadKnots);
        }
        let ctrl = points
            .iter()
            .zip(weights)
            .map(|(p, w)| [p[0] * w, p[1] * w, p[2] * w, *w])
            .collect();
        Ok(Self {
            degree,
            ctrl,
            knots,
        })
    }

    /// Curve with a clamped uniform knot vector on `0..=1`: it starts and
    /// ends on the first and last control points.
    /// Errors: [`SplineError`].
    pub fn clamped(degree: usize, points: &[V3], weights: &[f64]) -> Result<Self, SplineError> {
        let spans = points.len().saturating_sub(degree);
        let mut knots = vec![0.0; degree + 1];
        knots.extend((1..spans).map(|i| i as f64 / spans as f64));
        knots.extend(vec![1.0; degree + 1]);
        Self::new(degree, points, weights, knots)
    }

    /// The parameter interval the curve is defined on.
    pub fn domain(&self) -> (f64, f64) {
        (self.knots[self.degree], self.knots[self.ctrl.len()])
    }

    /// Point at parameter `u` (clamped into the domain).
    pub fn point(&self, u: f64) -> V3 {
        let h = de_boor(self.degree, &self.knots, &self.ctrl, self.clamp(u));
        [h[0] / h[3], h[1] / h[3], h[2] / h[3]]
    }

    /// Analytic derivative `dC/du` at `u`: `(A' - w' C) / w` over the
    /// homogeneous derivative curve. Never allocates: the derivative curve is
    /// evaluated in place, so one tangent costs `O(degree^2)` on the stack.
    pub fn tangent(&self, u: f64) -> V3 {
        let u = self.clamp(u);
        let h = de_boor(self.degree, &self.knots, &self.ctrl, u);
        let d = derivative_de_boor(self.degree, &self.knots, &self.ctrl, u);
        let c = [h[0] / h[3], h[1] / h[3], h[2] / h[3]];
        [
            (d[0] - d[3] * c[0]) / h[3],
            (d[1] - d[3] * c[1]) / h[3],
            (d[2] - d[3] * c[2]) / h[3],
        ]
    }

    fn clamp(&self, u: f64) -> f64 {
        let (lo, hi) = self.domain();
        u.clamp(lo, hi)
    }

    /// Arc-length table: cumulative chord lengths over `samples` even
    /// parameter steps.
    /// Errors: [`SplineError::BadSampleCount`].
    pub fn arc_table(&self, samples: usize) -> Result<ArcTable, SplineError> {
        if !(2..=MAX_SAMPLES).contains(&samples) {
            return Err(SplineError::BadSampleCount(samples));
        }
        let (lo, hi) = self.domain();
        let us: Vec<f64> = (0..samples)
            .map(|i| lo + (hi - lo) * i as f64 / (samples - 1) as f64)
            .collect();
        let mut lens = vec![0.0; samples];
        let mut prev = self.point(lo);
        for i in 1..samples {
            let p = self.point(us[i]);
            lens[i] = lens[i - 1] + dist(prev, p);
            prev = p;
        }
        Ok(ArcTable { us, lens })
    }

    /// Parameter of the curve point nearest to `p`: the closest of
    /// [`TABLE_SAMPLES`] samples, refined by interval shrinking.
    pub fn nearest(&self, p: V3) -> f64 {
        let (lo, hi) = self.domain();
        let step = (hi - lo) / (TABLE_SAMPLES - 1) as f64;
        let d2 = |u: f64| {
            let d = sub(self.point(u), p);
            dot(d, d)
        };
        let best = (0..TABLE_SAMPLES)
            .map(|i| lo + step * i as f64)
            .fold((f64::INFINITY, lo), |(bd, bu), u| {
                let d = d2(u);
                if d < bd {
                    (d, u)
                } else {
                    (bd, bu)
                }
            })
            .1;
        let (mut a, mut b) = ((best - step).max(lo), (best + step).min(hi));
        for _ in 0..48 {
            let (m1, m2) = (a + (b - a) / 3.0, b - (b - a) / 3.0);
            if d2(m1) <= d2(m2) {
                b = m2;
            } else {
                a = m1;
            }
        }
        a + (b - a) / 2.0
    }

    /// `count` frames at even arc-length steps from the start to the end,
    /// each with the unit tangent and a transported normal that never twists.
    /// The first normal is the tangent crossed with the axis it is least
    /// aligned with, so equal curves start equal.
    /// Errors: [`SplineError::BadSampleCount`].
    pub fn frames(&self, count: usize) -> Result<Vec<Frame>, SplineError> {
        if !(2..=MAX_SAMPLES).contains(&count) {
            return Err(SplineError::BadSampleCount(count));
        }
        let table = self.arc_table(TABLE_SAMPLES)?;
        let total = table.length();
        let mut frames: Vec<Frame> = Vec::with_capacity(count);
        for i in 0..count {
            let u = table.u_at(total * i as f64 / (count - 1) as f64);
            let (pos, tangent) = (self.point(u), unit(self.tangent(u)));
            let normal = match frames.last() {
                None => seed_normal(tangent),
                Some(prev) => transport(prev, pos, tangent),
            };
            frames.push(Frame {
                pos,
                tangent,
                normal,
                binormal: cross(tangent, normal),
            });
        }
        Ok(frames)
    }
}

/// Index `k` with `knots[k] <= u < knots[k + 1]`, the last real span at the end.
fn find_span(degree: usize, knots: &[f64], n: usize, u: f64) -> usize {
    if u >= knots[n] {
        return n - 1;
    }
    let (mut lo, mut hi) = (degree, n);
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if u < knots[mid] {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    lo
}

/// De Boor evaluation of a homogeneous B-spline curve at `u`.
fn de_boor(degree: usize, knots: &[f64], ctrl: &[[f64; 4]], u: f64) -> [f64; 4] {
    let k = find_span(degree, knots, ctrl.len(), u);
    let mut d = [[0.0; 4]; MAX_DEGREE + 1];
    d[..=degree].copy_from_slice(&ctrl[k - degree..=k]);
    for r in 1..=degree {
        for j in (r..=degree).rev() {
            let i = j + k - degree;
            let span = knots[i + degree + 1 - r] - knots[i];
            let alpha = if span > 0.0 {
                (u - knots[i]) / span
            } else {
                0.0
            };
            d[j] = [0, 1, 2, 3].map(|c| (1.0 - alpha) * d[j - 1][c] + alpha * d[j][c]);
        }
    }
    d[degree]
}

/// De Boor evaluation of the derivative curve (degree - 1) at `u`, without
/// materialising its control points: only the `degree` points around the span
/// are built, so a tangent never allocates. The derivative knot vector is the
/// open window `knots[1..len - 1]`, a slice and never a copy.
fn derivative_de_boor(degree: usize, knots: &[f64], ctrl: &[[f64; 4]], u: f64) -> [f64; 4] {
    let d_degree = degree - 1;
    let d_knots = &knots[1..knots.len() - 1];
    let last = ctrl.len() - 1;
    let k = find_span(d_degree, d_knots, last, u);
    let mut d = [[0.0; 4]; MAX_DEGREE + 1];
    for (j, slot) in d.iter_mut().enumerate().take(d_degree + 1) {
        *slot = derivative_ctrl(degree, knots, ctrl, k - d_degree + j);
    }
    for r in 1..=d_degree {
        for j in (r..=d_degree).rev() {
            let i = j + k - d_degree;
            let span = d_knots[i + d_degree + 1 - r] - d_knots[i];
            let alpha = if span > 0.0 {
                (u - d_knots[i]) / span
            } else {
                0.0
            };
            d[j] = [0, 1, 2, 3].map(|c| (1.0 - alpha) * d[j - 1][c] + alpha * d[j][c]);
        }
    }
    d[d_degree]
}

/// One control point of the derivative curve at index `i`: the value the
/// previous `derivative` builder wrote for that slot.
fn derivative_ctrl(degree: usize, knots: &[f64], ctrl: &[[f64; 4]], i: usize) -> [f64; 4] {
    let span = knots[i + degree + 1] - knots[i + 1];
    let f = if span > 0.0 {
        degree as f64 / span
    } else {
        0.0
    };
    [0, 1, 2, 3].map(|c| f * (ctrl[i + 1][c] - ctrl[i][c]))
}

/// Cumulative arc-length table of a curve.
#[derive(Clone, Debug, PartialEq)]
pub struct ArcTable {
    us: Vec<f64>,
    lens: Vec<f64>,
}

impl ArcTable {
    /// Total length of the sampled curve.
    pub fn length(&self) -> f64 {
        self.lens[self.lens.len() - 1]
    }

    /// Parameter at arc length `s` from the start (clamped to the curve).
    pub fn u_at(&self, s: f64) -> f64 {
        let s = s.clamp(0.0, self.length());
        let hi = self
            .lens
            .partition_point(|l| *l < s)
            .clamp(1, self.lens.len() - 1);
        let (l0, l1) = (self.lens[hi - 1], self.lens[hi]);
        let t = if l1 > l0 { (s - l0) / (l1 - l0) } else { 0.0 };
        self.us[hi - 1] + t * (self.us[hi] - self.us[hi - 1])
    }
}

/// One sweep frame: position, unit tangent and an orthonormal normal pair.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// Curve point.
    pub pos: [f64; 3],
    /// Unit tangent.
    pub tangent: [f64; 3],
    /// Unit normal, transported from the previous frame.
    pub normal: [f64; 3],
    /// Unit binormal, `tangent x normal`.
    pub binormal: [f64; 3],
}

/// Unit vector perpendicular to `t`, built from the axis `t` leans on least.
fn seed_normal(t: V3) -> V3 {
    let mut axis = 0;
    for i in 1..3 {
        if t[i].abs() < t[axis].abs() {
            axis = i;
        }
    }
    let mut e = [0.0; 3];
    e[axis] = 1.0;
    unit(cross(t, e))
}

/// Double-reflection step (Wang et al. 2008): carries `prev.normal` to the
/// next point without twisting, exact to second order.
fn transport(prev: &Frame, pos: V3, tangent: V3) -> V3 {
    let v1 = sub(pos, prev.pos);
    let c1 = dot(v1, v1);
    if c1 == 0.0 {
        return prev.normal;
    }
    let r_l = axpy(-2.0 / c1 * dot(v1, prev.normal), v1, prev.normal);
    let t_l = axpy(-2.0 / c1 * dot(v1, prev.tangent), v1, prev.tangent);
    let v2 = sub(tangent, t_l);
    let c2 = dot(v2, v2);
    if c2 == 0.0 {
        return unit(r_l);
    }
    unit(axpy(-2.0 / c2 * dot(v2, r_l), v2, r_l))
}

/// Triangulates a simple polygon by ear clipping; indices refer to `poly`
/// and wind counter-clockwise whichever way the input winds.
/// Errors: [`SplineError::BadProfile`].
pub fn ear_clip(poly: &[[f64; 2]]) -> Result<Vec<[u32; 3]>, SplineError> {
    let area2: f64 = (0..poly.len())
        .map(|i| {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            a[0] * b[1] - a[1] * b[0]
        })
        .sum();
    if poly.len() < 3 || poly.len() > MAX_SAMPLES || !area2.is_finite() || area2 == 0.0 {
        return Err(SplineError::BadProfile);
    }
    let mut ring: Vec<usize> = (0..poly.len()).collect();
    if area2 < 0.0 {
        ring.reverse();
    }
    let turn = |a: usize, b: usize, c: usize| {
        (poly[b][0] - poly[a][0]) * (poly[c][1] - poly[a][1])
            - (poly[b][1] - poly[a][1]) * (poly[c][0] - poly[a][0])
    };
    let mut tris = Vec::with_capacity(poly.len() - 2);
    while ring.len() > 3 {
        let n = ring.len();
        let ear = (0..n).find(|&i| {
            let (a, b, c) = (ring[(i + n - 1) % n], ring[i], ring[(i + 1) % n]);
            turn(a, b, c) > 0.0
                && !ring
                    .iter()
                    .filter(|&&p| p != a && p != b && p != c)
                    .any(|&p| turn(a, b, p) >= 0.0 && turn(b, c, p) >= 0.0 && turn(c, a, p) >= 0.0)
        });
        // A simple polygon always has an ear; degenerate input clips what it can.
        let i = ear.unwrap_or(0);
        tris.push([
            ring[(i + n - 1) % n] as u32,
            ring[i] as u32,
            ring[(i + 1) % n] as u32,
        ]);
        ring.remove(i);
    }
    tris.push([ring[0] as u32, ring[1] as u32, ring[2] as u32]);
    Ok(tris)
}

/// A swept solid: ring vertices and triangles.
#[derive(Clone, Debug, PartialEq)]
pub struct Mesh {
    /// Vertex positions, ring after ring.
    pub positions: Vec<[f64; 3]>,
    /// Triangle indices into `positions`.
    pub indices: Vec<[u32; 3]>,
}

/// Sweeps a closed 2D `profile` (x along the normal, y along the binormal)
/// through `frames`, with ear-clipped caps at both ends.
/// Errors: [`SplineError::BadProfile`].
pub fn sweep(profile: &[[f64; 2]], frames: &[Frame]) -> Result<Mesh, SplineError> {
    let caps = ear_clip(profile)?;
    let m = profile.len() as u32;
    let mut positions = Vec::with_capacity(profile.len() * frames.len());
    for f in frames {
        for p in profile {
            positions.push([0, 1, 2].map(|c| f.pos[c] + f.normal[c] * p[0] + f.binormal[c] * p[1]));
        }
    }
    let mut indices = Vec::new();
    for ring in 0..frames.len().saturating_sub(1) as u32 {
        for i in 0..m {
            let (a, b) = (ring * m + i, ring * m + (i + 1) % m);
            indices.push([a, b, a + m]);
            indices.push([b, b + m, a + m]);
        }
    }
    let last = (frames.len().saturating_sub(1) as u32) * m;
    for t in &caps {
        indices.push([t[0], t[2], t[1]]);
        indices.push([last + t[0], last + t[1], last + t[2]]);
    }
    Ok(Mesh { positions, indices })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT_HALF: f64 = std::f64::consts::FRAC_1_SQRT_2;

    /// Quarter of the unit circle in the XY plane as one rational quadratic
    /// Bezier span: exact, so every answer has a closed-form reference.
    fn quarter() -> Nurbs {
        Nurbs::clamped(
            2,
            &[[1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
            &[1.0, ROOT_HALF, 1.0],
        )
        .expect("quarter circle")
    }

    fn near(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() <= tol, "{a} vs {b}");
    }

    /// The pre-change tangent: it built the whole derivative control array in
    /// a heap vector on every call. Kept as the equality oracle and the A/B
    /// reference for the stack-only [`Nurbs::tangent`].
    fn tangent_allocating(curve: &Nurbs, u: f64) -> V3 {
        let u = curve.clamp(u);
        let h = de_boor(curve.degree, &curve.knots, &curve.ctrl, u);
        let q: Vec<[f64; 4]> = (0..curve.ctrl.len() - 1)
            .map(|i| derivative_ctrl(curve.degree, &curve.knots, &curve.ctrl, i))
            .collect();
        let d_knots = &curve.knots[1..curve.knots.len() - 1];
        let d = de_boor(curve.degree - 1, d_knots, &q, u);
        let c = [h[0] / h[3], h[1] / h[3], h[2] / h[3]];
        [
            (d[0] - d[3] * c[0]) / h[3],
            (d[1] - d[3] * c[1]) / h[3],
            (d[2] - d[3] * c[2]) / h[3],
        ]
    }

    /// Shared fixture: a 64-control-point clamped cubic, the rail a content
    /// tool bakes. Big enough that the old path's `Vec` dwarfs the evaluation.
    fn rib() -> Nurbs {
        let mut points: Vec<V3> = Vec::with_capacity(64);
        let mut t = 0.0;
        while points.len() < 64 {
            points.push([t, (t * 0.25).sqrt() - 1.0, t * t * 0.01]);
            t += 0.5;
        }
        Nurbs::clamped(3, &points, &[1.0; 64]).expect("rib curve")
    }

    /// The stack tangent equals the allocating oracle bit for bit on one
    /// 64-point fixture, across the domain and past both clamped ends.
    #[test]
    fn stack_tangent_equals_the_allocating_oracle_bit_for_bit() {
        let curve = rib();
        for step in 0u32..=64 {
            let u = f64::from(step) / 32.0 - 0.5;
            assert_eq!(
                curve.tangent(u),
                tangent_allocating(&curve, u),
                "u {u}: the stack tangent moved a bit"
            );
        }
    }

    /// Tangent receipt: p50 microseconds of one 64-sample tangent scan on the
    /// 64-point [`rib`], stack-only [`Nurbs::tangent`] against the allocating
    /// oracle, interleaved so the same load sits under both sides. The stack
    /// path stays only while the pair holds and the stack side wins.
    /// Run: cargo test -p engine-nav --lib --release spline -- --ignored --nocapture
    #[test]
    #[ignore = "timing: run with --ignored (release, one thread) for the receipt"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "timing test reads the wall clock; it never enters the tick"
    )]
    fn spline_tangent_bench_receipt() {
        const SAMPLES: u32 = 64;
        const ITERS: usize = 200;
        let curve = rib();
        let us: Vec<f64> = (0..SAMPLES)
            .map(|step| f64::from(step) / f64::from(SAMPLES - 1))
            .collect();
        let want: Vec<V3> = us.iter().map(|u| tangent_allocating(&curve, *u)).collect();
        let got: Vec<V3> = us.iter().map(|u| curve.tangent(*u)).collect();
        assert_eq!(got, want, "the two timed paths disagree");
        let mut old_runs = Vec::with_capacity(ITERS);
        let mut new_runs = Vec::with_capacity(ITERS);
        for _ in 0..ITERS {
            let begin = std::time::Instant::now();
            for u in &us {
                std::hint::black_box(tangent_allocating(&curve, *u));
            }
            old_runs.push(begin.elapsed().as_micros());
            let begin = std::time::Instant::now();
            for u in &us {
                std::hint::black_box(curve.tangent(*u));
            }
            new_runs.push(begin.elapsed().as_micros());
        }
        old_runs.sort_unstable();
        new_runs.sort_unstable();
        let (old, new) = (old_runs[ITERS / 2], new_runs[ITERS / 2]);
        eprintln!(
            "spline_tangent stack_p50_us={new} allocating_p50_us={old} speedup_x100={} points=64 samples={SAMPLES}",
            old.saturating_mul(100).checked_div(new).unwrap_or(0)
        );
        assert!(new < old, "the stack tangent must not be the slower one");
    }

    /// Reference point and unit tangent of the quarter circle by angle.
    #[expect(
        clippy::disallowed_methods,
        reason = "test reference values only; the spline itself uses no trig"
    )]
    fn circle_at(theta: f64) -> (V3, V3) {
        (
            [theta.cos(), theta.sin(), 0.0],
            [-theta.sin(), theta.cos(), 0.0],
        )
    }

    #[test]
    fn spline_frames_points_and_tangents_match_the_circle() {
        let c = quarter();
        for i in 0..=20 {
            let u = f64::from(i) / 20.0;
            let p = c.point(u);
            // On the unit circle, and the tangent is perpendicular to the radius.
            near(dot(p, p), 1.0, 1e-12);
            let t = c.tangent(u);
            near(dot(p, t), 0.0, 1e-12);
            assert!(t[2] == 0.0 && p[2] == 0.0);
        }
        assert_eq!(c.point(0.0), [1.0, 0.0, 0.0]);
        near(c.point(1.0)[1], 1.0, 1e-15);
        // Tangent at the ends: speed 2 * w1/w0 * |P1 - P0| along the leg.
        let t0 = c.tangent(0.0);
        near(t0[1], 2.0 * ROOT_HALF, 1e-12);
        near(t0[0], 0.0, 1e-12);
    }

    #[test]
    fn spline_frames_arc_length_and_nearest_match_the_reference() {
        let c = quarter();
        let table = c.arc_table(1024).expect("table");
        near(table.length(), std::f64::consts::FRAC_PI_2, 1e-5);
        // Half the arc length is the 45 degree point.
        let (want, _) = circle_at(std::f64::consts::FRAC_PI_4);
        let got = c.point(table.u_at(table.length() / 2.0));
        near(got[0], want[0], 1e-5);
        near(got[1], want[1], 1e-5);
        // A point off the arc at 30 degrees projects back onto it.
        let (on, _) = circle_at(std::f64::consts::FRAC_PI_6);
        let u = c.nearest([on[0] * 2.5, on[1] * 2.5, 0.7]);
        let hit = c.point(u);
        near(hit[0], on[0], 1e-6);
        near(hit[1], on[1], 1e-6);
        // Past the ends the nearest parameter clamps.
        near(c.nearest([5.0, -3.0, 0.0]), 0.0, 1e-9);
        assert_eq!(c.arc_table(1), Err(SplineError::BadSampleCount(1)));
    }

    #[test]
    fn spline_frames_are_orthonormal_and_twist_free() {
        let c = quarter();
        let frames = c.frames(33).expect("frames");
        let table = c.arc_table(1024).expect("table");
        for (i, f) in frames.iter().enumerate() {
            near(dot(f.tangent, f.tangent), 1.0, 1e-12);
            near(dot(f.normal, f.normal), 1.0, 1e-12);
            near(dot(f.tangent, f.normal), 0.0, 1e-12);
            near(dot(f.binormal, f.binormal), 1.0, 1e-12);
            // A planar curve keeps its normal in the plane's perpendicular:
            // the seed normal is -z and transport never turns it.
            near(f.normal[2], -1.0, 1e-9);
            // Frames sit at even arc length.
            let theta = std::f64::consts::FRAC_PI_2 * i as f64 / 32.0;
            let (p, t) = circle_at(theta);
            near(f.pos[0], p[0], 1e-4);
            near(f.pos[1], p[1], 1e-4);
            near(f.tangent[0], t[0], 1e-4);
            near(f.tangent[1], t[1], 1e-4);
        }
        near(table.length(), std::f64::consts::FRAC_PI_2, 1e-5);
    }

    /// A helix-like cubic bends in all three axes; the transported frame
    /// still ends orthonormal, and a rerun matches bit for bit.
    #[test]
    fn spline_frames_space_curve_is_orthonormal_and_replays() {
        let pts = [
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 1.0],
            [2.0, 2.0, 2.0],
            [0.0, 2.0, 3.0],
            [0.0, 0.0, 4.0],
        ];
        let c = Nurbs::clamped(3, &pts, &[1.0, 2.0, 0.5, 1.0, 1.0]).expect("curve");
        let run = || c.frames(64).expect("frames");
        let frames = run();
        assert_eq!(frames, run(), "x2 identical");
        let last = frames[63];
        near(dot(last.tangent, last.normal), 0.0, 1e-9);
        near(dot(last.normal, last.normal), 1.0, 1e-9);
        // Open clamped ends land on the end control points.
        assert_eq!(frames[0].pos, pts[0]);
        for (a, b) in frames[63].pos.iter().zip(pts[4]) {
            near(*a, b, 1e-9);
        }
    }

    #[test]
    fn spline_frames_bad_curves_refuse() {
        let p = [[0.0; 3], [1.0, 0.0, 0.0], [2.0, 1.0, 0.0]];
        assert_eq!(
            Nurbs::clamped(0, &p, &[1.0; 3]),
            Err(SplineError::BadDegree(0))
        );
        assert_eq!(
            Nurbs::clamped(3, &p, &[1.0; 3]),
            Err(SplineError::BadPointCount(3))
        );
        assert_eq!(
            Nurbs::clamped(2, &p, &[1.0; 2]),
            Err(SplineError::MismatchedWeights {
                points: 3,
                weights: 2
            })
        );
        assert_eq!(
            Nurbs::clamped(2, &p, &[1.0, 0.0, 1.0]),
            Err(SplineError::BadWeight(1))
        );
        assert_eq!(
            Nurbs::new(2, &p, &[1.0; 3], vec![0.0, 0.0, 0.0, 1.0, 0.5, 1.0]),
            Err(SplineError::BadKnots)
        );
        let mut bad = p;
        bad[1][0] = f64::NAN;
        assert_eq!(
            Nurbs::clamped(2, &bad, &[1.0; 3]),
            Err(SplineError::NonFinite)
        );
    }

    #[test]
    fn spline_frames_ear_clip_covers_the_polygon_once() {
        // An L shape, wound both ways: 4 triangles of total area 3.
        let l = [
            [0.0, 0.0],
            [2.0, 0.0],
            [2.0, 1.0],
            [1.0, 1.0],
            [1.0, 2.0],
            [0.0, 2.0],
        ];
        for poly in [l.to_vec(), l.iter().rev().copied().collect()] {
            let tris = ear_clip(&poly).expect("clips");
            assert_eq!(tris.len(), 4);
            let area: f64 = tris
                .iter()
                .map(|t| {
                    let [a, b, c] = t.map(|i| poly[i as usize]);
                    ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])) / 2.0
                })
                .sum();
            near(area, 3.0, 1e-12);
        }
        assert_eq!(ear_clip(&l[..2]), Err(SplineError::BadProfile));
    }

    #[test]
    fn spline_frames_sweep_builds_a_closed_tube() {
        let frames = quarter().frames(9).expect("frames");
        let square = [[-0.1, -0.1], [0.1, -0.1], [0.1, 0.1], [-0.1, 0.1]];
        let mesh = sweep(&square, &frames).expect("sweeps");
        assert_eq!(mesh.positions.len(), 4 * 9);
        // 8 spans x 4 sides x 2 triangles, plus 2 triangles per cap.
        assert_eq!(mesh.indices.len(), 8 * 4 * 2 + 2 * 2);
        assert!(mesh
            .indices
            .iter()
            .flatten()
            .all(|i| (*i as usize) < mesh.positions.len()));
        // Every ring vertex sits the profile distance from its frame.
        for (ring, f) in frames.iter().enumerate() {
            for i in 0..4 {
                near(
                    dist(mesh.positions[ring * 4 + i], f.pos),
                    0.02_f64.sqrt(),
                    1e-12,
                );
            }
        }
    }
}
