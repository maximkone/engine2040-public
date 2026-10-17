//! The arena frame in the browser (WEB5): the render crate's own presenter, driven from
//! `requestAnimationFrame` on a WebGPU device the page opened asynchronously.
//!
//! Nothing here blocks. The instance, surface, adapter and device come from `await`s (surface
//! before adapter, a fresh instance and adapter after a lost device because Chrome rejects a
//! second device on one adapter); the presenter is then built from those parts without a wait,
//! its error scopes are answered late (see `Presenter::deferred_validation_errors`), and its
//! frame path has no map or poll wait. The scene is the render crate's procedural arena: a
//! lit ring with eight GPU-skinned fighters, the stand-in until the cooked kit can be fetched.
//! Compiled for `wasm32` only; the decisions it prints live in [`crate::arena_view`].

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use engine_render::pipeline_warmup;
use engine_render::present::{last_live_wall_split, PresentError, Presenter, MAX_FRAME_DIM};
use engine_render::procedural_arena::ProceduralArena;
use engine_render::quality::{self, Quality, WEB_CUT_ORDER};
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::anim::FrameStats;
use crate::arena_view::{frame_size, CaptureFacts, Counters, CAPTURE_AFTER_FRAMES};
use crate::limits::{feature_names, limit_rows};
use crate::recovery::LossBudget;
use crate::report::{AdapterFacts, ArenaFacts, Failure, Ready};
use crate::shell::{
    browser_clock, document, fail, log_error, navigator_gpu, now_ms, pause, raw_adapter_info,
    recover_with, request_frame, run_golden, set_body_attr, set_state, set_text, user_agent,
    wasm_pages, FrameSlot, DESTROY_REQUESTED,
};
use crate::tier_view::{
    cut_line, scale_after_cut, us_to_ms, whole_ms, Cut, Ladder, SplitWindow, TierFacts,
};
use crate::COMMIT;

/// The frame budget the ladder holds the page to: 30 frames per second.
const BUDGET_MS: f64 = 1000.0 / 30.0;
/// The render scale never goes under this many percent (below it the picture is a smear).
const SCALE_FLOOR_PCT: u32 = 60;
/// Frames drawn behind the loading screen so every pipeline has been used once before the page
/// says it is ready.
const WARM_FRAMES: u32 = 3;
/// Longest the loader waits for the GPU to finish the warm-up frames, milliseconds.
const WARM_WAIT_MS: f64 = 60_000.0;

thread_local! {
    /// The losses so far; a storm of them ends in the fatal page, not in endless restarts.
    static LOSSES: RefCell<LossBudget> = const { RefCell::new(LossBudget::new()) };
    /// Set by [`request_arena_capture`]: the next frame asks the GPU for a canvas copy.
    static CAPTURE_WANTED: Cell<bool> = const { Cell::new(false) };
}

/// Errors the device raised outside any scope, and the first one's text.
#[derive(Default)]
struct Uncaptured {
    count: AtomicU64,
    first: Mutex<Option<String>>,
}

/// One frame loop's state: the presenter and everything the page reports about it.
struct ArenaLoop {
    presenter: Presenter,
    arena: ProceduralArena,
    canvas: HtmlCanvasElement,
    lost: Arc<AtomicBool>,
    uncaptured: Arc<Uncaptured>,
    max_dim: u32,
    stats: FrameStats,
    started_ms: f64,
    counters: Counters,
    first_error: Option<String>,
    surface_format: String,
    last_cpu_ms: f64,
    last_draws: u32,
    capture_asked: bool,
    tier: Quality,
    /// Render scale in percent: the tier's, lowered by the ladder.
    scale_pct: u32,
    start_scale_pct: u32,
    window: SplitWindow,
    /// `None` when the ladder is off (`?ladder=off`, or a software adapter that no cut helps).
    ladder: Option<Ladder>,
    sim_ms: f64,
    late_reported: usize,
}

