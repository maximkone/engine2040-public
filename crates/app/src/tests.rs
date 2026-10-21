//! Lifecycle proofs: phase order, refusal cases, scene load, the verify
//! frame, reverse shutdown, worker deadlines and extraction freshness.
//! Every test fails on a plausible defect in the behavior it pins.

use super::*;
use engine_arena::{ArenaConfig, FighterTape};
use engine_scene::codec::{Scene, CURRENT_VERSION};
use engine_scene::ids::ObjectId;
use engine_scene::transform::{Hierarchy, Transform};
use std::cell::RefCell;
use std::rc::Rc;

type Log = Rc<RefCell<Vec<&'static str>>>;

fn logged(log: &Log, name: &'static str) -> impl FnMut(&mut FrameCtx) + 'static {
    let log = Rc::clone(log);
    move |_| log.borrow_mut().push(name)
}

fn empty_scene() -> Scene {
    Scene {
        version: CURRENT_VERSION,
        nodes: Vec::new(),
    }
}

fn one_node_scene() -> Scene {
    let mut tree = Hierarchy::new();
    let root = ObjectId::new();
    tree.insert(root, ObjectId::NIL, Transform::identity())
        .unwrap();
    Scene::encode(&tree)
}

fn standard_builder(log: &Log) -> AppBuilder {
    App::builder()
        .register(
            "poll",
            SystemKind::Poll,
            Phase::PreUpdate,
            logged(log, "poll"),
        )
        .unwrap()
        .register(
            "sim",
            SystemKind::MatchStep,
            Phase::Update,
            logged(log, "sim"),
        )
        .unwrap()
        .register(
            "extract",
            SystemKind::Extract,
            Phase::PostUpdate,
            logged(log, "extract"),
        )
        .unwrap()
}

#[test]
fn startup_then_frames_run_phases_in_plan_order() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let mut app = standard_builder(&log)
        .register(
            "init",
            SystemKind::Setup,
            Phase::Startup,
            logged(&log, "init"),
        )
        .unwrap()
        .startup()
        .unwrap();
    assert_eq!(*log.borrow(), vec!["init"]);
    app.run_frame(1.0 / 60.0).unwrap();
    assert_eq!(*log.borrow(), vec!["init", "poll", "sim", "extract"]);
}

#[test]
fn startup_systems_run_once_across_frames() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let mut app = standard_builder(&log)
        .register(
            "init",
            SystemKind::Setup,
            Phase::Startup,
            logged(&log, "init"),
        )
        .unwrap()
        .startup()
        .unwrap();
    app.run_frame(1.0 / 60.0).unwrap();
    app.run_frame(1.0 / 60.0).unwrap();
    assert_eq!(
        *log.borrow(),
        vec!["init", "poll", "sim", "extract", "poll", "sim", "extract"]
    );
}

#[test]
fn post_update_extract_runs_before_present_despite_reverse_registration() {
    // The menu renders through the same extraction, so Extract lanes run
    // before Present lanes in PostUpdate whatever order the game used.
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let mut app = App::builder()
        .register(
            "hud",
            SystemKind::Present,
            Phase::PostUpdate,
            logged(&log, "hud"),
        )
        .unwrap()
        .register(
            "extract",
            SystemKind::Extract,
            Phase::PostUpdate,
            logged(&log, "extract"),
        )
        .unwrap()
        .startup()
        .unwrap();
    app.run_frame(1.0 / 60.0).unwrap();
    assert_eq!(*log.borrow(), vec!["extract", "hud"]);
}

#[test]
fn extract_lane_outside_post_update_refuses() {
    let error = App::builder()
        .register("extract", SystemKind::Extract, Phase::Update, |_| {})
        .unwrap_err();
    assert_eq!(
        error,
        AppError::WrongPhase {
            name: "extract",
            got: Phase::Update,
            want: Phase::PostUpdate,
        }
    );
}

