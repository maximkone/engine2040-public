//! Practice swap: staged combat tuning applied at a tick boundary.
//!
//! Consumer: the window session drains staged tuning after the watcher
//! settle (`bin/arena.rs::poll_hot_reload`). Data owner: this module owns
//! the staged tuning text plus the live generation; combat tables stay in
//! `engine-combat`. Units: frames/ticks as indices, generations as opaque
//! `u64`; drain order is note order, no clock. Errors: [`ReloadReason`];
//! a bad cook keeps last-good. Budget: one staged string plus one live
//! generation; cold frame-boundary path only, never per-tick allocation.

use super::reload::ReloadReason;

/// One combat-tuning generation applied at a tick boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppliedTuning {
    /// Opaque generation from the cook (a byte hash in the window).
    pub generation: u64,
    /// Presented-frame index that applied it (stamped, read-only otherwise).
    pub frame: u32,
    /// Tick index stamped at apply; apply never advances it.
    pub tick: u32,
    /// Bout-tape line to write for this apply.
    pub tape_line: String,
}

impl AppliedTuning {
    /// Tape line for the bout log: generation plus frame plus tick.
    /// Pure; cold path only.
    #[must_use]
    pub fn line(generation: u64, frame: u32, tick: u32) -> String {
        format!("tuning gen={generation} frame={frame} tick={tick}")
    }
}

/// Staging gate for one combat-tuning generation: notes land any time,
/// generations apply only at the tick boundary the caller stamps.
#[derive(Clone, Debug, Default)]
pub struct PracticeSwap {
    staged: Option<String>,
    live: Option<AppliedTuning>,
    failed: Option<ReloadReason>,
}

impl PracticeSwap {
    /// Empty gate: nothing staged, nothing live.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Stage one tuning text for the next boundary. Empty text is refused
    /// (keeps last-good); a new note replaces the older staged one.
    pub fn note_tuning(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.staged = Some(text.to_string());
    }

    /// Drain the staged tuning at (`frame`, `tick`) through `cook`: `Ok`
    /// applies with both indices stamped and returns the tape holder;
    /// `Err` keeps the old tuning live and records why. The tick index is
    /// copied, never advanced mid-settle.
    pub fn apply_at_tick_boundary(
        &mut self,
        frame: u32,
        tick: u32,
        cook: impl FnOnce(&str) -> Result<u64, ReloadReason>,
    ) -> Option<AppliedTuning> {
        let text = self.staged.take()?;
        match cook(&text) {
            Ok(generation) => {
                let applied = AppliedTuning {
                    generation,
                    frame,
                    tick,
                    tape_line: AppliedTuning::line(generation, frame, tick),
                };
                self.live = Some(applied.clone());
                self.failed = None;
                self.live.clone()
            }
            Err(reason) => {
                self.failed = Some(reason);
                None
            }
        }
    }

    /// Live generation, if one applied.
    #[must_use]
    pub fn generation(&self) -> Option<u64> {
        self.live.as_ref().map(|live| live.generation)
    }

    /// Reason the last apply kept last-good, if it did.
    #[must_use]
    pub fn last_failure(&self) -> Option<&ReloadReason> {
        self.failed.as_ref()
    }
}

