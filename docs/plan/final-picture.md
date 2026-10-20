# Final picture: what engine2040 is when it is done, and how it fixes itself

Owner 2026-10-20: "look final picture of this engine2040 ... what we steal ... to make it auto heal
auto fix easy af to make hard complex stuff ... make it compacter and code better af". This file is
the one-screen end state and the five layers that get there. Rows live in
`docs/SPRINT-27-final-picture.md` (AF auto-fix, CX compact). The physical half of the same request
(collision by default, crowds that touch, centre of mass, helmet-cam POV) is
`touch-and-pov.md` (rows TC, POV).

## 1. The final picture

One Rust engine. One game runs the same on a Windows PC, a potato laptop and a browser tab. It
looks like 2040 on an RTX 3050 and still runs on Intel UHD.

- **Worlds feel physical.** Everything collides unless it opts out. Cloth, ragdolls, limbs, debris
  and crowds touch each other. Bodies have weight: a push moves you, a shove can drop you.
- **Hard systems are one call.** A ragdoll, a destructible wall, a crowd, a game mode or a
  helmet-cam view is one recipe file or one door verb, with a receipt and a capture.
- **AI-native from the core.** Any model (mock, local, remote) drives the engine through one typed
  door. Every edit is checked, undoable and replayable without the model.
- **It fixes itself, and proves it.** At runtime a fault heals inside a budget, and the heal shows
  the state is good again (a digest), or it goes loud. In the dev loop a red test arrives with its
  own repro, its own localized files and the mechanical fixes already applied.
- **Small and sharp.** An agent works on one crate with a packet of a few thousand tokens. No dead
  modules, no unused features, one copy of each dependency where it can be.
- **Same hash everywhere.** The golden `d57b5c8ae3096614` holds on every PC and in replay; the
  float layers (cloth, physfx, lens) paint and never vote.

## 2. Where it stands (2026-10-20)

Deep libraries and a played arena duel (gates in `finish-line.md`). 42 library crates, about 772k
lines in `crates/*/src`; render, endpoint and game hold a third. About 700 open rows, about 300
ready. The heal plumbing exists (`crates/observe/src/heal.rs`, `supervisor.rs`) but no heal proves
its result and `RenderHeal` has no production caller. Content has fixers (`assets::import_repair`)
but no rule registry. Agents fix reds by hand.

## 3. The five layers

### L1 Runtime self-heal: a heal must prove itself

Already researched: `docs/research/powerups-2026-10-20/13-auto-heal-resilience.md` (12 ranked
steals and the 10 rules a heal must meet). Board rows that own it today: AD14 (heal tape carries
before and after digests), E75 (breaker and hedge on), plus the A9 chaos gate. Do these before any
new heal kind. Steals, best first: verified heal receipt (arXiv 2408.01055 idea), Envoy outlier
ejection (eject, count, keep the last good), resilience4j breaker window, Polly hedging delay,
Kubernetes liveness vs readiness split (presentation heal vs state heal). Rule: a heal that holds
the last good frame is a frozen frame and never counts as green.

### L2 Dev-loop auto-fix: the agent applies mechanical fixes without thinking

| Steal | Source | Licence | What we take | Row |
|---|---|---|---|---|
| rustfix / `cargo fix`, `cargo clippy --fix` | ships with the toolchain | MIT/Apache | apply machine-applicable suggestions, but only to the paths the agent owns | AF1 |
| ast-grep YAML rules with `fix:` | github.com/ast-grep/ast-grep | MIT | a rewrite per recurring bad idiom; our rules already live in `tools/rust-quality.mjs`, so first every FAIL line names its fix | AF2 |
| Agentless (localize, patch, validate) | github.com/OpenAutoCoder/Agentless, arXiv 2407.01489 | MIT | the repair protocol for a red test, as a skill, with our gh-check lines | AF3 |
| cargo-mutants | github.com/sourcefrog/cargo-mutants | MIT | a surviving mutant names a missing test; sim crates first | AF5 |
| cargo-shear `--fix` | github.com/Boshen/cargo-shear | MIT | delete unused deps in one run | AF6 |
| Dylint | github.com/trailofbits/dylint | MIT/Apache | engine lints (no `HashMap` iteration in the tick, no float in sim) as real compiler lints | later |
| nextest retries + flaky fail | nexte.st | MIT/Apache | `flaky-result = "fail"` so a flake is never green | OPT6 |

