//! Engine-app lifecycle: builder, registration, startup, frames, shutdown.
//!
//! The app orders systems and extracts snapshots only. It never steps
//! combat or physics itself: the only match contact is the startup verify
//! frame, which reads arena checkpoints through the public `run` entry.
//! Ordering and extraction never enter sim hashes; they are receipts.

use std::time::Instant;

use engine_arena::{ArenaConfig, FighterTape};
use engine_core::{hash_bytes, IntAcc, TickState};
use engine_ecs::archetype::{Position, Registry, Velocity};
use engine_ecs::schedule::Phase;
use engine_scene::codec::{Scene, CURRENT_VERSION};
use engine_scene::transform::Hierarchy;

use crate::config::AppConfig;
use crate::error::AppError;
use crate::journal::Journal;
use crate::platform::{NullPlatform, Platform};
use crate::snapshot::{build_hash_for, Extraction, EXTRACTION_SCHEMA};

/// Ticks simulated for the startup verify frame (brains fill every slot).
pub const VERIFY_TICKS: u32 = 8;

/// Arena config the startup verify frame runs under for one seed.
pub fn verify_config(seed: u64) -> ArenaConfig {
    ArenaConfig {
        seed,
        ticks: VERIFY_TICKS,
        fighters: 2,
        half_extent_m: 20.0,
        table_version: 1,
    }
}

/// Which lane a system belongs to; each lane has exactly one home phase
/// from the runtime phase table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemKind {
    /// Builds registries, schemas and the journal. Home: Startup.
    Setup,
    /// Polls inputs, files, network and watch notices. Home: PreUpdate.
    Poll,
    /// Steps the arena match plus finished sim-adjacent jobs. Home: Update.
    MatchStep,
    /// Extracts snapshots and publishes to render/audio/UI. Home: PostUpdate.
    /// Runs before Present lanes, so menus read this frame's snapshot.
    Extract,
    /// Presentation lanes (render, audio, UI). Home: PostUpdate.
    /// Runs after Extract lanes, reading this frame's snapshot.
    Present,
}

impl SystemKind {
    /// The phase this lane belongs in; anything else refuses at registration.
    pub fn home(self) -> Phase {
        match self {
            Self::Setup => Phase::Startup,
            Self::Poll => Phase::PreUpdate,
            Self::MatchStep => Phase::Update,
            Self::Extract | Self::Present => Phase::PostUpdate,
        }
    }
}

/// Per-system call context: fixed dt in, never wall-clock.
#[derive(Clone, Copy, Debug)]
pub struct FrameCtx {
    /// Fixed ticks elapsed (accumulator steps).
    pub tick: u64,
    /// Presentation frames since handover.
    pub frame: u64,
    /// Fixed step in seconds.
    pub dt: f32,
    /// Phase currently running.
    pub phase: Phase,
}

/// Boxed system body: ordering context only, never sim state.
pub type SystemFn = Box<dyn FnMut(&mut FrameCtx)>;

struct Registration {
    name: &'static str,
    kind: SystemKind,
    phase: Phase,
    run: SystemFn,
}

/// Worker handle awaiting shutdown drain. The handle carries the worker's
/// measured cost in milliseconds; drain enforces the deadline against it,
/// so deadline behavior is deterministic and needs no wall clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Worker {
    name: &'static str,
    cost_ms: u64,
}

/// Lifecycle position. Startup flips Unstarted to Running exactly once;
/// shutdown flips Running to Shutdown exactly once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lifecycle {
    Unstarted,
    Running,
    Shutdown,
}

/// Startup receipt: what booted, how long wall-clock startup took.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartupReceipt {
    /// Wall-clock startup time in milliseconds.
    pub startup_ms: u64,
    /// Last arena checkpoint hash of the verify frame, run twice.
    pub verify_hash: u64,
    /// Ticks in the verify frame ([`VERIFY_TICKS`]).
    pub verify_ticks: u32,
    /// Systems handed over in phase order.
    pub systems: usize,
    /// Scene nodes loaded.
    pub scene_nodes: usize,
    /// Scene schema version loaded.
    pub schema_version: u32,
    /// Build hash from the seed.
    pub build_hash: u64,
}

