//! stance: stand, crouch and prone capsules, ceiling-blocked changes, lean checked against walls,
//! and the weapon raised when its length meets geometry (UW9).
//!
//! Consumer: none yet. The headless composer (UW26) keeps one [`Body`] per fighter, hands it the
//! pose the character controller ([`crate::controller2`]) resolved plus the fighter's request each
//! tick, and reads back the stance, lean and weapon pose to feed combat and presentation. Data
//! owner: this module owns the three capsules, the lean throw and the weapon rule; the walls are
//! the caller's `Block` slice. Units: integer millimeters, facing as a Q10 unit vector (1024 is
//! one). Determinism: `+ - * /` on integers only (`i128` for the segment test), blocks visited in
//! slice order, no float, no clock; the same world, pose and requests give the same body and the
//! same `Body::hash`. Errors: none; a request the room refuses is answered with the nearest
//! pose that fits and a `Report` saying what was refused. Budget: one pass over the blocks per
//! check, a handful of checks per tick, no allocation.
//!
//! Each stance is one vertical capsule, so "clipping" means that capsule overlapping a block's
//! volume. Touching a face is not clipping.
#![forbid(unsafe_code)]

use crate::controller2::Block;
use engine_core::Fnv64;

/// Sideways throw of a full lean, millimeters.
pub const LEAN_MM: i32 = 300;
/// How far ahead of the body axis the weapon hand sits, millimeters.
pub const HAND_REACH_MM: i32 = 250;
/// The Q10 unit: a facing component of 1024 is one.
pub const Q10: i32 = 1024;
/// A raised weapon points 45 degrees up, in per mille of its length per axis.
const RAISE_PM: i32 = 707;

/// How the body stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stance {
    /// Upright.
    Stand,
    /// Bent at the knees.
    Crouch,
    /// Belly down.
    Prone,
}

impl Stance {
    /// Capsule height, millimeters.
    pub const fn height_mm(self) -> i32 {
        match self {
            Self::Stand => 1_700,
            Self::Crouch => 1_100,
            Self::Prone => 500,
        }
    }

    /// Capsule radius, millimeters.
    pub const fn radius_mm(self) -> i32 {
        match self {
            Self::Stand | Self::Crouch => 350,
            Self::Prone => 250,
        }
    }

    /// Height of the weapon hand above the feet, millimeters.
    pub const fn hand_mm(self) -> i32 {
        match self {
            Self::Stand => 1_300,
            Self::Crouch => 800,
            Self::Prone => 300,
        }
    }

    /// The next lower stance, if any.
    const fn lower(self) -> Option<Self> {
        match self {
            Self::Stand => Some(Self::Crouch),
            Self::Crouch => Some(Self::Prone),
            Self::Prone => None,
        }
    }
}

/// Which way the body leans round a corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Lean {
    /// Upright over the feet.
    Center,
    /// Out to the left of the facing.
    Left,
    /// Out to the right of the facing.
    Right,
}

/// Where the weapon points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weapon {
    /// Level, along the facing.
    Level,
    /// Tipped up 45 degrees because the level weapon would run into the room.
    Raised,
}

/// What a fighter asks for this tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    /// Wanted stance.
    pub stance: Stance,
    /// Wanted lean.
    pub lean: Lean,
}

/// What the room refused this tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// The wanted stance did not fit under the ceiling or between the blocks.
    pub ceiling: bool,
    /// The wanted lean would put the body into a wall.
    pub wall: bool,
}

/// One fighter's posture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Body {
    stance: Stance,
    lean: Lean,
    weapon: Weapon,
    pos_mm: [i32; 3],
    facing_q10: [i32; 2],
    weapon_len_mm: i32,
}

/// Distance from `v` to the closed interval `lo..=hi`, zero inside.
fn gap(lo: i32, hi: i32, v: i32) -> i64 {
    i64::from((lo - v).max(v - hi).max(0))
}