#[test]
fn duplicate_registration_refuses_and_keeps_the_first() {
    let error = App::builder()
        .register("poll", SystemKind::Poll, Phase::PreUpdate, |_| {})
        .unwrap()
        .register("poll", SystemKind::Poll, Phase::PreUpdate, |_| {})
        .unwrap_err();
    assert_eq!(error, AppError::Duplicate { name: "poll" });
    assert!(error.to_string().contains("poll"));

    let kept = App::builder()
        .register("poll", SystemKind::Poll, Phase::PreUpdate, |_| {})
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(kept.system_count(), 1);

    let mut app = App::builder().build().unwrap();
    app.register("hud", SystemKind::Present, Phase::PostUpdate, |_| {})
        .unwrap();
    assert_eq!(
        app.register("hud", SystemKind::Present, Phase::PostUpdate, |_| {}),
        Err(AppError::Duplicate { name: "hud" })
    );
    assert_eq!(app.system_count(), 1);
}

#[test]
fn wrong_phase_refuses_with_the_home_phase_named() {
    let error = App::builder()
        .register("hud", SystemKind::Present, Phase::Update, |_| {})
        .unwrap_err();
    assert_eq!(
        error,
        AppError::WrongPhase {
            name: "hud",
            got: Phase::Update,
            want: Phase::PostUpdate,
        }
    );
    let report = error.to_string();
    assert!(report.contains("hud") && report.contains("PostUpdate"));

    let error = App::builder()
        .register("brain", SystemKind::MatchStep, Phase::PostUpdate, |_| {})
        .unwrap_err();
    assert_eq!(
        error,
        AppError::WrongPhase {
            name: "brain",
            got: Phase::PostUpdate,
            want: Phase::Update,
        }
    );
}

#[test]
fn load_scene_reports_nodes_and_startup_keeps_them() {
    let mut app = App::builder().build().unwrap();
    app.load_scene(&one_node_scene()).unwrap();
    assert_eq!(app.scene_nodes(), 1);
    let receipt = app.startup().unwrap();
    assert_eq!(receipt.scene_nodes, 1);
    assert_eq!(receipt.schema_version, CURRENT_VERSION);
}

#[test]
fn bad_scene_version_refuses_and_leaves_the_app_unstarted() {
    let mut app = App::builder().build().unwrap();
    let bad = Scene {
        version: CURRENT_VERSION + 1,
        nodes: Vec::new(),
    };
    let error = app.load_scene(&bad).unwrap_err();
    assert!(matches!(error, AppError::SceneLoad { .. }));
    assert!(error.to_string().contains(&bad.version.to_string()));
    assert!(!app.is_running());
    assert_eq!(app.scene_nodes(), 0);

    app.load_scene(&empty_scene()).unwrap();
    let receipt = app.startup().unwrap();
    assert_eq!(receipt.scene_nodes, 0);
}

#[test]
fn verify_hash_matches_a_direct_arena_run() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let app = standard_builder(&log).startup().unwrap();
    let receipt = app.startup_receipt().unwrap();
    assert_eq!(receipt.verify_ticks, VERIFY_TICKS);
    let direct = engine_arena::run(&verify_config(2040), &FighterTape::default());
    assert_eq!(direct.len() as u32, VERIFY_TICKS);
    let expect = direct.last().map(|checkpoint| checkpoint.hash).unwrap();
    assert_eq!(receipt.verify_hash, expect);
    assert_ne!(receipt.verify_hash, 0);
}

#[test]
fn verify_config_uses_two_fighters_and_an_empty_tape() {
    let config: ArenaConfig = verify_config(2040);
    assert_eq!(config.seed, 2040);
    assert_eq!(config.ticks, VERIFY_TICKS);
    assert!(config.validate().is_ok());
}

