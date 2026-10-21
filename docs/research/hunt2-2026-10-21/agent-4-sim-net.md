# Hunt 2, agent 4: the simulation and the net (agent-4-sim-net)

Date: 2026-10-21. Scope: determinism, rollback netcode, simulation fidelity and the
fixed-step tick. Board rows served: RC26, RC27, CQ10, HS23, HX20, MP25, MP26, BX9,
ON28, ON30, EG39, OPT22, E23. Read-only hunt: no build, no cargo, no edit under
`crates/`, no board edit. One file written: this one.

## Method

- Read the wave-1 folder first: `docs/research/hunt-2026-10-21/00-README.md` and
  `donors-seen.txt`, plus `docs/research/LADDER.md` and `docs/EVIDENCE.md`. A donor
  already in the prior ledger, or already named as a row's own donor in wave 1, is
  **not** re-mined. Section 4 lists what was seen and skipped, with the reason.
- Every number here was measured on 2026-10-21 with
  `node docs/research/hunt-2026-10-21/ghf.mjs <owner/repo> meta` (stars, forks,
  license, archived, last push, blob count) and
  `node docs/research/hunt-2026-10-21/ghf.mjs <owner/repo> tree "<regex>"|file <path>`
  (file names and byte sizes). Nothing is estimated from a repo description.
- Our tree was read first for every landing point. The line numbers below are lines
  that were actually opened in our tree, not guesses.
- The golden hash is `d57b5c8ae3096614` (seed 2040, 600 ticks). Every donor below
  carries a **golden** line. The rule used: a donor may not change the bytes the
  tick hashes. Safe donors are diagnostic-only (they read a digest and print), or
  presentation-only (they touch the drawn pose, never the sim), or they land in a
  **new file** with no call site in the tick yet.
- Rows live in `docs/SPRINT.md` and `docs/SPRINT-29-hunt.md`. Row text was read with
  `node tools/board-slice.mjs <ID>`; "needs" were checked so the ranking names a row
  that is really open.

---

## 1. Ranked donors (best first)

