//! The selection panel and the command row: what the cursor picked and the
//! orders the page may give.
//!
//! The page never decides a rule of the match. It asks the sim which house or
//! unit stands under the cursor ([`select_at`]), reads the numbers of the one
//! it picked ([`house_state`], [`unit_state`]) and hands an order back as one
//! of the sim's own commands ([`command`]). The sim refuses in its own words,
//! so the bottom strip of the page carries the sim's code and never a copy of
//! it. A house has no health in the sim, so the panel of a house under
//! construction shows the ticks the sim still needs and the ware the sim's own
//! chain waits for.
//!
//! The pick is the renderer's own rule: the topmost thing by the depth key the
//! scene sorts by. A house is hit on the tiles of its footprint and a unit on
//! its own tile, and the two never share ground, so one cursor is one thing
//! and never a guess between two.

use kam2_sim::{Command, HouseKind, Sim, UnitKind, Ware};

/// The unit id the sim does not know: the page sends it when there is nothing
/// to order, and the sim answers `NoUnit` in its own words.
pub const NO_UNIT: u32 = u32::MAX;

/// The line for an order the page does not know; no sim code, because no sim
/// rule was ever asked.
pub const UNKNOWN_ORDER: &str = "BadOrder: that command does not exist";

/// The thing the page picked: a house or a unit, named by the sim's own id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    /// A house on the field.
    House(u32),
    /// A unit on the field.
    Unit(u32),
}

impl Selection {
    /// The id the page hands around: `h3` for a house, `u7` for a unit.
    #[must_use]
    pub fn id(self) -> String {
        match self {
            Self::House(id) => format!("h{id}"),
            Self::Unit(id) => format!("u{id}"),
        }
    }

    /// The selection an id names; `None` when the id names nothing.
    #[must_use]
    pub fn parse(id: &str) -> Option<Self> {
        let (tag, digits) = id.split_at_checked(1)?;
        let number = digits.parse().ok()?;
        match tag {
            "h" => Some(Self::House(number)),
            "u" => Some(Self::Unit(number)),
            _ => None,
        }
    }

    /// True when this selection is a unit: only a unit takes an order.
    #[must_use]
    pub const fn is_unit(self) -> bool {
        matches!(self, Self::Unit(_))
    }
}

/// The house or unit under the cursor at `(x, z)`.
///
/// The rule is the renderer's own: the topmost thing by the depth key the
/// scene sorts by, a house's footprint base row against a unit's tile. The
/// keys are whole tiles, so the order is the order the renderer draws in and
/// no float ever enters the answer.
#[must_use]
pub fn select_at(sim: &Sim, x: i32, z: i32) -> Option<Selection> {
    let mut best: Option<(i32, Selection)> = None;
    for house in sim.houses() {
        let (w, h) = house.kind.footprint();
        if !(house.x..house.x + w).contains(&x) || !(house.y..house.y + h).contains(&z) {
            continue;
        }
        let depth = house.x + house.y + h;
        if best.is_none_or(|(held, _)| depth > held) {
            best = Some((depth, Selection::House(house.id)));
        }
    }
    for unit in sim.units() {
        if unit.x != x || unit.y != z {
            continue;
        }
        let depth = unit.x + unit.y + 1;
        if best.is_none_or(|(held, _)| depth > held) {
            best = Some((depth, Selection::Unit(unit.id)));
        }
    }
    best.map(|(_, picked)| picked)
}

/// What the panel shows of a house: the sim's own numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HouseState {
    /// The sim's house id.
    pub id: u32,
    /// The side that owns it.
    pub side: u8,
    /// What it is.
    pub kind: HouseKind,
    /// Its top-left tile.
    pub x: i32,
    /// Its top-left row.
    pub z: i32,
    /// Ticks of building still to go; zero when it stands finished.
    pub ticks_left: u32,
    /// Ticks the build takes.
    pub ticks_total: u32,
    /// The ware it waits for once it works.
    pub waits_for: Option<Ware>,
}

/// The state of one house, read from the sim; `None` when no house has that
/// id.
#[must_use]
pub fn house_state(sim: &Sim, id: u32) -> Option<HouseState> {
    let house = sim.houses().iter().find(|house| house.id == id)?;
    Some(HouseState {
        id: house.id,
        side: house.side,
        kind: house.kind,
        x: house.x,
        z: house.y,
        ticks_left: house.built_at.saturating_sub(sim.ticks()),
        ticks_total: house.kind.build_ticks(),
        waits_for: waits_for(house.kind),
    })
}

