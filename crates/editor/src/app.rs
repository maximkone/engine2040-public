//! Editor executable: winit window, egui dock, render viewport, child-process play.
//!
//! The consumer is the operator editor session (the spatial, tuning and
//! template surfaces build on this app). The data owner is [`EditorApp`]: the
//! [`Shell`] document plus journal, the [`ShellUi`] dock, the live
//! [`ViewportImage`] texture, one
//! optional snapshot [`Presenter`](engine_render::present::Presenter) and
//! at most one supervised [`ChildPlay`] game process. Units are pixels for
//! sizes and textures, ticks for the frame counter. Every failure is a typed
//! [`EditorAppError`] with a [`Recovery`] action; a bad project, a missing
//! arena binary or a missing GPU never panics and never loses the document.
//!
//! Frame flow per redraw: take egui input, refresh the live viewport texture
//! when the shell revision or panel size moved, run the dock (top menu plus
//! [`ShellUi`]), tessellate, paint through `egui-wgpu`, present. The live
//! texture is CPU-rasterized ([`ViewportImage`]) and render-validated; the
//! GPU [`Presenter`](engine_render::present::Presenter) renders draw lists
//! derived from the same document offscreen into PNG receipts on demand
//! (Snapshot button, `--snapshot`). Surface-acquire outcomes in the window
//! loop map through the renderer's [`surface_action`](engine_render::present::surface_action),
//! so Outdated/Lost reconfigure while Timeout/Occluded skip the frame.
//!
//! This module is behind the `ui` feature so headless builds stay std-only.
//! Windowed runs and reference-GPU captures stay operator-verified (see the
//! editor report); headless tests below cover args, project files, draw lists,
//! error recovery and dock frames without a window or GPU.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui_wgpu::wgpu;
use engine_core::{
    hash::{FNV64_OFFSET, GOLDEN_GAMMA},
    Fnv64,
};
use engine_render::present::{
    self, DrawList, MeshKey, Presenter, RecoveryAction, SizeClass, SurfaceEvent,
};
use engine_render::v1;
use engine_scene::ids::ObjectId;
use engine_scene::transform::Transform;

use crate::a2_session::encode_png_rgba8;
use crate::commands::Document;
use crate::durable::write_atomic;
use crate::play::{ChildPlay, PlayEntrypoint};
use crate::save::{load_from_bytes, save_to_bytes, MAX_SAVE_BYTES};
use crate::shell::Shell;
use crate::shell_ui::ShellUi;
use crate::viewport_image::{ViewportImage, MAX_VIEWPORT_TEX};

/// Default project file next to the working directory.
pub const DEFAULT_PROJECT: &str = "scene.edoc";
/// Default arena program: the windowed game binary; override with `--arena-bin`.
pub const DEFAULT_ARENA_BIN: &str = "e2040-arena";
/// Default window width in pixels.
pub const DEFAULT_WIDTH: u32 = 1280;
/// Default window height in pixels.
pub const DEFAULT_HEIGHT: u32 = 800;
/// CLI help printed for `--help`.
pub const HELP: &str = "e2040-editor [--project PATH] [--arena-bin PROG] [--arena-arg ARG]... [--width PX] [--height PX] [--snapshot PATH] [--exit-frames N]";

// ---------------------------------------------------------------------------
// Errors: typed, with a recovery action. Nothing here panics.
// ---------------------------------------------------------------------------

/// What the operator (or a script) should do after an [`EditorAppError`].
/// Mirrors the renderer's recovery vocabulary so surface and snapshot failures map 1:1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recovery {
    /// Fix the input (path, size, draw list, command line) and retry.
    FixCallerInput,
    /// Retry the same frame (transient GPU state).
    RetryFrame,
    /// Reconfigure the surface/swapchain, keeping last-good state.
    ReconfigureSurface,
    /// Keep rendering with the last-good texture or pipeline.
    KeepLastGood,
    /// Skip this frame; try again next frame.
    SkipFrame,
    /// Drop the GPU presenter and build a new one (adapter gone).
    ReacquireDevice,
}

impl From<RecoveryAction> for Recovery {
    fn from(action: RecoveryAction) -> Self {
        match action {
            RecoveryAction::RetryFrame => Self::RetryFrame,
            RecoveryAction::ReconfigureSurface => Self::ReconfigureSurface,
            RecoveryAction::ReacquireDevice => Self::ReacquireDevice,
            RecoveryAction::KeepLastGood => Self::KeepLastGood,
            RecoveryAction::SkipFrame => Self::SkipFrame,
            RecoveryAction::FixCallerInput => Self::FixCallerInput,
        }
    }
}

/// Every way the editor app can fail, with a recovery for each.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditorAppError {
    /// CLI flags did not parse.
    BadArgs(String),
    /// The project file is missing, oversize or undecodable.
    BadProject {
        /// File that failed to open.
        path: PathBuf,
        /// Why it failed (IO text or the save-codec report).
        reason: String,
    },
    /// Saving the project file failed.
    SaveFailed {
        /// File that failed to write.
        path: PathBuf,
        /// IO text.
        reason: String,
    },
    /// A window or texture size was rejected (never zero-area: that suspends).
    BadSize {
        /// Requested width in pixels.
        width: u32,
        /// Requested height in pixels.
        height: u32,
    },
    /// Zero-area viewport (minimized): keep the last-good texture.
    ViewportSuspended,
    /// The viewport draw list or default scene was unusable.
    ViewportFailed(String),
    /// No wgpu adapter: live texture continues, snapshots report this.
    NoAdapter,
    /// A snapshot failed after the adapter was found.
    SnapshotFailed {
        /// Presenter report.
        reason: String,
        /// Mapped renderer recovery.
        recovery: Recovery,
    },
    /// The winit window or event loop failed.
    WindowFailed(String),
    /// The arena child failed to spawn.
    ChildFailed {
        /// Command line attempted.
        command: String,
        /// OS report.
        reason: String,
    },
}

impl EditorAppError {
    /// The caller action that recovers from this error.
    pub fn recovery(&self) -> Recovery {
        match self {
            Self::BadArgs(_)
            | Self::BadProject { .. }
            | Self::SaveFailed { .. }
            | Self::BadSize { .. }
            | Self::ViewportFailed(_)
            | Self::WindowFailed(_)
            | Self::ChildFailed { .. } => Recovery::FixCallerInput,
            Self::ViewportSuspended | Self::NoAdapter => Recovery::KeepLastGood,
            Self::SnapshotFailed { recovery, .. } => *recovery,
        }
    }

    /// Map a renderer presenter failure onto the editor error (and its recovery).
    /// `NoAdapter` and `InvalidSize` keep their own variants so callers can
    /// match them; everything else is a snapshot failure with the renderer's recovery.
    fn snapshot(error: present::PresentError) -> Self {
        match error {
            present::PresentError::NoAdapter => Self::NoAdapter,
            present::PresentError::InvalidSize { width, height } => Self::BadSize { width, height },
            other => Self::SnapshotFailed {
                reason: other.to_string(),
                recovery: Recovery::from(other.recovery()),
            },
        }
    }

    /// Map a live-panel raster failure onto the editor error. Suspension and
    /// bad sizes keep their own variants; a corrupt marker row is a viewport
    /// failure naming the row, so the caller fixes the document and retries.
    fn viewport_image(error: crate::viewport_image::ViewportImageError) -> Self {
        use crate::viewport_image::ViewportImageError as ImageError;
        match error {
            ImageError::Suspended => Self::ViewportSuspended,
            ImageError::BadSize { width, height } => Self::BadSize { width, height },
            ImageError::Hierarchy(error) => Self::ViewportFailed(error.to_string()),
        }
    }
}

impl fmt::Display for EditorAppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadArgs(reason) => write!(f, "bad editor args: {reason}"),
            Self::BadProject { path, reason } => {
                write!(f, "bad project {}: {reason}", path.display())
            }
            Self::SaveFailed { path, reason } => {
                write!(f, "save failed {}: {reason}", path.display())
            }
            Self::BadSize { width, height } => {
                write!(f, "editor size {width}x{height} rejected")
            }
            Self::ViewportSuspended => f.write_str("viewport suspended: keep last-good"),
            Self::ViewportFailed(reason) => write!(f, "viewport failed: {reason}"),
            Self::NoAdapter => f.write_str("no wgpu adapter: live texture continues"),
            Self::SnapshotFailed { reason, .. } => write!(f, "viewport snapshot failed: {reason}"),
            Self::WindowFailed(reason) => write!(f, "editor window failed: {reason}"),
            Self::ChildFailed { command, reason } => {
                write!(f, "arena child `{command}` failed: {reason}")
            }
        }
    }
}

impl std::error::Error for EditorAppError {}

// ---------------------------------------------------------------------------
// Setup future driver: std-only, one-time GPU setup only, never per-frame.
// ---------------------------------------------------------------------------

