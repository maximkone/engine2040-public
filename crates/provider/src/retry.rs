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
//! A [`Breaker`] is the slower rule on the same path: it opens after a run of
//! identical failures, refuses every call with `circuit_open` (no send, no
//! retry) while it is open, lets one probe through after its cooldown and
//! closes when that probe succeeds. It reads only the tick its caller passes,
//! so a replay makes the same transitions, and [`GuardedSend`] is the one
//! call site that puts a request behind it.
//!
//! Units: milliseconds, or caller ticks for the breaker. Errors:
//! [`TransportError`] as `post_json` reports, plus [`BreakerError`].

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

/// Consecutive identical failures that open a breaker.
pub const DEFAULT_BREAKER_THRESHOLD: u32 = 3;
/// Ticks a breaker stays open before it lets one probe through.
pub const DEFAULT_BREAKER_COOLDOWN_TICKS: u64 = 30;

/// Why a breaker would not build.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BreakerConfigError {
    /// A threshold of zero would open on no failure at all.
    ZeroThreshold,
    /// A cooldown of zero would probe on every tick.
    ZeroCooldown,
}

impl std::fmt::Display for BreakerConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            BreakerConfigError::ZeroThreshold => "breaker: threshold must be non-zero",
            BreakerConfigError::ZeroCooldown => "breaker: cooldown must be non-zero",
        };
        f.write_str(text)
    }
}

impl std::error::Error for BreakerConfigError {}

/// Phase a breaker is in at one tick.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BreakerState {
    /// Calls pass and failures are counted.
    Closed,
    /// Every call is refused until the cooldown ends.
    Open,
    /// One probe may run; the rest wait for its answer.
    HalfOpen,
}

impl BreakerState {
    /// Fixed label for logs and receipts.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            BreakerState::Closed => "closed",
            BreakerState::Open => "open",
            BreakerState::HalfOpen => "half_open",
        }
    }
}

/// What one finished call reported to the breaker.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BreakerOutcome {
    /// The provider answered: a closed breaker forgets its run of failures.
    Success,
    /// The call failed with `signature` (an HTTP status or a socket kind).
    /// Only the same signature repeats count; a different one starts a run.
    Failure {
        /// Stable number naming the failure kind.
        signature: u64,
    },
}

/// What one call may do under a breaker.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BreakerPermit {
    /// Circuit closed: send normally.
    Allow,
    /// Circuit half open: this is the one probe.
    Probe,
}

/// One call refused by an open circuit: the count of failures that opened it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BreakerRefusal {
    /// Failures that opened the circuit.
    pub failures: u32,
    /// Tick the call was refused at.
    pub tick: u64,
}

impl BreakerRefusal {
    /// Stable machine name, never caller text.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        "circuit_open"
    }
}

impl std::fmt::Display for BreakerRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "circuit_open failures={} tick={}",
            self.failures, self.tick
        )
    }
}

#[derive(Clone, Copy, Debug)]
struct BreakerCore {
    state: BreakerState,
    failures: u32,
    signature: u64,
    opened_at: u64,
    probe_taken: bool,
    trips: u64,
    refusals: u64,
}

/// A three-state failure breaker over one provider: closed while calls
/// answer, open for `cooldown_ticks` after `threshold` identical failures, one
/// probe in half-open, closed again when the probe succeeds.
///
/// Every decision reads the caller's tick, never a clock, so the same call
/// sequence gives the same transitions on a replay.
#[derive(Debug)]
pub struct Breaker {
    threshold: u32,
    cooldown_ticks: u64,
    core: Mutex<BreakerCore>,
}

impl Breaker {
    /// Build a breaker that opens after `threshold` identical consecutive
    /// failures and cools down for `cooldown_ticks`.
    ///
    /// Errors: [`BreakerConfigError`] for a zero threshold or cooldown.
    pub fn new(threshold: u32, cooldown_ticks: u64) -> Result<Self, BreakerConfigError> {
        if threshold == 0 {
            return Err(BreakerConfigError::ZeroThreshold);
        }
        if cooldown_ticks == 0 {
            return Err(BreakerConfigError::ZeroCooldown);
        }
        Ok(Self {
            threshold,
            cooldown_ticks,
            core: Mutex::new(BreakerCore {
                state: BreakerState::Closed,
                failures: 0,
                signature: 0,
                opened_at: 0,
                probe_taken: false,
                trips: 0,
                refusals: 0,
            }),
        })
    }

    /// Failures that open the circuit.
    #[must_use]
    pub const fn threshold(&self) -> u32 {
        self.threshold
    }

    /// Ticks an open circuit stays closed to new calls.
    #[must_use]
    pub const fn cooldown_ticks(&self) -> u64 {
        self.cooldown_ticks
    }

