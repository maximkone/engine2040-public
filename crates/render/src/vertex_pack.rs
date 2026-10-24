//! vertex_pack: the 20-byte vertex the potato tier uploads, and its decode.
//!
//! The forward lane has always uploaded 48 bytes per vertex (position,
//! normal, uv0 and tangent as eight f32 lanes). A `float16`-class body does not
//! need that: a mesh-local snorm16 position carries a sub-millimetre error over
//! a metre-scale mesh, an octahedral pair of snorm16 carries the normal inside
//! a fifth of a degree, and a 16-bit uv is one texel of a 64k texture. This
//! module owns that packing on the CPU, so the cook and the shader can be
//! proved against one implementation before the pipeline switches layout.
//!
//! Layout (20 bytes, one draw carries `mesh_scale` so the position stays
//! mesh-local):
//! - `pos: [i16; 4]`: snorm16 x, y, z and `w = 1` (kept for the 4-lane load),
//! - `n: u32`: two snorm16 octahedral coordinates,
//! - `uv: [u16; 2]`: normalized uv0,
//! - `extra: u32`: the lane the 48-byte vertex used for tangent `w` (byte 0,
//!   `0` or `1`) and the material kind byte (byte 1), reserved for the shader.
//!
//! Units: metres in, metres out. Determinism: every step is `+ - * /`, `sqrt`,
//! `round` and a saturating convert in a fixed order, so the same mesh packs to
//! the same bytes twice. Caller: the cook packs beside the raw f32 mesh; the
//! forward and shadow vertex stages call the same octahedral decode.

/// Bytes one packed vertex costs on the GPU.
pub const VERTEX_STRIDE_BYTES: usize = 20;

/// Bytes the f32 forward vertex costs today (position, normal, uv0, tangent).
pub const F32_VERTEX_STRIDE_BYTES: usize = 48;

/// Largest snorm16 magnitude, as a float: the divisor every encode uses.
const SNORM_MAX: f32 = 32_767.0;

/// One packed vertex: 20 bytes, the stride the potato lane uploads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct VertexPacked {
    /// Mesh-local position, snorm16 per axis; `w` is always 1.
    pub pos: [i16; 4],
    /// Octahedral normal, two snorm16 lanes.
    pub n: u32,
    /// Normalized uv0.
    pub uv: [u16; 2],
    /// Tangent `w` in byte 0 (`0` or `1`) and the material kind in byte 1.
    pub extra: u32,
}

impl VertexPacked {
    /// Bytes per vertex; the shader's vertex layout must match this.
    #[must_use]
    pub fn stride_bytes() -> usize {
        std::mem::size_of::<Self>()
    }
}

/// Quantise one axis to snorm16. `scale` is the mesh-local extent (half width
/// times two), so `value / scale` lands in `-1..=1`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the quotient is clamped to -1..=1 and scaled by 32767, so it fits i16 exactly"
)]
fn snorm16(value: f32, scale: f32) -> i16 {
    if !scale.is_finite() || scale <= 0.0 {
        return 0;
    }
    let unit = (value / scale).clamp(-1.0, 1.0);
    (unit * SNORM_MAX).round() as i16
}

/// snorm16 back to a unit float.
fn snorm16_to_f32(raw: i16) -> f32 {
    f32::from(raw) / SNORM_MAX
}

/// Octahedral encode of a unit direction into two snorm16 lanes.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the octahedral coordinates are clamped to -1..=1 before scaling by 32767"
)]
#[must_use]
pub fn oct_encode(normal: [f32; 3]) -> [i16; 2] {
    let len = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
    let n = if len.is_finite() && len > 1e-12 {
        [normal[0] / len, normal[1] / len, normal[2] / len]
    } else {
        [0.0, 0.0, 1.0]
    };
    let sum = n[0].abs() + n[1].abs() + n[2].abs();
    let sum = if sum > 1e-12 { sum } else { 1.0 };
    let mut x = n[0] / sum;
    let mut y = n[1] / sum;
    if n[2] < 0.0 {
        // Fold the lower hemisphere out: the sign of the coordinate picks the
        // side, so decoding can rebuild z from 1 - |x| - |y|.
        let sx = if x < 0.0 { -1.0 } else { 1.0 };
        let sy = if y < 0.0 { -1.0 } else { 1.0 };
        let fx = (1.0 - y.abs()) * sx;
        let fy = (1.0 - x.abs()) * sy;
        x = fx;
        y = fy;
    }
    [
        (x.clamp(-1.0, 1.0) * SNORM_MAX).round() as i16,
        (y.clamp(-1.0, 1.0) * SNORM_MAX).round() as i16,
    ]
}

