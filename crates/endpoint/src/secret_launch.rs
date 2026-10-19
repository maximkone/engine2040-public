//! MCP14 one-child vault injection for launched servers (kprun pattern, no dep).
//!
//! Consumer: the operator brief-to-game rig via [`mcp_server_spec`]; the
//! canary tests below launch real children. A vault file plus a provisioned
//! non-empty keyfile unlock headless with no prompts. The OS child env owns
//! the values; audit holds key names only. Errors never hold secret values.
//! Budget: vault and keyfile reads cap at 64 KiB each; at most 64 secrets.
//!
//! Scope note: only the inject-into-one-child plus never-log-values
//! discipline transfers. The file vault here is a JSON store gated on the
//! keyfile; real KeePass crypto stays out (packet forbids new dependencies,
//! and crypto is never hand-rolled). At-rest encryption is an orchestrator
//! decision; see the task report.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};

/// Key and value pairs in key order: the vault body, then the child env.
type Entries = BTreeMap<String, String>;

/// Largest vault file accepted: keeps unlock bounded and headless fast.
pub const MAX_VAULT_BYTES: usize = 64 * 1024;
/// Largest keyfile accepted: keyfiles are small provisioned blobs.
pub const MAX_KEYFILE_BYTES: usize = 64 * 1024;
/// Most secrets per launch: one child needs few, never a whole wallet.
pub const MAX_SECRETS: usize = 64;
/// Longest secret key name: env keys stay short and stable.
pub const MAX_KEY_LEN: usize = 64;
/// Longest secret value: tokens fit; files never ride as secrets.
pub const MAX_VALUE_LEN: usize = 4096;
/// Longest launcher program string: a binary name or path, never a script.
pub const MAX_PROGRAM_LEN: usize = 1024;
/// Most child args: the launcher passes flags, never payloads.
pub const MAX_ARGS: usize = 64;
/// Longest single child arg.
pub const MAX_ARG_LEN: usize = 4096;
/// Most parent vars an allowlist may inherit.
pub const MAX_INHERIT: usize = 128;
/// Cap on total secret bytes per launch.
pub const MAX_TOTAL_VALUE_BYTES: usize = 256 * 1024;

/// What stopped a vault load or a one-child launch. Every variant holds key
/// names, paths or reasons only: secret values never enter errors, audit or
/// diagnostics, so logs stay safe to ship.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchError {
    /// Filesystem failure with the operation named, never secret bytes.
    Io {
        /// What failed: operation plus path, never secret bytes.
        what: String,
    },
    /// The vault file is missing, oversized or not the entries object.
    BadVault {
        /// Reason the vault refused: shape or size, never file bytes.
        reason: String,
    },
    /// The keyfile is missing, empty or oversized: headless unlock refuses
    /// instead of prompting, so CI never blocks on stdin.
    BadKeyfile {
        /// Reason the keyfile refused: access or size, never key bytes.
        reason: String,
    },
    /// A secret key name breaks the env rule; names the key, never a value.
    BadKey {
        /// Key name that broke the env rule, never its value.
        key: String,
        /// Reason the key refused: shape rule, never a value.
        reason: String,
    },
    /// A secret value breaks the length/NUL rule; names the key only.
    BadValue {
        /// Key name whose value refused, never the value itself.
        key: String,
        /// Reason the value refused: empty, size or NUL, never bytes.
        reason: String,
    },
    /// The launcher program or an arg is empty, oversized or holds NUL.
    BadProgram {
        /// Reason the program or arg refused: empty, size or NUL, never secrets.
        reason: String,
    },
    /// The OS refused the single spawn; names the program basename only.
    Spawn {
        /// Reason the spawn refused: basename plus OS error, never secrets.
        reason: String,
    },
}

impl fmt::Display for LaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { what } => write!(f, "secret launch io failed: {what}"),
            Self::BadVault { reason } => write!(f, "bad vault: {reason}"),
            Self::BadKeyfile { reason } => write!(f, "bad keyfile: {reason}"),
            Self::BadKey { key, reason } => write!(f, "bad secret key `{key}`: {reason}"),
            Self::BadValue { key, reason } => {
                write!(f, "bad secret value for key `{key}`: {reason}")
            }
            Self::BadProgram { reason } => write!(f, "bad launch program: {reason}"),
            Self::Spawn { reason } => write!(f, "launch spawn failed: {reason}"),
        }
    }
}

impl std::error::Error for LaunchError {}

/// Env-safe secret key: starts `A-Z`, then `A-Z 0-9 _`, at most 64 bytes.
pub fn is_valid_key(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_KEY_LEN {
        return false;
    }
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_uppercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Secrets loaded from the vault file. The map drops zeroed: values never
/// linger in parent memory past use, while the running child owns its copy.
/// `Debug` never prints values: key names plus a stable `[REDACTED]`
/// placeholder, so crash dumps and logs stay safe to ship.
#[derive(Default)]
pub struct VaultSecrets {
    entries: Entries,
}

impl fmt::Debug for VaultSecrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VaultSecrets")
            .field("keys", &self.keys())
            .field("values", &"[REDACTED]")
            .finish()
    }
}

impl VaultSecrets {
    /// Sorted key names for audit lines: names only, never values.
    #[must_use]
    pub fn keys(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }

