//! Spine inspector: the arena spine's actors as read-only inspector nodes.
//!
//! The consumer is the inspector panel (`shell_ui.rs` draws the nodes held by
//! [`Shell::set_spine`](crate::shell::Shell::set_spine)). The data owner is
//! `engine-ecs`: this module reads one [`SpineSnapshot`] through the RC2
//! [`Bridge`] (pose rows in an [`ExtractionView`], never sim state) and
//! renders the other four columns (vitals, body, ai, anim) as components
//! whose names and versions the RC4 [`SchemaRegistry`] serves. Units: slots
//! (`u8`), meters for pose, integer millimeters for [`Fixed`] lanes,
//! centidegrees for facing. Determinism: nodes come out in slot order and the
//! same snapshot bytes always give the same nodes and payload text; no clock.
//! Errors are typed [`SpineInspectError`]; a refused snapshot changes nothing.
//! Spine state is sim state, so every row is a read-only view: edits belong to
//! the tape, never to this panel.

use std::fmt;

use engine_core::fixed::Fixed;
use engine_ecs::bridge::{Bridge, BridgeError, ExtractionView, StableId};
use engine_ecs::spine::{
    AiState, AnimState, Body, Spine, SpineError, SpineSnapshot, Vitals, MAX_SPINE_SLOTS,
};
use engine_scene::schema::{Schema, SchemaError, SchemaRegistry, VersionedComponent as Served};
use engine_scene::transform::Transform;

use crate::inspector::{NodeView, SchemaStep, VersionedComponent};

/// Spine component names, in grid order. Each is served at v1.
pub const COMPONENTS: [&str; 4] = ["vitals", "body", "ai", "anim"];

/// What refused a spine snapshot. Nothing is kept on refusal.
#[derive(Clone, Debug, PartialEq)]
pub enum SpineInspectError {
    /// The spine bytes did not decode.
    Spine(SpineError),
    /// The extraction bridge refused the snapshot (hash, tick, pose).
    Bridge(BridgeError),
    /// The registry does not serve a spine component at the version offered.
    Schema(SchemaError),
}

impl fmt::Display for SpineInspectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spine(error) => write!(f, "spine snapshot refused: {error}"),
            Self::Bridge(error) => write!(f, "spine extraction refused: {error}"),
            Self::Schema(error) => write!(f, "spine schema refused: {error}"),
        }
    }
}

impl std::error::Error for SpineInspectError {}

/// The RC4 registry for spine components: the shipped builtins plus the four
/// [`COMPONENTS`] at v1.
pub fn spine_registry() -> Result<SchemaRegistry, SchemaError> {
    let mut registry = SchemaRegistry::builtin();
    for component in COMPONENTS {
        registry.register(Schema {
            component,
            version: 1,
            migrate: None,
        })?;
    }
    Ok(registry)
}

/// One inspector node per live spine slot, in slot order.
///
/// Pose comes from the [`ExtractionView`] the bridge builds out of the
/// snapshot bytes; the four component payloads come from the same bytes and
/// pass through `registry` before the grid sees them, so an unserved
/// component or version refuses here instead of rendering. Returns the view
/// beside the nodes so the panel can show its tick and digest.
pub fn inspect_spine(
    tick: u32,
    snapshot: &SpineSnapshot,
    registry: &SchemaRegistry,
) -> Result<(ExtractionView, Vec<NodeView>), SpineInspectError> {
    let spine = Spine::restore(&snapshot.bytes).map_err(SpineInspectError::Spine)?;
    let mut bridge = Bridge::new();
    bridge
        .ingest_spine(tick, snapshot)
        .map_err(SpineInspectError::Bridge)?;
    let view = bridge.view();
    let mut nodes = Vec::with_capacity(spine.live());
    for slot in 0..spine.len().min(MAX_SPINE_SLOTS) {
        let Some(id) = u8::try_from(slot)
            .ok()
            .map(StableId::arena)
            .filter(|_| spine.is_alive(slot))
        else {
            continue;
        };
        let Some(row) = view.get(id) else { continue };
        let components = [
            ("vitals", vitals_text(&spine.vitals()[slot])),
            ("body", body_text(&spine.bodies()[slot])),
            ("ai", ai_text(&spine.ai()[slot])),
            ("anim", anim_text(&spine.anims()[slot])),
        ];
        let mut node = NodeView::new(id.raw());
        node.key = Some(format!("fighter {slot}"));
        node.pose = Transform::new(row.pos, quat_of_yaw(row.yaw), [1.0, 1.0, 1.0]);
        for (name, payload) in components {
            let served = registry
                .migrate_value(&Served {
                    component: name.to_owned(),
                    version: 1,
                    payload: payload.clone(),
                })
                .map_err(SpineInspectError::Schema)?;
            node.components.push(VersionedComponent::new(
                name,
                served.version,
                &served.payload,
            ));
            node.schema.push(SchemaStep {
                component: name,
                version: 1,
                is_current: registry.current_version(name) == Some(1),
            });
        }
        nodes.push(node);
    }
    Ok((view, nodes))
}

