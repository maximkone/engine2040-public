//! Browser glue for input and audio (WEB7, A5). Compiled for `wasm32` only. The decisions live in
//! [`crate::input`], [`crate::audio`] and [`crate::audio_worklet`] and are tested natively; this
//! file wires DOM events, the Gamepad API and the WebAudio sink to them, and prints the page lines:
//!
//! - `input: kb=<n> mouse=<n> pad=<n>` in `#input`
//! - `audio: state=<suspended|running> underruns=<n>` in `#audio` (the receipt, with the fallback
//!   reason when the worklet was refused)
//! - `audio: path=<worklet|script|none> blocks=<n> underruns=<n>` in `#audioworklet`
//!
//! It runs its own animation-frame pump (pads are polled, lines refreshed) instead of riding the
//! GPU frame loop: that loop stops while a lost device is rebuilt and does not exist at all on a
//! browser without WebGPU, and input and audio must keep working through both.
//!
//! The `AudioContext` is created only inside the first user gesture (a key that counts as
//! activation, a mouse press, a touch end), never before. Once it exists, the engine's own mixer
//! (`crate::audio_worklet::build_mixer`) plays the arena's cue table: the page renders ring blocks
//! with it and an `AudioWorklet` processor pulls them in the audio thread. When the worklet module
//! cannot load the page falls back to the deprecated `ScriptProcessorNode`, prints why, and keeps
//! playing the same mixer.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use engine_audio::combat_cues::cue_for_event;
use engine_audio::{AudioEngine, FeedTx, FightCue, FrameSource};
use engine_input::map::Key;
use engine_input::Buttons;
use engine_net::session::{BodyId, BodyInput};
use engine_net::webduel::Pilot;
use js_sys::Reflect;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AudioContext, AudioProcessingEvent, AudioWorkletNode, AudioWorkletNodeOptions, Blob,
    BlobPropertyBag, Gamepad, GamepadButton, HtmlCanvasElement, KeyboardEvent, MessageEvent,
    MessagePort, MouseEvent, ScriptProcessorNode, Url,
};

use crate::audio::{keydown_is_gesture, Act, AudioGate, CtxState, BLOCK_FRAMES};
use crate::audio_worklet::{
    build_mixer, count_u64, queue_cue_script, rate_hz, worklet_mode, MixerSnapshot, Path,
    SinkReport, WorkletMode, BROKEN_MODULE, CUE_ATTACKER_CM, CUE_VICTIM_CM, PROCESSOR, RING_BLOCKS,
    RING_LOW_WATER, WORKLET_BLOCK_FRAMES,
};
use crate::input::{
    input_line, swallows_default, Facing, InputState, PadReading, PadSource, MILLI_DEG_PER_PX,
};
use crate::input_tape::Tape;
use crate::shell::{document, now_ms, set_body_attr, set_text};

/// The processor itself: shipped inside the wasm bundle as a blob module, so the page fetches no
/// second script and the build copies no extra file (A5).
const WORKLET_JS: &str = include_str!("../audio-worklet.js");

thread_local! {
    static INPUT: RefCell<InputState> = RefCell::new(InputState::default());
    static GATE: RefCell<AudioGate> = const { RefCell::new(AudioGate::new()) };
    static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
    /// What the audio sink is doing, for the two printed lines.
    static REPORT: RefCell<SinkReport> = const { RefCell::new(SinkReport::pending()) };
    /// The records `input_sample` has taken (WEB7).
    static TAPE: RefCell<Tape> = RefCell::new(Tape::default());
    static INSTALLED: Cell<bool> = const { Cell::new(false) };
    /// The three printed lines as last written, and when the console last heard the input line.
    static SHOWN: RefCell<Shown> = const { RefCell::new(Shown::new()) };
}

/// The last text written to each line, so a frame writes only on change.
struct Shown {
    input: String,
    audio: String,
    path: String,
    logged_input: f64,
}

impl Shown {
    const fn new() -> Self {
        Self {
            input: String::new(),
            audio: String::new(),
            path: String::new(),
            logged_input: -1.0e9,
        }
    }
}

