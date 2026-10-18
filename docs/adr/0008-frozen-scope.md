# 0008 Frozen scope until the first real frame

- Status: Proposed
- Context: `docs/FOCUS.md:40` freezes games 2 and 3, the 2040-look rows, tier C spikes, net scale beyond the duel and new plan files until the first real frame from our own renderer (`crates/render`, D8, D9) and the one world land.
- Decision: Propose the lead keeps the freeze and lifts it per row as the renderer frame and the one world land. The architect picks no row.
- Consequences: Docs point one way. No code change. No hash change.
- Compliance: `docs/adr/adr.test.mjs` test `0008 FOCUS keeps the D7 freeze line`; the freeze is lifted per row by editing that line and this record together.
- Advice: Lead, via inbox `--ref ADR-0008 --by architect`.
