//! mix: combat mix over shared clips.
//!
//! [`CombatMix`] plays [`CuePlay`](crate::combat_cues::CuePlay)s through a
//! [`SpatialMixer`]: gain is full inside the cue's falloff min, falls as
//! `min / distance` out to its falloff max, then the play is culled with no
//! voice. Full groups steal by priority, never by age; each group holds at
//! most its instance limit. The camera is the listener, set once per tick.
//! Variant picks are seeded per cue, so the same script replays
//! bit-identical bytes. Offline and test path: the live callback drains the
//! same bank through the feed instead (see [`crate::AudioEngine`]).

use std::sync::Arc;

#[cfg(test)]
use crate::combat_cues::cue_for_event;
use crate::combat_cues::{footstep, CuePlay, Surface};
#[cfg(test)]
use crate::crowd::FightCue;
use crate::cue::{CueBank, CueId, Group, VariantPicker};
use crate::spatial::{MixCmd, SpatialMixer};
use crate::{MixerStats, VoiceId};

/// Most simultaneous impact voices (hits, blocks, parries, knockdowns).
pub const IMPACT_LIMIT: u8 = 8;
/// Most simultaneous swing voices.
pub const SWING_LIMIT: u8 = 4;
/// Most simultaneous footstep voices.
pub const FOOTSTEP_LIMIT: u8 = 6;

/// Combat mix: seeded cue plays through a positional mixer with a proof log.
pub struct CombatMix {
    bank: Arc<CueBank>,
    inner: SpatialMixer,
    pickers: Vec<VariantPicker>,
    played: u64,
}

impl CombatMix {
    /// Mix `bank` at `sample_rate` Hz with `seed` fixing every variant pick.
    /// Group limits are the combat caps above, so a flurry of hits never
    /// eats the footsteps.
    pub fn new(bank: Arc<CueBank>, sample_rate: u32, seed: u64) -> Self {
        let pickers = (0..bank.len())
            .map(|idx| VariantPicker::new(seed, CueId(u16::try_from(idx).unwrap_or(0))))
            .collect();
        let mut inner = SpatialMixer::new(sample_rate);
        inner
            .mixer_mut()
            .set_group_limit(Group::Impact, IMPACT_LIMIT);
        inner.mixer_mut().set_group_limit(Group::Swing, SWING_LIMIT);
        inner
            .mixer_mut()
            .set_group_limit(Group::Footstep, FOOTSTEP_LIMIT);
        Self {
            bank,
            inner,
            pickers,
            played: 0,
        }
    }

    /// Move the listener (the camera): `[east, up, south]` cm and yaw degrees
    /// clockwise from north. Call once per tick before the plays.
    pub fn set_camera(&mut self, pos_cm: [i32; 3], yaw_deg: i32) {
        self.inner.set_listener_3d(pos_cm, yaw_deg);
    }

    /// Play one mapped cue: resolve the name, pick the seeded variant, place
    /// it against the listener. `None` when the name is unknown, the variant
    /// is missing, the source is past falloff max (culled), or the mixer
    /// refused the voice (group muted, nothing weaker to steal).
    pub fn play(&mut self, play: CuePlay) -> Option<VoiceId> {
        let id = self.bank.id(play.name)?;
        let cue = self.bank.cue(id)?;
        let picker = self.pickers.get_mut(usize::from(id.0))?;
        let variant = picker.pick(cue.variants.len());
        let voice = self
            .inner
            .play_cue(id, cue, variant, play.gain, play.pos_cm)?;
        self.played += 1;
        Some(voice)
    }

    /// Map `event` onto its cue and play it. `None` for silent events
    /// (`StaminaOut`) or a refused play (see [`Self::play`]).
    ///
    /// Tests-only until wired to game live audio (`observe_into`, outside
    /// this packet): no production caller yet, so it stays behind `cfg(test)`.
    #[cfg(test)]
    pub fn play_event(
        &mut self,
        event: FightCue,
        attacker_pos: [i32; 3],
        victim_pos: [i32; 3],
    ) -> Option<VoiceId> {
        self.play(cue_for_event(event, attacker_pos, victim_pos)?)
    }

    /// One footstep on `surface` at `pos_cm`.
    pub fn play_footstep(&mut self, surface: Surface, pos_cm: [i32; 3]) -> Option<VoiceId> {
        self.play(footstep(surface, pos_cm))
    }

    /// Cap simultaneous voices of `group` (0 mutes the group).
    pub fn set_group_limit(&mut self, group: Group, limit: u8) {
        self.inner.mixer_mut().set_group_limit(group, limit);
    }

    /// Live voices in `group`.
    pub fn group_voices(&self, group: Group) -> usize {
        self.inner.mixer().group_voices(group)
    }

    /// Voices started so far.
    pub fn played(&self) -> u64 {
        self.played
    }

    /// Plays culled past their falloff max.
    pub fn culled(&self) -> u64 {
        self.inner.culled()
    }

    /// Voice counters of the inner mixer (started, stolen, rejected).
    pub fn stats(&self) -> MixerStats {
        self.inner.mixer_stats()
    }

    /// Recorded mix commands in issue order (proof log).
    pub fn commands(&self) -> &[MixCmd] {
        self.inner.commands()
    }

    /// Live (unfinished) voice count.
    pub fn voice_count(&self) -> usize {
        self.inner.voice_count()
    }

    /// True when no voice has frames left to render.
    pub fn is_idle(&self) -> bool {
        self.inner.is_idle()
    }

