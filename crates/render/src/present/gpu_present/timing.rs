//! timing: GPU pass timestamps resolved late and read without waiting.
//!
//! Owner: render lane, consumer the live frame encode in `submit`. Units:
//! device timestamp ticks converted to milliseconds through the queue's
//! timestamp period. A CPU `Instant` around a submit that waits measures
//! the wait, not the work; the device clock around the pass measures the
//! work (Game Engine Architecture 7.5). The timer is created only when the
//! adapter supports `Features::TIMESTAMP_QUERY`; without it the presenter
//! holds `None` and nothing else changes.
//! Determinism: timings are evidence only and never reach a hash or a
//! pixel. Errors: a failed map frees its slot and drops that sample.
//! Budget: one 32 byte resolve plus copy per frame, a ring of staging
//! buffers so a frame's result is read a few frames later by a
//! non-blocking `map_async` (the frame never waits for it).
use super::super::wait_stats::{percentiles, GpuTimingReport};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;

/// Timestamps per frame: forward begin and end, blit begin and end, then the
/// own-pass sky and the GTAO compute pass begin and end (each written only
/// on frames that run that pass).
const QUERY_COUNT: u32 = 8;
/// Bytes one frame resolves (eight u64 ticks).
const RESOLVE_BYTES: u64 = 64;
/// Staging buffers in flight: a result is read once its map completed.
const SLOT_COUNT: usize = 4;
/// Most samples kept per pass; older samples are overwritten.
const SAMPLE_CAP: usize = 8192;

const FREE: u8 = 0;
const PENDING: u8 = 1;
const READY: u8 = 2;
const FAILED: u8 = 3;

struct TimerSlot {
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
    /// The frame in flight wrote the sky timestamps (queries 4 and 5); a
    /// frame without the own-pass sky leaves them unwritten and unread.
    sky_written: AtomicBool,
    /// The same for the GTAO pass (queries 6 and 7).
    gtao_written: AtomicBool,
}

/// Which optional passes wrote their timestamps in one frame.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct PassesTimed {
    /// The own-pass sky wrote queries 4 and 5.
    pub(crate) sky: bool,
    /// The GTAO compute pass wrote queries 6 and 7.
    pub(crate) gtao: bool,
}

#[derive(Default)]
struct Ring {
    samples: Vec<f32>,
    next: usize,
}

impl Ring {
    fn push(&mut self, ms: f32) {
        if self.samples.len() < SAMPLE_CAP {
            self.samples.push(ms);
        } else if let Some(slot) = self.samples.get_mut(self.next % SAMPLE_CAP) {
            *slot = ms;
        }
        self.next = self.next.wrapping_add(1);
    }
}

/// Timestamp queries around the forward, own-pass sky, GTAO and blit passes.
pub(crate) struct GpuTimer {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    slots: Vec<TimerSlot>,
    period_ns: f32,
    forward: Ring,
    sky: Ring,
    blit: Ring,
    gtao: Ring,
    /// Forward plus sky plus GTAO plus blit per resolved frame.
    total: Ring,
    /// Device ms of the newest resolved sky pass; 0 until one lands.
    latest_sky_ms: f32,
    /// Device ms of the newest resolved GTAO pass; 0 until one lands.
    latest_gtao_ms: f32,
}

