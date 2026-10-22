use super::*;

/// Room shell: four thin grounded walls forming a 10 m square, the SP5
/// room convention. Non-physical so the nav gate skips them while the
/// overlap gate still covers them; corners touch at most, never overlap.
pub fn room_shell() -> Vec<Actor> {
    let wall = |id: u32, name: &str, x: f32, z: f32, sx: f32, sz: f32| Actor {
        id,
        name: name.into(),
        x,
        y: 1.0,
        z,
        sx,
        sy: 2.0,
        sz,
        parent: None,
        physical: false,
        vx: 0.0,
        vz: 0.0,
        yaw: 0.0,
    };
    vec![
        wall(SHELL_BASE, "wall_north", 0.0, -5.0, 9.8, 0.2),
        wall(SHELL_BASE + 1, "wall_south", 0.0, 5.0, 9.8, 0.2),
        wall(SHELL_BASE + 2, "wall_west", -5.0, 0.0, 0.2, 10.0),
        wall(SHELL_BASE + 3, "wall_east", 5.0, 0.0, 0.2, 10.0),
    ]
}

pub(super) fn is_shell(id: u32) -> bool {
    id >= SHELL_BASE
}

pub(super) fn stage_actor(plan: &PlannedProp, slot: usize) -> Actor {
    let (_, sy, _) = kind_size(&plan.kind).unwrap_or((1.0, 1.0, 1.0));
    let (sx, _, sz) = kind_size(&plan.kind).unwrap_or((1.0, 1.0, 1.0));
    // A floor plate is the ground the level stands on: it never routes, never moves and
    // never counts as a prop the solver can place. It sits at the room centre, flat, with
    // its top at ground level so the staged gate reads it as floor and not as a float.
    let floor = role_of(&plan.kind) == Role::Floor;
    Actor {
        id: plan.id,
        name: plan.name.clone(),
        x: if floor {
            0.0
        } else {
            -4.0 + (slot % 6) as f32 * 1.6
        },
        y: if floor { -sy * 0.5 } else { sy * 0.5 },
        z: if floor {
            0.0
        } else {
            3.2 - (slot / 6) as f32 * 1.6
        },
        sx,
        sy,
        sz,
        parent: None,
        physical: !floor,
        vx: 0.0,
        vz: 0.0,
        yaw: 0.0,
    }
}

pub(super) fn actor_with(actor: &Actor, solved: &engine_spatial::TransformOut) -> Actor {
    let mut next = actor.clone();
    next.x = solved.x;
    next.y = solved.y;
    next.z = solved.z;
    next.yaw = solved.yaw;
    next
}

/// Solve every planned prop against the shell plus the current positions.
/// Passes run in name order until every prop settles or no pass makes
/// progress; returns the actor list, the still unsolved ids in ascending
/// order and, per still-unsolved prop, the SP5 minimal conflict set from its
/// last failed solve (the refusal's conflict-set evidence). A prop that
/// solves on a later pass leaves no conflict entry.
pub(super) fn solve_plan(
    plan: &RelationPlan,
    fixed: &[Actor],
    solve_calls: &mut u32,
) -> (
    Vec<Actor>,
    BTreeSet<u32>,
    BTreeMap<u32, engine_spatial::ConflictSet>,
) {
    let mut actors: Vec<Actor> = room_shell();
    actors.extend_from_slice(fixed);
    let staged: Vec<Actor> = plan
        .props
        .iter()
        .enumerate()
        .map(|(slot, prop)| stage_actor(prop, slot))
        .collect();
    actors.extend(staged);
    let mut solved: BTreeSet<u32> = BTreeSet::new();
    let mut conflicts: BTreeMap<u32, engine_spatial::ConflictSet> = BTreeMap::new();
    // A floor plate is already where it belongs (the room centre, flat): it needs no
    // solve, so it never appears as unsolved or as a culprit.
    for prop in &plan.props {
        if role_of(&prop.kind) == Role::Floor {
            solved.insert(prop.id);
        }
    }
    for _ in 0..PASSES_MAX {
        let mut progress = false;
        for prop in &plan.props {
            if solved.contains(&prop.id) {
                continue;
            }
            let intent = Intent {
                object: ObjectRef::Id(prop.id),
                wants: prop.wants.clone(),
            };
            *solve_calls += 1;
            match engine_spatial::solve(&intent, &actors) {
                Ok(out) => {
                    if let Some(first) = out.first() {
                        if let Some(slot) = actors.iter().position(|a| a.id == prop.id) {
                            actors[slot] = actor_with(&actors[slot], first);
                        }
                        solved.insert(prop.id);
                        progress = true;
                    }
                }
                // Last failed attempt wins: the conflict set of the pose the
                // solver finished with names the wants it could not satisfy.
                Err(conflict) => {
                    conflicts.insert(prop.id, conflict);
                }
            }
        }
        if solved.len() == plan.props.len() || !progress {
            break;
        }
    }
    let unsolved: BTreeSet<u32> = plan
        .props
        .iter()
        .map(|prop| prop.id)
        .filter(|id| !solved.contains(id))
        .collect();
    conflicts.retain(|id, _| unsolved.contains(id));
    (actors, unsolved, conflicts)
}

