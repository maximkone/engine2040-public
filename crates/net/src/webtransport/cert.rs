//! cert: the pinned self-signed certificate a browser accepts through
//! `serverCertificateHashes`.
//!
//! The browser rules: X.509v3, an ECDSA P-256 key, a validity period of at
//! most two weeks that contains the current time, and a pin that is the
//! SHA-256 of the DER of the leaf certificate (not the hash of its public key
//! info). [`PinnedCert::mint`] makes such a certificate; [`check_pinnable`]
//! enforces the same rules on any DER and is cross-checked against the
//! `wtransport` client verifier in the tests. A pinned certificate dies after
//! at most 14 days, so every run mints a fresh one.

use std::fmt;

use wtransport::tls::self_signed::time::{Duration, OffsetDateTime};
use wtransport::tls::{Certificate, Identity};

/// Longest validity period a browser accepts for a pinned certificate.
pub const MAX_VALIDITY_DAYS: u32 = 14;
/// The same limit in seconds.
const MAX_VALIDITY_SECS: i64 = 14 * 86_400;
/// `notBefore` sits this far in the past so a client clock that runs a little
/// behind still sees a started certificate. The period stays inside the limit
/// because the requested days are counted from the backdated start.
const BACKDATE_HOURS: i64 = 1;

const TAG_INTEGER: u8 = 0x02;
const TAG_OID: u8 = 0x06;
const TAG_UTC_TIME: u8 = 0x17;
const TAG_GENERALIZED_TIME: u8 = 0x18;
const TAG_SEQUENCE: u8 = 0x30;
/// Explicit `[0]` tag of the optional certificate version.
const TAG_VERSION: u8 = 0xA0;
/// OID 1.2.840.10045.2.1, id-ecPublicKey.
const OID_EC_PUBLIC_KEY: [u8; 7] = [0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x02, 0x01];
/// OID 1.2.840.10045.3.1.7, prime256v1 (NIST P-256).
const OID_PRIME256V1: [u8; 8] = [0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x03, 0x01, 0x07];

/// Why a certificate cannot be pinned or could not be made.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CertError {
    /// The requested validity is zero days or over [`MAX_VALIDITY_DAYS`].
    BadValidityDays(u32),
    /// A subject alternative name is not a valid ASN.1 string.
    BadSan,
    /// The identity holds no certificate.
    BadChain,
    /// The bytes are not a certificate this reader understands.
    Malformed,
    /// The public key is not ECDSA P-256: RSA, another curve or another family.
    NotEcdsaP256,
    /// `notAfter` minus `notBefore` is longer than [`MAX_VALIDITY_DAYS`].
    ValidityTooLong {
        /// The certificate's validity period in seconds.
        seconds: i64,
    },
    /// The current time is before `notBefore`.
    NotYetValid,
    /// The current time is after `notAfter`.
    Expired,
}

impl fmt::Display for CertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::BadValidityDays(days) => {
                write!(
                    f,
                    "validity of {days} days is outside 1..={MAX_VALIDITY_DAYS}"
                )
            }
            Self::BadSan => write!(f, "invalid subject alternative name"),
            Self::BadChain => write!(f, "identity holds no certificate"),
            Self::Malformed => write!(f, "not a readable X.509 certificate"),
            Self::NotEcdsaP256 => write!(f, "public key is not ECDSA P-256"),
            Self::ValidityTooLong { seconds } => write!(
                f,
                "validity of {seconds} s exceeds the {MAX_VALIDITY_DAYS}-day browser limit"
            ),
            Self::NotYetValid => write!(f, "certificate is not valid yet"),
            Self::Expired => write!(f, "certificate has expired"),
        }
    }
}

impl std::error::Error for CertError {}

/// The validity period of a certificate in Unix seconds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Validity {
    /// First second the certificate is valid.
    pub not_before: i64,
    /// Last second the certificate is valid.
    pub not_after: i64,
}

/// A self-signed identity that passed [`check_pinnable`], with the SHA-256 of
/// its leaf DER ready for `serverCertificateHashes`.
pub struct PinnedCert {
    identity: Identity,
    der: Vec<u8>,
    hash: [u8; 32],
    validity: Validity,
}

