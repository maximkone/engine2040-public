//! `fx` pack: one-call physical effects over a session's float physfx world.
//!
//! `fx.ragdoll`, `fx.limb`, `fx.break` and `fx.blast` take a body id plus the
//! effect's own arguments and wrap `engine_physfx`'s verbs. The world is a
//! presentation layer: it is never hashed and never feeds the fixed tick. The
//! body id names a file under `content/bodies` (`fighter`), so a missing body
//! refuses `bad_args` with the id in the message.
//!
//! Each effect call is one recorded action: `action_id` is the idempotency key. The
//! same id with the same effect returns the stored receipt and adds no second
//! undo entry; a rebuilt world replays the kept actions, so one undo restores
//! the body set exactly, and `fx.undo` drops the newest action through the
//! door. The answer is the `physfx:` receipt (body, joints, chunks, impulse)
//! plus the capture path when a host capture hook drew a frame.
//!
//! Errors: [`McpToolError`] with `bad_args` naming the argument, the legal
//! range and one recovery; a physfx refusal leaves the world as it was.

use std::collections::btree_map::Entry as MapEntry;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use engine_anim::contract::LocalPose;
use engine_physfx::{
    fx_blast, fx_break, fx_limb, fx_ragdoll, load_body, Blast, BlastEvent, BodyDesc, BreakEvent,
    Error as PhysFxError, LimbEvent, PhysFx, RagdollEvent, Shard, Tier, Transform, MAX_BLAST_NS,
};
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use super::bad_args as bad;
use super::{PackDef, PackError, VerbSpec};
use crate::mcp::{McpServer, McpToolError};
use crate::tool_capabilities::CapabilityCard;

/// Pack name.
pub const PACK: &str = "fx";
/// Verb: turn a body into a ragdoll.
pub const TOOL_FX_RAGDOLL: &str = "fx.ragdoll";
/// Verb: paint a severed limb.
pub const TOOL_FX_LIMB: &str = "fx.limb";
/// Verb: paint a debris break.
pub const TOOL_FX_BREAK: &str = "fx.break";
/// Verb: shove the bodies a blast reaches.
pub const TOOL_FX_BLAST: &str = "fx.blast";
/// Verb: undo the newest recorded effect.
pub const TOOL_FX_UNDO: &str = "fx.undo";

/// Args for `fx.ragdoll`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct FxRagdollArgs {
    /// Idempotency key: the same id and effect replay the stored receipt with no second undo entry.
    pub action_id: u64,
    /// Body id under `content/bodies`, such as `fighter`.
    pub body: String,
    /// Fighter slot the ragdoll lies in.
    pub slot: u8,
    /// Bone positions in the body's own order, meters; omit for the rest pose.
    #[serde(default)]
    pub pose: Option<Vec<[f32; 3]>>,
    /// How fast the fighter was moving, meters per second.
    #[serde(default)]
    pub velocity: Option<[f32; 3]>,
    /// The blow that put it down, newton-seconds.
    #[serde(default)]
    pub impulse: Option<[f32; 3]>,
}

/// Args for `fx.limb`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct FxLimbArgs {
    /// Idempotency key: the same id and effect replay the stored receipt with no second undo entry.
    pub action_id: u64,
    /// Body id under `content/bodies`, such as `fighter`.
    pub body: String,
    /// Fighter slot the piece came from.
    pub slot: u8,
    /// Cuttable bone name the body file gives, such as `arm_lower_l`.
    pub bone: String,
    /// Part id the severed piece is known by.
    pub part: u32,
    /// Where the piece starts, meters.
    #[serde(default)]
    pub at: Option<[f32; 3]>,
    /// How fast the piece leaves, meters per second.
    #[serde(default)]
    pub velocity: Option<[f32; 3]>,
}

