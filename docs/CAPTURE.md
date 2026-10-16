# CAPTURE — gameplay for trailers

Two ways to record. Use the built-in dump first. Use OBS only for live play.

## Built-in dump

Run the window build with capture flags:

`cargo run -p engine-game --bin arena -- --capture-dir target/clips/<name> --capture-every 2 --tape <tape> --rows <ROW1> --sha <build-sha> --fps 30`

Output is `frame_000001.png` files plus `manifest.json` plus `cutlog.json`.
Same tape plus same build gives the same PNGs twice.

## Fixed step

Sim runs at fixed 60 ticks per second. Frames follow ticks.
For smooth clips use `--camera-path arena600` and `--capture-every 2` at 30 fps.
Headless check: `cargo run -p engine-game --features app --bin staged_fight -- --ticks 600` writes 4 fighter
frames (128x96 PNG, bodies drawn, hits flashing) plus `captures/manifest.json` and `receipt.json`.
The `app` feature is required: the frame raster lives in the app leg.

## OBS fallback

Use when you need live input. Set 1080p60, game capture, no scaling.
Lock the camera path. Record full screen. Keep the raw `.mkv`.

## Clip naming

Name the folder `YYYYMMDD-scene-rows`, such as `20261005-duel-GX1`.
Keep `frame_######.png` names as written. Never rename frames.

## Handoff to media-studio (video lane)

Copy the whole clip folder to the media-studio video lane intake.
Include the manifest, the cutlog, and the build SHA.
Keep `target/` out of git. Large video stays out of the repo.
