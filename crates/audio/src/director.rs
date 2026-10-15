//! director: fight-driven music/tension brain.
//!
//! Owner: audio lane; consumer: the game arena mix, which feeds one
//! [`PlacedCue`] per fight event with its listener distance, calls
//! [`Director::update`] once per tick, and applies [`Layers`] to the buses
//! plus [`Player`](crate::music::Player).
//! Units: integer cm for distances, ticks at 60 Hz, intensity and gains in
//! 0..1. Determinism: same table plus same cue script plus same tick cadence
//! give the same intensity curve and layers (presentation only, never hashed;
//! golden d57b5c8ae3096614 stands). Errors: typed [`DirectorError`]; the
//! per-tick path clamps, setters refuse. Budget: 0.05 ms per update on the
//! game thread (see [`BUDGET_MS_PER_UPDATE`]).

use crate::crowd::FightCue;
use std::fmt::{Display, Formatter, Result as FmtResult};

/// Ticks per second the game thread runs [`Director::update`].
pub const TICK_HZ: u32 = 60;
/// Distance in cm at or inside which an event lands at full weight.
pub const CLOSE_CM_DEFAULT: u32 = 400;
/// Distance in cm at or beyond which an event is unheard.
pub const FAR_CM_DEFAULT: u32 = 2500;
/// Scripted excitation of a landed hit per tick.
pub const HIT_WEIGHT_DEFAULT: f32 = 0.22;
/// Scripted excitation of a held block per tick (chip damage, room shrugs).
pub const BLOCKED_WEIGHT_DEFAULT: f32 = 0.03;
/// Scripted excitation of a parry per tick (crowd loves a parry).
pub const PARRIED_WEIGHT_DEFAULT: f32 = 0.34;
/// Scripted excitation of a dodge per tick.
pub const DODGED_WEIGHT_DEFAULT: f32 = 0.14;
/// Scripted excitation of a stamina-out per tick (a whiff, no heat).
pub const STAMINA_OUT_WEIGHT_DEFAULT: f32 = 0.0;
/// Scripted excitation of a knockdown per tick (the roof comes off).
pub const KNOCKDOWN_WEIGHT_DEFAULT: f32 = 0.9;
/// Scripted intensity lost per quiet tick (full to zero in 125 ticks).
pub const DECAY_PER_TICK_DEFAULT: f32 = 0.008;
/// Largest intensity rise in one tick: full swing in 9 ticks (~150 ms).
pub const ATTACK_STEP: f32 = 0.12;
/// Largest intensity fall in one tick: full swing in 67 ticks (~1.1 s).
pub const RELEASE_STEP: f32 = 0.015;
/// Intensity at or above which the room is rising.
pub const RISING_AT: f32 = 0.35;
/// Intensity at or above which the room is at climax.
pub const CLIMAX_AT: f32 = 0.70;
/// Intensity at which the cheer bed is fully on.
pub const CHEER_FULL_AT: f32 = 0.80;
/// Intensity where the gasp bed starts to open.
pub const GASP_LO: f32 = 0.55;
/// Intensity at which the gasp bed is fully on.
pub const GASP_FULL_AT: f32 = 0.85;
/// Ticks after a climax during which low intensity reads as release.
pub const RELEASE_HOLD_TICKS: u64 = 180;
/// Quiet ticks after a heard fight before the room starts to boo.
pub const BOO_QUIET_TICKS: u64 = 240;
/// Quiet ticks over which the boo bed ramps from zero to peak.
pub const BOO_RAMP_TICKS: u64 = 120;
/// Boo bed peak gain (a grumble, never a wall of hate).
pub const BOO_PEAK: f32 = 0.5;
/// Intensity below which a long lull turns into boos.
pub const BOO_INTENSITY_CEILING: f32 = 0.2;
/// Murmur bed floor: the room never drops past half bed (no silence cuts).
pub const MURMUR_FLOOR: f32 = 0.5;
/// Most cues one [`Director::update`] call integrates; bounds the tick cost.
/// Longer bursts keep only the first slice, in order, deterministically.
pub const MAX_EVENTS_PER_UPDATE: usize = 64;
/// Per-update budget in ms on the game thread, release profile; the timing
/// probe runs under `--perf`, never by default.
pub const BUDGET_MS_PER_UPDATE: f32 = 0.05;

/// Typed director failure. Table setters return these; the per-tick
/// [`Director::update`] path clamps instead so the mix never drops on data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectorError {
    /// A table field left its range (or was NaN); names the field.
    BadTable {
        /// Static field name (`"hit"`, `"range"`, `"decay_per_tick"`, ...).
        name: &'static str,
    },
}

