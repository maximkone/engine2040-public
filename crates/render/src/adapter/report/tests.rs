//! Tests for the parent module.

use super::*;
use crate::adapter::AdapterBackend;

fn row(name: &str, backend: AdapterBackend, kind: AdapterKind) -> AdapterSummary {
    AdapterSummary {
        name: name.to_string(),
        backend,
        kind,
        vendor: 0,
    }
}

/// The Optimus laptop plus the CPU rasterizer.
fn canned() -> Vec<AdapterSummary> {
    vec![
        row(
            "Intel UHD Graphics",
            AdapterBackend::Dx12,
            AdapterKind::Integrated,
        ),
        row(
            "NVIDIA GeForce RTX 3050 Laptop GPU",
            AdapterBackend::Dx12,
            AdapterKind::Discrete,
        ),
        row(
            "Basic Render Driver",
            AdapterBackend::Dx12,
            AdapterKind::Cpu,
        ),
    ]
}

fn software_only() -> Vec<AdapterSummary> {
    vec![row(
        "Basic Render Driver",
        AdapterBackend::Dx12,
        AdapterKind::Cpu,
    )]
}

fn request(choice: AdapterChoice, policy: SelectionPolicy, allow: bool) -> SelectionRequest {
    SelectionRequest {
        choice,
        source: SelectionSource::Default,
        policy,
        allow_software: allow,
    }
}

#[test]
fn auto_names_the_discrete_gpu_on_one_stable_line() {
    let req = request(AdapterChoice::Auto, SelectionPolicy::Prefer, false);
    let (index, report) = select(&canned(), None, &req).expect("auto picks");
    assert_eq!(index, 1);
    assert!(!report.fallback_used);
    assert_eq!(
        report.line(),
        "backend: source=default requested=auto selected=\"NVIDIA GeForce RTX 3050 Laptop GPU\" api=dx12 kind=discrete fallback=no reason=\"best hardware adapter (discrete first, then dx12, vulkan, gl)\" remedy=\"none\""
    );
}

#[test]
fn an_env_request_is_named_and_found() {
    let req = SelectionRequest {
        source: SelectionSource::Env,
        ..request(AdapterChoice::Integrated, SelectionPolicy::Strict, false)
    };
    let (index, report) = select(&canned(), None, &req).expect("integrated exists");
    assert_eq!(index, 0);
    let line = report.line();
    assert!(line.starts_with("backend: source=env requested=integrated "));
    assert!(line.contains("kind=integrated fallback=no"));
    assert!(line.contains("reason=\"the requested adapter was found\""));
}

#[test]
fn a_name_request_prints_its_quoted_fragment() {
    let req = request(
        AdapterChoice::Name("RTX 3050".to_string()),
        SelectionPolicy::Prefer,
        false,
    );
    let (_, report) = select(&canned(), None, &req).expect("fragment matches");
    assert!(report
        .line()
        .contains("requested=\"name:RTX 3050\" selected=\"NVIDIA"));
}

#[test]
fn prefer_names_a_software_fallback_and_the_remedy() {
    let req = request(AdapterChoice::Auto, SelectionPolicy::Prefer, true);
    let (_, report) = select(&software_only(), None, &req).expect("opt-in allows it");
    assert!(report.fallback_used);
    assert_eq!(report.selected.kind, AdapterKind::Cpu);
    let line = report.line();
    assert!(line.contains("kind=cpu fallback=yes"), "{line}");
    assert!(line.contains("unset E2040_ALLOW_SOFTWARE"), "{line}");
}

#[test]
fn strict_refuses_the_software_fallback_with_the_remedy() {
    let req = request(AdapterChoice::Auto, SelectionPolicy::Strict, true);
    let error = select(&software_only(), None, &req).expect_err("strict refuses software");
    assert_eq!(
        error,
        SelectionError::Fallback {
            selected: "Basic Render Driver".to_string(),
            reason: REASON_SOFTWARE,
            remedy: REMEDY_SOFTWARE,
        }
    );
    let text = error.to_string();
    assert!(text.contains("Basic Render Driver"), "{text}");
    assert!(
        text.contains("remedy: run on a machine with a GPU driver"),
        "{text}"
    );
}