impl ArenaLoop {
    /// Keep the canvas backing store at the CSS box times the device pixel ratio, within the
    /// presenter's frame bound and the page's pixel budget.
    fn fit_canvas(&mut self) {
        let dpr = web_sys::window().map_or(1.0, |w| w.device_pixel_ratio())
            * f64::from(self.scale_pct)
            / 100.0;
        let (w, h) = frame_size(
            f64::from(self.canvas.client_width()),
            f64::from(self.canvas.client_height()),
            dpr,
            self.max_dim,
        );
        if (w, h) != self.presenter.size() {
            self.canvas.set_width(w);
            self.canvas.set_height(h);
            if let Err(e) = self.presenter.resize(w, h) {
                self.note_error(&format!("resize {w}x{h}: {e}"));
            }
        }
    }

    fn note_error(&mut self, text: &str) {
        self.counters.frame_errors += 1;
        log_error(&format!("arena: {text}"));
        self.first_error.get_or_insert_with(|| text.to_string());
    }

    /// Draw one frame that began at `frame_ms` on the animation clock.
    fn draw(&mut self, frame_ms: f64) {
        self.stats.tick(frame_ms);
        self.fit_canvas();
        self.drive_capture();
        let t_s = ((frame_ms - self.started_ms) / 1000.0) as f32;
        let cpu_started = now_ms();
        let (w, h) = self.presenter.size();
        if let Err(e) = self.arena.pose(&mut self.presenter, t_s) {
            self.note_error(&format!("pose: {e}"));
        }
        let listed = self.arena.list_at(w, h, t_s);
        self.sim_ms = now_ms() - cpu_started;
        match listed {
            Ok(list) => match self.presenter.render_surface(&list) {
                Ok(receipt) => {
                    self.last_draws = receipt.draws;
                    self.counters.validation_errors += u64::from(receipt.total_validation_errors());
                    if self.counters.frames.is_multiple_of(15) {
                        set_text("passes", &receipt.per_pass_summary());
                    }
                }
                // The presenter reconfigured the surface or the browser has no image for us
                // this frame: skip it, the next animation frame tries again.
                Err(
                    PresentError::SurfaceOutdated { .. }
                    | PresentError::SurfaceLost { .. }
                    | PresentError::SurfaceTimeout
                    | PresentError::SurfaceOccluded,
                ) => {}
                Err(e) => self.note_error(&format!("frame: {e}")),
            },
            Err(e) => self.note_error(&format!("list: {e}")),
        }
        self.last_cpu_ms = now_ms() - cpu_started;
        self.counters.frames += 1;
        self.counters.deferred_validation_errors = self.presenter.deferred_validation_errors();
        self.counters.uncaptured_errors = self.uncaptured.count.load(Ordering::Relaxed);
        if self.first_error.is_none() {
            if let Some(first) = self.presenter.deferred_validation_first() {
                self.first_error = Some(first.to_string());
            } else if let Ok(first) = self.uncaptured.first.lock() {
                self.first_error.clone_from(&first);
            }
        }
        set_text(
            "hud",
            &format!("{} | cpu {:.1} ms", self.stats.line(), self.last_cpu_ms),
        );
        if self.counters.frames % 15 == 1 {
            self.publish_receipt();
        }
        self.tick_split(frame_ms);
    }

