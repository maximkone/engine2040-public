# GGPO rules for a rollback game

Condensed from the GGPO developer guide (pond3r/ggpo at commit 7ddadef, MIT, Copyright (c) 2009-2019 GroundStorm Studios, LLC.), reworded, short ideas kept. The source file is `ggpo:doc/DeveloperGuide.md`. ggrs is a Rust reimagination of GGPO: the three callbacks of the C library became the three requests of ggrs.

## State and inputs

- Only two things matter to synchronization. Game state is everything that decides the outcome: positions, bullets, health, score. Game inputs are everything that changes the state: pad and button presses, and also less obvious things such as the time of day at the start of a frame if the game uses it.
- Renderers, audio and effects that do not change the outcome are neither state nor inputs. Isolate the real state from the rest, for example in one struct, so saving and loading it is trivial (usually a tiny part of all game data; textures, tables and code are constant).
- Each player runs a complete copy of the game. Sending the whole state every frame is too expensive, so ggpo sends only inputs and each peer steps the game forward.

## What the game must be able to do

1. Be fully deterministic: for any given state and inputs, advancing exactly one frame must give identical states for all players.
2. Keep the state fully encapsulated and serializable.
3. Load, save and run one simulation frame without rendering it (rollback needs this).

Advance by a fixed time step per simulation frame, even if drawing uses a variable rate. Rollback re-runs frames one by one, which a variable tick makes hard.

## Update and render apart

The advance step is called many times during one rollback, so sounds and effects produced in it must wait until the rollback is finished. Separate game state from render state: state is decided only by inputs, rendering is driven by the current state, and the state can be advanced with a set of inputs without drawing. In ggrs this is the loop that fulfils all requests first and renders once afterwards.

## Determinism traps

- **Random numbers.** The seed must be the same at frame 0 for every player, and the generator's state must be part of the saved game state. Then a frame always draws the same numbers however many times it is replayed.
- **Wall-clock time.** Clocks of two machines are never in sync. Either remove time from the state calculation, or make the current time of one player part of the frame's input and always use that. Time used only for non-state things (how long an effect is shown, audio attenuation) is fine.
- **Dangling references.** Dynamically allocated memory in the state needs its pointers rebased when saved and loaded; a base plus offset instead of a pointer reduces the work.
- **Static and hidden state.** A `static` counter inside a function cannot be rolled back. Move it into the saved state and restore it in the load step.
- **Use the sync test.** A sync test session runs a one-frame rollback every frame, compares the state of the first run with the replay and reports a difference. In ggrs that is `SyncTestSession` with `with_check_distance(n)`. Run it on developer machines all the time so a desync bug shows up the moment it is written.

## Frame delay versus speculative execution

Latency is hidden in two ways: frame delay (local input is applied a fixed number of frames late) and speculative execution (predict the missing remote input, roll back when wrong). If a packet takes longer than the chosen delay, prediction hides the rest. The delay can be tuned during a game.

- Make the delay as high as the game tolerates without changing how it feels. A fighting game needs pixel-perfect timing: more than 1 frame is noticed by most intermediate players, and experts can notice 1. A board or puzzle game can use 4 or 5 frames.
- A higher delay also reduces glitching in rollbacks: the longer the rollback, the more visible the jump from the wrong prediction to the right state.
- Example from the guide: a screen flash that lasts exactly 2 frames after a button press, a delay of 1 frame and a packet time of 4 frames give a rollback of about 3 frames (4 minus 1). If the flash falls in the first rollback frame it is swallowed and the remote player never sees it. The cure is either a higher frame delay, or a renderer that delays the flash until after the rollback.
- In ggrs the delay is `with_input_delay(n)` on the builder and `set_input_delay(local_handle, delay)` on a running session.

## C library to ggrs

| GGPO C API | ggrs |
|---|---|
| `ggpo_start_session` plus `ggpo_add_player` | `SessionBuilder` with `add_player`, then `start_p2p_session` |
| `save_game_state` and `load_game_state` callbacks (and `free_buffer`) | `GgrsRequest::SaveGameState` and `LoadGameState` with `cell.save` and `cell.load` |
| `advance_frame` callback | `GgrsRequest::AdvanceFrame { inputs }` |
| `ggpo_add_local_input`, `ggpo_synchronize_inputs` | `add_local_input`, and the `inputs` of the advance request |
| `ggpo_idle` | `poll_remote_clients()` once per loop iteration |
| `ggpo_start_synctest` | `SyncTestSession` |
| `on_event` callback | `session.events()` |

In the C library `ggpo_synchronize_inputs` must be called every frame, also during a rollback, and the values it returns (not the ones read from the pad) advance the game; with input delay set, local inputs are returned only after the delay. In ggrs the inputs of every `AdvanceFrame` request play that role.
