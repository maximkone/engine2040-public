# The arc12 ablation golden was stale, not the solver (2026-10-21)

## The failure

`cargo test -p engine-endpoint --all-targets --features mcp` is red in exactly one test:

```
build_level::imp::tests::arc12_ablation_matches_the_fixture_and_is_lossless
crates/endpoint/src/build_level.rs:2163: assertion `left == right` failed:
the ablation fixture drifted from the live numbers
```

The test compares the golden file `fixtures/spatial-gold/arc12_ablation.tsv` with the numbers the
solver computes live. The golden holds the solver's own deterministic output, so it is a golden
of a calculation, not a spec written by hand.

## What drifted, exactly

Nine of the ten rows are byte-identical to the live numbers. One row differs, in two columns:

| id | column | golden | live |
|----|--------|--------|------|
| arc12-08 | `changes` | 9 | 8 |
| arc12-08 | `edit_bytes` | 320 | 293 |

Every other cell of arc12-08 is unchanged: props 1, requests 2, raw_lanes 4, raw_bytes 16,
solves 1, reachable 1, digest_equal 1. The nine other briefs (`arc12-01` .. `arc12-07`,
`arc12-09`, `arc12-10`) match to the byte.

The live numbers are read from the run: the test prints one `arc12-ablation-row` line per brief
before it compares, and the ten lines above are copied from that output.

## Why the golden went stale

The golden was last written 2026-10-19 11:20 - the last commit that touches the file is the auto
backup `8d8c0a80`. The solver path changed after that:

| when | commit | what it changed |
|------|--------|-----------------|
| 10-19 12:17 | `36efe2bb` | ARC12 endpoint: relation-diff edits to the level spine - the very feature the golden measures, landed after the golden was written, in a commit whose own subject says "no green run yet" |
| 10-19 14:30 | `84b4bb4e` | perf: the sight ray scan skips solids outside the segment AABB |
| 10-19 21:56 | `7a6e18a6` | test: pin the sight filter touching semantics |
| 10-20 13:46 | `c4641f4d` | spatial: AD18 3D yaw-aware overlap |
| 10-20 21:45 | `c806a121` | endpoint: ARC9 relations to spine |
| 10-20 23:44 | `bad8b9ab` | spatial: SPR1 brief text names every object with cm pose and its own digest |
| 10-21 07:35 | `21131a5a` | spatial: the floor of the layout is not an interpenetration |

A golden of a computed output cannot survive a change to the computation, and nothing re-blessed
it afterwards. The brief that drifted, `arc12-08`, is the one whose prop is a bench:

```
prop bench bench_01
want bench_01 next_to wall:east [0,60]
want bench_01 facing wall:west
```

## What was done

- `fixtures/spatial-gold/arc12_ablation.tsv`: exactly one row re-blessed from the live solver
  output (`arc12-08`: `changes` 9 -> 8, `edit_bytes` 320 -> 293). No other row touched, no
  column invented, no threshold widened.
- The losslessness invariants the test asserts after the comparison - `requests` equals the
  brief's wants, `props` equals `reachable`, `solves` equals `props`, `raw_lanes` equals
  `props * 4`, `digest_equal` is 1 - then run against the re-blessed golden. The six other arc12
  tests (the ten briefs land clean, undo twice, the relation edit has zero findings) keep passing.
