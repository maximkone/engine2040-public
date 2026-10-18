# RZ2w capture: the ragdoll on the sand (receipt)

Row RZ2 (slice RZ2w, e1c8de49). Evidence only, no code touched. Written 2026-10-18.

## Run

- Build: `cargo build --release -p engine-game --features app --bin arena` into `target/bl-gpu` (own target dir, the shared release dir was locked for 40 min by another agent). Source: working tree on top of f222553d (includes e1c8de49 and VR2 landing 2).
- Command (one GPU job, nothing else running on the GPU):
  `arena.exe --staged-fight --scene arena --fighters 4 --capture-dir target/captures/rz2w-a --capture-every 10 --sha 359a0d4d --rows RZ2 --exit --mute`
- Adapter: NVIDIA GeForce RTX 3050 6GB Laptop GPU, driver 32.0.16.1062 (nvidia-smi 610.62), window 1280x720, present mode Immediate, 0 validation errors.
- Scripted knockout: the staged fight (seed 2040, 600 ticks, 4 fighters) lands 27 hits and 3 knockdowns. Sim hash after 600 ticks `ef25059b432c9193` (this staged session, not the golden). The `arena view:` line reports `ragdolls=3`, so the layer spawned three ragdolls.
- Output: 65 PNGs + `manifest.json` + `per_pass_ms.json` + `run.log` in `target/captures/rz2w-a/` (not committed, target/).

## What the pictures show (looked at by eye)

- `frame_000026.png` (tick 256): one fallen fighter lies on the sand in the near-right of the frame. Zoom `crop26_body.png`: head turned to one side, dark torso, one bent leg and one straight leg, one arm up. A sprawled, asymmetric pose, not the standing bind pose and not a flat axis-aligned death clip. A dust cloud from the fall hangs over the pit. In the far crop `crop26_far.png` one fighter still stands, one lies on its back, one is sprawled mid-pit.
- `frame_000045.png` (tick 424): the same kind of body at rest on the sand, one arm raised, legs extended (`crop45_body.png`). The body is still there 170 ticks later, so it settled and stayed down.
- `sheet1.png` / `sheet2.png`: contact sheets. Bodies are lying from tick ~218 on and stay on the floor to tick 600.

## Honest limits

- The director camera is far from the bodies (a fallen fighter is about 90 to 120 px long in a 1280x720 frame). The pose reads, the joints do not. There is no CLI flag for a close camera (`--camera-path` only has `arena600`).
- No ragdoll-off A/B image: there is no flag to switch the layer off in the window, the on/off proof is the unit test (`ragdoll_look_hashes_equal_on_and_off`).
- "No limb 2 cm under the sand" cannot be read from a PNG; that is the unit test's job (7 ragdoll_look tests, board row).
- Still open on the row: window tier wiring (`set_ragdoll_tier`), walls for ragdolls, get-up clip.
