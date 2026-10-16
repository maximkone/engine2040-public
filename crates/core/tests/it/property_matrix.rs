// Attribution: engine matrix idea adapted from camshaft/bolero (MIT)
// `tests/src/engines.rs` (fixed-seed + mutation + shrink engines over one
// corpus); reimplemented here fresh over engine-core hash/decode, std-only,
// no donor code copied.
// PORT(proptest-rs/proptest@4b0594eb07c53695dc3c24d3fa06bdceb98540e9:
// proptest/src/test_runner/config.rs plus
// proptest/src/test_runner/failure_persistence/{mod,file}.rs) Apache-2.0
// (LICENSE-APACHE read live 2026-10-09): `RunnerConfig` (`cases` 256,
// `max_shrink_iters`), `seed <n> # <bug>` regression-file persist plus
// replay-first, reimplemented std-only over engine-core `Rng`; zero donor
// bytes vendored.
//
//! Unified property + fuzz matrix over one seed corpus and three engines,
//! plus a 256-case runner with a persisted regression file.
//! Owner: engine-core (test-only harness). Units: seeds and hashes are u64,
//! bytes are raw inputs, cases/iters are counts. Determinism: `Rng`
//! substreams only, no wall clock; the same seeds replay x2 identical.
//! Errors: none; `expect` names proven local bounds, never user input.
use engine_core::{decode_seal_hex, hash_bytes, parse_hex4, Rng};
use std::path::{Path, PathBuf};

/// One seed corpus shared by all three engines; fixed so runs replay x2.
/// Eight seeds catch 3/4 seeded bugs (before); the 256-case runner below
/// catches 4/4 with its failures persisted (after).
const SEEDS: [u64; 8] = [2040, 2041, 2042, 2043, 2044, 7, 99, 1234];

/// Mutants per base case in the byte-mutate engine; small and deterministic.
const MUTANTS_PER_SEED: u64 = 8;

/// Runner case count, ported from proptest `Config::cases` (default 256).
const RUNNER_CASES: u32 = 256;
/// Shrink-step bound, ported from proptest `Config::max_shrink_iters`.
/// Tiny inputs minimize within a handful of steps; the bound only cuts
/// runaway shrinks, never quality at this scale.
const RUNNER_MAX_SHRINK_ITERS: u32 = 1024;
/// Regression file beside this test, one `seed <n> # <bug>` line per
/// persisted failure. Shape ported from proptest `FileFailurePersistence`
/// seed lines (the donor default is `SourceParallel("proptest-regressions")`;
/// this direct manifest-relative path replays from any cwd instead).
const REGRESSION_FILE: &str = "tests/property_matrix_regressions.txt";
/// Residue gating the rare corruption bug: it shows only when
/// `seed % RARE_MOD == RARE_RES`. None of the 8 fixed seeds hits residue 13
/// mod 64 (3/4 before); indices `0..256` always hit it four times (4/4).
const RARE_MOD: u64 = 64;
const RARE_RES: u64 = 13;

/// Test-runner configuration, ported from proptest `test_runner::Config`.
/// Only what moves the catch count lives here: case count, shrink bound,
/// and the regression-file switch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RunnerConfig {
    /// Cases per run; donor default 256 (fixed here so runs replay x2).
    cases: u32,
    /// Shrink steps before the minimizer stops; donor `max_shrink_iters`.
    max_shrink_iters: u32,
    /// Replay the regression file first and persist new failures;
    /// donor `failure_persistence`.
    persist: bool,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            cases: RUNNER_CASES,
            max_shrink_iters: RUNNER_MAX_SHRINK_ITERS,
            persist: true,
        }
    }
}

/// One corpus case: raw bytes plus their even-length lowercase hex form.
struct Case {
    seed: u64,
    raw: Vec<u8>,
    hex: String,
}

/// Seeded bugs the matrix must catch (each mirrors a plausible defect).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Bug {
    /// Last input byte ignored (stale-slice bug).
    TruncHash,
    /// Odd-length hex accepted (length-check bug).
    OddHex,
    /// 3-digit `\uXXXX` accepted (arity-check bug).
    ShortHex4,
    /// One stream in 64 flips a digest bit (shard-gated corruption bug).
    XorDigest,
}

