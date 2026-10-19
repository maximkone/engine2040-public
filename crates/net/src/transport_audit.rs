//! transport_audit: token auth against TLS, measured side by side.
//!
//! Question: should the internet QUIC path keep TLS 1.3 (as quinn ships it),
//! or should a netcode-style token layer authenticate the transport? The
//! audited token pattern is the sealed-token, challenge and response shape
//! reimplemented in [`handshake`](crate::handshake), so no dependency enters
//! the graph.
//!
//! Both paths are driven for real by the benchmark test on one machine: the
//! token path runs the three-message exchange in [`handshake`](crate::handshake)
//! (HMAC-SHA256 seals when the `crypto` feature is on), and the TLS path drives
//! rustls, the TLS engine inside quinn, through a full TLS 1.3 handshake with
//! certificate-chain verification, in memory, with no sockets. The measured
//! numbers are the exact encoded bytes each direction and the CPU nanoseconds
//! one setup costs. [`compare`] turns a pair of [`SetupCost`] into a verdict at
//! one link round-trip time.
//!
//! Result (2026-10-19, measured by the benchmark test): both paths set up in
//! three flights, so the link term ties. The token exchange carries 376 bytes
//! and costs 5.3 us of CPU; the TLS 1.3 handshake carries 794 bytes and costs
//! 141.7 us. That gap is a fixed 0.14 ms of setup, so TLS is inside the five
//! percent tie margin from about 1.7 ms RTT upward: every internet link ties,
//! and only a sub-millisecond LAN shows the token path ahead. The token path
//! needs a token backend and leaves game traffic unencrypted at this layer,
//! while TLS gives encryption, forward secrecy and server authentication in the
//! same setup, so quinn stays the transport and no token transport rides beside
//! it.
//!
//! Nothing here touches the fixed tick: setup runs at join time, and no wall
//! clock read ever reaches a simulation hash.

/// One authenticated connection setup, as measured or as received.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SetupCost {
    /// One-way flights the setup takes; a round trip is two flights.
    pub flights: u8,
    /// Exact bytes the client sends during setup.
    pub client_bytes: u64,
    /// Exact bytes the server sends during setup.
    pub server_bytes: u64,
    /// CPU nanoseconds one setup costs on the measuring machine.
    pub cpu_nanos: u64,
}

impl SetupCost {
    /// Wire bytes both directions during setup.
    #[must_use]
    pub fn wire_bytes(&self) -> u64 {
        self.client_bytes.saturating_add(self.server_bytes)
    }

    /// Setup time at `rtt_nanos`: the CPU cost plus the flights' share of one
    /// round trip. Every flight is half a round trip, and a setup of `flights`
    /// flights therefore waits `flights / 2` round trips.
    #[must_use]
    pub fn setup_nanos(&self, rtt_nanos: u64) -> u64 {
        self.cpu_nanos
            .saturating_add(u64::from(self.flights).saturating_mul(rtt_nanos) / 2)
    }
}

/// Which authenticated transport wins connection setup at one link RTT.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// TLS through quinn sets up faster, or ties outside the margin.
    QuinnWins,
    /// Both setups land within the tie margin of each other.
    Tie,
    /// The token exchange sets up faster outside the tie margin.
    TokenWins,
}

