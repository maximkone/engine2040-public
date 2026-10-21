//! Rules bars: the shield bash, the refusals, determinism, old tapes, the budget.

use crate::fixtures::{fighter, ring_world, smoke_worlds};
use crate::host::{Action, World, BTN_ATTACK, BTN_BLOCK};
use crate::sim::{scripted_inputs, Sim, Wall};
use crate::tape::{record_scripted, replay, ReplayError, RulesTape, ScriptLibrary, TapeError};
use crate::{rules_abi, script_hash, HostFault, LimitKind, Limits, Rules, RulesError};

const SHIELD_BASH: &str = include_str!("../scripts/shield_bash.rhai");

/// A duel where fighter 0 blocks and attacks with fighter 1 in reach.
fn bash_world(tick: u32) -> World {
    let mut world = World {
        tick,
        seed: 1,
        fighters: vec![fighter(0, 0, 0, 0), fighter(1, 1, 900, 0)],
    };
    world.fighters[0].buttons = BTN_ATTACK | BTN_BLOCK;
    world.fighters[0].blocking = true;
    world.fighters[0].attacking = true;
    world
}

fn installed() -> Rules {
    let mut rules = Rules::default();
    rules.set(SHIELD_BASH, 0).expect("shield bash installs");
    rules
}

// ---------------------------------------------------------------- the shield bash

#[test]
fn shield_bash_installs_and_acts() {
    let mut rules = Rules::default();
    let receipt = rules.set(SHIELD_BASH, 0).expect("installs");
    assert_eq!(receipt.rev, 1);
    assert_eq!(receipt.hash, script_hash(SHIELD_BASH));
    assert!(
        receipt.max_ops > 0 && receipt.max_ops < 200,
        "ops {}",
        receipt.max_ops
    );
    assert!(SHIELD_BASH.lines().count() >= 30);

    let report = rules.tick(&bash_world(100)).expect("tick");
    assert!(report.faults.is_empty(), "{:?}", report.faults);
    let actions: Vec<Action> = report.actions.iter().map(|e| e.action).collect();
    assert_eq!(
        actions,
        vec![
            Action::Push {
                target: 1,
                dx_mm: 600,
                dz_mm: 0
            },
            Action::Stun {
                target: 1,
                ticks: 18
            },
            Action::Damage {
                target: 1,
                permille: 40
            },
            Action::DrainStamina {
                target: 0,
                amount: 250
            },
            Action::Shield {
                target: 0,
                ticks: 30
            },
        ]
    );
    assert!(report.actions.iter().all(|e| e.actor == 0));
}

#[test]
fn shield_bash_cooldown_uses_memory() {
    let mut rules = installed();
    assert_eq!(rules.tick(&bash_world(100)).unwrap().actions.len(), 5);
    // 50 ticks later the memory slot still says "bashed at 100".
    assert!(rules.tick(&bash_world(150)).unwrap().actions.is_empty());
    assert_eq!(rules.tick(&bash_world(191)).unwrap().actions.len(), 5);
}

#[test]
fn shield_bash_needs_reach_stamina_and_block() {
    let mut rules = installed();
    let mut far = bash_world(10);
    far.fighters[1].x_mm = 1_300;
    assert!(rules.tick(&far).unwrap().actions.is_empty());
    let mut tired = bash_world(10);
    tired.fighters[0].stamina = 29;
    assert!(rules.tick(&tired).unwrap().actions.is_empty());
    let mut no_block = bash_world(10);
    no_block.fighters[0].blocking = false;
    assert!(rules.tick(&no_block).unwrap().actions.is_empty());
}

// ---------------------------------------------------------------- bad edits, last good kept

fn refused(source: &str) -> RulesError {
    let mut rules = installed();
    let before = rules.script_hash();
    let history = rules.history().len();
    let err = rules.set(source, 5).expect_err("bad edit must be refused");
    assert_eq!(rules.script_hash(), before, "last good kept");
    assert_eq!(rules.history().len(), history);
    assert_eq!(rules.refusals(), 1);
    assert_eq!(rules.last_refusal(), Some(&err));
    // The last good script still plays the bash.
    assert_eq!(rules.tick(&bash_world(100)).unwrap().actions.len(), 5);
    err
}

