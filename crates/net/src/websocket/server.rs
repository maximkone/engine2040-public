//! server: WebSocket sessions behind the [`Transport`] trait.
//!
//! One dedicated thread runs a single-threaded tokio runtime that accepts
//! connections and pumps their messages; the tick loop never awaits. Per
//! session:
//!
//! - unreliable-class frames are queued from the caller's thread into a small
//!   bounded queue; a full queue sheds the frame like loss (the redundancy
//!   window recovers it) and counts it;
//! - reliable frames go to their own bounded queue that is always written
//!   first; a full queue means the client stopped reading and the send is
//!   refused loudly;
//! - received datagram-class frames fill one bounded inbox (a full inbox
//!   sheds), received reliable frames wait for room, so TCP backs the client
//!   up instead of losing events.
//!
//! A peer is the remote address seen at accept. A newer session from the same
//! address replaces the older one, so the address names at most one session.

use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinSet;
use tokio_tungstenite::tungstenite::error::{CapacityError, Error as WsError, ProtocolError};
use tokio_tungstenite::tungstenite::handshake::server::{
    Callback, ErrorResponse, Request, Response,
};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, WebSocketConfig};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{accept_hdr_async_with_config, WebSocketStream};

use super::wire::{self, Class, MAX_MESSAGE_BYTES};
use crate::quic::{
    ControlKind, Inbound, InboundFrame, PeerId, Transport, CHANNEL_MEMORY_BYTES, QUIC_KEEPALIVE,
};
use crate::session::{NetError, MAX_PACKET_BYTES};

/// URL path an upgrade request must name; anything else is answered 404.
pub const SESSION_PATH: &str = "/e2040";
/// Live sessions the server holds; the next request is answered 429.
pub const MAX_SESSIONS: usize = 16;
/// Frames the inbox holds before datagrams are dropped and streams wait.
const INBOX_FRAMES: usize = 1024;
/// Reliable frames queued per session: the channel memory cap in full frames.
pub const RELIABLE_FRAMES: usize = CHANNEL_MEMORY_BYTES / MAX_PACKET_BYTES;
/// Unreliable frames queued per session before the sender sheds.
const DATAGRAM_FRAMES: usize = 64;
/// Messages one write batch holds before it flushes.
const WRITE_BATCH: usize = 32;
/// Departures kept for the caller; the oldest drops first.
const MAX_DEPARTURES: usize = 64;
/// How long a connection may take to finish the upgrade.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// A session that sends nothing (not even a pong to the server's ping) for
/// this long is dropped.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// How long shutdown waits for sessions to say goodbye.
const DRAIN_WAIT: Duration = Duration::from_millis(500);

/// Why a session ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DepartReason {
    /// The client closed the session or its connection ended.
    PeerClosed,
    /// Nothing arrived within [`IDLE_TIMEOUT`].
    TimedOut,
    /// The server closed it: shutdown, or a newer session took the address.
    ServerClosed,
    /// The WebSocket layer failed (a broken frame, a reset socket).
    Protocol,
    /// The client sent a message the wire layout refuses.
    BadFrame(NetError),
}

/// One ended session, for the caller's heal and logging path.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Departure {
    /// The peer that left.
    pub peer: PeerId,
    /// Why it left.
    pub reason: DepartReason,
}

/// State shared between the tick loop and the I/O thread.
struct Shared {
    peers: Mutex<BTreeMap<SocketAddr, PeerLink>>,
    departures: Mutex<VecDeque<Departure>>,
    dropped_datagrams: AtomicU64,
    next_session: AtomicU64,
}

/// What the tick loop needs to reach one session. Dropping the link closes
/// both queues, which ends the session's writer.
struct PeerLink {
    session: u64,
    reliable: mpsc::Sender<Vec<u8>>,
    datagrams: mpsc::Sender<Vec<u8>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    /// Seat a session. A full table refuses; a session already seated at the
    /// address is replaced (its queues close, which ends it).
    fn admit(&self, peer: SocketAddr, link: PeerLink) -> bool {
        let mut peers = lock(&self.peers);
        if !peers.contains_key(&peer) && peers.len() >= MAX_SESSIONS {
            return false;
        }
        peers.insert(peer, link);
        true
    }

