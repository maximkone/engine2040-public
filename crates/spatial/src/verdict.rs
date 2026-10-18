//! SPF9: verdict allow-list and the family learner.
//!
//! Consumer: `scene.spatial` (`allow` rows). A deliberate overlap used to stay
//! red forever; an agent can now mark a finding `intentional`,
//! `false_positive` or `deferred` with a reason, and the finding stays in the
//! answer as suppressed with that reason (never dropped), out of the red
//! count. Three such verdicts on one check x name-family cell relax the cell
//! one step, up to [`MAX_LEVEL`]; red findings never relax. The table is a
//! pure function of the rows, so the server keeps no state. Units: revisions
//! (document revision numbers), percent. Determinism: rows sorted by their
//! key before use, so the same rows in any order give one table.

use super::fold::stem;
use super::{Actor, Finding};
use std::collections::BTreeMap;

/// Most allow rows one call may carry.
pub const MAX_ALLOW: usize = 256;
/// Longest reason in chars.
pub const REASON_MAX_CHARS: usize = 200;
/// Verdicts in one cell that buy one relax step.
const VERDICTS_PER_LEVEL: u32 = 3;
/// Highest relax level.
pub const MAX_LEVEL: u8 = 3;
/// Pct a finding may carry per relax level and still be suppressed.
const PCT_PER_LEVEL: f32 = 25.0;

/// What the reviewer decided about a finding. A closed set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    /// The problem was repaired: the row never suppresses anything.
    Fixed,
    /// The finding is the design.
    Intentional,
    /// The check is wrong about this one.
    FalsePositive,
    /// Park it; revisit later.
    Deferred,
}

impl Verdict {
    /// The legal words, for refusals.
    pub const WORDS: &'static str = "fixed, intentional, false_positive, deferred";

    /// Parse a verdict word.
    pub fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "fixed" => Self::Fixed,
            "intentional" => Self::Intentional,
            "false_positive" => Self::FalsePositive,
            "deferred" => Self::Deferred,
            _ => return None,
        })
    }

    /// The word for this verdict.
    pub fn word(self) -> &'static str {
        match self {
            Self::Fixed => "fixed",
            Self::Intentional => "intentional",
            Self::FalsePositive => "false_positive",
            Self::Deferred => "deferred",
        }
    }

    /// Verdicts that teach the learner.
    fn teaches(self) -> bool {
        matches!(self, Self::Intentional | Self::FalsePositive)
    }
}

/// A refused allow row: which field and why, in words an agent can fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowError {
    /// Field name as the caller sends it.
    pub field: &'static str,
    /// What is wrong and what is legal.
    pub message: String,
}

/// One validated allow row. `b` is `u32::MAX` for a single-actor finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Allow {
    pub check: String,
    pub a: u32,
    pub b: u32,
    pub verdict: Verdict,
    pub reason: String,
    /// Revision the verdict was made at; 0 means unknown.
    pub rev: u64,
    /// Last revision the verdict holds at; 0 means it never lapses.
    pub expiry: u64,
}

impl Allow {
    /// Validate one row. The reason is required (1..=[`REASON_MAX_CHARS`]
    /// chars), the check must be a known check name, the verdict a legal word.
    pub fn new(
        check: &str,
        a: u32,
        b: Option<u32>,
        verdict: &str,
        reason: &str,
        rev: u64,
        expiry: u64,
    ) -> Result<Self, AllowError> {
        let known = super::SPATIAL_CHECKS.contains(&check) || super::OPT_IN_CHECKS.contains(&check);
        if !known {
            return Err(AllowError {
                field: "check",
                message: format!("check `{check}` is unknown; use a name from scene.spatial"),
            });
        }
        let verdict = Verdict::parse(verdict).ok_or_else(|| AllowError {
            field: "verdict",
            message: format!("verdict `{verdict}` is unknown; use {}", Verdict::WORDS),
        })?;
        let chars = reason.trim().chars().count();
        if chars == 0 || chars > REASON_MAX_CHARS {
            return Err(AllowError {
                field: "reason",
                message: format!("reason needs 1..={REASON_MAX_CHARS} chars, got {chars}"),
            });
        }
        Ok(Self {
            check: check.to_owned(),
            a,
            b: b.unwrap_or(u32::MAX),
            verdict,
            reason: reason.trim().to_owned(),
            rev,
            expiry,
        })
    }

