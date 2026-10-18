//! netquery: where the page's address sends the net duel (WEB9, WEB12).
//!
//! The page joins a native dedicated server when its address carries
//! `?join=host:port&cert=<64 hex digits>`: one port for both legs (TCP for the WebSocket,
//! UDP for WebTransport), `cert` the SHA-256 of the server's pinned certificate. `net-serve
//! --wt` prints the whole link. The long form `?server=ws://host:port` names the WebSocket
//! leg alone; the optional `&wt=https://host:port/e2040` and `&cert=` add the WebTransport
//! leg of the connect race. Without `wt` or `cert` only the WebSocket leg runs, and the
//! race says so in its failure text if that leg fails.
//!
//! Three more keys shape the session: `ticks=N` (the match length, which the server must
//! share), `play` (the keyboard drives the seat; without it the seat plays the script the
//! proofs use) and `token=N` (a fixed seat token, so a second page can reclaim the seat).
//!
//! An https page may not open `ws://` to another machine (mixed content): [`ws_blocked`]
//! says so before the browser throws a bare security error.
//!
//! Pure text in, text out, so it builds and tests on every target; the browser
//! glue that reads `location.search` lives in `netplay` (wasm only).
//!
//! Errors: [`QueryError`], which names the key that is wrong and what it takes.

/// The path the dedicated server's WebSocket and WebTransport listeners serve
/// (`engine_net::websocket::SESSION_PATH`, which sits behind a server feature
/// this crate does not enable).
pub const SESSION_PATH: &str = "/e2040";

/// A query value the page cannot use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryError {
    /// The query key (`server`, `wt` or `cert`).
    pub key: &'static str,
    /// The value that was given.
    pub value: String,
    /// What the key takes.
    pub expected: &'static str,
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}={}: {}", self.key, self.value, self.expected)
    }
}

impl std::error::Error for QueryError {}

use engine_net::webduel::DUEL_TICKS;

use crate::arena_view::has_flag;

/// Longest match the page accepts (the arena's limit).
pub const MAX_TICKS: u32 = 100_000;

/// What the page was asked to join.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetTarget {
    /// The WebSocket URL, path included.
    pub ws: String,
    /// The WebTransport URL, when the page address offered one.
    pub wt: Option<String>,
    /// The SHA-256 the WebTransport certificate must have.
    pub cert: Option<[u8; 32]>,
}

/// Read the three query values into a target.
///
/// # Errors
///
/// A [`QueryError`] naming the key when `server` is not a `ws://` or `wss://` address,
/// `wt` is not `https://`, or `cert` is not 64 hex digits.
pub fn parse_target(
    server: &str,
    wt: Option<&str>,
    cert: Option<&str>,
) -> Result<NetTarget, QueryError> {
    let bad_server = |expected| QueryError {
        key: "server",
        value: server.to_owned(),
        expected,
    };
    let rest = server
        .strip_prefix("ws://")
        .or_else(|| server.strip_prefix("wss://"))
        .ok_or_else(|| bad_server("expected ws://host:port or wss://host"))?;
    if rest.is_empty() || rest.starts_with('/') {
        return Err(bad_server("the host is missing"));
    }
    let ws = if rest.contains('/') {
        server.to_owned()
    } else {
        format!("{server}{SESSION_PATH}")
    };
    let wt = match wt.filter(|text| !text.is_empty()) {
        Some(url) if url.starts_with("https://") => Some(url.to_owned()),
        Some(url) => {
            return Err(QueryError {
                key: "wt",
                value: url.to_owned(),
                expected: "expected https://host:port/e2040",
            })
        }
        None => None,
    };
    let cert = match cert.filter(|text| !text.is_empty()) {
        Some(hex) => Some(parse_hash(hex)?),
        None => None,
    };
    Ok(NetTarget { ws, wt, cert })
}

/// Read the shorthand `join=host:port` into a target: the WebSocket leg at `ws://host:port`
/// and, when `cert` is given, the WebTransport leg at `https://host:port` on the same port
/// number (TCP and UDP are different ports to the network).
///
/// # Errors
///
/// A [`QueryError`] naming `join` when it is not `host:port`, or `cert` when it is not 64
/// hex digits.
pub fn parse_join(join: &str, cert: Option<&str>) -> Result<NetTarget, QueryError> {
    let bad = |expected| QueryError {
        key: "join",
        value: join.to_owned(),
        expected,
    };
    let (host, port) = join
        .rsplit_once(':')
        .ok_or_else(|| bad("expected host:port"))?;
    let plain = |c: char| c.is_ascii_alphanumeric() || c == '.' || c == '-';
    let host_ok = match host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        Some(v6) => {
            !v6.is_empty()
                && v6
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.')
        }
        None => !host.is_empty() && host.chars().all(plain),
    };
    if !host_ok {
        return Err(bad("the host is a name, an IPv4 address or [IPv6]"));
    }
    if !port.parse::<u16>().is_ok_and(|port| port > 0) {
        return Err(bad("the port is 1 to 65535"));
    }
    let cert = cert.filter(|text| !text.is_empty());
    let wt = cert.map(|_| format!("https://{join}{SESSION_PATH}"));
    parse_target(&format!("ws://{join}"), wt.as_deref(), cert)
}

