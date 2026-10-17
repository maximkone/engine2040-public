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
