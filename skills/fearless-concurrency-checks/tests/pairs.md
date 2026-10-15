# Skill-on vs skill-off pairs: fearless-concurrency-checks

Three review cases. Off is habit without the list; On names the check, the failure it stops, and the proof it asks for.

## P1: borrowed vector sent to a spawned worker (checks 1, 2)

Input: diff in `game` builds `let v = vec![1,2,3]` then `thread::spawn(|| show(&v))` with no handle kept. Main keeps using `v` and exits. Author says it runs fine locally.

With skill: Reject per 1 and 2. The closure must take `v` by move with a static bound so main cannot drop it mid-thread; keep the handle and join before use and before shutdown. Proof: build shows the borrow error fixed by move, plus a run where main waits and output is complete.

Skill-less misses: suggests a shared reference or a longer sleep, hides the use-after-drop race, and never asks for the join.

## P2: ten threads bump one counter behind a lock (checks 5, 6, 8)

Input: diff in `physics` wraps `counter` in a lock, clones a single-thread counter handle into 10 spawns, each locks, adds 1, drops the guard. Reviewer sees count 10 once and approves.

With skill: Fix per 5, 6, 8. Shared ownership across threads must be atomic-counted, not single-thread counted; the guard must cover only the increment with poison handled; only one lock taken and released each time. Proof: send-error fixed by the atomic wrapper, ten joins collected, result 10 over repeated runs.

Skill-less misses: keeps the single-thread counter because it compiled once, leaves the guard held too long, and misses the deadlock and poison paths.

## P3: parallel tile bake merged by arrival order (checks 4, 9)

Input: diff in `arena` splits tiles over workers, two senders push meshes to one receiver, main collects in arrival order into a map and walks the map to build the tick graph. Note says 2x faster on one small debug map.

With skill: Hold per 4 and 9. Cloned senders mean arrival order is random, so merge must sort by tile id and never walk map order into the tick; speedup needs release before/after at real tile counts plus the golden hash unchanged. Proof: sorted merge plus repeated hash runs.

Skill-less misses: trusts one lucky order, ships nondeterministic ticks and replays, and accepts a debug anecdote as a budget number.