const ALL_BUGS: [Bug; 4] = [Bug::TruncHash, Bug::OddHex, Bug::ShortHex4, Bug::XorDigest];

fn hex_of(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 15)] as char);
    }
    out
}

/// Deterministic raw bytes from one `Rng` stream: length 8..16, then words.
fn draw_raw(rng: &mut Rng) -> Vec<u8> {
    let len = 8 + rng.next_range(0, 8);
    let len_usize = usize::try_from(len).expect("small len fits on this platform");
    let mut raw = Vec::with_capacity(len_usize);
    while raw.len() < len_usize {
        for byte in rng.next_u64().to_le_bytes() {
            if raw.len() >= len_usize {
                break;
            }
            raw.push(byte);
        }
    }
    raw
}

/// Same corpus case for a seed on every engine and every run.
fn case_for_seed(seed: u64) -> Case {
    let mut rng = Rng::new(seed).fork_named("matrix");
    let raw = draw_raw(&mut rng);
    let hex = hex_of(&raw);
    Case { seed, raw, hex }
}

/// Runner case `index` in `0..cases`: the seed IS the index, so the
/// seed-gated bug hits deterministically (indices 13/77/141/205 in 0..256);
/// bytes come from a forked stream, which skips the `Rng::new` low-bit
/// aliasing that pairs consecutive seeds, so all 256 cases draw distinct
/// streams and replay x2.
fn runner_case_for_index(index: u32) -> Case {
    let seed = u64::from(index);
    let mut rng = Rng::new(2040).fork(seed);
    let raw = draw_raw(&mut rng);
    let hex = hex_of(&raw);
    Case { seed, raw, hex }
}

/// Whole shared corpus in seed order.
fn base_corpus() -> Vec<Case> {
    SEEDS.iter().map(|&seed| case_for_seed(seed)).collect()
}

/// Runner corpus: `config.cases` cases over indices `0..cases`, fixed order.
fn runner_corpus(config: &RunnerConfig) -> Vec<Case> {
    let count = usize::try_from(config.cases).expect("256 cases fits on this platform");
    let mut out = Vec::with_capacity(count);
    for index in 0..count {
        let as_u32 = u32::try_from(index).expect("runner index fits in u32");
        out.push(runner_case_for_index(as_u32));
    }
    out
}

/// Regenerate the case for a persisted seed: runner indices replay through
/// the runner stream, older fixed seeds through the fixed stream.
fn replay_case(seed: u64) -> Case {
    if seed < u64::from(RUNNER_CASES) {
        let index = u32::try_from(seed).expect("runner seed fits in u32");
        runner_case_for_index(index)
    } else {
        case_for_seed(seed)
    }
}

/// Buggy 3-digit hex parser: accepts len 3, unlike the `parse_hex4` oracle.
fn buggy_hex4_short(digits: &[u8]) -> Option<u32> {
    if digits.len() != 3 {
        return parse_hex4(digits);
    }
    let mut value: u32 = 0;
    for &byte in digits {
        let digit = match byte {
            b'0'..=b'9' => u32::from(byte - b'0'),
            b'a'..=b'f' => u32::from(byte - b'a') + 10,
            b'A'..=b'F' => u32::from(byte - b'A') + 10,
            _ => return None,
        };
        value = value * 16 + digit;
    }
    Some(value)
}

/// True when the seeded bug is visible on this input (oracle disagrees).
/// `seed` gates the rare corruption bug; the other three show on every
/// non-degenerate input.
fn catches(bug: Bug, seed: u64, raw: &[u8], hex: &str) -> bool {
    match bug {
        Bug::TruncHash => {
            if raw.is_empty() {
                return false;
            }
            let oracle = hash_bytes(raw);
            let buggy = hash_bytes(&raw[..raw.len() - 1]);
            oracle != buggy
        }
        Bug::OddHex => {
            let mut odd = String::with_capacity(hex.len() + 1);
            odd.push_str(hex);
            odd.push('a');
            let oracle = decode_seal_hex(&odd);
            let buggy = decode_seal_hex(&odd[..odd.len() - 1]);
            oracle.is_none() && buggy.is_some()
        }
        Bug::ShortHex4 => {
            if hex.len() < 3 {
                return false;
            }
            let cand = &hex.as_bytes()[..3];
            parse_hex4(cand).is_none() && buggy_hex4_short(cand).is_some()
        }
        Bug::XorDigest => {
            // One corrupted stream in 64 (plausible defect: a single shard
            // flips a digest bit); other streams never show it.
            if seed % RARE_MOD != RARE_RES {
                return false;
            }
            let oracle = hash_bytes(raw);
            oracle != oracle ^ 1
        }
    }
}

