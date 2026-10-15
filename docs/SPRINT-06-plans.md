### Plans (planner packets, `docs/plan/README.md`)

Each row writes one plan file and proposes rows with its prefixes; the
orchestrator reviews them and moves accepted rows onto the board. A planner
reads the matching `docs/systems/` spec first and keeps what is still true;
after review the plan replaces that spec.

- [ ] PLN18 plan: general engine — audit every crate for arena or genre
  assumptions (names, fighter caps, units, camera, modes), propose moves behind
  `arena`/`game`, and fixture genres that prove generality. Owns
  `docs/plan/runtime.md`, prefix GE. needs: none.
  LATER: parked 2026-10-03 (circus verdict: planning backlog while 300+ rows are ready (RESTRUCTURE-2026-10-02)); unpark by deleting this line.
- [ ] PLN19 plan: living world and AI builds inside — NPC mind contract,
  memory streams, goals, dialogue, recording for replay; brief to level,
  character, encounter and playable slice through the command door. Owns
  `docs/plan/living-world.md`, prefixes LW and AB. needs: none.
  LATER: parked 2026-10-03 (circus verdict: planning backlog while 300+ rows are ready (RESTRUCTURE-2026-10-02)); unpark by deleting this line.
- [ ] PLN20 plan: AI-native architecture, spine first. The finding (audit
  2026-09-24, checked by git grep): the game does not run on the engine's own
  core. `engine-arena` does not depend on `engine-ecs`; the ECS `World` is a
  2D pos/vel/alive table with no consumer; `ecs/src/systems.rs` (5 phases)
  duplicates `schedule.rs` (4 phases) and has no users; the RC2 bridge
  (`ExtractionView`) has no consumer. A general engine (EMPIRE: any genre)
  needs one spine: a 3D world with real components (transform, body,
  fighter, AI, anim) that arena, net rollback, save, editor, VK1, VP1 and the
  AB brief-to-game rows all run on. Plan the migration so the old arena path
  and the new one run side by side until their 600-tick hashes match; then
  the old path retires and the golden moves with a migration note in
  HASH-LAW. Then the rest of the arXiv sweep in `docs/RESEARCH-VERDICT.md`:
  the queryable engine (API1), rules proven by verifiers (VK1, Code World
  Model 2608.25927), scene edits as relations (3D-Layout-R1 2603.22279),
   self-improving harness (Darwin Gödel Machine 2505.22954). Output: ARC rows
   in dependency order, the first ones S-size. Owns `docs/plan/runtime.md`,
   prefix ARC. needs: none.
  LATER: parked 2026-10-03 (circus verdict: planning backlog while 300+ rows are ready (RESTRUCTURE-2026-10-02)); unpark by deleting this line.
- [ ] PLN21 plan: Finish B breakdown — split `FINISH-RICH.md` Finish B
  (second game through public APIs, dedicated-server transport under loss,
  stable versions and migrations, templates and onboarding, soak with no
  unbounded growth, hardware matrix, stranger-dev workflow, AI-native
  pillars in the shipped sample) into Tier B board rows in dependency
  order, the first ones S-size. Owns `docs/plan/quality.md`, prefix FB.
  The second game is urban war (owner 2026-09-26): UW39 is its
  second-game receipt. needs: none.
  LATER: parked 2026-10-03 (circus verdict: planning backlog while 300+ rows are ready (RESTRUCTURE-2026-10-02)); unpark by deleting this line.
