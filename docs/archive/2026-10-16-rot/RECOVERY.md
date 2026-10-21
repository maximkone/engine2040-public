# Recovery list: Rust files removed by the rot cleanup, 2026-10-16 22:2x-22:3x

Nothing here is lost: every file is in git. Restore one with `git show <sha>^:<path> > <path>` (the parent of the deleting commit holds the last copy). They were unwired modules (no caller outside their own tests) removed as rot; board rows were not un-ticked. Before restoring, read the row that wants it and give it a real caller.

| path | lines | deleting commit | exists again |
|---|---|---|---|
| crates/destruction/src/bond_grade.rs | 993 | 3387a96e | no |
| crates/destruction/src/debris.rs | 682 | 3387a96e | no |
| crates/destruction/src/lib.rs | 16 | 3387a96e | yes (rewritten) |
| crates/destruction/src/nav_dirty.rs | 419 | 3387a96e | no |
| crates/destruction/src/wall_slice.rs | 395 | 3387a96e | no |
| crates/destruction/tests/contract.rs | 176 | 3387a96e | no |
| crates/endpoint/src/remote_shim.rs | 4 | 81cc2c04 | yes (rewritten) |
| crates/game/src/ballistics.rs | 1174 | d89bd98e | no |
| crates/game/src/camera_feel.rs | 201 | d89bd98e | no |
| crates/game/src/camera_stack.rs | 232 | d89bd98e | no |
| crates/game/src/cine.rs | 651 | d89bd98e | no |
| crates/game/src/eight_fight.rs | 211 | d89bd98e | no |
| crates/game/src/flow.rs | 6 | d89bd98e | yes (rewritten) |
| crates/game/src/modes.rs | 764 | d89bd98e | yes (072e79ba) |
| crates/game/src/package.rs | 1596 | d89bd98e | no |
| crates/game/src/quest.rs | 247 | d89bd98e | no |
| crates/game/src/window_bind.rs | 380 | d89bd98e | no |
| crates/game/tests/it/camera_stack.rs | 99 | d89bd98e | no |
| crates/game/tests/it/main.rs | 2 | d89bd98e | yes (rewritten) |
| crates/game/tests/it/package_portable_vectors.rs | 162 | d89bd98e | no |
| crates/nav/src/astar.rs | 170 | 3ed325c8 | no |
| crates/nav/src/crowd.rs | 1491 | 3ed325c8 | no |
| crates/nav/src/funnel.rs | 824 | 3ed325c8 | no |
| crates/nav/src/hpa.rs | 176 | 3ed325c8 | no |
| crates/nav/src/neighbors.rs | 139 | 3ed325c8 | no |
| crates/nav/tests/hpa.rs | 100 | 3ed325c8 | no |
| crates/net/src/anticheat.rs | 478 | 3151bd46 | no |
| crates/net/src/guard.rs | 846 | 3151bd46 | no |
| crates/net/src/lagcomp.rs | 828 | 3151bd46 | no |
| crates/net/src/nat.rs | 409 | 3151bd46 | no |
| crates/net/src/replicate.rs | 1501 | 3151bd46 | no |
| crates/net/src/resim.rs | 139 | 3151bd46 | no |
| crates/net/src/server.rs | 497 | 3151bd46 | no |
| crates/net/src/steam_relay.rs | 278 | 3151bd46 | no |
| crates/net/tests/it/a01_delay_bench.rs | 49 | 3151bd46 | no |
| crates/net/tests/it/command_log_replica.rs | 208 | 3151bd46 | no |
| crates/net/tests/it/dots_expectations.rs | 137 | 3151bd46 | no |
| crates/net/tests/it/frag_vectors.rs | 289 | 3151bd46 | no |
| crates/net/tests/it/input_validators.rs | 185 | 3151bd46 | no |
| crates/net/tests/it/join_gate_vectors.rs | 365 | 3151bd46 | no |
| crates/net/tests/it/main.rs | 7 | 3151bd46 | yes (rewritten) |
| crates/net/tests/it/protocol_hash_vectors.rs | 246 | 3151bd46 | no |
| crates/net/tests/it/visibility_client_vectors.rs | 163 | 3151bd46 | no |
| crates/perf/src/pgo_harness.rs | 551 | 65e2da7c | no |
| crates/perf/src/server_budget.rs | 912 | 65e2da7c | no |
| crates/physics/src/broadphase.rs | 212 | 3d6ee4a1 | no |
| crates/physics/src/cast.rs | 187 | 3d6ee4a1 | no |
| crates/physics/src/checked_fix.rs | 110 | 3d6ee4a1 | no |
| crates/physics/src/cloth.rs | 2654 | 3d6ee4a1 | yes (rewritten) |
| crates/physics/src/grapple.rs | 1171 | 3d6ee4a1 | no |
| crates/physics/src/islands_build.rs | 247 | 3d6ee4a1 | no |
| crates/physics/src/islands.rs | 1694 | 3d6ee4a1 | yes (rewritten) |
| crates/physics/src/lib.rs | 23 | 3d6ee4a1 | yes (rewritten) |
| crates/physics/src/lod.rs | 844 | 3d6ee4a1 | no |
| crates/physics/src/query.rs | 1573 | 3d6ee4a1 | yes (rewritten) |
| crates/physics/src/simd_math.rs | 150 | 3d6ee4a1 | no |
| crates/physics/src/solver.rs | 2059 | 3d6ee4a1 | no |
| crates/physics/src/substep.rs | 784 | 3d6ee4a1 | no |
| crates/physics/src/vehicle.rs | 621 | 3d6ee4a1 | no |
| crates/physics/src/verify.rs | 70 | 3d6ee4a1 | no |
| crates/physics/tests/cloth_tape.rs | 111 | 3d6ee4a1 | no |
| crates/physics/tests/it/contacts.rs | 268 | 3d6ee4a1 | no |
| crates/physics/tests/it/deterministic_order_vectors.rs | 156 | 3d6ee4a1 | no |
| crates/physics/tests/it/g21_locked_hash.rs | 55 | 3d6ee4a1 | no |
| crates/physics/tests/it/island_sleep_commit.rs | 179 | 3d6ee4a1 | no |
| crates/physics/tests/it/island.rs | 83 | 3d6ee4a1 | no |
| crates/physics/tests/it/main.rs | 9 | 3d6ee4a1 | yes (rewritten) |
| crates/physics/tests/it/physics_broadphase.rs | 164 | 3d6ee4a1 | no |
| crates/physics/tests/it/physics_solver.rs | 136 | 3d6ee4a1 | no |
| crates/physics/tests/it/physics_substep.rs | 207 | 3d6ee4a1 | no |
| crates/physics/tests/it/seq_impulse_bench.rs | 165 | 3d6ee4a1 | no |
| crates/physics/tests/rb_query_kinds.rs | 497 | 3d6ee4a1 | no |
| crates/playtest/src/agreement.rs | 373 | f1a4f9d7 | no |
| crates/playtest/src/live_probe.rs | 92 | f1a4f9d7 | no |
| crates/playtest/src/look_judge.rs | 196 | f1a4f9d7 | no |
| crates/playtest/src/look_score.rs | 169 | f1a4f9d7 | no |
| crates/playtest/src/look.rs | 203 | f1a4f9d7 | yes (034b30aa) |
| crates/playtest/src/look/judge.rs | 2266 | f1a4f9d7 | yes (034b30aa) |
| crates/playtest/src/look/manifest.rs | 476 | f1a4f9d7 | yes (034b30aa) |
| crates/playtest/src/look/metric.rs | 1110 | f1a4f9d7 | yes (034b30aa) |
| crates/playtest/src/machine_eyes.rs | 192 | f1a4f9d7 | no |

Lines by crate: physics 14429, net 6625, game 5725, playtest 5077, nav 2900, destruction 2681, perf 1463, endpoint 4