/// The live `AudioContext`, its sink node and the closures that keep it playing. Dropping it stops
/// the sound.
struct Sink {
    ctx: AudioContext,
    _node: SinkNode,
    _statechange: Closure<dyn FnMut()>,
    /// The game side of the mixer feed: cue plays go here.
    tx: Rc<RefCell<FeedTx>>,
    /// The mixer the sink renders (native-device path unchanged; the page just owns it here).
    engine: Rc<RefCell<AudioEngine>>,
}

/// Which node pulls the mixer. `None` means neither path could be built; the report says why.
enum SinkNode {
    Worklet {
        _node: AudioWorkletNode,
        _onmessage: Closure<dyn FnMut(MessageEvent)>,
        _ring: Rc<RefCell<Ring>>,
    },
    Script {
        _node: ScriptProcessorNode,
        _process: Closure<dyn FnMut(AudioProcessingEvent)>,
    },
    None,
}

fn log(text: &str) {
    web_sys::console::log_1(&JsValue::from_str(text));
}

/// One field of a message object; `None` when absent or of another type.
fn field(value: &JsValue, name: &str) -> Option<JsValue> {
    js_sys::Reflect::get(value, &JsValue::from_str(name)).ok()
}

fn field_str(value: &JsValue, name: &str) -> Option<String> {
    field(value, name)?.as_string()
}

fn field_f64(value: &JsValue, name: &str) -> Option<f64> {
    field(value, name)?.as_f64()
}

/// A `JsValue` error as one printable reason: `Name: message` when the browser gave both.
fn js_reason(e: &JsValue) -> String {
    if let Some(text) = e.as_string() {
        return text;
    }
    match (field_str(e, "name"), field_str(e, "message")) {
        (Some(name), Some(message)) => format!("{name}: {message}"),
        (Some(name), None) => name,
        (None, Some(message)) => message,
        (None, None) => format!("{e:?}"),
    }
}

/// Write both lines to the page when they changed, and to the console (the input line at most
/// once a second, the audio lines on every change: they are rare and they are the sink proof).
fn refresh() {
    let counts = INPUT.with(|i| i.borrow().counts());
    let input = input_line(counts);
    let (label, created, gestures) = GATE.with(|g| {
        let g = g.borrow();
        (g.label(), g.has_context(), g.gestures())
    });
    let (audio, path, sink_path) = REPORT.with(|r| {
        let r = r.borrow();
        let sink = match r.path() {
            Path::Worklet => "worklet",
            Path::Script => "script",
            Path::None => "none",
        };
        (r.line(label), r.path_line(), sink)
    });
    let now = now_ms();
    SHOWN.with(|shown| {
        let mut shown = shown.borrow_mut();
        if shown.input != input {
            set_text("input", &input);
            set_body_attr("data-input-kb", &counts.kb.to_string());
            set_body_attr("data-input-mouse", &counts.mouse.to_string());
            set_body_attr("data-input-pad", &counts.pad.to_string());
            if now - shown.logged_input >= 1000.0 {
                log(&input);
                shown.logged_input = now;
            }
            shown.input = input;
        }
        if shown.audio != audio {
            set_text("audio", &audio);
            set_body_attr("data-audio", label);
            log(&audio);
            shown.audio = audio;
        }
        if shown.path != path {
            set_text("audioworklet", &path);
            set_body_attr("data-audio-path", sink_path);
            shown.path = path;
        }
    });
    set_body_attr(
        "data-audio-context",
        if created { "created" } else { "none" },
    );
    set_body_attr("data-audio-gestures", &gestures.to_string());
}

/// Take a new underrun total from the processor and repaint when the printed total moved.
fn observe_underruns(total: u64) {
    let changed = REPORT.with(|r| r.borrow_mut().observe(total));
    if changed {
        refresh();
    }
}

// ---- pads ----

/// `navigator.getGamepads()` as the pure mapping reads it.
struct NavigatorPads;

