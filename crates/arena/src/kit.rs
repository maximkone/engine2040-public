//! kit: FPS template slice over the arena placement table, plus the drill
//! post of the armatura.
//!
//! Owner: engine-arena (`world::kit`). Consumer: the game windowed plus
//! headless twin stages this template (collision plus FPS spawn points)
//! beside the glTF kit. Data owner: this module owns the placement parse;
//! [`MatchWorld`] owns collision. Units: meters, radians (yaw about +Y).
//! Determinism: same bytes give the same sorted world and spawns; nothing
//! here feeds the sim hash. Errors: typed [`KitError`], never panics.
//! Budget: one box per physical actor (6 on the shipped placement) parsed
//! once at init; zero per-tick work.
//!
//! The armatura's drill post ([`PALUS_POST`]) is staged here too: a fixed
//! post with three marked targets. The game scores the drill against these
//! mark heights, so the post and the scoring cannot drift.

use core::fmt;

use super::{MatchWorld, Obstacle, WorldError};

/// Embedded placement table (`content/arena/placement.json`).
pub const KIT_JSON: &str = include_str!("../../../content/arena/placement.json");
/// Actors in the placement table.
pub const KIT_ACTOR_COUNT: usize = 11;
/// FPS duel spawns in the template map.
pub const FPS_SPAWN_COUNT: usize = 2;
/// Clamp half-extent for settling FPS spawns, meters.
pub const FPS_LIMIT_M: f32 = 40.0;
/// Largest placement coordinate accepted, meters.
const MAX_KIT_POS_M: f32 = 500.0;
/// Largest placement full extent accepted, meters.
const MAX_KIT_SIZE_M: f32 = 100.0;

/// Why the FPS template refused the placement; every malformed input maps
/// to one variant, never a panic.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KitError {
    /// Placement JSON has the wrong shape or a bad value; static detail.
    Parse(&'static str),
    /// Parsed boxes failed [`MatchWorld::try_new`]; culprit carried.
    World(WorldError),
}

impl fmt::Display for KitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(detail) => write!(f, "kit placement: {detail}"),
            Self::World(error) => write!(f, "kit world: {error}"),
        }
    }
}

impl std::error::Error for KitError {}

/// FPS template map: kit collision plus settled duel spawns.
#[derive(Clone, Debug, PartialEq)]
pub struct FpsMap {
    /// Collision world over the physical kit actors.
    pub world: MatchWorld,
    /// Settled spawn feet `[x, z]` in meters, free of obstacles.
    pub spawns: [[f32; 2]; FPS_SPAWN_COUNT],
    /// Spawn facing yaw about +Y in radians (0 faces +z).
    pub yaws: [f32; FPS_SPAWN_COUNT],
}

/// The drill post of the armatura: one fixed post with three marked
/// targets. The post is stage geometry, not a fighter: the box only stops
/// the trainee from standing inside the post.
/// Units: meters on the floor, millimeters up the post.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PalusPost {
    /// Post center on the arena floor, meters.
    pub center: [f32; 2],
    /// Half extents of the post box, meters.
    pub half: [f32; 2],
    /// Height of each mark above the floor, millimeters: head, body, legs.
    pub marks_mm: [u32; 3],
    /// Height of the post top above the floor, millimeters.
    pub height_mm: u32,
}

/// The drill post of the armatura stage: a fixed post with three marked
/// targets (head, body, legs). The game scores the drill against these
/// mark heights, so the post and the scoring cannot drift.
pub const PALUS_POST: PalusPost = PalusPost {
    center: [0.0, 0.0],
    half: [0.2, 0.2],
    marks_mm: [1700, 1200, 500],
    height_mm: 1800,
};

/// Where the trainee stands for the drill: in front of the post and clear
/// of it, so the wooden sword reaches the marks.
pub const PALUS_SPOT_M: [f32; 2] = [0.0, -1.5];

/// Facing yaw of the drill spot: 0 faces +z, toward the post.
pub const PALUS_SPOT_YAW: f32 = 0.0;

/// The drill stage world: the post box alone, so a fighter cannot stand
/// inside the post. Built once at init; zero per-tick work.
pub fn palus_post_world() -> Result<MatchWorld, KitError> {
    MatchWorld::try_new(vec![Obstacle::aligned(PALUS_POST.center, PALUS_POST.half)])
        .map_err(KitError::World)
}

/// Raw FPS spawn feet: west pair between the walls, clear of the gate and
/// the stands. Deterministic constants; presentation only, never hashed.
pub fn fps_spawns() -> [[f32; 2]; FPS_SPAWN_COUNT] {
    [[-2.0, -6.0], [-2.0, 6.0]]
}