    /// Unseat `session` (unless a newer one already took the address) and
    /// record why it ended.
    fn depart(&self, peer: SocketAddr, session: u64, reason: DepartReason) {
        {
            let mut peers = lock(&self.peers);
            if peers.get(&peer).is_some_and(|link| link.session == session) {
                peers.remove(&peer);
            }
        }
        let mut departures = lock(&self.departures);
        if departures.len() == MAX_DEPARTURES {
            departures.pop_front();
        }
        departures.push_back(Departure {
            peer: PeerId(peer),
            reason,
        });
    }
}

/// The WebSocket server transport. Dropping it says goodbye to every session
/// and stops the I/O thread.
pub struct WsServer {
    local_addr: SocketAddr,
    shared: Arc<Shared>,
    inbox: mpsc::Receiver<Inbound>,
    stop: Option<oneshot::Sender<()>>,
    worker: Option<JoinHandle<()>>,
}

impl WsServer {
    /// Bind a TCP listener at `addr` and start accepting sessions. Port 0
    /// picks a free port; read it back with [`Self::local_addr`].
    pub fn bind(addr: SocketAddr) -> io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        // The listener registers its socket with the runtime it is made in.
        let listener = runtime.block_on(TcpListener::bind(addr))?;
        let local_addr = listener.local_addr()?;
        let shared = Arc::new(Shared {
            peers: Mutex::new(BTreeMap::new()),
            departures: Mutex::new(VecDeque::new()),
            dropped_datagrams: AtomicU64::new(0),
            next_session: AtomicU64::new(0),
        });
        let (inbox_tx, inbox) = mpsc::channel(INBOX_FRAMES);
        let (stop, stop_rx) = oneshot::channel();
        let worker = thread::Builder::new().name("ws-server".into()).spawn({
            let shared = Arc::clone(&shared);
            move || runtime.block_on(accept_loop(listener, shared, inbox_tx, stop_rx))
        })?;
        Ok(Self {
            local_addr,
            shared,
            inbox,
            stop: Some(stop),
            worker: Some(worker),
        })
    }

    /// The address sessions connect to.
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// The peers with a live session, in address order.
    #[must_use]
    pub fn peers(&self) -> Vec<PeerId> {
        lock(&self.shared.peers)
            .keys()
            .copied()
            .map(PeerId)
            .collect()
    }

    /// Drain the sessions that ended since the last call, oldest first.
    pub fn take_departures(&mut self) -> Vec<Departure> {
        lock(&self.shared.departures).drain(..).collect()
    }

    /// Datagram-class frames dropped in either direction: over the budget, or
    /// a queue was full.
    #[must_use]
    pub fn dropped_datagrams(&self) -> u64 {
        self.shared.dropped_datagrams.load(Ordering::Relaxed)
    }
}

impl Transport for WsServer {
    fn send_unreliable(&mut self, to: PeerId, bytes: &[u8]) -> Result<(), NetError> {
        let message = wire::encode(Class::Datagram, bytes)?;
        let peers = lock(&self.shared.peers);
        let link = peers.get(&to.0).ok_or(NetError::BadInput)?;
        match link.datagrams.try_send(message) {
            Ok(()) => Ok(()),
            // A slow client sheds like loss; the redundancy window recovers.
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.shared
                    .dropped_datagrams
                    .fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(mpsc::error::TrySendError::Closed(_)) => Err(NetError::BadInput),
        }
    }

    fn send_reliable(
        &mut self,
        to: PeerId,
        kind: ControlKind,
        bytes: &[u8],
    ) -> Result<(), NetError> {
        let message = wire::encode(Class::Reliable(kind), bytes)?;
        let peers = lock(&self.shared.peers);
        let link = peers.get(&to.0).ok_or(NetError::BadInput)?;
        // A full queue means the client stopped reading: refuse loudly
        // instead of growing without bound.
        link.reliable
            .try_send(message)
            .map_err(|_| NetError::BadInput)
    }

    fn recv(&mut self) -> Vec<Inbound> {
        // Bound one drain so a burst never starves the caller.
        std::iter::from_fn(|| self.inbox.try_recv().ok())
            .take(INBOX_FRAMES)
            .collect()
    }
}

impl Drop for WsServer {
    fn drop(&mut self) {
        let (stop, worker) = (self.stop.take(), self.worker.take());
        // An error means the loop already ended on its own.
        let _ = stop.map(|stop| stop.send(()));
        // A panicked worker has nothing left to clean up here.
        let _ = worker.map(JoinHandle::join);
    }
}

