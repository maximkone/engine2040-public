//! What the shell asks the browser's GPU for, and how a device's answer is printed.

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
}