| # | repo (owner/repo) | stars | lic (LICENSE file) | donor file path (read) | what we take | our file:line + row | accept (measured) | golden |
|---|---|---|---|---|---|---|---|---|
| 1 | `xpTURN/Klotho` | 65 | Apache-2.0 (`LICENSE`, 11.1K) | `Docs/DesyncDiagnostics.md` (21.4K), `com.xpturn.klotho/Runtime/Core/Engine/KlothoEngine.Rollback.cs` (13.8K, 343 lines), `com.xpturn.klotho/Runtime/Core/Engine/KlothoEngine.DesyncProbe.cs` (40.8K) | The **desync funnel** with a one-line verdict: (A) classify `class=Input\|State` from a **command digest** beside the state hash, (B) find the first diverged tick from a 60-tick history ring, (C) name the **layer** `layer=Component(TransformComponent) \| System(PhysicsSystem)`, then one line: `tick=301 class=State layer=Component target=TransformComponent(1) local=0x… remote=0x…`. The `layer=System` line is the key: components alone cannot pin a divergence held in a system (an RNG). Second steal: a rollback target older than the snapshot ring must **resync, not clamp** — the donor's own comment says a clamp "leaves the mispredicted gap tick frozen". Third: invalidate the local sync hashes of the rolled-back range and the cached full-state bytes (a cache keyed only on `CurrentTick` goes stale after a rollback). | `crates/net/src/merkle.rs:51` (`EntityState`) and `:125` (`locate`), `crates/net/src/prediction.rs:205` (`DesyncDetector`) / `:255` (`observe`), `crates/net/src/rollback.rs:55` (`MAX_P2P_ROLLBACK` window). Rows **RC27** (50+ systems in verified order) and **ON30** (two-machine played proof) | `cargo test -p engine-net --lib merkle` x2: a `DesyncReport` carries a `class` (Input/State) and a `layer` (Component/System) field, and a state change with equal command digests reports `class=State`. | **Does not threaten.** The breakdown reads digests and prints; it never writes into the tick. The resync rule lives in the net layer, outside the hashed authority. |
| 2 | `pond3r/ggpo` | 3612 | MIT (`LICENSE`, 1.1K) | `src/lib/ggpo/backends/synctest.cpp`, `src/lib/ggpo/timesync.h` / `timesync.cpp`, `src/lib/ggpo/input_queue.h` / `input_queue.cpp` | The **synctest backend**: a deterministic script drives two players plus a seeded perturbation, the session is rolled back and replayed on a schedule, and a checksum compare proves the re-simulation is bit-identical to the forward run. Second steal: the **timesync window** — ggpo averages frame advantage over `FRAME_WINDOW_SIZE = 40` frames and only moves the delay when the advantage leaves `MIN_FRAME_ADVANTAGE = 3` .. `MAX = 9`; the input queue holds `INPUT_QUEUE_LENGTH = 128` entries. | `crates/net/tests/it/rollback.rs` (a new test module in the existing `crates/net/tests/it/main.rs` bin) and `crates/proof/src/differential.rs:390` (`compare`). Rows **RC26** (1,000 ticks recorded and replayed with 100% fidelity) and **MP26** (30-frame advantage window) | `cargo test -p engine-net --lib` green and a new test that runs 1,000 ticks through 20 seeded rollback-and-replay cycles: the per-tick checkpoint list is byte-equal to the no-rollback run, 20/20. | **Does not threaten.** The soak lives in the test harness. The delay knob is outside the tick hash; MP26's own Accept says "P2P: golden unchanged, tape bytes equal". |
| 3 | `matthart1983/ticktape` | 9 | Apache-2.0 (`LICENSE-APACHE` 11.1K + `LICENSE-MIT` 1.0K, dual) | `crates/ticktape-core/src/frame.rs` (10.7K), `crates/ticktape-journal/src/snapshot.rs` (12.8K), `crates/ticktape-core/src/crc32c.rs` (4.9K) | The **frame record**: a fixed 28-byte header `seq u64 \| timestamp u64 \| stream_id u16 \| kind u16 \| payload_len u32 \| header_crc u32 (CRC32C over bytes [0,24)) \| payload \| payload_crc u32`, plus a `FrameKind` enum where a **time-driven event is a sequenced input** (`TimerFired = 0x0013`) so it replays identically on every replica. The snapshot rule is the steal: "a snapshot is always taken at a deterministic point (a specific seq), never 'now', so every replica's snapshot at seq k is byte-identical", and "a corrupt or torn snapshot is skipped, never fatal — recovery must never lie". | `crates/proof/src/tape.rs:8` (`MAGIC`), `:11` (`HEADER_BYTES = 28`), `:17` (`encode`), `:41` (`decode`); `crates/core/src/crc32.rs:8` (`crc32`, IEEE reflected `0xEDB88320`) is where a Castagnoli CRC32C would sit beside it. Row **RC26** | The tape gains a per-record CRC32C and a `kind` byte. Accept: `cargo test -p engine-proof --lib tape` passes: a flipped payload byte is refused by name (not mis-read), and the decoded tape is byte-equal to the encoded one for 1,000 seeded runs. | **Does not threaten** if the tape format version is bumped and the sim is untouched. It does change the *fixture* bytes, so the accept must show the sim hash unchanged, not the tape bytes. |
| 4 | `Delta3-Studio/Backdash` | 176 | MIT (`LICENSE.md`, 1.1K) | `src/Backdash/Session/Backends/SyncTestSession.cs` (15.8K), `src/Backdash/Synchronizing/Input/Confirmed/ConfirmedInputs.cs` (3.9K), `src/Backdash/Synchronizing/Input/Confirmed/ConfirmedInputsSerializer.cs` (2.1K), `src/Backdash/Options/SyncTestOptions.cs` (3.8K) | The **sync-test session shape**: run the bout, run it again from the *saved* frames, compare a per-frame checksum, and on a mismatch call the desync handler and dump the state at the first bad frame — a test, not a runtime path. Second steal: `ConfirmedInputs`, an explicit set of "this input is final" ticks held **apart from the prediction buffer**, so a rollback knows exactly which inputs were a guess and which were truth. | `crates/proof/src/differential.rs:225` (`Report`) / `:236` (`is_clean`) and `crates/net/src/ack/input.rs:84` (`InputDelivery`, with `on_input_sent(tick)` at `:106` and `on_state(...)` at `:119`). Rows **RC26** (replay fidelity) and **BX9** (`Cut` on the wire and the tape: a confirmed-vs-predicted marker in the input log) | `cargo test -p engine-proof --lib differential` x2: a report names the first divergent step and the state at that step; `InputDelivery` reports a per-tick confirmed/predicted bit and the two agree with the wire bytes. | **Does not threaten.** The sync-test session is a test backend; the confirmed-input bit is ack bookkeeping outside the tick. |
| 5 | `nilpunch/unity-prediction-rollback` | 48 | MIT (`LICENSE`, 1.0K). Repo is **archived** | `Runtime/PredictionRollback/Command/Timeline/Prediction/FadeOutPrediction.cs` (2.1K), `Runtime/PredictionRollback/Command/Timeline/CommandTimeline.cs` (3.6K), `Runtime/PredictionRollback/Command/Timeline/Prediction/RepeatPrediction.cs` (1.4K) | **FadeOutPrediction**: a view over the command timeline that returns the command blended toward the truth — `fadeOutPercent = clamp(ticksPassed - startDecayTick, 0, decayDurationTicks) / decayDurationTicks`, defaults `startDecayTick = 30`, `decayDurationTicks = 60`. A predicted input is not a boolean; it fades into the confirmed one instead of snapping. Second steal: `CommandTimeline` keeps a `Dictionary<int, TCommand>` plus a **sorted** `List<int> _filledTicksInOrder` and answers "the latest tick with a command before T" with a `BinarySearch` (O(log n)), and trims with `RemoveAllCommandsDownTo` / `RemoveAllCommandsInRange` on a rollback. | `crates/net/src/interp.rs:463` (`Correction`) and `crates/anim/src/net_pose.rs:42` (`NetPose`). Rows **MP25** (visual correction after a rollback) and **EG39** (presentation is rollback-safe) | MP25's own Accept is the test: `cargo test -p engine-net --lib interp` x2 gives a first-frame step <= 6 percent of the jump and <= 2.5 percent left after 60 frames, **hash equal with correction on and off**. | **Does not threaten, and this is the safest donor on the list.** The fade is a drawn-pose rule, never hashed. MP25's Accept literally requires "hash equal with correction on and off". |
| 6 | `mrdav30/FixedMathSharp` | 94 | MIT (`LICENSE`, 1.1K) | `src/FixedMathSharp/Numerics/Scalars/Fixed64.WideRatio.cs` (38.0K), `Fixed64.FusedArithmetic.cs` (26.2K), `Fixed64.WideArithmetic.cs` (13.7K), `src/FixedMathSharp/Random/DeterministicRandom.cs` (7.3K, read in full), `src/FixedMathSharp/Core/FixedMath.Trigonometry.cs` (26.6K), `tests/FixedMathSharp.Tests/Core/FixedTrigonometry.Tests.cs` (34.9K) | Two steals. (a) The **exact wide-ratio ladder** (192 / 320 / 576-bit intermediates) so a fixed-point compare never loses a bit when the product overflows Q47.16. (b) The **addressed random stream**: `DeterministicRandom` is xoroshiro128++ with splitmix64 seeding, `NextBounded` uses the rejection threshold `(-bound) % bound` to kill modulo bias, and `FromWorldFeature(worldSeed, featureKey, index)` addresses a stream by (world, feature, index) instead of a mutable cursor — so a random draw is a pure function of its coordinates, not of how many draws came before. | `crates/core/src/fixed.rs:261` (`reference_cmp`) and `:276` (`reference_cmp_scalar`); the RNG half lands beside `crates/core/src/rng.rs` and `crates/core/src/counter_rng.rs`. Rows **CQ10** (core: checked numerics — 4,236 numeric `as` casts) and **HS23** (Kani harnesses on the integer kernels) | A Kani harness in `crates/proof/kani/` proves the wide compare never loses an ordering bit over the Q47.16 range; a unit test shows `FromWorldFeature(seed, key, i)` is equal to a fresh stream's i-th draw. | **Does not threaten only if it lands as a new file** (`crates/core/src/fixed_wide.rs`) and `reference_cmp` is **not** swapped in place. Swapping `reference_cmp` can reorder a sort and would move the golden. Say this in the row. |
| 7 | `devlinzhou/deterministic_float` | 50 | Apache-2.0 (`LICENSE`, 11.1K) | the whole tree (71 blobs): the soft-float core plus its header | The **soft float** as a standalone library: a 24-bit signed fraction and an 8-bit exponent, with add / mul / div / floor / ceil / fmod / inv-sqrt / log2 written as integer shifts. Wave 1 took the *sibling* repo `devlinzhou/deterministic_physics` (42 stars) for HX20; this repo is the clean library, without the physics engine wrapped around it. | `crates/core/src/soft_float.rs` (**does not exist yet** — HX20 owns exactly this path) and `crates/core/src/lib.rs`. Row **HX20** | `cargo test -p engine-core --lib soft_float`: the same soft-float ops give the same bits on every platform, and the file is not called by the tick. | **Does not threaten.** A new file with no call site in the tick. When it is wired it must stay in the float physfx layer, never in the hashed authority. |
| 8 | `rameshvarun/netplayjs` | 575 | ISC (`LICENSE`, 0.7K) | `ewmasd.ts` (the EWMASD ping estimator) | The **EWMASD estimator**: an exponentially weighted moving average with a standard deviation over the RTT samples, so the input delay tracks a noisy link instead of stepping +-1 on one sample. Our `delay.rs` steps one tick at a time from one sample; a smoothed estimate is what MP26 asks for. | `crates/net/src/delay.rs:38` (`frame_advantage`) and `:49` (`delay_for_advantage`). Row **MP26** (net: clock slew and a 30-frame advantage window) | `node tools/mp6-check.mjs`: over_cap 0 on every WAN profile, < 3 rollbacks per 1,000 ticks at 2-frame delay, max slew <= 5 percent, 0 skipped frames. | **Does not threaten.** The delay knob is outside the tick hash. MP26 says "P2P: golden unchanged, tape bytes equal". |
| 9 | `RJ/bevy_timewarp` | 43 | MIT (`LICENSE`, 1.1K). Repo is **archived** | `NOTES.md`, `src/frame_buffer.rs` | The **per-component frame history**: each component keeps its own ring of past values, so a rollback restores each component separately instead of restoring one whole-world blob. That is the difference between an O(world) restore and an O(changed) restore on a resim. | `crates/ecs/src/snapshot.rs:275` (`snapshot(world)`) and `crates/net/src/rollback.rs:91` (`FrameLog`). Rows **RC27** and **RC26** | `cargo test -p engine-core --lib snapshot`: restoring from a per-component ring reproduces the same `WorldDigest` as the whole-world restore, 1,000 ticks x2. | **Does not threaten.** Additive. Our `snapshot()` stays slot-ordered; the ring is a new structure beside it. |
| 10 | `gregorik/Rollback-Core` | 22 | MIT (`LICENSE`, 1.0K) | `Source/RollbackCore/Private/RollbackStateComponent.cpp` (8.1K), `RollbackManager.cpp` (12.0K), `RollbackNetSubsystem.cpp` (30.6K), `Private/Tests/RollbackCoreAutomationTests.cpp` (23.3K) | The **name-sorted state capture**: the component auto-caches every property flagged SaveGame, then "Sort tracked properties alphabetically by FName for cross-platform/compiler determinism" — so a captured blob's byte order is a *name* order, never a declaration or memory-layout order. Second steal: the state ring trims to `NewestFrame - Capacity + 1` and evicts the oldest frame first, which is the same rule as our `TAPE_CAP` ring. | `crates/net/src/snapshot.rs` (1,172 lines) and `crates/ecs/src/snapshot.rs:275`. Rows **RC27** (schedule verification) and **RC26** | The net snapshot codec emits components in name order: a byte-level test asserts the same component set gives the same bytes whatever order the components were registered in. | **Does not threaten.** A new capture order in a new codec path. The existing `snapshot()` is untouched. |
| 11 | `m2-md/rollback-netcode-lockstep` | 0 | MIT (`LICENSE`, 1.0K) | `src/bench-cli.ts` (4.6K), `src/rollback.ts` (4.1K), `src/sim.ts` (4.1K), `test/rollback.test.ts` (5.9K) | The **cost bench table**: a loopback of `FRAMES = 3000` at one-way delays `[0, 2, 4, 6, 9, 12, 18]` with `INPUT_DELAY = 2`, and the columns `rollbacks / replayed / stalls / msPerFrame / hash`. The hash column is the point: "Proves every latency value reaches the exact same final state via hash." A bench that proves the delay knob cannot change the outcome is exactly MP26's accept, printed. | `crates/net/src/delay.rs:72` (the `sweep` helper — one seeded 600-tick loopback bout per delay, seed `0x5EE9`) and `:81` (the `mod tests` beside it). Row **MP26** | The table lands as a receipt: 7 rows (delay 0..18), the last column constant across all 7, and the `replayed` column non-zero on at least 5 rows. | **Does not threaten.** A bench in the net crate; no tick bytes are involved. |
| 12 | `JiepengTan/LockstepEngine` | 966 | **none — there is no LICENSE file** (GitHub reports `license: none`) | `Engine.LockstepEngine/Src/Services/Core/ITimeMachine.cs` (0.5K), `Src/Services/Services/TimeMachineContainer.cs` (1.5K), `Src/Math/BaseType/LFloat.cs` (12.1K), `Src/Math/LUT/LUTAtan2.cs` (402.7K), `LUTSin.cs` (53.8K), `LUTCos.cs` (53.8K) | The **ITimeMachine contract**, three verbs and no more: `RollbackTo(tick)` (undo everything in [tick, ~)), `Backup(tick)`, `Clean(maxVerifiedTick)` (discard everything up to a verified tick) — plus the container that fans the call out to every registered subsystem, which is the shape of "one rollback undoes every stateful system, in order". Second: the pre-generated LUT trigonometry (LUTAtan2 is 402.7K of generated table, LUTSin/LUTCos 53.8K each). | `crates/ecs/src/snapshot.rs:153` (`WorldDigest`) and `crates/ecs/src/world_snapshot.rs:517` (the file); the LUT half would be `crates/core/src/det_math.rs:127` (`det_sin`) / `:165` (`det_atan2za`). Rows **RC27** (50+ systems in verified order) and **RC26** | RC27's Accept: 50+ systems in verified order, validation <= 0.1 ms, zero divergence. RC26's: 1,000 ticks recorded and replayed with 100% fidelity, overhead <= 0.2 ms/tick. | **Does not threaten — with two conditions, both binding.** (1) There is **no licence file**: re-derive the shape, never copy the table or the code. (2) Swapping `det_sin` / `det_atan2za` would move the golden: `crates/core/src/det_math.rs:411` pins `CORPUS_1M_DIGEST = 16_629_546_346_452_603_426`. The LUT may only land in a **new** file, with `det_math.rs` left alone. |