/// The ware a house waits for once it works: the sim's own chain, the mill
/// eats grain and the bakery eats flour, the other kinds eat nothing.
#[must_use]
pub const fn waits_for(kind: HouseKind) -> Option<Ware> {
    match kind {
        HouseKind::Mill => Some(Ware::Grain),
        HouseKind::Bakery => Some(Ware::Flour),
        _ => None,
    }
}

/// What the panel shows of a unit: the sim's own numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnitState {
    /// The sim's unit id.
    pub id: u32,
    /// The side that owns it.
    pub side: u8,
    /// What it is.
    pub kind: UnitKind,
    /// Its tile.
    pub x: i32,
    /// Its row.
    pub z: i32,
    /// Health left.
    pub hp: u32,
    /// Health it fights with.
    pub max_hp: u32,
    /// True while it walks a path.
    pub moving: bool,
}

/// The state of one unit, read from the sim; `None` when no unit has that id.
#[must_use]
pub fn unit_state(sim: &Sim, id: u32) -> Option<UnitState> {
    let unit = sim.unit(id)?;
    Some(UnitState {
        id: unit.id,
        side: unit.side,
        kind: unit.kind,
        x: unit.x,
        z: unit.y,
        hp: unit.hp,
        max_hp: unit.kind.hp(),
        moving: !unit.path.is_empty(),
    })
}

/// The orders the command row may give.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    /// Walk to the tile under the cursor: the sim's own move.
    AttackMove,
    /// Stand still and forget the target: the sim's own move onto the unit's
    /// own tile.
    Stop,
    /// Stand still and strike the unit under the cursor whenever it is in
    /// reach: the sim's own attack.
    Hold,
}

impl Order {
    /// The order a name names; `None` when the name is not an order.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "attack-move" => Some(Self::AttackMove),
            "stop" => Some(Self::Stop),
            "hold" => Some(Self::Hold),
            _ => None,
        }
    }

    /// The name the page and the logs use.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::AttackMove => "attack-move",
            Self::Stop => "stop",
            Self::Hold => "hold",
        }
    }
}

/// The sim's own command for one order.
///
/// The command is always well formed: when there is nothing to order the page
/// sends [`NO_UNIT`] and the sim refuses in its own words, never the page.
#[must_use]
pub fn command(sim: &Sim, picked: Option<Selection>, order: Order, x: i32, z: i32) -> Command {
    let id = match picked {
        Some(Selection::Unit(id)) => id,
        Some(Selection::House(_)) | None => NO_UNIT,
    };
    match order {
        Order::AttackMove => Command::MoveUnit { id, x, y: z },
        Order::Stop => {
            let (unit_x, unit_z) = sim.unit(id).map_or((0, 0), |unit| (unit.x, unit.y));
            Command::MoveUnit {
                id,
                x: unit_x,
                y: unit_z,
            }
        }
        Order::Hold => {
            let target = sim
                .units()
                .iter()
                .find(|unit| unit.x == x && unit.y == z)
                .map_or(NO_UNIT, |unit| unit.id);
            Command::Attack { id, target }
        }
    }
}

/// The panel's JSON for the picked thing: the fields the page draws.
#[must_use]
pub fn state_json(sim: &Sim, picked: Option<Selection>) -> String {
    match picked {
        Some(Selection::House(id)) => house_json(sim, id),
        Some(Selection::Unit(id)) => unit_json(sim, id),
        None => String::new(),
    }
}

/// The panel's JSON for one house; empty when no house has that id.
#[must_use]
pub fn house_json(sim: &Sim, id: u32) -> String {
    let Some(state) = house_state(sim, id) else {
        return String::new();
    };
    format!(
        "{{\"id\":\"h{}\",\"what\":\"house\",\"name\":\"{}\",\"side\":{},\"x\":{},\"z\":{},\"ticksLeft\":{},\"ticksTotal\":{},\"building\":{},\"waitsFor\":\"{}\"}}",
        state.id,
        state.kind.name(),
        state.side,
        state.x,
        state.z,
        state.ticks_left,
        state.ticks_total,
        state.ticks_left > 0,
        state.waits_for.map_or("", |ware| ware.name()),
    )
}

/// The panel's JSON for one unit; empty when no unit has that id.
#[must_use]
pub fn unit_json(sim: &Sim, id: u32) -> String {
    let Some(state) = unit_state(sim, id) else {
        return String::new();
    };
    format!(
        "{{\"id\":\"u{}\",\"what\":\"unit\",\"name\":\"{}\",\"side\":{},\"x\":{},\"z\":{},\"hp\":{},\"maxHp\":{},\"moving\":{}}}",
        state.id,
        state.kind.name(),
        state.side,
        state.x,
        state.z,
        state.hp,
        state.max_hp,
        state.moving,
    )
}

