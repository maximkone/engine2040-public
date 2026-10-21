# Stale 16-fighter sweep (cap raised 16 -> 32)

Read-only audit at `718684d1` plus the dirty tree (103 changed paths; other lanes in flight).
No file was edited, no test run, no commit.

Cap: `engine_input::MAX_PARTICIPANTS = 32` (`crates/input/src/fighter.rs:29`, asserted at `:253`,
`:379`, `crates/input/src/lib.rs:82`, `crates/ui/src/lib.rs:1919`, `crates/combat/src/sweep.rs:1139`,
`crates/arena/src/lib.rs:504`, `crates/game/src/director.rs:272`).
The owner decision is on the board: `docs/plan/runtime.md:299` (GE12, "Owner decision 2026-10-20: 32").
Where a consumer derives its size from `MAX_PARTICIPANTS` it is correct and does not appear below
(arena `MAX_SLOTS`, combat `SLOT_COUNT`, net `FighterSnap`, game `hp: [u16; MAX_PARTICIPANTS]`,
input tapes, guide/menus/lobby slot lists).

Legend: **stale** = the 16 was a mirror of the old cap and is now wrong (a bug or a lie);
**stale-bar** = a fixed 16 fixture/bar that was pinned to the old cap and must be re-based;
**deliberate** = a real, unrelated 16 that must stay (reason given).

## Table

