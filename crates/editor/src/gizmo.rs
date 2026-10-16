//! Transform gizmo: drag deltas staged as command batches.
//!
//! The consumer is the viewport panel (`shell_ui`, behind the `ui` feature)
//! and the [`Shell`](crate::shell::Shell) gizmo entrypoints. The data owner
//! for applied edits stays the command service: every gizmo drag builds a
//! [`Batch`](crate::commands::Batch) that is validated (`preview`) before it
//! can apply, so an invalid drag reports an error and changes nothing.
//! Spatial recheck of a staged batch is the caller's job.
//!
//! Two drag shapes funnel into batches:
//!
//! * [`GizmoDrag::Translate`] / [`GizmoDrag::Rotate`]: headless-friendly
//!   deltas in the selected node's *parent* frame, applied to its local pose
//!   by [`drag_to_local`]. Parent-frame math matches `engine-scene`
//!   composition (`world = parent * local`), so a delta premultiplies the
//!   local pose exactly.
//! * Absolute world poses from the real `transform-gizmo` session (see the
//!   [`ui`] bridge): converted back to the parent frame by
//!   [`world_to_local`], which inverts the parent world pose and rejects
//!   degenerate scales, mirroring the scene's invertibility rule.
//!
//! The windowed present loop stays with the window host: the [`ui`] bridge only
//! models the gizmo session (camera sync, drag, convert); it opens no window
//! and presents no frames.
//! PORT(urholaukkarinen/transform-gizmo@07a951ec17ebff2de4608b711ee0544ff2c8063c:
//! config modes + gizmo session + egui frame): keep the ui bridge as-is and
//! route interact output through grouped commits, one drag is one undo step.
//! Merge-lane route: one translate sweep folds via `merge_key` plus
//! `apply_coalesced` (one `pose.px` lane, seal on release) to one entry.

use engine_core::quat_mul;
use engine_scene::ids::ObjectId;
use engine_scene::transform::{Transform, MIN_INVERTIBLE_SCALE};

use crate::commands::Batch;

/// Which gizmo tool the viewport panel offers. Translate and rotate only:
/// scale stays in the inspector until a later task needs it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum GizmoTool {
    /// Arrow handles: move the selection in its parent frame.
    #[default]
    Translate,
    /// Ring handles: spin the selection in its parent frame.
    Rotate,
}

impl GizmoTool {
    /// Tab title shown in the viewport panel.
    pub fn title(self) -> &'static str {
        match self {
            Self::Translate => "Translate",
            Self::Rotate => "Rotate",
        }
    }
}

/// One validated gizmo gesture, expressed in the target's parent frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GizmoDrag {
    /// Parent-frame offset in meters, added to the local position.
    Translate {
        /// Offset in meters.
        delta: [f32; 3],
    },
    /// Parent-frame spin, premultiplied onto the local rotation.
    Rotate {
        /// Spin axis in the parent frame; must be finite and non-zero.
        axis: [f32; 3],
        /// Spin angle in radians; must be finite.
        angle_rad: f32,
    },
}

/// What rejected a gizmo drag. Nothing is staged or applied on error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GizmoError {
    /// A drag component held NaN or infinity.
    NonFinite,
    /// The spin axis was zero-length (or not a number once measured).
    DegenerateAxis,
    /// A world-to-local conversion crossed a near-zero scale.
    DegenerateTransform,
}

impl std::fmt::Display for GizmoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NonFinite => f.write_str("gizmo drag holds a non-finite value"),
            Self::DegenerateAxis => f.write_str("gizmo spin axis is degenerate"),
            Self::DegenerateTransform => f.write_str("gizmo pose sits across a degenerate scale"),
        }
    }
}

impl std::error::Error for GizmoError {}

