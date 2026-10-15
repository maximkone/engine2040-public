# balance-log — one change per run, trials before rates

One currency exists: glory (`crates/game/src/ladder.rs`).
Gold, crowd favor: not in this slice (economy-full CUT, `docs/plan/gameplay.md`).

Rule: one change per run; trial count stated before any rate; before and after below.
Trials 200 per difficulty (`MATCHES_PER_DIFFICULTY` in `crates/playtest/src/tournament.rs`).

| date | change | trials | before | after |
|---|---|---|---|---|
| 2026-10-12 | des-pq2 350 per-move bar landed (bench mirrors tournament, no second number) | 200 per difficulty | 1000 per-side ceiling that cannot trip, no per-move count | 350 per-move (`PQ2_KILL_SHARE_MAX_PERMILLE` in `crates/game/src/bench.rs:65` reuses `KILL_SHARE_MAX_PERMILLE` 350 in `crates/playtest/src/tournament.rs:40`), lopsided seeded move trips (`pq2_lopsided_seeded_move_trips`) |
| 2026-10-12 | log creation, no numbers changed, systems current | 200 per difficulty | ledger 5 pools, core-loop beat chart, matchups 3-type cycle, loop.rows v1 on disk | same, verified on disk; bench re-verify blocked by missing `crates/combat/src/classes.rs` (another crate) |
| 2026-10-14 | no numbers changed, dry-fallback re-verify, systems current | 200 per difficulty (`MATCHES_PER_DIFFICULTY` in `crates/playtest/src/tournament.rs:26`) | ledger 5 pools, core-loop beat chart, matchups 3-type cycle, loop.rows v1 on disk | same, verified on disk; `cargo test -p engine-playtest --lib winrates` 8 passed 0 failed; `cargo test -p engine-game --lib bench` 10 passed 0 failed 1 ignored (render breakage healed mid-run by the render seat); `bench::run` still has no non-test caller, wiring packet `wire-bench-run-20261014.md` filed for the wire builder |
| 2026-10-14 | no numbers changed, log corrects wiring note only, systems current | 200 per difficulty (`MATCHES_PER_DIFFICULTY` in `crates/playtest/src/tournament.rs:26`) | prior row said `bench::run` has no non-test caller | `crates/game/src/bin/staged_fight.rs` on disk now calls `bench::run` in `run_balance_screen` (`--balance-screen`, uncommitted, preserved, not this seat); re-verify BLOCKED by other crates: `cargo test -p engine-game --lib bench` fails at `crates/render/src/shadow_cascade.rs:65` unsafe-code deny, `cargo test -p engine-playtest --lib winrates` fails at `engine-physics` missing `MAX_AIR_MM_S`; no rates claimed, no numbers changed |
