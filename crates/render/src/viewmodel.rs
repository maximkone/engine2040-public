//! viewmodel: the first-person helmet rim.
//!
//! The rim is a lit 3D mesh built from an aperture outline table: each
//! preset carries its 2D outline (the opening the wearer looks through),
//! its holes (the eye holes of the face mask) and its bars (the nose bar,
//! the cheek guards). The table is plain data; the mesh builder turns one
//! row into closed solids, so every normal is finite and no edge is open.
//!
//! The rim draws in its own depth range on its own depth target, so the
//! world never clips it: a wall five centimetres in front of the eye
//! cannot cover the rim, because the world's depth never reaches the rim's
//! buffer.
//!
//! The rim takes the scene light (the arena day sun) and the dirt and
//! blood decals the wearer picks up.
//!
//! Owner: render lane. Consumer: the first-person view (the game's
//! `fps_view.rs`) and the capture helper.
//!
//! Units: the table, the outline and the decals are in millimeters, eye
//! space (x right, y up, z forward in front of the eye); the mesh and the
//! depth range are in meters. Determinism: the mesh, the decals and the
//! projection are pure functions of the table, so the same preset gives the
//! same bytes twice.

use std::fmt;

/// A quarter turn, the corner step of the rounded outlines.
const FRAC_PI_2: f32 = std::f32::consts::FRAC_PI_2;
/// Pi, the bound of the field of view.
const PI: f32 = std::f32::consts::PI;

/// The viewmodel shader, the file the naga gate validates.
pub const VIEWMODEL_WGSL: &str = include_str!("shaders/viewmodel.wgsl");

/// The iron the rim is made of: the dark metal of a legionary's galea.
const RIM_BASE_COLOR: [f32; 3] = [0.150, 0.146, 0.144];
/// The roughness of the bare iron.
const RIM_BASE_ROUGHNESS: f32 = 0.55;
/// The color of dried dirt on the rim.
const DIRT_COLOR: [f32; 3] = [0.255, 0.185, 0.115];
/// The roughness of dried dirt: matte.
const DIRT_ROUGHNESS: f32 = 0.92;
/// The color of wet blood on the rim.
const BLOOD_COLOR: [f32; 3] = [0.235, 0.022, 0.020];
/// The roughness of wet blood: glossy.
const BLOOD_ROUGHNESS: f32 = 0.28;
/// The sky fill: the light the open sky throws back onto the iron. The
/// sun is ahead of the eye, so the face of the rim that looks at the
/// wearer is lit by the sky alone.
const SKY_FILL: [f32; 3] = [0.45, 0.48, 0.56];
/// Points on every corner arc of an opening.
const OPENING_CORNER_POINTS: usize = 6;
/// Points on every corner arc of a hole.
const HOLE_CORNER_POINTS: usize = 5;

// ---------------------------------------------------------------------------
// The aperture outline table.
// ---------------------------------------------------------------------------

/// What a hole in an aperture is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HoleKind {
    /// An eye hole: the wearer looks through it.
    Eye,
    /// The gap a nose bar leaves.
    Nose,
}

/// What a bar across an aperture is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BarKind {
    /// The nose bar of a face mask.
    NoseBar,
    /// A cheek guard of a galea.
    CheekGuard,
}

/// One hole of an aperture outline.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ApertureHole {
    /// What the hole is.
    pub kind: HoleKind,
    /// The hole's center, in the outline's millimeters.
    pub center_mm: [f32; 2],
    /// The hole's half size, in millimeters.
    pub half_mm: [f32; 2],
    /// The round of the hole's corners, in millimeters.
    pub corner_mm: f32,
    /// The width of the band that rings the hole, in millimeters.
    pub surround_mm: f32,
    /// How far the ring sits in front of the eye, in millimeters.
    pub z_front_mm: f32,
    /// How deep the ring runs back from its front, in millimeters.
    pub depth_mm: f32,
}

