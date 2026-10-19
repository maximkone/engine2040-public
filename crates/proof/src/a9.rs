//! Recorded A9 chaos + fuzz run: HL2 suite plus FZ1 harness, one seed.
//!
//! Consumer: `target/evidence/a9/` (report + seed corpus) and the FINISH-RICH
//! A9 evidence line the orchestrator lands. Data owner: this module owns the
//! run seed, the per-fault outcome table and the corpus digests; `chaos` owns
//! fault injection, `fuzz` owns repro bytes, the fixture owns tape truth.
//! Units: ticks (`u32`), attempt counts (`u32`), digests (`u64`, hex).
//! Determinism scope: fixed [`A9_SEED`] plus per-row forked streams, so the
//! same binary always renders the same report on every machine. Budget: one
//! recorded run (12 faults, 4 proof tapes of 24 ticks, plus 20 arena matches
//! of at most 64 ticks under `fuzz`); offline proof code, never the tick.
//! Errors: [`A9Error`] for record/repro/IO failures when writing evidence.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU32, Ordering},
    OnceLock,
};

use engine_core::hash_bytes;
#[cfg(not(feature = "seal"))]
use engine_core::hash_hex;
#[cfg(feature = "seal")]
use engine_core::seal_key::decode_seal_hex;
use engine_core::seal_key::SealKeyError;
pub use engine_core::seal_key::MIN_SEAL_KEY_BYTES;
use engine_observe::EventCode;
#[cfg(feature = "seal")]
use hmac::{Hmac, Mac};
#[cfg(feature = "seal")]
use sha2::Sha256;

use crate::chaos::{self, LoudReason};
use crate::{record, Config, GOLDEN_600};

/// Recorded-run seed driving both the chaos suite and the fuzz corpus.
pub const A9_SEED: u64 = 9;
/// Ticks per proof-tape corpus case; small so the recorded run stays fast.
pub const A9_PROOF_TICKS: u32 = 24;
/// Proof-corpus seeds; disjoint from the golden seed so the corpus cannot
/// restate the golden while still covering distinct trajectories.
pub const A9_PROOF_SEEDS: [u64; 4] = [7, 19, 42, 999_983];

/// Recorded disposition of one chaos fault.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Disposition {
    /// Recovered with a validated `heal.applied` record on this attempt.
    Healed(u32),
    /// Failing tick quarantined with a `heal.failed` record on this attempt.
    Quarantined(u32),
    /// No fixture-exercisable recovery yet; reported loudly, never silent.
    Unsupported,
    /// The harness itself broke; the recorded run is red while any stands.
    InjectionFailed,
}

impl fmt::Display for Disposition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Healed(attempt) => {
                write!(f, "healed attempt {attempt} (record heal.applied x1)")
            }
            Self::Quarantined(attempt) => {
                write!(f, "quarantined attempt {attempt} (record heal.failed x1)")
            }
            Self::Unsupported => write!(f, "unsupported-loud"),
            Self::InjectionFailed => write!(f, "INJECTION-FAILED"),
        }
    }
}

/// One recorded per-fault outcome, in [`chaos::FAULTS`] order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FaultOutcome {
    /// Stable observe fault code.
    pub code: EventCode,
    /// Owning subsystem (first segment of the code's dotted name).
    pub subsystem: &'static str,
    /// What the recorded run did with the fault.
    pub disposition: Disposition,
}

/// Bounded record attempts: the net row folds a wall-clock resim-p99 bar
/// (`engine-net` WAN soak, 2 ms) into its verdict, so a load-hit sample can
/// report `InjectionFailed` while the simulation underneath is deterministic.
/// Retrying only replaces load-poisoned samples; a simulation divergence fails
/// identically every attempt and stays loud. Attempts are counted in the
/// report, never hidden.
const RECORD_ATTEMPTS: u32 = 3;

/// One recorded run per process: every test and the evidence writer share the
/// same live outcomes, so the suite pays for one chaos run instead of one per
/// test (and concurrent runs cannot poison each other's wall-clock bar).
static RECORDED: OnceLock<Vec<FaultOutcome>> = OnceLock::new();
/// Live `chaos::run` calls the recorded run needed; visible in the report.
static RECORD_CALLS: AtomicU32 = AtomicU32::new(0);

/// The one recorded run: first [`chaos_outcomes`] sample with zero harness
/// failures, else the last sample (tests then fail honestly on its loud row).
/// Shared per process via [`RECORDED`]; concurrent callers block on the one
/// live run instead of stampeding the wall-clock bar together.
#[must_use]
pub fn recorded_outcomes() -> Vec<FaultOutcome> {
    RECORDED
        .get_or_init(|| {
            let mut last = Vec::new();
            for _ in 0..RECORD_ATTEMPTS {
                RECORD_CALLS.fetch_add(1, Ordering::SeqCst);
                let outcomes = chaos_outcomes();
                if outcomes
                    .iter()
                    .all(|o| o.disposition != Disposition::InjectionFailed)
                {
                    return outcomes;
                }
                last = outcomes;
            }
            last
        })
        .clone()
}

/// Evidence-write failure: record, repro or filesystem.
#[derive(Debug)]
pub enum A9Error {
    /// Proof fixture record/encode failure.
    Proof(crate::Error),
    /// Fuzz repro save/load failure.
    #[cfg(feature = "fuzz")]
    Fuzz(crate::fuzz::FuzzError),
    /// Filesystem failure while writing evidence.
    Io(std::io::Error),
    /// The operator seal key is missing, malformed or too short; evidence is
    /// never sealed or verified without it, so a lost key reads as a loud
    /// refusal instead of a clean pass.
    SealKey(String),
    /// An evidence file failed its digest check; tamper fails loudly.
    DigestMismatch {
        /// Evidence file (or digest store) that failed the check.
        file: String,
        /// Stored digest (or expectation) that was not met.
        want: String,
        /// Recomputed digest (or what was found).
        got: String,
    },
}

