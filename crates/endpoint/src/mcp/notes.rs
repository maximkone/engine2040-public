//! D16: progress notes for long calls, split from `mcp.rs` so the door's
//! session core stays readable.
//!
//! A client that sent a progress token gets one note before a long call and
//! one after it. A client that sent no token gets nothing: the notes cost
//! nothing when nobody asked for them.

use rmcp::model::{Notification, ProgressNotificationParam, ProgressToken, ServerNotification};
use rmcp::service::{Peer, RequestContext};
use rmcp::RoleServer;

use super::{TOOL_GAME_BUILD, TOOL_SIM_BUILD_ENCOUNTER};
use crate::build_level::TOOL_BUILD_LEVEL;
use crate::play::{TOOL_CAPTURE, TOOL_PLAY_RUN};

/// D16: the progress notes one call sends.
///
/// The note is a notification, never a second answer: the call still answers
/// once, with its own result.
pub(super) struct ProgressNote {
    peer: Peer<RoleServer>,
    token: ProgressToken,
}

impl ProgressNote {
    /// The note for this call, when the client asked for one and the call
    /// is one of the long ones.
    pub(super) fn for_call(name: &str, context: &RequestContext<RoleServer>) -> Option<Self> {
        if !is_long_call(name) {
            return None;
        }
        let token = context.meta.get_progress_token()?;
        Some(Self {
            peer: context.peer.clone(),
            token,
        })
    }

    /// Send one note.
    pub(super) async fn send(&self, progress: f64, message: &str) {
        let param = progress_note_params(&self.token, progress, message);
        let _ = self
            .peer
            .send_notification(ServerNotification::ProgressNotification(Notification::new(
                param,
            )))
            .await;
    }
}

/// D16: the payload of one progress note.
///
/// The builder is pure: the same token, progress and message always make the
/// same payload, so a test can read what the client would have received
/// without a transport.
pub(crate) fn progress_note_params(
    token: &ProgressToken,
    progress: f64,
    message: &str,
) -> ProgressNotificationParam {
    ProgressNotificationParam::new(token.clone(), progress).with_message(message.to_owned())
}

/// The calls that are worth a progress note: the builds and the long runs.
///
/// D16: a note costs the client one message, so it is worth it only where
/// the call can take more than a heartbeat.
pub(crate) fn is_long_call(name: &str) -> bool {
    matches!(
        name,
        TOOL_BUILD_LEVEL
            | TOOL_GAME_BUILD
            | TOOL_SIM_BUILD_ENCOUNTER
            | TOOL_PLAY_RUN
            | TOOL_CAPTURE
            | "play.step_until"
            | "build.web"
            | "serve.web"
            | "browser.capture"
            | "sim.step"
    )
}
