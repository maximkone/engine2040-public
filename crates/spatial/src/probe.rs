//! Peg-transfer probe over coarse reach with a sync re-probe step.
//! Owner: spatial crate. Units: integer centimeters, polygon ids as u32.
//! Determinism: sorted edges, linear scans, integer math only; same slices
//! twice give same answers on every build. Errors: typed [`ProbeError`].
//! Budget: each probe walks the edge and blocker slices once, far inside
//! 0.2 ms per object on the reference workload; timing lives in tests.

use std::collections::BTreeSet;
use std::collections::VecDeque;
use std::fmt;

/// Lenient door clear for the wide pass, in cm.
pub const WIDE_DOOR_CM: i32 = 80;
/// Exact door clear for the sync pass, in cm (matches the nav door rule).
pub const EXACT_DOOR_CM: i32 = 90;
/// Clear assumed when an edge names no door, in cm.
pub const OPEN_DOOR_CM: i32 = 200;
/// Budget per probed object, in microseconds (0.2 ms).
pub const BUDGET_US_PER_OBJECT: u128 = 200;

/// What stopped a probe step. Every message names the legal values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeError {
    /// No peg carries this id in the given slice.
    UnknownPeg(u32),
    /// A door width is negative or past the world bound.
    BadDoor {
        /// Edge start polygon.
        a: u32,
        /// Edge end polygon.
        b: u32,
        /// Rejected width in cm.
        width_cm: i32,
    },
}

impl ProbeError {
    /// Machine code for this failure.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::UnknownPeg(_) => "unknown_peg",
            Self::BadDoor { .. } => "bad_door",
        }
    }
}

impl fmt::Display for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownPeg(id) => write!(f, "probe: unknown peg {id}"),
            Self::BadDoor { a, b, width_cm } => write!(
                f,
                "probe: door {a}-{b} width {width_cm} outside 0..=10000 cm"
            ),
        }
    }
}

impl std::error::Error for ProbeError {}

/// One peg: center in integer cm plus its polygon, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Peg {
    /// Stable peg id.
    pub id: u32,
    /// Center x in cm.
    pub x_cm: i32,
    /// Center z in cm.
    pub z_cm: i32,
    /// Polygon holding the peg, if any.
    pub poly: Option<u32>,
}

/// One ground blocker footprint in integer cm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Blocker {
    /// Minimum x in cm.
    pub min_x: i32,
    /// Maximum x in cm.
    pub max_x: i32,
    /// Minimum z in cm.
    pub min_z: i32,
    /// Maximum z in cm.
    pub max_z: i32,
}

/// Polygon holding an actor id, if any. Linear scan keeps order stable.
pub fn poly_of(at: &[(u32, u32)], actor: u32) -> Option<u32> {
    for (id, poly) in at {
        if *id == actor {
            return Some(*poly);
        }
    }
    None
}

/// Clear width for one undirected edge; missing edges read wide open.
fn door_width(doors: &[(u32, u32, i32)], a: u32, b: u32) -> i32 {
    for (x, y, w) in doors {
        if (*x == a && *y == b) || (*x == b && *y == a) {
            return *w;
        }
    }
    OPEN_DOOR_CM
}

/// Validate door widths; refuses negative or oversized entries loudly.
pub fn check_doors(doors: &[(u32, u32, i32)]) -> Result<(), ProbeError> {
    for (a, b, w) in doors {
        if *w < 0 || *w > 10_000 {
            return Err(ProbeError::BadDoor {
                a: *a,
                b: *b,
                width_cm: *w,
            });
        }
    }
    Ok(())
}

/// Neighbors of one polygon in ascending order.
fn neighbors(adj: &[(u32, u32)], cur: u32, out: &mut Vec<u32>) {
    out.clear();
    for (a, b) in adj {
        if *a == cur {
            out.push(*b);
        } else if *b == cur {
            out.push(*a);
        }
    }
    out.sort_unstable();
    out.dedup();
}

