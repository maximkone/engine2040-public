# Fix-bugs report — combat, rules, destruction, clock, ossim, engine-chaos

Owner of this file: the FIX lane (2026-10-21). Scope: the six crates the lane owns
(`crates/combat`, `crates/rules`, `crates/destruction`, `crates/clock`, `crates/ossim`,
`crates/engine-chaos`). Nothing outside them was edited except this report.

Golden `d57b5c8ae3096614` untouched: no crate in this lane feeds the legacy 2D proof
fixture (`docs/HASH-LAW.md` names the scope: the archetype ECS world,
`engine_physics::Space`, the tick clock and the input tape; it does not run arena,
combat, destruction or any of the other crates here). The only crate in this lane
that the golden could read is `engine-clock`, and it is **unchanged**. Re-measured
with these edits in the tree: `cargo test -p engine-proof --lib golden` →
`test result: ok. 6 passed; 0 failed` (`a9::tests::golden_hash_untouched`,
`fb_versions::tests::golden_hash_is_unchanged` among them). The arena digest
(`arena=2fbd56040fd091a7`) reads `engine-combat`, and the combat change here is
test-only (three assertions), so it cannot move either. No `git add`, no commit, no
push: the lead commits by path.

## The bugs

| crate | the bug | why it is a bug | the test that now fails without the fix | the proof command and its one-line result |
|---|---|---|---|---|
| combat | `sweep.rs` and `depth.rs` still pinned the **old 16-slot cap** after the owner raised `engine_input::MAX_PARTICIPANTS` to 32 (commit `15049ef3`). Three assertions still said "16" / "17 refuse". | The crate's own suite is **red at HEAD**: the cap moved, the tests did not. `strict_tick_rejects_counts_duplicates_and_overflow` expected 17 fighters to refuse (legal under 32) and `slots_beyond_four_resolve_in_id_order` expected `TooManyFighters(17)` where the code says 33. A stale expectation in an authoritative crate is a lie about the shipped cap. | `sweep::tests::strict_tick_rejects_counts_duplicates_and_overflow`, `sweep::tests::slots_beyond_four_resolve_in_id_order`, `depth::tests::depth_budgets_hold` (all three were red before the fix) | `node tools/gh-check.mjs cargo test -p engine-combat --lib` → `test result: ok. 314 passed; 0 failed; 2 ignored` (run 39507690661) |
| destruction | `DebrisLedger::step` only ever **set** `sleeping = true`; it never cleared it. A `kick` woke one shard, so the rest of its island stayed asleep forever: they stopped integrating (frozen in mid-air) and the island could never sleep again. The doc said "the island leaves sleep on the next step". | A sleep island is a unit. One kicked shard left its neighbours asleep and frozen, and the island's hash kept that half-asleep state forever. | `budget::tests::a_kick_wakes_the_whole_island_on_the_next_step` (new; the island must wake as one unit) | `node tools/gh-check.mjs cargo test -p engine-destruction --lib` → `test result: ok. 185 passed; 0 failed; 4 ignored` (run 39509011238) |
| ossim | The swap file was **over-committed**. `swap_out` (player) and `fault_slot` (ambient page fault) checked the room against `swap_used` only, but the space is only counted when the swap lands, so two swaps in flight both passed the check and an ambient fault never checked at all. `spawn` into swap had the same gap. | `State.swap_slots_used` could pass `State.swap_slots_cap`: the stage leaked a budget it does not own, and later spawns were refused because of it. | `tests::a_full_swap_file_refuses_the_page_fault` and `tests::the_swap_file_never_holds_more_pages_than_it_owns` (new) | `node tools/gh-check.mjs cargo test -p engine-ossim --lib` → `test result: ok. 33 passed; 0 failed` (run 39511426410) |
| rules | `tape::record` **panicked** on `checkpoint_every == 0` (`is_multiple_of(0)` divides by zero) and happily wrote tapes its own decoder refuses: 0 fighters, 0 ticks, two script installs on one tick, unsorted inputs. | A recorder that panics or writes an unreadable tape is a broken contract: the tape is the replay proof, and `RulesTape::decode` rejects exactly those tapes. | `tests::record_refuses_a_spec_the_decoder_would_reject` (new) — before the fix the zero-cadence case panicked instead of returning `Err`. | `node tools/gh-check.mjs cargo test -p engine-rules --lib` → `test result: ok. 52 passed; 0 failed; 1 ignored` (run 39510520593) |
| rules | `tape::replay` **indexed past the end** of a hand-built tape: `tape.checkpoints[next_check]` panicked when the tape held fewer checkpoints than the match produces, and unsorted inputs replayed the wrong tick. | A public replay entry that panics on a malformed tape takes the match down. The field rules existed only inside the byte decoder, so an in-memory tape skipped them. | `tests::replay_refuses_a_malformed_tape_instead_of_panicking` (new) — before the fix the short tape panicked with an index-out-of-bounds. | `node tools/gh-check.mjs cargo test -p engine-rules --lib` → `test result: ok. 52 passed; 0 failed; 1 ignored` (run 39510520593) |
| engine-chaos | The facade's counts came from **two separate suite runs**: `a9_counts()` called `healed_codes()` and `quarantined_codes()`, each of which runs the whole A9 suite. `LoudReason::InjectionFailed` rows were dropped from both lists, so a harness break vanished and the counts still looked complete (3 + 9 = 12). | The crate's own doc says "every count below comes from one `engine-proof` run at the recorded A9 seed". The code did not: the two numbers could come from different runs (the net row folds a wall-clock budget into its verdict), so they could describe a split that never happened, and a broken row was hidden instead of shown. | `tests::every_fault_row_is_accounted_for_by_one_run` (new) — it needs the single-run report and the broken-row list the old facade did not have, so the old crate cannot compile it; the old facade had no way to see an injection failure at all. | `node tools/gh-check.mjs cargo test -p engine-chaos --lib` → `test result: ok. 5 passed; 0 failed; 0 ignored` (the six-crate run on the runner: PASS, run 39535292227). |
| clock | none found. The crate is one `OnceLock` source, one saturating `since`, and five tests. | — | — | `node tools/gh-check.mjs cargo test -p engine-clock --lib` → `test result: ok. 5 passed; 0 failed` (run 39511818874) |

