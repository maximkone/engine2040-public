//! spec_lobe: the CPU twins of the forward shader's specular-lobe terms (RL47).
//!
//! Owner: render lane. Consumers: `ibl::energy_compensation` (the CPU IBL
//! shade) and the device tests below, which cut the shipped WGSL functions out
//! of `shaders/forward.wgsl`, run them in a compute pass and compare every
//! output with the twin here (a copied formula that drifted would fail, a
//! grep for a marker string would not). Units: unitless factors; `E` is the
//! lobe's directional albedo at `F0 = 1`; roughness is the perceptual value
//! unless a name says `alpha`. Determinism: pure f32, the same inputs give
//! the same bits on every run. Presentation only, never hashed into the
//! simulation.

/// RL47 A1: the floor the multiple-scatter `E` never drops below, so the gain
/// `1 / E - 1` is at most 3. A valid split-sum table holds `E >= 0.31`
/// everywhere (roughness 1 at normal view is the minimum), so the floor only
/// bites a degenerate texel: a zero or unbaked table, or a corrupt upload,
/// which would otherwise return a gain of up to 10000. Units: unitless `E`.
/// The WGSL twin is `MS_E_FLOOR` in `shaders/forward.wgsl` and in
/// `ibl::FORWARD_IBL_WGSL`.
pub const MS_ENERGY_FLOOR: f32 = 0.25;

/// The multiple-scatter gain `1 + F0 * (1 / E - 1)` of Filament 9.5 for one
/// specular lobe: single-scatter GGX loses the light that bounces more than
/// once (0.31 of it survives at roughness 1), and this puts it back, scaled
/// by `F0` because only the tinted reflectance bounces. `e` floors at
/// [`MS_ENERGY_FLOOR`]. The WGSL twin is `ms_gain` in `shaders/forward.wgsl`.
#[must_use]
pub fn ms_gain(f0: [f32; 3], e: f32) -> [f32; 3] {
    let gain = 1.0 / e.max(MS_ENERGY_FLOOR) - 1.0;
    [1.0 + f0[0] * gain, 1.0 + f0[1] * gain, 1.0 + f0[2] * gain]
}

/// RL47 A2: the weight on the screen-space change of the shading normal,
/// the donor's `screenVariance` (O3DE `SpecularAA.azsli`, after Tokuyoshi and
/// Kaplanyan 2019, "Improved Geometric Specular Antialiasing"). Unitless.
/// WGSL twin: `SPEC_AA_VARIANCE` in `shaders/forward.wgsl`.
pub const SPEC_AA_VARIANCE: f32 = 0.25;

/// RL47 A2: the cap on the squared alpha one pixel may add, the donor's
/// `varianceThresh`. Unitless. WGSL twin: `SPEC_AA_THRESHOLD`.
pub const SPEC_AA_THRESHOLD: f32 = 0.18;

/// Geometric specular anti-aliasing, the roughness the pixel's footprint
/// adds: a normal that turns fast across one pixel spreads the half vectors
/// the pixel integrates, which a point-sampled sharp lobe misses (it shows as
/// a sparkle that crawls on armour). `dn_dx` and `dn_dy` are the screen
/// derivatives of the unit shading normal; the result is the squared alpha
/// to add, `min(2 * 0.25 * (|dn_dx|^2 + |dn_dy|^2), 0.18)`. The WGSL twin is
/// `spec_aa_kernel` in `shaders/forward.wgsl`.
#[must_use]
pub fn spec_aa_kernel(dn_dx: [f32; 3], dn_dy: [f32; 3]) -> f32 {
    let sq = |v: [f32; 3]| v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    let variance = SPEC_AA_VARIANCE * (sq(dn_dx) + sq(dn_dy));
    (2.0 * variance).min(SPEC_AA_THRESHOLD)
}

/// The perceptual roughness after the footprint is added: the authored value
/// `perceptual` gives `alpha = perceptual^2`, the kernel adds to `alpha^2`
/// (the donor's `roughnessA2`), and the sum goes back through the fourth
/// root. A zero kernel returns `perceptual` untouched (not merely close), so
/// a flat surface and every path that never takes a derivative keep their
/// bits. The WGSL twin is `spec_aa_perceptual` in `shaders/forward.wgsl`.
#[must_use]
pub fn spec_aa_perceptual(perceptual: f32, kernel: f32) -> f32 {
    if kernel <= 0.0 {
        return perceptual;
    }
    let alpha = perceptual * perceptual;
    (alpha * alpha + kernel).clamp(0.0, 1.0).sqrt().sqrt()
}

