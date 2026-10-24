//! Tests for `window_audio_space`: the window's audio feed hears space,
//! surface and tension. Everything runs offline through `AudioEngine`, so no
//! device and no GPU are involved.

use std::sync::Arc;

use engine_audio::dsp::Bus;
use engine_audio::feed::{feed, Command};
use engine_audio::{AudioEngine, Clip, Cue, CueBank, FrameSource, Group, MusicBank, Pcm, Spatial};

use super::*;
use crate::audio_director::{drive, Director, TensionTable};

const RATE: u32 = 8_000;
/// Frames of one 60 Hz tick at the test rate.
const TICK_FRAMES: usize = 133;

fn bank() -> Arc<CueBank> {
    let music = MusicBank::procedural(11, RATE).expect("music bank");
    Arc::new(
        CueBank::placeholder(RATE)
            .expect("shipped table")
            .with_music(music),
    )
}

#[test]
fn slot_cap_tracks_the_canonical_participant_cap() {
    // The stride array is sized to one slot per fighter; drift is a bug.
    assert_eq!(
        MAX_SLOTS,
        usize::from(engine_input::MAX_PARTICIPANTS),
        "footstep slots must match MAX_PARTICIPANTS"
    );
}

fn view(slot: u8, x: f32, z: f32) -> FighterView {
    FighterView {
        slot,
        pos_m: [x, 0.0, z],
        yaw: 0.0,
    }
}

fn hit(attacker: u8, victim: u8) -> FightCue {
    FightCue::Hit {
        attacker,
        victim,
        damage: 120,
    }
}

fn knockdown(attacker: u8, victim: u8) -> FightCue {
    FightCue::Knockdown { attacker, victim }
}

fn outcome_with(cues: Vec<FightCue>) -> TickOutcome {
    TickOutcome {
        cues,
        ..TickOutcome::default()
    }
}

/// Per-ear energy of `frames` stereo frames.
fn ears(engine: &mut AudioEngine, frames: usize) -> (f32, f32) {
    let mut out = vec![0.0f32; frames * 2];
    engine.render_stereo_into(&mut out);
    out.chunks(2)
        .fold((0.0, 0.0), |(l, r), f| (l + f[0] * f[0], r + f[1] * f[1]))
}

/// One hit on slot 1 at `(x, z)` heard from the standard seat: eye 3 m up,
/// 6 m south of the center, looking north at it.
fn heard_hit(x: f32, z: f32) -> (f32, f32) {
    let (mut tx, mut engine) = AudioEngine::with_feed(bank(), 64, RATE, 7);
    let mut space = AudioSpace::with_parts(SpaceParts::EVENTS_ONLY);
    space.follow_camera([0.0, 3.0, 6.0], [0.0, 0.0, 0.0]);
    let views = [view(0, 0.0, 0.0), view(1, x, z)];
    space.feed(&mut tx, 0, &outcome_with(vec![hit(0, 1)]), &views);
    ears(&mut engine, 2_400)
}

#[test]
fn window_audio_space_two_hits_at_opposite_sides_pan_opposite() {
    let right = heard_hit(4.0, -2.0);
    let left = heard_hit(-4.0, -2.0);
    assert!(right.1 > right.0 * 1.5, "right hit sits right: {right:?}");
    assert!(left.0 > left.1 * 1.5, "left hit sits left: {left:?}");
    // The seat is on the middle line, so the two are mirror images.
    assert!(
        (right.1 - left.0).abs() <= right.1 * 0.01,
        "{right:?} {left:?}"
    );
    assert!(
        (right.0 - left.1).abs() <= right.0 * 0.01,
        "{right:?} {left:?}"
    );
}

#[test]
fn window_audio_space_the_listener_turns_with_the_camera() {
    // Same hit due east of the seat; only the camera's facing changes.
    let earful = |target: [f32; 3]| {
        let (mut tx, mut engine) = AudioEngine::with_feed(bank(), 64, RATE, 7);
        let mut space = AudioSpace::with_parts(SpaceParts::EVENTS_ONLY);
        space.follow_camera([0.0, 3.0, 0.0], target);
        let views = [view(0, 0.0, 0.0), view(1, 5.0, 0.0)];
        space.feed(&mut tx, 0, &outcome_with(vec![hit(0, 1)]), &views);
        (space.yaw_deg(), ears(&mut engine, 2_400))
    };
    let (yaw_north, north) = earful([0.0, 0.0, -5.0]);
    let (yaw_east, east) = earful([5.0, 0.0, 0.0]);
    let (yaw_south, south) = earful([0.0, 0.0, 5.0]);
    assert_eq!((yaw_north, yaw_east, yaw_south), (0, 90, 180));
    assert!(north.1 > north.0 * 1.5, "facing north, east is right");
    assert!(
        (east.0 - east.1).abs() <= east.0 * 0.01,
        "facing east, east is dead ahead: {east:?}"
    );
    assert!(south.0 > south.1 * 1.5, "facing south, east is left");
}