### Also read, not ranked

- `drftrun/driftengine` (15 stars, Apache-2.0, `LICENSE` 11.1K): a whole TypeScript 3D engine (28,752 KB). Its `packages/script/src/bindings/rollback.ts` is a thin script binding over a core that was not readable in the time we had, so the steal could not be named to a file we read.
- `HeatXD/GekkoNet` (62 stars, BSD-2-Clause): a P2P rollback SDK, C++, 7,152 KB. The model is the same one-lag-and-repeat our `crates/net/src/netcode.rs` already has; the extra machinery is Unity-shaped.
- `mrdav30/LockstepRTSEngine` (186 stars, MIT): the same author as #12 and #6, marked "(WIP)"; it repeats the ITimeMachine steal.

---

## 2. Wild ideas

1. **A CRC on every tape record.** Take ticktape's per-record `header_crc` + `payload_crc`
   and give our tape (`crates/proof/src/tape.rs:11`, a 28-byte header) a CRC32C. A torn
   record is then *detected and named* instead of silently mis-read. Cost: one 256-entry
   table and 4 bytes per record. Ours would be a Castagnoli CRC beside the IEEE one we
   already have at `crates/core/src/crc32.rs:8`.
2. **A predicted input is a blend, not a boolean.** Take nilpunch's fade and make it a
   first-class view over `crates/net/src/interp.rs:463`: the drawn pose carries a 0..1
   weight that reaches 0 exactly when the confirmed input arrives. The visual error then
   goes to zero by construction instead of decaying toward it.