impl PadSource for NavigatorPads {
    fn read(&mut self) -> Vec<PadReading> {
        let Some(window) = web_sys::window() else {
            return Vec::new();
        };
        // Blocked outside a secure context or by a permissions policy: no pads, not an error.
        let Ok(list) = window.navigator().get_gamepads() else {
            return Vec::new();
        };
        list.iter()
            .map(|slot| match slot.dyn_into::<Gamepad>() {
                Ok(pad) => PadReading {
                    connected: pad.connected(),
                    axes: pad
                        .axes()
                        .iter()
                        .map(|a| a.as_f64().unwrap_or(0.0))
                        .collect(),
                    buttons: pad
                        .buttons()
                        .iter()
                        .map(|b| b.dyn_into::<GamepadButton>().is_ok_and(|b| b.pressed()))
                        .collect(),
                },
                Err(_) => PadReading::default(),
            })
            .collect()
    }
}

// ---- audio ----

fn context_state(ctx: &AudioContext) -> Option<CtxState> {
    // Read as a word: web-sys's enum has no `interrupted`, which Safari reports.
    let word = js_sys::Reflect::get(ctx, &JsValue::from_str("state")).ok()?;
    CtxState::parse(&word.as_string()?)
}

fn sync_state(ctx: &AudioContext) {
    if let Some(state) = context_state(ctx) {
        GATE.with(|g| g.borrow_mut().on_state(state));
    }
    refresh();
}

/// The page's search string (`?worklet=broken`), for the sink flags.
fn search_param() -> String {
    web_sys::window()
        .and_then(|window| window.location().search().ok())
        .unwrap_or_default()
}

/// The context, the mixer feed and the node that pulls it. Only called from a gesture.
fn make_sink() -> Result<Sink, JsValue> {
    let ctx = AudioContext::new()?;
    let rate = rate_hz(ctx.sample_rate());
    let (tx, engine) =
        build_mixer(rate).map_err(|e| JsValue::from_str(&format!("mixer: {e:?}")))?;
    let watched = ctx.clone();
    let statechange = Closure::<dyn FnMut()>::new(move || sync_state(&watched));
    ctx.set_onstatechange(Some(statechange.as_ref().unchecked_ref()));
    Ok(Sink {
        ctx,
        _node: SinkNode::None,
        _statechange: statechange,
        tx: Rc::new(RefCell::new(tx)),
        engine: Rc::new(RefCell::new(engine)),
    })
}

/// One stereo block the page renders the mixer into: interleaved scratch plus the two planar
/// arrays the worklet receives.
struct Ring {
    mix: Vec<f32>,
    left: Vec<f32>,
    right: Vec<f32>,
}

impl Ring {
    fn new(frames: u32) -> Self {
        let frames = usize::try_from(frames).unwrap_or(1).max(1);
        Self {
            mix: vec![0.0; frames * 2],
            left: vec![0.0; frames],
            right: vec![0.0; frames],
        }
    }

    /// Render one block of the page's mixer and split it into the planar arrays.
    fn fill(&mut self, engine: &mut AudioEngine) -> (&[f32], &[f32]) {
        engine.render_stereo_into(&mut self.mix);
        for (out, frame) in self.left.iter_mut().zip(self.mix.chunks_exact(2)) {
            *out = frame[0];
        }
        for (out, frame) in self.right.iter_mut().zip(self.mix.chunks_exact(2)) {
            *out = frame[1];
        }
        (&self.left, &self.right)
    }
}

/// The processor script as a blob URL: the wasm bundle carries it, the page fetches nothing.
fn worklet_blob_url() -> Result<String, JsValue> {
    let parts = js_sys::Array::new();
    parts.push(&JsValue::from_str(WORKLET_JS));
    let bag = BlobPropertyBag::new();
    bag.set_type("text/javascript");
    let blob = Blob::new_with_str_sequence_and_options(&parts, &bag)?;
    Url::create_object_url_with_blob(&blob)
}

