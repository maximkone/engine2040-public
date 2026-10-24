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
/// `seat.step` and `seat.close`). The `rules` feature adds 5 (`rules.set`,
/// `rules.test`, `rules.explain`, `rules.get` and `rules.undo`), the `intake`
/// feature adds 1 (`asset.intake`),
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
/// more. `tune.sweep` (HX35, the play pack's ninth verb) moved every arm up by
/// one. DOOR2 (2026-10-22): `scene.save` and `scene.load` join the default
/// scene pack, so every arm moves up by two. After DOOR2 the router gained
/// `game.play` (DOOR1 step 2), `net.shape` (`325a54fa6`), `perf.doctor`
/// (`6330e67ba`) and `door.status` (`e8dcf966e`); the live count was 86, three
/// over the DOOR2 arm, so every arm moved up by three. The fx pack adds
/// `fx.ragdoll`, `fx.limb`, `fx.break`, `fx.blast` and `fx.undo` on demand
/// (AD30), so every arm moves up by five more. BG4 (2026-10-23) adds the four
/// clip verbs (`anim.clip.trim`, `anim.clip.retime`, `anim.clip.mirror`,
/// `anim.clip.undo`), so every arm moves up by four more. After BG4 five packs
/// joined `PACK_NAMES` without moving this pin: `world`, `level_paint` (DOOR8's
/// one verb `level.paint`, 2026-10-24), `stage`, `ui` and `audio`, and further
/// verbs landed inside existing packs. Measured 2026-10-24 under
/// `--features mcp`: the live count is 126, thirty-one over the BG4 arm, so
/// every arm moves up by thirty-one. The fx pack's four preset verbs
/// (`fx.preset.list|preview|set|clear`, 2026-10-24) add four more, so the live
/// router under `--features mcp` is 130. Measured again at HEAD the same day:
/// 134, three more verbs landed inside existing packs while DOOR8 and DOOR9
/// were in review. DOOR9 (2026-10-24) adds `anim.retarget`, the bone-map verb
/// over the 17 roles, and `anim.retarget.preview`, the same map drawn as one
/// PNG frame. Two seats landed while DOOR9 was in review: the project pack
/// (`project.new`, `project.build`, `project.run`) and the memory pack
/// (`memory.write`, `memory.recall`, `memory.share`), six verbs more. Measured
/// 2026-10-24 under `--features mcp` after all of that: 143.
const EXPECTED_ROUTER: usize = 143
    + if cfg!(feature = "rules") { 5 } else { 0 }
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