#[test]
fn float_op_refused() {
    let err = refused("let r = 1;\nlet f = 1.5;\ndamage(1, 10);\n");
    assert_eq!(err, RulesError::Float { line: 2 });
    assert!(matches!(refused("let r = 2e3;"), RulesError::Float { .. }));
    // `..` ranges next to integers are not floats.
    let mut rules = Rules::default();
    rules
        .set("for i in 0..3 { mem_set(0, i); }", 0)
        .expect("range is fine");
}

#[test]
fn unbounded_loop_refused() {
    let err = refused("loop {\n    mem_set(0, 1);\n}\n");
    assert_eq!(err, RulesError::UnboundedLoop { line: 1 });
    // A loop that never ends without a literal `true` runs out of budget in the smoke run.
    let err = refused("let n = 0; while n >= 0 { n += 1; }");
    assert_eq!(
        err,
        RulesError::Limit {
            kind: LimitKind::Operations
        }
    );
    let err = refused("for i in 0..100000000 { mem_set(0, i); }");
    assert_eq!(
        err,
        RulesError::Limit {
            kind: LimitKind::Operations
        }
    );
}

#[test]
fn unknown_host_call_refused() {
    let err = refused("\nlaunch_missiles(1);\n");
    assert_eq!(
        err,
        RulesError::UnknownHostCall {
            name: "launch_missiles".to_string(),
            arity: 1,
            line: 2
        }
    );
    // Right name, wrong argument count.
    assert!(matches!(
        refused("damage(1);"),
        RulesError::UnknownHostCall { arity: 1, .. }
    ));
    // Even in a branch the smoke worlds never take.
    let err = refused("if hp == 12345 { summon(1, 2); }");
    assert!(matches!(err, RulesError::UnknownHostCall { ref name, .. } if name == "summon"));
    // The old function-style entry point is just an unknown call now.
    assert!(matches!(
        refused("on_tick(1);"),
        RulesError::UnknownHostCall { .. }
    ));
}

#[test]
fn deny_all_registry_blocks_the_standard_library() {
    for call in [
        "print(1)",
        "debug(1)",
        "type_of(1)",
        "to_float(1)",
        "sleep(5)",
        "timestamp()",
        "eval(1)",
    ] {
        let src = format!("{call};");
        let err = refused(&src);
        assert!(
            matches!(
                err,
                RulesError::UnknownHostCall { .. } | RulesError::Syntax { .. }
            ),
            "{call}: {err:?}"
        );
    }
    for src in [
        "let s = \"hi\";",
        "let a = [1, 2];",
        "let m = #{a: 1};",
        "hp.abs();",
        "let f = Fn(\"damage\");",
        "import \"x\" as y;",
        "let q = hp.max;",
        "hp = 5;",
    ] {
        let err = refused(src);
        assert!(
            matches!(
                err,
                RulesError::Forbidden { .. }
                    | RulesError::Syntax { .. }
                    | RulesError::Runtime { .. }
                    | RulesError::UnknownHostCall { .. }
            ),
            "{src}: {err:?}"
        );
    }
}

#[test]
fn other_bad_edits_have_typed_errors() {
    assert!(matches!(refused("let x = ;"), RulesError::Syntax { .. }));
    assert!(matches!(
        refused("mem_set(0, nope);"),
        RulesError::UnknownVariable { .. }
    ));
    // A helper function cannot see the actor's variables.
    assert!(matches!(
        refused("fn peek() { hp }\nmem_set(0, peek());"),
        RulesError::UnknownVariable { .. } | RulesError::Runtime { .. }
    ));
    assert!(matches!(
        refused("damage(7, 10);"),
        RulesError::Host(HostFault::BadTarget(7))
    ));
    assert!(matches!(
        refused("damage(1, 5000);"),
        RulesError::Host(HostFault::BadArg { call: "damage", .. })
    ));
    assert!(matches!(
        refused("mem_set(8, 1);"),
        RulesError::Host(HostFault::MemSlot(8))
    ));
    assert!(matches!(
        refused("let z = 0; mem_set(0, 5 / z);"),
        RulesError::Arithmetic { .. }
    ));
    assert!(matches!(
        refused("let a = 9223372036854775807; mem_set(0, a + 1);"),
        RulesError::Arithmetic { .. }
    ));
    assert!(matches!(
        refused("throw 3;"),
        RulesError::Thrown { value: 3 }
    ));
    let nine_emits = "stun(1, 1); ".repeat(9);
    assert_eq!(refused(&nine_emits), RulesError::Host(HostFault::ActionCap));
    let deep = "f(0);\nfn f(n) { f(n + 1); }";
    assert_eq!(
        refused(deep),
        RulesError::Limit {
            kind: LimitKind::CallDepth
        }
    );
    let huge = format!("// {}", "x".repeat(20_000));
    assert!(matches!(refused(&huge), RulesError::TooLarge { .. }));
}

