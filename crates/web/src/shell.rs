//! Browser glue: the page's DOM, the WebGPU device through wgpu, and the animation loop.
//! Compiled for `wasm32` only. The decisions (colours, sizes, the JSON text, the limits) live
//! in the pure modules and are tested natively; this file only wires them to the browser.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::HtmlCanvasElement;

use crate::anim::{backing_size, clear_colour, duration_from_ms, FrameStats, Square};
use crate::golden::{self, Status};
use crate::limits::{feature_names, limit_rows, required_limits, wanted_features};
use crate::recovery::{
    reason_word, Cause, LossBudget, LossNote, LostBox, Verdict, ATTEMPTS, PAUSE_MS,
};
use crate::report::{failed_line, fatal_text, AdapterFacts, Failure, Ready};
use crate::{COMMIT, SHADER_WGSL};

thread_local! {
    /// The losses so far; a storm of them ends in the fatal page, not in endless restarts.
    pub(crate) static LOSSES: RefCell<LossBudget> = const { RefCell::new(LossBudget::new()) };
    /// Set by [`lose_device`] and [`destroy_device`]; the frame loop destroys its device on the
    /// next frame and remembers who asked.
    static DESTROY_ASKED: Cell<Option<Cause>> = const { Cell::new(None) };
    /// How many times the page was hidden or shown (`visibilitychange`), for the frames that span
    /// a gap (QT7).
    pub(crate) static VISIBILITY_CHANGES: Cell<u32> = const { Cell::new(0) };
}

/// Count every `visibilitychange` of the page. A frame loop that sees the count move between two
/// frames knows the tab was hidden in between: that frame spans a gap and is not a slow frame.
pub(crate) fn watch_visibility() {
    let Some(doc) = document() else {
        return;
    };
    let on_change = Closure::<dyn FnMut()>::new(|| {
        VISIBILITY_CHANGES.with(|count| count.set(count.get().wrapping_add(1)));
    });
    let _ = doc
        .add_event_listener_with_callback("visibilitychange", on_change.as_ref().unchecked_ref());
    // The listener lives as long as the page.
    on_change.forget();
}

pub(crate) fn document() -> Option<web_sys::Document> {
    web_sys::window()?.document()
}

pub(crate) fn set_text(id: &str, text: &str) {
    if let Some(el) = document().and_then(|d| d.get_element_by_id(id)) {
        el.set_text_content(Some(text));
    }
}

pub(crate) fn set_body_attr(name: &str, value: &str) {
    if let Some(body) = document().and_then(|d| d.body()) {
        let _ = body.set_attribute(name, value);
    }
}

/// `data-state` on `<body>` is how the page and the headless check see where the shell is:
/// `starting`, `running`, `recovering`, `stopped` (the page ended its device on purpose) or `fatal`.
pub(crate) fn set_state(state: &str) {
    set_body_attr("data-state", state);
}

pub(crate) fn user_agent() -> String {
    web_sys::window()
        .and_then(|w| w.navigator().user_agent().ok())
        .unwrap_or_default()
}

