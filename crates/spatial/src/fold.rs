//! SPF10: brief hygiene. Fold look-alikes into one counted row, absorb
//! colliders into the mesh they wrap, and say the top problems in words.
//!
//! Consumer: `scene.brief` (lane A wires it beside SPR1's per-object rows; see
//! `docs/plan/spatial-entry-points.md`). Pure functions over a plain actor
//! snapshot, so the brief module itself is untouched. Units: meters in,
//! integer cm out. Determinism: id-sorted groups, integer keys only.

use super::extent::{robust_extent, Excluded};
use super::index::to_cm;
use super::triage::{colocated, Role};
use super::{Actor, Report};
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// One row of look-alikes: same name stem, parent, physical flag and size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folded {
    /// Name with its trailing counter removed (`crate_07` -> `crate`).
    pub stem: String,
    /// Member ids, ascending.
    pub ids: Vec<u32>,
    /// Footprint size `[x, y, z]` in cm.
    pub size_cm: [i32; 3],
    /// Plan-view area of the members together, cm^2.
    pub area_cm2: i64,
    /// Colliders absorbed into this row because they wrap a member.
    pub colliders: u32,
}

/// `crate_07` -> `crate`, `torch1` -> `torch`, `wall` -> `wall`.
pub(crate) fn stem(name: &str) -> &str {
    let t = name
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .trim_end_matches(['_', '-', ' ']);
    if t.is_empty() {
        name
    } else {
        t
    }
}

/// Fold look-alikes. A collider-role actor that wraps a mesh-role actor is
/// counted on the mesh's row and gets no row of its own.
pub fn fold_lookalikes(actors: &[Actor], roles: &BTreeMap<u32, Role>) -> Vec<Folded> {
    let role = |id: u32| roles.get(&id).copied().unwrap_or(Role::Other);
    let meshes: Vec<&Actor> = actors.iter().filter(|a| role(a.id) == Role::Mesh).collect();
    let mut absorbed: BTreeMap<u32, u32> = BTreeMap::new();
    let mut gone = std::collections::BTreeSet::new();
    for c in actors.iter().filter(|a| role(a.id) == Role::Collider) {
        if let Some(m) = meshes
            .iter()
            .filter(|m| colocated(c, m))
            .map(|m| m.id)
            .min()
        {
            *absorbed.entry(m).or_insert(0) += 1;
            gone.insert(c.id);
        }
    }
    type Key = (String, Option<u32>, bool, [i32; 3]);
    let mut groups: BTreeMap<Key, Vec<&Actor>> = BTreeMap::new();
    for a in actors.iter().filter(|a| !gone.contains(&a.id)) {
        let size = [a.sx, a.sy, a.sz].map(to_cm);
        groups
            .entry((stem(&a.name).to_owned(), a.parent, a.physical, size))
            .or_default()
            .push(a);
    }
    let mut rows: Vec<Folded> = groups
        .into_iter()
        .map(|((stem, _, _, size_cm), mut members)| {
            members.sort_by_key(|a| a.id);
            Folded {
                stem,
                size_cm,
                area_cm2: i64::from(size_cm[0].abs())
                    * i64::from(size_cm[2].abs())
                    * i64::try_from(members.len()).unwrap_or(0),
                colliders: members
                    .iter()
                    .map(|a| absorbed.get(&a.id).copied().unwrap_or(0))
                    .sum(),
                ids: members.iter().map(|a| a.id).collect(),
            }
        })
        .collect();
    rows.sort_by_key(|r| r.ids[0]);
    rows
}

/// Rows as brief text, one line each: `crate x40 100x100x100 cm [ids 1..40]`.
pub fn fold_text(rows: &[Folded]) -> String {
    let mut out = String::new();
    for r in rows {
        let (first, last) = (r.ids[0], r.ids[r.ids.len() - 1]);
        let _ = write!(out, "{}", r.stem);
        if r.ids.len() > 1 {
            let _ = write!(out, " x{}", r.ids.len());
        }
        let _ = write!(
            out,
            " {}x{}x{} cm [{first}",
            r.size_cm[0], r.size_cm[1], r.size_cm[2]
        );
        if r.ids.len() > 1 {
            let _ = write!(out, "..{last}");
        }
        if r.colliders > 0 {
            let _ = write!(out, ", {} collider(s) folded", r.colliders);
        }
        out.push_str("]\n");
    }
    out
}

