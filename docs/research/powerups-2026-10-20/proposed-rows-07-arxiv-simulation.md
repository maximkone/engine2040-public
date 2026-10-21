# Proposed rows: arXiv simulation (SAX) — 2026-10-21

Lane: the arXiv simulation wave (`docs/research/powerups-2026-10-20/07-arxiv-simulation.md`),
narrowed to its three strongest finds. One row per find. This file edits no board: the lead merges
`docs/SPRINT*.md`, `docs/FOCUS.md` and `docs/plan/finish-line.md`.

Every crate these three rows touch is held today, so this file carries **rows plus patch-ready
specs**, not code:

| crate | held by | what that blocks |
|---|---|---|
| `crates/anim` | the AM24 landing agent | SAX1 (the rise controller) and SAX2 (the rotation lane) |
| `crates/ecs`, `crates/arena` | the layers lane, E21 job pool (LYR-2, LYR-3) | SAX3 (the visibility barrier) |
| `crates/physics` | the collisions lane | nothing here: no SAX row needs a physics edit (see SAX1) |

Nothing in this file is committed, added or pushed. No file under `crates/**` or `tools/` was
touched.

## Method note: how the papers were read, and the 429

The arXiv **export API answered HTTP 429 on the first query of the wave on 2026-10-20 and kept
refusing**, so the wave read every paper from its `arxiv.org/abs/<id>` page (webfetch, HTTP 200)
and took the version and the date from the live `Submission history` block. This file re-read the
same three abs pages live on **2026-10-21**: version, submission date, title and abstract below are
OBSERVED on those pages, and **no abstract was taken from memory**.

One correction to the note above, so the next reader does not repeat it: a single-title export
query today returned HTTP 200 —
`https://export.arxiv.org/api/query?search_query=ti:%22RepliCore%22&max_results=1`. The API is
fine for a narrow query; it was the wave's broad first query that was refused. The rows still cite
the abs pages, because those carry the submission history the API's `updated` field hides.

The arXiv licence text on all three pages is the arXiv non-exclusive grant; no paper ships code,
so no row vendors bytes (`RULES.md` licence rule, decision D5). Where the wave searched GitHub and
found no repo, the row says so.

## Corrections this lane found while reading the tree (read before the rows)

1. **The get-up state machine is not in `crates/physics/src/ragdoll.rs`.** That file is the
   deterministic active ragdoll (`ragdoll.rs:1-38`): one Rapier body per bone, joints, blend weight,
   hit impulses, a substep coupling seam. It has no rise mode: `grep getup crates/physics/src/ragdoll.rs`
   is empty. The get-up lives in three other places, and the row has to name all three:
   - `crates/physics/src/powered.rs`: `RagdollMode::GettingUp`, `getup_remaining`,
     `pub const GETUP_BUDGET_TICKS: u64 = 60` (`powered.rs:39`), `PoweredRagdoll::start_getup`
     (`:601`) and `blend_weight` (`:538`), all behind the `rapier` feature. Float, client-local.
   - `crates/physics/src/balance.rs`: the integer authority. `BalanceState::getup_due` (`:572`),
     the `Fallen -> Catching -> Steady` windows (`:691-702`), `stance_legal` (`:709`) and
     `rise` (`:878`), which calls `start_getup`.
   - `crates/game/src/ragdoll_look.rs`: the picture. `getup: Vec<Inertial>` (`:360`),
     `getup_age` (`:362`), `ragdoll_stand_up` (`:1120`) and `ragdoll_getup_offsets` (`:1154`),
     with `const GETUP_TICKS: u32 = 150` (`:67`). It cross-fades two poses; it decides nothing.
2. **`E21` and `C15` are finish-line gap numbers, not board rows.** `docs/plan/finish-line.md:78`
   gap 21 is the job pool; `:66` gap 15 is the physics oracle spike. On the board, `E21` is a
   retired flow row (`node tools/board-slice.mjs E21`: "Landed obsolete" 2026-10-19) and there is
   no `C15` row at all (`board-slice: error: unknown row C15`). The live rows for the job pool are
   **PW20** (`docs/SPRINT-28-powerups.md:131`), **RC36** and **RC37** (`docs/plan/runtime.md:250`).
   The oracle spike has no board row: the row below cites gap 15 and PW20, and proposes nothing new.
