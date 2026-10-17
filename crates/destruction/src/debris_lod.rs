//! debris_lod: distance tiers for debris, merge groups and impostors.
//!
//! Consumer: the draw-list builder reads one [`LodPlan`] per frame from the
//! DX4 [`DebrisLedger`]; the ledger's own `low_lod` flag stays the hashed
//! two-way switch and equals "tier is Merged or Impostor" (see the test).
//! Data owner: `engine-destruction`. Units: integer mm, shard ids. The
//! tiers: Full near the camera, Simplified(1..) down the mesh LOD chain
//! (the CT2 quadric chain picks the mesh by that level), Merged past the
//! ledger's LOD distance (small pieces of one island in one grid cell draw
//! as one proxy), Impostor far out (one per island). Determinism: integer
//! math only, groups sorted by `(island, cell, id)`, no hash order. Errors:
//! [`LodError`]. Budget: one pass over at most 2048 shards plus one sort;
//! the plan allocates, the ledger step does not.

use crate::budget::{DebrisBudget, DebrisLedger, ShardView, MAX_POS_MM};
use engine_core::Fnv64;

/// Deepest mesh LOD level a Simplified tier names.
pub const SIMPLIFY_LEVELS: u8 = 2;

/// Distance bands in mm; each is the radius at which the next tier starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LodConfig {
    /// Simplified level 1 starts here.
    pub simplify_mm: i32,
    /// Simplified level 2 starts here; at the ledger's LOD distance the
    /// tier turns Merged.
    pub level2_mm: i32,
    /// Merged starts here (the ledger's `low_lod` distance).
    pub merge_mm: i32,
    /// Impostor starts here.
    pub impostor_mm: i32,
    /// Merge grid cell edge; pieces of one island in one cell merge.
    pub cell_mm: i32,
}

impl LodConfig {
    /// Bands derived from a ledger budget: merge at its LOD distance,
    /// simplify at a quarter and then half of it, impostors at triple.
    #[must_use]
    pub fn from_budget(budget: &DebrisBudget) -> Self {
        let d = budget.lod_distance_mm;
        Self {
            simplify_mm: d / 4,
            level2_mm: d / 2,
            merge_mm: d,
            impostor_mm: d.saturating_mul(3),
            cell_mm: (d / 4).max(1),
        }
    }

    /// Range-checks the bands: positive, strictly rising, cell positive.
    pub fn validate(&self) -> Result<(), LodError> {
        let rising = 0 < self.simplify_mm
            && self.simplify_mm < self.level2_mm
            && self.level2_mm < self.merge_mm
            && self.merge_mm < self.impostor_mm;
        if rising && self.cell_mm > 0 && self.impostor_mm <= MAX_POS_MM {
            Ok(())
        } else {
            Err(LodError::BadConfig)
        }
    }
}

/// Why a plan was refused; input never panics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LodError {
    /// The bands are not positive and strictly rising.
    BadConfig,
}

impl core::fmt::Display for LodError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            LodError::BadConfig => write!(f, "debris LOD bands must be positive and rising"),
        }
    }
}

impl std::error::Error for LodError {}

/// How one shard draws this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// Full mesh.
    Full,
    /// Mesh LOD level `1..=SIMPLIFY_LEVELS` of the chain.
    Simplified(u8),
    /// Drawn by the merge group led by this shard id.
    Merged(u32),
    /// Drawn by the impostor of this island.
    Impostor(u32),
}

/// One merged proxy: pieces of one island in one cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MergeGroup {
    /// Lowest shard id of the group; names the proxy.
    pub lead: u32,
    /// Island of every member.
    pub island: u32,
    /// Member count, at least one.
    pub count: u32,
    /// Integer mean position of the members; mm.
    pub centroid_mm: [i32; 3],
}

/// One far impostor: every far piece of one island.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Impostor {
    /// Island drawn as one card.
    pub island: u32,
    /// Member count, at least one.
    pub count: u32,
    /// Integer mean position of the members; mm.
    pub centroid_mm: [i32; 3],
}

/// The frame's tiers and proxies. Tiers follow the ledger's canonical
/// `(island, id)` order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LodPlan {
    /// `(shard id, tier)` for every shard in the ledger.
    pub tiers: Vec<(u32, Tier)>,
    /// Merge proxies in `(island, cell, lead)` order.
    pub merged: Vec<MergeGroup>,
    /// Impostors in island order.
    pub impostors: Vec<Impostor>,
}

impl LodPlan {
    /// Draw items this frame: Full and Simplified shards plus the proxies.
    #[must_use]
    pub fn draws(&self) -> usize {
        let direct = self
            .tiers
            .iter()
            .filter(|(_, t)| matches!(t, Tier::Full | Tier::Simplified(_)))
            .count();
        direct + self.merged.len() + self.impostors.len()
    }

