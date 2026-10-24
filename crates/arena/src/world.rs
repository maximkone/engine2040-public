//! world: the static obstacles a match is played around.
//!
//! Owner: engine-arena. Producers: the scene-to-match loader (the endpoint
//! reads a scene document revision and builds one [`MatchWorld`]); consumer:
//! the spine step ([`crate::spine_step`]), which resolves every fighter
//! against the obstacles once per tick. An empty world is the legacy match
//! exactly (the spine step then reproduces the frozen checkpoints byte for
//! byte); a world with obstacles folds its digest into every checkpoint, so
//! two scene revisions never share a hash even when no fighter touches the
//! difference.
//!
//! Shapes: one oriented box per obstacle on the floor plane (center, half
//! extents, a unit axis `(cos yaw, sin yaw)`). The loader derives the axis
//! from the scene rotation algebraically, so the sim never calls a platform
//! `sin` or `cos`. Fighters are circles; resolution is a push-out along the
//! contact normal with a fixed iteration count, then a rollback to the last
//! valid position if a corner pinch cannot be solved, so a fighter is never
//! inside an obstacle at any tick.
//!
//! A world may also carry a kit zone of cooked static colliders
//! ([`kit_zone::KitZone`], added by [`MatchWorld::with_kit_zone`]): hulls and meshes
//! from the art, resolved by the same push-out. A world built without one is unchanged.
//!
//! Units: meters. Determinism: `+ - * /`, `sqrt` and comparisons only,
//! obstacles in canonical (sorted bytes) order, no hash maps.

use core::fmt;

use engine_core::Fnv64;

use crate::canon_zero;

/// FPS template slice over the placement table (collision plus FPS
/// spawns); start at [`kit::fps_test_map`].
#[path = "kit.rs"]
pub mod kit;

/// Cooked static colliders (hulls and meshes from the art) as extra walls for the
/// capsule controller; opt in through [`MatchWorld::with_kit_zone`].
#[path = "kit_zone.rs"]
pub mod kit_zone;

/// Most obstacles one match carries (a scene far above this is a level, not
/// an arena).
pub const MAX_OBSTACLES: usize = 64;

/// Largest obstacle half extent in meters (the arena itself caps at 100).
pub const MAX_OBSTACLE_HALF_M: f32 = 100.0;

/// Clearance added to every push-out so a resolved circle never touches.
const SKIN_M: f32 = 1.0e-4;

/// Push-out passes per resolution (obstacles touching each other need more
/// than one).
const PASSES: usize = 4;

/// One oriented box on the floor plane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Obstacle {
    /// Center `[x, z]` in meters.
    pub center: [f32; 2],
    /// Half extents `[x, z]` along the box axes, meters, `> 0`.
    pub half: [f32; 2],
    /// Unit box axis `[cos yaw, sin yaw]`; `[1, 0]` is axis-aligned.
    pub axis: [f32; 2],
}

impl Obstacle {
    /// Axis-aligned box.
    pub const fn aligned(center: [f32; 2], half: [f32; 2]) -> Self {
        Self {
            center,
            half,
            axis: [1.0, 0.0],
        }
    }

    /// Point `p` in the box frame (x along the axis, z across it).
    fn to_local(self, p: [f32; 2]) -> [f32; 2] {
        let (dx, dz) = (p[0] - self.center[0], p[1] - self.center[1]);
        [
            dx * self.axis[0] + dz * self.axis[1],
            -dx * self.axis[1] + dz * self.axis[0],
        ]
    }

    /// Box-frame vector back to the floor plane (no translation).
    fn to_world(self, l: [f32; 2]) -> [f32; 2] {
        [
            l[0] * self.axis[0] - l[1] * self.axis[1],
            l[0] * self.axis[1] + l[1] * self.axis[0],
        ]
    }

    /// True when a circle of `radius` at `p` overlaps the box.
    pub fn overlaps(&self, p: [f32; 2], radius: f32) -> bool {
        let l = self.to_local(p);
        let qx = l[0].clamp(-self.half[0], self.half[0]);
        let qz = l[1].clamp(-self.half[1], self.half[1]);
        let (ex, ez) = (l[0] - qx, l[1] - qz);
        if ex == 0.0 && ez == 0.0 {
            return true;
        }
        ex * ex + ez * ez < radius * radius
    }

