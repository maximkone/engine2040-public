//! Live attach proof: pid + socket + fresh file + HTTP + ready.
//!
//! The windowed game proves a live backend before it presents. The proof
//! carries the OS pid, the attached socket name, the fresh capture file
//! plus its hash, the HTTP health answer and the ready flag. A proof
//! lives 60 seconds and needs a heartbeat every 2 seconds. Expired or
//! incomplete proofs refuse, never present. Leases clamp to the same
//! 60 second ceiling so import and state locks cannot be held forever.

/// Milliseconds a live proof stays valid after issue.
pub const PROOF_TTL_MS: u64 = 60_000;

/// Milliseconds between required heartbeats.
pub const HEARTBEAT_MS: u64 = 2_000;

/// Milliseconds a lease may be held at most.
pub const LEASE_MAX_MS: u64 = 60_000;

/// Milliseconds a lease is held at least.
pub const LEASE_MIN_MS: u64 = 1_000;

/// Why a live proof or lease refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LiveProofError {
    /// OS pid was zero.
    BadPid,
    /// Socket name was empty.
    BadSocket,
    /// Fresh file path was empty.
    BadFile,
    /// File hash was not 64 lower-hex chars.
    BadHash,
    /// HTTP health answer was missing or failed.
    BadHttp,
    /// Ready flag was false.
    NotReady,
    /// Proof is older than the 60 second TTL.
    Expired,
    /// Heartbeat arrived after expiry.
    HeartbeatLate,
}

impl core::fmt::Display for LiveProofError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            Self::BadPid => "pid must be non-zero",
            Self::BadSocket => "socket must be named",
            Self::BadFile => "fresh file must be named",
            Self::BadHash => "file hash must be 64 lower-hex",
            Self::BadHttp => "http health must answer ok",
            Self::NotReady => "ready must be true",
            Self::Expired => "proof older than 60s TTL",
            Self::HeartbeatLate => "heartbeat after expiry",
        };
        f.write_str(s)
    }
}

impl std::error::Error for LiveProofError {}

/// Live backend proof, validated before present.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveProof {
    /// OS pid of the attached backend.
    pub pid: u32,
    /// Attached socket or window name.
    pub socket: String,
    /// Fresh capture file written by this backend.
    pub fresh_file: String,
    /// SHA-256 of the fresh file, 64 lower-hex chars.
    pub file_hash: String,
    /// HTTP /health answered ok.
    pub http_ok: bool,
    /// Backend reports ready.
    pub ready: bool,
    /// Issue time in wall-clock milliseconds.
    pub issued_ms: u64,
    /// Last heartbeat in wall-clock milliseconds.
    pub last_beat_ms: u64,
}

impl LiveProof {
    /// Build a proof; issue and heartbeat start at `now_ms`.
    pub fn of(
        pid: u32,
        socket: &str,
        fresh_file: &str,
        file_hash: &str,
        http_ok: bool,
        ready: bool,
        now_ms: u64,
    ) -> Self {
        Self {
            pid,
            socket: socket.to_owned(),
            fresh_file: fresh_file.to_owned(),
            file_hash: file_hash.to_owned(),
            http_ok,
            ready,
            issued_ms: now_ms,
            last_beat_ms: now_ms,
        }
    }

    /// Check the full chain at `now_ms`.
    pub fn attach(&self, now_ms: u64) -> Result<(), LiveProofError> {
        if self.pid == 0 {
            return Err(LiveProofError::BadPid);
        }
        if self.socket.trim().is_empty() {
            return Err(LiveProofError::BadSocket);
        }
        if self.fresh_file.trim().is_empty() {
            return Err(LiveProofError::BadFile);
        }
        if !is_hash64(&self.file_hash) {
            return Err(LiveProofError::BadHash);
        }
        if !self.http_ok {
            return Err(LiveProofError::BadHttp);
        }
        if !self.ready {
            return Err(LiveProofError::NotReady);
        }
        if now_ms.saturating_sub(self.issued_ms) > PROOF_TTL_MS {
            return Err(LiveProofError::Expired);
        }
        if now_ms.saturating_sub(self.last_beat_ms) > PROOF_TTL_MS {
            return Err(LiveProofError::HeartbeatLate);
        }
        Ok(())
    }

    /// Record a heartbeat at `now_ms`; refuses when expired.
    pub fn heartbeat(&mut self, now_ms: u64) -> Result<(), LiveProofError> {
        self.attach(now_ms)?;
        self.last_beat_ms = now_ms;
        Ok(())
    }

    /// True when the proof still holds at `now_ms`.
    pub fn live_at(&self, now_ms: u64) -> bool {
        self.attach(now_ms).is_ok()
    }
}

/// True for 64 lower-hex chars.
fn is_hash64(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 64 {
        return false;
    }
    for c in b {
        if !c.is_ascii_hexdigit() || c.is_ascii_uppercase() {
            return false;
        }
    }
    true
}

/// Clamp a lease hold to 1s..60s.
pub fn clamp_lease(requested_ms: u64) -> u64 {
    requested_ms.clamp(LEASE_MIN_MS, LEASE_MAX_MS)
}

/// A serial import or state lock hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lease {
    /// Lock name, e.g. import or state.
    pub name: String,
    /// Holder id.
    pub holder: String,
    /// Hold length after clamping.
    pub hold_ms: u64,
}

impl Lease {
    /// Take a lease; the hold clamps to 1s..60s.
    pub fn take(name: &str, holder: &str, requested_ms: u64) -> Self {
        Self {
            name: name.to_owned(),
            holder: holder.to_owned(),
            hold_ms: clamp_lease(requested_ms),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good(now: u64) -> LiveProof {
        LiveProof::of(
            1234,
            "window-0",
            "target/live/frame.png",
            &"a".repeat(64),
            true,
            true,
            now,
        )
    }

    #[test]
    fn attach_passes_on_full_chain() {
        assert_eq!(good(1_000).attach(1_500), Ok(()));
    }

    #[test]
    fn attach_refuses_empty_socket() {
        let mut p = good(0);
        p.socket.clear();
        assert_eq!(p.attach(10), Err(LiveProofError::BadSocket));
    }

    #[test]
    fn attach_refuses_bad_hash() {
        let mut p = good(0);
        p.file_hash = "xyz".to_owned();
        assert_eq!(p.attach(10), Err(LiveProofError::BadHash));
    }

    #[test]
    fn attach_refuses_expiry() {
        assert_eq!(
            good(0).attach(PROOF_TTL_MS + 1),
            Err(LiveProofError::Expired)
        );
    }

    #[test]
    fn heartbeat_moves_last_beat() {
        let mut p = good(0);
        assert_eq!(p.heartbeat(HEARTBEAT_MS), Ok(()));
        assert_eq!(p.last_beat_ms, HEARTBEAT_MS);
    }

    #[test]
    fn lease_clamps_to_ceiling_and_floor() {
        assert_eq!(clamp_lease(0), LEASE_MIN_MS);
        assert_eq!(clamp_lease(9_999_999), LEASE_MAX_MS);
        assert_eq!(Lease::take("import", "w1", 5_000).hold_ms, 5_000);
    }
}
