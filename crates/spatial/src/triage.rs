//! SPF8: triage findings by component role, z-fight ranking, unplayable first.
//!
//! Consumer: `scene.spatial` (lane A wires it; see
//! `docs/plan/spatial-entry-points.md`). Spatial knows no component types, so
//! the caller maps its `ComponentKind`s to [`Role`] and passes them in; no
//! decision here ever reads a name. Units: meters in, integer cm inside.
//! Determinism: every output is sorted by integer keys, so input order never
//! moves a byte.

use super::index::{pos_known, size_known, to_cm};
use super::{Actor, Finding};
use std::collections::{BTreeMap, BTreeSet};

/// What a node is for, as the caller read it off the node's components.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// Rendered geometry.
    Mesh,
    /// Physics collider, not drawn.
    Collider,
    /// Effect marker (spawner, light, emitter): no solid presence.
    Marker,
    /// Skeleton bone.
    Bone,
    /// Anything else.
    Other,
}

/// Verdict on one finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    /// Real problem: keeps its severity.
    Error,
    /// Meant to be like this (a collider wrapping its mesh, a seated joint).
    Intentional,
    /// Not a defect (bone against bone).
    FalsePositive,
    /// Park it: effect markers settle at runtime.
    Deferred,
}

impl Intent {
    /// The word an answer prints for this verdict.
    pub fn word(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Intentional => "intentional",
            Self::FalsePositive => "false_positive",
            Self::Deferred => "deferred",
        }
    }
}

/// A finding with its verdict.
#[derive(Debug, Clone, PartialEq)]
pub struct Triaged {
    pub finding: Finding,
    pub intent: Intent,
    /// Severity after triage: `info` for anything that is not an error.
    pub severity: &'static str,
    /// An error that stops play (overlap or sunken); sorted first.
    pub unplayable: bool,
}

/// A collider within this many cm of its mesh on every side wraps it.
const WRAP_CM: i32 = 5;

type Bounds = ([i32; 3], [i32; 3]);

fn bounds(a: &Actor) -> Option<Bounds> {
    if !(pos_known(a) && size_known(a) && a.sx > 0.0 && a.sy > 0.0 && a.sz > 0.0) {
        return None;
    }
    let half = [
        to_cm(a.sx).abs() / 2,
        to_cm(a.sy).abs() / 2,
        to_cm(a.sz).abs() / 2,
    ];
    let c = [to_cm(a.x), to_cm(a.y), to_cm(a.z)];
    Some((
        [c[0] - half[0], c[1] - half[1], c[2] - half[2]],
        [c[0] + half[0], c[1] + half[1], c[2] + half[2]],
    ))
}

fn wraps(outer: &Bounds, inner: &Bounds) -> bool {
    (0..3).all(|k| outer.0[k] - WRAP_CM <= inner.0[k] && inner.1[k] <= outer.1[k] + WRAP_CM)
}

pub(crate) fn colocated(x: &Actor, y: &Actor) -> bool {
    if x.parent == Some(y.id) || y.parent == Some(x.id) {
        return true;
    }
    match (bounds(x), bounds(y)) {
        (Some(bx), Some(by)) => wraps(&bx, &by) || wraps(&by, &bx),
        _ => false,
    }
}

fn judge(actors: &BTreeMap<u32, &Actor>, roles: &BTreeMap<u32, Role>, f: &Finding) -> Intent {
    let role = |id: u32| roles.get(&id).copied().unwrap_or(Role::Other);
    if f.check == "seatedJoint" {
        return Intent::Intentional;
    }
    if f.check != "overlap" || f.b == u32::MAX {
        return Intent::Error;
    }
    let (ra, rb) = (role(f.a), role(f.b));
    match (ra, rb) {
        (Role::Marker, _) | (_, Role::Marker) => Intent::Deferred,
        (Role::Bone, Role::Bone) => Intent::FalsePositive,
        (Role::Collider, Role::Mesh) | (Role::Mesh, Role::Collider) => {
            match (actors.get(&f.a), actors.get(&f.b)) {
                (Some(x), Some(y)) if colocated(x, y) => Intent::Intentional,
                _ => Intent::Error,
            }
        }
        _ => Intent::Error,
    }
}

