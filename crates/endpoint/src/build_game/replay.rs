use super::*;

// AB9 offline brief replay glue: every AB stage records its inputs/outputs in
// one shared per-stage format so the full chain replays providers-off. Owner
// is `engine-endpoint`; consumer is the endpoint offline replay path (plus the
// replay tests at the file end): `record_chain` stamps one `StageRecord` per
// AB stage from a packaged slice, `replay_chain_offline` rebuilds from the
// recorded brief inputs with no model call and requires byte-identical
// records. Units: brief chars, seeds, FNV-1a digests. Budget: four stages per
// brief, O(n) folds over the checkpoint tips, no per-tick work. Errors are
// typed `GameError`: `missing_record` when a stage record is absent (fails
// loudly instead of calling a model), `bad_record` for corrupt bytes,
// `replay_mismatch` when live and offline digests disagree. Determinism: FNV
// plus little-endian words only; same brief plus seed replays identically.

/// Stage-record format version stamped on every encoded record.
pub const STAGE_RECORD_VERSION: u8 = 1;
/// Seed domain for replay input/output digests (kept apart from play digests).
pub(super) const REPLAY_DOMAIN: u64 = 0x0ab4_9a1c_e009_0001;
/// Longest stage tag accepted while decoding a stage record.
pub(super) const STAGE_TAG_MAX: usize = 32;
/// Longest envelope record line accepted while decoding a stage record.
pub(super) const STAGE_LINE_MAX: usize = 512;
/// Chain stages in canonical record order: AB1 level, AB2 character, AB3
/// encounter, AB4 slice. One format for every stage, never a second.
pub const REPLAY_STAGES: [BriefStage; 4] = [
    BriefStage::Level,
    BriefStage::Character,
    BriefStage::Encounter,
    BriefStage::Slice,
];

impl GameError {
    /// Missing-record refusal: names the absent stage plus the legal stages.
    /// The caller re-records the brief providers-off instead of calling a
    /// model; nothing is read from any provider here.
    pub fn missing_record(stage: &str) -> Self {
        Self::new(
            "missing_record",
            format!(
                "record for stage `{stage}` is missing; submit one record per stage [{}]; \
                 re-record the brief providers-off instead of calling a model",
                BriefStage::all_tags().join(", "),
            ),
        )
    }

    /// Corrupt-record refusal: names what broke, never a guess.
    pub fn bad_record(detail: impl Into<String>) -> Self {
        Self::new(
            "bad_record",
            format!("{}; re-record the brief providers-off", detail.into(),),
        )
    }
}

/// FNV over one stage tag plus the brief id, seed and text: the canonical
/// stage inputs. Same inputs name the same digest on every machine.
pub(super) fn stage_input_digest(stage: BriefStage, brief_id: u64, seed: u64, text: &str) -> u64 {
    let salt = match stage {
        BriefStage::Level => 0x1e00_u64,
        BriefStage::Character => 0xc4a4_u64,
        BriefStage::Encounter => 0xe9c0_u64,
        BriefStage::Slice | BriefStage::Replay => 0x591c_u64,
    };
    let mut input_hasher = Fnv64::with_state(FNV64_OFFSET ^ (REPLAY_DOMAIN ^ salt));
    input_hasher.write(stage.as_str().as_bytes());
    let mut digest = input_hasher.finish();
    input_hasher = Fnv64::with_state(FNV64_OFFSET ^ digest);
    input_hasher.write(&brief_id.to_le_bytes());
    digest = input_hasher.finish();
    input_hasher = Fnv64::with_state(FNV64_OFFSET ^ digest);
    input_hasher.write(&seed.to_le_bytes());
    digest = input_hasher.finish();
    input_hasher = Fnv64::with_state(FNV64_OFFSET ^ digest);
    input_hasher.write(text.as_bytes());
    input_hasher.finish()
}

/// One recorded AB stage: the shared envelope line (the single per-stage
/// record format, stamped identically by live and offline runs) plus the FNV
/// digests of the canonical inputs and outputs. Encodes byte-identically.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageRecord {
    /// Which chain stage this record covers.
    pub stage: BriefStage,
    /// Shared envelope brief id (`brief_id_of` over text plus seed).
    pub brief_id: u64,
    /// Build seed.
    pub seed: u64,
    /// FNV over the canonical stage inputs (text, seed, stage tag).
    pub input_digest: u64,
    /// FNV over the canonical stage outputs (the stage digests).
    pub output_digest: u64,
    /// Shared envelope record line, verbatim (`BriefEnvelope::record_line`).
    pub record_line: String,
}

