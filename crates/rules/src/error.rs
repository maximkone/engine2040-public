//! Typed refusals: every way a rule script can be turned down or fail, with a bounded text.

use std::fmt;

/// Which sandbox limit a script ran into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitKind {
    /// Operation budget per fighter call (`Limits::max_ops`).
    Operations,
    /// Script call depth (`Limits::max_call_levels`).
    CallDepth,
    /// Expression nesting (`Limits::max_expr_depth`).
    ExprDepth,
    /// Size of a value or variable count.
    Data,
}

/// Why a host call (a function the script called into the engine) was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostFault {
    /// Target fighter id outside the world.
    BadTarget(i64),
    /// An argument outside its documented range.
    BadArg {
        /// Host function name.
        call: &'static str,
        /// Offending value.
        value: i64,
    },
    /// More actions emitted in one call than `Limits::max_actions_per_call`.
    ActionCap,
    /// Memory slot outside `0..MEM_SLOTS`.
    MemSlot(i64),
}

/// Every refusal or failure of the rules crate. Display text is short and has no float.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RulesError {
    /// Source longer than `Limits::max_source_bytes`.
    TooLarge {
        /// Bytes given.
        bytes: usize,
        /// Bytes allowed.
        max: usize,
    },
    /// A float literal or exponent: scripts are integers only.
    Float {
        /// One-based line.
        line: usize,
    },
    /// The text does not parse.
    Syntax {
        /// One-based line (0 when unknown).
        line: usize,
        /// Parser message.
        message: String,
    },
    /// A variable that was never defined (strict mode).
    UnknownVariable {
        /// Name used.
        name: String,
        /// One-based line.
        line: usize,
    },
    /// A call to a function that is not in the allowed host API or the script.
    UnknownHostCall {
        /// Name called.
        name: String,
        /// Argument count used.
        arity: usize,
        /// One-based line.
        line: usize,
    },
    /// Syntax outside the allowed integer-only subset (strings, arrays, method calls).
    Forbidden {
        /// What was used.
        what: &'static str,
        /// One-based line.
        line: usize,
    },
    /// A `loop` / `while true` with no `break` or `return` in its body.
    UnboundedLoop {
        /// One-based line.
        line: usize,
    },
    /// A sandbox limit was hit while running.
    Limit {
        /// Which one.
        kind: LimitKind,
    },
    /// A host call refused its arguments.
    Host(HostFault),
    /// Integer overflow or division by zero.
    Arithmetic {
        /// Engine message.
        message: String,
    },
    /// The script called `throw` (value kept when it is an integer).
    Thrown {
        /// Thrown integer, 0 when not an integer.
        value: i64,
    },
    /// Any other runtime failure.
    Runtime {
        /// Engine message.
        message: String,
    },
    /// The world given to a tick has more fighters than `MAX_FIGHTERS`, or ids out of slot order.
    BadWorld {
        /// Actors given.
        fighters: usize,
    },
    /// `explain` was asked with no script installed.
    NoScript,
}

impl fmt::Display for RulesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { bytes, max } => write!(f, "script is {bytes} bytes, max {max}"),
            Self::Float { line } => {
                write!(f, "line {line}: float literal, scripts are integers only")
            }
            Self::Syntax { line, message } => write!(f, "line {line}: syntax: {message}"),
            Self::UnknownVariable { name, line } => {
                write!(f, "line {line}: unknown variable {name}")
            }
            Self::UnknownHostCall { name, arity, line } => {
                write!(f, "line {line}: unknown host call {name}/{arity}")
            }
            Self::Forbidden { what, line } => write!(f, "line {line}: {what} is not allowed"),
            Self::UnboundedLoop { line } => write!(f, "line {line}: loop with no break or return"),
            Self::Limit { kind } => write!(f, "sandbox limit hit: {kind:?}"),
            Self::Host(fault) => write!(f, "host call refused: {fault:?}"),
            Self::Arithmetic { message } => write!(f, "arithmetic: {message}"),
            Self::Thrown { value } => write!(f, "script threw {value}"),
            Self::Runtime { message } => write!(f, "runtime: {message}"),
            Self::BadWorld { fighters } => write!(f, "bad world with {fighters} fighters"),
            Self::NoScript => f.write_str("no script installed"),
        }
    }
}

impl std::error::Error for RulesError {}
