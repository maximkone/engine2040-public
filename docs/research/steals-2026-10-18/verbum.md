STEAL CARD, wave 2026-10-18/5, position 25/50. Read only, no bytes vendored.
Donor: scriptura/verbum @ 46375b4ecbeb (main, 2026-10-18T10:52Z, 0 stars) https://github.com/scriptura/verbum
Source list: github.com/topics/game-engine?l=rust&o=desc&s=updated (position 25).
Every reader below was fetched live on 2026-10-18. Golden d57b5c8ae3096614 is P2P on every row.
Honest note: at 0 stars this is a young research engine, and it is the closest donor on this list to our
own thesis — determinism, AOT content, strict I/O isolation. Read it as a design we already half-hold.

== 1. WHAT IT IS ==
A deterministic 2D engine built on strict data-oriented design: five crates (ecs, schedule, assets, forge,
verbum). It enforces ahead-of-time asset/script compilation, zero runtime parsing, and strict I/O
isolation. Its whole argument lives in the type system: a system's ACCESS TYPE is its contract, the
scheduler derives execution order from contracts alone, and a bad declaration fails at COMPILE TIME.
Source and docs are in French; the code is plain Rust 2024, workspace denies unsafe (Cargo.toml:26).

== 2. LICENCE ==
Declared MIT OR Apache-2.0 in Cargo.toml:16 ("license = \"MIT OR Apache-2.0\"", read 2026-10-18).
WARNING: no LICENSE, LICENCE or COPYING file exists in the repository tree (full tree read 2026-10-18);
the SPDX is a Cargo manifest field only, not a file. Treat it as dual MIT/Apache per the manifest, and
record the missing file before any port.

== 3. STEALS (ranked by value to the duel per hour) ==
1. Typed access IS the contract: order and ownership derived from contracts, violations caught at compile
   time | crates/schedule/src/access.rs:1-56,50-56 ; crates/schedule/src/contract.rs:11-21,64-73 ; crates/schedule/src/order.rs:5-17,31-48,53,77,110,134 ; crates/schedule/src/scheduler.rs:6-12,133,150,183 (read 2026-10-18) | MIT OR Apache-2.0 (manifest) | SIM, verification only (test cfg, no tick byte changes) | crates/ecs/src/schedule.rs, crates/ecs/src/spine_schedule.rs, crates/ecs/src/dispatch_plan.rs | M
   Idea: a system cannot choose what it reaches. A view (Read/Write/Produce) carries a const-generic
   `ID: u8` that is the CATEGORY identity, never the value type (access.rs:16-19), and its field is
   private, so a system cannot build a view or pick the storage behind a category. The contract is
   computed from the access type (contract.rs:64-73: reads, writes, produces, plus a 4-bit
   StructuralEffects, :11-21) — "no contract is written by hand". Bad declarations are const-eval panics:
   an id >= CAPACITY or a category twice in one access fails at build time (access.rs:50-56, E0080), and
   the crate ships compile_fail doctests for exactly these (access.rs:84-102). Binding failure returns
   None and never panics (:41-46). Order is then DEDUCED: precedes(producer, consumer) = writes|produces
   intersects reads (order.rs:31-36), an edge exists only between distinct positions (:45-48), a cycle in
   order_phase is a CONSTRUCTION error, never a runtime one (order.rs:10-13, :77). check_ownership
   enforces one writer per persistent category per phase and "a category is either written or produced,
   never both" (order.rs:15-17, :134). The model never sees a system: two lists with equal contracts give
   equal models (scheduler.rs:6-9), and the system count is stated twice with an E0080 mismatch
   (scheduler.rs:11-12, :36). A Scheduler<B, L, SYSTEMS, PHASES> is built once (scheduler.rs:133-150) and
   run per tick (run_tick :183).
   Maps onto us: ecs/src/schedule.rs:36-47,97 is 4 fixed phases of named boxed closures in registration
   order with NO read/write sets, so nothing can say "these two conflict"; spine_schedule.rs:1-11 pins
   5 spine systems by a registration test. This donor is the missing mechanism that arch.md steal 8 asks
   for from Bevy, one step stronger: Bevy's ambiguity_detection runs at build time over declared sets,
   while verbum makes the sets unforgeable by construction. For us the value is a test-cfg proof, not a
   new runtime: give each phase's systems declared access, derive the order, and refuse a conflicting
   pair with no edge — then run the golden under N seeded shuffles of every free tie-break (the same
   shuffle also gives RC36 the "no hidden order dependency at 1/2/4/8 threads" receipt it needs).
   Moves: systems with declared access 0 -> all spine systems; conflicting unordered pairs unknown ->
   0 or named; golden under shuffles 0 -> 50 equal runs. Proof: `cargo test -p engine-ecs --lib
   sched_shuffle` x2; a seeded missing edge fails naming both systems; golden d57b5c8ae3096614 unchanged.
   Kill: if the spine (5 systems, 5 phases) makes it trivial but the arena step cannot expose an access
   set without a rewrite, ship the shuffle on the fighter step only and say so.