/// A `usize` as the JavaScript number the processor options carry.
fn count_f64(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

/// The constructor options: no input, one stereo output, and the ring geometry the Rust side owns.
fn node_options() -> Result<AudioWorkletNodeOptions, JsValue> {
    let options = AudioWorkletNodeOptions::new();
    options.set_number_of_inputs(0);
    options.set_number_of_outputs(1);
    let channels = js_sys::Array::new();
    channels.push(&JsValue::from_f64(2.0));
    options.set_output_channel_count(&channels);
    let processor = js_sys::Object::new();
    Reflect::set(
        &processor,
        &JsValue::from_str("lowWater"),
        &JsValue::from_f64(f64::from(RING_LOW_WATER)),
    )?;
    Reflect::set(
        &processor,
        &JsValue::from_str("maxQueued"),
        &JsValue::from_f64(count_f64(RING_BLOCKS)),
    )?;
    options.set_processor_options(Some(&processor));
    Ok(options)
}

/// Post two planar arrays as one `block` message, transferring their buffers (no copy).
fn post_arrays(
    port: &MessagePort,
    left: js_sys::Float32Array,
    right: js_sys::Float32Array,
) -> Result<(), JsValue> {
    let msg = js_sys::Object::new();
    Reflect::set(
        &msg,
        &JsValue::from_str("type"),
        &JsValue::from_str("block"),
    )?;
    Reflect::set(&msg, &JsValue::from_str("left"), &left)?;
    Reflect::set(&msg, &JsValue::from_str("right"), &right)?;
    let transfer = js_sys::Array::new();
    transfer.push(&JsValue::from(left.buffer()));
    transfer.push(&JsValue::from(right.buffer()));
    port.post_message_with_transferable(&msg, &transfer)
}

/// Render the mixer and post one fresh block to the processor.
fn send_block(port: &MessagePort, left: &[f32], right: &[f32]) -> Result<(), JsValue> {
    let l = js_sys::Float32Array::new_with_length(u32::try_from(left.len()).unwrap_or(0));
    let r = js_sys::Float32Array::new_with_length(u32::try_from(right.len()).unwrap_or(0));
    l.copy_from(left);
    r.copy_from(right);
    post_arrays(port, l, r)
}

/// Fill the ring before the first render quantum: every block is mixer audio, never silence.
fn prime_ring(
    ring: &Rc<RefCell<Ring>>,
    engine: &Rc<RefCell<AudioEngine>>,
    port: &MessagePort,
) -> Result<(), JsValue> {
    for _ in 0..RING_BLOCKS {
        let posted = {
            let mut ring = ring.borrow_mut();
            let (left, right) = ring.fill(&mut engine.borrow_mut());
            send_block(port, left, right)
        };
        posted?;
        REPORT.with(|r| r.borrow_mut().sent());
    }
    Ok(())
}

/// A block the processor fully played came back (with its empty arrays): refill and return it.
fn refill_spent(
    data: &JsValue,
    port: &MessagePort,
    ring: &Rc<RefCell<Ring>>,
    engine: &Rc<RefCell<AudioEngine>>,
) {
    let (Some(left), Some(right)) = (
        field(data, "left").and_then(|v| v.dyn_into::<js_sys::Float32Array>().ok()),
        field(data, "right").and_then(|v| v.dyn_into::<js_sys::Float32Array>().ok()),
    ) else {
        return;
    };
    let posted = {
        let mut ring = ring.borrow_mut();
        let (left_frames, right_frames) = ring.fill(&mut engine.borrow_mut());
        if left.length() == WORKLET_BLOCK_FRAMES && right.length() == WORKLET_BLOCK_FRAMES {
            left.copy_from(left_frames);
            right.copy_from(right_frames);
            post_arrays(port, left, right)
        } else {
            // A processor that changed the block size: give it the geometry the page owns.
            send_block(port, left_frames, right_frames)
        }
    };
    if posted.is_ok() {
        REPORT.with(|r| r.borrow_mut().sent());
    }
}

/// The page's half of the ring: the processor asks for `free` blocks when it runs low.
fn refill_wanted(
    data: &JsValue,
    port: &MessagePort,
    ring: &Rc<RefCell<Ring>>,
    engine: &Rc<RefCell<AudioEngine>>,
) {
    let free = usize::try_from(count_u64(field_f64(data, "free").unwrap_or(0.0)))
        .unwrap_or(0)
        .min(RING_BLOCKS);
    for _ in 0..free {
        let posted = {
            let mut ring = ring.borrow_mut();
            let (left, right) = ring.fill(&mut engine.borrow_mut());
            send_block(port, left, right)
        };
        if posted.is_err() {
            return;
        }
        REPORT.with(|r| r.borrow_mut().sent());
    }
}

/// One message from the audio thread. The underrun total is the processor's; the page prints it.
fn on_worklet_message(
    event: &MessageEvent,
    port: &MessagePort,
    ring: &Rc<RefCell<Ring>>,
    engine: &Rc<RefCell<AudioEngine>>,
) {
    let data = event.data();
    observe_underruns(count_u64(field_f64(&data, "underruns").unwrap_or(0.0)));
    match field_str(&data, "type").as_deref() {
        Some("spent") => refill_spent(&data, port, ring, engine),
        Some("want") => refill_wanted(&data, port, ring, engine),
        _ => {}
    }
}

/// Build the worklet node, open its port and prime the ring. The node is the only consumer.
fn attach_worklet(ctx: &AudioContext) -> Result<SinkNode, JsValue> {
    let engine = SINK
        .with(|s| s.borrow().as_ref().map(|s| Rc::clone(&s.engine)))
        .ok_or_else(|| JsValue::from_str("the audio sink went away"))?;
    let node = AudioWorkletNode::new_with_options(ctx, PROCESSOR, &node_options()?)?;
    let port = node.port()?;
    let ring = Rc::new(RefCell::new(Ring::new(WORKLET_BLOCK_FRAMES)));
    let onmessage = {
        let ring = Rc::clone(&ring);
        let engine = Rc::clone(&engine);
        let port = port.clone();
        Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
            on_worklet_message(&event, &port, &ring, &engine);
        })
    };
    port.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    node.connect_with_audio_node(&ctx.destination())?;
    prime_ring(&ring, &engine, &port)?;
    Ok(SinkNode::Worklet {
        _node: node,
        _onmessage: onmessage,
        _ring: ring,
    })
}

