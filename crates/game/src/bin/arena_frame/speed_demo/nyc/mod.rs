//! nyc (HC1-HC4, docs/SPRINT-34-hero-city.md): a procedural Manhattan for the speed demo's
//! second world. The grid (HC1) makes lots and roads, the building rows (HC2) turn a lot into
//! massing, the facades (HC3) give each style its tiling look and the streets (HC4) lay the
//! ground. These shared types are the seams between the rows. Presentation only, seeded.
//! Units: meters; x runs east, z runs south (avenues run along z, streets along x).

pub(super) mod building;
pub(super) mod facade;
pub(super) mod grid;
pub(super) mod streets;
pub(super) mod world;

/// A building's look family: picks its massing rules (HC2) and its facade tiles (HC3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Style {
    /// Walk-up brick: a box with a cornice, a water tank on the roof when short.
    Brick,
    /// Limestone and granite: setbacks above the street wall.
    Stone,
    /// Art Deco: deep setbacks, a crown, a spire when tall.
    Deco,
    /// Glass curtain wall: a straight shaft on a podium.
    Glass,
}

impl Style {
    /// Every style, in a fixed order (texture keys and chunk meshes follow it).
    pub(super) const ALL: [Style; 4] = [Style::Brick, Style::Stone, Style::Deco, Style::Glass];
}

/// One building plot: a footprint polygon on the ground and what stands on it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Lot {
    /// Footprint corners `[x, z]`, counter-clockwise seen from above; meters. Four for a grid
    /// lot, three or more for a wedge cut by the diagonal.
    pub footprint: Vec<[f32; 2]>,
    /// Roof height above the ground, meters.
    pub height_m: f32,
    /// Look family.
    pub style: Style,
    /// Per-lot seed for the massing and facade jitter.
    pub seed: u64,
}

/// What a road rectangle carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RoadKind {
    /// North-south, 30 m.
    Avenue,
    /// East-west, 18 m.
    Street,
    /// The diagonal (its rect list approximates the band).
    Broadway,
}

/// One axis-aligned road rectangle `[x0, z0]..[x1, z1]`; meters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct RoadRect {
    pub min: [f32; 2],
    pub max: [f32; 2],
    pub kind: RoadKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The seams hold plain data: every style is listed once and a lot or a road is a value.
    #[test]
    fn the_shared_types_are_plain_values() {
        for (i, a) in Style::ALL.iter().enumerate() {
            assert_eq!(Style::ALL.iter().position(|b| b == a), Some(i));
        }
        let lot = Lot {
            footprint: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 20.0], [0.0, 20.0]],
            height_m: 30.0,
            style: Style::Brick,
            seed: 1,
        };
        assert_eq!(lot.clone(), lot);
        let road = RoadRect {
            min: [0.0, 0.0],
            max: [30.0, 80.0],
            kind: RoadKind::Avenue,
        };
        assert!(road.max[0] > road.min[0] && road.kind != RoadKind::Street);
        assert_ne!(RoadKind::Broadway, RoadKind::Avenue);
    }
}
