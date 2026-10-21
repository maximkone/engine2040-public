//! collide: the one-way collider a sheet meets and the swept test that stops a fast particle on
//! the shell instead of letting it pass through.
//!
//! A collider carries the layer it is in and the surface penetration the cloth may keep; the
//! table in [`crate::layers`] decides which pairs may meet at all (cloth meets fighters, props,
//! the world and other cloth, and never debris). The swept test reuses the hashed tick's own Q16
//! detector ([`crate::query::swept_cast`]), so a 30 m/s gust cannot tunnel a cape through a body.

use super::convert::{from_fixed, to_fixed, to_fixed_point};
use super::{check_point, dot, sub, Cloth, ClothError, MAX_SIZE, MAX_SPEED, MIN_LEN2, TOUCH_EPS};
use crate::layers::{self, Layers};
use crate::query::{swept_cast, CastCapsule};
use engine_core::fixed::Fixed;
use engine_core::vec3::dist2;

/// One-way capsule (a sphere when both ends meet) that pushes cloth aside; cloth pushes nothing
/// back. Intended consumer: the fighter body (torso and legs) for capes, rods and poles for
/// banners.
///
/// The collider carries the layer it is in and the surface penetration the cloth may keep.
/// The layer decides whether the cloth may meet it at all (the table in [`crate::layers`]): a
/// cape meets fighters, props, the world and other cloth, and never meets debris.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClothCollider {
    a: [f32; 3],
    b: [f32; 3],
    pub(in crate::cloth) radius: f32,
    /// The layer this collider is in and the groups it accepts.
    layers: Layers,
    /// How deep inside the shell the cloth may sit, meters.
    max_distance: f32,
}

impl ClothCollider {
    /// A ball at `center`.
    pub fn sphere(center: [f32; 3], radius: f32) -> Result<Self, ClothError> {
        Self::capsule(center, center, radius)
    }

    /// A segment `a` to `b` swollen by `radius` meters.
    pub fn capsule(a: [f32; 3], b: [f32; 3], radius: f32) -> Result<Self, ClothError> {
        check_point(a, "collider")?;
        check_point(b, "collider")?;
        if !(0.001..=MAX_SIZE).contains(&radius) {
            return Err(ClothError::BadParam("collider radius"));
        }
        Ok(Self {
            a,
            b,
            radius,
            layers: fighter_layers(),
            max_distance: 0.0,
        })
    }

    /// Put the collider on another layer: the layer it is in and the accept mask of that layer.
    ///
    /// # Errors
    /// [`ClothError::BadParam`] when the layer is not one known layer bit.
    pub fn with_layer(mut self, layer: u32) -> Result<Self, ClothError> {
        self.layers = Layers::new(layer).map_err(|_| ClothError::BadParam("collider layer"))?;
        Ok(self)
    }

    /// Let the cloth keep up to `m` meters inside the shell: the surface penetration it may
    /// have. Zero (the default) means the cloth sits on the shell.
    ///
    /// # Errors
    /// [`ClothError::BadParam`] when the distance is not a finite number in `0..=MAX_SIZE`.
    pub fn with_max_distance(mut self, m: f32) -> Result<Self, ClothError> {
        if !m.is_finite() || !(0.0..=MAX_SIZE).contains(&m) {
            return Err(ClothError::BadParam("collider max distance"));
        }
        self.max_distance = m;
        Ok(self)
    }

    /// The layer this collider is in.
    pub fn layer(&self) -> u32 {
        self.layers.group
    }

    /// The groups this collider accepts.
    pub fn mask(&self) -> u32 {
        self.layers.mask
    }

    /// The surface penetration the cloth may keep, meters.
    pub fn max_distance(&self) -> f32 {
        self.max_distance
    }

    /// The radius the cloth must stay outside of.
    fn shell(&self) -> f32 {
        (self.radius - self.max_distance).max(0.0)
    }

    /// The point of the axis segment nearest `p`.
    pub(in crate::cloth) fn nearest_on_axis(&self, p: [f32; 3]) -> [f32; 3] {
        let ab = sub(self.b, self.a);
        let len2 = dot(ab, ab);
        if len2 < MIN_LEN2 {
            return self.a;
        }
        let t = (dot(sub(p, self.a), ab) / len2).clamp(0.0, 1.0);
        [
            self.a[0] + ab[0] * t,
            self.a[1] + ab[1] * t,
            self.a[2] + ab[2] * t,
        ]
    }

    /// True when `p` is inside the shell.
    fn contains(&self, p: [f32; 3]) -> bool {
        dist2(p, self.nearest_on_axis(p)) < self.shell() * self.shell()
    }

    /// `p` moved straight out to the shell surface; points outside stay put. A point exactly on
    /// the axis leaves along +Z.
    pub(in crate::cloth) fn push_out(&self, p: [f32; 3]) -> [f32; 3] {
        let shell = self.shell();
        let axis = self.nearest_on_axis(p);
        let away = sub(p, axis);
        let d2 = dot(away, away);
        if d2 >= shell * shell {
            return p;
        }
        if d2 < MIN_LEN2 {
            return [axis[0], axis[1], axis[2] + shell];
        }
        let k = shell / d2.sqrt();
        [
            axis[0] + away[0] * k,
            axis[1] + away[1] * k,
            axis[2] + away[2] * k,
        ]
    }
}

