// Donor: sharkdp/hyperfine, MIT, https://github.com/sharkdp/hyperfine
// (src/benchmark/mod.rs, src/export/markdown.rs). Fresh reimplementation, no donor bytes.
// Warmup-discard plus markdown/JSON bench-ledger export for the bench ledger line.

/// One bench-ledger row: benchmark name plus summary stats in milliseconds.
#[derive(Debug, Clone, PartialEq)]
struct BenchRow {
    name: String,
    mean_ms: f64,
    p50_ms: f64,
    p95_ms: f64,
}

/// Mean with the first (cold/warmup) sample discarded.
///
/// Returns 0.0 when fewer than two samples are present (nothing remains
/// after the warmup discard).
fn mean_without_warmup(samples: &[f64]) -> f64 {
    let kept = if samples.len() <= 1 {
        &[]
    } else {
        &samples[1..]
    };
    if kept.is_empty() {
        return 0.0;
    }
    let sum: f64 = kept.iter().sum();
    sum / (kept.len() as f64)
}

/// Render ledger rows as a markdown table with header plus divider.
fn to_markdown(rows: &[BenchRow]) -> String {
    let mut out = String::from("| name | mean_ms | p50_ms | p95_ms |\n| --- | --- | --- | --- |\n");
    for row in rows {
        out.push_str(&format!(
            "| {} | {:.3} | {:.3} | {:.3} |\n",
            row.name, row.mean_ms, row.p50_ms, row.p95_ms
        ));
    }
    out
}

fn escape_name(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            BACKSLASH => out.push_str(DOUBLE_BACKSLASH),
            QUOTE => out.push_str(ESCAPED_QUOTE),
            _ => out.push(ch),
        }
    }
    out
}

const BACKSLASH: char = '\x5C';
const QUOTE: char = '\x22';
const DOUBLE_BACKSLASH: &str = "\x5C\x5C";
const ESCAPED_QUOTE: &str = "\x5C\x22";

fn unescape_name(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(ch) = chars.next() {
        if ch == BACKSLASH {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(next) => out.push(next),
                None => out.push(BACKSLASH),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// Encode ledger rows as a compact JSON array (no external dependency).
fn to_json(rows: &[BenchRow]) -> String {
    let mut out = String::from("[");
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\x22name\x22:\x22{}\x22,\x22mean_ms\x22:{},\x22p50_ms\x22:{},\x22p95_ms\x22:{}}}",
            escape_name(&row.name),
            row.mean_ms,
            row.p50_ms,
            row.p95_ms
        ));
    }
    out.push(']');
    out
}

fn parse_quoted_name(from: &str) -> Option<(String, &str)> {
    let bytes = from.as_bytes();
    let mut end = None;
    let mut escaped = false;
    for (i, byte) in bytes.iter().enumerate() {
        if escaped {
            escaped = false;
        } else if *byte == 0x5C {
            escaped = true;
        } else if *byte == 0x22 {
            end = Some(i);
            break;
        }
    }
    let cut = end?;
    let raw = &from[..cut];
    let rest = &from[cut + 1..];
    Some((unescape_name(raw), rest))
}

fn parse_number(from: &str) -> Option<(f64, &str)> {
    let end = from.find([',', '}', ']']).unwrap_or(from.len());
    let value: f64 = from[..end].trim().parse().ok()?;
    Some((value, &from[end..]))
}

fn quoted_key(key: &str) -> String {
    let mut out = String::with_capacity(key.len() + 2);
    out.push(QUOTE);
    out.push_str(key);
    out.push(QUOTE);
    out
}

