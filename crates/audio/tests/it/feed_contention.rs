//! The game thread feeds the audio engine while the audio side drains it:
//! nothing may be lost and order must hold. Public API only.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use engine_audio::feed::feed;
use engine_audio::{
    cue_names, AudioCommand, AudioEngine, Command, CueBank, CueId, FeedError, FeedTx, FrameSource,
};

const COMMANDS: u32 = 1_000;
const RATE: u32 = 48_000;

fn bank() -> Arc<CueBank> {
    Arc::new(CueBank::placeholder(RATE).expect("shipped cue table is valid"))
}

/// Run `op` until the ring has room; any other error is a test failure.
fn retry(tx: &mut FeedTx, mut op: impl FnMut(&mut FeedTx) -> Result<(), FeedError>) {
    while let Err(err) = op(tx) {
        assert_eq!(err, FeedError::Full, "audio side stayed open");
        thread::yield_now();
    }
}

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(30)
}

#[test]
fn a_thousand_commands_cross_a_small_ring_in_order_with_nothing_lost() {
    // 64 slots for 1000 commands: the producer outruns the consumer, hits a
    // full ring and retries; every refusal is counted, none is a loss.
    let (mut tx, mut rx) = feed(64, bank());
    let producer = thread::spawn(move || {
        let mut refused = 0u64;
        for tick in 0..COMMANDS {
            let cmd = Command::Play(AudioCommand {
                tick,
                cue: CueId(1),
                gain: 1.0,
                pos_cm: Some([i32::try_from(tick).unwrap_or(0), 0, -5]),
            });
            while let Err(err) = tx.send(cmd) {
                assert_eq!(err, FeedError::Full);
                refused += 1;
                thread::yield_now();
            }
        }
        (tx, refused)
    });
    let limit = deadline();
    let mut seen = Vec::new();
    while seen.len() < usize::try_from(COMMANDS).unwrap_or(0) {
        match rx.pop() {
            Some(Command::Play(play)) => {
                assert_eq!(
                    play.pos_cm,
                    Some([i32::try_from(play.tick).unwrap_or(0), 0, -5])
                );
                seen.push(play.tick);
            }
            Some(other) => panic!("only plays were sent, got {other:?}"),
            None => {
                assert!(
                    Instant::now() < limit,
                    "consumer starved at {} commands",
                    seen.len()
                );
                thread::yield_now();
            }
        }
    }
    let (tx, refused) = producer.join().expect("producer thread ran to the end");
    let want: Vec<u32> = (0..COMMANDS).collect();
    assert_eq!(seen, want, "every command arrived, in send order");
    let stats = tx.stats();
    assert_eq!(stats.sent, u64::from(COMMANDS));
    assert_eq!(stats.dropped_full, refused, "each refusal was counted");
    assert!(rx.pop().is_none());
}

#[test]
fn engine_applies_every_command_while_the_game_thread_keeps_sending() {
    let (mut tx, mut engine) = AudioEngine::with_feed(bank(), 64, RATE, 2040);
    let producer = thread::spawn(move || {
        let mut sent = 0u64;
        for tick in 0..COMMANDS {
            let t = i32::try_from(tick).unwrap_or(0);
            retry(&mut tx, |tx| tx.listener(tick, [t, 300, 800], t % 360));
            retry(&mut tx, |tx| {
                tx.play(tick, cue_names::HIT, 1.0, Some([t, 100, 0]))
            });
            sent += 2;
        }
        (tx, sent)
    });
    // The audio side: render 256-frame buffers like the device callback.
    let limit = deadline();
    let mut buffer = vec![0.0f32; 512];
    let mut heard = false;
    while engine.applied() < u64::from(COMMANDS) * 2 {
        engine.render_stereo_into(&mut buffer);
        heard |= buffer.iter().any(|&s| s != 0.0);
        assert!(
            Instant::now() < limit,
            "engine applied only {}",
            engine.applied()
        );
        thread::yield_now();
    }
    let (tx, sent) = producer.join().expect("producer thread ran to the end");
    engine.render_stereo_into(&mut buffer);
    let stats = tx.stats();
    assert_eq!((stats.sent, stats.applied), (sent, sent));
    assert_eq!(
        stats.last_applied_tick,
        COMMANDS - 1,
        "stamps arrived in order"
    );
    let mix = engine.mixer().mixer_stats();
    assert_eq!(
        mix.started + mix.rejected + engine.mixer().culled(),
        u64::from(COMMANDS),
        "every play was started, refused or out of range; none vanished"
    );
    assert!(heard, "the hits were audible");
    assert_eq!(
        engine.mixer().listener_3d()[0],
        i32::try_from(COMMANDS - 1).unwrap_or(0)
    );
}