    /// True while the row holds at document revision `now`: a `fixed` row
    /// never does; `expiry == 0` or `rev == 0` never lapses.
    fn live(&self, now: u64) -> bool {
        self.verdict != Verdict::Fixed && (self.expiry == 0 || self.rev == 0 || now <= self.expiry)
    }

    fn key(&self) -> (&str, u32, u32) {
        (&self.check, self.a, self.b)
    }
}

/// Why a finding was set aside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    /// An allow row names this finding.
    Allowed,
    /// Its cell reached this relax level.
    Learned(u8),
}

/// A finding kept visible but out of the counts.
#[derive(Debug, Clone, PartialEq)]
pub struct Suppressed {
    pub finding: Finding,
    pub verdict: Verdict,
    pub reason: String,
    pub basis: Basis,
}

/// One check x name-family cell of the learner, with its audit trail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub check: String,
    /// Name stem of the actors the verdicts were about (`crate` for `crate_7`).
    pub family: String,
    /// Live intentional and false_positive verdicts in the cell.
    pub count: u32,
    /// Verdict of the newest row (by `rev`, then key).
    pub last: Verdict,
    /// Relax step: `count / 3`, at most [`MAX_LEVEL`].
    pub level: u8,
}

/// The learner table: cells sorted by `(check, family)`, from live rows only.
pub fn learn(allow: &[Allow], actors: &[Actor], now: u64) -> Vec<Cell> {
    let family_of = |id: u32| {
        actors
            .iter()
            .find(|a| a.id == id)
            .map(|a| stem(&a.name).to_owned())
    };
    let mut rows: Vec<&Allow> = allow.iter().filter(|r| r.live(now)).collect();
    rows.sort_by(|p, q| (p.rev, p.key(), p.verdict).cmp(&(q.rev, q.key(), q.verdict)));
    let mut cells: BTreeMap<(String, String), (u32, Verdict)> = BTreeMap::new();
    for row in rows {
        let Some(family) = family_of(row.a) else {
            continue;
        };
        let cell = cells
            .entry((row.check.clone(), family))
            .or_insert((0, row.verdict));
        cell.0 += u32::from(row.verdict.teaches());
        cell.1 = row.verdict;
    }
    cells
        .into_iter()
        .filter(|(_, (count, _))| *count > 0)
        .map(|((check, family), (count, last))| Cell {
            check,
            family,
            count,
            last,
            level: u8::try_from(count / VERDICTS_PER_LEVEL)
                .unwrap_or(MAX_LEVEL)
                .min(MAX_LEVEL),
        })
        .collect()
}

/// The findings split into kept and suppressed, and the learner table.
#[derive(Debug, Clone, PartialEq)]
pub struct Judged {
    /// Findings that still count, in the caller's order.
    pub kept: Vec<Finding>,
    /// Findings set aside, in the caller's order; each says why.
    pub suppressed: Vec<Suppressed>,
    pub cells: Vec<Cell>,
}

