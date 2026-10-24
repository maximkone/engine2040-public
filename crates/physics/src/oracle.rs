//! oracle: the published Half-Life movement model as an offline f64 reference.
//!
//! Source read as a spec, never copied: `ValveSoftware/halflife`
//! `pm_shared/pm_shared.c` `PM_Accelerate` :990, `PM_Friction` :1202,
//! `PM_AirAccelerate` :1279, `PM_Jump` :2474 and
//! `PM_PreventMegaBunnyJumping` :2444, with every law pinned in
//! `docs/research/blendworks-2026-10-23/README.md`. Frame order follows
//! `PM_PlayerMove` for a walking player on flat ground: half of the frame's
//! gravity, the jump, ground friction, ground or air acceleration, the move,
//! then the other half of gravity.
//!
//! The engine cvars (`maxspeed`, `accelerate`, `airaccelerate`, `friction`,
//! `stopspeed`, `edgefriction`) are not in the SDK, so they are fields of
//! `Params`, not constants of the law: a reader runs another server's table
//! without an edit. `Params::hl1` is the fixture's table.
//!
//! Units are HL units and seconds inside; `Params::unit_m` converts to
//! meters. The SDK's own jump line (`sqrt(2 * 800 * 45.0)`) pins gravity at
//! 800 u/s^2, which at the engine's inch-per-unit is 20.32 m/s^2, twice our
//! `GRAVITY_Y`. The fixture therefore picks `unit_m = |GRAVITY_Y| / 800` and
//! both models run the same acceleration, so the curves compare as curves; in
//! this tape one unit is 12.26 mm and the oracle's 72-unit hull is 0.88 m
//! against our 1.40 m capsule.
//!
//! Never in the tick: this module is compiled for the `rapier` lane (the lane
//! `super::controller` lives in), has no caller in the sim, and writes no
//! state the tick or the hash reads.
#![forbid(unsafe_code)]

use super::rapier::GRAVITY_Y;

/// `PM_Friction`'s low-speed early-out, units/s: under it the residual speed
/// is left standing (the header's `STOP_EPSILON`).
const FRICTION_FLOOR_U_S: f64 = 0.1;

/// `PM_AirAccelerate`'s addspeed cap, units/s. Only the addspeed test is
/// capped; `accelspeed` uses the full wishspeed, and that asymmetry is the
/// strafe gain.
const AIR_ADDSPEED_CAP_U_S: f64 = 30.0;

/// `PM_PreventMegaBunnyJumping`'s `BUNNYJUMP_MAX_SPEED_FACTOR`.
const BUNNYJUMP_MAX_SPEED_FACTOR: f64 = 1.7;

/// The crop a jump past the cap applies, `0.65` in the source.
const BUNNYJUMP_CROP: f64 = 0.65;

/// One server's move table: the SDK's published constants plus the engine
/// cvars the SDK does not ship. Units are HL units and seconds.
#[derive(Clone, Copy, Debug)]
pub struct Params {
    /// Move speed cap, units/s (`sv_maxspeed`; engine cvar, fixture value).
    pub maxspeed_u_s: f64,
    /// Ground acceleration, 1/s (`sv_accelerate`; engine cvar).
    pub accelerate: f64,
    /// Air acceleration, 1/s (`sv_airaccelerate`; engine cvar).
    pub airaccelerate: f64,
    /// Ground friction, 1/s (`sv_friction`; engine cvar).
    pub friction: f64,
    /// Speed under which friction bleeds the cap instead, units/s
    /// (`sv_stopspeed`; engine cvar).
    pub stopspeed_u_s: f64,
    /// Multiplier on friction when the ground trace ahead finds nothing
    /// (`sv_edgefriction`; engine cvar). A flat tape never reads it.
    pub edgefriction: f64,
    /// The player's own friction scale (`pmove->friction`, `1.0` for a player).
    pub player_friction: f64,
    /// Gravity, units/s^2, pinned by `PM_Jump`'s `sqrt(2 * 800 * 45.0)`.
    pub gravity_u_s2: f64,
    /// Take-off speed, units/s.
    pub jump_speed_u_s: f64,
    /// Meters per HL unit for the comparison.
    pub unit_m: f64,
}