/// Breadth-first reach over edges passing `pass`. Start reaching itself
/// reads reachable; missing endpoints read unreachable, never a guess.
fn bfs(adj: &[(u32, u32)], start: u32, goal: u32, pass: &dyn Fn(u32, u32) -> bool) -> bool {
    if start == goal {
        return true;
    }
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::new();
    seen.insert(start);
    queue.push_back(start);
    let mut next: Vec<u32> = Vec::new();
    while let Some(cur) = queue.pop_front() {
        neighbors(adj, cur, &mut next);
        for n in std::mem::take(&mut next) {
            if !pass(cur, n) {
                continue;
            }
            if n == goal {
                return true;
            }
            if seen.insert(n) {
                queue.push_back(n);
            }
        }
    }
    false
}

/// Coarse reach: polygon connectivity only, ignoring doors and blockers.
/// Off-mesh endpoints read false.
pub fn coarse_reachable(
    adj: &[(u32, u32)],
    at: &[(u32, u32)],
    from: u32,
    goal_poly: Option<u32>,
) -> bool {
    let (Some(a), Some(b)) = (poly_of(at, from), goal_poly) else {
        return false;
    };
    bfs(adj, a, b, &|_, _| true)
}

/// Wide pass: connectivity over edges clearing the lenient width.
/// Off-mesh endpoints read false.
pub fn wide_reachable(
    adj: &[(u32, u32)],
    doors: &[(u32, u32, i32)],
    at: &[(u32, u32)],
    from: u32,
    goal_poly: Option<u32>,
) -> bool {
    let (Some(a), Some(b)) = (poly_of(at, from), goal_poly) else {
        return false;
    };
    bfs(adj, a, b, &|x, y| door_width(doors, x, y) >= WIDE_DOOR_CM)
}

/// Strict interior test for a point against a blocker.
fn inside_strict(x: i32, z: i32, r: &Blocker) -> bool {
    x > r.min_x && x < r.max_x && z > r.min_z && z < r.max_z
}

/// Exact cross of segments ab and cd in i64.
fn cross(ax: i64, az: i64, bx: i64, bz: i64, cx: i64, cz: i64) -> i64 {
    (bx - ax) * (cz - az) - (bz - az) * (cx - ax)
}

/// One integer point in cm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pt {
    /// X in cm.
    x: i32,
    /// Z in cm.
    z: i32,
}

/// True when open segments ab and cd cross in strict interiors.
fn proper_cross(a: Pt, b: Pt, c: Pt, d: Pt) -> bool {
    let (ax, az, bx, bz, cx, cz, dx, dz) = (
        i64::from(a.x),
        i64::from(a.z),
        i64::from(b.x),
        i64::from(b.z),
        i64::from(c.x),
        i64::from(c.z),
        i64::from(d.x),
        i64::from(d.z),
    );
    let o1 = cross(ax, az, bx, bz, cx, cz);
    let o2 = cross(ax, az, bx, bz, dx, dz);
    let o3 = cross(cx, cz, dx, dz, ax, az);
    let o4 = cross(cx, cz, dx, dz, bx, bz);
    ((o1 > 0 && o2 < 0) || (o1 < 0 && o2 > 0)) && ((o3 > 0 && o4 < 0) || (o3 < 0 && o4 > 0))
}

/// True when the open segment enters any blocker interior. Touching an
/// edge or corner stays clear, so grazing carries never block.
pub fn segment_clear(ax: i32, az: i32, gx: i32, gz: i32, blockers: &[Blocker]) -> bool {
    for r in blockers {
        if inside_strict(ax, az, r) || inside_strict(gx, gz, r) {
            return false;
        }
        let corners = [
            (r.min_x, r.min_z),
            (r.max_x, r.min_z),
            (r.max_x, r.max_z),
            (r.min_x, r.max_z),
        ];
        for i in 0..4 {
            let (cx, cz) = corners[i];
            let (dx, dz) = corners[(i + 1) % 4];
            let query = Pt { x: ax, z: az };
            let goal = Pt { x: gx, z: gz };
            let edge_a = Pt { x: cx, z: cz };
            let edge_b = Pt { x: dx, z: dz };
            if proper_cross(query, goal, edge_a, edge_b) {
                return false;
            }
        }
    }
    true
}

