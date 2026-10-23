//! MCP5 stdio startup: arg parse, project-folder open, server build.
//!
//! Consumer: the `e2040-mcp` binary serves [`McpServer`](super::mcp::McpServer)
//! over stdio via rmcp transport-io, so any MCP client connects with one
//! config line. Data owner stays the editor command service; this module owns
//! only startup wiring. Units follow `engine-scene` (meters). Determinism
//! scope: none here; the command service owns revisions. Errors are typed
//! [`StartupError`] with nonzero exit codes; user input never panics.

use std::fmt;
use std::path::{Path, PathBuf};

use engine_editor::capability::{self, Approval, Capability};
use engine_editor::commands::{Applied, Batch, Document, Journal};
use engine_editor::durable::{rehydrate, reopen_sealed, PersistError};

use super::mcp::{cap_error, undo_gate_error, McpServer, McpToolError, ServerState};
use super::packs::web::WebScope;

/// Capability session the stdio door serves. One client, one session.
pub const MCP_SESSION: u64 = 1;

/// Help text for `--help`: usage plus the one-line MCP client config.
pub const USAGE: &str = "\
usage: e2040-mcp [--project-dir DIR] [--web-root DIR [--web-chrome PATH]] [DIR]\n\
\n\
Serve the MCP authoring door on a project folder over stdio.\n\
The folder holds the durable `gen-*.edoc` generations; a fresh folder\n\
starts empty. With no DIR, the current directory is the project.\n\
One door per folder: while a door runs it holds the folder's lease file\n\
`.e2040-door.lease`, so a second door on the same folder is refused.\n\
\n\
--web-root DIR allows the web verbs (`build.web`, `serve.web`,\n\
`browser.capture`, `browser.input`) over the repo root holding\n\
`tools/web-build.mjs` and `tools/web-browse.mjs`. Without it every web verb\n\
is refused with the typed code `unscoped`. Sets are built under\n\
`DIR/target/door-web`. --web-chrome names the Chrome `browser.capture`\n\
drives (default: the host picks one).\n\
\n\
One-line MCP client config:\n\
{\"command\": \"e2040-mcp\", \"args\": [\"--project-dir\", \"/path/to/project\", \"--web-root\", \"/path/to/engine\"]}\n\
";

/// Help text printed for `--help`.
pub fn usage() -> &'static str {
    USAGE
}

/// What stopped `e2040-mcp` before serving: CLI misuse, a bad project
/// directory, or a failed stdio transport. Each prints to stderr and exits
/// nonzero; there is no arbitrary execute/shell/file-write tool past this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupError {
    /// CLI misuse (unknown flag, missing value, stray positional).
    Usage { message: String },
    /// The project directory is missing, not a directory or unreadable.
    BadDir { path: String, reason: String },
    /// Another door holds the folder's single-writer lease.
    Leased { path: String },
    /// The stdio transport failed to start or serve.
    Transport { detail: String },
}

impl StartupError {
    /// A stdio transport failure: `what` names the step, `error` is the cause.
    pub fn transport(what: &str, error: &dyn fmt::Debug) -> Self {
        Self::Transport {
            detail: format!("{what}: {error:?}"),
        }
    }

    /// Process exit code: usage is 2 (CLI convention), bad dir, lease and
    /// transport failures are 1. All nonzero by contract.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Usage { .. } => 2,
            Self::BadDir { .. } | Self::Leased { .. } | Self::Transport { .. } => 1,
        }
    }
}

impl fmt::Display for StartupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage { message } => write!(f, "usage error: {message}\n{USAGE}"),
            Self::BadDir { path, reason } => write!(f, "bad project dir `{path}`: {reason}"),
            Self::Leased { path } => write!(
                f,
                "project `{path}` is already open in another door; close it first \
                 (a door holds the folder's lease until its process exits)"
            ),
            Self::Transport { detail } => write!(f, "transport failed: {detail}"),
        }
    }
}

impl std::error::Error for StartupError {}

/// What the project allows for the web verbs: the repo root holding the node
/// tools, plus the Chrome `browser.capture` drives. The door grants this; no
/// verb can.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebGrant {
    /// Repo root holding `tools/web-build.mjs` and `tools/web-browse.mjs`.
    pub root: PathBuf,
    /// Chrome program or path `browser.capture` drives; `None` lets the tool find one.
    pub chrome: Option<String>,
}