impl StageRecord {
    /// Canonical bytes: version, tag, ids, digests, record line. Same record
    /// gives the same bytes twice on every machine.
    pub fn encode(&self) -> Vec<u8> {
        let tag = self.stage.as_str();
        let mut out = Vec::with_capacity(1 + 4 + tag.len() + 8 * 4 + 4 + self.record_line.len());
        out.push(STAGE_RECORD_VERSION);
        write_u32(&mut out, tag.len() as u32);
        out.extend_from_slice(tag.as_bytes());
        write_u64(&mut out, self.brief_id);
        write_u64(&mut out, self.seed);
        write_u64(&mut out, self.input_digest);
        write_u64(&mut out, self.output_digest);
        write_u32(&mut out, self.record_line.len() as u32);
        out.extend_from_slice(self.record_line.as_bytes());
        out
    }

    /// Decode canonical bytes; any truncation, version skew, unknown stage or
    /// trailing byte is a loud `bad_record`, never a guess.
    pub fn decode(bytes: &[u8]) -> Result<Self, GameError> {
        let (record, consumed) = decode_stage_at(bytes)?;
        if consumed != bytes.len() {
            return Err(GameError::bad_record(
                "stage record has trailing bytes past the line",
            ));
        }
        Ok(record)
    }

    /// Converged digest of the canonical bytes. Equality across runs is the
    /// replay proof; the absolute value is never compared across shapes.
    pub fn digest(&self) -> u64 {
        let mut stage_hasher = Fnv64::with_state(FNV64_OFFSET ^ REPLAY_DOMAIN);
        stage_hasher.write(&self.encode());
        stage_hasher.finish()
    }
}

/// Full-chain record: one `StageRecord` per AB stage in canonical order plus
/// the play proof. The offline replay rebuilds from the brief inputs and
/// requires these bytes back byte-identically, providers off.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainRecord {
    /// Shared envelope brief id.
    pub brief_id: u64,
    /// Build seed.
    pub seed: u64,
    /// Character body assembled for this chain.
    pub variant: VariantId,
    /// True when the wet-blood veteran finish was applied.
    pub bloodied: bool,
    /// One record per stage in `REPLAY_STAGES` order.
    pub stages: Vec<StageRecord>,
    /// Play digest after five minutes.
    pub play_digest: u64,
    /// FNV fold over the checkpoint tips.
    pub checkpoint_digest: u64,
    /// FNV over the package bytes.
    pub package_digest: u64,
}

impl ChainRecord {
    /// Find one stage record; an absent stage is a loud `missing_record`
    /// instead of a model call.
    pub fn find_stage(&self, stage: BriefStage) -> Result<&StageRecord, GameError> {
        self.stages
            .iter()
            .find(|record| record.stage == stage)
            .ok_or_else(|| GameError::missing_record(stage.as_str()))
    }

    /// Require all four stage records in canonical order; the first gap is a
    /// loud `missing_record` before any rebuild is attempted.
    pub fn verify_complete(&self) -> Result<(), GameError> {
        for stage in REPLAY_STAGES {
            self.find_stage(stage)?;
        }
        Ok(())
    }

    /// Canonical bytes: version, ids, variant, stage records, play proof.
    /// Same chain gives the same bytes twice on every machine.
    pub fn encode(&self) -> Vec<u8> {
        let name = self.variant.name();
        let mut out = Vec::with_capacity(1 + 8 * 2 + 4 + name.len() + 1 + 4 + 256);
        out.push(STAGE_RECORD_VERSION);
        write_u64(&mut out, self.brief_id);
        write_u64(&mut out, self.seed);
        write_u32(&mut out, name.len() as u32);
        out.extend_from_slice(name.as_bytes());
        out.push(u8::from(self.bloodied));
        write_u32(&mut out, self.stages.len() as u32);
        for record in &self.stages {
            out.extend_from_slice(&record.encode());
        }
        write_u64(&mut out, self.play_digest);
        write_u64(&mut out, self.checkpoint_digest);
        write_u64(&mut out, self.package_digest);
        out
    }

