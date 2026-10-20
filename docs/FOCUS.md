# FOCUS — Rome at war, online, looking like 2040. Arena first, then the city and the siege field, on Windows and in the browser.

## Now (owner 2026-10-10; item 9 owner 2026-10-17): Core proofs, engine depth, web and realism, multiplayer last

Wins over everything below and over `SPRINT.md`, `ROADMAP.md`, `PLAN-REVIEW.md`. `node tools/board-slice.mjs --focus` prints the ready ids after `Rows:` in this section, in order, plus the rows they wait on. A `Rows:` line lists open rows only: when you tick a row, drop its id here in the same commit. Groups 4 to 9 run beside 1-3 in other crates, not after them. What "done" means: `docs/plan/finish-line.md` (gates G1-G11 and the 2026-10-17 gap list).

1. **Look:** moving fighter captures on the RTX 3050 vs references, scored by machine, at playable speed. Proof: `node tools/finish.mjs` A4 A5 A8. Rows: FC1 LF2 EG12 EG13 RL21 FC3
2. **Bodies:** cuts, limbs, ragdoll, collapse, fire: one hash on every PC and replay. Proof: `cargo test -p engine-proof break_corpus` x2, HS2 matrix. All rows ticked (EG45 EG19b EG46 HS2); keep the proof green.
3. **AI door:** same task, same result on mock, local, remote; 30 tasks vs rivals; a live model plays inside the tick and replays without it. Proof: `cargo test -p engine-playtest --lib mirror` x2. All rows ticked (SP61 SP64 HS29 HS30 HS31 HS20 AL1 DP1 RB1 LF16); keep the proof green.
4. **Engine depth:** embedded AI (provider cassettes and structured output, then voice, memory recall, the vision judge, local inference sharing the 6 GB GPU), spatial reasoning, fighter brains and nav, render quality. Proof: each row's Accept. Rows: OPT27
5. **Feel (owner 2026-10-11):** hits land, bodies move like people. Proof: each row's Accept; AM24 owns the 258-264 pops residual (attack 62 / idle 68 / run 128 in the 8-fighter 300-frame product check). Rows: LF14 AM24
6. **Multiplayer (last, owner 2026-10-10):** arena duel played by 2+ on LAN and internet, one hash. Proof: `cargo test -p engine-net --lib snapshot`; played bouts MP6 ON30. Rows: ON30
7. **Potato tier and steals (owner 2026-10-11):** every game must look great and run on a weak PC (weak GPU, RAM and CPU; the 3050 / 16 GB box is the comfortable tier). Wiring gates first, then the potato gate G9, then content fetch and the small steals. Proof: each row's Accept; potato: `node tools/potato-check.mjs`. Research: `docs/research/steals-2026-10-11/engine-landscape.md`, then the audit in `docs/research/steals-2026-10-20/wave-3.md`. Rows: GS2 GS43 GS44 GS45
8. **Game (owner 2026-10-12, 30 books: "a lot of game design", "full game dev teams"):** the duel plays like a designed game: attacks commit and read, opponents climb in tiers, walls and zones matter, rounds and a HUD; engine rows from the books beside them. Plan: `docs/plan/gameplay.md` sections 0-3. Proof: each row's Accept; golden unchanged (new rules are v2, the proof scene keeps v1). All rows ticked (GP27 GP28 GP29 GP30 GP31 GP37 GP39 GP41); next rows below.
9. **Web and realism (owner 2026-10-17: "heavy games on web, multiplayer, destruction, ragdoll, dismembering, realistic graphic and collision; ai sets it up easy; the arena looked meh"):** the same game on Windows and in a browser tab, with a physical layer (ragdoll, severed limbs, debris) painted from authority events, a lit arena with real materials, and an AI-driven project setup. Plan: `docs/plan/web-and-realism.md` (the two physics layers, the web design, the waves); rows in `docs/SPRINT-22-web-realism.md`. Proof: the golden tape equal on four targets incl. wasm, a browser frame beside the native frame, every look pass LIVE, physfx budgets in a soak. Rows: WEB6 RZ4
10. **Finish line gaps (2026-10-17):** what the engine still lacks for one game on Windows and web, AI-native, hard systems in one call: 23 ranked builds in `docs/plan/finish-line.md` "What we still lack", evidence in `docs/research/finish-line-2026-10-17/`. Take them in order when no row above fits your crate: (1) the page draws the real cooked fight, not stand-ins (WEB17), with parry/block cues (WEB16) and a browser frame/SSIM gate (WEB5); (2) finish the lit frame (AO prepass, bloom/TAA post chain, particles and decals, live cull/sort/instancing); (3) a machine look score (reference captures + SSIMULACRA2 `view.compare` verb). Steals first: box3d-rust (bit-exact physics oracle), moq-dev/web-transport (one net layer native + browser), wgpu `STRICT_WEBGPU_COMPLIANCE` CI leg, rust-av/ssimulacra2.

Next: OPT36 first (build speed); GP42 GP33 GP36 GP40 GP34 GA15 and the book rows (`docs/SPRINT-20-game.md`); look and feel steals `docs/SPRINT-21-look-feel.md`, first RL60 RL34 RL46 RL52 RL53 GP43 OPT35 PF25. `--focus` empty: take at most 2 other ready rows from `--ready`.

