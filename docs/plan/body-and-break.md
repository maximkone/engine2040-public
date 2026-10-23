# Body and break (Bodies)

Outcome: cuts, limbs, ragdoll, collapse and fire give one hash on every PC and in replay; a blade that meets an arm decides, the same way everywhere, whether it cuts, where, and what the fighter can still do.
Proof: (1) `cargo test -p engine-proof break_corpus` x2 (10,000 seeded cuts, 16-byte event, byte-equal); (2) PX17 64-fighter snapshot and rollback bytes inside budget, and the HS2 golden matrix on two architectures; (3) gore Off, Reduced, Full give one golden hash; an 8-fighter soak with destruction on, hash x2.
Order: PX17, then HS2 (FOCUS Now, Bodies); then section 7 in table order. BX28, BX37, BX38 and PX6 are ticked (done rows in `docs/archive/SPRINT-done-2026-10-10.md`).
Crates: physics, destruction, combat, arena, anim, render, assets, proof, net, endpoint, playtest, editor.
Open board IDs: PX17, HS2 (Now); the table in section 7.

Tag NOW (Bodies). Prefixes: BX, DX, PX (physics merged as `### physics and destruction base`). Plain summary: the authority layer is a small exact truth (integers, a 16-byte cut event) that every reader of break events shares; the GPU paints the rich look and can never change an outcome. Today the arena breaks a crate along baked bonds, spreads cracks, caps debris, ragdolls a fighter and sprays presentation blood. No limb loss, no load paths, no fire, no cuttable ropes; "dismember" is a knockdown cue.

This plan extends the physics base (PX, DX rows stay as written; BX rows sit on top and say what they need).

Merged 2026-10-10 (batch A of the plans review): `docs/archive/2026-10-10/plans/physics.md` as `### physics and destruction base`. Dated Status lines, the donor table, the physics hard problems and the physics session estimate are in `docs/archive/2026-10-10/plans/body-and-break-cut.md`.

Pre-merge text: `git show d96091cd:docs/plan/body-and-break.md`; `git show d96091cd:docs/plan/physics.md` (physics is archived at `docs/archive/2026-10-10/plans/physics.md`).

Status 2026-10-17 (owner: realistic physics in the picture): the playable window has no rigid-body physics, no ragdoll, no limb and no debris on screen; everything in this plan is authority or a library until the physical layer (`crates/physfx`, rows RZ1-RZ9 in `docs/plan/web-and-realism.md`) paints it from the authority events. Nothing in section 7 below changes: BX and DX rows stay the authority truth; RZ rows are the pictures and the budgets.

## 1. Where it stands

Rows that are ticked are dropped (git history and the board keep them). Kept:
the open capability rows of both plans.

| Capability | Status | Evidence (file or commit) |
|---|---|---|
| Gore events (CB2) | unit-tested, presentation only | `crates/combat/src/gore.rs`: `DismemberLite` is a knockdown cue, no cut |
| Limb body, limb-loss locomotion (BX4, BX5) | missing (rowed) | no `limb.rs`, `limbloss.rs`, `injured.rs` (glob 2026-09-30) |
| Structural load paths, fire, cuttable ropes, deformable ground | missing (this plan) | no `structure.rs`, `fire.rs`, `tie.rs`, `ground.rs` |
| Break verbs on the MCP door, BreakBench | missing (this plan) | no `break_tools.rs`, no `breakbench.rs` |
| Balance and stumble (PX4) | landed, PX4 ticked (cf0c1fb8, closed by PX25; the row sits in `docs/archive/SPRINT-done-2026-10-10.md`) | `crates/physics/src/balance.rs` (`BalanceState`, COM, stagger, stumble, fall; PX23-PX25 ticked); `cargo test -p engine-physics --lib balance` 13 passed |
| Physics-driven locomotion student (PX5) | PX5a landed (`29515678c`), PX5b leg 1 landed, leg 2 measured-blocked | `crates/physics/src/gait.rs` (walk cycle, bias, step-out), `crates/distill/src/train.rs` (continuous-target fit); no `learned_ctrl.rs` - read the 2026-10-24 status below |
| Deterministic parallel islands (PX6, ticked in FOCUS) | missing (split PX37-PX39 + PX15 salvage) | `islands.rs` builds only (canonical union-find + sorted output); `rapier.rs` spawn-order |
| Ragdoll contacts + joint tables (PX9) | shaped-but-unproven | depth-based swing/twist limits and parent-child `contacts_enabled(false)`; no `joint_limits.rs`, no `InteractionGroups`; needs PX1, PX10 |
| Continuous collision CCD (PX10) | shaped-but-unproven | `CCDSolver` plumbed; the one `lod::ccd_for` policy turns it on for a dynamic bone at spawn (AD17) and off for a driven body; the weapon/projectile arm has no caller yet |
| PID servo (PX11), servo corpus (PX22) | missing (rowed) | no `crates/physics/src/servo.rs`; velocity-set only |
| Swept shape-cast API (PX20) | missing (rowed) | segment-cast exists (`QueryBvh::cast_into`, BoxMm only); needs PH1, GP2 |
| Runtime Voronoi fracture (DX5) | shaped-but-unproven | runtime Voronoi in `crates/destruction/src/fracture.rs` + offline `Mode::Voronoi` in `bake.rs`; needs DX4 |
| Debris LOD (DX6), fracture authoring (DX7), bake rank (DX8), graded bonds (DX9) | missing (rowed) | `low_lod` flag still in tree; every bond still 1.0 - code cut 2026-10-16, recover via git log --follow -- crates/destruction/src/bond_grade.rs |
| Debris snapshot + resim (DX13) | shaped-but-unproven | snapshot bytes, digest and restore in `crates/destruction/src/snapshot.rs`; resim and rollback open; needs DX4, PX17 |
| Combined 64+500 tick proof (PX19), 1,000-agent LOD (PX16) | missing (rowed) | no perf workload posts the physics slice at 64 fighters + 500 agents |

