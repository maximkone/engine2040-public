# 0004 Handles and residency, build or cut

- Status: Proposed
- Context: `docs/plan/content.md:210` says "will not build: handles and residency (AS4/AS5)", but `:220-221` keeps AS4/AS5 open, `:89-90` streams bundles with placeholders, and `:144-146` queues to LodSwapper. `assets/src/registry.rs:32 Handle(pub u32)` has no generation and `stream.rs:499 evict` is an event with no unload owner.
- Decision: Propose the lead picks build (AS4 handles with generation plus AS5 residency manager) or cut (keep `:210` and close the rows). The architect picks no winner.
- Consequences: Docs point one way. No code change. No hash change.
- Compliance: none yet.
- Advice: Lead, via inbox `--ref ADR-0004 --by architect`.