/// Apply a parent-frame drag to a local pose. Err on non-finite input or a
/// degenerate spin axis; the caller stages the result as a batch.
pub fn drag_to_local(current: Transform, drag: &GizmoDrag) -> Result<Transform, GizmoError> {
    if !current.is_finite() {
        return Err(GizmoError::NonFinite);
    }
    match *drag {
        GizmoDrag::Translate { delta } => {
            if !delta.iter().all(|c| c.is_finite()) {
                return Err(GizmoError::NonFinite);
            }
            let position = [
                current.position[0] + delta[0],
                current.position[1] + delta[1],
                current.position[2] + delta[2],
            ];
            if !position.iter().all(|c| c.is_finite()) {
                return Err(GizmoError::NonFinite);
            }
            Ok(Transform::new(position, current.rotation, current.scale))
        }
        GizmoDrag::Rotate { axis, angle_rad } => {
            if !axis.iter().all(|c| c.is_finite()) || !angle_rad.is_finite() {
                return Err(GizmoError::NonFinite);
            }
            let len_sq = axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2];
            if !len_sq.is_finite() || len_sq < 1e-12 {
                return Err(GizmoError::DegenerateAxis);
            }
            let len = len_sq.sqrt();
            let half = angle_rad / 2.0;
            let (sin_h, cos_h) = half.sin_cos();
            if !sin_h.is_finite() || !cos_h.is_finite() {
                return Err(GizmoError::NonFinite);
            }
            let spin = [
                axis[0] / len * sin_h,
                axis[1] / len * sin_h,
                axis[2] / len * sin_h,
                cos_h,
            ];
            Ok(Transform::new(
                current.position,
                quat_normalized(quat_mul(spin, quat_normalized(current.rotation))),
                current.scale,
            ))
        }
    }
}

/// Convert a world pose back to the parent frame (`None` on non-finite input
/// or a near-zero scale on either pose). Pure `T * R * S` matrix math in
/// `f64`: invert the parent, compose with the child, decompose the result.
pub fn world_to_local(parent_world: Transform, child_world: Transform) -> Option<Transform> {
    if !parent_world.is_finite() || !child_world.is_finite() {
        return None;
    }
    if parent_world
        .scale
        .iter()
        .any(|s| !s.is_finite() || s.abs() < MIN_INVERTIBLE_SCALE)
    {
        return None;
    }
    let parent = Rigid::from_transform(parent_world)?;
    let child = Matrix::from_transform(child_world)?;
    // Parent inverse for `P(x) = R*S*x + t`: `S^-1 * R^T * (y - t)`.
    let rt = parent.rotation_transpose();
    let inv_scale = [
        1.0 / f64::from(parent_world.scale[0]),
        1.0 / f64::from(parent_world.scale[1]),
        1.0 / f64::from(parent_world.scale[2]),
    ];
    let rel_t = [
        f64::from(child_world.position[0]) - f64::from(parent_world.position[0]),
        f64::from(child_world.position[1]) - f64::from(parent_world.position[1]),
        f64::from(child_world.position[2]) - f64::from(parent_world.position[2]),
    ];
    let unrotated = rt.apply(rel_t);
    let local_t = [
        unrotated[0] * inv_scale[0],
        unrotated[1] * inv_scale[1],
        unrotated[2] * inv_scale[2],
    ];
    // Rotation part: `S^-1 * R^T * C_R`, then decompose column lengths.
    let mut cols = [[0.0; 3]; 3];
    for (slot, source) in cols.iter_mut().zip(child.rot.iter()) {
        let rotated = rt.apply(*source);
        *slot = [
            rotated[0] * inv_scale[0],
            rotated[1] * inv_scale[1],
            rotated[2] * inv_scale[2],
        ];
    }
    let mut scale = [0.0; 3];
    let mut rot = [[0.0; 3]; 3];
    for ((axis_len, axis_rot), column) in scale.iter_mut().zip(rot.iter_mut()).zip(cols.iter()) {
        let len = (column[0] * column[0] + column[1] * column[1] + column[2] * column[2]).sqrt();
        if !len.is_finite() || len < f64::from(MIN_INVERTIBLE_SCALE) {
            return None;
        }
        *axis_len = len;
        *axis_rot = [column[0] / len, column[1] / len, column[2] / len];
    }
    let rotation = quat_from_columns(rot)?;
    let out = Transform::new(
        [local_t[0] as f32, local_t[1] as f32, local_t[2] as f32],
        [
            rotation[0] as f32,
            rotation[1] as f32,
            rotation[2] as f32,
            rotation[3] as f32,
        ],
        [scale[0] as f32, scale[1] as f32, scale[2] as f32],
    );
    out.is_finite().then_some(out)
}

