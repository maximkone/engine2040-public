//! MCP10 EXPLAIN for AI edits: dry-run cost report plus caps, refused pre-apply.
//! Owner: endpoint crate, pure cost math over the editor command types.
//! Units: objects, subtree nodes, bytes, nav tiles; integer math only.
//! Determinism: hash sets count only (never iterated), saturating counts,
//! no clock or thread.
//! Errors: typed [`ExplainRefusal`]; every refusal names its invariant plus
//! one recovery, and converts to [`McpToolError`] without losing either.
//! Callers: `tx_apply` in `mcp.rs` and `scene.place` in `scene_tools.rs` call
//! [`check_batch`] after batch build, ahead of the apply. `tx_preview` reaches
//! it through `refuse_preview` in `tool_capabilities.rs`, which also hosts the
//! cost step of `refuse_write`. A refusal writes nothing. No new MCP tool: the
//! eighteen-tool discovery budget stays untouched.

use std::collections::HashSet;

use engine_combat::cut::saturate_q16;
use engine_editor::commands::{Batch, Document, Op};
use engine_scene::ids::ObjectId;

use super::mcp::McpToolError;

/// Default cap: most objects one AI batch may touch.
pub const EXPLAIN_MAX_OBJECTS_DEFAULT: u32 = 32;
/// Hard ceiling for the objects cap; larger caps are refused as `bad_caps`.
pub const EXPLAIN_MAX_OBJECTS_HARD: u32 = 1024;
/// Default cap: most dirty-subtree nodes one AI batch may cover.
pub const EXPLAIN_MAX_DIRTY_DEFAULT: u32 = 128;
/// Hard ceiling for the dirty-subtree cap.
pub const EXPLAIN_MAX_DIRTY_HARD: u32 = 100_000;
/// Default cap: most journal bytes one AI batch may stage.
pub const EXPLAIN_MAX_BYTES_DEFAULT: u32 = 16_384;
/// Hard ceiling for the bytes cap.
pub const EXPLAIN_MAX_BYTES_HARD: u32 = 1_000_000;
/// Default cap: most nav tiles one AI batch may dirty.
pub const EXPLAIN_MAX_NAV_TILES_DEFAULT: u32 = 64;
/// Hard ceiling for the nav-tiles cap.
pub const EXPLAIN_MAX_NAV_TILES_HARD: u32 = 4_096;

/// Fixed per-batch journal overhead in the byte model (revision, key, stamp).
pub const EXPLAIN_BATCH_OVERHEAD_BYTES: u64 = 32;
/// Per-op byte costs in the model: id (16) + parent (16) + transform (40).
pub const EXPLAIN_INSERT_BYTES: u64 = 72;
/// `SetLocal`: id (16) + transform (40).
pub const EXPLAIN_SET_LOCAL_BYTES: u64 = 56;
/// `Remove`: id only.
pub const EXPLAIN_REMOVE_BYTES: u64 = 16;
/// `Reparent`: id (16) + new parent (16).
pub const EXPLAIN_REPARENT_BYTES: u64 = 32;
/// `SetMaterial`: id (16) + worst-case name (64); the model overestimates so
/// a passing batch cannot surprise the journal.
pub const EXPLAIN_SET_MATERIAL_BYTES: u64 = 80;
/// `SetComponent` / `RemoveComponent`: id (16) + kind tag (1) + worst-case
/// canonical bytes (two 64-byte names plus a few fields, under 160).
pub const EXPLAIN_COMPONENT_BYTES: u64 = 176;
/// Nav tiles dirtied by a reparent: the world pose is kept by construction,
/// so only the hierarchy edge counts, conservatively one tile.
pub const EXPLAIN_REPARENT_NAV_TILES: u64 = 1;
/// Footprint edge of one staged actor in cm (the CAP1 unit-cube convention).
pub const EXPLAIN_FOOTPRINT_CM: i32 = 100;

/// Caps for one AI batch, in cost units. The consumer passes these per call
/// so tighter sessions (evals, untrusted clients) can lower them; values
/// above the hard ceilings or zero are refused with the legal ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExplainCaps {
    /// Most distinct objects (op ids plus linked parents) per batch.
    pub max_objects: u32,
    /// Most dirty-subtree nodes (union of subtrees under op ids) per batch.
    pub max_dirty_subtrees: u32,
    /// Most estimated journal bytes per batch.
    pub max_bytes: u32,
    /// Most dirtied nav tiles per batch.
    pub max_nav_tiles: u32,
}

impl ExplainCaps {
    /// Default caps: admit single moves and small prop batches, refuse
    /// whole-scene rewrites (see the 20-brief test below).
    pub fn default_caps() -> Self {
        Self {
            max_objects: EXPLAIN_MAX_OBJECTS_DEFAULT,
            max_dirty_subtrees: EXPLAIN_MAX_DIRTY_DEFAULT,
            max_bytes: EXPLAIN_MAX_BYTES_DEFAULT,
            max_nav_tiles: EXPLAIN_MAX_NAV_TILES_DEFAULT,
        }
    }

