use crate::project;
use engine_proof::journal;
use engine_proof::{record, replay, Config, Input, Tape};
use std::ffi::OsString;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

// Const table for singleton flags. `--input` is repeatable, so it has no bit.
// Bitmask replaces the old BTreeSet<String>: zero heap allocs while parsing.
const SEEN_HELP: u16 = 1 << 0;
const SEEN_VERIFY: u16 = 1 << 1;
const SEEN_SEED: u16 = 1 << 2;
const SEEN_TICKS: u16 = 1 << 3;
const SEEN_RECORD: u16 = 1 << 4;
const SEEN_REPLAY: u16 = 1 << 5;
const SEEN_FORMAT: u16 = 1 << 6;
const SEEN_BACKEND: u16 = 1 << 7;
const SEEN_JOURNAL: u16 = 1 << 8;

fn flag_bit(flag: &str) -> Option<u16> {
    match flag {
        "--help" => Some(SEEN_HELP),
        "--verify" => Some(SEEN_VERIFY),
        "--seed" => Some(SEEN_SEED),
        "--ticks" => Some(SEEN_TICKS),
        "--record" => Some(SEEN_RECORD),
        "--replay" => Some(SEEN_REPLAY),
        "--format" => Some(SEEN_FORMAT),
        "--backend" => Some(SEEN_BACKEND),
        "--journal" => Some(SEEN_JOURNAL),
        _ => None,
    }
}

pub(crate) const HELP: &str = "engine2040 deterministic proof
  --seed <u64>                Initialization seed (default 2040)
  --ticks <1..100000>          Fixed 60 Hz steps (default 600)
  --input <tick:body:vx:vy>    Repeatable velocity assignment, before zero-based tick
                             Inputs must be unique and ordered by (tick, body)
  --record <path>             Create a new versioned input/checkpoint tape
  --journal <path>            Stream a crash-safe journal tick by tick while running
  --replay <path>             Replay saved inputs and compare every checkpoint
                             (reads a tape or a complete journal; a torn journal names its missing ticks)
  --verify                    Repeat this run from fresh initial state
  --format <text|json>        Receipt format (default text)
  --backend <null>            Supported proof backend; others fail explicitly
  --help                     Show this help
Replay owns its seed/ticks/inputs and cannot be combined with record, journal or overrides.
This fixture covers 2D ECS/physics, not GPU images or a complete game snapshot.

Game projects (a directory with a game.toml; templates: arena-duel, siege):
  new <template> <dir>        Create a project from a template; never overwrites a non-empty dir
  run --headless [--ticks <1..100000>] [--manifest <path>]
                             Run the project's seeded proof headless and print its receipt
  build --target <windows|web> [--manifest <path>]
                             Build the game for one target and print where it is
game = \"crate:bin\" in game.toml picks the game to build and run (default engine-game:arena);
`run` without it plays the proof fixture. --manifest defaults to ./game.toml.";

#[derive(Debug)]
pub enum Error {
    Usage(String),
    Runtime(Box<dyn std::error::Error>),
    /// A project verb (`new`, `run`, `build`) could not finish; the message says what to fix.
    Project(String),
}

impl Error {
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Usage(_) => 2,
            Self::Runtime(_) | Self::Project(_) => 1,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) => write!(f, "argument error: {message}; use --help"),
            Self::Runtime(error) => write!(f, "proof failed: {error}"),
            Self::Project(message) => f.write_str(message),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Project(format!("cannot write output: {error}"))
    }
}

#[derive(Default)]
struct Options {
    config: Config,
    inputs: Vec<Input>,
    record: Option<PathBuf>,
    journal: Option<PathBuf>,
    replay: Option<PathBuf>,
    verify: bool,
    json: bool,
    help: bool,
}

pub(crate) fn usage(message: impl Into<String>) -> Error {
    Error::Usage(message.into())
}

fn text(value: &OsString) -> Result<&str, Error> {
    value
        .to_str()
        .ok_or_else(|| usage("option/value must be UTF-8 (file paths may be native)"))
}

