# Animation (Bodies)

Outcome: fighters move with ground feel and LOD at crowd scale, without a second runtime.
Proof: (1) LM2 foot slide under 1 cm from 0.5 to 7 m/s, replay x2 (`cargo test --locked -p engine-anim warp_ground`); (2) AM13 LOD has a production caller in the duel (BLOCKERS: unwired twice); (3) 1,000 agents sim p99 at most 2 ms (VISION speed bar).
Order: LM2, AM13, AM14, then section 7 in table order. No row of this plan is in FOCUS Now (2026-10-10).
Crates: anim, arena, render, game.
Open board IDs: LM2 LM3 AM13 AM14 AM15 AM18 and the table in section 7.

Tag NEXT (Bodies). Prefixes: AM, LM, TV. Plain summary: fighters already sample poses, blend locomotion, match motion, traverse the arena and settle plumes through a runtime graph. The graph, spaces, layers, events, pack, feet, exertion, full-body IK, hit-react and spring-bone core have landed. The open tail is ground feel, networked poses, LOD at crowd scale, parallel eval and one additive spine, all without a second graph.

Merged 2026-10-10 (batch A of the plans review): `docs/archive/2026-10-10/plans/deepdive-LM2-2026-10-08.md` as the LM2 note in section 7 (archived). Cut here: the dated Status lines, the rival check, the donors table and the superseded hard problems, in `docs/archive/2026-10-10/plans/animation-cut.md`.

Pre-merge text: `git show d96091cd:docs/plan/animation.md`; `git show d96091cd:docs/plan/deepdive-LM2-2026-10-08.md`.

## 1. Where it stands

Landed and ticked (kept out of this table; the board and git keep them): AM1
runtime graph, AM2 spaces, AM3 layers, AM4 events, AM6 foot lock, AM8 exertion,
AM9 full-body IK, AM10 hit reactions, AM12 faces, AM17 readable locomotion, LM1
motion matching, LMM1 learned pose nets, TV1 traversal warp, CT3 locomotion bake.
Implemented on disk with no board row: AM5 clip packing, AM7 dual-quaternion
skinning, AM11 springs. Their evidence paths are in git (`pack.rs`, `dq.rs`, `springs.rs`).

| Capability | Status | Evidence (file or commit) |
|---|---|---|
| GASP-class ground feel (LM2) | on disk, live consumer present (rowed) | `crates/anim/src/warp_ground.rs` and `crates/anim/src/warp_ground_feel_test.rs` both exist (Test-Path True) and the live consumer is `use engine_anim::warp_ground::GroundWarp;` at `crates/arena/src/lib.rs:109`; the old "does not exist yet" cell was false (corrected 2026-10-21; see the Status lines at `:33`/`:34`) |
| Networked motion (LM3) | missing (rowed) | `crates/anim/src/net_pose.rs` present on disk; needs LM1, NET2 |
| Animation LOD (AM13) | missing (rowed) | `crates/anim/src/lod.rs` present on disk; needs AM1, LM1 |
| Parallel SIMD eval (AM14) | present, 0 live callers (rowed) | `crates/anim/src/par.rs` on disk; equality 0/1/2/4/8 workers bit-identical; 2.0x on 64 fighters, the 3x bar is open and no game caller; needs AM1, RC3 |
| Additive layers spine (AM15) | present, 0 live callers (rowed) | `crates/anim/src/additive.rs` on disk with 7 tests and a per-tick budget; `fighter_look.rs:2078-2085` still uses a pelvis-only flinch, so `AdditiveRig::tick` has no game caller |
| Thousand-agent crowd pose budget (AM18) | missing (rowed) | needs AM13 |

