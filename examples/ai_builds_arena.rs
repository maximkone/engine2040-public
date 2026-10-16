//! Public demo: a scripted external MCP client builds an arena from a
//! brief through the `e2040-mcp` stdio door, plays it, captures it and replays
//! it exactly.
//!
//! One reproducible command (from the repository root; build the door first):
//! `cargo build -p engine-endpoint --features mcp --bin e2040-mcp` then
//! `cargo run --example ai_builds_arena`.
//!
//! The demo spawns the door binary as a child and speaks newline-delimited
//! JSON-RPC on its stdin/stdout, exactly like an outside client (Claude Code,
//! OpenCode, Cursor) would: `server/discover`, `tools/list`, `scene.brief`,
//! `tx.preview` + `tx.apply` (arena shell), `scene.place` (relation solve),
//! `play.run` (headless bout), `play.step` (software-raster PNG capture),
//! then `tx.undo` back to the empty start. It runs the whole loop
//! twice on fresh project folders and refuses to pass unless both passes
//! report identical scene digests, checkpoints and capture bytes.
//!
//! Units: meters in poses, integer centimeters inside wants, ticks for play.
//! Determinism scope: same binary plus same fixed ids/seed gives the same
//! report; wall clock never enters it. Errors are plain strings: any refused
//! tool, mismatch or missing binary fails loudly with the cause.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use engine_core::hash_bytes;

/// Bounded wait per door reply: a silent server fails the demo, never hangs it.
const REPLY_TIMEOUT: Duration = Duration::from_secs(60);
/// Door protocol the demo speaks (2026-07-28 inline lifecycle: every request
/// carries the client `_meta`, no separate `initialize` handshake).
const PROTOCOL_VERSION: &str = "2026-07-28";
/// Match seed: every bout in both passes plays this seed.
const MATCH_SEED: u64 = 2040;
/// Match length in ticks for `play.run` and the `play.step` capture.
const MATCH_TICKS: u32 = 8;
/// Fighter count for the demo bout.
const FIGHTERS: u8 = 2;
/// Arena half-extent in meters for the demo bout.
const HALF_EXTENT_M: &str = "20.0";
/// Capture receipt size in pixels (software rasterizer bound).
const CAPTURE_WIDTH: u32 = 96;
const CAPTURE_HEIGHT: u32 = 64;
/// Idempotency keys: the shell insert batch and the placement batch.
const KEY_SHELL: u64 = 7_001;
const KEY_PLACE: u64 = 7_002;
/// Nil id: "no parent" for root inserts.
const NIL_ID: &str = "00000000-0000-0000-0000-000000000000";
/// Fixed arena ids: cross-pass digests match only because the ids are fixed.
const WALL_NORTH: &str = "10000000-0000-4000-8000-000000000001";
const WALL_SOUTH: &str = "10000000-0000-4000-8000-000000000002";
const WALL_WEST: &str = "10000000-0000-4000-8000-000000000003";
const WALL_EAST: &str = "10000000-0000-4000-8000-000000000004";
const GATE: &str = "10000000-0000-4000-8000-000000000005";
const BRAZIER: &str = "10000000-0000-4000-8000-000000000006";
/// PNG file magic: every capture receipt starts with these bytes.
const PNG_MAGIC: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

fn main() {
    match run() {
        Ok(()) => {}
        Err(error) => {
            eprintln!("ai_builds_arena FAILED: {error}");
            std::process::exit(1);
        }
    }
}

