//! Per-pass GPU timestamp helpers, shared by the frame graph and the executor.
//!
//! These are pure math plus one wgpu query-set builder. They live below both
//! owners so `graph` can size and read its own timings without depending on
//! the pass executor (OPT11 cycle break: graph used to call `crate::executor`).

/// True while the device records per-pass GPU timestamps. Never fails:
/// callers post `None` timings when false.
#[cfg(feature = "gpu")]
pub(crate) fn timestamp_supported(device: &wgpu::Device) -> bool {
    device.features().contains(wgpu::Features::TIMESTAMP_QUERY)
}

/// Tick delta to ms via the queue period (ns per tick). Pure, no wgpu
/// types, so it also compiles on the headless twin for its math tests.
#[cfg(any(feature = "gpu", test))]
pub(crate) fn ticks_to_ms(delta_ticks: u64, period: f32) -> f64 {
    let hi = u32::try_from(delta_ticks >> 32).unwrap_or(u32::MAX);
    let lo = u32::try_from(delta_ticks & 0xffff_ffff).unwrap_or(u32::MAX);
    (f64::from(hi) * 4_294_967_296.0 + f64::from(lo)) * f64::from(period) / 1_000_000.0
}

/// One pass ms from a begin/end tick pair; inverted pairs read zero. Pure.
#[cfg(any(feature = "gpu", test))]
pub(crate) fn pass_ms_from_ticks(begin_ticks: u64, end_ticks: u64, period: f32) -> f64 {
    ticks_to_ms(end_ticks.saturating_sub(begin_ticks), period)
}

/// Query pair for one compiled-order position. Pure.
#[cfg(any(feature = "gpu", test))]
pub(crate) fn query_pair(pos: usize) -> (u32, u32) {
    let begin = u32::try_from(pos.saturating_mul(2)).unwrap_or(u32::MAX);
    (begin, begin.saturating_add(1))
}

/// Query count for a submit: two timestamps per pass. Pure.
#[cfg(any(feature = "gpu", test))]
pub(crate) fn timestamp_query_count(passes: usize) -> u32 {
    u32::try_from(passes.saturating_mul(2)).unwrap_or(u32::MAX)
}

/// All-`None` timings for a submit that skipped or lost its queries. Pure.
#[cfg(any(feature = "gpu", test))]
pub(crate) fn none_gpu_timings(passes: usize) -> Vec<Option<f64>> {
    let mut rows = Vec::with_capacity(passes);
    for _ in 0..passes {
        rows.push(None);
    }
    rows
}

/// Decode a resolved buffer (`2 * passes` little-endian u64) to per-pass
/// ms; short tails and bad samples degrade to `None`. Pure.
#[cfg(any(feature = "gpu", test))]
pub(crate) fn decode_pass_bytes(bytes: &[u8], passes: usize, period: f32) -> Vec<Option<f64>> {
    let mut rows = Vec::with_capacity(passes);
    for pos in 0..passes {
        let base = pos.saturating_mul(16);
        let sample = bytes.get(base..base.saturating_add(16)).and_then(|pair| {
            let begin = u64::from_le_bytes(pair[0..8].try_into().ok()?);
            let end = u64::from_le_bytes(pair[8..16].try_into().ok()?);
            let ms = pass_ms_from_ticks(begin, end, period);
            if ms.is_finite() && ms >= 0.0 {
                Some(ms)
            } else {
                None
            }
        });
        rows.push(sample);
    }
    rows
}

/// Live per-submit timestamp state: pair set plus resolve and staging
/// buffers. The resolve buffer carries `QUERY_RESOLVE | COPY_SRC` and the
/// staging buffer `MAP_READ | COPY_DST`: `MAP_READ` only combines with its
/// opposite `COPY_DST`, so resolving straight into a mapped buffer is a
/// validation error. Evidence only, never hashed.
#[cfg(feature = "gpu")]
pub(crate) struct TimestampSetup {
    /// Pair set holding `2 * passes` timestamps.
    pub query_set: wgpu::QuerySet,
    /// Resolve destination for `resolve_query_set`.
    pub resolve_buf: wgpu::Buffer,
    /// Mapped staging copy of the resolved ticks.
    pub staging_buf: wgpu::Buffer,
    /// Timestamp count (`2 * passes`).
    pub query_count: u32,
    /// Queue timestamp period in ns per tick.
    pub period: f32,
}

/// Size the per-submit timestamp state, or `None` when the device or period
/// cannot record. `None` never fails startup: the submit records without
/// queries and posts `None` per pass.
#[cfg(feature = "gpu")]
pub(crate) fn timestamp_setup(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    passes: usize,
) -> Option<TimestampSetup> {
    let query_count = timestamp_query_count(passes);
    let period = if timestamp_supported(device) {
        queue.get_timestamp_period()
    } else {
        0.0
    };
    if query_count == 0 || !period.is_finite() || period <= 0.0 {
        return None;
    }
    let size = u64::from(query_count).saturating_mul(8);
    Some(TimestampSetup {
        query_set: device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("ts-pass-pairs"),
            ty: wgpu::QueryType::Timestamp,
            count: query_count,
        }),
        resolve_buf: device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ts-resolve"),
            size,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }),
        staging_buf: device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ts-staging"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        query_count,
        period,
    })
}

/// Read back a resolved timestamp buffer into per-pass ms. Any map failure
/// degrades to `None`: timestamps never fail the frame pixels already proved.
#[cfg(feature = "gpu")]
pub(crate) fn read_pass_ms(
    ts_buf: Option<&wgpu::Buffer>,
    ts_slice: Option<&wgpu::BufferSlice<'_>>,
    ts_rx: std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
    passes: usize,
    period: f32,
) -> Vec<Option<f64>> {
    let Some(buf) = ts_buf else {
        return none_gpu_timings(passes);
    };
    let Some(slice) = ts_slice else {
        buf.unmap();
        return none_gpu_timings(passes);
    };
    if !matches!(ts_rx.recv(), Ok(Ok(()))) {
        buf.unmap();
        return none_gpu_timings(passes);
    }
    let rows = match slice.get_mapped_range() {
        Ok(mapped) => decode_pass_bytes(&mapped, passes, period),
        Err(_) => none_gpu_timings(passes),
    };
    buf.unmap();
    rows
}
