# HASH-LAW — what determinism proves

Legacy fixture: seed 2040, 600 ticks, `GOLDEN_600=d57b5c8ae3096614`.
Keep it frozen unless an intentional simulation change has an explicit receipt.

## Contract

Same initial state + engine/rules version + cooked assets + configuration +
ordered input/action log + seed streams must produce the same authoritative
state checkpoints within a declared platform/numerical determinism tier.
“Same seed” is shorthand for that complete contract, not a promise about every
model response, GPU image or floating-point program on every machine.

The proof fixture (`engine-proof`, the CLI default) hashes the legacy 2D
ECS/physics path only. The legacy demo example (removed; formerly examples/legacy_demo.rs)
touches other crates after that
fixture; most workspace crates are not included in `GOLDEN_600`. Tests in those
crates are useful, but cannot extend the golden's coverage by name.

Legacy seeding forces the low bit (`Rng::new(seed | 1)`), so seeds `2k` and
`2k + 1` simulate the same trajectory; receipts still differ because the raw
seed is folded into both hashes. Receipt inequality therefore cannot show that
two seeds simulated differently; `engine-proof` tests assert trajectories
directly. New fixtures must expand seeds without aliasing (for example
SplitMix64) under a new fixture version; v1 stays frozen.

## Hash scope

IN: canonical entity/component state that affects future ticks, RNG state,
timers, queued authoritative events, planner/blackboard state, relevant script
state, accepted external actions and asset/rules version references.

OUT: GPU handles/pixels/timestamps, render interpolation, UI selection/layout,
wall-clock durations, model credentials, transport timing and telemetry.
Hash editor documents and cooked artifacts separately where useful.

Canonical serialization defines field order, stable IDs, numeric encoding and
endianness. No native memory dumps, `HashMap` iteration order, pointer addresses
or Rust's unspecified hashing as a persistence contract. Non-finite values,
signed zero, overflow and rounding need explicit rules for included numerics.
FNV is a regression checksum, not a cryptographic integrity/authenticity proof.

## Evidence tiers

1. **Local reproducibility:** same binary/config/input, two independent runs.
2. **Renderer independence:** null and real GPU presentation consume the same
   simulation fixture and produce the same authoritative checkpoints. Rendering
   frames alone or invoking null twice does not prove this.
3. **Platform reproducibility:** recorded fixture across named OS/CPU/compiler
   configurations. Ordinary `f32`, fixed timesteps and quantization alone do not
   guarantee this. Prove selected math/physics modes or narrow the claim.
4. **Rollback/replay:** serialize full future-affecting state, restore and replay
   inputs/actions, compare every checkpoint and report the first divergence.

## Tier required by the game (decided 2026-09-23)

The multiplayer arena needs **tier 3 for the arena simulation**: one release
binary, x86-64 Windows on every player's PC, with Linux CI as the cross-check.
Replays and bug tapes must reproduce on any of those machines. Rules for the
authoritative simulation (`arena`, `physics`, `combat`, `ai` gameplay, `anim`
root motion, `net` state):

- Only `+ - * /`, `sqrt`, comparisons and integer/`Fixed` math. No `sin`,
  `cos`, `exp`, `powf` and friends from the platform runtime: they may differ
  between CPUs and C runtimes. Use `Fixed` or deterministic implementations.
  `tools/workspace-check.mjs` reports violations; task D1 makes it fail.
- Rapier runs with `enhanced-determinism` and without SIMD or parallel features.
- No fast-math, no thread-count-dependent reductions, no `HashMap` iteration
  in hashed state, fixed 60 Hz tick.
- Netcode is server-authoritative with client prediction and rollback. The
  server's checkpoints are truth; a client mismatch is corrected by
  resimulation from the server snapshot and counted, never ignored.
- The arena fixture (v2) uses its own fixture version and a non-aliasing
  seed expansion. The legacy v1 golden stays frozen.
- Arena fight rules v2 (EG3, `ArenaRules`, `FIGHT_RULES_V2`): an impact
  needs the sweep capsule test over integer-mm poses and fixed-point facing
  (`Fixed` LUT, no platform trig), teammates are never struck, block and
  parry need the shield cone to face the attacker, simultaneous impacts
  resolve from the pre-resolution state, and brain wishes actuate in the foe
  frame. A v2 checkpoint is the v1 hash bytes continued (FNV) with the rules
  version and the active team ids, so v1 and v2 never compare equal. V1 stays
  the default for every runner, tape, snapshot and `game::App`; v1 arena
  checkpoints and `GOLDEN_600` are unchanged (the golden fixture never runs
  arena or combat).