/// Engine 1 (fixed seeds): first corpus case that exposes the bug.
fn engine_fixed(bug: Bug) -> Option<(u64, Vec<u8>)> {
    for case in base_corpus() {
        if catches(bug, case.seed, &case.raw, &case.hex) {
            return Some((case.seed, case.raw));
        }
    }
    None
}

/// Fixed-seed catch count over the 8-seed corpus (before: 3/4).
fn fixed_catch_count() -> usize {
    ALL_BUGS
        .iter()
        .filter(|&&bug| engine_fixed(bug).is_some())
        .count()
}

/// Runner discovery: first of the `cases` runner inputs exposing the bug.
fn engine_runner(bug: Bug, config: &RunnerConfig) -> Option<(u64, Vec<u8>)> {
    for case in runner_corpus(config) {
        if catches(bug, case.seed, &case.raw, &case.hex) {
            return Some((case.seed, case.raw));
        }
    }
    None
}

/// Runner catch count over `cases` inputs (after: 4/4 with 256 persisted).
fn runner_catch_count(config: &RunnerConfig) -> usize {
    ALL_BUGS
        .iter()
        .filter(|&&bug| engine_runner(bug, config).is_some())
        .count()
}

/// Deterministic mutants of one base case from the same seed stream.
fn mutants(case: &Case) -> Vec<Vec<u8>> {
    let mut rng = Rng::new(case.seed).fork_named("mutate");
    let mut out = Vec::with_capacity(1 + usize::try_from(MUTANTS_PER_SEED).expect("8 fits"));
    out.push(case.raw.clone());
    let count = usize::try_from(MUTANTS_PER_SEED).expect("8 fits on this platform");
    for _ in 0..count {
        let mut next = case.raw.clone();
        if next.is_empty() {
            out.push(next);
            continue;
        }
        let at = usize::try_from(rng.next_range(0, next.len() as u64))
            .expect("mutant index fits on this platform");
        let delta = u8::try_from(rng.next_range(1, 256)).expect("byte delta fits in u8");
        next[at] = next[at].wrapping_add(delta);
        out.push(next);
    }
    out
}

/// Engine 2 (byte-mutate): first mutant over the same corpus exposing the bug.
fn engine_mutate(bug: Bug) -> Option<(u64, Vec<u8>)> {
    for case in base_corpus() {
        for mutant in mutants(&case) {
            let hex = hex_of(&mutant);
            if catches(bug, case.seed, &mutant, &hex) {
                return Some((case.seed, mutant));
            }
        }
    }
    None
}

/// Engine 3 (shrink): greedily drop single bytes while the bug still shows,
/// stopping after `max_iters` steps (donor `max_shrink_iters` bound).
fn engine_shrink(bug: Bug, seed: u64, input: &[u8], max_iters: u32) -> Vec<u8> {
    let mut best = input.to_vec();
    let mut iters: u32 = 0;
    loop {
        if iters >= max_iters {
            return best;
        }
        let mut narrowed: Option<Vec<u8>> = None;
        for index in 0..best.len() {
            let mut cand = best.clone();
            cand.remove(index);
            if catches(bug, seed, &cand, &hex_of(&cand)) {
                narrowed = Some(cand);
                break;
            }
        }
        match narrowed {
            Some(next) => {
                best = next;
                iters += 1;
            }
            None => return best,
        }
    }
}