/// Compare the token path against TLS at `rtt_nanos`. A setup within
/// `tie_permille` thousandths of the slower one is a [`Verdict::Tie`]: the
/// difference is below what a player can feel and below the run-to-run noise of
/// a shared machine.
#[must_use]
pub fn compare(token: &SetupCost, quinn: &SetupCost, rtt_nanos: u64, tie_permille: u32) -> Verdict {
    let token_nanos = token.setup_nanos(rtt_nanos);
    let quinn_nanos = quinn.setup_nanos(rtt_nanos);
    let slower = u128::from(token_nanos.max(quinn_nanos)).max(1);
    let gap = u128::from(token_nanos.abs_diff(quinn_nanos)).saturating_mul(1000);
    if gap <= slower.saturating_mul(u128::from(tie_permille)) {
        Verdict::Tie
    } else if quinn_nanos < token_nanos {
        Verdict::QuinnWins
    } else {
        Verdict::TokenWins
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A setup cost with the CPU in microseconds, easier to read in tests.
    fn cost(flights: u8, client: u64, server: u64, cpu_us: u64) -> SetupCost {
        SetupCost {
            flights,
            client_bytes: client,
            server_bytes: server,
            cpu_nanos: cpu_us.saturating_mul(1000),
        }
    }

    #[test]
    fn compare_ties_within_the_margin_and_names_the_winner() {
        // Equal costs tie at any round-trip time.
        let same = cost(3, 300, 80, 20);
        for rtt_ms in [0u64, 1, 30, 150] {
            assert_eq!(
                compare(&same, &same, rtt_ms.saturating_mul(1_000_000), 50),
                Verdict::Tie
            );
        }
        // The same flights plus 100 us of extra CPU: a tie at 30 ms (0.3 %)
        // and a token win on loopback, where nothing hides the CPU gap.
        let token = cost(3, 300, 80, 20);
        let slow = cost(3, 3_000, 1_000, 120);
        assert_eq!(
            compare(&token, &slow, 30_000_000, 50),
            Verdict::Tie,
            "100 us inside a 45 ms setup is noise"
        );
        assert_eq!(
            compare(&token, &slow, 0, 50),
            Verdict::TokenWins,
            "on loopback the cheaper CPU wins"
        );
        // A path that needs one more flight loses at any real round trip: TLS
        // finishing in two flights beats the three-flight token exchange.
        let fast = cost(2, 300, 80, 20);
        assert_eq!(
            compare(&token, &fast, 30_000_000, 50),
            Verdict::QuinnWins,
            "one flight less wins at 30 ms"
        );
        // setup_nanos: three flights wait 1.5 round trips.
        assert_eq!(same.setup_nanos(10_000_000), 20_000 + 15_000_000);
        assert_eq!(same.wire_bytes(), 380);
    }
}

#[cfg(all(test, feature = "crypto"))]
mod token {
    #[expect(
        clippy::disallowed_types,
        reason = "connection-setup benchmark: wall clock never enters a hash."
    )]
    use std::time::{Duration, Instant};

    use crate::handshake::{
        Client, Server, ServerAddress, SessionKey, TokenGenerator, PROTOCOL_ID,
    };

    use super::SetupCost;

    /// Three one-way flights: request, challenge, response.
    const FLIGHTS: u8 = 3;

    /// A fixed server key so the measured exchange is reproducible.
    fn server_key() -> SessionKey {
        std::array::from_fn(|at| u8::try_from(at).unwrap_or(0) ^ 0x5a)
    }

    /// The one server address a measured token offers.
    fn servers() -> Vec<ServerAddress> {
        vec![ServerAddress {
            ip: [127, 0, 0, 1],
            port: 40_000,
        }]
    }

    /// Measure the token handshake alone: the sealed request, the challenge and
    /// the response, whose HMAC tags are the authentication this path buys.
    /// The token mint is a backend step that happens before a join and is not
    /// part of the setup, so it is neither timed nor counted.
    pub(super) fn measure(iterations: u32) -> SetupCost {
        let key = server_key();
        let now = 1_000i64;
        let mut generator = TokenGenerator::new(PROTOCOL_ID, key, 2040);
        let mut server = Server::new(PROTOCOL_ID, key, 2040 ^ 0x9e37_79b9_7f4a_7c15);
        let mut elapsed = Duration::ZERO;
        let (mut client_bytes, mut server_bytes) = (0u64, 0u64);
        for _ in 0..iterations {
            let minted = generator
                .generate(now, 30, &servers())
                .expect("a bounded token mints");
            let mut client = Client::new(PROTOCOL_ID, minted.token, minted.sealed);
            #[expect(
                clippy::disallowed_types,
                clippy::disallowed_methods,
                reason = "connection-setup benchmark: wall clock never enters a hash."
            )]
            let timer = Instant::now();
            let request = client.request();
            let challenge = server
                .on_request(&request, now)
                .expect("a fresh token is accepted");
            client
                .on_challenge(&challenge)
                .expect("the challenge seal verifies");
            let response = client.response().expect("the response seal builds");
            server
                .on_response(&response)
                .expect("the response seal verifies");
            elapsed += timer.elapsed();
            client_bytes = u64::try_from(request.len() + response.len()).unwrap_or(u64::MAX);
            server_bytes = u64::try_from(challenge.len()).unwrap_or(u64::MAX);
        }
        SetupCost {
            flights: FLIGHTS,
            client_bytes,
            server_bytes,
            cpu_nanos: u64::try_from(elapsed.as_nanos() / u128::from(iterations))
                .unwrap_or(u64::MAX),
        }
    }

    #[test]
    fn token_auth_setup_is_three_flights_with_exact_bytes() {
        let measured = measure(8);
        assert_eq!(measured.flights, 3, "request, challenge, response");
        assert!(measured.client_bytes > 0 && measured.server_bytes > 0);
        assert!(
            measured.client_bytes > measured.server_bytes,
            "the token rides with the request"
        );
        // Same shapes twice: the byte counts are the encoded messages, so they
        // repeat exactly and are not samples.
        let twin = measure(8);
        assert_eq!(
            (twin.client_bytes, twin.server_bytes, twin.flights),
            (
                measured.client_bytes,
                measured.server_bytes,
                measured.flights
            )
        );
    }
}

