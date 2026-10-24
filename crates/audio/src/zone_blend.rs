//! SW114: walking out of the tunnel into the street eases the tail over.
//!
//! One zone owning the acoustics snaps the room: crossing the mouth swaps
//! RT60 1100 ms for 600 ms in a single block, so the old tail is dropped and
//! the new one starts mid-decay. This module keeps at most two zones live at
//! once. [`weights`] turns the listener's distance inside the mouth into two
//! Q16 powers that sum to unity over a 200 cm feather; a listener 50 cm inside
//! the edge reads 25/75 tunnel/street. [`Blend::gains`] takes the square root,
//! so the two tails add as equal power and the total wet holds while the room
//! changes. [`Walker`] keeps the switch decision apart from the weights and
//! only flips the dominant zone past a 60 cm hysteresis band, so a listener
//! jittering on the mouth does not rebuild the tail eight times.
//!
//! Units: centimetres inside the mouth (positive is deeper in the tunnel),
//! Q16 powers (unity is [`WEIGHT_ONE`]), linear gains. Determinism: same
//! position gives the same weights twice; no clock, no allocation.

use crate::reverb::{ReverbError, ReverbParams, MIN_RT60_MS, MIN_SAMPLE_RATE_HZ};

/// Zones a blend may hold at once.
pub const ZONE_PAIR: usize = 2;
/// Tunnel slot: the near room the listener starts in.
pub const TUNNEL: usize = 0;
/// Street slot: the open canyon past the mouth.
pub const STREET: usize = 1;
/// Unity power in Q16.
pub const WEIGHT_ONE: i32 = 65_536;
/// Feather length, centimetres: the band the two tails share.
pub const FEATHER_CM: i32 = 200;
/// Hysteresis band, centimetres: how far past the mid-point a walker must go
/// before it swaps the dominant zone.
pub const HYSTERESIS_CM: i32 = 60;

/// Two live zones and the power each one gets. [`weights`] always sums the
/// pair to unity, so no position can silence the room.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Blend {
    /// Zone ids: tunnel first, then street.
    pub zones: [u8; ZONE_PAIR],
    /// Power per zone in Q16, summing to [`WEIGHT_ONE`].
    pub weight_q16: [i32; ZONE_PAIR],
}

impl Blend {
    /// Equal-power gains: the square root of each power, so the two tails sum
    /// as energy and a half-and-half room is not 3 dB down. Renormalized when
    /// the caller built a blend by hand.
    pub fn gains(&self) -> [f32; ZONE_PAIR] {
        let total = i64::from(self.weight_q16[0].max(0)) + i64::from(self.weight_q16[1].max(0));
        let mut out = [0.0f32; ZONE_PAIR];
        if total == 0 {
            return out;
        }
        for (slot, weight) in out.iter_mut().zip(self.weight_q16.iter()) {
            let share = i64::from((*weight).max(0)) as f64 / total as f64;
            *slot = share.sqrt() as f32;
        }
        out
    }

    /// Power the room keeps as a whole; [`WEIGHT_ONE`] for a walk-built blend.
    pub fn total_weight_q16(&self) -> i32 {
        self.weight_q16[0].saturating_add(self.weight_q16[1])
    }

    /// Street's share of the power in `0.0..=1.0`: the crossfade parameter.
    pub fn street_share(&self) -> f32 {
        let total = self.total_weight_q16();
        if total <= 0 {
            return 0.0;
        }
        self.weight_q16[STREET].max(0) as f32 / total as f32
    }

    /// Slots whose power is above zero, in blend order.
    pub fn live(&self) -> [bool; ZONE_PAIR] {
        [self.weight_q16[0] > 0, self.weight_q16[1] > 0]
    }
}

/// Q16 power of one centimetre of the feather: `WEIGHT_ONE / FEATHER_CM`.
///
/// Weights are computed as `inside * WEIGHT_ONE / FEATHER_CM`, so the ends are
/// exact unity and 50 cm inside lands on exactly 25/75.
pub fn weights(inside_cm: i32) -> [i32; ZONE_PAIR] {
    let inside = inside_cm.clamp(0, FEATHER_CM);
    let tunnel = inside * WEIGHT_ONE / FEATHER_CM;
    [tunnel, WEIGHT_ONE - tunnel]
}

