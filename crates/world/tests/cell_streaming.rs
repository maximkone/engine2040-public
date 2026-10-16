//! Acceptance: 9-cell walk with zero hitches, 30-minute memory loop,
//! frame-boundary publish, byte-identical bakes, exact eviction, corrupt-cell
//! refusal, boundary rebake and repair-once. Wires the real asset stream queue
//! (`crates/assets/src/stream.rs`) and the real nav mesh bake
//! (`crates/nav/src/mesh.rs`) by source path so this lane edits no manifest;
//! the orchestrator promotes both to real `engine-world` dependency edges.
//! Status: unwired — no map assembler exists yet; gate: the publish check.

use engine_world::cells;

#[path = "../../assets/src/stream.rs"]
mod as2;

#[path = "../../nav/src/mesh.rs"]
mod mesh;

/// The byte cursor `mesh` decodes through, addressed there as `crate::cursor`.
#[path = "../../nav/src/cursor.rs"]
mod cursor;

#[path = "../../nav/src/flow.rs"]
mod flow;

#[path = "../../nav/src/tiles.rs"]
mod tiles;

/// Live-only mesh-tile rebake (`rebake_mesh_tile`); the alias
/// re-exports, `update` and `rebake_stacked_level` are retired, so this
/// include now resolves only the kept WD8 path for `mesh` tests.
#[path = "../../nav/src/rebake.rs"]
mod rebake;

use as2::{StreamBudget, StreamJob, StreamKind};
use cells::{
    bake_cell, cell_cost_bytes, parse_cell, seam_joins, CellBudget, CellCoord, CellError, Cells,
};

const SEED: u64 = 12345;
const SIZE: u16 = 16;
/// Exact baked cost of one 16-texel cell: 23 header + 256 heights + 32 hole bits.
const COST: u64 = 23 + 256 + 32;
/// One cell per `publish` call: the frame cap equals the cell cost.
const FRAME: u64 = COST;
/// 30 minutes at 60 Hz, run as a fast integer loop (no wall clock in sim gates).
const THIRTY_MIN_FRAMES: u32 = 108_000;

/// The 3x3 scripted-walk block, cell units.
fn block9() -> Vec<CellCoord> {
    let mut out = Vec::new();
    for z in -1..=1 {
        for x in -1..=1 {
            out.push(CellCoord::new(x, z).expect("fixture coords in range"));
        }
    }
    out
}

/// Fight-critical first: the hub cell streams before its ring.
fn priority_for(coord: &CellCoord) -> u8 {
    9u8.saturating_sub((coord.x.abs() + coord.z.abs()) as u8)
}

