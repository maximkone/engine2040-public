//! Pillars receipt: the shipped sample's AI artefacts leave exact evidence.
//!
//! Consumer: the pillars receipt row and the shipped sample; the report lands
//! in `target/evidence/b/pillars/`. Data owner: this module owns only the
//! pinned digests, the fixed percept tape and the report text. The distilled
//! integer gladiator net loads through `engine_ai::NetPolicy`, runs the tape
//! twice and must give the same fighter-input bytes; the living-world write log
//! records one record per AI-made object and must link digest to digest; the
//! format museum loads, migrates and replays the alpha tape, and the scene
//! fixture when the `scene-museum` tier is built.
//! Determinism scope: integer net math, integer event math and pure codecs, so
//! two runs of this module write identical report bytes.
//! Errors: a failed check fails its test with a [`ReceiptError`] naming the step.

use engine_ai::mind::ActionSource;
use engine_ai::provenance::{PendingRecord, ProvenanceError, ProvenanceLog};
use engine_ai::{Intent, NetPolicy, Percept};
use engine_core::hash_bytes;
use engine_input::{ParticipantInput, PARTICIPANT_ZERO};

use crate::fb_versions::{at, golden_checkpoint, ReceiptError};
use crate::museum::{load_tape_v1, migrate_and_replay};
use crate::{Checkpoint, GOLDEN_600};

/// Shipped distilled gladiator net: six percept lanes in, five intents out.
static NET: &[u8] = include_bytes!("../../../fixtures/nets/depth_student.e2nn");
/// Stored alpha tape fixture the museum loads, migrates and replays.
static TAPE_V1: &[u8] = include_bytes!("../../../fixtures/tape-v1.bin");
/// Stored alpha scene fixture the museum loads and migrates.
#[cfg(feature = "scene-museum")]
static SCENE_V1: &str = include_str!("../../../fixtures/scene-v1.json");

/// FNV-1a 64 of the shipped net bytes; the same value the distill gate pins.
pub const NET_DIGEST: u64 = 0x6ace_b82b_2a85_6f5c;
/// FNV-1a 64 of the 16 fighter-input records the percept tape produces.
pub const POLICY_DIGEST: u64 = 0xdf21_8622_c725_5b5b;
/// FNV-1a 64 of the tape fixture's stored checkpoint pairs.
pub const TAPE_CHECKPOINT_DIGEST: u64 = 0x1a69_b20f_0be6_d0eb;
/// AI-made objects the receipt walks: one living-world write record each.
pub const AI_OBJECTS: usize = 4;
/// Fewest distinct intents the net must pick over the tape, so the replay
/// proves a decision, not a constant output.
pub const MIN_DISTINCT_INTENTS: usize = 3;

/// Fixed percept tape: foe offset `[x, y, z]` in whole metres plus the
/// separation in metres, so every net lane is an exact binary fraction.
const PERCEPTS: [[f32; 4]; 16] = [
    [12.0, 0.0, 0.0, 12.0],
    [10.0, 0.0, 0.0, 10.0],
    [8.0, 0.0, 0.0, 8.0],
    [6.0, 0.0, 0.0, 6.0],
    [4.0, 0.0, 0.0, 4.0],
    [2.0, 0.0, 0.0, 2.0],
    [1.0, 0.0, 0.0, 1.0],
    [0.0, 0.0, 3.0, 3.0],
    [0.0, 0.0, 6.0, 6.0],
    [3.0, 1.0, 3.0, 7.0],
    [-4.0, 0.0, 0.0, 4.0],
    [-2.0, 0.0, 0.0, 2.0],
    [-1.0, 0.0, 0.0, 1.0],
    [0.0, 0.0, 0.0, 0.0],
    [5.0, 1.0, -2.0, 8.0],
    [-6.0, 2.0, 4.0, 12.0],
];

/// Foe-attacking lane per tape row, in tape order.
const FOE_ATTACKING: [bool; 16] = [
    false, false, false, false, false, false, false, false, false, false, false, true, true, false,
    true, true,
];

/// Intent names in the net's output-lane order, for the report mix line.
const INTENT_NAMES: [(&str, Intent); 5] = [
    ("attack", Intent::Attack),
    ("block", Intent::Block),
    ("dodge", Intent::Dodge),
    ("advance", Intent::Advance),
    ("retreat", Intent::Retreat),
];

