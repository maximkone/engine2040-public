//! AD15 cook gate: a planted foot may not skate between ticks.
//!
//! The target clip's cooked contact table decides planted, so this gate and
//! the runtime read the same samples. A step is measured between consecutive
//! planted ticks; a foot in the air may travel freely.

use super::*;
use engine_core::checked::f32_from_u64;

/// Worst planted-foot step a cook accepts, in integer millimeters per tick.
/// A planted foot that moves farther is skating: at 60 Hz this budget is the
/// 1.2 m/s of hidden slide a player would see as a frozen foot gliding.
pub const MAX_PLANTED_SLIDE_MM: u32 = 20;

/// Imported rig name prefixes stripped on both sides of a bone lookup, so a
/// `<prefix>Hips` joint maps to a plain `Hips` pair name and back.
pub const RIG_PREFIXES: [&str; 2] = ["mixamorig:", "mixamorig_"];

/// One planted foot step over [`MAX_PLANTED_SLIDE_MM`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FootSlide {
    /// Foot index: `0` left, `1` right.
    pub foot: u8,
    /// Absolute tick of the later sample.
    pub tick: u32,
    /// Step distance in millimeters.
    pub mm: u32,
}

/// Worst planted-foot step in a cooked clip, or `None` when every planted step
/// stays inside [`MAX_PLANTED_SLIDE_MM`]. Two samples count as a step only when
/// both ticks are planted, so a foot in the air may travel freely; the cooked
/// contact table decides planted, so this gate and the runtime agree. The step
/// is [`slide_mm`] on the ground plane: a planted foot skates sideways, while
/// the vertical move belongs to the plant itself (the arena's sunk/pop meter
/// owns that axis). Landing noise across the ground threshold is not skate.
pub fn clip_foot_slide(
    clip: &ImportedClip,
    skeleton: &Skeleton,
    contacts: &ContactTable,
    left_foot: usize,
    right_foot: usize,
) -> Result<Option<FootSlide>, RetargetError> {
    for joint in [left_foot, right_foot] {
        if joint >= skeleton.len() {
            return Err(RetargetError::UnknownJoint { joint });
        }
    }
    let ticks = usize::try_from(cook_tick_count(clip.duration).map_err(map_cook_error)?)
        .unwrap_or(usize::MAX);
    if contacts.contacts.len() != 2 * ticks {
        return Err(RetargetError::BadContact {
            index: contacts.contacts.len(),
        });
    }
    let feet: [(u8, usize); 2] = [(0, left_foot), (1, right_foot)];
    let mut previous: [Option<[f32; 3]>; 2] = [None; 2];
    let mut worst: Option<FootSlide> = None;
    for (index, pair) in contacts.contacts.chunks_exact(2).enumerate() {
        let time = f32_from_u64(u64::try_from(index).unwrap_or(u64::MAX)) * TICK_DT;
        for (foot, joint) in feet {
            let slot = usize::from(foot);
            let track = clip
                .positions
                .tracks
                .get(joint)
                .ok_or(RetargetError::BadClip { joint })?;
            let position = sample_track3(track, joint, time).map_err(map_cook_error)?;
            let contact = &pair[slot];
            if !contact.down {
                previous[slot] = None;
                continue;
            }
            if let Some(before) = previous[slot] {
                let mm = slide_mm(ground(before), ground(position));
                if mm > MAX_PLANTED_SLIDE_MM && worst.is_none_or(|worst| mm > worst.mm) {
                    worst = Some(FootSlide {
                        foot,
                        tick: contact.tick,
                        mm,
                    });
                }
            }
            previous[slot] = Some(position);
        }
    }
    Ok(worst)
}

/// The same point on the ground plane: a planted foot's skate is horizontal.
fn ground(point: [f32; 3]) -> [f32; 3] {
    [point[0], 0.0, point[2]]
}