/// Deterministic stream id for one coord: FNV over the coord, hexed out to 64 chars.
fn stream_id(coord: &CellCoord) -> as2::StreamId {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in coord
        .x
        .to_le_bytes()
        .iter()
        .chain(coord.z.to_le_bytes().iter())
    {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    as2::StreamId::parse(&format!("{hash:016x}").repeat(4)).expect("fixture id is 64 hex")
}

fn cell_budget() -> CellBudget {
    CellBudget::new(9 * COST, 9, FRAME).expect("fixture budget nonzero")
}

fn as2_budget() -> StreamBudget {
    StreamBudget::new(9 * COST, 9)
}

/// Stream one coord through both the cell registry and the real asset queue.
fn stream_both(
    cells: &mut Cells,
    queue: &mut as2::StreamQueue,
    coord: CellCoord,
) -> (cells::CellEvent, as2::StreamEvent) {
    let priority = priority_for(&coord);
    let cell_event = cells
        .stream(coord, SIZE, priority)
        .expect("fixture cell fits its budget");
    let job = StreamJob::new(
        stream_id(&coord),
        &format!("cell-{},{}", coord.x, coord.z),
        StreamKind::Mesh,
        COST,
        priority,
    )
    .expect("fixture job valid");
    let queue_event = queue.request(job).expect("fixture job fits AS2");
    (cell_event, queue_event)
}

#[test]
fn cell_streaming_bakes_byte_identical_x2() {
    assert_eq!(cell_cost_bytes(SIZE).expect("size valid"), COST);
    let coord = CellCoord::new(2, -3).expect("fixture coord in range");
    let first = bake_cell(SEED, coord, SIZE).expect("fixture cell bakes");
    let second = bake_cell(SEED, coord, SIZE).expect("fixture cell bakes");
    assert_eq!(first, second, "same seed plus coord bakes identical bytes");
    assert_eq!(
        Cells::digest(&first),
        Cells::digest(&second),
        "digests match across bakes"
    );
    assert_eq!(first.len() as u64, COST, "baked length equals posted cost");
    let parsed = parse_cell(&first).expect("own bake parses");
    assert_eq!(parsed.seed, SEED);
    assert_eq!(parsed.coord, coord);
    assert_eq!(parsed.size, SIZE);
    assert_eq!(parsed.heights.len(), 256);
    assert_eq!(parsed.holes.len(), 256);
    assert_eq!(parsed.to_bytes(), first, "encode round-trips exactly");
    assert!(
        parsed.walkable_count() > 200,
        "mostly walkable floor, got {}",
        parsed.walkable_count()
    );
    // Neighbour bakes differ (different ground) but share the seam scan.
    let east = CellCoord::new(3, -3).expect("fixture coord in range");
    let east_data =
        parse_cell(&bake_cell(SEED, east, SIZE).expect("east bakes")).expect("east parses");
    assert_ne!(
        first,
        east_data.to_bytes(),
        "distinct coords bake distinct bytes"
    );
    seam_joins(&parsed, &east_data).expect("adjacent bakes join seamlessly");
    // A different seed is different ground with the same shape of bytes.
    let other = bake_cell(SEED + 1, coord, SIZE).expect("other seed bakes");
    assert_ne!(first, other);
    assert_eq!(other.len() as u64, COST);
    assert!(matches!(
        seam_joins(&parsed, &parse_cell(&other).expect("parses")),
        Err(CellError::SeamMismatch("seed"))
    ));
}

#[test]
fn cell_streaming_scales_nest_seamlessly() {
    // A 32-cell must equal its four 16-children texel for texel: the halo
    // covers erosion reach, so no bake edge can crack. Without the halo this
    // fails at every shared edge, which is what makes it the seam proof.
    let parent =
        parse_cell(&bake_cell(SEED, CellCoord::new(0, 0).expect("hub"), 32).expect("bakes"))
            .expect("parent parses");
    for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        let child_coord = CellCoord::new(dx, dz).expect("child");
        let child =
            parse_cell(&bake_cell(SEED, child_coord, SIZE).expect("bakes")).expect("child parses");
        for y in 0..16 {
            for x in 0..16 {
                let px = dx as usize * 16 + x;
                let py = dz as usize * 16 + y;
                assert_eq!(
                    parent.heights[py * 32 + px],
                    child.heights[y * 16 + x],
                    "parent ({px},{py}) equals child {child_coord} ({x},{y})"
                );
            }
        }
        // Nesting holds for holes too: holes are window-independent global
        // singles, so every parent texel equals its child texel exactly,
        // borders included.
        for y in 0..16 {
            for x in 0..16 {
                let px = dx as usize * 16 + x;
                let py = dz as usize * 16 + y;
                assert_eq!(
                    parent.holes[py * 32 + px],
                    child.holes[y * 16 + x],
                    "hole ({px},{py}) nests"
                );
            }
        }
    }
    // Regression bound: no shared edge in the walk block steps past SEAM_TOL.
    let mut max_step: u16 = 0;
    for coord in block9() {
        for (dx, dz) in [(1, 0), (0, 1)] {
            let next = CellCoord::new(coord.x + dx, coord.z + dz).expect("neighbour");
            if next.x > 1 || next.z > 1 {
                continue;
            }
            let a = parse_cell(&bake_cell(SEED, coord, SIZE).expect("bakes")).expect("parses");
            let b = parse_cell(&bake_cell(SEED, next, SIZE).expect("bakes")).expect("parses");
            seam_joins(&a, &b).expect("block edges join");
            for k in 0..16 {
                let (ha, hb) = if dx == 1 {
                    (a.heights[k * 16 + 15], b.heights[k * 16])
                } else {
                    (a.heights[15 * 16 + k], b.heights[k])
                };
                max_step = max_step.max((i16::from(ha) - i16::from(hb)).unsigned_abs());
            }
        }
    }
    assert!(
        max_step <= u16::from(cells::SEAM_TOL),
        "measured max step {max_step} within SEAM_TOL {}",
        cells::SEAM_TOL
    );
}

