---
name: hotspot-recon
description: "Use when planning a row before reading code to rank git-history hotspots and bug magnets first"
version: 0.1.0
author: skillworks
---

# Hotspot recon

Rank risky files from git history before reading code. Plan the row around the hotspots, not blind. Not for book citations (see book-search).

1. **Churn rank.** Run `git log --format= --name-only -n 200` from the repo root, group by path in pwsh, sort by count down, keep the top 10.
2. **Hotspot rank.** Score each top file by churn times recent fixes. A file touched often and fixed often ranks above a file touched once long ago.
3. **Bug-magnet cite.** Name the top-10 risky files in the packet with one line each for why each is risky (churn count, fix count).
4. **Owner via CRATE-MAP.** Map each risky file to its owning crate in `docs/CRATE-MAP.md`. Name the crate beside each file.
5. **Risk note in packet.** Write a short risk note in the packet listing the top 3 hotspots and what the plan avoids touching in each.
6. **Do / Don't.** Do plan tests around the top hotspots first. Don't read the whole tree or skip the history step.
7. **Bus-factor flag.** Run `git shortlog -sn -- <file>` on each top file, list the overlapping owners, and flag a file when fewer than 30% of its authors are still active.
8. **Momentum and firefighting.** Count recent fix, revert, and hotfix commits for each top file and cite that rate beside the file.

## Test note

Check the packet cites the top-10 list with counts.
Check each cited file maps to one crate owner.
Check the risk note names the top 3 hotspots.
Check the bus-factor flag and the fix/revert rate are cited beside the top files.

source: target/reports/steal-triage-2026-10-14.md gap #1, donor composio codebase-recon (ported, not copied)
