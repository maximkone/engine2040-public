//! The rules worker: one thread per door session that owns the session's [`Rules`].
//!
//! Rhai without `sync` makes `Rules` `!Send`, so the engine never crosses a thread: the
//! worker builds it on its own thread and the door talks to it over a channel with plain,
//! `Send` messages (text, config, tape, checkpoints). The session state holds only the
//! sender, so dropping the session closes the channel and the worker exits. Requests are
//! answered one at a time in arrival order, which keeps `set`, `play.run` and `explain`
//! in the order the door took them.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{channel, Sender};
use std::thread::{Builder, JoinHandle};

use engine_arena::controller::SlotTape;
use engine_arena::spine_step::run_spine_controlled;
use engine_arena::world::MatchWorld;
use engine_arena::{run, run_controlled, ArenaConfig, Checkpoint};
use engine_rules::arena::{DriveStats, RulesController};
use engine_rules::{Explanation, Rules, RulesError, SetReceipt, TestReport};

use crate::mcp::McpToolError;

/// What `rules.set` reports back from the worker.
pub(crate) type SetReply = Result<SetReceipt, RulesError>;

/// What a driven match gives back.
pub(crate) struct RunReply {
    pub checkpoints: Vec<Checkpoint>,
    pub stats: DriveStats,
}

/// One installed script as `rules.get` and `rules.undo` report it.
pub(crate) struct ScriptInfo {
    pub rev: u32,
    pub hash: u64,
    pub source: String,
    pub slots: Vec<u8>,
}

/// Why a read-back or undo refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScriptRefusal {
    /// Nothing installed yet.
    NoScript,
    /// One install, so there is nothing before it.
    NothingToUndo,
    /// The previous script no longer compiles.
    RulesRefused,
}

/// A read-back or undo reply: the installed script, or the refusal.
type ScriptReply = Result<ScriptInfo, ScriptRefusal>;

/// Most previous scripts one session can walk back through.
const MAX_UNDO: usize = 16;

enum Request {
    Set {
        source: String,
        slots: Vec<u8>,
        reply: Sender<SetReply>,
    },
    Test {
        source: String,
        reply: Sender<TestReport>,
    },
    Get {
        reply: Sender<ScriptReply>,
    },
    Undo {
        reply: Sender<ScriptReply>,
    },
    Run {
        config: ArenaConfig,
        tape: SlotTape,
        world: Option<MatchWorld>,
        explain_at: Option<u32>,
        reply: Sender<RunReply>,
    },
}

/// The sender side plus what the door must know without asking the thread.
pub(crate) struct Worker {
    tx: Sender<Request>,
    /// Kept so the thread is owned by the session; never joined (it ends on channel close).
    _thread: JoinHandle<()>,
    /// Script hash and driven slots of the last good `rules.set`.
    installed: Option<(u64, Vec<u8>)>,
}

impl Worker {
    /// Start the thread. `None` when the OS refuses a thread.
    pub(crate) fn spawn() -> Option<Self> {
        let (tx, rx) = channel::<Request>();
        let thread = Builder::new()
            .name("rules-worker".to_owned())
            .spawn(move || serve(&rx))
            .ok()?;
        Some(Self {
            tx,
            _thread: thread,
            installed: None,
        })
    }

    /// Hash and slots of the installed script, if any.
    pub(crate) fn installed(&self) -> Option<&(u64, Vec<u8>)> {
        self.installed.as_ref()
    }

    /// Install `source` driving `slots`; the last good script stays on any refusal.
    pub(crate) fn set(&mut self, source: &str, slots: Vec<u8>) -> Result<SetReply, McpToolError> {
        let (reply, answer) = channel();
        self.send(Request::Set {
            source: source.to_owned(),
            slots: slots.clone(),
            reply,
        })?;
        let result = answer.recv().map_err(|_| down())?;
        if let Ok(receipt) = &result {
            self.installed = Some((receipt.hash, slots));
        }
        Ok(result)
    }

    /// The installed script, its revision and its slots.
    pub(crate) fn get(&self) -> Result<ScriptInfo, McpToolError> {
        let (reply, answer) = channel();
        self.send(Request::Get { reply })?;
        script_reply(answer.recv().map_err(|_| down())?)
    }

    /// Reinstall the script installed before the current one, and keep the
    /// current one so a second undo walks forward again.
    pub(crate) fn undo(&mut self) -> Result<ScriptInfo, McpToolError> {
        let (reply, answer) = channel();
        self.send(Request::Undo { reply })?;
        let info = script_reply(answer.recv().map_err(|_| down())?)?;
        self.installed = Some((info.hash, info.slots.clone()));
        Ok(info)
    }

    /// Dry-run `source` over the fixture worlds; installs nothing.
    pub(crate) fn test(&self, source: &str) -> Result<TestReport, McpToolError> {
        let (reply, answer) = channel();
        self.send(Request::Test {
            source: source.to_owned(),
            reply,
        })?;
        answer.recv().map_err(|_| down())
    }

    /// Play a match with the installed script driving its slots, or the plain arena when
    /// none is installed. `explain_at` also dry-runs the explanation at that tick.
    pub(crate) fn run(
        &self,
        config: &ArenaConfig,
        tape: &SlotTape,
        world: Option<&MatchWorld>,
        explain_at: Option<u32>,
    ) -> Result<RunReply, McpToolError> {
        let (reply, answer) = channel();
        self.send(Request::Run {
            config: *config,
            tape: tape.clone(),
            world: world.cloned(),
            explain_at,
            reply,
        })?;
        answer.recv().map_err(|_| down())
    }

