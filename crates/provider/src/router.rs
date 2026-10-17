//! router: pick a model per task class from the eval scorecard.
//!
//! The scorecard has one row per (task class, model): quality, p95 latency
//! and cost per request. [`Router::route`] takes an [`Ask`] (class, deadline,
//! cost cap) and walks the rows best quality first, local before remote on a
//! tie, then cheaper, then by label. A model that is down, slower than the
//! deadline, or over the cost cap is skipped and the skip is recorded. When
//! no model is left the answer is [`Choice::Scripted`], the always-available
//! last rung. A cost cap is never a silent downgrade: when the only models
//! left were skipped for cost, the route refuses with
//! [`RouterError::CostCap`].
//!
//! Every call appends a [`Decision`] to the record, refusals included. The
//! record holds the inputs (ask, models down), so [`replay`] re-derives each
//! decision from it and finds the first one that differs. Nothing here reads
//! a clock or a socket: the same scorecard and the same asks give the same
//! record.
//!
//! Units: milliseconds, permille, cost in millionths of the session
//! currency. Errors: [`RouterError`].

use std::collections::BTreeSet;

use super::protocol::{AiRequest, TaskKind};

/// What kind of work a request is, as the scorecard groups it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum TaskClass {
    /// A short combat bark or taunt.
    Bark,
    /// Announcer or character dialogue.
    Dialogue,
    /// A level brief.
    LevelBrief,
    /// An encounter design.
    Encounter,
    /// A review, diagnosis or repair of existing content.
    Review,
}

impl TaskClass {
    /// The class a request of `kind` belongs to. `Encounter` has no
    /// [`TaskKind`] of its own: callers that build encounters name it in
    /// their [`Ask`].
    #[must_use]
    pub fn of(kind: TaskKind) -> Self {
        match kind {
            TaskKind::Taunt => Self::Bark,
            TaskKind::Announce => Self::Dialogue,
            TaskKind::Propose => Self::LevelBrief,
            TaskKind::Inspect | TaskKind::Diagnose | TaskKind::Repair => Self::Review,
        }
    }
}

/// Where a model runs. Local sorts first, so it wins ties.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Tier {
    /// On this machine; costs nothing per request.
    Local,
    /// A configured remote host.
    Remote,
}

/// One scorecard row: how a model did on a class.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ScoreRow {
    /// Class the row scores.
    pub class: TaskClass,
    /// Model label, the name callers map to an adapter.
    pub label: String,
    /// Where the model runs.
    pub tier: Tier,
    /// Eval quality, permille.
    pub quality_permille: u32,
    /// Measured p95 latency, ms.
    pub p95_ms: u32,
    /// Cost of one request, millionths.
    pub cost_micro: u64,
}

/// One routing request.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ask {
    /// Class of the work.
    pub class: TaskClass,
    /// The reply must land inside this, ms; a model whose p95 is above it is
    /// skipped.
    pub deadline_ms: u32,
    /// Most this request may cost, millionths.
    pub cost_cap_micro: u64,
}

impl Ask {
    /// The ask a request makes: its kind's class and its own time budget,
    /// under `cost_cap_micro`.
    #[must_use]
    pub fn for_request(request: &AiRequest, cost_cap_micro: u64) -> Self {
        Self {
            class: TaskClass::of(request.kind),
            deadline_ms: u32::try_from(request.budget.max_ms).unwrap_or(u32::MAX),
            cost_cap_micro,
        }
    }
}

/// Why a model was passed over.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Skip {
    /// Marked down (an outage).
    Outage,
    /// Its p95 is over the ask's deadline.
    TooSlow,
    /// Its cost is over the request cap or the session's remaining budget.
    CostCap,
}

/// Where the request goes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Choice {
    /// The named model.
    Model(String),
    /// The scripted fallback, the last rung.
    Scripted,
}

/// A refused route.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RouterError {
    /// Every model that was up and fast enough costs more than the cap.
    CostCap {
        /// The best such model.
        label: String,
        /// What it costs, millionths.
        cost_micro: u64,
        /// The cap that applied (the request's, or what the session has left
        /// if lower), millionths.
        cap_micro: u64,
    },
}

impl core::fmt::Display for RouterError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let Self::CostCap {
            label,
            cost_micro,
            cap_micro,
        } = self;
        write!(
            f,
            "route refused: {label} costs {cost_micro} over the cap {cap_micro}"
        )
    }
}

impl std::error::Error for RouterError {}

/// One recorded routing decision: the inputs and what came of them.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Decision {
    /// Position in the record, from 0.
    pub seq: u32,
    /// The ask.
    pub ask: Ask,
    /// Models marked down at the time, sorted.
    pub down: Vec<String>,
    /// Models passed over, in the order they were considered.
    pub skipped: Vec<(String, Skip)>,
    /// The route, or the refusal.
    pub result: Result<Choice, RouterError>,
}

