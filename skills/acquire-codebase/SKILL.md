---
name: acquire-codebase
description: "Use when entering unfamiliar crates, onboarding, or assessing risk before reading code. Map intent and boundaries first, then hand off to hotspot-recon for git-history risk rank. Never for routine edits."
version: 0.1.0
author: skillworks
---

# Acquire codebase

Intent-first map before any code read. Four phases, under 120 s: state intent, scan bounded, fill seven sections, resolve ASK-USER. Narrow to the row's focus area; skip untouched crates. Read-only: never checkout, rebase, or edit. Complements hotspot-recon (git-history hotspots come after this map, never instead of it).

1. **Intent docs first.** Summarize the README, the roadmap slice, and the row's spec or design in three lines before opening any source file.
2. **Bounded scan.** Count files through the crate map and modmap plus one board slice. Never read whole trees or whole docs; slice by id.
3. **Architecture.** List one row per crate touched: package, directory, job, and engine dependencies.
4. **Boundaries.** State dependency direction per crate from the architecture doc; name any new edge the row would add.
5. **Determinism gates.** Name the hash law, the fixed tick, and what state the golden hash covers; flag wall clock or map iteration order inside hashed state.
6. **Tests and proof.** Name the acceptance command per touched row: crate check, crate test, then the proof or capture gate.
7. **Report with ASK-USER.** Fill the seven headings below plus an ASK-USER list; stop on ambiguity instead of guessing. No edits, no checkouts.

## Report

Keep each section to three lines. Mark unknowns ASK-USER with owner and done-when. Kill the map and report when it exceeds 120 s or two sections come back empty on a non-empty tree.

### Intent
### Scan
### Architecture
### Boundaries
### Determinism
### Tests
### ASK-USER

## Do / Don't

- Do: map intent and boundaries first, then run hotspot-recon for the git-history risk rank.
- Don't: open cold code, read whole trees, or rank hotspots without the map.

source: board row OPT29 onboarding budget (method ported, rewritten in own words)