/// True when the capsule of `stance` standing at `feet_mm` shifted sideways by `shift` overlaps
/// the volume of `block`. A vertical capsule against a box is exact per axis: the axis segment
/// and the box are both products of intervals, so the squared distance is the sum of the
/// squared interval gaps.
fn overlaps(block: &Block, feet_mm: [i32; 3], shift: [i32; 2], stance: Stance) -> bool {
    let r = stance.radius_mm();
    let (cx, cz) = (feet_mm[0] + shift[0], feet_mm[2] + shift[1]);
    let (y0, y1) = (feet_mm[1] + r, feet_mm[1] + stance.height_mm() - r);
    let dy = i64::from((block.min_mm[1] - y1).max(y0 - block.max_mm[1]).max(0));
    let dx = gap(block.min_mm[0], block.max_mm[0], cx);
    let dz = gap(block.min_mm[2], block.max_mm[2], cz);
    dx * dx + dy * dy + dz * dz < i64::from(r) * i64::from(r)
}

/// True when the segment `a..b` runs through the inside of `block` for some length. Grazing a
/// face, an edge or a corner does not count.
fn segment_hits(a: [i32; 3], b: [i32; 3], block: &Block) -> bool {
    // The part of the segment inside the box is `lo..hi` of its length, kept as fractions
    // `(numerator, denominator)` with a positive denominator and compared by cross-multiplying.
    let (mut lo, mut hi) = ((0i128, 1i128), (1i128, 1i128));
    for axis in 0..3 {
        let d = i128::from(b[axis] - a[axis]);
        let (p, min, max) = (
            i128::from(a[axis]),
            i128::from(block.min_mm[axis]),
            i128::from(block.max_mm[axis]),
        );
        if d == 0 {
            if p <= min || p >= max {
                return false;
            }
            continue;
        }
        let (t_in, t_out) = if d > 0 {
            ((min - p, d), (max - p, d))
        } else {
            ((p - max, -d), (p - min, -d))
        };
        if t_in.0 * lo.1 > lo.0 * t_in.1 {
            lo = t_in;
        }
        if t_out.0 * hi.1 < hi.0 * t_out.1 {
            hi = t_out;
        }
    }
    lo.0 * hi.1 < hi.0 * lo.1
}

impl Body {
    /// A standing, upright body with its weapon level, `weapon_len_mm` long, at `pos_mm` (feet)
    /// facing `facing_q10`.
    pub const fn new(pos_mm: [i32; 3], facing_q10: [i32; 2], weapon_len_mm: i32) -> Self {
        Self {
            stance: Stance::Stand,
            lean: Lean::Center,
            weapon: Weapon::Level,
            pos_mm,
            facing_q10,
            weapon_len_mm,
        }
    }

    /// Current stance.
    pub const fn stance(&self) -> Stance {
        self.stance
    }

    /// Current lean.
    pub const fn lean(&self) -> Lean {
        self.lean
    }

    /// Current weapon pose.
    pub const fn weapon(&self) -> Weapon {
        self.weapon
    }

    /// Sideways shift of the capsule for a lean, `[x, z]` millimeters. The right-hand side of a
    /// facing `(fx, fz)` is `(fz, -fx)`.
    pub fn lean_shift_mm(&self, lean: Lean) -> [i32; 2] {
        let side = match lean {
            Lean::Center => 0,
            Lean::Left => -1,
            Lean::Right => 1,
        };
        let [fx, fz] = self.facing_q10;
        [side * fz * LEAN_MM / Q10, -side * fx * LEAN_MM / Q10]
    }

    /// True when the capsule of `stance` with `lean` clears every block.
    fn fits(&self, world: &[Block], stance: Stance, lean: Lean) -> bool {
        let shift = self.lean_shift_mm(lean);
        !world
            .iter()
            .any(|b| overlaps(b, self.pos_mm, shift, stance))
    }

