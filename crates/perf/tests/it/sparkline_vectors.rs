// Donor: ratatui (MIT, https://github.com/ratatui/ratatui), sparkline widget.
// NINE_LEVELS min-max bar mapping idea only; fresh std-only implementation,
// no donor bytes copied.
//! Std-only ASCII/Unicode sparkline over BENCH CSV rows.
//!
//! Consumer: `engine-perf` bench ledger review (text fallback when no TUI).
//! Owner: this test module. Units: samples in milliseconds, levels are
//! indices 0..=8. Determinism: pure function of the input slice; latest
//! sample renders last. Errors: unparseable CSV cells are skipped, an
//! absent kernel renders as empty.
/// Nine render levels, lowest first, highest last.
///
/// Index 0 is a blank (quiescent sample), index 8 is the full block (row
/// maximum). Nine levels keep the min-max scaler branchless.
const NINE_LEVELS: [char; 9] = [
    ' ', '\u{2581}', '\u{2582}', '\u{2583}', '\u{2584}', '\u{2585}', '\u{2586}', '\u{2587}',
    '\u{2588}',
];
/// Flat-line level used when `max <= min` (no range to scale).
const FLAT_LEVEL: char = '\u{2584}';
/// Map one sample to a level index 0..=8 given the row min/max.
///
/// Flat input (`max <= min`) maps to the middle level so the row stays
/// visible instead of collapsing to blank.
fn level_for(value: f64, min: f64, max: f64) -> char {
    if !(value.is_finite() && min.is_finite() && max.is_finite()) {
        return ' ';
    }
    if max <= min {
        return FLAT_LEVEL;
    }
    let clamped = value.clamp(min, max);
    let ratio = (clamped - min) / (max - min);
    let scaled = (ratio * 8.0).round() as usize;
    NINE_LEVELS[scaled.min(8)]
}
/// Render one sparkline row: oldest sample first, latest sample last.
///
/// An absent (empty) row renders as empty string.
fn sparkline(samples: &[f64]) -> String {
    if samples.is_empty() {
        return String::new();
    }
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for sample in samples {
        if sample.is_finite() {
            if *sample < min {
                min = *sample;
            }
            if *sample > max {
                max = *sample;
            }
        }
    }
    if !min.is_finite() || !max.is_finite() {
        return String::new();
    }
    samples.iter().map(|v| level_for(*v, min, max)).collect()
}
/// One parsed BENCH CSV row: kernel name plus its history samples.
#[derive(Debug, Clone, PartialEq)]
struct BenchHistory {
    kernel: String,
    samples: Vec<f64>,
}
/// Parse a minimal BENCH CSV: header `kernel,samples` then one row per
/// kernel with decimal samples. Blank lines are skipped; unparseable cells
/// are skipped so one bad cell never drops the row.
fn parse_bench_csv(csv: &str) -> Vec<BenchHistory> {
    let mut rows = Vec::new();
    for (index, line) in csv.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if index == 0 {
            continue;
        }
        let mut cells = line.split(',');
        let kernel = cells.next().unwrap_or("").trim().to_string();
        if kernel.is_empty() {
            continue;
        }
        let mut samples = Vec::new();
        for cell in cells {
            let cell = cell.trim();
            if cell.is_empty() {
                continue;
            }
            if let Ok(value) = cell.parse::<f64>() {
                samples.push(value);
            }
        }
        rows.push(BenchHistory { kernel, samples });
    }
    rows
}
/// Render the sparkline for one kernel; an absent kernel renders empty.
fn sparkline_for_kernel(histories: &[BenchHistory], kernel: &str) -> String {
    histories
        .iter()
        .find(|row| row.kernel == kernel)
        .map(|row| sparkline(&row.samples))
        .unwrap_or_default()
}
/// Build a deterministic BENCH CSV fixture: 12 kernels, 60 samples each.
fn fixture_csv_twelve_by_sixty() -> String {
    let kernels = [
        "tick", "physics", "render", "ai", "combat", "net", "audio", "ui", "input", "save",
        "scene", "spatial",
    ];
    let mut out = String::from("kernel,samples\n");
    for (kernel_index, kernel) in kernels.iter().enumerate() {
        out.push_str(kernel);
        for sample_index in 0..60 {
            let value = (kernel_index as f64) * 10.0 + (sample_index as f64) * 0.5;
            out.push_str(&format!(",{value:.1}"));
        }
        out.push('\n');
    }
    out
}
#[test]
fn sixty_sample_row_keeps_shape_latest_last() {
    let histories = parse_bench_csv(&fixture_csv_twelve_by_sixty());
    assert_eq!(histories.len(), 12);
    let row = histories.iter().find(|row| row.kernel == "tick").unwrap();
    assert_eq!(row.samples.len(), 60);
    let rendered = sparkline(&row.samples);
    assert_eq!(rendered.chars().count(), 60);
    assert!(
        rendered.starts_with(' '),
        "oldest (min) renders blank, got {rendered}"
    );
    assert!(
        rendered.ends_with('\u{2588}'),
        "latest (max) renders top block, got {rendered}"
    );
}
#[test]
fn row_max_maps_to_top_block() {
    let rendered = sparkline(&[1.0, 2.0, 3.0]);
    let levels: Vec<char> = rendered.chars().collect();
    assert_eq!(levels.len(), 3);
    assert_eq!(levels[0], ' ');
    assert_eq!(
        levels[2], '\u{2588}',
        "row max must map to top block, got {rendered}"
    );
}
#[test]
fn single_spike_position_is_exact() {
    let rendered = sparkline(&[0.0, 0.0, 10.0, 0.0, 0.0]);
    assert_eq!(rendered, "  \u{2588}  ");
}
#[test]
fn empty_row_renders_empty() {
    assert_eq!(sparkline(&[]), "");
    assert_eq!(sparkline(&[f64::NAN, f64::INFINITY]), "");
    let histories = parse_bench_csv(&fixture_csv_twelve_by_sixty());
    assert_eq!(sparkline_for_kernel(&histories, "no-such-kernel"), "");
    assert_eq!(sparkline_for_kernel(&[], "tick"), "");
}
#[test]
fn twelve_kernels_render_absent_row_empty() {
    let histories = parse_bench_csv(&fixture_csv_twelve_by_sixty());
    let kernels = [
        "tick", "physics", "render", "ai", "combat", "net", "audio", "ui", "input", "save",
        "scene", "spatial",
    ];
    let mut rendered_count = 0;
    for kernel in kernels {
        let rendered = sparkline_for_kernel(&histories, kernel);
        assert_eq!(
            rendered.chars().count(),
            60,
            "kernel {kernel} must render 60 levels"
        );
        assert!(
            rendered.contains('\u{2588}'),
            "kernel {kernel} must contain its max block"
        );
        rendered_count += 1;
    }
    assert_eq!(rendered_count, 12);
    assert_eq!(
        sparkline_for_kernel(&histories, "absent-kernel"),
        "",
        "absent row renders empty"
    );
}