    fn send(&self, request: Request) -> Result<(), McpToolError> {
        self.tx.send(request).map_err(|_| down())
    }
}

fn down() -> McpToolError {
    McpToolError::new(
        "rules_worker_down",
        "the session's rules worker stopped; call rules.set again to start a fresh one",
    )
}

/// Turn a worker script reply into the door's answer.
fn script_reply(reply: ScriptReply) -> Result<ScriptInfo, McpToolError> {
    match reply {
        Ok(info) => Ok(info),
        Err(ScriptRefusal::NoScript) => Err(McpToolError::new(
            "no_script",
            "no rule script is installed; call rules.set first",
        )),
        Err(ScriptRefusal::NothingToUndo) => Err(McpToolError::new(
            "nothing_to_undo",
            "one script is installed and none came before it; call rules.set first",
        )),
        Err(ScriptRefusal::RulesRefused) => Err(McpToolError::new(
            "rules_refused",
            "the previous script no longer compiles under this engine; the last good script stays",
        )),
    }
}

/// The worker loop. Everything `!Send` lives and dies in here.
fn serve(rx: &std::sync::mpsc::Receiver<Request>) {
    let rules = Rc::new(RefCell::new(Rules::default()));
    let mut slots: Vec<u8> = Vec::new();
    let mut undo: Vec<(String, Vec<u8>)> = Vec::new();
    while let Ok(request) = rx.recv() {
        match request {
            Request::Set {
                source,
                slots: wanted,
                reply,
            } => {
                let previous = rules
                    .borrow()
                    .source()
                    .map(|text| (text.to_owned(), slots.clone()));
                let result = rules.borrow_mut().set(&source, 0);
                if result.is_ok() {
                    if let Some(previous) = previous {
                        if undo.len() == MAX_UNDO {
                            undo.remove(0);
                        }
                        undo.push(previous);
                    }
                    slots = wanted;
                }
                let _ = reply.send(result);
            }
            Request::Get { reply } => {
                let _ = reply.send(installed_script(&rules.borrow(), &slots));
            }
            Request::Undo { reply } => {
                let Some((source, previous_slots)) = undo.pop() else {
                    let _ = reply.send(Err(ScriptRefusal::NothingToUndo));
                    continue;
                };
                let current = rules
                    .borrow()
                    .source()
                    .map(|text| (text.to_owned(), slots.clone()));
                match rules.borrow_mut().set(&source, 0) {
                    Ok(receipt) => {
                        slots = previous_slots;
                        if let Some(current) = current {
                            if undo.len() == MAX_UNDO {
                                undo.remove(0);
                            }
                            undo.push(current);
                        }
                        let _ = reply.send(Ok(ScriptInfo {
                            rev: receipt.rev,
                            hash: receipt.hash,
                            source,
                            slots: slots.clone(),
                        }));
                    }
                    Err(_) => {
                        undo.push((source, previous_slots));
                        let _ = reply.send(Err(ScriptRefusal::RulesRefused));
                    }
                }
            }
            Request::Test { source, reply } => {
                let worlds = engine_rules::fixtures::smoke_worlds();
                let _ = reply.send(rules.borrow().test(&source, &worlds));
            }
            Request::Run {
                config,
                tape,
                world,
                explain_at,
                reply,
            } => {
                let _ = reply.send(play(&rules, &slots, &config, &tape, world, explain_at));
            }
        }
    }
}

fn installed_script(rules: &Rules, slots: &[u8]) -> ScriptReply {
    let source = rules.source().ok_or(ScriptRefusal::NoScript)?;
    Ok(ScriptInfo {
        rev: rules.history().last().map_or(0, |installed| installed.rev),
        hash: rules.script_hash().unwrap_or(0),
        source: source.to_owned(),
        slots: slots.to_vec(),
    })
}

fn play(
    rules: &Rc<RefCell<Rules>>,
    slots: &[u8],
    config: &ArenaConfig,
    tape: &SlotTape,
    world: Option<MatchWorld>,
    explain_at: Option<u32>,
) -> RunReply {
    let installed = rules.borrow().script_hash().is_some();
    if !installed || slots.is_empty() {
        let checkpoints = match &world {
            Some(world) => engine_arena::spine_step::run_spine(config, tape, world),
            None => run(config, tape),
        };
        return RunReply {
            checkpoints,
            stats: DriveStats::default(),
        };
    }
    let mut controller = RulesController::new(Rc::clone(rules), slots);
    if let Some(tick) = explain_at {
        controller = controller.explain_at(tick);
    }
    let stats = controller.stats();
    let checkpoints = match world {
        Some(world) => run_spine_controlled(config, tape, &world, Box::new(controller)),
        None => run_controlled(config, tape, &mut controller),
    };
    let stats = stats.borrow().clone();
    RunReply { checkpoints, stats }
}

/// The explanation a run produced, if it asked for one.
pub(crate) fn explanation(reply: &RunReply) -> Option<&Explanation> {
    reply.stats.explanation.as_ref()
}
