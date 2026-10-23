use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use engine_arena::{ArenaConfig, BodyId, BodyInput, MatchSpec};

mod common;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        // create_dir never reuses another process's existing directory.
        loop {
            let path = std::env::temp_dir().join(format!(
                "engine2040-proof-{}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("temporary directory: {error}"),
            }
        }
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_engine2040"))
        .args(args)
        .output()
        .unwrap()
}

fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn default_and_max_seed_have_real_machine_readable_receipts() {
    let default = success(run(&["--format", "json", "--verify"]));
    assert!(default.contains("\"final_hash\":\"d57b5c8ae3096614\""));
    assert!(default.contains("\"golden_checked\":true"));
    assert!(default.contains("\"verified\":true"));
    let maximum = success(run(&[
        "--seed",
        "18446744073709551615",
        "--ticks",
        "12",
        "--format",
        "json",
    ]));
    assert!(maximum.contains("\"seed\":\"18446744073709551615\""));
    assert!(maximum.contains("\"ticks\":12"));
    let changed = success(run(&["--seed", "19", "--ticks", "12", "--format", "json"]));
    assert_ne!(
        maximum.split("\"final_hash\":").nth(1).unwrap(),
        changed.split("\"final_hash\":").nth(1).unwrap()
    );
    // The stable text receipt is the reviewed snapshot: no version or platform
    // field, so it reads the same on every runner. A later drift leaves
    // `default-text-receipt.snap.new` and names it until someone reviews it and
    // reruns with E2040_ACCEPT_SNAPSHOTS=1.
    let text = success(run(&["--verify"]));
    common::assert_snapshot("proof-cli/default-text-receipt", &text);
}

#[test]
fn strict_arguments_and_unsupported_backends_fail() {
    for args in [
        vec!["--seed"],
        vec!["--seed", "no"],
        vec!["--unknown"],
        vec!["--seed", "1", "--seed", "2"],
        vec!["--backend", "wgpu"],
        vec!["--ticks", "0"],
        vec!["--ticks", "100001"],
        vec!["--input", "0:100:1:2"],
        vec!["--input", "0:0:NaN:2"],
        vec!["--replay", "missing", "--seed", "7"],
    ] {
        let output = run(&args);
        assert!(!output.status.success(), "unexpected success: {args:?}");
        assert!(!output.stderr.is_empty());
    }
    assert!(success(run(&["--help"])).contains("--replay"));
}

#[test]
fn saved_nonempty_tape_replays_and_corruption_fails_with_tick() {
    // Relative tape path: absolute record/replay paths are refused loudly,
    // so the roundtrip stays inside the working directory with a unique name.
    let file = format!(
        "engine2040-proof-{}-{} input tape.e2040",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    );
    let path = file.as_str();
    struct Guard(String);
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
    let _guard = Guard(file.clone());
    let receipt = success(run(&[
        "--seed", "19", "--ticks", "12", "--input", "3:0:-8:4", "--record", path, "--verify",
        "--format", "json",
    ]));
    let replayed = success(run(&["--replay", path, "--format", "json"]));
    assert!(replayed.contains("\"mode\":\"replay\""));
    assert!(replayed.contains("\"inputs\":1"));
    let suffix = |text: &str| text.split("\"final_hash\":").nth(1).unwrap().to_owned();
    assert_eq!(suffix(&receipt), suffix(&replayed));
    let bytes = fs::read(path).unwrap();
    assert!(!run(&["--record", path]).status.success());
    assert_eq!(fs::read(path).unwrap(), bytes);
    let mut tape = engine_proof::Tape::decode(&bytes).unwrap();
    tape.checkpoints[4].state ^= 1;
    fs::write(path, tape.encode().unwrap()).unwrap();
    let bad = run(&["--replay", path]);
    assert_eq!(bad.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&bad.stderr).contains("diverged at tick 4"));
    fs::write(path, &bytes[..bytes.len() - 1]).unwrap();
    assert!(!run(&["--replay", path]).status.success());
}

