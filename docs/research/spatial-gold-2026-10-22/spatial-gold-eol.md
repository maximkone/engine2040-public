# The spatial-gold proof gate, fixed (2026-10-22)

## What was broken

The proof gate named "spatial-gold generator matches tasks.tsv" failed. Not
sometimes, not on one box: it failed on a fresh clone of `origin/master`, which
is what the CI runner does.

```
[FAIL] spatial-gold generator matches tasks.tsv - first drift line 1: gen="# spatial-gold v1" disk="# spatial-gold v1\r"
```

## Why

`fixtures/spatial-gold/generate.mjs` writes `tasks.tsv` with LF endings
(`out.join("\n") + "\n"`, generate.mjs:184) and then compares the file
byte-for-byte against what it just made (generate.mjs:199, 210). The four
`fixtures/spatial-gold/**` TSVs were committed with CRLF, so the file on disk
and the generator's own output differed by one byte per line.

There were two ways to be wrong, and both were live at once:

1. The **blob** carried CRLF. On a Linux runner the blob is read raw, so the
   comparison failed there.
2. The **checkout** converted the line endings back. `core.autocrlf=true` on
   this Windows box turned the LF blob back into CRLF on disk, so it also
   failed here, even after the blob was fixed.

The Rust reader was never wrong: `crates/playtest/src/spatial_eval/tsv.rs`
trims the CR on every line it reads (lines 568, 576, 583). Only the byte-level
generator check cared, and it cared correctly - a fixture that is regenerated
must match its generator exactly.

## The fix

Two halves, and both are needed.

1. `.gitattributes`: `fixtures/spatial-gold/** -text`. The repo already pins its
   byte-exact fixtures this way (`fixtures/khronos/*.gltf`, `fixtures/gltf/*.gltf`),
   and the spatial-gold fixtures are the same kind of thing: read byte-for-byte
   and compared, never reformatted.
2. The blobs themselves, renormalized to LF. `tasks.tsv` was written by the
   generator's own writer; the other three had their CR stripped by hand.

The content does not change. Every one of the four files is identical after the
CR is stripped.

## The rule had to be recursive

The first attempt was `fixtures/spatial-gold/*.tsv`, which matched the three
fixtures at the top of that directory and not the one in `llm/`. A fresh clone
showed the gap plainly: the three root fixtures landed as LF and
`fixtures/spatial-gold/llm/minimax-m3.1-flash-free.tsv` landed as CRLF, 104 of
them. The rule is now `fixtures/spatial-gold/** -text`, and
`git check-attr text` reports `text: unset` for the nested path too.

## The golden did not move

The same fresh-clone run re-proved the golden digest, unchanged:
`[PASS] legacy golden - d57b5c8ae3096614 engine=0.1.0 workspace=0.1.0`, after a
267.9 s golden build.

## What the fresh clone shows

A fresh clone of `origin/master` at `c235c63`, on this Windows box, with
`core.autocrlf=true`:

| file | CR bytes |
|------|----------|
| `fixtures/spatial-gold/tasks.tsv` | 0 |
| `fixtures/spatial-gold/arc12_briefs.tsv` | 0 |
| `fixtures/spatial-gold/arc12_ablation.tsv` | 0 |
| `fixtures/spatial-gold/llm/minimax-m3.1-flash-free.tsv` | 0 |

`node fixtures/spatial-gold/generate.mjs --check` ->
`[PASS] tasks.tsv matches generator (100 rows, 16 fix bases, capture_hex kept)`,
exit 0.

The full proof gate in that same fresh clone: the spatial-gold check now
PASSES. Two other checks in that run are red, and neither is this lead's:
`crates/game/src/bin/arena.rs` (5101 -> 5138) and
`crates/render/src/present/gpu_present/presenter.rs` (3489 -> 3553) both trip
the rust-quality ratchet ("a file over 3000 lines cannot grow"). They come from
the VS7 night-rig lane (commits 89f94e710 and 016032cde), and the fix is that
lane's: move a section of each file into its own module. The ratchet is not
accepted to make it green.

## Proof

Every command below ran on this PC.

- `node fixtures/spatial-gold/generate.mjs --check` (fresh clone of
  `origin/master` at `c235c63`) ->
  `[PASS] tasks.tsv matches generator (100 rows, 16 fix bases, capture_hex kept)`,
  exit 0.
- Before the fix, the same command ->
  `[FAIL] tasks.tsv drifted from generator; run with --write`, first drift
  line 1: `gen="# spatial-gold v1" disk="# spatial-gold v1\r"`.
- `node tools/proof.mjs --skip-build` (fresh clone at `c235c63`) ->
  `[PASS] legacy golden - d57b5c8ae3096614 engine=0.1.0 workspace=0.1.0`, and
  `[PASS] spatial-gold generator matches tasks.tsv - [PASS] tasks.tsv matches
  generator (100 rows, 16 fix bases, capture_hex kept)`.
- `git check-attr text -- fixtures/spatial-gold/llm/minimax-m3.1-flash-free.tsv`
  -> `fixtures/spatial-gold/llm/minimax-m3.1-flash-free.tsv: text: unset`.
- A fresh clone, all four fixtures: CR=0, CR=0, CR=0, CR=0.
- All four fixtures: identical after CR strip = True.
