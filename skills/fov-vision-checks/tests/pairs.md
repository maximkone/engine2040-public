# Skill-on vs skill-off pairs: fov-vision-checks

Three sight situations. "Off" is what a reviewer says from habit; "On" is what the checklist makes it say. Pass = the On answer names the numbered item, the failure it stops, and the proof it asks for.

## P1: one flag for walls and sight (items 1, 5)

Situation: a diff in `crates/spatial` stores one `blocked` bool per tile. Movement and vision both read it. A new arrow slit tile blocks motion but should let sight through; it blocks both. A glass wall tile blocks sight but should let none pass; it is walkable. Render in `crates/render` draws all bodies with no sight test.

Off: "One flag is simpler and covers walls. The slit is an edge case; approve."

On: "Two findings. Item 1: vision blocking needs its own flag apart from movement blocking, or slits and glass invert. Add a test: slit tile denies entry yet keeps the far point in the viewer set; glass admits sight denial while entry stays open. Item 5: draw and hover must test set membership first, or hidden foes leak. Proof: a render test with a foe behind a wall shows no body and no tip."

What the skill-less answer misses: sight and motion are distinct channels; merging them breaks map features and leaks hidden bodies.

## P2: rebuilt each tick and chased through walls (items 3, 4, 6)

Situation: a diff in `crates/game` rebuilds every viewer set each tick for all 200 foes, then runs chase before the sight pass. Foes in `crates/ai` chase the player across the map through walls. The commit says "vision works".

Off: "Rebuild-all is correct and chase-first is fine; 200 foes is small. Approve."

On: "Three findings. Item 3: rebuild only when the owner moves, starting dirty and clearing after the run; show tick cost at 200 viewers before and after. Item 4: run sight after movement settles and before chase and draw in each schedule that moves, or chase reads last turn's spots. Item 6: pursuit must start only when the target point sits inside the chaser set and stop when sight breaks. Proof: a foe behind a wall stays idle, and one that loses sight halts within a turn."

What the skill-less answer misses: per-tick full rebuilds waste the frame, stale ordering makes chase nondeterministic, and ungated chase removes hiding as a tactic.

## P3: recall folded into the live set (items 7, 8)

Situation: a diff in `crates/spatial` keeps explored memory inside the live visible set and clears it on each rebuild. The map in `crates/render` draws only live tiles, so explored halls vanish when the viewer turns. The set is walked in hash order to feed an AI cache in `crates/ai`, and run-to-run order shifts.

Off: "One set is less state and hash order is fast. Approve."

On: "Two findings. Item 7: explored state needs a separate bool array set from the viewer set on move and never cleared; live tiles draw bright and recalled ones dim. Test: walk a hall, turn away, and the hall still draws dim. Item 8: the same origin, radius, and map must yield the same set, and hash-order walks must not feed sim state; sort or snapshot before use. Proof: a determinism test replays the same walk twice and gets the same set, plus a cost number at real viewer counts."

What the skill-less answer misses: recall is cumulative history, not live sight, and unstable order in sim state breaks replays.
