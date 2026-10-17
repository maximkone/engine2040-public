# 0001 Document engine-endpoint -> engine-net edge

- Status: Proposed
- Context: On 2026-10-12 `node tools/workspace-check.mjs` failed with `engine-endpoint -> engine-net: undocumented local dependency edge`. The edge landed with DP1 (3a1cd34c, net pack loopback). The `allowedEdges` row for `engine-endpoint` in `tools/workspace-check.mjs` did not list `engine-net`.
- Decision: Propose adding `engine-net` to the `engine-endpoint` row of `allowedEdges`. Adding it is the lead's call, not the architect's.
- Consequences: The audit passes on this edge. No code change. No hash change.
- Compliance: `tools/workspace-check.mjs` `allowedEdges` check (`engine-endpoint` row).
- Advice: Lead, via inbox `--ref ADR-0001 --by architect`.

Update 2026-10-17: carried out. Center's fixer added `engine-net` to the `engine-endpoint` row with its own comment (aca0fa6e, 2026-10-12 09:04Z, "document endpoint-net edge in workspace guard"). `node tools/workspace-check.mjs` exits 0 today with no such FAIL line. Status stays Proposed; nothing is left for the lead to decide here, so no inbox ask was filed.
