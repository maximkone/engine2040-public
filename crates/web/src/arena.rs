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

use engine_render::present::{PresentError, Presenter, MAX_FRAME_DIM};
use engine_render::procedural_arena::ProceduralArena;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::anim::FrameStats;
use crate::arena_view::{frame_size, CaptureFacts, Counters, CAPTURE_AFTER_FRAMES};
use crate::limits::{feature_names, limit_rows};
use crate::recovery::LossBudget;
use crate::report::{AdapterFacts, Failure, Ready};
use crate::shell::{
    browser_clock, document, fail, log_error, navigator_gpu, now_ms, raw_adapter_info, recover_with,
    request_frame, run_golden, set_body_attr, set_state, set_text, user_agent, wasm_pages,
    FrameSlot, DESTROY_REQUESTED,
};
use crate::COMMIT;

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
}

impl ArenaLoop {
    /// Keep the canvas backing store at the CSS box times the device pixel ratio, within the
    /// presenter's frame bound and the page's pixel budget.
    fn fit_canvas(&mut self) {
        let dpr = web_sys::window().map_or(1.0, |w| w.device_pixel_ratio());
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
        match self.arena.draw_list(w, h, t_s) {
            Ok(list) => match self.presenter.render_surface(&list) {
                Ok(receipt) => {
                    self.last_draws = receipt.draws;
                    self.counters.validation_errors += u64::from(receipt.total_validation_errors());
                    if self.counters.frames % 15 == 0 {
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
                self.first_error = first.clone();
            }
        }
        set_text(
            "hud",
            &format!("{} | cpu {:.1} ms", self.stats.line(), self.last_cpu_ms),
        );
        if self.counters.frames % 15 == 1 {
            self.publish_receipt();
        }
    }

    fn publish_receipt(&self) {
        let (waiting, dropped) = self.presenter.deferred_validation_waiting();
        let line = crate::arena_view::receipt_line(
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
                    distinct: stats.distinct as u64,
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

/// Everything up to the first frame. `Err` is a failure the page can show, not a crash.
async fn start(canvas_id: &str, generation: u32) -> Result<String, (Failure, String)> {
    let gpu = navigator_gpu().ok_or_else(|| {
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

    let max_dim = device.limits().max_texture_dimension_2d.min(MAX_FRAME_DIM);
    let features = feature_names(device.features());
    let limits = limit_rows(&device.limits());
    let dpr = web_sys::window().map_or(1.0, |w| w.device_pixel_ratio());
    let (w, h) = frame_size(
        f64::from(canvas.client_width()),
        f64::from(canvas.client_height()),
        dpr,
        max_dim,
    );
    canvas.set_width(w);
    canvas.set_height(h);

    let [vendor, architecture, web_device, description] = raw_adapter_info(&gpu).await;
    let info = adapter.get_info();

    // Building the presenter bakes the sky and the image-based light on this thread: say so
    // first and let the browser paint the message.
    set_text("hud", "loading the arena: baking sky and light...");
    pause(30).await;
    let built_at = now_ms();
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
    };
    frame
        .arena
        .stage(&mut frame.presenter)
        .map_err(|e| (Failure::NoDevice, e.to_string()))?;
    let init_ms = now_ms() - built_at;
    frame.surface_format = frame
        .presenter
        .surface_format()
        .map(|f| format!("{f:?}").to_ascii_lowercase())
        .unwrap_or_default();

    let ready = Ready {
        commit: COMMIT.to_string(),
        adapter: AdapterFacts {
            name: info.name.clone(),
            backend: format!("{:?}", info.backend),
            device_type: format!("{:?}", info.device_type),
            web_vendor: vendor,
            web_architecture: architecture,
            web_device,
            web_description: description,
        },
        features,
        limits,
        wasm_pages: wasm_pages(),
        generation,
        canvas: (w, h),
        surface_format: frame.surface_format.clone(),
        user_agent: user_agent(),
    };
    // The first frame is drawn here, so `running` means a frame reached the canvas.
    frame.draw(now_ms());
    let line = ready.line();
    set_text("report", &line);
    set_text("init", &format!("arena ready in {init_ms:.0} ms"));
    set_body_attr("data-generation", &generation.to_string());
    set_state("running");
    web_sys::console::log_1(&JsValue::from_str(&line));
    run_loop(frame, canvas_id.to_string(), generation);
    Ok(line)
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