- Status 2026-10-12: LM2 row above says `crates/anim/src/warp_ground.rs` does not exist yet, but `crates/anim/src/warp_ground.rs` exists on disk, so that cell is false.
- Status 2026-10-17: disk shows the false LM2 cell still at `docs/plan/animation.md:26` ("`crates/anim/src/warp_ground.rs` does not exist yet; needs LM1, AM6 (ticked)") with the 2026-10-12 Status at `:33` already marking it false, and the same "does not exist" claim a second time in this file's LM2 note (`:220` before this line was added, now `:221`), while the code side holds: `crates/anim/src/warp_ground.rs` on disk (Test-Path True, Length 27849) plus `crates/anim/src/warp_ground_feel_test.rs` (Test-Path True), the live consumer `use engine_anim::warp_ground::GroundWarp;` at `crates/arena/src/lib.rs:102` with the warp vector at `:1047` and `:1196`, and this file's `:145` recording it "landed with no window caller", so the S7 gap still holds and only the two "does not exist" lines stay false.
- Status 2026-10-19: Dunsky, Mastering C++ Game Animation Programming (2025), confirms the two code-side gaps: `par.rs` and `additive.rs` exist with 0 live callers, so the "no file" cells at :29-30 are false. Reader: `work/nb-gaps/nb-gaps-anim-dunsky.txt`; ledger: `BOOK-GAPS-3-2026-10-19.md`.

Open tail, in one line: starts, stops and pivots must match distance; remote
fighters must pose the same after rollback; crowds cheapen with distance; eval
goes parallel; every interruption blends through one inertializer. No second
graph, warp, search or leg solver. `ik.rs` and `inertialize.rs` are helpers, never
a second AM6 or AM2. No mocap bytes in git.

Module notes that constrain the rows below: `crates/anim` is std-only and never
imports assets, arena or render; float pose math is presentation, and tables
that cross into the sim are quantized per HASH-LAW; bakes stay offline; async
work rejoins at tick boundaries with recorded inputs, so a late result waits
instead of teleporting a fighter.

## 2. Finish line

Best existing implementations (the bar each row must meet or beat, measured): `ozz-animation` SoA sampling with graph-as-data (AM1/AM6 lineage); `nfrechette/acl` quantized packing with charted per-clip error (AM5); Holden-style motion matching with a brute-force equality receipt (LM1). We win on determinism and lose on content breadth (no mocap base, no learned controller; PX5 stays Tier C behind its A/B gate). The rival check (Unreal and Unity docs, D-SKEL-1) is in the cut file: docs only, never a runtime proof.

Tier A alpha (fighters move like fighters, audited, at scale): AM1–AM4 plus AM6 stand,
AM9–AM11 stand, LM2 ground feel and LM3 networked poses ticked, plus
AM16–AM19/LM4–LM6/TV2–TV3 and the r7 follow-ups AM20–AM22/LM7. Bar: P2 foot
slide under 1 cm from 0.5 to 7 m/s and under 2 cm on straights, held on
20-degree slopes and stairs (LM6); strafes to 135 degrees without skating; two
clients at 150 ms RTT show the same pose checkpoints after resim, and a
seeded N-client soak names its first divergence (LM5); every eval
path proves 0 per-tick heap allocs including springs (AM19); replay exact x2 everywhere.
Scale edge (P6): 64 fighters under 2 ms total with the 8 nearest matching
full-rate poses (AM13), extended to 1,000 agents at p99 ≤ 2 ms on the
reference CPU with the far crowd handed to VG4 impostors by contract (AM18);
the anim slice of the P6 server tick (p99 ≤ 4 ms at 64 players plus 500 AI) is
posted per LOD tier, never averaged away. Laptop edge: this dev box's RTX 3050
6 GB is the only GPU bar — AM7/AM12/VG4 rows post adapter plus driver plus
power-mode lines from a pilot release window build, never the operator's box.

Tier B v1 (motion): AM5 plus AM8 stand, AM7 option, AM13 LOD, AM15 additive spine, AM17 readable locomotion, TV3 siege traversal, plus TV4
rubble variance and AM17-style budget follow-ups. Bar: no
pops on any interruption; foot slide at most 20 mm straight and 40 mm on
pivots; springs never blow up over a soak at 0.1 ms for 8 fighters; candy
wrappers clean with the R3 path unchanged by default; ladder/rampart/rubble
edges land within 1 cm on the siege fixtures, rubble holds across height/noise
sweeps (TV4).

Tier C (only on demand): AM12 faces, AM14 parallel eval unless the win is
measured, PX5 learned controller effects, AM23 bake-drift gate first. Bar:
announcer and taunt lines lip-sync on blind review with morph cost posted;
at least 3x on 4 cores for 64 fighters with identical poses under every
chunking, else killed; learned paths need the LMM1 shortfall proof plus A/B
quality evidence first.