fn parse(mut args: impl Iterator<Item = OsString>) -> Result<Options, Error> {
    let mut options = Options::default();
    let mut seen: u16 = 0;
    while let Some(argument) = args.next() {
        let flag = text(&argument)?;
        if flag != "--input" {
            if let Some(bit) = flag_bit(flag) {
                if seen & bit != 0 {
                    return Err(usage(format!("duplicate option {flag}")));
                }
                seen |= bit;
            }
        }
        match flag {
            "--help" => options.help = true,
            "--verify" => options.verify = true,
            "--seed" | "--ticks" | "--input" | "--record" | "--journal" | "--replay"
            | "--format" | "--backend" => {
                let value = args
                    .next()
                    .ok_or_else(|| usage(format!("missing value for {flag}")))?;
                match flag {
                    "--record" => options.record = Some(PathBuf::from(value)),
                    "--journal" => options.journal = Some(PathBuf::from(value)),
                    "--replay" => options.replay = Some(PathBuf::from(value)),
                    "--seed" => {
                        options.config.seed = text(&value)?
                            .parse()
                            .map_err(|_| usage("seed must be a u64"))?;
                    }
                    "--ticks" => {
                        options.config.ticks = text(&value)?
                            .parse()
                            .map_err(|_| usage("ticks must be a u32"))?;
                    }
                    "--input" => {
                        if options.inputs.len() == engine_proof::MAX_INPUTS {
                            return Err(usage("too many inputs"));
                        }
                        options.inputs.push(parse_input(text(&value)?)?);
                    }
                    "--format" => {
                        options.json = match text(&value)? {
                            "json" => true,
                            "text" => false,
                            _ => return Err(usage("format must be text or json")),
                        }
                    }
                    "--backend" => {
                        if text(&value)? != "null" {
                            return Err(usage(
                                "unsupported proof backend; only null is implemented",
                            ));
                        }
                    }
                    _ => unreachable!("matched value option above"),
                }
            }
            _ => return Err(usage(format!("unknown option {flag}"))),
        }
    }
    if options.replay.is_some()
        && (options.record.is_some()
            || options.journal.is_some()
            || seen & (SEEN_SEED | SEEN_TICKS) != 0
            || !options.inputs.is_empty())
    {
        return Err(usage(
            "replay cannot override recorded seed/ticks/inputs or record a new tape",
        ));
    }
    Ok(options)
}

fn parse_input(value: &str) -> Result<Input, Error> {
    // No Vec: split borrows, early-out on missing or extra field.
    let mut parts = value.split(':');
    let missing = || usage("input must have tick:body:vx:vy");
    let tick = parts.next().ok_or_else(missing)?;
    let body = parts.next().ok_or_else(missing)?;
    let vx = parts.next().ok_or_else(missing)?;
    let vy = parts.next().ok_or_else(missing)?;
    if parts.next().is_some() {
        return Err(missing());
    }
    Ok(Input {
        tick: tick
            .parse()
            .map_err(|_| usage("input tick must be a u32"))?,
        body: body
            .parse()
            .map_err(|_| usage("input body must be a u32"))?,
        vx: vx.parse().map_err(|_| usage("input vx must be a number"))?,
        vy: vy.parse().map_err(|_| usage("input vy must be a number"))?,
    })
}

fn read_tape(path: &Path) -> Result<Tape, Box<dyn std::error::Error>> {
    engine_proof::check_tape_path(path)?;
    let file = File::open(path)?;
    let widest = engine_proof::tape::MAX_TAPE_BYTES.max(journal::max_journal_bytes());
    let limit = u64::try_from(widest)? + 1;
    // Single reserve from file size (capped): avoids ~21 reallocs growing to ~1.7 MB.
    let hint = usize::try_from(file.metadata().map(|m| m.len()).unwrap_or(0).min(limit))?;
    let mut bytes = Vec::with_capacity(hint);
    let mut capped = file.take(limit);
    capped.read_to_end(&mut bytes)?;
    // A journal replays once every planned tick is present; a torn one refuses
    // and says how many ticks are missing.
    let decoded = if bytes.starts_with(&journal::MAGIC) {
        journal::read_journal(&bytes)
            .and_then(journal::JournalRead::into_tape)
            .map_err(|error| error.to_string())
    } else {
        Tape::decode(&bytes).map_err(|error| error.to_string())
    };
    // Name the partial file on decode failure so a kill -9 truncated copy
    // refuses verify with the path as the reason (standalone EV19.b leg).
    decoded.map_err(|reason| {
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("{}: {reason}", path.display()),
        )) as Box<dyn std::error::Error>
    })
}

