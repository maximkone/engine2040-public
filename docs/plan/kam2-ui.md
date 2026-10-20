# KAM2 UI - the sidebar, the pointer and the receipt

Owner: kam2-game window and kam2-web page. Authority for numbers stays
`kam2-style-lock.md`; this file only says how the screen is arranged and what the
wasm must expose to draw it. No row yet.

## The screen

- View fills the window. Sidebar on the right, 176 px at 100%, scaling by integer
  steps (176, 264, 352) so the cameo art never resamples.
- Top strip over the view, 20 px: side name, five wares with their counts
  (`wood, stone, grain, flour, bread`), tick and match state. It belongs to the
  player's side only; the enemy's purse is scouting information, not a readout.
- Bottom strip, 24 px: the last order or refusal in plain words
  ("not enough wood", "that ground is taken"), plus the hotkeys hint.
- Chrome: parchment panels from the CC0 UI set, 1 px `#1B1F24` shadow line,
  never a bevel that pretends to be 3D.
- Fonts: OFL only, Cinzel titles at 14 px, IM Fell body at 12 px. The page may
  fall back to a system serif; never a paid or bundled non-OFL font.

## The sidebar, top to bottom

1. Minimap, 160 x 120: the whole 16 x 12 map, houses as 2 px blocks in team
   colour, units as 1 px dots, viewport rectangle. Click moves the camera; drag
   does not rotate the view (the view never rotates).
2. Build grid, 2 columns of 64 x 48 cameos with a 4 px gutter: five buildings in
   hotkey order 1-5, then unit buttons. Each cell carries its cost in the tooltip
   and its hotkey in the corner at 10 px.
3. Selection panel, 160 x 72: the selected thing's cameo, name, hit points as a
   bar, and for a house under construction the ticks left and the ware it is
   waiting for.
4. Command row, three 48 x 48 buttons: attack-move, stop, hold. Icons are rendered
   from the same model set, never glyph guesses.

## Pointer contract

- Left click selects the topmost unit or building under the cursor (depth key is
  the one the renderer already sorts by). Empty ground clears the selection.
- With a build cell armed: hover paints the footprint green or red from the sim's
  own placement check, left click places, right click or Esc cancels. The page
  must ask the sim, never re-implement the rule.
- With units selected: left click attacks the target under the cursor, right
  click attack-moves to the tile. Shift adds to the selection.
- Cursor changes to a crossed sword over a hostile, a hammer over buildable
  ground, an arrow otherwise. No cursor art is required for M3; a CSS cursor
  keyword is enough until the UI set lands.

## What the wasm must expose

The page draws and clicks; it never decides. Needed additions to `kam2-web`:
`build(side, kind, tile) -> refusal`, `select_at(x, z) -> id`,
`order_selected(kind, x, z) -> refusal`, `placement(kind, tile) -> ok|why`,
`stock_of(side)` and `house_state(id)`. Each returns a refusal as a stable code
plus an English line, so the bottom strip can print the sim's own words.

## Acceptance for the UI milestone

- One screenshot of the window and one of the page at 1280 x 720 with the
  sidebar, a selected house, and a refused build showing its reason.
- A pointer-only run: place a farm, select a swordsman, attack-move it, without
  touching the keyboard.
- The minimap rectangle matches the camera after a pan, by test, not by eye.

## Order

1. Resource strip and build grid with cameos cropped from the finished atlas
   cells (64 x 48 downscale of the 96 x 72 cameo, or of the cell itself until the
   cameo pass exists).
2. Placement via `placement` and `build`, with the refusal line.
3. Selection panel and the three command buttons.
4. Minimap and the pointer test.
