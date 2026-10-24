//! particle_forces: up to [`MAX_FORCES`] force fields that push embers, dust
//! and blood mist around the arena instead of letting them fall in a straight
//! line (SW52).
//!
//! Owner: render. Units: meters (positions, extents), metres per second for the
//! per-tick velocity change. View only, never hashed. Determinism: a plain
//! array read in slot order, no clock, no allocation, no map, so the same
//! forces and the same particles give the same bytes every run.
//!
//! Donor: godotengine/godot `servers/rendering/renderer_rd/shaders/
//! particles.glsl` (`main` attractor loop, `ATTRACTOR_TYPE_SPHERE/BOX/
//! VECTOR_FIELD`, `MAX_ATTRACTORS 32`). Ported shape: the per-particle
//! `amount` falls from 1 at the centre to 0 at the edge, `amount =
//! amount.powf(attenuation)`, `dir = mix(dir, forward, directionality)`, and
//! the velocity gains `mass * amount * dir * strength`.
//!
//! The one difference that matters for a capture: `strength` here is the
//! velocity change of one tick, not an acceleration scaled by a wall clock, so
//! a replay of the same ticks lands on the same bytes.

/// Most forces one pool applies per tick (the donor's `MAX_ATTRACTORS`).
pub const MAX_FORCES: usize = 32;

/// The three field shapes the donor names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForceKind {
    /// A sphere: `extents[0]` is the radius.
    Sphere,
    /// A box: `extents` are the half-extents per axis.
    Box,
    /// A 3D noise lattice baked once: `extents` are the half-extents of the
    /// region the lattice blows through, and the direction comes from the cell.
    VectorField,
}

/// One force field: where it is, how far it reaches, how hard it pushes and
/// which way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParticleForce {
    /// Sphere, box or vector field.
    pub kind: ForceKind,
    /// Centre in meters.
    pub centre: [f32; 3],
    /// Sphere: the radius in `[0]`; box and vector field: half-extents.
    pub extents: [f32; 3],
    /// Velocity change at full amount, in metres per second per tick. A
    /// negative strength pulls in (an attractor); a positive one pushes out.
    pub strength: f32,
    /// `amount.powf(attenuation)`: 1 is the straight ramp, above 1 keeps the
    /// push near the centre, below 1 spreads it to the edge.
    pub attenuation: f32,
    /// `mix(dir, forward, directionality)`: 0 pushes radially, 1 pushes along
    /// [`Self::forward`].
    pub directionality: f32,
    /// The direction `directionality` mixes toward (an updraft, a vortex
    /// tangent). Not normalised on input; the impulse normalises it.
    pub forward: [f32; 3],
}

/// The baked vector-field lattice: eight unit directions, picked by cell.
const LATTICE: [[f32; 3]; 8] = [
    [1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, 0.0, 1.0],
    [-1.0, 0.0, 0.0],
    [0.0, -1.0, 0.0],
    [0.0, 0.0, -1.0],
    [0.707_106_77, 0.707_106_77, 0.0],
    [0.0, 0.707_106_77, 0.707_106_77],
];

/// The forces one pool applies, in slot order, up to [`MAX_FORCES`].
///
/// A fixed array with a length: pushing never allocates, and a full list
/// refuses the next force instead of dropping an earlier one.
#[derive(Clone, Copy, Debug)]
pub struct ParticleForceList {
    items: [ParticleForce; MAX_FORCES],
    len: usize,
}