#[test]
fn record_verify_replays_twice_byte_identical_within_1pct_overhead() {
    // 600-tick default fixture: relative paths stay inside the working
    // directory because absolute record/replay paths are refused.
    let tag = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let file = format!("engine2040-verify-{pid}-{tag}.e2040");
    let partial = format!("engine2040-verify-{pid}-{tag}-partial.e2040");
    struct Guards(Vec<String>);
    impl Drop for Guards {
        fn drop(&mut self) {
            for name in &self.0 {
                let _ = fs::remove_file(name);
            }
        }
    }
    let _guards = Guards(vec![file.clone(), partial.clone()]);
    // Clean record with verify-after-record on the frozen 600-tick fixture.
    let receipt = success(run(&["--record", &file, "--verify", "--format", "json"]));
    assert!(receipt.contains("\"mode\":\"record\""));
    assert!(receipt.contains("\"verified\":true"));
    assert!(receipt.contains("\"golden_checked\":true"));
    assert!(receipt.contains("\"final_hash\":\"d57b5c8ae3096614\""));
    let bytes = fs::read(&file).unwrap();
    // Record overhead is the header share of the tape file, posted here.
    let tape = engine_proof::Tape::decode(&bytes).unwrap();
    let ticks = usize::try_from(tape.config.ticks).unwrap();
    let payload = ticks * 16 + tape.inputs.len() * 16;
    let total = bytes.len();
    let header = total - payload;
    println!("record overhead: header {header} payload {payload} total {total}");
    assert!(
        header * 100 <= total,
        "record overhead {header}/{total} exceeds 1% (ticks {ticks} inputs {})",
        tape.inputs.len()
    );
    // Clean replay x2 is byte-identical: same receipt and same file bytes.
    let first = success(run(&["--replay", &file, "--format", "json"]));
    let after_first = fs::read(&file).unwrap();
    let second = success(run(&["--replay", &file, "--format", "json"]));
    let after_second = fs::read(&file).unwrap();
    assert_eq!(bytes, after_first, "replay mutated the tape file");
    assert_eq!(bytes, after_second, "second replay mutated the tape file");
    assert_eq!(first, second, "replay receipts differ");
    let suffix = |text: &str| text.split("\"final_hash\":").nth(1).unwrap().to_owned();
    assert_eq!(suffix(&receipt), suffix(&first));
    assert_eq!(suffix(&first), suffix(&second));
    // Kill -9 style truncated copy refuses verify and names the partial file.
    let cut = bytes.len() / 2;
    fs::write(&partial, &bytes[..cut]).unwrap();
    let bad = run(&["--replay", &partial]);
    assert!(!bad.status.success(), "truncated partial replayed");
    let stderr = String::from_utf8_lossy(&bad.stderr).to_string();
    assert!(
        stderr.contains(&partial),
        "partial refusal must name {partial}: {stderr}"
    );
}

#[test]
fn lane_traversal_and_absolute_record_paths_are_refused_loudly() {
    let tag = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    // Absolute candidate inside an isolated temp dir; refusal happens before
    // any write so the file must never appear.
    let temp = Temp::new();
    let absolute = temp.0.join(format!("evil-{pid}-{tag}.e2040"));
    let absolute = absolute.to_str().unwrap().to_owned();
    let lane = format!(".lanes/evil-{pid}-{tag}.e2040");
    let traversal = format!("../evil-{pid}-{tag}.e2040");
    for path in [lane.as_str(), traversal.as_str(), absolute.as_str()] {
        let record = run(&["--seed", "19", "--ticks", "12", "--record", path]);
        assert!(!record.status.success(), "unexpected success: {path}");
        assert!(
            String::from_utf8_lossy(&record.stderr).contains("unsafe tape path"),
            "stderr for {path}: {}",
            String::from_utf8_lossy(&record.stderr)
        );
        assert!(!PathBuf::from(path).exists(), "refused path wrote {path}");
        let replay = run(&["--replay", path]);
        assert!(
            !replay.status.success(),
            "unexpected replay success: {path}"
        );
        assert!(
            String::from_utf8_lossy(&replay.stderr).contains("unsafe tape path"),
            "replay stderr for {path}: {}",
            String::from_utf8_lossy(&replay.stderr)
        );
    }
    assert!(!PathBuf::from(&absolute).exists());
}

