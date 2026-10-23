//! pool: one persistent job pool with scoped jobs and index-ordered merges.
//!
//! Owner: engine-ecs. [`JobPool::new`] starts its workers once; every
//! [`JobPool::scope`] after that queues erased jobs on them and spawns no
//! thread. A job small enough for inline storage costs no heap call, so a
//! warmed pool runs a tick without per-job allocation. A scope counts its
//! open jobs (the wait counter); the caller helps run queued jobs while the
//! counter is above zero, so a pool of zero workers runs everything inline
//! on the caller and a job may open a nested scope without starving the
//! pool.
//!
//! Determinism: the pool decides only where a job runs, never what it
//! returns. [`JobPool::par_chunks_map`] gives each chunk its own output slot
//! and returns the slots in chunk order, so 0, 1, 2, 4 or 8 workers give the
//! same bytes. Sim callers read last-tick data in jobs and merge in index
//! order; the hashed state stays single-threaded.
//!
//! Panics: every job runs under `catch_unwind`. The scope waits for all of
//! its jobs, then re-raises with `resume_unwind` the closure's own panic or,
//! failing that, the panic of the lowest-numbered job (a fixed choice, not
//! the first to finish). A worker never dies from a job panic. Dropping the
//! pool sets shutdown and joins every worker.
//!
//! Model check: `RUSTFLAGS="--cfg loom" cargo test -p engine-ecs --lib loom_pool`
//! runs the scope protocol under [loom](https://docs.rs/loom), which tries
//! every interleaving of the caller and the worker. A scope that returns
//! before its last job ran fails it.

use std::any::Any;
use std::cell::Cell;
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::num::NonZeroUsize;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::{Arc, OnceLock, PoisonError};
use std::thread;

// The lock and thread primitives the pool's protocol rests on. Under
// `--cfg loom` they are loom's mocks, so `loom_pool` below can try every
// interleaving of the queue, the wait counter and the workers.
#[cfg(loom)]
use loom::sync::{Condvar, Mutex, MutexGuard};
#[cfg(loom)]
use loom::thread::{Builder, JoinHandle};
#[cfg(not(loom))]
use std::sync::{Condvar, Mutex, MutexGuard};
#[cfg(not(loom))]
use std::thread::{Builder, JoinHandle};

use crate::jobs::ChunkSize;

/// Most workers one pool may start.
pub const MAX_WORKERS: usize = 256;

/// Workers of the crate's shared pool are capped here, whatever the core
/// count, so a bulk helper never floods a big machine.
const SHARED_MAX_WORKERS: usize = 15;

/// Erased storage for one queued job.
mod erased {
    use core::cell::Cell;
    use core::marker::PhantomData;
    use core::mem::{align_of, size_of, MaybeUninit};

    /// Bytes a job carries inline. A larger or over-aligned closure spills to
    /// one heap box and stores the same way.
    const INLINE_JOB_BYTES: usize = 128;

    /// Storage aligned like a `u128`, which covers ordinary closures.
    #[repr(align(16))]
    struct Buf([MaybeUninit<u8>; INLINE_JOB_BYTES]);

    /// One step over a stored closure: the store that creates it, then either
    /// the run that consumes it or the drop that discards it.
    enum Step<S> {
        Store(S),
        Run,
        Drop,
    }