/// Cook staged combat-rows text: validate via `engine-combat`, refuse bad
/// rows, else hash validated text. Cold path only, never tick.
///
/// Caller: `dev_hotpatch::cook_practice_tuning` for rows-like text; tests
/// for file swaps. Same text gives same generation or same refusal; stale
/// or invalid files refuse and last-good stays. Golden unchanged: the live
/// match keeps its compiled tables; this only stages practice tuning.
pub fn cook_combat_rows(text: &str) -> Result<u64, ReloadReason> {
    engine_combat::tables_text::parse_tables(text)
        .map(|_| engine_core::hash_bytes(text.as_bytes()))
        .map_err(|err| ReloadReason::Cook(format!("{err:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SCRATCH: AtomicU64 = AtomicU64::new(0);

    fn scratch(name: &str) -> String {
        let n = SCRATCH.fetch_add(1, Ordering::SeqCst);
        let mut path = std::env::temp_dir();
        path.push(format!(
            "engine-game-practice-{name}-{}-{n}.rows",
            std::process::id()
        ));
        path.to_string_lossy().into_owned()
    }

    const V1_LINE: &str = "v1 attack_damage=120 attack_startup_ticks=4 attack_recovery_ticks=8 attack_stamina_cost=25 dodge_stamina_cost=20 block_hold_cost_per_tick=1 stamina_regen_per_tick=2 dodge_duration_ticks=10 dodge_iframes_ticks=6 block_chip_num=1 block_chip_den=5 parry_window_ticks=4 parry_stagger_ticks=12 hit_react_ticks=6 hitstop_ticks=4 knockdown_ticks=30";
    const V2_LINE: &str = "v2 attack_damage=120 attack_startup_ticks=15 attack_recovery_ticks=8 attack_stamina_cost=25 dodge_stamina_cost=20 block_hold_cost_per_tick=1 stamina_regen_per_tick=2 dodge_duration_ticks=10 dodge_iframes_ticks=6 block_chip_num=1 block_chip_den=5 parry_window_ticks=7 parry_stagger_ticks=12 hit_react_ticks=6 hitstop_ticks=4 knockdown_ticks=30";

    fn v3_file() -> String {
        let v3_line = V2_LINE
            .replace("v2 ", "v3 ")
            .replace("attack_damage=120", "attack_damage=130");
        ["table combat version 3", V1_LINE, &v3_line].join("\n")
    }

    #[test]
    fn practice_swap_applies_at_tick_boundary_and_writes_tape() {
        let mut swap = PracticeSwap::new();
        assert_eq!(swap.generation(), None);
        swap.note_tuning("good-v1");
        let first = swap
            .apply_at_tick_boundary(7, 100, |_| Ok(11))
            .expect("good tuning applies");
        assert_eq!(first.tick, 100);
        assert_eq!(first.frame, 7);
        assert_eq!(swap.generation(), Some(11));
        assert!(first.tape_line.contains("tick=100"));
        swap.note_tuning("bad-v2");
        let kept =
            swap.apply_at_tick_boundary(8, 101, |_| Err(ReloadReason::Cook("bad cook".into())));
        assert_eq!(kept, None);
        assert_eq!(swap.generation(), Some(11));
        assert_eq!(
            swap.last_failure(),
            Some(&ReloadReason::Cook("bad cook".into()))
        );
        swap.note_tuning("good-v3");
        let second = swap
            .apply_at_tick_boundary(9, 102, |_| Ok(12))
            .expect("good tuning applies");
        assert_eq!(second.tick, 102);
        assert_eq!(second.generation, 12);
        assert!(second.tape_line.contains("frame=9"));
    }

    #[test]
    fn combat_rows_file_swap_lands_once_and_replays_identical() {
        let path = scratch("swap-once");
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, v3_file()).expect("temp rows write");
        let text = std::fs::read_to_string(&path).expect("temp rows read");
        assert_eq!(text, v3_file());
        let mut swap = PracticeSwap::new();
        // Staged file text lands once at the tick boundary with its hash.
        swap.note_tuning(&text);
        let applied = swap
            .apply_at_tick_boundary(7, 100, crate::dev_hotpatch::cook_practice_tuning)
            .expect("valid rows file applies");
        assert_eq!(applied.tick, 100);
        assert_eq!(applied.frame, 7);
        assert_eq!(applied.generation, engine_core::hash_bytes(text.as_bytes()));
        assert!(applied.tape_line.contains("tick=100"));
        // No staged text: nothing lands twice.
        assert_eq!(
            swap.apply_at_tick_boundary(8, 101, crate::dev_hotpatch::cook_practice_tuning),
            None
        );
        assert_eq!(swap.generation(), Some(applied.generation));
        // Invalid rows refuse and keep last-good.
        let bad_text = v3_file().replace("attack_damage=130", "attack_damage=0");
        let bad_path = scratch("swap-invalid");
        let _ = std::fs::remove_file(&bad_path);
        std::fs::write(&bad_path, &bad_text).expect("temp bad write");
        let bad_read = std::fs::read_to_string(&bad_path).expect("temp bad read");
        swap.note_tuning(&bad_read);
        assert_eq!(
            swap.apply_at_tick_boundary(8, 101, crate::dev_hotpatch::cook_practice_tuning),
            None,
            "bad rows value must refuse"
        );
        assert_eq!(swap.generation(), Some(applied.generation));
        assert!(swap.last_failure().is_some());
        // Two replays under the swapped tables give the same hash.
        let swap_input = engine_combat::tables_text::parse_tables(&text).expect("v3 parses");
        assert_eq!(swap_input.version(), 3);
        let run_once = || {
            use engine_combat::fight::{step_with_tables, FightState};
            use engine_input::fighter::ATTACK;
            use engine_input::{Buttons, FighterId, FighterInput};
            let mut state = FightState::new(2).expect("two fighters");
            let idle = |fighter: u8| FighterInput {
                tick: 0,
                fighter: FighterId::new(fighter).expect("slot"),
                mx: 0,
                mz: 0,
                facing: 0,
                buttons: Buttons(0),
            };
            let attack = |fighter: u8| FighterInput {
                tick: 0,
                fighter: FighterId::new(fighter).expect("slot"),
                mx: 0,
                mz: 0,
                facing: 0,
                buttons: Buttons(ATTACK),
            };
            let tables = swap_input.tables();
            let mut events = step_with_tables(&[attack(0), idle(1)], &mut state, &tables);
            for _ in 0..20 {
                let tick_events = step_with_tables(&[idle(0), idle(1)], &mut state, &tables);
                events.extend(tick_events);
            }
            let digest = engine_core::hash_bytes(format!("{state:?}{events:?}").as_bytes());
            (state, digest)
        };
        let (first_state, first_hash) = run_once();
        let (second_state, second_hash) = run_once();
        assert_eq!(first_state, second_state);
        assert_eq!(first_hash, second_hash);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&bad_path);
    }
}