/// Octahedral decode back to a unit direction, the same reconstruction the
/// shader runs: `z = 1 - |x| - |y|`, then the lower-hemisphere fold.
#[must_use]
pub fn oct_decode(packed: [i16; 2]) -> [f32; 3] {
    let mut x = snorm16_to_f32(packed[0]);
    let mut y = snorm16_to_f32(packed[1]);
    let mut z = 1.0 - x.abs() - y.abs();
    if z < 0.0 {
        let sx = if x < 0.0 { -1.0 } else { 1.0 };
        let sy = if y < 0.0 { -1.0 } else { 1.0 };
        let fx = (1.0 - y.abs()) * sx;
        let fy = (1.0 - x.abs()) * sy;
        x = fx;
        y = fy;
    }
    let len = (x * x + y * y + z * z).sqrt();
    if len.is_finite() && len > 1e-12 {
        [x / len, y / len, z / len]
    } else {
        [0.0, 0.0, 1.0]
    }
}

/// Two snorm16 octahedral lanes packed into one `u32`: lane 0 is the low
/// half, lane 1 the high half, the order the shader's `unpack2x16snorm` reads.
#[must_use]
pub fn pack_oct(packed: [i16; 2]) -> u32 {
    let low = packed[0].to_le_bytes();
    let high = packed[1].to_le_bytes();
    u32::from_le_bytes([low[0], low[1], high[0], high[1]])
}

/// The two snorm16 octahedral lanes back out of one `u32`.
#[must_use]
pub fn unpack_oct(raw: u32) -> [i16; 2] {
    let bytes = raw.to_le_bytes();
    [
        i16::from_le_bytes([bytes[0], bytes[1]]),
        i16::from_le_bytes([bytes[2], bytes[3]]),
    ]
}

/// Quantise one uv lane to 16 bits.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the uv is clamped to 0..=1 before scaling by 65535"
)]
#[expect(
    clippy::cast_sign_loss,
    reason = "the clamped uv is non-negative before the round"
)]
fn uv16(value: f32) -> u16 {
    if !value.is_finite() {
        return 0;
    }
    (value.clamp(0.0, 1.0) * 65_535.0).round() as u16
}

/// uv16 back to a unit float.
#[must_use]
pub fn uv16_to_f32(raw: u16) -> f32 {
    f32::from(raw) / 65_535.0
}

/// Pack one vertex. `scale` is the mesh's local half-extent per axis (the
/// largest `|coordinate|` of the mesh), so the position quantises without
/// clipping and the draw multiplies back by the same triple.
#[must_use]
pub fn pack_vertex(
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
    tangent_w: f32,
    material_kind: u8,
    scale: [f32; 3],
) -> VertexPacked {
    let w = if tangent_w < 0.0 { 0u32 } else { 1u32 };
    VertexPacked {
        pos: [
            snorm16(position[0], scale[0]),
            snorm16(position[1], scale[1]),
            snorm16(position[2], scale[2]),
            32_767,
        ],
        n: pack_oct(oct_encode(normal)),
        uv: [uv16(uv[0]), uv16(uv[1])],
        extra: w | (u32::from(material_kind) << 8),
    }
}

/// Decode one packed position back to metres.
#[must_use]
pub fn unpack_position(packed: &VertexPacked, scale: [f32; 3]) -> [f32; 3] {
    [
        snorm16_to_f32(packed.pos[0]) * scale[0],
        snorm16_to_f32(packed.pos[1]) * scale[1],
        snorm16_to_f32(packed.pos[2]) * scale[2],
    ]
}

/// Decode one packed normal back to a unit direction.
#[must_use]
pub fn unpack_normal(packed: &VertexPacked) -> [f32; 3] {
    oct_decode(unpack_oct(packed.n))
}

/// Decode one packed uv back to `0..=1`.
#[must_use]
pub fn unpack_uv(packed: &VertexPacked) -> [f32; 2] {
    [uv16_to_f32(packed.uv[0]), uv16_to_f32(packed.uv[1])]
}

/// Per-draw decode scale for a mesh: the largest absolute coordinate per axis,
/// at least 1 mm so a flat mesh never divides by zero.
#[must_use]
pub fn mesh_scale(positions: &[[f32; 3]]) -> [f32; 3] {
    let mut scale = [1e-3f32; 3];
    for point in positions {
        for (axis, value) in point.iter().enumerate() {
            scale[axis] = scale[axis].max(value.abs());
        }
    }
    scale
}

/// A whole mesh packed for the potato lane: one per-draw scale plus one packed
/// vertex per input vertex, in the input's vertex order.
#[derive(Clone, Debug, PartialEq)]
pub struct PackedMesh {
    /// Per-draw decode scale, multiply the snorm16 positions back by this.
    pub scale: [f32; 3],
    /// Packed vertices, input order.
    pub verts: Vec<VertexPacked>,
}

impl PackedMesh {
    /// Bytes one upload of this mesh costs.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.verts.len() * VERTEX_STRIDE_BYTES
    }
}