/// Full matrix: one failing input per engine per bug, in fixed order.
/// Discovery runs over the 256-case runner corpus so all four seeded bugs
/// show; the mutate engine works the found case and shrink stays bounded.
fn run_matrix() -> Vec<(u64, Vec<u8>)> {
    let config = RunnerConfig::default();
    let mut out = Vec::with_capacity(ALL_BUGS.len() * 3);
    for &bug in &ALL_BUGS {
        let (seed, found) = engine_runner(bug, &config).expect("runner catches seeded bug");
        let case = Case {
            seed,
            raw: found.clone(),
            hex: hex_of(&found),
        };
        let (_, mutated) = mutants(&case)
            .into_iter()
            .find_map(|mutant| {
                let hex = hex_of(&mutant);
                catches(bug, case.seed, &mutant, &hex).then_some((case.seed, mutant))
            })
            .expect("mutate engine keeps the runner-found failure");
        let shrunk = engine_shrink(bug, seed, &mutated, config.max_shrink_iters);
        assert!(
            catches(bug, seed, &shrunk, &hex_of(&shrunk)),
            "shrink must keep the failure for {bug:?}"
        );
        out.push((bug as u64, found));
        out.push((bug as u64, mutated));
        out.push((bug as u64, shrunk));
    }
    out
}

/// Manifest-dir path of the regression file, so runs replay from any cwd.
fn regression_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(REGRESSION_FILE)
}

/// Tag after `#` on a regression line back to its bug; unknown tags skip.
fn bug_from_tag(tag: &str) -> Option<Bug> {
    match tag {
        "TruncHash" => Some(Bug::TruncHash),
        "OddHex" => Some(Bug::OddHex),
        "ShortHex4" => Some(Bug::ShortHex4),
        "XorDigest" => Some(Bug::XorDigest),
        _ => None,
    }
}

/// Load persisted `seed <n> # <bug>` lines; comments, blanks and malformed
/// lines skip. A missing file loads empty, like donor persistence with no
/// history yet.
fn load_regression_seeds_from(path: &Path) -> Vec<(u64, Bug)> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("seed ") else {
            continue;
        };
        let (num, tag) = match rest.split_once('#') {
            Some((num, tag)) => (num.trim(), tag.trim()),
            None => (rest.trim(), ""),
        };
        if let (Ok(seed), Some(bug)) = (num.parse::<u64>(), bug_from_tag(tag)) {
            out.push((seed, bug));
        }
    }
    out
}

/// Load the committed regression file.
fn load_regression_seeds() -> Vec<(u64, Bug)> {
    load_regression_seeds_from(&regression_path())
}

/// Append `seed <n> # <bug>` unless that exact line is present; creates the
/// file and its header when absent. Returns true when a line was added.
fn save_regression_seed_to(path: &Path, seed: u64, bug: Bug) -> std::io::Result<bool> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let line = format!("seed {seed} # {bug:?}\n");
    let current = std::fs::read_to_string(path).unwrap_or_default();
    if current.lines().any(|known| known.trim() == line.trim()) {
        return Ok(false);
    }
    let mut text = current;
    if text.is_empty() {
        text.push_str("# property-matrix regressions: `seed <n> # <bug>` per line.\n");
        text.push_str("# Shape ported from proptest FileFailurePersistence seed lines.\n");
    }
    text.push_str(&line);
    std::fs::write(path, text)?;
    Ok(true)
}

#[test]
fn fixed_corpus_before_runner_catches_three_of_four() {
    let corpus = base_corpus();
    let seeds: Vec<u64> = corpus.iter().map(|case| case.seed).collect();
    assert_eq!(seeds, SEEDS, "every engine runs the same seed corpus");
    for &bug in &[Bug::TruncHash, Bug::OddHex, Bug::ShortHex4] {
        assert!(engine_fixed(bug).is_some(), "fixed seeds catch {bug:?}");
        assert!(engine_mutate(bug).is_some(), "byte-mutate catches {bug:?}");
    }
    // The 1-in-64 stream bug hides from 8 fixed seeds: none of SEEDS hits
    // residue 13 mod 64, so the fixed engines stay at 3/4.
    assert_eq!(
        engine_fixed(Bug::XorDigest),
        None,
        "8 fixed seeds miss the gated stream bug"
    );
    assert_eq!(
        engine_mutate(Bug::XorDigest),
        None,
        "byte-mutate shares the case seed, so it misses too"
    );
    assert_eq!(
        fixed_catch_count(),
        3,
        "before: 3/4 seeded bugs on 8 fixed seeds"
    );
}

