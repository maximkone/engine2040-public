//! playlist: the music bed, driven by an integer tension level.
//!
//! A [`MusicBank`] holds a short playlist of loops per [`Tension`] level
//! (calm, build, fight, climax). With no recorded music the bank is
//! generated at load from the seeded [`music`](crate::music) bake: every loop
//! shares one tempo and key, and level `n` plays layers `0..=n`, so any two
//! loops line up bar for bar. A [`MusicBed`] on the audio thread plays one
//! loop at a time. A tension change waits for the next bar line and then
//! crossfades over one bar: the new level's loop joins at the same bar of
//! the pattern while the old one fades out. Entering a level picks one of its
//! loops with a seeded picker that never repeats the previous pick, so the
//! same seed and tension sequence replay the same music. Stingers are
//! ordinary cues that duck the bed (see [`Group::Stinger`](crate::Group)).
//! Gregory, Game Engine Architecture 13.6.3 (tension playlists, stingers,
//! transitions on bar boundaries).

use std::sync::Arc;

use engine_core::splitmix64;

use crate::cue::{Clip, CueId, Group, VariantPicker, MAX_CLIP_FRAMES};
use crate::dsp::Bus;
use crate::music::{bake, pcm_len, render_pcm_into, Song, Tension};
use crate::{Mixer, VoiceId, VoiceRoute, VoiceStart};

/// Bars in one generated loop.
pub const LOOP_BARS: u8 = 2;
/// Loops per tension level in a generated bank.
pub const PLAYLIST_ENTRIES: usize = 2;
/// Voice trim for the bed (the music bus already sits 12 dB down).
const MUSIC_TRIM: f32 = 1.5;
/// Cue id the bed's voices carry; no [`CueBank`](crate::CueBank) hands it out.
const MUSIC_CUE: CueId = CueId(u16::MAX);

/// Why a [`MusicBank`] could not be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MusicError {
    /// The sample rate was zero.
    ZeroRate,
    /// A loop would pass the clip frame cap at this rate.
    TooLong,
}

impl std::fmt::Display for MusicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroRate => write!(f, "music: zero sample rate"),
            Self::TooLong => write!(f, "music: loop exceeds the clip frame cap"),
        }
    }
}

impl std::error::Error for MusicError {}

/// Loops per tension level, all the same length and tempo.
#[derive(Clone, Debug)]
pub struct MusicBank {
    sample_rate: u32,
    bpm: u16,
    bar_frames: usize,
    levels: [Vec<Clip>; Tension::COUNT],
}

impl MusicBank {
    /// Generate [`PLAYLIST_ENTRIES`] loops per level from `seed` at
    /// `sample_rate`: [`LOOP_BARS`] bars each, mono, a few hundred KiB apiece.
    /// Entry `e` of every level is the same arrangement with more layers at
    /// higher levels; entries differ in their lead line. Deterministic per
    /// `(seed, sample_rate)`.
    ///
    /// # Errors
    ///
    /// [`MusicError::ZeroRate`] for rate 0, [`MusicError::TooLong`] when a
    /// loop would exceed [`MAX_CLIP_FRAMES`].
    pub fn procedural(seed: u64, sample_rate: u32) -> Result<Self, MusicError> {
        if sample_rate == 0 {
            return Err(MusicError::ZeroRate);
        }
        let master = bake(seed, LOOP_BARS);
        let mut levels: [Vec<Clip>; Tension::COUNT] = Default::default();
        for entry in 0..PLAYLIST_ENTRIES {
            let mut song = bake(
                splitmix64(seed ^ u64::try_from(entry).unwrap_or(0)),
                LOOP_BARS,
            );
            // One tempo and key for every loop, so any pair crossfades in time.
            let shift = i16::from(master.root) - i16::from(song.root);
            song.bpm = master.bpm;
            song.root = master.root;
            for note in song.notes.iter_mut().filter(|n| n.layer != 2) {
                let moved = (i16::from(note.midi) + shift).clamp(21, 108);
                note.midi = u8::try_from(moved).unwrap_or(note.midi);
            }
            if entry > 0 {
                vary(&mut song);
            }
            let len = pcm_len(&song, sample_rate, MAX_CLIP_FRAMES + 1);
            if len > MAX_CLIP_FRAMES {
                return Err(MusicError::TooLong);
            }
            for level in Tension::ALL {
                let mut pcm = vec![0i16; len];
                render_pcm_into(&song, level.intensity(), sample_rate, &mut pcm);
                let samples: Vec<f32> = pcm.iter().map(|&v| f32::from(v) / 32_768.0).collect();
                levels[level.index()].push(Clip {
                    sample_rate,
                    channels: 1,
                    samples: Arc::from(samples),
                });
            }
        }
        let frames = levels[0].first().map_or(0, Clip::frames);
        let bar_frames = frames / usize::from(LOOP_BARS);
        if bar_frames == 0 {
            return Err(MusicError::ZeroRate);
        }
        Ok(Self {
            sample_rate,
            bpm: master.bpm,
            bar_frames,
            levels,
        })
    }

