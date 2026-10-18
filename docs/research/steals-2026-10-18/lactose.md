STEAL SCOUT: DETERMINISM BOUNDARY, HASH, AND THE TICK-SAFE HOST EDGE (MilkyEngineer/LACTOSE)   scout date 2026-10-18, read-only run
Repo: https://github.com/MilkyEngineer/LACTOSE  sha 2109be986621f86ba71db2872850ecb4c57eb96b (2026-10-18T15:10:55+10:00, "Merge the lactose-net-host implementation")
Files read live 2026-10-18 by shallow clone; no bytes vendored.
LICENCE: MIT (LICENSE, "Copyright (c) 2026 Alex Stevens"; API spdx MIT). The tree vendors one patched third-party copy under vendor/rapier3d-f64-0.36.0, which keeps its own
  licence file LICENSES/rapier3d-f64-Apache-2.0.txt (Apache-2.0); that is the vendored crate's licence, not this repo's.
What it is: a from-scratch Rust game engine (f64 physics, integer-safe sim) built around one idea engine2040 shares: a hard *determinism boundary*. Crates are sorted into
  foundation/backend/assembly/leaf/game tiers and a named [boundary] set, all stated once in crate-tiers.toml; a 0.36.0 f64 Rapier fork is vendored so the solver's float math is
  pinned rather than floated by a crates.io update. Its rates: raw floats are banned from hashed layouts, one order-independent world hash runs the same under any worker count,
  and a field-level digest explains a mismatch instead of only naming the tick.

== 1. WE HAVE ==
- Hash owner is integer and host-independent already: crates/core/src/hash.rs:1-22 (FNV-1a 64, integers little-endian, floats as IEEE-754 bits, no std::hash::Hasher on purpose).
  So the repro risk LACTOSE is built against (bytemuck byte views, NaN payloads) is mostly closed for us, but one hole stays: two NaN payloads hash as different bytes.
- Randomness is already forked and counter-based (crates/core/src/counter_rng.rs:32-145, fork_named/for_island/for_chunk); the steal below is the *test* discipline, not the RNG.
- ECS: crates/ecs/src/schedule.rs:12-96 has Phase, Ctx, ScheduleError and an AmbiguityLevel port from bevy; there is no named before/after edge statement in one file to check.
- Snapshots: crates/save/src/rollback.rs and crates/net/src/{netcode.rs,snapshot.rs} own save/restore today.

== 2. STEALS (ranked by value per hour) ==
1. Canonical float bytes: collapse every NaN to one encoding before a hash or a save | crates/math/src/canonical.rs:1-89 + Scalar::canonical/to_canonical_bits | MIT | SIM (hash + wire) | lands crates/core/src/hash.rs (write path) and crates/core/src/digest.rs | S
   Pattern: a `Canonical` trait whose `to_canonical_bytes` writes each scalar's canonical bits little-endian; `canonical()` replaces every NaN with the canonical NaN and changes
   nothing else; -0.0 and +0.0 stay distinct because they are distinct states (canonical.rs:7-15, 35-49). The doc states the exact reason we need it: a bytemuck byte view of a
   struct holding a NaN differs between architectures, so a bulk copy is only canonical after `canonical()` has run on the rows (:13-15).
   Maps onto: our Fnv64 writer already serialises float bits, so one cross-arch NaN (x87 vs SSE vs NEON) still changes the digest. One `canonical_f32/f64` step at the write
   boundary (hash.rs) plus a rule that no hashed column stores a raw NaN; the digest and the replay checkpoint then agree on every machine by construction.
   Moves: hashed bytes for any state holding a NaN a -> one encoding; F2P a crafted f32::from_bits(0x7fc0_0001) column hashes differently from 0x7fc0_0000 today.
   Proof: new `cargo test -p engine-core --lib canonical` (two NaN payloads, -0.0 vs +0.0) x2; golden d57b5c8ae3096614 must not move (no NaN in the fixture).
   Risk: a legitimate NaN payload carrying data would be flattened. Kill: if any hashed column is documented to carry NaN-payload bits, drop the collapse and keep the test as a
   guard only. Kill also if the golden hash moves.
