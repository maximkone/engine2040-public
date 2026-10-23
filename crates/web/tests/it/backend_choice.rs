//! The probe's decision table, black box: which backend the page brings up, why, and the one
//! receipt line it prints before the first frame.

use engine_web::limits::{choose, AdapterLimits, Backend, LimitState, Probe};

/// One limit's name with the edit that puts it below its WebGPU default.
type LowRow = (&'static str, fn(&mut AdapterLimits));

/// Each limit the probe reads, with the edit that puts it below its WebGPU default. The order
/// is the order [`AdapterLimits::first_below`] names a low limit in.
const LOW_ROWS: [LowRow; 5] = [
    ("maxStorageBuffersPerShaderStage", |l| {
        l.max_storage_buffers_per_shader_stage = 1;
    }),
    ("maxStorageTexturesPerShaderStage", |l| {
        l.max_storage_textures_per_shader_stage = 1;
    }),
    ("maxBindGroups", |l| l.max_bind_groups = 1),
    ("maxTextureDimension2D", |l| {
        l.max_texture_dimension_2d = 512;
    }),
    ("maxComputeInvocationsPerWorkgroup", |l| {
        l.max_compute_invocations_per_workgroup = 32;
    }),
];

/// The receipt line's three fields, split the way the page writes them: `backend=<word>
/// reason="<text>" limits=<text>`.
fn fields(line: &str) -> Option<(&str, &str, &str)> {
    let (backend, rest) = line.strip_prefix("backend=")?.split_once(" reason=\"")?;
    let (reason, limits) = rest.split_once("\" limits=")?;
    Some((backend, reason, limits))
}

#[test]
fn an_adapter_at_every_default_runs_on_webgpu() {
    let choice = choose(&Probe::Adapter(AdapterLimits::webgpu_defaults()));
    assert_eq!(choice.backend, Backend::WebGpu);
    assert_eq!(choice.limits, LimitState::Met);
    assert_eq!(
        fields(&choice.receipt_line()),
        Some((
            "webgpu",
            "navigator.gpu and an adapter that meets the required limits",
            "ok"
        ))
    );
}

#[test]
fn one_low_limit_sends_the_page_to_webgl2_and_the_line_quotes_it() {
    for (name, set_low) in LOW_ROWS {
        let mut limits = AdapterLimits::webgpu_defaults();
        set_low(&mut limits);
        let choice = choose(&Probe::Adapter(limits));
        assert_eq!(choice.backend, Backend::WebGl2, "{name}");
        let LimitState::Below {
            name: named,
            have,
            need,
        } = choice.limits
        else {
            panic!("{name}: a low limit must be named");
        };
        assert_eq!(named, name);
        assert!(have < need, "{name}: {have} is not below {need}");
        assert_eq!(choice.reason, format!("limit {name} below {need}"));
        assert_eq!(
            choice.receipt_line(),
            format!(
                "backend=webgl2 reason=\"limit {name} below {need}\" limits={name}:{have}<{need}"
            ),
            "{name}"
        );
    }
}

#[test]
fn the_first_low_limit_in_the_fixed_order_is_the_one_named() {
    for (i, (expect, _)) in LOW_ROWS.iter().enumerate() {
        let mut limits = AdapterLimits::webgpu_defaults();
        for (_, set_low) in LOW_ROWS.iter().skip(i) {
            set_low(&mut limits);
        }
        let named = limits.first_below().map(|(name, _, _)| name);
        assert_eq!(named, Some(*expect), "rows {i}.. are all low");
    }
}

#[test]
fn a_page_without_a_usable_webgpu_loads_webgl2_and_says_why() {
    for (probe, reason) in [
        (Probe::NoNavigatorGpu, "no navigator.gpu"),
        (
            Probe::AdapterFailed {
                name: "OperationError".to_string(),
            },
            "adapter request failed: OperationError",
        ),
        (
            Probe::AdapterFailed {
                name: "null".to_string(),
            },
            "adapter request failed: null",
        ),
        (Probe::NoCanvasContext, "the canvas gave no webgpu context"),
    ] {
        let choice = choose(&probe);
        assert_eq!(choice.backend, Backend::WebGl2);
        let met = choice.limits == LimitState::Met;
        assert_eq!(
            fields(&choice.receipt_line()),
            Some(("webgl2", reason, if met { "ok" } else { "unknown" }))
        );
    }
}

#[test]
fn every_choice_is_one_line_of_three_fields() {
    let mut all_low = AdapterLimits::webgpu_defaults();
    for (_, set_low) in LOW_ROWS {
        set_low(&mut all_low);
    }
    for probe in [
        Probe::NoNavigatorGpu,
        Probe::AdapterFailed {
            name: "null".to_string(),
        },
        Probe::NoCanvasContext,
        Probe::Adapter(all_low),
        Probe::Adapter(AdapterLimits::webgpu_defaults()),
    ] {
        let line = choose(&probe).receipt_line();
        assert!(!line.contains('\n') && !line.contains('\r'), "{line}");
        assert!(fields(&line).is_some(), "{line}");
        assert_eq!(line.matches("backend=").count(), 1, "{line}");
        assert_eq!(line.matches(" limits=").count(), 1, "{line}");
    }
}