    /// Apply `step` to the closure at `ptr`: the one place erased storage is
    /// touched. Each job gets one store, then at most one run or one drop.
    #[expect(
        unsafe_code,
        reason = "a job's closure type is erased into fixed storage; this one \
                  match writes, calls or drops the stored value"
    )]
    fn apply<S: FnOnce()>(ptr: *mut u8, step: Step<S>) {
        // SAFETY: `ptr` is the start of one job buffer. Store runs on empty
        // storage and initializes exactly one `S`; Run and Drop run only
        // after that Store, never together, and consume what they read. The
        // closure may borrow its scope: `Scope::spawn` counts the job open
        // before queueing it and the scope waits for every job before its
        // borrows end, so Run never calls a dead borrow.
        unsafe {
            match step {
                Step::Store(value) => ptr.cast::<S>().write(value),
                Step::Run => ptr.cast::<S>().read()(),
                Step::Drop => ptr.cast::<S>().drop_in_place(),
            }
        }
    }

    /// Run (`true`) or drop (`false`) the closure at `ptr`: the erased half
    /// a job stores as a plain `fn` pointer.
    fn walk<S: FnOnce()>(ptr: *mut u8, run: bool) {
        apply::<S>(ptr, if run { Step::Run } else { Step::Drop });
    }

    /// One queued job with its closure type erased.
    ///
    /// A closure that fits the buffer is stored inline, so queueing it costs
    /// no heap call; a larger or over-aligned one is boxed first. `walk` is
    /// the monomorphized run/drop pair for the stored type. Only `new`
    /// builds a job, and it takes only `Send` closures, so moving the job
    /// moves the closure. The stored closure need not be `Sync`, which the
    /// marker keeps off this type.
    pub(super) struct Job {
        buf: Buf,
        walk: fn(*mut u8, bool),
        /// Cleared by `run`, so a consumed job drops nothing.
        armed: bool,
        not_sync: PhantomData<Cell<()>>,
    }

    impl Job {
        /// Store `f` inline when its size and alignment fit, else in a box.
        pub(super) fn new<F: FnOnce() + Send>(f: F) -> Self {
            let mut buf = Buf([MaybeUninit::uninit(); INLINE_JOB_BYTES]);
            let walk: fn(*mut u8, bool) =
                if size_of::<F>() <= INLINE_JOB_BYTES && align_of::<F>() <= align_of::<Buf>() {
                    apply::<F>(buf.0.as_mut_ptr().cast(), Step::Store(f));
                    walk::<F>
                } else {
                    apply::<Box<F>>(buf.0.as_mut_ptr().cast(), Step::Store(Box::new(f)));
                    walk::<Box<F>>
                };
            Self {
                buf,
                walk,
                armed: true,
                not_sync: PhantomData,
            }
        }

        /// Run and consume the job. The scope runs each job once.
        pub(super) fn run(mut self) {
            self.armed = false;
            (self.walk)(self.buf.0.as_mut_ptr().cast(), true);
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            if self.armed {
                (self.walk)(self.buf.0.as_mut_ptr().cast(), false);
            }
        }
    }
}

use erased::Job;

/// A caught panic payload.
type Payload = Box<dyn Any + Send + 'static>;

/// Why a pool did not start. No worker is left running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoolError {
    /// More workers than [`MAX_WORKERS`] were asked for; the count is carried.
    TooManyWorkers(usize),
    /// The OS refused to start worker `index`; the workers already started
    /// were joined before this returned.
    Spawn {
        /// Index of the worker that failed to start.
        index: usize,
        /// The OS error kind.
        kind: std::io::ErrorKind,
    },
}

impl core::fmt::Display for PoolError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooManyWorkers(n) => {
                write!(f, "job pool of {n} workers is over the {MAX_WORKERS} cap")
            }
            Self::Spawn { index, kind } => {
                write!(f, "job pool worker {index} did not start: {kind}")
            }
        }
    }
}

impl std::error::Error for PoolError {}

/// Lock that survives poison. Every guarded state here (a job queue plus a
/// flag, a counter plus a payload slot) is whole after each statement and no
/// user code runs while a guard is held, so a poisoned guard is still valid.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Queue shared by the workers and every scope.
struct Queue {
    jobs: VecDeque<Job>,
    shutdown: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    /// Signals idle workers: a job was queued, or shutdown was set.
    ready: Condvar,
}

impl Shared {
    fn push(&self, job: Job) {
        lock(&self.queue).jobs.push_back(job);
        self.ready.notify_one();
    }

    fn try_pop(&self) -> Option<Job> {
        lock(&self.queue).jobs.pop_front()
    }
}

