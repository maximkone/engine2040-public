//! Registry pin: the frozen replay registry and the live router.
//!
//! Door v2 (2026-10-04) split the door into thirteen core tools plus pack
//! verbs (`mcp::tool_names`), while `tools::TOOLS` stays the frozen
//! twenty-six-name replay registry tapes pin (`REGISTRY_VERSION` 1). The
//! contract is: every frozen name is reachable, every core tool is
//! classified, and the reachable count is pinned per feature set, so a
//! silent add or drop fails here first. Tapes pin the frozen digest, never
//! the router size.
#![cfg(feature = "mcp")]
use engine_endpoint::{
    mcp::{tool_names, CORE_TOOL_NAMES},
    tools::TOOLS,
};

/// Reachable tools under `--features mcp`: 13 core + 65 pack verbs (the web pack adds
/// `build.web`, `serve.web`, `browser.capture` and `browser.input`; the physics
/// pack adds `physics.read`, `physics.raycast` and `physics.shapecast`; the play
/// pack adds `play.step_until`, `input.inject`, `state.digest`, `seat.open`,
/// `seat.step` and `seat.close`). The `rules` feature adds 3 (`rules.set`,
/// `rules.test`, `rules.explain`), the `intake` feature adds 1 (`asset.intake`),
/// and the `verify` feature adds 3 (`bug.replay`, `trace.summary`,
/// `verify.proof`; it also moves `replay.verify`/`replay.bisect` into the
/// verify pack, so the count itself does not move for those two). One arm per
/// combination keeps the pin exact under every feature set.
/// Measured 2026-10-17 with the door probe (`api.load` per pack): 42 pack
/// verbs. The `b2f8a5cf` replay/play-ext landing added four verbs and bumped
/// the pin by two, so this corrects the inherited two-verb drift; the physics
/// pack's own +1 was already counted. The two cast verbs
/// (`physics.raycast`, `physics.shapecast`) then moved every arm up by two.
/// `scene.next` (SPX4, the scene pack's seventh verb) moved every arm up by one.
/// `wound.table.get` (BX39, the break pack's fourth verb) moved every arm up by one.
/// `scene.pick` (EV21, the scene pack's eighth verb) moved every arm up by one.
/// `play.sweep` (GS33, the play pack's fourth verb) moved every arm up by one.
/// `view.observe` and `view.laws` (SP62, the view pack's two verbs, landed in
/// 1c70578) moved every arm up by two.
/// The settings pack adds `settings.get`, `settings.set` and `settings.undo`
/// (on-demand, not advertised by default), so every arm moves up by three.
/// The live seat's `seat.open`, `seat.step` and `seat.close` (AIN2) moved every
/// arm up by three. `view.compare` (the two-PNG machine score) moved every arm
/// up by one. `world.query` (GS46, the scene pack's ninth verb) moved every arm
/// up by one. `ai.decisions` (GS48, the play pack's eighth verb) moved every arm
/// up by one. The session pack adds `scene.diff` and `session.activity` (D17,
/// on-demand), so every arm moves up by two. The codex pack adds
/// `codex.search` and `codex.cite` (on-demand), so every arm moves up by two
/// more.
const EXPECTED_ROUTER: usize = 80
    + if cfg!(feature = "rules") { 3 } else { 0 }
    + if cfg!(feature = "intake") { 1 } else { 0 }
    + if cfg!(feature = "verify") { 3 } else { 0 };

#[test]
fn frozen_registry_pins_26_and_router_is_pinned() {
    assert_eq!(TOOLS.len(), 26, "frozen registry size");
    let names = tool_names();
    assert_eq!(names.len(), EXPECTED_ROUTER, "reachable router size");
    // No phantoms: every frozen name is reachable by exact name.
    for t in TOOLS {
        assert!(names.contains(&t.name), "router serves {}", t.name);
    }
    // Every core tool is classified in the frozen registry.
    for core in CORE_TOOL_NAMES {
        assert!(
            TOOLS.iter().any(|t| t.name == core),
            "core tool `{core}` is classified"
        );
    }
}