#[test]
fn journal_run_matches_the_tape_and_a_torn_or_damaged_journal_refuses_by_name() {
    let tag = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let journal = format!("engine2040-journal-{pid}-{tag}.e2040j");
    let tape = format!("engine2040-journal-{pid}-{tag}-tape.e2040");
    let plain = format!("engine2040-journal-{pid}-{tag}-plain.e2040");
    let torn = format!("engine2040-journal-{pid}-{tag}-torn.e2040j");
    let damaged = format!("engine2040-journal-{pid}-{tag}-damaged.e2040j");
    struct Files(Vec<String>);
    impl Drop for Files {
        fn drop(&mut self) {
            for name in &self.0 {
                let _ = fs::remove_file(name);
            }
        }
    }
    let _files = Files(vec![
        journal.clone(),
        tape.clone(),
        plain.clone(),
        torn.clone(),
        damaged.clone(),
    ]);
    let run_args = ["--seed", "19", "--ticks", "12", "--input", "3:0:-8:4"];
    let with = |extra: &[&str]| {
        let mut args = run_args.to_vec();
        args.extend_from_slice(extra);
        success(run(&args))
    };
    // One run streams the journal and writes the tape; --verify replays both.
    let receipt = with(&[
        "--journal",
        &journal,
        "--record",
        &tape,
        "--verify",
        "--format",
        "json",
    ]);
    assert!(receipt.contains("\"mode\":\"record\""));
    assert!(receipt.contains("\"verified\":true"));
    with(&["--record", &plain]);
    assert_eq!(fs::read(&tape).unwrap(), fs::read(&plain).unwrap());
    let bytes = fs::read(&journal).unwrap();
    assert!(bytes.starts_with(b"E2040JN\0"));
    // The complete journal replays to the same receipt as the tape.
    let suffix = |text: &str| text.split("\"final_hash\":").nth(1).unwrap().to_owned();
    let replayed = success(run(&["--replay", &journal, "--format", "json"]));
    assert!(replayed.contains("\"mode\":\"replay\""));
    assert_eq!(suffix(&receipt), suffix(&replayed));
    assert_eq!(
        fs::read(&journal).unwrap(),
        bytes,
        "replay mutated the journal"
    );
    // A journal cut inside its last record keeps no half tick: it refuses,
    // names the file and says one tick is missing.
    fs::write(&torn, &bytes[..bytes.len() - 5]).unwrap();
    let bad = run(&["--replay", &torn]);
    assert_eq!(bad.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&bad.stderr).to_string();
    assert!(stderr.contains(&torn), "{stderr}");
    assert!(stderr.contains("1 of 12 ticks missing"), "{stderr}");
    assert!(stderr.contains("torn trailing record"), "{stderr}");
    // A flipped byte inside a record is damage, never a shorter run.
    let mut flipped = bytes.clone();
    let middle = flipped.len() / 2;
    flipped[middle] ^= 0xff;
    fs::write(&damaged, &flipped).unwrap();
    let bad = run(&["--replay", &damaged]);
    assert_eq!(bad.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&bad.stderr).to_string();
    assert!(stderr.contains(&damaged), "{stderr}");
    assert!(stderr.contains("journal corrupt at tick"), "{stderr}");
    // Journals never overwrite, never mix with replay, never escape the cwd.
    assert!(!run(&["--journal", &journal, "--ticks", "3"])
        .status
        .success());
    assert_eq!(fs::read(&journal).unwrap(), bytes);
    assert_eq!(
        run(&["--replay", &journal, "--journal", "x"]).status.code(),
        Some(2)
    );
    let escape = run(&["--ticks", "3", "--journal", "../escape.e2040j"]);
    assert!(String::from_utf8_lossy(&escape.stderr).contains("unsafe tape path"));
}

/// Fixed steps of the shipped demo bout (`docs/DEMO.md`): 100,000 at 60 Hz.
const DEMO_TICKS: u32 = 100_000;
/// The duel's gladius row (`engine_game::DUEL_KIT_WEAPON`): the demo window
/// arms one per slot before the first tick.
const DEMO_WEAPON: &str = "gladius";

