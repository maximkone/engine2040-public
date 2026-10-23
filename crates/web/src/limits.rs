//! What the shell asks the browser's GPU for, how a device's answer is printed, and which
//! backend the page brings up when WebGPU is missing or too weak.

use wgpu::{Features, Limits};

/// The limits every `request_device` in `engine-render` opens its device with: the WebGPU
/// defaults (`..Default::default()` there). The shadow cluster bind needs three storage
/// buffers per stage; the defaults give eight, which `webgpu_limits.rs` in the render crate
/// holds every shipped shader to. A browser can always grant these, so a failed request means
/// the adapter is not a real WebGPU one.
#[must_use]
pub fn required_limits() -> Limits {
    Limits::defaults()
}

/// Optional features, as the render presenter takes them: pass timestamps only when the
/// adapter has them. Nothing else is asked for, so the shell opens on any WebGPU adapter.
#[must_use]
pub fn wanted_features(adapter: Features) -> Features {
    adapter & Features::TIMESTAMP_QUERY
}

/// Which graphics backend the page runs on. A probe picks it before the first frame: WebGPU
/// first, WebGL2 when the browser has no WebGPU or its adapter is below the required limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// `navigator.gpu` and an adapter that meets the WebGPU defaults.
    WebGpu,
    /// The canvas' WebGL2 context: the fallback that keeps the page running.
    WebGl2,
}

impl Backend {
    /// The word the page receipt prints.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::WebGpu => "webgpu",
            Self::WebGl2 => "webgl2",
        }
    }
}

/// The adapter limits the probe reads: the few the renderer cannot run without. A browser
/// may report more than these; a lower value is what forces the WebGL2 fallback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdapterLimits {
    /// Storage buffers one shader stage may bind; the shadow cluster bind needs three.
    pub max_storage_buffers_per_shader_stage: u32,
    /// Storage textures one shader stage may bind.
    pub max_storage_textures_per_shader_stage: u32,
    /// Bind groups one pipeline layout may use.
    pub max_bind_groups: u32,
    /// Longest texture side, which caps the canvas backing store.
    pub max_texture_dimension_2d: u32,
    /// Compute invocations in one workgroup.
    pub max_compute_invocations_per_workgroup: u32,
}

impl AdapterLimits {
    /// The WebGPU defaults: the same numbers [`required_limits`] opens a device with. Every
    /// browser adapter must meet them, so a lower one is not a full WebGPU adapter.
    #[must_use]
    pub fn webgpu_defaults() -> Self {
        Self::from_limits(&Limits::defaults())
    }

    /// The probe's numbers out of a wgpu [`Limits`] (a device's answer, or a test's fixture).
    #[must_use]
    pub fn from_limits(l: &Limits) -> Self {
        Self {
            max_storage_buffers_per_shader_stage: l.max_storage_buffers_per_shader_stage,
            max_storage_textures_per_shader_stage: l.max_storage_textures_per_shader_stage,
            max_bind_groups: l.max_bind_groups,
            max_texture_dimension_2d: l.max_texture_dimension_2d,
            max_compute_invocations_per_workgroup: l.max_compute_invocations_per_workgroup,
        }
    }

    /// The first limit below its WebGPU default, as `(WebGPU name, have, need)`. The order is
    /// fixed, so one adapter always names the same limit in the page receipt.
    #[must_use]
    pub fn first_below(&self) -> Option<(&'static str, u32, u32)> {
        let need = Self::webgpu_defaults();
        [
            (
                "maxStorageBuffersPerShaderStage",
                self.max_storage_buffers_per_shader_stage,
                need.max_storage_buffers_per_shader_stage,
            ),
            (
                "maxStorageTexturesPerShaderStage",
                self.max_storage_textures_per_shader_stage,
                need.max_storage_textures_per_shader_stage,
            ),
            ("maxBindGroups", self.max_bind_groups, need.max_bind_groups),
            (
                "maxTextureDimension2D",
                self.max_texture_dimension_2d,
                need.max_texture_dimension_2d,
            ),
            (
                "maxComputeInvocationsPerWorkgroup",
                self.max_compute_invocations_per_workgroup,
                need.max_compute_invocations_per_workgroup,
            ),
        ]
        .into_iter()
        .find(|(_, have, need)| have < need)
    }
}