    /// Decode canonical bytes; any skew, gap, reorder or trailing byte is a
    /// loud `bad_record`, never a guess.
    pub fn decode(bytes: &[u8]) -> Result<Self, GameError> {
        let bad = |detail: &str| GameError::bad_record(detail);
        if bytes.is_empty() || bytes[0] != STAGE_RECORD_VERSION {
            return Err(bad("chain record version is not 1"));
        }
        let mut at = 1usize;
        let brief_id = read_u64(bytes, &mut at).map_err(|_| bad("chain record is truncated"))?;
        let seed = read_u64(bytes, &mut at).map_err(|_| bad("chain record is truncated"))?;
        let name_len = read_u32(bytes, &mut at)
            .map_err(|_| bad("chain record is truncated"))
            .and_then(|len| {
                if len as usize > STAGE_TAG_MAX {
                    Err(bad("chain record variant is overlong"))
                } else {
                    Ok(len as usize)
                }
            })?;
        let name_end = at.saturating_add(name_len);
        if name_end > bytes.len() {
            return Err(bad("chain record is truncated"));
        }
        let name = core::str::from_utf8(&bytes[at..name_end])
            .map_err(|_| bad("chain record variant is not UTF-8"))?;
        let variant = VariantId::parse(name).map_err(|error| {
            bad(&format!(
                "chain record names unknown variant `{name}`: {error}"
            ))
        })?;
        at = name_end;
        if at >= bytes.len() {
            return Err(bad("chain record is truncated"));
        }
        let bloodied = match bytes[at] {
            0 => false,
            1 => true,
            _ => return Err(bad("chain record bloodied flag is not 0 or 1")),
        };
        at += 1;
        let count = read_u32(bytes, &mut at).map_err(|_| bad("chain record is truncated"))?;
        if count as usize != REPLAY_STAGES.len() {
            return Err(bad(&format!(
                "chain record holds {count} stage record(s); submit exactly {}",
                REPLAY_STAGES.len()
            )));
        }
        let mut stages = Vec::with_capacity(REPLAY_STAGES.len());
        for (index, stage) in REPLAY_STAGES.iter().enumerate() {
            let (record, consumed) = decode_stage_at(&bytes[at..])?;
            at += consumed;
            if record.stage != *stage {
                return Err(bad(&format!(
                    "chain record stage {index} is `{}`; submit `{}`",
                    record.stage.as_str(),
                    stage.as_str()
                )));
            }
            stages.push(record);
        }
        let play_digest = read_u64(bytes, &mut at).map_err(|_| bad("chain record is truncated"))?;
        let checkpoint_digest =
            read_u64(bytes, &mut at).map_err(|_| bad("chain record is truncated"))?;
        let package_digest =
            read_u64(bytes, &mut at).map_err(|_| bad("chain record is truncated"))?;
        if at != bytes.len() {
            return Err(bad("chain record has trailing bytes past the play proof"));
        }
        Ok(Self {
            brief_id,
            seed,
            variant,
            bloodied,
            stages,
            play_digest,
            checkpoint_digest,
            package_digest,
        })
    }

    /// Converged digest of the canonical bytes.
    pub fn digest(&self) -> u64 {
        let mut chain_hasher = Fnv64::with_state(FNV64_OFFSET ^ REPLAY_DOMAIN);
        chain_hasher.write(&self.encode());
        chain_hasher.finish()
    }

    /// One-line replay receipt: brief, seed, stages, play and package digests.
    /// The endpoint offline replay parses this line.
    pub fn summary(&self) -> String {
        format!(
            "replay brief {:016x} seed {} variant {}: {} stages, play {:016x} replay ok, package {:016x}",
            self.brief_id,
            self.seed,
            self.variant.name(),
            self.stages.len(),
            self.play_digest,
            self.package_digest,
        )
    }
}

