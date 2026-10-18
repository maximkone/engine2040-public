//! Brief-to-game eval: 20 briefs through the four authoring gates (level,
//! character, encounter, slice).
//!
//! Consumer: the quality-bar eval loop reads [`run_report`] plus the
//! `target/evidence/ev2` text report. Owner: this module owns briefs,
//! scores and reports; the authoring stages stay read-only in `engine-endpoint`.
//! Units: brief chars, fixed ticks, permille pass rates, advisory 0-100
//! vision points. Determinism: same briefs twice give the same report;
//! no wall clock or map order enters a score. Errors: [`EvalError`].

use std::path::{Path, PathBuf};

use engine_arena::{ArenaConfig, BoutTape};

use crate::frame_check::{check_scripted, Script, Thresholds};
use crate::keypoint;
use crate::rebuild;

/// Briefs scored in one eval run.
pub const BRIEF_COUNT: usize = 20;
/// Report format version stamped on every report.
pub const REPORT_VERSION: u16 = 1;
/// Headless match length per brief in fixed ticks.
pub const MATCH_TICKS: u32 = 120;
/// Arena half-extent per brief in meters.
pub const HALF_EXTENT_M: f32 = 20.0;
/// Participants per brief match.
pub const FIGHTERS: u8 = 2;
/// Longest brief text in chars (mirrors the AB envelope bound).
pub const BRIEF_MAX_CHARS: usize = 4096;

/// One eval brief: stable id, level directives plus one `#` prose line.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Brief {
    /// Stable id `b01`..`b20`.
    pub id: &'static str,
    /// Level directives plus one `#` prose comment.
    pub text: &'static str,
    /// Build seed.
    pub seed: u64,
}

/// The 20 eval briefs: ten slice-fixture shapes plus ten mirrored
/// variants with distinct props, walls and seeds.
pub const BRIEFS: [Brief; BRIEF_COUNT] = [
    Brief { id: "b01", text: "prop brazier slice_01\nwant slice_01 next_to wall:north [0,80]\nspawn slice_01\n# a quiet duel at dawn in the training yard", seed: 5000 },
    Brief { id: "b02", text: "prop crate slice_02\nwant slice_02 next_to wall:south [0,80]\nspawn slice_02\n# a rivalry grudge match under the noon sun", seed: 5001 },
    Brief { id: "b03", text: "prop pillar slice_03\nwant slice_03 next_to wall:east [0,80]\nspawn slice_03\n# a brutal cauldron final before the crowd", seed: 5002 },
    Brief { id: "b04", text: "prop torch slice_04\nwant slice_04 next_to wall:west [0,80]\nspawn slice_04\n# an advanced duel with footwork drills", seed: 5003 },
    Brief { id: "b05", text: "prop altar slice_05\nwant slice_05 next_to wall:north [0,80]\nspawn slice_05\n# a training duel for the new recruits", seed: 5004 },
    Brief { id: "b06", text: "prop stand slice_06\nwant slice_06 next_to wall:south [0,80]\nspawn slice_06\n# a pressure rivalry between rival houses", seed: 5005 },
    Brief { id: "b07", text: "prop post slice_07\nwant slice_07 next_to wall:east [0,80]\nspawn slice_07\n# an easy sparring circle for beginners", seed: 5006 },
    Brief { id: "b08", text: "prop bench slice_08\nwant slice_08 next_to wall:west [0,80]\nspawn slice_08\n# a hard final between champions", seed: 5007 },
    Brief { id: "b09", text: "prop barrel slice_09\nwant slice_09 next_to wall:north [0,80]\nspawn slice_09\n# a marathon duel of endurance", seed: 5008 },
    Brief { id: "b10", text: "prop banner slice_10\nwant slice_10 next_to wall:south [0,80]\nspawn slice_10\n# an exhausting rivalry across the sand", seed: 5009 },
    Brief { id: "b11", text: "prop brazier slice_11\nwant slice_11 next_to wall:east [0,80]\nspawn slice_11\n# a dawn rematch between tired guards", seed: 5010 },
    Brief { id: "b12", text: "prop crate slice_12\nwant slice_12 next_to wall:west [0,80]\nspawn slice_12\n# a noon trial for the second cohort", seed: 5011 },
    Brief { id: "b13", text: "prop pillar slice_13\nwant slice_13 next_to wall:north [0,80]\nspawn slice_13\n# a dusk semifinal before the elders", seed: 5012 },
    Brief { id: "b14", text: "prop torch slice_14\nwant slice_14 next_to wall:south [0,80]\nspawn slice_14\n# a night drill with slow footwork", seed: 5013 },
    Brief { id: "b15", text: "prop altar slice_15\nwant slice_15 next_to wall:east [0,80]\nspawn slice_15\n# a morning rite for the recruits", seed: 5014 },
    Brief { id: "b16", text: "prop stand slice_16\nwant slice_16 next_to wall:west [0,80]\nspawn slice_16\n# a house derby under strict judges", seed: 5015 },
    Brief { id: "b17", text: "prop post slice_17\nwant slice_17 next_to wall:north [0,80]\nspawn slice_17\n# a beginner circle with wooden blades", seed: 5016 },
    Brief { id: "b18", text: "prop bench slice_18\nwant slice_18 next_to wall:south [0,80]\nspawn slice_18\n# a champion bout past midnight", seed: 5017 },
    Brief { id: "b19", text: "prop barrel slice_19\nwant slice_19 next_to wall:east [0,80]\nspawn slice_19\n# an endurance trial across the yard", seed: 5018 },
    Brief { id: "b20", text: "prop banner slice_20\nwant slice_20 next_to wall:west [0,80]\nspawn slice_20\n# a sandstorm duel with low torches", seed: 5019 },
];

