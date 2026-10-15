# CT41 LOD ms/VRAM table over the CT35 bake

Packet: CT41 (content, Tier A, Size S). Lane `builder-content-r3`.
Owns only this file. `content/fighters/lod/` frozen (CT35 ticked),
`crates/assets/src/lod_swap.rs` frozen (CT5 ticked). No code touched,
no bytes cooked, nothing committed. Needs: CT35 (ticked), CT5 (ticked).

## Source (frozen)

- `content/fighters/lod/hero.json` (CT35): 4 meshes, Hero class,
  `class_max_tris 40000`, `asset_digest fcc2708f5643c9ef`,
  `"cook_x2_identical": true`, `"gate": "check_chain:ok"`,
  `"consumer": "CT41 LOD ms/VRAM table + lod_swap picker"`.
- Source bytes: `content/fighters/glb/hero_body.glb`, 2072320 bytes.
- Swap bands (frozen, `lod_swap.rs`): LOD0 below 15 m, LOD1 to 30 m,
  LOD2 to 60 m, LOD3 beyond; hysteresis 2000 mm; max 8 swaps/tick.

## Bake line (pasted, not invented)

Recook 2026-10-09 on root (tool cooks the same bytes twice, refuses
on drift; exit 0 means identical):

`CT35-LOD-BAKE meshes=4 mesh0 lod_tris=[1448,723,362,180] digest=2ea9b9b1b504edc2 mesh1 lod_tris=[2232,1115,558,279] digest=cc1d9ea78c322a9f mesh2 lod_tris=[1434,717,359,180] digest=06e81b6c8ee1b370 mesh3 lod_tris=[736,368,184,92] digest=a898fb422d898f35 asset_digest=fcc2708f5643c9ef gate=check_chain:ok`

Byte-identical to the CT35 receipt line. Mesh rows of the recook JSON
match the committed `hero.json` exactly (diff of the digest/tris/bytes/
hull rows: no output).

## Per-LOD table (exact cook numbers, pasted from `hero.json`)

Payload bytes are `lod_bytes` (attributes plus indices) per LOD, the
same numbers the cooker and the CT35 sidecar carry. GPU-resident VRAM
and GPU ms are PENDING (see below); nothing below is estimated.

| mesh | LOD0 tris/bytes | LOD1 tris/bytes | LOD2 tris/bytes | LOD3 tris/bytes | hulls |
|---|---|---|---|---|---|
| 0 | 1448 / 61536 | 723 / 32340 | 362 / 17208 | 180 / 9504 | 4/4 contain |
| 1 | 2232 / 92736 | 1115 / 46740 | 558 / 23880 | 279 / 12468 | 4/4 contain |
| 2 | 1434 / 73704 | 717 / 32316 | 359 / 17316 | 180 / 8496 | 4/4 contain |
| 3 | 736 / 31008 | 368 / 16128 | 184 / 8544 | 92 / 4800 | 4/4 contain |
| **hero total** | **5850 / 258984 (252.9 KiB)** | **2923 / 127524 (124.5 KiB)** | **1463 / 66948 (65.4 KiB)** | **731 / 35268 (34.4 KiB)** | **16/16 contain** |

- Monotonic: every chain strictly decreases
  (1448>723>362>180; 2232>1115>558>279; 1434>717>359>180;
  736>368>184>92), enforced by `check_chain`.
- Ratios of totals: 1.0 / 0.4997 / 0.2501 / 0.125 against
  `LOD_RATIOS [1.0, 0.5, 0.25, 0.125]`.
- LOD0 budgets: 1448, 2232, 1434, 736 all under Hero max 40000.
- 64-fighter payload scale (exact multiples for the CT27 soak):
  all-LOD0 64 x 258984 = 15.81 MiB; all-LOD3 64 x 35268 = 2.15 MiB.
  These are cooked-payload bytes, not GPU-resident VRAM.

## GPU ms / VRAM (reference GPU, PENDING with the adapter line)

- adapter=NVIDIA GeForce RTX 3050 6GB Laptop GPU
  driver=32.0.16.1062 (Win32_VideoController DriverVersion)
  power=Ultimate Performance (active scheme on this box)
- Per-LOD GPU ms: PENDING. GPU-resident VRAM per LOD: PENDING.
- Quiesce rule (from PF13): record only with CPU load <10%, 3 quiesced
  runs within +/-10%, 21 samples with per-sample hash assert; a loaded
  box is observed-only, never baseline. This box ran builds during the
  recook, so no GPU number is claimed here.
- Follow-up: the quiesced reference-GPU capture fills the ms/VRAM cells
  pilot-only (PF13 -> PF19 pattern); this file's payload rows are the
  compare-against input. Engine convention (`backend.rs`): GPU PENDING
  and VRAM unmeasured-headless markers stand until that run, never a
  fake number.

## Accept facts

- `cargo test --locked -p engine-assets --features import --lib lod_swap`
  9 passed (`bands_select_expected_lods_from_base`,
  `hysteresis_holds_finer_level_inside_depth`,
  `jitter_inside_hysteresis_never_swaps`,
  `headless_match_capture_lands_expected_lods`,
  `poll_budget_defers_in_actor_order`,
  `chains_from_the_cooker_register_and_refuse`,
  `bad_inputs_refused_without_state_change`,
  `full_event_log_applies_nothing`,
  `steady_state_poll_allocates_nothing`), 0 failed.
- `cargo test --locked -p engine-assets --features import --lib mesh_cook`
  13 passed (incl. `cook_identical_twice`,
  `hulls_contain_their_mesh`, `lod_chain_meets_triangle_budgets`,
  `over_budget_meshes_refused_with_counts`), 0 failed.
- Cook x2 identical: the bake tool cooks twice and refuses on drift;
  exit 0 plus the pasted line above. Recook mesh rows match `hero.json`.
- 0 tick allocs: proven by `steady_state_poll_allocates_nothing`
  (600-tick jitter poll, capacities unchanged), green above.
- PF13 join: `tools/perf-baseline.json` (11 rows: 4 CPU baseline pins,
  6 GPU passes + VRAM PENDING with box-conditions lines, quiesce rule
  pinned) is the compare-against file; this table's payload rows plus
  its PENDING GPU cells use the same adapter-line + quiesce shape.
  That file is untouched (owned by PF13).

## Consumer

- PF13: payload rows + PENDING GPU cells in this file compare against
  the PF13 baseline; the quiesced fill lands pilot-only.
- CT27 soak: 64-fighter payload multiples (15.81 MiB all-LOD0,
  2.15 MiB all-LOD3) plus bands/hysteresis above.
- CT34 card: per-band tris/bytes rows feed the 20k scale card.

## Standing conditions (not failures)

- No GPU ms or GPU-resident VRAM number is posted in this packet; the
  cells are PENDING with the adapter+driver+power lines above, per the
  quiesce rule. A number without that run would be invented.
- `lod/` and `lod_swap.rs` frozen: no hunks beside them in this lane.
