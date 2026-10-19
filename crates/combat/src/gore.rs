//! Gore and blood events for the renderer: presentation only, never authoritative.
//!
//! Consumer: render decals/particles read [`GoreEvent`]s; the arena (via its
//! match scheduler) feeds each tick's [`FightEvent`]s plus the authority tick it
//! stepped into [`gore_for_events`]. Data owner: `engine-combat`. Units:
//! ticks at 60 Hz, positions in integer centimetres, severity in permille
//! (`0..=1000`, same scale as hp). Determinism scope: the mapping is pure
//! (integer math and event order only); same tick plus same event slice
//! gives the same gore stream on every PC. Errors: none, the mapping is
//! infallible and never panics; unknown event shapes simply emit nothing.
//!
//! Presentation-only means these events NEVER alter hit outcomes, damage or
//! hashes: [`gore_for_events`] takes `&[FightEvent]` by shared reference and
//! never touches [`FightState`], so collecting gore cannot change a replay.
//! Integer centimetres only, so there are no NaN positions by construction.

use crate::fight::{FightEvent, Tables, MAX_HP_PERMILLE, TABLES_V1};
use engine_core::checked::saturating_u32_to_i32;
use engine_core::Fnv64;

/// Presentation gore policy: render richness only, never simulation.
///
/// `Full` keeps today's blood plus the fatal-only sever cue, `Reduced`
/// keeps blood but drops the severed-part cue, `Off` drops all blood and
/// emits a non-bloody impact flash plus a guard/downed cue instead. The
/// setting never feeds [`FightState`] or its hash; the arena and render
/// read it as a presentation filter over [`gore_for_events`] output.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum GoreSetting {
    /// No blood: impact flash plus guard/downed cue only.
    Off,
    /// Blood without the severed-part exaggeration.
    Reduced,
    /// Today's mapping: blood plus the fatal-only sever cue.
    #[default]
    Full,
}

/// Presentation gore kind for render decals/particles.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GoreKind {
    /// Airborne blood particles at the victim's torso.
    BloodSpray,
    /// Blood stamp on the sand at the victim's feet.
    DecalStamp,
    /// Fatal-only exaggeration cue (knockdown), still presentation only.
    DismemberLite,
    /// Non-bloody hit cue for [`GoreSetting::Off`]: a bright flash at the
    /// torso anchor, same integer-centimetre position as a spray.
    ImpactFlash,
    /// Guard or downed-pose cue for [`GoreSetting::Off`] knockdowns: tells
    /// anim to hold a guard or downed pose instead of showing a sever.
    GuardCue,
}

impl GoreKind {
    /// True for blood reads render must hide under [`GoreSetting::Off`].
    pub const fn is_blood(self) -> bool {
        matches!(
            self,
            GoreKind::BloodSpray | GoreKind::DecalStamp | GoreKind::DismemberLite
        )
    }
}

/// One presentation gore event. Never hashed into simulation state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GoreEvent {
    /// Authority tick the caller stepped (events carry no clock).
    pub tick: u32,
    /// Victim fighter slot, copied from the fight event.
    pub victim: u8,
    /// Which render effect reads this event.
    pub kind: GoreKind,
    /// Integer centimetres, `[x, y, z]`; y `== 0` for ground decals.
    pub pos_cm: [i32; 3],
    /// Intensity in permille, always `0..=1000`.
    pub severity_permille: u16,
}

/// Torso height in centimetres for spray positions.
pub const SPRAY_HEIGHT_CM: i32 = 110;
/// Lateral centimetres per fighter slot from the bout centre.
pub const SLOT_SPACING_CM: i32 = 150;
/// Deterministic per-tick jitter bound in centimetres (`±JITTER_CM`).
pub const JITTER_CM: i32 = 10;

/// Table-driven severity from damage in permille, bounded to `0..=1000`.
///
/// Damage already arrives in hp-permille from [`Tables`] rows, so severity
/// tracks table tuning directly; the clamp keeps custom rows safe.
pub const fn severity_for_damage(damage: u16) -> u16 {
    if damage > MAX_HP_PERMILLE {
        MAX_HP_PERMILLE
    } else {
        damage
    }
}

/// Ticks of one visible pulse beat for a wound cue (integer ticks, 60 Hz).
pub const WOUND_PULSE_TICKS: u32 = 6;
/// Ticks from wound opening until the cue tapers to zero (integer ticks).
pub const WOUND_FADE_TICKS: u32 = 48;