impl ParticleForceList {
    /// An empty list: every particle falls as it did before this row.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            items: [NO_FORCE; MAX_FORCES],
            len: 0,
        }
    }

    /// Forces in the list.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// True when no force pushes anything.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Every force, in slot order.
    #[must_use]
    pub fn forces(&self) -> &[ParticleForce] {
        &self.items[..self.len]
    }

    /// Drop every force.
    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// Add `force`; `false` when the list already holds [`MAX_FORCES`].
    pub fn push(&mut self, force: ParticleForce) -> bool {
        if self.len >= MAX_FORCES {
            return false;
        }
        self.items[self.len] = force;
        self.len += 1;
        true
    }

    /// The velocity change `mass` earns from every force at `pos`, in slot
    /// order. The caller subtracts it from the velocity (the donor's push out
    /// of a field), so a positive `strength` on a radial field pushes away and
    /// a negative one pulls in.
    #[must_use]
    pub fn impulse(&self, pos: [f32; 3], mass: f32) -> [f32; 3] {
        let mut total = [0.0; 3];
        for force in self.forces() {
            let Some((amount, radial)) = force.amount_and_direction(pos) else {
                continue;
            };
            let weight = mass * amount;
            if weight == 0.0 || !weight.is_finite() {
                continue;
            }
            let dir = mix_direction(radial, force.forward, force.directionality);
            total[0] += weight * dir[0] * force.strength;
            total[1] += weight * dir[1] * force.strength;
            total[2] += weight * dir[2] * force.strength;
        }
        total
    }
}

impl Default for ParticleForceList {
    fn default() -> Self {
        Self::new()
    }
}

/// The slot value of an unused force: nothing a caller can build by accident
/// (strength 0 pushes nothing).
const NO_FORCE: ParticleForce = ParticleForce {
    kind: ForceKind::Sphere,
    centre: [0.0; 3],
    extents: [0.0; 3],
    strength: 0.0,
    attenuation: 1.0,
    directionality: 0.0,
    forward: [0.0, 0.0, 0.0],
};

impl ParticleForce {
    /// How hard this force pushes at `pos` (`0` outside) and the radial
    /// direction it pushes along before [`Self::forward`] is mixed in.
    #[must_use]
    fn amount_and_direction(&self, pos: [f32; 3]) -> Option<(f32, [f32; 3])> {
        let offset = [
            pos[0] - self.centre[0],
            pos[1] - self.centre[1],
            pos[2] - self.centre[2],
        ];
        if !offset.iter().all(|v| v.is_finite()) || !self.strength.is_finite() {
            return None;
        }
        let length = (offset[0] * offset[0] + offset[1] * offset[1] + offset[2] * offset[2]).sqrt();
        let radial = if length > 1e-6 {
            [offset[0] / length, offset[1] / length, offset[2] / length]
        } else {
            [0.0, 0.0, 0.0]
        };
        let amount = match self.kind {
            ForceKind::Sphere => {
                let radius = self.extents[0];
                if !(radius > 0.0) {
                    return None;
                }
                (1.0 - length / radius).max(0.0)
            }
            ForceKind::Box | ForceKind::VectorField => {
                let far = [0, 1, 2]
                    .into_iter()
                    .map(|axis| {
                        let half = self.extents[axis];
                        if half > 0.0 {
                            (offset[axis] / half).abs()
                        } else {
                            f32::INFINITY
                        }
                    })
                    .fold(0.0f32, f32::max);
                if !far.is_finite() {
                    return None;
                }
                (1.0 - far).max(0.0)
            }
        };
        if amount <= 0.0 {
            return None;
        }
        let amount = amount.powf(self.attenuation.max(0.0));
        let direction = if self.kind == ForceKind::VectorField {
            lattice_direction(self.centre, self.extents, pos)
        } else {
            radial
        };
        Some((amount, direction))
    }
}

