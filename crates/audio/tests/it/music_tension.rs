//! Music tension playlists and stingers through the public feed: the game
//! sends tension levels and stinger names like cues; the bed switches on bar
//! lines and replays byte for byte. Public API only.

use std::sync::Arc;

use engine_audio::music::Tension;
use engine_audio::{cue_names, AudioEngine, CueBank, FrameSource, MusicBank};

const RATE: u32 = 8_000;

fn bank(music_seed: u64) -> Arc<CueBank> {
    let music = MusicBank::procedural(music_seed, RATE).expect("music bank builds");
    let cues = CueBank::placeholder(RATE).expect("shipped cue table");
    Arc::new(cues.with_music(music))
}

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

/// Run the scripted fight and return the output bits.
fn fight(seed: u64, music_seed: u64) -> Vec<u32> {
    let bank = bank(music_seed);
    let bar = bank.music().expect("music attached").bar_frames();
    let (mut tx, mut engine) = AudioEngine::with_feed(bank, 256, RATE, seed);
    let script = [
        (0, Tension::Calm),
        (bar, Tension::Build),
        (bar * 2 + bar / 3, Tension::Fight),
        (bar * 4, Tension::Climax),
        (bar * 6 + 11, Tension::Calm),
    ];
    let mut out = Vec::new();
    let mut at = 0;
    for (frame, level) in script {
        out.extend(render(&mut engine, frame - at, 480));
        at = frame;
        tx.tension(u32::try_from(frame).unwrap_or(0), level)
            .expect("room");
        if level == Tension::Climax {
            tx.stinger(0, cue_names::STINGER_ROAR).expect("a stinger");
        }
        if level == Tension::Fight {
            tx.stinger(0, cue_names::STINGER_KILL).expect("a stinger");
        }
    }
    out.extend(render(&mut engine, bar * 9 - at, 480));
    assert_eq!(tx.stats().unknown_cue, 0);
    assert_eq!(tx.stats().dropped_full, 0);
    out.into_iter().map(f32::to_bits).collect()
}

#[test]
fn the_same_seeds_and_tension_sequence_replay_byte_identical() {
    let first = fight(2040, 7);
    assert_eq!(first, fight(2040, 7), "run twice, same bytes");
    assert!(
        first.iter().any(|&b| f32::from_bits(b) != 0.0),
        "music was playing"
    );
    assert_ne!(
        first,
        fight(2041, 7),
        "another engine seed picks other loops"
    );
    assert_ne!(
        first,
        fight(2040, 8),
        "another music seed writes other music"
    );
}

#[test]
fn bed_follows_the_levels_it_is_sent_one_bar_late_at_most() {
    let bank = bank(7);
    let bar = bank.music().expect("music attached").bar_frames();
    let (mut tx, mut engine) = AudioEngine::with_feed(bank, 16, RATE, 3);
    tx.tension(0, Tension::Fight).expect("room");
    render(&mut engine, bar / 4, 480);
    assert_eq!(
        engine.music_state().expect("music").level,
        Some(Tension::Fight)
    );
    tx.tension(1, Tension::Climax).expect("room");
    render(&mut engine, bar, 480);
    assert_eq!(
        engine.music_state().expect("music").level,
        Some(Tension::Climax),
        "applied by the next bar line, at most one bar after the request"
    );
}
