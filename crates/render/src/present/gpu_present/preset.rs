//! preset: the presenter's constructors, one per sky rig.
//!
//! Owner: render lane; consumers are the window binary, the capture bundle
//! and the web build. Units: pixels for sizes, booleans for the rig choice.
//! Determinism: the rig is a fixed bake, so the same rig gives the same
//! presenter. Errors: typed [`PresentError`]; `NoAdapter` on a headless
//! machine is skip-not-green for callers, never a fake pass.
//!
//! Shape: the day rig is the default and keeps every existing capture
//! byte-equal. The night rig bakes the arena night preset (the moonlit sky,
//! its ambient and its fog), so the placement's torches are the light the
//! frame reads.

use std::sync::Arc;

use winit::window::Window;

use super::presenter::Presenter;
use crate::present::surface::{PresentError, SizeClass};

impl Presenter {
    /// Bring up a high-performance adapter with no surface and compile the
    /// forward plus blit pipelines. `NoAdapter` on headless machines is
    /// skip-not-green for callers, never a fake pass.
    pub fn init_offscreen() -> Result<Self, PresentError> {
        Self::init_offscreen_with_preset(false)
    }

    /// The night presenter: the arena night rig (the moonlit sky, its
    /// ambient and its fog), so the placement's torches are the light the
    /// frame reads. The day presenter is the default and keeps every
    /// existing capture byte-equal.
    pub fn init_offscreen_night() -> Result<Self, PresentError> {
        Self::init_offscreen_with_preset(true)
    }

    /// Bring up the offscreen presenter under one sky preset. `night`
    /// picks the arena night rig; the day rig is the default.
    pub fn init_offscreen_with_preset(night: bool) -> Result<Self, PresentError> {
        // Test builds only: cap the GPU tests that hold a device at once.
        #[cfg(test)]
        crate::test_gpu::hold_gpu_slot();
        // Backend pick is resolved here for the wgpu instance; the binary
        // logs `pick.startup_line()`. No stdout in lib.
        let (choice, _) = crate::backend::resolve_from_env(None);
        let pick = crate::backend::pick_backend(choice, None);
        let instance = crate::backend::new_instance(crate::backend::wgpu_backends(pick.primary));
        // GS6: enumerate and rank-pick the env choice instead of trusting
        // the driver's power-preference hint, so an Optimus laptop never
        // lands on the Intel adapter and the CPU rasterizer fails loudly
        // (NoAdapter) unless E2040_ALLOW_SOFTWARE=1.
        let (adapter, selection) = pollster::block_on(crate::presenter_pick::pick_adapter(
            &instance,
            crate::backend::wgpu_backends(pick.primary),
            &crate::adapter::SelectionRequest::from_env(),
        ))
        .map_err(|_| PresentError::NoAdapter)?;
        let (device, queue) = Self::request_device(&adapter)?;
        let mut presenter =
            Self::init_offscreen_from_device_with_preset(&adapter, device, queue, night)?;
        presenter.backend_selection = selection;
        Ok(presenter)
    }

    /// Bring up an adapter compatible with `surface`, configure the
    /// swapchain at `width` x `height` and compile the surface blit
    /// pipeline. Zero area is refused (configuring would panic); the
    /// caller suspends instead. The surface keeps the window alive.
    pub fn init_windowed(
        instance: &wgpu::Instance,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
    ) -> Result<Self, PresentError> {
        Self::init_windowed_with_preset(instance, surface, width, height, false, false)
    }

    /// The windowed presenter under one sky preset. `night` picks the
    /// arena night rig: the moonlit sky, its ambient and its fog, so the
    /// placement's torches are the light the frame reads. The day rig is
    /// the default and keeps every existing capture byte-equal. `uncapped`
    /// picks Immediate or Mailbox present mode instead of Fifo (vsync).
    pub fn init_windowed_with_preset(
        instance: &wgpu::Instance,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
        night: bool,
        uncapped: bool,
    ) -> Result<Self, PresentError> {
        if SizeClass::classify(width, height) != SizeClass::Sizes {
            return Err(PresentError::InvalidSize { width, height });
        }
        // Windowed path uses the caller-built instance; backend selection
        // is reported by the binary via `backend::startup_line`. No
        // stdout in lib. GS6: rank-pick over the surface-compatible
        // adapters so the env choice wins over the driver's hint.
        let (bchoice, _) = crate::backend::resolve_from_env(None);
        let bp = crate::backend::pick_backend(bchoice, None);
        let (adapter, selection) = Self::pick_surface_adapter(
            instance,
            &surface,
            crate::backend::wgpu_backends(bp.primary),
        )?;
        let info = adapter.get_info();
        let (device, queue) = Self::request_device(&adapter)?;
        // EG13: the adapter owns the MSAA count; read its 4x support once.
        let support = Self::msaa_support_for(&adapter);
        let mut presenter = Self::build_with_preset(device, queue, info, support, night)?;
        presenter.backend_selection = selection;
        presenter.attach_surface(instance, surface, width, height, uncapped)?;
        Ok(presenter)
    }

    /// Bring up the visible presenter for an owned winit window: build
    /// the backend-picked instance, attach the surface (which keeps the
    /// window alive) and configure the swapchain at `width` x `height`.
    /// The instance is retained through the attach so the surface stays
    /// valid past this call. This is the one call the window loop makes;
    /// the offscreen twin (`init_offscreen` plus `render_offscreen`)
    /// stays the explicit capture path. Zero area fails loudly (the
    /// caller suspends instead); `NoAdapter` is never a fake present.
    pub fn init_for_window(
        window: Arc<Window>,
        width: u32,
        height: u32,
    ) -> Result<Self, PresentError> {
        Self::init_for_window_with_preset(window, width, height, false, false)
    }

    /// The visible presenter under one sky preset. `night` picks the arena
    /// night rig: the moonlit sky, its ambient and its fog, so the
    /// placement's torches are the light the frame reads. The day rig is
    /// the default and keeps every existing capture byte-equal. `uncapped`
    /// picks Immediate or Mailbox present mode instead of Fifo (vsync).
    pub fn init_for_window_with_preset(
        window: Arc<Window>,
        width: u32,
        height: u32,
        night: bool,
        uncapped: bool,
    ) -> Result<Self, PresentError> {
        if SizeClass::classify(width, height) != SizeClass::Sizes {
            return Err(PresentError::InvalidSize { width, height });
        }
        let (choice, _) = crate::backend::resolve_from_env(None);
        let pick = crate::backend::pick_backend(choice, None);
        let instance = crate::backend::new_instance(crate::backend::wgpu_backends(pick.primary));
        let surface = crate::present::gpu_present::create_surface(&instance, window)?;
        Self::init_windowed_with_preset(&instance, surface, width, height, night, uncapped)
    }
}
