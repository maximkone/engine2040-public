//! The forward bind group: the shared resource list and its one builder.
//! Every pass that rides the forward group (IBL, live sky view, EG11b sun
//! depth map) adds its entries here, so `presenter` keeps the frame order and
//! this file the group's shape.
//! Owner render lane; Caller `presenter::{forward_bind, forward_bind_maps}`
//! on cold bind creation, never per frame. Units GPU handles. Presentation
//! only, never hashed.

use super::submit::SKIN_BLOCK_BYTES;

/// Every shared resource a forward bind group points at, besides the
/// per-texture base-color view. Borrowed, built where the bind is created.
#[derive(Clone, Copy)]
pub(super) struct ForwardBindSources<'a> {
    pub(super) layout: &'a wgpu::BindGroupLayout,
    pub(super) cam: &'a wgpu::Buffer,
    pub(super) light: &'a wgpu::Buffer,
    pub(super) material: &'a wgpu::Buffer,
    pub(super) material_bytes: u64,
    pub(super) shadow: &'a wgpu::Buffer,
    pub(super) sampler: &'a wgpu::Sampler,
    /// Sampler for the base map (5) and the normal and ORM maps (18): the
    /// clamped `sampler` for plain draws, the Repeat one for material sets.
    pub(super) map_sampler: &'a wgpu::Sampler,
    pub(super) fog: &'a wgpu::Buffer,
    pub(super) torch: &'a wgpu::Buffer,
    pub(super) pre_view: &'a wgpu::TextureView,
    pub(super) brdf_view: &'a wgpu::TextureView,
    pub(super) sh_buf: &'a wgpu::Buffer,
    pub(super) sky_view: &'a wgpu::TextureView,
    pub(super) normal_view: &'a wgpu::TextureView,
    /// ORM map view (binding 19).
    pub(super) orm_view: &'a wgpu::TextureView,
    /// Joint palette storage buffer (binding 7, one block per draw offset).
    pub(super) skin_joints: &'a wgpu::Buffer,
    /// `SkinParams` uniform (binding 8).
    pub(super) skin_params: &'a wgpu::Buffer,
    /// EG11b: live sun depth map (binding 13).
    pub(super) sun_view: &'a wgpu::TextureView,
    /// EG11b: its comparison sampler (binding 14).
    pub(super) sun_smp: &'a wgpu::Sampler,
    /// EG11b: the sun pass frame (binding 20).
    pub(super) sun_buf: &'a wgpu::Buffer,
    /// LF5: the previous frame's view-projection (binding 21).
    pub(super) prev_cam: &'a wgpu::Buffer,
    /// LF5: the previous joint palette (binding 22, one block per draw offset).
    pub(super) prev_skin_joints: &'a wgpu::Buffer,
    /// LF5b: this frame's un-jittered view-projection (binding 23).
    pub(super) steady_cam: &'a wgpu::Buffer,
}

/// LF1 live sky bind: the forward group plus the init-time sky-view
/// texture at 15. Same buffers the IBL live bind carries, plus the sky
/// view; cold bind creation only, never per frame. Keeps the binary
/// uniform layout, adds no pass.
pub(super) fn forward_bind_with_sky(
    device: &wgpu::Device,
    sources: &ForwardBindSources<'_>,
    base_view: &wgpu::TextureView,
) -> wgpu::BindGroup {
    let ForwardBindSources {
        layout,
        cam,
        light,
        material,
        material_bytes,
        shadow,
        sampler,
        map_sampler,
        fog,
        torch,
        pre_view,
        brdf_view,
        sh_buf,
        sky_view,
        normal_view,
        orm_view,
        skin_joints,
        skin_params,
        sun_view,
        sun_smp,
        sun_buf,
        prev_cam,
        prev_skin_joints,
        steady_cam,
    } = *sources;
    fn block(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
        wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer,
                offset: 0,
                size: wgpu::BufferSize::new(SKIN_BLOCK_BYTES),
            }),
        }
    }
    fn whole(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
        wgpu::BindGroupEntry {
            binding,
            resource: buffer.as_entire_binding(),
        }
    }
    fn tex(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
        wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(view),
        }
    }
    fn samp(binding: u32, sampler: &wgpu::Sampler) -> wgpu::BindGroupEntry<'_> {
        wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::Sampler(sampler),
        }
    }
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ra1-forward-bind-sky"),
        layout,
        entries: &[
            // One joint block window; the draw's dynamic offset moves it.
            block(7, skin_joints),
            whole(8, skin_params),
            whole(0, cam),
            whole(1, light),
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: material,
                    offset: 0,
                    size: wgpu::BufferSize::new(material_bytes),
                }),
            },
            whole(3, shadow),
            tex(4, base_view),
            samp(5, map_sampler),
            whole(6, fog),
            tex(9, pre_view),
            tex(10, brdf_view),
            samp(11, sampler),
            whole(12, sh_buf),
            // EG11b: the live sun depth map, its comparison sampler and the
            // pass frame that finds a fragment's texel.
            tex(13, sun_view),
            samp(14, sun_smp),
            whole(20, sun_buf),
            tex(15, sky_view),
            whole(16, torch),
            tex(17, normal_view),
            samp(18, map_sampler),
            tex(19, orm_view),
            whole(21, prev_cam),
            block(22, prev_skin_joints),
            whole(23, steady_cam),
        ],
    })
}
