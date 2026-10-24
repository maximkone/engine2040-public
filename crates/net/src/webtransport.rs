//! webtransport: browser clients join the dedicated server over WebTransport.
//!
//! Purpose: accept WebTransport sessions beside QUIC behind the same
//! [`Transport`](crate::quic::Transport)(crate::quic::Transport) trait, so the snapshot, state and
//! lobby codecs stay exactly as they are. Datagrams carry the unreliable
//! frames (inputs, states, deltas); one bidirectional stream per session,
//! opened by the client, carries the reliable frames (snapshots, lobby).
//!
//! Layers: [`cert`] mints the pinned certificate a browser accepts through
//! `serverCertificateHashes`; `frame` is the stream framing; `server` is the
//! transport. The async I/O lives on one dedicated thread, so the tick loop
//! only ever calls the synchronous trait methods.
//!
//! Units: bytes on the wire, certificate validity in whole seconds. Errors:
//! [`NetError`](crate::session::NetError) on the trait,
//! [`cert::CertError`] for certificates, std io for socket setup.

pub mod cert;
mod frame;
mod server;

#[cfg(test)]
mod tests;

pub use frame::{decode_header, encode_message, FRAME_HEADER_LEN};
pub use server::{DepartReason, Departure, WtServer, MAX_SESSIONS, RELIABLE_FRAMES, SESSION_PATH};