/// Presentation wound as a value: a cut opened by one fight event.
///
/// Consumer: render decals/particles keep these values and poll
/// [`wound_pulse_permille`] (or [`Wound::pulse_at`]) each tick for the
/// tapered intensity. Data owner: `engine-combat`. Units: ticks at 60 Hz,
/// severity in permille (`0..=1000`), positions in integer centimetres.
/// Determinism scope: pure integer math and event order only; same tick
/// plus same event slice gives the same wounds on every PC. Errors: none.
///
/// Idea-only port of the bevy_carnage wound-as-value plus pulse-taper
/// pattern (Apache-2.0): wounds are plain values with an integer-tick
/// pulse taper, never entities and never simulation. No bytes copied.
/// Presentation-only like [`GoreEvent`]: collecting wounds never touches
/// [`FightState`] and never enters its hash.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Wound {
    /// Victim fighter slot, copied from the fight event.
    pub victim: u8,
    /// Authority tick the wound opened (events carry no clock).
    pub opened_tick: u32,
    /// Cut size in permille, always `0..=1000`.
    pub severity_permille: u16,
    /// Integer centimetres, `[x, y, z]` at the torso anchor.
    pub pos_cm: [i32; 3],
}

impl Wound {
    /// Tapered intensity at `now_tick` in permille (`0..=severity`).
    ///
    /// Units: ticks at 60 Hz in, permille out. Determinism: integer math
    /// only, saturating on pre-opening reads. Errors: none.
    pub fn pulse_at(self, now_tick: u32) -> u16 {
        wound_pulse_permille(self.opened_tick, now_tick, self.severity_permille)
    }
}

/// Integer-tick pulse taper for a wound cue in permille.
///
/// `opened_tick` is the tick the wound opened, `now_tick` is the tick
/// being drawn, `severity` is the opening size (`0..=1000`). Returns
/// `0` before opening and at/after `opened + WOUND_FADE_TICKS`, else a
/// linear taper with a small integer pulse ripple (`age % PULSE`), so
/// the cue beats visibly while fading. Integer math only (`+ - * / %`),
/// no floats, no RNG, saturating: same inputs give the same output on
/// every PC. Units: ticks in, permille out. Errors: none.
pub fn wound_pulse_permille(opened_tick: u32, now_tick: u32, severity: u16) -> u16 {
    if now_tick < opened_tick {
        return 0;
    }
    let age = now_tick - opened_tick;
    if age >= WOUND_FADE_TICKS {
        return 0;
    }
    let capped = if severity > MAX_HP_PERMILLE {
        MAX_HP_PERMILLE
    } else {
        severity
    };
    let clamped = u32::from(capped);
    // Linear envelope: full at age 0, zero at the fade horizon.
    let base = (clamped * (WOUND_FADE_TICKS - age)) / WOUND_FADE_TICKS;
    // Pulse ripple: up to ~20% dip inside each beat, integer only.
    let phase = age % WOUND_PULSE_TICKS;
    let ripple = (base * phase) / (WOUND_PULSE_TICKS * 4);
    match u16::try_from(base - ripple) {
        Ok(intensity) => intensity,
        Err(_) => MAX_HP_PERMILLE,
    }
}

/// Pure mapping from one tick's fight events to presentation wound values.
///
/// `tick` is the authority tick the caller stepped. Mapping: `Hit` opens
/// one cut sized by damage, `Knockdown` opens one full-severity cut;
/// `Blocked`, `Dodged`, `Parried` and `StaminaOut` open nothing (a parry
/// graze is blood without a cut). Output order follows input order.
/// Hash-neutral: reads events by shared reference, never touches state.
pub fn wounds_for_events(tick: u32, events: &[FightEvent]) -> Vec<Wound> {
    wounds_for_events_with_tables(tick, events, &TABLES_V1)
}

/// Pure mapping under explicit table rows: same rows plus same tick and
/// events give the same wounds. Tables only matter through clamping today;
/// damage already arrives sized, so the row set travels for future tuning.
pub fn wounds_for_events_with_tables(
    tick: u32,
    events: &[FightEvent],
    _tables: &Tables,
) -> Vec<Wound> {
    let mut wounds = Vec::with_capacity(events.len());
    for event in events.iter() {
        match *event {
            FightEvent::Hit { victim, damage, .. } => {
                wounds.push(Wound {
                    victim,
                    opened_tick: tick,
                    severity_permille: severity_for_damage(damage),
                    pos_cm: spray_pos_cm(tick, victim),
                });
            }
            FightEvent::Knockdown { victim, .. } => {
                wounds.push(Wound {
                    victim,
                    opened_tick: tick,
                    severity_permille: MAX_HP_PERMILLE,
                    pos_cm: spray_pos_cm(tick, victim),
                });
            }
            FightEvent::Blocked { .. }
            | FightEvent::Dodged { .. }
            | FightEvent::Parried { .. }
            | FightEvent::StaminaOut { .. } => {}
        }
    }
    wounds
}

