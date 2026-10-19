//! websocket: the fallback leg of browser play, over TCP.
//!
//! Purpose: a browser whose network drops UDP (no WebTransport) still joins
//! the dedicated server over a WebSocket. The transport trait, the snapshot
//! codec, rollback and heal stay as they are: every message is one binary
//! WebSocket message, a one-byte class tag then the exact payload bytes the
//! codec produced, so a snapshot arrives byte-identical to the QUIC,
//! WebTransport and in-memory paths.
//!
//! Layers: [`wire`] is the message layout both ends share (it compiles for the
//! browser); `server` is the native transport on its own I/O thread, so the
//! tick loop only ever calls the synchronous trait methods. The browser client
//! is `browser::ws`.
//!
//! Delivery: WebSocket is one ordered reliable byte stream. Datagram-class
//! frames therefore ride it with head-of-line blocking (the documented
//! downgrade of the UDP path). The send side keeps one datagram slot: while
//! the writer is stalled a newer frame replaces the waiting one and the skip
//! is counted ([`LatestOnly`]), so a resume writes the newest state, never a
//! backlog, and the input redundancy window recovers the skipped ticks as
//! with loss.
//!
//! TLS: the server speaks plain `ws`. `wss` on 443 is terminated in front of
//! it (a reverse proxy), the same place a TLS-inspecting proxy sits.
//!
//! Units: bytes on the wire. Errors: [`NetError`](crate::session::NetError)
//! on the trait, std io for socket setup.

pub mod wire;

#[cfg(all(feature = "websocket", not(target_arch = "wasm32")))]
mod server;

#[cfg(all(test, feature = "websocket", not(target_arch = "wasm32")))]
mod tests;

#[cfg(all(feature = "websocket", not(target_arch = "wasm32")))]
pub use server::{
    DepartReason, Departure, LatestOnly, WsServer, IDLE_TIMEOUT, MAX_SESSIONS, RELIABLE_FRAMES,
    SESSION_PATH,
};
