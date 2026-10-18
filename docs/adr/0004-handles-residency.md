# 0004 Handles and residency, build or cut

- Status: Proposed
- Context: `docs/plan/content.md:210` says "will not build: handles and residency (AS4/AS5)", but `:220-221` keeps AS4/AS5 open, `:89-90` streams bundles with placeholders, and `:144-146` queues to LodSwapper. `assets/src/registry.rs:32 Handle(pub u32)` has no generation and `stream.rs:499 evict` is an event with no unload owner.
- Decision: Propose the lead picks build (AS4 handles with generation plus AS5 residency manager) or cut (keep `:210` and close the rows). The architect picks no winner.
- Consequences: Docs point one way. No code change. No hash change.
- Compliance: `crates/assets/src/handle.rs` test `stale_handle_never_panics_and_never_ready`: the generational `AssetHandle` rejects a stale handle after `free` (the build answer; the cut answer would delete that module and this check).
- Advice: Lead, via inbox `--ref ADR-0004 --by architect`.

Update 2026-10-18: `crates/assets/src/handle.rs` now holds a generational `AssetHandle` with load states (no caller yet); `registry.rs` `Handle(pub u32)` is unchanged. Status stays Proposed: the build or cut call is the lead's.