#[test]
fn test_reports_without_installing() {
    let rules = installed();
    let hash = rules.script_hash();
    let good = rules.test("stun(1, 3);", &smoke_worlds());
    assert!(good.ok());
    assert_eq!(good.cases.len(), smoke_worlds().len());
    assert!(good.cases.iter().all(|c| c.actions > 0));
    let bad = rules.test("damage(9, 1);", &[bash_world(1)]);
    assert!(!bad.ok());
    assert_eq!(
        bad.first_error(),
        Some(RulesError::Host(HostFault::BadTarget(9)))
    );
    let refused = rules.test("let r = 0.5;", &[]);
    assert_eq!(refused.refused, Some(RulesError::Float { line: 1 }));
    assert_eq!(rules.script_hash(), hash, "test installs nothing");
    assert_eq!(rules.history().len(), 1);
}

#[test]
fn good_edit_replaces_and_history_grows() {
    let mut rules = installed();
    let old = rules.script_hash().unwrap();
    let receipt = rules.set("heal(id, 1);", 77).expect("ok");
    assert_eq!(receipt.rev, 2);
    assert_eq!(receipt.replaced, Some(old));
    assert_eq!(rules.history().len(), 2);
    assert_eq!(rules.history()[1].tick, 77);
    // CRLF text hashes like LF text.
    assert_eq!(
        script_hash("heal(id, 1);\r\nstun(id, 1);\r\n"),
        script_hash("heal(id, 1);\nstun(id, 1);\n")
    );
    // An empty script is a valid (do-nothing) rule set.
    assert!(rules.set("", 80).is_ok());
    assert!(rules.tick(&bash_world(100)).unwrap().actions.is_empty());
}
#[test]
fn explain_logs_every_host_call_and_leaves_state_alone() {
    let rules = installed();
    let before = rules.state_digest();
    let explanation = rules.explain(&bash_world(100)).expect("explains");
    assert_eq!(
        rules.state_digest(),
        before,
        "explain does not write memory"
    );
    assert_eq!(explanation.rev, 1);
    assert_eq!(explanation.fighters.len(), 2);
    let me = &explanation.fighters[0];
    assert!(me.error.is_none());
    assert!(me.ops > 10 && me.ops < 200, "ops {}", me.ops);
    let names: Vec<&str> = me.calls.iter().map(|c| c.name).collect();
    assert_eq!(
        names,
        [
            "mem_get",
            "nearest_foe",
            "dist",
            "x_of",
            "z_of",
            "push",
            "stun",
            "damage",
            "drain_stamina",
            "shield",
            "tick",
            "mem_set"
        ]
    );
    assert_eq!(me.actions.len(), 5);
    assert_eq!(me.mem_after[0], 101);
    // The idle foe returns early: no host calls, a handful of operations.
    assert!(explanation.fighters[1].calls.is_empty());
    assert!(Rules::default().explain(&bash_world(1)).is_err());
}

#[test]
fn failed_call_discards_actions_and_memory() {
    let mut rules = Rules::default();
    // Actor 1 faults after writing memory and emitting; fighter 0 is healthy.
    let src = "mem_set(0, 9); stun(0, 5); if id == 1 { damage(40, 1); }";
    // `damage(40, 1)` is a bad target only in a 2-fighter world, which the smoke worlds with
    // 16 and 64 fighters do not hit for fighter 1: so install without the smoke run.
    rules.load_exact(src, 0).expect("vetted");
    let world = World {
        tick: 3,
        seed: 9,
        fighters: vec![fighter(0, 0, 0, 0), fighter(1, 1, 500, 0)],
    };
    let report = rules.tick(&world).expect("tick");
    assert_eq!(report.actions.len(), 1);
    assert_eq!(report.actions[0].actor, 0);
    assert_eq!(report.faults.len(), 1);
    assert_eq!(report.faults[0].actor, 1);
    assert_eq!(
        report.faults[0].error,
        RulesError::Host(HostFault::BadTarget(40))
    );
    let after = rules.explain(&world).expect("explain");
    assert_eq!(
        after.fighters[0].mem_after[0], 9,
        "healthy fighter keeps its write"
    );
    assert_eq!(
        after.fighters[1].mem_after[0], 0,
        "failed fighter's write rolled back"
    );
}

