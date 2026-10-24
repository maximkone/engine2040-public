//! forward_bind: EG12 slice b staged torch uniform array.
//!
//! Owner: render lane; staged consumer: the forward pipeline slice 2 (binds
//! the staged chunk at group 0 bindings 16-19, free in `forward.wgsl`
//! today). Units: meters for torch pos and range, linear radiance for color.
//! Determinism: fixed slot order; same inputs give same bytes. Normal and
//! ORM sampling hooks stay in the chunk for slice c; the live bind of these
//! bytes into the frame lands after this slice. The map-hook mirrors (the
//! error type, ORM split, normal decode and world normal) compile under
//! `cfg(test)` only: they pin the shader terms until the bind slice calls
//! them, then they lose the gate.

#[cfg(test)]
use std::fmt;

use crate::torch_light::TorchError;

impl crate::present::Presenter {
    /// Stage the frame's torch block into the forward uniform array (EG12
    /// slice b): the capture twin writes it inside `stage_frame`, the live and
    /// probe frames write it here beside the other per-frame uniforms, so the
    /// zero-allocation staging gate still measures the pack alone. One write,
    /// no heap. Presentation only.
    pub(super) fn write_torch_block(
        &self,
        list: &crate::present::DrawList,
    ) -> Result<(), crate::present::PresentError> {
        let torch = self.torch_bytes_from_scene(list)?;
        self.queue.write_buffer(&self.torch_buf, 0, &torch);
        Ok(())
    }
}

/// Staged EG12 WGSL chunk: torch uniform array plus map hooks.
#[cfg(test)]
pub(crate) const FORWARD_MAPS_WGSL: &str = include_str!("../../shaders/forward_maps.wgsl");

/// Markers the staged chunk must contain: the torch block plus its fill,
/// read-back and shade helpers. The slice-2 pipeline bind gates on these.
#[cfg(test)]
pub(crate) const FORWARD_MAPS_MARKERS: [&str; 6] = [
    "Eg12Torch",
    "Eg12TorchBlock",
    "eg12_torch_falloff",
    "eg12_torch_sum",
    "eg12_make_torch",
    "eg12_torch_at",
];

/// Most torch slots per frame; matches `Eg12TorchBlock` in the chunk.
pub(crate) const TORCH_MAX: usize = 8;
/// Byte image of one torch block: 8 slots of 32 B plus the live count.
pub(crate) const TORCH_BLOCK_BYTES: usize = 272;
/// West torch head in meters, mirrors the placement actors.
const TORCH_WEST_M: [f32; 3] = [20.0, 1.0, -3.0];
/// East torch head in meters, mirrors the placement actors.
const TORCH_EAST_M: [f32; 3] = [20.0, 1.0, 3.0];
/// Fixed warm linear color for both torches.
const TORCH_COLOR: [f32; 3] = [1.0, 0.55, 0.22];
/// Finite reach in meters; shade is zero at and past the edge.
const TORCH_RANGE_M: f32 = 12.0;

/// Staged bind failures: every refusal names its cause.
#[cfg(test)]
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum ForwardMapsError {
    /// Caller input is unusable; reason carried.
    Invalid(String),
    /// An index addresses no slot; the wild index carried.
    BadIndex(u32),
}

#[cfg(test)]
impl fmt::Display for ForwardMapsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(e) => write!(f, "forward maps: {e}"),
            Self::BadIndex(i) => write!(f, "forward maps torch slot {i} is out of range"),
        }
    }
}

#[cfg(test)]
impl std::error::Error for ForwardMapsError {}

/// Assert the staged chunk carries every torch marker. The window-only
/// wiring gate calls this before the slice-2 layout lands.
#[cfg(test)]
pub(crate) fn assert_forward_maps_chunk(chunk: &str) -> Result<(), ForwardMapsError> {
    for marker in FORWARD_MAPS_MARKERS {
        if !chunk.contains(marker) {
            return Err(ForwardMapsError::Invalid(format!(
                "forward maps chunk misses marker '{marker}'"
            )));
        }
    }
    Ok(())
}

