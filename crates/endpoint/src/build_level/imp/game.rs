//! The game parts of a placed level: which placements are the player, the enemy, the
//! goal, the floor and a wall.
//!
//! Consumer: `level.build` keeps these on the session and `game.play` reads them, so a
//! caller builds a game and plays exactly that level (the scene document holds poses,
//! not role names). Units: meters for a pose and a size. Determinism: role order then
//! name, so the list never moves with brief line order.

use super::{RelationPlan, SpineWrite};

/// What a brief kind means to the game, once the level is placed. Furniture is scenery;
/// the five game parts drive the objective.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Scenery: placed and gated like every prop, plays no part.
    Scenery,
    /// The seat the caller drives; reaching the goal while alive wins.
    Player,
    /// A seat that walks at the player every tick.
    Enemy,
    /// The zone that wins the game when the player stands in it.
    Goal,
    /// The walkable floor plate.
    Floor,
    /// A blocking wall slab.
    Wall,
}

impl Role {
    /// Stable tag for the door payload.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Scenery => "scenery",
            Self::Player => "player",
            Self::Enemy => "enemy",
            Self::Goal => "goal",
            Self::Floor => "floor",
            Self::Wall => "wall",
        }
    }

    /// The game roles in the order a level reports them.
    pub const GAME: [Role; 5] = [
        Self::Floor,
        Self::Wall,
        Self::Player,
        Self::Enemy,
        Self::Goal,
    ];
}

/// The role a brief kind plays. Unknown kinds read as scenery; the grammar refuses them
/// before this is asked.
pub fn role_of(kind: &str) -> Role {
    match kind {
        "player" => Role::Player,
        "enemy" => Role::Enemy,
        "goal" => Role::Goal,
        "floor" => Role::Floor,
        "wall" => Role::Wall,
        _ => Role::Scenery,
    }
}

/// One game part a level placed: its role, its brief name, its live document id and the
/// pose it landed at.
#[derive(Debug, Clone, PartialEq)]
pub struct GamePart {
    /// What the part is to the game.
    pub role: Role,
    /// Prop name from the brief.
    pub name: String,
    /// Live document id text.
    pub id: String,
    /// Position meters.
    pub x: f32,
    /// Position meters.
    pub z: f32,
    /// East-west size meters.
    pub sx: f32,
    /// North-south size meters.
    pub sz: f32,
}

/// The game parts of a placed level: only the kinds that carry a game role, never
/// scenery. `writes` and `ids` are the landed spine writes and their live document ids in
/// the same order; the kind comes from the plan by name. Pure; the caller keeps the
/// result.
pub fn game_parts(plan: &RelationPlan, writes: &[SpineWrite], ids: &[String]) -> Vec<GamePart> {
    let mut out: Vec<GamePart> = Vec::new();
    for (write, id) in writes.iter().zip(ids.iter()) {
        let Some(prop) = plan.props.iter().find(|prop| prop.name == write.name) else {
            continue;
        };
        let role = role_of(&prop.kind);
        if role == Role::Scenery {
            continue;
        }
        out.push(GamePart {
            role,
            name: prop.name.clone(),
            id: id.clone(),
            x: write.pos[0],
            z: write.pos[2],
            sx: write.scale[0].abs(),
            sz: write.scale[2].abs(),
        });
    }
    out.sort_by(|a, b| (a.role.as_str(), a.name.as_str()).cmp(&(b.role.as_str(), b.name.as_str())));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_level::{compile_brief, envelope, plan_from_compiled};

    fn parts_of(brief: &str) -> Vec<GamePart> {
        let env = envelope(brief, 11).expect("envelopes");
        let compiled = compile_brief(&env).expect("compiles");
        let plan = plan_from_compiled(&compiled);
        let writes: Vec<SpineWrite> = plan
            .props
            .iter()
            .map(|prop| SpineWrite {
                id: prop.id,
                name: prop.name.clone(),
                pos: [0.5, 0.0, -0.5],
                yaw: 0.0,
                scale: [1.0, 1.0, 1.0],
            })
            .collect();
        let ids: Vec<String> = plan.props.iter().map(|prop| prop.name.clone()).collect();
        game_parts(&plan, &writes, &ids)
    }

    #[test]
    fn scenery_is_dropped_and_the_five_game_parts_remain() {
        let brief = "prop crate crate_01\n\
                     prop floor floor_01\n\
                     prop wall wall_01\n\
                     prop player hero\n\
                     prop enemy brute\n\
                     prop goal keep\n\
                     spawn hero";
        let parts = parts_of(brief);
        let roles: Vec<&str> = parts.iter().map(|part| part.role.as_str()).collect();
        assert_eq!(roles, vec!["enemy", "floor", "goal", "player", "wall"]);
        assert!(
            !parts.iter().any(|part| part.name == "crate_01"),
            "scenery never becomes a game part"
        );
        assert_eq!(parts[0].x, 0.5);
        assert_eq!(parts[0].z, -0.5);
    }

    #[test]
    fn a_brief_with_no_game_kinds_has_no_parts() {
        assert!(parts_of("prop crate crate_01\nspawn crate_01").is_empty());
    }

    #[test]
    fn each_game_kind_reads_its_own_role() {
        for (kind, role) in [
            ("player", Role::Player),
            ("enemy", Role::Enemy),
            ("goal", Role::Goal),
            ("floor", Role::Floor),
            ("wall", Role::Wall),
            ("crate", Role::Scenery),
        ] {
            assert_eq!(role_of(kind), role, "{kind}");
        }
        assert_eq!(Role::GAME.len(), 5);
    }
}