/// Which AB stage a seeded break targets. `None` is the clean run.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BreakStage {
    /// No break; every brief runs clean.
    None,
    /// Force the level grammar stage to refuse one brief.
    Ab1,
    /// Force the character bind stage to refuse one brief.
    Ab2,
    /// Force the encounter prose stage to refuse one brief.
    Ab3,
    /// Force the slice play/replay stage to refuse one brief.
    Ab4,
}

/// What stopped the eval harness itself (never a per-brief gameplay fail).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum EvalError {
    /// Keypoint suite refused to load or run.
    Keypoints(String),
    /// Report directory refused.
    Io(String),
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Keypoints(reason) => write!(f, "keypoints refused: {reason}"),
            Self::Io(reason) => write!(f, "io refused: {reason}"),
        }
    }
}

impl std::error::Error for EvalError {}

/// Per-brief score: four authoring gates plus keypoints, replay and advisory vision.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BriefScore {
    /// Brief id.
    pub id: &'static str,
    /// Build seed.
    pub seed: u64,
    /// Level gate passed.
    pub ab1_ok: bool,
    /// Character gate passed.
    pub ab2_ok: bool,
    /// Encounter gate passed.
    pub ab3_ok: bool,
    /// Slice play gate passed.
    pub ab4_ok: bool,
    /// Keypoints passed (suite-wide count, same for every brief).
    pub keypoints_passed: usize,
    /// Keypoints total.
    pub keypoints_total: usize,
    /// True when the suite passed every keypoint.
    pub keypoint_ok: bool,
    /// True when play and replay digests matched.
    pub replay_ok: bool,
    /// Play digest after the headless window.
    pub play_digest: u64,
    /// Replay digest; must equal `play_digest`.
    pub replay_digest: u64,
    /// First tick the replay audit named as bad; `None` for a clean replay.
    pub first_bad_tick: Option<u32>,
    /// Advisory vision points 0..=100; never gates `pass`.
    pub vision_score: u32,
    /// Advisory capture digest.
    pub vision_digest: String,
    /// Advisory note; always starts with `advisory:`.
    pub vision_note: String,
    /// True when the four authoring gates, keypoints and replay all passed.
    pub pass: bool,
}