/// One torch point light: world pos in meters, warm linear color, finite
/// range in meters. Mirrors `Eg12Torch` in the chunk field for field.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct TorchSlot {
    /// World position in meters.
    pub(crate) pos: [f32; 3],
    /// Linear RGB color, components `>= 0`.
    pub(crate) color: [f32; 3],
    /// Influence radius in meters, `> 0` when lit.
    pub(crate) range: f32,
}

impl TorchSlot {
    /// Dark slot: zeroed, never lit.
    const OFF: Self = Self {
        pos: [0.0; 3],
        color: [0.0; 3],
        range: 0.0,
    };

    /// Check finiteness and ranges before the slot reaches the frame.
    #[cfg(test)]
    fn validate(&self) -> Result<(), ForwardMapsError> {
        if !self.pos.iter().all(|c| c.is_finite()) {
            return Err(ForwardMapsError::Invalid(
                "torch position is not finite".into(),
            ));
        }
        if !self.color.iter().all(|c| c.is_finite() && *c >= 0.0) {
            return Err(ForwardMapsError::Invalid(
                "torch color is not finite non-negative".into(),
            ));
        }
        if !self.range.is_finite() || self.range < 0.0 {
            return Err(ForwardMapsError::Invalid(format!(
                "torch range {} is not finite non-negative",
                self.range
            )));
        }
        Ok(())
    }
}

/// One frame of torch point lights: up to `TORCH_MAX` slots, live count
/// first, rest zeroed. Mirrors `Eg12TorchBlock` in the chunk.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct TorchFrame {
    slots: [TorchSlot; TORCH_MAX],
    count: u32,
}

impl TorchFrame {
    /// Build the frame block: two warm torches when draws exist, else
    /// zeroed. Pure with fixed order.
    pub(crate) fn for_draws(has_draws: bool) -> Self {
        if !has_draws {
            return Self {
                slots: [TorchSlot::OFF; TORCH_MAX],
                count: 0,
            };
        }
        let mut slots = [TorchSlot::OFF; TORCH_MAX];
        slots[0] = TorchSlot {
            pos: TORCH_WEST_M,
            color: TORCH_COLOR,
            range: TORCH_RANGE_M,
        };
        slots[1] = TorchSlot {
            pos: TORCH_EAST_M,
            color: TORCH_COLOR,
            range: TORCH_RANGE_M,
        };
        Self { slots, count: 2 }
    }

    /// Build the frame block from the scene's torch list (VS7): the
    /// placement actors become lights, in the order the scene gave them.
    /// An empty list with draws falls back to the arena's built-in pair,
    /// so a scene that feeds none still lights the arena. Refuses more
    /// than `TORCH_MAX` lights and any light that breaks the rules.
    ///
    /// The refusal is the crate's own [`TorchError`]: the staged
    /// `ForwardMapsError` is a test-only mirror of the shader terms and is
    /// not compiled into the window build that calls this.
    pub(crate) fn from_scene(
        has_draws: bool,
        lights: &[crate::torch_light::TorchLight],
    ) -> Result<Self, TorchError> {
        if !has_draws {
            return Ok(Self {
                slots: [TorchSlot::OFF; TORCH_MAX],
                count: 0,
            });
        }
        if lights.is_empty() {
            return Ok(Self::for_draws(true));
        }
        if lights.len() > TORCH_MAX {
            return Err(TorchError::TooMany(lights.len()));
        }
        let mut slots = [TorchSlot::OFF; TORCH_MAX];
        for (i, light) in lights.iter().enumerate() {
            light.validate()?;
            slots[i] = TorchSlot {
                pos: light.pos,
                color: light.color,
                range: light.range_m,
            };
        }
        let count = u32::try_from(lights.len())
            .map_err(|_| TorchError::Invalid("torch count exceeds u32".into()))?;
        Ok(Self { slots, count })
    }

