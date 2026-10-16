//! JSON form of the state machine: strict (unknown fields refused, errors
//! carry the document path), one canonical writer, no float anywhere.

use serde_json::{json, Map, Value};

use super::{
    AdvanceMode, CmpOp, Cond, CondOf, Curve, ParamDef, Source, StateDef, StateMachineDef,
    StateMachineError, SwitchMode, TransitionDef, MAX_COND_DEPTH,
};

/// Largest document read, in bytes.
pub const MAX_JSON_BYTES: usize = 1 << 20;

type Res<T> = Result<T, StateMachineError>;

fn bad(path: &str, reason: &'static str) -> StateMachineError {
    StateMachineError::Json {
        path: path.to_string(),
        reason,
    }
}

fn object<'a>(v: &'a Value, path: &str, allowed: &[&str]) -> Res<&'a Map<String, Value>> {
    let map = v
        .as_object()
        .ok_or_else(|| bad(path, "expected an object"))?;
    for key in map.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(StateMachineError::Json {
                path: format!("{path}.{key}"),
                reason: "unknown field",
            });
        }
    }
    Ok(map)
}

fn field<'a>(map: &'a Map<String, Value>, key: &str, path: &str) -> Res<&'a Value> {
    map.get(key).ok_or_else(|| StateMachineError::Json {
        path: format!("{path}.{key}"),
        reason: "missing field",
    })
}

fn text(map: &Map<String, Value>, key: &str, path: &str) -> Res<String> {
    let at = format!("{path}.{key}");
    field(map, key, path)?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| bad(&at, "expected a string"))
}

fn int(v: &Value, path: &str) -> Res<i64> {
    v.as_i64()
        .ok_or_else(|| bad(path, "expected an integer (no fractions)"))
}

fn uint(v: &Value, path: &str) -> Res<u32> {
    let n = v
        .as_u64()
        .ok_or_else(|| bad(path, "expected a non-negative integer"))?;
    u32::try_from(n).map_err(|_| bad(path, "integer too large"))
}

fn list<'a>(map: &'a Map<String, Value>, key: &str, path: &str) -> Res<&'a Vec<Value>> {
    match map.get(key) {
        None => Ok(empty()),
        Some(v) => v.as_array().ok_or_else(|| StateMachineError::Json {
            path: format!("{path}.{key}"),
            reason: "expected a list",
        }),
    }
}

fn empty() -> &'static Vec<Value> {
    static EMPTY: Vec<Value> = Vec::new();
    &EMPTY
}

fn choice<T: Copy>(
    map: &Map<String, Value>,
    key: &str,
    path: &str,
    table: &[(&str, T)],
    default: T,
) -> Res<T> {
    let Some(v) = map.get(key) else {
        return Ok(default);
    };
    let at = format!("{path}.{key}");
    let s = v.as_str().ok_or_else(|| bad(&at, "expected a string"))?;
    table
        .iter()
        .find(|(name, _)| *name == s)
        .map(|(_, value)| *value)
        .ok_or_else(|| bad(&at, "unknown choice"))
}

const SWITCH: [(&str, SwitchMode); 3] = [
    ("immediate", SwitchMode::Immediate),
    ("sync", SwitchMode::Sync),
    ("at_end", SwitchMode::AtEnd),
];
const ADVANCE: [(&str, AdvanceMode); 3] = [
    ("disabled", AdvanceMode::Disabled),
    ("enabled", AdvanceMode::Enabled),
    ("auto", AdvanceMode::Auto),
];
const CURVE: [(&str, Curve); 4] = [
    ("linear", Curve::Linear),
    ("ease_in", Curve::EaseIn),
    ("ease_out", Curve::EaseOut),
    ("ease_in_out", Curve::EaseInOut),
];
const CMP: [(&str, CmpOp); 6] = [
    ("==", CmpOp::Eq),
    ("!=", CmpOp::Ne),
    ("<", CmpOp::Lt),
    ("<=", CmpOp::Le),
    (">", CmpOp::Gt),
    (">=", CmpOp::Ge),
];

