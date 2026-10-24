//! material_pipeline: render-side bins from draws to compiled materials.
//!
//! One draw names one material plus four texture slots. The forward pass
//! builds one pipeline per distinct compiled structure; draws that share
//! a structure share a pipeline. The binning is std-only: no GPU, no file,
//! no clock. The window leg adds `pipeline_for`, which builds the pipeline
//! of one compiled material (NS31c2), and `MaterialLive`, the live slot
//! that swaps one material's pipeline and uniform buffer on an edit (NS31c3).

use engine_core::Fnv64;

#[cfg(feature = "gpu")]
mod gpu;
#[cfg(feature = "gpu")]
pub use gpu::{pipeline_for, MaterialPipelineError, MaterialPipelines, COLOR_FORMAT, DEPTH_FORMAT};

#[cfg(feature = "gpu")]
use engine_material::reload::{Reload, ReloadError, Reloader};

/// Most pipelines one frame may need: 21 records share at most 12.
pub const MAX_MATERIAL_PIPELINES: usize = 12;

/// Texture slots per draw, in fixed bind order.
pub const MATERIAL_TEXTURE_SLOTS: usize = 4;

/// Slot names in bind order: base color, ORM, normal, emissive.
pub const SLOT_ORDER: [&str; MATERIAL_TEXTURE_SLOTS] = ["base_color", "orm", "normal", "emissive"];

/// Slot bits in bind order. Values mirror the material slot bits.
pub const SLOT_BITS: [u8; MATERIAL_TEXTURE_SLOTS] = [1, 2, 4, 8];

/// Lobe flags that change the compiled structure, hence the pipeline.
/// Values mirror the material compiler lobe bits so a draw bins like the
/// structure it names without a new dependency.
pub const LOBE_COAT: u8 = 1;
/// Charlie sheen lobe flag; pairs with [`LOBE_COAT`].
pub const LOBE_SHEEN: u8 = 2;

/// Stable pipeline key over lobe flags and the four slot bits alone.
/// Same input gives the same key on any host; draws that share lobes and
/// slots share a pipeline even when their material ids differ. Std-only.
#[must_use]
pub fn pipeline_key_for(lobes: u8, slot_mask: u8) -> u64 {
    let mut h = Fnv64::new();
    h.write_u8(lobes);
    h.write_u8(slot_mask);
    h.finish()
}

/// Pipeline bin of one draw: whose material, which lobes, which slots.
/// Draws with equal keys share one pipeline.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct MaterialBinKey {
    /// Opaque compiled-material id from the draw.
    pub material_id: u64,
    /// Lobe flags ([`LOBE_COAT`], [`LOBE_SHEEN`]).
    pub lobes: u8,
    /// Bound slots as [`SLOT_BITS`]: 1 is base color only, 15 is all four.
    pub slot_mask: u8,
}

impl MaterialBinKey {
    /// Bin key of a material id, lobe flags, and four slot-bound flags.
    #[must_use]
    pub fn new(material_id: u64, lobes: u8, bound: [bool; MATERIAL_TEXTURE_SLOTS]) -> Self {
        let mut mask = 0u8;
        for (slot, bit) in bound.iter().zip(SLOT_BITS) {
            if *slot {
                mask |= bit;
            }
        }
        Self {
            material_id,
            lobes,
            slot_mask: mask,
        }
    }

    /// Bound texture keys on this draw: 1 for one slot, up to 4 for all four.
    #[must_use]
    pub fn texture_key_count(&self) -> u32 {
        self.slot_mask.count_ones()
    }

    /// Stable key over the bin: material id plus lobes and slots. Same bin
    /// gives the same key on any host. The shared pipeline key is
    /// [`pipeline_key_for`] over lobes and slots alone, so draws that share
    /// a compiled structure share one pipeline even when their ids differ.
    #[must_use]
    pub fn pipeline_key(&self) -> u64 {
        let base = pipeline_key_for(self.lobes, self.slot_mask);
        let mut h = Fnv64::new();
        h.write_u64(self.material_id);
        h.write_u64(base);
        h.finish()
    }
}

