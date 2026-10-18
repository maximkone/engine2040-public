//! contact_feet: live contact shade under the fighters' feet.
//!
//! Owner: render lane. Consumers: the forward shader (`sun_contact_factor`
//! reads `shadow.feet`, group 0 binding 3, one `vec4` per slot) fed by
//! `Presenter::set_contact_feet`, which the game's fighter look calls with
//! where the bodies really stand. Before this the shader darkened sand around
//! 24 hard-coded world positions (old duel fixtures), so the live arena showed
//! dark discs in an X across the sand and none under the fighters.
//! Units: meters, unit-free 0..=1 strengths. Threading: pure functions, no
//! shared state. Determinism: fixed f32 evaluation order, the same feet give
//! the same bytes and factors twice; pixels stay presentation, never the sim
//! hash. Errors: none. A non-finite or non-positive foot is a dead slot, a
//! wild sample point reads lit, so a bad feed can never punch black holes
//! into the sand. Budget: eight feet, a few dozen float ops per fragment.

/// Most feet one frame carries: one per fighter slot the look can show.
pub const CONTACT_FEET_MAX: usize = 8;
/// Bytes of the feet block inside the shadow uniform: one `vec4` per slot
/// (x meters, z meters, radius meters, strength).
pub const CONTACT_FEET_BYTES: usize = CONTACT_FEET_MAX * 16;
/// Share of the light a foot takes away at its centre (0.55 keeps 45 percent).
pub const CONTACT_DARKEN: f32 = 0.55;
/// Largest radius one foot may ask for, meters (a lying body, not a hall).
pub const CONTACT_RADIUS_MAX_M: f32 = 3.0;
/// Height where the contact shade starts to fade out, meters: the sand and
/// the ankles still take it.
pub const CONTACT_FADE_LO_M: f32 = 0.1;
/// Height where the contact shade is gone, meters: knees and above stay lit.
pub const CONTACT_FADE_HI_M: f32 = 0.3;

/// One fighter's contact blob on the sand.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ContactFoot {
    /// Floor x in meters.
    pub x_m: f32,
    /// Floor z in meters.
    pub z_m: f32,
    /// Blob radius in meters: the shade is gone at this distance.
    pub radius_m: f32,
}

impl ContactFoot {
    /// A live slot: finite, with a radius in `(0, CONTACT_RADIUS_MAX_M]`.
    #[must_use]
    fn is_live(&self) -> bool {
        self.x_m.is_finite()
            && self.z_m.is_finite()
            && self.radius_m.is_finite()
            && self.radius_m > 0.0
            && self.radius_m <= CONTACT_RADIUS_MAX_M
    }
}

