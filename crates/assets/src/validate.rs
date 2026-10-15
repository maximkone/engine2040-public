//! validate: std-only scene/rules schema gate. Compile once, refuse with key paths, never panic.
//! Owner: engine-assets. Units: JSON docs. Determinism: sorted keys, capped refusals, no IO.
//! Errors: Refusal names path plus reason; parse and compile refuse, never panic. No net, no fs.
//! PORT(Stranger6667/jsonschema@3cd27a35:crates/jsonschema/src/lib.rs): one-shot plus compiled
//! shape with structured refusals; method only, zero bytes vendored, offline core (no resolve).
use std::collections::BTreeMap;

/// Cap on refusals per instance; counts saturate here, order stays deterministic.
pub const MAX_REFUSALS: usize = 16;
/// Cap on JSON nesting; deeper docs refuse instead of recursing.
const MAX_DEPTH: usize = 32;

/// Typed refusal: dotted key path plus one-line reason. Never panics, always names its path.
#[derive(Debug, Clone, PartialEq)]
pub struct Refusal {
    /// Dotted key path (`count`, `transform.x`); `$` for the document root.
    pub path: String,
    /// One-line reason (`missing required key`, `expected integer`, `out of range`...).
    pub reason: String,
}

/// Minimal JSON value; objects sort keys so validation order is deterministic.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// JSON null.
    Null,
    /// JSON true/false.
    Bool(bool),
    /// Finite JSON number.
    Num(f64),
    /// JSON string.
    Str(String),
    /// JSON array.
    Arr(Vec<Value>),
    /// JSON object with sorted keys.
    Obj(BTreeMap<String, Value>),
}

/// Compiled schema: shape-checked once, then validates many instances.
#[derive(Debug, Clone)]
pub struct Schema {
    /// Checked schema document (object with required/properties/additionalProperties).
    v: Value,
}

fn bad(path: &str, reason: &str) -> Refusal {
    Refusal { path: path.into(), reason: reason.into() }
}

fn str_of(v: &Value) -> Option<&str> {
    match v {
        Value::Str(s) => Some(s),
        _ => None,
    }
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() { key.into() } else { format!("{path}.{key}") }
}

/// Parse designer JSON text into a Value; malformed docs refuse, never panic.
pub fn parse(doc: &str) -> Result<Value, Refusal> {
    let mut p = P { b: doc.as_bytes(), i: 0 };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    if p.i != p.b.len() {
        return Err(bad("$", "trailing characters"));
    }
    Ok(v)
}

