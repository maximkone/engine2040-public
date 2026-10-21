# Fix: a vacuous pass — a row's Accept that runs zero tests

Owner of this file: the GATE FIXER lane (2026-10-21). Scope: `tools/proof.mjs`,
`tools/finish.mjs`, `tools/product-check.mjs`, `crates/playtest/src/living.rs` and
their test files, plus this report. Nothing else was edited. No `git add`, no
commit, no push: the lead commits by path. Golden `d57b5c8ae3096614` untouched
(no hashed path is in this diff).

## 1. The defect

A row's Accept is usually `cargo test -p <crate> --lib <filter>`. When the filter
matches no test, cargo prints

```
running 0 tests
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 214 filtered out
```

and exits **0**. A builder who reads the exit code ticks the row on a run that
checked nothing. `docs/research/powerups-2026-10-20/board-audit-2.md` counted 84
open rows in exactly that shape (evidence class **A**), and
`docs/plan/forge-adoption.md` A.2 #5 and A.3 #4 name the same disease.

AD-04 already landed two halves of the rule: commit `022cf596` gave
`engine-playtest::Verdict` a `checks_run` count (`crates/playtest/src/lib.rs:242-296`,
`from_checks` adds a `NoChecks` failure when the count is zero), and `870231e1` put
`ranAtLeastOneTest` in `tools/proof.mjs` for the named consumer legs. The hole was
the **other** grading paths: each one still turned exit 0 into a pass.

## 2. Every path an Accept is run through, and whether it could pass on zero

| path (file:line at HEAD `3b853cb9`) | what it grades | pass on zero tests? |
|---|---|---|
| a bare `cargo test ... <filter>` typed by hand | the shell exit code | **yes, and no gate in the tree ever sees it.** Only wrapping the line in a tool below closes this. |
| `tools/proof.mjs:669` — the workspace leg loop (`cargo test -j4 --locked --workspace ...`) | `result.ok` (exit code) only | **yes** — a leg whose filter/exclude set matches nothing exits 0. |
| `tools/proof.mjs:691-704` — the named consumer legs | `result.ok && ranAtLeastOneTest(output)` | no (`870231e1`). |
| `tools/finish.mjs:423` — `runProbe`, the `--probe` mode of the finish scoreboard (`A6`, `A7`, `A9`, `A10`, `P3`, `P10`) | marker regex, then `minPassed` **only when the entry sets it** | **yes** — `--probes <fixture>` (or any entry without `minPassed`) passes `test result: ok. 0 passed` and TAP `# pass 0`. Its own test at HEAD only covered the `minPassed: 1` case. |
| `tools/product-check.mjs:158-188,352` — the keeper's product verdict | `redFloors(...)` empty ⇒ `GREEN` | **yes** — an empty capture dir scores `captures 0`, `blank 0`, `frozen 0`, so the look half judges nothing and the verdict is GREEN. |
| `crates/playtest/src/lib.rs:242-296` — `Verdict` | `checks_run` > 0 (else `NoChecks`) | no (`022cf596`); test `verdict_checks_run_zero_is_never_a_pass`. |
| `crates/playtest/src/living.rs:1320-1343` — `LivingVerdict` | `pass = bad == 0 && replay_bad == 0` over `rows` | **yes** — a report with zero rows has no bad row, so it passed. |
| `crates/playtest/src/mirror_kit.rs:92` — mirror `Outcome` | `!checks.is_empty() && all ok` | no; test `empty_check_lists_never_pass`. |
| `crates/playtest/src/charter.rs:114` — `CharterVerdict` | the pass line is in the log | no (an empty log has no pass line). |
| `tools/gh-check.mjs:431` — the GitHub runner result | `testsRan(log) === 0` ⇒ vacuous | no: it reruns on the PC or exits 2. |
| `tools/gh-check.mjs:218` — `runLocal`, the PC fallback | `r.status === 0` (stdio inherited, so it cannot read the log) | **yes** — see §6; not my path to edit. |
| `crates/proof/src/soak.rs:172` — `analyze` | refuses under `MIN_SAMPLES` | no. |
| `crates/proof/src/matrix.rs:272` — `MatrixVerdict` | tape + corpus digests | not a test-count verdict; left alone (another lane). |
| `tools/board-slice.mjs` | prints rows; runs nothing | no. |
| `tools/ticked-rows-audit.mjs:289-297` | static `filter-missing` rule over **ticked** rows only | catches the shape without running cargo, but not the open rows — see §6. |

## 3. What changed (small, in the place the check belongs)