    /// Feed the frame to the one-second window; once a second print the split and let the
    /// ladder decide, and report any pipeline built after the loader (`late_compile`).
    fn tick_split(&mut self, now: f64) {
        let wall = last_live_wall_split();
        let render_ms = us_to_ms(wall.stage_us.saturating_add(wall.submit_us));
        let present_ms = us_to_ms(wall.present_call_us);
        self.window.add(
            now,
            self.stats.last_ms(),
            self.sim_ms,
            render_ms,
            present_ms,
        );
        let Some(split) = self.window.take_if_due(now) else {
            return;
        };
        let line = split.line(self.tier.name());
        set_text("split", &line);
        web_sys::console::log_1(&JsValue::from_str(&line));
        let ledger = pipeline_warmup::snapshot();
        if let Some(late) = pipeline_warmup::late_line(&ledger) {
            if ledger.late().len() != self.late_reported {
                self.late_reported = ledger.late().len();
                self.counters.late_pipelines =
                    u64::try_from(self.late_reported).unwrap_or(u64::MAX);
                set_text(
                    "pipelines",
                    &format!("{} | {late}", pipeline_warmup::ready_line(&ledger, 0.0)),
                );
                set_body_attr("data-late-pipelines", &self.late_reported.to_string());
                log_error(&late);
            }
        }
        // A cut resizes the canvas; do not change the size under a capture that is on its way.
        let capture_pending = self.capture_asked && self.presenter.capture_busy();
        // A hidden tab is not a slow picture either.
        let hidden = document().is_some_and(|d| d.hidden());
        if capture_pending || hidden || split.frames == 0 {
            return;
        }
        if let Some(cut) = self.ladder.as_mut().and_then(|l| l.observe(split.frame_ms)) {
            self.apply_cut(cut);
        }
    }

    /// Take one cut of the ladder. Only the knobs the page really owns change the picture
    /// (render scale, bloom); the others are printed with `applied=no` so the order is visible.
    fn apply_cut(&mut self, cut: Cut) {
        let name = WEB_CUT_ORDER.get(cut.step).copied().unwrap_or("unknown");
        let applied = match name {
            "post_and_particles" => {
                self.presenter.set_bloom_enabled(false);
                true
            }
            "render_scale" => {
                self.scale_pct = scale_after_cut(self.start_scale_pct, cut.repeat, SCALE_FLOOR_PCT);
                true
            }
            _ => false,
        };
        let line = cut_line(self.tier.name(), name, cut.repeat, applied, self.scale_pct);
        set_text("tier", &line);
        web_sys::console::log_1(&JsValue::from_str(&line));
    }

    fn publish_receipt(&self) {
        let (waiting, dropped) = self.presenter.deferred_validation_waiting();
        let line = crate::arena_view::receipt_line(
            self.tier.name(),
            &self.counters,
            self.stats.last_ms(),
            self.last_cpu_ms,
            self.last_draws,
            self.first_error.as_deref(),
        );
        set_text("receipt", &line);
        set_body_attr(
            "data-validation-errors",
            &self.counters.validation_total().to_string(),
        );
        set_body_attr("data-scopes-waiting", &format!("{waiting}/{dropped}"));
    }

    /// Ask for the GPU copy of the canvas once the first frames have settled (or when the page
    /// asks), and publish it once its map has finished.
    fn drive_capture(&mut self) {
        let wanted = CAPTURE_WANTED.with(Cell::take)
            || (!self.capture_asked && self.counters.frames == CAPTURE_AFTER_FRAMES);
        if wanted {
            self.capture_asked = true;
            if let Err(e) = self.presenter.request_capture() {
                self.note_error(&format!("capture: {e}"));
            }
        }
        match self.presenter.take_capture() {
            Ok(Some(frame)) => {
                let stats = frame.stats();
                let facts = CaptureFacts {
                    width: frame.width,
                    height: frame.height,
                    hash: frame.hash(),
                    distinct: u64::try_from(stats.distinct).unwrap_or(0),
                    dominant_share: stats.dominant_share,
                    mean_luma: stats.mean_luma,
                    blank: stats.is_blank(),
                };
                let line = facts.line(self.counters.frames, &self.surface_format);
                publish_capture(&frame.rgba, &line);
                set_text("capture", &line);
                set_body_attr("data-capture", if facts.blank { "blank" } else { "ready" });
            }
            Ok(None) => {}
            Err(e) => {
                self.note_error(&format!("capture read: {e}"));
                set_body_attr("data-capture", "error");
            }
        }
    }
}