impl BriefScore {
    /// True when the asset stages (level, character, encounter) passed but
    /// runtime failed.
    pub fn asset_pass_runtime_fail(&self) -> bool {
        self.ab1_ok
            && self.ab2_ok
            && self.ab3_ok
            && !(self.ab4_ok && self.keypoint_ok && self.replay_ok)
    }

    /// The first gate that refused this build, typed; `None` when it passed.
    pub fn fault(&self) -> Option<BundleFault> {
        if !self.ab1_ok {
            Some(BundleFault::Level)
        } else if !self.ab2_ok {
            Some(BundleFault::Character)
        } else if !self.ab3_ok {
            Some(BundleFault::Encounter)
        } else if !(self.ab4_ok && self.replay_ok) {
            Some(BundleFault::Replay {
                first_bad_tick: self.first_bad_tick,
            })
        } else if !self.keypoint_ok {
            Some(BundleFault::Keypoints {
                failed: self.keypoints_total - self.keypoints_passed,
            })
        } else {
            None
        }
    }
}

/// Why a brief-to-game bundle was refused: the first gate that said no.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BundleFault {
    /// AB1: the level grammar found no prop, want or spawn line.
    Level,
    /// AB2: the character bind found nothing to bind.
    Character,
    /// AB3: the encounter prose stage found no prose line.
    Encounter,
    /// AB4: the match did not replay; the audit named the first bad tick.
    Replay {
        /// First tick whose checksum differs; `None` when no audit ran.
        first_bad_tick: Option<u32>,
    },
    /// The keypoint suite had failing specs.
    Keypoints {
        /// Specs that failed.
        failed: usize,
    },
    /// The keypoint suite could not run, so nothing was judged.
    Harness(String),
}

impl std::fmt::Display for BundleFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Level => write!(
                f,
                "AB1 level refused: needs a prop, a want and a spawn line"
            ),
            Self::Character => write!(f, "AB2 character refused: nothing to bind"),
            Self::Encounter => write!(f, "AB3 encounter refused: no prose line"),
            Self::Replay {
                first_bad_tick: Some(tick),
            } => write!(f, "AB4 slice refused: replay differs from tick {tick}"),
            Self::Replay {
                first_bad_tick: None,
            } => write!(f, "AB4 slice refused: no replay to audit"),
            Self::Keypoints { failed } => write!(f, "keypoints refused: {failed} specs failed"),
            Self::Harness(reason) => write!(f, "eval harness refused: {reason}"),
        }
    }
}

impl std::error::Error for BundleFault {}

/// A bad build seeded into one bundle run: the same brief, one thing broken.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BadBuild {
    /// The build lost its `spawn` line.
    NoSpawn,
    /// The build lost its `#` prose line.
    NoProse,
    /// The recorded match claims one wrong checksum at this tick.
    TamperedTick(u32),
}

impl BadBuild {
    /// The brief text as this bad build has it.
    fn text(self, brief: &Brief) -> String {
        let dropped = match self {
            Self::NoSpawn => "spawn",
            Self::NoProse => "#",
            Self::TamperedTick(_) => return brief.text.to_string(),
        };
        let kept: Vec<&str> = brief
            .text
            .lines()
            .filter(|line| !line.trim_start().starts_with(dropped))
            .collect();
        kept.join("\n")
    }

    fn tamper(self) -> Option<u32> {
        match self {
            Self::TamperedTick(tick) => Some(tick),
            _ => None,
        }
    }
}

/// Whole-eval report: 20 brief scores plus suite and stage pass rates.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EvalReport {
    /// Report format version.
    pub version: u16,
    /// One score per brief in `b01`..`b20` order.
    pub briefs: Vec<BriefScore>,
    /// Keypoints passed suite-wide.
    pub keypoints_passed: usize,
    /// Keypoints total suite-wide.
    pub keypoints_total: usize,
    /// Suite digest over ordered keypoint digests.
    pub suite_digest: u64,
    /// Simulated ticks behind the keypoint suite.
    pub suite_ticks: u64,
    /// Spine rules with a passing spec, of `rules_total` (ARC11 map).
    pub rules_covered: usize,
    /// Spine rules in the rule-to-spec map.
    pub rules_total: usize,
}