#[test]
fn cell_streaming_nine_cell_walk_zero_hitches() {
    let mut cells = Cells::new(SEED, cell_budget());
    let mut queue = as2::StreamQueue::new(as2_budget());
    assert_eq!(cells.seed(), SEED);
    assert_eq!(cells.budget(), cell_budget());
    assert_eq!(cells.live_count(), 0);
    assert_eq!(cells.pending_count(), 0);
    for coord in block9() {
        let (cell_event, queue_event) = stream_both(&mut cells, &mut queue, coord);
        assert_eq!(cell_event, cells::CellEvent::Queued { coord });
        assert!(matches!(queue_event, as2::StreamEvent::Queued { .. }));
    }
    assert_eq!(cells.pending_count(), 9);
    assert_eq!(queue.pending_count(), 9);
    assert_eq!(cells.used_bytes(), 9 * COST);
    assert_eq!(queue.used_bytes(), 9 * COST);
    // Publish order is priority first, coord ascending: hub before ring.
    let order = cells.pending_order();
    assert_eq!(order[0], CellCoord::new(0, 0).expect("hub"));
    // The tick reads live cells only: the far corner is not routable yet.
    let far = CellCoord::new(1, 1).expect("corner");
    assert_eq!(
        cells.route(CellCoord::new(0, 0).expect("hub"), (0, 0), far, (0, 0)),
        Err(CellError::NotLive),
        "unloaded endpoint never guesses"
    );
    // Scripted walk: each frame publishes one cell and steps onto it, so the
    // walker never waits at an edge. Per-cell load time is 1 frame, peak is
    // 9 cells = 2799 bytes = 100% of the stream budget (within the 5% bar).
    let mut hitches = 0u32;
    let mut completed: Vec<CellCoord> = Vec::new();
    let mut as2_completed: Vec<as2::StreamId> = Vec::new();
    for _ in 0..9 {
        let events = cells.publish(FRAME);
        let queue_events = queue.publish(FRAME);
        let done: Vec<CellCoord> = events
            .iter()
            .filter_map(|event| match event {
                cells::CellEvent::Completed { coord } => Some(*coord),
                _ => None,
            })
            .collect();
        assert_eq!(done.len(), 1, "one cell per frame at this cap: {events:?}");
        let coord = done[0];
        if !cells.is_live(&coord) {
            hitches += 1;
        }
        assert!(cells.is_live(&coord), "completed cell is tick-readable");
        assert!(!cells.is_placeholder(&coord));
        assert_eq!(cells.progress_of(&coord), Some((COST, COST)));
        assert_eq!(
            cells.load_frames(&coord),
            Some(1),
            "named load time: 1 frame"
        );
        assert_eq!(
            cells.cell_bytes(&coord),
            Some(COST),
            "named cost: {COST} bytes"
        );
        assert!(
            cells.collider_count(&coord).is_some(),
            "physics binding bound for {coord}"
        );
        completed.push(coord);
        for event in queue_events {
            if let as2::StreamEvent::Completed { id } = event {
                as2_completed.push(id);
            }
        }
    }
    assert_eq!(completed, order, "completion follows publish order");
    assert_eq!(as2_completed.len(), 9, "AS2 queue drained in lockstep");
    assert_eq!(cells.live_count(), 9);
    assert_eq!(cells.live_coords().len(), 9);
    assert_eq!(hitches, 0, "zero tick hitches across the 9-cell walk");
    assert_eq!(cells.used_bytes(), 9 * COST);
    assert_eq!(
        cells.high_water_bytes(),
        9 * COST,
        "named peak memory: {} bytes",
        9 * COST
    );
    assert_eq!(queue.high_water_bytes(), cells.high_water_bytes());
    assert_eq!(cells.remaining_bytes(), 0);
    assert_eq!(cells.remaining_jobs(), 0);
    assert_eq!(cells.refused_count(), 0);
    // Cross-cell nav binds: the far corner routes through live cells now.
    let steps = cells
        .route(
            CellCoord::new(-1, -1).expect("corner"),
            (0, 0),
            far,
            (15, 15),
        )
        .expect("live block routes");
    assert!(steps > 16, "route crosses cells, got {steps} steps");
    // The sim hash covers live cell bytes only: idle frames move nothing,
    // so repeated publishes leave it byte-identical.
    let sim_then = cells.sim_hash();
    assert!(
        cells.publish(u64::MAX).is_empty(),
        "steady state publishes nothing"
    );
    assert!(cells.publish(0).is_empty(), "zero cap touches nothing");
    assert_eq!(
        cells.sim_hash(),
        sim_then,
        "idle frames leave the sim hash alone"
    );
    // Nav binds per cell: the hub footprint bakes through the real mesh bake,
    // byte-identical twice and under shuffled input, with a walking query.
    let tris = [
        [[0.0, 0.0, 0.0], [0.0, 0.0, 16.0], [16.0, 0.0, 16.0]],
        [[0.0, 0.0, 0.0], [16.0, 0.0, 16.0], [16.0, 0.0, 0.0]],
    ];
    let config = mesh::BakeConfig {
        cell_size: 1.0,
        walkable_normal_y: 0.7,
    };
    let first = mesh::bake(&tris, &config).expect("hub footprint bakes");
    let second = mesh::bake(&tris, &config).expect("hub footprint bakes");
    assert_eq!(first.to_bytes(), second.to_bytes());
    assert_eq!(first.digest(), second.digest());
    let mut shuffled = tris.to_vec();
    shuffled.reverse();
    assert_eq!(
        mesh::bake(&shuffled, &config)
            .expect("shuffled bakes")
            .to_bytes(),
        first.to_bytes(),
        "NV1 order-permutation bar holds per cell"
    );
    assert!(first.walkable_cells() > 100, "hub floor is walkable");
    assert!(
        first.query([0.5, 0.0, 0.5], [15.5, 0.0, 15.5]).is_ok(),
        "hub query walks"
    );
    let via_method = mesh::NavMesh::bake(&tris, &config).expect("method bakes");
    assert_eq!(via_method.to_bytes(), first.to_bytes());
    assert_eq!(first.cell_size_m(), 1.0);
    assert!(
        first.grid_dims().0 > 0 && first.grid_dims().1 > 0,
        "grid dims held"
    );
    assert_eq!(first.polygon_count(), first.polygons().len());
    let poly0 = first.polygon(mesh::PolyId(0)).expect("polygon zero held");
    assert_eq!(poly0.area(), 0);
    assert!(!poly0.cells().is_empty());
    assert!(!first
        .query_area([8.0, 0.0, 8.0], 20.0)
        .expect("area scans")
        .is_empty());
}

