//! presenter_pick: wgpu glue over the `adapter` choice (GS6).
//!
//! Owner: render lane. Behind `window` so the headless default stays
//! std-only. Lifts enumerated wgpu adapters into [`adapter`](crate::adapter)
//! summaries and picks one on purpose. Cold startup path only;
//! nothing here runs per frame.
//! Consumer: the three `request_adapter` sites in
//! `present/gpu_present/presenter.rs`. Wiring lands with the presenter owner;
//! the exact replacement is:
//!
//! ```ignore
//! let choice = crate::adapter::choice_from_env();
//! let adapter = pollster::block_on(crate::presenter_pick::pick_adapter(
//!     &instance,
//!     crate::backend::wgpu_backends(pick.primary),
//!     &choice,
//! ))
//! .map_err(|_| PresentError::NoAdapter)?;
//! ```

use crate::adapter::{AdapterBackend, AdapterChoice, AdapterError, AdapterKind, AdapterSummary};

/// Lift one wgpu adapter description into the pure summary the pick ranks.
/// Virtual and unknown devices count as hardware; only `Cpu` is refused.
#[must_use]
pub fn summary_of(info: &wgpu::AdapterInfo) -> AdapterSummary {
    AdapterSummary {
        name: info.name.clone(),
        backend: match info.backend {
            wgpu::Backend::Dx12 => AdapterBackend::Dx12,
            wgpu::Backend::Vulkan => AdapterBackend::Vulkan,
            wgpu::Backend::Gl => AdapterBackend::Gl,
            _ => AdapterBackend::Other,
        },
        kind: match info.device_type {
            wgpu::DeviceType::DiscreteGpu => AdapterKind::Discrete,
            wgpu::DeviceType::IntegratedGpu => AdapterKind::Integrated,
            wgpu::DeviceType::Cpu => AdapterKind::Cpu,
            _ => AdapterKind::Other,
        },
        vendor: info.vendor,
    }
}

/// Rank `infos` through [`crate::adapter::choose`] and return the winning
/// index. Cold startup path only.
pub fn pick_over_infos(
    infos: &[wgpu::AdapterInfo],
    choice: &AdapterChoice,
) -> Result<usize, AdapterError> {
    let list: Vec<AdapterSummary> = infos.iter().map(summary_of).collect();
    crate::adapter::choose(&list, choice)
}

/// Enumerate `instance` and return the chosen adapter. Async so callers
/// `pollster::block_on` it like the existing request sites. A pick failure
/// (empty list, software-only, no match) is typed, never a silent fallback.
pub async fn pick_adapter(
    instance: &wgpu::Instance,
    backends: wgpu::Backends,
    choice: &AdapterChoice,
) -> Result<wgpu::Adapter, AdapterError> {
    let adapters = instance.enumerate_adapters(backends).await;
    let infos: Vec<wgpu::AdapterInfo> = adapters.iter().map(|found| found.get_info()).collect();
    let index = pick_over_infos(&infos, choice)?;
    adapters.into_iter().nth(index).ok_or(AdapterError::Empty)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(
        name: &str,
        backend: wgpu::Backend,
        kind: wgpu::DeviceType,
        vendor: u32,
    ) -> wgpu::AdapterInfo {
        let mut out = wgpu::AdapterInfo::new(kind, backend);
        out.name = name.to_string();
        out.vendor = vendor;
        out
    }

    /// The GS6 accept list as wgpu rows.
    fn canned() -> Vec<wgpu::AdapterInfo> {
        vec![
            info(
                "Intel UHD Graphics",
                wgpu::Backend::Dx12,
                wgpu::DeviceType::IntegratedGpu,
                0x8086,
            ),
            info(
                "NVIDIA GeForce RTX 3050",
                wgpu::Backend::Dx12,
                wgpu::DeviceType::DiscreteGpu,
                0x10de,
            ),
            info(
                "Basic Render Driver",
                wgpu::Backend::Dx12,
                wgpu::DeviceType::Cpu,
                0x1414,
            ),
        ]
    }

    #[test]
    fn pick_prefers_discrete_and_names_integrated() {
        let list = canned();
        assert_eq!(pick_over_infos(&list, &AdapterChoice::Auto), Ok(1));
        assert_eq!(pick_over_infos(&list, &AdapterChoice::Integrated), Ok(0));
        assert_eq!(
            pick_over_infos(&list, &AdapterChoice::Name("uhd".to_string())),
            Ok(0)
        );
    }

    #[test]
    fn pick_refuses_software_without_opt_in() {
        let only = vec![info(
            "Basic Render Driver",
            wgpu::Backend::Dx12,
            wgpu::DeviceType::Cpu,
            0x1414,
        )];
        assert_eq!(
            pick_over_infos(&only, &AdapterChoice::Auto),
            Err(AdapterError::OnlySoftware(
                "Basic Render Driver".to_string()
            ))
        );
    }

    #[test]
    fn summary_maps_wgpu_types() {
        let rows: Vec<AdapterSummary> = canned().iter().map(summary_of).collect();
        assert_eq!(
            (rows[0].kind, rows[0].backend),
            (AdapterKind::Integrated, AdapterBackend::Dx12)
        );
        assert_eq!(
            (rows[1].kind, rows[1].backend),
            (AdapterKind::Discrete, AdapterBackend::Dx12)
        );
        assert_eq!(
            rows[1].describe(),
            "adapter: NVIDIA GeForce RTX 3050 dx12 discrete vendor=0x10de"
        );
        let gl = info("Mesa", wgpu::Backend::Gl, wgpu::DeviceType::Other, 0);
        assert_eq!(
            (summary_of(&gl).kind, summary_of(&gl).backend),
            (AdapterKind::Other, AdapterBackend::Gl)
        );
    }
}
