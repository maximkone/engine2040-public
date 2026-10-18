# Steal wave 2026-10-18: combined ranking and what lands

Owner ask 2026-10-18: hunt Rust 3D engines (and AI-native engines, and new ones) for what we have not stolen, then land the good ones; lead decides what is worth it. Six Haiku hunters wrote `01-render.md` .. `06-new-engines.md` (each lists donor file, our file, grep proof, read depth). The lead spot-checked: two "0 hits" claims were false-new (entity-to-pixel table = EV21 `crates/endpoint/src/pick.rs` + `crates/spatial/src/compose.rs`; ragdoll velocity handoff = `PhysFx::spawn_ragdoll`, `crates/physfx/src/fx.rs:216-228`). Hunters grep too narrowly, so every implementer starts with a wide search and stops with ALREADY HAVE if the thing exists.

Lead rules for picking: (1) small, one crate, no collision with the ~50 files other sessions hold open; (2) must be reached by a real caller; (3) integer in sim crates, float only in presentation, goldens `d57b5c8ae3096614` / `2aca9338670aa0cb` untouched; (4) anything that changes fight balance or every golden is the owner's call, not ours.

Status key: LANDING = implementer running, result goes in the last column when committed. PARKED = owner call. LATER = good, needs a quiet file or a bigger slot. SKIP = not worth it.

## Landing now

| # | Steal | Donor (hunter file) | Lands in | Status |
|---|---|---|---|---|
| L1 | Reversible click-free fades (declick state machine, dry/wet pair) | Firewheel `dsp/declick.rs` (03) | `crates/audio` | landed 65b4fd5a (voices, steals, music crossfade, duck all fade through it; 256 passed) |
| L2 | Diagnostics store: bounded ring, O(1) mean, time-aware EMA | bevy_diagnostic `diagnostic.rs` (04) | `crates/perf` | landed 56a16c8c (FrameReport prints the smoothed frame_time row; perf lib has one pre-existing red, feel gold 500 vs fixture 510) |
| L3 | Oracle-normalized agent score + seeded bootstrap CI | agentick `scoring.py` (05) | `crates/playtest` | landed 1cd4740f (one `norm=` line in the tournament report; zero-gap task scores 1000, guard it before reading the number) |
| L4 | Native strict-WebGPU switch (`STRICT_WEBGPU_COMPLIANCE`) | wgpu 30.0.1 notes, 10-17 hunt #3 | `crates/render` | landed 51e4b15c (`E2040_STRICT_WEBGPU=1`; default off; ci.yml has the env but the Linux runner has no adapter, so it only bites on the PC) |
| L5 | Ragdoll momentum handoff | Fyrox `scene/ragdoll.rs` (02) | `crates/physfx` | ALREADY HAVE (linear part; angular `w x r` term left, visual gain small) |
| L6 | Crash-safe streaming tick journal (torn tail drops, middle corruption fails) | ferrets `replay.rs` (06) | `crates/proof` | landed 36e81785 (`engine2040 --journal`, `--replay` takes tape or journal; proof lib 154 passed) |
| L7 | Ack bitfield + loss permille tracker | naia `ack_manager.rs` (03) | `crates/net` | landed 0517bcdf (input-tick tracker, no wire byte changed; `in_loss=` on the BOUT line and MP6 rows; real ack bits need a WIRE_VERSION bump; split blob +6 words/seat without BOUT_BUILD bump, mixed revisions refuse with BadBlob; net lib 306 passed) |
| L8 | Harness presets: zero-shot / reasoner / history, last-line integer parse | agentick `prompt_templates.py` (05) | `crates/provider` | LATER: no live path takes an integer model reply (parse is strict JSON proposals), so presets would be an island. Board the gym integer -> action -> `input.inject` consumer first |
| L9 | Anim event strategy under blends (All / MaxWeight / MinWeight), first a double-fire test | Fyrox `blendspace.rs` (02) | `crates/anim` | landed 1c0ab6be (50/50 blend double-fire pinned; only the eval harness calls `collect_space_cues`, the live arena has no cue path: wire `SpacePlayer::tick` + `collect_space_cues` in the arena loop) |

## Later (good, blocked on a quiet file or a bigger slot)