/// Decode one stage record from a byte prefix, returning the record plus the
/// bytes consumed. The chain decoder walks concatenated records with this; a
/// bare record still decodes through `StageRecord::decode`, which additionally
/// forbids trailing bytes.
pub(super) fn decode_stage_at(bytes: &[u8]) -> Result<(StageRecord, usize), GameError> {
    let bad = |detail: &str| GameError::bad_record(detail);
    if bytes.is_empty() || bytes[0] != STAGE_RECORD_VERSION {
        return Err(bad("stage record version is not 1"));
    }
    let mut at = 1usize;
    let tag_len = read_u32(bytes, &mut at)
        .map_err(|_| bad("stage record is truncated"))
        .and_then(|len| {
            if len as usize > STAGE_TAG_MAX {
                Err(bad("stage record tag is overlong"))
            } else {
                Ok(len as usize)
            }
        })?;
    let tag_end = at.saturating_add(tag_len);
    if tag_end > bytes.len() {
        return Err(bad("stage record is truncated"));
    }
    let tag = core::str::from_utf8(&bytes[at..tag_end])
        .map_err(|_| bad("stage record tag is not UTF-8"))?;
    let stage = BriefStage::parse(tag).map_err(|error| {
        bad(&format!(
            "stage record names unknown stage `{tag}`: {error}"
        ))
    })?;
    at = tag_end;
    let brief_id = read_u64(bytes, &mut at).map_err(|_| bad("stage record is truncated"))?;
    let seed = read_u64(bytes, &mut at).map_err(|_| bad("stage record is truncated"))?;
    let input_digest = read_u64(bytes, &mut at).map_err(|_| bad("stage record is truncated"))?;
    let output_digest = read_u64(bytes, &mut at).map_err(|_| bad("stage record is truncated"))?;
    let line_len = read_u32(bytes, &mut at)
        .map_err(|_| bad("stage record is truncated"))
        .and_then(|len| {
            if len as usize > STAGE_LINE_MAX {
                Err(bad("stage record line is overlong"))
            } else {
                Ok(len as usize)
            }
        })?;
    let line_end = at.saturating_add(line_len);
    if line_end > bytes.len() {
        return Err(bad("stage record is truncated"));
    }
    let record_line = core::str::from_utf8(&bytes[at..line_end])
        .map_err(|_| bad("stage record line is not UTF-8"))?
        .to_owned();
    Ok((
        StageRecord {
            stage,
            brief_id,
            seed,
            input_digest,
            output_digest,
            record_line,
        },
        line_end,
    ))
}

/// Record every AB stage of one packaged slice: the shared envelope line per
/// stage (the single record format, stamped identically live and offline)
/// plus the FNV digests of each stage's canonical outputs. Providers are
/// never consulted; the slice already holds every stage output.
pub fn record_chain(
    text: &str,
    seed: u64,
    variant: VariantId,
    bloodied: bool,
    slice: &PlayableSlice,
) -> Result<ChainRecord, GameError> {
    let brief_id = crate::brief::brief_id_of(text, seed);
    let mut stages = Vec::with_capacity(REPLAY_STAGES.len());
    for stage in REPLAY_STAGES {
        let envelope = BriefEnvelope::new(text, seed, stage)
            .map_err(|error| GameError::new(error.code(), error.to_string()))?;
        debug_assert_eq!(envelope.brief_id(), brief_id);
        let input_digest = stage_input_digest(stage, brief_id, seed, text);
        let output_digest = match stage {
            BriefStage::Level => {
                let mut level_hasher = Fnv64::with_state(FNV64_OFFSET ^ REPLAY_DOMAIN);
                level_hasher.write(&slice.level.level_digest.to_le_bytes());
                let digest = level_hasher.finish();
                level_hasher = Fnv64::with_state(FNV64_OFFSET ^ digest);
                level_hasher.write(&slice.level.cell_digest.to_le_bytes());
                level_hasher.finish()
            }
            BriefStage::Character => slice.character.digest,
            BriefStage::Encounter => {
                let mut encounter_hasher = Fnv64::with_state(FNV64_OFFSET ^ REPLAY_DOMAIN);
                encounter_hasher.write(&slice.encounter.plan_id.to_le_bytes());
                let digest = encounter_hasher.finish();
                encounter_hasher = Fnv64::with_state(FNV64_OFFSET ^ digest);
                encounter_hasher.write(&slice.encounter.fork_id.to_le_bytes());
                encounter_hasher.finish()
            }
            BriefStage::Slice | BriefStage::Replay => slice.play_digest,
        };
        stages.push(StageRecord {
            stage,
            brief_id,
            seed,
            input_digest,
            output_digest,
            record_line: envelope.record_line(),
        });
    }
    let mut checkpoint_digest = REPLAY_DOMAIN;
    for tip in &slice.checkpoints {
        let mut tip_hasher = Fnv64::with_state(FNV64_OFFSET ^ checkpoint_digest);
        tip_hasher.write(&tip.to_le_bytes());
        checkpoint_digest = tip_hasher.finish();
    }
    Ok(ChainRecord {
        brief_id,
        seed,
        variant,
        bloodied,
        stages,
        play_digest: slice.play_digest,
        checkpoint_digest,
        package_digest: slice.package_digest,
    })
}

