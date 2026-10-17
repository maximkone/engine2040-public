//! clock: the one monotonic time source for the shell and tool crates, native and browser.
//!
//! Consumer: crates that time IO and startup (`engine-assets`, `engine-app`; the game, net and
//! render sites follow). Data owner: this crate holds one process-wide [`Clock`]. Units: a
//! [`Duration`] since the clock was first read. Determinism scope: none. A reading is a
//! diagnostic and never enters a simulation hash, so no simulation crate depends on this one.
//!
//! Native targets read `std::time::Instant`. On `wasm32-unknown-unknown` std has no clock and
//! `Instant::now` panics, so the browser shell installs its own reading once with [`install`]
//! (the page's `performance.now()`). Until it has, a read there panics with a message that
//! says so. The source is a function pointer, not a dependency: this crate stays std-only and
//! the headless graph audited by `tools/workspace-check.mjs` keeps no web crate in it.

use std::sync::OnceLock;
use std::time::Duration;

/// A monotonic clock whose time source the host may set once.
#[derive(Debug, Default)]
pub struct Clock {
    host: OnceLock<fn() -> Duration>,
}

impl Clock {
    /// A clock on the platform's own time source, until the host installs one.
    pub const fn new() -> Self {
        Self {
            host: OnceLock::new(),
        }
    }

    /// Use `source` for every later reading. The first source wins: when one is already set
    /// this returns `false` and changes nothing.
    pub fn install(&self, source: fn() -> Duration) -> bool {
        self.host.set(source).is_ok()
    }

    /// Time since this source started counting. Never goes back.
    pub fn now(&self) -> Duration {
        match self.host.get() {
            Some(source) => source(),
            None => platform::now(),
        }
    }
}

static CLOCK: Clock = Clock::new();

/// Time on the process-wide clock.
pub fn now() -> Duration {
    CLOCK.now()
}

/// Time passed on the process-wide clock since `start`, a value [`now`] gave earlier.
/// A `start` from after this reading gives zero, never an underflow.
pub fn since(start: Duration) -> Duration {
    now().saturating_sub(start)
}

/// Set the process-wide clock's time source (the browser shell calls this once at startup).
/// Returns `false` and changes nothing when a source is already set.
pub fn install(source: fn() -> Duration) -> bool {
    CLOCK.install(source)
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
mod platform {
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    pub(super) fn now() -> Duration {
        static ORIGIN: OnceLock<Instant> = OnceLock::new();
        ORIGIN.get_or_init(Instant::now).elapsed()
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod platform {
    use std::time::Duration;

    pub(super) fn now() -> Duration {
        panic!("engine-clock: the browser has no std clock; the shell must call engine_clock::install before any read")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn five_ms() -> Duration {
        Duration::from_millis(5)
    }

    fn nine_ms() -> Duration {
        Duration::from_millis(9)
    }

    #[test]
    fn an_installed_source_answers_every_read() {
        let clock = Clock::new();
        assert!(clock.install(five_ms));
        assert_eq!(clock.now(), Duration::from_millis(5));
        assert_eq!(clock.now(), Duration::from_millis(5));
    }

    #[test]
    fn the_first_source_wins() {
        let clock = Clock::new();
        assert!(clock.install(five_ms));
        assert!(!clock.install(nine_ms), "a second source must be refused");
        assert_eq!(clock.now(), Duration::from_millis(5));
    }

    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    #[test]
    fn the_native_clock_moves_forward_with_real_time() {
        let clock = Clock::new();
        let first = clock.now();
        std::thread::sleep(Duration::from_millis(3));
        let second = clock.now();
        assert!(
            second >= first + Duration::from_millis(3),
            "{first:?} then {second:?}"
        );
    }

    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    #[test]
    fn since_counts_from_the_start_it_was_given() {
        let start = now();
        std::thread::sleep(Duration::from_millis(3));
        assert!(since(start) >= Duration::from_millis(3));
    }

    #[test]
    fn since_a_start_in_the_future_is_zero() {
        assert_eq!(since(Duration::MAX), Duration::ZERO);
    }
}