    /// Count of loaded secrets.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when the vault held zero entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Zero every value byte, then forget all entries.
    pub fn clear(&mut self) {
        for value in self.entries.values_mut() {
            zero_string(value);
        }
        self.entries.clear();
    }
}

impl Drop for VaultSecrets {
    fn drop(&mut self) {
        self.clear();
    }
}

/// Overwrite a string's backing bytes with zeros, keeping its buffer and
/// length: the bytes move out, are zeroed, and move back in place, since
/// zero bytes (U+0000) are valid UTF-8.
fn zero_string(text: &mut String) {
    let mut bytes = std::mem::take(text).into_bytes();
    bytes.fill(0);
    *text = String::from_utf8(bytes).unwrap_or_default();
}

/// Load the vault file when the keyfile gate passes. Headless by contract:
/// a missing or empty keyfile is a typed error, never an interactive prompt.
pub fn load_vault(vault_path: &Path, keyfile_path: &Path) -> Result<VaultSecrets, LaunchError> {
    let keyfile = std::fs::read(keyfile_path).map_err(|error| LaunchError::BadKeyfile {
        reason: format!("cannot read keyfile `{}`: {error}", keyfile_path.display()),
    })?;
    if keyfile.is_empty() {
        return Err(LaunchError::BadKeyfile {
            reason: format!("keyfile `{}` is empty", keyfile_path.display()),
        });
    }
    if keyfile.len() > MAX_KEYFILE_BYTES {
        return Err(LaunchError::BadKeyfile {
            reason: format!(
                "keyfile `{}` exceeds {MAX_KEYFILE_BYTES} bytes",
                keyfile_path.display()
            ),
        });
    }
    let vault = std::fs::read(vault_path).map_err(|error| LaunchError::Io {
        what: format!("cannot read vault `{}`: {error}", vault_path.display()),
    })?;
    if vault.len() > MAX_VAULT_BYTES {
        return Err(LaunchError::BadVault {
            reason: format!(
                "vault `{}` exceeds {MAX_VAULT_BYTES} bytes",
                vault_path.display()
            ),
        });
    }
    let entries = parse_vault_bytes(&vault)?;
    if entries.len() > MAX_SECRETS {
        return Err(LaunchError::BadVault {
            reason: format!(
                "vault holds {} entries, cap is {MAX_SECRETS}",
                entries.len()
            ),
        });
    }
    let mut total = 0usize;
    for (key, value) in &entries {
        if !is_valid_key(key) {
            return Err(LaunchError::BadKey {
                key: key.clone(),
                reason: "key must match [A-Z][A-Z0-9_]{0,63}".to_owned(),
            });
        }
        validate_value(key, value)?;
        total += value.len();
    }
    if total > MAX_TOTAL_VALUE_BYTES {
        return Err(LaunchError::BadVault {
            reason: format!("vault values total {total} bytes, cap is {MAX_TOTAL_VALUE_BYTES}"),
        });
    }
    Ok(VaultSecrets { entries })
}

/// Value rule: non-empty, bounded, no NUL (the OS spawn would refuse it).
fn validate_value(key: &str, value: &str) -> Result<(), LaunchError> {
    if value.is_empty() {
        return Err(LaunchError::BadValue {
            key: key.to_owned(),
            reason: "value is empty".to_owned(),
        });
    }
    if value.len() > MAX_VALUE_LEN {
        return Err(LaunchError::BadValue {
            key: key.to_owned(),
            reason: format!("value exceeds {MAX_VALUE_LEN} bytes"),
        });
    }
    if value.bytes().any(|byte| byte == 0) {
        return Err(LaunchError::BadValue {
            key: key.to_owned(),
            reason: "value holds NUL".to_owned(),
        });
    }
    Ok(())
}

/// Parse the vault body: exactly `{"entries":{"KEY":"value", ...}}` with
/// string-only pairs. Reasons never echo file bytes, which may be secrets.
fn parse_vault_bytes(bytes: &[u8]) -> Result<Entries, LaunchError> {
    let text = std::str::from_utf8(bytes).map_err(|_| LaunchError::BadVault {
        reason: "vault is not valid UTF-8".to_owned(),
    })?;
    let mut parser = VaultParser {
        bytes: text.as_bytes(),
        pos: 0,
    };
    parser.eat_byte(b'{')?;
    parser.skip_ws();
    let top = parser.parse_string()?;
    if top != "entries" {
        return Err(LaunchError::BadVault {
            reason: "vault must be {\"entries\":{...}}".to_owned(),
        });
    }
    parser.skip_ws();
    parser.eat_byte(b':')?;
    parser.skip_ws();
    let entries = parser.parse_entries_object()?;
    parser.skip_ws();
    parser.eat_byte(b'}')?;
    parser.skip_ws();
    if parser.pos != parser.bytes.len() {
        return Err(LaunchError::BadVault {
            reason: "vault has trailing bytes".to_owned(),
        });
    }
    Ok(entries)
}