fn name_of<T: Copy + PartialEq>(table: &[(&'static str, T)], value: T) -> &'static str {
    table
        .iter()
        .find(|(_, v)| *v == value)
        .map_or("", |(n, _)| n)
}

/// Parse a machine from JSON text. Structure only; call `validate` after.
pub fn parse(source: &str) -> Res<StateMachineDef> {
    if source.len() > MAX_JSON_BYTES {
        return Err(bad("$", "document too large"));
    }
    let root: Value = serde_json::from_str(source).map_err(|_| bad("$", "not valid JSON"))?;
    from_value(&root)
}

/// Read a machine from a JSON value.
pub fn from_value(root: &Value) -> Res<StateMachineDef> {
    let map = object(
        root,
        "$",
        &[
            "version",
            "name",
            "start",
            "params",
            "states",
            "transitions",
        ],
    )?;
    let version = uint(field(map, "version", "$")?, "$.version")?;
    let mut def = StateMachineDef::new(&text(map, "name", "$")?, &text(map, "start", "$")?);
    def.version = version;
    for (i, p) in list(map, "params", "$")?.iter().enumerate() {
        def.params.push(param(p, &format!("$.params[{i}]"))?);
    }
    for (i, s) in list(map, "states", "$")?.iter().enumerate() {
        def.states.push(state(s, &format!("$.states[{i}]"))?);
    }
    for (i, t) in list(map, "transitions", "$")?.iter().enumerate() {
        def.transitions
            .push(transition(t, &format!("$.transitions[{i}]"))?);
    }
    Ok(def)
}

fn param(v: &Value, path: &str) -> Res<ParamDef> {
    let map = object(v, path, &["name", "default", "min", "max"])?;
    let get = |key: &str| -> Res<i64> { int(field(map, key, path)?, &format!("{path}.{key}")) };
    Ok(ParamDef {
        name: text(map, "name", path)?,
        default: get("default")?,
        min: get("min")?,
        max: get("max")?,
    })
}

fn state(v: &Value, path: &str) -> Res<StateDef> {
    let map = object(
        v,
        path,
        &["name", "clip", "node", "length_ticks", "looping"],
    )?;
    let source = match (map.get("clip"), map.get("node")) {
        (Some(c), None) => Source::Clip(uint(c, &format!("{path}.clip"))?),
        (None, Some(n)) => Source::Node(uint(n, &format!("{path}.node"))?),
        _ => return Err(bad(path, "give exactly one of clip or node")),
    };
    let looping = match map.get("looping") {
        None => true,
        Some(b) => b
            .as_bool()
            .ok_or_else(|| bad(&format!("{path}.looping"), "expected true or false"))?,
    };
    Ok(StateDef {
        name: text(map, "name", path)?,
        source,
        length_ticks: uint(
            field(map, "length_ticks", path)?,
            &format!("{path}.length_ticks"),
        )?,
        looping,
    })
}

fn transition(v: &Value, path: &str) -> Res<TransitionDef> {
    let keys = [
        "from",
        "to",
        "switch",
        "advance",
        "when",
        "fade_ticks",
        "curve",
        "priority",
        "reset",
    ];
    let map = object(v, path, &keys)?;
    let mut t = TransitionDef::new(&text(map, "from", path)?, &text(map, "to", path)?);
    t.switch = choice(map, "switch", path, &SWITCH, t.switch)?;
    t.advance = choice(map, "advance", path, &ADVANCE, t.advance)?;
    t.curve = choice(map, "curve", path, &CURVE, t.curve)?;
    if let Some(c) = map.get("when") {
        t.condition = cond(c, &format!("{path}.when"), 1)?;
    }
    if let Some(n) = map.get("fade_ticks") {
        t.fade_ticks = uint(n, &format!("{path}.fade_ticks"))?;
    }
    if let Some(n) = map.get("priority") {
        t.priority = uint(n, &format!("{path}.priority"))?;
    }
    if let Some(b) = map.get("reset") {
        t.reset = b
            .as_bool()
            .ok_or_else(|| bad(&format!("{path}.reset"), "expected true or false"))?;
    }
    Ok(t)
}

fn cond(v: &Value, path: &str, depth: usize) -> Res<Cond> {
    if depth > MAX_COND_DEPTH {
        return Err(bad(path, "condition too deep"));
    }
    if v.as_str() == Some("always") {
        return Ok(CondOf::Always);
    }
    let map = v
        .as_object()
        .ok_or_else(|| bad(path, "expected \"always\" or an object"))?;
    if map.len() != 1 && !map.contains_key("param") {
        return Err(bad(path, "give exactly one condition key"));
    }
    if map.contains_key("param") {
        let m = object(v, path, &["param", "op", "value"])?;
        let op = choice(m, "op", path, &CMP, CmpOp::Eq)?;
        let value = int(field(m, "value", path)?, &format!("{path}.value"))?;
        return Ok(CondOf::Param {
            param: text(m, "param", path)?,
            op,
            value,
        });
    }
    let (key, inner) = map
        .iter()
        .next()
        .ok_or_else(|| bad(path, "empty condition"))?;
    let at = format!("{path}.{key}");
    match key.as_str() {
        "time_ge" => Ok(CondOf::TimeAtLeast(uint(inner, &at)?)),
        "not" => Ok(CondOf::Not(Box::new(cond(inner, &at, depth + 1)?))),
        "all" | "any" => {
            let items = inner
                .as_array()
                .ok_or_else(|| bad(&at, "expected a list"))?;
            let kids = items
                .iter()
                .enumerate()
                .map(|(i, k)| cond(k, &format!("{at}[{i}]"), depth + 1))
                .collect::<Res<Vec<_>>>()?;
            Ok(if key == "all" {
                CondOf::All(kids)
            } else {
                CondOf::Any(kids)
            })
        }
        _ => Err(StateMachineError::Json {
            path: at,
            reason: "unknown condition",
        }),
    }
}

fn cond_value(c: &Cond) -> Value {
    match c {
        CondOf::Always => json!("always"),
        CondOf::Param { param, op, value } => {
            json!({"param": param, "op": name_of(&CMP, *op), "value": value})
        }
        CondOf::TimeAtLeast(n) => json!({"time_ge": n}),
        CondOf::Not(inner) => json!({"not": cond_value(inner)}),
        CondOf::All(kids) => json!({"all": kids.iter().map(cond_value).collect::<Vec<_>>()}),
        CondOf::Any(kids) => json!({"any": kids.iter().map(cond_value).collect::<Vec<_>>()}),
    }
}

/// Canonical JSON value of a machine.
pub fn to_value(def: &StateMachineDef) -> Value {
    let params: Vec<Value> = def
        .params
        .iter()
        .map(|p| json!({"name": p.name, "default": p.default, "min": p.min, "max": p.max}))
        .collect();
    let states: Vec<Value> = def
        .states
        .iter()
        .map(|s| {
            let mut m = Map::new();
            m.insert("name".into(), json!(s.name));
            match s.source {
                Source::Clip(c) => m.insert("clip".into(), json!(c)),
                Source::Node(n) => m.insert("node".into(), json!(n)),
            };
            m.insert("length_ticks".into(), json!(s.length_ticks));
            m.insert("looping".into(), json!(s.looping));
            Value::Object(m)
        })
        .collect();
    let transitions: Vec<Value> = def
        .transitions
        .iter()
        .map(|t| {
            json!({
                "from": t.from, "to": t.to,
                "switch": name_of(&SWITCH, t.switch), "advance": name_of(&ADVANCE, t.advance),
                "when": cond_value(&t.condition), "fade_ticks": t.fade_ticks,
                "curve": name_of(&CURVE, t.curve), "priority": t.priority, "reset": t.reset,
            })
        })
        .collect();
    json!({
        "version": def.version, "name": def.name, "start": def.start,
        "params": params, "states": states, "transitions": transitions,
    })
}

/// Canonical JSON text of a machine.
pub fn to_json(def: &StateMachineDef) -> String {
    to_value(def).to_string()
}