/// Pure-Y rotation quaternion `[x, y, z, w]` for a facing yaw in radians.
fn quat_of_yaw(yaw: f32) -> [f32; 4] {
    let half = 0.5 * yaw;
    [0.0, half.sin(), 0.0, half.cos()]
}

/// A [`Fixed`] meter lane as whole millimeters, rounded half away from zero
/// (integer math only).
fn mm(value: Fixed) -> i64 {
    let bits = value.to_bits();
    (bits.saturating_mul(1000) + bits.signum() * (Fixed::SCALE / 2)) / Fixed::SCALE
}

fn vitals_text(vitals: &Vitals) -> String {
    const BITS: [(u8, &str); 5] = [
        (Vitals::ATTACKING, "attacking"),
        (Vitals::BLOCKING, "blocking"),
        (Vitals::DODGING, "dodging"),
        (Vitals::DOWN, "down"),
        (Vitals::OUT, "out"),
    ];
    let mut state: Vec<&str> = BITS
        .iter()
        .filter(|(bit, _)| vitals.state & bit != 0)
        .map(|(_, name)| *name)
        .collect();
    if state.is_empty() {
        state.push("idle");
    }
    format!(
        "hp={} stamina={} poise={} state={}",
        vitals.hp_permille,
        vitals.stamina,
        vitals.poise,
        state.join("+")
    )
}

fn body_text(body: &Body) -> String {
    format!(
        "vel_x_mm={} vel_z_mm={} radius_mm={} height_mm={}",
        mm(body.vel[0]),
        mm(body.vel[1]),
        mm(body.radius),
        mm(body.height)
    )
}

fn ai_text(ai: &AiState) -> String {
    let source = match ai.source {
        AiState::FROM_TAPE => "tape",
        AiState::FROM_BRAIN => "brain",
        _ => "none",
    };
    format!(
        "mx={} mz={} facing_cdeg={} buttons={} clock={} source={source}",
        ai.intent.mx, ai.intent.mz, ai.intent.facing, ai.intent.buttons, ai.clock
    )
}

fn anim_text(anim: &AnimState) -> String {
    format!(
        "loco={} phase={:.3} facing_x={:.3} facing_z={:.3} airborne={} interrupt={} pose_ref={}",
        anim.loco,
        anim.phase,
        anim.facing[0],
        anim.facing[1],
        anim.airborne,
        anim.interrupt,
        anim.pose_ref
    )
}

/// Fixture shared with the inspector panel smoke in `shell_ui.rs`.
#[cfg(test)]
pub(crate) mod fixture {
    use super::*;
    use engine_ecs::spine::{Intent, Transform as SpinePose};