/// Triage `findings` over `actors` with the caller's `roles`. Output order:
/// unplayable errors first, then the rest, each by `(check, a, b)`.
pub fn triage(actors: &[Actor], roles: &BTreeMap<u32, Role>, findings: &[Finding]) -> Vec<Triaged> {
    let by_id: BTreeMap<u32, &Actor> = actors.iter().map(|a| (a.id, a)).collect();
    let mut out: Vec<Triaged> = findings
        .iter()
        .map(|f| {
            let intent = judge(&by_id, roles, f);
            let error = intent == Intent::Error;
            Triaged {
                finding: f.clone(),
                intent,
                severity: if error { f.severity } else { "info" },
                unplayable: error
                    && matches!(f.check, "overlap" | "sunken" | "supportChain")
                    && (f.severity == "red" || f.check != "supportChain"),
            }
        })
        .collect();
    out.sort_by(|p, q| {
        let key = |t: &Triaged| (!t.unplayable, t.finding.check, t.finding.a, t.finding.b);
        key(p).cmp(&key(q))
    });
    out
}

/// Two meshes whose faces on one axis nearly coincide and flicker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZFight {
    pub a: u32,
    pub b: u32,
    /// Axis of the shared plane: 0 x, 1 y, 2 z.
    pub axis: usize,
    /// Distance between the two planes, cm.
    pub gap_cm: i32,
    /// Overlap of the two faces, cm^2.
    pub area_cm2: i64,
    /// Rank: area over (gap + 1), higher first.
    pub score: i64,
    /// Share of the smaller face the two faces cover together, percent.
    pub cover_pct: i32,
    /// Signed cm to move `b` along `axis` so the faces clear `epsilon_cm`.
    pub fix_cm: i32,
}

impl ZFight {
    /// One plain sentence: where the faces meet and the one move that fixes it.
    pub fn note(&self) -> String {
        let axis = ["x", "y", "z"][self.axis];
        format!(
            "faces {} cm apart on {axis}, {}% shared; move b {:+} cm on {axis}",
            self.gap_cm, self.cover_pct, self.fix_cm
        )
    }
}

/// Z-fight pairs among Mesh-role actors: some face of one lies within
/// `epsilon_cm` of the same-facing face of the other on one axis and the faces
/// overlap with positive area. Ranked by score, then `(a, b)`. `in_view`, when
/// given, keeps only pairs whose both ids are in the set (camera frustum).
pub fn z_fights(
    actors: &[Actor],
    roles: &BTreeMap<u32, Role>,
    epsilon_cm: i32,
    in_view: Option<&BTreeSet<u32>>,
) -> Vec<ZFight> {
    let mut meshes: Vec<(u32, Bounds)> = actors
        .iter()
        .filter(|a| roles.get(&a.id) == Some(&Role::Mesh))
        .filter(|a| in_view.is_none_or(|v| v.contains(&a.id)))
        .filter_map(|a| bounds(a).map(|b| (a.id, b)))
        .collect();
    meshes.sort_by_key(|m| m.0);
    let mut out = Vec::new();
    for (i, (ia, ba)) in meshes.iter().enumerate() {
        for (ib, bb) in &meshes[i + 1..] {
            let mut best: Option<ZFight> = None;
            for axis in 0..3 {
                let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
                let ou = (ba.1[u].min(bb.1[u]) - ba.0[u].max(bb.0[u])).max(0);
                let ov = (ba.1[v].min(bb.1[v]) - ba.0[v].max(bb.0[v])).max(0);
                let area = i64::from(ou) * i64::from(ov);
                if area == 0 {
                    continue;
                }
                let (d_lo, d_hi) = (bb.0[axis] - ba.0[axis], bb.1[axis] - ba.1[axis]);
                let near = if d_lo.abs() <= d_hi.abs() { d_lo } else { d_hi };
                let gap = near.abs();
                if gap > epsilon_cm {
                    continue;
                }
                let face = |b: &Bounds| i64::from(b.1[u] - b.0[u]) * i64::from(b.1[v] - b.0[v]);
                let away = if near < 0 { -1 } else { 1 };
                let score = area * 1000 / i64::from(gap + 1);
                let row = ZFight {
                    a: *ia,
                    b: *ib,
                    axis,
                    gap_cm: gap,
                    area_cm2: area,
                    score,
                    cover_pct: i32::try_from(area * 100 / face(ba).min(face(bb))).unwrap_or(100),
                    fix_cm: away * (epsilon_cm + 1 - gap),
                };
                if best.is_none_or(|b| row.score > b.score) {
                    best = Some(row);
                }
            }
            out.extend(best);
        }
    }
    out.sort_by_key(|z| (-z.score, z.a, z.b));
    out
}

