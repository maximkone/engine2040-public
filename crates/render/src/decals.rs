//! decals: deferred blood decals with depth reject + roughness override.
//!
//! Owner: render lane; consumer WIN1 arena match on CB2 gore events plus
//! ED1 viewport and Q1 captures. Units: meters for world depth and radii,
//! pixels for frame sizes and decal footprints, ticks at 60 Hz for fades,
//! ms in receipts only. Determinism: integer pixel orders, stable slot
//! order, same inputs give identical bytes; wall clock feeds receipts only.
//! Errors are typed [`DecalError`]; the headless twin names the missing
//! window device while the window build binds the pool for real with 0
//! validation errors.

use std::fmt;
use std::time::Instant;

// ---------------------------------------------------------------------------
// Bounds: every external size is validated before allocation.
// ---------------------------------------------------------------------------

/// Largest accepted frame dimension in pixels (mirrors the RA1 frame bound).
pub const MAX_DECAL_DIM_PX: u32 = 2048;
/// Default decal slots; one atlas tile per decal, `128 * 64 * 64 * 4 B`.
pub const DEFAULT_DECALS: usize = 128;
/// Hard bound for custom pools and atlases.
pub const MAX_DECALS: usize = 256;
/// Bytes per atlas tile (RGBA8).
pub const ATLAS_TILE_BYTES: usize = 64 * 64 * 4;
/// Largest accepted splat radius in meters (arena-scale guard).
pub const MAX_DECAL_RADIUS_M: f32 = 2.0;
/// Largest accepted screen footprint radius in pixels.
pub const MAX_DECAL_RADIUS_PX: f32 = 256.0;
/// Depth mismatch that rejects a decal in meters. A decal floating this far
/// in front of the scene is air, not sand; this far behind is buried.
pub const DEPTH_REJECT_M: f32 = 0.15;
/// Largest accepted view depth in meters (far-plane guard for fixtures).
pub const MAX_VIEW_DEPTH_M: f32 = 10_000.0;
/// Blood fade time in ticks (600 = 10 s at 60 Hz), then the slot is culled.
pub const BLOOD_FADE_TICKS: u64 = 600;
/// Fresh blood roughness (wet, glossy).
pub const BLOOD_ROUGH_WET: f32 = 0.15;
/// Dried blood roughness (matte).
pub const BLOOD_ROUGH_DRY: f32 = 0.65;
/// Display color of blood on sand (dark red, linear).
pub const BLOOD_COLOR: [f32; 3] = [0.45, 0.03, 0.03];
/// Posted GPU budget at 1080p on the reference GPU (PF2 enforces).
pub const DECAL_BUDGET_MS: f64 = 0.5;

// ---------------------------------------------------------------------------
// Errors: typed; bad input fails loudly, the headless twin names the
// window device bind it cannot run without a device.
// ---------------------------------------------------------------------------

/// Decal pass failures. `BudgetExceeded` is a degrade signal for atlas
/// accounting; `NoDevice` covers headless-twin calls with no wgpu device in
/// hand; `Device`/`Validation` cover device-side failures on the window
/// build, which never takes the twin path.
#[derive(Clone, PartialEq, Debug)]
pub enum DecalError {
    /// Caller input is unusable; reason carried.
    Invalid(String),
    /// The request exceeded a bound; the caller clamps to `allowed`.
    BudgetExceeded {
        /// Requested amount.
        requested: u32,
        /// Amount the pass actually uses.
        allowed: u32,
    },
    /// No device behind this call; what to build first, carried.
    NoDevice(String),
    /// A device, queue or map call failed; message carried.
    Device(String),
    /// A wgpu validation error fired inside the submit scope; kept loud.
    Validation(String),
}

impl fmt::Display for DecalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(e) => write!(f, "decal input: {e}"),
            Self::BudgetExceeded { requested, allowed } => {
                write!(
                    f,
                    "decal budget exceeded: {requested} requested, {allowed} used"
                )
            }
            Self::NoDevice(e) => write!(f, "decal bind needs a device: {e}"),
            Self::Device(e) => write!(f, "decal device: {e}"),
            Self::Validation(e) => write!(f, "decal validation: {e}"),
        }
    }
}

impl std::error::Error for DecalError {}

// ---------------------------------------------------------------------------
// Gore events: the CB2 presentation input. WIN1 maps fight gore into these;
// the pool below owns projection, depth reject and fading.
// ---------------------------------------------------------------------------

/// Which gore visual a decal carries. Only blood ships here; new variants
/// extend this enum without changing the depth/fade core.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DecalKind {
    /// Blood on sand.
    Blood,
}

/// Presentation-only gore hit from the fight layer: view-space hit point in
/// meters (camera at the origin looking along `-Z`, so front has `z < 0`),
/// surface normal, splat radius and hit severity.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GoreEvent {
    /// Hit point in view space, meters; `z` must be negative (in front).
    pub pos_view_m: [f32; 3],
    /// Surface normal at the hit (unit; sand is `+Y`).
    pub normal: [f32; 3],
    /// Splat radius in meters, `(0, MAX_DECAL_RADIUS_M]`.
    pub radius_m: f32,
    /// Hit severity, `1..=3` (light, solid, heavy).
    pub severity: u8,
    /// Gore variant.
    pub kind: DecalKind,
}