    /// Push a circle out of the box along the contact normal (shortest way
    /// out when the center is inside); `None` when it does not overlap.
    pub fn push_out(&self, p: [f32; 2], radius: f32) -> Option<[f32; 2]> {
        if !self.overlaps(p, radius) {
            return None;
        }
        let l = self.to_local(p);
        let qx = l[0].clamp(-self.half[0], self.half[0]);
        let qz = l[1].clamp(-self.half[1], self.half[1]);
        let (ex, ez) = (l[0] - qx, l[1] - qz);
        let out = if ex == 0.0 && ez == 0.0 {
            // Center inside: leave through the nearest face.
            let (px, pz) = (self.half[0] - l[0].abs(), self.half[1] - l[1].abs());
            if px < pz {
                let side = if l[0] < 0.0 { -1.0 } else { 1.0 };
                [side * (self.half[0] + radius + SKIN_M), l[1]]
            } else {
                let side = if l[1] < 0.0 { -1.0 } else { 1.0 };
                [l[0], side * (self.half[1] + radius + SKIN_M)]
            }
        } else {
            let dist = (ex * ex + ez * ez).sqrt();
            let reach = radius + SKIN_M;
            [qx + ex / dist * reach, qz + ez / dist * reach]
        };
        let w = self.to_world(out);
        Some([
            canon_zero(self.center[0] + w[0]),
            canon_zero(self.center[1] + w[1]),
        ])
    }
}

/// Why a world was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldError {
    /// More than [`MAX_OBSTACLES`] obstacles.
    TooMany(usize),
    /// An obstacle held NaN or infinity (index carried).
    NonFinite(usize),
    /// A half extent was not in `(0, MAX_OBSTACLE_HALF_M]` (index carried).
    BadHalf(usize),
    /// An axis had no length (index carried).
    BadAxis(usize),
}

impl fmt::Display for WorldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooMany(n) => write!(f, "world holds {n} obstacles, at most {MAX_OBSTACLES}"),
            Self::NonFinite(i) => write!(f, "obstacle {i} holds a non-finite number"),
            Self::BadHalf(i) => write!(
                f,
                "obstacle {i} half extent must be in (0, {MAX_OBSTACLE_HALF_M}] meters"
            ),
            Self::BadAxis(i) => write!(f, "obstacle {i} axis has no length"),
        }
    }
}

impl std::error::Error for WorldError {}

/// The obstacles of one match, in canonical order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MatchWorld {
    obstacles: Vec<Obstacle>,
    /// Cooked kit-zone colliders; `None` for every world built without
    /// [`MatchWorld::with_kit_zone`], which then plays and hashes as before.
    zone: Option<kit_zone::KitZone>,
}

impl MatchWorld {
    /// No obstacles: the legacy match.
    pub fn empty() -> Self {
        Self::default()
    }

    /// FPS template map over the embedded kit placement: the physical kit
    /// actors as collision plus two settled duel spawns for the FPS view.
    ///
    /// Consumer: the game windowed plus headless twin stages this template
    /// beside the glTF kit. Units: meters, radians. Determinism: same
    /// bytes give the same world and spawns; never feeds the sim hash.
    /// Errors: [`kit::KitError`] when the placement or a spawn is bad.
    /// Budget: parsed once at init; zero per tick.
    ///
    /// Caller: [`kit::fps_test_map`] (same crate) builds the map.
    pub fn fps_template() -> Result<kit::FpsMap, kit::KitError> {
        kit::fps_test_map()
    }

    /// Validate, normalize axes and sort into canonical order (center, then
    /// half, then axis, by `to_bits`), so the same set of boxes always makes
    /// the same world whatever order a scene listed them in.
    pub fn try_new(mut obstacles: Vec<Obstacle>) -> Result<Self, WorldError> {
        if obstacles.len() > MAX_OBSTACLES {
            return Err(WorldError::TooMany(obstacles.len()));
        }
        for (index, o) in obstacles.iter_mut().enumerate() {
            let lanes = [
                o.center[0],
                o.center[1],
                o.half[0],
                o.half[1],
                o.axis[0],
                o.axis[1],
            ];
            if lanes.iter().any(|lane| !lane.is_finite()) {
                return Err(WorldError::NonFinite(index));
            }
            if !(o.half[0] > 0.0
                && o.half[1] > 0.0
                && o.half[0] <= MAX_OBSTACLE_HALF_M
                && o.half[1] <= MAX_OBSTACLE_HALF_M)
            {
                return Err(WorldError::BadHalf(index));
            }
            let len = (o.axis[0] * o.axis[0] + o.axis[1] * o.axis[1]).sqrt();
            if len < 1.0e-6 {
                return Err(WorldError::BadAxis(index));
            }
            o.axis = [o.axis[0] / len, o.axis[1] / len];
            o.center = [canon_zero(o.center[0]), canon_zero(o.center[1])];
        }
        obstacles.sort_by_key(bits);
        Ok(Self {
            obstacles,
            zone: None,
        })
    }

