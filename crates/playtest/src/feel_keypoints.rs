//! Feel keypoints (GP7): VK1 extended to the presentation feel.
//!
//! A keypoint is one feel claim as data in `fixtures/keypoints/feel/*.txt`
//! (one `k=v;k=v` line each): hit-stop length, shake amplitude, shake size
//! separation, cancel tightness and silhouette readability. The runner plays
//! each claim against the shipped feel code (`engine_combat::feel_flow`,
//! `shake_stage`, `camera_framing`, the jab row) through a [`Rig`], and a
//! seeded [`FeelBug`] corrupts the rig the way the real defect would so the
//! paired keypoint, and only that one, must fail.
//!
//! Consumer: the proof harness calls [`run_suite`] and [`run_ablation`]; the
//! GP6 gold test reads [`rubric`]. Units: ticks at 60 Hz, offsets in milli
//! pixels, shares in permille. Determinism: integer math over pure feel code;
//! the same files twice give the same report. Errors: [`FeelKeypointError`]
//! for bad files; a failed claim is a reason string inside a passing report.
//! Budget: milliseconds; nothing here touches the fight tick or its hash.

use std::path::{Path, PathBuf};

use engine_combat::camera_framing::{frame_melee, Shoulder, READABLE_PERMILLE};
use engine_combat::feel_flow::{Effect, FeelFlow, Trigger, V1_EFFECTS};
use engine_combat::fight::FightEvent;
use engine_combat::shake_stage;
use engine_combat::MOVE_JAB;

use crate::parse::{kv_expect_keys, kv_get, split_kv, ParseError};

/// Feel keypoints shipped in `fixtures/keypoints/feel/`.
pub const FEEL_KEYPOINT_COUNT: usize = 5;
/// Largest accepted spec file in bytes.
pub const MAX_SPEC_BYTES: usize = 2048;
/// Ticks the shake claims scan, past any shipped shake.
const SCAN_TICKS: u32 = 60;
/// Horizontal half field of view the shipped camera frames with (tan, permille).
const FOV_TAN_PERMILLE: u32 = 700;

/// Failures reading the feel specs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FeelKeypointError {
    /// The spec directory is missing or unreadable.
    DirUnreadable(String),
    /// One file failed to read or parse; stem plus reason.
    BadSpec {
        /// File stem.
        file: String,
        /// What was wrong.
        reason: String,
    },
}

impl core::fmt::Display for FeelKeypointError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::DirUnreadable(reason) => write!(f, "feel keypoint dir unreadable: {reason}"),
            Self::BadSpec { file, reason } => write!(f, "bad feel spec `{file}`: {reason}"),
        }
    }
}

impl std::error::Error for FeelKeypointError {}

/// What one feel keypoint checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    /// A hit holds the presented clock at or under `scale_permille` for exactly
    /// `hold_ticks` ticks.
    HitStop {
        /// Exact frozen ticks.
        hold_ticks: u32,
        /// Clock rate at or under which a tick counts as frozen.
        scale_permille: u32,
    },
    /// A hit shake peaks in `min_milli..=max_milli` and is gone after `ticks`.
    ShakePeak {
        /// Lowest accepted peak in milli pixels.
        min_milli: i32,
        /// Highest accepted peak in milli pixels.
        max_milli: i32,
        /// Ticks the shake runs.
        ticks: u32,
    },
    /// The small trauma shakes at most `max_permille` of the big one.
    ShakeSeparation {
        /// Small (jab) trauma in permille.
        small: u32,
        /// Big hit trauma in permille.
        big: u32,
        /// Largest accepted ratio of peaks, in permille.
        max_permille: u32,
    },
    /// The jab cancels into the cross from `first_frame` and not before.
    Cancel {
        /// First frame the cancel opens on.
        first_frame: u32,
    },
    /// Melee framing from 1 to 6 meters reads and holds both fighters.
    Readability {
        /// Least share of poses (percent) at or over the readable bar.
        min_share_percent: u32,
    },
}

/// One feel claim as data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeelSpec {
    /// File stem and registry id.
    pub id: String,
    /// Human claim, for reasons only.
    pub claim: String,
    /// What is checked.
    pub check: Check,
}

/// The code under test, with the knobs a seeded bug turns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rig {
    /// Registered (trigger, effect) rows the flow is built from.
    pub effects: Vec<(Trigger, Effect)>,
    /// Trauma floor applied to every shake input (0 is clean).
    pub trauma_floor: u32,
    /// Frames added before the cancel window is queried (0 is clean).
    pub cancel_slack: u32,
    /// Half field of view the framing check uses, tan permille.
    pub fov_tan_permille: u32,
}

