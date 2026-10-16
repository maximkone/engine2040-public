# Cross-crate harness wiring — `node tools/proof.mjs` wires verifier gates by name

Owner crate: `engine-proof` (`crates/proof`). Consumers: all crate
test suites that participate in the proof gate. Units: gate names
as strings. Determinism: the harness runs gates in a fixed order;
same seeds give same results. Errors: a removed or renamed gate
fails the build on the proof side instead of silently unwiring.

## Pattern

`node tools/proof.mjs` is the cross-crate harness that runs every
verifier gate by name. It wires three named gates plus the seed/tape
proofs:

- `engine-endpoint --test it redteam::` (RV1 red-team suite)
- `engine-playtest tournament::tests::proof_gate_reproduces`
  (PQ2 balance tournament)
- `engine-playtest keypoint` (VK1 keypoint suite)

The harness wires these by name in `tools/proof.mjs`. If a gate is
removed or renamed, the build fails on the proof side — never a
silent unwire. This is the cross-crate contract that guarantees every
quality row is actually tested.

The harness also runs `cargo doc --no-deps` (docs coverage),
`cargo fmt`, release build, default and all-features tests,
`cargo deny`, and `node tools/proof.mjs --perf` for release timing.

## Why it is reusable

Every new verifier gate must be wired into `tools/proof.mjs` by
name. The pattern is: add the gate name to the harness, the harness
runs it every wave, and if it is removed the build fails. This
prevents the common bug where a test exists but is never run.

**Transferable to:** any new test suite or verifier gate. The recipe:
(1) write the test in the crate, (2) add its name to
`node tools/proof.mjs`, (3) verify `node tools/proof.mjs` picks it up.

## Test

The harness itself is tested by running it:

```sh
node tools/proof.mjs              # all local gates
node tools/proof.mjs --skip-build # proof fixtures only (CI compiles itself)
node tools/proof.mjs --perf       # release timing tests, one thread
```

The proof harness wires the three verifier gates by name
(`engine-endpoint --test it redteam::`,
`engine-playtest tournament::tests::proof_gate_reproduces`,
`engine-playtest keypoint`), so a removed or renamed gate fails
the build on the proof side instead of silently unwiring.

## Reuse counter

This entry counts **5** reuses. It becomes an asset at **2**
reuses and archives at **0** reuses across 10 waves.

| Wave | Packet | How it reuses |
|---|---|---|
| 28 | all proof rows | Every wave runs `node tools/proof.mjs`; the harness wires all verifier gates by name |
| 29 | all proof rows | Same harness; new gates (SL18–SL20 alloc probes) are wired by name |
| 30 | all proof rows | Same harness; SL21 window-alloc audit waits for wiring |
| 31 | ED8 | The crash-safe journal test is wired into the proof suite by name |
| 32 | SP12 | The spatial-gold eval gate is wired into the proof suite by name |
| future | any new verifier | Add the gate name to `node tools/proof.mjs`; the harness runs it every wave |

## Reference

Full signatures live in rustdoc (`cargo doc -p engine-proof --no-deps`);
the file that owns this page: `tools/proof.mjs`.
See also: `docs/book/verifiers.md` — the verifier gate documentation
(`vk1`, `pq2`, `rv1`, `node tools/proof.mjs`).
