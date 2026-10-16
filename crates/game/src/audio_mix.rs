//! audio_mix: window mix wiring over the frozen audio brains.
//!
//! Consumer: [`windowed`](crate::windowed) plays each tick's voices through
//! this bridge before the headless mixer. Data owner: `engine-audio` owns the
//! reverb tails and the muffle query; this module only holds the tunnel zone,
//! one [`VoiceOcclusion`](engine_audio::occlude::VoiceOcclusion) slew per
//! voice and the per-tick borrow of the wall slice. Units: integer cm for
//! positions, Q16 gains inside, `0.0..=1.0` floats at the mixer call.
//! Determinism: same seeds plus same walls give the same gain curve twice
//! (presentation only, never hashed; golden `d57b5c8ae3096614` stands).
//! Errors: [`OccludeError`](engine_audio::occlude::OccludeError) on the
//! per-tick path (state untouched on refusal),
//! [`ReverbError`](engine_audio::reverb::ReverbError) at construction.
//! Budget: the occlusion brain's `0.1 ms` per voice per tick plus O(1) bridge math here,
//! no heap per tick (borrows plus fixed stack buffers only).

use engine_audio::occlude::{
    apply_floor, q16_to_gain, target_gain_q16, OccludeError, VoiceOcclusion, GAIN_ONE,
    MAX_WALLS_PER_QUERY, VOICE_FLOOR_Q16,
};
use engine_audio::reverb::{ReverbError, ReverbParams, ReverbZone, TUNNEL_WET};

pub use engine_audio::occlude::{Material, Wall};
pub use engine_audio::reverb::{TUNNEL_DAMPING, TUNNEL_PRE_DELAY_MS, TUNNEL_RT60_MS};

/// Tunnel zone center in integer cm (arena center, floor plane y = 0).
pub const TUNNEL_CENTER_CM: [i32; 3] = [0, 0, 0];
/// Tunnel send radius in cm: the 20 m arena sits inside, so the window mix
/// hears tails everywhere until zones land per-room data.
pub const TUNNEL_SEND_RADIUS_CM: i32 = 2000;
/// Most voices mixed per tick (matches the audio 32-voice stealing cap the
/// plan cites; head of the burst wins, same rule as the director bridge).
pub const MAX_MIX_VOICES: usize = 32;

/// One voice to place in the mix: where it rings, which bus it rides and
/// its pre-mix level. Positions are integer-cm floor plane `[x east, z south]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoiceSeed {
    /// Voice position in integer cm.
    pub pos_cm: [i32; 2],
    /// True for the voice bus (taunts/announcer): the `-12 dB`
    /// intelligibility floor holds instead of full muffling.
    pub voice_bus: bool,
    /// Pre-mix level `0.0..=1.0` (sanitized at the boundary).
    pub base_gain: f32,
}

/// Mix answer for one voice: what the mixer plays.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MixVoice {
    /// Occluded level `0.0..=1.0` (base gain times smoothed wall gain,
    /// floored for the voice bus).
    pub gain: f32,
    /// Reverb send `0.0..=1.0` ([`TUNNEL_WET`] inside the tunnel, else dry).
    pub send: f32,
}

/// Game-thread mix state: the tunnel zone plus one muffle slew per voice.
/// The DSP tails live on the audio thread; this bridge only feeds sends and
/// gains through the command queue, so a device reopen never resets it.
pub struct MixBridge {
    tunnel: ReverbZone,
    sample_rate_hz: u32,
    states: [VoiceOcclusion; MAX_MIX_VOICES],
}

impl MixBridge {
    /// Open the bridge at `sample_rate_hz` with the tunnel preset on. The
    /// rate validates through the frozen tunnel params, so a bad rate
    /// refuses here instead of hissing later.
    pub fn new(sample_rate_hz: u32) -> Result<Self, ReverbError> {
        ReverbParams::tunnel(sample_rate_hz)?;
        Ok(Self {
            tunnel: ReverbZone::tunnel(TUNNEL_CENTER_CM),
            sample_rate_hz,
            states: [VoiceOcclusion::new_open(); MAX_MIX_VOICES],
        })
    }