struct P<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> P<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], 32 | 9 | 10 | 13) {
            self.i += 1;
        }
    }
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }
    fn value(&mut self, d: usize) -> Result<Value, Refusal> {
        if d > MAX_DEPTH {
            return Err(bad("$", "nesting too deep"));
        }
        match self.peek() {
            None => Err(bad("$", "unexpected end")),
            Some(b'n') => self.lit("null", Value::Null),
            Some(b't') => self.lit("true", Value::Bool(true)),
            Some(b'f') => self.lit("false", Value::Bool(false)),
            Some(b'"') => Ok(Value::Str(self.string()?)),
            Some(b'[') => self.array(d),
            Some(b'{') => self.object(d),
            Some(c) if c == b'-' || c.is_ascii_digit() => self.number(),
            _ => Err(bad("$", "unexpected character")),
        }
    }
    fn lit(&mut self, s: &str, v: Value) -> Result<Value, Refusal> {
        let n = s.len();
        if self.b.len().saturating_sub(self.i) >= n && &self.b[self.i..self.i + n] == s.as_bytes() {
            self.i += n;
            Ok(v)
        } else {
            Err(bad("$", "invalid literal"))
        }
    }
    fn number(&mut self) -> Result<Value, Refusal> {
        let s = self.i;
        while self.i < self.b.len() && matches!(self.b[self.i], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E') {
            self.i += 1;
        }
        let t = core::str::from_utf8(&self.b[s..self.i]).map_err(|_| bad("$", "bad number"))?;
        let n: f64 = t.parse().map_err(|_| bad("$", "bad number"))?;
        if !n.is_finite() {
            return Err(bad("$", "number out of range"));
        }
        Ok(Value::Num(n))
    }
    fn hex(&mut self) -> Result<char, Refusal> {
        if self.b.len().saturating_sub(self.i) < 4 {
            return Err(bad("$", "bad unicode"));
        }
        let t = core::str::from_utf8(&self.b[self.i..self.i + 4]).map_err(|_| bad("$", "bad unicode"))?;
        let u = u32::from_str_radix(t, 16).map_err(|_| bad("$", "bad unicode"))?;
        self.i += 4;
        Ok(char::from_u32(u).unwrap_or('\u{FFFD}'))
    }
    fn string(&mut self) -> Result<String, Refusal> {
        self.i += 1;
        let mut s = String::new();
        loop {
            let c = self.b.get(self.i).copied().ok_or_else(|| bad("$", "unterminated string"))?;
            self.i += 1;
            match c {
                b'"' => return Ok(s),
                b'\\' => {
                    let e = self.b.get(self.i).copied().ok_or_else(|| bad("$", "bad escape"))?;
                    self.i += 1;
                    match e {
                        b'"' => s.push('"'),
                        b'\\' => s.push('\\'),
                        b'/' => s.push('/'),
                        b'b' => s.push('\u{8}'),
                        b'f' => s.push('\u{C}'),
                        b'n' => s.push('\n'),
                        b'r' => s.push('\r'),
                        b't' => s.push('\t'),
                        b'u' => s.push(self.hex()?),
                        _ => return Err(bad("$", "bad escape")),
                    }
                }
                0..=31 => return Err(bad("$", "control in string")),
                _ => {
                    let rest = core::str::from_utf8(&self.b[self.i - 1..]).map_err(|_| bad("$", "bad utf8"))?;
                    let ch = rest.chars().next().ok_or_else(|| bad("$", "bad utf8"))?;
                    self.i += ch.len_utf8() - 1;
                    s.push(ch);
                }
            }
        }
    }
    fn array(&mut self, d: usize) -> Result<Value, Refusal> {
        self.i += 1;
        let mut v = Vec::new();
        self.ws();
        if self.peek() == Some(b']') {
            self.i += 1;
            return Ok(Value::Arr(v));
        }
        loop {
            self.ws();
            v.push(self.value(d + 1)?);
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    return Ok(Value::Arr(v));
                }
                _ => return Err(bad("$", "expected , or ]")),
            }
        }
    }
    fn object(&mut self, d: usize) -> Result<Value, Refusal> {
        self.i += 1;
        let mut m = BTreeMap::new();
        self.ws();
        if self.peek() == Some(b'}') {
            self.i += 1;
            return Ok(Value::Obj(m));
        }
        loop {
            self.ws();
            if self.peek() != Some(b'"') {
                return Err(bad("$", "expected string key"));
            }
            let k = self.string()?;
            self.ws();
            if self.peek() != Some(b':') {
                return Err(bad("$", "expected :"));
            }
            self.i += 1;
            self.ws();
            let v = self.value(d + 1)?;
            m.insert(k, v);
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Value::Obj(m));
                }
                _ => return Err(bad("$", "expected , or }")),
            }
        }
    }
}

fn rule_shape(r: &Value, path: String) -> Result<(), Refusal> {
    let Value::Obj(o) = r else { return Err(Refusal { path, reason: "rule must be an object".into() }) };
    if let Some(t) = o.get("type") {
        match str_of(t) {
            Some("string" | "integer" | "number" | "boolean" | "array" | "object" | "null") => {}
            _ => return Err(Refusal { path, reason: "unknown type".into() }),
        }
    }
    for k in ["minimum", "maximum"] {
        if let Some(n) = o.get(k) {
            match n {
                Value::Num(x) if x.is_finite() => {}
                _ => return Err(Refusal { path, reason: "range bound must be a number".into() }),
            }
        }
    }
    if let Some(e) = o.get("enum") {
        if !matches!(e, Value::Arr(_)) {
            return Err(Refusal { path, reason: "enum must be an array".into() });
        }
    }
    if let Some(p) = o.get("properties") {
        match p {
            Value::Obj(m) => {
                for (k, v) in m {
                    rule_shape(v, join(&path, k))?;
                }
            }
            _ => return Err(Refusal { path, reason: "properties must be an object".into() }),
        }
    }
    Ok(())
}

