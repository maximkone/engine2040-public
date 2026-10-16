//! trails: weapon-trail rings plus the FPS viewmodel pose that feeds them.
//!
//! Consumer: the fighter look owns one [`TrailTable`] (a ring of the last
//! [`TRAIL_LEN`] blade base/tip pairs per attacking slot) and draws each
//! non-empty ring as a view-only triangle strip; the render forward pass
//! reads [`TrailVertex`] positions plus age alpha. Data owner:
//! `engine-anim`. Units: meters, seconds, unit vectors; presentation ticks
//! at 60 Hz. Determinism: presentation only, never in the tick hash; same
//! inputs twice give identical outputs, no wall clock, no trig (the camera
//! basis arrives as vectors, smoothed with lerp plus normalize). Errors:
//! [`TrailError`]; refusals change nothing. Budget: one linear pass over at
//! most 12 samples per slot per frame; steady-state frames allocate nothing.

use super::clip::clamp01;

/// Trail samples kept per slot: the last 12 presentation frames of blade
/// base plus tip.
pub const TRAIL_LEN: u8 = 12;
/// Trail slots: one ring per fighter the look can draw. Sized for the match
/// cap; the look maps its slot ids 1:1, extras refused with the culprit.
pub const TRAIL_SLOTS: u8 = 8;
/// Strip vertices per full ring: two (base, tip) per sample, oldest first.
pub const TRAIL_STRIP_VERTS: usize = 24;
/// Largest frame step the viewmodel trusts: hitches hold their rate instead
/// of jumping the weapon across the screen.
pub const DT_MAX_S: f32 = 0.25;
/// Squared length below which a direction reads as degenerate: the old dir
/// holds instead of dividing toward NaN.
pub const DIR_EPS2: f32 = 1e-12;

/// Ring capacity as the array length (mirrors [`TRAIL_LEN` by test]).
const RING_CAP: usize = 12;
/// Table width as the array length (mirrors [`TRAIL_SLOTS` by test]).
const SLOT_CAP: usize = 8;

/// What refused a trail call. Refusals change nothing.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum TrailError {
    /// Slot past [`TRAIL_SLOTS`]; culprit carried.
    BadSlot(u8),
    /// Non-finite float in the named lane.
    NonFinite(&'static str),
    /// Tuning field failed validation; field name carried.
    BadTuning(&'static str),
    /// Strip buffer too short; expected + got carried.
    ShortBuffer {
        /// Vertices the ring wants to write.
        expected: usize,
        /// Vertices the caller buffer holds.
        got: usize,
    },
}

impl core::fmt::Display for TrailError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadSlot(slot) => {
                write!(f, "trail: slot {slot} is past {}", TRAIL_SLOTS - 1)
            }
            Self::NonFinite(lane) => write!(f, "trail: {lane} is not finite"),
            Self::BadTuning(field) => write!(f, "trail: tuning {field} is not usable"),
            Self::ShortBuffer { expected, got } => {
                write!(f, "trail: strip needs {expected} vertices, got {got}")
            }
        }
    }
}

impl std::error::Error for TrailError {}

/// True when every lane is finite.
fn finite3(v: [f32; 3]) -> bool {
    v.iter().all(|c| c.is_finite())
}

/// Dot product over `+ - *` only, so the result replays bit-identically.
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Point plus scaled direction, one lane at a time.
fn add_scaled(p: [f32; 3], d: [f32; 3], s: f32) -> [f32; 3] {
    [p[0] + d[0] * s, p[1] + d[1] * s, p[2] + d[2] * s]
}

/// Camera-relative weapon anchor: eye plus right/up/forward offsets along
/// the normalized camera forward. One owner for the offset sum, so reset
/// and update can never drift apart.
fn anchor_target(
    eye: [f32; 3],
    fw: [f32; 3],
    right: [f32; 3],
    up: [f32; 3],
    tuning: &ViewmodelTuning,
) -> [f32; 3] {
    let (r, u, f) = (tuning.right_m(), tuning.up_m(), tuning.forward_m());
    [
        eye[0] + right[0] * r + up[0] * u + fw[0] * f,
        eye[1] + right[1] * r + up[1] * u + fw[1] * f,
        eye[2] + right[2] * r + up[2] * u + fw[2] * f,
    ]
}

/// One blade sample: base plus tip in meters, world space.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TrailSample {
    /// Blade base (guard) in meters.
    pub base: [f32; 3],
    /// Blade tip in meters.
    pub tip: [f32; 3],
}

impl TrailSample {
    /// Padding sample, never read past [`TrailRing::len`].
    const ZERO: Self = Self {
        base: [0.0; 3],
        tip: [0.0; 3],
    };
}