    /// Mix interleaved stereo (`LRLR...`) in place.
    pub fn render_stereo_into(&mut self, out: &mut [f32]) {
        self.inner.render_stereo_into(out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cue::names;

    const RATE: u32 = 8_000;

    fn bank() -> Arc<CueBank> {
        Arc::new(CueBank::placeholder(RATE).expect("shipped table is valid"))
    }

    fn hit() -> FightCue {
        FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 120,
        }
    }

    #[test]
    fn combat_caps_match_the_group_defaults() {
        assert_eq!(IMPACT_LIMIT, Group::Impact.default_limit());
        assert_eq!(SWING_LIMIT, Group::Swing.default_limit());
        assert_eq!(FOOTSTEP_LIMIT, Group::Footstep.default_limit());
    }

    #[test]
    fn close_hit_is_full_and_panned_by_the_camera() {
        let mut mix = CombatMix::new(bank(), RATE, 2040);
        mix.set_camera([0, 0, 0], 0);
        // 300 east, 100 south: inside the 800 cm falloff min, 300 right.
        mix.play_event(hit(), [0, 0, 0], [300, 0, 100])
            .expect("in range");
        let cmd = mix.commands()[0];
        assert_eq!((cmd.atten, cmd.pan), (1.0, 0.3));
        assert_eq!((cmd.left_gain, cmd.right_gain), (0.7, 1.0));
    }

    #[test]
    fn gain_falls_as_one_over_r_then_culls() {
        let mut mix = CombatMix::new(bank(), RATE, 2040);
        mix.set_camera([0, 0, 0], 0);
        // Twice the 800 cm min: half gain, hard right.
        mix.play_event(hit(), [0, 0, 0], [1_600, 0, 0])
            .expect("in range");
        let cmd = mix.commands()[0];
        assert_eq!(cmd.dist_cm, 1_600);
        assert_eq!(cmd.atten, 0.5);
        assert_eq!((cmd.left_gain, cmd.right_gain), (0.0, 0.5));
        // Past the 6000 cm max: no voice, counted.
        assert!(mix.play_event(hit(), [0, 0, 0], [6_001, 0, 0]).is_none());
        assert_eq!((mix.culled(), mix.voice_count()), (1, 1));
        // Exactly at max still sounds.
        mix.play_event(hit(), [0, 0, 0], [6_000, 0, 0])
            .expect("at max");
        assert_eq!(mix.culled(), 1);
    }

    #[test]
    fn full_group_steals_by_priority_not_by_age() {
        let mut mix = CombatMix::new(bank(), RATE, 2040);
        mix.set_camera([0, 0, 0], 0);
        mix.set_group_limit(Group::Impact, 1);
        let pos = [100, 0, 0];
        mix.play_event(hit(), [0, 0, 0], pos).expect("room");
        // Block (160) is weaker than hit (180): refused, hit keeps sounding.
        assert!(mix
            .play(CuePlay {
                name: names::BLOCK,
                gain: 1.0,
                pos_cm: Some(pos),
            })
            .is_none());
        assert_eq!(mix.group_voices(Group::Impact), 1);
        // Knockdown (200) outranks the hit: steals it.
        mix.play_event(
            FightCue::Knockdown {
                attacker: 0,
                victim: 1,
            },
            [0, 0, 0],
            pos,
        )
        .expect("steals the hit");
        assert_eq!(mix.group_voices(Group::Impact), 1);
        assert_eq!(mix.stats().stolen, 1);
        assert_eq!(mix.stats().rejected, 1);
        // Another group is untouched by the full one.
        mix.play_footstep(Surface::Sand, pos).expect("own slots");
        assert_eq!(mix.group_voices(Group::Footstep), 1);
    }

    #[test]
    fn hits_fill_their_cap_then_oldest_goes_first() {
        let mut mix = CombatMix::new(bank(), RATE, 2040);
        mix.set_camera([0, 0, 0], 0);
        // Synth hits are long; all eight stay live.
        for _ in 0..IMPACT_LIMIT {
            mix.play_event(hit(), [0, 0, 0], [100, 0, 0]).expect("room");
        }
        assert_eq!(mix.group_voices(Group::Impact), usize::from(IMPACT_LIMIT));
        mix.play_event(hit(), [0, 0, 0], [100, 0, 0])
            .expect("equal priority steals the oldest");
        assert_eq!(mix.group_voices(Group::Impact), usize::from(IMPACT_LIMIT));
        assert_eq!(mix.stats().stolen, 1);
        assert_eq!(mix.played(), u64::from(IMPACT_LIMIT) + 1);
    }

    #[test]
    fn same_seed_and_script_replay_byte_identical() {
        let script = [
            hit(),
            FightCue::Blocked {
                attacker: 1,
                victim: 0,
                chip: 24,
            },
            FightCue::Parried {
                attacker: 0,
                victim: 1,
            },
        ];
        let run = |seed: u64| {
            let mut mix = CombatMix::new(bank(), RATE, seed);
            mix.set_camera([0, 300, 0], 0);
            for (i, event) in script.iter().enumerate() {
                let at = i32::try_from(i * 100).unwrap_or(0);
                mix.play_event(*event, [at, 0, 0], [at + 200, 0, 50]);
            }
            mix.play_footstep(Surface::Sand, [50, 0, 0]);
            let mut out = vec![0.0f32; 4_000];
            mix.render_stereo_into(&mut out);
            out
        };
        let first = run(2040);
        assert!(first.iter().any(|&s| s != 0.0), "the fight is audible");
        assert_eq!(first, run(2040), "same seed replays");
        assert_ne!(first, run(2041), "another seed picks other takes");
    }

    #[test]
    fn unknown_cue_plays_nothing() {
        let mut mix = CombatMix::new(bank(), RATE, 2040);
        assert!(mix
            .play(CuePlay {
                name: "nope",
                gain: 1.0,
                pos_cm: None,
            })
            .is_none());
        assert_eq!(mix.played(), 0);
        assert!(mix.is_idle());
    }
}