/// Per-frame receipt: frame count, steps taken, tick and extraction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameReceipt {
    /// Presentation frames since handover, 1-based.
    pub frame: u64,
    /// Fixed steps this frame (accumulator, capped at 5).
    pub steps: u32,
    /// Fixed ticks elapsed.
    pub tick: u64,
    /// Snapshot published to render/audio/UI.
    pub extraction: Extraction,
}

/// Shutdown receipt: reverse drop order plus drain counts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShutdownReceipt {
    /// Systems dropped, reverse registration order.
    pub drop_order: Vec<&'static str>,
    /// Workers drained within their deadlines.
    pub workers_drained: usize,
    /// Journal entries sealed (attempt plus shutdown steps).
    pub journal_entries: usize,
}

/// Pre-start builder: config, systems, scene document, then build/startup.
#[derive(Default)]
pub struct AppBuilder {
    config: AppConfig,
    systems: Vec<Registration>,
    scene: Option<Scene>,
    platform: Option<Box<dyn Platform>>,
}

impl std::fmt::Debug for AppBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<(&'static str, Phase)> = self
            .systems
            .iter()
            .map(|entry| (entry.name, entry.phase))
            .collect();
        f.debug_struct("AppBuilder")
            .field("config", &self.config)
            .field("systems", &names)
            .field("has_scene", &self.scene.is_some())
            .field("has_platform", &self.platform.is_some())
            .finish()
    }
}

impl AppBuilder {
    /// Empty builder on hash-law defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Boot with this config instead of the default.
    pub fn with_config(mut self, config: AppConfig) -> Self {
        self.config = config;
        self
    }

    /// Register one system: duplicates and wrong-phase lanes refuse loudly.
    pub fn register(
        mut self,
        name: &'static str,
        kind: SystemKind,
        phase: Phase,
        system: impl FnMut(&mut FrameCtx) + 'static,
    ) -> Result<Self, AppError> {
        check_registration(&self.systems, name, kind, phase)?;
        self.systems.push(Registration {
            name,
            kind,
            phase,
            run: Box::new(system),
        });
        Ok(self)
    }

    /// Stage the scene document decoded at startup.
    pub fn scene_doc(mut self, doc: Scene) -> Self {
        self.scene = Some(doc);
        self
    }

    /// Attach a platform backend (achievements, stats, cloud, presence).
    /// The default is the null backend, so the arena runs identically with
    /// no store present; callbacks pump once per frame outside the tick.
    pub fn with_platform(mut self, platform: impl Platform + 'static) -> Self {
        self.platform = Some(Box::new(platform));
        self
    }

    /// Validate the config and return an unstarted app.
    pub fn build(self) -> Result<App, AppError> {
        self.config.validate()?;
        let journal = Journal::new(self.config.journal_capacity);
        let platform: Box<dyn Platform> = self
            .platform
            .unwrap_or_else(|| Box::new(NullPlatform::new()));
        Ok(App {
            core: self.config.core(),
            config: self.config,
            systems: self.systems,
            pending_scene: self.scene,
            platform,
            scene_nodes: 0,
            schema_version: CURRENT_VERSION,
            acc: IntAcc::new(),
            tick: TickState::new(self.config.seed),
            frame: 0,
            sim_ticks: 0,
            lifecycle: Lifecycle::Unstarted,
            journal,
            workers: Vec::new(),
            accepts_inputs: false,
            startup_receipt: None,
            last_extraction: None,
        })
    }

    /// Build and run the full startup sequence in one step.
    pub fn startup(self) -> Result<App, AppError> {
        let mut app = self.build()?;
        app.startup()?;
        Ok(app)
    }
}