#[test]
fn roll_is_deterministic_per_actor_tick_and_seed() {
    let mut rules = Rules::default();
    rules
        .load_exact("mem_set(0, roll(1000)); mem_set(1, roll(1000));", 0)
        .unwrap();
    let world = ring_world(4, 12, 77);
    let a = rules.explain(&world).unwrap();
    let b = rules.explain(&world).unwrap();
    assert_eq!(a, b);
    let rolls: Vec<i64> = a.fighters.iter().map(|f| f.mem_after[0]).collect();
    assert!(
        rolls.windows(2).any(|p| p[0] != p[1]),
        "different actors roll differently: {rolls:?}"
    );
    let mut other = world.clone();
    other.seed = 78;
    assert_ne!(rules.explain(&other).unwrap(), a);
    assert!(rules.test("roll(0);", &[world]).first_error().is_some());
}

// ---------------------------------------------------------------- replay

#[test]
fn replay_10000_ticks_equal_twice() {
    let mut library = ScriptLibrary::new();
    let tape = record_scripted(8, 2040, 10_000, SHIELD_BASH, &mut library).expect("records");
    assert_eq!(tape.checkpoints.len(), 100);
    let first = replay(&tape, &library, Limits::DEFAULT).expect("replay 1");
    let second = replay(&tape, &library, Limits::DEFAULT).expect("replay 2");
    assert_eq!(first, second);
    assert_eq!(first, *tape.checkpoints.last().unwrap());
    // A second recording of the same match is byte-identical.
    let again = record_scripted(8, 2040, 10_000, SHIELD_BASH, &mut ScriptLibrary::new()).unwrap();
    assert_eq!(again.encode(), tape.encode());
}

#[test]
fn rules_change_the_outcome() {
    let mut library = ScriptLibrary::new();
    let with = record_scripted(8, 5, 2_000, SHIELD_BASH, &mut library).unwrap();
    let without = record_scripted(8, 5, 2_000, "// no rules\n", &mut library).unwrap();
    assert_ne!(with.checkpoints, without.checkpoints);
    assert_ne!(with.header_digest(), without.header_digest());
    assert_eq!(with.scripts[0].hash, script_hash(SHIELD_BASH));
}

#[test]
fn old_tape_still_replays_after_the_script_moves_on() {
    let mut library = ScriptLibrary::new();
    let v1 = SHIELD_BASH;
    let v2 = &SHIELD_BASH.replace("damage(foe, 40)", "damage(foe, 90)");
    assert_ne!(script_hash(v1), script_hash(v2));
    let old = record_scripted(8, 11, 1_500, v1, &mut library).unwrap();
    let new = record_scripted(8, 11, 1_500, v2, &mut library).unwrap();
    assert_ne!(
        old.checkpoints, new.checkpoints,
        "the edit changes the match"
    );
    // The live script is now v2; the old tape names v1 by hash and still replays.
    assert_eq!(old.scripts[0].hash, script_hash(v1));
    replay(&old, &library, Limits::DEFAULT).expect("old tape replays");
    replay(&new, &library, Limits::DEFAULT).expect("new tape replays");
    // Through bytes too.
    let decoded = RulesTape::decode(&old.encode()).expect("decodes");
    assert_eq!(decoded, old);
    replay(&decoded, &library, Limits::DEFAULT).expect("decoded tape replays");
}