impl Schema {
    /// Compile a schema document once; bad shapes refuse here so validate stays total.
    pub fn compile(schema: &Value) -> Result<Schema, Refusal> {
        let Value::Obj(o) = schema else {
            return Err(bad("$", "schema must be an object"));
        };
        if let Some(t) = o.get("type") {
            if str_of(t) != Some("object") {
                return Err(bad("$", "only object schemas supported"));
            }
        }
        match o.get("required") {
            None => {}
            Some(Value::Arr(req)) if req.iter().all(|x| matches!(x, Value::Str(_))) => {}
            Some(_) => return Err(bad("$.required", "required must list key names")),
        }
        match o.get("properties") {
            None => {}
            Some(Value::Obj(m)) => {
                for (k, v) in m {
                    rule_shape(v, format!("$.properties.{k}"))?;
                }
            }
            Some(_) => return Err(bad("$.properties", "properties must be an object")),
        }
        Ok(Schema { v: schema.clone() })
    }
}

fn push(out: &mut Vec<Refusal>, path: String, reason: String) {
    if out.len() < MAX_REFUSALS {
        out.push(Refusal { path, reason });
    }
}

/// One-shot accept check over a compiled schema.
pub fn is_valid(schema: &Schema, instance: &Value) -> bool {
    validate(schema, instance).is_empty()
}

/// Validate many instances against one compiled schema; refusals sort by key, cap at 16.
pub fn validate(schema: &Schema, instance: &Value) -> Vec<Refusal> {
    let mut out = Vec::new();
    check_obj(&schema.v, instance, String::new(), &mut out);
    out
}

fn check_obj(sch: &Value, inst: &Value, path: String, out: &mut Vec<Refusal>) {
    let Value::Obj(sobj) = sch else { return };
    let Value::Obj(iobj) = inst else {
        let here = if path.is_empty() { "$".into() } else { path };
        push(out, here, "expected object".into());
        return;
    };
    if let Some(Value::Arr(req)) = sobj.get("required") {
        for r in req {
            if let Value::Str(k) = r {
                if !iobj.contains_key(k) {
                    push(out, join(&path, k), "missing required key".into());
                }
            }
        }
    }
    if let Some(Value::Obj(props)) = sobj.get("properties") {
        for (k, rule) in props {
            if let Some(v) = iobj.get(k) {
                check_rule(rule, v, join(&path, k), out);
            }
        }
        if matches!(sobj.get("additionalProperties"), Some(Value::Bool(false))) {
            for k in iobj.keys() {
                if !props.contains_key(k) {
                    push(out, join(&path, k), "unknown key".into());
                }
            }
        }
    }
}

