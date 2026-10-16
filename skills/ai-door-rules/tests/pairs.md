# Skill-on vs skill-off pairs: ai-door-rules

Three situations. "Off" is what an agent says from memory; "On" is what the skill makes it say. Pass = the On answer applies the rule, searches or cites the source for each claim, and names the test or measurement that proves it.

## P1: win rate from a few games (eval)

Situation: A builder reports that the new policy beats the old one. The result is 53 wins in 100 games.

Off: "53 of 100 is a clear win. The new policy is better, so ship it."

On: "Rule 5: 53 wins in 100 games proves nothing (p = 0.62). Play about 1000 games, swap sides, and run a binomial test. Search `node tools/books.mjs search "binomial test games"` and cite [gogame 10.3.1-10.3.2]. Report the win count, the game count and the p-value."

What skill-less misses: the p-value. With 100 games, a 53 percent record is within noise, and the builder ships a change that does nothing.

## P2: model call inside the tick (door)

Situation: A builder wants NPCs to ask a model every tick. They write the reply straight into the hashed state, so the NPCs react to it.

Off: "Call the model from the tick and store the reply. The NPCs get smarter and the tick stays simple."

On: "Rule 1: models run outside the tick. Tick-stamp the model action and record it before it enters the tick. Replay reads the record, never the model. Search `node tools/books.mjs search "agent action recorded replay"` and cite [dushkin 19.1]. Rule 7 also applies: no float enters hashed state."

What skill-less misses: replay. A model reply is not repeatable, so the same seed gives a different hash on each run, and the replay check fails.

## P3: door verb with no key and no undo (door)

Situation: A builder adds a door verb that moves a placed object and returns "done". The verb has no idempotency key and no undo. An agent that retries the call moves the object twice.

Off: "The verb moves the object and returns done. Retrying is safe enough."

On: "Rule 2: every door verb takes an idempotency key, can be undone, and states its cost before it runs. Search `node tools/books.mjs search "idempotency undo tool"` and cite [dushkin 11.1-11.3]. The caller's retry and the cost check are in tool-door-patterns."

What skill-less misses: a retry moves the object twice, and the agent has no way to back out. The verb cannot be trusted under retry.