/// Hand the captured pixels to the page: `window.arenaCapture = { line, rgba }` where `rgba` is
/// the tight RGBA bytes. The headless check reads it and writes the PNG.
fn publish_capture(rgba: &[u8], line: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let object = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&object, &"line".into(), &JsValue::from_str(line));
    let _ = js_sys::Reflect::set(&object, &"rgba".into(), &js_sys::Uint8Array::from(rgba));
    let _ = js_sys::Reflect::set(&window, &"arenaCapture".into(), &object);
}

/// Run `frame` on every animation frame until its device is lost. A lost device ends the loop
/// and starts [`recover`] on a fresh instance and adapter.
fn run_loop(mut frame: ArenaLoop, canvas_id: String, generation: u32) {
    let slot: Rc<FrameSlot> = Rc::new(RefCell::new(None));
    let again = Rc::clone(&slot);
    *slot.borrow_mut() = Some(Closure::new(move |now: f64| {
        let destroyed = DESTROY_REQUESTED.with(Cell::take);
        if destroyed {
            frame.presenter.device().destroy();
        }
        if frame.lost.load(Ordering::SeqCst) {
            // Dropping the closure ends this loop; wasm-bindgen frees it after this call returns.
            drop(again.borrow_mut().take());
            if LOSSES.with(|losses| losses.borrow_mut().record(now)) {
                wasm_bindgen_futures::spawn_local(recover_with(
                    canvas_id.clone(),
                    generation + 1,
                    |id, g| async move { start(&id, g).await },
                ));
            } else {
                fail(
                    Failure::NoDevice,
                    "the device was lost again and again; the browser cannot keep one.",
                );
            }
            return;
        }
        if !destroyed {
            frame.draw(now);
        }
        request_frame(&again);
    }));
    request_frame(&slot);
}

fn present_failure(e: &PresentError) -> (Failure, String) {
    match e {
        PresentError::NoAdapter => (Failure::NoAdapter, e.to_string()),
        PresentError::NoSurface | PresentError::InvalidSize { .. } => {
            (Failure::NoSurface, e.to_string())
        }
        _ => (Failure::NoDevice, e.to_string()),
    }
}

/// What the browser handed over before the presenter exists: the canvas, a fresh instance, the
/// surface, the adapter that can present to it and the device with its error and loss hooks.
struct Gpu {
    raw: JsValue,
    canvas: HtmlCanvasElement,
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    lost: Arc<AtomicBool>,
    uncaptured: Arc<Uncaptured>,
}

/// Ask the browser for everything the presenter needs, with `await`s and no blocking call.
async fn open_gpu(canvas_id: &str) -> Result<Gpu, (Failure, String)> {
    let raw = navigator_gpu().ok_or_else(|| {
        (
            Failure::NoWebGpu,
            "navigator.gpu is undefined: an older browser, or a page that is neither https nor localhost."
                .to_string(),
        )
    })?;
    let canvas = document()
        .and_then(|d| d.get_element_by_id(canvas_id))
        .and_then(|e| e.dyn_into::<HtmlCanvasElement>().ok())
        .ok_or_else(|| {
            (
                Failure::NoCanvas,
                format!("expected <canvas id=\"{canvas_id}\">"),
            )
        })?;

    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = wgpu::Backends::BROWSER_WEBGPU;
    let instance = wgpu::Instance::new(desc);
    // The surface first, then the adapter that can present to it.
    let surface = instance
        .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
        .map_err(|e| (Failure::NoSurface, e.to_string()))?;
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
            apply_limit_buckets: false,
        })
        .await
        .map_err(|e| (Failure::NoAdapter, e.to_string()))?;
    let (device, queue) = adapter
        .request_device(&Presenter::device_descriptor(&adapter))
        .await
        .map_err(|e| (Failure::NoDevice, e.to_string()))?;

    let uncaptured = Arc::new(Uncaptured::default());
    let sink = Arc::clone(&uncaptured);
    device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
        let text = format!("{e}");
        log_error(&format!("wgpu: {text}"));
        sink.count.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut first) = sink.first.lock() {
            first.get_or_insert(text);
        }
    }));
    let lost = Arc::new(AtomicBool::new(false));
    let lost_by_callback = Arc::clone(&lost);
    device.set_device_lost_callback(move |reason, message| {
        log_error(&format!("wgpu device lost ({reason:?}): {message}"));
        lost_by_callback.store(true, Ordering::SeqCst);
    });

    Ok(Gpu {
        raw,
        canvas,
        instance,
        surface,
        adapter,
        device,
        queue,
        lost,
        uncaptured,
    })
}