    /// Legal ranges for caps, for refusal messages.
    pub fn legal_ranges() -> String {
        format!(
            "objects 1..={EXPLAIN_MAX_OBJECTS_HARD}, dirty_subtrees 1..={EXPLAIN_MAX_DIRTY_HARD}, \
             bytes 1..={EXPLAIN_MAX_BYTES_HARD}, nav_tiles 1..={EXPLAIN_MAX_NAV_TILES_HARD}"
        )
    }

    /// One-line cap values, attached to every refusal so the caller sees the
    /// legal values without a second round trip.
    pub fn summary(&self) -> String {
        format!(
            "caps objects<={} dirty_subtrees<={} bytes<={} nav_tiles<={}",
            self.max_objects, self.max_dirty_subtrees, self.max_bytes, self.max_nav_tiles
        )
    }

    fn check(&self) -> Result<(), ExplainRefusal> {
        for (metric, value, hard) in [
            ("max_objects", self.max_objects, EXPLAIN_MAX_OBJECTS_HARD),
            (
                "max_dirty_subtrees",
                self.max_dirty_subtrees,
                EXPLAIN_MAX_DIRTY_HARD,
            ),
            ("max_bytes", self.max_bytes, EXPLAIN_MAX_BYTES_HARD),
            (
                "max_nav_tiles",
                self.max_nav_tiles,
                EXPLAIN_MAX_NAV_TILES_HARD,
            ),
        ] {
            if value == 0 || value > hard {
                return Err(ExplainRefusal {
                    metric: "caps",
                    value,
                    bound: hard,
                    invariant: format!("{metric} {value} is outside 1..={hard}"),
                    recovery: format!(
                        "submit caps within {} (defaults: {})",
                        Self::legal_ranges(),
                        Self::default_caps().summary()
                    ),
                });
            }
        }
        Ok(())
    }
}

impl Default for ExplainCaps {
    fn default() -> Self {
        Self::default_caps()
    }
}

/// Dry-run cost of one batch. Every field is a conservative upper bound:
/// EXPLAIN overestimates, never underestimates, so a passing batch cannot
/// surprise the journal, the index rebuild or the nav bake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CostReport {
    /// Ops in the batch.
    pub ops: u32,
    /// Distinct objects touched (op ids plus non-nil linked parents).
    pub objects: u32,
    /// Union of pre-apply subtrees rooted at the op ids; an id missing from
    /// the document (a fresh insert) contributes exactly itself.
    pub dirty_subtrees: u32,
    /// Estimated journal bytes under the per-op byte model.
    pub bytes: u32,
    /// Dirtied nav tiles: one 1 m footprint move straddles at most
    /// [`tiles_per_move`] tiles; reparents keep their world pose (1 tile).
    pub nav_tiles: u32,
}

/// What stopped a batch before apply. Every refusal carries the violated
/// invariant plus exactly one recovery (the CEGIS shape: the violating
/// metric is the counterexample, the recovery is the repair direction).
/// A refusal is pure: the document, journal, capability and ledger are
/// untouched, because [`check_batch`] only reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplainRefusal {
    /// Which dimension breached: `caps`, `objects`, `dirty_subtrees`,
    /// `bytes` or `nav_tiles`.
    pub metric: &'static str,
    /// Measured value in metric units.
    pub value: u32,
    /// The breached bound in the same units.
    pub bound: u32,
    /// Violated invariant, e.g. `objects 300 > max_objects 32`.
    pub invariant: String,
    /// One concrete recovery, e.g. how to split the batch.
    pub recovery: String,
}

impl ExplainRefusal {
    /// Machine code for this refusal.
    pub fn code(&self) -> &'static str {
        match self.metric {
            "caps" => "bad_caps",
            _ => "explain_over_budget",
        }
    }

    /// Refusal as an MCP tool error. The message always carries the
    /// invariant, the legal caps and the one recovery.
    pub fn to_tool_error(&self, caps: &ExplainCaps) -> McpToolError {
        McpToolError::new(
            self.code(),
            format!(
                "{}: {}; {}; recovery: {}",
                self.code(),
                self.invariant,
                caps.summary(),
                self.recovery
            ),
        )
    }

    fn over_budget(metric: &'static str, value: u32, bound: u32, recovery: String) -> Self {
        Self {
            metric,
            value,
            bound,
            invariant: format!("{metric} {value} exceeds cap {bound}"),
            recovery,
        }
    }
}

impl std::fmt::Display for ExplainRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: {}; recovery: {}",
            self.code(),
            self.invariant,
            self.recovery
        )
    }
}

impl std::error::Error for ExplainRefusal {}

/// Tiles one 1 m footprint move can straddle, from the real nav tile size
/// (never a mirrored constant): a 100 cm segment crosses at most
/// `100 / TILE_CM + 1` tile lines per axis. Falls back to 4 (the 1 m-tile
/// bound) when the tile size is degenerate, never dividing by zero.
fn tiles_per_move() -> u64 {
    let tile = engine_spatial::navcheck::TILE_CM;
    if tile <= 0 {
        return 4;
    }
    let per_axis = EXPLAIN_FOOTPRINT_CM / tile + 1;
    (per_axis as u64) * (per_axis as u64)
}