fn check_rule(rule: &Value, v: &Value, path: String, out: &mut Vec<Refusal>) {
    let Value::Obj(ro) = rule else { return };
    if let Some(t) = ro.get("type") {
        let ok = match (str_of(t), v) {
            (Some("string"), Value::Str(_)) => true,
            (Some("integer"), Value::Num(n)) => n.fract() == 0.0,
            (Some("number"), Value::Num(_)) => true,
            (Some("boolean"), Value::Bool(_)) => true,
            (Some("array"), Value::Arr(_)) => true,
            (Some("object"), Value::Obj(_)) => true,
            (Some("null"), Value::Null) => true,
            _ => false,
        };
        if !ok {
            push(out, path, format!("expected {}", str_of(t).unwrap_or("?")));
            return;
        }
    }
    if let Some(Value::Arr(allow)) = ro.get("enum") {
        if !allow.contains(v) {
            push(out, path, "value not in enum".into());
            return;
        }
    }
    if let Value::Num(n) = v {
        let low = ro.get("minimum").and_then(|x| match x {
            Value::Num(l) => Some(*l),
            _ => None,
        });
        let high = ro.get("maximum").and_then(|x| match x {
            Value::Num(h) => Some(*h),
            _ => None,
        });
        if low.is_some_and(|l| n < &l) || high.is_some_and(|h| n > &h) {
            push(out, path, "out of range".into());
            return;
        }
    }
    if matches!(v, Value::Obj(_)) && ro.contains_key("properties") {
        check_obj(rule, v, path, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SCENE: &str = "{\"type\":\"object\",\"required\":[\"name\",\"count\"],\"properties\":{\"name\":{\"type\":\"string\"},\"count\":{\"type\":\"integer\",\"minimum\":0,\"maximum\":8},\"mode\":{\"type\":\"string\",\"enum\":[\"day\",\"night\"]},\"transform\":{\"type\":\"object\",\"required\":[\"x\"],\"properties\":{\"x\":{\"type\":\"number\"},\"y\":{\"type\":\"number\"}}}},\"additionalProperties\":false}";
    const RULES: &str = "{\"type\":\"object\",\"required\":[\"seed\",\"rounds\"],\"properties\":{\"seed\":{\"type\":\"integer\",\"minimum\":1},\"rounds\":{\"type\":\"integer\",\"minimum\":1,\"maximum\":600},\"tier\":{\"type\":\"string\",\"enum\":[\"potato\",\"pc\"]},\"label\":{\"type\":\"string\"}},\"additionalProperties\":false}";
    fn schema(text: &str) -> Schema {
        Schema::compile(&parse(text).expect("schema parses")).expect("schema compiles")
    }
    #[test]
    fn validate_corpus_12_verdicts() {
        let scene = schema(SCENE);
        let rules = schema(RULES);
        let cases: [(&Schema, &str, bool, &str); 12] = [
            (&scene, "{\"name\":\"arena\",\"count\":4,\"mode\":\"day\",\"transform\":{\"x\":1.5}}", true, ""),
            (&scene, "{\"count\":4}", false, "name"),
            (&scene, "{\"name\":\"arena\",\"count\":\"four\"}", false, "count"),
            (&scene, "{\"name\":\"arena\",\"count\":99}", false, "count"),
            (&scene, "{\"name\":\"arena\",\"count\":4,\"mode\":\"dusk\"}", false, "mode"),
            (&scene, "{\"name\":\"arena\",\"count\":4,\"fog\":true}", false, "fog"),
            (&rules, "{\"seed\":2040,\"rounds\":600}", true, ""),
            (&rules, "{\"seed\":2040}", false, "rounds"),
            (&rules, "{\"seed\":\"2040\",\"rounds\":10}", false, "seed"),
            (&rules, "{\"seed\":2040,\"rounds\":601}", false, "rounds"),
            (&rules, "{\"seed\":1,\"rounds\":10,\"tier\":\"ps5\"}", false, "tier"),
            (&rules, "{\"seed\":1,\"rounds\":10,\"cheat\":true}", false, "cheat"),
        ];
        for (s, doc, ok, path) in cases {
            let v = parse(doc).expect("doc parses");
            let r = validate(s, &v);
            assert_eq!(r.is_empty(), ok, "doc {doc}");
            assert!(r.iter().all(|x| !x.path.is_empty()), "doc {doc}");
            if ok {
                assert!(is_valid(s, &v), "doc {doc}");
            } else {
                assert!(r.iter().any(|x| x.path == path), "doc {doc} gives {r:?}");
            }
        }
        let v = parse("{\"name\":\"a\",\"count\":1,\"transform\":{}}").expect("doc parses");
        let r = validate(&scene, &v);
        assert!(r.iter().any(|x| x.path == "transform.x"), "nested path: {r:?}");
        let mut big = String::from("{\"name\":\"a\",\"count\":1");
        for i in 0..24 {
            big.push_str(&format!(",\"k{i}\":1"));
        }
        big.push('}');
        let doc = parse(&big).expect("doc parses");
        let first = validate(&scene, &doc);
        assert_eq!(first.len(), MAX_REFUSALS);
        assert_eq!(first, validate(&scene, &doc));
    }
}