impl EvalReport {
    /// Briefs that passed every gate.
    pub fn passed(&self) -> usize {
        self.briefs.iter().filter(|score| score.pass).count()
    }

    /// Pass count for one AB stage by index 1..=4.
    pub fn stage_passed(&self, stage: u8) -> usize {
        match stage {
            1 => self.briefs.iter().filter(|score| score.ab1_ok).count(),
            2 => self.briefs.iter().filter(|score| score.ab2_ok).count(),
            3 => self.briefs.iter().filter(|score| score.ab3_ok).count(),
            _ => self.briefs.iter().filter(|score| score.ab4_ok).count(),
        }
    }

    /// Replay pass count across briefs.
    pub fn replay_passed(&self) -> usize {
        self.briefs.iter().filter(|score| score.replay_ok).count()
    }

    /// Headline metric: asset stages green but runtime red.
    pub fn asset_pass_runtime_fail(&self) -> usize {
        self.briefs
            .iter()
            .filter(|score| score.asset_pass_runtime_fail())
            .count()
    }

    /// Keypoint pass rate in permille.
    pub fn keypoint_rate_permille(&self) -> u16 {
        if self.keypoints_total == 0 {
            return 0;
        }
        ((self.keypoints_passed * 1000) / self.keypoints_total) as u16
    }

    /// Reproducible text report: summary plus one line per brief.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str("EV2 brief-to-game eval: 20 briefs through AB1-AB4\n");
        out.push_str(&format!(
            "version={} briefs={} passed={} ab1={} ab2={} ab3={} ab4={} replay={} asset_pass_runtime_fail={}\n",
            self.version,
            self.briefs.len(),
            self.passed(),
            self.stage_passed(1),
            self.stage_passed(2),
            self.stage_passed(3),
            self.stage_passed(4),
            self.replay_passed(),
            self.asset_pass_runtime_fail(),
        ));
        out.push_str(&format!(
            "keypoints passed={}/{} rate_permille={} suite_digest={:016x} suite_ticks={}\n",
            self.keypoints_passed,
            self.keypoints_total,
            self.keypoint_rate_permille(),
            self.suite_digest,
            self.suite_ticks,
        ));
        out.push_str(&format!(
            "spine rules covered={}/{}\n",
            self.rules_covered, self.rules_total,
        ));
        out.push_str("vision is advisory only and never gates pass\n");
        out.push_str(
            "id seed ab1 ab2 ab3 ab4 keypoint replay pass vision_score vision_digest play_digest\n",
        );
        for score in &self.briefs {
            out.push_str(&format!(
                "{} {} {} {} {} {} {} {} {} {} {} {:016x}\n",
                score.id,
                score.seed,
                flag(score.ab1_ok),
                flag(score.ab2_ok),
                flag(score.ab3_ok),
                flag(score.ab4_ok),
                flag(score.keypoint_ok),
                flag(score.replay_ok),
                flag(score.pass),
                score.vision_score,
                score.vision_digest,
                score.play_digest,
            ));
        }
        out
    }

    /// Machine TSV: one header plus one row per brief.
    pub fn to_tsv(&self) -> String {
        let mut out = String::from(
            "id\tseed\tab1\tab2\tab3\tab4\tkeypoints_passed\tkeypoints_total\treplay\tpass\tvision_score\tvision_digest\tplay_digest\n",
        );
        for score in &self.briefs {
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:016x}\n",
                score.id,
                score.seed,
                flag(score.ab1_ok),
                flag(score.ab2_ok),
                flag(score.ab3_ok),
                flag(score.ab4_ok),
                score.keypoints_passed,
                score.keypoints_total,
                flag(score.replay_ok),
                flag(score.pass),
                score.vision_score,
                score.vision_digest,
                score.play_digest,
            ));
        }
        out
    }
}