/// Stable checksum of a wound stream for fixtures (FNV-1a via owner).
///
/// Units: wounds in, `u64` hash out. Determinism: field order is
/// victim, opened tick, severity, position, all little-endian; no
/// pointers, floats or map order. Errors: none. Never feeds simulation.
pub fn wound_stream_hash(wounds: &[Wound]) -> u64 {
    let mut hasher = Fnv64::new();
    for wound in wounds {
        hasher.write_u8(wound.victim);
        hasher.write_u32(wound.opened_tick);
        hasher.write_u16(wound.severity_permille);
        for axis in wound.pos_cm {
            hasher.write_i32(axis);
        }
    }
    hasher.finish()
}

/// Max live wounds one render feed keeps; oldest drops first.
///
/// Budget: one board holds at most this many values, so a wild tick cannot
/// grow the presentation stream without bound.
pub const MAX_WOUNDS: usize = 16;

/// Render-owned pollable feed over wound values: the in-crate consumer.
///
/// Render decals/particles own one board per bout, push each tick's
/// [`FightEvent`]s via [`WoundBoard::open_for_events`], then poll
/// [`WoundBoard::intensity_at`] (or [`WoundBoard::live_count_at`]) each tick
/// for the tapered intensity. Data owner: `engine-combat`. Units: ticks at
/// 60 Hz in, permille out. Determinism: integer math and event order only;
/// same pushes plus same poll tick give the same intensities on every PC.
/// Presentation-only: the board never touches fight state or its hash.
/// Errors: out-of-range polls return `None`, never panic.
///
/// UNLOCKS: E42b render-decals poll `pulse_at` through this board.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WoundBoard {
    /// Live wounds in open order; oldest at index 0.
    wounds: Vec<Wound>,
}

impl WoundBoard {
    /// Empty feed with no heap yet; first pushes reserve up to [`MAX_WOUNDS`].
    pub fn new() -> Self {
        Self { wounds: Vec::new() }
    }

    /// Live wound count, for pre-sizing caller buffers.
    pub fn len(&self) -> usize {
        self.wounds.len()
    }

    /// True when no live wounds are kept.
    pub fn is_empty(&self) -> bool {
        self.wounds.is_empty()
    }

    /// Open wounds for one tick's events, dropping oldest past [`MAX_WOUNDS`].
    ///
    /// Calls [`wounds_for_events`]; output order follows input order.
    /// The temporary per-call vector is bounded by `events.len()` and freed
    /// on return; the board buffer stops reallocating once at capacity.
    pub fn open_for_events(&mut self, tick: u32, events: &[FightEvent]) {
        let fresh = wounds_for_events(tick, events);
        self.wounds.extend(fresh);
        let overflow = self.wounds.len().saturating_sub(MAX_WOUNDS);
        if overflow > 0 {
            self.wounds.drain(..overflow);
        }
    }

    /// Tapered intensity for one slot at `now_tick`; `None` when out of range.
    ///
    /// Calls [`Wound::pulse_at`]; integer math only, saturating.
    pub fn intensity_at(&self, index: usize, now_tick: u32) -> Option<u16> {
        self.wounds.get(index).map(|wound| wound.pulse_at(now_tick))
    }

    /// Live wound count at `now_tick` (`pulse_at > 0`); the decal draw count.
    ///
    /// Calls [`Wound::pulse_at`] once per kept wound; integer math only.
    pub fn live_count_at(&self, now_tick: u32) -> usize {
        self.wounds
            .iter()
            .filter(|wound| wound.pulse_at(now_tick) > 0)
            .count()
    }

    /// Drop fully tapered wounds (`pulse_at == 0`); keeps live order.
    ///
    /// Calls [`Wound::pulse_at`] once per kept wound; reuses the buffer.
    pub fn sweep_expired(&mut self, now_tick: u32) {
        self.wounds.retain(|wound| wound.pulse_at(now_tick) > 0);
    }
}

/// Pure mapping from one tick's fight events to presentation gore.
///
/// `tick` is the authority tick the caller stepped; [`FightEvent`] carries
/// no clock. Mapping: `Hit` gives spray plus stamp with severity from
/// damage (stamp at half intensity for the decal pass), `Parried` gives one
/// low-severity graze spray from the table chip row, `Knockdown` adds a
/// full-severity [`GoreKind::DismemberLite`]; `Blocked`, `Dodged` and
/// `StaminaOut` emit nothing. Output order follows input order.
pub fn gore_for_events(tick: u32, events: &[FightEvent]) -> Vec<GoreEvent> {
    gore_for_events_with_tables(tick, events, &TABLES_V1)
}

/// Pure mapping under explicit table rows: same rows plus same tick and
/// events give the same gore. The graze severity reads the table chip row,
/// so tuning damage retunes gore intensity with it.
pub fn gore_for_events_with_tables(
    tick: u32,
    events: &[FightEvent],
    tables: &Tables,
) -> Vec<GoreEvent> {
    gore_for_events_with_setting(tick, events, tables, GoreSetting::Full)
}