#[test]
fn cell_streaming_thirty_minute_loop_memory_within_five_percent() {
    let mut cells = Cells::new(SEED, cell_budget());
    for coord in block9() {
        cells
            .stream(coord, SIZE, priority_for(&coord))
            .expect("fixture fits");
    }
    while cells.live_count() < 9 {
        cells.publish(FRAME);
    }
    let peak = cells.high_water_bytes();
    assert_eq!(peak, 9 * COST, "named peak: {peak} bytes");
    let corners = [
        CellCoord::new(-1, -1).expect("corner"),
        CellCoord::new(1, 1).expect("corner"),
    ];
    let mut worst: u64 = peak;
    let mut frame: u32 = 0;
    while frame < THIRTY_MIN_FRAMES {
        frame += 1;
        // Steady frames move nothing: the tick reads live cells for free.
        let idle = cells.publish(FRAME);
        assert!(
            idle.is_empty(),
            "steady state publishes nothing at frame {frame}"
        );
        // Once a minute two far cells cycle out and back under the same budget.
        if frame.is_multiple_of(3600) {
            for coord in corners {
                let freed = cells.evict(&coord).expect("corner live");
                assert_eq!(
                    freed,
                    cells::CellEvent::Evicted {
                        coord,
                        freed_bytes: COST,
                    }
                );
                cells
                    .stream(coord, SIZE, priority_for(&coord))
                    .expect("freed bytes admit the cell");
            }
            while cells.live_count() < 9 {
                cells.publish(FRAME);
            }
        }
        worst = worst.max(cells.used_bytes());
        assert!(
            cells.used_bytes() <= cell_budget().bytes,
            "never past the AS2 budget at frame {frame}"
        );
    }
    assert_eq!(cells.live_count(), 9);
    assert_eq!(cells.used_bytes(), 9 * COST);
    assert_eq!(cells.high_water_bytes(), peak, "high-water never falls");
    let drift = peak.abs_diff(worst);
    assert!(
        drift * 100 <= 5 * cell_budget().bytes,
        "30-minute drift {drift} bytes within 5% of budget"
    );
    assert_eq!(cells.refused_count(), 0, "no refusal in the soak");
}

