//! The one JSON line the page prints to itself and to the console.
//!
//! Written by hand (no JSON crate on the web path). A line has no raw newline, so a log
//! reader takes it whole; every string is escaped; numbers are plain integers or floats.

use std::fmt::Write as _;

/// A JSON string literal for `s`: quotes, backslashes and control characters escaped.
#[must_use]
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A flat JSON object under construction: `{"k":v,...}`.
#[derive(Debug, Default)]
pub struct Object {
    body: String,
}

impl Object {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn key(&mut self, k: &str) {
        if !self.body.is_empty() {
            self.body.push(',');
        }
        self.body.push_str(&quote(k));
        self.body.push(':');
    }

    pub fn str(&mut self, k: &str, v: &str) -> &mut Self {
        self.key(k);
        self.body.push_str(&quote(v));
        self
    }

    pub fn uint(&mut self, k: &str, v: u64) -> &mut Self {
        self.key(k);
        let _ = write!(self.body, "{v}");
        self
    }

    pub fn bool(&mut self, k: &str, v: bool) -> &mut Self {
        self.key(k);
        self.body.push_str(if v { "true" } else { "false" });
        self
    }

    /// `v` is already JSON (an object or an array built here).
    pub fn raw(&mut self, k: &str, v: &str) -> &mut Self {
        self.key(k);
        self.body.push_str(v);
        self
    }

    #[must_use]
    pub fn finish(&self) -> String {
        format!("{{{}}}", self.body)
    }
}

/// A JSON array of strings.
#[must_use]
pub fn str_array(items: &[String]) -> String {
    let parts: Vec<String> = items.iter().map(|s| quote(s)).collect();
    format!("[{}]", parts.join(","))
}

/// What the browser said about the adapter. The `web_*` fields are the raw WebGPU
/// `GPUAdapterInfo` strings (vendor `nvidia`, `google` for the software adapter, ...); the
/// wgpu side only carries the description and cannot see the rest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AdapterFacts {
    pub name: String,
    pub backend: String,
    pub device_type: String,
    pub web_vendor: String,
    pub web_architecture: String,
    pub web_device: String,
    pub web_description: String,
}

/// Everything the ready line says. `commit` is the build's commit; `wasm_pages` counts 64 KiB
/// pages of linear memory (the browser's growth cap is a separate limit, not shown).
#[derive(Clone, Debug, Default)]
pub struct Ready {
    pub commit: String,
    pub adapter: AdapterFacts,
    pub features: Vec<String>,
    pub limits: Vec<(&'static str, u64)>,
    pub wasm_pages: u64,
    /// 1 for the first device; one more after each device loss the shell recovered from.
    pub generation: u32,
    pub canvas: (u32, u32),
    pub surface_format: String,
    pub user_agent: String,
    /// `window.crossOriginIsolated` when the line was printed (WEB14): true means the page got the
    /// COOP/COEP headers (from the service worker on a host that cannot send them) and may use
    /// `SharedArrayBuffer`. The page's own `#coi` line adds how many reloads it took.
    pub cross_origin_isolated: bool,
}

/// What the arena page adds to its ready line: the quality tier it chose (WEB8), the pipeline
/// warm-up (WEB13) and the validation errors counted up to the line. `tools/web-browse.mjs`
/// and the door's receipt line read `tier`, `validation` and `pipelines` from it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArenaFacts {
    pub tier: String,
    /// `query`, `limits` or `default`.
    pub tier_why: String,
    /// Pipelines built before the first frame.
    pub pipelines: u64,
    /// Milliseconds the warm-up took.
    pub pipelines_ms: u64,
    /// Pipelines built after the warm-up (0 on a healthy page).
    pub late_pipelines: u64,
    /// Validation errors of every kind counted when the line is printed.
    pub validation: u64,
    /// PW37: the scene the first frames drew (the cooked kit's name, or the stand-in ring's).
    pub scene: String,
}

