use engine_audio::dsp::{
    db_to_linear, gate_open, gate_window_len, peak, render_song, rms, song_hash, Bus, BusMixer,
    GATE_SILENCE, GATE_THRESHOLD,
};
use engine_audio::Mixer;

#[test]
fn sine_master_rms_positive_gate_opens() {
    let m = Mixer::new(48_000);
    let (_, r) = m.render_sine(440.0, 48_000);
    assert!(r > 0.01, "rms={r}");
    let (sfx, mus, dlg) = (vec![0.5; 960], vec![0.25; 960], vec![0.0; 960]);
    let mix = BusMixer::new(48_000);
    let master = mix.mix(&sfx, &mus, &dlg);
    let st = mix.stats(&master);
    assert!(st.rms > GATE_THRESHOLD, "st={st:?}");
    assert!(st.peak > 0.0 && st.gate_open);
    assert!(gate_open(rms(&master), GATE_THRESHOLD));
    assert!((rms(&master) - st.rms).abs() < 1e-6);
}

#[test]
fn silence_stays_shut() {
    let z = vec![0.0; 960];
    assert_eq!((rms(&z), peak(&z)), (0.0, 0.0));
    assert!(rms(&z) < GATE_SILENCE);
    assert!(!gate_open(rms(&z), GATE_THRESHOLD));
    let mix = BusMixer::new(48_000);
    let st = mix.stats(&mix.mix(&z, &z, &z));
    assert!(!st.gate_open && st.rms == 0.0 && st.peak == 0.0);
}

#[test]
fn gains_match_canon_and_duck_music() {
    assert!((db_to_linear(0.0) - 1.0).abs() < 1e-6);
    assert!((Bus::Sfx.gain_db() - engine_audio::GAIN_SFX_DB).abs() < 1e-9);
    assert!((Bus::Music.gain_db() - engine_audio::GAIN_MUSIC_DB).abs() < 1e-9);
    assert!((Bus::Dialogue.gain_db() - engine_audio::GAIN_DIALOGUE_DB).abs() < 1e-9);
    let mix = BusMixer::new(8_000);
    assert!(mix.g_music < mix.g_sfx && mix.g_dlg < mix.g_sfx);
    assert!((mix.render_frame(1.0, 0.0, 0.0) - mix.g_sfx).abs() < 1e-6);
    assert!((mix.render_frame(0.0, 1.0, 0.0) - mix.g_music).abs() < 1e-6);
    assert_eq!(mix.render_frame(9.0, 9.0, 9.0), 1.0);
}

#[test]
fn same_seed_song_bit_identical_twice() {
    let (a, b) = (
        render_song(2040, 8_000, 8_000),
        render_song(2040, 8_000, 8_000),
    );
    assert_eq!(a, b);
    assert_eq!(song_hash(&a), song_hash(&b));
    assert!(rms(&a) > 0.01 && peak(&a) > 0.1);
    assert!(gate_open(rms(&a), GATE_THRESHOLD));
    let m = Mixer::new(8_000);
    assert_eq!(m.song(2040, 8_000), a);
    assert_eq!(m.bus_mixer().mix(&a, &a, &a).len(), a.len());
}

#[test]
fn diff_seed_diff_song_and_window() {
    assert_eq!(gate_window_len(48_000), 960);
    assert_eq!(gate_window_len(8_000), 160);
    let (a, c) = (render_song(1, 8_000, 2_000), render_song(2, 8_000, 2_000));
    assert_ne!(a, c);
    assert_ne!(song_hash(&a), song_hash(&c));
}