| file | line | what it assumes | verdict | the reason | who owns it |
|---|---|---|---|---|---|
| `crates/ecs/src/spine.rs` | 32 | `MAX_SPINE_SLOTS = 16`, doc: "the input contract's participant cap" | **stale** | The doc claim is false. `spine.spawn` refuses past it (`:394`), `with_capacity` clamps (`:359`), `restore` rejects a 32-slot snapshot (`SpineError::TooManySlots`, `:561`). A 32-fighter match cannot have a spine and cannot restore one. | engine-ecs (ecs lane) |
| `crates/perf/src/workloads.rs` | 116 | doc: "`MAX_PARTICIPANTS` (16, the engine maximum)" | **stale** | `config()` at `:137` now passes the real `MAX_PARTICIPANTS` (32), so the comment lies and the "canonical" run silently changed shape. | engine-perf (perf lane) |
| `crates/perf/src/workloads.rs` | 1158 | `scaled_sixteen_matches_the_sixteen_slot_default`: 16-run hash == default run hash | **stale** | Default is now a 32-fighter run, so `ScaledArenaTick::canonical_for(16).run_hash() != ArenaTick::canonical().run_hash()`. This test is red. | engine-perf (perf lane) |
| `crates/perf/src/allocator.rs` | 185, 359 | `system_rows()` builds the arena row at `fighters: 16` and pins it equal to "the canonical sixteen-fighter run" | **stale** | Same mismatch as above; `system_arena_matches_canonical_sixteen` is red and the allocator/candidate comparison rows compare against a shape production no longer runs. | engine-perf (perf lane) |
| `crates/perf/src/baselines.rs` | 40 | `arena_tick` baseline `hash: 12857399269765116608`, `alloc_bytes: 63539` | **stale** | Recorded on the 16-fighter run; `check_report` now compares a 32-fighter run, so the budget gate fails even on an unchanged machine. The row needs re-recording (with the seal receipt), not a code change. | engine-perf (perf lane) |
| `crates/perf/src/workloads.rs` | 1318 | `assert_eq!(MAX_SPINE_SLOTS, 16)` | **stale** | Pins the stale ecs cap from the perf side: fixing `MAX_SPINE_SLOTS` breaks this test, so the two must move together. | engine-perf + engine-ecs |
| `crates/perf/src/workloads.rs` | 833 | `ARENA_MATRIX_COUNTS = [4, 8, 16]` | deliberate-gap | A named matrix whose rows each have a baseline; 16 stays a legitimate row. The gap is that no 32 row exists; that is a new row, not a wrong 16. | engine-perf (perf lane) |
| `crates/game/src/lib.rs` | 2991 | `assert_eq!(dst.fighters.len(), 16)` after filling `0..MAX_PARTICIPANTS` | **stale** | The loop above pushes 32 HUD rows (cap `MAX_PARTICIPANTS`), so this asserts 16 against 32. Red test in engine-game. | engine-game (game lane) |
| `crates/game/src/lib.rs` | 2177 | comment: "5 fighters fits the 16-slot cap" | **stale** | The admitted range is `2..=32` (`:258`); the comment misstates the gate it is testing. | engine-game (game lane) |
| `crates/game/src/capture.rs` | 1943 | tape slot byte `16` must be refused ("unknown fighter slot") | **stale** | Slot 16 is a legal fighter now (`0..MAX_PARTICIPANTS`), so `decode_tape_bytes` accepts it and `assert!(..is_err())` fails. Red test. The negative case needs a slot >= 32. | engine-game (game lane) |
| `crates/game/src/windowed.rs` | 1092 | `const NAMES: [&str; 16]` for `duel_overlay_for_app` | **stale** | `take(live)` runs to `config().fighters` (up to 32) over a 16-name array, so slots 16-31 get no HUD row at all. Silent, not a panic. | engine-game (game lane) |
| `crates/game/src/windowed.rs` | 3325, 3329 | `duel_overlay_caps_at_sixteen_with_stable_names`, `fighters: 16` | **stale-bar** | The test passes only because it asks for 16; the name asserts the old cap as the contract. It should ask for `MAX_PARTICIPANTS` and prove 32 rows. | engine-game (game lane) |
| `crates/game/src/windowed_captions.rs` | 44, 133 | comments: "outside the 16-fighter range" | **stale** | The code uses `FighterId::new` (cap 32); only the prose is wrong, so a reader re-derives a 16-cap from it. | engine-game (game lane) |
| `crates/game/src/siege.rs` | 60 | `SHARD_FIGHTERS = 16`, 4 shards = 64 slots | **deliberate** | A shard size for a 32v32 composition, documented at `:1-21` as "a shard size, never a reopened cap". `MAX_PARTICIPANTS` is untouched by it. | engine-game (game lane) |
| `crates/ui/src/lib.rs` | 637 | `MAX_OVERLAY_ROWS = 16`, doc: "matches the `engine-input` slot cap"; `OVERLAY_MAX_BYTES = 21 + 16*28` | **stale** | The doc claim is false. `upsert_fighter` refuses row 17 (`:1143`) and `duel_overlay_for_app` swallows the error (`let _ =`), so a 32-fighter match silently shows 16 names. The wire bound moves with it, so this is a protocol change, not a one-line edit. | engine-ui (ui lane) |
| `crates/ui/src/lib.rs` | 39 | `MAX_MAP_CAP = 16` minimap dots | **stale** (latent) | Capacity is derivable from the cap; at 32 fighters `upsert_dot` returns `MapFull` (`:360`) and the extra dots vanish. No live caller today (only tests), so latent. | engine-ui (ui lane) |
| `crates/ui/src/access.rs` | 383 | `sixteen_fighter_storm()`: every ordered pair of 16 = 240 cues | **stale-bar** | UIX9's bar was written at the old cap; a 32-fighter storm is 992 cues, the number that actually tests the queue bound. | engine-ui (ui lane) |
| `crates/ui/src/lobby_ui.rs` | 1109-1123 | ghost storm over 16 hosts, notice `count: 16` | deliberate | `max_hosts` is `MAX_PARTICIPANTS` (32) at `:1098`, so 16 is a legal fixture well inside the cap. The two tests this lane already fixed were the 48-unit / 4x-scale asserts, not this one. | engine-ui (ui lane) |
| `crates/app/src/snapshot.rs` | 23, 231 | `MAX_FRAME_ACTORS = 16`, doc: "(the arena participant cap)"; decode refuses `count > 16` | **stale** (latent) | The claim is false. A 32-actor `PresentationFrame` now refuses to encode/decode. Only tests call `PresentationFrame::new` today, so latent. `crates/app/src/tests.rs:591` pins the 16 and moves with it. | engine-app (app lane) |
| `crates/render/src/overlay.rs` | 377-388 | `hp_bar_rect` clamps `slot.min(15)`; "all 16 rows fit under y 1.0" | deliberate-now | Consistent with a 16-row overlay and a 0.04+0.01 stack. It becomes wrong (bars 16-31 all paint row 15) the moment `MAX_OVERLAY_ROWS` grows, so it must move with that const. | engine-render (render lane) |
| `crates/combat/src/sweep.rs` | 8 | doc: "over <=16 participants" | **stale** | The code validates against `MAX_PARTICIPANTS` (`:435`, `:1584`); only the module budget line still quotes 16. | engine-combat (combat lane) |
| `crates/combat/src/exchange.rs` | 1789 | comment: "the full 16-slot cap initializes" | **stale** | `SLOT_COUNT` is `MAX_PARTICIPANTS` (32); the test seats 6. Comment only. | engine-combat (combat lane) |
| `crates/combat/src/fight.rs` | 3892 | doc: "16-fighter fixture" | **stale** | The fixture is a fixed 16-pose array (`:3912`, `:3924`) used as a brawl sample; valid as a fixture, but it is labelled as the full size and is not. | engine-combat (combat lane) |
| `crates/combat/src/cut.rs` | 27, 29 | `MAX_CUTS_PER_SWING = 16`, `MAX_TARGETS_PER_SWING = 16` | deliberate | 16 joint capsules of one body (`anatomy::JOINT_TOTAL = 16`), not fighters; a swing resolves one skeleton. | engine-combat (combat lane) |
| `crates/arena/src/spear.rs` | 800 | doc: "slots are below 16, so the fallback never fires" | **stale** | Slots now reach 31; the `unwrap_or(u8::MAX)` fallback is now reachable by a wrong caller, and the doc saying otherwise hides that. | engine-arena (arena lane) |
| `crates/arena/src/eqs_bind.rs` | 71 | "fixed covers (millimetres, `0..16`)" | deliberate | Arena cover boxes, not fighters. | engine-arena (arena lane) |
| `crates/rules/src/arena.rs` | 67 | doc: "the arena has at most 16" | **stale** | The filter is `< MAX_FIGHTERS` (64, `limits.rs:7`) and the arena may carry 32, so the doc undercounts both. | engine-rules (rules lane) |
| `crates/net/src/delta.rs` | 781, 809, 823 | "a full 16-slot snapshot", "omits fifteen", "Slot 16 sits outside the canonical cap" | **stale** | Code is parametric (`0..MAX_PARTICIPANTS`) and the negative case already uses `MAX_PARTICIPANTS`; the comments are the leftovers and the last one is now actually false. | engine-net (net lane) |
| `crates/net/src/scale.rs` | 675-678, 1125 | `FLOOD_SLOTS = 16`, `FLOOD_VIEWS = 16`, chart row `fighters: 16` | **stale-bar** | The MP8/MP9 flood is still the 16-fighter shape; the chart also carries a 32 row (`:1132`), so the bar and the chart disagree about the cap. `crates/net/src/session.rs:2390` repeats the 16 row. | engine-net (net lane) |
| `crates/net/src/interest.rs` | 1753 | 16-fighter replication fixture | **stale-bar** | Rate-gated interest is exactly what a 32-slot roster stresses; the fixture stops at the old cap. | engine-net (net lane) |
| `crates/net/src/lobby.rs` | 28 | `MAX_BEACONS = 16` | deliberate | Beacon-browser ring (`entries: [Option<SeenBeacon>; 16]`, `:887`), a listing bound, never a match cap. The 32-slot AI mask is already `u32` (`:1103`, `:1113`). | engine-net (net lane) |
| `crates/web/src/input.rs` | 797 | `s.fighter_input(0, FighterId(16), 0)` must fail `UnknownFighter(16)` | **stale** | Slot 16 is legal, so the call succeeds and `.unwrap_err()` panics. Red test in engine-web. | engine-web (web lane) |
| `crates/input/src/fighter.rs` | 418 | `UnknownFighter(16).range_detail() == 16` | deliberate | The 16 is an error payload, not a cap; the constructor is not used as a slot. | engine-input (input lane) |
| `crates/proof/src/fb_pillars.rs` | 47, 67 | `PERCEPTS: [[f32; 4]; 16]`, `FOE_ATTACKING: [bool; 16]` | deliberate | 16 rows of a fixed percept tape (one per tick), not 16 fighters. | engine-proof (proof lane) |
| `crates/destruction/src/budget.rs` | 41 | 16 shards per event | deliberate | Per-event fracture budget of the 8-fighter soak. | engine-destruction (destruction lane) |
| `crates/save/src/lib.rs` | 13 | `SLOTS = 16` | deliberate | Save-file slots on disk, unrelated to fighters. | engine-save (save lane) |
| `crates/ecs/src/bridge.rs` | 280 | `id: [u8; 16]`, `parent: [u8; 16]` | deliberate | 128-bit stable scene ids. | engine-ecs (ecs lane) |
| `crates/game/src/fighter_look.rs` | 586, 632 | `base + slot * 16 + part`, "keys reserve 16 per slot" | deliberate | 16 body parts per fighter (atlas quadrants); stride, not capacity. | engine-game (game lane) |
| `crates/assets/src/bc.rs` | 16 | `BLOCK_TEXELS = 16` | deliberate | BC block size. | engine-assets (assets lane) |
| `crates/render/src/crowd_inst.rs` | 1246 | `take(16)` / `skip(16)` over chant blocks | deliberate | Crowd sectors per side, not fighters. | engine-render (render lane) |
| `crates/game/tests/it/config_vectors.rs` | 121, 129 | a `fighters` row valued `16` | deliberate | A legal value inside `2..=32`, used as a parse vector. | engine-game (game lane) |
| `docs/FOCUS.md` | 59 | "duels and team fights, 2-16 fighters" | **stale** | The shipped gate is `2..=MAX_PARTICIPANTS` (32); FOCUS is the page builders read first. | docs/planner |
| `docs/plan/rome-spec.md` | 43 | "**Team** 4-16, even sides" | **stale** | Mode spec still caps teams at the old 16; the engine now seats 32. | docs/planner |
| `docs/plan/quality.md` | 98 | "`ArenaTick` 16 slots seed 1" | **stale** | This is the sentence that made the perf shape look unchanged; it now contradicts `workloads.rs:137`. | docs/planner + perf |
| `docs/plan/runtime.md` | 42, 114, 119 | "8-window on a 16-slot bout", "camera frames 16 slots", "a 16-slot loopback soak" | **stale** | GE19/GE13/soak rows still size their proof at the old cap; `:299` (GE12) already rules 32, so the same file disagrees with itself. | docs/planner |
| `docs/plan/multiplayer.md` | 476 (also 148, 150, 334, 348, 465, 467) | MP16a "16-fighter LAN soak baseline within the current cap"; MP17/MP23 accept a 16-fighter jitter fixture | **stale-bar** | "the current cap" is 32; the 16 bar was the MP16a holding pattern until GE12, which landed 2026-10-20. The fixtures are still 16 so the accepts no longer prove the cap. | docs/planner + net |
| `docs/plan/parked.md` | 173, 174 | UIX8 "16-slot list virtualized", UIX9 "16-fighter storm" | **stale** | Both rows were written against the old cap; UIX8's test now runs at 32 (fixed in `718684d1`), so the row text lags its own proof. | docs/planner + ui |
| `docs/design/README.md` | 28 | "`MAX_ATTACK_STARTED_PER_TICK` = SLOT_COUNT (16 cues, hard bound)" | **stale** | `SLOT_COUNT` is `MAX_PARTICIPANTS` = 32 (`crates/combat/src/fight.rs:27`, `:451`), so the stated hard bound is half the real one. | docs/planner + combat |
| `docs/book/ref-arena-combat.md` | 195 | "`count` 1..=16. Fields `fighters: [FighterState; 16]`" | **stale** | The same page already says 32 at `:41` and `:328`; the API reference row is the leftover and would mislead any caller. | docs/planner + combat |
| `docs/book/ref-net.md` | 32, 214 | "8 to 16 fighter hardening", "8 to 16 fighter bandwidth caps" | **stale** | `scale.rs` charts a 32 row (`:1132`), so the book's own subject is wider than its summary. | docs/planner + net |
| `docs/plan/steals.md` | 32 | PF2 off-path bar measured on "ArenaTick canonical 16-slot 600-tick" fixture | **stale-bar** | The canonical run is 32 slots now; the bar's frame changed under it. | docs/planner + perf |

