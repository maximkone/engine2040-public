//! channels: per-object coverage for the gameplay layers (wet, dirt, blood,
//! damage, burn).
//!
//! Presentation only (HASH-LAW): the sim publishes events, this module turns
//! them into five `u8` coverage channels per object, and the shader reads the
//! channels through [`pack_channels`]. Nothing here is ever read back into the
//! tick. The channels are a pure function of the event tape up to a tick
//! ([`Coverage::replay`]), so a rollback and resim leaves no stale wetness:
//! the host recomputes from the tape instead of patching state. Integer math
//! only; no clock, no float in the state.
//!
//! [`twin_layers`] is the CPU twin of the generated layer code: the same
//! formulas on the same packed values, so tests can pin what a wet or bloody
//! surface looks like without a GPU.

use crate::compile::pack_uniform;
use crate::record::{Channel, LayerKind, MaterialRecord};

/// Ticks between one unit of wetness drying off.
pub const WET_DRY_PERIOD_TICKS: u32 = 4;
/// Ticks between one unit of heat cooling.
pub const HEAT_COOL_PERIOD_TICKS: u32 = 2;

/// Coverage of one object, each channel `0..=255` for `0..=1`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Coverage {
    /// Water coverage; dries over time.
    pub wetness: u8,
    /// Dust and grime; permanent.
    pub dirt: u8,
    /// Blood; permanent.
    pub blood: u8,
    /// Structural strain, as last published by the bonded solver.
    pub strain: u8,
    /// Heat; cools over time.
    pub heat: u8,
}

/// What the sim reported, in presentation terms.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CoverageEvent {
    /// Water hit the object (rain, a splash): adds `amount`.
    Water(u8),
    /// A hit drew blood: adds `amount`.
    Blood(u8),
    /// Dust settled: adds `amount`.
    Dust(u8),
    /// The solver's strain reading now: replaces the channel.
    Strain(u8),
    /// A fire or spark added heat: adds `amount`.
    Heat(u8),
}

/// One event at a tick.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TimedEvent {
    /// Sim tick the event happened on.
    pub tick: u32,
    /// What happened.
    pub event: CoverageEvent,
}

impl Coverage {
    /// Fold one event in (saturating).
    pub fn apply(&mut self, event: CoverageEvent) {
        match event {
            CoverageEvent::Water(a) => self.wetness = self.wetness.saturating_add(a),
            CoverageEvent::Blood(a) => self.blood = self.blood.saturating_add(a),
            CoverageEvent::Dust(a) => self.dirt = self.dirt.saturating_add(a),
            CoverageEvent::Strain(v) => self.strain = v,
            CoverageEvent::Heat(a) => self.heat = self.heat.saturating_add(a),
        }
    }

    /// Advance `ticks` ticks of drying and cooling, counted from tick
    /// `from` (so the result depends on absolute ticks, never on how the
    /// host chunked its frames).
    pub fn age(&mut self, from: u32, ticks: u32) {
        let drops = |from: u32, ticks: u32, period: u32| -> u32 {
            // Multiples of `period` in `(from, from + ticks]`.
            (from.saturating_add(ticks)) / period - from / period
        };
        let wet = drops(from, ticks, WET_DRY_PERIOD_TICKS).min(255);
        let heat = drops(from, ticks, HEAT_COOL_PERIOD_TICKS).min(255);
        self.wetness = self
            .wetness
            .saturating_sub(u8::try_from(wet).unwrap_or(255));
        self.heat = self.heat.saturating_sub(u8::try_from(heat).unwrap_or(255));
    }

    /// The coverage at `tick`, from the tape alone: events with a tick at or
    /// before it, in order (ties keep tape order), with drying between them.
    /// This is the rollback-safe path: call it again after a resim.
    #[must_use]
    pub fn replay(events: &[TimedEvent], tick: u32) -> Self {
        let mut state = Self::default();
        let mut at = 0u32;
        for timed in events {
            if timed.tick > tick {
                break;
            }
            state.age(at, timed.tick.saturating_sub(at));
            at = at.max(timed.tick);
            state.apply(timed.event);
        }
        state.age(at, tick.saturating_sub(at));
        state
    }