### Why the combat entry is a real defect and not a taste call

Commit `15049ef3` ("input/net: raise the canonical fighter cap to 32") changed the cap
and the sweep test that counted fighters, but left three expectations on the old
16/17 numbers. The crate's suite has been red at HEAD since. The fix keeps the
assertions tied to the owner (`MAX_PARTICIPANTS`) so the next cap move cannot leave
them stale again.

### Why the ossim fix does not hide the budget

`swap_committed()` is the pages the swap file must hold: the pages already in swap
plus the pages of a swap-out in flight (they are counted when the swap lands). Every
admission path (player swap-out, ambient page fault, spawn into swap) now measures
against that number, so `swap_used` never passes `swap_slots`. A page fault into a
full swap file is refused with the typed `Refusal::NoSwap` (the process keeps its
pages) instead of pushing the file past the slots the stage owns.

### Why the rules fix has one owner for the field rules

`RulesTape::validate()` is now the single owner of the field rules (fighter count,
ticks, cadence, script ticks, input ranges, input order, checkpoint count).
`RulesTape::decode` calls it after its byte reader, and `tape::replay` calls it
before it runs, so a decoded tape and a hand-built tape face the same rules.
`record` refuses a spec up front with the new typed `RulesError::BadSpec`.

## Before and after test counts per crate