/// `window.crossOriginIsolated` (WEB14): true when the page got the COOP and COEP headers, which is
/// what makes `SharedArrayBuffer` legal. Read through reflection (like `navigator.gpu`): the page's
/// service worker supplies the headers on a host that cannot send them.
pub(crate) fn cross_origin_isolated() -> bool {
    web_sys::window()
        .and_then(|w| js_sys::Reflect::get(&w, &JsValue::from_str("crossOriginIsolated")).ok())
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// 64 KiB pages of the module's linear memory right now.
pub(crate) fn wasm_pages() -> u64 {
    core::arch::wasm32::memory_size::<0>() as u64
}

pub(crate) fn log_error(text: &str) {
    web_sys::console::error_1(&JsValue::from_str(text));
}

/// Show the reason on the page, mark it fatal, print the failure line. Returns the line.
pub(crate) fn fail(why: Failure, detail: &str) -> String {
    let line = failed_line(why, detail, COMMIT, wasm_pages(), &user_agent());
    set_text("fatal-reason", &fatal_text(why, detail));
    set_text("report", &line);
    set_body_attr("data-fallback", why.code());
    set_state("fatal");
    log_error(&line);
    line
}

/// `navigator.gpu`, or `None` when the browser has no WebGPU (or the page is not a secure
/// context). Read through reflection: the browser object is not needed beyond this check.
pub(crate) fn navigator_gpu() -> Option<JsValue> {
    let nav = web_sys::window()?.navigator();
    let gpu = js_sys::Reflect::get(&nav, &JsValue::from_str("gpu")).ok()?;
    (!gpu.is_undefined() && !gpu.is_null()).then_some(gpu)
}

/// The raw `GPUAdapterInfo` strings (vendor, architecture, device, description), which wgpu's
/// web backend does not carry past the description. Asks the browser for an adapter a second
/// time; empty strings if it will not say.
pub(crate) async fn raw_adapter_info(gpu: &JsValue) -> [String; 4] {
    async fn call(target: &JsValue, name: &str) -> Option<JsValue> {
        let f: js_sys::Function = js_sys::Reflect::get(target, &JsValue::from_str(name))
            .ok()?
            .dyn_into()
            .ok()?;
        let promise: js_sys::Promise = f.call0(target).ok()?.dyn_into().ok()?;
        JsFuture::from(promise).await.ok()
    }
    async fn read(gpu: &JsValue) -> Option<[String; 4]> {
        let adapter = call(gpu, "requestAdapter").await?;
        if !adapter.is_object() {
            return None;
        }
        // `adapter.info` is the current shape; older Chrome only has `requestAdapterInfo()`.
        let info = match js_sys::Reflect::get(&adapter, &JsValue::from_str("info")) {
            Ok(v) if v.is_object() => v,
            _ => call(&adapter, "requestAdapterInfo").await?,
        };
        let field = |k: &str| {
            js_sys::Reflect::get(&info, &JsValue::from_str(k))
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_default()
        };
        Some([
            field("vendor"),
            field("architecture"),
            field("device"),
            field("description"),
        ])
    }
    read(gpu).await.unwrap_or_default()
}

/// Resolves after `ms` milliseconds.
pub(crate) async fn pause(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = JsFuture::from(promise).await;
}

/// One frame's worth of GPU objects and the counters.
struct Frame {
    /// Filled by this device's own lost callback. One box per device: dropping an old device
    /// reports it lost again, and that must not end the loop of the device that replaced it.
    lost: Arc<LostBox>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    canvas: HtmlCanvasElement,
    max_dim: u32,
    stats: FrameStats,
}

impl Frame {
    /// Keep the canvas backing store at its CSS size times the device pixel ratio.
    fn fit_canvas(&mut self) {
        let dpr = web_sys::window().map_or(1.0, |w| w.device_pixel_ratio());
        let (w, h) = backing_size(
            f64::from(self.canvas.client_width()),
            f64::from(self.canvas.client_height()),
            dpr,
            self.max_dim,
        );
        if (w, h) != (self.config.width, self.config.height) {
            self.canvas.set_width(w);
            self.canvas.set_height(h);
            self.config.width = w;
            self.config.height = h;
            self.surface.configure(&self.device, &self.config);
        }
    }

    /// Draw one frame that began at `now_ms`; true when a frame was submitted.
    fn draw(&mut self, now_ms: f64) -> bool {
        self.stats.tick(now_ms);
        set_text("hud", &self.stats.line());
        self.fit_canvas();
        let target = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => t,
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => {
                self.surface.configure(&self.device, &self.config);
                t
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return false;
            }
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => return false,
        };
        let view = target
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let aspect = f64::from(self.config.width) / f64::from(self.config.height);
        self.queue
            .write_buffer(&self.uniform, 0, &Square::at(now_ms, aspect).to_bytes());
        let [r, g, b] = clear_colour(now_ms);
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("engine-web-frame"),
            });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("engine-web-clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..6, 0..1);
        }
        self.queue.submit(Some(enc.finish()));
        self.queue.present(target);
        true
    }
}

/// The animation-frame callback, shared with itself so it can ask for the frame after it.
pub(crate) type FrameSlot = RefCell<Option<Closure<dyn FnMut(f64)>>>;

/// Ask the browser for its next animation frame, calling `slot`'s closure.
pub(crate) fn request_frame(slot: &FrameSlot) {
    if let (Some(window), Some(callback)) = (web_sys::window(), slot.borrow().as_ref()) {
        let _ = window.request_animation_frame(callback.as_ref().unchecked_ref());
    }
}

/// What a frame loop does after it looked at its device (see [`check_device`]).
pub(crate) enum Check {
    /// The device is fine: draw.
    Draw,
    /// The page just destroyed the device; the browser reports it lost a moment later.
    Wait,
    /// The device was lost and the page starts over: end this loop and call the recovery.
    Recover,
    /// The loop is over for good (the fatal page or the stopped page is already shown).
    Over,
}