impl PinnedCert {
    /// Mint a fresh ECDSA P-256 self-signed certificate for `subject_alt_names`
    /// valid for `valid_days` (1 to [`MAX_VALIDITY_DAYS`]). Zero or longer is
    /// refused before any key is made.
    pub fn mint<I, S>(subject_alt_names: I, valid_days: u32) -> Result<Self, CertError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        if valid_days == 0 || valid_days > MAX_VALIDITY_DAYS {
            return Err(CertError::BadValidityDays(valid_days));
        }
        let now = OffsetDateTime::now_utc();
        let identity = Identity::self_signed_builder()
            .subject_alt_names(subject_alt_names)
            .not_before(now - Duration::hours(BACKDATE_HOURS))
            .validity_days(valid_days)
            .build()
            .map_err(|_| CertError::BadSan)?;
        Self::from_identity(identity, now.unix_timestamp())
    }

    /// Wrap an existing identity (for example one loaded from PEM files) after
    /// the pinning rules hold at `now_unix`. The leaf is the first certificate
    /// of the chain.
    pub fn from_identity(identity: Identity, now_unix: i64) -> Result<Self, CertError> {
        let der = identity
            .certificate_chain()
            .as_slice()
            .first()
            .ok_or(CertError::BadChain)?
            .der()
            .to_vec();
        let validity = check_pinnable(&der, now_unix)?;
        let hash = leaf_hash(&der)?;
        Ok(Self {
            identity,
            der,
            hash,
            validity,
        })
    }

    /// The identity the server presents.
    #[must_use]
    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    /// DER of the leaf certificate.
    #[must_use]
    pub fn leaf_der(&self) -> &[u8] {
        &self.der
    }

    /// SHA-256 of the leaf DER: the value `serverCertificateHashes` takes.
    #[must_use]
    pub fn hash(&self) -> [u8; 32] {
        self.hash
    }

    /// The pin as 64 lowercase hex digits, for pages and logs.
    #[must_use]
    pub fn hash_hex(&self) -> String {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(64);
        for byte in self.hash {
            out.push(char::from(DIGITS[usize::from(byte >> 4)]));
            out.push(char::from(DIGITS[usize::from(byte & 0x0F)]));
        }
        out
    }

    /// The validity period of the leaf.
    #[must_use]
    pub fn validity(&self) -> Validity {
        self.validity
    }

    /// The leaf certificate as PEM.
    #[must_use]
    pub fn cert_pem(&self) -> String {
        self.identity
            .certificate_chain()
            .as_slice()
            .first()
            .map(Certificate::to_pem)
            .unwrap_or_default()
    }

    /// The private key as PEM. Secret: never log it.
    #[must_use]
    pub fn key_pem(&self) -> String {
        self.identity.private_key().to_secret_pem()
    }
}

/// SHA-256 of the DER of a leaf certificate. This hashes the whole
/// certificate, so changing any byte (the signature included) changes the pin;
/// the hash of the public key info is a different value that browsers do not
/// accept here.
pub fn leaf_hash(der: &[u8]) -> Result<[u8; 32], CertError> {
    let certificate = Certificate::from_der(der.to_vec()).map_err(|_| CertError::Malformed)?;
    let digest = certificate.hash();
    Ok(*digest.as_ref())
}

/// Enforce the browser pinning rules on a DER certificate at `now_unix`:
/// ECDSA P-256 key, validity period at most [`MAX_VALIDITY_DAYS`], and `now`
/// inside the period. Returns the validity period.
pub fn check_pinnable(der: &[u8], now_unix: i64) -> Result<Validity, CertError> {
    let facts = read_facts(der).ok_or(CertError::Malformed)?;
    if !facts.ecdsa_p256 {
        return Err(CertError::NotEcdsaP256);
    }
    let validity = facts.validity;
    let seconds = validity.not_after - validity.not_before;
    if seconds < 0 {
        return Err(CertError::Malformed);
    }
    if seconds > MAX_VALIDITY_SECS {
        return Err(CertError::ValidityTooLong { seconds });
    }
    if now_unix < validity.not_before {
        return Err(CertError::NotYetValid);
    }
    if now_unix > validity.not_after {
        return Err(CertError::Expired);
    }
    Ok(validity)
}

/// What the pinning rules need from a certificate.
struct Facts {
    ecdsa_p256: bool,
    validity: Validity,
}