    /// The weapon segment for `pose`: from the hand to the muzzle.
    pub fn weapon_segment(&self, pose: Weapon) -> ([i32; 3], [i32; 3]) {
        let shift = self.lean_shift_mm(self.lean);
        let [fx, fz] = self.facing_q10;
        let hand = [
            self.pos_mm[0] + shift[0] + fx * HAND_REACH_MM / Q10,
            self.pos_mm[1] + self.stance.hand_mm(),
            self.pos_mm[2] + shift[1] + fz * HAND_REACH_MM / Q10,
        ];
        let len = self.weapon_len_mm;
        let muzzle = match pose {
            Weapon::Level => [hand[0] + fx * len / Q10, hand[1], hand[2] + fz * len / Q10],
            Weapon::Raised => {
                let (along, up) = (len * RAISE_PM / 1_000, len * RAISE_PM / 1_000);
                [
                    hand[0] + fx * along / Q10,
                    hand[1] + up,
                    hand[2] + fz * along / Q10,
                ]
            }
        };
        (hand, muzzle)
    }

    /// Moves the body to `pos_mm` facing `facing_q10` and answers `want` as far as the room
    /// allows. A stance that no longer fits at the new place drops to the next lower one that
    /// does; a wanted stance that does not fit is refused; a lean into a wall falls back to
    /// upright; the weapon rises when level it would run into a block. Prone is the floor of the
    /// ladder: a place where even prone clips is the controller's fault, not this body's.
    pub fn update(
        &mut self,
        world: &[Block],
        pos_mm: [i32; 3],
        facing_q10: [i32; 2],
        want: Request,
    ) -> Report {
        self.pos_mm = pos_mm;
        self.facing_q10 = facing_q10;
        let mut report = Report::default();
        // Stance first, checked upright so a held lean cannot veto a change of height.
        let mut stance = want.stance;
        while !self.fits(world, stance, Lean::Center) {
            let Some(lower) = stance.lower() else {
                break;
            };
            report.ceiling = true;
            stance = lower;
        }
        self.stance = stance;
        self.lean = if self.fits(world, stance, want.lean) {
            want.lean
        } else {
            report.wall = true;
            Lean::Center
        };
        let (hand, muzzle) = self.weapon_segment(Weapon::Level);
        self.weapon = if world.iter().any(|b| segment_hits(hand, muzzle, b)) {
            Weapon::Raised
        } else {
            Weapon::Level
        };
        report
    }