/// FNV-1a 64 of the shipped net file bytes.
pub fn net_digest() -> u64 {
    hash_bytes(NET)
}

/// Run the fixed percept tape through a freshly loaded net policy.
fn replay_policy() -> Result<(Vec<ParticipantInput>, Vec<Intent>), ReceiptError> {
    let mut policy =
        NetPolicy::load(PARTICIPANT_ZERO, NET).map_err(at("load the gladiator net"))?;
    let mut inputs = Vec::with_capacity(PERCEPTS.len());
    let mut intents = Vec::with_capacity(PERCEPTS.len());
    for (index, (row, attacking)) in PERCEPTS.iter().zip(FOE_ATTACKING.iter()).enumerate() {
        let tick = u32::try_from(index).unwrap_or(u32::MAX);
        let percept = Percept::new(
            [0.0, 0.0, 0.0],
            0.0,
            [row[0], row[1], row[2]],
            *attacking,
            row[3],
            tick,
        );
        inputs.push(policy.decide(&percept));
        intents.push(policy.intent());
    }
    Ok((inputs, intents))
}

/// FNV-1a 64 over the canonical 10-byte participant-input records, in tape
/// order.
fn policy_digest(inputs: &[ParticipantInput]) -> u64 {
    let mut bytes = Vec::with_capacity(inputs.len() * 10);
    for input in inputs {
        input.append_bytes(&mut bytes);
    }
    hash_bytes(&bytes)
}

/// Distinct intents the policy picked over the tape.
fn distinct_intents(intents: &[Intent]) -> usize {
    let mut found = 0;
    for (_, intent) in INTENT_NAMES {
        if intents.contains(&intent) {
            found += 1;
        }
    }
    found
}

/// `name=count` pairs for the intents the policy picked, in lane order.
fn intent_mix(intents: &[Intent]) -> String {
    let mut parts = Vec::new();
    for (name, intent) in INTENT_NAMES {
        let count = intents.iter().filter(|seen| **seen == intent).count();
        if count > 0 {
            parts.push(format!("{name}={count}"));
        }
    }
    parts.join(" ")
}

/// Replay the net policy twice; both runs must give identical input bytes and
/// the digest the shipped net is pinned to.
pub fn net_replay() -> Result<(u64, String), ReceiptError> {
    let (first, intents) = replay_policy()?;
    let (second, again) = replay_policy()?;
    let one = policy_digest(&first);
    let two = policy_digest(&second);
    if first != second || one != two || intents != again {
        return Err(ReceiptError::Invariant(
            "the net policy replay differs across two runs",
        ));
    }
    if one != POLICY_DIGEST {
        return Err(ReceiptError::Invariant("the net policy digest moved"));
    }
    if distinct_intents(&intents) < MIN_DISTINCT_INTENTS {
        return Err(ReceiptError::Invariant(
            "the net policy picks too few distinct intents",
        ));
    }
    if !first.iter().all(|input| input.validate().is_ok()) {
        return Err(ReceiptError::Invariant(
            "a net policy input fails the fighter contract",
        ));
    }
    Ok((one, intent_mix(&intents)))
}

/// One record per AI-made object: a validated provider write, chained to the
/// digest the previous write left behind.
fn ai_write_log() -> Result<ProvenanceLog, ReceiptError> {
    let mut log = ProvenanceLog::new();
    let mut digest = hash_bytes(b"sample start");
    for object in 0..AI_OBJECTS {
        let id = u64::try_from(object).unwrap_or(u64::MAX);
        let after = digest.rotate_left(7) ^ id.wrapping_add(1);
        let detail = format!("ai object {id} authored");
        log.record(PendingRecord::external(
            id,
            ActionSource::Provider,
            id,
            id,
            digest,
            after,
            detail.as_str(),
        ))
        .map_err(at("record an AI write"))?;
        digest = after;
    }
    Ok(log)
}

/// True when the log holds one dense, digest-linked record per AI-made object
/// and survives its own bytes unchanged.
fn chain_complete(log: &ProvenanceLog) -> bool {
    let mut previous = None;
    for (index, record) in log.records().iter().enumerate() {
        if record.seq != u64::try_from(index).unwrap_or(u64::MAX) {
            return false;
        }
        if previous.is_some_and(|before| record.digest_before != before) {
            return false;
        }
        previous = Some(record.digest_after);
    }
    log.len() == AI_OBJECTS
        && ProvenanceLog::from_bytes(&log.bytes()).is_ok_and(|restored| &restored == log)
}

