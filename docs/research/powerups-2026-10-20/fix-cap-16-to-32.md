# Fix: the stale 16-fighter caps left behind by the 16 -> 32 participant cap raise

Owner of this file: the CAP FIXER lane (2026-10-21). Scope: `crates/perf`, `crates/ui`,
`crates/app`, `crates/game` and their test files, plus this report. Nothing else was edited.
No `git add`, no commit, no push: the lead commits by path. Golden `d57b5c8ae3096614`
untouched (no hashed path is in this diff).

Work list: `docs/research/powerups-2026-10-20/stale-16-sweep.md` (read-only audit at
`718684d1` plus the dirty tree). Every site below is that sweep's file:line, fixed or
explicitly left, plus the extra stale-16 sites this lane found by running its own suites.

## 1. The defect

`engine_input::MAX_PARTICIPANTS` moved 16 -> 32 on 2026-10-20 (owner decision GE12,
`docs/plan/runtime.md:299`). Every crate that *derives* its storage from the constant was
correct already. The crates that had *copied* the old cap into their own constant, their
own table, their own fixture or their own prose were not: the HUD overlay named 16 rows,
the app frame refused 17 actors, the game's name table stopped at F15, and the perf gate
compared a 32-fighter run against a 16-fighter baseline.

The fix is one rule, applied everywhere: **no crate in these four lanes owns an unpinned
copy of the cap.** The single source is

```rust
// crates/input/src/fighter.rs:29
pub const MAX_PARTICIPANTS: u8 = 32;
```

re-exported by `engine_arena` (`crates/arena/src/lib.rs:91`), so `crates/app` reads
`engine_arena::MAX_PARTICIPANTS` and needs no new dependency.

The cap constant is a `u8`; row counts, dot counts and byte budgets are `usize`. The house
way to bridge that without a numeric cast is the pattern already in
`crates/game/src/director.rs:271-272` - a literal sized for the array, pinned to the
canonical cap by a compile-time assertion:

```rust
// Scratch bound for one entry per fighter slot: a literal (never a
// numeric cast) pinned to the participant cap at compile time, so a
// cap change fails loudly here instead of mis-sizing the fold.
const SLOTS: usize = 32;
const _: () = assert!(MAX_PARTICIPANTS == 32);
```

Every count in this diff follows that pattern: the literal is not a second cap, it is a
compile-time pin on the one cap. When the cap moves, the build fails instead of quietly
lying. (`usize::from(u8)` is the named conversion the quality bar asks for, but it is not a
`const fn`, so it cannot type a `const` item; a numeric `as` cast would work and is exactly
what `tools/rust-quality.mjs` counts as a ratchet regression, so the pinned literal is
used instead.)

## 2. Every site fixed

