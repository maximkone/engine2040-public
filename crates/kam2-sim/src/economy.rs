//! Wares and houses: the chain the player feeds.

/// How many ware kinds the stock knows.
pub const WARE_COUNT: usize = 5;

/// What a house makes or eats.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ware {
    /// Cut wood.
    Wood,
    /// Quarried stone.
    Stone,
    /// Threshed grain.
    Grain,
    /// Milled flour.
    Flour,
    /// Baked bread; the score of a working town.
    Bread,
}

impl Ware {
    /// Every ware, in stock order.
    pub const ALL: [Ware; WARE_COUNT] = [
        Ware::Wood,
        Ware::Stone,
        Ware::Grain,
        Ware::Flour,
        Ware::Bread,
    ];

    /// Position in a stock array.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Ware::Wood => 0,
            Ware::Stone => 1,
            Ware::Grain => 2,
            Ware::Flour => 3,
            Ware::Bread => 4,
        }
    }

    /// Short name for logs and manifests.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Ware::Wood => "wood",
            Ware::Stone => "stone",
            Ware::Grain => "grain",
            Ware::Flour => "flour",
            Ware::Bread => "bread",
        }
    }
}

/// A count per ware.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stock {
    amounts: [u32; WARE_COUNT],
}

impl Stock {
    /// How many of one ware.
    #[must_use]
    pub fn get(&self, ware: Ware) -> u32 {
        self.amounts[ware.index()]
    }

    /// Add a ware.
    pub fn add(&mut self, ware: Ware, n: u32) {
        let slot = &mut self.amounts[ware.index()];
        *slot = slot.saturating_add(n);
    }

    /// Remove a ware when there is enough. All or nothing.
    pub fn take(&mut self, ware: Ware, n: u32) -> bool {
        let slot = &mut self.amounts[ware.index()];
        if *slot < n {
            return false;
        }
        *slot -= n;
        true
    }
}

/// The houses the first milestone knows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HouseKind {
    /// Stores what the town makes.
    Warehouse,
    /// Makes grain from fields.
    Farm,
    /// Turns grain into flour.
    Mill,
    /// Turns flour into bread.
    Bakery,
    /// Cuts wood from the forest.
    Woodcutter,
}

impl HouseKind {
    /// Position in a stable list; used by the hash and by manifests.
    #[must_use]
    pub const fn index(self) -> u8 {
        match self {
            HouseKind::Warehouse => 0,
            HouseKind::Farm => 1,
            HouseKind::Mill => 2,
            HouseKind::Bakery => 3,
            HouseKind::Woodcutter => 4,
        }
    }

    /// What the house costs to start.
    #[must_use]
    pub const fn cost(self) -> &'static [(Ware, u32)] {
        match self {
            HouseKind::Warehouse => &[(Ware::Wood, 10), (Ware::Stone, 5)],
            HouseKind::Farm => &[(Ware::Wood, 4)],
            HouseKind::Mill => &[(Ware::Wood, 6)],
            HouseKind::Bakery => &[(Ware::Wood, 8)],
            HouseKind::Woodcutter => &[(Ware::Wood, 2)],
        }
    }

    /// Ticks from placement to the first working tick.
    #[must_use]
    pub const fn build_ticks(self) -> u32 {
        match self {
            HouseKind::Warehouse => 0,
            HouseKind::Farm | HouseKind::Mill | HouseKind::Bakery => 30,
            HouseKind::Woodcutter => 20,
        }
    }

    /// Footprint in tiles: width and height.
    #[must_use]
    pub const fn footprint(self) -> (i32, i32) {
        match self {
            HouseKind::Warehouse => (3, 3),
            HouseKind::Farm | HouseKind::Mill | HouseKind::Bakery | HouseKind::Woodcutter => (2, 2),
        }
    }

    /// Short name for logs and manifests.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            HouseKind::Warehouse => "warehouse",
            HouseKind::Farm => "farm",
            HouseKind::Mill => "mill",
            HouseKind::Bakery => "bakery",
            HouseKind::Woodcutter => "woodcutter",
        }
    }
}

/// One placed house.
#[derive(Clone, Debug)]
pub struct House {
    /// Stable id, handed out in placement order.
    pub id: u32,
    /// What it does.
    pub kind: HouseKind,
    /// Top-left tile.
    pub x: i32,
    /// Top-left tile.
    pub y: i32,
    /// Road distance to the nearest warehouse, once a road joins them.
    pub link: Option<u32>,
    /// First tick the house works.
    pub built_at: u32,
}

/// Ticks between farm harvests.
pub const FARM_PERIOD: u32 = 40;
/// Ticks between mill runs.
pub const MILL_PERIOD: u32 = 30;
/// Ticks between bakery runs.
pub const BAKERY_PERIOD: u32 = 25;
/// Ticks between woodcutter loads.
pub const WOODCUTTER_PERIOD: u32 = 25;

#[cfg(test)]
mod tests {
    use super::{Stock, Ware};

    #[test]
    fn stock_is_all_or_nothing() {
        let mut stock = Stock::default();
        stock.add(Ware::Grain, 2);
        assert!(!stock.take(Ware::Grain, 3));
        assert_eq!(stock.get(Ware::Grain), 2);
        assert!(stock.take(Ware::Grain, 2));
        assert_eq!(stock.get(Ware::Grain), 0);
    }
}
