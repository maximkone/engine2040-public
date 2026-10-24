//! Tests for the gear table: every glb loads, the recorded bounds are the file's, the grip
//! transform lands the slot's length on the slot's axis, triangles stay under the caps row,
//! the same file gives the same part twice, and a missing file is `None` (never a panic).

use super::*;

/// The gear folder, or a skip message when the LFS content is not on this machine.
fn dir() -> PathBuf {
    let dir = source_dir();
    assert!(
        dir.join(ANCHOR_FILE).is_file(),
        "{}: the gear glbs are not checked out here",
        dir.display()
    );
    dir
}

fn import(row: &GearRow) -> GltfImport {
    load(&dir(), row.model).unwrap_or_else(|| panic!("{} loads", row.model.file_name()))
}

fn unit_len(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Where a fitted part starts and ends along one rig direction, metres.
fn span(part: &GearPart, axis: [f32; 3]) -> (f32, f32) {
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for pos in &part.positions {
        let d = dot(*pos, axis);
        lo = lo.min(d);
        hi = hi.max(d);
    }
    (lo, hi)
}

#[test]
fn the_table_fills_every_slot_once_with_a_file_the_source_folder_holds() {
    let dir = dir();
    for slot in SLOTS {
        let at = GEAR_TABLE.iter().filter(|row| row.slot == slot).count();
        assert_eq!(at, 1, "{} rows", slot.name());
        assert_eq!(row(slot).slot, slot);
    }
    for row in &GEAR_TABLE {
        let file = row.model.file_name();
        assert!(dir.join(file).is_file(), "{file} is not in the gear folder");
        assert!(
            row.long_axis < 3 && row.across_axis < 3 && row.long_axis != row.across_axis,
            "{file}: axes"
        );
        assert!((0.0..=1.0).contains(&row.grip_at), "{file}: grip_at");
        assert!(row.length_m > 0.0, "{file}: length");
        // The table's canonical frame is orthonormal, so a fit taken from it is a rotation.
        assert!(
            (unit_len(row.slot_axis) - 1.0).abs() < 1.0e-3,
            "{file}: axis"
        );
        assert!(
            (unit_len(row.slot_across) - 1.0).abs() < 1.0e-3,
            "{file}: across"
        );
        assert!(dot(row.slot_axis, row.slot_across).abs() < 1.0e-3, "{file}");
    }
    // The two sizes the fight poses need, from the one owner of the sword's length.
    assert_eq!(row(GearSlot::SwordHand).length_m, super::super::SWORD_LEN_M);
    let shield = row(GearSlot::ShieldArm);
    assert!((shield.length_m - 1.0).abs() < 1.0e-6, "scutum height");
    assert_eq!(shield.across_m, Some(0.65), "scutum width");
}

#[test]
fn every_table_glb_loads_and_its_measured_bounds_match_the_row() {
    for row in &GEAR_TABLE {
        let import = import(row);
        let file = row.model.file_name();
        let (lo, hi) = bounds(&import).unwrap_or_else(|| panic!("{file}: no positions"));
        for axis in 0..3 {
            let span = (row.src_max[axis] - row.src_min[axis]).max(1.0e-3);
            assert!(
                (lo[axis] - row.src_min[axis]).abs() <= span * 0.01,
                "{file} axis {axis}: min {} recorded {}",
                lo[axis],
                row.src_min[axis]
            );
            assert!(
                (hi[axis] - row.src_max[axis]).abs() <= span * 0.01,
                "{file} axis {axis}: max {} recorded {}",
                hi[axis],
                row.src_max[axis]
            );
        }
        assert_eq!(tri_count(&import), row.tris, "{file} triangles");
    }
}

#[test]
fn the_grip_transform_lands_the_slot_length_on_the_slot_axis_within_five_percent() {
    for row in &GEAR_TABLE {
        let import = import(row);
        let fitted = GripFit::canonical(row);
        let part = part(row, &import, &fitted, 0)
            .unwrap_or_else(|| panic!("{} part", row.model.file_name()));
        let file = row.model.file_name();
        let (lo, hi) = span(&part, row.slot_axis);
        let length = hi - lo;
        assert!(
            (length - row.length_m).abs() <= row.length_m * 0.05,
            "{file} length {length} m, want {} m",
            row.length_m
        );
        // The origin is the grip: `grip_at` of the length hangs below it, the rest above.
        assert!(
            (lo + row.grip_at * row.length_m).abs() <= row.length_m * 0.05,
            "{file} grip at {lo} m",
        );
        let (wide_lo, wide_hi) = span(&part, row.slot_across);
        let want = row.across_m.unwrap_or_else(|| {
            let src = row.src_max[row.across_axis] - row.src_min[row.across_axis];
            let long = row.src_max[row.long_axis] - row.src_min[row.long_axis];
            row.length_m * src / long
        });
        let across = wide_hi - wide_lo;
        assert!(
            (across - want).abs() <= want * 0.05,
            "{file} width {across} m, want {want} m"
        );
        assert_eq!(part.joints.len(), part.positions.len(), "{file} skin rows");
        assert!(part.weights.iter().all(|w| w[0] == 1.0), "{file} rigid");
        for index in &part.indices {
            assert!(
                usize::try_from(*index).unwrap_or(usize::MAX) < part.positions.len(),
                "{file} index"
            );
        }
    }
}

#[test]
fn every_table_model_stays_under_its_caps_row() {
    for row in &GEAR_TABLE {
        let tris = tri_count(&import(row));
        let cap = usize::try_from(row.slot.tris_cap()).unwrap_or(usize::MAX);
        // The cap is the one caps table, never a number of its own.
        assert!(
            cap == usize::try_from(POTATO.max_tris).unwrap_or(0)
                || cap == usize::try_from(COMFORT.max_tris).unwrap_or(0),
            "{}: {cap} is not a caps row",
            row.slot.name()
        );
        assert!(
            tris <= cap,
            "{} {} triangles over the {cap} cap",
            row.model.file_name(),
            tris
        );
    }
}

#[test]
fn the_same_file_gives_the_same_part_twice() {
    for row in &GEAR_TABLE {
        let fit = GripFit::canonical(row);
        let first = part(row, &import(row), &fit, 3).expect("a part");
        let second = part(row, &import(row), &fit, 3).expect("a part");
        assert_eq!(first, second, "{} twice", row.model.file_name());
        assert_eq!(first.image, second.image, "texture bytes");
    }
}

#[test]
fn every_table_file_is_credited_in_provenance() {
    let text = std::fs::read_to_string(dir().join("PROVENANCE.md")).expect("provenance");
    for row in &GEAR_TABLE {
        assert!(
            text.contains(row.model.file_name()),
            "{} is not in PROVENANCE.md",
            row.model.file_name()
        );
    }
}

#[test]
fn a_missing_gear_file_is_none_not_a_panic() {
    let missing = Path::new("target/no-such-gear-directory");
    for row in &GEAR_TABLE {
        assert!(
            load(missing, row.model).is_none(),
            "{}",
            row.model.file_name()
        );
    }
}