    /// Mix rate in Hz (what [`Self::new`] validated).
    #[must_use]
    pub fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz
    }

    /// Full DSP params for the tunnel at the bridge rate (what the audio
    /// thread renders; re-fetched after a reopen to prove tails stay glued).
    pub fn tunnel_params(&self) -> Result<ReverbParams, ReverbError> {
        self.tunnel.params(self.sample_rate_hz)
    }

    /// Reverb send for `listener_cm` (integer cm `[x east, y up, z south]`):
    /// [`TUNNEL_WET`] inside [`TUNNEL_SEND_RADIUS_CM`] on the floor plane,
    /// dry outside. Integer math only, never panics.
    #[must_use]
    pub fn send_for(&self, listener_cm: [i32; 3]) -> f32 {
        let dx = i64::from(listener_cm[0]) - i64::from(TUNNEL_CENTER_CM[0]);
        let dz = i64::from(listener_cm[2]) - i64::from(TUNNEL_CENTER_CM[2]);
        if dx * dx + dz * dz <= i64::from(TUNNEL_SEND_RADIUS_CM) * i64::from(TUNNEL_SEND_RADIUS_CM)
        {
            TUNNEL_WET
        } else {
            0.0
        }
    }

    /// Smoothed Q16 gain of voice slot `slot` (`None` past the cap).
    #[must_use]
    pub fn state_q16(&self, slot: usize) -> Option<i32> {
        self.states.get(slot).map(VoiceOcclusion::current_q16)
    }

    /// Back to fully open without touching the heap (seek/zone teleport).
    pub fn reset_open(&mut self) {
        for state in self.states.iter_mut() {
            state.reset_open();
        }
    }

    /// Mix one tick on the game thread: query the walls per voice, slew each
    /// slot toward its target (120 ms attack, 300 ms release inside), floor
    /// the voice bus at `-12 dB` and stamp the tunnel send. Borrows only;
    /// no heap. Returns voices mixed (head of the burst wins past the cap).
    ///
    /// Errors: [`OccludeError::TooManyWalls`] before any slew moves, so a
    /// refused tick leaves every slot exactly where it was.
    pub fn update(
        &mut self,
        seeds: &[VoiceSeed],
        listener_cm: [i32; 2],
        listener_3cm: [i32; 3],
        walls: &[Wall],
        out: &mut [MixVoice],
    ) -> Result<usize, OccludeError> {
        if walls.len() > MAX_WALLS_PER_QUERY {
            return Err(OccludeError::TooManyWalls { got: walls.len() });
        }
        let send = self.send_for(listener_3cm);
        let n = seeds
            .len()
            .min(out.len())
            .min(MAX_MIX_VOICES)
            .min(self.states.len());
        for index in 0..n {
            let seed = &seeds[index];
            let target = target_gain_q16(seed.pos_cm, listener_cm, walls)?;
            let smoothed = self.states[index].tick(target);
            let floored = if seed.voice_bus {
                apply_floor(smoothed, VOICE_FLOOR_Q16)
            } else {
                smoothed.clamp(0, GAIN_ONE)
            };
            let base = engine_audio::sanitize_gain(seed.base_gain);
            out[index] = MixVoice {
                gain: base * q16_to_gain(floored),
                send,
            };
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_audio::occlude::{apply_wall, ATTACK_STEP};

    const RATE: u32 = 48_000;
    const LISTENER: [i32; 2] = [0, 0];
    const LISTENER_3: [i32; 3] = [0, 0, 0];

    fn sfx_at(x: i32) -> VoiceSeed {
        VoiceSeed {
            pos_cm: [x, 0],
            voice_bus: false,
            base_gain: 1.0,
        }
    }

    fn voice_at(x: i32) -> VoiceSeed {
        VoiceSeed {
            pos_cm: [x, 0],
            voice_bus: true,
            base_gain: 1.0,
        }
    }

    fn stone_cross_wall() -> Wall {
        // Voice at (500, 0), listener at origin: this stone wall at x = 250
        // spans z -500..500, so the straight path crosses it dead center.
        Wall::new(250, -500, 250, 500, Material::Stone)
    }

    fn cloth_cross_wall() -> Wall {
        Wall::new(250, -500, 250, 500, Material::Cloth)
    }

    fn mix_one(bridge: &mut MixBridge, seed: &VoiceSeed, walls: &[Wall]) -> MixVoice {
        let mut out = [MixVoice {
            gain: -1.0,
            send: -1.0,
        }; 1];
        let n = bridge
            .update(
                std::slice::from_ref(seed),
                LISTENER,
                LISTENER_3,
                walls,
                &mut out,
            )
            .expect("one wall queries");
        assert_eq!(n, 1);
        out[0]
    }

    #[test]
    fn tunnel_preset_on_with_send_inside() {
        let bridge = MixBridge::new(RATE).expect("48 kHz validates");
        assert_eq!(bridge.sample_rate_hz(), RATE);
        let params = bridge.tunnel_params().expect("zone params build");
        assert_eq!(params.rt60_ms, TUNNEL_RT60_MS);
        assert_eq!(params.pre_delay_ms, TUNNEL_PRE_DELAY_MS);
        assert_eq!(params.damping, TUNNEL_DAMPING);
        assert_eq!(bridge.send_for(LISTENER_3), TUNNEL_WET);
        // Ringside seat (the director's 250 cm) still hears tails.
        assert_eq!(bridge.send_for([250, 0, 0]), TUNNEL_WET);
        // Open air with no walls passes unity with the tunnel send on.
        let mixed = mix_one(&mut MixBridge::new(RATE).unwrap(), &sfx_at(500), &[]);
        assert_eq!(mixed.gain, 1.0);
        assert_eq!(mixed.send, TUNNEL_WET);
    }

    #[test]
    fn send_silent_outside_the_tunnel() {
        let mut bridge = MixBridge::new(RATE).unwrap();
        // One radius step past the wall: dry, while the room keeps tails.
        let far: [i32; 3] = [TUNNEL_SEND_RADIUS_CM + 1, 0, 0];
        assert_eq!(bridge.send_for(far), 0.0);
        assert_eq!(bridge.send_for(LISTENER_3), TUNNEL_WET);
        let mut out = [MixVoice {
            gain: -1.0,
            send: -1.0,
        }; 1];
        let seeds = [sfx_at(500)];
        let n = bridge
            .update(&seeds, LISTENER, far, &[], &mut out)
            .expect("empty set queries");
        assert_eq!(n, 1);
        assert_eq!(
            out[0],
            MixVoice {
                gain: 1.0,
                send: 0.0
            }
        );
    }

    #[test]
    fn stone_wall_muffles_then_slews_to_chart() {
        let mut bridge = MixBridge::new(RATE).unwrap();
        let walls = [stone_cross_wall()];
        let seed = sfx_at(500);
        // First tick ducks by exactly one attack step: a stuck slew (target
        // applied at once, or never) fails here.
        let first = mix_one(&mut bridge, &seed, &walls);
        assert_eq!(bridge.state_q16(0), Some(GAIN_ONE - ATTACK_STEP));
        assert!((first.gain - q16_to_gain(GAIN_ONE - ATTACK_STEP)).abs() < 1e-6);
        assert_eq!(first.send, TUNNEL_WET);
        // Settled ticks land exactly on the charted stone pass (0.4).
        for _ in 0..16 {
            mix_one(&mut bridge, &seed, &walls);
        }
        let charted = apply_wall(GAIN_ONE, Material::Stone);
        assert_eq!(bridge.state_q16(0), Some(charted));
        let settled = mix_one(&mut bridge, &seed, &walls);
        // Same conversion the bridge applies, so this is exact; the charted
        // stone pass (26214 Q16) sits at -7.96 dB, not literal 0.4.
        assert_eq!(settled.gain, q16_to_gain(charted));
    }

    #[test]
    fn voice_bus_holds_minus_12db_behind_cloth() {
        let mut bridge = MixBridge::new(RATE).unwrap();
        let walls = [cloth_cross_wall()];
        // Cloth alone passes 0.2 (-13.98 dB): the voice bus floors at -12 dB
        // while sfx takes the full muffle. Slot 0 runs the voice seed and
        // slot 1 the sfx seed, both settled to their targets below.
        let mut out = [MixVoice {
            gain: -1.0,
            send: -1.0,
        }; 2];
        let seeds = [voice_at(500), sfx_at(500)];
        for _ in 0..16 {
            bridge
                .update(&seeds, LISTENER, LISTENER_3, &walls, &mut out)
                .expect("one wall queries");
        }
        let floor = q16_to_gain(VOICE_FLOOR_Q16);
        assert_eq!(out[0].gain, floor, "voice bus holds the floor");
        assert!((floor - 0.2512).abs() < 0.002, "floor {floor} is -12 dB");
        assert_eq!(
            out[1].gain,
            q16_to_gain(apply_wall(GAIN_ONE, Material::Cloth)),
            "sfx takes the full cloth muffle"
        );
        assert!(out[0].gain > out[1].gain, "floor keeps speech above sfx");
    }

    #[test]
    fn same_script_replays_the_same_gain_curve() {
        let walls = [stone_cross_wall()];
        let seeds = [sfx_at(500), voice_at(-300)];
        let mut first_out = [MixVoice {
            gain: -1.0,
            send: -1.0,
        }; 2];
        let mut second_out = [MixVoice {
            gain: -1.0,
            send: -1.0,
        }; 2];
        let mut first = MixBridge::new(RATE).unwrap();
        let mut second = MixBridge::new(RATE).unwrap();
        for _ in 0..12 {
            first
                .update(&seeds, LISTENER, LISTENER_3, &walls, &mut first_out)
                .unwrap();
            second
                .update(&seeds, LISTENER, LISTENER_3, &walls, &mut second_out)
                .unwrap();
            assert_eq!(first_out, second_out, "gain curve replayed");
        }
        assert_eq!(first.state_q16(0), second.state_q16(0));
        assert_eq!(first.state_q16(1), second.state_q16(1));
    }

    #[test]
    fn too_many_walls_refuse_without_moving_the_slew() {
        let mut bridge = MixBridge::new(RATE).unwrap();
        let open = [sfx_at(500)];
        let mut out = [MixVoice {
            gain: -1.0,
            send: -1.0,
        }; 1];
        // Duck partway so a moved slew would show.
        let walls = [stone_cross_wall()];
        mix_one(&mut bridge, &open[0], &walls);
        let mid = bridge.state_q16(0).expect("slot 0 exists");
        assert!(mid < GAIN_ONE && mid > apply_wall(GAIN_ONE, Material::Stone));
        // Oversized set refuses and the slew sits exactly where it was.
        let pile = vec![stone_cross_wall(); MAX_WALLS_PER_QUERY + 1];
        let err = bridge
            .update(&open, LISTENER, LISTENER_3, &pile, &mut out)
            .expect_err("over-cap set refuses");
        assert_eq!(
            err,
            OccludeError::TooManyWalls {
                got: MAX_WALLS_PER_QUERY + 1
            }
        );
        assert_eq!(bridge.state_q16(0), Some(mid));
        assert_eq!(out[0].gain, -1.0, "refused tick writes nothing");
    }

    #[test]
    fn bad_sample_rate_refuses_at_construction() {
        assert!(MixBridge::new(0).is_err(), "rate 0 refuses");
        assert!(MixBridge::new(7_999).is_err(), "below floor refuses");
        assert!(MixBridge::new(192_001).is_err(), "above ceiling refuses");
        assert!(MixBridge::new(RATE).is_ok(), "48 kHz builds");
    }

    #[test]
    fn tails_stay_glued_across_a_heal_reopen() {
        use engine_audio::heal::DeviceHeal;
        use engine_audio::Mixer;

        let mut bridge = MixBridge::new(RATE).unwrap();
        let walls = [stone_cross_wall()];
        let seeds = [sfx_at(500)];
        let mut out = [MixVoice {
            gain: -1.0,
            send: -1.0,
        }; 1];
        // Three ticks: mid-slew, tails ringing, nowhere near settled.
        for _ in 0..3 {
            bridge
                .update(&seeds, LISTENER, LISTENER_3, &walls, &mut out)
                .unwrap();
        }
        let params_before = bridge.tunnel_params().expect("params build");
        let slew_before = bridge.state_q16(0).expect("slot 0 exists");
        assert!(slew_before < GAIN_ONE);
        assert!(slew_before > apply_wall(GAIN_ONE, Material::Stone));

        // Device ladder: loss degrades, the silent mixer renders while down,
        // the reopen succeeds — and the bridge above never resets.
        let mut heal = DeviceHeal::new();
        let mut mixer = Mixer::new(RATE);
        heal.report_loss(7).expect("first loss degrades");
        assert!(!heal.is_live());
        heal.render_silent_frames(&mut mixer, 800);
        let (_device, _report) = heal
            .try_reopen(8, || Ok::<(), String>(()))
            .expect("reopen succeeds");
        assert!(heal.is_live());

        assert_eq!(bridge.tunnel_params().unwrap(), params_before);
        assert_eq!(bridge.state_q16(0), Some(slew_before));
        // The tail still converges on the charted stone pass afterwards.
        for _ in 0..16 {
            bridge
                .update(&seeds, LISTENER, LISTENER_3, &walls, &mut out)
                .unwrap();
        }
        assert_eq!(
            bridge.state_q16(0),
            Some(apply_wall(GAIN_ONE, Material::Stone))
        );
    }

    #[test]
    fn burst_past_the_cap_keeps_the_deterministic_head() {
        let mut bridge = MixBridge::new(RATE).unwrap();
        let seeds = vec![sfx_at(500); MAX_MIX_VOICES + 8];
        let mut out = vec![
            MixVoice {
                gain: -1.0,
                send: -1.0,
            };
            MAX_MIX_VOICES + 8
        ];
        let n = bridge
            .update(&seeds, LISTENER, LISTENER_3, &[], &mut out)
            .expect("empty set queries");
        assert_eq!(n, MAX_MIX_VOICES);
        assert!(out[..MAX_MIX_VOICES].iter().all(|m| *m
            == MixVoice {
                gain: 1.0,
                send: TUNNEL_WET
            }));
        assert_eq!(
            out[MAX_MIX_VOICES].gain, -1.0,
            "slots past the cap stay untouched"
        );
        assert!(bridge.state_q16(MAX_MIX_VOICES).is_none());
    }

    #[test]
    fn reset_open_reopens_the_slew_without_heap() {
        let mut bridge = MixBridge::new(RATE).unwrap();
        let walls = [stone_cross_wall()];
        mix_one(&mut bridge, &sfx_at(500), &walls);
        assert!(bridge.state_q16(0).expect("slot 0") < GAIN_ONE);
        bridge.reset_open();
        assert_eq!(bridge.state_q16(0), Some(GAIN_ONE));
        let mixed = mix_one(&mut bridge, &sfx_at(500), &[]);
        assert_eq!(mixed.gain, 1.0);
    }
}
