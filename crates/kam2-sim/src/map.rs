//! Terrain grid: what can be built, where roads run, who stands where.

use std::collections::VecDeque;

/// What a tile is made of.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Terrain {
    /// Open ground; the default.
    Grass,
    /// Open ground, slightly cheaper to read as a path.
    Dirt,
    /// Unbuildable rock.
    Rock,
    /// Unbuildable water.
    Water,
}

impl Terrain {
    /// True when a building or a road may sit on this tile.
    #[must_use]
    pub const fn buildable(self) -> bool {
        matches!(self, Terrain::Grass | Terrain::Dirt)
    }
}

/// A square grid of tiles, roads and occupants.
#[derive(Clone, Debug)]
pub struct Map {
    w: i32,
    h: i32,
    terrain: Vec<Terrain>,
    road: Vec<bool>,
    occupant: Vec<Option<u32>>,
}

impl Map {
    /// A grass map of `w` by `h` tiles.
    #[must_use]
    pub fn new(w: i32, h: i32) -> Self {
        let n = usize::try_from(w.max(0) * h.max(0)).unwrap_or(0);
        Self {
            w,
            h,
            terrain: vec![Terrain::Grass; n],
            road: vec![false; n],
            occupant: vec![None; n],
        }
    }

    /// Map width in tiles.
    #[must_use]
    pub const fn width(&self) -> i32 {
        self.w
    }

    /// Map height in tiles.
    #[must_use]
    pub const fn height(&self) -> i32 {
        self.h
    }