- Status 2026-10-12: BX4 row above says no `limb.rs`, but `crates/physics/src/limb.rs:1` and `crates/arena/src/limbs.rs:1` exist on disk, so that cell is false.
- Status 2026-10-17: disk shows `crates/physics/src/ragdoll.rs:378` and `crates/physics/src/powered.rs:317` call `.ccd_enabled(true)` (rg hits), so the `:32` "`ccd_enabled` 0 hits" cell and the `:45` "CCD is off by grep" line are false at those two sites.
- Status 2026-10-20 (AD17): the one CCD policy `lod::ccd_for(kind, motion, speed, cfg)` now has callers: `rapier::RagdollParams::ccd_flags` reads it at spawn for `ragdoll.rs` and `powered.rs`, so a dynamic bone rides the solver from spawn and a kinematic body never does. The PX10 cell above is corrected in place.
- Status 2026-10-17: disk shows the false BX4 cell still at `:25` ("no `limb.rs`, `limbloss.rs`, `injured.rs`; glob 2026-09-30") with `:40` already marking it false, while the code names now exist: `crates/physics/src/limb.rs:1`, `crates/arena/src/limbs.rs:1` and `crates/combat/src/limbloss.rs:1` (Test-Path True, `limb`/`limbs`/`limbloss` under `crates/`), and only `crates/physics/src/injured.rs` is still absent (Test-Path False), so the "no `limb.rs`" half stays stale and the `:25` cell stays false.
- Status 2026-10-17 (physics and destruction cut, on purpose): rot packet `0-rot-physics-dead-r1` removed about 17,000 lines that had no production caller. The bytes sit in `auto: backup` commits `3d6ee4a1` (physics, 14,466 lines) and `3387a96e` (destruction, 2,681 lines). The landing record is `9fdee9a4` ("cut: physics dead-module wave", review PASS, 95 physics tests green at the time). Removed: `cloth.rs` (PX2, PX27-PX29), `grapple.rs` (PX3), `lod.rs` (PX7), `vehicle.rs` (the D1 vehicles line), `query.rs` (UW1, and the `QueryBvh` cast that the PX20 cell cites), `solver.rs` and `verify.rs` (EG5), `substep.rs` (PX31), `broadphase.rs`, `cast.rs`, `checked_fix.rs`, `islands_build.rs`, `simd_math.rs`, and in destruction `bond_grade.rs` (DX9), `nav_dirty.rs` with `tests/contract.rs` (DX10), `debris.rs`, `wall_slice.rs`. The ticked rows stay ticked, so read PX2, PX3, PX7, D1 and DX10 as "built, then removed". Every line in this file that counts cloth, grapple, vehicles or army LOD as landed (the "Landed and ticked" list, the ownership line, the `step_cloth` sketch, the 2.5 ms and 4 ms budgets) describes code that is gone. Recover with `git log --follow -- <path>`.
- Status 2026-10-17 (PX6, parallel islands): PX6 is ticked, but no parallel island code is in the tree. `step_islands`, `ParConfig` and `JobPool` left with `islands.rs` in `3d6ee4a1`. A slim `islands.rs` came back today (`bc72d03d`): `roots`, `build_islands` and the snapshot half that the Rapier snapshot needs, nothing parallel. PX15 is the open row for real parallel islands.
- Status 2026-10-17 (board vs this plan): PX17 is ticked on the board and HS2 is open, so the Order line and the `[ ]` PX17 row in section 7 are older than the board. Checked today: `cargo test -p engine-physics --lib snapshot` 8 passed (the PX17 Accept command) and `cargo test -p engine-physics --features rapier --lib` 152 passed. Not run for this status: `cargo test -p engine-proof break_corpus` and the destruction suite.
- Status 2026-10-17 (open rows that name removed files): DX9 (`bond_grade.rs`), PX16 and PX19 (`lod.rs`), PX20 (`query.rs`), PX35 and E81 (`cloth.rs`), E28 (`solver.rs`), NS4 (`verify.rs`), BX9 (net `server.rs`), PX41 (game `ballistics.rs`). Each needs its file written again, or the row re-scoped. Owner call: restore from git, or re-scope.
- Status 2026-10-17 (red test, owner call): `cargo test -p engine-arena --features sim --lib` is 182 passed, 1 failed: `tests::loc2_wired_slope_step_air_course_x2`. The controller rewrite in `1d01a410` made autostep need `was_grounded`, so the capsule hops at a ledge face. This is a behavior change, not a build break. Nobody loosened or ignored the test. Choose: allow the air step after a step, or probe the full capsule radius after a step.
- Status 2026-10-24 (PX5b leg 2a, measured): the residual corrector has nothing to correct on the PX5a body. A probe swept the servo-target correction over 20 seeded stagger-threshold shoves (bias scale x0, x1, x2, x4, x8; recovery step 0, 150, 220 mm) and then out to eight times the threshold impulse (256, 512, 1024, 2048 Q8). What came back: the torso never touched the floor in 60 ticks at any level or any policy, the pelvis low point stayed 0.945..0.952 m and the head never left 0.90 m, so "upright again" fired at tick 10 in all 24 rows. Bias scale changed no metric at all, and the reason is in the code: for a straight-on shove `balance::servo_bias_targets` returns only an ankle shift (`hip_q8` 0, measured ankle -56 Q8), `gait::write_pose` puts that shift on the foot rotation, and the fixture's feet are spheres, so the shift is mechanically inert. Measured against a steady twin, the root is off by at most 0.022 m of height and back inside 0.005 m by tick 8, corrected or not. The only 10-tick-scale quantity here is the stagger timer (`balance::STAGGER_TICKS` 30), and the bias credit that could shorten it is capped at 5 (`balance::RECOVERY_CREDIT_MAX_TICKS`). So the row's bar - "a stagger-threshold shove recovers at least 10 ticks sooner" - has no headroom on this body, and the walk's root position target is why: the drive is stiff enough that no blow tips it. Owner call, two ways: kill leg 2 and ship PX5a plus the landed `engine-distill` continuous half (the row's own Kill clause), or make balance loss physical first (a root the ground can push, not a root a target places) and re-measure. Probe scaffolding lived in `crates/physics/tests/it/gait_walk.rs` and was removed with these numbers recorded.

