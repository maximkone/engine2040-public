# SYSTEMS — deep specs for the hard parts

One spec per hard system, written so many agents can build in parallel from
the spec alone. A spec is the contract: agents implement against it, tests
prove it, and the lane board (`../SPRINT.md`) schedules it. Cross-cutting rules
for logging and self-healing live in `observability-heal.md`; every system
applies them.

> 2026-10-03 slim: 10 of these specs are superseded by `../plan/*.md` (each plan
> names what it replaces) and now live in `../archive/2026-10-03/systems-*.md`.
> `ai-door.md` and `observability-heal.md` stay: the board and
> `crates/proof/src/chaos.rs` still name them.

| Spec | System | Main lanes |
|---|---|---|
| `render.md` | wgpu renderer V1–V4, look, captures | render |
| `body-and-break.md` (physics base section) | Rapier adapter, character, active ragdoll, destruction | physics, destruction |
| `netcode.md` | Server-authoritative LAN play with rollback | net, arena |
| `animation.md` | Skinning, blending, root motion, IK, motion matching | anim, render |
| `combat-arena.md` | Fight model, arena match, forks | combat, arena |
| `gameplay-ai.md` | Gladiator brains, director, integer nets, distillation | brain, nn, distill |
| `scene-spatial.md` | Scene document, relations, brief, placement solver | spatial |
| `commands-editor.md` | Transactions, journal, editor, crash recovery, provenance | editor |
| `content.md` (cook and delivery section) | Import, cook, streaming, hot reload, format museum | assets, proof |
| `parked.md` | Mixer, spatial sound, reactive crowd | audio |
| `ai-door.md` | Providers, MCP, eval, playtest agents, skill packs | provider, mcp, playtest, skills |
| `observability-heal.md` | Smart logging, retroactive logs, auto-heal, chaos | observe, proof |

## Spec shape

Every spec has the same sections, in this order: what is hard; design (data,
algorithms, invariants, references); determinism and authority; auto-heal
(fault → detect → act → record); smart logging (event codes and spans);
acceptance and chaos tests; board tasks. A section may say "none" but may not
be missing.

## How a large AI workforce uses these specs

Scale comes from isolation and verification, not headcount. The earlier
1,024-agent benchmark citation is an unverified research pointer, not evidence
of engine2040 throughput or a reason to build a larger workforce now. Tiers:

1. **Writers:** up to 12 `rust-coder` workers per working tree, one lane
   each (`../SPRINT.md`). Past 12, run more orchestrators in separate git
   worktrees, each with its own target folder and sprint lock, merging through
   one queue that rebases, runs `node tools/proof.mjs` and fast-forwards
   master (task SC2). The integrator and the gate set the pace.
2. **Specifiers and testers:** bounded parallel read-only reviews first:
   deepen a spec, write property tests, fuzz tapes and invariants (FZ1), add
   chaos cases (HL2). Writing tests/specs requires explicit file ownership too;
   read-mostly is not collision-free if several agents edit the same board.
3. **Tournaments:** for one hard function with a fixed interface, K agents
   write competing implementations in throwaway worktrees; the gate plus a
   benchmark picks the winner (AlphaEvolve 2506.13131, ShinkaEvolve,
   CodeEvolve 2510.14150; simple baselines stay competitive, 2602.16805, so
   keep evaluators strong). Only the winner merges (task SC3).
4. **Reviewers and red team:** second-opinion diff review against the spec,
   and adversarial playtesters hunting exploits (RV1).

Rate limits, cost and review capacity cap every tier; measure throughput in
ticked board tasks, never in agents launched.

Current planning uses review → source/dependency challenge → edited-contract
review. The implementation AI then dispatches ready whole-crate packets from
SPRINT; it must not copy a proposed multi-crate packet literally. SC1–SC3 retain
their board scope but are not dependencies of the first editable scene, native
assistant or locomotion course. Root AGENTS overrides stale skill fanout/build
folder advice. No parallel writer may ignore another sprint's live lease.
