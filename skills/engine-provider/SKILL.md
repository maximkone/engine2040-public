---
name: engine-provider
description: Use when testing engine2040 provider adapters - conformance matrix, recorded/offline replay, portable contracts. Use for PROVIDER rows.
---

# Engine provider

Take one PROVIDER row with exact adapter/test files in
its owned paths, crate and acceptance. Inspect existing contracts and
test both distinct available implementations against capabilities,
structured actions, streaming, timeouts, cancellation, malformed
output, unavailable service and resource limits. Add recorded/offline
replay.

No provider-specific fields in portable contracts and no network
calls inside the fixed tick. Distinguish fixture/contract tests,
adapter integration and real provider execution. Two mocks do not
prove two providers; absent credentials or a second implementation
are explicit gaps. Live calls require owner authorization in chat and
a call/cost budget; never read/print secrets or change pins.

Return patch, conformance matrix, exact checks, failing contract and
unknowns. Commit by path (skill `commit-by-path`); no secrets in the tree.
