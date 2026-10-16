//! extras: Blender node extras as load-time data, no name hacks.
//! Owner: engine-assets, std-only. Units: node indices in, attaches out.
//! Determinism: document order, no clock. Errors: typed, unknown keys refuse.
//! Consumer: weapons row consumes attaches next; this slice is data only.
//! PORT(rust-adventure/skein@e7dd81803dd37241505f60fd108216f7a18988f4:src/lib.rs skein_processing on extras plus extension insert-by-data)
use super::wire::bound_chars;
use std::fmt;
const MAX_BYTES: usize = 1_048_576;
const MAX_DETAIL: usize = 96;
/// Closed attach kind from the extras table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachKind {
    Hitbox,
    Socket,
    Spawn,
}
/// One node attach in document order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtrasAttach {
    pub node: usize,
    pub kind: AttachKind,
    pub payload: String,
}
/// Why extras parsing refused input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtrasError {
    BadJson { detail: String },
    UnknownKey { name: String },
}
impl fmt::Display for ExtrasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadJson { detail } => write!(f, "extras bad json: {detail}"),
            Self::UnknownKey { name } => write!(f, "extras unknown key {name}"),
        }
    }
}
impl std::error::Error for ExtrasError {}
fn bad(w: &str) -> ExtrasError {
    ExtrasError::BadJson {
        detail: bound_chars(w, MAX_DETAIL),
    }
}
fn ws(b: &[u8], p: &mut usize) {
    while matches!(b.get(*p), Some(b' ' | b'\t' | b'\n' | b'\r')) {
        *p += 1;
    }
}
fn end(b: &[u8], s: usize, d: u32) -> Result<usize, ExtrasError> {
    if d > 32 {
        return Err(bad("nesting"));
    }
    let mut p = s;
    ws(b, &mut p);
    match b.get(p) {
        Some(b'{' | b'[') => {
            let o = b[p];
            let c = if o == b'{' { b'}' } else { b']' };
            let mut n = 0usize;
            let mut q = false;
            let mut e = false;
            p += 1;
            // walk to the mate, strings mask brackets
            while let Some(x) = b.get(p) {
                if q {
                    if e {
                        e = false;
                    } else if *x == b'\\' {
                        e = true;
                    } else if *x == b'"' {
                        q = false;
                    }
                } else if *x == b'"' {
                    q = true;
                } else if *x == o {
                    n += 1;
                } else if *x == c {
                    if n == 0 {
                        return Ok(p + 1);
                    }
                    n -= 1;
                }
                p += 1;
            }
            Err(bad("open block"))
        }
        Some(b'"') => {
            p += 1;
            let mut e = false;
            while let Some(x) = b.get(p) {
                if e {
                    e = false;
                } else if *x == b'\\' {
                    e = true;
                } else if *x == b'"' {
                    return Ok(p + 1);
                }
                p += 1;
            }
            Err(bad("open string"))
        }
        Some(_) => {
            while let Some(x) = b.get(p) {
                if matches!(*x, b',' | b'}' | b']') {
                    break;
                }
                p += 1;
            }
            Ok(p)
        }
        None => Err(bad("want value")),
    }
}
fn at(b: &[u8], s: usize, e: usize, want: &str) -> Option<usize> {
    let w = want.as_bytes();
    let mut i = s;
    while i + w.len() + 2 <= e {
        if b.get(i) == Some(&b'"')
            && b.get(i + 1..i + 1 + w.len()) == Some(w)
            && b.get(i + 1 + w.len()) == Some(&b'"')
        {
            let mut p = i + 1 + w.len() + 1;
            while p < e && matches!(b[p], b' ' | b'\t' | b'\n' | b'\r') {
                p += 1;
            }
            if b.get(p) == Some(&b':') {
                return Some(p + 1);
            }
        }
        i += 1;
    }
    None
}
fn keys(
    b: &[u8],
    s: usize,
    e: usize,
    node: usize,
    out: &mut Vec<ExtrasAttach>,
) -> Result<(), ExtrasError> {
    let mut p = s + 1;
    loop {
        ws(b, &mut p);
        if p >= e {
            return Err(bad("skein end"));
        }
        if b.get(p) == Some(&b'}') {
            return Ok(());
        }
        if b.get(p) != Some(&b'"') {
            return Err(bad("want key"));
        }
        let ks = p + 1;
        let ke = end(b, p, 1)? - 1;
        let t = b.get(ks..ke).ok_or_else(|| bad("key range"))?;
        let k = std::str::from_utf8(t).map_err(|_| bad("key utf8"))?;
        p = ke + 1;
        ws(b, &mut p);
        if b.get(p) != Some(&b':') {
            return Err(bad("want colon"));
        }
        p += 1;
        ws(b, &mut p);
        let vs = p;
        let ve = end(b, p, 0)?;
        let kind = match k {
            "Hitbox" => AttachKind::Hitbox,
            "Socket" => AttachKind::Socket,
            "Spawn" => AttachKind::Spawn,
            _ => {
                return Err(ExtrasError::UnknownKey {
                    name: bound_chars(k, MAX_DETAIL),
                });
            }
        };
        let t = b.get(vs..ve).ok_or_else(|| bad("payload range"))?;
        let payload = std::str::from_utf8(t).map_err(|_| bad("payload utf8"))?;
        out.push(ExtrasAttach {
            node,
            kind,
            payload: payload.to_string(),
        });
        p = ve;
        ws(b, &mut p);
        match b.get(p) {
            Some(b',') => {
                p += 1;
            }
            Some(b'}') => {
                return Ok(());
            }
            _ => return Err(bad("want comma")),
        }
    }
}
/// Parse glTF JSON bytes into skein attaches in node document order.
///
/// Both `extras.skein` and `extensions.BEVY_skein` objects are read;
/// keys outside the closed table refuse with [`ExtrasError::UnknownKey`].
pub fn parse(bytes: &[u8]) -> Result<Vec<ExtrasAttach>, ExtrasError> {
    if bytes.len() > MAX_BYTES {
        return Err(bad("input too large"));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| bad("want utf-8"))?;
    let b = text.as_bytes();
    let np = at(b, 0, b.len(), "nodes").ok_or_else(|| bad("no nodes"))?;
    let mut ap = np;
    ws(b, &mut ap);
    if b.get(ap) != Some(&b'[') {
        return Err(bad("nodes array"));
    }
    let ae = end(b, ap, 0)?;
    let mut out = Vec::new();
    let mut p = ap + 1;
    let mut node = 0usize;
    loop {
        ws(b, &mut p);
        if p >= ae {
            return Err(bad("nodes end"));
        }
        if b.get(p) == Some(&b']') {
            return Ok(out);
        }
        if b.get(p) != Some(&b'{') {
            return Err(bad("node object"));
        }
        let ne = end(b, p, 0)?;
        for key in ["skein", "BEVY_skein"] {
            if let Some(vp) = at(b, p, ne, key) {
                let mut q = vp;
                ws(b, &mut q);
                if b.get(q) != Some(&b'{') {
                    return Err(bad("skein object"));
                }
                let se = end(b, q, 0)?;
                keys(b, q, se, node, &mut out)?;
            }
        }
        node += 1;
        p = ne;
        ws(b, &mut p);
        match b.get(p) {
            Some(b',') => {
                p += 1;
            }
            Some(b']') => {
                return Ok(out);
            }
            _ => return Err(bad("nodes comma")),
        }
    }
}
