//! Browser glue for input and audio (WEB7). Compiled for `wasm32` only. The decisions live in
//! [`crate::input`] and [`crate::audio`] and are tested natively; this file wires DOM events, the
//! Gamepad API and a WebAudio sink to them, and prints the two page lines:
//!
//! - `input: kb=<n> mouse=<n> pad=<n>` in `#input`
//! - `audio: state=<suspended|running>` in `#audio`
//!
//! It runs its own animation-frame pump (pads are polled, lines refreshed) instead of riding the
//! GPU frame loop: that loop stops while a lost device is rebuilt and does not exist at all on a
//! browser without WebGPU, and input and audio must keep working through both.
//!
//! The `AudioContext` is created only inside the first user gesture (a key that counts as
//! activation, a mouse press, a touch end), never before.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use engine_input::map::Key;
use engine_input::Buttons;
use engine_net::session::{BodyId, BodyInput};
use engine_net::webduel::Pilot;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AudioContext, AudioProcessingEvent, Gamepad, GamepadButton, HtmlCanvasElement, KeyboardEvent,
    MouseEvent, ScriptProcessorNode,
};

use crate::audio::{
    keydown_is_gesture, render_planar, Act, AudioGate, Chime, CtxState, BLOCK_FRAMES,
};
use crate::input::{
    input_line, swallows_default, Facing, InputState, PadReading, PadSource, MILLI_DEG_PER_PX,
};
use crate::input_tape::Tape;
use crate::shell::{document, now_ms, set_body_attr, set_text};

thread_local! {
    static INPUT: RefCell<InputState> = RefCell::new(InputState::default());
    static GATE: RefCell<AudioGate> = const { RefCell::new(AudioGate::new()) };
    static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
    /// The records `input_sample` has taken (WEB7).
    static TAPE: RefCell<Tape> = RefCell::new(Tape::default());
    static INSTALLED: Cell<bool> = const { Cell::new(false) };
    /// The two lines as last written, and when the console last heard the input line.
    static SHOWN: RefCell<(String, String, f64)> = const { RefCell::new((String::new(), String::new(), -1.0e9)) };
}

/// The live `AudioContext` and the closures that keep it playing. Dropping it stops the sound.
struct Sink {
    ctx: AudioContext,
    _node: ScriptProcessorNode,
    _process: Closure<dyn FnMut(AudioProcessingEvent)>,
    _statechange: Closure<dyn FnMut()>,
}

fn log(text: &str) {
    web_sys::console::log_1(&JsValue::from_str(text));
}

/// Write both lines to the page when they changed, and to the console (the input line at most
/// once a second, the audio line on every change: it is rare and it is the gesture proof).
fn refresh() {
    let counts = INPUT.with(|i| i.borrow().counts());
    let input = input_line(counts);
    let (audio, label, created, gestures) = GATE.with(|g| {
        let g = g.borrow();
        (g.line(), g.label(), g.has_context(), g.gestures())
    });
    let now = now_ms();
    SHOWN.with(|shown| {
        let mut shown = shown.borrow_mut();
        if shown.0 != input {
            set_text("input", &input);
            set_body_attr("data-input-kb", &counts.kb.to_string());
            set_body_attr("data-input-mouse", &counts.mouse.to_string());
            set_body_attr("data-input-pad", &counts.pad.to_string());
            if now - shown.2 >= 1000.0 {
                log(&input);
                shown.2 = now;
            }
            shown.0 = input;
        }
        if shown.1 != audio {
            set_text("audio", &audio);
            set_body_attr("data-audio", label);
            log(&audio);
            shown.1 = audio;
        }
    });
    set_body_attr(
        "data-audio-context",
        if created { "created" } else { "none" },
    );
    set_body_attr("data-audio-gestures", &gestures.to_string());
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

/// The context, the chime source and the node that pulls it. Only called from a gesture.
fn make_sink() -> Result<Sink, JsValue> {
    let ctx = AudioContext::new()?;
    let node = ctx.create_script_processor_with_buffer_size_and_number_of_input_channels_and_number_of_output_channels(
        BLOCK_FRAMES, 1, 2,
    )?;
    let mut source = Chime::new(ctx.sample_rate());
    let (mut scratch, mut left, mut right) = (Vec::new(), Vec::new(), Vec::new());
    let process =
        Closure::<dyn FnMut(AudioProcessingEvent)>::new(move |ev: AudioProcessingEvent| {
            let Ok(out) = ev.output_buffer() else {
                return;
            };
            let frames = out.length() as usize;
            left.resize(frames, 0.0);
            right.resize(frames, 0.0);
            render_planar(&mut source, &mut scratch, &mut left, &mut right);
            let _ = out.copy_to_channel(&left, 0);
            let _ = out.copy_to_channel(&right, 1);
        });
    node.set_onaudioprocess(Some(process.as_ref().unchecked_ref()));
    node.connect_with_audio_node(&ctx.destination())?;
    let watched = ctx.clone();
    let statechange = Closure::<dyn FnMut()>::new(move || sync_state(&watched));
    ctx.set_onstatechange(Some(statechange.as_ref().unchecked_ref()));
    Ok(Sink {
        ctx,
        _node: node,
        _process: process,
        _statechange: statechange,
    })
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
                SINK.with(|s| *s.borrow_mut() = Some(sink));
                resume();
            }
            Err(e) => {
                web_sys::console::warn_1(&e);
                // No context after all: back to waiting, the next gesture tries again.
                GATE.with(|g| g.borrow_mut().on_state(CtxState::Closed));
            }
        },
        Act::Resume => resume(),
    }
    refresh();
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
