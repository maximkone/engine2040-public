//! Audio presence: every module loads and answers without panic.
//!
//! One probe per module across the whole `engine-audio` crate: construct the
//! headless default, run one pure helper and render a few frames. No device,
//! no files, no wall clock. The count below is 14 headless probes plus 2
//! device-gated probes (`decode`, `device`) for 16 when the `device` feature
//! is on.

// Constant invariants, checked at compile time rather than per test run.
const _: () = assert!(engine_audio::MAX_VOICES > 0);
const _: () = assert!(engine_audio::MAX_GAIN > 0.0);

use engine_audio::bank::{Bank, BankSound, ModalVoice};
use engine_audio::coherence::{cue_kind, gain_permille, voice_for, CoherenceConfig, CoherenceSync};
use engine_audio::crowd::{Crowd, FightCue};
use engine_audio::director::{Director, PlacedCue, TensionTable};
use engine_audio::dsp::{
    adsr, db_to_linear, gate_open, gate_window_len, peak, render_song, rms, song_hash, BeatGrid,
    Bus, BusMixer, OnePole, TrackTween, GATE_THRESHOLD,
};
use engine_audio::heal::DeviceHeal;
use engine_audio::hrtf::{azimuth_for, octant_for, HrtfProcessor};
use engine_audio::music::{
    active_mask, bake, bar_samples, beat_hash, beat_list, pcm_hash, pcm_len, render_pcm_into,
    sample_for_beat, samples_per_beat, Player,
};
use engine_audio::occlude::{
    absorption_q16, apply_floor, apply_wall, q16_to_gain, segments_cross, target_gain_q16,
    transmission_q16, Material, VoiceOcclusion, Wall, GAIN_ONE,
};
use engine_audio::reverb::{
    band_rms, decay_db, level_db, ms_to_samples, rt60_feedback, Reverb, ReverbParams, ReverbZone,
    StereoReverb,
};
use engine_audio::spatial::{attenuation, isqrt, pan_for, Pos2, SpatialMixer};
use engine_audio::voice::{
    bake_line, bank_sha, bank_sha_hex, integrated_lufs, passes_lufs_cap, phoneme_id_for_char,
    sample_line, synthesize_live, VoiceBank, WavEntry,
};
use engine_audio::{clip_sample, rms_gate_open, sanitize_gain, Mixer, Pcm};

#[cfg(feature = "device")]
use engine_audio::{decode_vorbis, decode_wav, MAX_DECODE_BYTES, MAX_PCM_FRAMES};

#[cfg(feature = "device")]
const EXPECTED_PROBES: u32 = 16;
#[cfg(not(feature = "device"))]
const EXPECTED_PROBES: u32 = 14;

fn tone() -> Pcm {
    Pcm::new(8_000, 1, vec![0.25, 0.5, -0.25, -0.5])
}