/// The router: a scorecard, a session budget and the record.
#[derive(Clone, Debug)]
pub struct Router {
    rows: Vec<ScoreRow>,
    session_cap_micro: u64,
    spent_micro: u64,
    down: BTreeSet<String>,
    record: Vec<Decision>,
}

impl Router {
    /// A router over `rows` that may spend `session_cap_micro` in all.
    #[must_use]
    pub fn new(rows: Vec<ScoreRow>, session_cap_micro: u64) -> Self {
        Self {
            rows,
            session_cap_micro,
            spent_micro: 0,
            down: BTreeSet::new(),
            record: Vec::new(),
        }
    }

    /// Mark a model down (an outage) until [`Router::mark_up`].
    pub fn mark_down(&mut self, label: &str) {
        self.down.insert(label.to_owned());
    }

    /// Mark a model up again.
    pub fn mark_up(&mut self, label: &str) {
        self.down.remove(label);
    }

    /// Every decision so far, oldest first.
    #[must_use]
    pub fn record(&self) -> &[Decision] {
        &self.record
    }

    /// Spent so far, millionths.
    #[must_use]
    pub fn spent_micro(&self) -> u64 {
        self.spent_micro
    }

    /// Route `ask` and record the decision.
    /// Errors: [`RouterError::CostCap`].
    pub fn route(&mut self, ask: Ask) -> Result<Choice, RouterError> {
        let cap = ask
            .cost_cap_micro
            .min(self.session_cap_micro.saturating_sub(self.spent_micro));
        let mut rows: Vec<&ScoreRow> = self.rows.iter().filter(|r| r.class == ask.class).collect();
        // Best quality first, then local, then cheaper, then by label.
        rows.sort_by(|a, b| {
            (b.quality_permille, a.tier, a.cost_micro, &a.label).cmp(&(
                a.quality_permille,
                b.tier,
                b.cost_micro,
                &b.label,
            ))
        });
        let mut skipped = Vec::new();
        let mut over_cap: Option<&ScoreRow> = None;
        let mut chosen: Option<&ScoreRow> = None;
        for row in rows {
            let skip = if self.down.contains(&row.label) {
                Some(Skip::Outage)
            } else if row.p95_ms > ask.deadline_ms {
                Some(Skip::TooSlow)
            } else if row.cost_micro > cap {
                Some(Skip::CostCap)
            } else {
                None
            };
            match skip {
                None => {
                    chosen = Some(row);
                    break;
                }
                Some(Skip::CostCap) => {
                    over_cap = over_cap.or(Some(row));
                    skipped.push((row.label.clone(), Skip::CostCap));
                }
                Some(other) => skipped.push((row.label.clone(), other)),
            }
        }
        let result = match (chosen, over_cap) {
            (Some(row), _) => {
                self.spent_micro = self.spent_micro.saturating_add(row.cost_micro);
                Ok(Choice::Model(row.label.clone()))
            }
            (None, Some(row)) => Err(RouterError::CostCap {
                label: row.label.clone(),
                cost_micro: row.cost_micro,
                cap_micro: cap,
            }),
            (None, None) => Ok(Choice::Scripted),
        };
        self.record.push(Decision {
            seq: u32::try_from(self.record.len()).unwrap_or(u32::MAX),
            ask,
            down: self.down.iter().cloned().collect(),
            skipped,
            result: result.clone(),
        });
        result
    }
}

