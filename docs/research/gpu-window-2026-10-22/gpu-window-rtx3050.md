# The GPU and the window legs, measured on the RTX 3050 (2026-10-22)

The GPU rows (RL5, RL18, RL20, RL27, RL28) are not a CI check: the mirror's
`windows-latest` runner has no adapter and no window, so they are a PC-only leg
and they stay OPEN until a dated receipt exists that names the commit it was
measured at. This is that receipt.

## What was measured, and where

- Commit: `b8b2fd542` ("Merge remote-tracking branch 'origin/master'"), the tree's
  HEAD when the leg ran. It was measured in a clean worktree of that commit
  (`C:\Users\me\AppData\Local\Temp\opencode\quiet-wt`), not in the shared tree:
  the shared tree carries another lane's in-flight render code at the time (a
  duplicate `pub mod torch_light;` and a missing `ForwardMapsError`, both in
  `crates/render/src/`), and `engine-render` does not compile with them. A
  receipt has to name a commit anyway, and a clean tree is the only way to get
  one.
- Machine: this PC, NVIDIA GeForce RTX 3050 6GB Laptop GPU, driver 32.0.16.1062,
  1280x720.
- Build: `cargo build --release -p engine-game --features app --bin arena`, exit 0
  in 8 m 21 s, `arena.exe` 20,236,288 bytes.

## The keeper's own eyes: the product check

`node tools/product-check.mjs --no-build` -> exit 0.

```
product-check: GREEN b8b2fd542 fps=60.0(loaded) empty=0 tinted=0.12 pops=5 sunk=0 ticks=0.6..291.2 flip=- blank=0 frozen=0 val=0
```

| line | number |
|------|--------|
| fps | 60.0 (p50 16.655 ms, p99 18.907 ms) |
| gpu p50 / cpu p50 | 0.92 ms / 5.689 ms |
| framing empty | 0 of 300 frames |
| fewest in view | 6 |
| tinted share | 0.12 |
| pops (joint pops per 300 frames) | 5 |
| sunk (cm) | 0 |
| blank frames / frozen pairs | 0 / 0 |
| validation errors | 0 |
| sim ticks | 0.6 .. 291.2 over 300 frames, 2400 bodies |

The fps line is marked LOADED: 9 cargo/rustc processes were running on the box
during the timing run, so the fps is reported, not gated. The house bar is 60 fps
at 1080p on this GPU (FC3 asks 30 first); this run is 720p and loaded, so it is
not a 1080p claim either.

Per-pass GPU times, 300 frames at 1280x720 (the product run): sky 73.116 ms,
ibl 8.545 ms, fog 7.973 ms, gtao 5.213 ms, decals 0.708 ms, layers 0.664 ms,
shadow 0.026 ms, particles 0.000 ms, forward 0.000 ms, validation errors 0.

The live GPU passes that ran: sky, layers, decals, ibl, fog, gtao, shadow.

## The daily arena shot (VS9)

`node tools/arena-shot.mjs --dir target/vs9 --no-build` -> exit 0.

```
arena-shot: PASS build=b8b2fd542 frames=3 masks=3 magenta_max=0.000% dir=target/vs9
```

The capture is the pinned one: `--scene arena --fighters 2 --camera-path arena600
--frames 120 --exit`, capture every 40 frames. The gate refuses placeholder
frames by name (the repo's pinned cube frame, a 64x48 noise frame, a fighter
with no box, magenta over 0.5%) and refused none.

| frame | tick | size | magenta | sha256 (first 16) |
|-------|------|------|---------|--------------------|
| frame_000000.png | 1 | 1280x720 | 0 | d4083df49f212db2 |
| frame_000001.png | 48 | 1280x720 | 0 | 94b9fb1bf51aff6 |
| frame_000002.png | 96 | 1280x720 | 0 | 105566330c7aff54 |

Per-pass GPU times, 120 frames at 1280x720 (the shot run): sky 25.699 ms,
ibl 6.690 ms, fog 5.468 ms, gtao 4.083 ms, decals 0.405 ms, layers 0.563 ms,
shadow 0.013 ms, particles 0.000 ms, forward 0.000 ms, validation errors 0.

