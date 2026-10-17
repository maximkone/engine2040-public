//! retry: bounded retries and one shared rate-limit gate per provider.
//!
//! One HTTP exchange goes through [`post_json_retrying`]. A refused request
//! (400, 404, 422) never retries. A server fault (5xx, 408) or a socket error
//! retries at most [`MAX_RETRIES`] times after a capped, seeded jitter pause,
//! and only while the pause fits inside the request's own deadline. A 429
//! with a `Retry-After` header closes the provider's [`RateGate`] for that
//! long and the request retries once behind it; every caller of the same
//! provider waits out the one window instead of each opening its own.
//!
//! Determinism: the pause is a pure function of the request seed and the
//! attempt number, so one request replays the same schedule. The gate and
//! the sleeps read the wall clock; that is presentation time, never hashed.
//!
//! Units: milliseconds. Errors: [`TransportError`] as `post_json` reports.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use engine_core::Fnv64;

use super::transport::{post_json, Endpoint, TransportError, TransportResponse};

/// Most retries of one request after its first send.
pub const MAX_RETRIES: u32 = 2;
/// First retry pause ceiling, ms; doubles per retry.
const BACKOFF_BASE_MS: u64 = 100;
/// Longest retry pause ceiling, ms.
const BACKOFF_CAP_MS: u64 = 2_000;

/// The window a provider's 429 closed: one per provider, shared by every
/// caller, so a burst of callers waits once.
#[derive(Debug, Default)]
pub struct RateGate {
    state: Mutex<GateState>,
}

#[derive(Debug, Default)]
struct GateState {
    until: Option<Instant>,
    windows: u64,
}

impl RateGate {
    /// Close the gate for `hold` from `now`. A window that is already open
    /// stays as it is (a late 429 from a send made before it joins it), and
    /// the call says whether it opened a new one.
    pub fn hold(&self, now: Instant, hold: Duration) -> bool {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.until.is_some_and(|until| until > now) {
            return false;
        }
        state.until = Some(now + hold);
        state.windows += 1;
        true
    }

    /// Sleep until the open window ends. False, with no sleep, when it would
    /// end after `deadline`.
    pub fn wait(&self, deadline: Instant) -> bool {
        loop {
            let until = self
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .until;
            let now = Instant::now();
            match until {
                Some(until) if until > now => {
                    if until > deadline {
                        return false;
                    }
                    std::thread::sleep(until - now);
                }
                _ => return true,
            }
        }
    }

    /// Windows opened so far.
    pub fn windows(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .windows
    }
}

/// The one gate of the provider at `endpoint` (host and port), shared by
/// every adapter and thread in the process.
#[must_use]
pub fn gate_for(endpoint: &Endpoint) -> Arc<RateGate> {
    static GATES: OnceLock<Mutex<BTreeMap<String, Arc<RateGate>>>> = OnceLock::new();
    let key = format!("{}:{}", endpoint.host(), endpoint.port());
    let mut gates = GATES
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    Arc::clone(gates.entry(key).or_default())
}

/// Pause before retry number `retry` (0 is the first), in ms: a seeded draw
/// from the upper half of `min(BACKOFF_CAP_MS, BACKOFF_BASE_MS << retry)`.
#[must_use]
pub fn backoff_ms(seed: u64, retry: u32) -> u64 {
    let ceiling = (BACKOFF_BASE_MS << retry.min(8)).min(BACKOFF_CAP_MS);
    let mut hash = Fnv64::new();
    hash.write_u64(seed);
    hash.write_u32(retry);
    ceiling / 2 + hash.finish() % (ceiling / 2 + 1)
}

fn ms_left(deadline: Instant) -> u64 {
    let left = deadline.saturating_duration_since(Instant::now());
    u64::try_from(left.as_millis()).unwrap_or(u64::MAX)
}

/// A pause worth retrying after: a server fault or a socket error. A refused
/// request, a rate limit, a redirect, our own timeout and a TLS failure are
/// not.
fn transient(sent: &Result<TransportResponse, TransportError>) -> bool {
    match sent {
        Ok(response) => response.status == 408 || (500..=599).contains(&response.status),
        Err(error) => matches!(
            error,
            TransportError::Connect
                | TransportError::Write
                | TransportError::Read
                | TransportError::Protocol
        ),
    }
}