impl GpuTimer {
    /// Build the timer when the device was created with
    /// `TIMESTAMP_QUERY`; `None` otherwise.
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let period_ns = queue.get_timestamp_period();
        if !period_ns.is_finite() || period_ns <= 0.0 {
            return None;
        }
        let queries = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("frame-timestamps"),
            ty: wgpu::QueryType::Timestamp,
            count: QUERY_COUNT,
        });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame-timestamp-resolve"),
            size: RESOLVE_BYTES,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let slots = (0..SLOT_COUNT)
            .map(|_| TimerSlot {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("frame-timestamp-staging"),
                    size: RESOLVE_BYTES,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                state: Arc::new(AtomicU8::new(FREE)),
                sky_written: AtomicBool::new(false),
                gtao_written: AtomicBool::new(false),
            })
            .collect();
        Some(Self {
            queries,
            resolve,
            slots,
            period_ns,
            forward: Ring::default(),
            sky: Ring::default(),
            gtao: Ring::default(),
            blit: Ring::default(),
            total: Ring::default(),
            latest_sky_ms: 0.0,
            latest_gtao_ms: 0.0,
        })
    }

    /// Start a frame: let finished maps run their callbacks (a check, not
    /// a wait), read every result that landed, and hand back a free
    /// staging slot, or `None` when every slot is still in flight (the
    /// frame is simply not timed).
    pub(crate) fn begin_frame(&mut self, device: &wgpu::Device) -> Option<usize> {
        let _status = device.poll(wgpu::PollType::Poll);
        self.harvest();
        self.slots
            .iter()
            .position(|slot| slot.state.load(Ordering::Acquire) == FREE)
    }

    fn harvest(&mut self) {
        for index in 0..self.slots.len() {
            let Some(slot) = self.slots.get(index) else {
                continue;
            };
            match slot.state.load(Ordering::Acquire) {
                READY => {
                    let ticks = read_ticks(&slot.buffer);
                    let timed = PassesTimed {
                        sky: slot.sky_written.load(Ordering::Acquire),
                        gtao: slot.gtao_written.load(Ordering::Acquire),
                    };
                    slot.buffer.unmap();
                    slot.state.store(FREE, Ordering::Release);
                    if let Some(ticks) = ticks {
                        self.record(ticks, timed);
                    }
                }
                FAILED => slot.state.store(FREE, Ordering::Release),
                _ => {}
            }
        }
    }

    fn record(&mut self, ticks: [u64; 8], timed: PassesTimed) {
        let forward = self.span_ms(ticks[0], ticks[1]);
        let blit = self.span_ms(ticks[2], ticks[3]);
        self.forward.push(forward);
        self.blit.push(blit);
        let mut total = forward + blit;
        if timed.sky {
            let sky = self.span_ms(ticks[4], ticks[5]);
            self.sky.push(sky);
            self.latest_sky_ms = sky;
            total += sky;
        }
        if timed.gtao {
            let gtao = self.span_ms(ticks[6], ticks[7]);
            self.gtao.push(gtao);
            self.latest_gtao_ms = gtao;
            total += gtao;
        }
        self.total.push(total);
    }

    /// Forget every resolved sample (the slots in flight still land and are
    /// recorded after this call).
    pub(crate) fn clear_samples(&mut self) {
        self.forward = Ring::default();
        self.sky = Ring::default();
        self.gtao = Ring::default();
        self.blit = Ring::default();
        self.total = Ring::default();
        self.latest_sky_ms = 0.0;
        self.latest_gtao_ms = 0.0;
    }

    /// Device milliseconds of the newest resolved own-pass sky; 0.0 until one
    /// lands (no timestamps, sky off, or the first frames still in flight).
    /// A few frames late by construction: the read never waits.
    pub(crate) fn latest_sky_ms(&self) -> f32 {
        self.latest_sky_ms
    }

    /// Device milliseconds of the newest resolved GTAO pass; 0.0 until one
    /// lands. A few frames late by construction: the read never waits.
    pub(crate) fn latest_gtao_ms(&self) -> f32 {
        self.latest_gtao_ms
    }

    /// Milliseconds between two device ticks (period is ns per tick). A
    /// span past `u32::MAX` ticks or a reversed pair reads as a clamp, never
    /// a panic: the timestamps are evidence only.
    fn span_ms(&self, begin: u64, end: u64) -> f32 {
        let span = u32::try_from(end.saturating_sub(begin)).unwrap_or(u32::MAX);
        let ns = f64::from(span) * f64::from(self.period_ns);
        std::time::Duration::from_secs_f64(ns / 1.0e9).as_secs_f32() * 1000.0
    }

    /// Timestamp writes for the forward pass.
    pub(crate) fn forward_writes(&self) -> wgpu::RenderPassTimestampWrites<'_> {
        wgpu::RenderPassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: Some(0),
            end_of_pass_write_index: Some(1),
        }
    }

    /// Timestamp writes for the blit pass.
    pub(crate) fn blit_writes(&self) -> wgpu::RenderPassTimestampWrites<'_> {
        wgpu::RenderPassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: Some(2),
            end_of_pass_write_index: Some(3),
        }
    }

    /// Timestamp writes for the own-pass sky.
    pub(crate) fn sky_writes(&self) -> wgpu::RenderPassTimestampWrites<'_> {
        wgpu::RenderPassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: Some(4),
            end_of_pass_write_index: Some(5),
        }
    }

    /// Timestamp writes for the GTAO compute pass.
    pub(crate) fn gtao_writes(&self) -> wgpu::ComputePassTimestampWrites<'_> {
        wgpu::ComputePassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: Some(6),
            end_of_pass_write_index: Some(7),
        }
    }

    /// Resolve this frame's timestamps and copy them to staging slot
    /// `index`. Call after every pass, before the encoder finishes.
    /// `timed` says which optional passes wrote their queries; the late read
    /// ignores the rest.
    pub(crate) fn encode_resolve(
        &self,
        enc: &mut wgpu::CommandEncoder,
        index: usize,
        timed: PassesTimed,
    ) {
        let Some(slot) = self.slots.get(index) else {
            return;
        };
        slot.sky_written.store(timed.sky, Ordering::Release);
        slot.gtao_written.store(timed.gtao, Ordering::Release);
        enc.resolve_query_set(&self.queries, 0..QUERY_COUNT, &self.resolve, 0);
        enc.copy_buffer_to_buffer(&self.resolve, 0, &slot.buffer, 0, RESOLVE_BYTES);
    }

    /// Request the late read of slot `index` after the frame was
    /// submitted. The callback only flips the slot state; nothing waits.
    pub(crate) fn arm(&self, index: usize) {
        let Some(slot) = self.slots.get(index) else {
            return;
        };
        slot.state.store(PENDING, Ordering::Release);
        let state = Arc::clone(&slot.state);
        slot.buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let next = if result.is_ok() { READY } else { FAILED };
                state.store(next, Ordering::Release);
            });
    }

    /// Percentile summary of every resolved frame so far; `None` before
    /// the first sample lands.
    pub(crate) fn report(&self) -> Option<GpuTimingReport> {
        let mut forward = self.forward.samples.clone();
        let mut blit = self.blit.samples.clone();
        let mut total = self.total.samples.clone();
        // Frames that skipped the own-pass sky add no sky sample; a run with
        // no sky pass at all reports a zero sky, not a missing report.
        let mut sky = self.sky.samples.clone();
        let mut gtao = self.gtao.samples.clone();
        Some(GpuTimingReport {
            frames: u64::try_from(forward.len()).unwrap_or(u64::MAX),
            total: percentiles(&mut total)?,
            forward: percentiles(&mut forward)?,
            blit: percentiles(&mut blit)?,
            sky: percentiles(&mut sky).unwrap_or_default(),
            gtao: percentiles(&mut gtao).unwrap_or_default(),
        })
    }
}

/// Eight little-endian u64 ticks from a mapped staging buffer.
fn read_ticks(buffer: &wgpu::Buffer) -> Option<[u64; 8]> {
    let view = buffer.slice(..).get_mapped_range().ok()?;
    let mut ticks = [0u64; 8];
    for (tick, chunk) in ticks.iter_mut().zip(view.chunks_exact(8)) {
        let mut word = [0u8; 8];
        word.copy_from_slice(chunk);
        *tick = u64::from_le_bytes(word);
    }
    Some(ticks)
}