/// Worker body: run queued jobs until shutdown finds the queue empty. Jobs
/// catch their own panics, so this loop never unwinds.
fn worker(shared: &Shared) {
    loop {
        let job = {
            let mut queue = lock(&shared.queue);
            loop {
                if let Some(job) = queue.jobs.pop_front() {
                    break Some(job);
                }
                if queue.shutdown {
                    break None;
                }
                queue = shared
                    .ready
                    .wait(queue)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        };
        match job {
            Some(job) => job.run(),
            None => return,
        }
    }
}

/// A fixed set of worker threads started once and joined on drop.
///
/// Use [`scope`](Self::scope) for jobs that borrow caller data and
/// [`par_chunks_map`](Self::par_chunks_map) for an index-ordered map.
pub struct JobPool {
    shared: Arc<Shared>,
    workers: Vec<JoinHandle<()>>,
}

impl core::fmt::Debug for JobPool {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("JobPool")
            .field("workers", &self.workers.len())
            .finish()
    }
}

impl JobPool {
    /// Start `workers` threads, `0..=MAX_WORKERS`. Zero starts none: every
    /// job then runs inline on the caller of [`scope`](Self::scope).
    ///
    /// Errors: [`PoolError::TooManyWorkers`] past the cap, or
    /// [`PoolError::Spawn`] when the OS refuses a thread (the workers already
    /// started are joined first).
    pub fn new(workers: usize) -> Result<Self, PoolError> {
        if workers > MAX_WORKERS {
            return Err(PoolError::TooManyWorkers(workers));
        }
        let mut pool = Self::inline();
        pool.workers.reserve_exact(workers);
        for index in 0..workers {
            let shared = Arc::clone(&pool.shared);
            let handle = Builder::new()
                .name(format!("engine-job-{index}"))
                .spawn(move || worker(&shared))
                .map_err(|e| PoolError::Spawn {
                    index,
                    kind: e.kind(),
                })?;
            pool.workers.push(handle);
        }
        Ok(pool)
    }

    /// Start a pool for `threads` total threads, caller included: `1` starts
    /// none (every job runs inline), `8` starts seven workers.
    ///
    /// Errors: [`PoolError::TooManyWorkers`] past the cap, or
    /// [`PoolError::Spawn`] when the OS refuses a thread.
    pub fn for_threads(threads: usize) -> Result<Self, PoolError> {
        Self::new(workers_for_threads(threads))
    }

    /// Workers this pool started. Zero means every job runs inline on the
    /// caller of [`scope`](Self::scope).
    #[must_use]
    pub fn workers(&self) -> usize {
        self.workers.len()
    }

    /// A pool with no workers; it cannot fail to start.
    fn inline() -> Self {
        Self {
            shared: Arc::new(Shared {
                queue: Mutex::new(Queue {
                    jobs: VecDeque::new(),
                    shutdown: false,
                }),
                ready: Condvar::new(),
            }),
            workers: Vec::new(),
        }
    }

