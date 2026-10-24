//! hit_number: a blow prints its number where it lands (row SW151).
//!
//! A `27` damage hit floats up from the shoulder it hit and fades, so the fight
//! reads without the HUD. A 16-entry ring remembers the last blows
//! (`{tick, at_m, amount, side}`); a live entry rises 0.6 m over 36 ticks with
//! a `(1 - p)^2` alpha, its digits are laid out from the shared glyph atlas
//! (`engine_render::overlay::text`), and it becomes Y-billboard quads on the
//! camera plane at a fixed metres-per-atlas-pixel, so the glyph height holds
//! whether the camera pitches or not. Donor: Godot `scene/3d/label_3d.cpp`
//! (billboard enum with Y-Billboard, `pixel_size` metres per atlas pixel,
//! `FLAG_DISABLE_DEPTH_TEST`), read 2026-10-24.
//!
//! Presentation only: the ring is view state, never step state, so the tick
//! and the golden `d57b5c8ae3096614` never read it. Caller (later wiring): the
//! fight_fx hit event fills the ring and the frame draws its quads. Units:
//! metres, radians, atlas pixels; ticks at 60 Hz. Determinism: integer ages
//! and float geometry from the same inputs; the same tape gives the same ring
//! twice. Errors: none; a spent or off-screen number draws nothing.

use engine_render::overlay::text::{CELL_H, CELL_W};

/// Entries the ring keeps: the last sixteen blows on screen.
pub const RING: usize = 16;
/// Ticks a number lives before it is exactly clear.
pub const LIFE_TICKS: u32 = 36;
/// Metres a number rises over its whole life (row: "rising 0.6 m").
pub const RISE_M: f32 = 0.6;
/// Metres per atlas pixel at the number's distance (row: `pixel_size`): sized
/// so the 8-pixel cell is 12 px tall at 3 m on a 720p frame behind the arena's
/// 60-degree lens, `12 * tan(30 deg) * 3 m / (8 px * 360 px)`.
pub const NUMBER_PIXEL_M: f32 = 0.007_217;
/// Metres a left or right number steps aside so two blows do not stack.
pub const SIDE_NUDGE_M: f32 = 0.08;

/// Which side of the body the blow landed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// The victim's left: the number steps left.
    Left,
    /// The victim's right: the number steps right.
    Right,
}

impl Side {
    /// Lateral nudge for this side, metres.
    #[must_use]
    pub const fn nudge_m(self) -> f32 {
        match self {
            Self::Left => -SIDE_NUDGE_M,
            Self::Right => SIDE_NUDGE_M,
        }
    }
}

/// One blow's number: where it landed, how hard, and on which side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitNumber {
    /// Sim tick the blow landed on.
    pub tick: u32,
    /// Contact point in metres: the shoulder it hit.
    pub at_m: [f32; 3],
    /// Damage of the blow, printed as digits.
    pub amount: u32,
    /// Side of the body it hit.
    pub side: Side,
}

/// The last blows, oldest overwritten first.
#[derive(Clone, Copy, Debug)]
pub struct HitNumbers {
    ring: [Option<HitNumber>; RING],
    head: usize,
}

impl Default for HitNumbers {
    fn default() -> Self {
        Self::new()
    }
}

/// The camera plane a number billboards on: right across the view, up in the
/// world, so the digits stand upright however the camera pitches.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Billboard {
    /// Screen-right in world space.
    pub right: [f32; 3],
    /// Screen-up in world space: always world +Y for a Y-billboard.
    pub up: [f32; 3],
}

impl Billboard {
    /// The Y-billboard for an eye and a target: right is the view direction
    /// crossed with world up, up stays world up. A camera looking straight
    /// down keeps a stable right vector instead of a zero cross product.
    #[must_use]
    pub fn y_billboard(eye: [f32; 3], target: [f32; 3]) -> Self {
        let view = sub(target, eye);
        let mut right = cross(view, [0.0, 1.0, 0.0]);
        if length_sq(right) < 1.0e-12 {
            right = [1.0, 0.0, 0.0];
        }
        Self {
            right: normalize(right),
            up: [0.0, 1.0, 0.0],
        }
    }
}

