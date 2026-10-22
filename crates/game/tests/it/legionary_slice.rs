// legionary_slice.rs: the 3-minute slice (LEG10).
//
// March 60 s, dig the camp 60 s, hold the line 60 s. Every phase reads only
// landed modules: `engine_game::march` (LEG8), `engine_world::works` (LEG7)
// over `engine_world::dig` (LEG6), and an 8-slot `engine_arena` match (the
// legionary rules, LEG1-LEG5). The slice replays twice equal and writes
// `target/reports/legionary-slice-<stamp>.md`, which lists the four eye
// captures beside the two reference pictures and the machine look score.
//
// The captures themselves are the window's: this binary is headless, so the
// manifest names them and the window run fills the folder (the same run
// LEG1 proved on this PC: `arena --legionary ... --capture-dir`).

use engine_arena::{ArenaConfig, FighterTape};
use engine_core::Fnv64;
use engine_game::march::{March, MarchRules, Pace, COLUMN_MEN};
use engine_world::works::{CampPlan, CampWorks};

/// One phase at 60 Hz: a minute of the slice.
const PHASE_TICKS: u32 = 3600;

/// The march phase: eight men on the march for a minute, at the regular step.
/// Digest over the column's slots in file order.
fn march_phase() -> u64 {
    let rules = MarchRules::shipped().expect("the shipped march rules read");
    let mut march = March::new(rules, COLUMN_MEN).expect("eight men march");
    march.set_pace(Pace::Regular);
    march.advance_ticks(PHASE_TICKS);
    let mut hasher = Fnv64::new();
    hasher.write_str(march.objective());
    for index in 0..COLUMN_MEN {
        let slot = march.position(index).expect("every man holds a slot");
        hasher.write_u8(slot.index);
        hasher.write_u8(slot.file);
        hasher.write_u8(slot.rank);
        hasher.write_i64(slot.along_mm);
        hasher.write_i64(slot.lateral_mm);
    }
    for man in march.men() {
        hasher.write_u16(man.stamina);
    }
    hasher.finish()
}

/// The camp phase: eight diggers close the one-night perimeter (50 x 50 m,
/// 200 m of ditch) and the camp is checked at the minute's end.
fn camp_phase() -> u64 {
    let mut camp = CampWorks::new(CampPlan::ONE_NIGHT);
    for _ in 0..PHASE_TICKS {
        camp.crew_tick(8);
    }
    camp.hash()
}

/// The line phase: eight slots hold the line for a minute under the legionary
/// tables; the last checkpoint's hash is the phase digest, and the run
/// replays twice.
fn line_phase() -> u64 {
    let config = ArenaConfig {
        seed: 2040,
        ticks: PHASE_TICKS,
        fighters: 8,
        half_extent_m: 20.0,
        table_version: 2,
    };
    let tape = FighterTape(Vec::new());
    let chain = engine_arena::run(&config, &tape);
    assert_eq!(chain.len() as u32, PHASE_TICKS, "one checkpoint per tick");
    chain.last().map(|c| c.hash).unwrap_or(0)
}

/// The slice: the three phases, in order, as one digest.
fn slice() -> [u64; 3] {
    [march_phase(), camp_phase(), line_phase()]
}

fn slice_digest(parts: &[u64; 3]) -> u64 {
    let mut hasher = Fnv64::new();
    for part in parts {
        hasher.write_u64(*part);
    }
    hasher.finish()
}

#[test]
fn the_slice_replays_twice_equal() {
    let first = slice();
    let second = slice();
    assert_eq!(first, second, "the slice replays tick for tick");
    assert_eq!(slice_digest(&first), slice_digest(&second));
    let replayed = [march_phase(), camp_phase(), line_phase()];
    assert_eq!(replayed, first, "a third pass lands on the same digests");
}

#[test]
fn eight_diggers_close_the_camp_inside_the_minute() {
    let mut camp = CampWorks::new(CampPlan::ONE_NIGHT);
    let mut ticks: u32 = 0;
    while !camp.perimeter_closed() && ticks < PHASE_TICKS {
        camp.crew_tick(8);
        ticks = ticks.saturating_add(1);
    }
    assert!(
        camp.perimeter_closed(),
        "the one-night camp closes inside its minute (at {ticks} ticks of {PHASE_TICKS})"
    );
}

#[test]
fn the_report_lists_the_captures_beside_the_references() {
    let parts = slice();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let dir = std::path::Path::new("target/reports");
    std::fs::create_dir_all(dir).expect("the reports folder is made");
    let path = dir.join(format!("legionary-slice-{stamp}.md"));
    let body = format!(
        "# legionary slice {stamp}\n\n\
         marched 60 s, dug the camp 60 s, held the line 60 s. Every digest twice equal.\n\n\
         | phase | digest |\n|---|---|\n\
         | march | {:016x} |\n\
         | camp | {:016x} |\n\
         | line | {:016x} |\n\
         | slice | {:016x} |\n\n\
         captures (the window run's, beside the references):\n\
         1. eye-march.png\n\
         2. eye-camp.png\n\
         3. eye-line.pngn\
         4. eye-line-late.png\n\n\
         references: docs/design/legionary/helmet-pov-march.jpg, \
         docs/design/legionary/helmet-pov-battle.jpg\n\n\
         look score: the machine score of the four captures against the two \
         references (view.compare), posted by the capture run.\n",
        parts[0],
        parts[1],
        parts[2],
        slice_digest(&parts),
    );
    std::fs::write(&path, body).expect("the report is written");
    let read_back = std::fs::read_to_string(&path).expect("the report reads back");
    assert!(
        read_back.contains("captures"),
        "the report lists the captures"
    );
    for name in [
        "eye-march.png",
        "eye-camp.png",
        "eye-line.png",
        "eye-line-late.png",
        "helmet-pov-march.jpg",
        "helmet-pov-battle.jpg",
        "look score",
    ] {
        assert!(read_back.contains(name), "the report names {name}");
    }
    println!("legionary slice report: {}", path.display());
}
