//! `physics` pack: what one blow does to a fighter's balance, and what a ray or a
//! swept sphere meets in cooked collision.
//!
//! Three read-only verbs. `physics.raycast` and `physics.shapecast` cast a ray or a
//! swept sphere against cooked collider bytes (hex of the `E2CC` blob the asset cook
//! writes) and answer the nearest hit: collider id, distance, point and normal. A cast
//! must name its layer scope (`fighter`, `prop`, `debris`, `sensor`, `world`); a call
//! that names none is refused with the typed code `unscoped`, never answered against
//! everything. The geometry and the casts live in `engine-physics`
//! (`collider`, `layers`, `query`); this pack only parses, scopes and reports.
//!
//! `physics.read`: for a stagger line and a blow strength
//! (both Q8 impulse units, 256 reads unity) it answers the verdict the blow
//! warrants (`Steady`, `Staggered`, `Stumbling` or `Fallen`), the ticks the
//! stance then needs to go legal again, and the five standard rows of the
//! shove-response matrix for that tuning. The numbers themselves live in
//! `crates/physics/src/balance.rs`; this pack only reads its owner API
//! ([`engine_physics::balance::response_matrix`], `recovery_ticks` and
//! [`engine_physics::balance::BalanceState`]), so one tuning has one answer
//! in sim, harness and door.
//!
//! Caller: [`pack`] is registered by `crate::packs::PackRegistry::shipped`,
//! so the MCP door lists and dispatches `physics.read` for an AI that tunes
//! an encounter instead of guessing a blow strength.
//!
//! The verb takes `CapabilityCard::Observe`: it is pure integer math, reads
//! no bout and writes nothing, and allocates only the five response rows.
//! The door's live bout runs without the balance rules, so the live facts
//! (per-slot balance, bone velocities, bone contacts, joint drift) answer
//! `not_wired`, never a zero that would read as a measurement. Errors:
//! `bad_args` with the legal range named. Determinism: the same request gives
//! the same bytes on every machine.
//!
//! DTO structs carry no doc comments on purpose: every doc line inflates the
//! discovery schemas the door budgets.

use super::bad_args as bad;
use super::{PackDef, PackError, VerbSpec};
use crate::mcp::{McpServer, McpToolError};
use crate::tool_capabilities::CapabilityCard;
use engine_physics::balance::{
    recovery_ticks, response_matrix, BalanceParams, BalanceState, GaitVerdict, DEFAULT_STAGGER_Q8,
    MAX_IMPULSE_Q8,
};
use engine_physics::collider::{self, Collider};
use engine_physics::layers::{self, LayerError};
use engine_physics::query::{self, CastError, Hit};
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

/// Pack name.
pub const PACK: &str = "physics";
/// Verb: what one blow does to a fighter's balance.
pub const TOOL_PHYSICS_READ: &str = "physics.read";
/// Verb: the nearest thing a ray meets in cooked collision.
pub const TOOL_PHYSICS_RAYCAST: &str = "physics.raycast";
/// Verb: the nearest thing a swept sphere meets in cooked collision.
pub const TOOL_PHYSICS_SHAPECAST: &str = "physics.shapecast";
/// Typed refusal code of a cast that names no layer scope.
pub const UNSCOPED: &str = "unscoped";
/// Longest accepted collider blob in hex chars (64 KiB of cooked bytes).
pub const MAX_COLLIDER_HEX_CHARS: usize = 131_072;

/// Word every fact answers that only the live bout could know. The door's
/// bout carries no balance rules (`balance` is off in the arena rules), so a
/// live reading is not merely zero: it is unwired, and this word says so.
pub const NOT_WIRED: &str = "not_wired";

/// Stagger line and blow strength; both default to the shipped tuning, so
/// the default request reads the blow that lands exactly on the line.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde", default)]
#[schemars(crate = "rmcp::schemars")]
pub struct PhysicsReadArgs {
    pub stagger_line_q8: u16,
    pub blow_strength_q8: u16,
}

impl Default for PhysicsReadArgs {
    fn default() -> Self {
        Self {
            stagger_line_q8: DEFAULT_STAGGER_Q8,
            blow_strength_q8: DEFAULT_STAGGER_Q8,
        }
    }
}

