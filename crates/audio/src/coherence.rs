//! coherence: hit-event-synced impact audio over the spatial mix.
//!
//! Owner: audio lane; consumer: combat presentation (game lane), which maps
//! each fight event onto [`FightCue`](crate::crowd::FightCue), calls
//! [`CoherenceSync::notify_hit`] at the hit tick, plays the [`BankSound`]
//! from [`voice_for`] through the [`SpatialMixer`](crate::spatial::SpatialMixer),
//! calls [`notify_audio`](CoherenceSync::notify_audio) on the same tick, and
//! ticks [`tick`](CoherenceSync::tick) once per fixed tick.
//! Units: ticks are `u64`, distances are integer cm, gains are permille
//! `0..=1000`. Determinism: same cue script plus same tick cadence give the
//! same [`CoherenceReport`]; the tick path is integer-only, presentation
//! only, never hashed (golden d57b5c8ae3096614 stands). Errors: typed
//! [`CoherenceError`]; setters refuse, the tick path saturates. Budget: at
//! most [`MAX_PENDING`] integer slot scans per call on the game thread.

use crate::bank::BankSound;
use crate::crowd::FightCue;
use crate::spatial::{FAR_CM, NEAR_CM};
use std::fmt::{Display, Formatter, Result as FmtResult};

/// Most armed hit slots one call scans; longer bursts evict the oldest first
/// and count it missed, so the tick cost stays bounded. Never panics.
pub const MAX_PENDING: usize = 64;
/// Ticks late an impact voice may land and still count synced (frame-exact:
/// the voice must fire on the hit tick). Part of [`CoherenceConfig`].
pub const TOLERANCE_DEFAULT_TICKS: u32 = 0;
/// Ticks an unanswered hit stays armed before it counts missed (half a
/// second at 60 Hz). Part of [`CoherenceConfig`].
pub const EXPIRY_DEFAULT_TICKS: u32 = 30;
/// Largest legal expiry (4 s at 60 Hz); caps stale-slot lifetime.
pub const EXPIRY_MAX_TICKS: u32 = 240;
/// Full gain in permille (integer attenuation scale, see [`gain_permille`]).
pub const GAIN_FULL_PERMILLE: u16 = 1000;

/// Typed coherence failure. Config setters return these; the per-tick path
/// saturates instead so the mix never drops on data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoherenceError {
    /// A config field left its range; names the field.
    BadConfig {
        /// Static field name (`"tolerance"`, `"expiry"`).
        name: &'static str,
    },
}

impl Display for CoherenceError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::BadConfig { name } => write!(f, "coherence: bad config {name}"),
        }
    }
}

impl std::error::Error for CoherenceError {}

/// Sync window config: how late a voice may land, how long a hit waits.
/// Game thread owns it; the tracker only reads it back out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoherenceConfig {
    /// Ticks late a voice may land and still count synced.
    pub tolerance_ticks: u32,
    /// Ticks an unanswered hit stays armed before it counts missed.
    pub expiry_ticks: u32,
}

impl CoherenceConfig {
    /// Frame-exact default: voices fire on the hit tick, misses time out
    /// after [`EXPIRY_DEFAULT_TICKS`]. Valid by construction.
    pub fn coherent() -> Self {
        Self {
            tolerance_ticks: TOLERANCE_DEFAULT_TICKS,
            expiry_ticks: EXPIRY_DEFAULT_TICKS,
        }
    }

    /// Check ranges: tolerance must fit inside expiry, expiry inside
    /// [`EXPIRY_MAX_TICKS`].
    pub fn validate(&self) -> Result<(), CoherenceError> {
        if self.tolerance_ticks > self.expiry_ticks {
            return Err(CoherenceError::BadConfig { name: "tolerance" });
        }
        if self.expiry_ticks > EXPIRY_MAX_TICKS {
            return Err(CoherenceError::BadConfig { name: "expiry" });
        }
        Ok(())
    }
}

/// Variant order of [`FightCue`] as a small integer for slot matching.
/// Stable by construction; match arms follow the crowd enum order.
pub fn cue_kind(cue: FightCue) -> u8 {
    match cue {
        FightCue::Hit { .. } => 0,
        FightCue::Blocked { .. } => 1,
        FightCue::Parried { .. } => 2,
        FightCue::Dodged { .. } => 3,
        FightCue::StaminaOut { .. } => 4,
        FightCue::Knockdown { .. } => 5,
    }
}

