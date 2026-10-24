//! The game crate's root tests (sessions, ticks, presentation data, budgets), moved out of
//! lib.rs so the crate root stays under its line pin. `super` is the crate root.

use super::*;
use engine_arena::{run_dataset, run_events, run_prefix};
use engine_core::alloc_probe;
use engine_input::Buttons;

const TICKS: u32 = 600;

fn config() -> GameConfig {
    GameConfig {
        seed: 2040,
        ticks: TICKS,
        fighters: 2,
        half_extent_m: 20.0,
    }
}

pub(super) fn small_config() -> GameConfig {
    GameConfig {
        seed: 7,
        ticks: 64,
        fighters: 2,
        half_extent_m: 12.0,
    }
}

pub(super) fn human(tick: u32, mx: i8, mz: i8, buttons: Buttons) -> FighterInput {
    FighterInput {
        tick,
        fighter: FighterId::new(HUMAN_SLOT).unwrap(),
        mx,
        mz,
        facing: 0,
        buttons,
    }
}

fn idle(tick: u32) -> FighterInput {
    human(tick, 0, 0, Buttons::NONE)
}

/// Scripted session input: circle-strafe with an attack every 30 ticks.
fn script(tick: u32) -> FighterInput {
    let phase = (tick / 15) % 4;
    let (mx, mz) = match phase {
        0 => (100, 0),
        1 => (0, 100),
        2 => (-100, 0),
        _ => (0, -100),
    };
    let buttons = if tick.is_multiple_of(30) {
        Buttons(engine_input::fighter::ATTACK)
    } else {
        Buttons::NONE
    };
    human(tick, mx, mz, buttons)
}

fn run_session(config: GameConfig) -> (App, Vec<TickOutcome>) {
    let mut app = App::new(config).unwrap();
    let mut outcomes = Vec::with_capacity(config.ticks as usize);
    for tick in 0..config.ticks {
        outcomes.push(app.tick(script(tick)));
    }
    (app, outcomes)
}

/// The app shell is built by value, on the stack, by the arena bin at
/// startup and by every copy of it up the call chain. It carries the
/// live match, the hud, the crowd and the directors. With the legion
/// brain inlined into the live match the shell was 39 KB, and the
/// startup chain no longer fitted the stack the process had: the arena
/// bin died at startup with a stack overflow (0xC00000FD, 2026-10-21).
/// It fits the budget now because the presentation sim stays behind a
/// pointer; the budget keeps any new big part out of the shell.
#[test]
fn the_app_shell_stays_small() {
    assert!(
        core::mem::size_of::<App>() <= 32 * 1024,
        "the app shell is {} bytes: keep the big parts behind a pointer",
        core::mem::size_of::<App>()
    );
}

/// The game bins ask the linker for a big main thread stack. The
/// window session builds its big value structs (the app shell, the live
/// match, the ring) on the stack, and the 1 MB stack Windows gives a
/// process by default does not fit them: the arena bin died at startup
/// with a stack overflow (0xC00000FD, 2026-10-21). The linker reserves
/// the main thread stack, so the ask must stay, and it must stay a
/// plain byte count this test can read.
#[test]
fn the_game_bins_ask_the_linker_for_a_big_stack() {
    const BUILD: &str = include_str!("../build.rs");
    let at = BUILD
        .find("/STACK:")
        .expect("the build script asks the linker for a stack");
    let rest = &BUILD[at + "/STACK:".len()..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let reserve: u32 = digits
        .parse()
        .expect("the stack reserve is a plain byte count");
    assert!(
        reserve >= 16 * 1024 * 1024,
        "the linker reserves {reserve} bytes of stack: the arena startup path needs more than the 1 MB Windows default"
    );
}

#[test]
fn present_every_decouples_ticks_from_frames() {
    // N>1 plays every tick but presents fewer frames with identical tick
    // hashes (GM3 present decoupling, piece 1).
    let ticks = small_config().ticks;
    let run = |every: u32| {
        let mut app = App::new(small_config()).unwrap();
        let mut presented = 0u32;
        let mut hashes = Vec::with_capacity(ticks as usize);
        for tick in 0..ticks {
            let outcome = app.tick(idle(tick));
            presented += u32::from(should_present(tick, ticks, every));
            hashes.push(outcome.checkpoint.unwrap().hash);
        }
        assert_eq!(app.cursor(), ticks);
        (presented, hashes)
    };
    let (every1, hashes1) = run(1);
    let (every10, hashes10) = run(10);
    assert_eq!(every1, ticks);
    assert!(every10 < every1, "{every10} presented of {every1}");
    assert_eq!(hashes1, hashes10);
    assert_eq!(parse_present_every("10"), Ok(10));
    assert!(parse_present_every("0").is_err());
}

#[test]
fn rejects_bad_configs() {
    assert!(App::new(config()).is_ok());
    for bad in [
        GameConfig {
            ticks: 0,
            ..config()
        },
        GameConfig {
            ticks: MAX_TICKS + 1,
            ..config()
        },
        GameConfig {
            fighters: 1,
            ..config()
        },
        GameConfig {
            // One fighter past the participant cap.
            fighters: MAX_PARTICIPANTS + 1,
            ..config()
        },
        GameConfig {
            half_extent_m: f32::NAN,
            ..config()
        },
        GameConfig {
            half_extent_m: 4.9,
            ..config()
        },
        GameConfig {
            half_extent_m: 101.0,
            ..config()
        },
    ] {
        assert!(App::new(bad).is_err(), "config accepted: {bad:?}");
    }
    // 5 fighters fits the participant cap.
    assert!(App::new(GameConfig {
        fighters: 5,
        ..config()
    })
    .is_ok());
}

#[test]
fn match_advances_and_emits_hud_and_audio_data() {
    let (app, outcomes) = run_session(small_config());
    assert_eq!(outcomes.len(), 64);
    assert_eq!(app.cursor(), 64);
    assert_eq!(app.checkpoints().len(), 64);
    for (tick, outcome) in outcomes.iter().enumerate() {
        assert!(outcome.input_accepted);
        assert_eq!(outcome.tick, tick as u32);
        let checkpoint = outcome.checkpoint.unwrap();
        assert_eq!(checkpoint.tick, tick as u32);
        assert_eq!(outcome.hud.tick, tick as u64);
        assert_eq!(outcome.audio.len(), outcome.cues.len());
        assert_eq!(outcome.reactions.len(), outcome.cues.len());
        assert!((0.0..=1.0).contains(&outcome.bed_gain));
        for command in &outcome.audio {
            assert_eq!(command.tick, tick as u32);
            assert!((0.0..=1.0).contains(&command.gain));
        }
    }
    // The scripted match moves: checkpoints are tick-indexed and the run
    // is not one repeated hash.
    let mut distinct = outcomes
        .iter()
        .filter_map(|outcome| outcome.checkpoint.map(|checkpoint| checkpoint.hash))
        .collect::<Vec<_>>();
    distinct.sort_unstable();
    distinct.dedup();
    assert!(distinct.len() > 1, "match never moved");
}

/// Route the whole impact matrix through the shipped bank with the
/// picker shape the live mixer holds (one seeded stream per cue), and
/// return distinct clips, immediate repeats, kinds with a cue and the
/// heap count of the routing loop.
fn route_impact_matrix(
    bank: &engine_audio::CueBank,
    seed: u64,
    picks: &mut [(u16, usize)],
) -> (usize, usize, usize, alloc_probe::AllocCount) {
    let mut pickers: Vec<engine_audio::VariantPicker> = (0..bank.len())
        .map(|idx| {
            engine_audio::VariantPicker::new(
                seed,
                engine_audio::CueId(u16::try_from(idx).expect("bank fits a cue id")),
            )
        })
        .collect();
    let mut pointers = [0usize; 16 * engine_audio::MAX_VARIANTS];
    let mut kinds = [false; ImpactKind::COUNT];
    let mut distinct = 0usize;
    let mut repeats = 0usize;
    let ((), probe) = alloc_probe::measure(|| {
        let mut last = usize::MAX;
        for round in 0usize..2 {
            for kind in ImpactKind::ALL {
                for surface in ImpactSurface::ALL {
                    let id = bank
                        .id(impact_cue(kind, surface))
                        .expect("the table only names shipped cues");
                    let cue = bank.cue(id).expect("id resolves");
                    let variant = pickers[usize::from(id.0)].pick(cue.variants.len());
                    let clip = cue.variants[variant].samples.as_ptr() as usize;
                    kinds[kind.index()] = true;
                    if !pointers[..distinct].contains(&clip) {
                        pointers[distinct] = clip;
                        distinct += 1;
                    }
                    if clip == last {
                        repeats += 1;
                    }
                    last = clip;
                    let slot = round * ImpactKind::COUNT * ImpactSurface::COUNT
                        + kind.index() * ImpactSurface::COUNT
                        + surface.index();
                    picks[slot] = (id.0, variant);
                }
            }
        }
    });
    (
        distinct,
        repeats,
        kinds.iter().filter(|seen| **seen).count(),
        probe,
    )
}

/// SN12 gate (`engine-game --lib hit_matrix`): the kind-by-surface table
/// names 8 of 8 event kinds on the shipped bank, the seeded pickers reach
/// 12 or more distinct clips and never play one twice in a row, and the
/// routing loop allocates nothing. Same seed, same takes.
#[test]
fn hit_matrix_eight_kinds_twelve_clips_no_repeat_no_alloc() {
    let plays = 2 * ImpactKind::COUNT * ImpactSurface::COUNT;
    let bank = engine_audio::CueBank::placeholder(48_000).expect("shipped table is valid");
    // The ledger's size belongs to the audio lane's shipped cue table;
    // this gate pins that the bank carries every cue of it, not a
    // hardcoded count that goes stale when the table grows.
    assert_eq!(
        bank.len(),
        engine_audio::ARENA_CUES.len(),
        "the clip ledger carries the shipped cue table"
    );
    let mut first = vec![(0u16, 0usize); plays];
    let (distinct, repeats, kinds, probe) = route_impact_matrix(&bank, 2040, &mut first);
    assert!(distinct >= 12, "distinct clips {distinct} < 12");
    assert_eq!(repeats, 0, "the same clip played twice in a row");
    assert_eq!(kinds, ImpactKind::COUNT, "kinds with a cue {kinds}/8");
    assert_eq!(
        probe.calls, 0,
        "impact routing allocated {} calls",
        probe.calls
    );
    println!(
        "hit_matrix distinct_clips={distinct} repeat_run={repeats} kinds_with_cue={kinds}/8 allocations={}",
        probe.calls
    );
    let mut again = vec![(0u16, 0usize); plays];
    let (_, _, _, _) = route_impact_matrix(&bank, 2040, &mut again);
    assert_eq!(first, again, "same seed routes the same takes");
}

/// FEEL8: the live fold's hit level follows the damage it carries. A
/// standard blow plays full; a light one plays softer on the same flesh cue.
#[test]
fn live_fold_hit_gain_follows_damage() {
    let mut app = App::new(small_config()).expect("config");
    let mut out = TickOutcome::default();
    let mut fold = |damage: u16| {
        out.audio.clear();
        let hit = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage,
        };
        app.fold_cue_into(7, hit, &mut out);
        out.audio.first().copied().expect("a hit emits a voice")
    };
    let light = fold(60);
    let standard = fold(120);
    assert_eq!(light.cue, impact_cue(ImpactKind::Hit, ImpactSurface::Flesh));
    assert_eq!((light.gain, standard.gain), (0.75, 1.0));
}

