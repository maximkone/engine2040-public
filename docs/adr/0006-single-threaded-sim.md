# 0006 Single-threaded sim until RC36 parity

- Status: Proposed
- Context: The sim runs on one thread today. `docs/plan/runtime.md:138` lets it run index-ordered fork-join on the one `JobPool` only when the hash is equal at 1, 2, 4 and 8 threads (RC36); otherwise it stays single-threaded and deterministic. See `docs/plan/BOOK-GAPS-2026-10-12.md:100` and plan gap S17.
- Decision: Propose the lead keeps the single-threaded sim as the rule and lets RC36 parity (equal hash at 1, 2, 4, 8 threads) be the only door to fork-join, with HASH-LAW ordering (declared order, never thread-count-dependent). The architect picks no thread count.
- Consequences: Determinism first. Parallel bakes stay outside the tick. No hash change until RC36 passes.
- Compliance: the golden hash proof (`cargo run -p engine2040 --release`, `node tools/proof.mjs`); a thread-count-dependent sim moves the hash and fails it.
- Advice: Lead, via inbox `--ref ADR-0006 --by architect`.