impl fmt::Display for A9Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Proof(error) => write!(f, "a9 proof: {error}"),
            #[cfg(feature = "fuzz")]
            Self::Fuzz(error) => write!(f, "a9 fuzz: {error}"),
            Self::Io(error) => write!(f, "a9 io: {error}"),
            Self::SealKey(reason) => write!(f, "a9 seal key: {reason}"),
            Self::DigestMismatch { file, want, got } => write!(
                f,
                "a9 digest mismatch: {file} want {want} got {got}; evidence tampered or truncated"
            ),
        }
    }
}

impl std::error::Error for A9Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Proof(error) => Some(error),
            #[cfg(feature = "fuzz")]
            Self::Fuzz(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::SealKey(_) => None,
            Self::DigestMismatch { .. } => None,
        }
    }
}

impl From<std::io::Error> for A9Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Drive the HL2 chaos suite once at [`A9_SEED`] and tabulate every row.
///
/// `chaos::run` only lists a healed fault after observing exactly one
/// `heal.applied` event, and only lists a quarantine after exactly one
/// `heal.failed` event, so membership here already means "with its record".
#[must_use]
pub fn chaos_outcomes() -> Vec<FaultOutcome> {
    let report = chaos::run(A9_SEED);
    chaos::FAULTS
        .iter()
        .map(|row| {
            let disposition = if let Some(attempt) = report.healed_attempts(row.code) {
                Disposition::Healed(attempt)
            } else {
                match report.loud_reason(row.code) {
                    Some(LoudReason::Quarantined { attempts }) => {
                        Disposition::Quarantined(attempts)
                    }
                    Some(LoudReason::InjectionFailed(_)) | None => Disposition::InjectionFailed,
                }
            };
            FaultOutcome {
                code: row.code,
                subsystem: row.subsystem,
                disposition,
            }
        })
        .collect()
}

/// Deterministic proof-corpus bytes: concatenated tape encodings, one per
/// [`A9_PROOF_SEEDS`] entry at [`A9_PROOF_TICKS`] ticks with no inputs.
#[must_use]
pub fn proof_corpus_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    for seed in A9_PROOF_SEEDS {
        // Corpus seeds are fixed and valid; a failure means the fixture broke,
        // which the corpus test reports instead of panicking here.
        if let Ok(tape) = record(
            Config {
                seed,
                ticks: A9_PROOF_TICKS,
            },
            &[],
        ) {
            if let Ok(mut encoded) = tape.encode() {
                bytes.append(&mut encoded);
            }
        }
    }
    bytes
}

/// Digest of [`proof_corpus_bytes`]; run twice to prove byte-identical repro.
#[must_use]
pub fn proof_corpus_digest() -> u64 {
    hash_bytes(&proof_corpus_bytes())
}

/// Arena fuzz corpus digest: every [`A9_FUZZ_SEEDS`] gen tape plus every
/// adversarial script, hashed as ordered `(tick, hash)` checkpoint bytes.
#[cfg(feature = "fuzz")]
#[must_use]
pub fn fuzz_corpus_digest() -> u64 {
    let mut bytes = Vec::new();
    for (config, tape) in fuzz_corpus_cases() {
        bytes.extend_from_slice(&config.seed.to_le_bytes());
        bytes.extend_from_slice(&config.ticks.to_le_bytes());
        bytes.push(config.fighters);
        for checkpoint in engine_arena::run(&config, &tape) {
            bytes.extend_from_slice(&checkpoint.tick.to_le_bytes());
            bytes.extend_from_slice(&checkpoint.hash.to_le_bytes());
        }
    }
    hash_bytes(&bytes)
}

/// Fuzz corpus case count (gen tapes plus adversarial scripts over seeds).
#[cfg(feature = "fuzz")]
#[must_use]
pub fn fuzz_case_count() -> usize {
    fuzz_corpus_cases().len()
}

/// Invariant failures over the whole fuzz corpus; the recorded run is red
/// while any stands.
#[cfg(feature = "fuzz")]
#[must_use]
pub fn fuzz_invariant_failures() -> usize {
    fuzz_corpus_cases()
        .iter()
        .map(|(config, tape)| {
            let checkpoints = engine_arena::run(config, tape);
            crate::fuzz::check_invariants(config, tape, &checkpoints).len()
        })
        .sum()
}

/// Seeds driving the arena fuzz corpus; a subset already covered by the
/// harness's own replay tests.
#[cfg(feature = "fuzz")]
const A9_FUZZ_SEEDS: [u64; 4] = [7, 9, 19, 42];

/// Every fuzz corpus case: one gen tape plus all four adversarial scripts per
/// seed, each paired with the config it runs under.
#[cfg(feature = "fuzz")]
fn fuzz_corpus_cases() -> Vec<(engine_arena::ArenaConfig, engine_arena::BoutTape)> {
    use crate::fuzz::{adversarial, gen_tape, ADVERSARIAL_FIGHTERS, ADVERSARIAL_TICKS};
    let mut cases = Vec::new();
    for seed in A9_FUZZ_SEEDS {
        cases.push((
            engine_arena::ArenaConfig {
                seed,
                ticks: 32,
                fighters: 2,
                half_extent_m: 20.0,
                table_version: 1,
            },
            gen_tape(seed, 32, 2),
        ));
        let arena = engine_arena::ArenaConfig {
            seed,
            ticks: ADVERSARIAL_TICKS,
            fighters: ADVERSARIAL_FIGHTERS,
            half_extent_m: 10.0,
            table_version: 1,
        };
        for tape in adversarial(seed) {
            cases.push((arena, tape));
        }
    }
    cases
}

/// Combined recorded-run digest: proof corpus always, fuzz corpus when built.
#[must_use]
pub fn combined_digest() -> u64 {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&proof_corpus_digest().to_le_bytes());
    #[cfg(feature = "fuzz")]
    bytes.extend_from_slice(&fuzz_corpus_digest().to_le_bytes());
    hash_bytes(&bytes)
}