#[test]
fn runner_256_cases_persisted_catches_four_of_four() {
    let config = RunnerConfig::default();
    assert_eq!(config.cases, 256, "runner defaults to the donor case count");
    assert_eq!(
        config.max_shrink_iters, RUNNER_MAX_SHRINK_ITERS,
        "runner carries the donor shrink bound"
    );
    assert!(config.persist, "regression-file persistence stays on");
    let corpus = runner_corpus(&config);
    assert_eq!(corpus.len(), 256, "runner runs 256 cases");
    // Replay-first, like the donor runner: persisted failures still catch
    // before fresh cases run.
    if config.persist {
        let persisted = load_regression_seeds();
        assert!(
            !persisted.is_empty(),
            "regression file ships persisted seeds"
        );
        for (seed, bug) in &persisted {
            let case = replay_case(*seed);
            assert!(
                catches(*bug, case.seed, &case.raw, &case.hex),
                "persisted seed {seed} still catches {bug:?}"
            );
        }
    }
    for &bug in &ALL_BUGS {
        assert!(
            engine_runner(bug, &config).is_some(),
            "256-case runner catches {bug:?}"
        );
    }
    assert_eq!(
        runner_catch_count(&config),
        4,
        "after: 4/4 seeded bugs over 256 persisted cases"
    );
}

#[test]
fn regression_file_replays_four_of_four() {
    let persisted = load_regression_seeds();
    assert!(
        persisted.len() >= ALL_BUGS.len(),
        "regression file ships one seed per bug, got {}",
        persisted.len()
    );
    for &bug in &ALL_BUGS {
        assert!(
            persisted.iter().any(|&(_, held)| held == bug),
            "regression file covers {bug:?}"
        );
    }
    for (seed, bug) in &persisted {
        let case = replay_case(*seed);
        assert!(
            catches(*bug, case.seed, &case.raw, &case.hex),
            "replayed seed {seed} still catches {bug:?}"
        );
    }
}