    /// The channel a layer driver reads, as `0..=1`.
    #[must_use]
    pub fn value(&self, channel: Channel) -> f32 {
        let raw = match channel {
            Channel::Wetness => self.wetness,
            Channel::Dirt => self.dirt,
            Channel::Blood => self.blood,
            Channel::Strain => self.strain,
            Channel::Heat => self.heat,
        };
        f32::from(raw) / 255.0
    }
}

/// The `ChannelU` floats the shader reads: wetness, dirt, blood, strain,
/// then heat and three pad zeros.
#[must_use]
pub fn pack_channels(coverage: &Coverage) -> [f32; crate::compile::CHANNEL_FLOATS] {
    [
        coverage.value(Channel::Wetness),
        coverage.value(Channel::Dirt),
        coverage.value(Channel::Blood),
        coverage.value(Channel::Strain),
        coverage.value(Channel::Heat),
        0.0,
        0.0,
        0.0,
    ]
}

/// What the layers did to one surface point: the CPU twin of the generated
/// layer code, on the same packed numbers.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LayerResult {
    /// Base color after the layers, linear.
    pub base_color: [f32; 3],
    /// Perceptual roughness after the layers.
    pub roughness: f32,
    /// Coat weight after the layers (a wet or bloody surface gains one).
    pub coat_weight: f32,
    /// Coat perceptual roughness.
    pub coat_roughness: f32,
    /// Emission added (burn embers), linear.
    pub emissive: [f32; 3],
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Run the record's layers over its base look at `coverage`, in order.
#[must_use]
pub fn twin_layers(record: &MaterialRecord, coverage: &Coverage) -> LayerResult {
    let packed = pack_uniform(record, 1.0);
    let head = crate::compile::UNIFORM_HEAD_FLOATS;
    let mut out = LayerResult {
        base_color: [packed[0], packed[1], packed[2]],
        roughness: packed[4],
        coat_weight: packed[8],
        coat_roughness: packed[9],
        emissive: [0.0; 3],
    };
    if let Some(e) = record.look.emission {
        let lum = e.luminance * crate::compile::NITS_TO_SCENE;
        out.emissive = [e.color[0] * lum, e.color[1] * lum, e.color[2] * lum];
    }
    for (i, layer) in record
        .layers
        .iter()
        .enumerate()
        .take(crate::compile::MAX_LAYERS)
    {
        let a = &packed[head + 8 * i..head + 8 * i + 4];
        let b = &packed[head + 8 * i + 4..head + 8 * i + 8];
        let cover = coverage.value(layer.driver).clamp(0.0, 1.0) * a[0];
        for (channel, tint) in out.base_color.iter_mut().zip(&b[..3]) {
            *channel *= 1.0 - a[1] * cover;
            *channel = mix(*channel, *tint, b[3] * cover);
        }
        out.roughness = mix(out.roughness, a[2], a[3] * cover);
        match layer.kind {
            LayerKind::Wet => {
                out.coat_weight = out.coat_weight.max(cover);
                out.coat_roughness = mix(out.coat_roughness, 0.05, cover);
            }
            LayerKind::Blood => {
                out.coat_weight = out.coat_weight.max(cover * 0.6);
                out.coat_roughness = mix(out.coat_roughness, 0.15, cover);
            }
            LayerKind::Burn => {
                for (e, t) in out.emissive.iter_mut().zip(&b[..3]) {
                    *e += t * cover * cover;
                }
            }
            LayerKind::Dirt => out.roughness = mix(out.roughness, 1.0, cover * 0.5),
            LayerKind::Damage => out.roughness = mix(out.roughness, 1.0, cover * 0.4),
        }
    }
    out
}