/// The deprecated node, playing the same mixer: the printed fallback when no worklet loads.
fn attach_script(
    ctx: &AudioContext,
    engine: &Rc<RefCell<AudioEngine>>,
) -> Result<SinkNode, JsValue> {
    let node = ctx.create_script_processor_with_buffer_size_and_number_of_input_channels_and_number_of_output_channels(
        BLOCK_FRAMES, 1, 2,
    )?;
    let engine = Rc::clone(engine);
    let mut mix = Vec::new();
    let mut left = Vec::new();
    let mut right = Vec::new();
    let process =
        Closure::<dyn FnMut(AudioProcessingEvent)>::new(move |event: AudioProcessingEvent| {
            let Ok(out) = event.output_buffer() else {
                return;
            };
            let frames = usize::try_from(out.length()).unwrap_or(0);
            if frames == 0 {
                return;
            }
            mix.resize(frames * 2, 0.0);
            left.resize(frames, 0.0);
            right.resize(frames, 0.0);
            engine.borrow_mut().render_stereo_into(&mut mix);
            for (i, frame) in mix.chunks_exact(2).enumerate() {
                left[i] = frame[0];
                right[i] = frame[1];
            }
            let _ = out.copy_to_channel(&left, 0);
            let _ = out.copy_to_channel(&right, 1);
            REPORT.with(|r| r.borrow_mut().sent());
        });
    node.set_onaudioprocess(Some(process.as_ref().unchecked_ref()));
    node.connect_with_audio_node(&ctx.destination())?;
    Ok(SinkNode::Script {
        _node: node,
        _process: process,
    })
}

/// Load the processor module (blob, or the deliberately missing URL) before constructing its node.
async fn load_worklet_module(ctx: &AudioContext) -> Result<(), JsValue> {
    let url = match worklet_mode(&search_param()) {
        WorkletMode::Auto => worklet_blob_url()?,
        WorkletMode::Broken => BROKEN_MODULE.to_string(),
        WorkletMode::Off => {
            return Err(JsValue::from_str(
                "worklet=off: the page was asked to skip the worklet",
            ))
        }
    };
    let promise = ctx.audio_worklet()?.add_module(&url)?;
    JsFuture::from(promise).await.map(|_| ())
}

