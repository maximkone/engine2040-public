---
name: delegate-context-pack
description: "Use before dispatching a delegate/builder packet: compress to a minimal execution-ready packet (architecture, constraints, files, assumptions, unknowns) so the delegate starts without re-discovery."
version: 0.1.0
author: skillworks
---

# Delegate context pack

Pack the dispatch, not the repo. First read acquire-codebase + hotspot-recon when unfamiliar: theirs map the code and rank the risk; yours packs the dispatch — emit the packet below before any delegate/builder dispatch.

1. **Architecture map.** One line per crate touched: directory, job, owning file/symbol. Skip untouched crates.
2. **Constraints/conventions.** Only enforced ones from repo evidence: frozen APIs, naming scope, lint rules (expect never allow), invariants (e.g. parry < startup). Cite the source file each.
3. **Key files/symbols.** Each entry carries a path plus symbol or line anchor. No file without an anchor, no anchor without a decision it supports.
4. **Assumptions + confidence.** Mark every non-obvious claim confirmed (file:line proves it) or inferred (confidence high/med/low). Inferred-high still names its check.
5. **Unknowns + discovery order.** List open questions that change the plan, then the file read order that closes them. Name the stop rule (timebox, second identical failure ends it).
6. **Cut the rest.** Every item must support a delegate decision; drop background. Cap the packet at 40 lines.

## Checks

1. Packet is 40 lines or fewer.
2. Every file entry carries a symbol or line anchor.
3. Every constraint cites its source file.
4. Every assumption is marked confirmed or inferred with confidence.
5. Unknowns list the discovery file order plus the stop rule.
6. No item survives that supports no delegate decision.

## Test note

Build one packet from a DONE dispatch preamble and count lines before vs after.
Confirm each check above passes on the rebuilt packet, else cut further.
Record the cut and any delegate misfire (wrong file, no-owned-paths, bad guess).

source: target/reports/steal-context-manager.md ADOPT (method ported, rewritten in own words; donor VoltAgent/awesome-codex-subagents context-manager MIT, zero donor bytes)