/// Two full passes through fresh doors, then the equality verdict.
fn run() -> Result<(), String> {
    let bin = door_binary()?;
    let root = std::env::temp_dir().join(format!("ai-builds-arena-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("pass-a"))
        .map_err(|error| format!("scratch pass-a creates: {error}"))?;
    std::fs::create_dir_all(root.join("pass-b"))
        .map_err(|error| format!("scratch pass-b creates: {error}"))?;
    let capture_path = root.join("arena-capture.png");

    let first = run_pass(&bin, &root.join("pass-a"), Some(capture_path.as_path()))?;
    let second = run_pass(&bin, &root.join("pass-b"), None)?;
    if first != second {
        return Err(format!(
            "replay mismatch:\n  pass-a {first}\n  pass-b {second}"
        ));
    }
    println!("ai_builds_arena PASS");
    println!("  pass-a {first}");
    println!("  pass-b {second}");
    println!("  capture {}", capture_path.display());
    println!("  rerun: cargo run --example ai_builds_arena");
    Ok(())
}

/// One full loop: brief, build, place, play, capture, undo back to empty.
/// Returns the canonical report line; any refusal or surprise fails loudly.
fn run_pass(bin: &Path, project_dir: &Path, capture_path: Option<&Path>) -> Result<String, String> {
    let mut client = DoorClient::spawn(bin, project_dir)?;

    client.discover()?;
    client.require_tools(&[
        "scene.brief",
        "scene.read",
        "scene.place",
        "tx.preview",
        "tx.apply",
        "tx.undo",
        "play.run",
        "play.step",
    ])?;

    let empty = client.brief()?;
    if empty.revision != 0 {
        return Err(format!(
            "fresh project opens at revision {}, not 0",
            empty.revision
        ));
    }

    let approval = client.preview_shell()?;
    let applied_revision = client.apply_shell(&approval)?;
    if applied_revision != 1 {
        return Err(format!(
            "shell apply lands at revision {applied_revision}, not 1"
        ));
    }
    let built = client.brief()?;
    if built.revision != 1 || built.digest == empty.digest {
        return Err(format!(
            "shell build must move the digest at revision 1: {built:?}"
        ));
    }

    let placed = client.place_brazier(built.revision)?;
    if !placed.solved {
        return Err(format!("arena intent must solve: {}", placed.reason));
    }

    let checkpoints = client.play_bout()?;
    let want_points =
        usize::try_from(MATCH_TICKS).map_err(|_| "tick count fits usize".to_owned())?;
    if checkpoints.len() != want_points {
        return Err(format!(
            "bout keeps {} checkpoints, not {MATCH_TICKS}",
            checkpoints.len()
        ));
    }
    let (step_digest, png) = client.capture_step()?;
    if png.len() < PNG_MAGIC.len() || png[0..PNG_MAGIC.len()] != PNG_MAGIC {
        return Err(format!(
            "capture is not a PNG ({} bytes, bad magic)",
            png.len()
        ));
    }
    let png_hash = hash_bytes(&png);
    if let Some(path) = capture_path {
        std::fs::write(path, &png).map_err(|error| format!("capture writes: {error}"))?;
    }

    client.undo(KEY_PLACE)?;
    client.undo(KEY_SHELL)?;
    let restored = client.brief()?;
    let read_back = client.scene_total()?;
    if read_back != 0 || restored.digest != empty.digest {
        return Err(format!(
            "undo must restore the empty start, got total {read_back} digest {:016x} (empty {:016x})",
            restored.digest, empty.digest
        ));
    }

    if !client.close_and_wait() {
        return Err("door exits nonzero on stdin EOF".to_owned());
    }
    let checkpoint_text = checkpoints
        .iter()
        .map(|point| format!("{}:{:016x}", point.0, point.1))
        .collect::<Vec<_>>()
        .join(",");
    Ok(format!(
        "empty={:016x} built={:016x} placed=({:.3},{:.3},{:.3}) checkpoints=[{checkpoint_text}] step={step_digest:016x} png={png_hash:016x}/{}B",
        empty.digest, built.digest, placed.x, placed.y, placed.z, png.len()
    ))
}

/// Minimal JSON value: enough to read door replies without new dependencies.
#[derive(Debug, Clone, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    fn get(&self, key: &str) -> Result<&Json, String> {
        match self {
            Json::Obj(entries) => entries
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value)
                .ok_or_else(|| format!("reply lacks `{key}`")),
            other => Err(format!("reply is not an object: {other:?}")),
        }
    }

    fn as_bool(&self) -> Result<bool, String> {
        match self {
            Json::Bool(flag) => Ok(*flag),
            other => Err(format!("value is not a bool: {other:?}")),
        }
    }

    fn as_u64(&self) -> Result<u64, String> {
        match self {
            Json::Num(raw) => raw
                .parse::<u64>()
                .map_err(|_| format!("`{raw}` is not a u64")),
            other => Err(format!("value is not a number: {other:?}")),
        }
    }

    fn as_f32(&self) -> Result<f32, String> {
        match self {
            Json::Num(raw) => raw
                .parse::<f32>()
                .map_err(|_| format!("`{raw}` is not an f32")),
            other => Err(format!("value is not a number: {other:?}")),
        }
    }

    fn as_str(&self) -> Result<&str, String> {
        match self {
            Json::Str(text) => Ok(text),
            other => Err(format!("value is not a string: {other:?}")),
        }
    }

    fn as_array(&self) -> Result<&[Json], String> {
        match self {
            Json::Arr(items) => Ok(items),
            other => Err(format!("value is not an array: {other:?}")),
        }
    }
}

