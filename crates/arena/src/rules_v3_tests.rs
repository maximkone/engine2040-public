//! SW27: rules v3 is the append-only successor of v2. On a tap-only tape it
//! plays exactly like v2: the same tape under both versions gives the same
//! checkpoint hash on every tick, with and without the armour, heft and spear
//! channels armed. The one deliberate v3 difference is the press edge (SW26),
//! which this tape never reaches: every ATTACK is down for one tick only and
//! no brain drives a slot, so nothing is held across ticks.

use crate::{
    geometry_rules, ArenaConfig, ArenaRules, FighterInput, FighterTape, Sim, FIGHT_RULES_V2,
    FIGHT_RULES_V3,
};
use engine_combat::tables::TABLES_VERSION_V2;
use engine_input::fighter::ATTACK;
use engine_input::{Buttons, FighterId};

/// A rules-v2 bout config: v2 tables, so `ArenaConfig::rules` pins v2 at build.
fn config(fighters: u8) -> ArenaConfig {
    ArenaConfig {
        seed: 2040,
        ticks: 600,
        fighters,
        half_extent_m: 20.0,
        table_version: TABLES_VERSION_V2,
    }
}

/// Every slot on a tap-only tape: ATTACK on every 25th tick and released the
/// tick after, all slots tape-driven, so no brain can hold a press and rules
/// v3's press edge (SW26) is never reached.
fn press_tape(config: &ArenaConfig) -> FighterTape {
    let mut inputs = Vec::new();
    for tick in 0..config.ticks {
        let buttons = if tick % 25 == 0 {
            Buttons(ATTACK)
        } else {
            Buttons::NONE
        };
        for fighter in 0..config.fighters {
            inputs.push(FighterInput {
                tick,
                fighter: FighterId::new(fighter).expect("slots 0..8 are legal"),
                mx: 100,
                mz: 0,
                facing: 0,
                buttons,
            });
        }
    }
    FighterTape(inputs)
}

/// One 600-tick run under `version`: pin the rules before tick 0, arm the
/// extras, then collect every tick's checkpoint hash.
fn run(config: &ArenaConfig, version: u32, extra_channels: bool) -> Vec<u64> {
    let tape = press_tape(config);
    let mut sim = Sim::build(config, &tape, false, false).expect("the bout builds");
    let mut rules = ArenaRules::v2_free_for_all();
    rules.version = version;
    assert!(sim.set_rules(rules), "the rules pin before tick 0");
    assert_eq!(sim.rules.version, version);
    if extra_channels {
        sim.wear_armour(&[&["lorica", "helmet"], &["manica"]])
            .expect("the kit wears under a geometry rule set");
        sim.arm_weapons(&["gladius", "pugio"])
            .expect("the shipped rows arm under a geometry rule set");
        assert!(sim.set_spear(), "the spear arms before tick 0");
    }
    let mut hashes = Vec::with_capacity(config.ticks as usize);
    while sim.tick < config.ticks {
        sim.step(&tape).expect("the bout steps");
        hashes.push(
            sim.checkpoints
                .last()
                .expect("every tick folds one checkpoint")
                .hash,
        );
    }
    hashes
}

/// The same tape under v2 and v3: equal hash on every tick, at 2 and at 8
/// fighters, plus an armour/heft/spear bout.
#[test]
fn rules_v3_plays_exactly_like_v2() {
    for fighters in [2, 8] {
        let config = config(fighters);
        let v2 = run(&config, FIGHT_RULES_V2, false);
        let v3 = run(&config, FIGHT_RULES_V3, false);
        assert_eq!(v2.len(), 600, "one hash per tick");
        assert_eq!(v2, v3, "v3 must hash exactly like v2, {fighters} fighters");
        assert!(
            v2.windows(2).any(|pair| pair[0] != pair[1]),
            "the bout must actually move"
        );
    }
    let config = config(2);
    let v2 = run(&config, FIGHT_RULES_V2, true);
    let v3 = run(&config, FIGHT_RULES_V3, true);
    assert_eq!(
        v2, v3,
        "v3 must hash exactly like v2 with the channels armed"
    );
}

