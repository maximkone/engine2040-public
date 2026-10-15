# Skill-on vs skill-off pairs: frame-loop-rules

Three situations. "Off" is what an agent says from memory; "On" is what the skill makes it say. Pass = the On answer applies the rule, searches or cites the source for each claim, and names the file or measurement that proves it.

## P1: replay from tick 0 in a live loop (runtime)

Situation: A builder needs state at the current tick for rollback in the live window. They replay the match from tick 0 on every tick, because it is exact.

Off: "Replaying from tick 0 is correct and simple. Each tick rebuilds the state exactly."

On: "Rule 1: keep per-tick cost flat as the match grows. Never replay from tick 0 inside a live loop. Step the stored state, and keep a snapshot for rollback. Cite [geaen 7.2]. Measure the tick time at the start and near the end of a long match to show the cost stays flat."

What skill-less misses: the cost grows with the match length. The window slows down as the match goes on, and the builder never sees why.

## P2: a thread per fork (runtime)

Situation: A builder fans out AI lookahead every frame by calling `thread::spawn` once per candidate move.

Off: "Spawn a thread per candidate. The threads are short, so the cost is small."

On: "Rule 2: do not spawn a thread for each fork in a per-frame path. Use a persistent pool, and keep the sim single-threaded. Search `node tools/books.mjs search "thread pool game loop"` and cite [gearus 8.6.3]. Ask for the per-frame thread count and the frame time before and after."

What skill-less misses: the thread start cost on every frame, and the rule that the sim itself stays on one thread.

## P3: a mutex on the audio path (runtime)

Situation: A builder feeds new sound commands to the audio callback through a shared `Mutex` around the mixer. Under load the audio stutters, and the builder blames the device.

Off: "Use a mutex. It is fine if the lock is short. The stutter is the device."

On: "Rule 5: feed the audio callback through a single-writer queue. Search `node tools/books.mjs search "audio callback lock free"` and cite [gems2 29.2; bos ch.5]. The window's mixer `try_lock` at `crates/game/src/bin/arena.rs:518` is the case to fix, and the commands it drops when busy must be counted."

What skill-less misses: the lock on the real-time path, and the silent drop when the lock is busy. The stutter has a cause in the code, not the device.