#[test]
fn mid_match_hot_swap_replays() {
    let mut library = ScriptLibrary::new();
    let v2 = SHIELD_BASH.replace("stun(foe, 18)", "stun(foe, 30)");
    let spec = crate::tape::RecordSpec {
        fighters: 8,
        seed: 3,
        ticks: 2_000,
        checkpoint_every: 50,
        installs: &[(0, SHIELD_BASH), (700, &v2)],
        limits: Limits::DEFAULT,
    };
    let tape = crate::tape::record(&spec, scripted_inputs(8, 2_000, 3), &mut library).unwrap();
    assert_eq!(tape.scripts.len(), 2);
    assert_eq!(tape.scripts[1].tick, 700);
    replay(&tape, &library, Limits::DEFAULT).expect("swap replays");
    // A library without the second script refuses with a typed error.
    let mut partial = ScriptLibrary::new();
    partial.add(SHIELD_BASH);
    assert_eq!(
        replay(&tape, &partial, Limits::DEFAULT),
        Err(ReplayError::MissingScript(script_hash(&v2)))
    );
}

#[test]
fn replay_detects_divergence_and_abi_change() {
    let mut library = ScriptLibrary::new();
    let mut tape = record_scripted(4, 21, 600, SHIELD_BASH, &mut library).unwrap();
    replay(&tape, &library, Limits::DEFAULT).unwrap();
    // A different budget is a different ABI: refused before running.
    let tight = Limits {
        max_ops: 1_000,
        ..Limits::DEFAULT
    };
    assert_eq!(
        replay(&tape, &library, tight),
        Err(ReplayError::AbiMismatch {
            tape: rules_abi(&Limits::DEFAULT),
            engine: rules_abi(&tight)
        })
    );
    tape.checkpoints[2] ^= 1;
    assert!(matches!(
        replay(&tape, &library, Limits::DEFAULT),
        Err(ReplayError::Divergence { tick: 299, .. })
    ));
}

#[test]
fn tape_decoder_distrusts_everything() {
    let mut library = ScriptLibrary::new();
    let tape = record_scripted(4, 8, 300, SHIELD_BASH, &mut library).unwrap();
    let bytes = tape.encode();
    assert_eq!(RulesTape::decode(&bytes).unwrap(), tape);
    for cut in [0, 3, 5, 20, bytes.len() - 1] {
        assert!(RulesTape::decode(&bytes[..cut]).is_err(), "cut at {cut}");
    }
    let mut extra = bytes.clone();
    extra.push(0);
    assert_eq!(RulesTape::decode(&extra), Err(TapeError::Trailing));
    let mut bad_magic = bytes.clone();
    bad_magic[0] = b'X';
    assert_eq!(RulesTape::decode(&bad_magic), Err(TapeError::BadMagic));
    let mut bad_version = bytes.clone();
    bad_version[4] = 9;
    assert_eq!(RulesTape::decode(&bad_version), Err(TapeError::Version(9)));
    // A header that claims 4 billion inputs is refused before any allocation of that size.
    let mut huge = tape.header_bytes();
    huge.extend_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(
        RulesTape::decode(&huge),
        Err(TapeError::Invalid(_))
    ));
}

#[test]
fn second_machine_replays_tape_with_same_abi_and_ops() {
    // Machine A records the scripted match (shape: 2 fighters, 600 ticks).
    // Machine B is a fresh engine that receives only tape bytes: same ABI hash,
    // same checkpoints, same per-fighter op counts (ops are replay contract).
    for seed in [2040_u64, 7] {
        let mut library_a = ScriptLibrary::new();
        let tape_a = record_scripted(2, seed, 600, SHIELD_BASH, &mut library_a).expect("records");
        assert_eq!(tape_a.abi, rules_abi(&Limits::DEFAULT));
        // Ship bytes to the second machine and decode there.
        let bytes = tape_a.encode();
        let tape_b = RulesTape::decode(&bytes).expect("second machine decodes");
        assert_eq!(tape_b, tape_a);
        assert_eq!(tape_b.abi, rules_abi(&Limits::DEFAULT));
        let mut library_b = ScriptLibrary::new();
        library_b.add(SHIELD_BASH);
        let first = replay(&tape_b, &library_b, Limits::DEFAULT).expect("second replays");
        let second = replay(&tape_b, &library_b, Limits::DEFAULT).expect("second replays again");
        assert_eq!(first, second);
        assert_eq!(first, *tape_b.checkpoints.last().unwrap());
    }
    // Op counts are stable across two fresh engines on the same world.
    let world = bash_world(100);
    let a = installed();
    let b = installed();
    let ea = a.explain(&world).expect("explains");
    let eb = b.explain(&world).expect("explains");
    let ops_a: Vec<u64> = ea.fighters.iter().map(|f| f.ops).collect();
    let ops_b: Vec<u64> = eb.fighters.iter().map(|f| f.ops).collect();
    assert_eq!(ops_a, ops_b);
}