/// Union size of the pre-apply subtrees rooted at `roots`. Missing ids (fresh
/// inserts) contribute exactly themselves; visited ids dedupe the union, so
/// overlapping subtrees count once. One walk, every node visited at most once.
fn dirty_union(doc: &Document, roots: &[ObjectId]) -> u64 {
    let mut seen: HashSet<ObjectId> = HashSet::new();
    let mut stack: Vec<ObjectId> = roots.to_vec();
    while let Some(id) = stack.pop() {
        if seen.insert(id) {
            stack.extend(doc.hierarchy.children(id));
        }
    }
    seen.len() as u64
}

/// Cheap op scan: counts, touched ids, byte and nav estimates. No hierarchy
/// walk, so [`check_batch`] can fail fast on these dimensions before paying
/// for the dirty-subtree walk (the statement-timeout shape: hostile batches
/// refuse cheap, only plausible batches pay for the walk).
struct OpScan {
    ops: u64,
    touched: HashSet<ObjectId>,
    primary: Vec<ObjectId>,
    bytes: u64,
    nav_tiles: u64,
}

fn scan_ops(batch: &Batch) -> OpScan {
    let mut scan = OpScan {
        ops: batch.ops.len() as u64,
        touched: HashSet::with_capacity(batch.ops.len().saturating_mul(2)),
        primary: Vec::with_capacity(batch.ops.len()),
        bytes: EXPLAIN_BATCH_OVERHEAD_BYTES,
        nav_tiles: 0,
    };
    let move_tiles = tiles_per_move();
    for op in &batch.ops {
        match *op {
            Op::Insert { id, parent, .. } => {
                scan.touched.insert(id);
                if parent != ObjectId::NIL {
                    scan.touched.insert(parent);
                }
                scan.primary.push(id);
                scan.bytes = scan.bytes.saturating_add(EXPLAIN_INSERT_BYTES);
                scan.nav_tiles = scan.nav_tiles.saturating_add(move_tiles);
            }
            Op::Remove { id } => {
                scan.touched.insert(id);
                scan.primary.push(id);
                scan.bytes = scan.bytes.saturating_add(EXPLAIN_REMOVE_BYTES);
                scan.nav_tiles = scan.nav_tiles.saturating_add(move_tiles);
            }
            Op::SetLocal { id, .. } => {
                scan.touched.insert(id);
                scan.primary.push(id);
                scan.bytes = scan.bytes.saturating_add(EXPLAIN_SET_LOCAL_BYTES);
                scan.nav_tiles = scan.nav_tiles.saturating_add(move_tiles);
            }
            Op::Reparent { id, new_parent } => {
                scan.touched.insert(id);
                if new_parent != ObjectId::NIL {
                    scan.touched.insert(new_parent);
                }
                scan.primary.push(id);
                scan.bytes = scan.bytes.saturating_add(EXPLAIN_REPARENT_BYTES);
                scan.nav_tiles = scan.nav_tiles.saturating_add(EXPLAIN_REPARENT_NAV_TILES);
            }
            Op::SetMaterial { id, .. } => {
                scan.touched.insert(id);
                scan.primary.push(id);
                scan.bytes = scan.bytes.saturating_add(EXPLAIN_SET_MATERIAL_BYTES);
                // Material binds move nothing: no nav tiles dirtied.
            }
            Op::SetComponent { id, .. } | Op::RemoveComponent { id, .. } => {
                scan.touched.insert(id);
                scan.primary.push(id);
                scan.bytes = scan.bytes.saturating_add(EXPLAIN_COMPONENT_BYTES);
                // Components move nothing in the nav grid at preview time.
            }
        }
    }
    scan
}

