//! Play in a child process: launch the game, watch it, kill it.
//!
//! The consumer is the editor shell's play button (the host supplies the real
//! game binary); the data owner is [`ChildPlay`], which owns at most one
//! supervised child. Only [`std::process`] is used: the caller supplies the
//! entrypoint as a program path plus arguments, so this module never guesses
//! a binary location and the editor never shares its live [`Document`] with
//! the child. The host can pass a project snapshot path as an argument; a killed
//! game then leaves the authoring document recoverable by construction.
//!
//! The child's stdio is nulled so a chatty game can neither block the editor
//! on a full pipe nor steal its stdin. [`ChildPlay::kill`] reaps the child,
//! so a killed game is never a zombie, and [`Drop`] kills best-effort so a
//! dropped handle never leaks a game process.
//!
//! [`Document`]: crate::commands::Document

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// What to launch: a program path plus its arguments, supplied by the caller.
///
/// Tests use an instant-exit stub command; production supplies the packaged
/// game binary with a project snapshot argument.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayEntrypoint {
    /// Program to run (resolved by the OS `PATH` rules when relative).
    pub program: PathBuf,
    /// Arguments passed verbatim, e.g. a snapshot path for the real game.
    pub args: Vec<String>,
}

impl PlayEntrypoint {
    /// Build an entrypoint from a program plus verbatim arguments.
    pub fn new(
        program: impl Into<PathBuf>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
        }
    }

    /// The program path.
    pub fn program(&self) -> &Path {
        &self.program
    }
}

impl fmt::Display for PlayEntrypoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.program.display())?;
        for arg in &self.args {
            write!(f, " {arg}")?;
        }
        Ok(())
    }
}

/// One supervised game process: spawned by [`ChildPlay::spawn`], watched by
/// [`ChildPlay::is_alive`], stopped by [`ChildPlay::kill`].
pub struct ChildPlay {
    child: Option<Child>,
    command_line: String,
}