pub(super) fn tile_of(x_cm: i32, z_cm: i32) -> (i32, i32) {
    (x_cm.div_euclid(NAV_TILE_CM), z_cm.div_euclid(NAV_TILE_CM))
}

/// Integer-cm rect `(min_x, max_x, min_z, max_z)`.
pub(super) type Rect4 = (i32, i32, i32, i32);

/// Shell wall rects plus the room interior (the union bounds).
/// Deterministic, integer only.
pub(super) fn shell_shapes(actors: &[Actor]) -> (Vec<Rect4>, Rect4) {
    let mut rects = Vec::new();
    for actor in actors {
        if !is_shell(actor.id) {
            continue;
        }
        let hx = cm(actor.sx).abs() / 2;
        let hz = cm(actor.sz).abs() / 2;
        rects.push((
            cm(actor.x) - hx,
            cm(actor.x) + hx,
            cm(actor.z) - hz,
            cm(actor.z) + hz,
        ));
    }
    let mut interior = (i32::MAX, i32::MIN, i32::MAX, i32::MIN);
    for (min_x, max_x, min_z, max_z) in &rects {
        interior.0 = interior.0.min(*min_x);
        interior.1 = interior.1.max(*max_x);
        interior.2 = interior.2.min(*min_z);
        interior.3 = interior.3.max(*max_z);
    }
    (rects, interior)
}

pub(super) fn inside(px: i32, pz: i32, rect: &Rect4) -> bool {
    rect.0 <= px && px <= rect.1 && rect.2 <= pz && pz <= rect.3
}

/// Bake the cell layout plus its nav view: one tile per square meter over
/// the room plus an outer ring. A tile is walkable when its center stands
/// on the room floor, clear of every shell wall; wall-adjacent placements
/// (the normal SP5 case) stay walkable while the bands still bound the
/// room. Props stay walkable by design (furniture-scale; crowd steering
/// around them is the NV2 crowd layer, never this bake). This feeds the
/// same `NavView` shape the NV1 mesh bake produces; the WD1 cell streamer
/// owns async publish, and `cell_bytes` below is the bake input it reads.
/// Wall bands block movement edges in `routes`, never tiles here, so the
/// shared `navConnect` check verifies floor coverage while the route
/// proof below verifies real reachability.
pub fn bake_nav(actors: &[Actor], spawn_id: u32) -> NavView {
    let (shells, interior) = shell_shapes(actors);
    let mut tiles: BTreeMap<(i32, i32), Tile> = BTreeMap::new();
    for tx in -NAV_HALF..=NAV_HALF {
        for tz in -NAV_HALF..=NAV_HALF {
            let (cx, cz) = (
                tx * NAV_TILE_CM + NAV_TILE_CM / 2,
                tz * NAV_TILE_CM + NAV_TILE_CM / 2,
            );
            let in_room =
                interior.0 <= cx && cx <= interior.1 && interior.2 <= cz && cz <= interior.3;
            let clear = !shells.iter().any(|rect| inside(cx, cz, rect));
            tiles.insert(
                (tx, tz),
                Tile {
                    walk: in_room && clear,
                },
            );
        }
    }
    NavView {
        tiles,
        links: Vec::new(),
        flow: BTreeMap::new(),
        doors: BTreeMap::new(),
        spawn_id,
    }
}

/// One navigable route: BFS tiles from the spawn tile to a prop tile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteInfo {
    /// Prop actor id.
    pub id: u32,
    /// Prop name from the brief.
    pub name: String,
    /// True when the prop tile is reachable from the spawn tile.
    pub reachable: bool,
    /// BFS distance in tiles (`u32::MAX` when unreachable).
    pub tiles: u32,
}