/// Run the fixture and stream every tick into a new journal file as it
/// finishes, so a crash keeps the finished ticks. The final sync covers power
/// loss for a run that completes.
fn record_journal_file(
    path: &Path,
    config: Config,
    inputs: &[Input],
) -> Result<Tape, Box<dyn std::error::Error>> {
    engine_proof::check_tape_path(path)?;
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let tape = journal::record_journal(config, inputs, &mut file)?;
    file.sync_all()?;
    Ok(tape)
}

fn write_tape(path: &Path, tape: &Tape) -> Result<(), Box<dyn std::error::Error>> {
    engine_proof::check_tape_path(path)?;
    let bytes = tape.encode()?;
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

fn execute(options: Options) -> Result<(), Box<dyn std::error::Error>> {
    let (tape, mode, verified) = if let Some(path) = &options.replay {
        let tape = read_tape(path)?;
        replay(&tape)?;
        (tape, "replay", true)
    } else if options.record.is_some() || options.journal.is_some() {
        let tape = match &options.journal {
            Some(path) => record_journal_file(path, options.config, &options.inputs)?,
            None => record(options.config, &options.inputs)?,
        };
        if let Some(path) = &options.record {
            write_tape(path, &tape)?;
        }
        if options.verify {
            // Verify-after-record replays the bytes actually written, so a
            // kill -9 truncated file never verifies.
            for path in options.record.iter().chain(&options.journal) {
                replay(&read_tape(path)?)?;
            }
        }
        (tape, "record", options.verify)
    } else {
        let tape = record(options.config, &options.inputs)?;
        if options.verify {
            replay(&tape)?;
        }
        (tape, "run", options.verify)
    };
    let last = tape.checkpoints.last().ok_or("missing final checkpoint")?;
    let golden_checked = tape.config == Config::default() && tape.inputs.is_empty();
    if options.json {
        // Seed is a decimal string so JSON consumers do not round large u64 values.
        println!(
            "{{\"schema_version\":1,\"fixture\":\"legacy-2d-v1\",\"engine_version\":\"{}\",\"platform\":\"{}-{}\",\"backend\":\"null\",\"mode\":\"{mode}\",\"seed\":\"{}\",\"ticks\":{},\"inputs\":{},\"final_hash\":\"{:016x}\",\"state_hash\":\"{:016x}\",\"verified\":{verified},\"golden_checked\":{golden_checked}}}",
            env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH,
            tape.config.seed, tape.config.ticks, tape.inputs.len(), last.legacy, last.state
        );
    } else {
        write_text_receipt(&mut std::io::stdout(), &tape, mode, verified)?;
    }
    Ok(())
}

/// The three text receipt lines of a finished tape. The golden recorder and `run --headless`
/// both print through here, so a project run and a golden run read the same.
pub(crate) fn write_text_receipt(
    out: &mut dyn Write,
    tape: &Tape,
    mode: &str,
    verified: bool,
) -> std::io::Result<()> {
    let Some(last) = tape.checkpoints.last() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "tape has no checkpoint",
        ));
    };
    writeln!(out, "engine2040 headless proof:")?;
    writeln!(
        out,
        "  ticks={} final_hash={:016x}",
        tape.config.ticks, last.legacy
    )?;
    writeln!(
        out,
        "  seed={} inputs={} state_hash={:016x} backend=null mode={mode} verified={verified}",
        tape.config.seed,
        tape.inputs.len(),
        last.state
    )
}

pub fn run(args: impl Iterator<Item = OsString>) -> Result<(), Error> {
    let mut args = args.peekable();
    let verb = args
        .peek()
        .and_then(|word| word.to_str())
        .and_then(project::Verb::from_word);
    if let Some(verb) = verb {
        args.next();
        return project::execute(verb, args, &mut std::io::stdout());
    }
    let options = parse(args)?;
    if options.help {
        println!("{HELP}");
        return Ok(());
    }
    execute(options).map_err(Error::Runtime)
}