    /// Digest of the plan; equal inputs digest equal twice.
    #[must_use]
    pub fn digest(&self) -> u64 {
        let mut h = Fnv64::new();
        for (id, tier) in &self.tiers {
            h.write_u32(*id);
            let (tag, arg) = match *tier {
                Tier::Full => (0u8, 0),
                Tier::Simplified(l) => (1, u32::from(l)),
                Tier::Merged(lead) => (2, lead),
                Tier::Impostor(island) => (3, island),
            };
            h.write(&[tag]);
            h.write_u32(arg);
        }
        for g in &self.merged {
            h.write_u32(g.lead);
            h.write_u32(g.island);
            h.write_u32(g.count);
            g.centroid_mm.iter().for_each(|v| h.write(&v.to_le_bytes()));
        }
        for g in &self.impostors {
            h.write_u32(g.island);
            h.write_u32(g.count);
            g.centroid_mm.iter().for_each(|v| h.write(&v.to_le_bytes()));
        }
        h.finish()
    }
}

/// Squared distance of a shard to the camera; mm^2.
fn dist_sq(shard: &ShardView, camera: [i32; 3]) -> i64 {
    (0..3)
        .map(|a| {
            let d = i64::from(shard.pos_mm[a]) - i64::from(camera[a]);
            d * d
        })
        .sum()
}

/// Integer mean of positions.
fn centroid(sum: [i64; 3], count: u32) -> [i32; 3] {
    sum.map(|s| i32::try_from(s / i64::from(count.max(1))).unwrap_or(0))
}

