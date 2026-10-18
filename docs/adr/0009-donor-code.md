# 0009 Donor code may be ported, deny.toml is Cargo only

- Status: Proposed
- Context: `docs/FOCUS.md:39` (owner 2026-10-10) says any donor code may be ported; `deny.toml` covers Cargo dependencies only; no EVIDENCE rows. `AGENTS.md` rule 4 says the same.
- Decision: Propose the lead keeps the donor-port rule with the Cargo-only `deny.toml` scope. The architect picks no donor and adds no licence check.
- Consequences: Ports land as own code with a PORT note. No licence gate. No hash change.
- Compliance: `docs/adr/adr.test.mjs` test `0009 donor rule stays in FOCUS, AGENTS and deny.toml scope` pins the D5 line in `docs/FOCUS.md`, rule 4 in `AGENTS.md` and the Cargo-only scope comment in `deny.toml`.
- Advice: Lead, via inbox `--ref ADR-0009 --by architect`.