History: the 10-10 notes, the old Now and the delivery plan sit verbatim in `docs/archive/focus-notes-2026-10-15.md`, `docs/archive/focus-old-now-2026-10-10.md` and `docs/archive/focus-delivery-2026-10-11.md` (no authority).

## Core (owner 2026-09-30; wins over everything below where they differ)

Heavy graphics, animation, ragdoll, destruction and the other hard systems, each easy for an AI to drive through the engine's own MCP door. Four proofs a stranger can see:

1. **Look:** moving captures on the RTX 3050 judged by eyes beside the references (Q1, 0 of 10 today).
2. **Bodies:** animation, ragdoll, cuts, collapse and fire that give the same hash on every PC and in replay (BX28, none today).
3. **AI door:** the same task gives the same result on any model, a weak one, and none in replay; scores printed beside Unreal, Unity and Godot MCP (HS20, UNKNOWN today).
4. **Multiplayer (owner 2026-10-03: "engine2040 need multiplayer heavy"):** the arena duel played by 2+ people over LAN and internet, server-authoritative with rollback, AI filling empty slots, the same hash on every PC and in replay (rows MP and ON; steals in order: bevy_ggrs / ggrs rollback, lightyear replication, both MIT or Apache-2.0).

Speed (OPT, PF) serves them. One scene proves all four, the arena duel: a writer row that moves no pixel, hash or timing in it waits.

Engine to 10/10 (owner 2026-10-04: "steal, don't reinvent; better than Flax in materials; Blender capabilities needed for game dev"; decisions D1-D7 were made outside this repo and are kept here as text).
- D1 DEAD 2026-10-06 (D8, D9; text in archive) updated 2026-10-07: own crates/render draws, Bevy is a donor to steal code from.
- D2 Game rules an AI can write: a Rhai sandbox (integers only, op and size limits, script hash in the tape). This lifts the RC5 "data first, no scripting" deferral.
- D3 Asset generation lives in the editor as an off-tick job (free, commercial-use models only; cooked, hashed, provenanced files). The shipped game never generates (owner D3: RESEARCH-VERDICT.md `ai-makes-assets-inside-shipped-game: DROP`); `frontier.md` pillar 5 covers rough prop-mesh intake only.
- D4 Modeling, UV, LOD, collision, bake, weights and rig tools are door verbs (native Rust, or headless Blender 5.2 as a cook tool), not editor panels; `content.md` "no UV/retopo/rig tools in the editor" means panels only.
- D5 (owner 2026-10-10) Any donor code may be ported; `deny.toml` covers Cargo dependencies only; no EVIDENCE rows.
- D6 The door drops the "exactly 21 tools" cap for core tools plus verb packs (API4 unparked); every verb stays undoable and refuses by default.
- D7 Frozen until the first real frame from our own renderer (`crates/render`, D8, D9) and the one world (scene drives the match) land: games 2 and 3 (UW, UC), the 2040-look rows (RT, VG, VX, RL16, RL21-RL26), tier C spikes, net scale beyond the duel, new plan files.
- D8 Owner 2026-10-06 (viewer dropped in Claude Code, renderer is the move): D1's Bevy picture and D7's Bevy gate are dead. BV rows are BLOCKED on the board. RT, VG, VX, RL rows re-aim at `crates/render`, not the viewer; the freeze lifts per row.
- D9 Owner 2026-10-07 (Maxim: "i dont want bevy shit i want my own renderer", then "we steal from bevy just stuff"): no Bevy viewer and no Bevy dependency in the plan. No viewer work, no new BV row, no Bevy spike. The own renderer in `crates/render` is the only picture. Other engines, Bevy included, stay donors for the method and for licensed WGSL: read live, ported into `crates/render` with a PORT note.

Parked (rows stay and are never unticked; no new rows and no writers unless a bar needs them): audio, UI, modding, platform and shipping, living world. The second game (Urban war) is not parked (owner 2026-09-30) and keeps its board section. No tool enforces parking any more (the loop tools were removed 2026-10-17); this paragraph is the rule.

Operator 2026-09-24: the arena alone is too small and the look too safe. The
game grows from one arena into a Roman world of three streamed maps, full
online multiplayer (internet, not only LAN), and a render that reads as a
generation ahead (`VISUAL-TARGET.md` V5, tiers Base/RT/Ultra). The arena stays
the first proof; nothing below skips its gates.

## The game

- **Arena (Colosseum):** duels and team fights, 2–16 fighters, AI fills slots,
  a 20k–50k GPU crowd that reacts. First map, first proof (Finish A).
- **Rome district:** streets, rooftops and a market between the ludus and the
  arena; traversal/parkour fights, free-roam between matches. Streamed cells,
  not a seamless open world.
- **Siege field:** walls, gates, siege engines and ballistics; up to 32v32
  players with AI legions filling the ranks; walls break through the bonded
  destruction path.
- **Ludus hub:** the gladiator school — practice, repair-craft, glory rank,
  party up and queue.

