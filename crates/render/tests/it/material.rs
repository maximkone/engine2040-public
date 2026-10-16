//! material: RL6 layered-PBR bind-bytes determinism over the public API.
//!
//! Owner: render lane; consumer is the WIN1 forward bind, which packs the
//! same chart totals as little-endian bytes under the shared look bind.
//! Units: linear reflectance totals packed as LE f32 bytes. Determinism:
//! the same chart binds the same bytes on every run. Errors: chart rows
//! must stay within `CHART_TOLERANCE` via `chart_parity`.

use engine_render::material_layer::{apply_layer, chart, chart_parity};

/// Bind image the forward pass would upload: one LE f32 per channel over
/// every chart row, in chart order. Mirrors the `submit_chart_gpu` CPU pack
/// without needing a wgpu device, so it runs headless.
fn material_bind_bytes() -> Vec<u8> {
    let rows = chart();
    let mut out = Vec::with_capacity(rows.len() * 3 * 4);
    for entry in &rows {
        let got = apply_layer(&entry.base, &entry.layer, &entry.geo).expect("valid chart row");
        for v in got {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

#[test]
fn material_bind_bytes_equal_twice() {
    assert_eq!(
        chart_parity().expect("chart parity"),
        chart().len(),
        "chart stays green so the bind covers real rows"
    );
    let a = material_bind_bytes();
    let b = material_bind_bytes();
    assert_eq!(
        a.len(),
        chart().len() * 3 * 4,
        "five rows times three channels times four bytes"
    );
    assert_eq!(a, b, "same material chart binds the same bytes twice");
    assert!(
        a.iter().any(|byte| *byte != 0),
        "bind bytes carry the chart totals, never all zero"
    );
    // Sensitivity: distinct rows bind distinctly, so the equality above is
    // not a constant pass.
    let row_bytes = 3 * 4;
    assert_ne!(
        &a[0..row_bytes],
        &a[row_bytes..row_bytes * 2],
        "cloth front and graze bind distinctly"
    );
}