/// Single-op batch replacing a node's local pose. The caller validates it
/// (preview) before applying; the batch itself carries no verdict.
pub fn drag_batch(
    target: ObjectId,
    new_local: Transform,
    base_revision: u64,
    idempotency_key: u64,
) -> Batch {
    Batch {
        ops: vec![crate::commands::Op::SetLocal {
            id: target,
            local: new_local,
        }],
        base_revision,
        idempotency_key,
        provenance: None,
    }
}

/// Normalized parent rotation plus the pieces [`world_to_local`] needs.
struct Rigid {
    /// Rotation rows (orthonormal), in `f64`.
    rows: [[f64; 3]; 3],
}

impl Rigid {
    fn from_transform(t: Transform) -> Option<Self> {
        let q = quat_normalized_f64([
            f64::from(t.rotation[0]),
            f64::from(t.rotation[1]),
            f64::from(t.rotation[2]),
            f64::from(t.rotation[3]),
        ])?;
        // Rows of the `T * R * S` rotation block.
        let [x, y, z, w] = q;
        Some(Self {
            rows: [
                [
                    1.0 - 2.0 * (y * y + z * z),
                    2.0 * (x * y - z * w),
                    2.0 * (x * z + y * w),
                ],
                [
                    2.0 * (x * y + z * w),
                    1.0 - 2.0 * (x * x + z * z),
                    2.0 * (y * z - x * w),
                ],
                [
                    2.0 * (x * z - y * w),
                    2.0 * (y * z + x * w),
                    1.0 - 2.0 * (x * x + y * y),
                ],
            ],
        })
    }

    /// Row-matrix times a vector (callers pass an already-transposed
    /// rotation when they need the inverse turn).
    fn apply(&self, v: [f64; 3]) -> [f64; 3] {
        [
            self.rows[0][0] * v[0] + self.rows[0][1] * v[1] + self.rows[0][2] * v[2],
            self.rows[1][0] * v[0] + self.rows[1][1] * v[1] + self.rows[1][2] * v[2],
            self.rows[2][0] * v[0] + self.rows[2][1] * v[1] + self.rows[2][2] * v[2],
        ]
    }

    fn rotation_transpose(&self) -> Self {
        Self {
            rows: [
                [self.rows[0][0], self.rows[1][0], self.rows[2][0]],
                [self.rows[0][1], self.rows[1][1], self.rows[2][1]],
                [self.rows[0][2], self.rows[1][2], self.rows[2][2]],
            ],
        }
    }
}

/// Full `T * R * S` matrix split into rotation columns plus translation.
struct Matrix {
    /// Scaled rotation columns.
    rot: [[f64; 3]; 3],
}

impl Matrix {
    fn from_transform(t: Transform) -> Option<Self> {
        let parent = Rigid::from_transform(t)?;
        let s = [
            f64::from(t.scale[0]),
            f64::from(t.scale[1]),
            f64::from(t.scale[2]),
        ];
        if s.iter().any(|v| !v.is_finite()) {
            return None;
        }
        // Columns of `R * S`: rotation rows transposed, scaled per column.
        Some(Self {
            rot: [
                [
                    parent.rows[0][0] * s[0],
                    parent.rows[1][0] * s[0],
                    parent.rows[2][0] * s[0],
                ],
                [
                    parent.rows[0][1] * s[1],
                    parent.rows[1][1] * s[1],
                    parent.rows[2][1] * s[1],
                ],
                [
                    parent.rows[0][2] * s[2],
                    parent.rows[1][2] * s[2],
                    parent.rows[2][2] * s[2],
                ],
            ],
        })
    }
}