/// Apply `allow` at document revision `now`. An allow row wins over the
/// learner; the learner only relaxes yellow findings whose pct is within
/// 25 per level, and never a red one.
pub fn judge(findings: &[Finding], actors: &[Actor], allow: &[Allow], now: u64) -> Judged {
    if allow.is_empty() {
        return Judged {
            kept: findings.to_vec(),
            suppressed: Vec::new(),
            cells: Vec::new(),
        };
    }
    let cells = learn(allow, actors, now);
    let mut out = Judged {
        kept: Vec::new(),
        suppressed: Vec::new(),
        cells,
    };
    for f in findings {
        let named = allow
            .iter()
            .filter(|r| r.live(now) && r.key() == (f.check, f.a, f.b))
            .min_by_key(|r| (r.rev, r.verdict, r.reason.as_str()));
        let learned = actors
            .iter()
            .find(|a| a.id == f.a)
            .and_then(|a| {
                out.cells
                    .iter()
                    .find(|c| c.check == f.check && c.family == stem(&a.name))
            })
            .filter(|c| f.severity != "red" && f.pct <= PCT_PER_LEVEL * f32::from(c.level));
        let set_aside = match (named, learned) {
            (Some(r), _) => Some((r.verdict, r.reason.clone(), Basis::Allowed)),
            (None, Some(c)) => Some((
                c.last,
                format!(
                    "learned: {} verdicts on {} x {}",
                    c.count, c.check, c.family
                ),
                Basis::Learned(c.level),
            )),
            (None, None) => None,
        };
        match set_aside {
            Some((verdict, reason, basis)) => out.suppressed.push(Suppressed {
                finding: f.clone(),
                verdict,
                reason,
                basis,
            }),
            None => out.kept.push(f.clone()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actor(id: u32, name: &str) -> Actor {
        Actor {
            id,
            name: name.into(),
            x: id as f32 * 3.0,
            y: 0.5,
            z: 0.0,
            sx: 1.0,
            sy: 1.0,
            sz: 1.0,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        }
    }

    fn found(check: &'static str, a: u32, b: u32, severity: &'static str, pct: f32) -> Finding {
        Finding {
            check,
            a,
            b,
            pct,
            severity,
        }
    }

    fn allow(check: &str, a: u32, b: Option<u32>, verdict: &str, rev: u64, expiry: u64) -> Allow {
        Allow::new(check, a, b, verdict, "meant to be like this", rev, expiry).unwrap()
    }

    #[test]
    fn verdict_rows_refuse_naming_the_field() {
        let new =
            |check, verdict, reason: &str| Allow::new(check, 1, Some(2), verdict, reason, 1, 0);
        assert_eq!(
            new("overlap", "intentional", "").unwrap_err().field,
            "reason"
        );
        assert_eq!(
            new("overlap", "intentional", "   ").unwrap_err().field,
            "reason"
        );
        assert_eq!(
            new("overlap", "intentional", &"x".repeat(201))
                .unwrap_err()
                .field,
            "reason"
        );
        assert!(new("overlap", "intentional", &"x".repeat(200)).is_ok());
        assert_eq!(new("overlap", "ok", "why").unwrap_err().field, "verdict");
        assert_eq!(new("nope", "fixed", "why").unwrap_err().field, "check");
        assert!(Verdict::parse("false_positive").is_some());
    }

    #[test]
    fn verdict_allow_keeps_the_finding_visible_with_its_reason() {
        let actors = [actor(1, "crate_1"), actor(2, "crate_2")];
        let overlap = found("overlap", 1, 2, "red", 40.0);
        let floating = found("floating", 1, u32::MAX, "yellow", 0.0);
        let rows = [allow("overlap", 1, Some(2), "intentional", 5, 0)];
        let judged = judge(&[overlap.clone(), floating.clone()], &actors, &rows, 9);
        assert_eq!(judged.kept, vec![floating]);
        assert_eq!(judged.suppressed.len(), 1);
        let s = &judged.suppressed[0];
        assert_eq!(
            (s.finding.clone(), s.verdict, s.basis),
            (overlap, Verdict::Intentional, Basis::Allowed)
        );
        assert_eq!(s.reason, "meant to be like this");
    }

    #[test]
    fn verdict_expired_and_fixed_rows_never_suppress() {
        let actors = [actor(1, "crate_1"), actor(2, "crate_2")];
        let overlap = found("overlap", 1, 2, "red", 40.0);
        let kept = |row: Allow, now| {
            judge(std::slice::from_ref(&overlap), &actors, &[row], now)
                .kept
                .len()
        };
        // Holds through its expiry revision, lapses after it.
        assert_eq!(
            kept(allow("overlap", 1, Some(2), "intentional", 5, 8), 8),
            0
        );
        assert_eq!(
            kept(allow("overlap", 1, Some(2), "intentional", 5, 8), 9),
            1
        );
        // rev 0 and expiry 0 never lapse.
        assert_eq!(kept(allow("overlap", 1, Some(2), "deferred", 0, 8), 99), 0);
        assert_eq!(kept(allow("overlap", 1, Some(2), "deferred", 5, 0), 99), 0);
        // A fixed verdict is a note, not a suppression.
        assert_eq!(kept(allow("overlap", 1, Some(2), "fixed", 5, 0), 9), 1);
    }

    #[test]
    fn verdict_learner_relaxes_after_three_and_caps_at_level_3() {
        let actors: Vec<Actor> = (1..=14).map(|i| actor(i, &format!("lamp_{i}"))).collect();
        let rows = |n: u32| -> Vec<Allow> {
            (1..=n)
                .map(|i| allow("floating", i, None, "false_positive", i.into(), 0))
                .collect()
        };
        let level = |n| learn(&rows(n), &actors, 1).first().map_or(0, |c| c.level);
        assert_eq!(
            [level(2), level(3), level(6), level(9), level(12)],
            [0, 1, 2, 3, 3]
        );
        let cells = learn(&rows(12), &actors, 1);
        assert_eq!(cells.len(), 1);
        assert_eq!(
            (
                cells[0].check.as_str(),
                cells[0].family.as_str(),
                cells[0].count,
                cells[0].last
            ),
            ("floating", "lamp", 12, Verdict::FalsePositive)
        );
        // Deferred and fixed rows teach nothing; expired rows are not counted.
        let mut mixed = rows(2);
        mixed.push(allow("floating", 3, None, "deferred", 3, 0));
        mixed.push(allow("floating", 4, None, "intentional", 4, 4));
        assert_eq!(learn(&mixed, &actors, 5).first().map(|c| c.level), Some(0));
        assert_eq!(
            learn(&mixed, &actors, 4)
                .first()
                .map(|c| (c.count, c.level)),
            Some((3, 1))
        );
    }

    #[test]
    fn verdict_learned_level_relaxes_yellow_by_pct_and_never_red() {
        let actors: Vec<Actor> = (1..=8).map(|i| actor(i, &format!("lamp_{i}"))).collect();
        let rows: Vec<Allow> = (1..=3)
            .map(|i| allow("overlap", i, Some(i + 1), "intentional", i.into(), 0))
            .collect();
        let near = found("overlap", 5, 6, "yellow", 20.0);
        let far = found("overlap", 5, 7, "yellow", 30.0);
        let red = found("overlap", 5, 8, "red", 10.0);
        let other = found("floating", 5, u32::MAX, "yellow", 0.0);
        let judged = judge(
            &[near.clone(), far.clone(), red.clone(), other.clone()],
            &actors,
            &rows,
            1,
        );
        let learned: Vec<&Finding> = judged.suppressed.iter().map(|s| &s.finding).collect();
        assert_eq!(
            learned,
            vec![&near],
            "level 1 relaxes pct <= 25 and nothing else"
        );
        assert_eq!(judged.suppressed[0].basis, Basis::Learned(1));
        assert_eq!(judged.kept, vec![far, red, other]);
    }

    #[test]
    fn verdict_same_rows_in_any_order_give_one_table() {
        let actors: Vec<Actor> = (1..=10).map(|i| actor(i, &format!("lamp_{i}"))).collect();
        let rows: Vec<Allow> = (1..=9)
            .map(|i| {
                allow(
                    "floating",
                    i,
                    None,
                    if i % 2 == 0 {
                        "intentional"
                    } else {
                        "false_positive"
                    },
                    (i % 4).into(),
                    0,
                )
            })
            .collect();
        let want = learn(&rows, &actors, 1);
        let mut shuffled = rows.clone();
        for round in 0..20 {
            shuffled.rotate_left(round % 9 + 1);
            if round % 3 == 0 {
                shuffled.reverse();
            }
            assert_eq!(learn(&shuffled, &actors, 1), want, "round {round}");
            let f = [found("floating", 1, u32::MAX, "yellow", 0.0)];
            assert_eq!(
                judge(&f, &actors, &shuffled, 1),
                judge(&f, &actors, &rows, 1)
            );
        }
    }
}