/// The selection as one JSON list of ids: the newest pick last.
#[must_use]
pub fn selection_json(picks: &[Selection]) -> String {
    let ids = picks
        .iter()
        .map(|picked| format!("\"{}\"", picked.id()))
        .collect::<Vec<_>>()
        .join(",");
    format!("[{ids}]")
}

#[cfg(test)]
mod tests {
    use super::{
        command, house_state, select_at, selection_json, state_json, unit_state, waits_for, Order,
        Selection, NO_UNIT,
    };
    use kam2_game::AiMatch;
    use kam2_sim::{Command, HouseKind, Refusal, Ware};

    fn sim() -> kam2_sim::Sim {
        AiMatch::village()
            .expect("the start village places")
            .sim()
            .clone()
    }

    #[test]
    fn an_id_round_trips() {
        assert_eq!(Selection::parse("h3"), Some(Selection::House(3)));
        assert_eq!(Selection::parse("u7"), Some(Selection::Unit(7)));
        assert_eq!(Selection::House(3).id(), "h3");
        assert_eq!(Selection::Unit(7).id(), "u7");
        assert_eq!(Selection::parse(""), None);
        assert_eq!(Selection::parse("x3"), None);
        assert_eq!(Selection::parse("h"), None);
        assert_eq!(Selection::parse("u-1"), None);
    }

    #[test]
    fn the_cursor_picks_the_warehouse_and_the_swordsman() {
        let sim = sim();
        assert_eq!(
            select_at(&sim, 0, 3),
            Some(Selection::House(0)),
            "the warehouse covers its own footprint"
        );
        assert_eq!(
            select_at(&sim, 3, 9),
            Some(Selection::Unit(0)),
            "the swordsman stands on his tile"
        );
        assert_eq!(select_at(&sim, 15, 11), None, "empty ground picks nothing");
    }

    #[test]
    fn a_house_is_hit_on_every_tile_of_its_footprint() {
        let sim = sim();
        let warehouse = sim.houses().first().expect("the village has a warehouse");
        let (w, h) = warehouse.kind.footprint();
        for x in warehouse.x..warehouse.x + w {
            for z in warehouse.y..warehouse.y + h {
                assert_eq!(
                    select_at(&sim, x, z),
                    Some(Selection::House(warehouse.id)),
                    "the tile ({x},{z}) holds the warehouse"
                );
            }
        }
        assert_eq!(
            select_at(&sim, warehouse.x + w, warehouse.y),
            None,
            "the tile beside the footprint holds nothing"
        );
    }

    #[test]
    fn a_house_and_a_unit_never_share_ground_so_the_pick_is_never_a_guess() {
        let sim = sim();
        let farm = sim
            .houses()
            .iter()
            .find(|house| house.kind == HouseKind::Farm)
            .expect("the village has a farm");
        assert_eq!(
            select_at(&sim, farm.x, farm.y),
            Some(Selection::House(farm.id)),
            "the farm is hit on its own tile"
        );
        assert_eq!(
            select_at(&sim, 5, 9),
            Some(Selection::Unit(1)),
            "the second swordsman stands on his own tile"
        );
        for unit in sim.units() {
            for house in sim.houses() {
                let (w, h) = house.kind.footprint();
                assert!(
                    !(unit.x >= house.x
                        && unit.x < house.x + w
                        && unit.y >= house.y
                        && unit.y < house.y + h),
                    "a body and a house never share ground"
                );
            }
        }
    }

    #[test]
    fn a_picked_house_reports_the_ticks_and_the_ware_it_waits_for() {
        let mut sim = sim();
        sim.apply_for(
            0,
            Command::PlaceHouse {
                kind: HouseKind::Mill,
                x: 6,
                y: 4,
            },
        )
        .expect("the mill fits beside the road");
        let mill = sim.houses().last().expect("the mill stands");
        let state = house_state(&sim, mill.id).expect("the panel reads the mill");
        assert_eq!(state.kind, HouseKind::Mill);
        assert_eq!(state.ticks_total, 30, "the mill takes thirty ticks");
        assert_eq!(state.ticks_left, 30, "it is still building");
        assert_eq!(state.waits_for, Some(Ware::Grain), "the mill eats grain");
        assert_eq!(waits_for(HouseKind::Bakery), Some(Ware::Flour));
        assert_eq!(waits_for(HouseKind::Farm), None, "a farm waits for nothing");
    }