3. **Resync, never clamp.** Take Klotho's rule: a rollback target older than the snapshot
   ring must trigger a full resync, because a clamp leaves the mispredicted gap tick
   frozen forever. Our `MAX_P2P_ROLLBACK` is 128 (`crates/net/src/rollback.rs:55`) and
   `WAN_ROLLBACK_CAP` is 9 (`:60`); the same trap sits in both windows.
4. **A desync report that names the cause.** Our `crates/net/src/merkle.rs:125` `locate`
   returns a `DesyncReport` with *where*; Klotho adds *why*: a command digest beside the
   state hash says "the inputs differed" (an engine-side problem) or "the inputs were the
   same and the sim drifted" (a determinism violation). Two extra u64s on the report.
5. **A delay table as the MP26 receipt, and the delay decision on the tape.** Take
   m2-md's bench: rows = one-way delay 0..18, columns = rollbacks, replayed frames,
   ms/frame, final hash, and the last column constant on every row. Thirty lines of Rust
   and it *is* the proof that the delay knob cannot change the outcome. Then make the
   delay estimator a pure, seeded function (`crates/net/src/delay.rs:38`) and write the
   chosen delay into the tape, so a replay reproduces the same delays and "identical
   final hash across delays" becomes a property of the file, not of a bench.
6. **A snapshot is taken at a tick, never "now".** Take ticktape's rule and put it on
   `crates/ecs/src/snapshot.rs:153` (`WorldDigest`): a snapshot at tick k must be
   byte-identical on every peer. One test, one sentence in the row, and it kills a whole
   class of "why did the two sides differ" questions.