/// Pack every vertex of one mesh. The scale comes from the mesh itself, so two
/// different meshes never share a decode scale by accident, and the same mesh
/// always packs to the same bytes.
#[must_use]
pub fn pack_mesh(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uv0: &[[f32; 2]],
    tangent_w: f32,
    material_kind: u8,
) -> PackedMesh {
    let scale = mesh_scale(positions);
    let mut verts = Vec::with_capacity(positions.len());
    for (index, point) in positions.iter().enumerate() {
        let normal = normals.get(index).copied().unwrap_or([0.0, 0.0, 1.0]);
        let uv = uv0.get(index).copied().unwrap_or([0.0, 0.0]);
        verts.push(pack_vertex(
            *point,
            normal,
            uv,
            tangent_w,
            material_kind,
            scale,
        ));
    }
    PackedMesh { scale, verts }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fighter-scale body: a 1.8 m tall, 0.5 m wide capsule shell, the extents
    /// the arena kit fits.
    fn fighter() -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>) {
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut uv0 = Vec::new();
        for ring in 0..12 {
            let v = ring as f32 / 11.0;
            let y = -0.9 + 1.8 * v;
            for segment in 0..16 {
                let u = segment as f32 / 16.0;
                let angle = u * std::f32::consts::TAU;
                let (sin, cos) = angle.sin_cos();
                let radius = if v < 0.1 || v > 0.9 { 0.12 } else { 0.25 };
                positions.push([radius * cos, y, radius * sin]);
                normals.push([cos, 0.0, sin]);
                uv0.push([u, v]);
            }
        }
        (positions, normals, uv0)
    }

    #[test]
    fn packed_vertex_round_trips_within_a_third_of_a_millimetre() {
        let (positions, normals, uv0) = fighter();
        let scale = mesh_scale(&positions);
        let mut worst = 0.0f32;
        for (index, point) in positions.iter().enumerate() {
            let packed = pack_vertex(*point, normals[index], uv0[index], 1.0, 0, scale);
            let back = unpack_position(&packed, scale);
            for axis in 0..3 {
                worst = worst.max((back[axis] - point[axis]).abs());
            }
        }
        assert!(
            worst < 1.0 / 3000.0,
            "position drifted {worst} m, over a third of a millimetre"
        );
    }

    #[test]
    fn packed_vertex_is_twenty_bytes_and_saves_fifty_eight_percent() {
        assert_eq!(VertexPacked::stride_bytes(), VERTEX_STRIDE_BYTES);
        assert_eq!(VERTEX_STRIDE_BYTES, 20);
        assert_eq!(std::mem::size_of::<VertexPacked>(), 20);
        let saved = F32_VERTEX_STRIDE_BYTES - VERTEX_STRIDE_BYTES;
        assert_eq!(saved * 100 / F32_VERTEX_STRIDE_BYTES, 58, "48 -> 20 bytes");
    }

    #[test]
    fn packed_normal_round_trips_within_point_two_degrees() {
        let mut worst = 0.0f32;
        for ring in 0..64 {
            let polar = std::f32::consts::PI * (ring as f32 + 0.5) / 64.0;
            for segment in 0..64 {
                let azimuth = std::f32::consts::TAU * (segment as f32 + 0.5) / 64.0;
                let n = [
                    polar.sin() * azimuth.cos(),
                    polar.cos(),
                    polar.sin() * azimuth.sin(),
                ];
                let back = oct_decode(oct_encode(n));
                let dot = (n[0] * back[0] + n[1] * back[1] + n[2] * back[2]).clamp(-1.0, 1.0);
                worst = worst.max(dot.acos().to_degrees());
            }
        }
        assert!(
            worst <= 0.2,
            "the normal drifted {worst} deg, over a fifth of a degree"
        );
    }

    #[test]
    fn packed_uv_round_trips_to_one_16_bit_texel() {
        for step in 0..=64u16 {
            let u = f32::from(step) / 64.0;
            let back = uv16_to_f32(uv16(u));
            assert!(
                (back - u).abs() <= 1.0 / 65_535.0,
                "uv {u} came back as {back}"
            );
        }
    }

    #[test]
    fn packed_mesh_round_trips_and_repeats_its_bytes() {
        let (positions, normals, uv0) = fighter();
        let first = pack_mesh(&positions, &normals, &uv0, 1.0, 0);
        assert_eq!(first.verts.len(), positions.len(), "one vertex per input");
        assert_eq!(first.bytes(), positions.len() * VERTEX_STRIDE_BYTES);
        assert!(first.bytes() < positions.len() * F32_VERTEX_STRIDE_BYTES);
        let mut worst = 0.0f32;
        for (index, point) in positions.iter().enumerate() {
            let back = unpack_position(&first.verts[index], first.scale);
            for axis in 0..3 {
                worst = worst.max((back[axis] - point[axis]).abs());
            }
            let n = unpack_normal(&first.verts[index]);
            let dot =
                (n[0] * normals[index][0] + n[1] * normals[index][1] + n[2] * normals[index][2])
                    .clamp(-1.0, 1.0);
            assert!(dot.acos().to_degrees() <= 0.2, "normal at {index} drifted");
        }
        assert!(worst < 1.0 / 3000.0, "the mesh drifted {worst} m");
        let second = pack_mesh(&positions, &normals, &uv0, 1.0, 0);
        assert_eq!(first, second, "the same mesh packs to the same bytes x2");
    }
}
