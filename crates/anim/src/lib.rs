//! anim: SoA clip sampler + blend + quant. Copies [f32;3] out as Position, never shared types.

// Unit tests prove zero-allocation paths with the shared counting probe.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOC: engine_core::alloc_probe::CountingAlloc =
    engine_core::alloc_probe::CountingAlloc;

use clip::clamp01;
use engine_core::Fnv64;

/// Additive spine: locomotion base, masked upper-body override and flinch
/// additive through one inertializer, planted feet pinned by two-bone IK.
pub mod additive;
/// AN1 skinning: a glTF clip pose remapped onto the 45-node body rig and
/// composed into one row-vector skin matrix per joint.
pub mod an1;
pub mod blend;
pub mod clip;
pub mod contract;
/// Per-slot clip crossfade over integer ticks: the fade count plus the
/// new-clip weight over the leaving pose. Std-only; the game bridge drives it.
pub mod crossfade;
/// Thousand-agent crowd tape over the LOD tiers: seeded build, global
/// nearest-eight assign, far-ring impostor handoff and per-tier cost post.
pub mod crowd_scale;
pub mod edit;
pub mod events;
/// Exertion/gait layer on locomotion features: warps weights by fatigue,
/// tunable table. Std-only.
pub mod exert;
pub mod face;
/// Full-body IK: reach, look-at, aim, two-hand and shield grips.
/// Std-only; consumes graph poses plus foot-lock leg solves, combat and
/// hit-react read it.
pub mod fbik;
/// Foot lock over analytic two-bone IK: ground clamp, slide meter.
/// Std-only; arena feel and full-body IK consume the per-leg solve, the
/// graph the pose lock.
pub mod feet;
/// Runtime animation graph as data: nodes compile to a flat order,
/// cycles refused loudly. Std-only; blend spaces, layers and event
/// tracks consume compile + eval.
pub mod graph;
/// Procedural additive hit reactions over body layers.
pub mod hit_react;
/// Baked per-move hit tracks: integer-mm blade segments and bone capsules the
/// fight reads by `(move, tick)`. Std-only; no donor bytes.
pub mod hit_track;
pub mod ik;
/// Inertialized clip switches: the shown joint's offset and offset velocity
/// from the arriving clip decay on a critically damped spring, so a cut keeps
/// position and speed continuous. Std-only; the fighter look drives it.
pub mod inertial;
pub mod inertialize;
/// Single-loss joint masks plus support polygon after limb loss: rebuilt
/// [`MaskTable`](layers::MaskTable) weights feed the layers apply path.
pub mod injured;
/// Double-loss joint masks after two limbs are lost: narrows the single-loss
/// polygon through its composition, refuses a third loss loudly.
pub mod injured_double;
pub mod layers;
pub mod lipsync;
pub mod loco_bake;
pub mod locomotion;
pub mod lod;
#[cfg(test)]
mod lod_w1_test;
#[cfg(test)]
mod lod_warp_test;
/// Offline motion bake: the PartiGen endpoint rule plus the one AM5P pack half
/// a teacher bake must use. Std-only, no I/O, never called by the tick.
pub mod motion_bake;
pub mod motion_match;
pub mod motion_net;
#[cfg(test)]
mod motion_schema_test;
/// Style and terrain dials for the motion-matching search: style and grade tags
/// from the LOC2 bake steer the integer scan. Integer-only (NS7).
pub mod motion_style;
/// Per-fighter pose-hash rings naming the first divergent tick and fighter
/// between two clients. Diagnostics only, never authoritative.
pub mod net_diverge;
pub mod net_pose;
pub mod pack;
pub mod pack_file;
/// Per-character eval on the RC3 job pool: index-ordered chunks, one owner per
/// character, poses identical under every chunking. Presentation and bake only.
pub mod par;
pub mod pose;
#[cfg(test)]
mod pose_c1_test;
pub mod readable;
/// Facing arrow plus lean at crowd LOD: one [`readable::Readable`] per LOD
/// fighter, re-evaluated on the ticks its tier owns and held in between, so the
/// far ring never strobes. Presentation only: the sim bytes cannot move.
pub mod readable_lod;
pub mod space;
/// Blend-space cues: each sample's event track speaks by integer weight
/// under an [`EventStrategy`](space_events::EventStrategy). Std-only; the
/// footstep and hit-cue path reads it after each [`space::SpacePlayer`] tick.
pub mod space_events;
#[cfg(test)]
mod space_events_test;
pub mod springs;
/// Animation state machine as versioned data: validated, undoable, travel,
/// integer-tick playback. Std-only; the JSON and door layer sits behind the
/// `json` feature.
pub mod state_machine;
/// The spear throw as a masked override on the final pose: the arm loads back,
/// sweeps to point, holds and lets go. Std-only presentation (SG3).
pub mod throw_layer;
/// Weapon-trail rings plus the FPS viewmodel pose that feeds them.
/// Std-only presentation; the fighter look draws each attacking slot's ring
/// as a view-only strip, the render forward pass reads the strip vertices.
pub mod trails;
pub mod traversal;
#[cfg(test)]
mod traversal_link_test;
#[cfg(test)]
mod traversal_rubble_var_test;
#[cfg(test)]
mod traversal_siege_test;
pub mod viseme;
/// GASP-class ground feel: distance matching, orientation warping and
/// stride warping over locomotion output. Std-only; arena presentation
/// consumes the warp, foot lock meters it.
pub mod warp_ground;
#[cfg(test)]
mod warp_ground_feel_test;
/// Slope cling, stair stepping and soft-ground sink beside the ground warp:
/// surface-speed feed, tread plants and a sink that is not skate.
pub mod warp_ground_slope;
#[cfg(test)]
mod warp_ground_slope_test;
#[cfg(test)]
mod warp_ground_test;