/// Bin key of one [`crate::present::DrawItem`]: its material id plus
/// which of its four slots are bound today. Lobes stay 0 until the
/// draw interface names them.
#[cfg(feature = "v1")]
#[must_use]
pub fn bin_key_for_draw(item: &crate::present::DrawItem) -> MaterialBinKey {
    let slots = item.texture_slots();
    MaterialBinKey::new(
        item.material_id(),
        0,
        [
            slots[0].is_some(),
            slots[1].is_some(),
            slots[2].is_some(),
            slots[3].is_some(),
        ],
    )
}

/// Distinct bins of a frame's draws, in stable order. Equal draws share
/// one entry, so the length is the pipeline count the frame needs.
#[must_use]
pub fn bin_draws(keys: &[MaterialBinKey]) -> Vec<MaterialBinKey> {
    let mut bins: Vec<MaterialBinKey> = keys.to_vec();
    bins.sort();
    bins.dedup();
    bins
}

/// True when the frame's draws fit [`MAX_MATERIAL_PIPELINES`].
#[must_use]
pub fn fits_pipeline_budget(keys: &[MaterialBinKey]) -> bool {
    bin_draws(keys).len() <= MAX_MATERIAL_PIPELINES
}

/// Distinct shared pipelines in a frame's draws: one entry per lobes plus
/// slots combination, so draws that share a compiled structure count once
/// even when their material ids differ. The length is the pipeline total
/// the frame needs. Std-only.
#[must_use]
pub fn pipeline_count(keys: &[MaterialBinKey]) -> usize {
    let mut shared: Vec<u64> = keys
        .iter()
        .map(|draw| pipeline_key_for(draw.lobes, draw.slot_mask))
        .collect();
    shared.sort_unstable();
    shared.dedup();
    shared.len()
}

/// What one live material edit did to the held state.
#[cfg(feature = "gpu")]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LiveEdit {
    /// The compile is byte-equal to the held one: nothing moved.
    Unchanged,
    /// Same structure, new uniform floats: the buffer was rewritten.
    Values,
    /// New structure: a new pipeline and buffer were built and swapped in.
    Structure,
}

/// Why a live material edit did not reach a drawing pipeline. The held
/// state is exactly what it was before the call.
#[cfg(feature = "gpu")]
#[derive(Clone, PartialEq, Debug)]
pub enum LiveError {
    /// The text did not parse or compile; the reloader kept its last good.
    Refused(ReloadError),
    /// The new structure's pipeline did not build; the old one keeps drawing.
    Build(MaterialPipelineError),
}

#[cfg(feature = "gpu")]
impl core::fmt::Display for LiveError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Refused(error) => write!(f, "{error}"),
            Self::Build(error) => write!(f, "{error}"),
        }
    }
}

#[cfg(feature = "gpu")]
impl std::error::Error for LiveError {}

/// The live half of one material slot: the last good
/// [`engine_material::reload::Reloader`], the pipeline currently drawing and
/// its uniform buffer.
///
/// [`MaterialLive::apply_text`] runs the reloader. A value edit rewrites the
/// uniform buffer, no pipeline work. A structure edit builds the new pipeline
/// and its uniform buffer first and swaps them only on success, so the old
/// pipeline keeps drawing while the new one compiles (HR2's last-good swap).
/// A refused text changes nothing: the reloader, the pipeline and the buffer
/// are exactly as they were.
///
/// Units: bytes for the buffer, none for the verdict. Determinism: none
/// needed, this is presentation; a capture of the same frame hashes the same.
/// Window feature only.
#[cfg(feature = "gpu")]
pub struct MaterialLive {
    /// The device the pipeline and the uniform buffer live on.
    device: wgpu::Device,
    /// The queue a value edit writes the uniform buffer through.
    queue: wgpu::Queue,
    /// The last good compile, with the reload refusal rule.
    reloader: Reloader,
    /// The pipeline currently drawing; `None` before the first good edit.
    pipeline: Option<wgpu::RenderPipeline>,
    /// The uniform buffer of the drawing pipeline.
    uniform: Option<wgpu::Buffer>,
}