/// One ribbon vertex for the forward strip: position plus age alpha.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TrailVertex {
    /// World-space position in meters.
    pub pos: [f32; 3],
    /// Age alpha: oldest reads `1/len`, newest reads `1.0`, linear by age.
    pub alpha: f32,
}

/// Ring of the last [`TRAIL_LEN`] blade samples, oldest first. Fixed array,
/// no allocation; a 13th push evicts the oldest.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TrailRing {
    /// Samples in ring order; only the first [`TrailRing::len`] read live.
    samples: [TrailSample; RING_CAP],
    /// Live sample count, `0..=TRAIL_LEN`.
    len: u8,
    /// Index of the oldest live sample.
    head: u8,
}

impl TrailRing {
    /// Empty ring, const so the table builds without allocation.
    pub const fn new() -> Self {
        Self {
            samples: [TrailSample::ZERO; RING_CAP],
            len: 0,
            head: 0,
        }
    }
}

impl Default for TrailRing {
    fn default() -> Self {
        Self::new()
    }
}

impl TrailRing {
    /// Drop every sample; the slot draws nothing until it swings again.
    pub fn clear(&mut self) {
        self.len = 0;
        self.head = 0;
    }

    /// Live sample count.
    pub fn len(&self) -> usize {
        usize::from(self.len)
    }

    /// True when the slot draws nothing.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Record one frame: attacking slots push, idle slots clear so they draw
    /// none. Non-finite lanes are refused and change nothing.
    pub fn push(
        &mut self,
        base: [f32; 3],
        tip: [f32; 3],
        attacking: bool,
    ) -> Result<(), TrailError> {
        if !attacking {
            self.clear();
            return Ok(());
        }
        if !finite3(base) {
            return Err(TrailError::NonFinite("trail.base"));
        }
        if !finite3(tip) {
            return Err(TrailError::NonFinite("trail.tip"));
        }
        let sample = TrailSample { base, tip };
        if self.len < TRAIL_LEN {
            self.samples[usize::from((self.head + self.len) % TRAIL_LEN)] = sample;
            self.len += 1;
        } else {
            self.samples[usize::from(self.head)] = sample;
            self.head = (self.head + 1) % TRAIL_LEN;
        }
        Ok(())
    }

    /// Oldest-first sample (`0` reads oldest); `None` past the live count.
    pub fn sample(&self, index: u8) -> Option<&TrailSample> {
        if index >= self.len {
            return None;
        }
        Some(&self.samples[usize::from((self.head + index) % TRAIL_LEN)])
    }

    /// Age alpha for one live index: `(index + 1) / len`, oldest faintest.
    fn alpha_at(&self, index: u8) -> f32 {
        f32::from(index + 1) / f32::from(self.len)
    }

    /// Strip vertices this ring writes: two per live sample.
    pub fn strip_len(&self) -> usize {
        usize::from(self.len) * 2
    }

    /// Triangles the strip draws: two per segment, so an empty or single
    /// sample ring draws nothing.
    pub fn triangle_count(&self) -> usize {
        usize::from(self.len).saturating_sub(1) * 2
    }

    /// Write the ribbon into the caller buffer, oldest first, base then tip
    /// per sample sharing one age alpha. The buffer must hold at least
    /// [`TrailRing::strip_len`] vertices; a short buffer is refused and
    /// stays untouched. Returns vertices written.
    pub fn write_strip(&self, out: &mut [TrailVertex]) -> Result<usize, TrailError> {
        let need = self.strip_len();
        if out.len() < need {
            return Err(TrailError::ShortBuffer {
                expected: need,
                got: out.len(),
            });
        }
        let mut i: u8 = 0;
        while i < self.len {
            let sample = self.samples[usize::from((self.head + i) % TRAIL_LEN)];
            let alpha = self.alpha_at(i);
            let pair = usize::from(i) * 2;
            out[pair] = TrailVertex {
                pos: sample.base,
                alpha,
            };
            out[pair + 1] = TrailVertex {
                pos: sample.tip,
                alpha,
            };
            i += 1;
        }
        Ok(need)
    }
}

/// Per-slot trail rings, one per fighter the look can draw.
#[derive(Clone, PartialEq, Debug)]
pub struct TrailTable {
    /// Rings by slot id.
    rings: [TrailRing; SLOT_CAP],
}

impl TrailTable {
    /// Empty table, const so rigs build without allocation.
    pub const fn new() -> Self {
        Self {
            rings: [TrailRing::new(); SLOT_CAP],
        }
    }
}

impl Default for TrailTable {
    fn default() -> Self {
        Self::new()
    }
}