/// Byte cursor over the vault body; errors are fixed strings, never slices.
struct VaultParser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl VaultParser<'_> {
    fn skip_ws(&mut self) {
        while let Some(byte) = self.bytes.get(self.pos) {
            if matches!(byte, b' ' | b'\t' | b'\n' | b'\r') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn eat_byte(&mut self, want: u8) -> Result<(), LaunchError> {
        self.skip_ws();
        if self.bytes.get(self.pos) == Some(&want) {
            self.pos += 1;
            Ok(())
        } else {
            Err(LaunchError::BadVault {
                reason: "vault must be {\"entries\":{...}}".to_owned(),
            })
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn parse_entries_object(&mut self) -> Result<Entries, LaunchError> {
        self.eat_byte(b'{')?;
        let mut entries = BTreeMap::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(entries);
        }
        loop {
            self.skip_ws();
            let key = self.parse_string()?;
            self.skip_ws();
            self.eat_byte(b':')?;
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err(LaunchError::BadVault {
                    reason: "vault entry values must be strings".to_owned(),
                });
            }
            let value = self.parse_string()?;
            if entries.insert(key, value).is_some() {
                return Err(LaunchError::BadVault {
                    reason: "vault holds a duplicate key".to_owned(),
                });
            }
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(entries);
                }
                _ => {
                    return Err(LaunchError::BadVault {
                        reason: "vault entries must be comma-separated".to_owned(),
                    });
                }
            }
        }
    }

    fn parse_string(&mut self) -> Result<String, LaunchError> {
        self.skip_ws();
        if self.peek() != Some(b'"') {
            return Err(LaunchError::BadVault {
                reason: "vault must be {\"entries\":{...}}".to_owned(),
            });
        }
        self.pos += 1;
        let mut out = String::new();
        loop {
            let byte = self.peek().ok_or_else(|| LaunchError::BadVault {
                reason: "vault string is truncated".to_owned(),
            })?;
            match byte {
                b'"' => {
                    self.pos += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.pos += 1;
                    let escaped = self.peek().ok_or_else(|| LaunchError::BadVault {
                        reason: "vault escape is truncated".to_owned(),
                    })?;
                    self.pos += 1;
                    match escaped {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000C}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let point = self.escape_point()?;
                            out.push(decode_code_point(point, self)?);
                        }
                        _ => {
                            return Err(LaunchError::BadVault {
                                reason: "vault holds a bad escape".to_owned(),
                            });
                        }
                    }
                }
                0x00..=0x1F => {
                    return Err(LaunchError::BadVault {
                        reason: "vault holds a control byte".to_owned(),
                    });
                }
                _ => {
                    let rest = &self.bytes[self.pos..];
                    let text = std::str::from_utf8(rest).map_err(|_| LaunchError::BadVault {
                        reason: "vault is not valid UTF-8".to_owned(),
                    })?;
                    let next = text.chars().next().ok_or_else(|| LaunchError::BadVault {
                        reason: "vault string is truncated".to_owned(),
                    })?;
                    out.push(next);
                    self.pos += next.len_utf8();
                }
            }
        }
    }

    fn escape_point(&mut self) -> Result<u32, LaunchError> {
        let digits =
            self.bytes
                .get(self.pos..self.pos + 4)
                .ok_or_else(|| LaunchError::BadVault {
                    reason: "vault escape is truncated".to_owned(),
                })?;
        let point = engine_core::parse_hex4(digits).ok_or_else(|| LaunchError::BadVault {
            reason: "vault holds a bad unicode escape".to_owned(),
        })?;
        self.pos += 4;
        Ok(point)
    }
}

/// Decode one `\uXXXX` point, folding a high plus low surrogate pair.
fn decode_code_point(point: u32, parser: &mut VaultParser<'_>) -> Result<char, LaunchError> {
    let bad = || LaunchError::BadVault {
        reason: "vault holds a bad unicode escape".to_owned(),
    };
    if (0xD800..0xDC00).contains(&point) {
        if parser.peek() == Some(b'\\') {
            parser.pos += 1;
            if parser.peek() == Some(b'u') {
                parser.pos += 1;
                let low = parser.escape_point()?;
                if (0xDC00..0xE000).contains(&low) {
                    let full = 0x10000 + ((point - 0xD800) << 10) + (low - 0xDC00);
                    return char::from_u32(full).ok_or_else(bad);
                }
            } else {
                parser.pos -= 1;
            }
        }
        return Err(bad());
    }
    char::from_u32(point).ok_or_else(bad)
}

/// One launch: program plus args, secrets for the single child, and the
/// allowlist of parent vars the child may also see. Everything else is
/// cleared, so a full-env inherit can never smuggle secrets sideways.
/// `Debug` never prints secret values: secret key names plus a stable
/// `[REDACTED]` placeholder, so crash dumps and logs stay safe to ship.
#[derive(Default)]
pub struct LaunchSpec {
    program: String,
    args: Vec<String>,
    secrets: Entries,
    inherit_allow: Vec<String>,
}

impl fmt::Debug for LaunchSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LaunchSpec")
            .field("program", &self.program)
            .field("args", &self.args)
            .field("secret_keys", &self.secret_keys())
            .field("secrets", &"[REDACTED]")
            .field("inherit_allow", &self.inherit_allow)
            .finish()
    }
}

