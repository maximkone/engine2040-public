//! crowd_gpu: reacting crowd integer sim feeding the instanced stands.
//!
//! Owner: game lane. Units: excitement in permille (`0..=1000`), ticks at
//! the fixed arena rate, seats as counts. Determinism scope: same seed
//! plus same cue script plus same tick cadence gives identical levels and
//! hashes; the state is presentation only and never enters the golden
//! hash. Errors are typed [`CrowdGpuError`]. Budget: O(64) integer ops per
//! tick over a fixed array with no allocation. Caller:
//! `crates/game/src/lib.rs` (`App` owns one sim and drives it once per
//! accepted tick beside the audio crowd clock).

use engine_audio::FightCue;
use engine_core::Fnv64;
use std::fmt;

// ---------------------------------------------------------------------------
// Bounds: the operating crowd and its integer excitement scale.
// ---------------------------------------------------------------------------

/// Most seats the sim distributes: the FOCUS 20k-50k top.
pub const MAX_CROWD_SEATS: u32 = 50_000;
/// Production seats: the rich floor, 50 rows by 100 cols over 4 sectors.
pub const RICH_CROWD_SEATS: u32 = 20_000;
/// Chant blocks over the room; matches the render 4x4 sector grid.
pub const CROWD_BLOCKS: usize = 64;
/// Chant blocks as `u8`: cursor and excitement indices stay in this width,
/// so loops and remainders never narrow with `as`.
pub const CROWD_BLOCKS_U8: u8 = 64;
/// Chant blocks as `u32`: seat-share division stays in this width.
pub const CROWD_BLOCKS_U32: u32 = 64;
/// Resting murmur every block decays toward, in permille.
pub const BASELINE_PERMILLE: u16 = 300;
/// Ceiling for every block, in permille.
pub const PEAK_PERMILLE: u16 = 1000;
/// Linear decay per tick toward the baseline, in permille.
pub const DECAY_PER_TICK: u16 = 2;
/// Excitement at or above which the wave clock runs, in permille.
pub const WAVE_THRESHOLD_PERMILLE: u16 = 700;
/// Ticks between wave-front steps while the room stays loud.
pub const WAVE_PERIOD_TICKS: u64 = 12;
/// Excitement the wave front adds to its block, in permille.
pub const WAVE_BUMP_PERMILLE: u16 = 50;

// ---------------------------------------------------------------------------
// Errors: typed, no allocation; bad input fails loudly.
// ---------------------------------------------------------------------------

/// Crowd sim failures; every variant names what to fix.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CrowdGpuError {
    /// Seat count outside `1..=MAX_CROWD_SEATS`; request carried.
    BadSeats {
        /// Seats requested.
        seats: u32,
        /// Seats the cap allows.
        max: u32,
    },
    /// A caller buffer is shorter than the room; sizes carried.
    BadBuffer {
        /// Entries the room needs.
        want: usize,
        /// Entries the caller buffer holds.
        got: usize,
    },
    /// The camera is unusable; reason carried.
    BadCamera(&'static str),
    /// The render stage refused the frame; reason carried.
    Staging(engine_render::crowd_inst::CrowdInstError),
}

impl fmt::Display for CrowdGpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadSeats { seats, max } => {
                write!(f, "bad crowd seats ({seats}, want 1..={max})")
            }
            Self::BadBuffer { want, got } => {
                write!(f, "crowd buffer holds {got}, room needs {want}")
            }
            Self::BadCamera(why) => write!(f, "bad crowd camera ({why})"),
            Self::Staging(err) => write!(f, "crowd stage refused ({err})"),
        }
    }
}

impl std::error::Error for CrowdGpuError {}

impl From<engine_render::crowd_inst::CrowdInstError> for CrowdGpuError {
    fn from(err: engine_render::crowd_inst::CrowdInstError) -> Self {
        Self::Staging(err)
    }
}

// ---------------------------------------------------------------------------
// Sim: 64 integer excitement blocks plus a wave cursor.
// ---------------------------------------------------------------------------