/// Drive a GPU-setup future to completion on the calling thread.
///
/// Spins polling with a no-op waker: wgpu resolves adapter/device requests on
/// its own threads, so readiness is observed without parking. Used only for
/// one-time window setup, never on the frame path.
fn block_on<F>(future: F) -> F::Output
where
    F: std::future::Future,
{
    use std::task::{Context, Poll, Waker};
    let mut cx = Context::from_waker(Waker::noop());
    let mut pinned = std::pin::pin!(future);
    loop {
        match pinned.as_mut().poll(&mut cx) {
            Poll::Ready(out) => return out,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

// ---------------------------------------------------------------------------
// Config: CLI args plus validation. Units: pixels for sizes.
// ---------------------------------------------------------------------------

/// Editor startup config: project file, arena child, window size, snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorConfig {
    /// Project file (`.edoc` save bytes). Missing starts an empty scene.
    pub project: PathBuf,
    /// Arena binary plus args launched by Play.
    pub arena: PlayEntrypoint,
    /// Window width in pixels.
    pub width: u32,
    /// Window height in pixels.
    pub height: u32,
    /// Where Snapshot writes its PNG receipt (default: `<project>.png`).
    pub snapshot: Option<PathBuf>,
    /// Exit after this many frames (Xvfb smoke); None runs until closed.
    pub exit_frames: Option<u64>,
}

impl EditorConfig {
    /// Defaults: `scene.edoc`, the game binary, 1280x800, run until closed.
    pub fn new() -> Self {
        Self {
            project: PathBuf::from(DEFAULT_PROJECT),
            arena: PlayEntrypoint::new(DEFAULT_ARENA_BIN, Vec::<String>::new()),
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            snapshot: None,
            exit_frames: None,
        }
    }

    /// Validate sizes: only the renderer's `Sizes` passes (zero area suspends the
    /// viewport, never the window; over-bound is rejected, never clamped).
    pub fn validate(&self) -> Result<(), EditorAppError> {
        match SizeClass::classify(self.width, self.height) {
            SizeClass::Sizes => Ok(()),
            _ => Err(EditorAppError::BadSize {
                width: self.width,
                height: self.height,
            }),
        }
    }

    /// Snapshot output path: explicit `--snapshot`, else `<project>.png`.
    pub fn snapshot_path(&self) -> PathBuf {
        if let Some(path) = &self.snapshot {
            return path.clone();
        }
        let mut out = self.project.as_os_str().to_owned();
        out.push(".png");
        PathBuf::from(out)
    }
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse CLI args (`argv[0]` skipped). Err on unknown flags, positionals or
/// bad numbers; `--help` text lives in [`HELP`] (the bin prints it).
pub fn parse_args(args: &[String]) -> Result<EditorConfig, EditorAppError> {
    let mut config = EditorConfig::new();
    let mut arena_args: Vec<String> = Vec::new();
    let mut iter = args.iter().skip(1).peekable();
    while let Some(flag) = iter.next() {
        match flag.as_str() {
            "--project" => {
                let value = take_value(&mut iter, "--project")?;
                config.project = PathBuf::from(value);
            }
            "--arena-bin" => {
                let value = take_value(&mut iter, "--arena-bin")?;
                config.arena.program = PathBuf::from(value);
            }
            "--arena-arg" => arena_args.push(take_value(&mut iter, "--arena-arg")?),
            "--width" => {
                config.width = take_number(&mut iter, "--width")?;
            }
            "--height" => {
                config.height = take_number(&mut iter, "--height")?;
            }
            "--snapshot" => {
                config.snapshot = Some(PathBuf::from(take_value(&mut iter, "--snapshot")?));
            }
            "--exit-frames" => {
                config.exit_frames = Some(take_number(&mut iter, "--exit-frames")?);
            }
            "--help" => {
                return Err(EditorAppError::BadArgs(HELP.to_string()));
            }
            other => {
                return Err(EditorAppError::BadArgs(format!(
                    "unknown flag `{other}` (see --help)"
                )));
            }
        }
    }
    config.arena.args = arena_args;
    config.validate()?;
    Ok(config)
}

/// Next CLI token for a flag that needs a value.
fn take_value(
    iter: &mut std::iter::Peekable<impl Iterator<Item = impl AsRef<str>>>,
    flag: &str,
) -> Result<String, EditorAppError> {
    iter.next()
        .map(|v| v.as_ref().to_string())
        .ok_or_else(|| EditorAppError::BadArgs(format!("flag `{flag}` needs a value (see --help)")))
}

/// Next CLI token parsed as a number.
fn take_number<N>(
    iter: &mut std::iter::Peekable<impl Iterator<Item = impl AsRef<str>>>,
    flag: &str,
) -> Result<N, EditorAppError>
where
    N: std::str::FromStr,
{
    let text = take_value(iter, flag)?;
    text.parse()
        .map_err(|_| EditorAppError::BadArgs(format!("flag `{flag}` needs a number, got `{text}`")))
}

// ---------------------------------------------------------------------------
// Project files: `.edoc` save bytes plus fs. The codec stays pure in `save`.
// ---------------------------------------------------------------------------

/// Save the document to `path` (`.edoc` bytes), atomically: tmp plus rename,
/// so a crash mid-save leaves the prior file, never a truncated one. Returns
/// bytes written.
pub fn save_project(doc: &Document, path: &Path) -> Result<u64, EditorAppError> {
    let bytes = save_to_bytes(doc);
    write_atomic(path, &bytes).map_err(|error| EditorAppError::SaveFailed {
        path: path.to_path_buf(),
        reason: error.to_string(),
    })?;
    Ok(bytes.len() as u64)
}

/// Reopen the document at `path`. Oversize files fail before reading; codec
/// failures name the culprit node. Never panics on any input.
pub fn load_project(path: &Path) -> Result<Document, EditorAppError> {
    let bad = |reason: String| EditorAppError::BadProject {
        path: path.to_path_buf(),
        reason,
    };
    let meta = std::fs::metadata(path).map_err(|error| bad(error.to_string()))?;
    if meta.len() > MAX_SAVE_BYTES as u64 {
        return Err(bad(format!(
            "project is {} bytes, past the {MAX_SAVE_BYTES}-byte cap",
            meta.len()
        )));
    }
    let bytes = std::fs::read(path).map_err(|error| bad(error.to_string()))?;
    load_from_bytes(&bytes).map_err(|error| bad(error.to_string()))
}

// ---------------------------------------------------------------------------
// Render draw list from the document: the snapshot path's frame input.
// ---------------------------------------------------------------------------

/// Stable asset id for one node's pose-baked stand-in mesh: the low 64 bits
/// of the node UUID, so keys stay distinct per node and stable per node
/// across captures (a gizmo move re-stamps the digest below, never the id).
fn mesh_id_of(id: ObjectId) -> u64 {
    let uuid = id.as_uuid();
    let bytes = uuid.as_bytes();
    // The UUID is 16 bytes by construction, so the low half always exists
    // and this 8-into-8 copy cannot fail.
    let mut low = [0u8; 8];
    low.copy_from_slice(&bytes[..8]);
    u64::from_le_bytes(low)
}

/// 32-byte pose stamp over one node world pose: four FNV-1a passes with
/// distinct seeds over the node id plus the raw pose bits (integer math over
/// little-endian bits, so the same node plus same pose stamps the same bytes
/// on every machine; any gizmo/inspector move re-keys the draw below). The
/// mixer is the [`Fnv64`] owner in `engine-core`, shared with the
/// command-service artifact digest; the preimage bytes are unchanged (id
/// bytes, then position, rotation and scale bits in that order).
fn pose_digest(id: ObjectId, world: Transform) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut pass: u64 = 0;
    for slot in out.chunks_mut(8) {
        pass = pass.wrapping_add(1);
        let mut hash = Fnv64::with_state(FNV64_OFFSET ^ GOLDEN_GAMMA.wrapping_mul(pass));
        hash.write(id.as_uuid().as_bytes());
        for part in world
            .position
            .iter()
            .chain(world.rotation.iter())
            .chain(world.scale.iter())
        {
            hash.write(&part.to_bits().to_le_bytes());
        }
        slot.copy_from_slice(&hash.finish().to_le_bytes());
    }
    out
}

/// Mesh key for one node's pose-baked stand-in: stable node id plus the
/// pose stamp, so a move uploads under a fresh key and the presenter FIFO
/// (512 meshes) absorbs drag history by evicting the oldest poses.
fn node_mesh_key(id: ObjectId, world: Transform) -> MeshKey {
    MeshKey::new(mesh_id_of(id), pose_digest(id, world))
}

/// Unit quaternion, or identity when the input is degenerate: mirrors the
/// scene hierarchy convention (kept private in `engine-scene`), so baked
/// normals agree with [`Transform::to_matrix`] on the same pose.
fn quat_normalized_bake(q: [f32; 4]) -> [f32; 4] {
    let len_sq = q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3];
    if !len_sq.is_finite() || len_sq < 1e-12 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let len = len_sq.sqrt();
    [q[0] / len, q[1] / len, q[2] / len, q[3] / len]
}

/// Rotate a vector by a unit quaternion: mirrors the scene convention, so
/// the bake needs no new math owner.
fn quat_rotate_bake(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    // t = 2 * cross(q.xyz, v); v' = v + q.w * t + cross(q.xyz, t).
    let tx = 2.0 * (q[1] * v[2] - q[2] * v[1]);
    let ty = 2.0 * (q[2] * v[0] - q[0] * v[2]);
    let tz = 2.0 * (q[0] * v[1] - q[1] * v[0]);
    [
        v[0] + q[3] * tx + (q[1] * tz - q[2] * ty),
        v[1] + q[3] * ty + (q[2] * tx - q[0] * tz),
        v[2] + q[3] * tz + (q[0] * ty - q[1] * tx),
    ]
}

/// Apply a row-major `T * R * S` matrix ([`Transform::to_matrix`] layout,
/// translation in column 3) to a point.
fn apply_matrix_bake(matrix: &[[f32; 4]; 4], point: [f32; 3]) -> [f32; 3] {
    [
        matrix[0][0] * point[0] + matrix[0][1] * point[1] + matrix[0][2] * point[2] + matrix[0][3],
        matrix[1][0] * point[0] + matrix[1][1] * point[1] + matrix[1][2] * point[2] + matrix[1][3],
        matrix[2][0] * point[0] + matrix[2][1] * point[1] + matrix[2][2] * point[2] + matrix[2][3],
    ]
}

/// Bake one node world pose into a stand-in cube: the shared unit cube
/// moved through the node's `T * R * S` matrix, normals rotated only (exact
/// for uniform and axis-aligned scales; the documented scene approximation
/// under a rotated, nonuniformly scaled ancestor). Revalidated on bake so
/// only render-clean meshes reach the GPU. Pure and deterministic: same
/// pose, same bytes. Snapshot path only (never per tick), so the per-node
/// scratch never touches the frame budget.
fn bake_node_mesh(base: &v1::MeshData, world: Transform) -> Result<v1::MeshData, EditorAppError> {
    let base_positions = base.positions.len();
    let mut positions = Vec::with_capacity(base_positions);
    let mut normals = Vec::with_capacity(base.normals.len());
    let matrix = world.to_matrix();
    let rotation = quat_normalized_bake(world.rotation);
    for (position, normal) in base.positions.iter().zip(base.normals.iter()) {
        positions.push(apply_matrix_bake(&matrix, *position));
        normals.push(quat_rotate_bake(rotation, *normal));
    }
    v1::upload_mesh(&positions, &normals, &base.uvs, &base.indices)
        .map_err(|error| EditorAppError::ViewportFailed(format!("baked stand-in: {error}")))
}

/// Node world poses in submission order: parents first, siblings by id
/// (then re-sorted by id, so draws and uploads share one stable order).
fn sorted_worlds(shell: &Shell) -> Vec<(ObjectId, Transform)> {
    let mut worlds = shell.document().hierarchy.world_all();
    worlds.sort_unstable_by_key(|(id, _)| *id);
    worlds
}

/// Base color for one node's draw: the bound material name, the node id and
/// the world-pose bits folded into three visible channels.
///
/// The presenter draw item carries mesh plus material only (no per-draw
/// transform slot), so the shared stand-in cube cannot move per node yet;
/// the tint keeps every node distinct in the frame and lands every
/// gizmo/inspector pose edit in the draw list until the asset-browser mesh
/// keys (or a per-draw transform in `engine-render`) arrive. The preimage is
/// unambiguous without separators: the name is the only variable-length
/// field and the id plus pose bits after it are fixed-length. Channels stay
/// inside `0.30..=0.75`, so [`v1::PbrMaterial::validate`] always passes.
/// Same node plus same pose plus same binding always gives the same color on
/// every machine (integer math over little-endian bits).
///
/// The mixer is a tiny byte folder local to this tint: a stopgap until the
/// proposed `engine-core` dependency lands, when it becomes
/// `engine_core::Fnv64` with no behavior change beyond the tint values.
fn node_base_color(bound: Option<&str>, id: ObjectId, world: Transform) -> [f32; 3] {
    let mut state: u64 = 0;
    let mut mix = |bytes: &[u8]| {
        for &byte in bytes {
            state = state.wrapping_mul(131).wrapping_add(u64::from(byte));
        }
    };
    if let Some(name) = bound {
        mix(name.as_bytes());
    }
    mix(id.as_uuid().as_bytes());
    for part in world
        .position
        .iter()
        .chain(world.scale.iter())
        .chain(world.rotation.iter())
    {
        mix(&part.to_bits().to_le_bytes());
    }
    let bytes = state.to_le_bytes();
    let channel = |index: usize| 0.30 + 0.45 * (f32::from(bytes[index]) / 255.0);
    [channel(0), channel(3), channel(6)]
}

/// Build the render draw list the snapshot path renders: the editor viewport
/// camera over the default key light, one draw per document node in id
/// order, clear-only for an empty document.
///
/// Every draw carries its node's pose-baked mesh key ([`node_mesh_key`]:
/// the document carries poses plus material bindings, no mesh assets yet;
/// [`ViewportSnapshot::capture`] uploads the matching transformed cube)
/// with a per-node tint ([`node_base_color`]) folded from the node's world
/// pose and bound material, so each wall/crate/hero lands in the frame at
/// its own pose and gizmo/inspector edits move the list. Same document
/// always builds the same list; a reparent that preserves world poses builds
/// the same list too. Over-cap documents (past the presenter per-frame draw
/// bound) fail typed through [`EditorAppError::ViewportFailed`]: the frame
/// is refused, never silently truncated. Fov converts degrees (editor) to
/// radians (`v1`).
pub fn build_draw_list(shell: &Shell, width: u32, height: u32) -> Result<DrawList, EditorAppError> {
    let scene = v1::default_scene(width, height)
        .map_err(|error| EditorAppError::ViewportFailed(format!("default scene: {error}")))?;
    let view = shell.viewport.camera;
    let camera = v1::Camera::new(
        view.position,
        view.target,
        view.up,
        view.fov_y_deg.to_radians(),
    );
    camera
        .validate()
        .map_err(|error| EditorAppError::ViewportFailed(format!("viewport camera: {error}")))?;
    let worlds = sorted_worlds(shell);
    let mut items = Vec::with_capacity(worlds.len());
    for (id, world) in &worlds {
        let bound = shell.material_of(*id);
        items.push(present::DrawItem {
            mesh: node_mesh_key(*id, *world),
            material: v1::PbrMaterial {
                base_color: node_base_color(bound.as_ref().map(|name| name.as_str()), *id, *world),
                ..scene.material.clone()
            },
            base_color: None,
            normal_map: None,
        });
    }
    let list = DrawList {
        camera,
        light: scene.light,
        items,
    };
    list.validate()
        .map_err(|error| EditorAppError::ViewportFailed(format!("draw list: {error}")))?;
    Ok(list)
}

// ---------------------------------------------------------------------------
// Snapshot presenter: offscreen PNG receipts on demand, never on the
// frame path. Missing adapters degrade to the live texture (NoAdapter).
// ---------------------------------------------------------------------------

/// One offscreen frame: PNG receipt plus manifest fields for the log.
#[derive(Debug)]
pub struct SnapshotOutput {
    /// PNG bytes of the frame.
    pub png: Vec<u8>,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Draws submitted.
    pub draws: u32,
    /// Adapter behind the frame.
    pub adapter: String,
    /// Driver behind the frame.
    pub driver: String,
    /// wgpu validation errors observed; green means 0.
    pub validation_errors: u32,
}

/// Lazy offscreen presenter for snapshots. Inits on first capture so
/// headless and GPU-less sessions never pay for (or fail on) the adapter.
/// Pose-baked meshes upload on every capture (never cached across poses),
/// so a moved node can never render stale geometry.
pub struct ViewportSnapshot {
    presenter: Option<Presenter>,
    no_adapter: bool,
}

impl ViewportSnapshot {
    /// Uninitialized: the first [`capture`](Self::capture) inits the adapter.
    pub fn new() -> Self {
        Self {
            presenter: None,
            no_adapter: false,
        }
    }

    /// Render the document offscreen at `width` x `height` pixels. Zero area
    /// suspends (keep last-good); over-bound rejects; no adapter reports
    /// [`EditorAppError::NoAdapter`] and remembers it for later captures.
    pub fn capture(
        &mut self,
        shell: &Shell,
        width: u32,
        height: u32,
    ) -> Result<SnapshotOutput, EditorAppError> {
        match SizeClass::classify(width, height) {
            SizeClass::Suspend => return Err(EditorAppError::ViewportSuspended),
            SizeClass::Reject => {
                return Err(EditorAppError::BadSize { width, height });
            }
            SizeClass::Sizes => {}
        }
        if self.no_adapter {
            return Err(EditorAppError::NoAdapter);
        }
        // Build first: the draw cap and camera validate before any GPU work,
        // so oversize documents and bad cameras fail typed without touching
        // the adapter.
        let list = build_draw_list(shell, width, height)?;
        if self.presenter.is_none() {
            match Presenter::init_offscreen() {
                Ok(presenter) => self.presenter = Some(presenter),
                Err(present::PresentError::NoAdapter) => {
                    self.no_adapter = true;
                    return Err(EditorAppError::NoAdapter);
                }
                Err(other) => return Err(EditorAppError::snapshot(other)),
            }
        }
        let presenter = self.presenter.as_mut().expect("initialized above");
        // Pose-baked upload: one transformed cube per node under its stamped
        // key. Snapshot path only (button/`--snapshot`), so the per-capture
        // scratch plus re-upload never touch the frame budget; no cross-pose
        // cache means no stale geometry after a move.
        let base = v1::cube();
        for (id, world) in &sorted_worlds(shell) {
            let baked = bake_node_mesh(&base, *world)?;
            presenter
                .upload_mesh(node_mesh_key(*id, *world), &baked)
                .map_err(EditorAppError::snapshot)?;
        }
        presenter
            .resize(width, height)
            .map_err(EditorAppError::snapshot)?;
        let out = presenter
            .render_offscreen(&list)
            .map_err(EditorAppError::snapshot)?;
        let png = out.png.ok_or(EditorAppError::ViewportSuspended)?;
        Ok(SnapshotOutput {
            png,
            width: out.receipt.width,
            height: out.receipt.height,
            draws: out.receipt.draws,
            adapter: out.receipt.adapter,
            driver: out.receipt.driver,
            validation_errors: out.receipt.validation_errors,
        })
    }
}

impl Default for ViewportSnapshot {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// The app: winit event loop, egui dock, live texture, play child.
// ---------------------------------------------------------------------------

/// Window GPU state: wgpu device plus queue, the configured surface and the
/// egui renderer. Built once in `resumed`; the surface reconfigures lazily in
/// `frame` (resize) and on renderer-mapped acquire outcomes (Outdated/Lost).
struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    format: wgpu::TextureFormat,
    renderer: egui_wgpu::Renderer,
    configured: (u32, u32),
    adapter_name: String,
}

impl Gpu {
    /// Reconfigure the surface at `width` x `height` (both nonzero).
    /// Keeps the last-good renderer and pipeline: only the swapchain moves.
    fn configure(&mut self, adapter: &wgpu::Adapter, width: u32, height: u32) {
        if let Some(config) = self.surface.get_default_config(adapter, width, height) {
            let mut config = config;
            config.format = self.format;
            self.surface.configure(&self.device, &config);
            self.configured = (width, height);
        }
    }
}

/// Editor app state: document, dock, live texture, snapshot presenter, play
/// child, status line and frame counter. Built by [`EditorApp::new`], driven
/// by [`run`].
pub struct EditorApp {
    config: EditorConfig,
    shell: Shell,
    dock: ShellUi,
    ctx: egui::Context,
    egui_winit: Option<egui_winit::State>,
    gpu: Option<Gpu>,
    adapter: Option<wgpu::Adapter>,
    window: Option<Arc<winit::window::Window>>,
    viewport_tex: Option<egui::TextureHandle>,
    viewport_px: (u32, u32),
    drawn_revision: u64,
    /// Texture deltas not yet uploaded (skipped frames keep them queued;
    /// the next presented frame drains them).
    pending: egui::TexturesDelta,
    snapshot: ViewportSnapshot,
    play: Option<ChildPlay>,
    status: String,
    frames: u64,
}

impl EditorApp {
    /// Build from config: validate sizes, reopen the project when present
    /// (missing starts an empty scene; corrupt fails loudly so no work is
    /// silently dropped).
    pub fn new(config: EditorConfig) -> Result<Self, EditorAppError> {
        config.validate()?;
        let mut shell = Shell::new();
        let status = if config.project.exists() {
            let doc = load_project(&config.project)?;
            let revision = doc.revision;
            shell.open_document(doc);
            format!("opened {} (r{revision})", config.project.display())
        } else {
            format!("new scene ({} not found)", config.project.display())
        };
        let dock = ShellUi::new(&shell);
        Ok(Self {
            config,
            shell,
            dock,
            ctx: egui::Context::default(),
            egui_winit: None,
            gpu: None,
            adapter: None,
            window: None,
            viewport_tex: None,
            viewport_px: (0, 0),
            drawn_revision: u64::MAX,
            pending: egui::TexturesDelta::default(),
            snapshot: ViewportSnapshot::new(),
            play: None,
            status,
            frames: 0,
        })
    }

    /// Status line shown in the top menu.
    pub fn status(&self) -> &str {
        &self.status
    }

    /// Frames drawn this session (units: ticks of the editor loop).
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// True while the arena child is still running. Never blocks.
    pub fn play_alive(&mut self) -> bool {
        self.play.as_mut().is_some_and(|play| play.is_alive())
    }

    /// Start the arena child (project path appended as its first arg so the
    /// game opens what the editor saved). Err when the binary cannot start;
    /// the document is untouched.
    pub fn play_start(&mut self) -> Result<(), EditorAppError> {
        if self.play_alive() {
            return Ok(());
        }
        let mut entry = self.config.arena.clone();
        entry
            .args
            .insert(0, self.config.project.to_string_lossy().into_owned());
        match ChildPlay::spawn(&entry) {
            Ok(play) => {
                self.status = format!("playing: {entry}");
                self.play = Some(play);
                Ok(())
            }
            Err(error) => Err(EditorAppError::ChildFailed {
                command: entry.to_string(),
                reason: error.to_string(),
            }),
        }
    }

    /// Stop the arena child and reap it. Idempotent; the document is untouched.
    pub fn play_stop(&mut self) {
        if let Some(play) = self.play.as_mut() {
            let _ = play.kill();
        }
        self.play = None;
        self.status = "play stopped".to_string();
    }

    /// Save the document to the project path. Returns bytes written.
    pub fn save_now(&mut self) -> Result<u64, EditorAppError> {
        let bytes = save_project(self.shell.document(), &self.config.project)?;
        let revision = self.shell.revision();
        self.status = format!(
            "saved {} ({} bytes, r{revision})",
            self.config.project.display(),
            bytes
        );
        Ok(bytes)
    }

    /// Reopen the project path into the shell (journal resets; selection and
    /// staging clear). Corrupt files fail and keep the current document.
    pub fn open_now(&mut self) -> Result<(), EditorAppError> {
        let doc = load_project(&self.config.project)?;
        let revision = doc.revision;
        self.shell.open_document(doc);
        self.drawn_revision = u64::MAX;
        self.status = format!("opened {} (r{revision})", self.config.project.display());
        Ok(())
    }

    /// Offscreen snapshot of the document into the snapshot path.
    ///
    /// GPU first; when no wgpu adapter is present the live CPU panel raster
    /// encodes the receipt instead, so `--snapshot` never exits silent on a
    /// headless machine. Every other failure is typed and loud.
    pub fn snapshot_now(&mut self) -> Result<(), EditorAppError> {
        let (width, height) = self.live_viewport_px();
        match self.snapshot.capture(&self.shell, width, height) {
            Ok(out) => {
                let path = self.config.snapshot_path();
                write_atomic(&path, &out.png).map_err(|error| EditorAppError::SaveFailed {
                    path: path.clone(),
                    reason: error.to_string(),
                })?;
                self.status = format!(
                    "snapshot {} ({}x{}, {} draws, {} bytes, {} validation errors, {})",
                    path.display(),
                    out.width,
                    out.height,
                    out.draws,
                    out.png.len(),
                    out.validation_errors,
                    out.adapter
                );
                Ok(())
            }
            Err(EditorAppError::NoAdapter) => self.snapshot_fallback(),
            Err(other) => Err(other),
        }
    }

    /// CPU fallback receipt when no wgpu adapter exists: the live viewport
    /// raster through the shared headless PNG encoder. Same path and status
    /// shape as the GPU receipt, so `--snapshot --exit-frames` writes a real
    /// PNG on headless machines instead of failing silent.
    fn snapshot_fallback(&mut self) -> Result<(), EditorAppError> {
        let (width, height) = self.live_viewport_px();
        let image = ViewportImage::render(&self.shell, width, height)
            .map_err(EditorAppError::viewport_image)?;
        let png = encode_png_rgba8(&image.rgba, image.width, image.height).map_err(|error| {
            EditorAppError::SnapshotFailed {
                reason: error.to_string(),
                recovery: Recovery::KeepLastGood,
            }
        })?;
        let path = self.config.snapshot_path();
        write_atomic(&path, &png).map_err(|error| EditorAppError::SaveFailed {
            path: path.clone(),
            reason: error.to_string(),
        })?;
        self.status = format!(
            "snapshot {} ({}x{}, {} draws, {} bytes, live-panel fallback: no wgpu adapter)",
            path.display(),
            image.width,
            image.height,
            image.draws,
            png.len(),
        );
        Ok(())
    }

    /// Snapshot the document when the operator asked for one (`--snapshot`).
    ///
    /// Returns the receipt path when a snapshot was requested and written,
    /// or `None` when no `--snapshot` flag was given (nothing to do, not a
    /// silent skip). A requested snapshot that cannot be written fails typed
    /// and loud; the caller surfaces it as the process error.
    pub fn snapshot_on_exit(&mut self) -> Result<Option<PathBuf>, EditorAppError> {
        if self.config.snapshot.is_none() {
            return Ok(None);
        }
        self.snapshot_now()?;
        Ok(Some(self.config.snapshot_path()))
    }

    /// Live texture size in pixels: shell panel size clamped to the texture
    /// cap (zero falls back to the window size so the panel never suspends
    /// just because the canvas has not been laid out yet).
    fn live_viewport_px(&self) -> (u32, u32) {
        let size = self.shell.viewport.size;
        let mut width = (size.width as u32).min(MAX_VIEWPORT_TEX);
        let mut height = (size.height as u32).min(MAX_VIEWPORT_TEX);
        if width == 0 || height == 0 {
            width = self.config.width.min(MAX_VIEWPORT_TEX);
            height = self.config.height.min(MAX_VIEWPORT_TEX);
        }
        (width.max(1), height.max(1))
    }

    /// Refresh the live egui texture when the revision or size moved. Zero
    /// area keeps last-good; over-bound clamps (the live cap, not the renderer's).
    fn refresh_viewport_texture(&mut self) {
        let (width, height) = self.live_viewport_px();
        if self.viewport_tex.is_some()
            && self.viewport_px == (width, height)
            && self.drawn_revision == self.shell.revision()
        {
            return;
        }
        match ViewportImage::render(&self.shell, width, height) {
            Ok(image) => {
                let handle = self.ctx.load_texture(
                    "viewport",
                    image.to_color_image(),
                    egui::TextureOptions::LINEAR,
                );
                self.viewport_tex = Some(handle);
                self.viewport_px = (width, height);
                self.drawn_revision = self.shell.revision();
            }
            Err(_) => {
                // Suspended or rejected: keep the last-good texture.
            }
        }
    }

    /// One frame: menu, dock, tessellate, paint, present. Zero-area windows
    /// suspend (minimize keeps last-good); every acquire outcome maps through
    /// the renderer's [`surface_action`](present::surface_action), so Outdated/Lost
    /// reconfigure while Timeout/Occluded/validation skip the frame.
    fn frame(&mut self, window: &Arc<winit::window::Window>) {
        if self.gpu.is_none() || self.egui_winit.is_none() {
            return;
        }
        self.refresh_viewport_texture();
        let raw = self
            .egui_winit
            .as_mut()
            .expect("checked above")
            .take_egui_input(window);
        let viewport_tex = self.viewport_tex.clone();
        let ctx = self.ctx.clone();
        let full = ctx.run_ui(raw, |ui| {
            egui::Panel::top("menu").show(ui, |ui| {
                self.menu(ui);
            });
            egui::CentralPanel::default().show(ui, |ui| {
                self.dock
                    .show_with_viewport(&mut self.shell, ui, viewport_tex.as_ref());
            });
        });
        let pixels_per_point = ctx.pixels_per_point();
        let clipped = ctx.tessellate(full.shapes, full.pixels_per_point);
        // Queue texture deltas; skipped frames keep them pending so font and
        // viewport uploads are never lost, and the next presented frame
        // drains them. Same-id deltas append (later regions win per-texel in
        // the renderer), frees union. (`TexturesDelta` implements `Drop`, so
        // its maps move via `take`, not field moves.)
        let mut deltas = full.textures_delta;
        for (id, regions) in std::mem::take(&mut deltas.set) {
            self.pending.set.entry(id).or_default().extend(regions);
        }
        self.pending.free.extend(std::mem::take(&mut deltas.free));
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        let (Some(gpu), Some(adapter)) = (self.gpu.as_mut(), self.adapter.clone()) else {
            self.egui_winit
                .as_mut()
                .expect("checked above")
                .handle_platform_output(window, full.platform_output);
            return;
        };
        if gpu.configured != (size.width, size.height) {
            gpu.configure(&adapter, size.width, size.height);
        }
        let current = gpu.surface.get_current_texture();
        match present::surface_action(SurfaceEvent::from(&current)) {
            present::SurfaceAction::Render => {
                if let wgpu::CurrentSurfaceTexture::Success(frame) = current {
                    Self::present_frame(
                        gpu,
                        frame,
                        &clipped,
                        &mut self.pending,
                        size,
                        pixels_per_point,
                    );
                }
            }
            present::SurfaceAction::RenderThenReconfigure => {
                if let wgpu::CurrentSurfaceTexture::Suboptimal(frame) = current {
                    Self::present_frame(
                        gpu,
                        frame,
                        &clipped,
                        &mut self.pending,
                        size,
                        pixels_per_point,
                    );
                }
                gpu.configure(&adapter, size.width, size.height);
            }
            present::SurfaceAction::ReconfigureThenRetry => {
                gpu.configure(&adapter, size.width, size.height);
            }
            present::SurfaceAction::SkipRetryNext
            | present::SurfaceAction::SkipKeepLastGood
            | present::SurfaceAction::AbortKeepLastGood => {}
        }
        self.egui_winit
            .as_mut()
            .expect("checked above")
            .handle_platform_output(window, full.platform_output);
        self.frames += 1;
    }

    /// Upload egui textures, record paint commands, and present one frame.
    /// Consumes the acquired surface texture and drains the pending deltas;
    /// validation scope is the surface frame (kept last-good on failure by
    /// the caller mapping).
    fn present_frame(
        gpu: &mut Gpu,
        frame: wgpu::SurfaceTexture,
        jobs: &[egui::ClippedPrimitive],
        textures_delta: &mut egui::TexturesDelta,
        size: winit::dpi::PhysicalSize<u32>,
        pixels_per_point: f32,
    ) {
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [size.width, size.height],
            pixels_per_point,
        };
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("e2040-editor"),
            });
        for (id, deltas) in &textures_delta.set {
            for delta in deltas {
                gpu.renderer
                    .update_texture(&gpu.device, &gpu.queue, *id, delta);
            }
        }
        let buffers =
            gpu.renderer
                .update_buffers(&gpu.device, &gpu.queue, &mut encoder, jobs, &screen);
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("e2040-editor-egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.03,
                                g: 0.035,
                                b: 0.06,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            gpu.renderer.render(&mut pass, jobs, &screen);
        }
        for id in &textures_delta.free {
            gpu.renderer.free_texture(id);
        }
        textures_delta.set.clear();
        textures_delta.free.clear();
        gpu.queue
            .submit(buffers.into_iter().chain(std::iter::once(encoder.finish())));
        gpu.queue.present(frame);
    }

    /// Top menu: project, play and snapshot actions plus the status line.
    /// Failures surface as status notices (and the log panel); the document
    /// is never at risk from a button.
    fn menu(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("e2040");
            ui.separator();
            if ui.button("Save").clicked() {
                if let Err(error) = self.save_now() {
                    self.note(error);
                }
            }
            if ui.button("Open").clicked() {
                if let Err(error) = self.open_now() {
                    self.note(error);
                }
            }
            ui.separator();
            if self.play.as_mut().is_some_and(|play| play.is_alive()) {
                if ui.button("Stop").clicked() {
                    self.play_stop();
                }
            } else if ui.button("Play").clicked() {
                if let Err(error) = self.play_start() {
                    self.note(error);
                }
            }
            if ui.button("Snapshot").clicked() {
                if let Err(error) = self.snapshot_now() {
                    self.note(error);
                }
            }
            if ui.button("Undo").clicked() {
                self.shell.undo();
            }
            if ui.button("Redo").clicked() {
                self.shell.redo();
            }
            ui.separator();
            ui.label(self.status.clone());
        });
    }

    /// Surface a typed failure as a status notice plus a log line. The app
    /// keeps running on its last-good state (the error's recovery).
    fn note(&mut self, error: EditorAppError) {
        let recovery = error.recovery();
        self.status = format!("{error} (recovery: {recovery:?})");
        self.shell
            .log_message(format!("ERR menu: {error} (recovery: {recovery:?})"));
    }
}

