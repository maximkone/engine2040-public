//! sandbox manifest: which door tools take untrusted text, and how much.
//!
//! Consumer: the dispatch seam [`crate::door_dispatch::McpServer::dispatch_screened`],
//! which hands each call's untrusted text to [`super::check_before_run`];
//! that gate looks up the cap here before the static scan runs, so an
//! unknown tool refuses without ever scanning. The seam refuses a listed
//! name through plain `dispatch` as `unscreened_call`, so the manifest and
//! the routing cannot drift: every name here is screened on every call. The
//! data owner stays the frozen tool registry (`super::super::tools`); this
//! module owns only the scan surface: one payload cap per known tool. Units:
//! chars of untrusted payload text. Determinism scope: the same tool name
//! always maps to the same cap on every machine; integer compares only, no
//! clock, no float. Errors: none; unknown tools map to `None`, never to a
//! default cap.

use super::super::tools::digest_len;
use engine_core::Fnv64;

use super::super::transport::MAX_CHARS;

/// Every door tool whose args may carry untrusted text, in registry order.
/// It mirrors the frozen registry names (`tools::TOOLS`) read-only: the
/// registry test below refuses a name that misses here, and the seam's
/// `unscreened_call` guard refuses a listed name that skips the screen.
const SCANNED_TOOLS: &[&str] = &[
    "scene.read",
    "scene.brief",
    "scene.spatial",
    "tx.preview",
    "tx.apply",
    "tx.undo",
    "scene.place",
    "level.build",
    "scene.query",
    "sim.plan_goals",
    "play.run",
    "play.step",
    "capture",
    "log.digest",
    "log.query",
    "api.search",
    "api.describe",
    "api.call",
    "api.load",
    "code.callers",
    "asset.refs",
    "rpc.discover",
    "scene.watch",
    "test.run",
    "sim.build_encounter",
    "game.build",
];

/// True when the tool takes untrusted text and needs the pre-run scan.
/// Unknown names return false, so they refuse at the gate.
pub fn scans_payload(tool: &str) -> bool {
    SCANNED_TOOLS.contains(&tool)
}

/// The screened tool names in registry order. Consumer: the routing test,
/// which loops this list instead of a hand-written copy.
#[cfg(test)]
pub(crate) fn scanned_tools() -> &'static [&'static str] {
    SCANNED_TOOLS
}

/// Max untrusted payload chars one pre-run scan walks for this tool.
/// `None` for unknown tools: no cap means no run, fail closed.
pub fn max_payload_chars(tool: &str) -> Option<usize> {
    if scans_payload(tool) {
        Some(MAX_CHARS)
    } else {
        None
    }
}

/// Length prefix for the canonical digest below, in `u32` units (owner: [`crate::tools::digest_len`]).
/// Canonical digest over the manifest: count, then names in order with
/// their caps. The same manifest always seals the same digest, so a tape
/// can pin it and refuse on drift instead of scanning under a new surface.
#[must_use]
pub fn manifest_digest() -> u64 {
    let mut hasher = Fnv64::new();
    hasher.write_u32(digest_len(SCANNED_TOOLS.len()));
    for tool in SCANNED_TOOLS {
        hasher.write_u32(digest_len(tool.len()));
        hasher.write_str(tool);
        hasher.write_u32(digest_len(MAX_CHARS));
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_registry_tool_has_a_manifest_cap() {
        for tool in crate::tools::TOOLS {
            assert!(
                scans_payload(tool.name),
                "registry tool `{}` needs a manifest entry",
                tool.name
            );
            assert_eq!(
                max_payload_chars(tool.name),
                Some(MAX_CHARS),
                "registry tool `{}` needs the transport cap",
                tool.name
            );
        }
    }

    #[test]
    fn unknown_tools_have_no_manifest() {
        for name in [
            "shell.exec",
            "file.write",
            "batch.execute",
            "state.patch",
            "",
        ] {
            assert!(!scans_payload(name), "`{name}` must miss the manifest");
            assert_eq!(max_payload_chars(name), None, "`{name}` gets no cap");
        }
    }

    #[test]
    fn manifest_digest_replays_identically() {
        let first = manifest_digest();
        let second = manifest_digest();
        assert_eq!(first, second, "same manifest twice, same digest");
        assert_ne!(first, 0, "the digest seals something");
    }
}
