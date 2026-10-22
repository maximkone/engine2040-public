//! CPU naga gate over the shipped WGSL (CQ23): every file under `src/shaders`
//! parses and validates without a GPU, so a type error fails a test or a hot
//! edit instead of a pipeline build.
//!
//! The default headless graph stays std-only: naga rides only
//! `shader-validate`, through the material crate's one validator owner
//! (`engine_material::compile::validate`). Consumers: the window's hot-reload
//! cook (`engine_game::reload::cook_bytes`) refuses an edit that does not
//! check and names its line:col, so the live shader keeps last-good; this
//! module's tests walk the shipped dir plus the inline chunks that build
//! without `window`.

/// Shipped shader directory; [`shipped_files`] and the tests walk it.
pub const SHADER_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/shaders");

/// The display transform the frame-ending files are compiled with
/// prepended: `v1_gpu.rs:34` `FORWARD_PBR_WGSL`,
/// `present/gpu_present.rs:38` `BLIT_WGSL`, `post/gpu.rs:14`
/// `POST_CHAIN_WGSL` and `post/gpu.rs:119` `COMPOSITE_COMPUTE_WGSL` are
/// each `concat!` of this chunk and their file.
const TONEMAP_CHUNK: &str = include_str!("shaders/tonemap.wgsl");

/// One source as the pipelines compile it: `forward.wgsl`, `blit.wgsl` and
/// the two post shaders take [`TONEMAP_CHUNK`] prepended (they call
/// `aces_tonemap`), every other file compiles alone.
fn shipped_source<'a>(name: &str, wgsl: &'a str) -> std::borrow::Cow<'a, str> {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    match file {
        "forward.wgsl" | "blit.wgsl" | "post_chain.wgsl" | "composite_compute.wgsl" => {
            std::borrow::Cow::Owned(format!("{TONEMAP_CHUNK}{wgsl}"))
        }
        _ => std::borrow::Cow::Borrowed(wgsl),
    }
}

/// Why the gate refused a source: which one, and naga's message with the
/// source line of the span. A hot-reload refusal prints through `Display`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ShaderError {
    /// File name (relative to the shader dir) or inline const name.
    pub name: String,
    /// naga's message, source line and column included.
    pub message: String,
}

impl core::fmt::Display for ShaderError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.name, self.message)
    }
}

impl std::error::Error for ShaderError {}

/// Validate one WGSL source without a GPU, in its shipped composition.
/// `name` leads the error, so a refusal names the file a designer edited (or
/// the inline const checked).
pub fn validate_source(name: &str, wgsl: &str) -> Result<(), ShaderError> {
    let src = shipped_source(name, wgsl);
    engine_material::compile::validate(&src).map_err(|err| ShaderError {
        name: name.to_string(),
        message: err.to_string(),
    })
}

/// Every `.wgsl` under [`SHADER_DIR`], recursively, as (name relative to the
/// dir with `/` separators, source), sorted by name.
///
/// Native only: the browser build has no file system, so the walk is compiled
/// out there (the wasm gate counts a new `std::fs` call in this crate as a
/// hazard). Everything that calls it (this module's tests, the window's reload
/// cook) is native.
#[cfg(not(target_arch = "wasm32"))]
pub fn shipped_files() -> Result<Vec<(String, String)>, ShaderError> {
    let base = std::path::Path::new(SHADER_DIR);
    let mut out = Vec::new();
    walk(base, base, &mut out)?;
    out.sort();
    Ok(out)
}

