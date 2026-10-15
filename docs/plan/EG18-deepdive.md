# EG18 deep-dive 2026-10-14 (planner thinker for the stuck gate)

Row: EG18 game animation from sim state (board `docs/SPRINT-18-gaps.md:411-424`).
Tier A, Size L as written. Still open. FOCUS Now Look row (`docs/FOCUS.md:7`).

Rows: EG18 slice 1 (bridge caller, needs EG1, EG10 — both ticked), EG18b
slice 2 (per-slot compiled state machine, needs EG18), EG18c slice 3
(graph + layers sample, needs EG18b). Tier A, Size M each. All open.

## Symptom

Second identical oversize failure, zero landings. `docs/BLOCKERS.md:27`:
EG18 anim-crossfade abandoned at 233 changed lines over max_diff 200
twice; repro `target/abandoned/eg18-anim-crossfade-2026-10-11.patch`.
That patch's fade (`Fade { act, act_start, switched_at }`,
`FADE_TICKS = 9`, its own `fade_weight`) has since landed in a different
form as committed truth: `SlotAnim { prev_clip, prev_time, fade_left }`
(`crates/game/src/fighter_look.rs:191-217`), the blend in `slot_pose`
(`fighter_look.rs:1224-1239`) keyed on `FIGHTER_CROSSFADE_TICKS` from
`engine-anim`, speed-scaled run in `clip_time` (`fighter_look.rs:1189`),
and the additive pelvis flinch through `envelope`
(`fighter_look.rs:1250-1258`). The landed tests pin all three:
same-tick same clip and phase (`fighter_look.rs:2394`), midpoint blends
bit for bit at weight 4/9 (`fighter_look.rs:2423`), plus replay x2, run
scaling and flinch decay below. Resending any whole fade repeats the
abandon: it diffs over 200 lines against truth that already ticks.

## Root cause

Three conjuncts, none fixable by resending the fade:

1. Landed truth blocks the revert. The old patch needs a committed-truth
   revert to apply; the remedy line says so plainly. The fade behavior it
   wanted is already on HEAD in another shape, so the patch is
   superseded, not pending.
2. Unwired island beside the truth. `crates/game/src/anim_bridge.rs`
   (act pick plus `SlotFade` driver, `BridgeClip` pinning the look's 0-5
   clip order, own `fighter_anim_bridge_*` tests) has zero production
   callers: the only tree reference is `pub mod anim_bridge;`
   (`crates/game/src/lib.rs:24`). Its header promises "the fighter look
   (boarded next slice) reads" the pick — the next slice never came.
   Same shape as the FIX5/FIX3c BLOCKERS rows: helpers with no tick
   owner never tick a row.
3. The row's last sentence is still open. `FighterLook` picks acts from
   its own `Act` match (`clip_time`, `fighter_look.rs:1187-1200`), not
   from a compiled `state_machine` `Playback` per slot; the graph and
   layers paths (`crates/anim/src/graph.rs`, `layers.rs`,
   `hit_react::apply_hit_react`) sample nothing the window shows. The
   look hand-rolls both the blend and the flinch.

## Different approach

Stop resending the fade whole. Never revert the landed blend, run
scale, or flinch: the midpoint bit-for-bit test stays green under every
slice. One lane finishes in this order, each slice at most 200 changed
lines in `git diff --stat`:

1. EG18 slice 1 (keeps the ID, narrowed to the caller): give the bridge
   its production owner. The fold path builds one bridge slot per seen
   fighter and reads its `ClipPick` plus fade; sampling stays in
   `slot_pose`. Owns the `fighter_look.rs` fold hunk plus the
   `anim_bridge.rs` glue hunk. Size M. needs: EG1, EG10. F2P: new
   `fighter_anim_bridge_drives_look_pick` — same cues give bridge pick
   equal to look act, x2; fails before (the look never reads the
   bridge). P2P: `cargo test -p engine-game --lib fighter_anim` green
   x2; golden unchanged. Dispatch check: `git diff --stat` prints at
   most 200 changed lines and the F2P test exits 0.
2. EG18b slice 2 (needs EG18): drive the pick from a compiled machine.
   One `StateMachineDef` (idle, run, three attacks, down; transitions on
   fight flags and speed) evaluates per slot through `Playback`; the
   existing `rate_q8` parameter (`crates/anim/src/state_machine.rs:56-65`)
   carries the run scale, so no new call. Owns the `state_machine.rs`
   wiring hunk plus the bridge read hunk. Size M. F2P: new
   `state_machine_fighter_picks_from_flags` — flags plus speed give the
   exact state and rate, x2; fails before. P2P: anim lib green, game lib
   green, golden unchanged. Dispatch check: same 200-line plus F2P
   exit 0 gate.
3. EG18c slice 3 (needs EG18b): sample through the graph plus layers.
   `slot_pose` evaluates the compiled graph for leaving and current at
   the playback weights and carries the react through the layers
   additive path; the landed midpoint and flinch tests pin the pixels.
   Owns the `fighter_look.rs` sample hunk plus the graph/playback hunk.
   Size M. F2P: new `fighter_anim_graph_sample_matches_hand_roll` —
   graph sample equals the hand-rolled blend bit for bit, x2; fails
   before. P2P: game `--features app` lib green, golden unchanged.
   Dispatch check: same 200-line plus F2P exit 0 gate.

Out of scope on purpose: the swing still starts on the outcome cue 12
ticks late (`IMPACT_TICKS`, `fighter_look.rs:55,1161`) — that is EG19c
(needs EG19b), not this lane. Consumers already wait with `needs: EG18`:
LF9 inertialized switches, LF12 planted feet, EG40 sockets, AM23 flinch.

## Proof command

`cargo test -p engine-game --lib fighter_anim` green x2 (same tick same
clip and phase; midpoint blends bit for bit), then
`cargo test -p engine-game --lib` green with its count pasted, golden
`d57b5c8ae3096614` unchanged. F2P per board: no crossfade test
(Superseded: the crossfade tests exist; the missing F2P is the
slice-1 bridge-drives-pick test). P2P: game lib green; golden unchanged.
Judge verify per slice is that slice's F2P plus P2P line, each expected
exit 0.

Donor (pattern only, reimplemented, no vendored code):
godotengine/godot @ `c640059ece20632c9514a760828828df86372353` (via
api.github.com commits listing for
`scene/animation/animation_node_state_machine.cpp`) — the transition
vocabulary and shortest-path `travel` that `state_machine.rs:10-13`
already ports; slice 2 follows that shape.
