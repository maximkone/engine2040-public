---
name: engine-research
description: Use when doing engine2040 research - research card shape, arXiv form, LADDER lines, wild-hunt rotation, explore triage. Loaded by the researcher for RES-STEAL/RES-ARXIV/RES-WILD/EXPLORE packets.
---

# Engine research (from researcher, researcher, researcher)

Research serves the pillars in `docs/FOCUS.md` **Now** and the five in
`docs/plan/frontier.md`. A card without a runnable spike is a reject.

- **Card:** baseline, counterevidence, measured hypothesis, integration
  home, bounded reproduction, kill criterion. Separate paper claim,
  reproduced result and engine-integrated improvement. Compare
  provider-neutral/offline paths; no vendor-specific engine contracts.
  Every card names its pillar, the finish bar or row it would move, and
  the spike that would prove it (fixture, baseline, metric).
  RESEARCH-VERDICT IN follow-ups propose board rows; LATER and DROP need
  the user's explicit request.
- **Donors:** one narrow GitHub search, then default branch and
  exact file path read live with webfetch. Never guess a raw URL or repeat
  an answered query. Prefer a known repo and `get_file_contents` over
  another global search. If MCP is down after a restart, `gh repo view
  <owner>/<repo> --json defaultBranchRef,stargazerCount` is a
  read-only fallback; never `gh api` with `-f`/`-F` (a write). Pin the
  donor commit, give the link, say what we take.
- **Steals:** any donor source may be ported or adapted (owner 2026-10-10).
  Pin the donor commit, give the link, say what we take. An optional
  `PORT(owner/repo@rev:path)` note may stay so the builder can find it. Observe
  RESEARCH-VERDICT LATER/DROP.
- **Arxiv:** implement the paper's method; say which section. Use the exact API form
  https://export.arxiv.org/api/query?search_query=...&max_results=N (never
  a guessed variant); dedupe by paper ID before fetching the text.
- **Wild rotation (from researcher):** papers, maintained systems, negative
  results, one donor outside engines/games. Hunt donors open-mindedly with
  websearch (other engines, games, libraries, papers, talks), then read
  them with webfetch. Cite URL + revision/version or paper id + access
  date. Fetch primary sources.
- **LADDER:** one line per card or reject (`docs/research/LADDER.md`,
  format at its top); no new per-wave research file.
- **Triage (from researcher):** code outranks docs; follow root AGENTS.md.
  Answer, path:line evidence, existing owner, missing coverage, smallest
  actionable packet, acceptance command, unknowns.

Return changed paths, the cards or triage, and open questions.