/// Args for `fx.break`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct FxBreakArgs {
    /// Idempotency key: the same id and effect replay the stored receipt with no second undo entry.
    pub action_id: u64,
    /// Body id under `content/bodies`, such as `fighter`.
    pub body: String,
    /// Pieces to spawn, 1..=64, laid along +X from `at`.
    pub chunks: u32,
    /// First piece pose, meters.
    #[serde(default)]
    pub at: Option<[f32; 3]>,
    /// Box half-extents of every piece, meters.
    #[serde(default)]
    pub half: Option<[f32; 3]>,
    /// The break's total momentum, newton-seconds.
    #[serde(default)]
    pub impulse: Option<[f32; 3]>,
}

/// Args for `fx.blast`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct FxBlastArgs {
    /// Idempotency key: the same id and effect replay the stored receipt with no second undo entry.
    pub action_id: u64,
    /// Body id under `content/bodies`, such as `fighter`.
    pub body: String,
    /// Where it went off, millimeters.
    pub center_mm: [i32; 3],
    /// How far it reaches, millimeters, 1..=100000.
    pub radius_mm: u32,
    /// Blast momentum, newton-seconds, 1..=1000.
    pub impulse_ns: u32,
}

/// Args for `fx.undo`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct FxUndoArgs {
    /// Action id to undo; omit to undo the newest recorded effect.
    #[serde(default)]
    pub action_id: Option<u64>,
}

/// What one undo removed.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct FxUndoOutput {
    /// The action id the undo removed.
    pub undone: u64,
    /// Effects this slot can still undo.
    pub undo_depth: u32,
    /// Bodies the float world holds after the undo.
    pub bodies: u32,
}

/// What one effect painted, for the page and the window.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct FxOutput {
    /// Body id the call named.
    pub body: String,
    /// Bones the body carries.
    pub joints: u32,
    /// Pieces the call added.
    pub chunks: u32,
    /// The effect's impulse, newton-seconds (the blast runs along +X).
    pub impulse: [f32; 3],
    /// Bodies the blast shoved, 0 for the other effects.
    pub pushed: u32,
    /// The `physfx:` receipt line.
    pub receipt: String,
    /// Capture path when a host capture leg drew a frame; null otherwise.
    pub capture: Option<String>,
    /// Actions this slot can still undo.
    pub undo_depth: u32,
}

/// One effect, kept so an undo can rebuild the world from the same inputs.
#[derive(Clone, Debug, PartialEq)]
enum FxAction {
    /// A whole ragdoll.
    Ragdoll(RagdollEvent),
    /// One severed limb.
    Limb(LimbEvent),
    /// A debris break.
    Break(BreakEvent),
    /// A blast.
    Blast(BlastEvent),
}

/// One landed action: what an undo replays or drops.
struct Entry {
    action_id: u64,
    body: BodyDesc,
    action: FxAction,
}

/// One idempotency slot: the effect and the receipt it answered.
struct Applied {
    body_id: String,
    action: FxAction,
    output: FxOutput,
}

/// What a physfx verb measured for the receipt.
struct FxMeasure {
    joints: u32,
    chunks: u32,
    impulse: [f32; 3],
    pushed: u32,
}

/// A session's float world plus its recorded actions.
struct FxSlot {
    world: PhysFx,
    log: Vec<Entry>,
    applied: BTreeMap<u64, Applied>,
    capture: Option<fn(&PhysFx) -> Option<String>>,
}

fn slots() -> &'static Mutex<BTreeMap<u64, FxSlot>> {
    static SLOTS: OnceLock<Mutex<BTreeMap<u64, FxSlot>>> = OnceLock::new();
    SLOTS.get_or_init(Mutex::default)
}

fn lock_slots() -> Result<std::sync::MutexGuard<'static, BTreeMap<u64, FxSlot>>, McpToolError> {
    slots().lock().map_err(|_| {
        McpToolError::new(
            "lock_poisoned",
            "fx world table is poisoned; restart the door",
        )
    })
}

/// An empty Comfort world on a floor whose top is y = 0.
fn base_world() -> Result<PhysFx, McpToolError> {
    let mut world = PhysFx::new(Tier::Comfort);
    world
        .add_static_box([50.0, 0.5, 50.0], [0.0, -0.5, 0.0])
        .map_err(refused)?;
    Ok(world)
}