/// The frame loop's look at its device before each frame (QW8). It carries out a destroy the page
/// asked for and remembers who asked (`ended_by`). When the browser reports the device lost, the
/// loss budget judges it: a forced or real loss starts over, a storm gives up with the browser's
/// reason on the fatal page, and a requested destroy stops without a retry.
pub(crate) fn check_device(
    device: &wgpu::Device,
    lost: &LostBox,
    ended_by: &mut Option<Cause>,
    now: f64,
) -> Check {
    if let Some(cause) = DESTROY_ASKED.with(Cell::take) {
        device.destroy();
        *ended_by = Some(cause);
        return Check::Wait;
    }
    let Some((reason, message)) = lost.get() else {
        return Check::Draw;
    };
    let note = LossNote {
        reason,
        message,
        cause: ended_by.unwrap_or(Cause::Browser),
    };
    match LOSSES.with(|losses| losses.borrow_mut().judge(now, note.clone())) {
        Verdict::Retry => Check::Recover,
        Verdict::GiveUp => {
            let detail = LOSSES.with(|losses| losses.borrow().give_up_detail());
            fail(Failure::NoDevice, &detail);
            Check::Over
        }
        Verdict::Stop => {
            set_state("stopped");
            set_text(
                "hud",
                &format!(
                    "stopped: the page destroyed its graphics device (reason={}). Reload to play again.",
                    note.reason
                ),
            );
            Check::Over
        }
    }
}

/// Run `frame` on every animation frame until its device is lost. A lost device ends the
/// loop and starts [`recover`]: Chrome refuses a second device on the same adapter, so the
/// next one comes from a fresh instance and adapter.
fn run_loop(mut frame: Frame, canvas_id: String, generation: u32) {
    let slot: Rc<FrameSlot> = Rc::new(RefCell::new(None));
    let again = Rc::clone(&slot);
    let mut ended_by = None;
    *slot.borrow_mut() = Some(Closure::new(move |now: f64| {
        match check_device(&frame.device, &frame.lost, &mut ended_by, now) {
            Check::Draw => {
                frame.draw(now);
            }
            Check::Wait => {}
            Check::Recover => {
                // Dropping the closure ends this loop; wasm-bindgen frees it after this call returns.
                drop(again.borrow_mut().take());
                wasm_bindgen_futures::spawn_local(recover(canvas_id.clone(), generation + 1));
                return;
            }
            Check::Over => {
                drop(again.borrow_mut().take());
                return;
            }
        }
        request_frame(&again);
    }));
    request_frame(&slot);
}

/// Start over after a device loss, on a fresh instance, adapter and device.
async fn recover(canvas_id: String, generation: u32) {
    recover_with(
        canvas_id,
        generation,
        |id, g| async move { start(&id, g).await },
    )
    .await;
}