    /// Output rate the loops were generated at.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Tempo shared by every loop.
    pub fn bpm(&self) -> u16 {
        self.bpm
    }

    /// Frames in one bar: where the bed changes level.
    pub fn bar_frames(&self) -> usize {
        self.bar_frames
    }

    /// The playlist for `level`.
    pub fn entries(&self, level: Tension) -> &[Clip] {
        &self.levels[level.index()]
    }
}

/// Rework `song` into the alternate playlist entry without leaving its
/// harmony: pad an octave up, bass on the offbeats. Every layer below the
/// lead changes, so the entries differ at every tension level.
fn vary(song: &mut Song) {
    for note in song.notes.iter_mut() {
        match note.layer {
            0 => note.midi = note.midi.saturating_add(12).min(108),
            1 => note.step += 2,
            _ => {}
        }
    }
}

/// What the bed is doing, for tests and diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MusicState {
    /// Level playing now (`None` before the first tension command).
    pub level: Option<Tension>,
    /// Level that takes over at the next bar line, if one was requested.
    pub pending: Option<Tension>,
    /// Frames into the current bar.
    pub bar_pos: usize,
    /// Bar within the loop pattern, `0..LOOP_BARS`.
    pub bar_in_loop: u8,
    /// Playlist entry playing.
    pub entry: usize,
}

/// The music bed: one looping voice, a bar clock and a pending level.
pub struct MusicBed {
    bank: Arc<MusicBank>,
    pickers: [VariantPicker; Tension::COUNT],
    level: Option<Tension>,
    pending: Option<Tension>,
    voice: Option<VoiceId>,
    entry: usize,
    bar_pos: usize,
    bar_in_loop: u8,
}

impl MusicBed {
    /// A silent bed over `bank`; `seed` fixes which loop each level enters
    /// with.
    pub fn new(bank: Arc<MusicBank>, seed: u64) -> Self {
        Self {
            bank,
            pickers: std::array::from_fn(|idx| {
                VariantPicker::for_stream(seed, 0x6d75_7369_6300 + u64::try_from(idx).unwrap_or(0))
            }),
            level: None,
            pending: None,
            voice: None,
            entry: 0,
            bar_pos: 0,
            bar_in_loop: 0,
        }
    }

    /// Ask for `level`. A silent bed starts it now, fading in over a bar;
    /// otherwise it takes over at the next bar line (a later request before
    /// that line replaces this one, asking for the playing level cancels).
    pub fn request(&mut self, level: Tension, mixer: &mut Mixer) {
        match self.level {
            None => self.enter(level, 0, mixer),
            Some(now) if now == level => self.pending = None,
            Some(_) => self.pending = Some(level),
        }
    }

    /// Frames until the next bar line (`usize::MAX` while silent): the
    /// engine splits its render here so the switch lands on the exact frame.
    pub fn frames_to_boundary(&self) -> usize {
        match self.level {
            Some(_) => self.bank.bar_frames.saturating_sub(self.bar_pos).max(1),
            None => usize::MAX,
        }
    }

    /// Account for `frames` rendered; on reaching a bar line, apply a pending
    /// level.
    pub fn advance(&mut self, frames: usize, mixer: &mut Mixer) {
        if self.level.is_none() {
            return;
        }
        self.bar_pos += frames;
        if self.bar_pos >= self.bank.bar_frames {
            self.bar_pos = 0;
            self.bar_in_loop = (self.bar_in_loop + 1) % LOOP_BARS;
            if let Some(next) = self.pending.take() {
                self.enter(next, self.bar_offset(), mixer);
            }
        }
    }

    /// Where the bed is.
    pub fn state(&self) -> MusicState {
        MusicState {
            level: self.level,
            pending: self.pending,
            bar_pos: self.bar_pos,
            bar_in_loop: self.bar_in_loop,
            entry: self.entry,
        }
    }

    /// Frame offset of the current bar inside a loop.
    fn bar_offset(&self) -> usize {
        usize::from(self.bar_in_loop) * self.bank.bar_frames
    }

