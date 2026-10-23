//! One command a project verb runs, and the seam tests use instead of a toolchain.
//!
//! `build` and `run` both start programs: cargo, node, and the project's own game. They go
//! through [`Step`] and [`Runner`] so a test can script the steps and check the command line,
//! and so a program that is not installed is reported with the command that installs it.

use crate::cli::Error;
use std::io;
use std::path::PathBuf;
use std::process::Command;

/// One command a verb runs.
pub struct Step {
    /// The program: a path, or a name found on `PATH`.
    pub program: PathBuf,
    /// Its arguments, in order.
    pub args: Vec<String>,
    /// The directory it runs in.
    pub dir: PathBuf,
    /// Hold stdout for the caller instead of showing it live.
    pub capture: bool,
    /// What to run when the program is not installed.
    pub fix: &'static str,
}

/// How a step ended: its exit code (none when a signal killed it) and, when captured, stdout.
pub struct Outcome {
    /// The exit code; `None` when a signal ended the process.
    pub code: Option<i32>,
    /// Captured stdout; empty when the step was not captured.
    pub output: String,
}

/// Runs one step; tests replace it with a script.
pub type Runner<'a> = dyn FnMut(&Step) -> io::Result<Outcome> + 'a;

/// Start the process; a captured step's stdout is returned, the rest is shown live.
pub fn run_process(step: &Step) -> io::Result<Outcome> {
    let mut command = Command::new(&step.program);
    command.args(&step.args).current_dir(&step.dir);
    if step.capture {
        let output = command.output()?;
        Ok(Outcome {
            code: output.status.code(),
            output: String::from_utf8_lossy(&output.stdout).into_owned(),
        })
    } else {
        Ok(Outcome {
            code: command.status()?.code(),
            output: String::new(),
        })
    }
}

/// Start `step`; a program that is not installed is reported with its install command.
pub fn start(run: &mut Runner<'_>, step: &Step) -> Result<Outcome, Error> {
    run(step).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            Error::Project(format!(
                "`{}` was not found on PATH; install it: {}",
                step.program.display(),
                step.fix
            ))
        } else {
            Error::Project(format!(
                "cannot start `{}`: {error}",
                step.program.display()
            ))
        }
    })
}

/// How a finished step reads in a message.
pub fn exit_text(code: Option<i32>) -> String {
    code.map_or_else(
        || "ended by a signal".to_owned(),
        |code| format!("exit code {code}"),
    )
}

/// Where the builds run and where their results land.
pub struct Checkout {
    /// The engine checkout: holds `tools/` and the workspace `Cargo.toml`.
    pub root: PathBuf,
    /// Cargo's target directory.
    pub target_dir: PathBuf,
    /// `std::env::consts::OS` of the machine.
    pub os: &'static str,
    /// `std::env::consts::EXE_SUFFIX` of the machine.
    pub exe_suffix: &'static str,
}

impl Checkout {
    /// The checkout this exe was built from.
    pub fn host() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let target_dir = std::env::var_os("CARGO_TARGET_DIR")
            .map_or_else(|| root.join("target"), |dir| root.join(dir));
        Self {
            root,
            target_dir,
            os: std::env::consts::OS,
            exe_suffix: std::env::consts::EXE_SUFFIX,
        }
    }

    /// Where cargo puts the bin `bin` of profile `profile` (`release`, `debug`).
    pub fn exe(&self, profile: &str, bin: &str) -> PathBuf {
        self.target_dir
            .join(profile)
            .join(format!("{bin}{}", self.exe_suffix))
    }
}
