//! fps_move: integer Quake walk, sim side. mm, mm/s at 60Hz, no float/trig, clamped.
#![forbid(unsafe_code)]
const TICKS_PER_S: i32 = 60;
const WALK_MM_S: i32 = 9000;
const RUN_MM_S: i32 = 14000;
const CROUCHED_MM_S: i32 = 5000;
const AIR_CAP_MM_S: i32 = 2000;
const ACCEL_GROUND_Q: i32 = 10000;
const ACCEL_AIR_Q: i32 = 20000;
const FRICTION: i32 = 10;
const STOP_MM_S: i32 = 1000;
const FRICTION_CUTOFF_MM_S: i32 = 100;
const TRACTION_Q: i32 = 700;
const JUMP_MM_S: i32 = 8500;
const GRAVITY_MM_S2: i32 = 23000;
const UPRIGHT_MM: i32 = 3000;
const CROUCH_MM: i32 = 1500;
const CROUCH_RATE_MM_S: i32 = 6000;
const UNCROUCH_RATE_MM_S: i32 = 8000;
const FLY_MM_S: i32 = 10000;
const FAST_FLY_MM_S: i32 = 30000;
const FLY_FRICTION_Q: i32 = 500;
/// Sim move mode: grounded walk or free fly.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MoveMode {
    Ground,
    Noclip,
}
/// Per-tick input; wish is world-space XZ in mm/s, yaw-rotated by caller.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FpsInput {
    pub wish_mm_s: [i32; 2],
    pub sprint: bool,
    pub jump_pressed: bool,
    pub crouch: bool,
    pub fly_toggle: bool,
}
/// Walker state: feet pos mm, vel mm/s, height mm and ground flags.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FpsWalker {
    pub pos_mm: [i32; 3],
    pub vel_mm_s: [i32; 3],
    pub height_mm: i32,
    pub grounded: bool,
    pub mode: MoveMode,
}
impl FpsWalker {
    /// Spawn standing at feet pos with full height.
    pub fn spawn(pos_mm: [i32; 3]) -> Self {
        Self {
            pos_mm,
            vel_mm_s: [0, 0, 0],
            height_mm: UPRIGHT_MM,
            grounded: false,
            mode: MoveMode::Ground,
        }
    }
}
fn narrow(base: i32, delta: i64) -> i32 {
    let sum = i64::from(base) + delta;
    match i32::try_from(sum) {
        Ok(kept) => kept,
        Err(_) => {
            if sum < 0 {
                i32::MIN
            } else {
                i32::MAX
            }
        }
    }
}
fn accelerate(vel: &mut [i32; 3], wish: [i32; 2], accel_q: i32) {
    for (axis, want) in wish.into_iter().enumerate() {
        let lane = if axis == 0 { &mut vel[0] } else { &mut vel[2] };
        let raw = i64::from(accel_q) * i64::from(want) / 1000 / i64::from(TICKS_PER_S);
        let clamped = raw.clamp(-8000, 8000);
        let cap = i32::try_from(clamped).unwrap_or(if raw < 0 { i32::MIN } else { i32::MAX });
        *lane += (want - *lane).clamp(-cap.abs(), cap.abs());
    }
}
fn friction(vel: &mut [i32; 3]) {
    for lane in [0, 2] {
        let v = vel[lane];
        if v.abs() <= FRICTION_CUTOFF_MM_S {
            continue;
        }
        let drop = v.abs().max(STOP_MM_S) * FRICTION / TICKS_PER_S;
        let kept = (v.abs() - drop).max(0);
        vel[lane] = if v > 0 { kept } else { -kept };
    }
}
/// Advance one 1/60 s tick on world-space wish; traction_q is dot(normal, up) permille.
pub fn step_move(
    w: &mut FpsWalker,
    input: &FpsInput,
    traction_q: i32,
    support_mm: Option<i32>,
) -> [i32; 3] {
    let from = w.pos_mm;
    if input.fly_toggle {
        w.mode = if w.mode == MoveMode::Ground {
            MoveMode::Noclip
        } else {
            MoveMode::Ground
        };
    }
    let rate = if input.crouch {
        -CROUCH_RATE_MM_S
    } else {
        UNCROUCH_RATE_MM_S
    };
    w.height_mm = (w.height_mm + rate / TICKS_PER_S).clamp(CROUCH_MM, UPRIGHT_MM);
    if w.mode == MoveMode::Noclip {
        let speed = if input.sprint {
            FAST_FLY_MM_S
        } else {
            FLY_MM_S
        };
        if input.wish_mm_s == [0, 0] {
            let keep = 1000 - FLY_FRICTION_Q.clamp(0, 1000);
            w.vel_mm_s[0] = w.vel_mm_s[0] * keep / 1000;
            w.vel_mm_s[1] = w.vel_mm_s[1] * keep / 1000;
            w.vel_mm_s[2] = w.vel_mm_s[2] * keep / 1000;
        } else {
            w.vel_mm_s[0] = input.wish_mm_s[0].clamp(-speed, speed);
            w.vel_mm_s[2] = input.wish_mm_s[1].clamp(-speed, speed);
            w.vel_mm_s[1] = 0;
        }
        w.grounded = false;
    } else if support_mm.is_some() && traction_q > TRACTION_Q {
        w.grounded = true;
        friction(&mut w.vel_mm_s);
        let max = if input.crouch {
            CROUCHED_MM_S
        } else if input.sprint {
            RUN_MM_S
        } else {
            WALK_MM_S
        };
        accelerate(
            &mut w.vel_mm_s,
            [
                input.wish_mm_s[0].clamp(-max, max),
                input.wish_mm_s[1].clamp(-max, max),
            ],
            ACCEL_GROUND_Q,
        );
        if input.jump_pressed {
            w.vel_mm_s[1] = JUMP_MM_S;
            w.grounded = false;
        } else {
            w.vel_mm_s[1] = 0;
        }
    } else {
        w.grounded = false;
        accelerate(
            &mut w.vel_mm_s,
            [
                input.wish_mm_s[0].clamp(-AIR_CAP_MM_S, AIR_CAP_MM_S),
                input.wish_mm_s[1].clamp(-AIR_CAP_MM_S, AIR_CAP_MM_S),
            ],
            ACCEL_AIR_Q,
        );
        w.vel_mm_s[1] -= GRAVITY_MM_S2 / TICKS_PER_S;
    }
    w.pos_mm[0] = narrow(
        w.pos_mm[0],
        i64::from(w.vel_mm_s[0]) / i64::from(TICKS_PER_S),
    );
    w.pos_mm[1] = narrow(
        w.pos_mm[1],
        i64::from(w.vel_mm_s[1]) / i64::from(TICKS_PER_S),
    );
    w.pos_mm[2] = narrow(
        w.pos_mm[2],
        i64::from(w.vel_mm_s[2]) / i64::from(TICKS_PER_S),
    );
    [
        w.pos_mm[0] - from[0],
        w.pos_mm[1] - from[1],
        w.pos_mm[2] - from[2],
    ]
}
#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::Fnv64;
    fn digest(w: &FpsWalker) -> u64 {
        let mut h = Fnv64::new();
        for v in w.pos_mm {
            h.write_i32(v);
        }
        for v in w.vel_mm_s {
            h.write_i32(v);
        }
        h.write_i32(w.height_mm);
        h.write_bool(w.grounded);
        h.finish()
    }
    #[test]
    fn fps_walk_moves_and_friction_bites() {
        let push = FpsInput {
            wish_mm_s: [WALK_MM_S, 0],
            sprint: false,
            jump_pressed: false,
            crouch: false,
            fly_toggle: false,
        };
        let idle = FpsInput {
            wish_mm_s: [0, 0],
            ..push
        };
        let mut a = FpsWalker::spawn([0, 0, 0]);
        let d1 = step_move(&mut a, &push, 1000, Some(0));
        let d2 = step_move(&mut a, &idle, 1000, Some(0));
        assert!(d1[0] > d2[0], "friction did not bite: {d1:?} vs {d2:?}");
        assert!(a.grounded, "grounded lost: {a:?}");
        let run = |fly: bool| {
            let mut w = FpsWalker::spawn([0, 0, 0]);
            let mut inp = push;
            inp.fly_toggle = fly;
            for _ in 0..8 {
                step_move(&mut w, &inp, 1000, Some(0));
                inp.fly_toggle = false;
            }
            digest(&w)
        };
        assert_eq!(run(false), run(false), "replay diverged");
    }
}