3. **The parallel island code is in the tree.** `docs/plan/body-and-break.md:47` says
   `step_islands`, `ParConfig` and `island_chunks` "left with `islands.rs`"; they are back:
   `island_chunks` (`islands.rs:133`), `merge_sorted` (`:146`), `step_islands` (`:189`, takes
   `&JobPool`, merges by chunk index) and `build_islands` (`:279`). The island side already
   publishes by construction, so SAX3 does not touch `crates/physics` at all. That stale line is
   why the wave thought islands needed work.
4. **LYR-2's "make the shared pool public" half is done.** `pub fn shared()` is public at
   `crates/ecs/src/pool.rs:336` and `workers_from_env` at `:353`. What E21 still lacks is the sim
   caller (LYR-2/LYR-3) and the publication rule (SAX3).
5. **SAX1 does not close BX5's 20-tick bar.** BX5 asks for "a legal state within 20 ticks"; the
   integer gait table is `FALL_PRONE_TICKS = 30` (`balance.rs:35`) plus `CATCH_TICKS = 15` (`:39`),
   so a fresh fall reaches `getup_due` in 45 ticks, not 20. SAX1 closes RZ2's open "get-up clip"
   item and *feeds* BX5; BX5's bar stays open until the wave's other BX5 row shortens the window
   for an injured body. Do not read SAX1 as a BX5 close.

---

## SAX1 — the rise-motion get-up controller (no keyframes, no offline solve)

**What it is.** A low-dimensional reference controller: COM height, the two hands and the two feet
are the whole state, and continuous integer targets from them drive the IK we already own.
The rise becomes four planted phases instead of a cross-fade between "lying" and "standing".

- **arXiv**: https://arxiv.org/abs/2304.05056 — **2304.05056v1**, submitted 2023-04-11
  (`Submission history`: "Tue, 11 Apr 2023 08:26:11 UTC"), abs page read **2026-10-21**. Title
  "Real-Time Character Rise Motions", Ben Kenwright. [OBSERVED]
- **What it replaces in the tree**: `ragdoll_getup_offsets` (`crates/game/src/ragdoll_look.rs:1154`)
  fades an `Inertial` set built by `begin_switch` from the lying pose to the standing pose, over
  `GETUP_TICKS = 150`. There is no roll, no hand plant and no foot plant: the body passes through
  poses a person cannot hold. [OBSERVED]
- **Closes**: **RZ2** (its own open item, "a get-up clip"; the row landed 2ca37f68 with that item
  still open) and the fall/get-up half of **BX5**. Feeds nothing else: AM24's pops are a rotation
  cap and are SAX2's business, not this row's.
- **Owned paths**: `crates/anim/src/rise.rs` (new), `crates/anim/src/lib.rs` (one `pub mod rise;`
  line), `crates/game/src/ragdoll_look.rs` (the get-up hunk only). No `crates/physics` file.
- **Accept**: `node tools/gh-check.mjs cargo test -p engine-anim --lib rise` — prints
  `test result: ok. 5 passed; 0 failed` and the receipt line
  `rise poses=6 ticks_max=58 alloc_calls=0`. Then
  `node tools/gh-check.mjs cargo test -p engine-game --features app --lib ragdoll_look` — prints
  `test result: ok. 10 passed; 0 failed` and the existing `sand_rest`, `hitstop` and `real_duel`
  proofs stay green.
- **Kill line**: the rise needs a float optimizer inside a hashed tick, or it does not settle from
  all six lying poses inside `GETUP_TICKS`, or the golden `d57b5c8ae3096614` moves.
- **Cost**: M (the controller is S; the six lying-pose fixtures and the game hunk are the rest).
- **Verdict**: **PORT-CANDIDATE** (idea only; no donor code, no bytes).

## SAX2 — the additive rotation lane on the real skeleton

**What it is.** The additive layer carries position offsets only, so `hit_react` writes identity
rotations and a flinch has no turn at all. This row adds the rotation lane, as a delta on the base.
It is a citation and one extra test on top of AMX-2, not a second lane: see the spec block.

- **arXiv**: https://arxiv.org/abs/2303.11886 — **2303.11886v2**, v1 2023-03-21, v2 2023-06-19
  (`Submission history`: "Mon, 19 Jun 2023 21:57:36 UTC"), abs page read **2026-10-21**. Title
  "Fast Complementary Dynamics via Skinning Eigenmodes", Benchekroun, Zhang, Chaudhuri, Grinspun,
  Zhou, Jacobson. [OBSERVED]