/// The worklet path failed (module or node): print why and play the mixer through the script node.
fn fall_back(ctx: &AudioContext, reason: &str) {
    web_sys::console::warn_1(&JsValue::from_str(&format!(
        "audio: worklet=fallen-back reason={reason}"
    )));
    let attached = SINK
        .with(|s| s.borrow().as_ref().map(|s| Rc::clone(&s.engine)))
        .ok_or_else(|| JsValue::from_str("the audio sink went away"))
        .and_then(|engine| attach_script(ctx, &engine));
    match attached {
        Ok(node) => {
            SINK.with(|s| {
                if let Some(sink) = s.borrow_mut().as_mut() {
                    sink._node = node;
                }
            });
            REPORT.with(|r| *r.borrow_mut() = SinkReport::fallen_back(reason));
        }
        Err(e) => {
            let full = format!("{reason}; script: {}", js_reason(&e));
            REPORT.with(|r| *r.borrow_mut() = SinkReport::unavailable(full));
        }
    }
    refresh();
}

/// Try the worklet after the context exists; on any refusal take the printed fallback. Runs as a
/// task because `addModule` is a promise, so the gesture handler never blocks on a fetch.
fn start_worklet(ctx: AudioContext) {
    wasm_bindgen_futures::spawn_local(async move {
        match load_worklet_module(&ctx).await {
            Ok(()) => match attach_worklet(&ctx) {
                Ok(node) => {
                    let attached = SINK.with(|s| match s.borrow_mut().as_mut() {
                        Some(sink) => {
                            sink._node = node;
                            true
                        }
                        None => false,
                    });
                    if attached {
                        REPORT.with(|r| *r.borrow_mut() = SinkReport::worklet());
                    }
                    refresh();
                }
                Err(e) => fall_back(&ctx, &format!("node: {}", js_reason(&e))),
            },
            Err(e) => fall_back(&ctx, &js_reason(&e)),
        }
    });
}

/// Ask the existing context to resume (inside a gesture); a refusal is told to the gate.
fn resume() {
    let Some(ctx) = SINK.with(|s| s.borrow().as_ref().map(|s| s.ctx.clone())) else {
        return;
    };
    wasm_bindgen_futures::spawn_local(async move {
        let result = match ctx.resume() {
            Ok(promise) => JsFuture::from(promise).await.map(|_| ()),
            Err(e) => Err(e),
        };
        match result {
            Ok(()) => sync_state(&ctx),
            Err(e) => {
                web_sys::console::warn_1(&e);
                GATE.with(|g| g.borrow_mut().on_resume_failed());
                refresh();
            }
        }
    });
}

/// A user gesture happened: let the gate decide whether to create the context or resume it.
fn gesture() {
    match GATE.with(|g| g.borrow_mut().on_gesture()) {
        Act::Nothing => {}
        Act::CreateAndResume => match make_sink() {
            Ok(sink) => {
                let ctx = sink.ctx.clone();
                SINK.with(|s| *s.borrow_mut() = Some(sink));
                resume();
                start_worklet(ctx);
            }
            Err(e) => {
                let reason = js_reason(&e);
                web_sys::console::warn_1(&JsValue::from_str(&format!("audio: no sink: {reason}")));
                REPORT.with(|r| *r.borrow_mut() = SinkReport::unavailable(reason));
                // No context after all: back to waiting, the next gesture tries again.
                GATE.with(|g| g.borrow_mut().on_state(CtxState::Closed));
            }
        },
        Act::Resume => resume(),
    }
    refresh();
}

/// The cues of one played tick go to the page's mixer. Called by the arena when its cue set
/// changes; before the first gesture there is no sink and nothing is queued.
pub fn queue_fight_cues(cues: &[FightCue]) {
    let Some(tx) = SINK.with(|s| s.borrow().as_ref().map(|s| Rc::clone(&s.tx))) else {
        return;
    };
    let mut tx = tx.borrow_mut();
    for cue in cues {
        // The match has not exported fighter positions yet; the scripted blow's near-origin
        // placement keeps every cue inside the falloff's full-gain radius.
        if let Some(play) = cue_for_event(*cue, CUE_ATTACKER_CM, CUE_VICTIM_CM) {
            let _ = tx.play(0, play.name, play.gain, play.pos_cm);
        }
    }
}

