# GX14 (GTAO boosted-release proof): pilot measurement, bar NOT proven — posted FAIL (receipt)

Written 2026-10-19 on the RTX 3050 reference box. GX14 is a pilot-measurement row
(no pass code): pin a release window build, run the arena at 1080p, print the PF2
per-pass GPU stamp for GTAO, and report the `0.8 ms p95 at 1080p` bar. This run
posts the FAIL with numbers, because the tree has no p95 stamp and no window-size
control; bars were not loosened and `gtao.rs` was not touched.

Anything in `target/` is evidence on disk, not a git path (the repo ignores
`target/`); the numbers below are also in the row's Progress line.

## Build and box

- Adapter line for every number here: `NVIDIA GeForce RTX 3050 6GB Laptop GPU`,
  driver `32.0.16.1062` (nvidia-smi 610.62). Present mode Immediate.
- Pinned release window build: `target/release/arena.exe`, built 2026-10-19 05:43
  local off the tree at `4082c7d1` (`cargo build --release -p engine-game
  --features app --bin arena`; sha256 `C5F98DF9…373596C`). The only later commit
  touching `crates/game`/`crates/render` is `42f1c2a0` (RZ7, CPU cloth capes,
  presentation only), so the render half of the binary equals HEAD. A fresh
  rebuild at HEAD was attempted first and refused by another agent's
  uncommitted `crates/net/src/wan_soak.rs` (E0560 at the time), so the pinned
  binary is one CPU commit behind; no render/gtao code differs.
- Clocks: not locked (that needs admin). Graphics clock sampled every 200 ms
  during the 1080p phase (`clocks-1080p.csv`, 815 samples): min 210, p50 1087,
  max 1732 MHz of 2100. So "boosted" is observed, not pinned.
- Box load: shared tree, other agents' cargo/rustc ran during the phase
  (CPU load was 83–94% on 2026-10-18 with the same pattern). Every number here is
  observed-only, exactly like the GX13/PF19 receipt; none is a baseline and the
  PF13 quiesce rule cannot be met on this box.

## Why there is no p95 (the bar's miss, with numbers)

- The PF2 GPU stamp set exists and does cover GTAO: the presenter's timestamp
  ring keeps one sample per resolved frame for the GTAO compute pass
  (`crates/render/src/present/gpu_present/timing.rs`, queries 6/7,
  `latest_gtao_ms`), and `GpuTimingReport.gtao` is a `Percentiles`
  (`crates/render/src/present/wait_stats.rs`).
- That `Percentiles` struct holds **p50 and p99 only**. There is no p95 field
  anywhere in the tree (`rg p95 crates/render crates/game` finds only unrelated
  bars). The bar in this row is a **p95**.
- The window prints the GTAO stamp as a **mean** over frames
  (`arena per-pass: … gtao={}ms`, `crates/game/src/bin/arena_frame/mod.rs`
  `per_pass_line`/`pass_mean`, from the newest resolved device sample per frame),
  and the frame-split line prints p50/p99 only for `total/forward/sky/blit`
  — never for `gtao`.
- The window also has no size control: `WINDOW_SIZE` is a const `(1280, 720)`
  (`crates/game/src/windowed.rs`), no `--size` flag exists.

So `gtao p95 ≤ 0.8 ms at 1080p` is **unproven**: not passed, and not refuted.
What would close it is code outside this row's owned paths: a `p95` field on the
shared `Percentiles` printed for the gtao cell, plus a window-size flag (PF2 /
GX5 territory; `gtao.rs` stays frozen).

## The 1080p target (reached without code)

The window is a normal winit window and the presenter resizes the surface on
`WindowEvent::Resized` (`presenter.resize`). This pilot found the app's **real**
window (by process id + `engine2040 arena` title, never the console window),
made it borderless (`WS_POPUP|WS_VISIBLE`) and forced the client area to
`1937x1080` with `SWP_NOSENDCHANGING` (the shell clamps a 1920 request to 1903),
then printed the client rect from `GetClientRect`. GTAO runs at ceil-divide half:
969x540 at that target versus 960x540 at exactly 1920x1080 (+0.9% texels).

