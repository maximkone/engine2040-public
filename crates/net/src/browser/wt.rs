//! wt: the page's `WebTransport` behind the [`Transport`] trait (wasm32).
//!
//! Datagram-class frames ride WebTransport datagrams, reliable frames ride
//! the one bidirectional stream this client opens, in the
//! [`stream`](super::stream) framing the native server reads. The server sees
//! the stream with the first byte written on it, so the first reliable send
//! (the join request) is what opens the session for it.
//!
//! The WebTransport API is declared here by hand: web-sys gates it behind an
//! unstable cfg flag that would change every build of the workspace. Reads
//! run as `spawn_local` loops that push into one shared inbox; writes are
//! fire and forget with an in-flight count as backpressure (a datagram is
//! shed past the count, a reliable frame is refused).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use js_sys::{Array, Object, Promise, Reflect, Uint8Array};
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{spawn_local, JsFuture};

use super::inbox::Inbox;
use super::stream::{pack, StreamDecoder};
use super::ClientState;
use crate::connect_race::{Leg, LegError};
use crate::quic::{check_budget, ControlKind, Inbound, PeerId, Transport, CHANNEL_MEMORY_BYTES};
use crate::session::{NetError, MAX_PACKET_BYTES};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = WebTransport)]
    type JsWebTransport;
    #[wasm_bindgen(constructor, js_class = "WebTransport", catch)]
    fn create(url: &str, options: &JsValue) -> Result<JsWebTransport, JsValue>;
    #[wasm_bindgen(method, getter)]
    fn ready(this: &JsWebTransport) -> Promise;
    #[wasm_bindgen(method, getter)]
    fn closed(this: &JsWebTransport) -> Promise;
    #[wasm_bindgen(method, getter)]
    fn datagrams(this: &JsWebTransport) -> JsEndpoint;
    #[wasm_bindgen(method, js_name = createBidirectionalStream)]
    fn create_bidirectional_stream(this: &JsWebTransport) -> Promise;
    #[wasm_bindgen(method, js_name = close)]
    fn close_transport(this: &JsWebTransport);

    /// The datagram duplex and the bidirectional stream share this shape.
    type JsEndpoint;
    #[wasm_bindgen(method, getter = readable)]
    fn endpoint_readable(this: &JsEndpoint) -> JsReadable;
    #[wasm_bindgen(method, getter = writable)]
    fn endpoint_writable(this: &JsEndpoint) -> JsWritable;

    type JsReadable;
    #[wasm_bindgen(method, js_name = getReader)]
    fn get_reader(this: &JsReadable) -> JsReader;
    type JsReader;
    #[wasm_bindgen(method, js_name = read)]
    fn read_chunk(this: &JsReader) -> Promise;

    type JsWritable;
    #[wasm_bindgen(method, js_name = getWriter)]
    fn get_writer(this: &JsWritable) -> JsWriter;
    type JsWriter;
    #[wasm_bindgen(method, js_name = write)]
    fn write_chunk(this: &JsWriter, chunk: &JsValue) -> Promise;
}

/// Writes in flight before a datagram is shed.
const DATAGRAMS_IN_FLIGHT: usize = 64;
/// Reliable frames in flight before a send is refused: one channel of memory.
const RELIABLE_IN_FLIGHT: usize = CHANNEL_MEMORY_BYTES / MAX_PACKET_BYTES;

struct Shared {
    inbox: Inbox,
    state: ClientState,
}

impl Shared {
    /// End the session with the first reason given; later ones add nothing.
    fn close_with(&mut self, why: String) {
        if !matches!(self.state, ClientState::Closed(_)) {
            self.state = ClientState::Closed(why);
        }
    }
}

type SharedRef = Rc<RefCell<Shared>>;

/// A WebTransport session to the dedicated server.
pub struct WtClient {
    transport: JsWebTransport,
    server: PeerId,
    shared: SharedRef,
    datagram_writer: JsWriter,
    stream_writer: JsWriter,
    datagrams_in_flight: Rc<Cell<usize>>,
    reliable_in_flight: Rc<Cell<usize>>,
    stopped: Rc<Cell<bool>>,
}

