# CT74 PBR/HDRI intake commit-gate

Row: CT74 content. Tier A. Size S. Owns `content/arena/PBR-INTAKE.md`.
Intake bytes stay frozen: nothing here adds, moves or deletes a file.
Needs: CT8 (arena kit), CT46 (provenance matrix). Consumers: CT45, CT57.

## Gate rule

Each untracked file under `content/arena/` must carry a provenance row
(URL + SPDX + author + SHA, or the AI chain) in the CT46 matrix
(`content/PROVENANCE-AUDIT.md`). A file with no row is refused. The gate
names the file and the reason. This matches the Rust check in
`crates/assets/src/provenance.rs` (`check_record`).

## Untracked files on 2026-10-17

`git status --untracked-files=all content/arena` shows one untracked folder:
`content/arena/cooked/`. It holds 15 KTX2 maps and one receipt.

| file | gate result | reason |
| --- | --- | --- |
| `cooked/cook.json` | refused | no provenance row (receipt, not a shipped asset) |
| `cooked/{stone,sand,wood,iron,floor}_{base,normal,orm}.ktx2` (15 files) | refused | no provenance row |

The KTX2 maps are cooked outputs of five texture sets (`stone`, `sand`, `wood`,
`iron`, `floor`). Their source bytes are under `content/arena/sources/`. The
CT46 matrix has no row for those five sets, so the gate refuses all 15 maps.
The receipt `cook.json` lists the sizes and PSNR of each map. It is refused too.

Result: 16 of 16 untracked files refused. None passes. Each one names its
gap, which is the acceptance the row allows ("or refusal").

## Committed set cooks x2

Not run in this pass. Reasons:

- The cook tool `crates/assets/src/bin/arena_cook.rs` is untracked work of
  another lane. It is not part of the committed set.
- Running it writes into `content/arena/cooked/` (the untracked folder above)
  and reads `content/arena/sources/`, which this row must not touch.

So the x2 cook of the committed set is open. The row stays PARTIAL until a
lane commits the cook tool and adds rows for the five source sets.

## Proof

- Gate result: section "Untracked files" above (read from `git status`).
- Rust provenance tests: `cargo test --locked -p engine-assets provenance`
  on GitHub: PASS (run 39064454169). See the CT52 doc `content/PROVENANCE-ENFORCE.md`.
- `node tools/provenance-audit.mjs`: 58 records, 7 refusals, same as CT46.