/// One row of the shove-response matrix: a standard strength, what the blow
/// warrants there and the ticks that stance needs to recover.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PhysicsRowOut {
    pub strength_q8: u16,
    pub verdict: String,
    pub recovery_ticks: u32,
}

/// The live-bout facts this verb cannot answer yet: each one carries
/// [`NOT_WIRED`] instead of a zero.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PhysicsLiveBoutOut {
    pub per_slot_balance: String,
    pub bone_velocities: String,
    pub bone_contacts: String,
    pub joint_drift: String,
}

impl PhysicsLiveBoutOut {
    /// Every live fact unwired: one `not_wired` word per fact, never a zero.
    fn not_wired() -> Self {
        let word = || NOT_WIRED.to_owned();
        Self {
            per_slot_balance: word(),
            bone_velocities: word(),
            bone_contacts: word(),
            joint_drift: word(),
        }
    }
}

/// The blow's verdict, its recovery ticks, the two lines above the stagger
/// line and the five matrix rows for the same tuning.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PhysicsReadOutput {
    pub stagger_line_q8: u16,
    pub blow_strength_q8: u16,
    pub stumble_q8: u16,
    pub fall_q8: u16,
    pub verdict: String,
    pub recovery_ticks: u32,
    pub matrix: Vec<PhysicsRowOut>,
    pub live_bout: PhysicsLiveBoutOut,
}

/// Verdict word for one impact verdict: the four words an encounter tuner
/// switches on. Pure; the same verdict reads the same word twice.
#[must_use]
pub fn verdict_word(verdict: GaitVerdict) -> &'static str {
    match verdict {
        GaitVerdict::Steady => "Steady",
        GaitVerdict::Staggered => "Staggered",
        GaitVerdict::Stumbling => "Stumbling",
        GaitVerdict::Fallen => "Fallen",
    }
}

/// `physics.read` handler: validate the two Q8 numbers, then ask the balance
/// owner what this blow warrants under this line. Pure: one local
/// [`BalanceState`] carries the question, no bout is read and nothing is
/// written; the only allocation is the five-row matrix.
fn physics_read(
    _server: &McpServer,
    args: &PhysicsReadArgs,
) -> Result<PhysicsReadOutput, McpToolError> {
    let line = args.stagger_line_q8;
    if line == 0 || line > MAX_IMPULSE_Q8 {
        return Err(bad(format!(
            "stagger_line_q8 {line} must be 1..={MAX_IMPULSE_Q8} (Q8 impulse units, \
             256 reads unity)"
        )));
    }
    let blow = args.blow_strength_q8;
    if blow > MAX_IMPULSE_Q8 {
        return Err(bad(format!(
            "blow_strength_q8 {blow} must be 0..={MAX_IMPULSE_Q8} (Q8 impulse units, \
             256 reads unity)"
        )));
    }
    let params = BalanceParams::new(line).map_err(|error| {
        bad(format!(
            "{error}; re-run with stagger_line_q8 1..={MAX_IMPULSE_Q8}"
        ))
    })?;
    let mut state = BalanceState::new(params);
    let verdict = state.apply_impact(blow).map_err(|error| {
        bad(format!(
            "{error}; re-run with blow_strength_q8 0..={MAX_IMPULSE_Q8}"
        ))
    })?;
    Ok(PhysicsReadOutput {
        stagger_line_q8: line,
        blow_strength_q8: blow,
        stumble_q8: state.stumble_q8(),
        fall_q8: state.fall_q8(),
        verdict: verdict_word(verdict).to_owned(),
        recovery_ticks: recovery_ticks(verdict),
        matrix: response_matrix(params)
            .iter()
            .map(|row| PhysicsRowOut {
                strength_q8: row.strength_q8,
                verdict: verdict_word(row.verdict).to_owned(),
                recovery_ticks: row.recovery_ticks,
            })
            .collect(),
        live_bout: PhysicsLiveBoutOut::not_wired(),
    })
}

