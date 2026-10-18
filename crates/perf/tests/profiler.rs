//! Profiler proofs: off-by-default with zero overhead, on-records with
//! unperturbed hashes, typed failures, and a structured frame capture.
//! Flag-toggling tests serialize on one mutex, so the parallel harness never
//! observes a half-flipped switch. (Unit tests for pure helpers live in
//! `src/profiler.rs`; this binary owns every test that touches the flag.)

use engine_perf::alloc::measure;
use engine_perf::baselines::{arena_baseline, baseline};
use engine_perf::budget::check_report;
use engine_perf::profiler::{
    self, GpuPass, GpuSample, MarkSeries, PlotSeries, ProfileError, Quantiles, Sample,
    CAPTURE_FRAMES_MAX, MARK_EVERY, MARK_FRAME, MARK_TICK, SPAN_AI_DECIDE, SYSTEM_SPANS,
    TICK_FRAME_TICKS,
};
use engine_perf::workloads::{AiDecide, ArenaTick, DestructionStep, PhysicsStep};
use std::sync::Mutex;

static SERIAL: Mutex<()> = Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().expect("profiler test mutex poisons never")
}

#[test]
fn profiler_is_off_by_default() {
    let _guard = lock();
    profiler::set_enabled(false);
    assert!(!profiler::is_enabled());
}

#[test]
fn disabled_profiler_has_zero_overhead() {
    let _guard = lock();
    profiler::set_enabled(false);
    // Same hash through the wrapper as straight through the workload, and
    // the wrapper's sample slot stays empty: no clock, no scope, no record.
    let (hash, sample) = profiler::profiled_ai_decide();
    assert_eq!(hash, AiDecide::canonical().run_hash());
    assert_eq!(sample, None);
    // Same allocations too: the disabled wrapper adds no counting work of
    // its own beyond the workload's own single allocation.
    let (_, raw_alloc) = measure(|| AiDecide::canonical().run_hash());
    let (_, wrapped_alloc) = measure(profiler::profiled_ai_decide);
    assert_eq!(raw_alloc, wrapped_alloc);
}

#[test]
fn enabled_profiler_records_without_perturbing_hashes() {
    let _guard = lock();
    profiler::set_enabled(true);
    let recorded = baseline(AiDecide::NAME).expect("ai baseline recorded");
    let workload = AiDecide::canonical();
    let (hash, sample) = profiler::profiled_ai_decide();
    profiler::set_enabled(false);
    assert_eq!(hash, recorded.hash, "profiling never enters the hash");
    let sample = sample.expect("enabled profiler records a sample");
    assert_eq!(sample.span, SPAN_AI_DECIDE);
    assert!(sample.ms.is_finite() && sample.ms >= 0.0);
    // The profiled hash still passes the budget gate byte-for-byte.
    let report = workload.run_report();
    check_report(&report, &recorded).expect("profiled workload passes its gate");
}

#[test]
fn all_profiled_workloads_cover_their_spans() {
    let _guard = lock();
    profiler::set_enabled(true);
    let rows = [
        (ArenaTick::NAME, profiler::profiled_arena_tick()),
        (PhysicsStep::NAME, profiler::profiled_physics_step()),
        (AiDecide::NAME, profiler::profiled_ai_decide()),
        (DestructionStep::NAME, profiler::profiled_destruction_step()),
    ];
    profiler::set_enabled(false);
    assert_eq!(rows.len(), SYSTEM_SPANS.len());
    for (name, (hash, sample)) in rows {
        let expected = if name == ArenaTick::NAME {
            engine_perf::baselines::arena_baseline().hash
        } else {
            baseline(name).expect("baseline recorded").hash
        };
        assert_eq!(hash, expected, "{name} profiled hash matches its baseline");
        let sample = sample.expect("enabled profiler records every workload");
        assert!(SYSTEM_SPANS.contains(&sample.span), "span {}", sample.span);
        assert!(sample.ms.is_finite() && sample.ms >= 0.0);
    }
}

