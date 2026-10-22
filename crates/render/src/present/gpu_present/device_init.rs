//! device_init: building a presenter on a device the caller opened, and the few accessors a
//! caller that owns the device needs (WEB5).
//!
//! The browser opens its adapter, device and surface with `await`s, so these constructors take
//! the parts and make no blocking call; the native constructors in `presenter` end in the same
//! `init_offscreen_from_device`. Units: pixels for sizes. Caller: `crates/web` (the arena
//! frame) and the native parity test in `presenter`.
use super::super::*;
use super::presenter::Presenter;

impl Presenter {
    /// Build the headless presenter on an adapter, device and queue the caller opened itself
    /// (see [`Self::device_descriptor`]). No window, no surface and no blocking call: the
    /// browser opens its device with `await`s and the native tests open one to prove both
    /// routes give the same presenter. [`Self::init_offscreen`] ends here too.
    pub fn init_offscreen_from_device(
        adapter: &wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
    ) -> Result<Self, PresentError> {
        Self::init_offscreen_from_device_with_preset(adapter, device, queue, false)
    }

    /// Build the headless presenter under one sky preset on a device the
    /// caller opened itself. `night` picks the arena night rig: the
    /// moonlit sky, its ambient and its fog, so the placement's torches
    /// are the light the frame reads. The day rig is the default and keeps
    /// every existing capture byte-equal.
    pub fn init_offscreen_from_device_with_preset(
        adapter: &wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
        night: bool,
    ) -> Result<Self, PresentError> {
        // EG13: the adapter owns the MSAA count; read its 4x support once.
        let support = Self::msaa_support_for(adapter);
        Self::build_with_preset(device, queue, adapter.get_info(), support, night)
    }

    /// What every presenter device is opened with: pass timestamps when the
    /// adapter supports them (the GPU timing rows stay `unsupported`
    /// otherwise and nothing else changes) and the default limits, which
    /// every WebGPU adapter grants. The one descriptor for the blocking
    /// native request below and for a caller that opens its device
    /// asynchronously (the browser shell) before `init_from_device`.
    #[must_use]
    pub fn device_descriptor(adapter: &wgpu::Adapter) -> wgpu::DeviceDescriptor<'static> {
        wgpu::DeviceDescriptor {
            label: Some("engine2040-ra1-present"),
            required_features: adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
            ..Default::default()
        }
    }

    /// Build the visible presenter on a device the caller already opened,
    /// without a single blocking call: the browser creates its surface,
    /// adapter and device asynchronously and hands the results here. `adapter`
    /// is the one `device` came from, `instance` the one that created
    /// `surface` (kept alive by the presenter). The native constructors above
    /// are unchanged. Zero area fails loudly like `init_windowed`. `uncapped`
    /// picks Immediate or Mailbox present mode instead of Fifo (vsync).
    #[expect(
        clippy::too_many_arguments,
        reason = "the browser's async handles plus size and pacing, one call"
    )]
    pub fn init_from_device(
        instance: &wgpu::Instance,
        adapter: &wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
        uncapped: bool,
    ) -> Result<Self, PresentError> {
        if SizeClass::classify(width, height) != SizeClass::Sizes {
            return Err(PresentError::InvalidSize { width, height });
        }
        let mut presenter = Self::init_offscreen_from_device(adapter, device, queue)?;
        presenter.attach_surface_for(instance, adapter, surface, width, height, uncapped)?;
        Ok(presenter)
    }

    /// Validation errors the device reported after the frame that caused them returned. A browser
    /// answers its error scopes late, so its frames cannot fail on them; this count (and
    /// [`Self::deferred_validation_first`]) is where they show up. Always 0 natively, where a
    /// frame fails at once with `PresentError::Validation`.
    #[must_use]
    pub fn deferred_validation_errors(&self) -> u64 {
        #[cfg(target_arch = "wasm32")]
        {
            self.scopes.errors()
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            0
        }
    }

    /// Scope answers still on their way and waits dropped because the device stopped answering
    /// (browser build; always `(0, 0)` natively). A page that prints this shows a frozen device
    /// at a glance.
    #[must_use]
    pub fn deferred_validation_waiting(&self) -> (usize, u64) {
        #[cfg(target_arch = "wasm32")]
        {
            (self.scopes.pending(), self.scopes.dropped())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            (0, 0)
        }
    }

    /// Text of the first late validation error, if one arrived.
    #[must_use]
    pub fn deferred_validation_first(&self) -> Option<&str> {
        #[cfg(target_arch = "wasm32")]
        {
            self.scopes.first_error()
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            None
        }
    }

    /// The device this presenter draws on, for a caller that owns its lifetime (the browser shell
    /// destroys it to prove the page survives a lost device).
    #[must_use]
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// Which adapter this presenter draws on, who asked for it, and whether
    /// that was a fallback (a software adapter, or a surface that hides the
    /// best one). [`Self::init_offscreen`] and [`Self::init_windowed`] carry
    /// the report of their own pick; a presenter built on a device the caller
    /// opened reports source `caller`. Receipts quote
    /// [`BackendSelection::line`](crate::adapter::BackendSelection::line).
    #[must_use]
    pub fn backend_selection(&self) -> &crate::adapter::BackendSelection {
        &self.backend_selection
    }

    /// Format of the attached swapchain, `None` before a surface is attached.
    #[must_use]
    pub fn surface_format(&self) -> Option<wgpu::TextureFormat> {
        self.surface_config.as_ref().map(|config| config.format)
    }
}