    /// Start `level`'s next loop at `offset` frames, ramping in over one bar,
    /// and fade the previous loop out over the same bar.
    fn enter(&mut self, level: Tension, offset: usize, mixer: &mut Mixer) {
        let bank = Arc::clone(&self.bank);
        let entries = bank.entries(level);
        let entry = self.pickers[level.index()].pick(entries.len());
        let Some(clip) = entries.get(entry) else {
            return;
        };
        let route = VoiceRoute {
            cue: MUSIC_CUE,
            bus: Bus::Music,
            group: Group::Music,
            priority: u8::MAX,
            looped: true,
            ducks_music: false,
        };
        let start = VoiceStart {
            offset_frames: offset,
            fade_in_frames: u32::try_from(bank.bar_frames).unwrap_or(u32::MAX),
        };
        let gains = (MUSIC_TRIM, MUSIC_TRIM);
        let Some(voice) = mixer.play_clip_with(clip, route, gains, false, start) else {
            return;
        };
        if let Some(old) = self.voice.replace(voice) {
            mixer.fade_out(old, start.fade_in_frames);
        }
        if self.level.is_none() {
            self.bar_pos = 0;
            self.bar_in_loop = 0;
        }
        self.level = Some(level);
        self.entry = entry;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::rms;

    const RATE: u32 = 8_000;

    fn bank() -> Arc<MusicBank> {
        Arc::new(MusicBank::procedural(2040, RATE).expect("music bank builds"))
    }

    #[test]
    fn generated_playlists_are_aligned_distinct_and_replay() {
        let a = bank();
        let b = MusicBank::procedural(2040, RATE).expect("again");
        assert_eq!(a.sample_rate(), RATE);
        assert!((96..=128).contains(&a.bpm()));
        let frames = a.entries(Tension::Calm)[0].frames();
        assert_eq!(frames, a.bar_frames() * usize::from(LOOP_BARS));
        for level in Tension::ALL {
            let entries = a.entries(level);
            assert_eq!(entries.len(), PLAYLIST_ENTRIES);
            for (entry, twin) in entries.iter().zip(b.entries(level)) {
                assert_eq!(entry.frames(), frames, "every loop is the same length");
                assert_eq!(entry.samples, twin.samples, "same seed, same loops");
                assert!(entry.samples.iter().all(|s| s.abs() <= 1.0));
            }
            assert_ne!(
                entries[0].samples, entries[1].samples,
                "playlist entries differ"
            );
        }
        let other = MusicBank::procedural(2041, RATE).expect("other seed");
        assert_ne!(
            other.entries(Tension::Fight)[0].samples,
            a.entries(Tension::Fight)[0].samples
        );
    }

    #[test]
    fn higher_tension_plays_more_layers() {
        let a = bank();
        let loudness: Vec<f32> = Tension::ALL
            .iter()
            .map(|&t| rms(&a.entries(t)[0].samples))
            .collect();
        assert!(loudness[0] > 0.0, "calm is a pad, not silence");
        assert!(
            loudness.windows(2).all(|w| w[1] > w[0]),
            "each level adds a layer: {loudness:?}"
        );
    }

    #[test]
    fn bad_rates_are_refused() {
        assert_eq!(
            MusicBank::procedural(1, 0).unwrap_err(),
            MusicError::ZeroRate
        );
        assert!(!MusicError::TooLong.to_string().is_empty());
        assert_eq!(MusicError::ZeroRate.to_string(), "music: zero sample rate");
    }

    /// Run `bed` for `frames` against `mixer` the way the engine does.
    fn run(bed: &mut MusicBed, mixer: &mut Mixer, frames: usize) {
        let mut left = frames;
        while left > 0 {
            let step = left.min(bed.frames_to_boundary());
            mixer.render_stereo(step);
            bed.advance(step, mixer);
            left -= step;
        }
    }

    #[test]
    fn a_tension_change_waits_for_the_bar_line_then_crossfades_one_bar() {
        let bank = bank();
        let bar = bank.bar_frames();
        let mut mixer = Mixer::new(RATE);
        let mut bed = MusicBed::new(Arc::clone(&bank), 7);
        assert_eq!(bed.frames_to_boundary(), usize::MAX);
        bed.request(Tension::Calm, &mut mixer);
        run(&mut bed, &mut mixer, bar / 2);
        let s = bed.state();
        assert_eq!(
            (s.level, s.pending, s.bar_pos),
            (Some(Tension::Calm), None, bar / 2)
        );
        assert_eq!(mixer.fading_voices(), 0);

        bed.request(Tension::Fight, &mut mixer);
        run(&mut bed, &mut mixer, bar - bar / 2 - 1);
        let s = bed.state();
        assert_eq!(
            (s.level, s.pending, s.bar_pos),
            (Some(Tension::Calm), Some(Tension::Fight), bar - 1)
        );
        assert_eq!(
            (mixer.voice_count(), mixer.fading_voices()),
            (1, 0),
            "nothing moves mid-bar"
        );

        run(&mut bed, &mut mixer, 1);
        let s = bed.state();
        assert_eq!(
            (s.level, s.pending, s.bar_pos),
            (Some(Tension::Fight), None, 0)
        );
        assert_eq!(s.bar_in_loop, 1);
        assert_eq!(
            mixer.voice_count(),
            2,
            "old loop fading, new loop fading in"
        );
        assert_eq!(mixer.fading_voices(), 1);
        assert_eq!(mixer.group_voices(Group::Music), 1);

        run(&mut bed, &mut mixer, bar / 2);
        assert_eq!(
            mixer.fading_voices(),
            1,
            "the old loop is still fading half a bar on"
        );
        run(&mut bed, &mut mixer, bar - bar / 2);
        assert_eq!(
            (mixer.voice_count(), mixer.fading_voices()),
            (1, 0),
            "gone by the next bar line"
        );
    }

    #[test]
    fn asking_for_the_playing_level_cancels_a_pending_change() {
        let bank = bank();
        let mut mixer = Mixer::new(RATE);
        let mut bed = MusicBed::new(Arc::clone(&bank), 3);
        bed.request(Tension::Build, &mut mixer);
        bed.request(Tension::Climax, &mut mixer);
        bed.request(Tension::Fight, &mut mixer);
        assert_eq!(
            bed.state().pending,
            Some(Tension::Fight),
            "the last request wins"
        );
        bed.request(Tension::Build, &mut mixer);
        assert_eq!(bed.state().pending, None);
        run(&mut bed, &mut mixer, bank.bar_frames() * 3);
        assert_eq!(bed.state().level, Some(Tension::Build));
        assert_eq!(mixer.voice_count(), 1);
    }

    #[test]
    fn the_new_loop_joins_at_the_same_bar_of_the_pattern() {
        let bank = bank();
        let bar = bank.bar_frames();
        let mut mixer = Mixer::new(RATE);
        let mut bed = MusicBed::new(Arc::clone(&bank), 5);
        bed.request(Tension::Calm, &mut mixer);
        run(&mut bed, &mut mixer, bar);
        assert_eq!(bed.state().bar_in_loop, 1);
        bed.request(Tension::Climax, &mut mixer);
        run(&mut bed, &mut mixer, bar);
        // Bar 2 of the pattern is bar 0 again: the new voice wrapped with it.
        assert_eq!(bed.state().bar_in_loop, 0);
        assert_eq!(bed.state().level, Some(Tension::Climax));
        run(&mut bed, &mut mixer, bar * 5 + 17);
        assert_eq!(mixer.voice_count(), 1, "one looping voice carries on");
    }

    #[test]
    fn playlist_picks_replay_per_seed_and_never_repeat_within_a_level() {
        let bank = bank();
        let script = [
            Tension::Fight,
            Tension::Calm,
            Tension::Fight,
            Tension::Calm,
            Tension::Fight,
            Tension::Calm,
            Tension::Fight,
        ];
        let picks = |seed: u64| {
            let mut mixer = Mixer::new(RATE);
            let mut bed = MusicBed::new(Arc::clone(&bank), seed);
            let mut seen = Vec::new();
            for level in script {
                bed.request(level, &mut mixer);
                run(&mut bed, &mut mixer, bank.bar_frames());
                seen.push((level, bed.state().entry));
            }
            seen
        };
        let a = picks(99);
        assert_eq!(a, picks(99), "same seed, same loops");
        for level in [Tension::Fight, Tension::Calm] {
            let entries: Vec<usize> = a
                .iter()
                .filter(|(l, _)| *l == level)
                .map(|&(_, e)| e)
                .collect();
            assert!(
                entries.windows(2).all(|w| w[0] != w[1]),
                "{level:?}: {entries:?}"
            );
        }
        let distinct = (100..110u64).filter(|&seed| picks(seed) != a).count();
        assert!(distinct > 0, "other seeds pick other loops");
    }
}