impl GoreEvent {
    /// Validate finiteness, normal length, radius and severity before use.
    pub fn validate(&self) -> Result<(), DecalError> {
        if !self.pos_view_m.iter().all(|c| c.is_finite()) {
            return Err(DecalError::Invalid("gore position is not finite".into()));
        }
        if !self.normal.iter().all(|c| c.is_finite()) {
            return Err(DecalError::Invalid("gore normal is not finite".into()));
        }
        let l2 = self.normal[0] * self.normal[0]
            + self.normal[1] * self.normal[1]
            + self.normal[2] * self.normal[2];
        if !l2.is_finite() || l2 < 1e-12 {
            return Err(DecalError::Invalid("gore normal has ~zero length".into()));
        }
        if !self.radius_m.is_finite() || self.radius_m <= 0.0 || self.radius_m > MAX_DECAL_RADIUS_M
        {
            return Err(DecalError::Invalid(format!(
                "gore radius {} outside (0, {MAX_DECAL_RADIUS_M}]",
                self.radius_m
            )));
        }
        if self.severity == 0 || self.severity > 3 {
            return Err(DecalError::Invalid(format!(
                "gore severity {} outside 1..=3",
                self.severity
            )));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Camera and depth: pinhole projection plus the scene depth frame.
// The RA1 presenter owns the view matrix; this module projects points that
// are already in view space, so the core stays dependency-free.
// ---------------------------------------------------------------------------

/// Pinhole camera for decal projection: frame size in pixels, focal length
/// in pixels (`height / (2 tan(fov/2))`) and the view range in meters.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Pinhole {
    /// Frame width in pixels, `1..=MAX_DECAL_DIM_PX`.
    pub width_px: u32,
    /// Frame height in pixels, `1..=MAX_DECAL_DIM_PX`.
    pub height_px: u32,
    /// Focal length in pixels, finite and positive.
    pub focal_px: f32,
    /// View near in meters, `> 0`.
    pub near_m: f32,
    /// View far in meters, `> near_m`.
    pub far_m: f32,
}

impl Pinhole {
    /// Validate dims, focal length and range before any projection.
    pub fn validate(&self) -> Result<(), DecalError> {
        if self.width_px == 0 || self.height_px == 0 {
            return Err(DecalError::Invalid("decal frame size is zero".into()));
        }
        if self.width_px > MAX_DECAL_DIM_PX || self.height_px > MAX_DECAL_DIM_PX {
            return Err(DecalError::BudgetExceeded {
                requested: self.width_px.max(self.height_px),
                allowed: MAX_DECAL_DIM_PX,
            });
        }
        if !self.focal_px.is_finite() || self.focal_px <= 0.0 {
            return Err(DecalError::Invalid(format!(
                "decal focal {} is not positive finite",
                self.focal_px
            )));
        }
        if !self.near_m.is_finite() || !self.far_m.is_finite() {
            return Err(DecalError::Invalid("decal view range is not finite".into()));
        }
        if self.near_m <= 0.0 || self.far_m <= self.near_m {
            return Err(DecalError::Invalid(format!(
                "decal range {}..{} is not 0 < near < far",
                self.near_m, self.far_m
            )));
        }
        Ok(())
    }

    /// Frame pixel count.
    pub fn pixels(&self) -> usize {
        self.width_px as usize * self.height_px as usize
    }
}

/// Screen-space decal footprint: pixel center, view depth in meters and
/// influence radius in pixels.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ScreenDecal {
    /// Horizontal pixel center.
    pub x_px: f32,
    /// Vertical pixel center.
    pub y_px: f32,
    /// View depth in meters (`-z` of the view point).
    pub depth_m: f32,
    /// Influence radius in pixels, `> 0`.
    pub radius_px: f32,
}

impl ScreenDecal {
    /// Validate finiteness and ranges before the depth test.
    pub fn validate(&self) -> Result<(), DecalError> {
        if !self.x_px.is_finite() || !self.y_px.is_finite() || !self.depth_m.is_finite() {
            return Err(DecalError::Invalid(
                "screen decal center is not finite".into(),
            ));
        }
        if !self.radius_px.is_finite()
            || self.radius_px <= 0.0
            || self.radius_px > MAX_DECAL_RADIUS_PX
        {
            return Err(DecalError::Invalid(format!(
                "screen decal radius {} outside (0, {MAX_DECAL_RADIUS_PX}]",
                self.radius_px
            )));
        }
        if self.depth_m <= 0.0 || !self.depth_m.is_finite() {
            return Err(DecalError::Invalid(format!(
                "screen decal depth {} is not positive finite",
                self.depth_m
            )));
        }
        Ok(())
    }
}

/// Project a view-space point to the screen: depth is `-z`, pixel center is
/// the pinhole image of `(x, y)` and the footprint radius scales by
/// `focal / depth`. Returns `Invalid` behind the camera, outside the range
/// or off the frame — never a NaN footprint.
pub fn project_view(
    cam: &Pinhole,
    point_view_m: [f32; 3],
    radius_m: f32,
) -> Result<ScreenDecal, DecalError> {
    cam.validate()?;
    if !point_view_m.iter().all(|c| c.is_finite()) {
        return Err(DecalError::Invalid("decal view point is not finite".into()));
    }
    if !radius_m.is_finite() || radius_m <= 0.0 || radius_m > MAX_DECAL_RADIUS_M {
        return Err(DecalError::Invalid(format!(
            "decal radius {radius_m} outside (0, {MAX_DECAL_RADIUS_M}]"
        )));
    }
    let depth = -point_view_m[2];
    if !depth.is_finite() || depth <= 0.0 {
        return Err(DecalError::Invalid(
            "decal view point is behind the camera".into(),
        ));
    }
    if depth < cam.near_m || depth > cam.far_m {
        return Err(DecalError::Invalid(format!(
            "decal depth {depth} outside view range {}..{}",
            cam.near_m, cam.far_m
        )));
    }
    let cx = cam.width_px as f32 * 0.5;
    let cy = cam.height_px as f32 * 0.5;
    let x_px = cx + point_view_m[0] * cam.focal_px / depth;
    let y_px = cy - point_view_m[1] * cam.focal_px / depth;
    let radius_px = radius_m * cam.focal_px / depth;
    if !x_px.is_finite() || !y_px.is_finite() || !radius_px.is_finite() {
        return Err(DecalError::Invalid("decal projection is not finite".into()));
    }
    if x_px < 0.0 || y_px < 0.0 || x_px > cam.width_px as f32 || y_px > cam.height_px as f32 {
        return Err(DecalError::Invalid(format!(
            "decal center ({x_px:.1}, {y_px:.1}) is off the {}x{} frame",
            cam.width_px, cam.height_px
        )));
    }
    let screen = ScreenDecal {
        x_px,
        y_px,
        depth_m: depth,
        radius_px,
    };
    screen.validate()?;
    Ok(screen)
}

/// Scene depth frame: view distance per pixel in meters, row-major
/// `width * height`. The presenter renders it; the decal test reads it.
#[derive(Clone, PartialEq, Debug)]
pub struct DepthFrame {
    /// Frame width in pixels.
    pub width_px: u32,
    /// Frame height in pixels.
    pub height_px: u32,
    /// View distance per pixel in meters, row-major, all positive finite.
    pub depth_m: Vec<f32>,
}

impl DepthFrame {
    /// Build from explicit depths: validates dims, length and every sample
    /// before the decal test can read it.
    pub fn build(width_px: u32, height_px: u32, depth_m: Vec<f32>) -> Result<Self, DecalError> {
        if width_px == 0 || height_px == 0 {
            return Err(DecalError::Invalid("depth frame size is zero".into()));
        }
        if width_px > MAX_DECAL_DIM_PX || height_px > MAX_DECAL_DIM_PX {
            return Err(DecalError::BudgetExceeded {
                requested: width_px.max(height_px),
                allowed: MAX_DECAL_DIM_PX,
            });
        }
        let want = width_px as usize * height_px as usize;
        if depth_m.len() != want {
            return Err(DecalError::Invalid(format!(
                "depth samples {} differ from {width_px}x{height_px} ({want})",
                depth_m.len()
            )));
        }
        for (i, d) in depth_m.iter().enumerate() {
            if !d.is_finite() || *d <= 0.0 || *d > MAX_VIEW_DEPTH_M {
                return Err(DecalError::Invalid(format!(
                    "depth sample {i} value {d} is not positive finite depth"
                )));
            }
        }
        Ok(Self {
            width_px,
            height_px,
            depth_m,
        })
    }

    /// Flat field at one depth (sand-plane fixture helper).
    pub fn flat(width_px: u32, height_px: u32, depth: f32) -> Result<Self, DecalError> {
        if !depth.is_finite() || depth <= 0.0 || depth > MAX_VIEW_DEPTH_M {
            return Err(DecalError::Invalid(format!(
                "flat depth {depth} is not positive finite depth"
            )));
        }
        if width_px == 0 || height_px == 0 {
            return Err(DecalError::Invalid("depth frame size is zero".into()));
        }
        if width_px > MAX_DECAL_DIM_PX || height_px > MAX_DECAL_DIM_PX {
            return Err(DecalError::BudgetExceeded {
                requested: width_px.max(height_px),
                allowed: MAX_DECAL_DIM_PX,
            });
        }
        Ok(Self {
            width_px,
            height_px,
            depth_m: vec![depth; width_px as usize * height_px as usize],
        })
    }

    /// Read one texel; bounds-checked for callers outside the test.
    pub fn sample(&self, x: u32, y: u32) -> Result<f32, DecalError> {
        if x >= self.width_px || y >= self.height_px {
            return Err(DecalError::Invalid(format!(
                "depth texel ({x},{y}) outside {}x{}",
                self.width_px, self.height_px
            )));
        }
        Ok(self.depth_m[y as usize * self.width_px as usize + x as usize])
    }

    /// Deterministic little-endian bytes of the depth samples.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.depth_m.len() * 4);
        for d in &self.depth_m {
            out.extend_from_slice(&d.to_le_bytes());
        }
        out
    }
}

/// Accepted footprint: covered pixel count plus the worst lifts seen, for
/// receipts and tests. All covered pixels sit within `DEPTH_REJECT_M`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Footprint {
    /// Covered pixels (centers inside the radius).
    pub coverage_px: u32,
    /// Largest `scene - decal` gap over the footprint, meters (`>= 0`).
    pub max_lift_m: f32,
    /// Largest `decal - scene` burial over the footprint, meters (`>= 0`).
    pub max_bury_m: f32,
}