7. **A name-sorted capture order.** Take Rollback-Core's alphabetical-by-name sort. Our
   world is slot-ordered SoA, so this only bites when a component set grows; but the net
   snapshot codec can pin the order by name so a codec change never reorders bytes.
8. **Per-component history rings.** Take bevy_timewarp. Instead of one world blob per tick
   in the rollback window (`crates/net/src/rollback.rs:91`, `FrameLog`), keep one ring per
   component so a resim touches only the components that changed.
9. **An exact wide compare, proved.** Take FixedMathSharp's 192/320/576-bit ladder and
   put a Kani harness on it (HS23 owns `crates/proof/kani/`): "our Q47.16 compare never
   loses an ordering bit" is a claim a harness can make, and a cast cannot.
10. **A rollback ledger.** Every rollback appends `(from_tick, to_tick, hashes invalidated,
    frames replayed)` to a bounded ring in `crates/net/src/rollback.rs`. A fight that
    desyncs can then be replayed from the ledger instead of from tick 0 — which is the
    seed of RC26's "1,000 ticks recorded and replayed with 100% fidelity".

---

## 3. Rejected

1. `gschup/ggrs` — already mined (prior LADDER PORT cards for `input_queue` and
   `time_sync`; our `crates/net/src/netcode.rs:21` `MAX_ROLLBACK = 9` already carries the
   ggrs cap). Re-mining it spends an evening on a row we have already paid for.