/// One solid bar across an aperture: the nose bar of a face mask, a cheek
/// guard of a galea.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ApertureBar {
    /// What the bar is.
    pub kind: BarKind,
    /// The bar's center, in the outline's millimeters.
    pub center_mm: [f32; 2],
    /// The bar's half size, in millimeters.
    pub half_mm: [f32; 2],
    /// The bar's tilt, radians.
    pub angle_rad: f32,
    /// How far the bar sits in front of the eye, in millimeters.
    pub z_front_mm: f32,
    /// How deep the bar runs back from its front, in millimeters.
    pub depth_mm: f32,
}

/// One row of the aperture outline table: the 2D opening the wearer looks
/// through, the holes in it and the bars across it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ApertureOutline {
    /// The opening's half width, in millimeters.
    pub half_width_mm: f32,
    /// The opening's half height, in millimeters.
    pub half_height_mm: f32,
    /// The round of the opening's corners, in millimeters.
    pub corner_mm: f32,
    /// How far the rim sits in front of the eye, in millimeters.
    pub z_front_mm: f32,
    /// How deep the rim runs back from its front, in millimeters.
    pub depth_mm: f32,
    /// The width of the band that follows the opening, in millimeters.
    pub band_mm: f32,
    /// The holes in the opening.
    pub holes: &'static [ApertureHole],
    /// The bars across the opening.
    pub bars: &'static [ApertureBar],
}

/// The aperture outline table: the presets the first-person view ships.
///
/// The galea is the wide opening with a rim around it and two cheek guards
/// rising from the lower sides. The face mask is the mask edge with two eye
/// holes and a nose bar between them.
///
/// Every solid of a preset sits between 54 and 80 millimetres in front of
/// the eye: nearer than that, the world could stand in front of the rim,
/// and the rim's own depth range would have to fight for it.
pub const APERTURE_TABLE: [ApertureOutline; 2] = [
    ApertureOutline {
        half_width_mm: 72.0,
        half_height_mm: 50.0,
        corner_mm: 16.0,
        z_front_mm: 70.0,
        depth_mm: 15.0,
        band_mm: 16.0,
        holes: &[],
        bars: &[
            ApertureBar {
                kind: BarKind::CheekGuard,
                center_mm: [-46.0, -22.0],
                half_mm: [10.0, 18.0],
                angle_rad: -0.20,
                z_front_mm: 62.0,
                depth_mm: 8.0,
            },
            ApertureBar {
                kind: BarKind::CheekGuard,
                center_mm: [46.0, -22.0],
                half_mm: [10.0, 18.0],
                angle_rad: 0.20,
                z_front_mm: 62.0,
                depth_mm: 8.0,
            },
        ],
    },
    ApertureOutline {
        half_width_mm: 78.0,
        half_height_mm: 55.0,
        corner_mm: 18.0,
        z_front_mm: 75.0,
        depth_mm: 15.0,
        band_mm: 18.0,
        holes: &[
            ApertureHole {
                kind: HoleKind::Eye,
                center_mm: [-18.0, 5.0],
                half_mm: [13.0, 9.0],
                corner_mm: 5.0,
                surround_mm: 8.0,
                z_front_mm: 80.0,
                depth_mm: 14.0,
            },
            ApertureHole {
                kind: HoleKind::Eye,
                center_mm: [18.0, 5.0],
                half_mm: [13.0, 9.0],
                corner_mm: 5.0,
                surround_mm: 8.0,
                z_front_mm: 80.0,
                depth_mm: 14.0,
            },
        ],
        bars: &[ApertureBar {
            kind: BarKind::NoseBar,
            center_mm: [0.0, -12.0],
            half_mm: [5.0, 16.0],
            angle_rad: 0.0,
            z_front_mm: 78.0,
            depth_mm: 14.0,
        }],
    },
];

/// The aperture presets the first-person view ships.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AperturePreset {
    /// The galea with cheek guards: a wide opening with a rim around it.
    GaleaCheekGuards,
    /// The face mask: the mask edge, two eye holes and a nose bar.
    FaceMask,
}