    /// This world plus a kit zone of cooked static colliders: the explicit opt-in
    /// constructor for collision that comes from the art. The boxes keep their order
    /// and their digest words; the zone adds its own digest after them, so a world
    /// built without a zone is untouched. An empty zone changes nothing.
    ///
    /// Consumer: the game's window session opts in here; the door's cast verbs read
    /// the zone back through [`MatchWorld::kit_zone`].
    #[must_use]
    pub fn with_kit_zone(mut self, zone: kit_zone::KitZone) -> Self {
        self.zone = (!zone.is_empty()).then_some(zone);
        self
    }

    /// The kit zone, when this world was built with one.
    pub fn kit_zone(&self) -> Option<&kit_zone::KitZone> {
        self.zone.as_ref()
    }

    /// Obstacles in canonical order.
    pub fn obstacles(&self) -> &[Obstacle] {
        &self.obstacles
    }

    /// True when the world is the legacy empty arena.
    pub fn is_empty(&self) -> bool {
        self.obstacles.is_empty() && self.zone.is_none()
    }

    /// FNV-1a over the canonical obstacle bytes, then the kit zone digest when there is
    /// one; `0` for the empty world.
    pub fn digest(&self) -> u64 {
        if self.is_empty() {
            return 0;
        }
        let mut hash = Fnv64::new();
        hash.write_u32(u32::try_from(self.obstacles.len()).unwrap_or(u32::MAX));
        for o in &self.obstacles {
            for word in bits(o) {
                hash.write_u32(word);
            }
        }
        if let Some(zone) = &self.zone {
            hash.write_u64(zone.digest());
        }
        hash.finish()
    }

    /// True when a circle at `p` overlaps any obstacle or kit-zone footprint.
    pub fn overlaps(&self, p: [f32; 2], radius: f32) -> bool {
        self.obstacles.iter().any(|o| o.overlaps(p, radius))
            || self.zone.as_ref().is_some_and(|z| z.overlaps(p, radius))
    }

    /// Move a circle from `from` (a valid, non-overlapping position) to `to`
    /// and resolve it against the obstacles: push out in canonical order for
    /// `PASSES` passes, re-clamp to `+-limit`, and fall back to `from`
    /// when a pinch cannot be solved. The result never overlaps.
    pub fn resolve(&self, from: [f32; 2], to: [f32; 2], radius: f32, limit: f32) -> [f32; 2] {
        let mut p = to;
        for _ in 0..PASSES {
            let mut moved = false;
            for o in &self.obstacles {
                if let Some(q) = o.push_out(p, radius) {
                    p = q;
                    moved = true;
                }
            }
            if let Some(q) = self.zone.as_ref().and_then(|z| z.push_out(p, radius)) {
                p = q;
                moved = true;
            }
            p = [
                canon_zero(p[0].clamp(-limit, limit)),
                canon_zero(p[1].clamp(-limit, limit)),
            ];
            if !moved {
                break;
            }
        }
        if self.overlaps(p, radius) {
            from
        } else {
            p
        }
    }

    /// Place a circle that starts inside an obstacle: push out and clamp
    /// like [`MatchWorld::resolve`] without a rollback target. `None` when
    /// no free spot is found (the world blocks the spawn).
    pub fn settle(&self, at: [f32; 2], radius: f32, limit: f32) -> Option<[f32; 2]> {
        let mut p = at;
        for _ in 0..(PASSES * 2) {
            if !self.overlaps(p, radius) {
                return Some(p);
            }
            for o in &self.obstacles {
                if let Some(q) = o.push_out(p, radius) {
                    p = q;
                }
            }
            if let Some(q) = self.zone.as_ref().and_then(|z| z.push_out(p, radius)) {
                p = q;
            }
            p = [
                canon_zero(p[0].clamp(-limit, limit)),
                canon_zero(p[1].clamp(-limit, limit)),
            ];
        }
        if self.overlaps(p, radius) {
            None
        } else {
            Some(p)
        }
    }
}

