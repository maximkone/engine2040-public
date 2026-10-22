//! engine: the audio callback's owner.
//!
//! [`AudioEngine`] owns the mixer outright. The game thread never touches
//! it: it writes tick-stamped commands into a [`FeedTx`], and the engine
//! drains the matching [`FeedRx`] at the top of every render call, so the
//! callback takes no lock and allocates nothing. Cue names became ids on the
//! game thread; here an id picks a seeded variant and starts a voice that
//! shares the clip's samples.

use std::sync::Arc;

use crate::cue::{CueBank, VariantPicker};
use crate::feed::{feed, Command, FeedRx, FeedTx, MixCounts};
use crate::heal::{AudioHealError, DeviceHeal, HealReport};
use crate::playlist::{MusicBed, MusicState};
use crate::SpatialMixer;
use engine_observe::supervisor::SupervisedDecision;

/// Most commands one render call applies, so a flooding producer cannot
/// stretch a callback; the rest wait for the next buffer.
pub const DRAIN_LIMIT: usize = 4_096;

/// Anything the device callback can pull interleaved stereo from.
pub trait FrameSource: Send + 'static {
    /// Fill `out` with interleaved `LRLR...` frames. Runs on the audio
    /// thread: it must neither block nor allocate.
    fn render_stereo_into(&mut self, out: &mut [f32]);

    /// The device callback measured a hit-to-speaker latency past the UX
    /// budget: start one-shot cues `frames` into their clip from now on, so
    /// the sound reaches the speaker that much sooner (`0` is on time).
    /// Sources that do not start cues ignore it.
    fn set_early_frames(&mut self, _frames: u32) {}
}

/// Cue mixer plus its command feed; see the module docs.
pub struct AudioEngine {
    bank: Arc<CueBank>,
    rx: FeedRx,
    mixer: SpatialMixer,
    pickers: Vec<VariantPicker>,
    music: Option<MusicBed>,
    applied: u64,
    last_tick: u32,
    early_frames: u32,
    heal: DeviceHeal,
}

impl AudioEngine {
    /// Build the engine and the game-side [`FeedTx`] that feeds it. `seed`
    /// fixes every variant pick: the same seed and the same command sequence
    /// pick the same takes. `capacity` is the ring size (see
    /// [`crate::FEED_CAPACITY`]); `sample_rate` must match the clips in
    /// `bank` (the loader resamples to it).
    pub fn with_feed(
        bank: Arc<CueBank>,
        capacity: usize,
        sample_rate: u32,
        seed: u64,
    ) -> (FeedTx, Self) {
        let (tx, rx) = feed(capacity, Arc::clone(&bank));
        let pickers = (0..bank.len())
            .map(|idx| VariantPicker::new(seed, crate::CueId(u16::try_from(idx).unwrap_or(0))))
            .collect();
        let music = bank
            .music()
            .map(|music| MusicBed::new(Arc::clone(music), seed));
        let engine = Self {
            bank,
            rx,
            mixer: SpatialMixer::live(sample_rate),
            pickers,
            music,
            applied: 0,
            last_tick: 0,
            early_frames: 0,
            heal: DeviceHeal::new(),
        };
        (tx, engine)
    }

    /// Apply queued commands, oldest first, up to [`DRAIN_LIMIT`]; returns
    /// how many. Called at the top of each render, public so offline tests
    /// can step it.
    pub fn drain(&mut self) -> usize {
        let mut n = 0;
        while n < DRAIN_LIMIT {
            let Some(cmd) = self.rx.pop() else {
                break;
            };
            self.apply(cmd);
            self.applied += 1;
            self.last_tick = cmd.tick();
            n += 1;
        }
        n
    }