impl Rig {
    /// The shipped feel: nothing corrupted.
    #[must_use]
    pub fn clean() -> Self {
        Self {
            effects: V1_EFFECTS.to_vec(),
            trauma_floor: 0,
            cancel_slack: 0,
            fov_tan_permille: FOV_TAN_PERMILLE,
        }
    }

    fn flow(&self) -> FeelFlow {
        let mut flow = FeelFlow::new();
        for &(trigger, effect) in &self.effects {
            // The rows come from a shipped 7-row table; a bug only edits them.
            let _ = flow.register(trigger, effect);
        }
        flow
    }
}

/// One seeded feel bug: the keypoint it must fail, and the rig fault.
#[derive(Clone, Copy)]
pub struct FeelBug {
    /// Stable bug id for reports.
    pub id: &'static str,
    /// Bug class under test.
    pub class: &'static str,
    /// Paired keypoint id; exactly this keypoint must fail.
    pub target: &'static str,
    /// Fault applied to the rig.
    pub apply: fn(&mut Rig),
}

fn drop_hit_stop(rig: &mut Rig) {
    rig.effects
        .retain(|&(t, e)| !(t == Trigger::Hit && e == Effect::StopFrame));
}

fn quadruple_shake(rig: &mut Rig) {
    for (_, effect) in &mut rig.effects {
        if let Effect::Shake { power_permille, .. } = effect {
            *power_permille *= 4;
        }
    }
}

fn floor_trauma(rig: &mut Rig) {
    rig.trauma_floor = 600;
}

fn early_cancel(rig: &mut Rig) {
    rig.cancel_slack = 2;
}

fn tight_camera(rig: &mut Rig) {
    rig.fov_tan_permille = 250;
}

/// The seeded feel bugs: a missing hit-stop, a shake four times too strong,
/// a trauma floor that makes a jab shake like a big hit, a cancel window that
/// opens two frames early, and a camera that cannot hold both fighters.
pub const BUG_SEEDS: [FeelBug; 5] = [
    FeelBug {
        id: "feelbug-hitstop-missing",
        class: "missing hit-stop",
        target: "fk1-hitstop-holds-four",
        apply: drop_hit_stop,
    },
    FeelBug {
        id: "feelbug-shake-quadrupled",
        class: "shake amplitude",
        target: "fk2-shake-peak-capped",
        apply: quadruple_shake,
    },
    FeelBug {
        id: "feelbug-jab-shakes-like-super",
        class: "shake separation",
        target: "fk3-jab-shake-small",
        apply: floor_trauma,
    },
    FeelBug {
        id: "feelbug-cancel-early",
        class: "cancel tightness",
        target: "fk4-cancel-opens-on-startup",
        apply: early_cancel,
    },
    FeelBug {
        id: "feelbug-camera-too-tight",
        class: "readability",
        target: "fk5-melee-reads",
        apply: tight_camera,
    },
];

// ---- loading -------------------------------------------------------------

/// Directory holding the feel specs, resolved from the crate location.
#[must_use]
pub fn spec_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/keypoints/feel")
}

fn number(pairs: &[(String, String)], key: &str) -> Result<u32, ParseError> {
    kv_get(pairs, key)?
        .parse()
        .map_err(|_| ParseError::WrongType {
            field: key.to_owned(),
            want: "an unsigned integer",
        })
}