- `tools/proof.mjs:354-363` — new exported `legVerdict(isTest, ok, output)`: a
  compile/exit failure is red, a **test** leg that ran zero tests is red
  (`"zero tests ran (filter matched nothing)"`), other legs grade on exit. The
  workspace leg loop at `:682` and the named consumer loop at `:711` both use it,
  so the two loops can no longer disagree.
- `tools/finish.mjs:423-431` — `runProbe` refuses a run that reports tests but ran
  none, **independent of `minPassed`**: `test result:` present with `passed === 0`,
  or TAP `# pass 0`, is a fail. A probe can no longer be green because a fixture
  forgot `minPassed`.
- `tools/product-check.mjs:180-184` — `redFloors` adds `red.captures` when the run
  scored no captures: an empty capture dir proves nothing about the picture, so the
  verdict can no longer be GREEN without having looked at a frame.
- `crates/playtest/src/living.rs:1320-1343` — `verdict()` passes only when at least
  one row ran, and its reason says `0 rows ran ...` on an empty report.

No second gate: every change is inside the tool or type that already gave the
verdict.

## 4. The tests, red before and green after

| test | before the fix | after |
|---|---|---|
| `crates/playtest/src/living.rs::tests::zero_rows_is_never_a_pass` (new) | red: old `pass` is `bad == 0 && replay_bad == 0`, true on an empty report — `assert!(!verdict.pass)` fails | green |
| `tools/finish.test.mjs::runProbe enforces marker, exit and minPassed` (extended: the same hollow run with **no** `minPassed`, plus TAP `# pass 0`) | red on HEAD's `finish.mjs`: `AssertionError: a run that checked nothing fails without minPassed — 'pass' !== 'fail'` | green |
| `tools/product-check.test.mjs::a run that scored no captures is RED, not a silent green` (new) | red on HEAD's `product-check.mjs`: `redFloors` returns `[]`, the verdict would be GREEN | green |
| `tools/proof.test.mjs::legVerdict: a test leg that ran nothing is RED, every other cleared leg is green` (new) | the rule did not exist at HEAD (the leg graded `result.ok` only), so the module has no `legVerdict` to import | green |

The "before" runs used HEAD's modules (`git show HEAD:tools/<tool>.mjs` into a temp
dir) with the new tests; nothing in the tree was reverted to show red.

## 5. Proof

Node tool tests (the three files this lane touched):

```
node --test tools/proof.test.mjs tools/finish.test.mjs tools/product-check.test.mjs
ℹ tests 72  ℹ pass 72  ℹ fail 0
```

Rust (the new verdict test; the fix was briefly reverted to show the red, then re-applied):

```
cargo test -p engine-playtest --lib zero_rows_is_never_a_pass -> 1 passed; 0 failed; 0 ignored (exit 0)
```

The already-landed zero-checks rule, for the record:

```
node tools/gh-check.mjs cargo test -p engine-playtest --lib verdict_checks_run
```

## 6. Left for others (the hole in the gate is closed; these are not mine)

1. **The 84 rows.** That is the board fixer's job: each Accept filter must name a
   test that exists (or the row must carry the test it will add). The static rule
   already exists in `tools/ticked-rows-audit.mjs:289-297` (`filter-missing`: no
   `fn`/`mod`/file name holds the filter) but the tool only reads `- [x]` rows.
   Extending it to open rows (an `--open` mode over `parseBoard`) would name all 84
   without running cargo; that file is not this lane's.
2. **`tools/gh-check.mjs:218` `runLocal`** still returns a pass on exit 0 alone. The
   PC fallback uses `stdio: "inherit"`, so it cannot read the log; the fix is to
   capture stdout and apply the same `testsRan(...) === 0` rule the remote path uses
   at `:431`. Not this lane's file.
3. **`crates/playtest/src/keypoint/inject.rs`** (untracked, another lane's work right
   now): `UnitVerdict.pass` is true when `failures` is empty, with no count of the
   checks that ran. Same rule wanted: a unit with zero keypoints is never a pass.
4. **`tools/gh-check.mjs:95` `GPU_SRC`** matches `no (GPU|adapter)` inside plain
   prose: `crates/playtest/src/living.rs:1070` ("no adapter call" about a provider
   mock) routes every `-p engine-playtest` line to the slow PC. Worth a narrower
   pattern (`request_adapter|SKIPPED`), otherwise this lane's own proof runs locally.
5. **A filter in a hand-typed command** can never be caught by a tool the person does
   not run. The standing rule that closes it is process, not code: an Accept is
   proven by `node tools/gh-check.mjs <the cargo line>`, which refuses a vacuous pass
   on the runner, never by pasting cargo's exit code.