/// One tick of the demo's human seat: the fixed stand-in for the keyboard.
/// Slot 0 swings every 30 ticks, guards for a window and dodges once per 120,
/// and faces a slow sweep; slot 1 is an arena brain, as in the demo.
fn demo_input(tick: u32) -> BodyInput {
    let phase = tick % 120;
    let mut bits = 0u8;
    if phase.is_multiple_of(30) {
        bits |= 0x01;
    }
    if (84..96).contains(&phase) {
        bits |= 0x02;
    }
    if phase == 108 {
        bits |= 0x04;
    }
    let sweep = i32::try_from((tick * 5) % 36_000).unwrap_or(0) - 18_000;
    let mut input = BodyInput {
        tick,
        fighter: BodyId(0),
        mx: if (phase / 30).is_multiple_of(2) {
            100
        } else {
            -60
        },
        mz: 30,
        facing: i16::try_from(sweep).unwrap_or(0),
        buttons: Default::default(),
    };
    input.buttons.0 = bits;
    input
}

/// Play the demo bout once from a fresh match: the window's spec
/// (`App::with_world(..).into_duel().into_spear()`) at the demo's shipped
/// seed, length, fighter count and arena size. Returns the checkpoint hash at
/// tick 0 and the final checkpoint hash.
fn play_demo_bout() -> (u64, u64) {
    let config = ArenaConfig {
        seed: 2040,
        ticks: DEMO_TICKS,
        fighters: 2,
        half_extent_m: 20.0,
        table_version: 2,
    };
    let spec = MatchSpec::kit_duel(config)
        .expect("the demo kit world builds")
        .with_spear()
        .with_weapons(&[DEMO_WEAPON, DEMO_WEAPON]);
    let mut live = spec.live().expect("the demo match builds");
    let mut first = 0u64;
    let mut last = 0u64;
    for tick in 0..DEMO_TICKS {
        let stepped = live
            .step(&[demo_input(tick)])
            .unwrap_or_else(|| panic!("the demo match refused tick {tick}"));
        assert_eq!(stepped.checkpoint.tick, tick, "checkpoint names its tick");
        if tick == 0 {
            first = stepped.checkpoint.hash;
        }
        last = stepped.checkpoint.hash;
    }
    assert_eq!(live.tick(), DEMO_TICKS, "the bout ends at the demo length");
    (first, last)
}

/// The shipped demo length, proven: the arena bout `play-arena.bat` plays is
/// 100,000 fixed ticks (`docs/DEMO.md`), while the golden gate proves 600 and
/// the HS2 arena matrix 1,200. This plays the demo's own match — rules v2 on
/// the arena kit walls, one gladius per slot, spear on, seed 2040, two
/// fighters with a scripted human seat — twice from fresh state and requires
/// the same 16-hex final checkpoint hash. A drift that only shows after the
/// 1,200-tick matrix stops passing here.
///
/// Ignored so ordinary CI stays fast; run it explicitly:
/// `node tools/gh-check.mjs --budget 1700 cargo test -p engine2040 --release --test proof_cli demo_100k -- --ignored --exact --nocapture`
///
/// Resume gap, measured 2026-10-23 (findings only, no fix here):
/// `App::save_progress` seals the 25-byte `GameProgress` (three counters plus
/// seed and tick) through `engine_save::save_generation`; it never writes the
/// match. `App` already holds the full human input tape and can rebuild by
/// `spec().live()` plus replay, and `engine_net::replay::ReplayTape` already
/// carries a spec identity, per-tick checkpoints and a 600-tick keyframe grid.
/// What is missing is persistence: the payload has no fighter count, arena
/// size, table/rules version, spear/height/weapon flags, world digest or
/// inputs, and the 1 MiB payload cap is close to a 100k one-slot tape. A
/// `MatchSnapshot` cannot fill that hole: `LiveMatch::snapshot` returns `None`
/// for the demo match because the snapshot carries no obstacle world, no v2
/// balance/stagger/limb state and no spear slots.
#[test]
#[ignore = "the 100k demo-length gate; run explicitly"]
fn demo_100k() {
    let started = std::time::Instant::now();
    let (first, hash_a) = play_demo_bout();
    let (_, hash_b) = play_demo_bout();
    let wall_s = started.elapsed().as_secs_f64();
    println!(
        "demo_100k: ticks={DEMO_TICKS} hash_a={hash_a:016x} hash_b={hash_b:016x} wall_s={wall_s:.3}"
    );
    assert_ne!(
        hash_a, 0,
        "an all-zero checkpoint hash is not a played bout"
    );
    assert_ne!(hash_a, first, "the bout never left its first tick");
    assert_eq!(hash_a, hash_b, "two fresh 100k demo bouts diverged");
}
