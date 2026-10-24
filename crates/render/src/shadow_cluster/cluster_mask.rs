//! cluster_mask: the per-cell torch light masks the forward torch loop reads.
//!
//! Owner: render lane. The assignment rule lives once, in
//! [`torch_cluster::ClusterPass`](super::ClusterPass); this module only
//! reshapes its compact cell-major list into the mask words
//! `shaders/cluster.wgsl` indexes. The shader never re-derives a cell, so the
//! two cannot drift.
//!
//! One 32-bit word covers 32 lights, so a frame of `max_lights` lights needs
//! `ceil(max_lights / 32)` words per cell, in light order: word `g` bit `b` is
//! light `g * 32 + b`. That is the group shape Filament uses for its froxel
//! light records, and it keeps `MAX_TORCH_LIGHTS` (256) inside eight words
//! (four bytes per cell per 32 lights) instead of a list per cell.
//!
//! Units: bits and bytes. Determinism: cell-major integer order, no clock in
//! the bytes; the same assignment gives the same masks twice. Presentation
//! only, never hashed. Errors are typed `LightError`.
//!
//! Reimplemented shape, zero donor bytes vendored:
//! PORT(google/filament@479ee3548410f21c038e50f1fe9335fe09eb433b:filament/src/Froxelizer.cpp): light-group bitmask per froxel record.

use super::*;

/// Bits in one mask word: light index `0..31` inside one group.
pub const CLUSTER_MASK_LIGHTS: u32 = 32;

/// Per-cell mask words for a frame of `max_lights` lights: at least one.
#[must_use]
pub fn mask_groups(max_lights: usize) -> u32 {
    let bits = u64::try_from(max_lights).unwrap_or(u64::MAX);
    let groups = bits.div_ceil(u64::from(CLUSTER_MASK_LIGHTS));
    u32::try_from(groups).unwrap_or(u32::MAX)
}

/// Per-cell torch light masks plus the counts the frame report prints.
#[derive(Clone, PartialEq, Debug)]
pub struct ClusterMask {
    /// Grid the masks belong to.
    pub config: ClusterConfig,
    /// Mask words, length `config.cells() * groups`. Word
    /// `cell * groups + g` bit `b` set means light `g * 32 + b` reaches the
    /// cell. Word order is cell-major, so the buffer is one storage array.
    pub masks: Vec<u32>,
    /// Words per cell: `mask_groups(config.max_lights)`.
    pub groups: u32,
    /// Cells with at least one light (the frame report's coverage line).
    pub lit_cells: u32,
    /// Most lights in one cell, `<= max_per_cluster`.
    pub max_per_cell: u32,
}

impl ClusterMask {
    /// Pack a finished assignment: one mask group of words per cell.
    ///
    /// Refuses with [`LightError::Invalid`] when the pass holds a light index
    /// past `config.max_lights`, which no built pass does: nothing is silently
    /// dropped here.
    pub fn pack(pass: &ClusterPass) -> Result<Self, LightError> {
        let groups = mask_groups(pass.config.max_lights);
        let cells = pass.config.cells();
        let words = cells.saturating_mul(usize::try_from(groups).unwrap_or(0));
        let mut masks = vec![0u32; words];
        let mut lit_cells = 0u32;
        let mut max_per_cell = 0u32;
        for cell in 0..cells {
            let list = pass.lights_in(cell);
            let mut bits = 0u32;
            for id in list {
                if usize::try_from(*id).unwrap_or(usize::MAX) >= pass.config.max_lights {
                    return Err(LightError::Invalid(format!(
                        "torch cluster cell {cell}: light {id} is past the \
                         {} lights the mask groups cover",
                        pass.config.max_lights
                    )));
                }
                let group = id / CLUSTER_MASK_LIGHTS;
                let bit = id % CLUSTER_MASK_LIGHTS;
                let at = cell * usize::try_from(groups).unwrap_or(0)
                    + usize::try_from(group).unwrap_or(0);
                masks[at] |= 1u32 << bit;
                bits += 1;
            }
            if bits > 0 {
                lit_cells += 1;
            }
            max_per_cell = max_per_cell.max(bits);
            // Lights only ever land in a group the frame has words for.
            debug_assert_eq!(pass.counts[cell], bits);
        }
        Ok(Self {
            config: pass.config,
            masks,
            groups,
            lit_cells,
            max_per_cell,
        })
    }

    /// Mask word `group` of `cell`; 0 for a cell or group outside the table.
    #[must_use]
    pub fn word(&self, cell: usize, group: usize) -> u32 {
        let at = cell
            .saturating_mul(usize::try_from(self.groups).unwrap_or(0))
            .saturating_add(group);
        self.masks.get(at).copied().unwrap_or(0)
    }

