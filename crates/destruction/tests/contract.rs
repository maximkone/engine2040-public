//! Break-to-nav dirty-tile contract proof: every bonded-collection break set
//! names exactly the nav tiles it dirties and rebakes exactly those tiles,
//! proven x2.
//! Test-only edge (destruction dev-depends on nav); the runtime graph stays
//! core-only.

use engine_destruction::bonded::{BondedCollection, BreakEvent};
use engine_destruction::nav_dirty::DirtySetRecord;
use engine_nav::tiles::{BreakSet, TileId, TiledNav};

const GRID: usize = 16;
const TILES: usize = 16;
const GOAL: (usize, usize) = (15, 15);

fn tile_of(chunk: u32) -> Option<TileId> {
    Some(TileId(chunk % TILES as u32))
}

/// Sword break fixture: crate prop, huge impulse on every chunk at tick 3,
/// 30 steps of falling. Returns the drained events plus the chunk count.
fn sword_break() -> (Vec<BreakEvent>, usize) {
    let mut prop =
        BondedCollection::crate_prop(0.0, 0.5, 0.0, 0.5, 8, 7).expect("valid crate prop");
    let chunks = prop.chunks().len();
    for index in 0..chunks {
        prop.apply_impact(index, 1e9, 3).expect("impact lands");
    }
    for _ in 0..30 {
        prop.step(1.0 / 60.0);
    }
    let events = prop.drain_events();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, BreakEvent::ChunkFreed { .. })),
        "sword break must free at least one chunk"
    );
    (events, chunks)
}

fn open_nav() -> TiledNav {
    let blocked = vec![false; GRID * GRID];
    let mut nav = TiledNav::bake(&blocked, GRID, GRID, 4).expect("open grid bakes");
    nav.bake_goal(GOAL).expect("goal bakes");
    nav
}

#[test]
fn contract_break_set_names_exactly_the_dirty_tiles() {
    let (events, chunks) = sword_break();
    let record =
        DirtySetRecord::from_events(3, &events, chunks).expect("record names freed chunks");
    let freed: Vec<u32> = {
        let mut ids: Vec<u32> = events
            .iter()
            .filter_map(|e| match e {
                BreakEvent::ChunkFreed { chunk, .. } => Some(*chunk),
                BreakEvent::BondBroken { .. } => None,
            })
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    };
    assert_eq!(record.chunks(), freed.as_slice(), "record names exact set");
    let set =
        BreakSet::from_chunks(3, &freed, TILES, tile_of).expect("nav set builds from freed chunks");
    let mut want: Vec<TileId> = freed.iter().map(|c| tile_of(*c).unwrap()).collect();
    want.sort();
    want.dedup();
    assert_eq!(
        set.tiles.as_slice(),
        want.as_slice(),
        "set names exact tiles"
    );
    let mut nav = open_nav();
    let affected = nav.apply_breaks(&set).expect("breaks apply");
    assert_eq!(
        affected,
        set.tiles.as_slice(),
        "apply touches exactly the set"
    );
    assert_eq!(
        nav.dirty_tiles(),
        set.tiles.as_slice(),
        "dirty equals the set"
    );
    assert_eq!(
        nav.rebake_budget(set.tiles.as_slice().len()),
        set.tiles.as_slice().len(),
        "budget rebakes exactly those tiles"
    );
    assert_eq!(nav.dirty_count(), 0, "budget drains the dirty set");
    for tile in set.tiles.as_slice() {
        assert_eq!(nav.version(*tile), Ok(1), "bumped tile version 1");
        assert!(
            nav.needs_repath(*tile, 0).expect("repath query"),
            "stale walker must repath"
        );
        assert!(
            !nav.needs_repath(*tile, 1).expect("repath query"),
            "current walker skips repath"
        );
    }
    let untouched = TileId(0);
    if !set.tiles.as_slice().contains(&untouched) {
        assert_eq!(nav.version(untouched), Ok(0), "untouched stays 0");
        assert!(
            !nav.needs_repath(untouched, 0).expect("repath query"),
            "untouched never repaths"
        );
    }
}

#[test]
fn contract_break_to_nav_replays_identical_bytes_twice() {
    let (events_a, chunks_a) = sword_break();
    let (events_b, chunks_b) = sword_break();
    assert_eq!(chunks_a, chunks_b, "same chunk count");
    let record_a = DirtySetRecord::from_events(3, &events_a, chunks_a).expect("record A builds");
    let record_b = DirtySetRecord::from_events(3, &events_b, chunks_b).expect("record B builds");
    assert_eq!(record_a.to_bytes(), record_b.to_bytes(), "record bytes x2");
    assert_eq!(record_a.digest(), record_b.digest(), "record digest x2");
    let freed: Vec<u32> = {
        let mut ids: Vec<u32> = events_a
            .iter()
            .filter_map(|e| match e {
                BreakEvent::ChunkFreed { chunk, .. } => Some(*chunk),
                BreakEvent::BondBroken { .. } => None,
            })
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    };
    let mut nav_a = open_nav();
    let mut nav_b = open_nav();
    let set_a = BreakSet::from_chunks(3, &freed, TILES, tile_of).expect("set A");
    let set_b = BreakSet::from_chunks(3, &freed, TILES, tile_of).expect("set B");
    nav_a.apply_breaks(&set_a).expect("apply A");
    nav_b.apply_breaks(&set_b).expect("apply B");
    assert_eq!(nav_a.digest(), nav_b.digest(), "nav digest x2");
    // Reversed chunk order names the same set: sorted/deduped by design.
    let mut reversed = freed.clone();
    reversed.reverse();
    let set_reversed =
        BreakSet::from_chunks(3, &reversed, TILES, tile_of).expect("reversed set builds");
    assert_eq!(
        set_reversed.tiles.as_slice(),
        set_a.tiles.as_slice(),
        "order-free set"
    );
}

#[test]
fn contract_bad_handoffs_refuse_and_change_nothing() {
    use engine_nav::tiles::TileError;
    let nav = open_nav();
    assert_eq!(
        DirtySetRecord::from_chunks(9, &[], 8).map(|_| ()),
        Err(engine_destruction::nav_dirty::DirtySetError::EmptySet),
        "empty chunks refuse"
    );
    assert_eq!(
        BreakSet::new(9, &[], TILES).map(|_| ()),
        Err(TileError::EmptySet),
        "empty tiles refuse"
    );
    assert_eq!(
        BreakSet::from_chunks(9, &[3], TILES, |_| Some(TileId(99))).map(|_| ()),
        Err(TileError::BadTile { tile: 99 }),
        "out-of-range tile refuses"
    );
    assert_eq!(nav.dirty_count(), 0, "dirty untouched");
    assert_eq!(nav.version(TileId(0)), Ok(0), "version untouched");
}