`run-window.ps1` / `phase-1080p.ps1` in `target/evidence/gx5/` do this; each run
printed its verified client rect. One run's per-pass cell updated to the real
size (`live-1080p-r3.txt`: `resolution=1937x1080`) — the cell pins the first
presented frame, which is why the other logs still say 1280x720.

Mechanism control (same script, smaller target): client 640x360 gave
`gtao=0.125ms forward=0.117ms` against `gtao≈0.33/forward≈0.34` at 720p in the
same session, so the forced resize does reach the renderer (the pass gets
cheaper when the target shrinks). The difference between 720p and 1937x1080 is
the same direction but smaller (0.35 → 0.41), which is what a half-res AO pass
with fixed overhead should do.

## Table (600 frames each, 8 fighters, `--exit --timed-smoke`, `validation_errors=0` everywhere)

| run | client area | gtao ms | forward ms | gpu p50/p99 ms | validation errors |
|---|---|---|---|---|---|
| live-720p-r1 | 1280x720 (app default) | 0.348 | 0.329 | 0.897 / 6.551 | 0 |
| live-720p-r2 | 1280x720 | 0.375 | 0.341 | 0.930 / 4.322 | 0 |
| live-720p-r3 | 1280x720 | 0.337 | 0.318 | 0.890 / 3.669 | 0 |
| live-1080p-r1 | 1937x1080 forced | 0.426 | 0.394 | 1.322 / 3.529 | 0 |
| live-1080p-r2 | 1937x1080 forced | 0.441 | 0.485 | 1.405 / 3.120 | 0 |
| live-1080p-r3 | 1937x1080 forced | 0.365 | 0.381 | 1.085 / 2.388 | 0 |
| live-1080p-r4 | 1937x1080 forced | 0.414 | 0.389 | 1.102 / 3.002 | 0 |
| live-1080p-r5 | 1937x1080 forced | 0.402 | 0.384 | 1.089 / 3.001 | 0 |
| control-360p | 640x360 forced | 0.125 | 0.117 | — | 0 |

- `gtao` is the arena's per-pass cell: mean over the run's frames of the newest
  resolved **device-clock** sample of the GTAO compute pass. `forward` and
  `gpu_ms` come from the same ring.
- 1080p-class mean gtao 0.410 ms (range 0.365–0.441); 720p mean 0.353 ms
  (0.337–0.375). The 0.8 ms bar is a p95 at 1080p: the means sit about half the
  bar, but a mean is not a p95, and this box's spread (720p gtao alone ranged
  0.19–0.35 across sessions) means no claim can be read off them.
- Five 720p means on a comparably loaded box on 2026-10-18 were 0.188, 0.290,
  0.196, 0.313, 0.315 ms (GX13/PF19 receipt), same order as above.

## 1080p GTAO budget probe (existing ignored test)

The one existing exact-1920x1080 stamp is the ignored oracle-path budget probe
(`crates/render/src/gtao.rs` `gpu_1080p_budget_probe_reports_ms`, run release with
`cargo test -p engine-render --features window --release --lib
gpu_1080p_budget_probe -- --ignored --nocapture`, log
`target/evidence/gx5/gtao-1080p-probe-release.txt`):

```
window timestamps supported: true
GTAO 1080p probe on NVIDIA GeForce RTX 3050 6GB Laptop GPU: upload_ms=45.27 submit_ms=90.07 budget_ms=0.8
test result: ok. 1 passed; 0 failed
```