2040-level version: 20k reacting spectators (C3) with the near ring fully
posed, the mid ring at LOD tiers and the far ring as VG4 impostors, one graph
with no per-tier fork; a 64-slot siege replays exactly for 10 minutes (C5)
with ladders, ramparts and rubble as first-class edges; remote crowds pose
identically after rollback at 150 ms RTT and 2% loss with zero desyncs in an
hour (P4, LM5 diagnostics grown into the soak gate); faces carry the announcer
and taunts live (LAI3) while the sim stays integer-exact; the P6 server holds
64 players plus 500 AI with the anim slice posted per tier. Nothing in Tier A/B
blocks this: every row keeps the flat order, the quantized sim boundary and
the LOD contract the 2040 crowd needs.

CUT (never): own-video motion capture, non-commercial mocap data in git,
runtime float-weight nets in the tick, a behavior-tree visual editor, a
training harness (DT owns offline nets), runtime TTS nets.

## 3. Target design

Data flow (one direction, no cycles):

```
clips (CT1 cook, AM5 packed) -> graph eval (AM1 flat order, SoA poses)
  -> spaces (AM2, inertialized) -> layers (AM3 + AM15 additives)
  -> feet (AM6 lock) -> full-body IK (AM9) -> hit-react (AM10 additive)
  -> events (AM4 sorted track) -> poses out: root tables to arena,
     presentation poses to R3, cues to combat/audio, picks via LM1
```

Ownership: `crates/anim` owns skeletons, poses, graphs, spaces, layers,
events, packing, feet, exertion, IK, reactions and (new) ground-warp,
net-pose, springs, faces, LOD, parallel and additive spines. `assets`
imports in; `arena`/`render`/`audio`/`combat` consume out; `net` resims
LM3 indices; `nav` feeds NV2 links into TV1; `perf` records every budget.
AM rows deepen `crates/anim` only, never mirror into new crates.

Threads: graph eval runs on the sim thread with a presentation copy for
render; LM1 search keeps its per-character receipt; AM14 parallelizes
per-character eval over RC3 fork-join only (never the sim tick order, never
Rapier parallel). All bakes run offline.

Determinism: fixed eval order compiled once; quantized tables across the
sim boundary; BTree/index orders only; no `HashMap` iteration reaches
hashed state; contacts and picks break ties by stable IDs. Same seed plus
same tape gives the same poses, events and picks — asserted x2 in every row.

Budgets (kept green forever): full-graph eval at most 0.35 ms per fighter;
LM1 search cost per character re-posted after AM2/AM8/LM4 (no regression vs
the LM1 receipt); packed memory per fighter at most 2 MB; foot slide bars
above; event firing frame-exact (off-by-one fails the row); AM13 crowd bar
(64 under 2 ms) and AM14 parallel bar (3x or killed) posted on the
reference CPU before they tick; AM18 thousand-agent bar (1,000 at p99
≤ 2 ms, P6) and the anim slice of the P6 server tick (64 players plus
500 AI) posted per LOD tier, never averaged away.

API sketch (existing, rows deepen — never mirror):

```rust
pub struct AnimGraph { pub nodes: Vec<Node>, pub order: Vec<NodeId> }
pub fn eval_graph(g: &AnimGraph, dt_ms: i64, out: &mut PoseSoA) -> Events;
pub fn solve_leg(hip: Vec3, knee: f32, ankle: Vec3) -> LegSolve; // AM6, LM2/AM9 consume
pub fn search(db: &MotionDb, q: &Query) -> (MatchResult, SearchReceipt); // LM1, LM4 locks cost
```

**Steals 2026-10-12** (board shard `../SPRINT-21-look-feel.md`, scout files `../research/steals-2026-10-12/`). Window pose stages, view only: clip sample and fade -> inertialize (deepen `inertialize.rs`; there is no `inertial.rs`) -> rotation-additive flinch (AM23) -> look-at with soft limits (AM25) -> hand and foot IK (`ik.rs` exists; LF12 wires it) -> spring chains -> angular-speed limiter (AM24) -> pelvis pin. Each stage writes slerp(current, wanted, influence) (Godot modifier pattern). Stage state advances in observe_tick at 60 Hz and is evaluated at alpha, so slot_pose stays a pure function of tick and alpha.
- Fix: `crates/anim/src/warp_ground.rs` exists and the arena uses GroundWarp (`crates/arena/src/lib.rs:98`). The live view now uses inertial (`fighter_look_inertia.rs:20`), ik (`fighter_look_feet.rs:24`), throw_layer (`fighter_look_throw.rs:18`) and hit_react (`fighter_look.rs:24`); additive, fbik, springs and lod still have no live caller.