    /// Open a scope whose jobs may borrow anything that outlives this call.
    ///
    /// Returns once every job spawned in the scope has finished. A panic in
    /// `f` or in any job is re-raised here with `resume_unwind` after that
    /// wait: `f`'s own panic first, else the lowest-numbered job's. The
    /// caller runs queued jobs while it waits, so zero workers means inline.
    pub fn scope<'env, F, R>(&'env self, f: F) -> R
    where
        F: for<'scope> FnOnce(&'scope Scope<'scope, 'env>) -> R,
    {
        let scope = Scope {
            pool: self,
            state: Arc::new(ScopeState {
                tally: Mutex::new(Tally {
                    pending: 0,
                    panic: None,
                }),
                done: Condvar::new(),
            }),
            next: Cell::new(0),
            scope: PhantomData,
            env: PhantomData,
        };
        let result = catch_unwind(AssertUnwindSafe(|| f(&scope)));
        self.help_until_done(&scope.state);
        let job_panic = lock(&scope.state.tally).panic.take();
        match (result, job_panic) {
            (Err(payload), _) | (Ok(_), Some((_, payload))) => resume_unwind(payload),
            (Ok(value), None) => value,
        }
    }

    /// Wait until `state` counts no open job, running queued jobs meanwhile.
    ///
    /// Jobs borrow the scope's caller data, so this must not unwind before
    /// the counter reaches zero; [`AbortOnUnwind`] turns any such unwind
    /// into an abort instead of a use-after-free.
    fn help_until_done(&self, state: &ScopeState) {
        let _guard = AbortOnUnwind::arm();
        loop {
            if lock(&state.tally).pending == 0 {
                return;
            }
            if let Some(job) = self.shared.try_pop() {
                job.run();
                continue;
            }
            // The queue is empty, so every open job of this scope is running
            // on some other thread (only this thread queues this scope's
            // jobs). Sleep until the last one reports.
            let mut tally = lock(&state.tally);
            while tally.pending > 0 {
                tally = state
                    .done
                    .wait(tally)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            return;
        }
    }

    /// Map `data` chunk by chunk, chunk `k` covering items
    /// `[k * c, (k + 1) * c)` for `c = chunk.value()`. `f` receives the
    /// chunk's first item index and the chunk. Returns one result per chunk
    /// in chunk order, whatever the worker count or finish order, so the
    /// output is byte-identical for any pool size. One chunk (or none) runs
    /// inline with no job queued. A panic in `f` re-raises on the caller.
    pub fn par_chunks_map<T, R, F>(&self, data: &[T], chunk: ChunkSize, f: F) -> Vec<R>
    where
        T: Sync,
        R: Send,
        F: Fn(usize, &[T]) -> R + Sync,
    {
        let c = chunk.value();
        if data.len() <= c {
            return if data.is_empty() {
                Vec::new()
            } else {
                vec![f(0, data)]
            };
        }
        let mut slots: Vec<Option<R>> = Vec::new();
        slots.resize_with(data.len().div_ceil(c), || None);
        let f = &f;
        self.scope(|s| {
            for (k, (part, slot)) in data.chunks(c).zip(slots.iter_mut()).enumerate() {
                s.spawn(move || *slot = Some(f(k * c, part)));
            }
        });
        // The scope returned without a panic, so every job ran to the end
        // and filled its slot: nothing is dropped here.
        slots.into_iter().flatten().collect()
    }
}

impl Drop for JobPool {
    fn drop(&mut self) {
        lock(&self.shared.queue).shutdown = true;
        self.shared.ready.notify_all();
        let mut first: Option<Payload> = None;
        for handle in self.workers.drain(..) {
            if let Err(payload) = handle.join() {
                first.get_or_insert(payload);
            }
        }
        // A worker only unwinds on a bug in this file (jobs catch their own
        // panics); surface it unless that would be a double panic.
        if let Some(payload) = first {
            if !thread::panicking() {
                resume_unwind(payload);
            }
        }
    }
}

/// Total thread count the shared pool may use, caller included: `1` runs
/// every job inline on the caller, `8` starts seven workers. The sim reads it
/// to prove its bytes do not depend on how many threads run it.
pub const THREADS_ENV: &str = "E2040_THREADS";

/// Workers a pool for `threads` total threads starts, capped at
/// [`MAX_WORKERS`]. One thread means no worker: every job runs inline on the
/// caller, so the whole run is single-threaded.
#[must_use]
pub fn workers_for_threads(threads: usize) -> usize {
    threads.saturating_sub(1).min(MAX_WORKERS)
}

/// Parse a total thread count. Zero, an empty string and junk mean "unset",
/// so a mistyped variable keeps the core-count default.
fn parse_threads(text: &str) -> Option<usize> {
    text.trim().parse::<usize>().ok().filter(|n| *n > 0)
}

