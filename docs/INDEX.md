# INDEX — active contracts

Map first: `README.md` (one screen). Quick start: `ROADMAP.md` → `ARCHITECTURE.md` (21.0KB, checked 2026-10-21); the work list is the board in `SPRINT.md` (slice it, never read whole).
Then read `PLAN-REVIEW.md` → `VISION.md` → `AI-NATIVE.md`, plus
`VISUAL-TARGET.md`, `EDITOR_AI_CONTROL.md`, `HASH-LAW.md` or the
subsystem plan in `plan/` for the task.
Execution: root `AGENTS.md`, `WORKFLOW.md`, `../CONTRIBUTING.md`, `CODELAW.md`.
Skills: `../skills/SKILLS.md`.

## Precedence

The documents below are the product and engineering contracts. `ROADMAP.md`
owns order, `WORKFLOW.md` owns execution, `ARCHITECTURE.md` owns boundaries,
`CODELAW.md` owns code quality (`RUST-QUALITY.md` measures and enforces it),
`deny.toml` owns dependency license policy.
Evidence claims need receipts. Old research catalogs and wave plans were
removed 2026-09-23; git history keeps them (`git log --all -- docs/archive`)
and they carry no authority.

## Active set

| Doc | Role |
|---|---|
| PLAN-REVIEW.md | Verdict against the operator's goals, evidence found, order and cost |
| VISION.md | Product goals and AI/runtime boundaries |
| VISION-TABLES.md | The vision's research tables (Scorecard, Parts vs the best, Open gaps, Research contract, Look rubric, Delivery evidence); research seats only |
| research/COMPARE.md | Full-layer coverage and power rubrics; fixed denominators, source states and measured performance; look up one id, never read it whole |
| AI-NATIVE.md | What makes engine2040 a new kind of engine: ten pillars |
| RESEARCH-VERDICT.md | Which researched ideas are IN, LATER or DROP |
| ROADMAP.md | Integrated milestones M0–M6 |
| ARCHITECTURE.md | Acyclic layers, crate integration status, schema contracts |
| CRATE-MAP.md | One row per crate: package, dir, job, engine deps and users (generated: `node tools/crate-map.mjs`) |
| ../.opencode/modmap.md | Files per crate, one line each (Grep it, never read it whole); `node tools/modmap.mjs --pack <crate>` prints one crate's packet (map slice, crate row, plan line, board rows) under 6,000 tokens |
| FINISH-RICH.md | Developer alpha and v1 acceptance with gate status |
| FOCUS.md | Rome at war: Now order + keep/cut lists |
| VISUAL-TARGET.md | Reference project and render milestones |
| EDITOR_AI_CONTROL.md | Provider-neutral services and shared transactions |
| HASH-LAW.md | Determinism scope and golden hash law |
| WORKFLOW.md | Scoped tasks, ownership, gates and failure handling |
| CODELAW.md | The slim law and Rust quality guidance |
| RUST-QUALITY.md | Measured Rust quality: scorecard, ratchet, the five enforcement layers and the bar per engine layer |
| SPRINT.md | Board index plus docs/SPRINT-*.md shards; slice it, never read index or shards whole; hint: `node tools/board-slice.mjs --ready` |
| library/ | Reusable skills, patterns, and recipes with tests and reuse counters; `library/index.md` is the index |
| systems/ | Two specs left: `ai-door.md`, `observability-heal.md` (the rest folded into `plan/`, old copies in `archive/2026-10-03/systems-*.md`) |
| SMOOTH-BUILD.md | Building without friction: instant changes, auto-fixes, ready-to-fix bugs (A10) |
| plan/ | Subsystem plans from `PLN` rows (`plan/README.md` is the template); `plan/content.md` covers characters, animations and tools |
| design/ | Player notes on what is seen and felt (`design/README.md` is the index) |
| adr/ | Chosen answers with reason and date (one file per choice) |
| BLOCKERS.md | Open blockers with owner, repro and remedy |
| DEPENDENCIES.md | Wired, banned and candidate crates |
| DECISIONS-M1.md | Technical decisions: layer crates, netcode model, determinism tier, MCP SDK, render references |
| EVIDENCE.md | Provenance ledger for code ports and content |
| ROYALTY.md | Draft business terms (not an engine contract) |
| plan/frontier.md | Research topics for the focus areas (render, animation, physics, hard systems, AI door) |
| research/LADDER.md | Cards, spikes and verdicts, one ladder rung each; read the rung, not the file (226 KB, checked 2026-10-21) |
| archive/ | No authority, git history made browsable: `archive/INDEX.md` lists what moved on 2026-10-17, older dated folders are listed by their date |
| CAPTURE.md | Capture output: built-in dump first, OBS only for live play |
| receipts/ | Dated proof receipts: capture frames and run receipts, one folder per date |
| evidence/ | Hashed frame evidence (one JSON per proof); the provenance ledger is `EVIDENCE.md` |
| DEMO.md | How to launch and play the arena demo (exe and browser), controls, what to look for |
| ../fixtures/ | Fixture dir: pinned test data, start at `../fixtures/README.md` |
| README.md | The one-screen map of docs/: plans, rows, book, research, archive |
| INDEX.md | This list |

## How-to (docs/book/)

- [Quick start: from zero to a running arena](book/quick-start.md)
- [The AI door: twenty MCP tools](book/ai-door.md)
- [Verifiers: keypoints, balance gate, red-team](book/verifiers.md)
- [Recipe: add a weapon](book/recipes-add-weapon.md)
- [Recipe: new map](book/recipes-new-map.md)
- [Recipe: drive it from MCP](book/recipes-drive-from-mcp.md)
