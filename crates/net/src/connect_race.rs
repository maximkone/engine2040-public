//! connect_race: join a server over WebTransport or WebSocket, whichever
//! connects first.
//!
//! Purpose: many offices drop UDP (no QUIC, no WebTransport) and a
//! TLS-inspecting proxy can break `wss`, so the browser starts both legs and
//! keeps the first success. The loser is dropped, which cancels it. A failure
//! names every leg that failed and why, so a blocked network reads as a
//! blocked network and not as a dead server.
//!
//! Pure logic over futures: no sockets, no clock, no executor, so it runs
//! natively under fake legs and in the browser under `spawn_local`. A leg that
//! can hang is wrapped in [`bounded`](crate::connect_race::bounded) with the caller's own timer.
//!
//! Errors: [`RaceError`](crate::connect_race::RaceError) with one [`LegError`](crate::connect_race::LegError) per failed leg.

use std::fmt;
use std::future::{poll_fn, Future};
use std::pin::Pin;
use std::task::Poll;

/// One way to reach the server.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Leg {
    /// WebTransport over HTTP/3 (QUIC, UDP): datagrams plus one stream.
    WebTransport,
    /// WebSocket over TCP, usually port 443.
    WebSocket,
}

impl Leg {
    /// The leg's name as it appears in error text.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::WebTransport => "webtransport",
            Self::WebSocket => "websocket",
        }
    }
}

/// Why one leg did not connect.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LegError {
    /// The leg that failed.
    pub leg: Leg,
    /// What it reported.
    pub reason: String,
}

impl fmt::Display for LegError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} leg failed: {}", self.leg.name(), self.reason)
    }
}

/// Every leg failed. The failures are in the order they arrived.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RaceError {
    /// One entry per failed leg.
    pub failures: Vec<LegError>,
}

impl fmt::Display for RaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("no leg connected")?;
        for (index, failure) in self.failures.iter().enumerate() {
            f.write_str(if index == 0 { ": " } else { "; " })?;
            write!(f, "{failure}")?;
        }
        Ok(())
    }
}

impl std::error::Error for RaceError {}

/// The winning leg and what it connected.
#[derive(Debug)]
pub struct Connected<T> {
    /// The leg that connected first.
    pub leg: Leg,
    /// Its transport.
    pub value: T,
}

type Boxed<T, E> = Pin<Box<dyn Future<Output = Result<T, E>>>>;

/// Run both legs and keep the first success. The other leg is dropped the
/// moment one wins. When both are ready in the same poll WebTransport wins,
/// the better transport. When one leg fails the other keeps running; when
/// both fail the error carries both reasons.
pub async fn race<T, W, S>(
    webtransport: impl Future<Output = Result<T, W>> + 'static,
    websocket: impl Future<Output = Result<T, S>> + 'static,
) -> Result<Connected<T>, RaceError>
where
    T: 'static,
    W: fmt::Display + 'static,
    S: fmt::Display + 'static,
{
    let mut wt: Option<Boxed<T, W>> = Some(Box::pin(webtransport));
    let mut ws: Option<Boxed<T, S>> = Some(Box::pin(websocket));
    let mut failures = Vec::new();
    poll_fn(|cx| {
        if let Some(Poll::Ready(result)) = wt.as_mut().map(|leg| leg.as_mut().poll(cx)) {
            wt = None;
            match result {
                Ok(value) => {
                    ws = None;
                    return Poll::Ready(Ok(Connected {
                        leg: Leg::WebTransport,
                        value,
                    }));
                }
                Err(error) => failures.push(LegError {
                    leg: Leg::WebTransport,
                    reason: error.to_string(),
                }),
            }
        }
        if let Some(Poll::Ready(result)) = ws.as_mut().map(|leg| leg.as_mut().poll(cx)) {
            ws = None;
            match result {
                Ok(value) => {
                    wt = None;
                    return Poll::Ready(Ok(Connected {
                        leg: Leg::WebSocket,
                        value,
                    }));
                }
                Err(error) => failures.push(LegError {
                    leg: Leg::WebSocket,
                    reason: error.to_string(),
                }),
            }
        }
        if wt.is_none() && ws.is_none() {
            return Poll::Ready(Err(RaceError {
                failures: std::mem::take(&mut failures),
            }));
        }
        Poll::Pending
    })
    .await
}