fn flag(ok: bool) -> &'static str {
    if ok {
        "pass"
    } else {
        "fail"
    }
}

fn ab1_level_ok(text: &str) -> bool {
    let chars = text.chars().count();
    if text.split_whitespace().next().is_none() || chars > BRIEF_MAX_CHARS {
        return false;
    }
    text.contains("prop ") && text.contains("spawn") && text.contains("want ")
}

fn ab2_character_ok(text: &str) -> bool {
    text.len() > 20 && text.contains("slice_")
}

fn ab3_encounter_ok(text: &str) -> bool {
    text.lines().any(|line| line.trim_start().starts_with('#')) && text.chars().count() > 40
}

/// Play one brief's match and audit its record. `tamper` flips one claimed
/// checksum at that tick, the way a bad build leaves a record that lies.
fn audited_match(seed: u64, tamper: Option<u32>) -> Option<(u64, rebuild::Audit)> {
    let config = ArenaConfig {
        seed,
        ticks: MATCH_TICKS,
        fighters: FIGHTERS,
        half_extent_m: HALF_EXTENT_M,
        table_version: 1,
    };
    // EV16: the replay is a sparse rebuild (snapshots every few ticks, a
    // checksum per tick) and a bad tick anywhere refuses the whole replay.
    let (keys, mut claim) = rebuild::take(&config, &BoutTape::default(), rebuild::STRIDE)?;
    let play = *claim.checksums.last()?;
    if let Some(tick) = tamper {
        *claim.checksums.get_mut(usize::try_from(tick).ok()?)? ^= 1;
    }
    Some((play, rebuild::audit(keys, &claim)?))
}

fn vision_for(seed: u64) -> (u32, String, String) {
    match check_scripted(Script::Clean, 64, 64, seed, &Thresholds::default()) {
        Ok(metrics) => {
            let score = (metrics.cover_permille.min(1000) / 10) as u32;
            let note = format!(
                "advisory:cover {} permille over {} draws",
                metrics.cover_permille, metrics.draws
            );
            (score, metrics.digest, note)
        }
        Err(fault) => (
            0,
            crate::frame_check::digest_hex(&[]),
            format!("advisory:{fault}"),
        ),
    }
}

/// One build under test: the brief text as built, so a bad variant can
/// differ from its brief.
struct Build<'a> {
    id: &'static str,
    text: &'a str,
    seed: u64,
    tamper: Option<u32>,
}

/// Suite-wide keypoint numbers every brief row carries.
#[derive(Clone, Copy)]
struct KeypointTally {
    passed: usize,
    total: usize,
    ok: bool,
}

impl KeypointTally {
    fn of(suite: &keypoint::SuiteReport) -> Self {
        Self {
            passed: suite.passed(),
            total: suite.results.len(),
            ok: suite.pass(),
        }
    }
}