## HARDEN (address more, waves go here)
multiplayer-heavy fights (server-authoritative, rollback LAN, 2+ fighters, AI fills empty slots, dedicated server), **online play (QUIC transport, relay + NAT traversal, invite codes, 64 players + AI per dedicated server, interest management, WAN fault soak, server-authority cheat checks; operator 2026-09-24)**, combat, physics, **networked physics (predicted debris, ragdoll and projectiles)**, active ragdoll, **balance and stumble, full-body IK, procedural hit reactions, GASP-class ground feel, physics-driven locomotion (offline-trained integer net), animation and physics LOD for armies, optimized af (parallel deterministic islands, SIMD, frame arenas, PGO, fast dev loop)**, destruction (baked + crack spread + runtime fracture for siege walls, operator 2026-09-24), ballistics/siege, **large battles (AI legions, squad tactics)**, gore/VFX, anim, traversal/parkour (vault, mantle, climb, hurdle, drop over motion-matched locomotion; operator 2026-09-23), arena AI, audio, crowd/nav, **GPU crowd (20k+ reacting spectators)**, save, input, UI/HUD, render look, **2040 look (GPU-live frame first, then hardware ray tracing: RT shadows, many-light direct light, dynamic GI with no bake, RT reflections, denoiser, path-traced reference; GPU-driven virtual geometry; volumetric fog and clouds; skin, eyes and hair; cinematic camera; operator 2026-09-24)**, **streamed maps (arena, district, siege field, ludus)**, spatial, script-lite, dialogue-thin, glory-rank (progress-lite), repair-craft (craft-lite), minimap, **day/night + weather**, replay theater, haptics, fonts, HDR, subtitles-minimal, colorblind-palettes, tutorial-1-flow. Tutorial kept single.

## EMPIRE (owner 2026-09-24)
General engine (no genre assumptions in engine crates, headless fixture games
prove it), living world (async recorded model-driven NPC memory, goals and talk
over deterministic tick brains), AI builds inside (brief → level, character,
encounter, playable slice). Board section `### Empire`.

## Second game — urban war (owner 2026-09-26, `plan/urban-war.md` (city is inside it))
Finish B's second game: a Ready or Not / SWAT 4-style tactical CQB
shooter in a fictional modern city (five-officer teams, rules of
engagement, surrender and morale, less-lethal tools, doors and breaching,
bullets through thin walls, a command menu, five-player co-op), growing
into dense generated, partly ruined cities with structural destruction.
Genre-neutral engine rows (UW1–UW26, UC1–UC12) run after the Now pillars'
writers; game and content rows wait for UW0 (Finish A met). Board section
`### Urban war`. Rome's lists above govern Rome only; this game keeps its
own scope in its plans.

## LIGHT (minimal, one wave each max)
stealth (assassinations), stamina-needs, dialogue/quests (arena ladder only), glory rank, repair crafting (ludus blacksmith — the kept crafting half), leaderboards-local, persistence, mod-jail (market dropped), a11y/loc-minimal, LAN discovery, **server list + party + matchmaking-lite (glory rank)**, **proximity voice (Opus)**, **photo mode (over the path-traced reference)**, **Gaussian-splat backdrops (licensed captures, presentation only)**, social-morale (crowd only), chariot (circus keeper — the kept vehicle path, proves input-tape/replay is vehicle-agnostic), platformer-params (jump feel only).

## CUT (never — no files, no deps, no waves)
XR/VR, mobile/2D lanes, console cert, crossplay, server fleet orchestration (self-hosted dedicated servers and relays are HARDEN; autoscaling fleets are not), UGC marketplace, liveops/seasons, analytics, chat infra (text), social sim, RTS/TD/BR/racing/platformer/building/survival genre systems, naval, kernel anticheat (server authority is HARDEN), RTS controls, fighting-genre frame-data (arena uses own), stealth-full, survival-full, building-full, skilltrees-full, crafting-full, economy-full, progression-full, social-full, seamless open world (maps stream in cells), ship-matrix beyond Windows and the browser, tutorial-full, subtitles-full (minimal kept). "WASM ship" was on this list until owner 2026-10-17 made the browser a target (Now item 9).

## OPEN (undecided — not cut, not scheduled, do not build yet)
- Campaign (incl. co-op): when, if ever. The district and siege maps make it cheaper later.
- Full crafting tree: when; repair-craft LIGHT above is the only kept half for now.
- Neural texture compression at runtime: waits for cooperative-vector access in wgpu; offline encode may land earlier.

## Slim rule
CUT limits what the Rome game contains, not what the engine can do (owner 2026-09-24): no Rome content, modes or waves for a CUT item, and engine crates must stay genre-neutral so any genre could be built. A CUT genre may appear only as a headless fixture proving generality (GE rows). Existing stubs stay frozen (no delete — avoid churn). Game work ONLY in HARDEN, EMPIRE and the urban war second game (owner 2026-09-26). If it is not Roman combat (arena, district, siege, ludus) or a `### Urban war` row, on Windows or in the browser (Now item 9), online or LAN, it waits.