/// `post_json` with the retry rules above, inside `budget_ms` in total.
/// `seed` (the request id) makes the pause schedule repeatable. The last
/// response or error comes back when the rules stop retrying.
pub fn post_json_retrying(
    endpoint: &Endpoint,
    body: &str,
    bearer: Option<&str>,
    budget_ms: u64,
    seed: u64,
    gate: &RateGate,
) -> Result<TransportResponse, TransportError> {
    let deadline = Instant::now() + Duration::from_millis(budget_ms);
    let (mut retries, mut rate_waited) = (0u32, false);
    loop {
        if !gate.wait(deadline) {
            return Err(TransportError::Timeout);
        }
        let left = ms_left(deadline);
        if left == 0 {
            return Err(TransportError::Timeout);
        }
        let sent = post_json(endpoint, body, bearer, left);
        if let Ok(response) = &sent {
            if response.status == 429 && !rate_waited {
                match response.retry_after_ms {
                    Some(hold) if hold < ms_left(deadline) => {
                        rate_waited = true;
                        gate.hold(Instant::now(), Duration::from_millis(hold));
                        continue;
                    }
                    _ => return sent,
                }
            }
        }
        if !transient(&sent) || retries >= MAX_RETRIES {
            return sent;
        }
        let pause = backoff_ms(seed, retries);
        if pause >= ms_left(deadline) {
            return sent;
        }
        retries += 1;
        std::thread::sleep(Duration::from_millis(pause));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stub::StubServer;
    use crate::transport::{map_status, parse_endpoint};
    use crate::AiError;

    fn endpoint(stub: &StubServer) -> Endpoint {
        parse_endpoint(&stub.url()).expect("stub url parses")
    }

    fn send(stub: &StubServer, budget_ms: u64) -> Result<TransportResponse, TransportError> {
        post_json_retrying(
            &endpoint(stub),
            "{}",
            None,
            budget_ms,
            7,
            &RateGate::default(),
        )
    }

    /// F2P: `map_status` sent every non-200 to a retryable error, 400 included.
    #[test]
    fn status_map_names_what_may_retry() {
        for refused in [400, 404, 422] {
            let error = map_status(refused);
            assert_eq!(error, AiError::UnsupportedCapability, "{refused}");
            assert!(!error.retryable(), "{refused} is final");
        }
        for fault in [500, 502, 503] {
            assert!(map_status(fault).retryable(), "{fault}");
        }
        assert_eq!(map_status(408), AiError::Timeout);
        assert_eq!(map_status(429), AiError::QuotaAuth);
    }

    #[test]
    fn status_map_a_refused_request_gets_zero_retries() {
        for refused in [400, 404, 422] {
            let stub = StubServer::spawn_raw(refused, String::new());
            let response = send(&stub, 5_000).expect("answered");
            assert_eq!(response.status, refused);
            assert_eq!(stub.hits(), 1, "{refused} is never retried");
        }
    }

    #[test]
    fn status_map_503_twice_then_200_retries_inside_the_deadline() {
        let stub = StubServer::spawn_script(vec![
            (503, String::new()),
            (503, String::new()),
            (200, String::from("{}")),
        ]);
        let started = Instant::now();
        let response = send(&stub, 5_000).expect("answered");
        assert_eq!(response.status, 200);
        assert_eq!(stub.hits(), 3, "two retries");
        assert!(started.elapsed() < Duration::from_millis(5_000));
    }

    #[test]
    fn status_map_retries_stop_at_the_cap_and_at_the_deadline() {
        let stub = StubServer::spawn_raw(503, String::new());
        let response = send(&stub, 5_000).expect("answered");
        assert_eq!(response.status, 503, "the last fault comes back");
        assert_eq!(u32::try_from(stub.hits()).ok(), Some(1 + MAX_RETRIES));

        // A budget the first pause does not fit in: one send, no sleep past it.
        let tight = StubServer::spawn_raw(503, String::new());
        let started = Instant::now();
        let response = send(&tight, 20).expect("answered");
        assert_eq!(response.status, 503);
        assert_eq!(tight.hits(), 1);
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn status_map_a_dead_socket_retries_then_reports_it() {
        let dead = parse_endpoint("http://127.0.0.1:0/v1/chat/completions").expect("url");
        let started = Instant::now();
        let sent = post_json_retrying(&dead, "{}", None, 5_000, 7, &RateGate::default());
        assert_eq!(sent, Err(TransportError::Connect));
        let floor = backoff_ms(7, 0) + backoff_ms(7, 1);
        assert!(
            started.elapsed() >= Duration::from_millis(floor),
            "both retry pauses ran"
        );
    }

    #[test]
    fn status_map_backoff_is_seeded_capped_and_in_its_band() {
        for retry in 0..12 {
            let ceiling = (BACKOFF_BASE_MS << retry.min(8)).min(BACKOFF_CAP_MS);
            for seed in 0..32 {
                let pause = backoff_ms(seed, retry);
                assert!(pause >= ceiling / 2 && pause <= ceiling, "{seed} {retry}");
                assert_eq!(pause, backoff_ms(seed, retry), "same inputs, same pause");
            }
        }
        let draws: std::collections::BTreeSet<u64> =
            (0..32).map(|seed| backoff_ms(seed, 1)).collect();
        assert!(draws.len() > 1, "the seed moves the pause");
    }

    /// Twenty callers, three of them answered 429 with `Retry-After: 1`: the
    /// provider's one gate opens one window, everyone waits it once, and the
    /// three retry behind it.
    #[test]
    fn status_map_429_with_retry_after_waits_once_behind_one_gate() {
        let stub = StubServer::spawn_script_headers(
            vec![
                (429, String::new()),
                (429, String::new()),
                (429, String::new()),
                (200, String::from("{}")),
            ],
            "Retry-After: 1\r\n",
        );
        let gate = Arc::new(RateGate::default());
        let endpoint = endpoint(&stub);
        let started = Instant::now();
        let callers: Vec<_> = (0..20u64)
            .map(|seed| {
                let (gate, endpoint) = (Arc::clone(&gate), endpoint.clone());
                std::thread::spawn(move || {
                    post_json_retrying(&endpoint, "{}", None, 10_000, seed, &gate)
                })
            })
            .collect();
        for caller in callers {
            let response = caller.join().expect("caller joins").expect("answered");
            assert_eq!(response.status, 200);
        }
        assert_eq!(gate.windows(), 1, "one shared wait");
        assert_eq!(stub.hits(), 23, "20 first sends plus 3 retries");
        let took = started.elapsed();
        assert!(
            took >= Duration::from_millis(1_000),
            "the window was waited"
        );
        assert!(
            took < Duration::from_millis(2_500),
            "waited once, not per caller"
        );
    }

    #[test]
    fn status_map_429_waits_once_and_a_second_429_is_final() {
        let stub =
            StubServer::spawn_script_headers(vec![(429, String::new())], "Retry-After: 1\r\n");
        let response = send(&stub, 10_000).expect("answered");
        assert_eq!(response.status, 429);
        assert_eq!(stub.hits(), 2, "one wait, one retry");

        // No Retry-After, or one past the deadline: no wait at all.
        let bare = StubServer::spawn_raw(429, String::new());
        assert_eq!(send(&bare, 10_000).expect("answered").status, 429);
        assert_eq!(bare.hits(), 1);
        let far =
            StubServer::spawn_script_headers(vec![(429, String::new())], "Retry-After: 30\r\n");
        assert_eq!(send(&far, 2_000).expect("answered").status, 429);
        assert_eq!(far.hits(), 1);
    }

    #[test]
    fn gates_are_shared_per_provider() {
        let a = parse_endpoint("http://127.0.0.1:41001/x").expect("url");
        let same = parse_endpoint("http://127.0.0.1:41001/y").expect("url");
        let other = parse_endpoint("http://127.0.0.1:41002/x").expect("url");
        assert!(Arc::ptr_eq(&gate_for(&a), &gate_for(&same)));
        assert!(!Arc::ptr_eq(&gate_for(&a), &gate_for(&other)));
    }
}