/// A write whose detail is empty is refused and records nothing.
fn write_without_record_refused() -> bool {
    let mut log = ProvenanceLog::new();
    let refused = matches!(
        log.record(PendingRecord::external(
            0,
            ActionSource::Provider,
            0,
            0,
            0,
            0,
            " ",
        )),
        Err(ProvenanceError::EmptyDetail)
    );
    refused && log.is_empty()
}

/// Walk the AI write chain; returns the record count and its checkpoint.
pub fn provenance_chain() -> Result<(usize, u64), ReceiptError> {
    let log = ai_write_log()?;
    if !chain_complete(&log) {
        return Err(ReceiptError::Invariant("the AI write chain is broken"));
    }
    Ok((log.len(), log.checkpoint()))
}

/// FNV-1a 64 over the little-endian checkpoint pairs, in tick order.
fn checkpoint_digest(checkpoints: &[Checkpoint]) -> u64 {
    let mut bytes = Vec::with_capacity(checkpoints.len() * 16);
    for checkpoint in checkpoints {
        bytes.extend_from_slice(&checkpoint.legacy.to_le_bytes());
        bytes.extend_from_slice(&checkpoint.state.to_le_bytes());
    }
    hash_bytes(&bytes)
}

/// Load, migrate and replay the stored alpha tape twice; returns its checkpoint
/// count and pinned checkpoint digest.
pub fn museum_tape() -> Result<(usize, u64), ReceiptError> {
    let tape = load_tape_v1(TAPE_V1).map_err(at("load the museum tape"))?;
    let first = migrate_and_replay(TAPE_V1).map_err(at("replay the museum tape"))?;
    let second = migrate_and_replay(TAPE_V1).map_err(at("replay the museum tape again"))?;
    if first != second {
        return Err(ReceiptError::Invariant("the museum tape replay differs"));
    }
    if usize::try_from(tape.config.ticks).ok() != Some(first.len()) {
        return Err(ReceiptError::Invariant(
            "the museum tape checkpoints differ from its ticks",
        ));
    }
    let digest = checkpoint_digest(&first);
    if digest != TAPE_CHECKPOINT_DIGEST {
        return Err(ReceiptError::Invariant(
            "the museum tape checkpoint digest moved",
        ));
    }
    Ok((first.len(), digest))
}

/// Load and migrate the stored alpha scene twice; returns its node count and
/// the owner's canonical content digest. The digest comes from the parsed
/// hierarchy, so a line-ending change in the fixture text never moves it.
#[cfg(feature = "scene-museum")]
pub fn museum_scene() -> Result<(usize, String), ReceiptError> {
    use engine_scene::codec::Scene;

    let scene = crate::museum::load_scene_v1(SCENE_V1).map_err(at("load the museum scene"))?;
    let first = crate::museum::migrate_scene(SCENE_V1).map_err(at("migrate the museum scene"))?;
    let second =
        crate::museum::migrate_scene(SCENE_V1).map_err(at("migrate the museum scene again"))?;
    if first != second {
        return Err(ReceiptError::Invariant(
            "the museum scene migration differs",
        ));
    }
    if first.len() != scene.nodes.len() {
        return Err(ReceiptError::Invariant(
            "the museum scene hierarchy lost a node",
        ));
    }
    let digest = Scene::encode(&first).digest();
    if digest != Scene::encode(&second).digest() {
        return Err(ReceiptError::Invariant("the museum scene digest differs"));
    }
    Ok((first.len(), digest))
}

