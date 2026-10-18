//! SPF1 known-good scene gate with negative controls.
//!
//! Consumer: `cargo test -p engine-playtest --lib known_good` (the gate that
//! stops a spatial check from crying wolf on shipped content or going silent
//! on a real scene). Data owner: the shipped scene files under `content/`;
//! `engine-spatial` owns the checks. Method (Forge `SpatialRomeLiveTests`):
//! every shipped scene must give zero red findings and exactly the pinned
//! yellow list; every check of the catalog must still fire on a one-edit
//! mutant of a real scene, and boundary near-misses must stay silent.
//! Determinism: same files give the same rows, in any actor order.
//! Failure mode: a drifted scene or a changed check shows up as a row diff.

use engine_spatial::{run_spatial, Actor, Door, NavView, Report, Tile, SPATIAL_CHECKS};
use std::collections::BTreeSet;
use std::path::Path;

/// Shipped scenes: gate name and the files whose actors are concatenated.
/// `fixtures/games/*` hold rules only (no actors), so they are not scenes.
const SCENES: [(&str, &[&str]); 6] = [
    ("arena", &["content/arena/placement.json"]),
    (
        "arena_dressed",
        &[
            "content/arena/placement.json",
            "content/arena/dressing.json",
        ],
    ),
    ("district", &["content/maps/placement_district.json"]),
    ("ludus", &["content/maps/placement_ludus.json"]),
    ("siege", &["content/maps/placement_siege.json"]),
    ("corner", &["content/maps/urban-war-pilot/corner.json"]),
];

/// Text after `"key":` in one actor object, up to the next `,`, `}` or line end.
fn field<'a>(obj: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("\"{key}\"");
    for (at, _) in obj.match_indices(&pat) {
        let rest = obj[at + pat.len()..].trim_start();
        if let Some(rest) = rest.strip_prefix(':') {
            let rest = rest.trim_start();
            let end = rest.find([',', '}', '\n']).unwrap_or(rest.len());
            return Some(rest[..end].trim());
        }
    }
    None
}

fn number(obj: &str, key: &str) -> f32 {
    field(obj, key).and_then(|v| v.parse().ok()).unwrap_or(0.0)
}

/// Actors of one scene file: the objects inside its `"actors"` array.
/// Missing `physical` reads true (a building), other missing numbers read 0.
fn parse_actors(text: &str) -> Vec<Actor> {
    let start = text
        .find("\"actors\"")
        .expect("scene file has an actors array");
    let body = &text[start..];
    let end = body.find(']').expect("actors array closes");
    let mut out = Vec::new();
    for chunk in body[..end].split('{').skip(1) {
        let obj = chunk.split('}').next().unwrap_or("");
        out.push(Actor {
            id: number(obj, "id") as u32,
            name: field(obj, "name")
                .unwrap_or("\"\"")
                .trim_matches('"')
                .to_string(),
            x: number(obj, "x"),
            y: number(obj, "y"),
            z: number(obj, "z"),
            sx: number(obj, "sx"),
            sy: number(obj, "sy"),
            sz: number(obj, "sz"),
            parent: field(obj, "parent").and_then(|v| v.parse().ok()),
            physical: field(obj, "physical") != Some("false"),
            vx: number(obj, "vx"),
            vz: number(obj, "vz"),
            yaw: number(obj, "yaw"),
        });
    }
    out
}

fn load_scene(files: &[&str]) -> Vec<Actor> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut actors = Vec::new();
    for file in files {
        let text =
            std::fs::read_to_string(root.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        actors.extend(parse_actors(&text));
    }
    actors
}

fn tile_of(x: f32, z: f32) -> (i32, i32) {
    (
        ((x * 100.0).round() as i32).div_euclid(100),
        ((z * 100.0).round() as i32).div_euclid(100),
    )
}

/// Flat bake view: every tile under the scene is walkable, spawn is the
/// lowest id, no flow, no doors. Quiet on a real scene; mutants edit it.
fn flat_nav(actors: &[Actor]) -> NavView {
    let mut view = NavView::default();
    let corners = |a: &Actor| {
        [
            tile_of(a.x - a.sx.abs() / 2.0, a.z - a.sz.abs() / 2.0),
            tile_of(a.x + a.sx.abs() / 2.0, a.z + a.sz.abs() / 2.0),
        ]
    };
    let tiles: Vec<(i32, i32)> = actors.iter().flat_map(corners).collect();
    let lo = (
        tiles.iter().map(|t| t.0).min().unwrap_or(0),
        tiles.iter().map(|t| t.1).min().unwrap_or(0),
    );
    let hi = (
        tiles.iter().map(|t| t.0).max().unwrap_or(0),
        tiles.iter().map(|t| t.1).max().unwrap_or(0),
    );
    for tx in lo.0..=hi.0 {
        for tz in lo.1..=hi.1 {
            view.tiles.insert((tx, tz), Tile { walk: true });
        }
    }
    view.spawn_id = actors.iter().map(|a| a.id).min().unwrap_or(0);
    view
}