impl Ready {
    /// The ready line: `"ok":true` and every fact above.
    #[must_use]
    pub fn line(&self) -> String {
        self.object().finish()
    }

    /// The ready line with the arena facts (tier, pipelines, validation, scene) appended.
    #[must_use]
    pub fn line_with(&self, facts: &ArenaFacts) -> String {
        let mut line = self.object();
        line.str("tier", &facts.tier)
            .str("tierWhy", &facts.tier_why)
            .uint("pipelines", facts.pipelines)
            .uint("pipelinesMs", facts.pipelines_ms)
            .uint("latePipelines", facts.late_pipelines)
            .uint("validation", facts.validation)
            .str("scene", &facts.scene);
        line.finish()
    }

    fn object(&self) -> Object {
        let a = &self.adapter;
        let mut adapter = Object::new();
        adapter
            .str("name", &a.name)
            .str("deviceType", &a.device_type)
            .str("vendor", &a.web_vendor)
            .str("architecture", &a.web_architecture)
            .str("device", &a.web_device)
            .str("description", &a.web_description);
        let mut limits = Object::new();
        for (name, value) in &self.limits {
            limits.uint(name, *value);
        }
        let mut canvas = Object::new();
        canvas
            .uint("width", u64::from(self.canvas.0))
            .uint("height", u64::from(self.canvas.1));
        let mut line = Object::new();
        line.bool("ok", true)
            .str("commit", &self.commit)
            .str("backend", &a.backend)
            .raw("adapter", &adapter.finish())
            .raw("features", &str_array(&self.features))
            .raw("limits", &limits.finish())
            .uint("wasmMemoryPages", self.wasm_pages)
            .uint("wasmMemoryBytes", self.wasm_pages * 65_536)
            .uint("deviceGeneration", u64::from(self.generation))
            .raw("canvas", &canvas.finish())
            .str("surfaceFormat", &self.surface_format)
            .str("userAgent", &self.user_agent)
            .bool("crossOriginIsolated", self.cross_origin_isolated);
        line
    }
}

/// Why the shell stopped before the first frame. `code` is short and stable (a test and the
/// headless check key on it); `detail` is the browser's own words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// `navigator.gpu` is missing: an old browser, or a page that is not https or localhost.
    NoWebGpu,
    /// `navigator.gpu` is there and gave no adapter (GPU blocklisted, disabled, or none).
    NoAdapter,
    /// The adapter refused the device the renderer needs.
    NoDevice,
    /// The page has no canvas with the id the shell was given.
    NoCanvas,
    /// The canvas could not become a surface the adapter can present to.
    NoSurface,
}

impl Failure {
    /// The stable code and the plain sentence, in one place.
    fn text(self) -> (&'static str, &'static str) {
        match self {
            Failure::NoWebGpu => (
                "no-webgpu",
                "This browser has no WebGPU (navigator.gpu is missing).",
            ),
            Failure::NoAdapter => (
                "no-adapter",
                "WebGPU is here but the browser gave no graphics adapter.",
            ),
            Failure::NoDevice => (
                "no-device",
                "The graphics adapter refused the device this game needs.",
            ),
            Failure::NoCanvas => ("no-canvas", "The page has no canvas for the game."),
            Failure::NoSurface => (
                "no-surface",
                "The canvas could not be used as a drawing surface.",
            ),
        }
    }

    #[must_use]
    pub fn code(self) -> &'static str {
        self.text().0
    }

    /// One plain sentence for the person looking at the page.
    #[must_use]
    pub fn headline(self) -> &'static str {
        self.text().1
    }
}

/// What the fatal page says in its reason line: the plain sentence, the browser's own words and
/// `fallback=<code>` (QT11), the same code the receipt carries, so the page names why it is not
/// running the game and never shows a blank canvas.
#[must_use]
pub fn fatal_text(why: Failure, detail: &str) -> String {
    format!("{} {detail} (fallback={})", why.headline(), why.code())
}