Step contract (kept green forever): fixed 60 Hz, fixed iterations, sorted
insertion, hash x2 plus snapshot/restore equality; cloth stays
presentation-only and never enters the hash. Scale rows PX15-PX18 and
DX11-DX12 are boarded open; CCD is on for a dynamic bone through
`lod::ccd_for` (AD17).

Status 2026-10-17: disk shows that "CCD is off by grep" text two lines lower than the `:45` cite in the bullet above, at `docs/plan/body-and-break.md:52` (`rg -n "CCD is off by grep" docs/plan/body-and-break.md` returns only `47:`), and the code side still holds, so that line and the `:32` "`ccd_enabled` 0 hits" cell stay false: `crates/physics/src/ragdoll.rs:378` and `crates/physics/src/powered.rs:317` call `.ccd_enabled(true)` (rg hits), `crates/physics/src/lod.rs` is gone (Test-Path False) and `rg -n ccd_for crates` exits 1 with 0 hits, while bomb CCD still runs inside the solver as `CCDSolver` at `crates/physics/src/ragdoll.rs:299`, `crates/physics/src/powered.rs:248` and `crates/physics/src/rapier.rs:144`.

### physics and destruction base

The arena already falls, grapples, wears capes and breaks crates the same way
twice. Open: shoves that stagger, clean bone collisions, fast blades that never
tunnel, PID servants, parallel islands, runtime fracture, graded bonds. One
physics backend: Rapier with `enhanced-determinism` at fixed 60 Hz. Owner lanes:
`physics` (`crates/physics`), `destruction` (`crates/destruction`); consumers
`arena`, `net` (rollback), `nav` (NV3), `perf` (PF1).

Landed and ticked (kept as the base, not re-done): PH1 Rapier core with
hash and snapshot, PH2 controller, D1 vehicles, PH3 ragdoll blend, PX1 per-bone
servo with the 60-tick get-up, PX2 cloth (CPU verlet, presentation-only), PX3
grapple, PX7 army LOD, PX8 controller v2, DX1 bonded, DX2 CSG, DX3 spread, DX4
budgets, DX10 nav dirty. Carried over: DX1 break-event shape; cloth never hashed.