    fn apply(&mut self, cmd: Command) {
        match cmd {
            Command::Play(play) => {
                let Some(cue) = self.bank.cue(play.cue) else {
                    return;
                };
                // The producer may name the exact take (the round robin of
                // the impact table); otherwise the audio side picks one.
                let variant = match play.take {
                    Some(take) => usize::from(take),
                    None => {
                        let Some(picker) = self.pickers.get_mut(usize::from(play.cue.0)) else {
                            return;
                        };
                        picker.pick(cue.variants.len())
                    }
                };
                let voice = self
                    .mixer
                    .play_cue(play.cue, cue, variant, play.gain, play.pos_cm);
                if let Some(id) = voice.filter(|_| self.early_frames > 0 && !cue.looped) {
                    self.mixer.mixer_mut().skip_voice(id, self.early_frames);
                }
            }
            Command::Listener {
                pos_cm, yaw_deg, ..
            } => self.mixer.set_listener_3d(pos_cm, yaw_deg),
            Command::Stop { cue, .. } => {
                self.mixer.stop_cue(cue);
            }
            Command::Trim { cue, gain, .. } => {
                self.mixer.set_cue_trim(cue, gain);
            }
            Command::Tension { level, .. } => {
                if let Some(bed) = self.music.as_mut() {
                    bed.request(level, self.mixer.mixer_mut());
                }
            }
        }
    }

    /// What the music bed is doing; `None` when the bank carries no music.
    pub fn music_state(&self) -> Option<MusicState> {
        self.music.as_ref().map(MusicBed::state)
    }

    /// The owned mixer (listener, voice counters, duck level).
    pub fn mixer(&self) -> &SpatialMixer {
        &self.mixer
    }

    /// The owned mixer, mutable: group limits.
    pub fn mixer_mut(&mut self) -> &mut SpatialMixer {
        &mut self.mixer
    }

    /// Commands applied so far.
    pub fn applied(&self) -> u64 {
        self.applied
    }

    /// Report a lost default device: the heal degrades to the silent mixer
    /// and the engine keeps advancing voices, so the game never stops
    /// playing while the device is away.
    ///
    /// # Errors
    ///
    /// Same as [`DeviceHeal::report_loss`]: a typed refusal once the budget
    /// is spent, a supervisor fault when the tape needs exporting.
    pub fn report_loss(&mut self, tick: u64) -> Result<SupervisedDecision, AudioHealError> {
        self.heal.report_loss(tick)
    }

    /// Report a changed default device (headphones plugged, HDMI switched):
    /// the same reconnect policy as a loss, with its own notice wording.
    ///
    /// # Errors
    ///
    /// Same as [`DeviceHeal::report_change`].
    pub fn report_change(&mut self, tick: u64) -> Result<SupervisedDecision, AudioHealError> {
        self.heal.report_change(tick)
    }

    /// The device heal: its state, notice, attempts and silent frames are
    /// readable here, so a host can show the player why the sound is quiet.
    #[must_use]
    pub fn heal(&self) -> &DeviceHeal {
        &self.heal
    }

    /// Try to reopen the device once through the heal: on success the budget
    /// resets, the engine goes live and the report says how long the device
    /// was away; on refusal the backoff escalates.
    ///
    /// # Errors
    ///
    /// Same as [`DeviceHeal::try_reopen`]: a typed refusal while budget
    /// remains, a loud quarantine past it.
    pub fn try_reopen<T, E>(
        &mut self,
        tick: u64,
        opener: impl FnOnce() -> Result<T, E>,
    ) -> Result<(T, HealReport), AudioHealError> {
        self.heal.try_reopen(tick, opener)
    }
}

impl FrameSource for AudioEngine {
    fn set_early_frames(&mut self, frames: u32) {
        self.early_frames = frames;
    }