/// The stated extent line: robust extent plus what it left out and why.
pub fn extent_text(actors: &[Actor]) -> Option<String> {
    let e = robust_extent(actors)?;
    let mut out = format!(
        "extent cm [{},{},{}]..[{},{},{}]",
        e.min_cm[0], e.min_cm[1], e.min_cm[2], e.max_cm[0], e.max_cm[1], e.max_cm[2]
    );
    for Excluded { id, why, ratio } in &e.excluded {
        let _ = write!(out, "; excluded {id} why={why} x{ratio}");
    }
    if !e.excluded.is_empty() {
        let _ = write!(
            out,
            "; inflation x{}.{}",
            e.inflation_x10 / 10,
            e.inflation_x10 % 10
        );
    }
    Some(out)
}

/// The top `limit` problems as plain sentences: reds first, then by how many
/// findings share the check, then check name.
pub fn problems_in_words(actors: &[Actor], report: &Report, limit: usize) -> Vec<String> {
    let name_of = |id: u32| {
        actors
            .iter()
            .find(|a| a.id == id)
            .map_or("?", |a| a.name.as_str())
    };
    let mut by_check: BTreeMap<&str, Vec<&super::Finding>> = BTreeMap::new();
    for f in &report.findings {
        by_check.entry(f.check).or_default().push(f);
    }
    let mut groups: Vec<(&str, Vec<&super::Finding>)> = by_check.into_iter().collect();
    groups
        .sort_by_key(|(check, fs)| (fs[0].severity != "red", std::cmp::Reverse(fs.len()), *check));
    groups
        .into_iter()
        .take(limit)
        .map(|(check, fs)| {
            let first = fs[0];
            let at = if first.b == u32::MAX {
                format!("{} [{}]", name_of(first.a), first.a)
            } else {
                format!(
                    "{} [{}] and {} [{}]",
                    name_of(first.a),
                    first.a,
                    name_of(first.b),
                    first.b
                )
            };
            format!(
                "{} {check} finding(s), {}, first at {at}",
                fs.len(),
                first.severity
            )
        })
        .collect()
}

#[cfg(test)]
mod brief_fold {
    use super::*;

    fn crate_n(id: u32, x: f32) -> Actor {
        Actor {
            id,
            name: format!("crate_{id:02}"),
            x,
            y: 0.5,
            z: 0.0,
            sx: 1.0,
            sy: 1.0,
            sz: 1.0,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        }
    }