/// A spec the recorder must refuse: the tape it would write cannot decode
/// back into the same match, or the recorder trips over it.
fn spec_with(field: &str) -> crate::tape::RecordSpec<'static> {
    crate::tape::RecordSpec {
        fighters: 4,
        seed: 3,
        ticks: 100,
        checkpoint_every: 10,
        installs: &[],
        limits: Limits::DEFAULT,
    }
    .with(field)
}

trait With {
    fn with(self, field: &str) -> crate::tape::RecordSpec<'static>;
}

impl With for crate::tape::RecordSpec<'static> {
    fn with(mut self, field: &str) -> crate::tape::RecordSpec<'static> {
        match field {
            "fighters" => self.fighters = 0,
            "ticks" => self.ticks = 0,
            "checkpoint_every" => self.checkpoint_every = 0,
            "installs" => self.installs = &[(10, SHIELD_BASH), (10, SHIELD_BASH)],
            _ => panic!("unknown field {field}"),
        }
        self
    }
}
#[test]
fn record_refuses_a_spec_the_decoder_would_reject() {
    let mut library = ScriptLibrary::new();
    // A zero cadence divided the checkpoint count by zero: the recorder
    // panicked instead of refusing the spec.
    assert_eq!(
        crate::tape::record(&spec_with("checkpoint_every"), Vec::new(), &mut library),
        Err(RulesError::BadSpec {
            field: "checkpoint_every",
            value: 0
        })
    );
    // A match with no fighters or no ticks is a tape the decoder refuses.
    assert_eq!(
        crate::tape::record(&spec_with("fighters"), Vec::new(), &mut library),
        Err(RulesError::BadSpec {
            field: "fighters",
            value: 0
        })
    );
    assert_eq!(
        crate::tape::record(&spec_with("ticks"), Vec::new(), &mut library),
        Err(RulesError::BadSpec {
            field: "ticks",
            value: 0
        })
    );
    // Two installs on one tick write two rows with the same tick, and the
    // decoder refuses a tape whose script ticks do not rise.
    assert_eq!(
        crate::tape::record(&spec_with("installs"), Vec::new(), &mut library),
        Err(RulesError::BadSpec {
            field: "installs",
            value: 10
        })
    );
}

#[test]
fn a_recorded_tape_decodes_back_into_the_same_match() {
    let mut library = ScriptLibrary::new();
    let spec = crate::tape::RecordSpec {
        fighters: 4,
        seed: 3,
        ticks: 100,
        checkpoint_every: 10,
        installs: &[(0, SHIELD_BASH)],
        limits: Limits::DEFAULT,
    };
    let tape =
        crate::tape::record(&spec, scripted_inputs(4, 100, 3), &mut library).expect("records");
    let decoded = RulesTape::decode(&tape.encode()).expect("the recorded tape decodes");
    assert_eq!(decoded, tape);
    replay(&decoded, &library, Limits::DEFAULT).expect("the decoded tape replays");
}

#[test]
fn replay_refuses_a_malformed_tape_instead_of_panicking() {
    let mut library = ScriptLibrary::new();
    let mut tape = record_scripted(4, 21, 600, SHIELD_BASH, &mut library).unwrap();
    // One checkpoint short: the replay used to index past the end.
    tape.checkpoints.pop();
    assert!(matches!(
        replay(&tape, &library, Limits::DEFAULT),
        Err(ReplayError::Tape(TapeError::Invalid(_)))
    ));
    // A tape whose inputs are not sorted is refused as well.
    let mut unsorted = record_scripted(2, 21, 60, SHIELD_BASH, &mut library).unwrap();
    unsorted.inputs.reverse();
    assert!(matches!(
        replay(&unsorted, &library, Limits::DEFAULT),
        Err(ReplayError::Tape(TapeError::Invalid(_)))
    ));
}

