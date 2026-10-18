# 0005 Physics bodies off in the shipped app

- Status: Proposed
- Context: `crates/game/Cargo.toml:14-16` keeps `app` without `engine-arena/sim`, so the shipped fight is kinematic. Ragdoll and powered bones exist, but no Rapier body builds in the played path. See `docs/plan/BOOK-GAPS-2026-10-12.md:121` and plan gap S10.
- Decision: Propose the lead picks build (enable the sim leg in the shipped app and replay the golden) or cut (keep the kinematic fight and close PH3/PX1 as kinematic only). The architect picks no winner.
- Consequences: Docs point one way. No code change. No hash change until wired.
- Compliance: `docs/adr/adr.test.mjs` test `0005 the app feature leaves engine-arena/sim off` reads `crates/game/Cargo.toml`; turning the sim leg on goes red until this record is superseded.
- Advice: Lead, via inbox `--ref ADR-0005 --by architect`.