impl LaunchSpec {
    /// New spec for a program name or path; empty and NUL names refuse.
    pub fn new(program: &str) -> Result<Self, LaunchError> {
        if program.is_empty() {
            return Err(LaunchError::BadProgram {
                reason: "program is empty".to_owned(),
            });
        }
        if program.len() > MAX_PROGRAM_LEN {
            return Err(LaunchError::BadProgram {
                reason: format!("program exceeds {MAX_PROGRAM_LEN} bytes"),
            });
        }
        if program.bytes().any(|byte| byte == 0) {
            return Err(LaunchError::BadProgram {
                reason: "program holds NUL".to_owned(),
            });
        }
        Ok(Self {
            program: program.to_owned(),
            args: Vec::new(),
            secrets: BTreeMap::new(),
            inherit_allow: Vec::new(),
        })
    }

    /// By-value [`Self::arg`]: chains straight off a temporary, so a whole
    /// launch reads as one expression.
    ///
    /// ```rust
    /// use engine_endpoint::secret_launch::LaunchSpec;
    ///
    /// # fn main() -> Result<(), engine_endpoint::secret_launch::LaunchError> {
    /// let spec = LaunchSpec::new("e2040-mcp")?
    ///     .with_arg("--project-dir")?
    ///     .with_arg("/work")?
    ///     .with_secret("API_TOKEN", "token-abc-123")?
    ///     .with_inherit("PATH")?;
    /// assert_eq!(spec.secret_keys(), ["API_TOKEN"]);
    /// assert!(!format!("{spec:?}").contains("token-abc-123"));
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_arg(mut self, arg: &str) -> Result<Self, LaunchError> {
        self.arg(arg)?;
        Ok(self)
    }

    /// By-value [`Self::secret`].
    pub fn with_secret(mut self, key: &str, value: &str) -> Result<Self, LaunchError> {
        self.secret(key, value)?;
        Ok(self)
    }

    /// By-value [`Self::inherit`].
    pub fn with_inherit(mut self, name: &str) -> Result<Self, LaunchError> {
        self.inherit(name)?;
        Ok(self)
    }

    /// Push one child arg; NUL and oversized args refuse.
    pub fn arg(&mut self, arg: &str) -> Result<&mut Self, LaunchError> {
        if self.args.len() >= MAX_ARGS {
            return Err(LaunchError::BadProgram {
                reason: format!("args exceed {MAX_ARGS} entries"),
            });
        }
        if arg.len() > MAX_ARG_LEN {
            return Err(LaunchError::BadProgram {
                reason: format!("arg exceeds {MAX_ARG_LEN} bytes"),
            });
        }
        if arg.bytes().any(|byte| byte == 0) {
            return Err(LaunchError::BadProgram {
                reason: "arg holds NUL".to_owned(),
            });
        }
        self.args.push(arg.to_owned());
        Ok(self)
    }

    /// Add one secret for the single child; validated, never logged.
    pub fn secret(&mut self, key: &str, value: &str) -> Result<&mut Self, LaunchError> {
        if !is_valid_key(key) {
            return Err(LaunchError::BadKey {
                key: key.to_owned(),
                reason: "key must match [A-Z][A-Z0-9_]{0,63}".to_owned(),
            });
        }
        validate_value(key, value)?;
        if !self.secrets.contains_key(key) && self.secrets.len() >= MAX_SECRETS {
            return Err(LaunchError::BadValue {
                key: key.to_owned(),
                reason: format!("secrets exceed {MAX_SECRETS} entries"),
            });
        }
        self.secrets.insert(key.to_owned(), value.to_owned());
        Ok(self)
    }

    /// Allowlist one parent var into the child; everything else is cleared.
    pub fn inherit(&mut self, name: &str) -> Result<&mut Self, LaunchError> {
        if !is_valid_key(name) {
            return Err(LaunchError::BadKey {
                key: name.to_owned(),
                reason: "allowlist names match [A-Z][A-Z0-9_]{0,63}".to_owned(),
            });
        }
        if self.inherit_allow.len() >= MAX_INHERIT {
            return Err(LaunchError::BadProgram {
                reason: format!("allowlist exceeds {MAX_INHERIT} entries"),
            });
        }
        if !self.inherit_allow.iter().any(|known| known == name) {
            self.inherit_allow.push(name.to_owned());
        }
        Ok(self)
    }

    /// Copy every vault entry into this launch as a secret.
    pub fn import_vault(&mut self, secrets: &VaultSecrets) -> Result<&mut Self, LaunchError> {
        for (key, value) in &secrets.entries {
            self.secret(key, value)?;
        }
        Ok(self)
    }

    /// Sorted secret key names bound for the audit line.
    #[must_use]
    pub fn secret_keys(&self) -> Vec<String> {
        self.secrets.keys().cloned().collect()
    }

    /// Zero secret bytes held by this spec, then forget them.
    pub fn clear_secrets(&mut self) {
        for value in self.secrets.values_mut() {
            zero_string(value);
        }
        self.secrets.clear();
    }
}

impl Drop for LaunchSpec {
    fn drop(&mut self) {
        self.clear_secrets();
    }
}

/// Build the launch for one `e2040-mcp` server over a project folder with
/// vault secrets injected. This is the brief-to-game rig entry: the server
/// becomes the single child that sees the values. Nothing is spawned here;
/// hand the spec to [`launch_one_child`].
pub fn mcp_server_spec(
    project_dir: &Path,
    secrets: &VaultSecrets,
    inherit_allow: &[&str],
) -> Result<LaunchSpec, LaunchError> {
    let dir = project_dir
        .as_os_str()
        .to_str()
        .ok_or_else(|| LaunchError::BadProgram {
            reason: "project dir is not UTF-8".to_owned(),
        })?;
    let mut spec = LaunchSpec::new("e2040-mcp")?;
    spec.arg("--project-dir")?;
    spec.arg(dir)?;
    spec.import_vault(secrets)?;
    for name in inherit_allow {
        spec.inherit(name)?;
    }
    Ok(spec)
}

