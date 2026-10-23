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
| [ and ] | slow-mo speed one preset down or up (25, 50, 100, 200 percent; the title shows `speed N%` when it is not 100) |
| R | rematch (when the bout has ended) |
| T | back to title (when the bout has ended) |
| C | free camera on/off; while on, arrows look, WASD fly, Q/E down/up (view only) |

The window title shows the same list, and while the nearest rival is more than 10 m away it leads with `walk toward the rival with W (N m away)`; it goes back to the plain list once you are close. Mouse buttons are not bound. A gamepad is read through gilrs: the left stick moves you, camera-relative like WASD; face buttons are NOT bound to attack, block or dodge yet (the bindings table has one key per action), so use the keyboard to fight.

Camera-relative means the sim's world wish is rebuilt every tick from where the live camera looks on the floor (`crates/game/src/camera_wish.rs`, fed by the window each frame). It is input mapping only: the sim, the match identity and every hash are untouched, and the staged fight, the `--spear-demo` script and capture tapes keep their world axes. The free camera (`C`) does not steer: W still follows the shot you left.

Soft lock-on: every tick the window fills the human input's `facing` with the bearing to the nearest living rival (`crates/game/src/lock_on.rs`; a tie goes to the lower slot, a rival at 0 hp is ignored). `F` swings and `Q` throws along that exact bearing (the live duel runs rules v2, whose blade and spear read `facing` in fixed point), and the body yaw follows it. With no living rival, or while the free camera (`C`) is on, `facing` stays 0 (world +z) as before; the staged fight and capture tapes own the slot and keep their own facing. Input mapping only: no sim crate, match identity or hashed byte changed; the `--spear-demo` script aims with the same bearing function.

Not exposed: a quality tier flag (tiers exist on the web build only; native has the `E2040_MSAA` env var), a ragdoll switch (it is always on). A second human plays over the network: see "Two players" below.

## What to look for

- Camera (two fighters): over your shoulder, you in the lower third, the rival ahead; it closes to a tight melee shot when you meet and shortens its boom against the pit wall. At the start you are pinned to the wall 38 m from the rival, so he is small until you walk in: hold W and you are in melee after about 11 s (top speed 5 m/s; the gate tunnel mid-pit is in the way, W alone slides around it, A or D steers). `C` is the free camera. The shot is smoothed (`crates/game/src/fighter_look_cam.rs`): a cut or a quick move glides over about half a second instead of jumping, a hit pushes the lens in a little and a hard knockdown rolls the horizon for a moment, then it levels again.
- Walk bob (ported from the owner's Shady Knight, row SG6): while you walk the eye bobs side to side once per stride and up and down twice, about 2 cm of eye travel at top speed, and it fades out within about a second when you stop. A standing hero holds the eye dead still, a hero off the floor or one that jumps across the map in a frame holds it too, and the bob is the same at any frame rate. Numbers and grades: `docs/plan/shady-knight-port.md`. Run it: `play-arena.bat` and hold W. Headless receipt for the walking shot (twice, identical): `arena sim: ticks=620 hash=21ca9f5bbc27bdfd` and `arena capture: ... sha=12e72042102b2c1c` (command in the paragraph below). The rope swing from the same game is in the sim now (rows SG5, SG4: a fighter height channel plus the wall anchor as a rope, gravity landing the release) and `--height` arms it for the window (row SG7).
- Look: soft contact shadows under the fighters (they follow the bodies), a warm arena-sky ambient (no studio reflections, no white streaks). Before/after: `docs/receipts/2026-10-18-arena-look/README.md`.
- Break crate: the small wooden box in the centre. A sword hit breaks it into debris that falls and settles.
- Ragdoll: a knocked-out fighter falls as a loose body, then gets up when the sim says so.
- HUD: one red hp bar per fighter, bottom left, top bar is you (slot 0). A bar empties as hp drops. There is no text on screen; the console prints `arena TITLE` / `arena RESULT P0|P1|draw ticks=a/b` lines.
- Receipt lines (only printed when you run with `--frames N --exit`): `arena view:` (`ragdolls=` more than 0 after a knockdown, `sunk_cm_max`, `pops`), `arena break:` (`breaks=`, `debris=`), `arena sim:` (the frozen sim hash). Meanings: `skills/door-and-headless-runbook/SKILL.md`, section "Window run and the view line".
- A bad run warns itself: when a run breaks a product floor (non-finite vertices, a skin error part, the camera inside a body or under the sand, a body sunk past 3 cm), the arena prints one `arena WARN: <metric> value=<v> floor=<f>` line right after `arena view:`. A clean run stays silent; the floors are pinned to `tools/product-bar.json`.

## Proof of the demo length

The 100,000-tick length is gated, not assumed. `tests/proof_cli.rs::demo_100k`
plays the demo's match twice from fresh state (seed 2040, two fighters, rules
v2 on the kit walls, one gladius per slot, spear on; slot 0 scripted, slot 1 a
brain) and requires the same 16-hex final checkpoint hash. It is `#[ignore]`d,
so ordinary CI stays at the 600-tick golden and the 1,200-tick arena matrix.

