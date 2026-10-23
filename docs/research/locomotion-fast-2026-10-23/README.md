# Fast physics locomotion, wave 2026-10-23

Scope: PX5 (learned physics-driven locomotion) and PX5a (the procedural gait),
and the owner's question: how do we get a physics-driven walk in **minutes**, not
weeks? Every label below follows from a number that was read at the pinned SHA.
Nothing here is our result: no net was run on this PC (no GPU, and the tick binds
no net yet).

Method: each repo was cloned blob-less at its default branch and read with
`git show <sha>:<path>` (or the raw file at the same SHA). Licence files were
read at that SHA, not taken from memory. Sizes come from `git ls-tree -l`.
The G1 ONNX graph was parsed byte by byte (a small protobuf reader) to get the
layer shapes and the op list.

Number labels used everywhere: **measured** = read or parsed from a file at the
pinned SHA; **donor claim** = the donor's own sentence, no number of ours;
**our target** = the bar a row must meet; **arithmetic** = plain multiplication
of measured numbers, not a benchmark.

## The one-line answer

Only PX5a is a *minutes-to-hours* route, because it trains nothing. Every
"run a shipped policy today" route is minutes to **run** and days to
**retarget** (the shipped nets are 29-DoF robot commands, not our 15-bone hero).
Every "train it ourselves" route is weeks, because our tree has no vectorised
env (stated in `docs/plan/hard-systems.md`: "Agent gym (vectorized headless envs
for RL) | missing") and the donors train at 100,000,000 timesteps over 8,192
envs (measured in `mujoco_playground/config/locomotion_params.py`).

## The candidates

| # | what | licence | pinned rev | weights in the repo? | cost label | fits the integer tick? |
|---|---|---|---|---|---|---|
| 1 | MuJoCo Playground **G1 joystick** ONNX | Apache-2.0 (`LICENSE`) | `12ad7f3842791ced39f27e5ac96d066bbe361653` | **yes**: `experimental/sim2sim/onnx/g1_policy.onnx`, 903,543 B (measured) | minutes to run, **days** to retarget | yes: 4 dense + swish, 225,160 params, 103 obs, 29 act (measured) |
| 2 | Playground **T1** 860,343 B / **Apollo** 925,143 B / **Berkeley Humanoid** 781,140 B ONNX | Apache-2.0 | same | **yes**, four more files under `experimental/sim2sim/onnx/` (measured) | same as row 1 | same shape class (not parsed one by one) |
| 3 | unitree_rl_gym **G1** `motion.pt` | BSD-3-Clause (`LICENSE`) | `276801e46c5d433564f24658bac64f254b7d2d4b` | **yes**: `deploy/pre_train/g1/motion.pt`, 145,745 B (measured) | minutes to run, **days** to retarget | yes: 47 obs, 12 leg actions, 50 Hz, PD kp/kd given (measured) |
| 4 | **DeepMimic** `humanoid3d_*` 18 skills (walk, run, getup_faceup …) | MIT code (`LICENSE`); motions split, `data/motions/sie/` is CC-BY-NC-4.0 | `1f915c52fcd4b95b5f5f15b759ae91bd81e9a801` | **yes**: `data/policies/humanoid3d/*.ckpt` (measured) | **days**: README says the codebase is deprecated; needs Bullet 2.88, Eigen, Swig, TF1 | no: TF checkpoint, obs/act shaped for their 34-DoF body |
| 5 | **MimicKit** (DeepMimic/AMP successor) | Apache-2.0 | `fbe1a36e878f036d6bf5521defb8eb3b8ab4170d` | **no**: `data/models/.gitignore` and `data/logs/.gitignore` exist, so the README's `data/models/*.pt` path ships empty (measured) | **weeks** | yes, once trained |
| 6 | **IsaacGymEnvs** `HumanoidAMP` + 6 humanoid motion clips | BSD-3-Clause (`LICENSE.txt`); clips from CMU mocap | `aeed298638a1f7b5421b38f5f3cc2d1079b6d9c3` | clips **yes** (`assets/amp/motions/amp_humanoid_walk.npy` …), weights **no** | **days** (needs Isaac Gym Preview; 30 Hz control) | clips are not a net; only useful as a teacher *after* retarget |
| 7 | **IsaacLab** humanoid / velocity tasks | BSD-3-Clause | `b03345bbf2c9a54eb03f9bd823ae19c0ca3a014e` | **no** `.pt`/`.onnx`, and no `amp/` task dir in this tree (measured) | **weeks** (GPU + Isaac Sim) | only after our own training |
| 8 | **brax** `humanoid` | Apache-2.0 | `c8605294264a7b7546268adbb4beabc9eb624000` | **no** | donor claim: "train agents in seconds to minutes", one accelerator; **weeks** on our CPU | yes: 17 torque actions, obs 376, `dt` 0.015 (measured) |
| 9 | Hugging Face `sb3/sac-Humanoid-v3` | **no licence on the model card** (measured: no `license` field) | card, 2026-10-23 | **yes**: `sac-Humanoid-v3.zip`, `policy.pth` | minutes to run (`rl_zoo3.load_from_hub`) | yes, **but** an unlicensed net cannot ship |
| 10 | Procedural CPG: our `gait.rs` + the playground foot-lift formula | Apache-2.0 (formula) + ours | `12ad7f38` (formula) | n/a | **hours** | yes: already integer phase (Q15 table) |