/// Dry-run cost of one batch over a document. Pure read: no locks, no
/// threads, no clock, integer math only, so `play.run` and every read-only
/// tool never stall on it.
pub fn explain_batch(doc: &Document, batch: &Batch) -> CostReport {
    let scan = scan_ops(batch);
    CostReport {
        ops: saturate_q16(scan.ops),
        objects: saturate_q16(scan.touched.len() as u64),
        dirty_subtrees: saturate_q16(dirty_union(doc, &scan.primary)),
        bytes: saturate_q16(scan.bytes),
        nav_tiles: saturate_q16(scan.nav_tiles),
    }
}
/// EXPLAIN gate: validate the caps, fail fast on the cheap dimensions
/// (objects, bytes, nav need no hierarchy walk), then pay for the
/// dirty-subtree walk only when those pass. `Ok` carries the cost report
/// the caller may log; `Err` carries the violated invariant plus one
/// recovery and writes nothing.
pub fn check_batch(
    doc: &Document,
    batch: &Batch,
    caps: &ExplainCaps,
) -> Result<CostReport, ExplainRefusal> {
    caps.check()?;
    let scan = scan_ops(batch);
    let objects = saturate_q16(scan.touched.len() as u64);
    let bytes = saturate_q16(scan.bytes);
    let nav_tiles = saturate_q16(scan.nav_tiles);
    if objects > caps.max_objects {
        return Err(ExplainRefusal::over_budget(
            "objects",
            objects,
            caps.max_objects,
            format!(
                "split the batch into chunks of at most {} objects \
                 (this batch touches {}); group moves by room, inserts by parent",
                caps.max_objects, objects
            ),
        ));
    }
    if bytes > caps.max_bytes {
        return Err(ExplainRefusal::over_budget(
            "bytes",
            bytes,
            caps.max_bytes,
            format!(
                "submit at most {} bytes per batch (this batch estimates {}); \
                 split inserts from moves and send fewer ops per batch",
                caps.max_bytes, bytes
            ),
        ));
    }
    if nav_tiles > caps.max_nav_tiles {
        return Err(ExplainRefusal::over_budget(
            "nav_tiles",
            nav_tiles,
            caps.max_nav_tiles,
            format!(
                "keep the footprint within {} nav tiles (this batch dirties {}); \
                 stage the edit one region at a time",
                caps.max_nav_tiles, nav_tiles
            ),
        ));
    }
    let dirty_subtrees = saturate_q16(dirty_union(doc, &scan.primary));
    if dirty_subtrees > caps.max_dirty_subtrees {
        return Err(ExplainRefusal::over_budget(
            "dirty_subtrees",
            dirty_subtrees,
            caps.max_dirty_subtrees,
            format!(
                "retarget the edit at smaller subtrees (this batch dirties {} nodes, \
                 cap {}); move leaves instead of roots, or split one root per batch",
                dirty_subtrees, caps.max_dirty_subtrees
            ),
        ));
    }
    Ok(CostReport {
        ops: saturate_q16(scan.ops),
        objects,
        dirty_subtrees,
        bytes,
        nav_tiles,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_editor::capability::Capability;
    use engine_editor::commands::{self, Journal};
    use engine_scene::transform::Transform;

    fn pose_at(x: f32, y: f32, z: f32) -> Transform {
        Transform::new([x, y, z], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0])
    }

    fn insert(
        doc: &mut Document,
        journal: &mut Journal,
        id: ObjectId,
        parent: ObjectId,
        pos: [f32; 3],
    ) {
        let batch = Batch {
            ops: vec![Op::Insert {
                id,
                parent,
                local: pose_at(pos[0], pos[1], pos[2]),
            }],
            base_revision: doc.revision,
            idempotency_key: doc.revision + 1_000,
            provenance: None,
        };
        commands::apply(doc, journal, &batch).expect("seed insert applies");
    }

    /// Ten spread roots plus one bushy root, so removals and moves have real
    /// subtrees to dirty.
    struct Seed {
        doc: Document,
        journal: Journal,
        roots: Vec<ObjectId>,
        bushy: ObjectId,
    }

    fn seed() -> Seed {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let mut roots = Vec::new();
        for i in 0..10 {
            let id = ObjectId::new();
            insert(
                &mut doc,
                &mut journal,
                id,
                ObjectId::NIL,
                [i as f32 * 10.0, 0.0, 0.0],
            );
            roots.push(id);
        }
        let bushy = ObjectId::new();
        insert(
            &mut doc,
            &mut journal,
            bushy,
            ObjectId::NIL,
            [0.0, 0.0, 50.0],
        );
        for i in 0..150 {
            insert(
                &mut doc,
                &mut journal,
                ObjectId::new(),
                bushy,
                [i as f32, 0.0, 50.0],
            );
        }
        Seed {
            doc,
            journal,
            roots,
            bushy,
        }
    }

    fn batch(ops: Vec<Op>) -> Batch {
        Batch {
            ops,
            base_revision: 0,
            idempotency_key: 9_999,
            provenance: None,
        }
    }

    fn fresh_ids(n: usize) -> Vec<ObjectId> {
        (0..n).map(|_| ObjectId::new()).collect()
    }

    #[test]
    fn single_setlocal_passes_with_exact_cost() {
        let seed = seed();
        let target = seed.roots[0];
        let before = seed.doc.revision;
        let report = check_batch(
            &seed.doc,
            &batch(vec![Op::SetLocal {
                id: target,
                local: pose_at(1.0, 0.0, 0.0),
            }]),
            &ExplainCaps::default_caps(),
        )
        .expect("one legal move passes");
        assert_eq!(
            report,
            CostReport {
                ops: 1,
                objects: 1,
                dirty_subtrees: 1,
                bytes: (EXPLAIN_BATCH_OVERHEAD_BYTES + EXPLAIN_SET_LOCAL_BYTES) as u32,
                nav_tiles: tiles_per_move() as u32,
            }
        );
        assert_eq!(seed.doc.revision, before, "EXPLAIN stages nothing");
    }

    #[test]
    fn insert_under_parent_counts_both_objects_but_one_dirty_node() {
        let seed = seed();
        let fresh = ObjectId::new();
        let parent = seed.roots[1];
        let report = explain_batch(
            &seed.doc,
            &batch(vec![Op::Insert {
                id: fresh,
                parent,
                local: pose_at(0.0, 0.0, 0.0),
            }]),
        );
        assert_eq!(report.objects, 2, "fresh id plus its linked parent");
        assert_eq!(
            report.dirty_subtrees, 1,
            "missing id contributes itself only"
        );
        assert_eq!(
            report.bytes,
            (EXPLAIN_BATCH_OVERHEAD_BYTES + EXPLAIN_INSERT_BYTES) as u32
        );
        assert_eq!(report.nav_tiles, tiles_per_move() as u32);
    }

    #[test]
    fn reparent_keeps_world_pose_so_costs_one_nav_tile() {
        let seed = seed();
        let report = explain_batch(
            &seed.doc,
            &batch(vec![Op::Reparent {
                id: seed.roots[2],
                new_parent: seed.roots[3],
            }]),
        );
        assert_eq!(report.objects, 2);
        assert_eq!(report.dirty_subtrees, 1, "leaf subtree is just itself");
        assert_eq!(
            report.bytes,
            (EXPLAIN_BATCH_OVERHEAD_BYTES + EXPLAIN_REPARENT_BYTES) as u32
        );
        assert_eq!(report.nav_tiles, EXPLAIN_REPARENT_NAV_TILES as u32);
    }

    #[test]
    fn remove_bushy_root_dirties_its_whole_subtree() {
        let seed = seed();
        let report = explain_batch(&seed.doc, &batch(vec![Op::Remove { id: seed.bushy }]));
        assert_eq!(report.objects, 1);
        assert_eq!(report.dirty_subtrees, 151, "root plus its 150 children");
        assert_eq!(
            report.bytes,
            (EXPLAIN_BATCH_OVERHEAD_BYTES + EXPLAIN_REMOVE_BYTES) as u32
        );
        let refused = check_batch(
            &seed.doc,
            &batch(vec![Op::Remove { id: seed.bushy }]),
            &ExplainCaps::default_caps(),
        )
        .expect_err("151 dirty nodes exceed the 128 cap");
        assert_eq!(refused.code(), "explain_over_budget");
        assert_eq!(refused.metric, "dirty_subtrees");
        assert!(refused.invariant.contains("dirty_subtrees"));
        assert!(refused.invariant.contains("151"));
        assert!(!refused.recovery.is_empty());
        assert!(format!("{refused}").contains("recovery:"));
    }

    #[test]
    fn caps_boundary_exact_cap_passes_one_over_refused() {
        let seed = seed();
        let caps = ExplainCaps {
            max_objects: 1,
            ..ExplainCaps::default_caps()
        };
        check_batch(
            &seed.doc,
            &batch(vec![Op::SetLocal {
                id: seed.roots[0],
                local: pose_at(1.0, 0.0, 0.0),
            }]),
            &caps,
        )
        .expect("exactly at cap passes");
        let refused = check_batch(
            &seed.doc,
            &batch(vec![
                Op::SetLocal {
                    id: seed.roots[0],
                    local: pose_at(1.0, 0.0, 0.0),
                },
                Op::SetLocal {
                    id: seed.roots[1],
                    local: pose_at(2.0, 0.0, 0.0),
                },
            ]),
            &caps,
        )
        .expect_err("one over cap refuses");
        assert_eq!(refused.metric, "objects");
        assert_eq!((refused.value, refused.bound), (2, 1));
        assert!(refused.recovery.contains('1'));
    }

    #[test]
    fn bad_caps_name_the_legal_ranges() {
        let seed = seed();
        let tiny = batch(vec![Op::SetLocal {
            id: seed.roots[0],
            local: pose_at(1.0, 0.0, 0.0),
        }]);
        for caps in [
            ExplainCaps {
                max_objects: 0,
                ..ExplainCaps::default_caps()
            },
            ExplainCaps {
                max_bytes: EXPLAIN_MAX_BYTES_HARD + 1,
                ..ExplainCaps::default_caps()
            },
            ExplainCaps {
                max_nav_tiles: 0,
                ..ExplainCaps::default_caps()
            },
            ExplainCaps {
                max_dirty_subtrees: u32::MAX,
                ..ExplainCaps::default_caps()
            },
        ] {
            let refused = check_batch(&seed.doc, &tiny, &caps).expect_err("bad caps refuse");
            assert_eq!(refused.code(), "bad_caps");
            assert_eq!(refused.metric, "caps");
            assert!(
                refused.recovery.contains("1..="),
                "got: {}",
                refused.recovery
            );
            let tool = refused.to_tool_error(&ExplainCaps::default_caps());
            assert_eq!(tool.code, "bad_caps");
            assert!(tool.message.contains(&refused.invariant));
            assert!(tool.message.contains("recovery:"));
            assert!(tool.message.contains("caps objects<="));
        }
    }

    #[test]
    fn every_dimension_refuses_with_invariant_and_recovery() {
        let seed = seed();
        let ids = fresh_ids(40);
        let many_inserts = batch(
            ids.iter()
                .map(|id| Op::Insert {
                    id: *id,
                    parent: ObjectId::NIL,
                    local: pose_at(0.0, 0.0, 0.0),
                })
                .collect(),
        );
        // 40 objects over the 32 cap: objects fires first (fixed order).
        let refused = check_batch(&seed.doc, &many_inserts, &ExplainCaps::default_caps())
            .expect_err("40 inserts exceed objects");
        assert_eq!(refused.metric, "objects");

        // Dirty-only breach: roomy object/byte/nav caps, tight dirty cap.
        let roomy = ExplainCaps {
            max_objects: 1024,
            max_dirty_subtrees: 10,
            max_bytes: 1_000_000,
            max_nav_tiles: 4096,
        };
        let refused = check_batch(
            &seed.doc,
            &batch(vec![Op::Remove { id: seed.bushy }]),
            &roomy,
        )
        .expect_err("bushy remove exceeds dirty");
        assert_eq!(refused.metric, "dirty_subtrees");

        // Bytes-only breach.
        let tight_bytes = ExplainCaps {
            max_objects: 1024,
            max_dirty_subtrees: 100_000,
            max_bytes: 100,
            max_nav_tiles: 4096,
        };
        let refused = check_batch(&seed.doc, &many_inserts, &tight_bytes)
            .expect_err("40 inserts exceed 100 bytes");
        assert_eq!(refused.metric, "bytes");

        // Nav-only breach.
        let tight_nav = ExplainCaps {
            max_objects: 1024,
            max_dirty_subtrees: 100_000,
            max_bytes: 1_000_000,
            max_nav_tiles: 1,
        };
        let refused = check_batch(
            &seed.doc,
            &batch(vec![Op::SetLocal {
                id: seed.roots[0],
                local: pose_at(1.0, 0.0, 0.0),
            }]),
            &tight_nav,
        )
        .expect_err("one move exceeds 1 nav tile");
        assert_eq!(refused.metric, "nav_tiles");

        for refused in [
            check_batch(&seed.doc, &many_inserts, &ExplainCaps::default_caps())
                .expect_err("objects"),
            check_batch(
                &seed.doc,
                &batch(vec![Op::Remove { id: seed.bushy }]),
                &roomy,
            )
            .expect_err("dirty"),
        ] {
            assert!(!refused.invariant.is_empty());
            assert!(!refused.recovery.is_empty());
            let tool = refused.to_tool_error(&ExplainCaps::default_caps());
            assert_eq!(tool.code, "explain_over_budget");
            assert!(tool.message.contains(&refused.invariant));
            assert!(tool.message.contains(&refused.recovery));
        }
    }

    #[test]
    fn explain_is_deterministic_and_order_invariant() {
        let seed = seed();
        let ops = vec![
            Op::SetLocal {
                id: seed.roots[0],
                local: pose_at(1.0, 0.0, 0.0),
            },
            Op::Reparent {
                id: seed.roots[2],
                new_parent: seed.roots[3],
            },
            Op::Remove { id: seed.roots[4] },
        ];
        let forward = explain_batch(&seed.doc, &batch(ops.clone()));
        let mut reversed = ops.clone();
        reversed.reverse();
        let backward = explain_batch(&seed.doc, &batch(reversed));
        assert_eq!(forward, backward, "counts never depend on op order");
        assert_eq!(forward, explain_batch(&seed.doc, &batch(ops)));
    }

    #[test]
    fn empty_batch_costs_overhead_only_and_passes_cost() {
        let seed = seed();
        let report = check_batch(&seed.doc, &batch(vec![]), &ExplainCaps::default_caps())
            .expect("empty batch has no cost to refuse");
        assert_eq!(report.ops, 0);
        assert_eq!(report.objects, 0);
        assert_eq!(report.dirty_subtrees, 0);
        assert_eq!(report.bytes, EXPLAIN_BATCH_OVERHEAD_BYTES as u32);
        assert_eq!(report.nav_tiles, 0);
        // Shape still belongs to `tx.preview`: it refuses `empty_batch`.
    }

    /// MCP10 acceptance: 20 adversarial briefs, at least 80% refused
    /// pre-apply. Every check is a pure read: revision and journal never move.
    #[test]
    fn adversarial_briefs_refuse_at_least_sixteen_of_twenty_pre_apply() {
        let seed = seed();
        let caps = ExplainCaps::default_caps();
        let revision = seed.doc.revision;
        let journal_len = seed.journal.past_len();

        let setlocals = |ids: &[ObjectId]| {
            batch(
                ids.iter()
                    .map(|id| Op::SetLocal {
                        id: *id,
                        local: pose_at(1.0, 0.0, 1.0),
                    })
                    .collect(),
            )
        };
        let inserts = |n: usize| {
            batch(
                fresh_ids(n)
                    .iter()
                    .map(|id| Op::Insert {
                        id: *id,
                        parent: ObjectId::NIL,
                        local: pose_at(0.0, 0.0, 0.0),
                    })
                    .collect(),
            )
        };
        let chain = |n: usize| {
            let ids = fresh_ids(n);
            let mut ops = Vec::with_capacity(n);
            let mut parent = ObjectId::NIL;
            for id in ids {
                ops.push(Op::Insert {
                    id,
                    parent,
                    local: pose_at(0.0, 0.0, 0.0),
                });
                parent = id;
            }
            batch(ops)
        };

        // 161 live nodes: 10 spread roots plus the 151-node bushy tree.
        let mut all: Vec<ObjectId> = seed.roots.clone();
        all.push(seed.bushy);
        let leaves: Vec<ObjectId> = {
            let mut out = Vec::new();
            let mut stack = vec![seed.bushy];
            while let Some(id) = stack.pop() {
                for child in seed.doc.hierarchy.children(id) {
                    out.push(child);
                    stack.push(child);
                }
            }
            out
        };

        let briefs: Vec<Batch> = vec![
            inserts(300),                               // 1: objects 300
            inserts(100),                               // 2: objects 100
            setlocals(&leaves[..40]),                   // 3: 40 distinct leaves
            batch(vec![Op::Remove { id: seed.bushy }]), // 4: dirty 151
            setlocals(&all.repeat(20)[..161]),          // 5: whole scene moves
            batch(
                // 6: 200 inserts
                fresh_ids(200)
                    .iter()
                    .map(|id| Op::Insert {
                        id: *id,
                        parent: ObjectId::NIL,
                        local: pose_at(0.0, 0.0, 0.0),
                    })
                    .collect(),
            ),
            batch(
                // 7: 100-op reparent storm
                leaves[..100]
                    .iter()
                    .map(|id| Op::Reparent {
                        id: *id,
                        new_parent: seed.roots[0],
                    })
                    .collect(),
            ),
            inserts(150),              // 8: objects 150
            setlocals(&leaves[..130]), // 9: 130 distinct leaves
            inserts(40),               // 10: objects 40
            batch(
                // 11: 100 mixed ops, wide touch
                leaves[..50]
                    .iter()
                    .map(|id| Op::SetLocal {
                        id: *id,
                        local: pose_at(3.0, 0.0, 3.0),
                    })
                    .chain(fresh_ids(50).iter().map(|id| Op::Insert {
                        id: *id,
                        parent: ObjectId::NIL,
                        local: pose_at(0.0, 0.0, 0.0),
                    }))
                    .collect(),
            ),
            batch(
                // 12: reparent 50 leaves
                leaves[50..100]
                    .iter()
                    .map(|id| Op::Reparent {
                        id: *id,
                        new_parent: seed.roots[5],
                    })
                    .collect(),
            ),
            chain(35),                // 13: 35-deep insert chain
            setlocals(&leaves[..33]), // 14: just over the 32 cap
            inserts(200),             // 15: bytes hammer (also objects)
            batch(vec![
                Op::Remove { id: seed.roots[0] },
                Op::Remove { id: seed.bushy },
            ]), // 16: dirty 152
            inserts(60),              // 17: objects 60
            batch(vec![Op::SetLocal {
                // 18: PASS, one tuning nudge
                id: seed.roots[0],
                local: pose_at(1.0, 0.0, 0.0),
            }]),
            inserts(5), // 19: PASS, small prop batch
            batch(vec![
                // 20: PASS, 10-op mixed small edit
                Op::SetLocal {
                    id: seed.roots[0],
                    local: pose_at(1.0, 0.0, 0.0),
                },
                Op::SetLocal {
                    id: seed.roots[1],
                    local: pose_at(2.0, 0.0, 0.0),
                },
                Op::SetLocal {
                    id: seed.roots[2],
                    local: pose_at(3.0, 0.0, 0.0),
                },
                Op::Insert {
                    id: ObjectId::new(),
                    parent: seed.roots[0],
                    local: pose_at(0.0, 0.0, 1.0),
                },
                Op::Insert {
                    id: ObjectId::new(),
                    parent: seed.roots[1],
                    local: pose_at(0.0, 0.0, 2.0),
                },
                Op::Insert {
                    id: ObjectId::new(),
                    parent: ObjectId::NIL,
                    local: pose_at(9.0, 0.0, 9.0),
                },
                Op::Insert {
                    id: ObjectId::new(),
                    parent: ObjectId::NIL,
                    local: pose_at(9.0, 0.0, 8.0),
                },
                Op::Insert {
                    id: ObjectId::new(),
                    parent: ObjectId::NIL,
                    local: pose_at(8.0, 0.0, 9.0),
                },
                Op::Reparent {
                    id: seed.roots[7],
                    new_parent: seed.roots[8],
                },
                Op::Remove { id: seed.roots[9] },
            ]),
        ];
        assert_eq!(briefs.len(), 20);

        // Seventeen hostile briefs plus three legal ones: the gate must refuse
        // at least sixteen of twenty (80%) before anything applies.
        let mut refused = 0;
        for (index, brief) in briefs.iter().enumerate() {
            match check_batch(&seed.doc, brief, &caps) {
                Ok(_) => {
                    assert!(
                        index >= 17,
                        "brief {index} passed cost but is hostile by shape"
                    );
                }
                Err(refusal) => {
                    refused += 1;
                    assert!(
                        !refusal.invariant.is_empty(),
                        "brief {index} names its invariant"
                    );
                    assert!(
                        !refusal.recovery.is_empty(),
                        "brief {index} offers one recovery"
                    );
                    let tool = refusal.to_tool_error(&caps);
                    assert!(tool.message.contains(&refusal.invariant), "brief {index}");
                    assert!(tool.message.contains("recovery:"), "brief {index}");
                }
            }
        }
        assert!(
            refused >= 16,
            "at least 16 of 20 hostile briefs refuse pre-apply, got {refused}"
        );
        assert_eq!(seed.doc.revision, revision, "refusals stage nothing");
        assert_eq!(seed.journal.past_len(), journal_len, "journal never moves");
    }

    /// MCP10 acceptance: EXPLAIN never stalls `play.run`. A cost check runs
    /// first (pure read, no locks), then a full start/poll/poll lifecycle
    /// completes with its deterministic checkpoints.
    #[test]
    fn explain_never_stalls_play_run() {
        use super::super::mcp::McpServer;
        use super::super::play::{PlayRunArgs, PlayRunTool};
        use rmcp::handler::server::router::tool::SyncTool;

        let seed = seed();
        // EXPLAIN runs before any play job exists: pure read, no server lock.
        check_batch(
            &seed.doc,
            &batch(vec![Op::SetLocal {
                id: seed.roots[0],
                local: pose_at(1.0, 0.0, 0.0),
            }]),
            &ExplainCaps::default_caps(),
        )
        .expect("legal batch passes EXPLAIN");

        let server =
            McpServer::with_document(seed.doc, seed.journal, 7, Capability::unrestricted(7));
        let started = PlayRunTool::invoke(
            &server,
            PlayRunArgs {
                action: "start".to_owned(),
                seed: Some(2040),
                ticks: Some(32),
                fighters: Some(2),
                half_extent_m: Some(20.0),
                inputs: Some(Vec::new()),
                ttl_polls: None,
                job_id: None,
                scene_rev: None,
            },
        )
        .expect("play.run starts around an EXPLAIN check");
        let id = started.job_id;
        let poll = |action: &str| {
            PlayRunTool::invoke(
                &server,
                PlayRunArgs {
                    action: action.to_owned(),
                    seed: None,
                    ticks: None,
                    fighters: None,
                    half_extent_m: None,
                    inputs: None,
                    ttl_polls: None,
                    job_id: Some(id),
                    scene_rev: None,
                },
            )
            .expect("play.run polls around an EXPLAIN check")
        };
        assert_eq!(poll("poll").status, "running");
        let done = poll("poll");
        assert_eq!(done.status, "completed");
        assert_eq!(done.checkpoints.len(), 32);
        assert!(done.final_hash.is_some());
    }

    /// Perf probe (timing, runs under `--perf`/ignored only): the full-walk
    /// `explain_batch` over a 256-op batch on a 500-node scene, against the
    /// `validate` it precedes. Prints both timings for the record; asserts
    /// only a generous bound, never a ratio, so loaded machines stay green.
    /// The gate path is `check_batch`, which fails fast on objects/bytes/nav
    /// without the walk; the release-mode p95 gate for the wired preview
    /// path belongs to the perf harness, not to this single-sample probe.
    #[test]
    #[ignore = "timing: wall-clock probe, run under --perf"]
    fn explain_timing_probe_on_large_batch() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        for i in 0..500 {
            insert(
                &mut doc,
                &mut journal,
                ObjectId::new(),
                ObjectId::NIL,
                [i as f32, 0.0, 0.0],
            );
        }
        let mut ids: Vec<ObjectId> = Vec::new();
        let mut stack = doc.hierarchy.roots();
        while let Some(id) = stack.pop() {
            ids.push(id);
            stack.extend(doc.hierarchy.children(id));
        }
        let ops: Vec<Op> = ids
            .iter()
            .take(256)
            .map(|id| Op::SetLocal {
                id: *id,
                local: pose_at(1.0, 0.0, 1.0),
            })
            .collect();
        let big = Batch {
            ops,
            base_revision: doc.revision,
            idempotency_key: 4242,
            provenance: None,
        };
        let start = std::time::Instant::now();
        let _ = explain_batch(&doc, &big);
        let explain_us = start.elapsed().as_micros();
        let start = std::time::Instant::now();
        let _ = commands::validate(&doc, &big);
        let validate_us = start.elapsed().as_micros();
        eprintln!("explain {explain_us}us vs validate {validate_us}us over 256 ops / 500 nodes");
        assert!(
            explain_us < 50_000,
            "EXPLAIN must stay in microseconds, took {explain_us}us"
        );
    }
}