crate::e2_tool!(
    /// `physics.read`: what one blow does to a fighter's balance.
    PhysicsReadTool,
    name = TOOL_PHYSICS_READ,
    description = "Read what one blow does to a fighter's balance (stagger_line_q8 1..=4096, blow_strength_q8 0..=4096): the verdict word, its recovery ticks and the five standard shove-response rows for that tuning. Pure integer, reads no bout; live-bout facts answer not_wired. Read-only.",
    readonly = true,
    args = PhysicsReadArgs,
    output = PhysicsReadOutput,
    invoke = physics_read
);

/// Request shared by both casts: the cooked world, the cast line and the layer scope.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde", default)]
#[schemars(crate = "rmcp::schemars")]
pub struct PhysicsRaycastArgs {
    pub colliders_hex: String,
    pub origin: [f32; 3],
    pub direction: [f32; 3],
    pub max_distance_m: f32,
    pub layers: Vec<String>,
}

/// A raycast plus the sphere radius.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde", default)]
#[schemars(crate = "rmcp::schemars")]
pub struct PhysicsShapecastArgs {
    pub colliders_hex: String,
    pub origin: [f32; 3],
    pub direction: [f32; 3],
    pub radius_m: f32,
    pub max_distance_m: f32,
    pub layers: Vec<String>,
}

/// What a cast met first.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PhysicsHitOut {
    pub collider: String,
    pub distance_m: f32,
    pub point: [f32; 3],
    pub normal: [f32; 3],
}

/// The nearest hit (absent on a clean miss) plus how many colliders the blob held.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PhysicsCastOutput {
    pub hit: Option<PhysicsHitOut>,
    pub colliders: u32,
    pub scope: Vec<String>,
}

/// Lowercase or uppercase hex into bytes; `None` for odd length or a non-hex char.
fn bytes_of_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).ok())
        .collect()
}

/// Decode the request's collider blob into shapes, or the typed refusal.
fn world_of(colliders_hex: &str) -> Result<Vec<Collider>, McpToolError> {
    if colliders_hex.len() > MAX_COLLIDER_HEX_CHARS {
        return Err(bad(format!(
            "colliders_hex holds {} chars; at most {MAX_COLLIDER_HEX_CHARS} (64 KiB of cooked \
             bytes); cast against fewer colliders",
            colliders_hex.len()
        )));
    }
    let bytes = bytes_of_hex(colliders_hex).ok_or_else(|| {
        McpToolError::new(
            "bad_colliders",
            "colliders_hex is not hex; pass the cooked E2CC collider bytes as an even-length \
             hex string",
        )
    })?;
    collider::decode(&bytes).map_err(|error| {
        McpToolError::new(
            "bad_colliders",
            format!("{error}; pass the bytes the asset collider cook wrote, unchanged"),
        )
    })
}

/// The layer mask a request's words name. Naming none is `unscoped`: a cast never
/// falls back to "every layer".
fn scope_of(words: &[String]) -> Result<u32, McpToolError> {
    layers::scope(words).map_err(|error| match error {
        LayerError::Empty => McpToolError::new(
            UNSCOPED,
            format!(
                "layers is empty; a cast must name the layers it may hit ({}); re-run with at \
                 least one",
                layers::names().collect::<Vec<_>>().join(", ")
            ),
        ),
        unknown @ LayerError::Unknown(_) => bad(format!("{unknown}; re-run with known layers")),
    })
}

fn cast_error(error: CastError) -> McpToolError {
    match error {
        CastError::Unscoped => McpToolError::new(UNSCOPED, "the layer scope names no layer"),
        other => bad(format!(
            "{other}; re-run with a finite origin, a non-zero direction, and distances above zero"
        )),
    }
}

fn report(world: &[Collider], scope: &[String], hit: Option<Hit>) -> PhysicsCastOutput {
    PhysicsCastOutput {
        hit: hit.and_then(|hit| {
            Some(PhysicsHitOut {
                collider: world.get(hit.collider)?.id.clone(),
                distance_m: hit.distance,
                point: hit.point,
                normal: hit.normal,
            })
        }),
        colliders: u32::try_from(world.len()).unwrap_or(u32::MAX),
        scope: scope.to_vec(),
    }
}

