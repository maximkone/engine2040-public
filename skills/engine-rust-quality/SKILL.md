---
name: engine-rust-quality
description: Use when scoring engine2040 Rust quality as a reviewer - packet-gate and rust-quality gates, the 0-5 rust score, what tools cannot see. Loaded by the judge for RUST-gated REVIEW/CHECK packets (0-5 rust score only; depth, evidence and captures stay in engine-review, never double-gate).
---

# Engine rust quality (from judge)

You get the crate's `rust-quality` line; the reviewer reruns it targeted
(never a full-workspace suite). A `rust-*` FAIL or a ratchet regression is FAIL
before you read further.

Commands:

- `node tools/rust-quality.mjs --crate <crate>` must print RESULT PASS
  with 0 ratchet regressions.

Then read the diff for what tools cannot see and score it 0-5 (0 would
not compile in review, 3 idiomatic and owned, 5 a top engine team would
copy it):

- Ownership and borrowing without needless clones; data layout fit for
  the hot path (SoA, slices, reused buffers); errors typed and specific;
  conversions explicit; the smallest `pub` surface; names with units;
  functions short and single-purpose; no re-derived helper that an owner
  already exports (`engine_core::{Fnv64, hash_bytes, splitmix64, Rng}`,
  `engine_core::alloc_probe`); comments that state invariants, not
  history (no board IDs, wave numbers or "was X"); tests that would fail
  on a plausible bug (oracle, negative case, property over seeds).
- Never a new `#[allow(`; an exception is `#[expect(lint, reason =
  "...")]` you would defend to the judge. No new copy of an
  `engine_core` helper. No board IDs in new comments.

Name the file:line for each deduction. Rust score under 3 is FAIL.
Report `RUST: <n>/5` beside `DEPTH: <n>/5`. The writer-side checklist
lives in `engine-code`; this skill is the review-side bar.