fn parse_spec(text: &str, stem: &str) -> Result<FeelSpec, ParseError> {
    let pairs = split_kv(text.trim())?;
    let id = kv_get(&pairs, "id")?;
    if id != stem {
        return Err(ParseError::Mismatch {
            what: format!("id `{id}` must equal the file stem `{stem}`"),
        });
    }
    let claim = kv_get(&pairs, "claim")?.to_owned();
    let (check, keys): (Check, &[&str]) = match kv_get(&pairs, "kind")? {
        "hitstop" => (
            Check::HitStop {
                hold_ticks: number(&pairs, "hold_ticks")?,
                scale_permille: number(&pairs, "scale_permille")?,
            },
            &["hold_ticks", "scale_permille"],
        ),
        "shake_peak" => (
            Check::ShakePeak {
                min_milli: i32::try_from(number(&pairs, "min_milli")?).unwrap_or(i32::MAX),
                max_milli: i32::try_from(number(&pairs, "max_milli")?).unwrap_or(i32::MAX),
                ticks: number(&pairs, "ticks")?,
            },
            &["min_milli", "max_milli", "ticks"],
        ),
        "shake_separation" => (
            Check::ShakeSeparation {
                small: number(&pairs, "small")?,
                big: number(&pairs, "big")?,
                max_permille: number(&pairs, "max_permille")?,
            },
            &["small", "big", "max_permille"],
        ),
        "cancel" => (
            Check::Cancel {
                first_frame: number(&pairs, "first_frame")?,
            },
            &["first_frame"],
        ),
        "readability" => (
            Check::Readability {
                min_share_percent: number(&pairs, "min_share_percent")?,
            },
            &["min_share_percent"],
        ),
        other => {
            return Err(ParseError::Unknown {
                what: format!(
                "kind `{other}`; legal: hitstop, shake_peak, shake_separation, cancel, readability"
            ),
            })
        }
    };
    let mut want = vec!["id", "claim", "kind"];
    want.extend_from_slice(keys);
    kv_expect_keys(&pairs, &want)?;
    Ok(FeelSpec {
        id: id.to_owned(),
        claim,
        check,
    })
}

/// Load every `*.txt` spec in [`spec_dir`], sorted by file stem.
///
/// # Errors
///
/// [`FeelKeypointError`] when the directory or a file is unreadable or bad.
pub fn load_specs() -> Result<Vec<FeelSpec>, FeelKeypointError> {
    let dir = spec_dir();
    let entries =
        std::fs::read_dir(&dir).map_err(|e| FeelKeypointError::DirUnreadable(e.to_string()))?;
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| FeelKeypointError::DirUnreadable(e.to_string()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".txt") {
            names.push(name);
        }
    }
    names.sort();
    let mut specs = Vec::with_capacity(names.len());
    for name in &names {
        let stem = name.trim_end_matches(".txt").to_owned();
        let bad = |reason: String| FeelKeypointError::BadSpec {
            file: stem.clone(),
            reason,
        };
        let bytes = std::fs::read(dir.join(name)).map_err(|e| bad(e.to_string()))?;
        if bytes.len() > MAX_SPEC_BYTES {
            return Err(bad(format!("file over {MAX_SPEC_BYTES} bytes")));
        }
        let text = String::from_utf8(bytes).map_err(|e| bad(e.to_string()))?;
        specs.push(parse_spec(&text, &stem).map_err(|e| bad(e.to_string()))?);
    }
    Ok(specs)
}

// ---- running -------------------------------------------------------------

fn hit_event() -> [FightEvent; 1] {
    [FightEvent::Hit {
        attacker: 0,
        victim: 1,
        damage: 100,
    }]
}

fn peak_of(trauma: u32, floor: u32) -> i32 {
    let trauma = trauma.max(floor);
    (0..600)
        .map(|t| {
            let s = shake_stage::sample(trauma, t);
            s.x_permille.abs().max(s.y_permille.abs())
        })
        .max()
        .unwrap_or(0)
}

/// Why a claim did not hold, as one line of text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure(pub String);