## 4. Hard problems

Kept: the four problems that open rows need (feet, remote poses, crowd LOD, thousand agents). The rest (inertialization, small clips, siege walls, strobing, silent divergence) are in the cut file.

2. Feet that plant at speed and on pivots. Problem: capsule speed outruns
   clip speed and strafes skate. Technique: distance matching for
   starts/stops/pivots, orientation warping for strafes, stride warping so
   feet match capsule speed (Paragon GDC 2016 shapes, reimplemented) over
   the AM6 analytic lock plus slide meter. Why: analytic solves once,
   warps keep contact without hiding error. Reference: LM2 acceptance
   (1 cm bar, 135-degree strafes, replay x2). Donor: none (TV1 warp owns
   edges; graph never reimplements it).
3. Remote fighters that pose the same after rollback. Problem: resim must
   reproduce poses bit-for-bit under packet loss and jitter. Technique:
   replicate pick index plus phase and re-evaluate deterministically;
   record per-hit lag-compensation windows. Why: indices plus phase are
   tiny and exact; poses are derived, never sent. Reference: LM3
   acceptance (150 ms RTT checkpoints, kbps charted). Donor: none (net
   owns transport; anim owns determinism).
5. Crowds that stay cheap without popping. Problem: 64 fighters at full
rate cost too much; far IK and springs waste cycles. Technique:
update-rate tiers by distance and screen size, IK/springs off far away,
crowd handed to VG4 impostors. Why: nearest-8 equality proves the tiers
are invisible where it matters. Reference: AM13 acceptance (2 ms bar,
nearest-8 match). Donor: none (own LOD code; VG4 owns impostors).
6. A thousand agents inside 2 ms. Problem: AM13 stops at 64 fighters;
P6 demands a 1,000-agent sim at p99 ≤ 2 ms and a server tick at 64
players plus 500 AI, and nobody has posted the anim slice. First
approach: tier the eval the AM13 way (full rate near, strobed mid,
impostor far by VG4 contract), then measure with a seeded crowd tape on
the reference CPU and post per-tier costs; if strobing breaks nearest-8
equality or blows the p99, kill strobing and keep distance tiers only.
Reference: AM18 acceptance (p99 bar, per-tier post, golden unchanged).
Donor: none (own tiers; ozz SoA discipline as the cache pattern).

**Steals 2026-10-12, hard problems.** Hit react must outlast the sim window: the sim lock is 6 ticks (100 ms), a readable flinch is about 450 ms, so the view envelope has its own in, hold and out and overlapping hits add (AM23; proof react_head_cm_max and pops in `arena view:`). Telegraph: the sim gives the attack age; markers in the clip event track map age to clip time (GP44, telegraph_hand_cm_min); heavy rows, not the warp, carry reaction time.

## 5. Donors

Donors: `steals.md`; porting policy is AGENTS rule 4. The donor table is in `docs/archive/2026-10-10/plans/animation-cut.md`.

## 6. Slim

- Delete: the legacy float `match_db.rs` once LM1 plus LM4 cover its
  queries with numbers; `ik.rs` helpers graduate into AM6/AM9 or go; any
  second warp, second search, or second SH/post/particle path stays out.
- Merge: AM15 owns locomotion base plus combat upper plus flinch additives
  through one inertializer in `additive.rs` — never per-concern blend
  crates. LM4 owns search cost plus schema freeze in the LM1 module — never
  a cost crate. TV2 owns a contract test, never a second link registry.
  LM6 deepens `warp_ground.rs` (slopes, stairs, sand) — never a second
  warp. LM5 owns diagnostics hunks beside `net_pose.rs` — never a second
  resim. TV3 deepens `traversal.rs` — never a second chooser.
- Freeze: flat eval order, `PoseSoA` shape, event-track sort, mask-table
  format, pack wire version, the slide bars, the LM1 feature schema once
  LM4 lands, the AM13 LOD tier contract once AM18 proves it. Moving any goalpost invalidates every receipt on file.
