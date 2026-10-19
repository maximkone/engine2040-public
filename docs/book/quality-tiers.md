# Quality tiers: what each one cuts

Owner rule (2026-10-11): every game must look great and run on a potato. The 3050 with 16 GB is the comfortable tier; the floor is an integrated GPU with 2 to 4 GB of video memory and 8 GB of RAM. Three ladders carry that rule today. Each one is plain data in one file, and a test pins the numbers on this page to those files.

## 1. Render quality: `Quality` in `crates/render/src/quality.rs`

Pure integer data; `table_hash` is a regression checksum over the fixed field order. The knobs of each tier:

| Knob | Unit | Potato | Comfort | High |
|---|---|---|---|---|
| `render_scale_pct` | % | 67 | 100 | 100 |
| `shadow_cascades` | count | 1 | 3 | 4 |
| `shadow_map_px` | px | 1024 | 2048 | 4096 |
| `gtao` | level | Off | Half | Full |
| `bloom_half_res` | bool | false | true | true |
| `taa` | bool | false | true | true |
| `msaa` | x | 0 | 0 | 4 |
| `aniso` | x | 0 | 4 | 8 |
| `stream_pool_mb` | MB | 200 | 700 | 1400 |
| `particle_cap` | count | 512 | 4096 | 8192 |
| `decal_cap` | count | 64 | 256 | 512 |
| `crowd_cap` | cards | 1500 | 5000 | 10000 |
| `look_potato_guard` | bool | true | false | false |
| `physfx_bodies` | bodies | 96 | 384 | 768 |
| `physfx_ragdolls` | ragdolls | 2 | 8 | 16 |
| `physfx_contacts` | pairs | 256 | 1024 | 2048 |
| `texture_max_px` | px | 512 | 2048 | 4096 |

When a frame is over budget, cut in this fixed order (`CUT_ORDER`): crowd, particles, aniso, motion blur, ssr, ao, shadow cascades, render scale. There is no `Web` row here yet; the web tier is board row WEB8 and, until it lands, a browser tab should pick Potato.

Check it:

```sh
node tools/gh-check.mjs cargo test -p engine-render --lib quality_monotone_tiers
```

## 2. Lighting tier: `Tier` in `crates/render/src/tier.rs`

Picked once at startup from the adapter, never per frame. `base` runs on any adapter, including software rasterizers. `rt` needs ray queries and acceleration structures (ray-query shadows, ReSTIR, reflections, denoiser). `ultra` also needs 8 GiB of device memory. The reference RTX 3050 6 GB laptop GPU probes to `rt`, never `ultra`. A forced tier whose features are missing is refused loudly.

## 3. Physics caps: `Tier` in `crates/physfx/src/caps.rs`

The physical layer (see [Physics layers](physics-layers.md)) has four tiers, one of them for a browser tab:

| Cap | Potato | Web | Comfort | High |
|---|---|---|---|---|
| bodies | 96 | 192 | 384 | 768 |
| ragdolls | 2 | 4 | 8 | 16 |
| contacts | 256 | 512 | 1024 | 2048 |
| statics | 64 | 128 | 256 | 512 |
| solver iterations | 2 | 3 | 4 | 6 |
| CCD substeps | 0 | 1 | 1 | 2 |
| blast bodies | 24 | 48 | 96 | 192 |
| impact bodies | 3 | 4 | 6 | 8 |
| tethers | 2 | 3 | 6 | 12 |
| tether nodes | 8 | 12 | 20 | 32 |
| step bar (microseconds) | 400 | 800 | 1500 | 2000 |

The step bar is five times the mean measured on a GitHub runner on 2026-10-17 (72, 161, 266 and 380 microseconds) for the plan scene cut to what the tier admits: 8 ragdolls, 200 debris bodies and 6 limbs. A slowdown of that size fails the soak; runner noise does not. At the cap the oldest body is recycled first, and a whole ragdoll counts as one unit.

Check it:

```sh
node tools/gh-check.mjs cargo test -p engine-physfx --features rapier --lib every_tier_table_is_valid_and_climbs
```

## The potato gate

Gate G9 runs the arena on the integrated GPU with downlevel limits and compares three numbers with `tools/potato-bar.json`: p50 50 ms, p99 80 ms and a working set of 3072 MB. They are proposals until the owner accepts them; raise them only with a receipt.

```sh
node tools/potato-check.mjs --help
```

`node tools/potato-check.mjs` prints the `potato:` line and one PASS or FAIL per number and writes `target/reports/potato-<date>.md`. It needs the arena binary and an integrated adapter; with no adapter evidence it exits 2. A run on a loaded machine is stamped `LOADED` and the `--floor` verdict is withheld (exit 3), because a number read while a build runs is not a claim of speed.

## Web limits that set the web tier

Measured by the wasm probe (`docs/research/web-2026-10-17/07-wasm-probe.md`): Rapier costs 1.7 to 1.85 times native in scalar wasm (1.4 to 1.55 with simd128), a tab is single-threaded, and WebGPU has no hardware ray tracing and no bindless textures, so the `rt` and `ultra` lighting tiers stay native. The plan's web bars are a wasm under 25 MB, a first bundle under 60 MB compressed, a peak memory under 1.5 GB and a first frame under 8 seconds. They are targets, not measurements.