impl AperturePreset {
    /// The preset name, used for capture file names.
    pub fn name(self) -> &'static str {
        match self {
            Self::GaleaCheekGuards => "galea",
            Self::FaceMask => "face_mask",
        }
    }

    /// The row of the outline table this preset reads.
    pub fn table_row(self) -> ApertureOutline {
        APERTURE_TABLE[self.table_index()]
    }

    /// The index of the preset's row in the table.
    pub fn table_index(self) -> usize {
        match self {
            Self::GaleaCheekGuards => 0,
            Self::FaceMask => 1,
        }
    }

    /// Every preset the table holds.
    pub fn all() -> [AperturePreset; 2] {
        [Self::GaleaCheekGuards, Self::FaceMask]
    }
}

impl ApertureOutline {
    /// The 2D outline of the opening: a rounded rectangle around the eye,
    /// counter-clockwise, in millimeters.
    pub fn points(&self) -> Vec<[f32; 2]> {
        rounded_rect(
            self.half_width_mm,
            self.half_height_mm,
            self.corner_mm,
            OPENING_CORNER_POINTS,
        )
    }
}

impl ApertureHole {
    /// The 2D points of the hole: a rounded rectangle around its own
    /// center, counter-clockwise, in millimeters.
    pub fn points(&self) -> Vec<[f32; 2]> {
        let local = rounded_rect(
            self.half_mm[0],
            self.half_mm[1],
            self.corner_mm,
            HOLE_CORNER_POINTS,
        );
        local
            .iter()
            .map(|point| [point[0] + self.center_mm[0], point[1] + self.center_mm[1]])
            .collect()
    }
}

impl ApertureBar {
    /// The 2D points of the bar: a tilted rectangle around its center,
    /// counter-clockwise, in millimeters.
    pub fn points(&self) -> Vec<[f32; 2]> {
        let (sin, cos) = self.angle_rad.sin_cos();
        let local = [
            [-self.half_mm[0], -self.half_mm[1]],
            [self.half_mm[0], -self.half_mm[1]],
            [self.half_mm[0], self.half_mm[1]],
            [-self.half_mm[0], self.half_mm[1]],
        ];
        local
            .iter()
            .map(|point| {
                [
                    point[0] * cos - point[1] * sin + self.center_mm[0],
                    point[0] * sin + point[1] * cos + self.center_mm[1],
                ]
            })
            .collect()
    }
}