#[test]
fn shutdown_reverses_startup_and_seals_the_journal() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let mut app = App::builder()
        .register(
            "first",
            SystemKind::Poll,
            Phase::PreUpdate,
            logged(&log, "first"),
        )
        .unwrap()
        .register(
            "second",
            SystemKind::Poll,
            Phase::PreUpdate,
            logged(&log, "second"),
        )
        .unwrap()
        .register(
            "third",
            SystemKind::Poll,
            Phase::PreUpdate,
            logged(&log, "third"),
        )
        .unwrap()
        .startup()
        .unwrap();
    app.spawn_worker("bake", 10).unwrap();
    assert_eq!(app.system_kind("first"), Some(SystemKind::Poll));
    assert_eq!(app.system_kind("missing"), None);
    assert!(app.accepts_inputs());
    let receipt = app.shutdown().unwrap();
    assert_eq!(receipt.drop_order, vec!["third", "second", "first"]);
    assert_eq!(receipt.workers_drained, 1);
    assert!(!app.accepts_inputs());
    assert!(app.journal().is_sealed());
    assert_eq!(receipt.journal_entries, app.journal().len());

    assert_eq!(
        app.shutdown(),
        Err(AppError::AlreadyShutdown { what: "shutdown" })
    );
    assert_eq!(
        app.run_frame(1.0 / 60.0),
        Err(AppError::AlreadyShutdown { what: "run_frame" })
    );
    assert_eq!(
        app.register("late", SystemKind::Poll, Phase::PreUpdate, |_| {}),
        Err(AppError::AlreadyShutdown { what: "register" })
    );
}

#[test]
fn worker_over_deadline_aborts_shutdown_and_stays_running() {
    let config = AppConfig {
        worker_deadline_ms: 100,
        ..AppConfig::default()
    };
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let mut app = standard_builder(&log)
        .with_config(config)
        .startup()
        .unwrap();
    app.spawn_worker("slow-bake", 5000).unwrap();
    let error = app.shutdown().unwrap_err();
    assert_eq!(
        error,
        AppError::WorkerTimeout {
            name: "slow-bake",
            cost_ms: 5000,
            deadline_ms: 100,
        }
    );
    assert!(app.is_running());
    app.run_frame(1.0 / 60.0).unwrap();
}

#[test]
fn bad_config_refuses_startup_before_anything_starts() {
    let config = AppConfig {
        tick_hz: 59,
        ..AppConfig::default()
    };
    let error = App::builder().with_config(config).build().unwrap_err();
    assert!(matches!(error, AppError::BadConfig { .. }));
}

#[test]
fn register_after_startup_refuses() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let mut app = standard_builder(&log).startup().unwrap();
    assert_eq!(
        app.register("late", SystemKind::Poll, Phase::PreUpdate, |_| {}),
        Err(AppError::RegisterAfterStart { name: "late" })
    );
    assert_eq!(app.system_count(), 3);
}

#[test]
fn frames_and_shutdown_before_startup_refuse() {
    let mut app = App::builder().build().unwrap();
    assert_eq!(
        app.run_frame(1.0 / 60.0),
        Err(AppError::NotStarted { what: "run_frame" })
    );
    assert_eq!(
        app.shutdown(),
        Err(AppError::NotStarted { what: "shutdown" })
    );
    assert!(!app.is_running());
}

#[test]
fn bad_frame_dt_refuses_without_advancing() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let mut app = standard_builder(&log).startup().unwrap();
    assert!(matches!(
        app.run_frame(f32::NAN),
        Err(AppError::BadFrameDt { .. })
    ));
    assert_eq!(
        app.run_frame(-0.016),
        Err(AppError::BadFrameDt { value: -0.016 })
    );
    assert_eq!(app.frame(), 0);
    assert_eq!(app.sim_ticks(), 0);
}

#[test]
fn extraction_carries_tick_build_and_schema() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let mut app = standard_builder(&log).startup().unwrap();
    let first = app.run_frame(1.0 / 60.0).unwrap();
    assert_eq!(first.frame, 1);
    assert_eq!(first.steps, 1);
    assert_eq!(first.tick, 1);
    let snapshot = first.extraction;
    assert_eq!(snapshot.frame, 1);
    assert!(snapshot.is_for(1));
    assert_eq!(snapshot.build_hash, build_hash_for(2040));
    assert_eq!(snapshot.schema_version, EXTRACTION_SCHEMA);
    assert_eq!(
        snapshot.verify_hash,
        app.startup_receipt().unwrap().verify_hash
    );
    snapshot.require_tick(1).unwrap();

    let second = app.run_frame(1.0 / 60.0).unwrap();
    assert_eq!(second.frame, 2);
    assert_eq!(second.tick, 2);
    assert_ne!(second.extraction.tick, first.extraction.tick);
    assert_eq!(app.last_extraction(), Some(second.extraction));
    assert_ne!(app.app_hash(), 2040);
}

