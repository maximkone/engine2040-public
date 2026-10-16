//! Device heal over the public audio API: a simulated device loss
//! recovers with a report, and a stubborn outage quarantines loudly while
//! the mixer keeps rendering throughout. No hardware: openers are scripted
//! closures, so this runs on headless CI.

use engine_audio::{AudioHealError, DeviceHeal, DeviceState, Mixer, Pcm};

fn tone() -> Pcm {
    Pcm::new(8_000, 1, vec![0.25, 0.5, -0.25, -0.5])
}

#[test]
fn simulated_loss_recovers_and_reports_with_mixer_running() {
    let mut heal = DeviceHeal::new();
    let mut mixer = Mixer::new(8_000);
    mixer.play(&tone(), 1.0);

    let decision = heal.report_loss(10).expect("loss must degrade");
    assert_eq!(heal.state(), DeviceState::Silent);
    assert_eq!(decision.backoff_ms, 100);

    // Mixer keeps running while the device is down.
    assert_eq!(heal.render_silent_frames(&mut mixer, 4), 4);
    assert!(mixer.is_idle());

    // One refusal backs off; the reopen recovers with a full report.
    let err = heal
        .try_reopen(11, || Result::<(), &str>::Err("busy"))
        .expect_err("refused reopen backs off");
    assert_eq!(
        err,
        AudioHealError::ReopenFailed {
            backoff_ms: 200,
            attempt: 2
        }
    );
    let (_, report) = heal
        .try_reopen(12, || Result::<&str, &str>::Ok("stream"))
        .expect("reopen must recover");
    assert!(heal.is_live());
    assert_eq!(report.attempts, 2);
    assert_eq!(report.silent_frames, 4);
    assert!(report.message.contains("recovered"));
    assert!(!heal.notice().is_empty());
}

#[test]
fn stubborn_outage_quarantines_loudly_with_notice() {
    let mut heal = DeviceHeal::new();
    heal.report_loss(1).expect("loss must degrade");
    let mut mixer = Mixer::new(8_000);

    // Three refused reopens spend the 3-attempt reconnect budget.
    for tick in 2..=4 {
        mixer.play(&tone(), 1.0);
        heal.render_silent_frames(&mut mixer, 4);
        let _ = heal.try_reopen(tick, || Result::<(), &str>::Err("gone"));
    }
    assert!(heal.is_quarantined());
    assert_eq!(heal.silent_frames(), 12);
    assert!(heal.notice().contains("quarantined"));
    // Even quarantined, the mixer keeps rendering new voices.
    mixer.play(&tone(), 1.0);
    heal.render_silent_frames(&mut mixer, 4);
    assert!(mixer.is_idle());
    assert_eq!(heal.silent_frames(), 16);
}