impl HitNumbers {
    /// An empty ring.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            ring: [None; RING],
            head: 0,
        }
    }

    /// Add one blow's number, overwriting the oldest when the ring is full.
    pub fn push(&mut self, number: HitNumber) {
        self.ring[self.head] = Some(number);
        self.head = (self.head + 1) % RING;
    }

    /// Blows remembered (at most [`RING`]).
    #[must_use]
    pub fn len(&self) -> usize {
        self.ring.iter().flatten().count()
    }

    /// True when no blow is remembered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Fill `out` with the live numbers at `now`, oldest first (tick order,
    /// so the draw order is stable for the same tape); returns how many were
    /// written. Heap-free: the caller owns the buffer.
    pub fn live_into<'a>(&'a self, now: u32, out: &mut [Option<&'a HitNumber>; RING]) -> usize {
        let mut n = 0;
        for number in self.ring.iter().flatten() {
            if age(now, number.tick) >= LIFE_TICKS {
                continue;
            }
            // Insert by tick, oldest first: the ring's own order is slot order.
            let mut at = n;
            while at > 0 {
                let before = out[at - 1].map_or(0, |prev| prev.tick);
                if before <= number.tick {
                    break;
                }
                at -= 1;
            }
            let mut i = n;
            while i > at {
                out[i] = out[i - 1];
                i -= 1;
            }
            out[at] = Some(number);
            n += 1;
        }
        n
    }

    /// The number of live entries at `now`.
    #[must_use]
    pub fn live_count(&self, now: u32) -> usize {
        let mut out = [None; RING];
        self.live_into(now, &mut out)
    }
}

/// Ticks since the blow landed, saturating so a tick before the blow reads 0.
#[must_use]
pub fn age(now: u32, tick: u32) -> u32 {
    now.saturating_sub(tick)
}

/// Progress through the number's life, `0..=1`.
#[must_use]
pub fn progress(age: u32) -> f32 {
    let life = f32::from(u16::try_from(LIFE_TICKS).unwrap_or(0));
    let spent = f32::from(u16::try_from(age.min(LIFE_TICKS)).unwrap_or(0));
    spent / life
}

/// Alpha at `age`: 1 on the hit tick, `(1 - p)^2` after, exactly 0 at
/// [`LIFE_TICKS`] and past it (row: fades with a squared falloff).
#[must_use]
pub fn alpha_at(age: u32) -> f32 {
    if age >= LIFE_TICKS {
        return 0.0;
    }
    let left = 1.0 - progress(age);
    left * left
}

/// Metres the number has risen at `age`.
#[must_use]
pub fn rise_at(age: u32) -> f32 {
    RISE_M * progress(age)
}

/// The anchor a number draws at now: its contact point, risen and stepped to
/// its side.
#[must_use]
pub fn anchor_m(number: &HitNumber, now: u32) -> [f32; 3] {
    let a = age(now, number.tick);
    [
        number.at_m[0] + number.side.nudge_m(),
        number.at_m[1] + rise_at(a),
        number.at_m[2],
    ]
}

/// `a > b`, with `false` for a NaN on either side: the form every rejection
/// below wants, stated once so the NaN rule reads the same at each site.
fn is_greater(a: f32, b: f32) -> bool {
    a.partial_cmp(&b) == Some(core::cmp::Ordering::Greater)
}

/// The world height of one atlas pixel at `distance_m` behind a lens of
/// `viewport_h` pixels and `fov_y_rad` vertical field of view. Pure geometry:
/// `glyph_px = world_h * (viewport_h / 2) / (tan(fov/2) * distance)`.
#[must_use]
pub fn glyph_px(world_h_m: f32, distance_m: f32, viewport_h: f32, fov_y_rad: f32) -> f32 {
    if !is_greater(distance_m, 0.0) || !is_greater(viewport_h, 0.0) || !is_greater(fov_y_rad, 0.0) {
        return 0.0;
    }
    let half = (fov_y_rad * 0.5).tan();
    if !is_greater(half, 0.0) {
        return 0.0;
    }
    world_h_m * (viewport_h * 0.5) / (half * distance_m)
}

/// The quad size of a laid-out label in metres: one atlas cell per column.
#[must_use]
pub fn label_size_m(columns: usize) -> (f32, f32) {
    let cols = f32::from(u16::try_from(columns).unwrap_or(u16::MAX));
    (
        cols * f32::from(u16::try_from(CELL_W).unwrap_or(u16::MAX)) * NUMBER_PIXEL_M,
        f32::from(u16::try_from(CELL_H).unwrap_or(u16::MAX)) * NUMBER_PIXEL_M,
    )
}

