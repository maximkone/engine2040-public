# Arena demo: play it

## On this PC (Windows, RTX 3050)

1. Double-click `play-arena.bat` at the repo root. A 1280x720 window opens and the fight starts at once.
2. You are fighter slot 0. One AI bot fights you. The bout is long (100000 ticks, about 27 minutes).
3. Quit: close the window (the black console shows the log; Ctrl+C there also quits).

If `target\release\arena.exe` is missing, build it once:
`cargo build --release -p engine-game --features app --bin arena`.

Exact command the bat runs:
`target\release\arena.exe --scene arena --fighters 2 --play --spear --ticks 100000`
Extra flags go after the bat name, e.g. `play-arena.bat --fighters 4` (you vs three bots, round decided by your hp against the summed field) or `--seed 7` (another seed).

## Controls (read from `crates/input/src/bindings.rs`, `crates/game/src/bin/arena.rs`, `crates/game/src/lock_on.rs`)

| key | does |
|---|---|
| W A S D | move relative to the camera: W walks where the camera looks (at the start, toward the rival), S back, A and D strafe; a diagonal is one full step, not faster |
| F | attack: swings at the nearest living rival (soft lock-on, see below) |
| Q | throw the spear at the nearest living rival (only with `--spear`, which `play-arena.bat` passes) |
| Left Shift | block |
| Space | dodge (also Enter or Space at the title if you start without `--play`) |
| Esc | pause / resume |
| R | rematch (when the bout has ended) |
| T | back to title (when the bout has ended) |
| C | free camera on/off; while on, arrows look, WASD fly, Q/E down/up (view only) |

The window title shows the same list, and while the nearest rival is more than 10 m away it leads with `walk toward the rival with W (N m away)`; it goes back to the plain list once you are close. Mouse buttons are not bound. A gamepad is read through gilrs: the left stick moves you, camera-relative like WASD; face buttons are NOT bound to attack, block or dodge yet (the bindings table has one key per action), so use the keyboard to fight.

Camera-relative means the sim's world wish is rebuilt every tick from where the live camera looks on the floor (`crates/game/src/camera_wish.rs`, fed by the window each frame). It is input mapping only: the sim, the match identity and every hash are untouched, and the staged fight, the `--spear-demo` script and capture tapes keep their world axes. The free camera (`C`) does not steer: W still follows the shot you left.

Soft lock-on: every tick the window fills the human input's `facing` with the bearing to the nearest living rival (`crates/game/src/lock_on.rs`; a tie goes to the lower slot, a rival at 0 hp is ignored). `F` swings and `Q` throws along that exact bearing (the live duel runs rules v2, whose blade and spear read `facing` in fixed point), and the body yaw follows it. With no living rival, or while the free camera (`C`) is on, `facing` stays 0 (world +z) as before; the staged fight and capture tapes own the slot and keep their own facing. Input mapping only: no sim crate, match identity or hashed byte changed; the `--spear-demo` script aims with the same bearing function.

Not exposed: a quality tier flag (tiers exist on the web build only; native has the `E2040_MSAA` env var), a second human fighter (only slot 0 is human), a ragdoll switch (it is always on).

## What to look for

- Camera (two fighters): over your shoulder, you in the lower third, the rival ahead; it closes to a tight melee shot when you meet and shortens its boom against the pit wall. At the start you are pinned to the wall 38 m from the rival, so he is small until you walk in: hold W and you are in melee after about 11 s (top speed 5 m/s; the gate tunnel mid-pit is in the way, W alone slides around it, A or D steers). `C` is the free camera.
- Look: soft contact shadows under the fighters (they follow the bodies), a warm arena-sky ambient (no studio reflections, no white streaks). Before/after: `docs/receipts/2026-10-18-arena-look/README.md`.
- Break crate: the small wooden box in the centre. A sword hit breaks it into debris that falls and settles.
- Ragdoll: a knocked-out fighter falls as a loose body, then gets up when the sim says so.
- HUD: one red hp bar per fighter, bottom left, top bar is you (slot 0). A bar empties as hp drops. There is no text on screen; the console prints `arena TITLE` / `arena RESULT P0|P1|draw ticks=a/b` lines.
- Receipt lines (only printed when you run with `--frames N --exit`): `arena view:` (`ragdolls=` more than 0 after a knockdown, `sunk_cm_max`, `pops`), `arena break:` (`breaks=`, `debris=`), `arena sim:` (the frozen sim hash). Meanings: `skills/arena-capture-and-view-line/SKILL.md`.

Scripted run with screenshots (no hands needed):
`target\release\arena.exe --scene arena --fighters 4 --staged-fight --frames 700 --exit --capture-dir target/clips/demo --capture-every 70 --fps 30 --sha abcdef1`
(`--staged-fight` plays a scripted fight with knockdowns and a crate break; the PNGs land in `target/clips/demo`).

Hands-off walk through the real keyboard road (`--hold W` holds keys from the first tick: W A S D F Q SPACE SHIFT, comma list; `--pause-at-tick` paces one tick per frame so the walk lands on exact ticks):
`target\release\arena.exe --scene arena --fighters 2 --play --spear --hold W --pause-at-tick 620 --frames 640 --exit --capture-dir target/captures/w-walk --capture-every 310 --fps 30 --sha abcdef1`
(proof of W walking toward the rival: `docs/receipts/2026-10-18-arena-look/`).

## Spear (grapple)

`--spear` is in this build and `play-arena.bat` passes it: Q throws a spear that tethers a fighter or a wall and the pair flies as ragdolls (plan: `docs/plan/spear-grapple.md`). `--spear-demo` plays the human slot by script (walks at the nearest fighter, aims, throws) for captures. By hand, Q throws along the soft lock-on bearing, so a throw from inside 12 m goes at the nearest living rival; hold Q to keep the tether, tap it again to let go. With the free camera on (`C`) the throw flies along world +z as the old window did.

## In the browser

Open https://maximkone.github.io/engine2040-public/ in Chrome or Edge with WebGPU (the same GPU is used). It loads about 3.6 MB of wasm, runs the golden check, then draws the arena. Healthy page lines:

- `golden d57b5c8ae3096614 state 2aca9338670aa0cb corpus b37fc6d66df2d417 OK` and `tape 600 ticks ...`
- `arena ready in ~2000 ms`, `tier=web ... pipelines=22`, `validation_errors=0`
- `frame N` growing. If it stays at a low number the tab is hidden; bring it to the front.
- The web arena draws simple coloured box fighters, not the skinned ones the exe shows. Audio stays off until you press a key. The page lists `input: kb=.. mouse=.. pad=..` so you can see whether your keys register. `?square` opens a plain WebGPU shell.