#[test]
fn platform_pumps_once_per_frame_outside_the_tick() {
    use crate::platform::{Platform, PlatformError};
    use std::sync::{Arc, Mutex};

    struct Counting {
        pumps: Arc<Mutex<usize>>,
    }
    impl Platform for Counting {
        fn name(&self) -> &'static str {
            "counting"
        }
        fn unlock_achievement(&mut self, id: &str) -> Result<bool, PlatformError> {
            crate::platform::check_id(id)?;
            Ok(false)
        }
        fn record_stat(&mut self, name: &str, _value: i64) -> Result<(), PlatformError> {
            crate::platform::check_id(name)?;
            Ok(())
        }
        fn stat(&self, _name: &str) -> Result<i64, PlatformError> {
            Err(PlatformError::NotConfigured)
        }
        fn store_cloud(&mut self, slot: u8, data: &[u8]) -> Result<(), PlatformError> {
            crate::platform::check_slot(slot)?;
            if data.len() > crate::platform::MAX_CLOUD_BYTES {
                return Err(PlatformError::TooLarge);
            }
            Ok(())
        }
        fn load_cloud(&self, _slot: u8) -> Result<Vec<u8>, PlatformError> {
            Err(PlatformError::NotConfigured)
        }
        fn set_presence(&mut self, _presence: &crate::platform::Presence) {}
        fn presence(&self) -> &crate::platform::Presence {
            static EMPTY: crate::platform::Presence = crate::platform::Presence {
                state: String::new(),
                details: String::new(),
            };
            &EMPTY
        }
        fn pump(&mut self) {
            *self.pumps.lock().expect("pump count") += 1;
        }
        fn create_lobby(&mut self) -> Result<crate::platform::LobbyId, PlatformError> {
            Err(PlatformError::NotConfigured)
        }
        fn invite_to_lobby(
            &mut self,
            _lobby: crate::platform::LobbyId,
            _user: &str,
        ) -> Result<(), PlatformError> {
            Err(PlatformError::NotConfigured)
        }
        fn publish_workshop_item(
            &mut self,
            _name: &str,
            _payload: &[u8],
        ) -> Result<crate::platform::WorkshopId, PlatformError> {
            Err(PlatformError::NotConfigured)
        }
        fn subscribed_workshop_items(&self) -> Vec<crate::platform::WorkshopId> {
            Vec::new()
        }
    }

    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let pumps = Arc::new(Mutex::new(0usize));
    let mut app = standard_builder(&log)
        .with_platform(Counting {
            pumps: Arc::clone(&pumps),
        })
        .startup()
        .unwrap();
    assert_eq!(app.platform_name(), "counting");
    let hash_before = app.app_hash();
    for frame in 1..=3u64 {
        let receipt = app.run_frame(1.0 / 60.0).unwrap();
        assert_eq!(receipt.frame, frame);
        assert_eq!(*pumps.lock().expect("pump count"), frame as usize);
    }
    // Same frames on the default null backend give the same app hash: the
    // platform never enters sim state.
    let mut bare = standard_builder(&log).startup().unwrap();
    assert_eq!(bare.platform_name(), "null");
    for _ in 1..=3u64 {
        bare.run_frame(1.0 / 60.0).unwrap();
    }
    assert_eq!(bare.app_hash(), app.app_hash());
    assert_ne!(hash_before, app.app_hash());
}

