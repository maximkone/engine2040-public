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

use engine_editor::capability::Capability;
use engine_editor::commands::{Document, Journal};
use engine_editor::durable::{rehydrate, reopen_sealed};

use super::mcp::McpServer;
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

    /// Process exit code: usage is 2 (CLI convention), bad dir and transport
    /// failures are 1. All nonzero by contract.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Usage { .. } => 2,
            Self::BadDir { .. } | Self::Transport { .. } => 1,
        }
    }
}

impl fmt::Display for StartupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage { message } => write!(f, "usage error: {message}\n{USAGE}"),
            Self::BadDir { path, reason } => write!(f, "bad project dir `{path}`: {reason}"),
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

/// Build the door server (core tools plus verb packs) over the opened
/// project folder. A [`WebGrant`] is the one host call that allows the web
/// verbs for this session; the verbs stay refused (`unscoped`) without it.
pub fn build_server(config: &StartupConfig) -> Result<McpServer, StartupError> {
    let (doc, journal) = open_project(&config.project_dir)?;
    // DOOR2: scene.save and scene.load seal into and read from this folder.
    let mut state = super::mcp::ServerState::with_document(
        doc,
        journal,
        MCP_SESSION,
        Capability::unrestricted(MCP_SESSION),
    );
    state.project_dir = Some(config.project_dir.clone());
    let server = McpServer::with_state(state);
    if let Some(grant) = &config.web {
        if !grant.root.is_dir() {
            return Err(StartupError::BadDir {
                path: grant.root.display().to_string(),
                reason: "--web-root must be the repo root holding tools/web-build.mjs and \
                         tools/web-browse.mjs; pass an existing directory"
                    .to_owned(),
            });
        }
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
    use engine_editor::commands::{self, Batch, CommandMeta, Op};
    use engine_editor::durable::{apply_persisted, persist_generation};
    use engine_scene::ids::ObjectId;
    use engine_scene::transform::Transform;

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
        let transport = StartupError::Transport {
            detail: "d".to_owned(),
        };
        assert_eq!(usage.exit_code(), 2);
        assert_eq!(bad.exit_code(), 1);
        assert_eq!(transport.exit_code(), 1);
        assert!(usage.to_string().starts_with("usage error: "));
        assert!(bad.to_string().contains("bad project dir `p`"));
        assert!(transport.to_string().contains("transport failed: d"));
    }
}
