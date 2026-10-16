//! torch_cluster: clustered torch light assignment over screen tiles.
//!
//! Owner: render lane; consumer is the parent shadow plus cluster submit.
//! Units: pixels for centers and radii, meters for depth and range.
//! Determinism: sorted cell-major order, stable per inputs.
//! Errors: typed parent [`LightError`]; over-budget degrades, bad input fails.

use super::*;
use std::time::Instant;

// ---------------------------------------------------------------------------
// Clustered torch lights: screen-space tiles times log depth slices. The
// pass owns assignment only; the presenter owns projection (the versioned
// helper below) so this core stays std-only and headless-testable.
// ---------------------------------------------------------------------------

/// One torch point light in world units: position in meters, range in
/// meters, linear color and intensity.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TorchLight {
    /// World position in meters.
    pub pos: [f32; 3],
    /// Influence radius in meters, `> 0`.
    pub range_m: f32,
    /// Linear RGB color, components `>= 0`.
    pub color: [f32; 3],
    /// Brightness scale, `>= 0`.
    pub intensity: f32,
}

impl TorchLight {
    /// Validate finiteness and ranges before the light reaches assignment.
    pub fn validate(&self) -> Result<(), LightError> {
        if !self.pos.iter().all(|c| c.is_finite()) {
            return Err(LightError::Invalid("torch position is not finite".into()));
        }
        if !self.range_m.is_finite() || self.range_m <= 0.0 {
            return Err(LightError::Invalid(format!(
                "torch range {} is not positive finite",
                self.range_m
            )));
        }
        if !self.color.iter().all(|c| c.is_finite() && *c >= 0.0) {
            return Err(LightError::Invalid(
                "torch color is not finite non-negative".into(),
            ));
        }
        if !self.intensity.is_finite() || self.intensity < 0.0 {
            return Err(LightError::Invalid(format!(
                "torch intensity {} is not finite non-negative",
                self.intensity
            )));
        }
        Ok(())
    }
}

/// One light in cluster space: pixel center, view depth in meters and pixel
/// radius. Built by the presenter projection helpers, consumed by assign.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ScreenLight {
    /// Horizontal pixel center.
    pub x_px: f32,
    /// Vertical pixel center.
    pub y_px: f32,
    /// View depth in meters.
    pub depth_m: f32,
    /// Influence radius in pixels, `>= 0`.
    pub radius_px: f32,
}

impl ScreenLight {
    /// Validate finiteness and ranges before assignment.
    pub fn validate(&self) -> Result<(), LightError> {
        if !self.x_px.is_finite() || !self.y_px.is_finite() || !self.depth_m.is_finite() {
            return Err(LightError::Invalid(
                "screen light center is not finite".into(),
            ));
        }
        if !self.radius_px.is_finite() || self.radius_px < 0.0 {
            return Err(LightError::Invalid(format!(
                "screen light radius {} is not finite non-negative",
                self.radius_px
            )));
        }
        Ok(())
    }
}

/// Cluster grid configuration: frame size, tile edge, depth slice count,
/// view range and budgets.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ClusterConfig {
    /// Frame width in pixels, `1..=2048`.
    pub width_px: u32,
    /// Frame height in pixels, `1..=2048`.
    pub height_px: u32,
    /// Tile edge in pixels; defaults to `CLUSTER_TILE_PX`.
    pub tile_px: u32,
    /// Depth slice count, `1..=MAX_DEPTH_SLICES`.
    pub depth_slices: u32,
    /// View near in meters, `> 0`.
    pub near_m: f32,
    /// View far in meters, `> near_m`.
    pub far_m: f32,
    /// Most lights assigned per frame; above this the tail is dropped.
    pub max_lights: usize,
    /// Most lights per cluster; above this the cell clamps and overflows.
    pub max_per_cluster: usize,
}