/// Render the recorded-run report: per-fault outcomes plus each corpus digest
/// computed twice on the same machine. The fault table comes from the one
/// [`recorded_outcomes`] run; every digest below is freshly computed twice.
#[must_use]
pub fn render_report() -> String {
    let outcomes = recorded_outcomes();
    let (mut healed, mut quarantined, mut unsupported, mut broken) = (0, 0, 0, 0);
    let mut faults = String::new();
    for outcome in &outcomes {
        match outcome.disposition {
            Disposition::Healed(_) => healed += 1,
            Disposition::Quarantined(_) => quarantined += 1,
            Disposition::Unsupported => unsupported += 1,
            Disposition::InjectionFailed => broken += 1,
        }
        let name = outcome.code.name().unwrap_or("?");
        faults.push_str(&format!("fault {name} -> {}\n", outcome.disposition));
    }
    let golden = match record(Config::default(), &[]) {
        Ok(tape) => *tape.checkpoints.last().map(|c| &c.legacy).unwrap_or(&0),
        Err(_) => 0,
    };
    let golden_line = if golden == GOLDEN_600 {
        format!("golden {golden:016x}: match\n")
    } else {
        format!("golden {golden:016x}: MISMATCH want {GOLDEN_600:016x}\n")
    };
    let proof_first = proof_corpus_digest();
    let proof_second = proof_corpus_digest();
    let proof_line = format!(
        "proof corpus: {} tapes, digest {proof_first:016x} (run 1) == {proof_second:016x} (run 2): {}\n",
        A9_PROOF_SEEDS.len(),
        if proof_first == proof_second {
            "match"
        } else {
            "MISMATCH"
        }
    );
    #[cfg(feature = "fuzz")]
    let fuzz_block = {
        let fails = fuzz_invariant_failures();
        let first = fuzz_corpus_digest();
        let second = fuzz_corpus_digest();
        format!(
            "fuzz corpus: {} cases, {fails} invariant failures\nfuzz digest {first:016x} (run 1) == {second:016x} (run 2): {}\n",
            fuzz_case_count(),
            if first == second { "match" } else { "MISMATCH" }
        )
    };
    #[cfg(not(feature = "fuzz"))]
    let fuzz_block = "fuzz corpus: harness not built (run with --features fuzz)\n".to_string();
    let combined_first = combined_digest();
    let combined_second = combined_digest();
    let combined_line = format!(
        "combined digest {combined_first:016x} (run 1) == {combined_second:016x} (run 2): {}\n",
        if combined_first == combined_second {
            "match"
        } else {
            "MISMATCH"
        }
    );
    format!(
        "A9 recorded chaos + fuzz run\nseed: {A9_SEED}\n{golden_line}\
         chaos: {} faults (healed {healed}, quarantined {quarantined}, \
         unsupported {unsupported}, injection-failed {broken})\n\
         record calls: {} (bounded retry over load-hit wall-clock bars)\n\
         {faults}{proof_line}{fuzz_block}{combined_line}\
         command: cargo test -p engine-proof --all-features --lib a9\n",
        outcomes.len(),
        RECORD_CALLS.load(Ordering::SeqCst)
    )
}

/// Digest manifest inside the corpus dir; covers the report plus every file.
pub const EVIDENCE_MANIFEST: &str = "digests.txt";
/// Seal over the manifest bytes; the manifest is never trusted bare.
pub const EVIDENCE_SEAL: &str = "digests.sha";
/// Keyed-seal scheme for the seal file: HMAC-SHA256 over the manifest bytes
/// under the operator key (sealed builds). A seal without this prefix is
/// rejected, never verified, so an unkeyed reseal cannot pass as genuine.
pub const SEAL_SCHEME: &str = "hmac-sha256";
/// Environment holding the operator seal key as hex (64+ chars for 32+
/// bytes). The key lives with the operator, never in the repo; sealing runs
/// at evidence-write time, off the tick, so there is no per-tick budget.
pub const OPERATOR_KEY_ENV: &str = "E2040_EVIDENCE_KEY";
/// A refused operator key with the remediation attached (HMAC backend case).
#[cfg(feature = "seal")]
fn seal_key_error(detail: &str) -> A9Error {
    A9Error::SealKey(format!(
        "{detail}; set {OPERATOR_KEY_ENV} to {MIN_SEAL_KEY_BYTES} or more bytes as hex"
    ))
}

/// A refused operator key from the core parser, with the core's wording.
fn seal_key_refused(error: SealKeyError) -> A9Error {
    A9Error::SealKey(engine_core::seal_key::seal_key_reason(
        error,
        OPERATOR_KEY_ENV,
    ))
}

/// Parse an operator seal key from hex text: even-length hex decoding to at
/// least [`MIN_SEAL_KEY_BYTES`] bytes. Delegates to the core owner; malformed
/// or short keys fail typed [`A9Error::SealKey`], never truncate or pad.
pub fn parse_seal_key(text: &str) -> Result<Vec<u8>, A9Error> {
    engine_core::seal_key::parse_seal_key(text).map_err(seal_key_refused)
}

/// Read the operator seal key from [`OPERATOR_KEY_ENV`]. Missing or malformed
/// keys fail typed [`A9Error::SealKey`]: verify fails closed, never open.
pub fn operator_seal_key() -> Result<Vec<u8>, A9Error> {
    seal_key_from_env(OPERATOR_KEY_ENV)
}

/// Read an operator seal key from the named environment variable. The split
/// from [`operator_seal_key`] keeps the missing-key branch hermetically
/// testable without mutating the process environment.
fn seal_key_from_env(var: &str) -> Result<Vec<u8>, A9Error> {
    engine_core::seal_key::seal_key_from_env(var).map_err(seal_key_refused)
}

/// A seal failure naming the seal file with what was wanted and found.
#[cfg(feature = "seal")]
fn seal_mismatch(want: String, got: &str) -> A9Error {
    A9Error::DigestMismatch {
        file: EVIDENCE_SEAL.to_owned(),
        want,
        got: got.to_owned(),
    }
}