| crate | before | after | note |
|---|---|---|---|
| engine-combat | 314 tests: **311 passed, 3 failed**, 2 ignored (measured, run 39501106549) | 314 tests: **314 passed, 0 failed**, 2 ignored (measured, run 39507690661); today's tree: **310 passed, 0 failed**, 2 ignored | no test added; three stale assertions now follow `MAX_PARTICIPANTS`. The suite is four tests shorter today (316 → 312): the only other combat file that differed then was `crates/combat/src/design.rs` (another lane's uncommitted edit), and that uncommitted work was lost with a tree wipe. The three fixed tests are inside today's 310 and are green. |
| engine-destruction | 188 tests: 184 passed, 4 ignored (count = 189 − 1 new test) | 189 tests: **185 passed, 0 failed**, 4 ignored (measured, run 39509011238; today's tree: 185 passed, 0 failed) | one new test |
| engine-ossim | 31 tests (count = 33 − 2 new tests) | 33 tests: **33 passed, 0 failed** (measured, run 39511426410; today's tree: 33 passed, 0 failed) | two new tests |
| engine-rules | 50 tests (count = 53 − 3 new tests) | 53 tests: **52 passed, 0 failed**, 1 ignored (measured, run 39510520593; today's tree: 52 passed, 0 failed) | three new tests |
| engine-clock | 5 tests: **5 passed** (measured, run 39500731585) | 5 tests: **5 passed** (measured, run 39511818874; today's tree: 5 passed, 0 failed) | unchanged |
| engine-chaos | 4 tests (count = 5 − 1 new test) | 5 tests: **5 passed, 0 failed** (the six-crate run on the runner: PASS, run 39535292227; the earlier GitHub attempts died in `crates/physics`, see the reds table) | one new test |

One command proves all six suites in today's tree:
`node tools/gh-check.mjs cargo test -p engine-combat -p engine-rules -p engine-destruction
-p engine-ossim -p engine-clock -p engine-chaos --lib` → **PASS on the runner**
(run 39535292227; the same command also passed as run 39532228896). Per crate, the
same command on the PC: combat 310 passed / 2 ignored, rules 52 / 1 ignored,
destruction 185 / 4 ignored, ossim 33, clock 5, chaos 5.

Clippy (`cargo clippy -p <crate> --all-targets -- -D warnings`):

- engine-combat, engine-destruction, engine-ossim, engine-clock → **PASS on the runner**,
  run 39534229937 (one command for the four).
- engine-rules and engine-chaos → the exact command **fails on a red outside this
  lane**: `crates/anim/src/pose.rs:162` (`if !(span > 0.0)` →
  `clippy::neg_cmp_op_on_partial_ord`) and `crates/anim/src/pose.rs:379`
  (`for k in 0..4` → `clippy::needless_range_loop`). `cargo clippy` runs
  clippy-driver over every workspace path dependency, so the anim lane's in-flight
  file kills the run before the rules or chaos crate is linted (runner run
  39534852971). With the dependency lint excluded, both crates are clean:
  `cargo clippy -p engine-rules -p engine-chaos --all-targets --no-deps -- -D warnings`
  → **PASS on the runner**, run 39534549578; the same command over all six crates
  → PASS on the PC (exit 0).
- One earlier attempt over all six crates failed in `crates/physics` (E0761: the
  snapshot carried both `cloth.rs` and `cloth/mod.rs` while the cloth split was in
  flight), run 39532680681. The physics lane landed the split (commit `a2531f3b`)
  and the later runs pass.

## Reds found in other crates and left alone

These are not mine. Each one blocked a proof of mine at the time; every one of them
is a crate this lane must not touch. None of them is in the six crates above.

| crate | the red | what it blocked here |
|---|---|---|
| `crates/arena` | `crates/arena/src/fork.rs:664` — `Sim { ... }` missing field `cuts` (the arena `Sim` grew a `cuts: Vec<TickCuts>` field) | `cargo test -p engine-rules --lib` and `cargo test -p engine-chaos --lib` could not build (runs 39506830607, 39500936231). Fixed by its owner later in the session; the rules proof then ran. |
| `crates/ecs` | `crates/ecs/src/pool.rs` — `pub(crate) fn shared()` is private, but `crates/arena/src/fork.rs:426,839` calls `engine_ecs::pool::shared()` | `cargo clippy -p engine-rules --all-targets -- -D warnings` (run 39515239184). |
| `crates/net` | `crates/net/src/heal.rs:95` — non-exhaustive match on the new `HealError` variants (`UnverifiedRecovery`, `ScopeMismatch`, `UnknownHeal`) from `crates/observe` | `cargo test -p engine-chaos --lib` (run 39512151393). |
| `crates/physics` | `crates/physics/src/cloth.rs` — `index`, `from_fixed`, `to_fixed` not found (23 errors) while the cloth rewrite is in flight | `cargo test -p engine-chaos --lib` (run 39517702457, 39518323704) |
| `crates/physics` | `crates/physics/src/cloth/` — the cloth file was split into a module directory mid-flight; the snapshot caught it half-done (`hash_shape`, `cloth_meets`, `touched`, `cap_speed`, `sweep_colliders`, `push_out`, `resolve_self`, `SweptHit`, `MAX_ITERATIONS` not found or private) | `cargo test -p engine-chaos --lib` (run 39518577128), `cargo clippy -p engine-rules --all-targets -- -D warnings` (run 39519254180) |
| `crates/observe` | `crates/observe/src/supervisor.rs` — `QuarantineEntry` was more private than `Supervisor::quarantine` | `cargo test -p engine-chaos --lib` (run 39510996144). |
| `crates/observe` | `crates/observe/src/recovery.rs:18` — unused import `HealTapeRecord` (warning, `-D warnings` makes it an error) | would fail `cargo clippy -p engine-chaos --all-targets -- -D warnings`. |
| `crates/physics` | `crates/physics/src/cloth.rs:448` — field `hash_cell_of` never read (warning) | would fail `cargo clippy -p engine-chaos --all-targets -- -D warnings`. |
| `crates/physics` | `crates/physics/src/lib.rs:14` — E0761: the snapshot carried both `cloth.rs` and `cloth/mod.rs` while the cloth split was in flight | `cargo clippy` over all six crates (run 39532680681). The physics lane landed the split (commit `a2531f3b`); the later runs pass. |
| `crates/anim` | `crates/anim/src/pose.rs:162` — `if !(span > 0.0)` is `clippy::neg_cmp_op_on_partial_ord`, and `crates/anim/src/pose.rs:379` — `for k in 0..4` is `clippy::needless_range_loop` (the anim lane is editing this file) | the exact `cargo clippy -p engine-rules --all-targets -- -D warnings` and `cargo clippy -p engine-chaos --all-targets -- -D warnings` runs: clippy-driver runs over every workspace path dependency, so these lints fail both before the rules or chaos crate is linted (runner run 39534852971). Both crates are clean under `--no-deps` (runner run 39534549578). |
| `crates/input` | `crates/input/src/layers.rs` — `no method named 'byte' found for reference '&ActionSet'` (a new module, the input lane mid-flight) | one local re-run of the three combat tests could not compile `engine-input`; the six suites were already green before this red landed. |

## Files changed in this lane

- `crates/combat/src/sweep.rs`, `crates/combat/src/depth.rs`
- `crates/destruction/src/budget.rs`
- `crates/ossim/src/sim.rs`, `crates/ossim/src/tests.rs`
- `crates/rules/src/tape.rs`, `crates/rules/src/error.rs`, `crates/rules/src/rules.rs`, `crates/rules/src/tests.rs`
- `crates/engine-chaos/src/lib.rs`

`crates/combat/src/design.rs` was already modified in the tree by another lane when
this lane started; it is **not** part of this work and was left alone.

## Note for the lead

The working tree lost these edits three times during the session (an upstream pull
landed while they were uncommitted, and the working tree came back without them).
Every time, the ten files were restored from the copy in
`C:\Users\me\AppData\Local\Temp\opencode\fixbugs-backup\` (the report file is
copied there too). If the files are missing from the tree, copy them back from
there.

The ten crate files are modified but uncommitted: the lead commits them by path —
`crates/combat/src/sweep.rs`, `crates/combat/src/depth.rs`,
`crates/destruction/src/budget.rs`, `crates/ossim/src/sim.rs`,
`crates/ossim/src/tests.rs`, `crates/rules/src/tape.rs`,
`crates/rules/src/error.rs`, `crates/rules/src/rules.rs`,
`crates/rules/src/tests.rs`, `crates/engine-chaos/src/lib.rs` — plus this report
(`docs/research/powerups-2026-10-20/fix-bugs.md`).