impl TrailTable {
    /// Slot id to ring index, refusing extras with the culprit.
    fn check_slot(slot: u8) -> Result<usize, TrailError> {
        if slot >= TRAIL_SLOTS {
            return Err(TrailError::BadSlot(slot));
        }
        Ok(usize::from(slot))
    }

    /// Record one frame for one slot: attacking pushes, idle clears.
    /// A bad slot is refused before anything is written.
    pub fn update(
        &mut self,
        slot: u8,
        base: [f32; 3],
        tip: [f32; 3],
        attacking: bool,
    ) -> Result<(), TrailError> {
        let index = Self::check_slot(slot)?;
        self.rings[index].push(base, tip, attacking)
    }

    /// Record one frame from the viewmodel pose: the production path that
    /// wires [`ViewmodelState::blade_endpoints`] into the rings, so the
    /// endpoints never cross crates unwitnessed.
    pub fn update_viewmodel(
        &mut self,
        slot: u8,
        vm: &ViewmodelState,
        tuning: &ViewmodelTuning,
        attacking: bool,
    ) -> Result<(), TrailError> {
        let (base, tip) = vm.blade_endpoints(tuning);
        self.update(slot, base, tip, attacking)
    }

    /// Ring for one slot; `None` past the match cap.
    pub fn ring(&self, slot: u8) -> Option<&TrailRing> {
        if slot >= TRAIL_SLOTS {
            return None;
        }
        Some(&self.rings[usize::from(slot)])
    }

    /// Write one slot's ribbon into the caller buffer (see
    /// [`TrailRing::write_strip`]); a bad slot is refused first.
    pub fn strip_into(&self, slot: u8, out: &mut [TrailVertex]) -> Result<usize, TrailError> {
        let index = Self::check_slot(slot)?;
        self.rings[index].write_strip(out)
    }

    /// Triangles one slot draws; `None` past the match cap.
    pub fn triangles(&self, slot: u8) -> Option<usize> {
        self.ring(slot).map(TrailRing::triangle_count)
    }

    /// Slots holding at least one sample.
    pub fn active_slots(&self) -> u8 {
        let mut count: u8 = 0;
        for ring in self.rings.iter() {
            if !ring.is_empty() {
                count += 1;
            }
        }
        count
    }
}

/// FPS viewmodel tuning: camera-relative offsets plus lag, validated once at
/// construction and immutable after, so per-frame updates never re-check.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ViewmodelTuning {
    /// Follow lag in seconds; larger trails further behind the camera.
    lag_s: f32,
    /// Weapon anchor right of the eye in meters (negative reads left).
    right_m: f32,
    /// Weapon anchor above the eye in meters (negative reads below).
    up_m: f32,
    /// Weapon anchor ahead of the eye in meters.
    forward_m: f32,
    /// Blade base ahead of the anchor along the smoothed forward, meters.
    blade_base_m: f32,
    /// Blade length past the base in meters.
    blade_len_m: f32,
}

impl ViewmodelTuning {
    /// Build validated tuning; a bad field is refused with its name.
    pub fn new(
        lag_s: f32,
        right_m: f32,
        up_m: f32,
        forward_m: f32,
        blade_base_m: f32,
        blade_len_m: f32,
    ) -> Result<Self, TrailError> {
        let tuning = Self {
            lag_s,
            right_m,
            up_m,
            forward_m,
            blade_base_m,
            blade_len_m,
        };
        tuning.validate()?;
        Ok(tuning)
    }

    /// Refuse a non-positive or non-finite lag, a non-finite offset, or a
    /// negative blade length; the caller keeps its previous tuning then.
    pub fn validate(&self) -> Result<(), TrailError> {
        if !self.lag_s.is_finite() || self.lag_s <= 0.0 {
            return Err(TrailError::BadTuning("lag_s"));
        }
        if !self.right_m.is_finite() {
            return Err(TrailError::BadTuning("right_m"));
        }
        if !self.up_m.is_finite() {
            return Err(TrailError::BadTuning("up_m"));
        }
        if !self.forward_m.is_finite() {
            return Err(TrailError::BadTuning("forward_m"));
        }
        if !self.blade_base_m.is_finite() || self.blade_base_m < 0.0 {
            return Err(TrailError::BadTuning("blade_base_m"));
        }
        if !self.blade_len_m.is_finite() || self.blade_len_m < 0.0 {
            return Err(TrailError::BadTuning("blade_len_m"));
        }
        Ok(())
    }

    /// Follow lag in seconds.
    pub fn lag_s(&self) -> f32 {
        self.lag_s
    }

    /// Anchor right offset in meters.
    pub fn right_m(&self) -> f32 {
        self.right_m
    }

