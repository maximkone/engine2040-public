//! Family table preset-wrap vectors for readable playtest output.
//!
//! Consumer: playtest readability checks and the duel capture footer.
//! Units: columns and display cells (wide glyphs count two).
//! Determinism: fixed rows and widths, no clock or threads.
//! Errors: TableError on bad widths, never a panic in render.
//!
//! PORT(Nukesor/comfy-table@main:src/table.rs preset plus wrap idea, MIT;
//! fresh Rust here, no new dependency).

use std::error::Error;
use std::fmt::{self, Display};

/// Widest line the family table may emit, in display cells.
const MAX_LINE_WIDTH: usize = 80;
/// Column widths for the four family columns, in display cells.
const COL_WIDTHS: [usize; 4] = [10, 12, 14, 16];
/// Header labels for the four family columns.
const HEADERS: [&str; 4] = ["Seat", "Fighter", "Family", "Note"];
/// Rows exercised by the wrap vectors.
const ROW_COUNT: usize = 20;

/// Border preset in the spirit of a full UTF-8 table preset.
#[derive(Clone, Copy, Debug)]
struct Preset {
    top_left: char,
    top_mid: char,
    top_right: char,
    mid_left: char,
    mid_mid: char,
    mid_right: char,
    bottom_left: char,
    bottom_mid: char,
    bottom_right: char,
    horizontal: char,
    vertical: char,
}

/// Full UTF-8 box preset used by every family render.
const UTF8_FULL: Preset = Preset {
    top_left: '\u{250C}',
    top_mid: '\u{252C}',
    top_right: '\u{2510}',
    mid_left: '\u{251C}',
    mid_mid: '\u{253C}',
    mid_right: '\u{2524}',
    bottom_left: '\u{2514}',
    bottom_mid: '\u{2534}',
    bottom_right: '\u{2518}',
    horizontal: '\u{2500}',
    vertical: '\u{2502}',
};

/// Table build failures, returned instead of panicking.
#[derive(Clone, PartialEq, Eq, Debug)]
enum TableError {
    /// A column width was zero so wrapping cannot proceed.
    ZeroWidth,
    /// The framed line would exceed the overflow cap.
    TooWide { width: usize, limit: usize },
}

impl Display for TableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroWidth => write!(f, "column width must be nonzero"),
            Self::TooWide { width, limit } => {
                write!(f, "framed width {width} exceeds limit {limit}")
            }
        }
    }
}

impl Error for TableError {}

/// True for glyphs that occupy two display cells.
fn is_wide(glyph: char) -> bool {
    let value = u32::from(glyph);
    matches!(
        value,
        0x1100..=0x115F
            | 0x2E80..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE10..=0xFE19
            | 0xFE30..=0xFE4F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F300..=0x1FAFF
            | 0x20000..=0x3FFFD
    )
}

/// Display width of one glyph: control yields zero, wide yields two.
fn glyph_width(glyph: char) -> usize {
    if glyph.is_control() {
        0
    } else if is_wide(glyph) {
        2
    } else {
        1
    }
}

/// Display width of a string, summing glyph cells.
fn display_width(text: &str) -> usize {
    text.chars().map(glyph_width).sum()
}

/// Split a long word into chunks that each fit the width.
fn split_long_word(word: &str, width: usize, out: &mut String, lines: &mut Vec<String>) {
    let mut current_width = display_width(out);
    for glyph in word.chars() {
        let w = glyph_width(glyph);
        if current_width + w > width {
            lines.push(std::mem::take(out));
            current_width = 0;
        }
        out.push(glyph);
        current_width += w;
    }
}