    /// Two fighters: slot 0 mid-attack on the tape, slot 1 down and driven by
    /// the brain; a third slot is spawned then killed so it must not appear.
    pub(crate) fn fight() -> SpineSnapshot {
        let mut spine = Spine::with_capacity(3);
        for (x, hp, state, source) in [
            (
                -2.0,
                800u16,
                Vitals::ATTACKING | Vitals::BLOCKING,
                AiState::FROM_TAPE,
            ),
            (2.5, 0, Vitals::DOWN | Vitals::OUT, AiState::FROM_BRAIN),
            (9.0, 1000, 0, AiState::FROM_NONE),
        ] {
            let slot = spine
                .spawn(
                    SpinePose {
                        pos: [x, 0.0, 1.0],
                        yaw: 1.5,
                    },
                    Body {
                        vel: [Fixed::HALF, Fixed::ZERO],
                        radius: Fixed(26_214),
                        height: Fixed(117_965),
                    },
                    Vitals {
                        hp_permille: hp,
                        stamina: 90,
                        poise: 0,
                        state,
                    },
                )
                .expect("slot is free");
            let column = spine.slot_mut(slot).expect("slot exists");
            *column.ai = AiState {
                intent: Intent {
                    mx: 40,
                    mz: -40,
                    facing: 9000,
                    buttons: 1,
                },
                clock: 7,
                source,
            };
        }
        assert!(spine.kill(2));
        spine.snapshot()
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::fight;
    use super::*;

    fn text<'a>(node: &'a NodeView, component: &str) -> &'a str {
        &node
            .components
            .iter()
            .find(|c| c.component == component)
            .expect("component renders")
            .payload
    }

    #[test]
    fn live_slots_render_four_components_through_the_registry() {
        let registry = spine_registry().expect("spine components register");
        let (view, nodes) = inspect_spine(41, &fight(), &registry).expect("spine inspects");
        assert_eq!(
            (view.tick, nodes.len()),
            (41, 2),
            "the killed slot is absent"
        );
        let first = &nodes[0];
        assert_eq!(first.key.as_deref(), Some("fighter 0"));
        assert_eq!(first.id, StableId::arena(0).raw());
        assert_eq!(first.pose.position, [-2.0, 0.0, 1.0]);
        assert_eq!(
            text(first, "vitals"),
            "hp=800 stamina=90 poise=0 state=attacking+blocking"
        );
        assert_eq!(
            text(&nodes[1], "vitals"),
            "hp=0 stamina=90 poise=0 state=down+out"
        );
        assert_eq!(
            text(first, "body"),
            "vel_x_mm=500 vel_z_mm=0 radius_mm=400 height_mm=1800"
        );
        assert_eq!(
            text(first, "ai"),
            "mx=40 mz=-40 facing_cdeg=9000 buttons=1 clock=7 source=tape"
        );
        assert!(text(&nodes[1], "ai").ends_with("source=brain"));
        assert!(text(first, "anim").starts_with("loco=0 phase=0.000"));
        assert!(first.schema.iter().all(|step| step.is_current));
        assert_eq!(first.components.len(), COMPONENTS.len());
    }

    #[test]
    fn same_bytes_same_nodes_and_bad_bytes_refuse() {
        let registry = spine_registry().expect("registers");
        let one = inspect_spine(41, &fight(), &registry).expect("first");
        let two = inspect_spine(41, &fight(), &registry).expect("second");
        assert_eq!(one.1, two.1);
        assert_eq!(one.0.digest(), two.0.digest());

        let mut forged = fight();
        forged.hash ^= 1;
        assert_eq!(
            inspect_spine(41, &forged, &registry).err(),
            Some(SpineInspectError::Bridge(BridgeError::BadHash {
                domain: "spine"
            }))
        );
        let mut cut = fight();
        cut.bytes.truncate(3);
        assert_eq!(
            inspect_spine(41, &cut, &registry).err(),
            Some(SpineInspectError::Spine(SpineError::TooShort))
        );
        // A registry that does not serve the spine components refuses loudly.
        let bare = SchemaRegistry::builtin();
        let refused = inspect_spine(41, &fight(), &bare).expect_err("unserved component");
        assert!(matches!(
            refused,
            SpineInspectError::Schema(SchemaError::UnknownVersion { .. })
        ));
    }
}
