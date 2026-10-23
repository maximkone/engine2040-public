//! View truth: what the window draws, as numbers, before any pixel exists.
//!
//! Owner 2026-10-11: an AI-native engine knows its scene by math, not by
//! screenshots. The fighter look skins every body on the CPU each frame, so
//! the exact posed vertices, joints, clip and tint of every drawn body are in
//! hand before the GPU runs. This module turns them into per-body numbers
//! ([`BodyTruth`]), projects them through the frame camera
//! ([`screen_rect`]), and folds a run into laws ([`ViewLaws`]) that a script
//! or a model reads with no image: bodies off screen, sunk into the sand or
//! floating, joints that pop between frames (clip snaps), planted feet that
//! slide off their spot on the sand (`foot_slide_cm`, the p95 over
//! `planted_feet` foot-frames), floored fighters drawn from their ragdoll
//! (`ragdolls`, the most in one frame), non-finite vertices, the skin error material,
//! the camera inside a body or under the ground.
//!
//! Consumer: the arena's `arena view:` receipt line and its `--view-brief`
//! JSONL (one line per presented frame), which `tools/product-check.mjs`
//! scores against `tools/product-bar.json`; next the door's `view.observe`.
//! Units: meters in, centimeters in the receipt, sim ticks for time, picture
//! share `0..=1` for screen cover (clipped NDC rect area over 4). View only:
//! nothing here reaches the simulation or its hash. Errors: none; a
//! non-finite input counts as a `nonfinite` breach, never a panic.

use std::collections::BTreeMap;

use engine_render::v1::Camera;

use super::camera_arm::Aabb;

/// Sand top: the pit floor every body stands on, in meters.
pub const GROUND_Y: f32 = 0.0;
/// A joint that misses its constant-velocity prediction by more than this
/// in one presented frame popped: a clip snap, not motion (a swinging hand
/// misses by well under a centimeter at 60 frames a second).
pub const POP_M: f64 = 0.10;
/// Ticks a live jump may stay off the floor before the idle-float law counts
/// it again: a real jump or swing is shorter; a body that keeps hovering
/// becomes a warned idle float. One second at the 60 Hz fixed tick.
const AIR_GRACE_TICKS: f64 = 60.0;
/// Bins of the foot-slide histogram, one per millimeter: the last bin holds a
/// slide of a meter or more.
const SLIDE_BINS: usize = 1001;

/// One drawn body in one frame.
#[derive(Clone, Debug, PartialEq)]
pub struct BodyTruth {
    /// Match slot.
    pub slot: u8,
    /// What the look plays: `idle`, `run`, `attack` or `down`.
    pub act: &'static str,
    /// Clip index sampled.
    pub clip: u8,
    /// Clip time sampled, in seconds.
    pub clip_s: f32,
    /// Wears the red hit tint this frame.
    pub tinted: bool,
    /// Interpolated root position in meters.
    pub root: [f32; 3],
    /// Interpolated yaw in radians.
    pub yaw: f32,
    /// World minimum of every posed vertex (`f32::MAX` until one folds in).
    pub min: [f32; 3],
    /// World maximum of every posed vertex (`f32::MIN` until one folds in).
    pub max: [f32; 3],
    /// Joint positions in model space: root motion and yaw removed.
    pub joints: Vec<[f32; 3]>,
    /// Posed vertices or joints with a non-finite component.
    pub nonfinite: u32,
    /// Body parts drawn with the skin error material.
    pub error_parts: u32,
    /// Per foot (left, right): how far the drawn ankle sits from the spot it
    /// is planted on, in meters, while the foot is at full plant; `None` while
    /// the foot is free. The slide the `foot_slide_cm` receipt meters.
    pub foot_slide_m: [Option<f32>; 2],
    /// Head travel from the rotation flinch this frame, meters (AM23); 0 when
    /// no hit reaction is live. The max is the `react_head_cm_max` receipt.
    pub react_head_m: f32,
    /// Drawn from its ragdoll bones this frame (RZ2w), not from a clip. A ragdoll falls and
    /// tumbles under physics, so the clip-snap law does not judge its joints.
    pub ragdolled: bool,
}

impl BodyTruth {
    /// A body with no vertices folded yet; joints with a non-finite
    /// component count toward [`BodyTruth::nonfinite`] here.
    #[must_use]
    pub fn new(
        slot: u8,
        act: &'static str,
        root: [f32; 3],
        yaw: f32,
        joints: Vec<[f32; 3]>,
    ) -> Self {
        let nonfinite = joints
            .iter()
            .filter(|joint| !joint.iter().all(|v| v.is_finite()))
            .count();
        Self {
            slot,
            act,
            clip: 0,
            clip_s: 0.0,
            tinted: false,
            root,
            yaw,
            min: [f32::MAX; 3],
            max: [f32::MIN; 3],
            joints,
            nonfinite: u32::try_from(nonfinite).unwrap_or(u32::MAX),
            error_parts: 0,
            foot_slide_m: [None; 2],
            react_head_m: 0.0,
            ragdolled: false,
        }
    }

    /// Fold one posed part's world vertices into the bounds; a vertex with
    /// a non-finite component counts and stays out of the bounds.
    pub fn fold_vertices(&mut self, positions: &[[f32; 3]]) {
        for p in positions {
            if !p.iter().all(|v| v.is_finite()) {
                self.nonfinite = self.nonfinite.saturating_add(1);
                continue;
            }
            for ((lo, hi), v) in self.min.iter_mut().zip(self.max.iter_mut()).zip(p) {
                *lo = lo.min(*v);
                *hi = hi.max(*v);
            }
        }
    }

    /// True once at least one finite vertex folded in.
    #[must_use]
    pub fn has_bounds(&self) -> bool {
        let bounds = Aabb::new(self.min, self.max);
        bounds.is_finite() && bounds.is_ordered()
    }
}

/// One staged prop draw in one frame: kit and ring draws as one row each.
///
/// Consumer: the `--view-brief` prop rows via [`prop_row`]; view only, never
/// the sim hash. Units: meters for bounds. Determinism: same inputs give the
/// same row bytes. Budget: one flat row per draw, no per-tick sim work.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PropTruth {
    /// Draw kind from the staged asset name (kit file or ring floor).
    pub kind: &'static str,
    /// World minimum of the draw bounds in meters.
    pub min: [f32; 3],
    /// World maximum of the draw bounds in meters.
    pub max: [f32; 3],
}

/// Frame light as data: the sun, exposure, fog and pass mask for one frame.
///
/// Consumer: the `--view-brief` light verdict via [`light_ok`]; view only,
/// never the sim hash. Units: direction unitless, elevation radians, exposure
/// multiplier, fog share `0..=1`. Determinism: same inputs, same verdict.
/// Budget: one flat struct per frame.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FrameLight {
    /// Sun direction in world space.
    pub sun_dir: [f32; 3],
    /// Sun elevation in radians; below 0 is below the horizon.
    pub sun_elev: f32,
    /// Exposure multiplier; the brief law keeps `0.25..=4.0`.
    pub exposure: f32,
    /// Fog share `0..=1`.
    pub fog: f32,
    /// Pass mask for the frame.
    pub passes: u32,
}

/// The screen rect `[x0, y0, x1, y1]` of world bounds `min..max` in NDC
/// (`-1..=1`, y up), clipped to the picture. `None` when the bounds lie
/// wholly behind the near plane or the rect misses the picture. Bounds that
/// cross the near plane are clipped exactly: the in-front corners plus the
/// points where the 12 box edges cross the plane, so a body beside the
/// camera stays off screen and a body around the eye fills it.
#[must_use]
pub fn screen_rect(camera: &Camera, min: [f32; 3], max: [f32; 3]) -> Option<[f32; 4]> {
    let bounds = Aabb::new(min, max);
    if !bounds.is_finite() || !bounds.is_ordered() {
        return None;
    }
    let view = camera.view_matrix().ok()?;
    let half = (camera.fov_y * 0.5).tan();
    if !(half > 0.0 && camera.aspect > 0.0) {
        return None;
    }
    // View-space corners as (x, y, depth in front of the eye).
    let corners: [[f32; 3]; 8] = std::array::from_fn(|corner| {
        let pick = |axis: usize| {
            if corner & (1 << axis) == 0 {
                min[axis]
            } else {
                max[axis]
            }
        };
        let mut point = camera.eye;
        (point.x, point.y, point.z) = (pick(0), pick(1), pick(2));
        let local = view.transform_point3(point);
        [local.x, local.y, -local.z]
    });
    let near = camera.near;
    let mut kept: Vec<[f32; 3]> = corners.iter().copied().filter(|c| c[2] > near).collect();
    for (a, corner) in corners.iter().enumerate() {
        for bit in [1usize, 2, 4] {
            let Some(other) = corners.get(a | bit).filter(|_| a & bit == 0) else {
                continue;
            };
            if (corner[2] > near) != (other[2] > near) {
                let s = (near - corner[2]) / (other[2] - corner[2]);
                kept.push(std::array::from_fn(|k| {
                    corner[k] + (other[k] - corner[k]) * s
                }));
            }
        }
    }
    if kept.is_empty() {
        return None;
    }
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for [x, y, depth] in kept {
        let depth = depth.max(near);
        let x = x / (depth * half * camera.aspect);
        let y = y / (depth * half);
        lo = [lo[0].min(x), lo[1].min(y)];
        hi = [hi[0].max(x), hi[1].max(y)];
    }
    let rect = [
        lo[0].max(-1.0),
        lo[1].max(-1.0),
        hi[0].min(1.0),
        hi[1].min(1.0),
    ];
    (rect[0] < rect[2] && rect[1] < rect[3]).then_some(rect)
}

