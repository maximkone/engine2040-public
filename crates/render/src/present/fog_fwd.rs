//! fog_fwd: unified height fog evaluated per fragment in the forward shader.
//!
//! Owner: render lane, consumer the windowed presenter's live frame. Units:
//! meters for positions and the fog plane, `1/m^2` for the density slope,
//! linear HDR radiance triples for colors (the forward target is linear; the
//! blit tone-maps once, so fog acts before it). Fog is a function of the view
//! vector and a fog plane, not a post pass over a read-back depth image
//! (Game Engine Gems 3 ch 3.2, Lengyel Foundations vol 2 sec 8.5.2). The
//! density is zero above the plane and grows linearly with depth below it,
//! so the optical depth along any segment has a closed form with no loop
//! and no texture. The fog color is the sky horizon color; sky pixels use
//! the infinite-distance form (Game Engine Gems 1 ch 16.1, 16.3).
//! Determinism: pure f32 arithmetic; the CPU twin below and the fog functions
//! in `shaders/forward.wgsl` evaluate the same expression, pinned by the unit
//! and device tests.
//! Errors: none (inputs are finite by construction; a zero density switches
//! the effect off exactly). Budget: one 128 byte uniform write per frame,
//! about 25 ALU ops per fragment.

/// Bytes of the packed fog uniform the forward and sky entry points read
/// (`FogFwdU` in `shaders/forward.wgsl`, std140 friendly: inverse
/// view-projection, eye plus density, fog color plus sky flag, zenith color
/// plus pad, fog plane).
pub const FOG_FWD_UNIFORM_BYTES: usize = 128;

/// Fog top above the ground in meters: the haze fades to nothing here.
pub const FOG_FWD_TOP_M: f32 = 8.0;
/// Fog density at the ground plane in `1/m` (the day haze the live frame
/// has always carried).
pub const FOG_FWD_GROUND_DENSITY_PER_M: f32 = 0.02;

/// Optical depth cap before `exp` underflows to a clean 1.0 factor.
const TAU_CAP: f32 = 80.0;
/// Smallest plane-normal dot at which the infinite form is finite.
const FD_EPS: f32 = 1.0e-5;

/// Fog parameters for one presenter: the plane, the density slope and the
/// two sky colors, all fixed at init from the look bake.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FogFwd {
    /// Fog color (the sky horizon) as linear radiance.
    pub color: [f32; 3],
    /// Sky color straight up, linear radiance.
    pub zenith: [f32; 3],
    /// Density slope in `1/m^2`: density at depth `h` below the plane is
    /// `slope * h`. Zero switches the fog off exactly.
    pub slope_per_m2: f32,
    /// Fog plane `(a, b, c, d)`; `a x + b y + c z + d < 0` is fogged.
    pub plane: [f32; 4],
    /// Live-sky switch the truth gate reads: true samples the bake.
    pub sky_on: bool,
}

impl FogFwd {
    /// Day fog over the ground: horizon and zenith radiance (linear) from
    /// the look bake, a horizontal plane `FOG_FWD_TOP_M` up, and the ground
    /// density the live frame has always carried.
    #[must_use]
    pub fn day(horizon_linear: [f32; 3], zenith_linear: [f32; 3]) -> Self {
        Self {
            color: radiance(horizon_linear),
            zenith: radiance(zenith_linear),
            slope_per_m2: FOG_FWD_GROUND_DENSITY_PER_M / FOG_FWD_TOP_M,
            plane: [0.0, 1.0, 0.0, -FOG_FWD_TOP_M],
            sky_on: true,
        }
    }

    /// Sky switch: false collapses the sky to the fog color.
    #[must_use]
    pub fn with_sky_on(self, on: bool) -> Self {
        Self { sky_on: on, ..self }
    }

    /// Current sky switch.
    pub fn sky_on(self) -> bool {
        self.sky_on
    }

    /// Same colors with the fog switched off (density slope zero).
    #[must_use]
    pub fn off(self) -> Self {
        Self {
            slope_per_m2: 0.0,
            ..self
        }
    }