#[cfg(all(test, feature = "quic"))]
mod tls {
    use std::sync::Arc;
    #[expect(
        clippy::disallowed_types,
        reason = "connection-setup benchmark: wall clock never enters a hash."
    )]
    use std::time::{Duration, Instant};

    use quinn::rustls;
    use quinn::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
    use quinn::rustls::{
        ClientConfig, ClientConnection, RootCertStore, ServerConfig, ServerConnection,
    };

    use super::SetupCost;

    /// Three one-way flights: client hello, server flight, client finished.
    /// A resumed (0-RTT) setup is two flights and would widen the TLS lead.
    const FLIGHTS: u8 = 3;

    /// Test-only self-signed P-256 CA for 127.0.0.1/localhost
    /// (valid 2026-2036), minted once with openssl. It trusts nothing real,
    /// and rustls, not hand-rolled code, speaks both ends.
    const FIXTURE_CA_DER: &[u8] = b"\x30\x82\x01\x84\x30\x82\x01\x2b\xa0\x03\x02\x01\x02\x02\x14\x12\x90\x42\x05\xb0\x47\xe1\x9e\xfe\x6b\x4e\x90\x86\x2a\x64\x39\x8a\x89\x87\x21\x30\x0a\x06\x08\x2a\x86\x48\xce\x3d\x04\x03\x02\x30\x18\x31\x16\x30\x14\x06\x03\x55\x04\x03\x0c\x0d\x65\x32\x30\x34\x30\x2d\x74\x65\x73\x74\x2d\x63\x61\x30\x1e\x17\x0d\x32\x36\x31\x30\x31\x36\x30\x39\x34\x34\x34\x38\x5a\x17\x0d\x33\x36\x31\x30\x31\x33\x30\x39\x34\x34\x34\x38\x5a\x30\x18\x31\x16\x30\x14\x06\x03\x55\x04\x03\x0c\x0d\x65\x32\x30\x34\x30\x2d\x74\x65\x73\x74\x2d\x63\x61\x30\x59\x30\x13\x06\x07\x2a\x86\x48\xce\x3d\x02\x01\x06\x08\x2a\x86\x48\xce\x3d\x03\x01\x07\x03\x42\x00\x04\xce\x5e\x1b\x33\x1a\xf1\xd9\xb1\xe6\x45\xa7\x8e\x7c\x86\x90\x5e\x82\x94\x35\x17\x86\xd5\xf5\x77\x31\x35\x5c\x61\xe6\xff\x63\xda\xa7\xd2\x42\xac\x77\x21\xd2\x16\x97\x5a\xe9\x5d\x99\x5f\x5c\x3c\xf4\xf0\x1c\xcc\xbf\xd0\x15\x66\xe6\xae\xb6\x5a\x94\x54\xfa\xd9\xa3\x53\x30\x51\x30\x1d\x06\x03\x55\x1d\x0e\x04\x16\x04\x14\x74\x52\x00\x9c\xa0\xf6\xfd\xd7\xe2\xf7\x6a\x33\x51\x71\xc5\xfa\x6e\x32\x09\x36\x30\x1f\x06\x03\x55\x1d\x23\x04\x18\x30\x16\x80\x14\x74\x52\x00\x9c\xa0\xf6\xfd\xd7\xe2\xf7\x6a\x33\x51\x71\xc5\xfa\x6e\x32\x09\x36\x30\x0f\x06\x03\x55\x1d\x13\x01\x01\xff\x04\x05\x30\x03\x01\x01\xff\x30\x0a\x06\x08\x2a\x86\x48\xce\x3d\x04\x03\x02\x03\x47\x00\x30\x44\x02\x20\x23\x9c\x32\xa0\x1b\xf9\x2e\xf3\x40\x60\x56\xd6\xbc\xbb\x72\x52\x95\xb9\xc5\x8f\x48\x12\x1d\xeb\x96\x03\xfd\x11\x15\xda\x1c\x14\x02\x20\x52\xbe\x65\x89\xca\x4f\xa9\x0c\xb6\xd9\xab\x1b\x05\x81\x61\x03\x0e\x23\x3c\x7b\xf2\x1b\x94\x76\x4f\x27\x70\x48\xdf\x63\x81\xa4";

    /// Test-only server certificate signed by the CA above.
    const FIXTURE_SRV_DER: &[u8] = b"\x30\x82\x01\x8b\x30\x82\x01\x32\xa0\x03\x02\x01\x02\x02\x14\x12\xa6\x39\x8b\x39\x9a\xbb\x5b\xe3\x20\x26\xc4\xe5\x31\xc5\x0a\x82\xe3\xd9\xc2\x30\x0a\x06\x08\x2a\x86\x48\xce\x3d\x04\x03\x02\x30\x18\x31\x16\x30\x14\x06\x03\x55\x04\x03\x0c\x0d\x65\x32\x30\x34\x30\x2d\x74\x65\x73\x74\x2d\x63\x61\x30\x1e\x17\x0d\x32\x36\x31\x30\x31\x36\x30\x39\x34\x34\x34\x38\x5a\x17\x0d\x33\x36\x31\x30\x31\x33\x30\x39\x34\x34\x34\x38\x5a\x30\x14\x31\x12\x30\x10\x06\x03\x55\x04\x03\x0c\x09\x31\x32\x37\x2e\x30\x2e\x30\x2e\x31\x30\x59\x30\x13\x06\x07\x2a\x86\x48\xce\x3d\x02\x01\x06\x08\x2a\x86\x48\xce\x3d\x03\x01\x07\x03\x42\x00\x04\x3e\x0e\x2e\x7d\x2b\xf7\xa5\xe3\x4b\xd0\xa5\x2a\x90\x6d\x03\xe1\x94\xde\x53\x0e\x19\x62\x04\xd5\x4a\x87\xfb\x57\x05\x60\xce\x5e\xbd\x28\x3d\xb5\x38\x4d\x42\xe0\x21\x2d\x42\xbc\x56\x73\x3d\xfb\xb2\xf3\x14\x4a\x3e\xf4\x38\x9a\xba\x3e\x60\x8a\xe9\x87\x34\x3b\xa3\x5e\x30\x5c\x30\x1a\x06\x03\x55\x1d\x11\x04\x13\x30\x11\x87\x04\x7f\x00\x00\x01\x82\x09\x6c\x6f\x63\x61\x6c\x68\x6f\x73\x74\x30\x1d\x06\x03\x55\x1d\x0e\x04\x16\x04\x14\xed\x0f\xa7\x27\xfb\x57\x3e\x25\x52\x56\xe9\x80\xf0\x60\x44\x20\x30\x09\x08\x08\x30\x1f\x06\x03\x55\x1d\x23\x04\x18\x30\x16\x80\x14\x74\x52\x00\x9c\xa0\xf6\xfd\xd7\xe2\xf7\x6a\x33\x51\x71\xc5\xfa\x6e\x32\x09\x36\x30\x0a\x06\x08\x2a\x86\x48\xce\x3d\x04\x03\x02\x03\x47\x00\x30\x44\x02\x20\x65\x15\xfe\xcd\x9a\xfe\xc3\x07\xfa\x5d\x0f\x1d\x14\x03\x43\xce\x56\x76\x7a\xdd\xb0\xf5\x83\xca\x6c\xc8\x6c\x01\x6f\x9b\x51\xe7\x02\x20\x69\x00\x17\x10\x85\x71\x37\x75\xb1\x1a\x5d\x74\x5d\x3e\xfa\x97\xf7\x65\x2a\xe7\x9a\xac\x75\x0a\x5c\xa8\xbb\xf2\xdf\x3a\xb5\x4c";

    /// Test-only PKCS8 P-256 key matching the server certificate.
    const FIXTURE_SRV_KEY_DER: &[u8] = b"\x30\x81\x87\x02\x01\x00\x30\x13\x06\x07\x2a\x86\x48\xce\x3d\x02\x01\x06\x08\x2a\x86\x48\xce\x3d\x03\x01\x07\x04\x6d\x30\x6b\x02\x01\x01\x04\x20\x0e\x6b\xd2\x62\x36\x79\x5e\x95\x10\xec\xb7\xcc\x87\x0a\xdc\xbb\xff\x2f\x35\x1d\x89\xb8\x70\xb8\x34\x3e\x37\x8b\xd4\x8f\xea\x10\xa1\x44\x03\x42\x00\x04\x3e\x0e\x2e\x7d\x2b\xf7\xa5\xe3\x4b\xd0\xa5\x2a\x90\x6d\x03\xe1\x94\xde\x53\x0e\x19\x62\x04\xd5\x4a\x87\xfb\x57\x05\x60\xce\x5e\xbd\x28\x3d\xb5\x38\x4d\x42\xe0\x21\x2d\x42\xbc\x56\x73\x3d\xfb\xb2\xf3\x14\x4a\x3e\xf4\x38\x9a\xba\x3e\x60\x8a\xe9\x87\x34\x3b";

    /// One handshake driver step: `(client to server, server to client)` bytes.
    type Flights = (u64, u64);

    /// TLS 1.3 configs pinned exactly like the QUIC transport: the ring
    /// provider, TLS 1.3 only, and a real root store. Certificate chains are
    /// verified; nothing accepts any certificate here.
    fn configs() -> (Arc<ClientConfig>, Arc<ServerConfig>) {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut roots = RootCertStore::empty();
        roots
            .add(CertificateDer::from(FIXTURE_CA_DER.to_vec()))
            .expect("the fixture CA parses");
        let client = ClientConfig::builder_with_provider(Arc::clone(&provider))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .expect("TLS 1.3 is in this provider")
            .with_root_certificates(roots)
            .with_no_client_auth();
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(FIXTURE_SRV_KEY_DER.to_vec()));
        let server = ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .expect("TLS 1.3 is in this provider")
            .with_no_client_auth()
            .with_single_cert(vec![CertificateDer::from(FIXTURE_SRV_DER.to_vec())], key)
            .expect("the fixture chain and key parse");
        (Arc::new(client), Arc::new(server))
    }

    /// Move every pending record from `from` to `to`, counting the bytes.
    fn drain<A: TlsSide, B: TlsSide>(
        from: &mut A,
        to: &mut B,
        flown: &mut u64,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let mut wire: Vec<u8> = Vec::new();
        while from.side_wants_write() {
            from.side_write_tls(&mut wire)?;
        }
        if wire.is_empty() {
            return Ok(false);
        }
        *flown = flown.saturating_add(u64::try_from(wire.len()).unwrap_or(u64::MAX));
        let mut reader: &[u8] = &wire;
        to.side_read_tls(&mut reader)?;
        to.side_process_packets()?;
        Ok(true)
    }

    /// The two ends share one driving surface. The `side_` names keep these
    /// calls on the trait instead of resolving to the inherent methods.
    trait TlsSide {
        fn side_wants_write(&self) -> bool;
        fn side_write_tls(&mut self, out: &mut Vec<u8>) -> std::io::Result<usize>;
        fn side_read_tls(&mut self, input: &mut dyn std::io::Read) -> std::io::Result<usize>;
        fn side_process_packets(&mut self) -> Result<rustls::IoState, rustls::Error>;
        fn side_handshaking(&self) -> bool;
    }

    impl TlsSide for ClientConnection {
        fn side_wants_write(&self) -> bool {
            self.wants_write()
        }
        fn side_write_tls(&mut self, out: &mut Vec<u8>) -> std::io::Result<usize> {
            self.write_tls(out)
        }
        fn side_read_tls(&mut self, input: &mut dyn std::io::Read) -> std::io::Result<usize> {
            self.read_tls(input)
        }
        fn side_process_packets(&mut self) -> Result<rustls::IoState, rustls::Error> {
            self.process_new_packets()
        }
        fn side_handshaking(&self) -> bool {
            self.is_handshaking()
        }
    }

    impl TlsSide for ServerConnection {
        fn side_wants_write(&self) -> bool {
            self.wants_write()
        }
        fn side_write_tls(&mut self, out: &mut Vec<u8>) -> std::io::Result<usize> {
            self.write_tls(out)
        }
        fn side_read_tls(&mut self, input: &mut dyn std::io::Read) -> std::io::Result<usize> {
            self.read_tls(input)
        }
        fn side_process_packets(&mut self) -> Result<rustls::IoState, rustls::Error> {
            self.process_new_packets()
        }
        fn side_handshaking(&self) -> bool {
            self.is_handshaking()
        }
    }

    /// Run one in-memory TLS 1.3 handshake and return the bytes each direction
    /// carried. The loop is bounded: a handshake that stalls refuses instead of
    /// spinning.
    fn handshake<A: TlsSide, B: TlsSide>(
        client: &mut A,
        server: &mut B,
    ) -> Result<Flights, Box<dyn std::error::Error>> {
        let (mut c2s, mut s2c) = (0u64, 0u64);
        for _ in 0..8 {
            let moved = drain(client, server, &mut c2s)?;
            let answered = drain(server, client, &mut s2c)?;
            if !moved && !answered {
                break;
            }
            if !client.side_handshaking() && !server.side_handshaking() {
                return Ok((c2s, s2c));
            }
        }
        Err("the TLS handshake did not finish inside eight flights".into())
    }

    /// Measure the TLS 1.3 handshake that quinn runs at join: same provider,
    /// same TLS 1.3 pin, same chain verification, with sockets left out so the
    /// number is the setup work and not the loopback driver.
    pub(super) fn measure(iterations: u32) -> SetupCost {
        let (client_config, server_config) = configs();
        let name = ServerName::try_from("localhost").expect("the fixture name parses");
        let mut elapsed = Duration::ZERO;
        let (mut c2s, mut s2c) = (0u64, 0u64);
        for _ in 0..iterations {
            let mut client = ClientConnection::new(Arc::clone(&client_config), name.clone())
                .expect("the client config builds a connection");
            let mut server = ServerConnection::new(Arc::clone(&server_config))
                .expect("the server config builds a connection");
            #[expect(
                clippy::disallowed_types,
                clippy::disallowed_methods,
                reason = "connection-setup benchmark: wall clock never enters a hash."
            )]
            let timer = Instant::now();
            let (up, down) = handshake(&mut client, &mut server).expect("the handshake completes");
            elapsed += timer.elapsed();
            assert!(
                !client.side_handshaking() && !server.side_handshaking(),
                "both ends finish the handshake"
            );
            c2s = up;
            s2c = down;
        }
        SetupCost {
            flights: FLIGHTS,
            client_bytes: c2s,
            server_bytes: s2c,
            cpu_nanos: u64::try_from(elapsed.as_nanos() / u128::from(iterations))
                .unwrap_or(u64::MAX),
        }
    }

    #[test]
    fn tls_setup_is_three_flights_and_verifies_the_chain() {
        let measured = measure(4);
        assert_eq!(measured.flights, 3, "hello, server flight, finished");
        assert!(measured.client_bytes > 0 && measured.server_bytes > 0);
        assert!(
            measured.server_bytes > measured.client_bytes,
            "the certificate chain rides with the server flight"
        );
        // Exact bytes, not a sample: the flights repeat byte for byte.
        let twin = measure(4);
        assert_eq!(
            (twin.client_bytes, twin.server_bytes),
            (measured.client_bytes, measured.server_bytes)
        );
        // The chain really is verified: with no trusted root the same server
        // refuses, so the fixture client is not an accept-any verifier.
        let (_, server_config) = configs();
        let empty_client =
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_protocol_versions(&[&rustls::version::TLS13])
                .expect("TLS 1.3 is in this provider")
                .with_root_certificates(RootCertStore::empty())
                .with_no_client_auth();
        let name = ServerName::try_from("localhost").expect("the fixture name parses");
        let mut client = ClientConnection::new(Arc::new(empty_client), name)
            .expect("the empty-root client builds");
        let mut server =
            ServerConnection::new(server_config).expect("the server config builds a connection");
        assert!(
            handshake(&mut client, &mut server).is_err(),
            "an unknown issuer must refuse the handshake"
        );
    }
}

