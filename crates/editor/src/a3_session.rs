//! Recorded editor run: viewport, picking, hierarchy, inspector, gizmo move,
//! undo/redo, logs, isolated play and a forced game-process crash.
//!
//! Owner: the recorded evidence run; reads licensed content bytes, owns only
//! the session report plus the journal and captures beside it. Units: bytes
//! for files, meters/radians for poses, ticks for the play loop. The run is
//! std-only: it drives the same [`Shell`], browser, gizmo, inspector and
//! supervised-child paths the editor executable uses, and the executable
//! itself invokes it for evidence (passing its window-path captures in).
//! Determinism: same content bytes give the same placements, the same
//! deterministic node ids and the same save/reopen byte equality; two runs
//! write byte-identical `scene.edoc` files. Wall-clock durations are never
//! recorded, so polling a crashing child cannot skew a rerun.
//! Errors: [`A3Error`]; any failure stops the session with its cause.

use std::fmt;
use std::path::{Path, PathBuf};

use engine_scene::ids::ObjectId;
use engine_scene::transform::Transform;

use crate::a2_session::{capture_viewport_png, fnv_hex, ExeCapture};
use crate::browser::{search, AssetKind, BundleEntry, BundleView, SearchQuery};
use crate::gizmo::GizmoDrag;
use crate::play::{ChildPlay, PlayEntrypoint};
use crate::save::{load_from_bytes, save_to_bytes};
use crate::shell::Shell;

/// Content files read by the session, relative to the content root.
const HERO_MESH: &str = "fighters/glb/hero_body.glb";
const CRATE_MESH: &str = "arena/src/crate.gltf";
const ARENA_MANIFEST: &str = "arena/manifest.json";
const FIGHTERS_MANIFEST: &str = "fighters/manifest.json";

/// First four bytes of a binary glTF asset.
const GLB_MAGIC: [u8; 4] = *b"glTF";

/// What stopped the recorded run. Every variant names its file or node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum A3Error {
    /// A content, manifest or output file could not be read or written.
    Io {
        /// File that failed.
        path: String,
        /// OS report.
        reason: String,
    },
    /// A content file missed its format magic: not the licensed bytes.
    BadMagic {
        /// File refused.
        file: String,
        /// Magic expected (`glTF`, `JSON`).
        expected: &'static str,
    },
    /// A manifest does not name a file it must cover.
    Manifest {
        /// Manifest checked.
        file: String,
        /// Entry text missing from it.
        missing: String,
    },
    /// The asset browser refused a lookup.
    Browser(String),
    /// The command service (or a panel driving it) refused an edit.
    Shell(String),
    /// The supervised game process refused to behave as staged.
    Play(String),
    /// The save codec refused a reopen.
    Save(String),
    /// A viewport capture could not be encoded or written.
    Capture(String),
    /// A session invariant broke (counts, revisions, byte equality).
    Check(String),
}

impl fmt::Display for A3Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, reason } => write!(f, "cannot read {path}: {reason}"),
            Self::BadMagic { file, expected } => {
                write!(
                    f,
                    "licensed bytes check failed for {file}: expected {expected} magic"
                )
            }
            Self::Manifest { file, missing } => {
                write!(f, "manifest {file} does not cover `{missing}`")
            }
            Self::Browser(reason) => write!(f, "browser refused: {reason}"),
            Self::Shell(reason) => write!(f, "shell refused: {reason}"),
            Self::Play(reason) => write!(f, "game process refused: {reason}"),
            Self::Save(reason) => write!(f, "save/reopen refused: {reason}"),
            Self::Capture(reason) => write!(f, "viewport capture failed: {reason}"),
            Self::Check(reason) => write!(f, "session check failed: {reason}"),
        }
    }
}

impl std::error::Error for A3Error {}

/// Full receipt of one recorded editor run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorReport {
    /// Licensed sources verified, in read order.
    pub sources: Vec<String>,
    /// Viewport panel line (size plus marker count).
    pub viewport: String,
    /// Picking line (pixel plus node picked).
    pub picking: String,
    /// Hierarchy rows at select time.
    pub hierarchy: Vec<String>,
    /// Inspector edit receipt.
    pub inspector: String,
    /// Gizmo move receipt.
    pub gizmo: String,
    /// Material binding receipt.
    pub material: String,
    /// Undo/redo receipts, each naming its revision.
    pub undo_redo: Vec<String>,
    /// Log panel line (depth plus tail message).
    pub logs: String,
    /// Isolated play receipt (child command plus observation).
    pub play: String,
    /// Forced-crash receipt (what died plus what survived).
    pub crash: String,
    /// In-viewport loop receipt (executable appends this; empty in library).
    pub play_in: String,
    /// Saved scene file name plus bytes plus receipt hash.
    pub saved: String,
    /// Reopened scene file name plus bytes plus receipt hash.
    pub reopened: String,
    /// True when reopen-then-save is byte-identical to the save.
    pub bytes_match: bool,
    /// Document revision at save time.
    pub revision: u64,
    /// Node count at save time.
    pub nodes: usize,
    /// Viewport captures written beside the scene (`file (bytes, fnv)`).
    pub captures: Vec<String>,
    /// How the capture PNGs were produced (exe window path or fallback).
    pub capture_method: String,
}

