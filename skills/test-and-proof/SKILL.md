---
name: test-and-proof
description: "Use when you write or review a test, gate, capture check or timing assert, or before you write DONE, green, a number or a score: pixels not manifests, GPU skip counts, oracle first, command-exit-hash-excerpt-verdict pack."
---

# Test and proof

A green check must show what the player sees. A result without a pack is not DONE.
Run lines: `node tools/gh-check.mjs cargo test ...` (skill `github-check`). Local `cargo check -p <crate>` only iterates.

## 1. Truth rules

1. A gate that trusts the producer's own counts is fake. Check pixels or the rendered surface, never a manifest field.
2. A test that returns early when no GPU adapter exists passes by skipping. Mark it SKIPPED, count it, and fail when the reference-GPU leg requires it. Such tests print `SKIPPED <name>: no wgpu adapter on this machine`. Run GPU tests with `E2040_REQUIRE_GPU=1` so a missing adapter fails. A runner pass with 0 tests is not a pass.
3. Never assert wall-clock time in the default suite. Put the assert in `#[ignore = "timing: ..."]` and run it in release under `--perf` or `-- --ignored --test-threads=1`.
4. The fixed-step loop drops draws, never updates. Test the update count and the digest, not the draw count.
5. Capture from the presented surface, or say the capture is not the window. An offscreen CPU raster is a fallback and must be labeled.
6. A crash path must not depend on the subsystem that crashed. Keep the crash code std-only and install its hook first in `main`.
7. Write the expected outcome down before you run the check. Test the oracle, not the number.
8. Run smoke tests first and time-limit them.
9. Saying that something works in the window needs a gate that runs the window path. `arena` has `--require-framing`: it fails the run when any presented frame shows no fighter.

## 2. Captures: pixels, never counts

| # | check | flag |
|---|---|---|
| 1 | The shot copies the presented framebuffer (crate game). | An offscreen CPU raster labelled as the window. A manifest trusted for the picture. |
| 2 | Each shot is written to a fixed folder with a date and time in the name (game). | Overwrites the last good shots. Two runs share a name. |
| 3 | SSIM and colour deltas run on decoded RGBA bytes (playtest). | Similarity over PNG bytes: one pixel looks huge, a wrong frame looks close. |
| 4 | A fighter-colour pixel count above a floor on every checked frame (playtest). | A gate that passes on file counts while the frame shows no fighter. |
| 5 | A blank set fails the gate and a real capture set passes it (playtest). | A gate that cannot say no. |
| 6 | Capture options name HUD on or off and debug lines on or off (render). | A look verdict polluted by HUD quads, or missing the overlay it claims. |
| 7 | The resolution is stated. Quad tiles only when high-res is asserted (render). | A tiled combine with a shifted projection. A resolution statement with no pixels. |
| 8 | A motion statement is backed by a frame-rate sequence, not one still (game). | A frozen frame sold as smooth play. |
| 9 | A test where blank frames fail and staged captures pass, with node and cargo green (playtest). | Synthetic frames only. The window path never runs in any gate. |

Eyes rule (every visual, render and content check; other skills link here):

- Open every capture PNG with the Read tool. Open its reference next to it for look work.
- Write one line on what is on screen: the region, the object, what is wrong or right ("left wall, shadow acne").
- Score from the pixels, never from logs or numbers alone.
- No capture means unverified. Say "unverified" and stop. Never score a shot you did not open.
- Black, empty, magenta, NaN-speckled or T-posed frames are `FAIL - <reason>`.
- Read hit frames in order: anticipation, impact, hit-stop, reaction. Check silhouettes, UI and camera at each one.
- You see images, not sound. Rate audio only by numbers if they are given (loudness, clipping, hit sync).

## 3. Rust test shape

- Unit tests sit in `src` in a `#[cfg(test)]` module. A helper only a test uses is not compiled into the ship build.
- Equality with `assert_eq!` or `assert_ne!` so failure prints both sides. Put a message with the actual value in each assert. No test named `it_works`.
- Error cases: `#[should_panic(expected = "...")]` pinned to the right guard. A bare `should_panic` passes on any panic. A test that can fail with `Err` returns `Result` and uses `?`.
- Tests run in parallel. No shared file, dir or env between tests unless pinned to one thread.
- Run control: filter by name, run one file, park slow tests as `#[ignore]`.
- Integration tests live as modules of one binary: `crates/<c>/tests/it/` (each `tests/it/<name>.rs` plus one `mod <name>;` line in `main.rs`; shared setup is a module too). Run one with `node tools/gh-check.mjs cargo test -p <crate> --test it <name>::`.
- A new top-level `crates/<c>/tests/*.rs` fails `tools/workspace-check.mjs` (test layout guard). The exception is a file that needs its own process, listed in `OWN_TEST_BINARY` in that tool with a reason (for example `game/crash_hook`).
- Integration tests use the public API. Binary logic goes in a lib so tests can reach it.
- PASS needs a run line (`node tools/gh-check.mjs cargo test -p <crate>`), never a promise.

## 4. Evidence pack

NO RESULT WITHOUT A PACK. A report of green, a number or a score ships with five parts:

1. Command: exact, in pwsh form, with workdir, flags and seed.
2. Exit code.
3. SHA-256 of the exact output bytes (save the output to a file, then `Get-FileHash -Algorithm SHA256`). Hash what the command printed, not a summary.
4. Excerpt: at most 20 lines that show the pass or fail and the number.
5. Verdict: one line, PASS or FAIL plus the measured number. No adjectives.

Report line:
`EVIDENCE: <command> | exit <code> | sha <hash> | <verdict>`
Example: `EVIDENCE: node --test tools/fnv1a64.test.mjs | exit 0 | sha <64 hex> | PASS 7 pass 0 fail`

- Any missing part keeps the result DRAFT, not DONE.
- Keep these levels apart in every report: implemented, compiled, unit-tested, integrated, GPU-run, visually reviewed. Never mark a stub as passing (AGENTS.md).
- The hash is a replay aid: anyone can rerun the command and compare.

## 5. Repo facts

- `crates/game/src/bin/arena.rs::main` installs the crash hook first (`crash::install_run_hook`). `crates/game/tests/crash_hook.rs` is the forced-panic test (it sets `E2040_TEST_PANIC_AT_TICK`). A new binary must do the same (rule 6).
- The arena window has `--require-framing` and writes its status into `target/logs/arena-<UTC>.log` and `crash-<UTC>.toml`.

Sources: Finska, Modern Game Testing, ch. 8; Gregory, Game Engine Architecture 2nd ed., ch. 9.1.5 and 9.7 (book `geaen`); Klabnik et al., The Rust Programming Language 3rd ed., ch. 11.