#[test]
fn window_audio_space_a_far_hit_is_quieter() {
    let near = heard_hit(1.5, 0.0);
    let far = heard_hit(1.5, -12.0);
    let (near_sum, far_sum) = (near.0 + near.1, far.0 + far.1);
    assert!(near_sum > 0.0, "the near hit is heard");
    assert!(far_sum < near_sum * 0.6, "near {near:?} far {far:?}");
}

#[test]
fn window_audio_space_every_event_carries_the_hit_position() {
    let (mut tx, mut rx) = feed(64, bank());
    let mut space = AudioSpace::with_parts(SpaceParts::EVENTS_ONLY);
    let views = [view(0, -1.0, 0.0), view(1, 3.0, 2.0)];
    let cues = vec![
        hit(0, 1),
        FightCue::Dodged {
            attacker: 0,
            victim: 1,
        },
    ];
    space.feed(&mut tx, 4, &outcome_with(cues), &views);
    let mut plays = Vec::new();
    while let Some(cmd) = rx.pop() {
        if let Command::Play(play) = cmd {
            plays.push(play);
        }
    }
    // Impact sounds at the victim's chest, the dodged swing at the
    // attacker's chest.
    assert_eq!(plays.len(), 2);
    assert_eq!(plays[0].pos_cm, Some([300, 100, 200]));
    assert_eq!(plays[1].pos_cm, Some([-100, 100, 0]));
    assert_eq!(space.stats().positioned, 2);
    // Fighters the views do not know play flat, counted.
    space.feed(&mut tx, 5, &outcome_with(vec![hit(6, 7)]), &views);
    assert_eq!(space.stats().flat, 1);
}

/// Tension commands (tick, level) a scripted fight produces, plus the level
/// the tracker ends on.
fn fight_tension() -> (Vec<(u32, Tension)>, Tension) {
    let (mut tx, mut rx) = feed(64, bank());
    let mut space = AudioSpace::with_parts(SpaceParts {
        tension: true,
        ..SpaceParts::EVENTS_ONLY
    });
    let mut director = Director::new(TensionTable::scripted()).expect("table");
    let views = [view(0, 0.0, 0.0), view(1, 1.0, 0.0)];
    let mut levels = Vec::new();
    for tick in 0..1_500u32 {
        // A quiet room, then a hot burst: hits, then a knockdown.
        let cues: Vec<FightCue> = match tick {
            30..=40 if tick % 2 == 0 => vec![hit(0, 1), hit(1, 0)],
            41 => vec![hit(0, 1), knockdown(0, 1)],
            _ => Vec::new(),
        };
        let mut outcome = outcome_with(cues);
        outcome.layers = drive(&mut director, &outcome.cues);
        space.feed(&mut tx, tick, &outcome, &views);
        while let Some(cmd) = rx.pop() {
            if let Command::Tension { tick, level } = cmd {
                levels.push((tick, level));
            }
        }
    }
    (levels, space.tension_level())
}

#[test]
fn window_audio_space_tension_rises_with_a_knockdown_and_decays() {
    let (levels, end) = fight_tension();
    assert_eq!(levels[0], (0, Tension::Calm), "the bed is told at tick 0");
    let peak = levels
        .iter()
        .map(|&(_, level)| level)
        .max()
        .expect("levels");
    let (peak_tick, _) = levels
        .iter()
        .copied()
        .find(|&(_, level)| level == peak)
        .expect("peak");
    assert!(
        peak >= Tension::Fight,
        "a knockdown fight is hot: {levels:?}"
    );
    assert!(
        (30..=60).contains(&peak_tick),
        "the rise lands in the burst, not before: {levels:?}"
    );
    assert_eq!(end, Tension::Calm, "the room cools: {levels:?}");
    // After the peak the level only falls.
    let after: Vec<Tension> = levels
        .iter()
        .filter(|&&(tick, _)| tick > peak_tick)
        .map(|&(_, level)| level)
        .collect();
    assert!(after.windows(2).all(|w| w[1] < w[0]), "{levels:?}");
    assert!(!after.is_empty(), "it decays: {levels:?}");
}

