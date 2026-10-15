//! compare: how two records differ, in numbers an AI can act on.
//!
//! No render needed: the verdict is analytic. `delta_e` is CIEDE2000 between
//! the two base colors (linear RGB to XYZ to Lab, D65), `luminance_ratio`
//! is b over a for the relative luminance, `specular_peak_ratio` compares the
//! brightest highlight each record can produce for a head-on light (the GGX
//! peak `1 / (pi * alpha^2)` times the Fresnel reflectance at normal incidence,
//! for the base lobe and for a coat, whichever is higher), and the remaining
//! fields are plain differences. The sim half is compared apart, so a look
//! edit and a gameplay edit never blur. Pure math, no clock, no GPU.
//!
//! Formulas: CIEDE2000 (Sharma, Wu and Dalal 2005, the published equations),
//! sRGB primaries and D65 white (IEC 61966-2-1).

use crate::record::MaterialRecord;

/// The difference of record `b` against record `a`.
#[derive(Clone, PartialEq, Debug)]
pub struct Delta {
    /// CIEDE2000 between the base colors (0 is identical, about 1 is the
    /// just-noticeable step).
    pub delta_e: f64,
    /// Relative luminance of `b` over `a`.
    pub luminance_ratio: f64,
    /// Brightest-highlight ratio, `b` over `a`.
    pub specular_peak_ratio: f64,
    /// `b - a` perceptual roughness.
    pub roughness: f32,
    /// `b - a` metalness.
    pub metalness: f32,
    /// `b - a` coat weight (0 when a record has no coat).
    pub coat_weight: f32,
    /// `b - a` index of refraction.
    pub ior: f32,
    /// `b - a` emission luminance in nits.
    pub emission_nits: f32,
    /// `b - a` toughness.
    pub toughness: f64,
    /// `b - a` brittleness.
    pub brittleness: f64,
    /// True when the impact sound family differs.
    pub sound_changed: bool,
    /// True when the shard rule differs.
    pub shard_changed: bool,
    /// True when the sim digests differ (anything the tick reads).
    pub sim_changed: bool,
    /// True when the look digests differ.
    pub look_changed: bool,
    /// True when the two shaders have different structure (a pipeline each).
    pub structure_changed: bool,
}

