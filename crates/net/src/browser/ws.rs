//! ws: the page's `WebSocket` behind the [`Transport`] trait (wasm32).
//!
//! Callbacks push into one shared inbox; the tick loop drains it with
//! `recv`. Sends go straight to the socket in the
//! [`wire`](crate::websocket::wire) layout. The socket's own send buffer is
//! the backpressure: past one channel's memory a datagram-class frame is shed
//! (loss) and a reliable one is refused loudly.

use std::cell::RefCell;
use std::rc::Rc;

use js_sys::{ArrayBuffer, Promise, Uint8Array};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{BinaryType, CloseEvent, Event, MessageEvent, WebSocket};

use super::inbox::Inbox;
use super::ClientState;
use crate::connect_race::{Leg, LegError};
use crate::quic::{check_budget, ControlKind, Inbound, PeerId, Transport, CHANNEL_MEMORY_BYTES};
use crate::session::NetError;
use crate::websocket::wire::{self, Class};

struct Shared {
    inbox: Inbox,
    state: ClientState,
}

type Handler<E> = Closure<dyn FnMut(E)>;

/// A WebSocket session to the dedicated server.
pub struct WsClient {
    socket: WebSocket,
    server: PeerId,
    shared: Rc<RefCell<Shared>>,
    // Held so the callbacks live as long as the socket does.
    _on_message: Handler<MessageEvent>,
    _on_close: Handler<CloseEvent>,
    _on_error: Handler<Event>,
}

fn leg_error(reason: impl Into<String>) -> LegError {
    LegError {
        leg: Leg::WebSocket,
        reason: reason.into(),
    }
}

fn describe(value: &JsValue) -> String {
    value
        .as_string()
        .or_else(|| {
            js_sys::JSON::stringify(value)
                .ok()
                .and_then(|text| text.as_string())
        })
        .unwrap_or_else(|| "unknown error".to_owned())
}

/// Wait until the socket opens, or fails before it does.
async fn opened(socket: &WebSocket) -> Result<(), LegError> {
    let mut keep: Vec<Handler<JsValue>> = Vec::new();
    let promise = Promise::new(&mut |resolve, reject| {
        let on_open = Closure::<dyn FnMut(JsValue)>::new(move |_| {
            // A resolved promise ignores a second call.
            let _ = resolve.call0(&JsValue::UNDEFINED);
        });
        socket.set_onopen(Some(on_open.as_ref().unchecked_ref()));
        let on_error = {
            let reject = reject.clone();
            Closure::<dyn FnMut(JsValue)>::new(move |_| {
                let _ = reject.call1(
                    &JsValue::UNDEFINED,
                    &JsValue::from_str("the socket could not connect (refused, blocked or no TLS)"),
                );
            })
        };
        socket.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        let on_close = Closure::<dyn FnMut(JsValue)>::new(move |event: JsValue| {
            let code = event.dyn_ref::<CloseEvent>().map_or(0, CloseEvent::code);
            let _ = reject.call1(
                &JsValue::UNDEFINED,
                &JsValue::from_str(&format!("closed before open (close code {code})")),
            );
        });
        socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));
        keep.extend([on_open, on_error, on_close]);
    });
    let outcome = JsFuture::from(promise).await;
    socket.set_onopen(None);
    drop(keep);
    outcome
        .map(drop)
        .map_err(|reason| leg_error(describe(&reason)))
}

