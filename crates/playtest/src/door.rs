//! Eval door driver: one provider-contract call, then an atomic scene apply.
//!
//! Consumer: the eval authoring rows and the public-benchmark mirror suites.
//! Owner: this module owns the request shape, the adapter choice and the
//! scene executor; the provider crate owns the contract and the playtest
//! `Backend` labels pick the policy. Each backend names the adapter that
//! answers: `Mock` always builds [`MockAdapter`], `Local` builds
//! [`LocalAdapter`] and `Remote` builds [`RemoteAdapter`] when its endpoint env
//! is set (`E2040_LOCAL_URL`, `E2040_REMOTE_URL`), and each falls back to the
//! in-process mock when that env is unset so the default matrix stays offline.
//! Every answer names the adapter that produced it, so a mock answering a
//! `Local` row is reported as `mock`, never as a model. Recorded content (a
//! proposal replayed from a tape) replaces the answered no-op; admission,
//! revision freshness, validation and fallback stay the real contract, so a
//! recorded run needs no model.
//! Units: centimeters on the wire, meters in the scene, integer revisions.
//! Determinism: the same call sequence gives the same outcomes and scene
//! hash; no wall clock or map order is read.
//! Errors: [`DoorRefusal`] for the contract, [`ApplyError`] for the scene;
//! a refused or invalid proposal writes nothing.

use std::path::PathBuf;

use engine_core::Fnv64;
use engine_provider::protocol::{
    AiError, AiRequest, Budget, ContextRef, DataClass, ModelSelector, Proposal, ProposalAction,
    RequestId, TaskKind,
};
use engine_provider::{
    fallback_for, parse_endpoint, validate_proposal, AdapterOutcome, AiSessionPolicy,
    CassetteAdapter, CassetteError, CassetteMode, CassetteStore, LocalAdapter, MockAdapter,
    MockStep, ProviderAdapter, ProviderCapabilities, RemoteAdapter, RequestPins, ScriptedFallback,
    SecretRef, PROTOCOL_VERSION,
};
use engine_spatial::Actor;

use crate::agent::Backend;
use crate::eval::{
    backend_wiring, script_steps, TaskScript, LOCAL_URL_ENV, REMOTE_TOKEN_ENV, REMOTE_URL_ENV,
};
use crate::{reachability, reference_arena, verify, ArenaVariant, Prop};

/// Cassette wiring for one door call: replay mode plus its store directory.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct CassetteSetup {
    /// Record, replay or off (off never touches a file).
    pub(crate) mode: CassetteMode,
    /// Store directory; defaults to the store default.
    pub(crate) dir: PathBuf,
}

/// Pure cassette wiring: mode text through `CassetteMode::parse`; `None`,
/// unknown text or `off` gives `None`; dir defaults to the store default.
pub(crate) fn cassette_setup(mode: Option<&str>, dir: Option<PathBuf>) -> Option<CassetteSetup> {
    let mode = CassetteMode::parse(mode?)?;
    if mode == CassetteMode::Off {
        return None;
    }
    let dir = match dir {
        Some(dir) => dir,
        None => CassetteStore::default_dir(),
    };
    Some(CassetteSetup { mode, dir })
}

/// Env cassette wiring; the only place the cassette env is read.
pub(crate) fn cassette_from_env() -> Option<CassetteSetup> {
    let mode = std::env::var("E2040_CASSETTE").ok();
    let dir = std::env::var("E2040_CASSETTE_DIR").ok().map(PathBuf::from);
    cassette_setup(mode.as_deref(), dir)
}

/// Where the local and remote adapters dial. An unset endpoint keeps that
/// backend on the in-process mock, so the default matrix stays offline and
/// the row says `adapter=mock`. Empty env values count as unset.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub(crate) struct BackendTargets {
    /// Loopback sidecar URL for [`Backend::Local`].
    pub(crate) local_url: Option<String>,
    /// Configured host URL for [`Backend::Remote`].
    pub(crate) remote_url: Option<String>,
    /// Env variable holding the remote bearer token; `None` uses
    /// [`REMOTE_TOKEN_ENV`].
    pub(crate) remote_token_env: Option<String>,
}

impl BackendTargets {
    /// Targets from the operator env; unset or empty endpoints stay mock.
    pub(crate) fn from_env() -> Self {
        Self {
            local_url: non_empty_env(LOCAL_URL_ENV),
            remote_url: non_empty_env(REMOTE_URL_ENV),
            remote_token_env: None,
        }
    }
}

/// One env value, treating an empty string as unset.
fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// The adapter a door call sends to: one variant per backend, so the label
/// in the report names the code that produced the answer.
#[derive(Debug)]
enum DoorAdapter {
    /// In-process deterministic stand-in (also the offline default).
    Mock(MockAdapter),
    /// Loopback llama.cpp-style sidecar over `/v1/chat/completions`.
    Local(LocalAdapter),
    /// Configured remote host, authenticated per call.
    Remote(RemoteAdapter),
}

