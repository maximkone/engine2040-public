//! Live affordance probe over the production `engine-spatial` probe.
//! Owner: playtest crate. Units: integer centimeters, polygon ids as u32.
//! Determinism: delegates to `engine_spatial::probe` slice walks; same task
//! twice gives the same answers on every build. Errors: live probe refusals
//! (unknown peg, bad door) read as `false`; the fixture set never triggers
//! them. Edge: playtest -> spatial (probe only, std-only).

use crate::probe_sync::{AffordProbe, GoldQuery, ProbeTask};
use engine_spatial::probe as live;

/// Production probe answering the affordance set through live code.
/// The gold oracle (`slab_open`/`gold_answer`) stays math-independent;
/// only this adapter calls the live `engine_spatial::probe` functions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LiveProbe;

impl AffordProbe for LiveProbe {
    /// Coarse reach through the live connectivity walk, ignoring doors.
    fn coarse(&self, task: &ProbeTask, query: &GoldQuery) -> bool {
        live::coarse_reachable(&task.adj, &task.at, query.peg, query.goal_poly)
    }

    /// Wide reach through the live lenient-door walk.
    fn wide(&self, task: &ProbeTask, query: &GoldQuery) -> bool {
        live::wide_reachable(&task.adj, &task.doors, &task.at, query.peg, query.goal_poly)
    }

    /// Sync re-probe through the live exact-door plus corridor check.
    /// Fixture worlds carry no blockers, so the corridor reads clear once
    /// exact-door reach passes. Refusals read `false`, never a guess.
    fn sync(&self, task: &ProbeTask, query: &GoldQuery) -> bool {
        let Some(peg) = task.pegs.iter().find(|p| p.id == query.peg) else {
            return false;
        };
        let live_peg = live::Peg {
            id: peg.id,
            x_cm: peg.x_cm,
            z_cm: peg.z_cm,
            poly: peg.poly,
        };
        let pegs = [live_peg];
        let req = live::SyncRequest {
            adj: &task.adj,
            doors: &task.doors,
            blockers: &[],
            pegs: &pegs,
            at: &task.at,
            peg_id: query.peg,
            gx_cm: query.gx_cm,
            gz_cm: query.gz_cm,
            goal_poly: query.goal_poly,
        };
        live::sync_transfer(&req).unwrap_or_default()
    }
}
#[cfg(test)]
mod tests {
    use super::LiveProbe;
    use crate::probe_sync::AffordProbe;
    use crate::probe_sync::{
        generate_tasks, gold_answer, run_session, EXPECTED_COARSE, EXPECTED_SYNC, EXPECTED_WIDE,
        TOTAL_QUERIES,
    };

    #[test]
    fn live_agrees_with_independent_gold() {
        let tasks = generate_tasks();
        let first = run_session(&LiveProbe, &tasks).expect("live session scores");
        let second = run_session(&LiveProbe, &tasks).expect("live session repeats");
        assert_eq!(first, second);
        assert_eq!(first.total, TOTAL_QUERIES);
        assert_eq!(first.coarse_agree, EXPECTED_COARSE);
        assert_eq!(first.wide_agree, EXPECTED_WIDE);
        assert_eq!(first.sync_agree, EXPECTED_SYNC);
        assert_eq!(first.failed_sync.len(), TOTAL_QUERIES - EXPECTED_SYNC);
        let mut held_total: usize = 0;
        for task in &tasks {
            for query in &task.queries {
                let gold = gold_answer(task, query);
                let live = LiveProbe.sync(task, query);
                if query.held {
                    held_total += 1;
                    assert!(!gold);
                    assert!(live);
                } else if gold != live {
                    panic!("live drifts from gold on unheld {}", task.name);
                }
            }
        }
        assert_eq!(held_total, TOTAL_QUERIES - EXPECTED_SYNC);
    }
}