/// Whole catalog (all 18 checks) with the flat view.
fn full(actors: &[Actor], nav: &NavView) -> Report {
    run_spatial(actors, &SPATIAL_CHECKS, Some(nav)).expect("catalog is legal")
}

fn rows(report: &Report) -> Vec<String> {
    report.findings.iter().map(|f| f.row()).collect()
}

fn prop(id: u32, name: &str, x: f32, y: f32, z: f32, s: f32) -> Actor {
    Actor {
        id,
        name: name.into(),
        x,
        y,
        z,
        sx: s,
        sy: s,
        sz: s,
        parent: None,
        physical: false,
        vx: 0.0,
        vz: 0.0,
        yaw: 0.0,
    }
}

/// One-edit mutant: the check it must trip and the actor id it must name.
struct Mutant {
    check: &'static str,
    id: u32,
    edit: fn(&mut Vec<Actor>, &mut NavView),
}

fn at(actors: &mut [Actor], id: u32) -> &mut Actor {
    actors
        .iter_mut()
        .find(|a| a.id == id)
        .expect("mutant target exists")
}

const MUTANTS: [Mutant; 18] = [
    Mutant {
        check: "overlap",
        id: 900,
        edit: |s, _| {
            let mut twin = at(s, 2).clone();
            twin.id = 900;
            s.push(twin);
        },
    },
    Mutant {
        check: "unknown-position",
        id: 2,
        edit: |s, _| at(s, 2).x = f32::NAN,
    },
    Mutant {
        check: "floating",
        id: 900,
        edit: |s, _| s.push(prop(900, "crate", 0.0, 5.0, 40.0, 1.0)),
    },
    Mutant {
        check: "bounds",
        id: 900,
        edit: |s, _| s.push(prop(900, "crate", 150.0, 0.5, 0.0, 1.0)),
    },
    Mutant {
        check: "sunken",
        id: 900,
        edit: |s, _| s.push(prop(900, "crate", 0.0, -3.0, 40.0, 1.0)),
    },
    Mutant {
        check: "scale",
        id: 900,
        edit: |s, _| s.push(prop(900, "crate", 0.0, 0.5, 40.0, 0.0)),
    },
    Mutant {
        check: "orphan",
        id: 900,
        edit: |s, _| {
            let mut a = prop(900, "crate", 0.0, 0.5, 40.0, 1.0);
            a.parent = Some(9999);
            s.push(a);
        },
    },
    Mutant {
        check: "giantObject",
        id: 900,
        edit: |s, _| {
            // Bigger than the rest of the layout (SPF5): solid, 90 m a side.
            let mut a = prop(900, "crate", 0.0, 45.0, 60.0, 90.0);
            a.physical = true;
            s.push(a);
        },
    },
    Mutant {
        check: "support",
        id: 900,
        edit: |s, _| {
            let mut a = prop(900, "crate", 0.0, 5.0, 40.0, 1.0);
            a.physical = true;
            s.push(a);
        },
    },
    Mutant {
        check: "dupeSpam",
        id: 900,
        edit: |s, _| {
            for i in 0..11u8 {
                s.push(prop(
                    900 + u32::from(i),
                    "crate",
                    -90.0 + 3.0 * f32::from(i),
                    0.5,
                    70.0,
                    1.0,
                ));
            }
        },
    },
    Mutant {
        check: "oob99",
        id: 900,
        edit: |s, _| s.push(prop(900, "crate", 99.5, 0.5, 0.0, 1.0)),
    },
    Mutant {
        check: "sweep",
        id: 900,
        edit: |s, _| {
            let mut mover = prop(900, "crate", 80.0, 0.5, 60.0, 1.0);
            mover.vx = 3.0;
            s.push(mover);
            s.push(prop(901, "crate", 83.0, 0.5, 60.0, 1.0));
        },
    },
    Mutant {
        check: "overlapDensity",
        id: 900,
        edit: |s, _| {
            for i in 0..9u8 {
                s.push(prop(
                    900 + u32::from(i),
                    "crate",
                    70.0 + 0.01 * f32::from(i),
                    0.5,
                    -60.0,
                    0.1,
                ));
            }
        },
    },
    Mutant {
        check: "grammar",
        id: 900,
        edit: |s, _| s.push(prop(900, "Bad Name", 0.0, 0.5, 40.0, 1.0)),
    },
    Mutant {
        check: "navConnect",
        id: 2,
        edit: |s, nav| {
            let a = at(s, 2);
            let t = tile_of(a.x, a.z);
            nav.tiles.insert(t, Tile { walk: false });
        },
    },
    Mutant {
        check: "flowPath",
        id: 900,
        edit: |s, nav| {
            let mut mover = prop(900, "crate", 0.0, 0.5, 40.0, 1.0);
            mover.vx = 2.0;
            nav.flow.insert(tile_of(mover.x, mover.z), (-1, 0));
            s.push(mover);
        },
    },
    Mutant {
        check: "accessFront",
        id: 900,
        edit: |s, _| {
            s.push(prop(900, "crate", 60.0, 0.5, 60.0, 1.0));
            s.push(prop(901, "crate", 60.5, 0.5, 60.0, 1.0));
        },
    },
    Mutant {
        check: "doorClear",
        id: 2,
        edit: |_, nav| {
            nav.doors.insert(
                2,
                Door {
                    width_cm: 100,
                    two_way: false,
                },
            );
        },
    },
];