impl ClusterConfig {
    /// Validate dims, slices, range and budgets before allocation.
    pub fn validate(&self) -> Result<(), LightError> {
        if self.width_px == 0
            || self.height_px == 0
            || self.width_px > 2048
            || self.height_px > 2048
        {
            return Err(LightError::Invalid(format!(
                "cluster frame {}x{} outside 1..=2048",
                self.width_px, self.height_px
            )));
        }
        if self.tile_px == 0 || self.tile_px > self.width_px.max(self.height_px) {
            return Err(LightError::Invalid(format!(
                "cluster tile {} is not 1..=frame",
                self.tile_px
            )));
        }
        if self.depth_slices == 0 || self.depth_slices > MAX_DEPTH_SLICES {
            return Err(LightError::Invalid(format!(
                "cluster slices {} outside 1..={MAX_DEPTH_SLICES}",
                self.depth_slices
            )));
        }
        if !self.near_m.is_finite() || !self.far_m.is_finite() {
            return Err(LightError::Invalid("cluster range is not finite".into()));
        }
        if self.near_m <= 0.0 || self.far_m <= self.near_m {
            return Err(LightError::Invalid(format!(
                "cluster range {}..{} is not 0 < near < far",
                self.near_m, self.far_m
            )));
        }
        if self.max_lights == 0 || self.max_per_cluster == 0 {
            return Err(LightError::Invalid(
                "cluster budgets must be nonzero".into(),
            ));
        }
        let cells = self.cells();
        if cells > MAX_CLUSTER_CELLS {
            return Err(LightError::BudgetExceeded {
                requested: cells as u32,
                allowed: MAX_CLUSTER_CELLS as u32,
            });
        }
        Ok(())
    }

    /// Tiles across the frame.
    pub fn tiles_x(&self) -> usize {
        self.width_px.div_ceil(self.tile_px) as usize
    }

    /// Tiles down the frame.
    pub fn tiles_y(&self) -> usize {
        self.height_px.div_ceil(self.tile_px) as usize
    }

    /// Total cluster cells: tiles times slices.
    pub fn cells(&self) -> usize {
        self.tiles_x() * self.tiles_y() * self.depth_slices as usize
    }

    /// Depth slice boundaries in meters, `slices + 1` entries from `near`
    /// to `far` with the cascade log/uniform blend. Deterministic per config.
    pub fn slice_bounds(&self) -> Vec<f32> {
        let n = self.depth_slices as usize;
        let mut bounds = Vec::with_capacity(n + 1);
        for i in 0..=n {
            if i == 0 {
                bounds.push(self.near_m);
            } else if i == n {
                bounds.push(self.far_m);
            } else {
                let t = i as f32 / n as f32;
                let log = self.near_m * (self.far_m / self.near_m).powf(t);
                let uni = self.near_m + (self.far_m - self.near_m) * t;
                bounds.push(CASCADE_LAMBDA * log + (1.0 - CASCADE_LAMBDA) * uni);
            }
        }
        bounds
    }
}

/// Clustered torch assignment: per-cell light counts plus a global index
/// list in cell-major order with prefix offsets. Overflow clamps per cell
/// (degrade, counted) instead of failing the frame.
#[derive(Clone, PartialEq, Debug)]
pub struct ClusterPass {
    /// Validated grid configuration.
    pub config: ClusterConfig,
    /// Lights per cell, length `cells()`.
    pub counts: Vec<u32>,
    /// Prefix offset per cell into `indices`.
    pub offsets: Vec<u32>,
    /// Light indices in cell-major order.
    pub indices: Vec<u32>,
    /// Rejected pushes past `max_per_cluster` (degrade count).
    pub overflow_cells: u32,
    /// Tail lights dropped past `max_lights` (degrade count).
    pub dropped_lights: u32,
    /// Degrade signal when the light count exceeded the budget.
    pub degraded: Option<LightError>,
    /// Last assign time in ms for the receipt.
    pub last_ms: f64,
}

impl ClusterPass {
    /// Build and assign: validates every light, drops the tail past
    /// `max_lights` (recording `BudgetExceeded`), then assigns the kept
    /// lights to overlapped cells. Deterministic per inputs.
    pub fn build(config: ClusterConfig, lights: &[ScreenLight]) -> Result<Self, LightError> {
        config.validate()?;
        for light in lights {
            light.validate()?;
        }
        let started = Instant::now();
        let kept = lights.len().min(config.max_lights);
        let dropped = (lights.len() - kept) as u32;
        let degraded = if dropped > 0 {
            Some(LightError::BudgetExceeded {
                requested: lights.len() as u32,
                allowed: config.max_lights as u32,
            })
        } else {
            None
        };
        let mut pass = Self {
            config,
            counts: vec![0; config.cells()],
            offsets: vec![0; config.cells()],
            indices: Vec::new(),
            overflow_cells: 0,
            dropped_lights: dropped,
            degraded,
            last_ms: 0.0,
        };
        pass.assign(&lights[..kept]);
        pass.last_ms = started.elapsed().as_secs_f64() * 1000.0;
        Ok(pass)
    }

