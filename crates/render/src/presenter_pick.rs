//! presenter_pick: wgpu glue over the `adapter` choice (GS6).
//!
//! Owner: render lane. Behind `window` so the headless default stays
//! std-only. Lifts enumerated wgpu adapters into [`adapter`](crate::adapter)
//! summaries and picks one on purpose. Cold startup path only;
//! nothing here runs per frame.
//! Consumer: `Presenter::init_offscreen` (through [`pick_adapter`]) and
//! `Presenter::init_windowed` (through `crate::adapter::select`), which keep
//! the [`BackendSelection`] on the presenter.

use crate::adapter::{
    AdapterBackend, AdapterError, AdapterKind, AdapterSummary, BackendSelection, SelectionError,
    SelectionRequest,
};

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

/// Rank `infos` through [`crate::adapter::select`] (no surface mask) and
/// return the winning index with its report. Cold startup path only.
pub fn select_over_infos(
    infos: &[wgpu::AdapterInfo],
    request: &SelectionRequest,
) -> Result<(usize, BackendSelection), SelectionError> {
    let list: Vec<AdapterSummary> = infos.iter().map(summary_of).collect();
    crate::adapter::select(&list, None, request)
}

/// Enumerate `instance` and return the chosen adapter with the report of how
/// it was chosen. Async so callers `pollster::block_on` it like the existing
/// request sites. A pick failure (empty list, software-only, no match) or a
/// refused fallback under `E2040_REQUIRE_GPU=1` is typed, never silent.
pub async fn pick_adapter(
    instance: &wgpu::Instance,
    backends: wgpu::Backends,
    request: &SelectionRequest,
) -> Result<(wgpu::Adapter, BackendSelection), SelectionError> {
    let adapters = instance.enumerate_adapters(backends).await;
    let infos: Vec<wgpu::AdapterInfo> = adapters.iter().map(|found| found.get_info()).collect();
    let (index, report) = select_over_infos(&infos, request)?;
    let adapter = adapters
        .into_iter()
        .nth(index)
        .ok_or(SelectionError::Pick(AdapterError::Empty))?;
    Ok((adapter, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{AdapterChoice, SelectionPolicy, SelectionSource};

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

    fn request(choice: AdapterChoice, allow_software: bool) -> SelectionRequest {
        SelectionRequest {
            choice,
            source: SelectionSource::Default,
            policy: SelectionPolicy::Prefer,
            allow_software,
        }
    }

    fn picked(list: &[wgpu::AdapterInfo], choice: AdapterChoice) -> Option<usize> {
        select_over_infos(list, &request(choice, false))
            .ok()
            .map(|(index, _)| index)
    }

    #[test]
    fn pick_prefers_discrete_and_names_integrated() {
        let list = canned();
        assert_eq!(picked(&list, AdapterChoice::Auto), Some(1));
        assert_eq!(picked(&list, AdapterChoice::Integrated), Some(0));
        assert_eq!(
            picked(&list, AdapterChoice::Name("uhd".to_string())),
            Some(0)
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
            select_over_infos(&only, &request(AdapterChoice::Auto, false)),
            Err(SelectionError::Pick(AdapterError::OnlySoftware(
                "Basic Render Driver".to_string()
            )))
        );
        let (_, report) = select_over_infos(&only, &request(AdapterChoice::Auto, true))
            .expect("opt-in lets the software adapter win");
        assert!(report.fallback_used, "and the report calls it a fallback");
    }

    /// The offscreen bring-up keeps the report of its own pick: the adapter
    /// the presenter drew on is the one the report names, and the report line
    /// is written to stderr like the strict-WebGPU test does.
    #[test]
    fn offscreen_presenter_keeps_the_report_of_its_pick() {
        use crate::present::{PresentError, Presenter};
        use crate::test_gpu::{require_or_skip, NoAdapter};
        use std::io::Write;
        let probe = match Presenter::init_offscreen() {
            Ok(presenter) => Ok(presenter),
            Err(PresentError::NoAdapter) => Err(NoAdapter(
                "no acceptable wgpu adapter on this machine".to_string(),
            )),
            Err(other) => panic!("presenter init failed (must fail loudly): {other}"),
        };
        let Some(presenter) =
            require_or_skip("offscreen_presenter_keeps_the_report_of_its_pick", probe)
        else {
            return;
        };
        let report = presenter.backend_selection();
        assert_eq!(report.selected.name, presenter.adapter_name());
        assert_eq!(report.source, SelectionSource::from_env());
        assert_eq!(report.requested, Some(crate::adapter::choice_from_env()));
        assert_eq!(
            report.fallback_used,
            report.selected.kind == AdapterKind::Cpu
        );
        let line = report.line();
        assert!(line.starts_with("backend: source="), "{line}");
        assert!(line.contains(" fallback="), "{line}");
        let _ = writeln!(std::io::stderr(), "{line}");
    }

    /// The pair `pick_adapter` hands back agrees with itself: the report
    /// names the adapter it returned.
    #[test]
    fn pick_adapter_report_names_the_returned_adapter() {
        use crate::test_gpu::{require_or_skip, NoAdapter};
        crate::test_gpu::hold_gpu_slot();
        // PRIMARY: test builds skip GL, whose driver DLL can wedge the loader lock.
        let backends = wgpu::Backends::PRIMARY;
        let instance = crate::backend::new_instance(backends);
        let probe = pollster::block_on(pick_adapter(
            &instance,
            backends,
            &SelectionRequest::from_env(),
        ))
        .map_err(|error| NoAdapter(error.to_string()));
        let Some((adapter, report)) =
            require_or_skip("pick_adapter_report_names_the_returned_adapter", probe)
        else {
            return;
        };
        assert_eq!(report.selected, summary_of(&adapter.get_info()));
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