/// Crate-root wiring: every `pub mod` in `lib.rs` stays reachable with its
/// headline receipt intact, so a dropped module line fails at the `use`
/// below instead of at a caller's `use`.
///
/// Why this test exists: `crates/app/src/lib.rs` changed 5 times in 30 days
/// yet held zero `#[test]` and the crate has no `tests/it/` binary, so only
/// `src/tests.rs` can pin the root. The lifecycle tests never name
/// `live_proof`, so removing that module kept the crate green while breaking
/// every presenter caller at their `use` line. Each path below names the
/// root, and each value pins a caller-visible budget.
#[test]
fn root_reaches_every_module_with_headline_receipts() {
    use crate::{
        config::{AppConfig, QualityTier},
        journal::Journal,
        live_proof::{
            clamp_lease, Lease, LiveProof, LiveProofError, HEARTBEAT_MS, LEASE_MAX_MS,
            LEASE_MIN_MS, PROOF_TTL_MS,
        },
        platform::{NullPlatform, Platform},
        snapshot::{build_hash_for, MAX_FRAME_ACTORS},
    };

    // Config defaults callers boot with: hash-law seed, 60 Hz, 2 s drain.
    let defaults = AppConfig::default();
    assert_eq!(defaults.seed, 2040, "default seed via root");
    assert_eq!(defaults.tick_hz, 60, "default tick via root");
    assert_eq!(
        defaults.quality,
        QualityTier::Medium,
        "default tier via root"
    );
    assert_eq!(
        defaults.worker_deadline_ms, 2000,
        "default deadline via root"
    );
    assert_eq!(defaults.journal_capacity, 256, "default journal via root");
    assert_eq!(AppConfig::new(7).seed, 7, "explicit seed via root");
    assert_eq!(AppConfig::MIN_JOURNAL, 16, "journal floor via root");
    assert_eq!(
        AppConfig::MAX_DEADLINE_MS,
        30_000,
        "deadline ceiling via root"
    );
    defaults.validate().expect("defaults validate via root");

    // Journal receipt callers read back: record counts, seal refuses.
    let mut journal = Journal::new(2);
    assert!(journal.is_empty(), "fresh journal empty via root");
    journal
        .record("startup.ok", "booted".to_string())
        .expect("record via root");
    assert_eq!(journal.len(), 1, "one entry via root");
    journal.seal();
    assert!(journal.is_sealed(), "sealed via root");
    assert!(
        matches!(
            journal.record("late.step", "no".to_string()),
            Err(AppError::JournalSealed { .. })
        ),
        "sealed journal refuses via root"
    );

    // Live-proof budgets the presenter gates on: 60 s TTL, 2 s heartbeat.
    assert_eq!(PROOF_TTL_MS, 60_000, "proof TTL via root");
    assert_eq!(HEARTBEAT_MS, 2_000, "heartbeat via root");
    assert_eq!(LEASE_MAX_MS, 60_000, "lease ceiling via root");
    assert_eq!(LEASE_MIN_MS, 1_000, "lease floor via root");
    assert_eq!(clamp_lease(0), LEASE_MIN_MS, "clamp floor via root");
    assert_eq!(
        clamp_lease(u64::MAX),
        LEASE_MAX_MS,
        "clamp ceiling via root"
    );
    assert_eq!(
        Lease::take("import", "w1", 5_000).hold_ms,
        5_000,
        "lease hold via root"
    );
    let proof = LiveProof::of(
        7,
        "window-0",
        "target/live/frame.png",
        &"b".repeat(64),
        true,
        true,
        1_000,
    );
    assert!(proof.live_at(1_500), "full chain lives via root");
    assert_eq!(proof.attach(1_500), Ok(()), "full chain attaches via root");
    assert_eq!(
        LiveProof::of(0, "w", "f", &"b".repeat(64), true, true, 0).attach(1),
        Err(LiveProofError::BadPid),
        "zero pid refuses via root"
    );

    // Platform plus snapshot plus verify frame: null name, actor cap,
    // deterministic build hash and the two-fighter verify config.
    assert_eq!(NullPlatform::new().name(), "null", "null backend via root");
    assert_eq!(
        MAX_FRAME_ACTORS,
        usize::from(engine_arena::MAX_PARTICIPANTS),
        "frame actor cap via root"
    );
    assert_eq!(
        build_hash_for(2040),
        build_hash_for(2040),
        "build hash deterministic via root"
    );
    assert_eq!(VERIFY_TICKS, 8, "verify ticks via root");
    let config = verify_config(9);
    assert_eq!(config.seed, 9, "verify seed threads via root");
    assert_eq!(config.ticks, VERIFY_TICKS, "verify ticks via root");
    assert_eq!(config.fighters, 2, "verify fighters via root");
}

