//! arena_capture: headless plus chart capture helpers for the arena binary.
//!
//! Owner: engine-game bin; consumer: `arena.rs` (`run`, `ArenaApp`, bin
//! tests) plus the `arena.exe --frames N --exit` smoke. Units: ticks,
//! frames, milliseconds, PNG bytes. Determinism: the same tape plus session
//! files the same frames plus manifest; pixels never enter the sim hash.
//! Errors: [`WinError`](engine_game::windowed::WinError); loud, never silent.
//!
//! Helper module of the `arena` binary only (`mod arena_capture;` in
//! `arena.rs`, which builds with the `app` feature): `Cargo.toml` sets
//! `autobins = false`, so Cargo no longer builds this file as a second target
//! of its own, and it has no `main`. The real entry stays `arena`.

/// Headless plus chart capture helpers (the presenter and windowed types they
/// use ride the `app` feature, which the `arena` binary requires).
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

    use crate::headless::{headless_smoke_line, HeadlessWhy, HEADLESS_FLAG};

    use engine_game::break_window::BreakScene;
    use engine_game::capture::{self, CaptureSink, LookTap, TapeInputs};
    use engine_game::director::AuditReport;
    use engine_game::windowed::{
        cap_message as capped, dressing_camera, format_timed_smoke, hud_draw_list,
        push_dressing_draws, run_headless_smoke_with_options, stage_dressing, SceneSelect, WinArgs,
        WinError, SMOKE_FRAME_SIZE,
    };
    use engine_game::{App, GameConfig, HUMAN_SLOT};
    use engine_input::FighterId;
    use engine_render::present::{DrawList, FrameOutput, MeshKey, Presenter, TextureKey};

    mod reference_tier;
    pub(crate) use reference_tier::{run_reference_mode, ReferenceTier};

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

    /// Reference-set flags: `--reference-set` draws the ten reference captures
    /// of one tier headless (no window) and files the PNGs plus the manifest
    /// fragment `tools/reference-captures.mjs` merges. Parsed before chart and
    /// capture flags, so `WinArgs` still refuses every other unknown flag
    /// exactly as before.
    pub(crate) struct ReferenceArgs {
        /// True while `--reference-set` was given.
        pub(crate) enabled: bool,
        /// Root the tier folder plus the fragment go into.
        pub(crate) dir: std::path::PathBuf,
        /// Tier to draw; native when `--reference-tier` is absent.
        pub(crate) tier: ReferenceTier,
    }

    /// Default reference-set root: the tree's `content/reference`.
    pub(crate) const REFERENCE_DEFAULT_DIR: &str = "content/reference";
    /// Native capture canvas, pixels: the manifest's declared size.
    pub(crate) const REFERENCE_SIZE: (u32, u32) = (1280, 720);
    /// Potato capture canvas, pixels: the low tier's declared size.
    pub(crate) const REFERENCE_POTATO_SIZE: (u32, u32) = (320, 180);
    /// The ten tick offsets the camera path is sampled at: six sim-seconds
    /// apart across the fixed seed-2040 `arena600` orbit (600 frames).
    pub(crate) const REFERENCE_TICKS: [u32; 10] = [0, 60, 120, 180, 240, 300, 360, 420, 480, 540];
    /// Longest accepted reference flag path in bytes (mirrors the chart bound).
    pub(crate) const REFERENCE_MAX_PATH_BYTES: usize = 4096;
    /// Fragment file the mode writes beside its PNGs.
    pub(crate) const REFERENCE_FRAGMENT: &str = "fragment.json";

    /// Split the reference-set flags out of `raw`, passing everything else
    /// through in order for chart, capture and `WinArgs`. Unknown
    /// `--reference*` flags, missing values, duplicates and a `--reference-dir`
    /// without `--reference-set` fail loudly; other flags are never touched.
    pub(crate) fn split_reference_args(
        raw: &[String],
    ) -> Result<(ReferenceArgs, Vec<String>), WinError> {
        let mut enabled = false;
        let mut seen_set = false;
        let mut seen_dir = false;
        let mut seen_tier = false;
        let mut dir: Option<String> = None;
        let mut tier: Option<ReferenceTier> = None;
        let mut rest = Vec::with_capacity(raw.len());
        let mut index = 0usize;
        while index < raw.len() {
            match raw[index].as_str() {
                "--reference-set" => {
                    if seen_set {
                        return Err(WinError::BadArgs("duplicate --reference-set".to_string()));
                    }
                    seen_set = true;
                    enabled = true;
                }
                "--reference-dir" => {
                    if seen_dir {
                        return Err(WinError::BadArgs("duplicate --reference-dir".to_string()));
                    }
                    seen_dir = true;
                    index += 1;
                    match raw.get(index) {
                        Some(value) if !value.starts_with("--") => dir = Some(value.clone()),
                        _ => {
                            return Err(WinError::BadArgs(
                                "--reference-dir needs a value".to_string(),
                            ));
                        }
                    }
                }
                "--reference-tier" => {
                    if seen_tier {
                        return Err(WinError::BadArgs("duplicate --reference-tier".to_string()));
                    }
                    seen_tier = true;
                    index += 1;
                    match raw.get(index) {
                        Some(value) if !value.starts_with("--") => {
                            tier = Some(ReferenceTier::parse(value).ok_or_else(|| {
                                WinError::BadArgs(format!("unknown --reference-tier {value}"))
                            })?);
                        }
                        _ => {
                            return Err(WinError::BadArgs(
                                "--reference-tier needs a value".to_string(),
                            ));
                        }
                    }
                }
                other if other.starts_with("--reference") => {
                    return Err(WinError::BadArgs(other.to_string()));
                }
                _ => rest.push(raw[index].clone()),
            }
            index += 1;
        }
        if dir.is_some() && !enabled {
            return Err(WinError::BadArgs(
                "--reference-dir needs --reference-set".to_string(),
            ));
        }
        if tier.is_some() && !enabled {
            return Err(WinError::BadArgs(
                "--reference-tier needs --reference-set".to_string(),
            ));
        }
        let tier = tier.unwrap_or(ReferenceTier::Native);
        let dir = match dir {
            Some(raw) => {
                if raw.is_empty() || raw.len() > REFERENCE_MAX_PATH_BYTES {
                    return Err(WinError::BadArgs(
                        "--reference-dir path rejected".to_string(),
                    ));
                }
                std::path::PathBuf::from(raw)
            }
            None => std::path::PathBuf::from(REFERENCE_DEFAULT_DIR),
        };
        Ok((ReferenceArgs { enabled, dir, tier }, rest))
    }

    /// One reference capture the mode filed: the shot name, its manifest path,
    /// the tick it was taken on and the FNV of the PNG bytes (the tool's
    /// pixel digest is computed over the decoded pixels it reads back).
    pub(crate) struct ReferenceShot {
        /// Shot name inside the tier (`arena600-t000`).
        pub(crate) shot: String,
        /// Filed path relative to the set root (`native/arena600-t000.png`).
        pub(crate) png: String,
        /// Sim tick the capture ran on.
        pub(crate) tick: u32,
        /// FNV-1a of the PNG bytes, hex.
        pub(crate) png_fnv: String,
        /// PNG size in bytes.
        pub(crate) bytes: usize,
    }

    /// `arena600-tNNN`: the shot name for one tick offset, zero-padded to the
    /// three digits the 600-frame path uses.
    pub(crate) fn reference_shot_name(tick: u32) -> String {
        format!("arena600-t{tick:03}")
    }

    /// JSON text escaping for the fragment's one-line strings.
    fn reference_json_text(raw: &str) -> String {
        let mut out = String::with_capacity(raw.len());
        for ch in raw.chars() {
            match ch {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                other => out.push(other),
            }
        }
        out
    }

    /// The fragment `tools/reference-captures.mjs` reads: the tier, the canvas,
    /// the adapter and command line behind every shot, and the ten rows. The
    /// tool re-verifies each PNG and computes the manifest's `pixel_fnv` over
    /// the decoded pixels, so the fragment carries the byte digest it prints.
    #[expect(
        clippy::too_many_arguments,
        reason = "the fragment names tier, canvas, adapter, driver, command, seed and rows"
    )]
    pub(crate) fn reference_fragment_json(
        tier: &str,
        camera: &str,
        seed: u64,
        width: u32,
        height: u32,
        adapter: &str,
        driver: &str,
        command: &str,
        shots: &[ReferenceShot],
    ) -> String {
        let mut out = String::with_capacity(256 + shots.len() * 256);
        out.push_str("{\"schema\":1");
        out.push_str(&format!(",\"tier\":\"{}\"", reference_json_text(tier)));
        out.push_str(&format!(",\"camera\":\"{}\"", reference_json_text(camera)));
        out.push_str(&format!(",\"seed\":{seed}"));
        out.push_str(&format!(",\"width\":{width},\"height\":{height}"));
        out.push_str(&format!(
            ",\"adapter\":\"{}\"",
            reference_json_text(adapter)
        ));
        out.push_str(&format!(",\"driver\":\"{}\"", reference_json_text(driver)));
        out.push_str(&format!(
            ",\"command\":\"{}\"",
            reference_json_text(command)
        ));
        out.push_str(",\"shots\":[");
        for (at, shot) in shots.iter().enumerate() {
            if at > 0 {
                out.push(',');
            }
            out.push_str(&format!(
                "{{\"shot\":\"{}\",\"png\":\"{}\",\"tick\":{},\"width\":{width},\"height\":{height},\"seed\":{seed},\"png_fnv\":\"{}\",\"bytes\":{}}}",
                reference_json_text(&shot.shot),
                reference_json_text(&shot.png),
                shot.tick,
                reference_json_text(&shot.png_fnv),
                shot.bytes
            ));
        }
        out.push_str("]}");
        out
    }

    /// The kit draws the twin binds: one triple per staged arena-kit draw.
    pub(crate) type KitDraws = Vec<(MeshKey, engine_render::v1::PbrMaterial, Option<TextureKey>)>;

    /// The config the reference run draws: the arena600 seed and enough ticks
    /// to reach the last capture offset.
    pub(crate) fn reference_config(config: &GameConfig) -> GameConfig {
        let mut config = *config;
        config.seed = engine_game::windowed::CameraPath::Arena600.seed();
        let last = REFERENCE_TICKS[REFERENCE_TICKS.len() - 1];
        config.ticks = config.ticks.max(last + 1);
        config
    }

    /// Stage the twin for one reference tier: the tier canvas, the proof cube,
    /// the dressing, the two fighter bodies and the arena kit.
    fn reference_stage(
        presenter: &mut Presenter,
        config: &GameConfig,
        size: (u32, u32),
    ) -> Result<KitDraws, WinError> {
        presenter.resize(size.0, size.1)?;
        let key = MeshKey::new(1, [0xA1; 32]);
        let _ = presenter.upload_mesh(key, &engine_render::v1::cube());
        let _ = stage_dressing(presenter);
        engine_render::twin_fighters::stage_twin_fighters(presenter, config.fighters)
            .map_err(|error| WinError::Render(error.to_string()))?;
        let report = engine_game::arena_kit::stage_kit_or_default(presenter);
        Ok(kit_triples(&report))
    }

    /// Advance the sim one tick and, on a [`REFERENCE_TICKS`] offset, render
    /// the frame at the reference canvas and file its PNG. Returns the row plus
    /// the frame's adapter and driver on a shot tick, `None` on any other tick.
    #[expect(
        clippy::too_many_arguments,
        reason = "one tick needs the presenter, app, scene, draws, fighter, tier and dir"
    )]
    fn reference_tick(
        presenter: &mut Presenter,
        app: &mut App,
        crate_scene: &mut Option<BreakScene>,
        kit_draws: &[(MeshKey, engine_render::v1::PbrMaterial, Option<TextureKey>)],
        human: FighterId,
        tier: ReferenceTier,
        dir: &std::path::Path,
        tick: u32,
    ) -> Result<Option<(ReferenceShot, String, String)>, WinError> {
        let input = engine_input::FighterInput {
            tick,
            fighter: human,
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: engine_input::Buttons::NONE,
        };
        let outcome = app.tick(input);
        if !outcome.input_accepted {
            return Err(WinError::Render(format!("tick {tick} rejected")));
        }
        observe_crate(crate_scene, &outcome);
        if !REFERENCE_TICKS.contains(&tick) {
            return Ok(None);
        }
        let (width, height) = tier.size();
        let out = capture_frame_output(
            presenter,
            app,
            crate_scene,
            &FrameInputs {
                camera_path: Some(engine_game::windowed::CameraPath::Arena600),
                kit_draws,
                night: false,
                torch_actors: None,
                frame: tick,
                tick,
                size: (width, height),
            },
        )?;
        let Some(png) = out.png else {
            return Err(WinError::Render(format!("tick {tick} carried no bytes")));
        };
        let shot = reference_shot_name(tick);
        let path = dir.join(format!("{shot}.png"));
        std::fs::write(&path, &png)
            .map_err(|error| WinError::Render(format!("{}: {error}", path.display())))?;
        let png_fnv = engine_game::capture::fnv_hex(&png);
        say!(
            "arena reference: tier={} shot={shot} tick={tick} size={width}x{height} png=\"{}\" png_fnv={png_fnv} bytes={}",
            tier.name(),
            path.display(),
            png.len()
        );
        Ok(Some((
            ReferenceShot {
                png: format!("{}/{shot}.png", tier.name()),
                shot,
                tick,
                png_fnv,
                bytes: png.len(),
            },
            out.receipt.adapter,
            out.receipt.driver,
        )))
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
        // VS7: the night preset bakes the arena night rig (the moonlit sky,
        // its ambient and its fog), so the placement's torches are the light
        // the frame reads. Off by default: the day capture keeps the day rig
        // and stays byte-equal.
        let night = engine_game::windowed::night_enabled(
            std::env::var(engine_game::windowed::NIGHT_ENV)
                .ok()
                .as_deref(),
        );
        // One adapter probe: the twin owns it when present, otherwise the synth
        // path runs (same receipts, static pixels, never a fake orbit).
        let report = match Presenter::init_offscreen_with_preset(night) {
            Ok(presenter) => {
                twin_headless_capture(config, ticks, presenter, tape, flags, camera_path, night)
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

    /// Textured kit triples from a staged [`KitReport`](engine_game::arena_kit::KitReport):
    /// one `(key, material, base_color)` per draw in placement order, preserving
    /// [`KitDraw::base_color`](engine_game::arena_kit::KitDraw) (`None` stays
    /// `None`, so untextured draws bind the 1x1 white fallback). Pure, no GPU,
    /// no file IO; the caller owns residency (missing keys draw nothing).
    /// Units: meters for meshes, 0..=1 factors for materials. Determinism: the
    /// same report gives the same triples in the same order; pixels stay
    /// presentation (HASH-LAW).
    ///
    /// Consumer: `twin_headless_capture` (headless, staged once) plus the window
    /// `frame_list` (shared twin list, imported once per call); both bind through
    /// `push_twin_arena_textured` with cube-swap semantics.
    ///
    /// Caller: `twin_headless_capture` plus `frame_list` (same file) keep the
    /// base-color lane live, so the twin draws the recooked kit textured.
    pub(crate) fn kit_triples(
        report: &engine_game::arena_kit::KitReport,
    ) -> Vec<(MeshKey, engine_render::v1::PbrMaterial, Option<TextureKey>)> {
        report
            .draws
            .iter()
            .map(|draw| (draw.key, draw.material.clone(), draw.base_color))
            .collect()
    }

    /// The seal's editorial verdict: a clean cut log gives the receipt line, any
    /// continuity flag is an error that names every count. The seal prints
    /// either and files the capture regardless (`seal_capture_cut_log`).
    fn cut_log_audit_line(audit: AuditReport) -> Result<String, WinError> {
        if audit.is_clean() {
            return Ok(format!(
                "arena capture: cut log clean cuts={} flags=0",
                audit.cuts
            ));
        }
        Err(WinError::Render(format!(
            "cut log audit: {} flags (jump {} line {} hold {} slew {} stale {})",
            audit.total_flags(),
            audit.jump_cuts,
            audit.line_crosses,
            audit.short_holds,
            audit.slew_overs,
            audit.stale_cuts
        )))
    }

    /// File one twin capture's cut log and print the editorial receipt. A
    /// flagged log is reported loudly but does not fail the capture: a plain
    /// brawl already audits with jump flags, so refusing would fail normal
    /// captures.
    fn seal_capture_cut_log(sink: &mut CaptureSink, app: &App) {
        sink.note_director_cuts(app.director_cuts(), app.director_shot());
        match cut_log_audit_line(app.director_audit()) {
            Ok(line) => say!("{line}"),
            Err(flagged) => say!("arena capture: {flagged}"),
        }
    }

    /// One captured twin frame's inputs: the frame's own camera, the kit
    /// draws it binds, the night preset and the placement's torch actors.
    /// One struct, so the frame helper stays a short call.
    struct FrameInputs<'a> {
        /// The camera path the capture flies (None keeps the establishing view).
        camera_path: Option<engine_game::windowed::CameraPath>,
        /// The staged kit draws the frame binds.
        kit_draws: &'a [(
            engine_render::present::MeshKey,
            engine_render::v1::PbrMaterial,
            Option<TextureKey>,
        )],
        /// The night preset: the sun drops and the torches carry the picture.
        night: bool,
        /// The placement's torch actors, when the placement parsed.
        torch_actors: Option<&'a [engine_game::arena_kit::KitActor]>,
        /// The capture's frame index (the orbit eye) and the sim tick.
        frame: u32,
        /// The sim tick the flame flickers on.
        tick: u32,
        /// The canvas the frame renders at (pixels).
        size: (u32, u32),
    }

    /// One captured twin frame: the draw list, the kit bind, the HUD bars,
    /// the crate and the night lights, then the offscreen render. The output
    /// (PNG bytes plus the adapter receipt) comes back; the sink writes the
    /// bytes. A frame that fails validation or carries no bytes is refused
    /// loudly.
    fn capture_frame_output(
        presenter: &mut Presenter,
        app: &App,
        crate_scene: &mut Option<BreakScene>,
        inputs: &FrameInputs<'_>,
    ) -> Result<FrameOutput, WinError> {
        let mut list = frame_list(
            presenter,
            inputs.size.0,
            inputs.size.1,
            inputs.camera_path,
            inputs.frame,
        )?;
        // Arena kit bind: cube leaves when a kit draw is resident.
        // Textured cutover keeps `base_color` on every triple.
        engine_render::twin_arena::push_twin_arena_textured(
            &mut list,
            &presenter.mesh_keys(),
            inputs.kit_draws,
        )
        .map_err(|err| WinError::Render(capped(&format!("{err}"))))?;
        let overlay = engine_game::windowed::duel_overlay_for_app(app)
            .map_err(|err| WinError::Render(capped(&format!("{err}"))))?;
        engine_game::windowed::stage_hp_bars(presenter, &overlay);
        push_crate(crate_scene, presenter, &mut list);
        // VS7: the night preset drops the sun and the placement's torch
        // actors burn, so the torches carry the picture. Off by default: the
        // day capture keeps the built-in light pair and stays byte-equal.
        if inputs.night {
            feed_night_lights(presenter, &mut list, inputs.torch_actors, inputs.tick);
        }
        let out = presenter.render_offscreen(&list)?;
        if !out.manifest.is_clean() {
            return Err(WinError::Render("twin frame failed validation".to_string()));
        }
        if out.png.is_none() {
            return Err(WinError::Render("twin frame carried no bytes".to_string()));
        }
        Ok(out)
    }

    /// The PNG bytes of one captured twin frame: the sink's half of
    /// [`capture_frame_output`], which the reference-set mode calls for the
    /// adapter receipt it needs in the manifest fragment.
    fn capture_frame(
        presenter: &mut Presenter,
        app: &App,
        crate_scene: &mut Option<BreakScene>,
        inputs: &FrameInputs<'_>,
    ) -> Result<Vec<u8>, WinError> {
        let out = capture_frame_output(presenter, app, crate_scene, inputs)?;
        out.png
            .ok_or_else(|| WinError::Render("twin frame carried no bytes".to_string()))
    }

    /// The night preset's light for one captured frame (VS7): the placement's
    /// torch actors burn and the sun drops to a moonlit fraction, so the
    /// torches carry the picture. A refused light set keeps the built-in
    /// pair and never fails the frame. Off by default, so every existing
    /// capture stays byte-equal.
    fn feed_night_lights(
        presenter: &mut Presenter,
        list: &mut DrawList,
        actors: Option<&[engine_game::arena_kit::KitActor]>,
        tick: u32,
    ) {
        if let Some(actors) = actors {
            if let Ok(lights) = engine_game::arena_kit::torch_feed::torch_lights_at(actors, tick) {
                presenter.set_torch_lights(lights);
            }
        }
        engine_game::arena_kit::torch_feed::apply_night_sun(list);
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
        night: bool,
    ) -> Result<capture::CaptureReport, WinError> {
        let inputs = tape
            .as_ref()
            .map(|loaded| loaded.inputs.as_slice())
            .unwrap_or(&[]);
        capture::validate_tape_for_run(inputs, HUMAN_SLOT, config.ticks)
            .map_err(|err| WinError::Render(capped(&err.to_string())))?;
        let mut sink = open_twin_sink(config, tape.as_ref(), flags)?;
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
        // Textured triples keep `base_color` so the kit draws textured.
        let kit_report = engine_game::arena_kit::stage_kit_or_default(&mut presenter);
        let kit_draws = kit_triples(&kit_report);
        let mut app = App::new(config)?;
        // The center crate's break picture rides the twin frames like the window's (RZ4w).
        let mut crate_scene = BreakScene::new(&app.live_match().config());
        let human = FighterId::new(HUMAN_SLOT).ok_or(WinError::BadConfig("fighters"))?;
        let mut cursor = TapeInputs::new(tape.map(|loaded| loaded.inputs).unwrap_or_default());
        let mut presented = 0u32;
        // VS7: the placement's torch actors burn, so the torches carry the
        // picture. Off by default: the day capture keeps the built-in light
        // pair and stays byte-equal.
        let torch_actors = engine_game::arena_kit::kit_actors().ok();
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
            observe_crate(&mut crate_scene, &outcome);
            if sink.should_capture(tick + 1) {
                let hash = outcome.checkpoint.map(|point| point.hash).unwrap_or(0);
                let png = capture_frame(
                    &mut presenter,
                    &app,
                    &mut crate_scene,
                    &FrameInputs {
                        camera_path,
                        kit_draws: &kit_draws,
                        night,
                        torch_actors: torch_actors.as_deref(),
                        frame: presented,
                        tick,
                        size: SMOKE_FRAME_SIZE,
                    },
                )?;
                sink.write_frame(tick, hash, &png)
                    .map_err(|err| WinError::Render(capped(&err.to_string())))?;
                presented += 1;
            }
        }
        if let Some(scene) = crate_scene.as_ref() {
            say!("{}", scene.receipt_line());
            say!("{}", scene.stats().receipt_line());
        }
        seal_capture_cut_log(&mut sink, &app);
        sink.finish()
            .map_err(|err| WinError::Render(capped(&err.to_string())))
    }

    /// The twin capture's PNG sink: manifest rows from the flags, bound to the tape.
    fn open_twin_sink(
        config: GameConfig,
        tape: Option<&capture::LoadedTape>,
        flags: &capture::CaptureFlags,
    ) -> Result<CaptureSink, WinError> {
        let meta = capture::ManifestMeta {
            rows: flags.rows.clone(),
            sha: flags.sha.clone(),
            fps: flags.fps,
        };
        capture::CaptureSink::create(
            &flags.dir,
            flags.every,
            config.seed,
            tape,
            capture::TWIN_RENDERER,
            &meta,
        )
        .map_err(|err| WinError::Render(capped(&err.to_string())))
    }

    /// Twin capture, break picture: one accepted tick for the crate (RZ4w). A refusal drops
    /// the picture loudly; the capture goes on without the crate.
    fn observe_crate(scene: &mut Option<BreakScene>, outcome: &engine_game::TickOutcome) {
        if let Some(Err(err)) = scene.as_mut().map(|s| s.observe_outcome(outcome)) {
            tell!("arena notice: break picture dropped: {err}");
            *scene = None;
        }
    }

    /// Twin capture, break picture: the crate's boxes onto the frame list (RZ4w).
    fn push_crate(scene: &mut Option<BreakScene>, presenter: &mut Presenter, list: &mut DrawList) {
        let eye = list.camera.eye.to_array();
        if let Some(Err(why)) = scene.as_mut().map(|s| s.push(presenter, list, 1.0, eye)) {
            tell!("arena notice: break picture dropped: {why}");
            *scene = None;
        }
    }

    /// Capture slot for the window session: tape cursor plus the PNG sink.
    /// Built before the loop so a bad dir or tape fails fast with no window.
    pub(crate) struct CaptureSlot {
        pub(crate) sink: CaptureSink,
        pub(crate) tape: TapeInputs,
        pub(crate) look_tap: LookTap,
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
            look_tap: LookTap::new(),
        }))
    }

    /// Headless `--frames` run: same idle tape, same checkpoints, offscreen
    /// twin when an adapter exists. With capture flags the same session runs
    /// through the capture sink instead (synth frames bound to the tape),
    /// leaving the same PNG sequence plus manifest on disk.
    fn headless_receipts(
        config: GameConfig,
        args: &WinArgs,
        display_err: &WinError,
        capture_flags: &Option<capture::CaptureFlags>,
        why: HeadlessWhy,
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
                let display = match why {
                    HeadlessWhy::Forced => "none requested".to_string(),
                    HeadlessWhy::NoDisplay => display_err.to_string(),
                };
                say!("{}", headless_smoke_line(&report, why, &display));
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

    /// Headless `--frames` fallback when no display exists. Consumer: the
    /// binary's `EventLoop::new` failure arm; the receipt names
    /// `headless=no-display` and carries the display error.
    pub(crate) fn headless_fallback(
        config: GameConfig,
        args: &WinArgs,
        display_err: &WinError,
        capture_flags: &Option<capture::CaptureFlags>,
    ) -> i32 {
        headless_receipts(
            config,
            args,
            display_err,
            capture_flags,
            HeadlessWhy::NoDisplay,
        )
    }

    /// `--headless`: the same fixed-tick session with no window and no event
    /// loop. The shipped check runs this on a runner with no GPU adapter, so
    /// the game plays instead of being skipped by name; the receipt carries
    /// `headless=forced` and the last checkpoint hash.
    pub(crate) fn forced_headless(
        config: GameConfig,
        args: &WinArgs,
        capture_flags: &Option<capture::CaptureFlags>,
    ) -> i32 {
        let no_window = WinError::NoDisplay("--headless".to_string());
        headless_receipts(config, args, &no_window, capture_flags, HeadlessWhy::Forced)
    }

    /// The `--headless` gate: `None` when the flag is off; `Some(2)` when a
    /// window-only mode was also asked for; else `Some(code)` from the no-window
    /// session. `window_only` names every flag whose promise needs the window,
    /// so a shipped run never silently drops a mode.
    pub(crate) fn headless_gate(
        headless: bool,
        window_only: bool,
        config: GameConfig,
        args: &WinArgs,
        capture_flags: &Option<capture::CaptureFlags>,
    ) -> Option<i32> {
        if !headless {
            return None;
        }
        if window_only || args.backend.is_some() {
            tell!(
                "arena: {HEADLESS_FLAG} plays the plain session; --play seats, --staged-fight, --spear, --height, --legionary, --speed-demo, {} and --backend need the window",
                crate::arena_args::VIEW_BRIEF_FLAG
            );
            return Some(2);
        }
        Some(forced_headless(config, args, capture_flags))
    }

    /// One baked kit actor as the twin binds it: mesh key, material and the
    /// texture key (`None` when the actor has none or it is not resident).
    type KitTriple = (MeshKey, engine_render::v1::PbrMaterial, Option<TextureKey>);

    /// Cached kit triples for `frame_list`: the kit glTF import parsed once per
    /// process and reused on every frame. Stores the unfiltered
    /// `(key, material, texture)` per baked actor; each call filters `texture`
    /// against the presenter's resident set, so residency still decides the
    /// `None` lanes per frame. Same triples as a fresh `import_kit` for the
    /// same content dir (the dir is stable per process, resolved from the exe
    /// layout). A refused import fails loudly and caches nothing, so the next
    /// frame retries. No GPU upload here; init already uploaded.
    ///
    /// Caller: `frame_list` (same file) keeps the twin kit bind live without
    /// re-parsing the ~45 MB kit on every frame.
    fn cached_kit_source() -> Result<&'static [KitTriple], WinError> {
        static CACHE: std::sync::OnceLock<Vec<KitTriple>> = std::sync::OnceLock::new();
        if let Some(cached) = CACHE.get() {
            return Ok(cached.as_slice());
        }
        let imported =
            engine_game::arena_kit::import_kit(&engine_game::arena_kit::kit_content_dir())
                .map_err(|err| WinError::Render(capped(&format!("{err}"))))?;
        let triples: Vec<KitTriple> = imported
            .baked
            .iter()
            .map(|baked| (baked.key, baked.material.clone(), baked.texture))
            .collect();
        // First-frame race from two threads keeps the winner: both triples
        // come from the same content dir, so either copy reads identical.
        let _ = CACHE.set(triples);
        CACHE
            .get()
            .map(Vec::as_slice)
            .ok_or_else(|| WinError::Render("kit cache missing after init".to_string()))
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
                normal_map: None,
                orm: None,
                emissive: None,
            });
        }
        let resident = presenter.mesh_keys();
        let _ = push_dressing_draws(&mut list, &resident)?;
        // Twin fighters (RL26): unstaged slots draw nothing.
        engine_render::twin_fighters::push_twin_fighters(&mut list, &resident)
            .map_err(|err| WinError::Render(capped(&format!("{err}"))))?;
        // Arena kit for the window twin (RL21 slice 1): textured cube-swap when
        // resident, cube stays on miss. Triples come from the once-per-process
        // cache (CPU only, no GPU upload here; the window/headless init
        // already uploaded); a refused import fails loudly, never a fake
        // frame. `None` lanes draw untextured.
        let tex_resident = presenter.texture_keys();
        let source = cached_kit_source()?;
        let kit_binds: Vec<(MeshKey, engine_render::v1::PbrMaterial, Option<TextureKey>)> = source
            .iter()
            .map(|(key, material, texture)| {
                (
                    *key,
                    material.clone(),
                    texture.filter(|key| tex_resident.contains(key)),
                )
            })
            .collect();
        engine_render::twin_arena::push_twin_arena_textured(&mut list, &resident, &kit_binds)
            .map_err(|err| WinError::Render(capped(&format!("{err}"))))?;
        Ok(list)
    }

    #[cfg(test)]
    mod chart_unit_tests {
        use super::*;

        #[test]
        fn chart_gamma_oracle_holds_ends_and_monotonic() {
            // Oracle: evidence gamma maps 0 to 0 and 1 to 255.
            assert_eq!(chart_linear_to_srgb_u8(0.0), 0);
            assert_eq!(chart_linear_to_srgb_u8(1.0), 255);
            // Clamps hold outside 0..=1 so bad linear never wraps.
            assert_eq!(chart_linear_to_srgb_u8(-0.5), 0);
            assert_eq!(chart_linear_to_srgb_u8(2.0), 255);
            // Monotonic rise proves the curve bends the right way.
            let steps = [0.0, 0.25, 0.5, 0.75, 1.0];
            let mapped: Vec<u8> = steps.iter().map(|v| chart_linear_to_srgb_u8(*v)).collect();
            for pair in mapped.windows(2) {
                assert!(pair[0] < pair[1], "gamma rises: {mapped:?}");
            }
        }

        #[test]
        fn chart_adler32_empty_is_one_and_stable() {
            // Standard Adler-32: empty input is 1.
            assert_eq!(chart_adler32(&[]), 1);
            let bytes = [1u8, 2, 3, 4];
            assert_eq!(chart_adler32(&bytes), chart_adler32(&bytes));
            assert_ne!(chart_adler32(&bytes), chart_adler32(&[]));
        }

        #[test]
        fn chart_crc32_table_is_ieee_and_update_stable() {
            // IEEE polynomial table anchors the PNG checksums.
            let table = chart_crc32_table();
            assert_eq!(table[0], 0);
            assert_eq!(table[1], 0x7707_3096);
            let tag = *b"IHDR";
            let first = chart_crc32_update(0xFFFF_FFFF, &tag);
            assert_eq!(first, chart_crc32_update(0xFFFF_FFFF, &tag));
            assert_ne!(first, 0xFFFF_FFFF);
        }

        #[test]
        fn chart_png_bytes_are_png_and_deterministic() {
            // Caller files these bytes beside the manifest, so magic plus
            // determinism is the contract.
            let pixels = [
                255u8, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ];
            let first = chart_encode_png_rgba8(&pixels, 2, 2).expect("2x2 encodes");
            let second = chart_encode_png_rgba8(&pixels, 2, 2).expect("2x2 encodes x2");
            assert_eq!(first, second);
            assert_eq!(&first[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
            assert!(chart_encode_png_rgba8(&pixels, 2, 0).is_err());
            assert!(chart_encode_png_rgba8(&pixels[..4], 2, 2).is_err());
        }

        #[test]
        fn chart_manifest_json_carries_viewport_target_seed() {
            let rows = engine_render::material_layer::chart();
            let live: Vec<[f32; 3]> = rows.iter().map(|row| row.expected).collect();
            let json = chart_manifest_json(&rows, &live, 256, 144, 0.0, 1e-5, 2040);
            let again = chart_manifest_json(&rows, &live, 256, 144, 0.0, 1e-5, 2040);
            assert_eq!(json, again);
            assert!(
                json.contains("\"viewport\":[1600,900]"),
                "viewport pinned: {json}"
            );
            assert!(
                json.contains("\"target\":[256,144]"),
                "target pinned: {json}"
            );
            assert!(json.contains("\"seed\":2040"), "seed pinned: {json}");
            assert!(json.contains("cloth/front"), "rows named: {json}");
        }

        #[test]
        fn split_chart_args_passes_plain_flags_through_in_order() {
            // Arena splits chart flags before WinArgs, so plain flags ride through untouched.
            let raw = [
                "arena".to_string(),
                "--frames".to_string(),
                "3".to_string(),
                "--exit".to_string(),
            ];
            let (chart, rest) = split_chart_args(&raw).expect("plain flags pass");
            assert!(!chart.enabled);
            assert_eq!(chart.dir, std::path::PathBuf::from(CHART_DEFAULT_DIR));
            assert_eq!(rest, raw);
        }

        #[test]
        fn kit_triples_carry_base_color() {
            // Triples preserve `KitDraw.base_color` in placement order; `None`
            // stays `None` so untextured draws bind the 1x1 white fallback.
            // A dropped lane would render every kit draw flat grey.
            use engine_game::arena_kit::{KitDraw, KitReport};
            use engine_render::present::{MeshKey as BinMeshKey, TextureKey as BinTextureKey};
            let material = engine_render::v1::PbrMaterial {
                base_color: [0.5, 0.5, 0.5],
                metallic: 0.0,
                roughness: 0.9,
                exposure: 1.0,
            };
            let tex_a = BinTextureKey::new(0xB001, [0xA1; 32]);
            let tex_b = BinTextureKey::new(0xB002, [0xA2; 32]);
            let report = KitReport {
                draws: vec![
                    KitDraw {
                        key: BinMeshKey::new(0x4717_0001, [0x11; 32]),
                        material: material.clone(),
                        base_color: Some(tex_a),
                        normal_map: None,
                    },
                    KitDraw {
                        key: BinMeshKey::new(0x4717_0002, [0x22; 32]),
                        material: material.clone(),
                        base_color: None,
                        normal_map: None,
                    },
                    KitDraw {
                        key: BinMeshKey::new(0x4717_0003, [0x33; 32]),
                        material: material.clone(),
                        base_color: Some(tex_b),
                        normal_map: None,
                    },
                ],
                assets_staged: 2,
                triangles: 100,
                skipped: 0,
                material_fallbacks: 0,
                ..KitReport::default()
            };
            let triples = kit_triples(&report);
            assert_eq!(triples.len(), 3, "one triple per draw");
            assert_eq!(triples[0].0, report.draws[0].key, "order stable");
            assert_eq!(triples[1].0, report.draws[1].key, "order stable");
            assert_eq!(triples[2].0, report.draws[2].key, "order stable");
            assert_eq!(triples[0].2, Some(tex_a), "textured lane kept");
            assert_eq!(triples[1].2, None, "untextured lane stays None");
            assert_eq!(triples[2].2, Some(tex_b), "textured lane kept");
            assert_eq!(triples[0].1, material, "factors ride with the key");
        }
    }

    #[cfg(test)]
    mod reference_unit_tests {
        use super::*;

        fn strings(items: &[&str]) -> Vec<String> {
            items.iter().map(|item| (*item).to_string()).collect()
        }

        #[test]
        fn reference_flags_split_off_before_capture_flags() {
            let raw = strings(&[
                "arena",
                "--reference-set",
                "--reference-tier",
                "potato",
                "--reference-dir",
                "out/ref",
                "--frames",
                "2",
                "--exit",
            ]);
            let (reference, rest) = split_reference_args(&raw).expect("split");
            assert!(reference.enabled);
            assert_eq!(reference.tier, ReferenceTier::Potato);
            assert_eq!(reference.dir, std::path::PathBuf::from("out/ref"));
            assert_eq!(rest, strings(&["arena", "--frames", "2", "--exit"]));
            // The default root is the tree's content dir and native is the
            // default tier, so the old command keeps its meaning.
            let bare = strings(&["arena", "--reference-set"]);
            let (default, _) = split_reference_args(&bare).expect("split");
            assert_eq!(default.dir, std::path::PathBuf::from(REFERENCE_DEFAULT_DIR));
            assert_eq!(default.tier, ReferenceTier::Native);
            // Loud refusals: duplicates, an unknown reference flag or tier, and
            // a dir or tier without the switch that would capture nothing.
            for bad in [
                strings(&["arena", "--reference-set", "--reference-set"]),
                strings(&[
                    "arena",
                    "--reference-set",
                    "--reference-dir",
                    "a",
                    "--reference-dir",
                    "b",
                ]),
                strings(&[
                    "arena",
                    "--reference-set",
                    "--reference-tier",
                    "native",
                    "--reference-tier",
                    "potato",
                ]),
                strings(&["arena", "--reference-set", "--reference-tier", "web"]),
                strings(&["arena", "--reference-set", "--reference-fps", "30"]),
                strings(&["arena", "--reference-set", "--reference-dir"]),
                strings(&["arena", "--reference-set", "--reference-tier"]),
                strings(&["arena", "--reference-dir", "out"]),
                strings(&["arena", "--reference-tier", "potato"]),
            ] {
                assert!(split_reference_args(&bad).is_err(), "{bad:?}");
            }
        }

        #[test]
        fn reference_tiers_declare_their_canvas_and_quality() {
            assert_eq!(ReferenceTier::parse("native"), Some(ReferenceTier::Native));
            assert_eq!(ReferenceTier::parse("potato"), Some(ReferenceTier::Potato));
            assert_eq!(ReferenceTier::parse("web"), None, "the page shoots web");
            assert_eq!(ReferenceTier::Native.name(), "native");
            assert_eq!(ReferenceTier::Potato.name(), "potato");
            assert_eq!(ReferenceTier::Native.size(), (1280, 720));
            assert_eq!(ReferenceTier::Potato.size(), (320, 180));
            assert_eq!(ReferenceTier::Native.quality(), None);
            assert_eq!(
                ReferenceTier::Potato.quality(),
                Some(engine_render::quality::Quality::Potato),
                "the potato shot stages under the low tier's caps"
            );
        }

        #[test]
        fn reference_shots_are_ten_distinct_ticks_on_the_orbit() {
            assert_eq!(REFERENCE_TICKS.len(), 10, "ten captures per tier");
            let last = REFERENCE_TICKS[REFERENCE_TICKS.len() - 1];
            assert!(
                last < engine_game::windowed::ARENA600_FRAMES,
                "every tick stays inside the 600-frame path"
            );
            for pair in REFERENCE_TICKS.windows(2) {
                assert!(pair[0] < pair[1], "ticks ascend: {REFERENCE_TICKS:?}");
            }
            let mut names: Vec<String> = REFERENCE_TICKS
                .iter()
                .map(|tick| reference_shot_name(*tick))
                .collect();
            assert_eq!(names.len(), 10);
            assert_eq!(names[0], "arena600-t000");
            assert_eq!(names[9], "arena600-t540");
            names.sort();
            names.dedup();
            assert_eq!(names.len(), 10, "shot names are unique");
        }

        #[test]
        fn reference_fragment_carries_every_shot_and_its_source() {
            let shots = vec![
                ReferenceShot {
                    shot: reference_shot_name(0),
                    png: format!("{}/arena600-t000.png", ReferenceTier::Native.name()),
                    tick: 0,
                    png_fnv: "0123456789abcdef".to_string(),
                    bytes: 4096,
                },
                ReferenceShot {
                    shot: reference_shot_name(540),
                    png: format!("{}/arena600-t540.png", ReferenceTier::Native.name()),
                    tick: 540,
                    png_fnv: "fedcba9876543210".to_string(),
                    bytes: 8192,
                },
            ];
            let json = reference_fragment_json(
                ReferenceTier::Native.name(),
                "arena600",
                2040,
                ReferenceTier::Native.size().0,
                ReferenceTier::Native.size().1,
                "RTX \"3050\"",
                "driver",
                "arena --reference-set",
                &shots,
            );
            assert!(json.starts_with("{\"schema\":1"), "{json}");
            assert!(json.contains("\"tier\":\"native\""), "{json}");
            assert!(json.contains("\"camera\":\"arena600\""), "{json}");
            assert!(json.contains("\"seed\":2040"), "{json}");
            assert!(json.contains("\"width\":1280,\"height\":720"), "{json}");
            // A quote in the adapter name is escaped, so the fragment stays one
            // valid JSON document.
            assert!(json.contains("\"adapter\":\"RTX \\\"3050\\\"\""), "{json}");
            assert!(
                json.contains("\"command\":\"arena --reference-set\""),
                "{json}"
            );
            assert!(json.contains("\"shot\":\"arena600-t000\""), "{json}");
            assert!(
                json.contains("\"png\":\"native/arena600-t540.png\""),
                "{json}"
            );
            assert!(json.contains("\"tick\":540"), "{json}");
            assert!(json.contains("\"png_fnv\":\"fedcba9876543210\""), "{json}");
            assert!(json.contains("\"bytes\":8192"), "{json}");
            assert_eq!(json.matches("\"shot\":").count(), 2, "{json}");
            let potato = reference_fragment_json(
                ReferenceTier::Potato.name(),
                "arena600",
                2040,
                ReferenceTier::Potato.size().0,
                ReferenceTier::Potato.size().1,
                "adapter",
                "driver",
                "arena --reference-set --reference-tier potato",
                &[],
            );
            assert!(potato.contains("\"tier\":\"potato\""), "{potato}");
            assert!(potato.contains("\"width\":320,\"height\":180"), "{potato}");
            assert!(potato.contains("\"shots\":[]"), "{potato}");
        }
    }

    #[cfg(test)]
    mod cut_audit_tests {
        use super::*;
        use engine_game::director::{
            audit_cuts, Cut, CutReason, ShotKind, ShotSide, MIN_SHOT_TICKS,
        };

        /// Behaviour: a running session's own cut log, read through the
        /// accessor the capture seal uses, gets a receipt that agrees with the
        /// audit: clean gives the cut count, flagged names the same flag total.
        /// A real brawl repeats a peak past the hold floor, which the audit
        /// flags as a jump, so this session is refused; the test pins that the
        /// seal and the audit never disagree.
        #[test]
        fn live_session_receipt_agrees_with_the_audit() {
            let mut app = App::new(GameConfig {
                seed: 2040,
                ticks: 600,
                fighters: 2,
                half_extent_m: 20.0,
            })
            .expect("charted config");
            let human = FighterId::new(HUMAN_SLOT).expect("human slot");
            for tick in 0..600u32 {
                // Scripted brawl tape: strafe plus an attack every 12 ticks, so
                // the session actually lands beats and its director cuts.
                let outcome = app.tick(engine_input::FighterInput {
                    tick,
                    fighter: human,
                    mx: if tick % 24 < 12 { 60 } else { -60 },
                    mz: 0,
                    facing: 0,
                    buttons: if tick.is_multiple_of(12) {
                        engine_input::Buttons(engine_input::fighter::ATTACK)
                    } else {
                        engine_input::Buttons::NONE
                    },
                });
                assert!(outcome.input_accepted, "tick {tick} accepted");
            }
            let cuts = app.director_cuts().len();
            assert!(
                cuts >= 2,
                "a 600-tick brawl cuts more than once, got {cuts}"
            );
            let audit = app.director_audit();
            let flags = audit.total_flags();
            assert_eq!(usize::try_from(audit.cuts).expect("count fits"), cuts);
            match cut_log_audit_line(audit) {
                Ok(line) => {
                    assert_eq!(flags, 0, "only a clean log files");
                    assert_eq!(
                        line,
                        format!("arena capture: cut log clean cuts={cuts} flags=0")
                    );
                }
                Err(err) => {
                    assert!(flags > 0, "only a flagged log is an error");
                    let text = format!("{err:?}");
                    assert!(
                        text.contains(&format!("cut log audit: {flags} flags")),
                        "names the audit total: {text}"
                    );
                }
            }
        }

        /// Failure: a cut log whose continuity flags fire (a jump cut plus the
        /// short hold under it) gives an error that names every count, so the
        /// seal can print what is wrong with the clip.
        #[test]
        fn flagged_cut_log_names_every_flag() {
            // A real cold open: the framing changes, so only the second cut is a jump.
            let open = Cut {
                tick: 0,
                from_kind: ShotKind::Wide,
                to_kind: ShotKind::Duel,
                from_focus: 0,
                to_focus: 0,
                side: ShotSide::Left,
                crossed_line: false,
                reason: CutReason::Hit,
                beat_tick: 0,
            };
            let jump = Cut {
                tick: 30,
                from_kind: ShotKind::Close,
                to_kind: ShotKind::Close,
                from_focus: 1,
                to_focus: 1,
                side: ShotSide::Left,
                crossed_line: false,
                reason: CutReason::Hit,
                beat_tick: 30,
            };
            let audit = audit_cuts(&[open, jump], MIN_SHOT_TICKS);
            assert!(!audit.is_clean(), "a jump under the hold floor flags");
            assert_eq!(audit.jump_cuts, 1, "same framing and focus is a jump");
            assert_eq!(audit.short_holds, 1, "30 ticks is under the 60 hold");
            let err = cut_log_audit_line(audit).expect_err("a flagged log is an error");
            let text = format!("{err:?}");
            assert!(text.contains("cut log audit: 2 flags"), "loud: {text}");
            assert!(
                text.contains("jump 1 line 0 hold 1 slew 0 stale 0"),
                "names each flag: {text}"
            );
        }
    }

    #[cfg(test)]
    mod twin_fighter_tests {
        use super::*;

        /// RL26: the twin frame carries the staged fighter bodies into the
        /// pixels. `frame_list` pushes one draw per resident body (the RL26
        /// bind), and the same frame rendered with those draws removed differs,
        /// so an 8-fighter twin capture can no longer be the old zero-pixel
        /// cube frame. Before the bind `frame_list` pushed no body at all, so
        /// the removal changed nothing and this test failed.
        ///
        /// Runs the offscreen twin on the reference GPU; a machine with no
        /// adapter skips loudly, the same skip the twin smokes take.
        #[test]
        fn twin_fighters_reach_the_frame_pixels() {
            let probe = Presenter::init_offscreen().map_err(|error| match error {
                engine_render::present::PresentError::NoAdapter => {
                    engine_render::test_gpu::NoAdapter("no wgpu adapter on this machine".into())
                }
                other => panic!("offscreen presenter: {other}"),
            });
            let Some(mut presenter) = engine_render::test_gpu::require_or_skip(
                "twin_fighters_reach_the_frame_pixels",
                probe,
            ) else {
                return;
            };
            presenter
                .resize(SMOKE_FRAME_SIZE.0, SMOKE_FRAME_SIZE.1)
                .expect("twin resizes");
            let key = MeshKey::new(1, [0xA1; 32]);
            let _ = presenter.upload_mesh(key, &engine_render::v1::cube());
            let _ = stage_dressing(&mut presenter);
            let max = engine_render::twin_fighters::TWIN_FIGHTER_MAX;
            let staged = engine_render::twin_fighters::stage_twin_fighters(&mut presenter, max)
                .expect("fighter bodies stage");
            assert_eq!(staged, 8, "one uploaded body per configured fighter slot");
            let full = frame_list(&presenter, SMOKE_FRAME_SIZE.0, SMOKE_FRAME_SIZE.1, None, 0)
                .expect("twin frame list builds");
            let bodies: Vec<MeshKey> = (0..max)
                .map(engine_render::twin_fighters::twin_fighter_key)
                .collect();
            let pushed = full
                .items
                .iter()
                .filter(|item| bodies.contains(&item.mesh))
                .count();
            assert_eq!(pushed, 8, "`frame_list` pushes one draw per staged body");
            let mut bare = full.clone();
            bare.items.retain(|item| !bodies.contains(&item.mesh));
            let with = presenter.render_offscreen(&full).expect("twin renders");
            let without = presenter.render_offscreen(&bare).expect("twin renders");
            assert!(with.manifest.is_clean(), "clean validation with fighters");
            let (Some(with_png), Some(without_png)) = (with.png, without.png) else {
                panic!("twin frame carried no bytes");
            };
            assert_ne!(
                with_png, without_png,
                "the staged fighter bodies change the twin pixels"
            );
        }
    }

    #[cfg(test)]
    mod headless_tests {
        use super::*;

        use crate::headless::{headless_smoke_line, split_plain_headless, HeadlessWhy};

        fn string_args(args: &[&str]) -> Vec<String> {
            args.iter().map(|arg| arg.to_string()).collect()
        }

        fn test_config() -> GameConfig {
            GameConfig {
                seed: 2040,
                ticks: 24,
                fighters: 2,
                half_extent_m: 20.0,
            }
        }

        #[test]
        fn headless_flag_splits_off_and_the_strict_parser_still_refuses_it() {
            let (on, rest) =
                split_plain_headless(&string_args(&["arena", "--headless", "--frames", "3"]));
            assert!(on, "the flag asks for the no-window route");
            assert_eq!(rest, string_args(&["arena", "--frames", "3"]));
            // A net seat keeps the flag: there it asks the net session to run
            // windowless, so the net split owns it.
            let (net, same) =
                split_plain_headless(&string_args(&["arena", "--headless", "--host", "1"]));
            assert!(!net, "a net seat keeps its own --headless meaning");
            assert_eq!(same, string_args(&["arena", "--headless", "--host", "1"]));
            let (off, same) = split_plain_headless(&string_args(&["arena", "--exit"]));
            assert!(!off, "the route ships off");
            assert_eq!(same, string_args(&["arena", "--exit"]));
            // The strict parser still refuses the flag if it were not split.
            assert!(WinArgs::parse(&string_args(&["arena", "--headless"])).is_err());
        }

        #[test]
        fn headless_smoke_line_carries_the_last_checkpoint_hash() {
            let mut report = engine_game::windowed::WinReport {
                ticks_run: 600,
                frames_presented: 0,
                last_checkpoint: Some(engine_arena::Checkpoint {
                    tick: 600,
                    hash: 0x0123_4567_89ab_cdef,
                }),
                hud_cells: 5,
                hud_quads: 3,
                mixed_frames: 0,
                mean_send: 0.0,
                audio_notice: "audio muted".to_string(),
                presenter_notice: "presenter skipped: no wgpu adapter on this machine".to_string(),
                camera_frames: 600,
            };
            let line = headless_smoke_line(&report, HeadlessWhy::Forced, "none requested");
            assert!(
                line.starts_with("arena headless smoke: ticks=600 frames=0 hash=0123456789abcdef "),
                "{line}"
            );
            assert!(line.contains("headless=forced"), "{line}");
            assert!(
                line.contains("presenter=\"presenter skipped: no wgpu adapter on this machine\""),
                "{line}"
            );
            // A run that never ticked names no hash, so a checker cannot read one.
            report.ticks_run = 0;
            report.last_checkpoint = None;
            let empty = headless_smoke_line(&report, HeadlessWhy::NoDisplay, "no display (test)");
            assert!(empty.contains(" ticks=0 frames=0 hash=- "), "{empty}");
            assert!(empty.contains("headless=no-display"), "{empty}");
        }

        #[test]
        fn forced_headless_plays_the_session_with_no_window() {
            // The shipped check uses this route: the fixed ticks run with no
            // window and no presenter, and the run reports green on its own.
            let args = WinArgs::parse(&string_args(&[
                "arena", "--frames", "6", "--exit", "--mute",
            ]))
            .expect("headless args parse");
            assert_eq!(forced_headless(test_config(), &args, &None), 0);
        }
    }
}
