# AI-native verification: what to steal next (2026-10-23)

Question this note answers: is engine2040 actually smart, or does it only talk
about it? Read the code, then rank the next five steals by what makes an agent
believe its own result.

## Sources read

- Unity, "MCP servers in game development explained" (2026-06-08): the value of
  an engine MCP is the live scene graph, component and asset context, not a
  text prompt. https://unity.com/blog/mcp-servers-game-development
- Unreal Engine 5.8 experimental MCP (State of Unreal 2026): the editor ships
  the server, so the agent reads and changes the running editor.
  https://www.youtube.com/watch?v=AlV__BFg8qk
- Blender MCP connector guides (2026): the first useful verb is "query the
  active scene structure: objects, materials, modifiers, node trees, animation
  channels", then change, then read back.
  https://www.eigent.ai/blog/claude-blender-mcp
- VeriGraph (ICRA 2026): plans over a scene graph that are checked for
  execution, not just for syntax. https://verigraph-agent.github.io
- WorldAgent (arXiv 2609.33208): an agent that verifies each physical step
  before the next one.

## What engine2040 already has (opened, not claimed)

| Surface | Where | Honest state |
|---|---|---|
| 83 door tools in 18 packs | `crates/endpoint/tests/it/tool_count_pin.rs` (`EXPECTED_ROUTER = 83`) | Live, pinned |
| Scene brief with digest and labels | `crates/endpoint/src/scene_tools.rs` (`scene.brief`) | Live, budgeted |
| 18 frozen spatial checks with `checks_run`, `skipped`, `size_source` | `crates/spatial/src/checks.rs` (`SPATIAL_CHECKS`), `scene_tools.rs` | Live since AD3 (`eeac21f`) |
| Sight, reach, cover queries | `crates/endpoint/src/scene_query.rs` (`near`, `between`, `visible`, `reachable`, `cover_near`) | Live; `reachable` refuses without a nav mesh (AD20 half) |
| Why a view is hidden, affordance tags | `crates/spatial/src/why_hidden.rs` (`why_hidden`), `afford.rs` (`afford_text`, `derive_all`) | Live: `scene.query` kind `why_hidden` answers `occluded_by:<id>` plus one evidence number, and the afford **tags** ride `scene.brief`. The blocker's bounds and cheapest fix, and the afford **text**, are still unwired |
| Typed refusal plus recovery on every call | `crates/endpoint/src/mcp.rs`, `packs/*` | Strong |
| Refusals in the observe ring | `door_dispatch.rs` (`note_refusal`), `log_tools.rs` | Live (AD9 half); the `door.status` verb that read the ring was dropped with its source, so refusals are written but not yet readable through the door |
| Assistant loop with budgets and apply | `crates/endpoint/src/assistant.rs` (EA1d) | Live |
| Provider ladder, cassettes, arbiter | `crates/provider/src/{ladder,cassette,arbiter}.rs` | Live |

## The five steals, ranked

### 1. A claim gate: the engine checks the agent's words
An agent says "landed", "confirmed" or "viewed". The engine answers `backed`
or refuses `claim_not_backed` and names the missing proof. A preview key can
never count as landed. A key the journal dropped answers `unknown`.
Why: this is VeriGraph's execution check at the door, and it is the one verb
that turns "trust me" into a machine answer. Row AD21 already specifies it.
Cost: S. Needs AD2 for the digest half; the key/revision half works today.

### 2. Evidence age on every green
A saved PASS older than the tree is not evidence. Write
`target/proof/<gate>-latest.json` on GREEN only, with the sha and the time;
a record whose sha is not HEAD or an ancestor, or older than the window,
prints FAIL. A red run never overwrites the green record.
Why: WorldAgent's verify-before-next-step, applied to the repo itself. Row
AD4 owns the tool. Cost: S.

### 3. `scene.why`: put the blocker's bounds and fix in the answer
The door already answers `scene.query` kind `why_hidden` with
`occluded_by:<id>` and one evidence number (cm to the occluder). Missing: the
blocker's bounds, the eye-to-target segment, and the cheapest fix (move,
remove, shrink). Blender MCP's scene query is the parity bar; naming the
blocker is past parity, naming the fix is past it again.
Why: an agent can act on a blocker id and its bounds; it cannot act on `false`.
Cost: S - deepen one existing kind, no new verb.
Watch the pack budget: the answer grows inside `scene.query`, not in the list.

### 4. Affordance text at the door
`crates/spatial/src/afford.rs` derives door clearance, choke points, spawn
tiles, cover approaches, animation caps and mistags; the **tags** already ride
`scene.brief` under its byte budget (`scene_tools.rs`). Only the bounded
`afford_text` is unwired. A read that carries the prose gives the agent what a
room is for, not only where its walls are.
Why: the engine's own spatial crate is ahead of its door. Cost: S - and the
lowest of the five, because the tags already answer most of the question.

### 5. Size honesty through one adapter
`scene.spatial` now reports `size_source{collider,default}`. The remaining
lie is that the CAP1 gate still sizes every node as a unit cube. AD7 is the
fix; it is in flight, not done. Until it lands, "clean" means "clean for 1 m
cubes" and the report must keep saying so.
Why: the Forge audit's 541-findings-from-one-unit-error incident. Cost: M.

## What not to steal

- A free-text "run this Python/C#" escape hatch. Both Blender and Unity MCP
  offer one; it makes every claim unverifiable and every write unsettled.
  engine2040's typed commands are the better trade.
- A model in the tick. The engine's bet (same input, same hash, model outside
  the tick) stays.

## Next three, in order

1. AD21 claim gate (S) — the trust verb.
2. AD4 evidence age (S) — the trust record.
3. Deepen `scene.query` `why_hidden` with the blocker's bounds and cheapest fix
   (S), and only then the afford text (XS, the tags are already there).

Rows: AD21, AD4, then a row for the `why_hidden` depth. No new subsystem.
