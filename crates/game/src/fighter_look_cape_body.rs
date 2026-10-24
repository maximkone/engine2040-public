//! The body capsules a cape's draw keeps its cloth outside of, and the push-out that does
//! it. The cloth layer keeps its own collider geometry private, so the look builds these
//! from the same joints it pins the cape to.
//!
//! Presentation only: nothing here is hashed and the sim never reads it.

use super::fighter_look_soft_cape::{SOFT_CAPE_COLS, SOFT_CAPE_ROWS};
use super::{dot3, sub3, FighterLook, CAPE_DRAW_SKIN_M, CAPE_TIER};

/// One body capsule the cape's draw keeps the cloth outside of: segment `a` to `b`
/// swollen by `radius`, in world meters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct CapeShell {
    pub(super) a: [f32; 3],
    pub(super) b: [f32; 3],
    pub(super) radius: f32,
}

/// Signed distance from `p` to the shell's surface, meters: negative inside.
pub(super) fn shell_distance(shell: CapeShell, p: [f32; 3]) -> f32 {
    let seg = sub3(shell.b, shell.a);
    let len2 = dot3(seg, seg);
    let t = if len2 > 1e-9 {
        (dot3(sub3(p, shell.a), seg) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let closest = [
        shell.a[0] + seg[0] * t,
        shell.a[1] + seg[1] * t,
        shell.a[2] + seg[2] * t,
    ];
    let d = sub3(p, closest);
    dot3(d, d).sqrt() - shell.radius
}

/// Push every vertex out of `shells` to its surface plus `skin`, meters: the cape's draw
/// keeps the cloth off the body when a lean, a twist or a wind would otherwise carry it
/// inside. Two passes, so a vertex cleared of one shell is still cleared of the next.
/// Radial push-out only (the direction from the capsule axis); pure and allocation free.
/// Returns how many vertices moved.
pub(super) fn push_off_shells(vertices: &mut [[f32; 3]], shells: &[CapeShell], skin: f32) -> usize {
    let mut moved = 0usize;
    for _ in 0..2 {
        for vertex in vertices.iter_mut() {
            for shell in shells {
                if shell_distance(*shell, *vertex) >= skin {
                    continue;
                }
                let seg = sub3(shell.b, shell.a);
                let len2 = dot3(seg, seg);
                let t = if len2 > 1e-9 {
                    (dot3(sub3(*vertex, shell.a), seg) / len2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let closest = [
                    shell.a[0] + seg[0] * t,
                    shell.a[1] + seg[1] * t,
                    shell.a[2] + seg[2] * t,
                ];
                let out = sub3(*vertex, closest);
                let len = dot3(out, out).sqrt();
                let dir = if len > 1e-6 {
                    [out[0] / len, out[1] / len, out[2] / len]
                } else {
                    [0.0, 1.0, 0.0]
                };
                let target = shell.radius + skin;
                *vertex = [
                    closest[0] + dir[0] * target,
                    closest[1] + dir[1] * target,
                    closest[2] + dir[2] * target,
                ];
                moved += 1;
            }
        }
    }
    moved
}

/// One slot's cape for a frame: positions, columns, rows and the outward direction.
pub(super) type CapeDraw = (Vec<[f32; 3]>, usize, usize, [f32; 3]);

impl FighterLook {
    /// The cape vertices `publish_capes` draws for `slot` this frame: the physfx soft
    /// cloth's particles with the switch on, the RZ7 Verlet sheet with it off. `None`
    /// leaves the slot without a cape draw.
    pub(super) fn cape_draw(&self, slot: usize) -> Option<CapeDraw> {
        if let Some(soft) = self.capes.soft.as_ref() {
            let mut positions = soft.positions(slot)?;
            // The cloth hangs in its own frame, so a lean or a twist can carry it inside
            // the body: push the drawn sheet off the body's own capsules.
            if let Some(pose) = self.capes.poses[slot] {
                let shells = &self.capes.shells[pose.shells.0..pose.shells.1];
                push_off_shells(&mut positions, shells, CAPE_DRAW_SKIN_M);
            }
            return Some((
                positions,
                SOFT_CAPE_COLS,
                SOFT_CAPE_ROWS,
                soft.outward(slot),
            ));
        }
        let grid = CAPE_TIER.knobs().cape;
        let positions = self.cape_positions(slot)?.to_vec();
        Some((positions, grid.cols, grid.rows, self.capes.outward[slot]))
    }
}