Not copied: an auto-commit repair loop. Patches stay proposals until tests and replay pass, and the
lead commits by path (same rule as `ai-native-steals.md`).

### L3 Content auto-validate and auto-fix

Unreal `EditorValidatorBase`, Godot `_get_configuration_warnings`, Bevy lint, glTF-Validator and
O3DE asset builders share one shape: rule id, severity, message. None of them ship a fix callback,
so that is where we go further: `Rule { id, severity, check, fix }`, with the fix as one undoable
door change. First rules: missing tangents (fix: `import_repair::recompute_tangents`, already wired),
NaN or zero-area triangles, non-unit normals, a mesh with no collider (fix: cook a hull, see TC2).
Row AF4.

### L4 Hard systems in one call

The shape to steal is Bevy's plugin (`app.add_plugins(X)`) plus Houdini's digital asset (typed
parameters in, geometry out) plus Unreal's ability set (one data asset grants a bundle). Here it is
a recipe: a typed TOML file plus one door verb that applies it, prints its receipt and returns a
capture. The first recipes are already gaps on the finish line: C10 (`fx.ragdoll`, `fx.limb`,
`fx.break`, `fx.blast` with `body.toml`) and C11 (`game.rules` per mode). Build those as recipes
first; add the next ones (crowd, helmet cam) only after both have a caller. Rerun's one-line
logging is the debug side (`observe/rerun_sink.rs` exists with no caller; CMP-07 decides).

### L5 Compact code: smaller crates, smaller packets

The spec is written: `docs/research/powerups-2026-10-20/09-code-compaction.md` and its proposed
rows CMP-01..CMP-17 (baseline file, dead modules, dead features, duplicate deps, profraw, per-crate
packet, stale docs, render split). The powerups lead boards those; do not copy them here.
Corrections from this pass:

- CMP-10 is done: the 91 `.profraw` files are untracked and ignored (`e435e5e9`).
- CMP-04 (`nav/orca.rs`) must not be deleted: TC4 wires it into the crowd. CMP-02
  (`physics/kcc.rs`) waits for LEG6, which ports its slope limit and ground snap into the integer
  controller; then the float file is deleted (decided 2026-10-21, section 5).
- Two new rows beside them: CX1 moves the cape layer out of `fighter_look.rs` (6k lines) into its
  own module file, the way the shoulder and camera parts already left; CX2 makes a file over 3,000
  lines unable to grow (a ratchet, not a rewrite).

## 4. Order

1. AF1 and AF3 (every agent session gets faster at once).
2. AD14, E75 (heal proof) and AF4 (content rules) beside them, different crates.
3. CX1, CX2, then the CMP rows in the lead's order.
4. Recipes after C10 and C11 land.

## 5. Decisions (owner 2026-10-21: "yes u do all decide on em")

- `cargo-mutants` and `cargo-shear` (both MIT) are installed into `.tools/bin` (gitignored); rows
  AF5 and AF6 use them.
- `orca.rs` is wired (TC4). `kcc.rs` is f32 and cannot drive the hashed tick: its slope limit and
  ground snap move into the integer controller (LEG6), then the float file goes. CMP-02 deletes it
  only after that port.
- The Roman game is now a legionary simulator: `rome.md`.

## Sources

Read 2026-10-20 by three research agents and the powerups lanes: rustfix, clippy usage page,
cargo-shear, ast-grep YAML reference, Dylint, cargo-mutants, Agentless, nextest retries, bevy_cli,
Unreal EditorValidatorBase, Godot Node warnings, glTF-Validator, meshoptimizer, O3DE asset
builders, Bevy plugins, Avian, Houdini HDA API, Unreal GameplayAbilitySet and PCG, Unity bakers,
Rerun. Licences as listed; Epic, Unity and SideFX pages are ideas only.