    /// Cell index for tile `(tx, ty)` and slice `s` in tile-major order.
    pub fn cell_index(&self, tx: usize, ty: usize, s: usize) -> usize {
        (s * self.config.tiles_y() + ty) * self.config.tiles_x() + tx
    }

    fn assign(&mut self, lights: &[ScreenLight]) {
        let tx_n = self.config.tiles_x();
        let ty_n = self.config.tiles_y();
        let slices = self.config.depth_slices as usize;
        let tile = self.config.tile_px as f32;
        let bounds = self.config.slice_bounds();
        // Collect (cell, light) pairs over covered ranges only, then sort by
        // (cell, light): deterministic cell-major order at torch-scale cost
        // instead of cells-times-lights brute force.
        let mut pairs: Vec<(usize, u32)> = Vec::new();
        for (li, light) in lights.iter().enumerate() {
            // Tile range padded by one on each side: tile spans are closed,
            // so a center exactly on an edge touches both neighbors; the
            // per-tile closest-point test below filters back to the exact
            // set, matching the brute-force reference bit-for-bit.
            let x0 = ((light.x_px - light.radius_px) / tile).floor() as i32 - 1;
            let x1 = ((light.x_px + light.radius_px) / tile).floor() as i32 + 1;
            let y0 = ((light.y_px - light.radius_px) / tile).floor() as i32 - 1;
            let y1 = ((light.y_px + light.radius_px) / tile).floor() as i32 + 1;
            let tx0 = x0.clamp(0, tx_n as i32 - 1);
            let tx1 = x1.clamp(0, tx_n as i32 - 1);
            let ty0 = y0.clamp(0, ty_n as i32 - 1);
            let ty1 = y1.clamp(0, ty_n as i32 - 1);
            if tx1 < tx0 || ty1 < ty0 {
                continue;
            }
            let d_lo = light.depth_m - light.radius_px;
            let d_hi = light.depth_m + light.radius_px;
            for s in 0..slices {
                // Overlap of the light depth span with the slice span.
                if d_hi < bounds[s] || d_lo > bounds[s + 1] {
                    continue;
                }
                // Circle-vs-tile refine per tile: keeps corner tiles a
                // circle only grazes out of the list.
                for ty in ty0..=ty1 {
                    for tx in tx0..=tx1 {
                        // Closest point on the tile rect to the center.
                        let qx = light.x_px.clamp(tx as f32 * tile, tx as f32 * tile + tile);
                        let qy = light.y_px.clamp(ty as f32 * tile, ty as f32 * tile + tile);
                        let dx = light.x_px - qx;
                        let dy = light.y_px - qy;
                        if dx * dx + dy * dy <= light.radius_px * light.radius_px {
                            pairs.push((self.cell_index(tx as usize, ty as usize, s), li as u32));
                        }
                    }
                }
            }
        }
        pairs.sort_unstable();
        for (cell, light) in pairs {
            if self.counts[cell] < self.config.max_per_cluster as u32 {
                self.counts[cell] += 1;
                self.indices.push(light);
            } else {
                self.overflow_cells += 1;
            }
        }
        let mut base = 0u32;
        for c in 0..self.counts.len() {
            self.offsets[c] = base;
            base += self.counts[c];
        }
    }

    /// Light indices assigned to `cell`, in increasing light order.
    pub fn lights_in(&self, cell: usize) -> &[u32] {
        let start = self.offsets[cell] as usize;
        let end = start + self.counts[cell] as usize;
        &self.indices[start..end]
    }

    /// Receipt for the last assign.
    pub fn receipt(&self) -> PassReceipt {
        PassReceipt {
            pass: "torch_cluster",
            texels: 0,
            cells: self.config.cells() as u32,
            ms: self.last_ms,
            degraded: self.degraded.is_some() || self.overflow_cells > 0,
        }
    }

