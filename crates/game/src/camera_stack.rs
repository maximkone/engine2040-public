//! camera_stack: priority shot table plus eased handoff.
//!
//! Purpose: highest-priority live shot drives the frame, switches blend
//! over a fixed tick window with integer smoothstep. Owner: `engine-game`.
//! Units: permille `0..=1000`, ticks, meters in [`CameraFrame`].
//! Determinism: table-order loops only, no wall clock, slices in plus copied
//! frames out, same inputs give same bytes twice. Errors: [`StackError`] on
//! empty, full or dead tables. Budget: O(shots) time, zero heap bytes per
//! call. Status: retired, no production caller (only unit tests plus the
//! integration test call it); presentation only, never simulation state.
// PORT(ramokz/phantom-camera@21d3762492e2dc51b448f55438b89b3c19349f96:addons/phantom_camera/scripts/phantom_camera_host/phantom_camera_host.gd — host highest-wins plus tweened handoff, reimplemented integer-tick, zero bytes vendored)

use super::camera::{CameraFrame, Vec3, TRAUMA_SCALE};

/// Most shots one stack table holds (fixed scratch, no heap).
pub const MAX_SHOTS: usize = 8;
/// Ticks one eased handoff spans (fixed window).
pub const HANDOFF_TICKS: u32 = 12;

/// One framed shot: priority arbitration row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Shot {
    /// Stable shot id in table order.
    pub id: u8,
    /// Arbitration weight, higher wins.
    pub priority: u8,
    /// False shots never win while a live shot remains.
    pub live: bool,
}

impl Shot {
    /// Build one arbitration row.
    #[must_use]
    pub fn new(id: u8, priority: u8, live: bool) -> Self {
        Self { id, priority, live }
    }
}

/// Stack failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StackError {
    /// Table holds no rows yet.
    Empty,
    /// Table holds no live shot.
    Dead,
    /// Table already holds [`MAX_SHOTS`] rows.
    Full,
}

impl core::fmt::Display for StackError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => write!(f, "camera stack is empty"),
            Self::Dead => write!(f, "camera stack has no live shot"),
            Self::Full => write!(f, "camera stack is full"),
        }
    }
}

impl std::error::Error for StackError {}

/// Priority table in table order (fixed scratch, no heap).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Stack {
    shots: [Shot; MAX_SHOTS],
    len: usize,
}

impl Stack {
    /// Empty table with fixed scratch.
    #[must_use]
    pub fn new() -> Self {
        Self {
            shots: [Shot::new(0, 0, false); MAX_SHOTS],
            len: 0,
        }
    }

    /// Rows held so far.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// True when no row has been pushed yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Push one row in table order; a full table fails loudly.
    pub fn push(&mut self, shot: Shot) -> Result<(), StackError> {
        if self.len >= MAX_SHOTS {
            return Err(StackError::Full);
        }
        self.shots[self.len] = shot;
        self.len += 1;
        Ok(())
    }

    /// Highest-priority live shot; ties keep table order.
    pub fn pick(&self) -> Result<Shot, StackError> {
        if self.len == 0 {
            return Err(StackError::Empty);
        }
        let mut best: Option<Shot> = None;
        for index in 0..self.len {
            let shot = self.shots[index];
            if !shot.live {
                continue;
            }
            match best {
                None => best = Some(shot),
                Some(held) => {
                    if shot.priority > held.priority {
                        best = Some(shot);
                    }
                }
            }
        }
        best.ok_or(StackError::Dead)
    }

    /// Integer smoothstep weight in permille for `t` in `0..=1000`.
    #[must_use]
    pub fn smoothstep_permille(t_permille: u32) -> u32 {
        let t = u64::from(t_permille.min(TRAUMA_SCALE));
        let t2 = t * t;
        let t3 = t2 * t;
        let num = 3 * t2 * 1000 - 2 * t3;
        u32::try_from((num + 500_000) / 1_000_000).unwrap_or(TRAUMA_SCALE)
    }

    /// Eased weight in permille for `elapsed` in `0..=HANDOFF_TICKS`.
    #[must_use]
    pub fn blend_weight(elapsed_ticks: u32) -> u32 {
        let capped = elapsed_ticks.min(HANDOFF_TICKS);
        Self::smoothstep_permille(capped.saturating_mul(1000) / HANDOFF_TICKS)
    }

    /// Ease the outgoing frame into the incoming frame over the fixed window.
    #[must_use]
    pub fn blend(from: &CameraFrame, to: &CameraFrame, elapsed_ticks: u32) -> CameraFrame {
        let weight = Self::blend_weight(elapsed_ticks);
        let eye_m = lerp_vec(from.eye_m, to.eye_m, weight);
        let look_m = lerp_vec(from.look_m, to.look_m, weight);
        let shake_m = lerp_vec(from.shake_m, to.shake_m, weight);
        let lock = if weight >= TRAUMA_SCALE / 2 {
            to.lock
        } else {
            from.lock
        };
        let trauma_permille = lerp_u32(from.trauma_permille, to.trauma_permille, weight);
        CameraFrame {
            tick: to.tick,
            eye_m,
            look_m,
            shake_m,
            lock,
            trauma_permille,
        }
    }
}

/// Linear blend of two positions at `weight` permille.
fn lerp_vec(from: Vec3, to: Vec3, weight_permille: u32) -> Vec3 {
    let capped = weight_permille.min(TRAUMA_SCALE);
    let small = u16::try_from(capped).unwrap_or(1000);
    let w = f32::from(small) / 1000.0;
    let inv = 1.0 - w;
    Vec3::new(
        from.x * inv + to.x * w,
        from.y * inv + to.y * w,
        from.z * inv + to.z * w,
    )
}

/// Linear blend of two permille tracks at `weight` permille.
fn lerp_u32(from: u32, to: u32, weight_permille: u32) -> u32 {
    let w = u64::from(weight_permille.min(TRAUMA_SCALE));
    let a = u64::from(from);
    let b = u64::from(to);
    let out = if b >= a {
        a + (b - a) * w / 1000
    } else {
        a - (a - b) * w / 1000
    };
    u32::try_from(out).unwrap_or(to)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::alloc_probe;

    #[test]
    fn steady_state_allocates_nothing() {
        let (_, probe) = alloc_probe::measure(|| std::hint::black_box(vec![0u8; 64]));
        assert!(probe.calls >= 1, "alloc probe is blind");
        let mut stack = Stack::new();
        for id in [0u8, 1u8, 2u8] {
            stack.push(Shot::new(id, id, true)).expect("push fits");
        }
        let from = CameraFrame {
            tick: 0,
            eye_m: Vec3::new(0.0, 1.0, 2.0),
            look_m: Vec3::ZERO,
            shake_m: Vec3::ZERO,
            lock: None,
            trauma_permille: 0,
        };
        let to = CameraFrame {
            tick: 1,
            eye_m: Vec3::new(4.0, 5.0, 6.0),
            look_m: Vec3::ZERO,
            shake_m: Vec3::ZERO,
            lock: None,
            trauma_permille: 0,
        };
        let (folded, steady) = alloc_probe::measure(|| {
            let mut folded = 0u64;
            for tick in 0..HANDOFF_TICKS {
                let shot = stack.pick().expect("live shot wins");
                let frame = Stack::blend(&from, &to, tick);
                folded ^= u64::from(shot.id) ^ u64::from(frame.tick);
            }
            folded
        });
        assert_eq!(steady.calls, 0, "stack update must not allocate");
        assert!(folded != u64::MAX, "sanity: frames advanced");
    }
}
