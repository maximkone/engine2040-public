# Verifiers: keypoints, balance gate, red-team

Verifiers turn gameplay claims into runnable gates. A render feature counts
when its pass submits on a real device; a gameplay claim counts when its
keypoint passes; a balance claim counts when the tournament gate passes.
Stubs, CPU references, and data paths are never evidence — each section below
names the command that reproduces its verdict, and `node tools/proof.mjs`
runs all of them every wave.

## VK1: the keypoint suite

A gameplay claim ("parry opens a riposte window", "ragdoll gets up in 2 s")
is a keypoint spec: inject state into a fork, run a bounded input script for
N ticks, assert on typed state, take one capture. Specs are data in
`fixtures/keypoints/` (30 files, `kp01-jab-lands.json` through
`kp30-riposte-kills.json`); the runner is `crates/playtest/src/keypoint.rs`.

One spec (`pub struct KeypointSpec`) carries its registry `id`, the human
`claim`, the match `seed`, `fighters`, and `half_extent_m` in meters, a setup
`prefix_ticks` prefix with `setup` inputs, a scripted `window_ticks` window
with `window` inputs, `freeze` slots held frozen via a fork proposal, and
typed `asserts` (`pub struct KeypointAsserts`: `hp`/`damage` in permille,
`stuck` ticks, `killed`, exact `damage_ticks`, and more — every field
optional, so each keypoint asserts only its claim).

Bounds (all enforced, all named in refusals): spec format `SPEC_VERSION: u16 = 1`,
`MAX_SPEC_BYTES: usize = 64 * 1024`, `MAX_SPECS: usize = 64`,
`MAX_PREFIX_TICKS: u32 = 240`, `MAX_WINDOW_TICKS: u32 = 120`,
claims to `MAX_CLAIM_CHARS: usize = 280` chars, one `CAPTURE_EDGE: u32 = 64`
raster per keypoint, whole-suite tick budget `SUITE_TICK_BUDGET: u64 = 20_000`.
Bad specs fail typed as `pub enum KeypointError`
(`DirUnreadable`, `TooManySpecs`, `BadSpec { file, reason }`,
`ArenaRefused`); gameplay mismatches are failure strings inside a passing
report, never panics.

Entry points:

```rust
pub fn run_suite() -> Result<SuiteReport, KeypointError>
pub fn run_suite_with_fault(fault: &BugSeed) -> Result<SuiteReport, KeypointError>
pub fn keypoint_ids() -> Result<Vec<String>, KeypointError>
pub fn spec_dir() -> std::path::PathBuf
pub fn load_specs() -> Result<Vec<KeypointSpec>, KeypointError>
```

`pub struct SuiteReport` holds per-keypoint `results` sorted by id, an FNV
`digest` over the ordered per-keypoint digests, and the `sim_ticks` behind
the report; `SuiteReport::pass` is true exactly when every result passes,
`SuiteReport::passed` counts them.

Five seeded gameplay bugs (`BUG_SEEDS`: wrong window, missing hit-stop,
stuck state, dead stamina regen, dropped kill confirm) each fail exactly
their paired keypoint — and the ablation without state injection must miss
at least 3 of the 5, else injection is not worth its cost. The full set runs
in under 60 seconds with identical reports across runs.

Run it:

```sh
cargo test --locked -p engine-playtest keypoint
```

## PQ2: the balance-tournament gate

`pub fn proof_gate() -> Result<GateOutcome, Failure>` in
`crates/playtest/src/tournament.rs` (re-exported at the crate root so the
proof harness resolves it by name) runs the standard P3 tournament —
`TournamentConfig::standard`: 200 AI-vs-AI fork matches per difficulty at
600 ticks — and checks it against the P3 bars. Registration name:

```rust
pub const GATE_NAME: &str = "pq2-balance-tournament";
```

`pub struct GateOutcome` pairs the full `TournamentReport` with its `failures`
from `check`; `GateOutcome::pass` is true exactly when every bar passes.
Same call twice gives the same report text (seeds fan out by
`SEED_GAP: u64 = 1_000_003` per difficulty, above `MAX_MATCHES`, so ranges
never overlap). Cost is the shape itself: 600 tournament matches plus 60
spotlight matches at 600 ticks, about 19 s in a debug build.