#[test]
fn profiler_failures_are_typed() {
    let _guard = lock();
    profiler::set_enabled(false);
    assert_eq!(profiler::capture(5), Err(ProfileError::Disabled));
    profiler::set_enabled(true);
    let bad_frames = profiler::capture(0);
    let over_frames = profiler::capture(CAPTURE_FRAMES_MAX + 1);
    profiler::set_enabled(false);
    assert_eq!(bad_frames, Err(ProfileError::InvalidFrames { frames: 0 }));
    assert_eq!(
        over_frames,
        Err(ProfileError::InvalidFrames {
            frames: CAPTURE_FRAMES_MAX + 1
        })
    );
    assert!(matches!(
        profiler::system_span("r9.fake"),
        Err(ProfileError::UnknownSpan { .. })
    ));
    assert!(matches!(
        GpuPass::from_name("r9.fake"),
        Err(ProfileError::UnknownPass { .. })
    ));
    assert!(matches!(
        Quantiles::of("probe", &[]),
        Err(ProfileError::EmptySamples { .. })
    ));
    assert!(matches!(
        Sample::new(SPAN_AI_DECIDE, f64::NAN),
        Err(ProfileError::NonFiniteSample { .. })
    ));
    assert!(matches!(
        GpuSample::cpu_only(GpuPass::Rl2Ibl, -1.0),
        Err(ProfileError::NonFiniteSample { .. })
    ));
}

#[test]
fn frame_capture_names_costliest_system_and_pass() {
    let _guard = lock();
    profiler::set_enabled(true);
    let report = profiler::capture(5).expect("five fixed frames capture");
    profiler::set_enabled(false);
    assert_eq!(report.frames, 5);
    assert_eq!(report.systems.len(), SYSTEM_SPANS.len());
    assert_eq!(report.passes.len(), GpuPass::ALL.len());
    for row in report.systems.iter() {
        assert!(SYSTEM_SPANS.contains(&row.span));
        assert!(row.quantiles.p50_ms <= row.quantiles.p95_ms);
        assert!(row.quantiles.p95_ms <= row.quantiles.p99_ms);
    }
    for row in &report.passes {
        assert!(GpuPass::ALL.contains(&row.pass));
        assert!(row.quantiles.p50_ms <= row.quantiles.p95_ms);
        assert!(row.quantiles.p95_ms <= row.quantiles.p99_ms);
    }
    // The verdicts equal the argmax over p50 (ties keep report order).
    let top_system = report
        .systems
        .iter()
        .max_by(|a, b| {
            a.quantiles
                .p50_ms
                .partial_cmp(&b.quantiles.p50_ms)
                .expect("finite")
        })
        .expect("system rows exist");
    let top_pass = report
        .passes
        .iter()
        .max_by(|a, b| {
            a.quantiles
                .p50_ms
                .partial_cmp(&b.quantiles.p50_ms)
                .expect("finite")
        })
        .expect("pass rows exist");
    assert_eq!(report.costliest_system, top_system.span);
    assert_eq!(report.costliest_system_p50_ms, top_system.quantiles.p50_ms);
    assert_eq!(report.costliest_pass, top_pass.pass.name());
    assert_eq!(report.costliest_pass_p50_ms, top_pass.quantiles.p50_ms);
    // GPU numbers stay operator-side: every sample leaves gpu_ms empty.
    for pass in GpuPass::ALL {
        let sample = GpuSample::cpu_only(*pass, 0.25).expect("probe ms builds");
        assert_eq!(
            sample.gpu_ms,
            None,
            "{} waits for the reference GPU",
            pass.name()
        );
    }
    // The summary names both winners in stable units for the perf report.
    let summary = report.summary();
    assert!(summary.contains("frame_capture frames=5"));
    // The frame-time series fed by capture prints its row and smoothed ms.
    assert!(summary.contains("frame_time value="));
    assert!(summary.contains(&format!("smoothed={:.3}ms", report.frame_time_smoothed_ms)));
    assert!(summary.contains(report.costliest_system));
    assert!(summary.contains(report.costliest_pass));
    assert!(summary.contains("costliest_system"));
    assert!(summary.contains("costliest_pass"));
    assert!(summary.contains("ms"));
    assert!(summary.contains("gpu_ms=operator-side"));
}