// Per-frame overhead harness: dispatch plus snapshot publish and read under
// a tenth of a millisecond p50 on the canonical workload (fixed seed, fixed
// sixtieth step, warmup then measured frames through poll, sim, extract and
// present). Pure helpers stay deterministic with no clock; only the ignored
// probe reads the clock, so the default suite stays green on slow machines
// while the probe enforces the budget and names the slowest stage on failure.
const RC31_BUDGET_US: u64 = 100;
// Same budget in nanoseconds for integer compares, no float in the gate.
const RC31_BUDGET_NS: u64 = 100_000;
// Fixed seed the workload boots with, matching the default config.
const RC31_SEED: u64 = 2040;
// Fixed step the workload frames with, matching the tick rate.
const RC31_DT: f32 = 1.0 / 60.0;
// Warmup frames before the window, so caches and allocators settle.
const RC31_WARMUP_FRAMES: usize = 20;
// Measured frames in the window, enough for a stable median.
const RC31_MEASURED_FRAMES: usize = 200;

// Median of ascending samples at fifty percent: rank length times fifty over
// one hundred, clamped to the last sample. Pure integer math; empty gives
// none. The caller sorts first with sort_unstable.
fn rc31_p50_sorted(sorted: &[u64]) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let mut rank = sorted.len() * 50 / 100;
    if rank >= sorted.len() {
        rank = sorted.len() - 1;
    }
    sorted.get(rank).copied()
}

// Slower of the two per-frame stages; ties name dispatch first so the
// refusal always points at one stage. Pure; names the budget culprit.
fn rc31_slowest_stage(ordering_us: u64, extraction_us: u64) -> &'static str {
    if ordering_us >= extraction_us {
        "ordering"
    } else {
        "extraction"
    }
}

// Canonical workload app: poll plus sim plus extract plus present on the
// fixed seed, so ordering and snapshot publish run every frame.
fn rc31_app(log: &Log) -> App {
    App::builder()
        .register(
            "poll",
            SystemKind::Poll,
            Phase::PreUpdate,
            logged(log, "poll"),
        )
        .expect("poll registers")
        .register(
            "sim",
            SystemKind::MatchStep,
            Phase::Update,
            logged(log, "sim"),
        )
        .expect("sim registers")
        .register(
            "extract",
            SystemKind::Extract,
            Phase::PostUpdate,
            logged(log, "extract"),
        )
        .expect("extract registers")
        .register(
            "hud",
            SystemKind::Present,
            Phase::PostUpdate,
            logged(log, "hud"),
        )
        .expect("hud registers")
        .startup()
        .expect("workload boots")
}

// Run the canonical workload and return sorted per-stage nanos plus the
// per-frame combined nanos, all sorted ascending for the median helper.
// The clock lives only here and in its two callers below.
fn rc31_measure() -> (Vec<u64>, Vec<u64>, Vec<u64>) {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let mut app = rc31_app(&log);
    for _ in 0..RC31_WARMUP_FRAMES {
        app.run_frame(RC31_DT).expect("warmup runs");
    }
    let mut ordering: Vec<u64> = Vec::with_capacity(RC31_MEASURED_FRAMES);
    let mut extraction: Vec<u64> = Vec::with_capacity(RC31_MEASURED_FRAMES);
    let mut combined: Vec<u64> = Vec::with_capacity(RC31_MEASURED_FRAMES);
    for _ in 0..RC31_MEASURED_FRAMES {
        let start = std::time::Instant::now();
        let receipt = app.run_frame(RC31_DT).expect("measured frame runs");
        let ordering_ns = u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX);
        let read_start = std::time::Instant::now();
        let snapshot = app.last_extraction().expect("publish posts");
        snapshot.require_tick(receipt.tick).expect("fresh reads");
        assert!(snapshot.is_for(receipt.tick));
        std::hint::black_box(snapshot);
        let extraction_ns = u64::try_from(read_start.elapsed().as_nanos()).unwrap_or(u64::MAX);
        ordering.push(ordering_ns);
        extraction.push(extraction_ns);
        combined.push(ordering_ns.saturating_add(extraction_ns));
    }
    ordering.sort_unstable();
    extraction.sort_unstable();
    combined.sort_unstable();
    (ordering, extraction, combined)
}