impl core::fmt::Display for Failure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Run one claim against the rig; `Err` carries the reason it failed.
fn run_check(check: Check, rig: &Rig) -> Result<(), Failure> {
    match check {
        Check::HitStop {
            hold_ticks,
            scale_permille,
        } => {
            let mut flow = rig.flow();
            let events = hit_event();
            let mut frozen = Vec::new();
            for tick in 0..hold_ticks + 6 {
                let ev: &[FightEvent] = if tick == 0 { &events } else { &[] };
                let (frame, _) = flow.step(ev, tick);
                frozen.push(frame.time_permille <= scale_permille);
            }
            let held = frozen.iter().filter(|f| **f).count();
            let contiguous = frozen.iter().take(held).all(|f| *f);
            if held == usize::try_from(hold_ticks).unwrap_or(0) && contiguous {
                Ok(())
            } else {
                Err(Failure(format!(
                    "clock held {held} ticks, claim wants {hold_ticks}"
                )))
            }
        }
        Check::ShakePeak {
            min_milli,
            max_milli,
            ticks,
        } => {
            let mut flow = rig.flow();
            let events = hit_event();
            let (mut peak, mut tail) = (0_i32, 0_i32);
            for tick in 0..ticks + SCAN_TICKS {
                let ev: &[FightEvent] = if tick == 0 { &events } else { &[] };
                let (frame, _) = flow.step(ev, tick);
                let size = frame.shake_milli.0.abs().max(frame.shake_milli.1.abs());
                peak = peak.max(size);
                if tick >= ticks {
                    tail = tail.max(size);
                }
            }
            if peak < min_milli || peak > max_milli {
                Err(Failure(format!(
                    "shake peak {peak} outside {min_milli}..={max_milli}"
                )))
            } else if tail != 0 {
                Err(Failure(format!(
                    "shake still moving {tail} after {ticks} ticks"
                )))
            } else {
                Ok(())
            }
        }
        Check::ShakeSeparation {
            small,
            big,
            max_permille,
        } => {
            let (s, b) = (
                peak_of(small, rig.trauma_floor),
                peak_of(big, rig.trauma_floor),
            );
            let ratio = if b == 0 { 0 } else { s * 1000 / b };
            if ratio <= i32::try_from(max_permille).unwrap_or(i32::MAX) {
                Ok(())
            } else {
                Err(Failure(format!(
                    "small shake is {ratio} permille of big, limit {max_permille}"
                )))
            }
        }
        Check::Cancel { first_frame } => {
            let before =
                MOVE_JAB.can_cancel("cross", first_frame.saturating_sub(1) + rig.cancel_slack);
            let at = MOVE_JAB.can_cancel("cross", first_frame + rig.cancel_slack);
            let wrong = MOVE_JAB.can_cancel("uppercut", first_frame + rig.cancel_slack);
            if before {
                Err(Failure(format!(
                    "cancel open at frame {}",
                    first_frame.saturating_sub(1)
                )))
            } else if !at {
                Err(Failure(format!("cancel closed at frame {first_frame}")))
            } else if wrong {
                Err(Failure(
                    "cancel opens into a move not on the list".to_owned(),
                ))
            } else {
                Ok(())
            }
        }
        Check::Readability { min_share_percent } => {
            let headings: [(i64, i64); 8] = [
                (1000, 0),
                (707, 707),
                (0, 1000),
                (-707, 707),
                (-1000, 0),
                (-707, -707),
                (0, -1000),
                (707, -707),
            ];
            let (mut total, mut readable, mut framed) = (0_u32, 0_u32, 0_u32);
            for dist in (1000..=6000_i64).step_by(250) {
                for (hx, hz) in headings {
                    for shoulder in [Shoulder::Left, Shoulder::Right] {
                        let d = [
                            i32::try_from(hx * dist / 1000).unwrap_or(0),
                            i32::try_from(hz * dist / 1000).unwrap_or(0),
                        ];
                        let f = frame_melee([0, 0], d, shoulder, rig.fov_tan_permille);
                        total += 1;
                        readable += u32::from(f.readability_permille >= READABLE_PERMILLE);
                        framed += u32::from(f.both_in_frame);
                    }
                }
            }
            if framed != total {
                Err(Failure(format!(
                    "{} of {total} poses lose a fighter out of frame",
                    total - framed
                )))
            } else if readable * 100 < total * min_share_percent {
                Err(Failure(format!(
                    "only {readable} of {total} poses read, want {min_share_percent} percent"
                )))
            } else {
                Ok(())
            }
        }
    }
}

/// One keypoint outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeelResult {
    /// Keypoint id.
    pub id: String,
    /// `Ok` when the claim held, else the reason it did not.
    pub outcome: Result<(), Failure>,
}

/// A whole-suite run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeelReport {
    /// One result per spec, in id order.
    pub results: Vec<FeelResult>,
}

impl FeelReport {
    /// Keypoints that held.
    #[must_use]
    pub fn passed(&self) -> usize {
        self.results.iter().filter(|r| r.outcome.is_ok()).count()
    }

    /// Ids of keypoints that failed.
    #[must_use]
    pub fn failed_ids(&self) -> Vec<&str> {
        self.results
            .iter()
            .filter(|r| r.outcome.is_err())
            .map(|r| r.id.as_str())
            .collect()
    }
}

/// Run the shipped specs against a rig.
///
/// # Errors
///
/// [`FeelKeypointError`] when the specs cannot load.
pub fn run_suite_with(rig: &Rig) -> Result<FeelReport, FeelKeypointError> {
    let results = load_specs()?
        .into_iter()
        .map(|spec| FeelResult {
            outcome: run_check(spec.check, rig),
            id: spec.id,
        })
        .collect();
    Ok(FeelReport { results })
}

