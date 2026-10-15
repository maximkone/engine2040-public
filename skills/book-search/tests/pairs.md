# Skill-on vs skill-off pairs: book-search

Three situations. "Off" is what an agent says from memory; "On" is what the skill makes it say. Pass = the On answer runs a search, cites `[id | heading]` for each claim, and quotes at most 2 short lines.

## P1: culling step with no source (render)

Situation: a builder changes the draw path in `render` and asks whether the culling step should test each bounding sphere against the camera frustum before the draw call.

Off: "Yes, frustum culling is standard. Test each bounding sphere against the six planes and skip the draw."

On: "Search `culling frustum` first. The best hit is [found2 | 9.4 Frustum Culling]. Cite it for the plane test. Evidence needed: the same visible set before and after the change in one captured frame."

What skill-less misses: it states a textbook rule with no cite, so a reviewer cannot check which book or section the rule came from.

## P2: multiplayer loop named as a book fact (engine)

Situation: a builder plans a rollback design for the networked game loop and says the books require it.

Off: "Rollback is the standard way to do multiplayer. Use it."

On: "Search `networked multiplayer game loop`. The hit is [geaen | 7.7 Networked Multiplayer Game Loops]. Rollback netcode is not in these books, so say that. Treat rollback as a design choice to prove with a desync test."

What skill-less misses: it turns a design choice into a book fact, and it does not say the books are silent on the exact technique.

## P3: shared counter between threads (Rust)

Situation: a builder shares a counter between threads in `core` with `Arc` and `Mutex`, and asks if the lock is enough.

Off: "Mutex is fine. Lock, increment, unlock."

On: "Search `shared state mutex threads`. The hit is [trpl | Shared Access to Mutex<T>]: cite it for the lock rule. The hash check still needs cache-and-ordering-checks, because a lock does not fix contention or merge order."

What skill-less misses: it answers from habit and does not name the source, and it treats a lock as the whole answer for a hash that must match at any thread count.