#[test]
fn window_audio_space_the_crowd_bed_rides_the_outcome_gain() {
    let (mut tx, mut engine) = AudioEngine::with_feed(bank(), 64, RATE, 3);
    let mut space = AudioSpace::with_parts(SpaceParts {
        bed: true,
        ..SpaceParts::EVENTS_ONLY
    });
    let mut tick = 0u32;
    let mut run = |bed_gain: f32| {
        let mut outcome = outcome_with(Vec::new());
        outcome.bed_gain = bed_gain;
        space.feed(&mut tx, tick, &outcome, &[]);
        tick += 1;
        let (l, r) = ears(&mut engine, TICK_FRAMES * 4);
        l + r
    };
    let full = run(1.0);
    let ducked = run(0.5);
    let gone = run(0.0);
    let back = run(1.0);
    assert!(full > 0.0, "the bed sounds");
    assert!(ducked < full * 0.5, "half level is well under full energy");
    assert_eq!(gone, 0.0, "a muted bed is silent, not restarted");
    assert!(back > full * 0.5, "and it comes back");
    assert_eq!(space.stats().bed_writes, 3, "one write per move");
}

#[test]
fn window_audio_space_a_cheer_queues_one_crowd_cheer_inside_the_cooldown() {
    // LOUD: the crowd's own reaction table owns the spacing (a cheer every
    // 45 ticks); this module plays one `crowd_cheer` per reaction it is
    // handed, so a burst inside the cooldown is one play, not one per cue.
    let (mut tx, mut rx) = feed(256, bank());
    let cheer = tx.bank().id(cue_names::CROWD_CHEER).expect("cheer cue");
    let mut space = AudioSpace::with_parts(SpaceParts {
        bed: true,
        ..SpaceParts::EVENTS_ONLY
    });
    let mut crowd = engine_audio::Crowd::new();
    let mut plays = Vec::new();
    let parried = FightCue::Parried {
        attacker: 0,
        victim: 1,
    };
    let mut fire = |space: &mut AudioSpace, tick: u32, reaction: Reaction, plays: &mut Vec<u32>| {
        let mut outcome = outcome_with(Vec::new());
        outcome.reactions.push(reaction);
        space.feed(&mut tx, tick, &outcome, &[]);
        while let Some(command) = rx.pop() {
            if let Command::Play(play) = command {
                if play.cue == cheer {
                    plays.push(tick);
                }
            }
        }
    };
    // Two parries inside the cooldown: the table answers one cheer.
    for tick in 0..3u32 {
        let reaction = crowd.on_fight_event(parried);
        if tick == 0 {
            assert!(
                matches!(reaction, Reaction::Cheer { .. }),
                "a parry is a cheer, got {reaction:?}"
            );
        }
        fire(&mut space, tick, reaction, &mut plays);
        crowd.tick();
    }
    assert_eq!(plays, vec![0], "one cheer inside the cooldown");
    // Past the cooldown the next parry cheers again.
    for tick in 3..50u32 {
        let reaction = crowd.on_fight_event(parried);
        fire(&mut space, tick, reaction, &mut plays);
        crowd.tick();
    }
    assert_eq!(
        plays.len(),
        2,
        "a second cheer after the cooldown: {plays:?}"
    );
    assert!(plays[1] >= 45, "not before CHEER_COOLDOWN_TICKS: {plays:?}");
    assert_eq!(
        usize::try_from(space.stats().cheers).expect("plays fit"),
        plays.len()
    );
}

#[test]
fn window_audio_space_walking_fighters_step_on_the_surface_cue() {
    let (mut tx, mut rx) = feed(256, bank());
    let mut space = AudioSpace::with_parts(SpaceParts {
        footsteps: true,
        ..SpaceParts::EVENTS_ONLY
    });
    let sand = tx.bank().id(cue_names::FOOTSTEP).expect("sand cue");
    let stone = tx.bank().id(cue_names::FOOTSTEP_STONE).expect("stone cue");
    let mut seen = Vec::new();
    for surface in [Surface::Sand, Surface::Stone] {
        space.set_surface(surface);
        // 5 m/s along x for one second: 5 m walked, a step per 0.8 m.
        for tick in 0..60u32 {
            let x = tick as f32 / 12.0;
            space.feed(&mut tx, tick, &outcome_with(Vec::new()), &[view(0, x, 0.0)]);
            while let Some(cmd) = rx.pop() {
                if let Command::Play(play) = cmd {
                    seen.push(play.cue);
                }
            }
        }
        // A jump across the arena is a respawn, not a stride.
        space.feed(
            &mut tx,
            61,
            &outcome_with(Vec::new()),
            &[view(0, 40.0, 0.0)],
        );
        while rx.pop().is_some() {}
    }
    let on = |id| seen.iter().filter(|&&cue| cue == id).count();
    assert!(on(sand) >= 5 && on(stone) >= 5, "{seen:?}");
    assert_eq!(on(sand) + on(stone), seen.len(), "only footsteps play");
    assert_eq!(space.stats().footsteps as usize, seen.len());
}