#[test]
fn committed_old_tape_replays() {
    // Written once by `bless_old_tape` and committed: a tape from the past with its script.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    let tape_bytes = std::fs::read(dir.join("shield_bash_v1.e2rl")).expect("committed tape");
    let script =
        std::fs::read_to_string(dir.join("shield_bash_v1.rhai")).expect("committed script");
    let tape = RulesTape::decode(&tape_bytes).expect("committed tape decodes");
    let mut library = ScriptLibrary::new();
    library.add(&script);
    assert_eq!(tape.scripts[0].hash, script_hash(&script));
    replay(&tape, &library, Limits::DEFAULT).expect("a tape from the past still replays");
}

#[test]
#[ignore = "writes tests/data once; run by hand when the ABI version is bumped on purpose"]
fn bless_old_tape() {
    let mut library = ScriptLibrary::new();
    let tape = record_scripted(8, 1991, 1_200, SHIELD_BASH, &mut library).unwrap();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("shield_bash_v1.e2rl"), tape.encode()).unwrap();
    std::fs::write(dir.join("shield_bash_v1.rhai"), SHIELD_BASH).unwrap();
}

// ---------------------------------------------------------------- budget and build guards

#[test]
fn rules_crate_is_not_a_default_member() {
    let root = include_str!("../../../Cargo.toml");
    let members = root
        .split("default-members")
        .nth(1)
        .expect("default-members block");
    let block = &members[..members.find(']').expect("block end")];
    assert!(
        !block.contains("crates/rules"),
        "engine-rules must stay out of the default build"
    );
}

#[test]
fn bad_worlds_are_refused_not_run() {
    let mut rules = installed();
    let mut world = bash_world(1);
    world.fighters[1].id = 5;
    assert_eq!(
        rules.tick(&world),
        Err(RulesError::BadWorld { fighters: 2 })
    );
    let big = World {
        tick: 0,
        seed: 0,
        fighters: (0..65).map(|i| fighter(i as u8, 0, 0, 0)).collect(),
    };
    assert_eq!(rules.tick(&big), Err(RulesError::BadWorld { fighters: 65 }));
}

#[test]
fn state_snapshot_restores_memory() {
    let mut rules = installed();
    let snap = rules.snapshot();
    let digest = rules.state_digest();
    rules.tick(&bash_world(100)).unwrap();
    assert_ne!(rules.state_digest(), digest);
    rules.restore(&snap);
    assert_eq!(rules.state_digest(), digest);
}

// ---------------------------------------------------------------- walled scene, zero providers

/// One 2 m wall across the middle of the reference arena, millimetres.
fn center_wall() -> Wall {
    Wall::new(-1_000, -100, 1_000, 100)
}

/// Run `ticks` of a 2-fighter match on the walled scene with `script` installed (`None`
/// means the plain walled run). Only `Rules` and the static world take part: no arena, no
/// door, no providers. Returns the end digest and the actions the script emitted.
fn run_walled(script: Option<&str>, seed: u64, ticks: u32) -> (u64, u64) {
    let mut rules = Rules::default();
    if let Some(source) = script {
        rules.set(source, 0).expect("walled script installs");
    }
    let mut sim = Sim::new(2, seed).with_wall(center_wall());
    let inputs = scripted_inputs(2, ticks, seed);
    let mut actions = 0_u64;
    for _ in 0..ticks {
        let report = sim.step(&inputs, &mut rules).expect("walled step");
        actions += report.actions.len() as u64;
        for view in sim.fighters() {
            assert!(
                !center_wall().contains(view.x_mm, view.z_mm),
                "seed {seed}: fighter {} inside the wall at ({}, {})",
                view.id,
                view.x_mm,
                view.z_mm
            );
        }
    }
    (sim.digest(), actions)
}

#[test]
fn scripted_slot_on_a_walled_scene() {
    const SHOVE: &str = "let foe = nearest_foe(id); if foe >= 0 { push(foe, 500, 0); }";
    const TICKS: u32 = 600;
    let (scripted, actions) = run_walled(Some(SHOVE), 7, TICKS);
    let (plain, _) = run_walled(None, 7, TICKS);
    assert!(actions > 0, "the script must fire on the walled scene");
    assert_ne!(
        scripted, plain,
        "the scripted walled run must differ from the plain walled run"
    );
    let (again, _) = run_walled(Some(SHOVE), 7, TICKS);
    assert_eq!(
        scripted, again,
        "the scripted walled run must replay exactly"
    );
    for seed in 0..20 {
        run_walled(Some(SHOVE), seed, TICKS);
    }
}