2. An ahead-of-time asset manifest with opaque ids and no paths | crates/assets/src/manifest.rs:3-16,20-36,38-105,107-110 (read 2026-10-18) | MIT OR Apache-2.0 (manifest) | OFFLINE cook, never the tick | crates/assets/src/cooker.rs:216-219, crates/assets/src/recook.rs:5-8, crates/assets/src/gen_intake.rs:756-765 | S-M
   Idea: the manifest (signature "VMAN", :21) has a fixed little-endian layout: signature 4 + version 2 +
   entry count 4, then n entries of (AssetId u64, artifact_index u32, slot_index u32) — total length
   exactly 10 + 16n, and the header/entry sizes are checked by a const assert (:33-36). "Aucun nom, aucun
   chemin" (:16): the manifest carries no filename and no path. AssetId (:105) is deliberately opaque:
   pub(crate) from_raw only (:109), no numeric accessor and no From/Into an integer, enforced by six
   compile_fail doctests (:45-102). The id comes from an FNV-1a hash of the LOGICAL key (:38), so
   renaming a file cannot change identity.
   Maps onto us: assets/src/cooker.rs:216-219 hits on the SOURCE digest alone; recook.rs:5-8 mints ids
   per path; gen_intake.rs:756-765 writes three plain files non-atomically. This is arch.md steal 6 / row
   FB2 seen from the other end: not "what goes into the key" but "what an id may be allowed to be". Two
   ideas are portable immediately — an id type with no integer escape hatch (a rename cannot silently
   change a cooked id), and a length-pinned, names-free manifest table.
   Moves: id after a rename new -> same (with steal 6's dependency key, a dep edit recooks); a manifest a
   tool cannot reinterpret as a path list -> opaque. Proof: `cargo test -p engine-assets --lib
   manifest_layout` x2: bytes of a fixed 3-entry manifest equal the expected 10+16*3 buffer, and a
   renamed source keeps its id.
   Kill: if the opaque id blocks a debugger from mapping an id to an asset, keep a sidecar name table
   outside the shipped manifest (never inside the hashed bytes).

3. A fixed-capacity command buffer and generation-checked entity handles | crates/ecs/src/command.rs:3-10,16-35,30,57-76 ; crates/ecs/tests/world_identity.rs:3-19 (read 2026-10-18) | MIT OR Apache-2.0 (manifest) | SIM (determinism) | crates/ecs/src/snapshot.rs, crates/ecs/src/spine.rs, crates/ecs/src/archetype.rs | S
   Idea: structural mutations are deferred into a CommandBuffer<const COMMANDS> whose capacity is fixed at
   construction: "Aucune allocation" (:9) and a full buffer is an explicit BufferFull error, "jamais une
   croissance" (:28-30) — spawn/despawn/add/remove all return Result<(), BufferFull> (:57-76). The buffer
   never interprets its commands; resolution is unique and belongs to the World::apply (:5-7), so the
   order of application is one place, not many. Spawn carries no id — only the World mints ids (:17).
   The identity test (:8-19) pins the rule: a stale handle stays invalid after an index is reused
   (is_alive(old) false, despawn(old) == Err(StaleEntity)), and overflowing the capacity is
   Err(CapacityExceeded) rather than a panic.
   Maps onto us: the spine is 16 fixed slots (spine.rs:32) and our tick rule forbids per-tick allocation
   (tick-and-runtime). The steal is the pair of receipts: a capacity error that is a value, not a panic,
   and a generation check that makes a recycled slot's old handle provably dead — the exact safety a
   fighter slot needs when a duel ends and a slot is reused.
   Moves: a full buffer panics or grows -> explicit BufferFull/CapacityExceeded; a stale fighter handle
   silently aliasing a reused slot -> Err(StaleEntity). Proof: `cargo test -p engine-ecs --lib
   stale_handle` x2 mirroring world_identity.rs, plus golden unchanged.
   Kill: if the generation byte grows an existing hashed struct, keep the check in a test-only identity
   map instead of changing the snapshot layout.

== 4. NOT WORTH IT ==
- The French ADR corpus (docs/architecture/adr/*, docs/reflexivity/*): good methodology prose, no code,
  and our own docs already own these contracts (docs/README.md, docs/INDEX.md).
- The forge crate (batch/id/sprite pipeline) and the LPC YAML drafts: 2D sprite content for another game.
- Its asset loader's runtime formats: we cook to a bundle and a manifest already; take the identity rule,
  not the file format.
DONE 3 steals, 8 donor files read.
