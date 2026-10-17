//! WebGPU default-limit gate over the shipped WGSL.
//!
//! The web target binds each shader under the WebGPU default limits. This
//! test reads every `.wgsl` file under `src/shaders` and parses it by hand:
//! comments are dropped, each `@group(N) @binding(M) var ...` declaration is
//! collected, and each `@compute`, `@vertex` and `@fragment` entry point is
//! found. The counts are an upper bound: every declared binding of a file is
//! charged to each entry point of that file, so a binding that only one stage
//! reads still counts toward the others. A file with no entry point is a chunk
//! that a module includes; its own declarations are checked alone, which
//! bounds from below every module that includes it.
//!
//! Vertex inputs are the `@location` members of each vertex entry's
//! parameters. `maxVertexBuffers` is not checked here: the host layout picks
//! the buffer count, and the shader text does not carry it.

#![cfg(test)]

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// WebGPU default limits that each shader stage must fit.
const MAX_BIND_GROUPS: usize = 4;
const MAX_SAMPLED_TEXTURES_PER_STAGE: usize = 16;
const MAX_SAMPLERS_PER_STAGE: usize = 16;
const MAX_STORAGE_BUFFERS_PER_STAGE: usize = 8;
const MAX_STORAGE_TEXTURES_PER_STAGE: usize = 4;
const MAX_UNIFORM_BUFFERS_PER_STAGE: usize = 12;
const MAX_VERTEX_ATTRIBUTES: usize = 16;

/// Shaders over a limit on purpose, as `(file under shaders/, reason)`. A row
/// whose file is within the limits fails the test, so an exception cannot go
/// stale without the test saying so.
const EXCEPTIONS: &[(&str, &str)] = &[];

/// One WGSL token: a run of word characters, or a single symbol.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    Word(String),
    Sym(char),
}

impl Tok {
    fn is_sym(&self, c: char) -> bool {
        matches!(self, Tok::Sym(s) if *s == c)
    }

    fn is_word(&self, w: &str) -> bool {
        matches!(self, Tok::Word(s) if s.as_str() == w)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Compute,
    Vertex,
    Fragment,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Stage::Compute => "compute",
            Stage::Vertex => "vertex",
            Stage::Fragment => "fragment",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Uniform,
    StorageBuffer,
    StorageTexture,
    SampledTexture,
    Sampler,
}

#[derive(Debug)]
struct Binding {
    name: String,
    group: usize,
    slot: usize,
    kind: Kind,
}

#[derive(Debug)]
struct Entry {
    name: String,
    stage: Stage,
    vertex_inputs: usize,
}

#[derive(Debug, Default)]
struct Module {
    bindings: Vec<Binding>,
    entries: Vec<Entry>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Counts {
    storage_buffers: usize,
    storage_textures: usize,
    sampled_textures: usize,
    samplers: usize,
    uniform_buffers: usize,
    /// Highest `@group` index used, plus one: the layout slots the stage needs.
    bind_groups: usize,
    vertex_attributes: usize,
}

struct Row {
    entry: String,
    stage: &'static str,
    counts: Counts,
}

impl Module {
    /// One row per entry point, or one `chunk` row for a file with none.
    fn rows(&self) -> Vec<Row> {
        if self.entries.is_empty() {
            return vec![Row {
                entry: "(chunk)".to_string(),
                stage: "chunk",
                counts: self.counts(0),
            }];
        }
        self.entries
            .iter()
            .map(|e| Row {
                entry: e.name.clone(),
                stage: e.stage.name(),
                counts: self.counts(e.vertex_inputs),
            })
            .collect()
    }