`node tools/gh-check.mjs --budget 1700 cargo test -p engine2040 --release --test proof_cli demo_100k -- --ignored --exact --nocapture`

First receipt (2026-10-23, run 40017917792, ubuntu-x86_64, both runs in 0.802 s):
`demo_100k: ticks=100000 hash_a=c600e309490b8095 hash_b=c600e309490b8095 wall_s=0.802`.
The hash is one-machine evidence, not a pinned cross-machine constant yet.
The resume gap (a save carries counters, not the match) is written up in
`docs/receipts/2026-10-23-demo-100k-resume-gap.md`.

**An agent plays the live window.** `--agent-seat PORT` hands slot 0 to one loopback client: the client sends one validated fighter input per fixed tick and reads the match hash back, so an agent plays the same build a person does. The seat owns slot 0 (the boot line says so) and starts the bout like `--play` (a seat has no Enter key), and `--staged-fight`, `--spear-demo`, `--tape` and `--host/--join` refuse to mix with it. Run:
`target\release\arena.exe --scene arena --fighters 2 --frames 150 --exit --agent-seat 7117`
One line each way, `\n` terminated: the client sends `input <tick> <mx> <mz> <facing> <buttons>` (`buttons` is the legal bit set, `0..=55`; `JUMP` is `0x20` and `ping` is ignored); the window sends `tick <tick> <hash:016x>` after every fixed tick and `view <arena view: ...>` every 60 ticks. The run ends with `arena seat: port=... clients=1 inputs=... late=0 held=0 invalid=0 dropped=0`. Two runs with the same inputs read the same hash at the same tick (2026-10-20: tick 20 `dcc10f28eaabb591` twice; the runs presented 25 and 45 ticks, so the final `arena sim:` hashes differ by design). The door's `seat.*` verbs are next (row AN2).

Scripted run with screenshots (no hands needed):
`target\release\arena.exe --scene arena --fighters 4 --staged-fight --frames 700 --exit --capture-dir target/clips/demo --capture-every 70 --fps 30 --sha abcdef1`
(`--staged-fight` plays a scripted fight with knockdowns and a crate break; the PNGs land in `target/clips/demo`).

**Takeover flags play your slot: your keys do nothing while they run.** `--staged-fight`, `--spear-demo` and a bound capture tape drive slot 0 by script; the console prints `script plays you (...)` and the window title leads with it. `--hold W` is milder: it holds named keys from the first tick and your other keys still work.

Hands-off walk through the real keyboard road (`--hold W` holds keys from the first tick: W A S D F Q SPACE SHIFT, comma list; `--pause-at-tick` paces one tick per frame so the walk lands on exact ticks):
`target\release\arena.exe --scene arena --fighters 2 --play --spear --hold W --pause-at-tick 620 --frames 640 --exit --capture-dir target/captures/w-walk --capture-every 310 --fps 30 --sha abcdef1`
(proof of W walking toward the rival: `docs/receipts/2026-10-18-arena-look/`).

## Spear (grapple)

`--spear` is in this build and `play-arena.bat` passes it: Q throws a spear that tethers a fighter or a wall and the pair flies as ragdolls (plan: `docs/plan/spear-grapple.md`). `--spear-demo` plays the human slot by script (walks at the nearest fighter, aims, throws) for captures. By hand, Q throws along the soft lock-on bearing, so a throw from inside 12 m goes at the nearest living rival; hold Q to keep the tether, tap it again to let go. With the free camera on (`C`) the throw flies along world +z as the old window did.

`--height` (rows SG5 and SG4) arms the fighter height channel: fighters fall and land under gravity, and a spear thrown at a **wall** becomes a rope instead of a winch. The hook holds you at the distance you hit it at, the reel takes rope in, and your own run bends into an arc around the anchor; tap Q again to release with the arc's speed and land. A throw at a *rival* is still the old straight zip. Try it: `play-arena.bat --height`, walk to a wall, run along it and throw Q into it.