2. `HouraiTeahouse/backroll-rs` — 369 stars, ISC. Already mined (EVIDENCE.md marks it
   "ADOPT - pattern") **and** the pattern already landed: `crates/net/src/session.rs:1074`
   `encode_input_compressed` with the XOR delta and the zero-run RLE at `:1167`. Nothing
   left to take.
3. `devlinzhou/deterministic_physics` — 42 stars, Apache-2.0. Already mined: it is HX20's
   own named donor (`gphysics/include/math/glacier_float.h`). Its sibling
   `deterministic_float` is the standalone library and is ranked #7.
4. `braedonsaunders/voidstrike` — 26 stars, MIT. Already mined: HX16's named donor
   (`src/engine/network/MerkleTree.ts`), and the row is landed — `crates/net/src/merkle.rs:125`
   `locate` is the result.
5. `mrdav30/Gravitas` — 1 star, MIT. Already mined: HX19's named donor
   (`src/Gravitas/Determinism/GravitasReplayHashMode.cs`). One star, and the row already
   names it.
6. `nithish101/resonance` — already mined: HX17's named donor
   (`packages/web/src/render/loop.ts`, `docs/decisions/0007-deterministic-capture.md`).
   The web lane belongs to hunter 2 anyway.
7. `albertok/godot-rocket-league` — 124 stars, MIT. The tree is a game project whose
   netcode is the **third-party netfox addon** (`addons/netfox.extras/`), not the repo's
   own code, and the rest of the tree is prebuilt binaries (`addons/debug_draw_3d/libs/*.so`,
   2,091K each). We would be reading someone else's addon through a game.
