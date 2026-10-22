# Research 2026-10-22: the lead's verdict

Owner, 2026-10-22: "research if this engine finish line is worthy, compare with 10+ GitHub engines, what to steal; research arXiv to apply to the repo, the engine and the agents". Four Haiku reports (`01-engines.md`, `02-feel.md`, `03-arxiv-engine.md`, `04-arxiv-agents.md`); the lead checked the top claims on the source pages. Booked rows: `docs/SPRINT-31-research.md`.

## Is the finish line worthy?

Yes, as a bet. No engine read (Godot, Bevy, Fyrox, O3DE, Stride, Flax, Wicked, Defold, macroquad, Babylon.js, PlayCanvas, Castle, quench3d, Open-Godot-MCP) claims a bit-identical sim across native and web plus an AI door. The nearest peer is `gh503/quench3d` (0 stars, Rust, wgpu, web, MCP; checked on GitHub), and it calls its own digest "a change alarm, not proof". Determinism + wasm parity + the door is a real niche.

Too big as written: 23 gap rows, several L. Competing on breadth with Godot (119k stars) is a losing race. The engines report proposes a must list (A1, A2, A4, A5, B7, B8, B9, C10, C13, D16, D17, D19, E21, E22) and deferring A3, C11, C14, C15, E23 until G7 and G8 are green. That cut is the owner's call (FIN1).

Missing staples: update and patch delivery, and a crash receipt on every run. Undercounted: rebinding, settings, save slots, menus, en/de locale, accessibility, the puffin profiler and cpal audio already exist.

## What the lead checked

- Veloren knockback (`common/src/combat.rs`, raw file read): impulse = 50 x strength x target resistance (0.5 when resistant) x attacker knockback_mult x a direction slerped toward up (0.5 away/towards, 0.85 towards-up). The 50 is marked temporary in Veloren. It feeds FEEL2 as a mechanism; our numbers come from weapon mass (FEEL1).
- 2512.08296 "Towards a Science of Scaling Agent Systems" (abstract read): multi-agent gains +80.8% on decomposable work, loses up to 70% on sequential planning, and setups without central verification spread errors more. The 17.2x / 4.4x amplification numbers are not on the abstract page (unverified).
- 2603.08619 is an RL humanoid-recovery paper that uses the capture point as a training signal, not a rule. ARX01 keeps the capture-point idea (classic balance control) but cites it honestly.
- 2609.25624 fixes float reduction order in LLM GEMMs. Our GPU cull counts are integers, whose sums do not depend on order, so ARX04 is dropped.

## Agents: the answer to "lead spawns 5, each spawns 5"

Not supported. The evidence supports one lead that splits rows by file set, 3-4 helpers per wave on independent rows, each committing its own proven step, deterministic gates (tests, rust-quality, the AGENTS.md rules as a script), and a strong judge reading the gate result and the diff. No helper spawns helpers. Our rows are mostly coupled (one golden hash, shared interfaces), which is the case where extra layers hurt. The worker-count curve is unmeasured anywhere: AGT19 measures it here.

Note: `opencode.jsonc` sets `experimental.subagent_depth` 2 (the owner's 2026-10-04 choice). The papers argue for 1. That is the owner's call (AGT01).
