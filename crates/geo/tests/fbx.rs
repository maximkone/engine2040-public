//! GEO9a: the pinned-Blender fixture (`tools/dcc/blender/make_fbx_fixture.py`) through
//! `engine_geo::fbx::import_fbx`. Needs feature `fbx`.
#![cfg(feature = "fbx")]

use std::time::{Duration, Instant};

use engine_core::Rng;
use engine_geo::{fbx, GeoError, GeoMesh};

/// The fixture: a 1 m cube plus a UV sphere, exported by Blender 5.2 (axis forward -Z, up Y).
const FBX: &[u8] = include_bytes!("data/two_solids.fbx");
/// What Blender counted for each mesh when it wrote the fixture.
const JSON: &str = include_str!("data/two_solids.json");

fn imported() -> Vec<GeoMesh> {
    fbx::import_fbx(FBX).expect("the Blender fixture imports")
}

/// The `triangles` numbers of the fixture JSON, in file order.
fn expected_triangles(json: &str) -> Vec<usize> {
    json.match_indices("\"triangles\"")
        .map(|(at, key)| {
            let after_key = &json[at + key.len()..];
            let after_colon = after_key.split_once(':').expect("key has a value").1;
            let digits: String = after_colon
                .trim_start()
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            digits.parse().expect("triangles is a number")
        })
        .collect()
}

#[test]
fn fixture_gives_two_meshes_with_blender_triangle_counts() {
    let meshes = imported();
    assert_eq!(meshes.len(), 2, "one mesh per mesh node");

    let mut expected = expected_triangles(JSON);
    let mut got: Vec<usize> = meshes.iter().map(GeoMesh::triangle_count).collect();
    assert_eq!(expected.len(), 2, "the JSON names both meshes");
    assert_eq!(expected.len(), meshes.len());
    expected.sort_unstable();
    got.sort_unstable();
    assert_eq!(got, expected, "triangle counts are Blender's own");

    for mesh in &meshes {
        mesh.validate().expect("an imported mesh validates");
        assert_eq!(
            mesh.normals.len(),
            mesh.positions.len(),
            "normals per vertex"
        );
        assert_eq!(mesh.uvs.len(), mesh.positions.len(), "uv0 per vertex");
        for normal in &mesh.normals {
            let length = normal.iter().map(|v| v * v).sum::<f32>().sqrt();
            assert!((length - 1.0).abs() < 1e-3, "normals are unit: {normal:?}");
        }
        assert!(mesh.edge_report().is_closed(), "the fixture is watertight");
        assert!(
            mesh.volume() > 0.0,
            "triangles wind outward: {}",
            mesh.volume()
        );
    }
}

#[test]
fn cube_bounds_are_one_meter_with_plus_y_up() {
    let meshes = imported();
    let cube = meshes
        .iter()
        .find(|mesh| mesh.triangle_count() == 12)
        .expect("the cube is the 12-triangle mesh");
    let (low, high) = cube.bounds();
    for axis in 0..3 {
        let span = high[axis] - low[axis];
        assert!(
            (span - 1.0).abs() < 1e-4,
            "cube axis {axis} spans one meter: {low:?}..{high:?}"
        );
    }
    // The file is centimeters with -Z forward; the importer converts to meters and +Y up, so
    // every solid keeps its Blender floor at y = 0.
    for mesh in &meshes {
        let (low, _) = mesh.bounds();
        assert!(
            low[1].abs() < 1e-4,
            "mesh floor sits at y = 0: {:?}",
            mesh.bounds()
        );
    }
}

#[test]
fn same_bytes_give_the_same_digests() {
    let first: Vec<u64> = imported().iter().map(GeoMesh::digest).collect();
    let second: Vec<u64> = imported().iter().map(GeoMesh::digest).collect();
    assert_eq!(first, second, "cook twice, same bytes");
    assert_eq!(first.len(), 2);
    assert!(first.iter().all(|&digest| digest != 0));
}

#[test]
fn seeded_byte_mutations_never_panic_and_always_refuse_typed() {
    let started = Instant::now();
    let mut rng = Rng::new(2040);
    let mut failed = 0usize;
    for _ in 0..1_000 {
        let mut bytes = FBX.to_vec();
        let flips = 1 + rng.next_range(0, 8);
        for _ in 0..flips {
            let at = rng.next_range(0, bytes.len() as u64) as usize;
            bytes[at] ^= 1 << rng.next_range(0, 8);
        }
        match fbx::import_fbx(&bytes) {
            Ok(meshes) => {
                for mesh in &meshes {
                    mesh.validate().expect("a parsed mesh stays a valid mesh");
                }
            }
            Err(error) => {
                failed += 1;
                // The type is the contract: every refusal is a GeoError, never a panic text.
                match error {
                    GeoError::Donor(_)
                    | GeoError::Empty
                    | GeoError::Malformed(_)
                    | GeoError::NonFinite
                    | GeoError::NotClosed => {}
                }
            }
        }
    }
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(20),
        "1000 mutated loads stay under 20 s: {elapsed:?}"
    );
    assert!(failed > 0, "the mutations do reach the refusal path");
    println!("1000 mutations: {failed} refusals, 0 panics, {elapsed:?}");
}