## Two players

Status 2026-10-18: PROVEN ON ONE PC, NOT ON TWO NETWORKS. `node tools/arena-net.mjs --exe <arena.exe>` starts a real host process and a real guest process over loopback and passed 6 runs: plain duel hash `8bcc8890475af689` (x2), spear duel hash `244f2e2c63a267d3` (x2), and plain duel with injected loss and reordering (about 40 packets dropped, over 100 reordered) again `8bcc8890475af689` (x2); host and guest hashes were equal every time. Not run yet: the relay path (`net_relay`) and a real LAN or internet run between two machines (operator step), and nobody has played it with a keyboard yet.

Two people, two Windows PCs with the same build (or two windows on one PC). The host plays slot 0, the guest slot 1; with `--fighters 3` or more, AI bots fill the other slots. The code the host prints carries the whole match (seed, length, fighters, spear), so the guest passes nothing else.

On one LAN:

1. Host: double-click `play-arena-host.bat`. The console prints (your numbers differ):
   `arena net: hosting slot 0 on 0.0.0.0:7043 (duel+spear, 100000 ticks, 2 fighters), waiting for a guest`
   `arena net: invite code E2040NET-v1-00000000000007f8-100000-2-41a00000-duel-spear`
   `arena net: a guest on the LAN types: arena.exe --scene arena --join 192.168.1.20:7043 E2040NET-v1-...-duel-spear`
   The first time, Windows Firewall asks about arena.exe: allow it on private networks (the guest reaches UDP port 7043).
2. Guest: `play-arena-join.bat 192.168.1.20:7043 E2040NET-v1-...-duel-spear` (or double-click it and paste the address and the code when asked).
3. The host console says `a guest joined`, the guest's says `joined, round trip N ms, inputs sent K ticks ahead`, and the bout runs in both windows.

The same without the bats: host `target\release\arena.exe --scene arena --fighters 2 --spear --ticks 100000 --host` (`--bind IP:PORT` picks another socket), guest `target\release\arena.exe --scene arena --join HOST:7043 CODE`.

Across the internet (no open port on either PC): a third machine both can reach runs the relay.

1. Relay machine (a small VPS, or any PC with UDP 7041 forwarded to it): `cargo build --release -p engine-net --bin net_relay`, then `target/release/net_relay --bind 0.0.0.0:7041`. It prints `net-relay: serving on 0.0.0.0:7041` and a line per registration.
2. Host: `play-arena-host.bat --relay RELAY_IP:7041`. It prints `the guest types: arena.exe --scene arena --join RELAY_IP:7041 E2040NET-...-r<16 hex>` (the `-r...` tail is the host's relay id).
3. Guest: `play-arena-join.bat RELAY_IP:7041 E2040NET-...-r<16 hex>`.

How it plays:

- The host has no input delay. The guest sends each input a few ticks ahead (the measured round trip plus 2 ticks; about 4 ticks, 67 ms, on a LAN) and its window shows only the host's authority: no prediction, so nothing ever snaps back. W, A, S, D are camera-relative and F and Q aim at the nearest rival for the guest exactly as for the host; the guest's camera rides its own shoulder.
- The host plays a tick only once the guest's input for it arrived: a slow or lossy link slows the match down (the host receipt counts `stalls=`), it never makes the two windows disagree. Esc on either side pauses both.
- If the guest is silent for 5 s the host plays on with AI in the guest's slot and takes the guest back when it returns (it catches up from the host's re-sent windows).
- R (rematch) and T (title) do nothing in a network match: restart both sides.
- A code for another match (edited spear, length or seed, or another build or content) is refused on both consoles: the host prints `REFUSED a guest whose code names another match`, the guest stops with `the host plays another match (host identity ..., this code ...)`.
- When the bout ends both consoles print `arena net: receipt role=... ticks=... hash=... stalls=... repairs=... rtt_ms=... lead_max=...`: the two `hash=` values are equal.

Proof on one PC without a window: `node tools/arena-net.mjs --relay-exe target/release/net_relay.exe` runs host and guest as two real `arena.exe --headless` processes over loopback with scripted inputs: the plain duel, the spear duel, the duel with `--net-fault 2,2,5,5` on both sides (2 ticks latency, up to 2 more of jitter, 5 percent loss, 5 percent reorder) and the duel through `net_relay`, each twice. It checks both peers print the same `arena sim:` hash, the two runs agree, the faulted and relayed duels end on the plain duel's hash and the spear duel does not; the receipt lands in `target/arena-net/receipt.json`. A run on two real networks is an operator step: the commands above on two PCs.