/// Run the shipped specs against the clean rig.
///
/// # Errors
///
/// [`FeelKeypointError`] when the specs cannot load.
pub fn run_suite() -> Result<FeelReport, FeelKeypointError> {
    run_suite_with(&Rig::clean())
}

/// Clean-suite pass counts `(passed, total)`: the VK1 rubric the GP6 feel
/// scorer reads.
///
/// # Errors
///
/// [`FeelKeypointError`] when the specs cannot load.
pub fn rubric() -> Result<(u32, u32), FeelKeypointError> {
    let report = run_suite()?;
    Ok((
        u32::try_from(report.passed()).unwrap_or(0),
        u32::try_from(report.results.len()).unwrap_or(0),
    ))
}

/// One seeded bug's verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AblationRow {
    /// Bug id.
    pub bug: &'static str,
    /// Keypoint it must fail.
    pub target: &'static str,
    /// Every keypoint that failed under the bug.
    pub failed: Vec<String>,
}

impl AblationRow {
    /// True when exactly the paired keypoint failed.
    #[must_use]
    pub fn caught(&self) -> bool {
        self.failed.len() == 1 && self.failed[0] == self.target
    }
}

/// Run every seeded bug and report which keypoints failed under each.
///
/// # Errors
///
/// [`FeelKeypointError`] when the specs cannot load.
pub fn run_ablation() -> Result<Vec<AblationRow>, FeelKeypointError> {
    BUG_SEEDS
        .iter()
        .map(|bug| {
            let mut rig = Rig::clean();
            (bug.apply)(&mut rig);
            let report = run_suite_with(&rig)?;
            Ok(AblationRow {
                bug: bug.id,
                target: bug.target,
                failed: report.failed_ids().into_iter().map(str::to_owned).collect(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_combat::feel::{balance_over_200, FEEL_V1};
    use engine_combat::TABLES_V1;

    #[test]
    fn five_specs_load_and_the_clean_rig_passes_them_all() {
        let specs = load_specs().expect("specs load");
        assert_eq!(specs.len(), FEEL_KEYPOINT_COUNT);
        let report = run_suite().expect("suite runs");
        assert_eq!(report.failed_ids(), Vec::<&str>::new(), "{report:?}");
        assert_eq!(rubric().unwrap(), (5, 5));
        assert_eq!(report, run_suite().unwrap(), "suite replays identically");
    }

    #[test]
    fn every_seeded_bug_fails_exactly_its_keypoint() {
        let rows = run_ablation().expect("ablation runs");
        assert_eq!(rows.len(), BUG_SEEDS.len());
        let caught = rows.iter().filter(|r| r.caught()).count();
        assert!(
            caught >= 4,
            "4 of 5 bugs must fail exactly their keypoint: {rows:?}"
        );
        for row in &rows {
            assert!(row.caught(), "{row:?}");
        }
    }

    #[test]
    fn bad_specs_fail_typed() {
        for (text, want) in [
            ("id=x;claim=c;kind=nope", "kind `nope`"),
            (
                "id=y;claim=c;kind=cancel;first_frame=4",
                "must equal the file stem",
            ),
            ("id=x;claim=c;kind=cancel", "first_frame"),
            (
                "id=x;claim=c;kind=cancel;first_frame=4;extra=1",
                "id,claim,kind,first_frame",
            ),
            (
                "id=x;claim=c;kind=cancel;first_frame=-1",
                "unsigned integer",
            ),
        ] {
            let err = parse_spec(text, "x").unwrap_err().to_string();
            assert!(err.contains(want), "{text}: {err}");
        }
    }

    #[test]
    fn feel_gold_inputs_are_the_real_pq2_and_vk1_numbers() {
        // GP6: `fixtures/feel-gold/` stores the PQ2 win share and the VK1 rubric
        // beside the verdict the perf scorer reproduces; here the stored inputs
        // are held to the live balance survey and the live feel suite.
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/feel-gold/feel-gold.txt");
        let gold = std::fs::read_to_string(path).expect("gold file ships");
        let win = u32::from(balance_over_200(&FEEL_V1.tables(TABLES_V1)).share_a_permille());
        let (passed, total) = rubric().expect("suite runs");
        let mut rows = 0;
        for line in gold
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        {
            let head = line.split('|').next().unwrap_or_default();
            let words: Vec<u32> = head
                .split_whitespace()
                .skip(1)
                .map(|w| w.parse().expect("number"))
                .collect();
            assert_eq!(words, [win, passed, total], "{line}");
            rows += 1;
        }
        assert_eq!(rows, 3, "one row per feel domain");
    }
}