/// The points of a rounded rectangle, counter-clockwise, `per_corner` points
/// on every corner arc.
fn rounded_rect(
    half_width: f32,
    half_height: f32,
    corner: f32,
    per_corner: usize,
) -> Vec<[f32; 2]> {
    let corner = corner.min(half_width).min(half_height);
    let inset_x = half_width - corner;
    let inset_y = half_height - corner;
    let centers = [
        [inset_x, inset_y],
        [-inset_x, inset_y],
        [-inset_x, -inset_y],
        [inset_x, -inset_y],
    ];
    let mut out = Vec::with_capacity(per_corner * 4);
    for (index, center) in centers.iter().enumerate() {
        let start = FRAC_PI_2 * f32::from(u16::try_from(index).unwrap_or(0));
        for step in 0..per_corner {
            let last = f32::from(u16::try_from(per_corner.saturating_sub(1)).unwrap_or(1));
            let t = f32::from(u16::try_from(step).unwrap_or(0)) / last;
            let angle = start + FRAC_PI_2 * t;
            out.push([
                center[0] + corner * angle.cos(),
                center[1] + corner * angle.sin(),
            ]);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The mesh.
// ---------------------------------------------------------------------------

/// One vertex of the rim mesh.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RimVertex {
    /// The position in eye space, meters: x right, y up, z forward.
    pub position_m: [f32; 3],
    /// The unit normal of the face.
    pub normal: [f32; 3],
    /// The base color of the iron, linear.
    pub color: [f32; 3],
    /// The perceptual roughness, 0..1.
    pub roughness: f32,
}

/// The rim mesh: the vertices and the triangle indices.
#[derive(Clone, PartialEq, Debug)]
pub struct RimMesh {
    /// The vertices, three per triangle (the rim is flat shaded).
    pub vertices: Vec<RimVertex>,
    /// The triangle indices, three per triangle.
    pub indices: Vec<u32>,
}

impl RimMesh {
    /// The number of triangles in the mesh.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

/// Build the rim mesh of one preset: the band around the opening, the
/// rings around the holes and the bars across it.
///
/// Every solid in the mesh is closed: each edge of each triangle is shared
/// by exactly one other triangle, so the mesh has no open edge and every
/// normal is finite. Errors: [`ViewmodelError::Invalid`] when the table
/// row cannot build a closed solid.
pub fn build_rim_mesh(preset: AperturePreset) -> Result<RimMesh, ViewmodelError> {
    let outline = preset.table_row();
    let mut mesh = RimMesh {
        vertices: Vec::new(),
        indices: Vec::new(),
    };
    let outer = outline.points();
    let inner = offset_loop(&outer, outline.band_mm)?;
    push_band(
        &mut mesh,
        &outer,
        &inner,
        outline.z_front_mm,
        outline.depth_mm,
    )?;
    for hole in outline.holes {
        let hole_points = hole.points();
        let hole_outer = offset_loop(&hole_points, -hole.surround_mm)?;
        push_band(
            &mut mesh,
            &hole_outer,
            &hole_points,
            hole.z_front_mm,
            hole.depth_mm,
        )?;
    }
    for bar in outline.bars {
        push_prism(&mut mesh, &bar.points(), bar.z_front_mm, bar.depth_mm)?;
    }
    Ok(mesh)
}

/// Offset a closed loop: a positive distance moves every point to the left
/// of the walk (into the loop's interior), a negative one to the right.
fn offset_loop(points: &[[f32; 2]], distance_mm: f32) -> Result<Vec<[f32; 2]>, ViewmodelError> {
    let count = points.len();
    if count < 3 {
        return Err(ViewmodelError::Invalid(
            "a rim loop needs three points".into(),
        ));
    }
    let mut out = Vec::with_capacity(count);
    for index in 0..count {
        let previous = points[(index + count - 1) % count];
        let next = points[(index + 1) % count];
        let here = points[index];
        let first = left_normal([here[0] - previous[0], here[1] - previous[1]])?;
        let second = left_normal([next[0] - here[0], next[1] - here[1]])?;
        let mut bisector = [first[0] + second[0], first[1] + second[1]];
        let length = (bisector[0] * bisector[0] + bisector[1] * bisector[1]).sqrt();
        if length < 1e-6 {
            bisector = first;
        } else {
            bisector = [bisector[0] / length, bisector[1] / length];
        }
        let along = (bisector[0] * first[0] + bisector[1] * first[1]).max(0.25);
        let scale = distance_mm / along;
        out.push([here[0] + bisector[0] * scale, here[1] + bisector[1] * scale]);
    }
    Ok(out)
}

/// The unit normal to the left of an edge.
fn left_normal(edge: [f32; 2]) -> Result<[f32; 2], ViewmodelError> {
    let length = (edge[0] * edge[0] + edge[1] * edge[1]).sqrt();
    if length < 1e-9 {
        return Err(ViewmodelError::Invalid(
            "a rim loop has a zero-length edge".into(),
        ));
    }
    Ok([-edge[1] / length, edge[0] / length])
}

/// Push one closed band: the solid between two loops of the same length,
/// `z_front_mm` in front of the eye and `depth_mm` deep.
fn push_band(
    mesh: &mut RimMesh,
    outer: &[[f32; 2]],
    inner: &[[f32; 2]],
    z_front_mm: f32,
    depth_mm: f32,
) -> Result<(), ViewmodelError> {
    let count = outer.len();
    if count < 3 || inner.len() != count {
        return Err(ViewmodelError::Invalid(
            "a rim band needs two loops of the same length".into(),
        ));
    }
    let z_front = z_front_mm / 1000.0;
    let z_back = (z_front_mm - depth_mm) / 1000.0;
    for index in 0..count {
        let next = (index + 1) % count;
        let outer_here = point_3d(outer[index], z_front);
        let outer_next = point_3d(outer[next], z_front);
        let inner_here = point_3d(inner[index], z_front);
        let inner_next = point_3d(inner[next], z_front);
        let outer_back = point_3d(outer[index], z_back);
        let outer_back_next = point_3d(outer[next], z_back);
        let inner_back = point_3d(inner[index], z_back);
        let inner_back_next = point_3d(inner[next], z_back);
        // The front face looks at the eye.
        push_triangle(mesh, outer_here, outer_next, inner_next)?;
        push_triangle(mesh, outer_here, inner_next, inner_here)?;
        // The back face looks away from it.
        push_triangle(mesh, outer_back, inner_back, inner_back_next)?;
        push_triangle(mesh, outer_back, inner_back_next, outer_back_next)?;
        // The outer wall looks away from the opening.
        push_triangle(mesh, outer_here, outer_back, outer_back_next)?;
        push_triangle(mesh, outer_here, outer_back_next, outer_next)?;
        // The inner wall looks into the hole.
        push_triangle(mesh, inner_here, inner_next, inner_back_next)?;
        push_triangle(mesh, inner_here, inner_back_next, inner_back)?;
    }
    Ok(())
}

/// Push one closed prism: the solid under a loop, `z_front_mm` in front of
/// the eye and `depth_mm` deep.
fn push_prism(
    mesh: &mut RimMesh,
    points: &[[f32; 2]],
    z_front_mm: f32,
    depth_mm: f32,
) -> Result<(), ViewmodelError> {
    let count = points.len();
    if count < 3 {
        return Err(ViewmodelError::Invalid(
            "a rim prism needs three points".into(),
        ));
    }
    let z_front = z_front_mm / 1000.0;
    let z_back = (z_front_mm - depth_mm) / 1000.0;
    let front: Vec<[f32; 3]> = points.iter().map(|p| point_3d(*p, z_front)).collect();
    let back: Vec<[f32; 3]> = points.iter().map(|p| point_3d(*p, z_back)).collect();
    for index in 1..count.saturating_sub(1) {
        push_triangle(mesh, front[0], front[index], front[index + 1])?;
    }
    for index in 1..count.saturating_sub(1) {
        push_triangle(mesh, back[0], back[index + 1], back[index])?;
    }
    for index in 0..count {
        let next = (index + 1) % count;
        push_triangle(mesh, front[index], back[index], back[next])?;
        push_triangle(mesh, front[index], back[next], front[next])?;
    }
    Ok(())
}

/// A 2D point of the outline as a 3D point of the mesh.
fn point_3d(point: [f32; 2], z: f32) -> [f32; 3] {
    [point[0] / 1000.0, point[1] / 1000.0, z]
}

/// Push one triangle with its flat normal.
fn push_triangle(
    mesh: &mut RimMesh,
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
) -> Result<(), ViewmodelError> {
    let normal = triangle_normal(a, b, c)?;
    let base = u32::try_from(mesh.vertices.len()).map_err(|_| {
        ViewmodelError::Invalid("the rim mesh has more vertices than an index can name".into())
    })?;
    for point in [a, b, c] {
        mesh.vertices.push(RimVertex {
            position_m: point,
            normal,
            color: RIM_BASE_COLOR,
            roughness: RIM_BASE_ROUGHNESS,
        });
    }
    for offset in 0..3u32 {
        mesh.indices.push(base + offset);
    }
    Ok(())
}

/// The unit normal of a triangle, or a refusal when the triangle is
/// degenerate.
fn triangle_normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> Result<[f32; 3], ViewmodelError> {
    let first = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let second = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let cross = [
        first[1] * second[2] - first[2] * second[1],
        first[2] * second[0] - first[0] * second[2],
        first[0] * second[1] - first[1] * second[0],
    ];
    let length = (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
    if !length.is_finite() || length < 1e-12 {
        return Err(ViewmodelError::Invalid(
            "a rim triangle is degenerate".into(),
        ));
    }
    Ok([cross[0] / length, cross[1] / length, cross[2] / length])
}

// ---------------------------------------------------------------------------
// The decals: the dirt and the blood the wearer picks up.
// ---------------------------------------------------------------------------

/// What a decal paints on the rim.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DecalKind {
    /// Dried dirt.
    Dirt,
    /// Wet blood.
    Blood,
}

/// One dirt or blood patch on the rim.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RimDecal {
    /// What the patch is.
    pub kind: DecalKind,
    /// Where the patch sits, in the outline's millimeters.
    pub at_mm: [f32; 2],
    /// The patch radius, in millimeters.
    pub radius_mm: f32,
    /// How strong the patch is, 0..1.
    pub strength: f32,
}

/// The decals a wearer carries after a fight: the same patches for every
/// preset, so the rim never changes shape when it gets dirty. Every patch
/// sits on the band, where the wearer's hand and the battle reach it.
pub fn worn_decals() -> Vec<RimDecal> {
    vec![
        RimDecal {
            kind: DecalKind::Dirt,
            at_mm: [-64.0, 10.0],
            radius_mm: 13.0,
            strength: 0.75,
        },
        RimDecal {
            kind: DecalKind::Dirt,
            at_mm: [62.0, -20.0],
            radius_mm: 11.0,
            strength: 0.6,
        },
        RimDecal {
            kind: DecalKind::Blood,
            at_mm: [30.0, 42.0],
            radius_mm: 10.0,
            strength: 0.85,
        },
        RimDecal {
            kind: DecalKind::Blood,
            at_mm: [-10.0, -44.0],
            radius_mm: 9.0,
            strength: 0.7,
        },
    ]
}

/// Paint the decals on the rim: the vertex colors and roughnesses carry
/// the dirt and the blood.
pub fn apply_decals(mesh: &mut RimMesh, decals: &[RimDecal]) {
    for vertex in &mut mesh.vertices {
        let at = [vertex.position_m[0] * 1000.0, vertex.position_m[1] * 1000.0];
        for decal in decals {
            let dx = at[0] - decal.at_mm[0];
            let dy = at[1] - decal.at_mm[1];
            let distance = (dx * dx + dy * dy).sqrt();
            if !distance.is_finite() || distance >= decal.radius_mm {
                continue;
            }
            let coverage = (decal.strength * (1.0 - distance / decal.radius_mm)).clamp(0.0, 1.0);
            let (color, roughness) = match decal.kind {
                DecalKind::Dirt => (DIRT_COLOR, DIRT_ROUGHNESS),
                DecalKind::Blood => (BLOOD_COLOR, BLOOD_ROUGHNESS),
            };
            for (base, paint) in vertex.color.iter_mut().zip(color.iter()) {
                *base += (*paint - *base) * coverage;
            }
            vertex.roughness += (roughness - vertex.roughness) * coverage;
        }
    }
}

// ---------------------------------------------------------------------------
// The scene light.
// ---------------------------------------------------------------------------

/// The scene light the rim takes: the sun the world is lit by.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RimLight {
    /// The direction toward the light, unit length.
    pub direction: [f32; 3],
    /// The light color, linear.
    pub color: [f32; 3],
    /// The intensity scale.
    pub intensity: f32,
    /// The ambient term.
    pub ambient: [f32; 3],
}

impl RimLight {
    /// The arena day sun: the light the world is lit by.
    pub fn scene() -> Self {
        let sun = crate::arena_day::ARENA_DAY_SUN_DIR;
        let length = (sun[0] * sun[0] + sun[1] * sun[1] + sun[2] * sun[2]).sqrt();
        Self {
            direction: [sun[0] / length, sun[1] / length, sun[2] / length],
            color: [1.0, 0.94, 0.82],
            intensity: 1.35,
            ambient: SKY_FILL,
        }
    }

    /// The lit color of one rim vertex: the scene light over the base
    /// color, with a soft highlight off the iron. The WGSL in
    /// `shaders/viewmodel.wgsl` runs the same law.
    pub fn shade(&self, color: [f32; 3], normal: [f32; 3], roughness: f32) -> [f32; 3] {
        let n = normalize3(normal);
        let l = normalize3(self.direction);
        let lambert = (n[0] * l[0] + n[1] * l[1] + n[2] * l[2]).max(0.0);
        let half = normalize3([l[0], l[1], l[2] + 1.0]);
        let exponent = 64.0 + (4.0 - 64.0) * roughness;
        let facing = (n[0] * half[0] + n[1] * half[1] + n[2] * half[2]).max(0.0);
        let highlight = facing.powf(exponent) * (1.0 - roughness) * 0.3;
        let radiance = [
            self.color[0] * self.intensity,
            self.color[1] * self.intensity,
            self.color[2] * self.intensity,
        ];
        let mut out = [0.0f32; 3];
        for channel in 0..3 {
            out[channel] =
                color[channel] * (self.ambient[channel] + radiance[channel] * lambert) + highlight;
        }
        out
    }
}

#[cfg(feature = "v1")]
impl RimLight {
    /// The rim light of the world's directional light: the same sun the
    /// world is lit by.
    pub fn from_directional(light: &crate::v1::DirectionalLight) -> Result<Self, ViewmodelError> {
        light
            .validate()
            .map_err(|err| ViewmodelError::Invalid(format!("viewmodel light: {err}")))?;
        let direction = light.direction.normalize();
        Ok(Self {
            direction: direction.to_array(),
            color: light.color.to_array(),
            intensity: light.intensity,
            ambient: SKY_FILL,
        })
    }
}

/// A unit vector, or the zero vector when the input has no length.
fn normalize3(vector: [f32; 3]) -> [f32; 3] {
    let length = (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt();
    if !length.is_finite() || length < 1e-9 {
        return [0.0, 0.0, 0.0];
    }
    [vector[0] / length, vector[1] / length, vector[2] / length]
}

// ---------------------------------------------------------------------------
// The depth range: the rim draws in its own range, on its own target.
// ---------------------------------------------------------------------------

/// One depth range: the near and far planes, in meters.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct DepthRange {
    /// The near plane, in meters.
    pub near_m: f32,
    /// The far plane, in meters.
    pub far_m: f32,
}

/// The rim's own depth range: the rim lives between 5 mm and 500 mm in
/// front of the eye. The world never writes into this range: the rim draws
/// on its own depth target with its own projection, so a wall five
/// centimetres in front of the eye cannot cover it.
pub const VIEWMODEL_DEPTH: DepthRange = DepthRange {
    near_m: 0.005,
    far_m: 0.5,
};

impl DepthRange {
    /// A range with the near and far planes, in meters. The planes must be
    /// finite, positive and ordered.
    pub fn new(near_m: f32, far_m: f32) -> Result<Self, ViewmodelError> {
        if !near_m.is_finite() || !far_m.is_finite() || near_m <= 0.0 || far_m <= near_m {
            return Err(ViewmodelError::Invalid(format!(
                "viewmodel depth range {near_m}..{far_m} is not 0 < near < far"
            )));
        }
        Ok(Self { near_m, far_m })
    }

    /// The normalized depth of a point `z_m` in front of the eye: 0 at the
    /// near plane, 1 at the far plane. `None` when the point is outside the
    /// range.
    pub fn depth_at(&self, z_m: f32) -> Option<f32> {
        if !z_m.is_finite() || z_m < self.near_m || z_m > self.far_m {
            return None;
        }
        let depth = self.far_m * (z_m - self.near_m) / ((self.far_m - self.near_m) * z_m);
        Some(depth.clamp(0.0, 1.0))
    }
}

/// One depth target: a range plus the depth written so far. The world's
/// target and the rim's target are two of these, never one.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct DepthTarget {
    range: DepthRange,
    depth: f32,
}

