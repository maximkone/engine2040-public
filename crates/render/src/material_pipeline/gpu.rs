//! gpu: one wgpu render pipeline per compiled material structure (NS31c2).
//!
//! `engine_material::compile` writes the WGSL and names the bindings; this
//! module turns that text into a pipeline whose layouts come from the same
//! structure, so the shader and the layout cannot drift apart:
//!
//! - group 0: binding 0 camera, 1 light, 2 the material uniform of
//!   `structure.uniform_bytes()` bytes, 3 the per-object channel block.
//! - group 1, only when the structure binds a texture: binding 0 the shared
//!   sampler, then one filterable 2D texture per bound slot, at the binding
//!   numbers of `structure.texture_bindings()`.
//!
//! Every uniform binding carries its exact byte size, so a shader whose
//! struct is larger than the host writes fails validation here.
//!
//! Units: bytes and binding numbers. Determinism: none needed (presentation
//! only, never a sim hash). Errors: a wgpu validation error inside the
//! build scope becomes [`MaterialPipelineError::Validation`]; nothing
//! panics. Caller: the forward-pass bind (RL46) and `MaterialLive` (NS31c3);
//! until they land the tests below are the proof. Window feature only.

use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::fmt;

use engine_material::compile::{Compiled, CHANNEL_FLOATS};

use crate::variants::{VariantError, VariantManifest};

/// Color target of every material pipeline: the linear working format of
/// the forward capture (the fragment encodes sRGB itself).
pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Depth target of every material pipeline.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Why a compiled material did not become a pipeline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MaterialPipelineError {
    /// wgpu rejected the shader module, a layout or the pipeline.
    Validation(String),
    /// The variant gate refused one more structure.
    Variants(VariantError),
}

impl fmt::Display for MaterialPipelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(text) => write!(f, "material pipeline validation: {text}"),
            Self::Variants(error) => write!(f, "material pipeline gate: {error}"),
        }
    }
}

impl std::error::Error for MaterialPipelineError {}

impl From<VariantError> for MaterialPipelineError {
    fn from(error: VariantError) -> Self {
        Self::Variants(error)
    }
}

/// Bytes as a wgpu size; an unrepresentable size maxes out and fails validation.
fn bytes_u64(bytes: usize) -> u64 {
    u64::try_from(bytes).unwrap_or(u64::MAX)
}

fn uniform_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
    bytes: u64,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(bytes),
        },
        count: None,
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// Group 0 entries: camera, light, material, channels.
fn group0_entries(compiled: &Compiled) -> [wgpu::BindGroupLayoutEntry; 4] {
    let both = wgpu::ShaderStages::VERTEX_FRAGMENT;
    let fragment = wgpu::ShaderStages::FRAGMENT;
    [
        uniform_entry(0, both, crate::v1_gpu::CAMERA_UNIFORM_BYTES),
        uniform_entry(1, fragment, crate::v1_gpu::LIGHT_UNIFORM_BYTES),
        uniform_entry(2, fragment, bytes_u64(compiled.structure.uniform_bytes())),
        uniform_entry(3, fragment, bytes_u64(CHANNEL_FLOATS * 4)),
    ]
}

/// Group 1 entries: the sampler, then the bound texture slots in order.
/// Empty when the structure binds no texture (the shader declares no group 1).
fn group1_entries(compiled: &Compiled) -> Vec<wgpu::BindGroupLayoutEntry> {
    let bindings = compiled.structure.texture_bindings();
    if bindings.is_empty() {
        return Vec::new();
    }
    let sampler = wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    };
    let mut entries = vec![sampler];
    entries.extend(bindings.iter().map(|&(_, binding)| texture_entry(binding)));
    entries
}

