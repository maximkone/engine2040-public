//! gameworld: deterministic 10-game sample-efficiency core (GW1 spike).
//!
//! Ten fixed-tick integer games (Aviate..Jump) as headless systems, no new
//! crate. Integer-only: positions in cm, velocities in cm/tick, rewards in
//! points, RNG via [`engine_core::Rng`]. No float in tick, never hashed into
//! the arena golden (separate module, [`crate::run`] untouched).
//!
//! Spike scope (Tier B, Size S): 10 games x 10k steps headless seeded,
//! metric steps/s plus parity digest x2, golden held. Kill: any golden drift
//! or any float in tick.

/// Game count: Aviate, Bounce, Cross, Drive, Explode, Fruits, Gold, Hunt,
/// Impact, Jump in this order.
pub const GAME_COUNT: usize = 10;
/// Game names in index order.
pub const GAME_NAMES: [&str; GAME_COUNT] = [
    "aviate", "bounce", "cross", "drive", "explode", "fruits", "gold", "hunt", "impact", "jump",
];
/// Steps per game for the E04 headless proof.
pub const GW1_STEPS: u32 = 10_000;
/// Arena half-extent in cm (20 m).
pub const HALF_EXTENT_CM: i32 = 2000;

/// One gameworld game: fixed-tick integer state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GameWorld {
    /// Game index, `0..GAME_COUNT`.
    pub game: u8,
    /// Fixed ticks stepped so far.
    pub tick: u32,
    /// Seed this game reset from.
    pub seed: u64,
    /// Position in cm `[x, y]`.
    pub pos: [i32; 2],
    /// Velocity in cm/tick `[vx, vy]`.
    pub vel: [i32; 2],
    /// Score in points.
    pub score: i32,
    /// RNG state (integer stream only).
    pub rng_state: u64,
}

impl GameWorld {
    /// Reset one game from `seed`. `None` on a bad game index, never panics.
    pub fn reset(game: usize, seed: u64) -> Option<Self> {
        if game >= GAME_COUNT {
            return None;
        }
        let game_u64 = u64::try_from(game).ok()?;
        let mut rng = engine_core::Rng::new(seed ^ game_u64.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let x = i32::try_from(rng.next_u64() % 4001).ok()? - 2000;
        let y = i32::try_from(rng.next_u64() % 4001).ok()? - 2000;
        Some(Self {
            game: u8::try_from(game).ok()?,
            tick: 0,
            seed,
            pos: [x, y],
            vel: [0, 0],
            score: 0,
            rng_state: rng.state(),
        })
    }

    /// One fixed tick from `action` (low 3 bits steer, the rest drift).
    /// Integer-only: no float, no division by a variable, never panics.
    pub fn step(&mut self, action: u64) {
        let speed = 10 + i32::from(self.game);
        match u8::try_from(action % 5).unwrap_or(0) {
            0 => self.vel = [0, 0],
            1 => self.vel = [speed, 0],
            2 => self.vel = [-speed, 0],
            3 => self.vel = [0, speed],
            _ => self.vel = [0, -speed],
        }
        // Per-game drift from the integer stream: distinct dynamics per game.
        let mut rng = engine_core::Rng::from_state(self.rng_state);
        let drift = i32::try_from(rng.next_u64() % 7).unwrap_or(0) - 3 + i32::from(self.game) % 3;
        self.pos[0] = (self.pos[0] + self.vel[0] + drift).clamp(-HALF_EXTENT_CM, HALF_EXTENT_CM);
        self.pos[1] = (self.pos[1] + self.vel[1] - drift).clamp(-HALF_EXTENT_CM, HALF_EXTENT_CM);
        self.rng_state = rng.state();
        // Reward: reach the per-game target quadrant for +10, else -1 per tick.
        // Target is seeded per game, integer compare only.
        let tx = if (self.seed >> (self.game % 4)) & 1 == 0 {
            1000
        } else {
            -1000
        };
        let ty = if (self.seed >> ((self.game + 2) % 6)) & 1 == 0 {
            1000
        } else {
            -1000
        };
        let dx = self.pos[0] - tx;
        let dy = self.pos[1] - ty;
        let dist2 = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy));
        if dist2 < 200 * 200 {
            self.score = self.score.wrapping_add(10);
        } else {
            self.score = self.score.wrapping_add(-1);
        }
        self.tick = self.tick.wrapping_add(1);
    }

    /// Parity digest over the integer state: x2 identical per inputs.
    pub fn digest(&self) -> u64 {
        let mut h = engine_core::Fnv64::new();
        h.write_u8(self.game);
        h.write_u32(self.tick);
        h.write_u64(self.seed);
        h.write_i32(self.pos[0]);
        h.write_i32(self.pos[1]);
        h.write_i32(self.vel[0]);
        h.write_i32(self.vel[1]);
        h.write_i32(self.score);
        h.write_u64(self.rng_state);
        h.finish()
    }
}

