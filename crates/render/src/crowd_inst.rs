//! crowd_inst: instanced reacting crowd staging (slice 1).
//!
//! Owner: render lane. Units: meters for seats and bounds, permille
//! (`0..=1000`) for excitement and bob phase, counts for instances.
//! Determinism: seat math uses `+ - * /` and one `sqrt` in fixed
//! sector/row/col order, so the same bytes give the same frame twice;
//! visuals are client-local and never hashed. Errors are typed
//! [`CrowdInstError`]. Budget: staging is O(seats) over caller-owned
//! slices with no allocation; the posted frame bar is
//! [`CROWD_INST_BUDGET_MS`] ms. Caller: `crates/game/src/crowd_gpu.rs`
//! (the integer sim feeds one frame per window tick); the next slice
//! binds [`GpuBindRequest`] through `queue.write_buffer` in the presenter.

use engine_core::Fnv64;
use std::fmt;

/// Far-row impostor select over the staged crowd (octahedral atlas cell).
#[path = "crowd_impostor.rs"]
pub mod crowd_impostor;

const CROWD_VAT_WGSL: &str = include_str!("shaders/crowd_vat.wgsl");

// ---------------------------------------------------------------------------
// Bounds: the operating crowd and its per-frame bar.
// ---------------------------------------------------------------------------

/// Most spectators one frame stages: the FOCUS 20k-50k top.
pub const MAX_CROWD_SEATS: usize = 50_000;
/// Sectors around the pit: north, south, west, east straight stands.
pub const CROWD_SECTORS: usize = 4;
/// Chant blocks over the room: 16 per sector in a 4x4 grid.
pub const CROWD_BLOCKS: usize = 64;
/// Visual levels per seat: full body plus head plus bob, body only,
/// merged static. Distant seats freeze instead of popping.
pub const CROWD_LOD_LEVELS: u8 = 3;
/// Distance bands in meters: L0 below 18 m, L1 below 45 m, else L2.
pub const CROWD_LOD_BANDS_M: [f32; 2] = [18.0, 45.0];
/// Hysteresis half-width in meters around each band.
pub const CROWD_LOD_HYST_M: f32 = 1.5;
/// Sentinel LOD for frustum-culled seats: fixed index, skipped by the bind.
pub const CROWD_LOD_CULLED: u8 = 255;
/// Posted frame bar in ms for the crowd instance stage at 1080p.
pub const CROWD_INST_BUDGET_MS: f64 = 0.5;
/// Packed GPU words per instance: xyz bits plus two attribute words.
pub const CROWD_PACKED_WORDS: usize = 5;
/// Millimeters per meter for the integer depth digest.
const MM_PER_M: f32 = 1000.0;
/// Ceiling at 2^32 for the saturating millimeter cast: values at or past
/// this pin at `u32::MAX` instead of wrapping. Written as an exact
/// product of exact literals so the value stays exact without tripping
/// the lossy-float literal lint (a single 2^32 literal is not exact).
const U32_MAX_F: f32 = 8_388_608.0 * 512.0;

// Stand shape continues the present 96-seat ring (`draw::crowd_*`, same
// wall, rise and run), extended upward and outward for 20k-50k seats.

/// Inner wall face distance in meters, matches the present ring wall.
pub const CROWD_STAND_WALL_M: f32 = 21.0;
/// Pit wall top in meters.
pub const CROWD_STAND_WALL_H_M: f32 = 3.0;
/// Tier rise in meters, one step up per row.
pub const CROWD_STAND_RISE_M: f32 = 0.9;
/// Tier run in meters, one step back per row.
pub const CROWD_STAND_RUN_M: f32 = 1.4;
/// Spectator box width in meters.
pub const CROWD_BODY_W_M: f32 = 0.45;
/// Spectator torso box height in meters.
pub const CROWD_BODY_H_M: f32 = 1.1;
/// Spectator box depth in meters.
pub const CROWD_BODY_D_M: f32 = 0.35;

// ---------------------------------------------------------------------------
// Errors: typed, no allocation; bad input fails loudly.
// ---------------------------------------------------------------------------

/// Crowd staging failures; every variant names what to fix.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CrowdInstError {
    /// The layout is unusable; reason carried.
    BadLayout(&'static str),
    /// The camera is unusable; reason carried.
    BadCamera(&'static str),
    /// One frustum plane word is not finite; index carried.
    BadPlanes(usize),
    /// A caller buffer is shorter than the layout's seats; sizes carried.
    BadBuffer {
        /// Seats the layout holds.
        want: usize,
        /// Entries the caller buffer holds.
        got: usize,
    },
    /// One staged instance is unusable; seat index carried.
    BadInstance(usize),
    /// The request exceeds the operating cap; sizes carried.
    OverBudget {
        /// Seats requested.
        requested: u32,
        /// Seats the cap allows.
        allowed: u32,
    },
    /// Headless twin has no device; what to build first, carried.
    NoDevice(&'static str),
}

impl fmt::Display for CrowdInstError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadLayout(why) => write!(f, "bad crowd layout ({why})"),
            Self::BadCamera(why) => write!(f, "bad crowd camera ({why})"),
            Self::BadPlanes(index) => write!(f, "bad crowd plane {index} (not finite)"),
            Self::BadBuffer { want, got } => {
                write!(f, "crowd buffer holds {got}, layout needs {want}")
            }
            Self::BadInstance(index) => write!(f, "bad crowd instance {index}"),
            Self::OverBudget { requested, allowed } => write!(
                f,
                "crowd budget exceeded ({requested} requested, {allowed} allowed)"
            ),
            Self::NoDevice(why) => write!(f, "crowd bind needs a device ({why})"),
        }
    }
}

impl std::error::Error for CrowdInstError {}

// ---------------------------------------------------------------------------
// Layout: four straight stands in fixed sector order.
// ---------------------------------------------------------------------------

/// Seat grid over the four stands: every sector holds `rows_per_sector`
/// times `cols_per_row` seats, so seats equal `4 * rows * cols`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SeatLayout {
    /// Tier rows per sector, `1..=64`.
    pub rows_per_sector: u16,
    /// Seats per row, `1..=320`.
    pub cols_per_row: u16,
}