/// Placeholder voice for one cue, or `None` when the cue stays silent.
/// Contact cues share the bank thump until recorded clips land without
/// changing this shape; the dodge whoosh shares the tap; stamina-out
/// whiffs start nothing and expect silence.
pub fn voice_for(cue: FightCue) -> Option<BankSound> {
    match cue {
        FightCue::Hit { .. }
        | FightCue::Blocked { .. }
        | FightCue::Parried { .. }
        | FightCue::Knockdown { .. } => Some(BankSound::Hit),
        FightCue::Dodged { .. } => Some(BankSound::Footstep),
        FightCue::StaminaOut { .. } => None,
    }
}

/// Integer distance gain in permille `0..=1000`: full inside [`NEAR_CM`],
/// zero at and past [`FAR_CM`], linear between, matching the SN mix
/// falloff with integer division only. Never panics.
pub fn gain_permille(distance_cm: u32) -> u16 {
    if distance_cm <= NEAR_CM {
        GAIN_FULL_PERMILLE
    } else if distance_cm >= FAR_CM {
        0
    } else {
        u16::try_from((FAR_CM - distance_cm) * u32::from(GAIN_FULL_PERMILLE) / (FAR_CM - NEAR_CM))
            .map_or(GAIN_FULL_PERMILLE, |v| v)
    }
}

/// Whole-run sync answer, all counters saturating. `synced + late +
/// missed == expected` always holds; `spurious` counts voices with no hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoherenceReport {
    /// Hits that armed a voice expectation.
    pub expected: u32,
    /// Voices that fired inside the tolerance window.
    pub synced: u32,
    /// Voices that fired after tolerance but inside expiry.
    pub late: u32,
    /// Hits whose voice never fired before expiry (plus cap evictions).
    pub missed: u32,
    /// Voices with no armed hit behind them.
    pub spurious: u32,
}

impl CoherenceReport {
    /// Share of expected hits synced on time, permille `0..=1000`;
    /// a silent run (nothing expected) reads full.
    pub fn coherence_permille(&self) -> u32 {
        if self.expected == 0 {
            u32::from(GAIN_FULL_PERMILLE)
        } else {
            match u32::try_from(
                u64::from(self.synced) * u64::from(GAIN_FULL_PERMILLE) / u64::from(self.expected),
            ) {
                Ok(v) => v,
                Err(_) => u32::from(GAIN_FULL_PERMILLE),
            }
        }
    }

    /// True when every expected voice fired on time and no stray voice
    /// fired: no late, no missed, no spurious.
    pub fn is_coherent(&self) -> bool {
        self.late == 0 && self.missed == 0 && self.spurious == 0
    }
}

/// One armed hit expectation: cue kind plus the tick it fired on.
#[derive(Clone, Copy, Debug)]
struct Slot {
    armed: bool,
    kind: u8,
    tick: u64,
}

/// Hit-event-synced impact tracker: fixed-slot expectations matched to
/// same-tick voices. The game thread builds it, notifies hits and voices,
/// and reads [`CoherenceReport`]; it keeps scalars plus one fixed array,
/// so the steady-state tick borrows nothing and touches no heap.
#[derive(Clone, Debug)]
pub struct CoherenceSync {
    config: CoherenceConfig,
    tick: u64,
    slots: [Slot; MAX_PENDING],
    expected: u32,
    synced: u32,
    late: u32,
    missed: u32,
    spurious: u32,
}

impl CoherenceSync {
    /// Build on a caller config; refuses a bad config and builds nothing.
    ///
    /// Errors: [`CoherenceError::BadConfig`] from [`CoherenceConfig::validate`].
    pub fn new(config: CoherenceConfig) -> Result<Self, CoherenceError> {
        config.validate()?;
        Ok(Self {
            config,
            tick: 0,
            slots: [Slot {
                armed: false,
                kind: 0,
                tick: 0,
            }; MAX_PENDING],
            expected: 0,
            synced: 0,
            late: 0,
            missed: 0,
            spurious: 0,
        })
    }

    /// Build on the frame-exact default config; valid by construction.
    pub fn coherent() -> Self {
        Self {
            config: CoherenceConfig::coherent(),
            tick: 0,
            slots: [Slot {
                armed: false,
                kind: 0,
                tick: 0,
            }; MAX_PENDING],
            expected: 0,
            synced: 0,
            late: 0,
            missed: 0,
            spurious: 0,
        }
    }