/// Reject a duplicate name or a lane outside its home phase.
fn check_registration(
    systems: &[Registration],
    name: &'static str,
    kind: SystemKind,
    phase: Phase,
) -> Result<(), AppError> {
    if systems.iter().any(|entry| entry.name == name) {
        return Err(AppError::Duplicate { name });
    }
    let want = kind.home();
    if phase != want {
        return Err(AppError::WrongPhase {
            name,
            got: phase,
            want,
        });
    }
    Ok(())
}

/// Decode plus validate a scene document without leaving partial state.
fn decode_scene(doc: &Scene) -> Result<Hierarchy, AppError> {
    let tree = doc.decode().map_err(|error| AppError::SceneLoad {
        reason: error.to_string(),
    })?;
    tree.validate().map_err(|error| AppError::SceneLoad {
        reason: error.to_string(),
    })?;
    Ok(tree)
}

/// Run the verify frame twice; identical checkpoints prove determinism.
fn verify_checkpoints(seed: u64) -> Result<u64, AppError> {
    let config = verify_config(seed);
    let tape = FighterTape::default();
    let first = engine_arena::run(&config, &tape);
    let second = engine_arena::run(&config, &tape);
    if first != second {
        return Err(AppError::Verify {
            reason: format!("arena checkpoints diverged over {VERIFY_TICKS} ticks at seed {seed}"),
        });
    }
    first
        .last()
        .map(|checkpoint| checkpoint.hash)
        .ok_or_else(|| AppError::Verify {
            reason: format!("arena verify frame came back empty at seed {seed}"),
        })
}

/// The engine application: owns startup order, frames and shutdown.
pub struct App {
    config: AppConfig,
    core: engine_core::config::Config,
    systems: Vec<Registration>,
    pending_scene: Option<Scene>,
    platform: Box<dyn Platform>,
    scene_nodes: usize,
    schema_version: u32,
    acc: IntAcc,
    tick: TickState,
    frame: u64,
    sim_ticks: u64,
    lifecycle: Lifecycle,
    journal: Journal,
    workers: Vec<Worker>,
    accepts_inputs: bool,
    startup_receipt: Option<StartupReceipt>,
    last_extraction: Option<Extraction>,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<(&'static str, Phase)> = self
            .systems
            .iter()
            .map(|entry| (entry.name, entry.phase))
            .collect();
        f.debug_struct("App")
            .field("config", &self.config)
            .field("systems", &names)
            .field("platform", &self.platform.name())
            .field("frame", &self.frame)
            .field("sim_ticks", &self.sim_ticks)
            .field("lifecycle", &self.lifecycle)
            .finish()
    }
}

impl App {
    /// Start building an app.
    pub fn builder() -> AppBuilder {
        AppBuilder::new()
    }

    /// Systems registered so far, in registration order.
    pub fn system_count(&self) -> usize {
        self.systems.len()
    }