That test stages the full 1920x1080 depth+normals from the host and blocks on the
readback, so 45.27/90.07 ms are host-inclusive wall numbers, not device execution
(the test's own line says the budget row judges device execution). It proves the
1080p pass runs clean on the device (0 validation errors asserted) and that this
host-staged path is hundreds of times the bar by construction. It cannot speak to
the 0.8 ms p95 device bar either way.

## RA8 manifest

- Capture run: `--scene arena --fighters 8 --frames 300 --exit --capture-dir
  target/evidence/gx5/capture --capture-every 20 --rows GX14 --sha 4082c7d1`,
  exit 0, `validation_errors=0`, 15 PNGs, `per_pass_ms.json` beside them.
- Manifest: `target/evidence/gx5/ra8.json`, written by the tool's own generator
  (`generateRa8Manifest`) from the PNG bytes on disk: 15 frames, ticks 1..300,
  `sequence_fnv=e7cd5be5c38a77de`, `renderer` and `device` set from this run.
  `validateRa8Manifest` returns `ok: true`.
- Honesty note: the capture path renders at its **own** fixed 1280x720 size (the
  PNGs and `manifest.json` say 1280x720), so the RA8 manifest is size evidence
  for the capture path only, not for the 1080p live target. The 1080p target is
  evidenced by the forced client rect plus the mechanism control above.

## llvmpipe note

- This PC has no llvmpipe/lavapipe adapter: the only adapters are the NVIDIA
  RTX 3050 and the Intel UHD iGPU (`Get-CimInstance Win32_VideoController`), and
  a software fallback would need `E2040_ALLOW_SOFTWARE=1` plus a lavapipe ICD
  that is not installed.
- The GTAO pass has no CPU/software submit path at all, and the device stamp
  needs `Features::TIMESTAMP_QUERY`, which llvmpipe does not expose. So no
  llvmpipe number for this row exists or can exist; a llvmpipe "pass" would be a
  meter that cannot read.
- The cross-platform leg is the CPU oracle/parity suite instead:
  `cargo test -p engine-render --lib -- gtao` = 30 passed, 0 failed on this PC
  (2026-10-19) and the same 30 on the Linux runner
  (`node tools/gh-check.mjs --remote-only cargo test -p engine-render --lib -- gtao`,
  run https://github.com/maximkone/engine2040-public/actions/runs/39211727608).
  `tools/gh-check.mjs` classifies `crates/render/src/gtao.rs` as GPU code and
  refuses to send it remotely, which is why that leg had to be forced with
  `--remote-only` to get the runner row.

## VERDICT (posted FAIL)

- Bar `gtao p95 ≤ 0.8 ms at 1080p` on the reference GPU: **NOT PROVEN**. The
  measurement that the bar names does not exist in the tree (no p95 stamp, no
  size flag), and the closest honest numbers are device-clock means of 0.365 to
  0.441 ms at a 1937x1080 target (0.337–0.375 at 720p) in five 1080p-class and
  three 720p 600-frame runs on a shared box, with 0 validation errors.
- `0 validation errors`: met in every run above (720p, 1080p, captures).
- `node tools/proof.mjs`: see below.

## proof.mjs

`node tools/proof.mjs --skip-build` ran on this tree (2026-10-19, log
`target/evidence/gx5/proof-skipbuild.txt`). The golden legs this row names are
green:

```
[PASS] legacy golden — d57b5c8ae3096614 engine=0.1.0 workspace=0.1.0
[PASS] seed 0 repeated — 1e2c8ec791c4d062
[PASS] seed 2 repeated — 7e401aa2cfa39055
[PASS] seed 2040 repeated — fc0a8a2bf4d09353
[PASS] seed 999983 repeated — e75f39caf0d10af5
[PASS] different seeds give different receipts
[PASS] saved nonempty input tape replay — 3982e34dde17cda5
[PASS] corrupted checkpoint fails with first divergent tick
[PASS] unsupported proof backend cannot fall back silently
[PASS] spine parity column / canary
RESULT FAIL
```

The RESULT FAIL is three ratchets, none of them from this row's files (this row
touched docs only, no Rust, `git status` shows no `crates/render` or
`crates/game` diff):

- GP5 engine diff: `crates/material/src/twin.rs` uncommitted by another agent.
- rust-quality: `engine-render dup_fns 8 -> 9` (committed code, no render diff).
- docs coverage: four undocumented endpoint items.

So this row leaves the overall proof gate red by other rows' state; the golden x2
leg itself is proven, and the tick waits for a clean tree.
