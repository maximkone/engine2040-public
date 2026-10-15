// Attribution: contact resolve idea adapted from richardhozak/fppe
// (MIT OR Apache-2.0, https://github.com/richardhozak/fppe) src/lib.rs vec
// resolve; reimplemented here fresh over engine-core Fixed, std-only,
// no donor code copied.
//
//! Fixed-point Vec3 collision vectors over a 2-body head-on fixture.
//! Owner: engine-core (test-only harness). Units: positions and velocities
//! are Fixed meters and meters per second, ticks and hashes are u64.
//! Determinism: integer-only Fixed math, no float, no HashMap, no wall clock.
//! Errors: none; `expect` names proven local bounds, never user input.

use engine_core::fixed::Fixed;

/// Integer-only 3D vector over Q47.16 Fixed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FpVec3 {
    x: Fixed,
    y: Fixed,
    z: Fixed,
}

impl FpVec3 {
    /// Zero vector.
    const ZERO: Self = Self {
        x: Fixed::ZERO,
        y: Fixed::ZERO,
        z: Fixed::ZERO,
    };
    /// Fallback contact axis when both centers coincide.
    const X_AXIS: Self = Self {
        x: Fixed::ONE,
        y: Fixed::ZERO,
        z: Fixed::ZERO,
    };

    /// New vector from three Fixed lanes.
    fn new(x: Fixed, y: Fixed, z: Fixed) -> Self {
        Self { x, y, z }
    }

    /// Component-wise sum with wrapping overflow.
    fn add(self, other: Self) -> Self {
        Self {
            x: self.x.wrapping_add(other.x),
            y: self.y.wrapping_add(other.y),
            z: self.z.wrapping_add(other.z),
        }
    }

    /// Component-wise difference with wrapping overflow.
    fn sub(self, other: Self) -> Self {
        Self {
            x: self.x.wrapping_sub(other.x),
            y: self.y.wrapping_sub(other.y),
            z: self.z.wrapping_sub(other.z),
        }
    }

    /// Scale every lane by a Fixed scalar.
    fn scale(self, s: Fixed) -> Self {
        Self {
            x: self.x.wrapping_mul(s),
            y: self.y.wrapping_mul(s),
            z: self.z.wrapping_mul(s),
        }
    }

    /// Dot product with wrapping overflow.
    fn dot(self, other: Self) -> Fixed {
        self.x
            .wrapping_mul(other.x)
            .wrapping_add(self.y.wrapping_mul(other.y))
            .wrapping_add(self.z.wrapping_mul(other.z))
    }

    /// Cross product with wrapping overflow.
    fn cross(self, other: Self) -> Self {
        Self {
            x: self
                .y
                .wrapping_mul(other.z)
                .wrapping_sub(self.z.wrapping_mul(other.y)),
            y: self
                .z
                .wrapping_mul(other.x)
                .wrapping_sub(self.x.wrapping_mul(other.z)),
            z: self
                .x
                .wrapping_mul(other.y)
                .wrapping_sub(self.y.wrapping_mul(other.x)),
        }
    }

    /// Squared length, non-negative for small vectors.
    fn len_sq(self) -> Fixed {
        self.dot(self)
    }

    /// Length rounded down to 1/65536, zero for zero.
    fn len(self) -> Fixed {
        self.len_sq().sqrt()
    }

    /// Unit vector; the +X axis when the input is zero.
    fn normalized(self) -> Self {
        let sq = self.len_sq();
        if sq == Fixed::ZERO {
            return Self::X_AXIS;
        }
        let len = sq.sqrt();
        if len == Fixed::ZERO {
            return Self::X_AXIS;
        }
        Self {
            x: self.x.wrapping_div(len),
            y: self.y.wrapping_div(len),
            z: self.z.wrapping_div(len),
        }
    }

    /// Distance between two points.
    fn distance(self, other: Self) -> Fixed {
        self.sub(other).len()
    }
}

/// One spherical body: center, velocity, radius.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Body {
    pos: FpVec3,
    vel: FpVec3,
    radius: Fixed,
}

/// Ticks per head-on journal: matches the legacy 600-tick span.
const TICKS: u64 = 600;
/// Fixed step of 1/60 s truncated to 1/65536 units (1092 raw).
const DT: Fixed = Fixed::from_bits(1092);
/// Contact margin kept between surfaces, about 0.01 m (655 raw).
const MARGIN: Fixed = Fixed::from_bits(655);
/// Body radius of 0.5 m.
const RADIUS: Fixed = Fixed::HALF;

/// Head-on start: A at x=-2 moving +1 m/s, B at x=+2 moving -1 m/s.
fn head_on_pair() -> [Body; 2] {
    let a = Body {
        pos: FpVec3::new(Fixed::from_int(-2), Fixed::ZERO, Fixed::ZERO),
        vel: FpVec3::new(Fixed::ONE, Fixed::ZERO, Fixed::ZERO),
        radius: RADIUS,
    };
    let b = Body {
        pos: FpVec3::new(Fixed::from_int(2), Fixed::ZERO, Fixed::ZERO),
        vel: FpVec3::new(Fixed::NEG_ONE, Fixed::ZERO, Fixed::ZERO),
        radius: RADIUS,
    };
    [a, b]
}