/// Read the TBSCertificate fields in order, skipping what the rules ignore:
/// `[0]` version (optional), serial, signature algorithm, issuer, validity,
/// subject, then the public key algorithm.
fn read_facts(der: &[u8]) -> Option<Facts> {
    let (certificate, trailing) = take(der, TAG_SEQUENCE)?;
    if !trailing.is_empty() {
        return None;
    }
    let (tbs, _) = take(certificate, TAG_SEQUENCE)?;
    let mut cursor = tbs;
    if cursor.first() == Some(&TAG_VERSION) {
        cursor = take(cursor, TAG_VERSION)?.1;
    }
    cursor = take(cursor, TAG_INTEGER)?.1;
    cursor = take(cursor, TAG_SEQUENCE)?.1;
    cursor = take(cursor, TAG_SEQUENCE)?.1;
    let (validity, cursor) = take(cursor, TAG_SEQUENCE)?;
    let cursor = take(cursor, TAG_SEQUENCE)?.1;
    let (public_key_info, _) = take(cursor, TAG_SEQUENCE)?;
    let (algorithm, _) = take(public_key_info, TAG_SEQUENCE)?;
    let (key_oid, parameters) = take(algorithm, TAG_OID)?;
    // RSA carries NULL parameters instead of a curve OID: that is a wrong
    // key family, not a malformed certificate.
    let curve_is_p256 = take(parameters, TAG_OID).is_some_and(|(curve, _)| curve == OID_PRIME256V1);
    let (not_before_tag, not_before, rest) = read_tlv(validity)?;
    let (not_after_tag, not_after, rest) = read_tlv(rest)?;
    if !rest.is_empty() {
        return None;
    }
    Some(Facts {
        ecdsa_p256: key_oid == OID_EC_PUBLIC_KEY && curve_is_p256,
        validity: Validity {
            not_before: parse_time(not_before_tag, not_before)?,
            not_after: parse_time(not_after_tag, not_after)?,
        },
    })
}

/// One DER element: `(tag, content, rest)`. Only definite lengths up to four
/// bytes and single-byte tags are read; anything else is refused.
fn read_tlv(input: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = input.split_first()?;
    if tag & 0x1F == 0x1F {
        return None;
    }
    let (&first, rest) = rest.split_first()?;
    let (length, rest) = if first < 0x80 {
        (usize::from(first), rest)
    } else {
        let count = usize::from(first & 0x7F);
        if count == 0 || count > 4 {
            return None;
        }
        let (digits, rest) = rest.split_at_checked(count)?;
        let length = digits
            .iter()
            .fold(0usize, |acc, &byte| (acc << 8) | usize::from(byte));
        (length, rest)
    };
    let (content, rest) = rest.split_at_checked(length)?;
    Some((tag, content, rest))
}

/// `(content, rest)` of the next element when it carries `tag`.
fn take(input: &[u8], tag: u8) -> Option<(&[u8], &[u8])> {
    let (found, content, rest) = read_tlv(input)?;
    (found == tag).then_some((content, rest))
}