/// Replay one recorded chain providers-off: rebuild the slice from the brief
/// inputs (deterministic kit, oracle and FNV only; no model is ever called),
/// re-record every stage and require the canonical bytes back byte-identically
/// with matching digests. A missing stage record fails loudly before any
/// rebuild; any divergence is a loud `replay_mismatch`.
pub fn replay_chain_offline(
    record: &ChainRecord,
    text: &str,
    seed: u64,
    variant: VariantId,
    bloodied: bool,
) -> Result<ChainRecord, GameError> {
    record.verify_complete()?;
    let slice = build_slice(text, seed, variant, bloodied)?;
    let fresh = record_chain(text, seed, variant, bloodied, &slice)?;
    if fresh.encode() != record.encode() || fresh.digest() != record.digest() {
        return Err(GameError::replay_mismatch(
            record.play_digest,
            fresh.play_digest,
        ));
    }
    Ok(fresh)
}

/// Record one fixture chain, then replay it twice providers-off. The three
/// records must match byte-identically with matching live and offline
/// digests; this is the endpoint offline replay entry point over the ten
/// packaged builds.
pub fn replay_fixture_offline(
    index: usize,
) -> Result<(ChainRecord, ChainRecord, ChainRecord), GameError> {
    let fixtures = fixture_briefs();
    let fixture = fixtures.get(index).ok_or_else(|| {
        GameError::new(
            "bad_brief",
            format!(
                "fixture {index} is unknown; submit 0..={} for the ten recorded builds",
                fixtures.len() - 1
            ),
        )
    })?;
    let slice = build_slice(
        fixture.text,
        fixture.seed,
        fixture.variant,
        fixture.bloodied,
    )?;
    let live = record_chain(
        fixture.text,
        fixture.seed,
        fixture.variant,
        fixture.bloodied,
        &slice,
    )?;
    let first = replay_chain_offline(
        &live,
        fixture.text,
        fixture.seed,
        fixture.variant,
        fixture.bloodied,
    )?;
    let second = replay_chain_offline(
        &live,
        fixture.text,
        fixture.seed,
        fixture.variant,
        fixture.bloodied,
    )?;
    Ok((live, first, second))
}

// AB11 audit glue: per-stage LW6 records for AB2 (character) and AB3
// (encounter), stamping the single `StageRecord` format byte-identically
// to `record_chain` for the same inputs. Owner is `engine-endpoint`;
// consumer is the AB11 audit test below plus the AB9 offline replay.
// Units are brief chars, seeds and FNV digests; providers are never
// consulted. Errors ride `GameError`: stage builds refuse with the stage
// named, a foreign stage is a loud `bad_record`, a missing stage is a
// loud `missing_record`, and any divergence is a `replay_mismatch`.

/// Record one AB2 character stage: envelope line plus the FNV digests of
/// the canonical inputs and the character receipt digest. Uses the slice
/// clip set so the record matches `record_chain` byte-identically.
pub fn record_character_stage(
    text: &str,
    seed: u64,
    variant: VariantId,
    bloodied: bool,
) -> Result<StageRecord, GameError> {
    let envelope = BriefEnvelope::new(text, seed, BriefStage::Character)
        .map_err(|error| GameError::new(error.code(), error.to_string()))?;
    let brief = CharacterBrief {
        brief_text: text.to_owned(),
        variant,
        want_clips: SLICE_CLIPS.iter().map(|slot| (*slot).to_owned()).collect(),
        bloodied,
        license: LicenseRef::ct7(),
        repairs_used: 0,
        seed,
    };
    let character = build_character(&brief).map_err(|error| {
        GameError::new(
            "character_refused",
            format!("AB2 refused with {}: {error}", error.code),
        )
    })?;
    let brief_id = envelope.brief_id();
    Ok(StageRecord {
        stage: BriefStage::Character,
        brief_id,
        seed,
        input_digest: stage_input_digest(BriefStage::Character, brief_id, seed, text),
        output_digest: character.digest,
        record_line: envelope.record_line(),
    })
}