/// Everything up to the first frame. `Err` is a failure the page can show, not a crash.
async fn start(canvas_id: &str, generation: u32) -> Result<String, (Failure, String)> {
    let Gpu {
        raw: gpu,
        canvas,
        instance,
        surface,
        adapter,
        device,
        queue,
        lost,
        uncaptured,
    } = open_gpu(canvas_id).await?;
    let max_dim = device.limits().max_texture_dimension_2d.min(MAX_FRAME_DIM);
    let features = feature_names(device.features());
    let limits = limit_rows(&device.limits());
    let TierPick {
        search,
        tier,
        why: tier_why,
        scale_pct,
        size: (w, h),
    } = pick_tier(&canvas, &device, max_dim);

    let [vendor, architecture, web_device, description] = raw_adapter_info(&gpu).await;
    let info = adapter.get_info();
    let software = is_software(&info, &architecture, &description);

    // Building the presenter bakes the sky and the image-based light on this thread: say so
    // first and let the browser paint the message.
    set_text("hud", "loading the arena: baking sky and light...");
    pause(30).await;
    let built_at = now_ms();
    // WEB13: a new presenter, a new ledger. Every pipeline it builds from here on is counted.
    pipeline_warmup::reset();
    let work_queue = queue.clone();
    let presenter = Presenter::init_from_device(&instance, &adapter, device, queue, surface, w, h)
        .map_err(|e| present_failure(&e))?;
    let arena = ProceduralArena::build().map_err(|e| (Failure::NoDevice, e.to_string()))?;
    let mut frame = ArenaLoop {
        presenter,
        arena,
        canvas,
        lost,
        uncaptured,
        max_dim,
        stats: FrameStats::default(),
        started_ms: now_ms(),
        counters: Counters::default(),
        first_error: None,
        surface_format: String::new(),
        last_cpu_ms: 0.0,
        last_draws: 0,
        capture_asked: false,
        tier,
        scale_pct,
        start_scale_pct: scale_pct,
        window: SplitWindow::default(),
        ladder: (!search.contains("ladder=off") && !software)
            .then(|| Ladder::new(WEB_CUT_ORDER.len(), 2, BUDGET_MS)),
        sim_ms: 0.0,
        late_reported: 0,
    };
    frame
        .arena
        .stage(&mut frame.presenter)
        .map_err(|e| (Failure::NoDevice, e.to_string()))?;
    frame.surface_format = frame
        .presenter
        .surface_format()
        .map(|f| format!("{f:?}").to_ascii_lowercase())
        .unwrap_or_default();

    let ready = Ready {
        commit: COMMIT.to_string(),
        adapter: adapter_facts(&info, [vendor, architecture, web_device, description]),
        features,
        limits,
        wasm_pages: wasm_pages(),
        generation,
        canvas: (w, h),
        surface_format: frame.surface_format.clone(),
        user_agent: user_agent(),
    };
    let (ledger, warm_ms) = warm_behind_loader(&mut frame, &work_queue, built_at).await;
    let init_ms = now_ms() - built_at;
    let line = publish_ready(
        &frame,
        &ready,
        (tier, tier_why),
        (&ledger, warm_ms),
        init_ms,
    );
    set_body_attr("data-generation", &generation.to_string());
    set_state("running");
    web_sys::console::log_1(&JsValue::from_str(&line));
    run_loop(frame, canvas_id.to_string(), generation);
    Ok(line)
}

/// The tier and the canvas size it implies.
struct TierPick {
    /// The page's query string (`?tier=web`).
    search: String,
    tier: Quality,
    /// `query`, `limits` or `default`.
    why: &'static str,
    scale_pct: u32,
    size: (u32, u32),
}

