//! netquery: where the page's `?server=` address sends the net duel (WEB9, WEB12).
//!
//! The page joins a native dedicated server when its address carries
//! `?server=ws://host:port`. Two optional keys add the WebTransport leg of the
//! connect race: `&wt=https://host:port/e2040` and `&cert=<64 hex digits>`, the
//! SHA-256 of the server's pinned certificate (`net-serve --wt` prints both).
//! Without `wt` only the WebSocket leg runs, and the race says so in its
//! failure text if that leg fails.
//!
//! Pure text in, text out, so it builds and tests on every target; the browser
//! glue that reads `location.search` lives in `netplay` (wasm only).
//!
//! Errors: a `String` that says which key is wrong and what was expected.

/// The path the dedicated server's WebSocket and WebTransport listeners serve
/// (`engine_net::websocket::SESSION_PATH`, which sits behind a server feature
/// this crate does not enable).
pub const SESSION_PATH: &str = "/e2040";

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
/// A message naming the key when `server` is not a `ws://` or `wss://` address,
/// `wt` is not `https://`, or `cert` is not 64 hex digits.
pub fn parse_target(
    server: &str,
    wt: Option<&str>,
    cert: Option<&str>,
) -> Result<NetTarget, String> {
    let rest = server
        .strip_prefix("ws://")
        .or_else(|| server.strip_prefix("wss://"))
        .ok_or_else(|| format!("server={server}: expected ws://host:port or wss://host"))?;
    if rest.is_empty() || rest.starts_with('/') {
        return Err(format!("server={server}: the host is missing"));
    }
    let ws = if rest.contains('/') {
        server.to_owned()
    } else {
        format!("{server}{SESSION_PATH}")
    };
    let wt = match wt.filter(|text| !text.is_empty()) {
        Some(url) if url.starts_with("https://") => Some(url.to_owned()),
        Some(url) => return Err(format!("wt={url}: expected https://host:port/e2040")),
        None => None,
    };
    let cert = match cert.filter(|text| !text.is_empty()) {
        Some(hex) => Some(parse_hash(hex)?),
        None => None,
    };
    Ok(NetTarget { ws, wt, cert })
}

fn parse_hash(hex: &str) -> Result<[u8; 32], String> {
    let bad = || format!("cert={hex}: expected 64 hex digits (the SHA-256 of the certificate)");
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
        let target = parse_target(
            "ws://h:1",
            Some("https://h:2/e2040"),
            Some(hash.as_str()),
        )
        .expect("parses");
        assert_eq!(target.wt.as_deref(), Some("https://h:2/e2040"));
        let cert = target.cert.expect("cert");
        assert_eq!((cert[0], cert[31]), (0, 255));
        // Empty values are the same as missing keys.
        let empty = parse_target("ws://h:1", Some(""), Some("")).expect("parses");
        assert_eq!((empty.wt, empty.cert), (None, None));
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
            let error = parse_target(server, wt, cert).expect_err(server);
            assert!(error.contains(expected), "{error} should contain {expected}");
        }
    }
}