impl DepthTarget {
    /// A cleared target over the range.
    pub fn new(range: DepthRange) -> Self {
        Self { range, depth: 1.0 }
    }

    /// The range the target reads.
    pub fn range(&self) -> DepthRange {
        self.range
    }

    /// The depth written so far, 0 at the near plane and 1 at the far one.
    pub fn depth(&self) -> f32 {
        self.depth
    }

    /// Forget everything written so far.
    pub fn clear(&mut self) {
        self.depth = 1.0;
    }

    /// Write a fragment at `z_m` in front of the eye: it shows when it is
    /// inside the range and nearer than what is already there.
    pub fn write(&mut self, z_m: f32) -> bool {
        let Some(depth) = self.range.depth_at(z_m) else {
            return false;
        };
        if depth < self.depth {
            self.depth = depth;
            true
        } else {
            false
        }
    }
}

/// The view-projection of the viewmodel: the eye at the origin looking
/// down +z, with the range's own near and far planes.
///
/// The matrix is column-major, the layout a WGSL `mat4x4<f32>` reads.
/// Errors: [`ViewmodelError::Invalid`] for a field of view outside
/// `(0, pi)` or an aspect that is not positive.
pub fn viewmodel_view_proj(
    range: DepthRange,
    fov_y_rad: f32,
    aspect: f32,
) -> Result<[f32; 16], ViewmodelError> {
    if !fov_y_rad.is_finite() || fov_y_rad <= 0.0 || fov_y_rad >= PI {
        return Err(ViewmodelError::Invalid(format!(
            "viewmodel fov {fov_y_rad} is outside 0..pi"
        )));
    }
    if !aspect.is_finite() || aspect <= 0.0 {
        return Err(ViewmodelError::Invalid(format!(
            "viewmodel aspect {aspect} is not positive"
        )));
    }
    let focal = 1.0 / (fov_y_rad * 0.5).tan();
    let near = range.near_m;
    let far = range.far_m;
    let depth_a = far / (far - near);
    let depth_b = -near * far / (far - near);
    Ok([
        focal / aspect,
        0.0,
        0.0,
        0.0,
        0.0,
        focal,
        0.0,
        0.0,
        0.0,
        0.0,
        depth_a,
        1.0,
        0.0,
        0.0,
        depth_b,
        0.0,
    ])
}