/// File name without directories: audit names the binary, never full paths
/// and never the vault path.
fn program_basename(program: &str) -> &str {
    program.rsplit(['/', '\\']).next().unwrap_or(program)
}

/// One audit line per launch: launch id, program basename and key names.
/// Values and the vault path can never reach this record by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRecord {
    /// Caller-supplied launch counter; keeps lines ordered without a clock.
    pub launch: u64,
    /// Program basename only.
    pub program: String,
    /// Sorted secret key names, never values.
    pub keys: Vec<String>,
}

/// Audit record for a spec: basename plus the sorted secret key names.
#[must_use]
pub fn audit_for(spec: &LaunchSpec, launch: u64) -> AuditRecord {
    AuditRecord {
        launch,
        program: program_basename(&spec.program).to_owned(),
        keys: spec.secret_keys(),
    }
}

/// Render one audit record as a single JSON line with key names only.
///
/// Units: one JSON line per call; key names only, never secret values or the
/// vault path. Ownership: caller owns the record and the returned line.
/// Threading: pure function, safe on any thread. Determinism: same record
/// always renders the same line (keys ride in sorted order).
///
/// # Example
///
/// ```rust
/// use engine_endpoint::secret_launch::{audit_for, audit_is_clean, audit_json, LaunchSpec};
///
/// assert!(engine_endpoint::secret_launch::is_valid_key("API_TOKEN"));
/// let mut spec = LaunchSpec::new("e2040-mcp").expect("program parses");
/// spec.secret("API_TOKEN", "token-abc-123").expect("secret validates");
/// let line = audit_json(&audit_for(&spec, 7));
/// assert!(line.contains("API_TOKEN"));
/// assert!(audit_is_clean(&line, &["token-abc-123"], "/vault/path.json"));
/// ```
#[must_use]
pub fn audit_json(record: &AuditRecord) -> String {
    let mut out = String::from("{\"launch\":");
    out.push_str(&record.launch.to_string());
    out.push_str(",\"program\":\"");
    engine_core::json::escape_into(&mut out, &record.program);
    out.push_str("\",\"keys\":[");
    for (index, key) in record.keys.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push('"');
        engine_core::json::escape_into(&mut out, key);
        out.push('"');
    }
    out.push_str("]}");
    out
}

/// True when neither any secret value nor the vault path text appears in the
/// audit blob: the key-names-only gate as a callable check.
#[must_use]
pub fn audit_is_clean(audit_text: &str, secret_values: &[&str], vault_path_text: &str) -> bool {
    if !vault_path_text.is_empty() && audit_text.contains(vault_path_text) {
        return false;
    }
    !secret_values
        .iter()
        .any(|value| !value.is_empty() && audit_text.contains(value))
}

/// What the single child returned: exit status, its piped stdio (the child
/// owns stdio; the launcher prints nothing) and the key-names-only audit.
#[derive(Debug)]
pub struct LaunchOutcome {
    /// Child exit status.
    pub status: ExitStatus,
    /// Bytes the child wrote to stdout.
    pub stdout: Vec<u8>,
    /// Bytes the child wrote to stderr.
    pub stderr: Vec<u8>,
    /// Audit record for this launch: key names only.
    pub audit: AuditRecord,
}

impl LaunchOutcome {
    /// True when the child exited successfully.
    #[must_use]
    pub fn success(&self) -> bool {
        self.status.success()
    }

    /// Raw byte search over captured child stdout.
    #[must_use]
    pub fn stdout_contains(&self, needle: &[u8]) -> bool {
        stdout_contains(&self.stdout, needle)
    }
}

fn stdout_contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack.len() >= needle.len()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