    /// Headless seam: the real bind lives in the window
    /// `submit_shadow_cluster` (needs a wgpu device), so this reports a
    /// typed error naming it instead of pretending. The CPU assignment
    /// above stays the reference. Never silent success.
    pub fn submit_gpu(&self) -> Result<PassReceipt, LightError> {
        Err(LightError::Invalid(
            "clustered torch GPU bind runs through submit_shadow_cluster on a wgpu device, not this headless seam".into(),
        ))
    }

    /// Tightened digest: FNV-1a over counts plus indices, x2 identical per inputs.
    /// Presentation only, never hashed into the tick golden.
    pub fn rt19_digest(&self) -> u64 {
        let mut h = engine_core::Fnv64::new();
        for c in &self.counts {
            h.write_u32(*c);
        }
        for ix in &self.indices {
            h.write_u32(*ix);
        }
        h.finish()
    }

    /// Tightened pre-pass build: 0.5-20 span plus early-z center reject
    /// (`depth_m > 20` never assigns). Same assign as [`Self::build`], so the
    /// tightened grid stays brute-identical to its own brute reference.
    /// Presentation CPU only, tick untouched.
    pub fn build_rt19_tightened(
        base: ClusterConfig,
        lights: &[ScreenLight],
    ) -> Result<Self, LightError> {
        let tight = base.rt19_tightened();
        let mut kept: Vec<ScreenLight> = Vec::with_capacity(lights.len());
        for light in lights {
            light.validate()?;
            if light.depth_m <= RT19_REJECT_DEPTH_M {
                kept.push(*light);
            }
        }
        Self::build(tight, &kept)
    }

    /// Temporal carry build: one representative per lit cell carried from the
    /// prior frame with combine plus confidence plus one mutation per cell.
    /// Shape follows the donor temporal reservoir (current plus reprojected
    /// prior, clamp, combine, kill guard, confidence) plus one mutation step.
    /// Reprojection is identity on one grid, the prior is the full light set
    /// on the tightened grid so far lights exercise the kill path. Weights
    /// are exact from brute sums (like the donor exact pdfs), so the shade
    /// estimate is exact and the cut counts real pairs saved. Cpu only,
    /// present only, tick untouched. Port of the temporal reservoir shape
    /// from Patryk27/strolle at 69cbcdd4 (temporal resampling file, MIT, read
    /// live per the steal row) plus the mutation idea, zero donor bytes.
    pub fn build_temporal_carry(
        base: ClusterConfig,
        lights: &[ScreenLight],
        seed: u64,
    ) -> Result<(Self, Vec<TemporalReservoir>, TemporalReport), LightError> {
        base.validate()?;
        for light in lights {
            light.validate()?;
        }
        let started = Instant::now();
        let tight_cfg = base.rt19_tightened();
        let mut kept: Vec<ScreenLight> = Vec::with_capacity(lights.len());
        let mut prior_map: Vec<Option<usize>> = Vec::with_capacity(lights.len());
        for light in lights {
            if light.depth_m <= RT19_REJECT_DEPTH_M {
                prior_map.push(Some(kept.len()));
                kept.push(*light);
            } else {
                prior_map.push(None);
            }
        }
        let keep_n = kept.len().min(base.max_lights);
        kept.truncate(keep_n);
        for slot in prior_map.iter_mut() {
            if let Some(pos) = *slot {
                if pos >= keep_n {
                    *slot = None;
                }
            }
        }
        let tight = Self::build_rt19_tightened(base, lights)?;
        let prior = Self::build(tight_cfg, lights)?;
        let cells = tight_cfg.cells();
        if tight.counts.len() != cells || prior.counts.len() != cells {
            return Err(LightError::Invalid("temporal grid shape mismatch".into()));
        }
        let (counts, offsets, indices, reservoirs, sse, hit_cells) =
            temporal_fill_cells(&tight, &prior, &kept, lights, &prior_map, seed)?;
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        let rmse = if hit_cells == 0 {
            0.0
        } else {
            let n = f64::from(hit_cells);
            (sse / n).sqrt()
        };
        let pairs_u = u32::try_from(indices.len())
            .map_err(|e| LightError::Invalid(format!("pair count too large: {e}")))?;
        let base_f = f64::from(TEMPORAL_BASELINE_PAIRS);
        let pairs_f = f64::from(pairs_u);
        let cut = 100.0 * (base_f - pairs_f) / base_f;
        let pass = Self {
            config: tight_cfg,
            counts,
            offsets,
            indices,
            overflow_cells: 0,
            dropped_lights: tight.dropped_lights,
            degraded: tight.degraded.clone(),
            last_ms: ms,
        };
        let digest = pass.rt19_digest();
        let report = TemporalReport {
            pairs: pass.indices.len(),
            rmse,
            ms,
            cut_pct: cut,
            digest,
        };
        Ok((pass, reservoirs, report))
    }
}