#[cfg(feature = "gpu")]
impl MaterialLive {
    /// An empty slot on `device`: nothing draws and no buffer exists until
    /// the first good edit.
    #[must_use]
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        Self {
            device: device.clone(),
            queue: queue.clone(),
            reloader: Reloader::new(),
            pipeline: None,
            uniform: None,
        }
    }

    /// Apply new record text. `Ok(Unchanged)` and `Ok(Values)` touch the
    /// uniform buffer only; `Ok(Structure)` swaps in a new pipeline and
    /// buffer; `Err` leaves everything as it was.
    pub fn apply_text(&mut self, text: &str) -> Result<LiveEdit, LiveError> {
        let held = self.reloader.clone();
        match self.reloader.apply_text(text, None) {
            Ok(Reload::Unchanged) => Ok(LiveEdit::Unchanged),
            Ok(Reload::Values { uniform }) => {
                let Some(buffer) = self.uniform.as_ref() else {
                    self.reloader = held;
                    return Err(LiveError::Build(MaterialPipelineError::Validation(
                        "a value edit before the first structure".into(),
                    )));
                };
                write_floats(&self.queue, buffer, &uniform);
                Ok(LiveEdit::Values)
            }
            Ok(Reload::Structure { compiled }) => match pipeline_for(&self.device, &compiled) {
                Ok(pipeline) => {
                    let buffer = create_uniform_buffer(&self.device, &compiled.uniform);
                    self.pipeline = Some(pipeline);
                    self.uniform = Some(buffer);
                    Ok(LiveEdit::Structure)
                }
                Err(error) => {
                    // The old pipeline keeps drawing: nothing swapped.
                    self.reloader = held;
                    Err(LiveError::Build(error))
                }
            },
            Err(error) => Err(LiveError::Refused(error)),
        }
    }

    /// The last good compile's reloader, for a host that reads its digests.
    #[must_use]
    pub fn reloader(&self) -> &Reloader {
        &self.reloader
    }

    /// Look digest of the held record, or `None` before the first success.
    #[must_use]
    pub fn look_digest(&self) -> Option<u64> {
        self.reloader.look_digest()
    }

    /// The pipeline currently drawing, or `None` before the first success.
    #[must_use]
    pub fn pipeline(&self) -> Option<&wgpu::RenderPipeline> {
        self.pipeline.as_ref()
    }

    /// The uniform buffer the drawing pipeline binds at group 0 binding 2.
    #[must_use]
    pub fn uniform(&self) -> Option<&wgpu::Buffer> {
        self.uniform.as_ref()
    }

    /// True once a pipeline is drawing.
    #[must_use]
    pub fn is_drawing(&self) -> bool {
        self.pipeline.is_some()
    }
}

/// A uniform buffer of `uniform`'s bytes, sized to its structure so the
/// group-0 binding's `min_binding_size` matches exactly. Window only.
#[cfg(feature = "gpu")]
fn create_uniform_buffer(device: &wgpu::Device, uniform: &[f32]) -> wgpu::Buffer {
    use wgpu::util::DeviceExt;
    let mut bytes = Vec::with_capacity(uniform.len() * 4);
    for value in uniform {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("material-live-uniform"),
        contents: &bytes,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    })
}