/// Linear sRGB to CIE Lab (D65 white).
#[must_use]
pub fn lab_of(rgb: [f32; 3]) -> [f64; 3] {
    let [r, g, b] = rgb.map(|c| f64::from(c).clamp(0.0, 1.0));
    let x = 0.412_456_4 * r + 0.357_576_1 * g + 0.180_437_5 * b;
    let y = 0.212_672_9 * r + 0.715_152_2 * g + 0.072_175_0 * b;
    let z = 0.019_333_9 * r + 0.119_192_0 * g + 0.950_304_1 * b;
    let f = |t: f64| {
        if t > 216.0 / 24_389.0 {
            t.cbrt()
        } else {
            (24_389.0 / 27.0 * t + 16.0) / 116.0
        }
    };
    let (fx, fy, fz) = (f(x / 0.950_47), f(y), f(z / 1.088_83));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// CIEDE2000 between two Lab colors.
#[must_use]
pub fn ciede2000(l1: [f64; 3], l2: [f64; 3]) -> f64 {
    let rad = std::f64::consts::PI / 180.0;
    let (c1, c2) = (l1[1].hypot(l1[2]), l2[1].hypot(l2[2]));
    let c_bar = (c1 + c2) / 2.0;
    let c7 = c_bar.powi(7);
    let g = 0.5 * (1.0 - (c7 / (c7 + 25f64.powi(7))).sqrt());
    let (a1, a2) = ((1.0 + g) * l1[1], (1.0 + g) * l2[1]);
    let (cp1, cp2) = (a1.hypot(l1[2]), a2.hypot(l2[2]));
    let hue = |a: f64, b: f64| {
        if a == 0.0 && b == 0.0 {
            0.0
        } else {
            let h = b.atan2(a) / rad;
            if h < 0.0 {
                h + 360.0
            } else {
                h
            }
        }
    };
    let (h1, h2) = (hue(a1, l1[2]), hue(a2, l2[2]));
    let dl = l2[0] - l1[0];
    let dc = cp2 - cp1;
    let dh_deg = if cp1 * cp2 == 0.0 {
        0.0
    } else if (h2 - h1).abs() <= 180.0 {
        h2 - h1
    } else if h2 - h1 > 180.0 {
        h2 - h1 - 360.0
    } else {
        h2 - h1 + 360.0
    };
    let dh = 2.0 * (cp1 * cp2).sqrt() * (dh_deg * rad / 2.0).sin();
    let l_bar = (l1[0] + l2[0]) / 2.0;
    let cp_bar = (cp1 + cp2) / 2.0;
    let h_bar = if cp1 * cp2 == 0.0 {
        h1 + h2
    } else if (h1 - h2).abs() <= 180.0 {
        (h1 + h2) / 2.0
    } else if h1 + h2 < 360.0 {
        (h1 + h2 + 360.0) / 2.0
    } else {
        (h1 + h2 - 360.0) / 2.0
    };
    let t = 1.0 - 0.17 * ((h_bar - 30.0) * rad).cos()
        + 0.24 * ((2.0 * h_bar) * rad).cos()
        + 0.32 * ((3.0 * h_bar + 6.0) * rad).cos()
        - 0.20 * ((4.0 * h_bar - 63.0) * rad).cos();
    let d_theta = 30.0 * (-(((h_bar - 275.0) / 25.0).powi(2))).exp();
    let cp7 = cp_bar.powi(7);
    let rc = 2.0 * (cp7 / (cp7 + 25f64.powi(7))).sqrt();
    let sl = 1.0 + 0.015 * (l_bar - 50.0).powi(2) / (20.0 + (l_bar - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * cp_bar;
    let sh = 1.0 + 0.015 * cp_bar * t;
    let rt = -(2.0 * d_theta * rad).sin() * rc;
    ((dl / sl).powi(2) + (dc / sc).powi(2) + (dh / sh).powi(2) + rt * (dc / sc) * (dh / sh)).sqrt()
}

fn luminance(rgb: [f32; 3]) -> f64 {
    0.2126 * f64::from(rgb[0]) + 0.7152 * f64::from(rgb[1]) + 0.0722 * f64::from(rgb[2])
}

/// The brightest highlight a head-on light can make: GGX peak times the
/// normal-incidence Fresnel reflectance, over the base lobe and the coat.
#[must_use]
pub fn specular_peak(record: &MaterialRecord) -> f64 {
    let look = &record.look;
    let alpha = |p: f32| f64::from(p).clamp(0.089, 1.0).powi(2);
    let ior = f64::from(look.specular_ior);
    let dielectric = ((ior - 1.0) / (ior + 1.0)).powi(2);
    let metal = f64::from(look.base_metalness);
    let f0 = dielectric * (1.0 - metal) + luminance(look.base_color) * metal;
    let peak = |a: f64| 1.0 / (std::f64::consts::PI * a * a);
    let mut best = f0 * peak(alpha(look.specular_roughness));
    if let Some(c) = look.coat {
        let cior = f64::from(c.ior);
        let cf0 = ((cior - 1.0) / (cior + 1.0)).powi(2);
        best = best.max(f64::from(c.weight) * cf0 * peak(alpha(c.roughness)));
    }
    best
}

fn q16(value: u32) -> f64 {
    f64::from(value) / 65_536.0
}

/// Compare `b` against `a`.
#[must_use]
pub fn compare(a: &MaterialRecord, b: &MaterialRecord) -> Delta {
    let (la, lb) = (luminance(a.look.base_color), luminance(b.look.base_color));
    let coat = |r: &MaterialRecord| r.look.coat.map_or(0.0, |c| c.weight);
    let nits = |r: &MaterialRecord| r.look.emission.map_or(0.0, |e| e.luminance);
    let (pa, pb) = (specular_peak(a), specular_peak(b));
    Delta {
        delta_e: ciede2000(lab_of(a.look.base_color), lab_of(b.look.base_color)),
        luminance_ratio: if la > 0.0 { lb / la } else { 1.0 },
        specular_peak_ratio: if pa > 0.0 { pb / pa } else { 1.0 },
        roughness: b.look.specular_roughness - a.look.specular_roughness,
        metalness: b.look.base_metalness - a.look.base_metalness,
        coat_weight: coat(b) - coat(a),
        ior: b.look.specular_ior - a.look.specular_ior,
        emission_nits: nits(b) - nits(a),
        toughness: q16(b.body.toughness_q16) - q16(a.body.toughness_q16),
        brittleness: q16(b.body.brittleness_q16) - q16(a.body.brittleness_q16),
        sound_changed: a.body.sound != b.body.sound,
        shard_changed: a.body.shard != b.body.shard,
        sim_changed: a.sim_digest() != b.sim_digest(),
        look_changed: a.look_digest() != b.look_digest(),
        structure_changed: crate::compile::Structure::of(a) != crate::compile::Structure::of(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::set::MaterialSet;

    #[test]
    fn ciede2000_matches_the_published_pairs() {
        // Sharma, Wu and Dalal 2005, table 1 (pairs 1, 2, 3 and 17).
        let cases = [
            ([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485], 2.0425),
            ([50.0, 3.1571, -77.2803], [50.0, 0.0, -82.7485], 2.8615),
            ([50.0, 2.8361, -74.0200], [50.0, 0.0, -82.7485], 3.4412),
            (
                [60.2574, -34.0099, 36.2677],
                [60.4626, -34.1751, 39.4387],
                1.2644,
            ),
        ];
        for (a, b, want) in cases {
            let got = ciede2000(a, b);
            assert!((got - want).abs() < 1e-3, "{a:?} {b:?}: {got} vs {want}");
            // Symmetric.
            assert!((ciede2000(b, a) - got).abs() < 1e-9);
        }
        assert_eq!(ciede2000([50.0, 10.0, 10.0], [50.0, 10.0, 10.0]), 0.0);
    }

    #[test]
    fn white_and_black_have_the_expected_lab() {
        let white = lab_of([1.0; 3]);
        assert!((white[0] - 100.0).abs() < 0.05 && white[1].abs() < 0.05 && white[2].abs() < 0.1);
        assert!(lab_of([0.0; 3])[0].abs() < 1e-9);
    }

    #[test]
    fn identical_records_have_no_delta() {
        let set = MaterialSet::shipped();
        let a = set.get("marble").expect("marble");
        let d = compare(a, a);
        assert_eq!(d.delta_e, 0.0);
        assert_eq!((d.luminance_ratio, d.specular_peak_ratio), (1.0, 1.0));
        assert!(!d.sim_changed && !d.look_changed && !d.structure_changed);
    }

    #[test]
    fn dry_versus_wet_marble_is_darker_and_much_shinier() {
        let set = MaterialSet::shipped();
        let dry = set.get("marble").expect("marble");
        let wet = set.get("marble_wet").expect("marble_wet");
        let d = compare(dry, wet);
        assert!(d.luminance_ratio < 0.95, "{}", d.luminance_ratio);
        assert!(d.specular_peak_ratio >= 2.0, "{}", d.specular_peak_ratio);
        assert!(d.delta_e > 2.0, "{}", d.delta_e);
        assert!(d.roughness < 0.0 && d.sim_changed && d.look_changed);
        assert!(d.sound_changed, "wet marble sounds soft");
        assert!(d.toughness < 0.0);
    }

    #[test]
    fn a_roughness_edit_moves_the_peak_and_not_the_sim() {
        let set = MaterialSet::shipped();
        let a = set.get("stone").expect("stone").clone();
        let mut b = a.clone();
        b.look.specular_roughness = 0.3;
        let d = compare(&a, &b);
        assert!(d.specular_peak_ratio > 10.0);
        assert!(!d.sim_changed && d.look_changed && !d.structure_changed);
    }

    #[test]
    fn separate_digests() {
        // A look edit moves only the look digest; a sim edit moves only the
        // sim digest. Sweeps all 14 siege records so one swapped field shows
        // as a cross move, never a silent pass.
        use engine_destruction::material::MATERIAL_COUNT;
        let set = MaterialSet::siege_kit();
        assert_eq!(set.len(), MATERIAL_COUNT);
        assert_eq!(MATERIAL_COUNT, 14);
        let (mut look_cross, mut sim_cross) = (0, 0);
        for base in set.records() {
            // Bytes x2: the fixture text is the canonical form twice.
            let text = base.to_text();
            let again = MaterialRecord::parse(&text).expect("canonical text parses");
            assert_eq!(&again, base, "{}", base.name);
            assert_eq!(again.to_text(), text, "{}", base.name);
            // Look edit: a roughness no record ships with.
            let mut look_edited = base.clone();
            let rough = if base.look.specular_roughness == 0.2 {
                0.8
            } else {
                0.2
            };
            look_edited.look.specular_roughness = rough;
            assert_ne!(
                look_edited.look_digest(),
                base.look_digest(),
                "{}",
                base.name
            );
            if look_edited.sim_digest() != base.sim_digest() {
                look_cross += 1;
            }
            let d = compare(base, &look_edited);
            assert!(d.look_changed, "{}", base.name);
            if d.sim_changed {
                look_cross += 1;
            }
            assert!(d.roughness != 0.0, "{}", base.name);
            // Sim edit: one Q16 step of toughness.
            let mut sim_edited = base.clone();
            sim_edited.body.toughness_q16 = base.body.toughness_q16.wrapping_add(1);
            assert_ne!(sim_edited.sim_digest(), base.sim_digest(), "{}", base.name);
            if sim_edited.look_digest() != base.look_digest() {
                sim_cross += 1;
            }
            let e = compare(base, &sim_edited);
            assert!(e.sim_changed, "{}", base.name);
            if e.look_changed {
                sim_cross += 1;
            }
            assert!(e.toughness != 0.0, "{}", base.name);
        }
        assert_eq!((look_cross, sim_cross), (0, 0));
    }
}