/// Decode to_json output back into ledger rows.
///
/// Skips objects that do not carry all four fields; returns every
/// well-formed row in order.
fn parse_json(json: &str) -> Vec<BenchRow> {
    let mut rows = Vec::new();
    let mut rest = json;
    let name_key = quoted_key("name");
    let mean_key = quoted_key("mean_ms");
    let p50_key = quoted_key("p50_ms");
    let p95_key = quoted_key("p95_ms");
    while let Some(name_at) = rest.find(name_key.as_str()) {
        rest = &rest[name_at + name_key.len()..];
        let colon = match rest.find(':') {
            Some(i) => i,
            None => break,
        };
        rest = rest[colon + 1..].trim_start();
        if !rest.starts_with(QUOTE) {
            continue;
        }
        let (name, after_name) = match parse_quoted_name(&rest[1..]) {
            Some(pair) => pair,
            None => break,
        };
        rest = after_name;
        let mean_at = match rest.find(mean_key.as_str()) {
            Some(i) => i,
            None => continue,
        };
        rest = &rest[mean_at + mean_key.len()..];
        let colon = match rest.find(':') {
            Some(i) => i,
            None => continue,
        };
        let (mean_ms, after_mean) = match parse_number(&rest[colon + 1..]) {
            Some(pair) => pair,
            None => continue,
        };
        rest = after_mean;
        let p50_at = match rest.find(p50_key.as_str()) {
            Some(i) => i,
            None => continue,
        };
        rest = &rest[p50_at + p50_key.len()..];
        let colon = match rest.find(':') {
            Some(i) => i,
            None => continue,
        };
        let (p50_ms, after_p50) = match parse_number(&rest[colon + 1..]) {
            Some(pair) => pair,
            None => continue,
        };
        rest = after_p50;
        let p95_at = match rest.find(p95_key.as_str()) {
            Some(i) => i,
            None => continue,
        };
        rest = &rest[p95_at + p95_key.len()..];
        let colon = match rest.find(':') {
            Some(i) => i,
            None => continue,
        };
        let (p95_ms, after_p95) = match parse_number(&rest[colon + 1..]) {
            Some(pair) => pair,
            None => continue,
        };
        rest = after_p95;
        rows.push(BenchRow {
            name,
            mean_ms,
            p50_ms,
            p95_ms,
        });
    }
    rows
}

#[test]
fn markdown_golden_equals_expected_string() {
    let rows = vec![
        BenchRow {
            name: "tick".to_string(),
            mean_ms: 1.23456,
            p50_ms: 1.2,
            p95_ms: 1.5,
        },
        BenchRow {
            name: "render".to_string(),
            mean_ms: 2.0,
            p50_ms: 2.0,
            p95_ms: 2.5,
        },
    ];
    let expected = "| name | mean_ms | p50_ms | p95_ms |\n\
| --- | --- | --- | --- |\n\
| tick | 1.235 | 1.200 | 1.500 |\n\
| render | 2.000 | 2.000 | 2.500 |\n";
    assert_eq!(to_markdown(&rows), expected);
}

#[test]
fn json_roundtrip_three_rows() {
    let rows = vec![
        BenchRow {
            name: "tick".to_string(),
            mean_ms: 1.25,
            p50_ms: 1.2,
            p95_ms: 1.5,
        },
        BenchRow {
            name: "render".to_string(),
            mean_ms: 2.5,
            p50_ms: 2.4,
            p95_ms: 2.9,
        },
        BenchRow {
            name: "physics".to_string(),
            mean_ms: 0.75,
            p50_ms: 0.7,
            p95_ms: 0.9,
        },
    ];
    let encoded = to_json(&rows);
    let decoded = parse_json(&encoded);
    assert_eq!(decoded.len(), 3);
    assert_eq!(decoded, rows);
    println!("bench_export passed: json roundtrip 3/3 rows");
}

#[test]
fn warmup_discard_excludes_first_cold_sample() {
    let samples = [100.0, 10.0, 12.0, 14.0];
    let without_warmup = mean_without_warmup(&samples);
    assert!(
        (without_warmup - 12.0).abs() < 1e-9,
        "expected 12.0, got {without_warmup}"
    );
    let with_warmup: f64 = samples.iter().sum::<f64>() / (samples.len() as f64);
    assert!(
        (with_warmup - without_warmup).abs() > 1e-9,
        "warmup sample must change the mean"
    );
}