/// Seal manifest bytes with the operator key: HMAC-SHA256 from the `hmac`
/// crate (never hand-rolled), rendered as `hmac-sha256:<64 hex>\n`.
/// Deterministic: the same bytes plus the same key always seal identically.
///
/// Available under the `seal` feature so the headless default graph pulls no
/// external package; without it the legacy unkeyed seal below applies and
/// production evidence must use the sealed build.
#[cfg(feature = "seal")]
fn seal_manifest(manifest: &[u8], key: &[u8]) -> Result<String, A9Error> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key)
        .map_err(|_| seal_key_error("key was rejected by the HMAC backend"))?;
    mac.update(manifest);
    let tag = mac.finalize().into_bytes();
    let mut hex = String::with_capacity(64);
    for byte in tag {
        hex.push_str(&format!("{byte:02x}"));
    }
    Ok(format!("{SEAL_SCHEME}:{hex}\n"))
}

/// Legacy unkeyed seal (no `seal` feature): FNV digest of the manifest bytes.
/// Tamper-evident, not forgery-proof; sealed builds reject these seals.
#[cfg(not(feature = "seal"))]
fn seal_manifest(manifest: &[u8], _key: &[u8]) -> Result<String, A9Error> {
    Ok(format!("{}\n", hash_hex(manifest)))
}

/// Check a seal against manifest bytes under the operator key. A seal without
/// the [`SEAL_SCHEME`] prefix (including every legacy unkeyed seal), a
/// malformed tag, or a tag mismatch fails typed
/// [`A9Error::DigestMismatch`] naming [`EVIDENCE_SEAL`]; the tag comparison
/// runs inside the `hmac` crate in constant time.
#[cfg(feature = "seal")]
fn verify_manifest_seal(manifest: &[u8], seal: &str, key: &[u8]) -> Result<(), A9Error> {
    let scheme = format!("{SEAL_SCHEME}:<64 hex>");
    let tag_text = match seal.trim().strip_prefix(SEAL_SCHEME) {
        Some(rest) => match rest.strip_prefix(':') {
            Some(tag) => tag,
            None => return Err(seal_mismatch(scheme, "unkeyed-seal")),
        },
        None => return Err(seal_mismatch(scheme, "unkeyed-seal")),
    };
    let tag = match decode_seal_hex(tag_text) {
        Some(tag) if tag.len() == 32 => tag,
        _ => {
            return Err(seal_mismatch(
                format!("{SEAL_SCHEME}:<64 hex>"),
                "malformed-seal",
            ))
        }
    };
    let mut mac = Hmac::<Sha256>::new_from_slice(key)
        .map_err(|_| seal_key_error("key was rejected by the HMAC backend"))?;
    mac.update(manifest);
    if mac.verify_slice(&tag).is_err() {
        return Err(seal_mismatch("valid-hmac-sha256-tag".to_owned(), tag_text));
    }
    Ok(())
}

/// Legacy unkeyed check (no `seal` feature): the seal must match the FNV
/// digest of the manifest bytes.
#[cfg(not(feature = "seal"))]
fn verify_manifest_seal(manifest: &[u8], seal: &str, _key: &[u8]) -> Result<(), A9Error> {
    let want_seal = seal.trim().to_owned();
    let got_seal = hash_hex(manifest);
    if want_seal != got_seal {
        return Err(A9Error::DigestMismatch {
            file: EVIDENCE_SEAL.to_owned(),
            want: want_seal,
            got: got_seal,
        });
    }
    Ok(())
}
/// Manifest path for the report, relative to the corpus dir.
const REPORT_MANIFEST_PATH: &str = "../report.txt";

