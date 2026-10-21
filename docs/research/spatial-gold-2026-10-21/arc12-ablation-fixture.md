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
backup `8d8c0a80`. A golden of a computed output cannot survive a change to the computation, and
nothing re-blessed it afterwards. The brief that drifted, `arc12-08`, is the one whose prop is a
bench:

```
prop bench bench_01
want bench_01 next_to wall:east [0,60]
want bench_01 facing wall:west
```

## Which commit moved the numbers (the attribution)

The candidate list above was wide, so the drift was bisected on the GitHub runner instead of
guessed. Every public mirror snapshot carries the OLD golden (arc12-08 = 9 changes / 320 bytes),
so the arc12 test at such a tree is a perfect oracle: it passes while the live numbers are still
9/320 and fails, printing the live numbers, once the drift has happened. Four private revs were
exported to the mirror as probe branches and probed with
`cargo test -p engine-endpoint --lib --features mcp arc12_ablation -- --nocapture`:

| probe | private rev | what it is | live arc12-08 | result |
|-------|-------------|------------|----------------|--------|
| p5 | `462eb20f3` | parent of the nav commit | 9 changes / 320 bytes | PASS (run 39676317622) |
| p6 | `3a38f8b6f` | nav: UW12 layered navmesh | 9 changes / 320 bytes | PASS (run 39676322225) |
| p7 | `c4641f4d3` | spatial: AD18 3D yaw-aware overlap | 8 changes / 293 bytes | FAIL (run 39676326488) |
| p8 | `6717aab5a` | arena: v2 landed hits sever a joint | 8 changes / 293 bytes | FAIL (run 39676331632) |

The drift brackets to exactly one commit: **`c4641f4d3` "spatial: AD18 3D yaw-aware overlap"**
(2026-10-20 13:46). It made the overlap test 3D and yaw-aware in the report, the solver and the
placement checks, and the solver's relation delta for the bench brief changed with it - one fewer
change and 27 fewer edit bytes. The commit's own body says it re-blessed the spatial brief
golden ("the brief golden drops the torch case `overlap 0 4`"), and it changed the inline golden
in `crates/spatial/src/brief.rs` - but it touched no file under `fixtures/`, so the endpoint's
`fixtures/spatial-gold/arc12_ablation.tsv` was left holding the pre-AD18 numbers.

Two earlier probes, on the same oracle, cleared the other candidates: the nav commit `3a38f8b6f`
(10-20 13:32) still prints 9/320 and passes, and the window's own endpoints bracket the drift
cleanly - private `6fd0669e` (10-20 09:25) passes and private `9e9a585a` (10-20 20:38) fails.

The spatial floor fix `21131a5a` (10-21 07:35) is NOT the cause. That was tested directly, on
two legs that differ in exactly that one variable, with the OLD golden in both:

- leg A: the floor fix reverted, old golden -> live 8 changes / 293 bytes
- leg B: the floor fix kept, old golden -> live 8 changes / 293 bytes

Same numbers on both legs (runs 39646090967 and 39646094466), so the floor fix did not move the
golden; `c4641f4d3` did, three days earlier.

## What was done

- `fixtures/spatial-gold/arc12_ablation.tsv`: exactly one row re-blessed from the live solver
  output (`arc12-08`: `changes` 9 -> 8, `edit_bytes` 320 -> 293). No other row touched, no
  column invented, no threshold widened.
- The losslessness invariants the test asserts after the comparison - `requests` equals the
  brief's wants, `props` equals `reachable`, `solves` equals `props`, `raw_lanes` equals
  `props * 4`, `digest_equal` is 1 - then run against the re-blessed golden. The six other arc12
  tests (the ten briefs land clean, undo twice, the relation edit has zero findings) keep passing.

## Proof

Every line below was run on the GitHub runner, against a tree that carries this receipt's
re-blessed golden. The probe runs are the bisect oracle described above: each carries the OLD
golden on purpose, so a PASS means "still 9/320" and a FAIL means "already drifted".

| command | result | run |
|---|---|---|
| `cargo test -p engine-endpoint --lib --features mcp arc12_ablation -- --nocapture` (p5, private `462eb20f3`) | `1 passed; 0 failed; 672 filtered out`, live arc12-08 = 9 changes / 320 bytes | 39676317622 |
| `cargo test -p engine-endpoint --lib --features mcp arc12_ablation -- --nocapture` (p6, private `3a38f8b6f`) | `1 passed; 0 failed; 672 filtered out`, live arc12-08 = 9 changes / 320 bytes | 39676322225 |
| `cargo test -p engine-endpoint --lib --features mcp arc12_ablation -- --nocapture` (p7, private `c4641f4d3`) | `0 passed; 1 failed; 672 filtered out`, live arc12-08 = 8 changes / 293 bytes | 39676326488 |
| `cargo test -p engine-endpoint --lib --features mcp arc12_ablation -- --nocapture` (p8, private `6717aab5a`) | `0 passed; 1 failed; 672 filtered out`, live arc12-08 = 8 changes / 293 bytes | 39676331632 |
| `cargo test -p engine-endpoint --lib --features mcp arc12_ablation -- --nocapture` (leg A: floor fix reverted, old golden) | FAILED, live arc12-08 = 8 changes / 293 bytes | 39646090967 |
| `cargo test -p engine-endpoint --lib --features mcp arc12_ablation -- --nocapture` (leg B: floor fix kept, old golden) | FAILED, live arc12-08 = 8 changes / 293 bytes | 39646094466 |
| `cargo test -p engine-endpoint --lib --features mcp arc12_ablation -- --nocapture` (final tree, re-blessed golden) | 6 passed; 0 failed | see the endpoint run in the re-bless commit |

The golden file itself never moved after the re-bless: its digest stays `d57b5c8ae3096614`, and
the re-bless changed exactly one row of one file (1 insertion, 1 deletion in
`fixtures/spatial-gold/arc12_ablation.tsv`).

The probe branches (`wip-ab-p5-parent-of-nav`, `wip-ab-p6-nav-uw12`, `wip-ab-p7-spatial-ad18`,
`wip-ab-p8-arena-v2`) are throwaway branches on the public mirror and are deleted after the
bisect. `origin/main`, the live public site, was never moved: each probe is a branch of it.