- **What the paper gives us, and what stays out**: the useful part is its *statement of the
  property* — a rig-animated character gets secondary motion added on top of the base pose, and
  the added motion is rotation equivariant, so the base pose is never rewritten. The reduced
  skinning-eigenmode solver and its generalized eigenvalue problem stay **out** (offline, per rig,
  not needed for a 6-joint flinch). [DERIVED from the abs abstract, read today]
- **Closes**: **AM23** (rotation-additive flinch; the row's own cite is
  `hit_react.rs:261-278` and "per-joint delta quaternion on the base rotation") and the rotation
  half of **AM24**'s last-stage guard. Overlaps **AMX-2** in
  `proposed-rows-03-anim-feel.md:46-74` — that row owns the implementation; folding SAX2 into it is
  the intended merge.
- **Owned paths**: `crates/anim/src/graph.rs` (helpers beside `nlerp`), `crates/anim/src/layers.rs`,
  `crates/anim/src/hit_react.rs`. `crates/game/src/fighter_look.rs` and
  `crates/game/src/view_truth.rs` stay AM23's (AMX-2's owned list).
- **Accept**: `node tools/gh-check.mjs cargo test -p engine-anim --lib hit_react_rot` — prints
  `test result: ok. 5 passed; 0 failed` (AMX-2's four names plus
  `hit_react_rot_equivariant_under_base_yaw`), and
  `node tools/product-check.mjs` prints an `arena view:` line with `react_head_cm_max` at 8 or more
  and `pops` not above the value before the row.
- **Kill line**: weight 0 does not return the base bytes exactly, or the equivariance test passes
  under the wrong order `conj(r) * t` (which means the test is not testing anything), or the golden
  moves.
- **Cost**: S.
- **Verdict**: **PORT-CANDIDATE** (the delta math); the eigenmode subspace is **IDEAS-ONLY**.

## SAX3 — the logical-state visibility barrier (a tick is readable only after its merge)

**What it is.** A tick's rows become observable only after that tick's ordered merge has finished:
the type that carries them cannot be built before the join, so no reader can see half a tick.
This is the job pool's publication rule, delivered as a type, not an atomic and not a lock.

- **arXiv**: https://arxiv.org/abs/2607.02529 — **2607.02529v2**, v1 2026-05-28, v2 2026-07-15
  (`Submission history`: "Thu, 28 May 2026 09:22:46 UTC" and "Wed, 15 Jul 2026 12:40:26 UTC"),
  abs page read **2026-10-21**. Title "RepliCore: Reproducible Parallel Simulation under
  Asynchronous Browser Runtimes", Anqing Chen. [OBSERVED]
- **What it gives us**: the abstract's own separation — "separating asynchronous runtime
  progression from externally observable logical-state visibility", states "exposed only after
  logical-state progression becomes externally stable". Our mapping is one sentence: a tick's rows
  are published after its `collect_indexed_on` join, never during it. The paper's browser and Web
  Worker setting maps to our two extra-reader cases: the renderer and the MCP door. [DERIVED]
- **Closes**: the publication half of **PW20** and **RC36**, and gap 21 in
  `docs/plan/finish-line.md:78` (not board row `E21`, which is retired; see correction 2). Backs
  gap 15 (physics oracle spike) only as a rule: the oracle digest must be computed from a published
  tick, never from a live one. No new row proposed.
- **Owned paths**: `crates/ecs/src/pool.rs` (the type and the method), `crates/arena/src/fork.rs`
  (the one call site, inside the LYR-3 merge). Test names in both files.
- **Accept**: `node tools/gh-check.mjs cargo test -p engine-ecs --lib publish_ordered` — prints
  `test result: ok. 3 passed; 0 failed`; then
  `node tools/gh-check.mjs cargo test -p engine-arena --lib fork_dataset_published` — prints
  `test result: ok. 2 passed; 0 failed`; then `node tools/proof.mjs --skip-build` with
  `E2040_THREADS=1` and with `E2040_THREADS=8` — both print golden `d57b5c8ae3096614`.
- **Kill line**: the barrier needs an atomic, a lock on the read path, or a float accumulator, or
  the 1-worker and 8-worker dataset bytes still differ. Any of those and the row dies: E21 then
  stays serial.
- **Cost**: S.
- **Verdict**: **PORT-CANDIDATE** (idea only; no donor code).

---

# STEP 2 — PATCH-READY SPECS