The P3 bars (permille = tenths of a percent):

| Bar | Constant | Bar |
|---|---|---|
| Win rate 40–60% | `WIN_MIN_PERMILLE: u16 = 400`, `WIN_MAX_PERMILLE: u16 = 600` | mirror-match bias, not imbalance, on `Difficulty::even` |
| No move above 35% of kills | `KILL_SHARE_MAX_PERMILLE: u16 = 350` | kills attributed to the killer's observed button byte |
| Zero stuck | `STUCK_WINDOW_TICKS: u32 = 180` | 3 s below fork stuck speed at 60 Hz, corpse holds excluded |
| Parry, riposte, clash reachable | `EXCHANGE_TICKS: u32 = 240` | scripted exchange-reachability probes |

Spotlight overpowered flags ride inside `report.spotlight` for the move-table
consumers and never fail the gate (the strike lever is expected to fire).
A seeded overpowered move is flagged; the report reproduces byte-identical
twice (`tournament::tests::proof_gate_reproduces_*`).

Run it:

```sh
cargo test --locked -p engine-playtest tournament::tests::proof_gate_reproduces
```

## RV1: the red-team suite

`crates/endpoint/tests/it/redteam.rs` replays `fixtures/redteam/` (twelve
tapes: `01-brief-ignore` through `12-transport-oversize-remote` — prompt
injection via briefs, asset names, scene text, and tool results;
out-of-scope tool calls; secret-exfiltration tries). Every case is refused
or contained with a logged reason; no secret appears in any log; a new case
is one fixture file.

Run it:

```sh
cargo test --locked -p engine-endpoint --test it redteam::
```

Red-team notes for door users live on the [AI door page](ai-door.md#redteam-notes-rv1).

## The proof harness

`node tools/proof.mjs` runs everything, in order: workspace audit, fmt
check, release build, default and all-features tests and lint, `cargo deny`
when installed, the named verifier gates above, then the seed/tape proofs
(legacy golden `d57b5c8ae3096614` twice, four distinct-seed receipts, a
saved-tape replay, a corrupted-checkpoint failure naming the first divergent
tick, and an unsupported-backend refusal):

```sh
node tools/proof.mjs              # all local gates
node tools/proof.mjs --skip-build # proof fixtures only (CI compiles itself)
node tools/proof.mjs --perf       # release timing tests, one thread
```

The harness wires the three verifier gates by name
(`engine-endpoint --test it redteam::`,
`engine-playtest tournament::tests::proof_gate_reproduces`,
`engine-playtest keypoint`), so a removed or renamed gate fails the build on
the proof side instead of silently unwiring. See the library entry
[Cross-crate harness wiring](https://github.com/maximkone/engine2040-public/blob/main/docs/library/cross-crate-harness-wiring.md)
for the pattern and reuse counter.

## Reference

Full signatures live in rustdoc
(`cargo doc -p engine-playtest --no-deps`); the files that own this page:

- `crates/playtest/src/keypoint.rs` — `KeypointSpec`, `KeypointAsserts`,
  `ScriptInput`, `KeypointResult`, `SuiteReport`, `BugSeed`, `BUG_SEEDS`,
  `run_suite`, `run_suite_with_fault`, `keypoint_ids`, `spec_dir`, `load_specs`
- `crates/playtest/src/tournament.rs` — `proof_gate`, `GateOutcome`,
  `GATE_NAME`, `TournamentConfig::standard`, `run`, `check`, `check_spotlight`
- `crates/playtest/src/lib.rs` — `pub use tournament::{proof_gate, GateOutcome, GATE_NAME};`
- `crates/endpoint/tests/it/redteam.rs` + `fixtures/redteam/` — the RV1 suite
- `fixtures/keypoints/` (`kp01`–`kp30`) — the VK1 spec corpus
- `tools/proof.mjs` — the wave gate runner