    /// Set bits of `cell` as light indices, in increasing order.
    pub fn lights_in(&self, cell: usize) -> impl Iterator<Item = u32> + '_ {
        (0..self.groups).flat_map(move |group| {
            let word = self.word(cell, usize::try_from(group).unwrap_or(usize::MAX));
            (0..CLUSTER_MASK_LIGHTS)
                .filter(move |bit| word & (1u32 << *bit) != 0)
                .map(move |bit| group * CLUSTER_MASK_LIGHTS + bit)
        })
    }

    /// Device bytes for the mask buffer: four per word.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        u64::try_from(self.masks.len())
            .unwrap_or(u64::MAX)
            .saturating_mul(4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ClusterConfig {
        ClusterConfig {
            width_px: 320,
            height_px: 180,
            tile_px: CLUSTER_TILE_PX,
            depth_slices: 8,
            near_m: 0.5,
            far_m: 50.0,
            max_lights: MAX_TORCH_LIGHTS,
            max_per_cluster: MAX_LIGHTS_PER_CLUSTER,
        }
    }

    /// Seeded unit value in `0..1` without a numeric cast or an RNG dep.
    fn unit(seed: &mut u64) -> f32 {
        *seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let top = u16::try_from(*seed >> 48).unwrap_or(0);
        f32::from(top) / f32::from(u16::MAX)
    }

    /// The many-light fixture: 25 near large, 25 far large, 200 small, the
    /// shape the 250-torch arena audit uses.
    fn seeded_lights() -> Vec<ScreenLight> {
        let mut seed = 2040u64;
        let mut out = Vec::with_capacity(250);
        for i in 0..250 {
            let (radius_px, depth_m) = match i {
                0..=24 => (60.0, 10.0),
                25..=49 => (60.0, 35.0),
                _ => (8.0, 10.0),
            };
            out.push(ScreenLight {
                x_px: unit(&mut seed) * 320.0,
                y_px: unit(&mut seed) * 180.0,
                depth_m,
                radius_px,
            });
        }
        out
    }

    /// Every cell's mask is exactly the assignment's own light list.
    #[test]
    fn mask_matches_assign() {
        let pass = ClusterPass::build(config(), &seeded_lights()).expect("assign builds");
        let mask = ClusterMask::pack(&pass).expect("mask packs");
        assert_eq!(mask.groups, mask_groups(MAX_TORCH_LIGHTS));
        assert_eq!(
            mask.masks.len(),
            pass.config.cells() * usize::try_from(mask.groups).unwrap_or(0)
        );
        for cell in 0..pass.config.cells() {
            let want: Vec<u32> = pass.lights_in(cell).to_vec();
            let got: Vec<u32> = mask.lights_in(cell).collect();
            assert_eq!(got, want, "cell {cell}");
            let group_words = usize::try_from(mask.groups).unwrap_or(0);
            let bits: u32 = mask.masks[cell * group_words..(cell + 1) * group_words]
                .iter()
                .map(|word| word.count_ones())
                .sum();
            assert_eq!(bits, pass.counts[cell], "cell {cell} bit count");
            assert!(got.iter().all(|id| *id < 32 * mask.groups));
        }
        assert!(mask.lit_cells > 0, "the fixture lights something");
    }

    /// A cell can hold lights from two groups: nothing truncates at 32.
    #[test]
    fn mask_spans_groups() {
        let wide = ClusterConfig {
            max_per_cluster: 40,
            ..config()
        };
        let lights: Vec<ScreenLight> = (0..40)
            .map(|_| ScreenLight {
                x_px: 160.0,
                y_px: 90.0,
                depth_m: 10.0,
                radius_px: 40.0,
            })
            .collect();
        let pass = ClusterPass::build(wide, &lights).expect("assign builds");
        let mask = ClusterMask::pack(&pass).expect("mask packs");
        let busy = (0..pass.config.cells())
            .find(|cell| pass.lights_in(*cell).len() > 32)
            .expect("40 co-located lights fill one cell past 32");
        let want: Vec<u32> = pass.lights_in(busy).to_vec();
        let got: Vec<u32> = mask.lights_in(busy).collect();
        assert_eq!(got, want, "cell {busy} keeps every light across groups");
        assert_ne!(mask.word(busy, 0), 0, "group 0 holds bits");
        assert_ne!(mask.word(busy, 1), 0, "group 1 holds the overflow bits");
    }

    /// The receipt numbers come from the masks, not from a second count.
    #[test]
    fn mask_stats_follow_the_cells() {
        let pass = ClusterPass::build(config(), &seeded_lights()).expect("assign builds");
        let mask = ClusterMask::pack(&pass).expect("mask packs");
        let lit = mask
            .masks
            .chunks(usize::try_from(mask.groups).unwrap_or(1))
            .filter(|w| w.iter().any(|x| *x != 0))
            .count();
        let max = mask
            .masks
            .chunks(usize::try_from(mask.groups).unwrap_or(1))
            .map(|w| w.iter().map(|x| x.count_ones()).sum::<u32>())
            .max()
            .unwrap_or(0);
        assert_eq!(u32::try_from(lit).unwrap_or(0), mask.lit_cells);
        assert_eq!(max, mask.max_per_cell);
        assert_eq!(mask.max_per_cell, *pass.counts.iter().max().unwrap_or(&0));
        assert_eq!(
            mask.bytes(),
            u64::try_from(mask.masks.len()).unwrap_or(0) * 4
        );
        let again = ClusterMask::pack(&pass).expect("packs twice");
        assert_eq!(mask.masks, again.masks, "same assignment, same masks");
    }

    /// The shader's default group count is the CPU table's: a `max_lights`
    /// change that forgets the pipeline override is caught here, headless,
    /// instead of shading the wrong cell.
    #[test]
    fn shader_default_groups_match_the_table() {
        let src = include_str!("../shaders/cluster.wgsl");
        let line = src
            .lines()
            .find(|line| line.trim_start().starts_with("override LIGHT_GROUPS"))
            .expect("cluster.wgsl declares the LIGHT_GROUPS override");
        let value = line
            .split('=')
            .nth(1)
            .and_then(|tail| tail.trim().trim_end_matches(';').strip_suffix('u'))
            .and_then(|digits| digits.trim().parse::<u32>().ok())
            .expect("the override default is a u32 literal");
        assert_eq!(value, mask_groups(MAX_TORCH_LIGHTS));
    }
}