impl EditorReport {
    /// Render the evidence report text.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("recorded editor run\n");
        out.push_str(
            "viewport, picking, hierarchy, inspector, gizmo, undo/redo, logs, \
             isolated play and crash recovery in one session\n\n",
        );
        out.push_str("sources (path, bytes, license):\n");
        for source in &self.sources {
            out.push_str(&format!("  {source}\n"));
        }
        out.push_str(&format!("viewport: {}\n", self.viewport));
        out.push_str(&format!("picking: {}\n", self.picking));
        out.push_str("hierarchy rows:\n");
        for row in &self.hierarchy {
            out.push_str(&format!("  {row}\n"));
        }
        out.push_str(&format!("inspector: {}\n", self.inspector));
        out.push_str(&format!("gizmo: {}\n", self.gizmo));
        out.push_str(&format!("material: {}\n", self.material));
        out.push_str("undo/redo (each a new revision, nothing lost):\n");
        for line in &self.undo_redo {
            out.push_str(&format!("  {line}\n"));
        }
        out.push_str(&format!("logs: {}\n", self.logs));
        out.push_str(&format!("isolated play: {}\n", self.play));
        out.push_str(&format!("forced crash: {}\n", self.crash));
        if self.play_in.is_empty() {
            out.push_str("in-viewport loop: (library run; the editor exe appends its loop)\n");
        } else {
            out.push_str(&format!("in-viewport loop: {}\n", self.play_in));
        }
        out.push_str(&format!(
            "save: {} | reopen: {} | byte-identical: {}\n",
            self.saved, self.reopened, self.bytes_match
        ));
        out.push_str(&format!(
            "revision r{} with {} nodes; save v2 persists hierarchy plus revision \
             plus material bindings (references)\n",
            self.revision, self.nodes
        ));
        out.push_str(&format!("capture method: {}\n", self.capture_method));
        out.push_str("captures (file, bytes, fnv):\n");
        for line in &self.captures {
            out.push_str(&format!("  {line}\n"));
        }
        out
    }
}

/// Deterministic session node id for `slot` (1 = wall, 2 = hero).
///
/// Fixed `a3e70001-0000-4000-8000-<slot>` UUIDs shaped like minted ids but
/// constant across runs, so `scene.edoc` bytes match run to run.
fn deterministic_id(slot: u32) -> Result<ObjectId, A3Error> {
    let text = format!("a3e70001-0000-4000-8000-{slot:012x}");
    ObjectId::parse(&text)
        .map_err(|_| A3Error::Check(format!("session slot {slot} id is not a UUID")))
}

/// Read one content file or stop with its path.
fn read_content(root: &Path, rel: &str) -> Result<Vec<u8>, A3Error> {
    let path = root.join(rel);
    std::fs::read(&path).map_err(|error| A3Error::Io {
        path: path.to_string_lossy().into_owned(),
        reason: error.to_string(),
    })
}

/// Write one evidence file or stop with its path.
fn write_out(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), A3Error> {
    let path = dir.join(name);
    std::fs::write(&path, bytes).map_err(|error| A3Error::Io {
        path: path.to_string_lossy().into_owned(),
        reason: error.to_string(),
    })
}

/// Check the manifest names the file it must cover.
fn check_manifest(manifest: &str, file: &str, entry: &str) -> Result<(), A3Error> {
    if manifest.contains(entry) {
        Ok(())
    } else {
        Err(A3Error::Manifest {
            file: file.to_string(),
            missing: entry.to_string(),
        })
    }
}

/// A stub game: sleeps ~30 s, long enough to watch and kill.
#[cfg(windows)]
fn stub_entrypoint() -> PlayEntrypoint {
    PlayEntrypoint::new("cmd", ["/C", "ping", "-n", "30", "127.0.0.1"])
}

/// A stub game: sleeps ~30 s, long enough to watch and kill.
#[cfg(not(windows))]
fn stub_entrypoint() -> PlayEntrypoint {
    PlayEntrypoint::new("sleep", ["30"])
}

/// A stub game that exits 1 at once: the abrupt-crash stand-in.
#[cfg(windows)]
fn crash_entrypoint() -> PlayEntrypoint {
    PlayEntrypoint::new("cmd", ["/C", "exit", "1"])
}

/// A stub game that exits 1 at once: the abrupt-crash stand-in.
#[cfg(not(windows))]
fn crash_entrypoint() -> PlayEntrypoint {
    PlayEntrypoint::new("sh", ["-c", "exit 1"])
}

