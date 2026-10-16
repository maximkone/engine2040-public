//! A7 seeded-bug proof: one deterministic bug with replay + cause.
//!
//! Consumer: the A7 gate plus `target/evidence/a7/` (report + tape) this
//! module writes via [`write_evidence`]. Data owner: fixtures live here; the
//! caller owns nothing after the call. Units: fixed ticks, fighter slots,
//! integer hashes. Determinism: the same seed twice gives the identical
//! outcome (agent order fixed, resimulation deterministic, no wall clock).
//! Errors: [`EvidenceError`]; corrupt tapes fail typed, never panic. Budget:
//! one 400-tick agent match plus one 30-keypoint suite run, ms-scale on dev
//! hardware. Tick paths below use integer arithmetic only; the `20.0`
//! half-extent is arena config plumbing, never tick math.

use std::path::PathBuf;

use engine_core::{decode_seal_hex, hash_bytes, SealKeyError};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::agent::{decode_tape, Agent, AgentConfig, BugFile, Caps, Driver, Metric, Policy};
use crate::causal::{attribute_bug, bisect_replay, verify_replay, CausalReport};
use crate::keypoint::{run_suite_with_fault, BUG_SEEDS, KEYPOINT_COUNT};

/// Stable bug id for the report and the A7 evidence line.
pub const SEEDED_BUG_ID: &str = "ev4-seeded-divert-8-1";
/// Gameplay system under test: one real agent probe input is diverted.
pub const SEEDED_SYSTEM: &str = "qa1-divert-probe";
/// Fixed tick of the seeded input.
pub const SEEDED_TICK: u32 = 8;
/// Slot of the seeded input.
pub const SEEDED_SLOT: u8 = 1;
/// Paired keypoint the seeded bug must fail (and nothing else).
pub const SEEDED_KEYPOINT_ID: &str = "kp06-parry-riposte";
/// Keypoint-suite bug modelling the same wrong-window class.
pub const SEEDED_KEYPOINT_BUG: &str = "bug-window-parry-late";
/// First match seed.
pub const BASE_SEED: u64 = 2040;
/// Fixed ticks per agent match.
pub const MATCH_TICKS: u32 = 400;
/// Slots per agent match.
pub const MATCH_SLOTS: u8 = 2;
/// Evidence report file name under `target/evidence/a7/`.
pub const EVIDENCE_REPORT: &str = "report.txt";
/// Evidence tape file name under `target/evidence/a7/`.
pub const EVIDENCE_TAPE: &str = "tape.pta1";
/// Digest manifest beside the A7 evidence files.
pub const EVIDENCE_MANIFEST: &str = "digests.txt";
/// Seal over the manifest bytes; the manifest is never trusted bare.
pub const EVIDENCE_SEAL: &str = "digests.sha";

/// Evidence failures: bounded diagnostics, never panics.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum EvidenceError {
    /// The seeded run filed no desync bug.
    NoDesync,
    /// Causal replay refused the filed tape; reason carried.
    NoAttribution(String),
    /// The keypoint suite did not fail exactly the paired keypoint.
    KeypointMiss(String),
    /// An evidence directory or file write failed; detail carried.
    Io(String),
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

impl core::fmt::Display for EvidenceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoDesync => write!(f, "seeded run filed no desync bug"),
            Self::NoAttribution(reason) => write!(f, "attribution refused: {reason}"),
            Self::KeypointMiss(reason) => write!(f, "keypoint miss: {reason}"),
            Self::Io(reason) => write!(f, "evidence io refused: {reason}"),
            Self::SealKey(reason) => write!(f, "seal key refused: {reason}"),
            Self::DigestMismatch { file, want, got } => write!(
                f,
                "evidence digest mismatch: {file} want {want} got {got}; evidence tampered or truncated"
            ),
        }
    }
}

impl std::error::Error for EvidenceError {}

/// Directory holding the A7 evidence, resolved from the crate location so
/// the proof runs from any working directory.
pub fn evidence_dir() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/a7")
}

/// The seeded agent: brains everywhere except one diverted probe input
/// at ([`SEEDED_TICK`], [`SEEDED_SLOT`]).
pub fn seeded_agent() -> Agent {
    Agent {
        config: AgentConfig {
            base_seed: BASE_SEED,
            ticks: MATCH_TICKS,
            fighters: MATCH_SLOTS,
            half_extent_m: 20.0,
        },
        driver: Driver {
            policy: Policy::Divert {
                tick: SEEDED_TICK,
                fighter: SEEDED_SLOT,
            },
            caps: Caps::mock(),
        },
    }
}