Status 2026-10-17: disk shows this PH3/PX1 landed list at `docs/plan/body-and-break.md:66` ("PH2 controller, D1 vehicles, PH3 ragdoll blend, PX1 per-bone"), so the `BOOK-GAPS-2026-10-12.md:188` S10 cite `body-and-break.md:9,54-55` is stale; the disk half still holds: `crates/game/Cargo.toml:14` reads "`app` intentionally does NOT enable `engine-arena/sim`" with `:15-16` the same comment and `:17` `app = ["engine-render/window", "engine-audio/device", "dep:winit", "dep:engine-assets", "dep:gilrs"]` naming no `sim`, while the sim feature that would pull Rapier bodies in sits at `crates/arena/Cargo.toml:26` `sim = ["engine-physics/rapier"]`, so S10 still holds with a drifted left cite only.

## 2. Finish line

Tier A (Finish A, the arena): BX1-BX11 and BX28. A gladius sweep severs a
bare forearm or lower leg and cannot pass a manica or greave; the fighter
reacts (drops the shield, falls, crawls, yields, fights one-handed); the
limb lies on the sand as a real body; 64-client soak at 8 cuts/s shows zero
desync; gore Off/Reduced/Full give one golden hash. Bar: cut event
16 bytes; 10,000 seeded cuts byte-equal x2 on two PCs; authority ≤ 0.15 ms
per cut, ≤ 0.5 ms per tick at 64 fighters; presentation ≤ 0.3 ms per cut.

Tier B (siege and urban war, Finish B and C): BX12-BX20, BX24-BX27, BX29.
A 3-story, 400-element building collapses by load path, identical x2 and
across rollback; ties cut and drop their load; fire spreads by material;
ground keeps craters and traction; a 10-minute 64-fighter siege replays
with an empty diff; an agent authors a wall and a creature from a brief.

Tier C (on demand, measured first): BX21-BX22, BX30-BX32. Hero-tier flesh and
cloth tearing, wound masks, crowd LOD for cutting, learned injured locomotion,
creature onboarding from any rig. Each needs a blind A/B or a number before it
stays.

CUT (never): GPU-authoritative physics, material-point/FEM soft bodies in
the tick, authoritative fluids, a second physics backend, kernel anticheat.
Blood, dust, jiggle, cloth tear and GPU debris are presentation (any float,
any GPU — never the hash). `docs/archive/2026-10-10/plans/physics.md` CUT "custom XPBD" is revised to
"XPBD in the tick" (presentation XPBD allowed); tick XPBD stays CUT.

Best implementation: see the competitor table in §4 and Scorecard rows.


### physics and destruction base

- Tier A alpha (arena holds together at scale): PX1-PX3 plus DX1-DX4 and DX10 stand; PX9 contacts, PX10 CCD wiring and DX9 grading are required, plus the scale proofs PX16 (1,000-agent LOD, physics slice p99 <= 2 ms), PX17 (64-fighter snapshot in budget), PX18 (8-fighter clinch soak), PX19-PX22 and DX13-DX14. Bar: 8-fighter soak with destruction on hash x2; a 40 m/s blade never passes a 5 cm shield in 1,000 seeded swings; no bone interpenetration over 1 cm in 50 seeded falls; anchors hold until bonds fail; per-event 16/1024 caps hold; 64+500 tick server p99 <= 4 ms (P6); debris restores byte-equal under rollback.
- Tier B v1: PX4 balance, PX11 PID, PX12 DAT coupling, DX5 runtime fracture, DX6 debris LOD, DX7 authoring. Bar: a 5-strength shove gives stagger, stumble, fall in order and recovers in budget; get-up jitter <= servo at <= 10% overhead; 1,000 debris within the DX4 budget, merges identical x2.
- Tier C (only on demand): PX5 learned locomotion, DX8 bake-rank taste. Bar: AI walks, turns and recovers from shoves with replay exact x2; a blind A/B against LM1 decides adoption, else killed with numbers; no mocap bytes in git, no non-commercial training data in a shipped student (DT5 manifest gate).
- CUT (never): a second physics backend, custom XPBD in the tick (presentation-only XPBD allowed), soft-body shields in the tick, naval, online RL in the tick, vendor denoiser in sim. Cores deepened, never forked: the Rapier step contract, `powered.rs` feel, `budget.rs` scale - code cut 2026-10-16, recover via git log --follow -- crates/destruction/src/nav_dirty.rs (`nav_dirty.rs` nav contract had zero non-test callers).

### cross-architecture matrix (HS2, FOCUS Now)

The row is kept verbatim (it is also the Now row in section 7).

