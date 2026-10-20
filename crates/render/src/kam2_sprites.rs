//! Sprite layer of the fixed 2.5D view: eight baked facings, painter depth
//! order, independent turret aim and the team-colour mask tint.
//!
//! The ground projection stays in [`crate::kam2_iso`]; this module only
//! consumes its world axes (x east, y up, z south) and never changes a
//! camera number. Facings follow `docs/plan/kam2-style-lock.md`: index 0 is S
//! and the ring runs clockwise, S, SW, W, NW, N, NE, E, SE.

use std::f32::consts::{FRAC_1_SQRT_2, PI, TAU};

/// How many baked facings one mobile sprite carries.
pub const FACING_COUNT: usize = 8;

/// One of the eight baked facings, clockwise from south.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Facing {
    /// Toward the camera.
    S = 0,
    /// South-west.
    SW = 1,
    /// West.
    W = 2,
    /// North-west.
    NW = 3,
    /// Away from the camera.
    N = 4,
    /// North-east.
    NE = 5,
    /// East.
    E = 6,
    /// South-east.
    SE = 7,
}

impl Facing {
    /// Every facing in style-lock order, index 0 first.
    pub const ALL: [Facing; FACING_COUNT] = [
        Facing::S,
        Facing::SW,
        Facing::W,
        Facing::NW,
        Facing::N,
        Facing::NE,
        Facing::E,
        Facing::SE,
    ];

    /// Atlas column of this facing: its index in the lock order.
    pub fn index(self) -> u8 {
        self as u8
    }

    /// Facing for an atlas column; `None` outside `0..8`.
    pub fn from_index(index: u8) -> Option<Self> {
        Self::ALL.get(usize::from(index)).copied()
    }

    /// Unit ground direction, x east and z south.
    pub fn direction(self) -> [f32; 2] {
        match self {
            Facing::S => [0.0, 1.0],
            Facing::SW => [-FRAC_1_SQRT_2, FRAC_1_SQRT_2],
            Facing::W => [-1.0, 0.0],
            Facing::NW => [-FRAC_1_SQRT_2, -FRAC_1_SQRT_2],
            Facing::N => [0.0, -1.0],
            Facing::NE => [FRAC_1_SQRT_2, -FRAC_1_SQRT_2],
            Facing::E => [1.0, 0.0],
            Facing::SE => [FRAC_1_SQRT_2, FRAC_1_SQRT_2],
        }
    }

    /// Facing four steps around the ring, the one that looks back.
    pub fn opposite(self) -> Self {
        Self::ALL[(usize::from(self.index()) + FACING_COUNT / 2) % FACING_COUNT]
    }
}

/// Facing that a ground direction points at; `None` for a zero or
/// non-finite vector.
///
/// The nearest of the eight sectors wins, so a hull can use any motion
/// vector without snapping to an axis first.
pub fn facing_from_direction(direction: [f32; 2]) -> Option<Facing> {
    bearing_rad([0.0, 0.0], direction).map(Facing::from_bearing)
}

impl Facing {
    /// Facing at the nearest sector to a clockwise bearing from north.
    fn from_bearing(bearing: f32) -> Self {
        let step = TAU / FACING_COUNT as f32;
        let offset = (bearing - PI).rem_euclid(TAU);
        let slot = (offset / step + 0.5).floor();
        let index = (slot as usize) % FACING_COUNT;
        Self::ALL[index]
    }
}

/// Clockwise ground bearing from `from` to `to`, in `0..2*PI` radians with
/// north at zero; `None` when the points coincide or carry a non-finite
/// coordinate.
///
/// East is `PI/2`, south is `PI`, west is `3*PI/2`. This is the turret
/// angle: it comes from the target point alone, not from the hull facing.
pub fn bearing_rad(from: [f32; 2], to: [f32; 2]) -> Option<f32> {
    let delta = [to[0] - from[0], to[1] - from[1]];
    if !delta[0].is_finite() || !delta[1].is_finite() {
        return None;
    }
    if delta[0] == 0.0 && delta[1] == 0.0 {
        return None;
    }
    Some(delta[0].atan2(-delta[1]).rem_euclid(TAU))
}