    /// Phase at `tick`, with a cooled-down open circuit reported half open.
    #[must_use]
    pub fn state(&self, tick: u64) -> BreakerState {
        let core = self.core.lock().unwrap_or_else(PoisonError::into_inner);
        cooled(core.state, core.opened_at, tick, self.cooldown_ticks)
    }

    /// Consecutive identical failures counted right now.
    #[must_use]
    pub fn failures(&self) -> u32 {
        self.core
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .failures
    }

    /// Times the circuit opened.
    #[must_use]
    pub fn trips(&self) -> u64 {
        self.core
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .trips
    }

    /// Calls refused while the circuit was open.
    #[must_use]
    pub fn refusals(&self) -> u64 {
        self.core
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .refusals
    }

    /// Ask to make one call at `tick`.
    ///
    /// Errors: [`BreakerRefusal`] with `code()` `circuit_open` while the
    /// circuit is open or its one probe is already out.
    pub fn permit(&self, tick: u64) -> Result<BreakerPermit, BreakerRefusal> {
        let mut core = self.core.lock().unwrap_or_else(PoisonError::into_inner);
        core.state = cooled(core.state, core.opened_at, tick, self.cooldown_ticks);
        match core.state {
            BreakerState::Closed => Ok(BreakerPermit::Allow),
            BreakerState::HalfOpen if !core.probe_taken => {
                core.probe_taken = true;
                Ok(BreakerPermit::Probe)
            }
            _ => {
                core.refusals = core.refusals.saturating_add(1);
                Err(BreakerRefusal {
                    failures: core.failures,
                    tick,
                })
            }
        }
    }

    /// Report how the call permitted at `tick` ended. A report that arrives
    /// while the circuit is open is stale and changes nothing.
    pub fn record(&self, tick: u64, outcome: BreakerOutcome) {
        let mut core = self.core.lock().unwrap_or_else(PoisonError::into_inner);
        core.state = cooled(core.state, core.opened_at, tick, self.cooldown_ticks);
        match (core.state, outcome) {
            (BreakerState::Closed, BreakerOutcome::Success) => {
                core.failures = 0;
                core.signature = 0;
            }
            (BreakerState::Closed, BreakerOutcome::Failure { signature }) => {
                core.failures = match core.failures {
                    0 => 1,
                    count if core.signature == signature => count.saturating_add(1),
                    _ => 1,
                };
                core.signature = signature;
                if core.failures >= self.threshold {
                    core.state = BreakerState::Open;
                    core.opened_at = tick;
                    core.probe_taken = false;
                    core.trips = core.trips.saturating_add(1);
                }
            }
            (BreakerState::HalfOpen, BreakerOutcome::Success) => {
                core.state = BreakerState::Closed;
                core.failures = 0;
                core.signature = 0;
                core.probe_taken = false;
            }
            (BreakerState::HalfOpen, BreakerOutcome::Failure { signature }) => {
                core.state = BreakerState::Open;
                core.opened_at = tick;
                core.probe_taken = false;
                core.signature = signature;
                core.failures = core.failures.max(self.threshold).saturating_add(1);
                core.trips = core.trips.saturating_add(1);
            }
            (BreakerState::Open, _) => {}
        }
    }
}

/// Open becomes half open once its cooldown has passed.
fn cooled(state: BreakerState, opened_at: u64, tick: u64, cooldown_ticks: u64) -> BreakerState {
    if state == BreakerState::Open && tick.saturating_sub(opened_at) >= cooldown_ticks {
        BreakerState::HalfOpen
    } else {
        state
    }
}

/// One request behind a breaker: everything `post_json_retrying` takes plus
/// the caller's tick for the breaker's transition.
#[derive(Clone, Copy, Debug)]
pub struct GuardedSend<'a> {
    /// Provider host and port.
    pub endpoint: &'a Endpoint,
    /// JSON body to POST.
    pub body: &'a str,
    /// Bearer token, when the provider needs one.
    pub bearer: Option<&'a str>,
    /// Whole-request budget including retries, ms.
    pub budget_ms: u64,
    /// Request id; it seeds the retry pauses.
    pub seed: u64,
    /// Caller tick the breaker reads.
    pub tick: u64,
}

impl GuardedSend<'_> {
    /// Send once behind `breaker`, retrying under `gate` as usual.
    ///
    /// Errors: [`BreakerError::Open`] without touching the network while the
    /// circuit is open, or [`BreakerError::Transport`] from the send itself.
    pub fn send(
        &self,
        gate: &RateGate,
        breaker: &Breaker,
    ) -> Result<TransportResponse, BreakerError> {
        if let Err(refusal) = breaker.permit(self.tick) {
            return Err(BreakerError::Open(refusal));
        }
        let sent = post_json_retrying(
            self.endpoint,
            self.body,
            self.bearer,
            self.budget_ms,
            self.seed,
            gate,
        );
        breaker.record(self.tick, breaker_outcome(&sent));
        sent.map_err(BreakerError::Transport)
    }
}