    #[test]
    fn forty_identical_crates_fold_to_one_row() {
        let crates: Vec<Actor> = (1..=40).map(|i| crate_n(i, i as f32 * 2.0)).collect();
        let rows = fold_lookalikes(&crates, &BTreeMap::new());
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].stem.as_str(), rows[0].ids.len(), rows[0].area_cm2),
            ("crate", 40, 400_000)
        );
        assert_eq!(fold_text(&rows), "crate x40 100x100x100 cm [1..40]\n");
        // A different size or parent is another row; order never matters.
        let mut mixed = crates.clone();
        mixed[3].sx = 2.0;
        mixed[5].parent = Some(1);
        let want = fold_lookalikes(&mixed, &BTreeMap::new());
        assert_eq!(want.len(), 3);
        mixed.reverse();
        assert_eq!(fold_lookalikes(&mixed, &BTreeMap::new()), want);
    }

    #[test]
    fn a_collider_wrapping_its_mesh_folds_into_the_mesh_row() {
        let mesh = crate_n(1, 0.0);
        let mut collider = crate_n(2, 0.0);
        collider.name = "crate_01_collider".into();
        let roles: BTreeMap<u32, Role> =
            [(1, Role::Mesh), (2, Role::Collider)].into_iter().collect();
        let rows = fold_lookalikes(&[mesh, collider], &roles);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].colliders, 1);
        assert_eq!(
            fold_text(&rows),
            "crate 100x100x100 cm [1, 1 collider(s) folded]\n"
        );
    }

    #[test]
    fn a_500_m_stray_is_excluded_with_its_inflation() {
        let mut scene: Vec<Actor> = (1..=30).map(|i| crate_n(i, (i % 6) as f32 * 2.0)).collect();
        let near = extent_text(&scene).unwrap();
        assert!(!near.contains("excluded"));
        let far = crate_n(99, 500.0);
        scene.push(far);
        let text = extent_text(&scene).unwrap();
        assert!(text.contains("excluded 99 why=stray"), "{text}");
        assert!(text.contains("inflation x"), "{text}");
    }

    #[test]
    fn the_115_object_scene_stays_inside_the_default_budget() {
        let mut scene: Vec<Actor> = (1..=100)
            .map(|i| crate_n(i, (i % 10) as f32 * 3.0))
            .collect();
        for i in 101..=115 {
            let mut a = crate_n(i, 0.0);
            a.name = format!("lamp_{i}");
            a.sy = 2.0;
            a.z = (i - 100) as f32 * 3.0;
            scene.push(a);
        }
        let text = fold_text(&fold_lookalikes(&scene, &BTreeMap::new()));
        assert!(text.lines().count() <= 3, "{text}");
        assert!(text.len() < 4096);
    }

    /// 100 crates on a 3 m grid, 15 tall lamps and one crate 500 m away.
    fn scene_115() -> Vec<Actor> {
        let mut scene: Vec<Actor> = (1..=100)
            .map(|i| Actor {
                z: (i / 10) as f32 * 3.0,
                ..crate_n(i, (i % 10) as f32 * 3.0)
            })
            .collect();
        for i in 101..=115 {
            scene.push(Actor {
                name: format!("lamp_{i}"),
                sy: 2.0,
                z: 40.0 + (i - 100) as f32 * 3.0,
                ..crate_n(i, 0.0)
            });
        }
        scene.push(crate_n(116, 500.0));
        scene
    }

    fn door_brief(scene: &[Actor], budget: usize) -> crate::brief::FoldedBrief {
        let (graph, report) = (crate::build(scene), crate::run(scene));
        crate::brief::brief_folded(scene, &graph, &report, &BTreeMap::new(), budget)
    }

    #[test]
    fn brief_fold_115_objects_fit_the_default_budget_in_any_order() {
        let scene = scene_115();
        let folded = door_brief(&scene, 4096);
        let text = &folded.text;
        assert!(text.len() <= 4096, "{} bytes", text.len());
        assert!(text.starts_with("digest "), "{text}");
        assert!(
            text.contains("object crate x101 100x100x100 cm [1..116]"),
            "{text}"
        );
        assert!(
            text.contains("object lamp x15 100x200x100 cm [101..115]"),
            "{text}"
        );
        assert!(text.contains("excluded 116 why=stray"), "{text}");
        assert_eq!(text.lines().filter(|l| l.starts_with("object ")).count(), 2);
        // The unfolded brief spends one row on every object.
        let plain = crate::brief::brief_text(&crate::brief::brief(
            &scene,
            &crate::build(&scene),
            &crate::run(&scene),
            usize::MAX,
        ));
        assert!(plain.lines().filter(|l| l.starts_with("object ")).count() == 116);
        let mut reversed = scene.clone();
        reversed.reverse();
        assert_eq!(
            door_brief(&reversed, 4096),
            folded,
            "same bytes in any order"
        );
    }

    #[test]
    fn brief_fold_cuts_whole_rows_and_always_keeps_the_digest() {
        let scene = scene_115();
        let full = door_brief(&scene, usize::MAX);
        assert!(!full.truncated && full.findings_cut == 0);
        let digest_row = full.text.lines().next().unwrap();
        for budget in [0, digest_row.len(), digest_row.len() + 40, 300, 1000] {
            let tight = door_brief(&scene, budget);
            assert!(tight.text.starts_with(digest_row), "budget {budget}");
            assert!(
                tight.text.len() <= budget.max(digest_row.len() + 1),
                "budget {budget}: {} bytes",
                tight.text.len()
            );
            assert!(tight.text.ends_with('\n'));
        }
        assert_eq!(door_brief(&scene, 0).text, format!("{digest_row}\n"));
        let crowded: Vec<Actor> = (1..=6).map(|i| crate_n(i, 0.0)).collect();
        let tight = door_brief(&crowded, 200);
        assert!(tight.truncated && tight.findings_cut > 0, "{tight:?}");
    }

    #[test]
    fn brief_fold_names_the_problems_before_the_rows() {
        let crowded: Vec<Actor> = (1..=3).map(|i| crate_n(i, 0.0)).collect();
        let text = door_brief(&crowded, 4096).text;
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].starts_with("digest "), "{text}");
        let problem = lines
            .iter()
            .position(|l| l.starts_with("problem:"))
            .unwrap();
        let row = lines.iter().position(|l| l.starts_with("object ")).unwrap();
        assert!(problem < row, "{text}");
        assert!(lines[problem].contains("overlap") && lines[problem].contains("red"));
    }

    #[test]
    fn problems_come_out_in_words_reds_first() {
        let scene = vec![crate_n(1, 0.0), crate_n(2, 0.0), crate_n(3, 0.0)];
        let report = crate::run(&scene);
        let said = problems_in_words(&scene, &report, 5);
        assert!(said[0].contains("overlap"), "{said:?}");
        assert!(said[0].contains("red"));
        assert!(said.len() <= 5);
        assert_eq!(said, problems_in_words(&scene, &report, 5), "x2");
    }
}
