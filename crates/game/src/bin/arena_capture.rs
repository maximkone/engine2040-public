//! arena_capture: headless plus chart capture helpers for the arena binary.
//!
//! Owner: engine-game bin; consumer: `arena.rs` (`run`, `ArenaApp`, bin
//! tests) plus the `arena.exe --frames N --exit` smoke. Units: ticks,
//! frames, milliseconds, PNG bytes. Determinism: the same tape plus session
//! files the same frames plus manifest; pixels never enter the sim hash.
//! Errors: [`WinError`](engine_game::windowed::WinError); loud, never silent.
//!
//! Dual role: Cargo autodiscovers this file as its own `arena_capture`
//! target, so the app-only helpers live behind one `app` gate while a loud
//! stub `main` covers the headless target. The real entry stays `arena`.

/// Stub entry for the autodiscovered `arena_capture` target: this file is a
/// helper module of the `arena` binary, never a runnable tool.
fn main() {
    eprintln!("arena_capture is a helper module of the arena binary, not a tool");
    std::process::exit(2);
}

/// Headless plus chart capture helpers (app-only: presenter and windowed
/// types ride the `app` feature, so the headless default stays std-only).
#[cfg(feature = "app")]
pub(crate) mod capture {
    /// Status line on stdout: `println!` output, also mirrored into the run log.
    macro_rules! say {
        ($($arg:tt)*) => {{
            let line = format!($($arg)*);
            println!("{line}");
            engine_game::crash::mirror_line(&line);
        }};
    }

    /// Status line on stderr: `eprintln!` output, also mirrored into the run log.
    macro_rules! tell {
        ($($arg:tt)*) => {{
            let line = format!($($arg)*);
            eprintln!("{line}");
            engine_game::crash::mirror_line(&line);
        }};
    }

    use std::time::Instant;

    use engine_game::capture::{self, CaptureSink, TapeInputs};
    use engine_game::windowed::{
        cap_message as capped, dressing_camera, format_timed_smoke, hud_draw_list,
        push_dressing_draws, run_headless_smoke_with_options, stage_dressing, SceneSelect, WinArgs,
        WinError, SMOKE_FRAME_SIZE,
    };
    use engine_game::{App, GameConfig, HUMAN_SLOT};
    use engine_input::FighterId;
    use engine_render::present::{DrawList, MeshKey, Presenter};

    /// Chart flags: `--chart` runs the material-chart path synchronously
    /// (no window), `--chart-dir <path>` picks the output directory (default
    /// `target/chart-gx15`). Parsed before capture flags so `WinArgs` still
    /// refuses every other unknown flag exactly as before.
    pub(crate) struct ChartArgs {
        /// True while `--chart` was given.
        pub(crate) enabled: bool,
        /// Directory the chart PNGs plus the reference go into.
        pub(crate) dir: std::path::PathBuf,
    }

    /// Default chart output directory beside the build outputs.
    pub(crate) const CHART_DEFAULT_DIR: &str = "target/chart-gx15";
    /// Acceptance viewport in pixels (1600x900 downscales to 256x144).
    pub(crate) const CHART_VIEWPORT: (u32, u32) = (1600, 900);
    /// Longest accepted chart flag path in bytes (mirrors the capture bound).
    pub(crate) const CHART_MAX_PATH_BYTES: usize = 4096;

    /// Split chart flags out of `raw`, passing everything else through in order
    /// for capture flags plus `WinArgs`. Unknown `--chart*` flags, missing
    /// values, duplicates and a `--chart-dir` without `--chart` fail loudly;
    /// non-chart flags (known or not) are never touched here.
    pub(crate) fn split_chart_args(raw: &[String]) -> Result<(ChartArgs, Vec<String>), WinError> {
        let mut enabled = false;
        let mut seen_chart = false;
        let mut seen_dir = false;
        let mut dir: Option<String> = None;
        let mut rest = Vec::with_capacity(raw.len());
        let mut index = 0usize;
        while index < raw.len() {
            match raw[index].as_str() {
                "--chart" => {
                    if seen_chart {
                        return Err(WinError::BadArgs("duplicate --chart".to_string()));
                    }
                    seen_chart = true;
                    enabled = true;
                }
                "--chart-dir" => {
                    if seen_dir {
                        return Err(WinError::BadArgs("duplicate --chart-dir".to_string()));
                    }
                    seen_dir = true;
                    index += 1;
                    match raw.get(index) {
                        Some(value) if !value.starts_with("--") => dir = Some(value.clone()),
                        _ => {
                            return Err(WinError::BadArgs("--chart-dir needs a value".to_string()));
                        }
                    }
                }
                other if other.starts_with("--chart") => {
                    return Err(WinError::BadArgs(other.to_string()));
                }
                _ => rest.push(raw[index].clone()),
            }
            index += 1;
        }
        if dir.is_some() && !enabled {
            return Err(WinError::BadArgs("--chart-dir needs --chart".to_string()));
        }
        let dir = match dir {
            Some(raw) => {
                if raw.is_empty() || raw.len() > CHART_MAX_PATH_BYTES {
                    return Err(WinError::BadArgs("--chart-dir path rejected".to_string()));
                }
                std::path::PathBuf::from(raw)
            }
            None => std::path::PathBuf::from(CHART_DEFAULT_DIR),
        };
        Ok((ChartArgs { enabled, dir }, rest))
    }

