# 0007 No Bevy viewer and no Bevy dependency

- Status: Proposed
- Context: `docs/FOCUS.md:41-42` (D8, D9) says the own renderer in `crates/render` is the only picture. No Bevy viewer work, no new BV row, no Bevy spike, no Bevy dependency in the plan. Other engines stay donors for method and licensed WGSL only, ported with a PORT note.
- Decision: Propose the lead keeps D8 and D9 as the rule. No viewer work starts. No `bevy` crate enters any `crates/*/Cargo.toml`. The architect picks no renderer design.
- Consequences: Docs point one way. No code change. No hash change.
- Compliance: `docs/adr/adr.test.mjs` test `0007 no Cargo.toml names a bevy dependency` scans the root `Cargo.toml` and every crate manifest.
- Advice: Lead, via inbox `--ref ADR-0007 --by architect`.