## What would break (or already breaks) with 32 in a 16-sized slot

No memory-unsafe overflow exists: every 16-slot holder is a `Vec`/slice with a guard, so the failure
mode is refusal or silent truncation, not a panic or UB. Ranked by damage:

1. **`crates/ecs/src/spine.rs:32` — hard refusal of the whole ECS path.** `spawn` returns
   `SpineError::Full` at the 17th fighter and `restore` rejects any snapshot with more than 16 slots
   (`:561`). `crates/arena/src/spine_step.rs:107-131` builds `Spine::with_capacity(config.fighters)`
   and `.ok()?` on each spawn, so a 32-fighter match yields `None` — no spine at all, silently, from
   the caller's point of view.
2. **`crates/perf/src/baselines.rs:40` + `workloads.rs:116` — the perf gate is measuring a shape it
   no longer runs.** The 32-fighter canonical run cannot match a 16-fighter baseline hash or alloc
   count, so `check_report` fails and two perf tests (`workloads.rs:1158`, `allocator.rs:359`) are red.
3. **`crates/ui/src/lib.rs:637` — the HUD overlay drops half the roster silently.** Row 17 is refused
   and the error is discarded at `crates/game/src/windowed.rs:1102`, so fighters 16-31 have no name or
   HP row in a match that is supposed to seat them.