/// Drive the seeded match and return its desync bug file.
pub fn seeded_bugfile() -> Result<BugFile, EvidenceError> {
    let report = seeded_agent().drive(1);
    report
        .bugs
        .into_iter()
        .find(|bug| matches!(bug.metric, Metric::Desync { .. }))
        .ok_or(EvidenceError::NoDesync)
}

/// Attribute the seeded bug file to its causing input.
pub fn attribute_seeded(bug: &BugFile) -> Result<CausalReport, EvidenceError> {
    attribute_bug(bug).map_err(|error| EvidenceError::NoAttribution(format!("{error:?}")))
}

/// Name the first tick where the seeded probe leaves the control chain:
/// decode the filed tape with [`decode_tape`] then bisect control versus
/// probe with [`bisect_replay`]. Units: fixed ticks. Deterministic: the
/// same bug bytes twice give the same tick (deterministic resimulation, no
/// wall clock, no float).
///
/// # Errors
///
/// Returns [`EvidenceError::NoAttribution`] for malformed tape bytes or a
/// clean control/probe pair.
pub fn bisect_seeded(bug: &BugFile) -> Result<u32, EvidenceError> {
    let Some((config, control, probe)) = decode_tape(&bug.tape_bytes) else {
        return Err(EvidenceError::NoAttribution(
            "malformed tape bytes".to_owned(),
        ));
    };
    bisect_replay(&config, &control, &probe)
        .map_err(|error| EvidenceError::NoAttribution(format!("{error:?}")))
}

/// True when the filed tape decodes and its control replays clean against
/// itself. Units: fixed ticks. Deterministic: the same bug bytes twice give
/// the same answer (deterministic resimulation, no wall clock, no float).
/// Malformed bytes read as false, never panic.
pub fn verify_seeded_clean(bug: &BugFile) -> bool {
    let Some((config, control, _)) = decode_tape(&bug.tape_bytes) else {
        return false;
    };
    verify_replay(&config, &control, &control).is_ok()
}

/// One-line attribution naming the system and the causing input tick.
pub fn attribution_line(causal: &CausalReport) -> String {
    let mut line = String::new();
    for cause in causal.causes.iter().take(1) {
        line = format!(
            "{SEEDED_SYSTEM} tick {} fighter {}",
            cause.tick, cause.fighter
        );
    }
    line
}

/// Keyed-seal scheme for `digests.sha`: HMAC-SHA256 over the manifest bytes
/// under the operator key. A seal without this prefix is rejected, never
/// verified, so an unkeyed reseal cannot pass as genuine evidence.
pub const SEAL_SCHEME: &str = "hmac-sha256";
/// Environment holding the operator seal key as hex (64+ chars for 32+
/// bytes). The key lives with the operator, never in the repo; sealing runs
/// at evidence-write time, off the tick, so there is no per-tick budget.
pub const OPERATOR_KEY_ENV: &str = "E2040_EVIDENCE_KEY";
/// Minimum operator key length in bytes; short keys fail loudly at parse.
pub use engine_core::MIN_SEAL_KEY_BYTES;

/// A refused operator key with the remediation attached.
fn seal_key_error(detail: &str) -> EvidenceError {
    EvidenceError::SealKey(format!(
        "{detail}; set {OPERATOR_KEY_ENV} to {MIN_SEAL_KEY_BYTES} or more bytes as hex"
    ))
}

/// The evidence error for a key the shared parser refused.
fn key_refusal(error: SealKeyError) -> EvidenceError {
    seal_key_error(match error {
        SealKeyError::NotHex => "key must be hex",
        SealKeyError::TooShort => "key is too short",
        SealKeyError::Missing => "operator key is missing",
    })
}

/// Parse an operator seal key from hex text: even-length hex decoding to at
/// least [`MIN_SEAL_KEY_BYTES`] bytes. Pure and deterministic; malformed or
/// short keys fail typed [`EvidenceError::SealKey`], never truncate or pad.
pub fn parse_seal_key(text: &str) -> Result<Vec<u8>, EvidenceError> {
    engine_core::parse_seal_key(text).map_err(key_refusal)
}

/// Read the operator seal key from [`OPERATOR_KEY_ENV`]. Missing or malformed
/// keys fail typed [`EvidenceError::SealKey`]: verify fails closed, never
/// open.
pub fn operator_seal_key() -> Result<Vec<u8>, EvidenceError> {
    seal_key_from_env(OPERATOR_KEY_ENV)
}