impl SeatLayout {
    /// Check rows, cols and the operating cap before a build trusts this.
    pub fn validate(&self) -> Result<(), CrowdInstError> {
        if self.rows_per_sector == 0 || self.rows_per_sector > 64 {
            return Err(CrowdInstError::BadLayout("rows_per_sector outside 1..=64"));
        }
        if self.cols_per_row == 0 || self.cols_per_row > 320 {
            return Err(CrowdInstError::BadLayout("cols_per_row outside 1..=320"));
        }
        let total = 4u32
            .saturating_mul(u32::from(self.rows_per_sector))
            .saturating_mul(u32::from(self.cols_per_row));
        let max = u32::try_from(MAX_CROWD_SEATS).unwrap_or(u32::MAX);
        if total > max {
            return Err(CrowdInstError::OverBudget {
                requested: total,
                allowed: max,
            });
        }
        Ok(())
    }

    /// Seats the grid holds: `4 * rows * cols`, exact.
    #[must_use]
    pub fn seat_count(&self) -> usize {
        let total = 4u32
            .saturating_mul(u32::from(self.rows_per_sector))
            .saturating_mul(u32::from(self.cols_per_row));
        usize::try_from(total).unwrap_or(0)
    }

    /// Smallest 50-row layout covering `total` seats (`1..=MAX`).
    /// Production 20k seats give 50 rows by 100 cols; tests use small
    /// literals instead. Refuses 0 and anything past the cap.
    pub fn for_seats(total: u32) -> Result<Self, CrowdInstError> {
        let max = u32::try_from(MAX_CROWD_SEATS).unwrap_or(u32::MAX);
        if total == 0 {
            return Err(CrowdInstError::BadLayout("for_seats needs at least 1"));
        }
        if total > max {
            return Err(CrowdInstError::OverBudget {
                requested: total,
                allowed: max,
            });
        }
        let per_sector = total.div_ceil(4);
        let cols = per_sector.div_ceil(50).clamp(1, 320);
        Ok(Self {
            rows_per_sector: 50,
            cols_per_row: u16::try_from(cols).unwrap_or(320),
        })
    }

    /// Feet position for one seat in meters, feet on its tread top.
    /// Sector 0 faces from the north (-z), 1 from the south, 2 from the
    /// west, 3 from the east; indices wrap, so any input lands on a seat.
    #[must_use]
    pub fn seat_pos(&self, sector: u8, row: u16, col: u16) -> [f32; 3] {
        let sec = sector % 4;
        let rows = self.rows_per_sector.max(1);
        let cols = self.cols_per_row.max(1);
        let r = row % rows;
        let c = col % cols;
        let depth = CROWD_STAND_WALL_M + CROWD_STAND_RUN_M * (f32::from(r) + 1.5);
        let height = CROWD_STAND_WALL_H_M + CROWD_STAND_RISE_M * (f32::from(r) + 1.0);
        let width = 8.0 + depth * 0.5;
        let col01 = (f32::from(c) + 0.5) / f32::from(cols);
        let lateral = (col01 - 0.5) * width;
        if sec == 0 {
            [lateral, height, -depth]
        } else if sec == 1 {
            [lateral, height, depth]
        } else if sec == 2 {
            [-depth, height, lateral]
        } else {
            [depth, height, lateral]
        }
    }

    /// Axis bounds for one sector in meters, min then max, from its four
    /// corner seats plus the body half extents. O(1); the cull hook runs
    /// one sphere test per sector over this box, never per-seat tests.
    #[must_use]
    pub fn sector_bounds(&self, sector: u8) -> [[f32; 3]; 2] {
        let rows = self.rows_per_sector.max(1);
        let cols = self.cols_per_row.max(1);
        let corners = [
            self.seat_pos(sector, 0, 0),
            self.seat_pos(sector, 0, cols - 1),
            self.seat_pos(sector, rows - 1, 0),
            self.seat_pos(sector, rows - 1, cols - 1),
        ];
        let mut lo = corners[0];
        let mut hi = corners[0];
        for corner in corners.iter().skip(1) {
            for axis in 0..3 {
                if corner[axis] < lo[axis] {
                    lo[axis] = corner[axis];
                }
                if corner[axis] > hi[axis] {
                    hi[axis] = corner[axis];
                }
            }
        }
        lo[0] -= CROWD_BODY_W_M * 0.5;
        hi[0] += CROWD_BODY_W_M * 0.5;
        hi[1] += CROWD_BODY_H_M;
        lo[2] -= CROWD_BODY_D_M * 0.5;
        hi[2] += CROWD_BODY_D_M * 0.5;
        [lo, hi]
    }
}

/// Chant block for one seat: 16 blocks per sector in a 4x4 row/col grid,
/// so the integer sim's 64 excitement entries line up with the stands.
/// Indices wrap; every seat belongs to exactly one block.
#[must_use]
pub fn block_for_seat(sector: u8, row: u16, col: u16) -> u8 {
    let sec = sector % 4;
    let r = u8::try_from(row % 4).unwrap_or(3);
    let c = u8::try_from(col % 4).unwrap_or(3);
    sec.saturating_mul(16)
        .saturating_add(r.saturating_mul(4).saturating_add(c))
}

/// Bob phase in permille (`0..=999`) for one block at one tick: a fixed
/// integer wave, so the same tick stages the same phase twice. Visuals
/// only; excitement lives in the game sim, never here.
#[must_use]
pub fn phase_for(block: u8, tick: u64) -> u16 {
    let wave = (tick % 1000)
        .saturating_mul(7)
        .saturating_add(u64::from(block % 64).saturating_mul(131))
        % 1000;
    u16::try_from(wave).unwrap_or(999)
}

/// Per-seat parts from index plus excitement via FNV: 6-bit tint, 3-bit
/// variant, permille offset, centimetre jitter. Same inputs give the
/// same parts twice; neighbours differ by lattice (O(1), no allocation).
#[must_use]
pub(crate) fn seat_parts(seat: u32, block: u8, excitement: u16) -> (u8, u8, u16, [i8; 3]) {
    let mut h = Fnv64::new();
    h.write_u32(seat);
    h.write_u8(block);
    h.write_u16(excitement.min(1000));
    let v = h.finish();
    let lat = seat.saturating_mul(37);
    let low = u32::try_from(v % 64).unwrap_or(0);
    let tint = u8::try_from(lat.saturating_add(low) % 64).unwrap_or(0);
    let variant = u8::try_from((v >> 6) % 8).unwrap_or(0);
    let offset = u16::try_from((v >> 9) % 1000).unwrap_or(0);
    let jx = i8::try_from((v >> 19) % 5).unwrap_or(2) - 2;
    let jy = i8::try_from((v >> 22) % 5).unwrap_or(2) - 2;
    let jz = i8::try_from((v >> 25) % 5).unwrap_or(2) - 2;
    (tint, variant, offset, [jx, jy, jz])
}

