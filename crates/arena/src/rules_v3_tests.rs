//! SW27: rules v3 is the append-only successor of v2. At this step it plays
//! exactly like v2: the same tape under both versions gives the same
//! checkpoint hash on every tick, with and without the armour, heft and spear
//! channels armed.

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

/// Slot 0 presses attack every 25 ticks; every other slot is left to its
/// brain, so the v2 plan, brain wish and seeded personality gates all run.
fn press_tape(config: &ArenaConfig) -> FighterTape {
    let mut inputs = Vec::new();
    for tick in 0..config.ticks {
        if tick % 25 == 0 {
            inputs.push(FighterInput {
                tick,
                fighter: FighterId::new(0).expect("slot 0 is legal"),
                mx: 100,
                mz: 0,
                facing: 0,
                buttons: Buttons(ATTACK),
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