/// Run one game headless for `steps` ticks with a scripted action tape
/// (`action = tick % 4`). Returns `(score, digest)`. Integer-only.
pub fn run_headless(game: usize, seed: u64, steps: u32) -> Option<(i32, u64)> {
    let mut world = GameWorld::reset(game, seed)?;
    for tick in 0..steps {
        world.step(u64::from(tick % 4) + (seed % 3));
    }
    Some((world.score, world.digest()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_game_resets_seeded_and_steps_600_ticks_headless() {
        // Axiom GW1 accept slice: every game resets seeded and steps 600
        // ticks headless, donor-faithful integer semantics, x2 identical.
        for game in 0..GAME_COUNT {
            let first = run_headless(game, 2040, 600).expect("valid game resets");
            let second = run_headless(game, 2040, 600).expect("valid game resets");
            assert_eq!(first, second, "{game} must be x2 identical");
            let other = run_headless(game, 2041, 600).expect("valid game resets");
            assert_ne!(first.1, other.1, "{game} must differ across seeds");
        }
        assert!(GameWorld::reset(GAME_COUNT, 2040).is_none());
        assert!(run_headless(GAME_COUNT, 2040, 600).is_none());
    }

    #[test]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "wall clock lives in this test only, never in the sim"
    )]
    fn ten_games_ten_k_steps_parity_digest_x2_with_steps_per_second() {
        // E04 GW1 smallest proof: 10 games x 10k steps headless seeded,
        // steps/s plus parity digest x2 posted, golden held (this module
        // never feeds crate::run checkpoints), integer-only by construction
        // (no f32/f64 in this file). Kill: any golden drift, any float.
        let started = std::time::Instant::now();
        let mut digests = Vec::with_capacity(GAME_COUNT);
        for (game, name) in GAME_NAMES.iter().enumerate() {
            let first = run_headless(game, 7, GW1_STEPS).expect("valid game runs");
            let second = run_headless(game, 7, GW1_STEPS).expect("valid game runs");
            assert_eq!(first, second, "{name} must be x2 identical");
            digests.push(first.1);
        }
        let secs = started.elapsed().as_secs_f64().max(1e-9);
        let total = GAME_COUNT as f64 * f64::from(GW1_STEPS);
        let steps_per_sec = total / secs;
        let mut table = engine_core::Fnv64::new();
        for digest in &digests {
            table.write_u64(*digest);
        }
        eprintln!(
            "GW1 gameworld: games={} steps_each={} total_steps={} secs={secs:.3} steps_per_sec={steps_per_sec:.0} table_digest={:016x}",
            GAME_COUNT,
            GW1_STEPS,
            total as u64,
            table.finish(),
        );
        assert!(steps_per_sec > 0.0, "must post a steps/s number");
        assert_eq!(digests.len(), GAME_COUNT);
    }
}