/// Spawn exactly one child with the secrets in its env and wait for it.
/// The parent env is never mutated, the launcher prints nothing, and the
/// returned audit holds key names only. On Windows `SystemRoot` rides along
/// (process creation needs it); every other parent var needs the allowlist.
pub fn launch_one_child(spec: &LaunchSpec, launch: u64) -> Result<LaunchOutcome, LaunchError> {
    if spec.program.is_empty() {
        return Err(LaunchError::BadProgram {
            reason: "program is empty".to_owned(),
        });
    }
    for key in spec.secrets.keys() {
        if !is_valid_key(key) {
            return Err(LaunchError::BadKey {
                key: key.clone(),
                reason: "key must match [A-Z][A-Z0-9_]{0,63}".to_owned(),
            });
        }
    }
    for (key, value) in &spec.secrets {
        if value.bytes().any(|byte| byte == 0) {
            return Err(LaunchError::BadValue {
                key: key.clone(),
                reason: "value holds NUL".to_owned(),
            });
        }
    }
    let audit = audit_for(spec, launch);
    let mut command = Command::new(&spec.program);
    command.args(&spec.args);
    command.env_clear();
    for name in &spec.inherit_allow {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    #[cfg(windows)]
    if let Some(system_root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", system_root);
    }
    for (key, value) in &spec.secrets {
        command.env(key, value);
    }
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    let output = command.output().map_err(|error| LaunchError::Spawn {
        reason: format!(
            "cannot spawn `{}`: {error}",
            program_basename(&spec.program)
        ),
    })?;
    Ok(LaunchOutcome {
        status: output.status,
        stdout: output.stdout,
        stderr: output.stderr,
        audit,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SCRATCH_SEQ: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn zero_string_zeroes_every_byte_at_the_same_length() {
        let mut secret = String::from("sk-live-0123456789");
        let len = secret.len();
        zero_string(&mut secret);
        assert_eq!(secret.len(), len, "the wipe keeps the length");
        assert!(secret.bytes().all(|b| b == 0), "every byte is zero");
        let mut empty = String::new();
        zero_string(&mut empty);
        assert!(empty.is_empty());
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let seq = SCRATCH_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "e2040-mcp14-{pid}-{seq}-{name}",
            pid = std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir creates");
        dir
    }

    fn json_string_into(out: &mut String, text: &str) {
        out.push('"');
        for next in text.chars() {
            match next {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                next => out.push(next),
            }
        }
        out.push('"');
    }

    fn write_vault(dir: &Path, entries: &[(&str, &str)]) -> std::path::PathBuf {
        let mut body = String::from("{\"entries\":{");
        for (index, (key, value)) in entries.iter().enumerate() {
            if index > 0 {
                body.push(',');
            }
            json_string_into(&mut body, key);
            body.push(':');
            json_string_into(&mut body, value);
        }
        body.push_str("}}");
        let path = dir.join("vault.json");
        std::fs::write(&path, body.as_bytes()).expect("vault writes");
        path
    }

    fn write_keyfile(dir: &Path) -> std::path::PathBuf {
        let path = dir.join("vault.key");
        std::fs::write(&path, b"headless-operator-keyfile-bytes").expect("keyfile writes");
        path
    }

    /// Deterministic canary values: xorshift64 over the launch index, hex
    /// encoded. Same input always yields the same canary; distinct launches
    /// never share one.
    fn canary(index: u64) -> (String, String) {
        let mut state = 0x9E37_79B9_7F4A_7C15u64 ^ index.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut hex = String::with_capacity(16);
        for _ in 0..16 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let nibble = (state & 0xF) as u8;
            hex.push(char::from_digit(u32::from(nibble), 16).expect("nibble is hex"));
        }
        (
            format!("E2040_MCP14_CANARY_{index:02}"),
            format!("e2040-canary-{index:02}-{hex}"),
        )
    }

    /// Portable env dumper: the child that must see the secret. Absolute
    /// paths keep `env_clear` spawns working on both OSes.
    fn env_dumper() -> (String, Vec<String>) {
        #[cfg(windows)]
        {
            let root = std::env::var_os("SystemRoot")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"));
            let cmd = root.join("System32").join("cmd.exe");
            (
                cmd.to_string_lossy().into_owned(),
                vec!["/C".to_owned(), "set".to_owned()],
            )
        }
        #[cfg(not(windows))]
        {
            if Path::new("/usr/bin/env").exists() {
                ("/usr/bin/env".to_owned(), Vec::new())
            } else {
                ("env".to_owned(), Vec::new())
            }
        }
    }

    fn ascii_lower(bytes: &[u8]) -> Vec<u8> {
        bytes
            .iter()
            .map(|byte| {
                if byte.is_ascii_uppercase() {
                    byte + 32
                } else {
                    *byte
                }
            })
            .collect()
    }

    #[test]
    fn headless_keyfile_unlock_loads_entries() {
        let dir = scratch("headless");
        let vault = write_vault(
            &dir,
            &[("API_TOKEN", "token-abc-123"), ("OTHER_KEY", "other-xyz")],
        );
        let keyfile = write_keyfile(&dir);
        let secrets = load_vault(&vault, &keyfile).expect("headless unlock loads");
        assert_eq!(secrets.len(), 2, "both vault entries load");
        assert_eq!(
            secrets.keys(),
            vec!["API_TOKEN".to_owned(), "OTHER_KEY".to_owned()],
            "keys arrive sorted"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_keyfile_fails_without_prompt() {
        let dir = scratch("no-keyfile");
        let vault = write_vault(&dir, &[("API_TOKEN", "token-abc-123")]);
        let missing = dir.join("absent.key");
        let error = load_vault(&vault, &missing).expect_err("no keyfile must fail");
        assert!(
            matches!(error, LaunchError::BadKeyfile { .. }),
            "missing keyfile is typed: {error:?}"
        );
        let empty = dir.join("empty.key");
        std::fs::write(&empty, b"").expect("empty keyfile writes");
        let error = load_vault(&vault, &empty).expect_err("empty keyfile must fail");
        assert!(matches!(error, LaunchError::BadKeyfile { .. }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn audit_json_holds_key_names_only() {
        let dir = scratch("audit-names");
        let vault_path = dir.join("vault.json");
        let vault_text = vault_path.display().to_string();
        let mut spec = LaunchSpec::new("e2040-mcp").expect("program parses");
        spec.secret("API_TOKEN", "super-secret-value-1")
            .expect("secret validates");
        spec.secret("OTHER_KEY", "super-secret-value-2")
            .expect("secret validates");
        let record = audit_for(&spec, 7);
        let line = audit_json(&record);
        assert!(line.contains("API_TOKEN"), "audit names the first key");
        assert!(line.contains("OTHER_KEY"), "audit names the second key");
        assert!(
            audit_is_clean(
                &line,
                &["super-secret-value-1", "super-secret-value-2"],
                &vault_text
            ),
            "audit holds no value and no vault path"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn twenty_canary_launches_zero_bytes_outside_child() {
        const LAUNCHES: u64 = 20;
        let dir = scratch("canary20");
        let (program, args) = env_dumper();
        let mut audit_blob = String::new();
        let mut values: Vec<String> = Vec::new();
        for index in 0..LAUNCHES {
            let (key, value) = canary(index);
            let mut spec = LaunchSpec::new(&program).expect("dumper program parses");
            for arg in &args {
                spec.arg(arg).expect("dumper arg parses");
            }
            spec.secret(&key, &value).expect("canary secret validates");
            let outcome = launch_one_child(&spec, index).expect("child launches");
            assert!(outcome.success(), "dumper child exits zero");
            assert!(
                outcome.stdout_contains(value.as_bytes()),
                "launch {index}: the one child sees its canary"
            );
            let line = audit_json(&outcome.audit);
            assert_eq!(outcome.audit.keys, vec![key.clone()], "one launch, one key");
            assert!(
                !line.contains(&value),
                "launch {index}: audit skips the value"
            );
            audit_blob.push_str(&line);
            audit_blob.push('\n');
            assert!(
                std::env::var_os(&key).is_none(),
                "launch {index}: parent env stays clean"
            );
            values.push(value);
        }
        let value_refs: Vec<&str> = values.iter().map(String::as_str).collect();
        let vault_text = dir.join("vault.json").display().to_string();
        assert!(
            audit_is_clean(&audit_blob, &value_refs, &vault_text),
            "20 launches leave 0 canary bytes in audit"
        );
        for (index, value) in values.iter().enumerate() {
            assert!(
                !audit_blob.contains(value),
                "canary {index} never appears outside its child"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn allowlist_gates_parent_env_ablation() {
        let (program, args) = env_dumper();
        assert!(
            std::env::var_os("PATH").is_some(),
            "test env carries PATH to gate on"
        );
        let mut closed = LaunchSpec::new(&program).expect("program parses");
        for arg in &args {
            closed.arg(arg).expect("arg parses");
        }
        let shut = launch_one_child(&closed, 0).expect("closed launch runs");
        assert!(
            !ascii_lower(&shut.stdout).windows(5).any(|w| w == b"path="),
            "cleared env hides the parent PATH"
        );
        let mut open = LaunchSpec::new(&program).expect("program parses");
        for arg in &args {
            open.arg(arg).expect("arg parses");
        }
        open.inherit("PATH").expect("PATH allowlists");
        let shown = launch_one_child(&open, 1).expect("open launch runs");
        assert!(
            ascii_lower(&shown.stdout).windows(5).any(|w| w == b"path="),
            "allowlisted PATH reaches the child"
        );
    }

    #[test]
    fn bad_vault_inputs_refuse_as_typed_errors() {
        let dir = scratch("bad-vault");
        let keyfile = write_keyfile(&dir);
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("empty", b"".to_vec()),
            ("array", b"[]".to_vec()),
            ("no-entries", b"{\"other\":{}}".to_vec()),
            ("entries-array", b"{\"entries\":[]}".to_vec()),
            ("number-value", b"{\"entries\":{\"A\":1}}".to_vec()),
            (
                "dup-key",
                b"{\"entries\":{\"A\":\"1\",\"A\":\"2\"}}".to_vec(),
            ),
            ("bad-key", b"{\"entries\":{\"lower\":\"1\"}}".to_vec()),
            ("empty-value", b"{\"entries\":{\"A\":\"\"}}".to_vec()),
            ("trailing", b"{\"entries\":{}} trailing".to_vec()),
            ("truncated", b"{\"entries\":{\"A\":\"1\"".to_vec()),
        ];
        for (name, body) in &cases {
            let path = dir.join(format!("vault-{name}.json"));
            std::fs::write(&path, body).expect("case writes");
            let error = load_vault(&path, &keyfile).expect_err("bad vault must fail");
            assert!(
                matches!(
                    error,
                    LaunchError::BadVault { .. }
                        | LaunchError::BadKey { .. }
                        | LaunchError::BadValue { .. }
                ),
                "case {name} is typed: {error:?}"
            );
        }
        let nul_case = dir.join("vault-nul.json");
        std::fs::write(&nul_case, b"{\"entries\":{\"A\":\"x\\u0000y\"}}").expect("nul case writes");
        let error = load_vault(&nul_case, &keyfile).expect_err("NUL value must fail");
        assert!(matches!(error, LaunchError::BadValue { .. }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn errors_and_audit_never_carry_values() {
        let secret = "e2040-canary-zz-0123456789abcdef";
        let error = LaunchError::BadValue {
            key: "API_TOKEN".to_owned(),
            reason: "value exceeds 4096 bytes".to_owned(),
        };
        assert!(!format!("{error:?}").contains(secret));
        assert!(!error.to_string().contains(secret));
        let mut spec = LaunchSpec::new("e2040-mcp").expect("program parses");
        spec.secret("API_TOKEN", secret).expect("secret validates");
        let line = audit_json(&audit_for(&spec, 0));
        assert!(line.contains("API_TOKEN"));
        assert!(!line.contains(secret), "audit never carries values");
        assert!(
            !audit_is_clean("prefix", &[], "")
                || audit_is_clean(&line, &[secret], "/vault/secret/path.json"),
            "clean check flags values and vault paths"
        );
        assert!(
            !audit_is_clean(&format!("leak {secret}"), &[secret], ""),
            "clean check catches a leaked value"
        );
        assert!(
            !audit_is_clean("at /vault/secret/path.json", &[], "/vault/secret/path.json"),
            "clean check catches a vault path"
        );
    }

    #[test]
    fn debug_impls_redact_secret_values() {
        let dir = scratch("debug-redact");
        let vault = write_vault(&dir, &[("API_TOKEN", "e2040-canary-debug-9f3ac271b4e5")]);
        let keyfile = write_keyfile(&dir);
        let secrets = load_vault(&vault, &keyfile).expect("redact vault loads");
        let secret = "e2040-canary-debug-9f3ac271b4e5";
        let vault_compact = format!("{secrets:?}");
        let vault_pretty = format!("{secrets:#?}");
        for view in [&vault_compact, &vault_pretty] {
            assert!(!view.contains(secret), "VaultSecrets Debug skips the value");
            assert!(
                view.contains("API_TOKEN"),
                "VaultSecrets Debug names the key"
            );
            assert!(
                view.contains("[REDACTED]"),
                "VaultSecrets Debug holds the stable placeholder"
            );
        }
        assert_eq!(
            vault_compact,
            format!("{secrets:?}"),
            "VaultSecrets Debug is stable without panic"
        );
        let mut spec = LaunchSpec::new("e2040-mcp").expect("program parses");
        spec.secret("API_TOKEN", secret).expect("secret validates");
        let spec_compact = format!("{spec:?}");
        let spec_pretty = format!("{spec:#?}");
        for view in [&spec_compact, &spec_pretty] {
            assert!(!view.contains(secret), "LaunchSpec Debug skips the value");
            assert!(view.contains("API_TOKEN"), "LaunchSpec Debug names the key");
            assert!(
                view.contains("[REDACTED]"),
                "LaunchSpec Debug holds the stable placeholder"
            );
            assert!(
                view.contains("e2040-mcp"),
                "LaunchSpec Debug names the program"
            );
        }
        assert_eq!(
            spec_compact,
            format!("{spec:?}"),
            "LaunchSpec Debug is stable without panic"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn launch_rejects_bad_program_and_inputs() {
        assert!(LaunchSpec::new("").is_err(), "empty program refuses");
        assert!(LaunchSpec::new("a\0b").is_err(), "NUL program refuses");
        let (program, _) = env_dumper();
        let mut spec = LaunchSpec::new(&program).expect("program parses");
        assert!(spec.secret("lower", "v").is_err(), "bad key refuses");
        assert!(spec.secret("A", "").is_err(), "empty value refuses");
        assert!(spec.arg("a\0b").is_err(), "NUL arg refuses");
        assert!(spec.inherit("lower").is_err(), "bad allowlist name refuses");
        let missing = launch_one_child(
            &LaunchSpec::new("e2040-mcp14-absent-binary-zz").expect("program parses"),
            0,
        )
        .expect_err("absent binary must fail to spawn");
        assert!(matches!(missing, LaunchError::Spawn { .. }));
    }

    #[test]
    fn mcp_server_spec_targets_the_door_without_spawning() {
        let dir = scratch("rig");
        let vault = write_vault(&dir, &[("API_TOKEN", "rig-token-1")]);
        let keyfile = write_keyfile(&dir);
        let secrets = load_vault(&vault, &keyfile).expect("rig vault loads");
        let project = dir.join("project");
        std::fs::create_dir_all(&project).expect("project dir creates");
        let spec = mcp_server_spec(&project, &secrets, &[]).expect("rig spec builds");
        assert_eq!(spec.secret_keys(), vec!["API_TOKEN".to_owned()]);
        let record = audit_for(&spec, 0);
        assert_eq!(record.program, "e2040-mcp");
        let line = audit_json(&record);
        assert!(line.contains("API_TOKEN"));
        assert!(!line.contains("rig-token-1"), "rig audit skips values");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(feature = "mcp")]
    #[test]
    fn mcp5_and_mcp7_ticked_in_lane() {
        let dir = scratch("mcp57");
        let config = crate::startup::StartupConfig {
            project_dir: dir.clone(),
            web: None,
        };
        let server = crate::startup::build_server(&config).expect("MCP5 server builds");
        let mut names: Vec<String> = server
            .all_tools()
            .iter()
            .map(|tool| tool.name.to_string())
            .collect();
        names.sort();
        assert!(
            names.len() >= 21,
            "the door reaches every old tool plus the new core: {}",
            names.len()
        );
        assert!(
            names.contains(&"rpc.discover".to_owned()),
            "MCP7 manifest rides"
        );
        assert!(
            names.contains(&"scene.watch".to_owned()),
            "MCP7 watch rides"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
