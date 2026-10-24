# DELIVERY O-042: game-ui for engine2040 UI-1 (main, pause, settings, HUD, inventory)
1. Load: `atlas.png` + `atlas.json` (27 9-slice pieces, nearest, integer x2/x3/x4), `font-5x7.png` + `.json` + `.ttf` (95 ASCII glyphs), `tokens.json`, `layout.json` (5 screens in 640x360 reference, the data the AI edits).
2. Loader (FORMATS.md): engine2040 `crates/ui/src/lib.rs` (`Hud`), `menus.rs`, `lobby_ui.rs`, `text.rs` (`Atlas` + `emit_quads`); unknown: which renderer file consumes `emit_quads`, and the forge HUD owner (repos absent from this PC).
3. Look: `preview-menu/pause/settings/hud/inventory.png` (1280x720, composed from the atlas), `preview-atlas.png`, `icons.png` + `icons@2x.png`, `page.html` + `out.png` kit sheet.
4. Buttons: normal/hover/pressed/disabled + primary trio, all in the atlas; settings bars use mana/gold fills; touch targets 22px reference (=44px at x2).
5. Proof: `node tools/game-ui.mjs --check`, `node tools/orders-check.mjs --built O-042`, `node tools/ui-pack.mjs --check`; licence CC0, no third-party bytes.