    #[test]
    fn a_finished_house_has_no_ticks_left() {
        let sim = sim();
        let warehouse = sim.houses().first().expect("the village has a warehouse");
        let state = house_state(&sim, warehouse.id).expect("the panel reads it");
        assert_eq!(state.ticks_total, 0, "a warehouse stands at once");
        assert_eq!(state.ticks_left, 0);
        assert_eq!(state.waits_for, None, "a warehouse waits for nothing");
    }

    #[test]
    fn a_picked_unit_reports_its_health() {
        let sim = sim();
        let unit = sim.units().first().expect("the village fields soldiers");
        let state = unit_state(&sim, unit.id).expect("the panel reads the soldier");
        assert_eq!(state.side, unit.side);
        assert_eq!(state.max_hp, unit.kind.hp());
        assert_eq!(state.hp, unit.hp);
        assert!(!state.moving, "a standing soldier walks nowhere");
    }

    #[test]
    fn an_order_is_the_sim_s_own_command() {
        let sim = sim();
        let swordsman = select_at(&sim, 3, 9).expect("a swordsman stands");
        assert_eq!(
            command(&sim, Some(swordsman), Order::AttackMove, 7, 9),
            Command::MoveUnit { id: 0, x: 7, y: 9 }
        );
        assert_eq!(
            command(&sim, Some(swordsman), Order::Stop, 0, 0),
            Command::MoveUnit { id: 0, x: 3, y: 9 },
            "stop is a move onto the unit's own tile"
        );
        assert_eq!(
            command(&sim, Some(swordsman), Order::Hold, 11, 9),
            Command::Attack { id: 0, target: 2 },
            "hold strikes the unit under the cursor"
        );
    }

    #[test]
    fn nothing_to_order_asks_the_sim_not_the_page() {
        let sim = sim();
        let command = command(&sim, None, Order::AttackMove, 7, 9);
        assert_eq!(
            command,
            Command::MoveUnit {
                id: NO_UNIT,
                x: 7,
                y: 9
            },
            "the page sends the id the sim does not know"
        );
        let mut live = sim.clone();
        assert_eq!(
            live.apply_for(0, command).err(),
            Some(Refusal::NoUnit),
            "the sim refuses in its own words"
        );
    }

    #[test]
    fn a_house_takes_no_order() {
        let sim = sim();
        let warehouse = select_at(&sim, 0, 3).expect("the warehouse stands");
        let command = command(&sim, Some(warehouse), Order::Stop, 0, 0);
        assert_eq!(
            command,
            Command::MoveUnit {
                id: NO_UNIT,
                x: 0,
                y: 0
            },
            "a house is not a unit, so the sim hears no unit"
        );
    }

    #[test]
    fn the_hold_target_is_the_sim_s_own_lookup() {
        let sim = sim();
        let swordsman = select_at(&sim, 3, 9).expect("a swordsman stands");
        let command = command(&sim, Some(swordsman), Order::Hold, 12, 11);
        assert_eq!(
            command,
            Command::Attack {
                id: 0,
                target: NO_UNIT
            },
            "no unit under the cursor is no target"
        );
    }

    #[test]
    fn the_json_names_the_kind_the_side_and_the_numbers() {
        let sim = sim();
        let json = state_json(&sim, select_at(&sim, 3, 9));
        assert!(json.contains("\"what\":\"unit\""), "{json}");
        assert!(json.contains("\"name\":\"swordsman\""), "{json}");
        assert!(json.contains("\"side\":0"), "{json}");
        assert!(json.contains("\"maxHp\":"), "{json}");
        let house = state_json(&sim, select_at(&sim, 0, 3));
        assert!(house.contains("\"what\":\"house\""), "{house}");
        assert!(house.contains("\"name\":\"warehouse\""), "{house}");
        assert!(house.contains("\"ticksLeft\":"), "{house}");
        assert_eq!(state_json(&sim, None), "", "nothing picked is no json");
    }

    #[test]
    fn the_selection_is_a_json_list_of_ids() {
        let picks = [Selection::Unit(0), Selection::House(2)];
        assert_eq!(selection_json(&picks), "[\"u0\",\"h2\"]");
        assert_eq!(selection_json(&[]), "[]");
    }

    #[test]
    fn the_names_of_the_orders_are_the_ones_the_page_sends() {
        assert_eq!(Order::parse("attack-move"), Some(Order::AttackMove));
        assert_eq!(Order::parse("stop"), Some(Order::Stop));
        assert_eq!(Order::parse("hold"), Some(Order::Hold));
        assert_eq!(Order::parse("dance"), None);
        assert_eq!(Order::AttackMove.name(), "attack-move");
    }
}