8. `w1ne/labwired-core` — 83 stars, MIT. A hardware co-simulation CLI: `crates/cli/src/commands/test.rs`
   is 123.2K, `run.rs` 113.6K, `execute.rs` 86.5K, all aimed at ESP32 / RISC-V baskets. We
   want the simulation, not an RTL test rig.
9. `mii443/detllm` — 0 stars, 403 KB, and GitHub reports `license: none`: there is no
   licence file, so all rights reserved and there is nothing to reuse.
10. `drftrun/driftengine` — 15 stars, Apache-2.0, **28,752 KB**. A whole TypeScript 3D
    engine (WebGPU, physics, editor, scripting). The only rollback file we could name is
    `packages/script/src/bindings/rollback.ts`, a thin binding. Too large to verify a
    single steal inside, and the row it would serve (RC27) has better donors.

---

## 4. Donors seen but skipped

Already in the prior ledger (`docs/research/LADDER.md`, `docs/EVIDENCE.md`) or named as a
row's own donor in wave 1 — **not re-mined**:

`gschup/ggrs`, `jamesmunns/postcard`, `naia-lib/naia`, `simgine/bevy_replicon`,
`UkoeHB/renet2`, `paritytech/trie`, `pawlenartowicz/rand_philox`, `tspiteri/fixed`,
`PetteriAimonen/libfixmath`, `simba/libm_force`, `parry3d/enhanced-determinism`,
`kan7/Klotho-Lockstep` (LADDER card; its successor `xpTURN/Klotho` is ranked #1),
`HouraiTeahouse/backroll-rs`, `braedonsaunders/voidstrike`, `nithish101/resonance`,
`mrdav30/Gravitas`, `devlinzhou/deterministic_physics`, `madsim-rs/madsim`,
`alloy-rs/ruint`, `mas-bandwidth/fixed`, `mrdav30/GridForge`, `markclausing/websoccer`,
`cBournhonesque/lightyear`, `security-union/videocall-rs`, `axelromero99/motumbo`,
`gastownhall/beads`, `gfx-rs/wgpu`, `rust-random`, `foxglove/mcap`,
`tigerbeetle/tigerbeetle`, `tikv/raft-rs`, `dzhng/battlesim` (HX14's donor).

Seen in this hunt, read enough to judge, and skipped with the reason:

| donor | stars | lic | why skipped |
|---|---|---|---|
| `johanhelsing/extreme_bevy` | 154 | CC0-1.0 | A Bevy action game. Bevy is on the wave-1 do-not-mine list, and the repo is a game, not a library we can lift from. |
| `mrdav30/LockstepRTSEngine` | 186 | MIT | Same author as #12 and #6, marked "(WIP)"; repeats the ITimeMachine steal and adds an RTS game around it. |
| `HeatXD/GekkoNet` | 62 | BSD-2-Clause | A P2P rollback SDK whose model is the one-lag-and-repeat we already have in `crates/net/src/netcode.rs`; the rest is Unity transport. |
| `penberg/hiisi` | 128 | MIT (archived) | A proof-of-concept libSQL in Rust with deterministic simulation testing. A database, not a tick: the interesting part (a deterministic simulator) is what we already are. |
| `jackmaiorino/mtg-kernel` | 2 | MIT | A deterministic MTG rules kernel, 57,488 KB. Surfaced only by the `filename:deterministic_math` code-search trick. Two stars, and the rules engine is not a rollback donor. |
| `mrdav30/FixedMathSharp` (trig half) | 94 | MIT | Ranked #6 for the wide-ratio ladder and the RNG. Its trig half (`FixedMath.Trigonometry.cs`, 26.6K) is **not** taken, because swapping our `det_sin` would move the golden pinned at `crates/core/src/det_math.rs:411`. |

---

## 5. Verdict

**The first thing to steal** is #5, `nilpunch/unity-prediction-rollback`: the fade-out
prediction (`startDecayTick = 30`, `decayDurationTicks = 60`). It is the smallest change
on the list, it lands on two files we already own (`crates/net/src/interp.rs:463`,
`crates/anim/src/net_pose.rs:42`), it serves two open rows (MP25, EG39), and it is the
only steal whose Accept is already written for us — MP25 wants a first-frame step <= 6
percent of the jump, <= 2.5 percent left after 60 frames, **hash equal with correction on
and off**. It cannot move the golden because it never touches the sim.

**The biggest** is #1, `xpTURN/Klotho`: a 65-star Apache-2.0 engine whose desync funnel
answers *which tick, which class, which layer* in one line, and whose rollback code
knows that a deep rollback must resync rather than clamp. It serves RC27 and ON30, and
it is the only donor that adds a **cause** to our desync report, which today knows only
*where*.

**The most Rust** is #3, `matthart1983/ticktape` (9 stars, Apache-2.0/MIT dual): a
frame record with a per-record CRC32C and the rule that a snapshot is taken at a
deterministic point, never "now". It lands on `crates/proof/src/tape.rs` for RC26 and
needs no re-derivation — it is already Rust, already little-endian, already 28 bytes.

**The one to refuse** is #12, `JiepengTan/LockstepEngine`: 966 stars, no licence file at
all, and its LUT trigonometry would move the digest pinned at
`crates/core/src/det_math.rs:411`. Read the `ITimeMachine` contract, re-derive it, do not
copy the table.