/// What the page could see about WebGPU when the probe ran, before the first frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Probe {
    /// `navigator.gpu` is missing: an old browser, or a page that is not https or localhost.
    NoNavigatorGpu,
    /// WebGPU is here and its adapter request failed. `name` is the browser's error name
    /// (`OperationError`), or `null` when the browser answered with no adapter at all.
    AdapterFailed {
        /// The browser's own word for the failure.
        name: String,
    },
    /// The browser answered with an adapter and these limits.
    Adapter(AdapterLimits),
    /// The adapter meets the limits but the canvas gave no `webgpu` context, so WebGPU cannot
    /// draw: a browser with the API on and the canvas compositor off, as headless Chrome can be.
    /// The probe asks this last and only when the limits are met: a canvas that holds a `webgpu`
    /// context can never then give the `webgl2` one the fallback needs.
    NoCanvasContext,
}

/// What the probe learned about the adapter's limits, as the receipt prints it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitState {
    /// No adapter answered, so nothing is known.
    Unknown,
    /// The adapter met every WebGPU default.
    Met,
    /// The first default the adapter is below, by WebGPU name.
    Below {
        /// The WebGPU limit name.
        name: &'static str,
        /// What the adapter reports.
        have: u32,
        /// The WebGPU default the renderer needs.
        need: u32,
    },
}

/// The probe's answer: which backend to bring up, why, and what the limits were.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    /// The backend the page starts.
    pub backend: Backend,
    /// Why, in the words the page receipt prints.
    pub reason: String,
    /// The adapter's limits against the WebGPU defaults.
    pub limits: LimitState,
}

impl Choice {
    /// The page receipt line: one line, stable keys in this order, the reason quoted so a
    /// value with spaces stays one field. `limits=ok` when the adapter met the defaults,
    /// `limits=unknown` when no adapter answered, and `limits=<name>:<have><<need>` for the
    /// limit that forced WebGL2.
    #[must_use]
    pub fn receipt_line(&self) -> String {
        let limits = match self.limits {
            LimitState::Unknown => "unknown".to_string(),
            LimitState::Met => "ok".to_string(),
            LimitState::Below { name, have, need } => format!("{name}:{have}<{need}"),
        };
        format!(
            "backend={} reason=\"{}\" limits={limits}",
            self.backend.name(),
            self.reason
        )
    }
}

/// The backend decision: WebGPU when the page has it and its adapter meets the WebGPU
/// defaults, WebGL2 otherwise, with the reason the page receipt prints.
#[must_use]
pub fn choose(probe: &Probe) -> Choice {
    match probe {
        Probe::NoNavigatorGpu => Choice {
            backend: Backend::WebGl2,
            reason: "no navigator.gpu".to_string(),
            limits: LimitState::Unknown,
        },
        Probe::AdapterFailed { name } => Choice {
            backend: Backend::WebGl2,
            reason: format!("adapter request failed: {name}"),
            limits: LimitState::Unknown,
        },
        Probe::NoCanvasContext => Choice {
            backend: Backend::WebGl2,
            reason: "the canvas gave no webgpu context".to_string(),
            // The probe only gets here on an adapter that met the defaults.
            limits: LimitState::Met,
        },
        Probe::Adapter(limits) => match limits.first_below() {
            Some((name, have, need)) => Choice {
                backend: Backend::WebGl2,
                reason: format!("limit {name} below {need}"),
                limits: LimitState::Below { name, have, need },
            },
            None => Choice {
                backend: Backend::WebGpu,
                reason: "navigator.gpu and an adapter that meets the required limits".to_string(),
                limits: LimitState::Met,
            },
        },
    }
}

/// Longest `adapter=` text the page prints.
const ADAPTER_HINT_MAX: usize = 64;

/// The adapter as the page prints it, `adapter=<text>` (QT3): the browser's vendor, architecture,
/// device and description (`raw`, in that order) joined with `/`, blanks left out, spaces turned
/// into `_` so the text stays one word of a `key=value` line, cut at 64 characters. `none` when
/// the browser gives nothing (it may blank them so a card cannot be told from another). This is
/// a hint for a person reading the receipt: no tier is picked from it, except that the denylist
/// (`engine_render::quality::apply_denylist`) matches fragments against this same text.
#[must_use]
pub fn adapter_hint(raw: &[String; 4]) -> String {
    let parts: Vec<String> = raw
        .iter()
        .map(|part| part.split_whitespace().collect::<Vec<_>>().join("_"))
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        return "none".to_string();
    }
    parts
        .join("/")
        .chars()
        .filter(|c| !c.is_control())
        .take(ADAPTER_HINT_MAX)
        .collect()
}