/// Recursive-descent JSON reader over the reply bytes. Objects keep
/// first-seen order; duplicate keys keep the last value.
struct JsonReader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

fn parse_json(text: &str) -> Result<Json, String> {
    let mut reader = JsonReader {
        bytes: text.as_bytes(),
        cursor: 0,
    };
    let value = reader.value()?;
    reader.spaces();
    if reader.cursor != reader.bytes.len() {
        return Err("trailing bytes after the JSON reply".to_owned());
    }
    Ok(value)
}

impl<'a> JsonReader<'a> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.cursor).copied()
    }

    fn spaces(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.cursor += 1;
        }
    }

    fn value(&mut self) -> Result<Json, String> {
        self.spaces();
        match self.peek() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => self.word("true", Json::Bool(true)),
            Some(b'f') => self.word("false", Json::Bool(false)),
            Some(b'n') => self.word("null", Json::Null),
            Some(ch) if ch == b'-' || ch.is_ascii_digit() => self.number(),
            other => Err(format!("unexpected JSON byte: {other:?}")),
        }
    }

    fn word(&mut self, text: &str, value: Json) -> Result<Json, String> {
        if self.bytes[self.cursor..].starts_with(text.as_bytes()) {
            self.cursor += text.len();
            Ok(value)
        } else {
            Err(format!("bad JSON literal near `{text}`"))
        }
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.cursor;
        if self.peek() == Some(b'-') {
            self.cursor += 1;
        }
        match self.peek() {
            Some(b'0') => {
                self.cursor += 1;
                if self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
                    return Err("leading zero in JSON number".to_owned());
                }
            }
            Some(ch) if ch.is_ascii_digit() => {
                while self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
                    self.cursor += 1;
                }
            }
            _ => return Err("empty JSON number".to_owned()),
        }
        if self.peek() == Some(b'.') {
            self.cursor += 1;
            while self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
                self.cursor += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.cursor += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.cursor += 1;
            }
            while self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
                self.cursor += 1;
            }
        }
        let raw = std::str::from_utf8(&self.bytes[start..self.cursor])
            .map_err(|_| "reply number is not UTF-8".to_owned())?;
        if raw.is_empty() || raw == "-" {
            return Err("empty JSON number".to_owned());
        }
        Ok(Json::Num(raw.to_owned()))
    }

    fn string(&mut self) -> Result<String, String> {
        // Caller peeked the opening quote.
        self.cursor += 1;
        let mut out = String::new();
        loop {
            let ch = self
                .peek()
                .ok_or_else(|| "unterminated JSON string".to_owned())?;
            self.cursor += 1;
            match ch {
                b'"' => return Ok(out),
                b'\\' => {
                    let esc = self
                        .peek()
                        .ok_or_else(|| "unterminated JSON escape".to_owned())?;
                    self.cursor += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000C}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(self.unicode()?),
                        other => return Err(format!("bad JSON escape `\\{other}`")),
                    }
                }
                0x00..=0x1F => return Err("raw control byte in JSON string".to_owned()),
                _ => {
                    // Continuation bytes ride with their leading byte below.
                    let start = self.cursor - 1;
                    let width = utf8_width(ch);
                    while self.cursor < start + width {
                        if self.peek().is_none() {
                            return Err("cut UTF-8 in JSON string".to_owned());
                        }
                        self.cursor += 1;
                    }
                    let slice = std::str::from_utf8(&self.bytes[start..self.cursor])
                        .map_err(|_| "bad UTF-8 in JSON string".to_owned())?;
                    out.push_str(slice);
                }
            }
        }
    }

    fn unicode(&mut self) -> Result<char, String> {
        let high = self.hex4()?;
        if (0xD800..0xDC00).contains(&high) {
            if self.bytes.get(self.cursor..self.cursor + 2) == Some(b"\\u".as_slice()) {
                self.cursor += 2;
                let low = self.hex4()?;
                if (0xDC00..0xE000).contains(&low) {
                    let point = 0x1_0000 + ((high - 0xD800) << 10) + (low - 0xDC00);
                    return char::from_u32(point).ok_or_else(|| "bad surrogate pair".to_owned());
                }
            }
            return Err("lone high surrogate".to_owned());
        }
        if (0xDC00..0xE000).contains(&high) {
            return Err("lone low surrogate".to_owned());
        }
        char::from_u32(high).ok_or_else(|| "bad code point".to_owned())
    }

    fn hex4(&mut self) -> Result<u32, String> {
        if self.cursor + 4 > self.bytes.len() {
            return Err("cut \\u escape".to_owned());
        }
        let mut value = 0u32;
        for byte in &self.bytes[self.cursor..self.cursor + 4] {
            value = value * 16
                + u32::from(match byte {
                    b'0'..=b'9' => byte - b'0',
                    b'a'..=b'f' => byte - b'a' + 10,
                    b'A'..=b'F' => byte - b'A' + 10,
                    other => return Err(format!("bad hex digit `{other}`")),
                });
        }
        self.cursor += 4;
        Ok(value)
    }

    fn object(&mut self) -> Result<Json, String> {
        // Caller peeked `{`.
        self.cursor += 1;
        let mut entries: Vec<(String, Json)> = Vec::new();
        self.spaces();
        if self.peek() == Some(b'}') {
            self.cursor += 1;
            return Ok(Json::Obj(entries));
        }
        loop {
            self.spaces();
            if self.peek() != Some(b'"') {
                return Err("object key must be a string".to_owned());
            }
            let key = self.string()?;
            self.spaces();
            if self.peek() != Some(b':') {
                return Err("object key misses `:`".to_owned());
            }
            self.cursor += 1;
            let value = self.value()?;
            if let Some(slot) = entries.iter_mut().find(|(name, _)| *name == key) {
                slot.1 = value;
            } else {
                entries.push((key, value));
            }
            self.spaces();
            match self.peek() {
                Some(b',') => self.cursor += 1,
                Some(b'}') => {
                    self.cursor += 1;
                    return Ok(Json::Obj(entries));
                }
                other => return Err(format!("bad object separator: {other:?}")),
            }
        }
    }

    fn array(&mut self) -> Result<Json, String> {
        // Caller peeked `[`.
        self.cursor += 1;
        let mut items = Vec::new();
        self.spaces();
        if self.peek() == Some(b']') {
            self.cursor += 1;
            return Ok(Json::Arr(items));
        }
        loop {
            items.push(self.value()?);
            self.spaces();
            match self.peek() {
                Some(b',') => self.cursor += 1,
                Some(b']') => {
                    self.cursor += 1;
                    return Ok(Json::Arr(items));
                }
                other => return Err(format!("bad array separator: {other:?}")),
            }
        }
    }
}