    /// Build a frame from an explicit slot list: the first `slots.len()`
    /// entries burn, the rest stay dark. Refuses more than `TORCH_MAX`
    /// entries and any non-finite slot. Sizes nothing.
    #[cfg(test)]
    pub(crate) fn from_list(slots: &[TorchSlot]) -> Result<Self, ForwardMapsError> {
        if slots.len() > TORCH_MAX {
            return Err(ForwardMapsError::Invalid(format!(
                "torch list holds {} slots, at most {TORCH_MAX}",
                slots.len()
            )));
        }
        for slot in slots {
            slot.validate()?;
        }
        let mut packed = [TorchSlot::OFF; TORCH_MAX];
        for (i, slot) in slots.iter().enumerate() {
            packed[i] = *slot;
        }
        let count = u32::try_from(slots.len())
            .map_err(|_| ForwardMapsError::Invalid("torch count exceeds u32".into()))?;
        Ok(Self {
            slots: packed,
            count,
        })
    }

    /// Live torch count: two when draws exist, zero for clears.
    #[cfg(test)]
    pub(crate) fn count(&self) -> u32 {
        self.count
    }

    /// Borrow slot `index`; refuses past the live count.
    #[cfg(test)]
    pub(crate) fn slot(&self, index: usize) -> Result<TorchSlot, ForwardMapsError> {
        if index >= TORCH_MAX {
            return Err(ForwardMapsError::BadIndex(
                u32::try_from(index).unwrap_or(u32::MAX),
            ));
        }
        let count = usize::try_from(self.count)
            .map_err(|_| ForwardMapsError::Invalid("torch count exceeds usize".into()))?;
        if index >= count {
            return Err(ForwardMapsError::Invalid(format!(
                "torch slot {index} past live count {}",
                self.count
            )));
        }
        Ok(self.slots[index])
    }

    /// Little-endian block image ([`TORCH_BLOCK_BYTES`] bytes) into
    /// caller-owned storage: slot `i` packs pos at `i * 32`, color at
    /// `i * 32 + 16`, range at `i * 32 + 28`, count at byte 256. Slices
    /// in, slices out: steady-state frames pack into a reused scratch and
    /// grow nothing. Mirrors the `Eg12TorchBlock` layout term for term.
    pub(crate) fn pack_into(&self, out: &mut [u8; TORCH_BLOCK_BYTES]) {
        for (i, slot) in self.slots.iter().enumerate() {
            let base = i * 32;
            for (j, c) in slot.pos.iter().enumerate() {
                let at = base + j * 4;
                out[at..at + 4].copy_from_slice(&c.to_le_bytes());
            }
            for (j, c) in slot.color.iter().enumerate() {
                let at = base + 16 + j * 4;
                out[at..at + 4].copy_from_slice(&c.to_le_bytes());
            }
            let at = base + 28;
            out[at..at + 4].copy_from_slice(&slot.range.to_le_bytes());
        }
        out[256..260].copy_from_slice(&self.count.to_le_bytes());
    }

    /// Read slot `index` back from a packed block image: the inverse of
    /// [`TorchFrame::pack_into`]. Refuses wild indices and corrupt counts
    /// loudly; validates the decoded slot before returning it. Sizes nothing.
    #[cfg(test)]
    pub(crate) fn unpack_slot(
        bytes: &[u8; TORCH_BLOCK_BYTES],
        index: usize,
    ) -> Result<TorchSlot, ForwardMapsError> {
        if index >= TORCH_MAX {
            return Err(ForwardMapsError::BadIndex(
                u32::try_from(index).unwrap_or(u32::MAX),
            ));
        }
        let count = u32::from_le_bytes(
            bytes[256..260]
                .try_into()
                .map_err(|_| ForwardMapsError::Invalid("torch block count is corrupt".into()))?,
        );
        let live = usize::try_from(count)
            .map_err(|_| ForwardMapsError::Invalid("torch count exceeds usize".into()))?;
        if index >= live {
            return Err(ForwardMapsError::Invalid(format!(
                "torch slot {index} past live count {count}"
            )));
        }
        let base = index * 32;
        let mut pos = [0.0f32; 3];
        let mut color = [0.0f32; 3];
        for (j, c) in pos.iter_mut().enumerate() {
            let at = base + j * 4;
            let lane: [u8; 4] = bytes[at..at + 4]
                .try_into()
                .map_err(|_| ForwardMapsError::Invalid("torch block pos is corrupt".into()))?;
            *c = f32::from_le_bytes(lane);
        }
        for (j, c) in color.iter_mut().enumerate() {
            let at = base + 16 + j * 4;
            let lane: [u8; 4] = bytes[at..at + 4]
                .try_into()
                .map_err(|_| ForwardMapsError::Invalid("torch block color is corrupt".into()))?;
            *c = f32::from_le_bytes(lane);
        }
        let lane: [u8; 4] = bytes[base + 28..base + 32]
            .try_into()
            .map_err(|_| ForwardMapsError::Invalid("torch block range is corrupt".into()))?;
        let slot = TorchSlot {
            pos,
            color,
            range: f32::from_le_bytes(lane),
        };
        slot.validate()?;
        Ok(slot)
    }
}