impl Display for DirectorError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::BadTable { name } => write!(f, "director: bad table {name}"),
        }
    }
}

impl std::error::Error for DirectorError {}

/// Scripted excitation table: how much heat each fight cue adds, how fast it
/// lands with distance, and how fast the room cools. Game thread owns it;
/// the audio side only reads gains out of [`Layers`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TensionTable {
    /// Excitation per landed hit, 0..1.
    pub hit: f32,
    /// Excitation per held block, 0..1.
    pub blocked: f32,
    /// Excitation per parry, 0..1.
    pub parried: f32,
    /// Excitation per dodge, 0..1.
    pub dodged: f32,
    /// Excitation per stamina-out whiff, 0..1.
    pub stamina_out: f32,
    /// Excitation per knockdown, 0..1.
    pub knockdown: f32,
    /// Full-weight radius in cm; must be below `far_cm`.
    pub close_cm: u32,
    /// Unheard radius in cm; must be above `close_cm`.
    pub far_cm: u32,
    /// Intensity lost per tick with no excitation, 0..1.
    pub decay_per_tick: f32,
}

impl TensionTable {
    /// Scripted default table: close fights boil, distant ones murmur.
    /// Valid by construction (see the matching test).
    pub fn scripted() -> Self {
        Self {
            hit: HIT_WEIGHT_DEFAULT,
            blocked: BLOCKED_WEIGHT_DEFAULT,
            parried: PARRIED_WEIGHT_DEFAULT,
            dodged: DODGED_WEIGHT_DEFAULT,
            stamina_out: STAMINA_OUT_WEIGHT_DEFAULT,
            knockdown: KNOCKDOWN_WEIGHT_DEFAULT,
            close_cm: CLOSE_CM_DEFAULT,
            far_cm: FAR_CM_DEFAULT,
            decay_per_tick: DECAY_PER_TICK_DEFAULT,
        }
    }

    /// Check ranges; NaN refuses as [`DirectorError::BadTable`].
    pub fn validate(&self) -> Result<(), DirectorError> {
        for (value, name) in [
            (self.hit, "hit"),
            (self.blocked, "blocked"),
            (self.parried, "parried"),
            (self.dodged, "dodged"),
            (self.stamina_out, "stamina_out"),
            (self.knockdown, "knockdown"),
        ] {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(DirectorError::BadTable { name });
            }
        }
        if self.close_cm >= self.far_cm || self.far_cm == 0 {
            return Err(DirectorError::BadTable { name: "range" });
        }
        if !self.decay_per_tick.is_finite() || !(0.0..=1.0).contains(&self.decay_per_tick) {
            return Err(DirectorError::BadTable {
                name: "decay_per_tick",
            });
        }
        Ok(())
    }

    /// Excitation of one cue kind under this table.
    fn weight(&self, cue: FightCue) -> f32 {
        match cue {
            FightCue::Hit { .. } => self.hit,
            FightCue::Blocked { .. } => self.blocked,
            FightCue::Parried { .. } => self.parried,
            FightCue::Dodged { .. } => self.dodged,
            FightCue::StaminaOut { .. } => self.stamina_out,
            FightCue::Knockdown { .. } => self.knockdown,
        }
    }
}

/// One fight cue with its listener distance. The game lane maps each combat
/// fight event onto the same-named [`FightCue`] field for field (see
/// [`crate::crowd`]) and stamps how far the exchange is from the listener.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PlacedCue {
    /// What happened (crowd-mapped fight cue).
    pub cue: FightCue,
    /// Listener distance in integer cm.
    pub distance_cm: u32,
}

impl PlacedCue {
    /// Stamp a cue with its listener distance in cm.
    pub fn new(cue: FightCue, distance_cm: u32) -> Self {
        Self { cue, distance_cm }
    }
}

/// Per-tick mix answer, all gains in 0..1. `intensity` doubles as the music
/// scalar (see [`Layers::request_music`]); the rest ride the crowd beds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layers {
    /// Room heat 0..1; feed to the music player.
    pub intensity: f32,
    /// Music bed scalar; always equals `intensity`.
    pub music: f32,
    /// Crowd bed; never below [`MURMUR_FLOOR`], so beds never cut to air.
    pub murmur: f32,
    /// Cheer bed; opens past [`RISING_AT`], full at [`CHEER_FULL_AT`].
    pub cheer: f32,
    /// Gasp bed; opens past [`GASP_LO`], full at [`GASP_FULL_AT`].
    pub gasp: f32,
    /// Boo bed; ramps only after a heard fight goes long quiet.
    pub boo: f32,
}