/// True when the text is a 16-char hex digest.
fn is_digest(text: &str) -> bool {
    text.len() == 16 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Write the recorded evidence: `report.txt` plus the seed corpus.
///
/// Proof tapes land as `corpus/proof-<seed>-<ticks>.bin` (exact
/// [`crate::Tape::encode`] bytes); under `fuzz`, arena repros land beside
/// them via [`crate::fuzz::save_repro`] (content-hashed names). The
/// `corpus/digests.txt` manifest covers the report plus every corpus file
/// (FNV digests of bytes as-is, never re-rendered), and `corpus/digests.sha`
/// seals the manifest with the operator key (see [`seal_manifest`]; the key
/// arrives explicitly so tests stay hermetic). Returns every file written.
///
/// # Errors
///
/// Returns [`A9Error`] for record/repro failures, filesystem errors or a
/// refused seal key.
pub fn write_evidence(dir: &Path, seal_key: &[u8]) -> Result<Vec<PathBuf>, A9Error> {
    std::fs::create_dir_all(dir)?;
    let corpus = dir.join("corpus");
    std::fs::create_dir_all(&corpus)?;
    let mut written = Vec::new();
    let report_path = dir.join("report.txt");
    let report_text = render_report();
    std::fs::write(&report_path, &report_text)?;
    written.push(report_path);
    let mut manifest = String::from("file digest\n");
    manifest.push_str(&format!(
        "{REPORT_MANIFEST_PATH} {:016x}\n",
        hash_bytes(report_text.as_bytes())
    ));
    for seed in A9_PROOF_SEEDS {
        let tape = record(
            Config {
                seed,
                ticks: A9_PROOF_TICKS,
            },
            &[],
        )
        .map_err(A9Error::Proof)?;
        let bytes = tape.encode().map_err(A9Error::Proof)?;
        let path = corpus.join(format!("proof-{seed:016x}-{A9_PROOF_TICKS}.bin"));
        std::fs::write(&path, &bytes)?;
        manifest.push_str(&format!(
            "{} {:016x}\n",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("?"),
            hash_bytes(&bytes)
        ));
        written.push(path);
    }
    #[cfg(feature = "fuzz")]
    for (config, tape) in fuzz_corpus_cases() {
        let path = crate::fuzz::save_repro(&corpus, &config, &tape).map_err(A9Error::Fuzz)?;
        let bytes = std::fs::read(&path)?;
        manifest.push_str(&format!(
            "{} {:016x}\n",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("?"),
            hash_bytes(&bytes)
        ));
        written.push(path);
    }
    let manifest_path = corpus.join(EVIDENCE_MANIFEST);
    std::fs::write(&manifest_path, &manifest)?;
    written.push(manifest_path);
    let seal = seal_manifest(manifest.as_bytes(), seal_key)?;
    let seal_path = corpus.join(EVIDENCE_SEAL);
    std::fs::write(&seal_path, &seal)?;
    written.push(seal_path);
    Ok(written)
}

/// Verify evidence written by [`write_evidence`] under the operator key: the
/// seal must authenticate the manifest (keyed tag in sealed builds, legacy
/// digest otherwise), the manifest must name the report plus every proof
/// tape, and every listed digest must match the file bytes. Any tamper —
/// report, corpus file, manifest or seal — fails typed
/// [`A9Error::DigestMismatch`]; missing files fail [`A9Error::Io`]. In
/// sealed builds a reseal without the operator key fails on the seal itself.
///
/// # Errors
///
/// Returns [`A9Error`] for unreadable files, digest failures or a refused
/// seal key.
pub fn verify_evidence(dir: &Path, seal_key: &[u8]) -> Result<(), A9Error> {
    let corpus = dir.join("corpus");
    let read_named = |path: PathBuf, name: &str| {
        std::fs::read(&path).map_err(|error| {
            A9Error::Io(std::io::Error::new(
                error.kind(),
                format!("{name}: {error}"),
            ))
        })
    };
    let manifest_path = corpus.join(EVIDENCE_MANIFEST);
    let seal_path = corpus.join(EVIDENCE_SEAL);
    let manifest_bytes = read_named(manifest_path, EVIDENCE_MANIFEST)?;
    let seal_bytes = read_named(seal_path, EVIDENCE_SEAL)?;
    verify_manifest_seal(
        &manifest_bytes,
        &String::from_utf8_lossy(&seal_bytes),
        seal_key,
    )?;
    let manifest = String::from_utf8(manifest_bytes).map_err(|error| A9Error::DigestMismatch {
        file: EVIDENCE_MANIFEST.to_owned(),
        want: "utf8 manifest".to_owned(),
        got: error.to_string(),
    })?;
    let mut lines = manifest.lines();
    let header = lines.next().unwrap_or("");
    if header != "file digest" {
        return Err(A9Error::DigestMismatch {
            file: EVIDENCE_MANIFEST.to_owned(),
            want: "file digest".to_owned(),
            got: header.to_owned(),
        });
    }
    let mut seen_report = false;
    let mut seen_proofs = [false; A9_PROOF_SEEDS.len()];
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (name, want) = line.split_once(' ').unwrap_or((line, ""));
        if !is_digest(want) {
            return Err(A9Error::DigestMismatch {
                file: EVIDENCE_MANIFEST.to_owned(),
                want: line.to_owned(),
                got: "malformed-digest".to_owned(),
            });
        }
        let bytes = read_named(corpus.join(name), name)?;
        let got = format!("{:016x}", hash_bytes(&bytes));
        if got != want {
            return Err(A9Error::DigestMismatch {
                file: name.to_owned(),
                want: want.to_owned(),
                got,
            });
        }
        if name == REPORT_MANIFEST_PATH {
            seen_report = true;
        }
        for (index, seed) in A9_PROOF_SEEDS.iter().enumerate() {
            if name == format!("proof-{seed:016x}-{A9_PROOF_TICKS}.bin") {
                seen_proofs[index] = true;
            }
        }
    }
    if !seen_report || seen_proofs.iter().any(|seen| !seen) {
        return Err(A9Error::DigestMismatch {
            file: EVIDENCE_MANIFEST.to_owned(),
            want: format!(
                "{REPORT_MANIFEST_PATH} + {} proof tapes",
                A9_PROOF_SEEDS.len()
            ),
            got: "entry-missing".to_owned(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replay;

    #[test]
    fn golden_hash_untouched() {
        let tape = record(Config::default(), &[]).expect("default fixture records");
        assert_eq!(tape.checkpoints.last().unwrap().legacy, GOLDEN_600);
        replay(&tape).expect("golden tape replays");
    }

    #[test]
    fn every_fault_heals_quarantines_or_reports_loud_never_silent() {
        let outcomes = recorded_outcomes();
        assert_eq!(outcomes.len(), chaos::FAULTS.len());
        // Report order follows the fault table, one outcome per row.
        for (outcome, row) in outcomes.iter().zip(chaos::FAULTS.iter()) {
            assert_eq!(outcome.code, row.code);
            assert_eq!(outcome.subsystem, row.subsystem);
        }
        // Shipped recoveries: three heals on the first attempt (net AI
        // takeover, assets fallback drop, save retry) plus nine quarantines
        // on the first attempt: the physics NaN tick (no authority snapshot
        // to resimulate from) and eight unshipped-subsystem rows whose real
        // recovery needs an arena match, a GPU, a provider adapter, the
        // command service or a child process. Each quarantines with a
        // `heal.failed` record naming its shipping row: no unsupported
        // carve-out, no silent pass.
        let healed: Vec<EventCode> = outcomes
            .iter()
            .filter(|o| matches!(o.disposition, Disposition::Healed(_)))
            .map(|o| o.code)
            .collect();
        assert_eq!(
            healed,
            vec![
                EventCode::NET_CLIENT_DESYNC,
                EventCode::ASSETS_MISSING,
                EventCode::SAVE_CORRUPT,
            ]
        );
        for outcome in &outcomes {
            if healed.contains(&outcome.code) {
                assert_eq!(outcome.disposition, Disposition::Healed(1));
            }
        }
        let quarantined: Vec<EventCode> = outcomes
            .iter()
            .filter(|o| matches!(o.disposition, Disposition::Quarantined(_)))
            .map(|o| o.code)
            .collect();
        assert_eq!(
            quarantined,
            vec![
                EventCode::PHYSICS_BODY_NONFINITE,
                EventCode::BRAIN_FIGHTER_STUCK,
                EventCode::RENDER_DEVICE_LOST,
                EventCode::RENDER_PIPELINE_FAILED,
                EventCode::PROVIDER_TIMEOUT,
                EventCode::PROVIDER_MALFORMED,
                EventCode::COMMANDS_STALE,
                EventCode::EDITOR_CRASH,
                EventCode::RENDER_FRAME_OVER_BUDGET,
            ]
        );
        for outcome in &outcomes {
            if quarantined.contains(&outcome.code) {
                assert_eq!(outcome.disposition, Disposition::Quarantined(1));
            }
        }
        // Zero loud-unsupported across all twelve rows; the harness itself
        // never breaks silently.
        let mut unsupported = 0;
        for outcome in &outcomes {
            if outcome.disposition == Disposition::Unsupported {
                unsupported += 1;
            }
            assert_ne!(outcome.disposition, Disposition::InjectionFailed);
        }
        assert_eq!(unsupported, 0);
    }

    #[test]
    fn heal_and_quarantine_records_are_validated() {
        // `chaos_outcomes` only reports a heal after the registry emitted
        // exactly one `heal.applied` (and a quarantine after `heal.failed`),
        // so the rendered report must name both records and no silent pass.
        let report = render_report();
        assert!(report.contains("heal.applied x1"));
        assert!(report.contains("heal.failed x1"));
        assert!(report.contains("injection-failed 0"));
        // A registry regression that healed the NaN tick without a snapshot,
        // or dropped a heal record, changes these lines.
        assert!(report.contains("physics.body.nonfinite -> quarantined"));
        assert!(report.contains("net.client.desync -> healed"));
        // Unshipped-subsystem quarantines carry the same receipt shape.
        assert!(report.contains("brain.fighter.stuck -> quarantined"));
        assert!(report.contains("unsupported 0"));
    }

    #[test]
    fn recorded_report_is_stable_and_digests_match_twice() {
        // The fault table is the one recorded run (shared per process, so no
        // test stampedes the wall-clock bar); suite determinism itself is
        // HL2's `same_seed_twice_gives_same_report`. What must reproduce here
        // is re-rendered byte-identically, with every digest freshly computed
        // twice on the same machine.
        assert_eq!(render_report(), render_report());
        assert_eq!(proof_corpus_digest(), proof_corpus_digest());
        assert_eq!(combined_digest(), combined_digest());
        assert!(!proof_corpus_bytes().is_empty());
    }

    #[test]
    fn proof_corpus_reproduces_byte_identically() {
        let first = proof_corpus_bytes();
        let second = proof_corpus_bytes();
        assert_eq!(first, second);
        // Every corpus tape decodes and replays: a corrupt corpus fails here,
        // not downstream in the evidence consumer.
        let mut offset = 0;
        for seed in A9_PROOF_SEEDS {
            let tape = record(
                Config {
                    seed,
                    ticks: A9_PROOF_TICKS,
                },
                &[],
            )
            .expect("corpus seed records");
            let encoded = tape.encode().expect("corpus tape encodes");
            assert_eq!(&first[offset..offset + encoded.len()], encoded.as_slice());
            offset += encoded.len();
            let decoded = crate::Tape::decode(&encoded).expect("corpus tape decodes");
            replay(&decoded).expect("corpus tape replays");
        }
        assert_eq!(offset, first.len());
    }

    #[cfg(feature = "fuzz")]
    #[test]
    fn fuzz_corpus_runs_clean_and_reproduces() {
        use crate::fuzz::{adversarial, check_invariants, gen_tape, load_repro, save_repro};
        assert_eq!(fuzz_case_count(), A9_FUZZ_SEEDS.len() * 5);
        for (config, tape) in fuzz_corpus_cases() {
            let first = engine_arena::run(&config, &tape);
            let second = engine_arena::run(&config, &tape);
            assert_eq!(first, second);
            assert_eq!(first.len(), config.ticks as usize);
            assert!(
                check_invariants(&config, &tape, &first).is_empty(),
                "seed {:016x}",
                config.seed
            );
        }
        assert_eq!(fuzz_invariant_failures(), 0);
        assert_eq!(fuzz_corpus_digest(), fuzz_corpus_digest());
        // A mismatched baseline must trip the hash-responds rule, or the rule
        // is not watching: adversarial script zero against an empty run.
        let config = engine_arena::ArenaConfig {
            seed: 13,
            ticks: crate::fuzz::ADVERSARIAL_TICKS,
            fighters: crate::fuzz::ADVERSARIAL_FIGHTERS,
            half_extent_m: 10.0,
            table_version: 1,
        };
        let baseline = engine_arena::run(&config, &engine_arena::BoutTape::default());
        let fails = check_invariants(&config, &adversarial(13)[0], &baseline);
        assert!(fails.iter().any(|f| f.tick == 0));
        // Repro files round-trip byte-identically: same case, same path and
        // same bytes on re-save, and the reload replays to the same hash.
        let dir = std::env::temp_dir().join(format!("e2040-a9-{}", std::process::id()));
        let config = engine_arena::ArenaConfig {
            seed: 21,
            ticks: 24,
            fighters: 2,
            half_extent_m: 20.0,
            table_version: 1,
        };
        let tape = gen_tape(21, 24, 2);
        let path = save_repro(&dir, &config, &tape).expect("repro saves");
        let bytes = std::fs::read(&path).expect("repro reads");
        assert_eq!(save_repro(&dir, &config, &tape).expect("re-save"), path);
        assert_eq!(std::fs::read(&path).expect("re-read"), bytes);
        let (back_config, back_tape) = load_repro(&path).expect("repro loads");
        assert_eq!((config, tape), (back_config, back_tape.clone()));
        assert_eq!(
            engine_arena::run(&back_config, &back_tape),
            engine_arena::run(&config, &back_tape)
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn evidence_writer_roundtrip() {
        let dir = std::env::temp_dir().join(format!("e2040-a9-ev-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let written = write_evidence(&dir, TEST_SEAL_KEY).expect("evidence writes");
        verify_evidence(&dir, TEST_SEAL_KEY).expect("clean evidence verifies");
        let report = std::fs::read_to_string(dir.join("report.txt")).expect("report reads");
        assert_eq!(report, render_report());
        assert!(report.contains("seed: 9"));
        assert!(report.contains("injection-failed 0"));
        assert!(written.contains(&dir.join("report.txt")));
        // Proof corpus files decode and replay; the manifest names each one.
        let manifest =
            std::fs::read_to_string(dir.join("corpus").join("digests.txt")).expect("manifest");
        assert!(manifest.contains("../report.txt"), "{manifest}");
        for seed in A9_PROOF_SEEDS {
            let name = format!("proof-{seed:016x}-{A9_PROOF_TICKS}.bin");
            assert!(manifest.contains(&name), "{manifest}");
            let bytes = std::fs::read(dir.join("corpus").join(&name)).expect("tape bin");
            let tape = crate::Tape::decode(&bytes).expect("bin decodes");
            replay(&tape).expect("bin replays");
        }
        #[cfg(feature = "fuzz")]
        {
            use crate::fuzz::load_repro;
            let mut repros = 0;
            for path in &written {
                if path.extension().and_then(|e| e.to_str()) == Some("bin")
                    && !path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .starts_with("proof-")
                {
                    let (config, tape) = load_repro(path).expect("repro loads");
                    let checkpoints = engine_arena::run(&config, &tape);
                    assert!(crate::fuzz::check_invariants(&config, &tape, &checkpoints).is_empty());
                    repros += 1;
                }
            }
            assert_eq!(repros, fuzz_case_count());
        }
        // Re-emit beside it: byte-identical corpus, same report.
        let again = dir.join("again");
        write_evidence(&again, TEST_SEAL_KEY).expect("second emit");
        verify_evidence(&again, TEST_SEAL_KEY).expect("second emit verifies");
        assert_eq!(
            std::fs::read_to_string(again.join("report.txt")).expect("report 2"),
            report
        );
        for seed in A9_PROOF_SEEDS {
            let name = format!("proof-{seed:016x}-{A9_PROOF_TICKS}.bin");
            assert_eq!(
                std::fs::read(again.join("corpus").join(&name)).expect("bin 2"),
                std::fs::read(dir.join("corpus").join(&name)).expect("bin 1")
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
        // Opt-in lane emission for the recorded evidence run: the packet's
        // acceptance runs without this variable, so CI stays hermetic while
        // the lane run lands real files under target/evidence/a9/.
        if std::env::var("E2040_A9_EMIT").is_ok() {
            let lane = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/a9");
            let key = operator_seal_key().expect("lane emission needs E2040_EVIDENCE_KEY");
            write_evidence(&lane, &key).expect("lane evidence writes");
        }
    }

    fn temp_a9_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("e2040-ev11-a9-{tag}-{}", std::process::id()))
    }

    /// Fixed operator key for hermetic tests: 32 bytes, distinct from the
    /// playtest keys so cross-suite seals never verify by accident.
    const TEST_SEAL_KEY: &[u8] = b"a9-chaos-fuzz-operator-test-key!!!";
    /// A different key standing in for the attacker's (sealed builds only).
    #[cfg(feature = "seal")]
    const WRONG_SEAL_KEY: &[u8] = b"a9-attacker-key-wrong-not-operate";

    fn mismatch_file(error: &A9Error) -> &str {
        match error {
            A9Error::DigestMismatch { file, .. } => file,
            other => panic!("tamper must fail typed, got {other}"),
        }
    }

    #[test]
    fn clean_regen_passes_verify() {
        let dir = temp_a9_dir("clean");
        let _ = std::fs::remove_dir_all(&dir);
        write_evidence(&dir, TEST_SEAL_KEY).expect("evidence writes");
        verify_evidence(&dir, TEST_SEAL_KEY).expect("untampered regen passes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tampered_report_fails_verify_loudly() {
        let dir = temp_a9_dir("tamper-report");
        let _ = std::fs::remove_dir_all(&dir);
        write_evidence(&dir, TEST_SEAL_KEY).expect("evidence writes");
        verify_evidence(&dir, TEST_SEAL_KEY).expect("clean passes before tamper");
        // Edited a6/a7-style report bytes fail typed and name the report.
        let mut report = std::fs::read(dir.join("report.txt")).expect("report reads");
        let last = report.len() - 1;
        report[last] = report[last].wrapping_add(1);
        std::fs::write(dir.join("report.txt"), &report).expect("tamper writes");
        let error = verify_evidence(&dir, TEST_SEAL_KEY).expect_err("tampered report must fail");
        assert_eq!(mismatch_file(&error), "../report.txt");
        assert!(error.to_string().contains("digest mismatch"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tampered_corpus_manifest_or_seal_fails_verify() {
        let dir = temp_a9_dir("tamper-seal");
        let _ = std::fs::remove_dir_all(&dir);
        write_evidence(&dir, TEST_SEAL_KEY).expect("evidence writes");
        // A swapped proof tape fails even though the report is untouched.
        let name = format!("proof-{:016x}-{A9_PROOF_TICKS}.bin", A9_PROOF_SEEDS[0]);
        let mut tape = std::fs::read(dir.join("corpus").join(&name)).expect("tape reads");
        tape[0] = tape[0].wrapping_add(1);
        std::fs::write(dir.join("corpus").join(&name), &tape).expect("tamper writes");
        let error = verify_evidence(&dir, TEST_SEAL_KEY).expect_err("tampered tape must fail");
        assert_eq!(mismatch_file(&error), name);
        write_evidence(&dir, TEST_SEAL_KEY).expect("restore");
        // The manifest cannot be silently shrunk: the seal pins its bytes.
        let manifest = std::fs::read_to_string(dir.join("corpus").join(EVIDENCE_MANIFEST))
            .expect("manifest reads");
        let shrunk: String = manifest
            .lines()
            .filter(|line| !line.starts_with(REPORT_MANIFEST_PATH))
            .map(|line| format!("{line}\n"))
            .collect();
        std::fs::write(dir.join("corpus").join(EVIDENCE_MANIFEST), &shrunk)
            .expect("manifest tamper");
        assert!(
            matches!(
                verify_evidence(&dir, TEST_SEAL_KEY).expect_err("shrunk manifest must fail"),
                A9Error::DigestMismatch { .. }
            ),
            "shrunk manifest must fail typed"
        );
        write_evidence(&dir, TEST_SEAL_KEY).expect("restore");
        // A forged seal fails against the manifest bytes.
        std::fs::write(
            dir.join("corpus").join(EVIDENCE_SEAL),
            b"ffffffffffffffff\n",
        )
        .expect("seal tamper");
        assert!(
            matches!(
                verify_evidence(&dir, TEST_SEAL_KEY).expect_err("forged seal must fail"),
                A9Error::DigestMismatch { .. }
            ),
            "forged seal must fail typed"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The edited-report attack against sealed evidence: change the report,
    /// refresh the manifest digests, and reseal. Without the operator key the
    /// attacker can only write an unkeyed seal, and under a wrong key only a
    /// mismatched tag; both fail typed on the seal file itself.
    #[cfg(feature = "seal")]
    #[test]
    fn resealed_forgery_without_the_key_fails_verify() {
        let dir = temp_a9_dir("forgery");
        let _ = std::fs::remove_dir_all(&dir);
        write_evidence(&dir, TEST_SEAL_KEY).expect("evidence writes");
        verify_evidence(&dir, TEST_SEAL_KEY).expect("clean passes before forgery");
        let mut report = std::fs::read(dir.join("report.txt")).expect("report reads");
        let last = report.len() - 1;
        report[last] = report[last].wrapping_add(1);
        std::fs::write(dir.join("report.txt"), &report).expect("forgery writes");
        let manifest = std::fs::read_to_string(dir.join("corpus").join(EVIDENCE_MANIFEST))
            .expect("manifest reads");
        let resealed: String = manifest
            .lines()
            .map(|line| {
                if line.starts_with(REPORT_MANIFEST_PATH) {
                    format!("{REPORT_MANIFEST_PATH} {:016x}\n", hash_bytes(&report))
                } else {
                    format!("{line}\n")
                }
            })
            .collect();
        std::fs::write(dir.join("corpus").join(EVIDENCE_MANIFEST), &resealed)
            .expect("manifest reseal");
        // Unkeyed reseal in the legacy format: rejected as unkeyed.
        std::fs::write(
            dir.join("corpus").join(EVIDENCE_SEAL),
            format!("{:016x}\n", hash_bytes(resealed.as_bytes())),
        )
        .expect("seal reseal");
        let error = verify_evidence(&dir, TEST_SEAL_KEY).expect_err("resealed forgery must fail");
        assert_eq!(mismatch_file(&error), EVIDENCE_SEAL);
        assert!(error.to_string().contains("digest mismatch"), "{error}");
        // Same manifest resealed under the wrong key: rejected on the tag.
        let forged = seal_manifest(resealed.as_bytes(), WRONG_SEAL_KEY).expect("wrong-key seal");
        std::fs::write(dir.join("corpus").join(EVIDENCE_SEAL), forged.as_bytes())
            .expect("seal rewrite");
        let error = verify_evidence(&dir, TEST_SEAL_KEY).expect_err("wrong-key reseal must fail");
        assert_eq!(mismatch_file(&error), EVIDENCE_SEAL);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(feature = "seal")]
    #[test]
    fn wrong_key_rejects_clean_evidence() {
        let dir = temp_a9_dir("wrong-key");
        let _ = std::fs::remove_dir_all(&dir);
        write_evidence(&dir, TEST_SEAL_KEY).expect("evidence writes");
        let error = verify_evidence(&dir, WRONG_SEAL_KEY).expect_err("wrong key must fail");
        assert_eq!(mismatch_file(&error), EVIDENCE_SEAL);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(feature = "seal")]
    #[test]
    fn legacy_and_malformed_seals_fail_loudly() {
        let manifest = b"file digest\n../report.txt 0000000000000000\n";
        for legacy in ["0000000000000000\n", "ffffffffffffffff\n", ""] {
            let error = verify_manifest_seal(manifest, legacy, TEST_SEAL_KEY)
                .expect_err("legacy seal must fail");
            assert_eq!(mismatch_file(&error), EVIDENCE_SEAL);
        }
        for broken in ["hmac-sha256:abc\n", "hmac-sha256\n"] {
            let error = verify_manifest_seal(manifest, broken, TEST_SEAL_KEY)
                .expect_err("malformed seal must fail");
            assert_eq!(mismatch_file(&error), EVIDENCE_SEAL);
        }
        let seal = seal_manifest(manifest, TEST_SEAL_KEY).expect("seal renders");
        assert!(seal.starts_with("hmac-sha256:"), "{seal}");
        verify_manifest_seal(manifest, &seal, TEST_SEAL_KEY).expect("seal verifies");
    }

    #[test]
    fn seal_key_parsing_refuses_short_and_malformed_keys() {
        assert_eq!(
            parse_seal_key(&"ab".repeat(32)).expect("64 hex chars parse"),
            vec![0xab; 32]
        );
        let short = "00".repeat(31);
        let nonhex = "zz".repeat(32);
        for bad in ["", "abc", short.as_str(), nonhex.as_str()] {
            match parse_seal_key(bad).expect_err("bad key must fail") {
                A9Error::SealKey(reason) => {
                    assert!(reason.contains(OPERATOR_KEY_ENV), "{reason}");
                }
                error => panic!("bad key must fail typed, got {error}"),
            }
        }
    }

    #[test]
    fn missing_operator_key_fails_closed() {
        match seal_key_from_env("E2040_TEST_KEY_ABSENT_a9_ev14").expect_err("missing key must fail")
        {
            A9Error::SealKey(reason) => assert!(reason.contains("missing"), "{reason}"),
            error => panic!("missing key must fail typed, got {error}"),
        }
    }
}
