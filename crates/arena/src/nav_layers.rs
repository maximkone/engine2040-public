//! nav_layers: the match floor as a layered navmesh (UW12/UW40 consumer).
//!
//! Consumer: [`LiveMatch`](super::live::LiveMatch) asks the table which
//! walkable layer sits under a fighter after every stepped tick, so the
//! layered navmesh that `engine-nav` baked (`engine_nav::layers`, several
//! walkable spans per column) has one caller in the sim lane instead of
//! sitting unread. Data owner: [`NavLayers`] owns the layered table; the
//! arena owns the floor geometry it bakes from.
//! Units: meters in and out, integer millimeters inside the table.
//! Determinism: the bake is a pure function of the floor list and the bytes
//! are identical twice; no answer here ever enters the tick hash, so the
//! golden stays where it is.
//! Errors: [`NavLayerError`], never a panic. A point off the mesh answers
//! `None`.
//! Budget: bake once per match, one column lookup per fighter per tick.

use engine_nav::layers::{LayerConfig, LayeredNav, Span};

/// One walkable floor: a rectangle at one height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Floor {
    /// Low X of the rectangle, meters.
    pub min_x: f32,
    /// High X of the rectangle, meters.
    pub max_x: f32,
    /// Low Z of the rectangle, meters.
    pub min_z: f32,
    /// High Z of the rectangle, meters.
    pub max_z: f32,
    /// Floor height, meters.
    pub y: f32,
}

/// Why a floor list refused to bake.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavLayerError {
    /// A floor rectangle is empty or inverted.
    BadFloor,
    /// The nav bake refused: the floors had no walkable surface.
    BakeRefused,
}

/// The answer for a point with no walkable surface under it.
pub const NO_NAV_LAYER: u32 = u32::MAX;

/// The match floor as a layered navmesh: one walkable span per floor per
/// column, so a four-floor building keeps four spans in one column instead
/// of averaging one height.
#[derive(Clone, Debug)]
pub struct NavLayers {
    nav: LayeredNav,
}

impl NavLayers {
    /// The arena's own sand: one rectangle at `y = 0` over the match square.
    /// `None` when the square is degenerate, which a valid config never is.
    #[must_use]
    pub fn arena_floor(half_extent_m: f32) -> Option<Self> {
        Self::from_floors(&[Floor {
            min_x: -half_extent_m,
            max_x: half_extent_m,
            min_z: -half_extent_m,
            max_z: half_extent_m,
            y: 0.0,
        }])
        .ok()
    }

    /// Bake a stack of floors into the layered table. Each floor becomes two
    /// up-facing triangles, so the bake keeps one span per floor per column.
    /// Refuses an empty or inverted rectangle and a bake with no walkable
    /// surface; never panics.
    pub fn from_floors(floors: &[Floor]) -> Result<Self, NavLayerError> {
        let mut tris: Vec<[[f32; 3]; 3]> = Vec::with_capacity(floors.len() * 2);
        for floor in floors {
            if !(floor.min_x < floor.max_x && floor.min_z < floor.max_z) {
                return Err(NavLayerError::BadFloor);
            }
            let (min_x, max_x, min_z, max_z, y) =
                (floor.min_x, floor.max_x, floor.min_z, floor.max_z, floor.y);
            // Counter-clockwise seen from above: the walkable test wants the
            // normal to point up.
            tris.push([[min_x, y, min_z], [max_x, y, max_z], [max_x, y, min_z]]);
            tris.push([[min_x, y, min_z], [min_x, y, max_z], [max_x, y, max_z]]);
        }
        let nav = LayeredNav::bake(&tris, &LayerConfig::default())
            .map_err(|_| NavLayerError::BakeRefused)?;
        Ok(Self { nav })
    }

    /// The connected walk surface under `point`: the span nearest the
    /// point's height in its column. `None` off the mesh or over a column
    /// with no walkable surface.
    #[must_use]
    pub fn layer_under(&self, point: [f32; 3]) -> Option<u32> {
        self.nav.span_at(point).ok().map(Span::layer)
    }

    /// The walkable height under `point`, meters. `None` off the mesh.
    #[must_use]
    pub fn floor_under(&self, point: [f32; 3]) -> Option<f32> {
        self.nav.span_at(point).ok().map(Span::top_m)
    }

    /// Every walkable span of the column holding `point`, ascending by
    /// height: the stacked floors of one column. Empty off the mesh.
    #[must_use]
    pub fn spans_under(&self, point: [f32; 3]) -> &[Span] {
        self.nav.spans_at(point).unwrap_or(&[])
    }

    /// Walkable spans over the whole table.
    #[must_use]
    pub fn span_count(&self) -> usize {
        self.nav.span_count()
    }

    /// Connected walk surfaces: four floors without a stair make four.
    #[must_use]
    pub fn layer_count(&self) -> u32 {
        self.nav.layer_count()
    }

    /// Digest of the canonical bytes: the same floors bake the same digest.
    #[must_use]
    pub fn digest(&self) -> u64 {
        self.nav.digest()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two floors over the same square: the column under the middle keeps
    /// both walkable spans, and each floor is its own connected surface.
    #[test]
    fn nav_layers_two_floors_keep_two_spans_in_one_column() {
        let floors = [
            Floor {
                min_x: -2.0,
                max_x: 2.0,
                min_z: -2.0,
                max_z: 2.0,
                y: 0.0,
            },
            Floor {
                min_x: -2.0,
                max_x: 2.0,
                min_z: -2.0,
                max_z: 2.0,
                y: 3.0,
            },
        ];
        let nav = NavLayers::from_floors(&floors).expect("two floors bake");
        assert_eq!(nav.layer_count(), 2, "two floors, no stair, two surfaces");
        let column = nav.spans_under([0.0, 0.0, 0.0]);
        assert_eq!(column.len(), 2, "one column keeps both spans");
        assert_eq!(nav.layer_under([0.0, 0.0, 0.0]), Some(0));
        assert_eq!(nav.floor_under([0.0, 0.0, 0.0]), Some(0.0));
        assert_eq!(
            nav.layer_under([0.0, 3.0, 0.0]),
            Some(1),
            "the upper floor is the other surface"
        );
        // The same floors bake the same bytes twice.
        let again = NavLayers::from_floors(&floors).expect("two floors bake");
        assert_eq!(nav.digest(), again.digest());
    }

    /// The arena floor is one walkable surface, and a point off the square
    /// has no floor under it.
    #[test]
    fn nav_layers_arena_floor_is_one_layer_and_answers_off_mesh_points() {
        let nav = NavLayers::arena_floor(10.0).expect("the arena floor bakes");
        assert_eq!(nav.layer_count(), 1);
        assert!(nav.span_count() > 0);
        assert_eq!(nav.layer_under([0.0, 0.0, 0.0]), Some(0));
        assert_eq!(nav.layer_under([100.0, 0.0, 0.0]), None);
        assert_eq!(nav.floor_under([100.0, 0.0, 0.0]), None);
    }

    /// An inverted rectangle refuses before the bake.
    #[test]
    fn nav_layers_refuse_an_inverted_floor() {
        let floors = [Floor {
            min_x: 2.0,
            max_x: -2.0,
            min_z: -2.0,
            max_z: 2.0,
            y: 0.0,
        }];
        assert_eq!(
            NavLayers::from_floors(&floors).err(),
            Some(NavLayerError::BadFloor)
        );
        assert_eq!(
            NavLayers::from_floors(&[]).err(),
            Some(NavLayerError::BakeRefused)
        );
    }
}