// ---------------------------------------------------------------- effect actions as integer Rhai

/// Install a one-line effect script: only slot 0 fires, so one tick is one action.
fn effect_rules(source: &str) -> Rules {
    let mut rules = Rules::default();
    rules.set(source, 0).expect("effect script installs");
    rules
}

#[test]
fn damage_is_integer_and_kills_at_zero() {
    let mut rules = effect_rules("if id == 0 { damage(1, 300); }");
    let mut sim = Sim::new(2, 11);
    sim.step(&[], &mut rules).expect("step");
    assert_eq!(sim.fighters()[1].hp_permille, 700);
    assert!(sim.fighters()[1].alive);
    let mut killer = effect_rules("if id == 0 { damage(1, 1000); }");
    sim.step(&[], &mut killer).expect("step");
    assert_eq!(sim.fighters()[1].hp_permille, 0);
    assert!(!sim.fighters()[1].alive);
    // Overkill saturates, never wraps.
    sim.step(&[], &mut killer).expect("step");
    assert_eq!(sim.fighters()[1].hp_permille, 0);
}

#[test]
fn heal_restores_capped_and_never_raises_the_dead() {
    let mut rules = effect_rules("if id == 0 { damage(1, 800); }");
    let mut sim = Sim::new(2, 12);
    sim.step(&[], &mut rules).expect("step");
    assert_eq!(sim.fighters()[1].hp_permille, 200);
    let mut healer = effect_rules("if id == 0 { heal(1, 500); }");
    sim.step(&[], &mut healer).expect("step");
    assert_eq!(sim.fighters()[1].hp_permille, 700);
    sim.step(&[], &mut healer).expect("step");
    assert_eq!(sim.fighters()[1].hp_permille, 1_000, "heal caps at full");
    // Kill, then heal: the dead stay dead.
    let mut killer = effect_rules("if id == 0 { damage(1, 1000); }");
    sim.step(&[], &mut killer).expect("step");
    assert!(!sim.fighters()[1].alive);
    sim.step(&[], &mut healer).expect("step");
    assert_eq!(sim.fighters()[1].hp_permille, 0);
    assert!(!sim.fighters()[1].alive);
}

#[test]
fn stun_sets_react_and_keeps_the_longest() {
    let mut rules = effect_rules("if id == 0 { stun(1, 45); }");
    let mut sim = Sim::new(2, 13);
    sim.step(&[], &mut rules).expect("step");
    assert_eq!(
        sim.fighters()[1].react,
        44,
        "applied before the timers tick down"
    );
    let mut tap = effect_rules("if id == 0 { stun(1, 5); }");
    sim.step(&[], &mut tap).expect("step");
    assert_eq!(sim.fighters()[1].react, 43, "a shorter stun never shortens");
}

#[test]
fn drain_removes_stamina_and_saturates() {
    let mut rules = effect_rules("if id == 0 { drain_stamina(1, 300); }");
    let mut sim = Sim::new(2, 14);
    sim.step(&[], &mut rules).expect("step");
    assert_eq!(sim.fighters()[1].stamina, 702, "300 drained, 2 regrown");
    let mut empty = effect_rules("if id == 0 { drain_stamina(1, 1000); }");
    sim.step(&[], &mut empty).expect("step");
    assert_eq!(
        sim.fighters()[1].stamina,
        2,
        "drain saturates at zero, then 2 regrow"
    );
}

#[test]
fn effects_replay_exactly() {
    const ALL: &str =
        "if id == 0 { damage(1, 100); heal(0, 50); stun(1, 10); drain_stamina(1, 20); }";
    let mut first = Sim::new(2, 15);
    let mut second = Sim::new(2, 15);
    let mut a = effect_rules(ALL);
    let mut b = effect_rules(ALL);
    for _ in 0..60 {
        let ra = first.step(&[], &mut a).expect("step");
        let rb = second.step(&[], &mut b).expect("step");
        assert_eq!(ra.digest(), rb.digest(), "every tick digests equal");
    }
    assert_eq!(first.digest(), second.digest(), "same seed, same match");
}