/// Record one AB3 encounter stage: envelope line plus the FNV digests of
/// the canonical inputs and the plan/fork pair. Uses the deterministic
/// oracle so the record matches `record_chain` byte-identically.
pub fn record_encounter_stage(text: &str, seed: u64) -> Result<StageRecord, GameError> {
    let envelope = BriefEnvelope::new(text, seed, BriefStage::Encounter)
        .map_err(|error| GameError::new(error.code(), error.to_string()))?;
    let brief = EncounterBrief::new(text, seed).map_err(|error| {
        let message = error.to_string();
        GameError::new(error.code(), message)
    })?;
    let encounter = build_encounter(&brief, &DeterministicOracle).map_err(|refusal| {
        GameError::new("encounter_refused", format!("AB3 refused: {refusal}"))
    })?;
    let brief_id = envelope.brief_id();
    let output_digest = {
        let mut encounter_hasher = Fnv64::with_state(FNV64_OFFSET ^ REPLAY_DOMAIN);
        encounter_hasher.write(&encounter.plan_id.to_le_bytes());
        let digest = encounter_hasher.finish();
        encounter_hasher = Fnv64::with_state(FNV64_OFFSET ^ digest);
        encounter_hasher.write(&encounter.fork_id.to_le_bytes());
        encounter_hasher.finish()
    };
    Ok(StageRecord {
        stage: BriefStage::Encounter,
        brief_id,
        seed,
        input_digest: stage_input_digest(BriefStage::Encounter, brief_id, seed, text),
        output_digest,
        record_line: envelope.record_line(),
    })
}

/// Replay one recorded AB2 character stage providers-off and require the
/// canonical bytes back byte-identically. A foreign stage is a loud
/// `bad_record`; any divergence is a loud `replay_mismatch`.
pub fn replay_character_stage_offline(
    record: &StageRecord,
    text: &str,
    seed: u64,
    variant: VariantId,
    bloodied: bool,
) -> Result<StageRecord, GameError> {
    if record.stage != BriefStage::Character {
        return Err(GameError::bad_record(format!(
            "stage record is `{}`; submit `{}`",
            record.stage.as_str(),
            BriefStage::Character.as_str()
        )));
    }
    let fresh = record_character_stage(text, seed, variant, bloodied)?;
    if fresh.encode() != record.encode() || fresh.digest() != record.digest() {
        return Err(GameError::replay_mismatch(
            record.output_digest,
            fresh.output_digest,
        ));
    }
    Ok(fresh)
}

/// Replay one recorded AB3 encounter stage providers-off and require the
/// canonical bytes back byte-identically. A foreign stage is a loud
/// `bad_record`; any divergence is a loud `replay_mismatch`.
pub fn replay_encounter_stage_offline(
    record: &StageRecord,
    text: &str,
    seed: u64,
) -> Result<StageRecord, GameError> {
    if record.stage != BriefStage::Encounter {
        return Err(GameError::bad_record(format!(
            "stage record is `{}`; submit `{}`",
            record.stage.as_str(),
            BriefStage::Encounter.as_str()
        )));
    }
    let fresh = record_encounter_stage(text, seed)?;
    if fresh.encode() != record.encode() || fresh.digest() != record.digest() {
        return Err(GameError::replay_mismatch(
            record.output_digest,
            fresh.output_digest,
        ));
    }
    Ok(fresh)
}

/// Find the Character stage record; an absent stage is a loud
/// `missing_record` instead of a model call.
pub fn find_character_record(records: &[StageRecord]) -> Result<&StageRecord, GameError> {
    records
        .iter()
        .find(|record| record.stage == BriefStage::Character)
        .ok_or_else(|| GameError::missing_record(BriefStage::Character.as_str()))
}

/// Find the Encounter stage record; an absent stage is a loud
/// `missing_record` instead of a model call.
pub fn find_encounter_record(records: &[StageRecord]) -> Result<&StageRecord, GameError> {
    records
        .iter()
        .find(|record| record.stage == BriefStage::Encounter)
        .ok_or_else(|| GameError::missing_record(BriefStage::Encounter.as_str()))
}

/// Replay one stored Character record providers-off: the missing-record
/// check runs before any rebuild, then the bytes must come back
/// byte-identically.
pub fn replay_stored_character(
    records: &[StageRecord],
    text: &str,
    seed: u64,
    variant: VariantId,
    bloodied: bool,
) -> Result<StageRecord, GameError> {
    let record = find_character_record(records)?;
    replay_character_stage_offline(record, text, seed, variant, bloodied)
}

/// Replay one stored Encounter record providers-off: the missing-record
/// check runs before any rebuild, then the bytes must come back
/// byte-identically.
pub fn replay_stored_encounter(
    records: &[StageRecord],
    text: &str,
    seed: u64,
) -> Result<StageRecord, GameError> {
    let record = find_encounter_record(records)?;
    replay_encounter_stage_offline(record, text, seed)
}
