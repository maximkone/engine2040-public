# Finish line: what "done" means, as numbers the loop prints itself (owner 2026-10-11)

Owner 2026-10-11: "a really visually good AI-native game engine from the core", "why i got
to babysit this". So the finish line is numbers that code prints with no owner and no model
judging by eye, and the loop climbs toward them in a fixed order. `tools/product-bar.json`
holds the floors (a broken one files a `bug:` packet by itself) and the targets below.

## The gates

| Gate | Done when | Printed by | 2026-10-13 |
|---|---|---|---|
| G1 Never broken | product check GREEN on every code tree for 7 days: no empty frame, red strobe, blank or frozen capture, wgpu validation error, hang, panic or stall; no NaN vertex, camera in a body or under the sand, deep sink, or fight played unseen (`arena view:`) | `tools/product-check.mjs` (the keeper runs it) | GREEN 2026-10-13 (stale build): blank 0, frozen pairs 0, validation errors 0, tinted 0.04; nonfinite 0, eye in body 0, eye below ground 0 |
| G2 Speed | 60 fps p50 at 1080p, 8 fighters, RTX 3050 6 GB; 30 first (FC3) | product check timing run, perf seat (FC1) | 120.1 fps p50 2026-10-13 (8.323 ms, p99 13.898 ms, GPU 0.327 ms), still unlit; 1080p and the lit frame not measured yet |
| G3 Lit frame | sky, IBL, sun shadow, GTAO, bloom, TAA, particles, decals: each pass changes pixels on vs off (EG8) | product check "Live GPU passes", EG8 test | live GPU passes: none 2026-10-13; ACES and height fog only, the rest never reaches the window |
| G4 Bodies read as people | skin, metal and cloth shade differently (LF8); textured armor, no box bodies; machine look score at the bar | look judge (EG30, LJ1, LF15 FLIP) | one BRDF, base color |
| G5 Motion and feel | no pops on clip switches, feet planted, hits land (hold, shake, trail) | anim tests and the `arena view:` line (`pops`, `sunk_cm_max`) | 128 pops per 300 frames 2026-10-13 (attack 12, idle 50, run 66; max 128.7 cm); sunk_cm_max 0 this run |
| G6 Same hash | break corpus x2 and the HS2 matrix | Core proof 2 | open |
| G7 AI door | same task, same result on mock, local and remote; play.step_until, input.inject, state.digest; asset jobs offline under 6 GB | Core proof 3, LF16 | 75/75 recorded |
| G8 Multiplayer | the duel on LAN and internet, one hash | Core proof 4 (last) | open |
| G9 Potato (owner 2026-10-11) | the same arena runs on this PC's Intel UHD with downlevel limits: proposal p50 50 ms, p99 80 ms, working set 3 GB; heap peak does not grow; assets stay inside the potato caps | `node tools/potato-check.mjs` (GS16), `crates/perf` heap peak (GS14), assets budget (GS15) | not measured on the Intel UHD yet (2026-10-13 run: adapter UNKNOWN, NO_IGPU, exit 2) |

## The order (docs/FOCUS.md Now holds the rows; this is why)

1. Light the frame (weeks 1-2): wire what exists. LF1 sky, LF2 IBL, EG11 sun shadow,
   EG12 ambient and maps, LF3 GTAO, LF4 bloom, with FC1/FC3 speed beside it. A steal
   replaces an island only when its on vs off test shows it is wrong; nothing counts until
   the window shows it.
2. Bodies and feel (weeks 2-4): LF10 hitstop holds poses, LF17 the dead lie on the sand, LF11 impact shake, EG18 then LF9
   inertialized blends, LF12 foot and hand IK, LF13 look-at, LF14 trails, LF8 skin, metal
   and cloth, LF5 TAA, LF6 dust and sparks, LF7 blood decals.
3. Machine eyes and polish (weeks 4-8): the view truth (SP61 no window, SP62 door verbs,
   SP63 GPU numbers instead of PNGs, SP64 props and light, SP65 picks and sight), EG30 then
   LF15 FLIP regression score, raise the
   bar's targets as they are met, LF16 door verbs, editor asset jobs (TripoSR or Stable
   Fast 3D, MoMask clips) once the frame is lit. Multiplayer stays last.

## Who does what

- Owner 2026-10-11: "add all to plans so sprints will do it". Seats skip `Diff: hard`, so
  every hard row is split into loop-sized slices that keep the parent ID as slice 1 (EG11
  b-c, LF5 b-d, SP63 b-c, EG19 b-c, EG46 b, AL1 b): one crate, at most 200 changed lines, a
  named file or donor, one test that fails before. A slice that fails review twice is split
  smaller with the review's reason in the new slice; no row waits for a paid session.
- The keeper's product check is the loop's eyes: it runs the built game after every code
  change and files `bug:product-<metric>` packets, which run right after `loop:` fixes.
- Walkers judge the product check's fresh captures (`target/reports/product/runs/`).

## Sources

`docs/research/steals-2026-10-11/`: `render-steals.md` (shader files to port, build order),
`anim-feel-steals.md` (springs, inertialization, IK, shake, spring bones),
`arxiv-neural.md` (FLIP for the judge; offline motion models; what to skip),
`ai-native.md` (door verb gaps from the Godot, Unity, Unreal and Blender MCP servers;
asset models that fit 6 GB).

## Plans

Look on the RTX 3050: gates G3-G4 light the frame and score it on O5.
Sim speed: gate G2 60 fps budgets with BENCH beside it.
Determinism and exact replay: gate G6 same-hash corpus x2.
AI builds the game: gate G7 same task same result on mock, local, remote.
Online, 64 players: gate G8 duel on LAN and internet with one hash.
Shipping a clean build: gates G1 never-broken plus G9 potato on Intel UHD.
Headless proofs and tests: product check receipts with no hollow green.
