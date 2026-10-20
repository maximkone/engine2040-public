# Owner decisions, taken by the lead (2026-10-21)

The owner asked for the lead's call on three things and said he was too tired to think them
through. These are the calls, with the reason and who does the work. Recorded here, and the
engine one lands on the board when the lead merges the wave rows.

## D1. The duel brain: GA11 behaviour tree executes, GA12 GOAP plans

**Call: one brain, two layers, both kept, nothing deleted.**

- The duel brain lives in `crates/ai` and is the **only** consumer: the arena's foe brain.
- **GA11** (`crates/ai/src/behavior.rs`) is the per-tick decision layer. It is stateless, actions
  append to an `Op` buffer, traces are byte-identical x2, 264 tests green (landed 453811f).
- **GA12** (`crates/ai/src/goap.rs`) is the planner: A* over a 64-fact integer state, 16-step cap,
  integer only, replans after a state flip, replay x2 equal (landed 49df5811). It plans when the
  goal or the state changes, not every tick.
- The blackboard and the goal stack feed that one brain.

Why this way: it is not "BT or GOAP", it is which one decides per tick. The BT is that one. The
GOAP search is a plan, and planning every tick for 8 fighters buys nothing. The owner's own
folded decision (forge-adoption.md E11-Q2) names GA11 as the sanctioned BT shape with data trees
and a load-time validator, so this follows a decision already made rather than making a new one.
One consumer per module, no second brain, no zoo.

Owner: the smart-engine lane finishes `crates/ai/src/plan_trace.rs`, then the foe brain lands in
`crates/ai` with the arena calling it. `crates/arena` is held by the layers lane, so the arena
call site is a patch-ready spec until that crate frees.

The other seven dead modules: `utility` is live (internal callers only), `blackboard` and
`goalstack` feed this brain, and `director`, `opponent`, `curves_text`, `rows` go on the deletion
manifest with their measured line counts. No module is deleted before its consumer exists.

## D2. Forge's unreachable shelf (100 files, 13,719 lines, 37 kits): it feeds the Rome arena

**Call: the home is forge's arena game. No new project, no new window of its own.**

- The kits and the finished code move into `projects/ForgeArena` and `game/slice` - the one game
  forge is building - so every adopted file has a caller in the arena.
- The kits the arena uses first: `body` (wounds, needs, rations), `ai` and `bt` (the brain),
  `move` (fair jump, roll, knockback, sweep), `motion` (camera boom, lock-on, shake).
- Everything else stays as stock in forge's own ledger, which is the honest record of "finished,
  not yet in a game". It is not deleted and it is not promoted to a project.
- The empire rule is one game per project and Rome is that game, on both engines.

Owner: forge's lead picks the kits per arena milestone and ticks each with a caller. The lead's
wave rows do not touch forge's board.

## D3. The center arsenal (130 tools, 19 with no run file): a contract, so the 19 are a bug

**Call: it is a contract, and a listed tool that cannot run is a broken promise, not a choice.**

- `center/arsenal.json` is how one repo offers a tested tool to another. A line whose run file is
  missing advertises something that cannot be called, and another agent will call it.
- So the registry keeps only entries that have a run file and a test that passes. The tool that
  already checks this is `node center/arsenal.mjs --check` (it validates each manifest and runs
  each tool's test).
- The 19 therefore need no judgement from the owner: each one's owner either lands the run file or
  the line is retired by the path that already exists for it - the toolsmith retirement check plus
  the reuse meter, both read-only and both refusing to act on stale data.
- Nothing is deleted by hand and no row is closed on a guess.

Owner: the center seat runs `node arsenal.mjs --check`, works the 19 one at a time, and reports
the count that passes. The lead does not touch the registry.

## What is still the owner's word

- Which job boards may be added to the jobhunt approve queue: filed as `EB-2026-10-21-S1`.
- Anything that changes a fate or a scope line in `README.md`: still his.
