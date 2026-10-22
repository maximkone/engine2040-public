//! DOOR3 scene frame: `play.step` with `scene_rev` plays that pinned scene.
//!
//! The scene's solid blocks (the same [`blocks`] `play.run` builds its world
//! from) feed two things. The step's digest is the spine checkpoint over the
//! scene's [`MatchWorld`], the authority `play.run` hashes with for the same
//! scene. The frame paints each block as a top-down box over the backdrop.
//!
//! Limit: fighter discs are still placed by the digest stream, and the live
//! bout behind `play.step` takes no world, so fighters do not collide with the
//! props yet. That needs a world in the arena's live bout (out of this lane).
//! A GPU hook paints its own pixels, so a scene step always renders in software.

use engine_editor::commands::Document;

use crate::mcp::McpToolError;
use crate::play::world_load::{blocks, Block};
use engine_arena::world::MatchWorld;

/// Frame width `play.step` answers when `width` is absent: readable at a glance.
pub const PLAY_STEP_FRAME_WIDTH: u32 = 320;
/// Frame height `play.step` answers when `height` is absent.
pub const PLAY_STEP_FRAME_HEIGHT: u32 = 180;

/// Warm wood fill for a solid block, and its edge.
const PROP_FILL: [u8; 3] = [150, 112, 74];
const PROP_EDGE: [u8; 3] = [78, 56, 34];

/// One pinned scene a step plays: its match world and its solid blocks.
pub(crate) struct StepScene {
    pub(crate) world: MatchWorld,
    pub(crate) blocks: Vec<Block>,
}

/// Pin the scene at `revision` (refusals as `play.run` names them) and read
/// its match world and blocks from the same document.
pub(crate) fn pin_scene(doc: &Document, revision: u64) -> Result<StepScene, McpToolError> {
    let pinned = crate::scene_tools::pinned_world(doc, Some(revision))?;
    Ok(StepScene {
        world: pinned.world,
        blocks: blocks(doc),
    })
}