/// Plans the frame's LOD for every shard of `ledger` seen from `camera_mm`.
pub fn plan_lod(
    ledger: &DebrisLedger,
    camera_mm: [i32; 3],
    cfg: &LodConfig,
) -> Result<LodPlan, LodError> {
    cfg.validate()?;
    let sq = |mm: i32| i64::from(mm) * i64::from(mm);
    let camera = camera_mm.map(|v| v.clamp(-MAX_POS_MM, MAX_POS_MM));
    let mut tiers = Vec::with_capacity(ledger.len());
    // (island, cell, id, pos, tier slot) of every merge-band shard, sorted below.
    let mut near_band: Vec<(u32, [i32; 3], u32, [i32; 3], usize)> = Vec::new();
    let mut far: Vec<(u32, u32, [i32; 3])> = Vec::new();
    for i in 0..ledger.len() {
        let Some(shard) = ledger.shard_at(i) else {
            continue;
        };
        let d = dist_sq(&shard, camera);
        let tier = if d >= sq(cfg.impostor_mm) {
            far.push((shard.island, shard.id, shard.pos_mm));
            Tier::Impostor(shard.island)
        } else if d > sq(cfg.merge_mm) {
            let cell = shard.pos_mm.map(|p| p.div_euclid(cfg.cell_mm));
            near_band.push((shard.island, cell, shard.id, shard.pos_mm, tiers.len()));
            Tier::Merged(shard.id)
        } else if d >= sq(cfg.level2_mm) {
            Tier::Simplified(2)
        } else if d >= sq(cfg.simplify_mm) {
            Tier::Simplified(1)
        } else {
            Tier::Full
        };
        tiers.push((shard.id, tier));
    }
    near_band.sort_unstable();
    let mut merged = Vec::new();
    let mut at = 0;
    while at < near_band.len() {
        let key = (near_band[at].0, near_band[at].1);
        let end = near_band[at..]
            .iter()
            .position(|m| (m.0, m.1) != key)
            .map_or(near_band.len(), |n| at + n);
        let lead = near_band[at..end].iter().map(|m| m.2).min().unwrap_or(0);
        let mut sum = [0i64; 3];
        for m in &near_band[at..end] {
            (0..3).for_each(|a| sum[a] += i64::from(m.3[a]));
        }
        let count = u32::try_from(end - at).unwrap_or(u32::MAX);
        for m in &near_band[at..end] {
            tiers[m.4].1 = Tier::Merged(lead);
        }
        merged.push(MergeGroup {
            lead,
            island: key.0,
            count,
            centroid_mm: centroid(sum, count),
        });
        at = end;
    }
    far.sort_unstable();
    let mut impostors: Vec<Impostor> = Vec::new();
    let mut at = 0;
    while at < far.len() {
        let island = far[at].0;
        let end = far[at..]
            .iter()
            .position(|f| f.0 != island)
            .map_or(far.len(), |n| at + n);
        let mut sum = [0i64; 3];
        for f in &far[at..end] {
            (0..3).for_each(|a| sum[a] += i64::from(f.2[a]));
        }
        let count = u32::try_from(end - at).unwrap_or(u32::MAX);
        impostors.push(Impostor {
            island,
            count,
            centroid_mm: centroid(sum, count),
        });
        at = end;
    }
    Ok(LodPlan {
        tiers,
        merged,
        impostors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::{arena_default_budget, ShardSeed};
    use engine_core::Rng;

    /// 1000 pieces in 100 islands scattered over 30 m around the origin.
    fn ledger() -> DebrisLedger {
        let mut cfg = arena_default_budget();
        cfg.per_event_cap = 10;
        let mut ledger = DebrisLedger::new(&cfg).expect("ledger");
        let mut rng = Rng::new(21).fork_named("debris-lod");
        for island in 0..100u32 {
            let seeds: Vec<ShardSeed> = (0..10u32)
                .map(|k| {
                    let mut p = [0i32; 3];
                    for v in &mut p {
                        *v = i32::try_from(rng.next_range(0, 30_000)).expect("fits") - 15_000;
                    }
                    p[1] = p[1].abs();
                    ShardSeed {
                        id: island * 10 + k,
                        island,
                        pos_mm: p,
                        vel_mm_per_tick: [0, 0, 0],
                    }
                })
                .collect();
            ledger.admit(island, &seeds).expect("admit");
        }
        ledger
    }

    #[test]
    fn every_shard_has_one_tier_and_the_draw_count_drops() {
        let ledger = ledger();
        assert_eq!(ledger.len(), 1000);
        let cfg = LodConfig::from_budget(&ledger.config());
        let plan = plan_lod(&ledger, [0, 1500, 0], &cfg).expect("plans");
        assert_eq!(plan.tiers.len(), 1000);
        let proxied: u32 = plan.merged.iter().map(|g| g.count).sum::<u32>()
            + plan.impostors.iter().map(|g| g.count).sum::<u32>();
        let direct = plan.draws() - plan.merged.len() - plan.impostors.len();
        assert_eq!(direct + proxied as usize, 1000);
        assert!(!plan.merged.is_empty() && !plan.impostors.is_empty());
        assert!(plan.draws() < 800, "{} draws for 1000 pieces", plan.draws());
        assert!(plan.merged.iter().all(|g| g.count >= 1));
        assert!(plan
            .tiers
            .iter()
            .any(|(_, t)| matches!(t, Tier::Simplified(2))));
        assert!(plan.tiers.iter().any(|(_, t)| *t == Tier::Full));
    }

    #[test]
    fn plans_repeat_and_follow_the_camera() {
        let ledger = ledger();
        let cfg = LodConfig::from_budget(&ledger.config());
        let a = plan_lod(&ledger, [0, 1500, 0], &cfg).expect("plans");
        assert_eq!(a, plan_lod(&ledger, [0, 1500, 0], &cfg).expect("plans"));
        assert_eq!(
            a.digest(),
            plan_lod(&ledger, [0, 1500, 0], &cfg)
                .expect("plans")
                .digest()
        );
        let moved = plan_lod(&ledger, [9000, 1500, -9000], &cfg).expect("plans");
        assert_ne!(a.digest(), moved.digest());
    }

    #[test]
    fn merged_and_impostor_tiers_are_the_ledgers_low_lod_flag() {
        let mut ledger = ledger();
        let camera = [3000, 1500, -2000];
        ledger.step(camera);
        let cfg = LodConfig::from_budget(&ledger.config());
        let plan = plan_lod(&ledger, camera, &cfg).expect("plans");
        for (i, (id, tier)) in plan.tiers.iter().enumerate() {
            let view = ledger.shard_at(i).expect("shard");
            assert_eq!(view.id, *id);
            assert_eq!(
                view.low_lod,
                matches!(tier, Tier::Merged(_) | Tier::Impostor(_)),
                "shard {id}"
            );
        }
    }

    #[test]
    fn merge_groups_hold_one_island_and_name_their_lowest_id() {
        let ledger = ledger();
        let cfg = LodConfig::from_budget(&ledger.config());
        let plan = plan_lod(&ledger, [0, 0, 0], &cfg).expect("plans");
        for g in &plan.merged {
            let members: Vec<u32> = plan
                .tiers
                .iter()
                .filter(|(_, t)| *t == Tier::Merged(g.lead))
                .map(|(id, _)| *id)
                .collect();
            assert_eq!(members.len(), g.count as usize);
            assert_eq!(members.iter().min(), Some(&g.lead));
            assert!(members.iter().all(|id| id / 10 == g.island));
        }
    }

    #[test]
    fn bad_bands_refuse() {
        let ledger = ledger();
        let good = LodConfig::from_budget(&ledger.config());
        for bad in [
            LodConfig {
                simplify_mm: 0,
                ..good
            },
            LodConfig {
                level2_mm: good.simplify_mm,
                ..good
            },
            LodConfig {
                impostor_mm: good.merge_mm,
                ..good
            },
            LodConfig { cell_mm: 0, ..good },
        ] {
            assert_eq!(
                plan_lod(&ledger, [0; 3], &bad).unwrap_err(),
                LodError::BadConfig
            );
        }
    }

    #[test]
    #[ignore = "wall-clock budget: run alone with --ignored on a quiet box"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "test-only timing, never the tick"
    )]
    fn a_thousand_pieces_plan_inside_the_dx4_millisecond() {
        let ledger = ledger();
        let cfg = LodConfig::from_budget(&ledger.config());
        let mut worst = std::time::Duration::ZERO;
        for _ in 0..100 {
            let start = std::time::Instant::now();
            let plan = plan_lod(&ledger, [0, 1500, 0], &cfg).expect("plans");
            worst = worst.max(start.elapsed());
            assert_eq!(plan.tiers.len(), 1000);
        }
        assert!(worst.as_micros() <= 1000, "plan {worst:?} over 1 ms");
    }
}