#[test]
fn window_audio_space_a_run_clip_plant_plays_on_its_tick_not_the_stride() {
    let (mut tx, mut rx) = feed(64, bank());
    let sand = tx.bank().id(cue_names::FOOTSTEP).expect("sand cue");
    let mut space = AudioSpace::with_parts(SpaceParts {
        footsteps: true,
        ..SpaceParts::EVENTS_ONLY
    });
    let quiet = outcome_with(Vec::new());
    let mut plays = Vec::new();
    for tick in 0..=4u32 {
        // 0.5 m a tick: two ticks walk the 0.8 m stride.
        let views = [view(0, 0.5 * tick as f32, 0.0)];
        if tick == 2 {
            // The run clip plants a foot on tick 2: one sand cue at the
            // fighter's feet, and this tick's stride stands down.
            let steps = [ClipFootstep {
                slot: 0,
                tick,
                code: FOOTSTEP_CODE,
            }];
            space.feed_clip_steps(&mut tx, tick, &steps, &views);
        }
        space.feed(&mut tx, tick, &quiet, &views);
        while let Some(cmd) = rx.pop() {
            if let Command::Play(play) = cmd {
                plays.push((play.tick, play.cue, play.pos_cm));
            }
        }
    }
    // Tick 2 is the clip's plant; tick 4 is the first stride after it, so
    // the plant reset the stride meter instead of double-tapping.
    assert_eq!(
        plays,
        vec![(2, sand, Some([100, 0, 0])), (4, sand, Some([200, 0, 0])),]
    );
    let stats = space.stats();
    assert_eq!(stats.clip_footsteps, 1);
    assert_eq!(stats.footsteps, 1, "the plant tick's stride stood down");
}

#[test]
fn window_audio_space_a_break_plays_where_the_blow_landed() {
    let (mut tx, mut rx) = feed(64, bank());
    let mut space = AudioSpace::with_parts(SpaceParts::EVENTS_ONLY);
    space.feed_break(&mut tx, 7, cue_names::IMPACT_STONE, 0.8, [1.5, 0.5, -2.0]);
    let mut plays = Vec::new();
    while let Some(cmd) = rx.pop() {
        if let Command::Play(play) = cmd {
            plays.push(play);
        }
    }
    assert_eq!(plays.len(), 1, "one write per break");
    assert_eq!(plays[0].pos_cm, Some([150, 50, -200]), "meters become cm");
    assert_eq!(space.stats().breaks, 1);
    // A bank without the sample counts the refusal instead of dropping it.
    let (mut bare_tx, _bare_rx) = feed(64, hit_only_bank());
    space.feed_break(&mut bare_tx, 8, cue_names::IMPACT_STONE, 1.0, [0.0; 3]);
    assert_eq!(space.stats().breaks, 2, "asked twice");
    assert_eq!(space.stats().unknown_cue, 1, "the bank lacks the sample");
}

/// A bank with only the hit cue: everything else is a missing clip.
fn hit_only_bank() -> Arc<CueBank> {
    let cue = Cue {
        name: cue_names::HIT.to_string(),
        variants: vec![Clip::from_pcm(&Pcm::new(RATE, 1, vec![0.2; 64]))],
        spatial: Spatial::Flat,
        group: Group::Impact,
        looped: false,
        priority: 100,
        bus: Bus::Sfx,
        gain: 1.0,
        ducks_music: false,
    };
    Arc::new(CueBank::new(vec![cue]).expect("one valid cue"))
}

#[test]
fn window_audio_space_missing_clips_are_counted_loudly() {
    let (mut tx, mut engine) = AudioEngine::with_feed(hit_only_bank(), 64, RATE, 1);
    let mut space = AudioSpace::new();
    let views = [view(0, 0.0, 0.0), view(1, 1.0, 0.0)];
    for tick in 0..10 {
        let cues = vec![hit(0, 1), knockdown(0, 1)];
        space.feed(&mut tx, tick, &outcome_with(cues), &views);
    }
    let mut out = [0.0f32; 4];
    engine.render_stereo_into(&mut out);
    let stats = tx.stats();
    // Per tick the knockdown cue and its stinger are missing; the crowd bed
    // is asked for once and then given up on.
    assert_eq!(stats.unknown_cue, 10 * 2 + 1, "{stats:?}");
    assert_eq!(space.stats().unknown_cue, stats.unknown_cue);
    assert_eq!(stats.sent, stats.applied, "what was queued was applied");
    // No music in this bank: the levels are sent and counted as unheard.
    assert!(space.stats().tension_unheard >= 1);
}