## In the browser

Open https://maximkone.github.io/engine2040-public/ in Chrome or Edge with WebGPU (the same GPU is used). It loads about 3.6 MB of wasm, runs the golden check, then draws the arena. Healthy page lines:

- `golden d57b5c8ae3096614 state 2aca9338670aa0cb corpus b37fc6d66df2d417 OK` and `tape 600 ticks ...`
- `arena ready in ~2000 ms`, `tier=web ... pipelines=22`, `validation_errors=0`
- `frame N` growing. If it stays at a low number the tab is hidden; bring it to the front.
- The web arena draws simple coloured box fighters, not the skinned ones the exe shows. Audio stays off until you press a key. The page lists `input: kb=.. mouse=.. pad=..` so you can see whether your keys register. `?square` opens a plain WebGPU shell.

## Browser multiplayer

A person on the Pages site can play a native server. The page is https, so it reaches the server only over WebTransport with a pinned certificate (`cert=`), or over `wss://` behind a TLS tunnel; a plain `ws://` address on another machine is blocked by the browser as mixed content and the page says so. Chrome or Edge (other browsers do not take a pinned certificate for WebTransport yet). This was proved on loopback in CI (`web-build` `net` job: the server, a native client and headless Chrome end on one hash; a killed page is AI-filled and a second page takes the seat back; real key events drive a live seat). It has not been run across two real networks; that is the first thing to do with the steps below.

What you get: the page joins one duel (rules v2, kit walls, you against an AI fighter, or against a second person with `--seats 2`) and plays it through the same rollback client the native tests use. The `net:` line on the page shows the leg (`webtransport`), the tick, the ping, the rollback depth and how old the newest server state is; a desync is named with its first divergent tick; the last line carries the match hash, which must equal the `net-serve: done ... hash=` line on the server. The picture is still the procedural arena: the fighters of the duel are not drawn yet, so you steer by the numbers.

On the machine that serves (needs `cargo build --release -p engine-net --features websocket,webtransport --bin net_serve` once; `cargo build --release -p engine-web --bin net_join` too for the native seat):

1. `serve-browser-duel.bat HOST` where HOST is the name or address the other PC reaches this machine by (public IP, DNS name, or the LAN address for a test). It runs `net_serve --ws 0.0.0.0:7050 --wt 0.0.0.0:7050 --udp 0.0.0.0:7051 --live --ticks 3600 --repeat --host HOST` (one 60 second match after another).
2. WebTransport is UDP. Allow UDP 7050 in the Windows firewall (`netsh advfirewall firewall add rule name="engine2040 duel" dir=in action=allow protocol=UDP localport=7050-7051`, as administrator) and forward UDP 7050 on the router to this machine. TCP 7050 is the WebSocket leg; the https page cannot use it without a TLS tunnel, so it needs no forward.
3. The server prints `net-serve: webtransport https://... cert-sha256=<64 hex>` and a `net-serve: link https://maximkone.github.io/engine2040-public/?join=HOST:7050&cert=<64 hex>&ticks=3600&play` line. The certificate is minted at every start (13 days at most); restart means a new link.

On the other PC: open that link in Chrome or Edge. Keys: WASD move, F attack, Shift block, Space dodge, Q and E turn (click the page for mouse look). The server log shows `joined slot=0 via=webtransport`, then `done ticks=3600 hash=...`; the page's last `net:` line shows the same hash. If it says `webtransport leg failed: no answer within 5 s`, UDP 7050 does not get through (a firewall on either side; many offices block UDP). Chrome 142 and later may ask once to allow access to the local network when HOST is a private address: allow it.

Two people: start the server with `--seats 2` and open the link in two browsers (the match starts when both are in). A native seat (`net_join --server HOST:7051`, scripted, no keys) takes the second seat the same way, which is how CI proves a native and a browser client on one hash.

When UDP is blocked: put a TLS tunnel in front of the WebSocket port (for example `cloudflared tunnel --url http://localhost:7050`, not tried here) and open `https://maximkone.github.io/engine2040-public/?server=wss://THE-TUNNEL-HOST&ticks=3600&play`.

The server prints one `net-serve: admission slot=N ...` line per seat: the packets, the inputs admitted and every refusal by kind, so a cheating or broken client shows up as `cheats=` and `refused=`.