/// The live fold reads the same table: each impact event names the cue
/// its kind and the surface the arena knows (a held block is wood, a
/// parry is steel on steel) resolve to.
#[test]
fn hit_matrix_live_fold_names_the_impact_cue() {
    let mut app = App::new(small_config()).expect("config");
    let mut out = TickOutcome::default();
    let cases = [
        (
            FightCue::Hit {
                attacker: 0,
                victim: 1,
                damage: 120,
            },
            ImpactKind::Hit,
            ImpactSurface::Flesh,
        ),
        (
            FightCue::Blocked {
                attacker: 0,
                victim: 1,
                chip: 24,
            },
            ImpactKind::Block,
            ImpactSurface::ShieldWood,
        ),
        (
            FightCue::Parried {
                attacker: 0,
                victim: 1,
            },
            ImpactKind::Parry,
            ImpactSurface::Plate,
        ),
        (
            FightCue::Knockdown {
                attacker: 0,
                victim: 1,
            },
            ImpactKind::Knockdown,
            ImpactSurface::Flesh,
        ),
    ];
    for (event, kind, surface) in cases {
        out.audio.clear();
        app.fold_cue_into(7, event, &mut out);
        let command = out.audio.first().expect("an impact emits a voice");
        assert_eq!(command.cue, impact_cue(kind, surface), "{event:?}");
    }
}

#[test]
fn into_duel_moves_a_fresh_session_to_tables_v2_and_refuses_later() {
    use engine_combat::tables::{TABLES_VERSION_V1, TABLES_VERSION_V2};
    // Default sessions stay on the frozen v1 rules (the LAN authority
    // builds from the same v1 config).
    let v1 = App::new(small_config()).unwrap();
    assert_eq!(v1.arena.table_version, TABLES_VERSION_V1);
    assert_eq!(v1.rules, ArenaRules::V1);
    let duel = App::new(small_config()).unwrap().into_duel().unwrap();
    assert_eq!(duel.arena.table_version, TABLES_VERSION_V2);
    assert_eq!(duel.rules, ArenaRules::v2_free_for_all());
    assert_eq!(duel.live.rules(), duel.rules);
    // Same inputs: v2 checkpoints differ from v1 and repeat exactly.
    let hashes = |mut app: App| -> Vec<u64> {
        (0..24)
            .filter_map(|tick| app.tick(script(tick)).checkpoint.map(|cp| cp.hash))
            .collect()
    };
    let first = hashes(App::new(small_config()).unwrap().into_duel().unwrap());
    assert_eq!(first.len(), 24);
    assert_eq!(
        first,
        hashes(App::new(small_config()).unwrap().into_duel().unwrap())
    );
    assert_ne!(first, hashes(App::new(small_config()).unwrap()));
    // After the first tick the tables are fixed.
    let mut ticked = App::new(small_config()).unwrap();
    ticked.tick(script(0));
    assert!(matches!(
        ticked.into_duel(),
        Err(GameError::BadConfig("duel_tables"))
    ));
}

/// EG44: the window plays rules v2 through `into_duel`
/// (`windowed.rs::run_windowed`), never through `GameConfig`, so
/// `App::new` stays v1 by design; both constructors must agree on that v1
/// start and on the v2 duel they then select.
#[test]
fn app_new_equals_new_for_slot_on_rules_v2() {
    use engine_combat::fight::{FIGHT_RULES_V1, FIGHT_RULES_V2};
    use engine_combat::tables::TABLES_VERSION_V1;
    let plain = App::new(small_config()).unwrap();
    let slotted = App::new_for_slot(small_config(), HUMAN_SLOT).unwrap();
    assert_eq!(
        plain.rules.version, FIGHT_RULES_V1,
        "App::new stays v1 by design"
    );
    assert_eq!(plain.arena.table_version, TABLES_VERSION_V1);
    assert_eq!(plain.rules, slotted.rules);
    assert_eq!(plain.human_slot(), slotted.human_slot());
    assert_eq!(plain.spec(), slotted.spec(), "one v1 identity");
    // The v2 match the window builds: the same `into_duel` step.
    let mut plain = plain.into_duel().unwrap();
    let mut slotted = slotted.into_duel().unwrap();
    assert_eq!(plain.rules.version, FIGHT_RULES_V2);
    assert_eq!(plain.rules, slotted.rules);
    assert_eq!(plain.spec(), slotted.spec(), "one v2 identity");
    let mut plain_hashes = Vec::new();
    let mut slotted_hashes = Vec::new();
    for tick in 0..8 {
        let a = plain.tick(script(tick)).checkpoint.expect("duel tick");
        let b = slotted.tick(script(tick)).checkpoint.expect("duel tick");
        assert_eq!(a, b, "tick {tick}");
        plain_hashes.push(a.hash);
        slotted_hashes.push(b.hash);
    }
    assert_eq!(plain_hashes, slotted_hashes, "the same v2 hashes");
    assert_eq!(
        plain.checkpoints(),
        slotted.checkpoints(),
        "the same v2 checkpoints"
    );
}

/// DSN1: the window session and the net server and client, all built from
/// one spec, play the same match: equal state hash on every tick of a
/// scripted 300-tick duel on the kit walls.
#[test]
fn spec_window_server_and_client_play_one_match_for_300_ticks() {
    use engine_net::session::{Client, Server};
    let config = GameConfig {
        seed: 0x5EED,
        ticks: 300,
        fighters: 4,
        half_extent_m: 18.0,
    };
    let mut window = App::with_world(config, crate::arena_kit::kit_match_world_or_empty())
        .unwrap()
        .into_duel()
        .unwrap();
    let spec = MatchSpec::kit_duel(config.arena_config())
        .expect("kit duel")
        .with_weapons(&[DUEL_KIT_WEAPON; 4]);
    assert_eq!(
        window.spec().world_digest(),
        spec.world_digest(),
        "the game and the arena load one kit world"
    );
    assert_eq!(window.spec().identity(), spec.identity());
    let mut server = Server::from_spec(window.spec()).expect("server");
    let mut client =
        Client::from_spec(window.spec(), FighterId::new(HUMAN_SLOT).unwrap()).expect("client");
    let mut checkpoints = Vec::new();
    for tick in 0..config.ticks {
        let input = script(tick);
        let seen = window.tick(input).checkpoint.expect("window tick");
        let (authored, _) = server.tick(&[input]).expect("server tick");
        let predicted = client.predict(input).expect("client predict");
        assert_eq!(seen, authored, "window vs server at tick {tick}");
        assert_eq!(seen, predicted, "window vs client at tick {tick}");
        checkpoints.push(seen.hash);
    }
    let default = App::new(config).unwrap().spec();
    assert_eq!(default.rules, ArenaRules::V1, "the default stays v1");
    assert_eq!(default.world_digest(), 0, "the default stays open sand");
    let mut v1 = LiveMatch::new(&config.arena_config()).unwrap();
    let v1_hashes: Vec<u64> = (0..config.ticks)
        .map(|tick| v1.step(&[script(tick)]).unwrap().checkpoint.hash)
        .collect();
    assert_ne!(checkpoints, v1_hashes, "the spec is not the old default");
}

