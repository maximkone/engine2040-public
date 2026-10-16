# Steal-papers P5 run 2026-10-08 (researcher-steal-papers)

Pillar: P5 AI builds the game (LADDER cursor: last served 4, next serves P5).
Method: arXiv API search (2 narrow searches) + 3 abstract reads live 2026-10-08.
Papers are ideas-only, reimplemented in own words, zero bytes vendored.
No GitHub donor this run (paper-only cards); no live repo license to record.

Baselines (git grep first, this run):
- `crates/spatial/src/place.rs:1` ("model states relations, deterministic solver owns geometry"), `:83` relation constraint, `:720` `solve()` — the constraint-satisfaction home C1/C2 deepen.
- `crates/endpoint/src/plan_goals.rs:391-454` staged level + fork-checked encounter (`plan_staged_encounter`) — the brief-to-game home.
- `crates/playtest/src/brief_eval.rs:1` (20 briefs through level/character/encounter/slice gates) + `crates/endpoint/src/build_game.rs` packaged slice — the slice home C3 deepens; no persistent skill store exists.

## C1 — Self-consistent relation decode before the solver (LayoutVLM shape)

- Source: arXiv 2412.02193v3 (LayoutVLM, verified live 2026-10-08 via abstract read; categories cs.CV/cs.AI).
- Donor take (own words): the paper gets VLM layout proposals into two mutually reinforcing representations and decodes them self-consistently, then runs differentiable optimization so the result is physically plausible and matches the language intent.
- License: UNVERIFIED (paper ideas-only; code, data, weights all NOT taken).
- Home: `crates/spatial/src/place.rs` beside frozen `solve()`; consumes `plan_goals.rs` staged briefs. No new file.
- Spike (one step, via builder EXPERIMENT, not run here): fixture 10 AB1 briefs (arena/ludus/siege rooms); baseline single-pass relation propose -> `solve`; candidate sample-3 relation proposals + majority-vote decode -> `solve`. Metric: clean-first-pass share + residual constraint violations; post bytes x2 + golden held.
- Number: baseline measured at spike time; candidate must lift clean-first-pass by >= 30% relative.
- Kill: lift < 30%, or any float enters the solver, or any digest/byte drift, or golden moves.
- Row: none this run; spike first (AB1 detail). Moves FOCUS Now P5 brief-to-level.
- EMPIRE: AI builds inside (brief -> level).

## C2 — Room-type constraint templates for brief-to-level (Holodeck shape)

- Source: arXiv 2312.09067v2 (Holodeck, verified live 2026-10-08 via abstract read; cs.CV/cs.AI/cs.CL/cs.RO).
- Donor take (own words): the paper uses an LLM for commonsense scene content, prompts it for spatial relational constraints between objects, then optimizes the layout to satisfy those constraints; human judges preferred its rooms over procedural baselines.
- License: UNVERIFIED (paper ideas-only; Objaverse asset bytes explicitly NOT taken — mixed third-party terms).
- Home: `crates/spatial/src/place.rs` (template -> relations -> `solve`); templates keyed by room type from the brief. No new file.
- Spike (one step, via builder EXPERIMENT, not run here): fixture same 10 AB1 briefs; baseline free-relation propose -> `solve`; candidate room-type template constraints -> `solve`. Metric: solve-first-pass share + object-diversity count (distinct objects placed); bytes x2 + golden held.
- Number: candidate must lift solve-first-pass by >= 20% with diversity not dropping.
- Kill: lift < 20%, or diversity drops, or any float in solver, or any digest drift.
- Row: none this run; spike first (AB1 detail, sibling of C1 — winner takes the home). Moves FOCUS Now P5 brief-to-level.
- EMPIRE: AI builds inside (brief -> level).

## C3 — Tested slice-skill store with reuse counters (Voyager shape)

- Source: arXiv 2305.16291v2 (Voyager, verified live 2026-10-08 via abstract read; cs.AI/cs.LG).
- Donor take (own words): the paper keeps an ever-growing library of executable, tested, compositional skills with an automatic curriculum and self-verification from environment feedback, reusing the library in new worlds instead of relearning.
- License: UNVERIFIED (paper ideas-only; MineDojo code/prompts NOT taken).
- Home: `crates/endpoint/src/build_game.rs` (slice build) + `crates/playtest/src/brief_eval.rs` (20-brief eval); store = tested snippet + pass receipt + reuse counter, advisory only, never auto-applies untested code. No new crate.
- Spike (one step, via builder EXPERIMENT, not run here): fixture 20 `brief_eval` briefs; baseline build each slice from scratch; candidate build with skill-store reuse (tested snippets only). Metric: slice-pass share + reuse rate + replay bytes x2.
- Number: candidate must gain >= 10pp slice-pass at reuse > 0 with zero untested executions.
- Kill: gain < 10pp, or any untested skill executes, or any replay/golden drift.
- Row: none this run; spike first (brief-to-slice detail). Moves FOCUS Now P5 brief-to-slice.
- EMPIRE: AI builds inside (brief -> playable slice).

## Rejects

- R1 Holodeck Objaverse asset bytes as content donor: REJECTED — Objaverse items carry mixed third-party terms; content rule needs per-capture license rows, so no bytes this run; ideas-only stands.
- R2 Separate Holodeck 2.0 (arXiv 2508.05899v3) take: REJECTED as DUPE of C2 this run — same author line and constraint-then-optimize shape; its editing delta needs the landed ED11 journal home and names no separate gap; re-open only with a pinned editing-gap fixture the C2 templates fail.