/// `physics.raycast` handler: decode the blob, resolve the scope, cast once.
/// Pure: reads only its arguments and writes nothing.
fn physics_raycast(
    _server: &McpServer,
    args: &PhysicsRaycastArgs,
) -> Result<PhysicsCastOutput, McpToolError> {
    let scope = scope_of(&args.layers)?;
    let world = world_of(&args.colliders_hex)?;
    let hit = query::ray_cast(
        &world,
        args.origin,
        args.direction,
        args.max_distance_m,
        scope,
    )
    .map_err(cast_error)?;
    Ok(report(&world, &args.layers, hit))
}

/// `physics.shapecast` handler: as the raycast, with a sphere of `radius_m`.
fn physics_shapecast(
    _server: &McpServer,
    args: &PhysicsShapecastArgs,
) -> Result<PhysicsCastOutput, McpToolError> {
    let scope = scope_of(&args.layers)?;
    let world = world_of(&args.colliders_hex)?;
    let hit = query::sphere_cast(
        &world,
        args.origin,
        args.direction,
        args.radius_m,
        args.max_distance_m,
        scope,
    )
    .map_err(cast_error)?;
    Ok(report(&world, &args.layers, hit))
}

crate::e2_tool!(
    /// `physics.raycast`: the nearest hit of a ray in cooked collision.
    PhysicsRaycastTool,
    name = TOOL_PHYSICS_RAYCAST,
    description = "Cast a ray (origin, direction, max_distance_m) against cooked collider bytes (colliders_hex, the E2CC blob as hex) and answer the nearest hit: collider id, distance_m, point, normal. layers is required (fighter, prop, debris, sensor, world); naming none is refused as unscoped. Read-only.",
    readonly = true,
    args = PhysicsRaycastArgs,
    output = PhysicsCastOutput,
    invoke = physics_raycast
);

crate::e2_tool!(
    /// `physics.shapecast`: the nearest hit of a swept sphere in cooked collision.
    PhysicsShapecastTool,
    name = TOOL_PHYSICS_SHAPECAST,
    description = "Sweep a sphere (radius_m) from origin along direction for max_distance_m against cooked collider bytes (colliders_hex) and answer the nearest hit: collider id, distance_m the centre travels, contact point, normal. layers is required; naming none is refused as unscoped. Read-only.",
    readonly = true,
    args = PhysicsShapecastArgs,
    output = PhysicsCastOutput,
    invoke = physics_shapecast
);