/// Packed ORM split in 0..=1: occlusion, perceptual roughness, metalness.
/// Mirrors `Eg12Orm` in the staged chunk field for field. Units: unitless
/// ratios. Determinism: same texel gives same split.
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct OrmSplit {
    /// Ambient opening, 1 open and 0 shut.
    pub(crate) occlusion: f32,
    /// Perceptual roughness, 0 mirror and 1 matte.
    pub(crate) roughness: f32,
    /// Metalness, 0 dielectric and 1 metal.
    pub(crate) metallic: f32,
}

/// Split one ORM texel with R occlusion, G roughness and B metallic,
/// clamping each lane 0..=1 like `eg12_split_orm`. Refuses non-finite
/// texels. Allocates nothing, no clock, deterministic.
#[cfg(test)]
pub(crate) fn split_orm(texel: [f32; 3]) -> Result<OrmSplit, ForwardMapsError> {
    if !texel.iter().all(|c| c.is_finite()) {
        return Err(ForwardMapsError::Invalid("orm texel is not finite".into()));
    }
    Ok(OrmSplit {
        occlusion: texel[0].clamp(0.0, 1.0),
        roughness: texel[1].clamp(0.0, 1.0),
        metallic: texel[2].clamp(0.0, 1.0),
    })
}

/// Normalize or keep the input when degenerate so no NaN escapes.
#[cfg(test)]
fn normalize_or_input(v: [f32; 3]) -> Result<[f32; 3], ForwardMapsError> {
    let l2 = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    if l2 < 1e-12 {
        return Ok(v);
    }
    let len = l2.sqrt();
    Ok([v[0] / len, v[1] / len, v[2] / len])
}

/// Decode a tangent-space normal from a 0..=1 texel to -1..=1 with a
/// guarded normalize like `eg12_decode_normal`: a zero-length decode
/// falls back to the geometric normal. Refuses non-finite inputs.
/// Allocates nothing, no clock, deterministic.
#[cfg(test)]
pub(crate) fn decode_normal(
    texel: [f32; 3],
    geom_n: [f32; 3],
) -> Result<[f32; 3], ForwardMapsError> {
    if !texel.iter().all(|c| c.is_finite()) {
        return Err(ForwardMapsError::Invalid(
            "normal texel is not finite".into(),
        ));
    }
    if !geom_n.iter().all(|c| c.is_finite()) {
        return Err(ForwardMapsError::Invalid(
            "geometric normal is not finite".into(),
        ));
    }
    let v = [
        texel[0] * 2.0 - 1.0,
        texel[1] * 2.0 - 1.0,
        texel[2] * 2.0 - 1.0,
    ];
    let l2 = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    if l2 < 1e-12 {
        return normalize_or_input(geom_n);
    }
    let len = l2.sqrt();
    Ok([v[0] / len, v[1] / len, v[2] / len])
}