    /// Anchor up offset in meters.
    pub fn up_m(&self) -> f32 {
        self.up_m
    }

    /// Anchor forward offset in meters.
    pub fn forward_m(&self) -> f32 {
        self.forward_m
    }

    /// Blade base reach past the anchor in meters.
    pub fn blade_base_m(&self) -> f32 {
        self.blade_base_m
    }

    /// Blade length past the base in meters.
    pub fn blade_len_m(&self) -> f32 {
        self.blade_len_m
    }
}

/// One camera frame as viewmodel input: eye plus basis plus step. Plain
/// input bundle (mirrors the game `FighterView` shape as data, never as a
/// dependency edge): the look fills it from its fight camera each frame.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CameraFrame {
    /// Camera eye in meters, world space.
    pub eye: [f32; 3],
    /// Camera forward (need not be unit; normalized on use).
    pub fwd: [f32; 3],
    /// Camera right in meters per meter.
    pub right: [f32; 3],
    /// Camera up in meters per meter.
    pub up: [f32; 3],
    /// Frame step in seconds.
    pub dt_s: f32,
}

/// FPS viewmodel pose: a smoothed weapon anchor plus a unit forward. The
/// weapon follows the camera with lag instead of riding it rigidly, so fast
/// mouse motion leaves the blade visibly behind for a frame or two.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ViewmodelState {
    /// Smoothed weapon anchor in meters, world space.
    anchor: [f32; 3],
    /// Smoothed unit forward the blade points along.
    dir: [f32; 3],
}

impl ViewmodelState {
    /// Bind-pose viewmodel at the origin facing +Z; the rig snaps it to the
    /// camera on its first live frame.
    pub const fn new() -> Self {
        Self {
            anchor: [0.0; 3],
            dir: [0.0, 0.0, 1.0],
        }
    }
}

impl Default for ViewmodelState {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewmodelState {
    /// Smoothed weapon anchor in meters.
    pub fn anchor(&self) -> [f32; 3] {
        self.anchor
    }

    /// Smoothed unit forward.
    pub fn dir(&self) -> [f32; 3] {
        self.dir
    }

    /// Snap to the camera now (spawn or teleport): the anchor takes the full
    /// offset target and the dir takes the normalized forward. Bad input
    /// holds the pose and reports false, so the caller knows the snap is
    /// still owed.
    pub fn reset(&mut self, frame: CameraFrame, tuning: &ViewmodelTuning) -> bool {
        let CameraFrame {
            eye,
            fwd,
            right,
            up,
            ..
        } = frame;
        if !finite3(eye) || !finite3(fwd) || !finite3(right) || !finite3(up) {
            return false;
        }
        let len2 = dot(fwd, fwd);
        if !len2.is_finite() || len2 <= DIR_EPS2 {
            return false;
        }
        let inv = 1.0 / len2.sqrt();
        let fw = [fwd[0] * inv, fwd[1] * inv, fwd[2] * inv];
        let target = anchor_target(eye, fw, right, up, tuning);
        if !finite3(target) {
            return false;
        }
        self.anchor = target;
        self.dir = fw;
        true
    }

    /// Follow one frame toward the camera anchor: position lerps by
    /// `clamp01(dt / lag)`, direction takes the same normalized lerp, so an
    /// exact-opposite flick holds the old dir instead of dividing to NaN.
    /// Non-finite inputs hold the pose; a negative step reads as zero and a
    /// hitch clamps to [`DT_MAX_S`], so the weapon never jumps or poisons.
    pub fn update(&mut self, frame: CameraFrame, tuning: &ViewmodelTuning) {
        let CameraFrame {
            eye,
            fwd,
            right,
            up,
            dt_s,
        } = frame;
        if !finite3(eye) || !finite3(fwd) || !finite3(right) || !finite3(up) {
            return;
        }
        let len2 = dot(fwd, fwd);
        if !len2.is_finite() || len2 <= DIR_EPS2 {
            return;
        }
        let inv = 1.0 / len2.sqrt();
        let fw = [fwd[0] * inv, fwd[1] * inv, fwd[2] * inv];
        let dt = if dt_s.is_finite() {
            dt_s.clamp(0.0, DT_MAX_S)
        } else {
            0.0
        };
        let k = clamp01(dt / tuning.lag_s());
        let target = anchor_target(eye, fw, right, up, tuning);
        if !finite3(target) {
            return;
        }
        self.anchor = [
            self.anchor[0] + (target[0] - self.anchor[0]) * k,
            self.anchor[1] + (target[1] - self.anchor[1]) * k,
            self.anchor[2] + (target[2] - self.anchor[2]) * k,
        ];
        let blended = [
            self.dir[0] + (fw[0] - self.dir[0]) * k,
            self.dir[1] + (fw[1] - self.dir[1]) * k,
            self.dir[2] + (fw[2] - self.dir[2]) * k,
        ];
        let blended2 = dot(blended, blended);
        if blended2.is_finite() && blended2 > DIR_EPS2 {
            let di = 1.0 / blended2.sqrt();
            self.dir = [blended[0] * di, blended[1] * di, blended[2] * di];
        }
    }