/// What one send reports to its breaker: a server fault or a socket fault is
/// a failure signed by its status or kind; a refused request, a rate limit and
/// an answer all count as the provider being alive.
fn breaker_outcome(sent: &Result<TransportResponse, TransportError>) -> BreakerOutcome {
    match sent {
        Ok(response) if (500..=599).contains(&response.status) || response.status == 408 => {
            BreakerOutcome::Failure {
                signature: u64::from(response.status),
            }
        }
        Err(error) => match error {
            TransportError::Connect
            | TransportError::Write
            | TransportError::Read
            | TransportError::Protocol
            | TransportError::Timeout => BreakerOutcome::Failure {
                signature: error_signature(*error),
            },
            _ => BreakerOutcome::Success,
        },
        Ok(_) => BreakerOutcome::Success,
    }
}

/// Stable number per socket failure kind, so a run of the same fault repeats
/// its signature and a different one starts a new run.
fn error_signature(error: TransportError) -> u64 {
    match error {
        TransportError::Connect => 1,
        TransportError::Timeout => 2,
        TransportError::Write => 3,
        TransportError::Read => 4,
        TransportError::Protocol => 5,
        TransportError::BadEncoding => 6,
        TransportError::TooLarge => 7,
        TransportError::Redirect => 8,
        TransportError::Tls => 9,
        TransportError::Status(code) => 1_000 + u64::from(code),
    }
}

/// A guarded send that did not reach the provider's answer path.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BreakerError {
    /// The circuit is open: nothing was sent and nothing was retried.
    Open(BreakerRefusal),
    /// The send itself failed after its retries.
    Transport(TransportError),
}

impl BreakerError {
    /// Stable machine name: `circuit_open` or the transport kind.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            BreakerError::Open(refusal) => refusal.code(),
            BreakerError::Transport(_) => "transport",
        }
    }
}

