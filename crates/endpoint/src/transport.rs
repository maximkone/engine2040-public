//! transport: loopback-only + token + caps + timeout tiers. std-only.
//!
//! Admission kit for a remote transport, not for the live door: the door is
//! rmcp stdio (one local child), so it has no host to check, no
//! presented-token channel in a tool call and no deadline to own; wiring
//! these gates into it would invent its contract. The one member that runs
//! live is [`MAX_CHARS`], as the sandbox manifest's per-tool payload cap
//! (`door_refuses_over_cap_and_admits_under_cap` pins it); the rest run in
//! the redteam fixtures and the `session_chaos` remote shim.

/// A single tool-call frame. Shape-only: no socket, no JSON yet.
pub struct Frame {
    pub method: String,
    pub name: String,
    pub id: u64,
}

impl Frame {
    /// Build one tool-call frame: method word, tool name and numeric id.
    /// Units are chars of method and name. No errors: shape checks run in
    /// `validate`. Budget rides on the 8 MB payload cap.
    pub fn new(method: &str, name: &str, id: u64) -> Self {
        Self {
            method: method.into(),
            name: name.into(),
            id,
        }
    }
}

/// Shape check: known method, non-empty name. Transport auth deferred.
pub fn validate(f: &Frame) -> bool {
    matches!(
        f.method.as_str(),
        "tools.call" | "tools.list" | "tools.search"
    ) && !f.name.is_empty()
}

/// Loopback hosts only. Non-loopback needs explicit token, never by default.
pub fn is_loopback(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "::1" | "localhost")
}

/// Token intersect: presented must equal expected, expected never empty.
pub fn token_ok(presented: &str, expected: &str) -> bool {
    !expected.is_empty() && presented == expected
}

/// Payload caps: 8MB bytes + 30k chars (ENDPOINT transport).
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
/// Payload caps: 30k chars.
pub const MAX_CHARS: usize = 30_000;

/// True when both caps hold.
pub fn check_caps(nbytes: usize, nchars: usize) -> bool {
    nbytes <= MAX_BYTES && nchars <= MAX_CHARS
}

/// Single timeout table (seconds): search 5 / read 7 / patch 10 / batch 29 / proof 30.
pub fn timeout_for(kind: &str) -> u64 {
    match kind {
        "search" => 5,
        "read" => 7,
        "patch" => 10,
        "batch" => 29,
        "proof" => 30,
        _ => 10,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_only() {
        assert!(is_loopback("127.0.0.1"));
        assert!(is_loopback("::1"));
        assert!(is_loopback("localhost"));
        assert!(!is_loopback("192.168.1.2"));
        assert!(!is_loopback(""));
    }

    #[test]
    fn token_intersect() {
        assert!(token_ok("s3cret", "s3cret"));
        assert!(!token_ok("wrong", "s3cret"));
        assert!(!token_ok("", "s3cret"));
        assert!(!token_ok("s3cret", ""));
    }

    #[test]
    fn caps_and_tiers() {
        assert!(check_caps(0, 0));
        assert!(check_caps(MAX_BYTES, MAX_CHARS));
        assert!(!check_caps(MAX_BYTES + 1, 0));
        assert!(!check_caps(0, MAX_CHARS + 1));
        assert_eq!(timeout_for("search"), 5);
        assert_eq!(timeout_for("read"), 7);
        assert_eq!(timeout_for("patch"), 10);
        assert_eq!(timeout_for("batch"), 29);
        assert_eq!(timeout_for("proof"), 30);
    }

    /// The one transport gate that runs on the live door: the payload cap
    /// arrives as the sandbox manifest's per-tool cap, so a frame past
    /// [`MAX_CHARS`] refuses `over_cap` through the audited seam (one
    /// `Refused` record, nothing written) while a frame under the cap still
    /// answers. The host, token and deadline gates stay off the stdio door
    /// for the reason in the module doc above.
    #[cfg(feature = "mcp")]
    #[test]
    fn door_refuses_over_cap_and_admits_under_cap() {
        use crate::audit::{code_hash, AuditOutcome};
        use crate::insight::mcp_tools::{ApiSearchArgs, ApiSearchTool};
        use crate::mcp::McpServer;
        use engine_editor::capability::Capability;
        use rmcp::handler::server::router::tool::SyncTool;

        let server = McpServer::new(7, Capability::unrestricted(7));
        let refused = ApiSearchTool::invoke(
            &server,
            ApiSearchArgs {
                query: "x".repeat(MAX_CHARS + 1),
                limit: None,
            },
        )
        .expect_err("a payload past the transport cap must refuse at the door");
        assert_eq!(refused.code, "over_cap");
        assert!(
            refused.message.contains(&MAX_CHARS.to_string()),
            "refusal names the cap: {}",
            refused.message
        );
        let log = server.audit_log().expect("audit reads");
        assert_eq!(log.len(), 1, "one refused call chains one record");
        assert_eq!(log.total_appended(), 1);
        let record = log.iter().next().expect("one record");
        assert_eq!(record.tool, "api.search");
        assert_eq!(record.outcome, AuditOutcome::Refused);
        assert_eq!(record.detail, code_hash("over_cap"));
        // The same gate admits a frame under the cap: still dispatched, one
        // more record, and a read writes no revision.
        let found = ApiSearchTool::invoke(
            &server,
            ApiSearchArgs {
                query: "quaternion".to_owned(),
                limit: None,
            },
        )
        .expect("a frame under the cap still dispatches");
        assert!(found.hits.is_empty());
        let log = server.audit_log().expect("audit reads");
        assert_eq!(log.len(), 2);
        assert_eq!(
            log.iter().nth(1).expect("second record").outcome,
            AuditOutcome::Ok
        );
    }
}