/// Depth reject: scan the screen footprint over the depth frame. Pixels whose
/// centers fall inside the radius must agree with the decal depth within
/// `DEPTH_REJECT_M`: floating decals (air) and buried decals (under the
/// sand) fail loudly with the gap carried. Integer pixel order, no float
/// keys; same inputs give the same verdict.
pub fn depth_reject(screen: &ScreenDecal, depth: &DepthFrame) -> Result<Footprint, DecalError> {
    screen.validate()?;
    if depth.width_px == 0 || depth.height_px == 0 {
        return Err(DecalError::Invalid("depth frame size is zero".into()));
    }
    let want = depth.width_px as usize * depth.height_px as usize;
    if depth.depth_m.len() != want {
        return Err(DecalError::Invalid(format!(
            "depth samples {} differ from {}x{} ({want})",
            depth.depth_m.len(),
            depth.width_px,
            depth.height_px
        )));
    }
    let w = depth.width_px as i32;
    let h = depth.height_px as i32;
    let r = screen.radius_px;
    let x0 = (screen.x_px - r).floor() as i32;
    let x1 = (screen.x_px + r).floor() as i32;
    let y0 = (screen.y_px - r).floor() as i32;
    let y1 = (screen.y_px + r).floor() as i32;
    let mut coverage = 0u32;
    let mut max_lift = 0.0f32;
    let mut max_bury = 0.0f32;
    for iy in y0..=y1 {
        for ix in x0..=x1 {
            if ix < 0 || iy < 0 || ix >= w || iy >= h {
                continue;
            }
            let px = ix as f32 + 0.5;
            let py = iy as f32 + 0.5;
            let dx = px - screen.x_px;
            let dy = py - screen.y_px;
            if dx * dx + dy * dy > r * r {
                continue;
            }
            let scene = depth.depth_m[iy as usize * w as usize + ix as usize];
            if !scene.is_finite() || scene <= 0.0 {
                return Err(DecalError::Invalid(format!(
                    "depth texel ({ix},{iy}) value {scene} is not positive finite"
                )));
            }
            let lift = scene - screen.depth_m;
            if lift > DEPTH_REJECT_M {
                return Err(DecalError::Invalid(format!(
                    "decal floats in air: lift {lift:.3} m over {DEPTH_REJECT_M} m at ({ix},{iy})"
                )));
            }
            let bury = screen.depth_m - scene;
            if bury > DEPTH_REJECT_M {
                return Err(DecalError::Invalid(format!(
                    "decal buried under the surface: bury {bury:.3} m over {DEPTH_REJECT_M} m at ({ix},{iy})"
                )));
            }
            if lift > max_lift {
                max_lift = lift;
            }
            if bury > max_bury {
                max_bury = bury;
            }
            coverage += 1;
        }
    }
    if coverage == 0 {
        return Err(DecalError::Invalid(
            "decal footprint covers no pixels".into(),
        ));
    }
    Ok(Footprint {
        coverage_px: coverage,
        max_lift_m: max_lift,
        max_bury_m: max_bury,
    })
}

// ---------------------------------------------------------------------------
// Blood finish: roughness override plus tick fade. Fresh blood is wet and
// glossy; it dries matte over BLOOD_FADE_TICKS while alpha fades to zero.
// ---------------------------------------------------------------------------

/// Blood roughness at `age_ticks`: linear `WET -> DRY` over the fade, aged
/// past the fade clamps at dry. Pure function of age for exact tests.
pub fn blood_roughness(age_ticks: u64) -> f32 {
    let t = (age_ticks.min(BLOOD_FADE_TICKS) as f32) / (BLOOD_FADE_TICKS as f32);
    BLOOD_ROUGH_WET + (BLOOD_ROUGH_DRY - BLOOD_ROUGH_WET) * t
}

/// Blood alpha at `age_ticks`: linear `1 -> 0` over the fade, terminal zero.
/// Pure function of age for exact tick-script tests.
pub fn blood_alpha(age_ticks: u64) -> f32 {
    if age_ticks >= BLOOD_FADE_TICKS {
        return 0.0;
    }
    (BLOOD_FADE_TICKS - age_ticks) as f32 / BLOOD_FADE_TICKS as f32
}

// ---------------------------------------------------------------------------
// Atlas: bounded tile accounting. One tile per decal; alloc hands the lowest
// free index (deterministic), full atlases report BudgetExceeded, bad frees
// fail loudly. Bytes resident are tiles times ATLAS_TILE_BYTES.
// ---------------------------------------------------------------------------

/// Bounded decal atlas: tile occupancy plus used/free counts for receipts.
#[derive(Clone, PartialEq, Debug)]
pub struct DecalAtlas {
    /// Tile count (capacity).
    capacity: usize,
    /// Occupancy per tile; `true` is resident.
    used_tiles: Vec<bool>,
    /// Resident tile count.
    used: usize,
}

impl DecalAtlas {
    /// Empty atlas with the default [`DEFAULT_DECALS`] tiles.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_DECALS)
    }

    /// Empty atlas with `cap` tiles, clamped to `1..=MAX_DECALS` so tests
    /// can prove exact full accounting on a tiny atlas.
    pub fn with_capacity(cap: usize) -> Self {
        let capacity = cap.clamp(1, MAX_DECALS);
        Self {
            capacity,
            used_tiles: vec![false; capacity],
            used: 0,
        }
    }

    /// Tile capacity.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Resident tiles.
    pub fn used(&self) -> usize {
        self.used
    }

    /// Free tiles.
    pub fn free_count(&self) -> usize {
        self.capacity - self.used
    }

    /// Resident bytes (`used * ATLAS_TILE_BYTES`).
    pub fn used_bytes(&self) -> usize {
        self.used * ATLAS_TILE_BYTES
    }

    /// Capacity bytes (`capacity * ATLAS_TILE_BYTES`).
    pub fn capacity_bytes(&self) -> usize {
        self.capacity * ATLAS_TILE_BYTES
    }

    /// Allocate the lowest free tile. Full atlases report `BudgetExceeded`
    /// instead of failing silently.
    pub fn alloc(&mut self) -> Result<u32, DecalError> {
        for (i, used) in self.used_tiles.iter_mut().enumerate() {
            if !*used {
                *used = true;
                self.used += 1;
                return Ok(i as u32);
            }
        }
        Err(DecalError::BudgetExceeded {
            requested: self.capacity as u32 + 1,
            allowed: self.capacity as u32,
        })
    }

    /// Release `slot`. Freeing a free or out-of-range slot fails loudly.
    pub fn free(&mut self, slot: u32) -> Result<(), DecalError> {
        let i = slot as usize;
        if i >= self.capacity {
            return Err(DecalError::Invalid(format!(
                "atlas slot {slot} outside 0..{}",
                self.capacity
            )));
        }
        if !self.used_tiles[i] {
            return Err(DecalError::Invalid(format!(
                "atlas slot {slot} is already free"
            )));
        }
        self.used_tiles[i] = false;
        self.used -= 1;
        Ok(())
    }

    /// True while `slot` is resident.
    pub fn is_used(&self, slot: u32) -> bool {
        (slot as usize) < self.capacity && self.used_tiles[slot as usize]
    }
}

impl Default for DecalAtlas {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Pool: bounded deferred decals fed by gore events. Full pools evict the
// oldest decal (smallest sequence) and recycle its tile, so CB2 bursts
// degrade to "oldest blood fades first". Ticks age and cull at the fade.
// ---------------------------------------------------------------------------

/// One resident decal: view-space center, normalized normal, radii, screen
/// footprint, atlas tile, age and insertion sequence for oldest-eviction.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PlacedDecal {
    /// Center in view space, meters.
    pub center_view_m: [f32; 3],
    /// Normalized surface normal.
    pub normal: [f32; 3],
    /// Splat radius in meters.
    pub radius_m: f32,
    /// Screen footprint at placement.
    pub screen: ScreenDecal,
    /// Atlas tile.
    pub atlas_slot: u32,
    /// Ticks since placement; culled at [`BLOOD_FADE_TICKS`].
    pub age_ticks: u64,
    /// Covered pixels at placement.
    pub coverage_px: u32,
    /// Insertion sequence (oldest-eviction order).
    pub seq: u64,
    /// Gore variant.
    pub kind: DecalKind,
}

impl PlacedDecal {
    /// Current roughness override for this decal.
    pub fn roughness(&self) -> f32 {
        blood_roughness(self.age_ticks)
    }

    /// Current fade alpha for this decal.
    pub fn alpha(&self) -> f32 {
        blood_alpha(self.age_ticks)
    }
}

/// One decal frame receipt: slots, tiles, coverage and wall-clock ms for
/// the frame report (ms is evidence only, never hashed).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct DecalReceipt {
    /// Resident decals.
    pub decals: u64,
    /// Resident atlas tiles.
    pub tiles_used: u64,
    /// Covered pixels summed over resident decals at placement.
    pub coverage_px: u64,
    /// Wall-clock place time in ms.
    pub ms: f64,
}

impl DecalReceipt {
    /// Posted GPU budget this receipt is charged against.
    pub fn budget_ms(&self) -> f64 {
        DECAL_BUDGET_MS
    }

    /// True while the pass stayed inside the posted budget.
    pub fn within_budget(&self) -> bool {
        self.ms <= DECAL_BUDGET_MS
    }
}