// ---------------------------------------------------------------------------
// LOD: hysteresis bands over camera distance, then the cull hook.
// ---------------------------------------------------------------------------

/// Pick the visual level for one seat: coarsen only past `band + hyst`
/// and refine only below `band - hyst`, so jitter inside the band holds
/// (discrete LOD chains switch outward with distance [gearus | 11.1.1]).
/// A non-finite distance or an out-of-range previous level falls back to
/// the coarsest live level instead of failing the frame.
#[must_use]
pub fn pick_crowd_lod(dist_m: f32, prev_lod: u8) -> u8 {
    if !dist_m.is_finite() {
        return CROWD_LOD_LEVELS - 1;
    }
    let mut lod = prev_lod.min(CROWD_LOD_LEVELS - 1);
    while lod < CROWD_LOD_LEVELS - 1
        && dist_m > CROWD_LOD_BANDS_M[usize::from(lod)] + CROWD_LOD_HYST_M
    {
        lod += 1;
    }
    while lod > 0 && dist_m < CROWD_LOD_BANDS_M[usize::from(lod - 1)] - CROWD_LOD_HYST_M {
        lod -= 1;
    }
    lod
}

/// True when one sector's bound sphere survives the six packed planes
/// through the shared CPU oracle (the same `-radius` rule the GPU cull
/// shader runs). One sphere test per sector, never per-seat culls.
#[must_use]
pub fn sector_visible(layout: &SeatLayout, sector: u8, planes: &[[f32; 4]; 6]) -> bool {
    let bounds = layout.sector_bounds(sector % 4);
    let center = [
        (bounds[0][0] + bounds[1][0]) * 0.5,
        (bounds[0][1] + bounds[1][1]) * 0.5,
        (bounds[0][2] + bounds[1][2]) * 0.5,
    ];
    let dx = bounds[1][0] - bounds[0][0];
    let dy = bounds[1][1] - bounds[0][1];
    let dz = bounds[1][2] - bounds[0][2];
    let radius = (dx * dx + dy * dy + dz * dz).sqrt() * 0.5;
    crate::draw::sphere_visible_cpu(center, radius, planes)
}

/// Sector flags in sector order: the per-frame cull hook. All four read
/// through [`sector_visible`]; a non-finite plane word culls everything
/// instead of failing the frame.
#[must_use]
pub fn visible_sectors(layout: &SeatLayout, planes: &[[f32; 4]; 6]) -> [bool; 4] {
    for plane in planes.iter() {
        for word in plane.iter() {
            if !word.is_finite() {
                return [false; 4];
            }
        }
    }
    [
        sector_visible(layout, 0, planes),
        sector_visible(layout, 1, planes),
        sector_visible(layout, 2, planes),
        sector_visible(layout, 3, planes),
    ]
}

// ---------------------------------------------------------------------------
// Staging: caller-owned buffers in, one instanced frame out.
// ---------------------------------------------------------------------------

/// One staged seat: fixed index order (sector, row, col), so the CPU
/// buffer maps 1:1 onto the GPU upload the next slice binds.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CrowdInstance {
    /// Feet position in meters.
    pub pos_m: [f32; 3],
    /// Palette slot `0..=3`, cycled by sector plus row plus col.
    pub palette: u8,
    /// Visual level, or [`CROWD_LOD_CULLED`] past the frustum.
    pub lod: u8,
    /// Bob phase in permille (`0..=999`).
    pub phase_permille: u16,
    /// Widened tint `0..64` (6 bits in word3 `18..23`).
    pub tint: u8,
    /// Offset variant `0..8` (3 bits folded into phase high bits).
    pub variant: u8,
}

impl CrowdInstance {
    /// Culled sentinel: fixed index, skipped by the bind, digest-stable.
    #[must_use]
    pub fn culled(pos_m: [f32; 3]) -> Self {
        Self {
            pos_m,
            palette: 0,
            lod: CROWD_LOD_CULLED,
            phase_permille: 0,
            tint: 0,
            variant: 0,
        }
    }

    /// True while the seat survived the frustum.
    #[must_use]
    pub fn is_visible(self) -> bool {
        self.lod != CROWD_LOD_CULLED
    }

    /// True while the seat takes the quad path past the L1 band.
    #[must_use]
    pub fn is_impostor(self) -> bool {
        self.lod == 2
    }
}

/// One staged frame: counts plus the stability digest.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CrowdFrameStats {
    /// Sectors surviving the cull hook, `0..=4`.
    pub sectors_visible: u8,
    /// Visible seats staged.
    pub instances: u32,
    /// Visible seats per live LOD level.
    pub lod_counts: [u32; 3],
    /// FNV-1a digest over every staged seat in index order; equal bytes
    /// twice prove the build is bit-stable.
    pub digest: u64,
}

impl CrowdFrameStats {
    /// Posted frame bar this stage is charged against.
    #[must_use]
    pub fn budget_ms(self) -> f64 {
        CROWD_INST_BUDGET_MS
    }
}

/// Caller-owned frame builder: layout plus precomputed sector flags plus two reused buffers.
/// Construct once per window, rebuild every frame with no allocation.
pub struct CrowdBuilder<'a> {
    layout: &'a SeatLayout,
    seats: usize,
    block_levels: &'a [u16; CROWD_BLOCKS],
    tick: u64,
    camera_m: [f32; 3],
    flags: [bool; CROWD_SECTORS],
    prev_lod: &'a mut [u8],
    out: &'a mut [CrowdInstance],
}