    /// Blade endpoints off the smoothed pose: base rides `blade_base_m`
    /// ahead of the anchor, tip rides the full reach. Infallible: the state
    /// stays finite by construction and the tuning is validated at build.
    pub fn blade_endpoints(&self, tuning: &ViewmodelTuning) -> ([f32; 3], [f32; 3]) {
        (
            add_scaled(self.anchor, self.dir, tuning.blade_base_m()),
            add_scaled(
                self.anchor,
                self.dir,
                tuning.blade_base_m() + tuning.blade_len_m(),
            ),
        )
    }
}

/// One FPS rig: the viewmodel pose plus the trail table it feeds. The look
/// owns one per local view (single-screen play holds one); the table keeps
/// a ring per fighter slot while the weapon itself is the camera's.
#[derive(Clone, PartialEq, Debug)]
pub struct FpsRig {
    /// Validated tuning shared by the pose and the blade reach.
    tuning: ViewmodelTuning,
    /// Smoothed weapon pose following the camera.
    vm: ViewmodelState,
    /// Per-slot blade trail rings.
    table: TrailTable,
    /// False until the first live camera frame snaps the pose.
    snapped: bool,
}

impl FpsRig {
    /// Build a rig from raw tuning values; a bad field is refused with its
    /// name and builds nothing.
    pub fn new(
        right_m: f32,
        up_m: f32,
        forward_m: f32,
        blade_base_m: f32,
        blade_len_m: f32,
        lag_s: f32,
    ) -> Result<Self, TrailError> {
        Ok(Self {
            tuning: ViewmodelTuning::new(
                lag_s,
                right_m,
                up_m,
                forward_m,
                blade_base_m,
                blade_len_m,
            )?,
            vm: ViewmodelState::new(),
            table: TrailTable::new(),
            snapped: false,
        })
    }

    /// Drive one frame from the camera: snap the pose on the first live
    /// frame, then follow it and record the blade into `slot`'s ring
    /// (attacking pushes, idle clears). A bad slot is refused before
    /// anything moves; bad camera input holds the pose and leaves the snap
    /// owed.
    pub fn update(
        &mut self,
        slot: u8,
        frame: CameraFrame,
        attacking: bool,
    ) -> Result<(), TrailError> {
        if slot >= TRAIL_SLOTS {
            return Err(TrailError::BadSlot(slot));
        }
        if !self.snapped {
            self.snapped = self.vm.reset(frame, &self.tuning);
            if !self.snapped {
                return Ok(());
            }
        } else {
            self.vm.update(frame, &self.tuning);
        }
        self.table
            .update_viewmodel(slot, &self.vm, &self.tuning, attacking)
    }

    /// Write one slot's ribbon into the caller buffer (see
    /// [`TrailRing::write_strip`]).
    pub fn strip_into(&self, slot: u8, out: &mut [TrailVertex]) -> Result<usize, TrailError> {
        self.table.strip_into(slot, out)
    }

    /// Triangles one slot draws; `None` past the match cap.
    pub fn triangles(&self, slot: u8) -> Option<usize> {
        self.table.triangles(slot)
    }

    /// Slots holding at least one sample.
    pub fn active_slots(&self) -> u8 {
        self.table.active_slots()
    }

    /// Ring for one slot; `None` past the match cap.
    pub fn ring(&self, slot: u8) -> Option<&TrailRing> {
        self.table.ring(slot)
    }

    /// Smoothed weapon pose: the look reads the anchor plus dir to place
    /// the first-person weapon mesh the trail is sampled from.
    pub fn viewmodel(&self) -> &ViewmodelState {
        &self.vm
    }