/// [`z_fights`] with every box counted as a mesh, for callers that know no
/// component roles (the `zfight` check and its door notes).
pub fn z_fights_all(actors: &[Actor], epsilon_cm: i32) -> Vec<ZFight> {
    let meshes: BTreeMap<u32, Role> = actors.iter().map(|a| (a.id, Role::Mesh)).collect();
    z_fights(actors, &meshes, epsilon_cm, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::Rng;

    fn boxed(id: u32, lo: [i32; 3], size: [i32; 3]) -> Actor {
        let c = |k: usize| (lo[k] * 2 + size[k]) as f32 / 200.0;
        Actor {
            id,
            name: format!("n{id}"),
            x: c(0),
            y: c(1),
            z: c(2),
            sx: size[0] as f32 / 100.0,
            sy: size[1] as f32 / 100.0,
            sz: size[2] as f32 / 100.0,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        }
    }

    fn fnd(check: &'static str, a: u32, b: u32, severity: &'static str) -> Finding {
        Finding {
            check,
            a,
            b,
            pct: 100.0,
            severity,
        }
    }

    fn roles(pairs: &[(u32, Role)]) -> BTreeMap<u32, Role> {
        pairs.iter().copied().collect()
    }

    #[test]
    fn collider_on_its_mesh_is_intentional_and_everything_else_is_judged_by_role() {
        let actors = [
            boxed(1, [0, 0, 0], [100, 100, 100]),
            boxed(2, [2, 0, 2], [96, 100, 96]),
            boxed(3, [50, 0, 50], [100, 100, 100]),
            boxed(4, [0, 0, 0], [20, 20, 20]),
            boxed(5, [10, 0, 10], [20, 20, 20]),
        ];
        let r = roles(&[
            (1, Role::Mesh),
            (2, Role::Collider),
            (3, Role::Mesh),
            (4, Role::Bone),
            (5, Role::Bone),
        ]);
        let found = [
            fnd("overlap", 1, 2, "red"),
            fnd("overlap", 1, 3, "red"),
            fnd("overlap", 4, 5, "red"),
            fnd("overlap", 2, 3, "red"),
        ];
        let t = triage(&actors, &r, &found);
        let intent = |a, b| {
            t.iter()
                .find(|x| (x.finding.a, x.finding.b) == (a, b))
                .unwrap()
        };
        assert_eq!(intent(1, 2).intent, Intent::Intentional);
        assert_eq!(intent(1, 2).severity, "info");
        assert_eq!(
            intent(1, 3).intent,
            Intent::Error,
            "mesh against mesh stays red"
        );
        assert_eq!(intent(4, 5).intent, Intent::FalsePositive);
        assert_eq!(
            intent(2, 3).intent,
            Intent::Error,
            "a collider far from the mesh"
        );
        // Errors that stop play lead.
        assert!(t[0].unplayable && t[1].unplayable);
        assert!(t[2..].iter().all(|x| !x.unplayable));
        // A marker defers; a seated joint reads info.
        let mut r2 = r.clone();
        r2.insert(3, Role::Marker);
        let d = triage(
            &actors,
            &r2,
            &[
                fnd("overlap", 1, 3, "red"),
                fnd("seatedJoint", 5, 4, "yellow"),
            ],
        );
        assert_eq!(
            d.iter().find(|x| x.finding.a == 1).unwrap().intent,
            Intent::Deferred
        );
        assert_eq!(
            d.iter()
                .find(|x| x.finding.check == "seatedJoint")
                .unwrap()
                .severity,
            "info"
        );
    }

    #[test]
    fn triage_ignores_input_order() {
        let actors = [boxed(1, [0, 0, 0], [100; 3]), boxed(2, [0, 0, 0], [100; 3])];
        let r = roles(&[(1, Role::Mesh), (2, Role::Mesh)]);
        let mut found = vec![
            fnd("floating", 2, u32::MAX, "yellow"),
            fnd("overlap", 1, 2, "red"),
            fnd("sunken", 1, u32::MAX, "yellow"),
        ];
        let want = triage(&actors, &r, &found);
        found.reverse();
        assert_eq!(triage(&actors, &r, &found), want);
        assert_eq!(want[0].finding.check, "overlap");
    }

    #[test]
    fn z_fight_ranks_zero_gap_above_one_and_ignores_three() {
        let slab = |id, top: i32| boxed(id, [0, top - 10, 0], [200, 10, 200]);
        let actors = [
            slab(1, 100),
            slab(2, 100),
            slab(3, 101),
            boxed(4, [20, 93, 20], [200, 10, 200]),
        ];
        let r: BTreeMap<u32, Role> = (1..=4).map(|i| (i, Role::Mesh)).collect();
        let rows = z_fights(&actors, &r, 1, None);
        let pair = |a, b| rows.iter().position(|z| (z.a, z.b) == (a, b));
        assert_eq!((rows[0].a, rows[0].b, rows[0].gap_cm), (1, 2, 0));
        assert!(pair(1, 2) < pair(1, 3), "0 cm ranks above 1 cm");
        assert!(
            pair(1, 4).is_none() && pair(3, 4).is_none(),
            "3 cm and 2 cm are clear"
        );
        // The frustum filter drops pairs with an unseen member.
        let view: BTreeSet<u32> = [1, 3].into_iter().collect();
        let seen = z_fights(&actors, &r, 1, Some(&view));
        assert_eq!(seen.len(), 1);
        // Colliders are not drawn, so they never fight.
        let mut colliders = r.clone();
        colliders.insert(2, Role::Collider);
        assert!(z_fights(&actors, &colliders, 1, None)
            .iter()
            .all(|z| z.a != 2 && z.b != 2));
    }

    /// Oracle: list all six planes of each box and compare same-facing planes.
    fn brute(actors: &[Actor], eps: i32) -> BTreeSet<(u32, u32)> {
        let mut set = BTreeSet::new();
        for x in actors {
            for y in actors.iter().filter(|y| y.id > x.id) {
                let (bx, by) = (bounds(x).unwrap(), bounds(y).unwrap());
                let hit = (0..3).any(|k| {
                    let (u, v) = ((k + 1) % 3, (k + 2) % 3);
                    let over = |d: usize| bx.1[d].min(by.1[d]) > bx.0[d].max(by.0[d]);
                    over(u)
                        && over(v)
                        && [(bx.0[k], by.0[k]), (bx.1[k], by.1[k])]
                            .iter()
                            .any(|(p, q)| (p - q).abs() <= eps)
                });
                if hit {
                    set.insert((x.id, y.id));
                }
            }
        }
        set
    }

    #[test]
    fn z_fights_match_the_plane_list_oracle_on_seeded_scenes() {
        let mut rng = Rng::new(0x2f16);
        let mut seen = 0;
        for case in 0..100 {
            let actors: Vec<Actor> = (0..7)
                .map(|i| {
                    let mut r = |m: u32| (rng.next_u32() % m) as i32;
                    boxed(
                        i + 1,
                        [r(6) * 5, r(4) * 5, r(6) * 5],
                        [(1 + r(4)) * 10, (1 + r(3)) * 10, (1 + r(4)) * 10],
                    )
                })
                .collect();
            let r: BTreeMap<u32, Role> = actors.iter().map(|a| (a.id, Role::Mesh)).collect();
            let got: BTreeSet<(u32, u32)> = z_fights(&actors, &r, 1, None)
                .iter()
                .map(|z| (z.a, z.b))
                .collect();
            let want = brute(&actors, 1);
            seen += want.len();
            assert_eq!(got, want, "case {case}");
            assert_eq!(
                z_fights(&actors, &r, 1, None),
                z_fights(&actors, &r, 1, None)
            );
        }
        assert!(seen > 50, "{seen}");
    }
}