    /// The lane of a registered system, if present.
    pub fn system_kind(&self, name: &str) -> Option<SystemKind> {
        self.systems
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| entry.kind)
    }

    /// True while the app accepts inputs: between handover and shutdown.
    pub fn accepts_inputs(&self) -> bool {
        self.accepts_inputs
    }

    /// True once startup handed over and shutdown has not run.
    pub fn is_running(&self) -> bool {
        self.lifecycle == Lifecycle::Running
    }

    /// Presentation frames since handover.
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// Fixed ticks elapsed through the accumulator.
    pub fn sim_ticks(&self) -> u64 {
        self.sim_ticks
    }

    /// Scene nodes loaded at startup.
    pub fn scene_nodes(&self) -> usize {
        self.scene_nodes
    }

    /// Schema version the scene was written with.
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Ordering receipt hash (never a sim hash).
    pub fn app_hash(&self) -> u64 {
        self.tick.hash()
    }

    /// The startup/shutdown journal.
    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    /// The startup receipt, once handed over.
    pub fn startup_receipt(&self) -> Option<&StartupReceipt> {
        self.startup_receipt.as_ref()
    }

    /// Backend name for receipts (`"null"`, `"mock"`, `"steam"`).
    pub fn platform_name(&self) -> &'static str {
        self.platform.name()
    }

    /// The last published extraction snapshot, if any frame ran.
    pub fn last_extraction(&self) -> Option<Extraction> {
        self.last_extraction
    }

    /// Register one system before startup; duplicates and wrong-phase
    /// lanes refuse loudly, and anything after startup refuses.
    pub fn register(
        &mut self,
        name: &'static str,
        kind: SystemKind,
        phase: Phase,
        system: impl FnMut(&mut FrameCtx) + 'static,
    ) -> Result<(), AppError> {
        match self.lifecycle {
            Lifecycle::Unstarted => {
                check_registration(&self.systems, name, kind, phase)?;
                self.systems.push(Registration {
                    name,
                    kind,
                    phase,
                    run: Box::new(system),
                });
                Ok(())
            }
            Lifecycle::Running => Err(AppError::RegisterAfterStart { name }),
            Lifecycle::Shutdown => Err(AppError::AlreadyShutdown { what: "register" }),
        }
    }

    /// Stage a scene document before startup; it decodes now so a bad
    /// save refuses before anything starts and leaves prior state intact.
    pub fn load_scene(&mut self, doc: &Scene) -> Result<(), AppError> {
        match self.lifecycle {
            Lifecycle::Unstarted => {}
            Lifecycle::Running => return Err(AppError::AlreadyStarted { what: "load_scene" }),
            Lifecycle::Shutdown => return Err(AppError::AlreadyShutdown { what: "load_scene" }),
        }
        let tree = decode_scene(doc)?;
        self.pending_scene = Some(doc.clone());
        self.scene_nodes = tree.len();
        self.schema_version = doc.version;
        self.journal.record(
            "scene.staged",
            format!("{} nodes at v{}", tree.len(), doc.version),
        )?;
        Ok(())
    }

    /// Enqueue a worker handle for shutdown drain with its measured cost.
    pub fn spawn_worker(&mut self, name: &'static str, cost_ms: u64) -> Result<(), AppError> {
        match self.lifecycle {
            Lifecycle::Shutdown => Err(AppError::AlreadyShutdown {
                what: "spawn_worker",
            }),
            Lifecycle::Unstarted | Lifecycle::Running => {
                self.workers.push(Worker { name, cost_ms });
                Ok(())
            }
        }
    }

    /// Run the startup sequence: config, registries, phase-order
    /// registration, scene load, one verified headless frame, handover.
    /// Any failure leaves the app unstarted and retryable.
    pub fn startup(&mut self) -> Result<StartupReceipt, AppError> {
        if self.lifecycle != Lifecycle::Unstarted {
            return Err(AppError::AlreadyStarted { what: "startup" });
        }
        let wall = Instant::now();
        self.journal.record(
            "startup.config",
            format!(
                "seed {} quality {:?}",
                self.config.seed, self.config.quality
            ),
        )?;

        let mut registry = Registry::new();
        let pos = registry.register::<Position>("position");
        let vel = registry.register::<Velocity>("velocity");
        if pos.is_none() || vel.is_none() {
            return Err(AppError::Registry {
                reason: "Position/Velocity bits missing from the component registry".to_string(),
            });
        }
        self.journal.record(
            "startup.registries",
            format!("{} components, schema v{CURRENT_VERSION}", registry.len()),
        )?;
        self.run_phase(Phase::Startup, 0, 0);

        self.journal.record(
            "startup.systems_ordered",
            format!("{} systems in phase order", self.systems.len()),
        )?;

        let doc = self.pending_scene.clone().unwrap_or(Scene {
            version: CURRENT_VERSION,
            nodes: Vec::new(),
        });
        let tree = decode_scene(&doc)?;
        self.scene_nodes = tree.len();
        self.schema_version = doc.version;
        self.journal.record(
            "startup.scene_loaded",
            format!("{} nodes at v{}", tree.len(), doc.version),
        )?;

        let verify_hash = verify_checkpoints(self.config.seed)?;
        self.journal.record(
            "startup.frame_verified",
            format!("{VERIFY_TICKS} ticks agree twice, hash {verify_hash:016x}"),
        )?;

        let startup_ms = u64::try_from(wall.elapsed().as_millis()).unwrap_or(u64::MAX);
        let receipt = StartupReceipt {
            startup_ms,
            verify_hash,
            verify_ticks: VERIFY_TICKS,
            systems: self.systems.len(),
            scene_nodes: self.scene_nodes,
            schema_version: self.schema_version,
            build_hash: build_hash_for(self.config.seed),
        };
        self.journal.record(
            "startup.handover",
            format!("{} systems, {}ms", receipt.systems, receipt.startup_ms),
        )?;
        self.lifecycle = Lifecycle::Running;
        self.accepts_inputs = true;
        self.startup_receipt = Some(receipt.clone());
        Ok(receipt)
    }

    /// Run one frame: accumulator steps, then PreUpdate, Update, PostUpdate
    /// in order, then extraction. Startup systems never run here.
    pub fn run_frame(&mut self, frame_dt: f32) -> Result<FrameReceipt, AppError> {
        match self.lifecycle {
            Lifecycle::Unstarted => return Err(AppError::NotStarted { what: "run_frame" }),
            Lifecycle::Shutdown => return Err(AppError::AlreadyShutdown { what: "run_frame" }),
            Lifecycle::Running => {}
        }
        if !frame_dt.is_finite() || frame_dt < 0.0 {
            return Err(AppError::BadFrameDt { value: frame_dt });
        }
        // Finite and non-negative above, so only an impossible 2^64-tick
        // overflow can refuse here; it reports the same typed frame error.
        let steps = self
            .acc
            .push_secs(frame_dt)
            .map_err(|_| AppError::BadFrameDt { value: frame_dt })?;
        self.sim_ticks += u64::from(steps);
        self.frame += 1;
        let frame = self.frame;
        let tick_value = self.sim_ticks;
        self.tick.step(hash_bytes(&frame.to_le_bytes()));
        for phase in [Phase::PreUpdate, Phase::Update, Phase::PostUpdate] {
            self.run_phase(phase, tick_value, frame);
        }
        let extraction = Extraction {
            tick: self.sim_ticks,
            frame: self.frame,
            build_hash: build_hash_for(self.config.seed),
            schema_version: EXTRACTION_SCHEMA,
            scene_nodes: self.scene_nodes,
            verify_hash: self
                .startup_receipt
                .as_ref()
                .map_or(0, |receipt| receipt.verify_hash),
        };
        self.last_extraction = Some(extraction);
        // Platform callbacks pump once per frame outside the fixed tick: the
        // accumulator already stepped above, and nothing here enters hashes.
        self.platform.pump();
        Ok(FrameReceipt {
            frame,
            steps,
            tick: self.sim_ticks,
            extraction,
        })
    }

    /// Shut down in reverse: stop inputs, drain workers against their
    /// deadlines, seal the journal, drop systems in reverse order.
    /// A worker over deadline aborts shutdown and leaves the app running.
    pub fn shutdown(&mut self) -> Result<ShutdownReceipt, AppError> {
        match self.lifecycle {
            Lifecycle::Unstarted => return Err(AppError::NotStarted { what: "shutdown" }),
            Lifecycle::Shutdown => return Err(AppError::AlreadyShutdown { what: "shutdown" }),
            Lifecycle::Running => {}
        }
        self.accepts_inputs = false;
        self.journal.record(
            "shutdown.inputs_stopped",
            "no new inputs accepted".to_string(),
        )?;
        let deadline = self.config.worker_deadline_ms;
        for worker in &self.workers {
            if worker.cost_ms > deadline {
                self.journal.record(
                    "shutdown.workers_drained",
                    format!("worker '{}' over deadline", worker.name),
                )?;
                self.accepts_inputs = true;
                return Err(AppError::WorkerTimeout {
                    name: worker.name,
                    cost_ms: worker.cost_ms,
                    deadline_ms: deadline,
                });
            }
        }
        let workers_drained = self.workers.len();
        self.workers.clear();
        self.journal.record(
            "shutdown.workers_drained",
            format!("{workers_drained} workers within {deadline}ms"),
        )?;
        self.journal
            .record("shutdown.journal_sealing", "sealing".to_string())?;
        self.journal.seal();
        let drop_order: Vec<&'static str> =
            self.systems.iter().rev().map(|entry| entry.name).collect();
        self.systems.clear();
        self.lifecycle = Lifecycle::Shutdown;
        Ok(ShutdownReceipt {
            drop_order,
            workers_drained,
            journal_entries: self.journal.len(),
        })
    }

    /// Run every system in one phase, in registration order, except
    /// PostUpdate runs Extract lanes before Present lanes so menus read
    /// this frame's snapshot whatever order the game registered them in.
    fn run_phase(&mut self, phase: Phase, tick: u64, frame: u64) {
        let mut order: Vec<usize> = self
            .systems
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.phase == phase)
            .map(|(index, _)| index)
            .collect();
        if phase == Phase::PostUpdate {
            order.sort_by_key(|&index| match self.systems[index].kind {
                SystemKind::Extract => 0u8,
                _ => 1u8,
            });
        }
        for index in order {
            let mut ctx = FrameCtx {
                tick,
                frame,
                dt: self.core.dt(),
                phase,
            };
            (self.systems[index].run)(&mut ctx);
        }
    }
}

