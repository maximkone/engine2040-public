//! chart: the live material-chart target size and its edge guard (RL2).

use super::*;

/// Size the chart target for a look viewport in pixels: identity while both
/// edges fit [`IBL_CHART_MAX_EDGE`], otherwise a uniform downscale by
/// `edge / long_edge` so the long edge lands exactly on the cap and the
/// aspect is preserved (1600x900 becomes 256x144). Pure integer math in
/// fixed order; the same viewport gives the same target on every run.
/// Zero-area viewports refuse with [`IblError::Invalid`]; see
/// [`check_chart_target_size`] for the direct target guard.
pub fn chart_target_size(viewport_w: u32, viewport_h: u32) -> Result<(u32, u32), IblError> {
    if viewport_w == 0 || viewport_h == 0 {
        return Err(IblError::Invalid(format!(
            "ibl chart viewport {viewport_w}x{viewport_h} has zero area"
        )));
    }
    if viewport_w <= IBL_CHART_MAX_EDGE && viewport_h <= IBL_CHART_MAX_EDGE {
        return Ok((viewport_w, viewport_h));
    }
    let wide = u64::from(viewport_w);
    let high = u64::from(viewport_h);
    let edge = u64::from(IBL_CHART_MAX_EDGE);
    let long = wide.max(high);
    let wide_scaled = wide
        .checked_mul(edge)
        .ok_or_else(|| IblError::Invalid("ibl chart viewport product overflows".into()))?;
    let high_scaled = high
        .checked_mul(edge)
        .ok_or_else(|| IblError::Invalid("ibl chart viewport product overflows".into()))?;
    let clamped_wide = (wide_scaled / long).clamp(1, edge);
    let clamped_high = (high_scaled / long).clamp(1, edge);
    let w = u32::try_from(clamped_wide)
        .map_err(|_| IblError::Invalid("ibl chart width exceeds u32".into()))?;
    let h = u32::try_from(clamped_high)
        .map_err(|_| IblError::Invalid("ibl chart height exceeds u32".into()))?;
    Ok((w, h))
}

/// Guard the chart target itself in pixels: both edges must be nonzero and
/// within [`IBL_CHART_MAX_EDGE`]. The viewport scaler above clamps window
/// sizes into this bound; direct target sizes over the edge refuse loudly
/// with [`IblError::Invalid`] before any texture sizes, so the refusal never
/// reaches the presenter validation scope.
pub fn check_chart_target_size(w: u32, h: u32) -> Result<(), IblError> {
    if w == 0 || h == 0 {
        return Err(IblError::Invalid(format!(
            "ibl chart target {w}x{h} has zero area"
        )));
    }
    if w > IBL_CHART_MAX_EDGE || h > IBL_CHART_MAX_EDGE {
        return Err(IblError::Invalid(format!(
            "ibl chart target {w}x{h} exceeds edge {IBL_CHART_MAX_EDGE}"
        )));
    }
    Ok(())
}