impl DoorAdapter {
    /// Build the adapter for `backend`. A configured endpoint that does not
    /// parse or is not loopback-local refuses instead of silently falling
    /// back, so a typo cannot turn a live leg into a mock run.
    fn build(
        backend: Backend,
        script: Vec<MockStep>,
        targets: &BackendTargets,
    ) -> Result<Self, AiError> {
        match backend {
            Backend::Mock => Ok(Self::Mock(MockAdapter::new(script))),
            Backend::Local => match targets.local_url.as_deref() {
                None => Ok(Self::Mock(MockAdapter::new(script))),
                Some(url) => {
                    let endpoint =
                        parse_endpoint(url).map_err(|_| AiError::UnsupportedCapability)?;
                    LocalAdapter::new(script, endpoint)
                        .map(Self::Local)
                        .map_err(|_| AiError::UnsupportedCapability)
                }
            },
            Backend::Remote => match targets.remote_url.as_deref() {
                None => Ok(Self::Mock(MockAdapter::new(script))),
                Some(url) => {
                    let endpoint =
                        parse_endpoint(url).map_err(|_| AiError::UnsupportedCapability)?;
                    let token_env = targets
                        .remote_token_env
                        .clone()
                        .unwrap_or_else(|| REMOTE_TOKEN_ENV.to_owned());
                    Ok(Self::Remote(RemoteAdapter::new(
                        script,
                        endpoint,
                        SecretRef::EnvVar(token_env),
                    )))
                }
            },
        }
    }

    /// Label of the adapter that answers: `mock`, `local` or `remote`.
    const fn label(&self) -> &'static str {
        match self {
            Self::Mock(_) => "mock",
            Self::Local(_) => "local",
            Self::Remote(_) => "remote",
        }
    }

    /// Pin the live revision for the shared freshness check.
    fn set_revision(&mut self, revision: u64) {
        match self {
            Self::Mock(adapter) => adapter.set_revision(revision),
            Self::Local(adapter) => adapter.set_revision(revision),
            Self::Remote(adapter) => adapter.set_revision(revision),
        }
    }
}

impl ProviderAdapter for DoorAdapter {
    fn call(
        &mut self,
        request: &AiRequest,
        policy: &AiSessionPolicy,
        capabilities: &ProviderCapabilities,
    ) -> AdapterOutcome {
        match self {
            Self::Mock(adapter) => adapter.call(request, policy, capabilities),
            Self::Local(adapter) => adapter.call(request, policy, capabilities),
            Self::Remote(adapter) => adapter.call(request, policy, capabilities),
        }
    }

    fn request_pins(&self) -> Option<&RequestPins> {
        match self {
            Self::Mock(adapter) => adapter.request_pins(),
            Self::Local(adapter) => adapter.request_pins(),
            Self::Remote(adapter) => adapter.request_pins(),
        }
    }
}

/// One answered call: the validated proposal plus the adapter that answered
/// it. A cassette hit names the adapter that recorded the answer, so replay
/// never claims a model that did not run.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct DoorAnswer {
    /// Validated proposal the door may apply.
    pub(crate) proposal: Proposal,
    /// Adapter label that produced the answer.
    pub(crate) adapter: &'static str,
}

/// Id of the floor actor that stands in for the arena in spatial reports.
pub(crate) const FLOOR_ID: u32 = 1000;
/// Floor slab thickness in meters (top face at y = 0).
const FLOOR_THICKNESS_M: f32 = 0.2;

/// Replayed provider output: the actions and diagnostics a recorded tape holds.
#[derive(Clone, Copy)]
pub(crate) struct Recorded<'a> {
    /// Typed actions to substitute for the mock's no-op.
    pub actions: &'a [ProposalAction],
    /// One-line diagnostics to substitute.
    pub diagnostics: &'a [String],
}

/// One provider-contract call.
pub(crate) struct DoorCall<'a> {
    /// Backend whose policy and capabilities apply.
    pub backend: Backend,
    /// Task slot, carried as the context revision like the eval rows.
    pub index: u64,
    /// Unique request id; a repeated id replays the first outcome.
    pub request_id: u64,
    /// Inspect or propose.
    pub kind: TaskKind,
    /// How the scripted provider behaves for this call.
    pub script: TaskScript,
    /// Revision the proposal was built against.
    pub base_revision: u64,
    /// Live revision the contract checks freshness against, when pinned.
    pub live_revision: Option<u64>,
    /// Replayed content; `None` keeps the mock's no-op proposal.
    pub recorded: Option<Recorded<'a>>,
}