/// A physfx refusal names the field and the range; keep one recovery step.
fn refused(error: PhysFxError) -> McpToolError {
    bad(format!(
        "physfx refused the effect: {error}. Next: narrow the named value and retry."
    ))
}

/// Load a body id from `content/bodies`, refusing an id that could leave it.
fn body_of(id: &str) -> Result<BodyDesc, McpToolError> {
    let ok = !id.is_empty()
        && id.len() <= 48
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if !ok {
        return Err(bad(format!(
            "body `{id}` is not a body id; use 1..=48 ascii letters, digits, `_` or `-`"
        )));
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../content/bodies")
        .join(format!("{id}.toml"));
    load_body(&path).map_err(|error| {
        bad(format!(
            "body `{id}` could not be loaded from {}: {error}. Next: name a body under content/bodies, such as `fighter`.",
            path.display()
        ))
    })
}

/// Check one finite vector without a lossy cast.
fn vector(v: [f32; 3], what: &str) -> Result<[f32; 3], McpToolError> {
    if v.iter().all(|c| c.is_finite() && c.abs() <= 10_000.0) {
        Ok(v)
    } else {
        Err(bad(format!(
            "{what} must be three finite numbers within -10000..=10000"
        )))
    }
}

/// The rest pose: every bone at the root, no turn.
fn rest_pose(body: &BodyDesc) -> LocalPose {
    LocalPose {
        positions: vec![[0.0; 3]; body.bones.len()],
        rotations: vec![Transform::IDENTITY.rot; body.bones.len()],
    }
}

/// A blast's scalar momentum as a one-axis vector, exact under the blast ceiling.
fn blast_impulse(ns: u32) -> [f32; 3] {
    let exact = u16::try_from(ns).unwrap_or(u16::MAX);
    [f32::from(exact), 0.0, 0.0]
}

fn receipt_line(output: &FxOutput) -> String {
    let [x, y, z] = output.impulse;
    let mut line = format!(
        "physfx: body={} joints={} chunks={} impulse={x:.3},{y:.3},{z:.3}",
        output.body, output.joints, output.chunks
    );
    if output.pushed > 0 {
        line.push_str(&format!(" pushed={}", output.pushed));
    }
    line
}

impl FxSlot {
    fn new() -> Result<Self, McpToolError> {
        Ok(Self {
            world: base_world()?,
            log: Vec::new(),
            applied: BTreeMap::new(),
            capture: None,
        })
    }

    /// The body keys the float world holds, in spawn order.
    #[cfg(test)]
    fn body_set(&self) -> Result<Vec<engine_physfx::Key>, McpToolError> {
        self.world
            .transforms(1.0)
            .map(|placed| placed.into_iter().map(|one| one.key).collect())
            .map_err(refused)
    }

    /// Run one effect under its id, or replay the receipt the same id stored.
    fn apply(
        &mut self,
        action_id: u64,
        body_id: &str,
        body: BodyDesc,
        action: FxAction,
    ) -> Result<FxOutput, McpToolError> {
        if let Some(entry) = self.applied.get(&action_id) {
            if entry.body_id == body_id && entry.action == action {
                let mut output = entry.output.clone();
                output.undo_depth = u32::try_from(self.log.len()).unwrap_or(u32::MAX);
                return Ok(output);
            }
            return Err(bad(format!(
                "action id {action_id} was already used with a different effect or body; use a new id (the stored action stays undoable)"
            )));
        }
        let measure = run(&mut self.world, &body, &action)?;
        let mut output = FxOutput {
            body: body_id.to_owned(),
            joints: measure.joints,
            chunks: measure.chunks,
            impulse: measure.impulse,
            pushed: measure.pushed,
            receipt: String::new(),
            capture: self.capture.and_then(|draw| draw(&self.world)),
            undo_depth: u32::try_from(self.log.len() + 1).unwrap_or(u32::MAX),
        };
        output.receipt = receipt_line(&output);
        self.applied.insert(
            action_id,
            Applied {
                body_id: body_id.to_owned(),
                action: action.clone(),
                output: output.clone(),
            },
        );
        self.log.push(Entry {
            action_id,
            body,
            action,
        });
        Ok(output)
    }

    /// Drop the newest effect: rebuild the world, then replay the older ones.
    fn undo(&mut self, action_id: Option<u64>) -> Result<FxUndoOutput, McpToolError> {
        let Some(last) = self.log.last() else {
            return Err(bad("nothing to undo; apply an fx effect first".to_owned()));
        };
        let newest = last.action_id;
        if let Some(named) = action_id {
            if named != newest {
                return Err(bad(format!(
                    "action id {named} is not the newest recorded effect; the newest undoable action is {newest} (omit action_id to undo it)"
                )));
            }
        }
        let mut world = base_world()?;
        for entry in &self.log[..self.log.len() - 1] {
            run(&mut world, &entry.body, &entry.action)?;
        }
        let bodies = world
            .transforms(1.0)
            .map_err(refused)?
            .len()
            .try_into()
            .unwrap_or(u32::MAX);
        self.world = world;
        self.log.pop();
        self.applied.remove(&newest);
        Ok(FxUndoOutput {
            undone: newest,
            undo_depth: u32::try_from(self.log.len()).unwrap_or(u32::MAX),
            bodies,
        })
    }
}

fn run(world: &mut PhysFx, body: &BodyDesc, action: &FxAction) -> Result<FxMeasure, McpToolError> {
    let joints = u32::try_from(body.bones.len()).unwrap_or(u32::MAX);
    match action {
        FxAction::Ragdoll(event) => {
            let fx = fx_ragdoll(world, body, event).map_err(refused)?;
            Ok(FxMeasure {
                joints: u32::try_from(fx.bones).unwrap_or(joints),
                chunks: 0,
                impulse: event.impulse,
                pushed: 0,
            })
        }
        FxAction::Limb(event) => {
            fx_limb(world, body, event).map_err(refused)?;
            Ok(FxMeasure {
                joints,
                chunks: 1,
                impulse: [0.0; 3],
                pushed: 0,
            })
        }
        FxAction::Break(event) => {
            let fx = fx_break(world, body, event).map_err(refused)?;
            Ok(FxMeasure {
                joints,
                chunks: u32::try_from(fx.admitted).unwrap_or(u32::MAX),
                impulse: event.impulse,
                pushed: 0,
            })
        }
        FxAction::Blast(event) => {
            let fx = fx_blast(world, body, event).map_err(refused)?;
            Ok(FxMeasure {
                joints,
                chunks: 0,
                impulse: blast_impulse(event.blast.impulse_ns),
                pushed: u32::try_from(fx.pushed).unwrap_or(u32::MAX),
            })
        }
    }
}

/// Run `f` on this session's float world under the audited dispatch seam.
fn with_slot<R>(
    server: &McpServer,
    tool: &str,
    f: impl FnOnce(&mut FxSlot) -> Result<R, McpToolError>,
) -> Result<R, McpToolError> {
    server.dispatch(tool, |state| {
        let mut slots = lock_slots()?;
        let slot = match slots.entry(state.session()) {
            MapEntry::Occupied(entry) => entry.into_mut(),
            MapEntry::Vacant(entry) => entry.insert(FxSlot::new()?),
        };
        f(slot)
    })
}

fn ragdoll(server: &McpServer, args: &FxRagdollArgs) -> Result<FxOutput, McpToolError> {
    let body = body_of(&args.body)?;
    let pose = match &args.pose {
        None => rest_pose(&body),
        Some(positions) => {
            if positions.len() != body.bones.len() {
                return Err(bad(format!(
                    "pose holds {} positions; body `{}` has {} bones (pass one position per bone in the file's order)",
                    positions.len(),
                    args.body,
                    body.bones.len()
                )));
            }
            let mut checked = Vec::with_capacity(positions.len());
            for position in positions {
                checked.push(vector(*position, "each pose position")?);
            }
            LocalPose {
                positions: checked,
                rotations: vec![Transform::IDENTITY.rot; body.bones.len()],
            }
        }
    };
    let event = RagdollEvent {
        slot: args.slot,
        pose,
        velocity: vector(args.velocity.unwrap_or([0.0; 3]), "velocity")?,
        impulse: vector(args.impulse.unwrap_or([0.0; 3]), "impulse")?,
    };
    with_slot(server, TOOL_FX_RAGDOLL, |slot| {
        slot.apply(args.action_id, &args.body, body, FxAction::Ragdoll(event))
    })
}

fn limb(server: &McpServer, args: &FxLimbArgs) -> Result<FxOutput, McpToolError> {
    let body = body_of(&args.body)?;
    if body.bone(&args.bone).is_none() {
        return Err(bad(format!(
            "body `{}` has no bone `{}`; call the body file's bone names (parents first)",
            args.body, args.bone
        )));
    }
    if body.cut_energy(&args.bone).is_none() {
        return Err(bad(format!(
            "body `{}`'s joint `{}` has no cut row; pick a joint the body file lets a blade take",
            args.body, args.bone
        )));
    }
    let event = LimbEvent {
        slot: args.slot,
        bone: args.bone.clone(),
        part: args.part,
        at: Transform {
            pos: vector(args.at.unwrap_or([0.0; 3]), "at")?,
            rot: Transform::IDENTITY.rot,
        },
        velocity: vector(args.velocity.unwrap_or([0.0; 3]), "velocity")?,
    };
    with_slot(server, TOOL_FX_LIMB, |slot| {
        slot.apply(args.action_id, &args.body, body, FxAction::Limb(event))
    })
}

fn fracture(server: &McpServer, args: &FxBreakArgs) -> Result<FxOutput, McpToolError> {
    if !(1..=64).contains(&args.chunks) {
        return Err(bad(format!(
            "chunks is {}; pass 1..=64 pieces",
            args.chunks
        )));
    }
    let body = body_of(&args.body)?;
    let at = vector(args.at.unwrap_or([0.0, 1.0, 0.0]), "at")?;
    let half = args.half.unwrap_or([0.1; 3]);
    if half.iter().any(|h| !h.is_finite() || *h < 0.01 || *h > 5.0) {
        return Err(bad(
            "half must hold three finite half-extents within 0.01..=5.0 meters".to_owned(),
        ));
    }
    let step = half[0] * 2.0 + 0.02;
    let pieces = usize::try_from(args.chunks).unwrap_or(0);
    let mut shards = Vec::with_capacity(pieces);
    for index in 0..args.chunks {
        let offset = f32::from(u16::try_from(index).unwrap_or(u16::MAX)) * step;
        shards.push(Shard {
            at: Transform {
                pos: [at[0] + offset, at[1], at[2]],
                rot: Transform::IDENTITY.rot,
            },
            half,
            velocity: [0.0; 3],
        });
    }
    let event = BreakEvent {
        shards,
        impulse: vector(args.impulse.unwrap_or([0.0; 3]), "impulse")?,
    };
    with_slot(server, TOOL_FX_BREAK, |slot| {
        slot.apply(args.action_id, &args.body, body, FxAction::Break(event))
    })
}

fn blast(server: &McpServer, args: &FxBlastArgs) -> Result<FxOutput, McpToolError> {
    let body = body_of(&args.body)?;
    if args.radius_mm == 0 || args.radius_mm > 100_000 {
        return Err(bad(format!(
            "radius_mm is {}; pass 1..=100000 millimeters",
            args.radius_mm
        )));
    }
    if args.impulse_ns == 0 || args.impulse_ns > MAX_BLAST_NS {
        return Err(bad(format!(
            "impulse_ns is {}; pass 1..={MAX_BLAST_NS} newton-seconds",
            args.impulse_ns
        )));
    }
    if args
        .center_mm
        .iter()
        .any(|c| i64::from(*c).abs() > 1_000_000)
    {
        return Err(bad(
            "center_mm must hold three millimeters within -1000000..=1000000".to_owned(),
        ));
    }
    let event = BlastEvent {
        blast: Blast {
            center_mm: args.center_mm,
            radius_mm: args.radius_mm,
            impulse_ns: args.impulse_ns,
        },
    };
    with_slot(server, TOOL_FX_BLAST, |slot| {
        slot.apply(args.action_id, &args.body, body, FxAction::Blast(event))
    })
}

fn undo(server: &McpServer, args: &FxUndoArgs) -> Result<FxUndoOutput, McpToolError> {
    with_slot(server, TOOL_FX_UNDO, |slot| slot.undo(args.action_id))
}

crate::e2_tool!(
    /// `fx.ragdoll`: turn a body into a ragdoll.
    FxRagdollTool,
    name = TOOL_FX_RAGDOLL,
    description = "Turn a body into a ragdoll from an optional bone pose, velocity and impulse; answers the physfx receipt (body, joints, chunks, impulse) and one undo entry.",
    readonly = false,
    args = FxRagdollArgs,
    output = FxOutput,
    invoke = ragdoll
);

crate::e2_tool!(
    /// `fx.limb`: paint a severed limb.
    FxLimbTool,
    name = TOOL_FX_LIMB,
    description = "Paint a severed, cuttable limb of a body at a pose with a velocity; answers the physfx receipt and one undo entry.",
    readonly = false,
    args = FxLimbArgs,
    output = FxOutput,
    invoke = limb
);

crate::e2_tool!(
    /// `fx.break`: paint a debris break.
    FxBreakTool,
    name = TOOL_FX_BREAK,
    description = "Paint a debris break of 1..=64 boxes laid along +X from a pose; answers the physfx receipt and one undo entry.",
    readonly = false,
    args = FxBreakArgs,
    output = FxOutput,
    invoke = fracture
);

crate::e2_tool!(
    /// `fx.blast`: shove the bodies a blast reaches.
    FxBlastTool,
    name = TOOL_FX_BLAST,
    description = "Shove the float bodies a blast reaches from a center in millimeters; answers the physfx receipt with the pushed count and one undo entry.",
    readonly = false,
    args = FxBlastArgs,
    output = FxOutput,
    invoke = blast
);

crate::e2_tool!(
    /// `fx.undo`: drop the newest recorded effect.
    FxUndoTool,
    name = TOOL_FX_UNDO,
    description = "Undo the newest recorded fx effect by rebuilding the float world and replaying the older ones; answers the undone action id, the remaining depth and the body count. Refuses bad_args when nothing is pending or the named id is not newest.",
    readonly = false,
    args = FxUndoArgs,
    output = FxUndoOutput,
    invoke = undo
);

/// The fx pack: four effect verbs plus the undo that drops the newest.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "physical effects: a ragdoll, a severed limb, a debris break and a blast over a \
         session's float physfx world, one recorded action and one undo entry each, and one \
         undo verb that drops the newest",
        false,
    );
    pack.add(VerbSpec::of::<FxRagdollTool>(
        TOOL_FX_RAGDOLL,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<FxLimbTool>(
        TOOL_FX_LIMB,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<FxBreakTool>(
        TOOL_FX_BREAK,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<FxBlastTool>(
        TOOL_FX_BLAST,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<FxUndoTool>(
        TOOL_FX_UNDO,
        PACK,
        CapabilityCard::Commit,
    )?);
    Ok(pack)
}

#[cfg(test)]
mod fx_verbs {
    use super::*;
    use crate::packs::PackRegistry;
    use engine_editor::capability::Capability;
    use engine_physfx::Key;
    use rmcp::handler::server::router::tool::SyncTool;

    fn server(session: u64) -> McpServer {
        McpServer::new(session, Capability::unrestricted(session))
    }

    fn ragdoll_args(action_id: u64) -> FxRagdollArgs {
        FxRagdollArgs {
            action_id,
            body: "fighter".to_owned(),
            slot: 0,
            pose: None,
            velocity: Some([0.0, 0.0, 0.0]),
            impulse: Some([0.0, 0.0, -5.0]),
        }
    }

    fn limb_args(action_id: u64) -> FxLimbArgs {
        FxLimbArgs {
            action_id,
            body: "fighter".to_owned(),
            slot: 0,
            bone: "arm_lower_l".to_owned(),
            part: 100,
            at: Some([1.0, 1.0, 0.0]),
            velocity: Some([0.0, 0.0, 0.0]),
        }
    }

    fn break_args(action_id: u64) -> FxBreakArgs {
        FxBreakArgs {
            action_id,
            body: "fighter".to_owned(),
            chunks: 3,
            at: Some([0.0, 1.0, 0.0]),
            half: Some([0.1, 0.1, 0.1]),
            impulse: Some([1.0, 2.0, 3.0]),
        }
    }

    fn blast_args(action_id: u64) -> FxBlastArgs {
        FxBlastArgs {
            action_id,
            body: "fighter".to_owned(),
            center_mm: [0, 0, 0],
            radius_mm: 2_000,
            impulse_ns: 20,
        }
    }

    fn body_set_of(session: u64) -> Vec<Key> {
        let slots = slots().lock().expect("slots");
        slots
            .get(&session)
            .map(|slot| slot.body_set().expect("transforms"))
            .unwrap_or_default()
    }
    fn set_capture(session: u64, capture: Option<fn(&PhysFx) -> Option<String>>) {
        let mut slots = slots().lock().expect("slots");
        let slot = slots
            .entry(session)
            .or_insert_with(|| FxSlot::new().expect("float world"));
        slot.capture = capture;
    }

    fn frame(_: &PhysFx) -> Option<String> {
        Some("fixtures/fx/ragdoll.ppm".to_owned())
    }

    #[test]
    fn pack_registers_five_annotated_verbs() {
        let pack = pack().expect("registers");
        assert_eq!(
            pack.verb_names(),
            vec![
                TOOL_FX_RAGDOLL,
                TOOL_FX_LIMB,
                TOOL_FX_BREAK,
                TOOL_FX_BLAST,
                TOOL_FX_UNDO
            ]
        );
        for verb in pack.verbs() {
            assert_eq!(verb.card(), CapabilityCard::Commit);
            assert!(
                verb.model().annotations.is_some(),
                "{} is annotated",
                verb.name()
            );
        }
        assert!(PackRegistry::shipped().errors().is_empty());
        assert!(crate::mcp::tool_names().contains(&TOOL_FX_RAGDOLL));
    }

    #[test]
    fn each_verb_answers_its_four_anchors_and_receipt() {
        let ragdoll = FxRagdollTool::invoke(&server(71), ragdoll_args(1)).expect("ragdoll");
        assert_eq!(
            (ragdoll.body.as_str(), ragdoll.joints, ragdoll.chunks),
            ("fighter", 15, 0)
        );
        assert_eq!(ragdoll.impulse, [0.0, 0.0, -5.0]);
        assert_eq!(ragdoll.pushed, 0);
        assert!(
            ragdoll
                .receipt
                .starts_with("physfx: body=fighter joints=15 chunks=0 impulse="),
            "{}",
            ragdoll.receipt
        );
        assert!(ragdoll.capture.is_none(), "no capture hook is wired");

        let limb = FxLimbTool::invoke(&server(72), limb_args(1)).expect("limb");
        assert_eq!((limb.joints, limb.chunks), (15, 1));
        assert_eq!(limb.impulse, [0.0; 3]);
        assert!(
            limb.receipt
                .starts_with("physfx: body=fighter joints=15 chunks=1 impulse="),
            "{}",
            limb.receipt
        );

        let fracture = FxBreakTool::invoke(&server(73), break_args(1)).expect("break");
        assert_eq!((fracture.joints, fracture.chunks), (15, 3));
        assert_eq!(fracture.impulse, [1.0, 2.0, 3.0]);
        assert!(
            fracture
                .receipt
                .starts_with("physfx: body=fighter joints=15 chunks=3 impulse="),
            "{}",
            fracture.receipt
        );

        let blast_server = server(74);
        FxRagdollTool::invoke(&blast_server, ragdoll_args(1)).expect("ragdoll");
        let blast = FxBlastTool::invoke(&blast_server, blast_args(2)).expect("blast");
        assert_eq!((blast.joints, blast.chunks), (15, 0));
        assert_eq!(blast.impulse, [20.0, 0.0, 0.0]);
        assert_eq!(blast.pushed, 15, "the blast shoves every bone");
        assert!(
            blast.receipt.starts_with(
                "physfx: body=fighter joints=15 chunks=0 impulse=20.000,0.000,0.000 pushed=15"
            ),
            "{}",
            blast.receipt
        );
    }

    #[test]
    fn the_same_action_id_twice_writes_one_undo_entry() {
        let server = server(75);
        let first = FxLimbTool::invoke(&server, limb_args(9)).expect("limb");
        let second = FxLimbTool::invoke(&server, limb_args(9)).expect("replay");
        assert_eq!(first, second, "the stored receipt replays exactly");
        assert_eq!(first.undo_depth, 1);
        assert_eq!(body_set_of(75).len(), 1, "one limb, not two");
        let refused = FxLimbTool::invoke(
            &server,
            FxLimbArgs {
                bone: "palm_l".to_owned(),
                ..limb_args(9)
            },
        )
        .expect_err("the id belongs to a different effect");
        assert_eq!(refused.code, "bad_args");
        assert!(
            refused.message.contains("action id 9"),
            "{}",
            refused.message
        );
    }

    #[test]
    fn undo_restores_the_body_set() {
        let server = server(76);
        let empty = body_set_of(76);
        FxRagdollTool::invoke(&server, ragdoll_args(1)).expect("ragdoll");
        let bones = body_set_of(76);
        assert_eq!(bones.len(), 15);
        FxLimbTool::invoke(&server, limb_args(2)).expect("limb");
        assert_eq!(body_set_of(76).len(), 16);
        let stale = FxUndoTool::invoke(&server, FxUndoArgs { action_id: Some(1) })
            .expect_err("action 1 is not the newest");
        assert_eq!(stale.code, "bad_args");
        assert!(stale.message.contains("newest"), "{}", stale.message);
        let limb_undone =
            FxUndoTool::invoke(&server, FxUndoArgs { action_id: None }).expect("undo the limb");
        assert_eq!(
            (
                limb_undone.undone,
                limb_undone.undo_depth,
                limb_undone.bodies
            ),
            (2, 1, 15)
        );
        assert_eq!(body_set_of(76), bones, "the limb is gone");
        let all_undone = FxUndoTool::invoke(&server, FxUndoArgs { action_id: Some(1) })
            .expect("undo the ragdoll");
        assert_eq!(
            (all_undone.undone, all_undone.undo_depth, all_undone.bodies),
            (1, 0, 0)
        );
        assert_eq!(body_set_of(76), empty, "the original body set is back");
        let nothing =
            FxUndoTool::invoke(&server, FxUndoArgs { action_id: None }).expect_err("nothing left");
        assert_eq!(nothing.code, "bad_args");
        assert!(
            nothing.message.contains("nothing to undo"),
            "{}",
            nothing.message
        );
    }

    #[test]
    fn a_missing_body_is_bad_args_and_names_it() {
        let refused = FxRagdollTool::invoke(
            &server(77),
            FxRagdollArgs {
                body: "ghost".to_owned(),
                ..ragdoll_args(1)
            },
        )
        .expect_err("no such body");
        assert_eq!(refused.code, "bad_args");
        assert!(refused.message.contains("ghost"), "{}", refused.message);
    }

    #[test]
    fn the_capture_leg_answers_its_path() {
        let server = server(78);
        set_capture(78, Some(frame));
        let painted = FxRagdollTool::invoke(&server, ragdoll_args(1)).expect("ragdoll");
        assert_eq!(painted.capture.as_deref(), Some("fixtures/fx/ragdoll.ppm"));
        set_capture(78, None);
    }

    #[test]
    fn the_default_tool_list_stays_under_budget() {
        let server = server(79);
        assert!(
            server.tool_list_budget_chars() < crate::mcp::TOOL_LIST_BUDGET_CHARS,
            "the default list is {} chars",
            server.tool_list_budget_chars()
        );
    }
}