/// The crate's shared pool for the bulk helpers in [`crate::jobs`] and
/// [`crate::parallel`], sized to `E2040_THREADS` when it is set and to the
/// core count minus the caller otherwise.
///
/// Started on first use and never dropped: its idle workers sleep on the
/// queue condvar until the process exits. It holds no session or match
/// state, only threads; sim code takes an explicit [`JobPool`] instead.
pub fn shared() -> &'static JobPool {
    static SHARED: OnceLock<JobPool> = OnceLock::new();
    SHARED.get_or_init(|| {
        let cores = thread::available_parallelism().map_or(1, NonZeroUsize::get);
        let asked = std::env::var(THREADS_ENV)
            .ok()
            .and_then(|text| parse_threads(&text));
        let workers = asked
            .map_or_else(|| cores.saturating_sub(1), workers_for_threads)
            .min(SHARED_MAX_WORKERS);
        JobPool::new(workers).unwrap_or_else(|_| JobPool::inline())
    })
}

/// Open-job counter plus the lowest-numbered job panic of one scope.
struct Tally {
    pending: usize,
    panic: Option<(usize, Payload)>,
}

struct ScopeState {
    tally: Mutex<Tally>,
    /// Signals the scope owner: the counter reached zero.
    done: Condvar,
}

impl ScopeState {
    /// Count job `index` as finished, keeping its panic if it is the lowest.
    fn finish(&self, index: usize, outcome: Result<(), Payload>) {
        let loser = {
            let mut tally = lock(&self.tally);
            tally.pending = tally.pending.saturating_sub(1);
            let loser = match (outcome, tally.panic.take()) {
                (Ok(()), kept) => {
                    tally.panic = kept;
                    None
                }
                (Err(payload), None) => {
                    tally.panic = Some((index, payload));
                    None
                }
                (Err(payload), Some(kept)) if index < kept.0 => {
                    tally.panic = Some((index, payload));
                    Some(kept.1)
                }
                (Err(payload), kept) => {
                    tally.panic = kept;
                    Some(payload)
                }
            };
            if tally.pending == 0 {
                self.done.notify_all();
            }
            loser
        };
        // A dropped payload runs user Drop code: never under the lock.
        drop(loser);
    }
}

/// Aborts the process if dropped by an unwind that started after it was
/// armed. Guards the wait that keeps borrowed job data alive.
struct AbortOnUnwind {
    was_panicking: bool,
}

impl AbortOnUnwind {
    fn arm() -> Self {
        Self {
            was_panicking: thread::panicking(),
        }
    }
}

impl Drop for AbortOnUnwind {
    fn drop(&mut self) {
        if thread::panicking() && !self.was_panicking {
            std::process::abort();
        }
    }
}

/// A scope of jobs that may borrow data living for `'env`. Built only by
/// [`JobPool::scope`], which waits for every job before it returns.
///
/// Not `Sync`: only the thread that opened the scope can spawn into it, so
/// that thread alone queues this scope's jobs.
pub struct Scope<'scope, 'env: 'scope> {
    pool: &'env JobPool,
    state: Arc<ScopeState>,
    /// Next job number; also keeps the type `!Sync`.
    next: Cell<usize>,
    scope: PhantomData<&'scope mut &'scope ()>,
    env: PhantomData<&'env mut &'env ()>,
}

impl<'scope, 'env> Scope<'scope, 'env> {
    /// Queue `f` on the pool. Jobs are numbered in spawn order; the number
    /// only picks which panic re-raises when several jobs panic.
    ///
    /// Safety of the erased borrow lifetime rests on the scope: a `Scope`
    /// exists only inside [`JobPool::scope`], and only by reference. The job
    /// was counted open above, before it is queued; that call does not
    /// return or unwind until the counter is zero again
    /// (`help_until_done`, guarded by [`AbortOnUnwind`]), and the counter
    /// drops only after the job has consumed `f` and its borrows. So the job
    /// never runs or holds a borrow past `'scope`.
    pub fn spawn<F>(&'scope self, f: F)
    where
        F: FnOnce() + Send + 'scope,
    {
        let index = self.next.get();
        self.next.set(index.saturating_add(1));
        lock(&self.state.tally).pending += 1;
        let state = Arc::clone(&self.state);
        self.pool.shared.push(Job::new(move || {
            // `f` and everything it borrows is consumed inside
            // `catch_unwind`; after it only the `'static` state is touched.
            let outcome = catch_unwind(AssertUnwindSafe(f));
            state.finish(index, outcome);
        }));
    }
}

#[cfg(all(test, loom))]
mod loom_pool {
    use super::*;
    use loom::sync::atomic::{AtomicUsize, Ordering};