/// Unix seconds of a DER `UTCTime` (`YYMMDDHHMMSSZ`, two-digit years from 50
/// are 19xx) or `GeneralizedTime` (`YYYYMMDDHHMMSSZ`). Only the exact DER
/// shapes with seconds and `Z` are accepted.
fn parse_time(tag: u8, text: &[u8]) -> Option<i64> {
    let (year, rest) = match tag {
        TAG_UTC_TIME => {
            let (short, rest) = digits(text, 2)?;
            (
                if short >= 50 {
                    1900 + short
                } else {
                    2000 + short
                },
                rest,
            )
        }
        TAG_GENERALIZED_TIME => digits(text, 4)?,
        _ => return None,
    };
    let (month, rest) = digits(rest, 2)?;
    let (day, rest) = digits(rest, 2)?;
    let (hour, rest) = digits(rest, 2)?;
    let (minute, rest) = digits(rest, 2)?;
    let (second, rest) = digits(rest, 2)?;
    if rest != b"Z" || !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    if day < 1 || day > days_in_month(year, month) {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// `count` ASCII digits as a number, plus the text after them.
fn digits(text: &[u8], count: usize) -> Option<(i64, &[u8])> {
    let (head, rest) = text.split_at_checked(count)?;
    let mut value = 0i64;
    for &byte in head {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value * 10 + i64::from(byte - b'0');
    }
    Some((value, rest))
}

/// Days in `month` (1 to 12) of `year` in the proleptic Gregorian calendar.
fn days_in_month(year: i64, month: i64) -> i64 {
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date (month 1 to 12).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use wtransport::tls::client::ServerHashVerification;
    use wtransport::tls::rustls::client::danger::ServerCertVerifier;
    use wtransport::tls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use wtransport::tls::Sha256Digest;

    const P256_14D: &[u8] = include_bytes!("../../testdata/webtransport/p256-14d.der");
    const P256_15D: &[u8] = include_bytes!("../../testdata/webtransport/p256-15d.der");
    const P256_2050: &[u8] = include_bytes!("../../testdata/webtransport/p256-2050.der");
    const P384_14D: &[u8] = include_bytes!("../../testdata/webtransport/p384-14d.der");
    const RSA_14D: &[u8] = include_bytes!("../../testdata/webtransport/rsa-14d.der");

    /// 2026-10-01T00:00:00Z, the fixtures' `notBefore`.
    const FIXTURE_START: i64 = 1_790_812_800;
    /// 2026-10-15T00:00:00Z, the 14-day fixtures' `notAfter`.
    const FIXTURE_END_14D: i64 = 1_792_022_400;
    /// 2026-10-08T00:00:00Z, inside every 2026 fixture.
    const FIXTURE_MID: i64 = 1_791_417_600;

    /// SHA-256 of `p256-14d.der`, computed outside this crate with
    /// `openssl dgst -sha256` and node's crypto module.
    const P256_14D_SHA256: &str =
        "a32c5d0fc1fcd46693d67b38172f204ba193020f828528e0f06b68d1117e5da4";

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn webtransport_cert_hash_matches_an_independent_sha256_of_the_der() {
        let hash = leaf_hash(P256_14D).expect("fixture parses");
        assert_eq!(hash.len(), 32, "SHA-256 is 32 bytes");
        assert_eq!(hex(&hash), P256_14D_SHA256);
    }

    #[test]
    fn webtransport_cert_hash_is_stable_and_covers_the_whole_der() {
        assert_eq!(
            leaf_hash(P256_14D),
            leaf_hash(P256_14D),
            "same DER, same pin"
        );
        assert_ne!(
            leaf_hash(P256_14D),
            leaf_hash(P256_15D),
            "different certificates, different pins"
        );
        // The last byte sits in the signature, outside the public key info: a
        // pin over the key info would not move, the leaf pin must.
        let mut tampered = P256_14D.to_vec();
        *tampered.last_mut().expect("fixture is not empty") ^= 0x01;
        assert_ne!(leaf_hash(&tampered), leaf_hash(P256_14D));
    }

    #[test]
    fn webtransport_cert_refuses_rsa_and_other_curves() {
        for der in [RSA_14D, P384_14D] {
            assert_eq!(
                check_pinnable(der, FIXTURE_MID),
                Err(CertError::NotEcdsaP256)
            );
        }
        // The control: the same dates and shape with a P-256 key pass.
        assert!(check_pinnable(P256_14D, FIXTURE_MID).is_ok());
    }

    #[test]
    fn webtransport_cert_refuses_validity_over_14_days_and_accepts_exactly_14() {
        assert_eq!(
            check_pinnable(P256_15D, FIXTURE_MID),
            Err(CertError::ValidityTooLong {
                seconds: 15 * 86_400
            })
        );
        assert_eq!(
            check_pinnable(P256_14D, FIXTURE_MID),
            Ok(Validity {
                not_before: FIXTURE_START,
                not_after: FIXTURE_END_14D
            })
        );
    }

    #[test]
    fn webtransport_cert_requires_the_current_time_inside_the_period() {
        assert_eq!(
            check_pinnable(P256_14D, FIXTURE_START - 1),
            Err(CertError::NotYetValid)
        );
        assert!(check_pinnable(P256_14D, FIXTURE_START).is_ok());
        assert!(check_pinnable(P256_14D, FIXTURE_END_14D).is_ok());
        assert_eq!(
            check_pinnable(P256_14D, FIXTURE_END_14D + 1),
            Err(CertError::Expired)
        );
    }

    #[test]
    fn webtransport_cert_reads_generalized_time_after_2049() {
        // 2050-01-01T00:00:00Z to 2050-01-15T00:00:00Z.
        assert_eq!(
            check_pinnable(P256_2050, 2_525_000_000),
            Ok(Validity {
                not_before: 2_524_608_000,
                not_after: 2_525_817_600
            })
        );
    }

    #[test]
    fn webtransport_cert_time_parser_pins_both_encodings_and_refuses_junk() {
        assert_eq!(parse_time(TAG_UTC_TIME, b"700101000000Z"), Some(0));
        assert_eq!(
            parse_time(TAG_UTC_TIME, b"991231235959Z"),
            Some(946_684_799),
            "99 is 1999"
        );
        assert_eq!(
            parse_time(TAG_UTC_TIME, b"491231235959Z"),
            Some(2_524_607_999),
            "49 is 2049"
        );
        assert_eq!(
            parse_time(TAG_GENERALIZED_TIME, b"20500101000000Z"),
            Some(2_524_608_000)
        );
        assert_eq!(
            parse_time(TAG_UTC_TIME, b"280229000000Z"),
            Some(1_835_395_200),
            "2028 is a leap year"
        );
        for junk in [
            &b"261301000000Z"[..],
            b"261032000000Z",
            b"260230000000Z",
            b"270229000000Z",
            b"261001240000Z",
            b"261001006000Z",
            b"261001000060Z",
            b"261001000000",
            b"261001000000+0100",
            b"26100100000aZ",
            b"2610010000Z",
            b"",
        ] {
            assert_eq!(parse_time(TAG_UTC_TIME, junk), None, "{junk:?}");
        }
        assert_eq!(parse_time(0x04, b"261001000000Z"), None, "wrong tag");
    }

    #[test]
    fn webtransport_cert_reader_never_panics_on_damaged_input() {
        // Every proper prefix is malformed, never a panic or a pass.
        for length in 0..P256_14D.len() {
            assert_eq!(
                check_pinnable(&P256_14D[..length], FIXTURE_MID),
                Err(CertError::Malformed),
                "prefix of {length} bytes"
            );
        }
        // Every single-byte corruption returns some verdict without panicking.
        for index in 0..P256_14D.len() {
            let mut damaged = P256_14D.to_vec();
            damaged[index] ^= 0xFF;
            let _ = check_pinnable(&damaged, FIXTURE_MID);
        }
        // Trailing bytes after the certificate are refused.
        let mut padded = P256_14D.to_vec();
        padded.push(0);
        assert_eq!(
            check_pinnable(&padded, FIXTURE_MID),
            Err(CertError::Malformed)
        );
        // Indefinite and five-byte lengths are not DER.
        assert_eq!(read_tlv(&[0x30, 0x80, 0x00, 0x00]), None);
        assert_eq!(read_tlv(&[0x30, 0x85, 1, 0, 0, 0, 0]), None);
    }

    #[test]
    fn webtransport_cert_mint_refuses_bad_validity_before_making_a_key() {
        for days in [0, MAX_VALIDITY_DAYS + 1, u32::MAX] {
            assert!(matches!(
                PinnedCert::mint(["localhost"], days),
                Err(CertError::BadValidityDays(refused)) if refused == days
            ));
        }
    }

    #[test]
    fn webtransport_cert_mint_makes_a_pinnable_fresh_certificate() {
        let first = PinnedCert::mint(["localhost", "127.0.0.1"], MAX_VALIDITY_DAYS)
            .expect("14 days is the longest allowed");
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let validity = check_pinnable(first.leaf_der(), now).expect("minted cert passes the rules");
        assert_eq!(validity, first.validity());
        assert!(
            validity.not_after - validity.not_before <= MAX_VALIDITY_SECS,
            "never longer than the browser limit"
        );
        assert!(validity.not_before < now, "backdated for clock skew");
        assert_eq!(
            leaf_hash(first.leaf_der()),
            Ok(first.hash()),
            "the pin is the leaf DER hash"
        );
        assert_eq!(first.hash_hex().len(), 64);
        assert_eq!(first.hash_hex(), hex(&first.hash()));
        assert!(first.cert_pem().starts_with("-----BEGIN CERTIFICATE-----"));
        // Split so the source never holds a whole key marker.
        assert!(first
            .key_pem()
            .starts_with(concat!("-----BEGIN ", "PRIVATE KEY-----")));
        let second = PinnedCert::mint(["localhost"], 1).expect("one day is allowed");
        assert_ne!(first.hash(), second.hash(), "fresh key, fresh pin");
    }

    #[test]
    fn webtransport_cert_verdicts_match_the_wtransport_client_verifier() {
        let minted = PinnedCert::mint(["localhost"], MAX_VALIDITY_DAYS).expect("mint");
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let cases: [(&[u8], i64); 8] = [
            (P256_14D, FIXTURE_MID),
            (P256_14D, FIXTURE_START - 1),
            (P256_14D, FIXTURE_END_14D + 1),
            (P256_15D, FIXTURE_MID),
            (P384_14D, FIXTURE_MID),
            (RSA_14D, FIXTURE_MID),
            (P256_2050, 2_525_000_000),
            (minted.leaf_der(), now),
        ];
        for (der, at) in cases {
            let pin = Sha256Digest::new(leaf_hash(der).expect("every fixture parses"));
            let verifier = ServerHashVerification::new([pin]);
            let at_time = UnixTime::since_unix_epoch(std::time::Duration::from_secs(
                u64::try_from(at).expect("positive time"),
            ));
            let client_accepts = verifier
                .verify_server_cert(
                    &CertificateDer::from(der.to_vec()),
                    &[],
                    &ServerName::try_from("localhost").expect("valid name"),
                    &[],
                    at_time,
                )
                .is_ok();
            assert_eq!(
                check_pinnable(der, at).is_ok(),
                client_accepts,
                "policy and client disagree on a {}-byte certificate at {at}",
                der.len()
            );
        }
    }
}