- [ ] HS2 proof: cross-architecture golden matrix — the golden and the BX28 corpus on Windows x86-64, Linux x86-64 and, if a runner exists, aarch64. Tier A. Size M. Owns hunks in `crates/proof` and CI files. Accept: digests equal or the first divergent tick is named with its cause (sort tie, libm, FMA). needs: PX17.

## 3. Target design


Two layers: the tick knows little exactly; the screen knows a lot.

**Authority layer (in the hash, integer and Q16, stable order).** One typed
`BreakEvent` family, tick-stamped, logged, replayed and rolled back like any
input: `Cut` (16 bytes: tick, victim, joint, plane, blade, energy, pad),
`Break` (DX1 shape), `Collapse` (root id + tick), `Ignite`, `Sever Tie`,
`Crater` (cell range). Everything downstream (limb body, wounds, nav dirty
tiles, AI observation, sound, blood, gore level) reads the event; nothing
reads a rendered pose.

**Anatomy (BX1).** A creature is a joint graph plus tissue layers: 15
severable joints (neck, shoulders, elbows, wrists, hips, knees, ankles,
waist bisect), three layers per segment (skin, flesh, bone) with Q16
toughness and per-armor coverage (galea, manica, greave, lorica, scutum).
Data, versioned, checker-validated, agent-editable. Any rig can carry an
anatomy (BX32), so the engine stays genre-neutral.

**Cut resolution (BX2).** BX2 lands a pure integer resolver over the
combat-local sweep (`cut.rs`, ticked, needs `BX1, GP2` — the PX20 shared
shape-cast API stays a physics-lane follow-up, never a BX2 gate). The
resolver intersects the swept segment with each segment capsule in
integers, tie-breaks by (victim, joint) order, compares blade energy x
edge sharpness (decays with hits) against tissue toughness plus armor,
and emits at most one `Cut` per joint per swing. The plane is the swept
edge's normal at contact, quantized to 256 steps. Nothing reads a
skinned mesh, so drivers and GPUs cannot change a cut.

**Limb as a body (BX4).** At the cut tick the segment chain leaves the
victim's ragdoll as a new rigid body: mass from volume x density, velocity
from the parent bone plus blade impulse, equal-and-opposite recoil into
the stump servo. Creation order is canonical (event order, then joint
id); the body count is capped with debris; it joins every snapshot once
PX17/DX13 land (BX4 carries the membership, BX19 proves resim).

**Life after loss (BX3, BX5, BX6).** Bleed rate, shock, exsanguination,
surrender/morale break are tabled and deterministic. The joint mask is
rebuilt, balance (PX4) gets a new support polygon, locomotion switches to
hop, crawl or fall inside 20 ticks. Game AI receives a capability
observation and planners choose one-hand moves, retreat, yield or press on;
a witnessed cut can break morale (tuned, replayed exactly).

**Cut presentation (BX7, BX8, BX21, BX22).** The cook (BX8) gives each
character interior templates per segment and proves them watertight. At
runtime the GPU clips at the exact plane (one clip plane per segment, no
CPU mesh generation), draws the cap, wets it with blood from the bleed
rate, and hangs a wound mask for the match. Jiggle is XPBD, cloth tears
are sheet tearing, all presentation. Gore Off/Reduced/Full changes only
what is drawn (Off: impact flash and a guard/downed pose; Reduced: blood
without severing); the hash is identical (BX10).

**World (BX12-BX20).** Materials (BX12): 14 with toughness, shard rule,
sound, burn rate. Structure (BX13): support graph from anchors; boarded
load-leg computes per-chunk integer loads, exposing the weakest link;
sliced passes (≤ 1 ms/tick) cascade identically. Fracture (DX5) takes cell
count/size from energy and material (BX14). Ties (BX15): rope, chain,
winch, hinge, chandelier. Fire (BX16): integer heat/fuel grid, burns
weaken BX13. Ground (BX17): Q16 height/traction. Settle/freeze (BX18).
One snapshot (BX19) covers all — rollback byte-equal.

**AI-agnostic door (BX24-BX27, BX32).** Data behind typed verbs on the
MCP command service (one write path, undo, typed refusals):
`anatomy.set`, `material.set`, `structure.analyze`, `fracture.preview`,
`cut.preview`, `hit.simulate`, `wound.table.set`. The brief (BX27) reports
cover, sightlines, load-bearing warnings. BreakBench (BX25) pins 20 tasks;
parity prints every 5 waves.

Budgets: cut authority 0.15 ms/event; structure 1 ms/tick sliced; fire
0.3 ms for 250 torches; ground 0.2 ms; BX adds ≤ 0.8 ms server p99.

### physics and destruction base


Data flow (one direction, no cycles):