/// Contact normal from A toward B; +X when centers coincide.
fn contact_normal(a: &Body, b: &Body) -> FpVec3 {
    let delta = b.pos.sub(a.pos);
    if delta.len_sq() == Fixed::ZERO {
        return FpVec3::X_AXIS;
    }
    delta.normalized()
}

/// Push overlapping spheres apart and swap the approaching normal velocity.
/// Odd penetrations split floor/ceil so the pair keeps the exact span.
fn resolve(a: &mut Body, b: &mut Body, margin: Fixed) {
    let delta = b.pos.sub(a.pos);
    let dist = if delta.len_sq() == Fixed::ZERO {
        Fixed::ZERO
    } else {
        delta.len_sq().sqrt()
    };
    let sum = a.radius.wrapping_add(b.radius).wrapping_add(margin);
    if dist >= sum {
        return;
    }
    let normal = if delta.len_sq() == Fixed::ZERO {
        FpVec3::X_AXIS
    } else {
        delta.normalized()
    };
    let pen = sum.wrapping_sub(dist);
    let half = pen.wrapping_mul(Fixed::HALF);
    let other_half = pen.wrapping_sub(half);
    a.pos = a.pos.sub(normal.scale(half));
    b.pos = b.pos.add(normal.scale(other_half));
    let van = a.vel.dot(normal);
    let vbn = b.vel.dot(normal);
    let rel = vbn.wrapping_sub(van);
    if rel < Fixed::ZERO {
        let impulse = normal.scale(rel);
        a.vel = a.vel.add(impulse);
        b.vel = b.vel.sub(impulse);
    }
}

/// One fixed tick: integrate both centers, then resolve the pair.
fn step(pair: &mut [Body; 2], dt: Fixed, margin: Fixed) {
    for body in pair.iter_mut() {
        body.pos = body.pos.add(body.vel.scale(dt));
    }
    let (a, b) = pair.split_at_mut(1);
    resolve(&mut a[0], &mut b[0], margin);
}

/// Canonical state hash over both centers and velocities in order.
fn state_hash(pair: &[Body; 2]) -> u64 {
    Fixed::digest(&[
        pair[0].pos.x,
        pair[0].pos.y,
        pair[0].pos.z,
        pair[1].pos.x,
        pair[1].pos.y,
        pair[1].pos.z,
        pair[0].vel.x,
        pair[0].vel.y,
        pair[0].vel.z,
        pair[1].vel.x,
        pair[1].vel.y,
        pair[1].vel.z,
    ])
}

/// Run the head-on fixture and journal one hash per tick.
fn run_fixture(ticks: u64) -> Vec<u64> {
    let count = usize::try_from(ticks).expect("tick count fits on this platform");
    let mut pair = head_on_pair();
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        step(&mut pair, DT, MARGIN);
        out.push(state_hash(&pair));
    }
    out
}

/// Run without any contact handling; the desync oracle for the soak test.
fn run_without_resolve(ticks: u64) -> Vec<u64> {
    let count = usize::try_from(ticks).expect("tick count fits on this platform");
    let mut pair = head_on_pair();
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        for body in pair.iter_mut() {
            body.pos = body.pos.add(body.vel.scale(DT));
        }
        out.push(state_hash(&pair));
    }
    out
}

/// First tick where two journals differ; `None` when they agree throughout.
fn first_divergence(a: &[u64], b: &[u64]) -> Option<u64> {
    for (index, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        if x != y {
            return Some(u64::try_from(index).expect("tick index fits in u64"));
        }
    }
    None
}

#[test]
fn vec_ops_are_integer_exact() {
    let a = FpVec3::new(Fixed::from_int(1), Fixed::from_int(2), Fixed::from_int(3));
    let b = FpVec3::new(Fixed::from_int(4), Fixed::from_int(5), Fixed::from_int(6));
    assert_eq!(a.dot(b), Fixed::from_int(32), "1*4 + 2*5 + 3*6 is 32");
    let x = FpVec3::new(Fixed::ONE, Fixed::ZERO, Fixed::ZERO);
    let y = FpVec3::new(Fixed::ZERO, Fixed::ONE, Fixed::ZERO);
    let z = FpVec3::new(Fixed::ZERO, Fixed::ZERO, Fixed::ONE);
    assert_eq!(x.cross(y), z, "x cross y is z");
    assert_eq!(y.cross(x), FpVec3::ZERO.sub(z), "cross is antisymmetric");
    let v = FpVec3::new(Fixed::from_int(3), Fixed::from_int(4), Fixed::ZERO);
    assert_eq!(v.len_sq(), Fixed::from_int(25), "3-4-5 squared length");
    assert_eq!(v.len(), Fixed::from_int(5), "3-4-5 length is exact");
    assert_eq!(x.add(y), FpVec3::new(Fixed::ONE, Fixed::ONE, Fixed::ZERO));
    assert_eq!(
        b.sub(a),
        FpVec3::new(Fixed::from_int(3), Fixed::from_int(3), Fixed::from_int(3))
    );
    assert_eq!(
        a.scale(Fixed::from_int(2)),
        FpVec3::new(Fixed::from_int(2), Fixed::from_int(4), Fixed::from_int(6))
    );
    assert_eq!(
        FpVec3::ZERO.normalized(),
        FpVec3::X_AXIS,
        "zero has fallback axis"
    );
}