impl WsClient {
    /// Open `url` (`wss://host/e2040`) and wait for the handshake. `server`
    /// is the peer id the trait reports frames from, and the only peer sends
    /// accept.
    pub async fn connect(url: &str, server: PeerId) -> Result<Self, LegError> {
        let socket = WebSocket::new(url).map_err(|reason| leg_error(describe(&reason)))?;
        socket.set_binary_type(BinaryType::Arraybuffer);
        opened(&socket).await?;

        let shared = Rc::new(RefCell::new(Shared {
            inbox: Inbox::new(server),
            state: ClientState::Open,
        }));
        let on_message = {
            let shared = Rc::clone(&shared);
            let socket = socket.clone();
            Handler::<MessageEvent>::new(move |event: MessageEvent| {
                let Ok(buffer) = event.data().dyn_into::<ArrayBuffer>() else {
                    fail(&shared, &socket, "the server sent a text message");
                    return;
                };
                let bytes = Uint8Array::new(&buffer).to_vec();
                match wire::decode(&bytes) {
                    Ok((Class::Datagram, payload)) => {
                        shared.borrow_mut().inbox.push_datagram(payload);
                    }
                    Ok((Class::Reliable(kind), payload)) => {
                        shared
                            .borrow_mut()
                            .inbox
                            .push_stream(kind, payload.to_vec());
                    }
                    Err(error) => fail(&shared, &socket, &format!("bad message: {error:?}")),
                }
            })
        };
        let on_close = {
            let shared = Rc::clone(&shared);
            Handler::<CloseEvent>::new(move |event: CloseEvent| {
                let mut shared = shared.borrow_mut();
                if shared.state == ClientState::Open {
                    shared.state = ClientState::Closed(format!(
                        "closed by the server (code {}, {})",
                        event.code(),
                        event.reason()
                    ));
                }
            })
        };
        let on_error = {
            let shared = Rc::clone(&shared);
            Handler::<Event>::new(move |_: Event| {
                let mut shared = shared.borrow_mut();
                if shared.state == ClientState::Open {
                    shared.state = ClientState::Closed("socket error".to_owned());
                }
            })
        };
        socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));
        socket.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        Ok(Self {
            socket,
            server,
            shared,
            _on_message: on_message,
            _on_close: on_close,
            _on_error: on_error,
        })
    }

    /// Whether frames flow, or why they stopped.
    #[must_use]
    pub fn state(&self) -> ClientState {
        self.shared.borrow().state.clone()
    }

    /// Datagram-class frames shed on receive so far.
    #[must_use]
    pub fn dropped_datagrams(&self) -> u64 {
        self.shared.borrow().inbox.dropped_datagrams()
    }

    fn send(&self, to: PeerId, message: &[u8], shed_when_full: bool) -> Result<(), NetError> {
        if to != self.server || self.state() != ClientState::Open {
            return Err(NetError::BadInput);
        }
        // The socket may hold one channel of memory unsent before sends shed or refuse.
        let queued = usize::try_from(self.socket.buffered_amount()).unwrap_or(usize::MAX);
        if queued > CHANNEL_MEMORY_BYTES {
            return if shed_when_full {
                Ok(())
            } else {
                Err(NetError::BadInput)
            };
        }
        self.socket
            .send_with_u8_array(message)
            .map_err(|_| NetError::BadInput)
    }
}

/// End the session loudly: record why, then close.
fn fail(shared: &Rc<RefCell<Shared>>, socket: &WebSocket, why: &str) {
    shared.borrow_mut().state = ClientState::Closed(why.to_owned());
    // Closing an already closing socket is harmless.
    let _ = socket.close_with_code_and_reason(1002, "protocol violation");
}

impl Transport for WsClient {
    fn send_unreliable(&mut self, to: PeerId, bytes: &[u8]) -> Result<(), NetError> {
        check_budget(bytes)?;
        self.send(to, &wire::encode(Class::Datagram, bytes)?, true)
    }

    fn send_reliable(
        &mut self,
        to: PeerId,
        kind: ControlKind,
        bytes: &[u8],
    ) -> Result<(), NetError> {
        self.send(to, &wire::encode(Class::Reliable(kind), bytes)?, false)
    }

    fn recv(&mut self) -> Vec<Inbound> {
        self.shared.borrow_mut().inbox.drain()
    }
}

impl Drop for WsClient {
    fn drop(&mut self) {
        self.socket.set_onmessage(None);
        self.socket.set_onclose(None);
        self.socket.set_onerror(None);
        // Closing an already closed socket is harmless.
        let _ = self.socket.close();
    }
}