#[test]
fn conditional_gate_pairs_with_plain_workload() {
    let _guard = lock();
    // Gate-off identical to plain: same hash, no sample either way.
    profiler::set_enabled(false);
    let (gated_off, gated_off_sample) = profiler::profiled_physics_step_if(true);
    let (plain_off, plain_off_sample) = profiler::profiled_physics_step();
    assert_eq!(gated_off, plain_off);
    assert_eq!(gated_off, PhysicsStep::canonical().run_hash());
    assert_eq!(gated_off_sample, None);
    assert_eq!(plain_off_sample, None);
    // Gate-on measures the same workload: same hash, same span, finite ms.
    profiler::set_enabled(true);
    let (gated_on, gated_on_sample) = profiler::profiled_physics_step_if(true);
    let (plain_on, plain_on_sample) = profiler::profiled_physics_step();
    profiler::set_enabled(false);
    assert_eq!(gated_on, plain_on);
    let gated_sample = gated_on_sample.expect("gate-on records a sample");
    let plain_sample = plain_on_sample.expect("plain records while enabled");
    assert_eq!(gated_sample.span, profiler::SPAN_PHYSICS_STEP);
    assert_eq!(plain_sample.span, profiler::SPAN_PHYSICS_STEP);
    assert!(gated_sample.ms.is_finite() && gated_sample.ms >= 0.0);
    // Per-call false while on records nothing but keeps the hash.
    profiler::set_enabled(true);
    let (skipped, skipped_sample) = profiler::profiled_physics_step_if(false);
    profiler::set_enabled(false);
    assert_eq!(skipped, PhysicsStep::canonical().run_hash());
    assert_eq!(skipped_sample, None);
}

#[test]
fn engine_hash_unperturbed_x2_with_profiler_off() {
    // Determinism rule for gated profiling: the canonical arena hash repeats
    // exactly with the profiler gated off and matches its baseline row.
    let _guard = lock();
    profiler::set_enabled(false);
    let first = ArenaTick::canonical().run_hash();
    let second = ArenaTick::canonical().run_hash();
    assert_eq!(first, second, "engine hash repeats with profiler off");
    assert_eq!(
        first,
        arena_baseline().hash,
        "off-gate hash matches the recorded baseline"
    );
    let report = ArenaTick::canonical().run_report();
    check_report(&report, &arena_baseline()).expect("off-gate run passes its gate");
}

#[test]
fn tick_frame_capture_refuses_bad_input() {
    let _guard = lock();
    profiler::set_enabled(false);
    assert_eq!(
        profiler::capture_tick_frame(16),
        Err(ProfileError::Disabled),
        "off profiler refuses the tick-frame capture"
    );
    profiler::set_enabled(true);
    let empty = profiler::capture_tick_frame(0);
    let over = profiler::capture_tick_frame(TICK_FRAME_TICKS + 1);
    profiler::set_enabled(false);
    assert_eq!(empty, Err(ProfileError::InvalidTicks { ticks: 0 }));
    assert_eq!(
        over,
        Err(ProfileError::InvalidTicks {
            ticks: TICK_FRAME_TICKS + 1
        })
    );
    assert!(matches!(
        MarkSeries::new(0),
        Err(ProfileError::InvalidTicks { .. })
    ));
    assert!(matches!(
        PlotSeries::new("r9.fake"),
        Err(ProfileError::UnknownMark { .. })
    ));
}