/// Map a tangent-space normal to world with an orthonormalized frame like
/// `eg12_world_normal`: tangent orthonormalized against the geometric
/// normal, handedness from `tangent.w`, bitangent `w * (n x t)`.
/// Degenerate tangents or outputs keep `n`. Refuses non-finite inputs.
/// Allocates nothing, no clock, deterministic.
#[cfg(test)]
pub(crate) fn world_normal(
    geom_n: [f32; 3],
    tangent: [f32; 4],
    map_n: [f32; 3],
) -> Result<[f32; 3], ForwardMapsError> {
    if !geom_n.iter().all(|c| c.is_finite()) {
        return Err(ForwardMapsError::Invalid(
            "geometric normal is not finite".into(),
        ));
    }
    if !tangent.iter().all(|c| c.is_finite()) {
        return Err(ForwardMapsError::Invalid("tangent is not finite".into()));
    }
    if !map_n.iter().all(|c| c.is_finite()) {
        return Err(ForwardMapsError::Invalid("map normal is not finite".into()));
    }
    let n = normalize_or_input(geom_n)?;
    let d = tangent[0] * n[0] + tangent[1] * n[1] + tangent[2] * n[2];
    let tx = [
        tangent[0] - n[0] * d,
        tangent[1] - n[1] * d,
        tangent[2] - n[2] * d,
    ];
    let tl2 = tx[0] * tx[0] + tx[1] * tx[1] + tx[2] * tx[2];
    if tl2 < 1e-12 {
        return Ok(n);
    }
    let tlen = tl2.sqrt();
    let t = [tx[0] / tlen, tx[1] / tlen, tx[2] / tlen];
    let w = if tangent[3] >= 0.0 { 1.0 } else { -1.0 };
    let b = [
        w * (n[1] * t[2] - n[2] * t[1]),
        w * (n[2] * t[0] - n[0] * t[2]),
        w * (n[0] * t[1] - n[1] * t[0]),
    ];
    let out = [
        t[0] * map_n[0] + b[0] * map_n[1] + n[0] * map_n[2],
        t[1] * map_n[0] + b[1] * map_n[1] + n[1] * map_n[2],
        t[2] * map_n[0] + b[2] * map_n[1] + n[2] * map_n[2],
    ];
    let ol2 = out[0] * out[0] + out[1] * out[1] + out[2] * out[2];
    if ol2 < 1e-12 {
        return Ok(n);
    }
    let olen = ol2.sqrt();
    Ok([out[0] / olen, out[1] / olen, out[2] / olen])
}

#[cfg(test)]
mod forward_maps_tests {
    use super::{
        assert_forward_maps_chunk, TorchFrame, TorchSlot, FORWARD_MAPS_MARKERS, FORWARD_MAPS_WGSL,
        TORCH_BLOCK_BYTES, TORCH_MAX,
    };

    #[test]
    fn chunk_carries_torch_markers() {
        assert_forward_maps_chunk(FORWARD_MAPS_WGSL).expect("staged chunk is whole");
        for marker in FORWARD_MAPS_MARKERS {
            assert!(FORWARD_MAPS_WGSL.contains(marker), "chunk names {marker}");
        }
    }

    #[test]
    fn torch_pack_matches_the_shader_block() {
        let frame = TorchFrame::for_draws(true);
        assert_eq!(frame.count(), 2, "two placement torches burn");
        let mut out = [0u8; TORCH_BLOCK_BYTES];
        frame.pack_into(&mut out);
        let x = f32::from_le_bytes(out[0..4].try_into().expect("slot0 x"));
        assert!((x - 20.0).abs() < 1e-6, "slot 0 sits at the west head");
        let range = f32::from_le_bytes(out[28..32].try_into().expect("slot0 range"));
        assert!((range - 12.0).abs() < 1e-6, "slot 0 reaches 12 m");
        let count = u32::from_le_bytes(out[256..260].try_into().expect("count"));
        assert_eq!(count, 2, "count follows the slots");
        let clear = TorchFrame::for_draws(false);
        let mut zero = [0u8; TORCH_BLOCK_BYTES];
        clear.pack_into(&mut zero);
        assert_eq!(
            zero, [0u8; TORCH_BLOCK_BYTES],
            "clears stage zeroed torches"
        );
    }