    /// Pack the 128 byte uniform: inverse view-projection (column major),
    /// eye, density slope, fog color, zenith color, fog plane.
    #[must_use]
    pub fn to_bytes(
        &self,
        inv_view_proj: &[f32; 16],
        eye: [f32; 3],
    ) -> [u8; FOG_FWD_UNIFORM_BYTES] {
        let mut words = [0.0f32; 32];
        words[..16].copy_from_slice(inv_view_proj);
        words[16..19].copy_from_slice(&eye);
        words[19] = self.slope_per_m2;
        words[20..23].copy_from_slice(&self.color);
        words[23] = if self.sky_on { 1.0 } else { 0.0 };
        words[24..27].copy_from_slice(&self.zenith);
        words[28..32].copy_from_slice(&self.plane);
        let mut out = [0u8; FOG_FWD_UNIFORM_BYTES];
        for (chunk, word) in out.chunks_exact_mut(4).zip(words.iter()) {
            chunk.copy_from_slice(&word.to_le_bytes());
        }
        out
    }

    /// Signed plane distance of a point (negative is inside the fog).
    fn plane_distance(&self, p: [f32; 3]) -> f32 {
        self.plane[0] * p[0] + self.plane[1] * p[1] + self.plane[2] * p[2] + self.plane[3]
    }

    /// CPU twin of `fog_factor_finite`: fog factor for the segment from
    /// `eye` to the surface point `p`, in `0..=1`.
    #[must_use]
    pub fn factor(&self, eye: [f32; 3], p: [f32; 3]) -> f32 {
        let v = [p[0] - eye[0], p[1] - eye[1], p[2] - eye[2]];
        let d = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        let fc = self.plane_distance(eye);
        let fp = self.plane_distance(p);
        let nc = (-fc).max(0.0);
        let np = (-fp).max(0.0);
        let span = (fc - fp).abs().max(1.0e-4);
        let straddle = (nc * nc + np * np) / (2.0 * span);
        let inside = 0.5 * (nc + np);
        let g = if fc <= 0.0 && fp <= 0.0 {
            inside
        } else {
            straddle
        };
        let tau = (self.slope_per_m2 * d * g).min(TAU_CAP);
        1.0 - (-tau).exp()
    }

    /// CPU twin of `fog_factor_infinite`: fog factor along the unit view
    /// direction `dir` out to infinity.
    #[must_use]
    pub fn factor_infinite(&self, eye: [f32; 3], dir: [f32; 3]) -> f32 {
        let fc = self.plane_distance(eye);
        let fd = self.plane[0] * dir[0] + self.plane[1] * dir[1] + self.plane[2] * dir[2];
        let in_fog_up = fc <= 0.0 && fd > FD_EPS;
        let never_leaves = (fc <= 0.0 && !in_fog_up) || (fc > 0.0 && fd < -FD_EPS);
        let g = if in_fog_up {
            fc * fc / (2.0 * fd.max(FD_EPS))
        } else if never_leaves {
            80_000.0
        } else {
            0.0
        };
        let tau = (self.slope_per_m2 * g).min(TAU_CAP);
        1.0 - (-tau).exp()
    }

    /// CPU twin of the sky fragment: the sky color along `dir` for an eye
    /// at `eye`, as linear radiance (the blit tone-maps it).
    #[must_use]
    pub fn sky_color(&self, eye: [f32; 3], dir: [f32; 3]) -> [f32; 3] {
        if !self.sky_on {
            return self.color;
        }
        let t = dir[1].clamp(0.0, 1.0).sqrt();
        let f = self.factor_infinite(eye, dir);
        let mut out = [0.0f32; 3];
        for (c, slot) in out.iter_mut().enumerate() {
            let base = self.color[c] + (self.zenith[c] - self.color[c]) * t;
            *slot = base + (self.color[c] - base) * f;
        }
        out
    }
}