fn score_brief(
    build: &Build<'_>,
    keypoints: KeypointTally,
    broken: Option<(&'static str, BreakStage)>,
) -> BriefScore {
    let breaks_this = broken.is_some_and(|(id, _)| id == build.id);
    let break_stage = broken.map(|(_, stage)| stage).unwrap_or(BreakStage::None);
    let mut ab1_ok = ab1_level_ok(build.text);
    let mut ab2_ok = ab2_character_ok(build.text);
    let mut ab3_ok = ab3_encounter_ok(build.text);
    let audited = audited_match(build.seed, build.tamper);
    let first_bad_tick = audited.as_ref().and_then(|(_, audit)| audit.first_bad);
    let (mut play, replay) = match audited {
        Some((play, audit)) if audit.first_bad.is_none() => (play, audit.last),
        _ => (0, 1),
    };
    let mut replay_ok = play == replay && play != 0;
    let mut ab4_ok = replay_ok;
    if breaks_this {
        match break_stage {
            BreakStage::Ab1 => ab1_ok = false,
            BreakStage::Ab2 => ab2_ok = false,
            BreakStage::Ab3 => ab3_ok = false,
            BreakStage::Ab4 => {
                ab4_ok = false;
                replay_ok = false;
                play = play.wrapping_add(1);
            }
            BreakStage::None => {}
        }
    }
    let (vision_score, vision_digest, vision_note) = vision_for(build.seed);
    let pass = ab1_ok && ab2_ok && ab3_ok && ab4_ok && keypoints.ok && replay_ok;
    BriefScore {
        id: build.id,
        seed: build.seed,
        ab1_ok,
        ab2_ok,
        ab3_ok,
        ab4_ok,
        keypoints_passed: keypoints.passed,
        keypoints_total: keypoints.total,
        keypoint_ok: keypoints.ok,
        replay_ok,
        play_digest: play,
        replay_digest: replay,
        first_bad_tick,
        vision_score,
        vision_digest,
        vision_note,
        pass,
    }
}

/// Run the clean eval: 20 briefs through the four authoring gates with the
/// keypoint suite.
pub fn run_report() -> Result<EvalReport, EvalError> {
    run_report_with_break(None)
}

/// Run the eval with one seeded broken stage on one brief. The break
/// lowers exactly that brief's stage score; every other row is clean.
pub fn run_report_with_break(
    broken: Option<(&'static str, BreakStage)>,
) -> Result<EvalReport, EvalError> {
    let suite = keypoint::run_suite().map_err(|error| EvalError::Keypoints(error.to_string()))?;
    let keypoints = KeypointTally::of(&suite);
    let mut briefs = Vec::with_capacity(BRIEF_COUNT);
    for brief in &BRIEFS {
        let build = Build {
            id: brief.id,
            text: brief.text,
            seed: brief.seed,
            tamper: None,
        };
        briefs.push(score_brief(&build, keypoints, broken));
    }
    let (rules_covered, rules_total) = suite.rules_covered();
    Ok(EvalReport {
        version: REPORT_VERSION,
        briefs,
        keypoints_passed: suite.passed(),
        keypoints_total: suite.results.len(),
        suite_digest: suite.digest,
        suite_ticks: suite.sim_ticks,
        rules_covered,
        rules_total,
    })
}

/// Build one brief into a game bundle, with one seeded bad build or none.
/// The bundle passes with its score, or fails loud with the typed reason
/// of the first gate that refused it. The same call twice gives the same
/// answer.
pub fn run_bundle(brief: &Brief, bad: Option<BadBuild>) -> Result<BriefScore, BundleFault> {
    let suite = keypoint::run_suite().map_err(|error| BundleFault::Harness(error.to_string()))?;
    let keypoints = KeypointTally::of(&suite);
    let text = bad.map_or_else(|| brief.text.to_string(), |bad| bad.text(brief));
    let build = Build {
        id: brief.id,
        text: &text,
        seed: brief.seed,
        tamper: bad.and_then(BadBuild::tamper),
    };
    let score = score_brief(&build, keypoints, None);
    score.fault().map_or(Ok(score), Err)
}

/// Default report directory inside the lane checkout target tree.
pub fn default_report_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/ev2")
}

/// Write `report.txt` plus `report.tsv` under `dir`. Returns both paths.
pub fn write_reports(dir: &Path) -> Result<Vec<PathBuf>, EvalError> {
    let report = run_report()?;
    write_report_files(dir, &report)
}

/// Write one already-built report under `dir`. Tests use this so the
/// files and the asserted report are the same value.
pub fn write_report_files(dir: &Path, report: &EvalReport) -> Result<Vec<PathBuf>, EvalError> {
    std::fs::create_dir_all(dir).map_err(|error| EvalError::Io(error.to_string()))?;
    let text_path = dir.join("report.txt");
    let tsv_path = dir.join("report.tsv");
    std::fs::write(&text_path, report.to_text())
        .map_err(|error| EvalError::Io(error.to_string()))?;
    std::fs::write(&tsv_path, report.to_tsv()).map_err(|error| EvalError::Io(error.to_string()))?;
    Ok(vec![text_path, tsv_path])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twenty_briefs_have_unique_ids_and_seeds() {
        assert_eq!(BRIEFS.len(), BRIEF_COUNT);
        let mut ids: Vec<&str> = BRIEFS.iter().map(|brief| brief.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), BRIEF_COUNT);
        let mut seeds: Vec<u64> = BRIEFS.iter().map(|brief| brief.seed).collect();
        seeds.sort_unstable();
        seeds.dedup();
        assert_eq!(seeds.len(), BRIEF_COUNT);
        for brief in &BRIEFS {
            assert!(ab1_level_ok(brief.text), "{} must clear AB1", brief.id);
            assert!(ab2_character_ok(brief.text), "{} must clear AB2", brief.id);
            assert!(ab3_encounter_ok(brief.text), "{} must clear AB3", brief.id);
        }
    }

    #[test]
    fn keypoint_suite_shape_matches_vk1() {
        let suite = keypoint::run_suite().expect("suite runs");
        assert_eq!(suite.results.len(), keypoint::KEYPOINT_COUNT);
        // The 30 VK1 fork specs stay; ARC8/ARC11 add `spine_*` specs beside them.
        let forks = suite.results.iter().filter(|r| r.id.starts_with("kp"));
        assert_eq!(forks.count(), 30);
    }

    #[test]
    fn clean_report_scores_every_brief() {
        let report = run_report().expect("clean eval runs");
        assert_eq!(report.briefs.len(), BRIEF_COUNT);
        assert_eq!(report.keypoints_total, keypoint::KEYPOINT_COUNT);
        assert_eq!(report.replay_passed(), BRIEF_COUNT);
        for score in &report.briefs {
            assert!(
                score.replay_ok,
                "{} must replay: {:016x}",
                score.id, score.play_digest
            );
            assert_eq!(score.play_digest, score.replay_digest);
            assert!(score.vision_note.starts_with("advisory:"));
            assert_eq!(score.vision_digest.len(), 16);
        }
        let text = report.to_text();
        assert!(text.contains("keypoints"));
        assert!(text.contains(&format!(
            "spine rules covered={0}/{0}",
            keypoint::SPINE_RULES.len()
        )));
        assert!(text.contains("vision is advisory only"));
        let tsv = report.to_tsv();
        assert!(tsv.starts_with("id\tseed"));
    }

    #[test]
    fn report_reproduces_twice() {
        let first = run_report().expect("first run");
        let second = run_report().expect("second run");
        assert_eq!(first, second);
        assert_eq!(first.to_text(), second.to_text());
    }

    #[test]
    fn seeded_broken_ab1_lowers_exactly_its_brief() {
        let report =
            run_report_with_break(Some(("b03", BreakStage::Ab1))).expect("broken eval runs");
        for score in &report.briefs {
            if score.id == "b03" {
                assert!(!score.ab1_ok);
                assert!(!score.pass);
            } else {
                assert!(score.ab1_ok, "{} stays green", score.id);
            }
        }
        assert_eq!(report.stage_passed(1), BRIEF_COUNT - 1);
        assert_eq!(report.stage_passed(4), BRIEF_COUNT);
    }

    #[test]
    fn seeded_broken_ab4_lowers_exactly_its_brief() {
        let report =
            run_report_with_break(Some(("b07", BreakStage::Ab4))).expect("broken eval runs");
        for score in &report.briefs {
            if score.id == "b07" {
                assert!(!score.ab4_ok);
                assert!(!score.replay_ok);
                assert!(!score.pass);
            } else {
                assert!(score.ab4_ok, "{} stays green", score.id);
                assert!(score.replay_ok, "{} still replays", score.id);
            }
        }
        assert_eq!(report.stage_passed(4), BRIEF_COUNT - 1);
        assert_eq!(report.replay_passed(), BRIEF_COUNT - 1);
    }

    #[test]
    fn seeded_bad_brief_delta_is_positive_and_reproduces_twice() {
        // AB14: a seeded bad stage lowers the pass count by exactly one and
        // the broken report is identical on a second run.
        let clean = run_report().expect("clean eval runs");
        let broken =
            || run_report_with_break(Some(("b01", BreakStage::Ab3))).expect("broken eval runs");
        let first = broken();
        assert_eq!(first, broken(), "broken report reproduces x2");
        assert_eq!(clean.passed() - first.passed(), 1, "delta is exactly one");
        assert!(!first.briefs[0].ab3_ok, "b01 is the seeded bad brief");
    }

    #[test]
    fn bundle_fails_loud_on_each_seeded_bad_build_and_passes_the_good_one() {
        // NS16: one brief, three seeded bad builds, typed reasons, x2.
        let brief = &BRIEFS[0];
        let good = run_bundle(brief, None).expect("the good build passes");
        assert!(good.pass);
        assert_eq!(Ok(good), run_bundle(brief, None), "good build equal x2");
        let cases = [
            (BadBuild::NoSpawn, BundleFault::Level),
            (BadBuild::NoProse, BundleFault::Encounter),
            (
                BadBuild::TamperedTick(37),
                BundleFault::Replay {
                    first_bad_tick: Some(37),
                },
            ),
        ];
        for (bad, fault) in cases {
            let refused = run_bundle(brief, Some(bad)).expect_err("a bad build is refused");
            assert_eq!(refused, fault, "{bad:?} gets a typed reason");
            assert!(
                refused.to_string().contains("refused"),
                "{bad:?} reads loud"
            );
            assert_eq!(
                run_bundle(brief, Some(bad)).expect_err("refused x2"),
                refused,
                "{bad:?} refused the same way x2"
            );
        }
    }

    #[test]
    fn tampered_checksum_is_named_by_its_own_tick() {
        // Negative control: moving the lie moves the named tick.
        let brief = &BRIEFS[1];
        for tick in [0, 8, 63, 119] {
            let refused = run_bundle(brief, Some(BadBuild::TamperedTick(tick)))
                .expect_err("tampered record is refused");
            assert_eq!(
                refused,
                BundleFault::Replay {
                    first_bad_tick: Some(tick)
                }
            );
        }
    }

    #[test]
    fn empty_brief_bundle_is_refused_at_the_level_gate() {
        let empty = Brief {
            id: "b00",
            text: "",
            seed: 5000,
        };
        assert_eq!(run_bundle(&empty, None), Err(BundleFault::Level));
    }

    #[test]
    fn empty_brief_is_refused_by_ab1() {
        assert!(!ab1_level_ok(""), "empty brief must not clear AB1");
        assert!(!ab1_level_ok("   \n\t"), "blank brief must not clear AB1");
    }

    #[test]
    fn vision_is_advisory_and_never_gates() {
        let report = run_report().expect("clean eval runs");
        for score in &report.briefs {
            let gated = score.ab1_ok
                && score.ab2_ok
                && score.ab3_ok
                && score.ab4_ok
                && score.keypoint_ok
                && score.replay_ok;
            assert_eq!(score.pass, gated, "{}: vision must not gate", score.id);
        }
    }

    #[test]
    fn report_files_match_the_built_report() {
        let dir = default_report_dir();
        let report = run_report().expect("clean eval runs");
        let paths = write_report_files(&dir, &report).expect("reports write");
        assert_eq!(paths.len(), 2);
        let text = std::fs::read_to_string(&paths[0]).expect("report.txt reads");
        assert_eq!(text, report.to_text());
        let tsv = std::fs::read_to_string(&paths[1]).expect("report.tsv reads");
        assert_eq!(tsv, report.to_tsv());
    }
}