#[test]
fn cell_streaming_corrupt_cells_refuse_named() {
    let coord = CellCoord::new(0, 0).expect("hub");
    let good = bake_cell(SEED, coord, SIZE).expect("hub bakes");
    let mut bad_magic = good.clone();
    bad_magic[0] = b'X';
    assert_eq!(
        parse_cell(&bad_magic),
        Err(CellError::Corrupt("magic")),
        "magic names its check"
    );
    let mut bad_version = good.clone();
    bad_version[4] = 0x7f;
    assert_eq!(
        parse_cell(&bad_version),
        Err(CellError::Corrupt("version")),
        "version names its check"
    );
    assert_eq!(
        parse_cell(&good[..good.len() / 2]),
        Err(CellError::Corrupt("truncated")),
        "short blob names its check"
    );
    let mut trailing = good.clone();
    trailing.push(0);
    assert_eq!(
        parse_cell(&trailing),
        Err(CellError::Corrupt("trailing")),
        "extra bytes name their check"
    );
    // Holes are window-independent global data, so a hole bit on the window
    // border still parses: borders carry ground like any other texel.
    let mut border = good.clone();
    let hole_byte = cells::CELL_HEADER_LEN + 256;
    border[hole_byte] |= 1;
    assert!(
        parse_cell(&border).is_ok(),
        "border texels carry holes like any ground"
    );
    // A header claiming a wider side than the bytes hold is truncation.
    let mut bad_size = good.clone();
    bad_size[21] = 64;
    bad_size[22] = 0;
    assert_eq!(
        parse_cell(&bad_size),
        Err(CellError::Corrupt("truncated")),
        "short body names its check"
    );
    // A header below the side bound refuses before any length math.
    let mut tiny_size = good.clone();
    tiny_size[21] = 3;
    tiny_size[22] = 0;
    assert_eq!(
        parse_cell(&tiny_size),
        Err(CellError::Corrupt("size")),
        "tiny side names its check"
    );
    assert_eq!(parse_cell(&[]), Err(CellError::Corrupt("truncated")));
    assert!(matches!(CellCoord::new(9999, 0), Err(CellError::BadCoord)));
    assert!(matches!(
        bake_cell(SEED, coord, 3),
        Err(CellError::BadSize { got: 3 })
    ));
    assert!(matches!(
        CellBudget::new(0, 1, 1),
        Err(CellError::BadBudget)
    ));
    // Every error displays: refusals are never silent drops.
    for error in [
        CellError::BadCoord,
        CellError::BadBudget,
        CellError::Duplicate,
        CellError::OverBudget {
            want_bytes: 9,
            free_bytes: 1,
        },
        CellError::JobCap { limit: 1 },
        CellError::Unknown,
        CellError::PendingNeedsCancel,
        CellError::LiveNeedsEvict,
        CellError::Corrupt("magic"),
        CellError::SeamMismatch("step"),
        CellError::NotLive,
        CellError::Unreachable,
        CellError::BadIndex,
        CellError::AlreadyRepaired,
        CellError::BadSize { got: 3 },
    ] {
        assert!(!format!("{error}").is_empty(), "displays: {error:?}");
        assert!(!format!("{coord}").is_empty(), "coord displays");
    }
}