/// WEB8: the tier comes from the adapter's limits and a `?tier=` in the address; the canvas
/// backing store is set to the tier's render scale of its CSS box.
fn pick_tier(canvas: &HtmlCanvasElement, device: &wgpu::Device, max_dim: u32) -> TierPick {
    let search = page_search();
    let (tier, why) = quality::choose_web_tier(
        quality::tier_from_query(&search),
        device.limits().max_texture_dimension_2d,
    );
    let scale_pct = u32::from(quality::knobs(tier).render_scale_pct);
    let dpr =
        web_sys::window().map_or(1.0, |w| w.device_pixel_ratio()) * f64::from(scale_pct) / 100.0;
    let size = frame_size(
        f64::from(canvas.client_width()),
        f64::from(canvas.client_height()),
        dpr,
        max_dim,
    );
    canvas.set_width(size.0);
    canvas.set_height(size.1);
    TierPick {
        search,
        tier,
        why,
        scale_pct,
        size,
    }
}

/// A software adapter (SwiftShader) is slow by nature; no cut of the ladder would help it.
fn is_software(info: &wgpu::AdapterInfo, architecture: &str, description: &str) -> bool {
    info.device_type == wgpu::DeviceType::Cpu
        || [architecture, description, info.name.as_str()]
            .iter()
            .any(|text| text.to_ascii_lowercase().contains("swiftshader"))
}

/// What the ready line says about the adapter: wgpu's view plus the browser's raw strings
/// (`[vendor, architecture, device, description]`).
fn adapter_facts(info: &wgpu::AdapterInfo, raw: [String; 4]) -> AdapterFacts {
    let [web_vendor, web_architecture, web_device, web_description] = raw;
    AdapterFacts {
        name: info.name.clone(),
        backend: format!("{:?}", info.backend),
        device_type: format!("{:?}", info.device_type),
        web_vendor,
        web_architecture,
        web_device,
        web_description,
    }
}

/// WEB13: behind the loading screen, build what the presenter did not at creation (the HP-bar
/// overlay), draw a few real frames so every pipeline has been used once and wait for the GPU
/// to finish them: the browser compiles on its own threads meanwhile. wgpu 30 has no async
/// pipeline call, so this is the closest non-blocking route. The ledger is sealed here: a build
/// after it is a `late_compile`. The frames are real, so `running` still means a frame reached
/// the canvas. Returns the builds and the milliseconds since `built_at`.
async fn warm_behind_loader(
    frame: &mut ArenaLoop,
    queue: &wgpu::Queue,
    built_at: f64,
) -> (pipeline_warmup::Ledger, f64) {
    frame.presenter.warm_pipelines();
    for _ in 0..WARM_FRAMES {
        frame.draw(now_ms());
        pause(0).await;
    }
    wait_for_gpu(queue).await;
    let ledger = pipeline_warmup::snapshot();
    pipeline_warmup::seal();
    // The scope answers of the warm frames are in by now; count them before the ready line.
    frame.counters.deferred_validation_errors = frame.presenter.deferred_validation_errors();
    frame.counters.uncaptured_errors = frame.uncaptured.count.load(Ordering::Relaxed);
    (ledger, now_ms() - built_at)
}

/// The tier's knobs as the page prints them (see [`TierFacts`]).
fn tier_facts(tier: Quality, why: &str) -> TierFacts {
    let knobs = quality::knobs(tier);
    TierFacts {
        name: tier.name().to_string(),
        why: why.to_string(),
        scale_pct: u32::from(knobs.render_scale_pct),
        shadow_px: u32::from(knobs.shadow_map_px),
        cascades: u32::from(knobs.shadow_cascades),
        gtao: match knobs.gtao {
            quality::Gtao::Off => "off",
            quality::Gtao::Half => "half",
            quality::Gtao::Full => "full",
        }
        .to_string(),
        taa: knobs.taa,
        particles: knobs.particle_cap,
        decals: knobs.decal_cap,
        physfx_bodies: u32::from(knobs.physfx_bodies),
        physfx_ragdolls: u32::from(knobs.physfx_ragdolls),
        physfx_contacts: u32::from(knobs.physfx_contacts),
        texture_px: u32::from(knobs.texture_max_px),
        wired: "scale,bloom".to_string(),
    }
}