impl Cloth {
    /// How many particles the swept collider test stopped on the last tick.
    pub fn swept_stops(&self) -> u32 {
        self.swept_stops
    }

    /// True when a free particle sits inside a collider it may meet.
    pub(in crate::cloth) fn touched(&self, colliders: &[ClothCollider]) -> bool {
        colliders.iter().any(|c| {
            if !cloth_meets(c) {
                return false;
            }
            self.pos
                .iter()
                .zip(&self.pinned)
                .any(|(p, pinned)| !pinned && c.contains(*p))
        })
    }

    /// The arena bound: a particle never travels more than [`MAX_SPEED`] meters in a second.
    /// The swept collider test is what stops the cloth at a body; this only bounds the motion
    /// when no collider is in the way.
    pub(in crate::cloth) fn cap_speed(&mut self, h: f32) {
        let limit = MAX_SPEED * h;
        let limit2 = limit * limit;
        for i in 0..self.pos.len() {
            let moved = sub(self.pos[i], self.prev[i]);
            let moved2 = dot(moved, moved);
            if moved2 <= limit2 {
                continue;
            }
            let k = limit / moved2.sqrt();
            self.pos[i] = [
                self.prev[i][0] + moved[0] * k,
                self.prev[i][1] + moved[1] * k,
                self.prev[i][2] + moved[2] * k,
            ];
        }
    }

    /// The swept particle-vs-collider test: a particle whose path crosses a collider stops at
    /// the time of impact instead of passing through it. The test uses the hashed tick's own Q16
    /// detector ([`crate::query::swept_cast`]), so the answer is the same on every machine and a
    /// fast gust cannot tunnel a cape through a body. Returns how many particles were stopped.
    pub(in crate::cloth) fn sweep_colliders(&mut self, colliders: &[ClothCollider]) -> u32 {
        let mut stopped = 0;
        for i in 0..self.pos.len() {
            if self.pinned[i] {
                continue;
            }
            let from = self.prev[i];
            let to = self.pos[i];
            let moved = dist2(from, to).sqrt();
            if moved <= 0.0 {
                continue;
            }
            for collider in colliders {
                if !cloth_meets(collider) {
                    continue;
                }
                let shell = collider.shell();
                // A particle too far away cannot reach the shell this move.
                let reach_from = dist2(from, collider.nearest_on_axis(from)).sqrt();
                if reach_from > shell + moved {
                    continue;
                }
                // A particle that starts on the shell and moves in may not enter: the push-out
                // would send it out on the far side. It holds its place instead.
                let reach_to = dist2(to, collider.nearest_on_axis(to)).sqrt();
                if reach_from <= shell + TOUCH_EPS && reach_to < shell - TOUCH_EPS {
                    self.pos[i] = from;
                    stopped += 1;
                    break;
                }
                let target = [CastCapsule {
                    id: 0,
                    a: to_fixed_point(collider.a),
                    b: to_fixed_point(collider.b),
                    radius: to_fixed(collider.radius),
                }];
                if swept_cast(
                    to_fixed_point(from),
                    to_fixed_point(to),
                    Fixed::ZERO,
                    &target,
                    &mut self.cast_hits,
                )
                .is_err()
                {
                    continue;
                }
                let Some(hit) = self.cast_hits.first().copied() else {
                    continue;
                };
                // A hit at time zero means the particle already touched the shell; the push-out
                // below settles it.
                if hit.toi.0 <= 0 {
                    continue;
                }
                let t = from_fixed(hit.toi);
                let normal = [
                    from_fixed(hit.normal[0]),
                    from_fixed(hit.normal[1]),
                    from_fixed(hit.normal[2]),
                ];
                self.pos[i] = [
                    from[0] + (to[0] - from[0]) * t + normal[0] * TOUCH_EPS,
                    from[1] + (to[1] - from[1]) * t + normal[1] * TOUCH_EPS,
                    from[2] + (to[2] - from[2]) * t + normal[2] * TOUCH_EPS,
                ];
                stopped += 1;
                break;
            }
        }
        stopped
    }
}

/// The layers a cloth sheet is in: the cloth group and the groups it accepts.
fn cloth_layers() -> Layers {
    Layers {
        group: layers::CLOTH,
        mask: layers::default_mask(layers::CLOTH),
    }
}

/// The layers a collider starts on: the fighter group and its accept mask.
fn fighter_layers() -> Layers {
    Layers {
        group: layers::BODY,
        mask: layers::default_mask(layers::BODY),
    }
}

/// True when the cloth may meet this collider: the Bullet pair rule between the cloth group
/// and the collider's own pair. Cloth meets fighters, props, the world and other cloth; it never
/// meets debris or a sensor.
pub(in crate::cloth) fn cloth_meets(collider: &ClothCollider) -> bool {
    layers::collides(&cloth_layers(), &collider.layers)
}
