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

/// Reachable tools under `--features mcp`: 13 core + 48 pack verbs (the web pack adds
/// `build.web`, `serve.web`, `browser.capture` and `browser.input`; the physics
/// pack adds `physics.read`, `physics.raycast` and `physics.shapecast`; the play
/// pack adds `play.step_until`, `input.inject` and `state.digest`). The `rules` feature adds 3 (`rules.set`,
/// `rules.test`, `rules.explain`), the `intake` feature adds 1 (`asset.intake`);
/// one arm per combination keeps the pin exact under every feature set.
/// Measured 2026-10-17 with the door probe (`api.load` per pack): 42 pack
/// verbs. The `b2f8a5cf` replay/play-ext landing added four verbs and bumped
/// the pin by two, so this corrects the inherited two-verb drift; the physics
/// pack's own +1 was already counted. The two cast verbs
/// (`physics.raycast`, `physics.shapecast`) then moved every arm up by two.
#[cfg(not(any(feature = "rules", feature = "intake")))]
const EXPECTED_ROUTER: usize = 61;
#[cfg(all(feature = "rules", not(feature = "intake")))]
const EXPECTED_ROUTER: usize = 64;
#[cfg(all(feature = "intake", not(feature = "rules")))]
const EXPECTED_ROUTER: usize = 62;
#[cfg(all(feature = "rules", feature = "intake"))]
const EXPECTED_ROUTER: usize = 65;

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