impl Params {
    /// The fixture's table: the SDK's published numbers plus a default HL1
    /// listen server's move cvars. The cvars are fixture values (the SDK does
    /// not ship them), so nothing here is claimed as an engine fact.
    #[must_use]
    pub fn hl1() -> Self {
        let gravity_u_s2 = 800.0;
        Self {
            maxspeed_u_s: 320.0,
            accelerate: 10.0,
            airaccelerate: 10.0,
            friction: 4.0,
            stopspeed_u_s: 100.0,
            edgefriction: 2.0,
            player_friction: 1.0,
            gravity_u_s2,
            // PM_Jump's two standing branches both take 45 units; only the
            // ducking super long jump takes 56.
            jump_speed_u_s: (2.0 * gravity_u_s2 * 45.0).sqrt(),
            // Hold acceleration equal to our controller's gravity.
            unit_m: -f64::from(GRAVITY_Y) / gravity_u_s2,
        }
    }

    /// Apex of a jump of [`Params::jump_speed_u_s`], HL units: `v^2 / (2 g)`,
    /// which the published pair `sqrt(2 * 800 * 45.0)` makes exactly 45.
    #[must_use]
    pub fn jump_apex_u(&self) -> f64 {
        self.jump_speed_u_s * self.jump_speed_u_s / (2.0 * self.gravity_u_s2)
    }

    /// The bunnyhop crop cap, `1.7 * maxspeed`, units/s.
    #[must_use]
    pub fn bunnyhop_cap_u_s(&self) -> f64 {
        BUNNYJUMP_MAX_SPEED_FACTOR * self.maxspeed_u_s
    }

    /// HL units to meters.
    #[must_use]
    pub fn meters(&self, units: f64) -> f64 {
        units * self.unit_m
    }
}

/// The reference mover: one walking player on flat ground in the HL frame
/// (x and y horizontal, z up, floor at `z == 0`).
#[derive(Debug)]
pub struct HlMove {
    params: Params,
    /// Velocity, units/s.
    vel: [f64; 3],
    /// Origin, units; the floor top is `z == 0`.
    origin: [f64; 3],
    /// Grounded after the last frame.
    on_ground: bool,
    /// The jump button's edge state: a held button never jumps twice
    /// (`oldbuttons & IN_JUMP`).
    jump_held: bool,
}

impl HlMove {
    /// Spawn standing on the floor at the origin, at rest.
    #[must_use]
    pub fn new(params: Params) -> Self {
        Self {
            params,
            vel: [0.0; 3],
            origin: [0.0; 3],
            on_ground: true,
            jump_held: false,
        }
    }

    /// Horizontal speed, units/s.
    #[must_use]
    pub fn speed_u_s(&self) -> f64 {
        (self.vel[0] * self.vel[0] + self.vel[1] * self.vel[1]).sqrt()
    }

    /// Height above the floor, units.
    #[must_use]
    pub fn height_u(&self) -> f64 {
        self.origin[2]
    }

    /// Vertical speed, units/s.
    #[must_use]
    pub fn rise_u_s(&self) -> f64 {
        self.vel[2]
    }

    /// Grounded flag after the last frame.
    #[must_use]
    pub fn on_ground(&self) -> bool {
        self.on_ground
    }

    /// One frame of `PM_PlayerMove` on flat ground. `wish` is the stick as a
    /// world-frame direction whose length is the stick fraction (1 is full
    /// forward); `jump` is the button's state this frame, and only its press
    /// edge takes off.
    pub fn tick(&mut self, wish: [f64; 2], jump: bool, dt: f64) {
        let p = self.params;
        // PM_AddCorrectGravity: half the frame's gravity before the move.
        self.vel[2] -= p.gravity_u_s2 * 0.5 * dt;
        if jump && !self.jump_held && self.on_ground {
            // PM_Jump: airborne first, then the crop, then the take-off.
            self.on_ground = false;
            self.crop_bunnyhop();
            self.vel[2] = p.jump_speed_u_s;
            // PM_Jump ends with PM_FixupGravityVelocity.
            self.vel[2] -= p.gravity_u_s2 * 0.5 * dt;
        }
        self.jump_held = jump;
        // PM_PlayerMove: friction only while grounded, with the vertical speed
        // zeroed first. A jump frame is already airborne and skips it, which is
        // the bunnyhop route the crop above guards.
        if self.on_ground {
            self.vel[2] = 0.0;
            self.friction(dt);
        }
        let (dir, wishspeed) = self.wish(wish);
        if self.on_ground {
            self.accelerate(dir, wishspeed, p.accelerate, dt);
        } else {
            self.air_accelerate(dir, wishspeed, p.airaccelerate, dt);
        }
        // PM_FlyMove on open flat ground: no clip, so the whole velocity
        // applies for the frame.
        self.origin[0] += self.vel[0] * dt;
        self.origin[1] += self.vel[1] * dt;
        self.origin[2] += self.vel[2] * dt;
        if self.origin[2] <= 0.0 {
            // PM_CatagorizePosition: the floor hold, and a landing eats the
            // fall speed.
            self.origin[2] = 0.0;
            if self.vel[2] < 0.0 {
                self.vel[2] = 0.0;
            }
            self.on_ground = true;
        }
        // PM_FixupGravityVelocity, then the grounded clamp.
        self.vel[2] -= p.gravity_u_s2 * 0.5 * dt;
        if self.on_ground {
            self.vel[2] = 0.0;
        }
    }