/// The four world corners of one number quad, counter-clockwise from the
/// bottom left as seen by the camera. `center_m` is the quad's middle.
#[must_use]
pub fn quad_corners(
    center_m: [f32; 3],
    plane: &Billboard,
    width_m: f32,
    height_m: f32,
) -> [[f32; 3]; 4] {
    let (hw, hh) = (width_m * 0.5, height_m * 0.5);
    let corner = |sx: f32, sy: f32| -> [f32; 3] {
        [
            center_m[0] + plane.right[0] * hw * sx + plane.up[0] * hh * sy,
            center_m[1] + plane.right[1] * hw * sx + plane.up[1] * hh * sy,
            center_m[2] + plane.right[2] * hw * sx + plane.up[2] * hh * sy,
        ]
    };
    [
        corner(-1.0, -1.0),
        corner(1.0, -1.0),
        corner(1.0, 1.0),
        corner(-1.0, 1.0),
    ]
}

/// Write `amount`'s digits into `out` (cleared first), no heap: the caller
/// keeps the buffer and the digits come from a stack buffer. Zero prints `0`.
pub fn label_into(amount: u32, out: &mut String) {
    out.clear();
    let mut digits = [0_u8; 10];
    let mut at = digits.len();
    let mut left = amount;
    loop {
        at -= 1;
        digits[at] = b'0' + u8::try_from(left % 10).unwrap_or(0);
        left /= 10;
        if left == 0 || at == 0 {
            break;
        }
    }
    if let Ok(text) = core::str::from_utf8(&digits[at..]) {
        out.push_str(text);
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn length_sq(a: [f32; 3]) -> f32 {
    a[0] * a[0] + a[1] * a[1] + a[2] * a[2]
}

fn normalize(a: [f32; 3]) -> [f32; 3] {
    let len = length_sq(a).sqrt();
    if !is_greater(len, 0.0) {
        return [0.0, 0.0, 0.0];
    }
    [a[0] / len, a[1] / len, a[2] / len]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blow(tick: u32, amount: u32, side: Side) -> HitNumber {
        HitNumber {
            tick,
            at_m: [0.0, 1.4, 3.0],
            amount,
            side,
        }
    }

    #[test]
    fn hit_number_rises_and_fades_to_zero() {
        // SW151: a 27 hit reads 27 where it landed, floats 0.6 m up over 36
        // ticks and is exactly clear on tick + 36.
        let mut numbers = HitNumbers::new();
        assert!(numbers.is_empty());
        numbers.push(blow(100, 27, Side::Right));
        assert_eq!(numbers.len(), 1);
        assert_eq!(numbers.live_count(100), 1, "live on the hit tick");

        let mut text = String::new();
        let mut live = [None; RING];
        assert_eq!(numbers.live_into(100, &mut live), 1, "one live number");
        let number = *live[0].expect("one live number");
        label_into(number.amount, &mut text);
        assert_eq!(text, "27", "the blow prints its number");
        label_into(0, &mut text);
        assert_eq!(text, "0", "a zero still prints");
        label_into(1_234_567, &mut text);
        assert_eq!(text, "1234567", "every digit, in order");

        // Alpha 1 on the hit tick, squared falloff after, 0 at tick + 36.
        assert_eq!(alpha_at(0), 1.0, "full on the hit tick");
        let mut last = alpha_at(0);
        for age in 1..LIFE_TICKS {
            let now = alpha_at(age);
            assert!(now < last && now > 0.0, "age {age} fades: {last} -> {now}");
            last = now;
        }
        assert_eq!(alpha_at(LIFE_TICKS), 0.0, "exactly 0 at tick + 36");
        assert_eq!(alpha_at(LIFE_TICKS + 5), 0.0, "and it stays clear");
        assert_eq!(
            numbers.live_count(100 + LIFE_TICKS),
            0,
            "the ring goes quiet"
        );

        // Rise: none at the hit, 0.6 m at the end, and the anchor follows.
        assert_eq!(rise_at(0), 0.0);
        assert!((rise_at(LIFE_TICKS) - RISE_M).abs() < 1e-6, "0.6 m up");
        let start = anchor_m(&number, 100);
        let end = anchor_m(&number, 100 + LIFE_TICKS);
        assert!((end[1] - start[1] - RISE_M).abs() < 1e-6, "it floated up");
        assert!(start[0] > 0.0, "a right-side hit steps right");

        // x2 equal: the same tape fills the same rings twice.
        let tape = |ring: &mut HitNumbers| {
            for i in 0..20_u32 {
                ring.push(HitNumber {
                    tick: i,
                    at_m: [0.1 * i as f32, 1.4, 2.0],
                    amount: 10 + i,
                    side: if i % 2 == 0 { Side::Left } else { Side::Right },
                });
            }
        };
        let (mut a, mut b) = (HitNumbers::new(), HitNumbers::new());
        tape(&mut a);
        tape(&mut b);
        assert_eq!(a.len(), RING, "the ring keeps sixteen");
        assert_eq!(a.ring, b.ring, "same tape, same ring");
        assert_eq!(a.head, b.head);
    }

    #[test]
    fn a_27_at_three_metres_is_twelve_pixels_tall_at_720p() {
        // The accept's number: one atlas cell (8 px) at NUMBER_PIXEL_M, on a
        // 720p frame behind the arena's 60-degree lens, 3 m away.
        let world_h = f32::from(u16::try_from(CELL_H).unwrap_or(0)) * NUMBER_PIXEL_M;
        let px = glyph_px(world_h, 3.0, 720.0, core::f32::consts::FRAC_PI_3);
        assert!((px - 12.0).abs() < 0.05, "glyph height {px} px, want 12");
        // Two digits span two atlas cells, so "27" is wider than it is tall.
        let (w, h) = label_size_m(2);
        assert!((h - world_h).abs() < 1e-6);
        assert!(w > h, "\"27\" is two cells wide: {w} x {h}");
        // A body further away reads smaller; a spent number draws nothing.
        let far = glyph_px(world_h, 6.0, 720.0, core::f32::consts::FRAC_PI_3);
        assert!((far - 6.0).abs() < 0.05, "6 px at 6 m: {far}");
        assert_eq!(
            glyph_px(world_h, 0.0, 720.0, core::f32::consts::FRAC_PI_3),
            0.0
        );
    }

    #[test]
    fn y_billboard_keeps_the_quad_upright() {
        // The quad stands on world +Y whatever the camera does: a 30-degree
        // pitch changes the right vector, never the up axis or the height.
        let flat = Billboard::y_billboard([0.0, 1.6, 0.0], [0.0, 1.6, 1.0]);
        let pitched = Billboard::y_billboard([0.0, 3.0, -4.0], [0.0, 1.0, 1.0]);
        // A pitch leaves the quad alone: Y-billboard keeps its width
        // horizontal, which is the whole point of the up vector staying world
        // up. A yaw does turn the width around.
        assert_eq!(flat.right, pitched.right, "a pitch does not turn the quad");
        let yawed = Billboard::y_billboard([0.0, 1.6, 0.0], [1.0, 1.6, 1.0]);
        assert_ne!(yawed.right, flat.right, "a yaw turns the width, not the up");
        for plane in [flat, pitched, yawed] {
            assert_eq!(plane.up, [0.0, 1.0, 0.0], "up is world up");
            let (_, h) = label_size_m(2);
            let corners = quad_corners([0.5, 1.5, 3.0], &plane, 0.05, h);
            let height = corners[2][1] - corners[0][1];
            assert!(
                (height - h).abs() < 1e-6,
                "the quad keeps its height: {height}"
            );
            assert_eq!(corners[0][1], corners[1][1], "the bottom edge is level");
            assert_eq!(corners[2][1], corners[3][1], "the top edge is level");
            // The bottom edge is the quad's width: along screen-right, level.
            let across = sub(corners[1], corners[0]);
            assert!(
                length_sq(cross(across, plane.right)) < 1e-12,
                "the width runs along screen-right"
            );
            assert!(across[1].abs() < 1e-6, "the width is level: {across:?}");
        }
    }

    #[test]
    fn the_ring_keeps_the_last_sixteen_blows_in_order() {
        let mut numbers = HitNumbers::new();
        for i in 0..20_u32 {
            numbers.push(blow(i, 5 + i, Side::Left));
        }
        assert_eq!(numbers.len(), RING, "oldest overwritten first");
        let mut live = [None; RING];
        let n = numbers.live_into(19, &mut live);
        assert_eq!(n, RING);
        let ticks: Vec<u32> = live[..n].iter().flatten().map(|n| n.tick).collect();
        assert_eq!(ticks.first(), Some(&4), "the four oldest fell off");
        assert_eq!(ticks.last(), Some(&19));
        assert!(
            ticks.windows(2).all(|w| w[0] < w[1]),
            "tick order: {ticks:?}"
        );
    }
}