/// The physics pack: three read-only verbs, loaded on demand.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "balance: what one blow does to a fighter's stance (verdict word, recovery ticks, \
         the five-strength shove-response matrix), pure and integer over the physics balance \
         core, with live-bout facts unwired; casts: nearest hit of a ray or a swept sphere \
         in cooked collider bytes, scoped by layer",
        false,
    );
    pack.add(VerbSpec::of::<PhysicsReadTool>(
        TOOL_PHYSICS_READ,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<PhysicsRaycastTool>(
        TOOL_PHYSICS_RAYCAST,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<PhysicsShapecastTool>(
        TOOL_PHYSICS_SHAPECAST,
        PACK,
        CapabilityCard::Observe,
    )?);
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_editor::capability::Capability;
    use engine_physics::balance::{
        CATCH_TICKS, FALL_ABOVE_STAGGER_Q8, FALL_PRONE_TICKS, STAGGER_TICKS,
    };
    use rmcp::handler::server::router::tool::SyncTool;

    fn server() -> McpServer {
        McpServer::new(7, Capability::unrestricted(7))
    }

    #[test]
    fn physics_read_orders_blows_over_the_line_and_the_fall_line() {
        let server = server();
        let under = PhysicsReadTool::invoke(
            &server,
            PhysicsReadArgs {
                stagger_line_q8: DEFAULT_STAGGER_Q8,
                blow_strength_q8: DEFAULT_STAGGER_Q8 - 1,
            },
        )
        .expect("reads");
        assert_eq!(under.verdict, "Steady");
        assert_eq!(under.recovery_ticks, 0, "a steady stance owes no recovery");
        let at = PhysicsReadTool::invoke(&server, PhysicsReadArgs::default()).expect("reads");
        assert_eq!(at.blow_strength_q8, at.stagger_line_q8);
        assert_eq!(at.verdict, "Staggered");
        assert_eq!(at.recovery_ticks, STAGGER_TICKS);
        let over = PhysicsReadTool::invoke(
            &server,
            PhysicsReadArgs {
                stagger_line_q8: DEFAULT_STAGGER_Q8,
                blow_strength_q8: DEFAULT_STAGGER_Q8.saturating_add(FALL_ABOVE_STAGGER_Q8),
            },
        )
        .expect("reads");
        assert_eq!(over.verdict, "Fallen");
        assert_eq!(over.recovery_ticks, FALL_PRONE_TICKS + CATCH_TICKS);
        assert_eq!(
            over.fall_q8,
            DEFAULT_STAGGER_Q8.saturating_add(FALL_ABOVE_STAGGER_Q8)
        );
        // The same request twice gives the same bytes.
        let again = PhysicsReadTool::invoke(&server, PhysicsReadArgs::default()).expect("reads");
        assert_eq!(at, again);
    }

    #[test]
    fn physics_read_matrix_equals_the_balance_owner() {
        let server = server();
        let args = PhysicsReadArgs {
            stagger_line_q8: 300,
            blow_strength_q8: 256,
        };
        let out = PhysicsReadTool::invoke(&server, args.clone()).expect("reads");
        let want = response_matrix(BalanceParams::new(args.stagger_line_q8).expect("legal line"));
        assert_eq!(
            out.matrix.len(),
            want.len(),
            "one row per standard strength"
        );
        for (row, expected) in out.matrix.iter().zip(want.iter()) {
            assert_eq!(row.strength_q8, expected.strength_q8);
            assert_eq!(row.verdict, verdict_word(expected.verdict));
            assert_eq!(row.recovery_ticks, recovery_ticks(expected.verdict));
        }
        // The default tuning falls on its last row; a lower line falls sooner.
        let default_rows =
            PhysicsReadTool::invoke(&server, PhysicsReadArgs::default()).expect("reads");
        assert_eq!(
            default_rows
                .matrix
                .iter()
                .map(|row| row.verdict.as_str())
                .collect::<Vec<_>>(),
            vec!["Steady", "Staggered", "Stumbling", "Stumbling", "Fallen"]
        );
        let lower = PhysicsReadTool::invoke(
            &server,
            PhysicsReadArgs {
                stagger_line_q8: 128,
                ..args
            },
        )
        .expect("reads");
        assert_ne!(out.matrix, lower.matrix, "the line moves the rows");
    }

    #[test]
    fn physics_read_refuses_a_zero_line_and_names_the_legal_range() {
        let server = server();
        let zero = PhysicsReadTool::invoke(
            &server,
            PhysicsReadArgs {
                stagger_line_q8: 0,
                blow_strength_q8: 256,
            },
        )
        .expect_err("a zero line refuses");
        assert_eq!(zero.code, "bad_args");
        assert!(zero.message.contains("stagger_line_q8"));
        assert!(
            zero.message.contains(&format!("1..={MAX_IMPULSE_Q8}")),
            "the refusal names the legal range: {}",
            zero.message
        );
        for args in [
            PhysicsReadArgs {
                stagger_line_q8: MAX_IMPULSE_Q8 + 1,
                blow_strength_q8: 256,
            },
            PhysicsReadArgs {
                stagger_line_q8: 256,
                blow_strength_q8: MAX_IMPULSE_Q8 + 1,
            },
        ] {
            let refused = PhysicsReadTool::invoke(&server, args).expect_err("over the ceiling");
            assert_eq!(refused.code, "bad_args");
            assert!(refused.message.contains(&MAX_IMPULSE_Q8.to_string()));
        }
    }

    #[test]
    fn physics_read_reports_live_bout_facts_as_not_wired() {
        let server = server();
        let out = PhysicsReadTool::invoke(&server, PhysicsReadArgs::default()).expect("reads");
        let live = &out.live_bout;
        for (name, fact) in [
            ("per_slot_balance", &live.per_slot_balance),
            ("bone_velocities", &live.bone_velocities),
            ("bone_contacts", &live.bone_contacts),
            ("joint_drift", &live.joint_drift),
        ] {
            assert_eq!(
                fact, NOT_WIRED,
                "{name} answers not_wired, never a zero reading"
            );
        }
        assert_ne!(NOT_WIRED, "0");
    }

    /// The cook's pinned bytes for one unit-cube hull `COL_box` on the `prop` layer, a
    /// checked-in fixture (`engine-assets` `collider_cook` asserts its writer makes exactly these).
    const CUBE_BYTES: &[u8] = include_bytes!("../../../../content/physics/unit_cube_hull.e2cc");

    fn cube_hex() -> String {
        CUBE_BYTES.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn ray_args(layers: &[&str]) -> PhysicsRaycastArgs {
        PhysicsRaycastArgs {
            colliders_hex: cube_hex(),
            origin: [-3.0, 0.5, 0.5],
            direction: [1.0, 0.0, 0.0],
            max_distance_m: 10.0,
            layers: layers.iter().map(|word| (*word).to_owned()).collect(),
        }
    }

    fn near(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1.0e-4)
    }

    #[test]
    fn physics_raycast_returns_the_hit_point_and_normal_for_a_scripted_ray() {
        let server = server();
        let out = PhysicsRaycastTool::invoke(&server, ray_args(&["prop", "world"])).expect("casts");
        let hit = out.hit.clone().expect("the ray meets the cube");
        assert_eq!(hit.collider, "COL_box");
        assert!((hit.distance_m - 3.0).abs() < 1.0e-5, "{}", hit.distance_m);
        assert!(near(hit.point, [0.0, 0.5, 0.5]), "{:?}", hit.point);
        assert!(near(hit.normal, [-1.0, 0.0, 0.0]), "{:?}", hit.normal);
        assert_eq!(
            (out.colliders, out.scope.as_slice()),
            (1, &["prop".to_owned(), "world".to_owned()][..])
        );
        // The same request twice gives the same bytes.
        let again =
            PhysicsRaycastTool::invoke(&server, ray_args(&["prop", "world"])).expect("casts");
        assert_eq!(
            rmcp::serde_json::to_string(&again).expect("serializes"),
            rmcp::serde_json::to_string(&out).expect("serializes")
        );
        // A ray that points away, or stops short, is a clean miss (no hit, not an error).
        let mut away = ray_args(&["prop"]);
        away.direction = [-1.0, 0.0, 0.0];
        assert_eq!(
            PhysicsRaycastTool::invoke(&server, away)
                .expect("casts")
                .hit,
            None
        );
        let mut short = ray_args(&["prop"]);
        short.max_distance_m = 2.0;
        assert_eq!(
            PhysicsRaycastTool::invoke(&server, short)
                .expect("casts")
                .hit,
            None
        );
    }

    #[test]
    fn physics_raycast_an_unscoped_call_is_refused_with_a_typed_reason() {
        let server = server();
        let unscoped =
            PhysicsRaycastTool::invoke(&server, ray_args(&[])).expect_err("no scope refuses");
        assert_eq!(unscoped.code, UNSCOPED);
        assert!(
            unscoped
                .message
                .contains("fighter, prop, debris, sensor, world"),
            "the refusal names the legal layers: {}",
            unscoped.message
        );
        // A request that omits `layers` entirely parses to the same refusal.
        let json = format!(
            "{{\"colliders_hex\":\"{}\",\"direction\":[1,0,0],\"max_distance_m\":5}}",
            cube_hex()
        );
        let omitted: PhysicsRaycastArgs =
            rmcp::serde_json::from_str(&json).expect("defaults fill the rest");
        assert_eq!(
            PhysicsRaycastTool::invoke(&server, omitted)
                .expect_err("refuses")
                .code,
            UNSCOPED
        );
        let shape = PhysicsShapecastTool::invoke(&server, PhysicsShapecastArgs::default())
            .expect_err("refuses");
        assert_eq!(
            shape.code, UNSCOPED,
            "the sphere cast is scoped the same way"
        );
        // A scope that excludes the cube's layer sees nothing there.
        let blind =
            PhysicsRaycastTool::invoke(&server, ray_args(&["world", "fighter"])).expect("casts");
        assert_eq!(blind.hit, None, "the cube is on the prop layer");
        // An unknown layer word is bad_args, naming the legal words.
        let ghost =
            PhysicsRaycastTool::invoke(&server, ray_args(&["ghost"])).expect_err("unknown layer");
        assert_eq!(ghost.code, "bad_args");
        assert!(ghost.message.contains("ghost") && ghost.message.contains("fighter"));
    }

    #[test]
    fn physics_raycast_refuses_bad_blobs_and_bad_lines() {
        let server = server();
        let with = |edit: fn(&mut PhysicsRaycastArgs)| {
            let mut args = ray_args(&["prop"]);
            edit(&mut args);
            PhysicsRaycastTool::invoke(&server, args).expect_err("refuses")
        };
        assert_eq!(
            with(|a| a.colliders_hex = "zz".to_owned()).code,
            "bad_colliders"
        );
        assert_eq!(
            with(|a| a.colliders_hex = "abc".to_owned()).code,
            "bad_colliders"
        );
        assert_eq!(
            with(|a| a.colliders_hex = "+f".to_owned()).code,
            "bad_colliders",
            "a sign is not hex"
        );
        let bad_magic = with(|a| a.colliders_hex = cube_hex().replacen("45", "46", 1));
        assert_eq!(bad_magic.code, "bad_colliders");
        assert!(bad_magic.message.contains("E2CC"), "{}", bad_magic.message);
        assert_eq!(
            with(|a| a.colliders_hex = cube_hex()[..40].to_owned()).code,
            "bad_colliders"
        );
        assert_eq!(
            with(|a| a.colliders_hex = "00".repeat(MAX_COLLIDER_HEX_CHARS)).code,
            "bad_args"
        );
        assert_eq!(with(|a| a.direction = [0.0; 3]).code, "bad_args");
        assert_eq!(with(|a| a.max_distance_m = 0.0).code, "bad_args");
        assert_eq!(with(|a| a.origin[0] = f32::NAN).code, "bad_args");
    }

    #[test]
    fn physics_shapecast_stops_a_radius_short_with_the_contact_point_and_normal() {
        let server = server();
        let args = PhysicsShapecastArgs {
            colliders_hex: cube_hex(),
            origin: [-3.0, 0.5, 0.5],
            direction: [1.0, 0.0, 0.0],
            radius_m: 0.5,
            max_distance_m: 10.0,
            layers: vec!["prop".to_owned()],
        };
        let out = PhysicsShapecastTool::invoke(&server, args.clone()).expect("casts");
        let hit = out.hit.expect("the sphere meets the cube");
        assert_eq!(hit.collider, "COL_box");
        assert!((hit.distance_m - 2.5).abs() < 1.0e-5, "{}", hit.distance_m);
        assert!(near(hit.point, [0.0, 0.5, 0.5]) && near(hit.normal, [-1.0, 0.0, 0.0]));
        // Clearing the cube by more than the radius is a miss; a negative radius refuses.
        let mut high = args.clone();
        high.origin[1] = 1.6;
        assert_eq!(
            PhysicsShapecastTool::invoke(&server, high)
                .expect("casts")
                .hit,
            None
        );
        let mut negative = args;
        negative.radius_m = -1.0;
        assert_eq!(
            PhysicsShapecastTool::invoke(&server, negative)
                .expect_err("refuses")
                .code,
            "bad_args"
        );
    }

    #[test]
    fn physics_pack_registers_three_observe_verbs() {
        let pack = pack().expect("pack builds");
        assert_eq!(pack.name(), PACK);
        assert_eq!(
            pack.verb_names(),
            vec![
                TOOL_PHYSICS_READ,
                TOOL_PHYSICS_RAYCAST,
                TOOL_PHYSICS_SHAPECAST
            ]
        );
        assert!(!pack.default_loaded(), "the pack loads on demand");
        assert!(
            !pack.summary().is_empty(),
            "the pack carries its one-line summary"
        );
        for verb in pack.verbs() {
            assert_eq!(verb.card(), CapabilityCard::Observe);
            assert!(verb.readonly());
        }
    }
}

// WG2: this pack publishes its own row; `packs/mod.rs` keeps no hand list.
#[expect(
    unsafe_code,
    reason = "linkme publishes a pack row through a link section"
)]
#[linkme::distributed_slice(super::PACK_MODULES)]
static PACK_ROW: super::PackModule = super::PackModule {
    order: 16,
    name: PACK,
    default_loaded: false,
    tools: &["physics.read", "physics.raycast", "physics.shapecast"],
    build: pack,
};
