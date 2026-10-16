---
name: gate-red-fix
description: Use when a gate or CI check goes red in engine2040 and complements engine-review with a red-gate loop plus a smallest fix and a same-check rerun before any claim
version: 0.1.0
author: skillworks
---

# Gate red fix

Use when a gate or CI check goes red. Read the failure text first, make one smallest fix, then re-run the same check.

1. **Read the failure text first.** Open the red log and quote the failing step plus the error line before any edit. Never guess from the check name.
2. **Repro the same check.** Run the exact red command with the same flags and the same scope. Never swap in a greener check.
3. **One smallest fix.** Fix one cause inside owned paths only. No refactors, no extra files, no weakened asserts or gates.
4. **Re-run the same check.** The same command must go green after the fix. A different green check proves nothing.
5. **Hand depth to engine-review.** Depth, evidence bar, Eyes on captures and API audit stay in engine-review. This skill owns only the red-to-green loop.
6. **Keep truth from test-truth-rules.** No skipped legs and no wall-clock asserts. Write the expected outcome down before the rerun and quote command plus result.

## Test note

Write the failing command down before the fix.
Re-run that same command after the fix.
Paste both exits in the report.

source: composio gh-fix-ci with pr-review-ci-fix merged, ported not copied