/// Findings of `check` that touch `id`.
fn hits(report: &Report, check: &str, id: u32) -> usize {
    report
        .findings
        .iter()
        .filter(|f| f.check == check && (f.a == id || f.b == id))
        .count()
}

/// Pinned result per shipped scene: zero red, zero yellow (the exact true list is empty).
const PINNED: [&str; 6] = [
    "arena actors=11 verdict=clean\n",
    "arena_dressed actors=17 verdict=clean\n",
    "district actors=7 verdict=clean\n",
    "ludus actors=8 verdict=clean\n",
    "siege actors=7 verdict=clean\n",
    "corner actors=3 verdict=clean\n",
];

fn gate_text() -> String {
    let mut text = String::new();
    for (name, files) in SCENES {
        let actors = load_scene(files);
        let report = full(&actors, &flat_nav(&actors));
        text.push_str(&format!(
            "{name} actors={} verdict={}\n",
            actors.len(),
            report.verdict()
        ));
        for row in rows(&report) {
            text.push_str(&format!("  {row}\n"));
        }
    }
    text
}

#[test]
fn known_good_shipped_scenes_pin_zero_red_and_the_yellow_list() {
    let text = gate_text();
    assert!(
        !text.contains("verdict=red"),
        "a shipped scene reads red:\n{text}"
    );
    assert_eq!(
        text,
        PINNED.concat(),
        "pinned findings drifted; actual:\n{text}"
    );
    assert_eq!(text, gate_text(), "same files, same rows twice");
}

#[test]
fn known_good_gate_is_input_order_invariant() {
    for (_, files) in SCENES {
        let mut actors = load_scene(files);
        let nav = flat_nav(&actors);
        let forward = full(&actors, &nav);
        actors.reverse();
        let backward = full(&actors, &nav);
        assert_eq!(rows(&forward), rows(&backward));
        assert_eq!(forward.hash, backward.hash);
    }
}

#[test]
fn known_good_every_check_fires_on_a_one_edit_mutant() {
    let base = load_scene(SCENES[1].1);
    let clean = full(&base, &flat_nav(&base));
    let covered: BTreeSet<&str> = MUTANTS.iter().map(|m| m.check).collect();
    assert_eq!(
        covered,
        SPATIAL_CHECKS.iter().copied().collect(),
        "one mutant per catalog check"
    );
    for m in &MUTANTS {
        assert_eq!(
            hits(&clean, m.check, m.id),
            0,
            "{}: clean scene already names {}",
            m.check,
            m.id
        );
        let mut actors = base.clone();
        let mut nav = flat_nav(&actors);
        (m.edit)(&mut actors, &mut nav);
        let report = full(&actors, &nav);
        assert!(
            hits(&report, m.check, m.id) > 0,
            "{}: mutant did not fire\n{:?}",
            m.check,
            rows(&report)
        );
    }
}

#[test]
fn known_good_boundary_near_misses_stay_silent() {
    let count = |actors: &[Actor], check: &str| {
        run_spatial(actors, &[check], None)
            .expect("legal")
            .count(check)
    };
    // (check, silent scene, firing scene): one centimeter apart.
    let float = |y: f32| vec![prop(1, "crate", 0.0, y, 0.0, 2.0)];
    assert_eq!(
        (
            count(&float(1.01), "floating"),
            count(&float(1.02), "floating")
        ),
        (0, 1)
    );
    let edge = |x: f32| vec![prop(1, "crate", x, 0.5, 0.0, 1.0)];
    assert_eq!(
        (count(&edge(99.0), "oob99"), count(&edge(99.5), "oob99")),
        (0, 1)
    );
    let dupes = |n: u32| {
        (0..n)
            .map(|i| prop(i + 1, "crate", 3.0 * i as f32, 0.5, 0.0, 1.0))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        (count(&dupes(10), "dupeSpam"), count(&dupes(11), "dupeSpam")),
        (0, 11)
    );
    let touch = |dx: f32| {
        vec![
            prop(1, "a", 0.0, 0.5, 0.0, 1.0),
            prop(2, "b", dx, 0.5, 0.0, 1.0),
        ]
    };
    assert_eq!(
        (
            count(&touch(1.0), "overlap"),
            count(&touch(0.99), "overlap")
        ),
        (0, 1)
    );
}