/// Run the editor until the window closes (or `--exit-frames` elapses).
///
/// When `--snapshot` was requested, the `--exit-frames` exit writes the
/// receipt through [`EditorApp::snapshot_on_exit`] before leaving: the flag
/// combo never exits 0 silently. A snapshot failure is the process error.
pub fn run(config: EditorConfig) -> Result<(), EditorAppError> {
    let event_loop = winit::event_loop::EventLoop::new()
        .map_err(|error| EditorAppError::WindowFailed(error.to_string()))?;
    let mut holder = Holder {
        app: EditorApp::new(config)?,
        init_error: None,
        exit_error: None,
    };
    event_loop
        .run_app(&mut holder)
        .map_err(|error| EditorAppError::WindowFailed(error.to_string()))?;
    if let Some(error) = holder.init_error {
        return Err(error);
    }
    if let Some(error) = holder.exit_error {
        return Err(error);
    }
    Ok(())
}

/// [`winit::application::ApplicationHandler`] glue: window plus GPU setup on
/// `resumed`, events into egui, frames on redraw, snapshot plus exit on close
/// or `--exit-frames`.
struct Holder {
    app: EditorApp,
    init_error: Option<EditorAppError>,
    exit_error: Option<EditorAppError>,
}

impl Holder {
    /// Create the window and GPU state. Failures land in `init_error` and
    /// exit the loop: a window without a GPU backend cannot present.
    fn setup_window(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if self.app.window.is_some() {
            return;
        }
        let window = match event_loop.create_window(
            winit::window::Window::default_attributes()
                .with_title("e2040 editor")
                .with_inner_size(winit::dpi::LogicalSize::new(
                    self.app.config.width,
                    self.app.config.height,
                )),
        ) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                self.init_error = Some(EditorAppError::WindowFailed(error.to_string()));
                event_loop.exit();
                return;
            }
        };
        let state = egui_winit::State::new(
            self.app.ctx.clone(),
            egui::ViewportId::ROOT,
            window.as_ref(),
            None,
            None,
            None,
        );
        self.app.egui_winit = Some(state);
        // GPU setup, driven synchronously by `block_on` (one-time only).
        // Mirrors the renderer's offscreen bring-up: high-performance adapter, its
        // device plus queue, then the surface configured at the window size.
        let mut fail = |reason: String| {
            self.init_error = Some(EditorAppError::WindowFailed(reason));
            event_loop.exit();
        };
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = match instance.create_surface(Arc::clone(&window)) {
            Ok(surface) => surface,
            Err(error) => {
                fail(format!("surface: {error}"));
                return;
            }
        };
        let adapter = match block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        })) {
            Ok(adapter) => adapter,
            Err(_) => {
                self.init_error = Some(EditorAppError::NoAdapter);
                event_loop.exit();
                return;
            }
        };
        let info = adapter.get_info();
        let (device, queue) = match block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("e2040-editor"),
            ..Default::default()
        })) {
            Ok(pair) => pair,
            Err(error) => {
                fail(format!("device: {error}"));
                return;
            }
        };
        // Prefer an sRGB surface format; fall back to the first offered.
        let caps = surface.get_capabilities(&adapter);
        let Some(format) = caps
            .formats
            .iter()
            .copied()
            .find(|format| format.is_srgb())
            .or_else(|| caps.formats.first().copied())
        else {
            fail("surface reports no formats".to_string());
            return;
        };
        let size = window.inner_size();
        let (width, height) = (size.width.max(1), size.height.max(1));
        let renderer = egui_wgpu::Renderer::new(
            &device,
            format,
            egui_wgpu::RendererOptions {
                msaa_samples: 1,
                depth_stencil_format: None,
                dithering: false,
                predictable_texture_filtering: false,
            },
        );
        let mut gpu = Gpu {
            device,
            queue,
            surface,
            format,
            renderer,
            configured: (0, 0),
            adapter_name: format!("{} ({})", info.name, info.backend),
        };
        self.app.adapter = Some(adapter);
        // Configure through the stored adapter (borrowed from the app).
        let adapter_ref = self.app.adapter.as_ref().expect("stored above");
        gpu.configure(adapter_ref, width, height);
        self.app.gpu = Some(gpu);
        self.app.window = Some(window);
        self.app.status = format!(
            "window {}x{} on {}",
            width,
            height,
            self.app
                .gpu
                .as_ref()
                .map(|gpu| gpu.adapter_name.as_str())
                .unwrap_or("unknown adapter")
        );
    }
}