#[test]
fn cell_streaming_eviction_exact_under_load() {
    let budget = CellBudget::new(2 * COST, 4, FRAME).expect("small budget");
    let mut cells = Cells::new(SEED, budget);
    let mut queue = as2::StreamQueue::new(StreamBudget::new(2 * COST, 4));
    let a = CellCoord::new(0, 0).expect("hub");
    let b = CellCoord::new(1, 0).expect("east");
    let c = CellCoord::new(0, 1).expect("south");
    cells.stream(a, SIZE, 9).expect("a fits");
    cells.stream(b, SIZE, 5).expect("b fits");
    queue
        .request(
            StreamJob::new(stream_id(&a), "cell-a", StreamKind::Mesh, COST, 9)
                .expect("job a valid"),
        )
        .expect("AS2 admits a");
    queue
        .request(
            StreamJob::new(stream_id(&b), "cell-b", StreamKind::Mesh, COST, 5)
                .expect("job b valid"),
        )
        .expect("AS2 admits b");
    // Duplicate pending keeps the first slot.
    assert_eq!(cells.stream(a, SIZE, 9), Err(CellError::Duplicate));
    assert!(cells.is_placeholder(&a));
    cells.publish(u64::MAX);
    queue.publish(u64::MAX);
    assert_eq!(cells.live_count(), 2);
    assert_eq!(cells.used_bytes(), 2 * COST);
    assert_eq!(queue.used_bytes(), 2 * COST);
    // Live re-request is a cache hit, not a second reservation.
    assert_eq!(
        cells.stream(a, SIZE, 9),
        Ok(cells::CellEvent::Completed { coord: a })
    );
    assert_eq!(cells.used_bytes(), 2 * COST);
    // Full budget refuses the third cell on both queues with numbers.
    assert_eq!(
        cells.stream(c, SIZE, 5),
        Err(CellError::OverBudget {
            want_bytes: COST,
            free_bytes: 0,
        })
    );
    assert!(matches!(
        queue.request(
            StreamJob::new(stream_id(&c), "cell-c", StreamKind::Mesh, COST, 5)
                .expect("job c valid")
        ),
        Err(as2::StreamError::OverBudget { .. })
    ));
    assert_eq!(cells.refused_count(), 1);
    // Eviction is exact: the freed bytes equal the baked cost on both queues.
    assert_eq!(
        cells.evict(&a),
        Ok(cells::CellEvent::Evicted {
            coord: a,
            freed_bytes: COST,
        })
    );
    assert_eq!(
        queue.evict(&stream_id(&a)),
        Ok(as2::StreamEvent::Evicted {
            id: stream_id(&a),
            freed_bytes: COST,
        })
    );
    assert_eq!(cells.used_bytes(), COST);
    assert_eq!(
        cells.high_water_bytes(),
        2 * COST,
        "high-water never falls after evict"
    );
    cells.stream(c, SIZE, 5).expect("freed bytes admit c");
    cells.publish(u64::MAX);
    assert!(cells.is_live(&c));
    assert_eq!(cells.used_bytes(), 2 * COST);
    // Pending cancels free reservations; live cancels need evict and vice versa.
    cells.evict(&b).expect("b evicted for room");
    let d = CellCoord::new(2, 0).expect("far");
    cells.stream(d, SIZE, 1).expect("d pending");
    assert_eq!(
        cells.evict(&d),
        Err(CellError::PendingNeedsCancel),
        "pending needs cancel"
    );
    assert_eq!(
        cells.cancel(&d),
        Ok(cells::CellEvent::Cancelled { coord: d })
    );
    assert_eq!(
        cells.cancel(&c),
        Err(CellError::LiveNeedsEvict),
        "live needs evict"
    );
    assert_eq!(
        cells.evict(&CellCoord::new(7, 7).expect("far")),
        Err(CellError::Unknown)
    );
    assert_eq!(cells.remaining_bytes(), COST);
    assert_eq!(cells.remaining_jobs(), 3);
}