    #[test]
    fn packs_eight_torches_and_reads_one_back() {
        let slots = [
            TorchSlot {
                pos: [10.0, 1.0, -7.0],
                color: [1.0, 0.5, 0.2],
                range: 9.0,
            },
            TorchSlot {
                pos: [11.0, 1.5, -6.0],
                color: [0.9, 0.5, 0.25],
                range: 9.5,
            },
            TorchSlot {
                pos: [12.0, 2.0, -5.0],
                color: [0.8, 0.55, 0.3],
                range: 10.0,
            },
            TorchSlot {
                pos: [13.0, 2.5, -4.0],
                color: [0.7, 0.6, 0.35],
                range: 10.5,
            },
            TorchSlot {
                pos: [14.0, 3.0, -3.0],
                color: [0.6, 0.65, 0.4],
                range: 11.0,
            },
            TorchSlot {
                pos: [15.0, 3.5, -2.0],
                color: [1.0, 0.4, 0.15],
                range: 11.5,
            },
            TorchSlot {
                pos: [16.0, 4.0, -1.0],
                color: [0.95, 0.45, 0.2],
                range: 12.0,
            },
            TorchSlot {
                pos: [17.0, 4.5, 0.0],
                color: [0.85, 0.5, 0.25],
                range: 12.5,
            },
        ];
        assert_eq!(slots.len(), TORCH_MAX, "eight torch actors fill the array");
        let frame = TorchFrame::from_list(&slots).expect("eight torches build");
        assert_eq!(frame.count(), 8, "all eight slots burn");
        let mut bytes = [0u8; TORCH_BLOCK_BYTES];
        frame.pack_into(&mut bytes);
        let back = TorchFrame::unpack_slot(&bytes, 5).expect("slot 5 reads back");
        assert!((back.pos[0] - 15.0).abs() < 1e-6, "slot 5 x reads back");
        assert!((back.pos[1] - 3.5).abs() < 1e-6, "slot 5 y reads back");
        assert!((back.pos[2] + 2.0).abs() < 1e-6, "slot 5 z reads back");
        assert!((back.color[0] - 1.0).abs() < 1e-6, "slot 5 red reads back");
        assert!((back.range - 11.5).abs() < 1e-6, "slot 5 range reads back");
    }

    #[test]
    fn pack_into_allocates_nothing() {
        let frame = TorchFrame::for_draws(true);
        let ((), heap) = engine_core::alloc_probe::measure(|| {
            let mut out = [0u8; TORCH_BLOCK_BYTES];
            frame.pack_into(&mut out);
            core::hint::black_box(out);
        });
        assert_eq!(
            (heap.calls, heap.bytes),
            (0, 0),
            "per-frame torch pack sizes nothing"
        );
    }

    #[test]
    fn orm_split_clamps_and_splits() {
        let orm = super::split_orm([0.25, 0.5, 0.75]).expect("finite texel splits");
        assert!((orm.occlusion - 0.25).abs() < 1e-6, "R carries occlusion");
        assert!((orm.roughness - 0.5).abs() < 1e-6, "G carries roughness");
        assert!((orm.metallic - 0.75).abs() < 1e-6, "B carries metallic");
        let cut = super::split_orm([-0.5, 0.5, 1.5]).expect("lanes clamp");
        assert!((cut.occlusion - 0.0).abs() < 1e-6, "R clamps low");
        assert!((cut.roughness - 0.5).abs() < 1e-6, "G passes through");
        assert!((cut.metallic - 1.0).abs() < 1e-6, "B clamps high");
        assert!(
            super::split_orm([f32::NAN, 0.0, 0.0]).is_err(),
            "NaN refuses"
        );
    }

    #[test]
    fn decode_normal_guards_zero_texel() {
        let geom = [0.0, 0.0, 1.0];
        let flat = super::decode_normal([0.5, 0.5, 0.5], geom).expect("flat decodes");
        assert!((flat[0] - geom[0]).abs() < 1e-6, "flat keeps x");
        assert!((flat[1] - geom[1]).abs() < 1e-6, "flat keeps y");
        assert!((flat[2] - geom[2]).abs() < 1e-6, "flat keeps z");
        let up = super::decode_normal([0.5, 0.5, 1.0], geom).expect("up decodes");
        let len = (up[0] * up[0] + up[1] * up[1] + up[2] * up[2]).sqrt();
        assert!((len - 1.0).abs() < 1e-6, "unit texel normalizes");
        assert!((up[2] - 1.0).abs() < 1e-6, "up stays up");
        assert!(
            super::decode_normal([f32::INFINITY, 0.5, 0.5], geom).is_err(),
            "inf refuses"
        );
    }