/// Turret facing for a hull at `from` aiming at `to`; `None` when the
/// target sits exactly on the hull, so the caller keeps the last aim.
pub fn turret_facing(from: [f32; 2], to: [f32; 2]) -> Option<Facing> {
    bearing_rad(from, to).map(Facing::from_bearing)
}

/// One ground-anchored sprite in the fixed view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sprite {
    /// Ground position: x east, z south, in world units.
    pub ground: [f32; 2],
    /// Which of the eight baked facings to draw.
    pub facing: Facing,
}

impl Sprite {
    /// Sprite standing at `ground` and turned to `facing`.
    pub fn new(ground: [f32; 2], facing: Facing) -> Self {
        Self { ground, facing }
    }

    /// Screen nearness along the one fixed view axis.
    ///
    /// The camera sits toward +x and +z, so `x + z` grows toward the eye and
    /// a larger key must be drawn later, on top of a smaller one.
    pub fn depth_key(&self) -> f32 {
        self.ground[0] + self.ground[1]
    }
}

/// Draw order from farthest to nearest, as indices into `sprites`.
///
/// Painter's algorithm: the caller draws the returned order front to back
/// of the list. Equal keys keep their slice order, so a frame is stable.
pub fn depth_order(sprites: &[Sprite]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..sprites.len()).collect();
    order.sort_by(|a, b| sprites[*a].depth_key().total_cmp(&sprites[*b].depth_key()));
    order
}

/// Uniform atlas grid: one column per facing, one row per sprite set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpriteAtlas {
    /// Cell width in atlas pixels.
    pub cell_w: u32,
    /// Cell height in atlas pixels.
    pub cell_h: u32,
    /// Rows in the grid, one per sprite set (hull, turret, marker, ...).
    pub rows: u32,
}

impl SpriteAtlas {
    /// Atlas width in pixels.
    pub fn width(&self) -> u32 {
        self.cell_w * FACING_COUNT as u32
    }

    /// Atlas height in pixels.
    pub fn height(&self) -> u32 {
        self.cell_h * self.rows
    }

    /// Top-left atlas pixel of the cell for `row` and `facing`.
    pub fn cell_origin(&self, row: u32, facing: Facing) -> Option<[u32; 2]> {
        if row >= self.rows || self.cell_w == 0 || self.cell_h == 0 {
            return None;
        }
        Some([u32::from(facing.index()) * self.cell_w, row * self.cell_h])
    }

    /// Normalized UV origin and size of a cell; `None` outside the grid.
    pub fn cell_uv(&self, row: u32, facing: Facing) -> Option<[[f32; 2]; 2]> {
        let origin = self.cell_origin(row, facing)?;
        let width = self.width() as f32;
        let height = self.height() as f32;
        Some([
            [origin[0] as f32 / width, origin[1] as f32 / height],
            [self.cell_w as f32 / width, self.cell_h as f32 / height],
        ])
    }
}

/// Rec. 709 luma weights used by the tint; they sum to one, so luma is
/// preserved when no channel clips.
pub const LUMA_709: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// Luma of an sRGB byte colour in `0..=255`.
pub fn luminance(rgb: [u8; 3]) -> f32 {
    LUMA_709[0] * f32::from(rgb[0])
        + LUMA_709[1] * f32::from(rgb[1])
        + LUMA_709[2] * f32::from(rgb[2])
}