/// Re-derive `record` from its own inputs on a fresh router over `rows` with
/// the same session budget. `Ok` when every decision comes out the same, else
/// the `seq` of the first that does not.
pub fn replay(rows: &[ScoreRow], session_cap_micro: u64, record: &[Decision]) -> Result<(), u32> {
    let mut router = Router::new(rows.to_vec(), session_cap_micro);
    for decision in record {
        router.down = decision.down.iter().cloned().collect();
        let _ = router.route(decision.ask);
        if router.record.last() != Some(decision) {
            return Err(decision.seq);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(class: TaskClass, label: &str, tier: Tier, q: u32, p95: u32, cost: u64) -> ScoreRow {
        ScoreRow {
            class,
            label: label.to_owned(),
            tier,
            quality_permille: q,
            p95_ms: p95,
            cost_micro: cost,
        }
    }

    fn card() -> Vec<ScoreRow> {
        use TaskClass::{Bark, Dialogue};
        vec![
            row(Bark, "big-remote", Tier::Remote, 900, 300, 50),
            row(Bark, "local-7b", Tier::Local, 700, 120, 0),
            row(Bark, "local-3b", Tier::Local, 600, 60, 0),
            row(Dialogue, "dlg-remote", Tier::Remote, 850, 400, 80),
        ]
    }

    fn bark(deadline_ms: u32) -> Ask {
        Ask {
            class: TaskClass::Bark,
            deadline_ms,
            cost_cap_micro: 100,
        }
    }

    fn model(label: &str) -> Result<Choice, RouterError> {
        Ok(Choice::Model(label.to_owned()))
    }

    /// F2P: there was no router. A seeded outage of the top model reroutes to
    /// the next one that still meets the deadline, and the record says why.
    #[test]
    fn an_outage_of_the_top_model_reroutes_inside_the_deadline() {
        let mut router = Router::new(card(), 1_000);
        assert_eq!(router.route(bark(400)), model("big-remote"));
        router.mark_down("big-remote");
        assert_eq!(router.route(bark(400)), model("local-7b"));
        let rerouted = &router.record()[1];
        assert_eq!(
            rerouted.skipped,
            vec![(String::from("big-remote"), Skip::Outage)]
        );
        assert_eq!(rerouted.down, vec![String::from("big-remote")]);
        let picked = card()
            .into_iter()
            .find(|r| r.label == "local-7b")
            .expect("row");
        assert!(picked.p95_ms <= 400, "the reroute meets the deadline");
        router.mark_up("big-remote");
        assert_eq!(router.route(bark(400)), model("big-remote"));
    }

    #[test]
    fn a_tight_deadline_skips_slow_models_and_the_floor_is_scripted() {
        let mut router = Router::new(card(), 1_000);
        assert_eq!(router.route(bark(100)), model("local-3b"));
        assert_eq!(
            router.record()[0].skipped,
            vec![
                (String::from("big-remote"), Skip::TooSlow),
                (String::from("local-7b"), Skip::TooSlow),
            ]
        );
        assert_eq!(router.route(bark(10)), Ok(Choice::Scripted));
        for label in ["big-remote", "local-7b", "local-3b"] {
            router.mark_down(label);
        }
        assert_eq!(router.route(bark(400)), Ok(Choice::Scripted));
    }

    #[test]
    fn a_cost_cap_refuses_loudly_unless_a_free_model_fits() {
        let mut router = Router::new(card(), 1_000);
        let dialogue = |cap| Ask {
            class: TaskClass::Dialogue,
            deadline_ms: 500,
            cost_cap_micro: cap,
        };
        let refused = router.route(dialogue(50));
        assert_eq!(
            refused,
            Err(RouterError::CostCap {
                label: String::from("dlg-remote"),
                cost_micro: 80,
                cap_micro: 50,
            })
        );
        assert!(refused
            .expect_err("refused")
            .to_string()
            .contains("dlg-remote"));
        assert_eq!(router.spent_micro(), 0, "a refusal spends nothing");
        assert_eq!(router.record().len(), 1, "a refusal is recorded");

        // A free local model under the same tight cap is just chosen.
        let tight = Ask {
            cost_cap_micro: 10,
            ..bark(400)
        };
        assert_eq!(router.route(tight), model("local-7b"));
        assert_eq!(
            router.record()[1].skipped,
            vec![(String::from("big-remote"), Skip::CostCap)]
        );
    }

    #[test]
    fn the_session_budget_runs_out_across_requests() {
        let mut router = Router::new(card(), 120);
        let dialogue = Ask {
            class: TaskClass::Dialogue,
            deadline_ms: 500,
            cost_cap_micro: 100,
        };
        assert_eq!(router.route(dialogue), model("dlg-remote"));
        assert_eq!(router.spent_micro(), 80);
        let second = router.route(dialogue);
        assert_eq!(
            second,
            Err(RouterError::CostCap {
                label: String::from("dlg-remote"),
                cost_micro: 80,
                cap_micro: 40,
            }),
            "the cap is what the session has left"
        );
    }

    #[test]
    fn decisions_replay_from_the_record_twice() {
        let mut router = Router::new(card(), 1_000);
        let asks = [bark(400), bark(100), bark(400), bark(10)];
        for (n, ask) in asks.into_iter().enumerate() {
            if n == 2 {
                router.mark_down("big-remote");
            }
            let _ = router.route(ask);
        }
        let record = router.record().to_vec();
        assert_eq!(replay(&card(), 1_000, &record), Ok(()));
        assert_eq!(replay(&card(), 1_000, &record), Ok(()), "x2");
        // A changed scorecard no longer reproduces the record.
        let mut other = card();
        other[0].quality_permille = 100;
        assert_eq!(replay(&other, 1_000, &record), Err(0));
    }

    #[test]
    fn a_request_names_its_class_and_deadline() {
        let mut request = crate::mock::fixtures::request(1, 3);
        request.kind = TaskKind::Taunt;
        request.budget.max_ms = 250;
        let ask = Ask::for_request(&request, 5);
        assert_eq!(
            ask,
            Ask {
                class: TaskClass::Bark,
                deadline_ms: 250,
                cost_cap_micro: 5
            }
        );
    }
}