Two donor facts that make row 1 usable: the ONNX runs on
`onnxruntime` with `CPUExecutionProvider` (`play_g1_joystick.py`: `ctrl_dt`
0.02 s = 50 Hz, `sim_dt` 0.002, `action_scale` 0.5, 1.5 Hz phase clock), and the
graph is `Gemm, (Sigmoid, Mul)` ×4 then `Split, Tanh` — SiLU, then a tanh action
squash (measured from the op list). Caveat worth knowing before trusting it: the
driver in the same commit builds `14 + 3 × 29 = 101` lanes while the graph wants **103**
(measured), so one of the two files is stale.

## Ranked: the three I would keep

### 1. PX5a: the procedural gait drives the PD servo (hours, no training)

Home: `crates/physics/src/gait.rs` (already on disk, PX5a owns it) →
`PoweredRagdoll::set_animated` in `crates/physics/src/powered.rs`; bias comes
from `crates/physics/src/balance.rs` (`servo_bias_targets`, `SUPPORT_HALF_MM`
150 mm, `CP_MAX_ERR_MM` 500 mm, `SERVO_BIAS_MAX_Q8` 128).

- The exact trick, not the vibe: a phase clock advances at a cadence that carries
  the body (`CYCLE_TICKS_AT_3MPS` 24 ticks, `REF_SPEED_MM_S` 3000); hip pitch and
  knee flexion are read from the clock; the swing foot's height is the playground
  formula `y0 + (y1-y0)(x³ + 3x²(1-x))`, stance half first, swing half back
  (`mujoco_playground/_src/gait.py::get_rz`, read at `12ad7f38`). Feet do not
  slide because the pose target of a *planted* foot is held at the world point
  recorded at touchdown, not at the pelvis-relative rest pose — so as the pelvis
  travels, the stance foot's local offset moves backwards by the same amount.
- Integration cost that is easy to miss: `LocalPose` positions are
  **parent-relative** and `write_targets` rotates them by the parent's world
  rotation, so `gait.rs` must carry its own small leg FK to turn a world foot
  target into a parent-local offset (the ragdoll exposes `bone_position` but no
  per-bone world-target setter). That is the real work in this row.
- Bounded reproduction, one test: `cargo test -p engine-physics --lib gait` — 3 m
  in 120 ticks (1.5 m/s, our target) with the pelvis inside 150 mm of the line
  and 60 mm of its start height, a stop inside 30 ticks, and the stance foot's
  slip printed in mm. Replay twice, same hash.
- Kill: stance foot slides more than **100 mm** at 3 m/s, or the body glides
  while the shove at `DEFAULT_STAGGER_Q8` 256 does not produce a step.
