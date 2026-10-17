//! types: the plain data that crosses the boundary. No arena, combat or anim type appears here.

/// A rigid transform: position in meters and a unit quaternion `[x, y, z, w]`, Y up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    /// Position, meters.
    pub pos: [f32; 3],
    /// Orientation, unit quaternion `[x, y, z, w]`.
    pub rot: [f32; 4],
}

impl Transform {
    /// At the origin, no rotation.
    pub const IDENTITY: Self = Self {
        pos: [0.0; 3],
        rot: [0.0, 0.0, 0.0, 1.0],
    };

    /// `a` at `t = 0`, `b` at `t = 1`, both exactly: a lerp of the position and a
    /// shortest-arc normalized lerp of the rotation in between. `t` is already in `[0, 1]`.
    pub(crate) fn blend(a: Self, b: Self, t: f32) -> Self {
        if t <= 0.0 || a == b {
            return a;
        }
        if t >= 1.0 {
            return b;
        }
        let keep = 1.0 - t;
        let pos = [
            a.pos[0] * keep + b.pos[0] * t,
            a.pos[1] * keep + b.pos[1] * t,
            a.pos[2] * keep + b.pos[2] * t,
        ];
        // q and -q are one rotation: blend toward the sign that is on a's side.
        let dot: f32 = a.rot.iter().zip(&b.rot).map(|(x, y)| x * y).sum();
        let sign = if dot < 0.0 { -1.0 } else { 1.0 };
        let mut rot = [0.0; 4];
        for (out, (x, y)) in rot.iter_mut().zip(a.rot.iter().zip(&b.rot)) {
            *out = x * keep + sign * y * t;
        }
        let len = rot.iter().map(|c| c * c).sum::<f32>().sqrt();
        if len > 0.0 {
            rot = rot.map(|c| c / len);
        } else {
            rot = b.rot;
        }
        Self { pos, rot }
    }
}

/// What a read-back transform belongs to. The caller maps a key to its mesh.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// One bone of the ragdoll in a fighter slot, skeleton order.
    Bone {
        /// Fighter slot the ragdoll was spawned for.
        slot: u8,
        /// Bone index in the rig's skeleton.
        bone: u16,
    },
    /// A severed limb, named by the caller's part id.
    Limb {
        /// The part id given to `spawn_limb`.
        part: u32,
    },
    /// One piece of a debris batch.
    Shard {
        /// The [`Batch::id`] the shard came from.
        batch: u32,
        /// Index in the shard list given to `spawn_debris`.
        index: u32,
    },
    /// An arrow or a thrown object, named by the caller's id.
    Thrown {
        /// The id given to `spawn_thrown`.
        id: u32,
    },
}

/// One body's interpolated transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    /// Which body this is.
    pub key: Key,
    /// Where it is, blended between the last two ticks.
    pub at: Transform,
}

/// One piece of a broken wall: a box with a start pose and velocity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shard {
    /// Start pose.
    pub at: Transform,
    /// Box half-extents, meters.
    pub half: [f32; 3],
    /// Start velocity, m/s.
    pub velocity: [f32; 3],
}

/// What `spawn_debris` took from a shard list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Batch {
    /// Names this spawn in [`Key::Shard`].
    pub id: u32,
    /// Shards admitted: the first `admitted` of the list. The caller's order is its priority;
    /// the rest did not fit the body cap and were not spawned.
    pub admitted: usize,
}

/// An authority blast, with the integers the sim holds it in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Blast {
    /// Where it went off, millimeters.
    pub center_mm: [i32; 3],
    /// How far it reaches, millimeters: a body at this distance or farther is not touched.
    pub radius_mm: u32,
    /// The momentum, in newton-seconds, that a 70 kg body at the center would gain. It falls
    /// off linearly to nothing at the radius, and every body gains speed as if it weighed 70 kg,
    /// so a ragdoll stays in one piece and a metal shard flies as far as a wooden one.
    pub impulse_ns: u32,
}

/// An authority projectile hit, with the integers the sim holds it in: an arrow, a bolt or a
/// thrown stone landing on the world. Unlike a [`Blast`] it pushes along the projectile's heading
/// and the push is a real impulse, so a heavy body moves less than a light one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Impact {
    /// Where it landed, millimeters.
    pub at_mm: [i32; 3],
    /// The projectile's heading. Only the direction counts, not the length; not all zero.
    pub direction: [i16; 3],
    /// The projectile's momentum in newton-seconds at the hit (mass times speed), what a body
    /// at the hit point receives. It falls off linearly to nothing at `radius_mm`.
    pub impulse_ns: u32,
    /// How far around the hit point it shoves, millimeters.
    pub radius_mm: u32,
}

/// The authority's account of an arrow or a thrown object: where it left, where it hit and how
/// many ticks later. The hit stays authority; the picture only draws the arc between the two.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flight {
    /// Where the object left the hand or the string, meters.
    pub from: [f32; 3],
    /// Where authority says it hit, meters.
    pub to: [f32; 3],
    /// Ticks from launch to the hit.
    pub ticks: u32,
}

/// Counters for the page and the window to print.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Dynamic bodies alive.
    pub bodies: usize,
    /// Bodies the last step solved. Sleeping bodies are not in this number.
    pub awake: usize,
    /// Active contact pairs the last step paid for, counted before any shedding.
    pub contacts: usize,
    /// Bodies removed to honor a cap, since creation.
    pub recycled: u64,
    /// Ticks stepped since creation.
    pub steps: u64,
    /// Cost of the last step in microseconds by the clock given to `with_clock`; `0` without one.
    pub micros: u64,
}

impl Stats {
    /// The one receipt line the page and the window print for the physical layer, and the soak
    /// prints once per tier: `physfx: bodies=.. contacts=.. us=..`.
    pub fn receipt_line(&self) -> String {
        format!(
            "physfx: bodies={} contacts={} us={}",
            self.bodies, self.contacts, self.micros
        )
    }
}