/// Tightened pair baseline on the seed 19 250 torch fixture.
pub const TEMPORAL_BASELINE_PAIRS: u32 = 14264;
/// Kept lights per lit cell in the temporal pass.
pub const TEMPORAL_PER_CELL: usize = 1;
/// Sample count cap per reservoir, mirrors the donor clamp.
pub const TEMPORAL_M_CLAMP: f32 = 64.0;
/// Assign budget in ms for the temporal pass, mirrors the window budget.
pub const TEMPORAL_ASSIGN_BUDGET_MS: f64 = 16.0;
/// Falloff radius squared for the shade proxy.
pub const TEMPORAL_FALLOFF_R2: f32 = 256.0;

/// One temporal reservoir per cell: representative plus weight.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TemporalReservoir {
    /// Representative light index into the kept set, max value when empty.
    pub light: u32,
    /// Weight for unbiased shade: estimate equals weight times contrib.
    pub w: f32,
    /// Sample count, clamped to the cap.
    pub m: f32,
    /// One when carried, zero when the prior slot was killed.
    pub confidence: f32,
}

/// Temporal carry numbers: pairs, error, time, cut, digest.
#[derive(Clone, PartialEq, Debug)]
pub struct TemporalReport {
    /// Stored pairs in the temporal pass.
    pub pairs: usize,
    /// Relative error vs brute shade.
    pub rmse: f64,
    /// Assign time in ms.
    pub ms: f64,
    /// Cut over the baseline in percent.
    pub cut_pct: f64,
    /// Digest of the temporal pass.
    pub digest: u64,
}

/// Cell fill output: counts, offsets, indices, reservoirs, error, hits.
type TemporalFill = (
    Vec<u32>,
    Vec<u32>,
    Vec<u32>,
    Vec<TemporalReservoir>,
    f64,
    u32,
);

// Falloff proxy for one light center vs the cell center.
fn temporal_contrib(lx: f32, ly: f32, cx: f32, cy: f32) -> f32 {
    let dx = lx - cx;
    let dy = ly - cy;
    1.0 / (1.0 + (dx * dx + dy * dy) / TEMPORAL_FALLOFF_R2)
}

// Combine factors mirroring the donor eval (defensive plus pdf weighted).
fn temporal_mis(lhs_m: f32, rhs_m: f32, lhs_pdf: f32, rhs_pdf: f32) -> (f32, f32, f32) {
    fn mis(x: f32, y: f32) -> f32 {
        let sum = x + y;
        if sum == 0.0 {
            0.0
        } else {
            x / sum
        }
    }
    fn falloff(q0: f32, q1: f32) -> f32 {
        if q0 <= 0.0 {
            1.0
        } else {
            (q1 / q0).min(1.0).powf(8.0).clamp(0.0, 1.0)
        }
    }
    let m = rhs_m * falloff(rhs_pdf, rhs_pdf).min(falloff(lhs_pdf, lhs_pdf));
    let t = mis(lhs_m, rhs_m);
    let lhs_mis = t + (1.0 - t) * mis(lhs_m * lhs_pdf, rhs_m * rhs_pdf);
    let rhs_mis = (1.0 - t) * mis(rhs_m * rhs_pdf, lhs_m * lhs_pdf);
    (lhs_mis, rhs_mis, m)
}

// Closest prior light plus its contrib and the prior sum.
fn temporal_prior_rep(
    prior_list: &[u32],
    all: &[ScreenLight],
    cx: f32,
    cy: f32,
) -> Result<Option<(usize, f32, f32)>, LightError> {
    if prior_list.is_empty() {
        return Ok(None);
    }
    let mut sum = 0.0f32;
    let mut best_c = -1.0f32;
    let mut best_idx = 0usize;
    let mut first = true;
    for picked in prior_list {
        let idx = usize::try_from(*picked)
            .map_err(|e| LightError::Invalid(format!("prior index too large: {e}")))?;
        let light = all
            .get(idx)
            .ok_or_else(|| LightError::Invalid("prior index out of range".into()))?;
        let c = temporal_contrib(light.x_px, light.y_px, cx, cy);
        sum += c;
        if first || c > best_c {
            best_c = c;
            best_idx = idx;
            first = false;
        }
    }
    Ok(Some((best_idx, best_c, sum)))
}