impl ChildPlay {
    /// Launch the entrypoint as a child with nulled stdio.
    ///
    /// Err when the program cannot start (missing binary, denied); nothing
    /// is spawned on failure.
    pub fn spawn(entry: &PlayEntrypoint) -> std::io::Result<Self> {
        let child = Command::new(&entry.program)
            .args(&entry.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        Ok(Self {
            child: Some(child),
            command_line: entry.to_string(),
        })
    }

    /// True while the child is still running. A spawn failure, an exited
    /// child and a killed child all read false; never blocks.
    pub fn is_alive(&mut self) -> bool {
        match self.child.as_mut() {
            None => false,
            Some(child) => match child.try_wait() {
                Ok(None) => true,
                Ok(Some(_)) | Err(_) => false,
            },
        }
    }

    /// Stop the child and reap it. Idempotent: an exited or already-killed
    /// child is Ok without a signal. The editor's document is untouched;
    /// only the child process is affected.
    pub fn kill(&mut self) -> std::io::Result<()> {
        if let Some(child) = self.child.as_mut() {
            match child.try_wait()? {
                Some(_) => Ok(()),
                None => {
                    child.kill()?;
                    child.wait()?;
                    Ok(())
                }
            }
        } else {
            Ok(())
        }
    }

    /// The command line as spawned, for log diagnostics.
    pub fn command_line(&self) -> &str {
        &self.command_line
    }
}

impl Drop for ChildPlay {
    /// Best-effort stop so a dropped handle never leaks a running game.
    fn drop(&mut self) {
        let _ = self.kill();
    }
}

/// Fixed ship files; mirrors `tools/ship.mjs` MANIFESTED.
#[rustfmt::skip]
pub const SHIP_FILES: [&str; 5] = ["bin/arena.exe", "content/arena-bundle.bin", "content/contents.txt", "saves/progress-schema-v1.txt", "launch.txt"];
/// Stage ship files from `src` to clean `dist`; missing fails loudly.
#[rustfmt::skip]
pub fn export_game(src: &Path, dist: &Path) -> std::io::Result<()> {
    for rel in SHIP_FILES {
        let from = src.join(rel);
        if !from.is_file() {
            return Err(std::io::Error::new(std::io::ErrorKind::NotFound, format!("ship missing {rel}")));
        }
        if let Some(parent) = dist.join(rel).parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&from, &dist.join(rel))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{apply, Batch, Document, Journal, Op};
    use engine_scene::ids::ObjectId;
    use engine_scene::transform::Transform;
    use std::time::{Duration, Instant};

    /// A stub game: exits at once so tests never sleep. `cmd` is a system
    /// binary on Windows; `true` covers unix CI the same way.
    #[cfg(windows)]
    fn stub_entrypoint() -> PlayEntrypoint {
        PlayEntrypoint::new("cmd", ["/C", "exit", "0"])
    }

    /// A stub game: exits at once so tests never sleep.
    #[cfg(not(windows))]
    fn stub_entrypoint() -> PlayEntrypoint {
        PlayEntrypoint::new("true", Vec::<String>::new())
    }

    /// Wait until the stub has exited. An instant stub lands in
    /// milliseconds; a sleeping stub times out and fails here.
    fn wait_until_dead(play: &mut ChildPlay) -> bool {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            if !play.is_alive() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        !play.is_alive()
    }

    #[test]
    fn exited_child_reads_dead_and_kill_is_idempotent() {
        let entry = stub_entrypoint();
        assert!(!entry.to_string().is_empty());
        let mut play = ChildPlay::spawn(&entry).expect("stub process starts");
        assert!(wait_until_dead(&mut play), "instant stub exits at once");

        play.kill().expect("kill on an exited stub is Ok");
        assert!(!play.is_alive(), "exited stub reads dead");

        // Second kill is a no-op Ok; liveness stays false.
        play.kill().expect("repeat kill is idempotent");
        assert!(!play.is_alive());
    }

    #[test]
    fn spawn_failure_is_an_error_not_a_handle() {
        let missing = PlayEntrypoint::new("e2040-no-such-program-xyz", ["--stub"]);
        assert!(ChildPlay::spawn(&missing).is_err());
    }

    #[test]
    fn exited_game_leaves_the_document_intact() {
        // Authoring document with one applied batch.
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let id = ObjectId::new();
        let rev = doc.revision;
        apply(
            &mut doc,
            &mut journal,
            &Batch {
                ops: vec![Op::Insert {
                    id,
                    parent: ObjectId::NIL,
                    local: Transform::new([1.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
                }],
                base_revision: rev,
                idempotency_key: 1,
                provenance: None,
            },
        )
        .unwrap();
        let snapshot = doc.clone();

        // Play the instant stub to exit: only the child is affected.
        let mut play = ChildPlay::spawn(&stub_entrypoint()).expect("stub process starts");
        assert!(wait_until_dead(&mut play), "instant stub exits at once");
        play.kill().expect("kill on an exited stub is Ok");
        assert!(!play.is_alive());

        assert_eq!(doc, snapshot, "document survives the exited game");
        assert_eq!(doc.hierarchy.local(id), snapshot.hierarchy.local(id));
    }

    #[test]
    #[rustfmt::skip]
    fn export_stages_and_missing_fails() {
        let src = std::env::temp_dir().join(format!("e2040-export-src-{}", std::process::id()));
        let dist = std::env::temp_dir().join(format!("e2040-export-dist-{}", std::process::id()));
        assert!(export_game(&src, &dist).unwrap_err().to_string().contains("arena.exe"));
        for rel in SHIP_FILES {
            std::fs::create_dir_all(src.join(rel).parent().unwrap()).unwrap();
            std::fs::write(src.join(rel), b"ship").unwrap();
        }
        export_game(&src, &dist).unwrap();
    }
}