/// Build the pillars receipt text. Same fixtures and same owner calls give the
/// same text.
pub fn build_report() -> Result<String, ReceiptError> {
    let golden = golden_checkpoint()?;
    let net = net_digest();
    if net != NET_DIGEST {
        return Err(ReceiptError::Invariant("the shipped net bytes moved"));
    }
    let (policy, mix) = net_replay()?;
    let (records, checkpoint) = provenance_chain()?;
    let (tape, tape_digest) = museum_tape()?;
    let tier = if cfg!(feature = "scene-museum") {
        "contract+scene"
    } else {
        "contract"
    };
    let mut lines = vec![
        "ai pillars receipt".to_string(),
        format!("tier {tier}"),
        format!(
            "golden {golden:016x} {}",
            if golden == GOLDEN_600 { "ok" } else { "CHANGED" }
        ),
        format!(
            "net depth_student.e2nn bytes {} digest {net:016x} ok",
            NET.len()
        ),
        format!(
            "net policy records {} digest {policy:016x} x2 identical",
            PERCEPTS.len()
        ),
        format!("net policy intents {mix}"),
        format!(
            "provenance objects {AI_OBJECTS} records {records} chain complete true checkpoint {checkpoint:016x}"
        ),
        format!(
            "provenance write-without-record refused {}",
            write_without_record_refused()
        ),
        format!("museum tape-v1 checkpoints {tape} digest {tape_digest:016x} replay ok"),
    ];
    #[cfg(feature = "scene-museum")]
    {
        let (nodes, digest) = museum_scene()?;
        lines.push(format!(
            "museum scene-v1 nodes {nodes} digest {digest} migrate ok"
        ));
    }
    #[cfg(not(feature = "scene-museum"))]
    lines.push("museum scene not run (build with --features scene-museum)".to_string());
    Ok(format!("{}\n", lines.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn evidence_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/b/pillars")
    }

    #[test]
    fn net_policy_replays_bit_exactly() {
        assert_eq!(net_digest(), NET_DIGEST);
        let (digest, mix) = net_replay().expect("the net policy replays");
        assert_eq!(digest, POLICY_DIGEST);
        let (_, intents) = replay_policy().expect("the policy replays");
        assert!(distinct_intents(&intents) >= MIN_DISTINCT_INTENTS, "{mix}");
        assert!(mix.contains("advance=") && mix.contains("attack="), "{mix}");
    }

    #[test]
    fn ai_writes_carry_a_complete_chain() {
        let (records, checkpoint) = provenance_chain().expect("the chain is complete");
        assert_eq!(records, AI_OBJECTS);
        let log = ai_write_log().expect("the log records");
        assert_eq!(log.checkpoint(), checkpoint);
        let again = ai_write_log().expect("the log records twice");
        assert_eq!(again.checkpoint(), checkpoint);
        assert!(write_without_record_refused());
        assert!(!chain_complete(&ProvenanceLog::new()));
    }

    #[test]
    fn museum_tape_loads_migrates_and_replays() {
        let (checkpoints, digest) = museum_tape().expect("the alpha tape replays");
        assert_eq!(checkpoints, 24);
        assert_eq!(digest, TAPE_CHECKPOINT_DIGEST);
    }

    #[cfg(feature = "scene-museum")]
    #[test]
    fn museum_scene_loads_migrates_and_replays() {
        let (nodes, digest) = museum_scene().expect("the alpha scene migrates");
        assert_eq!(nodes, 3);
        assert!(!digest.is_empty());
    }

    #[test]
    fn golden_is_unchanged() {
        assert_eq!(golden_checkpoint(), Ok(GOLDEN_600));
    }

    #[test]
    fn report_states_every_pillar() {
        let report = build_report().expect("the receipt builds");
        for needle in [
            "golden",
            "net policy records 16 digest",
            "x2 identical",
            "chain complete true",
            "replay ok",
        ] {
            assert!(report.contains(needle), "{needle} missing from {report}");
        }
    }

    #[test]
    fn report_reproduces_twice() {
        let first = build_report().expect("first report");
        let second = build_report().expect("second report");
        assert_eq!(first, second);
        let prefix = if cfg!(feature = "scene-museum") {
            "report-scene"
        } else {
            "report"
        };
        let dir = evidence_dir();
        fs::create_dir_all(&dir).expect("evidence folder");
        fs::write(dir.join(format!("{prefix}-1.txt")), &first).expect("write run 1");
        fs::write(dir.join(format!("{prefix}-2.txt")), &second).expect("write run 2");
        let one = fs::read(dir.join(format!("{prefix}-1.txt"))).expect("read run 1");
        let two = fs::read(dir.join(format!("{prefix}-2.txt"))).expect("read run 2");
        assert_eq!(one, two);
    }
}