/// Parsed stdio startup: the project folder to open, plus the optional web
/// grant. The server speaks on stdin/stdout; there is no port and no extra tool
/// past the 26-tool registry (`crate::tools::TOOLS`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupConfig {
    /// Project folder holding the durable `gen-*.edoc` generations.
    pub project_dir: PathBuf,
    /// `--web-root`: allow the web verbs under that repo root; `None` (the
    /// default) refuses every web verb with `unscoped`.
    pub web: Option<WebGrant>,
}

/// Parse `argv` (including `argv[0]`): `--project-dir DIR`,
/// `--project-dir=DIR`, `--web-root DIR`, `--web-chrome PATH` or one positional
/// DIR. Anything else is [`Usage`](StartupError::Usage).
pub fn parse_args(args: &[String]) -> Result<StartupConfig, StartupError> {
    let usage = |message: &str| StartupError::Usage {
        message: message.to_owned(),
    };
    let mut flag_dir: Option<PathBuf> = None;
    let mut positional: Option<PathBuf> = None;
    let mut web_root: Option<PathBuf> = None;
    let mut web_chrome: Option<String> = None;
    let mut rest = args.iter().skip(1);
    while let Some(arg) = rest.next() {
        if arg == "--project-dir" {
            let value = rest
                .next()
                .ok_or_else(|| usage("--project-dir needs a directory"))?;
            if value.is_empty() {
                return Err(usage("--project-dir needs a non-empty directory"));
            }
            if flag_dir.is_some() {
                return Err(usage("--project-dir given twice"));
            }
            flag_dir = Some(PathBuf::from(value));
        } else if let Some(value) = arg.strip_prefix("--project-dir=") {
            if value.is_empty() {
                return Err(usage("--project-dir needs a non-empty directory"));
            }
            if flag_dir.is_some() {
                return Err(usage("--project-dir given twice"));
            }
            flag_dir = Some(PathBuf::from(value));
        } else if arg == "--web-root" || arg.starts_with("--web-root=") {
            let value = if arg == "--web-root" {
                rest.next()
                    .ok_or_else(|| usage("--web-root needs a repo directory"))?
                    .as_str()
            } else {
                arg.trim_start_matches("--web-root=")
            };
            if value.is_empty() {
                return Err(usage("--web-root needs a non-empty repo directory"));
            }
            if web_root.is_some() {
                return Err(usage("--web-root given twice"));
            }
            web_root = Some(PathBuf::from(value));
        } else if arg == "--web-chrome" || arg.starts_with("--web-chrome=") {
            let value = if arg == "--web-chrome" {
                rest.next()
                    .ok_or_else(|| usage("--web-chrome needs a program or path"))?
                    .as_str()
            } else {
                arg.trim_start_matches("--web-chrome=")
            };
            if value.is_empty() {
                return Err(usage("--web-chrome needs a non-empty program or path"));
            }
            if web_chrome.is_some() {
                return Err(usage("--web-chrome given twice"));
            }
            web_chrome = Some(value.to_owned());
        } else if arg == "--help" || arg == "-h" {
            return Err(usage("help requested"));
        } else if arg.starts_with('-') {
            return Err(usage(&format!("unknown flag `{arg}`")));
        } else if positional.is_some() {
            return Err(usage("at most one project directory"));
        } else {
            positional = Some(PathBuf::from(arg));
        }
    }
    if flag_dir.is_some() && positional.is_some() {
        return Err(usage("pass the project directory once, not twice"));
    }
    if web_chrome.is_some() && web_root.is_none() {
        return Err(usage(
            "--web-chrome needs --web-root: the web verbs stay refused until the project allows them",
        ));
    }
    Ok(StartupConfig {
        project_dir: flag_dir
            .or(positional)
            .unwrap_or_else(|| PathBuf::from(".")),
        web: web_root.map(|root| WebGrant {
            root,
            chrome: web_chrome,
        }),
    })
}

