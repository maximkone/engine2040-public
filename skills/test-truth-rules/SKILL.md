---
name: test-truth-rules
description: Use when you write or review a gate, a test, a window capture, a crash check or a timing assert, before anyone calls a check green or claims the window works.
version: 0.1.0
author: skillworks
---

# Test truth rules

Use when you write or review a gate, a test, a capture, a crash check, or a timing assert, or before you call a check green. A green check must show what the player sees.

1. A gate that trusts the producer's own counts is fake. Check pixels or the rendered surface, never a manifest field. [testing ch.9]
2. A test that returns early when no GPU adapter exists passes by skipping. Mark it SKIPPED, count it, and fail when the reference-GPU leg requires it. [testing ch.1]
3. Never assert wall-clock time in the default suite. Put the assert in `#[ignore = "timing: ..."]` and run it under `--perf`. [rustwasm ch.9]
4. The fixed-step loop drops draws, never updates. Test the update count and the digest, not the draw count. [rustwasm ch.3]
5. Capture from the presented surface, or say the capture is not the window. An offscreen CPU raster is a fallback and must be labeled. [crates/game/src/bin/arena.rs header; geaen 9.7]
6. A crash path must not depend on the subsystem that crashed. Keep the crash code std-only and install its hook in `main`. [geaen 9.1.5; crates/game/src/crash.rs header]
7. Write the expected outcome down before you run the check. Test the oracle, not the number. [testing ch.1]
8. Run smoke tests first, and time-limit them with a benchmark. [testing ch.3]
9. Hands-on Rust has no tests. Do not cite it as a test model. [handsrust]

## Repo facts

- `crates/game/src/bin/arena.rs` installs the crash hook first in `main` (`crash::install_run_hook`), and `crates/game/tests/crash_hook.rs` is the forced-panic test. A new binary must do the same (rule 6).
- `crates/game/src/bin/arena.rs`: the window path runs in no gate. A J1 "first picture" walk scored 0 of 10 because no capture shows a fighter (testing review).

Search more: `node tools/books.mjs search "game testing quality criteria"`
