# Gate card (blank, one per planned cross-crate refactor)

## Refactor

Date:
Goal:
Sources:

## Boundary map

In play crates:
Exclusions (generated, vendor, lockfiles, local cargo config):

## Blast radius

| batch | crates + files | exclusions | risk (security/breaking/arch/perf/style) |
|---|---|---|---|
| | | | |
| | | | |

Rule: every touched crate named, style never above a higher item.

## Staged batches

| batch | work | diff preview | per-batch CI | rollback line | approval |
|---|---|---|---|---|---|
| | | | | | |
| | | | | | |

Rule: one CI check plus one rollback line per batch; never share one approval across two high-risk batches.

## Approval state

- [ ] Batch 1 approved, CI green.
- [ ] Batch 2 approved, CI green.
- [ ] Batch 3 approved, CI green.
- [ ] HALT: no next batch without human approval.

## Unknowns

- [ ]