/// Reacting crowd state: one permille excitement entry per chant block,
/// a fixed-tick clock and a Mexican-wave cursor. The whole room is a
/// fixed array, so ticks never grow, hash or allocate.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CrowdGpuSim {
    seed: u64,
    seats: u32,
    blocks: [u16; CROWD_BLOCKS],
    tick: u64,
    wave_cursor: u8,
    wave_clock: u64,
}

/// Excitement one cue adds to its home end, in permille. Kicks land
/// harder than blocks; the room-ender dwarfs them all. A held block
/// bounces with the chip actually dealt, never above a clean hit.
fn delta_for(cue: FightCue) -> u16 {
    match cue {
        FightCue::Hit { .. } => 80,
        FightCue::Blocked { chip, .. } => chip.min(80),
        FightCue::Parried { .. } => 150,
        FightCue::Dodged { .. } => 40,
        FightCue::StaminaOut { .. } => 0,
        FightCue::Knockdown { .. } => 400,
    }
}

/// Home end for one cue: the victim's sector, so reactions start where
/// the blow landed and the wave carries them around the room.
fn home_sector(cue: FightCue) -> u8 {
    let slot = match cue {
        FightCue::Hit { victim, .. }
        | FightCue::Blocked { victim, .. }
        | FightCue::Parried { victim, .. }
        | FightCue::Dodged { victim, .. }
        | FightCue::Knockdown { victim, .. } => victim,
        FightCue::StaminaOut { fighter } => fighter,
    };
    slot % 4
}

/// Per-seat VAT variation: offset, tint, jitter, VAT frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct SeatVariation {
    pub phase_offset: u16,
    pub tint: u8,
    pub variant: u8,
    pub jitter_cm: [i8; 3],
    pub vat_frame: u8,
}

/// Derive per-seat parts via FNV over seed, index, block, excitement.
#[must_use]
pub(crate) fn seat_variation(seed: u64, seat: u32, block: u8, excitement: u16) -> SeatVariation {
    let mut h = Fnv64::new();
    h.write_u64(seed);
    h.write_u32(seat);
    h.write_u8(block);
    h.write_u16(excitement.min(1000));
    let v = h.finish();
    let lat = seat.saturating_mul(37);
    let low = u32::try_from(v % 64).unwrap_or(0);
    SeatVariation {
        phase_offset: u16::try_from((v >> 9) % 1000).unwrap_or(0),
        tint: u8::try_from(lat.saturating_add(low) % 64).unwrap_or(0),
        variant: u8::try_from((v >> 6) % 8).unwrap_or(0),
        jitter_cm: [
            i8::try_from((v >> 19) % 5).unwrap_or(2) - 2,
            i8::try_from((v >> 22) % 5).unwrap_or(2) - 2,
            i8::try_from((v >> 25) % 5).unwrap_or(2) - 2,
        ],
        vat_frame: u8::try_from((v >> 28) % 8).unwrap_or(0),
    }
}

impl CrowdGpuSim {
    /// New room over `seats` (`1..=MAX_CROWD_SEATS`): every block rests
    /// at the baseline murmur, the clock and the wave sit at zero.
    pub fn new(seed: u64, seats: u32) -> Result<Self, CrowdGpuError> {
        if seats == 0 || seats > MAX_CROWD_SEATS {
            return Err(CrowdGpuError::BadSeats {
                seats,
                max: MAX_CROWD_SEATS,
            });
        }
        Ok(Self {
            seed,
            seats,
            blocks: [BASELINE_PERMILLE; CROWD_BLOCKS],
            tick: 0,
            wave_cursor: 0,
            wave_clock: 0,
        })
    }

    /// Production room: 20k rich seats over the seed. Infallible by
    /// construction; the seat literal sits inside the validated range.
    #[must_use]
    pub fn rich(seed: u64) -> Self {
        Self {
            seed,
            seats: RICH_CROWD_SEATS,
            blocks: [BASELINE_PERMILLE; CROWD_BLOCKS],
            tick: 0,
            wave_cursor: 0,
            wave_clock: 0,
        }
    }

    /// Seats the room distributes.
    #[must_use]
    pub fn seats(&self) -> u32 {
        self.seats
    }

    /// Fixed-tick clock: advances exactly once per [`CrowdGpuSim::tick`].
    #[must_use]
    pub fn tick_count(&self) -> u64 {
        self.tick
    }

