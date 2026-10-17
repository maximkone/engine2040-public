//! browser: the web client side of the dedicated server's two browser legs.
//!
//! Purpose: a page joins the native server over WebTransport (datagrams plus
//! one reliable stream) or, when UDP is blocked, over a WebSocket, behind the
//! same [`Transport`](crate::quic::Transport) trait and the same payload
//! bytes the native transports carry. [`connect_race`](crate::connect_race)
//! starts both and keeps the first.
//!
//! Layers: `inbox` and `stream` are plain logic (they run and are tested
//! natively); `ws` and `wt` are the wasm32 glue over the page's own
//! `WebSocket` and `WebTransport`. `wt` declares the WebTransport bindings
//! itself: web-sys gates them behind an unstable cfg that would change every
//! build in the workspace.
//!
//! The browser has no sockets and no threads: everything is callback driven
//! on the page's one thread, so shared state is `Rc<RefCell<..>>`.
//!
//! Errors: [`LegError`](crate::connect_race::LegError) at connect,
//! [`NetError`](crate::session::NetError) on the trait, a closed-state reason
//! string once a session ends.

pub mod inbox;
pub mod stream;

#[cfg(target_arch = "wasm32")]
pub mod ws;
#[cfg(target_arch = "wasm32")]
pub mod wt;

/// What a browser client knows about its session.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ClientState {
    /// The handshake has not finished.
    Connecting,
    /// Frames flow.
    Open,
    /// The session ended; the reason names who ended it and how.
    Closed(String),
}
