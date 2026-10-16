//! dev_hotpatch: dev-only seam for one gameplay hot point.
//! Consumer: `arena.rs::poll_hot_reload`. Units: frames/ticks, gen `u64`.
//! Determinism: presentation only, never hashed. Errors: [`ReloadReason`].
use super::practice::{AppliedTuning, PracticeSwap};
use super::reload::ReloadReason;
/// Cook staged text: `bad` refuses, combat rows validate, else hashes.
/// Rows-like text (`table combat` or `attack_damage=`) validates through
/// [`crate::practice::cook_combat_rows`]; a staged file path reads and
/// validates when it exists, so the window path notes become validated swaps
/// without touching the poll hunk. Pure aside from the cold file read; cold
/// path only, never tick. Errors: [`ReloadReason::Cook`].
pub fn cook_practice_tuning(text: &str) -> Result<u64, ReloadReason> {
    if text.contains("bad") {
        return Err(ReloadReason::Cook("bad tuning".to_string()));
    }
    let looks_like_rows = text.contains("table combat") || text.contains("attack_damage=");
    if looks_like_rows {
        return crate::practice::cook_combat_rows(text);
    }
    if std::path::Path::new(text).is_file() {
        match std::fs::read_to_string(text) {
            Ok(contents) if contents.contains("table combat") => {
                return crate::practice::cook_combat_rows(&contents);
            }
            Ok(contents) => return Ok(engine_core::hash_bytes(contents.as_bytes())),
            Err(err) => return Err(ReloadReason::Cook(err.to_string())),
        }
    }
    Ok(engine_core::hash_bytes(text.as_bytes()))
}
/// Debug twin via indirect call for `subsecond::call` detour once dep lands.
/// PORT(DioxusLabs/dioxus@main:packages/subsecond subsecond::call jump-table detour) — call-shape only.
/// PORT(DioxusLabs/dioxus@main:packages/subsecond ThinLink) — link idea only.
#[cfg(debug_assertions)]
pub fn apply_practice_at_boundary(
    swap: &mut PracticeSwap,
    frame: u32,
    tick: u32,
) -> Option<AppliedTuning> {
    let hot: fn(&str) -> Result<u64, ReloadReason> = cook_practice_tuning;
    swap.apply_at_tick_boundary(frame, tick, hot)
}
/// Release twin: direct, zero overhead, zero behavior change.
#[cfg(not(debug_assertions))]
pub fn apply_practice_at_boundary(
    swap: &mut PracticeSwap,
    frame: u32,
    tick: u32,
) -> Option<AppliedTuning> {
    swap.apply_at_tick_boundary(frame, tick, cook_practice_tuning)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hot_seam_applies_then_keeps_last_good() {
        let mut swap = PracticeSwap::new();
        swap.note_tuning("good-v1");
        let first = apply_practice_at_boundary(&mut swap, 7, 100).expect("good applies");
        assert_eq!((first.frame, first.tick), (7, 100));
        swap.note_tuning("bad-v2-has-bad");
        assert_eq!(apply_practice_at_boundary(&mut swap, 8, 101), None);
        assert_eq!(swap.generation(), Some(first.generation));
        assert!(swap.last_failure().is_some());
    }
}