/// Why the door produced no proposal, plus the scripted path when one exists.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct DoorRefusal {
    /// The contract error.
    pub error: AiError,
    /// No-op service for timeouts and refusals; `None` for stale or cancelled.
    pub scripted: Option<ScriptedFallback>,
    /// Typed cassette miss that caused the refusal, if any.
    pub miss: Option<CassetteError>,
    /// Adapter the call was wired to (the one that answered, or the one that
    /// could not be built).
    pub adapter: &'static str,
}

/// Run one call with explicit cassette and endpoint wiring. The adapter that
/// answers is named by the request policy and the labels in the result.
#[expect(
    clippy::result_large_err,
    reason = "a refusal is built once per door call and returned by value"
)]
pub(crate) fn door_call_with_targets(
    call: &DoorCall<'_>,
    cassette: Option<&CassetteSetup>,
    targets: &BackendTargets,
) -> Result<DoorAnswer, DoorRefusal> {
    let probe = matches!(call.script, TaskScript::PrivacyProbe);
    let (adapter_kind, model, privacy, policy, caps) = backend_wiring(call.backend, probe);
    let request = AiRequest {
        id: RequestId(call.request_id),
        kind: call.kind,
        model: ModelSelector {
            adapter: adapter_kind,
            model: model.to_owned(),
        },
        context: vec![ContextRef {
            class: DataClass::Project,
            digest: [3u8; 32],
            revision: call.index,
        }],
        schema_version: PROTOCOL_VERSION,
        budget: Budget {
            max_output_tokens: 512,
            max_ms: 5000,
            spend_limit: None,
        },
        deadline_ms: 60_000,
        privacy,
        base_revision: call.base_revision,
    };
    let refuse = |adapter: &'static str, error: AiError, miss: Option<CassetteError>| DoorRefusal {
        error,
        scripted: fallback_for(error, &request, &policy).ok(),
        miss,
        adapter,
    };
    let (mode, dir) = match cassette {
        Some(setup) => (setup.mode, setup.dir.clone()),
        None => (CassetteMode::Off, CassetteStore::default_dir()),
    };
    let adapter = match DoorAdapter::build(call.backend, script_steps(call.script), targets) {
        Ok(adapter) => adapter,
        Err(error) => return Err(refuse(backend_label(call.backend), error, None)),
    };
    let label = adapter.label();
    let mut adapter = match CassetteAdapter::new(adapter, CassetteStore::new(dir), mode, label) {
        Ok(adapter) => adapter,
        Err(_) => return Err(refuse(label, AiError::UnsupportedCapability, None)),
    };
    if let Some(live) = call.live_revision {
        adapter.inner_mut().set_revision(live);
    }
    let outcome = adapter.call(&request, &policy, &caps);
    let miss = adapter.last_miss();
    let result = outcome.result.map_err(|error| refuse(label, error, miss))?;
    let mut proposal = result
        .proposal
        .ok_or_else(|| refuse(label, AiError::MalformedOutput, None))?;
    if let Some(recorded) = call.recorded {
        proposal.actions = recorded.actions.to_vec();
        proposal.diagnostics = recorded.diagnostics.to_vec();
    }
    validate_proposal(&proposal).map_err(|_| refuse(label, AiError::MalformedOutput, None))?;
    Ok(DoorAnswer {
        proposal,
        adapter: label,
    })
}

/// Why the scene refused a contract-valid proposal. Nothing is written.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ApplyError {
    /// The proposal was built against another revision than the live one.
    Stale {
        /// Revision the proposal carries.
        base: u64,
        /// Revision the scene holds.
        live: u64,
    },
    /// No prop carries this id.
    UnknownObject(u64),
    /// A coordinate lies outside the +-327 m scene range.
    OutOfWorld(u64),
}

/// A variant plus its revision: the state the door edits and hashes.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Scene {
    /// Plain arena data the verifiers read.
    pub variant: ArenaVariant,
    /// Revision counter, bumped once per applied change.
    pub revision: u64,
}

impl Scene {
    /// Fresh scene at revision 1.
    pub(crate) fn new(variant: ArenaVariant) -> Self {
        Self {
            variant,
            revision: 1,
        }
    }

    /// Prop by id.
    pub(crate) fn prop(&self, id: u32) -> Option<&Prop> {
        self.variant.props.iter().find(|prop| prop.id == id)
    }