/// Why a bounded leg did not connect.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BoundedError<E> {
    /// The leg failed on its own.
    Failed(E),
    /// The timer fired first; the field says how long that was.
    Silent(String),
}

impl<E: fmt::Display> fmt::Display for BoundedError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed(error) => write!(f, "{error}"),
            Self::Silent(after) => write!(f, "no answer within {after}"),
        }
    }
}

/// Give a leg a deadline: `timer` is any future that completes when time is
/// up (a `setTimeout` promise in the browser, `tokio::time::sleep` natively),
/// `after` says how long that is for the error text. A leg that answers first
/// keeps its own result; a leg that does not answer in time fails with a
/// reason that says so, which is what a UDP-dropping firewall looks like.
pub async fn bounded<T, E>(
    leg: impl Future<Output = Result<T, E>>,
    timer: impl Future<Output = ()>,
    after: &str,
) -> Result<T, BoundedError<E>> {
    let mut leg = std::pin::pin!(leg);
    let mut timer = std::pin::pin!(timer);
    poll_fn(|cx| {
        if let Poll::Ready(result) = leg.as_mut().poll(cx) {
            return Poll::Ready(result.map_err(BoundedError::Failed));
        }
        if timer.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(BoundedError::Silent(after.to_owned())));
        }
        Poll::Pending
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::task::{Context, Waker};

    /// Poll `future` until it completes or `limit` polls pass.
    fn drive<F: Future>(future: F, limit: usize) -> Option<F::Output> {
        let mut cx = Context::from_waker(Waker::noop());
        let mut future = std::pin::pin!(future);
        for _ in 0..limit {
            if let Poll::Ready(out) = future.as_mut().poll(&mut cx) {
                return Some(out);
            }
        }
        None
    }

    /// A fake leg: pending for `polls` polls, then `result`. Sets `dropped`
    /// when it is dropped, finished or not.
    struct Fake {
        polls: usize,
        result: Option<Result<u32, String>>,
        dropped: Rc<Cell<bool>>,
    }

    impl Future for Fake {
        type Output = Result<u32, String>;
        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            if self.polls == 0 {
                Poll::Ready(
                    self.result
                        .take()
                        .expect("a fake leg is polled to the end once"),
                )
            } else {
                self.polls -= 1;
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            self.dropped.set(true);
        }
    }

    fn fake(polls: usize, result: Result<u32, &str>) -> (Fake, Rc<Cell<bool>>) {
        let dropped = Rc::new(Cell::new(false));
        let leg = Fake {
            polls,
            result: Some(result.map_err(str::to_owned)),
            dropped: Rc::clone(&dropped),
        };
        (leg, dropped)
    }

    #[test]
    fn connect_race_keeps_the_first_success_and_cancels_the_loser() {
        let (wt, wt_dropped) = fake(5, Ok(1));
        let (ws, ws_dropped) = fake(2, Ok(2));
        let won = drive(race(wt, ws), 100)
            .expect("the race ends")
            .expect("one leg connected");
        assert_eq!((won.leg, won.value), (Leg::WebSocket, 2));
        assert!(wt_dropped.get(), "the slower leg is cancelled by the drop");

        let (wt, _) = fake(1, Ok(1));
        let (ws, ws_dropped_late) = fake(4, Ok(2));
        let won = drive(race(wt, ws), 100).expect("ends").expect("connects");
        assert_eq!((won.leg, won.value), (Leg::WebTransport, 1));
        assert!(ws_dropped_late.get());
        assert!(ws_dropped.get(), "the winner is consumed too");
    }

    #[test]
    fn connect_race_prefers_webtransport_when_both_are_ready_together() {
        let (wt, _) = fake(0, Ok(1));
        let (ws, _) = fake(0, Ok(2));
        let won = drive(race(wt, ws), 10).expect("ends").expect("connects");
        assert_eq!((won.leg, won.value), (Leg::WebTransport, 1));
    }

    #[test]
    fn connect_race_survives_one_failed_leg() {
        let (wt, _) = fake(1, Err("udp is blocked"));
        let (ws, _) = fake(6, Ok(9));
        let won = drive(race(wt, ws), 100)
            .expect("ends")
            .expect("ws still connects");
        assert_eq!((won.leg, won.value), (Leg::WebSocket, 9));

        let (wt, _) = fake(6, Ok(8));
        let (ws, _) = fake(1, Err("proxy refused the upgrade"));
        let won = drive(race(wt, ws), 100)
            .expect("ends")
            .expect("wt still connects");
        assert_eq!((won.leg, won.value), (Leg::WebTransport, 8));
    }

    #[test]
    fn connect_race_failure_names_every_blocked_leg_in_arrival_order() {
        let (wt, _) = fake(3, Err("handshake timed out"));
        let (ws, _) = fake(1, Err("proxy refused the upgrade"));
        let error = drive(race(wt, ws), 100)
            .expect("ends")
            .expect_err("both legs failed");
        assert_eq!(
            error.failures,
            vec![
                LegError {
                    leg: Leg::WebSocket,
                    reason: "proxy refused the upgrade".into()
                },
                LegError {
                    leg: Leg::WebTransport,
                    reason: "handshake timed out".into()
                },
            ]
        );
        assert_eq!(
            error.to_string(),
            "no leg connected: websocket leg failed: proxy refused the upgrade; \
             webtransport leg failed: handshake timed out"
        );
    }

    #[test]
    fn connect_race_stays_pending_while_a_leg_is_still_running() {
        let (wt, wt_dropped) = fake(usize::MAX, Ok(1));
        let (ws, _) = fake(0, Err("closed"));
        assert!(drive(race(wt, ws), 50).is_none(), "wt may still win");
        assert!(
            wt_dropped.get(),
            "dropping an unfinished race drops the leg"
        );
    }

    #[test]
    fn connect_race_deadline_turns_a_silent_leg_into_a_named_failure() {
        let (silent, silent_dropped) = fake(usize::MAX, Ok(1));
        let (timer, _) = fake(3, Ok(0));
        let result = drive(
            bounded(silent, async move { drop(timer.await) }, "4 s"),
            100,
        )
        .expect("the deadline ends it");
        assert_eq!(result, Err(BoundedError::Silent("4 s".to_owned())));
        assert_eq!(result.unwrap_err().to_string(), "no answer within 4 s");
        assert!(silent_dropped.get());

        let (quick, _) = fake(1, Ok(7));
        let (timer, _) = fake(usize::MAX, Ok(0));
        let result = drive(bounded(quick, async move { drop(timer.await) }, "4 s"), 100)
            .expect("the leg ends it");
        assert_eq!(result, Ok(7));
    }

    #[test]
    fn connect_race_deadline_leg_lets_the_race_fall_back_to_websocket() {
        let order = Rc::new(RefCell::new(Vec::new()));
        let (silent, _) = fake(usize::MAX, Ok(1));
        let (timer, _) = fake(4, Ok(0));
        let (ws, _) = fake(8, Ok(5));
        let seen = Rc::clone(&order);
        let wt = async move {
            let result = bounded(silent, async move { drop(timer.await) }, "4 s").await;
            seen.borrow_mut().push("wt gave up");
            result
        };
        let won = drive(race(wt, ws), 100)
            .expect("ends")
            .expect("ws connects");
        assert_eq!((won.leg, won.value), (Leg::WebSocket, 5));
        assert_eq!(*order.borrow(), vec!["wt gave up"]);
    }
}