/// Pick the hero through the shell viewport: center pixel first (the default
/// framing looks at the origin), then a coarse scan. Returns the winning
/// pixel plus a line describing which path hit. Pixels stay `f32` (the
/// viewport ray takes meters-space floats), formatted whole in the receipt.
fn pick_hero(shell: &Shell, hero: ObjectId) -> Result<((f32, f32), String), A3Error> {
    let width = shell.viewport.size.width;
    let height = shell.viewport.size.height;
    let cx = (width / 2.0).round();
    let cy = (height / 2.0).round();
    if shell.pick(cx, cy) == Some(hero) {
        return Ok((
            (cx, cy),
            format!("center pixel ({cx:.0},{cy:.0}) picked the hero"),
        ));
    }
    let mut y = 0.0f32;
    while y < height {
        let mut x = 0.0f32;
        while x < width {
            if shell.pick(x, y) == Some(hero) {
                return Ok((
                    (x, y),
                    format!("scan pixel ({x:.0},{y:.0}) picked the hero"),
                ));
            }
            x += 8.0;
        }
        y += 8.0;
    }
    Err(A3Error::Check(
        "viewport pick missed the hero on every scanned pixel".to_string(),
    ))
}

/// Pose x of a node, or a check failure naming it.
fn pose_x(shell: &Shell, node: ObjectId, what: &str) -> Result<f32, A3Error> {
    shell
        .document()
        .hierarchy
        .local(node)
        .map(|local| local.position[0])
        .ok_or_else(|| A3Error::Check(format!("{what}: hero pose missing")))
}

/// Loaded licensed bytes plus receipt rows: hero mesh, crate mesh, sources.
type SourceBundle = (Vec<u8>, Vec<u8>, Vec<String>);

/// Licensed content plus receipt rows: hero and crate bytes, each covered by
/// its manifest, with the two source lines for the report.
fn read_sources(content_root: &Path) -> Result<SourceBundle, A3Error> {
    let hero = read_content(content_root, HERO_MESH)?;
    if hero.len() < 4 || hero[0..4] != GLB_MAGIC {
        return Err(A3Error::BadMagic {
            file: HERO_MESH.to_string(),
            expected: "glTF",
        });
    }
    let crate_bytes = read_content(content_root, CRATE_MESH)?;
    let crate_text = core::str::from_utf8(&crate_bytes).map_err(|_| A3Error::BadMagic {
        file: CRATE_MESH.to_string(),
        expected: "JSON",
    })?;
    if !crate_text.contains("\"asset\"") {
        return Err(A3Error::BadMagic {
            file: CRATE_MESH.to_string(),
            expected: "JSON",
        });
    }
    let arena_manifest = read_content(content_root, ARENA_MANIFEST)?;
    let arena_text = core::str::from_utf8(&arena_manifest).map_err(|_| A3Error::BadMagic {
        file: ARENA_MANIFEST.to_string(),
        expected: "JSON",
    })?;
    check_manifest(arena_text, ARENA_MANIFEST, "src/crate.gltf")?;
    let fighters_manifest = read_content(content_root, FIGHTERS_MANIFEST)?;
    let fighters_text =
        core::str::from_utf8(&fighters_manifest).map_err(|_| A3Error::BadMagic {
            file: FIGHTERS_MANIFEST.to_string(),
            expected: "JSON",
        })?;
    check_manifest(fighters_text, FIGHTERS_MANIFEST, "CC0-1.0")?;
    check_manifest(fighters_text, FIGHTERS_MANIFEST, "hero")?;
    let sources = vec![
        format!(
            "hero mesh: {HERO_MESH} ({} bytes, CC0-1.0 via OGA System G6 (fighters manifest))",
            hero.len()
        ),
        format!(
            "crate mesh: {CRATE_MESH} ({} bytes, CC0-1.0 originals (arena manifest))",
            crate_bytes.len()
        ),
    ];
    Ok((hero, crate_bytes, sources))
}

/// Asset bundle for the run; the hero lookup proves search answers.
fn build_bundle() -> Result<BundleView, A3Error> {
    let mut bundle = BundleView::new();
    for (kind, name, tags) in [
        (AssetKind::Mesh, "hero_body", &["fighter", "hero"][..]),
        (AssetKind::Mesh, "crate", &["prop"][..]),
        (AssetKind::Material, "hero-mat", &["fighter"][..]),
    ] {
        bundle
            .push(BundleEntry::new(kind, name, tags))
            .map_err(|error| A3Error::Browser(error.to_string()))?;
    }
    let hero_hits = search(&bundle, &SearchQuery::new("hero", None, None))
        .map_err(|error| A3Error::Browser(error.to_string()))?;
    if !hero_hits.iter().any(|hit| hit.name == "hero_body") {
        return Err(A3Error::Check("search `hero` missed hero_body".to_string()));
    }
    Ok(bundle)
}