// One mutation: random neighbor in the brute set, keep if closer.
fn temporal_mcmc_propose(
    cur_list: &[u32],
    kept: &[ScreenLight],
    winner: usize,
    winner_c: f32,
    cx: f32,
    cy: f32,
    rng: &mut engine_core::Rng,
) -> Result<(usize, f32), LightError> {
    if cur_list.is_empty() {
        return Ok((winner, winner_c));
    }
    let len_u = u64::try_from(cur_list.len())
        .map_err(|e| LightError::Invalid(format!("cell list too large: {e}")))?;
    let draw = rng.next_u64() % len_u;
    let pos = usize::try_from(draw)
        .map_err(|e| LightError::Invalid(format!("proposal index too large: {e}")))?;
    let prop_u32 = *cur_list
        .get(pos)
        .ok_or_else(|| LightError::Invalid("proposal out of range".into()))?;
    let prop_usize = usize::try_from(prop_u32)
        .map_err(|e| LightError::Invalid(format!("proposal light too large: {e}")))?;
    let light = kept
        .get(prop_usize)
        .ok_or_else(|| LightError::Invalid("proposal light out of range".into()))?;
    let c = temporal_contrib(light.x_px, light.y_px, cx, cy);
    if c > winner_c {
        Ok((prop_usize, c))
    } else {
        Ok((winner, winner_c))
    }
}

// Fill one representative per cell with carry plus combine plus mutation.
fn temporal_fill_cells(
    tight: &ClusterPass,
    prior: &ClusterPass,
    kept: &[ScreenLight],
    all: &[ScreenLight],
    prior_map: &[Option<usize>],
    seed: u64,
) -> Result<TemporalFill, LightError> {
    let cfg = &tight.config;
    let cells = cfg.cells();
    let tx_n = cfg.tiles_x();
    let ty_n = cfg.tiles_y();
    let tile_u16 = u16::try_from(cfg.tile_px)
        .map_err(|e| LightError::Invalid(format!("tile edge too large: {e}")))?;
    let tile_f = f32::from(tile_u16);
    let mut counts = vec![0u32; cells];
    let mut offsets = vec![0u32; cells];
    let mut indices: Vec<u32> = Vec::with_capacity(cells);
    let mut reservoirs = vec![
        TemporalReservoir {
            light: u32::MAX,
            w: 0.0,
            m: 0.0,
            confidence: 0.0,
        };
        cells
    ];
    let mut sse = 0.0f64;
    let mut hit_cells = 0u32;
    for cell in 0..cells {
        let per_slice = tx_n * ty_n;
        let rem = cell % per_slice;
        let ty = rem / tx_n;
        let tx = rem % tx_n;
        let tx_u =
            u16::try_from(tx).map_err(|e| LightError::Invalid(format!("tile x too large: {e}")))?;
        let ty_u =
            u16::try_from(ty).map_err(|e| LightError::Invalid(format!("tile y too large: {e}")))?;
        let cx = f32::from(tx_u) * tile_f + tile_f / 2.0;
        let cy = f32::from(ty_u) * tile_f + tile_f / 2.0;
        let cur_list = tight.lights_in(cell);
        let prior_list = prior.lights_in(cell);
        let picked = temporal_pick_cell(&TemporalPick {
            cur_list,
            prior_list,
            kept,
            all,
            prior_map,
            cx,
            cy,
            cell,
            seed,
        })?;
        if let Some((kept_u32, res, brute_sum, estimate)) = picked {
            counts[cell] = 1;
            indices.push(kept_u32);
            reservoirs[cell] = res;
            if brute_sum > 0.0 {
                let b = f64::from(brute_sum);
                let e = f64::from(estimate);
                let rel = (e - b) / b;
                sse += rel * rel;
                hit_cells += 1;
            }
        }
    }
    let mut base_off = 0u32;
    for c in 0..cells {
        offsets[c] = base_off;
        base_off += counts[c];
    }
    Ok((counts, offsets, indices, reservoirs, sse, hit_cells))
}