/// Late lifecycle refusals as the app owner sees them.
///
/// Why this module exists: `crates/app/src/app.rs` is hot (6 changes in
/// 30 days) with no `#[test]` in it and the crate has no `tests/it/`
/// binary, so only an in-file module can pin the refusal arms. The
/// sibling `src/tests.rs` never calls `load_scene` after startup, never
/// restarts, and never spawns after shutdown, so dropping any of those
/// arms kept the crate green while breaking every caller that retries.
#[cfg(test)]
mod lifecycle_refusal_tests {
    use super::{App, AppError};
    use engine_scene::codec::{Scene, CURRENT_VERSION};

    fn empty_scene() -> Scene {
        Scene {
            version: CURRENT_VERSION,
            nodes: Vec::new(),
        }
    }

    #[test]
    fn late_ops_refuse_and_leave_state_intact() {
        let mut app = App::builder().build().expect("defaults build");
        app.load_scene(&empty_scene())
            .expect("pre-start scene stages");
        let receipt = app.startup().expect("startup hands over");
        assert_eq!(receipt.scene_nodes, 0);
        assert!(app.is_running());
        assert!(app.accepts_inputs());

        // A staged-again scene after handover refuses and keeps the old scene.
        assert_eq!(
            app.load_scene(&empty_scene()),
            Err(AppError::AlreadyStarted { what: "load_scene" })
        );
        assert!(app.is_running());
        assert_eq!(app.scene_nodes(), 0);

        // A second startup refuses and keeps the app running.
        assert_eq!(
            app.startup(),
            Err(AppError::AlreadyStarted { what: "startup" })
        );
        assert!(app.is_running());
        app.run_frame(1.0 / 60.0)
            .expect("still usable after refusals");

        app.shutdown().expect("shutdown drains");
        assert!(!app.is_running());

        // Sealed apps refuse new work instead of reopening the journal.
        assert_eq!(
            app.load_scene(&empty_scene()),
            Err(AppError::AlreadyShutdown { what: "load_scene" })
        );
        assert_eq!(
            app.spawn_worker("late", 1),
            Err(AppError::AlreadyShutdown {
                what: "spawn_worker"
            })
        );
    }
}