/// RL47 A3: Lagarde specular occlusion (Lagarde and de Rousiers 2014, "Moving
/// Frostbite to PBR"; Filament `SpecularAO_Lagarde`): how much of the
/// reflection survives a pixel whose hemisphere is `ao` open, seen at
/// `ndotv`. `alpha` is the lobe's alpha (perceptual roughness squared), the
/// argument Filament passes. `1` for an open pixel at any view or roughness;
/// a rough lobe averages the whole hemisphere (the result tends to `ao`), a
/// smooth one at a normal view reflects the open sky above the cavity (above
/// `ao`) and at a grazing view the cavity wall (below `ao`). The WGSL twin is
/// `spec_occlusion` in `shaders/forward.wgsl`.
#[must_use]
pub fn specular_ao(ndotv: f32, ao: f32, alpha: f32) -> f32 {
    ((ndotv + ao).powf((-16.0 * alpha - 1.0).exp2()) - 1.0 + ao).clamp(0.0, 1.0)
}

/// RL47 A4: multi-bounce ambient occlusion (Jimenez et al. 2016, "Practical
/// Realtime Strategies for Accurate Indirect Occlusion"; Filament
/// `gtaoMultiBounce`): the cubic fit of the light that bounces between the
/// walls of a cavity, per albedo channel, never darker than `ao`. Clamped to
/// 1 because the fit reads 1.0005 at `ao = 1` for white, and an open pixel
/// must stay exactly open. The WGSL twin is `multi_bounce_ao`.
#[must_use]
pub fn multi_bounce_ao(ao: f32, albedo: [f32; 3]) -> [f32; 3] {
    let one = |t: f32| {
        let a = 2.0404 * t - 0.3324;
        let b = -4.7951 * t + 0.6417;
        let c = 2.7552 * t + 0.6903;
        (((ao * a + b) * ao + c) * ao).max(ao).min(1.0)
    };
    [one(albedo[0]), one(albedo[1]), one(albedo[2])]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_valid_table_never_reaches_the_floor_and_a_dead_one_gains_three() {
        // E of a white metal at roughness 1, normal view: the table minimum.
        let closed = ms_gain([1.0; 3], 0.3068);
        assert!((0.3068 * closed[0] - 1.0).abs() < 1e-5, "the lobe closes");
        // A zero texel used to gain 10000 (E floored at 1e-4); now 3.
        for e in [0.0, 1e-6, 1e-4, 0.1, 0.2499] {
            let g = ms_gain([1.0; 3], e);
            assert_eq!(g, [4.0; 3], "E {e} gains at most 3x on a white metal");
        }
        let half = ms_gain([1.0, 0.5, 0.04], 0.0);
        for (got, want) in half.iter().zip([4.0, 2.5, 1.12]) {
            assert!(
                (got - want).abs() < 1e-6,
                "the gain scales with F0: {half:?}"
            );
        }
        // The floor is the lowest E that is still honest.
        assert_eq!(ms_gain([1.0; 3], MS_ENERGY_FLOOR), [4.0; 3]);
        assert!(ms_gain([1.0; 3], 0.26)[0] < 4.0, "above the floor it falls");
        // A lobe that keeps all its light needs nothing.
        assert_eq!(ms_gain([1.0, 0.5, 0.04], 1.0), [1.0; 3]);
    }

    #[test]
    fn spec_aa_kernel_is_the_donors_variance_kernel() {
        assert_eq!(
            spec_aa_kernel([0.0; 3], [0.0; 3]),
            0.0,
            "a flat normal adds none"
        );
        // |dx|^2 = 0.09, |dy|^2 = 0.16: variance 0.25 * 0.25, kernel twice that.
        let k = spec_aa_kernel([0.3, 0.0, 0.0], [0.0, 0.4, 0.0]);
        assert!((k - 0.125).abs() < 1e-7, "kernel {k}");
        // The cap: a normal that flips across a pixel adds 0.18, never more.
        assert_eq!(
            spec_aa_kernel([10.0, 0.0, 0.0], [0.0, 10.0, 0.0]),
            SPEC_AA_THRESHOLD
        );
        assert_eq!(
            spec_aa_kernel([0.5, 0.5, 0.5], [0.5, 0.5, 0.5]),
            SPEC_AA_THRESHOLD
        );
    }

    #[test]
    fn spec_aa_perceptual_only_ever_roughens_and_a_zero_kernel_keeps_the_bits() {
        for step in 0..=20 {
            let p = step as f32 / 20.0;
            assert_eq!(spec_aa_perceptual(p, 0.0).to_bits(), p.to_bits(), "p {p}");
            let mut last = p;
            for k in [1e-4, 1e-3, 0.01, 0.05, 0.125, 0.18] {
                let q = spec_aa_perceptual(p, k);
                assert!(q >= last && q <= 1.0, "p {p} k {k}: {q} after {last}");
                last = q;
            }
        }
        // Worked numbers: p 0.3 is alpha 0.09, alpha^2 0.0081; + 0.125 -> 0.60401.
        let q = spec_aa_perceptual(0.3, 0.125);
        assert!((q - 0.604_01).abs() < 1e-4, "got {q}");
        // A perfect mirror under the full cap is p = 0.18^(1/4).
        assert!((spec_aa_perceptual(0.0, SPEC_AA_THRESHOLD) - 0.651_35).abs() < 1e-4);
        // A fully rough surface cannot go past 1.
        assert_eq!(spec_aa_perceptual(1.0, 0.18), 1.0);
    }

    #[test]
    fn specular_ao_follows_lagarde() {
        // Open stays open for every view and roughness (bit for bit at the
        // ends the live frame meets most).
        for alpha in [0.0, 0.01, 0.09, 0.25, 0.5, 1.0] {
            for ndotv in [0.0, 0.1, 0.5, 0.9, 1.0] {
                let open = specular_ao(ndotv, 1.0, alpha);
                assert!(
                    (open - 1.0).abs() < 1e-6,
                    "ndotv {ndotv} alpha {alpha}: {open}"
                );
            }
        }
        // A rough lobe sees the whole cavity: the term tends to the field.
        let rough = specular_ao(0.7, 0.4, 1.0);
        assert!(
            (rough - 0.4).abs() < 1e-3,
            "alpha 1 reads the field, got {rough}"
        );
        // A mirror at a normal view reflects the open sky above the cavity;
        // at a grazing view it reflects the cavity wall.
        let up = specular_ao(1.0, 0.4, 0.0);
        let wall = specular_ao(0.05, 0.4, 0.0);
        assert!(up > 0.4 && wall < 0.4 && wall > 0.0, "up {up} wall {wall}");
        // Worked number: pow(0.7 + 0.4, exp2(-1)) - 1 + 0.4 = 1.1^0.5 - 0.6.
        assert!((specular_ao(0.7, 0.4, 0.0) - (1.1_f32.sqrt() - 0.6)).abs() < 1e-6);
        // Fully shut and facing away reaches 0; the clamp holds the ends.
        assert_eq!(specular_ao(0.0, 0.0, 0.5), 0.0);
        for ao in [0.0, 0.2, 0.5, 0.8, 1.0] {
            let s = specular_ao(0.3, ao, 0.2);
            assert!((0.0..=1.0).contains(&s), "ao {ao}: {s}");
        }
    }

    #[test]
    fn multi_bounce_ao_lifts_a_cavity_by_its_albedo_and_leaves_open_alone() {
        // Open is exactly open, even for white (the raw fit reads 1.0005).
        for albedo in [[0.0; 3], [0.5; 3], [1.0; 3], [0.9, 0.2, 0.05]] {
            assert_eq!(multi_bounce_ao(1.0, albedo), [1.0; 3], "albedo {albedo:?}");
        }
        // Never darker than the field; a white surface in a half-open cavity
        // keeps far more than 0.5, a black one exactly the field.
        let white = multi_bounce_ao(0.5, [1.0; 3]);
        assert!((white[0] - 0.8979).abs() < 1e-4, "white at 0.5: {white:?}");
        let black = multi_bounce_ao(0.5, [0.0; 3]);
        assert_eq!(black, [0.5; 3], "black bounces nothing");
        // The lift grows with albedo and the channels are independent.
        let tinted = multi_bounce_ao(0.5, [0.9, 0.5, 0.1]);
        assert!(tinted[0] > tinted[1] && tinted[1] > tinted[2], "{tinted:?}");
        for ao in [0.0, 0.1, 0.3, 0.6, 0.9, 1.0] {
            for t in [0.0, 0.3, 0.7, 1.0] {
                let m = multi_bounce_ao(ao, [t; 3]);
                assert!(m[0] >= ao && m[0] <= 1.0, "ao {ao} albedo {t}: {m:?}");
            }
        }
    }

    /// The bumpy metal of the shimmer bar: a unit normal field with more
    /// detail than the pixel grid, so a sharp lobe point-sampled on it
    /// sparkles. Pure of position `(x, y)` in pixels.
    fn bump(x: f32, y: f32) -> [f32; 3] {
        let a = 0.5;
        let v = [
            a * (x * 2.1 + (y * 1.3).sin()).sin(),
            a * (y * 1.9 + (x * 1.1).sin()).sin(),
            1.0,
        ];
        let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        [v[0] / len, v[1] / len, v[2] / len]
    }

    /// The linear highlight of one normal under the fixed light, viewed
    /// straight on: GGX `D` at the given perceptual roughness times the
    /// constant visibility 0.25 and `n.l`.
    fn highlight(perceptual: f32, n: [f32; 3]) -> f32 {
        let l = [0.3_f32, 0.7, 0.65];
        let ll = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
        let l = [l[0] / ll, l[1] / ll, l[2] / ll];
        let h = [l[0], l[1], l[2] + 1.0];
        let hl = (h[0] * h[0] + h[1] * h[1] + h[2] * h[2]).sqrt();
        let n_h = (n[0] * h[0] + n[1] * h[1] + n[2] * h[2]) / hl;
        let n_l = (n[0] * l[0] + n[1] * l[1] + n[2] * l[2]).max(0.0);
        let a2 = (perceptual * perceptual).powi(2);
        let d = a2 / (std::f32::consts::PI * (n_h * n_h * (a2 - 1.0) + 1.0).powi(2));
        d * 0.25 * n_l
    }

    /// One orbit frame of the bumpy plate, 32 x 32 pixels, as the aliasing
    /// error against a 4 x 4 supersampled reference (the lobe integrated over
    /// the pixel footprint, the picture the pixel should hold). `orbit` shifts
    /// the sample point by 0.37 px and 0.21 px per frame, the sub-pixel crawl
    /// of a camera circling the armour; `anti_alias` takes the footprint from
    /// the finite difference of the normal to the next pixel, as `dpdx` does.
    fn orbit_error_frame(frame: u32, anti_alias: bool) -> Vec<[f32; 3]> {
        let (sx, sy) = (0.37 * frame as f32, 0.21 * frame as f32);
        let tone = |s: f32| s / (1.0 + s);
        let perceptual = 0.3;
        let mut out = Vec::with_capacity(32 * 32);
        for y in 0..32 {
            for x in 0..32 {
                let (px, py) = (x as f32 + sx, y as f32 + sy);
                let n = bump(px, py);
                let p = if anti_alias {
                    let (nx, ny) = (bump(px + 1.0, py), bump(px, py + 1.0));
                    let d = |m: [f32; 3]| [m[0] - n[0], m[1] - n[1], m[2] - n[2]];
                    spec_aa_perceptual(perceptual, spec_aa_kernel(d(nx), d(ny)))
                } else {
                    perceptual
                };
                let mut reference = 0.0;
                for j in 0..4 {
                    for i in 0..4 {
                        let (qx, qy) = (
                            px + (i as f32 + 0.5) / 4.0 - 0.5,
                            py + (j as f32 + 0.5) / 4.0 - 0.5,
                        );
                        reference += highlight(perceptual, bump(qx, qy));
                    }
                }
                let err = tone(highlight(p, n)) - tone(reference / 16.0);
                out.push([err, err, err]);
            }
        }
        out
    }

    /// The row's orbit bar: the temporal variance of the aliasing error over
    /// a 16-frame sub-pixel orbit. Without the footprint a metal at roughness
    /// 0.3 on a bumpy plate crawls well past the SMAA gate; with it the crawl
    /// sits under the gate and at under half of the plain crawl.
    #[test]
    fn orbit_shimmer_stays_under_the_smaa_gate_with_the_footprint() {
        let clip = |anti_alias: bool| -> Vec<Vec<[f32; 3]>> {
            (0..16).map(|f| orbit_error_frame(f, anti_alias)).collect()
        };
        let var = |clip: &[Vec<[f32; 3]>]| crate::post::ShimmerScene::temporal_variance(clip);
        let plain = var(&clip(false));
        let aa = var(&clip(true));
        eprintln!(
            "RL47 orbit shimmer: plain {plain:.4}, spec aa {aa:.4} (gate {})",
            crate::aa_scale::SMAA_SHIMMER_MAX_VAR
        );
        assert!(
            plain > crate::aa_scale::SMAA_SHIMMER_MAX_VAR,
            "the plate must really crawl, got {plain}"
        );
        assert!(
            aa <= crate::aa_scale::SMAA_SHIMMER_MAX_VAR,
            "orbit shimmer {aa} exceeds SMAA_SHIMMER_MAX_VAR"
        );
        assert!(
            aa * 2.0 < plain,
            "the footprint halves the crawl: {aa} vs {plain}"
        );
        assert_eq!(aa, var(&clip(true)), "the bar is bit-stable twice");
    }

    #[test]
    fn the_shipped_tap_count_is_closer_to_the_converged_table_than_half_of_it() {
        // RL47 A7: the table the gain divides by. Mean |E - E_converged| over a
        // grid: more taps must be closer, and the roughness-1 texel (the one
        // the furnace test stands on) must stay inside 1% of the converged one.
        use crate::ibl::{brdf_integrate_taps, BRDF_SAMPLES};
        let e = |nv: f32, r: f32, taps: u32| {
            let pair = brdf_integrate_taps(nv, r, taps).expect("integrates");
            pair[0] + pair[1]
        };
        let mean_err = |taps: u32| {
            let mut sum = 0.0;
            for i in 0..8 {
                for j in 0..8 {
                    let (nv, r) = ((i as f32 + 0.5) / 8.0, (j as f32 + 0.5) / 8.0);
                    sum += (e(nv, r, taps) - e(nv, r, 8192)).abs();
                }
            }
            sum / 64.0
        };
        let (half, shipped) = (mean_err(BRDF_SAMPLES / 2), mean_err(BRDF_SAMPLES));
        assert!(
            shipped < half,
            "{BRDF_SAMPLES} taps err {shipped} vs half {half}"
        );
        let rough = e(1.0, 1.0, BRDF_SAMPLES);
        assert!(
            ((rough - e(1.0, 1.0, 8192)) / rough).abs() < 0.01,
            "rough texel E {rough}"
        );
        assert!(
            brdf_integrate_taps(0.5, 0.5, 0).is_err(),
            "zero taps refuse"
        );
    }
}