/// Share of the picture a clipped NDC rect covers, `0..=1`.
#[must_use]
pub fn cover(rect: [f32; 4]) -> f32 {
    ((rect[2] - rect[0]) * (rect[3] - rect[1]) * 0.25).clamp(0.0, 1.0)
}

/// One slot's joints at its last drawn frame, for pop prediction.
#[derive(Clone, Debug)]
struct Track {
    t: f64,
    joints: Vec<[f32; 3]>,
    /// Meters per tick from the frame before; `None` right after a pop or
    /// on the first frame, so one snap counts once.
    vel: Option<Vec<[f64; 3]>>,
    /// World root height at this frame, meters.
    root_y: f32,
    /// True while the body is off the floor with vertical motion (a jump or
    /// a fall), so the idle-float law does not count a live jump.
    airborne: bool,
    /// When this air time began, seconds; zero while on the floor.
    air_since: f64,
    /// Clip the body played at this frame, so a pop can name the clip it rode.
    clip: u8,
}

/// The worst pop of a run: which slot, which joint, when, how far the joint
/// missed its prediction, and where the joint stood.
#[derive(Clone, Copy, Debug)]
pub struct PopDetail {
    /// The slot whose joint popped.
    pub slot: u8,
    /// The joint index in the slot's joint order.
    pub joint: usize,
    /// The tick the joint popped on.
    pub tick: f64,
    /// How far the joint missed its constant-velocity prediction, meters.
    pub miss_m: f64,
    /// Where the joint stood the frame before, meters.
    pub from: [f32; 3],
    /// The world position of the joint on that tick, meters.
    pub at: [f32; 3],
    /// What the body played when it popped.
    pub act: &'static str,
    /// The clip it sampled when it popped.
    pub clip: u8,
    /// The clip's time when it popped, seconds.
    pub clip_s: f32,
    /// The clip it sampled the frame before: a different clip means a switch.
    pub prev_clip: u8,
}

/// A run's view laws: folded frame by frame, printed as one receipt line.
#[derive(Clone, Debug, Default)]
pub struct ViewLaws {
    frames: u32,
    /// Sim time of the first and last folded frame, in ticks.
    first_t: Option<f64>,
    last_t: f64,
    bodies: u32,
    offscreen: u32,
    nonfinite: u32,
    error_parts: u32,
    sunk_m: f32,
    idle_float_m: f32,
    eye_in_body: u32,
    eye_below_ground: u32,
    pops: u32,
    /// Pops by the act the body played when it popped.
    pop_acts: BTreeMap<&'static str, u32>,
    pop_m: f64,
    /// The worst pop of the run, with the joint that popped.
    pop_worst: Option<PopDetail>,
    closest_m: Option<f32>,
    cover_max: f32,
    /// Foot-frames at full plant by how far the drawn ankle sat from the spot
    /// it is planted on, one bin per millimeter; empty until the first one.
    slide_bins: Vec<u32>,
    /// Most bodies drawn from a ragdoll in one folded frame.
    ragdolls_max: u32,
    /// Furthest the rotation flinch moved a head in one folded frame, meters
    /// (AM23); 0 when no reaction was live.
    react_head_m: f32,
    /// Most spears alive at once in a folded frame (spear grapple).
    spears_max: u32,
    /// Most ropes alive at once in a folded frame (spear grapple).
    tethers_max: u32,
    tracks: Vec<Option<Track>>,
}

/// Floors the `arena view:` receipt must stay inside. Mirrors the `view_*`
/// entries of `tools/product-bar.json` (owner 2026-10-11); a test keeps the
/// two in step, so a floor is never remembered here and changed there.
/// View only, never hashed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewFloors {
    /// Deepest body bottom under the sand, in centimeters.
    pub sunk_cm_max: f32,
    /// Posed vertices or joints with a non-finite component.
    pub nonfinite: u32,
    /// Body parts drawn with the skin error material.
    pub error_parts: u32,
    /// Frames with the camera inside a drawn body.
    pub eye_in_body: u32,
    /// Frames with the camera under the sand.
    pub eye_below_ground: u32,
}

impl Default for ViewFloors {
    fn default() -> Self {
        Self {
            sunk_cm_max: 3.0,
            nonfinite: 0,
            error_parts: 0,
            eye_in_body: 0,
            eye_below_ground: 0,
        }
    }
}

impl ViewLaws {
    /// Fold one presented frame: the camera that drew it, sim time `t` in
    /// ticks (tick plus interpolation alpha) and every drawn body. This is the
    /// door and test entry point; the window splits it into [`Self::note_frame`]
    /// per drawn frame and [`Self::note_tick`] per sim tick so a cached pose or
    /// a two-tick frame cannot fake a pop.
    pub fn note(&mut self, camera: &Camera, t: f64, bodies: &[BodyTruth]) {
        self.note_frame(camera, t, bodies);
        self.note_tick(t, bodies);
    }

    /// Fold one presented frame's camera, bounds, cover, sand rest and foot
    /// slides. Joint motion is deliberately absent: [`Self::note_tick`] owns
    /// it, once per sim tick.
    pub fn note_frame(&mut self, camera: &Camera, t: f64, bodies: &[BodyTruth]) {
        self.frames = self.frames.saturating_add(1);
        self.first_t.get_or_insert(t);
        self.last_t = t;
        let eye = [camera.eye.x, camera.eye.y, camera.eye.z];
        if eye[1] < GROUND_Y {
            self.eye_below_ground = self.eye_below_ground.saturating_add(1);
        }
        if bodies
            .iter()
            .any(|body| (0..3).all(|axis| body.min[axis] < eye[axis] && eye[axis] < body.max[axis]))
        {
            self.eye_in_body = self.eye_in_body.saturating_add(1);
        }
        let ragdolls = bodies.iter().filter(|body| body.ragdolled).count();
        self.ragdolls_max = self
            .ragdolls_max
            .max(u32::try_from(ragdolls).unwrap_or(u32::MAX));
        let mut drawn = vec![false; self.tracks.len()];
        for body in bodies {
            self.bodies = self.bodies.saturating_add(1);
            self.nonfinite = self.nonfinite.saturating_add(body.nonfinite);
            self.error_parts = self.error_parts.saturating_add(body.error_parts);
            if body.react_head_m.is_finite() {
                self.react_head_m = self.react_head_m.max(body.react_head_m.max(0.0));
            }
            match screen_rect(camera, body.min, body.max) {
                Some(rect) => self.cover_max = self.cover_max.max(cover(rect)),
                None => self.offscreen = self.offscreen.saturating_add(1),
            }
            let index = usize::from(body.slot);
            let prev = self.tracks.get(index).and_then(|track| track.as_ref());
            let was_air = prev.is_some_and(|track| track.airborne);
            let rising = prev.is_some_and(|track| body.root[1] - track.root_y > 0.002);
            let air_grace =
                prev.is_some_and(|track| track.airborne && t - track.air_since < AIR_GRACE_TICKS);
            let airborne = body.min[1] - GROUND_Y > 0.01 && (was_air || rising);
            let live_air = airborne && (rising || air_grace);
            if body.has_bounds() {
                self.sunk_m = self.sunk_m.max(GROUND_Y - body.min[1]);
                if body.act == "idle" && !live_air {
                    self.idle_float_m = self.idle_float_m.max(body.min[1] - GROUND_Y);
                }
            }
            for slide in body.foot_slide_m.into_iter().flatten() {
                self.note_slide(slide);
            }
            let index = usize::from(body.slot);
            if drawn.len() <= index {
                drawn.resize(index + 1, false);
            }
            drawn[index] = true;
        }
        for (track, seen) in self.tracks.iter_mut().zip(drawn) {
            if !seen {
                *track = None;
            }
        }
        for (i, a) in bodies.iter().enumerate() {
            for b in &bodies[i + 1..] {
                if a.act != "down" && b.act != "down" {
                    let d = (a.root[0] - b.root[0]).hypot(a.root[2] - b.root[2]);
                    self.closest_m = Some(self.closest_m.map_or(d, |c| c.min(d)));
                }
            }
        }
    }