/// Cold-startup plus prefab-instantiate budget proof on the reference box.
///
/// Consumer: `App::startup` for cold boot and `Prefab::instantiate` for the
/// arena kit path. Data owner: the RC4 museum fixture text, read-only here.
/// Units: startup in ms, instantiate in us/ns. Determinism: budgets compare
/// integer clocks only; wall time never enters sim hashes.
#[cfg(test)]
mod rc32_budget_tests {
    use super::{App, VERIFY_TICKS};
    use engine_scene::prefab::{Prefab, PrefabNode, MUSEUM_V1_JSON, PREFAB_VERSION};
    use engine_scene::schema::{SchemaRegistry, VersionedComponent};
    use engine_scene::transform::{Hierarchy, Transform};
    use std::time::Instant;

    /// Cold startup budget on the reference machine: under 2 s.
    const STARTUP_BUDGET_MS: u64 = 2_000;
    /// Prefab instantiate budget per arena kit: under 1 ms.
    const KIT_BUDGET_US: u64 = 1_000;
    /// Same kit budget in nanoseconds for integer compares.
    const KIT_BUDGET_NS: u64 = KIT_BUDGET_US * 1_000;
    /// Timed kit samples per gate run.
    const KIT_SAMPLES: usize = 200;
    /// Warmup instantiates before the timed window.
    const KIT_WARMUP: usize = 10;