/// BFS routes from the spawn tile over walkable tiles. A step between two
/// tiles is blocked when the shared edge midpoint stands inside a shell
/// wall band, so thin walls seal routes even though tiles stay floor
/// tiles. A missing or unwalkable spawn marks every route unreachable;
/// distances are exact.
pub fn routes(nav: &NavView, actors: &[Actor]) -> Vec<RouteInfo> {
    let id_of: BTreeMap<u32, &Actor> = actors.iter().map(|a| (a.id, a)).collect();
    let spawn_tile = id_of.get(&nav.spawn_id).map(|a| tile_of(cm(a.x), cm(a.z)));
    let (shells, _) = shell_shapes(actors);
    let walkable = |tile: &(i32, i32)| nav.tiles.get(tile).map(|t| t.walk).unwrap_or(false);
    let edge_open = |from: &(i32, i32), to: &(i32, i32)| {
        let mid = (
            from.0 * NAV_TILE_CM + (to.0 - from.0) * NAV_TILE_CM / 2 + NAV_TILE_CM / 2,
            from.1 * NAV_TILE_CM + (to.1 - from.1) * NAV_TILE_CM / 2 + NAV_TILE_CM / 2,
        );
        !shells.iter().any(|rect| inside(mid.0, mid.1, rect))
    };
    let mut dist: BTreeMap<(i32, i32), u32> = BTreeMap::new();
    if let Some(start) = spawn_tile {
        if walkable(&start) {
            let mut queue = VecDeque::from([start]);
            dist.insert(start, 0);
            while let Some(tile) = queue.pop_front() {
                let here = dist[&tile];
                for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let next = (tile.0 + dx, tile.1 + dz);
                    if dist.contains_key(&next) {
                        continue;
                    }
                    if !walkable(&next) || !edge_open(&tile, &next) {
                        continue;
                    }
                    dist.insert(next, here + 1);
                    queue.push_back(next);
                }
            }
        }
    }
    let mut infos: Vec<RouteInfo> = actors
        .iter()
        .filter(|a| !is_shell(a.id) && a.id < LIVE_BASE && a.physical)
        .map(|a| {
            let tile = tile_of(cm(a.x), cm(a.z));
            match dist.get(&tile).copied() {
                Some(tiles) => RouteInfo {
                    id: a.id,
                    name: a.name.clone(),
                    reachable: true,
                    tiles,
                },
                None => RouteInfo {
                    id: a.id,
                    name: a.name.clone(),
                    reachable: false,
                    tiles: u32::MAX,
                },
            }
        })
        .collect();
    infos.sort_by_key(|info| info.id);
    infos
}

pub(super) struct GateReport {
    pub(super) findings: Vec<Finding>,
    pub(super) unsolved: BTreeSet<u32>,
    pub(super) nav: NavView,
    pub(super) routes: Vec<RouteInfo>,
}

/// The 14 offline checks with `giantObject` judged against the room. The
/// shell walls are non-physical (the nav bake skips them), and `giantObject`
/// measures a solid against the span of the other solid boxes, so with the
/// walls left out a 1.6 m torch beside one 0.8 m crate is a "giant" in a
/// layout of two props. The room IS the layout: that one check reads a copy
/// of the actors with the shell solid, so props are measured against the
/// 10 m room and a stray oversized live-document box still flags. Findings
/// of the other checks keep their frozen order; giants append last.
fn offline_findings(actors: &[Actor]) -> Vec<Finding> {
    let mut findings = engine_spatial::run(actors).findings;
    findings.retain(|finding| finding.check != "giantObject");
    let mut roomed = actors.to_vec();
    for actor in roomed.iter_mut().filter(|actor| is_shell(actor.id)) {
        actor.physical = true;
    }
    if let Ok(report) = engine_spatial::run_checks(&roomed, &["giantObject"]) {
        findings.extend(report.findings);
    }
    findings
}

/// Full gate: the 14 offline checks plus `navConnect`, `doorClear` and
/// `flowPath` over the baked view, plus per-prop BFS routes. `accessFront`
/// stays out: yaw is solver-chosen, and facing gates belong to encounter
/// staging, never level layout.
pub(super) fn gates(actors: &[Actor], spawn_id: u32, unsolved: &BTreeSet<u32>) -> GateReport {
    let nav = bake_nav(actors, spawn_id);
    let mut findings = offline_findings(actors);
    if let Ok(nav_report) =
        engine_spatial::run_spatial(actors, &["navConnect", "doorClear", "flowPath"], Some(&nav))
    {
        findings.extend(nav_report.findings);
    }
    let routes = routes(&nav, actors);
    GateReport {
        findings,
        unsolved: unsolved.clone(),
        nav,
        routes,
    }
}