// One cell's temporal-pick inputs, grouped so the picker takes a single
// argument. All slices borrow the frame's cluster grids; `cx`/`cy` are the
// cell-center pixels, `cell` its index, `seed` the frame seed.
#[derive(Clone, Copy)]
struct TemporalPick<'a> {
    cur_list: &'a [u32],
    prior_list: &'a [u32],
    kept: &'a [ScreenLight],
    all: &'a [ScreenLight],
    prior_map: &'a [Option<usize>],
    cx: f32,
    cy: f32,
    cell: usize,
    seed: u64,
}

// Combined prior side of one cell pick: the kept position when the prior
// light survives in the current list, its pdf, weight and count, and
// whether the prior died. A prior light missing from the current list
// dies instead of leaking across frames.
struct PriorRhs {
    pos: Option<usize>,
    pdf: f32,
    w: f32,
    m: f32,
    killed: bool,
}

// Fold the prior representative into the current list, dropping a prior
// light the current frame no longer holds.
fn temporal_prior_rhs(
    prior_list: &[u32],
    all: &[ScreenLight],
    prior_map: &[Option<usize>],
    cur_list: &[u32],
    cx: f32,
    cy: f32,
) -> Result<PriorRhs, LightError> {
    let prior_opt = temporal_prior_rep(prior_list, all, cx, cy)?;
    let (pos, pdf, w, m, killed) = match prior_opt {
        None => (None, 0.0, 0.0, 0.0, false),
        Some((prior_idx, prior_c, prior_sum)) => {
            let mapped = prior_map
                .get(prior_idx)
                .ok_or_else(|| LightError::Invalid("prior map out of range".into()))?;
            match *mapped {
                None => (None, 0.0, 0.0, 1.0, true),
                Some(kept_pos) => {
                    let kept_u = u32::try_from(kept_pos)
                        .map_err(|e| LightError::Invalid(format!("kept pos too large: {e}")))?;
                    if cur_list.contains(&kept_u) {
                        let w = if prior_c > 0.0 {
                            prior_sum / prior_c
                        } else {
                            0.0
                        };
                        (Some(kept_pos), prior_c, w, 1.0, false)
                    } else {
                        (None, 0.0, 0.0, 1.0, true)
                    }
                }
            }
        }
    };
    Ok(PriorRhs {
        pos,
        pdf,
        w,
        m,
        killed,
    })
}