impl<'a> CrowdBuilder<'a> {
    /// Bind caller buffers to one frame: validates the layout, the
    /// camera, the planes and both buffer lengths. Loud errors leave
    /// nothing half-written; the cold path may reject before the loop.
    pub fn new(
        layout: &'a SeatLayout,
        block_levels: &'a [u16; CROWD_BLOCKS],
        tick: u64,
        camera_m: [f32; 3],
        planes: &'a [[f32; 4]; 6],
        prev_lod: &'a mut [u8],
        out: &'a mut [CrowdInstance],
    ) -> Result<Self, CrowdInstError> {
        layout.validate()?;
        if !camera_m.iter().all(|c| c.is_finite()) {
            return Err(CrowdInstError::BadCamera("camera_m is not finite"));
        }
        for (index, plane) in planes.iter().enumerate() {
            for word in plane.iter() {
                if !word.is_finite() {
                    return Err(CrowdInstError::BadPlanes(index));
                }
            }
        }
        let seats = layout.seat_count();
        if prev_lod.len() < seats {
            return Err(CrowdInstError::BadBuffer {
                want: seats,
                got: prev_lod.len(),
            });
        }
        if out.len() < seats {
            return Err(CrowdInstError::BadBuffer {
                want: seats,
                got: out.len(),
            });
        }
        let flags = visible_sectors(layout, planes);
        Ok(Self {
            layout,
            seats,
            block_levels,
            tick,
            camera_m,
            flags,
            prev_lod,
            out,
        })
    }

    /// Stage every seat in fixed index order: culled sectors write the
    /// sentinel, visible seats pick LOD with hysteresis and carry their
    /// block excitement's phase. Buffers are refilled in place, so a warm
    /// frame allocates nothing. Budget: O(seats) time, one `sqrt` per seat.
    pub fn build(&mut self) -> CrowdFrameStats {
        let _wgsl = CROWD_VAT_WGSL.len();
        let mut hash = Fnv64::new();
        let mut instances: u32 = 0;
        let mut lod_counts = [0u32; 3];
        let mut sectors: u8 = 0;
        for flag in self.flags.iter() {
            if *flag {
                sectors = sectors.saturating_add(1);
            }
        }
        let rows = self.layout.rows_per_sector;
        let cols = self.layout.cols_per_row;
        let mut index = 0usize;
        for sector in 0..4u8 {
            let keep = self.flags[usize::from(sector)];
            for row in 0..rows {
                for col in 0..cols {
                    let at = self.layout.seat_pos(sector, row, col);
                    if keep {
                        let block = block_for_seat(sector, row, col);
                        let excite = self.block_levels[usize::from(block)];
                        let palette =
                            u8::try_from((u32::from(sector) + u32::from(row) + u32::from(col)) % 4)
                                .unwrap_or(0);
                        let seat_id = u32::try_from(index).unwrap_or(u32::MAX);
                        let (tint, variant, offset, jit) = seat_parts(seat_id, block, excite);
                        let phased = phase_for(block, self.tick).saturating_add(offset) % 1000;
                        let staged = [
                            at[0] + f32::from(jit[0]) * 0.01,
                            at[1] + f32::from(jit[1]) * 0.01,
                            at[2] + f32::from(jit[2]) * 0.01,
                        ];
                        let dx = staged[0] - self.camera_m[0];
                        let dy = staged[1] - self.camera_m[1];
                        let dz = staged[2] - self.camera_m[2];
                        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
                        let picked = pick_crowd_lod(dist, self.prev_lod[index]);
                        let lod = if crowd_impostor::select_impostor(dist) {
                            2
                        } else {
                            picked
                        };
                        self.prev_lod[index] = lod;
                        let seat = CrowdInstance {
                            pos_m: staged,
                            palette,
                            lod,
                            phase_permille: phased,
                            tint,
                            variant,
                        };
                        self.out[index] = seat;
                        instances = instances.saturating_add(1);
                        if lod < CROWD_LOD_LEVELS {
                            let slot = usize::from(lod);
                            lod_counts[slot] = lod_counts[slot].saturating_add(1);
                        }
                        mix_seat(&mut hash, &seat, excite);
                    } else {
                        let seat = CrowdInstance::culled(at);
                        self.out[index] = seat;
                        mix_seat(&mut hash, &seat, 0);
                    }
                    index += 1;
                }
            }
        }
        debug_assert!(index == self.seats);
        CrowdFrameStats {
            sectors_visible: sectors,
            instances,
            lod_counts,
            digest: hash.finish(),
        }
    }
}

/// Fold one staged seat into the frame digest: quantized millimeter
/// depth plus the attribute bytes, in fixed field order.
fn mix_seat(hash: &mut Fnv64, seat: &CrowdInstance, excitement_permille: u16) {
    for axis in 0..3 {
        hash.write_u32(dist_mm(seat.pos_m[axis]));
    }
    hash.write_u8(seat.lod);
    hash.write_u8(seat.palette);
    hash.write_u16(seat.phase_permille.min(1000));
    hash.write_u16(excitement_permille.min(1000));
    hash.write_u8(seat.tint.min(63));
    hash.write_u8(seat.variant.min(7));
}

/// Truncate a finite `1.0..2^32` float to `u32` with integer bit work,
/// so no numeric cast is needed. The caller range-pins `mm` first.
fn trunc_f32_to_u32(mm: f32) -> u32 {
    if mm < 1.0 {
        return 0;
    }
    let bits = mm.to_bits();
    let exp = (bits >> 23) & 0xFF;
    let mant = (bits & 0x7F_FFFF) | 0x80_0000;
    let e = exp.saturating_sub(127);
    if e >= 23 {
        mant << (e - 23)
    } else {
        mant >> (23 - e)
    }
}

/// Saturating float-to-millimeter bucket for the digest: non-finite and
/// non-positive values read 0, huge values pin at `u32::MAX`.
fn dist_mm(v: f32) -> u32 {
    if !v.is_finite() || v <= 0.0 {
        return 0;
    }
    let mm = v * MM_PER_M;
    if mm >= U32_MAX_F {
        return u32::MAX;
    }
    // The range check above holds `mm` inside `u32`, so truncation is exact.
    trunc_f32_to_u32(mm)
}

// ---------------------------------------------------------------------------
// Client-local visuals: integer phase in, meters out, never hashed.
// ---------------------------------------------------------------------------

/// Bob height in meters for one seat: a triangle wave over the integer
/// phase (exact, no trig) scaled by excitement. Phase and excitement
/// clamp at 1000 permille; phase 0 reads exactly 0. Presentation only.
#[must_use]
pub fn bob_offset_m(phase_permille: u16, excitement_permille: u16) -> f32 {
    let phase = phase_permille.min(1000);
    let tri = if phase <= 500 { phase } else { 1000 - phase };
    let excite = excitement_permille.min(1000);
    let amp = 0.02 + 0.12 * (f32::from(excite) / 1000.0);
    f32::from(tri) / 500.0 * amp
}

// ---------------------------------------------------------------------------
// GPU contract: packed words plus the headless twin refusal.
// ---------------------------------------------------------------------------