    /// Swap the sync window; a bad window refuses and keeps the old one.
    ///
    /// Errors: [`CoherenceError::BadConfig`] from [`CoherenceConfig::validate`].
    pub fn set_config(&mut self, config: CoherenceConfig) -> Result<(), CoherenceError> {
        config.validate()?;
        self.config = config;
        Ok(())
    }

    /// Active sync window (copy; the struct owns its state).
    pub fn config(&self) -> CoherenceConfig {
        self.config
    }

    /// Fixed-tick clock (one [`tick`](Self::tick) per tick).
    pub fn tick_count(&self) -> u64 {
        self.tick
    }

    /// Armed (unanswered) hit count.
    pub fn pending(&self) -> u32 {
        let mut n = 0u32;
        for slot in self.slots.iter() {
            if slot.armed {
                n = n.saturating_add(1);
            }
        }
        n
    }

    /// Back to a silent tracker at tick 0; keeps the config.
    pub fn reset(&mut self) {
        for slot in self.slots.iter_mut() {
            slot.armed = false;
        }
        self.tick = 0;
        self.expected = 0;
        self.synced = 0;
        self.late = 0;
        self.missed = 0;
        self.spurious = 0;
    }

    /// Record a fight hit at this tick: arms one expectation unless the cue
    /// is silent ([`StaminaOut`](FightCue::StaminaOut)). Past the cap the
    /// oldest armed hit evicts as missed, in index order on ties. Integer
    /// scans only; never panics.
    pub fn notify_hit(&mut self, cue: FightCue) {
        if voice_for(cue).is_none() {
            return;
        }
        self.expected = self.expected.saturating_add(1);
        let kind = cue_kind(cue);
        for slot in self.slots.iter_mut() {
            if !slot.armed {
                slot.armed = true;
                slot.kind = kind;
                slot.tick = self.tick;
                return;
            }
        }
        let mut oldest = 0usize;
        for (i, slot) in self.slots.iter().enumerate() {
            if slot.tick < self.slots[oldest].tick {
                oldest = i;
            }
        }
        self.slots[oldest].kind = kind;
        self.slots[oldest].tick = self.tick;
        self.missed = self.missed.saturating_add(1);
    }

    /// Record an impact voice firing at this tick: matches the earliest
    /// armed hit of the same kind inside the expiry window. On time counts
    /// synced, past tolerance counts late, with no armed hit counts
    /// spurious. Integer compares only; never panics.
    pub fn notify_audio(&mut self, cue: FightCue) {
        let kind = cue_kind(cue);
        let mut best: Option<usize> = None;
        let mut best_tick = u64::MAX;
        for (i, slot) in self.slots.iter().enumerate() {
            if !slot.armed || slot.kind != kind || slot.tick > self.tick {
                continue;
            }
            if self.tick - slot.tick > u64::from(self.config.expiry_ticks) {
                continue;
            }
            if slot.tick < best_tick {
                best_tick = slot.tick;
                best = Some(i);
            }
        }
        match best {
            Some(i) => {
                let age = self.tick - self.slots[i].tick;
                self.slots[i].armed = false;
                if age <= u64::from(self.config.tolerance_ticks) {
                    self.synced = self.synced.saturating_add(1);
                } else {
                    self.late = self.late.saturating_add(1);
                }
            }
            None => {
                self.spurious = self.spurious.saturating_add(1);
            }
        }
    }

    /// Advance one fixed tick: the clock runs and hits armed past expiry
    /// count missed. Integer compares only; never panics.
    pub fn tick(&mut self) {
        self.tick = self.tick.saturating_add(1);
        for slot in self.slots.iter_mut() {
            if slot.armed && self.tick.wrapping_sub(slot.tick) > u64::from(self.config.expiry_ticks)
            {
                slot.armed = false;
                self.missed = self.missed.saturating_add(1);
            }
        }
    }