/// Pure mapping under an explicit presentation policy.
///
/// `Full` is today's mapping (spray plus stamp, graze spray, fatal
/// [`GoreKind::DismemberLite`]); `Reduced` keeps the blood but drops the
/// severed-part cue; `Off` drops every [`GoreKind::is_blood`] event and
/// emits [`GoreKind::ImpactFlash`] for hits and grazes plus
/// [`GoreKind::GuardCue`] for knockdowns, so the bout still reads without
/// blood. The setting never touches [`FightState`]: same tape under any
/// setting steps the same hash. Integer math and event order only.
pub fn gore_for_events_with_setting(
    tick: u32,
    events: &[FightEvent],
    tables: &Tables,
    setting: GoreSetting,
) -> Vec<GoreEvent> {
    let mut gore = Vec::with_capacity(events.len().saturating_mul(2));
    for event in events.iter() {
        match *event {
            FightEvent::Hit { victim, damage, .. } => {
                let severity = severity_for_damage(damage);
                match setting {
                    GoreSetting::Full | GoreSetting::Reduced => {
                        gore.push(GoreEvent {
                            tick,
                            victim,
                            kind: GoreKind::BloodSpray,
                            pos_cm: spray_pos_cm(tick, victim),
                            severity_permille: severity,
                        });
                        gore.push(GoreEvent {
                            tick,
                            victim,
                            kind: GoreKind::DecalStamp,
                            pos_cm: decal_pos_cm(tick, victim),
                            severity_permille: severity / 2,
                        });
                    }
                    GoreSetting::Off => {
                        gore.push(GoreEvent {
                            tick,
                            victim,
                            kind: GoreKind::ImpactFlash,
                            pos_cm: spray_pos_cm(tick, victim),
                            severity_permille: severity,
                        });
                    }
                }
            }
            FightEvent::Parried { victim, .. } => {
                // No wound: a graze mark sized by the table chip row.
                let graze = severity_for_damage(tables.chip(tables.attack_damage));
                match setting {
                    GoreSetting::Full | GoreSetting::Reduced => {
                        gore.push(GoreEvent {
                            tick,
                            victim,
                            kind: GoreKind::BloodSpray,
                            pos_cm: spray_pos_cm(tick, victim),
                            severity_permille: graze,
                        });
                    }
                    GoreSetting::Off => {
                        gore.push(GoreEvent {
                            tick,
                            victim,
                            kind: GoreKind::ImpactFlash,
                            pos_cm: spray_pos_cm(tick, victim),
                            severity_permille: graze,
                        });
                    }
                }
            }
            FightEvent::Knockdown { victim, .. } => match setting {
                GoreSetting::Full => {
                    gore.push(GoreEvent {
                        tick,
                        victim,
                        kind: GoreKind::DismemberLite,
                        pos_cm: spray_pos_cm(tick, victim),
                        severity_permille: MAX_HP_PERMILLE,
                    });
                }
                GoreSetting::Reduced => {}
                GoreSetting::Off => {
                    gore.push(GoreEvent {
                        tick,
                        victim,
                        kind: GoreKind::GuardCue,
                        pos_cm: spray_pos_cm(tick, victim),
                        severity_permille: MAX_HP_PERMILLE,
                    });
                }
            },
            FightEvent::Blocked { .. }
            | FightEvent::Dodged { .. }
            | FightEvent::StaminaOut { .. } => {}
        }
    }
    gore
}

/// Torso anchor in centimetres for a victim slot: slot spacing plus a small
/// deterministic per-tick jitter, integer math only.
pub const fn spray_pos_cm(tick: u32, victim: u8) -> [i32; 3] {
    let (jx, jz) = jitter_cm(tick, victim);
    [victim as i32 * SLOT_SPACING_CM + jx, SPRAY_HEIGHT_CM, jz]
}

/// Ground stamp in centimetres: same lateral anchor as the spray at `y == 0`.
pub const fn decal_pos_cm(tick: u32, victim: u8) -> [i32; 3] {
    let (jx, jz) = jitter_cm(tick, victim);
    [victim as i32 * SLOT_SPACING_CM + jx, 0, jz]
}

