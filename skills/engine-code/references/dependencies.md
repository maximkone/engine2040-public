# Dependencies

Full text behind the dependency rule in `SKILL.md` More rules.

- A new Cargo dependency's license must pass `deny.toml`. Run `cargo deny check`.
- Add its row to `docs/DEPENDENCIES.md`.
- Prefer small crates without C toolchains. Put heavy ones behind a feature.
- Donor source from any repo may be ported directly (owner 2026-10-10). No EVIDENCE
  row. Never hand-roll crypto.
- Use `workspace = true` and the root `[workspace.dependencies]` table.