    /// Flat position of a tile, if the tile exists.
    pub(crate) fn index(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return None;
        }
        usize::try_from(y * self.w + x).ok()
    }

    /// Tile coordinates of a flat position, if it is on the map.
    pub(crate) fn coords(&self, i: usize) -> Option<(i32, i32)> {
        if i >= self.terrain.len() {
            return None;
        }
        let w = usize::try_from(self.w).ok()?;
        if w == 0 {
            return None;
        }
        let x = i32::try_from(i % w).ok()?;
        let y = i32::try_from(i / w).ok()?;
        Some((x, y))
    }

    /// True when a unit may stand here: open ground with no house on it.
    #[must_use]
    pub fn walkable(&self, x: i32, y: i32) -> bool {
        self.terrain(x, y).is_some_and(Terrain::buildable) && self.occupant(x, y).is_none()
    }

    /// Terrain at a tile, if the tile exists.
    #[must_use]
    pub fn terrain(&self, x: i32, y: i32) -> Option<Terrain> {
        self.index(x, y).map(|i| self.terrain[i])
    }

    /// Set terrain. Refuses tiles that hold an occupant or a road.
    pub fn set_terrain(&mut self, x: i32, y: i32, terrain: Terrain) -> bool {
        match self.index(x, y) {
            Some(i) if self.occupant[i].is_none() && !self.road[i] => {
                self.terrain[i] = terrain;
                true
            }
            _ => false,
        }
    }

    /// True when a road tile sits here.
    #[must_use]
    pub fn is_road(&self, x: i32, y: i32) -> bool {
        self.index(x, y).is_some_and(|i| self.road[i])
    }

    /// Lay one road tile. Refuses water, rock, occupants and existing roads.
    pub fn lay_road(&mut self, x: i32, y: i32) -> bool {
        match self.index(x, y) {
            Some(i)
                if !self.road[i] && self.occupant[i].is_none() && self.terrain[i].buildable() =>
            {
                self.road[i] = true;
                true
            }
            _ => false,
        }
    }

    /// House id standing on a tile, if any.
    #[must_use]
    pub fn occupant(&self, x: i32, y: i32) -> Option<u32> {
        self.index(x, y).and_then(|i| self.occupant[i])
    }

    /// True when a `w` by `h` house may stand with its top-left at `(x, y)`.
    #[must_use]
    pub fn can_place(&self, x: i32, y: i32, w: i32, h: i32) -> bool {
        if w <= 0 || h <= 0 {
            return false;
        }
        for dy in 0..h {
            for dx in 0..w {
                match self.index(x + dx, y + dy) {
                    Some(i)
                        if self.occupant[i].is_none()
                            && !self.road[i]
                            && self.terrain[i].buildable() => {}
                    _ => return false,
                }
            }
        }
        true
    }

    /// Mark a footprint with an owner id. The caller checks `can_place` first.
    pub fn place(&mut self, id: u32, x: i32, y: i32, w: i32, h: i32) {
        for dy in 0..h {
            for dx in 0..w {
                if let Some(i) = self.index(x + dx, y + dy) {
                    self.occupant[i] = Some(id);
                }
            }
        }
    }

    /// True when some tile on the footprint border is a road.
    #[must_use]
    pub fn touches_road(&self, x: i32, y: i32, w: i32, h: i32) -> bool {
        for dx in 0..w {
            if self.is_road(x + dx, y - 1) || self.is_road(x + dx, y + h) {
                return true;
            }
        }
        for dy in 0..h {
            if self.is_road(x - 1, y + dy) || self.is_road(x + w, y + dy) {
                return true;
            }
        }
        false
    }

    fn border_roads(&self, x: i32, y: i32, w: i32, h: i32) -> Vec<usize> {
        let mut out = Vec::new();
        for dx in 0..w {
            for (tx, ty) in [(x + dx, y - 1), (x + dx, y + h)] {
                if let Some(i) = self.index(tx, ty) {
                    if self.road[i] {
                        out.push(i);
                    }
                }
            }
        }
        for dy in 0..h {
            for (tx, ty) in [(x - 1, y + dy), (x + w, y + dy)] {
                if let Some(i) = self.index(tx, ty) {
                    if self.road[i] {
                        out.push(i);
                    }
                }
            }
        }
        out
    }

    /// Shortest road distance in tiles between two footprints, or `None` when the road
    /// graph does not join them.
    #[must_use]
    pub fn road_distance(
        &self,
        from: (i32, i32, i32, i32),
        to: (i32, i32, i32, i32),
    ) -> Option<u32> {
        let starts = self.border_roads(from.0, from.1, from.2, from.3);
        if starts.is_empty() {
            return None;
        }
        let goals: Vec<usize> = self.border_roads(to.0, to.1, to.2, to.3);
        if goals.is_empty() {
            return None;
        }
        let mut seen = vec![false; self.road.len()];
        let mut queue: VecDeque<(usize, u32)> = VecDeque::new();
        for s in starts {
            if !seen[s] {
                seen[s] = true;
                queue.push_back((s, 1));
            }
        }
        while let Some((i, d)) = queue.pop_front() {
            if goals.contains(&i) {
                return Some(d);
            }
            let x = i32::try_from(i % usize::try_from(self.w.max(1)).ok()?).ok()?;
            let y = i32::try_from(i / usize::try_from(self.w.max(1)).ok()?).ok()?;
            for (nx, ny) in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)] {
                if let Some(j) = self.index(nx, ny) {
                    if self.road[j] && !seen[j] {
                        seen[j] = true;
                        queue.push_back((j, d + 1));
                    }
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{Map, Terrain};

    #[test]
    fn water_refuses_roads_and_houses() {
        let mut map = Map::new(8, 8);
        assert!(map.set_terrain(2, 2, Terrain::Water));
        assert!(!map.lay_road(2, 2));
        assert!(!map.can_place(2, 2, 1, 1));
    }

    #[test]
    fn a_road_line_joins_two_footprints() {
        let mut map = Map::new(12, 12);
        for x in 0..10 {
            assert!(map.lay_road(x, 6));
        }
        assert!(map.can_place(0, 3, 3, 3));
        assert!(map.touches_road(0, 3, 3, 3));
        assert_eq!(map.road_distance((0, 3, 3, 3), (7, 4, 2, 2)), Some(6));
    }

    #[test]
    fn a_broken_road_has_no_distance() {
        let mut map = Map::new(12, 12);
        for x in 0..3 {
            assert!(map.lay_road(x, 6));
        }
        for x in 6..9 {
            assert!(map.lay_road(x, 6));
        }
        assert_eq!(map.road_distance((0, 3, 3, 3), (7, 4, 2, 2)), None);
    }
}
