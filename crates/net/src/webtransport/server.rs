//! server: WebTransport sessions behind the [`Transport`] trait.
//!
//! One dedicated thread runs a single-threaded tokio runtime that accepts
//! sessions and pumps their frames; the tick loop never awaits. Per session:
//!
//! - unreliable frames ride datagrams, sent straight from the caller's thread
//!   and received into one bounded inbox (a full inbox drops the datagram);
//! - reliable frames ride the one bidirectional stream the client opens, in
//!   the [`encode_message`](super::encode_message) framing; a slow tick loop backs
//!   the stream up instead of losing events.
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

use tokio::sync::{mpsc, oneshot};
use wtransport::endpoint::{endpoint_side, IncomingSession};
use wtransport::error::{
    ConnectionError, SendDatagramError, StreamReadError, StreamReadExactError, StreamWriteError,
};
use wtransport::{Connection, Endpoint, RecvStream, SendStream, ServerConfig, VarInt};

use super::cert::PinnedCert;
use super::frame::{decode_header, encode_message, FRAME_HEADER_LEN};
use crate::quic::{
    check_budget, ControlKind, Inbound, InboundFrame, PeerId, Transport, CHANNEL_MEMORY_BYTES,
    QUIC_KEEPALIVE,
};
use crate::session::{NetError, MAX_PACKET_BYTES};

/// URL path a session request must name; anything else is answered 404.
pub const SESSION_PATH: &str = "/e2040";
/// Live sessions the server holds; the next request is answered 429.
pub const MAX_SESSIONS: usize = 16;
/// Frames the inbox holds before datagrams are dropped and streams wait.
const INBOX_FRAMES: usize = 1024;
/// Reliable frames queued per session: the channel memory cap in full frames.
/// A session whose client stops reading refuses the next frame, loudly.
pub const RELIABLE_FRAMES: usize = CHANNEL_MEMORY_BYTES / MAX_PACKET_BYTES;
/// Departures kept for the caller; the oldest drops first.
const MAX_DEPARTURES: usize = 64;
/// How long shutdown waits for sessions to drain after the close frames.
const DRAIN_WAIT: Duration = Duration::from_millis(500);

/// Application close codes the server sends.
const CLOSE_SHUTDOWN: u32 = 0;
const CLOSE_PROTOCOL: u32 = 1;
const CLOSE_REPLACED: u32 = 2;
const CLOSE_FULL: u32 = 3;

/// Why a session ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DepartReason {
    /// The client closed the session (a clean close or an application code).
    PeerClosed,
    /// No packet arrived within the QUIC idle timeout.
    TimedOut,
    /// The server closed it: shutdown, or a newer session took the address.
    ServerClosed,
    /// The connection failed at the QUIC or HTTP/3 layer, or the client reset
    /// its stream.
    Protocol,
    /// The client sent a reliable frame the framing refuses.
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
}

/// What the tick loop needs to reach one session.
struct PeerLink {
    connection: Connection,
    reliable: mpsc::Sender<Vec<u8>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    /// Seat a session. A full table refuses; a session already seated at the
    /// address is closed and replaced.
    fn admit(&self, peer: SocketAddr, link: PeerLink) -> bool {
        let mut peers = lock(&self.peers);
        if !peers.contains_key(&peer) && peers.len() >= MAX_SESSIONS {
            return false;
        }
        if let Some(older) = peers.insert(peer, link) {
            older.connection.close(
                VarInt::from_u32(CLOSE_REPLACED),
                b"replaced by a newer session",
            );
        }
        true
    }