    /// FNV-1a over the whole posture, for tapes and checkpoints.
    pub fn hash(&self) -> u64 {
        let mut h = Fnv64::new();
        h.write_u8(match self.stance {
            Stance::Stand => 0,
            Stance::Crouch => 1,
            Stance::Prone => 2,
        });
        h.write_u8(match self.lean {
            Lean::Center => 0,
            Lean::Left => 1,
            Lean::Right => 2,
        });
        h.write_u8(match self.weapon {
            Weapon::Level => 0,
            Weapon::Raised => 1,
        });
        for v in self.pos_mm {
            h.write_i32(v);
        }
        for v in self.facing_q10 {
            h.write_i32(v);
        }
        h.write_i32(self.weapon_len_mm);
        h.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::Rng;

    const AHEAD: [i32; 2] = [Q10, 0];

    fn body() -> Body {
        Body::new([0, 0, 0], AHEAD, 900)
    }

    fn want(stance: Stance, lean: Lean) -> Request {
        Request { stance, lean }
    }

    /// A beam across the way: its underside at 1200 mm blocks standing but not crouching.
    fn beam() -> Block {
        Block::new([-2_000, 1_200, -2_000], [2_000, 1_500, 2_000])
    }

    #[test]
    fn a_low_ceiling_blocks_standing_but_not_crouching() {
        let (mut b, world) = (body(), [beam()]);
        let report = b.update(&world, [0, 0, 0], AHEAD, want(Stance::Stand, Lean::Center));
        assert_eq!((b.stance(), report.ceiling), (Stance::Crouch, true));
        let report = b.update(&world, [0, 0, 0], AHEAD, want(Stance::Crouch, Lean::Center));
        assert_eq!((b.stance(), report.ceiling), (Stance::Crouch, false));
    }

    #[test]
    fn standing_up_waits_until_the_ceiling_is_gone() {
        let world = [Block::new([-300, 1_200, -300], [300, 1_500, 300])];
        let mut b = body();
        b.update(&world, [0, 0, 0], AHEAD, want(Stance::Crouch, Lean::Center));
        let held = b.update(&world, [0, 0, 0], AHEAD, want(Stance::Stand, Lean::Center));
        assert_eq!((b.stance(), held.ceiling), (Stance::Crouch, true));
        // One step clear of the beam, the same request is granted.
        let freed = b.update(
            &world,
            [900, 0, 0],
            AHEAD,
            want(Stance::Stand, Lean::Center),
        );
        assert_eq!((b.stance(), freed.ceiling), (Stance::Stand, false));
    }

    #[test]
    fn a_walker_carried_under_a_beam_is_pressed_down() {
        let world = [Block::new([-300, 1_000, -300], [300, 1_500, 300])];
        let mut b = body();
        b.update(
            &world,
            [2_000, 0, 0],
            AHEAD,
            want(Stance::Stand, Lean::Center),
        );
        assert_eq!(b.stance(), Stance::Stand);
        b.update(&world, [0, 0, 0], AHEAD, want(Stance::Stand, Lean::Center));
        assert_eq!(b.stance(), Stance::Prone, "1000 mm of room is prone only");
    }

    #[test]
    fn a_lean_needs_room_to_the_side() {
        // Facing +x, the right hand is -z. A wall 500 mm off the axis on that side leaves a
        // lean of 300 mm plus the 350 mm body short by 150 mm.
        let world = [Block::new([-1_000, 0, -1_500], [1_000, 2_500, -500])];
        let mut b = body();
        let report = b.update(&world, [0, 0, 0], AHEAD, want(Stance::Stand, Lean::Right));
        assert_eq!((b.lean(), report.wall), (Lean::Center, true));
        let report = b.update(&world, [0, 0, 0], AHEAD, want(Stance::Stand, Lean::Left));
        assert_eq!((b.lean(), report.wall), (Lean::Left, false));
        // Two steps off the wall the right lean fits.
        let report = b.update(&world, [0, 0, 400], AHEAD, want(Stance::Stand, Lean::Right));
        assert_eq!((b.lean(), report.wall), (Lean::Right, false));
    }

    #[test]
    fn the_weapon_rises_when_its_length_meets_a_wall() {
        // The hand is 250 mm ahead of the body, so a wall 1000 mm ahead is met by a 900 mm
        // weapon (it reaches 1150 mm) and missed by a 500 mm one.
        let world = [Block::new([1_000, 0, -2_000], [1_200, 2_500, 2_000])];
        let mut long = Body::new([0, 0, 0], AHEAD, 900);
        long.update(&world, [0, 0, 0], AHEAD, want(Stance::Stand, Lean::Center));
        assert_eq!(long.weapon(), Weapon::Raised);
        let mut short = Body::new([0, 0, 0], AHEAD, 500);
        short.update(&world, [0, 0, 0], AHEAD, want(Stance::Stand, Lean::Center));
        assert_eq!(short.weapon(), Weapon::Level);
        // Backing off lowers it again.
        long.update(
            &world,
            [-600, 0, 0],
            AHEAD,
            want(Stance::Stand, Lean::Center),
        );
        assert_eq!(long.weapon(), Weapon::Level);
    }

    #[test]
    fn grazing_a_face_is_neither_clipping_nor_a_weapon_hit() {
        // Body 350 mm from a wall face is touching it; a weapon along the face grazes it.
        let world = [Block::new([-2_000, 0, 350], [2_000, 2_500, 600])];
        let mut b = body();
        let report = b.update(&world, [0, 0, 0], AHEAD, want(Stance::Stand, Lean::Center));
        assert_eq!(report, Report::default());
        assert_eq!(b.weapon(), Weapon::Level);
        let (hand, muzzle) = b.weapon_segment(Weapon::Level);
        assert!(!segment_hits(hand, muzzle, &world[0]));
        // The same weapon one millimeter into the wall does hit it.
        assert!(segment_hits(
            [hand[0], hand[1], 351],
            [muzzle[0], muzzle[1], 351],
            &world[0]
        ));
    }

    #[test]
    fn segment_hits_covers_every_axis_and_direction() {
        let b = Block::new([0, 0, 0], [100, 100, 100]);
        for (from, to, hit) in [
            ([-50, 50, 50], [150, 50, 50], true),
            ([150, 50, 50], [-50, 50, 50], true),
            ([50, -50, 50], [50, 150, 50], true),
            ([50, 50, 150], [50, 50, -50], true),
            ([-50, -50, -50], [150, 150, 150], true),
            ([-50, 50, 50], [-1, 50, 50], false),
            ([-50, 50, 50], [0, 50, 50], false),
            ([-50, 200, 50], [150, 200, 50], false),
            ([-50, 0, 50], [150, 0, 50], false),
            ([50, 50, 50], [50, 50, 50], true), // a point inside
            ([150, 50, 50], [150, 50, 50], false),
        ] {
            assert_eq!(segment_hits(from, to, &b), hit, "{from:?} -> {to:?}");
        }
    }

    // ---- 1,000 seeded rooms, checked by an oracle that shares no code with the module ----

    /// Integer square root, for the oracle only.
    fn isqrt(n: i64) -> i64 {
        let mut x = (n as f64).sqrt() as i64;
        while x * x > n {
            x -= 1;
        }
        while (x + 1) * (x + 1) <= n {
            x += 1;
        }
        x
    }

    /// True when a sampled point of the capsule lies strictly inside `block`. The capsule is a
    /// cylinder with round caps; each height gets the radius the caps allow.
    fn oracle_capsule_inside(body: &Body, block: &Block) -> bool {
        let stance = body.stance();
        let r = i64::from(stance.radius_mm());
        let h = i64::from(stance.height_mm());
        let shift = body.lean_shift_mm(body.lean());
        let (cx, cz) = (
            i64::from(body.pos_mm[0] + shift[0]),
            i64::from(body.pos_mm[2] + shift[1]),
        );
        let inside = |p: [i64; 3]| {
            (0..3).all(|a| p[a] > i64::from(block.min_mm[a]) && p[a] < i64::from(block.max_mm[a]))
        };
        for dy in (0..=h).step_by(20) {
            // Radius at this height: full in the middle, a circle arc in the two caps.
            let up = (r - dy).max(dy - (h - r)).max(0);
            let ring = isqrt((r * r - up * up).max(0)) - 1;
            if ring < 0 {
                continue;
            }
            let y = i64::from(body.pos_mm[1]) + dy;
            // Eight spokes at 45 degrees (707/1000), plus the axis itself.
            let diag = ring * 707 / 1_000;
            for (ox, oz) in [
                (0, 0),
                (ring, 0),
                (-ring, 0),
                (0, ring),
                (0, -ring),
                (diag, diag),
                (-diag, diag),
                (diag, -diag),
                (-diag, -diag),
            ] {
                if inside([cx + ox, y, cz + oz]) {
                    return true;
                }
            }
        }
        false
    }

    /// True when a point sampled along the level weapon lies strictly inside a block.
    fn oracle_weapon_inside(body: &Body, block: &Block) -> bool {
        let (a, b) = body.weapon_segment(Weapon::Level);
        (0..=80).any(|i| {
            let p =
                |axis: usize| i64::from(a[axis]) + i64::from(b[axis] - a[axis]) * i64::from(i) / 80;
            (0..3).all(|ax| {
                p(ax) > i64::from(block.min_mm[ax]) && p(ax) < i64::from(block.max_mm[ax])
            })
        })
    }

    /// A draw in `lo..hi`.
    fn draw(rng: &mut Rng, lo: i32, hi: i32) -> i32 {
        let span = u64::try_from(hi - lo).expect("a non-empty range");
        lo + i32::try_from(rng.next_u64() % span).expect("fits")
    }

    fn room(rng: &mut Rng) -> Vec<Block> {
        // Four walls round an eight meter room, then furniture of four kinds.
        let mut world = vec![
            Block::new([-4_200, 0, -4_200], [-4_000, 2_600, 4_200]),
            Block::new([4_000, 0, -4_200], [4_200, 2_600, 4_200]),
            Block::new([-4_200, 0, -4_200], [4_200, 2_600, -4_000]),
            Block::new([-4_200, 0, 4_000], [4_200, 2_600, 4_200]),
        ];
        for _ in 0..draw(rng, 3, 9) {
            let (x, z) = (draw(rng, -3_500, 3_000), draw(rng, -3_500, 3_000));
            let (w, d) = (draw(rng, 300, 1_500), draw(rng, 300, 1_500));
            let (lo, hi) = match draw(rng, 0, 4) {
                0 => (0, 750),                                           // table
                1 => (0, 1_200),                                         // crate
                2 => (0, 2_000),                                         // shelf
                _ => (draw(rng, 1_100, 1_500), draw(rng, 1_800, 2_200)), // beam
            };
            world.push(Block::new([x, lo, z], [x + w, hi, z + d]));
        }
        world
    }

    /// Sixteen headings round the compass as Q10 unit vectors (no trig in the tests either).
    const HEADINGS: [[i32; 2]; 16] = [
        [1024, 0],
        [946, 392],
        [724, 724],
        [392, 946],
        [0, 1024],
        [-392, 946],
        [-724, 724],
        [-946, 392],
        [-1024, 0],
        [-946, -392],
        [-724, -724],
        [-392, -946],
        [0, -1024],
        [392, -946],
        [724, -724],
        [946, -392],
    ];

    fn facing(rng: &mut Rng) -> [i32; 2] {
        HEADINGS[usize::try_from(draw(rng, 0, 16)).expect("a heading index")]
    }

    /// One room, 60 ticks of random requests and moves. Returns the hash tape and the counts of
    /// ceiling refusals, wall refusals and raised weapons seen, so the test knows the rooms bite.
    fn play_room(seed: u64) -> (Vec<u64>, [u32; 3]) {
        let mut rng = Rng::new(seed);
        let world = room(&mut rng);
        let len = draw(&mut rng, 400, 1_100);
        // Fighters only stand where even prone fits; a room with no such place is skipped.
        let prone_fits = |p: [i32; 3]| !world.iter().any(|b| overlaps(b, p, [0, 0], Stance::Prone));
        let mut pos = None;
        for _ in 0..40 {
            let c = [
                draw(&mut rng, -3_500, 3_500),
                0,
                draw(&mut rng, -3_500, 3_500),
            ];
            if prone_fits(c) {
                pos = Some(c);
                break;
            }
        }
        let Some(mut pos) = pos else {
            return (Vec::new(), [0; 3]);
        };
        let mut body = Body::new(pos, AHEAD, len);
        let (mut tape, mut counts) = (Vec::new(), [0u32; 3]);
        for _ in 0..60 {
            for _ in 0..8 {
                let c = [
                    pos[0] + draw(&mut rng, -600, 601),
                    0,
                    pos[2] + draw(&mut rng, -600, 601),
                ];
                if prone_fits(c) {
                    pos = c;
                    break;
                }
            }
            let stance =
                [Stance::Stand, Stance::Crouch, Stance::Prone][draw(&mut rng, 0, 3) as usize];
            let lean = [Lean::Center, Lean::Left, Lean::Right][draw(&mut rng, 0, 3) as usize];
            let report = body.update(&world, pos, facing(&mut rng), want(stance, lean));
            counts[0] += u32::from(report.ceiling);
            counts[1] += u32::from(report.wall);
            counts[2] += u32::from(body.weapon() == Weapon::Raised);
            for block in &world {
                assert!(
                    !oracle_capsule_inside(&body, block),
                    "seed {seed}: {body:?} clips {block:?}"
                );
                if body.weapon() == Weapon::Level {
                    assert!(
                        !oracle_weapon_inside(&body, block),
                        "seed {seed}: level weapon in {block:?}"
                    );
                }
            }
            tape.push(body.hash());
        }
        (tape, counts)
    }

    #[test]
    fn a_thousand_seeded_rooms_have_no_clipping_and_replay_x2() {
        let mut totals = [0u32; 3];
        for seed in 0..1_000 {
            let (first, counts) = play_room(seed);
            assert_eq!(first, play_room(seed).0, "seed {seed} must replay");
            for (t, c) in totals.iter_mut().zip(counts) {
                *t += c;
            }
        }
        // The rooms must actually refuse things, or "no clipping" proves nothing.
        assert!(
            totals.iter().all(|t| *t > 100),
            "rooms too tame: {totals:?}"
        );
    }
}