/// Create the pipeline. The caller owns the error scope.
fn build(device: &wgpu::Device, compiled: &Compiled) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("material-compiled"),
        source: wgpu::ShaderSource::Wgsl(compiled.wgsl.as_str().into()),
    });
    let group0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("material-group0"),
        entries: &group0_entries(compiled),
    });
    let group1_entries = group1_entries(compiled);
    let group1 = (!group1_entries.is_empty()).then(|| {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material-group1"),
            entries: &group1_entries,
        })
    });
    let mut layouts = vec![Some(&group0)];
    if let Some(group1) = &group1 {
        layouts.push(Some(group1));
    }
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("material-layout"),
        bind_group_layouts: &layouts,
        immediate_size: 0,
    });
    // Blended structures (transmission) read the depth buffer but never write it.
    let blended = compiled.structure.blended();
    let blend = if blended {
        wgpu::BlendState::ALPHA_BLENDING
    } else {
        wgpu::BlendState::REPLACE
    };
    let attributes = [
        (wgpu::VertexFormat::Float32x3, 0),
        (wgpu::VertexFormat::Float32x3, 12),
        (wgpu::VertexFormat::Float32x2, 24),
    ]
    .iter()
    .zip(0u32..)
    .map(
        |(&(format, offset), shader_location)| wgpu::VertexAttribute {
            format,
            offset,
            shader_location,
        },
    )
    .collect::<Vec<_>>();
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("material-pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs"),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: crate::v1_gpu::VERTEX_STRIDE,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &attributes,
            })],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs"),
            targets: &[Some(wgpu::ColorTargetState {
                format: COLOR_FORMAT,
                blend: Some(blend),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(!blended),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// Build the pipeline of one compiled material inside a validation error
/// scope. A wgpu validation error (a bad shader, a layout the shader does
/// not fit) comes back as [`MaterialPipelineError::Validation`] with wgpu's
/// text; it never panics and never reaches the device's uncaptured handler.
pub fn pipeline_for(
    device: &wgpu::Device,
    compiled: &Compiled,
) -> Result<wgpu::RenderPipeline, MaterialPipelineError> {
    // Reading a validation error scope needs a blocked device, which a
    // browser tab cannot do: the wasm32 build builds plainly and lets the
    // uncaptured handler report a bad shader (QW9 owns the async path).
    #[cfg(not(target_arch = "wasm32"))]
    {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let pipeline = build(device, compiled);
        match pollster::block_on(scope.pop()) {
            Some(error) => Err(MaterialPipelineError::Validation(error.to_string())),
            None => Ok(pipeline),
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        Ok(build(device, compiled))
    }
}

/// Pipelines built so far, one per compiled structure key, under the
/// variant gate (`MAX_VARIANTS`). Records that share a structure share a
/// pipeline. A structure whose build failed stays listed in the gate (one
/// slot), so a bad record retried every frame cannot eat more than one.
pub struct MaterialPipelines {
    gate: VariantManifest,
    built: BTreeMap<u64, wgpu::RenderPipeline>,
}

impl Default for MaterialPipelines {
    fn default() -> Self {
        Self::new()
    }
}

impl MaterialPipelines {
    /// Empty set under the shipped variant cap.
    pub fn new() -> Self {
        Self::with_gate(VariantManifest::new())
    }

    fn with_gate(gate: VariantManifest) -> Self {
        Self {
            gate,
            built: BTreeMap::new(),
        }
    }

    /// Pipelines built, one per distinct structure key.
    pub fn len(&self) -> usize {
        self.built.len()
    }

    /// True before the first build.
    pub fn is_empty(&self) -> bool {
        self.built.is_empty()
    }

    /// The pipeline of `compiled`'s structure: a cache hit for a structure
    /// built before, otherwise one gate check and one [`pipeline_for`].
    pub fn get_or_build(
        &mut self,
        device: &wgpu::Device,
        compiled: &Compiled,
    ) -> Result<&wgpu::RenderPipeline, MaterialPipelineError> {
        match self.built.entry(compiled.key) {
            Entry::Occupied(hit) => Ok(hit.into_mut()),
            Entry::Vacant(slot) => {
                let name = format!("material/{:016x}", compiled.key);
                if !self.gate.contains(&name) {
                    self.gate.add_variant(&name, &[])?;
                }
                Ok(slot.insert(pipeline_for(device, compiled)?))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_gpu::{require_or_skip, NoAdapter};
    use engine_material::compile::compile;
    use engine_material::MaterialSet;

    fn device_or_skip(test: &str) -> Option<(wgpu::Device, String)> {
        crate::test_gpu::hold_gpu_slot();
        let instance = crate::backend::new_instance(wgpu::Backends::PRIMARY);
        let options = wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        };
        let probe = pollster::block_on(instance.request_adapter(&options))
            .map_err(|error| NoAdapter(format!("no wgpu adapter: {error}")));
        let adapter = require_or_skip(test, probe)?;
        let request = wgpu::DeviceDescriptor {
            label: Some("material-pipeline-test"),
            ..Default::default()
        };
        let (device, _queue) = pollster::block_on(adapter.request_device(&request))
            .unwrap_or_else(|error| panic!("adapter present, device request failed: {error}"));
        Some((device, adapter.get_info().name))
    }

    #[test]
    fn material_pipeline_all_shipped_records_build_with_zero_validation_errors() {
        let Some((device, adapter)) = device_or_skip("material_pipeline_shipped") else {
            return;
        };
        // The 21 shipped records: the 14 siege kit plus the 7 reference records.
        let (kit, reference) = (MaterialSet::siege_kit(), MaterialSet::reference());
        let records: Vec<_> = kit.records().iter().chain(reference.records()).collect();
        assert_eq!(records.len(), 21);
        let mut pipelines = MaterialPipelines::new();
        for record in records {
            let compiled = compile(record);
            if let Err(error) = pipelines.get_or_build(&device, &compiled) {
                panic!("record {} failed to build: {error}", record.name);
            }
        }
        eprintln!(
            "NS31c2 adapter {adapter}: 21 records, {} pipelines",
            pipelines.len()
        );
        assert!(!pipelines.is_empty());
        assert!(
            pipelines.len() <= crate::material_pipeline::MAX_MATERIAL_PIPELINES,
            "21 records share {} pipelines",
            pipelines.len()
        );
    }

    #[test]
    fn material_pipeline_a_textured_structure_gets_its_group_one_layout() {
        let Some((device, _)) = device_or_skip("material_pipeline_textured") else {
            return;
        };
        let mut record = MaterialSet::shipped().records()[0].clone();
        record.look.textures.base_color = Some("base.png".to_string());
        record.look.textures.normal = Some("normal.png".to_string());
        let compiled = compile(&record);
        assert_eq!(compiled.structure.texture_bindings().len(), 2);
        pipeline_for(&device, &compiled).expect("a textured structure builds");
    }

    #[test]
    fn material_pipeline_a_broken_text_is_an_error_not_a_panic() {
        let Some((device, _)) = device_or_skip("material_pipeline_broken") else {
            return;
        };
        let mut compiled = compile(&MaterialSet::shipped().records()[0]);
        compiled.wgsl = compiled.wgsl.replace("fn fs(", "fn fs_renamed(");
        match pipeline_for(&device, &compiled) {
            Err(MaterialPipelineError::Validation(text)) => assert!(!text.is_empty()),
            other => panic!("expected a validation error, got {other:?}"),
        }
    }

    #[test]
    fn material_pipeline_the_gate_refuses_the_structure_past_its_cap() {
        let Some((device, _)) = device_or_skip("material_pipeline_gate") else {
            return;
        };
        let shipped = MaterialSet::shipped();
        let first = compile(&shipped.records()[0]);
        let other = shipped
            .records()
            .iter()
            .map(compile)
            .find(|compiled| compiled.key != first.key)
            .expect("the shipped set holds two structures");
        let mut pipelines = MaterialPipelines::with_gate(VariantManifest::with_cap(1));
        pipelines.get_or_build(&device, &first).expect("first fits");
        assert!(pipelines.get_or_build(&device, &first).is_ok(), "a hit");
        assert!(matches!(
            pipelines.get_or_build(&device, &other),
            Err(MaterialPipelineError::Variants(
                VariantError::OverCap { .. }
            ))
        ));
        assert_eq!(pipelines.len(), 1);
    }
}