/// The blend at `inside_cm`, with the zone ids filled in.
pub fn blend(inside_cm: i32) -> Blend {
    let zone_ids = [TUNNEL as u8, STREET as u8];
    Blend {
        zones: zone_ids,
        weight_q16: weights(inside_cm),
    }
}

/// Which zone owns the tail rebuild, with a hysteresis band so a walk that
/// jitters on the mouth does not flip the room back and forth.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Walker {
    dominant: u8,
    switches: u32,
}

impl Walker {
    /// A walker starting in the tunnel.
    pub fn new() -> Self {
        Self {
            dominant: TUNNEL as u8,
            switches: 0,
        }
    }

    /// The zone whose params the tail is built from.
    pub fn dominant(&self) -> u8 {
        self.dominant
    }

    /// How many times the dominant zone changed; the jitter test counts these.
    pub fn switches(&self) -> u32 {
        self.switches
    }

    /// Walk to `inside_cm` and answer the blend there.
    ///
    /// The weights follow the position every call — that is the feather — and
    /// only the dominant zone waits: it flips to the street once the listener
    /// is `HYSTERESIS_CM / 2` past the mid-point toward the mouth, and back to
    /// the tunnel once that far past it the other way.
    pub fn walk(&mut self, inside_cm: i32) -> Blend {
        let mid = FEATHER_CM / 2;
        let band = HYSTERESIS_CM / 2;
        let toward_street = self.dominant == TUNNEL as u8 && inside_cm < mid - band;
        let toward_tunnel = self.dominant == STREET as u8 && inside_cm > mid + band;
        if toward_street {
            self.dominant = STREET as u8;
            self.switches += 1;
        } else if toward_tunnel {
            self.dominant = TUNNEL as u8;
            self.switches += 1;
        }
        blend(inside_cm)
    }
}

impl Default for Walker {
    fn default() -> Self {
        Self::new()
    }
}

/// Equal-power crossfade over one `RT60 / 4` into the new weights.
///
/// The fade runs along the equal-power arc — `t` is street's share of the
/// power, the gains are `sqrt(1 - t)` and `sqrt(t)` — so the pair squares to
/// one at every step and the total wet holds mid-fade instead of dipping 3 dB.
/// A 1100 ms room eases its tail over 275 ms and never snaps.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Ramp {
    t: f32,
    target_t: f32,
    step: f32,
}

impl Ramp {
    /// A ramp whose full crossfade takes `rt60_ms / 4` of `sample_rate_hz`.
    pub fn new(rt60_ms: f32, sample_rate_hz: u32) -> Result<Self, ReverbError> {
        if !rt60_ms.is_finite() || rt60_ms < MIN_RT60_MS {
            return Err(ReverbError::BadParam { name: "rt60_ms" });
        }
        if sample_rate_hz < MIN_SAMPLE_RATE_HZ {
            return Err(ReverbError::BadSampleRate {
                got: sample_rate_hz,
            });
        }
        let frames = rt60_ms / 4.0 * sample_rate_hz as f32 / 1000.0;
        Ok(Self {
            t: 0.0,
            target_t: 0.0,
            step: if frames > 1.0 { 1.0 / frames } else { 1.0 },
        })
    }

    /// Aim the ramp at `blend`'s street power share.
    pub fn aim(&mut self, blend: &Blend) {
        self.target_t = blend.street_share().clamp(0.0, 1.0);
    }

    /// Current equal-power gains: tunnel, then street.
    pub fn gains(&self) -> [f32; ZONE_PAIR] {
        let t = self.t.clamp(0.0, 1.0);
        [(1.0 - t).sqrt(), t.sqrt()]
    }

    /// How far along the fade the ramp is, in street power share.
    pub fn crossfade(&self) -> f32 {
        self.t
    }

    /// Move `frames` samples toward the target; returns the gains now in use.
    pub fn advance(&mut self, frames: usize) -> [f32; ZONE_PAIR] {
        let travel = self.step * frames as f32;
        let delta = self.target_t - self.t;
        if delta.abs() <= travel {
            self.t = self.target_t;
        } else if delta > 0.0 {
            self.t += travel;
        } else {
            self.t -= travel;
        }
        self.gains()
    }

    /// True once the fade sits on its target.
    pub fn settled(&self) -> bool {
        (self.t - self.target_t).abs() <= 1e-6
    }
}

#[cfg(test)]
#[path = "zone_blend_tests.rs"]
mod tests;