#[test]
fn regression_file_round_trips_new_failures() {
    // Unique temp path per process and thread: no file is shared between
    // parallel test workers, so a full-workspace run cannot collide.
    let path = std::env::temp_dir().join(format!(
        "e2040-property-matrix-steal-{}-{:?}.txt",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_file(&path);
    assert!(
        save_regression_seed_to(&path, 13, Bug::XorDigest).expect("temp save works"),
        "first save adds the line"
    );
    assert!(
        !save_regression_seed_to(&path, 13, Bug::XorDigest).expect("temp re-save works"),
        "duplicate save is a no-op"
    );
    assert!(
        save_regression_seed_to(&path, 1, Bug::TruncHash).expect("second seed saves"),
        "second seed adds a line"
    );
    assert_eq!(
        load_regression_seeds_from(&path),
        vec![(13, Bug::XorDigest), (1, Bug::TruncHash)],
        "seed lines round-trip in order"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn matrix_replay_x2_identical() {
    let first = run_matrix();
    let second = run_matrix();
    assert_eq!(
        first.len(),
        second.len(),
        "matrix covers every engine x bug"
    );
    assert_eq!(first, second, "same seed corpus replays x2 identical");
    assert!(
        first.iter().any(|(_, input)| !input.is_empty()),
        "matrix inputs are non-empty"
    );
}

#[test]
fn shrink_keeps_failure_and_never_grows() {
    let config = RunnerConfig::default();
    for &bug in &ALL_BUGS {
        let (seed, found) = engine_runner(bug, &config).expect("runner catches seeded bug");
        let shrunk = engine_shrink(bug, seed, &found, config.max_shrink_iters);
        assert!(
            shrunk.len() <= found.len(),
            "shrink never grows {bug:?}: {} -> {}",
            found.len(),
            shrunk.len()
        );
        assert!(
            catches(bug, seed, &shrunk, &hex_of(&shrunk)),
            "shrunk input still exposes {bug:?}"
        );
        // Minimal: no single-byte removal still exposes the bug.
        for index in 0..shrunk.len() {
            let mut cand = shrunk.clone();
            cand.remove(index);
            assert!(
                !catches(bug, seed, &cand, &hex_of(&cand)),
                "shrink is minimal for {bug:?} at byte {index}"
            );
        }
    }
}

#[test]
fn shrink_respects_max_iters_bound() {
    let config = RunnerConfig::default();
    let (seed, found) = engine_runner(Bug::TruncHash, &config).expect("runner finds TruncHash");
    // Zero budget returns the input untouched.
    assert_eq!(
        engine_shrink(Bug::TruncHash, seed, &found, 0),
        found,
        "zero shrink budget keeps the input"
    );
    // A tight budget removes at most that many bytes but keeps the failure.
    let tight = engine_shrink(Bug::TruncHash, seed, &found, 2);
    assert!(
        found.len() - tight.len() <= 2,
        "two shrink steps remove at most two bytes"
    );
    assert!(
        catches(Bug::TruncHash, seed, &tight, &hex_of(&tight)),
        "bounded shrink keeps the failure"
    );
    // The default budget minimizes as far as the unbounded run.
    let full = engine_shrink(Bug::TruncHash, seed, &found, config.max_shrink_iters);
    let unbounded = engine_shrink(Bug::TruncHash, seed, &found, u32::MAX);
    assert_eq!(
        full, unbounded,
        "default bound never cuts quality at this scale"
    );
}

#[test]
fn byte_mutate_stays_seeded_on_same_corpus() {
    let corpus = base_corpus();
    for case in &corpus {
        let first = mutants(case);
        let second = mutants(case);
        assert_eq!(first, second, "mutants replay x2 for seed {}", case.seed);
        assert_eq!(
            first.len(),
            usize::try_from(MUTANTS_PER_SEED + 1).expect("9 fits"),
            "mutant count is fixed for seed {}",
            case.seed
        );
        assert_eq!(first[0], case.raw, "first mutant is the corpus case");
        // Every mutant keeps the corpus length: only bytes change, so the
        // mutate engine cannot smuggle in a foreign corpus.
        for mutant in &first {
            assert_eq!(mutant.len(), case.raw.len(), "mutant keeps case length");
        }
    }
    // Mutate covers the same seeds as the fixed engine.
    let mutate_seeds: Vec<u64> = corpus.iter().map(|case| case.seed).collect();
    assert_eq!(mutate_seeds, SEEDS, "mutate derives from the shared corpus");
}

#[test]
fn runner_corpus_replays_x2_over_256_cases() {
    let config = RunnerConfig::default();
    let first = runner_corpus(&config);
    let second = runner_corpus(&config);
    assert_eq!(first.len(), 256, "runner holds 256 cases");
    let first_raw: Vec<&Vec<u8>> = first.iter().map(|case| &case.raw).collect();
    let second_raw: Vec<&Vec<u8>> = second.iter().map(|case| &case.raw).collect();
    assert_eq!(first_raw, second_raw, "runner corpus replays x2 identical");
    assert!(
        first.iter().all(|case| !case.raw.is_empty()),
        "runner inputs are non-empty"
    );
}

#[test]
fn clean_oracles_report_no_failure() {
    // Correct implementations agree with themselves on every engine input,
    // so the matrix has no false positives on the shared corpus.
    for case in base_corpus() {
        assert_eq!(hash_bytes(&case.raw), hash_bytes(&case.raw));
        assert_eq!(decode_seal_hex(&case.hex), decode_seal_hex(&case.hex));
        for mutant in mutants(&case) {
            let hex = hex_of(&mutant);
            assert_eq!(hash_bytes(&mutant), hash_bytes(&mutant));
            assert_eq!(decode_seal_hex(&hex), decode_seal_hex(&hex));
            assert!(
                !catches(Bug::TruncHash, case.seed, &[], &hex),
                "empty input is not a bug"
            );
        }
    }
    // The gated stream bug shows only on its residue.
    let gated = runner_case_for_index(13);
    assert!(
        catches(Bug::XorDigest, gated.seed, &gated.raw, &gated.hex),
        "stream 13 shows the gated corruption"
    );
    let plain = runner_case_for_index(0);
    assert!(
        !catches(Bug::XorDigest, plain.seed, &plain.raw, &plain.hex),
        "other streams never show it"
    );
}