/// Unit quaternion from orthonormal rotation *columns*, `[x, y, z, w]`.
/// Deterministic Shepperd branches; None on garbage input.
fn quat_from_columns(cols: [[f64; 3]; 3]) -> Option<[f64; 4]> {
    for col in &cols {
        for c in col {
            if !c.is_finite() {
                return None;
            }
        }
    }
    // Row view of the column storage.
    let m = [
        [cols[0][0], cols[1][0], cols[2][0]],
        [cols[0][1], cols[1][1], cols[2][1]],
        [cols[0][2], cols[1][2], cols[2][2]],
    ];
    let trace = m[0][0] + m[1][1] + m[2][2];
    let q = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        [
            (m[2][1] - m[1][2]) / s,
            (m[0][2] - m[2][0]) / s,
            (m[1][0] - m[0][1]) / s,
            0.25 * s,
        ]
    } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
        let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
        [
            0.25 * s,
            (m[0][1] + m[1][0]) / s,
            (m[0][2] + m[2][0]) / s,
            (m[2][1] - m[1][2]) / s,
        ]
    } else if m[1][1] > m[2][2] {
        let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
        [
            (m[0][1] + m[1][0]) / s,
            0.25 * s,
            (m[1][2] + m[2][1]) / s,
            (m[0][2] - m[2][0]) / s,
        ]
    } else {
        let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
        [
            (m[0][2] + m[2][0]) / s,
            (m[1][2] + m[2][1]) / s,
            0.25 * s,
            (m[1][0] - m[0][1]) / s,
        ]
    };
    quat_normalized_f64(q)
}

/// Unit quaternion, or identity for degenerate input, f32.
fn quat_normalized(q: [f32; 4]) -> [f32; 4] {
    let len_sq = q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3];
    if !len_sq.is_finite() || len_sq < 1e-12 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let len = len_sq.sqrt();
    [q[0] / len, q[1] / len, q[2] / len, q[3] / len]
}

/// Unit quaternion, or None for degenerate input, f64.
fn quat_normalized_f64(q: [f64; 4]) -> Option<[f64; 4]> {
    let len_sq = q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3];
    if !len_sq.is_finite() || len_sq < 1e-12 {
        return None;
    }
    let len = len_sq.sqrt();
    Some([q[0] / len, q[1] / len, q[2] / len, q[3] / len])
}

/// `transform-gizmo` session bridge: engine [`Transform`] values in and out
/// of the real gizmo crate, plus camera sync from [`Viewport`](crate::viewport::Viewport).
///
/// Lives behind the `ui` feature because it names `egui` and
/// `transform-gizmo-egui` types. Headless gizmo math stays in this module's
/// core; this bridge only converts values and drives one gizmo frame.
#[cfg(feature = "ui")]
pub mod ui {
    use super::GizmoTool;
    use crate::viewport::Viewport;
    use engine_scene::transform::Transform;

    /// Engine pose in `transform-gizmo` value form (f64 world frame).
    pub fn to_gizmo_transform(local: Transform) -> transform_gizmo::math::Transform {
        transform_gizmo::math::Transform::from_scale_rotation_translation(
            [
                f64::from(local.scale[0]),
                f64::from(local.scale[1]),
                f64::from(local.scale[2]),
            ],
            [
                f64::from(local.rotation[0]),
                f64::from(local.rotation[1]),
                f64::from(local.rotation[2]),
                f64::from(local.rotation[3]),
            ],
            [
                f64::from(local.position[0]),
                f64::from(local.position[1]),
                f64::from(local.position[2]),
            ],
        )
    }

    /// Back to an engine pose. None when the gizmo returned garbage.
    pub fn from_gizmo_transform(input: &transform_gizmo::math::Transform) -> Option<Transform> {
        let t: [f64; 3] = input.translation.into();
        let r: [f64; 4] = input.rotation.into();
        let s: [f64; 3] = input.scale.into();
        if t.iter().any(|c| !c.is_finite())
            || r.iter().any(|c| !c.is_finite())
            || s.iter().any(|c| !c.is_finite())
        {
            return None;
        }
        let out = Transform::new(
            [t[0] as f32, t[1] as f32, t[2] as f32],
            [r[0] as f32, r[1] as f32, r[2] as f32, r[3] as f32],
            [s[0] as f32, s[1] as f32, s[2] as f32],
        );
        out.is_finite().then_some(out)
    }

    /// Row-major rows into the `mint` layout the gizmo config expects.
    /// (`mint` arrives through the wired `transform-gizmo` re-export, so no
    /// extra dependency is needed.)
    fn to_row_matrix(rows: [[f64; 4]; 4]) -> transform_gizmo::mint::RowMatrix4<f64> {
        use transform_gizmo::mint::{RowMatrix4, Vector4};
        let row = |r: [f64; 4]| Vector4 {
            x: r[0],
            y: r[1],
            z: r[2],
            w: r[3],
        };
        RowMatrix4 {
            x: row(rows[0]),
            y: row(rows[1]),
            z: row(rows[2]),
            w: row(rows[3]),
        }
    }