/// UTF-8 width from the leading byte (caller matched ASCII and quotes away).
fn utf8_width(lead: u8) -> usize {
    if lead < 0x80 {
        1
    } else if lead < 0xE0 {
        2
    } else if lead < 0xF0 {
        3
    } else {
        4
    }
}

/// Locate the door binary: explicit env first, then beside this example's own
/// executable (`target/<profile>/examples/` -> `target/<profile>/`).
fn door_binary() -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var("E2040_MCP_BIN") {
        let bin = PathBuf::from(&path);
        if bin.is_file() {
            return Ok(bin);
        }
        return Err(format!("E2040_MCP_BIN points nowhere: {path}"));
    }
    let exe = std::env::current_exe().map_err(|error| format!("own path reads: {error}"))?;
    let exe_name = format!("e2040-mcp{}", std::env::consts::EXE_SUFFIX);
    let mut dir = exe
        .parent()
        .ok_or_else(|| "example executable has no parent".to_owned())?
        .to_path_buf();
    if dir
        .file_name()
        .is_some_and(|name| name == "examples" || name == "debug" || name == "release")
        && dir.file_name().is_some_and(|name| name == "examples")
    {
        dir.pop();
    }
    let bin = dir.join(&exe_name);
    if bin.is_file() {
        return Ok(bin);
    }
    Err(format!(
        "door binary missing at {}: build it with \
         `cargo build -p engine-endpoint --features mcp --bin e2040-mcp` \
         (or set E2040_MCP_BIN)",
        bin.display()
    ))
}