    /// React to one fight cue: the home end takes the full delta, the
    /// far ends take half (integer floor), every block saturating at the
    /// peak. Silent cues (stamina) move nothing. Never advances the clock.
    pub fn on_cue(&mut self, cue: FightCue) {
        let delta = delta_for(cue);
        if delta == 0 {
            return;
        }
        let home = home_sector(cue);
        for block in 0..CROWD_BLOCKS_U8 {
            let sector = block / 16;
            let add = if sector == home { delta } else { delta / 2 };
            let slot = usize::from(block);
            self.blocks[slot] = self.blocks[slot].saturating_add(add).min(PEAK_PERMILLE);
        }
    }

    /// Advance one fixed tick: every block decays linearly toward the
    /// baseline, and a loud room steps the wave front every
    /// [`WAVE_PERIOD_TICKS`] ticks. Same cue script plus same cadence
    /// gives the same levels twice.
    pub fn tick(&mut self) {
        for slot in 0..CROWD_BLOCKS {
            let level = self.blocks[slot];
            if level > BASELINE_PERMILLE {
                self.blocks[slot] = level.saturating_sub(DECAY_PER_TICK).max(BASELINE_PERMILLE);
            } else if level < BASELINE_PERMILLE {
                self.blocks[slot] = level.saturating_add(DECAY_PER_TICK).min(BASELINE_PERMILLE);
            }
        }
        let loud = self
            .blocks
            .iter()
            .any(|level| *level >= WAVE_THRESHOLD_PERMILLE);
        if loud {
            self.wave_clock = self.wave_clock.saturating_add(1);
            if self.wave_clock >= WAVE_PERIOD_TICKS {
                self.wave_clock = 0;
                self.wave_cursor = (self.wave_cursor + 1) % CROWD_BLOCKS_U8;
                let slot = usize::from(self.wave_cursor);
                self.blocks[slot] = self.blocks[slot]
                    .saturating_add(WAVE_BUMP_PERMILLE)
                    .min(PEAK_PERMILLE);
            }
        } else {
            self.wave_clock = 0;
        }
        self.tick = self.tick.saturating_add(1);
    }

    /// Excitement for one block in permille; indices wrap around the room.
    #[must_use]
    pub fn excitement(&self, block: u8) -> u16 {
        self.blocks[usize::from(block % CROWD_BLOCKS_U8)]
    }

    /// Loudest block in permille.
    #[must_use]
    pub fn peak(&self) -> u16 {
        let mut top = BASELINE_PERMILLE;
        for level in self.blocks.iter() {
            if *level > top {
                top = *level;
            }
        }
        top
    }

    /// Mean excitement in permille, integer floor over the 64 blocks.
    #[must_use]
    pub fn mean_permille(&self) -> u16 {
        let mut sum: u32 = 0;
        for level in self.blocks.iter() {
            sum = sum.saturating_add(u32::from(*level));
        }
        u16::try_from(sum / CROWD_BLOCKS_U32).unwrap_or(PEAK_PERMILLE)
    }

    /// Wave-front block, `0..64`.
    #[must_use]
    pub fn wave_cursor(&self) -> u8 {
        self.wave_cursor
    }

    /// Seats dealt to one block: the base share plus one for the first
    /// `seats % 64` blocks, so the shares sum to `seats` exactly.
    #[must_use]
    pub fn seats_for_block(&self, block: u8) -> u32 {
        let base = self.seats / CROWD_BLOCKS_U32;
        let rest = self.seats % CROWD_BLOCKS_U32;
        if u32::from(block % CROWD_BLOCKS_U8) < rest {
            base.saturating_add(1)
        } else {
            base
        }
    }

    /// FNV-1a digest over the seed, seats, clock, cursor and every block:
    /// equal scripts give equal hashes, a frozen hash would hide a dead
    /// room. Presentation evidence only, never a consensus input.
    #[must_use]
    pub fn state_hash(&self) -> u64 {
        let mut hash = Fnv64::new();
        hash.write_u64(self.seed);
        hash.write_u32(self.seats);
        hash.write_u64(self.tick);
        hash.write_u8(self.wave_cursor);
        for level in self.blocks.iter() {
            hash.write_u16(*level);
        }
        hash.finish()
    }

