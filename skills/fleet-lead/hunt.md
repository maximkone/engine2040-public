# Steal hunt brief (lead, 2026-10-24). Read all of it once.

You hunt code to steal for engine2040 (C:\engine2040): a Rust/wgpu gladiator-arena game engine with a
deterministic integer sim (combat, physics via rapier 0.36, destruction), presentation layers (anim,
render, physfx), an AI MCP door (crates/endpoint), and a web build. It must look great AND run on a
potato (iGPU, 2-4 GB VRAM, 8 GB RAM). Owner policy: port or adapt code from ANY source, no licence
checks, no EVIDENCE rows. Small repos count: 0-star repos with one good file are welcome.

READ-ONLY on the repo. Never edit files in C:\engine2040, never run cargo, never git commit.

## How to hunt
- websearch to find candidates, then webfetch the PRIMARY source file (github.com/<o>/<r>/blob/<sha or
  branch>/<path> or raw.githubusercontent.com). Read the code, not the README. A steal is a file or a
  function you read, not a repo name.
- GitHub limits are shared with the build tooling (20 agents use `gh` for proofs): NO `gh api` calls;
  at most 10 `gh search repos|code` calls in your whole session. Prefer websearch + webfetch.
- Dedupe first: `git -C C:\engine2040 grep -il "<repo or author name>" -- docs skills` shows if we
  already stole it (docs/archive/*/research/hunts/ has old hunts). Already-ported donors (skip unless you
  found a NEW file in them): Bevy 0.19 wgsl, three.js CSM, Godot contact shadows, KinoBloom, Filament
  cloth, Khronos pbrNeutral, mmikk hextile, DeepMimic, rapier motor_model/PidController, ALS get-up,
  orangeduck damper_exact, XPBD/PBD basics, Ikemen hitstop, Embark crash-handling, ast-grep.
- Prove the gap (the 2026-10-18 lesson: hunters' "engine has no X" was wrong 2 of 9 times): for every
  steal run `git -C C:\engine2040 grep -n "<key word>" -- crates` (two or three words) and quote what
  exists today. If the engine already has it, the steal must say what it does BETTER, or drop it.
- Mark SIM if the port would change the hashed sim (crates combat, physics, destruction, arena tick):
  those are parked unless they fix a bug; prefer presentation, tooling, AI-door, render, anim, tests.

## Return (your final message is the deliverable; keep it under 2,500 words)
Up to 10 steals, best first. Each:

STEAL <n>: <short name>  [value H/M/L] [size S/M/L] [SIM or no-sim]
- Source: <exact file URL with commit or branch> (+ the function names you read)
- What it does: 1-2 lines, with the key numbers/constants if any
- engine2040 today: <the git grep line(s) you ran and what they show: file:line or "0 hits for X, Y">
- Target: the engine2040 file/function it would change or the new file it would add
- Port sketch: 2-4 lines (types/functions to add, the test that proves it, the proof filter)

Then: REJECTED (one line each: repo, why) and DEAD ENDS (searches that found nothing).
45-minute box: at 40 minutes stop and send what you have.

## Wave 2 (evening 2026-10-24): write board rows, not only a report
- Also dedupe against today's wave: `docs/SPRINT-39-steal-wave.md` (SW rows) and these donors already taken
  today: Juicee, phantom-camera, ALS-Refactored, Veloren melee, Fyrox BlendByIndex, BehaviorTree.CPP reactive,
  bevy-inspector-egui derive, hot-lib-reloader, WebGL-Fluid-Simulation, Filament ShadowMapManager, dovetail
  depth_reduce, ellioman indirect culling, Godot occlusion HZB, Godot vertex lights, OpenKCC foot bake,
  blue-archive-fighting-game, godot-liquid-ui, Jolt drive-to-pose, rust-analyzer tidy, Zed/nushell cargo config.
- Write each steal you would build as a board row into the rows file the lead named
  (the lead gives you one path and one ID range), using ONLY the ID range you were given,
  in exactly the shape of the rows in `docs/SPRINT-39-steal-wave.md` (title line: what the player or dev sees;
  today's file:line; Do; Donor: URL + function names; Tier; Size; Owns; Accept with a cargo filter and numbers;
  F2P; P2P; needs:). SIM rows land on rules v3 (row SW27 in `docs/SPRINT-39-steal-wave.md`): v1 and v2 tapes
  and the golden never move. Write the file as you go (after each row).
- Final message: one line per row (ID, title, H/M/L) + REJECTED + DEAD ENDS. Under 600 words.