    /// Evidence-only gamma for the chart PNG strips: linear 0..=1 to sRGB u8
    /// with a 2.2 exponent. Deterministic per value; pixels are evidence only,
    /// never hashed, so the exact curve only needs to be stable, not canonical.
    pub(crate) fn chart_linear_to_srgb_u8(value: f32) -> u8 {
        let clamped = value.clamp(0.0, 1.0);
        let scaled = (clamped.powf(1.0 / 2.2) * 255.0).round().clamp(0.0, 255.0);
        // Float-to-int without `as`: step the u8 ladder until its `From` image
        // reaches the rounded target. NaN never compares greater, so it folds
        // to 0 exactly like the old saturating cast; cold evidence path only.
        let mut out = 0u8;
        while out < u8::MAX && f32::from(out) < scaled {
            out = out.saturating_add(1);
        }
        out
    }

    pub(crate) fn chart_crc32_table() -> &'static [u32; 256] {
        static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
        TABLE.get_or_init(|| {
            let mut table = [0u32; 256];
            for (index, cell) in (0u32..).zip(table.iter_mut()) {
                let mut c = index;
                for _ in 0..8 {
                    c = if c & 1 == 1 {
                        0xEDB8_8320 ^ (c >> 1)
                    } else {
                        c >> 1
                    };
                }
                *cell = c;
            }
            table
        })
    }

    pub(crate) fn chart_crc32_update(mut crc: u32, data: &[u8]) -> u32 {
        for byte in data {
            // Low byte of `crc ^ byte` without a u32->usize cast: the xor low
            // byte is the crc low byte xored with the input byte, widened by `From`.
            let low = crc.to_le_bytes()[0] ^ *byte;
            crc = chart_crc32_table()[usize::from(low)] ^ (crc >> 8);
        }
        crc
    }

    pub(crate) fn chart_write_chunk(
        out: &mut Vec<u8>,
        tag: &[u8; 4],
        data: &[u8],
    ) -> Result<(), WinError> {
        let len = u32::try_from(data.len())
            .map_err(|_| WinError::BadArgs("chart chunk length exceeds u32".to_string()))?;
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(tag);
        out.extend_from_slice(data);
        let mut crc = chart_crc32_update(0xFFFF_FFFF, tag);
        crc = chart_crc32_update(crc, data);
        out.extend_from_slice(&(crc ^ 0xFFFF_FFFF).to_be_bytes());
        Ok(())
    }

    pub(crate) fn chart_adler32(data: &[u8]) -> u32 {
        const MOD: u32 = 65521;
        let mut a: u32 = 1;
        let mut b: u32 = 0;
        for piece in data.chunks(5552) {
            for byte in piece {
                a += u32::from(*byte);
                b += a;
            }
            a %= MOD;
            b %= MOD;
        }
        (b << 16) | a
    }

    /// Encode RGBA bytes as a PNG (8-bit, filter 0, stored deflate): the same
    /// pixels always give the same bytes. Sizes over the 2048 frame bound or a
    /// pixel length mismatch fail loudly instead of writing a corrupt file.
    pub(crate) fn chart_encode_png_rgba8(
        pixels: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>, WinError> {
        if width == 0 || height == 0 || width > 2048 || height > 2048 {
            return Err(WinError::BadArgs(format!(
                "chart size {width}x{height} rejected"
            )));
        }
        let wide = usize::try_from(width)
            .map_err(|_| WinError::BadArgs(format!("chart size {width}x{height} rejected")))?;
        let high = usize::try_from(height)
            .map_err(|_| WinError::BadArgs(format!("chart size {width}x{height} rejected")))?;
        let stride = wide
            .checked_mul(4)
            .ok_or_else(|| WinError::BadArgs(format!("chart size {width}x{height} rejected")))?;
        let total = stride
            .checked_mul(high)
            .ok_or_else(|| WinError::BadArgs(format!("chart size {width}x{height} rejected")))?;
        if pixels.len() != total {
            return Err(WinError::Render("chart pixel length mismatch".to_string()));
        }
        let mut raw = Vec::with_capacity(pixels.len().saturating_add(high));
        for row in pixels.chunks_exact(stride) {
            raw.push(0u8);
            raw.extend_from_slice(row);
        }
        let mut zlib = Vec::with_capacity(raw.len() + 16);
        zlib.extend_from_slice(&[0x78, 0x01]);
        let mut blocks = raw.chunks(65535).peekable();
        while let Some(block) = blocks.next() {
            let last = blocks.peek().is_none();
            zlib.push(u8::from(last));
            let len = u16::try_from(block.len())
                .map_err(|_| WinError::Render("chart deflate block too long".to_string()))?;
            zlib.extend_from_slice(&len.to_le_bytes());
            zlib.extend_from_slice(&(!len).to_le_bytes());
            zlib.extend_from_slice(block);
        }
        zlib.extend_from_slice(&chart_adler32(&raw).to_be_bytes());
        let mut out = Vec::with_capacity(8 + 25 + zlib.len() + 16 + 12);
        out.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
        let mut ihdr = [0u8; 13];
        ihdr[0..4].copy_from_slice(&width.to_be_bytes());
        ihdr[4..8].copy_from_slice(&height.to_be_bytes());
        ihdr[8] = 8;
        ihdr[9] = 6;
        chart_write_chunk(&mut out, b"IHDR", &ihdr)?;
        chart_write_chunk(&mut out, b"IDAT", &zlib)?;
        chart_write_chunk(&mut out, b"IEND", &[])?;
        Ok(out)
    }

    /// Material-chart path: size the 1600x900 viewport through the
    /// `chart_target_size` downscale path, evaluate the five-row material chart
    /// on the CPU, file the live chart PNG plus the on-disk reference beside
    /// it, then best-effort reach the frozen `submit_ibl_chart` seam through
    /// one public `render_offscreen` frame (skipped loudly with no adapter).
    /// Always exits 0 on the green path with the beside-reference tolerance
    /// posted; bad input exits 2, device failures exit 1.
    pub(crate) fn run_chart_mode(config: &GameConfig, _args: &WinArgs, chart: &ChartArgs) -> i32 {
        let (target_w, target_h) = match chart_target_checked() {
            Ok(size) => size,
            Err(msg) => {
                tell!("arena chart: {msg}");
                return 2;
            }
        };
        let rows = engine_render::material_layer::chart();
        let (live, max_abs) = match chart_eval_rows(&rows) {
            Ok(eval) => eval,
            Err(msg) => {
                tell!("arena chart: {msg}");
                return 1;
            }
        };
        let tolerance = engine_render::material_layer::CHART_TOLERANCE;
        if max_abs > tolerance {
            tell!("arena chart: beside-RL6 {max_abs:.8} exceeds {tolerance:.8}");
            return 1;
        }
        let (live_px, ref_px) = match chart_paint_strips(&rows, &live, target_w, target_h) {
            Ok(px) => px,
            Err(err) => {
                tell!("arena: {err}");
                return 2;
            }
        };
        let live_png = match chart_encode_png_rgba8(&live_px, target_w, target_h) {
            Ok(png) => png,
            Err(err) => {
                tell!("arena: {err}");
                return 2;
            }
        };
        let ref_png = match chart_encode_png_rgba8(&ref_px, target_w, target_h) {
            Ok(png) => png,
            Err(err) => {
                tell!("arena: {err}");
                return 2;
            }
        };
        if let Err(err) = std::fs::create_dir_all(&chart.dir) {
            tell!("arena chart: dir {}: {err}", chart.dir.display());
            return 2;
        }
        let json = chart_manifest_json(
            &rows,
            &live,
            target_w,
            target_h,
            max_abs,
            tolerance,
            config.seed,
        );
        let (chart_path, ref_path, json_path) = match chart_write_outputs(
            &chart.dir,
            target_w,
            target_h,
            &live_png,
            &ref_png,
            json.as_bytes(),
        ) {
            Ok(paths) => paths,
            Err(msg) => {
                tell!("arena chart: {msg}");
                return 2;
            }
        };
        let gpu_note = match chart_gpu_probe() {
            Ok(note) => note,
            Err(err) => {
                tell!("arena chart presenter: {err}");
                return 1;
            }
        };
        say!(
            "arena chart: viewport=1600x900 target={target_w}x{target_h} rows={} max_abs_diff={max_abs:.8} tolerance={tolerance:.8} within_tol=true chart=\"{}\" reference=\"{}\" manifest=\"{}\" seed={} {gpu_note}",
            rows.len(),
            chart_path.display(),
            ref_path.display(),
            json_path.display(),
            config.seed
        );
        0
    }

    /// Resolve the chart target through the `chart_target_size` downscale path
    /// and pin the 256x144 contract for the acceptance viewport. Fails loudly
    /// on a bad viewport or a target that drifts from the contract.
    pub(crate) fn chart_target_checked() -> Result<(u32, u32), String> {
        let (target_w, target_h) =
            engine_render::ibl::chart_target_size(CHART_VIEWPORT.0, CHART_VIEWPORT.1)
                .map_err(|err| err.to_string())?;
        if (target_w, target_h) != (256, 144) {
            return Err(format!(
                "target {target_w}x{target_h} differs from the 256x144 contract"
            ));
        }
        Ok((target_w, target_h))
    }

    /// Evaluate every material-chart row on the CPU: the live triple per row
    /// plus the worst absolute drift from the expected triple. A refused row
    /// fails loudly; the caller owns the tolerance verdict.
    pub(crate) fn chart_eval_rows(
        rows: &[engine_render::material_layer::ChartEntry],
    ) -> Result<(Vec<[f32; 3]>, f32), String> {
        let mut live: Vec<[f32; 3]> = Vec::with_capacity(rows.len());
        let mut max_abs = 0.0f32;
        for entry in rows {
            match engine_render::material_layer::check_entry(entry) {
                Ok(got) => {
                    for (live_c, ref_c) in got.iter().zip(entry.expected.iter()) {
                        max_abs = max_abs.max((live_c - ref_c).abs());
                    }
                    live.push(got);
                }
                Err(err) => return Err(err.to_string()),
            }
        }
        Ok((live, max_abs))
    }

    /// Paint the live strip plus the reference strip: one horizontal band per
    /// chart row, stretched over the downscaled target. Evidence pixels only,
    /// never hashed; the transfer matches the chart PNG encoder.
    pub(crate) fn chart_paint_strips(
        rows: &[engine_render::material_layer::ChartEntry],
        live: &[[f32; 3]],
        target_w: u32,
        target_h: u32,
    ) -> Result<(Vec<u8>, Vec<u8>), WinError> {
        let wide = usize::try_from(target_w)
            .map_err(|_| WinError::BadArgs(format!("chart size {target_w}x{target_h} rejected")))?;
        let high = usize::try_from(target_h)
            .map_err(|_| WinError::BadArgs(format!("chart size {target_w}x{target_h} rejected")))?;
        let pixels = wide
            .checked_mul(high)
            .and_then(|cells| cells.checked_mul(4))
            .ok_or_else(|| {
                WinError::BadArgs(format!("chart size {target_w}x{target_h} rejected"))
            })?;
        let mut live_px = vec![0u8; pixels];
        let mut ref_px = vec![0u8; pixels];
        for y in 0..high {
            let row = (y * rows.len() / high).min(rows.len() - 1);
            let live_rgb = [
                chart_linear_to_srgb_u8(live[row][0]),
                chart_linear_to_srgb_u8(live[row][1]),
                chart_linear_to_srgb_u8(live[row][2]),
            ];
            let ref_rgb = [
                chart_linear_to_srgb_u8(rows[row].expected[0]),
                chart_linear_to_srgb_u8(rows[row].expected[1]),
                chart_linear_to_srgb_u8(rows[row].expected[2]),
            ];
            for x in 0..wide {
                let at = (y * wide + x) * 4;
                live_px[at..at + 3].copy_from_slice(&live_rgb);
                live_px[at + 3] = 255;
                ref_px[at..at + 3].copy_from_slice(&ref_rgb);
                ref_px[at + 3] = 255;
            }
        }
        Ok((live_px, ref_px))
    }

    /// Build the chart manifest JSON: viewport, per-row expected plus live
    /// triples, target, worst drift, tolerance and seed. Same bytes for the
    /// same chart, so the manifest is stable evidence beside the PNGs.
    pub(crate) fn chart_manifest_json(
        rows: &[engine_render::material_layer::ChartEntry],
        live: &[[f32; 3]],
        target_w: u32,
        target_h: u32,
        max_abs: f32,
        tolerance: f32,
        seed: u64,
    ) -> String {
        let mut json = String::from("{\"viewport\":[1600,900],\"rows\":[");
        for (index, entry) in rows.iter().enumerate() {
            if index > 0 {
                json.push(',');
            }
            json.push_str(&format!(
                "{{\"name\":\"{}\",\"expected\":[{:.6},{:.6},{:.6}],\"live\":[{:.6},{:.6},{:.6}]}}",
                entry.name,
                entry.expected[0],
                entry.expected[1],
                entry.expected[2],
                live[index][0],
                live[index][1],
                live[index][2]
            ));
        }
        json.push_str(&format!(
            "],\"target\":[{target_w},{target_h}],\"max_abs_diff\":{max_abs:.8},\"tolerance\":{tolerance:.8},\"seed\":{seed}}}"
        ));
        json
    }

    /// File the live chart PNG, the reference PNG and the manifest JSON under
    /// `dir`, creating it first. Paths return for the receipt line; every IO
    /// failure names its path loudly instead of dropping evidence.
    pub(crate) fn chart_write_outputs(
        dir: &std::path::Path,
        target_w: u32,
        target_h: u32,
        live_png: &[u8],
        ref_png: &[u8],
        json: &[u8],
    ) -> Result<(std::path::PathBuf, std::path::PathBuf, std::path::PathBuf), String> {
        if let Err(err) = std::fs::create_dir_all(dir) {
            return Err(format!("dir {}: {err}", dir.display()));
        }
        let chart_path = dir.join(format!("chart_{target_w}x{target_h}.png"));
        let ref_path = dir.join(format!("chart_reference_{target_w}x{target_h}.png"));
        let json_path = dir.join("chart_reference.json");
        for (path, bytes) in [
            (&chart_path, live_png),
            (&ref_path, ref_png),
            (&json_path, json),
        ] {
            if let Err(err) = std::fs::write(path, bytes) {
                return Err(format!("{}: {err}", path.display()));
            }
        }
        Ok((chart_path, ref_path, json_path))
    }

    /// Best-effort reach of the frozen chart seam: one public
    /// `render_offscreen` frame over the acceptance viewport, which folds the
    /// chart pass into the receipt. Returns the receipt note for the chart
    /// line; no adapter resolves to the loud skip note, never a fake pass. A
    /// twin that presents nothing or skips the chart pass fails loudly.
    pub(crate) fn chart_gpu_probe() -> Result<String, WinError> {
        let mut presenter = match Presenter::init_offscreen() {
            Err(engine_render::present::PresentError::NoAdapter) => {
                return Ok("presenter skipped: no wgpu adapter on this machine".to_string());
            }
            Err(err) => return Err(WinError::Present(err)),
            Ok(presenter) => presenter,
        };
        if let Err(err) = presenter.resize(CHART_VIEWPORT.0, CHART_VIEWPORT.1) {
            return Err(WinError::Present(err));
        }
        let key = MeshKey::new(1, [0xA1; 32]);
        let _ = presenter.upload_mesh(key, &engine_render::v1::cube());
        let _ = stage_dressing(&mut presenter);
        let list = frame_list(&presenter, CHART_VIEWPORT.0, CHART_VIEWPORT.1, None, 0)?;
        match presenter.render_offscreen(&list) {
            Ok(out) => {
                if !out.receipt.presented || out.receipt.ibl_passes != 1 {
                    return Err(WinError::Render(format!(
                        "twin presented={} ibl_passes={}",
                        out.receipt.presented, out.receipt.ibl_passes
                    )));
                }
                Ok(format!(
                    "presenter twin: {} ({}) ibl_passes=1 ibl_submit_us={}",
                    out.receipt.adapter, out.receipt.driver, out.receipt.ibl_submit_us
                ))
            }
            Err(err) => Err(WinError::Present(err)),
        }
    }

    /// Resolve the capture scene id: `None` keeps the baked-arena default and
    /// `Some(Arena)` names it explicitly. Wrong scene names never reach here:
    /// [`WinArgs::parse`] refuses them loudly, so the wrong scene is unreachable
    /// on every path that carries the parsed args.
    pub(crate) fn resolve_capture_scene(scene: Option<SceneSelect>) -> &'static str {
        match scene {
            None | Some(SceneSelect::Arena) => engine_game::windowed::ARENA_SCENE_ID,
        }
    }

    /// Format the headless-capture receipt line: ticks plus frames plus scene
    /// plus measured fps plus manifest plus sequence sha. Non-finite or
    /// non-positive elapsed and zero frames fail loudly (same rule as the
    /// timed-smoke line), so a capture never reports a fake fps.
    pub(crate) fn format_headless_capture_line(
        ticks: u32,
        frames: u32,
        scene: &str,
        elapsed_ms: f64,
        manifest: &std::path::Path,
        sha: &str,
    ) -> Result<String, WinError> {
        let fps = engine_game::windowed::smoke_fps(frames, elapsed_ms)?;
        Ok(format!(
            "arena headless capture: ticks={ticks} frames={frames} scene=\"{scene}\" elapsed_ms={elapsed_ms:.3} fps={fps:.1} manifest=\"{}\" sha={sha}",
            manifest.display(),
        ))
    }

    /// Headless capture: the same session over the tape (idle where silent),
    /// frames bound to the sim through the twin when an adapter exists (so
    /// `--camera-path arena600` flies headless too) or the synth fallback on
    /// adapter-less machines, manifest beside PNGs. The synth fallback keeps
    /// adapter-less CI green with identical receipts; only the twin path moves.
    pub(crate) fn headless_capture(
        config: GameConfig,
        frames: u32,
        flags: &capture::CaptureFlags,
        display_err: &WinError,
        scene: Option<SceneSelect>,
        camera_path: Option<engine_game::windowed::CameraPath>,
    ) -> i32 {
        let scene_id = resolve_capture_scene(scene);
        let tape = match flags
            .tape
            .as_ref()
            .map(|path| capture::load_tape_file(path.as_path()))
            .transpose()
        {
            Ok(tape) => tape,
            Err(err) => {
                tell!("arena: {err} (display was: {display_err})");
                return 2;
            }
        };
        let ticks = frames.min(config.ticks);
        let started = Instant::now();
        // One adapter probe: the twin owns it when present, otherwise the synth
        // path runs (same receipts, static pixels, never a fake orbit).
        let report = match Presenter::init_offscreen() {
            Ok(presenter) => {
                twin_headless_capture(config, ticks, presenter, tape, flags, camera_path)
            }
            Err(engine_render::present::PresentError::NoAdapter) => {
                let req = capture::HeadlessRequest {
                    dir: flags.dir.clone(),
                    every: flags.every,
                    width: SMOKE_FRAME_SIZE.0,
                    height: SMOKE_FRAME_SIZE.1,
                    frames: ticks,
                    rows: flags.rows.clone(),
                    sha: flags.sha.clone(),
                    fps: flags.fps,
                };
                capture::run_headless_capture(config, tape.as_ref(), &req)
                    .map_err(|err| WinError::Render(capped(&err.to_string())))
            }
            Err(err) => Err(WinError::Present(err)),
        };
        match report {
            Ok(report) => {
                let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                match format_headless_capture_line(
                    ticks,
                    report.frames,
                    scene_id,
                    elapsed_ms,
                    &report.manifest,
                    &report.sequence_fnv,
                ) {
                    Ok(line) => say!("{line} display=\"{display_err}\""),
                    Err(err) => {
                        tell!("arena: {err}");
                        return 2;
                    }
                }
                0
            }
            Err(err) => {
                tell!("arena: {err} (display was: {display_err})");
                2
            }
        }
    }

    /// Headless twin capture: the same session over the tape with every due tick
    /// rendered through the offscreen twin under `camera_path` (the orbit eye is
    /// the captured-frame count, so the flags run moves while the baseline stays
    /// on the establishing view). Sim plus tape decide every pixel row; the tick
    /// never sees the camera. Loud on device, validation or byte failures.
    pub(crate) fn twin_headless_capture(
        config: GameConfig,
        ticks: u32,
        mut presenter: Presenter,
        tape: Option<capture::LoadedTape>,
        flags: &capture::CaptureFlags,
        camera_path: Option<engine_game::windowed::CameraPath>,
    ) -> Result<capture::CaptureReport, WinError> {
        let inputs = tape
            .as_ref()
            .map(|loaded| loaded.inputs.as_slice())
            .unwrap_or(&[]);
        capture::validate_tape_for_run(inputs, HUMAN_SLOT, config.ticks)
            .map_err(|err| WinError::Render(capped(&err.to_string())))?;
        let meta = capture::ManifestMeta {
            rows: flags.rows.clone(),
            sha: flags.sha.clone(),
            fps: flags.fps,
        };
        let mut sink = capture::CaptureSink::create(
            &flags.dir,
            flags.every,
            config.seed,
            tape.as_ref(),
            capture::TWIN_RENDERER,
            &meta,
        )
        .map_err(|err| WinError::Render(capped(&err.to_string())))?;
        presenter.resize(SMOKE_FRAME_SIZE.0, SMOKE_FRAME_SIZE.1)?;
        // One lit cube proves the forward pass; a refused upload falls back to
        // clear-only frames rather than failing the capture.
        let key = MeshKey::new(1, [0xA1; 32]);
        let _ = presenter.upload_mesh(key, &engine_render::v1::cube());
        // Dressing beside the cube; refused uploads skip draws.
        let _ = stage_dressing(&mut presenter);
        // Twin fighters: one body per configured slot (RL26).
        engine_render::twin_fighters::stage_twin_fighters(&mut presenter, config.fighters)
            .map_err(|err| WinError::Render(capped(&format!("{err}"))))?;
        // Recooked arena kit (CT8): staged once; the per-frame bind swaps
        // the cube for the kit draws when resident, else the cube stays.
        let kit_report = engine_game::arena_kit::stage_kit_or_default(&mut presenter);
        let kit_draws: Vec<(MeshKey, engine_render::v1::PbrMaterial)> = kit_report
            .draws
            .iter()
            .map(|draw| (draw.key, draw.material.clone()))
            .collect();
        let mut app = App::new(config)?;
        let human = FighterId::new(HUMAN_SLOT).ok_or(WinError::BadConfig("fighters"))?;
        let mut cursor = TapeInputs::new(tape.map(|loaded| loaded.inputs).unwrap_or_default());
        let mut presented = 0u32;
        for tick in 0..ticks {
            let live = engine_input::FighterInput {
                tick,
                fighter: human,
                mx: 0,
                mz: 0,
                facing: 0,
                buttons: engine_input::Buttons::NONE,
            };
            let input = cursor.input_for_tick(tick).unwrap_or(live);
            let outcome = app.tick(input);
            if !outcome.input_accepted {
                return Err(WinError::Render(format!("tick {tick} rejected")));
            }
            if sink.should_capture(tick + 1) {
                let hash = outcome.checkpoint.map(|point| point.hash).unwrap_or(0);
                let mut list = frame_list(
                    &presenter,
                    SMOKE_FRAME_SIZE.0,
                    SMOKE_FRAME_SIZE.1,
                    camera_path,
                    presented,
                )?;
                // Arena kit bind: cube leaves when a kit draw is resident.
                engine_render::twin_arena::push_twin_arena(
                    &mut list,
                    &presenter.mesh_keys(),
                    &kit_draws,
                )
                .map_err(|err| WinError::Render(capped(&format!("{err}"))))?;
                let out = presenter.render_offscreen(&list)?;
                if !out.manifest.is_clean() {
                    return Err(WinError::Render("twin frame failed validation".to_string()));
                }
                match out.png {
                    Some(png) => {
                        sink.write_frame(tick, hash, &png)
                            .map_err(|err| WinError::Render(capped(&err.to_string())))?;
                        presented += 1;
                    }
                    None => {
                        return Err(WinError::Render("twin frame carried no bytes".to_string()));
                    }
                }
            }
        }
        sink.note_director_cuts(app.director_cuts(), app.director_shot());
        sink.finish()
            .map_err(|err| WinError::Render(capped(&err.to_string())))
    }

    /// Capture slot for the window session: tape cursor plus the PNG sink.
    /// Built before the loop so a bad dir or tape fails fast with no window.
    pub(crate) struct CaptureSlot {
        pub(crate) sink: CaptureSink,
        pub(crate) tape: TapeInputs,
    }

    /// Arm the window capture: load the tape, validate it against the session
    /// (human slot, inside the tick range) and open the sink. Loud, no window.
    pub(crate) fn setup_capture(
        flags: &Option<capture::CaptureFlags>,
        config: &GameConfig,
    ) -> Result<Option<CaptureSlot>, capture::CaptureError> {
        let Some(flags) = flags else {
            return Ok(None);
        };
        let tape = flags
            .tape
            .as_ref()
            .map(|path| capture::load_tape_file(path.as_path()))
            .transpose()?;
        if let Some(tape) = &tape {
            capture::validate_tape_for_run(&tape.inputs, HUMAN_SLOT, config.ticks)?;
        }
        let meta = capture::ManifestMeta {
            rows: flags.rows.clone(),
            sha: flags.sha.clone(),
            fps: flags.fps,
        };
        let sink = CaptureSink::create(
            &flags.dir,
            flags.every,
            config.seed,
            tape.as_ref(),
            capture::TWIN_RENDERER,
            &meta,
        )?;
        Ok(Some(CaptureSlot {
            sink,
            tape: TapeInputs::new(tape.map(|loaded| loaded.inputs).unwrap_or_default()),
        }))
    }

    /// Headless `--frames` fallback when no display exists: same idle tape, same
    /// checkpoints, offscreen twin when an adapter exists. With capture flags the
    /// same session runs through the capture sink instead (synth frames bound to
    /// the tape), leaving the same PNG sequence plus manifest on disk.
    pub(crate) fn headless_fallback(
        config: GameConfig,
        args: &WinArgs,
        display_err: &WinError,
        capture_flags: &Option<capture::CaptureFlags>,
    ) -> i32 {
        let frames = args.frames.unwrap_or(120);
        let started = Instant::now();
        if let Some(flags) = capture_flags {
            let code = headless_capture(
                config,
                frames,
                flags,
                display_err,
                args.scene,
                args.camera_path,
            );
            if args.timed_smoke && code == 0 {
                let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                match format_timed_smoke(frames, config.ticks.min(frames), elapsed_ms) {
                    Ok(line) => say!("{line}"),
                    Err(err) => {
                        tell!("arena: {err}");
                        return 2;
                    }
                }
            }
            return code;
        }
        match run_headless_smoke_with_options(
            config,
            frames,
            args.camera_path,
            args.mute,
            args.wall_limit_ms(),
            args.present_every,
            args.present_skip,
        ) {
            Ok(report) => {
                say!(
                    "arena headless smoke: ticks={} frames={} hud_cells={} hud_quads={} mixed_frames={} audio=\"{}\" presenter=\"{}\" display=\"{display_err}\"",
                    report.ticks_run,
                    report.frames_presented,
                    report.hud_cells,
                    report.hud_quads,
                    report.mixed_frames,
                    report.audio_notice,
                    report.presenter_notice,
                );
                if args.timed_smoke {
                    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                    match format_timed_smoke(frames, report.ticks_run, elapsed_ms) {
                        Ok(line) => say!("{line}"),
                        Err(err) => {
                            tell!("arena: {err}");
                            return 2;
                        }
                    }
                }
                0
            }
            Err(err) => {
                tell!("arena: {err} (display was: {display_err})");
                2
            }
        }
    }

    /// One-frame draw list for the twin: the HUD clear-only list plus one lit
    /// cube draw when the mesh is resident (proves the forward pass; the HUD
    /// text/quads ride the frame receipt) plus the dressing boxes staged
    /// from `content/arena/dressing.json` (torch/banner silhouettes down the
    /// walls-to-gate axis). The camera is the establishing view, not the
    /// close-up cube view, so both the cube and the dressing stay in frame.
    /// Under `--camera-path arena600` the camera eye rides the fixed seed-2040
    /// orbit for `frame`, so the capture flies while the tick never sees it.
    pub(crate) fn frame_list(
        presenter: &Presenter,
        width: u32,
        height: u32,
        camera_path: Option<engine_game::windowed::CameraPath>,
        frame: u32,
    ) -> Result<DrawList, WinError> {
        let mut list = hud_draw_list(width.max(1), height.max(1))?;
        // Establishing view: replaces the close-up cube camera so the dressing
        // axis is visible; the light still comes from the default scene.
        list.camera = dressing_camera(width.max(1), height.max(1))?;
        engine_game::windowed::apply_camera_orbit(&mut list, frame, camera_path);
        let key = MeshKey::new(1, [0xA1; 32]);
        if presenter.mesh_keys().contains(&key) {
            let scene = engine_render::v1::default_scene(width.max(1), height.max(1))
                .map_err(|err| WinError::Render(capped(&format!("{err}"))))?;
            list.items.push(engine_render::present::DrawItem {
                mesh: key,
                material: scene.material,
                base_color: None,
            });
        }
        let resident = presenter.mesh_keys();
        let _ = push_dressing_draws(&mut list, &resident)?;
        // Twin fighters (RL26): unstaged slots draw nothing.
        engine_render::twin_fighters::push_twin_fighters(&mut list, &resident)
            .map_err(|err| WinError::Render(capped(&format!("{err}"))))?;
        Ok(list)
    }
}