// ---------------------------------------------------------------------------
// Errors.
// ---------------------------------------------------------------------------

/// Viewmodel failures. Every variant is a typed refusal; the pass never
/// guesses.
#[derive(Clone, PartialEq, Debug)]
pub enum ViewmodelError {
    /// The table or the caller gave data the mesh builder cannot use.
    Invalid(String),
    /// No live device on this call.
    Device(String),
    /// A wgpu validation scope fired inside the pass.
    Validation(String),
    /// A capture file could not be read or written.
    Io(String),
}

impl fmt::Display for ViewmodelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(err) => write!(f, "viewmodel input: {err}"),
            Self::Device(err) => write!(f, "viewmodel device: {err}"),
            Self::Validation(err) => write!(f, "viewmodel validation: {err}"),
            Self::Io(err) => write!(f, "viewmodel capture file: {err}"),
        }
    }
}

impl std::error::Error for ViewmodelError {}

// ---------------------------------------------------------------------------
// The GPU pass and the tests.
// ---------------------------------------------------------------------------

/// The GPU pass: the rim on its own depth target, plus the capture helper.
///
/// Native only: it blocks on the GPU and writes the capture PNG to disk.
#[cfg(all(feature = "gpu", not(target_arch = "wasm32")))]
pub mod gpu;

/// The tests of the rim: the outline table, the closed mesh, the depth
/// range and the capture pair.
#[cfg(test)]
mod tests;