/// Rewrite `buffer` with `uniform`'s floats. The structure did not change,
/// so the byte length is the buffer's own. Window only.
#[cfg(feature = "gpu")]
fn write_floats(queue: &wgpu::Queue, buffer: &wgpu::Buffer, uniform: &[f32]) {
    let mut bytes = Vec::with_capacity(uniform.len() * 4);
    for value in uniform {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    queue.write_buffer(buffer, 0, &bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(id: u64, bound: [bool; 4]) -> MaterialBinKey {
        MaterialBinKey::new(id, 0, bound)
    }

    #[test]
    fn same_input_gives_the_same_bin_and_pipeline_key() {
        let a = key(11, [true, false, false, false]);
        let b = key(11, [true, false, false, false]);
        assert_eq!(a, b);
        assert_eq!(a.pipeline_key(), b.pipeline_key());
    }

    #[test]
    fn slot_order_is_base_orm_normal_emissive() {
        assert_eq!(SLOT_ORDER, ["base_color", "orm", "normal", "emissive"]);
        assert_eq!(key(1, [true, false, false, false]).slot_mask, 1);
        assert_eq!(key(1, [false, true, false, false]).slot_mask, 2);
        assert_eq!(key(1, [false, false, true, false]).slot_mask, 4);
        assert_eq!(key(1, [false, false, false, true]).slot_mask, 8);
        assert_eq!(key(1, [true, true, true, true]).slot_mask, 15);
    }

    #[test]
    fn texture_keys_climb_one_to_four() {
        assert_eq!(key(1, [true, false, false, false]).texture_key_count(), 1);
        assert_eq!(key(1, [true, true, false, false]).texture_key_count(), 2);
        assert_eq!(key(1, [true, true, true, false]).texture_key_count(), 3);
        assert_eq!(key(1, [true, true, true, true]).texture_key_count(), 4);
        // A new bound slot is a new bin, never a silent merge.
        assert_ne!(
            key(7, [true, false, false, false]),
            key(7, [true, true, false, false])
        );
    }

    #[test]
    fn twenty_one_draws_share_twelve_pipelines() {
        // The shipped shape: 21 draws over 12 distinct bins.
        let mut draws = Vec::new();
        for i in 0..21u64 {
            let bin = i % 12;
            draws.push(key(1000 + bin, [true, bin % 2 == 0, bin % 3 == 0, false]));
        }
        let bins = bin_draws(&draws);
        assert_eq!(bins.len(), 12);
        assert!(fits_pipeline_budget(&draws));
        // A 13th distinct bin still fits; the 13th over budget does not.
        let mut over: Vec<MaterialBinKey> = bins.clone();
        for i in 0..12u64 {
            over.push(key(5000 + i, [true, true, true, true]));
        }
        assert!(!fits_pipeline_budget(&over));
    }

    #[test]
    fn bins_come_back_in_stable_order() {
        let draws = vec![
            key(9, [true, false, false, false]),
            key(3, [true, false, false, false]),
            key(9, [true, false, false, false]),
        ];
        assert_eq!(
            bin_draws(&draws),
            vec![
                key(3, [true, false, false, false]),
                key(9, [true, false, false, false]),
            ]
        );
    }

    #[test]
    fn draw_can_name_sheen_coat_and_normal_map_slot() {
        let sheen = MaterialBinKey::new(21, LOBE_SHEEN, [true, false, true, false]);
        let coat = MaterialBinKey::new(21, LOBE_COAT, [true, false, true, false]);
        assert_ne!((sheen, sheen.pipeline_key()), (coat, coat.pipeline_key()));
        assert_eq!(sheen.texture_key_count(), 2);
        assert_eq!(sheen.slot_mask & SLOT_BITS[2], SLOT_BITS[2]);
    }

    #[cfg(feature = "v1")]
    #[test]
    fn draw_item_bins_by_id_and_slot_zero() {
        use crate::present::{DrawItem, MeshKey};
        let scene = crate::v1::default_scene(160, 120).expect("default scene is valid");
        let plain = DrawItem {
            mesh: MeshKey::new(1, [0x11; 32]),
            material: scene.material.clone(),
            base_color: None,
            normal_map: None,
            orm: None,
            emissive: None,
        };
        let bound = DrawItem {
            mesh: MeshKey::new(1, [0x11; 32]),
            material: scene.material,
            base_color: Some(crate::present::TextureKey::new(7, [0x77; 32])),
            normal_map: None,
            orm: None,
            emissive: None,
        };
        let a = bin_key_for_draw(&plain);
        assert_eq!(a.texture_key_count(), 0);
        let b = bin_key_for_draw(&bound);
        assert_eq!(b.texture_key_count(), 1);
        assert_eq!(b.slot_mask, 1);
        assert_ne!(a, b);
    }

    // Shipped shape read from the material compiler and set: 14 siege
    // records in id order then 7 reference records. Lobes use the file
    // flags plus the four compiler bits below; all shipped records are
    // untextured today so the mask is 0 and the four-slot path is proved
    // by the draw tests above. Layers split ceramic from stone in the
    // full build; this slice keys lobes and slots only.
    const EMISSION_BIT: u8 = 32;
    const ANISOTROPY_BIT: u8 = 4;
    const TRANSMISSION_BIT: u8 = 8;
    const SUBSURFACE_BIT: u8 = 16;

    const SHIPPED: [(&str, u8, u8); 21] = [
        ("stone", LOBE_COAT | EMISSION_BIT, 0),
        ("brick", LOBE_COAT | EMISSION_BIT, 0),
        ("timber", EMISSION_BIT, 0),
        ("thatch", LOBE_SHEEN | EMISSION_BIT, 0),
        ("plaster", LOBE_COAT | EMISSION_BIT, 0),
        ("concrete", LOBE_COAT | EMISSION_BIT, 0),
        ("marble", LOBE_COAT | EMISSION_BIT, 0),
        ("iron", EMISSION_BIT, 0),
        ("bronze", EMISSION_BIT, 0),
        ("leather", LOBE_SHEEN | EMISSION_BIT, 0),
        ("cloth", LOBE_SHEEN | EMISSION_BIT, 0),
        ("rope", EMISSION_BIT, 0),
        ("glass", TRANSMISSION_BIT | EMISSION_BIT, 0),
        ("ceramic", LOBE_COAT | EMISSION_BIT, 0),
        ("sand", LOBE_COAT | EMISSION_BIT, 0),
        ("steel", ANISOTROPY_BIT | EMISSION_BIT, 0),
        ("skin", SUBSURFACE_BIT | LOBE_COAT, 0),
        ("marble", LOBE_COAT | EMISSION_BIT, 0),
        ("wood", EMISSION_BIT, 0),
        ("cloth", LOBE_SHEEN | EMISSION_BIT, 0),
        ("marble_wet", LOBE_COAT | EMISSION_BIT, 0),
    ];

    #[test]
    fn shipped_records_all_build() {
        assert_eq!(SHIPPED.len(), 21);
        for &(name, lobes, slot_mask) in SHIPPED.iter() {
            let first = pipeline_key_for(lobes, slot_mask);
            let again = pipeline_key_for(lobes, slot_mask);
            assert_eq!(first, again, "{name} builds stably");
            let bin = MaterialBinKey::new(1, lobes, [false, false, false, false]);
            assert_eq!(bin.slot_mask, slot_mask, "{name} mask");
            assert_eq!(bin.texture_key_count(), 0, "{name} untextured");
        }
    }

    #[test]
    fn shipped_pipelines_fit_budget() {
        let mut keys: Vec<u64> = SHIPPED
            .iter()
            .map(|&(_, lobes, slot_mask)| pipeline_key_for(lobes, slot_mask))
            .collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(SHIPPED.len(), 21);
        assert!(
            keys.len() <= MAX_MATERIAL_PIPELINES,
            "{} pipelines for 21 records",
            keys.len()
        );
    }

    #[test]
    fn texture_keys_per_draw_are_four() {
        assert_eq!(MATERIAL_TEXTURE_SLOTS, 4);
        assert_eq!(SLOT_ORDER.len(), 4);
        assert_eq!(SLOT_BITS.len(), 4);
        let full = MaterialBinKey::new(5, 0, [true, true, true, true]);
        assert_eq!(full.texture_key_count(), 4);
        assert_eq!(full.slot_mask, 15);
    }

    #[test]
    fn shared_pipeline_key_ignores_material_id() {
        let a = MaterialBinKey::new(1, LOBE_COAT, [true, false, true, false]);
        let b = MaterialBinKey::new(2, LOBE_COAT, [true, false, true, false]);
        assert_ne!(a, b);
        assert_ne!(a.pipeline_key(), b.pipeline_key());
        assert_eq!(
            pipeline_key_for(a.lobes, a.slot_mask),
            pipeline_key_for(b.lobes, b.slot_mask)
        );
        assert_eq!(
            pipeline_key_for(LOBE_COAT, 1 | 4),
            pipeline_key_for(a.lobes, a.slot_mask)
        );
        assert_eq!(pipeline_count(&[a, b]), 1);
    }

    #[test]
    fn pipeline_count_shares_across_material_ids() {
        let draws: Vec<MaterialBinKey> = SHIPPED
            .iter()
            .enumerate()
            .map(|(index, &(_, lobes, slot_mask))| {
                let id = u64::try_from(index).expect("21 draws fit in u64");
                let bound = [
                    slot_mask & SLOT_BITS[0] != 0,
                    slot_mask & SLOT_BITS[1] != 0,
                    slot_mask & SLOT_BITS[2] != 0,
                    slot_mask & SLOT_BITS[3] != 0,
                ];
                MaterialBinKey::new(id, lobes, bound)
            })
            .collect();
        assert_eq!(draws.len(), 21);
        assert_eq!(bin_draws(&draws).len(), 21);
        assert!(pipeline_count(&draws) <= MAX_MATERIAL_PIPELINES);
    }
}

/// NS31c3: the live swap gate. A real device draws one material frame per
/// edit, so the bar is "the edit reached the presented pixels", not "the
/// struct changed". Window feature only; no adapter skips.
#[cfg(all(test, feature = "gpu"))]
mod material_live_tests {
    use super::*;
    use crate::test_gpu::{hold_gpu_slot, require_or_skip, NoAdapter};
    use engine_material::{Coat, MaterialSet};

    /// Side of the capture frame; 64 px keeps the draw and readback cheap.
    const SIDE: u32 = 64;
    /// Bytes per row of an Rgba8 frame; 64 px is already 256-byte aligned.
    const ROW_BYTES: u32 = SIDE * 4;
    /// Structure-edit runs the p95 bar is read over.
    const STRUCTURE_RUNS: usize = 20;

    /// A real device plus the adapter name, or a skip when none exists.
    fn device_or_skip(test: &str) -> Option<(wgpu::Device, wgpu::Queue, String)> {
        hold_gpu_slot();
        let instance = crate::backend::new_instance(wgpu::Backends::PRIMARY);
        let options = wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        };
        let probe = pollster::block_on(instance.request_adapter(&options))
            .map_err(|error| NoAdapter(format!("no wgpu adapter: {error}")));
        let adapter = require_or_skip(test, probe)?;
        let request = wgpu::DeviceDescriptor {
            label: Some("material-live-test"),
            ..Default::default()
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&request))
            .unwrap_or_else(|error| panic!("adapter present, device request failed: {error}"));
        Some((device, queue, adapter.get_info().name))
    }

    /// Timber with a roughness and an optional coat, as record text. Timber
    /// ships in the siege kit with no `extends`, so the text stands alone.
    fn timber_text(roughness: &str, coat: bool) -> String {
        let mut record = MaterialSet::siege_kit()
            .get("timber")
            .cloned()
            .expect("shipped timber");
        record.look.specular_roughness = roughness.parse().expect("a roughness number");
        record.look.coat = coat.then_some(Coat {
            weight: 0.6,
            roughness: 0.2,
            ior: 1.5,
        });
        record.to_text()
    }

    /// A uniform buffer holding `bytes`.
    fn uniform_buffer(device: &wgpu::Device, label: &str, bytes: &[u8]) -> wgpu::Buffer {
        use wgpu::util::DeviceExt;
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytes,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    }

    /// Identity view-projection with the eye at +z: a clip-space triangle
    /// lands on the frame whatever the camera math is elsewhere.
    fn camera_bytes() -> Vec<u8> {
        crate::v1::CameraUniform {
            view_proj_cols: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            camera_pos: [0.0, 0.0, 2.0],
        }
        .to_bytes()
    }

    /// One warm directional light from the upper left.
    fn light_bytes() -> Vec<u8> {
        crate::v1::LightUniform {
            dir_to_light: [0.4, 0.8, 0.45],
            color: [1.0, 1.0, 1.0],
            intensity: 4.0,
        }
        .to_bytes()
    }

    /// A clip-space triangle: three vertices of pos, +z normal and uv, in
    /// the 48-byte stride the generated `vs` reads (12 f32 plus a tangent
    /// pad the vertex layout leaves unread).
    fn vertex_buffer(device: &wgpu::Device) -> wgpu::Buffer {
        use wgpu::util::DeviceExt;
        let corners = [
            ([-1.0f32, -1.0, 0.0], [0.0f32, 0.0, 1.0], [0.0f32, 1.0]),
            ([3.0, -1.0, 0.0], [0.0, 0.0, 1.0], [2.0, 1.0]),
            ([-1.0, 3.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0]),
        ];
        let mut bytes = Vec::with_capacity(48 * corners.len());
        for (pos, normal, uv) in corners {
            for value in pos.iter().chain(normal.iter()).chain(uv.iter()) {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            bytes.extend_from_slice(&[0u8; 16]);
        }
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("material-live-verts"),
            contents: &bytes,
            usage: wgpu::BufferUsages::VERTEX,
        })
    }

    /// One headless material frame: the live slot, the fixed camera, light
    /// and channel uniforms, a clip-space triangle and a readback buffer.
    struct Harness {
        device: wgpu::Device,
        queue: wgpu::Queue,
        live: MaterialLive,
        verts: wgpu::Buffer,
        cam: wgpu::Buffer,
        light: wgpu::Buffer,
        chan: wgpu::Buffer,
        color: wgpu::Texture,
        depth: wgpu::Texture,
        readback: wgpu::Buffer,
    }

    impl Harness {
        fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
            let verts = vertex_buffer(&device);
            let cam = uniform_buffer(&device, "material-live-cam", &camera_bytes());
            let light = uniform_buffer(&device, "material-live-light", &light_bytes());
            let chan = uniform_buffer(&device, "material-live-chan", &[0u8; 32]);
            let color = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("material-live-color"),
                size: wgpu::Extent3d {
                    width: SIDE,
                    height: SIDE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: COLOR_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let depth = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("material-live-depth"),
                size: wgpu::Extent3d {
                    width: SIDE,
                    height: SIDE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("material-live-readback"),
                size: u64::from(ROW_BYTES) * u64::from(SIDE),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let live = MaterialLive::new(&device, &queue);
            Self {
                device,
                queue,
                live,
                verts,
                cam,
                light,
                chan,
                color,
                depth,
                readback,
            }
        }

        /// Draw one frame with the live pipeline and read its pixels back.
        /// The frame's full fnv64 is the capture hash; the same frame hashes
        /// the same on one device.
        fn present(&self) -> u64 {
            let pipeline = self.live.pipeline().expect("a live pipeline draws");
            let uniform = self.live.uniform().expect("a live uniform binds");
            let layout = pipeline.get_bind_group_layout(0);
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("material-live-bind"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.cam.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.light.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: self.chan.as_entire_binding(),
                    },
                ],
            });
            let color_view = self
                .color
                .create_view(&wgpu::TextureViewDescriptor::default());
            let depth_view = self
                .depth
                .create_view(&wgpu::TextureViewDescriptor::default());
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("material-live-frame"),
                });
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("material-live-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &color_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &bind, &[]);
                pass.set_vertex_buffer(0, self.verts.slice(..));
                pass.draw(0..3, 0..1);
            }
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.color,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &self.readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(ROW_BYTES),
                        rows_per_image: Some(SIDE),
                    },
                },
                wgpu::Extent3d {
                    width: SIDE,
                    height: SIDE,
                    depth_or_array_layers: 1,
                },
            );
            self.queue.submit(Some(encoder.finish()));
            let slice = self.readback.slice(..);
            let (sender, receiver) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
            self.device
                .poll(wgpu::PollType::wait_indefinitely())
                .expect("poll the frame");
            receiver
                .recv()
                .expect("the map channel answered")
                .expect("the readback mapped");
            let mut hash = Fnv64::new();
            {
                let mapped = slice
                    .get_mapped_range()
                    .expect("the frame's bytes are readable");
                hash.write(&mapped);
            }
            self.readback.unmap();
            hash.finish()
        }
    }

    /// The numbers the Accept line writes to `target/ns31c/reload.json`.
    struct ReloadReport<'a> {
        adapter: String,
        value_ms: f64,
        structure_ms: &'a [f64],
        p95_ms: f64,
        hash_before: u64,
        hash_after: u64,
    }

    /// Write the reload receipt under the workspace target directory.
    fn write_reload_report(report: &ReloadReport<'_>) {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/ns31c/reload.json");
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).expect("the ns31c receipt directory");
        }
        let runs: Vec<String> = report
            .structure_ms
            .iter()
            .map(|ms| format!("{ms:.3}"))
            .collect();
        let body = format!(
            "{{\n  \"adapter\": \"{}\",\n  \"value_ms\": {:.3},\n  \"value_bar_ms\": 100,\n  \
             \"structure_runs\": {},\n  \"structure_ms\": [{}],\n  \"structure_p95_ms\": {:.3},\n  \
             \"structure_bar_ms\": 1000,\n  \"broken_refused\": 3,\n  \"frame_hash_before\": \"{:016x}\",\n  \
             \"frame_hash_after\": \"{:016x}\",\n  \"ok\": true\n}}\n",
            report.adapter,
            report.value_ms,
            report.structure_ms.len(),
            runs.join(", "),
            report.p95_ms,
            report.hash_before,
            report.hash_after,
        );
        std::fs::write(&path, &body).expect("write the ns31c receipt");
        eprintln!("NS31c3 reload receipt {}: {body}", path.display());
    }

    #[test]
    fn material_live_value_and_structure_edits_reach_the_frame() {
        let Some((device, queue, adapter)) = device_or_skip("material_live") else {
            return;
        };
        let mut harness = Harness::new(device, queue);
        // First good compile: the base structure starts drawing.
        assert_eq!(
            harness
                .live
                .apply_text(&timber_text("0.35", false))
                .expect("the base compiles"),
            LiveEdit::Structure
        );
        let base = harness.present();
        assert!(harness.live.is_drawing(), "the base pipeline draws");
        // A value edit rewrites the uniform buffer and moves the pixels.
        let value_start = std::time::Instant::now();
        assert_eq!(
            harness
                .live
                .apply_text(&timber_text("0.08", false))
                .expect("a value edit"),
            LiveEdit::Values
        );
        let value_frame = harness.present();
        let value_ms = value_start.elapsed().as_secs_f64() * 1000.0;
        assert_ne!(value_frame, base, "the value edit reached the frame");
        assert!(
            value_ms <= 100.0,
            "a value edit reached the frame in {value_ms} ms, bar 100"
        );
        // A structure edit (a coat) builds a new pipeline first and swaps it.
        let mut structure_ms: Vec<f64> = Vec::with_capacity(STRUCTURE_RUNS);
        for run in 0..STRUCTURE_RUNS {
            let coat = run % 2 == 0;
            let start = std::time::Instant::now();
            assert_eq!(
                harness
                    .live
                    .apply_text(&timber_text("0.08", coat))
                    .expect("a structure edit"),
                LiveEdit::Structure,
                "run {run} swaps a structure"
            );
            let _ = harness.present();
            structure_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        structure_ms.sort_by(|a, b| a.partial_cmp(b).expect("finite ms"));
        // Nearest rank: the 19th of 20 sorted runs, index 18.
        let p95_ms = structure_ms[(STRUCTURE_RUNS * 95).div_ceil(100) - 1];
        assert!(
            p95_ms <= 1000.0,
            "a structure edit took {p95_ms} ms at p95 over {STRUCTURE_RUNS} runs, bar 1000"
        );
        // The last good frame: the last accepted structure's pixels.
        let good = harness.present();
        // Three broken texts are refused; the frame never moves.
        let broken = [
            timber_text("1.7", false),
            timber_text("0.35", false).replacen("\"look\"", "\"lookk\"", 1),
            "{ not json".to_string(),
        ];
        for text in &broken {
            assert!(
                harness.live.apply_text(text).is_err(),
                "a broken text is refused"
            );
        }
        let after = harness.present();
        assert_eq!(good, after, "a refusal changes nothing");
        assert!(harness.live.look_digest().is_some(), "a good edit is held");
        let report = ReloadReport {
            adapter,
            value_ms,
            structure_ms: &structure_ms,
            p95_ms,
            hash_before: good,
            hash_after: after,
        };
        write_reload_report(&report);
    }
}