    /// Unseat the session `stable_id` (unless a newer one already took the
    /// address) and record why it ended.
    fn depart(&self, peer: SocketAddr, stable_id: usize, reason: DepartReason) {
        {
            let mut peers = lock(&self.peers);
            if peers
                .get(&peer)
                .is_some_and(|link| link.connection.stable_id() == stable_id)
            {
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

/// The WebTransport server transport. Dropping it closes every session and
/// stops the I/O thread.
pub struct WtServer {
    local_addr: SocketAddr,
    shared: Arc<Shared>,
    inbox: mpsc::Receiver<Inbound>,
    stop: Option<oneshot::Sender<()>>,
    worker: Option<JoinHandle<()>>,
}

impl WtServer {
    /// Bind a UDP socket at `addr` and start accepting sessions presenting
    /// `cert`. Port 0 picks a free port; read it back with [`Self::local_addr`].
    pub fn bind(addr: SocketAddr, cert: &PinnedCert) -> io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let config = ServerConfig::builder()
            .with_bind_address(addr)
            .with_identity(cert.identity().clone_identity())
            .keep_alive_interval(Some(QUIC_KEEPALIVE))
            .build();
        // The endpoint registers its socket with the runtime it is made in.
        let endpoint = {
            let _entered = runtime.enter();
            Endpoint::server(config)?
        };
        let local_addr = endpoint.local_addr()?;
        let shared = Arc::new(Shared {
            peers: Mutex::new(BTreeMap::new()),
            departures: Mutex::new(VecDeque::new()),
            dropped_datagrams: AtomicU64::new(0),
        });
        let (inbox_tx, inbox) = mpsc::channel(INBOX_FRAMES);
        let (stop, stop_rx) = oneshot::channel();
        let worker = thread::Builder::new().name("wt-server".into()).spawn({
            let shared = Arc::clone(&shared);
            move || runtime.block_on(accept_loop(endpoint, shared, inbox_tx, stop_rx))
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

    /// Datagrams received and dropped: over the budget, or the inbox was full.
    #[must_use]
    pub fn dropped_datagrams(&self) -> u64 {
        self.shared.dropped_datagrams.load(Ordering::Relaxed)
    }
}

impl Transport for WtServer {
    fn send_unreliable(&mut self, to: PeerId, bytes: &[u8]) -> Result<(), NetError> {
        check_budget(bytes)?;
        let peers = lock(&self.shared.peers);
        let link = peers.get(&to.0).ok_or(NetError::BadInput)?;
        link.connection
            .send_datagram(bytes)
            .map_err(|error| match error {
                SendDatagramError::TooLarge => NetError::TooLarge,
                SendDatagramError::NotConnected | SendDatagramError::UnsupportedByPeer => {
                    NetError::BadInput
                }
            })
    }

    fn send_reliable(
        &mut self,
        to: PeerId,
        kind: ControlKind,
        bytes: &[u8],
    ) -> Result<(), NetError> {
        let frame = encode_message(kind, bytes)?;
        let peers = lock(&self.shared.peers);
        let link = peers.get(&to.0).ok_or(NetError::BadInput)?;
        // A full queue means the client stopped reading its stream: refuse
        // loudly instead of growing without bound.
        link.reliable
            .try_send(frame)
            .map_err(|_| NetError::BadInput)
    }

    fn recv(&mut self) -> Vec<Inbound> {
        let mut out = Vec::new();
        // Bound one drain so a burst never starves the caller.
        for _ in 0..INBOX_FRAMES {
            match self.inbox.try_recv() {
                Ok(frame) => out.push(frame),
                Err(_) => break,
            }
        }
        out
    }
}

impl Drop for WtServer {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            // An error means the loop already ended on its own.
            let _ = stop.send(());
        }
        if let Some(worker) = self.worker.take() {
            // A panicked worker has nothing left to clean up here.
            let _ = worker.join();
        }
    }
}

async fn accept_loop(
    endpoint: Endpoint<endpoint_side::Server>,
    shared: Arc<Shared>,
    inbox: mpsc::Sender<Inbound>,
    mut stop: oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            _ = &mut stop => break,
            incoming = endpoint.accept() => {
                tokio::spawn(serve(Arc::clone(&shared), inbox.clone(), incoming));
            }
        }
    }
    endpoint.close(VarInt::from_u32(CLOSE_SHUTDOWN), b"server shutdown");
    // Waiting is best effort: the close frames are already queued.
    let _ = tokio::time::timeout(DRAIN_WAIT, endpoint.wait_idle()).await;
}

/// One session from request to departure.
async fn serve(shared: Arc<Shared>, inbox: mpsc::Sender<Inbound>, incoming: IncomingSession) {
    let Ok(request) = incoming.await else {
        return;
    };
    if request.path() != SESSION_PATH {
        request.not_found().await;
        return;
    }
    if lock(&shared.peers).len() >= MAX_SESSIONS {
        request.too_many_requests().await;
        return;
    }
    let Ok(connection) = request.accept().await else {
        return;
    };
    let peer = connection.remote_address();
    let (reliable, outbound) = mpsc::channel(RELIABLE_FRAMES);
    let link = PeerLink {
        connection: connection.clone(),
        reliable,
    };
    if !shared.admit(peer, link) {
        connection.close(VarInt::from_u32(CLOSE_FULL), b"server full");
        return;
    }
    let reason = tokio::select! {
        reason = pump_datagrams(&shared, &inbox, &connection, PeerId(peer)) => reason,
        reason = pump_stream(&inbox, &connection, PeerId(peer), outbound) => reason,
    };
    if matches!(reason, DepartReason::BadFrame(_) | DepartReason::Protocol) {
        connection.close(VarInt::from_u32(CLOSE_PROTOCOL), b"protocol violation");
    }
    shared.depart(peer, connection.stable_id(), reason);
}

/// Why a connection error ended the session.
fn reason_of(error: &ConnectionError) -> DepartReason {
    match error {
        ConnectionError::ConnectionClosed(_) | ConnectionError::ApplicationClosed(_) => {
            DepartReason::PeerClosed
        }
        ConnectionError::TimedOut => DepartReason::TimedOut,
        ConnectionError::LocallyClosed => DepartReason::ServerClosed,
        ConnectionError::LocalH3Error(_)
        | ConnectionError::QuicProto(_)
        | ConnectionError::CidsExhausted => DepartReason::Protocol,
    }
}

/// Never resolves: used by a pump whose end says nothing about the session,
/// so the other pump (or the connection) decides the departure.
async fn never() -> DepartReason {
    std::future::pending().await
}

/// Move every received datagram into the inbox until the connection ends.
async fn pump_datagrams(
    shared: &Shared,
    inbox: &mpsc::Sender<Inbound>,
    connection: &Connection,
    peer: PeerId,
) -> DepartReason {
    loop {
        let datagram = match connection.receive_datagram().await {
            Ok(datagram) => datagram,
            Err(error) => return reason_of(&error),
        };
        let payload = datagram.payload();
        // Unreliable frames never wait: over the budget or into a full inbox
        // they are dropped and counted, like loss.
        let delivered = payload.len() <= MAX_PACKET_BYTES
            && inbox
                .try_send(Inbound {
                    from: peer,
                    frame: InboundFrame::Datagram(payload.to_vec()),
                })
                .is_ok();
        if !delivered {
            shared.dropped_datagrams.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Serve the session's one bidirectional stream: reliable frames in, queued
/// reliable frames out. Only a fault ends it.
async fn pump_stream(
    inbox: &mpsc::Sender<Inbound>,
    connection: &Connection,
    peer: PeerId,
    mut outbound: mpsc::Receiver<Vec<u8>>,
) -> DepartReason {
    let (mut send, mut recv) = match connection.accept_bi().await {
        Ok(pair) => pair,
        Err(error) => return reason_of(&error),
    };
    tokio::select! {
        reason = read_frames(inbox, &mut recv, peer) => reason,
        reason = write_frames(&mut send, &mut outbound) => reason,
    }
}

async fn read_frames(
    inbox: &mpsc::Sender<Inbound>,
    recv: &mut RecvStream,
    peer: PeerId,
) -> DepartReason {
    loop {
        let mut header = [0u8; FRAME_HEADER_LEN];
        match recv.read_exact(&mut header).await {
            Ok(()) => {}
            // The client finished its sending side: no more reliable frames
            // arrive, the session lives on.
            Err(StreamReadExactError::FinishedEarly(0)) => return never().await,
            Err(StreamReadExactError::FinishedEarly(_)) => {
                return DepartReason::BadFrame(NetError::Truncated)
            }
            Err(StreamReadExactError::Read(StreamReadError::NotConnected)) => return never().await,
            Err(StreamReadExactError::Read(_)) => return DepartReason::Protocol,
        }
        let (kind, length) = match decode_header(header) {
            Ok(parsed) => parsed,
            Err(error) => return DepartReason::BadFrame(error),
        };
        let mut payload = vec![0u8; length];
        if recv.read_exact(&mut payload).await.is_err() {
            return DepartReason::BadFrame(NetError::Truncated);
        }
        // Reliable frames wait for room: the stream backs up, nothing is lost.
        let frame = Inbound {
            from: peer,
            frame: InboundFrame::Stream {
                kind,
                bytes: payload,
            },
        };
        if inbox.send(frame).await.is_err() {
            return DepartReason::ServerClosed;
        }
    }
}

async fn write_frames(
    send: &mut SendStream,
    outbound: &mut mpsc::Receiver<Vec<u8>>,
) -> DepartReason {
    while let Some(frame) = outbound.recv().await {
        match send.write_all(&frame).await {
            Ok(()) => {}
            Err(StreamWriteError::NotConnected) => return never().await,
            Err(_) => return DepartReason::Protocol,
        }
    }
    // The queue closed because the link left the table: replaced or shut down.
    DepartReason::ServerClosed
}