    /// Apply a validated proposal atomically. Returns whether anything
    /// changed; a changed scene bumps its revision once.
    pub(crate) fn apply(&mut self, proposal: &Proposal) -> Result<bool, ApplyError> {
        if proposal.base_revision != self.revision {
            return Err(ApplyError::Stale {
                base: proposal.base_revision,
                live: self.revision,
            });
        }
        let mut next = self.variant.clone();
        let mut changed = false;
        for action in &proposal.actions {
            match action {
                ProposalAction::Noop => {}
                ProposalAction::SetPosition { object, xyz_cm } => {
                    let prop = prop_mut(&mut next, *object)?;
                    prop.x = metres(xyz_cm[0], *object)?;
                    prop.y = metres(xyz_cm[1], *object)?;
                    prop.z = metres(xyz_cm[2], *object)?;
                    changed = true;
                }
                ProposalAction::SetLabel { object, label } => {
                    prop_mut(&mut next, *object)?.name.clone_from(label);
                    changed = true;
                }
            }
        }
        if changed {
            self.variant = next;
            self.revision += 1;
        }
        Ok(changed)
    }

    /// Floor slab plus every prop as spatial actors, in prop order.
    pub(crate) fn actors(&self) -> Vec<Actor> {
        let side = self.variant.half_extent_m * 2.0;
        let mut out = vec![Actor {
            id: FLOOR_ID,
            name: "arena".to_owned(),
            x: 0.0,
            y: -FLOOR_THICKNESS_M / 2.0,
            z: 0.0,
            sx: side,
            sy: FLOOR_THICKNESS_M,
            sz: side,
            parent: None,
            physical: false,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        }];
        out.extend(self.variant.props.iter().map(|prop| Actor {
            id: prop.id,
            name: prop.name.clone(),
            x: prop.x,
            y: prop.y,
            z: prop.z,
            sx: prop.sx,
            sy: prop.sy,
            sz: prop.sz,
            parent: None,
            physical: prop.physical,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        }));
        out
    }

    /// Order-independent digest of revision, extent, props, spawns and gates.
    pub(crate) fn hash(&self) -> u64 {
        // Props sort by id so input order never moves the digest.
        let mut hash = Fnv64::new();
        hash.write(b"door-scene:1");
        hash.write_u64(self.revision);
        hash.write_f32(self.variant.half_extent_m);
        let mut props: Vec<&Prop> = self.variant.props.iter().collect();
        props.sort_by_key(|prop| prop.id);
        hash.write_u64(u64::try_from(props.len()).unwrap_or(u64::MAX));
        for prop in props {
            hash.write_u32(prop.id);
            hash.write_str(&prop.name);
            for value in [prop.x, prop.y, prop.z, prop.sx, prop.sy, prop.sz] {
                hash.write_f32(value);
            }
            hash.write_bool(prop.physical);
        }
        for spawn in &self.variant.spawns {
            hash.write_u32(spawn.id);
            hash.write_f32(spawn.x);
            hash.write_f32(spawn.z);
        }
        for gate in &self.variant.gates {
            hash.write_u32(gate.id);
            hash.write_f32(gate.x);
            hash.write_f32(gate.z);
        }
        hash.finish()
    }

    /// The scene after `actions` applied directly, bypassing the provider
    /// contract: the oracle a door run is compared against.
    pub(crate) fn direct(&self, actions: &[ProposalAction]) -> Option<Self> {
        let proposal = Proposal {
            request: RequestId(0),
            base_revision: self.revision,
            actions: actions.to_vec(),
            object_versions: Vec::new(),
            diagnostics: Vec::new(),
        };
        let mut next = self.clone();
        next.apply(&proposal).ok().map(|_| next)
    }

    /// True when every prop footprint lies inside the arena square. The
    /// spatial bounds check is world-scale (+-327 m), so it cannot see an
    /// arena edge; this one uses the arena's own half extent.
    pub(crate) fn inside_arena(&self) -> bool {
        let half = self.variant.half_extent_m;
        self.variant.props.iter().all(|prop| {
            prop.x.abs() + prop.sx / 2.0 <= half && prop.z.abs() + prop.sz / 2.0 <= half
        })
    }

    /// Spatial findings of one check between two props. Pairs that touch
    /// the floor slab are left out: the reference arena's own props rest on
    /// it, so those pairs are the fixture's, not the edit's.
    pub(crate) fn prop_findings(&self, check: &str) -> usize {
        engine_spatial::run(&self.actors())
            .findings
            .iter()
            .filter(|finding| {
                finding.check == check && finding.a != FLOOR_ID && finding.b != FLOOR_ID
            })
            .count()
    }

    /// True when every spawn reaches every gate around the physical props.
    pub(crate) fn walkable(&self) -> bool {
        reachability::check(&self.variant).is_empty()
    }

    /// True when every verifier (reachability, win rates, budgets) passes.
    pub(crate) fn verified(&self) -> bool {
        verify(&self.variant).pass
    }

    /// Gap in meters between a prop's north edge and the arena's north wall;
    /// negative when the prop pokes through the wall.
    pub(crate) fn north_gap_m(&self, id: u32) -> Option<f32> {
        let prop = self.prop(id)?;
        Some((prop.z - prop.sz / 2.0) + self.variant.half_extent_m)
    }
}

