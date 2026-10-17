---
name: research
description: "Use when you do research, a steal or an experiment: a research card, donor repo or arXiv paper, a spike with one ablation, the books (tools/books.mjs), or porting Bevy 0.19 code into crates/render."
---

# Research

Research serves the pillars in `docs/FOCUS.md` "## Now" and the five in `docs/plan/frontier.md`. A card without a runnable spike is a reject. Verdicts live in `docs/RESEARCH-VERDICT.md`: IN follow-ups propose board rows. LATER and DROP need the user's explicit request.

Related: `engine-code` (porting rules), `perf-bench` (numbers), `render-pipeline-math`.

## 1. Card

A card has:

- baseline, counterevidence, measured hypothesis;
- integration home, bounded reproduction, kill criterion;
- the pillar, the finish bar or row it would move, and the spike that would prove it (fixture, baseline, metric).

Keep three things apart: paper claim, reproduced result, engine-integrated improvement. Compare provider-neutral and offline paths. No vendor-specific engine contracts.

## 2. Donors and steals

- Any donor source may be ported or adapted (owner 2026-10-10). No license check, no EVIDENCE row.
- One narrow GitHub search. Then read the default branch and exact file path live with webfetch. Never guess a raw URL. Never repeat an answered query. Prefer a known repo and a file read over another global search.
- If the GitHub MCP is down, `gh repo view <owner>/<repo> --json defaultBranchRef,stargazerCount` is a read-only fallback. Never `gh api` with `-f` or `-F` (that is a write).
- Pin the donor commit. Give the link. Say what we take.
- Optional port note so the next reader can find it: `PORT(owner/repo@rev:path)`.
- Hunt open-mindedly: other engines, games, libraries, papers, talks, negative results, one donor from outside engines and games. Search with websearch, then read primary sources with webfetch. Cite URL plus revision (or paper id) plus access date.
- Research how the best engine docs and tools teach. Steal structure, never prose.

## 3. arXiv

- Implement the paper's method. Say which section.
- Use the exact API form: `https://export.arxiv.org/api/query?search_query=...&max_results=N`. Never a guessed variant.
- Dedupe by paper id before you fetch the text.

## 4. Spike

One experiment. State up front: paper or version, hypothesis, baseline, fixture and seed, metric, resource budget, stop rule, real engine consumer, owned files. No new framework, no golden change, no paid service.

- Smallest runnable reproduction on the named fixture, plus one discriminating ablation.
- Same build, same seed, baseline vs candidate on the one deciding metric.
- Like for like. State non-transferable assumptions and the code, data and weight licenses. No cherry-picking. No unsupported speed claims.
- Three numbers stay distinct: paper claim, reproduced effect, integrated gain.
- Keep negative results. They stop repeated work.
- Return: patch, commands, baseline and candidate outputs, uncertainty, consumer, and adopt, reject or insufficient evidence.

LADDER: one line per card, spike or reject in `docs/research/LADDER.md` (format at its top; never read the file whole, read one rung with `Select-String -Pattern "^- CARD"`). One dated folder per research dive (`docs/research/<topic>-<date>/`), no loose files. Spike line:

`- SPIKE | P<n> | <idea> | <source> | <fixture, baseline -> candidate, numbers> | ROW proposal or REJECTED because <numbers>`

Triage questions (a code question, not a card): code outranks docs. Answer, path and symbol evidence, existing owner, missing coverage, smallest actionable row, acceptance command, unknowns.

## 5. Books

`node tools/books.mjs search "<words>" [--book <id>[,<id>]] [--k 5]` prints each hit as `[id | heading] score` plus up to 700 characters. The first search builds an index (about 4 s, cached in `target/books/index.json`). Later ones take under 1 s.
`node tools/books.mjs list` shows ids, language and size. `node tools/books.mjs toc <id>` lists headings.

| id | book |
|---|---|
| geaen | Gregory, Game Engine Architecture, 2nd ed |
| gearus | Gregory, Game Engine Architecture, 3rd ed (Russian, adds a concurrency chapter) |
| found1, found2 | Lengyel, Foundations of Game Engine Development, vol 1 Mathematics, vol 2 Rendering |
| gems1, gems2, gems3 | Game Engine Gems 1, 2, 3 |
| trpl | The Rust Programming Language, 3rd ed |
| progrust | Programming Rust, 3rd ed |
| idiomatic | Idiomatic Rust |
| bos | Rust Atomics and Locks (Russian) |
| transition | Refactoring to Rust (Russian) |
| rustwasm | Game Development with Rust and WebAssembly |
| handsrust | Wolverson, Hands-on Rust (roguelike with ECS) |
| testing | Modern Game Testing |
| gogame | Deep Learning and the Game of Go (Russian) |
| dushkin | Agentic and multi-agent design patterns (Russian) |

Example queries: `culling frustum` (found2 9.4), `animation blending` (geaen 11.6), `networked multiplayer game loop` (geaen 7.7), `memory ordering acquire release` (bos), `playtest bug report` (testing). Game loop timing is weak: see `tick-and-runtime`.

Rules:

- Cite a hit as `[id | heading]` in the commit or row. Quote at most 2 short lines.
- `(no heading)` means no heading was detected. Cite the book id only.
- No stemming. A Russian query needs the word form the text uses.
- geaen lost word spaces in about a quarter of its long lines ("Onecommonoptimization"). If a hit is missing, search the book's full text with the spaces removed.
- The book texts sit in the folder named by `BOOKS_DIR` (default `C:/empire/lab/skillworks/work`, if present on this machine). Without it the tool cannot index.

## 6. Bevy port map

The Bevy viewer was dropped on 2026-10-07. The picture is our own renderer in `crates/render`. Never build or extend a Bevy viewer. Reading Bevy source and stealing its methods into our code is fine.

Reference version: Bevy 0.19.1. Bevy 0.20 is a release candidate: do not use it. Old tutorials (0.12 to 0.18) use names that no longer exist. The WGSL ports in `crates/render/src/v1_gpu.rs` came from Bevy `v0.15.0`, so the 0.19 names apply to new ports only. Write the donor version in the header of each port.

How to port a Bevy method:

- Write plain Rust over our own types. A Bevy system becomes a function over our state. Sim crates never depend on Bevy.
- Keep the fixed tick free of Bevy scheduling. Time and order come from our tick (`docs/HASH-LAW.md`).
- Check any Bevy name you are unsure of in the 0.19.1 source, not a tutorial.
- Steal note: `bevy@<tag>`, the donor path and symbol, then the line you wrote in our code.

Renamed names are in `references/bevy-0.19-names.md`.

Sources: tools/books.mjs and the books it lists.