    fn render_stereo_into(&mut self, out: &mut [f32]) {
        self.drain();
        // Render in runs that end on the music's bar lines, so a level switch
        // lands on its exact frame whatever the device buffer size.
        let frames = out.len() / 2;
        let mut done = 0;
        while done < frames {
            let left = frames - done;
            let run = self
                .music
                .as_ref()
                .map_or(left, |bed| left.min(bed.frames_to_boundary()));
            self.mixer
                .render_stereo_into(&mut out[done * 2..(done + run) * 2]);
            if let Some(bed) = self.music.as_mut() {
                bed.advance(run, self.mixer.mixer_mut());
            }
            done += run;
        }
        if let Some(odd) = out.get_mut(frames * 2..) {
            odd.fill(0.0);
        }
        if !self.heal.is_live() {
            // The device is down: the mixer keeps advancing (voices and the
            // music bed move on), the wire gets silence and every frame
            // counts as a silent frame.
            out.fill(0.0);
            self.heal.count_silent_frames(frames);
        }
        let stats = self.mixer.mixer_stats();
        self.rx.publish(
            self.applied,
            self.last_tick,
            MixCounts {
                stolen: stats.stolen,
                rejected: stats.rejected,
                culled: self.mixer.culled(),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cue::{Clip, Cue, CueId, Group, Spatial};
    use crate::dsp::Bus;
    use crate::{FeedError, Pcm};

    const RATE: u32 = 8_000;

    fn cue(name: &str, takes: &[f32], spatial: Spatial, bus: Bus, looped: bool) -> Cue {
        Cue {
            name: name.to_string(),
            variants: takes
                .iter()
                .map(|&v| Clip::from_pcm(&Pcm::new(RATE, 1, vec![v; 64])))
                .collect(),
            spatial,
            group: Group::Impact,
            looped,
            priority: 100,
            bus,
            gain: 1.0,
            ducks_music: false,
        }
    }

    fn test_bank() -> Arc<CueBank> {
        let near = Spatial::Positional {
            falloff_min_cm: 100,
            falloff_max_cm: 1_000,
        };
        Arc::new(
            CueBank::new(vec![
                cue("bed", &[0.1], Spatial::Flat, Bus::Sfx, true),
                cue("clack", &[0.1, 0.2, 0.3, 0.4], near, Bus::Sfx, false),
                cue("flat", &[0.25], Spatial::Flat, Bus::Sfx, false),
            ])
            .expect("valid test cues"),
        )
    }

    /// Frame 0 of a stereo render after the engine drains its feed.
    fn first_frame(engine: &mut AudioEngine) -> (f32, f32) {
        let mut out = [0.0f32; 2];
        engine.render_stereo_into(&mut out);
        (out[0], out[1])
    }

    #[test]
    fn queued_plays_reach_the_mixer_in_order_with_their_stamps() {
        let (mut tx, mut engine) = AudioEngine::with_feed(test_bank(), 16, RATE, 1);
        tx.listener(1, [0, 0, 0], 0).expect("room");
        tx.play(2, "flat", 1.0, None).expect("room");
        tx.play(3, "flat", 1.0, None).expect("room");
        assert_eq!(engine.drain(), 3);
        assert_eq!(engine.mixer().voice_count(), 2);
        let stats = tx.stats();
        assert_eq!((stats.sent, stats.dropped_full), (3, 0));
        let mut out = [0.0f32; 4];
        engine.render_stereo_into(&mut out);
        let stats = tx.stats();
        assert_eq!((stats.applied, stats.last_applied_tick), (3, 3));
        assert!(out.iter().all(|&s| s > 0.0), "two flat voices sounding");
    }

    #[test]
    fn a_late_device_starts_one_shot_cues_early() {
        // The flat clip is 64 frames of 0.25: count the frames it sounds.
        let sounding = |early: u32| {
            let (mut tx, mut engine) = AudioEngine::with_feed(test_bank(), 16, RATE, 1);
            engine.set_early_frames(early);
            tx.play(0, "flat", 1.0, None).expect("room");
            let mut out = vec![0.0f32; 2 * 128];
            engine.render_stereo_into(&mut out);
            out.chunks_exact(2).filter(|f| f[0] > 0.0).count()
        };
        assert_eq!(sounding(0), 64, "on time: the whole clip");
        assert_eq!(sounding(10), 54, "10 frames early: the head is skipped");
        assert_eq!(sounding(500), 32, "never more than half the clip");
    }

    #[test]
    fn same_seed_and_sequence_pick_the_same_takes() {
        let run = |seed: u64| {
            let (mut tx, mut engine) = AudioEngine::with_feed(test_bank(), 64, RATE, seed);
            let mut heard = Vec::new();
            for t in 0..24 {
                tx.play(t, "clack", 1.0, None).expect("room");
                // 64 frames: the whole clip, so voices never overlap.
                let mut out = [0.0f32; 128];
                engine.render_stereo_into(&mut out);
                heard.push(out[0].to_bits());
            }
            heard
        };
        let a = run(2040);
        assert_eq!(a, run(2040), "same seed replays the same variants");
        assert_ne!(a, run(2041), "another seed picks another sequence");
        let distinct: std::collections::BTreeSet<_> = a.iter().collect();
        assert_eq!(distinct.len(), 4, "all four takes heard in 24 plays");
    }

    #[test]
    fn listener_command_moves_the_mix() {
        let src = Some([500, 0, 0]);
        let (mut tx, mut engine) = AudioEngine::with_feed(test_bank(), 16, RATE, 7);
        tx.listener(0, [500, 0, 0], 0).expect("room");
        tx.play(0, "clack", 1.0, src).expect("room");
        let on_top = first_frame(&mut engine);
        let (mut tx, mut engine) = AudioEngine::with_feed(test_bank(), 16, RATE, 7);
        tx.listener(0, [-400, 0, 0], 0).expect("room");
        tx.play(0, "clack", 1.0, src).expect("room");
        let far = first_frame(&mut engine);
        // 900 cm away is 100/900 of the level, and the source is hard right.
        assert!(on_top.0 > far.0 * 8.0, "{on_top:?} vs {far:?}");
        assert!(
            far.0 < far.1 || far.0 == 0.0,
            "source right of the listener"
        );
    }

    #[test]
    fn stop_command_ends_a_loop() {
        let (mut tx, mut engine) = AudioEngine::with_feed(test_bank(), 16, RATE, 3);
        tx.play(0, "bed", 1.0, None).expect("room");
        let mut out = vec![0.0f32; 400];
        engine.render_stereo_into(&mut out);
        assert!(
            engine.mixer().voice_count() == 1,
            "a 64-frame loop outlives 200 frames"
        );
        tx.stop(1, "bed").expect("room");
        engine.render_stereo_into(&mut out);
        assert!(engine.mixer().is_idle());
        assert!(out.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn trim_command_rides_a_looped_bed_without_restarting_it() {
        let (mut tx, mut engine) = AudioEngine::with_feed(test_bank(), 16, RATE, 3);
        tx.play(0, "bed", 1.0, None).expect("room");
        let level = |engine: &mut AudioEngine| {
            let mut out = [0.0f32; 8];
            engine.render_stereo_into(&mut out);
            out[0]
        };
        let full = level(&mut engine);
        assert!(full > 0.0, "the bed sounds");
        tx.trim(1, "bed", 0.5).expect("room");
        let half = level(&mut engine);
        assert!((half - full * 0.5).abs() < 1e-6, "{half} vs {full}");
        tx.trim(2, "bed", 0.0).expect("room");
        assert_eq!(level(&mut engine), 0.0, "muted, not stopped");
        assert_eq!(engine.mixer().voice_count(), 1, "same voice, still looping");
        tx.trim(3, "bed", 1.0).expect("room");
        assert_eq!(level(&mut engine), full, "back to the play level");
        // A cue the bank lacks is counted, a cue with no live voice is a no-op.
        assert_eq!(tx.trim(4, "nope", 1.0), Err(FeedError::UnknownCue));
        tx.trim(4, "clack", 0.3).expect("room");
        level(&mut engine);
        let stats = tx.stats();
        assert_eq!((stats.unknown_cue, stats.sent), (1, stats.applied));
    }

    #[test]
    fn render_with_commands_in_flight_allocates_nothing() {
        let (mut tx, mut engine) = AudioEngine::with_feed(test_bank(), 64, RATE, 9);
        let pos = Some([300, 0, 100]);
        // Warm every path once: a play, a listener move, a stop.
        tx.listener(0, [0, 0, 0], 30).expect("room");
        tx.play(0, "clack", 1.0, pos).expect("room");
        tx.stop(0, "bed").expect("room");
        let mut out = vec![0.0f32; 256];
        engine.render_stereo_into(&mut out);
        let ((), heap) = engine_core::alloc_probe::measure(|| {
            for t in 1..40 {
                tx.listener(t, [t.try_into().unwrap_or(0), 0, 0], 45).ok();
                tx.play(t, "clack", 0.5, pos).ok();
                tx.play(t, "flat", 0.5, None).ok();
                engine.render_stereo_into(&mut out);
            }
        });
        assert_eq!(heap.calls, 0, "callback path: {} bytes", heap.bytes);
        assert!(out.iter().any(|&s| s != 0.0));
    }

    #[test]
    fn one_render_applies_a_bounded_batch_and_the_rest_wait() {
        let (mut tx, mut engine) = AudioEngine::with_feed(test_bank(), 8_192, RATE, 4);
        let queued = DRAIN_LIMIT + 10;
        for t in 0..queued {
            tx.listener(u32::try_from(t).unwrap_or(0), [0, 0, 0], 0)
                .expect("room");
        }
        assert_eq!(engine.drain(), DRAIN_LIMIT);
        assert_eq!(engine.drain(), 10, "nothing is lost, only deferred");
        assert_eq!(engine.drain(), 0);
        assert_eq!(engine.applied(), u64::try_from(queued).unwrap_or(0));
    }

    #[test]
    fn closing_the_engine_closes_the_feed() {
        let (mut tx, engine) = AudioEngine::with_feed(test_bank(), 4, RATE, 0);
        drop(engine);
        assert_eq!(tx.play(0, "flat", 1.0, None), Err(FeedError::Closed));
    }

    #[test]
    fn contended_feed_loses_nothing() {
        // The tick-stamped feed owns the device path: 1000 commands with
        // room for all of them drain to empty with nothing dropped, so a
        // busy callback defers work instead of silencing it like the
        // deleted `try_lock` hand-off did.
        const COMMANDS: u64 = 1_000;
        let (mut tx, mut engine) = AudioEngine::with_feed(test_bank(), 1_024, RATE, 2040);
        for tick in 0..u32::try_from(COMMANDS).unwrap_or(0) {
            tx.play(tick, "clack", 1.0, None)
                .expect("room for all 1000");
        }
        let mut total = 0usize;
        loop {
            let n = engine.drain();
            if n == 0 {
                break;
            }
            total += n;
        }
        assert_eq!(
            u64::try_from(total).unwrap_or(0),
            COMMANDS,
            "every queued command applied"
        );
        assert_eq!(engine.applied(), COMMANDS);
        // Publish the counts so the game thread sees them, then check.
        let mut out = [0.0f32; 4];
        engine.render_stereo_into(&mut out);
        let stats = tx.stats();
        assert_eq!((stats.sent, stats.dropped_full), (COMMANDS, 0));
        assert_eq!(
            (stats.applied, stats.last_applied_tick),
            (COMMANDS, u32::try_from(COMMANDS - 1).unwrap_or(0))
        );
        // Seeded variant picks repeat per seed: the same seed replays the
        // same takes, another seed picks another sequence.
        let run = |seed: u64| {
            let (mut tx, mut engine) = AudioEngine::with_feed(test_bank(), 64, RATE, seed);
            let mut heard = Vec::new();
            for t in 0..24 {
                tx.play(t, "clack", 1.0, None).expect("room");
                let mut out = [0.0f32; 128];
                engine.render_stereo_into(&mut out);
                heard.push(out[0].to_bits());
            }
            heard
        };
        let first = run(2040);
        assert_eq!(first, run(2040), "same seed replays the same takes");
        assert_ne!(first, run(2041), "another seed picks another sequence");
    }

    /// The take the producer names is the take that sounds. The round robin
    /// of the impact table picks a take on the game thread; the audio
    /// thread must play that one, not pick its own.
    #[test]
    fn the_named_take_is_the_take_that_sounds() {
        // The "clack" cue has four takes, each a constant level: 0.1, 0.2,
        // 0.3, 0.4. The Sfx bus trims them all by the same factor, so the
        // ratios are what matter.
        let level = |take: u16| {
            let (mut tx, mut engine) = AudioEngine::with_feed(test_bank(), 16, RATE, 1);
            tx.play_take(0, "clack", 1.0, None, Some(take))
                .expect("room");
            let mut out = [0.0f32; 4];
            engine.render_stereo_into(&mut out);
            out[0]
        };
        let (low, mid, high) = (level(0), level(2), level(3));
        assert!(low > 0.0, "take 0 sounds");
        assert!(
            (mid / low - 3.0).abs() < 0.01,
            "take 2 is three times take 0: {low} {mid}"
        );
        assert!(
            (high / low - 4.0).abs() < 0.01,
            "take 3 is four times take 0: {low} {high}"
        );
    }

    #[test]
    fn unknown_ids_in_the_ring_are_ignored() {
        let (mut tx, mut engine) = AudioEngine::with_feed(test_bank(), 4, RATE, 0);
        tx.send(Command::Play(crate::AudioCommand {
            tick: 0,
            cue: CueId(77),
            gain: 1.0,
            pos_cm: None,
            take: None,
        }))
        .expect("room");
        assert_eq!(engine.drain(), 1);
        assert!(engine.mixer().is_idle());
    }

    use crate::music::Tension;
    use crate::playlist::MusicBank;

    /// The test cues plus a silent stinger (so only the duck is audible) and
    /// generated music.
    fn music_bank() -> Arc<CueBank> {
        let flat = Spatial::Flat;
        let mut sting = cue("sting", &[0.0], flat, Bus::Sfx, false);
        sting.variants = vec![Clip::from_pcm(&Pcm::new(RATE, 1, vec![0.0; 2_400]))];
        sting.group = Group::Stinger;
        sting.ducks_music = true;
        let cues = vec![
            cue("bed", &[0.1], flat, Bus::Sfx, true),
            cue("clack", &[0.1, 0.2, 0.3, 0.4], flat, Bus::Sfx, false),
            sting,
        ];
        let music = MusicBank::procedural(11, RATE).expect("music bank");
        Arc::new(CueBank::new(cues).expect("valid cues").with_music(music))
    }

    /// Render `frames` stereo frames in blocks of `block` frames.
    fn render(engine: &mut AudioEngine, frames: usize, block: usize) -> Vec<f32> {
        let mut all = Vec::with_capacity(frames * 2);
        let mut left = frames;
        while left > 0 {
            let n = left.min(block);
            let mut buf = vec![0.0f32; n * 2];
            engine.render_stereo_into(&mut buf);
            all.extend_from_slice(&buf);
            left -= n;
        }
        all
    }

    fn fingerprint(samples: &[f32]) -> u64 {
        let mut h = engine_core::Fnv64::new();
        for s in samples {
            h.write_f32(*s);
        }
        h.finish()
    }

    #[test]
    fn tension_commands_switch_the_bed_on_the_exact_bar_frame() {
        let bank = music_bank();
        let bar = bank.music().expect("music").bar_frames();
        let (mut tx, mut engine) = AudioEngine::with_feed(bank, 64, RATE, 21);
        assert!(
            engine.music_state().is_some_and(|s| s.level.is_none()),
            "silent until asked"
        );
        tx.tension(0, Tension::Calm).expect("room");
        render(&mut engine, bar / 2, 480);
        let s = engine.music_state().expect("music");
        assert_eq!(
            (s.level, s.pending, s.bar_pos),
            (Some(Tension::Calm), None, bar / 2)
        );

        tx.tension(1, Tension::Fight).expect("room");
        render(&mut engine, bar - bar / 2 - 1, 480);
        let s = engine.music_state().expect("music");
        assert_eq!(
            (s.level, s.pending),
            (Some(Tension::Calm), Some(Tension::Fight))
        );
        assert_eq!(
            engine.mixer().mixer().fading_voices(),
            0,
            "nothing moves before the bar line"
        );

        render(&mut engine, 1, 480);
        let s = engine.music_state().expect("music");
        assert_eq!(
            (s.level, s.pending, s.bar_pos),
            (Some(Tension::Fight), None, 0)
        );
        assert_eq!(
            engine.mixer().mixer().fading_voices(),
            1,
            "old loop crossfading out"
        );
        assert_eq!(tx.stats().applied, 2);
    }

    #[test]
    fn crossfade_output_is_continuous_across_the_bar_line() {
        let bank = music_bank();
        let bar = bank.music().expect("music").bar_frames();
        let (mut tx, mut engine) = AudioEngine::with_feed(bank, 64, RATE, 21);
        tx.tension(0, Tension::Calm).expect("room");
        tx.tension(1, Tension::Fight).expect("room");
        let out = render(&mut engine, bar * 3, 1_000);
        let left: Vec<f32> = out.chunks(2).map(|f| f[0]).collect();
        let step = |from: usize, to: usize| {
            left[from..to]
                .windows(2)
                .map(|w| (w[1] - w[0]).abs())
                .fold(0.0f32, f32::max)
        };
        // Bar 1 is the crossfade; the steps across its start must not exceed
        // what the music itself does inside a bar.
        let inside = step(bar / 4, bar - 8).max(step(bar + bar / 4, 2 * bar - 8));
        let across = step(bar - 4, bar + 4);
        assert!(
            across <= inside * 1.5 + 1e-4,
            "step across the line {across} vs inside {inside}"
        );
        assert!(left.iter().all(|s| s.abs() <= 1.0));
    }

    #[test]
    fn a_stinger_ducks_the_bed_then_the_bed_comes_back_unchanged() {
        let bank = music_bank();
        let bar = bank.music().expect("music").bar_frames();
        let make = || AudioEngine::with_feed(Arc::clone(&bank), 64, RATE, 33);
        let ((mut tx_a, mut plain), (mut tx_b, mut stung)) = (make(), make());
        for tx in [&mut tx_a, &mut tx_b] {
            tx.tension(0, Tension::Climax).expect("room");
        }
        let warm = bar * 2;
        assert_eq!(render(&mut plain, warm, 512), render(&mut stung, warm, 512));
        assert_eq!(
            stung.mixer().mixer().duck_gain(),
            1.0,
            "no stinger, no duck"
        );

        tx_b.stinger(5, "sting").expect("a stinger cue");
        assert_eq!(
            tx_b.stinger(5, "bed"),
            Err(crate::FeedError::UnknownCue),
            "only stingers qualify"
        );
        render(&mut plain, 600, 100);
        render(&mut stung, 600, 100);
        let floor = f32_floor();
        assert!(
            (stung.mixer().mixer().duck_gain() - floor).abs() < 0.01,
            "ducked to the floor"
        );
        // Same bed under the duck: the ratio of the two outputs is the duck gain.
        let a = render(&mut plain, 800, 100);
        let b = render(&mut stung, 800, 100);
        let ratio = crate::dsp::rms(&b) / crate::dsp::rms(&a);
        assert!((ratio - floor).abs() < 0.02, "bed at {ratio} of its level");

        // The stinger lasts 2400 frames, the release 4800: then the bed is back,
        // sample for sample.
        render(&mut plain, 8_000, 500);
        render(&mut stung, 8_000, 500);
        assert_eq!(stung.mixer().mixer().duck_gain(), 1.0);
        let a = render(&mut plain, 2_000, 500);
        let b = render(&mut stung, 2_000, 500);
        assert_eq!(
            fingerprint(&a),
            fingerprint(&b),
            "recovered bed is bit-identical"
        );
    }

    /// The music duck floor as a gain.
    fn f32_floor() -> f32 {
        f32::from(u8::try_from(crate::DUCK_FLOOR_PERCENT).unwrap_or(35)) / 100.0
    }

    #[test]
    fn same_seed_and_commands_replay_byte_identical_whatever_the_block_size() {
        let bank = music_bank();
        let bar = bank.music().expect("music").bar_frames();
        let run = |seed: u64, block: usize| {
            let (mut tx, mut engine) = AudioEngine::with_feed(Arc::clone(&bank), 64, RATE, seed);
            for (tick, level) in [Tension::Build, Tension::Climax, Tension::Calm]
                .into_iter()
                .enumerate()
            {
                tx.tension(u32::try_from(tick).unwrap_or(0), level)
                    .expect("room");
            }
            tx.stinger(3, "sting").expect("room");
            tx.play(4, "clack", 1.0, None).expect("room");
            fingerprint(&render(&mut engine, bar * 5, block))
        };
        let first = run(77, 480);
        assert_eq!(first, run(77, 480), "x2 byte-identical");
        assert_eq!(first, run(77, 1_024), "block size does not matter");
        assert_eq!(first, run(77, 37), "not even an awkward one");
        assert_ne!(first, run(78, 480), "another seed picks other takes");
    }

    #[test]
    fn music_and_stingers_render_without_allocating() {
        let bank = music_bank();
        let bar = bank.music().expect("music").bar_frames();
        let (mut tx, mut engine) = AudioEngine::with_feed(bank, 64, RATE, 5);
        tx.tension(0, Tension::Build).expect("room");
        tx.stinger(0, "sting").expect("room");
        let mut buf = vec![0.0f32; 1_024];
        for _ in 0..4 {
            engine.render_stereo_into(&mut buf);
        }
        let blocks = bar * 4 / 512;
        let ((), heap) = engine_core::alloc_probe::measure(|| {
            for i in 0..blocks {
                if i % 7 == 0 {
                    let level = Tension::from_level(u8::try_from(i % 4).unwrap_or(0))
                        .unwrap_or(Tension::Calm);
                    tx.tension(1, level).ok();
                    tx.stinger(1, "sting").ok();
                }
                engine.render_stereo_into(&mut buf);
            }
        });
        assert_eq!(
            heap.calls, 0,
            "bar switches and stingers: {} bytes",
            heap.bytes
        );
        assert!(buf.iter().any(|&s| s != 0.0));
    }

    #[test]
    fn tension_without_music_is_ignored() {
        let (mut tx, mut engine) = AudioEngine::with_feed(test_bank(), 8, RATE, 1);
        tx.tension(0, Tension::Climax).expect("room");
        assert_eq!(engine.drain(), 1);
        assert!(engine.music_state().is_none());
        assert!(engine.mixer().is_idle());
    }

    #[test]
    fn a_device_loss_degrades_the_engine_to_silence_and_the_heal_state_is_readable() {
        use crate::heal::DeviceState;

        let (mut tx, mut engine) = AudioEngine::with_feed(test_bank(), 16, RATE, 1);
        tx.play(0, "flat", 1.0, None).expect("room");
        let mut out = [0.0f32; 64];
        engine.render_stereo_into(&mut out);
        assert!(out.iter().any(|&s| s > 0.0), "live: the voice sounds");
        assert!(engine.heal().is_live());
        assert_eq!(engine.heal().state(), DeviceState::Live);

        // The mixer path reports the loss into the heal; the state is readable.
        let decision = engine.report_loss(10).expect("first loss must heal");
        assert_eq!(decision.attempt, 1);
        assert_eq!(engine.heal().state(), DeviceState::Silent);
        assert!(!engine.heal().is_live());
        assert!(engine.heal().notice().contains("degraded"));
        assert_eq!(engine.heal().attempts(), 1);

        // Down: the mixer keeps advancing but the wire is silent, and every
        // silent frame is counted.
        let mut out = [0.0f32; 64];
        engine.render_stereo_into(&mut out);
        assert!(
            out.iter().all(|&s| s == 0.0),
            "silent while the device is down"
        );
        assert_eq!(engine.heal().silent_frames(), 32);

        // A verified reopen brings the wire back.
        let (_, report) = engine
            .try_reopen(20, || Result::<&str, &str>::Ok("stream"))
            .expect("verified reopen must recover");
        assert!(report.message.contains("recovered"));
        assert!(engine.heal().is_live());
        tx.play(1, "flat", 1.0, None).expect("room");
        let mut out = [0.0f32; 64];
        engine.render_stereo_into(&mut out);
        assert!(out.iter().any(|&s| s > 0.0), "live again");
    }

    #[test]
    fn a_device_change_reports_into_the_same_heal() {
        let (_, mut engine) = AudioEngine::with_feed(test_bank(), 16, RATE, 1);
        let decision = engine.report_change(7).expect("change must heal");
        assert_eq!(decision.attempt, 1);
        assert!(engine.heal().notice().contains("changed"));
        assert!(!engine.heal().is_live());
    }
}
