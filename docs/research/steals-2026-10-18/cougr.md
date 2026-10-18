STEAL SCOUT: SYSTEM ORDER AS ONE DECLARATIVE GRAPH, QUERY CACHES, DIRTY TRACKING (salazarsebas/Cougr)   scout date 2026-10-18, read-only run
Repo: https://github.com/salazarsebas/Cougr  sha 8f4f34f1f63ca25547d95c8822b74b07bc1ae26f (2026-10-18T16:59:24+02:00, "feat(example): add sokoban puzzle demonstrating ArchetypeWorld and component-signature queries (#457)")
Files read live 2026-10-18 by shallow clone; no bytes vendored.
LICENCE: MIT (LICENSE, "Copyright (c) 2025 Cougr Contributors"; API spdx MIT).
What it is: an ECS framework for on-chain games on Stellar/Soroban (no_std, alloc-only, `soroban_sdk` types). It is a small engine that runs inside a contract: every iteration is paid
  for, so it is unusually explicit about *bounded* and *ordered* work. Two things are worth taking: a schedule stated as one declarative before/after graph with a cycle error, and
  version-stamped caches plus a dirty tracker so nothing is recomputed or rewritten that did not change.

== 1. WE HAVE ==
- crates/ecs/src/schedule.rs:12-96 has Phase, Ctx, ScheduleError and an AmbiguityLevel port from bevy_ecs 0.20; crates/ecs/src/dispatch_plan.rs and parallel.rs split work.
- crates/ecs/src/change_tick.rs + archetype::is_newer_than give change detection per column (the equivalent of a dirty flag for reads).
- crates/save/{persist.rs,rollback.rs} own what is written; crates/spatial has an explored-map idea and per-tile visibility.
- There is no single file that states the *system order* the way the donor's SystemConfig does, and no query cache keyed by a world version.

== 2. STEALS (ranked by value per hour) ==
1. System order as one declarative before/after/before_set/after_set graph, resolved by Kahn's algorithm with a cycle error naming the systems | src/scheduler/mod.rs:377-463 + src/scheduler/types.rs:12-66 | MIT | SIM (tick structure) | lands crates/ecs/src/schedule.rs (+ dispatch_plan.rs) | M
   Pattern: a `SystemConfig` carries stage (Startup/PreUpdate/Update/PostUpdate/Cleanup), set, `after`, `before`, `after_sets`, `before_sets` (types.rs:12-66). `execution_plan_for_stage`
   builds a plan per stage; `topological_order` builds indegrees from those edges and runs Kahn's algorithm with a FIFO queue seeded in *registration* order, so the tie-break is the
   order systems were declared, not a hash (:431-449); if the plan is short of the set it returns `ScheduleError::DependencyCycle { stage, systems }` listing every system in the stage
   (:451-459). `validate_unique_names` and `validate_stage_dependencies` guard the graph before it runs (:298-311).
   Maps onto: our fixed tick order is a documented sequence plus a package-level schedule; a declarative graph with a *cycle refusal at startup* turns "the order is a comment" into
   "the order is checked". The registration-order tie-break is the part to copy exactly: it makes the resolved order independent of any hash or sort instability. `ScheduleError`
   already exists in our tree, so the addition is the named-edge graph and the cycle message, not a new error type.
   Moves: every ordering edge between systems is stated where the system is declared; a cycle is refused at build, naming the systems; two runs resolve one order.
   Proof: `cargo test -p engine-ecs --lib schedule_order` new case (chain resolves; a 3-cycle is refused with all three names; 100 runs give one order) x2; golden d57b5c8ae3096614
   unchanged.
   Risk: a declarative graph is easy to leave half-filled, so a real edge goes unstated and the tie-break decides it by accident. Kill: if any two systems write the same hashed column
   and their edge is only implied by registration order.
2. A query cache stamped by the world version, invalidated by one integer | src/query/state.rs:6-104 + src/query/mod.rs:20-33 | MIT | SIM (broad phase) | lands crates/ecs/src/{views.rs,archetype.rs} and crates/spatial/src/index.rs | S
   Pattern: `SimpleQueryState` holds `cached_results` and `cached_version`; `execute` re-runs the query only when `cached_version != world.version()` (:23-29); `invalidate()` zeroes the
   version and `is_valid(world_version)` is one integer compare (:31-37). `SimpleQuery` itself selects "the narrowest indexed candidate set available" with required / excluded /
   any components and a Table-vs-Any storage choice (:20-33, ::new .. without_components).
   Maps onto: our spatial index and archetype iteration re-scan on every query today. A world version number bumped by any structural change, plus a per-query cache keyed on it, is the
   smallest correct memo: no dirty-flag bookkeeping per column, just one counter and one compare. It also pairs with our existing change detection instead of replacing it.
   Moves: a repeated query without a structural change does not re-scan; the cache is provably stale-free because one counter moves on every spawn/despawn/component add/remove.
   Proof: `cargo test -p engine-ecs --lib query_cache` x2 (a hit after no change, a miss after one spawn); golden unchanged.
   Risk: a structural change that does not bump the version returns stale rows. Kill: if the version bump is not inside the single place mutations happen, or if a stale row is observed
   in the fuzz tape.
3. A dirty tracker: mark what changed, flush only that, clear after | src/incremental/dirty_tracker.rs:13-111 | MIT | SIM-adjacent (persistence) | lands crates/save/src/persist.rs | S
   Pattern: four lists (dirty entities, dirty (entity, component) pairs, despawned, new) plus one `meta_dirty` bool; each mark is idempotent (`if !contains { push }`), `is_dirty` is an
   or over them, and `clear()` resets after a flush (:33-110). The header states the payoff in the donor's own currency: only dirty entries are written, "avoiding redundant storage
   writes and reducing gas costs" (:3-5).
   Maps onto: our save path and any per-bout record. Marking changes and flushing only those is the cheap half of incremental persistence; the idempotent-mark + clear-after-flush shape
   is what keeps it from growing without bound. The dedup `contains` scan is O(n^2) and fine only because the sets stay small, which the donor relies on too.
   Moves: a bout's save writes only changed rows; a no-op tick writes nothing; the tracker is empty after flush.
   Proof: `cargo test -p engine-save --lib dirty_flush` x2 (a no-change tick writes zero rows; a changed row writes once).
   Risk: the dedup scan degrades on a large change set. Kill: if the dirty set exceeds a few thousand entries in a bout, replace the Vec scan with a sorted set before landing.

== 3. NOT WORTH IT ==
- The zk/ subtree (src/zk/advanced.rs, src/zk/merkle/tree.rs, src/circuits/generated.rs) and src/accounts/recovery.rs: zero-knowledge proofs and on-chain account recovery for a
  contract. Merkle-per-tick receipts are a possible future proof row, but rule 4 says never hand-roll crypto, and there is no consumer today (Slim law: no consumer, no steal).
- src/simple_world and src/archetype_world: two ECS storages plus `soroban_sdk::Vec` everywhere. Our crates/ecs already has archetype and sparse storage; only the cache and the
  tracker generalise.
- src/session/{manager,status}.rs: a turn/session manager for on-chain matches. Our arena and net sessions are different machinery.
- src/cors.rs: a hand-rolled HTTP CORS policy validator. Not engine work.

DONE 3 steals, 6 donor files read (scheduler/mod.rs, scheduler/types.rs, query/mod.rs, query/state.rs, incremental/dirty_tracker.rs, LICENSE).