#[test]
fn rc31_percentile_pins_median() {
    let ladder = vec![10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
    assert_eq!(rc31_p50_sorted(&ladder), Some(60));
    assert_eq!(rc31_p50_sorted(&[]), None);
    assert_eq!(rc31_p50_sorted(&[7]), Some(7));
    assert_eq!(rc31_p50_sorted(&ladder), rc31_p50_sorted(&ladder));
    let moved = vec![11, 21, 31, 41, 51, 61, 71, 81, 91, 101];
    assert_eq!(rc31_p50_sorted(&moved), Some(61));
}

#[test]
fn rc31_over_budget_names_slowest_stage() {
    assert_eq!(rc31_slowest_stage(80, 20), "ordering");
    assert_eq!(rc31_slowest_stage(20, 80), "extraction");
    assert_eq!(rc31_slowest_stage(50, 50), "ordering");
    let slowest = rc31_slowest_stage(120, 30);
    let report = format!(
        "over budget: combined {}us over {}us; slowest stage: {slowest} (ordering {}us, extraction {}us)",
        150,
        RC31_BUDGET_US,
        120,
        30
    );
    assert!(report.contains(slowest));
    assert!(report.contains("ordering") && report.contains("extraction"));
}

#[test]
fn rc31_workload_runs_canonical_shape() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let mut app = rc31_app(&log);
    assert_eq!(app.system_count(), 4);
    app.run_frame(RC31_DT).expect("first frame runs");
    assert_eq!(*log.borrow(), vec!["poll", "sim", "extract", "hud"]);
    let first = app.last_extraction().expect("publish posts");
    assert!(first.is_for(1));
    first.require_tick(1).expect("fresh snapshot reads");
    assert_eq!(first.build_hash, build_hash_for(RC31_SEED));
    let mut again = rc31_app(&Rc::new(RefCell::new(Vec::new())));
    again.run_frame(RC31_DT).expect("repeat runs");
    let repeat = again.last_extraction().expect("repeat publishes");
    assert_eq!(repeat.tick, first.tick);
    assert_eq!(repeat.build_hash, first.build_hash);
    assert_eq!(repeat.schema_version, first.schema_version);
}

#[test]
fn rc31_harness_posts_p50() {
    let (ordering, extraction, combined) = rc31_measure();
    let ordering_p50 = rc31_p50_sorted(&ordering).expect("samples give a median");
    let extraction_p50 = rc31_p50_sorted(&extraction).expect("samples give a median");
    let combined_p50 = rc31_p50_sorted(&combined).expect("samples give a median");
    eprintln!(
        "RC31 ordering+extraction p50={}us (ordering {}us, extraction {}us) over {} frames, seed {}, dt {}s",
        combined_p50 / 1000,
        ordering_p50 / 1000,
        extraction_p50 / 1000,
        RC31_MEASURED_FRAMES,
        RC31_SEED,
        RC31_DT
    );
    assert_eq!(ordering.len(), RC31_MEASURED_FRAMES);
    assert_eq!(extraction.len(), RC31_MEASURED_FRAMES);
    assert_eq!(combined.len(), RC31_MEASURED_FRAMES);
    assert!(combined_p50 >= ordering_p50);
    assert!(combined_p50 >= extraction_p50);
}

#[test]
#[ignore = "timing: needs a quiet release box; run with --release -- --ignored --test-threads=1"]
fn rc31_ordering_plus_extraction_under_budget() {
    let (ordering, extraction, combined) = rc31_measure();
    let ordering_p50 = rc31_p50_sorted(&ordering).expect("samples give a median");
    let extraction_p50 = rc31_p50_sorted(&extraction).expect("samples give a median");
    let combined_p50 = rc31_p50_sorted(&combined).expect("samples give a median");
    let ordering_us = ordering_p50 / 1000;
    let extraction_us = extraction_p50 / 1000;
    let combined_us = combined_p50 / 1000;
    eprintln!(
        "RC31 ordering+extraction p50={combined_us}us (ordering {ordering_us}us, extraction {extraction_us}us) over {} frames, seed {}, dt {}s",
        RC31_MEASURED_FRAMES,
        RC31_SEED,
        RC31_DT
    );
    assert!(
        combined_p50 <= RC31_BUDGET_NS,
        "over budget: combined {combined_us}us over {}us; slowest stage: {} (ordering {ordering_us}us, extraction {extraction_us}us)",
        RC31_BUDGET_US,
        rc31_slowest_stage(ordering_us, extraction_us)
    );
}