pub(super) fn gates_clean(report: &GateReport) -> bool {
    report.findings.is_empty()
        && report.unsolved.is_empty()
        && report.routes.iter().all(|route| route.reachable)
}

/// First prop id behind a gate failure: the earliest finding touching a
/// planned prop, else the first unreachable prop, else the first unsolved
/// prop. Live-document and shell actors are never culprits: the repair
/// loop moves brief props only.
pub(super) fn culprit(report: &GateReport, plan: &RelationPlan) -> Option<u32> {
    let owned: BTreeSet<u32> = plan.props.iter().map(|prop| prop.id).collect();
    for finding in &report.findings {
        if owned.contains(&finding.a) {
            return Some(finding.a);
        }
        if finding.b != u32::MAX && owned.contains(&finding.b) {
            return Some(finding.b);
        }
    }
    if let Some(route) = report.routes.iter().find(|route| !route.reachable) {
        return Some(route.id);
    }
    report.unsolved.iter().copied().next()
}

pub(super) fn resolve_single(
    actors: &mut [Actor],
    prop_id: u32,
    wants: &[Want],
    conflict: &mut Option<engine_spatial::ConflictSet>,
    solve_calls: &mut u32,
) -> bool {
    let intent = Intent {
        object: ObjectRef::Id(prop_id),
        wants: wants.to_vec(),
    };
    *solve_calls += 1;
    match engine_spatial::solve(&intent, actors) {
        Ok(out) => {
            *conflict = None;
            if let (Some(first), Some(slot)) =
                (out.first(), actors.iter().position(|a| a.id == prop_id))
            {
                actors[slot] = actor_with(&actors[slot], first);
                return true;
            }
            false
        }
        Err(set) => {
            *conflict = Some(set);
            false
        }
    }
}

/// One repair round over the culprit prop. Round 0 drops its last want,
/// round 1 resets every range to the solver default, round 2 relocates it
/// to the first clean cell; the last prop is never removed, so a level
/// keeps at least one prop and a stuck single-prop brief refuses instead
/// of voiding itself. Every applied round counts, even a failed re-solve.
/// A re-solved culprit leaves the `unsolved` set and its conflict entry; a
/// failed re-solve refreshes the SP5 conflict set the refusal reports.
pub(super) fn repair_round(
    plan: &mut RelationPlan,
    actors: &mut [Actor],
    round: u8,
    culprit: u32,
    unsolved: &mut BTreeSet<u32>,
    conflicts: &mut BTreeMap<u32, engine_spatial::ConflictSet>,
    solve_calls: &mut u32,
) -> bool {
    let multi = plan.props.len() > 1;
    let Some(entry) = plan.props.iter_mut().find(|prop| prop.id == culprit) else {
        return false;
    };
    match round {
        0 => {
            if entry.wants.is_empty() {
                return false;
            }
            entry.wants.pop();
            entry.sources.pop();
            let wants = entry.wants.clone();
            let mut conflict = None;
            if resolve_single(actors, culprit, &wants, &mut conflict, solve_calls) {
                unsolved.remove(&culprit);
                conflicts.remove(&culprit);
            } else if let Some(set) = conflict {
                conflicts.insert(culprit, set);
            }
            true
        }
        1 => {
            for want in &mut entry.wants {
                want.distance_cm = None;
            }
            let wants = entry.wants.clone();
            let mut conflict = None;
            if resolve_single(actors, culprit, &wants, &mut conflict, solve_calls) {
                unsolved.remove(&culprit);
                conflicts.remove(&culprit);
            } else if let Some(set) = conflict {
                conflicts.insert(culprit, set);
            }
            true
        }
        _ => {
            if !multi {
                return true;
            }
            entry.wants.clear();
            entry.sources.clear();
            let mut conflict = None;
            if resolve_single(actors, culprit, &[], &mut conflict, solve_calls) {
                unsolved.remove(&culprit);
                conflicts.remove(&culprit);
            } else if let Some(set) = conflict {
                conflicts.insert(culprit, set);
            }
            true
        }
    }
}