    /// Fold one sim tick's joint motion: the pop law runs once per tick, not
    /// once per presented frame, so a frame that covers two ticks or repeats a
    /// cached pose cannot invent a pop. Call after posing the tick's bodies.
    pub fn note_tick(&mut self, t: f64, bodies: &[BodyTruth]) {
        for body in bodies {
            let index = usize::from(body.slot);
            let prev = self.tracks.get(index).and_then(|track| track.as_ref());
            let was_air = prev.is_some_and(|track| track.airborne);
            let rising = prev.is_some_and(|track| body.root[1] - track.root_y > 0.002);
            let airborne = body.min[1] - GROUND_Y > 0.01 && (was_air || rising);
            self.track(t, body, airborne);
        }
    }

    /// Count one planted foot-frame by its slide in meters. A slide that is
    /// not finite counts as a `nonfinite` breach and stays out of the bins.
    fn note_slide(&mut self, slide_m: f32) {
        if !slide_m.is_finite() {
            self.nonfinite = self.nonfinite.saturating_add(1);
            return;
        }
        if self.slide_bins.is_empty() {
            self.slide_bins.resize(SLIDE_BINS, 0);
        }
        let bin = (slide_m * 1000.0).round().max(0.0) as usize;
        let bin = bin.min(SLIDE_BINS - 1);
        self.slide_bins[bin] = self.slide_bins[bin].saturating_add(1);
    }

    /// Foot-frames folded at full plant.
    #[must_use]
    pub fn planted_feet(&self) -> u32 {
        self.slide_bins
            .iter()
            .fold(0u32, |sum, n| sum.saturating_add(*n))
    }

    /// The slide, in meters, that 95 of 100 planted foot-frames stayed within
    /// (the smallest millimeter that covers them); zero with none folded.
    #[must_use]
    pub fn foot_slide_p95_m(&self) -> f32 {
        let total = u64::from(self.planted_feet());
        let rank = (total * 95).div_ceil(100);
        let mut seen = 0u64;
        for (bin, n) in self.slide_bins.iter().enumerate() {
            seen += u64::from(*n);
            if seen >= rank && seen > 0 {
                return bin as f32 / 1000.0;
            }
        }
        0.0
    }

    /// Predict each joint from the slot's last velocity; the worst miss
    /// above [`POP_M`] is one pop.
    fn track(&mut self, t: f64, body: &BodyTruth, airborne: bool) {
        let index = usize::from(body.slot);
        if self.tracks.len() <= index {
            self.tracks.resize_with(index + 1, || None);
        }
        let prev = match self.tracks[index].take() {
            Some(prev) if prev.joints.len() == body.joints.len() => prev,
            _ => {
                self.tracks[index] = Some(Track {
                    t,
                    joints: body.joints.clone(),
                    vel: None,
                    root_y: body.root[1],
                    airborne,
                    air_since: if airborne { t } else { 0.0 },
                    clip: body.clip,
                });
                return;
            }
        };
        let dt = t - prev.t;
        // A repeated sample time is not a standstill. The window avoids this
        // path for a cached pose by calling note_tick only on sim ticks; this
        // guard covers a caller that repeats the same time.
        if dt <= 1e-6 {
            self.tracks[index] = Some(prev);
            return;
        }
        if body.ragdolled {
            // Physics moves these joints, not a clip: nothing to predict, nothing to snap.
            self.tracks[index] = Some(Track {
                t,
                joints: body.joints.clone(),
                vel: None,
                root_y: body.root[1],
                airborne,
                air_since: if airborne { t } else { 0.0 },
                clip: body.clip,
            });
            return;
        }
        let mut popped = false;
        // A takeoff or a landing between frames is a real floor crossing, not
        // a pose snap: the root moves with the height channel, so the next
        // frame restarts prediction from the crossing pose instead of counting
        // one pop.
        let crossing = airborne != prev.airborne;
        if !crossing {
            if let Some(vel) = &prev.vel {
                let mut worst = 0.0f64;
                let mut worst_joint = 0usize;
                for (index, ((now, then), v)) in
                    body.joints.iter().zip(&prev.joints).zip(vel).enumerate()
                {
                    let miss = (0..3)
                        .map(|k| (f64::from(now[k]) - (f64::from(then[k]) + v[k] * dt)).powi(2))
                        .sum::<f64>()
                        .sqrt();
                    if miss > worst {
                        worst = miss;
                        worst_joint = index;
                    }
                }
                self.pop_m = self.pop_m.max(worst);
                if worst > POP_M {
                    self.pops = self.pops.saturating_add(1);
                    *self.pop_acts.entry(body.act).or_insert(0) += 1;
                    self.note_pop(
                        body.slot,
                        worst_joint,
                        t,
                        worst,
                        prev.joints[worst_joint],
                        body.joints[worst_joint],
                        body.act,
                        body.clip,
                        body.clip_s,
                        prev.clip,
                    );
                    popped = true;
                }
            }
        }
        let vel = (!popped && !crossing).then(|| {
            body.joints
                .iter()
                .zip(&prev.joints)
                .map(|(now, then)| {
                    std::array::from_fn(|k| (f64::from(now[k]) - f64::from(then[k])) / dt)
                })
                .collect()
        });
        self.tracks[index] = Some(Track {
            t,
            joints: body.joints.clone(),
            vel,
            root_y: body.root[1],
            airborne,
            air_since: if airborne {
                if prev.airborne {
                    prev.air_since
                } else {
                    t
                }
            } else {
                0.0
            },
            clip: body.clip,
        });
    }