/// Reference arena with a fast balance config (two 60-tick matches, open
/// win band) so a full verify stays cheap.
pub(crate) fn fixture_scene() -> Scene {
    let mut variant = reference_arena();
    variant.balance.matches = 2;
    variant.balance.match_ticks = 60;
    variant.balance.min_win_permille = 0;
    variant.balance.max_win_permille = 1000;
    Scene::new(variant)
}

/// A physical wall-like prop standing on the floor, sizes in meters.
pub(crate) fn block_prop(id: u32, name: &str, center: [f32; 2], size: [f32; 3]) -> Prop {
    Prop {
        id,
        name: name.to_owned(),
        x: center[0],
        y: size[1] / 2.0,
        z: center[1],
        sx: size[0],
        sy: size[1],
        sz: size[2],
        physical: true,
        triangles: 12,
        memory_bytes: 1024,
    }
}

/// Move one prop to a centimeter position.
pub(crate) fn set_position(object: u64, xyz_cm: [i32; 3]) -> ProposalAction {
    ProposalAction::SetPosition { object, xyz_cm }
}

/// Rename one prop.
pub(crate) fn set_label(object: u64, label: &str) -> ProposalAction {
    ProposalAction::SetLabel {
        object,
        label: label.to_owned(),
    }
}

/// Mutable prop by wire id.
fn prop_mut(variant: &mut ArenaVariant, object: u64) -> Result<&mut Prop, ApplyError> {
    let id = u32::try_from(object).map_err(|_| ApplyError::UnknownObject(object))?;
    variant
        .props
        .iter_mut()
        .find(|prop| prop.id == id)
        .ok_or(ApplyError::UnknownObject(object))
}

/// Centimeters to meters inside the scene range.
fn metres(cm: i32, object: u64) -> Result<f32, ApplyError> {
    let cm = i16::try_from(cm).map_err(|_| ApplyError::OutOfWorld(object))?;
    Ok(f32::from(cm) / 100.0)
}

/// What one submission did to the scene.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum Delivery {
    /// The proposal applied; `true` when the scene changed.
    Applied(bool),
    /// The scripted no-op served after a timeout or refusal; label named.
    Fallback(&'static str),
    /// The contract refused (stale or cancelled); nothing served.
    Refused(AiError),
    /// The scene refused a contract-valid proposal.
    Rejected(ApplyError),
}

impl Delivery {
    /// Short stable text for report details.
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Applied(true) => "applied".to_owned(),
            Self::Applied(false) => "noop".to_owned(),
            Self::Fallback(cause) => format!("fallback:{cause}"),
            Self::Refused(error) => format!("refused:{error:?}"),
            Self::Rejected(error) => format!("rejected:{error:?}"),
        }
    }
}

/// One submission to the door.
pub(crate) struct Submit<'a> {
    /// Provider behavior for the call.
    pub script: TaskScript,
    /// Revision the proposal was built against.
    pub base_revision: u64,
    /// Recorded actions to replay.
    pub actions: &'a [ProposalAction],
    /// Recorded diagnostics to replay.
    pub diagnostics: &'a [String],
}

impl<'a> Submit<'a> {
    /// A clean provider call replaying `actions` against `base_revision`.
    pub(crate) fn clean(base_revision: u64, actions: &'a [ProposalAction]) -> Self {
        Self {
            script: TaskScript::Clean,
            base_revision,
            actions,
            diagnostics: &[],
        }
    }
}

/// The kind of a logged door call.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CallKind {
    /// A read of the scene.
    Read,
    /// A submitted write.
    Write,
}

/// One call the door executor saw, in order.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum Call {
    /// A read of the scene; `ids` are the object ids it returned.
    Read {
        /// Object ids the read handed back.
        ids: Vec<u64>,
    },
    /// A submitted write; `ids` are the object ids its actions named.
    Write {
        /// Object ids the actions named.
        ids: Vec<u64>,
    },
}

impl Call {
    /// Read or write.
    pub(crate) fn kind(&self) -> CallKind {
        match self {
            Self::Read { .. } => CallKind::Read,
            Self::Write { .. } => CallKind::Write,
        }
    }
}

/// A door bound to one backend and task slot, issuing unique request ids and
/// logging every call with the object ids it returned or named. The door owns
/// offline targets by default; [`Door::with_targets`] names live endpoints.
pub(crate) struct Door {
    backend: Backend,
    index: u64,
    calls: u64,
    log: Vec<Call>,
    targets: BackendTargets,
}

impl Door {
    /// Door for one task slot on one backend, offline by default: the mock
    /// answers unless test wiring names endpoints.
    pub(crate) fn new(backend: Backend, index: u64) -> Self {
        Self {
            backend,
            index,
            calls: 0,
            log: Vec::new(),
            targets: BackendTargets::default(),
        }
    }