/// The line for a shell that did not start: `"ok":false` with the reason, and `fallback`, the
/// same code as a word of its own (QT11).
#[must_use]
pub fn failed_line(
    why: Failure,
    detail: &str,
    commit: &str,
    wasm_pages: u64,
    user_agent: &str,
) -> String {
    let mut line = Object::new();
    line.bool("ok", false)
        .str("reason", why.code())
        .str("fallback", why.code())
        .str("message", why.headline())
        .str("detail", detail)
        .str("commit", commit)
        .uint("wasmMemoryPages", wasm_pages)
        .str("userAgent", user_agent);
    line.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Ready {
        Ready {
            commit: "abc1234".into(),
            adapter: AdapterFacts {
                name: "NVIDIA \"RTX\" 3050\\x".into(),
                backend: "BrowserWebGpu".into(),
                device_type: "Other".into(),
                web_vendor: "nvidia".into(),
                web_architecture: "ampere".into(),
                web_device: String::new(),
                web_description: "line one\nline two".into(),
            },
            features: vec!["timestamp-query".into(), "shader-f16".into()],
            limits: vec![("maxBindGroups", 4), ("maxBufferSize", 4_294_967_296)],
            wasm_pages: 258,
            generation: 2,
            canvas: (1280, 720),
            surface_format: "bgra8unorm".into(),
            user_agent: "Mozilla/5.0 HeadlessChrome".into(),
            cross_origin_isolated: false,
        }
    }

    #[test]
    fn quote_escapes_what_breaks_a_json_line() {
        assert_eq!(quote("a\"b"), r#""a\"b""#);
        assert_eq!(quote("a\\b"), r#""a\\b""#);
        assert_eq!(quote("a\nb\tc\r"), r#""a\nb\tc\r""#);
        assert_eq!(quote("\u{1}"), r#""\u0001""#);
        assert_eq!(quote("plain \u{e9}"), "\"plain \u{e9}\"");
    }

    #[test]
    fn ready_line_is_one_line_and_carries_every_fact() {
        let line = sample().line();
        assert!(!line.contains('\n') && !line.contains('\r'), "{line}");
        assert!(line.starts_with('{') && line.ends_with('}'));
        for needle in [
            r#""ok":true"#,
            r#""commit":"abc1234""#,
            r#""backend":"BrowserWebGpu""#,
            r#""vendor":"nvidia""#,
            r#""architecture":"ampere""#,
            r#""features":["timestamp-query","shader-f16"]"#,
            r#""limits":{"maxBindGroups":4,"maxBufferSize":4294967296}"#,
            r#""wasmMemoryPages":258"#,
            r#""wasmMemoryBytes":16908288"#,
            r#""deviceGeneration":2"#,
            r#""canvas":{"width":1280,"height":720}"#,
            r#""surfaceFormat":"bgra8unorm""#,
            // WEB14: the ready line carries the page's cross-origin isolation beside the other facts.
            r#""crossOriginIsolated":false"#,
        ] {
            assert!(line.contains(needle), "{needle} missing from {line}");
        }
        // A quote or a newline inside a value must not end the line early.
        assert!(line.contains(r#"NVIDIA \"RTX\" 3050\\x"#), "{line}");
        assert!(line.contains(r#"line one\nline two"#), "{line}");
    }

    #[test]
    fn the_ready_line_reports_cross_origin_isolation_when_it_is_true() {
        let mut ready = sample();
        ready.cross_origin_isolated = true;
        let line = ready.line();
        assert!(line.contains(r#""crossOriginIsolated":true"#), "{line}");
        // The arena line (tier, pipelines, validation) keeps it too.
        let facts = ArenaFacts {
            tier: "web".into(),
            tier_why: "default".into(),
            pipelines: 1,
            pipelines_ms: 2,
            late_pipelines: 0,
            validation: 0,
            scene: "arena-kit".into(),
        };
        let arena = ready.line_with(&facts);
        assert!(arena.contains(r#""crossOriginIsolated":true"#), "{arena}");
        assert!(sample().line().contains(r#""crossOriginIsolated":false"#));
    }

    #[test]
    fn the_arena_ready_line_adds_tier_pipelines_and_validation() {
        let facts = ArenaFacts {
            tier: "web".into(),
            tier_why: "default".into(),
            pipelines: 24,
            pipelines_ms: 412,
            late_pipelines: 0,
            validation: 0,
            scene: "arena-kit".into(),
        };
        let line = sample().line_with(&facts);
        assert!(!line.contains('\n'), "{line}");
        assert!(line.starts_with("{\"ok\":true"), "{line}");
        for needle in [
            r#""tier":"web""#,
            r#""tierWhy":"default""#,
            r#""pipelines":24"#,
            r#""pipelinesMs":412"#,
            r#""latePipelines":0"#,
            r#""validation":0"#,
            // PW37: the line names the scene the first frames drew.
            r#""scene":"arena-kit""#,
            // The plain facts are still there, in front.
            r#""commit":"abc1234""#,
            r#""userAgent":"Mozilla/5.0 HeadlessChrome""#,
        ] {
            assert!(line.contains(needle), "{needle} missing from {line}");
        }
        // The plain line (the square shell) does not carry them.
        assert!(!sample().line().contains("\"tier\""));
    }

    #[test]
    fn braces_balance_outside_strings() {
        let line = sample().line();
        let mut depth = 0i32;
        let mut in_str = false;
        let mut escaped = false;
        for c in line.chars() {
            if in_str {
                match (escaped, c) {
                    (true, _) => escaped = false,
                    (false, '\\') => escaped = true,
                    (false, '"') => in_str = false,
                    _ => {}
                }
            } else {
                match c {
                    '"' => in_str = true,
                    '{' | '[' => depth += 1,
                    '}' | ']' => depth -= 1,
                    _ => {}
                }
            }
            assert!(depth >= 0, "closed too early in {line}");
        }
        assert_eq!(depth, 0, "unbalanced {line}");
        assert!(!in_str, "unterminated string in {line}");
    }

    #[test]
    fn failure_line_names_the_reason_and_is_not_ok() {
        let line = failed_line(
            Failure::NoWebGpu,
            "navigator.gpu is undefined",
            "abc1234",
            17,
            "UA",
        );
        assert!(line.contains(r#""ok":false"#), "{line}");
        assert!(line.contains(r#""reason":"no-webgpu""#), "{line}");
        assert!(line.contains(r#""fallback":"no-webgpu""#), "{line}");
        assert!(line.contains("navigator.gpu is missing"), "{line}");
        assert!(line.contains(r#""wasmMemoryPages":17"#), "{line}");
        assert!(!line.contains('\n'));
    }

    #[test]
    fn the_fatal_page_names_the_fallback_beside_the_browsers_words() {
        let text = fatal_text(Failure::NoWebGpu, "navigator.gpu is undefined.");
        assert!(
            text.starts_with("This browser has no WebGPU")
                && text.contains("navigator.gpu is undefined.")
                && text.ends_with("(fallback=no-webgpu)"),
            "{text}"
        );
        for why in [
            Failure::NoWebGpu,
            Failure::NoAdapter,
            Failure::NoDevice,
            Failure::NoCanvas,
            Failure::NoSurface,
        ] {
            assert!(fatal_text(why, "x").contains(&format!("fallback={}", why.code())));
        }
    }

    #[test]
    fn failure_codes_are_distinct_and_have_a_sentence() {
        let all = [
            Failure::NoWebGpu,
            Failure::NoAdapter,
            Failure::NoDevice,
            Failure::NoCanvas,
            Failure::NoSurface,
        ];
        let mut codes: Vec<_> = all.iter().map(|f| f.code()).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), all.len());
        assert!(all.iter().all(|f| f.headline().ends_with('.')));
    }

    #[test]
    fn empty_array_and_object_are_valid() {
        assert_eq!(str_array(&[]), "[]");
        assert_eq!(Object::new().finish(), "{}");
    }
}
