//! gpu_cull: GS18 slice 1 device runner for the frustum cull.
//!
//! Owner: render lane. The rule owner stays `crate::draw`: this runner calls
//! [`draw::cull_cpu_flags_into`](crate::draw::cull_cpu_flags_into),
//! [`draw::visible_count`](crate::draw::visible_count) and
//! [`draw::indirect_args_5`](crate::draw::indirect_args_5) over the same
//! packed `[x, y, z, radius]` spheres, so the device pass cannot drift from
//! the CPU oracle `draw.rs` already pins for `shaders/cull.wgsl`.
//! UNLOCKS: GS18b (slice 2, "draws go indirect") owns the `draw.rs` hook that
//! reads these 5-u32 args through `draw_indexed_indirect`; until it lands no
//! draw path calls this runner, and the row's own text says so ("No draw path
//! uses it yet"). The device entry is a proof path: it maps buffers and waits,
//! so the frame loop must never call it (render-pipeline rule 2).
//! Units: meters for sphere centers and radii, u32 words for the args.
//! Determinism: the cull rule is `+ - *` in fixed plane order and the flags
//! are per index, so the same bytes give the same answer twice; the visible
//! set is order-free and [`CullProof::visible_ids`] sorts it ascending.
//! Errors: [`CullError::BadInput`] before any device work, `Device` for a
//! failed buffer or map step, `Validation` for a wgpu validation error and
//! `Mismatch` when the device answer misses the CPU oracle. Never panics.

use std::fmt;

use crate::draw::{cull_cpu_flags_into, indirect_args_5, visible_count};

/// Cull shader source: the same file `draw.rs` includes for the CPU pin, so
/// one rule serves both halves (one asset, no copy).
const CULL_WGSL: &str = include_str!("shaders/cull.wgsl");

/// Threads per cull workgroup: matches `@workgroup_size(64)` in the shader.
pub const CULL_WORKGROUP: u32 = 64;

/// Indirect args words: index count, instance count, first index, base
/// vertex, first instance (an indexed draw, the 5-word form).
pub const INDIRECT_WORDS: usize = 5;

/// Slice-1 cull failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CullError {
    /// Input refused before any device work; the message names the field.
    BadInput(String),
    /// A device call, buffer or map step failed; the message carries it.
    Device(String),
    /// The device reported a wgpu validation error; the text carries it.
    Validation(String),
    /// The device answer disagrees with the CPU oracle. `device` is the
    /// visible count the pass produced, `cpu` the oracle's.
    Mismatch {
        /// Visible count read back from the device.
        device: u32,
        /// Visible count the CPU oracle produces for the same input.
        cpu: u32,
    },
}

impl fmt::Display for CullError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CullError::BadInput(why) => write!(out, "bad cull input: {why}"),
            CullError::Device(why) => write!(out, "cull device: {why}"),
            CullError::Validation(why) => write!(out, "cull validation: {why}"),
            CullError::Mismatch { device, cpu } => {
                write!(out, "cull mismatch: device {device} visible, oracle {cpu}")
            }
        }
    }
}

impl std::error::Error for CullError {}

/// One slice-1 proof: what the device wrote next to the oracle's count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CullProof {
    /// Visible count the pass counted (the `visible_count` atomic).
    pub count: u32,
    /// Visible count the CPU oracle produces for the same input.
    pub cpu_count: u32,
    /// The 5-u32 indirect args the pass wrote (36 indices, the count).
    pub args: [u32; INDIRECT_WORDS],
    /// One 0/1 flag per sphere, in input order, as the device wrote them.
    pub flags: Vec<u32>,
    /// wgpu validation errors caught around the pass; must be 0.
    pub validation_errors: u32,
}

impl CullProof {
    /// Sphere indices the device kept, ascending.
    #[must_use]
    pub fn visible_ids(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = Vec::new();
        for (index, flag) in self.flags.iter().enumerate() {
            if *flag == 1 {
                ids.push(index as u32);
            }
        }
        ids
    }
}