    fn counts(&self, vertex_attributes: usize) -> Counts {
        let mut c = Counts {
            vertex_attributes,
            ..Counts::default()
        };
        for b in &self.bindings {
            match b.kind {
                Kind::Uniform => c.uniform_buffers += 1,
                Kind::StorageBuffer => c.storage_buffers += 1,
                Kind::StorageTexture => c.storage_textures += 1,
                Kind::SampledTexture => c.sampled_textures += 1,
                Kind::Sampler => c.samplers += 1,
            }
            c.bind_groups = c.bind_groups.max(b.group + 1);
        }
        c
    }
}

/// Splits WGSL into tokens, dropping `//` lines and nested `/* */` blocks.
fn lex(src: &str) -> Vec<Tok> {
    let chars: Vec<char> = src.chars().collect();
    let at = |i: usize| chars.get(i).copied();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '/' && at(i + 1) == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && at(i + 1) == Some('*') {
            let mut depth = 0usize;
            while i < chars.len() {
                if chars[i] == '/' && at(i + 1) == Some('*') {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && at(i + 1) == Some('/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
        } else if c.is_alphanumeric() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.push(Tok::Word(chars[start..i].iter().collect()));
        } else {
            if !c.is_whitespace() {
                out.push(Tok::Sym(c));
            }
            i += 1;
        }
    }
    out
}

/// Reads one WGSL module: its resource declarations and its entry points.
fn parse(src: &str) -> Result<Module, String> {
    let toks = lex(src);
    let structs = struct_locations(&toks)?;
    let mut module = Module::default();
    let mut pending = None;
    for (i, tok) in toks.iter().enumerate() {
        if tok.is_sym(';') || tok.is_sym('}') {
            pending = None;
        } else if tok.is_sym('@') {
            if let Some(Tok::Word(w)) = toks.get(i + 1) {
                pending = match w.as_str() {
                    "compute" => Some(Stage::Compute),
                    "vertex" => Some(Stage::Vertex),
                    "fragment" => Some(Stage::Fragment),
                    _ => pending,
                };
            }
        } else if tok.is_word("var") {
            module.bindings.extend(binding_at(&toks, i)?);
        } else if tok.is_word("fn") {
            if let (Some(Tok::Word(name)), Some(stage)) = (toks.get(i + 1), pending.take()) {
                module
                    .entries
                    .push(entry_point(&toks, i + 2, name, stage, &structs)?);
            }
        }
    }
    for (i, b) in module.bindings.iter().enumerate() {
        if module.bindings[..i]
            .iter()
            .any(|o| o.group == b.group && o.slot == b.slot)
        {
            return Err(format!(
                "{} reuses @group({}) @binding({})",
                b.name, b.group, b.slot
            ));
        }
    }
    Ok(module)
}

/// The binding declared by the `var` at `var_at`, when its attributes carry
/// `@group` and `@binding`. A `var` without either is a local and is skipped.
fn binding_at(toks: &[Tok], var_at: usize) -> Result<Option<Binding>, String> {
    let (mut group, mut slot) = (None, None);
    let mut j = var_at;
    while j > 0 && toks[j - 1].is_sym(')') {
        let close = j - 1;
        let open = matching_back(toks, close).ok_or("unbalanced attribute")?;
        if open < 2 || !toks[open - 2].is_sym('@') {
            break;
        }
        let Tok::Word(attr) = &toks[open - 1] else {
            break;
        };
        match attr.as_str() {
            "group" => group = Some(index_arg(&toks[open + 1..close], attr)?),
            "binding" => slot = Some(index_arg(&toks[open + 1..close], attr)?),
            _ => {}
        }
        j = open - 2;
    }
    if group.is_none() && slot.is_none() {
        return Ok(None);
    }

    let mut k = var_at + 1;
    let mut space = None;
    if toks.get(k).is_some_and(|t| t.is_sym('<')) {
        let end = (k..toks.len())
            .find(|&m| toks[m].is_sym('>'))
            .ok_or("unclosed address space")?;
        space = toks[k + 1..end].iter().find_map(|t| match t {
            Tok::Word(w) => Some(w.as_str()),
            Tok::Sym(_) => None,
        });
        k = end + 1;
    }
    let Some(Tok::Word(name)) = toks.get(k) else {
        return Err(format!("var at token {var_at} has no name"));
    };
    if !toks.get(k + 1).is_some_and(|t| t.is_sym(':')) {
        return Err(format!("{name}: resource declared without a type"));
    }
    let Some(Tok::Word(ty)) = toks.get(k + 2) else {
        return Err(format!("{name}: resource declared without a type"));
    };
    let (Some(group), Some(slot)) = (group, slot) else {
        return Err(format!("{name}: needs both @group and @binding"));
    };
    let kind = match (space, ty.as_str()) {
        (Some("uniform"), _) => Kind::Uniform,
        (Some("storage"), _) => Kind::StorageBuffer,
        (Some(other), _) => {
            return Err(format!("{name}: address space `{other}` is not a binding"));
        }
        (None, t) if t.starts_with("texture_storage_") => Kind::StorageTexture,
        (None, t) if t.starts_with("texture_") => Kind::SampledTexture,
        (None, "sampler" | "sampler_comparison") => Kind::Sampler,
        (None, t) => return Err(format!("{name}: unknown resource type `{t}`")),
    };
    Ok(Some(Binding {
        name: name.clone(),
        group,
        slot,
        kind,
    }))
}

fn index_arg(args: &[Tok], attr: &str) -> Result<usize, String> {
    match args {
        [Tok::Word(w)] => w
            .parse::<usize>()
            .map_err(|_| format!("@{attr}({w}) is not an index")),
        _ => Err(format!("@{attr} takes one literal index")),
    }
}

/// One entry point: its stage and, for a vertex entry, its `@location` inputs.
/// `open` is the index of the `(` after the function name.
fn entry_point(
    toks: &[Tok],
    open: usize,
    name: &str,
    stage: Stage,
    structs: &HashMap<String, usize>,
) -> Result<Entry, String> {
    let close =
        matching(toks, open).ok_or_else(|| format!("fn {name}: no closed parameter list"))?;
    let vertex_inputs = match stage {
        Stage::Vertex => params_locations(&toks[open + 1..close], structs),
        Stage::Compute | Stage::Fragment => 0,
    };
    Ok(Entry {
        name: name.to_string(),
        stage,
        vertex_inputs,
    })
}

/// `@location` count per struct name, from each struct body.
fn struct_locations(toks: &[Tok]) -> Result<HashMap<String, usize>, String> {
    let mut out = HashMap::new();
    for (i, tok) in toks.iter().enumerate() {
        if !tok.is_word("struct") {
            continue;
        }
        let (Some(Tok::Word(name)), Some(brace)) = (toks.get(i + 1), toks.get(i + 2)) else {
            continue;
        };
        if !brace.is_sym('{') {
            continue;
        }
        let end = matching(toks, i + 2).ok_or_else(|| format!("struct {name}: body not closed"))?;
        out.insert(name.clone(), count_locations(&toks[i + 2..=end]));
    }
    Ok(out)
}

/// `@location` inputs of a parameter list: a parameter's own `@location`, or
/// the members of its struct type.
fn params_locations(params: &[Tok], structs: &HashMap<String, usize>) -> usize {
    split_params(params)
        .into_iter()
        .map(|param| {
            let via_type = param
                .iter()
                .position(|t| t.is_sym(':'))
                .and_then(|colon| match param.get(colon + 1) {
                    Some(Tok::Word(ty)) => structs.get(ty).copied(),
                    _ => None,
                })
                .unwrap_or(0);
            count_locations(param) + via_type
        })
        .sum()
}

/// Splits a parameter list at the commas that sit outside `()` and `<>`.
fn split_params(params: &[Tok]) -> Vec<&[Tok]> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (k, t) in params.iter().enumerate() {
        match t {
            Tok::Sym('(' | '<') => depth += 1,
            Tok::Sym(')' | '>') => depth -= 1,
            Tok::Sym(',') if depth == 0 => {
                out.push(&params[start..k]);
                start = k + 1;
            }
            _ => {}
        }
    }
    if start < params.len() {
        out.push(&params[start..]);
    }
    out
}

fn count_locations(toks: &[Tok]) -> usize {
    toks.windows(2)
        .filter(|w| w[0].is_sym('@') && w[1].is_word("location"))
        .count()
}

/// Index of the `(` that closes at `close`, scanning backwards.
fn matching_back(toks: &[Tok], close: usize) -> Option<usize> {
    let mut depth = 0usize;
    for k in (0..=close).rev() {
        if toks[k].is_sym(')') {
            depth += 1;
        } else if toks[k].is_sym('(') {
            depth -= 1;
            if depth == 0 {
                return Some(k);
            }
        }
    }
    None
}

/// Index of the `)` or `}` that closes the `(` or `{` at `open`.
fn matching(toks: &[Tok], open: usize) -> Option<usize> {
    let Tok::Sym(open_ch) = *toks.get(open)? else {
        return None;
    };
    let close_ch = match open_ch {
        '(' => ')',
        '{' => '}',
        _ => return None,
    };
    let mut depth = 0usize;
    for (k, tok) in toks.iter().enumerate().skip(open) {
        if tok.is_sym(open_ch) {
            depth += 1;
        } else if tok.is_sym(close_ch) {
            depth -= 1;
            if depth == 0 {
                return Some(k);
            }
        }
    }
    None
}

/// Each limit a counted row exceeds, as `what count > max`.
fn over_limits(c: &Counts) -> Vec<String> {
    [
        (
            "storage buffers",
            c.storage_buffers,
            MAX_STORAGE_BUFFERS_PER_STAGE,
        ),
        (
            "storage textures",
            c.storage_textures,
            MAX_STORAGE_TEXTURES_PER_STAGE,
        ),
        (
            "sampled textures",
            c.sampled_textures,
            MAX_SAMPLED_TEXTURES_PER_STAGE,
        ),
        ("samplers", c.samplers, MAX_SAMPLERS_PER_STAGE),
        (
            "uniform buffers",
            c.uniform_buffers,
            MAX_UNIFORM_BUFFERS_PER_STAGE,
        ),
        ("bind groups", c.bind_groups, MAX_BIND_GROUPS),
        (
            "vertex attributes",
            c.vertex_attributes,
            MAX_VERTEX_ATTRIBUTES,
        ),
    ]
    .into_iter()
    .filter(|&(_, count, max)| count > max)
    .map(|(what, count, max)| format!("{what} {count} > {max}"))
    .collect()
}

fn table_line(file: &str, row: &Row, verdict: &str) -> String {
    let c = &row.counts;
    format!(
        "{file:<30} {:<18} {:<8} {:>5} {:>5} {:>5} {:>5} {:>5} {:>5} {:>5}  {verdict}\n",
        row.entry,
        row.stage,
        c.storage_buffers,
        c.storage_textures,
        c.sampled_textures,
        c.samplers,
        c.uniform_buffers,
        c.bind_groups,
        c.vertex_attributes,
    )
}

fn collect_wgsl(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
            .path();
        if path.is_dir() {
            collect_wgsl(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "wgsl") {
            out.push(path);
        }
    }
}

fn shader_name(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[test]
fn shaders_fit_webgpu_default_limits() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("shaders");
    let mut files = Vec::new();
    collect_wgsl(&root, &mut files);
    files.sort();
    assert!(!files.is_empty(), "no .wgsl under {}", root.display());

    let mut table = format!(
        "{:<30} {:<18} {:<8} {:>5} {:>5} {:>5} {:>5} {:>5} {:>5} {:>5}  verdict\n",
        "file", "entry", "stage", "stor", "stex", "tex", "samp", "unif", "grp", "attr"
    );
    let mut problems = Vec::new();
    let mut breached_files = Vec::new();
    let mut entry_points = 0;
    for path in &files {
        let file = shader_name(&root, path);
        let src = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {file}: {e}"));
        let module = parse(&src).unwrap_or_else(|e| panic!("parse {file}: {e}"));
        let excepted = EXCEPTIONS.iter().any(|&(name, _)| name == file.as_str());
        for row in module.rows() {
            entry_points += 1;
            let hits = over_limits(&row.counts);
            let verdict = if hits.is_empty() {
                "ok".to_string()
            } else {
                breached_files.push(file.clone());
                if excepted {
                    format!("exception: {}", hits.join(", "))
                } else {
                    problems.push(format!("{file} [{}]: {}", row.entry, hits.join(", ")));
                    format!("OVER: {}", hits.join(", "))
                }
            };
            table.push_str(&table_line(&file, &row, &verdict));
        }
    }
    for &(name, reason) in EXCEPTIONS {
        if !breached_files.iter().any(|f| f.as_str() == name) {
            problems.push(format!(
                "stale exception {name} ({reason}): within the limits or missing"
            ));
        }
    }
    println!("{table}");
    println!(
        "columns: stor storage buffers (max {MAX_STORAGE_BUFFERS_PER_STAGE}), stex storage textures ({MAX_STORAGE_TEXTURES_PER_STAGE}), tex sampled textures ({MAX_SAMPLED_TEXTURES_PER_STAGE}), samp samplers ({MAX_SAMPLERS_PER_STAGE}), unif uniform buffers ({MAX_UNIFORM_BUFFERS_PER_STAGE}), grp bind groups ({MAX_BIND_GROUPS}), attr vertex @location inputs ({MAX_VERTEX_ATTRIBUTES})"
    );
    println!(
        "{} shaders, {entry_points} entry points or chunks, {} exception(s) listed",
        files.len(),
        EXCEPTIONS.len()
    );
    assert!(
        problems.is_empty(),
        "WebGPU default limits exceeded:\n{}",
        problems.join("\n")
    );
}

/// A fixture with `buffers` storage buffers, all in one compute stage.
fn storage_fixture(buffers: usize) -> String {
    let mut src = String::new();
    for slot in 0..buffers {
        src.push_str(&format!(
            "@group(0) @binding({slot}) var<storage, read_write> buf{slot}: array<u32>;\n"
        ));
    }
    src.push_str("@compute @workgroup_size(1)\nfn main() {}\n");
    src
}

#[test]
fn nine_storage_buffers_in_one_stage_are_over_the_limit() {
    let module = parse(&storage_fixture(9)).expect("fixture parses");
    let rows = module.rows();
    assert_eq!(rows.len(), 1, "one compute entry point");
    assert_eq!(
        rows[0].counts.storage_buffers, 9,
        "all nine declarations counted"
    );
    assert_eq!(
        over_limits(&rows[0].counts),
        vec!["storage buffers 9 > 8".to_string()]
    );
}

#[test]
fn eight_storage_buffers_in_one_stage_fit() {
    let module = parse(&storage_fixture(8)).expect("fixture parses");
    let rows = module.rows();
    assert_eq!(rows[0].counts.storage_buffers, 8);
    assert!(over_limits(&rows[0].counts).is_empty());
}

#[test]
fn commented_bindings_are_skipped_and_vertex_inputs_are_capped() {
    let vertex_fixture = |inputs: usize| {
        let mut members = String::new();
        for i in 0..inputs {
            members.push_str(&format!("    @location({i}) v{i}: vec4<f32>,\n"));
        }
        format!(
            "// @group(0) @binding(0) var<storage> ghost: array<u32>;\n\
             /* @group(0) @binding(1) var<uniform> ghost2: X; */\n\
             struct VIn {{\n{members}}}\n\
             @vertex fn vs(v: VIn) -> @builtin(position) vec4<f32> {{\n    return vec4<f32>(0.0);\n}}\n"
        )
    };

    let fits = parse(&vertex_fixture(16)).expect("fixture parses");
    let fits_rows = fits.rows();
    assert_eq!(fits_rows[0].counts.vertex_attributes, 16);
    assert_eq!(
        fits_rows[0].counts.storage_buffers, 0,
        "comment is not a binding"
    );
    assert!(over_limits(&fits_rows[0].counts).is_empty());

    let over = parse(&vertex_fixture(17)).expect("fixture parses");
    assert_eq!(
        over_limits(&over.rows()[0].counts),
        vec!["vertex attributes 17 > 16".to_string()]
    );
}