fn leg_error(reason: impl Into<String>) -> LegError {
    LegError {
        leg: Leg::WebTransport,
        reason: reason.into(),
    }
}

fn describe(value: &JsValue) -> String {
    let message = Reflect::get(value, &JsValue::from_str("message")).ok();
    message
        .and_then(|message| message.as_string())
        .or_else(|| value.as_string())
        .unwrap_or_else(|| "unknown error".to_owned())
}

/// The constructor options: pin the server certificate by its SHA-256 hash
/// (`serverCertificateHashes`) when one is given, else trust the browser's
/// own roots.
fn options(certificate_sha256: Option<[u8; 32]>) -> Result<JsValue, JsValue> {
    let options = Object::new();
    if let Some(hash) = certificate_sha256 {
        let entry = Object::new();
        Reflect::set(&entry, &"algorithm".into(), &"sha-256".into())?;
        Reflect::set(&entry, &"value".into(), &Uint8Array::from(&hash[..]))?;
        Reflect::set(
            &options,
            &"serverCertificateHashes".into(),
            &Array::of1(&entry),
        )?;
    }
    Ok(options.into())
}

/// Read one chunk: `Ok(Some(bytes))`, or `Ok(None)` when the stream is done.
async fn read_chunk(reader: &JsReader) -> Result<Option<Vec<u8>>, JsValue> {
    let result = JsFuture::from(reader.read_chunk()).await?;
    if Reflect::get(&result, &"done".into())?.is_truthy() {
        return Ok(None);
    }
    let value = Reflect::get(&result, &"value".into())?;
    Ok(Some(Uint8Array::new(&value).to_vec()))
}

async fn pump_datagrams(reader: JsReader, shared: SharedRef, stopped: Rc<Cell<bool>>) {
    while !stopped.get() {
        match read_chunk(&reader).await {
            Ok(Some(bytes)) => shared.borrow_mut().inbox.push_datagram(&bytes),
            Ok(None) => return,
            Err(error) => {
                shared
                    .borrow_mut()
                    .close_with(format!("datagram read failed: {}", describe(&error)));
                return;
            }
        }
    }
}

async fn pump_stream(reader: JsReader, shared: SharedRef, stopped: Rc<Cell<bool>>) {
    let mut decoder = StreamDecoder::new();
    while !stopped.get() {
        match read_chunk(&reader).await {
            Ok(Some(bytes)) => decoder.push(&bytes),
            Ok(None) => return,
            Err(error) => {
                shared
                    .borrow_mut()
                    .close_with(format!("stream read failed: {}", describe(&error)));
                return;
            }
        }
        loop {
            match decoder.next_message() {
                Ok(Some((kind, payload))) => shared.borrow_mut().inbox.push_stream(kind, payload),
                Ok(None) => break,
                Err(error) => {
                    shared
                        .borrow_mut()
                        .close_with(format!("bad stream frame: {error:?}"));
                    return;
                }
            }
        }
    }
}

/// Resolve when the session ends and say how.
async fn watch_closed(closed: Promise, shared: SharedRef) {
    let why = match JsFuture::from(closed).await {
        Ok(info) => {
            let code = Reflect::get(&info, &"closeCode".into())
                .ok()
                .and_then(|code| code.as_f64());
            let reason = Reflect::get(&info, &"reason".into())
                .ok()
                .and_then(|reason| reason.as_string());
            format!(
                "closed by the server (code {}, {})",
                code.unwrap_or(0.0),
                reason.unwrap_or_default()
            )
        }
        Err(error) => format!("session failed: {}", describe(&error)),
    };
    shared.borrow_mut().close_with(why);
}