- Arena collision (EG20, `ArenaRules::collision`, on in rules v2, default
  leg only): fighter capsules push apart through `controller2`, centres are
  pushed out of rule pillars and a round wall holds them, all in integer mm;
  only the integer displacement is added back onto the float pose. The fold
  appends the collision tag byte `0xC0` plus every pillar footprint after
  the rules bytes, so collision and no-collision runs never compare equal.
  The `sim` leg skips the pass and the tag. V1 never collides.
- Ragdoll split (EG34, FOCUS decision 2026-10-10): stagger, knockdown, limb
  and cut state are integer and hashed; the Rapier ragdoll is client-local,
  seeded from the event, blended by powered drive, never hashed. Balance
  wire (`ArenaRules::balance`, on in rules v2): each landed blow feeds the
  victim's integer `BalanceState` (hit stumbles, a hit inside an open stagger
  window or a knockdown falls, a block chips, a parry staggers the attacker),
  a fallen or catching fighter's root holds, and the fold appends tag byte
  `0xB1` plus every slot's `BalanceState::state_hash` (centre of mass as
  integer mm). Proof that the ragdoll stays out of the hash:
  `cargo test -p engine-physics --features rapier --lib balance_hash`.
  Stagger memory (EG46, 350b3b25): each landed blow also feeds the victim's
  `StaggerMemory` (`crates/physics/src/stagger_memory.rs`) in the same v2 leg,
  and the fold then appends tag byte `0xB2` plus every slot's
  `StaggerMemory::state_hash`, so repeated stagger reaches the authority.
  Proof: `cargo test -p engine-arena --lib stagger_hash` (three staggers in the
  window floor the victim; digest equal x2 and unlike a two-stagger run).
  `LimbPool` and cut state join the same fold in later rows.
- EG43 (655cce21): rules v2 gives each slot a temperament seeded from the
  match seed, so v2 digests now include the per-slot temperaments and moved
  with that commit; v1 checkpoints and `GOLDEN_600=d57b5c8ae3096614` are
  unchanged.
- EG44 (test + note only): `App::new` stays rules v1 by design
  (`GameConfig::arena_config` pins `TABLES_VERSION_V1`); the window build
  plays v2 through `into_duel` (`windowed.rs::run_windowed`,
  `MatchSpec::duel`), so `GOLDEN_600` and the v1 arena checkpoints are
  unchanged. Proof:
  `cargo test -p engine-game --lib app_new_equals_new_for_slot_on_rules_v2`.
- DSN1/DSN2 (2026-10-17): a match is one `MatchSpec` (config, rules, world;
  `crates/arena/src/spec.rs`) that the window (`App::spec`), `Server::from_spec`
  and `Client::from_spec` all build from; `MatchSpec::v1` is `LiveMatch::new`
  byte for byte. The arena match is pinned across machines: `arena_line`
  (`cargo run -p engine-proof --features arena --bin arena_line`) plays 1200
  ticks of 8 fighters under rules v2 on the kit walls and prints a digest and
  every tick hash. Golden-matrix run 39054403838: `arena=d29b3b17bbd3b244` on
  linux-x86_64, windows-x86_64, linux-aarch64, wasm32-wasip1 and
  wasm32-wasip1-simd128, all MATCH. It is a new fixture (own digest); the
  legacy golden is unchanged. A limb cut is not in the tape yet: the sim step
  has no caller of `spawn_limbs`.

## AI and asynchronous services

Model outputs are nondeterministic external proposals. Authority validates them,
chooses an application tick and records accepted actions or a fallback decision.
Replay uses that record, never a new provider call. Identical prompts/seeds do
not establish model reproducibility. Job completion order cannot decide outcomes.

## Golden changes and failures

- Add new versioned fixtures for new integrations rather than pretending the
  legacy golden covers them. Include negative tests that alter covered state.
- An intentional bump records fixture/version, reason, old/new hashes and both
  run outputs; if committing, include that receipt with the scoped commit.
- On failure, capture command, seed, build/config, first divergent tick and lane
  state. Diagnose CPU/sim vs extraction vs GPU; do not blame a driver by default.
- Quarantine in `BLOCKERS.md` with owner and reproduction. A missing adapter is
  unsupported coverage, not a green parity result. Pixels have their own
  toleranced regression suite in `VISUAL-TARGET.md`.
- PX30 com_mm: state_hash folds integer mm; golden d57b5c8ae3096614 verified unchanged x2
  via cargo test -p engine-proof --lib golden_hash_untouched; no re-pin needed
  because arena/game never calls BalanceState.