#[test]
fn tick_frame_small_capture_reads_clean() {
    // Shape proof at 16 ticks (fast leg): both plots fill, secondary marks
    // land every 8th tick on both series, and the stored hash equals the
    // off-gate canonical hash, so profiling never perturbs it.
    let _guard = lock();
    profiler::set_enabled(true);
    let capture = profiler::capture_tick_frame(16).expect("16 fixed ticks capture");
    profiler::set_enabled(false);
    assert_eq!(capture.ticks, 16);
    assert_eq!(capture.tick_plot.len(), 16);
    assert_eq!(capture.frame_plot.len(), 16);
    assert_eq!(capture.marks.len(), 2 * (16 / MARK_EVERY as usize));
    assert_eq!(
        capture.marks.marks_for(MARK_TICK).len(),
        16 / MARK_EVERY as usize
    );
    assert_eq!(
        capture.marks.marks_for(MARK_FRAME).len(),
        16 / MARK_EVERY as usize
    );
    for row in [&capture.tick_plot, &capture.frame_plot] {
        let quantiles = row.quantiles().expect("series ranks");
        assert!(quantiles.p50_ms <= quantiles.p95_ms);
        assert!(quantiles.p95_ms <= quantiles.p99_ms);
    }
    assert_eq!(
        capture.hash,
        ArenaTick::canonical().run_hash(),
        "capture hash equals the off-gate hash"
    );
    let summary = capture.try_summary().expect("16-tick capture summarizes");
    assert!(summary.contains("tick_frame ticks=16"));
    assert!(summary.contains("tick p50="));
    assert!(summary.contains("frame p50="));
    assert!(summary.contains("marks=4"));
    let marks_text = capture.marks.summary();
    assert!(marks_text.contains("tick_frame_marks total=16 marks=4"));
    assert!(marks_text.contains("label=tick"));
    assert!(marks_text.contains("label=frame"));
    let plot = capture.plot_text(4).expect("four buckets render");
    assert!(plot.contains("plot buckets=4 ticks=16"));
    assert!(plot.contains("bucket 00"));
    assert!(plot.contains("tick_p50="));
    assert!(plot.contains("frame_p50="));
}

#[test]
#[ignore = "determinism receipt: 600-tick capture, quantile ordering and off-gate hash equality; run under --perf"]
fn tick_frame_600_capture_reads_clean() {
    // Full proof at the canonical tick count: 600 tick-vs-frame p50s plus
    // the bucketed plot read clean, with the canonical hash attached.
    let _guard = lock();
    profiler::set_enabled(true);
    let capture = profiler::capture_tick_frame(TICK_FRAME_TICKS).expect("600 ticks capture");
    profiler::set_enabled(false);
    assert_eq!(capture.ticks, TICK_FRAME_TICKS);
    assert_eq!(capture.tick_plot.len(), TICK_FRAME_TICKS as usize);
    assert_eq!(capture.frame_plot.len(), TICK_FRAME_TICKS as usize);
    assert_eq!(
        capture.marks.len(),
        2 * (TICK_FRAME_TICKS / MARK_EVERY) as usize
    );
    let tick_q = capture.tick_plot.quantiles().expect("tick ranks");
    let frame_q = capture.frame_plot.quantiles().expect("frame ranks");
    assert!(tick_q.p50_ms <= tick_q.p95_ms && tick_q.p95_ms <= tick_q.p99_ms);
    assert!(frame_q.p50_ms <= frame_q.p95_ms && frame_q.p95_ms <= frame_q.p99_ms);
    assert_eq!(
        capture.hash,
        ArenaTick::canonical().run_hash(),
        "600-tick hash equals the off-gate hash"
    );
    let summary = capture.try_summary().expect("600-tick capture summarizes");
    assert!(summary.contains("tick_frame ticks=600"));
    assert!(summary.contains("tick p50=") && summary.contains("frame p50="));
    let plot = capture.plot_text(60).expect("sixty buckets render");
    assert!(plot.contains("plot buckets=60 ticks=600"));
    assert!(plot.contains("bucket 59"));
    println!("{summary}{plot}marks={}\n", capture.marks.len());
}