    /// Identity fallback when the camera is degenerate.
    fn identity_matrix() -> transform_gizmo::mint::RowMatrix4<f64> {
        to_row_matrix([
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ])
    }

    /// One gizmo across frames: holds drag state, so the panel keeps one.
    pub struct GizmoSession {
        gizmo: transform_gizmo_egui::Gizmo,
        tool: GizmoTool,
    }

    impl GizmoSession {
        /// Fresh session with the translate tool.
        pub fn new() -> Self {
            Self {
                gizmo: transform_gizmo_egui::Gizmo::default(),
                tool: GizmoTool::Translate,
            }
        }

        /// Switch the offered handles.
        pub fn set_tool(&mut self, tool: GizmoTool) {
            self.tool = tool;
        }

        /// Current tool.
        pub fn tool(&self) -> GizmoTool {
            self.tool
        }

        /// Point the gizmo cameras at the viewport. The matrices stay
        /// unverified on a real GPU until the present loop is wired; the
        /// layout here follows the gizmo crate's column-vector convention
        /// (`projection * view * model`, OpenGL-style depth).
        pub fn sync_camera(&mut self, viewport: &Viewport) {
            let aspect = f64::from(viewport.size.aspect());
            let config = *self.gizmo.config();
            let modes = match self.tool {
                GizmoTool::Translate => transform_gizmo::config::GizmoMode::all_translate(),
                GizmoTool::Rotate => transform_gizmo::config::GizmoMode::all_rotate(),
            };
            self.gizmo
                .update_config(transform_gizmo::config::GizmoConfig {
                    view_matrix: viewport
                        .camera
                        .view_matrix_rows()
                        .map(to_row_matrix)
                        .unwrap_or_else(identity_matrix),
                    projection_matrix: viewport
                        .camera
                        .projection_matrix_rows(aspect)
                        .map(to_row_matrix)
                        .unwrap_or_else(identity_matrix),
                    viewport: transform_gizmo::math::Rect::from_min_size(
                        transform_gizmo::math::Pos2::ZERO,
                        transform_gizmo::math::Vec2::new(viewport.size.width, viewport.size.height),
                    ),
                    modes,
                    mode_override: None,
                    orientation: transform_gizmo::config::GizmoOrientation::Local,
                    ..config
                });
        }

        /// Run one gizmo frame over the selection's *world* pose. Returns the
        /// edited world pose when the drag moved it this frame, else None.
        /// The caller converts to the parent frame and stages a batch.
        pub fn interact(&mut self, ui: &egui::Ui, world: Transform) -> Option<Transform> {
            use transform_gizmo_egui::GizmoExt as _;
            let current = to_gizmo_transform(world);
            let (_result, edited) = self.gizmo.interact(ui, &[current])?;
            edited.first().and_then(from_gizmo_transform)
        }
    }

    impl Default for GizmoSession {
        fn default() -> Self {
            Self::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_scene::transform::Hierarchy;

    const EPS: f32 = 1e-4;

    fn near(a: &[f32], b: &[f32]) -> bool {
        a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() <= EPS)
    }

    #[test]
    fn translate_drag_shifts_position_only() {
        let start = Transform::new([1.0, 2.0, 3.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]);
        let out = drag_to_local(
            start,
            &GizmoDrag::Translate {
                delta: [4.0, -2.0, 0.5],
            },
        )
        .unwrap();
        assert!(near(&out.position, &[5.0, 0.0, 3.5]));
        assert_eq!(out.rotation, start.rotation);
        assert_eq!(out.scale, start.scale);
    }