/// Version 3 is known; 4 is refused as unknown and keeps the current rules.
#[test]
fn rules_v3_is_known_and_v4_is_refused() {
    let v3 = ArenaRules {
        version: FIGHT_RULES_V3,
        ..ArenaRules::v2_free_for_all()
    };
    assert!(v3.is_known(), "v3 is a known rule set");
    assert!(geometry_rules(FIGHT_RULES_V3), "v3 takes every v2 gate");
    assert!(!geometry_rules(crate::FIGHT_RULES_V1), "v1 takes none");

    let config = config(2);
    let tape = press_tape(&config);
    let mut sim = Sim::build(&config, &tape, false, false).expect("the bout builds");
    assert!(sim.set_rules(v3), "v3 pins before tick 0");
    let v4 = ArenaRules {
        version: FIGHT_RULES_V3 + 1,
        ..v3
    };
    assert!(!v4.is_known(), "v4 is unknown");
    assert!(!sim.set_rules(v4), "an unknown version refuses");
    assert_eq!(sim.rules.version, FIGHT_RULES_V3, "the refusal keeps v3");
}

/// A gladius duel tape: slot 0 taps ATTACK on tick 0 and on each tick of
/// `extra`; every slot carries a row every tick, so no brain decides a slot.
fn duel_tape(config: &ArenaConfig, extra: &[u32]) -> FighterTape {
    let mut inputs = Vec::new();
    for tick in 0..config.ticks {
        for fighter in 0..config.fighters {
            let down = fighter == 0 && (tick == 0 || extra.contains(&tick));
            inputs.push(FighterInput {
                tick,
                fighter: FighterId::new(fighter).expect("slots 0..8 are legal"),
                mx: 0,
                mz: 0,
                facing: 0,
                buttons: if down { Buttons(ATTACK) } else { Buttons::NONE },
            });
        }
    }
    FighterTape(inputs)
}

/// The gladius duel with `version` pinned before tick 0: slot 0 at the origin
/// facing +z, slot 1 one metre ahead, both armed with the kit gladius.
fn duel_sim(config: &ArenaConfig, tape: &FighterTape, version: u32) -> Sim {
    let mut sim = Sim::build(config, tape, false, false).expect("the duel builds");
    let mut rules = ArenaRules::v2_free_for_all();
    rules.version = version;
    assert!(sim.set_rules(rules), "the rules pin before tick 0");
    sim.states[0].pos = [0.0, 0.0, 0.0];
    sim.states[1].pos = [0.0, 0.0, 1.0];
    sim.arm_weapons(&["gladius", "gladius"])
        .expect("the kit gladii arm under a geometry rule set");
    sim
}

/// Every tick's checkpoint hash for one full duel.
fn duel_hashes(config: &ArenaConfig, tape: &FighterTape, version: u32) -> Vec<u64> {
    let mut sim = duel_sim(config, tape, version);
    while sim.tick < config.ticks {
        sim.step(tape).expect("the duel steps");
    }
    sim.checkpoints.iter().map(|point| point.hash).collect()
}

/// SW20's fight tape, pinned at the arena checkpoint level: a gladius duel
/// where slot 0 taps on the first tick the hit-stop freezes. Rules v3 keeps
/// that press and swings again after the freeze; rules v2 drops it. This is a
/// new v3 pin: the v1 golden and every v2 tape pin stay untouched.
#[test]
fn rules_v3_fight_tape_hash_is_pinned() {
    let config = ArenaConfig {
        ticks: 80,
        ..config(2)
    };
    let plain = duel_tape(&config, &[]);
    let mut probe = duel_sim(&config, &plain, FIGHT_RULES_V3);
    let mut frozen = None;
    while probe.tick < config.ticks {
        probe.step(&plain).expect("the probe steps");
        if probe.fight.hitstop > 0 {
            frozen = Some(probe.tick);
            break;
        }
    }
    let frozen = frozen.expect("the gladius lands and freezes the clock");
    assert_eq!(frozen, 31, "the gladius lands on tick 30 and freezes 31");
    let tape = duel_tape(&config, &[frozen]);
    let v3 = duel_hashes(&config, &tape, FIGHT_RULES_V3);
    let v2 = duel_hashes(&config, &tape, FIGHT_RULES_V2);
    let (last_v3, last_v2) = (
        *v3.last().expect("one hash per tick"),
        *v2.last().expect("one hash per tick"),
    );
    println!("sw20 fight tape: frozen={frozen} v3={last_v3:016x} v2={last_v2:016x}");
    assert_eq!(
        last_v3, 0x546d_c84e_9b60_8a4a,
        "the v3 fight tape checkpoint is pinned: SW20"
    );
    assert_ne!(v2, v3, "v2 drops the frozen press, so its tape differs");
}
