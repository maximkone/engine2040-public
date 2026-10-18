//! `e2040-mcp`: the stdio MCP door on a project folder.
//!
//! Consumer: any MCP client (Claude Code, OpenCode, Cursor) plus the EV1 eval
//! harness, with one config line pointing at a project folder. Data owner
//! stays the editor command service; this binary owns only process startup
//! and the tokio runtime. Units follow `engine-scene`. Determinism scope:
//! none here. Errors print to stderr with a nonzero exit; stdout carries
//! only MCP JSON-RPC frames.

use engine_endpoint::startup::{build_server, parse_args, usage};

/// Serve one project folder until the client closes stdin.
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{}", usage());
        return;
    }
    std::process::exit(run(&args).await);
}

/// Parse, open and serve: every failure maps to a typed exit code.
async fn run(args: &[String]) -> i32 {
    let config = match parse_args(args) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("e2040-mcp: {error}");
            return error.exit_code();
        }
    };
    let server = match build_server(&config) {
        Ok(server) => server,
        Err(error) => {
            eprintln!("e2040-mcp: {error}");
            return error.exit_code();
        }
    };
    eprintln!(
        "e2040-mcp: serving `{dir}` over stdio",
        dir = config.project_dir.display()
    );
    match server.run_stdio().await {
        Ok(_) => 0,
        Err(error) => {
            eprintln!("e2040-mcp: {error}");
            error.exit_code()
        }
    }
}
