# OPT39: the dev build is not measurably faster (2026-10-22)

The raw material for the OPT39 measurement, kept so the next seat can check
the numbers instead of re-measuring them. The row is NOT ticked; the verdict
is in `docs/SPRINT-27-final-picture.md` and the receipt is in
`docs/SPRINT-19-receipts.md`.

## The arms

Six interleaved cold builds of `cargo build -p engine-game --timings`, arms
alternating (before, after, before, after, three rounds) so contention on the
shared box hits both arms the same way and no single slow sample can carry
the claim. Each build ran in its own `CARGO_TARGET_DIR`, on a copy of HEAD
laid out with `git archive HEAD`, so both arms build the same sources and
the shared tree was never written to.

The "after" arm is the row's own setting: `debug = false` under
`[profile.dev.package."*"]` in the copy's manifest.

## The wall

    before  341.70 s   195.93 s   242.65 s      mean 260.10 s
    after   314.92 s   188.78 s   323.44 s      mean 275.71 s

The before arm alone spans 196 s to 342 s, so the wall cannot carry a 10 %
claim on this box. (An earlier, non-interleaved pass showed -33.5 %; its four
wall times fell monotonically while the box was quieting down. That number
was wrong.)

## The CPU time, the measure that counts

Cargo's per-unit times in the `--timings` report are WALL times, not CPU
times - the same crate read 22.9 s and 67.7 s across arms - so the CPU time
the build burns was measured instead, with every process's CPU accumulated
over its life. This is the measure this repository already quotes for build
speed (-17 % user CPU for the `[profile.test] debug = 0` line in
`.cargo/config.toml`, 2026-10-11).

    before  627.4 s     621.9 s     819.0 s       mean 689.4 s
    after   690.6 s     869.0 s     712.3 s       mean 757.3 s

The after arm burns no less CPU than the before arm: +9.8 %, with a 30 %
spread inside each arm.

## The rlib sizes, the number that did not move

    dependency rlibs   32.71 MB (before)  ->  30.56 MB (after)   -6.6 %
    workspace rlibs   113.25 MB in all six arms, 0.00 % mean change

The `target/` total moves 0.2 % (944.1 MB -> 943.6 MB), because
`incremental/` is 701 MB of it and the setting does not touch that, so the
row's 15 % size bar is missed by a wide margin.

## The files here

- `timings-distilled.md` - cargo's own timing report for every build, one
  table per arm, with each crate's total, frontend and codegen time
  (`opt39i-` = the interleaved wall pass, `opt39cpu-` = the CPU pass,
  `opt39f-` = the first pass on the fixed master, `opt39-` = the pre-fix
  pass whose tree did not compile, `opt39-landed` = a run that failed on a
  duplicate `pub mod torch_light;` and is kept only for the record).
- `arms.txt` - the per-arm wall and target-size receipt lines.
- `rlib-sizes.txt` - the rlib sizes.

The raw HTML reports are not kept: the table above is what they say, and it
is the readable one.

## The P2P legs, on a runner

- `cargo test -p engine-core --lib` -> PASS, 160 passed, 0 failed, 4 ignored
  (run 39701380630).
- `cargo test -p engine2040 --release --test proof_cli default_and_max_seed`
  -> PASS (run 39702418567), so the golden did not move.

## Why the workspace crates keep their line tables

`.cargo/config.toml` sets `[profile.dev] debug = "line-tables-only"`, and a
config file's profile wins over the manifest's, so the manifest's
`package."*"` block never reaches a workspace member. That is why every
workspace rlib measured byte-identical (23 crates, 0.00 % mean change) while
the dependency rlibs shrank.