/// Print the tier, the pipeline warm-up and the ready line to the page and the console; returns
/// the ready line (JSON with the tier, the pipelines and the validation count).
fn publish_ready(
    frame: &ArenaLoop,
    ready: &Ready,
    (tier, tier_why): (Quality, &str),
    (ledger, warm_ms): (&pipeline_warmup::Ledger, f64),
    init_ms: f64,
) -> String {
    let tier_line = tier_facts(tier, tier_why).line();
    let pipelines_line = pipeline_warmup::ready_line(ledger, warm_ms);
    let facts = ArenaFacts {
        tier: tier.name().to_string(),
        tier_why: tier_why.to_string(),
        pipelines: u64::try_from(ledger.warm()).unwrap_or(u64::MAX),
        pipelines_ms: whole_ms(warm_ms),
        late_pipelines: 0,
        validation: frame.counters.validation_total(),
    };
    let line = ready.line_with(&facts);
    set_text("report", &line);
    set_text("tier", &tier_line);
    set_text("pipelines", &pipelines_line);
    set_body_attr("data-tier", tier.name());
    set_body_attr("data-pipelines", &ledger.warm().to_string());
    web_sys::console::log_1(&JsValue::from_str(&tier_line));
    web_sys::console::log_1(&JsValue::from_str(&pipelines_line));
    set_text("init", &format!("arena ready in {init_ms:.0} ms"));
    line
}

/// The page's query string (`?tier=web`), empty when it cannot be read.
fn page_search() -> String {
    web_sys::window()
        .and_then(|w| js_sys::Reflect::get(&w, &"location".into()).ok())
        .and_then(|loc| js_sys::Reflect::get(&loc, &"search".into()).ok())
        .and_then(|s| s.as_string())
        .unwrap_or_default()
}

/// Wait until the GPU has finished everything submitted so far (or [`WARM_WAIT_MS`] passed).
/// The callback and a short timer poll: no blocking call.
async fn wait_for_gpu(queue: &wgpu::Queue) {
    let done = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&done);
    queue.on_submitted_work_done(move || flag.store(true, Ordering::SeqCst));
    let started = now_ms();
    while !done.load(Ordering::SeqCst) && now_ms() - started < WARM_WAIT_MS {
        pause(10).await;
    }
    // Error-scope answers ride the same queue; give them a moment to land.
    pause(30).await;
}

/// Start the arena on the canvas with id `canvas_id`. Resolves with the JSON line it printed
/// (`"ok":true` once the first frame is on the canvas, `"ok":false` with a reason when the
/// browser has no WebGPU or refuses a device). It does not throw for those cases. After that
/// line, with or without a GPU, it runs the golden check and prints its verdict line.
#[wasm_bindgen]
pub async fn run_arena(canvas_id: String) -> String {
    std::panic::set_hook(Box::new(|info| log_error(&format!("panic: {info}"))));
    set_state("starting");
    // The headless check keys on this: the arena page has a GPU copy of its canvas to read.
    set_body_attr("data-mode", "arena");
    // Input and audio need the DOM only, so they start before (and without) the GPU, as in the shell.
    crate::browser::install(&canvas_id);
    // First, before anything can read a clock: the page's performance.now() becomes the engine's.
    engine_clock::install(browser_clock);
    let line = match start(&canvas_id, 1).await {
        Ok(line) => line,
        Err((why, detail)) => fail(why, &detail),
    };
    run_golden();
    line
}

/// Ask for a GPU copy of the next canvas frame (the headless check calls this to re-take it).
#[wasm_bindgen]
pub fn request_arena_capture() {
    CAPTURE_WANTED.with(|c| c.set(true));
}