/// Bounded deferred-decal pool with its atlas: placement validates, projects
/// and depth-rejects each gore event, then binds one tile per decal.
#[derive(Clone, PartialEq, Debug)]
pub struct DecalPool {
    /// Resident decals in placement order.
    decals: Vec<PlacedDecal>,
    /// Tile accounting; one tile per resident decal.
    atlas: DecalAtlas,
    /// Pool cap (slots, not bytes).
    cap: usize,
    /// Next insertion sequence.
    next_seq: u64,
    /// Decals evicted as oldest because the pool or atlas was full.
    evicted: u64,
    /// Ticks advanced so far.
    tick: u64,
    /// Last placement wall-clock ms for the receipt.
    last_ms: f64,
}

impl DecalPool {
    /// Empty pool with [`DEFAULT_DECALS`] slots and tiles.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_DECALS)
    }

    /// Empty pool with `cap` slots and tiles, clamped to `1..=MAX_DECALS`
    /// so tests prove exact eviction on a tiny pool.
    pub fn with_capacity(cap: usize) -> Self {
        let clamped = cap.clamp(1, MAX_DECALS);
        Self {
            decals: Vec::new(),
            atlas: DecalAtlas::with_capacity(clamped),
            cap: clamped,
            next_seq: 0,
            evicted: 0,
            tick: 0,
            last_ms: 0.0,
        }
    }

    /// Slot cap.
    pub fn capacity(&self) -> usize {
        self.cap
    }

    /// Resident decal count.
    pub fn len(&self) -> usize {
        self.decals.len()
    }

    /// Whether no decals are resident.
    pub fn is_empty(&self) -> bool {
        self.decals.is_empty()
    }

    /// Decals evicted as oldest because the pool was full.
    pub fn evicted_count(&self) -> u64 {
        self.evicted
    }

    /// Ticks advanced so far.
    pub fn tick_count(&self) -> u64 {
        self.tick
    }

    /// Resident atlas tiles.
    pub fn tiles_used(&self) -> usize {
        self.atlas.used()
    }

    /// All resident decals in placement order.
    pub fn decals(&self) -> &[PlacedDecal] {
        &self.decals
    }

    /// Place one gore event: validate, project, depth-reject against `depth`,
    /// bind an atlas tile (evicting the oldest decal when full) and record
    /// the footprint. Returns the bound tile. Floating or buried decals leave
    /// the pool untouched and report `Invalid`.
    pub fn place(
        &mut self,
        cam: &Pinhole,
        depth: &DepthFrame,
        event: &GoreEvent,
    ) -> Result<u32, DecalError> {
        event.validate()?;
        cam.validate()?;
        if depth.width_px != cam.width_px || depth.height_px != cam.height_px {
            return Err(DecalError::Invalid(format!(
                "depth {}x{} differs from camera {}x{}",
                depth.width_px, depth.height_px, cam.width_px, cam.height_px
            )));
        }
        let started = Instant::now();
        let screen = project_view(cam, event.pos_view_m, event.radius_m)?;
        let footprint = depth_reject(&screen, depth)?;
        let l2 = event.normal[0] * event.normal[0]
            + event.normal[1] * event.normal[1]
            + event.normal[2] * event.normal[2];
        let l = l2.sqrt();
        let normal = [
            event.normal[0] / l,
            event.normal[1] / l,
            event.normal[2] / l,
        ];
        let seq = self.next_seq;
        self.next_seq += 1;
        let slot = match self.atlas.alloc() {
            Ok(slot) => slot,
            Err(_) => {
                self.evict_oldest()?;
                self.atlas.alloc()?
            }
        };
        let placed = PlacedDecal {
            center_view_m: event.pos_view_m,
            normal,
            radius_m: event.radius_m,
            screen,
            atlas_slot: slot,
            age_ticks: 0,
            coverage_px: footprint.coverage_px,
            seq,
            kind: event.kind,
        };
        if self.decals.len() < self.cap {
            self.decals.push(placed);
        } else {
            let mut oldest = 0;
            for i in 1..self.decals.len() {
                if self.decals[i].seq < self.decals[oldest].seq {
                    oldest = i;
                }
            }
            self.decals[oldest] = placed;
            self.evicted += 1;
        }
        self.last_ms = started.elapsed().as_secs_f64() * 1000.0;
        Ok(slot)
    }

    /// Free the oldest resident decal and recycle its tile. Only called when
    /// the atlas is full, so an empty pool here is a logic error, never
    /// silent.
    fn evict_oldest(&mut self) -> Result<(), DecalError> {
        if self.decals.is_empty() {
            return Err(DecalError::Invalid(
                "decal pool is empty during eviction".into(),
            ));
        }
        let mut oldest = 0;
        for i in 1..self.decals.len() {
            if self.decals[i].seq < self.decals[oldest].seq {
                oldest = i;
            }
        }
        let slot = self.decals[oldest].atlas_slot;
        self.atlas.free(slot)?;
        self.decals.swap_remove(oldest);
        self.evicted += 1;
        Ok(())
    }

    /// Advance one tick: every decal ages by one, decals reaching
    /// [`BLOOD_FADE_TICKS`] are culled in stable order and their tiles freed.
    pub fn tick(&mut self) {
        for d in &mut self.decals {
            d.age_ticks += 1;
        }
        let mut kept: Vec<PlacedDecal> = Vec::with_capacity(self.decals.len());
        for d in &self.decals {
            if d.age_ticks < BLOOD_FADE_TICKS {
                kept.push(*d);
            } else if self.atlas.is_used(d.atlas_slot) {
                let _ = self.atlas.free(d.atlas_slot);
            }
        }
        self.decals = kept;
        self.tick += 1;
    }

    /// Receipt for the last placement.
    pub fn receipt(&self) -> DecalReceipt {
        DecalReceipt {
            decals: self.decals.len() as u64,
            tiles_used: self.atlas.used() as u64,
            coverage_px: self.decals.iter().map(|d| d.coverage_px as u64).sum(),
            ms: self.last_ms,
        }
    }

    /// Deterministic little-endian bytes over resident decals in order:
    /// center, normal, radius, roughness, alpha, slot, age, coverage, kind.
    /// Same complete scripts give bit-identical bytes.
    pub fn state_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.decals.len() * 48);
        for d in &self.decals {
            for c in d.center_view_m {
                out.extend_from_slice(&c.to_le_bytes());
            }
            for c in d.normal {
                out.extend_from_slice(&c.to_le_bytes());
            }
            out.extend_from_slice(&d.radius_m.to_le_bytes());
            out.extend_from_slice(&d.screen.x_px.to_le_bytes());
            out.extend_from_slice(&d.screen.depth_m.to_le_bytes());
            out.extend_from_slice(&d.screen.radius_px.to_le_bytes());
            out.extend_from_slice(&d.roughness().to_le_bytes());
            out.extend_from_slice(&d.alpha().to_le_bytes());
            out.extend_from_slice(&d.atlas_slot.to_le_bytes());
            out.extend_from_slice(&d.age_ticks.to_le_bytes());
            out.extend_from_slice(&d.coverage_px.to_le_bytes());
            out.push(match d.kind {
                DecalKind::Blood => 0,
            });
        }
        out
    }

    /// Per-instance bytes for the deferred decal pass: one 32-byte record
    /// per resident decal in order — center view pos (3) + radius (1) +
    /// rgba (4), f32 little-endian. Alpha carries the tick fade; the
    /// roughness override rides a second upload the GPU pass reads.
    pub fn upload_instances(&self) -> Vec<u8> {
        const STRIDE: usize = 8 * 4;
        let mut out = Vec::with_capacity(self.decals.len() * STRIDE);
        for d in &self.decals {
            for c in d.center_view_m {
                out.extend_from_slice(&c.to_le_bytes());
            }
            out.extend_from_slice(&d.radius_m.to_le_bytes());
            for c in BLOOD_COLOR {
                out.extend_from_slice(&c.to_le_bytes());
            }
            out.extend_from_slice(&d.alpha().to_le_bytes());
        }
        out
    }

    /// Per-decal roughness overrides in slot order: one f32 per resident
    /// decal, matching [`Self::upload_instances`] row for row.
    pub fn upload_roughness(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.decals.len() * 4);
        for d in &self.decals {
            out.extend_from_slice(&d.roughness().to_le_bytes());
        }
        out
    }

    /// GPU submit seam for the RA2 deferred decal pass. Validates the camera
    /// first, then names the missing window device bind on the headless twin
    /// (never silent success). The CPU placement above is the reference here;
    /// the `window` build below records the live pass instead.
    #[cfg(not(feature = "window"))]
    pub fn submit_gpu(&self, cam: &Pinhole) -> Result<DecalReceipt, DecalError> {
        cam.validate()?;
        // FC3 skip switch: idle frames carry no decals, so the RP3 pass skips
        // the bind entirely (0 decals, 0 tiles, 0 ms, 0 errors) instead of
        // requiring a device. Skipped frames stay inside `DECAL_BUDGET_MS`.
        if self.decals.is_empty() {
            return Ok(DecalReceipt {
                decals: 0,
                tiles_used: 0,
                coverage_px: 0,
                ms: 0.0,
            });
        }
        if self.decals.len() != self.atlas.used() {
            return Err(DecalError::Invalid(format!(
                "decal pool {} differs from atlas {}",
                self.decals.len(),
                self.atlas.used()
            )));
        }
        Err(DecalError::NoDevice(
            "decal GPU bind needs the window build with a live device; CPU placement is the reference".into(),
        ))
    }
}