    /// Total speed, units/s.
    fn speed3(&self) -> f64 {
        (self.vel[0] * self.vel[0] + self.vel[1] * self.vel[1] + self.vel[2] * self.vel[2]).sqrt()
    }

    /// Wish direction plus wish speed: the stick's length is a fraction of
    /// `maxspeed`, and the server clamps the result to `maxspeed`.
    fn wish(&self, wish: [f64; 2]) -> ([f64; 2], f64) {
        let len = (wish[0] * wish[0] + wish[1] * wish[1]).sqrt();
        if len <= 0.0 {
            return ([0.0, 0.0], 0.0);
        }
        let dir = [wish[0] / len, wish[1] / len];
        let speed = (len * self.params.maxspeed_u_s).min(self.params.maxspeed_u_s);
        (dir, speed)
    }

    /// PM_Friction: bleed `max(speed, stopspeed) * friction * dt`, scaled onto
    /// the whole velocity and floored at zero. Edge friction only applies when
    /// the trace ahead leaves the floor, which a flat tape never does.
    fn friction(&mut self, dt: f64) {
        let speed = self.speed3();
        if speed < FRICTION_FLOOR_U_S {
            return;
        }
        let control = speed.max(self.params.stopspeed_u_s);
        let drop = control * self.params.friction * self.params.player_friction * dt;
        let scale = (speed - drop).max(0.0) / speed;
        self.vel[0] *= scale;
        self.vel[1] *= scale;
        self.vel[2] *= scale;
    }

    /// PM_Accelerate: add `accel * dt * wishspeed * friction`, capped at the
    /// remaining speed along the wish direction.
    fn accelerate(&mut self, dir: [f64; 2], wishspeed: f64, accel: f64, dt: f64) {
        let current = self.vel[0] * dir[0] + self.vel[1] * dir[1];
        let addspeed = wishspeed - current;
        if addspeed <= 0.0 {
            return;
        }
        let gained = (accel * dt * wishspeed * self.params.player_friction).min(addspeed);
        self.vel[0] += gained * dir[0];
        self.vel[1] += gained * dir[1];
    }

    /// PM_AirAccelerate: the addspeed test is capped at 30 u/s, the gain is
    /// not, so a perpendicular wish keeps adding speed every frame.
    fn air_accelerate(&mut self, dir: [f64; 2], wishspeed: f64, accel: f64, dt: f64) {
        let current = self.vel[0] * dir[0] + self.vel[1] * dir[1];
        let addspeed = wishspeed.min(AIR_ADDSPEED_CAP_U_S) - current;
        if addspeed <= 0.0 {
            return;
        }
        let gained = (accel * wishspeed * dt * self.params.player_friction).min(addspeed);
        self.vel[0] += gained * dir[0];
        self.vel[1] += gained * dir[1];
    }

    /// PM_PreventMegaBunnyJumping: past `1.7 * maxspeed` a jump crops the whole
    /// velocity to `(cap / speed) * 0.65`.
    fn crop_bunnyhop(&mut self) {
        let cap = self.params.bunnyhop_cap_u_s();
        if cap <= 0.0 {
            return;
        }
        let speed = self.speed3();
        if speed <= cap {
            return;
        }
        let scale = (cap / speed) * BUNNYJUMP_CROP;
        self.vel[0] *= scale;
        self.vel[1] *= scale;
        self.vel[2] *= scale;
    }
}
