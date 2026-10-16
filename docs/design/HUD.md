# HUD — one rule per element (EG14 rule book)

Status: rule book, not code. The HUD pass (EG14: one textured-quad
pipeline after the blit, dynamic quad ring from `ui` layout cells,
bitmap font atlas, one draw call in new
`crates/render/src/overlay.rs` plus the presenter call) reads this
page. `DuelOverlay` in `crates/ui/src/lib.rs` is built and has a
live caller: `crates/game/src/windowed.rs` builds it per frame in
`duel_overlay_for_app` (one named row per live fighter slot with
permille hp from the match), pairs rows in `hp_bars_for_overlay`,
and stages back-plus-fill pairs into the presenter overlay ring in
`stage_hp_bars` (checked with `rg DuelOverlay`; only hp bars ride
the ring today). Names, stamina, poise, timer, banner and marker
below stay EG14 remainder until their quads land.

Proof it serves (EG14 row): Accept — the arena capture shows the hp
bars (pixel test on their screen rect). F2P — that rect holds scene
pixels today. P2P — `cargo test -p engine-render --lib` green.

Reads beside this page: `content/design/controls.rows` (verbs and
feedback flags; gamepad and rumble read it), `content/design/cues.rows`
(sound cues, 40 ms budget; the visual answer rides the next frame, 25 ms),
`docs/VISUAL-TARGET.md` (1920x1080 presentation; sizes below are px at
720p and scale linearly), `crates/game/src/camera.rs` (director),
`crates/game/src/camera_arm.rs` (boom), `crates/game/src/camera_feed.rs`
(feed), spectator cuts (`crates/game/src/lib.rs`).

Methods: health bars as engine-assisted bars [handsrust | Rendering the HUD];
camera states as look-at / follow shapes [geaen | 16.2.2 Cameras];
one key plus one pad button plus feedback per verb from controls v1.

## Elements

`show`: always on, or on demand with its event. `text`: min height in
px at 720p (n/a where the element is bar/icon only). `contrast`: min
ratio against what sits behind it (4.5:1 text, 3:1 large graphics).
`second`: the non-colour channel — no information by colour alone
(Basic tier; `DuelOverlay` already carries `palette_id` 0-3 for
deuteranopia, protanopia, tritanopia). `feel`: the engageability line —
what the player should feel when it changes.

| element | show | event | text | contrast | second | feel |
|---|---|---|---|---|---|---|
| health bars | always, top corners | every tick | numbers 20px, bars 10px tall | 3:1 vs sand | shape: bar length plus permille number | I see exactly how hurt we both are, no guessing. |
| stamina bars | always, under health | every tick | label 16px, bars 8px tall | 3:1 vs sand | shape: bar plus low-flash icon under 25 | I know when I can afford the next swing. |
| fighter names | always, above bars | every tick | 16px | 4.5:1 | icon: fixed corner slot plus palette, never colour alone | I know who is who at a glance. |
| hit-direction indicator | on demand, 300ms | Hit, Blocked, Parried, Dodged | icon 24px, no text | 3:1 | shape: edge arrow plus small event icon, never a full red screen | I feel where the hit came from and can answer it. |
| round timer | always, top centre small | every tick | 18px | 4.5:1 | shape: digits plus a shrinking ring | I feel the round ending and pick my moment. |
| reward banner | on demand, at most 1.2s | Knockdown | 24px | 4.5:1 | sound: the knockdown stinger from cues.rows, short and loud | The kill feels big for one second, then it is gone. |
| director-cut marker | on demand, 1s | spectator-camera cut | 16px | 4.5:1 | icon: camera glyph plus shot name | I notice the camera moved and trust who moved it. |

Layout law: health and stamina live in the corners; the centre stays
clean for the fight. Hit direction is an edge arrow, not a full-screen
wash. The reward banner is short (at most 1.2s) and loud (stinger), then
gone. Visual answers land on the next frame (25 ms); sound answers land
inside the 40 ms cue budget.

Clean-screen toggle: one press hides every always-on element; hits,
banners and markers still flash. The toggle binds in controls v2
(remainder; every v1 verb already rebinds per the `every_verb_rebindable`
test, and block already offers its toggle with `toggle_ok=1`).

Subtitles: any voice line ships with a subtitle line in the same box
style as names (18px, 4.5:1). No voice lines play today (crowd and
stingers only), so this rule waits untested.

## Camera

One row per state: who owns the camera, and the one-line tell the
player gets when it is taken.

| state | owner | take tell |
|---|---|---|
| player-look | player: focus fighter plus the `camera_arm` boom | The eye follows your fighter; the lock target only suggests the frame. |
| director | `camera::Director` plus spectator cuts from fight beats | The cut marker flashes with the shot name on every cut. |
| hybrid | director frames, player trims inside rails (`camera_feed`) | Hands off for 1s eases the frame back to the director rail; never a snap. |