/// [`recover`] for any start routine (the shell's and the arena's): a few attempts with growing
/// pauses, each on a fresh instance, then the fatal page.
pub(crate) async fn recover_with<F, Fut>(canvas_id: String, generation: u32, start: F)
where
    F: Fn(String, u32) -> Fut,
    Fut: std::future::Future<Output = Result<String, (Failure, String)>>,
{
    set_state("recovering");
    let mut last = None;
    for attempt in 0..ATTEMPTS {
        pause(PAUSE_MS * i32::try_from(attempt).unwrap_or(0)).await;
        log_error(&format!(
            "device lost: starting device generation {generation}, attempt {}",
            attempt + 1
        ));
        match start(canvas_id.clone(), generation).await {
            Ok(_) => return,
            Err(e) => last = Some(e),
        }
    }
    if let Some((why, detail)) = last {
        fail(why, &format!("after a device loss: {detail}"));
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
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("engine-web"),
            required_features: wanted_features(adapter.features()),
            required_limits: required_limits(),
            ..Default::default()
        })
        .await
        .map_err(|e| (Failure::NoDevice, e.to_string()))?;
    device.on_uncaptured_error(Arc::new(|e: wgpu::Error| log_error(&format!("wgpu: {e}"))));
    let lost = Arc::new(LostBox::default());
    let lost_by_callback = Arc::clone(&lost);
    device.set_device_lost_callback(move |reason, message| {
        log_error(&format!("wgpu device lost ({reason:?}): {message}"));
        lost_by_callback.set(reason_word(reason), &message);
    });

    let max_dim = device.limits().max_texture_dimension_2d;
    let dpr = web_sys::window().map_or(1.0, |w| w.device_pixel_ratio());
    let (w, h) = backing_size(
        f64::from(canvas.client_width()),
        f64::from(canvas.client_height()),
        dpr,
        max_dim,
    );
    canvas.set_width(w);
    canvas.set_height(h);
    let config = surface.get_default_config(&adapter, w, h).ok_or_else(|| {
        (
            Failure::NoSurface,
            "the adapter cannot present to this canvas".to_string(),
        )
    })?;
    surface.configure(&device, &config);

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("engine-web-square"),
        source: wgpu::ShaderSource::Wgsl(SHADER_WGSL.into()),
    });
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("engine-web-square-uniform"),
        size: 32,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let layout_entries = [wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }];
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("engine-web-square-layout"),
        entries: &layout_entries,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("engine-web-square-bind"),
        layout: &bgl,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        }],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("engine-web-square-pipeline-layout"),
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("engine-web-square-pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs"),
            targets: &[Some(wgpu::ColorTargetState {
                format: config.format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    let [vendor, architecture, web_device, description] = raw_adapter_info(&gpu).await;
    let info = adapter.get_info();
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
        features: feature_names(device.features()),
        limits: limit_rows(&device.limits()),
        wasm_pages: wasm_pages(),
        generation,
        canvas: (w, h),
        surface_format: format!("{:?}", config.format).to_ascii_lowercase(),
        user_agent: user_agent(),
        cross_origin_isolated: cross_origin_isolated(),
    };

    let mut frame = Frame {
        lost,
        surface,
        device,
        queue,
        config,
        pipeline,
        bind_group,
        uniform,
        canvas,
        max_dim,
        stats: FrameStats::default(),
    };
    // The first frame is drawn here, so `running` means a frame reached the canvas.
    frame.draw(now_ms());
    let line = ready.line();
    set_text("report", &line);
    set_body_attr("data-generation", &generation.to_string());
    set_state("running");
    web_sys::console::log_1(&JsValue::from_str(&line));
    run_loop(frame, canvas_id.to_string(), generation);
    Ok(line)
}

/// The browser's millisecond clock (`performance.now()`); `std::time` traps on this target.
pub(crate) fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map_or(0.0, |p| p.now())
}

/// The page's clock as the time source of `engine-clock`, which the asset and app crates read
/// (they panic on this target until a source is installed).
pub(crate) fn browser_clock() -> std::time::Duration {
    duration_from_ms(now_ms())
}

/// Run the golden tape and the cut corpus in this module and show the verdict on the page
/// (`#golden`, `data-golden` on `<body>`) and in the console.
pub(crate) fn run_golden() {
    let outcome = golden::check(&mut now_ms);
    set_text("golden", &outcome.text());
    set_body_attr("data-golden", outcome.status.word());
    let line = JsValue::from_str(&outcome.line);
    if outcome.status == Status::Ok {
        web_sys::console::log_1(&line);
    } else {
        web_sys::console::error_1(&line);
    }
    if let Some(tick) = &outcome.tick_line {
        web_sys::console::log_1(&JsValue::from_str(tick));
    }
}

/// Start the shell on the canvas with id `canvas_id`. Resolves with the JSON line it printed
/// (`"ok":true` once the first frame is on the canvas, `"ok":false` with a reason when the
/// browser has no WebGPU or refuses a device). It does not throw for those cases. After that
/// line, with or without a GPU, it runs the golden check and prints its verdict line.
#[wasm_bindgen]
pub async fn run(canvas_id: String) -> String {
    std::panic::set_hook(Box::new(|info| log_error(&format!("panic: {info}"))));
    set_state("starting");
    // Input and audio need the DOM only, so they start before (and without) the GPU.
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

/// Destroy the current device on the next frame and treat it as a loss (a forced loss, QW8). For
/// the headless check: it proves the page survives a lost device by coming back with
/// `data-generation` one higher, the frame counter moving, and the loss in the receipt.
#[wasm_bindgen]
pub fn lose_device() {
    DESTROY_ASKED.with(|d| d.set(Some(Cause::Forced)));
}

/// Destroy the current device on the next frame, on purpose and for good (a requested destroy,
/// QW8): the page is going away. It is not a loss and nothing starts over; the page ends in
/// `data-state="stopped"`. The page calls it from `pagehide`.
#[wasm_bindgen]
pub fn destroy_device() {
    DESTROY_ASKED.with(|d| d.set(Some(Cause::Requested)));
}
