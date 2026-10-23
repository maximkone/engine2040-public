//! debris_lod: distance tiers for debris, merge groups and impostors.
//!
//! Consumer: the draw-list builder reads one [`LodPlan`] per frame from the
//! DX4 [`DebrisLedger`]; the ledger's own `low_lod` flag stays the hashed
//! two-way switch and equals "tier is Merged or Impostor" (see the test).
//! Data owner: `engine-destruction`. Units: integer mm, shard ids. The
//! tiers: Full near the camera, Simplified(1..) down the mesh LOD chain
//! (the CT2 quadric chain, cooked by `engine-assets::mesh_cook::
//! build_lod_chain`; the level indexes that chain), Merged past the
//! ledger's LOD distance (small pieces of one island in one grid cell draw
//! as one proxy), Impostor far out (one per island). The chain is a cook
//! artifact: this module names the level, the caller that owns the meshes
//! reads the chain. Determinism: integer math only, groups sorted by
//! `(island, cell, id)`, no hash order. Errors: [`LodError`]. Budget: one
//! pass over at most 2048 shards plus one sort; the plan allocates, the
//! ledger step does not.

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
            for v in g.centroid_mm {
                h.write(&v.to_le_bytes());
            }
        }
        for g in &self.impostors {
            h.write_u32(g.island);
            h.write_u32(g.count);
            for v in g.centroid_mm {
                h.write(&v.to_le_bytes());
            }
        }
        h.finish()
    }
}