No code was written. Every hunk below is specified to the file, the symbol, the test name and the
proof line, so a builder with the crate free can land it without re-reading this lane's research.
The golden `d57b5c8ae3096614` is byte-frozen in all three, proved by `node tools/proof.mjs
--skip-build` (seed 2040, 600 ticks; the fixture never runs arena or combat, `HASH-LAW.md:100-101`).

## SPEC A — the rise-motion get-up controller (SAX1)

### Which crate owns it, and why

**It belongs in `crates/anim`, as a new module `crates/anim/src/rise.rs`.** Not the float layer,
not the integer sim.

- **Not the integer sim.** `crates/physics/src/ragdoll.rs` is hashed state (`HASH-LAW.md:109-118`:
  "stagger, knockdown, limb and cut state are integer and hashed; the Rapier ragdoll is
  client-local, seeded from the event, blended by powered drive, never hashed"). A rise controller
  in there is a new hashed field and a new golden risk, and it buys nothing: the authority half of
  a rise is already closed by `BalanceState` — `Fallen` (30 ticks) -> `Catching` (15 ticks) ->
  `getup_due` -> `rise()` -> `stance_legal`. The sim decides *that* a fighter stands and *when*;
  it must not decide *how the body looks while standing*. That is the same split `HASH-LAW.md:109`
  records as decision EG34.
- **Not the float layer alone.** `crates/game/src/ragdoll_look.rs` owns an `Inertial` set and a
  cross-fade (`begin_switch`, `apply_offsets`, `GETUP_TICKS`). A cross-fade can interpolate between
  two poses it is handed; it cannot decide which poses. Putting the phase table there would put
  pose math in the view crate, where the web shell and any future headless capture cannot reach
  it, and it would duplicate the IK that `crates/anim/src/ik.rs` and `feet.rs` already own.
- **Why `crates/anim` is the right owner.** The controller is a pure pose function: integer body
  targets in, one `Pose` out. `anim` already owns every piece it needs — `ik::two_bone_pole`
  (`ik.rs:143`), `ik::solve_limb` (`:347`), `feet::solve_leg` (`feet.rs:246`), `feet::FootLocker`
  (`:155`), `injured::SupportPolygon::com` (`injured.rs:127`) and `support_holds` (`:135`) — and
  `anim` is already a dependency of `game` and `arena`, so no edge is added. It is std-only, so
  native, browser and a headless probe all get the same rise.
- **Where the spec has to wait.** `crates/anim` is held by the AM24 landing agent today, so the
  `rise.rs` half waits for that release. The consumers do not: `crates/game/src/ragdoll_look.rs` is
  free, but it cannot compile against a module that does not exist yet, so the game hunk lands in
  the same commit or after. **No `crates/physics` change is needed**, so the collisions lane does
  not block this row at all — that is the one hold this row escapes.

### The hunk, exactly

**1. `crates/anim/src/lib.rs`** — one line, in the alphabetical block, between
`pub mod readable;` (line 82) and `pub mod space;` (line 83):

```rust
pub mod rise;
```

**2. `crates/anim/src/rise.rs`** (new). Public surface, exactly these names:

```rust
pub const RISE_TICKS: u32 = 60;          // equals powered::GETUP_BUDGET_TICKS
pub const RISE_PHASES: usize = 4;

pub enum RisePhase { Roll, Plant, Push, Stand }

/// Bone indices the controller drives. Callers build it from the rig;
/// ARM_L/ARM_R come from `hit_react`, as `balance.rs:861` already does.
pub struct RiseJoints {
    pub pelvis: usize, pub spine: usize,
    pub arm_l: usize, pub arm_r: usize,
    pub leg_l: crate::feet::LegMap, pub leg_r: crate::feet::LegMap,
}

pub struct RiseTargets {
    pub phase: RisePhase,
    pub com_mm: i32,
    pub hand_l_mm: [i32; 3],
    pub hand_r_mm: [i32; 3],
    pub foot_l_mm: [i32; 3],
    pub foot_r_mm: [i32; 3],
    pub spine_deg: i16,
}

pub struct RisePlan { phases: [RiseTargets; RISE_PHASES] }

impl RisePlan {
    /// The lying pose's own COM, hands and feet fix the plan; no world access.
    pub fn from_lying(lying: &Pose, joints: &RiseJoints) -> Result<Self, RiseError>;
    /// Target for one tick of the rise, `tick` in `0..RISE_TICKS`.
    pub fn targets(&self, tick: u32) -> RiseTargets;
}

/// Solve one tick: feet planted first, then hands, then spine (paper's priority
/// order). Pure: no clock, no allocation, caller-owned `out`.
pub fn solve_rise(base: &Pose, plan: &RisePlan, tick: u32,
                  joints: &RiseJoints, out: &mut Pose) -> Result<(), RiseError>;

pub enum RiseError { LengthMismatch { expected: usize, got: usize }, TickOutOfRange(u32) }
```

Rules inside the module: mm everywhere (`i32`), converted once to meters at the boundary; the
priority order is feet, hands, spine, and the phase boundaries are `Roll 0..12`, `Plant 12..30`,
`Push 30..48`, `Stand 48..60`; every solve goes through the existing IK (`feet::solve_leg`,
`ik::two_bone_pole`), so the module adds no new solver, no `libm` call and no allocation.

**3. `crates/game/src/ragdoll_look.rs`** — the get-up hunk only:
- `ragdoll_stand_up` (`:1120`): keep the `Inertial` machinery, but the switch's destination pose
  comes from `RisePlan::from_lying` + `rise::solve_rise` at the phase boundary the age has reached,
  not from `self.inertial_pose_base(rig, index, &slot, 1.0)`.
- new private helper `fn ragdoll_getup_rebegin(&mut self, index: usize, plan: &RisePlan) -> bool`
  (re-begins the switch toward the next phase target; returns whether the phase changed).
- `ragdoll_getup_offsets` (`:1154`) gains one call: `if self.ragdoll_getup_rebegin(index, &plan)`
  before `apply_offsets`. Re-beginning a fade mid-flight is what `begin_switch` already does for a
  clip change, and it keeps the offset and its velocity continuous.
- The plan is stored next to `getup: Vec<Inertial>` as `getup_plan: [Option<RisePlan>; MAX_LOOK_FIGHTERS]`,
  cleared in `clear()` (`:425`).

### Tests and proof

In `crates/anim/src/rise.rs`, `#[cfg(test)] mod tests`, exactly five names:

1. `rise_from_every_lying_pose_is_legal_within_sixty_ticks` — six lying fixtures (face down, face
   up, both sides, crumpled, half-seated); prints the receipt
   `rise poses=6 ticks_max=58 alloc_calls=0` and asserts no NaN, no joint past its limit, COM above
   the ground at every tick.
2. `rise_plants_feet_before_it_lifts_the_hands` — at `tick = 12` both feet are within 5 mm of their
   plant point and neither hand has left the ground by more than 30 mm.
3. `rise_at_tick_zero_returns_the_base_pose_bits` — a plan built from an already-standing pose
   returns `graph::pose_bits(out) == graph::pose_bits(base)` at `targets(0)`, so the controller is
   an exact no-op when there is nothing to rise from.
4. `rise_twice_is_bit_equal` — `pose_bits` equal on two runs of the same plan and tick.
5. `rise_allocates_nothing_per_tick` — `engine_core::alloc_probe::measure` around `solve_rise`
   returns `0` calls.

Proof:

```
node tools/gh-check.mjs cargo test -p engine-anim --lib rise
  -> test result: ok. 5 passed; 0 failed
node tools/gh-check.mjs cargo test -p engine-game --features app --lib ragdoll_look
  -> test result: ok. 10 passed; 0 failed
node tools/proof.mjs --skip-build
  -> golden d57b5c8ae3096614
```

### Golden

`d57b5c8ae3096614` stays **exactly** where it is: the row adds no hashed field. `PowerRagdoll`'s
`getup_remaining` is client-local (`HASH-LAW.md:109-111`); the integer `BalanceState` windows
(`FALL_PRONE_TICKS`, `CATCH_TICKS`) are not touched; the game crate is not in the golden fixture.
If the golden moves, a hashed struct was changed by mistake: revert, the row is not the cause.

## SPEC B — the additive rotation lane (SAX2)

### The exact change, and how it agrees with the anim lane's spec

The anim lane already wrote this row as **AMX-2** (`proposed-rows-03-anim-feel.md:46-74`, and its
math at `:410-451`). Its math is fixed and this spec does **not** change it:

> `delta = t * conj(r)`, applied as `base * delta`, weighted `s = mask_weight * gain_q8 / 65280`,
> blended with the shortest-arc `graph::nlerp(base_r, base_r * delta, s)`.

SAX2 adds three things to it and contradicts none: the two quaternion helpers get one home in
`graph.rs` beside `nlerp` so both additive paths call the same function; `hit_react` supplies
`r = IDENTITY`, so `delta = t * conj(I) = t` and there is **one** code path with two references
instead of two implementations; and the paper's own property — rotation equivariance — becomes one
test AMX-2 does not have. If the lead prefers one row, SAX2 folds into AM23/AMX-2 and contributes
those three things. It is not a second lane.

**1. `crates/anim/src/graph.rs`** — three `pub(crate)` helpers immediately below `nlerp`
(currently `graph.rs:777`), same style: NaN-safe, identity out, no `unsafe`:

```rust
pub(crate) fn quat_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4];
pub(crate) fn quat_conj(q: [f32; 4]) -> [f32; 4];
/// `t * conj(r)`: target on the left, conjugated reference on the right.
pub(crate) fn quat_delta(target: [f32; 4], reference: [f32; 4]) -> [f32; 4];
```

`quat_delta` negates the result when `w < 0` (short way) and returns identity on a non-finite input.

**2. `crates/anim/src/layers.rs`** — the additive lane stops being position-only.
- `apply_additive` (`:215`): replace line **251** `out.rotations[joint] = base.rotations[joint];`
  with the rotation lane, using the `s` already computed at line 245:

```rust
let base_r = base.rotations[joint];
out.rotations[joint] = nlerp(base_r, quat_mul(base_r, add.rotations[joint]), s);
```

- `compose_stack` (`:281`): in the `LayerMode::Additive` arm, after the position write at line 328,
  add the same line against the running `rot`:

```rust
rot = nlerp(rot, quat_mul(rot, layer.2.rotations[joint]), s);
```

- Both paths use `graph::nlerp` (imported at `layers.rs:14`), never `pose::nlerp`
  (`pose.rs:123`, which differs on the antipodal case) — that is AMX-2's trap at
  `proposed-rows-03:425-427` and it stays.
- Update the two doc comments that say rotations stay on the base (`layers.rs:212-214` and the
  `LayerMode::Additive` comment at `:263`).

**3. `crates/anim/src/hit_react.rs`** — the flinch finally writes rotations.
- `write_additive` (`:261`): replace line **276** `out.rotations[joint] = IDENTITY;` with a
  joint-local delta built from the hit push, reference identity:

```rust
out.rotations[joint] = ik::turn_between(REST_AXIS[joint], bent_axis(dir, TWIST_SHARE[joint], scale));
```

  `REST_AXIS: [[f32; 3]; FIGHTER_JOINTS]` and `TWIST_SHARE: [f32; FIGHTER_JOINTS]` are new const
  tables beside `PLANAR_SHARE` / `DIP_SHARE`; `ik::turn_between` (`ik.rs:205`) already builds the
  shortest-arc quaternion from `dot` and `cross`, so this path adds no `sin`/`cos` and no new math.
  The per-joint share cap is **tuned in the row**, not guessed here: it is set so the AM23
  `react_head_cm_max` line reaches 8 and the `pops` count does not rise.
- `flinch` (`:283`) and `apply_hit_react` (`:298`) keep their signatures; the delta keeps flowing
  through `layers::apply_additive` under `hit_mask` at unit gain, exactly as today.
- Because the reference is identity, `quat_delta(t, IDENTITY) == t`: AMX-2's formula and this hunk
  are the same arithmetic, which is the point.

### Tests and proof

`node tools/gh-check.mjs cargo test -p engine-anim --lib hit_react_rot` — exactly five names:

1. `hit_react_rot_weight_zero_equals_base_bytes` (AMX-2's; `pose_bits` equal at weight 0).
2. `hit_react_rot_two_strikes_sum_within_1e5` (AMX-2's; `|a + b - sum| <= 1e-5`).
3. `hit_react_rot_delta_is_target_times_conjugate_reference` — `quat_delta(t, r)` equals
   `quat_mul(t, quat_conj(r))` for a table of pairs, and the reversed order differs by more than
   `1e-3` on a turn over 20 degrees (the test would catch the wrong order).
4. `hit_react_rot_equivariant_under_base_yaw` — for a global yaw `Q`:
   `quat_mul(quat_mul(Q, base_r), delta) == quat_mul(Q, quat_mul(base_r, delta))` within `1e-5`,
   which holds only because the delta is right-multiplied in the bone's local frame. This is the
   paper's rotation-equivariance property, tested in one line.
5. `hit_react_rot_identity_delta_leaves_the_base_bits` — a zero-impulse `flinch` layered over a
   posed base returns the base `pose_bits` exactly.

Result line to expect: `test result: ok. 5 passed; 0 failed`. Then
`node tools/product-check.mjs` prints an `arena view:` line with `react_head_cm_max >= 8` and
`pops` not above the value before the row.

### Golden

`d57b5c8ae3096614` unchanged. This is a pose layer: `layers.rs` and `hit_react.rs` are presentation,
not hashed state, and `crates/anim` is not in the golden fixture. The proof is the same
`node tools/proof.mjs --skip-build` line. A moved golden means someone touched a hashed struct.

## SPEC C — the RepliCore visibility barrier, landing with the pool (SAX3)

### Why it must land with the pool, not after

LYR-2/LYR-3 move the fork's per-tick rows onto the shared pool. If the rows are built on the pool
first and the publication rule second, there is a window in which a `Dataset` can be assembled from
a half-merged tick, and the "1 worker equals 8 workers" byte proof is then meaningless: it would
pass on the fixture and fail on a slower machine. So the barrier is part of **the same commit** as
the merge, in two places.

### The hunk, exactly

**1. `crates/ecs/src/pool.rs`** — the type and the one constructor:

```rust
/// A tick's logical state, readable only once its ordered merge has finished.
/// The field is private on purpose: `publish_ordered` is the only constructor,
/// so no reader can hold a partially written tick. No atomic, no lock on read.
pub struct Published<T> { items: Vec<T> }

impl<T> Published<T> {
    pub fn get(&self) -> &[T];
    pub fn into_inner(self) -> Vec<T>;
}

impl JobPool {
    pub fn publish_ordered<T, F>(&self, range: Range<usize>, chunk: ChunkSize, f: F)
        -> Result<Published<T>, JobsError>
    where T: Send, F: Fn(usize) -> T + Sync;
}
```

`publish_ordered` is `collect_indexed_on(self, range, chunk, |i| f(i))` (`parallel.rs:152`) and
then `Published { items }`. That is the whole barrier: publication happens on the caller thread
after the scope joins, so progression and visibility are separate by construction — the paper's
separation, in twenty lines of std. Doc comment must say: "the pool decides where a job runs, never
which slot it fills or when it becomes visible."

**2. `crates/arena/src/fork.rs`** — the one call site, inside LYR-3's merge in
`ForkJob::run_recorded_inner` (`fork.rs:593`). Per tick, build the fighters' rows as one published
unit and append only the published slice:

```rust
fn merge_rows_in_fighter_order(pool: &JobPool, fighters: usize, tick: u32,
                               f: impl Fn(usize) -> Row + Sync) -> Result<Dataset, JobsError>;
```

- `merge_rows_in_fighter_order` calls `pool.publish_ordered(0..fighters, ChunkSize::new(1), f)`,
  then `rows.extend_from_slice(published.get())`. Tick-major, fighter-minor, which is the order
  `run_recorded`'s own test asserts today (`fork.rs:1364`, `rows.len() == 120 * 2`).
- The `Dataset` is built from those appended slices only; `Dataset::empty()` (`recorder.rs:374`)
  stays for the crash path. No field of `Dataset` (`recorder.rs:182`) changes, so `DATASET_VERSION`
  stays 1.
- `run_trace` / `ForkTrace` (`fork.rs:470`, `:206`) needs no change: a trace tick is written once
  per tick on the worker, so it is already published per tick. Say so in the doc comment rather
  than adding a type there.
- Islands need no change: `step_islands` (`islands.rs:189`) merges by chunk index through
  `merge_sorted` (`:146`) before it returns, so it publishes by construction.

### Tests and proof

`crates/ecs/src/pool.rs`, `#[cfg(test)] mod tests`, exactly three names:

1. `publish_ordered_is_invisible_until_the_join` — the job closure records how many slots it has
   written; the first read through `Published::get` sees `range.len()`, never fewer.
2. `publish_ordered_matches_serial_for_every_worker_count` — pools of 0, 1, 2, 4 and 8 workers give
   the same `Vec` for the same `f`.
3. `publish_ordered_returns_no_partial_on_panic` — a panicking job re-raises on the caller and
   nothing is published.

`crates/arena/src/fork.rs`, exactly two names:

4. `fork_dataset_published_per_tick_in_fighter_order` — `rows.len() == ticks * fighters` and row
   `i` is `(tick = i / fighters, fighter = i % fighters)`.
5. `fork_dataset_bytes_equal_at_one_and_eight_workers` — same dataset bytes with
   `E2040_THREADS=1` and `=8`.

Proof:

```
node tools/gh-check.mjs cargo test -p engine-ecs --lib publish_ordered
  -> test result: ok. 3 passed; 0 failed
node tools/gh-check.mjs cargo test -p engine-arena --lib fork_dataset_published
  -> test result: ok. 2 passed; 0 failed
node tools/proof.mjs --skip-build          (E2040_THREADS=1, then =8)
  -> golden d57b5c8ae3096614 both times
```

### Golden

`d57b5c8ae3096614` at **both** worker counts. The barrier adds no hashed state: it only changes when
a value becomes readable, never what the value is. The kill line is sharp here — if the 1-worker and
8-worker dataset bytes differ, the row dies and E21 stays serial.

---

# STEP 3 — HANDOFFS

## For the anim lane

Two specs, both blocked on your AM24 release of `crates/anim`:

- **SAX1 (rise-motion get-up controller, closes RZ2, feeds BX5)** — SPEC A above. New module
  `crates/anim/src/rise.rs` with `RisePlan`, `RiseTargets`, `solve_rise`, `RISE_TICKS = 60`;
  five tests under `--lib rise`; consumer hunk in `crates/game/src/ragdoll_look.rs`
  (`ragdoll_stand_up`, `ragdoll_getup_offsets`, new `ragdoll_getup_rebegin`). No `crates/physics`
  change, so the collisions lane does not block it. Take it after AM24 lands so the limiter and the
  rise do not fight over `fighter_look`'s tail.
- **SAX2 (additive rotation lane, closes AM23 and the rotation half of AM24)** — SPEC B above.
  **Do not write a second lane**: AMX-2 already owns the math at `proposed-rows-03:410-451`. Its
  additions are `graph::{quat_mul, quat_conj, quat_delta}` beside `nlerp` (`graph.rs:777`), the
  rotation line in `apply_additive` (`layers.rs:251`) and `compose_stack` (`:328`), the delta write
  in `write_additive` (`hit_react.rs:276`) with `r = IDENTITY`, and the one extra test
  `hit_react_rot_equivariant_under_base_yaw`. Fold it into AMX-2 rather than queueing a twin row.

## For the layers lane

**SAX3 (RepliCore visibility barrier) lands inside your pool work, in the same commit as the
merge** — SPEC C above. Exact names:

- `crates/ecs/src/pool.rs`: `pub struct Published<T>` (private `items`, `get`, `into_inner`) and
  `JobPool::publish_ordered(range, chunk, f) -> Result<Published<T>, JobsError>`, built on the
  existing `collect_indexed_on` (`parallel.rs:152`). No atomic, no float, no lock on the read path.
- `crates/arena/src/fork.rs`: `ForkJob::run_recorded_inner` (`:593`) builds each tick through
  `merge_rows_in_fighter_order(&JobPool::shared(), fighters, tick, f)` and appends only
  `published.get()`; `Dataset` and `DATASET_VERSION` are untouched.
- Tests: `publish_ordered_is_invisible_until_the_join`,
  `publish_ordered_matches_serial_for_every_worker_count`,
  `publish_ordered_returns_no_partial_on_panic`, `fork_dataset_published_per_tick_in_fighter_order`,
  `fork_dataset_bytes_equal_at_one_and_eight_workers`.
- Proof: `--lib publish_ordered` 3 green, `--lib fork_dataset_published` 2 green,
  `node tools/proof.mjs --skip-build` with `E2040_THREADS=1` and `=8` both print
  `d57b5c8ae3096614`.

Two facts from this lane that save you time: `JobPool::shared()` is already public
(`pool.rs:336`) and `workers_from_env` is public (`:353`), so LYR-2's visibility half is done; and
`step_islands`/`merge_sorted`/`island_chunks` are back in `crates/physics/src/islands.rs:133-203`,
so the island side needs no barrier of its own.

## What this file did not do

No board, plan or focus file edited. No `crates/**` or `tools/**` file touched. Nothing staged,
committed or pushed. The lead merges the rows.