/// `mix(dir, forward, directionality)`, with `forward` normalised first and the
/// result normalised so `directionality` changes the angle, never the strength.
fn mix_direction(radial: [f32; 3], forward: [f32; 3], directionality: f32) -> [f32; 3] {
    let blend = directionality.clamp(0.0, 1.0);
    let forward_length =
        (forward[0] * forward[0] + forward[1] * forward[1] + forward[2] * forward[2]).sqrt();
    let forward = if forward_length > 1e-6 {
        [
            forward[0] / forward_length,
            forward[1] / forward_length,
            forward[2] / forward_length,
        ]
    } else {
        radial
    };
    let mixed = [
        radial[0] * (1.0 - blend) + forward[0] * blend,
        radial[1] * (1.0 - blend) + forward[1] * blend,
        radial[2] * (1.0 - blend) + forward[2] * blend,
    ];
    let length = (mixed[0] * mixed[0] + mixed[1] * mixed[1] + mixed[2] * mixed[2]).sqrt();
    if length > 1e-6 {
        [mixed[0] / length, mixed[1] / length, mixed[2] / length]
    } else {
        radial
    }
}

/// The baked lattice's direction for the cell `pos` falls in: the cell is the
/// coordinate's own bit pattern, so the lattice is fixed at build time and two
/// runs of the same particle see the same cell.
fn lattice_direction(centre: [f32; 3], _extents: [f32; 3], pos: [f32; 3]) -> [f32; 3] {
    let bits = |axis: usize| (pos[axis] - centre[axis]).to_bits();
    let hash = bits(0).wrapping_mul(73_856_093)
        ^ bits(1).wrapping_mul(19_349_663)
        ^ bits(2).wrapping_mul(83_492_791);
    let slot = usize::try_from(hash % 8).unwrap_or(0);
    LATTICE[slot]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sphere(centre: [f32; 3], radius: f32, strength: f32) -> ParticleForce {
        ParticleForce {
            kind: ForceKind::Sphere,
            centre,
            extents: [radius, radius, radius],
            strength,
            attenuation: 1.0,
            directionality: 0.0,
            forward: [0.0, 0.0, 0.0],
        }
    }

    /// The sphere's amount is the straight ramp from 1 at the centre to 0 at the
    /// radius, and nothing outside it is pushed at all.
    #[test]
    fn a_sphere_force_falls_off_with_distance_and_stops_at_its_radius() {
        let mut list = ParticleForceList::new();
        assert!(list.push(sphere([0.0, 0.0, 0.0], 1.0, 3.0)));
        assert_eq!(list.len(), 1);
        assert_eq!(
            list.impulse([0.5, 0.0, 0.0], 1.0),
            [1.5, 0.0, 0.0],
            "half way out is half of the strength, radially out"
        );
        assert_eq!(
            list.impulse([0.0, 0.5, 0.0], 0.5),
            [0.0, 0.75, 0.0],
            "a dying particle at half mass gets half the push"
        );
        assert_eq!(
            list.impulse([1.0, 0.0, 0.0], 1.0),
            [0.0, 0.0, 0.0],
            "the edge is out"
        );
        assert_eq!(
            list.impulse([4.0, 2.0, 0.0], 1.0),
            [0.0, 0.0, 0.0],
            "outside the radius is untouched"
        );
    }

    /// Directionality turns a radial push into an updraft, and the list holds
    /// exactly [`MAX_FORCES`] forces.
    #[test]
    fn directionality_mixes_toward_forward_and_the_list_stops_at_32() {
        let mut list = ParticleForceList::new();
        list.push(ParticleForce {
            directionality: 1.0,
            forward: [0.0, 2.0, 0.0],
            ..sphere([0.0; 3], 1.0, 1.0)
        });
        assert_eq!(
            list.impulse([0.5, 0.0, 0.0], 1.0),
            [0.0, 0.5, 0.0],
            "full directionality pushes along forward (normalised)"
        );
        let mut full = ParticleForceList::new();
        for _ in 0..MAX_FORCES {
            assert!(full.push(sphere([0.0; 3], 1.0, 1.0)));
        }
        assert_eq!(full.len(), MAX_FORCES);
        assert!(
            !full.push(sphere([0.0; 3], 1.0, 1.0)),
            "the 33rd is refused"
        );
        full.clear();
        assert!(full.is_empty());
    }
}