/// A feature name the way WebGPU spells it (`TIMESTAMP_QUERY` is `timestamp-query`).
#[must_use]
pub fn feature_name(rust_name: &str) -> String {
    rust_name.to_ascii_lowercase().replace('_', "-")
}

/// The names of the features in `features`, in WebGPU spelling.
#[must_use]
pub fn feature_names(features: Features) -> Vec<String> {
    features
        .iter_names()
        .map(|(name, _)| feature_name(name))
        .collect()
}

/// Every limit WebGPU defines, under its WebGPU name, as a number (the 64-bit ones fit a
/// JSON number up to 2^53, far above any value a browser reports).
#[must_use]
pub fn limit_rows(l: &Limits) -> Vec<(&'static str, u64)> {
    vec![
        (
            "maxTextureDimension1D",
            u64::from(l.max_texture_dimension_1d),
        ),
        (
            "maxTextureDimension2D",
            u64::from(l.max_texture_dimension_2d),
        ),
        (
            "maxTextureDimension3D",
            u64::from(l.max_texture_dimension_3d),
        ),
        (
            "maxTextureArrayLayers",
            u64::from(l.max_texture_array_layers),
        ),
        ("maxBindGroups", u64::from(l.max_bind_groups)),
        (
            "maxBindGroupsPlusVertexBuffers",
            u64::from(l.max_bind_groups_plus_vertex_buffers),
        ),
        (
            "maxBindingsPerBindGroup",
            u64::from(l.max_bindings_per_bind_group),
        ),
        (
            "maxDynamicUniformBuffersPerPipelineLayout",
            u64::from(l.max_dynamic_uniform_buffers_per_pipeline_layout),
        ),
        (
            "maxDynamicStorageBuffersPerPipelineLayout",
            u64::from(l.max_dynamic_storage_buffers_per_pipeline_layout),
        ),
        (
            "maxSampledTexturesPerShaderStage",
            u64::from(l.max_sampled_textures_per_shader_stage),
        ),
        (
            "maxSamplersPerShaderStage",
            u64::from(l.max_samplers_per_shader_stage),
        ),
        (
            "maxStorageBuffersPerShaderStage",
            u64::from(l.max_storage_buffers_per_shader_stage),
        ),
        (
            "maxStorageTexturesPerShaderStage",
            u64::from(l.max_storage_textures_per_shader_stage),
        ),
        (
            "maxUniformBuffersPerShaderStage",
            u64::from(l.max_uniform_buffers_per_shader_stage),
        ),
        (
            "maxUniformBufferBindingSize",
            l.max_uniform_buffer_binding_size,
        ),
        (
            "maxStorageBufferBindingSize",
            l.max_storage_buffer_binding_size,
        ),
        (
            "minUniformBufferOffsetAlignment",
            u64::from(l.min_uniform_buffer_offset_alignment),
        ),
        (
            "minStorageBufferOffsetAlignment",
            u64::from(l.min_storage_buffer_offset_alignment),
        ),
        ("maxVertexBuffers", u64::from(l.max_vertex_buffers)),
        ("maxBufferSize", l.max_buffer_size),
        ("maxVertexAttributes", u64::from(l.max_vertex_attributes)),
        (
            "maxVertexBufferArrayStride",
            u64::from(l.max_vertex_buffer_array_stride),
        ),
        (
            "maxInterStageShaderVariables",
            u64::from(l.max_inter_stage_shader_variables),
        ),
        ("maxColorAttachments", u64::from(l.max_color_attachments)),
        (
            "maxColorAttachmentBytesPerSample",
            u64::from(l.max_color_attachment_bytes_per_sample),
        ),
        (
            "maxComputeWorkgroupStorageSize",
            u64::from(l.max_compute_workgroup_storage_size),
        ),
        (
            "maxComputeInvocationsPerWorkgroup",
            u64::from(l.max_compute_invocations_per_workgroup),
        ),
        (
            "maxComputeWorkgroupSizeX",
            u64::from(l.max_compute_workgroup_size_x),
        ),
        (
            "maxComputeWorkgroupSizeY",
            u64::from(l.max_compute_workgroup_size_y),
        ),
        (
            "maxComputeWorkgroupSizeZ",
            u64::from(l.max_compute_workgroup_size_z),
        ),
        (
            "maxComputeWorkgroupsPerDimension",
            u64::from(l.max_compute_workgroups_per_dimension),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(rows: &[(&'static str, u64)], name: &str) -> u64 {
        rows.iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| *v)
            .unwrap_or_else(|| panic!("limit {name} is not printed"))
    }

    #[test]
    fn required_limits_are_the_webgpu_defaults_the_renderer_opens_with() {
        let rows = limit_rows(&required_limits());
        // The shadow cluster bind needs three; WebGPU promises eight everywhere.
        assert_eq!(row(&rows, "maxStorageBuffersPerShaderStage"), 8);
        assert_eq!(row(&rows, "maxBindGroups"), 4);
        assert_eq!(row(&rows, "maxStorageTexturesPerShaderStage"), 4);
        assert_eq!(row(&rows, "maxTextureDimension2D"), 8192);
        assert_eq!(row(&rows, "maxComputeInvocationsPerWorkgroup"), 256);
    }

    #[test]
    fn webgl2_class_limits_would_fail_the_renderer_need() {
        // The look this shell stands for has compute passes; the downlevel set has none.
        let weak = limit_rows(&Limits::downlevel_webgl2_defaults());
        assert!(row(&weak, "maxStorageBuffersPerShaderStage") < 3);
        let ours = limit_rows(&required_limits());
        assert!(row(&ours, "maxStorageBuffersPerShaderStage") >= 3);
    }

    #[test]
    fn every_row_has_a_distinct_webgpu_name() {
        let rows = limit_rows(&required_limits());
        let mut names: Vec<_> = rows.iter().map(|(n, _)| *n).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before);
        assert_eq!(before, 31);
        assert!(names
            .iter()
            .all(|n| n.starts_with("max") || n.starts_with("min")));
    }

    fn raw(vendor: &str, architecture: &str, device: &str, description: &str) -> [String; 4] {
        [vendor, architecture, device, description].map(str::to_string)
    }

    #[test]
    fn a_blanked_adapter_prints_none() {
        assert_eq!(adapter_hint(&raw("", "", "", "")), "none");
        assert_eq!(adapter_hint(&raw("  ", "\t", "", "\n")), "none");
        assert_eq!(adapter_hint(&Default::default()), "none");
    }

    #[test]
    fn the_adapter_hint_is_one_word_of_the_receipt_line() {
        assert_eq!(
            adapter_hint(&raw("google", "swiftshader", "", "SwiftShader Device")),
            "google/swiftshader/SwiftShader_Device"
        );
        assert_eq!(adapter_hint(&raw("nvidia", "", "", "")), "nvidia");
        let hint = adapter_hint(&raw("a  b", "c\td", "", "e\u{0}f"));
        assert_eq!(hint, "a_b/c_d/ef");
        assert!(!hint.contains(char::is_whitespace));
        let long = adapter_hint(&raw("", "", "", &"x".repeat(500)));
        assert_eq!(long.chars().count(), ADAPTER_HINT_MAX);
    }

    #[test]
    fn features_print_in_webgpu_spelling() {
        assert_eq!(feature_name("TIMESTAMP_QUERY"), "timestamp-query");
        assert_eq!(feature_name("SHADER_F16"), "shader-f16");
        let names = feature_names(Features::TIMESTAMP_QUERY);
        assert_eq!(names, vec!["timestamp-query".to_string()]);
        assert!(feature_names(Features::empty()).is_empty());
    }

    #[test]
    fn only_timestamps_are_wanted_and_only_when_the_adapter_has_them() {
        assert_eq!(
            wanted_features(Features::TIMESTAMP_QUERY | Features::SHADER_F16),
            Features::TIMESTAMP_QUERY
        );
        assert_eq!(wanted_features(Features::SHADER_F16), Features::empty());
        assert_eq!(wanted_features(Features::empty()), Features::empty());
    }

    #[test]
    fn the_probe_limits_are_the_webgpu_defaults_the_shell_asks_for() {
        let defaults = AdapterLimits::webgpu_defaults();
        assert_eq!(defaults, AdapterLimits::from_limits(&required_limits()));
        assert_eq!(defaults.first_below(), None);
        // A WebGL2-class adapter is below them: that is what forces the fallback.
        let weak = AdapterLimits::from_limits(&Limits::downlevel_webgl2_defaults());
        assert!(weak.first_below().is_some());
    }

    #[test]
    fn webgpu_is_chosen_when_the_adapter_meets_the_limits() {
        let choice = choose(&Probe::Adapter(AdapterLimits::webgpu_defaults()));
        assert_eq!(choice.backend, Backend::WebGpu);
        assert_eq!(
            choice.reason,
            "navigator.gpu and an adapter that meets the required limits"
        );
        assert_eq!(choice.limits, LimitState::Met);
        assert_eq!(
            choice.receipt_line(),
            "backend=webgpu reason=\"navigator.gpu and an adapter that meets the required limits\" limits=ok"
        );
    }

    #[test]
    fn no_navigator_gpu_falls_back_to_webgl2_and_says_why() {
        let choice = choose(&Probe::NoNavigatorGpu);
        assert_eq!(choice.backend, Backend::WebGl2);
        assert_eq!(choice.reason, "no navigator.gpu");
        assert_eq!(
            choice.receipt_line(),
            "backend=webgl2 reason=\"no navigator.gpu\" limits=unknown"
        );
    }

    #[test]
    fn a_failed_adapter_request_falls_back_with_the_browsers_name() {
        let choice = choose(&Probe::AdapterFailed {
            name: "OperationError".to_string(),
        });
        assert_eq!(choice.backend, Backend::WebGl2);
        assert_eq!(choice.reason, "adapter request failed: OperationError");
        assert_eq!(
            choice.receipt_line(),
            "backend=webgl2 reason=\"adapter request failed: OperationError\" limits=unknown"
        );
        // A browser that answers with no adapter at all prints that word, never a blank.
        let none = choose(&Probe::AdapterFailed {
            name: "null".to_string(),
        });
        assert!(none.receipt_line().contains("adapter request failed: null"));
    }

    #[test]
    fn a_low_limit_falls_back_and_the_line_quotes_it() {
        let mut limits = AdapterLimits::webgpu_defaults();
        limits.max_storage_buffers_per_shader_stage = 2;
        let choice = choose(&Probe::Adapter(limits));
        assert_eq!(choice.backend, Backend::WebGl2);
        assert_eq!(
            choice.reason,
            "limit maxStorageBuffersPerShaderStage below 8"
        );
        assert_eq!(
            choice.limits,
            LimitState::Below {
                name: "maxStorageBuffersPerShaderStage",
                have: 2,
                need: 8
            }
        );
        assert_eq!(
            choice.receipt_line(),
            "backend=webgl2 reason=\"limit maxStorageBuffersPerShaderStage below 8\" limits=maxStorageBuffersPerShaderStage:2<8"
        );
    }

    #[test]
    fn the_first_low_limit_is_named_in_a_fixed_order() {
        let mut limits = AdapterLimits::webgpu_defaults();
        limits.max_texture_dimension_2d = 4096;
        limits.max_bind_groups = 2;
        assert_eq!(limits.first_below(), Some(("maxBindGroups", 2, 4)));
        limits.max_bind_groups = 4;
        assert_eq!(
            limits.first_below(),
            Some(("maxTextureDimension2D", 4096, 8192))
        );
        limits.max_texture_dimension_2d = 8192;
        assert_eq!(limits.first_below(), None);
    }

    #[test]
    fn a_canvas_without_a_webgpu_context_falls_back_and_says_so() {
        let choice = choose(&Probe::NoCanvasContext);
        assert_eq!(choice.backend, Backend::WebGl2);
        assert_eq!(choice.reason, "the canvas gave no webgpu context");
        // The adapter met the defaults; only the canvas refused.
        assert_eq!(choice.limits, LimitState::Met);
        assert_eq!(
            choice.receipt_line(),
            "backend=webgl2 reason=\"the canvas gave no webgpu context\" limits=ok"
        );
    }

    #[test]
    fn every_receipt_line_is_one_parseable_line() {
        let mut low = AdapterLimits::webgpu_defaults();
        low.max_compute_invocations_per_workgroup = 128;
        for probe in [
            Probe::NoNavigatorGpu,
            Probe::AdapterFailed {
                name: "OperationError".to_string(),
            },
            Probe::NoCanvasContext,
            Probe::Adapter(low),
            Probe::Adapter(AdapterLimits::webgpu_defaults()),
        ] {
            let choice = choose(&probe);
            let line = choice.receipt_line();
            assert!(!line.contains('\n') && !line.contains('\r'), "{line}");
            assert!(line.starts_with("backend="), "{line}");
            assert_eq!(line.matches("backend=").count(), 1, "{line}");
            assert_eq!(line.matches(" reason=\"").count(), 1, "{line}");
            assert_eq!(line.matches("\" limits=").count(), 1, "{line}");
            assert!(
                line.contains(&format!("backend={}", choice.backend.name())),
                "{line}"
            );
            assert!(
                line.contains(&format!("reason=\"{}\"", choice.reason)),
                "{line}"
            );
        }
    }
}