/// Open the project folder: the latest sealed `gen-*.edoc` generation wins,
/// corrupt generations are skipped by reopen, and a fresh folder starts
/// empty. The sealed document plus its idempotency map and undo stacks
/// rehydrate into the command service, so a replayed key answers its recorded
/// result without a new revision and undo keeps working after restart.
pub fn open_project(dir: &Path) -> Result<(Document, Journal), StartupError> {
    let bad = |reason: String| StartupError::BadDir {
        path: dir.display().to_string(),
        reason,
    };
    if dir.as_os_str().is_empty() {
        return Err(bad("empty path; pass a project directory".to_owned()));
    }
    match std::fs::metadata(dir) {
        Ok(meta) => {
            if !meta.is_dir() {
                return Err(bad("not a directory".to_owned()));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(bad("directory does not exist".to_owned()));
        }
        Err(error) => return Err(bad(format!("cannot stat directory: {error}"))),
    }
    match reopen_sealed(dir) {
        Ok(Some(opened)) => {
            let mut doc = Document::new();
            let mut journal = Journal::new();
            rehydrate(&mut doc, &mut journal, opened);
            Ok((doc, journal))
        }
        Ok(None) => Ok((Document::new(), Journal::new())),
        Err(error) => Err(bad(format!("cannot list directory: {error}"))),
    }
}

/// Name of the single-writer lease file a door holds on its project folder.
pub const LEASE_FILE: &str = ".e2040-door.lease";

/// One door's exclusive lease on a project folder.
///
/// The file stays on disk so every door locks the same path; the advisory
/// lock lives with this handle and the OS drops it on clean exit or process
/// death, so a crashed door never leaves the folder locked. The file itself
/// never refuses a door: only a live lock does.
#[derive(Debug)]
pub struct ProjectLease {
    _file: std::fs::File,
}

impl ProjectLease {
    /// Take the folder's lease, or refuse while another door holds it.
    pub fn acquire(dir: &Path) -> Result<Self, StartupError> {
        let path = dir.join(LEASE_FILE);
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| StartupError::Transport {
                detail: format!("cannot open the project lease {}: {error}", path.display()),
            })?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(std::fs::TryLockError::WouldBlock) => Err(StartupError::Leased {
                path: dir.display().to_string(),
            }),
            Err(std::fs::TryLockError::Error(error)) => Err(StartupError::Transport {
                detail: format!("cannot lock the project lease {}: {error}", path.display()),
            }),
        }
    }
}

/// Number the next sealed generation takes: one past the highest
/// `gen-*.edoc` in `dir`, ignoring in-flight `.tmp` leftovers. The folder owns
/// the number because `scene.save` seals there too, so it never tracks the
/// document revision.
pub(crate) fn next_generation(dir: &Path) -> std::io::Result<u64> {
    let mut newest = 0u64;
    for entry in std::fs::read_dir(dir)? {
        let name = entry?.file_name();
        let name = name.to_string_lossy();
        let Some(stem) = name
            .strip_prefix("gen-")
            .and_then(|rest| rest.strip_suffix(".edoc"))
        else {
            continue;
        };
        if let Ok(number) = stem.parse::<u64>() {
            newest = newest.max(number);
        }
    }
    Ok(newest.saturating_add(1))
}

/// A failed seal is a refusal: the door answers the error and the live
/// document keeps the state it had before the call.
fn persist_refusal(dir: &Path, error: &PersistError) -> McpToolError {
    McpToolError::new(
        "persist_failed",
        format!(
            "the write did not reach the project folder {}: {error}; \
             the document is unchanged, retry once the folder is writable",
            dir.display()
        ),
    )
}

/// Land one gated batch durably through the door's project folder: number the
/// generation, run the gate and seal on clones, and commit only a read-back
/// match. Crate-visible so `ServerState::tx_apply` branches on one call.
pub(crate) fn apply_durable(
    state: &mut ServerState,
    batch: &Batch,
    approval: &Approval,
    granted: &[&str],
    dir: &Path,
) -> Result<Applied, McpToolError> {
    let generation =
        next_generation(dir).map_err(|error| persist_refusal(dir, &PersistError::Io(error)))?;
    capability::apply_gated_persisted(
        &mut state.doc,
        &mut state.journal,
        batch,
        &mut state.cap,
        approval,
        &mut state.ledger,
        capability::SealTarget { dir, generation },
    )
    .map_err(|error| match error {
        capability::GatedPersistError::Cap(cap) => cap_error(&cap, granted, &state.doc),
        capability::GatedPersistError::Persist(persist) => persist_refusal(dir, &persist),
    })
}