```
combat move tables -> swept volumes -> physics step (Rapier + servo)
  -> contacts/hits -> ragdoll/balance/grapple -> checkpoints + hashes
  -> destruction break events (DX1 shape) -> budgets/LOD -> NV3 rebake tiles
  -> presentation (cloth verts, debris poses, R5 particles)
```

Ownership: `crates/physics` owns bodies, servos, ragdoll, cloth,
grapple, islands, controller, balance/servo/lod joints.
`crates/destruction` owns bonded sets, bakes, spread, budgets,
fracture/LOD/grade ranks. `arena` owns match wiring; `net` owns
rollback/resim; `editor` owns the DX7 paint tool with undo. Bakes stay
offline; the tick stays integer/`Fixed` with sorted orders.

Threads: sim thread only for the tick. RC3 fork-join may parallelize
bake/verify/broadphase lanes later, never the sim tick; Rapier parallel
stays DROP. PX6 splits islands into RC3 chunks with a sorted merge only
if the same hash holds under 1, 2, 4 and 8 threads, else killed.

Determinism: `+ - * /`, `sqrt`, integer/`Fixed` math and stable orders
only in sim; no `HashMap` iteration reaches hashed state; bond/graph
traversals sort by ID first; contacts read during the step, mutated
after it (Jolt `ContactListener` discipline). Sim builds hash
differently by tag byte BY DESIGN (PF1 lesson); baselines pin per track
and every row posts which baseline it proves.

Budgets (kept green forever): physics plus cloth ≤ 2.5 ms/tick at
8 fighters; cloth 0.5 ms; spread 0.5 ms per event; debris amortized
1 ms; fracture events 1 ms each; PX7 army (500 + 64 fighters) under
4 ms physics on the reference CPU; PF1 workloads extended, never forked.

API sketch (existing, rows deepen — never mirror):

```rust
pub fn step_cloth(world: &mut ClothWorld, dt_ticks: u32); // fixed-point, exact replay
pub fn spread_cracks(set: &mut BondedSet, hit: BondId) -> Vec<BreakEvent>; // sorted ID order
pub struct DebrisBudget { pub per_event_cap: u32, pub sleep_ticks: u32, pub lod_distance_mm: i32 }
pub fn build_islands(n: usize, pairs: &[(usize, usize)]) -> Vec<Vec<usize>>; // canonical, sorted
```


**Steals 2026-10-12** (board shard `../SPRINT-21-look-feel.md`, scout files `../research/steals-2026-10-12/`). Stump cap = the clipped skinned segment drawn two-sided: back faces shade as cap (skin rim, flesh ring, bone disc) and write frag_depth from the plane hit; one plane uniform per draw; stencil capping only for non-watertight segments (BX41, needs BX8). Ragdoll: a humanoid joint table per bone (hinge knee and elbow, cone plus twist elsewhere), a friction motor of 5-10 N.m, a soft spring, mass ratio clamped to 0.8-1.2 (Box3D human.c, Jolt Stabilize; PX42, PX43). Limb rest angle and chunk tumble are client-local presentation seeded from the event, never hashed.
- Fix: CCD is ON for ragdoll bones (`crates/physics/src/ragdoll.rs:378`, `powered.rs:317`); the line saying it is off by grep is stale.

## 4. Hard problems


Competitor cells are estimates from public material until replaced with
dated, sourced numbers; games are design references, never copied.