/// Arena shell ops: six unit-cube roots (four walls, gate, brazier).
/// Shared by the preview and the apply so the two batches never drift.
fn shell_ops() -> String {
    fn insert(id: &str, x: &str, z: &str) -> String {
        format!(
            "{{\"op\":\"insert\",\"id\":\"{id}\",\"parent\":\"{NIL_ID}\",\
             \"local\":{{\"pos\":[{x},0.0,{z}],\"rot\":[0.0,0.0,0.0,1.0],\
             \"scale\":[1.0,1.0,1.0]}}}}"
        )
    }
    [
        insert(WALL_NORTH, "0.0", "-5.0"),
        insert(WALL_SOUTH, "0.0", "5.0"),
        insert(WALL_WEST, "-5.0", "0.0"),
        insert(WALL_EAST, "5.0", "0.0"),
        insert(GATE, "4.0", "-4.0"),
        insert(BRAZIER, "0.0", "2.0"),
    ]
    .join(",")
}

/// One outside client: the child door plus its piped stdio.
struct DoorClient {
    child: Child,
    stdin: std::process::ChildStdin,
    stdout: Arc<Mutex<BufReader<ChildStdout>>>,
    next_id: u64,
}

/// Brief read: revision plus digest.
#[derive(Debug)]
struct Brief {
    revision: u64,
    digest: u64,
}

/// Solved placement pose in meters.
#[derive(Debug)]
struct Placement {
    solved: bool,
    reason: String,
    x: f32,
    y: f32,
    z: f32,
}