/// Relative luminance of a linear color.
#[must_use]
pub fn luminance(rgb: [f32; 3]) -> f32 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::set::MaterialSet;

    fn tape() -> Vec<TimedEvent> {
        vec![
            TimedEvent {
                tick: 10,
                event: CoverageEvent::Blood(60),
            },
            TimedEvent {
                tick: 12,
                event: CoverageEvent::Blood(60),
            },
            TimedEvent {
                tick: 20,
                event: CoverageEvent::Water(200),
            },
            TimedEvent {
                tick: 30,
                event: CoverageEvent::Blood(60),
            },
        ]
    }

    #[test]
    fn the_same_tape_gives_the_same_channels_twice() {
        let a = Coverage::replay(&tape(), 40);
        let b = Coverage::replay(&tape(), 40);
        assert_eq!(a, b);
        assert_eq!(a.blood, 180);
        // 200 added at tick 20, dried one unit per 4 ticks until 40.
        assert_eq!(a.wetness, 195);
    }

    #[test]
    fn replay_after_a_rollback_leaves_no_stale_wetness() {
        let full = tape();
        // The sim rolled back to tick 25 and the water event never happened
        // on the new timeline: the tape holds only the first two events.
        let resim = &full[..2];
        let after = Coverage::replay(resim, 40);
        assert_eq!(after.wetness, 0);
        assert_eq!(after.blood, 120);
    }

    #[test]
    fn age_depends_on_absolute_ticks_not_on_frame_chunking() {
        let mut a = Coverage {
            wetness: 100,
            ..Coverage::default()
        };
        let mut b = a;
        a.age(0, 40);
        for t in 0..40 {
            b.age(t, 1);
        }
        assert_eq!(a, b);
        assert_eq!(a.wetness, 90);
    }

    #[test]
    fn saturates_and_strain_replaces() {
        let mut c = Coverage::default();
        c.apply(CoverageEvent::Blood(200));
        c.apply(CoverageEvent::Blood(200));
        c.apply(CoverageEvent::Strain(90));
        c.apply(CoverageEvent::Strain(30));
        assert_eq!((c.blood, c.strain), (255, 30));
    }

    #[test]
    fn wet_marble_is_darker_and_smoother_in_the_twin() {
        let set = MaterialSet::siege_kit();
        let marble = set.get("marble").expect("shipped");
        let dry = twin_layers(marble, &Coverage::default());
        let wet = twin_layers(
            marble,
            &Coverage {
                wetness: 255,
                ..Coverage::default()
            },
        );
        let drop = 1.0 - luminance(wet.base_color) / luminance(dry.base_color);
        assert!(drop >= 0.08, "luminance drop {drop}");
        assert!(wet.roughness <= 0.35, "roughness {}", wet.roughness);
        assert!(wet.coat_weight > dry.coat_weight);
        assert!((dry.roughness - 0.35).abs() < 1e-6);
    }

    #[test]
    fn blood_tints_and_burn_glows() {
        let set = MaterialSet::siege_kit();
        let stone = set.get("stone").expect("shipped");
        let bloody = twin_layers(
            stone,
            &Coverage {
                blood: 255,
                ..Coverage::default()
            },
        );
        assert!(bloody.base_color[0] > bloody.base_color[1] * 4.0);
        let burnt = twin_layers(
            stone,
            &Coverage {
                heat: 255,
                ..Coverage::default()
            },
        );
        assert!(burnt.emissive[0] > 0.9);
    }

    #[test]
    fn packed_channels_are_five_values_and_pads() {
        let p = pack_channels(&Coverage {
            wetness: 255,
            heat: 51,
            ..Coverage::default()
        });
        assert_eq!(p[0], 1.0);
        assert!((p[4] - 0.2).abs() < 1e-6);
        assert_eq!(&p[5..], &[0.0, 0.0, 0.0]);
    }
}