async fn accept_loop(
    listener: TcpListener,
    shared: Arc<Shared>,
    inbox: mpsc::Sender<Inbound>,
    mut stop: oneshot::Receiver<()>,
) {
    let (goodbye, goodbye_rx) = watch::channel(false);
    let mut sessions = JoinSet::new();
    loop {
        tokio::select! {
            _ = &mut stop => break,
            // Reap finished sessions so the set never grows.
            Some(_) = sessions.join_next(), if !sessions.is_empty() => {}
            accepted = listener.accept() => {
                // A failed accept (a reset before the handshake) is not the
                // server's problem.
                if let Ok((stream, peer)) = accepted {
                    sessions.spawn(serve(
                        Arc::clone(&shared),
                        inbox.clone(),
                        stream,
                        peer,
                        goodbye_rx.clone(),
                    ));
                }
            }
        }
    }
    // A send fails only when no session is left to hear it.
    let _ = goodbye.send(true);
    let drain = async { while sessions.join_next().await.is_some() {} };
    // Waiting is best effort: the close frames are already queued.
    let _ = tokio::time::timeout(DRAIN_WAIT, drain).await;
}

/// An upgrade refusal with an empty body.
fn refusal(status: StatusCode) -> ErrorResponse {
    let mut refusal = ErrorResponse::new(None);
    *refusal.status_mut() = status;
    refusal
}

/// Answers an upgrade request: the session path, and room for one more.
struct Vet<'a>(&'a Shared);

impl Callback for Vet<'_> {
    fn on_request(self, request: &Request, response: Response) -> Result<Response, ErrorResponse> {
        if request.uri().path() != SESSION_PATH {
            return Err(refusal(StatusCode::NOT_FOUND));
        }
        if lock(&self.0.peers).len() >= MAX_SESSIONS {
            return Err(refusal(StatusCode::TOO_MANY_REQUESTS));
        }
        Ok(response)
    }
}

/// The most a message may be: the wire's budget, not tungstenite's default.
fn socket_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_MESSAGE_BYTES))
}

/// One session from upgrade to departure.
async fn serve(
    shared: Arc<Shared>,
    inbox: mpsc::Sender<Inbound>,
    stream: TcpStream,
    peer: SocketAddr,
    mut goodbye: watch::Receiver<bool>,
) {
    // Small frames at tick rate: Nagle would batch them into latency.
    if stream.set_nodelay(true).is_err() {
        return;
    }
    let upgrade = accept_hdr_async_with_config(stream, Vet(&shared), Some(socket_config()));
    let Ok(Ok(socket)) = tokio::time::timeout(HANDSHAKE_TIMEOUT, upgrade).await else {
        return;
    };
    let session = shared.next_session.fetch_add(1, Ordering::Relaxed);
    let (reliable, reliable_rx) = mpsc::channel(RELIABLE_FRAMES);
    let (datagrams, datagrams_rx) = mpsc::channel(DATAGRAM_FRAMES);
    let link = PeerLink {
        session,
        reliable,
        datagrams,
    };
    let (mut sink, mut source) = socket.split();
    if !shared.admit(peer, link) {
        say_goodbye(&mut sink, CloseCode::Again, "server full").await;
        return;
    }
    let reason = tokio::select! {
        reason = read_messages(&shared, &inbox, &mut source, PeerId(peer)) => reason,
        reason = write_messages(&mut sink, reliable_rx, datagrams_rx) => reason,
        _ = goodbye.changed() => DepartReason::ServerClosed,
    };
    match reason {
        DepartReason::ServerClosed => {
            say_goodbye(&mut sink, CloseCode::Away, "server closing").await;
        }
        DepartReason::BadFrame(_) | DepartReason::Protocol => {
            say_goodbye(&mut sink, CloseCode::Protocol, "protocol violation").await;
        }
        DepartReason::PeerClosed | DepartReason::TimedOut => {}
    }
    shared.depart(peer, session, reason);
}

type Sink = futures_util::stream::SplitSink<WebSocketStream<TcpStream>, Message>;
type Source = futures_util::stream::SplitStream<WebSocketStream<TcpStream>>;