| # | Steal | Why not now |
|---|---|---|
| T1 | Fighter outline: vertex extrusion along a cooked outline normal, one extra draw per fighter (bevy_mod_outline `outline.wgsl`, 01) | Wiring goes through `crates/game/src/fighter_look.rs`, which another session is editing today. Best G4 silhouette win per cost. |
| T2 | Render proof honesty: `backend_selection` report in every render envelope + golden tolerance as two numbers + perturbation self-test (scena `headless-rendering.md`, minecraft-2 D3; 06) | Same files as L4 (`adapter.rs`, instance helper). Do after L4 lands. |
| T3 | Web: wgpu loop in a worker on OffscreenCanvas (webgpu-worker `worker.ts`, 06) | Needs a real browser run to prove; WEB slot, M size. |
| T4 | Torch point glow, fragment-only (Veloren `point_glow.glsl`, 01) | Needs the EG12 torch light list in uniforms; check it exists first. |
| T5 | `ssimulacra2` crate as the golden image metric (10-17 hunt #4; `tools/iqa-gate.mjs` still leaves it null) | Adds a dependency and a golden-bless flow; one slot after T2. |
| T6 | `plan_check` pre-flight and `read_guide` verbs (bevy_mcp_harness ADR 0004/0006, 04) | `crates/endpoint/src/mcp.rs` and `discover.rs` are dirty (spatial session). Pays chars in the 43,514 tool-list budget. |
| T7 | Per-entity priority accumulator + token-bucket send budget (naia, 03) | After L7; touches `interest.rs`. |
| T8 | Visible-state projection (public/private tags) on gym observe (ftl-bench, ogremcp, 05) | `packs/gym*` area; small, idea-only license. |
| T9 | Save field versioning rules (Fyrox `visitor`, 04), UI accessibility tree (harness README, 04), CI frame script (bevy `ci_testing`, 04) | Real but no gate depends on them today. |

## Parked: owner decides (they change fight feel or every golden)

| # | Steal | Donor | What it changes |
|---|---|---|---|
| P1 | Jump shaping: shorten while button released, takeoff/peak/fall extra gravities | bevy-tnua `jump.rs` (02) | `controller2.rs` is hashed; any tape with a short jump moves the golden. Needs a measured apex first (ours is 1.45 m, no release shortening). |
| P2 | Five-tier poise ladder with timed stuns and tier-scaled movement | Veloren `poise.rs` (02) | Combat balance (we have 3 stagger levels, one poise trip). |
| P3 | Flank precision, block poise cost, penetration | Veloren `combat.rs` (02) | Damage numbers. Veloren's parry line is a bug as written (copy the intent). |
| P4 | Crush threshold table + 2.0 m/s no-damage floor | voxel-demolition `physics.rs` (06) | `crates/destruction` is hashed; cascade counts move. |
| P5 | AgX display transform instead of ACES | minecraft-2 D4, kajiya Notorious6 (06, 01) | Every golden re-blesses; the look judge is cut, so nobody signs the new look. |
| P6 | Chord clash rule (longest chord wins) | leafwing `clashing_inputs.rs` (03) | Changes which inputs reach the tape: a v2 input bit (XX6 rule). |

## Skipped on purpose

- Entity-to-pixel table (EV21 already), ragdoll handoff (already), Bevy easing (a private smoothstep is enough until XX2/XX4 ask), bevy_diagnostic overlay widgets (need bevy_dev_tools), puffin_http (3/5 value, dev only), lightyear `diff_history` and ggrs spectator (listing-only reads), renet2 sockets (we have our own browser legs), Fyrox occlusion cell memory (readback stall), kajiya/rend3 (archived), rustyAC (reverse-engineered commercial binary), voxel engines (wrong shape).
- 10-17 hunt items that are still open and still good: box3d-rust determinism oracle (M), `scene.diff` verb (endpoint dirty), `rapidhash`, AudioWorklet, LoadQueue, WESL, meshopt decode. They stay in `docs/research/finish-line-2026-10-17/02-github.md`.

## Facts the hunters corrected in our own docs

- ggrs XOR+RLE input compression is already in `crates/net/src/session.rs:996-1007`; the multiplayer plan line saying it is missing is stale.
- Browser legs exist in `crates/net` (`webtransport.rs`, `webtransport/server.rs`, `websocket/server.rs`, `browser.rs`); plan text saying "not built" is stale.
</content>
