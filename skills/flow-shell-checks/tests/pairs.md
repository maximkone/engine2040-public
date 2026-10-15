# Skill-on vs skill-off pairs: flow-shell-checks

Three shell reviews. Off is habit; On names the check, the failure it stops, and the proof it asks for.

## P1: pause that keeps stepping (checks 5, 8)

Input: a diff in `game` adds Paused to the window. Pause skips present but still calls `tick_into`. Note says 60 paused frames look right.

Answer with the skill: Reject. Check 5: pause must halt the accumulator in crate game so 60 paused frames add zero ticks. Check 8: add `flow_cycle` in crate game and show a pause-in-match bout hashes equal to one without, golden x2 unchanged.

What a skill-less answer misses: it approves hidden ticks. Replay and the golden drift because the tick count moved during pause.

## P2: result with no rematch (checks 3, 6)

Input: a diff in `game` adds Result text and a quit button. Scores stay in a static. The next match reuses them.

Answer with the skill: Reject. Check 3: each objective needs its success and its failure move in crate game. Check 6: Result must lead to the next match with prior state cleared in crate game. Add a test that plays Title Match Result Match and checks fresh state and seed path.

What a skill-less answer misses: it calls text a shell. Old scores leak and there is no next-match path.

## P3: chunk id used as objective id (checks 2, 4)

Input: a diff in `game` keys progress by chunk file name. Renaming a chunk resets saves. Flow rules sit beside mesh refs in one table.

Answer with the skill: Reject. Check 2: objectives are a sequence tree or graph in crate game, not one per chunk. Check 4: chunks and objectives change apart in crate game with maps by id. Split the tables and test a chunk rename with progress intact.

What a skill-less answer misses: it ties world layout to game flow. Content edits break saves and block separate work.
