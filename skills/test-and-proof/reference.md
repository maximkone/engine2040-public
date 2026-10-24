# Test and proof — reference

Moved out of `SKILL.md`: the capture checklist. The eyes rule and the evidence pack stay in `SKILL.md`.

## Captures: pixels, never counts

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

## 6. Repo facts

- `crates/game/src/bin/arena.rs::main` installs the crash hook first (`crash::install_run_hook`). `crates/game/tests/crash_hook.rs` is the forced-panic test (it sets `E2040_TEST_PANIC_AT_TICK`). A new binary must do the same (rule 6).
- The arena window has `--require-framing` and writes its status into `target/logs/arena-<UTC>.log` and `crash-<UTC>.toml`.

Sources: Finska, Modern Game Testing, ch. 8; Gregory, Game Engine Architecture 2nd ed., ch. 9.1.5 and 9.7 (book `geaen`); Klabnik et al., The Rust Programming Language 3rd ed., ch. 11.