impl Default for DecalPool {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Window device submit (behind `window`): the resident instance plus
// roughness uploads bound under the shared LookFrame bind shape.
// Engine-owned WGSL, no donor code.
// ---------------------------------------------------------------------------

/// Bind probe over the decal uploads: copies the bound words to prove the
/// deferred bind-group path validates. Look uniform bound and live.
#[cfg(feature = "window")]
const DECAL_BIND_WGSL: &str = r#"
struct LookU {
    frame_idx: u32,
    width_px: u32,
    height_px: u32,
    exposure: f32,
    sun_dir: vec3<f32>,
    _pad: f32,
};
@group(0) @binding(0) var<uniform> look: LookU;
@group(0) @binding(1) var<storage, read> src: array<u32>;
@group(0) @binding(2) var<storage, read_write> dst: array<u32>;
@compute @workgroup_size(64)
fn bind_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= arrayLength(&src)) { return; }
    dst[i] = src[i] + u32(look.exposure * 0.0);
}
"#;

/// One window submit over the resident decals: slots, coverage, the
/// validation outcome and the wall-clock encode + submit + readback ms.
#[cfg(feature = "window")]
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct DecalGpuReceipt {
    /// Resident decals bound.
    pub decals: u64,
    /// Resident atlas tiles bound.
    pub tiles_used: u64,
    /// Covered pixels summed over resident decals at placement.
    pub coverage_px: u64,
    /// wgpu validation errors observed; green means 0.
    pub validation_errors: u32,
    /// Wall time for encode + submit + readback in ms (evidence only).
    pub submit_ms: f64,
}

#[cfg(feature = "window")]
impl DecalGpuReceipt {
    /// Posted GPU budget this submit is charged against.
    pub fn budget_ms(&self) -> f64 {
        DECAL_BUDGET_MS
    }

    /// True while the submit stayed inside the posted budget.
    pub fn within_budget(&self) -> bool {
        self.submit_ms <= DECAL_BUDGET_MS
    }

    /// True when the submit observed zero validation errors.
    pub fn is_clean(&self) -> bool {
        self.validation_errors == 0
    }
}

/// Reusable window submit state: the bind-probe pipeline plus the look bind
/// layout over one uniform buffer, with shape-keyed word buffers and one
/// bind group. Built once per frame loop (WIN1 and ED1 each own one);
/// steady-state submits on a matching shape create no buffers or bind
/// groups (the reuse test proves the count stays flat).
#[cfg(feature = "window")]
pub struct DecalSubmitter {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    look_buf: wgpu::Buffer,
    cached_words: Option<u64>,
    src_buf: Option<wgpu::Buffer>,
    dst_buf: Option<wgpu::Buffer>,
    readback: Option<wgpu::Buffer>,
    bind: Option<wgpu::BindGroup>,
    created_buffers: u64,
}

#[cfg(feature = "window")]
impl DecalSubmitter {
    /// Compile the bind-probe pipeline and the look bind layout; allocate
    /// the 32-byte look uniform buffer. No pool-size state yet: word
    /// buffers size on the first submit and resize only on shape change.
    pub fn new(device: &wgpu::Device) -> Self {
        use crate::pipeline_warmup::CountedPipelines as _;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rp3-decal-bind"),
            source: wgpu::ShaderSource::Wgsl(DECAL_BIND_WGSL.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rp3-decal-bind-layout"),
            entries: &[
                compute_uniform_entry(0),
                compute_storage_entry(1, true),
                compute_storage_entry(2, false),
            ],
        });
        let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rp3-decal-pipe-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline_counted(&wgpu::ComputePipelineDescriptor {
            label: Some("rp3-decal-pipeline"),
            layout: Some(&pipe_layout),
            module: &shader,
            entry_point: Some("bind_main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let look_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rp3-decal-look-uniform"),
            size: crate::graph::LOOK_UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            layout,
            look_buf,
            cached_words: None,
            src_buf: None,
            dst_buf: None,
            readback: None,
            bind: None,
            created_buffers: 0,
        }
    }

    /// Sized word plus readback buffers created so far (probe for the
    /// reuse test: flat across matching-shape submits).
    pub fn created_buffers(&self) -> u64 {
        self.created_buffers
    }
}

/// Uniform bind-group entry over the compute stage.
#[cfg(feature = "window")]
pub(crate) fn compute_uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// Storage bind-group entry over the compute stage.
#[cfg(feature = "window")]
pub(crate) fn compute_storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// Map a readback buffer and wait for it. One poll covers every pending map.
/// Shared [`crate::accel`] core with the `decal` error prefix kept.
#[cfg(feature = "window")]
fn decal_map_and_wait(
    device: &wgpu::Device,
    buffer: &wgpu::Buffer,
) -> Result<(), crate::accel::ReadbackError> {
    crate::accel::map_readback(device, buffer, "decal")
}

#[cfg(feature = "window")]
impl From<crate::accel::ReadbackError> for DecalError {
    fn from(error: crate::accel::ReadbackError) -> Self {
        Self::Device(error.to_string())
    }
}

/// Read `count` little-endian u32 words from a mapped readback buffer.
/// Shared [`crate::accel`] core with the `decal` text kept.
#[cfg(feature = "window")]
fn decal_readback_u32(
    buffer: &wgpu::Buffer,
    count: usize,
) -> Result<Vec<u32>, crate::accel::ReadbackError> {
    crate::accel::readback_u32(buffer, count, "decal", "words")
}

