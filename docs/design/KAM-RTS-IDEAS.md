# KaM-inspired RTS ideas

Source: the kam-web project (a browser port of the KaM Remake simulation, 2026-10). This file keeps what it taught us. It holds ideas, not rows. The owner picks which ones become rows.

## Licence rule

The KaM Remake code is AGPL-3.0, and so is kam-web. Ideas are free. Do not copy that code into a game that is not AGPL. Rebuild from the design, in Rust, with new code.

## What kam-web proved

1. **A deterministic command layer is the whole network game.** Every player action is a small command with a tick number. The sim steps the commands in order. Same seed plus same commands gives the same state hash. Replays, tests, AI and multiplayer all use this one door. engine2040 already has the hash law. Keep every new RTS action as a command from day one.
2. **Lockstep multiplayer is cheap if the sim is deterministic.** A relay only orders and forwards commands. Each client checks a state hash every N ticks. On a mismatch it names the first bad tick. That one message finds most desync bugs.
3. **Freeze a hash, then prove it in a real run.** Each subsystem got a frozen sha and a check that can fail on purpose. A real-browser run covered the parts a hash cannot see. Workers' PASS lines proved nothing until the lead re-ran them.
4. **Spike before port.** We lost days hand-porting 80,000 lines. A later spike showed the real engine could be compiled instead. Research the shortcut first, with a time box.
5. **Oracles beat opinions.** Real replays, a reference decoder (ffmpeg), the original source. Use a ground truth wherever one exists.

## Ideas for the Roman arena and the city

- **Logistics is the strategy.** KaM's delivery system matches every supply with every demand by importance, then by distance cost, with small random terms. Roads and carriers decide who wins. A Roman siege field could be the same: supply lines to the legions, and cutting them is the attack.
- **Difficulty as knobs, not cheats.** kam-web's AI has a start level (none, light, normal, hard: a rich starting army) and a peacetime timer. Without peacetime the AI hit an idle player at 4:31. With 10 minutes it hit at 13:31. Both are one number each.
- **Fog of war that remembers.** Revealed ground stays known, current sight is live. Needed for any strategy view.
- **A rule-based AI supervisor.** One supervisor schedules attack windows and defence. Each AI player only builds and trains. This is easy to test and easy to explain to a player.
- **Notifications and a minimap.** The in-game UI that worked: a build menu, a minimap, a resource bar, short notices when a house is done or a group is attacked.

## Hybrid idea (owner, 2026-10): fighter plus commander

The arena fighter (third person) is also a commander. Group orders (move, attack, hold) go through the same command layer as every other action. The player walks into the fight or steps back and orders. Design questions, not decided:
- One avatar with a command radius, or a separate commander camera?
- Do orders from far away cost time (a runner)?
- Does the fighter's death end the order flow?

## Not now

No row, no crate and no board change comes from this file. Pick a row only when the arena Core proofs in `docs/FOCUS.md` allow it.
