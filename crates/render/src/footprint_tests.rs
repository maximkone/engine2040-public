//! footprint_tests: the print's fade curve, its two lobes and its raster.
//!
//! Run: `cargo test -p engine-render --lib footprint`

use super::*;
use crate::ground_state::MAP_DIM;

fn print_at(x: f32, y: f32, heading: f32) -> Print {
    Print {
        x_m: x,
        y_m: y,
        heading,
        born_tick: 0,
    }
}

fn non_zero(map: &[u8]) -> usize {
    map.iter().filter(|d| **d > 0).count()
}

#[test]
fn the_dent_is_full_on_the_plant_tick_and_zero_at_the_lifetime() {
    assert_eq!(depth_at(0), PRINT_FULL_DEPTH);
    assert_eq!(depth_at(PRINT_LIFETIME_TICKS - 1), PRINT_FLOOR_DEPTH);
    assert_eq!(depth_at(PRINT_LIFETIME_TICKS), 0);
    assert_eq!(depth_at(PRINT_LIFETIME_TICKS + 5), 0);
    // Whole units cannot fall on each of 600 ticks (the span is 254), so the
    // curve is non-increasing with a real drop at both ends; it never rises.
    let mut last = depth_at(0);
    for age in 1..PRINT_LIFETIME_TICKS {
        let depth = depth_at(age);
        assert!(depth <= last, "depth must never rise: age {age}");
        last = depth;
    }
    assert!(
        depth_at(0) > depth_at(PRINT_LIFETIME_TICKS / 2),
        "halfway is shallower"
    );
    assert!(
        depth_at(PRINT_LIFETIME_TICKS / 2) > depth_at(PRINT_LIFETIME_TICKS - 1),
        "the last tick is the floor"
    );
}

#[test]
fn a_print_is_a_heel_and_a_toe_one_foot_length_apart() {
    let flat = print_at(10.0, 10.0, 0.0);
    let (heel, toe) = flat.lobes();
    assert_eq!(heel, [10.0, 10.0]);
    assert!((toe[0] - (10.0 + PRINT_LENGTH_M)).abs() < 1e-6);
    assert!((toe[1] - 10.0).abs() < 1e-6, "a zero heading runs along +x");
    // A quarter turn sends the toe along +y and leaves the heel put.
    let turned = print_at(10.0, 10.0, std::f32::consts::FRAC_PI_2);
    let (heel, toe) = turned.lobes();
    assert_eq!(heel, [10.0, 10.0], "the heel stays at the planted point");
    assert!(
        (toe[0] - 10.0).abs() < 1e-5,
        "a quarter turn puts no toe offset along x"
    );
    assert!((toe[1] - (10.0 + PRINT_LENGTH_M)).abs() < 1e-5);
}

#[test]
fn one_print_writes_texels_and_two_identical_rasters_match() {
    let print = print_at(10.0, 10.0, 0.4);
    let mut a = vec![0u8; MAP_DIM * MAP_DIM];
    let mut b = vec![0u8; MAP_DIM * MAP_DIM];
    stamp(&mut a, &print, PRINT_FULL_DEPTH);
    stamp(&mut b, &print, PRINT_FULL_DEPTH);
    let written = non_zero(&a);
    assert!(written > 0, "a planted foot writes texels");
    assert_eq!(a, b, "the same print bytes give the same map bytes");
    // Both lobes are filled: the toe disc is the bigger of the two.
    let toe_texels = (2.0 * PRINT_TOE_RADIUS_M / PIT_METERS * MAP_DIM as f32) as usize;
    assert!(
        written > toe_texels,
        "heel and toe both print: {written} texels against a toe width of {toe_texels}"
    );
}

#[test]
fn a_deeper_print_wins_where_two_overlap_and_zero_prints_nothing() {
    let print = print_at(10.0, 10.0, 0.0);
    let at = (texel_of(10.0) as usize, texel_of(10.0) as usize);
    let i = at.1 * MAP_DIM + at.0;
    let mut map = vec![0u8; MAP_DIM * MAP_DIM];
    stamp(&mut map, &print, 40);
    assert_eq!(map[i], 40);
    stamp(&mut map, &print, 90);
    assert_eq!(map[i], 90, "the deeper dent wins");
    stamp(&mut map, &print, 20);
    assert_eq!(map[i], 90, "a shallower overlap never digs out");
    let mut empty = vec![0u8; MAP_DIM * MAP_DIM];
    stamp(&mut empty, &print, 0);
    assert_eq!(non_zero(&empty), 0, "depth 0 writes nothing");
}

#[test]
fn a_print_at_the_pit_edge_clips_instead_of_writing_outside() {
    let edge = print_at(0.0, 0.0, std::f32::consts::PI);
    let mut map = vec![0u8; MAP_DIM * MAP_DIM];
    stamp(&mut map, &edge, PRINT_FULL_DEPTH);
    assert!(
        non_zero(&map) > 0,
        "the inside half of the print still lands"
    );
}

#[test]
fn a_map_smaller_than_the_ground_is_refused_without_panicking() {
    let mut short = vec![0u8; 16];
    stamp(&mut short, &print_at(10.0, 10.0, 0.0), PRINT_FULL_DEPTH);
    assert!(short.iter().all(|d| *d == 0));
}
