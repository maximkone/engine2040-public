use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

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