/// Bind request over one staged frame: a single contiguous upload the
/// next slice writes with `queue.write_buffer` (allocate once at max
/// size, never map inside the frame).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GpuBindRequest {
    /// Visible seats bound.
    pub instances: u32,
    /// Upload words (`instances * 5`), exact.
    pub words: u32,
    /// Upload bytes (`words * 4`), exact.
    pub bytes: u32,
}

impl GpuBindRequest {
    /// Posted frame bar this bind is charged against.
    #[must_use]
    pub fn budget_ms(self) -> f64 {
        CROWD_INST_BUDGET_MS
    }
}

/// Describe the upload for `instances` visible seats: words and bytes
/// are checked products, never wrapping. Refuses 0 and past-cap counts.
pub fn describe_bind(instances: usize) -> Result<GpuBindRequest, CrowdInstError> {
    let count = u32::try_from(instances).map_err(|_| CrowdInstError::OverBudget {
        requested: u32::MAX,
        allowed: u32::try_from(MAX_CROWD_SEATS).unwrap_or(u32::MAX),
    })?;
    let max = u32::try_from(MAX_CROWD_SEATS).unwrap_or(u32::MAX);
    if count == 0 {
        return Err(CrowdInstError::BadBuffer { want: 1, got: 0 });
    }
    if count > max {
        return Err(CrowdInstError::OverBudget {
            requested: count,
            allowed: max,
        });
    }
    let words = count
        .checked_mul(u32::try_from(CROWD_PACKED_WORDS).unwrap_or(5))
        .ok_or(CrowdInstError::OverBudget {
            requested: count,
            allowed: max,
        })?;
    let bytes = words.checked_mul(4).ok_or(CrowdInstError::OverBudget {
        requested: count,
        allowed: max,
    })?;
    Ok(GpuBindRequest {
        instances: count,
        words,
        bytes,
    })
}

/// Pack one seat into five words: xyz bits plus `lod | palette | phase`
/// plus excitement. Inputs clamp at their permille ceilings; the reader
/// below inverts this exactly.
pub fn pack_instance(
    pos_m: [f32; 3],
    lod: u8,
    palette: u8,
    phase_permille: u16,
    excitement_permille: u16,
    out: &mut [u32; CROWD_PACKED_WORDS],
) {
    pack_instance_full(
        &CrowdInstance {
            pos_m,
            palette,
            lod,
            phase_permille,
            tint: 0,
            variant: 0,
        },
        excitement_permille,
        out,
    );
}

/// Full pack with the 6-bit tint (`18..23`) and 3-bit variant folded
/// into phase high bits (`13..15`); phase stays 10 bits below.
pub fn pack_instance_full(
    seat: &CrowdInstance,
    excitement_permille: u16,
    out: &mut [u32; CROWD_PACKED_WORDS],
) {
    out[0] = seat.pos_m[0].to_bits();
    out[1] = seat.pos_m[1].to_bits();
    out[2] = seat.pos_m[2].to_bits();
    out[3] = (u32::from(seat.lod) << 24)
        | (u32::from(seat.tint.min(63)) << 18)
        | (u32::from(seat.palette & 3) << 16)
        | (u32::from(seat.variant.min(7)) << 13)
        | u32::from(seat.phase_permille.min(1000));
    out[4] = u32::from(excitement_permille.min(1000));
}

/// One unpacked bind cell: the exact inverse of [`pack_instance`].
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct UnpackedCell {
    /// Feet position in meters.
    pub pos_m: [f32; 3],
    /// Visual level.
    pub lod: u8,
    /// Palette slot.
    pub palette: u8,
    /// Widened tint `0..64`.
    pub tint: u8,
    /// Offset variant `0..8`.
    pub variant: u8,
    /// Bob phase in permille.
    pub phase_permille: u16,
    /// Excitement in permille.
    pub excitement_permille: u16,
}

/// Invert [`pack_instance`]; `None` on a short slice, never a panic.
#[must_use]
pub fn unpack_instance(words: &[u32]) -> Option<UnpackedCell> {
    if words.len() < CROWD_PACKED_WORDS {
        return None;
    }
    let packed = words[3];
    let lod = u8::try_from(packed >> 24).unwrap_or(CROWD_LOD_CULLED);
    let tint = u8::try_from((packed >> 18) & 63).unwrap_or(0);
    let palette = u8::try_from((packed >> 16) & 3).unwrap_or(0);
    let variant = u8::try_from((packed >> 13) & 7).unwrap_or(0);
    let phase = u16::try_from(packed & 0x3FF).unwrap_or(1000).min(1000);
    Some(UnpackedCell {
        pos_m: [
            f32::from_bits(words[0]),
            f32::from_bits(words[1]),
            f32::from_bits(words[2]),
        ],
        lod,
        palette,
        tint,
        variant,
        phase_permille: phase,
        excitement_permille: u16::try_from(words[4]).unwrap_or(1000).min(1000),
    })
}