impl WtClient {
    /// Open `url` (`https://host:port/e2040`) and wait for the session and
    /// its stream. With `certificate_sha256` the browser accepts a
    /// self-signed certificate with exactly that hash (at most 14 days old);
    /// without it the certificate must chain to the browser's roots.
    pub async fn connect(
        url: &str,
        certificate_sha256: Option<[u8; 32]>,
        server: PeerId,
    ) -> Result<Self, LegError> {
        let options = options(certificate_sha256).map_err(|error| leg_error(describe(&error)))?;
        let transport =
            JsWebTransport::create(url, &options).map_err(|error| leg_error(describe(&error)))?;
        JsFuture::from(transport.ready())
            .await
            .map_err(|error| leg_error(describe(&error)))?;
        let datagrams = transport.datagrams();
        let bidirectional: JsEndpoint = JsFuture::from(transport.create_bidirectional_stream())
            .await
            .map_err(|error| leg_error(describe(&error)))?
            .unchecked_into();

        let shared = Rc::new(RefCell::new(Shared {
            inbox: Inbox::new(server),
            state: ClientState::Open,
        }));
        let stopped = Rc::new(Cell::new(false));
        spawn_local(pump_datagrams(
            datagrams.endpoint_readable().get_reader(),
            Rc::clone(&shared),
            Rc::clone(&stopped),
        ));
        spawn_local(pump_stream(
            bidirectional.endpoint_readable().get_reader(),
            Rc::clone(&shared),
            Rc::clone(&stopped),
        ));
        spawn_local(watch_closed(transport.closed(), Rc::clone(&shared)));
        Ok(Self {
            datagram_writer: datagrams.endpoint_writable().get_writer(),
            stream_writer: bidirectional.endpoint_writable().get_writer(),
            transport,
            server,
            shared,
            datagrams_in_flight: Rc::new(Cell::new(0)),
            reliable_in_flight: Rc::new(Cell::new(0)),
            stopped,
        })
    }

    /// Whether frames flow, or why they stopped.
    #[must_use]
    pub fn state(&self) -> ClientState {
        self.shared.borrow().state.clone()
    }

    /// Datagrams shed on receive so far.
    #[must_use]
    pub fn dropped_datagrams(&self) -> u64 {
        self.shared.borrow().inbox.dropped_datagrams()
    }

    fn check_peer(&self, to: PeerId) -> Result<(), NetError> {
        if to == self.server && self.state() == ClientState::Open {
            Ok(())
        } else {
            Err(NetError::BadInput)
        }
    }

    /// Queue one write; the in-flight count falls when the browser has taken it.
    fn write(&self, writer: &JsWriter, chunk: &[u8], in_flight: &Rc<Cell<usize>>) {
        let promise = writer.write_chunk(&Uint8Array::from(chunk));
        in_flight.set(in_flight.get() + 1);
        let (in_flight, shared) = (Rc::clone(in_flight), Rc::clone(&self.shared));
        spawn_local(async move {
            let written = JsFuture::from(promise).await;
            in_flight.set(in_flight.get().saturating_sub(1));
            if let Err(error) = written {
                shared
                    .borrow_mut()
                    .close_with(format!("write failed: {}", describe(&error)));
            }
        });
    }
}

impl Transport for WtClient {
    fn send_unreliable(&mut self, to: PeerId, bytes: &[u8]) -> Result<(), NetError> {
        check_budget(bytes)?;
        self.check_peer(to)?;
        // A browser that cannot keep up sheds like loss; the redundancy
        // window recovers it.
        if self.datagrams_in_flight.get() < DATAGRAMS_IN_FLIGHT {
            self.write(&self.datagram_writer, bytes, &self.datagrams_in_flight);
        }
        Ok(())
    }

    fn send_reliable(
        &mut self,
        to: PeerId,
        kind: ControlKind,
        bytes: &[u8],
    ) -> Result<(), NetError> {
        let frame = pack(kind, bytes)?;
        self.check_peer(to)?;
        if self.reliable_in_flight.get() >= RELIABLE_IN_FLIGHT {
            return Err(NetError::BadInput);
        }
        self.write(&self.stream_writer, &frame, &self.reliable_in_flight);
        Ok(())
    }

    fn recv(&mut self) -> Vec<Inbound> {
        self.shared.borrow_mut().inbox.drain()
    }
}

impl Drop for WtClient {
    fn drop(&mut self) {
        self.stopped.set(true);
        // Closing an already closed session is harmless.
        self.transport.close_transport();
    }
}