#[test]
fn resolve_projects_penetration_with_margin() {
    let mut a = Body {
        pos: FpVec3::new(Fixed::from_bits(-24576), Fixed::ZERO, Fixed::ZERO),
        vel: FpVec3::ZERO,
        radius: RADIUS,
    };
    let mut b = Body {
        pos: FpVec3::new(Fixed::from_bits(24576), Fixed::ZERO, Fixed::ZERO),
        vel: FpVec3::ZERO,
        radius: RADIUS,
    };
    resolve(&mut a, &mut b, MARGIN);
    let sum = RADIUS.wrapping_add(RADIUS).wrapping_add(MARGIN);
    let dist = a.pos.distance(b.pos);
    let gap = dist.wrapping_sub(sum).abs().to_bits();
    assert!(gap <= 2, "margin gap closes to 2 ulps, got raw");
    assert!(
        dist.wrapping_add(Fixed::from_bits(2)) >= sum,
        "surfaces keep the margin within 2 ulps"
    );
    let mid = a.pos.add(b.pos).scale(Fixed::HALF);
    assert!(
        mid.x.abs().to_bits() <= 2,
        "symmetric push keeps the midpoint"
    );
}

#[test]
fn zero_distance_fallback_is_deterministic() {
    let make = || {
        let mut a = Body {
            pos: FpVec3::ZERO,
            vel: FpVec3::ZERO,
            radius: RADIUS,
        };
        let mut b = Body {
            pos: FpVec3::ZERO,
            vel: FpVec3::ZERO,
            radius: RADIUS,
        };
        resolve(&mut a, &mut b, MARGIN);
        [a, b]
    };
    let first = make();
    let second = make();
    assert_eq!(first, second, "coincident resolve must replay x2");
    let sum = RADIUS.wrapping_add(RADIUS).wrapping_add(MARGIN);
    let dist = first[0].pos.distance(first[1].pos);
    let gap = dist.wrapping_sub(sum).abs().to_bits();
    assert!(gap <= 2, "fallback separates within 2 ulps");
    assert_eq!(
        contact_normal(&first[0], &first[0]),
        FpVec3::X_AXIS,
        "coincident normal is +X"
    );
}

#[test]
fn head_on_600_tick_hash_x2_identical() {
    let first = run_fixture(TICKS);
    let second = run_fixture(TICKS);
    let count = usize::try_from(TICKS).expect("600 fits on this platform");
    assert_eq!(first.len(), count, "journal holds one hash per tick");
    assert_eq!(second.len(), count, "rerun holds one hash per tick");
    assert_eq!(first, second, "same fixture must hash x2 identical");
    assert_ne!(
        first.first(),
        first.last(),
        "bodies must move across 600 ticks"
    );
    let mut pair = head_on_pair();
    for _ in 0..count {
        step(&mut pair, DT, MARGIN);
    }
    let sum = RADIUS.wrapping_add(RADIUS).wrapping_add(MARGIN);
    let dist = pair[0].pos.distance(pair[1].pos);
    assert!(dist >= sum, "600 ticks end separated with margin kept");
    assert!(
        pair[0].vel.x < Fixed::ZERO,
        "A rebounds toward -X after the head-on hit"
    );
    assert!(
        pair[1].vel.x > Fixed::ZERO,
        "B rebounds toward +X after the head-on hit"
    );
}

#[test]
fn skipped_resolve_diverges_like_a_desync() {
    let clean = run_fixture(TICKS);
    let clean_again = run_fixture(TICKS);
    assert_eq!(clean, clean_again, "clean replay must be identical x2");
    let faulty = run_without_resolve(TICKS);
    let faulty_again = run_without_resolve(TICKS);
    assert_eq!(faulty, faulty_again, "faulty replay must be identical x2");
    let first = first_divergence(&clean, &faulty);
    assert!(
        first.is_some(),
        "skipping the projection must diverge like a desync"
    );
    let at = first.expect("divergence seen above");
    assert!(at < TICKS, "divergence lands inside the 600-tick span");
    assert_eq!(
        &clean[..usize::try_from(at).expect("tick fits")],
        &faulty[..usize::try_from(at).expect("tick fits")],
        "journals agree before the first divergent tick"
    );
}
