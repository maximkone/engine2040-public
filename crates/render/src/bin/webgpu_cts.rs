//! webgpu_cts: the native half of the WebGPU conformance gate (HX4).
//!
//! The tool `tools/webgpu-cts.mjs` runs one case per process, so a driver
//! abort in one case never takes the run with it. This binary is that one
//! case: it opens the strict WebGPU instance of the render stack
//! (`E2040_STRICT_WEBGPU=1` adds `InstanceFlags::STRICT_WEBGPU_COMPLIANCE`),
//! compiles the WGSL the case names and prints one verdict line.
//!
//! The verdict is the WebGPU oracle the conformance suite uses for the
//! shader-validation area: a case that must compile and one that must be
//! refused. The line is one machine-readable record:
//!
//! ```text
//! webgpu_cts_case: verdict=compiled|rejected|error adapter=<name> strict=<0|1> why=<text>
//! ```
//!
//! `verdict=rejected` means the device raised a validation error for the
//! shader. `verdict=error` means the case could not run at all (no adapter,
//! no device): the tool counts that as a crash, never as a pass.
//!
//! The adapter name carries blanks as `_`, so every field is one token. The
//! `why` text is the last field and may hold blanks: the tool reads the rest
//! of the line as the reason.
//!
//! Usage: `webgpu_cts --wgsl <file>` (the WGSL text of one case) or
//! `webgpu_cts --wgsl-stdin` (the text on stdin). The strict switch is read
//! from the environment, never from a flag, so the flag cannot be forgotten:
//! `strict=0` in the line means the instance was not strict and the tool
//! refuses the run.

use std::io::Read;

/// The verdict of one case.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Verdict {
    /// The device accepted the WGSL.
    Compiled,
    /// The device refused the WGSL with a validation error.
    Rejected,
    /// The case could not run: no adapter, no device, no WGSL.
    Error,
}

impl Verdict {
    /// The word the tool reads.
    const fn name(self) -> &'static str {
        match self {
            Self::Compiled => "compiled",
            Self::Rejected => "rejected",
            Self::Error => "error",
        }
    }
}

/// The WGSL text of one case, read from a file or from stdin.
fn read_wgsl() -> Result<String, String> {
    let mut args = std::env::args().skip(1);
    let mut path: Option<String> = None;
    let mut stdin = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--wgsl" => {
                path = Some(args.next().ok_or("--wgsl needs a file path")?);
            }
            "--wgsl-stdin" => stdin = true,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if stdin {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .map_err(|err| format!("cannot read the WGSL from stdin: {err}"))?;
        return Ok(text);
    }
    let path = path.ok_or("give --wgsl <file> or --wgsl-stdin")?;
    std::fs::read_to_string(&path).map_err(|err| format!("cannot read {path}: {err}"))
}

/// The strict switch, read from the render stack's own env name.
fn strict_requested() -> bool {
    engine_render::backend::strict_webgpu_from_env()
}

/// The adapter name as one token: the blanks become `_`, so the receipt line
/// stays parseable field by field.
fn one_token(text: &str) -> String {
    let out: String = text
        .chars()
        .map(|c| if c.is_whitespace() { '_' } else { c })
        .collect();
    if out.is_empty() {
        "none".to_string()
    } else {
        out
    }
}

/// The reason, on one line: the tool reads the rest of the line after `why=`.
fn one_line(text: &str) -> String {
    text.replace(['\n', '\r'], " ").trim().to_string()
}

/// Open the strict instance, the adapter and the device, then compile the
/// WGSL and report what the device said.
fn run(wgsl: &str) -> (Verdict, String, bool, String) {
    if !strict_requested() {
        return (
            Verdict::Error,
            "none".to_string(),
            false,
            format!(
                "{} is not 1: the instance was not strict, so the run proves nothing",
                engine_render::backend::STRICT_WEBGPU_ENV
            ),
        );
    }
    // The one native instance maker of the render stack: the strict flag
    // reaches the device through it, never through a fresh `Instance::new`.
    let instance = engine_render::backend::new_instance(wgpu::Backends::PRIMARY);
    let options = wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    };
    let adapter = match pollster::block_on(instance.request_adapter(&options)) {
        Ok(adapter) => adapter,
        Err(err) => {
            return (
                Verdict::Error,
                "none".to_string(),
                true,
                format!("no wgpu adapter on this machine: {err}"),
            )
        }
    };
    let name = adapter.get_info().name.clone();
    let descriptor = engine_render::present::Presenter::device_descriptor(&adapter);
    let (device, _queue) = match pollster::block_on(adapter.request_device(&descriptor)) {
        Ok(pair) => pair,
        Err(err) => {
            return (
                Verdict::Error,
                one_token(&name),
                true,
                format!("request_device: {err}"),
            )
        }
    };
    // The error scope catches the validation error a bad program raises, so
    // the device stays usable and the next case gets a clean device.
    // `pop` gives `None` when no error was raised (the program compiled)
    // and the error itself when the device refused the program.
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    // The module itself is not used: the error scope is the answer.
    let _module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("webgpu-cts-case"),
        source: wgpu::ShaderSource::Wgsl(wgsl.into()),
    });
    let verdict = match pollster::block_on(scope.pop()) {
        None => Verdict::Compiled,
        Some(err) => {
            return (
                Verdict::Rejected,
                one_token(&name),
                true,
                format!("the device refused the shader: {err}"),
            )
        }
    };
    (verdict, one_token(&name), true, String::new())
}

fn main() {
    let wgsl = match read_wgsl() {
        Ok(text) => text,
        Err(why) => {
            println!(
                "webgpu_cts_case: verdict={} adapter=none strict=0 why={}",
                Verdict::Error.name(),
                one_line(&why)
            );
            std::process::exit(2);
        }
    };
    let (verdict, name, strict, why) = run(&wgsl);
    println!(
        "webgpu_cts_case: verdict={} adapter={} strict={} why={}",
        verdict.name(),
        name,
        u8::from(strict),
        one_line(&why)
    );
    // A case that could not run is not a pass: the exit code says so.
    if verdict == Verdict::Error {
        std::process::exit(1);
    }
}
