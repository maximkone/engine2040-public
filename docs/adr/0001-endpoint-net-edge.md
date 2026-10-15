# 0001 Document engine-endpoint -> engine-net edge

- Status: Proposed
- Context: `node tools/workspace-check.mjs` fails today with `engine-endpoint -> engine-net: undocumented local dependency edge`. The edge landed with DP1 (3a1cd34c, net pack loopback). The `allowedEdges` row for `engine-endpoint` in `tools/workspace-check.mjs` does not list `engine-net`.
- Decision: Propose adding `engine-net` to the `engine-endpoint` row of `allowedEdges`. Adding it is the lead's call, not the architect's.
- Consequences: The audit passes on this edge. No code change. No hash change.
- Compliance: `tools/workspace-check.mjs` `allowedEdges` check (`engine-endpoint` row).
- Advice: Lead, via inbox `--ref ADR-0001 --by architect`.