#[test]
fn a_failed_pick_carries_its_remedy() {
    let auto = request(AdapterChoice::Auto, SelectionPolicy::Prefer, false);
    let only = select(&software_only(), None, &auto).expect_err("software refused");
    assert_eq!(
        only,
        SelectionError::Pick(AdapterError::OnlySoftware(
            "Basic Render Driver".to_string()
        ))
    );
    assert!(only.to_string().contains("set E2040_ALLOW_SOFTWARE=1"));
    let radeon = request(
        AdapterChoice::Name("radeon".to_string()),
        SelectionPolicy::Prefer,
        false,
    );
    let none = select(&canned(), None, &radeon).expect_err("no radeon");
    assert!(none.to_string().contains("radeon"));
    assert!(none.to_string().contains("unset E2040_ADAPTER"));
    let empty = select(&[], None, &auto).expect_err("nothing enumerated");
    assert_eq!(empty, SelectionError::Pick(AdapterError::Empty));
    assert!(empty.to_string().contains("install a GPU driver"));
    assert!(empty.source().is_some());
}

#[test]
fn a_surface_that_hides_the_best_adapter_is_a_fallback() {
    let prefer = request(AdapterChoice::Auto, SelectionPolicy::Prefer, false);
    let mask = [true, false, true];
    let (index, report) = select(&canned(), Some(&mask), &prefer).expect("integrated presents");
    assert_eq!(index, 0);
    assert!(report.fallback_used);
    assert_eq!(report.reason, REASON_SURFACE);
    let strict = request(AdapterChoice::Auto, SelectionPolicy::Strict, false);
    assert!(matches!(
        select(&canned(), Some(&mask), &strict),
        Err(SelectionError::Fallback { .. })
    ));
}

#[test]
fn a_surface_that_fits_the_best_adapter_is_no_fallback() {
    let strict = request(AdapterChoice::Auto, SelectionPolicy::Strict, false);
    let mask = [true, true, true];
    let (index, report) = select(&canned(), Some(&mask), &strict).expect("discrete presents");
    assert_eq!(index, 1);
    assert!(!report.fallback_used);
}

#[test]
fn a_caller_opened_adapter_has_no_request_and_no_fallback() {
    let report = BackendSelection::caller_opened(canned()[1].clone());
    assert!(!report.fallback_used);
    assert!(report.requested.is_none());
    assert!(report
        .line()
        .starts_with("backend: source=caller requested=none selected=\"NVIDIA"));
}

#[test]
fn a_hostile_adapter_name_cannot_split_the_line() {
    let list = vec![row(
        "Evil \"x\"\nbackend: forged",
        AdapterBackend::Vulkan,
        AdapterKind::Discrete,
    )];
    let req = request(AdapterChoice::Auto, SelectionPolicy::Prefer, false);
    let (_, report) = select(&list, None, &req).expect("hardware row");
    let line = report.line();
    assert!(!line.contains('\n'), "{line}");
    assert_eq!(line.matches("backend:").count(), 2, "{line}");
    assert!(
        line.contains("selected=\"Evil 'x' backend: forged\""),
        "{line}"
    );
}

#[test]
fn source_and_policy_come_from_plain_values() {
    assert_eq!(SelectionSource::from_raw(None), SelectionSource::Default);
    assert_eq!(
        SelectionSource::from_raw(Some("")),
        SelectionSource::Default
    );
    assert_eq!(
        SelectionSource::from_raw(Some("  ")),
        SelectionSource::Default
    );
    assert_eq!(
        SelectionSource::from_raw(Some("auto")),
        SelectionSource::Env
    );
    assert_eq!(
        SelectionPolicy::from_required(true),
        SelectionPolicy::Strict
    );
    assert_eq!(
        SelectionPolicy::from_required(false),
        SelectionPolicy::Prefer
    );
}