impl std::fmt::Display for BreakerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BreakerError::Open(refusal) => refusal.fmt(f),
            BreakerError::Transport(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for BreakerError {}

impl From<BreakerRefusal> for BreakerError {
    fn from(refusal: BreakerRefusal) -> Self {
        BreakerError::Open(refusal)
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

    fn test_breaker() -> Breaker {
        Breaker::new(3, 10).expect("breaker config")
    }

    #[test]
    fn breaker_opens_after_k_identical_failures_and_refuses_by_name() {
        let breaker = test_breaker();
        assert_eq!(breaker.state(0), BreakerState::Closed);
        for tick in 0..2 {
            assert_eq!(breaker.permit(tick), Ok(BreakerPermit::Allow));
            breaker.record(tick, BreakerOutcome::Failure { signature: 503 });
        }
        assert_eq!(breaker.state(2), BreakerState::Closed, "two of three");
        assert_eq!(breaker.failures(), 2);
        breaker.record(2, BreakerOutcome::Failure { signature: 503 });
        assert_eq!(breaker.state(2), BreakerState::Open);
        assert_eq!(breaker.trips(), 1);
        assert_eq!(breaker.failures(), 3);

        let refusal = breaker.permit(3).expect_err("the circuit is open");
        assert_eq!(refusal.code(), "circuit_open");
        assert_eq!(refusal.failures, 3);
        assert_eq!(
            refusal.to_string(),
            "circuit_open failures=3 tick=3",
            "the refusal names the count"
        );
        assert_eq!(breaker.refusals(), 1);
        assert_eq!(BreakerError::Open(refusal).code(), "circuit_open");

        // A different failure is a different run: three of them are needed too.
        let mixed = test_breaker();
        mixed.record(0, BreakerOutcome::Failure { signature: 503 });
        mixed.record(1, BreakerOutcome::Failure { signature: 503 });
        mixed.record(2, BreakerOutcome::Failure { signature: 500 });
        assert_eq!(mixed.failures(), 1, "the run starts again");
        assert_eq!(mixed.state(2), BreakerState::Closed);

        // A success in the closed state forgets the run.
        let reset = test_breaker();
        reset.record(0, BreakerOutcome::Failure { signature: 503 });
        reset.record(1, BreakerOutcome::Failure { signature: 503 });
        reset.record(2, BreakerOutcome::Success);
        assert_eq!(reset.failures(), 0);
        reset.record(3, BreakerOutcome::Failure { signature: 503 });
        assert_eq!(reset.state(3), BreakerState::Closed);

        assert_eq!(
            Breaker::new(0, 10).expect_err("zero threshold"),
            BreakerConfigError::ZeroThreshold
        );
        assert_eq!(
            Breaker::new(1, 0).expect_err("zero cooldown"),
            BreakerConfigError::ZeroCooldown
        );
    }

    #[test]
    fn breaker_half_opens_after_the_cooldown_and_one_probe_closes_it() {
        let breaker = Breaker::new(2, 10).expect("breaker config");
        breaker.record(0, BreakerOutcome::Failure { signature: 503 });
        breaker.record(1, BreakerOutcome::Failure { signature: 503 });
        assert_eq!(breaker.state(1), BreakerState::Open);
        assert_eq!(breaker.state(10), BreakerState::Open, "still cooling");
        assert_eq!(breaker.state(11), BreakerState::HalfOpen);
        assert_eq!(breaker.permit(11), Ok(BreakerPermit::Probe), "one probe");
        let second = breaker.permit(11).expect_err("the probe is out");
        assert_eq!(second.code(), "circuit_open");
        assert_eq!(breaker.refusals(), 1);

        breaker.record(11, BreakerOutcome::Success);
        assert_eq!(breaker.state(11), BreakerState::Closed);
        assert_eq!(breaker.failures(), 0);
        assert_eq!(breaker.permit(12), Ok(BreakerPermit::Allow));
        assert_eq!(breaker.trips(), 1);
    }

    #[test]
    fn breaker_probe_failure_reopens_for_a_fresh_cooldown() {
        let breaker = Breaker::new(2, 10).expect("breaker config");
        breaker.record(0, BreakerOutcome::Failure { signature: 503 });
        breaker.record(1, BreakerOutcome::Failure { signature: 503 });
        assert_eq!(breaker.permit(11), Ok(BreakerPermit::Probe));
        breaker.record(11, BreakerOutcome::Failure { signature: 503 });
        assert_eq!(
            breaker.state(11),
            BreakerState::Open,
            "reopened at the probe"
        );
        assert_eq!(breaker.trips(), 2);
        assert_eq!(
            breaker.permit(20),
            Err(BreakerRefusal {
                failures: breaker.failures(),
                tick: 20,
            })
        );
        assert_eq!(breaker.permit(21), Ok(BreakerPermit::Probe));
    }

    /// The open circuit refuses before any send: the stub sees no extra hit.
    #[test]
    fn guarded_send_refuses_while_open_without_touching_the_network() {
        let stub = StubServer::spawn_raw(503, String::new());
        let endpoint = endpoint(&stub);
        let breaker = Breaker::new(2, 5).expect("breaker config");
        let gate = RateGate::default();
        let send = |tick: u64| {
            GuardedSend {
                endpoint: &endpoint,
                body: "{}",
                bearer: None,
                budget_ms: 5_000,
                seed: 7,
                tick,
            }
            .send(&gate, &breaker)
        };

        // One refused call spends the first send and every retry it allows.
        let per_call = 1 + u64::from(MAX_RETRIES);
        assert_eq!(send(0).expect("answered").status, 503);
        assert_eq!(stub.hits(), per_call);
        assert_eq!(send(1).expect("answered").status, 503);
        assert_eq!(stub.hits(), 2 * per_call);
        let hits = stub.hits();
        assert_eq!(breaker.state(1), BreakerState::Open);

        let refused = send(1).expect_err("the circuit is open");
        assert_eq!(refused.code(), "circuit_open");
        match &refused {
            BreakerError::Open(refusal) => assert_eq!(refusal.failures, 2),
            other => panic!("expected an open refusal, got {other:?}"),
        }
        assert_eq!(stub.hits(), hits, "nothing was sent, nothing retried");

        // The probe goes out after the cooldown and reopens on its own fault.
        assert_eq!(send(6).expect("answered").status, 503);
        assert_eq!(stub.hits(), hits + per_call);
        assert_eq!(breaker.trips(), 2);
        assert_eq!(send(6).expect_err("open again").code(), "circuit_open");
    }

    /// A successful probe closes the circuit and the next call passes.
    #[test]
    fn guarded_send_closes_on_a_successful_probe() {
        let stub = StubServer::spawn_raw(200, String::from("{}"));
        let endpoint = endpoint(&stub);
        let breaker = Breaker::new(1, 5).expect("breaker config");
        let gate = RateGate::default();
        // One recorded failure opens it; the probe at tick 5 answers 200.
        breaker.record(0, BreakerOutcome::Failure { signature: 503 });
        assert_eq!(breaker.state(0), BreakerState::Open);
        let response = GuardedSend {
            endpoint: &endpoint,
            body: "{}",
            bearer: None,
            budget_ms: 5_000,
            seed: 7,
            tick: 5,
        }
        .send(&gate, &breaker)
        .expect("the probe is allowed");
        assert_eq!(response.status, 200);
        assert_eq!(breaker.state(5), BreakerState::Closed);
        assert_eq!(breaker.failures(), 0);
        assert_eq!(stub.hits(), 1);
    }
}