    #[test]
    fn world_normal_orthonormalizes_and_keeps_degenerate() {
        let n = [0.0, 0.0, 1.0];
        let keep = super::world_normal(n, [1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0])
            .expect("aligned frame maps");
        assert!((keep[2] - 1.0).abs() < 1e-6, "aligned keeps normal");
        let pos = super::world_normal(n, [1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0])
            .expect("positive handedness maps");
        let neg = super::world_normal(n, [1.0, 0.0, 0.0, -1.0], [0.0, 1.0, 0.0])
            .expect("negative handedness maps");
        assert!((pos[1] - 1.0).abs() < 1e-6, "w >= 0 keeps bitangent");
        assert!((neg[1] + 1.0).abs() < 1e-6, "w < 0 flips bitangent");
        let deg = super::world_normal(n, [0.0, 0.0, 1.0, 1.0], [0.0, 0.0, 1.0])
            .expect("degenerate keeps geom");
        assert!((deg[2] - 1.0).abs() < 1e-6, "degenerate keeps geom_n");
        let len = (pos[0] * pos[0] + pos[1] * pos[1] + pos[2] * pos[2]).sqrt();
        assert!((len - 1.0).abs() < 1e-6, "output is unit");
    }

    #[test]
    fn normal_map_on_off_differs_in_masked_region() {
        // EG12(c) pixel test: 4x4 raster, center 2x2 mask, perturbed
        // world normal vs the geometric normal under one diagonal light.
        // Fails when the map is ignored (gap 0), passes with live
        // `decode_normal` plus `world_normal`.
        let geom = [0.0, 0.0, 1.0];
        let tangent = [1.0, 0.0, 0.0, 1.0];
        // Tilted texel decodes to [~0.447, 0, ~0.894], 26 deg off geom.
        let texel = [0.75, 0.5, 1.0];
        let albedo = [0.5, 0.5, 0.5];
        let inv = core::f32::consts::FRAC_1_SQRT_2;
        let light = [inv, 0.0, inv];
        let diffuse = |n: [f32; 3]| {
            let d = (n[0] * light[0] + n[1] * light[1] + n[2] * light[2]).max(0.0);
            [
                albedo[0] * d / core::f32::consts::PI,
                albedo[1] * d / core::f32::consts::PI,
                albedo[2] * d / core::f32::consts::PI,
            ]
        };
        let off = diffuse(geom);
        let mut max_gap = 0.0f32;
        let mut masked = 0u32;
        for y in 0..4u32 {
            for x in 0..4u32 {
                let in_mask = (1..3).contains(&x) && (1..3).contains(&y);
                if !in_mask {
                    continue;
                }
                masked += 1;
                let map_n = super::decode_normal(texel, geom).expect("tilted texel decodes");
                let world = super::world_normal(geom, tangent, map_n).expect("frame maps");
                let on = diffuse(world);
                for c in 0..3 {
                    max_gap = max_gap.max((on[c] - off[c]).abs());
                }
            }
        }
        assert_eq!(masked, 4, "mask covers the center 2x2");
        assert!(
            max_gap > 0.002,
            "masked normal map moves above geometric, got {max_gap}"
        );
        let mut again = 0.0f32;
        for y in 1..3u32 {
            for x in 1..3u32 {
                let _ = (x, y);
                let map_n = super::decode_normal(texel, geom).expect("tilted texel decodes");
                let world = super::world_normal(geom, tangent, map_n).expect("frame maps");
                let on = diffuse(world);
                for c in 0..3 {
                    again = again.max((on[c] - off[c]).abs());
                }
            }
        }
        assert_eq!(max_gap, again, "masked gap is deterministic");
        let map_n = super::decode_normal(texel, geom).expect("tilted texel decodes");
        let deg =
            super::world_normal(geom, [0.0, 0.0, 1.0, 1.0], map_n).expect("degenerate keeps geom");
        for c in 0..3 {
            assert!(
                (deg[c] - geom[c]).abs() < 1e-6,
                "degenerate tangent keeps geometric, got {deg:?}"
            );
        }
    }