#[test]
fn presence_all_modules_load_without_panic() {
    let mut loaded: u32 = 0;

    // 1: crate root (Mixer + Pcm + helpers).
    {
        let mut mixer = Mixer::new(8_000);
        let pcm = tone();
        assert_eq!(pcm.frames(), 4);
        let id = mixer.play(&pcm, 1.0);
        assert_ne!(id.to_u64(), 0);
        let out = mixer.render(4);
        assert_eq!(out.len(), 4);
        assert_eq!(clip_sample(f32::NAN), 0.0);
        assert_eq!(sanitize_gain(f32::NAN), 0.0);
        assert!(!rms_gate_open(0.0, 0.001));
        assert_eq!(engine_audio::GAIN_MASTER_DB, 0.0);
        loaded += 1;
    }

    // 2: bank.
    {
        let bank = Bank::placeholder(8_000);
        assert_eq!(bank.sample_rate(), 8_000);
        for sound in [BankSound::Hit, BankSound::Footstep, BankSound::CrowdBed] {
            assert!(!bank.get(sound).is_empty());
        }
        let modal = ModalVoice::bake(8_000);
        assert!(!modal.bar.is_empty());
        assert!(!modal.thump.is_empty());
        let mut scratch = vec![0.0f32; 16];
        modal.render_bar_into(&mut scratch);
        modal.render_thump_into(&mut scratch);
        loaded += 1;
    }

    // 3: coherence.
    {
        let config = CoherenceConfig::coherent();
        config.validate().expect("coherent default is valid");
        let cue = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 100,
        };
        assert_eq!(cue_kind(cue), 0);
        assert!(voice_for(cue).is_some());
        assert_eq!(gain_permille(0), 1_000);
        let mut sync = CoherenceSync::coherent();
        sync.notify_hit(cue);
        sync.notify_audio(cue);
        sync.tick();
        let report = sync.report();
        assert!(report.is_coherent());
        assert_eq!(report.coherence_permille(), 1_000);
        loaded += 1;
    }

    // 4: crowd.
    {
        let mut crowd = Crowd::new();
        let cue = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 130,
        };
        let reaction = crowd.on_fight_event(cue);
        assert_eq!(reaction.level(), 2);
        assert!(!reaction.is_quiet());
        crowd.tick();
        assert!(crowd.bed_gain().is_finite());
        loaded += 1;
    }

    // 5: director.
    {
        let table = TensionTable::scripted();
        table.validate().expect("scripted table is valid");
        let mut director = Director::scripted();
        assert_eq!(director.tick_count(), 0);
        let placed = PlacedCue::new(
            FightCue::Hit {
                attacker: 0,
                victim: 1,
                damage: 100,
            },
            100,
        );
        let layers = director.update(&[placed]);
        assert!((0.0..=1.0).contains(&layers.intensity));
        assert!((0.0..=1.0).contains(&director.intensity()));
        assert_eq!(director.intensity(), layers.intensity);
        assert!(!format!("{:?}", director.state()).is_empty());
        loaded += 1;
    }

    // 6: dsp.
    {
        assert!((db_to_linear(0.0) - 1.0).abs() < 1e-6);
        assert_eq!(gate_window_len(48_000), 960);
        let probe = vec![0.5, -0.5, 0.25, -0.25];
        assert!(rms(&probe) > 0.0);
        assert!(peak(&probe) > 0.0);
        assert!(gate_open(rms(&probe), GATE_THRESHOLD));
        let bus = BusMixer::new(8_000);
        assert!(bus.g_music < bus.g_sfx);
        assert!((Bus::Sfx.gain_db() - engine_audio::GAIN_SFX_DB).abs() < 1e-6);
        assert!(Bus::Music.gain_linear() > 0.0);
        let song = render_song(1, 8_000, 16);
        assert_eq!(song.len(), 16);
        assert_eq!(song_hash(&song), song_hash(&song));
        assert!(adsr(0.05, 0.01, 0.05, 0.8, 0.1, 0.3).is_finite());
        let mut pole = OnePole::new(0.5);
        assert!(pole.step(0.5).is_finite());
        let grid = BeatGrid::new(120.0, 0.0);
        assert!(grid.beat_time(1).is_finite());
        let tween = TrackTween::new(0, 1_000, 48);
        assert_eq!(tween.value_q(), TrackTween::new(0, 1_000, 48).value_q());
        loaded += 1;
    }

    // 7: heal.
    {
        let mut heal = DeviceHeal::new();
        assert!(heal.is_live());
        let mut mixer = Mixer::new(8_000);
        mixer.play(&tone(), 1.0);
        assert_eq!(heal.render_silent_frames(&mut mixer, 4), 0);
        assert!(mixer.is_idle());
        assert!(!heal.notice().is_empty());
        assert_eq!(heal.next_backoff_ms(), 0);
        loaded += 1;
    }

    // 8: hrtf.
    {
        assert_eq!(octant_for(0), 0);
        assert_eq!(azimuth_for(0), Some(0));
        assert_eq!(azimuth_for(8), None);
        let mut proc = HrtfProcessor::new(48_000).expect("48k is a valid rate");
        assert_eq!(proc.octant(), 0);
        proc.set_octant(2).expect("octant 2 is valid");
        assert_eq!(proc.octant(), 2);
        let (left, right) = proc.gains();
        assert!(left.is_finite() && right.is_finite());
        let (itd_l, itd_r) = proc.itd_samples();
        assert_eq!((itd_l, itd_r), proc.itd_samples());
        assert_eq!(proc.sample_rate(), 48_000);
        assert!(!format!("{:?}", proc.mode()).is_empty());
        loaded += 1;
    }

    // 9: music.
    {
        let song = bake(2040, 2);
        assert_eq!(
            engine_audio::music::song_hash(&song),
            engine_audio::music::song_hash(&song)
        );
        assert_eq!(
            engine_audio::music::bake_bytes(&song),
            engine_audio::music::bake_bytes(&song)
        );
        let beats = beat_list(&song, 8_000);
        assert_eq!(beats.len(), usize::from(song.bars) * 4);
        assert!(!beats.is_empty());
        assert_eq!(beat_hash(&song, 8_000), beat_hash(&song, 8_000));
        assert_eq!(samples_per_beat(8_000, 120), 4_000);
        assert_eq!(sample_for_beat(2, 8_000, 120), 8_000);
        assert_eq!(bar_samples(8_000, 120), 16_000);
        assert_eq!(active_mask(0.5), 0b0011);
        let mut player = Player::new(0.2);
        player.request(0.9);
        for gain in player.gains(0.5) {
            assert!((0.0..=1.0).contains(&gain));
        }
        assert_eq!(player.gains(0.5), player.gains(0.5));
        assert!(player.advance_bar());
        let n = pcm_len(&song, 8_000, 64);
        assert!(n <= 64);
        let mut out = vec![0i16; n.max(1)];
        render_pcm_into(&song, 1.0, 8_000, &mut out);
        assert_eq!(pcm_hash(&out), pcm_hash(&out));
        loaded += 1;
    }

    // 10: occlude.
    {
        assert!(absorption_q16(Material::Stone) >= 0);
        assert!(transmission_q16(Material::Stone) >= 0);
        assert!(apply_wall(GAIN_ONE, Material::Stone) <= GAIN_ONE);
        assert!(apply_floor(GAIN_ONE, GAIN_ONE) <= GAIN_ONE);
        assert!((0.0..=1.0).contains(&q16_to_gain(GAIN_ONE)));
        assert!(!segments_cross([0, 0], [1, 0], [0, 1], [1, 1]));
        let walls: &[Wall] = &[];
        let target = target_gain_q16([500, 0], [0, 0], walls).expect("open query must succeed");
        assert_eq!(target, GAIN_ONE);
        let mut voice = VoiceOcclusion::new_open();
        assert_eq!(voice.current_q16(), GAIN_ONE);
        voice.tick(0);
        voice.reset_open();
        assert_eq!(voice.current_gain(), 1.0);
        loaded += 1;
    }

    // 11: reverb.
    {
        let params = ReverbParams::tunnel(48_000).expect("tunnel params are valid");
        params.validate().expect("tunnel validates");
        let zone = ReverbZone::tunnel([0, 0, 0]);
        zone.params(48_000).expect("zone params are valid");
        ms_to_samples(10.0, 48_000).expect("10ms converts");
        rt60_feedback(256, 1_100.0, 48_000).expect("feedback computes");
        assert!(level_db(0.5).is_finite());
        assert!(decay_db(1.0, 0.5).is_finite());
        let block = vec![0.5; 64];
        assert!(band_rms(&block, 48_000, 100.0, 1_000.0) >= 0.0);
        let verb = Reverb::new(&params).expect("verb builds");
        assert!(verb.wet().is_finite());
        assert_eq!(verb.params(), params);
        assert!(verb.pre_delay_samples() <= 48_000);
        let mut stereo = StereoReverb::new(&params).expect("stereo pair builds");
        stereo.reset();
        let mut left = vec![0.0f32; 16];
        let mut right = vec![0.0f32; 16];
        let input = vec![0.5f32; 16];
        stereo
            .process_mono_to_stereo(&input, &mut left, &mut right)
            .expect("stereo render works");
        loaded += 1;
    }

    // 12: spatial.
    {
        let pos = Pos2::new(100, 0);
        assert_eq!(isqrt(16), 4);
        assert!((0.0..=1.0).contains(&attenuation(0)));
        assert!((-1.0..=1.0).contains(&pan_for(0)));
        let mut spatial = SpatialMixer::new(8_000);
        spatial.set_listener(Pos2::new(0, 0), 0);
        spatial.play_at(&tone(), pos, 1.0);
        assert_eq!(spatial.voice_count(), 1);
        let out = spatial.render_stereo(4);
        assert_eq!(out.len(), 8);
        assert_eq!(spatial.commands().len(), 1);
        assert_eq!(spatial.listener(), Pos2::new(0, 0));
        assert_eq!(spatial.yaw_deg(), 0);
        loaded += 1;
    }

    // 13: voice.
    {
        assert!(phoneme_id_for_char(97) <= 39);
        let line = bake_line("hi", "hi there", 7, 2040);
        assert_eq!(line.key, "hi");
        assert!(sample_line(&line, 0).is_some());
        assert!(integrated_lufs(&[0.5, -0.5]).is_finite());
        assert!(passes_lufs_cap(-30.0, -16.0));
        assert!(synthesize_live("hello").is_err());
        let mut bank = VoiceBank::new(48_000);
        bank.push_line(line.clone()).expect("first line inserts");
        assert!(bank.find_line("hi").is_some());
        assert!(bank.find_wav(7).is_none());
        assert!(bank.clip_for("hi").is_ok());
        assert!(bank.clip_for("missing").is_err());
        bank.push_wav(WavEntry {
            id: 7,
            path: "vo/hi.wav".to_owned(),
            content_hash: 1,
            bytes: 4,
            lufs: -20.0,
            sample_rate: 48_000,
        })
        .expect("first wav inserts");
        assert!(bank.find_wav(7).is_some());
        assert!(bank.peak_lufs().is_finite());
        assert!(bank.bank_passes_cap(-16.0));
        assert_eq!(bank_sha(&bank), bank_sha(&bank));
        assert!(!bank_sha_hex(&bank).is_empty());
        loaded += 1;
    }

    // 14: crate-root Pcm edge math (keeps the shared sample type honest).
    {
        let empty = Pcm::new(8_000, 0, vec![]);
        assert_eq!(empty.channels, 1);
        assert!(empty.is_empty());
        assert_eq!(empty.mono_at(99), 0.0);
        assert_eq!(empty.stereo_at(99), (0.0, 0.0));
        loaded += 1;
    }

    #[cfg(feature = "device")]
    {
        // 15: decode (bounded rejection only, no files).
        const _: () = assert!(MAX_DECODE_BYTES > 0);
        const _: () = assert!(MAX_PCM_FRAMES > 0);
        assert!(decode_wav(&[]).is_err());
        assert!(decode_vorbis(&[]).is_err());
        loaded += 1;

        // 16: device (typed errors only, no hardware).
        const _: () = assert!(engine_audio::device::MAX_BACKEND_MSG > 0);
        let err = engine_audio::device::DeviceError::NoDevice;
        assert!(!format!("{err}").is_empty());
        assert!(!format!("{}", engine_audio::decode::DecodeError::Empty).is_empty());
        loaded += 1;
    }

    assert_eq!(
        loaded, EXPECTED_PROBES,
        "presence must touch every module once"
    );
}

#[test]
fn presence_edge_inputs_never_panic() {
    // Empty, zero-rate and non-finite inputs stay silent, never panic.
    let mut mixer = Mixer::new(0);
    let empty = Pcm::new(0, 0, vec![]);
    mixer.play(&empty, f32::NAN);
    assert_eq!(mixer.render(4), vec![0.0; 4]);

    let bank = Bank::placeholder(0);
    assert!(bank.hit.frames() > 0);

    let mut crowd = Crowd::new();
    crowd.on_fight_event(FightCue::StaminaOut { fighter: 0 });
    crowd.tick();

    let mut director = Director::scripted();
    let layers = director.update(&[]);
    assert!((0.0..=1.0).contains(&layers.intensity));

    assert!(HrtfProcessor::new(0).is_err());
    assert!(ReverbParams::tunnel(0).is_err());
    assert!(ms_to_samples(10.0, 0).is_err());
    assert_eq!(azimuth_for(9), None);
    assert!(segments_cross([0, 0], [0, 0], [0, 0], [0, 0]));
    assert_eq!(q16_to_gain(-1), 0.0);
}