/// Deterministic jitter in `±JITTER_CM` from tick and slot (integer hash,
///
/// no floats, no RNG state), so repeated calls with the same inputs agree.
const fn jitter_cm(tick: u32, victim: u8) -> (i32, i32) {
    let hash = tick
        .wrapping_mul(0x9E37_79B1)
        .wrapping_add((victim as u32).wrapping_mul(0x85EB_CA6B));
    let span = JITTER_CM.unsigned_abs() * 2 + 1;
    let jx = saturating_u32_to_i32(hash % span) - JITTER_CM;
    let jz = saturating_u32_to_i32((hash >> 16) % span) - JITTER_CM;
    (jx, jz)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fight::{step, FightState, ATTACK_DAMAGE};
    use engine_input::fighter::ATTACK;
    use engine_input::{Buttons, FighterId, FighterInput};

    fn input(fighter: u8, buttons: u8) -> FighterInput {
        FighterInput {
            tick: 0,
            fighter: FighterId::new(fighter).unwrap(),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons(buttons),
        }
    }

    fn none(fighter: u8) -> FighterInput {
        input(fighter, 0)
    }

    /// FNV-1a hash of the authoritative state only; gore never feeds it.
    fn state_hash(state: &FightState) -> u64 {
        let mut hash: u64 = 0xcbf29ce484222325;
        let mut mix = |byte: u8| {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        };
        mix(state.fighter_count);
        for byte in state.tick.to_le_bytes() {
            mix(byte);
        }
        for byte in state.hitstop.to_le_bytes() {
            mix(byte);
        }
        for id in 0..state.fighter_count {
            let f = state.fighters[id as usize];
            for byte in f.hp_permille.to_le_bytes() {
                mix(byte);
            }
            for byte in f.stamina.to_le_bytes() {
                mix(byte);
            }
            for byte in f.attack_age.to_le_bytes() {
                mix(byte);
            }
            mix(u8::from(f.is_attacking));
            for byte in f.block_age.to_le_bytes() {
                mix(byte);
            }
            mix(u8::from(f.is_blocking));
            for byte in f.dodge_age.to_le_bytes() {
                mix(byte);
            }
            mix(u8::from(f.is_dodging));
            for byte in f.react.to_le_bytes() {
                mix(byte);
            }
            for byte in f.knockdown.to_le_bytes() {
                mix(byte);
            }
        }
        hash
    }

    /// Scripted bout covering attack, block, parry, stamina and knockdown
    /// paths, stepped like the arena scheduler feeding ticks and events out.
    fn run_bout() -> (Vec<Vec<FightEvent>>, FightState) {
        let mut state = FightState::new(2).unwrap();
        // Victim at exactly one hit of health, so the first impact is fatal:
        // Hit plus Knockdown in one bout covers spray, stamp and fatal cue.
        state.fighters[1].hp_permille = ATTACK_DAMAGE;
        let script: Vec<Vec<FighterInput>> = vec![
            vec![input(0, ATTACK), none(1)],
            vec![none(0), none(1)],
            vec![none(0), none(1)],
            vec![none(0), none(1)],
            vec![none(0), none(1)],
            vec![none(0), none(1)],
            vec![none(0), none(1)],
            vec![none(0), none(1)],
            vec![none(0), none(1)],
            vec![input(0, ATTACK), none(1)],
            vec![none(0), none(1)],
            vec![none(0), none(1)],
            vec![none(0), none(1)],
            vec![none(0), none(1)],
        ];
        let mut all = Vec::with_capacity(script.len());
        for tick_inputs in &script {
            all.push(step(tick_inputs, &mut state));
        }
        (all, state)
    }

    #[test]
    fn hit_maps_to_spray_and_stamp_with_severity_from_damage() {
        let hit = FightEvent::Hit {
            attacker: 0,
            victim: 1,
            damage: ATTACK_DAMAGE,
        };
        let gore = gore_for_events(7, &[hit]);
        assert_eq!(gore.len(), 2);
        let spray = &gore[0];
        let stamp = &gore[1];
        assert_eq!(spray.kind, GoreKind::BloodSpray);
        assert_eq!(stamp.kind, GoreKind::DecalStamp);
        assert_eq!((spray.tick, spray.victim), (7, 1));
        assert_eq!((stamp.tick, stamp.victim), (7, 1));
        // Severity comes from the table damage row.
        assert_eq!(spray.severity_permille, ATTACK_DAMAGE);
        assert_eq!(stamp.severity_permille, ATTACK_DAMAGE / 2);
        // Integer centimetres only (no NaN possible): spray at torso
        // height, stamp on the ground, sharing the lateral anchor.
        assert_eq!(spray.pos_cm[1], SPRAY_HEIGHT_CM);
        assert_eq!(stamp.pos_cm[1], 0);
        assert_eq!(spray.pos_cm[0], stamp.pos_cm[0]);
        assert_eq!(spray.pos_cm[2], stamp.pos_cm[2]);
        for event in &gore {
            assert!(event.severity_permille <= MAX_HP_PERMILLE);
        }
        // Custom table damage retunes severity with the row.
        let mut rows = TABLES_V1;
        rows.attack_damage = 240;
        let big_hit = FightEvent::Hit {
            attacker: 0,
            victim: 1,
            damage: rows.attack_damage,
        };
        let gore = gore_for_events_with_tables(7, &[big_hit], &rows);
        assert_eq!(gore[0].severity_permille, 240);
        assert_eq!(gore[1].severity_permille, 120);
    }

    #[test]
    fn blocked_dodged_and_empty_hits_emit_nothing() {
        let quiet = [
            FightEvent::Blocked {
                attacker: 0,
                victim: 1,
                chip: crate::fight::block_chip(ATTACK_DAMAGE),
            },
            FightEvent::Dodged {
                attacker: 0,
                victim: 1,
            },
            FightEvent::StaminaOut { fighter: 0 },
        ];
        assert!(gore_for_events(3, &quiet).is_empty());
        assert!(gore_for_events(3, &[]).is_empty());
        // A parry grazes (table chip row) without a wound.
        let parry = [FightEvent::Parried {
            attacker: 0,
            victim: 1,
        }];
        let gore = gore_for_events(3, &parry);
        assert_eq!(gore.len(), 1);
        assert_eq!(gore[0].kind, GoreKind::BloodSpray);
        assert_eq!(
            gore[0].severity_permille,
            TABLES_V1.chip(TABLES_V1.attack_damage)
        );
        // A knockdown adds the fatal-only cue at full severity.
        let down = [FightEvent::Knockdown {
            attacker: 0,
            victim: 1,
        }];
        let gore = gore_for_events(3, &down);
        assert_eq!(gore.len(), 1);
        assert_eq!(gore[0].kind, GoreKind::DismemberLite);
        assert_eq!(gore[0].severity_permille, MAX_HP_PERMILLE);
    }

    #[test]
    fn severity_bounded_for_extreme_damage() {
        let hit = FightEvent::Hit {
            attacker: 0,
            victim: 1,
            damage: u16::MAX,
        };
        let gore = gore_for_events(0, &[hit]);
        assert_eq!(gore[0].severity_permille, MAX_HP_PERMILLE);
        assert_eq!(gore[1].severity_permille, MAX_HP_PERMILLE / 2);
        assert_eq!(severity_for_damage(0), 0);
        assert_eq!(severity_for_damage(MAX_HP_PERMILLE), MAX_HP_PERMILLE);
    }

    #[test]
    fn same_bout_twice_gives_identical_gore_streams() {
        let collect = || {
            let (bout, _) = run_bout();
            let mut stream = Vec::new();
            for (tick, events) in bout.iter().enumerate() {
                stream.extend(gore_for_events(
                    u32::try_from(tick).unwrap_or(u32::MAX),
                    events,
                ));
            }
            stream
        };
        let first = collect();
        let second = collect();
        assert_eq!(first, second);
        // The scripted bout really bleeds: spray, stamp and fatal cue.
        assert!(first.iter().any(|e| e.kind == GoreKind::BloodSpray));
        assert!(first.iter().any(|e| e.kind == GoreKind::DecalStamp));
        assert!(first.iter().any(|e| e.kind == GoreKind::DismemberLite));
    }

    #[test]
    fn gore_stream_excluded_from_state_hash() {
        // With gore collection: hash each tick after mapping events to gore.
        let mut with_gore = FightState::new(2).unwrap();
        with_gore.fighters[1].hp_permille = ATTACK_DAMAGE;
        let script: Vec<Vec<FighterInput>> = vec![
            vec![input(0, ATTACK), none(1)],
            vec![none(0), none(1)],
            vec![none(0), none(1)],
            vec![none(0), none(1)],
            vec![none(0), none(1)],
            vec![none(0), none(1)],
            vec![none(0), none(1)],
            vec![none(0), none(1)],
            vec![none(0), none(1)],
            vec![input(0, ATTACK), none(1)],
            vec![none(0), none(1)],
            vec![none(0), none(1)],
            vec![none(0), none(1)],
            vec![none(0), none(1)],
        ];
        let mut hashes_with = Vec::new();
        let mut stream = Vec::new();
        for (tick, tick_inputs) in script.iter().enumerate() {
            let before = with_gore;
            let events = step(tick_inputs, &mut with_gore);
            // Gore reads events by shared reference and never touches state.
            let gore = gore_for_events(u32::try_from(tick).unwrap_or(u32::MAX), &events);
            assert_eq!(with_gore, {
                let mut untouched = before;
                let replayed = step(tick_inputs, &mut untouched);
                assert_eq!(replayed, events);
                untouched
            });
            stream.extend(gore);
            hashes_with.push(state_hash(&with_gore));
        }
        // Without gore collection: same script, hashes must match exactly.
        let mut without_gore = FightState::new(2).unwrap();
        without_gore.fighters[1].hp_permille = ATTACK_DAMAGE;
        let mut hashes_without = Vec::new();
        for tick_inputs in &script {
            step(tick_inputs, &mut without_gore);
            hashes_without.push(state_hash(&without_gore));
        }
        assert!(!stream.is_empty(), "script should bleed");
        assert_eq!(hashes_with, hashes_without);
        assert_eq!(with_gore, without_gore);
    }

    /// BX10: one arena tape at three settings gives one hash; Off shows
    /// impact flash plus a guard/downed cue and no severed part.
    #[test]
    fn gore_setting_never_changes_hash_off_shows_flash_and_guard() {
        use super::GoreSetting;
        fn replay(setting: GoreSetting) -> (Vec<u64>, Vec<GoreEvent>) {
            let mut state = FightState::new(2).unwrap();
            state.fighters[1].hp_permille = ATTACK_DAMAGE;
            let script: Vec<Vec<FighterInput>> = vec![
                vec![input(0, ATTACK), none(1)],
                vec![none(0), none(1)],
                vec![none(0), none(1)],
                vec![none(0), none(1)],
                vec![none(0), none(1)],
                vec![none(0), none(1)],
                vec![none(0), none(1)],
                vec![none(0), none(1)],
                vec![none(0), none(1)],
                vec![input(0, ATTACK), none(1)],
                vec![none(0), none(1)],
                vec![none(0), none(1)],
                vec![none(0), none(1)],
                vec![none(0), none(1)],
            ];
            let mut hashes = Vec::with_capacity(script.len());
            let mut stream = Vec::new();
            for (tick, tick_inputs) in script.iter().enumerate() {
                let events = step(tick_inputs, &mut state);
                stream.extend(super::gore_for_events_with_setting(
                    u32::try_from(tick).unwrap_or(u32::MAX),
                    &events,
                    &crate::fight::TABLES_V1,
                    setting,
                ));
                hashes.push(state_hash(&state));
            }
            (hashes, stream)
        }
        let (hash_off, stream_off) = replay(GoreSetting::Off);
        let (hash_reduced, stream_reduced) = replay(GoreSetting::Reduced);
        let (hash_full, stream_full) = replay(GoreSetting::Full);
        // Presentation policy never changes the authoritative hash.
        assert_eq!(hash_off, hash_reduced);
        assert_eq!(hash_off, hash_full);
        // Full keeps today's blood plus the fatal-only sever cue.
        assert!(stream_full
            .iter()
            .any(|e| e.kind == super::GoreKind::BloodSpray));
        assert!(stream_full
            .iter()
            .any(|e| e.kind == super::GoreKind::DecalStamp));
        assert!(stream_full
            .iter()
            .any(|e| e.kind == super::GoreKind::DismemberLite));
        // Reduced keeps blood but drops the severed-part cue.
        assert!(stream_reduced
            .iter()
            .any(|e| e.kind == super::GoreKind::BloodSpray));
        assert!(!stream_reduced
            .iter()
            .any(|e| e.kind == super::GoreKind::DismemberLite));
        // Off shows impact flash and a guard/downed cue, never blood.
        assert!(!stream_off.is_empty(), "Off still cues impacts");
        assert!(stream_off
            .iter()
            .any(|e| e.kind == super::GoreKind::ImpactFlash));
        assert!(stream_off
            .iter()
            .any(|e| e.kind == super::GoreKind::GuardCue));
        assert!(!stream_off
            .iter()
            .any(|e| e.kind == super::GoreKind::BloodSpray));
        assert!(!stream_off
            .iter()
            .any(|e| e.kind == super::GoreKind::DecalStamp));
        assert!(!stream_off
            .iter()
            .any(|e| e.kind == super::GoreKind::DismemberLite));
        for event in &stream_off {
            assert!(event.severity_permille <= MAX_HP_PERMILLE);
        }
    }

    /// E42 leg 1: first hashed cut-wound fixture. A cut opens one wound
    /// value at full damage severity, the integer-tick pulse tapers it to
    /// zero, and the stream hash pins the cut. Wounds never move state.
    #[test]
    fn cut_wound_fixture_hashes_stably_and_tapers_to_zero() {
        let hit = FightEvent::Hit {
            attacker: 0,
            victim: 1,
            damage: ATTACK_DAMAGE,
        };
        let wounds = super::wounds_for_events(10, &[hit]);
        assert_eq!(wounds.len(), 1);
        let wound = wounds[0];
        assert_eq!((wound.victim, wound.opened_tick), (1, 10));
        assert_eq!(wound.severity_permille, ATTACK_DAMAGE);
        assert_eq!(wound.pos_cm, super::spray_pos_cm(10, 1));
        // Pinned cut-wound stream hash (Fnv64 owner, little-endian fields).
        assert_eq!(super::wound_stream_hash(&wounds), 438314581738813528);
        // Same inputs twice give the same wounds and hash.
        let again = super::wounds_for_events(10, &[hit]);
        assert_eq!(again, wounds);
        assert_eq!(
            super::wound_stream_hash(&again),
            super::wound_stream_hash(&wounds)
        );
        // Integer-tick taper: full at opening, bounded mid-way, zero after.
        assert_eq!(wound.pulse_at(10), ATTACK_DAMAGE);
        assert_eq!(super::wound_pulse_permille(10, 9, ATTACK_DAMAGE), 0);
        let mid = wound.pulse_at(10 + super::WOUND_PULSE_TICKS);
        assert!(mid > 0 && mid <= ATTACK_DAMAGE);
        assert_eq!(wound.pulse_at(10 + super::WOUND_FADE_TICKS), 0);
        assert_eq!(wound.pulse_at(10 + super::WOUND_FADE_TICKS + 1), 0);
        // No wound without a cut: graze, block, dodge and empty emit none.
        let graze = [FightEvent::Parried {
            attacker: 0,
            victim: 1,
        }];
        assert!(super::wounds_for_events(10, &graze).is_empty());
        let quiet = [
            FightEvent::Blocked {
                attacker: 0,
                victim: 1,
                chip: crate::fight::block_chip(ATTACK_DAMAGE),
            },
            FightEvent::Dodged {
                attacker: 0,
                victim: 1,
            },
            FightEvent::StaminaOut { fighter: 0 },
        ];
        assert!(super::wounds_for_events(10, &quiet).is_empty());
        // Knockdown opens a full-severity cut at the same anchor.
        let down = [FightEvent::Knockdown {
            attacker: 0,
            victim: 1,
        }];
        let fatal = super::wounds_for_events(11, &down);
        assert_eq!(fatal.len(), 1);
        assert_eq!(fatal[0].severity_permille, MAX_HP_PERMILLE);
        assert_eq!(fatal[0].pulse_at(11), MAX_HP_PERMILLE);
        // Hash-neutral: collecting wounds leaves authority state alone.
        let mut state = FightState::new(2).unwrap();
        let inputs = [input(0, ATTACK), none(1)];
        let before = state;
        let events = step(&inputs, &mut state);
        let with_wounds = state;
        let _ = super::wounds_for_events(0, &events);
        assert_eq!(state, with_wounds);
        let mut replay = before;
        let replayed = step(&inputs, &mut replay);
        assert_eq!(replayed, events);
        assert_eq!(replay, state);
    }

    /// E42a repair: the board is the in-crate consumer. It opens cuts via
    /// `wounds_for_events`, polls `pulse_at` per slot, caps at `MAX_WOUNDS`
    /// by dropping oldest, and sweeps fully tapered wounds.
    #[test]
    fn wound_board_polls_taper_and_drops_oldest() {
        let hit = FightEvent::Hit {
            attacker: 0,
            victim: 1,
            damage: ATTACK_DAMAGE,
        };
        // Failure first: empty board polls `None` and counts zero.
        let mut board = super::WoundBoard::new();
        assert!(board.is_empty());
        assert_eq!(board.len(), 0);
        assert_eq!(board.intensity_at(0, 10), None);
        assert_eq!(board.live_count_at(10), 0);
        // Quiet events open nothing through the board either.
        let graze = [FightEvent::Parried {
            attacker: 0,
            victim: 1,
        }];
        board.open_for_events(10, &graze);
        assert!(board.is_empty());

        // One cut opens one live slot polled through `pulse_at`.
        board.open_for_events(10, &[hit]);
        assert_eq!(board.len(), 1);
        assert_eq!(board.intensity_at(0, 10), Some(ATTACK_DAMAGE));
        assert_eq!(board.live_count_at(10), 1);
        let mid = board.intensity_at(0, 10 + super::WOUND_PULSE_TICKS);
        assert!(mid.is_some() && mid.unwrap() > 0 && mid.unwrap() <= ATTACK_DAMAGE);
        // Fully tapered at the horizon, then swept away.
        assert_eq!(board.intensity_at(0, 10 + super::WOUND_FADE_TICKS), Some(0));
        assert_eq!(board.live_count_at(10 + super::WOUND_FADE_TICKS), 0);
        board.sweep_expired(10 + super::WOUND_FADE_TICKS);
        assert!(board.is_empty());
        // Failure: polling past the end returns `None`, never panics.
        board.open_for_events(20, &[hit]);
        assert_eq!(board.intensity_at(1, 20), None);

        // Cap: oldest drops first, newest survives.
        let mut full = super::WoundBoard::new();
        for tick in 0..(u32::try_from(super::MAX_WOUNDS).unwrap_or(u32::MAX) + 1) {
            full.open_for_events(tick, &[hit]);
        }
        assert_eq!(full.len(), super::MAX_WOUNDS);
        // Tick 0 fell off; tick 1 is now oldest with a live taper.
        assert!(full.intensity_at(0, 1).unwrap() > 0);
        // All sixteen survive at the newest tick (ages 15..0, none tapered).
        assert_eq!(
            full.live_count_at(u32::try_from(super::MAX_WOUNDS).unwrap_or(u32::MAX)),
            super::MAX_WOUNDS
        );
    }
}