- The number it moves: foot slip from "not measured" to ≤100 mm, and travel from
  0 to 3 m in 2 s. Nothing else in the tree walks today.

### 2. Distil that gait into the integer net (hours; the CPG is the teacher)

Home: `crates/distill/src/mlp.rs` (two-layer ReLU, hidden ≤256, Adam, softmax
cross-entropy) + `export_mlp` → `.e2nn`. The blocker is **not** the network: the
loss is a class loss. Continuous joint targets need one regression loss
(quantised MSE) plus a target column in the dataset. `crates/nn` already reads
16 layers of dense/GRU with ReLU/tanh/sigmoid/LUT and 4,194,304 params max, and
`quantize_lane` gives ±127 with round-half-even.

- Bounded reproduction, one test: record 600 tick-rows of the PX5a teacher
  (obs → 12 leg targets), fit, export, then assert one `.e2nn` forward is within
  **2 LSB** of the teacher per lane and the student body still stands for 120
  ticks in the same fixture. Arithmetic size at obs 34 → 64 → 32 → 12: 4,608
  params = 4.6 KB as i8, 4,608 MAC per tick (arithmetic, not measured).
- Kill: the student falls inside 120 ticks, or needs more than 60k params — at
  that point the CPG is cheaper than the net and PX5 has no reason to ship.
- The number it moves: tick cost (4,608 MAC/tick against 225,160 for the donor
  G1 net, measured) and the "no continuous-target path in the trainer" line in
  `docs/plan/hard-systems.md`.

### 3. The playground G1 ONNX as a *reference generator* (days, not the controller)

Home: an offline tool (not the tick) that records `obs → action` rows and feeds
`crates/assets/src/retarget.rs` (bind-relative rotation delta, bone-length ratio,
BoneMap coverage). Do **not** try to make a 29-DoF robot gait the arena walk: it
commands root velocity and 29 joint targets at 50 Hz, and our tick is 60 Hz
(`crates/core/src/tick.rs`: `TICK_HZ` 60). Re-time by holding an action for one
tick and duplicating one in six; the 1.5 Hz phase input makes that legal.

- Bounded reproduction, one test: 1,000 recorded rows through the quantised net,
  assert max per-lane error ≤2 LSB of the float32 graph; then one 600-tick MuJoCo
  standing run in the donor's own scene (not our body) to prove the recording is
  a walk.
- Kill: any lane over 2 LSB after QAT (`crates/nn/src/qat.rs`), or the 50→60 Hz
  retime changing foot height by more than 50% of swing height.
- The number it moves: gives the retarget ladder a real reference today instead
  of a blank; 225,160 params, 903,543 B, 103 lanes in, 29 out (measured).

## What I would do this week

Take route 1, and start the 600-row recording for route 2 as soon as the gait
passes its test. It is the only route where the first walking body is hours away
and every number is ours.

I would refuse "train AMP/DeepMimic/brax from scratch". The reason is a number,
not taste: the donors' own config is 100,000,000 timesteps at 8,192 envs, our
tree has no vectorised env, and the physics we would have to drive is f32 Rapier
inside the tick, not MJX on a GPU. That is weeks before the first step, and it
would still end as a distillation job — the same job route 2 does in hours.

I would also refuse the unlicensed HF checkpoints (row 9) and the CC-BY-NC SIE
motions (row 4), because the PX5 row already says clips from non-commercial
training data never train a shipped student.

## Still unverified

- No CPU inference time is stated by any donor. Our own measurement of
  onnxruntime-CPU versus our i8 loop is missing, so "cost per step" above is a
  size, not a time.
- The "minutes" figure for Playground is a paper claim with no number
  (arXiv:2502.08844 abstract: "train policies in minutes on a single GPU"); the
  repo gives 100M timesteps, not a wall clock.
- The 103-vs-101 lane mismatch in row 1 (measured both) is unexplained.
- Nothing here was executed: no GPU, no onnxruntime, and `crates/nn` still has
  no arena bind (EG24(d)).