/// The headless check's cue proof (A5): queue the scripted fight on the page's own mixer, apply it,
/// and compare the mixer's `started` delta with the plays the script queued. Works on both paths,
/// because both render this mixer.
#[wasm_bindgen]
pub fn audio_cue_check() -> String {
    let line = match SINK.with(|s| {
        s.borrow()
            .as_ref()
            .map(|s| (Rc::clone(&s.tx), Rc::clone(&s.engine)))
    }) {
        Some((tx, engine)) => {
            let before = MixerSnapshot::read(engine.borrow().mixer());
            let expected = queue_cue_script(&mut tx.borrow_mut());
            let _applied = engine.borrow_mut().drain();
            let proof = before.proof(expected, MixerSnapshot::read(engine.borrow().mixer()));
            proof.line()
        }
        None => "audio: cues none reason=\"no sink\"".to_string(),
    };
    set_body_attr("data-audio-cues", &line);
    log(&line);
    line
}

// ---- DOM events ----

fn listen<E: JsCast + 'static>(
    target: &web_sys::EventTarget,
    name: &str,
    mut handler: impl FnMut(E) + 'static,
) {
    let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
        if let Ok(ev) = ev.dyn_into::<E>() {
            handler(ev);
        }
    });
    let _ = target.add_event_listener_with_callback(name, closure.as_ref().unchecked_ref());
    // The listeners live as long as the page.
    closure.forget();
}

/// Whether two DOM objects are the same object (JavaScript `===`).
fn same(a: impl Into<JsValue>, canvas: &HtmlCanvasElement) -> bool {
    a.into() == JsValue::from(canvas.clone())
}

fn canvas_locked(canvas: &HtmlCanvasElement) -> bool {
    document()
        .and_then(|d| d.pointer_lock_element())
        .is_some_and(|el| same(el, canvas))
}

/// The human seat's input for `tick` at `facing` centidegrees: the keys, mouse and pad the page
/// holds right now, through the same bindings table the native window uses. Caller: the played
/// match (`crate::fight`), one call per fixed tick. The type names stay neutral here on purpose.
#[must_use]
pub fn seat_input(tick: u32, facing: i16) -> Option<BodyInput> {
    INPUT.with(|input| input.borrow().fighter_input(tick, BodyId(0), facing).ok())
}

/// The seat's turn for this frame: the pending mouse look in millidegrees plus whether Q and E
/// are held, for the caller's [`Facing`](crate::input::Facing).
#[must_use]
pub fn seat_look() -> (i32, bool, bool) {
    INPUT.with(|input| {
        let mut input = input.borrow_mut();
        let (yaw, _pitch) = input.take_look(MILLI_DEG_PER_PX);
        (yaw, input.held(Key::Q), input.held(Key::E))
    })
}

/// Start listening and printing. Safe to call once per page (a second call does nothing); works
/// with or without WebGPU, because it needs only the DOM.
pub fn install(canvas_id: &str) {
    if INSTALLED.with(|i| i.replace(true)) {
        return;
    }
    let (Some(window), Some(doc)) = (web_sys::window(), document()) else {
        return;
    };
    let canvas = doc
        .get_element_by_id(canvas_id)
        .and_then(|e| e.dyn_into::<HtmlCanvasElement>().ok());

    listen::<KeyboardEvent>(&doc, "keydown", |ev| {
        let code = ev.code();
        if swallows_default(&code) {
            ev.prevent_default();
        }
        INPUT.with(|i| i.borrow_mut().key_event(&code, true));
        if keydown_is_gesture(&code) {
            gesture();
        }
        refresh();
    });
    listen::<KeyboardEvent>(&doc, "keyup", |ev| {
        let code = ev.code();
        if swallows_default(&code) {
            ev.prevent_default();
        }
        INPUT.with(|i| i.borrow_mut().key_event(&code, false));
        refresh();
    });

    let for_down = canvas.clone();
    listen::<MouseEvent>(&doc, "mousedown", move |ev| {
        INPUT.with(|i| i.borrow_mut().mouse_button(ev.button(), true));
        gesture();
        // A press on the canvas takes the pointer (this is the user gesture pointer lock needs).
        if let Some(canvas) = &for_down {
            if !canvas_locked(canvas) && ev.target().is_some_and(|t| same(t, canvas)) {
                canvas.request_pointer_lock();
            }
        }
        refresh();
    });
    listen::<MouseEvent>(&doc, "mouseup", |ev| {
        INPUT.with(|i| i.borrow_mut().mouse_button(ev.button(), false));
        refresh();
    });
    listen::<MouseEvent>(&doc, "mousemove", |ev| {
        let moved = INPUT.with(|i| {
            i.borrow_mut()
                .mouse_move(f64::from(ev.movement_x()), f64::from(ev.movement_y()))
        });
        if moved {
            refresh();
        }
    });
    // A touch is a gesture for the audio context; it is not mapped to the fight yet.
    listen::<web_sys::Event>(&doc, "touchend", |_| gesture());
    // The right mouse button blocks; the page menu must not open over it.
    listen::<web_sys::Event>(&doc, "contextmenu", |ev| ev.prevent_default());

    let for_lock = canvas.clone();
    listen::<web_sys::Event>(&doc, "pointerlockchange", move |_| {
        let locked = for_lock.as_ref().is_some_and(canvas_locked);
        INPUT.with(|i| i.borrow_mut().set_pointer_lock(locked));
        log(&format!(
            "pointer lock: {}",
            if locked { "on" } else { "off" }
        ));
        refresh();
    });
    listen::<web_sys::Event>(&window, "blur", |_| {
        INPUT.with(|i| i.borrow_mut().release_all());
    });

    pump();
    refresh();
}