impl DoorClient {
    fn spawn(bin: &Path, project_dir: &Path) -> Result<Self, String> {
        let mut child = Command::new(bin)
            .arg("--project-dir")
            .arg(project_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("door spawns: {error}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "child stdin pipes".to_owned())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "child stdout pipes".to_owned())?;
        Ok(Self {
            child,
            stdin,
            stdout: Arc::new(Mutex::new(BufReader::new(stdout))),
            next_id: 1,
        })
    }

    /// One JSON-RPC request: write a line, read the reply line, return `result`.
    fn send(&mut self, method: &str, params: &str) -> Result<Json, String> {
        let id = self.next_id;
        self.next_id += 1;
        writeln!(
            self.stdin,
            "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"{method}\",\"params\":{params}}}"
        )
        .map_err(|error| format!("request writes: {error}"))?;
        self.stdin
            .flush()
            .map_err(|error| format!("request flushes: {error}"))?;
        let reply = read_line(&self.stdout)?;
        if reply.get("jsonrpc").ok().and_then(|v| v.as_str().ok()) != Some("2.0") {
            return Err(format!("door stops speaking JSON-RPC 2.0: {reply:?}"));
        }
        if reply.get("id").ok().and_then(|v| v.as_u64().ok()) != Some(id) {
            return Err(format!("door echoes a foreign id: {reply:?}"));
        }
        if let Ok(error) = reply.get("error") {
            return Err(format!("door answers error: {error:?}"));
        }
        reply.get("result").cloned()
    }

    /// One tool call: `tools/call` with a name plus raw arguments JSON.
    fn call(&mut self, tool: &str, arguments: &str) -> Result<Json, String> {
        let meta = format!(
            "\"_meta\":{{\"io.modelcontextprotocol/protocolVersion\":\"{PROTOCOL_VERSION}\",\
             \"io.modelcontextprotocol/clientCapabilities\":{{}},\
             \"io.modelcontextprotocol/clientInfo\":{{\"name\":\"ai-builds-arena\",\"version\":\"1.0.0\"}}}}"
        );
        let params = format!("{{{meta},\"name\":\"{tool}\",\"arguments\":{arguments}}}");
        let result = self.send("tools/call", &params)?;
        if result.get("isError").ok().and_then(|v| v.as_bool().ok()) == Some(true) {
            return Err(format!("tool `{tool}` reports error: {result:?}"));
        }
        payload(&result)
    }

    /// Discovery first: the inline lifecycle opens with `server/discover`.
    fn discover(&mut self) -> Result<(), String> {
        let meta = format!(
            "{{\"_meta\":{{\"io.modelcontextprotocol/protocolVersion\":\"{PROTOCOL_VERSION}\",\
             \"io.modelcontextprotocol/clientCapabilities\":{{}},\
             \"io.modelcontextprotocol/clientInfo\":{{\"name\":\"ai-builds-arena\",\"version\":\"1.0.0\"}}}}}}"
        );
        let discovered = self.send("server/discover", &meta)?;
        let versions = discovered.get("supportedVersions")?.as_array()?;
        if !versions
            .iter()
            .any(|v| v.as_str().is_ok_and(|name| name == PROTOCOL_VERSION))
        {
            return Err(format!("door serves no {PROTOCOL_VERSION}: {discovered:?}"));
        }
        Ok(())
    }

    /// The door must advertise every tool this demo drives.
    fn require_tools(&mut self, tools: &[&str]) -> Result<(), String> {
        let meta = format!(
            "{{\"_meta\":{{\"io.modelcontextprotocol/protocolVersion\":\"{PROTOCOL_VERSION}\",\
             \"io.modelcontextprotocol/clientCapabilities\":{{}},\
             \"io.modelcontextprotocol/clientInfo\":{{\"name\":\"ai-builds-arena\",\"version\":\"1.0.0\"}}}}}}"
        );
        let list = self.send("tools/list", &meta)?;
        let advertised = list.get("tools")?.as_array()?;
        let mut names = Vec::with_capacity(advertised.len());
        for tool in advertised {
            names.push(tool.get("name")?.as_str()?.to_owned());
        }
        for tool in tools {
            if !names.iter().any(|name| name == tool) {
                return Err(format!("door hides `{tool}`: {names:?}"));
            }
        }
        Ok(())
    }

    fn brief(&mut self) -> Result<Brief, String> {
        let brief = self.call("scene.brief", "{}")?;
        let revision = brief.get("revision")?.as_u64()?;
        let text = brief.get("text")?.as_str()?;
        let digest = if let Ok(digest) = brief.get("digest") {
            digest.as_u64()?
        } else {
            text.strip_prefix("digest ")
                .and_then(|rest| rest.split_whitespace().next())
                .ok_or_else(|| "brief text carries no digest".to_owned())?
                .parse::<u64>()
                .map_err(|_| "brief digest parses")?
        };
        Ok(Brief { revision, digest })
    }

    fn scene_total(&mut self) -> Result<u64, String> {
        self.call("scene.read", "{}")?.get("total")?.as_u64()
    }

    /// Arena shell: six unit-cube roots (four walls, gate, brazier) in one
    /// previewed batch, returning the minted approval JSON.
    fn preview_shell(&mut self) -> Result<String, String> {
        let ops = shell_ops();
        let args =
            format!("{{\"expected_revision\":0,\"idempotency_key\":{KEY_SHELL},\"ops\":[{ops}]}}");
        let preview = self.call("tx.preview", &args)?;
        if preview.get("resulting_revision")?.as_u64()? != 1 {
            return Err(format!("shell preview must reach revision 1: {preview:?}"));
        }
        if preview.get("effects")?.as_array()?.len() != 6 {
            return Err(format!("shell preview carries six effects: {preview:?}"));
        }
        let approval = preview.get("approval")?.clone();
        if approval.get("action_hash").is_err() {
            return Err(format!("shell preview mints an approval: {preview:?}"));
        }
        Ok(render(&approval))
    }

    fn apply_shell(&mut self, approval: &str) -> Result<u64, String> {
        let ops = shell_ops();
        let args = format!(
            "{{\"expected_revision\":0,\"idempotency_key\":{KEY_SHELL},\"ops\":[{ops}],\"approval\":{approval}}}"
        );
        let applied = self.call("tx.apply", &args)?;
        if applied.get("key")?.as_u64()? != KEY_SHELL {
            return Err(format!("shell apply keeps its key: {applied:?}"));
        }
        applied.get("revision")?.as_u64()
    }

    /// Placement: the brazier settles against the north wall, a fixed
    /// edge gap from the gate, through the gated apply.
    fn place_brazier(&mut self, revision: u64) -> Result<Placement, String> {
        let args = format!(
            "{{\"expected_revision\":{revision},\"idempotency_key\":{KEY_PLACE},\
             \"object\":\"{BRAZIER}\",\
             \"wants\":[\"next_to object:{WALL_NORTH}\",\"next_to object:{GATE} [150,250]\"]}}"
        );
        let out = self.call("scene.place", &args)?;
        let solved = out
            .get("solved")?
            .as_bool()
            .map_err(|_| "scene.place answers solved".to_owned())?;
        let reason = out
            .get("reason")
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        if !solved {
            return Ok(Placement {
                solved,
                reason,
                x: 0.0,
                y: 0.0,
                z: 0.0,
            });
        }
        let pose = out.get("placement")?;
        Ok(Placement {
            solved,
            reason,
            x: pose.get("x")?.as_f32()?,
            y: pose.get("y")?.as_f32()?,
            z: pose.get("z")?.as_f32()?,
        })
    }

    /// Headless bout: start, then poll to `completed`, returning checkpoints.
    fn play_bout(&mut self) -> Result<Vec<(u32, u64)>, String> {
        let args = format!(
            "{{\"action\":\"start\",\"seed\":{MATCH_SEED},\"ticks\":{MATCH_TICKS},\
             \"fighters\":{FIGHTERS},\"half_extent_m\":{HALF_EXTENT_M}}}"
        );
        let started = self.call("play.run", &args)?;
        if started.get("status")?.as_str()? != "running" {
            return Err(format!("bout starts running: {started:?}"));
        }
        let job = started.get("job_id")?.as_u64()?;
        for _ in 0..6 {
            let polled = self.call(
                "play.run",
                &format!("{{\"action\":\"poll\",\"job_id\":{job}}}"),
            )?;
            match polled.get("status")?.as_str()? {
                "completed" => {
                    let checkpoints = polled.get("checkpoints")?.as_array()?;
                    let mut points = Vec::with_capacity(checkpoints.len());
                    for point in checkpoints {
                        points.push((
                            point.get("tick")?.as_u64().and_then(uint_to_u32)?,
                            point.get("hash")?.as_u64()?,
                        ));
                    }
                    return Ok(points);
                }
                "running" => {}
                status => return Err(format!("bout polls `{status}`: {polled:?}")),
            }
        }
        Err("bout never completes in six polls".to_owned())
    }

    /// Frame capture: one `play.step` duel receipt, PNG bytes out.
    fn capture_step(&mut self) -> Result<(u64, Vec<u8>), String> {
        let args = format!(
            "{{\"seed\":{MATCH_SEED},\"participants\":{FIGHTERS},\
             \"half_extent_m\":{HALF_EXTENT_M},\"upto_tick\":{MATCH_TICKS},\
             \"inputs\":[],\"width\":{CAPTURE_WIDTH},\"height\":{CAPTURE_HEIGHT}}}"
        );
        let step = self.call("play.step", &args)?;
        let digest = step.get("digest")?.as_u64()?;
        let bytes = step.get("png_bytes")?.as_array()?;
        let mut png = Vec::with_capacity(bytes.len());
        for byte in bytes {
            png.push(byte.as_u64().and_then(uint_to_u8)?);
        }
        Ok((digest, png))
    }

    fn undo(&mut self, token: u64) -> Result<(), String> {
        let undone = self.call("tx.undo", &format!("{{\"undo_token\":{token}}}"))?;
        if undone.get("undone_key")?.as_u64()? != token {
            return Err(format!("undo reverts key {token}: {undone:?}"));
        }
        Ok(())
    }

    /// Close stdin and wait for the door to exit on EOF.
    fn close_and_wait(mut self) -> bool {
        drop(self.stdin);
        for _ in 0..200 {
            match self.child.try_wait() {
                Ok(Some(status)) => return status.success(),
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(_) => return false,
            }
        }
        let _ = self.child.kill();
        false
    }
}

fn uint_to_u32(value: u64) -> Result<u32, String> {
    u32::try_from(value).map_err(|_| format!("{value} fits u32"))
}

fn uint_to_u8(value: u64) -> Result<u8, String> {
    u8::try_from(value).map_err(|_| format!("{value} fits u8"))
}

/// Tool result payload: `structuredContent` when the door sends it, else the
/// JSON text of the first content block.
fn payload(result: &Json) -> Result<Json, String> {
    if let Ok(structured) = result.get("structuredContent") {
        return Ok(structured.clone());
    }
    let blocks = result.get("content")?.as_array()?;
    let text = blocks
        .first()
        .ok_or_else(|| "tool answers no content block".to_owned())?
        .get("text")?
        .as_str()?;
    parse_json(text)
}

/// Render a [`Json`] value back to canonical JSON text (for approvals).
fn render(value: &Json) -> String {
    match value {
        Json::Null => "null".to_owned(),
        Json::Bool(true) => "true".to_owned(),
        Json::Bool(false) => "false".to_owned(),
        Json::Num(raw) => raw.clone(),
        Json::Str(text) => {
            let mut out = String::from("\"");
            for ch in text.chars() {
                match ch {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    _ if ch.is_control() => {
                        out.push_str(&format!("\\u{:04x}", u32::from(ch)));
                    }
                    _ => out.push(ch),
                }
            }
            out.push('"');
            out
        }
        Json::Arr(items) => {
            let inner = items.iter().map(render).collect::<Vec<_>>().join(",");
            format!("[{inner}]")
        }
        Json::Obj(entries) => {
            let inner = entries
                .iter()
                .map(|(name, item)| {
                    format!("{}:{}", render(&Json::Str(name.clone())), render(item))
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{inner}}}")
        }
    }
}

/// Read one stdout line on a helper thread: bounded, never hangs the demo.
fn read_line(stdout: &Arc<Mutex<BufReader<ChildStdout>>>) -> Result<Json, String> {
    let (tx, rx) = mpsc::channel();
    let stdout = Arc::clone(stdout);
    std::thread::spawn(move || {
        let mut line = String::new();
        let read = stdout
            .lock()
            .map_err(|_| "reader lock holds".to_owned())
            .and_then(|mut guard| {
                guard
                    .read_line(&mut line)
                    .map_err(|error| format!("door stdout reads: {error}"))
            });
        let _ = tx.send(read.map(|_| line));
    });
    match rx.recv_timeout(REPLY_TIMEOUT) {
        Ok(Ok(line)) => parse_json(line.trim()),
        Ok(Err(error)) => Err(error),
        Err(_) => Err("timed out waiting for the door reply".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_json, render, Json};

    fn roundtrip(text: &str) {
        let value = parse_json(text).expect("fixture parses");
        let back = render(&value);
        assert_eq!(
            parse_json(&back).expect("render parses"),
            value,
            "render keeps the value"
        );
    }

    #[test]
    fn json_numbers_stay_exact() {
        let digest = parse_json(r#"{"digest":18446744073709551615}"#).expect("u64 max parses");
        assert_eq!(
            digest.get("digest").expect("digest reads").as_u64(),
            Ok(u64::MAX)
        );
        roundtrip(r#"{"a":0,"b":-12,"c":20.0,"d":1e3,"e":2.5E-2}"#);
    }

    #[test]
    fn json_strings_unescape() {
        let value = parse_json(r#"{"t":"a\"b\\c\nA"}"#).expect("escapes parse");
        assert_eq!(
            value.get("t").expect("text reads").as_str(),
            Ok("a\"b\\c\nA")
        );
        roundtrip(r#"{"t":"digest 1234 revision 1"}"#);
    }

    #[test]
    fn json_nesting_and_voids() {
        let value = parse_json(r#"{"o":{"n":null,"b":true,"f":false},"a":[1,[2,{}]]}"#)
            .expect("nesting parses");
        assert_eq!(
            value
                .get("o")
                .expect("object reads")
                .get("b")
                .expect("bool reads"),
            &Json::Bool(true)
        );
        assert_eq!(
            value
                .get("a")
                .expect("array reads")
                .as_array()
                .map(<[Json]>::len),
            Ok(2)
        );
    }

    #[test]
    fn json_garbage_fails() {
        for bad in [
            "",
            "{",
            "{\"a\":}",
            "[1,]",
            "\"cut",
            "{\"a\":01}",
            "tru",
            "[1 2]",
            "{\"a\":\"bad \\x escape\"}",
        ] {
            assert!(parse_json(bad).is_err(), "`{bad}` must fail");
        }
    }

    #[test]
    fn json_duplicate_keys_keep_last() {
        let value = parse_json(r#"{"k":1,"k":2}"#).expect("dupes parse");
        assert_eq!(value.get("k").expect("key reads").as_u64(), Ok(2));
    }
}