impl Layers {
    /// Hand the intensity scalar to a [`Player`](crate::music::Player) as its
    /// next bar request (edge: director reads music thresholds, same crate).
    /// The player still commits only at bar lines.
    pub fn request_music(&self, player: &mut crate::music::Player) {
        player.request(self.intensity);
    }
}

/// Four-state room arc: the bed murmurs, rises with exchanges, climaxes on
/// heavy hits, then releases back down instead of cutting to silence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DirectorState {
    /// Quiet room, beds at murmur.
    Murmur,
    /// Exchanges landing, beds opening.
    Rising,
    /// Heavy heat, all beds open.
    Climax,
    /// Was hot within [`RELEASE_HOLD_TICKS`], now cooling.
    Release,
}

/// Fight-driven tension brain: leaky intensity integrator with capped slew
/// plus a four-state room arc. Game thread builds it, feeds it slices, and
/// reads [`Layers`]; it keeps scalars only, so the steady-state tick borrows
/// the cue slice and touches no heap.
#[derive(Clone, Debug)]
pub struct Director {
    table: TensionTable,
    intensity: f32,
    tick: u64,
    last_event_tick: u64,
    had_event: bool,
    high_tick: u64,
    has_high: bool,
    state: DirectorState,
}

/// Clamp to 0..1; NaN becomes 0 so bad math never leaves the struct.
fn clamp01(v: f32) -> f32 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(0.0, 1.0)
    }
}

/// Proximity share of an event at `distance_cm`: full inside `close_cm`,
/// zero past `far_cm`, linear between. Needs `close_cm < far_cm`.
fn proximity(distance_cm: u32, close_cm: u32, far_cm: u32) -> f32 {
    if distance_cm <= close_cm {
        1.0
    } else if distance_cm >= far_cm {
        0.0
    } else {
        1.0 - (distance_cm - close_cm) as f32 / (far_cm - close_cm) as f32
    }
}

/// Layer ramps for `intensity` plus the boo lull meter. Pure in intensity,
/// so the capped intensity slew caps every layer jump by construction.
fn layers_for(intensity: f32, quiet_ticks: u64, had_event: bool) -> Layers {
    let cheer = clamp01((intensity - RISING_AT) / (CHEER_FULL_AT - RISING_AT));
    let gasp = clamp01((intensity - GASP_LO) / (GASP_FULL_AT - GASP_LO));
    let boo = if had_event && intensity < BOO_INTENSITY_CEILING && quiet_ticks >= BOO_QUIET_TICKS {
        ((quiet_ticks - BOO_QUIET_TICKS) as f32 / BOO_RAMP_TICKS as f32).min(1.0) * BOO_PEAK
    } else {
        0.0
    };
    Layers {
        intensity,
        music: intensity,
        murmur: 1.0 - MURMUR_FLOOR * intensity,
        cheer,
        gasp,
        boo,
    }
}

impl Director {
    /// Build on a caller table; refuses a bad table and builds nothing.
    ///
    /// Errors: [`DirectorError::BadTable`] from [`TensionTable::validate`].
    pub fn new(table: TensionTable) -> Result<Self, DirectorError> {
        table.validate()?;
        Ok(Self {
            table,
            intensity: 0.0,
            tick: 0,
            last_event_tick: 0,
            had_event: false,
            high_tick: 0,
            has_high: false,
            state: DirectorState::Murmur,
        })
    }

    /// Build on the scripted default table; valid by construction.
    pub fn scripted() -> Self {
        Self {
            table: TensionTable::scripted(),
            intensity: 0.0,
            tick: 0,
            last_event_tick: 0,
            had_event: false,
            high_tick: 0,
            has_high: false,
            state: DirectorState::Murmur,
        }
    }

    /// Swap the excitation table; a bad table refuses and keeps the old one.
    ///
    /// Errors: [`DirectorError::BadTable`] from [`TensionTable::validate`].
    pub fn set_table(&mut self, table: TensionTable) -> Result<(), DirectorError> {
        table.validate()?;
        self.table = table;
        Ok(())
    }

    /// Active excitation table (copy; the struct owns its state).
    pub fn table(&self) -> TensionTable {
        self.table
    }

    /// Room heat 0..1 (also the music scalar).
    pub fn intensity(&self) -> f32 {
        self.intensity
    }

    /// Room arc state for beds and blind-review tracks.
    pub fn state(&self) -> DirectorState {
        self.state
    }

    /// Fixed-tick clock (one [`update`](Self::update) per tick).
    pub fn tick_count(&self) -> u64 {
        self.tick
    }