    /// Whole-run sync answer (copy; the struct owns its state).
    pub fn report(&self) -> CoherenceReport {
        CoherenceReport {
            expected: self.expected,
            synced: self.synced,
            late: self.late,
            missed: self.missed,
            spurious: self.spurious,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::alloc_probe::measure;

    fn hit() -> FightCue {
        FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 120,
        }
    }

    fn knockdown() -> FightCue {
        FightCue::Knockdown {
            attacker: 0,
            victim: 1,
        }
    }

    fn scenario_report(cue: FightCue) -> CoherenceReport {
        let mut sync = CoherenceSync::coherent();
        sync.notify_hit(cue);
        sync.notify_audio(cue);
        sync.tick();
        sync.report()
    }

    #[test]
    fn scenario_hit_synced() {
        let report = scenario_report(hit());
        assert_eq!(
            (report.expected, report.synced),
            (1, 1),
            "one hit fires one voice on time"
        );
        assert_eq!((report.late, report.missed, report.spurious), (0, 0, 0));
        assert!(report.is_coherent());
        assert_eq!(report.coherence_permille(), 1000);
    }

    #[test]
    fn scenario_blocked_chip_synced() {
        let cue = FightCue::Blocked {
            attacker: 0,
            victim: 1,
            chip: 24,
        };
        let report = scenario_report(cue);
        assert_eq!((report.expected, report.synced), (1, 1));
        assert!(report.is_coherent(), "chip damage still lands a voice");
    }

    #[test]
    fn scenario_parry_synced() {
        let cue = FightCue::Parried {
            attacker: 1,
            victim: 0,
        };
        let report = scenario_report(cue);
        assert_eq!((report.expected, report.synced), (1, 1));
        assert!(report.is_coherent());
    }

    #[test]
    fn scenario_dodge_synced() {
        let cue = FightCue::Dodged {
            attacker: 1,
            victim: 0,
        };
        let report = scenario_report(cue);
        assert_eq!((report.expected, report.synced), (1, 1));
        assert!(report.is_coherent(), "the dodge whoosh fires on the cue");
    }

    #[test]
    fn scenario_knockdown_synced() {
        let report = scenario_report(knockdown());
        assert_eq!((report.expected, report.synced), (1, 1));
        assert!(report.is_coherent());
    }

    #[test]
    fn scenario_stamina_out_silent() {
        let mut sync = CoherenceSync::coherent();
        sync.notify_hit(FightCue::StaminaOut { fighter: 1 });
        assert_eq!(sync.pending(), 0, "a whiff arms no voice");
        for _ in 0..EXPIRY_DEFAULT_TICKS + 4 {
            sync.tick();
        }
        let report = sync.report();
        assert_eq!(report.expected, 0, "silence expects nothing");
        assert_eq!((report.missed, report.spurious), (0, 0));
        assert!(report.is_coherent());
        assert_eq!(report.coherence_permille(), 1000);
    }

    #[test]
    fn scenario_late_voice_breaks_sync() {
        let mut sync = CoherenceSync::coherent();
        sync.notify_hit(hit());
        sync.tick();
        sync.notify_audio(hit());
        let report = sync.report();
        assert_eq!(report.synced, 0, "a voice one tick late is not synced");
        assert_eq!(report.late, 1);
        assert!(!report.is_coherent(), "late audio breaks coherence");
        assert_eq!(report.coherence_permille(), 0);
    }

    #[test]
    fn scenario_missing_voice_counts_missed() {
        let mut sync = CoherenceSync::coherent();
        sync.notify_hit(knockdown());
        for _ in 0..EXPIRY_DEFAULT_TICKS + 1 {
            sync.tick();
        }
        let report = sync.report();
        assert_eq!((report.expected, report.synced), (1, 0));
        assert_eq!(report.missed, 1, "a dropped voice times out as missed");
        assert!(!report.is_coherent());
    }

    #[test]
    fn scenario_same_tick_burst_all_synced() {
        let mut sync = CoherenceSync::coherent();
        sync.notify_hit(hit());
        sync.notify_hit(hit());
        sync.notify_hit(knockdown());
        assert_eq!(sync.pending(), 3);
        sync.notify_audio(hit());
        sync.notify_audio(hit());
        sync.notify_audio(knockdown());
        sync.tick();
        let report = sync.report();
        assert_eq!((report.expected, report.synced), (3, 3));
        assert!(report.is_coherent(), "a trade burst stays frame-exact");
    }

    #[test]
    fn scenario_consumer_edge_bank_spatial_synced() {
        use crate::bank::Bank;
        use crate::spatial::{Pos2, SpatialMixer};
        // Combat presentation wiring: the mapped bank voice plays positioned
        // on the SN mix on the same tick the hit lands.
        let bank = Bank::placeholder(8_000);
        let mut mix = SpatialMixer::new(8_000);
        mix.set_listener(Pos2::new(0, -800), 0);
        let mut sync = CoherenceSync::coherent();
        let cue = hit();
        let sound = voice_for(cue).expect("hit maps to a bank voice");
        sync.notify_hit(cue);
        mix.play_at(bank.get(sound), Pos2::new(200, 0), 1.0);
        sync.notify_audio(cue);
        let bytes = mix.render_stereo(4);
        assert!(
            bytes.iter().any(|&s| s != 0.0),
            "the hit voice must be audible on the SN mix"
        );
        assert_eq!(mix.commands().len(), 1, "one positioned voice logged");
        sync.tick();
        let report = sync.report();
        assert_eq!((report.expected, report.synced), (1, 1));
        assert!(report.is_coherent());
    }

    #[test]
    fn script_twice_same_report() {
        let script = [
            hit(),
            FightCue::Parried {
                attacker: 1,
                victim: 0,
            },
            FightCue::Dodged {
                attacker: 1,
                victim: 0,
            },
            hit(),
            knockdown(),
            FightCue::Blocked {
                attacker: 0,
                victim: 1,
                chip: 24,
            },
            FightCue::StaminaOut { fighter: 1 },
        ];
        let run = || {
            let mut sync = CoherenceSync::coherent();
            for (t, cue) in script.iter().enumerate() {
                if t.is_multiple_of(2) {
                    sync.tick();
                }
                sync.notify_hit(*cue);
                if voice_for(*cue).is_some() {
                    sync.notify_audio(*cue);
                }
                sync.tick();
            }
            (sync.report(), sync.tick_count(), sync.pending())
        };
        let (first, first_ticks, first_pending) = run();
        let (second, second_ticks, second_pending) = run();
        assert_eq!((first, first_ticks), (second, second_ticks));
        assert_eq!(first_pending, second_pending);
        assert!(first.is_coherent(), "the scripted fight stays synced");
        assert_eq!(first.expected, 6, "six audible cues, one silent whiff");
    }

    #[test]
    fn tolerance_window_still_syncs_next_tick() {
        let config = CoherenceConfig {
            tolerance_ticks: 1,
            expiry_ticks: 30,
        };
        let mut sync = CoherenceSync::new(config).expect("window is legal");
        sync.notify_hit(hit());
        sync.tick();
        sync.notify_audio(hit());
        let report = sync.report();
        assert_eq!(report.synced, 1, "tolerance 1 covers a one-tick pipe");
        assert!(report.is_coherent());
    }

    #[test]
    fn invalid_config_refuses_and_keeps_old() {
        let mut sync = CoherenceSync::coherent();
        let good = sync.config();
        let bad = CoherenceConfig {
            tolerance_ticks: 31,
            expiry_ticks: 30,
        };
        assert_eq!(
            sync.set_config(bad),
            Err(CoherenceError::BadConfig { name: "tolerance" })
        );
        let bad = CoherenceConfig {
            tolerance_ticks: 0,
            expiry_ticks: EXPIRY_MAX_TICKS + 1,
        };
        assert_eq!(
            sync.set_config(bad),
            Err(CoherenceError::BadConfig { name: "expiry" })
        );
        assert_eq!(sync.config(), good, "refusals keep the old window");
        assert_eq!(
            CoherenceSync::new(bad).expect_err("bad window refuses"),
            CoherenceError::BadConfig { name: "expiry" }
        );
        assert_eq!(
            format!("{}", CoherenceError::BadConfig { name: "tolerance" }),
            "coherence: bad config tolerance"
        );
        let retune = CoherenceConfig {
            tolerance_ticks: 1,
            expiry_ticks: 60,
        };
        sync.set_config(retune).expect("legal retune lands");
        assert_eq!(sync.config(), retune);
        sync.notify_hit(hit());
        sync.notify_audio(hit());
        sync.reset();
        assert_eq!(
            sync.report(),
            CoherenceReport {
                expected: 0,
                synced: 0,
                late: 0,
                missed: 0,
                spurious: 0,
            }
        );
        assert_eq!(sync.tick_count(), 0);
        assert_eq!(sync.config(), retune, "reset keeps the retune");
    }

    #[test]
    fn pending_cap_evicts_oldest_as_missed() {
        let mut sync = CoherenceSync::coherent();
        for _ in 0..MAX_PENDING + 4 {
            sync.notify_hit(hit());
        }
        assert_eq!(sync.pending() as usize, MAX_PENDING);
        let report = sync.report();
        assert_eq!(report.missed, 4, "overflow evicts the oldest first");
        assert!(!report.is_coherent());
    }

    #[test]
    fn stray_voice_counts_spurious() {
        let mut sync = CoherenceSync::coherent();
        sync.notify_audio(hit());
        sync.notify_audio(FightCue::StaminaOut { fighter: 0 });
        let report = sync.report();
        assert_eq!(report.spurious, 2, "voices with no hit are stray");
        assert_eq!(report.expected, 0);
        assert!(!report.is_coherent());
    }

    #[test]
    fn voice_map_covers_all_cues() {
        assert!(voice_for(hit()).is_some());
        assert!(voice_for(knockdown()).is_some());
        assert!(voice_for(FightCue::Parried {
            attacker: 0,
            victim: 1,
        })
        .is_some());
        assert!(voice_for(FightCue::StaminaOut { fighter: 0 }).is_none());
        let kinds = [
            cue_kind(hit()),
            cue_kind(FightCue::Blocked {
                attacker: 0,
                victim: 1,
                chip: 1,
            }),
            cue_kind(FightCue::Parried {
                attacker: 0,
                victim: 1,
            }),
            cue_kind(FightCue::Dodged {
                attacker: 0,
                victim: 1,
            }),
            cue_kind(FightCue::StaminaOut { fighter: 0 }),
            cue_kind(knockdown()),
        ];
        assert_eq!(kinds, [0, 1, 2, 3, 4, 5], "kinds stay distinct per cue");
    }

    #[test]
    fn gain_permille_integer_script() {
        assert_eq!(gain_permille(0), 1000);
        assert_eq!(gain_permille(NEAR_CM), 1000);
        assert_eq!(gain_permille((NEAR_CM + FAR_CM) / 2), 500);
        assert_eq!(gain_permille(FAR_CM), 0);
        assert_eq!(gain_permille(FAR_CM + 500), 0);
        assert_eq!(gain_permille(u32::MAX), 0);
    }

    #[test]
    fn coherence_permille_partial_credit() {
        let mut sync = CoherenceSync::coherent();
        sync.notify_hit(hit());
        sync.notify_hit(hit());
        sync.notify_audio(hit());
        for _ in 0..EXPIRY_DEFAULT_TICKS + 1 {
            sync.tick();
        }
        let report = sync.report();
        assert_eq!(report.coherence_permille(), 500, "one of two synced");
        assert!(!report.is_coherent());
    }

    #[test]
    fn hot_path_names_no_heap_or_float_types() {
        // Structural guard beside the counting window below: the code above
        // the cfg(test) marker must not name heap, lock, print, failure or
        // float APIs, so a per-tick container or float step cannot land
        // unnoticed even if the window ever goes quiet.
        const SRC: &str = include_str!("coherence.rs");
        let head = SRC
            .split("#[cfg(test)]")
            .next()
            .expect("coherence.rs keeps its cfg(test) marker");
        for token in [
            "Vec",
            "Box<",
            "String",
            "vec!",
            "format!",
            "collect",
            "HashMap",
            "BTreeMap",
            "println",
            "print!",
            "eprintln",
            "dbg!",
            "Rc<",
            "Arc<",
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
            "f32",
            "f64",
            ".sin(",
            ".cos(",
            "powf",
            "sqrt",
            "exp(",
            "alloc",
        ] {
            assert!(
                !head.contains(token),
                "hot path must not name heap/float/failure API: {token}"
            );
        }
    }

    #[test]
    fn steady_state_tick_allocates_nothing() {
        // Alternating full-cue and quiet ticks: 600 notify plus tick rounds
        // inside the counting window.
        let cues = [
            hit(),
            FightCue::Blocked {
                attacker: 0,
                victim: 1,
                chip: 24,
            },
            FightCue::Parried {
                attacker: 1,
                victim: 0,
            },
            FightCue::Dodged {
                attacker: 1,
                victim: 0,
            },
            knockdown(),
        ];
        let mut sync = CoherenceSync::coherent();
        for _ in 0..16u32 {
            for cue in cues.iter() {
                sync.notify_hit(*cue);
                sync.notify_audio(*cue);
            }
            sync.notify_hit(FightCue::StaminaOut { fighter: 1 });
            sync.tick();
        }
        let (folded, heap) = measure(|| {
            let mut folded = 0u64;
            for round in 0..600u32 {
                for cue in cues.iter() {
                    sync.notify_hit(*cue);
                    sync.notify_audio(*cue);
                }
                sync.notify_hit(FightCue::StaminaOut { fighter: 1 });
                sync.tick();
                let report = sync.report();
                folded ^= report.synced as u64 ^ (u64::from(round) ^ sync.tick_count());
            }
            folded
        });
        std::hint::black_box(folded);
        assert_eq!(heap.calls, 0, "steady-state sync must not allocate");
        assert!(folded != 0, "sanity: hits synced during the window");
    }
}