    /// Keeps the worst pop of the run: the joint, the tick and the miss.
    #[expect(
        clippy::too_many_arguments,
        reason = "one field per PopDetail column; kept flat until the pop moves into a params struct"
    )]
    fn note_pop(
        &mut self,
        slot: u8,
        joint: usize,
        tick: f64,
        miss_m: f64,
        from: [f32; 3],
        at: [f32; 3],
        act: &'static str,
        clip: u8,
        clip_s: f32,
        prev_clip: u8,
    ) {
        let worse = self.pop_worst.is_none_or(|worst| miss_m > worst.miss_m);
        if worse {
            self.pop_worst = Some(PopDetail {
                slot,
                joint,
                tick,
                miss_m,
                from,
                at,
                act,
                clip,
                clip_s,
                prev_clip,
            });
        }
    }

    /// The worst pop of the run: which slot and joint, when, and how far the
    /// joint missed its constant-velocity prediction.
    #[must_use]
    pub fn pop_detail(&self) -> Option<PopDetail> {
        self.pop_worst
    }

    /// Most bodies drawn from a ragdoll in one folded frame (RZ2w).
    #[must_use]
    pub fn ragdolls(&self) -> u32 {
        self.ragdolls_max
    }

    /// Furthest the rotation flinch moved a head in one folded frame, meters
    /// (AM23); 0 when no reaction was live. `react_head_cm_max` prints this
    /// times 100.
    #[must_use]
    pub fn react_head_m(&self) -> f32 {
        self.react_head_m
    }

    /// Folds the spear grapple's counts for the frame just drawn: spears and ropes alive.
    pub fn note_spear(&mut self, spears: usize, tethers: usize) {
        let clamp = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        self.spears_max = self.spears_max.max(clamp(spears));
        self.tethers_max = self.tethers_max.max(clamp(tethers));
    }

    /// Most spears and most ropes alive at once in a folded frame.
    #[must_use]
    pub fn spear_counts(&self) -> (u32, u32) {
        (self.spears_max, self.tethers_max)
    }

    /// Bodies drawn with no on-screen rect, over bodies drawn.
    #[must_use]
    pub fn offscreen_share(&self) -> f64 {
        f64::from(self.offscreen) / f64::from(self.bodies.max(1))
    }

    /// The `arena view:` receipt line; `None` when no frame was folded.
    #[must_use]
    pub fn line(&self) -> Option<String> {
        if self.frames == 0 {
            return None;
        }
        let closest = self
            .closest_m
            .map_or_else(|| "none".to_string(), |m| format!("{:.0}", m * 100.0));
        let acts = if self.pop_acts.is_empty() {
            "none".to_string()
        } else {
            let parts: Vec<String> = self
                .pop_acts
                .iter()
                .map(|(act, n)| format!("{act}:{n}"))
                .collect();
            parts.join("/")
        };
        Some(format!(
            "arena view: frames={} sim_ticks={:.1}..{:.1} bodies={} offscreen_share={:.2} nonfinite={} error_parts={} sunk_cm_max={:.1} idle_float_cm_max={:.1} pops={} pop_acts={acts} pop_cm_max={:.1} react_head_cm_max={:.1} eye_in_body={} eye_below_ground={} closest_cm={closest} cover_max={:.2} foot_slide_cm={:.1} planted_feet={} ragdolls={} spear={} tether={}",
            self.frames,
            self.first_t.unwrap_or(0.0),
            self.last_t,
            self.bodies,
            self.offscreen_share(),
            self.nonfinite,
            self.error_parts,
            self.sunk_m * 100.0,
            self.idle_float_m * 100.0,
            self.pops,
            self.pop_m * 100.0,
            self.react_head_m * 100.0,
            self.eye_in_body,
            self.eye_below_ground,
            self.cover_max,
            self.foot_slide_p95_m() * 100.0,
            self.planted_feet(),
            self.ragdolls(),
            self.spears_max,
            self.tethers_max,
        ))
    }

    /// Every floor this run breaks, one `arena WARN:` line each, in receipt
    /// order: counters first, then the sand. A run inside every floor warns
    /// nothing. The warning is not a refusal: the caller keeps its exit code,
    /// and `tools/product-check.mjs` still owns the gate.
    #[must_use]
    pub fn warnings(&self) -> Vec<String> {
        self.warnings_with(&ViewFloors::default())
    }

    /// The run's view verdict for the capture line: `ok`, or `FAIL(` and the
    /// broken floors' metrics `)`, e.g. `FAIL(view_eye_in_body)`. The WARN
    /// lines alone went unread: a run with the camera inside a giant body
    /// still printed a clean-looking capture line (2026-10-22), so the
    /// verdict rides on the one line every caller reads.
    #[must_use]
    pub fn verdict(&self) -> String {
        let broken: Vec<String> = self
            .warnings()
            .iter()
            .filter_map(|w| w.strip_prefix("arena WARN: ")?.split_whitespace().next())
            .map(str::to_string)
            .collect();
        if broken.is_empty() {
            "ok".to_string()
        } else {
            format!("FAIL({})", broken.join(","))
        }
    }

    /// [`Self::warnings`] against explicit floors; the bar-pin test reads the
    /// product bar this way.
    #[must_use]
    pub fn warnings_with(&self, floors: &ViewFloors) -> Vec<String> {
        let mut out = Vec::new();
        if self.nonfinite > floors.nonfinite {
            out.push(format!(
                "arena WARN: view_nonfinite value={} floor={}",
                self.nonfinite, floors.nonfinite
            ));
        }
        if self.error_parts > floors.error_parts {
            out.push(format!(
                "arena WARN: view_error_parts value={} floor={}",
                self.error_parts, floors.error_parts
            ));
        }
        if self.eye_in_body > floors.eye_in_body {
            out.push(format!(
                "arena WARN: view_eye_in_body value={} floor={}",
                self.eye_in_body, floors.eye_in_body
            ));
        }
        if self.eye_below_ground > floors.eye_below_ground {
            out.push(format!(
                "arena WARN: view_eye_below_ground value={} floor={}",
                self.eye_below_ground, floors.eye_below_ground
            ));
        }
        let sunk_cm = self.sunk_m * 100.0;
        if sunk_cm > floors.sunk_cm_max {
            out.push(format!(
                "arena WARN: view_sunk_cm_max value={sunk_cm:.1} floor={:.1}",
                floors.sunk_cm_max
            ));
        }
        out
    }
}

/// A finite number to 3 places, else JSON `null`.
fn num(v: f32) -> String {
    if v.is_finite() {
        format!("{v:.3}")
    } else {
        "null".to_string()
    }
}

fn vec3(v: [f32; 3]) -> String {
    format!("[{},{},{}]", num(v[0]), num(v[1]), num(v[2]))
}

/// One frame of the `--view-brief` JSONL: camera, then every drawn body with
/// its act, clip, tint, root, bounds, screen rect and cover. Same inputs,
/// same bytes.
#[must_use]
pub fn brief_line(frame: u32, t: f64, camera: &Camera, bodies: &[BodyTruth]) -> String {
    let rows: Vec<String> = bodies
        .iter()
        .map(|b| {
            let rect = screen_rect(camera, b.min, b.max);
            let screen = rect.map_or_else(
                || "null".to_string(),
                |r| format!("[{},{},{},{}]", num(r[0]), num(r[1]), num(r[2]), num(r[3])),
            );
            let (min, max) = if b.has_bounds() { (vec3(b.min), vec3(b.max)) } else { ("null".to_string(), "null".to_string()) };
            format!(
                "{{\"slot\":{},\"act\":\"{}\",\"clip\":{},\"clip_s\":{},\"tint\":{},\"root\":{},\"yaw\":{},\"min\":{min},\"max\":{max},\"screen\":{screen},\"cover\":{},\"nonfinite\":{},\"error_parts\":{}}}",
                b.slot,
                b.act,
                b.clip,
                num(b.clip_s),
                b.tinted,
                vec3(b.root),
                num(b.yaw),
                num(rect.map_or(0.0, cover)),
                b.nonfinite,
                b.error_parts,
            )
        })
        .collect();
    format!(
        "{{\"frame\":{frame},\"t\":{t:.3},\"eye\":{},\"target\":{},\"fov_deg\":{},\"bodies\":[{}]}}",
        vec3(camera.eye.to_array()),
        vec3(camera.target.to_array()),
        num(camera.fov_y.to_degrees()),
        rows.join(","),
    )
}

/// One prop draw as one flat brief row: kind, bounds, screen rect and cover.
///
/// Reuses [`screen_rect`] and [`cover`]: missing (`min > max` on any axis)
/// or wholly behind-near bounds give screen `null` and cover `0`. Same
/// inputs, same bytes. View only; no float reaches any hash.
///
/// Consumer: the `--view-brief` prop list. Units: meters in, picture share
/// `0..=1` for cover. Errors: none; bad bounds render as `null`, never panic.
#[must_use]
pub fn prop_row(camera: &Camera, kind: &'static str, min: [f32; 3], max: [f32; 3]) -> String {
    let rect = screen_rect(camera, min, max);
    let screen = rect.map_or_else(
        || "null".to_string(),
        |r| format!("[{},{},{},{}]", num(r[0]), num(r[1]), num(r[2]), num(r[3])),
    );
    let (min_text, max_text) = if (0..3).all(|axis| min[axis] <= max[axis]) {
        (vec3(min), vec3(max))
    } else {
        ("null".to_string(), "null".to_string())
    };
    format!(
        "{{\"kind\":\"{kind}\",\"min\":{min_text},\"max\":{max_text},\"screen\":{screen},\"cover\":{}}}",
        num(rect.map_or(0.0, cover)),
    )
}

/// True when the frame light sits inside the brief laws: sun at or above the
/// horizon and exposure inside `0.25..=4.0`.
///
/// View only; feeds the brief text, never the sim hash. Units: elevation
/// radians, exposure multiplier. Errors: none.
#[must_use]
pub fn light_ok(light: &FrameLight) -> bool {
    if light.sun_elev < 0.0 {
        return false;
    }
    if !(0.25..=4.0).contains(&light.exposure) {
        return false;
    }
    true
}

/// A prop no taller than this is ground a foot stands on (the dune sand
/// slab, a kerb), never a volume a fighter is trapped in: feet sunk into it
/// are the `sunk_cm` law's business, not `inside_props`. Meters.
pub const STEP_M: f32 = 0.3;