/// The device half of the twin tests: cut named items out of the shipped
/// shader text, drop them into a small compute shader and run it on the
/// shared test device.
#[cfg(all(test, feature = "gpu"))]
mod device_twin {
    use super::*;
    use wgpu::util::DeviceExt;

    /// The source of `forward.wgsl` as it ships.
    pub(super) const FORWARD: &str = include_str!("shaders/forward.wgsl");

    /// One module-scope item cut out of `src`: `head` is its first words
    /// (`const MS_E_FLOOR` or `fn ms_gain(`). A `const` runs to its `;`, a
    /// `fn` through the brace that closes its body.
    pub(super) fn wgsl_item<'a>(src: &'a str, head: &str) -> Result<&'a str, String> {
        let at = src
            .match_indices(head)
            .map(|(i, _)| i)
            .find(|&i| i == 0 || src.as_bytes()[i - 1] == b'\n')
            .ok_or_else(|| format!("no `{head}` at the start of a line"))?;
        let rest = &src[at..];
        if head.starts_with("const ") {
            let end = rest.find(';').ok_or("a const with no `;`")?;
            return Ok(&rest[..=end]);
        }
        let open = rest.find('{').ok_or("a fn with no body")?;
        let mut depth = 0usize;
        for (i, c) in rest[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(&rest[..open + i + 1]);
                    }
                }
                _ => {}
            }
        }
        Err(format!("`{head}` never closes its body"))
    }

    /// Every named item of the shipped forward shader, joined for a kernel.
    pub(super) fn forward_items(heads: &[&str]) -> String {
        heads
            .iter()
            .map(|h| wgsl_item(FORWARD, h).unwrap_or_else(|e| panic!("{e}")))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Run `items` plus a one-thread-per-row kernel over `rows`: each row is
    /// two input vec4s, `x` and `z`; `body` assigns the output `y: vec4<f32>`.
    /// Returns one output vec4 per row, or nothing when the machine has no
    /// adapter; panics on a validation error.
    pub(super) fn run_rows(items: &str, body: &str, rows: &[[f32; 8]]) -> Vec<[f32; 4]> {
        let Some((device, queue, _)) =
            crate::test_gpu::require_or_skip("spec_lobe device twin", crate::graph::gx1_probe())
        else {
            return Vec::new();
        };
        let src = format!(
            "{items}\n\
             @group(0) @binding(0) var<storage, read> xs: array<vec4<f32>>;\n\
             @group(0) @binding(1) var<storage, read_write> ys: array<vec4<f32>>;\n\
             @compute @workgroup_size(64)\n\
             fn main(@builtin(global_invocation_id) id: vec3<u32>) {{\n\
                 if (id.x >= arrayLength(&ys)) {{ return; }}\n\
                 let x = xs[2u * id.x];\n\
                 let z = xs[2u * id.x + 1u];\n\
                 var y = vec4<f32>(0.0);\n\
                 {body}\n\
                 ys[id.x] = y;\n\
             }}\n"
        );
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let bytes: Vec<u8> = rows
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let xs = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("spec-lobe-xs"),
            contents: &bytes,
            usage: wgpu::BufferUsages::STORAGE,
        });
        let size = (rows.len() * 16) as u64;
        let ys = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("spec-lobe-ys"),
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let back = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("spec-lobe-back"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("spec-lobe-kernel"),
            source: wgpu::ShaderSource::Wgsl(src.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("spec-lobe-kernel"),
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("spec-lobe-bind"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: xs.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: ys.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("spec-lobe-encoder"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("spec-lobe-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups((rows.len() as u32).div_ceil(64), 1, 1);
        }
        encoder.copy_buffer_to_buffer(&ys, 0, &back, 0, size);
        queue.submit(Some(encoder.finish()));
        crate::accel::map_readback(device, &back, "spec-lobe").expect("readback maps");
        let flat = crate::accel::readback_f32(&back, rows.len() * 4, "spec-lobe")
            .expect("readback decodes");
        back.unmap();
        let err = pollster::block_on(scope.pop());
        assert!(err.is_none(), "zero validation errors, got {err:?}");
        flat.chunks_exact(4)
            .map(|c| [c[0], c[1], c[2], c[3]])
            .collect()
    }

    /// `got` within f32 slack of `want`: relative 1e-5 with an absolute floor.
    pub(super) fn close(got: f32, want: f32) -> bool {
        (got - want).abs() <= 1e-5 * want.abs().max(1.0)
    }

    #[test]
    fn wgsl_item_cuts_a_const_and_a_nested_fn() {
        let src = "const A: f32 = 1.0;\nfn f(x: f32) -> f32 {\n  if (x > 0.0) { return x; }\n  return 0.0;\n}\nfn g() {}\n";
        assert_eq!(wgsl_item(src, "const A").unwrap(), "const A: f32 = 1.0;");
        let f = wgsl_item(src, "fn f(").unwrap();
        assert!(
            f.starts_with("fn f(") && f.ends_with("return 0.0;\n}"),
            "{f}"
        );
        assert!(wgsl_item(src, "fn h(").is_err(), "a missing item is named");
        assert!(wgsl_item("x fn f() { 1", "fn f(").is_err(), "no line start");
    }

    #[test]
    fn ms_gain_wgsl_matches_the_cpu_twin() {
        let items = forward_items(&["const MS_E_FLOOR", "fn ms_gain("]);
        let mut rows = Vec::new();
        for f0 in [
            [0.04, 0.04, 0.04],
            [1.0, 0.5, 0.04],
            [0.95, 0.64, 0.54],
            [1.0, 1.0, 1.0],
        ] {
            for e in [
                0.0, 1e-6, 1e-4, 0.1, 0.2499, 0.25, 0.2501, 0.3068, 0.45, 0.8, 1.0, 1.5,
            ] {
                rows.push([f0[0], f0[1], f0[2], e, 0.0, 0.0, 0.0, 0.0]);
            }
        }
        let got = run_rows(&items, "y = vec4<f32>(ms_gain(x.xyz, x.w), 0.0);", &rows);
        if got.is_empty() {
            return;
        }
        assert_eq!(got.len(), rows.len());
        for (row, out) in rows.iter().zip(&got) {
            let want = ms_gain([row[0], row[1], row[2]], row[3]);
            for c in 0..3 {
                assert!(
                    close(out[c], want[c]),
                    "f0 {:?} e {}: shader {} twin {}",
                    &row[..3],
                    row[3],
                    out[c],
                    want[c]
                );
            }
        }
    }

    #[test]
    fn spec_aa_wgsl_matches_the_cpu_twin() {
        let items = forward_items(&[
            "const SPEC_AA_VARIANCE",
            "const SPEC_AA_THRESHOLD",
            "fn spec_aa_kernel(",
            "fn spec_aa_perceptual(",
        ]);
        let derivs: [([f32; 3], [f32; 3]); 7] = [
            ([0.0; 3], [0.0; 3]),
            ([0.3, 0.0, 0.0], [0.0, 0.4, 0.0]),
            ([0.01, 0.02, 0.0], [0.0, 0.01, 0.03]),
            ([0.1, 0.2, 0.3], [0.3, 0.2, 0.1]),
            ([0.5, 0.5, 0.5], [0.5, 0.5, 0.5]),
            ([10.0, 0.0, 0.0], [0.0, 10.0, 0.0]),
            ([0.2, -0.1, 0.05], [-0.15, 0.05, 0.2]),
        ];
        let mut rows = Vec::new();
        for (dx, dy) in derivs {
            for p in [0.0, 0.089, 0.2, 0.3, 0.5, 0.8, 1.0] {
                rows.push([dx[0], dx[1], dx[2], p, dy[0], dy[1], dy[2], 0.0]);
            }
        }
        let body = "let k = spec_aa_kernel(x.xyz, z.xyz); \
                    y = vec4<f32>(k, spec_aa_perceptual(x.w, k), spec_aa_perceptual(x.w, 0.0), 0.0);";
        let got = run_rows(&items, body, &rows);
        if got.is_empty() {
            return;
        }
        for (row, out) in rows.iter().zip(&got) {
            let k = spec_aa_kernel([row[0], row[1], row[2]], [row[4], row[5], row[6]]);
            let p = row[3];
            assert!(
                close(out[0], k),
                "kernel {:?}: shader {} twin {k}",
                row,
                out[0]
            );
            let want = spec_aa_perceptual(p, k);
            assert!(
                close(out[1], want),
                "perceptual p {p} k {k}: shader {} twin {want}",
                out[1]
            );
            assert_eq!(
                out[2].to_bits(),
                p.to_bits(),
                "a zero kernel keeps p {p} bit for bit"
            );
        }
    }

    #[test]
    fn ambient_occlusion_wgsl_matches_the_cpu_twins() {
        let items = forward_items(&["fn spec_occlusion(", "fn multi_bounce_ao("]);
        let mut rows = Vec::new();
        for ndotv in [0.0, 0.05, 0.3, 0.7, 1.0] {
            for ao in [0.0, 0.1, 0.4, 0.75, 1.0] {
                for alpha in [0.0, 0.0081, 0.09, 0.3, 1.0] {
                    for albedo in [[0.0; 3], [1.0; 3], [0.9, 0.5, 0.1], [0.3, 0.6, 0.8]] {
                        // x = (ndotv, ao, alpha, albedo r), z = (albedo g, albedo b).
                        rows.push([ndotv, ao, alpha, albedo[0], albedo[1], albedo[2], 0.0, 0.0]);
                    }
                }
            }
        }
        let body = "let s = spec_occlusion(x.x, x.y, x.z);                     let m = multi_bounce_ao(x.y, vec3<f32>(x.w, z.x, z.y));                     y = vec4<f32>(s, m.x, m.y, m.z);";
        let got = run_rows(&items, body, &rows);
        if got.is_empty() {
            return;
        }
        assert_eq!(got.len(), rows.len());
        for (row, out) in rows.iter().zip(&got) {
            let want_s = specular_ao(row[0], row[1], row[2]);
            assert!(
                (out[0] - want_s).abs() < 2e-4,
                "specular ao {:?}: shader {} twin {want_s}",
                &row[..3],
                out[0]
            );
            let want_m = multi_bounce_ao(row[1], [row[3], row[4], row[5]]);
            for c in 0..3 {
                assert!(
                    (out[1 + c] - want_m[c]).abs() < 1e-5,
                    "multi bounce ao {} albedo {:?}: shader {} twin {}",
                    row[1],
                    &row[3..6],
                    out[1 + c],
                    want_m[c]
                );
            }
        }
    }
}