    fn label_component(payload: &str) -> VersionedComponent {
        VersionedComponent {
            component: "label".to_string(),
            version: 1,
            payload: payload.to_string(),
        }
    }

    fn arena_kit() -> Prefab {
        let hall_pose = Transform::new([0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]);
        let plinth_pose = Transform::new([0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]);
        let bust_pose = Transform::new([0.0, 0.6, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]);
        Prefab {
            schema_version: PREFAB_VERSION,
            nodes: vec![
                PrefabNode {
                    key: "hall".to_string(),
                    parent: None,
                    transform: hall_pose,
                    components: vec![label_component("Grand Hall")],
                },
                PrefabNode {
                    key: "hall/plinth".to_string(),
                    parent: Some("hall".to_string()),
                    transform: plinth_pose,
                    components: vec![label_component("Plinth")],
                },
                PrefabNode {
                    key: "hall/plinth/bust".to_string(),
                    parent: Some("hall/plinth".to_string()),
                    transform: bust_pose,
                    components: vec![label_component("Bust of the Founder")],
                },
            ],
        }
    }

    fn check_budgets(startup_ms: u64, kit_ns: u64) -> Result<(), &'static str> {
        if startup_ms >= STARTUP_BUDGET_MS {
            return Err("startup");
        }
        if kit_ns >= KIT_BUDGET_NS {
            return Err("kit");
        }
        Ok(())
    }

    fn median_ns(sorted: &[u64]) -> Option<u64> {
        if sorted.is_empty() {
            return None;
        }
        sorted.get(sorted.len() / 2).copied()
    }

    fn cold_startup_ms() -> u64 {
        let wall = Instant::now();
        let app = App::builder().startup().expect("defaults boot");
        let receipt = app.startup_receipt().expect("receipt handed over");
        assert_eq!(receipt.verify_ticks, VERIFY_TICKS);
        let wall_ms = u64::try_from(wall.elapsed().as_millis()).unwrap_or(u64::MAX);
        wall_ms.max(receipt.startup_ms)
    }

    fn kit_median_ns() -> u64 {
        let kit = arena_kit();
        let registry = SchemaRegistry::builtin();
        for _ in 0..KIT_WARMUP {
            let mut tree = Hierarchy::new();
            let made = kit.instantiate(&mut tree, &registry).expect("kit builds");
            std::hint::black_box(made);
        }
        let mut samples = Vec::with_capacity(KIT_SAMPLES);
        for _ in 0..KIT_SAMPLES {
            let mut tree = Hierarchy::new();
            let start = Instant::now();
            let made = kit.instantiate(&mut tree, &registry).expect("kit builds");
            let ns = u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX);
            assert_eq!(tree.len(), 3);
            std::hint::black_box(made);
            samples.push(ns);
        }
        samples.sort_unstable();
        median_ns(&samples).expect("samples give a median")
    }

    #[test]
    fn kit_shape_matches_rc4_fixture() {
        let kit = arena_kit();
        assert_eq!(kit.nodes.len(), 3);
        assert!(MUSEUM_V1_JSON.contains("hall/plinth/bust"));
        assert!(MUSEUM_V1_JSON.contains("Bust of the Founder"));
        let registry = SchemaRegistry::builtin();
        kit.validate(&registry).expect("kit validates");
        let mut tree = Hierarchy::new();
        let made = kit.instantiate(&mut tree, &registry).expect("kit builds");
        assert_eq!(tree.len(), 3);
        assert!(made.get("hall").is_some());
        assert!(made.get("hall/plinth").is_some());
        assert!(made.get("hall/plinth/bust").is_some());
    }

    #[test]
    fn median_pins_middle() {
        let mut ladder = vec![30, 10, 20];
        ladder.sort_unstable();
        assert_eq!(median_ns(&ladder), Some(20));
        assert_eq!(median_ns(&[]), None);
        assert_eq!(median_ns(&[7]), Some(7));
    }

    #[test]
    fn over_budget_inputs_fail_the_gate() {
        assert_eq!(check_budgets(2_000, 0), Err("startup"));
        assert_eq!(check_budgets(0, 1_000_000), Err("kit"));
        assert_eq!(check_budgets(2_000, 1_000_000), Err("startup"));
        assert_eq!(check_budgets(0, 0), Ok(()));
    }

    #[test]
    fn harness_posts_numbers_twice() {
        let first_startup = cold_startup_ms();
        let first_kit = kit_median_ns();
        let second_startup = cold_startup_ms();
        let second_kit = kit_median_ns();
        eprintln!(
            "RC32 run1: startup {first_startup}ms (budget {STARTUP_BUDGET_MS}ms), \
             kit {}us (budget {KIT_BUDGET_US}us)",
            first_kit / 1_000,
        );
        eprintln!(
            "RC32 run2: startup {second_startup}ms (budget {STARTUP_BUDGET_MS}ms), \
             kit {}us (budget {KIT_BUDGET_US}us)",
            second_kit / 1_000,
        );
        assert_ne!(first_startup, u64::MAX);
        assert_ne!(second_startup, u64::MAX);
        assert_ne!(first_kit, u64::MAX);
        assert_ne!(second_kit, u64::MAX);
    }

    #[test]
    #[ignore = "timing: needs a quiet release box; run with --release -- --ignored --test-threads=1"]
    fn cold_startup_under_2s() {
        let startup = cold_startup_ms();
        eprintln!("RC32 startup {startup}ms (budget {STARTUP_BUDGET_MS}ms)");
        assert_eq!(check_budgets(startup, 0), Ok(()));
    }

    #[test]
    #[ignore = "timing: needs a quiet release box; run with --release -- --ignored --test-threads=1"]
    fn kit_instantiate_under_1ms() {
        let median = kit_median_ns();
        eprintln!(
            "RC32 kit median {}us (budget {KIT_BUDGET_US}us) over {KIT_SAMPLES} samples",
            median / 1_000,
        );
        assert_eq!(check_budgets(0, median), Ok(()));
    }
}