2. Order-independent world hash: per-column (count, wrapping sum), folded through a map | crates/sim/src/hash.rs:63-120, 216-247, 260-280 | MIT | SIM (proof) | lands crates/proof | M
   Pattern: the world hash walks every hashed column, computes (row count, wrapping sum of row hashes) per chunk, then folds the chunks by addition into a BTreeMap keyed by
   component. The header adds seed, region mode and the asset version table, and a single xxh64 over that buffer gives the region hash (:260-279). The doc says it plainly: same
   under every chunk layout and worker count (:53-57), and the forked form is "bit-identical to the sequential form by construction" because an addition fold has no order.
   There is a test that proves it across chunk_rows {1, 7, None}, permuted rows, shuffled chunk order, split batches (:933-980).
   Maps onto: our per-tick hash rows already name the first divergent tick, but a chunk-count or worker-count change is still a way to move the hash. Adding the count+sum fold
   per column makes the hash invariant to our ecs dispatch_plan/parallel splitting, so a red row can only mean a real state change, and the `{1,7,None}` x permute test is the
   F2P that proves it.
   Moves: hash under 1 worker equals hash under N workers on the golden tape, shown, not asserted; one extra (count, sum) pair per component in the digest.
   Proof: `cargo test -p engine-proof` new layout-independence case x2; golden unchanged.
   Risk: a wrapping sum loses position within a column, so two rows with swapped values read equal; the digest below is the compensator. Kill: if the digest is not landed beside
   it, or if any capture/hash pair that should differ reads equal in the new test.
3. Field-level digest + diff that names entity, component and field on a mismatch | crates/sim/src/hash.rs:611-908 | MIT | SIM (proof) | lands crates/proof (beside read.rs/tape.rs) | S
   Pattern: `RegionDigest` keeps the hash of every field of every row, keyed by (origin key, component), ascending; `hash()` recomputes exactly the world hash it stands for, so a
   digest taken on a mismatch explains it (:666-688); `diff()` walks two digests and yields a whole-row difference or a per-field difference (:690-700, 855-887); `encode/decode`
   is canonical and re-encodes to check itself (:702-776).
   Maps onto: crates/arena/src/sync.rs splits hashes by system and crates/proof/read.rs reads a divergence; today a mismatch names a split point, not the field. A digest taken at
   the same tick on both peers turns "MATCH/DIVERGE" into "fighter 2, component X, field id 5", which is the difference between a bisect and a root cause.
   Moves: a desync report names (tick, component, field) not just (tick, split); report bytes <= 1 KB for a 2-fighter bout.
   Proof: `cargo test -p engine-proof` new digest round-trip + diff-names-field case x2, plus a deliberate one-field mutation caught with the field id in the message.
   Risk: the digest is bigger than a u64 and must never enter the hash itself. Kill: if a digest byte reaches the golden hash path or the digest grows the tick budget past
   the gate.
4. Seqlock handoff from the host to the tick: publish a fixed 6-word sample, read it lock-free | crates/input-host/src/seqlock.rs:1-132 | MIT | PRESENTATION (host edge, off-tick) | lands crates/input (+ crates/clock) | S
   Pattern: the sequence is even while the words are stable and odd while a publisher writes (:6-8); publish does a compare_exchange even->odd (Acquire), a Release fence, the
   words Relaxed, then the next even sequence (Release), so "the last to finish wins whole" (:85-114); read loads the sequence Acquire, the words Relaxed, an Acquire fence, then
   reloads the sequence and retries on an odd or changed value (:116-132). Words are 6 x u32 and the whole thing is written once against a `Memory` trait so a loom model runs the
   same functions (:1-4, 17-35).
   Maps onto: our device/host thread must hand the latest input sample to the fixed tick with no lock inside the tick. This is the smallest correct shape for that, and the
   loom-modelled single implementation means the model checks the shipped code, not a copy.
   Proof: `cargo test -p engine-input --lib seqlock` (reader never sees a torn sample; publisher wins whole) x2; golden unchanged (inputs still reach the sim only through the tape).
   Risk: a lock-free reader that spins on a preempted publisher; the donor yields after 64 spins (:74-82). Kill: if any input reader is reachable from the sim tick rather than
   from the host edge, or if a loom model is not added (unmodelled atomics are not allowed by our rust-safety rule).

== 3. NOT WORTH IT ==
- The vendored f64 Rapier fork: our 3D path is Rapier f32 and the frozen golden path; a second solver is CUT. Read as pinning evidence only.
- The full tier/boundary policy machinery (crate-tiers.toml:1-186 plus tools/xtask/src/{tiers,boundary,policy}.rs): our tools/workspace-check.mjs already holds layering, and the
  donor's five tiers plus a [boundary] set is a bigger machine than the tree needs. One line worth borrowing if we ever add a crate: every dependency whose name appears in the
  file is held to the same rules wherever it comes from.
- Read but not carded (no consumer yet, Slim law): crates/task/src/quiesce.rs:28-67 (one atomic word holding a job count and a closing bit, so a graph run's future resolves exactly
  once and never while a job still writes) and crates/task/src/gate.rs:31-177 (run permits that a block_in_place reservation closes). Both are the shape our off-thread results
  would want at a tick boundary; park them until a row exists that needs a worker-scoped run to be provably quiet before the tick reads it.

DONE 4 steals, 6 donor files read (math/src/canonical.rs, input-host/src/seqlock.rs, sim/src/hash.rs, task/src/gate.rs, task/src/quiesce.rs, crate-tiers.toml).
