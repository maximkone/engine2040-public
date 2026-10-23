//! Determinism oracle for the falling-ragdoll scene (row PX45).
//!
//! Builds the donor's own `determinism` scene (the same one `test_determinism.c`
//! gates on), steps it a fixed number of ticks, and digests the full body state:
//! every live body in body-id order, its world transform, linear velocity and
//! angular velocity, hashed with the donor's `b3Hash` port.
//!
//! Two processes must print the same digest on one host; the wasm run must print
//! the same digest too. `--dump` writes the exact per-body values so a later run
//! can name the first field that differs instead of only the digest.

use box3d_rust::body::{
    body_get_angular_velocity, body_get_linear_velocity, body_get_transform, body_is_valid,
    make_body_id,
};
use box3d_rust::core::{hash, HASH_INIT};
use box3d_rust::determinism::{
    create_falling_ragdolls, hash_world_transform, update_falling_ragdolls,
};
use box3d_rust::types::default_world_def;
use box3d_rust::world::World;

/// Fixed simulation step. Matches the donor scene driver.
const TIME_STEP: f32 = 1.0 / 60.0;
const SUB_STEPS: i32 = 4;

/// Default digest tick. 120 is mid-fall: bodies are still moving, so a solve
/// difference cannot hide behind sleep.
const DEFAULT_STEPS: u32 = 120;

/// Field names in dump order, so a mismatch can name a field.
const FIELD_NAMES: [&str; 13] = [
    "pos.x", "pos.y", "pos.z", "rot.x", "rot.y", "rot.z", "rot.w", "vel.x", "vel.y", "vel.z",
    "spin.x", "spin.y", "spin.z",
];

struct BodyLine {
    body_index: i32,
    fields: Vec<f64>,
}

fn body_lines(world: &World) -> Vec<BodyLine> {
    let mut lines = Vec::new();
    for index in 0..world.bodies.len() as i32 {
        let id = make_body_id(world, index);
        if !body_is_valid(world, id) {
            continue;
        }

        let xf = body_get_transform(world, id);
        let v = body_get_linear_velocity(world, id);
        let w = body_get_angular_velocity(world, id);

        lines.push(BodyLine {
            body_index: index,
            fields: vec![
                xf.p.x as f64,
                xf.p.y as f64,
                xf.p.z as f64,
                xf.q.v.x as f64,
                xf.q.v.y as f64,
                xf.q.v.z as f64,
                xf.q.s as f64,
                v.x as f64,
                v.y as f64,
                v.z as f64,
                w.x as f64,
                w.y as f64,
                w.z as f64,
            ],
        });
    }
    lines
}

fn vec3_bytes(v: box3d_rust::math_functions::Vec3) -> [u8; 12] {
    let mut bytes = [0u8; 12];
    bytes[0..4].copy_from_slice(&v.x.to_le_bytes());
    bytes[4..8].copy_from_slice(&v.y.to_le_bytes());
    bytes[8..12].copy_from_slice(&v.z.to_le_bytes());
    bytes
}

fn digest_of(world: &World) -> (u32, usize) {
    let mut h = HASH_INIT;
    let mut count = 0usize;
    for index in 0..world.bodies.len() as i32 {
        let id = make_body_id(world, index);
        if !body_is_valid(world, id) {
            continue;
        }

        h = hash_world_transform(h, &body_get_transform(world, id));
        h = hash(h, &vec3_bytes(body_get_linear_velocity(world, id)));
        h = hash(h, &vec3_bytes(body_get_angular_velocity(world, id)));
        count += 1;
    }
    (h, count)
}

fn dump_text(lines: &[BodyLine]) -> String {
    let mut out = String::new();
    for line in lines {
        out.push_str(&format!("body {}:", line.body_index));
        for value in &line.fields {
            out.push_str(&format!(" {:?}", value));
        }
        out.push('\n');
    }
    out
}

/// Name the first body and field that differs from `reference`, in dump order.
fn first_difference(reference: &[BodyLine], other: &[BodyLine]) -> String {
    if reference.len() != other.len() {
        return format!(
            "body count differs: reference {} vs run {}",
            reference.len(),
            other.len()
        );
    }

    for (a, b) in reference.iter().zip(other) {
        if a.body_index != b.body_index {
            return format!(
                "body order differs at slot: reference body {} vs run body {}",
                a.body_index, b.body_index
            );
        }
        for (field, (x, y)) in FIELD_NAMES.iter().zip(a.fields.iter().zip(&b.fields)) {
            if x != y || x.is_nan() != y.is_nan() {
                return format!(
                    "body {} field {}: reference {:?} vs run {:?}",
                    a.body_index, field, x, y
                );
            }
        }
    }

    "no field differs: every body state matches".to_string()
}

fn parse_dump(text: &str) -> Vec<BodyLine> {
    let mut lines = Vec::new();
    for line in text.lines() {
        let rest = match line.strip_prefix("body ") {
            Some(rest) => rest,
            None => continue,
        };
        let (index, values) = match rest.split_once(':') {
            Some(parts) => parts,
            None => continue,
        };
        let body_index: i32 = index.trim().parse().expect("body index");
        let fields: Vec<f64> = values
            .split_whitespace()
            .map(|v| v.parse().expect("body field"))
            .collect();
        assert_eq!(fields.len(), FIELD_NAMES.len(), "dump field count");
        lines.push(BodyLine { body_index, fields });
    }
    lines
}

/// The donor's own settled check: run until every body sleeps and print the
/// pinned scene hash. Proves the vendored scene still matches the donor gate.
fn settled_check() -> (u32, i32) {
    let mut world = World::new(&default_world_def());
    let mut data = create_falling_ragdolls(&mut world);
    for _ in 0..500 {
        world.step(TIME_STEP, SUB_STEPS);
        if update_falling_ragdolls(&world, &mut data) {
            break;
        }
    }
    (data.hash, data.sleep_step)
}

fn main() {
    let mut steps = DEFAULT_STEPS;
    let mut dump_path: Option<String> = None;
    let mut compare_path: Option<String> = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--steps" => {
                steps = args
                    .next()
                    .expect("--steps needs a count")
                    .parse()
                    .expect("--steps count");
            }
            "--dump" => dump_path = Some(args.next().expect("--dump needs a path")),
            "--compare" => compare_path = Some(args.next().expect("--compare needs a path")),
            other => panic!("unknown argument {other}"),
        }
    }

    let mut world = World::new(&default_world_def());
    let _data = create_falling_ragdolls(&mut world);

    for _ in 0..steps {
        world.step(TIME_STEP, SUB_STEPS);
    }

    let lines = body_lines(&world);
    let (digest, bodies) = digest_of(&world);

    println!(
        "oracle=box3d-rust scene=falling_ragdolls tick={steps} substeps={SUB_STEPS} bodies={bodies} digest=0x{digest:08X}"
    );

    let (settled_hash, sleep_step) = settled_check();
    println!("donor_settled=0x{settled_hash:08X} sleep_step={sleep_step}");

    if let Some(path) = dump_path {
        std::fs::write(&path, dump_text(&lines)).expect("write dump");
        println!("dump={path}");
    }

    if let Some(path) = compare_path {
        let reference = parse_dump(&std::fs::read_to_string(&path).expect("read reference"));
        println!("compare={path}");
        println!("first_difference={}", first_difference(&reference, &lines));
    }
}