/// Everything the page address asks of the net duel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetQuery {
    /// Where to join.
    pub target: NetTarget,
    /// Match length in ticks; the server must serve the same.
    pub ticks: u32,
    /// True when the keyboard drives the seat, false for the proofs' script.
    pub play: bool,
    /// A fixed seat token, so a second page can take the seat back.
    pub token: Option<u64>,
}

/// The percent-decoded value of `key` in a query string (`?a=b&c`), if the key is there.
fn query_value(search: &str, key: &str) -> Option<String> {
    search.trim_start_matches('?').split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        (percent_decode(name) == key).then(|| percent_decode(value))
    })
}

/// `%XX` escapes and `+` decoded; a malformed escape stays as written.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let escaped = match (bytes[at], bytes.get(at + 1..at + 3)) {
            (b'%', Some(pair)) => std::str::from_utf8(pair)
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok()),
            _ => None,
        };
        match (escaped, bytes[at]) {
            (Some(byte), _) => {
                out.push(byte);
                at += 3;
            }
            (None, b'+') => {
                out.push(b' ');
                at += 1;
            }
            (None, byte) => {
                out.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Read the page address (`location.search`). `None` when it asks for no net duel at all.
///
/// # Errors
///
/// A [`QueryError`] naming the key that is wrong: `join` or `server` (not both), `wt`,
/// `cert`, `ticks` (1 to [`MAX_TICKS`]) or `token` (a nonzero number).
pub fn parse_search(search: &str) -> Result<Option<NetQuery>, QueryError> {
    let get = |key| query_value(search, key);
    let target = match (get("join"), get("server")) {
        (None, None) => return Ok(None),
        (Some(join), None) => parse_join(&join, get("cert").as_deref())?,
        (None, Some(server)) => {
            parse_target(&server, get("wt").as_deref(), get("cert").as_deref())?
        }
        (Some(join), Some(_)) => {
            return Err(QueryError {
                key: "join",
                value: join,
                expected: "give join=host:port or server=ws://host:port, not both",
            })
        }
    };
    let number = |key: &'static str, expected: &'static str| match get(key) {
        None => Ok(None),
        Some(text) => text
            .parse::<u64>()
            .ok()
            .filter(|&n| n > 0)
            .map(Some)
            .ok_or(QueryError {
                key,
                value: text,
                expected,
            }),
    };
    let long = "expected a match length of 1 to 100000 ticks";
    let ticks = match number("ticks", long)? {
        None => DUEL_TICKS,
        Some(n) => u32::try_from(n)
            .ok()
            .filter(|&n| n <= MAX_TICKS)
            .ok_or(QueryError {
                key: "ticks",
                value: n.to_string(),
                expected: long,
            })?,
    };
    let token = number("token", "expected a nonzero number")?;
    Ok(Some(NetQuery {
        target,
        ticks,
        play: has_flag(search, "play"),
        token,
    }))
}

/// Why an https page cannot open `ws_url`, when it cannot: a browser blocks `ws://` to any
/// host but this machine from a secure page (mixed content), and the bare error it throws
/// does not say so.
#[must_use]
pub fn ws_blocked(page_is_https: bool, ws_url: &str) -> Option<String> {
    let rest = ws_url.strip_prefix("ws://").filter(|_| page_is_https)?;
    let authority = rest.split('/').next().unwrap_or("");
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().map(|inner| format!("[{inner}]")),
        None => authority.rsplit_once(':').map(|(host, _)| host.to_owned()),
    }
    .unwrap_or_else(|| authority.to_owned());
    let this_machine = host == "localhost"
        || host.ends_with(".localhost")
        || host == "[::1]"
        || host.starts_with("127.");
    (!this_machine).then(|| {
        format!(
            "the page is https, so the browser blocks {ws_url} as mixed content: \
             use a wss:// address, or the WebTransport leg (join=host:port with cert=)"
        )
    })
}

fn parse_hash(hex: &str) -> Result<[u8; 32], QueryError> {
    let bad = || QueryError {
        key: "cert",
        value: hex.to_owned(),
        expected: "expected 64 hex digits (the SHA-256 of the certificate)",
    };
    if hex.len() != 64 || !hex.is_ascii() {
        return Err(bad());
    }
    let mut out = [0u8; 32];
    for (byte, pair) in out.iter_mut().zip(hex.as_bytes().chunks_exact(2)) {
        let text = std::str::from_utf8(pair).map_err(|_| bad())?;
        *byte = u8::from_str_radix(text, 16).map_err(|_| bad())?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_ws_address_gets_the_session_path() {
        let target = parse_target("ws://127.0.0.1:7050", None, None).expect("parses");
        assert_eq!(target.ws, "ws://127.0.0.1:7050/e2040");
        assert_eq!((target.wt, target.cert), (None, None));
        let own = parse_target("wss://play.example/duel", None, None).expect("parses");
        assert_eq!(own.ws, "wss://play.example/duel", "a given path is kept");
    }

    #[test]
    fn the_webtransport_leg_needs_both_its_keys_to_parse() {
        let hash = "00".repeat(31) + "ff";
        let target = parse_target("ws://h:1", Some("https://h:2/e2040"), Some(hash.as_str()))
            .expect("parses");
        assert_eq!(target.wt.as_deref(), Some("https://h:2/e2040"));
        let cert = target.cert.expect("cert");
        assert_eq!((cert[0], cert[31]), (0, 255));
        // Empty values are the same as missing keys.
        let empty = parse_target("ws://h:1", Some(""), Some("")).expect("parses");
        assert_eq!((empty.wt, empty.cert), (None, None));
    }

    #[test]
    fn join_is_one_port_for_both_legs_and_needs_cert_for_webtransport() {
        let hash = "ab".repeat(32);
        let both = parse_join("192.168.1.20:7050", Some(hash.as_str())).expect("parses");
        assert_eq!(both.ws, "ws://192.168.1.20:7050/e2040");
        assert_eq!(both.wt.as_deref(), Some("https://192.168.1.20:7050/e2040"));
        assert_eq!(both.cert.map(|c| c[0]), Some(0xab));
        let ws_only = parse_join("duel.example:443", None).expect("parses");
        assert_eq!((ws_only.wt, ws_only.cert), (None, None));
        assert_eq!(
            parse_join("[::1]:7050", None).expect("v6").ws,
            "ws://[::1]:7050/e2040"
        );
        for (join, expected) in [
            ("nohost", "host:port"),
            ("host:0", "1 to 65535"),
            ("host:99999", "1 to 65535"),
            (":7050", "the host is"),
            ("bad host:7050", "the host is"),
        ] {
            let error = parse_join(join, None).expect_err(join).to_string();
            assert!(
                error.contains(expected),
                "{error} should contain {expected}"
            );
        }
        assert!(parse_join("h:1", Some("abcd"))
            .expect_err("short cert")
            .to_string()
            .contains("cert="));
    }

    #[test]
    fn the_address_gives_target_length_play_and_token() {
        assert_eq!(parse_search("").expect("no key"), None);
        assert_eq!(parse_search("?square&tier=web").expect("other keys"), None);
        let hash = "00".repeat(32);
        let search = format!("?join=h%3A7050&cert={hash}&ticks=3600&play&token=424242");
        let query = parse_search(&search).expect("parses").expect("a net query");
        assert_eq!(query.target.ws, "ws://h:7050/e2040");
        assert_eq!(
            (query.ticks, query.play, query.token),
            (3600, true, Some(424_242))
        );
        let plain = parse_search("?server=ws://127.0.0.1:7050")
            .expect("parses")
            .expect("a net query");
        assert_eq!((plain.ticks, plain.play, plain.token), (600, false, None));
        for (search, expected) in [
            ("?join=h:1&server=ws://h:1", "not both"),
            ("?join=h:1&ticks=0", "ticks=0"),
            ("?join=h:1&ticks=100001", "ticks=100001"),
            ("?join=h:1&ticks=abc", "ticks=abc"),
            ("?join=h:1&token=0", "token=0"),
        ] {
            let error = parse_search(search).expect_err(search).to_string();
            assert!(
                error.contains(expected),
                "{error} should contain {expected}"
            );
        }
        assert_eq!(percent_decode("a%2Fb+c%zz%4"), "a/b c%zz%4");
    }

    #[test]
    fn an_https_page_names_mixed_content_for_ws_to_another_machine() {
        let blocked = ws_blocked(true, "ws://192.168.1.20:7050/e2040").expect("blocked");
        assert!(
            blocked.contains("mixed content") && blocked.contains("wss://"),
            "{blocked}"
        );
        for fine in [
            "ws://127.0.0.1:7050/e2040",
            "ws://localhost:7050/e2040",
            "ws://[::1]:7050/e2040",
            "ws://duel.localhost:7050/e2040",
            "wss://duel.example/e2040",
        ] {
            assert_eq!(ws_blocked(true, fine), None, "{fine}");
        }
        assert_eq!(
            ws_blocked(false, "ws://192.168.1.20:7050/e2040"),
            None,
            "a plain page may"
        );
        assert!(ws_blocked(true, "ws://[fe80::1]:7050/e2040").is_some());
    }

    #[test]
    fn wrong_values_name_their_key() {
        let cases = [
            (("http://h:1", None, None), "server=http://h:1"),
            (("ws://", None, None), "host is missing"),
            (("ws:///e2040", None, None), "host is missing"),
            (("ws://h:1", Some("ws://h:2"), None), "wt=ws://h:2"),
            (("ws://h:1", None, Some("abcd")), "cert=abcd"),
            (("ws://h:1", None, Some(&"zz".repeat(32))), "cert=zz"),
        ];
        for ((server, wt, cert), expected) in cases {
            let error = parse_target(server, wt, cert)
                .expect_err(server)
                .to_string();
            assert!(
                error.contains(expected),
                "{error} should contain {expected}"
            );
        }
    }
}