/// Cubic ease of `t` in `0..=1`: the curve of WGSL `smoothstep` between its
/// edges.
fn ease(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// WGSL `smoothstep(lo, hi, x)`: `0` at or below `lo`, `1` at or above `hi`.
fn smoothstep(lo: f32, hi: f32, x: f32) -> f32 {
    ease(((x - lo) / (hi - lo)).clamp(0.0, 1.0))
}

/// The feet block of the shadow uniform: slot `i` is `[x, z, radius,
/// strength]` little-endian, dead slots and slots past [`CONTACT_FEET_MAX`]
/// are zero (strength 0 switches the slot off in the shader). Pure.
#[must_use]
pub fn contact_feet_bytes(feet: &[ContactFoot]) -> [u8; CONTACT_FEET_BYTES] {
    let mut out = [0u8; CONTACT_FEET_BYTES];
    let live = feet.iter().filter(|foot| foot.is_live());
    for (slot, foot) in live.take(CONTACT_FEET_MAX).enumerate() {
        let words = [foot.x_m, foot.z_m, foot.radius_m, CONTACT_DARKEN];
        for (word, value) in words.iter().enumerate() {
            let at = slot * 16 + word * 4;
            out[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
    out
}

/// CPU oracle of the shader's `sun_contact_factor`: the share of the light a
/// point at `point` (x, y, z meters) keeps, `1.0` away from every foot. Each
/// live foot takes up to [`CONTACT_DARKEN`] with a smoothstep falloff to its
/// radius, scaled down to nothing between [`CONTACT_FADE_LO_M`] and
/// [`CONTACT_FADE_HI_M`] up the body. A wild point reads lit. Pure.
#[must_use]
pub fn contact_factor(feet: &[ContactFoot], point: [f32; 3]) -> f32 {
    if !point.iter().all(|v| v.is_finite()) {
        return 1.0;
    }
    let ground = 1.0 - smoothstep(CONTACT_FADE_LO_M, CONTACT_FADE_HI_M, point[1]);
    let mut keep = 1.0f32;
    let live = feet.iter().filter(|foot| foot.is_live());
    for foot in live.take(CONTACT_FEET_MAX) {
        let dist = (point[0] - foot.x_m).hypot(point[2] - foot.z_m);
        let fall = smoothstep(0.0, 1.0, 1.0 - dist / foot.radius_m);
        keep *= 1.0 - CONTACT_DARKEN * ground * fall;
    }
    keep
}

#[cfg(test)]
mod tests {
    use super::*;

    const FORWARD_WGSL: &str = include_str!("shaders/forward.wgsl");

    fn foot(x_m: f32, z_m: f32) -> ContactFoot {
        ContactFoot {
            x_m,
            z_m,
            radius_m: 0.9,
        }
    }

    #[test]
    fn no_feet_leaves_every_sand_point_lit() {
        // The old shader darkened (2, -1), (2, 1), (0, 0) and twenty-one more
        // fixed spots whatever the fighters did: with nobody standing there
        // the sand is open.
        for at in [
            [2.0, 0.0, -1.0],
            [2.0, 0.0, 1.0],
            [0.0, 0.0, 0.0],
            [-8.0, 0.0, 0.0],
        ] {
            assert_eq!(contact_factor(&[], at), 1.0, "open sand at {at:?}");
        }
        assert_eq!(contact_feet_bytes(&[]), [0u8; CONTACT_FEET_BYTES]);
    }

    #[test]
    fn the_shade_follows_the_feet_and_nothing_else() {
        let feet = [foot(5.0, 3.0), foot(-6.0, -2.0)];
        let under = contact_factor(&feet, [5.0, 0.0, 3.0]);
        assert!(
            (under - (1.0 - CONTACT_DARKEN)).abs() < 1e-6,
            "centre: {under}"
        );
        assert!(
            contact_factor(&feet, [-6.0, 0.0, -2.0]) < 0.5,
            "second foot too"
        );
        // The old fixture spots stay lit while the fighters stand elsewhere.
        assert_eq!(contact_factor(&feet, [2.0, 0.0, -1.0]), 1.0);
        assert_eq!(contact_factor(&feet, [2.0, 0.0, 1.0]), 1.0);
        // Soft edge: strictly between the centre and open sand, then lit at the radius.
        let mid = contact_factor(&feet, [5.45, 0.0, 3.0]);
        assert!(mid > under && mid < 1.0, "soft edge: {mid}");
        assert_eq!(
            contact_factor(&feet, [5.9, 0.0, 3.0]),
            1.0,
            "gone at the radius"
        );
        // A moved foot moves the shade.
        let moved = [foot(5.0, -3.0)];
        assert_eq!(contact_factor(&moved, [5.0, 0.0, 3.0]), 1.0);
        assert!(contact_factor(&moved, [5.0, 0.0, -3.0]) < 0.5);
    }

    #[test]
    fn the_shade_fades_up_the_body_and_never_climbs_to_the_head() {
        let feet = [foot(0.0, 0.0)];
        let ankle = contact_factor(&feet, [0.0, 0.1, 0.0]);
        let shin = contact_factor(&feet, [0.0, 0.2, 0.0]);
        assert!(
            ankle < shin && shin < 1.0,
            "fades with height: {ankle} {shin}"
        );
        assert_eq!(contact_factor(&feet, [0.0, 0.3, 0.0]), 1.0, "knee is lit");
        assert_eq!(contact_factor(&feet, [0.0, 1.7, 0.0]), 1.0, "head is lit");
    }

    #[test]
    fn overlapping_feet_stack_but_never_go_black() {
        let feet = [foot(0.0, 0.0), foot(0.2, 0.0), foot(0.0, 0.2)];
        let stacked = contact_factor(&feet, [0.0, 0.0, 0.0]);
        assert!(
            stacked < contact_factor(&feet[..1], [0.0, 0.0, 0.0]),
            "stacks"
        );
        assert!(stacked > 0.05, "never a black hole: {stacked}");
    }

    #[test]
    fn dead_feet_and_wild_points_read_lit() {
        let dead = [
            foot(f32::NAN, 0.0),
            foot(0.0, f32::INFINITY),
            ContactFoot {
                x_m: 0.0,
                z_m: 0.0,
                radius_m: 0.0,
            },
            ContactFoot {
                x_m: 0.0,
                z_m: 0.0,
                radius_m: -1.0,
            },
            ContactFoot {
                x_m: 0.0,
                z_m: 0.0,
                radius_m: CONTACT_RADIUS_MAX_M + 0.1,
            },
        ];
        assert_eq!(contact_factor(&dead, [0.0, 0.0, 0.0]), 1.0);
        assert_eq!(contact_feet_bytes(&dead), [0u8; CONTACT_FEET_BYTES]);
        assert_eq!(contact_factor(&[foot(0.0, 0.0)], [f32::NAN, 0.0, 0.0]), 1.0);
        assert_eq!(
            contact_factor(&[foot(0.0, 0.0)], [0.0, f32::INFINITY, 0.0]),
            1.0
        );
    }

    #[test]
    fn the_bytes_are_one_vec4_per_live_foot_in_order_capped_at_eight() {
        let bytes = contact_feet_bytes(&[foot(1.5, -2.5), foot(f32::NAN, 0.0), foot(3.0, 4.0)]);
        let word = |slot: usize, at: usize| {
            let from = slot * 16 + at * 4;
            f32::from_le_bytes([
                bytes[from],
                bytes[from + 1],
                bytes[from + 2],
                bytes[from + 3],
            ])
        };
        assert_eq!(
            [word(0, 0), word(0, 1), word(0, 2), word(0, 3)],
            [1.5, -2.5, 0.9, CONTACT_DARKEN]
        );
        assert_eq!(
            [word(1, 0), word(1, 1)],
            [3.0, 4.0],
            "the dead slot is skipped"
        );
        assert_eq!(word(2, 3), 0.0, "unused slots stay off");
        let many: Vec<ContactFoot> = (0..12u8).map(|i| foot(f32::from(i), 0.0)).collect();
        let capped = contact_feet_bytes(&many);
        assert_eq!(capped.len(), CONTACT_FEET_BYTES);
        let last = (CONTACT_FEET_MAX - 1) * 16;
        assert_eq!(
            f32::from_le_bytes([
                capped[last],
                capped[last + 1],
                capped[last + 2],
                capped[last + 3]
            ]),
            7.0,
            "the eighth foot is the last one carried"
        );
    }

    #[test]
    fn the_forward_shader_reads_the_live_block_and_carries_no_fixed_disks() {
        // The parity pin: the WGSL loops over `shadow.feet` with the same
        // constants as this oracle, and the old fixture positions are gone.
        for probe in [
            "feet: array<vec4<f32>, 8>",
            "shadow.feet[i]",
            "const CONTACT_FEET_MAX: u32 = 8u;",
            "smoothstep(0.1, 0.3, world.y)",
            "foot.w",
        ] {
            assert!(
                FORWARD_WGSL.contains(probe),
                "forward reads the feet: {probe}"
            );
        }
        assert!(
            FORWARD_WGSL.contains("contact_live = contact_from_feet(in.world_pos);"),
            "the window fs computes the contact once per fragment"
        );
        // Only `fs` reads the feet block: the offscreen entries run in
        // pipelines with no shadow uniform bound and keep the neutral value.
        for entry in ["fn fs_display", "fn fs_skinned"] {
            let body = FORWARD_WGSL
                .split(entry)
                .nth(1)
                .and_then(|tail| tail.split("@fragment").next())
                .expect("offscreen entry exists");
            assert!(!body.contains("shadow."), "{entry} binds no shadow uniform");
        }
        for gone in [
            "SUN_STAGED",
            "near_a",
            "near_center",
            "mix(SUN_STAGED_DARKEN",
        ] {
            assert!(
                !FORWARD_WGSL.contains(gone),
                "no hard-coded contact disks left: {gone}"
            );
        }
        // Constants the shader hard-codes equal the oracle's.
        assert_eq!(CONTACT_FADE_LO_M, 0.1);
        assert_eq!(CONTACT_FADE_HI_M, 0.3);
        assert_eq!(CONTACT_FEET_MAX, 8);
    }
}