    /// Back to a silent room at tick 0; keeps the table.
    pub fn reset(&mut self) {
        self.intensity = 0.0;
        self.tick = 0;
        self.last_event_tick = 0;
        self.had_event = false;
        self.high_tick = 0;
        self.has_high = false;
        self.state = DirectorState::Murmur;
    }

    /// Ticks since the last heard cue (`u64::MAX` before the first one).
    fn quiet_ticks(&self) -> u64 {
        if self.had_event {
            self.tick.wrapping_sub(self.last_event_tick)
        } else {
            u64::MAX
        }
    }

    /// Room arc from heat plus recent peak: hot reads climax, warm reads
    /// rising, cool-but-recently-hot reads release, the rest is murmur.
    fn compute_state(&self) -> DirectorState {
        if self.intensity >= CLIMAX_AT {
            DirectorState::Climax
        } else if self.intensity >= RISING_AT {
            DirectorState::Rising
        } else if self.has_high && self.tick.wrapping_sub(self.high_tick) <= RELEASE_HOLD_TICKS {
            DirectorState::Release
        } else {
            DirectorState::Murmur
        }
    }

    /// Whole per-tick step: integrate the first [`MAX_EVENTS_PER_UPDATE`]
    /// cues with distance falloff, leak the decay, slew with the attack and
    /// release caps, then answer the layer ramps. Borrows only; the clock
    /// advances even on an empty slice so lulls cool on schedule. Same table
    /// plus same cue script plus same tick cadence give the same curve.
    pub fn update(&mut self, events: &[PlacedCue]) -> Layers {
        self.tick = self.tick.saturating_add(1);
        let mut excitation = 0.0f32;
        let mut heard = 0usize;
        for placed in events.iter().take(MAX_EVENTS_PER_UPDATE) {
            excitation += self.table.weight(placed.cue)
                * proximity(placed.distance_cm, self.table.close_cm, self.table.far_cm);
            heard += 1;
        }
        excitation = excitation.min(1.0);
        if heard > 0 {
            self.last_event_tick = self.tick;
            self.had_event = true;
        }
        let delta = (excitation - self.table.decay_per_tick).clamp(-RELEASE_STEP, ATTACK_STEP);
        self.intensity = clamp01(self.intensity + delta);
        if self.intensity >= CLIMAX_AT {
            self.high_tick = self.tick;
            self.has_high = true;
        }
        self.state = self.compute_state();
        layers_for(self.intensity, self.quiet_ticks(), self.had_event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::music::{active_mask, Player};
    use engine_core::alloc_probe::measure;

    fn hit_at(distance_cm: u32) -> PlacedCue {
        PlacedCue::new(
            FightCue::Hit {
                attacker: 0,
                victim: 1,
                damage: 120,
            },
            distance_cm,
        )
    }

    fn knockdown_at(distance_cm: u32) -> PlacedCue {
        PlacedCue::new(
            FightCue::Knockdown {
                attacker: 0,
                victim: 1,
            },
            distance_cm,
        )
    }

    #[test]
    fn scripted_defaults_validate_and_order() {
        let table = TensionTable::scripted();
        table.validate().expect("scripted table is valid");
        assert!(table.close_cm < table.far_cm, "close sits inside far");
        assert_eq!(table.close_cm, CLOSE_CM_DEFAULT);
        assert_eq!(table.far_cm, FAR_CM_DEFAULT);
        assert_eq!(table.hit, HIT_WEIGHT_DEFAULT);
        assert_eq!(table.knockdown, KNOCKDOWN_WEIGHT_DEFAULT);
        assert_eq!(table.stamina_out, 0.0, "whiffs add no heat");
        assert!(
            table.parried > table.hit,
            "a parry pops louder than a trade hit"
        );
        let director = Director::scripted();
        assert_eq!(director.intensity(), 0.0);
        assert_eq!(director.state(), DirectorState::Murmur);
        assert_eq!(director.tick_count(), 0);
        assert_eq!(director.table(), table);
    }

    #[test]
    fn close_fight_raises_intensity_vs_distant() {
        // Same script, two distances: ringside at 2 m, nosebleeds at 30 m.
        fn peak(distance_cm: u32) -> (f32, f32) {
            let mut director = Director::scripted();
            let (mut heat, mut cheer) = (0.0f32, 0.0f32);
            for tick in 0..300u32 {
                let events: &[PlacedCue] = if tick.is_multiple_of(3) {
                    &[hit_at(distance_cm)]
                } else {
                    &[]
                };
                let layers = director.update(events);
                heat = heat.max(layers.intensity);
                cheer = cheer.max(layers.cheer);
            }
            (heat, cheer)
        }
        let (close_heat, close_cheer) = peak(200);
        let (far_heat, far_cheer) = peak(3000);
        assert!(close_heat >= 0.70, "ringside boils: peak {close_heat}");
        assert!(far_heat <= 0.10, "nosebleeds murmur: peak {far_heat}");
        assert!(
            close_heat - far_heat >= 0.50,
            "distance must separate the room"
        );
        assert!(close_cheer >= 0.90, "ringside opens cheer: {close_cheer}");
        assert_eq!(far_cheer, 0.0, "distant jabs never open cheer");
    }

    #[test]
    fn scripted_default_deterministic_x2() {
        // Mixed script: trades, parries, dodges, a block, a whiff, a finish.
        fn script(tick: u32) -> ([PlacedCue; 5], usize) {
            let mut events = [hit_at(999_999); 5];
            let mut n = 0usize;
            if tick.is_multiple_of(3) {
                events[n] = hit_at(200);
                n += 1;
            }
            if tick.is_multiple_of(7) {
                events[n] = PlacedCue::new(
                    FightCue::Parried {
                        attacker: 1,
                        victim: 0,
                    },
                    600,
                );
                n += 1;
            }
            if tick.is_multiple_of(5) {
                events[n] = PlacedCue::new(
                    FightCue::Dodged {
                        attacker: 1,
                        victim: 0,
                    },
                    900,
                );
                n += 1;
            }
            if tick.is_multiple_of(11) {
                events[n] = knockdown_at(1500);
                n += 1;
            }
            if tick.is_multiple_of(13) {
                events[n] = PlacedCue::new(
                    FightCue::Blocked {
                        attacker: 0,
                        victim: 1,
                        chip: 24,
                    },
                    300,
                );
                n += 1;
            }
            let _ = n;
            (events, n)
        }
        fn run() -> (std::vec::Vec<Layers>, std::vec::Vec<DirectorState>) {
            let mut director = Director::scripted();
            let mut layers = std::vec::Vec::with_capacity(600);
            let mut states = std::vec::Vec::with_capacity(600);
            for tick in 0..600u32 {
                let (full, n) = script(tick);
                let out = director.update(&full[..n]);
                layers.push(out);
                states.push(director.state());
            }
            (layers, states)
        }
        let (first_layers, first_states) = run();
        let (second_layers, second_states) = run();
        assert_eq!(first_layers, second_layers, "same script, same layers");
        assert_eq!(first_states, second_states, "same script, same arc");
        assert!(
            first_layers.iter().any(|l| l.intensity >= 0.70),
            "script must reach climax heat"
        );
        assert!(
            first_states.contains(&DirectorState::Climax),
            "script must visit climax"
        );
        // Fresh struct, same script: identical again (no hidden state).
        assert_eq!(run().0, first_layers, "no hidden state across rooms");
    }

    #[test]
    fn intensity_curve_receipt_posts_track() {
        // Blind-review hook: run with `-- --nocapture` to post the track.
        // Title fight: close exchanges, a knockdown, a lull, distant jabs.
        let mut director = Director::scripted();
        let mut peak = 0.0f32;
        for tick in 0..700u32 {
            let events: &[PlacedCue] = if tick < 300 && tick.is_multiple_of(3) {
                &[hit_at(250)]
            } else if tick == 300 {
                &[knockdown_at(200)]
            } else if tick > 400 && tick < 500 && tick.is_multiple_of(4) {
                &[hit_at(2000)]
            } else {
                &[]
            };
            let layers = director.update(events);
            peak = peak.max(layers.intensity);
            if tick.is_multiple_of(30) {
                println!(
                    "director-curve tick={tick} intensity={:.3} state={:?} cheer={:.3} boo={:.3}",
                    layers.intensity,
                    director.state(),
                    layers.cheer,
                    layers.boo
                );
            }
        }
        println!(
            "director-curve peak={peak:.3} end={:.3} state={:?}",
            director.intensity(),
            director.state()
        );
        assert!(peak >= 0.90, "knockdown must spike the room: {peak}");
        assert!(
            director.intensity() < 0.20,
            "tail must cool: {}",
            director.intensity()
        );
    }

    #[test]
    fn restraint_no_hard_cuts() {
        // Ten-minute-shape close fight, compressed: 1200 hot ticks, then air.
        let mut director = Director::scripted();
        let mut cheers = std::vec::Vec::with_capacity(1600);
        let mut musics = std::vec::Vec::with_capacity(1600);
        let mut murmurs = std::vec::Vec::with_capacity(1600);
        for tick in 0..1200u32 {
            let events: &[PlacedCue] = if tick.is_multiple_of(200) {
                &[knockdown_at(200), hit_at(200)]
            } else if tick.is_multiple_of(2) {
                &[hit_at(200)]
            } else {
                &[]
            };
            let layers = director.update(events);
            cheers.push(layers.cheer);
            musics.push(layers.music);
            murmurs.push(layers.murmur);
        }
        for _ in 0..400u32 {
            let layers = director.update(&[]);
            cheers.push(layers.cheer);
            musics.push(layers.music);
            murmurs.push(layers.murmur);
        }
        for pair in cheers.windows(2) {
            assert!(
                (pair[1] - pair[0]).abs() <= 0.28,
                "cheer never jumps: {} -> {}",
                pair[0],
                pair[1]
            );
        }
        for pair in musics.windows(2) {
            assert!(
                (pair[1] - pair[0]).abs() <= 0.13,
                "music never jumps: {} -> {}",
                pair[0],
                pair[1]
            );
        }
        assert!(
            murmurs.iter().all(|&m| m >= MURMUR_FLOOR - 1e-6),
            "bed never drops past half"
        );
        // Cheer-to-silence takes the long stairs: first hot cheer >= 0.5
        // must stay audible for at least 25 ticks of the cooldown.
        let hot = cheers.iter().position(|&c| c >= 0.5).expect("fight cheers");
        let silent = cheers[hot..]
            .iter()
            .position(|&c| c == 0.0)
            .expect("room cools to silence");
        assert!(
            silent >= 25,
            "cheer fade {silent} ticks is a cut, not a fade"
        );
    }

    #[test]
    fn state_arc_runs_murmur_rising_climax_release() {
        let mut director = Director::scripted();
        let mut arc = std::vec::Vec::new();
        let push = |director: &Director, arc: &mut std::vec::Vec<DirectorState>| {
            let state = director.state();
            if arc.last() != Some(&state) {
                arc.push(state);
            }
        };
        for _ in 0..10u32 {
            director.update(&[]);
            push(&director, &mut arc);
        }
        for _ in 0..80u32 {
            director.update(&[hit_at(200), hit_at(250)]);
            push(&director, &mut arc);
        }
        for _ in 0..400u32 {
            director.update(&[]);
            push(&director, &mut arc);
        }
        let want = [
            DirectorState::Murmur,
            DirectorState::Rising,
            DirectorState::Climax,
            DirectorState::Release,
            DirectorState::Murmur,
        ];
        let mut next = 0usize;
        for state in arc {
            if next < want.len() && state == want[next] {
                next += 1;
            }
        }
        assert_eq!(next, want.len(), "arc must run murmur to murmur");
    }

    #[test]
    fn boo_rises_only_after_a_heard_fight_goes_quiet() {
        // A room that never heard a punch never boos.
        let mut fresh = Director::scripted();
        for _ in 0..600u32 {
            let layers = fresh.update(&[]);
            assert_eq!(layers.boo, 0.0, "no fight, no boos");
        }
        // A heard fight gone quiet grumbles, ramping over BOO_RAMP_TICKS.
        let mut director = Director::scripted();
        for tick in 0..100u32 {
            let events: &[PlacedCue] = if tick.is_multiple_of(2) {
                &[hit_at(200)]
            } else {
                &[]
            };
            director.update(events);
        }
        let mut boo_at_300 = 0.0f32;
        for _ in 0..500u32 {
            let layers = director.update(&[]);
            if director.tick_count() - director.last_event_tick == 300 {
                boo_at_300 = layers.boo;
            }
        }
        assert!(
            (boo_at_300 - 0.25).abs() < 0.05,
            "boo ramps mid-lull: {boo_at_300}"
        );
        let tail = director.update(&[]);
        assert_eq!(tail.boo, BOO_PEAK, "long lull boos at peak");
        // Fresh action clears the grumble at once (the roar covers it).
        let resume = director.update(&[knockdown_at(200)]);
        assert_eq!(resume.boo, 0.0, "action resumes, boos stop");
    }

    #[test]
    fn invalid_tables_refuse_and_keep_old() {
        let mut director = Director::scripted();
        let good = director.table();
        let mut bad = good;
        bad.hit = f32::NAN;
        assert_eq!(
            director.set_table(bad),
            Err(DirectorError::BadTable { name: "hit" })
        );
        bad = good;
        bad.parried = 1.5;
        assert_eq!(
            director.set_table(bad),
            Err(DirectorError::BadTable { name: "parried" })
        );
        bad = good;
        bad.blocked = -0.1;
        assert!(director.set_table(bad).is_err(), "negative heat refuses");
        bad = good;
        bad.close_cm = 2500;
        bad.far_cm = 2500;
        assert_eq!(
            director.set_table(bad),
            Err(DirectorError::BadTable { name: "range" })
        );
        bad = good;
        bad.far_cm = 0;
        assert_eq!(
            director.set_table(bad),
            Err(DirectorError::BadTable { name: "range" })
        );
        bad = good;
        bad.decay_per_tick = 2.0;
        assert_eq!(
            director.set_table(bad),
            Err(DirectorError::BadTable {
                name: "decay_per_tick"
            })
        );
        assert_eq!(director.table(), good, "refusals keep the old table");
        assert_eq!(
            Director::new(bad).expect_err("bad table refuses"),
            DirectorError::BadTable {
                name: "decay_per_tick"
            }
        );
        assert_eq!(
            format!("{}", DirectorError::BadTable { name: "hit" }),
            "director: bad table hit"
        );
        // A legal retune lands.
        let mut retune = good;
        retune.hit = 0.5;
        director.set_table(retune).expect("legal retune lands");
        assert_eq!(director.table().hit, 0.5);
        // Reset keeps the table but clears the heat.
        director.update(&[knockdown_at(100)]);
        director.reset();
        assert_eq!(director.intensity(), 0.0);
        assert_eq!(director.state(), DirectorState::Murmur);
        assert_eq!(director.tick_count(), 0);
        assert_eq!(director.table().hit, 0.5, "reset keeps the retune");
    }

    #[test]
    fn empty_air_decays_to_silence_on_schedule() {
        let mut director = Director::scripted();
        for tick in 0..200u32 {
            let events: &[PlacedCue] = if tick.is_multiple_of(2) {
                &[hit_at(200)]
            } else {
                &[]
            };
            director.update(events);
        }
        assert!(
            director.intensity() >= 0.99,
            "sustained trades pin the room: {}",
            director.intensity()
        );
        let pinned = director.update(&[hit_at(200)]);
        assert_eq!(pinned.intensity, 1.0, "one more trade pins it exactly");
        let mut prev = director.intensity();
        for _ in 0..60u32 {
            let layers = director.update(&[]);
            assert!(layers.intensity <= prev, "cooling never reheats");
            prev = layers.intensity;
        }
        assert!(
            (0.40..0.65).contains(&prev),
            "60 quiet ticks land mid-cool: {prev}"
        );
        for _ in 0..240u32 {
            director.update(&[]);
        }
        assert_eq!(director.intensity(), 0.0, "300 quiet ticks clear the room");
        assert_eq!(director.state(), DirectorState::Murmur);
    }

    #[test]
    fn events_past_the_cap_use_the_first_slice() {
        // 65 ringside hits integrate exactly like the first 64, in order.
        let many = [hit_at(200); 65];
        let mut full = Director::scripted();
        let first = full.update(&many);
        let mut capped = Director::scripted();
        let second = capped.update(&many[..MAX_EVENTS_PER_UPDATE]);
        assert_eq!(first, second, "cap keeps the head of the burst");
        assert_eq!(full.intensity(), capped.intensity());
    }

    #[test]
    fn layers_drive_the_music_player() {
        // Real consumer edge: director intensity requests music layers.
        let mut director = Director::scripted();
        let calm = director.update(&[]);
        assert_eq!(active_mask(calm.music), 0b0001, "calm room beds pad only");
        let mut player = Player::new(0.0);
        calm.request_music(&mut player);
        assert!(!player.advance_bar(), "calm requests no change");
        for tick in 0..120u32 {
            let events: &[PlacedCue] = if tick.is_multiple_of(2) {
                &[hit_at(200)]
            } else {
                &[]
            };
            director.update(events);
        }
        let hot = director.update(&[knockdown_at(150)]);
        let mut player = Player::new(0.0);
        hot.request_music(&mut player);
        assert!(player.advance_bar(), "heat commits at the bar line");
        assert_eq!(player.cur, 1.0);
        assert_eq!(active_mask(player.cur), 0b1111, "hot room opens all layers");
    }

    #[test]
    fn hot_path_names_no_heap_types() {
        // Structural guard beside the counting window below: the code above
        // the cfg(test) marker must not name heap or failure-injection APIs,
        // so a per-tick container cannot land unnoticed even if the window
        // ever goes quiet.
        const SRC: &str = include_str!("director.rs");
        let head = SRC
            .split("#[cfg(test)]")
            .next()
            .expect("director.rs keeps its cfg(test) marker");
        for token in [
            "Vec",
            "Box<",
            "Box::",
            "String",
            "vec!",
            "format!",
            "format(",
            "collect",
            "to_vec",
            "HashMap",
            "BTreeMap",
            "println",
            "print!",
            "eprintln",
            "dbg!",
            "Rc<",
            "Arc<",
            "alloc",
            "Cell<",
            "RefCell",
            "Mutex",
            "RwLock",
            "unreachable!",
            "unimplemented!",
            "todo!",
            "panic!",
            "unwrap",
            "expect(",
        ] {
            assert!(
                !head.contains(token),
                "hot path must not name heap/failure API: {token}"
            );
        }
    }

    #[test]
    fn steady_state_update_allocates_nothing() {
        // Alternating full-cue and empty ticks: 600 updates through excite,
        // slew and ramps inside the counting window.
        let cues = [
            hit_at(200),
            hit_at(2000),
            knockdown_at(300),
            PlacedCue::new(
                FightCue::Parried {
                    attacker: 1,
                    victim: 0,
                },
                500,
            ),
            PlacedCue::new(
                FightCue::Dodged {
                    attacker: 1,
                    victim: 0,
                },
                800,
            ),
            PlacedCue::new(
                FightCue::Blocked {
                    attacker: 0,
                    victim: 1,
                    chip: 24,
                },
                1200,
            ),
            PlacedCue::new(FightCue::StaminaOut { fighter: 1 }, 400),
            knockdown_at(30_000),
        ];
        let mut director = Director::scripted();
        for tick in 0..16u32 {
            let events = if tick.is_multiple_of(2) {
                &cues[..]
            } else {
                &[]
            };
            director.update(events);
        }
        let ((folded, lo, hi), heap) = measure(|| {
            let mut folded = 0u32;
            let (mut lo, mut hi) = (1.0f32, 0.0f32);
            for tick in 0..600u32 {
                let events = if tick.is_multiple_of(2) {
                    &cues[..]
                } else {
                    &[]
                };
                let layers = director.update(events);
                lo = lo.min(layers.intensity);
                hi = hi.max(layers.intensity);
                folded ^= layers
                    .intensity
                    .to_bits()
                    .wrapping_add(tick.wrapping_mul(0x9E37_79B9));
            }
            (folded, lo, hi)
        });
        std::hint::black_box(folded);
        assert_eq!(heap.calls, 0, "steady-state director must not allocate");
        assert!(hi - lo > 0.001, "sanity: heat moved during the window");
    }

    #[test]
    #[ignore = "timing: 0.05 ms/update budget probe, run under --perf in release"]
    fn budget_probe_per_update() {
        // 32-cue arena burst, one room, 4000 ticks: mean update time must
        // stay under BUDGET_MS_PER_UPDATE. Wall clock lives only in this
        // ignored probe, never in the update path.
        let cues = [hit_at(300); 32];
        let mut director = Director::scripted();
        director.update(&cues);
        let start = std::time::Instant::now();
        let rounds = 4000u32;
        let mut folded = 0u32;
        for _ in 0..rounds {
            let layers = director.update(&cues);
            folded ^= layers.intensity.to_bits();
        }
        std::hint::black_box(folded);
        let mean_ms = start.elapsed().as_secs_f64() * 1000.0 / f64::from(rounds);
        assert!(
            mean_ms < BUDGET_MS_PER_UPDATE as f64,
            "mean {mean_ms} ms per update over budget"
        );
    }

    /// Heavy knockdowns for the first 400 ticks, then silence.
    fn fight_then_lull(tick: u32) -> Vec<PlacedCue> {
        if tick % 20 < 2 && tick < 400 {
            vec![knockdown_at(300)]
        } else {
            vec![]
        }
    }

    #[test]
    fn director_intensity_drives_tension_levels_without_flapping() {
        use crate::music::{Tension, TensionTracker};
        let run = || {
            let mut director = Director::scripted();
            let mut tracker = TensionTracker::new();
            let mut seen = vec![(0u32, Tension::Calm)];
            for tick in 0..1_500u32 {
                let layers = director.update(&fight_then_lull(tick));
                if let Some(level) = tracker.update(layers.intensity) {
                    seen.push((tick, level));
                }
            }
            (tracker.level(), seen)
        };
        let (level, seen) = run();
        let levels: Vec<Tension> = seen.iter().map(|&(_, l)| l).collect();
        assert!(
            levels.contains(&Tension::Climax),
            "heavy fighting reaches the climax: {seen:?}"
        );
        assert_eq!(
            level,
            Tension::Calm,
            "a long lull settles back to calm: {seen:?}"
        );
        assert!(seen.len() <= 7, "{} changes: {seen:?}", seen.len() - 1);
        assert_eq!(run(), (level, seen), "replays exactly");
    }
}