    /// One worker and the helping caller race for two jobs. In every
    /// interleaving the scope returns only after both jobs ran, and dropping
    /// the pool joins the worker.
    #[test]
    fn loom_pool_scope_returns_after_every_job() {
        loom::model(|| {
            let pool = JobPool::new(1).expect("one loom worker starts");
            let ran = AtomicUsize::new(0);
            pool.scope(|s| {
                s.spawn(|| {
                    ran.fetch_add(1, Ordering::SeqCst);
                });
                s.spawn(|| {
                    ran.fetch_add(1, Ordering::SeqCst);
                });
            });
            assert_eq!(ran.load(Ordering::SeqCst), 2);
        });
    }
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;
    use engine_core::hash_u64s;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Weak;
    use std::thread::ThreadId;

    /// Fixed workload: per-chunk digest of uneven busy work.
    fn digest(start: usize, part: &[u64]) -> u64 {
        let mut acc = u64::try_from(start).unwrap_or(u64::MAX);
        for (i, x) in part.iter().enumerate() {
            let rounds = (start + i) % 13 + 1;
            let mut v = *x;
            for _ in 0..rounds * 20 {
                v = engine_core::splitmix64(v ^ acc);
            }
            acc = acc.rotate_left(9) ^ v;
        }
        acc
    }

    fn workload() -> Vec<u64> {
        (0..4096u64).map(engine_core::splitmix64).collect()
    }

    fn serial(data: &[u64], chunk: usize) -> Vec<u64> {
        data.chunks(chunk)
            .enumerate()
            .map(|(k, part)| digest(k * chunk, part))
            .collect()
    }