/// Refuse input the pass cannot answer for, before any device work: an empty
/// list, a non-finite center or radius, a negative radius, or a plane with a
/// non-finite component.
pub fn check_inputs(spheres: &[[f32; 4]], planes: &[[f32; 4]; 6]) -> Result<(), CullError> {
    if spheres.is_empty() {
        return Err(CullError::BadInput(
            "no spheres: an empty cull has nothing to count".to_string(),
        ));
    }
    for (index, sphere) in spheres.iter().enumerate() {
        if !sphere.iter().all(|word| word.is_finite()) {
            return Err(CullError::BadInput(format!(
                "sphere {index} center or radius is not finite"
            )));
        }
        if sphere[3] < 0.0 {
            return Err(CullError::BadInput(format!(
                "sphere {index} radius {} is negative",
                sphere[3]
            )));
        }
    }
    for (index, plane) in planes.iter().enumerate() {
        if !plane.iter().all(|word| word.is_finite()) {
            return Err(CullError::BadInput(format!("plane {index} is not finite")));
        }
    }
    Ok(())
}

/// `words` as little-endian `f32` bytes; the shader's storage layout is
/// `vec4<f32>`, so one `[f32; 4]` is one record with no padding.
fn pack_vec4(words: &[[f32; 4]]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(words.len() * 16);
    for word in words {
        for value in word {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    bytes
}

/// Reusable slice-1 device state: the cull pipeline plus its `write_args`
/// pipeline and one bind layout. Built once per device; buffers size per
/// call (the proof path, never the frame).
pub struct GpuCuller {
    cull: wgpu::ComputePipeline,
    write_args: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

impl GpuCuller {
    /// Compile both entries of `shaders/cull.wgsl` over one bind layout.
    pub fn new(device: &wgpu::Device) -> Self {
        let entry = |binding: u32, read_only: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gs18-cull-layout"),
            entries: &[
                entry(0, true),
                entry(1, true),
                entry(2, false),
                entry(3, false),
                entry(4, false),
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gs18-cull"),
            source: wgpu::ShaderSource::Wgsl(CULL_WGSL.into()),
        });
        let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gs18-cull"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipe = |label: &str, entry_point: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&pipe_layout),
                module: &module,
                entry_point: Some(entry_point),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        Self {
            cull: pipe("gs18-cull-main", "main"),
            write_args: pipe("gs18-cull-args", "write_args"),
            layout,
        }
    }

    /// Run the slice-1 pass over `spheres` (`[x, y, z, radius]`, packed) and
    /// six `[nx, ny, nz, dist]` planes, then read the flags, the count and
    /// the args back and refuse when they miss the CPU oracle.
    pub fn cull(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        spheres: &[[f32; 4]],
        planes: &[[f32; 4]; 6],
    ) -> Result<CullProof, CullError> {
        check_inputs(spheres, planes)?;
        let count = spheres.len() as u32;
        let flags_bytes = u64::from(count) * 4;
        let args_bytes = INDIRECT_WORDS as u64 * 4;
        use wgpu::util::DeviceExt;
        let storage = wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC;
        let spheres_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gs18-cull-spheres"),
            contents: &pack_vec4(spheres),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        let planes_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gs18-cull-planes"),
            contents: &pack_vec4(planes),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        let flags_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gs18-cull-flags"),
            size: flags_bytes,
            usage: storage,
            mapped_at_creation: false,
        });
        let count_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gs18-cull-count"),
            size: 4,
            usage: storage,
            mapped_at_creation: false,
        });
        let args_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gs18-cull-args"),
            size: args_bytes,
            usage: storage,
            mapped_at_creation: false,
        });
        queue.write_buffer(&flags_buf, 0, &vec![0u8; flags_bytes as usize]);
        queue.write_buffer(&count_buf, 0, &0u32.to_le_bytes());
        queue.write_buffer(&args_buf, 0, &vec![0u8; args_bytes as usize]);
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gs18-cull-bind"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: spheres_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: planes_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: flags_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: count_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: args_buf.as_entire_binding(),
                },
            ],
        });
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gs18-cull"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gs18-cull-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.cull);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(count.div_ceil(CULL_WORKGROUP), 1, 1);
            // One thread turns the counted survivors into the draw args.
            pass.set_pipeline(&self.write_args);
            pass.dispatch_workgroups(1, 1, 1);
        }
        queue.submit(Some(encoder.finish()));
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|error| CullError::Device(format!("poll: {error}")))?;
        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(CullError::Validation(format!("{error:?}")));
        }
        // Past the scope every validation error is a refusal above, so a
        // proof only ever carries a clean 0 here.
        let validation_errors = 0u32;
        // One mapped read: count, then the flags, then the args.
        let back_size = 4 + flags_bytes + args_bytes;
        let back = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gs18-cull-readback"),
            size: back_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gs18-cull-copy"),
        });
        encoder.copy_buffer_to_buffer(&count_buf, 0, &back, 0, 4);
        encoder.copy_buffer_to_buffer(&flags_buf, 0, &back, 4, flags_bytes);
        encoder.copy_buffer_to_buffer(&args_buf, 0, &back, 4 + flags_bytes, args_bytes);
        queue.submit(Some(encoder.finish()));
        let slice = back.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|error| CullError::Device(format!("poll: {error}")))?;
        receiver
            .recv()
            .map_err(|error| CullError::Device(format!("map channel: {error}")))?
            .map_err(|error| CullError::Device(format!("map: {error}")))?;
        let device_count;
        let device_flags;
        let device_args;
        {
            let mapped = slice
                .get_mapped_range()
                .map_err(|error| CullError::Device(format!("mapped range: {error}")))?;
            let word = |at: usize| {
                let mut four = [0u8; 4];
                four.copy_from_slice(&mapped[at..at + 4]);
                u32::from_le_bytes(four)
            };
            device_count = word(0);
            device_flags = (0..count as usize)
                .map(|index| word(4 + index * 4))
                .collect::<Vec<u32>>();
            let mut args = [0u32; INDIRECT_WORDS];
            for (index, slot) in args.iter_mut().enumerate() {
                *slot = word(4 + flags_bytes as usize + index * 4);
            }
            device_args = args;
        }
        // The range guard is gone with the block above: the map may go.
        back.unmap();
        // The oracle is the check, not a second opinion.
        let mut cpu_flags = vec![0u8; spheres.len()];
        let written = cull_cpu_flags_into(spheres, planes, &mut cpu_flags);
        let cpu_count = visible_count(&cpu_flags[..written]);
        if device_count != cpu_count {
            return Err(CullError::Mismatch {
                device: device_count,
                cpu: cpu_count,
            });
        }
        if device_flags
            .iter()
            .zip(cpu_flags.iter())
            .any(|(device, cpu)| *device != u32::from(*cpu))
        {
            return Err(CullError::Mismatch {
                device: device_count,
                cpu: cpu_count,
            });
        }
        let want_args = indirect_args_5(cpu_count);
        if device_args != want_args {
            return Err(CullError::Mismatch {
                device: device_count,
                cpu: cpu_count,
            });
        }
        Ok(CullProof {
            count: device_count,
            cpu_count,
            args: device_args,
            flags: device_flags,
            validation_errors,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_gpu::{require_or_skip, NoAdapter};

    /// A box frustum with axis-aligned inward normals: `0` and `1` plane
    /// components keep every product exact, so the device and the oracle
    /// compare the same floats whatever the driver contracts (no FMA drift).
    fn box_planes() -> [[f32; 4]; 6] {
        [
            [1.0, 0.0, 0.0, 5.0],
            [-1.0, 0.0, 0.0, 5.0],
            [0.0, 1.0, 0.0, 5.0],
            [0.0, -1.0, 0.0, 5.0],
            [0.0, 0.0, -1.0, -1.0],
            [0.0, 0.0, 1.0, 20.0],
        ]
    }

    fn device_or_skip(test: &str) -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let options = wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        };
        let probe = match pollster::block_on(instance.request_adapter(&options)) {
            Ok(adapter) => Ok(adapter),
            Err(error) => Err(NoAdapter(format!("no wgpu adapter: {error}"))),
        };
        let adapter = require_or_skip(test, probe)?;
        match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("gs18-cull-test"),
            ..Default::default()
        })) {
            Ok((device, queue)) => Some((device, queue)),
            Err(error) => panic!("adapter present, device request must succeed: {error}"),
        }
    }

    /// `n` spheres from a fixed LCG, straddling the box so both verdicts
    /// appear. Separate statements: the draw order stays fixed.
    fn seeded_spheres(n: usize) -> Vec<[f32; 4]> {
        let mut state = 2040u64;
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((state >> 40) as f32 / 8_388_608.0) - 1.0
        };
        let mut out = Vec::with_capacity(n);
        while out.len() < n {
            let x = next() * 12.0;
            let y = next() * 12.0;
            let z = next() * 22.0 - 12.0;
            let radius = 0.05 + next().abs() * 0.6;
            out.push([x, y, z, radius]);
        }
        out
    }

    /// Touching counts as visible: a center on the plane, one exactly
    /// touching, one a hair outside, the three-plane corner, one behind the
    /// far plane and one past the right plane.
    fn boundary_spheres() -> Vec<[f32; 4]> {
        vec![
            [-5.0, 0.0, -10.0, 0.0],
            [-5.5, 0.0, -10.0, 0.5],
            [-5.5, 0.0, -10.0, 0.25],
            [-5.0, 5.0, -1.0, 0.0],
            [0.0, 0.0, 25.0, 0.5],
            [6.0, 0.0, -10.0, 0.0],
        ]
    }

    #[test]
    fn cull_refuses_bad_input_before_any_device_work() {
        let planes = box_planes();
        assert!(matches!(
            check_inputs(&[], &planes),
            Err(CullError::BadInput(_))
        ));
        let mut bad_plane = planes;
        bad_plane[2][1] = f32::NAN;
        assert!(matches!(
            check_inputs(&[[0.0, 0.0, -10.0, 1.0]], &bad_plane),
            Err(CullError::BadInput(_))
        ));
        assert!(matches!(
            check_inputs(&[[0.0, 0.0, -10.0, -1.0]], &planes),
            Err(CullError::BadInput(_))
        ));
        assert!(check_inputs(&[[0.0, 0.0, -10.0, 1.0]], &planes).is_ok());
    }

    #[test]
    fn gpu_cull_matches_the_cpu_oracle_over_10k_spheres() {
        let Some((device, queue)) =
            device_or_skip("gpu_cull_matches_the_cpu_oracle_over_10k_spheres")
        else {
            return;
        };
        let mut spheres = seeded_spheres(10_000);
        spheres.extend(boundary_spheres());
        let planes = box_planes();
        let culler = GpuCuller::new(&device);
        let first = culler
            .cull(&device, &queue, &spheres, &planes)
            .expect("first cull runs");
        let second = culler
            .cull(&device, &queue, &spheres, &planes)
            .expect("second cull runs");
        let mut oracle = vec![0u8; spheres.len()];
        let written = cull_cpu_flags_into(&spheres, &planes, &mut oracle);
        assert_eq!(written, spheres.len(), "the oracle covered every sphere");
        let cpu_count = visible_count(&oracle);
        assert_eq!(
            first.count, cpu_count,
            "the device count is the oracle count"
        );
        assert!(
            cpu_count > 0 && cpu_count < spheres.len() as u32,
            "the fixture must hold both verdicts, saw {cpu_count}"
        );
        let want_flags: Vec<u32> = oracle.iter().map(|flag| u32::from(*flag)).collect();
        assert_eq!(
            first.flags, want_flags,
            "device flags equal the oracle's, in input order"
        );
        assert_eq!(first.args.len(), 5, "args are 5 u32, not 4");
        assert_eq!(
            first.args,
            indirect_args_5(cpu_count),
            "36 indices, one instance per visible sphere, no count buffer"
        );
        assert_eq!(first.validation_errors, 0, "the pass is validation clean");
        assert_eq!(second.count, first.count, "x2 count");
        assert_eq!(second.args, first.args, "x2 args");
        assert_eq!(second.flags, first.flags, "x2 flags");
        assert_eq!(
            second.visible_ids(),
            first.visible_ids(),
            "the same visible set twice, ascending"
        );
        let ids = first.visible_ids();
        assert!(ids.windows(2).all(|pair| pair[0] < pair[1]), "ids ascend");
        assert_eq!(ids.len(), cpu_count as usize, "one id per visible sphere");
        eprintln!(
            "GS18 gpu cull: {cpu_count} of {} visible, args {:?}",
            spheres.len(),
            first.args
        );
    }
}