#[test]
fn attack_started_tell_surfaces_on_duel_press_tick() {
    // EG19b remainder: the game surfaces the v2 press tell on the tick
    // outcome (the swing clip starts from these); v1 sessions stay silent.
    use engine_input::fighter::ATTACK;
    let mut v1 = App::new(small_config()).unwrap();
    let pressed = v1.tick(human(0, 0, 0, Buttons(ATTACK)));
    assert!(pressed.input_accepted, "press accepted");
    assert!(
        pressed.started.is_empty(),
        "v1 rows cue nothing: {:?}",
        pressed.started
    );
    let mut duel = App::new(small_config()).unwrap().into_duel().unwrap();
    let first = duel.tick(human(0, 0, 0, Buttons(ATTACK)));
    assert!(first.input_accepted, "duel press accepted");
    assert_eq!(
        first.started,
        vec![HUMAN_SLOT],
        "press tick tells its starter once"
    );
    for slot in &first.started {
        assert!(*slot < small_config().fighters, "tell names a live slot");
    }
    let second = duel.tick(idle(1));
    assert!(second.input_accepted, "idle accepted");
    assert!(
        !second.started.contains(&HUMAN_SLOT),
        "the tell never lingers: {:?}",
        second.started
    );
    // Same duel script tells the same starter twice.
    let mut replay = App::new(small_config()).unwrap().into_duel().unwrap();
    let again = replay.tick(human(0, 0, 0, Buttons(ATTACK)));
    assert_eq!(again.started, first.started, "tell replays");
}

#[test]
fn hitstop_hold_sets_four_on_hit_then_counts_down() {
    // Feel LF10: the sim freezes 4 ticks on impact; the app mirrors the
    // count read-only so the look can hold clip clocks (presentation
    // only, never hashed). The scripted bout lands Hit at ticks 4 and 34.
    let mut app = App::new(small_config()).unwrap();
    assert_eq!(app.hitstop_remaining(), 0);
    assert!(!App::hitstop_holds(app.hitstop_remaining()));
    let ticks = small_config().ticks;
    let mut counts: Vec<u32> = Vec::with_capacity(usize::try_from(ticks).expect("ticks fit"));
    let mut seen: Option<u32> = None;
    for tick in 0..ticks {
        let outcome = app.tick(script(tick));
        assert!(outcome.input_accepted, "tick {tick}");
        counts.push(app.hitstop_remaining());
        if seen.is_none()
            && outcome
                .cues
                .iter()
                .any(|cue| matches!(cue, FightCue::Hit { .. }))
        {
            seen = Some(tick);
        }
    }
    let impact = seen.expect("scripted bout must land a Hit");
    let base = usize::try_from(impact).expect("tick fits");
    assert_eq!(counts[base], 4, "hitstop equals 4 right after impact");
    for (step, want) in [3u32, 2, 1, 0].iter().enumerate() {
        let offset = u32::try_from(step).expect("step fits");
        let at_tick = impact + 1 + offset;
        let at = base + 1 + step;
        assert_eq!(counts[at], *want, "tick {at_tick}");
    }
    assert!(App::hitstop_holds(4));
    assert!(App::hitstop_holds(1));
    assert!(!App::hitstop_holds(0));
}

#[test]
fn hitstop_mirror_is_the_live_count_under_v2() {
    // FEEL3: under v2 rows the App mirror is the sim's own freeze, read from
    // the live fight after each step, never a second count. The per-kind
    // freeze values (light 3, heavy 6, parry 8) are pinned against the sim's
    // rows by the look and feel tests, which feed those counts directly.
    let mut app = App::new(small_config()).unwrap().into_duel().unwrap();
    let ticks = small_config().ticks;
    for tick in 0..ticks {
        let outcome = app.tick(script(tick));
        assert!(outcome.input_accepted, "tick {tick}");
        assert_eq!(
            app.hitstop_remaining(),
            app.live_match().hitstop(),
            "tick {tick}: the mirror is the live count"
        );
    }
}

#[test]
fn hitstop_hold_predicate_true_at_four_and_one_false_at_zero() {
    assert!(App::hitstop_holds(4));
    assert!(App::hitstop_holds(1));
    assert!(!App::hitstop_holds(0));
    let fresh = App::new(small_config()).unwrap();
    assert_eq!(fresh.hitstop_remaining(), 0);
    assert!(!App::hitstop_holds(fresh.hitstop_remaining()));
}

#[test]
fn headless_600_tick_session_identical_twice() {
    let (first_app, first_outcomes) = run_session(config());
    let (second_app, second_outcomes) = run_session(config());
    assert_eq!(first_app.checkpoints(), second_app.checkpoints());
    assert_eq!(first_app.checkpoints().len(), TICKS as usize);
    assert_eq!(first_app.hp(), second_app.hp());
    assert_eq!(first_app.hud().state_hash(), second_app.hud().state_hash());
    assert_eq!(first_app.progression(), second_app.progression());
    for (first, second) in first_outcomes.iter().zip(second_outcomes.iter()) {
        assert_eq!(first, second);
    }
}

#[test]
fn hp_and_progression_accounting_match_emitted_cues() {
    let (app, outcomes) = run_session(config());
    let mut hp = [MAX_HP_PERMILLE; MAX_PARTICIPANTS as usize];
    let (mut wins, mut losses) = (0u32, 0u32);
    let mut knockdowns = 0u32;
    for outcome in &outcomes {
        for cue in &outcome.cues {
            match *cue {
                FightCue::Hit { victim, damage, .. } => {
                    hp[victim as usize] = hp[victim as usize].saturating_sub(damage);
                }
                FightCue::Blocked { victim, chip, .. } => {
                    hp[victim as usize] = hp[victim as usize].saturating_sub(chip);
                }
                FightCue::Knockdown { attacker, victim } => {
                    hp[victim as usize] = 0;
                    knockdowns += 1;
                    if attacker == HUMAN_SLOT {
                        wins += 1;
                    }
                    if victim == HUMAN_SLOT {
                        losses += 1;
                    }
                }
                FightCue::Parried { .. }
                | FightCue::Dodged { .. }
                | FightCue::StaminaOut { .. } => {}
            }
        }
    }
    assert_eq!(app.hp(), &hp);
    let progress = app.progression();
    assert_eq!((progress.wins, progress.losses), (wins, losses));
    assert_eq!(progress.matches_completed, u32::from(knockdowns > 0));
    assert_eq!(progress.match_seed, config().seed);
    assert_eq!(progress.match_ticks, TICKS);
    assert_eq!(app.hud().hp, u32::from(hp[HUMAN_SLOT as usize]));
}

#[test]
fn invalid_inputs_rejected_without_advancing() {
    let mut app = App::new(small_config()).unwrap();
    let accepted = app.tick(idle(999));
    assert!(
        accepted.input_accepted,
        "caller tick is stamped, not trusted"
    );
    assert_eq!(app.cursor(), 1);

    let wrong_slot = FighterInput {
        fighter: FighterId::new(1).unwrap(),
        ..idle(0)
    };
    let rejected = app.tick(wrong_slot);
    assert!(!rejected.input_accepted);
    assert_eq!(app.cursor(), 1);
    assert_eq!(rejected.checkpoint, accepted.checkpoint);

    let bad_wish = human(0, 101, 0, Buttons::NONE);
    let rejected = app.tick(bad_wish);
    assert!(!rejected.input_accepted);
    assert_eq!(app.cursor(), 1);

    // First input rejected before tick 0: no checkpoint yet, empty data.
    let mut fresh = App::new(small_config()).unwrap();
    let rejected = fresh.tick(wrong_slot);
    assert!(!rejected.input_accepted);
    assert_eq!(rejected.checkpoint, None);
    assert!(rejected.audio.is_empty() && rejected.cues.is_empty());
    assert_eq!(fresh.cursor(), 0);

    // Exhausted match rejects without moving.
    let mut app = App::new(GameConfig {
        ticks: 2,
        ..small_config()
    })
    .unwrap();
    assert!(app.tick(idle(0)).input_accepted);
    assert!(app.tick(idle(1)).input_accepted);
    assert!(!app.tick(idle(2)).input_accepted);
    assert_eq!(app.cursor(), 2);
}