/// True when the body root (feet) sits inside the prop volume: X and Z
/// within bounds, Y from the base face up to but below the top face. A
/// fighter standing on the ring floor or atop a prop is on it, not in it;
/// one clipped into the volume counts as inside. A prop no taller than
/// [`STEP_M`] is ground and never holds a fighter inside.
///
/// View only; feeds the brief text, never the sim hash. Units: meters.
/// Errors: none; a non-finite root or prop bound never counts as inside.
#[must_use]
pub fn fighter_in_prop(body: &BodyTruth, prop: &PropTruth) -> bool {
    let feet = body.root;
    if !feet.iter().all(|v| v.is_finite()) {
        return false;
    }
    if !prop.min.iter().all(|v| v.is_finite()) || !prop.max.iter().all(|v| v.is_finite()) {
        return false;
    }
    let in_xz = [0usize, 2]
        .iter()
        .all(|&axis| prop.min[axis] <= feet[axis] && feet[axis] <= prop.max[axis]);
    let solid = prop.max[1] - prop.min[1] > STEP_M;
    solid && in_xz && prop.min[1] <= feet[1] && feet[1] < prop.max[1]
}

/// One frame of the `--view-brief` JSONL with props and light: the bodies
/// line plus every kit and ring draw as one row each, plus the frame light
/// and its laws.
///
/// Reuses [`brief_line`] for the bodies bytes (identical prefix), [`prop_row`]
/// per draw, [`light_ok`] and [`fighter_in_prop`] for the verdicts.
///
/// Consumer: `crates/game/src/bin/arena.rs` `--view-brief` writer, which builds
/// `props` from `arena_kit::kit_prop_rows` plus `fight_ring::ring_prop_row`.
/// Units: meters in, picture share `0..=1` for cover, elevation radians,
/// exposure multiplier, fog share `0..=1`. Determinism: same inputs give the
/// same bytes (props in caller order, no sorting). Budget: one string per
/// brief frame on the presentation path only (built only when the flag is on);
/// zero per tick. Errors: none; never a panic.
#[must_use]
pub fn brief_line_with_props(
    frame: u32,
    t: f64,
    camera: &Camera,
    bodies: &[BodyTruth],
    props: &[PropTruth],
    light: &FrameLight,
) -> String {
    let base = brief_line(frame, t, camera, bodies);
    let prop_rows: Vec<String> = props
        .iter()
        .map(|prop| prop_row(camera, prop.kind, prop.min, prop.max))
        .collect();
    let inside = bodies
        .iter()
        .filter(|body| props.iter().any(|prop| fighter_in_prop(body, prop)))
        .count();
    let light_text = format!(
        "{{\"sun_dir\":{},\"sun_elev\":{},\"exposure\":{},\"fog\":{},\"passes\":{}}}",
        vec3(light.sun_dir),
        num(light.sun_elev),
        num(light.exposure),
        num(light.fog),
        light.passes,
    );
    let mut out = base;
    out.pop();
    out.push_str(&format!(
        ",\"props\":[{}],\"light\":{light_text},\"light_ok\":{},\"inside_props\":{inside}}}",
        prop_rows.join(","),
        light_ok(light),
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> Camera {
        Camera::new([0.0, 1.5, 6.0], [0.0, 1.0, 0.0], [0.0, 1.0, 0.0], 1.0)
    }

    fn body(slot: u8, act: &'static str, root: [f32; 3], joints: Vec<[f32; 3]>) -> BodyTruth {
        let mut b = BodyTruth::new(slot, act, root, 0.0, joints);
        b.fold_vertices(&[
            [root[0] - 0.3, root[1], root[2] - 0.3],
            [root[0] + 0.3, root[1] + 1.8, root[2] + 0.3],
        ]);
        b
    }

    #[test]
    fn a_body_ahead_is_on_screen_and_one_behind_or_aside_is_not() {
        let cam = camera();
        let ahead = screen_rect(&cam, [-0.3, 0.0, -0.3], [0.3, 1.8, 0.3]).expect("ahead is drawn");
        assert!(ahead[0] < 0.0 && ahead[2] > 0.0, "centered: {ahead:?}");
        assert!(
            cover(ahead) > 0.01 && cover(ahead) < 0.5,
            "a person 6 m off: {}",
            cover(ahead)
        );
        assert_eq!(
            screen_rect(&cam, [-0.3, 0.0, 9.7], [0.3, 1.8, 10.3]),
            None,
            "behind the camera"
        );
        assert_eq!(
            screen_rect(&cam, [40.0, 0.0, -0.3], [40.6, 1.8, 0.3]),
            None,
            "far aside"
        );
        assert_eq!(
            screen_rect(&cam, [-0.5, 1.0, 5.5], [0.5, 2.0, 6.5]),
            Some([-1.0, -1.0, 1.0, 1.0]),
            "the eye sits inside the bounds"
        );
        assert_eq!(
            screen_rect(&cam, [f32::MAX; 3], [f32::MIN; 3]),
            None,
            "no vertices, no rect"
        );
        // Beside the camera, crossing its plane 8 m to the right: off screen,
        // not "fills the picture" (the arena brief caught this, 2026-10-11).
        assert_eq!(
            screen_rect(&cam, [8.0, 0.0, 5.0], [8.6, 1.8, 7.0]),
            None,
            "beside the eye"
        );
    }

    #[test]
    fn a_live_jump_is_not_idle_float_but_a_hover_is() {
        let cam = camera();
        let mut laws = ViewLaws::default();
        let at = |y: f32| body(0, "idle", [0.0, y, 0.0], vec![[0.0, y + 1.0, 0.0]]);
        laws.note(&cam, 0.0, &[at(0.0)]);
        laws.note(&cam, 1.0, &[at(0.08)]);
        laws.note(&cam, 2.0, &[at(0.30)]);
        assert_eq!(laws.idle_float_m * 100.0, 0.0, "a jump is not idle float");
        // The same height with no vertical motion after the grace is a hover.
        laws.note(&cam, 180.0, &[at(0.30)]);
        assert!(
            laws.idle_float_m * 100.0 > 20.0,
            "a hover flags after the grace: {}",
            laws.idle_float_m * 100.0
        );
    }

    #[test]
    fn bounds_reuse_the_camera_arm_owner() {
        // `has_bounds` and `screen_rect` read through the shared `Aabb`
        // owner: finite ordered bounds pass, anything else refuses.
        let mut fresh = BodyTruth::new(0, "idle", [0.0; 3], 0.0, Vec::new());
        assert!(!fresh.has_bounds(), "no vertices, no bounds");
        fresh.fold_vertices(&[[0.0, 0.0, 0.0], [1.0, 1.8, 0.6]]);
        assert!(fresh.has_bounds(), "folded vertices bound");
        // Non-finite bounds never count, even when ordered: no finite
        // vertex folded in, so there is nothing to draw.
        fresh.min = [f32::INFINITY; 3];
        fresh.max = [f32::INFINITY; 3];
        assert!(!fresh.has_bounds(), "infinite bounds are not bounds");
        // Unordered bounds never count either.
        fresh.min = [1.0, 0.0, 0.0];
        fresh.max = [0.0, 1.0, 1.0];
        assert!(!fresh.has_bounds(), "min above max is not bounds");
        // The rect refuses the same inputs instead of projecting them.
        let cam = camera();
        assert_eq!(
            screen_rect(&cam, [0.0, 0.0, 0.0], [f32::INFINITY, 1.8, 0.6]),
            None,
            "infinite extent has no rect"
        );
        assert_eq!(
            screen_rect(&cam, [1.0, 0.0, 0.0], [0.0, 1.0, 1.0]),
            None,
            "unordered bounds have no rect"
        );
    }

    #[test]
    fn smooth_joints_never_pop_and_one_snap_counts_once() {
        let cam = camera();
        let mut laws = ViewLaws::default();
        for frame in 0..8u8 {
            let t = f64::from(frame) * 0.5;
            let x = f32::from(frame) * 0.02;
            laws.note(
                &cam,
                t,
                &[body(0, "run", [0.0; 3], vec![[x, 1.0, 0.0], [x, 1.5, 0.1]])],
            );
        }
        assert_eq!(laws.pops, 0, "steady motion is predicted");
        // The hand jumps 0.5 m in one frame (a clip snap), then moves on smoothly.
        laws.note(
            &cam,
            4.0,
            &[body(
                0,
                "run",
                [0.0; 3],
                vec![[0.66, 1.0, 0.0], [0.16, 1.5, 0.1]],
            )],
        );
        assert_eq!(laws.pops, 1);
        for frame in 9..12u8 {
            let t = f64::from(frame) * 0.5;
            let x = 0.66 + f32::from(frame - 8) * 0.02;
            laws.note(
                &cam,
                t,
                &[body(
                    0,
                    "run",
                    [0.0; 3],
                    vec![[x, 1.0, 0.0], [x - 0.5, 1.5, 0.1]],
                )],
            );
        }
        assert_eq!(laws.pops, 1, "the frame after a snap re-seeds, no echo");
        assert!(laws
            .line()
            .unwrap_or_default()
            .contains(" pops=1 pop_acts=run:1 "));
        assert!(
            laws.pop_m > 0.4 && laws.pop_m < 0.6,
            "worst miss {}",
            laws.pop_m
        );
    }

    #[test]
    fn a_repeated_pose_keeps_its_velocity_and_the_next_fresh_pose_does_not_pop() {
        let cam = camera();
        let mut laws = ViewLaws::default();
        for (i, t) in [0.0, 0.5, 1.0, 1.5].into_iter().enumerate() {
            let x = i as f32 * 0.3;
            laws.note(
                &cam,
                t,
                &[body(0, "run", [0.0; 3], vec![[x, 1.0, 0.0], [x, 1.5, 0.1]])],
            );
        }
        assert_eq!(laws.pops, 0, "steady motion is predicted");
        // Same pose at a later frame time: a cache, not a standstill. Use the
        // exact value the loop produced (3 * 0.3 in f32 is not the literal 0.9).
        let repeated = 3.0f32 * 0.3;
        laws.note_frame(
            &cam,
            2.0,
            &[body(
                0,
                "run",
                [0.0; 3],
                vec![[repeated, 1.0, 0.0], [repeated, 1.5, 0.1]],
            )],
        );
        // The next sim tick continues the old velocity; a zeroed velocity
        // would predict 0.9 and count a 0.6 m pop. The window poses per tick,
        // not per cached frame, so the cached frame above adds no track.
        laws.note_frame(
            &cam,
            2.5,
            &[body(
                0,
                "run",
                [0.0; 3],
                vec![[1.5, 1.0, 0.0], [1.5, 1.5, 0.1]],
            )],
        );
        laws.note_tick(
            2.5,
            &[body(
                0,
                "run",
                [0.0; 3],
                vec![[1.5, 1.0, 0.0], [1.5, 1.5, 0.1]],
            )],
        );
        assert_eq!(
            laws.pops, 0,
            "a cached frame must not zero the tick velocity"
        );
    }

    #[test]
    fn a_tick_note_counts_joint_motion_without_a_presented_frame() {
        let mut laws = ViewLaws::default();
        for tick in 0..4u8 {
            let t = f64::from(tick) + 0.5;
            let x = f32::from(tick) * 0.02;
            laws.note_tick(
                t,
                &[body(0, "run", [0.0; 3], vec![[x, 1.0, 0.0], [x, 1.5, 0.1]])],
            );
        }
        assert_eq!(laws.frames, 0, "a tick note draws no frame");
        assert_eq!(laws.pops, 0, "steady joint motion is predicted");
    }

    #[test]
    fn breaches_count_and_the_receipt_line_is_exact() {
        let cam = camera();
        let mut laws = ViewLaws::default();
        assert_eq!(laws.line(), None);
        let mut sunk = body(1, "down", [1.0, -0.12, 0.0], vec![[0.0, 0.2, 0.0]]);
        sunk.fold_vertices(&[[f32::NAN, 0.0, 0.0]]);
        sunk.error_parts = 1;
        let floating = body(0, "idle", [-1.0, 0.04, 0.0], vec![[0.0, 1.0, 0.0]]);
        let aside = body(2, "idle", [40.0, 0.0, 0.0], vec![[0.0, 1.0, 0.0]]);
        laws.note(&cam, 1.0, &[floating, sunk, aside]);
        let mut under = camera();
        under.eye.y = -0.5;
        laws.note(&under, 2.0, &[]);
        assert_eq!(
            laws.line().as_deref(),
            Some(
                "arena view: frames=2 sim_ticks=1.0..2.0 bodies=3 offscreen_share=0.33 nonfinite=1 error_parts=1 sunk_cm_max=12.0 idle_float_cm_max=4.0 pops=0 pop_acts=none pop_cm_max=0.0 react_head_cm_max=0.0 eye_in_body=0 eye_below_ground=1 closest_cm=4100 cover_max=0.02 foot_slide_cm=0.0 planted_feet=0 ragdolls=0 spear=0 tether=0"
            )
        );
        // The run warns by itself, one line per broken floor, receipt order.
        assert_eq!(
            laws.warnings(),
            vec![
                "arena WARN: view_nonfinite value=1 floor=0".to_string(),
                "arena WARN: view_error_parts value=1 floor=0".to_string(),
                "arena WARN: view_eye_below_ground value=1 floor=0".to_string(),
                "arena WARN: view_sunk_cm_max value=12.0 floor=3.0".to_string(),
            ]
        );
        // The capture line carries the verdict, so no caller has to read
        // the WARN lines to learn the view broke.
        assert_eq!(
            laws.verdict(),
            "FAIL(view_nonfinite,view_error_parts,view_eye_below_ground,view_sunk_cm_max)"
        );
    }

    #[test]
    fn react_head_metric_folds_the_max() {
        let mut laws = ViewLaws::default();
        let mut first = body(0, "idle", [-1.0, 0.0, 0.0], vec![[0.0, 1.0, 0.0]]);
        first.react_head_m = 0.03;
        let mut second = body(1, "idle", [1.0, 0.0, 0.0], vec![[0.0, 1.0, 0.0]]);
        second.react_head_m = 0.11;
        laws.note(&camera(), 1.0, &[first, second]);
        assert_eq!(laws.react_head_m(), 0.11);
        assert!(
            laws.line()
                .as_deref()
                .is_some_and(|line| line.contains("react_head_cm_max=11.0")),
            "the receipt prints the max in centimeters: {:?}",
            laws.line()
        );
    }

    #[test]
    fn a_clean_run_warns_about_nothing() {
        let mut laws = ViewLaws::default();
        laws.note(
            &camera(),
            1.0,
            &[body(0, "idle", [0.0, 0.0, 0.0], vec![[0.0, 1.0, 0.0]])],
        );
        assert!(laws.line().is_some(), "the run still prints its receipt");
        assert!(laws.warnings().is_empty(), "a clean run is silent");
        assert_eq!(
            laws.verdict(),
            "ok",
            "a clean run reads ok on the capture line"
        );
    }

    #[test]
    fn the_warning_floors_are_the_product_bar_floors() {
        // One owner of the floors: the bar file the gate reads. A change on
        // either side that forgets the other fails here.
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/product-bar.json");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("{}: {err}", path.display()));
        let floors = ViewFloors::default();
        for (key, want) in [
            ("view_sunk_cm_max", f64::from(floors.sunk_cm_max)),
            ("view_nonfinite", f64::from(floors.nonfinite)),
            ("view_error_parts", f64::from(floors.error_parts)),
            ("view_eye_in_body", f64::from(floors.eye_in_body)),
            ("view_eye_below_ground", f64::from(floors.eye_below_ground)),
        ] {
            assert_eq!(json_floor(&text, key), want, "{key} matches the bar");
        }
    }

    /// The number `key` carries in the product bar's flat JSON. This crate
    /// keeps no serde dependency for one small file.
    fn json_floor(text: &str, key: &str) -> f64 {
        let at = text
            .find(&format!("\"{key}\""))
            .unwrap_or_else(|| panic!("the product bar names {key}"));
        let rest = &text[at + key.len() + 2..];
        let colon = rest.find(':').expect("a key has a value");
        let value = rest[colon + 1..].trim_start();
        let end = value
            .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
            .unwrap_or(value.len());
        value[..end].parse().expect("the floor is a number")
    }

    #[test]
    fn foot_slide_p95_reads_the_planted_frames_and_nothing_else() {
        let cam = camera();
        let mut laws = ViewLaws::default();
        assert_eq!(laws.foot_slide_p95_m(), 0.0, "nothing planted reads zero");
        assert_eq!(laws.planted_feet(), 0);
        // 100 planted foot-frames slide 1 mm, 2 mm, ... 100 mm; the other foot
        // is free, so it adds nothing.
        for mm in 1..=100u16 {
            let mut planted = body(0, "idle", [0.0; 3], vec![[0.0, 1.0, 0.0]]);
            planted.foot_slide_m = [Some(f32::from(mm) / 1000.0), None];
            laws.note(&cam, f64::from(mm), &[planted]);
        }
        assert_eq!(laws.planted_feet(), 100);
        assert!(
            (laws.foot_slide_p95_m() - 0.095).abs() < 1e-6,
            "p95 of 1..=100 mm is 95 mm, got {}",
            laws.foot_slide_p95_m()
        );
        let line = laws.line().expect("frames folded");
        assert!(
            line.ends_with(" foot_slide_cm=9.5 planted_feet=100 ragdolls=0 spear=0 tether=0"),
            "{line}"
        );
        // One slide of five meters lands in the last bin and moves p95 by one
        // rank; a NaN slide is a `nonfinite` breach and is not a planted frame.
        let mut wild = body(0, "idle", [0.0; 3], vec![[0.0, 1.0, 0.0]]);
        wild.foot_slide_m = [Some(f32::NAN), Some(5.0)];
        laws.note(&cam, 101.0, &[wild]);
        assert_eq!(laws.planted_feet(), 101);
        assert!(
            (laws.foot_slide_p95_m() - 0.096).abs() < 1e-6,
            "{}",
            laws.foot_slide_p95_m()
        );
        let line = laws.line().expect("frames folded");
        assert!(line.contains(" nonfinite=1 "), "{line}");
        assert!(
            line.ends_with(" planted_feet=101 ragdolls=0 spear=0 tether=0"),
            "{line}"
        );
    }

    #[test]
    fn the_brief_names_every_body_and_repeats_byte_for_byte() {
        let cam = camera();
        let mut tinted = body(3, "attack", [0.5, 0.0, 0.0], vec![[0.0, 1.0, 0.0]]);
        tinted.tinted = true;
        tinted.clip = 2;
        tinted.clip_s = 0.25;
        let mut empty = BodyTruth::new(4, "idle", [f32::NAN, 0.0, 0.0], 0.0, vec![]);
        empty.yaw = f32::INFINITY;
        let a = brief_line(7, 21.5, &cam, &[tinted.clone(), empty.clone()]);
        assert_eq!(a, brief_line(7, 21.5, &cam, &[tinted, empty]));
        assert!(
            a.starts_with("{\"frame\":7,\"t\":21.500,\"eye\":[0.000,1.500,6.000]"),
            "{a}"
        );
        assert!(
            a.contains("\"slot\":3,\"act\":\"attack\",\"clip\":2,\"clip_s\":0.250,\"tint\":true"),
            "{a}"
        );
        assert!(a.contains("\"slot\":4,\"act\":\"idle\",\"clip\":0,\"clip_s\":0.000,\"tint\":false,\"root\":[null,0.000,0.000],\"yaw\":null,\"min\":null,\"max\":null,\"screen\":null,\"cover\":0.000"), "{a}");
    }

    #[test]
    fn brief_props_list_every_draw_once() {
        let cam = camera();
        let pillar_min = [-0.5, 0.0, -0.5];
        let pillar_max = [0.5, 3.0, 0.5];
        let wall_min = [4.5, 0.0, -0.5];
        let wall_max = [5.5, 3.0, 0.5];
        let _at_origin = body(0, "idle", [0.0, 0.0, 0.0], vec![[0.0, 1.0, 0.0]]);
        let pillar = prop_row(&cam, "pillar", pillar_min, pillar_max);
        let wall = prop_row(&cam, "wall", wall_min, wall_max);
        assert_ne!(pillar, wall, "one row per draw kind");
        assert!(pillar.contains("\"kind\":\"pillar\""), "{pillar}");
        assert!(wall.contains("\"kind\":\"wall\""), "{wall}");
        assert_eq!(pillar, prop_row(&cam, "pillar", pillar_min, pillar_max));
        assert_eq!(wall, prop_row(&cam, "wall", wall_min, wall_max));
    }

    #[test]
    fn brief_light_laws_hold() {
        let good = FrameLight {
            sun_dir: [0.0, 1.0, 0.0],
            sun_elev: 0.5,
            exposure: 1.0,
            fog: 0.0,
            passes: 1,
        };
        assert!(light_ok(&good));
        let below = FrameLight {
            sun_elev: -0.1,
            ..good
        };
        assert!(!light_ok(&below), "sun below the horizon fails");
        let dim = FrameLight {
            exposure: 0.1,
            ..good
        };
        assert!(!light_ok(&dim), "exposure 0.1 fails");
        let hot = FrameLight {
            exposure: 5.0,
            ..good
        };
        assert!(!light_ok(&hot), "exposure 5.0 fails");
    }

    #[test]
    fn brief_fighter_inside_pillar_breaks_law() {
        let pillar = PropTruth {
            kind: "pillar",
            min: [-0.5, 0.0, -0.5],
            max: [0.5, 3.0, 0.5],
        };
        let inside = body(0, "idle", [0.0, 1.0, 0.0], vec![[0.0, 1.0, 0.0]]);
        assert!(fighter_in_prop(&inside, &pillar));
        let outside = body(1, "idle", [5.0, 0.0, 0.0], vec![[0.0, 1.0, 0.0]]);
        assert!(!fighter_in_prop(&outside, &pillar));
    }

    #[test]
    fn fighter_in_prop_refuses_nonfinite() {
        let pillar = PropTruth {
            kind: "pillar",
            min: [-0.5, 0.0, -0.5],
            max: [0.5, 3.0, 0.5],
        };
        let finite = body(0, "idle", [0.0, 1.0, 0.0], vec![[0.0, 1.0, 0.0]]);
        assert!(fighter_in_prop(&finite, &pillar));
        let wild = body(0, "idle", [f32::NAN, 1.0, 0.0], vec![[0.0, 1.0, 0.0]]);
        assert!(!fighter_in_prop(&wild, &pillar), "NaN root stays outside");
        let infinite = PropTruth {
            kind: "pillar",
            min: [f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY],
            max: [f32::INFINITY, f32::INFINITY, f32::INFINITY],
        };
        assert!(
            !fighter_in_prop(&finite, &infinite),
            "infinite prop never contains"
        );
    }

    #[test]
    fn brief_with_props_lists_every_kit_draw_once_and_repeats() {
        use crate::arena_kit::kit_prop_rows;
        use crate::fight_ring::ring_prop_row;
        let cam = camera();
        let kit = kit_prop_rows();
        assert!(!kit.is_empty(), "placement stages kit draws");
        let ring = ring_prop_row();
        let mut props: Vec<PropTruth> = kit
            .iter()
            .map(|(kind, bounds)| PropTruth {
                kind,
                min: bounds[0],
                max: bounds[1],
            })
            .collect();
        props.push(PropTruth {
            kind: ring.0,
            min: ring.1[0],
            max: ring.1[1],
        });
        let bodies = [body(0, "idle", [0.0, 0.0, 0.0], vec![[0.0, 1.0, 0.0]])];
        let light = FrameLight {
            sun_dir: [0.0, 1.0, 0.0],
            sun_elev: 0.5,
            exposure: 1.0,
            fog: 0.0,
            passes: 1,
        };
        let a = brief_line_with_props(3, 9.0, &cam, &bodies, &props, &light);
        assert_eq!(
            a,
            brief_line_with_props(3, 9.0, &cam, &bodies, &props, &light),
            "same inputs give same brief bytes"
        );
        for (kind, _) in &kit {
            let needle = format!("\"kind\":\"{kind}\"");
            let draws = kit.iter().filter(|(other, _)| other == kind).count();
            assert_eq!(
                a.matches(&needle).count(),
                draws,
                "kit draw {kind} lists once per draw"
            );
        }
        assert!(
            a.contains("\"kind\":\"ring_floor\""),
            "ring draw lists once: {a}"
        );
        assert!(a.contains("\"light_ok\":true"), "good light holds: {a}");
        let first = props
            .iter()
            .find(|prop| prop.max[1] - prop.min[1] > STEP_M)
            .expect("the kit has a prop taller than a step");
        let center = [
            (first.min[0] + first.max[0]) / 2.0,
            (first.min[1] + first.max[1]) / 2.0,
            (first.min[2] + first.max[2]) / 2.0,
        ];
        let inside = body(1, "idle", center, vec![[0.0, 1.0, 0.0]]);
        assert!(
            fighter_in_prop(&inside, first),
            "prop center sits inside its bounds"
        );
        let with_inside = brief_line_with_props(3, 9.0, &cam, &[inside], &props, &light);
        assert!(
            !with_inside.contains("\"inside_props\":0"),
            "fixture fighter inside a prop breaks the law: {with_inside}"
        );
        let bad = FrameLight {
            sun_elev: -0.1,
            ..light
        };
        let c = brief_line_with_props(3, 9.0, &cam, &bodies, &props, &bad);
        assert!(
            c.contains("\"light_ok\":false"),
            "sun below horizon fails: {c}"
        );
        assert_ne!(a, c, "bad light changes the brief");
        let far = [body(7, "idle", [500.0, 0.0, 500.0], vec![[0.0, 1.0, 0.0]])];
        let d = brief_line_with_props(3, 9.0, &cam, &far, &props, &light);
        assert!(
            d.contains("\"inside_props\":0"),
            "far fighter sits in no prop: {d}"
        );
    }

    #[test]
    fn brief_lists_both_ring_draws_once_each_and_repeats() {
        // SP64: the ring draws twice per frame (floor plus wall/tiers), so
        // the brief lists both rows once each, byte-equal x2.
        use crate::fight_ring::{ring_prop_row, ring_wall_row};
        let cam = camera();
        let floor = ring_prop_row();
        let wall = ring_wall_row();
        assert_ne!(floor.0, wall.0, "two draws, two kinds");
        let props = [
            PropTruth {
                kind: floor.0,
                min: floor.1[0],
                max: floor.1[1],
            },
            PropTruth {
                kind: wall.0,
                min: wall.1[0],
                max: wall.1[1],
            },
        ];
        let bodies = [body(0, "idle", [0.0, 0.0, 0.0], vec![[0.0, 1.0, 0.0]])];
        let light = FrameLight {
            sun_dir: [0.0, 1.0, 0.0],
            sun_elev: 0.5,
            exposure: 1.0,
            fog: 0.0,
            passes: 1,
        };
        let a = brief_line_with_props(3, 9.0, &cam, &bodies, &props, &light);
        assert_eq!(
            a,
            brief_line_with_props(3, 9.0, &cam, &bodies, &props, &light),
            "same inputs give same brief bytes"
        );
        for kind in [floor.0, wall.0] {
            let needle = format!("\"kind\":\"{kind}\"");
            assert_eq!(
                a.matches(&needle).count(),
                1,
                "ring draw {kind} lists once: {a}"
            );
        }
    }

    #[test]
    fn fighter_on_the_floor_or_atop_a_prop_is_on_it_not_in_it() {
        let floor = PropTruth {
            kind: "ring_floor",
            min: [-21.0, 0.0, -21.0],
            max: [21.0, 0.0, 21.0],
        };
        let standing = body(0, "idle", [0.0, 0.0, 0.0], vec![[0.0, 1.0, 0.0]]);
        assert!(
            !fighter_in_prop(&standing, &floor),
            "feet on the floor plane are on it, not in it"
        );
        let crate_prop = PropTruth {
            kind: "crate.gltf",
            min: [28.5, 0.0, -0.5],
            max: [29.5, 1.0, 0.5],
        };
        let atop = body(1, "idle", [29.0, 1.0, 0.0], vec![[0.0, 1.0, 0.0]]);
        assert!(
            !fighter_in_prop(&atop, &crate_prop),
            "feet on the top face are on the crate, not in it"
        );
        let at_base = body(2, "idle", [29.0, 0.0, 0.0], vec![[0.0, 1.0, 0.0]]);
        assert!(
            fighter_in_prop(&at_base, &crate_prop),
            "feet at the base footprint stay inside"
        );
        // The dune sand: a 20 cm slab under the lane is ground, not a box.
        let sand = PropTruth {
            kind: "sand_floor.gltf",
            min: [-17.0, 0.0, -15.0],
            max: [13.0, 0.2, 15.0],
        };
        let on_sand = body(3, "idle", [0.0, 0.0, 0.0], vec![[0.0, 1.0, 0.0]]);
        assert!(
            !fighter_in_prop(&on_sand, &sand),
            "feet in a slab lower than a step stand on it"
        );
    }

    #[test]
    fn brief_standing_fighter_on_real_kit_counts_no_inside() {
        use crate::arena_kit::kit_prop_rows;
        use crate::fight_ring::ring_prop_row;
        let cam = camera();
        let kit = kit_prop_rows();
        let ring = ring_prop_row();
        let mut props: Vec<PropTruth> = kit
            .iter()
            .map(|(kind, bounds)| PropTruth {
                kind,
                min: bounds[0],
                max: bounds[1],
            })
            .collect();
        props.push(PropTruth {
            kind: ring.0,
            min: ring.1[0],
            max: ring.1[1],
        });
        let light = FrameLight {
            sun_dir: [0.0, 1.0, 0.0],
            sun_elev: 0.5,
            exposure: 1.0,
            fog: 0.0,
            passes: 1,
        };
        let standing = [body(0, "idle", [0.0, 0.0, 0.0], vec![[0.0, 1.0, 0.0]])];
        let line = brief_line_with_props(3, 9.0, &cam, &standing, &props, &light);
        assert!(
            line.contains("\"inside_props\":0"),
            "a fighter standing on the floor is on it, not in a prop: {line}"
        );
    }

    #[test]
    fn brief_ten_frame_lists_every_kit_draw_once_per_frame_x2() {
        use crate::arena_kit::kit_prop_rows;
        use crate::fight_ring::{ring_prop_row, ring_wall_row};
        let cam = camera();
        let kit = kit_prop_rows();
        assert!(!kit.is_empty(), "placement stages kit draws");
        let floor = ring_prop_row();
        let wall = ring_wall_row();
        let mut props: Vec<PropTruth> = kit
            .iter()
            .map(|(kind, bounds)| PropTruth {
                kind,
                min: bounds[0],
                max: bounds[1],
            })
            .collect();
        props.push(PropTruth {
            kind: floor.0,
            min: floor.1[0],
            max: floor.1[1],
        });
        props.push(PropTruth {
            kind: wall.0,
            min: wall.1[0],
            max: wall.1[1],
        });
        let bodies = [body(0, "idle", [0.0, 0.0, 0.0], vec![[0.0, 1.0, 0.0]])];
        let light = FrameLight {
            sun_dir: [0.0, 1.0, 0.0],
            sun_elev: 0.5,
            exposure: 1.0,
            fog: 0.0,
            passes: 1,
        };
        for frame in 0..10u32 {
            let t = 9.0 + f64::from(frame);
            let a = brief_line_with_props(frame, t, &cam, &bodies, &props, &light);
            assert_eq!(
                a,
                brief_line_with_props(frame, t, &cam, &bodies, &props, &light)
            );
            for (kind, _) in &kit {
                let needle = format!("\"kind\":\"{kind}\"");
                let draws = kit.iter().filter(|(other, _)| other == kind).count();
                assert_eq!(a.matches(&needle).count(), draws);
            }
            for kind in [floor.0, wall.0] {
                assert_eq!(a.matches(&format!("\"kind\":\"{kind}\"")).count(), 1);
            }
            assert!(a.contains("\"light_ok\":true"));
        }
        let bad = FrameLight {
            sun_elev: -0.1,
            ..light
        };
        let good = brief_line_with_props(0, 9.0, &cam, &bodies, &props, &light);
        let bad_line = brief_line_with_props(0, 9.0, &cam, &bodies, &props, &bad);
        assert!(bad_line.contains("\"light_ok\":false"));
        assert_ne!(good, bad_line);
    }

    #[test]
    fn headless_view_run_is_public_and_needs_no_presenter() {
        // SP61: the headless bout runs with no `Presenter`: zero ticks fold
        // no frame, so no content loads and the line reads `""` twice.
        let first = crate::fighter_look::FighterLook::view_run(2040, 0, 8)
            .expect("zero ticks fold no frame");
        let second =
            crate::fighter_look::FighterLook::view_run(2040, 0, 8).expect("zero ticks repeat");
        assert_eq!(first, "", "zero ticks fold no frame");
        assert_eq!(second, "", "same seed repeats");
        assert_eq!(first, second, "same inputs repeat bit for bit");
    }

    #[test]
    fn the_headless_fold_names_its_worst_pop() {
        // AM24: the receipt says how many pops the fold had, but the fix
        // needs to know which joint moved, so the fold names the worst one
        // and the name agrees with the count.
        let (line, pop) = crate::fighter_look::FighterLook::view_run_detail(2040, 300, 8)
            .expect("the headless bout folds");
        let pops: u32 = line
            .split(" pops=")
            .nth(1)
            .and_then(|rest| rest.split(' ').next())
            .and_then(|num| num.parse().ok())
            .unwrap_or_else(|| panic!("the line counts the pops: {line}"));
        assert_eq!(
            pop.is_some(),
            pops > 0,
            "the name follows the count: {line}"
        );
        if let Some(pop) = pop {
            assert!(
                pop.miss_m > super::POP_M,
                "a named pop is over the law: {pop:?}"
            );
            println!("am24 pop: {pop:?}");
            println!(
                "am24 clip: act={} clip={} clip_s={} prev_clip={} (a different clip means a cut)",
                pop.act, pop.clip, pop.clip_s, pop.prev_clip
            );
        }
        println!("am24 line: {line}");
    }
}