    /// Route this door's local and remote rows at `targets`. Test-only today:
    /// the eval rows call [`door_call_with_targets`] directly.
    #[cfg(test)]
    pub(crate) fn with_targets(mut self, targets: BackendTargets) -> Self {
        self.targets = targets;
        self
    }

    /// Read the scene: the id of every prop, logged as one read call.
    pub(crate) fn read(&mut self, scene: &Scene) -> Vec<u64> {
        let ids: Vec<u64> = scene
            .variant
            .props
            .iter()
            .map(|prop| u64::from(prop.id))
            .collect();
        self.log.push(Call::Read { ids: ids.clone() });
        ids
    }

    /// Every call so far, in order.
    pub(crate) fn log(&self) -> &[Call] {
        &self.log
    }

    /// Call the provider contract, then apply what it admits to `scene`.
    /// The scene's live revision is pinned, so a stale proposal is refused
    /// by the contract before any apply.
    pub(crate) fn submit(&mut self, scene: &mut Scene, submit: &Submit<'_>) -> Delivery {
        self.calls += 1;
        let ids = submit
            .actions
            .iter()
            .filter_map(|action| match action {
                ProposalAction::SetPosition { object, .. }
                | ProposalAction::SetLabel { object, .. } => Some(*object),
                ProposalAction::Noop => None,
            })
            .collect();
        self.log.push(Call::Write { ids });
        let call = DoorCall {
            backend: self.backend,
            index: self.index,
            request_id: 10_000 + self.index * 100 + self.calls * 10 + backend_slot(self.backend),
            kind: TaskKind::Propose,
            script: submit.script,
            base_revision: submit.base_revision,
            live_revision: Some(scene.revision),
            recorded: Some(Recorded {
                actions: submit.actions,
                diagnostics: submit.diagnostics,
            }),
        };
        match door_call_with_targets(&call, cassette_from_env().as_ref(), &self.targets) {
            Ok(answer) => match scene.apply(&answer.proposal) {
                Ok(changed) => Delivery::Applied(changed),
                Err(error) => Delivery::Rejected(error),
            },
            Err(DoorRefusal {
                scripted: Some(served),
                ..
            }) => match scene.apply(&served.proposal) {
                Ok(_) => Delivery::Fallback(served.cause.label()),
                Err(error) => Delivery::Rejected(error),
            },
            Err(DoorRefusal { error, .. }) => Delivery::Refused(error),
        }
    }
}

/// Distinct request-id digit per backend.
const fn backend_slot(backend: Backend) -> u64 {
    match backend {
        Backend::Mock => 0,
        Backend::Local => 1,
        Backend::Remote => 2,
    }
}