    /// Per-seat variation for one block: mirrors the render lattice.
    pub(crate) fn variation_for(&self, seat: u32, block: u8) -> SeatVariation {
        let excite = self.blocks[usize::from(block % CROWD_BLOCKS_U8)];
        seat_variation(self.seed, seat, block, excite)
    }

    /// Copy the 64 block levels into a caller buffer for the render
    /// stage: returns the entries written. The buffer must hold 64.
    pub fn export_levels(&self, out: &mut [u16]) -> Result<usize, CrowdGpuError> {
        if out.len() < CROWD_BLOCKS {
            return Err(CrowdGpuError::BadBuffer {
                want: CROWD_BLOCKS,
                got: out.len(),
            });
        }
        out[..CROWD_BLOCKS].copy_from_slice(&self.blocks);
        Ok(CROWD_BLOCKS)
    }

    /// Stage one window frame from this sim: block levels plus the tick
    /// cursor flow into the render instancer over caller-owned LOD and
    /// instance buffers. Pure over the sim; the buffers refill in place.
    /// Budget: O(seats) time, no allocation once the buffers are sized.
    pub fn feed_frame(
        &self,
        layout: &engine_render::crowd_inst::SeatLayout,
        camera_m: [f32; 3],
        planes: &[[f32; 4]; 6],
        prev_lod: &mut [u8],
        out: &mut [engine_render::crowd_inst::CrowdInstance],
    ) -> Result<engine_render::crowd_inst::CrowdFrameStats, CrowdGpuError> {
        if !camera_m.iter().all(|c| c.is_finite()) {
            return Err(CrowdGpuError::BadCamera("camera_m is not finite"));
        }
        debug_assert_eq!(
            seat_variation(self.seed, 0, 0, self.blocks[0]),
            self.variation_for(0, 0)
        );
        let mut staged = engine_render::crowd_inst::CrowdBuilder::new(
            layout,
            &self.blocks,
            self.tick,
            camera_m,
            planes,
            prev_lod,
            out,
        )?;
        Ok(staged.build())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_render::crowd_inst::{CrowdInstance, SeatLayout};

    fn hit(victim: u8, damage: u16) -> FightCue {
        FightCue::Hit {
            attacker: 0,
            victim,
            damage,
        }
    }

    fn knockdown(victim: u8) -> FightCue {
        FightCue::Knockdown {
            attacker: 0,
            victim,
        }
    }

    fn open_planes() -> [[f32; 4]; 6] {
        [[0.0, 0.0, 0.0, 1.0e9]; 6]
    }

    fn shut_planes() -> [[f32; 4]; 6] {
        [[0.0, 0.0, 0.0, -1.0e9]; 6]
    }

    #[test]
    fn new_refuses_zero_and_past_cap() {
        match CrowdGpuSim::new(2040, 0) {
            Err(CrowdGpuError::BadSeats { seats, max }) => {
                assert_eq!((seats, max), (0, MAX_CROWD_SEATS));
            }
            other => panic!("zero seats must refuse, got {other:?}"),
        }
        match CrowdGpuSim::new(2040, MAX_CROWD_SEATS + 1) {
            Err(CrowdGpuError::BadSeats { seats, max }) => {
                assert_eq!((seats, max), (MAX_CROWD_SEATS + 1, MAX_CROWD_SEATS));
            }
            other => panic!("past-cap seats must refuse, got {other:?}"),
        }
        let room = CrowdGpuSim::new(2040, 24).expect("small rooms build for tests");
        assert_eq!(room.seats(), 24);
        assert_eq!(room.peak(), BASELINE_PERMILLE);
        let rich = CrowdGpuSim::rich(2040);
        assert_eq!(rich.seats(), RICH_CROWD_SEATS);
    }

    #[test]
    fn knockdown_spikes_home_end_then_decays_to_baseline() {
        let mut room = CrowdGpuSim::new(7, 640).expect("room builds");
        room.on_cue(knockdown(1));
        // Victim slot 1 seats the home end at blocks 16..32.
        assert_eq!(room.excitement(16), BASELINE_PERMILLE + 400);
        assert_eq!(room.excitement(0), BASELINE_PERMILLE + 200);
        assert_eq!(room.peak(), BASELINE_PERMILLE + 400);
        assert!(room.mean_permille() > BASELINE_PERMILLE);
        // Loudest block needs (400 / 2) decay ticks to rest exactly.
        for _ in 0..200 {
            room.tick();
        }
        assert_eq!(room.peak(), BASELINE_PERMILLE, "decay must land, not hover");
        assert_eq!(room.mean_permille(), BASELINE_PERMILLE);
        assert_eq!(room.tick_count(), 200);
    }

    #[test]
    fn quiet_cues_move_nothing_loud_cues_saturate() {
        let mut room = CrowdGpuSim::new(3, 640).expect("room builds");
        let before = room.state_hash();
        room.on_cue(FightCue::StaminaOut { fighter: 0 });
        room.on_cue(FightCue::Blocked {
            attacker: 0,
            victim: 1,
            chip: 5,
        });
        assert_eq!(room.excitement(16), BASELINE_PERMILLE + 5);
        assert_ne!(room.state_hash(), before, "chip must move the hash");
        for _ in 0..40 {
            room.on_cue(knockdown(2));
        }
        assert_eq!(room.peak(), PEAK_PERMILLE, "blocks saturate, never wrap");
        assert!(
            room.excitement(200) <= PEAK_PERMILLE,
            "wrapped indices stay in range"
        );
    }

    #[test]
    fn same_script_twice_same_levels_and_hash() {
        let script = [
            hit(0, 120),
            FightCue::Parried {
                attacker: 1,
                victim: 0,
            },
            hit(1, 24),
            knockdown(0),
            FightCue::Dodged {
                attacker: 1,
                victim: 0,
            },
        ];
        let run = || {
            let mut room = CrowdGpuSim::new(2040, 640).expect("room builds");
            for cue in script.iter() {
                room.on_cue(*cue);
                room.tick();
            }
            (room.state_hash(), room.peak(), room.mean_permille())
        };
        assert_eq!(run(), run());
        let (hash, _, _) = run();
        let mut later = CrowdGpuSim::new(2040, 640).expect("room builds");
        for cue in script.iter() {
            later.on_cue(*cue);
            later.tick();
        }
        for _ in 0..500 {
            later.tick();
        }
        assert_eq!(later.peak(), BASELINE_PERMILLE);
        assert!(
            later.state_hash() != hash,
            "a frozen hash would hide a dead room"
        );
        let fresh = CrowdGpuSim::new(2041, 640).expect("room builds");
        assert!(fresh.state_hash() != hash, "seeds must diverge");
    }

    #[test]
    fn seat_shares_sum_to_seats_exactly() {
        for seats in [1u32, 24, 640, 20_000, 50_000] {
            let room = CrowdGpuSim::new(11, seats).expect("room builds");
            let mut sum: u32 = 0;
            let mut top: u32 = 0;
            let mut bottom: u32 = u32::MAX;
            for block in 0..CROWD_BLOCKS_U8 {
                let share = room.seats_for_block(block);
                sum = sum.saturating_add(share);
                if share > top {
                    top = share;
                }
                if share < bottom {
                    bottom = share;
                }
            }
            assert_eq!(sum, seats, "shares must deal every seat");
            assert!(top - bottom <= 1, "shares differ by at most one");
        }
    }

    #[test]
    fn block_width_consts_match_room() {
        assert_eq!(usize::from(CROWD_BLOCKS_U8), CROWD_BLOCKS);
        assert_eq!(usize::try_from(CROWD_BLOCKS_U32).unwrap_or(0), CROWD_BLOCKS);
    }

    #[test]
    fn steady_tick_allocates_nothing() {
        let mut room = CrowdGpuSim::new(5, 640).expect("room builds");
        room.on_cue(knockdown(0));
        let ((), probe) = engine_core::alloc_probe::measure(|| {
            for _ in 0..30 {
                room.tick();
            }
        });
        assert_eq!(
            probe.calls, 0,
            "warm ticks must not touch the heap: {probe:?}"
        );
        let mut levels = [0u16; CROWD_BLOCKS];
        let (n, export_probe) =
            engine_core::alloc_probe::measure(|| room.export_levels(&mut levels));
        assert_eq!(export_probe.calls, 0);
        assert_eq!(n.expect("export fits"), CROWD_BLOCKS);
        let mut short = [0u16; 7];
        match room.export_levels(&mut short) {
            Err(CrowdGpuError::BadBuffer { want, got }) => {
                assert_eq!((want, got), (CROWD_BLOCKS, 7));
            }
            other => panic!("short export buffer must refuse, got {other:?}"),
        }
    }

    #[test]
    fn seat_variation_is_stable_and_neighbour_distinct() {
        let a = seat_variation(2040, 7, 16, 700);
        assert_eq!(a, seat_variation(2040, 7, 16, 700));
        assert!(a.tint < 64 && a.variant < 8 && a.vat_frame < 8);
        assert!(a.phase_offset < 1000);
        for j in a.jitter_cm.iter() {
            assert!((-2..=2).contains(j));
        }
        let b = seat_variation(2040, 8, 17, 700);
        assert!(a.tint != b.tint || a.variant != b.variant || a.phase_offset != b.phase_offset);
        let room = CrowdGpuSim::new(2040, 640).expect("room builds");
        assert_eq!(
            room.variation_for(7, 16),
            seat_variation(2040, 7, 16, room.excitement(16))
        );
    }

    #[test]
    fn feed_frame_wires_levels_into_staging() {
        let layout = SeatLayout {
            rows_per_sector: 2,
            cols_per_row: 3,
        };
        let seats = layout.seat_count();
        let mut room = CrowdGpuSim::new(9, 640).expect("room builds");
        room.on_cue(knockdown(0));
        room.tick();
        let stage = || {
            let mut prev = vec![0u8; seats];
            let mut out = vec![CrowdInstance::culled([0.0, 0.0, 0.0]); seats];
            room.feed_frame(
                &layout,
                [0.0, 6.0, 0.0],
                &open_planes(),
                &mut prev,
                &mut out,
            )
            .expect("open frame stages")
        };
        let stats = stage();
        assert_eq!(
            stats.instances,
            u32::try_from(seats).expect("test layout fits u32")
        );
        let mut prev = vec![0u8; seats];
        let mut out = vec![CrowdInstance::culled([0.0, 0.0, 0.0]); seats];
        let shut = room
            .feed_frame(
                &layout,
                [0.0, 6.0, 0.0],
                &shut_planes(),
                &mut prev,
                &mut out,
            )
            .expect("shut frame still stages");
        assert_eq!((shut.sectors_visible, shut.instances), (0, 0));
        assert!(shut.digest != stats.digest, "cull must move the digest");
        let mut prev = vec![0u8; seats - 1];
        let mut out = vec![CrowdInstance::culled([0.0, 0.0, 0.0]); seats];
        match room.feed_frame(
            &layout,
            [0.0, 6.0, 0.0],
            &open_planes(),
            &mut prev,
            &mut out,
        ) {
            Err(CrowdGpuError::Staging(_)) => {}
            other => panic!("short stage buffer must refuse, got {other:?}"),
        }
        match room.feed_frame(
            &layout,
            [f32::NAN, 0.0, 0.0],
            &open_planes(),
            &mut prev,
            &mut out,
        ) {
            Err(CrowdGpuError::BadCamera(_)) => {}
            other => panic!("NaN camera must refuse, got {other:?}"),
        }
    }

    #[test]
    fn app_drives_the_room_clock_beside_the_audio_crowd() {
        use crate::{App, GameConfig};
        let config = GameConfig {
            ticks: 12,
            ..GameConfig::default()
        };
        let mut first = App::new(config).expect("session builds");
        let mut second = App::new(config).expect("session builds");
        for _ in 0..3 {
            first.tick_empty();
            second.tick_empty();
        }
        assert_eq!(first.cursor(), 3);
        assert_eq!(
            first.crowd_gpu().tick_count(),
            3,
            "room clock tracks the cursor"
        );
        assert_eq!(
            first.crowd_gpu().state_hash(),
            second.crowd_gpu().state_hash(),
            "same session twice gives the same room"
        );
        assert_eq!(first.crowd_gpu().seats(), RICH_CROWD_SEATS);
    }
}
