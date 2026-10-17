### Plans (planner packets, `docs/plan/README.md`)

Each row writes one plan file and proposes rows with its prefixes; the
orchestrator reviews them and moves accepted rows onto the board. A planner
reads the matching `docs/systems/` spec first and keeps what is still true;
after review the plan replaces that spec.

- [x] PLN18 plan: general engine — audit every crate for arena or genre Landed 31421a87. Plan on disk: `docs/plan/runtime.md` carries the genre-neutral ledger (GE1-GE20, ticked and open rows) and the GE rows boarded; fixture genres prove generality (`tests/ge1_genres.rs`, commit b0914f4e).
  assumptions (names, fighter caps, units, camera, modes), propose moves behind
  `arena`/`game`, and fixture genres that prove generality. Owns
  `docs/plan/runtime.md`, prefix GE. needs: none.
  LATER: parked 2026-10-03 (circus verdict: planning backlog while 300+ rows are ready (RESTRUCTURE-2026-10-02)); unpark by deleting this line.
- [x] PLN19 plan: living world and AI builds inside — NPC mind contract, Landed b36a8f47. Plan on disk: `docs/plan/living-world.md` with the LW and AB rows (NPC mind contract, memory, provenance, brief to playable slice) boarded; ticked LW/AB rows listed in its status table.
  memory streams, goals, dialogue, recording for replay; brief to level,
  character, encounter and playable slice through the command door. Owns
  `docs/plan/living-world.md`, prefixes LW and AB. needs: none.
  LATER: parked 2026-10-03 (circus verdict: planning backlog while 300+ rows are ready (RESTRUCTURE-2026-10-02)); unpark by deleting this line.
- [x] PLN20 plan: AI-native architecture, spine first. The finding (audit Landed 31421a87. Plan on disk: `docs/plan/runtime.md` section `### spine` plus `### ARC rows` (ARC1-ARC19 in dependency order), research sweep in `docs/RESEARCH-VERDICT.md`.
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
- [x] PLN21 plan: Finish B breakdown — split `FINISH-RICH.md` Finish B Landed 627041ab. Plan on disk: `docs/plan/quality.md` Finish B breakdown rows FB1-FB7 (transport, versions, ship, AI pillars, soak, matrix, workflow).
  (second game through public APIs, dedicated-server transport under loss,
  stable versions and migrations, templates and onboarding, soak with no
  unbounded growth, hardware matrix, stranger-dev workflow, AI-native
  pillars in the shipped sample) into Tier B board rows in dependency
  order, the first ones S-size. Owns `docs/plan/quality.md`, prefix FB.
  The second game is urban war (owner 2026-09-26): UW39 is its
  second-game receipt. needs: none.
  LATER: parked 2026-10-03 (circus verdict: planning backlog while 300+ rows are ready (RESTRUCTURE-2026-10-02)); unpark by deleting this line.