/// Read an operator seal key from the named environment variable. The split
/// from [`operator_seal_key`] keeps the missing-key branch hermetically
/// testable without mutating the process environment.
fn seal_key_from_env(var: &str) -> Result<Vec<u8>, EvidenceError> {
    engine_core::seal_key_from_env(var).map_err(key_refusal)
}

/// HMAC-SHA256 of `message` under `key` via the `hmac` crate, never
/// hand-rolled. Deterministic: the same key plus the same bytes always tag
/// identically.
fn hmac_tag(key: &[u8], message: &[u8]) -> Result<[u8; 32], EvidenceError> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key)
        .map_err(|_| seal_key_error("key was rejected by the HMAC backend"))?;
    mac.update(message);
    Ok(mac.finalize().into_bytes().into())
}

/// Lowercase hex of a 32-byte tag.
fn tag_hex(tag: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in tag {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Seal manifest bytes with the operator key, rendered as
/// `hmac-sha256:<64 hex>\n`.
fn seal_manifest(manifest: &[u8], key: &[u8]) -> Result<String, EvidenceError> {
    Ok(format!(
        "{SEAL_SCHEME}:{}\n",
        tag_hex(&hmac_tag(key, manifest)?)
    ))
}

/// Check a seal against manifest bytes under the operator key. A seal without
/// the [`SEAL_SCHEME`] prefix (including every legacy unkeyed seal), a
/// malformed tag, or a tag mismatch fails typed
/// [`EvidenceError::DigestMismatch`] naming [`EVIDENCE_SEAL`]; the tag
/// comparison runs inside the `hmac` crate in constant time.
fn check_manifest_seal(manifest: &[u8], seal: &str, key: &[u8]) -> Result<(), EvidenceError> {
    let prefix = format!("{SEAL_SCHEME}:");
    let Some(tag_text) = seal.trim().strip_prefix(prefix.as_str()) else {
        return Err(EvidenceError::DigestMismatch {
            file: EVIDENCE_SEAL.to_owned(),
            want: format!("{SEAL_SCHEME}:<64 hex>"),
            got: "unkeyed-seal".to_owned(),
        });
    };
    let Some(tag) = decode_seal_hex(tag_text).filter(|tag| tag.len() == 32) else {
        return Err(EvidenceError::DigestMismatch {
            file: EVIDENCE_SEAL.to_owned(),
            want: format!("{SEAL_SCHEME}:<64 hex>"),
            got: "malformed-seal".to_owned(),
        });
    };
    let mut mac = Hmac::<Sha256>::new_from_slice(key)
        .map_err(|_| seal_key_error("key was rejected by the HMAC backend"))?;
    mac.update(manifest);
    if mac.verify_slice(&tag).is_err() {
        return Err(EvidenceError::DigestMismatch {
            file: EVIDENCE_SEAL.to_owned(),
            want: "valid-hmac-sha256-tag".to_owned(),
            got: tag_text.to_owned(),
        });
    }
    Ok(())
}

/// FNV-1a over bytes via the engine-core owner; evidence digests fold
/// integers only. Per-file digests stay unkeyed because the keyed seal over
/// the manifest authenticates the whole set at once.
fn fnv_hex(bytes: &[u8]) -> String {
    format!("{:016x}", hash_bytes(bytes))
}

/// Render the deterministic A7 report text.
pub fn render_report(
    bug: &BugFile,
    causal: &CausalReport,
    failed: &[String],
    passed: usize,
    cause_text: &str,
) -> String {
    let mut out = String::new();
    out.push_str("A7 seeded-bug run (EV4): one deterministic gameplay bug, replaying tape, causal attribution\n");
    out.push_str(&format!(
        "bug={SEEDED_BUG_ID} system={SEEDED_SYSTEM} tick={} fighter={}\n",
        SEEDED_TICK, SEEDED_SLOT
    ));
    out.push_str(&format!(
        "keypoint_bug={SEEDED_KEYPOINT_BUG} keypoint={SEEDED_KEYPOINT_ID}\n"
    ));
    let metric = match bug.metric {
        Metric::Desync { expected, observed } => {
            format!("desync expected={expected:016x} observed={observed:016x}")
        }
        _ => "other".to_owned(),
    };
    // Replay status is derived from an actual re-run of the filed tape, never
    // a literal: a broken replay prints MISMATCH and the marker assert in
    // `seeded_bug_fails_exactly_its_keypoint_and_writes_evidence` can fail.
    let replay_status = if bug.replay().contains(bug) {
        "ok"
    } else {
        "MISMATCH"
    };
    out.push_str(&format!(
        "qa1: seed={BASE_SEED} ticks={MATCH_TICKS} fighters={MATCH_SLOTS} first_bad_tick={} divergent_tick={} {} cause=\"{}\" replay={replay_status}\n",
        bug.first_bad_tick, causal.divergent_tick, metric,
        attribution_line(causal),
    ));
    out.push_str(&format!(
        "keypoint: suite={KEYPOINT_COUNT} failed=[{}] passed={passed} cause=\"{cause_text}\" siblings_green={}\n",
        failed.join(","),
        failed == [SEEDED_KEYPOINT_ID.to_owned()],
    ));
    out.push_str(&format!(
        "tape: file={EVIDENCE_TAPE} bytes={} fnv={}\n",
        bug.tape_bytes.len(),
        fnv_hex(&bug.tape_bytes),
    ));
    if let Some((config, control, probe)) = decode_tape(&bug.tape_bytes) {
        out.push_str(&format!(
            "{}\n",
            crate::causal::bisect_line(&config, &control, &probe)
        ));
        out.push_str(&format!(
            "{}\n",
            crate::causal::verify_line(&config, &control, &probe)
        ));
    }
    out.push_str("repro: cargo test --manifest-path .lanes/ev4/Cargo.toml --target-dir target/pool-2 -j 4 --locked -p engine-playtest seeded_bug\n");
    // No golden marker here: the canonical 600-tick golden gate lives in
    // engine-proof; a static copy of its hash would assert nothing.
    out
}

/// Run the full A7 proof and write `report.txt` plus the replaying tape
/// under the default evidence dir, sealed with the operator key (see
/// [`operator_seal_key`]). Returns the four written paths
/// (report, tape, manifest, seal). Idempotent: the same seed always
/// writes the same bytes.
///
/// # Errors
///
/// Returns [`EvidenceError`] for proof, filesystem or seal-key failures.
pub fn write_evidence(
    seal_key: &[u8],
) -> Result<(PathBuf, PathBuf, PathBuf, PathBuf), EvidenceError> {
    write_evidence_in(&evidence_dir(), seal_key)
}

/// Run the full A7 proof and write `report.txt`, the replaying tape, the
/// `digests.txt` manifest covering both, and the `digests.sha` keyed seal
/// over the manifest under `dir`. Returns all four paths in write
/// order. Idempotent: the same seed always writes the same bytes.
///
/// # Errors
///
/// Returns [`EvidenceError`] for proof, filesystem or seal-key failures.
pub fn write_evidence_in(
    dir: &std::path::Path,
    seal_key: &[u8],
) -> Result<(PathBuf, PathBuf, PathBuf, PathBuf), EvidenceError> {
    let bug = seeded_bugfile()?;
    let causal = attribute_seeded(&bug)?;
    if causal.causes.len() != 1
        || (causal.causes[0].tick, causal.causes[0].fighter) != (SEEDED_TICK, SEEDED_SLOT)
    {
        return Err(EvidenceError::NoAttribution(format!(
            "cause is not tick {SEEDED_TICK} fighter {SEEDED_SLOT}: {:?}",
            causal.causes
        )));
    }
    if causal.divergent_tick != bug.first_bad_tick {
        return Err(EvidenceError::NoAttribution(format!(
            "divergent {} != first_bad {}",
            causal.divergent_tick, bug.first_bad_tick
        )));
    }
    let fault = &BUG_SEEDS[0];
    if fault.id != SEEDED_KEYPOINT_BUG || fault.target != SEEDED_KEYPOINT_ID {
        return Err(EvidenceError::KeypointMiss(format!(
            "BUG_SEEDS[0] moved: {} -> {}",
            fault.id, fault.target
        )));
    }
    let suite = run_suite_with_fault(fault)
        .map_err(|error| EvidenceError::KeypointMiss(error.to_string()))?;
    let mut failed: Vec<String> = suite
        .results
        .iter()
        .filter(|result| !result.pass)
        .map(|result| result.id.clone())
        .collect();
    failed.sort();
    if failed != [SEEDED_KEYPOINT_ID.to_owned()] {
        return Err(EvidenceError::KeypointMiss(format!("failed={failed:?}")));
    }
    let target = suite
        .results
        .iter()
        .find(|result| result.id == SEEDED_KEYPOINT_ID)
        .ok_or_else(|| EvidenceError::KeypointMiss("target missing".to_owned()))?;
    if target.causes != [format!("tick {SEEDED_TICK} fighter {SEEDED_SLOT}")] {
        return Err(EvidenceError::KeypointMiss(format!(
            "cause={:?}",
            target.causes
        )));
    }
    let passed = suite.results.iter().filter(|result| result.pass).count();
    let report = render_report(&bug, &causal, &failed, passed, &target.causes.join(","));
    std::fs::create_dir_all(dir).map_err(|error| EvidenceError::Io(error.to_string()))?;
    let report_path = dir.join(EVIDENCE_REPORT);
    let tape_path = dir.join(EVIDENCE_TAPE);
    let manifest_path = dir.join(EVIDENCE_MANIFEST);
    let seal_path = dir.join(EVIDENCE_SEAL);
    std::fs::write(&report_path, report.as_bytes())
        .map_err(|error| EvidenceError::Io(error.to_string()))?;
    std::fs::write(&tape_path, &bug.tape_bytes)
        .map_err(|error| EvidenceError::Io(error.to_string()))?;
    let manifest = format!(
        "file digest\n{EVIDENCE_REPORT} {}\n{EVIDENCE_TAPE} {}\n",
        fnv_hex(report.as_bytes()),
        fnv_hex(&bug.tape_bytes),
    );
    std::fs::write(&manifest_path, manifest.as_bytes())
        .map_err(|error| EvidenceError::Io(error.to_string()))?;
    let seal = seal_manifest(manifest.as_bytes(), seal_key)?;
    std::fs::write(&seal_path, seal.as_bytes())
        .map_err(|error| EvidenceError::Io(error.to_string()))?;
    Ok((report_path, tape_path, manifest_path, seal_path))
}

/// True when the text is a 16-char hex digest.
fn is_digest(text: &str) -> bool {
    text.len() == 16 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Verify evidence under the default evidence dir and operator key; see
/// [`verify_evidence_in`].
///
/// # Errors
///
/// Returns [`EvidenceError`] for unreadable files, digest failures or a
/// refused seal key.
pub fn verify_evidence(seal_key: &[u8]) -> Result<(), EvidenceError> {
    verify_evidence_in(&evidence_dir(), seal_key)
}

/// Verify evidence written by [`write_evidence_in`] under the operator key:
/// the seal must be a valid [`SEAL_SCHEME`] tag over the manifest, the
/// manifest must name both evidence files, and every listed digest must
/// match the file bytes. Any tamper — report, tape, manifest or seal — and
/// any reseal without the operator key fails typed
/// [`EvidenceError::DigestMismatch`]; missing or unreadable files fail typed
/// [`EvidenceError::Io`].
///
/// # Errors
///
/// Returns [`EvidenceError`] for unreadable files, digest failures or a
/// refused seal key.
pub fn verify_evidence_in(dir: &std::path::Path, seal_key: &[u8]) -> Result<(), EvidenceError> {
    let read = |name: &str| {
        std::fs::read(dir.join(name)).map_err(|error| EvidenceError::Io(format!("{name}: {error}")))
    };
    let manifest_bytes = read(EVIDENCE_MANIFEST)?;
    let seal_bytes = read(EVIDENCE_SEAL)?;
    check_manifest_seal(
        &manifest_bytes,
        &String::from_utf8_lossy(&seal_bytes),
        seal_key,
    )?;
    let manifest =
        String::from_utf8(manifest_bytes).map_err(|error| EvidenceError::DigestMismatch {
            file: EVIDENCE_MANIFEST.to_owned(),
            want: "utf8 manifest".to_owned(),
            got: error.to_string(),
        })?;
    let mut lines = manifest.lines();
    let header = lines.next().unwrap_or("");
    if header != "file digest" {
        return Err(EvidenceError::DigestMismatch {
            file: EVIDENCE_MANIFEST.to_owned(),
            want: "file digest".to_owned(),
            got: header.to_owned(),
        });
    }
    let mut seen_report = false;
    let mut seen_tape = false;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (name, want) = line.split_once(' ').unwrap_or((line, ""));
        if !is_digest(want) {
            return Err(EvidenceError::DigestMismatch {
                file: EVIDENCE_MANIFEST.to_owned(),
                want: line.to_owned(),
                got: "malformed-digest".to_owned(),
            });
        }
        let bytes = read(name)?;
        let got = fnv_hex(&bytes);
        if got != want {
            return Err(EvidenceError::DigestMismatch {
                file: name.to_owned(),
                want: want.to_owned(),
                got,
            });
        }
        if name == EVIDENCE_REPORT {
            seen_report = true;
        }
        if name == EVIDENCE_TAPE {
            seen_tape = true;
        }
    }
    if !(seen_report && seen_tape) {
        return Err(EvidenceError::DigestMismatch {
            file: EVIDENCE_MANIFEST.to_owned(),
            want: format!("{EVIDENCE_REPORT} + {EVIDENCE_TAPE} entries"),
            got: "entry-missing".to_owned(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentConfig as QaConfig;
    use crate::agent::Driver as QaDriver;

    #[test]
    fn seeded_bug_files_one_replaying_tape() {
        let bug = seeded_bugfile().expect("seeded run files desync");
        assert!(
            matches!(bug.metric, Metric::Desync { .. }),
            "seeded bug is a desync: {:?}",
            bug.metric
        );
        // The filed tape replays exactly to the same bug.
        let replayed = bug.replay();
        assert!(
            replayed.contains(&bug),
            "replay must reproduce the filed bug"
        );
        // Deterministic: the same seed twice files the identical bug.
        let again = seeded_bugfile().expect("second run files desync");
        assert_eq!(bug, again, "seeded bug must repeat exactly");
        // Sibling without the divert files nothing on a short match.
        let clean = Agent {
            config: QaConfig {
                base_seed: BASE_SEED,
                ticks: 300,
                fighters: MATCH_SLOTS,
                half_extent_m: 20.0,
            },
            driver: QaDriver::mock(),
        };
        assert!(clean.drive(2).bugs.is_empty(), "clean sibling stays green");
        // Garbage bytes never attribute and never panic.
        let garbage = BugFile {
            tape_bytes: vec![0x50, 0x54, 0x41, 0x00, 1, 2, 3],
            first_bad_tick: 0,
            metric: Metric::Stuck { fighter: 0 },
        };
        assert!(attribute_seeded(&garbage).is_err(), "garbage must fail");
    }

    #[test]
    fn seeded_bug_attribution_names_system_and_tick() {
        let bug = seeded_bugfile().expect("seeded run files desync");
        let causal = attribute_seeded(&bug).expect("seeded bug attributes");
        // Exactly the diverted input, nothing else.
        assert_eq!(causal.causes.len(), 1, "one cause: {causal:?}");
        assert_eq!(
            (causal.causes[0].tick, causal.causes[0].fighter),
            (SEEDED_TICK, SEEDED_SLOT),
            "cause is the seeded input: {causal:?}"
        );
        // Divergence cannot predate the seeded tick and matches the watcher.
        assert!(causal.divergent_tick >= SEEDED_TICK);
        assert_eq!(causal.divergent_tick, bug.first_bad_tick);
        assert_ne!(causal.expected, causal.observed);
        // The attribution line names the system and the tick exactly.
        assert_eq!(
            attribution_line(&causal),
            "qa1-divert-probe tick 8 fighter 1"
        );
        // Deterministic: attributing twice gives the identical report.
        assert_eq!(attribute_seeded(&bug), Ok(causal));
    }

    #[test]
    fn seeded_bug_fails_exactly_its_keypoint_and_writes_evidence() {
        let (report_path, tape_path, _, _) =
            write_evidence(TEST_SEAL_KEY).expect("evidence writes");
        verify_evidence(TEST_SEAL_KEY).expect("clean evidence verifies");
        let report = std::fs::read_to_string(&report_path).expect("report reads");
        for marker in [
            SEEDED_BUG_ID,
            SEEDED_SYSTEM,
            SEEDED_KEYPOINT_ID,
            SEEDED_KEYPOINT_BUG,
            "siblings_green=true",
            "replay=ok",
        ] {
            assert!(report.contains(marker), "report names {marker}");
        }
        assert!(
            report.contains("tick 8 fighter 1"),
            "report names the tick: {report}"
        );
        // The written tape is the filed bug's bytes and replays.
        let bug = seeded_bugfile().expect("seeded run files desync");
        let tape = std::fs::read(&tape_path).expect("tape reads");
        assert_eq!(tape, bug.tape_bytes, "tape holds the filed bytes");
        let back = BugFile {
            tape_bytes: tape,
            first_bad_tick: bug.first_bad_tick,
            metric: bug.metric,
        };
        assert!(
            back.replay().contains(&bug),
            "written tape must replay the bug"
        );
    }

    fn temp_a7_dir(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("e2040-ev11-a7-{tag}-{}", std::process::id()))
    }

    /// Fixed operator key for hermetic tests: 32 bytes, distinct from the a6
    /// test key so cross-suite seals never verify by accident.
    const TEST_SEAL_KEY: &[u8] = b"a7-seeded-bug-operator-test-key!!";
    /// A different key standing in for the attacker's: same length, no match.
    const WRONG_SEAL_KEY: &[u8] = b"a7-attacker-key-wrong-not-operator";

    #[test]
    fn clean_regen_passes_verify() {
        let dir = temp_a7_dir("clean");
        let _ = std::fs::remove_dir_all(&dir);
        let (report_path, _, manifest_path, seal_path) =
            write_evidence_in(&dir, TEST_SEAL_KEY).expect("evidence writes");
        assert!(report_path.is_file() && manifest_path.is_file() && seal_path.is_file());
        let seal = std::fs::read_to_string(&seal_path).expect("seal reads");
        assert!(seal.starts_with("hmac-sha256:"), "{seal}");
        verify_evidence_in(&dir, TEST_SEAL_KEY).expect("untampered regen passes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tampered_report_or_tape_fails_verify_loudly() {
        let dir = temp_a7_dir("tamper");
        let _ = std::fs::remove_dir_all(&dir);
        write_evidence_in(&dir, TEST_SEAL_KEY).expect("evidence writes");
        verify_evidence_in(&dir, TEST_SEAL_KEY).expect("clean passes before tamper");
        // Edited report bytes fail typed and name the file.
        let mut report = std::fs::read(dir.join(EVIDENCE_REPORT)).expect("report reads");
        report[0] = report[0].wrapping_add(1);
        std::fs::write(dir.join(EVIDENCE_REPORT), &report).expect("tamper writes");
        match verify_evidence_in(&dir, TEST_SEAL_KEY).expect_err("tampered report must fail") {
            EvidenceError::DigestMismatch { file, .. } => assert_eq!(file, EVIDENCE_REPORT),
            error => panic!("tampered report must fail typed, got {error}"),
        }
        write_evidence_in(&dir, TEST_SEAL_KEY).expect("restore");
        // Edited tape bytes fail the same way.
        let mut tape = std::fs::read(dir.join(EVIDENCE_TAPE)).expect("tape reads");
        let last = tape.len() - 1;
        tape[last] = tape[last].wrapping_add(1);
        std::fs::write(dir.join(EVIDENCE_TAPE), &tape).expect("tamper writes");
        match verify_evidence_in(&dir, TEST_SEAL_KEY).expect_err("tampered tape must fail") {
            EvidenceError::DigestMismatch { file, .. } => assert_eq!(file, EVIDENCE_TAPE),
            error => panic!("tampered tape must fail typed, got {error}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn swapped_manifest_or_seal_fails_verify() {
        let dir = temp_a7_dir("seal");
        let _ = std::fs::remove_dir_all(&dir);
        write_evidence_in(&dir, TEST_SEAL_KEY).expect("evidence writes");
        // Rewriting the manifest without the seal fails on the tag.
        let manifest = std::fs::read_to_string(dir.join(EVIDENCE_MANIFEST)).expect("manifest");
        std::fs::write(
            dir.join(EVIDENCE_MANIFEST),
            manifest.replace('a', "b").as_bytes(),
        )
        .expect("manifest tamper");
        assert!(
            matches!(
                verify_evidence_in(&dir, TEST_SEAL_KEY).expect_err("swapped manifest must fail"),
                EvidenceError::DigestMismatch { .. }
            ),
            "swapped manifest must fail typed"
        );
        write_evidence_in(&dir, TEST_SEAL_KEY).expect("restore");
        // A forged seal fails against the manifest bytes.
        std::fs::write(dir.join(EVIDENCE_SEAL), b"ffffffffffffffff\n").expect("seal tamper");
        assert!(
            matches!(
                verify_evidence_in(&dir, TEST_SEAL_KEY).expect_err("forged seal must fail"),
                EvidenceError::DigestMismatch { .. }
            ),
            "forged seal must fail typed"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resealed_forgery_without_the_key_fails_verify() {
        // The edited-report attack: change the report, refresh the manifest
        // digests, and reseal without the operator key (bare digest, the old
        // format). Verify must reject the unkeyed seal loudly.
        let dir = temp_a7_dir("forgery");
        let _ = std::fs::remove_dir_all(&dir);
        write_evidence_in(&dir, TEST_SEAL_KEY).expect("evidence writes");
        let mut report = std::fs::read(dir.join(EVIDENCE_REPORT)).expect("report reads");
        report[0] = report[0].wrapping_add(1);
        std::fs::write(dir.join(EVIDENCE_REPORT), &report).expect("forgery writes");
        let tape = std::fs::read(dir.join(EVIDENCE_TAPE)).expect("tape reads");
        let manifest = format!(
            "file digest\n{EVIDENCE_REPORT} {}\n{EVIDENCE_TAPE} {}\n",
            fnv_hex(&report),
            fnv_hex(&tape),
        );
        std::fs::write(dir.join(EVIDENCE_MANIFEST), &manifest).expect("manifest reseal");
        std::fs::write(
            dir.join(EVIDENCE_SEAL),
            format!("{}\n", fnv_hex(manifest.as_bytes())),
        )
        .expect("seal reseal");
        match verify_evidence_in(&dir, TEST_SEAL_KEY).expect_err("resealed forgery must fail") {
            EvidenceError::DigestMismatch { file, .. } => assert_eq!(file, EVIDENCE_SEAL),
            error => panic!("resealed forgery must fail typed, got {error}"),
        }
        // Same attack resealed under the wrong key fails on the tag.
        let forged = seal_manifest(manifest.as_bytes(), WRONG_SEAL_KEY).expect("wrong-key seal");
        std::fs::write(dir.join(EVIDENCE_SEAL), forged.as_bytes()).expect("seal rewrite");
        match verify_evidence_in(&dir, TEST_SEAL_KEY).expect_err("wrong-key reseal must fail") {
            EvidenceError::DigestMismatch { file, .. } => assert_eq!(file, EVIDENCE_SEAL),
            error => panic!("wrong-key reseal must fail typed, got {error}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrong_key_rejects_clean_evidence() {
        let dir = temp_a7_dir("wrong-key");
        let _ = std::fs::remove_dir_all(&dir);
        write_evidence_in(&dir, TEST_SEAL_KEY).expect("evidence writes");
        match verify_evidence_in(&dir, WRONG_SEAL_KEY).expect_err("wrong key must fail") {
            EvidenceError::DigestMismatch { file, .. } => assert_eq!(file, EVIDENCE_SEAL),
            error => panic!("wrong key must fail typed, got {error}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_and_malformed_seals_fail_loudly() {
        let manifest = b"file digest\nreport.txt 0000000000000000\n";
        for legacy in ["0000000000000000\n", "ffffffffffffffff\n", ""] {
            match check_manifest_seal(manifest, legacy, TEST_SEAL_KEY)
                .expect_err("legacy seal must fail")
            {
                EvidenceError::DigestMismatch { file, .. } => assert_eq!(file, EVIDENCE_SEAL),
                error => panic!("legacy seal must fail typed, got {error}"),
            }
        }
        for broken in [
            "hmac-sha256:abc\n",
            "hmac-sha256:zz00000000000000000000000000000000000000000000000000000000000000\n",
            "hmac-sha256\n",
        ] {
            match check_manifest_seal(manifest, broken, TEST_SEAL_KEY)
                .expect_err("malformed seal must fail")
            {
                EvidenceError::DigestMismatch { file, want, .. } => {
                    assert_eq!(file, EVIDENCE_SEAL);
                    assert!(want.contains("64 hex"), "{want}");
                }
                error => panic!("malformed seal must fail typed, got {error}"),
            }
        }
        let seal = seal_manifest(manifest, TEST_SEAL_KEY).expect("seal renders");
        check_manifest_seal(manifest, &seal, TEST_SEAL_KEY).expect("seal verifies");
    }

    #[test]
    fn seal_key_parsing_refuses_short_and_malformed_keys() {
        assert_eq!(
            parse_seal_key(&"ab".repeat(32)).expect("64 hex chars parse"),
            vec![0xab; 32]
        );
        let short = "00".repeat(31);
        let nonhex = "zz".repeat(32);
        for bad in [
            "",
            "abc",
            "0123456789abcdef",
            short.as_str(),
            nonhex.as_str(),
        ] {
            match parse_seal_key(bad).expect_err("bad key must fail") {
                EvidenceError::SealKey(reason) => {
                    assert!(reason.contains(OPERATOR_KEY_ENV), "{reason}");
                }
                error => panic!("bad key must fail typed, got {error}"),
            }
        }
    }

    #[test]
    fn missing_operator_key_fails_closed() {
        match seal_key_from_env("E2040_TEST_KEY_ABSENT_a7_ev14").expect_err("missing key must fail")
        {
            EvidenceError::SealKey(reason) => assert!(reason.contains("missing"), "{reason}"),
            error => panic!("missing key must fail typed, got {error}"),
        }
    }
}