    fn bytes(xs: &[u64]) -> Vec<u8> {
        xs.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    #[test]
    fn job_pool_threads_workers_follow_the_total_thread_count() {
        assert_eq!(workers_for_threads(0), 0);
        assert_eq!(workers_for_threads(1), 0);
        assert_eq!(workers_for_threads(2), 1);
        assert_eq!(workers_for_threads(4), 3);
        assert_eq!(workers_for_threads(8), 7);
        assert_eq!(workers_for_threads(usize::MAX), MAX_WORKERS);
        // Junk keeps the core-count default; a real count is read.
        assert_eq!(parse_threads("8"), Some(8));
        assert_eq!(parse_threads(" 1 "), Some(1));
        assert_eq!(parse_threads("0"), None);
        assert_eq!(parse_threads(""), None);
        assert_eq!(parse_threads("many"), None);
        let inline = JobPool::for_threads(1).expect("inline pool starts");
        assert_eq!(inline.workers(), 0);
        let eight = JobPool::for_threads(8).expect("eight threads start");
        assert_eq!(eight.workers(), 7);
    }

    #[test]
    fn job_pool_threads_give_identical_bytes_at_1_2_4_and_8() {
        let data = workload();
        for chunk in [1, 7, 64, 1000, 5000] {
            let want = serial(&data, chunk);
            let want_bytes = bytes(&want);
            for workers in [0, 1, 2, 4, 8] {
                let pool = JobPool::new(workers).expect("pool starts");
                for round in 0..3 {
                    let got = pool.par_chunks_map(&data, ChunkSize::new(chunk), digest);
                    assert_eq!(bytes(&got), want_bytes, "w{workers} c{chunk} r{round}");
                    assert_eq!(hash_u64s(&got), hash_u64s(&want));
                }
            }
        }
        let empty: Vec<u64> = Vec::new();
        let pool = JobPool::new(2).expect("pool starts");
        assert!(pool
            .par_chunks_map(&empty, ChunkSize::new(4), digest)
            .is_empty());
    }

    #[test]
    fn job_pool_threads_scope_writes_borrowed_slots_in_index_order() {
        let pool = JobPool::new(4).expect("pool starts");
        let mut out = vec![0u64; 300];
        pool.scope(|s| {
            for (i, slot) in (0..300u64).zip(out.iter_mut()) {
                s.spawn(move || *slot = engine_core::splitmix64(i));
            }
        });
        let want: Vec<u64> = (0..300u64).map(engine_core::splitmix64).collect();
        assert_eq!(out, want);
        // The scope's return value passes through.
        assert_eq!(pool.scope(|_| 41) + 1, 42);
    }

    #[test]
    fn job_pool_threads_spawn_allocates_nothing_once_the_queue_is_warm() {
        let pool = JobPool::new(0).expect("inline pool starts");
        let ran = AtomicUsize::new(0);
        // Warm the shared queue to its steady size: growing it is the one
        // allocation a spawn may still trigger. Boxed jobs used to count one
        // heap call per spawn here; inline storage counts none.
        pool.scope(|s| {
            for _ in 0..64 {
                let ran = &ran;
                s.spawn(move || {
                    ran.fetch_add(1, Ordering::Relaxed);
                });
            }
        });
        assert_eq!(ran.load(Ordering::Relaxed), 64);
        ran.store(0, Ordering::Relaxed);
        let count = pool.scope(|s| {
            let ((), count) = engine_core::alloc_probe::measure(|| {
                for _ in 0..64 {
                    let ran = &ran;
                    s.spawn(move || {
                        ran.fetch_add(1, Ordering::Relaxed);
                    });
                }
            });
            count
        });
        assert_eq!(ran.load(Ordering::Relaxed), 64, "every spawned job ran");
        assert_eq!(count.calls, 0, "spawn after warmup allocated: {count:?}");
    }

    #[test]
    fn job_pool_threads_panic_reraises_on_the_caller() {
        for workers in [0, 1, 4] {
            let pool = JobPool::new(workers).expect("pool starts");
            let ran = AtomicUsize::new(0);
            let caught = catch_unwind(AssertUnwindSafe(|| {
                pool.scope(|s| {
                    for i in 0..16 {
                        let ran = &ran;
                        s.spawn(move || {
                            ran.fetch_add(1, Ordering::Relaxed);
                            assert!(i != 5 && i != 11, "job {i} blew up");
                        });
                    }
                });
            }));
            let payload = caught.expect_err("a job panic reaches the caller");
            let text = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .unwrap_or_default();
            // The lowest-numbered panicking job wins, whatever finished first.
            assert!(text.contains("job 5 blew up"), "w{workers}: {text:?}");
            assert_eq!(ran.load(Ordering::Relaxed), 16, "every job still ran");
            // The pool survives a job panic.
            let again = pool.par_chunks_map(&[1u64, 2, 3, 4], ChunkSize::new(1), |_, p| p[0]);
            assert_eq!(again, vec![1, 2, 3, 4], "w{workers}");
        }
    }

    #[test]
    fn job_pool_threads_scope_panic_waits_for_its_jobs_first() {
        let pool = JobPool::new(2).expect("pool starts");
        let ran = AtomicUsize::new(0);
        let caught = catch_unwind(AssertUnwindSafe(|| {
            pool.scope(|s| {
                for _ in 0..32 {
                    s.spawn(|| {
                        ran.fetch_add(1, Ordering::Relaxed);
                    });
                }
                panic!("scope body blew up");
            });
        }));
        let payload = caught.expect_err("the body panic reaches the caller");
        assert_eq!(payload.downcast_ref::<&str>(), Some(&"scope body blew up"));
        assert_eq!(
            ran.load(Ordering::Relaxed),
            32,
            "jobs finished before unwind"
        );
    }

    /// Record the current thread once. `ThreadId` has no order, so a
    /// short list stands in for a set.
    fn note(seen: &Mutex<Vec<ThreadId>>) {
        let id = thread::current().id();
        let mut ids = lock(seen);
        if !ids.contains(&id) {
            ids.push(id);
        }
    }

    /// Thread ids are never reused in a process, so a spawn per call would
    /// show up as new ids here.
    fn ids_seen(pool: &JobPool, calls: usize) -> Vec<ThreadId> {
        let seen = Mutex::new(Vec::new());
        let data: Vec<u64> = (0..64).collect();
        for _ in 0..calls {
            pool.par_chunks_map(&data, ChunkSize::new(4), |_, _| note(&seen));
        }
        seen.into_inner().unwrap_or_else(PoisonError::into_inner)
    }

    fn pool_threads(pool: &JobPool) -> Vec<ThreadId> {
        let mut ids: Vec<ThreadId> = pool.workers.iter().map(|h| h.thread().id()).collect();
        ids.push(thread::current().id());
        ids
    }

    #[test]
    fn job_pool_threads_zero_spawn_after_warm_up() {
        let pool = JobPool::new(3).expect("pool starts");
        let allowed = pool_threads(&pool);
        let seen = ids_seen(&pool, 200);
        assert!(
            seen.iter().all(|id| allowed.contains(id)),
            "{seen:?} vs {allowed:?}"
        );
        assert!(seen.len() <= 4, "200 calls ran on {} threads", seen.len());
        // The ported bulk helpers ride the shared pool. Other test threads
        // waiting on that pool may help run these jobs, so a few foreign
        // ids are fine; a spawn per call would add a fresh id every call.
        let allowed = pool_threads(shared());
        let seen = Mutex::new(Vec::new());
        let calls = 100;
        for _ in 0..calls {
            crate::parallel::fork_join(0..256, ChunkSize::new(8), |_| note(&seen), (), |(), ()| ())
                .expect("fork-join runs");
        }
        let seen = seen.into_inner().unwrap_or_else(PoisonError::into_inner);
        let foreign = seen.iter().filter(|id| !allowed.contains(id)).count();
        assert!(foreign < calls, "{foreign} threads outside the shared pool");
    }

    #[test]
    fn job_pool_threads_zero_workers_run_inline_on_the_caller() {
        let pool = JobPool::new(0).expect("inline pool");
        assert_eq!(ids_seen(&pool, 5), vec![thread::current().id()]);
    }

    #[test]
    fn job_pool_threads_drop_joins_every_worker() {
        let pool = JobPool::new(6).expect("pool starts");
        assert!(ids_seen(&pool, 3).len() <= 7);
        let weak: Weak<Shared> = Arc::downgrade(&pool.shared);
        // Each worker holds the shared state until its thread body returns.
        assert_eq!(weak.strong_count(), 7);
        drop(pool);
        assert_eq!(weak.strong_count(), 0, "a worker outlived the drop");
    }

    #[test]
    fn job_pool_threads_nested_scopes_do_not_starve() {
        // Every worker blocks in an inner scope; the helping wait keeps
        // the inner jobs moving.
        for workers in [0, 1, 2] {
            let pool = JobPool::new(workers).expect("pool starts");
            let outer: Vec<u64> = (0..8).collect();
            let got = pool.par_chunks_map(&outer, ChunkSize::new(1), |start, _| {
                let base = u64::try_from(start).unwrap_or(u64::MAX).wrapping_mul(100);
                let inner: Vec<u64> = (0..16).map(|i| i + base).collect();
                pool.par_chunks_map(&inner, ChunkSize::new(2), |_, p| p.iter().sum::<u64>())
                    .iter()
                    .sum::<u64>()
            });
            let want: Vec<u64> = (0..8u64)
                .map(|s| (0..16).map(|i| i + s * 100).sum())
                .collect();
            assert_eq!(got, want, "w{workers}");
        }
    }

    #[test]
    fn job_pool_threads_refuse_an_oversize_pool() {
        assert_eq!(
            JobPool::new(MAX_WORKERS + 1).err(),
            Some(PoolError::TooManyWorkers(MAX_WORKERS + 1))
        );
        assert!(PoolError::TooManyWorkers(9).to_string().contains("cap"));
        let spawn = PoolError::Spawn {
            index: 2,
            kind: std::io::ErrorKind::OutOfMemory,
        };
        assert!(spawn.to_string().contains("worker 2"));
    }
}