/// Cassette file label per backend, beside the request-id slot.
const fn backend_label(backend: Backend) -> &'static str {
    match backend {
        Backend::Mock => "mock",
        Backend::Local => "local",
        Backend::Remote => "remote",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::BACKENDS;
    use crate::reference_arena;

    fn move_prop(object: u64, xyz_cm: [i32; 3]) -> ProposalAction {
        ProposalAction::SetPosition { object, xyz_cm }
    }

    fn submit<'a>(base: u64, actions: &'a [ProposalAction]) -> Submit<'a> {
        Submit {
            script: TaskScript::Clean,
            base_revision: base,
            actions,
            diagnostics: &[],
        }
    }

    #[test]
    fn recorded_actions_replace_the_mock_noop_and_move_the_prop() {
        let mut scene = Scene::new(reference_arena());
        let mut door = Door::new(Backend::Mock, 0);
        let actions = [move_prop(20, [-1930, 50, 700])];
        let delivery = door.submit(&mut scene, &submit(1, &actions));
        assert_eq!(delivery, Delivery::Applied(true));
        let prop = scene.prop(20).expect("prop 20 exists");
        assert!((prop.x + 19.3).abs() < 1e-4 && (prop.z - 7.0).abs() < 1e-4);
        assert_eq!(scene.revision, 2, "one applied change bumps once");
    }

    #[test]
    fn with_targets_routes_the_door_to_the_named_adapter() {
        let mut scene = Scene::new(reference_arena());
        // A non-loopback sidecar is refused at build time, so this proves the
        // builder choice reached the call without dialing anything.
        let targets = BackendTargets {
            local_url: Some(String::from("http://203.0.113.7:8080/v1/chat/completions")),
            remote_url: None,
            remote_token_env: None,
        };
        let mut door = Door::new(Backend::Local, 5).with_targets(targets);
        let actions = [move_prop(20, [0, 50, 0])];
        assert_eq!(
            door.submit(&mut scene, &submit(1, &actions)),
            Delivery::Fallback("refused"),
            "a refused local route serves the scripted path"
        );
        assert_eq!(scene.revision, 1, "a refused route writes nothing");
    }

    #[test]
    fn stale_base_revision_is_refused_by_the_contract_and_writes_nothing() {
        let mut scene = Scene::new(reference_arena());
        let before = scene.hash();
        let mut door = Door::new(Backend::Remote, 3);
        let actions = [move_prop(20, [0, 50, 0])];
        let delivery = door.submit(&mut scene, &submit(0, &actions));
        assert_eq!(delivery, Delivery::Refused(AiError::StaleContext));
        assert_eq!(scene.hash(), before, "a refused proposal leaves the hash");
    }

    #[test]
    fn a_batch_with_one_bad_object_applies_nothing() {
        let mut scene = Scene::new(reference_arena());
        let before = scene.hash();
        let mut door = Door::new(Backend::Local, 1);
        let actions = [move_prop(20, [100, 50, 100]), move_prop(999, [0, 0, 0])];
        let delivery = door.submit(&mut scene, &submit(1, &actions));
        assert_eq!(
            delivery,
            Delivery::Rejected(ApplyError::UnknownObject(999)),
            "unknown ids name the culprit"
        );
        assert_eq!(scene.hash(), before, "the first action must roll back too");
        assert_eq!(scene.revision, 1);
    }

    #[test]
    fn scene_refuses_a_stale_proposal_even_without_the_contract_pin() {
        let mut scene = Scene::new(reference_arena());
        let proposal = Proposal {
            request: RequestId(1),
            base_revision: 7,
            actions: vec![ProposalAction::Noop],
            object_versions: Vec::new(),
            diagnostics: Vec::new(),
        };
        assert_eq!(
            scene.apply(&proposal),
            Err(ApplyError::Stale { base: 7, live: 1 })
        );
    }

    #[test]
    fn out_of_range_coordinates_are_refused_not_wrapped() {
        let mut scene = Scene::new(reference_arena());
        let mut door = Door::new(Backend::Mock, 2);
        let actions = [move_prop(20, [40_000, 50, 0])];
        let delivery = door.submit(&mut scene, &submit(1, &actions));
        assert_eq!(delivery, Delivery::Rejected(ApplyError::OutOfWorld(20)));
    }

    #[test]
    fn oversized_batches_fail_validation_even_when_the_provider_succeeds() {
        let mut scene = Scene::new(reference_arena());
        let before = scene.hash();
        let mut door = Door::new(Backend::Mock, 4);
        let actions = vec![ProposalAction::Noop; engine_provider::MAX_ACTIONS + 1];
        let delivery = door.submit(&mut scene, &submit(1, &actions));
        assert!(
            matches!(delivery, Delivery::Fallback("refused")),
            "malformed output takes the scripted no-op: {delivery:?}"
        );
        assert_eq!(scene.hash(), before);
    }

    #[test]
    fn timeouts_serve_the_scripted_noop_and_leave_the_scene() {
        for backend in BACKENDS {
            let mut scene = Scene::new(reference_arena());
            let before = scene.hash();
            let mut door = Door::new(backend, 5);
            let actions = [move_prop(20, [0, 50, 0])];
            let delivery = door.submit(
                &mut scene,
                &Submit {
                    script: TaskScript::Timeout,
                    ..submit(1, &actions)
                },
            );
            assert_eq!(delivery, Delivery::Fallback("timeout"), "{backend:?}");
            assert_eq!(scene.hash(), before);
        }
    }

    #[test]
    fn every_backend_applies_the_same_recorded_change_to_the_same_hash() {
        let actions = [
            move_prop(21, [-500, 50, 900]),
            ProposalAction::SetLabel {
                object: 22,
                label: "lantern".to_owned(),
            },
        ];
        let hashes: Vec<u64> = BACKENDS
            .iter()
            .map(|backend| {
                let mut scene = Scene::new(reference_arena());
                let mut door = Door::new(*backend, 6);
                door.submit(&mut scene, &submit(1, &actions));
                scene.hash()
            })
            .collect();
        assert!(
            hashes.windows(2).all(|pair| pair[0] == pair[1]),
            "{hashes:?}"
        );
        assert_ne!(
            hashes[0],
            Scene::new(reference_arena()).hash(),
            "the change must move the hash"
        );
    }

    #[test]
    fn scene_hash_ignores_prop_order_but_not_position_or_label() {
        let base = Scene::new(reference_arena());
        let mut reversed = base.clone();
        reversed.variant.props.reverse();
        assert_eq!(base.hash(), reversed.hash());
        let mut moved = base.clone();
        moved.variant.props[0].x += 0.01;
        assert_ne!(base.hash(), moved.hash());
        let mut renamed = base.clone();
        renamed.variant.props[0].name.push('x');
        assert_ne!(base.hash(), renamed.hash());
    }

    fn cassette_temp_dir(suffix: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("e2040-door-{}-{suffix}", std::process::id()))
    }

    fn cassette_call(
        request_id: u64,
        index: u64,
        base: u64,
        script: TaskScript,
    ) -> DoorCall<'static> {
        DoorCall {
            backend: Backend::Mock,
            index,
            request_id,
            kind: TaskKind::Propose,
            script,
            base_revision: base,
            live_revision: None,
            recorded: None,
        }
    }

    #[test]
    fn recorded_actions_replay_from_a_cassette_without_the_provider() {
        let dir = cassette_temp_dir("replay");
        let _ = std::fs::remove_dir_all(&dir);
        let request_id = 4241u64;
        let index = 41u64;
        let base = 7u64;
        let record_setup =
            cassette_setup(Some("record"), Some(dir.clone())).expect("record setup parses");
        let recorded = door_call_with_targets(
            &cassette_call(request_id, index, base, TaskScript::Clean),
            Some(&record_setup),
            &BackendTargets::default(),
        )
        .expect("record answers live");
        let replay_setup =
            cassette_setup(Some("replay"), Some(dir.clone())).expect("replay setup parses");
        // Timeout would fail live; replay must still serve the record.
        let replay_call = cassette_call(request_id, index, base, TaskScript::Timeout);
        let first = door_call_with_targets(
            &replay_call,
            Some(&replay_setup),
            &BackendTargets::default(),
        )
        .expect("replay answers");
        assert_eq!(first, recorded, "replay serves the recorded proposal");
        let second = door_call_with_targets(
            &replay_call,
            Some(&replay_setup),
            &BackendTargets::default(),
        )
        .expect("second replay");
        assert_eq!(second, recorded, "second replay stays identical");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recorded_actions_cassette_miss_says_why() {
        let dir = cassette_temp_dir("miss");
        let _ = std::fs::remove_dir_all(&dir);
        let empty_setup =
            cassette_setup(Some("replay"), Some(dir.clone())).expect("replay setup parses");
        let absent = door_call_with_targets(
            &cassette_call(4343, 43, 9, TaskScript::Clean),
            Some(&empty_setup),
            &BackendTargets::default(),
        )
        .expect_err("empty store must miss");
        assert_eq!(absent.error, AiError::UnsupportedCapability);
        assert!(absent.miss.is_some(), "empty replay names the miss");
        let record_setup =
            cassette_setup(Some("record"), Some(dir.clone())).expect("record setup parses");
        let _ = door_call_with_targets(
            &cassette_call(4343, 43, 9, TaskScript::Clean),
            Some(&record_setup),
            &BackendTargets::default(),
        )
        .expect("record answers live");
        let replay_setup =
            cassette_setup(Some("replay"), Some(dir.clone())).expect("replay setup parses");
        let changed = door_call_with_targets(
            &cassette_call(4343, 44, 10, TaskScript::Clean),
            Some(&replay_setup),
            &BackendTargets::default(),
        )
        .expect_err("changed index must miss");
        assert_eq!(changed.error, AiError::UnsupportedCapability);
        assert!(changed.miss.is_some(), "changed replay names the miss");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recorded_actions_cassette_setup_parses_modes() {
        assert!(cassette_setup(None, None).is_none(), "no mode is off");
        assert!(cassette_setup(Some("off"), None).is_none(), "off is off");
        assert!(
            cassette_setup(Some("bogus"), None).is_none(),
            "unknown is off"
        );
        let dir = cassette_temp_dir("modes");
        let record = cassette_setup(Some("record"), Some(dir.clone())).expect("record parses");
        assert_eq!(record.mode, CassetteMode::Record);
        assert_eq!(record.dir, dir);
        let replay = cassette_setup(Some("replay"), Some(dir.clone())).expect("replay parses");
        assert_eq!(replay.mode, CassetteMode::Replay);
        assert_eq!(replay.dir, dir);
        let defaulted = cassette_setup(Some("record"), None).expect("dir defaults");
        assert_eq!(defaulted.dir, CassetteStore::default_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recorded_actions_cassette_off_touches_no_file() {
        let dir = cassette_temp_dir("off");
        let _ = std::fs::remove_dir_all(&dir);
        let call = cassette_call(4444, 45, 11, TaskScript::Clean);
        let without =
            door_call_with_targets(&call, None, &BackendTargets::default()).expect("off answers");
        assert!(!dir.exists(), "off touches no file");
        let off = CassetteSetup {
            mode: CassetteMode::Off,
            dir: dir.clone(),
        };
        let explicit = door_call_with_targets(&call, Some(&off), &BackendTargets::default())
            .expect("explicit off answers");
        assert_eq!(explicit, without, "None equals the off path");
        assert!(!dir.exists(), "explicit off still touches no file");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