impl winit::application::ApplicationHandler for Holder {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.setup_window(event_loop);
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        let (Some(window), Some(state)) = (self.app.window.clone(), self.app.egui_winit.as_mut())
        else {
            return;
        };
        let _ = state.on_window_event(window.as_ref(), &event);
        match event {
            winit::event::WindowEvent::CloseRequested => event_loop.exit(),
            // Resizes reconfigure lazily in `frame` (plus renderer-mapped acquire
            // outcomes), so a resize storm never touches the GPU here.
            winit::event::WindowEvent::RedrawRequested => {
                self.app.frame(&window);
                if let Some(exit_after) = self.app.config.exit_frames {
                    if self.app.frames >= exit_after {
                        // Snapshot-on-exit: a requested `--snapshot` receipt is
                        // written before leaving, never silently ignored. A
                        // failure becomes the process error in `run`.
                        if let Err(error) = self.app.snapshot_on_exit() {
                            self.exit_error = Some(error);
                        }
                        event_loop.exit();
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {
        if let Some(window) = &self.app.window {
            window.request_redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_scene::ids::ObjectId;
    use engine_scene::transform::Transform;

    fn args(flag: &[&str]) -> Vec<String> {
        std::iter::once("e2040-editor")
            .chain(flag.iter().copied())
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn every_error_names_its_recovery() {
        let dir = std::env::temp_dir();
        let path = dir.join("ed1-missing.edoc");
        let cases: Vec<(EditorAppError, Recovery)> = vec![
            (
                EditorAppError::BadArgs("x".into()),
                Recovery::FixCallerInput,
            ),
            (
                EditorAppError::BadProject {
                    path: path.clone(),
                    reason: "gone".into(),
                },
                Recovery::FixCallerInput,
            ),
            (
                EditorAppError::SaveFailed {
                    path: path.clone(),
                    reason: "ro".into(),
                },
                Recovery::FixCallerInput,
            ),
            (
                EditorAppError::BadSize {
                    width: 9999,
                    height: 1,
                },
                Recovery::FixCallerInput,
            ),
            (EditorAppError::ViewportSuspended, Recovery::KeepLastGood),
            (
                EditorAppError::ViewportFailed("x".into()),
                Recovery::FixCallerInput,
            ),
            (EditorAppError::NoAdapter, Recovery::KeepLastGood),
            (
                EditorAppError::SnapshotFailed {
                    reason: "x".into(),
                    recovery: Recovery::RetryFrame,
                },
                Recovery::RetryFrame,
            ),
            (
                EditorAppError::WindowFailed("x".into()),
                Recovery::FixCallerInput,
            ),
            (
                EditorAppError::ChildFailed {
                    command: "game".into(),
                    reason: "missing".into(),
                },
                Recovery::FixCallerInput,
            ),
        ];
        for (error, recovery) in cases {
            assert_eq!(error.recovery(), recovery, "recovery for {error}");
            assert!(!format!("{error}").is_empty(), "display for {error:?}");
        }
    }

    #[test]
    fn ra1_present_failures_map_with_recovery() {
        use present::PresentError;
        assert_eq!(
            EditorAppError::snapshot(PresentError::NoAdapter),
            EditorAppError::NoAdapter
        );
        assert_eq!(
            EditorAppError::snapshot(PresentError::InvalidSize {
                width: 9999,
                height: 2
            }),
            EditorAppError::BadSize {
                width: 9999,
                height: 2
            }
        );
        let lost = EditorAppError::snapshot(PresentError::SurfaceLost { generation: 3 });
        assert_eq!(
            lost,
            EditorAppError::SnapshotFailed {
                reason: PresentError::SurfaceLost { generation: 3 }.to_string(),
                recovery: Recovery::ReconfigureSurface,
            }
        );
        assert_eq!(lost.recovery(), Recovery::ReconfigureSurface);
        let timeout = EditorAppError::snapshot(PresentError::SurfaceTimeout);
        assert_eq!(timeout.recovery(), Recovery::SkipFrame);
    }

    #[test]
    fn viewport_image_failures_map_with_recovery() {
        use crate::viewport_image::ViewportImageError;
        use engine_scene::transform::HierarchyError;
        assert_eq!(
            EditorAppError::viewport_image(ViewportImageError::Suspended),
            EditorAppError::ViewportSuspended
        );
        assert_eq!(
            EditorAppError::viewport_image(ViewportImageError::BadSize {
                width: 9999,
                height: 2
            }),
            EditorAppError::BadSize {
                width: 9999,
                height: 2
            }
        );
        // Failure case: a corrupt marker row is a typed viewport failure
        // naming the row, never a silent skip; the caller fixes the document.
        let corrupt = EditorAppError::viewport_image(ViewportImageError::Hierarchy(
            HierarchyError::Duplicate,
        ));
        assert_eq!(
            corrupt,
            EditorAppError::ViewportFailed(HierarchyError::Duplicate.to_string())
        );
        assert_eq!(corrupt.recovery(), Recovery::FixCallerInput);
        assert!(!format!("{corrupt}").is_empty());
    }

    #[test]
    fn args_parse_flags_and_reject_garbage() {
        let config = parse_args(&args(&[
            "--project",
            "demo.edoc",
            "--arena-bin",
            "my-arena",
            "--arena-arg",
            "--smoke",
            "--width",
            "800",
            "--height",
            "600",
            "--snapshot",
            "out.png",
            "--exit-frames",
            "10",
        ]))
        .unwrap();
        assert_eq!(config.project, PathBuf::from("demo.edoc"));
        assert_eq!(config.arena.program, PathBuf::from("my-arena"));
        assert_eq!(config.arena.args, vec!["--smoke".to_string()]);
        assert_eq!((config.width, config.height), (800, 600));
        assert_eq!(config.snapshot, Some(PathBuf::from("out.png")));
        assert_eq!(config.exit_frames, Some(10));

        assert_eq!(
            parse_args(&args(&["--nope"])).unwrap_err(),
            EditorAppError::BadArgs("unknown flag `--nope` (see --help)".into())
        );
        assert!(matches!(
            parse_args(&args(&["--width"])).unwrap_err(),
            EditorAppError::BadArgs(_)
        ));
        assert!(matches!(
            parse_args(&args(&["--width", "wide"])).unwrap_err(),
            EditorAppError::BadArgs(_)
        ));
        assert_eq!(
            parse_args(&args(&["--width", "0", "--height", "600"])).unwrap_err(),
            EditorAppError::BadSize {
                width: 0,
                height: 600
            }
        );
        assert_eq!(
            parse_args(&args(&["--width", "99999", "--height", "600"])).unwrap_err(),
            EditorAppError::BadSize {
                width: 99999,
                height: 600
            }
        );
    }

    #[test]
    fn config_defaults_validate_and_snapshot_path() {
        let config = EditorConfig::new();
        config.validate().unwrap();
        assert_eq!(config.snapshot_path(), PathBuf::from("scene.edoc.png"));
        let explicit = EditorConfig {
            snapshot: Some(PathBuf::from("a.png")),
            ..EditorConfig::new()
        };
        assert_eq!(explicit.snapshot_path(), PathBuf::from("a.png"));
    }

    /// Unique temp project path per test (parallel-safe, no collisions).
    fn temp_project(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ed1-{name}-{}-{}.edoc",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ))
    }

    fn sample_doc() -> Document {
        use crate::commands::{apply, Batch, Journal, Op};
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let id = ObjectId::new();
        apply(
            &mut doc,
            &mut journal,
            &Batch {
                ops: vec![Op::Insert {
                    id,
                    parent: ObjectId::NIL,
                    local: Transform::new([1.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
                }],
                base_revision: 0,
                idempotency_key: 1,
                provenance: None,
            },
        )
        .unwrap();
        doc
    }

    #[test]
    fn project_save_reopen_roundtrip() {
        let path = temp_project("roundtrip");
        let doc = sample_doc();
        let bytes = save_project(&doc, &path).unwrap();
        assert!(bytes > 0);
        let back = load_project(&path).unwrap();
        assert_eq!(back, doc);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn project_open_failures_are_typed() {
        let missing = temp_project("missing");
        let _ = std::fs::remove_file(&missing);
        let error = load_project(&missing).unwrap_err();
        assert!(matches!(error, EditorAppError::BadProject { .. }));
        assert!(format!("{error}").contains("ed1-missing"));

        let corrupt = temp_project("corrupt");
        std::fs::write(&corrupt, b"not a document").unwrap();
        let error = load_project(&corrupt).unwrap_err();
        assert!(matches!(error, EditorAppError::BadProject { .. }));
        let _ = std::fs::remove_file(&corrupt);
    }

    #[test]
    fn draw_list_maps_every_node_with_pose_and_material_tints() {
        use crate::commands::MaterialName;

        let shell = Shell::new();
        let empty = build_draw_list(&shell, 160, 120).unwrap();
        assert!(empty.items.is_empty());

        // Three wall/crate/hero-style nodes: one draw each on its own
        // pose-baked stand-in mesh, every tint valid and pairwise distinct.
        let mut full = Shell::new();
        let wall = ObjectId::new();
        let krate = ObjectId::new();
        let hero = ObjectId::new();
        for (id, pose) in [
            (
                wall,
                Transform::new([-2.0, 1.5, -8.0], [0.0, 0.0, 0.0, 1.0], [8.0, 3.0, 1.0]),
            ),
            (
                krate,
                Transform::new([1.0, 0.5, 0.5], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
            ),
            (
                hero,
                Transform::new([0.0, 0.9, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
            ),
        ] {
            full.insert_node(id, ObjectId::NIL, pose).unwrap();
        }
        // A bound material joins the tint preimage like the pose does.
        full.apply_ops(vec![crate::commands::Op::SetMaterial {
            id: hero,
            material: Some(MaterialName::try_from("hero-mat").unwrap()),
        }])
        .unwrap();
        let list = build_draw_list(&full, 160, 120).unwrap();
        assert_eq!(list.items.len(), 3);
        for item in &list.items {
            item.material
                .validate()
                .expect("per-node tint stays in range");
        }
        // Each node bakes under its own pose-stamped key.
        let keys: Vec<MeshKey> = list.items.iter().map(|item| item.mesh).collect();
        assert_ne!(keys[0], keys[1], "wall and crate keys differ");
        assert_ne!(keys[1], keys[2], "crate and hero keys differ");
        assert_ne!(keys[0], keys[2], "wall and hero keys differ");
        let colors: Vec<[f32; 3]> = list
            .items
            .iter()
            .map(|item| item.material.base_color)
            .collect();
        assert_ne!(colors[0], colors[1], "wall and crate tint apart");
        assert_ne!(colors[1], colors[2], "crate and bound hero tint apart");
        assert_ne!(colors[0], colors[2], "wall and bound hero tint apart");
        // Same document builds the same list twice.
        assert_eq!(build_draw_list(&full, 160, 120).unwrap(), list);
    }

    #[test]
    fn draw_list_follows_pose_edits_node_by_node() {
        let mut shell = Shell::new();
        let wall = ObjectId::new();
        let hero = ObjectId::new();
        shell
            .insert_node(
                wall,
                ObjectId::NIL,
                Transform::new([-2.0, 1.5, -8.0], [0.0, 0.0, 0.0, 1.0], [8.0, 3.0, 1.0]),
            )
            .unwrap();
        shell
            .insert_node(
                hero,
                ObjectId::NIL,
                Transform::new([0.0, 0.9, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
            )
            .unwrap();
        let before = build_draw_list(&shell, 160, 120).unwrap();
        // The gizmo/inspector path: a pose edit through the command service.
        shell
            .apply_ops(vec![crate::commands::Op::SetLocal {
                id: hero,
                local: Transform::new([2.5, 0.9, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
            }])
            .unwrap();
        let after = build_draw_list(&shell, 160, 120).unwrap();
        assert_ne!(before, after, "the gizmo move lands in the draw list");
        // Submission order is id-sorted, so exactly the hero draw changed.
        let changed = before
            .items
            .iter()
            .zip(after.items.iter())
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(changed, 1, "only the moved node re-keys and retints");
    }

    /// Axis-aligned bounds of baked positions, the geometric oracle for the
    /// pose bake (min/max per axis over every vertex).
    fn baked_bounds(mesh: &v1::MeshData) -> ([f32; 3], [f32; 3]) {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for position in &mesh.positions {
            for axis in 0..3 {
                min[axis] = min[axis].min(position[axis]);
                max[axis] = max[axis].max(position[axis]);
            }
        }
        (min, max)
    }

    #[test]
    fn baked_stand_in_follows_world_poses() {
        let base = v1::cube();
        // Wall: an 8x3x1 box centered at (-2, 1.5, -8).
        let wall = bake_node_mesh(
            &base,
            Transform::new([-2.0, 1.5, -8.0], [0.0, 0.0, 0.0, 1.0], [8.0, 3.0, 1.0]),
        )
        .unwrap();
        assert_eq!(wall.positions.len(), base.positions.len());
        let (min, max) = baked_bounds(&wall);
        for axis in 0..3 {
            assert!(
                (min[axis] - [-6.0, 0.0, -8.5][axis]).abs() < 1e-4,
                "wall min drifts on axis {axis}: {min:?}"
            );
            assert!(
                (max[axis] - [2.0, 3.0, -7.5][axis]).abs() < 1e-4,
                "wall max drifts on axis {axis}: {max:?}"
            );
        }
        // Hero: a unit cube centered at (0, 0.9, 0).
        let hero = bake_node_mesh(
            &base,
            Transform::new([0.0, 0.9, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
        )
        .unwrap();
        let (min, max) = baked_bounds(&hero);
        for axis in 0..3 {
            assert!(
                (min[axis] - [-0.5, 0.4, -0.5][axis]).abs() < 1e-4,
                "hero min drifts on axis {axis}: {min:?}"
            );
            assert!(
                (max[axis] - [0.5, 1.4, 0.5][axis]).abs() < 1e-4,
                "hero max drifts on axis {axis}: {max:?}"
            );
        }
        // The gizmo move shifts the baked box exactly with the pose.
        let moved = bake_node_mesh(
            &base,
            Transform::new([2.5, 0.9, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
        )
        .unwrap();
        assert_ne!(
            moved.positions, hero.positions,
            "the move lands in baked vertices"
        );
        let (min, max) = baked_bounds(&moved);
        assert!((min[0] - 2.0).abs() < 1e-4 && (max[0] - 3.0).abs() < 1e-4);
        // Same pose bakes the same bytes twice.
        assert_eq!(
            bake_node_mesh(
                &base,
                Transform::new([0.0, 0.9, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
            )
            .unwrap(),
            hero
        );
    }

    #[test]
    fn node_mesh_keys_restamp_on_move() {
        let id = ObjectId::new();
        let here = Transform::new([0.0, 0.9, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]);
        let moved = Transform::new([2.5, 0.9, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]);
        assert_eq!(
            node_mesh_key(id, here),
            node_mesh_key(id, here),
            "same node plus same pose keeps its key"
        );
        assert_ne!(
            node_mesh_key(id, here),
            node_mesh_key(id, moved),
            "the gizmo move re-stamps the key"
        );
        assert_ne!(
            node_mesh_key(id, here),
            node_mesh_key(ObjectId::new(), here),
            "distinct nodes never share a key"
        );
    }

    /// Headless frame oracle for the GPU snapshot path: the merged baked
    /// meshes through the software PBR rasterizer under the same camera
    /// mapping [`build_draw_list`] uses, so geometry moves prove out without
    /// an adapter.
    fn oracle_frame_png(shell: &Shell, width: u32, height: u32) -> Vec<u8> {
        let base = v1::cube();
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut uvs = Vec::new();
        let mut indices = Vec::new();
        for (_, world) in &sorted_worlds(shell) {
            let baked = bake_node_mesh(&base, *world).expect("oracle bake validates");
            let offset = positions.len() as u32;
            positions.extend_from_slice(&baked.positions);
            normals.extend_from_slice(&baked.normals);
            uvs.extend_from_slice(&baked.uvs);
            indices.extend(baked.indices.iter().map(|index| index + offset));
        }
        let merged = v1::upload_mesh(&positions, &normals, &uvs, &indices)
            .expect("merged oracle mesh validates");
        let view = shell.viewport.camera;
        let camera = v1::Camera::new(
            view.position,
            view.target,
            view.up,
            view.fov_y_deg.to_radians(),
        );
        let defaults = v1::default_scene(width, height).expect("default scene");
        let scene = v1::Scene {
            mesh: merged,
            camera,
            light: defaults.light,
            material: defaults.material,
        };
        v1::capture_scene(&scene, width, height)
            .expect("oracle frame renders")
            .0
    }

    #[test]
    fn moved_node_rerenders_different_pixels() {
        let mut shell = Shell::new();
        let hero = ObjectId::new();
        for (id, pose) in [
            (
                ObjectId::new(),
                Transform::new([-2.0, 1.5, -8.0], [0.0, 0.0, 0.0, 1.0], [8.0, 3.0, 1.0]),
            ),
            (
                ObjectId::new(),
                Transform::new([1.0, 0.5, 0.5], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
            ),
            (
                hero,
                Transform::new([0.0, 0.9, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
            ),
        ] {
            shell.insert_node(id, ObjectId::NIL, pose).unwrap();
        }
        let before = oracle_frame_png(&shell, 320, 200);
        // Wall/crate/hero geometry is really in the frame: the posed scene
        // renders nothing like one bare cube at the origin.
        let view = shell.viewport.camera;
        let origin_camera = v1::Camera::new(
            view.position,
            view.target,
            view.up,
            view.fov_y_deg.to_radians(),
        );
        let defaults = v1::default_scene(320, 200).expect("default scene");
        let origin = v1::capture_scene(
            &v1::Scene {
                mesh: v1::cube(),
                camera: origin_camera,
                light: defaults.light,
                material: defaults.material,
            },
            320,
            200,
        )
        .expect("origin frame renders")
        .0;
        assert_ne!(before, origin, "posed boxes differ from the bare cube");
        // The gizmo/inspector path: the same SetLocal a drag tick commits.
        shell
            .apply_ops(vec![crate::commands::Op::SetLocal {
                id: hero,
                local: Transform::new([2.5, 0.9, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
            }])
            .unwrap();
        let after = oracle_frame_png(&shell, 320, 200);
        assert_ne!(before, after, "the gizmo move re-renders the frame");
    }

    #[test]
    fn snapshot_refuses_over_cap_before_touching_the_gpu() {
        let mut shell = Shell::new();
        for _ in 0..=present::MAX_DRAWS_PER_FRAME {
            shell
                .insert_node(
                    ObjectId::new(),
                    ObjectId::NIL,
                    Transform::new([0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
                )
                .unwrap();
        }
        // Headless and adapter-free: the draw-list build fails first, so the
        // refusal is typed even with no GPU in the machine.
        let mut snapshots = ViewportSnapshot::new();
        let error = snapshots.capture(&shell, 160, 120).unwrap_err();
        assert!(
            matches!(error, EditorAppError::ViewportFailed(_)),
            "over-cap refuses typed, got {error}"
        );
        assert!(
            format!("{error}").contains("draw count"),
            "the refusal names the cap, got {error}"
        );
    }

    #[test]
    fn draw_list_follows_material_binds_and_unbinds() {
        use crate::commands::MaterialName;

        let mut shell = Shell::new();
        let hero = ObjectId::new();
        shell
            .insert_node(
                hero,
                ObjectId::NIL,
                Transform::new([0.0, 0.9, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
            )
            .unwrap();
        let plain = build_draw_list(&shell, 160, 120).unwrap();
        shell
            .apply_ops(vec![crate::commands::Op::SetMaterial {
                id: hero,
                material: Some(MaterialName::try_from("hero-mat").unwrap()),
            }])
            .unwrap();
        let bound = build_draw_list(&shell, 160, 120).unwrap();
        assert_ne!(plain, bound, "the material bind lands in the draw list");
        shell
            .apply_ops(vec![crate::commands::Op::SetMaterial {
                id: hero,
                material: None,
            }])
            .unwrap();
        assert_eq!(
            build_draw_list(&shell, 160, 120).unwrap(),
            plain,
            "unbinding restores the original draw"
        );
    }

    #[test]
    fn draw_list_refuses_over_cap_documents() {
        let mut shell = Shell::new();
        for _ in 0..=present::MAX_DRAWS_PER_FRAME {
            shell
                .insert_node(
                    ObjectId::new(),
                    ObjectId::NIL,
                    Transform::new([0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
                )
                .unwrap();
        }
        let error = build_draw_list(&shell, 160, 120).unwrap_err();
        assert!(
            matches!(error, EditorAppError::ViewportFailed(_)),
            "over-cap refuses typed, got {error}"
        );
        assert!(
            format!("{error}").contains("draw count"),
            "the refusal names the cap, got {error}"
        );
    }

    #[test]
    fn block_on_drives_a_ready_future() {
        async fn forty_two() -> u32 {
            41 + 1
        }
        assert_eq!(block_on(forty_two()), 42);
    }

    #[test]
    fn app_opens_missing_project_empty_and_refuses_corrupt() {
        let missing = temp_project("app-missing");
        let _ = std::fs::remove_file(&missing);
        let config = EditorConfig {
            project: missing.clone(),
            ..EditorConfig::new()
        };
        let app = EditorApp::new(config).unwrap();
        assert_eq!(app.shell.document().hierarchy.len(), 0);
        assert!(app.status().contains("new scene"));

        let corrupt = temp_project("app-corrupt");
        std::fs::write(&corrupt, b"junk").unwrap();
        let config = EditorConfig {
            project: corrupt.clone(),
            ..EditorConfig::new()
        };
        match EditorApp::new(config) {
            Err(EditorAppError::BadProject { .. }) => {}
            Ok(_) => panic!("corrupt project must fail typed"),
            Err(other) => panic!("corrupt project must be BadProject, got {other}"),
        }
        let _ = std::fs::remove_file(&corrupt);
    }

    #[test]
    fn app_save_then_open_roundtrips_through_shell() {
        let path = temp_project("app-save");
        let _ = std::fs::remove_file(&path);
        let config = EditorConfig {
            project: path.clone(),
            ..EditorConfig::new()
        };
        let mut app = EditorApp::new(config).unwrap();
        let id = ObjectId::new();
        app.shell
            .insert_node(
                id,
                ObjectId::NIL,
                Transform::new([2.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
            )
            .unwrap();
        app.save_now().unwrap();
        assert!(app.status().contains("saved"));

        // Mutate, then reopen: the file wins, staging clears.
        app.shell.remove_node(id).unwrap();
        app.open_now().unwrap();
        assert!(app.shell.document().hierarchy.contains(id));
        assert!(app.status().contains("opened"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn play_spawn_failure_is_typed_and_stop_is_idempotent() {
        let config = EditorConfig {
            arena: PlayEntrypoint::new("e2040-no-such-arena-xyz", Vec::<String>::new()),
            ..EditorConfig::new()
        };
        let mut app = EditorApp::new(config).unwrap();
        let error = app.play_start().unwrap_err();
        assert!(matches!(error, EditorAppError::ChildFailed { .. }));
        assert!(format!("{error}").contains("e2040-no-such-arena-xyz"));
        assert!(!app.play_alive());
        // Stop with no child is a no-op Ok.
        app.play_stop();
        assert!(!app.play_alive());
    }

    #[test]
    fn snapshot_rejects_zero_area_before_touching_the_gpu() {
        let mut snapshots = ViewportSnapshot::new();
        let shell = Shell::new();
        assert_eq!(
            snapshots.capture(&shell, 0, 600).unwrap_err(),
            EditorAppError::ViewportSuspended
        );
        assert_eq!(
            snapshots.capture(&shell, 99999, 8).unwrap_err(),
            EditorAppError::BadSize {
                width: 99999,
                height: 8
            }
        );
    }

    /// Headless dock frame with the live viewport texture: no window, no GPU,
    /// just one egui pass over the menu plus all four panels with the
    /// texture attached. Proves the panels draw with a texture present.
    #[test]
    fn headless_dock_frame_with_viewport_texture() {
        let mut app = EditorApp::new(EditorConfig::new()).unwrap();
        let id = ObjectId::new();
        app.shell
            .insert_node(
                id,
                ObjectId::NIL,
                Transform::new([0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
            )
            .unwrap();
        assert!(app.shell.select(id));

        // Refresh the live texture headlessly (CPU raster only, no GPU).
        app.refresh_viewport_texture();
        let handle = app.viewport_tex.clone().expect("texture rasterized");
        assert_eq!(app.viewport_px, (800, 600));

        let ctx = app.ctx.clone();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            app.menu(ui);
            app.dock
                .show_with_viewport(&mut app.shell, ui, Some(&handle));
        });
        output.textures_delta.clear();

        // Drawing changed no document state; the menu ran without a play child.
        assert_eq!(app.shell.selection, Some(id));
        assert!(app.shell.staged_preview().is_none());
        assert!(!app.play_alive());
    }

    #[test]
    fn exit_snapshot_without_flag_is_noop() {
        let config = EditorConfig {
            exit_frames: Some(1),
            ..EditorConfig::new()
        };
        assert!(config.snapshot.is_none());
        let mut app = EditorApp::new(config).unwrap();
        assert_eq!(app.snapshot_on_exit().unwrap(), None);
    }

    #[test]
    fn exit_snapshot_writes_a_real_png() {
        let dir = std::env::temp_dir();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let path = dir.join(format!("ed1-exit-snap-{}-{stamp}.png", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let config = EditorConfig {
            snapshot: Some(path.clone()),
            exit_frames: Some(30),
            width: 320,
            height: 200,
            ..EditorConfig::new()
        };
        let mut app = EditorApp::new(config).unwrap();
        // GPU when an adapter exists, live-panel fallback when headless:
        // either way a real PNG receipt lands at the requested path.
        let written = app.snapshot_on_exit().unwrap();
        assert_eq!(written, Some(path.clone()));
        let bytes = std::fs::read(&path).expect("exit snapshot writes its PNG");
        assert!(
            bytes.len() > 100,
            "receipt is a real frame, got {}",
            bytes.len()
        );
        assert_eq!(
            &bytes[0..8],
            &[137, 80, 78, 71, 13, 10, 26, 10],
            "receipt is PNG magic"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn snapshot_fallback_encodes_the_live_panel() {
        let dir = std::env::temp_dir();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(1);
        let path = dir.join(format!(
            "ed1-fallback-snap-{}-{stamp}.png",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let config = EditorConfig {
            snapshot: Some(path.clone()),
            width: 320,
            height: 200,
            ..EditorConfig::new()
        };
        let mut app = EditorApp::new(config).unwrap();
        app.snapshot_fallback().unwrap();
        let bytes = std::fs::read(&path).expect("fallback writes its PNG");
        assert_eq!(&bytes[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        assert!(app.status().contains("fallback"));
        let _ = std::fs::remove_file(&path);
    }
}