/// One merge-band shard; the derived order is the sort order of the groups.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct BandShard {
    island: u32,
    cell: [i32; 3],
    id: u32,
    pos: [i32; 3],
    /// Index of the shard's entry in the tier list.
    slot: usize,
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
    let mut near_band: Vec<BandShard> = Vec::new();
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
            near_band.push(BandShard {
                island: shard.island,
                cell,
                id: shard.id,
                pos: shard.pos_mm,
                slot: tiers.len(),
            });
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
        let key = (near_band[at].island, near_band[at].cell);
        let end = near_band[at..]
            .iter()
            .position(|m| (m.island, m.cell) != key)
            .map_or(near_band.len(), |n| at + n);
        let lead = near_band[at..end].iter().map(|m| m.id).min().unwrap_or(0);
        let mut sum = [0i64; 3];
        for m in &near_band[at..end] {
            for (s, p) in sum.iter_mut().zip(m.pos) {
                *s += i64::from(p);
            }
        }
        let count = u32::try_from(end - at).unwrap_or(u32::MAX);
        for m in &near_band[at..end] {
            tiers[m.slot].1 = Tier::Merged(lead);
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
            for (s, p) in sum.iter_mut().zip(f.2) {
                *s += i64::from(p);
            }
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
    use engine_assets::mesh_cook::{build_lod_chain, tri_count, CookedMesh};
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

    /// One triangulated chunk like a fractured slab: 12x12 cells on the unit
    /// square, 288 triangles, +Z normals and unit UVs. The live bonded crate
    /// is boxes (12 tris); this fixture is the shape a chunk chain is for.
    fn chunk_mesh() -> CookedMesh {
        const N: u8 = 12;
        let mut mesh = CookedMesh::default();
        for j in 0..=N {
            for i in 0..=N {
                let x = f32::from(i) / f32::from(N);
                let y = f32::from(j) / f32::from(N);
                mesh.positions.push([x, y, 0.0]);
                mesh.normals.push([0.0, 0.0, 1.0]);
                mesh.uv0.push([x, y]);
                mesh.tangents.push([1.0, 0.0, 0.0, 1.0]);
            }
        }
        let stride = u32::from(N) + 1;
        for j in 0..N {
            for i in 0..N {
                let a = u32::from(j) * stride + u32::from(i);
                let b = a + 1;
                let c = a + stride;
                let d = c + 1;
                mesh.indices.extend_from_slice(&[a, b, d, a, d, c]);
            }
        }
        mesh
    }

    /// Every attribute byte of every chain level, so two cooks compare as bytes.
    fn chain_bytes(chain: &[CookedMesh]) -> Vec<u8> {
        let mut out = Vec::new();
        for mesh in chain {
            let push3 = |out: &mut Vec<u8>, values: &[[f32; 3]]| {
                for v in values {
                    for part in v {
                        out.extend_from_slice(&part.to_le_bytes());
                    }
                }
            };
            push3(&mut out, &mesh.positions);
            push3(&mut out, &mesh.normals);
            for uv in &mesh.uv0 {
                for part in uv {
                    out.extend_from_slice(&part.to_le_bytes());
                }
            }
            for t in &mesh.tangents {
                for part in t {
                    out.extend_from_slice(&part.to_le_bytes());
                }
            }
            for index in &mesh.indices {
                out.extend_from_slice(&index.to_le_bytes());
            }
        }
        out
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

    /// DX6 chain receipt: the same chunk fixture cooked through the CT2 owner
    /// (`engine-assets::mesh_cook::build_lod_chain`) twice must give the same
    /// bytes, and the levels the plan names must cut the frame's triangles.
    /// Run: cargo test -p engine-destruction --lib debris_lod::chunk_chain --
    /// --nocapture
    #[test]
    fn chunk_chain_cooks_identical_bytes_and_the_plan_uses_its_levels() {
        let chunk = chunk_mesh();
        assert_eq!(tri_count(&chunk), 288);
        let levels = usize::from(SIMPLIFY_LEVELS) + 1;
        assert_eq!(levels, 3, "the test indexes level 0, 1 and 2");
        let first = build_lod_chain(&chunk, levels).expect("chunk chain");
        let again = build_lod_chain(&chunk, levels).expect("chunk recook");
        let bytes = chain_bytes(&first);
        assert_eq!(bytes, chain_bytes(&again), "same fixture, same bytes");
        let tris: Vec<u32> = first.iter().map(tri_count).collect();
        assert_eq!(tris.len(), levels);
        assert_eq!(tris[0], 288, "level 0 is the base mesh");
        assert!(
            tris[1] < tris[0] && tris[2] < tris[1],
            "the chain drops triangles: {tris:?}"
        );
        let mut hash = Fnv64::new();
        hash.write(&bytes);
        let digest = hash.finish();

        let ledger = ledger();
        let cfg = LodConfig::from_budget(&ledger.config());
        let plan = plan_lod(&ledger, [0, 1500, 0], &cfg).expect("plans");
        let twice = plan_lod(&ledger, [0, 1500, 0], &cfg).expect("plans");
        assert_eq!(plan, twice, "the same ledger plans the same merges");
        assert_eq!(plan.digest(), twice.digest());

        // Feed every tier through the chain: Full reads level 0, Simplified
        // reads its level, a merge proxy is one box (12 tris as the window
        // bakes it) and an impostor is a card the window does not draw.
        const PROXY_TRIS: u32 = 12;
        let mut direct_tris = 0u64;
        for (_, tier) in &plan.tiers {
            match *tier {
                Tier::Full => direct_tris += u64::from(tris[0]),
                Tier::Simplified(level) => {
                    let level = usize::from(level);
                    assert!(level < tris.len(), "tier {level} names a chain level");
                    direct_tris += u64::from(tris[level]);
                }
                Tier::Merged(_) | Tier::Impostor(_) => {}
            }
        }
        let proxies = u32::try_from(plan.merged.len()).expect("fits");
        let drawn_tris = direct_tris + u64::from(proxies) * u64::from(PROXY_TRIS);
        let base_tris = 288u64 * 1000;
        assert!(drawn_tris < base_tris, "the plan must drop triangles");
        let drop_pct = (base_tris - drawn_tris) * 100 / base_tris;
        eprintln!(
            "debris chain: tris={tris:?} bytes={} digest={digest:016x} cook_x2=identical",
            bytes.len()
        );
        eprintln!(
            "debris frame: pieces=1000 draws={} tris={base_tris}->{drawn_tris} drop={drop_pct}%",
            plan.draws()
        );
    }

    /// DX6 budget receipt: one [`plan_lod`] over 1,000 debris pieces must stay
    /// under the DX4 amortized 1 ms. 201 samples after one warm plan; the gate
    /// is p50 (the amortized reading of the budget), p95 and max print above.
    /// Run: cargo test -p engine-destruction --lib
    /// debris_lod::a_thousand_pieces -- --nocapture
    #[test]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "test-only timing, never the tick"
    )]
    fn a_thousand_pieces_plan_inside_the_dx4_millisecond() {
        const SAMPLES: usize = 201;
        let ledger = ledger();
        let cfg = LodConfig::from_budget(&ledger.config());
        let warm = plan_lod(&ledger, [0, 1500, 0], &cfg).expect("plans");
        let digest = warm.digest();
        let draws = warm.draws();
        let mut ns = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let start = std::time::Instant::now();
            let plan = plan_lod(&ledger, [0, 1500, 0], &cfg).expect("plans");
            ns.push(start.elapsed().as_nanos());
            std::hint::black_box(&plan);
            assert_eq!(plan.digest(), digest, "the timed plan must repeat");
        }
        ns.sort_unstable();
        let (p50, p95, max) = (ns[SAMPLES / 2], ns[SAMPLES * 95 / 100], ns[SAMPLES - 1]);
        eprintln!(
            "debris budget: pieces=1000 draws={draws} samples={SAMPLES} \
             p50_us={} p95_us={} max_us={} bar_us=1000",
            p50 / 1000,
            p95 / 1000,
            max / 1000
        );
        assert!(
            p50 <= 1_000_000,
            "plan p50 {p50} ns over the DX4 1 ms budget (p95 {p95} ns, max {max} ns, draws {draws})"
        );
    }
}