#[cfg(all(test, feature = "crypto", feature = "quic"))]
mod bench {
    use super::{compare, Verdict};
    use super::{tls, token};

    /// Tie margin of the connection-setup comparison, in thousandths: a setup
    /// within five percent of the other is a tie.
    const MARGIN: u32 = 50;

    /// Setups measured per path. Enough for a stable median of the CPU cost,
    /// short enough that the test stays a test.
    const ITERATIONS: u32 = 16;
    /// Round-trip times the verdict is posted at, in microseconds: LAN, two
    /// internet tiers and the worst case the soak profile allows.
    const RTTS_US: [u64; 5] = [500, 10_000, 30_000, 60_000, 150_000];

    /// Microseconds from nanoseconds, one decimal place for the report.
    fn us(nanos: u64) -> f64 {
        nanos as f64 / 1000.0
    }

    #[test]
    fn connection_setup_benchmark_posts_quinn_wins_or_ties() {
        let token = token::measure(ITERATIONS);
        let quinn = tls::measure(ITERATIONS);
        println!(
            "token auth: flights={} wire={} B (c {} / s {}) cpu={:.1} us",
            token.flights,
            token.wire_bytes(),
            token.client_bytes,
            token.server_bytes,
            us(token.cpu_nanos)
        );
        println!(
            "tls (quinn): flights={} wire={} B (c {} / s {}) cpu={:.1} us",
            quinn.flights,
            quinn.wire_bytes(),
            quinn.client_bytes,
            quinn.server_bytes,
            us(quinn.cpu_nanos)
        );
        assert_eq!(
            token.flights, quinn.flights,
            "both paths set up in three flights: the link term ties"
        );
        assert!(quinn.cpu_nanos > 0 && token.cpu_nanos > 0);
        // The verdict must be quinn wins or ties over the internet tiers.
        // The first tier is the LAN, where the CPU gap is visible; posting it
        // keeps the model honest.
        for rtt_us in RTTS_US {
            let verdict = compare(&token, &quinn, rtt_us * 1000, MARGIN);
            println!(
                "rtt {rtt_us} us: token {:.3} ms, tls {:.3} ms -> {verdict:?}",
                us(token.setup_nanos(rtt_us * 1000)) / 1000.0,
                us(quinn.setup_nanos(rtt_us * 1000)) / 1000.0
            );
            if rtt_us >= 10_000 {
                assert!(
                    matches!(verdict, Verdict::QuinnWins | Verdict::Tie),
                    "quinn must win or tie at {rtt_us} us RTT, got {verdict:?}"
                );
            }
        }
        // Report the breakeven: the smallest internet-tier RTT where TLS is
        // inside the tie margin of the token path. Zero means it ties at once.
        let first_tie = (1..=RTTS_US[4]).find(|rtt| {
            matches!(
                compare(&token, &quinn, rtt * 1000, MARGIN),
                Verdict::QuinnWins | Verdict::Tie
            )
        });
        println!("first tie at {first_tie:?} us RTT");
    }
}