#[test]
fn window_audio_space_sent_equals_applied_over_a_scripted_duel() {
    let (mut tx, mut engine) = AudioEngine::with_feed(bank(), 1_024, RATE, 2040);
    let mut space = AudioSpace::new();
    let mut director = Director::new(TensionTable::scripted()).expect("table");
    let mut crowd = engine_audio::Crowd::new();
    for tick in 0..600u32 {
        let t = tick as f32 / 60.0;
        space.follow_camera([0.0, 4.0, 8.0], [t.sin(), 0.0, 0.0]);
        let views = [view(0, -1.0 + t * 0.2, 0.0), view(1, 1.5, t.sin())];
        let cues = if (100..140).contains(&tick) && tick % 4 == 0 {
            vec![hit(0, 1)]
        } else if tick == 300 {
            vec![hit(1, 0), knockdown(1, 0)]
        } else {
            Vec::new()
        };
        for cue in &cues {
            crowd.on_fight_event(*cue);
        }
        crowd.tick();
        let mut outcome = outcome_with(cues);
        outcome.layers = drive(&mut director, &outcome.cues);
        outcome.bed_gain = crowd.bed_gain();
        space.feed(&mut tx, tick, &outcome, &views);
        let mut out = vec![0.0f32; TICK_FRAMES * 2];
        engine.render_stereo_into(&mut out);
    }
    let mut out = [0.0f32; 4];
    engine.render_stereo_into(&mut out);
    let stats = tx.stats();
    assert_eq!(stats.sent, stats.applied, "{stats:?}");
    assert_eq!((stats.dropped_full, stats.unknown_cue), (0, 0), "{stats:?}");
    let mine = space.stats();
    assert!(mine.positioned > 0 && mine.footsteps > 0 && mine.bed_writes > 0);
    assert!(mine.tension_sends >= 2, "{mine:?}");
    assert_eq!(
        (mine.unknown_cue, mine.refused, mine.tension_unheard),
        (0, 0, 0)
    );
}

/// The takes a routed cue names over `plays` hits of the same kind, each on
/// its own tick so the round robin moves once per blow.
fn routed_takes(plays: usize) -> Vec<Option<u16>> {
    let (mut tx, mut rx) = feed(64, bank());
    let mut space = AudioSpace::with_parts(SpaceParts::EVENTS_ONLY);
    let views = [view(0, 0.0, 0.0), view(1, 1.0, 0.0)];
    let mut takes = Vec::new();
    for tick in 0..plays as u32 {
        space.feed(&mut tx, tick, &outcome_with(vec![hit(0, 1)]), &views);
        while let Some(cmd) = rx.pop() {
            if let Command::Play(play) = cmd {
                takes.push(play.take);
            }
        }
    }
    takes
}

/// SN12 game half: the window routes its impacts over the live bank, so the
/// take the round robin picked rides the feed and the mixer plays that take
/// instead of choosing its own. Takes walk without repeating one in a row.
#[test]
fn window_audio_space_routed_impacts_name_a_take_and_never_repeat_one() {
    let takes = routed_takes(6);
    assert_eq!(takes.len(), 6, "one hit per tick: {takes:?}");
    assert!(
        takes.iter().all(Option::is_some),
        "every take named: {takes:?}"
    );
    let values: Vec<u16> = takes.iter().flatten().copied().collect();
    let hit_bank = bank();
    let cue = hit_bank
        .cue(
            hit_bank
                .id(cue_names::HIT)
                .expect("the shipped bank carries the hit cue"),
        )
        .expect("id resolves");
    let limit = u16::try_from(cue.variants.len()).expect("a small take");
    assert!(
        values.iter().all(|&t| t < limit),
        "a take the cue has: {values:?} of {} takes",
        cue.variants.len()
    );
    assert!(cue.variants.len() >= 2, "the hit cue offers takes");
    assert!(
        values.windows(2).all(|w| w[0] != w[1]),
        "the same take twice in a row: {values:?}"
    );
    // Two rounds of plays cover every take of the cue.
    let mut seen: Vec<u16> = values.clone();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), cue.variants.len(), "every take heard: {seen:?}");
}