4. **`crates/game/src/windowed.rs:1092` — the 16-name table.** Even if the overlay cap moved, the name
   source stops at F15, so slots 16-31 stay unnamed. Same silent class as 3, one layer down.
5. **`crates/app/src/snapshot.rs:23` — `PresentationFrame` refuses 16+ actors** (`:231`), so the
   editor/consumer frame of a 32-fighter match cannot encode or decode. Latent: only tests build a
   frame today.
6. **`crates/ui/src/lib.rs:39` — minimap stops at 16 dots** (`MapFull`), fighter 17-31 have no blip.
   Latent: no production caller of `upsert_dot` yet.
7. **`crates/render/src/overlay.rs:388` — HP bars clamp to row 15**, so rows 16-31 would all stack on
   the last row rather than fail. Purely cosmetic and only after the overlay is raised.

Red-at-HEAD tests caused by the cap, not by the 16-buffer class (same root cause, listed so they are
not mistaken for lane breakage): `crates/game/src/lib.rs:2991`, `crates/game/src/capture.rs:1943`,
`crates/web/src/input.rs:797`, `crates/perf/src/workloads.rs:1158`,
`crates/perf/src/allocator.rs:359`.

## What the cap change should have touched and did not

Raising `MAX_PARTICIPANTS` from 16 to 32 was done as a constant change plus a pass over the crates that
read the constant (input, arena, combat, game config, menus, lobby, net codec, director) — every one of
those derives its storage from `MAX_PARTICIPANTS` and is correct. What it did not touch is the ring of
consumers that had *copied* the old cap into their own constant or had pinned it in a test, and those
sit exactly one layer outside the sim: `engine-ecs` (`MAX_SPINE_SLOTS`), `engine-app`
(`MAX_FRAME_ACTORS`), `engine-ui` (`MAX_OVERLAY_ROWS`, `MAX_MAP_CAP`), `engine-game`'s name table and
HUD count assert, `engine-web`'s negative input case, and `engine-perf`, where the canonical workload
now runs 32 fighters against a baseline recorded at 16 — which is why the perf gate and two perf tests
are red while the sim itself is fine. The same change also left the *documents that state the cap*
behind: FOCUS, the Rome mode spec, the quality/workload description, the net and combat book pages and
several live plan rows still say 16 (or "at most 16"), some in the same file whose GE12 row records the
owner's 32 decision. The general lesson to record: the cap should be asserted where it is used
(`const _: () = assert!(MAX_OVERLAY_ROWS == MAX_PARTICIPANTS as usize);`-style, as `menus.rs:33` already
does for `MAX_MENU_FIGHTERS`), so the next change of the number fails a build instead of quietly
shrinking a buffer.