    /// Whole trail table, for receipts and replay proofs.
    pub fn table(&self) -> &TrailTable {
        &self.table
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::alloc_probe::measure;

    fn eye() -> [f32; 3] {
        [0.0, 1.6, 5.0]
    }

    fn fwd() -> [f32; 3] {
        [0.0, 0.0, 1.0]
    }

    fn right() -> [f32; 3] {
        [1.0, 0.0, 0.0]
    }

    fn up() -> [f32; 3] {
        [0.0, 1.0, 0.0]
    }

    fn tuning() -> ViewmodelTuning {
        ViewmodelTuning::new(0.12, 0.25, -0.2, 0.5, 0.1, 0.9).expect("valid tuning")
    }

    fn frame(eye: [f32; 3], fwd: [f32; 3], dt: f32) -> CameraFrame {
        CameraFrame {
            eye,
            fwd,
            right: right(),
            up: up(),
            dt_s: dt,
        }
    }

    #[test]
    fn public_shape_matches_storage() {
        assert_eq!(RING_CAP, usize::from(TRAIL_LEN), "ring stores TRAIL_LEN");
        assert_eq!(
            SLOT_CAP,
            usize::from(TRAIL_SLOTS),
            "table stores TRAIL_SLOTS"
        );
        assert_eq!(
            TRAIL_STRIP_VERTS,
            usize::from(TRAIL_LEN) * 2,
            "strip holds two verts per sample"
        );
        assert!(TrailTable::new().ring(7).is_some(), "slot 7 reads live");
        assert!(TrailTable::new().ring(8).is_none(), "slot 8 reads past");
    }

    #[test]
    fn trail_ring_holds_the_last_12_frames_oldest_first() {
        let mut ring = TrailRing::new();
        for i in 0u8..13u8 {
            let f = f32::from(i);
            ring.push([f, 0.0, 0.0], [f, 0.0, 1.0], true)
                .expect("finite push");
        }
        assert_eq!(ring.len(), 12, "ring keeps the last 12, got {}", ring.len());
        assert_eq!(
            ring.sample(0).expect("oldest").base,
            [1.0, 0.0, 0.0],
            "the 13th push evicts the oldest"
        );
        assert_eq!(
            ring.sample(11).expect("newest").base,
            [12.0, 0.0, 0.0],
            "newest reads last"
        );
        assert_eq!(ring.sample(12), None, "past the live count reads none");
        assert_eq!(ring.triangle_count(), 22, "11 quads read 22 triangles");
    }

    #[test]
    fn idle_slots_clear_and_draw_nothing() {
        let mut ring = TrailRing::new();
        for i in 0u8..3u8 {
            let f = f32::from(i);
            ring.push([f, 0.0, 0.0], [f, 0.0, 1.0], true)
                .expect("finite push");
        }
        ring.push([9.0, 9.0, 9.0], [9.0, 9.0, 9.0], false)
            .expect("idle clears");
        assert!(ring.is_empty(), "idle slots draw none");
        assert_eq!(ring.strip_len(), 0);
        assert_eq!(ring.triangle_count(), 0);
        let mut out = [TrailVertex {
            pos: [0.0; 3],
            alpha: 0.0,
        }; TRAIL_STRIP_VERTS];
        assert_eq!(ring.write_strip(&mut out).expect("empty strip"), 0);
    }

    #[test]
    fn strip_alpha_fades_by_age_mid_swing() {
        let mut ring = TrailRing::new();
        for i in 0u8..12u8 {
            let f = f32::from(i);
            ring.push([f, 1.0, 0.0], [f, 2.0, 0.0], true)
                .expect("finite push");
        }
        let mut out = [TrailVertex {
            pos: [0.0; 3],
            alpha: 0.0,
        }; TRAIL_STRIP_VERTS];
        assert_eq!(ring.write_strip(&mut out).expect("full strip"), 24);
        let mut prev = 0.0f32;
        for i in 0u8..12u8 {
            let pair = usize::from(i) * 2;
            let f = f32::from(i);
            assert_eq!(out[pair].pos, [f, 1.0, 0.0], "base vertex {i}");
            assert_eq!(out[pair + 1].pos, [f, 2.0, 0.0], "tip vertex {i}");
            assert_eq!(out[pair].alpha, out[pair + 1].alpha, "pair shares age {i}");
            assert!(
                out[pair].alpha > prev,
                "alpha grows toward newest, pair {i}"
            );
            prev = out[pair].alpha;
        }
        assert!(
            (out[0].alpha - 1.0 / 12.0).abs() < 1e-6,
            "oldest reads 1/12, got {}",
            out[0].alpha
        );
        assert_eq!(out[23].alpha, 1.0, "newest reads full alpha");
    }

    #[test]
    fn refusals_change_nothing_and_name_the_cause() {
        let mut ring = TrailRing::new();
        ring.push([1.0, 0.0, 0.0], [1.0, 0.0, 1.0], true)
            .expect("seed push");
        assert_eq!(
            ring.push([f32::NAN, 0.0, 0.0], [0.0; 3], true).unwrap_err(),
            TrailError::NonFinite("trail.base")
        );
        assert_eq!(
            ring.push([0.0; 3], [0.0, f32::INFINITY, 0.0], true)
                .unwrap_err(),
            TrailError::NonFinite("trail.tip")
        );
        assert_eq!(ring.len(), 1, "refused pushes write nothing");
        let mut table = TrailTable::new();
        assert_eq!(
            table
                .update(TRAIL_SLOTS, [0.0; 3], [0.0; 3], true)
                .unwrap_err(),
            TrailError::BadSlot(TRAIL_SLOTS)
        );
        assert_eq!(
            table.strip_into(9u8, &mut []).unwrap_err(),
            TrailError::BadSlot(9)
        );
        table
            .update(0, [0.0; 3], [1.0; 3], true)
            .expect("seed slot");
        table
            .update(0, [2.0; 3], [3.0; 3], true)
            .expect("second sample");
        let mut short = [TrailVertex {
            pos: [0.0; 3],
            alpha: -1.0,
        }; 4];
        assert_eq!(
            table.strip_into(0, &mut short[..3]).unwrap_err(),
            TrailError::ShortBuffer {
                expected: 4,
                got: 3
            },
            "short buffer names expected vs got"
        );
        assert!(
            short.iter().all(|v| v.alpha == -1.0),
            "a refused strip writes nothing"
        );
        assert!(TrailError::BadSlot(7).to_string().contains('7'));
        assert!(TrailError::NonFinite("trail.base")
            .to_string()
            .contains("trail.base"));
        assert!(TrailError::ShortBuffer {
            expected: 4,
            got: 3
        }
        .to_string()
        .contains('4'));
        assert_eq!(
            ViewmodelTuning::new(0.0, 0.0, 0.0, 0.0, 0.0, 1.0).unwrap_err(),
            TrailError::BadTuning("lag_s")
        );
        assert_eq!(
            ViewmodelTuning::new(f32::NAN, 0.0, 0.0, 0.0, 0.0, 1.0).unwrap_err(),
            TrailError::BadTuning("lag_s")
        );
        assert_eq!(
            ViewmodelTuning::new(0.1, 0.0, 0.0, 0.0, 0.0, -1.0).unwrap_err(),
            TrailError::BadTuning("blade_len_m")
        );
    }

    #[test]
    fn viewmodel_lag_follows_the_camera_without_teleporting() {
        let t = tuning();
        let mut vm = ViewmodelState::new();
        let dt = 1.0f32 / 60.0;
        vm.update(frame(eye(), fwd(), dt), &t);
        // Hand-rolled target: eye + right*0.25 + up*(-0.2) + fwd*0.5.
        let target = [0.25f32, 1.4, 5.5];
        let k = dt / t.lag_s();
        for lane in 0..3 {
            assert!(
                (vm.anchor()[lane] - target[lane] * k).abs() < 1e-6,
                "lane {lane}: {:?} vs {target:?} * {k}",
                vm.anchor()
            );
            assert!(
                vm.anchor()[lane] > 0.0 && vm.anchor()[lane] < target[lane],
                "first frame moves partway, lane {lane}: {:?}",
                vm.anchor()
            );
        }
        for _ in 0..600u32 {
            vm.update(frame(eye(), fwd(), dt), &t);
        }
        for lane in 0..3 {
            assert!(
                (vm.anchor()[lane] - target[lane]).abs() < 1e-4,
                "lag converges, lane {lane}: {:?} vs {target:?}",
                vm.anchor()
            );
        }
        let dir = vm.dir();
        let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
        assert!((len - 1.0).abs() < 1e-5, "smoothed dir stays unit: {dir:?}");
        assert!(
            dir[2] > 0.999,
            "dir converges on the camera forward: {dir:?}"
        );
    }

    #[test]
    fn viewmodel_holds_pose_on_bad_input() {
        let t = tuning();
        let mut vm = ViewmodelState::new();
        vm.update(frame(eye(), fwd(), 1.0 / 60.0), &t);
        let held = vm;
        vm.update(frame([f32::NAN, 0.0, 0.0], fwd(), 1.0 / 60.0), &t);
        assert_eq!(vm, held, "NaN eye holds");
        vm.update(frame(eye(), [0.0; 3], 1.0 / 60.0), &t);
        assert_eq!(vm, held, "degenerate forward holds");
        vm.update(frame(eye(), fwd(), f32::NAN), &t);
        assert_eq!(vm, held, "NaN dt holds");
        vm.update(frame(eye(), fwd(), -1.0), &t);
        assert_eq!(vm, held, "negative dt holds");
        vm.update(frame(eye(), fwd(), 10.0), &t);
        assert_ne!(
            vm, held,
            "a long hitch still follows (clamped), never jumps past"
        );
        assert!(vm.anchor().iter().all(|v| v.is_finite()));
        assert!(vm.dir().iter().all(|v| v.is_finite()));
    }

    #[test]
    fn viewmodel_dir_turns_the_short_way() {
        let t = tuning();
        let mut vm = ViewmodelState::new();
        // Exact-opposite snap at half weight degenerates: the old dir holds
        // instead of NaN, so a 180-degree flick never blacks the weapon out.
        vm.update(frame(eye(), [0.0, 0.0, -1.0], t.lag_s() * 0.5), &t);
        assert_eq!(
            vm.dir(),
            [0.0, 0.0, 1.0],
            "antipodal lerp holds: {:?}",
            vm.dir()
        );
        // A half turn toward +X leans right, never left, and stays unit.
        vm.update(frame(eye(), [1.0, 0.0, 0.0], t.lag_s() * 0.5), &t);
        let dir = vm.dir();
        assert!(dir[0] > 0.7 && dir[2] > 0.0, "turns right: {dir:?}");
        let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
        assert!((len - 1.0).abs() < 1e-5, "turn stays unit: {dir:?}");
    }

    #[test]
    fn fps_rig_snaps_then_drives_viewmodel_into_trail() {
        let mut first = FpsRig::new(0.25, -0.2, 0.5, 0.1, 0.9, 0.12).expect("valid rig");
        let mut second = FpsRig::new(0.25, -0.2, 0.5, 0.1, 0.9, 0.12).expect("valid rig");
        for rig in [&mut first, &mut second] {
            // First live frame snaps exactly: no streak in from the origin.
            rig.update(3, frame(eye(), fwd(), 1.0 / 60.0), true)
                .expect("snap frame");
            let anchor = rig.viewmodel().anchor();
            assert!(
                (anchor[0] - 0.25).abs() < 1e-6
                    && (anchor[1] - 1.4).abs() < 1e-6
                    && (anchor[2] - 5.5).abs() < 1e-6,
                "snap lands on the camera anchor: {anchor:?}"
            );
            for step in 1u8..20u8 {
                let at = [f32::from(step) * 0.05, 1.6, 5.0];
                rig.update(3, frame(at, fwd(), 1.0 / 60.0), true)
                    .expect("attack frame");
            }
        }
        assert_eq!(first.active_slots(), 1, "only the swinging slot trails");
        assert_eq!(first.triangles(3), Some(22), "full ring reads 22 triangles");
        assert_eq!(first.triangles(2), Some(0), "idle slots draw none");
        let (base, tip) = first.viewmodel().blade_endpoints(&tuning());
        let dx = tip[0] - base[0];
        let dy = tip[1] - base[1];
        let dz = tip[2] - base[2];
        let len = (dx * dx + dy * dy + dz * dz).sqrt();
        assert!(
            (len - 0.9).abs() < 1e-5,
            "camera motion never stretches the blade: {len}"
        );
        let ring = first.ring(3).expect("swinging slot");
        let newest = ring.sample(11).expect("newest");
        assert_eq!(newest.base, base, "trail newest matches the viewmodel base");
        assert_eq!(newest.tip, tip, "trail newest matches the viewmodel tip");
        assert_eq!(
            first.table(),
            second.table(),
            "same frames replay bit-identical"
        );
        first
            .update(3, frame(eye(), fwd(), 1.0 / 60.0), false)
            .expect("idle frame");
        assert_eq!(
            first.triangles(3),
            Some(0),
            "one idle frame clears the strip"
        );
        assert_eq!(first.active_slots(), 0);
        assert_eq!(
            first
                .update(9, frame(eye(), fwd(), 1.0 / 60.0), true)
                .unwrap_err(),
            TrailError::BadSlot(9)
        );
    }

    #[test]
    fn steady_state_frames_allocate_nothing() {
        let mut rig = FpsRig::new(0.25, -0.2, 0.5, 0.1, 0.9, 0.12).expect("valid rig");
        let mut out = [TrailVertex {
            pos: [0.0; 3],
            alpha: 0.0,
        }; TRAIL_STRIP_VERTS];
        let ((), heap) = measure(|| {
            for step in 0u8..120u8 {
                let at = [f32::from(step) * 0.01, 1.6, 5.0];
                let attacking = step % 30u8 < 20u8;
                rig.update(1, frame(at, fwd(), 1.0 / 60.0), attacking)
                    .expect("march stays valid");
                let _ = rig.strip_into(1, &mut out).expect("strip fits");
                std::hint::black_box(&out);
            }
        });
        assert_eq!(
            heap.calls, 0,
            "trail frames must not allocate, got {heap:?}"
        );
    }
}