#[test]
fn cell_streaming_boundary_rebake_and_repair_once() {
    let mut cells = Cells::new(SEED, cell_budget());
    let block = [
        CellCoord::new(0, 0).expect("hub"),
        CellCoord::new(1, 0).expect("east"),
        CellCoord::new(0, 1).expect("south"),
        CellCoord::new(1, 1).expect("corner"),
    ];
    for coord in block {
        cells
            .stream(coord, SIZE, priority_for(&coord))
            .expect("block fits");
    }
    cells.publish(u64::MAX);
    assert_eq!(cells.live_count(), 4);
    let hash_before = cells.sim_hash();
    // Seams join around the whole block before any eviction.
    let hub = parse_cell(&bake_cell(SEED, block[0], SIZE).expect("hub bakes")).expect("hub parses");
    let east =
        parse_cell(&bake_cell(SEED, block[1], SIZE).expect("east bakes")).expect("east parses");
    let south =
        parse_cell(&bake_cell(SEED, block[2], SIZE).expect("south bakes")).expect("south parses");
    seam_joins(&hub, &east).expect("east seam joins");
    seam_joins(&hub, &south).expect("south seam joins");
    assert_eq!(
        seam_joins(&east, &south),
        Err(CellError::SeamMismatch("adjacent")),
        "diagonal cells are not neighbours"
    );
    assert_eq!(
        seam_joins(
            &hub,
            &parse_cell(&bake_cell(SEED, block[1], 32).expect("wide bakes")).expect("wide parses")
        ),
        Err(CellError::SeamMismatch("size")),
        "size drift names its check"
    );
    // A forced cliff across the shared edge refuses with its check: flipping
    // the high bit moves the texel by exactly 128, past any honest tolerance.
    let mut cliff = east.clone();
    cliff.heights[0] = hub.heights[15] ^ 0x80;
    assert_eq!(
        seam_joins(&hub, &cliff),
        Err(CellError::SeamMismatch("step")),
        "cliff names its check"
    );
    // Boundary rebake: evict the hub and reload it; the sim hash returns to
    // exactly its pre-eviction value, so edges did not shift under reload.
    cells.evict(&block[0]).expect("hub evicted");
    cells
        .stream(block[0], SIZE, priority_for(&block[0]))
        .expect("hub re-admitted");
    cells.publish(u64::MAX);
    assert_eq!(
        cells.sim_hash(),
        hash_before,
        "boundary rebake is byte-identical"
    );
    seam_joins(&hub, &east).expect("seam still joins after rebake");
    // Self-repair budget of one: a refused coord heals once, then stops.
    let hurt = CellCoord::new(-2, 0).expect("repair coord");
    assert_eq!(
        cells.repair(hurt, SIZE, 7),
        Ok(cells::CellEvent::Queued { coord: hurt })
    );
    cells.publish(u64::MAX);
    assert!(cells.is_live(&hurt), "repaired cell goes live");
    assert_eq!(
        cells.repair(hurt, SIZE, 7),
        Err(CellError::AlreadyRepaired),
        "second repair without news fails"
    );
}

