# BX25: Blender-bpy fracture column for BreakBench

Status: **spike PASS at fixture level** (2026-10-23). ADOPT as a BreakBench
reference column, not as engine code. The parent session ticks the board row;
this seat does not.

Row: BX25 (`docs/SPRINT.md`), needs BX12 (ticked). Pillar: Bodies
(`docs/FOCUS.md` Now item 2). Blender 5.2.0 LTS on this PC.

## Question

The G6 premise (vision-r4, 2026-09-29) says only Blender
`--background --python` plus `bpy.ops.object.add_fracture_cell_objects` and
Houdini HDA are headless-scriptable for fracture. Can Blender's cell fracture
serve as a BreakBench column on the engine's wall fixture: does it produce the
expected cell count, and is its bake byte-identical twice?

## Baseline and fixture

- Engine wall (`crates/destruction/src/fracture.rs::wall`):
  `[-2, 2] x [0, 3] x [-0.2, 0.2] m`, 12 triangles. Hit `[0.3, 1.4, 0.2]`,
  `spec(24, 7)`. Test `same_hit_gives_identical_chunk_sets_twice` prints
  **24 chunks**, digest `e8370912b2c4e50a`, and asserts its bytes and digest
  equal twice. gh-check run [39922766956](https://github.com/maximkone/engine2040-public/actions/runs/39922766956)
  from base `18d25d0e2`; the same test passed at `a3a153b56` (run
  [39924019609](https://github.com/maximkone/engine2040-public/actions/runs/39924019609)).
- Material: stone, the first BX12 row (`crates/destruction/src/material.rs:524`):
  toughness 8.0, brittleness 0.85, shard rule `FewLarge`.
- Blender column: the same wall in Blender space (x `[-2, 2]`,
  y `[-0.2, 0.2]`, z `[0, 3]`); 24 sites from `sites-24.json` (`sites.py`,
  scipy 1.17.1 Latin-hypercube, seed 7, minimum separation 0.179 m).
- The engine's site cloud is its own integer RNG, not the scipy cloud. The
  fixture matches the **cell count**, not the cell shapes.

### The BX12 energy-table caveat

BX12 is a material table; it has no impact-energy to cell-count column.
BX14 (fracture takes cell count/size from energy and material) is not landed
and has no board row. The only expected count available today is therefore the
engine spec's **24**. This spike matches that count and pins the stone row; it
does not claim a BX12 energy rule. When BX14 lands, rerun with its count; a
mismatch kills the column.

## Tool and one-time install

Blender 5.2.0 LTS (build hash `fbe6228777e7`). Cell Fracture is not bundled
in 5.2; it is the official extension **Cell Fracture 0.2.1** (Blender 4.2+),
GPL-3.0-or-later, 9,722 bytes, SHA-256
`878bbd46e0c735da689101aeed9ebe99f18c0d8d0cbd2e9e17ed8cda6b264fb2`
(source: <https://extensions.blender.org/add-ons/cell-fracture/>, read live
2026-10-23). Direct zip:
<https://extensions.blender.org/download/sha256:878bbd46e0c735da689101aeed9ebe99f18c0d8d0cbd2e9e17ed8cda6b264fb2/add-on-cell-fracture-v0.2.1.zip>.
Check the SHA above, then install once:

```powershell
& "C:\Program Files\Blender Foundation\Blender 5.2\blender.exe" `
  --command extension install-file -r user_default -e add-on-cell-fracture-v0.2.1.zip
```

The script enables `bl_ext.user_default.cell_fracture` per run, so
`--factory-startup` still works. No add-on bytes are committed or shipped.
The G6 premise holds only after this extension install; vanilla 5.2 has no
`bpy.ops.object.add_fracture_cell_objects` (checked: the add-on module is
`ModuleNotFoundError`, and `hasattr` on `bpy.ops.object` is misleading).

## Commands

```powershell
python docs/research/breakbench-blender-2026-10-23/sites.py `
  --out docs/research/breakbench-blender-2026-10-23/sites-24.json

& "C:\Program Files\Blender Foundation\Blender 5.2\blender.exe" `
  --background --factory-startup --python-exit-code 2 `
  --python docs/research/breakbench-blender-2026-10-23/fracture_column.py -- `
  --sites docs/research/breakbench-blender-2026-10-23/sites-24.json `
  --out docs/research/breakbench-blender-2026-10-23/run1.json `
  --seed 7 --expect-cells 24
# repeat into run2.json, then:
python docs/research/breakbench-blender-2026-10-23/verify.py `
  docs/research/breakbench-blender-2026-10-23/run1.json `
  docs/research/breakbench-blender-2026-10-23/run2.json
```

The add-on jitters every hull vertex by 1 mm using Python's global RNG. The
script calls `random.seed(--seed)` immediately before the operator. The
canonical bake quantises world vertices to 10 um, deduplicates them, maps
faces to those vertices, sorts faces and cell records, then SHA-256 hashes the
stream.

## Numbers

| run | cells | bake bytes | SHA-256 | operator ms | process ms |
|---|---:|---:|---|---:|---:|
| Blender run1 (seed 7) | 24 | 11,737 | `d79c07f15ab880c860110e708aa3bf41d17442fc1682ccc2bcfae6ac3ef1a8c3` | 71.255 | 228.745 |
| Blender run2 (seed 7) | 24 | 11,737 | `d79c07f15ab880c860110e708aa3bf41d17442fc1682ccc2bcfae6ac3ef1a8c3` | 51.927 | 176.516 |
| ablation seed 8 | 24 | 10,657 | `51fba00588690ba63c8b351d845118c32e914d656251e8af0c61baa69a3bbde0` | 76.783 | 221.853 |
| engine test | 24 chunks | equal x2 (asserted) | digest `e8370912b2c4e50a` | - | - |

`verify.py` prints `PASS cells=24 bake_bytes=11737 sha256=d79c07f1... operator_ms=71.255/51.927`.
Cell-count match: **24 = 24**. Bake bytes x2: **11,737 bytes and the same SHA
in both independent runs**; the engine test also asserts its own bytes equal
x2.

## Ablation (one seed change)

Changing only the Python jitter seed from 7 to 8 keeps **24** cells but changes
the bake: 10,657 bytes, SHA `51fba005...`, 379 vertices / 255 faces versus
421 / 275 at seed 7. The count is site-driven and robust; the baked shape is
seed-sensitive, so the seed must be pinned. Without seeding, the add-on's
1 mm jitter would make the column nondeterministic. This is the one
discriminating ablation.

## Metric and kill rule

Metric: cell count, canonical bake bytes and SHA-256, and operator ms for the
24-site wall.

Kill the column if any of:

1. the cell count is not 24 (or not the count BX14 produces once it lands);
2. two independent runs at the same seed differ in bake bytes or SHA-256;
3. a run needs a GUI or any step beyond `--background --python` plus the
   one-time extension install;
4. a 24-cell wall bake exceeds 2,000 ms operator time on this PC (measured
   52-89 ms).

If killed, BreakBench prints the Blender column unbuilt and the engine's
integer Voronoi bake remains the only fracture column.

## Verdict

ADOPT as a BreakBench reference column, with limits:

- Headless works, count matches, and the bake is byte-identical x2 when the
  add-on RNG is seeded.
- It is GPL-3.0-or-later, ideas only. No add-on code is vendored or shipped;
  the engine's `crates/destruction/src/fracture.rs` and `bake.rs` remain the
  authority.
- BreakBench wiring is not done here. The follow-up is a BX14 table
  (energy and material to cell count), then a BreakBench row that writes this
  column's count/bytes/SHA beside the engine column.
- The source card's rule (DeepFracture, arXiv 2310.13344v2): a fracture cell
  set is a function of the impact, so a future column must say which cell a
  hit lands in, not only how many cells break. This spike proves the tool and
  the count, not that rule.

## Files

`README.md`, `sites.py`, `sites-24.json`, `fracture_column.py`, `verify.py`,
`run1.json`, `run2.json`, `ablation-seed8.json`; one `SPIKE` line in
`docs/research/LADDER.md`. No board, plan, engine or big binary change.