    #[test]
    fn torch_block_binds_live_count_and_one_slot() {
        assert!(
            FORWARD_MAPS_WGSL.contains("eg12_torches"),
            "staged chunk binds the torch uniform"
        );
        assert!(
            FORWARD_MAPS_WGSL.contains("@binding(19)"),
            "torch uniform lives at 19, last of 16-19"
        );
        let frame = TorchFrame::for_draws(true);
        assert_eq!(frame.count(), 2, "two placement torches burn");
        let mut bytes = [0u8; TORCH_BLOCK_BYTES];
        frame.pack_into(&mut bytes);
        let count = u32::from_le_bytes(bytes[256..260].try_into().expect("count"));
        assert_eq!(count, 2, "bound count follows the live pair");
        let back = TorchFrame::unpack_slot(&bytes, 1).expect("east slot reads back");
        assert!((back.pos[0] - 20.0).abs() < 1e-6, "east x reads back");
        assert!((back.pos[1] - 1.0).abs() < 1e-6, "east y reads back");
        assert!((back.pos[2] - 3.0).abs() < 1e-6, "east z reads back");
        assert!((back.color[0] - 1.0).abs() < 1e-6, "east red reads back");
        assert!((back.color[1] - 0.55).abs() < 1e-6, "east green reads back");
        assert!((back.range - 12.0).abs() < 1e-6, "east range reads back");
        let albedo = [0.5, 0.5, 0.5];
        let geom_n = [0.0, 1.0, 0.0];
        let falloff = |dist: f32, range: f32| {
            if range <= 0.0 || dist >= range {
                return 0.0;
            }
            let att = 1.0 / (1.0 + dist * dist);
            let edge = 1.0 - dist / range;
            att * edge * edge
        };
        let shade = |world: [f32; 3]| {
            let mut out = [0.0f32; 3];
            for i in 0..2 {
                let slot = frame.slot(i).expect("live slot");
                let dx = slot.pos[0] - world[0];
                let dy = slot.pos[1] - world[1];
                let dz = slot.pos[2] - world[2];
                let dist = (dx * dx + dy * dy + dz * dz).sqrt();
                if dist >= slot.range || slot.range <= 0.0 || dist < 1e-6 {
                    continue;
                }
                let inv = 1.0 / dist.max(0.0001);
                let l = [dx * inv, dy * inv, dz * inv];
                let ndotl =
                    (geom_n[0] * l[0] + geom_n[1] * l[1] + geom_n[2] * l[2]).clamp(0.0, 1.0);
                if ndotl <= 0.0 {
                    continue;
                }
                let f = falloff(dist, slot.range);
                for c in 0..3 {
                    out[c] += albedo[c] * ndotl * slot.color[c] * f;
                }
            }
            out
        };
        let mut max_gap = 0.0f32;
        let mut masked = 0u32;
        for y in 0..4u32 {
            for x in 0..4u32 {
                if !((1..3).contains(&x) && (1..3).contains(&y)) {
                    continue;
                }
                masked += 1;
                let world = [18.0 + x as f32 * 0.5, 0.0, -4.0 + y as f32 * 0.5];
                let on = shade(world);
                for lane in on {
                    max_gap = max_gap.max(lane.abs());
                }
            }
        }
        assert_eq!(masked, 4, "mask covers the center 2x2");
        assert!(
            max_gap > 0.01,
            "masked torch on moves above off, got {max_gap}"
        );
        let clear = TorchFrame::for_draws(false);
        assert_eq!(clear.count(), 0, "clears bind zero torches");
        let mut zero = [0u8; TORCH_BLOCK_BYTES];
        clear.pack_into(&mut zero);
        assert_eq!(zero, [0u8; TORCH_BLOCK_BYTES], "clears stay zeroed");
    }
}