// Pick one representative for a cell with carry plus combine plus mutation.
fn temporal_pick_cell(
    pick: &TemporalPick<'_>,
) -> Result<Option<(u32, TemporalReservoir, f32, f32)>, LightError> {
    let TemporalPick {
        cur_list,
        prior_list,
        kept,
        all,
        prior_map,
        cx,
        cy,
        cell,
        seed,
    } = *pick;
    if cur_list.is_empty() {
        return Ok(None);
    }
    let mut cur_sum = 0.0f32;
    for picked in cur_list {
        let idx = usize::try_from(*picked)
            .map_err(|e| LightError::Invalid(format!("current index too large: {e}")))?;
        let light = kept
            .get(idx)
            .ok_or_else(|| LightError::Invalid("current index out of range".into()))?;
        cur_sum += temporal_contrib(light.x_px, light.y_px, cx, cy);
    }
    let rhs = temporal_prior_rhs(prior_list, all, prior_map, cur_list, cx, cy)?;
    let PriorRhs {
        pos: rhs_pos,
        pdf: rhs_pdf,
        w: rhs_w,
        m: rhs_m,
        killed: rhs_killed,
    } = rhs;
    let cell_u = u64::try_from(cell)
        .map_err(|e| LightError::Invalid(format!("cell index too large: {e}")))?;
    let mut rng = engine_core::Rng::new(seed).fork(cell_u);
    let len_u = u64::try_from(cur_list.len())
        .map_err(|e| LightError::Invalid(format!("cell list too large: {e}")))?;
    let draw = rng.next_u64() % len_u;
    let lhs_pos = usize::try_from(draw)
        .map_err(|e| LightError::Invalid(format!("draw index too large: {e}")))?;
    let lhs_u32 = *cur_list
        .get(lhs_pos)
        .ok_or_else(|| LightError::Invalid("draw out of range".into()))?;
    let lhs_usize = usize::try_from(lhs_u32)
        .map_err(|e| LightError::Invalid(format!("draw light too large: {e}")))?;
    let lhs_light = kept
        .get(lhs_usize)
        .ok_or_else(|| LightError::Invalid("draw light out of range".into()))?;
    let lhs_pdf = temporal_contrib(lhs_light.x_px, lhs_light.y_px, cx, cy);
    let lhs_w = if lhs_pdf > 0.0 {
        cur_sum / lhs_pdf
    } else {
        0.0
    };
    let (lhs_mis, rhs_mis, mis_m) = temporal_mis(1.0, rhs_m, lhs_pdf, rhs_pdf);
    let lhs_score = lhs_mis * lhs_pdf * lhs_w;
    let rhs_score = rhs_mis * rhs_pdf * rhs_w;
    let (win_pos, win_c) = if let Some(rhs_p) = rhs_pos {
        if rhs_score > lhs_score {
            (rhs_p, rhs_pdf)
        } else {
            (lhs_usize, lhs_pdf)
        }
    } else {
        (lhs_usize, lhs_pdf)
    };
    let (mut_win, mut_c) = temporal_mcmc_propose(cur_list, kept, win_pos, win_c, cx, cy, &mut rng)?;
    let final_w = if mut_c > 0.0 { cur_sum / mut_c } else { 0.0 };
    let final_m = (1.0 + mis_m).min(TEMPORAL_M_CLAMP);
    let confidence = if rhs_killed { 0.0 } else { 1.0 };
    let win_u32 = u32::try_from(mut_win)
        .map_err(|e| LightError::Invalid(format!("winner index too large: {e}")))?;
    let res = TemporalReservoir {
        light: win_u32,
        w: final_w,
        m: final_m,
        confidence,
    };
    let estimate = final_w * mut_c;
    Ok(Some((win_u32, res, cur_sum, estimate)))
}

/// Tightened near in meters: same near as the full span.
pub const RT19_TIGHT_NEAR_M: f32 = 0.5;
/// Tightened far in meters: 0.5-20 replaces the full 0.5-50 span.
pub const RT19_TIGHT_FAR_M: f32 = 20.0;
/// Early-z reject: light centers beyond this depth never assign.
pub const RT19_REJECT_DEPTH_M: f32 = 20.0;

impl ClusterConfig {
    /// Tightened grid: same tiles plus slices, far clamped to 20 m.
    /// Near stays 0.5 m so the slice blend keeps its shape.
    pub fn rt19_tightened(&self) -> Self {
        Self {
            near_m: RT19_TIGHT_NEAR_M,
            far_m: RT19_TIGHT_FAR_M,
            ..*self
        }
    }
}

/// Project a world point into cluster space under a perspective view-projection
/// matrix: pixel center from NDC, pixel radius from `radius_m` through
/// `focal_px` (`height_px / (2 tan(fov / 2))`). Returns `None` behind the
/// camera or on a degenerate `w`. Behind `v1` with the glam camera math.
#[cfg(feature = "v1")]
pub fn project_perspective(
    world: [f32; 3],
    view_proj: glam::Mat4,
    width_px: u32,
    height_px: u32,
    focal_px: f32,
    radius_m: f32,
) -> Option<ScreenLight> {
    if width_px == 0 || height_px == 0 {
        return None;
    }
    if !focal_px.is_finite() || focal_px <= 0.0 || !radius_m.is_finite() || radius_m < 0.0 {
        return None;
    }
    let clip = view_proj * glam::Vec4::new(world[0], world[1], world[2], 1.0);
    if !clip.w.is_finite() || clip.w <= 0.0 {
        return None;
    }
    let ndc = [clip.x / clip.w, clip.y / clip.w];
    if !ndc.iter().all(|c| c.is_finite()) {
        return None;
    }
    let depth = clip.w;
    Some(ScreenLight {
        x_px: (ndc[0] * 0.5 + 0.5) * width_px as f32,
        y_px: (0.5 - ndc[1] * 0.5) * height_px as f32,
        depth_m: depth,
        radius_px: radius_m * focal_px / depth,
    })
}