| Problem | Technique | Why | Reference (and what it lacks) | Donor |
|---|---|---|---|---|
| Same cut on every PC | swept blade vs segment capsules in integers, plane quantized to 256, order by (victim, joint) | float mesh slicing differs across CPUs and GPUs and breaks replay | Blade and Sorcery and Overgrowth slice at runtime in floats (single-player, no replay guarantee, est.); Mordhau and Chivalry swap pre-cut meshes (not physically driven, est.); Dead Space authored cut points | our own; Rapier for queries (Apache-2.0, already in) |
| Cut that looks continuous | GPU clip plane at the exact plane plus cook-time cap templates | quantized authority, exact-looking screen; no runtime mesh generation stalls | TLOU2-style authored gore parts (fixed cut lines, est.) | our own; sever cook validator |
| Fighter after loss | rebuild joint mask, new support polygon, injured locomotion states, capability observation for AI | a fighter that ignores a lost leg breaks the game | Dwarf Fortress tissue and injury depth (no real-time body, ideas), Rimworld body parts (no locomotion), Kenshi limb loss (no replay) | our own on PX1, PX4 |
| Walls that carry weight | integer message-passing load propagation over the bonded graph (BFS layers, smallest-index parents, ascending post-order, canonical digest) plus time-sliced relaxation from anchors | learned float GNNs drift and break replay; integer aggregate-add replays x2 at 50 us | Best rival: geoelements/gns Graph Network Simulator (MIT, message/aggregate/update, learned float weights) plus arXiv 2609.02991v2 TRACE (ideas only); Teardown voxel destruction (float, no authority, est.); Red Faction Geo-Mod (design, no replay) | GNS shape reimplemented std-only integer-only (no bytes vendored); learned float authority killed (fractional-mass split fails the integer guard); Rapier for debris |
| Fracture that scales | seeded Voronoi over convex pieces, cell count from energy and material, budgets and merge | fracture cost must be bounded per event | Müller, Chentanez, Kim 2013 (volumetric approximate convex decompositions, SIGGRAPH); Bao, Hong, Teran, Fedkiw 2007 (fracturing rigid materials); Unreal Chaos Geometry Collections (rich tools, not replay-deterministic across machines, est.) | papers reimplemented; Blender cell fracture is ideas only (GPL) |
| Rollback through a collapse | one canonical snapshot for debris, structure, fire, ties, ground | netcode must resim a break | Unreal replicates state, does not resim (est.); no engine in the scorecard resimulates destruction | our own on DX13 |
| Fire and burning | integer heat and fuel cellular grid, materials decide spread and burn time | 250 torches at night must be safe and burnable | Unreal and Unity fire are VFX or plugins (est.); Noita pixel fire (2D, no replay) | our own |
| Cuttable siege gear | tie constraints with break thresholds and a cut source (blade, arrow, stone) | portcullis, drawbridge and chandelier are Rome | authored trigger scripts in every engine (est.) | our own |
| Sand that remembers | Q16 height and traction layer with normal-map stamps | footprints, craters and slick blood change play | Unreal landscape deformation plugins, Snow tech (visual, not authoritative, est.) | our own |
| Agents author and balance it | typed verbs, `hit.simulate` distributions, fork play-tests, BreakBench | tuning by eye does not scale; agents need a loop that says "this blade cuts arms 31% of clean hits" | no engine offers a typed, undoable destruction and anatomy API with fork tests; MCP servers for Godot and Unity drive editors, not destruction (est.) | our door (`MCP5`), `spatial_eval.rs` pattern |
| Ratings and regions | gore Off, Reduced, Full as presentation only | one build ships everywhere; outcomes stay identical | most engines make gore a content fork (est.) | our own |
**Steals 2026-10-12, hard problem.** Same hash on every PC: Rust never fuses mul-add unless asked, so the C -ffp-contract risk does not apply; the risks are libm trig, sort_unstable on tied keys and HashMap order. HS2 posts a per-tick hash row per leg with one chaotic scene, an expected final hash and settle tick (Avian, Box2D and Jolt CI shapes).

## 5. Donors

Donors: `steals.md`; porting policy is AGENTS rule 4. The donor table (repo, SPDX, take, do not take) and the physics hard problems are in `docs/archive/2026-10-10/plans/body-and-break-cut.md`.

## 6. Slim

- `GoreKind::DismemberLite` folds into a reader of `Cut`; `gore.rs` stays
  presentation only, a mapper from events to blood.
- DX9 graded bonds read the BX12 material table, never a second table; the
  temporary `spread.rs`/`budget.rs` mirrors die when BX13 lands.
- One `BreakEvent` family; no second event bus, no mirrored types.
- No new agent roles: a specialty is a skill the role loads.
- Not built: authoritative fluids, GPU physics authority, FEM/MPM in the
  tick, a second physics backend, creature-specific code in engine crates.


### physics and destruction base


- Delete: the `low_lod` flag once DX6 lands; the TEMPORARY MIRROR
  shapes in `spread.rs`/`budget.rs` once the orchestrator swaps them for
  `bonded.rs` imports (mirrors die, never fork); any second SH/post/
  particle path stays out (look plan owns its singles).
- Merge: PX9 owns contacts plus joint tables plus motors in
  `ragdoll.rs`/`joint_limits.rs` — never a contacts crate plus a joints
  crate. DX9 grading plus density cuts plus sliver reject - code cut 2026-10-16, recover via git log --follow -- crates/destruction/src/bond_grade.rs. PX13/DX10 audits own hunks and tests, never new systems.
- Freeze: the DX1 break-event shape, the 60-tick get-up budget, the
  fixed 60 Hz step with fixed iterations, and the `GRID`/solver order
  proofs. Changes arrive as new tables or layers, never as silent edits.
- What we will not build: a second physics backend or custom XPBD; a
  cloth GPU path; runtime cracks beyond capped seeded Voronoi cuts;
  soft-body shields without a gameplay row; naval; `parry3d`.


## 7. Board tasks

Rows from the steals (2026-10-12, board shard `../SPRINT-21-look-feel.md`, scout files `../research/steals-2026-10-12/`): BX41 stump cap, PX42 human ragdoll joints, PX43 ragdoll Stabilize.