/// Canonical sort and digest key: the six `f32` lanes as `to_bits`.
fn bits(o: &Obstacle) -> [u32; 6] {
    [
        o.center[0].to_bits(),
        o.center[1].to_bits(),
        o.half[0].to_bits(),
        o.half[1].to_bits(),
        o.axis[0].to_bits(),
        o.axis[1].to_bits(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall() -> Obstacle {
        // A 2 m long, 0.4 m thick wall across x at z = 0.
        Obstacle::aligned([0.0, 0.0], [1.0, 0.2])
    }

    #[test]
    fn empty_world_digest_is_zero_and_never_overlaps() {
        let world = MatchWorld::empty();
        assert!(world.is_empty());
        assert_eq!(world.digest(), 0);
        assert!(!world.overlaps([0.0, 0.0], 0.4));
    }

    #[test]
    fn push_out_leaves_the_box_by_the_shortest_way() {
        let o = wall();
        // Inside, near the north face: leaves through the north face.
        let q = o.push_out([0.0, -0.1], 0.4).expect("inside overlaps");
        assert!(q[1] < -0.2 - 0.4, "out of the box: {q:?}");
        assert!(!o.overlaps(q, 0.4));
        // Outside but touching the end cap.
        let q = o.push_out([1.2, 0.0], 0.4).expect("touching overlaps");
        assert!(q[0] >= 1.4 && !o.overlaps(q, 0.4));
        // Clear of the wall: no push.
        assert!(o.push_out([0.0, 3.0], 0.4).is_none());
    }

    #[test]
    fn rotated_box_pushes_along_its_own_axes() {
        // Box turned 90 degrees: long axis along z.
        let o = Obstacle {
            center: [0.0, 0.0],
            half: [1.0, 0.2],
            axis: [0.0, 1.0],
        };
        let q = o
            .push_out([0.05, 0.0], 0.4)
            .expect("overlaps the long side");
        assert!(!o.overlaps(q, 0.4));
        assert!(q[0].abs() > 0.2 + 0.4 - 1.0e-3, "pushed sideways: {q:?}");
    }

    #[test]
    fn resolve_blocks_a_walk_through_the_wall() {
        let world = MatchWorld::try_new(vec![wall()]).expect("valid");
        let mut p = [0.0, 3.0];
        // Walk straight at the wall in 1 cm steps, 4 m: never overlaps.
        for _ in 0..400 {
            p = world.resolve(p, [p[0], p[1] - 0.01], 0.4, 19.6);
            assert!(!world.overlaps(p, 0.4), "inside the wall at {p:?}");
        }
        assert!(p[1] >= 0.2 + 0.4 - 1.0e-3, "stopped at the face: {p:?}");
    }

    #[test]
    fn order_and_digest_are_canonical_and_move_with_the_set() {
        let a = Obstacle::aligned([3.0, 1.0], [0.5, 0.5]);
        let b = Obstacle::aligned([-2.0, 4.0], [1.0, 0.25]);
        let ab = MatchWorld::try_new(vec![a, b]).expect("valid");
        let ba = MatchWorld::try_new(vec![b, a]).expect("valid");
        assert_eq!(ab, ba);
        assert_eq!(ab.digest(), ba.digest());
        assert_ne!(ab.digest(), 0);
        let only_a = MatchWorld::try_new(vec![a]).expect("valid");
        assert_ne!(ab.digest(), only_a.digest());
    }

    #[test]
    fn bad_worlds_refuse_loudly() {
        let nan = Obstacle::aligned([f32::NAN, 0.0], [1.0, 1.0]);
        assert_eq!(
            MatchWorld::try_new(vec![nan]),
            Err(WorldError::NonFinite(0))
        );
        let flat = Obstacle::aligned([0.0, 0.0], [0.0, 1.0]);
        assert_eq!(MatchWorld::try_new(vec![flat]), Err(WorldError::BadHalf(0)));
        let dead = Obstacle {
            center: [0.0, 0.0],
            half: [1.0, 1.0],
            axis: [0.0, 0.0],
        };
        assert_eq!(MatchWorld::try_new(vec![dead]), Err(WorldError::BadAxis(0)));
        let many = vec![wall(); MAX_OBSTACLES + 1];
        assert_eq!(
            MatchWorld::try_new(many),
            Err(WorldError::TooMany(MAX_OBSTACLES + 1))
        );
    }

    #[test]
    fn settle_moves_a_spawn_out_or_refuses() {
        let world = MatchWorld::try_new(vec![wall()]).expect("valid");
        let p = world
            .settle([0.0, 0.0], 0.4, 19.6)
            .expect("a free spot exists");
        assert!(!world.overlaps(p, 0.4));
        // A box covering the whole arena leaves no free spot.
        let all =
            MatchWorld::try_new(vec![Obstacle::aligned([0.0, 0.0], [30.0, 30.0])]).expect("valid");
        assert_eq!(all.settle([0.0, 0.0], 0.4, 4.6), None);
    }
}