#[cfg(not(target_arch = "wasm32"))]
fn walk(
    dir: &std::path::Path,
    base: &std::path::Path,
    out: &mut Vec<(String, String)>,
) -> Result<(), ShaderError> {
    let at = |path: &std::path::Path, err: &dyn core::fmt::Display| ShaderError {
        name: path.display().to_string(),
        message: err.to_string(),
    };
    let entries = std::fs::read_dir(dir).map_err(|err| at(dir, &err))?;
    for entry in entries {
        let path = entry.map_err(|err| at(dir, &err))?.path();
        if path.is_dir() {
            walk(&path, base, out)?;
        } else if path.extension().is_some_and(|ext| ext == "wgsl") {
            let src = std::fs::read_to_string(&path).map_err(|err| at(&path, &err))?;
            let name = path
                .strip_prefix(base)
                .map_err(|err| at(&path, &err))?
                .to_string_lossy()
                .replace('\\', "/");
            out.push((name, src));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CQ23 accept: every shipped file and every reachable inline chunk
    /// parses and validates, no GPU. A type error here fails the suite
    /// instead of a pipeline build. Run it with `window` too
    /// (`--features shader-validate,window`) to add the window-leg chunks.
    #[test]
    fn shader_validate() {
        let files = shipped_files().expect("shipped shader dir");
        for must in [
            "blit.wgsl",
            "forward.wgsl",
            "tonemap.wgsl",
            "post/composite_compute.wgsl",
        ] {
            assert!(
                files.iter().any(|(name, _)| name == must),
                "walk missed {must}: {} files",
                files.len()
            );
        }
        let mut failures: Vec<String> = Vec::new();
        for (name, src) in &files {
            if let Err(err) = validate_source(name, src) {
                failures.push(err.to_string());
            }
        }
        // Inline chunks that build without `window` ...
        let inline: Vec<(&str, &str)> = vec![
            (
                "baked_gi::FORWARD_GI_WGSL",
                crate::baked_gi::FORWARD_GI_WGSL,
            ),
            ("overlay::OVERLAY_WGSL", crate::overlay::OVERLAY_WGSL),
            ("sky::FORWARD_SKYBOX_WGSL", crate::sky::FORWARD_SKYBOX_WGSL),
        ];
        // ... and the `window`-leg ones when that feature is on.
        #[cfg(feature = "window")]
        let inline = {
            let mut all = inline;
            all.extend([
                ("graph::GX_PASS_WGSL", crate::graph::GX_PASS_WGSL),
                (
                    "shadow_cluster::SHADOW_DEPTH_WGSL",
                    crate::shadow_cluster::SHADOW_DEPTH_WGSL,
                ),
                (
                    "shadow_cluster::SHADOW_FORWARD_WGSL",
                    crate::shadow_cluster::SHADOW_FORWARD_WGSL,
                ),
                (
                    "ibl::gpu::FORWARD_IBL_WGSL",
                    crate::ibl::gpu::FORWARD_IBL_WGSL,
                ),
                (
                    "aa_scale::gpu::SMAA_COMPUTE_WGSL",
                    crate::aa_scale::gpu::SMAA_COMPUTE_WGSL,
                ),
                (
                    "aa_scale::gpu::SMAA_PREDICATED_COMPUTE_WGSL",
                    crate::aa_scale::gpu::SMAA_PREDICATED_COMPUTE_WGSL,
                ),
            ]);
            all
        };
        for (name, src) in inline {
            if let Err(err) = validate_source(name, src) {
                failures.push(err.to_string());
            }
        }
        assert!(
            failures.is_empty(),
            "{} source(s) fail naga:\n{}",
            failures.len(),
            failures.join("\n\n")
        );
    }

    /// A seeded type error in forward.wgsl fails with naga's line:col, which
    /// the brace check the live swap used before would accept.
    #[test]
    fn shader_validate_seeded_type_error_names_line_and_column() {
        let mut src = shipped_files()
            .expect("shipped shader dir")
            .into_iter()
            .find(|(name, _)| name == "forward.wgsl")
            .expect("forward.wgsl ships")
            .1;
        src.push_str("\nfn cq23_seeded() -> f32 { return vec3<f32>(1.0); }\n");
        let line = shipped_source("forward.wgsl", &src).lines().count();
        let err = validate_source("forward.wgsl", &src)
            .expect_err("type mismatch")
            .to_string();
        assert!(err.contains("forward.wgsl"), "{err}");
        let at = format!("{line}:");
        assert!(err.contains(&at), "no line:col in {err}");
    }
}