/// Undo the newest batch durably: same clone, seal and read-back rule as
/// [`apply_durable`], so a revert the store did not land never reports.
pub(crate) fn undo_durable(
    state: &mut ServerState,
    granted: &[&str],
    dir: &Path,
) -> Result<Applied, McpToolError> {
    let generation =
        next_generation(dir).map_err(|error| persist_refusal(dir, &PersistError::Io(error)))?;
    capability::undo_gated_persisted(
        &mut state.doc,
        &mut state.journal,
        &mut state.cap,
        capability::SealTarget { dir, generation },
    )
    .map_err(|error| match error {
        capability::GatedUndoPersistError::Gate(gate) => {
            undo_gate_error(&gate, granted, &state.doc)
        }
        capability::GatedUndoPersistError::Persist(persist) => persist_refusal(dir, &persist),
    })
}

/// Build the door server (core tools plus verb packs) over the opened
/// project folder. A [`WebGrant`] is the one host call that allows the web
/// verbs for this session; the verbs stay refused (`unscoped`) without it.
pub fn build_server(config: &StartupConfig) -> Result<McpServer, StartupError> {
    // Validate the one host grant that opens the web verbs before taking the
    // folder lease, so a bad grant never leaves a lock behind.
    if let Some(grant) = &config.web {
        if !grant.root.is_dir() {
            return Err(StartupError::BadDir {
                path: grant.root.display().to_string(),
                reason: "--web-root must be the repo root holding tools/web-build.mjs and \
                         tools/web-browse.mjs; pass an existing directory"
                    .to_owned(),
            });
        }
    }
    let (doc, journal) = open_project(&config.project_dir)?;
    // AD12: one writer per folder. The lease is taken after the store opens so
    // a bad directory still fails as a bad directory.
    let lease = ProjectLease::acquire(&config.project_dir)?;
    // DOOR2: scene.save and scene.load seal into and read from this folder.
    let mut state = super::mcp::ServerState::with_document(
        doc,
        journal,
        MCP_SESSION,
        Capability::unrestricted(MCP_SESSION),
    );
    state.project_dir = Some(config.project_dir.clone());
    state.project_lease = Some(lease);
    let server = McpServer::with_state(state);
    if let Some(grant) = &config.web {
        let scope = match &grant.chrome {
            Some(chrome) => WebScope::new(grant.root.clone()).with_chrome(chrome.clone()),
            None => WebScope::new(grant.root.clone()),
        };
        server
            .allow_web(scope)
            .map_err(|error| StartupError::Transport {
                detail: format!("cannot grant the web scope: {error}"),
            })?;
    }
    Ok(server)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::{
        ApprovalDto, McpToolError, OpDto, PoseDto, TxApplyArgs, TxApplyOutput, TxApplyTool,
        TxPreviewArgs, TxPreviewOutput, TxPreviewTool, TxUndoArgs, TxUndoTool,
    };
    use engine_editor::commands::{self, Batch, CommandMeta, Op};
    use engine_editor::durable::{apply_persisted, generation_file_name, persist_generation};
    use engine_scene::ids::ObjectId;
    use engine_scene::transform::Transform;
    use rmcp::handler::server::router::tool::SyncTool;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(ToString::to_string).collect()
    }

    /// Fresh scratch directory per test (parallel-safe: unique names).
    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("e2040-mcp5-{n}-{name}", n = std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir creates");
        dir
    }

    #[test]
    fn parse_args_defaults_flag_forms_and_positional() {
        assert_eq!(
            parse_args(&argv(&["e2040-mcp"]))
                .expect("bare invocation parses")
                .project_dir,
            PathBuf::from(".")
        );
        assert_eq!(
            parse_args(&argv(&["e2040-mcp", "proj"]))
                .expect("positional parses")
                .project_dir,
            PathBuf::from("proj")
        );
        assert_eq!(
            parse_args(&argv(&["e2040-mcp", "--project-dir", "proj"]))
                .expect("flag form parses")
                .project_dir,
            PathBuf::from("proj")
        );
        assert_eq!(
            parse_args(&argv(&["e2040-mcp", "--project-dir=proj"]))
                .expect("equals form parses")
                .project_dir,
            PathBuf::from("proj")
        );
    }

    #[test]
    fn parse_args_rejects_misuse_as_typed_usage() {
        for words in [
            vec!["e2040-mcp", "--frobnicate"],
            vec!["e2040-mcp", "--project-dir"],
            vec!["e2040-mcp", "--project-dir="],
            vec!["e2040-mcp", "--project-dir", "a", "b"],
            vec!["e2040-mcp", "a", "b"],
            vec!["e2040-mcp", "--project-dir", "a", "--project-dir", "b"],
            vec!["e2040-mcp", "--help"],
            vec!["e2040-mcp", "--web-root"],
            vec!["e2040-mcp", "--web-root="],
            vec!["e2040-mcp", "--web-root", "a", "--web-root", "b"],
            vec!["e2040-mcp", "--web-chrome"],
            vec!["e2040-mcp", "--web-chrome="],
            vec![
                "e2040-mcp",
                "--web-root",
                "a",
                "--web-chrome",
                "c",
                "--web-chrome",
                "d",
            ],
            vec!["e2040-mcp", "--web-chrome", "chrome"],
        ] {
            let error = parse_args(&argv(&words)).expect_err("misuse must fail");
            assert!(
                matches!(error, StartupError::Usage { .. }),
                "not a usage error for {words:?}: {error:?}"
            );
            assert_ne!(error.exit_code(), 0, "usage errors exit nonzero");
        }
        // `--web-chrome` alone names the fix: the verbs stay refused until the
        // project allows them.
        let error =
            parse_args(&argv(&["e2040-mcp", "--web-chrome", "chrome"])).expect_err("no root");
        assert!(
            error.to_string().contains("--web-root"),
            "the refusal names the missing flag: {error}"
        );
    }

    #[test]
    fn parse_args_reads_the_web_grant_and_leaves_it_off_by_default() {
        let bare = parse_args(&argv(&["e2040-mcp"])).expect("bare parses");
        assert_eq!(bare.web, None, "no flag means the web verbs stay refused");
        let explicit = parse_args(&argv(&[
            "e2040-mcp",
            "--web-root",
            "engine",
            "--web-chrome",
            "chrome",
        ]))
        .expect("flag form parses");
        assert_eq!(
            explicit.web,
            Some(WebGrant {
                root: PathBuf::from("engine"),
                chrome: Some("chrome".to_owned())
            })
        );
        let equals = parse_args(&argv(&[
            "e2040-mcp",
            "--web-root=engine",
            "--web-chrome=chrome",
        ]))
        .expect("equals form parses");
        assert_eq!(equals.web, explicit.web);
        let root_only =
            parse_args(&argv(&["e2040-mcp", "--web-root", "engine"])).expect("root alone parses");
        assert_eq!(
            root_only.web,
            Some(WebGrant {
                root: PathBuf::from("engine"),
                chrome: None
            }),
            "the grant works without Chrome: build and serve do not need it"
        );
    }

    /// A scratch repo root with no `tools/` folder: a granted verb opens and
    /// stops at the missing tool instead of refusing `unscoped`.
    fn grant_rig(name: &str) -> (PathBuf, PathBuf) {
        (
            scratch(&format!("{name}-project")),
            scratch(&format!("{name}-root")),
        )
    }

    #[test]
    fn the_web_grant_opens_the_verbs_and_a_bad_root_fails_at_start() {
        use crate::packs::web::{TOOL_BUILD_WEB, UNSCOPED};
        use crate::packs::{ApiCallArgs, ApiCallTool};
        use rmcp::handler::server::router::tool::SyncTool;

        let call = |server: &McpServer| {
            ApiCallTool::invoke(
                server,
                ApiCallArgs {
                    verb: TOOL_BUILD_WEB.to_owned(),
                    args: None,
                },
            )
        };
        let (project, root) = grant_rig("web-grant");
        // Refused by default: the door grant is the only way in.
        let closed = build_server(&StartupConfig {
            project_dir: project.clone(),
            web: None,
        })
        .expect("server builds without a grant");
        assert_eq!(call(&closed).expect_err("refused").code, UNSCOPED);
        // One door per folder: the grant rig reopens the same folder.
        drop(closed);
        // Granted: the verb reaches its own gate and names the missing tool.
        let open = build_server(&StartupConfig {
            project_dir: project.clone(),
            web: Some(WebGrant {
                root: root.clone(),
                chrome: Some("chrome-fixture".to_owned()),
            }),
        })
        .expect("server builds with a grant");
        let missing = call(&open).expect_err("no tools folder");
        assert_eq!(missing.code, "tool_missing", "{missing}");
        assert!(missing.message.contains("web-build.mjs"), "{missing}");
        // A grant root that is not a directory is a typed start failure.
        let error = build_server(&StartupConfig {
            project_dir: project.clone(),
            web: Some(WebGrant {
                root: root.join("nope"),
                chrome: None,
            }),
        })
        .err()
        .expect("bad grant root");
        assert!(matches!(error, StartupError::BadDir { .. }), "{error:?}");
        assert_ne!(error.exit_code(), 0);
        drop(open);
        let _ = std::fs::remove_dir_all(&project);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn open_project_missing_file_and_empty_cases() {
        let missing =
            std::env::temp_dir().join(format!("e2040-mcp5-nope-{n}", n = std::process::id()));
        let _ = std::fs::remove_dir_all(&missing);
        let error = open_project(&missing).expect_err("missing dir must fail");
        assert!(
            matches!(error, StartupError::BadDir { .. }),
            "missing dir is a bad dir: {error:?}"
        );
        assert_ne!(error.exit_code(), 0);
        assert!(error.to_string().contains("does not exist"));

        let dir = scratch("file-is-dir");
        let file = dir.join("note.txt");
        std::fs::write(&file, b"not a directory").expect("note writes");
        let error = open_project(&file).expect_err("file must fail");
        assert!(matches!(error, StartupError::BadDir { .. }));
        assert!(error.to_string().contains("not a directory"));

        let (doc, _) = open_project(&dir).expect("empty folder starts empty");
        assert_eq!(doc, Document::new());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_project_reopens_the_sealed_generation() {
        let dir = scratch("reopen");
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let id = ObjectId::new();
        commands::apply(
            &mut doc,
            &mut journal,
            &Batch {
                ops: vec![Op::Insert {
                    id,
                    parent: ObjectId::NIL,
                    local: Transform::new([1.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
                }],
                base_revision: 0,
                idempotency_key: 77,
                provenance: None,
            },
        )
        .expect("seed applies");
        persist_generation(&dir, 1, &doc).expect("seed seals");

        let (back, _) = open_project(&dir).expect("project reopens");
        assert_eq!(back, doc, "the sealed generation is the served document");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_project_rehydrates_journal_for_replay_and_undo() {
        let dir = scratch("rehydrate");
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let meta = CommandMeta::default();
        let node = ObjectId::new();
        let batch = Batch {
            ops: vec![Op::Insert {
                id: node,
                parent: ObjectId::NIL,
                local: Transform::new([1.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
            }],
            base_revision: 0,
            idempotency_key: 91,
            provenance: None,
        };
        let receipt =
            apply_persisted(&mut doc, &mut journal, &batch, &meta, &dir, 1).expect("seed seals");
        assert_eq!(doc.revision, 1);

        // Restart: the project-open path restores the document plus the
        // idempotency map and undo stacks.
        let (mut back, mut back_journal) = open_project(&dir).expect("project reopens");
        assert_eq!(back, doc);
        assert_eq!(back_journal.past_len(), 1);

        // Same key plus same payload replays the recorded result without a
        // new revision; undo still works after rehydrate.
        let replayed =
            commands::apply(&mut back, &mut back_journal, &batch).expect("stored key replays");
        assert_eq!(replayed, receipt);
        assert_eq!(back.revision, 1);
        assert_eq!(back, doc);
        commands::undo(&mut back, &mut back_journal).expect("rehydrated undo works");
        assert_eq!(back.revision, 2);
        assert!(!back.hierarchy.contains(node));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn built_server_holds_exactly_the_contract_tools() {
        let dir = scratch("tools");
        let config = StartupConfig {
            project_dir: dir.clone(),
            web: None,
        };
        let server = build_server(&config).expect("server builds");
        let mut names: Vec<String> = server
            .tools()
            .iter()
            .map(|tool| tool.name.to_string())
            .collect();
        names.sort();
        let mut expected: Vec<String> = super::super::mcp::default_tool_names()
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        expected.sort();
        assert_eq!(
            names, expected,
            "a fresh door lists the core plus the default packs"
        );
        let mut all: Vec<String> = server
            .all_tools()
            .iter()
            .map(|tool| tool.name.to_string())
            .collect();
        all.sort();
        let mut reachable: Vec<String> = super::super::mcp::tool_names()
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        reachable.sort();
        assert_eq!(all, reachable, "no execute/shell/file-write tool exists");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn startup_errors_are_typed_and_nonzero() {
        let usage = StartupError::Usage {
            message: "x".to_owned(),
        };
        let bad = StartupError::BadDir {
            path: "p".to_owned(),
            reason: "r".to_owned(),
        };
        let leased = StartupError::Leased {
            path: "p".to_owned(),
        };
        let transport = StartupError::Transport {
            detail: "d".to_owned(),
        };
        assert_eq!(usage.exit_code(), 2);
        assert_eq!(bad.exit_code(), 1);
        assert_eq!(leased.exit_code(), 1);
        assert_eq!(transport.exit_code(), 1);
        assert!(usage.to_string().starts_with("usage error: "));
        assert!(bad.to_string().contains("bad project dir `p`"));
        assert!(leased.to_string().contains("already open in another door"));
        assert!(transport.to_string().contains("transport failed: d"));
    }

    fn pose_x(x: f32) -> PoseDto {
        PoseDto {
            pos: [x, 0.0, 0.0],
            rot: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
        }
    }

    fn move_ops(id: ObjectId, x: f32) -> Vec<OpDto> {
        vec![OpDto::SetLocal {
            id: id.to_string(),
            local: pose_x(x),
        }]
    }

    fn preview(server: &McpServer, rev: u64, key: u64, ops: &[OpDto]) -> TxPreviewOutput {
        TxPreviewTool::invoke(
            server,
            TxPreviewArgs {
                expected_revision: rev,
                idempotency_key: key,
                ops: ops.to_vec(),
            },
        )
        .expect("preview drafts")
    }

    fn apply(
        server: &McpServer,
        rev: u64,
        key: u64,
        ops: &[OpDto],
        approval: ApprovalDto,
    ) -> Result<TxApplyOutput, McpToolError> {
        TxApplyTool::invoke(
            server,
            TxApplyArgs {
                expected_revision: rev,
                idempotency_key: key,
                ops: ops.to_vec(),
                approval,
                delta: Some(false),
                origin: None,
            },
        )
    }

    /// One crate at the origin, sealed as generation 1 (revision 1).
    fn seed_crate(dir: &Path) -> ObjectId {
        let id = ObjectId::new();
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let batch = Batch {
            ops: vec![Op::Insert {
                id,
                parent: ObjectId::NIL,
                local: Transform::new([0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
            }],
            base_revision: 0,
            idempotency_key: 41,
            provenance: None,
        };
        let meta = CommandMeta::default();
        apply_persisted(&mut doc, &mut journal, &batch, &meta, dir, 1).expect("seed seals");
        id
    }

    fn crate_x(doc: &Document, id: ObjectId) -> Option<f32> {
        doc.hierarchy.local(id).map(|local| local.position[0])
    }

    fn door(dir: &Path) -> McpServer {
        build_server(&StartupConfig {
            project_dir: dir.to_path_buf(),
            web: None,
        })
        .expect("door builds")
    }

    #[test]
    fn durable_door_kill_after_ack_reopens_with_the_edit() {
        let dir = scratch("durable-after-ack");
        let id = seed_crate(&dir);
        let server = door(&dir);
        let ops = move_ops(id, 5.0);
        let drafted = preview(&server, 1, 71, &ops);
        let landed = apply(&server, 1, 71, &ops, drafted.approval).expect("move lands");
        assert_eq!(landed.revision, 2);
        // Kill after the receipt: the process dies without any shutdown hook.
        drop(server);
        let (back, back_journal) = open_project(&dir).expect("restart opens the folder");
        assert_eq!(back.revision, 2, "the acked revision is on disk");
        assert_eq!(crate_x(&back, id), Some(5.0), "the edit survived");
        assert_eq!(
            back_journal.past_len(),
            2,
            "the sealed journal holds the seed and the move"
        );
        assert!(dir.join(generation_file_name(2)).is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn durable_door_kill_before_ack_reopens_without_the_edit() {
        let dir = scratch("durable-before-ack");
        let id = seed_crate(&dir);
        // A door killed mid-seal leaves the temp file, and one killed after
        // the rename but before the bytes flush leaves a torn generation.
        std::fs::write(dir.join("gen-00000002.edoc.tmp"), b"torn mid-write")
            .expect("torn temp writes");
        std::fs::write(dir.join(generation_file_name(2)), b"torn after rename")
            .expect("torn generation writes");
        let server = door(&dir);
        let state = server.lock().expect("lock holds");
        assert_eq!(state.doc.revision, 1, "the unacked revision never exists");
        assert_eq!(crate_x(&state.doc, id), Some(0.0), "the edit is absent");
        drop(state);
        drop(server);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn durable_door_persist_failure_returns_an_error_and_changes_nothing() {
        let dir = scratch("durable-failure");
        let id = seed_crate(&dir);
        let server = door(&dir);
        // Block the writer's temp path with a directory: the seal fails after
        // the gate already staged the write.
        let blocker = dir.join("gen-00000002.edoc.tmp");
        std::fs::create_dir(&blocker).expect("blocker creates");
        let ops = move_ops(id, 4.0);
        let drafted = preview(&server, 1, 73, &ops);
        let error = apply(&server, 1, 73, &ops, drafted.approval).expect_err("the seal fails");
        assert_eq!(error.code, "persist_failed", "{error}");
        {
            let state = server.lock().expect("lock holds");
            assert_eq!(state.doc.revision, 1, "the live revision did not move");
            assert_eq!(
                crate_x(&state.doc, id),
                Some(0.0),
                "the live edit is absent"
            );
            assert_eq!(state.journal.past_len(), 1, "the live journal did not move");
        }
        // Kill the refused door, then reopen: nothing reached the folder.
        drop(server);
        std::fs::remove_dir(&blocker).expect("blocker clears");
        let (back, back_journal) = open_project(&dir).expect("restart opens the folder");
        assert_eq!(back.revision, 1);
        assert_eq!(crate_x(&back, id), Some(0.0));
        assert_eq!(back_journal.past_len(), 1);
        assert!(!dir.join(generation_file_name(2)).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn durable_door_replay_after_restart_returns_the_stored_receipt() {
        let dir = scratch("durable-replay");
        let id = seed_crate(&dir);
        let ops = move_ops(id, 3.0);
        let drafted;
        let first;
        {
            let server = door(&dir);
            drafted = preview(&server, 1, 72, &ops);
            first = apply(&server, 1, 72, &ops, drafted.approval.clone()).expect("move lands");
            assert_eq!(first.revision, 2);
        }
        // Restart, then resend the identical request: same key, ops and base
        // revision, with the approval minted before the first apply.
        let server = door(&dir);
        let again = apply(&server, 1, 72, &ops, drafted.approval)
            .expect("the stored key replays after a restart");
        assert_eq!(again.revision, first.revision);
        assert_eq!(again.key, first.key);
        assert_eq!(again.journal_index, first.journal_index);
        assert_eq!(again.before_digest, first.before_digest);
        assert_eq!(again.staged_digest, first.staged_digest);
        assert_eq!(again.after_digest, first.after_digest);
        assert!(
            !dir.join(generation_file_name(3)).exists(),
            "a replay seals nothing new"
        );
        drop(server);
        let (back, _) = open_project(&dir).expect("restart opens the folder");
        assert_eq!(back.revision, 2, "the replay did not move the revision");
        assert_eq!(crate_x(&back, id), Some(3.0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn durable_door_second_door_on_the_folder_is_refused() {
        let dir = scratch("durable-lease");
        let config = StartupConfig {
            project_dir: dir.clone(),
            web: None,
        };
        let first = build_server(&config).expect("first door opens");
        let error = match build_server(&config) {
            Ok(_) => panic!("a second door must refuse"),
            Err(error) => error,
        };
        assert!(matches!(error, StartupError::Leased { .. }), "{error:?}");
        assert_ne!(error.exit_code(), 0);
        assert!(error.to_string().contains("already open in another door"));
        // The lock dies with the door; the file alone never refuses.
        drop(first);
        assert!(dir.join(LEASE_FILE).is_file());
        let again = build_server(&config).expect("the lease releases with the door");
        drop(again);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn durable_door_undo_persists_across_a_restart() {
        let dir = scratch("durable-undo");
        let id = seed_crate(&dir);
        let server = door(&dir);
        let ops = move_ops(id, 7.0);
        let drafted = preview(&server, 1, 75, &ops);
        let landed = apply(&server, 1, 75, &ops, drafted.approval).expect("move lands");
        let undone = TxUndoTool::invoke(
            &server,
            TxUndoArgs {
                undo_token: Some(landed.key),
            },
        )
        .expect("undo lands");
        assert_eq!(undone.revision, 3);
        drop(server);
        let (back, back_journal) = open_project(&dir).expect("restart opens the folder");
        assert_eq!(back.revision, 3, "the undo is a revision, not a rollback");
        assert_eq!(
            crate_x(&back, id),
            Some(0.0),
            "the restart opens on the undone scene"
        );
        assert_eq!(
            back_journal.past_len(),
            1,
            "the undone move left the past stack"
        );
        assert!(dir.join(generation_file_name(3)).is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