fn progress_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("e2040-game-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn save_load_roundtrip_preserves_progression_and_seed() {
    // `save_progress` mirrors through `steam::probe`, which reads the
    // process-global Steam switches: share the Steam tests' env lock so a
    // mirror here can never land inside their cloud dir mid-window.
    let _env = crate::steam::ENV_LOCK.lock().expect("steam env lock");
    let dir = progress_dir("roundtrip");
    let (app, _) = run_session(small_config());
    app.save_progress(&dir, 3).unwrap();
    let loaded = App::load_progress(&dir, 3).unwrap();
    assert_eq!(loaded.progress, app.progression());
    assert_eq!(loaded.envelope_seed, small_config().seed);
    assert_eq!(loaded.envelope_tick, u64::from(app.cursor()));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn failed_save_write_preserves_prior_valid_generation() {
    // Same env-lock owner as the Steam tests: this save may mirror, and
    // the Steam switches are process-global.
    let _env = crate::steam::ENV_LOCK.lock().expect("steam env lock");
    let dir = progress_dir("recover");
    let (app, _) = run_session(small_config());
    app.save_progress(&dir, 1).unwrap();
    let before = App::load_progress(&dir, 1).unwrap();
    // Bad slot: rejected before touching the filesystem.
    assert_eq!(app.save_progress(&dir, 99), Err(GameError::BadSlot(99)));
    assert_eq!(App::load_progress(&dir, 1).unwrap(), before);
    // Missing directory: IO error, prior generation intact.
    let missing = dir.join("no-such-dir");
    assert!(app.save_progress(&missing, 1).is_err());
    assert_eq!(App::load_progress(&dir, 1).unwrap(), before);
    // Corrupt bytes in the slot: load fails loudly.
    std::fs::write(dir.join("save01.e2040"), b"not a save").unwrap();
    assert!(App::load_progress(&dir, 1).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn dropping_the_presentation_cursor_keeps_ticks_and_serves_legs_on_demand() {
    let config = small_config();
    let (live, live_out) = run_session(config);
    let mut lean = App::new(config).unwrap();
    lean.set_presentation_cursor(false);
    let mut lean_out = Vec::new();
    for tick in 0..config.ticks {
        lean_out.push(lean.tick(script(tick)));
    }
    // Same authority: checkpoints, cues and views match tick for tick.
    assert_eq!(live.checkpoints(), lean.checkpoints());
    assert_eq!(live_out, lean_out);
    assert_eq!(live.fighter_views(), lean.fighter_views());
    // No rolling cursor, yet the full legs still replay on demand.
    assert!(lean.pose_cursor().is_empty());
    assert_eq!(lean.presentation(), live.presentation());
    assert_eq!(
        lean.pose(PoseSource::LocoPresentation).unwrap(),
        live.pose(PoseSource::LocoPresentation).unwrap()
    );
}

#[test]
fn presentation_calls_run_loco_poses_for_the_movement_path() {
    let (app, _) = run_session(small_config());
    let first = app.presentation();
    assert_eq!(first.pose_tracks.len(), small_config().fighters as usize);
    for track in &first.pose_tracks {
        assert_eq!(track.len(), app.cursor() as usize);
    }
    assert_eq!(first.root_tables.len(), small_config().fighters as usize);
    // Same prefix twice gives the same tracks.
    assert_eq!(first, app.presentation());
    // Poses validate against the locomotion presentation skeleton.
    let skeleton = engine_anim::locomotion::loco_skeleton();
    for track in &first.pose_tracks {
        engine_anim::contract::validate_pose(&skeleton, &track[0]).unwrap();
    }
    // The arena movement path serves the latest pose; the imported fighter is the
    // documented art-hook stub.
    assert_eq!(
        app.pose(PoseSource::LocoPresentation).unwrap(),
        first.pose_tracks[0].last().cloned()
    );
    assert_eq!(
        app.pose(PoseSource::ImportedFighter),
        Err(GameError::ArtDeferred)
    );
    // Empty before tick 0, never a failure.
    let fresh = App::new(small_config()).unwrap();
    assert!(fresh.presentation().pose_tracks.is_empty());
    assert_eq!(fresh.pose(PoseSource::LocoPresentation).unwrap(), None);
}

/// Every cue shape the fold handles, ending on a knockdown pair so the
/// refill path also covers the kill log and a full feed.
fn burst_cues() -> Vec<FightCue> {
    vec![
        FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 120,
        },
        FightCue::Blocked {
            attacker: 1,
            victim: 0,
            chip: 10,
        },
        FightCue::Parried {
            attacker: 0,
            victim: 1,
        },
        FightCue::Dodged {
            attacker: 1,
            victim: 0,
        },
        FightCue::StaminaOut { fighter: 0 },
        FightCue::Knockdown {
            attacker: 0,
            victim: 1,
        },
        FightCue::Hit {
            attacker: 1,
            victim: 0,
            damage: 40,
        },
        FightCue::Knockdown {
            attacker: 0,
            victim: 1,
        },
    ]
}

/// Close-range assault: attacks every 5 ticks so the session lands real
/// cues (hits, blocks, knockdowns) instead of idling.
fn assault(tick: u32) -> FighterInput {
    let phase = (tick / 15) % 4;
    let (mx, mz) = match phase {
        0 => (100, 0),
        1 => (0, 100),
        2 => (-100, 0),
        _ => (0, -100),
    };
    let buttons = if tick.is_multiple_of(5) {
        Buttons(engine_input::fighter::ATTACK)
    } else {
        Buttons::NONE
    };
    human(tick, mx, mz, buttons)
}

#[test]
fn default_outcome_matches_rejected_before_tick_zero() {
    let out = TickOutcome::default();
    assert!(!out.input_accepted);
    assert_eq!(out.tick, 0);
    assert_eq!(out.checkpoint, None);
    assert!(out.audio.is_empty() && out.cues.is_empty() && out.reactions.is_empty());
    assert!(out.started.is_empty(), "fresh outcome tells nothing");
    assert_eq!(out.bed_gain, 1.0);
    assert_eq!(out.layers.murmur, 1.0);
    // A fresh session's first rejection carries the same shape.
    let mut fresh = App::new(small_config()).unwrap();
    let wrong_slot = FighterInput {
        fighter: FighterId::new(1).unwrap(),
        ..idle(0)
    };
    let rejected = fresh.tick(wrong_slot);
    assert!(!rejected.input_accepted);
    assert_eq!(rejected.tick, out.tick);
    assert_eq!(rejected.checkpoint, out.checkpoint);
    assert_eq!(rejected.hud, out.hud);
    assert_eq!(rejected.bed_gain, out.bed_gain);
    assert_eq!(rejected.layers, out.layers);
}

#[test]
fn refilled_outcome_matches_fresh_tick_for_tick() {
    // A forgotten clear (stale tail) or a missed HUD sync fails here:
    // one app collects owned outcomes via `tick`, a twin refills a
    // single reused outcome with the same cues and must match exactly.
    let config = GameConfig {
        ticks: 200,
        ..small_config()
    };
    let mut fresh_app = App::new(config).unwrap();
    let mut refill_app = App::new(config).unwrap();
    let mut reused = TickOutcome::default();
    let mut total_cues = 0usize;
    let mut total_breaks = 0usize;
    for tick in 0..config.ticks {
        let fresh = fresh_app.tick(assault(tick));
        assert!(fresh.input_accepted, "assault inputs validate");
        let checkpoint = fresh.checkpoint.unwrap();
        let cues = fresh.cues.clone();
        let started = fresh.started.clone();
        total_cues += cues.len();
        total_breaks += fresh.breaks.len();
        // The twin never simulates, so it has no authority break events of its own: hand it
        // the fresh tick's, the way `simulate_tick` fills the scratch for `observe_into`.
        refill_app.tick_breaks.extend_from_slice(&fresh.breaks);
        refill_app.tick_cuts.extend_from_slice(&fresh.cuts);
        refill_app.observe_into(fresh.tick, &cues, &started, checkpoint, &mut reused);
        assert_eq!(reused, fresh, "refilled outcome diverged at tick {tick}");
        assert!(
            refill_app.tick_breaks.is_empty(),
            "the refill moves the break scratch out, tick {tick}"
        );
        assert!(refill_app.tick_cuts.is_empty(), "cut scratch at {tick}");
    }
    assert!(
        total_cues > 0,
        "session must land cues or the check is vacuous"
    );
    assert!(
        total_breaks > 0,
        "session must break bonds or the break leg is vacuous"
    );
    // Folded session state matches too, not just the outcomes. The
    // cursor-derived `match_ticks` is sim state, not observe-fold
    // state, so only the folded counters and seed compare.
    assert_eq!(refill_app.hp(), fresh_app.hp());
    assert_eq!(refill_app.hud(), fresh_app.hud());
    let (refill_progress, fresh_progress) = (refill_app.progression(), fresh_app.progression());
    assert_eq!(
        refill_progress.matches_completed,
        fresh_progress.matches_completed
    );
    assert_eq!(refill_progress.wins, fresh_progress.wins);
    assert_eq!(refill_progress.losses, fresh_progress.losses);
    assert_eq!(refill_progress.match_seed, fresh_progress.match_seed);
}

#[test]
fn observe_refill_allocates_nothing_at_steady_state() {
    // Calibration: the shared probe must observe a real allocation,
    // otherwise a zero reading would prove nothing.
    let (_, probe) = alloc_probe::measure(|| std::hint::black_box(vec![0u8; 64]));
    assert!(probe.calls >= 1, "alloc probe is blind");

    let mut app = App::new(small_config()).unwrap();
    // Seed checkpoint from a real accepted tick (window closed).
    let first = app.tick(idle(0));
    assert!(first.input_accepted);
    let checkpoint = first.checkpoint.unwrap();

    let mut out = TickOutcome::default();
    let burst = burst_cues();
    // Warmup with the window closed: sizes every buffer on the path
    // (outcome vecs, kill log, full kill feed) to steady capacity, and
    // ends on a full burst so the steady-state shape asserts below.
    for tick in 1..=24u32 {
        let cues = if tick % 4 == 0 { &[][..] } else { &*burst };
        app.observe_into(tick, cues, &[], checkpoint, &mut out);
    }
    app.observe_into(25, &burst, &[], checkpoint, &mut out);
    std::hint::black_box(&out);
    assert_eq!(out.cues.len(), burst.len());
    assert_eq!(out.audio.len(), 6);
    assert_eq!(out.reactions.len(), burst.len());
    assert_eq!(app.hud().feed.len(), app.hud().feed.cap());

    // Steady state: empty, single, partial, full and knockdown-tail
    // refills each perform zero heap allocations.
    let measured: [&[FightCue]; 5] = [&[], &burst[..1], &burst[..6], (&*burst), &burst[6..]];
    for (round, cues) in measured.iter().enumerate() {
        let ((), steady) = alloc_probe::measure(|| {
            app.observe_into(1000 + round as u32, cues, &[], checkpoint, &mut out);
            std::hint::black_box(&out);
        });
        assert_eq!(
            steady.calls,
            0,
            "observe_into allocated on {} cues",
            cues.len()
        );
        assert_eq!(
            out.cues.len(),
            cues.len(),
            "refill must carry exactly the tick's cues"
        );
    }
    assert_eq!(out.cues, burst[6..]);
    assert!(!out.audio.is_empty(), "final refill did real work");
}

#[test]
fn tick_into_matches_tick_byte_for_byte() {
    // Production wiring proof: the reused-outcome path returns the same
    // bytes as the owned path on accepted, rejected and empty ticks.
    let mut fresh = App::new(small_config()).unwrap();
    let mut reused = App::new(small_config()).unwrap();
    let mut scratch = TickOutcome::default();
    for tick in 0..32u32 {
        let expected = fresh.tick(idle(tick));
        reused.tick_into(idle(tick), &mut scratch);
        assert_eq!(scratch, expected, "tick {tick} must refill identically");
    }
    // Rejected inputs repeat the last accepted data on both paths.
    let bad = FighterInput {
        fighter: FighterId::new(1).unwrap(),
        ..idle(0)
    };
    let expected = fresh.tick(bad);
    reused.tick_into(bad, &mut scratch);
    assert_eq!(scratch, expected, "rejected ticks must refill identically");
    assert!(!scratch.input_accepted);
}

#[test]
fn populated_hud_sync_reuses_buffers() {
    // sync_hud paths fighters/feed/map rows: pre-fill both sides fully,
    // then prove the sync performs zero heap allocations. Production
    // never populates these rows today (upsert_* uncalled outside ui
    // tests), so this guards the mechanism, not a live path.
    use engine_ui::{FighterHud, Hud};
    let mut src = Hud::new(0, 1000, 1000, 0, 0);
    let mut dst = Hud::new(0, 1000, 1000, 0, 0);
    for slot in 0..MAX_PARTICIPANTS {
        let id = FighterId::new(slot).expect("slot in cap");
        src.upsert_fighter(FighterHud::new(id, 1000, 1000)).unwrap();
        if slot < 8 {
            src.upsert_dot(id, 1.0, 2.0).unwrap();
        }
    }
    sync_hud(&mut dst, &src);
    let ((), steady) = alloc_probe::measure(|| {
        sync_hud(&mut dst, &src);
        std::hint::black_box(&dst);
    });
    assert_eq!(steady.calls, 0, "populated sync must not alloc");
    assert_eq!(dst.fighters.len(), usize::from(MAX_PARTICIPANTS));
}

/// Full replay spelling for the first `ticks` of a session's tape: the
/// legs the rolling cursor must stay byte-equal to.
fn replay_reference(app: &App, ticks: u32) -> PresentationData {
    App::prefix_presentation(&app.arena, &app.tape, ticks)
}

#[test]
fn rolling_cursor_matches_full_replay_tick_for_tick() {
    let config = GameConfig {
        ticks: 48,
        ..small_config()
    };
    let mut app = App::new(config).unwrap();
    assert!(app.pose_cursor().is_empty());
    assert_eq!(app.presentation_len(), 0);
    for tick in 0..config.ticks {
        let out = app.tick(assault(tick));
        assert!(out.input_accepted, "assault inputs validate");
        assert_eq!(
            app.presentation_len(),
            tick + 1,
            "cursor covers the accepted tick"
        );
        assert_eq!(
            app.pose_cursor().len(),
            usize::from(config.fighters),
            "one latest pose per fighter"
        );
        let expected = replay_reference(&app, tick + 1);
        assert_eq!(
            app.presentation(),
            expected,
            "rolling tracks diverge at tick {tick}"
        );
        assert_eq!(
            app.presentation_ref(),
            &expected,
            "reference read diverges at tick {tick}"
        );
        let tails: Vec<LocalPose> = expected
            .pose_tracks
            .iter()
            .map(|track| track.last().cloned().expect("covered tick has a pose"))
            .collect();
        assert_eq!(app.pose_cursor(), &*tails);
    }
    // P2P twice: a twin session serves byte-equal poses.
    let mut twin = App::new(config).unwrap();
    for tick in 0..config.ticks {
        twin.tick(assault(tick));
    }
    assert_eq!(app.presentation(), twin.presentation());
    assert_eq!(app.pose_cursor(), twin.pose_cursor());
}

#[test]
fn pose_cursor_advances_only_on_accepted_ticks() {
    let mut app = App::new(small_config()).unwrap();
    let first = app.tick(idle(0));
    assert!(first.input_accepted);
    let held = app.pose_cursor().to_vec();
    assert_eq!(held.len(), usize::from(small_config().fighters));
    // Rejected input: the cursor and the presentation both hold.
    let bad = FighterInput {
        fighter: FighterId::new(1).unwrap(),
        ..idle(0)
    };
    let rejected = app.tick(bad);
    assert!(!rejected.input_accepted);
    assert_eq!(app.cursor(), 1);
    assert_eq!(app.presentation_len(), 1);
    assert_eq!(app.pose_cursor(), &*held);
    // AI-filled ticks advance too.
    let out = app.tick_empty();
    assert!(out.input_accepted);
    assert_eq!(app.presentation_len(), 2);
    assert_eq!(
        app.pose_cursor().len(),
        usize::from(small_config().fighters)
    );
    assert_eq!(app.presentation(), replay_reference(&app, 2));
}

#[test]
fn frame_reads_allocate_nothing_at_any_cursor() {
    // Calibration: the shared probe must observe a real allocation,
    // otherwise a zero reading would prove nothing.
    let (_, probe) = alloc_probe::measure(|| std::hint::black_box(vec![0u8; 64]));
    assert!(probe.calls >= 1, "alloc probe is blind");

    let mut app = App::new(small_config()).unwrap();
    for cursor in [16u32, 48u32] {
        while app.cursor() < cursor {
            let tick = app.cursor();
            assert!(app.tick(idle(tick)).input_accepted);
        }
        assert_eq!(app.presentation_len(), cursor);
        // The frame path (latest poses, reference tracks, length)
        // performs zero heap allocations at either cursor: flat cost.
        let ((), steady) = alloc_probe::measure(|| {
            std::hint::black_box(app.pose_cursor());
            std::hint::black_box(app.presentation_ref());
            std::hint::black_box(app.presentation_len());
        });
        assert_eq!(steady.calls, 0, "frame reads allocated at cursor {cursor}");
        assert!(!app.pose_cursor().is_empty());
    }
    // `pose` serves the same latest pose without the full tracks.
    assert_eq!(
        app.pose(PoseSource::LocoPresentation).unwrap(),
        app.pose_cursor().first().cloned()
    );
}

#[test]
fn window_frame_serves_cursor_and_empty_before_tick0() {
    // Failure case first: no frame before tick 0.
    let fresh = App::new(small_config()).unwrap();
    assert!(fresh.window_frame_poses().is_empty());
    let mut app = App::new(small_config()).unwrap();
    assert!(app.tick(idle(0)).input_accepted);
    // The window entry serves the cursor slice after every tick.
    assert_eq!(app.window_frame_poses(), app.pose_cursor());
    assert_eq!(
        app.window_frame_poses().len(),
        usize::from(small_config().fighters)
    );
    assert_eq!(
        app.pose(PoseSource::LocoPresentation).unwrap(),
        app.window_frame_poses().first().cloned()
    );
}

#[test]
fn layered_extract_env_beats_file_and_defaults() {
    let defaults = GameConfig::default();
    let file_rows = [
        ("seed", "7"),
        ("ticks", "64"),
        ("fighters", "4"),
        ("half_extent_m", "12.0"),
    ];
    let env_rows = [
        ("seed", "99"),
        ("ticks", "120"),
        ("fighters", "5"),
        ("half_extent_m", "30.0"),
    ];
    let merged = layered_extract(defaults, &file_rows, &env_rows);
    assert_eq!(merged.seed, 99);
    assert_eq!(merged.ticks, 120);
    assert_eq!(merged.fighters, 5);
    assert_eq!(merged.half_extent_m, 30.0);
    assert!(merged.validate().is_ok());
    assert!(App::new(merged).is_ok());
}

#[test]
fn layered_extract_file_beats_defaults() {
    let defaults = GameConfig::default();
    let file_rows = [
        ("seed", "7"),
        ("ticks", "64"),
        ("fighters", "4"),
        ("half_extent_m", "12.0"),
    ];
    let empty: [(&str, &str); 0] = [];
    let merged = layered_extract(defaults, &file_rows, &empty);
    assert_eq!(merged.seed, 7);
    assert_eq!(merged.ticks, 64);
    assert_eq!(merged.fighters, 4);
    assert_eq!(merged.half_extent_m, 12.0);
    assert!(merged.validate().is_ok());
}

#[test]
fn layered_extract_defaults_when_layers_empty() {
    let defaults = GameConfig::default();
    let empty: [(&str, &str); 0] = [];
    let merged = layered_extract(defaults, &empty, &empty);
    assert_eq!(merged, defaults);
    assert!(merged.validate().is_ok());
}

#[test]
fn layered_extract_bad_env_row_falls_back() {
    let defaults = GameConfig::default();
    let file_rows = [
        ("seed", "7"),
        ("ticks", "64"),
        ("fighters", "4"),
        ("half_extent_m", "12.0"),
    ];
    let env_rows = [
        ("seed", "99"),
        ("ticks", "not-a-number"),
        ("fighters", "99"),
        ("half_extent_m", "NaN"),
    ];
    let merged = layered_extract(defaults, &file_rows, &env_rows);
    assert_eq!(merged.seed, 99);
    assert_eq!(merged.ticks, 64);
    assert_eq!(merged.fighters, 4);
    assert_eq!(merged.half_extent_m, 12.0);
    assert!(merged.validate().is_ok());
    assert!(App::new(merged).is_ok());
}

#[test]
fn layered_extract_report_names_every_refused_row() {
    let defaults = GameConfig::default();
    let file_rows = [
        ("seed", "7"),
        ("ticks", "64"),
        ("fighters", "4"),
        ("half_extent_m", "12.0"),
    ];
    let env_rows = [
        ("seed", "not-a-number"),
        ("ticks", "not-a-number"),
        ("fighters", "99"),
        ("half_extent_m", "NaN"),
        ("mode", "arena"),
    ];
    let (merged, refused) = layered_extract_report(defaults, &file_rows, &env_rows);
    assert_eq!(merged.seed, 7);
    assert_eq!(merged.ticks, 64);
    assert_eq!(merged.fighters, 4);
    assert_eq!(merged.half_extent_m, 12.0);
    assert_eq!(
        refused,
        vec!["seed", "ticks", "fighters", "half_extent_m", "unknown"]
    );
    assert!(merged.validate().is_ok());
    assert!(App::new(merged).is_ok());
}

#[test]
fn layered_extract_report_empty_when_layers_clean() {
    let defaults = GameConfig::default();
    let file_rows = [
        ("seed", "7"),
        ("ticks", "64"),
        ("fighters", "4"),
        ("half_extent_m", "12.0"),
    ];
    let env_rows = [
        ("seed", "99"),
        ("ticks", "120"),
        ("fighters", "5"),
        ("half_extent_m", "30.0"),
    ];
    let (merged, refused) = layered_extract_report(defaults, &file_rows, &env_rows);
    assert_eq!(merged.seed, 99);
    assert_eq!(merged.ticks, 120);
    assert_eq!(merged.fighters, 5);
    assert_eq!(merged.half_extent_m, 30.0);
    assert!(refused.is_empty());
    assert!(merged.validate().is_ok());
    assert!(App::new(merged).is_ok());
}

#[test]
fn layered_extract_strict_schema_owns_every_row() {
    let schema = schemars::schema_for!(GameConfig);
    let root = schema.as_object().expect("GameConfig schema is an object");
    let properties = root
        .get("properties")
        .and_then(|properties| properties.as_object())
        .expect("GameConfig schema has properties");
    let mut keys: Vec<&str> = properties.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["fighters", "half_extent_m", "seed", "ticks"]);
    assert_eq!(
        root.get("additionalProperties")
            .and_then(|extra| extra.as_bool()),
        Some(false)
    );
}

#[test]
fn layered_extract_strict_takes_env_over_file_and_defaults() {
    let defaults = GameConfig::default();
    let file_rows = [
        ("seed", "7"),
        ("ticks", "64"),
        ("fighters", "4"),
        ("half_extent_m", "12.0"),
    ];
    let env_rows = [
        ("seed", "99"),
        ("ticks", "120"),
        ("fighters", "5"),
        ("half_extent_m", "30.0"),
    ];
    let merged = layered_extract_strict(defaults, &file_rows, &env_rows).unwrap();
    assert_eq!(
        merged,
        GameConfig {
            seed: 99,
            ticks: 120,
            fighters: 5,
            half_extent_m: 30.0,
        }
    );
    // Absent rows keep the defaults.
    let sparse = [("seed", "8")];
    let empty: [(&str, &str); 0] = [];
    let kept = layered_extract_strict(defaults, &sparse, &empty).unwrap();
    assert_eq!(
        kept,
        GameConfig {
            seed: 8,
            ..defaults
        }
    );
    // Strict refuses a bad file row even when env would hide it.
    assert_eq!(
        layered_extract_strict(defaults, &[("ticks", "0")], &[("ticks", "120")]),
        Err(GameError::BadConfig("ticks"))
    );
}

#[test]
fn layered_extract_strict_refuses_all_five_keys() {
    let defaults = GameConfig::default();
    let cases: [(&'static str, [(&str, &str); 1]); 5] = [
        ("seed", [("seed", "not-a-number")]),
        ("ticks", [("ticks", "600.0")]),
        ("fighters", [("fighters", "99")]),
        ("half_extent_m", [("half_extent_m", "NaN")]),
        ("unknown", [("mode", "arena")]),
    ];
    let empty: [(&str, &str); 0] = [];
    let mut refused = Vec::new();
    for (key, rows) in cases {
        let err = layered_extract_strict(defaults, &rows, &empty).expect_err(key);
        assert_eq!(err, GameError::BadConfig(key));
        refused.push(key);
    }
    assert_eq!(
        refused,
        ["seed", "ticks", "fighters", "half_extent_m", "unknown"]
    );
}

/// One oracle tick: checkpoint, cues, laterals and views.
type OracleTick = (
    Checkpoint,
    Vec<FightCue>,
    Vec<FighterLateral>,
    Vec<FighterView>,
);

/// The pre-EG1 `simulate_tick`, kept as the oracle: three whole-prefix
/// replays of `0..=tick` over the session's tape.
fn prefix_oracle(app: &App, tick: u32) -> OracleTick {
    let fighters = app.config.fighters;
    let (_, checkpoints) = run_prefix(&app.arena, &app.tape, tick + 1).expect("prefix");
    let short = ArenaConfig {
        ticks: tick + 1,
        ..app.arena
    };
    let cues = run_events(&short, &app.tape)
        .last()
        .map(|row| row.events.iter().map(map_cue).collect())
        .unwrap_or_default();
    let dataset = run_dataset(&short, &app.tape).expect("dataset replays");
    let base = usize::try_from(tick).unwrap() * usize::from(fighters);
    let rows = &dataset.rows[base..];
    (
        *checkpoints.last().expect("one checkpoint per tick"),
        cues,
        laterals_for_tick(rows, tick, fighters).expect("laterals"),
        views_for_tick(rows, tick, fighters).expect("views"),
    )
}

#[test]
fn live_match_equals_prefix_replay_oracle_600_ticks() {
    // EG1 parity: per-tick checkpoints, cues, laterals and views from the
    // incremental live match equal the old three prefix replays, at 2
    // and 8 fighters, with a LAN peer staging slot 1 every 7th tick.
    for fighters in [2u8, 8] {
        let config = GameConfig {
            fighters,
            ..config()
        };
        let mut app = App::new(config).unwrap();
        let mut fought = false;
        for tick in 0..config.ticks {
            if tick.is_multiple_of(7) {
                let peer = FighterInput {
                    fighter: FighterId::new(1).unwrap(),
                    mx: -50,
                    mz: 25,
                    ..idle(tick)
                };
                app.offer_remote(peer).unwrap();
            }
            let outcome = app.tick(script(tick));
            assert!(outcome.input_accepted, "tick {tick} at {fighters} fighters");
            let (checkpoint, cues, laterals, views) = prefix_oracle(&app, tick);
            assert_eq!(outcome.checkpoint, Some(checkpoint), "checkpoint {tick}");
            assert_eq!(outcome.cues, cues, "cues {tick}");
            assert_eq!(app.laterals, laterals, "laterals {tick}");
            assert_eq!(app.fighter_views(), views.as_slice(), "views {tick}");
            fought |= !cues.is_empty();
        }
        assert!(fought, "the {fighters}-fighter match lands cues");
        assert_eq!(app.sim_steps(), u64::from(config.ticks));
    }
}

#[test]
fn ticking_600_times_costs_exactly_600_sim_steps() {
    // EG1 cost proof without a wall clock: one arena step per accepted
    // tick (the old prefix replays cost 3 * (tick + 1) steps per tick,
    // 540,900 over 600 ticks). Rejected ticks step nothing.
    let mut app = App::new(config()).unwrap();
    assert_eq!(app.sim_steps(), 0);
    for tick in 0..TICKS {
        assert!(app.tick(script(tick)).input_accepted);
        assert_eq!(app.sim_steps(), u64::from(tick) + 1, "tick {tick}");
    }
    assert!(!app.tick(script(TICKS)).input_accepted, "match over");
    assert_eq!(app.sim_steps(), u64::from(TICKS));
}

#[test]
fn stale_live_match_rebuilds_from_tape_byte_identically() {
    // The rare path: a live match out of step with the cursor rebuilds
    // from tick 0 over the tape, then continues identically.
    let config = small_config();
    let (reference, reference_out) = run_session(config);
    let mut app = App::new(config).unwrap();
    let mut outs = Vec::new();
    for tick in 0..config.ticks {
        if tick == 20 {
            app.live = LiveMatch::with_rules(&app.arena, app.rules).unwrap();
        }
        outs.push(app.tick(script(tick)));
    }
    assert_eq!(outs, reference_out);
    assert_eq!(app.checkpoints(), reference.checkpoints());
    assert_eq!(app.fighter_views(), reference.fighter_views());
    // The hand swap above dropped the first 20 steps unbilled; the
    // rebuild then replays 20 ticks and steps the 44 left.
    assert_eq!(app.sim_steps(), u64::from(config.ticks));
}

/// Presentation sim steps the session paid (EG1b cost proof).
fn presentation_steps(app: &App) -> u64 {
    app.present_retired_steps
        .saturating_add(app.present_sim.as_ref().map_or(0, |sim| sim.steps()))
}

#[test]
fn live_presentation_equals_prefix_replay_oracle_600_ticks() {
    // EG1b parity: with the cursor on (the default), the cached tracks,
    // root tables and latest poses equal the old two whole-prefix legs
    // after every tick, at 2 and 8 fighters, LAN peer on slot 1.
    for fighters in [2u8, 8] {
        let config = GameConfig {
            fighters,
            ..config()
        };
        let mut app = App::new(config).unwrap();
        for tick in 0..config.ticks {
            if tick.is_multiple_of(7) {
                let peer = FighterInput {
                    fighter: FighterId::new(1).unwrap(),
                    mx: -50,
                    mz: 25,
                    ..idle(tick)
                };
                app.offer_remote(peer).unwrap();
            }
            assert!(app.tick(script(tick)).input_accepted);
            let expected = replay_reference(&app, tick + 1);
            assert_eq!(expected.pose_tracks.len(), usize::from(fighters));
            assert_eq!(app.presentation_ref(), &expected, "tick {tick}");
            let tails: Vec<LocalPose> = expected
                .pose_tracks
                .iter()
                .map(|track| track.last().cloned().expect("covered tick"))
                .collect();
            assert_eq!(app.pose_cursor(), tails.as_slice(), "tails {tick}");
        }
        assert_eq!(presentation_steps(&app), u64::from(config.ticks));
    }
}

#[test]
fn presentation_costs_one_step_per_tick_and_rebuilds_only_when_turned_back_on() {
    // EG1b cost proof without a wall clock: one presentation step per
    // accepted tick (the old cursor replayed the prefix twice per tick,
    // 2 * (tick + 1) record steps). Turning the cursor off and back on
    // is the one replay path: it rebuilds once, then steps singly again.
    let mut app = App::new(config()).unwrap();
    for tick in 0..TICKS / 2 {
        assert!(app.tick(script(tick)).input_accepted);
        assert_eq!(presentation_steps(&app), u64::from(tick) + 1, "tick {tick}");
    }
    app.set_presentation_cursor(false);
    assert!(app.tick(script(TICKS / 2)).input_accepted);
    assert_eq!(presentation_steps(&app), u64::from(TICKS / 2));
    app.set_presentation_cursor(true);
    let before = presentation_steps(&app);
    assert!(app.tick(script(TICKS / 2 + 1)).input_accepted);
    // Rebuild replays TICKS / 2 + 1 ticks, then steps the new one.
    assert_eq!(presentation_steps(&app), before + u64::from(TICKS / 2 + 2));
    for tick in TICKS / 2 + 2..TICKS {
        let paid = presentation_steps(&app);
        assert!(app.tick(script(tick)).input_accepted);
        assert_eq!(presentation_steps(&app), paid + 1, "tick {tick}");
    }
    assert_eq!(app.presentation_ref(), &replay_reference(&app, TICKS));
    assert_eq!(app.sim_steps(), u64::from(TICKS));
}

#[test]
fn presentation_frame_cost_flat_to_18k_bench() {
    // Rolling-cursor accept bench: frame-read cost at tick 18,000 within
    // 2 times the cost at tick 10. Frame reads are slice borrows, so this seeds
    // a true 18,000-tick cache from one real pose and measures the
    // reads directly. Ticking 18k times through `App::tick` stays out:
    // each extension still replays the prefix (the arena keeps no
    // single-tick pose step), so that loop is O(n^2) by construction.
    use engine_anim::contract::RootDelta;
    const BENCH_TICKS: u32 = 18_000;
    const NEAR_TICKS: u32 = 10;
    const READS: u32 = 100_000;
    let (_, probe) = alloc_probe::measure(|| std::hint::black_box(vec![0u8; 64]));
    assert!(probe.calls >= 1, "alloc probe is blind");

    let mut near = App::new(small_config()).unwrap();
    while near.cursor() < NEAR_TICKS {
        let tick = near.cursor();
        assert!(near.tick(idle(tick)).input_accepted);
    }
    assert_eq!(near.presentation_len(), NEAR_TICKS);

    let mut far = App::new(small_config()).unwrap();
    while far.cursor() < NEAR_TICKS {
        let tick = far.cursor();
        assert!(far.tick(idle(tick)).input_accepted);
    }
    let seed_pose = far
        .pose_cursor()
        .first()
        .cloned()
        .expect("covered tick has a pose");
    let seed_table = far
        .presentation_ref()
        .root_tables
        .first()
        .cloned()
        .unwrap_or_default();
    let seed_delta = seed_table.deltas.last().copied().unwrap_or(RootDelta {
        tick: 0,
        dp: [0.0, 0.0],
        dyaw: 0.0,
    });
    let fighters = usize::from(small_config().fighters);
    let long = usize::try_from(BENCH_TICKS).unwrap_or(0);
    let mut tracks = Vec::with_capacity(fighters);
    let mut tables = Vec::with_capacity(fighters);
    let mut tails = Vec::with_capacity(fighters);
    for _ in 0..fighters {
        tracks.push(vec![seed_pose.clone(); long]);
        let mut table = seed_table.clone();
        table.deltas.clear();
        table.deltas.extend((0..BENCH_TICKS).map(|tick| RootDelta {
            tick,
            dp: seed_delta.dp,
            dyaw: seed_delta.dyaw,
        }));
        tables.push(table);
        tails.push(seed_pose.clone());
    }
    far.present = PresentationData {
        pose_tracks: tracks,
        root_tables: tables,
    };
    far.present_latest = tails;
    far.present_ticks = BENCH_TICKS;
    assert_eq!(far.presentation_len(), BENCH_TICKS);
    assert_eq!(far.window_frame_poses(), far.pose_cursor());

    let ((), near_probe) = alloc_probe::measure(|| {
        for _ in 0..READS {
            std::hint::black_box(near.pose_cursor());
            std::hint::black_box(near.presentation_ref());
            std::hint::black_box(near.window_frame_poses());
            std::hint::black_box(near.presentation_len());
        }
    });
    let ((), far_probe) = alloc_probe::measure(|| {
        for _ in 0..READS {
            std::hint::black_box(far.pose_cursor());
            std::hint::black_box(far.presentation_ref());
            std::hint::black_box(far.window_frame_poses());
            std::hint::black_box(far.presentation_len());
        }
    });
    assert_eq!(near_probe.calls, 0, "frame reads allocated at tick 10");
    assert_eq!(far_probe.calls, 0, "frame reads allocated at tick 18000");

    let start = std::time::Instant::now();
    for _ in 0..READS {
        std::hint::black_box(near.pose_cursor());
        std::hint::black_box(near.presentation_ref());
        std::hint::black_box(near.window_frame_poses());
        std::hint::black_box(near.presentation_len());
    }
    let near_elapsed = start.elapsed();
    let start = std::time::Instant::now();
    for _ in 0..READS {
        std::hint::black_box(far.pose_cursor());
        std::hint::black_box(far.presentation_ref());
        std::hint::black_box(far.window_frame_poses());
        std::hint::black_box(far.presentation_len());
    }
    let far_elapsed = start.elapsed();
    assert!(
        far_elapsed <= near_elapsed * 2 + std::time::Duration::from_millis(50),
        "18k frame reads took {far_elapsed:?} vs 10-tick {near_elapsed:?}"
    );
}

#[test]
fn crowd_levels_export_64_in_range_and_repeat() {
    // Behavior: the staged helper copies the room's 64 levels through the
    // new `crowd_levels` caller; levels sit in 300..=1000 and repeat.
    let mut app = App::new(small_config()).unwrap();
    for tick in 0..3u32 {
        assert!(app.tick(idle(tick)).input_accepted);
    }
    let mut first = [0u16; crate::crowd_gpu::CROWD_BLOCKS];
    let n = app.crowd_levels(&mut first).expect("64 fit");
    assert_eq!(n, crate::crowd_gpu::CROWD_BLOCKS);
    for level in first.iter() {
        assert!(
            (*level >= crate::crowd_gpu::BASELINE_PERMILLE)
                && (*level <= crate::crowd_gpu::PEAK_PERMILLE),
            "level {level} outside murmur..peak"
        );
    }
    let mut again = [0u16; crate::crowd_gpu::CROWD_BLOCKS];
    let m = app.crowd_levels(&mut again).expect("64 fit twice");
    assert_eq!(m, n);
    assert_eq!(first, again, "same room exports the same levels");
}

#[test]
fn crowd_levels_short_buffer_refuses_loudly() {
    // Failure: a short caller buffer refuses with the want/got sizes,
    // never a truncation; the staged run maps this into `crowd: ...`.
    let app = App::new(small_config()).unwrap();
    let mut short = [0u16; 7];
    match app.crowd_levels(&mut short) {
        Err(crate::crowd_gpu::CrowdGpuError::BadBuffer { want, got }) => {
            assert_eq!(want, crate::crowd_gpu::CROWD_BLOCKS);
            assert_eq!(got, 7);
        }
        other => panic!("short crowd buffer must refuse, got {other:?}"),
    }
}

#[test]
fn out_of_order_stage_keeps_tape_sorted() {
    // Tape-sort repair: out-of-order LAN arrivals insert sorted, so the
    // tape stays `(tick, fighter)`-sorted and validates.
    let mut app = App::new(small_config()).unwrap();
    let at = |tick: u32, fighter: u8| FighterInput {
        tick,
        fighter: FighterId::new(fighter).unwrap(),
        mx: 0,
        mz: 0,
        facing: 0,
        buttons: Buttons::NONE,
    };
    app.stage_input(at(2, 1));
    app.stage_input(at(0, 0));
    app.stage_input(at(0, 1));
    app.stage_input(at(2, 0));
    app.stage_input(at(1, 0));
    let keys: Vec<(u32, u8)> = app
        .tape
        .0
        .iter()
        .map(|input| (input.tick, input.fighter.0))
        .collect();
    assert_eq!(
        keys,
        vec![(0, 0), (0, 1), (1, 0), (2, 0), (2, 1)],
        "tape must stay sorted"
    );
    assert!(app.tape.validate().is_ok());
    assert!(tape_should_append(&app.tape, (2, 1)));
    assert!(tape_should_append(&app.tape, (3, 0)));
    assert!(!tape_should_append(&app.tape, (1, 0)));
}

#[test]
fn without_presentation_extras_keeps_the_match_bytes() {
    // The lean session (browser page path) drops only presentation work:
    // the authority checkpoint, cues, hp and views must equal the
    // default session tick for tick, while the extras stay cold.
    let mut full = App::new(small_config()).unwrap();
    let mut lean = App::new(small_config())
        .unwrap()
        .without_presentation_extras();
    assert!(full.presentation_extras());
    assert!(!lean.presentation_extras());
    let ticks = small_config().ticks;
    for tick in 0..ticks {
        let a = full.tick(script(tick));
        let b = lean.tick(script(tick));
        assert_eq!(a.checkpoint, b.checkpoint, "checkpoint at tick {tick}");
        assert_eq!(a.cues, b.cues, "cues at tick {tick}");
        assert_eq!(a.audio, b.audio, "audio at tick {tick}");
        assert_eq!(a.reactions, b.reactions, "crowd reactions at tick {tick}");
        assert_eq!(a.bed_gain.to_bits(), b.bed_gain.to_bits());
    }
    assert_eq!(full.hp(), lean.hp());
    assert_eq!(full.fighter_views(), lean.fighter_views());
    assert_eq!(full.checkpoints().len(), ticks as usize);
    assert!(lean.checkpoints().is_empty(), "lean keeps no tick log");
    assert_eq!(lean.presentation_len(), 0, "lean keeps no pose cursor");
    assert!(lean.pose_cursor().is_empty());
    assert_eq!(full.crowd_gpu().tick_count(), u64::from(ticks));
    assert_eq!(lean.crowd_gpu().tick_count(), 0, "visual crowd stays cold");
    assert_eq!(lean.director_updates(), 0, "director stays cold");
    assert!(lean.director_cuts().is_empty(), "cut camera stays cold");
    // Still one authority sim step per accepted tick either way.
    assert_eq!(full.sim_steps(), u64::from(ticks));
    assert_eq!(lean.sim_steps(), u64::from(ticks));
}

/// Wall-clock phase table for the page's tick path: per-phase us for one
/// accepted tick, then whole-`App::tick` cost with the cursor on, off and
/// for a lean session, at 2 and 8 fighters. Diagnostic only (never a
/// gate): run `cargo test -p engine-game --lib tick_phase_table_native
/// -- --ignored --nocapture`.
#[test]
#[ignore = "wall-clock phase table; run with --ignored --nocapture"]
fn tick_phase_table_native() {
    use engine_clock::now;
    use std::time::Duration;

    const WARM: u32 = 200;
    const MEASURED: u32 = 2_000;
    let cfg = GameConfig {
        seed: 2040,
        ticks: 20_000,
        fighters: 2,
        half_extent_m: 12.0,
    };
    let mut app = App::with_world(cfg, crate::arena_kit::kit_match_world_or_empty())
        .unwrap()
        .into_duel()
        .unwrap();
    let mut out = TickOutcome::default();
    let mut phase = [Duration::ZERO; 6];
    let started_all = now();
    for tick in 0..(WARM + MEASURED) {
        let mut stamped = script(tick);
        stamped.tick = app.cursor;
        let record = tick >= WARM;
        let t = now();
        assert!(app.cursor < app.config.ticks && stamped.validate().is_ok());
        app.stage_input(stamped);
        if record {
            phase[0] += now() - t;
        }
        let t = now();
        let (checkpoint, cues, started) = app.simulate_tick(tick).unwrap();
        if record {
            phase[1] += now() - t;
        }
        let t = now();
        app.checkpoints.push(checkpoint);
        app.cursor += 1;
        if record {
            phase[2] += now() - t;
        }
        let t = now();
        app.observe_into(tick, &cues, &started, checkpoint, &mut out);
        if record {
            phase[3] += now() - t;
        }
        let t = now();
        app.extend_presentation();
        if record {
            phase[4] += now() - t;
        }
        let t = now();
        app.restore_tick_scratch(cues, started);
        if record {
            phase[5] += now() - t;
        }
    }
    let total = now() - started_all;
    let per = |d: Duration| d.as_secs_f64() * 1_000.0 / f64::from(MEASURED);
    println!(
        "phase us/tick: stage={:.1} sim={:.1} log={:.1} observe={:.1} present={:.1} restore={:.1} sum={:.1} wall={:.1}",
        per(phase[0]) * 1000.0,
        per(phase[1]) * 1000.0,
        per(phase[2]) * 1000.0,
        per(phase[3]) * 1000.0,
        per(phase[4]) * 1000.0,
        per(phase[5]) * 1000.0,
        phase.iter().map(|d| per(*d)).sum::<f64>() * 1000.0,
        per(total) * 1000.0,
    );

    // Whole owned `App::tick` path (the page's call) with the cursor on
    // and off, plus 8 fighters (the window's seat count).
    let whole = |fighters: u8, cursor: bool| {
        let cfg = GameConfig {
            seed: 2040,
            ticks: 20_000,
            fighters,
            half_extent_m: 12.0,
        };
        let mut app = App::with_world(cfg, crate::arena_kit::kit_match_world_or_empty())
            .unwrap()
            .into_duel()
            .unwrap();
        if !cursor {
            app.set_presentation_cursor(false);
        }
        for tick in 0..WARM {
            let _ = app.tick(script(tick));
        }
        let t = now();
        for tick in WARM..(WARM + MEASURED) {
            let _ = app.tick(script(tick));
        }
        let d = now() - t;
        (per(d) * 1000.0, app.sim_steps(), app.presentation_len())
    };
    for (fighters, cursor) in [(2u8, true), (2, false), (8, true), (8, false)] {
        let (us, steps, presented) = whole(fighters, cursor);
        println!(
            "tick() fighters={fighters} cursor={cursor}: {us:.2} us/tick (sim_steps={steps} present_ticks={presented})"
        );
    }
    let lean = |fighters: u8| {
        let cfg = GameConfig {
            seed: 2040,
            ticks: 20_000,
            fighters,
            half_extent_m: 12.0,
        };
        let mut app = App::with_world(cfg, crate::arena_kit::kit_match_world_or_empty())
            .unwrap()
            .into_duel()
            .unwrap()
            .without_presentation_extras();
        for tick in 0..WARM {
            let _ = app.tick(script(tick));
        }
        let t = now();
        for tick in WARM..(WARM + MEASURED) {
            let _ = app.tick(script(tick));
        }
        (per(now() - t) * 1000.0, app.sim_steps())
    };
    for fighters in [2u8, 8] {
        let (us, steps) = lean(fighters);
        println!("tick() lean fighters={fighters}: {us:.2} us/tick (sim_steps={steps})");
    }
}