/// Wall plus hero at the origin (the default framing looks there, so the
/// center pick lands on the hero sphere). Returns `(wall, hero)`.
fn place_scene(shell: &mut Shell) -> Result<(ObjectId, ObjectId), A3Error> {
    let wall = deterministic_id(1)?;
    shell
        .insert_node(
            wall,
            ObjectId::NIL,
            Transform::new([-2.0, 1.5, -8.0], [0.0, 0.0, 0.0, 1.0], [8.0, 3.0, 1.0]),
        )
        .map_err(|error| A3Error::Shell(error.to_string()))?;
    let hero = deterministic_id(2)?;
    shell
        .insert_node(
            hero,
            ObjectId::NIL,
            Transform::new([0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
        )
        .map_err(|error| A3Error::Shell(error.to_string()))?;
    Ok((wall, hero))
}

/// Viewport line, hero pick and hierarchy rows; selects the hero for the
/// panel steps that follow. Returns `(viewport, picking, hierarchy)`.
fn frame_view(shell: &mut Shell, hero: ObjectId) -> Result<(String, String, Vec<String>), A3Error> {
    let viewport = format!(
        "panel {:.0}x{:.0} px, revision r{}",
        shell.viewport.size.width,
        shell.viewport.size.height,
        shell.revision()
    );
    let ((px, py), pick_how) = pick_hero(shell, hero)?;
    let picking = format!("{pick_how}; ray at ({px:.0},{py:.0}) resolves node {hero}");
    let mut hierarchy = Vec::new();
    for row in shell.rows() {
        hierarchy.push(format!(
            "{} parent={} depth={} pos=({:.2},{:.2},{:.2})",
            row.id,
            row.parent,
            row.depth,
            row.local.position[0],
            row.local.position[1],
            row.local.position[2]
        ));
    }
    if shell.rows().len() != 2 {
        return Err(A3Error::Check(
            "hierarchy does not hold two rows".to_string(),
        ));
    }
    if !shell.select(hero) {
        return Err(A3Error::Check(
            "hierarchy select missed the hero".to_string(),
        ));
    }
    Ok((viewport, picking, hierarchy))
}

/// Inspector schema edit of the hero pose lane through the service.
fn inspector_edit(shell: &mut Shell, hero: ObjectId) -> Result<String, A3Error> {
    let before = pose_x(shell, hero, "inspector")?;
    shell
        .auto_edit("pose", "pose.px", "1.5")
        .map_err(|error| A3Error::Shell(error.to_string()))?;
    let receipt = format!(
        "node {hero} pose.px {before:.2} -> 1.50 (inspector schema edit, r{})",
        shell.revision()
    );
    if (pose_x(shell, hero, "inspector")? - 1.5).abs() > 1e-6 {
        return Err(A3Error::Check(
            "inspector pose.px edit did not land".to_string(),
        ));
    }
    Ok(receipt)
}

/// Translate drag on the selection, committed through the service.
fn gizmo_move(shell: &mut Shell, hero: ObjectId) -> Result<String, A3Error> {
    let drag = GizmoDrag::Translate {
        delta: [1.0, 0.0, 0.0],
    };
    shell
        .apply_gizmo_drag(&drag)
        .map_err(|error| A3Error::Shell(error.to_string()))?;
    let receipt = format!(
        "node {hero} gizmo translate +1.00 m on x -> {:.2} (r{})",
        pose_x(shell, hero, "gizmo")?,
        shell.revision()
    );
    if (pose_x(shell, hero, "gizmo")? - 2.5).abs() > 1e-6 {
        return Err(A3Error::Check("gizmo drag did not land".to_string()));
    }
    Ok(receipt)
}

/// Browser material bind (journaled, saved in the bytes) plus the refusal
/// of an unknown material, which must write nothing.
fn material_step(
    shell: &mut Shell,
    hero: ObjectId,
    bundle: &BundleView,
) -> Result<String, A3Error> {
    let revision_before_mat = shell.revision();
    shell
        .bind_material(hero, "hero-mat", bundle)
        .map_err(|error| A3Error::Shell(error.to_string()))?;
    if shell.revision() != revision_before_mat + 1 {
        return Err(A3Error::Check(
            "material edit did not bump the revision".to_string(),
        ));
    }
    let revision_before_bad = shell.revision();
    match shell.bind_material(hero, "no-such-mat", bundle) {
        Ok(_) => {
            return Err(A3Error::Check(
                "unknown material bound instead of refusing".to_string(),
            ))
        }
        Err(error) => {
            shell.log_message(format!("refused unknown material: {error}"));
        }
    }
    if shell.revision() != revision_before_bad {
        return Err(A3Error::Check(
            "refused material edit moved the revision".to_string(),
        ));
    }
    Ok(format!(
        "node {hero} bound to material `hero-mat` (r{revision_before_mat} -> r{})",
        shell.revision()
    ))
}

/// Walk the material bind and the gizmo move back and forth: every step is
/// a new revision and the poses/bindings roundtrip exactly.
fn undo_redo_cycle(shell: &mut Shell, hero: ObjectId) -> Result<Vec<String>, A3Error> {
    let mut undo_redo = Vec::new();
    if shell.undo().is_none() {
        return Err(A3Error::Check("undo stack empty at bind".to_string()));
    }
    undo_redo.push(format!(
        "undo bind -> r{} (material cleared: {})",
        shell.revision(),
        shell.material_of(hero).is_none()
    ));
    if shell.material_of(hero).is_some() {
        return Err(A3Error::Check("undo did not clear the binding".to_string()));
    }
    if shell.undo().is_none() {
        return Err(A3Error::Check("undo stack empty at gizmo".to_string()));
    }
    undo_redo.push(format!(
        "undo gizmo -> r{} (pose.px {:.2})",
        shell.revision(),
        pose_x(shell, hero, "undo")?
    ));
    if (pose_x(shell, hero, "undo")? - 1.5).abs() > 1e-6 {
        return Err(A3Error::Check(
            "undo did not restore the gizmo move".to_string(),
        ));
    }
    if shell.redo().is_none() {
        return Err(A3Error::Check("redo stack empty at gizmo".to_string()));
    }
    undo_redo.push(format!(
        "redo gizmo -> r{} (pose.px {:.2})",
        shell.revision(),
        pose_x(shell, hero, "redo")?
    ));
    if (pose_x(shell, hero, "redo")? - 2.5).abs() > 1e-6 {
        return Err(A3Error::Check(
            "redo did not replay the gizmo move".to_string(),
        ));
    }
    if shell.redo().is_none() {
        return Err(A3Error::Check("redo stack empty at bind".to_string()));
    }
    undo_redo.push(format!(
        "redo bind -> r{} (material `{}`)",
        shell.revision(),
        shell
            .material_of(hero)
            .map(|name| name.as_str().to_string())
            .unwrap_or_else(|| "<none>".to_string())
    ));
    if !shell
        .material_of(hero)
        .is_some_and(|name| name.as_str() == "hero-mat")
    {
        return Err(A3Error::Check(
            "redo did not replay the binding".to_string(),
        ));
    }
    Ok(undo_redo)
}

/// Checkpoint line over the log panel, which kept every receipt.
fn checkpoint_logs(shell: &mut Shell) -> Result<String, A3Error> {
    shell.log_message(format!("recorded run checkpoint at r{}", shell.revision()));
    let tail = shell.log().iter().last().unwrap_or("<empty>").to_string();
    let logs = format!("log panel holds {} lines; tail: {tail}", shell.log().len());
    if shell.log().is_empty() {
        return Err(A3Error::Check("log panel lost its lines".to_string()));
    }
    Ok(logs)
}

/// Isolated play: the game runs in a supervised child while the editor stays
/// responsive (an inspector edit lands mid-play), then the child is
/// force-stopped and the document is untouched by its death.
fn isolated_play(shell: &mut Shell, wall: ObjectId) -> Result<String, A3Error> {
    let entry = stub_entrypoint();
    let mut game = ChildPlay::spawn(&entry).map_err(|error| A3Error::Play(error.to_string()))?;
    if !game.is_alive() {
        return Err(A3Error::Play("stub game died at once".to_string()));
    }
    let revision_before_play = shell.revision();
    if !shell.select(wall) {
        return Err(A3Error::Check(
            "play-time select missed the wall".to_string(),
        ));
    }
    shell
        .auto_edit("pose", "pose.px", "-1.0")
        .map_err(|error| A3Error::Shell(error.to_string()))?;
    if shell.revision() != revision_before_play + 1 {
        return Err(A3Error::Check("play-time edit did not land".to_string()));
    }
    if !game.is_alive() {
        return Err(A3Error::Play("game died before the kill".to_string()));
    }
    game.kill()
        .map_err(|error| A3Error::Play(error.to_string()))?;
    if game.is_alive() {
        return Err(A3Error::Play("killed game still reads alive".to_string()));
    }
    Ok(format!(
        "child `{entry}` ran isolated (editor edited r{revision_before_play} -> r{} mid-play); \
         forced stop reaped it, document intact",
        shell.revision()
    ))
}

/// Forced crash: a game that exits abruptly on its own. The editor notes the
/// death, keeps every edit (bytes plus journal depth unchanged), and the
/// service still applies afterwards.
fn crash_recovery(shell: &mut Shell, hero: ObjectId) -> Result<String, A3Error> {
    let bytes_before = save_to_bytes(shell.document());
    let journal_before = shell.journal().past.len();
    let revision_before_crash = shell.revision();
    let crash_entry = crash_entrypoint();
    let mut doomed =
        ChildPlay::spawn(&crash_entry).map_err(|error| A3Error::Play(error.to_string()))?;
    let mut waited = 0u32;
    while doomed.is_alive() && waited < 200 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        waited += 1;
    }
    if doomed.is_alive() {
        doomed
            .kill()
            .map_err(|error| A3Error::Play(error.to_string()))?;
    }
    if doomed.is_alive() {
        return Err(A3Error::Play("crashed game still reads alive".to_string()));
    }
    shell.log_message(format!(
        "game process `{crash_entry}` terminated; edits kept"
    ));
    if save_to_bytes(shell.document()) != bytes_before {
        return Err(A3Error::Check(
            "crash changed the document bytes".to_string(),
        ));
    }
    if shell.journal().past.len() != journal_before {
        return Err(A3Error::Check("crash dropped journal entries".to_string()));
    }
    if !shell.select(hero) {
        return Err(A3Error::Check(
            "post-crash select missed the hero".to_string(),
        ));
    }
    shell
        .auto_edit("pose", "pose.rx", "0.5")
        .map_err(|error| A3Error::Shell(error.to_string()))?;
    if shell.revision() != revision_before_crash + 1 {
        return Err(A3Error::Check("post-crash edit did not land".to_string()));
    }
    Ok(format!(
        "child `{crash_entry}` terminated abruptly; no edit lost \
         (bytes plus {} journal entries kept, r{revision_before_crash} -> r{} after)",
        journal_before,
        shell.revision()
    ))
}

/// Live save outputs: revision, node count, scene bytes, viewport PNG and
/// the method that produced the PNG.
struct LiveSave {
    revision: u64,
    nodes: usize,
    bytes: Vec<u8>,
    png: Vec<u8>,
    method: String,
}

/// Save the live document plus its rows, journal and viewport PNG.
fn save_live(
    shell: &Shell,
    out_dir: &Path,
    exe_capture: Option<ExeCapture<'_>>,
) -> Result<LiveSave, A3Error> {
    let revision = shell.revision();
    let nodes = shell.document().hierarchy.len();
    let bytes = save_to_bytes(shell.document());
    write_out(out_dir, "scene.edoc", &bytes)?;
    let mut rows_text = String::new();
    for row in shell.rows() {
        rows_text.push_str(&format!(
            "{} parent={} depth={} pos=({:.2},{:.2},{:.2})\n",
            row.id,
            row.parent,
            row.depth,
            row.local.position[0],
            row.local.position[1],
            row.local.position[2]
        ));
    }
    write_out(out_dir, "rows.txt", rows_text.as_bytes())?;
    let mut journal_text = String::new();
    for entry in &shell.journal().past {
        for line in &entry.effects {
            journal_text.push_str(&format!("r{} {line}\n", entry.result_revision));
        }
    }
    write_out(out_dir, "journal.txt", journal_text.as_bytes())?;
    let (png, method) = match exe_capture.and_then(|capture| capture(shell)) {
        Some(pair) => pair,
        None => (
            capture_viewport_png(shell).map_err(|error| A3Error::Capture(error.to_string()))?,
            "headless-cpu-raster (library fallback; the editor-exe record \
             overwrites these PNGs with window-path captures)"
                .to_string(),
        ),
    };
    Ok(LiveSave {
        revision,
        nodes,
        bytes,
        png,
        method,
    })
}

/// Reopened save outputs: resaved bytes plus the capture lines.
struct ReopenedSave {
    resaved: Vec<u8>,
    captures: Vec<String>,
}

/// Close, reopen and byte-compare: the reopened document equals the saved
/// one, resaving gives back the exact same bytes, and the material binding
/// survives in the scene bytes.
fn reopen_verify(
    out_dir: &Path,
    hero: ObjectId,
    live: &LiveSave,
    exe_capture: Option<ExeCapture<'_>>,
) -> Result<ReopenedSave, A3Error> {
    let saved_back = read_content(out_dir, "scene.edoc")?;
    if saved_back != live.bytes {
        return Err(A3Error::Check(
            "scene file differs from saved bytes".to_string(),
        ));
    }
    let reopened =
        load_from_bytes(&saved_back).map_err(|error| A3Error::Save(error.to_string()))?;
    let resaved = save_to_bytes(&reopened);
    write_out(out_dir, "reopened.edoc", &resaved)?;
    if resaved != live.bytes {
        return Err(A3Error::Check(
            "reopened scene resaved to different bytes".to_string(),
        ));
    }
    if reopened.revision != live.revision {
        return Err(A3Error::Check("reopened revision moved".to_string()));
    }
    if reopened.hierarchy.len() != live.nodes {
        return Err(A3Error::Check("reopened node count moved".to_string()));
    }
    if !reopened
        .materials
        .get(&hero.to_string())
        .is_some_and(|name| name.as_str() == "hero-mat")
    {
        return Err(A3Error::Check(
            "reopened document lost the hero-mat binding".to_string(),
        ));
    }
    let needle = b"hero-mat";
    if !live.bytes.windows(needle.len()).any(|w| w == needle) {
        return Err(A3Error::Check(
            "scene bytes do not carry the bound material name".to_string(),
        ));
    }
    let mut reopened_shell = Shell::new();
    reopened_shell.open_document(reopened.clone());
    let png = match exe_capture.and_then(|capture| capture(&reopened_shell)) {
        Some((png, _)) => png,
        None => capture_viewport_png(&reopened_shell)
            .map_err(|error| A3Error::Capture(error.to_string()))?,
    };
    write_out(out_dir, "viewport.png", &live.png)?;
    write_out(out_dir, "reopened.png", &png)?;
    let captures = vec![
        format!(
            "viewport.png ({} bytes, fnv {})",
            live.png.len(),
            fnv_hex(&live.png)
        ),
        format!("reopened.png ({} bytes, fnv {})", png.len(), fnv_hex(&png)),
    ];
    Ok(ReopenedSave { resaved, captures })
}

/// Run the full recorded editor session and write its evidence files.
/// See the module header for the step list. `exe_capture` works like the
/// companion session run: the executable supplies window-path PNGs, the
/// library falls back to the headless raster.
pub fn run_session(
    content_root: &Path,
    out_dir: &Path,
    exe_capture: Option<ExeCapture<'_>>,
) -> Result<EditorReport, A3Error> {
    std::fs::create_dir_all(out_dir).map_err(|error| A3Error::Io {
        path: out_dir.to_string_lossy().into_owned(),
        reason: error.to_string(),
    })?;

    // Licensed bytes in: hero mesh plus crate mesh, each covered by a manifest.
    let (_, _, sources) = read_sources(content_root)?;

    // Browser: the imported assets answer search (placement path).
    let bundle = build_bundle()?;

    // Hierarchy: wall plus hero at the origin (the default framing looks
    // there, so the center pick below lands on the hero sphere).
    let mut shell = Shell::new();
    let (wall, hero_id) = place_scene(&mut shell)?;

    // Viewport, picking and hierarchy panel; selects the hero for the edits.
    let (viewport, picking, hierarchy) = frame_view(&mut shell, hero_id)?;

    // Inspector: schema edit of the hero pose lane through the service.
    let inspector = inspector_edit(&mut shell, hero_id)?;

    // Gizmo: a translate drag on the selection commits through the service.
    let gizmo = gizmo_move(&mut shell, hero_id)?;

    // Material: bind the browser material (journaled, saved in the bytes).
    let material = material_step(&mut shell, hero_id, &bundle)?;

    // Undo/redo: walk the material bind and the gizmo move back and forth.
    // Every step is a new revision and the poses/bindings roundtrip exactly.
    let undo_redo = undo_redo_cycle(&mut shell, hero_id)?;

    // Logs: the panel kept every receipt plus the refusal note.
    let logs = checkpoint_logs(&mut shell)?;

    // Isolated play: the game runs in a supervised child while the editor
    // stays responsive (an inspector edit lands mid-play), then the child is
    // force-stopped and the document is untouched by its death.
    let play = isolated_play(&mut shell, wall)?;

    // Forced crash: a game that exits abruptly on its own. The editor notes
    // the death, keeps every edit (bytes plus journal depth unchanged), and
    // the service still applies afterwards.
    let crash = crash_recovery(&mut shell, hero_id)?;

    // Save, close, reopen: the reopened document equals the saved one and
    // saving it again gives back the exact same bytes.
    let live = save_live(&shell, out_dir, exe_capture)?;
    drop(shell);

    let reopened_out = reopen_verify(out_dir, hero_id, &live, exe_capture)?;
    let bytes_match = reopened_out.resaved == live.bytes;

    let report = EditorReport {
        sources,
        viewport,
        picking,
        hierarchy,
        inspector,
        gizmo,
        material,
        undo_redo,
        logs,
        play,
        crash,
        play_in: String::new(),
        saved: format!(
            "scene.edoc ({} bytes, fnv {})",
            live.bytes.len(),
            fnv_hex(&live.bytes)
        ),
        reopened: format!(
            "reopened.edoc ({} bytes, fnv {})",
            reopened_out.resaved.len(),
            fnv_hex(&reopened_out.resaved)
        ),
        bytes_match,
        revision: live.revision,
        nodes: live.nodes,
        captures: reopened_out.captures,
        capture_method: live.method,
    };
    write_out(out_dir, "report.txt", report.render().as_bytes())?;
    Ok(report)
}

/// Content root of the lane checkout holding `content/`, resolved at
/// runtime from the launch or executable directory (owner:
/// [`crate::a2_session::checkout_root`], so no absolute source path bakes).
pub fn lane_content_root() -> PathBuf {
    crate::a2_session::checkout_root().join("content")
}

/// Default evidence dir for the recorded editor run inside the checkout.
pub fn lane_evidence_dir() -> PathBuf {
    crate::a2_session::checkout_root()
        .join("target")
        .join("evidence")
        .join("a3")
}

/// In-viewport loop smoke for the executable record: start the hosted loop
/// on a copy of the recorded document, drive three frames with neutral
/// inputs, checkpoint, stop. The document is untouched; the returned line
/// names ticks and the checkpoint for the report.
#[cfg(feature = "ui")]
pub fn play_in_smoke(
    doc: &crate::commands::Document,
) -> Result<String, crate::play_in::PlayInError> {
    use crate::instant::RoundInput;
    use crate::play::PlayEntrypoint;
    use crate::play_in::PlayInSession;

    let entry = PlayEntrypoint::new("recorded-game", Vec::<String>::new());
    let mut session = PlayInSession::start(doc.clone(), entry, 7)?;
    for _ in 0..3 {
        session.frame(1.0 / 60.0, &[RoundInput::neutral()])?;
    }
    let checkpoint = session.checkpoint()?;
    let line = format!(
        "hosted loop: 3 frames -> tick {} checkpoint {checkpoint}",
        session.tick()
    );
    session.stop()?;
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_ids_are_stable_and_shaped() {
        assert_eq!(deterministic_id(1).unwrap(), deterministic_id(1).unwrap());
        assert_ne!(deterministic_id(1).unwrap(), deterministic_id(2).unwrap());
        assert_eq!(deterministic_id(1).unwrap().as_uuid().get_version_num(), 4);
    }

    #[test]
    fn pick_scan_finds_the_hero() {
        let mut shell = Shell::new();
        let hero = deterministic_id(7).unwrap();
        shell
            .insert_node(
                hero,
                ObjectId::NIL,
                Transform::new([0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
            )
            .unwrap();
        let ((px, py), _) = pick_hero(&shell, hero).unwrap();
        assert_eq!(shell.pick(px, py), Some(hero));
        // A removed hero is unpickable: the scan fails loudly instead of
        // returning a stale node.
        shell.remove_node(hero).unwrap();
        assert!(pick_hero(&shell, hero).is_err());
    }

    #[test]
    fn full_run_survives_a_crash_with_no_lost_edit() {
        let content = lane_content_root();
        let hero_path = content.join(HERO_MESH);
        assert!(
            hero_path.exists(),
            "missing content fixture {}: a sparse checkout must fail loudly, never green silently",
            hero_path.display()
        );
        let scratch = std::env::temp_dir().join("recorded-editor-scratch");
        let _ = std::fs::remove_dir_all(&scratch);
        let report = run_session(&content, &scratch, None).unwrap();
        assert!(report.bytes_match);
        assert_eq!(report.nodes, 2);
        assert_eq!(report.undo_redo.len(), 4);
        assert!(report.play.contains("mid-play"));
        assert!(report.crash.contains("no edit lost"));
        assert!(report.play_in.is_empty());
        let saved = std::fs::read(scratch.join("scene.edoc")).unwrap();
        let reopened = std::fs::read(scratch.join("reopened.edoc")).unwrap();
        assert_eq!(saved, reopened);
        let journal = std::fs::read_to_string(scratch.join("journal.txt")).unwrap();
        assert!(journal.contains("set-material"));
        assert!(journal.contains("undo batch key") || journal.contains("set-local"));
        for png in ["viewport.png", "reopened.png"] {
            let bytes = std::fs::read(scratch.join(png)).unwrap();
            assert_eq!(&bytes[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        }
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn two_runs_write_identical_scene_bytes() {
        let content = lane_content_root();
        let hero_path = content.join(HERO_MESH);
        assert!(
            hero_path.exists(),
            "missing content fixture {}: a sparse checkout must fail loudly, never green silently",
            hero_path.display()
        );
        let first_dir = std::env::temp_dir().join("recorded-editor-cross-a");
        let second_dir = std::env::temp_dir().join("recorded-editor-cross-b");
        let _ = std::fs::remove_dir_all(&first_dir);
        let _ = std::fs::remove_dir_all(&second_dir);
        let first = run_session(&content, &first_dir, None).unwrap();
        let second = run_session(&content, &second_dir, None).unwrap();
        assert_eq!(first.saved, second.saved);
        // No wall-clock text leaks into the rerun: whole reports match.
        assert_eq!(first.render(), second.render());
        let a = std::fs::read(first_dir.join("scene.edoc")).unwrap();
        let b = std::fs::read(second_dir.join("scene.edoc")).unwrap();
        assert_eq!(a, b, "cross-run scene.edoc bytes match");
        let _ = std::fs::remove_dir_all(&first_dir);
        let _ = std::fs::remove_dir_all(&second_dir);
    }

    #[test]
    fn roots_share_the_runtime_checkout() {
        assert_eq!(
            lane_content_root(),
            crate::a2_session::checkout_root().join("content")
        );
        assert_eq!(
            lane_evidence_dir(),
            crate::a2_session::checkout_root()
                .join("target")
                .join("evidence")
                .join("a3")
        );
    }
}