/// One sync re-probe request: world slices plus the probe to answer.
#[derive(Debug, Clone, Copy)]
pub struct SyncRequest<'a> {
    /// Undirected poly edges.
    pub adj: &'a [(u32, u32)],
    /// Door clear per undirected edge, in cm.
    pub doors: &'a [(u32, u32, i32)],
    /// Ground blocker footprints.
    pub blockers: &'a [Blocker],
    /// Pegs under test.
    pub pegs: &'a [Peg],
    /// Actor-to-poly rows.
    pub at: &'a [(u32, u32)],
    /// Peg under test.
    pub peg_id: u32,
    /// Goal x in cm.
    pub gx_cm: i32,
    /// Goal z in cm.
    pub gz_cm: i32,
    /// Goal polygon, if any.
    pub goal_poly: Option<u32>,
}

/// Sync re-probe: exact door widths plus the corridor segment check.
/// Unknown pegs refuse with [`ProbeError`]; off-mesh goals read false.
pub fn sync_transfer(req: &SyncRequest<'_>) -> Result<bool, ProbeError> {
    check_doors(req.doors)?;
    let peg = req
        .pegs
        .iter()
        .find(|p| p.id == req.peg_id)
        .ok_or(ProbeError::UnknownPeg(req.peg_id))?;
    let (Some(a), Some(b)) = (poly_of(req.at, req.peg_id), req.goal_poly) else {
        return Ok(false);
    };
    if !bfs(req.adj, a, b, &|x, y| {
        door_width(req.doors, x, y) >= EXACT_DOOR_CM
    }) {
        return Ok(false);
    }
    Ok(segment_clear(
        peg.x_cm,
        peg.z_cm,
        req.gx_cm,
        req.gz_cm,
        req.blockers,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peg(id: u32, x: i32, z: i32, poly: Option<u32>) -> Peg {
        Peg {
            id,
            x_cm: x,
            z_cm: z,
            poly,
        }
    }

    fn open_world() -> World {
        World {
            adj: vec![(0, 1), (1, 2)],
            doors: vec![(0, 1, 200), (1, 2, 200)],
            at: vec![(10, 0), (11, 1), (12, 2)],
            pegs: vec![
                peg(10, -300, 0, Some(0)),
                peg(11, 0, 0, Some(1)),
                peg(12, 300, 0, Some(2)),
            ],
        }
    }

    struct World {
        adj: Vec<(u32, u32)>,
        doors: Vec<(u32, u32, i32)>,
        at: Vec<(u32, u32)>,
        pegs: Vec<Peg>,
    }

    fn narrow_world(width: i32) -> World {
        World {
            adj: vec![(0, 1), (1, 2)],
            doors: vec![(0, 1, width), (1, 2, 200)],
            at: vec![(10, 0), (11, 1)],
            pegs: vec![peg(10, -300, 0, Some(0)), peg(11, 0, 0, Some(1))],
        }
    }

    fn ask<'a>(
        world: &'a World,
        blockers: &'a [Blocker],
        peg_id: u32,
        gx: i32,
        gz: i32,
        goal: Option<u32>,
    ) -> SyncRequest<'a> {
        SyncRequest {
            adj: &world.adj,
            doors: &world.doors,
            blockers,
            pegs: &world.pegs,
            at: &world.at,
            peg_id,
            gx_cm: gx,
            gz_cm: gz,
            goal_poly: goal,
        }
    }

    #[test]
    fn coarse_ignores_narrow_door() {
        let world = narrow_world(70);
        assert!(coarse_reachable(&world.adj, &world.at, 10, Some(1)));
    }

    #[test]
    fn sync_blocks_narrow_door() {
        let world = narrow_world(70);
        let ok = sync_transfer(&ask(&world, &[], 10, 0, 0, Some(1))).expect("known peg answers");
        assert!(!ok);
        let open = open_world();
        let ok2 =
            sync_transfer(&ask(&open, &[], 10, 0, 0, Some(1))).expect("open reach answers true");
        assert!(ok2);
    }

    #[test]
    fn wide_is_lenient_on_mid_door() {
        let world = narrow_world(85);
        assert!(wide_reachable(
            &world.adj,
            &[(0, 1, 85), (1, 2, 200)],
            &world.at,
            10,
            Some(1)
        ));
        let ok = sync_transfer(&ask(&world, &[], 10, 0, 0, Some(1))).expect("narrow probe answers");
        assert!(!ok);
    }

    #[test]
    fn sync_blocks_walled_segment() {
        let world = open_world();
        let wall = Blocker {
            min_x: -100,
            max_x: 100,
            min_z: -200,
            max_z: 200,
        };
        let ok = sync_transfer(&ask(&world, &[wall], 10, 300, 0, Some(2)))
            .expect("walled probe answers");
        assert!(!ok);
        assert!(segment_clear(-300, 0, -250, 0, &[wall]));
        assert!(!segment_clear(-300, 0, 300, 0, &[wall]));
    }

    #[test]
    fn unknown_peg_refuses_with_code() {
        let world = open_world();
        let err = sync_transfer(&ask(&world, &[], 999, 0, 0, Some(1)))
            .expect_err("unknown peg must refuse");
        assert_eq!(err.code(), "unknown_peg");
        assert!(err.to_string().contains("999"));
        let bad_doors = vec![(0, 1, -5)];
        let bad = SyncRequest {
            adj: &world.adj,
            doors: &bad_doors,
            blockers: &[],
            pegs: &world.pegs,
            at: &world.at,
            peg_id: 10,
            gx_cm: 0,
            gz_cm: 0,
            goal_poly: Some(1),
        };
        let err2 = sync_transfer(&bad).expect_err("bad door must refuse");
        assert_eq!(err2.code(), "bad_door");
    }

    #[test]
    fn build_order_never_moves_answers() {
        let world = open_world();
        let mut adj_rev = world.adj.clone();
        adj_rev.reverse();
        assert_eq!(
            coarse_reachable(&world.adj, &world.at, 10, Some(2)),
            coarse_reachable(&adj_rev, &world.at, 10, Some(2))
        );
        let flipped = SyncRequest {
            adj: &adj_rev,
            doors: &world.doors,
            blockers: &[],
            pegs: &world.pegs,
            at: &world.at,
            peg_id: 10,
            gx_cm: 300,
            gz_cm: 0,
            goal_poly: Some(2),
        };
        assert_eq!(
            sync_transfer(&ask(&world, &[], 10, 300, 0, Some(2))),
            sync_transfer(&flipped)
        );
    }

    #[test]
    #[ignore = "timing: probe budget, run under --perf in release"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "test-only timing, never the tick"
    )]
    fn probe_budget_us_per_object() {
        let world = open_world();
        let total = 180u32;
        let start = std::time::Instant::now();
        let mut hits = 0u32;
        for _ in 0..total {
            let ok = sync_transfer(&ask(&world, &[], 10, 300, 0, Some(2))).expect("probe answers");
            hits += u32::from(ok);
        }
        assert_eq!(hits, total);
        let per_us = start.elapsed().as_micros() / u128::from(total);
        println!("probe: {per_us} us per object, hits={hits}");
        assert!(
            per_us <= BUDGET_US_PER_OBJECT,
            "probe budget 0.2 ms, measured {per_us} us"
        );
    }
}