#[cfg(feature = "window")]
impl DecalPool {
    /// Window submit over the resident decals (RP3 live): bind the
    /// per-instance plus roughness uploads (zero-padded to whole words;
    /// one zero word while the pool is empty so idle frames still record
    /// the pass) under the shared [`crate::gtao::LookFrame`] bind, run one
    /// bind-group probe pass over them and read them back exactly, proving
    /// the deferred bind validates with 0 errors. Fails loudly (never
    /// silent, never the twin refusal) on bad cameras or any device fault.
    pub fn submit_gpu(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &crate::gtao::LookFrame,
        cam: &Pinhole,
        submitter: &mut DecalSubmitter,
    ) -> Result<DecalGpuReceipt, DecalError> {
        use std::time::Instant;
        frame
            .validate()
            .map_err(|e| DecalError::Invalid(format!("decal look frame: {e}")))?;
        cam.validate()?;
        if self.decals.len() != self.atlas.used() {
            return Err(DecalError::Invalid(format!(
                "decal pool {} differs from atlas {}",
                self.decals.len(),
                self.atlas.used()
            )));
        }
        let mut bytes = self.upload_instances();
        bytes.extend_from_slice(&self.upload_roughness());
        if bytes.is_empty() {
            bytes.extend_from_slice(&0u32.to_le_bytes());
        }
        let words = bytes.len().div_ceil(4) as u64;
        let count = u32::try_from(words)
            .map_err(|_| DecalError::Invalid(format!("decal words {words} exceed u32")))?;
        let byte_len = words * 4;
        let started = Instant::now();

        if submitter.cached_words != Some(words) {
            let src_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rp3-decal-src"),
                size: byte_len,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let dst_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rp3-decal-dst"),
                size: byte_len,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rp3-decal-readback"),
                size: byte_len,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("rp3-decal-bind"),
                layout: &submitter.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: submitter.look_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: src_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: dst_buf.as_entire_binding(),
                    },
                ],
            });
            submitter.src_buf = Some(src_buf);
            submitter.dst_buf = Some(dst_buf);
            submitter.readback = Some(readback);
            submitter.bind = Some(bind);
            submitter.cached_words = Some(words);
            submitter.created_buffers = submitter.created_buffers.saturating_add(3);
        }
        let (src_buf, dst_buf, readback, bind) = match (
            &submitter.src_buf,
            &submitter.dst_buf,
            &submitter.readback,
            &submitter.bind,
        ) {
            (Some(a), Some(b), Some(c), Some(d)) => (a, b, c, d),
            _ => {
                return Err(DecalError::Device(
                    "decal submitter lost its sized buffers".into(),
                ));
            }
        };

        queue.write_buffer(
            &submitter.look_buf,
            0,
            &crate::graph::look_frame_uniform_bytes(frame),
        );
        let mut padded = vec![0u8; byte_len as usize];
        padded[..bytes.len()].copy_from_slice(&bytes);
        queue.write_buffer(src_buf, 0, &padded);

        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("rp3-decal-frame"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rp3-decal-bind-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&submitter.pipeline);
            pass.set_bind_group(0, bind, &[]);
            pass.dispatch_workgroups(count.div_ceil(64), 1, 1);
        }
        encoder.copy_buffer_to_buffer(dst_buf, 0, readback, 0, byte_len);
        queue.submit(Some(encoder.finish()));
        decal_map_and_wait(device, readback)?;
        let back = decal_readback_u32(readback, words as usize)?;
        readback.unmap();
        if let Some(err) = pollster::block_on(scope.pop()) {
            return Err(DecalError::Validation(format!(
                "decal bind validation: {err:?}"
            )));
        }
        for (i, chunk) in padded.chunks_exact(4).enumerate() {
            let want = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            if back[i] != want {
                return Err(DecalError::Device(format!(
                    "decal bind roundtrip word {i} differs: {} vs {want}",
                    back[i]
                )));
            }
        }
        Ok(DecalGpuReceipt {
            decals: self.decals.len() as u64,
            tiles_used: self.atlas.used() as u64,
            coverage_px: self.decals.iter().map(|d| d.coverage_px as u64).sum(),
            validation_errors: 0,
            submit_ms: started.elapsed().as_secs_f64() * 1000.0,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests: projection, depth fixtures (sand vs air), roughness, atlas bounds,
// pool eviction and fades, determinism x2, loud failures, twin refusal.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn cam_64() -> Pinhole {
        Pinhole {
            width_px: 64,
            height_px: 64,
            focal_px: 32.0,
            near_m: 0.5,
            far_m: 60.0,
        }
    }

    fn gore_at(depth: f32, radius: f32) -> GoreEvent {
        GoreEvent {
            pos_view_m: [0.0, 0.0, -depth],
            normal: [0.0, 1.0, 0.0],
            radius_m: radius,
            severity: 2,
            kind: DecalKind::Blood,
        }
    }

    #[test]
    fn projection_hits_frame_center_with_scaled_radius() {
        let s = project_view(&cam_64(), [0.0, 0.0, -5.0], 0.5).expect("projects");
        assert_eq!((s.x_px, s.y_px), (32.0, 32.0));
        assert!(
            (s.radius_px - 3.2).abs() < 1e-4,
            "radius scales by focal/depth"
        );
        assert_eq!(s.depth_m, 5.0);
        s.validate().expect("screen validates");
        let off = project_view(&cam_64(), [2.5, 1.0, -5.0], 0.5).expect("projects");
        assert!((off.x_px - 48.0).abs() < 1e-4);
        assert!((off.y_px - 25.6).abs() < 1e-4);
    }

    #[test]
    fn projection_rejects_behind_camera_off_frame_and_bad_input() {
        assert!(matches!(
            project_view(&cam_64(), [0.0, 0.0, 2.0], 0.5),
            Err(DecalError::Invalid(_))
        ));
        assert!(matches!(
            project_view(&cam_64(), [40.0, 0.0, -5.0], 0.5),
            Err(DecalError::Invalid(_))
        ));
        assert!(matches!(
            project_view(&cam_64(), [0.0, 0.0, -500.0], 0.5),
            Err(DecalError::Invalid(_))
        ));
        assert!(matches!(
            project_view(&cam_64(), [f32::NAN, 0.0, -5.0], 0.5),
            Err(DecalError::Invalid(_))
        ));
        assert!(matches!(
            project_view(&cam_64(), [0.0, 0.0, -5.0], 0.0),
            Err(DecalError::Invalid(_))
        ));
        assert!(matches!(
            project_view(&cam_64(), [0.0, 0.0, -5.0], MAX_DECAL_RADIUS_M + 1.0),
            Err(DecalError::Invalid(_))
        ));
        let bad = Pinhole {
            width_px: 0,
            ..cam_64()
        };
        assert!(matches!(
            project_view(&bad, [0.0, 0.0, -5.0], 0.5),
            Err(DecalError::Invalid(_))
        ));
    }

    #[test]
    fn decal_lands_on_sand_not_air_in_depth_fixtures() {
        let cam = cam_64();
        let sand = DepthFrame::flat(64, 64, 5.0).expect("sand builds");
        let screen = project_view(&cam, [0.0, 0.0, -5.0], 0.5).expect("projects");
        let hit = depth_reject(&screen, &sand).expect("sand accepts");
        assert!(hit.coverage_px > 0, "footprint covers sand pixels");
        assert_eq!((hit.max_lift_m, hit.max_bury_m), (0.0, 0.0));
        let air = project_view(&cam, [0.0, 0.0, -4.0], 0.5).expect("projects");
        let err = depth_reject(&air, &sand).expect_err("air must reject");
        match err {
            DecalError::Invalid(msg) => {
                assert!(msg.contains("air"), "air rejection names air, got {msg}");
            }
            other => panic!("air must be Invalid, got {other:?}"),
        }
        let buried = project_view(&cam, [0.0, 0.0, -6.0], 0.5).expect("projects");
        let err = depth_reject(&buried, &sand).expect_err("buried must reject");
        match err {
            DecalError::Invalid(msg) => assert!(
                msg.contains("buried"),
                "buried rejection names burial, got {msg}"
            ),
            other => panic!("buried must be Invalid, got {other:?}"),
        }
        let mut pool = DecalPool::new();
        pool.place(&cam, &sand, &gore_at(5.0, 0.5))
            .expect("sand placement lands");
        assert_eq!(pool.len(), 1);
        assert!(matches!(
            pool.place(&cam, &sand, &gore_at(4.0, 0.5)),
            Err(DecalError::Invalid(_))
        ));
        assert_eq!(pool.len(), 1, "rejected air leaves the pool untouched");
    }

    #[test]
    fn depth_step_edge_rejects_the_air_side() {
        let cam = cam_64();
        let mut samples = vec![5.0f32; 64 * 64];
        for y in 0..64 {
            for x in 32..64 {
                samples[y * 64 + x] = 8.0;
            }
        }
        let frame = DepthFrame::build(64, 64, samples).expect("step builds");
        // Near plateau point projects well left of the x=32 step, so its
        // small footprint never touches the far side.
        let near = project_view(&cam, [-2.0, 0.0, -5.0], 0.3).expect("projects");
        assert!(near.x_px < 32.0, "near point stays left, got {}", near.x_px);
        depth_reject(&near, &frame).expect("near side lands on its plateau");
        // Far plateau point at its own depth lands; the same pixels with the
        // near depth float 3 m in front of the far plateau, so they reject.
        let far_lands = project_view(&cam, [2.0, 0.0, -8.0], 0.3).expect("projects");
        assert!(far_lands.x_px > 32.0, "far point stays right");
        depth_reject(&far_lands, &frame).expect("far side lands on its plateau");
        let far = ScreenDecal {
            x_px: 48.0,
            y_px: 32.0,
            depth_m: 5.0,
            radius_px: 2.0,
        };
        assert!(matches!(
            depth_reject(&far, &frame),
            Err(DecalError::Invalid(_))
        ));
    }

    #[test]
    fn blood_roughness_dries_wet_to_matte_and_clamps() {
        assert_eq!(blood_roughness(0), BLOOD_ROUGH_WET);
        assert_eq!(blood_roughness(BLOOD_FADE_TICKS), BLOOD_ROUGH_DRY);
        assert_eq!(blood_roughness(u64::MAX), BLOOD_ROUGH_DRY);
        let mid = blood_roughness(300);
        assert!(mid > BLOOD_ROUGH_WET && mid < BLOOD_ROUGH_DRY);
        assert_eq!(mid, 0.5 * (BLOOD_ROUGH_WET + BLOOD_ROUGH_DRY));
        let (mut prev, mut monotonic) = (blood_roughness(0), true);
        for age in [60, 120, 300, 599, 600] {
            let r = blood_roughness(age);
            if r < prev {
                monotonic = false;
            }
            prev = r;
        }
        assert!(monotonic, "drying never gets glossier");
        assert_eq!(blood_alpha(0), 1.0);
        assert_eq!(blood_alpha(BLOOD_FADE_TICKS), 0.0);
        assert_eq!(blood_alpha(u64::MAX), 0.0);
    }

    #[test]
    fn atlas_is_bounded_with_lowest_free_reuse() {
        let mut atlas = DecalAtlas::with_capacity(4);
        assert_eq!(
            (atlas.capacity(), atlas.used(), atlas.free_count()),
            (4, 0, 4)
        );
        assert_eq!(atlas.capacity_bytes(), 4 * ATLAS_TILE_BYTES);
        let mut slots = Vec::new();
        for _ in 0..4 {
            slots.push(atlas.alloc().expect("tiles alloc"));
        }
        assert_eq!(slots, vec![0, 1, 2, 3]);
        assert_eq!(atlas.used_bytes(), 4 * ATLAS_TILE_BYTES);
        assert!(matches!(
            atlas.alloc(),
            Err(DecalError::BudgetExceeded {
                requested: 5,
                allowed: 4
            })
        ));
        atlas.free(1).expect("slot 1 frees");
        assert_eq!(atlas.free_count(), 1);
        assert_eq!(atlas.alloc().expect("reuses lowest"), 1);
        assert!(matches!(atlas.free(9), Err(DecalError::Invalid(_))));
        atlas.free(1).expect("reused slot 1 frees again");
        assert!(
            matches!(atlas.free(1), Err(DecalError::Invalid(_))),
            "double free fails loudly"
        );
        let double_free = {
            atlas.free(2).expect("slot 2 frees");
            atlas.free(2)
        };
        assert!(matches!(double_free, Err(DecalError::Invalid(_))));
    }

    #[test]
    fn pool_evicts_oldest_and_recycles_its_tile() {
        let cam = cam_64();
        let sand = DepthFrame::flat(64, 64, 5.0).expect("sand builds");
        let mut pool = DecalPool::with_capacity(2);
        let s0 = pool.place(&cam, &sand, &gore_at(5.0, 0.3)).expect("lands");
        let s1 = pool.place(&cam, &sand, &gore_at(5.0, 0.4)).expect("lands");
        assert_ne!(s0, s1);
        assert_eq!(
            (pool.len(), pool.tiles_used(), pool.evicted_count()),
            (2, 2, 0)
        );
        pool.place(&cam, &sand, &gore_at(5.0, 0.5)).expect("evicts");
        assert_eq!((pool.len(), pool.tiles_used()), (2, 2));
        assert_eq!(pool.evicted_count(), 1, "full atlas evicts the oldest once");
        let radii: Vec<f32> = pool.decals().iter().map(|d| d.radius_m).collect();
        assert_eq!(radii.len(), 2);
        assert!(
            radii.contains(&0.4) && radii.contains(&0.5),
            "oldest gone, got {radii:?}"
        );
        assert_eq!(pool.upload_instances().len(), 2 * 32);
        assert_eq!(pool.upload_roughness().len(), 2 * 4);
    }

    #[test]
    fn pool_fade_schedule_culls_and_frees_tiles() {
        let cam = cam_64();
        let sand = DepthFrame::flat(64, 64, 5.0).expect("sand builds");
        let mut pool = DecalPool::new();
        pool.place(&cam, &sand, &gore_at(5.0, 0.5)).expect("lands");
        assert_eq!(blood_alpha(0), 1.0);
        for _ in 0..300 {
            pool.tick();
        }
        assert_eq!(pool.len(), 1);
        assert_eq!(pool.decals()[0].age_ticks, 300);
        assert_eq!(pool.decals()[0].roughness(), blood_roughness(300));
        assert_eq!(pool.tiles_used(), 1);
        let bytes = pool.upload_instances();
        let alpha = f32::from_le_bytes(bytes[28..32].try_into().expect("alpha reads"));
        assert_eq!(alpha, blood_alpha(300));
        for _ in 300..BLOOD_FADE_TICKS {
            pool.tick();
        }
        assert!(pool.is_empty(), "fade terminal culls the decal");
        assert_eq!(pool.tiles_used(), 0, "cull frees the tile");
        assert_eq!(pool.tick_count(), BLOOD_FADE_TICKS);
    }

    #[test]
    fn placement_script_is_deterministic_with_stable_bytes() {
        let cam = cam_64();
        let sand = DepthFrame::flat(64, 64, 5.0).expect("sand builds");
        let script = [
            gore_at(5.0, 0.3),
            gore_at(5.0, 0.5),
            GoreEvent {
                pos_view_m: [1.0, 0.5, -5.0],
                normal: [0.0, 1.0, 0.0],
                radius_m: 0.4,
                severity: 1,
                kind: DecalKind::Blood,
            },
        ];
        let mut a = DecalPool::new();
        let mut b = DecalPool::new();
        for event in &script {
            assert_eq!(
                a.place(&cam, &sand, event).expect("lands"),
                b.place(&cam, &sand, event).expect("lands")
            );
        }
        assert_eq!(a.state_bytes(), b.state_bytes(), "bytes identical twice");
        assert_eq!(a.upload_instances(), b.upload_instances());
        assert_eq!(a.upload_roughness(), b.upload_roughness());
        assert_ne!(a.state_bytes(), DecalPool::new().state_bytes());
    }

    #[test]
    fn bad_inputs_fail_loudly_without_touching_the_pool() {
        let cam = cam_64();
        let sand = DepthFrame::flat(64, 64, 5.0).expect("sand builds");
        let mut pool = DecalPool::new();
        let bad_events = [
            GoreEvent {
                pos_view_m: [f32::NAN, 0.0, -5.0],
                ..gore_at(5.0, 0.5)
            },
            GoreEvent {
                normal: [0.0, 0.0, 0.0],
                ..gore_at(5.0, 0.5)
            },
            GoreEvent {
                radius_m: 0.0,
                ..gore_at(5.0, 0.5)
            },
            GoreEvent {
                radius_m: MAX_DECAL_RADIUS_M + 0.5,
                ..gore_at(5.0, 0.5)
            },
            GoreEvent {
                severity: 0,
                ..gore_at(5.0, 0.5)
            },
            GoreEvent {
                severity: 9,
                ..gore_at(5.0, 0.5)
            },
        ];
        for event in &bad_events {
            assert!(matches!(
                pool.place(&cam, &sand, event),
                Err(DecalError::Invalid(_))
            ));
        }
        assert!(pool.is_empty());
        assert!(matches!(
            DepthFrame::build(4, 4, vec![5.0; 15]),
            Err(DecalError::Invalid(_))
        ));
        let mut bad_depth = vec![5.0f32; 64 * 64];
        bad_depth[7] = f32::NAN;
        assert!(matches!(
            DepthFrame::build(64, 64, bad_depth),
            Err(DecalError::Invalid(_))
        ));
        let mut zero_depth = vec![5.0f32; 64 * 64];
        zero_depth[7] = 0.0;
        assert!(matches!(
            DepthFrame::build(64, 64, zero_depth),
            Err(DecalError::Invalid(_))
        ));
        let wrong = DepthFrame::flat(32, 32, 5.0).expect("builds");
        assert!(matches!(
            pool.place(&cam, &wrong, &gore_at(5.0, 0.5)),
            Err(DecalError::Invalid(_))
        ));
        assert!(matches!(sand.sample(99, 0), Err(DecalError::Invalid(_))));
        assert_eq!(cam.pixels(), 64 * 64);
        assert!(gore_at(5.0, 0.5).validate().is_ok());
    }

    #[test]
    #[cfg(not(feature = "window"))]
    fn fc3_empty_pool_skips_decal_submit() {
        // FC3 skip switch: idle frames carry no decals, so the RP3 pass skips
        // the bind entirely (0 decals, 0 tiles, 0 ms, 0 errors) instead of
        // requiring a device. Placed pools still need the window device.
        let empty = DecalPool::new();
        assert!(empty.is_empty(), "new pool carries no decals");
        let cam = cam_64();
        let skipped = empty
            .submit_gpu(&cam)
            .expect("empty skips without a device");
        assert_eq!(
            (skipped.decals, skipped.tiles_used, skipped.coverage_px),
            (0, 0, 0)
        );
        assert_eq!(skipped.ms, 0.0);
        assert!(skipped.within_budget());
        let sand = DepthFrame::flat(64, 64, 5.0).expect("sand builds");
        let mut pool = DecalPool::new();
        pool.place(&cam, &sand, &gore_at(5.0, 0.5)).expect("lands");
        assert!(!pool.is_empty(), "placed pool must run the decal pass");
        assert!(matches!(
            pool.submit_gpu(&cam),
            Err(DecalError::NoDevice(_))
        ));
    }

    #[test]
    #[cfg(not(feature = "window"))]
    fn gpu_submit_without_device_names_the_window_build() {
        let cam = cam_64();
        let sand = DepthFrame::flat(64, 64, 5.0).expect("sand builds");
        let mut pool = DecalPool::new();
        pool.place(&cam, &sand, &gore_at(5.0, 0.5)).expect("lands");
        assert!(matches!(
            pool.submit_gpu(&cam),
            Err(DecalError::NoDevice(_))
        ));
        let bad = Pinhole {
            width_px: 0,
            ..cam_64()
        };
        assert!(matches!(pool.submit_gpu(&bad), Err(DecalError::Invalid(_))));
        let receipt = pool.receipt();
        assert_eq!((receipt.decals, receipt.tiles_used), (1, 1));
        assert_eq!(receipt.coverage_px, pool.decals()[0].coverage_px as u64);
        assert_eq!(receipt.budget_ms(), DECAL_BUDGET_MS);
        assert!(receipt.ms >= 0.0);
    }

    #[test]
    fn error_display_names_each_variant() {
        let invalid = format!("{}", DecalError::Invalid("x".into()));
        assert!(invalid.contains("decal input"));
        let budget = format!(
            "{}",
            DecalError::BudgetExceeded {
                requested: 9,
                allowed: 4
            }
        );
        assert!(budget.contains('9') && budget.contains('4'));
        let nodevice = format!("{}", DecalError::NoDevice("build this".into()));
        assert!(nodevice.contains("device"));
        let device = format!("{}", DecalError::Device("map failed".into()));
        assert!(device.contains("map failed"));
        let validation = format!("{}", DecalError::Validation("scope hit".into()));
        assert!(validation.contains("scope hit"));
    }

    // ----- Window device tests (behind `window`; skip loudly without one) -----

    /// Device shared with the GX1 suite: one device for the run, so parallel
    /// tests never exhaust adapters. `None` is skip-not-green, never a pass.
    #[cfg(feature = "window")]
    fn decal_device() -> Option<(
        &'static wgpu::Device,
        &'static wgpu::Queue,
        &'static wgpu::AdapterInfo,
    )> {
        crate::graph::gx1_test_device()
    }

    #[cfg(feature = "window")]
    fn decal_frame() -> crate::gtao::LookFrame {
        crate::gtao::LookFrame {
            frame_index: 9,
            viewport_px: [64, 64],
            exposure_ev100: 10.0,
            sun_dir: [0.3, 0.8, 0.5],
        }
    }

    #[test]
    #[cfg(feature = "window")]
    fn gpu_placed_pool_binds_exact_with_look_frame() {
        let Some((device, queue, info)) = decal_device() else {
            eprintln!("SKIPPED decal bind: no wgpu adapter on this machine");
            return;
        };
        let cam = cam_64();
        let sand = DepthFrame::flat(64, 64, 5.0).expect("sand builds");
        let mut pool = DecalPool::new();
        pool.place(&cam, &sand, &gore_at(5.0, 0.5))
            .expect("sand placement lands");
        let mut submitter = DecalSubmitter::new(device);
        assert_eq!(submitter.created_buffers(), 0);
        let receipt = pool
            .submit_gpu(device, queue, &decal_frame(), &cam, &mut submitter)
            .expect("placed pool binds clean");
        assert!(receipt.is_clean(), "0 validation errors");
        assert_eq!((receipt.decals, receipt.tiles_used), (1, 1));
        assert_eq!(receipt.coverage_px, pool.decals()[0].coverage_px as u64);
        assert_eq!(receipt.budget_ms(), DECAL_BUDGET_MS);
        // Steady state reuses the sized buffers: no per-frame allocation.
        let before = submitter.created_buffers();
        let again = pool
            .submit_gpu(device, queue, &decal_frame(), &cam, &mut submitter)
            .expect("second bind reuses");
        assert!(again.is_clean());
        assert_eq!(
            submitter.created_buffers(),
            before,
            "matching-shape submits size nothing"
        );
        eprintln!(
            "RP3 decal bind on {}: decals={} coverage={} submit_ms={:.3} resubmit_ms={:.3} (budget {DECAL_BUDGET_MS} ms)",
            info.name, receipt.decals, receipt.coverage_px, receipt.submit_ms, again.submit_ms
        );
    }

    #[test]
    #[cfg(feature = "window")]
    fn gpu_empty_pool_still_binds() {
        let Some((device, queue, _)) = decal_device() else {
            eprintln!("SKIPPED decal empty bind: no wgpu adapter on this machine");
            return;
        };
        // Idle frames still record the pass over one zero word.
        let pool = DecalPool::new();
        let mut submitter = DecalSubmitter::new(device);
        let receipt = pool
            .submit_gpu(device, queue, &decal_frame(), &cam_64(), &mut submitter)
            .expect("empty pool binds clean");
        assert!(receipt.is_clean());
        assert_eq!((receipt.decals, receipt.tiles_used), (0, 0));
    }

    #[test]
    #[cfg(feature = "window")]
    fn gpu_refuses_bad_camera_before_sizing() {
        let Some((device, queue, _)) = decal_device() else {
            eprintln!("SKIPPED decal refusals: no wgpu adapter on this machine");
            return;
        };
        let cam = cam_64();
        let sand = DepthFrame::flat(64, 64, 5.0).expect("sand builds");
        let mut pool = DecalPool::new();
        pool.place(&cam, &sand, &gore_at(5.0, 0.5)).expect("lands");
        let mut submitter = DecalSubmitter::new(device);
        assert_eq!(submitter.created_buffers(), 0);
        let bad = Pinhole {
            width_px: 0,
            ..cam_64()
        };
        assert!(matches!(
            pool.submit_gpu(device, queue, &decal_frame(), &bad, &mut submitter),
            Err(DecalError::Invalid(_))
        ));
        assert_eq!(submitter.created_buffers(), 0, "refusals size no buffers");
    }

    #[test]
    fn decal_receipt_budget_accessors_are_exact() {
        // Exactness default (no wall clock): bar value plus synthetic
        // receipts pin the 0.5 ms flag both ways. Boosted-clock
        // enforcement lives in decal_place_wall_budget (--perf only).
        assert_eq!(DECAL_BUDGET_MS, 0.5, "decal bar stays 0.5 ms");
        let at = DecalReceipt {
            decals: 1,
            tiles_used: 1,
            coverage_px: 64,
            ms: DECAL_BUDGET_MS,
        };
        assert_eq!(at.budget_ms(), DECAL_BUDGET_MS);
        assert!(at.within_budget(), "exactly at budget is within");
        let over = DecalReceipt {
            ms: DECAL_BUDGET_MS + 0.5,
            ..at
        };
        assert!(!over.within_budget(), "over budget fails loudly");
    }

    #[test]
    #[ignore = "timing: decal place wall-clock needs release --perf on boosted clocks (fails under parallel load)"]
    fn decal_place_wall_budget() {
        // Seeded budget regression: the same sand placement the default
        // tests prove exact must clear the 0.5 ms bar on boosted release
        // clocks. Perf harness line keeps --perf medians readable.
        let cam = cam_64();
        let sand = DepthFrame::flat(64, 64, 5.0).expect("sand builds");
        let mut pool = DecalPool::new();
        let started = std::time::Instant::now();
        pool.place(&cam, &sand, &gore_at(5.0, 0.5))
            .expect("sand placement lands");
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        let receipt = pool.receipt();
        eprintln!(
            "decal_place_wall_budget: measured {ms:.3} ms vs {:.1} ms budget",
            receipt.budget_ms()
        );
        assert!(
            ms <= receipt.budget_ms(),
            "seeded place over budget: {ms:.3} ms vs {:.1} ms",
            receipt.budget_ms()
        );
    }
}