## What the lead did with the doc rows (2026-10-21)

The code half of this sweep is the cap-fix lane's receipt (`fix-cap-16-to-32.md`). The doc half was the
lead's, because the plan and book pages are the lead's. Every number below was checked against the
code before it was written; no bar was renumbered without saying so.

| doc site | what it said | what it says now | why |
|---|---|---|---|
| `docs/FOCUS.md:59` | "duels and team fights, 2-16 fighters" | "2-32 fighters" | the shipped gate is `2..=MAX_PARTICIPANTS` |
| `docs/plan/rome-spec.md:43` | "**Team** 4-16, even sides" | "4-32" | the engine seats 32 |
| `docs/plan/quality.md:98` | "`ArenaTick` 16 slots seed 1" | "32 slots" | `workloads.rs:135` now says `MAX_PARTICIPANTS` slots |
| `docs/design/README.md:28` | "`MAX_ATTACK_STARTED_PER_TICK` = SLOT_COUNT (16 cues)" | "32 cues" | `SLOT_COUNT` is `MAX_PARTICIPANTS` (`crates/combat/src/fight.rs:27`, `:451`) |
| `docs/book/ref-arena-combat.md:38`, `:93`, `:195` | `teams: [u8;16]`, `v2_teams(teams: [u8;16])`, "`count` 1..=16, `fighters: [FighterState; 16]`" | 32 in all three | `ArenaRules.teams` is `[u8; MAX_SLOTS]` and `FightState.fighters` is `[FighterState; SLOT_COUNT]`, both 32 |
| `docs/book/ref-net.md:32`, `:214` | "8 to 16 fighter hardening", "8 to 16 fighter bandwidth caps" | "8 to 32" | `crates/net/src/scale.rs` charts a 32 row (`:1132`) |
| `docs/plan/runtime.md:42` | "Provider observation 8-window on a 16-slot bout" | "32-slot bout" | the bout is sized by the cap |
| `docs/plan/runtime.md:114` | "camera frames 16 slots without heap" | "camera frames the full roster without heap (last measured at 16 slots, the cap is 32)" | the camera is roster-sized, but no 32-slot camera test exists yet, so the bar says what was measured |
| `docs/plan/runtime.md:119` | "a 16-slot loopback soak converges" | same, plus "measured at 16; a soak at the 32 cap is still owed" | the soak ran at 16; renaming the number would fake a measurement |
| `docs/plan/multiplayer.md:476` | "16-fighter LAN soak baseline within the current cap" | "16-fighter LAN soak baseline (the pre-GE12 holding bar; the cap row is MP16b)" | the current cap is 32, so the old title was false; MP16b already exists for the cap |
| `docs/plan/parked.md:173`, `:174` | UIX8 "16-slot list virtualized", UIX9 "16-fighter storm" | "32-slot list virtualized", "full-roster (32) storm" | the ui tests now run at `MAX_PARTICIPANTS` (the overlay cap is `PARTICIPANT_CAP` with a compile-time assert) |

Left alone on purpose: `docs/plan/steals.md:32` (PF2). Its "16-slot 600-tick fixture" is the history of
a measurement that was really taken, and the same file records PF2 as landed. The honest fix is the new
32-fighter row in the perf baselines, which the cap lane appended (`PF11`, append-only).

