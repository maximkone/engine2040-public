# Skill-on vs skill-off pairs: gaffer-netcode

Three netcode review cases. "Off" is what an agent says from memory;
"On" is what the skill makes it say. Winner names the answer that cites
a rule, an article section and a crates/net file.

## P1: client sends its own position

Situation: A builder lets each client send its current position each tick.
The server trusts it. Speed hacks appear in playtests.

Off: "Trust the client position. It saves server work and feels fast."

On: "Rule 1: the server owns the world (§Client/Server). Clients send
inputs via `InputPacket`, never positions. Keep authority in `Server` at
`crates/net/src/session.rs:201`."

Winner: On. Off blesses the cheat; On names the rule and the file.

## P2: whole snapshots every tick

Situation: A builder sends the full world snapshot every tick to all peers.
Bandwidth blows past budget at 20 players.

Off: "Full state every tick is simplest and always correct. Buy bandwidth."

On: "Rule 4: snapshot the shared truth (§Client/Server). Save full state
with `save_snapshot` at `crates/net/src/snapshot.rs:38`, then send deltas
against confirmed state only."

Winner: On. Off ignores cost; On gives the delta rule and the file.

## P3: remotes snap to the newest packet

Situation: Remote fighters teleport between updates. The builder shows the
latest snapshot at once and blames the network.

Off: "Show the newest data at once. Old data is wrong by definition."

On: "Rule 5: show others in the past (§Entity Interpolation). Render
remotes through `SnapshotInterpolator` at `crates/net/src/interp.rs:212`
and check path health in `resolve_path` at `crates/net/src/online.rs:63`."

Winner: On. Off causes the jitter; On names the smoother and the check.

Score: On wins 3 of 3.