/// Headless-twin GPU seam: validates the staged frame and the viewport,
/// then names the window build that binds it (never silent success). The
/// CPU staging above is the authority here; the window build records the
/// live pass instead.
#[cfg(not(feature = "gpu"))]
pub fn submit_gpu(
    instances: &[CrowdInstance],
    viewport_px: [u32; 2],
) -> Result<GpuBindRequest, CrowdInstError> {
    if instances.is_empty() {
        return Err(CrowdInstError::BadBuffer { want: 1, got: 0 });
    }
    if viewport_px[0] == 0 || viewport_px[1] == 0 {
        return Err(CrowdInstError::BadCamera("crowd viewport is zero"));
    }
    if viewport_px[0] > 2048 || viewport_px[1] > 2048 {
        return Err(CrowdInstError::BadCamera("crowd viewport past 2048"));
    }
    for (index, seat) in instances.iter().enumerate() {
        if !seat.pos_m.iter().all(|c| c.is_finite()) {
            return Err(CrowdInstError::BadInstance(index));
        }
    }
    Err(CrowdInstError::NoDevice(
        "crowd_inst: window build with a live device binds the staged frame; CPU staging is the authority",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small_layout() -> SeatLayout {
        SeatLayout {
            rows_per_sector: 2,
            cols_per_row: 3,
        }
    }

    fn open_planes() -> [[f32; 4]; 6] {
        [[0.0, 0.0, 0.0, 1.0e9]; 6]
    }

    fn shut_planes() -> [[f32; 4]; 6] {
        [[0.0, 0.0, 0.0, -1.0e9]; 6]
    }

    fn levels_full() -> [u16; CROWD_BLOCKS] {
        [600u16; CROWD_BLOCKS]
    }

    #[test]
    fn layout_for_seats_covers_and_caps() {
        let rich = SeatLayout::for_seats(20_000).expect("20k builds");
        assert_eq!((rich.rows_per_sector, rich.cols_per_row), (50, 100));
        assert_eq!(rich.seat_count(), 20_000);
        rich.validate().expect("rich layout is valid");
        let maxed = SeatLayout::for_seats(50_000).expect("cap builds");
        assert_eq!(maxed.seat_count(), 50_000);
        match SeatLayout::for_seats(0) {
            Err(CrowdInstError::BadLayout(_)) => {}
            other => panic!("zero seats must refuse, got {other:?}"),
        }
        match SeatLayout::for_seats(50_001) {
            Err(CrowdInstError::OverBudget { requested, allowed }) => {
                assert_eq!((requested, allowed), (50_001, 50_000));
            }
            other => panic!("past-cap seats must refuse, got {other:?}"),
        }
        let bad = SeatLayout {
            rows_per_sector: 0,
            cols_per_row: 3,
        };
        assert!(bad.validate().is_err(), "zero rows must refuse");
    }

    #[test]
    fn seat_pos_wraps_and_mirrors() {
        let layout = small_layout();
        let north = layout.seat_pos(0, 0, 0);
        let south = layout.seat_pos(1, 0, 0);
        assert!(
            north[2] < 0.0 && south[2] > 0.0,
            "opposite sectors face across the pit: {north:?} vs {south:?}"
        );
        assert_eq!(
            layout.seat_pos(0, 0, 0),
            layout.seat_pos(4, 2, 3),
            "indices wrap onto the same seat"
        );
        for sector in 0..4u8 {
            for row in 0..2u16 {
                for col in 0..3u16 {
                    let at = layout.seat_pos(sector, row, col);
                    assert!(at.iter().all(|c| c.is_finite()), "seat must stage: {at:?}");
                    assert!(at[1] > CROWD_STAND_WALL_H_M, "feet ride above the wall top");
                }
            }
        }
        let low = layout.seat_pos(0, 0, 1);
        let high = layout.seat_pos(0, 1, 1);
        assert!(high[1] > low[1], "upper rows sit higher");
        assert!(high[2] < low[2], "upper rows sit further back");
    }

    #[test]
    fn blocks_cover_every_seat_exactly_once() {
        let mut seen = [false; CROWD_BLOCKS];
        for sector in 0..4u8 {
            for row in 0..2u16 {
                for col in 0..3u16 {
                    let block = usize::from(block_for_seat(sector, row, col));
                    assert!(block < CROWD_BLOCKS, "block {block} escapes the room");
                    seen[block] = true;
                }
            }
        }
        assert!(seen.iter().any(|s| *s), "seats must land on blocks");
        assert_eq!(block_for_seat(0, 0, 0), block_for_seat(4, 4, 4));
    }

    #[test]
    fn lod_holds_inside_hysteresis_then_switches_once() {
        let mut prev = 0u8;
        let mut switches = 0u32;
        for frame in 0..300u32 {
            let jitter =
                (frame.wrapping_mul(2654435761).wrapping_shr(8) % 61) as f32 / 100.0 - 0.30;
            let held = pick_crowd_lod(18.0 + jitter, prev);
            assert_eq!(held, 0, "jitter inside the band must hold L0");
            prev = held;
        }
        assert_eq!(switches, 0);
        prev = 0;
        // Sweep 10 m to 40 m: crosses the L0 band (18 m + 1.5 m) exactly
        // once and stops below the L1 band (45 m - 1.5 m), so one switch.
        for step in 0..60u32 {
            let dist = 10.0 + step as f32 * (30.0 / 59.0);
            let next = pick_crowd_lod(dist, prev);
            if next != prev {
                switches += 1;
                assert_eq!((prev, next), (0, 1));
                assert!(dist > 18.0 + CROWD_LOD_HYST_M, "switched early at {dist}");
                prev = next;
            }
        }
        assert_eq!((switches, prev), (1, 1));
        assert_eq!(pick_crowd_lod(200.0, 0), 2, "far seats rest at L2");
        assert_eq!(
            pick_crowd_lod(f32::NAN, 0),
            2,
            "NaN distance rests, never panics"
        );
        assert_eq!(
            pick_crowd_lod(5.0, 200),
            0,
            "wild prev levels clamp, never panic"
        );
    }

    #[test]
    fn cull_hook_keeps_open_sectors_culls_shut_ones() {
        let layout = small_layout();
        assert_eq!(visible_sectors(&layout, &open_planes()), [true; 4]);
        assert_eq!(visible_sectors(&layout, &shut_planes()), [false; 4]);
        let mut nan = open_planes();
        nan[2] = [0.0, f32::NAN, 0.0, 1.0];
        assert_eq!(visible_sectors(&layout, &nan), [false; 4]);
        // Slab around the north stand only: z in [-40, -8] keeps sector 0
        // while the west/east bound spheres (radius ~7 m about z = 0) cull.
        let slab = [
            [1.0, 0.0, 0.0, 60.0],
            [-1.0, 0.0, 0.0, 60.0],
            [0.0, 1.0, 0.0, 60.0],
            [0.0, -1.0, 0.0, 60.0],
            [0.0, 0.0, 1.0, 40.0],
            [0.0, 0.0, -1.0, -8.0],
        ];
        assert_eq!(visible_sectors(&layout, &slab), [true, false, false, false]);
    }

    #[test]
    fn build_stages_every_seat_in_fixed_order_x2() {
        let layout = small_layout();
        let seats = layout.seat_count();
        assert_eq!(seats, 24);
        let levels = levels_full();
        let camera = [0.0, 6.0, 0.0];
        let run = || {
            let planes = open_planes();
            let mut prev = vec![0u8; seats];
            let mut out = vec![CrowdInstance::culled([0.0, 0.0, 0.0]); seats];
            let mut builder =
                CrowdBuilder::new(&layout, &levels, 41, camera, &planes, &mut prev, &mut out)
                    .expect("open frame builds");
            let stats = builder.build();
            (stats, prev, out)
        };
        let (a_stats, a_prev, a_out) = run();
        let (b_stats, b_prev, b_out) = run();
        assert_eq!(a_stats, b_stats, "same bytes stage the same frame twice");
        assert_eq!(a_prev, b_prev);
        assert_eq!(a_out, b_out);
        assert_eq!(a_stats.sectors_visible, 4);
        assert_eq!(a_stats.instances, seats as u32);
        let live: u32 = a_stats.lod_counts.iter().sum();
        assert_eq!(live, seats as u32, "every visible seat lands in a live LOD");
        for seat in &a_out {
            assert!(seat.is_visible(), "open planes keep every seat");
            assert!(seat.pos_m.iter().all(|c| c.is_finite()));
            assert!(seat.palette < 4, "palette stays in the 4-slot atlas");
            assert!(seat.phase_permille < 1000, "phase stays in permille");
        }
    }

    #[test]
    fn build_culls_shut_sectors_to_sentinels() {
        let layout = small_layout();
        let seats = layout.seat_count();
        let levels = levels_full();
        let planes = shut_planes();
        let mut prev = vec![0u8; seats];
        let mut out = vec![CrowdInstance::culled([0.0, 0.0, 0.0]); seats];
        let mut builder = CrowdBuilder::new(
            &layout,
            &levels,
            7,
            [0.0, 6.0, 0.0],
            &planes,
            &mut prev,
            &mut out,
        )
        .expect("shut frame still builds");
        let stats = builder.build();
        assert_eq!((stats.sectors_visible, stats.instances), (0, 0));
        assert_eq!(stats.lod_counts, [0, 0, 0]);
        assert!(
            out.iter().all(|s| !s.is_visible()),
            "shut seats read sentinel"
        );
        assert!(stats.digest != 0, "even a culled frame digests");
    }

    #[test]
    fn build_refuses_short_buffers_and_bad_inputs() {
        let layout = small_layout();
        let seats = layout.seat_count();
        let levels = levels_full();
        let mut prev = vec![0u8; seats - 1];
        let mut out = vec![CrowdInstance::culled([0.0, 0.0, 0.0]); seats];
        match CrowdBuilder::new(
            &layout,
            &levels,
            0,
            [0.0, 6.0, 0.0],
            &open_planes(),
            &mut prev,
            &mut out,
        ) {
            Err(CrowdInstError::BadBuffer { want, got }) => {
                assert_eq!((want, got), (seats, seats - 1));
            }
            Ok(_) => panic!("short prev buffer must refuse, it built"),
            Err(e) => panic!("short prev buffer must refuse, got {e:?}"),
        }
        let mut prev = vec![0u8; seats];
        let mut out = vec![CrowdInstance::culled([0.0, 0.0, 0.0]); seats];
        match CrowdBuilder::new(
            &layout,
            &levels,
            0,
            [f32::NAN, 0.0, 0.0],
            &open_planes(),
            &mut prev,
            &mut out,
        ) {
            Err(CrowdInstError::BadCamera(_)) => {}
            Ok(_) => panic!("NaN camera must refuse, it built"),
            Err(e) => panic!("NaN camera must refuse, got {e:?}"),
        }
        let mut bad_planes = open_planes();
        bad_planes[0][0] = f32::INFINITY;
        match CrowdBuilder::new(
            &layout,
            &levels,
            0,
            [0.0, 6.0, 0.0],
            &bad_planes,
            &mut prev,
            &mut out,
        ) {
            Err(CrowdInstError::BadPlanes(0)) => {}
            Ok(_) => panic!("non-finite plane must refuse, it built"),
            Err(e) => panic!("non-finite plane must refuse, got {e:?}"),
        }
        let bad = SeatLayout {
            rows_per_sector: 0,
            cols_per_row: 3,
        };
        match CrowdBuilder::new(
            &bad,
            &levels,
            0,
            [0.0, 6.0, 0.0],
            &open_planes(),
            &mut prev,
            &mut out,
        ) {
            Err(CrowdInstError::BadLayout(_)) => {}
            Ok(_) => panic!("bad layout must refuse, it built"),
            Err(e) => panic!("bad layout must refuse, got {e:?}"),
        }
    }

    #[test]
    fn steady_build_allocates_nothing() {
        let layout = small_layout();
        let seats = layout.seat_count();
        let levels = levels_full();
        let planes = open_planes();
        let mut prev = vec![0u8; seats];
        let mut out = vec![CrowdInstance::culled([0.0, 0.0, 0.0]); seats];
        let out_len = out.len();
        let mut builder = CrowdBuilder::new(
            &layout,
            &levels,
            3,
            [0.0, 6.0, 0.0],
            &planes,
            &mut prev,
            &mut out,
        )
        .expect("frame binds");
        let before = (builder.build(), out_len);
        let ((), probe) = engine_core::alloc_probe::measure(|| {
            builder.build();
        });
        assert_eq!(
            probe.calls, 0,
            "warm frames must not touch the heap, got {probe:?}"
        );
        assert_eq!(builder.build().digest, before.0.digest);
    }

    #[test]
    fn bob_offset_pins_exact_edges() {
        assert_eq!(bob_offset_m(0, 1000), 0.0, "phase 0 never lifts");
        assert_eq!(bob_offset_m(1000, 1000), 0.0, "phase 1000 rests");
        let top = bob_offset_m(500, 1000);
        assert!(
            (top - 0.14).abs() < 1e-6,
            "full excitement peaks at 14 cm, got {top}"
        );
        let half = bob_offset_m(250, 1000);
        assert!(
            (half - 0.07).abs() < 1e-6,
            "triangle wave climbs linearly, got {half}"
        );
        assert_eq!(
            bob_offset_m(500, 0),
            0.02,
            "quiet room still breathes at 2 cm"
        );
        assert_eq!(bob_offset_m(2500, 2500), bob_offset_m(1000, 1000));
        assert!(bob_offset_m(500, 1000) > bob_offset_m(500, 0));
    }

    #[test]
    fn pack_roundtrips_every_field() {
        let mut words = [0u32; CROWD_PACKED_WORDS];
        pack_instance([1.5, 6.25, -22.0], 1, 3, 742, 815, &mut words);
        let back = unpack_instance(&words).expect("packed words unpack");
        assert_eq!(back.pos_m, [1.5, 6.25, -22.0]);
        assert_eq!((back.lod, back.palette), (1, 3));
        assert_eq!((back.phase_permille, back.excitement_permille), (742, 815));
        assert!(
            unpack_instance(&words[..3]).is_none(),
            "short slices refuse"
        );
        let mut clamped = [0u32; CROWD_PACKED_WORDS];
        pack_instance([0.0, 0.0, 0.0], 9, 9, 5000, 5000, &mut clamped);
        let back = unpack_instance(&clamped).expect("clamped words unpack");
        assert_eq!(
            (back.palette, back.phase_permille, back.excitement_permille),
            (1, 1000, 1000)
        );
    }

    #[test]
    fn bind_contract_counts_words_exactly() {
        let req = describe_bind(20_000).expect("rich crowd binds");
        assert_eq!(
            (req.instances, req.words, req.bytes),
            (20_000, 100_000, 400_000)
        );
        assert_eq!(req.budget_ms(), CROWD_INST_BUDGET_MS);
        match describe_bind(0) {
            Err(CrowdInstError::BadBuffer { want: 1, got: 0 }) => {}
            other => panic!("zero instances must refuse, got {other:?}"),
        }
        match describe_bind(MAX_CROWD_SEATS + 1) {
            Err(CrowdInstError::OverBudget { .. }) => {}
            other => panic!("past-cap binds must refuse, got {other:?}"),
        }
    }

    #[test]
    #[cfg(not(feature = "gpu"))]
    fn gpu_submit_without_device_names_the_window_build() {
        let layout = small_layout();
        let seats = layout.seat_count();
        let levels = levels_full();
        let planes = open_planes();
        let mut prev = vec![0u8; seats];
        let mut out = vec![CrowdInstance::culled([0.0, 0.0, 0.0]); seats];
        let mut builder = CrowdBuilder::new(
            &layout,
            &levels,
            1,
            [0.0, 6.0, 0.0],
            &planes,
            &mut prev,
            &mut out,
        )
        .expect("frame binds");
        builder.build();
        match submit_gpu(&out, [1280, 720]) {
            Err(CrowdInstError::NoDevice(_)) => {}
            other => panic!("good frame must hit the twin refusal, got {other:?}"),
        }
        match submit_gpu(&[], [1280, 720]) {
            Err(CrowdInstError::BadBuffer { .. }) => {}
            other => panic!("empty frame must refuse, got {other:?}"),
        }
        match submit_gpu(&out, [0, 720]) {
            Err(CrowdInstError::BadCamera(_)) => {}
            other => panic!("zero viewport must refuse, got {other:?}"),
        }
    }

    #[test]
    fn vat_crowd_spike() {
        let layout = SeatLayout {
            rows_per_sector: 50,
            cols_per_row: 100,
        };
        assert_eq!(layout.seat_count(), 20_000);
        let camera = [0.0, 6.0, 0.0];
        let slab = [
            [1.0, 0.0, 0.0, 60.0],
            [-1.0, 0.0, 0.0, 60.0],
            [0.0, 1.0, 0.0, 60.0],
            [0.0, -1.0, 0.0, 60.0],
            [0.0, 0.0, 1.0, 40.0],
            [0.0, 0.0, -1.0, -8.0],
        ];
        for tick in [0u64, 12, 41] {
            let dec = u16::try_from(tick.saturating_mul(2)).unwrap_or(u16::MAX);
            let mut levels = [300u16; CROWD_BLOCKS];
            for slot in levels.iter_mut().take(16) {
                *slot = 700u16.saturating_sub(dec).max(300);
            }
            for slot in levels.iter_mut().skip(16) {
                *slot = 500u16.saturating_sub(dec).max(300);
            }
            let run = || {
                let planes = open_planes();
                let mut prev = vec![0u8; 20_000];
                let mut out = vec![CrowdInstance::culled([0.0, 0.0, 0.0]); 20_000];
                let mut bd =
                    CrowdBuilder::new(&layout, &levels, tick, camera, &planes, &mut prev, &mut out)
                        .expect("20k builds");
                let st = bd.build();
                (st, out)
            };
            let (a, a_out) = run();
            let (b, _) = run();
            assert_eq!(a.digest, b.digest, "tick {tick} stages x2 equal");
            assert_eq!(a.instances, 20_000);
            let req = describe_bind(usize::try_from(a.instances).unwrap_or(0)).expect("binds");
            assert_eq!(req.words, a.instances.saturating_mul(5));
            assert_eq!(a.budget_ms(), CROWD_INST_BUDGET_MS);
            let mut far = 0u32;
            let mut far_quad = 0u32;
            for s in a_out.iter() {
                let dx = s.pos_m[0] - camera[0];
                let dy = s.pos_m[1] - camera[1];
                let dz = s.pos_m[2] - camera[2];
                if (dx * dx + dy * dy + dz * dz).sqrt() > 45.0 {
                    far += 1;
                    if s.is_impostor() {
                        far_quad += 1;
                    }
                }
            }
            assert!(far > 0, "tick {tick} has far rows");
            assert_eq!(far_quad, far, "all past 45 m take the quad path");
            let mut varied = 0u32;
            for i in 0..16usize {
                let x = a_out[i];
                let y = a_out[i + 1];
                if x.tint != y.tint
                    || x.variant != y.variant
                    || x.phase_permille != y.phase_permille
                {
                    varied += 1;
                }
            }
            assert!(varied >= 15, "neighbour variety {varied}/16 at tick {tick}");
            let mut prev = vec![0u8; 20_000];
            let mut out = vec![CrowdInstance::culled([0.0, 0.0, 0.0]); 20_000];
            let mut bd =
                CrowdBuilder::new(&layout, &levels, tick, camera, &slab, &mut prev, &mut out)
                    .expect("slab builds");
            let shut = bd.build();
            assert!(shut.digest != a.digest, "slab cull moves the digest");
            assert!(shut.sectors_visible < 4, "slab culls at tick {tick}");
        }
        let layout_s = small_layout();
        let seats = layout_s.seat_count();
        let levels = levels_full();
        let planes = open_planes();
        let mut prev = vec![0u8; seats];
        let mut out = vec![CrowdInstance::culled([0.0, 0.0, 0.0]); seats];
        let mut bd = CrowdBuilder::new(&layout_s, &levels, 3, camera, &planes, &mut prev, &mut out)
            .expect("binds");
        bd.build();
        let ((), probe) = engine_core::alloc_probe::measure(|| {
            bd.build();
        });
        assert_eq!(probe.calls, 0, "warm 20k-shape stage allocates nothing");
        assert!(
            !CROWD_VAT_WGSL.is_empty(),
            "VAT shader ships with the stage"
        );
    }
}