/// Sample one record of the input tape from what is held now (WEB7) and return the tape so far as
/// `input-tape: ticks=<n> bytes=<hex>`, also kept in `data-input-tape` on `<body>`. The headless check
/// calls it at the `tick` lines of a script it replays as real key and mouse events, and compares the
/// result with the `input_tape` bin's.
#[wasm_bindgen]
pub fn input_sample() -> String {
    let line = TAPE.with(|tape| {
        let mut tape = tape.borrow_mut();
        match INPUT.with(|input| tape.sample(&input.borrow())) {
            Ok(()) => tape.line(),
            Err(e) => format!("input-tape: failed {e:?}"),
        }
    });
    set_body_attr("data-input-tape", &line);
    line
}

/// The net duel's seat driven by the keyboard, mouse and pad the page listens to (WEB9): the
/// keys held when the client predicts a tick are that tick's input. Q and E turn, the mouse
/// turns too once the pointer is locked (click the canvas), the rest is the bindings table.
pub struct LivePilot {
    facing: Facing,
}

impl LivePilot {
    /// A pilot facing straight ahead.
    #[must_use]
    pub fn new() -> Self {
        Self {
            facing: Facing::default(),
        }
    }
}

impl Pilot for LivePilot {
    fn input(&mut self, tick: u32, slot: u8) -> BodyInput {
        let fighter = BodyId(slot);
        INPUT.with(|input| {
            let mut input = input.borrow_mut();
            let (yaw, _pitch) = input.take_look(MILLI_DEG_PER_PX);
            let facing = self
                .facing
                .turn(yaw, input.held(Key::Q), input.held(Key::E));
            // Every value is in range by construction; a seat that still fails idles.
            input
                .fighter_input(tick, fighter, facing)
                .unwrap_or(BodyInput {
                    tick,
                    fighter,
                    mx: 0,
                    mz: 0,
                    facing: 0,
                    buttons: Buttons::NONE,
                })
        })
    }
}

/// The pump's callback, shared with itself so it can ask for the frame after it.
type PumpSlot = RefCell<Option<Closure<dyn FnMut(f64)>>>;

/// The animation-frame pump: poll the pads, refresh the lines, ask for the next frame.
fn pump() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let slot: Rc<PumpSlot> = Rc::new(RefCell::new(None));
    let again = Rc::clone(&slot);
    let next = window.clone();
    *slot.borrow_mut() = Some(Closure::new(move |_now: f64| {
        INPUT.with(|i| i.borrow_mut().poll_pads(&mut NavigatorPads));
        refresh();
        if let Some(callback) = again.borrow().as_ref() {
            let _ = next.request_animation_frame(callback.as_ref().unchecked_ref());
        }
    }));
    if let Some(callback) = slot.borrow().as_ref() {
        let _ = window.request_animation_frame(callback.as_ref().unchecked_ref());
    };
}