/// Team tint of one pixel: multiply the base by the team colour, then scale
/// the product back to the base luma so baked shading survives.
///
/// `mask` is the mask-atlas alpha: 0 leaves the base pixel alone, 255 takes
/// the full tint, values between blend at the edge of a team band. Channels
/// clamp at 255, which is the only case where luma cannot be held.
pub fn team_tint(base: [u8; 3], mask: u8, team: [u8; 3]) -> [u8; 3] {
    if mask == 0 {
        return base;
    }
    let product = [
        f32::from(base[0]) * f32::from(team[0]) / 255.0,
        f32::from(base[1]) * f32::from(team[1]) / 255.0,
        f32::from(base[2]) * f32::from(team[2]) / 255.0,
    ];
    let product_luma =
        LUMA_709[0] * product[0] + LUMA_709[1] * product[1] + LUMA_709[2] * product[2];
    let base_luma = luminance(base);
    let gain = if product_luma > f32::EPSILON {
        base_luma / product_luma
    } else {
        0.0
    };
    let coverage = f32::from(mask) / 255.0;
    let mut out = [0u8; 3];
    for (slot, channel) in out.iter_mut().enumerate() {
        let tinted = (product[slot] * gain).clamp(0.0, 255.0);
        let mixed = f32::from(base[slot]) + (tinted - f32::from(base[slot])) * coverage;
        *channel = mixed.round().clamp(0.0, 255.0) as u8;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-5;

    #[test]
    fn facings_follow_the_style_lock_order() {
        let expected = ["S", "SW", "W", "NW", "N", "NE", "E", "SE"];
        for (index, facing) in Facing::ALL.into_iter().enumerate() {
            assert_eq!(facing.index(), index as u8);
            assert_eq!(Facing::from_index(index as u8), Some(facing));
            assert_eq!(format!("{facing:?}"), expected[index]);
            assert_eq!(facing.opposite().index(), (facing.index() + 4) % 8);
        }
        assert_eq!(Facing::from_index(8), None);
    }

    #[test]
    fn facing_index_from_a_direction_vector() {
        let cases = [
            ([0.0, 5.0], Facing::S),
            ([-1.0, 1.0], Facing::SW),
            ([-5.0, 0.0], Facing::W),
            ([-1.0, -1.0], Facing::NW),
            ([0.0, -5.0], Facing::N),
            ([1.0, -1.0], Facing::NE),
            ([5.0, 0.0], Facing::E),
            ([1.0, 1.0], Facing::SE),
            ([2.0, 1.0], Facing::SE),
        ];
        for (direction, facing) in cases {
            assert_eq!(facing_from_direction(direction), Some(facing));
        }
        for facing in Facing::ALL {
            assert_eq!(
                facing_from_direction(facing.direction()),
                Some(facing),
                "{facing:?} does not round-trip"
            );
        }
        assert_eq!(facing_from_direction([0.0, 0.0]), None);
        assert_eq!(facing_from_direction([f32::NAN, 1.0]), None);
        assert_eq!(facing_from_direction([f32::INFINITY, 1.0]), None);
    }

    #[test]
    fn facing_sectors_break_on_the_diagonal() {
        let at = |degrees: f32| {
            let radians = degrees * PI / 180.0;
            [radians.sin(), -radians.cos()]
        };
        assert_eq!(facing_from_direction(at(202.4)), Some(Facing::S));
        assert_eq!(facing_from_direction(at(202.6)), Some(Facing::SW));
        assert_eq!(facing_from_direction(at(157.4)), Some(Facing::SE));
        assert_eq!(facing_from_direction(at(157.6)), Some(Facing::S));
    }

    #[test]
    fn depth_sort_draws_far_before_near() {
        let sprites = [
            Sprite::new([0.0, 0.0], Facing::S),
            Sprite::new([2.0, 1.0], Facing::E),
            Sprite::new([-2.0, 0.5], Facing::N),
            Sprite::new([1.0, 2.0], Facing::W),
        ];
        assert_eq!(depth_order(&sprites), vec![2, 0, 1, 3]);
        for pair in depth_order(&sprites).windows(2) {
            assert!(
                sprites[pair[0]].depth_key() <= sprites[pair[1]].depth_key(),
                "near sprite drawn first"
            );
        }
        assert_eq!(depth_order(&sprites)[3], 3);
        assert_eq!(sprites[3].depth_key(), 3.0);
        assert_eq!(depth_order(&[]), Vec::<usize>::new());
    }

    #[test]
    fn turret_angle_is_a_clockwise_bearing() {
        let hull = [0.0, 0.0];
        let quarter = PI / 2.0;
        assert!((bearing_rad(hull, [0.0, -1.0]).expect("north") - 0.0).abs() < EPS);
        assert!((bearing_rad(hull, [1.0, 0.0]).expect("east") - quarter).abs() < EPS);
        assert!((bearing_rad(hull, [0.0, 1.0]).expect("south") - PI).abs() < EPS);
        assert!((bearing_rad(hull, [-1.0, 0.0]).expect("west") - 3.0 * quarter).abs() < EPS);
        assert!((bearing_rad(hull, [1.0, 1.0]).expect("south-east") - 0.75 * PI).abs() < EPS);
        assert_eq!(bearing_rad(hull, hull), None);
        assert_eq!(turret_facing(hull, hull), None);
        for bearing in [0.0, 0.3, quarter, 2.2, 3.1, 4.7, 5.9, TAU - 0.01] {
            let direction = [bearing.sin(), -bearing.cos()];
            let angle = bearing_rad(hull, direction).expect("non-zero target");
            assert!((angle - bearing).abs() < 1e-4, "bearing {bearing}");
        }
    }

    #[test]
    fn turret_facing_ignores_the_hull_facing() {
        let hull = Sprite::new([2.0, 3.0], Facing::S);
        let target = [hull.ground[0] + 4.0, hull.ground[1] + 0.0];
        assert_eq!(turret_facing(hull.ground, target), Some(Facing::E));
        assert_eq!(hull.facing, Facing::S);
        let target = [hull.ground[0], hull.ground[1] - 2.0];
        assert_eq!(turret_facing(hull.ground, target), Some(Facing::N));
    }

    #[test]
    fn team_tint_keeps_luminance_on_known_pixels() {
        let blue = [0x2B, 0x6F, 0xD6];
        let red = [0xC6, 0x37, 0x2E];
        let grey = [96u8, 96, 96];
        let tinted = team_tint(grey, 255, blue);
        assert_eq!(tinted, [40, 102, 198]);
        assert!((luminance(tinted) - luminance(grey)).abs() < 1.0);

        let red_tinted = team_tint([90, 90, 90], 255, red);
        assert_eq!(red_tinted, [210, 58, 49]);
        assert!((luminance(red_tinted) - 90.0).abs() < 1.0);

        assert_eq!(team_tint(grey, 255, grey), grey);
        assert_eq!(team_tint(grey, 0, blue), grey);
        assert_eq!(team_tint(grey, 0, red), grey);
        assert_eq!(team_tint([1, 2, 3], 0, [255, 0, 255]), [1, 2, 3]);
    }

    #[test]
    fn team_tint_blends_at_half_mask() {
        let blue = [0x2B, 0x6F, 0xD6];
        assert_eq!(team_tint([96, 96, 96], 128, blue), [68, 99, 147]);
        assert_eq!(team_tint([96, 96, 96], 255, blue), [40, 102, 198]);
        assert_eq!(team_tint([96, 96, 96], 64, blue), [82, 98, 121]);
    }

    #[test]
    fn atlas_cells_are_one_per_facing_and_row() {
        let atlas = SpriteAtlas {
            cell_w: 48,
            cell_h: 40,
            rows: 4,
        };
        assert_eq!(atlas.width(), 384);
        assert_eq!(atlas.height(), 160);
        assert_eq!(atlas.cell_origin(0, Facing::S), Some([0, 0]));
        assert_eq!(atlas.cell_origin(2, Facing::SE), Some([336, 80]));
        assert_eq!(atlas.cell_origin(4, Facing::S), None);
        let [origin, size] = atlas.cell_uv(1, Facing::E).expect("inside the grid");
        assert!((origin[0] - 0.75).abs() < EPS);
        assert!((origin[1] - 0.25).abs() < EPS);
        assert!((size[0] - 0.125).abs() < EPS);
        assert!((size[1] - 0.25).abs() < EPS);
        assert_eq!(
            atlas.cell_uv(0, Facing::S),
            Some([[0.0, 0.0], [0.125, 0.25]])
        );
        let empty = SpriteAtlas {
            cell_w: 0,
            cell_h: 40,
            rows: 1,
        };
        assert_eq!(empty.cell_uv(0, Facing::S), None);
    }
}