/// Wrap one cell into lines that each fit the width.
fn wrap_cell(cell: &str, width: usize) -> Vec<String> {
    let flat = cell.replace('\n', " ");
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in flat.split_whitespace() {
        if display_width(word) > width {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            split_long_word(word, width, &mut current, &mut lines);
            continue;
        }
        if current.is_empty() {
            current.push_str(word);
        } else if display_width(&current) + 1 + display_width(word) <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    if current.is_empty() && lines.is_empty() {
        lines.push(String::new());
    } else if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Pad a cell with trailing spaces to exactly the width.
fn pad_to(cell: &str, width: usize) -> String {
    let mut out = String::from(cell);
    let missing = width.saturating_sub(display_width(cell));
    for _ in 0..missing {
        out.push(' ');
    }
    out
}

/// Framed line width for the given column widths.
fn framed_width(col_widths: [usize; 4]) -> usize {
    let inner: usize = col_widths.iter().sum();
    inner + 3 * col_widths.len() + 1
}

/// Build one border line such as the top, middle, or bottom rule.
fn border_line(
    preset: Preset,
    left: char,
    mid: char,
    right: char,
    col_widths: [usize; 4],
) -> String {
    let mut out = String::new();
    out.push(left);
    for (index, width) in col_widths.iter().enumerate() {
        for _ in 0..width + 2 {
            out.push(preset.horizontal);
        }
        if index + 1 < col_widths.len() {
            out.push(mid);
        }
    }
    out.push(right);
    out
}

/// Render the four-column family table with wrapping and a box preset.
fn render_family_table(
    headers: [&str; 4],
    rows: &[[String; 4]],
    col_widths: [usize; 4],
) -> Result<String, TableError> {
    if col_widths.contains(&0) {
        return Err(TableError::ZeroWidth);
    }
    let width = framed_width(col_widths);
    if width > MAX_LINE_WIDTH {
        return Err(TableError::TooWide {
            width,
            limit: MAX_LINE_WIDTH,
        });
    }
    let preset = UTF8_FULL;
    let mut out = String::new();
    out.push_str(&border_line(
        preset,
        preset.top_left,
        preset.top_mid,
        preset.top_right,
        col_widths,
    ));
    out.push('\n');
    let header_cells: [String; 4] = [
        String::from(headers[0]),
        String::from(headers[1]),
        String::from(headers[2]),
        String::from(headers[3]),
    ];
    let header_wrapped: Vec<Vec<String>> = header_cells
        .iter()
        .enumerate()
        .map(|(index, cell)| wrap_cell(cell, col_widths[index]))
        .collect();
    let header_height = header_wrapped.iter().map(Vec::len).max().unwrap_or(1);
    for line in 0..header_height {
        out.push(preset.vertical);
        for index in 0..4 {
            let text = header_wrapped[index].get(line).map_or("", String::as_str);
            out.push(' ');
            out.push_str(&pad_to(text, col_widths[index]));
            out.push(' ');
            out.push(preset.vertical);
        }
        out.push('\n');
    }
    out.push_str(&border_line(
        preset,
        preset.mid_left,
        preset.mid_mid,
        preset.mid_right,
        col_widths,
    ));
    out.push('\n');
    for row in rows {
        let wrapped: Vec<Vec<String>> = row
            .iter()
            .enumerate()
            .map(|(index, cell)| wrap_cell(cell, col_widths[index]))
            .collect();
        let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
        for line in 0..height {
            out.push(preset.vertical);
            for index in 0..4 {
                let text = wrapped[index].get(line).map_or("", String::as_str);
                out.push(' ');
                out.push_str(&pad_to(text, col_widths[index]));
                out.push(' ');
                out.push(preset.vertical);
            }
            out.push('\n');
        }
    }
    out.push_str(&border_line(
        preset,
        preset.bottom_left,
        preset.bottom_mid,
        preset.bottom_right,
        col_widths,
    ));
    out.push('\n');
    Ok(out)
}

/// Twenty deterministic rows mixing wide glyphs and long overflow notes.
fn family_rows() -> Vec<[String; 4]> {
    let fighters = [
        "Ryu \u{9F8D}",
        "Ken \u{1F525}",
        "Chun \u{6625}\u{9E97}",
        "Guile \u{5BB6}\u{65CF}",
        "Zangief \u{529B}",
        "Dhalsim \u{708E}",
        "Blanka \u{91CE}",
        "Honda \u{76F8}\u{64B2}",
    ];
    let families = [
        "Fuji \u{5BCC}\u{58EB}",
        "Sora \u{7A7A}",
        "Kaze \u{98A8}",
        "Umi \u{6D77}",
        "Yama \u{5C71}",
        "Kawa \u{5DDD}",
        "Hoshi \u{661F}",
        "Tsuki \u{6708}",
    ];
    let notes = [
        "counter hit into corner carry with wall splat followup pressure",
        "whiff punish with long reach normal then safe knockdown setup",
        "fireball war zoning with patient walk and block punish routine",
        "dragon punch anti air read with meter burn extension combo path",
        "throw loop pressure with shimmy bait and late tech catch option",
        "crossup jump mix with empty low short into super confirm window",
    ];
    let mut rows = Vec::with_capacity(ROW_COUNT);
    for index in 0..ROW_COUNT {
        let seat = format!("Seat {:02}", index + 1);
        let fighter = String::from(fighters[index % fighters.len()]);
        let family = String::from(families[(index * 3) % families.len()]);
        let note = String::from(notes[(index * 5) % notes.len()]);
        rows.push([seat, fighter, family, note]);
    }
    rows
}

/// Count lines whose display width differs from the first line.
fn misaligns(rendered: &str) -> usize {
    let mut widths = rendered.lines().map(display_width);
    let Some(first) = widths.next() else {
        return 0;
    };
    let mut count = 0;
    for w in widths {
        if w != first {
            count += 1;
        }
    }
    count
}

/// Widest display width across all rendered lines.
fn max_line_width(rendered: &str) -> usize {
    rendered.lines().map(display_width).max().unwrap_or(0)
}

#[test]
fn family_table_has_zero_misaligns_over_20_wide_rows() {
    let rows = family_rows();
    assert_eq!(rows.len(), ROW_COUNT, "fixture keeps twenty rows");
    let rendered = render_family_table(HEADERS, &rows, COL_WIDTHS).expect("family table renders");
    let bad = misaligns(&rendered);
    println!("rows={} misaligns={bad}", rows.len());
    assert_eq!(bad, 0, "wide cells stay aligned:\n{rendered}");
}

#[test]
fn family_table_wraps_overflow_at_80_cols() {
    let rows = family_rows();
    let rendered = render_family_table(HEADERS, &rows, COL_WIDTHS).expect("family table renders");
    let widest = max_line_width(&rendered);
    let physical = rendered.lines().count();
    println!("widest={widest} physical_lines={physical}");
    assert!(
        widest <= MAX_LINE_WIDTH,
        "overflow wraps at 80: widest={widest}\n{rendered}"
    );
    assert!(
        physical > ROW_COUNT,
        "long notes wrap into extra lines: physical={physical}"
    );
}

#[test]
fn family_table_rejects_zero_width() {
    let rows = family_rows();
    let err = render_family_table(HEADERS, &rows, [10, 0, 14, 16]).unwrap_err();
    println!("rejected={err}");
    assert_eq!(err, TableError::ZeroWidth);
}