/// Facing yaw per raw spawn: slot 0 faces +z at its foe, slot 1 faces -z.
pub fn fps_spawn_yaws() -> [f32; FPS_SPAWN_COUNT] {
    [0.0, core::f32::consts::PI]
}

/// Collision world for the embedded placement: every physical actor as a
/// floor box, so walls stop being pictures.
///
/// Caller: [`fps_test_map`] (same module) settles the FPS spawns against
/// this world; the game twin reads it beside the glTF kit.
pub fn kit_match_world() -> Result<MatchWorld, KitError> {
    kit_match_world_from_json(KIT_JSON)
}

/// Collision world for explicit placement bytes: same parse as
/// [`kit_match_world`], so tests pin the failure cases without touching
/// the embedded file.
pub fn kit_match_world_from_json(text: &str) -> Result<MatchWorld, KitError> {
    let obstacles = placement_obstacles(text)?;
    MatchWorld::try_new(obstacles).map_err(KitError::World)
}

/// FPS template map over the embedded placement: collision plus the raw
/// spawns settled out of every obstacle.
///
/// Caller: [`MatchWorld::fps_template`](super::MatchWorld::fps_template)
/// (same crate) wires the map to the game twin in one call.
pub fn fps_test_map() -> Result<FpsMap, KitError> {
    let world = kit_match_world()?;
    let raw = fps_spawns();
    let mut spawns = raw;
    for (index, spawn) in spawns.iter_mut().enumerate() {
        let settled = world
            .settle(raw[index], crate::FIGHTER_RADIUS_M, FPS_LIMIT_M)
            .ok_or(KitError::Parse("kit fps spawn blocked"))?;
        *spawn = settled;
    }
    Ok(FpsMap {
        world,
        spawns,
        yaws: fps_spawn_yaws(),
    })
}

/// Physical floor boxes in file order, skipping non-physical dressing.
fn placement_obstacles(text: &str) -> Result<Vec<Obstacle>, KitError> {
    let mut out = Vec::with_capacity(KIT_ACTOR_COUNT);
    for block in actor_blocks(text)? {
        if let Some(obstacle) = parse_box(block)? {
            out.push(obstacle);
        }
    }
    Ok(out)
}

/// Actor object slices between the `"actors"` array brackets; anything but
/// exactly [`KIT_ACTOR_COUNT`] is shape drift and fails loudly.
fn actor_blocks(text: &str) -> Result<Vec<&str>, KitError> {
    let anchor = text
        .find("\"actors\"")
        .ok_or(KitError::Parse("kit placement misses actors"))?;
    let tail = text
        .get(anchor..)
        .ok_or(KitError::Parse("kit placement misses actors"))?;
    let open = tail
        .find('[')
        .ok_or(KitError::Parse("kit placement misses actors"))?;
    let mut cursor = anchor.saturating_add(open).saturating_add(1);
    let mut out = Vec::new();
    loop {
        let rest = text
            .get(cursor..)
            .ok_or(KitError::Parse("kit actor missing"))?;
        let is_shut = match (rest.find('{'), rest.find(']')) {
            (_, None) => false,
            (None, Some(_)) => true,
            (Some(o), Some(c)) => c < o,
        };
        if is_shut {
            break;
        }
        let open_at = rest.find('{').ok_or(KitError::Parse("kit actor missing"))?;
        let body = rest
            .get(open_at.saturating_add(1)..)
            .ok_or(KitError::Parse("kit actor missing"))?;
        let shut_at = body.find('}').ok_or(KitError::Parse("kit actor missing"))?;
        let block = body
            .get(..shut_at)
            .ok_or(KitError::Parse("kit actor missing"))?;
        out.push(block);
        cursor = cursor
            .saturating_add(open_at)
            .saturating_add(1)
            .saturating_add(shut_at)
            .saturating_add(1);
    }
    if out.len() != KIT_ACTOR_COUNT {
        return Err(KitError::Parse("kit actor count drift"));
    }
    Ok(out)
}

/// Raw field slice after `"needle":` in one actor block.
fn field_raw<'a>(block: &'a str, needle: &str) -> Result<&'a str, KitError> {
    let at = block
        .find(needle)
        .ok_or(KitError::Parse("kit actor field missing"))?;
    let rest = block
        .get(at.saturating_add(needle.len())..)
        .ok_or(KitError::Parse("kit actor field missing"))?;
    let colon = rest
        .find(':')
        .ok_or(KitError::Parse("kit actor field missing"))?;
    rest.get(colon.saturating_add(1)..)
        .ok_or(KitError::Parse("kit actor field missing"))
}

