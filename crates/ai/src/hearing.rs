//! hearing: pure integer hearing table for AI listeners.
//!
//! Consumer: next slice feeds live positions as distances into [`heard_by`].
//! Data owner: caller owns noises and listener slots; this module keeps no globals.
//! Units: centimetres for distance, ticks for time, 0..=1000 for strength.
//! Determinism: integer math only, no float, no clock; same inputs give
//! the same output twice, so replays stay exact.
//! Errors: none; [`heard_by`] returns `None` when unheard.

/// Kind of noise a listener can hear.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SoundKind {
    /// Sharp weapon report, carries far.
    Gunfire,
    /// Stunning blast, carries far.
    Flashbang,
    /// Heavy explosion, carries far.
    Blast,
    /// Hinged panel slam, carries mid range.
    Door,
    /// Boot fall, carries mid range.
    Footstep,
    /// Faint rustle, carries only nearby.
    Quiet,
}

/// One noise event stamped with its start tick.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Noise {
    /// Which table row governs range and life.
    pub kind: SoundKind,
    /// First tick the noise is audible.
    pub start_tick: u64,
}

/// Audible range for a kind.
///
/// Units: centimetres. Determinism: table lookup only. Errors: none.
pub fn range_cm(kind: SoundKind) -> u32 {
    match kind {
        SoundKind::Gunfire => 4000,
        SoundKind::Flashbang => 4000,
        SoundKind::Blast => 4000,
        SoundKind::Door => 2000,
        SoundKind::Footstep => 2400,
        SoundKind::Quiet => 1000,
    }
}

/// Audible life for a kind.
///
/// Units: ticks from `start_tick`. Determinism: table lookup only. Errors: none.
pub fn life_ticks(kind: SoundKind) -> u64 {
    match kind {
        SoundKind::Gunfire => 120,
        SoundKind::Flashbang => 120,
        SoundKind::Blast => 180,
        SoundKind::Door => 210,
        SoundKind::Footstep => 90,
        SoundKind::Quiet => 30,
    }
}

/// Strength of one noise at one listener distance and tick.
///
/// Units: `dist_cm` is centimetres, `tick` is ticks, output is 0..=1000.
/// Determinism: integer math only, no float, no clock; distance is tested
/// first, then full strength for the first 40pct of life, then a straight
/// fade to zero; same inputs give the same output twice.
/// Errors: none; returns `None` when `tick` is before `start_tick`, when age
/// reaches life, or when `dist_cm` exceeds range.
pub fn heard_by(dist_cm: u32, noise: Noise, tick: u64) -> Option<u16> {
    if tick < noise.start_tick {
        return None;
    }
    let age = tick - noise.start_tick;
    let life = life_ticks(noise.kind);
    if age >= life {
        return None;
    }
    if dist_cm > range_cm(noise.kind) {
        return None;
    }
    if age * 5 < life * 2 {
        return Some(1000);
    }
    let full = life * 2 / 5;
    let denom = life - full;
    if denom == 0 {
        return Some(1000);
    }
    let remain = life - age;
    let scaled = 1000 * remain / denom;
    u16::try_from(scaled).ok()
}

/// Louder of two listener-slot strengths.
///
/// Units: 0..=1000 strengths. Determinism: one integer compare; same inputs
/// give the same output twice. Errors: none; `None` when both are `None`.
pub fn pick_louder(a: Option<u16>, b: Option<u16>) -> Option<u16> {
    match (a, b) {
        (None, None) => None,
        (Some(x), None) => Some(x),
        (None, Some(y)) => Some(y),
        (Some(x), Some(y)) => Some(x.max(y)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hearing_gunshot_heard_then_expires() {
        let noise = Noise {
            kind: SoundKind::Gunfire,
            start_tick: 10,
        };
        assert_eq!(heard_by(3900, noise, 10), Some(1000));
        assert_eq!(heard_by(3900, noise, 140), None);
    }

    #[test]
    fn hearing_footstep_out_of_range_never() {
        let noise = Noise {
            kind: SoundKind::Footstep,
            start_tick: 5,
        };
        assert_eq!(heard_by(2500, noise, 5), None);
        assert_eq!(heard_by(2500, noise, 50), None);
    }

    #[test]
    fn hearing_fade_linear_after_40pct() {
        let noise = Noise {
            kind: SoundKind::Gunfire,
            start_tick: 0,
        };
        assert_eq!(heard_by(0, noise, 48), Some(1000));
        assert_eq!(heard_by(0, noise, 84), Some(500));
        assert_eq!(heard_by(0, noise, 120), None);
    }

    #[test]
    fn hearing_louder_replaces_and_stable_x2() {
        let loud = Noise {
            kind: SoundKind::Gunfire,
            start_tick: 100,
        };
        let soft = Noise {
            kind: SoundKind::Quiet,
            start_tick: 100,
        };
        let a = heard_by(500, loud, 120);
        let b = heard_by(500, soft, 120);
        assert_eq!(a, Some(1000));
        assert_eq!(b, Some(555));
        assert_eq!(pick_louder(a, b), a);
        assert_eq!(pick_louder(None, None), None);
        assert_eq!(heard_by(500, loud, 120), a);
    }
}
