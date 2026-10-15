# Quick start: from zero to a running arena

This page takes a fresh checkout to a running arena with nothing but a
terminal. Every command below runs headless (no display, no GPU) and was
run literally to produce the outputs quoted. It finishes with a scripted
4-fighter arena match plus PNG captures and a JSON receipt.

The manual itself is board row API2: this quick start plus the AI-door and
verifier pages exist today; one page per system and the MCP recipes are
still to come. Full signatures live in rustdoc, never copied here — run
`cargo doc -p <crate> --no-deps` for the crate you are driving.

## Prereqs

- Windows 10, git, and platform build tools.
- Rust 1.97.1 (`rust-toolchain.toml` pins it; rustup installs it on first
  use — check with `cargo --version`).
- Node 24 for the gate scripts (`node --version`).
- Optional: `cargo-deny@0.18.7` for the license gate. No other installs.

## 1. Prove the toolchain: the proof CLI help

From the repository root:

```powershell
cargo run --locked -- --help
```

Expected output (the deterministic proof CLI: seed, fixed 60 Hz ticks,
repeatable inputs, record/replay tapes, verify, text/JSON receipts, and
the `null` backend — anything else fails explicitly):

```text
engine2040 deterministic proof
  --seed <u64>                Initialization seed (default 2040)
  --ticks <1..100000>          Fixed 60 Hz steps (default 600)
  --input <tick:body:vx:vy>    Repeatable velocity assignment, before zero-based tick
                             Inputs must be unique and ordered by (tick, body)
  --record <path>             Create a new versioned input/checkpoint tape
  --replay <path>             Replay saved inputs and compare every checkpoint
  --verify                    Repeat this run from fresh initial state
  --format <text|json>        Receipt format (default text)
  --backend <null>            Supported proof backend; others fail explicitly
  --help                     Show this help
Replay owns its seed/ticks/inputs and cannot be combined with record or overrides.
This fixture covers 2D ECS/physics, not GPU images or a complete game snapshot.
```

That last line matters: this fixture pins the 2D ECS/physics determinism
golden. Pixels and the full game snapshot have their own gates (see the
[verifiers page](verifiers.md)).

## 2. Run the headless proof golden

```powershell
cargo run --locked -- --verify
```

Expected output:

```text
engine2040 headless proof:
  ticks=600 final_hash=d57b5c8ae3096614
  seed=2040 inputs=0 state_hash=2aca9338670aa0cb backend=null mode=run verified=true
```

`final_hash=d57b5c8ae3096614` is the legacy golden (`CONTRIBUTING.md`,
`tools/proof.mjs`): the default 600-tick, seed-2040 run must print it, and
`--verify` replays it from fresh state so `verified=true` means the hash
survived a from-scratch repeat. Same complete input always gives the same
hash; corrupted tapes fail naming the first divergent tick.

## 3. Run the headless example

```powershell
cargo run --locked --example headless
```

Expected output:

```text
example final_hash=e8a4e9c86ae12991 entities=10
```

This spawns 10 entities from seed 2040, integrates 120 ticks at 1/60 s,
and prints the hash (`examples/headless.rs`). If this works, the
simulation crates build and tick on your machine.

## 4. Run the arena: a scripted 4-fighter match

The headless staged fight runs the real arena match (`engine-game`
`App`: locomotion, combat, brains) with a scripted human input
(circle-strafe, attack every 5 ticks), binds 4 fighter variants from
`content/fighters`, and writes PNG captures plus a receipt:

```powershell
cargo run --locked -p engine-game --features app --bin staged_fight -- --ticks 600 --out-dir target/quickstart-fight
```

Expected output (hashes attest the content bundles; the `done` line is
the receipt):

```text
staged_fight bind: variant=hero ...
staged_fight bind: variant=opp_male_light ...
staged_fight bind: variant=opp_female_medium ...
staged_fight bind: variant=opp_female_light ...
staged_fight done: ticks=600 cues=30 hits=27 knockdowns=3 final_hash=2935468d340b210d captures=4 manifest="target/quickstart-fight\captures\manifest.json" receipt="target/quickstart-fight\receipt.json"
```

Run it twice with different `--out-dir` values: the `final_hash` repeats
exactly (`2935468d340b210d` both times) — same manifest plus seed plus
script gives the same fight and the same PNGs. The receipt is built for
byte-identical repeats (fixed JSON field order, file names only, no
absolute paths).
Flags: `--ticks` must be `600` (default 600; anything else exits 2),
`--content-dir` defaults to `content/fighters` under the current
directory, `--out-dir` defaults to `target/ct15-staged-fight`. Unknown
flags exit 2 naming the culprit, never a panic.

Units: fixed ticks at 60 Hz, arena half-extent 20.0 meters, 4 fighters,
seed 2040. Errors exit 2 for bad flags/config and 1 for
bind/fight/capture failures, each naming its culprit on stderr.

## What this does not cover (yet)

- **Windowed play** needs a display and a GPU adapter
  (`cargo run --locked --release --example window --features window --
  10000 5` draws procedural sprites; the `arena` window binary takes
  `--frames N --exit` and falls back to a headless smoke only when no
  display exists). Window binaries fail loudly on init errors instead of
 ever printing a fake frame.
- **Full gates**: `node tools/proof.mjs` runs everything (audit, fmt,
  release build, default and all-features tests and clippy, the verifier
  suites, seed/tape proofs). Start with the targeted legs on the
  [verifiers page](verifiers.md) before running the whole harness.
- **Driving it from MCP**: the [AI door page](ai-door.md) documents the
  twenty-tool stdio door (`scene.read` → `tx.preview` → `tx.apply` →
  `tx.undo`, plus `play.step` live duels and the Insight search tools).
- **Reusable patterns**: `docs/library/index.md` in the repo indexes
  skills, patterns, and recipes with tests and reuse counters.

## Reference

Ownership and contracts: `src/cli.rs` (proof CLI), `examples/headless.rs`,
`crates/game/src/bin/staged_fight.rs` (CT15 headless fight),
`crates/game/src/capture.rs` (headless capture), `CONTRIBUTING.md` (gates),
`docs/HASH-LAW.md` (determinism scope). Rustdoc is the signature source:
`cargo doc -p engine-game --no-deps`.