- What we will not build: a mocap rig, a BT editor, a training harness, a
  float-weight runtime net, runtime TTS, a cloth GPU path (physics owns
  CPU verlet or nothing), `parry3d` or any sim-touching second engine.

## 7. Board tasks

Rows from the steals (2026-10-12, board shard `../SPRINT-21-look-feel.md`, scout files `../research/steals-2026-10-12/`): AM23 flinch, AM24 angular limiter, AM25 soft look-at. Row changes: LF13's hard 70/30 clamp is replaced by AM25; LF9 deepens `inertialize.rs` and LF12 wires the existing `ik.rs` (both rows say "new file", the files exist).

Rows live on the board; slice any row with `node tools/board-slice.mjs <ID>`. Tier A rows first (LM2 with its note, then AM13 and AM14), Accept lines verbatim. The rest of the open AM, LM and TV rows follow in a table.

- [ ] LM2 GASP-class ground feel. Tier A. needs: -
  Accept: foot slide under 1 cm from 0.5 to 7 m/s; strafes to 135 degrees without skating; replay exact x2.
- [ ] AM13 animation LOD. Tier A. needs: -
  Accept: 64 fighters under 2 ms total on the reference CPU; the 8 nearest match full-rate poses.
- [ ] AM14 parallel SIMD eval. Tier A. needs: -
  Accept: at least 3x on 4 cores for 64 fighters; identical poses under every chunking.

LM2 note (deep-dive 2026-10-08): `warp_ground.rs` does not exist; the warp helpers live in `traversal.rs` (edge warp), `feet.rs` (foot lock) and `locomotion.rs`. Make the home file first: a thin warp over what exists (distance match for starts, stops and pivots; orientation warp for strafes; stride warp so feet match capsule speed), integer math like `traversal.rs`. One seeded test: foot slide under 1 cm from 0.5 to 7 m/s, replay exact x2. Writer unblocked when the file exists and the test is green; then the 135-degree strafe skate check is added.

| ID | Title | Tier | Needs |
|---|---|---|---|
| LM3 | networked motion | A | - |
| AM15 | additive layers | B | - |
| AM18 | thousand-agent crowd pose budget | A | AM13 |
| LM5 | net-pose divergence diagnostics | A | LM3 |
| LM6 | slope/stair/sand ground feel | A | LM2 |
| TV3 | siege traversal set | B | TV2 |
| AM21 | LOD strobing vs warp contract | A | AM13 |
| LM7 | motion-search fatigue-soak lock | A | LM4 |
| AM22 | crowd readability at LOD | A | AM18 |
| TV4 | breach-rubble scramble variance | B | TV3 TV2 |
| LM4 | motion-search cost + schema lock | A | - |
| TV2 | traversal-to-nav link contract proof | A | - |

## 8. Quality gates

- `cargo test -p engine-anim` green; AM1–AM6 plus AM8–AM11 suites stay green
  under every row (replay x2, permutation proofs, mirror proofs).
- Determinism: same tape twice identical; blend-space interruption equals
  pre-interrupt pose then settles; masked eval within tolerance of the
  full-body reference; events fire on the exact frame twice; insertion
  order never reaches a hash (workspace-check).
- Budgets: graph eval at most 0.35 ms per fighter; packed memory per
  fighter at most 2 MB; slide/pops/decode/search-cost bars charted raw vs
  corrected and kept green forever once landed; PF1 workloads extended,
  never forked; AM18 thousand-agent p99, LM5 divergence-naming, AM20 order,
  AM21 warp-under-strobe, LM7 soak, AM22 LOD-readability, TV4 rubble and
  AM23 bake-drift gates kept green forever once landed.
- Reuse pointer: the counting-allocator floors plus buffer-reuse entries
  (`../library/index.md`, proven by `engine-anim` tests) back the AM16
  and AM19 zero-alloc proofs — cite them, never re-describe them.
- Safety: no platform transcendentals in eval/search/warp code; no `Unsafe`
  in new modules; typed refusals, never panics, on bad caller input.
- License gate: every pattern port carries `PORT()` plus an `EVIDENCE.md`
  row with the verified SPDX URL above; anything UNVERIFIED stays a
  paper-shaped reimplementation with no donor code.

## 9. Session estimate

Moved to git history: `git show d96091cd:docs/plan/animation.md` (section 9). Each row carries its own estimate on the board.