    #[test]
    fn rotate_quarter_about_y_maps_x_to_minus_z() {
        let start = Transform::identity();
        let out = drag_to_local(
            start,
            &GizmoDrag::Rotate {
                axis: [0.0, 1.0, 0.0],
                angle_rad: std::f32::consts::FRAC_PI_2,
            },
        )
        .unwrap();
        // The engine convention turns +X onto -Z for a +90° Y spin.
        let half = std::f32::consts::FRAC_PI_4;
        assert!(near(&out.rotation, &[0.0, half.sin(), 0.0, half.cos()]));
        assert!(near(&out.position, &[0.0, 0.0, 0.0]));
        assert!(near(&out.scale, &[1.0, 1.0, 1.0]));
        // Unnormalized axes behave like normalized ones.
        let big = drag_to_local(
            start,
            &GizmoDrag::Rotate {
                axis: [0.0, 3.0, 0.0],
                angle_rad: std::f32::consts::FRAC_PI_2,
            },
        )
        .unwrap();
        assert!(near(&big.rotation, &out.rotation));
    }

    #[test]
    fn invalid_drag_rejected_before_any_batch() {
        let start = Transform::identity();
        let nan_delta = GizmoDrag::Translate {
            delta: [f32::NAN, 0.0, 0.0],
        };
        assert_eq!(drag_to_local(start, &nan_delta), Err(GizmoError::NonFinite));
        let inf_spin = GizmoDrag::Rotate {
            axis: [0.0, 1.0, 0.0],
            angle_rad: f32::INFINITY,
        };
        assert_eq!(drag_to_local(start, &inf_spin), Err(GizmoError::NonFinite));
        for axis in [[0.0, 0.0, 0.0], [f32::NAN, 0.0, 0.0], [1e-20, 0.0, 0.0]] {
            let spin = GizmoDrag::Rotate {
                axis,
                angle_rad: 0.5,
            };
            let error = drag_to_local(start, &spin).unwrap_err();
            assert!(
                error == GizmoError::DegenerateAxis || error == GizmoError::NonFinite,
                "axis {axis:?} rejected as {error}"
            );
        }
        let bad_pose = Transform::new([f32::NAN, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]);
        assert_eq!(
            drag_to_local(
                bad_pose,
                &GizmoDrag::Translate {
                    delta: [1.0, 0.0, 0.0]
                }
            ),
            Err(GizmoError::NonFinite)
        );
    }

    #[test]
    fn world_to_local_roundtrips_through_hierarchy_world() {
        let mut tree = Hierarchy::new();
        let parent = ObjectId::new();
        let child = ObjectId::new();
        let quarter = std::f32::consts::FRAC_1_SQRT_2;
        let parent_local = Transform::new(
            [5.0, 0.0, 0.0],
            [0.0, quarter, 0.0, quarter],
            [2.0, 2.0, 2.0],
        );
        let child_local = Transform::new([1.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0], [0.5, 1.0, 1.5]);
        tree.insert(parent, ObjectId::NIL, parent_local).unwrap();
        tree.insert(child, parent, child_local).unwrap();
        let parent_world = tree.world(parent).unwrap();
        let child_world = tree.world(child).unwrap();
        let back = world_to_local(parent_world, child_world).expect("invertible chain");
        assert!(near(&back.position, &child_local.position), "{back:?}");
        assert!(near(&back.rotation, &child_local.rotation), "{back:?}");
        assert!(near(&back.scale, &child_local.scale), "{back:?}");
        // Roots convert through identity.
        let root_back = world_to_local(Transform::identity(), parent_world).expect("root converts");
        assert!(near(&root_back.position, &parent_local.position));
    }

    #[test]
    fn world_to_local_rejects_degenerate_scales() {
        let child_world = Transform::new([1.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]);
        let flat_parent = Transform::new([0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [0.0, 1.0, 1.0]);
        assert_eq!(world_to_local(flat_parent, child_world), None);
        let nan_world = Transform::new([f32::NAN, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]);
        assert_eq!(world_to_local(Transform::identity(), nan_world), None);
    }

    #[test]
    fn drag_batch_carries_single_set_local_op() {
        let target = ObjectId::new();
        let local = Transform::new([9.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]);
        let batch = drag_batch(target, local, 4, 77);
        assert_eq!(batch.base_revision, 4);
        assert_eq!(batch.idempotency_key, 77);
        assert_eq!(batch.ops.len(), 1);
        assert!(matches!(
            batch.ops[0],
            crate::commands::Op::SetLocal { id, local: l } if id == target && l == local
        ));
    }
}
