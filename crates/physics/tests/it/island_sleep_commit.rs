//! island_sleep_commit pin: Rapier sleep-chunk commit over one awake island.
//!
//! One awake island of 11 bodies takes three disjoint sleep-eligible chunks
//! of 3 plus 2 awake stragglers. Whole-cover sleeps in place with an epoch
//! bump; partial cover extracts each chunk into its own sleeping island in
//! index order with the epoch held. Shuffled chunk and member order gives
//! identical bytes; a kick wakes the whole island through [`update_sleep`].

use engine_physics::islands::{
    build_islands, commit_sleeping_chunks, update_sleep, SleepCommit, SleepConfig,
};

/// One awake island of 11: a contact chain joins every body.
fn awake_eleven() -> Vec<usize> {
    let chain: Vec<(usize, usize)> = (0..10).map(|i| (i, i + 1)).collect();
    let islands = build_islands(11, &chain);
    assert_eq!(islands.len(), 1, "chain joins all 11: {islands:?}");
    islands.into_iter().next().unwrap_or_default()
}

/// Three disjoint sleep-eligible chunks of 3; bodies 9 and 10 straggle awake.
fn three_chunks() -> Vec<Vec<usize>> {
    vec![vec![0, 1, 2], vec![3, 4, 5], vec![6, 7, 8]]
}

#[test]
fn sleep_commit_whole_cover_sleeps_in_place_with_epoch_bump() {
    let awake = awake_eleven();
    let full = vec![awake.clone()];
    let got = commit_sleeping_chunks(&awake, &full, 7);
    assert_eq!(
        got,
        SleepCommit {
            awake: None,
            sleeping: vec![awake],
            epoch: 8,
        },
        "whole-cover sleeps in place with epoch 7 to 8"
    );
}

#[test]
fn sleep_commit_partial_extracts_sorted_with_epoch_held() {
    let awake = awake_eleven();
    let got = commit_sleeping_chunks(&awake, &three_chunks(), 7);
    assert_eq!(
        got,
        SleepCommit {
            awake: Some(vec![9, 10]),
            sleeping: vec![vec![0, 1, 2], vec![3, 4, 5], vec![6, 7, 8]],
            epoch: 7,
        },
        "partial cover extracts 3 islands plus 2 awake with epoch held"
    );
}

#[test]
fn sleep_commit_shuffled_order_is_byte_equal_x2() {
    let awake = awake_eleven();
    let ordered = commit_sleeping_chunks(&awake, &three_chunks(), 7);
    // Reversed chunk order with reversed members must commit identically.
    let shuffled = vec![vec![8, 7, 6], vec![5, 4, 3], vec![2, 1, 0]];
    let first = commit_sleeping_chunks(&awake, &shuffled, 7);
    let second = commit_sleeping_chunks(&awake, &shuffled, 7);
    assert_eq!(first, ordered, "shuffled order commits the same bytes");
    assert_eq!(second, ordered, "shuffled order commits the same bytes x2");
}

#[test]
fn sleep_commit_overlap_and_unknown_stay_awake_never_sleep() {
    let awake = awake_eleven();
    // Body 2 is listed twice, 40 was never awake, empties carry nothing.
    let chunks = vec![
        vec![0, 1, 2],
        vec![2, 3, 4],
        vec![],
        vec![40],
        vec![6, 7, 8],
    ];
    let got = commit_sleeping_chunks(&awake, &chunks, 7);
    assert_eq!(
        got.sleeping,
        vec![vec![0, 1], vec![3, 4], vec![6, 7, 8]],
        "the overlapping member 2 leaves every chunk"
    );
    let rest = got.awake.expect("partial cover keeps an awake remainder");
    assert!(rest.contains(&2), "overlap stays awake: {rest:?}");
    assert!(rest.contains(&40), "unknown stays awake: {rest:?}");
    assert_eq!(rest, vec![2, 5, 9, 10, 40], "remainder stays ascending");
    for island in &got.sleeping {
        assert!(!island.contains(&40), "never sleep what was not listed");
    }
    assert_eq!(got.epoch, 7, "partial cover holds the epoch");
}

#[test]
fn sleep_commit_kick_wakes_the_whole_island_with_timers_zeroed() {
    let awake = awake_eleven();
    let cfg = SleepConfig::default();
    let (mut timers, mut asleep) = (vec![0u32; 11], vec![true; 11]);
    let mut v2 = vec![0.0f32; 11];
    v2[0] = cfg.wake_v2 + 1.0;
    update_sleep(11, &[awake], &v2, &mut timers, &mut asleep, cfg);
    assert_eq!(asleep, vec![false; 11], "one kick wakes the whole island");
    assert_eq!(timers, vec![0u32; 11], "wake resets every timer");
}