/// One joint track: strictly increasing times + values. Empty = invalid (sample = zero).
#[derive(Clone, Debug)]
pub struct Track {
    pub times: Vec<f32>,
    pub values: Vec<[f32; 3]>,
}

/// Clip: index = joint id. Out-of-range joint = zero.
#[derive(Clone, Debug, Default)]
pub struct Clip {
    pub tracks: Vec<Track>,
}

/// Sample: clamp t, binary-search interval, per-component lerp. Endpoints exact.
pub fn sample(clip: &Clip, joint: usize, t: f32) -> [f32; 3] {
    let tr = match clip.tracks.get(joint) {
        Some(tr) if tr.times.len() == tr.values.len() && !tr.times.is_empty() => tr,
        _ => return [0.0, 0.0, 0.0],
    };
    if t <= tr.times[0] {
        return tr.values[0];
    }
    let last = tr.times.len() - 1;
    if t >= tr.times[last] {
        return tr.values[last];
    }
    let mut lo = 0usize;
    let mut hi = last;
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if tr.times[mid] <= t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let t0 = tr.times[lo];
    let t1 = tr.times[hi];
    let span = t1 - t0;
    let f = if span <= 0.0 {
        0.0
    } else {
        clamp01((t - t0) / span)
    };
    let a = tr.values[lo];
    let b = tr.values[hi];
    [
        a[0] + (b[0] - a[0]) * f,
        a[1] + (b[1] - a[1]) * f,
        a[2] + (b[2] - a[2]) * f,
    ]
}

/// Scalar quant: per-channel uniform to u16. max==min guards div-zero.
pub fn quantize(v: [f32; 3], min: [f32; 3], max: [f32; 3]) -> [u16; 3] {
    let mut q = [0u16; 3];
    for i in 0..3 {
        let span = max[i] - min[i];
        q[i] = if span <= 0.0 {
            0
        } else {
            (clamp01((v[i] - min[i]) / span) * 65535.0)
                .round()
                .clamp(0.0, 65535.0) as u16
        };
    }
    q
}

/// Dequantize u16 back to f32.
pub fn dequantize(q: [u16; 3], min: [f32; 3], max: [f32; 3]) -> [f32; 3] {
    let mut v = [0.0f32; 3];
    for i in 0..3 {
        v[i] = min[i] + (max[i] - min[i]) * (f32::from(q[i]) / 65535.0);
    }
    v
}

/// FNV-1a64 over track count, lens, and f32 bits (same constants as ecs).
pub fn clip_hash(clip: &Clip) -> u64 {
    let mut h = Fnv64::new();
    h.write_len(clip.tracks.len());
    for tr in &clip.tracks {
        h.write_len(tr.times.len());
        for w in tr
            .times
            .iter()
            .chain(tr.values.iter().flat_map(|v| v.iter()))
        {
            h.write_f32(*w);
        }
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_key() -> Clip {
        Clip {
            tracks: vec![Track {
                times: vec![0.0, 1.0],
                values: vec![[0.0, 0.0, 0.0], [2.0, 4.0, 6.0]],
            }],
        }
    }

    #[test]
    fn sample_endpoints() {
        let c = two_key();
        assert_eq!(sample(&c, 0, -1.0), [0.0, 0.0, 0.0]);
        assert_eq!(sample(&c, 0, 0.0), [0.0, 0.0, 0.0]);
        assert_eq!(sample(&c, 0, 1.0), [2.0, 4.0, 6.0]);
        assert_eq!(sample(&c, 0, 99.0), [2.0, 4.0, 6.0]);
        assert_eq!(sample(&c, 7, 0.5), [0.0, 0.0, 0.0]);
    }

    #[test]
    fn blend_midpoint() {
        assert_eq!(
            blend::lerp3([0.0, 0.0, 0.0], [2.0, 4.0, 6.0], 0.5),
            [1.0, 2.0, 3.0]
        );
    }

    #[test]
    fn quant_roundtrip() {
        let (min, max) = ([-10.0, -10.0, -10.0], [10.0, 10.0, 10.0]);
        for v in [[0.0, 0.0, 0.0], min, max, [3.25, -7.5, 9.99]] {
            let r = dequantize(quantize(v, min, max), min, max);
            for i in 0..3 {
                assert!((r[i] - v[i]).abs() < 1e-3, "comp {i}: {r:?} vs {v:?}");
            }
        }
    }

    #[test]
    fn hash_deterministic() {
        let (a, b) = (two_key(), two_key());
        assert_eq!(clip_hash(&a), clip_hash(&b));
        let mut c = two_key();
        c.tracks[0].values[0][0] += f32::EPSILON;
        assert_ne!(clip_hash(&a), clip_hash(&c));
    }
}