## What the eye saw

Every PNG was opened and looked at (8 images: 3 from the arena shot, 5 from the
product run). The numbers do not replace the eye and the eye does not replace
the numbers.

- The three arena-shot frames are a real arena: sand with a rippled texture, a
  curved cavea wall of pale stone, coloured spectators in the tiers, a sunset sky
  grading from violet at the zenith to warm orange at the horizon.
- `frame_000000.png` is a single fighter seen close: a white-armoured gladiator
  with a dark kite scutum and a spear held across the body, standing on the sand
  with the wall at the right. The body reads as a body - limbs, greaves, a helm.
- `frame_000001.png` is the same fight from a different point of the orbit: the
  fighter is at the right edge by the wall, scutum forward, spear levelled. The
  camera moved and the world moved with it.
- `frame_000002.png` is a wide shot: the arena from further out, a fighter small
  at the left near the gate wall, a pale block on the sand (a prop), the stands
  and the sky above. A chunk of sky is visible in the upper right, which is the
  camera's own framing, not a hole in the render.
- The five product-run frames are an 8-fighter brawl. The HUD's eight health
  bars are on the left and they move: the bars drain over the sequence and the
  black (lost) portion grows. Fighters run, clash, and one spear is in flight in
  `frame_000002.png`. Blood decals appear on the sand by `frame_000004.png`.
  The spectator blocks are the same in every frame, and the fighters are not:
  no frozen pair.
- Nothing in any frame is magenta or blank. No frame is a solid colour, no
  frame is a 64x48 noise square. The gate's own refusals all stayed silent.

## What this receipt does NOT claim

- It is not a 1080p measurement. The capture is 1280x720. The house bar (60 fps
  at 1080p) is not met or missed here; it is untested by this leg.
- The fps line is LOADED (9 cargo/rustc processes on the box), so it is a report,
  not a gate. A quiet-box fps number is still owed for the 1080p bar.
- It does not tick the GPU rows. RL5 (baked GI), RL18, RL20, RL27 (shadow
  cascades) and RL28 each name a specific feature and a specific comparison
  (a capture pair with the feature on and off, or a per-pass ms bar for that
  pass). This leg proves the window opens, the GPU runs, the passes execute and
  the captures are real; it does not measure any one of those five features
  against its own bar. Each needs its own dated receipt.
- The `q1-check` and `frame-triage` legs did not run green, and they are not
  evidence of failure: both read pinned dated reference sets
  (`target/q1/2026-10-05/orbit` and `target/showcase/2026-10-08/orbit`), which
  are git-ignored and are not on this box. `q1-check` reports 0 PNGs and 2 gaps
  because the dirs are missing, not because a capture was refused. The Q1
  numeric receipt on this box (`target/q1/receipt.md`, 2026-10-21) says the same
  thing in its own words: "zero reference-GPU pixels claimed. GPU lines PENDING."

## Proof

Every command below ran on this PC, in the clean worktree of `b8b2fd542`.

- `cargo build --release -p engine-game --features app --bin arena` ->
  `Finished release profile in 8m 21s`, `_exit=0`, arena.exe 20,236,288 bytes.
- `node tools/product-check.mjs --no-build` ->
  `product-check: GREEN b8b2fd542 fps=60.0(loaded) empty=0 tinted=0.12 pops=5 sunk=0 ticks=0.6..291.2 flip=- blank=0 frozen=0 val=0`, `_exit=0`.
- `node tools/arena-shot.mjs --dir target/vs9 --no-build` ->
  `arena-shot: PASS build=b8b2fd542 frames=3 masks=3 magenta_max=0.000% dir=target/vs9`, `_exit=0`.
- `node tools/q1-check.mjs` -> `synthPng: 0, orbitPng: 0, totalPng: 0, manifests: 0`, exit 1, both capture dirs missing (a pinned reference set, not a refusal).
- `node tools/frame-triage.mjs` -> `GATE FAIL frames=0`, the pinned orbit dir `target/showcase/2026-10-08/orbit` is absent.