| file:line | was | now |
|---|---|---|
| `crates/ui/src/lib.rs:39` | `MAX_MAP_CAP = 16` minimap dots | `PARTICIPANT_CAP`, a literal pinned to the cap by `assert!(MAX_PARTICIPANTS == 32)`; the legal range `1..=MAX_MAP_CAP` now covers the roster |
| `crates/ui/src/lib.rs:637` | `MAX_OVERLAY_ROWS = 16`, doc: "matches the `engine-input` slot cap" | `PARTICIPANT_CAP` (same pin); the wire bound `OVERLAY_MAX_BYTES` follows (21 + 32*28 = 917) |
| `crates/ui/src/lib.rs:2337,2340` | the over-cap decode test wrote `17` and expected `TooManyRows(17)` | writes one row over the cap (`MAX_OVERLAY_ROWS + 1` = 33) and expects `TooManyRows(33)` |
| `crates/ui/src/lib.rs:2609` | the full-house overlay test filled `0..16u8` | fills `0..MAX_PARTICIPANTS` (32 rows) |
| `crates/ui/src/access.rs:383` | `sixteen_fighter_storm()`: every ordered pair of 16 = 240 cues | `full_roster_storm()`: every ordered pair of the full roster = cap * (cap - 1) = 992 cues, the number that actually tests the queue bound |
| `crates/app/src/snapshot.rs:23` | `MAX_FRAME_ACTORS = 16` ("the arena participant cap") | `32`, pinned by `assert!(engine_arena::MAX_PARTICIPANTS == 32)` |
| `crates/app/src/snapshot.rs:231` | the decode refused `count > 16` | the decode refuses `count > MAX_FRAME_ACTORS` (32) |
| `crates/app/src/snapshot.rs:422,426` | the test wrote `17` into the count byte and expected `"17 actors"` | writes one past the cap (33) and expects `"33 actors"` |
| `crates/app/src/tests.rs:591` | `assert_eq!(MAX_FRAME_ACTORS, 16)` | `assert_eq!(MAX_FRAME_ACTORS, usize::from(engine_arena::MAX_PARTICIPANTS))` |
| `crates/game/src/windowed.rs:1092` | `const NAMES: [&str; 16]` ("F0".."F15") | `const NAMES: [&str; 32]` ("F0".."F31") pinned by `assert!(MAX_PARTICIPANTS == 32)`; slots 16-31 get a row instead of none |
| `crates/game/src/windowed.rs:1102` | `let _ = overlay.upsert_fighter(id, name, hp);` (the refusal was swallowed) | `duel_overlay_for_app` returns `Result<DuelOverlay, OverlayError>`; the refusal is reported, never dropped |
| `crates/game/src/windowed.rs:1718`, `crates/game/src/bin/arena.rs:2097`, `crates/game/src/bin/arena_capture.rs:765` | the overlay's error had nowhere to go | each call site maps the error to `WinError::Render(cap_message(...))` |
| `crates/game/src/windowed.rs:3325,3329` | `duel_overlay_caps_at_sixteen_with_stable_names`, `fighters: 16`, 16 rows | `duel_overlay_covers_the_full_roster_with_stable_names`, `fighters: MAX_PARTICIPANTS`, 32 rows with stable names `F{slot}` |
| `crates/game/src/windowed_captions.rs:44,133` | "outside the 16-fighter range" | "outside the participant range" |
| `crates/game/src/lib.rs:2159` | `GameConfig { fighters: 17 }` listed as a bad config (17 was one past the old cap) | `fighters: MAX_PARTICIPANTS + 1` (33) |
| `crates/game/src/lib.rs:2177` | comment: "5 fighters fits the 16-slot cap" | "5 fighters fits the participant cap" |
| `crates/game/src/lib.rs:2281` | `assert!(bank.len() <= 16, "the clip ledger sizes 16 cues")` | `assert_eq!(bank.len(), engine_audio::ARENA_CUES.len())`: the ledger's size belongs to the audio lane's shipped cue table (19 cues today), so the gate follows the owner instead of a number that goes stale |
| `crates/game/src/lib.rs:2991` | `assert_eq!(dst.fighters.len(), 16)` after filling `0..MAX_PARTICIPANTS` | `assert_eq!(dst.fighters.len(), usize::from(MAX_PARTICIPANTS))` |
| `crates/game/src/capture.rs:1903` | `arena_capture_config("arena", 2040, 48, 17).is_err()` | `MAX_PARTICIPANTS + 1` (33) |
| `crates/game/src/capture.rs:1943` | the tape's slot byte `16` had to be refused | the byte is `MAX_PARTICIPANTS` (32), one past the legal range |
| `crates/game/src/director.rs:1397` | the corrupt slots `20, 21` had to be out of range | `MAX_PARTICIPANTS, MAX_PARTICIPANTS + 1` (32, 33): the corrupt slots are past the cap, not inside it |
| `crates/game/src/modes_world.rs:1022` | a lobby boot with `17` fighters had to be refused | `MAX_PARTICIPANTS + 1` (33) |
| `crates/perf/src/workloads.rs:116` | doc: "`MAX_PARTICIPANTS` (16, the engine maximum)" | the real cap, and the unmeasured "40 combat events and 17 crate breaks" claim softened to "hits and crate breaks" |
| `crates/perf/src/workloads.rs:833` | `ARENA_MATRIX_COUNTS = [4, 8, 16]` | `[4, 8, 16, MAX_PARTICIPANTS]`: the 16 row stays a legitimate bar beside the new cap row |
| `crates/perf/src/workloads.rs:890,901,906` | `workload_name` / `tracked_name` fell through to the 16 row | the 16 row is named by count, the cap row by the fall-through; `NAME_32` / `NAME_32_SIM` added |
| `crates/perf/src/workloads.rs:1158` | `scaled_sixteen_matches_the_sixteen_slot_default` (red) | `scaled_cap_run_matches_the_cap_slot_default`: the cap-sized row is the same shape as the canonical default (same hash, same totals) |
| `crates/perf/src/workloads.rs:1318` | `assert_eq!(MAX_SPINE_SLOTS, 16)` (pins the ecs lane's stale cap) | `assert!(MAX_SPINE_SLOTS <= usize::from(MAX_PARTICIPANTS))`: the relation, not the copied number |
| `crates/perf/src/allocator.rs:185,359` | the system allocator's arena row ran `fighters: 16` and was pinned equal to "the canonical sixteen-fighter run" (red) | `fighters: MAX_PARTICIPANTS`, pinned equal to "the canonical cap run" |
| `crates/perf/tests/it/allocator_baseline.rs:39,71` | the harness read the 16-fighter `arena_tick` row; the wall-ms arm ran the 16-fighter shape | the harness reads `arena_baseline()` (the cap row); the wall-ms arm runs the cap-sized shape |
| `crates/perf/src/baselines.rs:40` | the `arena_tick` row was the canonical gate row | the cap row `arena_tick_32` is appended and read by `arena_baseline()`; the old row stays as the historical record (section 3) |
| `crates/perf/baselines/heap_peak.txt` | the bar `56672` bytes (the arena's slot arrays were 16 wide) | the bar `86560` bytes, re-recorded after the cap raise (section 4) |

## 3. The perf baseline: appended, never overwritten

The perf gate compares a live run to a recorded row, so the canonical arena row had to be
re-recorded. The board's rule is that baselines are **append-only**, and the cap raise is
the board's allowed re-record reason (a fixed-array size change), so the re-record is
dated in place and the old row stays where it was.

`crates/perf/src/baselines.rs`:

- The legacy 16-fighter `arena_tick` row (600 ticks, hash `12857399269765116608`, 63539
  bytes, 151 counts) stays **unchanged** as the historical record, with a dated comment
  that the `arena_tick_32` row replaced it for the canonical run.
- The new `arena_tick_32` row is **appended**: 600 ticks, hash `12449757284742186622`,
  106932 bytes, 240 counts. `arena_baseline()` reads it on the default track.
- The three scaled rows were re-recorded in place with dated notes, because the arena's
  arrays are now cap-sized: 4 fighters 57102/60 -> 86254/61, 8 fighters 59044/65 ->
  88932/66, 16 fighters 63539/151 -> 94899/152. **Every hash stayed identical**, which is
  the proof that the sim state did not move and only the allocation totals followed the
  wider arrays.
- The `ai_decide` row was re-recorded in place with a dated note (hash
  5808393643730122605 -> 3634382762935724637, 1344 -> 4096 bytes, 1 count): the brain table
  is now cap-sized.

`scaled_arena_baseline(count)` maps a count to the row of the compiled track
(`engine_arena::SIM_TRACK` is a compile-time constant), and `arena_baseline()` picks the
cap row of that track.

## 4. The heap-peak bar, re-recorded with its cause

`crates/perf/baselines/heap_peak.txt` holds the peak live-heap bar of the 8-fighter
headless match (one number; a second row would read as a corrupt pin). The cap raise
doubled the arena's fixed slot arrays, so the same match holds more live heap and the
gate tripped: `FAIL peak=86560 baseline=56672`.

- The bar moved `56672 -> 86560` bytes. The file cannot carry a note, so the old bar and
  the cause are recorded in the harness doc (`crates/perf/src/main.rs:566`, "2026-10-21:
  the recorded bar ... moved 56672 -> 86560 bytes ... the arena's fixed slot arrays
  doubled").
- The measured peak is the debug-profile number the gate test itself measures (CI runs
  `cargo test` in debug; a release profile allocates less, so the bar has room in both).

## 5. What could not be done here: the `sim` track

There is **no `arena_tick_32_sim` row**, and no sim number was invented.

`--features engine-arena/sim` does not compile in this workspace: `LiveMatch::snapshot`
(`crates/arena/src/live.rs:447`) and `LiveMatch::from_snapshot` (`:487`) are
`#[cfg(not(feature = "sim"))]`, but `crates/net/src/session.rs:769` and
`crates/net/src/played.rs:971` call them, so `engine-net` fails to compile with the
feature on (`error[E0599]: no method named snapshot found for struct LiveMatch`). That is
the net lane's breakage, not this lane's, and `engine-perf` depends on `engine-net`.

The honest shape is therefore: on the `sim` track `arena_baseline()` panics with an
explicit message naming the reason, and `scaled_arena_baseline(MAX_PARTICIPANTS)` returns
`None`. The lookup fails loudly until that row is recorded where that track builds. The
existing `_sim` twins (4/8/16 fighters) keep their historical values untouched.

## 6. Six red tests the sweep did not list, found in this lane

The cap raise left these red at HEAD; they are in this lane's crates, so they are fixed
here (the stale expectation was fixed, not the strength of the test):

- `crates/app/src/snapshot.rs:422` - the over-cap frame case wrote `17`, which is legal now.
- `crates/game/src/lib.rs:2159` - `fighters: 17` was listed as a bad config.
- `crates/game/src/lib.rs:2281` - the clip ledger's hardcoded 16-cue bound.
- `crates/game/src/capture.rs:1903` - `arena_capture_config(..., 17)` had to be refused.
- `crates/game/src/modes_world.rs:1022` - a lobby boot with 17 fighters had to be refused.
- `crates/game/src/director.rs:1397` - the corrupt slots 20/21 are inside the cap now.

And one more red gate, found by running the whole perf crate (all targets, not only
`--lib`): the heap-peak gate of section 4.

## 7. Pre-existing clippy debt in this lane, cleared so the proof could run

`cargo clippy -p engine-game --all-targets -- -D warnings` failed at HEAD on three lints in
files this lane owns that the cap work never touched (they arrived with the 2026-10-20
merges). They are fixed here, in the same lane, because the lane's clippy proof cannot be
green otherwise:

- `crates/game/src/camera.rs:1451` - `-1.0 * LEAD_HORIZON_S` -> `-LEAD_HORIZON_S`.
- `crates/game/src/camera_rig.rs:541` - the negated comparison `!(axis_len > 0.0)` ->
  `axis_len.partial_cmp(&0.0) != Some(Ordering::Greater)`: the same NaN-safe meaning.
- `crates/game/src/crash.rs:658` - `#[expect(clippy::too_many_arguments, reason = "the
  bundle's fields mirror report.toml one to one")]`, the house pattern for this lint.

## 8. Cross-lane notes (not fixed here: another lane owns the file)

- **`crates/render/src/overlay.rs:377-388`** - `hp_bar_rect` (`:382`) clamps the slot with
  `slot.min(15)` (`:388`), so with 32 overlay rows the bars of slots 16-31 all paint row 15.
  The render lane must move that clamp with `MAX_OVERLAY_ROWS`.
- **`crates/ecs/src/spine.rs:32`** - `MAX_SPINE_SLOTS = 16` still refuses a 32-fighter
  match (`SpineError::Full`, `TooManySlots`). The perf side now pins only the relation
  (`MAX_SPINE_SLOTS <= MAX_PARTICIPANTS`), so the two move together without a fight; when
  the ecs lane raises its constant, the perf spine rows (`spine_transform_body`,
  `spine_full_tick`) move with it and must be re-recorded.
- **`crates/net`** - `LiveMatch::snapshot` / `from_snapshot` behind `cfg(not(feature =
  "sim"))` while `engine-net` calls them: the `sim` track of the perf matrix cannot be
  built or recorded until that is fixed (section 5).
- **`crates/web/src/input.rs:798`** - at HEAD the case demands `UnknownFighter(16)`, and
  slot 16 is legal now, so the case is red at HEAD. The web lane already has the file
  dirty in the tree (`UnknownFighter(MAX_PARTICIPANTS)`), so it needs nothing from this
  lane; it is listed here so the receipt does not look like an oversight.
- **`crates/arena` and `crates/render`** carry clippy debt at HEAD
  (`crates/arena/src/fork.rs:824` `question_mark`, `crates/arena/src/match_wire.rs:178`
  `large_types_passed_by_value`, `crates/render/src/present.rs:184` `type_complexity`),
  so the prescribed `cargo clippy -p <crate> --all-targets -- -D warnings` form fails for
  `engine-perf` and `engine-game` on their **dependencies** (`engine-render` is in both
  dependency trees) even though these four crates are clean. Those crates are not in this
  lane's ownership; the proof below therefore carries both the prescribed form (with the
  dependency failure named, at the file and line) and the `--no-deps` form (the lint of
  these four crates themselves). `engine-ui` and `engine-app` do not depend on
  `engine-render`, so their prescribed form passes as is.

## 9. Proof

All commands were run on the GitHub runner through `tools/gh-check.mjs` with `--only` on
exactly the paths this lane changed, so no other lane's dirty file could reach the build.

| command | result |
|---|---|
| `cargo test -p engine-perf --lib` | PASS, 149 tests, run 39543171297 |
| `cargo test -p engine-ui --lib` | PASS, 161 tests, run 39543183540 |
| `cargo test -p engine-app --lib` | PASS, 53 tests + 1 ignored, run 39543739620 |
| `cargo test -p engine-game --lib` | PASS, 476 tests + 6 ignored, run 39543549196 |
| `cargo test -p engine-perf -p engine-ui -p engine-app -p engine-game` (all targets, local) | PASS, 0 failed (perf lib 149, bin 27, it 53 + 3 ignored, profiler 10 + 1 ignored, paired 2 + 1 ignored, doc 1; ui lib 161 + 2; app lib 53 + 1 ignored; game lib 476 + 6 ignored, crash_hook 4 + 1 ignored, it 87) |
| `cargo clippy -p engine-perf --all-targets --no-deps -- -D warnings` | PASS, run 39543664188 |
| `cargo clippy -p engine-ui --all-targets -- -D warnings` | PASS, run 39543530707 |
| `cargo clippy -p engine-app --all-targets -- -D warnings` | PASS, run 39543599246 |
| `cargo clippy -p engine-game --all-targets --no-deps -- -D warnings` | PASS, run 39543594598 |
| `cargo clippy -p engine-app --all-targets --no-deps -- -D warnings` | PASS, run 39543547710 |
| `cargo clippy -p engine-perf --all-targets -- -D warnings` | FAIL on a **dependency**: `crates/render/src/present.rs:184` `clippy::type_complexity` (render lane, not this lane's file), run 39543609172 |
| `cargo clippy -p engine-game --all-targets -- -D warnings` | FAIL on the same dependency lint (`crates/render/src/present.rs:184`), run 39543573654 |
| `cargo fmt -p engine-perf -p engine-ui -p engine-app -p engine-game -- --check` | PASS, exit 0 |
| `node tools/rust-quality.mjs --crate perf --crate ui --crate app --crate game` | `RESULT PASS: score 97 (A), 0 ratchet regressions over 4 crate(s)` |

### 9.1 Results

Every runner run above pushed the same tree: HEAD `37c33077` plus the 23 paths this lane
changed (`--only` on exactly those paths, so no other lane's dirty file could reach the
build). The two prescribed-form clippy failures are the render lane's debt, named at the
file and the line; the `--no-deps` runs are the lints of these four crates themselves and
they are green.

The local all-targets run above is the only line that saw the shared working tree: other
lanes had their own dirty files in it (`crates/input`, `crates/arena`, `crates/endpoint`,
`crates/playtest`, `crates/spatial`, `crates/web`, `Cargo.lock`). That run still passed,
so the four crates also compile and pass next to the other lanes' work in progress; the
runner runs are the isolated proof, because only this lane's paths were pushed there.

The scorecard (production code only; tests are excluded from the counts):

```
crate      kloc  score  gr  unwrap  panic  casts  allow  print  strErr  ids  hash  dup  longFn  bigF  hotAl  nondet  float  undoc  tests
game       32.8     96   A       0      6    129      0      0       0    0     0    1       4     4      0       0      0      0   1012
perf        5.0     98   A       0      5     54      0      0       0    0     0    0       1     0      0       0      0      0    246
ui          4.5     98   A       0      0     24      0      0       0    0     0    0       1     1      0       0      0      0    163
app         1.5    100   A       0      0      1      0      0       0    0     0    0       0     0      0       0      0      0     54
RESULT PASS: score 97 (A), 0 ratchet regressions over 4 crate(s)
```

No `as` cast and no new panic site was added in production code: the cap is carried by a
literal pinned with `const _: () = assert!(... == 32)`, and the perf baseline lookup keeps
its single `expect` (two expects were merged into one).

The golden did not move: the legacy null fixture (`cargo run -p engine2040 --locked
--release --quiet`) still prints `ticks=600 final_hash=d57b5c8ae3096614` twice in a row
(the CI `legacy null fixture` job greps exactly that line, `.github/workflows/ci.yml:199`).


## 10. Deliberately left at 16

- `crates/game/src/siege.rs:60` - `SHARD_FIGHTERS = 16`: a shard size for a 32v32
  composition, never a reopened cap (sweep: deliberate).
- `crates/game/src/fighter_look.rs:586,632` - 16 body parts per fighter slot (atlas
  quadrants): a stride, not a capacity (sweep: deliberate).
- `crates/game/tests/it/config_vectors.rs:121,129` - a `fighters` row valued 16: a legal
  value inside `2..=32`, used as a parse vector (sweep: deliberate).
- `crates/ui/src/lobby_ui.rs:1109-1123` - the ghost storm over 16 hosts: `max_hosts` is
  `MAX_PARTICIPANTS` (32), so 16 is a legal fixture well inside the cap (sweep: deliberate).
- The perf matrix keeps its 16-fighter row: it is a size bar beside the cap row, never a
  claim about the cap (sweep: deliberate-gap, with the gap closed by the new cap row).
- The spine slot count (`crates/ecs/src/spine.rs:32`, 16 today): the ecs lane's constant;
  this lane pins only the relation and does not copy the number.