/// Paint each block as a filled top-down box with an edge one pixel wide. The
/// arena spans `-half..half` on both axes; world x runs left to right and
/// world z top to bottom. Each pixel centre is tested against the block's
/// axis-aligned reach, so a rotated block paints its bound and a block outside
/// the arena paints nothing.
pub(crate) fn paint_props(rgb: &mut [u8], w: usize, h: usize, half: f32, props: &[Block]) {
    let (Ok(cols), Ok(rows)) = (u16::try_from(w), u16::try_from(h)) else {
        return;
    };
    if cols == 0 || rows == 0 || !(half.is_finite() && half > 0.0) {
        return;
    }
    let pixel_w = 2.0 * half / f32::from(cols);
    let pixel_h = 2.0 * half / f32::from(rows);
    for block in props {
        let [cx, cz] = block.center;
        let [sx, sz] = block.size;
        let [ax, az] = block.axis;
        // The box axes are (ax, az) and (-az, ax): their world reach folds the
        // two half sizes through |cos| and |sin|.
        let reach_x = ax.abs() * sx * 0.5 + az.abs() * sz * 0.5;
        let reach_z = az.abs() * sx * 0.5 + ax.abs() * sz * 0.5;
        for row in 0..rows {
            let dz = ((f32::from(row) + 0.5) * pixel_h - half - cz).abs();
            if dz > reach_z {
                continue;
            }
            for col in 0..cols {
                let dx = ((f32::from(col) + 0.5) * pixel_w - half - cx).abs();
                if dx > reach_x {
                    continue;
                }
                let edge = reach_x - dx < pixel_w || reach_z - dz < pixel_h;
                let color = if edge { PROP_EDGE } else { PROP_FILL };
                let at = (usize::from(row) * w + usize::from(col)) * 3;
                rgb[at..at + 3].copy_from_slice(&color);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_paints_its_footprint_and_leaves_the_rest() {
        let (w, h) = (64usize, 36usize);
        let mut rgb = vec![0u8; w * h * 3];
        // A 10 m square at the middle of a 40 m arena.
        let block = Block {
            center: [0.0, 0.0],
            size: [10.0, 10.0],
            axis: [1.0, 0.0],
        };
        paint_props(&mut rgb, w, h, 20.0, &[block]);
        let pixel = |x: usize, y: usize| {
            let at = (y * w + x) * 3;
            [rgb[at], rgb[at + 1], rgb[at + 2]]
        };
        assert_eq!(pixel(32, 18), PROP_FILL, "centre is the fill");
        assert_eq!(pixel(24, 18), PROP_EDGE, "left edge is the outline");
        assert_eq!(pixel(0, 0), [0, 0, 0], "corners stay the backdrop");
    }

    #[test]
    fn a_block_outside_the_arena_and_a_bad_half_paint_nothing() {
        let (w, h) = (16usize, 16usize);
        let far = Block {
            center: [500.0, 500.0],
            size: [1.0, 1.0],
            axis: [1.0, 0.0],
        };
        let mut rgb = vec![0u8; w * h * 3];
        paint_props(&mut rgb, w, h, 20.0, &[far]);
        assert!(rgb.iter().all(|&byte| byte == 0), "outside the arena");

        paint_props(&mut rgb, w, h, f32::NAN, &[far]);
        assert!(rgb.iter().all(|&byte| byte == 0), "non-finite half");
    }

    use crate::mcp::ServerState;
    use crate::play::play_session::{play_step_with, PlayStepArgs};
    use engine_editor::capability::Capability;
    use engine_editor::commands::{self, Batch, Journal, Op};
    use engine_scene::ids::ObjectId;
    use engine_scene::transform::Transform;

    /// A door whose document holds one 2 m cube at the arena centre (revision 1).
    fn door_with_prop() -> ServerState {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        commands::apply(
            &mut doc,
            &mut journal,
            &Batch {
                ops: vec![Op::Insert {
                    id: ObjectId::new(),
                    parent: ObjectId::NIL,
                    local: Transform::new([0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0], [2.0, 2.0, 2.0]),
                }],
                base_revision: 0,
                idempotency_key: 31,
                provenance: None,
            },
        )
        .expect("prop applies");
        ServerState::with_document(doc, journal, 9, Capability::unrestricted(9))
    }

    /// One-tick step, optionally pinned to a scene revision.
    fn step(upto: u32, scene_rev: Option<u64>) -> PlayStepArgs {
        PlayStepArgs {
            upto_tick: Some(upto),
            scene_rev,
            ..PlayStepArgs::default()
        }
    }

    #[test]
    fn scene_step_frame_shows_the_props_at_the_default_size() {
        let mut door = door_with_prop();
        let plain = door.play_step(&step(1, None)).expect("default duel serves");
        let scene = door
            .play_step(&step(1, Some(1)))
            .expect("pinned scene plays");
        assert_eq!((scene.manifest.width, scene.manifest.height), (320, 180));
        assert_eq!((plain.manifest.width, plain.manifest.height), (320, 180));
        assert_ne!(
            scene.png_bytes, plain.png_bytes,
            "the prop changes the frame"
        );
        let again = door.play_step(&step(1, Some(1))).expect("same pin again");
        assert_eq!(again.png_bytes, scene.png_bytes, "same scene, same frame");
    }

    #[test]
    fn scene_step_refuses_a_stale_or_empty_pin_by_name() {
        let mut door = door_with_prop();
        let stale = door
            .play_step(&step(1, Some(7)))
            .expect_err("revision 7 is not the live scene");
        assert_eq!(stale.code(), "stale_scene_rev");
        let mut empty = ServerState::new(9, Capability::unrestricted(9));
        let none = empty
            .play_step(&step(1, Some(0)))
            .expect_err("an empty arena never pins");
        assert_eq!(none.code(), "empty_arena");
    }

    #[test]
    fn stateless_step_refuses_a_scene_rev_and_names_the_door() {
        let refused = play_step_with(&step(1, Some(1)), None).expect_err("no scene here");
        assert_eq!(refused.code(), "scene_needs_door");
    }
}
