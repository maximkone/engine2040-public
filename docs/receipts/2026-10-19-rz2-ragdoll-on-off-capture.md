# RZ2 capture: the ragdoll on the sand, on and off (receipt)

Row RZ2 (slice RZ2w, e1c8de49, plus `--no-ragdoll` 2ca37f68). Evidence only; the
only code this receipt adds is the off switch. Written 2026-10-19.

## Why a new pair

The 2026-10-18 receipt (`2026-10-18-rz2w-ragdoll-capture.md`) named two limits: no
way to switch the layer off in the window, and a far director camera. `--no-ragdoll`
(2ca37f68, the model of `--no-break-look`) is that switch, so the same staged fight
now runs once as it is and once with the flat death clip.

## Runs

- Build: `cargo build --release -p engine-game --features app --bin arena`,
  `target/release/arena.exe` from the tree of 2ca37f68.
- Adapter: NVIDIA GeForce RTX 3050 6GB Laptop GPU, driver 32.0.16.1062, window
  1280x720, present mode Immediate, `validation_errors=0`, one GPU job at a time.
- On:
  `arena.exe --staged-fight --scene arena --fighters 4 --capture-dir target/captures/rz2-ab/on --capture-every 10 --sha 2ca37f68 --rows RZ2 --exit --mute`
- Off: the same line plus `--no-ragdoll`, `--capture-dir target/captures/rz2-ab/off`.

## The numbers (the on/off proof)

| line | on | off |
|---|---|---|
| `arena view:` `ragdolls=` | **3** | **0** |
| `arena sim: ticks=600 hash=` | `ef25059b432c9193` | `ef25059b432c9193` |
| `arena staged-fight: hits= knockdowns=` | `hits=27 knockdowns=3` | `hits=27 knockdowns=3` |
| `ct7=` bundle shas | `522e78a8,0c1b528c,9a341a41,a9d1f3f9` | the same four |
| `arena view:` `pops=` / `pop_acts` | `181`, no `down` act | `205`, `down:18` |
| `arena view:` `sunk_cm_max=` | `14.1` | `14.2` |
| `arena capture:` frames / sha | `70` / `8d63a385da28ca51` | `71` / `d185e4518b8cc0cb` |

The sim hash is the frozen staged-fight digest `ef25059b432c9193` on both runs: the
authority is untouched by the picture (the same claim as
`ragdoll_look_hashes_equal_on_and_off`, now in the window). With the layer on the
floored bodies stop popping the death clip (`down` leaves `pop_acts`); `sunk_cm_max`
is the same on both runs, so the 14 cm is the pre-existing live-body sand rest, not
a ragdoll limb under the sand.

## What the pictures show

- `on/frame_000026.png`: two floored fighters on the sand. The near one is sprawled
  forward mid-pit, arms out and legs behind it; the far one lies on its side with one
  arm raised. Asymmetric, physics-driven poses, not the bind pose and not a flat
  axis-aligned death clip.
- `off/frame_000045.png`: the same sand with the floored fighters in the death clip
  lying pose (3 bodies down, `ragdolls=0`).
- `on/frame_000045.png`, `off/frame_000026.png`: the same kind of body at other
  capture indices.

## Honest limits

- The two runs are not pixel-aligned: the director camera smooths on wall time, so
  the same capture index is not the same moment (`frames=700` vs `707`). The on/off
  proof is the receipt-line pair above and the unit test, not an image diff.
- The camera is still far: a body is about 90 to 120 px long in 1280x720. The pose
  reads; the joints do not. The joint truth (above the sand, no explosion, settles in
  3 s) is the unit test's job.
- `arena.exe` prints the debris `physfx:` line (the crate); the ragdoll world's own
  counters stay in `FighterLook::ragdoll_receipt()` (unit test only).
- Still open outside this row's Accept: walls for ragdolls (kit collision, RZ5), a
  get-up clip (content), and a hardware tier pick that calls
  `FighterLook::set_ragdoll_tier` (the window runs the Comfort default, and
  `break_window::WINDOW_TIER` is Comfort too).

## Tests

`cargo test -p engine-game --features app --lib ragdoll_look`: 10 passed, 0 failed
(local leg of `node tools/gh-check.mjs`, feature `app` is Windows/GPU-only).