/// Radiance of a bake sample: finite and non-negative, HDR values kept (the
/// forward target is Rgba16Float, so nothing clamps to 1 before the blit).
fn radiance(linear: [f32; 3]) -> [f32; 3] {
    linear.map(|c| if c.is_finite() { c.max(0.0) } else { 0.0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fog() -> FogFwd {
        FogFwd::day([0.6, 0.7, 0.8], [0.1, 0.3, 0.7])
    }

    /// Reference: midpoint integration of the density along the segment.
    fn numeric_factor(fog: &FogFwd, eye: [f32; 3], p: [f32; 3]) -> f32 {
        let steps = 20_000;
        let mut tau = 0.0f64;
        let len = ((p[0] - eye[0]).powi(2) + (p[1] - eye[1]).powi(2) + (p[2] - eye[2]).powi(2))
            .sqrt() as f64;
        for i in 0..steps {
            let s = (f64::from(i) + 0.5) / f64::from(steps);
            let q = [
                eye[0] + (p[0] - eye[0]) * s as f32,
                eye[1] + (p[1] - eye[1]) * s as f32,
                eye[2] + (p[2] - eye[2]) * s as f32,
            ];
            let h = f64::from(fog.plane_distance(q));
            tau += f64::from(fog.slope_per_m2) * (-h).max(0.0) * len / f64::from(steps);
        }
        (1.0 - (-tau).exp()) as f32
    }

    #[test]
    fn closed_form_matches_numeric_integration_in_every_case() {
        let f = fog();
        // Eye and point: both fogged, eye fogged and point clear, eye clear
        // and point fogged, both clear, and a ray that dips and rises.
        let cases: [([f32; 3], [f32; 3]); 6] = [
            ([0.0, 2.0, 0.0], [30.0, 1.0, 5.0]),
            ([0.0, 2.0, 0.0], [20.0, 14.0, 5.0]),
            ([0.0, 15.0, 0.0], [20.0, 0.5, 5.0]),
            ([0.0, 12.0, 0.0], [40.0, 20.0, 5.0]),
            ([0.0, 9.0, 0.0], [40.0, 9.0, 0.0]),
            ([3.0, 0.0, 3.0], [3.0, 0.0, 40.0]),
        ];
        for (eye, p) in cases {
            let got = f.factor(eye, p);
            let want = numeric_factor(&f, eye, p);
            assert!(
                (got - want).abs() < 2.0e-4,
                "eye {eye:?} point {p:?}: closed {got} numeric {want}"
            );
        }
    }

    #[test]
    fn clear_air_above_the_plane_has_no_fog_and_depth_grows_it() {
        let f = fog();
        assert_eq!(f.factor([0.0, 20.0, 0.0], [10.0, 15.0, 4.0]), 0.0);
        let near = f.factor([0.0, 2.0, 0.0], [10.0, 0.0, 0.0]);
        let far = f.factor([0.0, 2.0, 0.0], [60.0, 0.0, 0.0]);
        assert!(0.0 < near && near < far && far < 1.0, "{near} {far}");
        assert_eq!(f.off().factor([0.0, 2.0, 0.0], [60.0, 0.0, 0.0]), 0.0);
    }

    #[test]
    fn infinite_form_is_the_limit_of_the_finite_form() {
        let f = fog();
        // Eye in the fog looking up out of it: the finite factor at a very
        // far point converges to the closed infinite form.
        let eye = [0.0, 1.0, 0.0];
        let dir = {
            let v = [0.8f32, 0.3, 0.5];
            let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            [v[0] / l, v[1] / l, v[2] / l]
        };
        let far = [
            eye[0] + dir[0] * 1.0e6,
            eye[1] + dir[1] * 1.0e6,
            eye[2] + dir[2] * 1.0e6,
        ];
        let finite = f.factor(eye, far);
        let infinite = f.factor_infinite(eye, dir);
        assert!(
            (finite - infinite).abs() < 1.0e-3,
            "finite {finite} vs infinite {infinite}"
        );
        // Eye above the fog: a ray that goes up never sees it, a ray that
        // goes down is fully fogged at infinity.
        assert_eq!(f.factor_infinite([0.0, 20.0, 0.0], [0.0, 1.0, 0.0]), 0.0);
        assert!(f.factor_infinite([0.0, 20.0, 0.0], [0.0, -1.0, 0.0]) > 0.999);
        // Horizontal ray inside the fog never leaves it.
        assert!(f.factor_infinite([0.0, 1.0, 0.0], [1.0, 0.0, 0.0]) > 0.999);
    }

    #[test]
    fn uniform_bytes_pack_the_documented_offsets() {
        let f = fog();
        let mut inv = [0.0f32; 16];
        for (i, v) in inv.iter_mut().enumerate() {
            *v = i as f32;
        }
        let bytes = f.to_bytes(&inv, [1.0, 2.0, 3.0]);
        let word = |i: usize| {
            f32::from_le_bytes([
                bytes[i * 4],
                bytes[i * 4 + 1],
                bytes[i * 4 + 2],
                bytes[i * 4 + 3],
            ])
        };
        assert_eq!(word(0), 0.0);
        assert_eq!(word(15), 15.0);
        assert_eq!((word(16), word(17), word(18)), (1.0, 2.0, 3.0));
        assert_eq!(word(19), f.slope_per_m2);
        assert_eq!(word(20), f.color[0]);
        assert_eq!(word(24), f.zenith[0]);
        assert_eq!(
            (word(28), word(29), word(30), word(31)),
            (0.0, 1.0, 0.0, -FOG_FWD_TOP_M)
        );
        assert_eq!(bytes.len(), FOG_FWD_UNIFORM_BYTES);
    }

    #[test]
    fn sky_color_reaches_the_fog_color_where_the_ray_stays_fogged() {
        let f = fog();
        let fogged = f.sky_color([0.0, 1.0, 0.0], [1.0, 0.0, 0.0]);
        for c in 0..3 {
            assert!((fogged[c] - f.color[c]).abs() < 1.0e-4, "{fogged:?}");
        }
        // From above the fog the zenith shows its own color.
        let up = f.sky_color([0.0, 30.0, 0.0], [0.0, 1.0, 0.0]);
        for c in 0..3 {
            assert!((up[c] - f.zenith[c]).abs() < 1.0e-4, "{up:?}");
        }
    }

    #[test]
    fn uniform_bytes_carry_sky_on_off_flag() {
        let f = fog();
        let inv = [0.0f32; 16];
        let eye = [1.0, 2.0, 3.0];
        let word = |b: &[u8], i: usize| {
            f32::from_le_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]])
        };
        let on = f.to_bytes(&inv, eye);
        let off_b = f.with_sky_on(false).to_bytes(&inv, eye);
        assert_eq!(word(&on, 23), 1.0);
        assert_eq!(word(&off_b, 23), 0.0);
        assert!(f.sky_on());
        assert!(!f.with_sky_on(false).sky_on());
        assert!(f.off().sky_on());
        assert!(!f.with_sky_on(false).off().sky_on());
        for i in 0..32 {
            if i != 23 {
                assert_eq!(word(&on, i), word(&off_b, i), "word {i}");
            }
        }
    }

    #[test]
    fn uniform_bytes_sky_off_collapses_to_fog() {
        let f = fog();
        let off = f.with_sky_on(false);
        for dir in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, -1.0, 0.0]] {
            assert_eq!(off.sky_color([0.0, 1.0, 0.0], dir), f.color);
        }
        let up = f.sky_color([0.0, 30.0, 0.0], [0.0, 1.0, 0.0]);
        for c in 0..3 {
            assert!((up[c] - f.zenith[c]).abs() < 1.0e-4, "{up:?}");
        }
    }

    #[test]
    fn uniform_bytes_fs_sky_reads_sky_on_flag() {
        const WGSL: &str = include_str!("../shaders/forward.wgsl");
        let at = WGSL.find("fn fs_sky").expect("fs_sky exists");
        let body = &WGSL[at..];
        // Column-0 brace without its line end: a CRLF checkout of the
        // shader closes the entry the same way an LF one does.
        let end = body.find("\n}").expect("fs_sky closes");
        let sky = &body[..end];
        assert!(sky.contains("sky_on"), "{sky}");
        assert!(sky.contains("sky_view_sample"), "{sky}");
        assert!(sky.contains("fog_factor_infinite"), "{sky}");
        assert!(!sky.contains("aces_fitted"), "{sky}");
        assert!(!sky.contains("linear_to_srgb"), "{sky}");
    }
}