Rows from the books (2026-10-12, board shard `../SPRINT-20-game.md`; `BOOK-GAPS-2-2026-10-12.md` 2.3): PX41 one semi-implicit stepper for siege flight with an exact drop test.

Rows live on the board; slice any row with `node tools/board-slice.mjs <ID>`. Now rows first, with the Accept line verbatim. Open rows of the two merged plans that are on the board are in the table; the plan-only rows (BX3-BX4, BX12-BX20, BX21-BX22, BX24-BX27, BX29-BX32, and others) are not on the board and stay in git until boarded.

- [ ] PX17 64-fighter snapshot + rollback-byte proof. Tier A. needs: -
  Accept: `cargo test -p engine-physics --lib snapshot` green; 64-fighter snapshot restores byte-equal x2; bytes within the posted budget; golden unchanged.
- [ ] HS2 cross-architecture golden matrix. Tier A. needs: PX17
  Accept: digests equal or the first divergent tick is named with its cause (sort tie, libm, FMA).

| ID | Title | Tier | Needs |
|---|---|---|---|
| PX35 | long-range attachments for cloth and soft sheets | B | - |
| PX5 | physics-driven locomotion | C | - |
| PX15 | deterministic parallel islands via chunked split plus sorted merge with step frozen | B | - |
| PX9 | ragdoll contacts and joint tables | A | - |
| PX11 | PID servo | B | PX9 |
| PX32 | CPU SPH fluid step | B | LATER |
| DX6 | debris LOD | B | - |
| DX7 | fracture authoring | B | - |
| DX8 | detail where it breaks | C | - |
| DX9 | graded bonds | B | - |
| PX16 | 1,000-agent LOD + sleep scale proof | A | - |
| PX18 | 8-fighter clinch soak | A | PX9 |
| DX11 | siege-wall fracture at 64-player scale | B | - |
| DX12 | debris-to-GPU handoff proof | B | DX6 |
| PX19 | combined 64-fighter + 500-agent tick proof | A | PX16 |
| PX20 | deterministic swept shape-cast API | A | - |
| PX21 | moving-platform ride | B | - |
| PX22 | servo calibration corpus | B | PX9 |
| DX14 | bond-grade calibration corpus | B | - |
| DX16 | bond-graph damage propagation | B | - |
| DX17 | impact-keyed fracture table over bonded chunks | A | - |
| BX5 | injured locomotion | A | - |
| BX6 | capability observation and reaction | A | - |
| BX7 | cut presentation | A | BX8 |
| BX8 | sever cook | A | - |
| BX9 | `Cut` on the wire and the tape | A | - |
| BX11 | material and tissue sound | A | - |
| BX36 | fracture event at 32 cells under 1 ms | B | - |
| BX39 | `wound.table.get` read verb | B | - |
| BX40 | one parry3d version | B | - |
| BX25 | Blender-bpy fracture column for BreakBench | B | - |

## 8. Quality gates


- Golden hash unchanged; BX tapes hash x2 and snapshot-restore equal.
- Order-permutation proof for cuts, ties and structure (RC3 pattern).
- No sim math outside `+ - * /`, `sqrt`, integer or fixed; presentation
  stays out of checkpoint types by grep.
- Gore sweep: one tape, three settings, one hash.
- BX load-leg gate: oracle-wall weakest 8/8 with digest x2, else red.
- Bounds: per-event caps, body cap, sliced structure pass; a miss is red.
- Captures judged beside a reference, never by feature count.
- BreakBench and provider parity print every 5 waves and go up.


### physics and destruction base


- `cargo test -p engine-physics -p engine-destruction` green; PX1–PX3
  plus DX1–DX4 suites stay green under every row (hash x2, permutation
  proofs, mirror proofs).
- Determinism: same tape twice identical; bond insertion shuffled 8 ways
  gives the same spread set; grapple IDs swapped gives mirrored
  checkpoints; insertion order never reaches a hash.
- Budgets: physics plus cloth at most 2.5 ms/tick at 8 fighters; cloth
  0.5 ms; spread 0.5 ms per event; debris amortized 1 ms; fracture
  events 1 ms each; PX7 army bar posted on the reference CPU first.
- Safety: no platform transcendentals in sim (D1 audit strict); no
  `Unsafe` in new modules; typed refusals, never panics, on bad input.
- License gate: every pattern port carries `PORT()` plus an
  `EVIDENCE.md` row with the verified SPDX URL above; UNVERIFIED stays
  a paper-shaped reimplementation with no donor code.

## 9. Session estimate

Moved to git history: `git show d96091cd:docs/plan/body-and-break.md` (section 9) and `docs/archive/2026-10-10/plans/physics.md` (section 9). Each row carries its own estimate on the board.

