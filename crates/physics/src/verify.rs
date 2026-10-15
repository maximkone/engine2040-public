//! verify: determinism harness. Seed in, hash out, twice on one seed.
#![forbid(unsafe_code)]

use crate::rigid::Body;
use crate::solver::Solver;

/// FNV-1a over x/y/vx/vy bits (re-export of the `rigid` canon, never a copy).
pub use crate::rigid::state_hash;

/// 5-box stack with seed-derived sub-mm x jitter. Deterministic on seed.
/// Byte `i` of the LE seed is the jitter source (`to_le_bytes` is the same
/// shift-and-mask without an `as` cast); `u16`/`From` keep it cast-free.
pub fn box5(seed: u64) -> Vec<Body> {
    (0u16..5)
        .map(|i| {
            let j = f32::from(seed.to_le_bytes()[usize::from(i)]) / 255.0 * 0.002 - 0.001;
            Body::new(j, 0.5 + f32::from(i) * 1.02, 0.0, 0.0, 0.5, 0.5)
        })
        .collect()
}

/// Build, step fixed `dt` x `ticks`, hash — twice on one seed. Asserts identical.
pub fn verify_same_seed(
    seed: u64,
    ticks: usize,
    dt: f32,
    mut build: impl FnMut() -> (Vec<Body>, Solver),
) -> u64 {
    let mut run = || {
        let (mut b, mut s) = build();
        for _ in 0..ticks {
            s.step(&mut b, dt);
        }
        s.state_hash(&b)
    };
    let (h1, h2) = (run(), run());
    assert_eq!(h1, h2, "desync seed={seed}");
    h1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rigid::FIXED_DT;

    #[test]
    fn five_box_sleep() {
        let h = verify_same_seed(2040, 900, FIXED_DT, || (box5(2040), Solver::new()));
        assert_eq!(state_hash(&box5(2040)), state_hash(&box5(2040)));
        let (mut b, mut s) = (box5(2040), Solver::new());
        for _ in 0..900 {
            s.step(&mut b, FIXED_DT);
        }
        assert_eq!(s.sleeping_count(), 5, "all boxes must sleep");
        assert_eq!(s.state_hash(&b), h, "hash identical x2 runs");
        for _ in 0..120 {
            s.step(&mut b, FIXED_DT);
        }
        assert_eq!(s.state_hash(&b), h, "sleeping hash stable");
        for w in b.windows(2) {
            assert!(w[1].y > w[0].y, "stack order kept");
        }
        for body in &b {
            assert!(
                body.y >= body.hy - 0.01 && body.x.abs() < 0.05,
                "no tunnel/explosion: {body:?}"
            );
        }
    }
}