#[test]
fn cell_streaming_unloaded_queries_never_guess() {
    let mut cells = Cells::new(SEED, cell_budget());
    let home = CellCoord::new(0, 0).expect("hub");
    let far = CellCoord::new(5, 5).expect("far");
    cells.stream(home, SIZE, 9).expect("home fits");
    cells.stream(far, SIZE, 1).expect("far fits");
    cells.publish(u64::MAX);
    assert!(cells.is_live(&home) && cells.is_live(&far));
    // Live but disconnected islands: endpoints walkable, no path, no guess.
    assert_eq!(
        cells.route(home, (0, 0), far, (0, 0)),
        Err(CellError::Unreachable)
    );
    // A texel past the side refuses instead of wrapping into a neighbour.
    assert_eq!(
        cells.route(home, (99, 0), far, (0, 0)),
        Err(CellError::BadIndex)
    );
    // The gap between the islands is unloaded ground: wait, never guess.
    assert_eq!(
        cells.route(home, (0, 0), CellCoord::new(2, 2).expect("gap"), (0, 0)),
        Err(CellError::NotLive)
    );
    // The same silence rule as nav: disconnected nav islands never guess either.
    let mut island = vec![
        [[0.0, 0.0, 0.0], [0.0, 0.0, 2.0], [2.0, 0.0, 2.0]],
        [[0.0, 0.0, 0.0], [2.0, 0.0, 2.0], [2.0, 0.0, 0.0]],
    ];
    island.extend([
        [[6.0, 0.0, 6.0], [6.0, 0.0, 8.0], [8.0, 0.0, 8.0]],
        [[6.0, 0.0, 6.0], [8.0, 0.0, 8.0], [8.0, 0.0, 6.0]],
    ]);
    let mesh = mesh::bake(
        &island,
        &mesh::BakeConfig {
            cell_size: 0.5,
            walkable_normal_y: 0.7,
        },
    )
    .expect("islands bake");
    assert_eq!(
        mesh.query([0.5, 0.0, 0.5], [7.0, 0.0, 7.0]),
        Err(mesh::NavError::Unreachable)
    );
}

#[test]
fn cell_streaming_publish_happens_at_frame_boundaries() {
    let mut cells = Cells::new(SEED, cell_budget());
    let coord = CellCoord::new(0, 0).expect("hub");
    cells.stream(coord, SIZE, 9).expect("hub fits");
    // A zero cap emits nothing and touches nothing.
    assert!(cells.publish(0).is_empty());
    assert_eq!(cells.progress_of(&coord), Some((0, COST)));
    assert!(cells.is_placeholder(&coord));
    assert!(!cells.is_live(&coord));
    // A partial frame reports progress but the cell is still not tick-readable.
    let first = cells.publish(100);
    assert_eq!(
        first,
        vec![
            cells::CellEvent::Started { coord },
            cells::CellEvent::Progress {
                coord,
                done_bytes: 100,
                total_bytes: COST,
            },
        ]
    );
    assert!(!cells.is_live(&coord), "partial cells never go live early");
    assert_eq!(cells.used_bytes(), COST, "reservation held across frames");
    // The rest completes at the next boundary with its named load time.
    let rest = cells.publish(u64::MAX);
    assert_eq!(
        rest,
        vec![
            cells::CellEvent::Progress {
                coord,
                done_bytes: COST,
                total_bytes: COST,
            },
            cells::CellEvent::Completed { coord },
        ]
    );
    assert!(cells.is_live(&coord));
    assert_eq!(cells.load_frames(&coord), Some(2));
    assert_eq!(
        cells.used_bytes(),
        COST,
        "completion converts the reservation, never double-counts"
    );
    let bytes = bake_cell(SEED, coord, SIZE).expect("hub bakes");
    assert_eq!(
        Cells::digest(&bytes),
        Cells::digest(&bytes),
        "digest deterministic per blob"
    );
}