/// One finite `f32` field; missing, unparsable and non-finite fail loudly.
fn parse_f32(block: &str, needle: &str) -> Result<f32, KitError> {
    let rest = field_raw(block, needle)?;
    let end = rest.find([',', '}']).unwrap_or(rest.len());
    let token = rest
        .get(..end)
        .ok_or(KitError::Parse("kit actor number unparsable"))?;
    let value: f32 = token
        .trim()
        .parse()
        .map_err(|_| KitError::Parse("kit actor number unparsable"))?;
    if !value.is_finite() {
        return Err(KitError::Parse("kit actor number not finite"));
    }
    Ok(value)
}

/// One `"physical"` flag; missing and non-boolean fail loudly so an unread
/// flag never silently drops collision.
fn parse_physical(block: &str) -> Result<bool, KitError> {
    let rest = field_raw(block, "\"physical\"")?;
    let end = rest.find([',', '}']).unwrap_or(rest.len());
    let token = rest
        .get(..end)
        .ok_or(KitError::Parse("kit actor physical unparsable"))?;
    match token.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(KitError::Parse("kit actor physical unparsable")),
    }
}

/// Floor box for one actor block; `None` for non-physical dressing. Only
/// the floor-plane lanes parse (`y` stays presentation-only); nonzero yaw
/// on a physical actor fails loudly so the sim never calls `sin` or `cos`.
fn parse_box(block: &str) -> Result<Option<Obstacle>, KitError> {
    if !parse_physical(block)? {
        return Ok(None);
    }
    let x = parse_f32(block, "\"x\"")?;
    let z = parse_f32(block, "\"z\"")?;
    let sx = parse_f32(block, "\"sx\"")?;
    let sz = parse_f32(block, "\"sz\"")?;
    let yaw = parse_f32(block, "\"yaw\"")?;
    if x.abs() > MAX_KIT_POS_M || z.abs() > MAX_KIT_POS_M {
        return Err(KitError::Parse("kit actor out of bounds"));
    }
    if !(sx > 0.0 && sx <= MAX_KIT_SIZE_M && sz > 0.0 && sz <= MAX_KIT_SIZE_M) {
        return Err(KitError::Parse("kit actor size out of range"));
    }
    if yaw != 0.0 {
        return Err(KitError::Parse("kit actor yaw must be 0 for collision"));
    }
    Ok(Some(Obstacle {
        center: [x, z],
        half: [sx * 0.5, sz * 0.5],
        axis: [1.0, 0.0],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eleven(block: &str) -> String {
        let mut text = String::from("{\"actors\": [");
        for index in 0..KIT_ACTOR_COUNT {
            if index > 0 {
                text.push(',');
            }
            text.push_str(block);
        }
        text.push_str("]}");
        text
    }

    /// The embedded placement stages 11 actors and parses 6 physical boxes
    /// (the name counts the staged actors, the world holds the physical set).
    #[test]
    fn kit_parses_11_physical_obstacles() {
        let world = kit_match_world().expect("embedded placement collides");
        assert_eq!(world.obstacles().len(), 6, "6 physical boxes collide");
        assert!(!world.is_empty());
        assert!(
            world
                .obstacles()
                .iter()
                .any(|o| o.center == [-2.0, -10.0] && o.half == [4.0, 0.5]),
            "north wall blocks: {:?}",
            world.obstacles()
        );
    }

    #[test]
    fn kit_world_digest_is_stable_x2() {
        let first = kit_match_world().expect("embedded placement collides");
        let second = kit_match_world().expect("world builds twice");
        assert_eq!(first.digest(), second.digest());
        assert_ne!(first.digest(), MatchWorld::empty().digest());
    }

    /// The FPS spawns settle free over the 6 physical boxes of the shipped
    /// placement (the dressing no longer pads the world).
    #[test]
    fn kit_fps_spawns_settle_free_and_face() {
        let map = fps_test_map().expect("fps template settles");
        assert_eq!(map.world.obstacles().len(), 6, "6 physical boxes collide");
        assert_eq!(map.spawns, fps_spawns());
        assert_eq!(map.yaws, fps_spawn_yaws());
        for spawn in map.spawns {
            assert!(
                !map.world.overlaps(spawn, crate::FIGHTER_RADIUS_M),
                "spawn sits free: {spawn:?}"
            );
        }
        assert!(map.yaws[0] < map.yaws[1]);
    }

    #[test]
    fn kit_template_wires_through_match_world() {
        let via_world = MatchWorld::fps_template().expect("template wires through the world");
        let direct = fps_test_map().expect("direct template matches");
        assert_eq!(via_world, direct);
    }

    #[test]
    fn kit_rejects_shape_drift() {
        assert!(kit_match_world_from_json("{\"actors\": []}").is_err());
        assert!(kit_match_world_from_json("{\"nope\": 1}").is_err());
        assert!(kit_match_world_from_json("").is_err());
        let one = eleven(
            "{\"x\": 0.0, \"z\": 0.0, \"sx\": 1.0, \"sz\": 1.0, \"yaw\": 0.0, \"physical\": true}",
        );
        let mut ten = one.clone();
        ten.truncate(one.rfind(',').expect("eleven blocks joined"));
        ten.push_str("]}");
        assert!(kit_match_world_from_json(&ten).is_err());
    }

    #[test]
    fn kit_rejects_bad_yaw_size_or_flag() {
        // Bad shapes ride the `eleven` fixture (all boxes physical), so a
        // dressing flag flip in the shipped data can never disarm them: a
        // non-physical actor never reaches the box checks at all.
        let yawed = eleven(
            "{\"x\": 0.0, \"z\": 0.0, \"sx\": 1.0, \"sz\": 1.0, \"yaw\": 1.0, \"physical\": true}",
        );
        assert!(kit_match_world_from_json(&yawed).is_err());
        let flat = eleven(
            "{\"x\": 0.0, \"z\": 0.0, \"sx\": 0.0, \"sz\": 1.0, \"yaw\": 0.0, \"physical\": true}",
        );
        assert!(kit_match_world_from_json(&flat).is_err());
        let missing = eleven("{\"x\": 0.0, \"z\": 0.0, \"sx\": 1.0, \"sz\": 1.0, \"yaw\": 0.0}");
        assert!(kit_match_world_from_json(&missing).is_err());
        let bad_flag = eleven(
            "{\"x\": 0.0, \"z\": 0.0, \"sx\": 1.0, \"sz\": 1.0, \"yaw\": 0.0, \"physical\": maybe}",
        );
        assert!(kit_match_world_from_json(&bad_flag).is_err());
    }

    #[test]
    fn kit_blocked_spawn_has_no_settlement() {
        let cover = eleven(
            "{\"x\": 0.0, \"z\": 0.0, \"sx\": 80.0, \"sz\": 80.0, \"yaw\": 0.0, \"physical\": true}",
        );
        let world = kit_match_world_from_json(&cover).expect("covering boxes collide");
        assert_eq!(world.obstacles().len(), KIT_ACTOR_COUNT);
        assert!(world
            .settle([-2.0, -6.0], crate::FIGHTER_RADIUS_M, FPS_LIMIT_M)
            .is_none());
    }

    /// The drill post of the armatura: one post, three marks and a spot the
    /// trainee can stand on.
    #[test]
    fn kit_palus_post_stands_with_three_marks() {
        let world = palus_post_world().expect("the post collides");
        assert_eq!(world.obstacles().len(), 1);
        assert_eq!(world.obstacles()[0].center, PALUS_POST.center);
        assert_eq!(world.obstacles()[0].half, PALUS_POST.half);
        // The post blocks its own center and leaves the drill spot free.
        assert!(world.overlaps(PALUS_POST.center, crate::FIGHTER_RADIUS_M));
        assert!(!world.overlaps(PALUS_SPOT_M, crate::FIGHTER_RADIUS_M));
        // The marks run head, body, legs down the post.
        assert_eq!(PALUS_POST.marks_mm, [1700, 1200, 500]);
        const {
            assert!(PALUS_POST.marks_mm[0] > PALUS_POST.marks_mm[1]);
            assert!(PALUS_POST.marks_mm[1] > PALUS_POST.marks_mm[2]);
            assert!(PALUS_POST.marks_mm[0] <= PALUS_POST.height_mm);
        }
        // The spot faces the post: yaw 0 faces +z.
        assert_eq!(PALUS_SPOT_YAW, 0.0);
        const {
            assert!(PALUS_POST.center[1] > PALUS_SPOT_M[1]);
        }
    }

    #[test]
    fn kit_palus_post_world_digest_is_stable_x2() {
        let first = palus_post_world().expect("the post collides");
        let second = palus_post_world().expect("the post collides");
        assert_eq!(first.digest(), second.digest());
        assert_ne!(first.digest(), MatchWorld::empty().digest());
    }
}