/// Send a close frame, best effort: the peer may already be gone.
async fn say_goodbye(sink: &mut Sink, code: CloseCode, why: &'static str) {
    let frame = CloseFrame {
        code,
        reason: why.into(),
    };
    let _ = tokio::time::timeout(DRAIN_WAIT, sink.send(Message::Close(Some(frame)))).await;
}

/// Why a WebSocket error ended the session.
fn reason_of(error: &WsError) -> DepartReason {
    match error {
        WsError::ConnectionClosed
        | WsError::AlreadyClosed
        | WsError::Protocol(ProtocolError::ResetWithoutClosingHandshake) => {
            DepartReason::PeerClosed
        }
        WsError::Io(error)
            if matches!(
                error.kind(),
                io::ErrorKind::ConnectionReset
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::BrokenPipe
                    | io::ErrorKind::UnexpectedEof
            ) =>
        {
            DepartReason::PeerClosed
        }
        WsError::Capacity(CapacityError::MessageTooLong { .. }) => {
            DepartReason::BadFrame(NetError::TooLarge)
        }
        _ => DepartReason::Protocol,
    }
}

/// Move every received message into the inbox until the session ends.
async fn read_messages(
    shared: &Shared,
    inbox: &mpsc::Sender<Inbound>,
    source: &mut Source,
    peer: PeerId,
) -> DepartReason {
    loop {
        let next = match tokio::time::timeout(IDLE_TIMEOUT, source.next()).await {
            Err(_) => return DepartReason::TimedOut,
            Ok(next) => next,
        };
        let message = match next {
            None => return DepartReason::PeerClosed,
            Some(Err(error)) => return reason_of(&error),
            Some(Ok(message)) => message,
        };
        let bytes = match message {
            Message::Binary(bytes) => bytes,
            Message::Close(_) => return DepartReason::PeerClosed,
            // Keepalive traffic: tungstenite answers pings itself.
            Message::Ping(_) | Message::Pong(_) => continue,
            // Text and raw frames have no meaning on this wire.
            Message::Text(_) | Message::Frame(_) => {
                return DepartReason::BadFrame(NetError::BadKind(b'T'))
            }
        };
        let (class, payload) = match wire::decode(&bytes) {
            Ok(parsed) => parsed,
            Err(error) => return DepartReason::BadFrame(error),
        };
        match class {
            // Unreliable-class frames never wait: a full inbox sheds.
            Class::Datagram => {
                let delivered = inbox
                    .try_send(Inbound {
                        from: peer,
                        frame: InboundFrame::Datagram(payload.to_vec()),
                    })
                    .is_ok();
                if !delivered {
                    shared.dropped_datagrams.fetch_add(1, Ordering::Relaxed);
                }
            }
            // Reliable frames wait for room: TCP backs up, nothing is lost.
            Class::Reliable(kind) => {
                let frame = Inbound {
                    from: peer,
                    frame: InboundFrame::Stream {
                        kind,
                        bytes: payload.to_vec(),
                    },
                };
                if inbox.send(frame).await.is_err() {
                    return DepartReason::ServerClosed;
                }
            }
        }
    }
}

/// Write queued messages, reliable first, plus a keepalive ping. Returns only
/// when the session is over.
async fn write_messages(
    sink: &mut Sink,
    mut reliable: mpsc::Receiver<Vec<u8>>,
    mut datagrams: mpsc::Receiver<Vec<u8>>,
) -> DepartReason {
    let mut keepalive = tokio::time::interval(QUIC_KEEPALIVE);
    loop {
        let first = tokio::select! {
            biased;
            queued = reliable.recv() => queued,
            queued = datagrams.recv() => queued,
            _ = keepalive.tick() => {
                if let Err(error) = sink.send(Message::Ping(Vec::new().into())).await {
                    return reason_of(&error);
                }
                continue;
            }
        };
        // A closed queue means the link left the table: replaced or shut down.
        let Some(first) = first else {
            return DepartReason::ServerClosed;
        };
        let mut batch = vec![first];
        while batch.len() < WRITE_BATCH {
            match reliable.try_recv().or_else(|_| datagrams.try_recv()) {
                Ok(next) => batch.push(next),
                Err(_) => break,
            }
        }
        for message in batch {
            if let Err(error) = sink.feed(Message::Binary(message.into())).await {
                return reason_of(&error);
            }
        }
        if let Err(error) = sink.flush().await {
            return reason_of(&error);
        }
    }
}
