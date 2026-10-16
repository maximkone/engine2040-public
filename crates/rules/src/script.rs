//! Compile and vet one script: the raw sandbox engine, the float scan, the AST checks.
//!
//! A script is a list of statements (plus optional helper functions) that runs once per living
//! actor per tick, with the actor's numbers bound as read-only variables (`hp`, `blocking`, ...,
//! see `host::PROPS`). It may call only the allowed host API and its own functions, and uses no
//! strings, arrays, maps, property access or method calls. Floats, imports, closures and the
//! clock are absent from the engine itself (crate features), so the checks here give typed
//! errors, not safety.

use std::rc::Rc;

use engine_core::Fnv64;
use rhai::packages::{ArithmeticPackage, BasicIteratorPackage, LogicPackage, Package};
use rhai::{
    ASTFlags, ASTNode, Engine, EvalAltResult, Expr, OptimizationLevel, ParseErrorType, Scope, Stmt,
    AST,
};

use crate::error::{HostFault, LimitKind, RulesError};
use crate::host::{self, prop_value, ActorView, HostCtx, HOST_FNS, INPUT_FNS, PROPS};
use crate::limits::Limits;

/// A script that passed `compile`: normalized text, its hash, the checked AST and which
/// `PROPS` the script reads (indices, ascending), so only those are bound per actor.
pub(crate) struct Compiled {
    pub(crate) source: String,
    pub(crate) hash: u64,
    pub(crate) ast: AST,
    pub(crate) vars: Vec<usize>,
}

/// FNV-1a 64 of the script after line-ending normalization. The tape header stores this.
pub fn script_hash(source: &str) -> u64 {
    let mut hasher = Fnv64::new();
    hasher.write(normalize(source).as_bytes());
    hasher.finish()
}

/// `\r\n` and lone `\r` become `\n`, so a script hashes the same on every checkout.
pub(crate) fn normalize(source: &str) -> String {
    source.replace("\r\n", "\n").replace('\r', "\n")
}

/// Build the deny-all engine: no standard library, only integer arithmetic, comparisons and
/// range iteration, the host API of `host`, and the limits. `count_ops` adds the progress hook
/// that reports each actor's operation count (tests and explains; the tick engine skips it).
pub(crate) fn build_engine(ctx: &Rc<HostCtx>, limits: &Limits, count_ops: bool) -> Engine {
    let mut engine = Engine::new_raw();
    engine.register_global_module(ArithmeticPackage::new().as_shared_module());
    engine.register_global_module(LogicPackage::new().as_shared_module());
    engine.register_global_module(BasicIteratorPackage::new().as_shared_module());
    host::register(&mut engine, ctx);

    // One engine run per actor, so this is the per-actor budget.
    engine.set_max_operations(limits.max_ops);
    engine.set_max_call_levels(limits.max_call_levels);
    engine.set_max_expr_depths(limits.max_expr_depth, limits.max_fn_expr_depth);
    // The bound read-only variables count as variables too.
    engine.set_max_variables(limits.max_variables + PROPS.len());
    engine.set_max_functions(limits.max_functions);
    // Strings, arrays and maps are refused by the AST check; the engine caps are a second lock.
    engine.set_max_string_size(16);
    engine.set_max_array_size(0);
    engine.set_max_map_size(0);
    engine.set_strict_variables(true);
    // No constant propagation: the bound variables are re-valued for every actor, so the compiled
    // text must keep reading them instead of folding in the compile-time dummies.
    engine.set_optimization_level(OptimizationLevel::None);
    for symbol in [
        "eval",
        "Fn",
        "call",
        "curry",
        "print",
        "debug",
        "type_of",
        "is_def_fn",
        "is_def_var",
    ] {
        engine.disable_symbol(symbol);
    }
    if count_ops {
        let progress_ctx = Rc::clone(ctx);
        engine.on_progress(move |ops| {
            progress_ctx.progress(ops);
            None
        });
    }
    engine
}

/// Bind the chosen `PROPS` for one actor. Used with a rewound scope at run time and with
/// dummy values at compile time, so both have the same layout.
pub(crate) fn push_vars(scope: &mut Scope<'_>, vars: &[usize], view: &ActorView) {
    for &index in vars {
        scope.push_constant_dynamic(PROPS[index], prop_value(index, view));
    }
}

fn parse(engine: &Engine, source: &str, vars: &[usize]) -> Result<AST, RulesError> {
    let mut scope = Scope::new();
    push_vars(&mut scope, vars, &ActorView::default());
    engine.compile_with_scope(&scope, source).map_err(|err| {
        let line = err.1.line().unwrap_or(0);
        match *err.0 {
            ParseErrorType::VariableUndefined(name) => RulesError::UnknownVariable { name, line },
            ref other => RulesError::Syntax {
                line,
                message: other.to_string(),
            },
        }
    })
}

/// Compile and vet `source`. Errors are typed; nothing is installed.
pub(crate) fn compile(
    engine: &Engine,
    source: &str,
    limits: &Limits,
) -> Result<Compiled, RulesError> {
    if source.len() > limits.max_source_bytes {
        return Err(RulesError::TooLarge {
            bytes: source.len(),
            max: limits.max_source_bytes,
        });
    }
    let normalized = normalize(source);
    if let Some(line) = find_float(&normalized) {
        return Err(RulesError::Float { line });
    }
    // Pass 1 knows every variable and finds which ones the script reads; pass 2 binds only
    // those, so the compiled variable positions match the scope built for each actor.
    let every: Vec<usize> = (0..PROPS.len()).collect();
    let first = parse(engine, &normalized, &every)?;
    let vars = check_ast(&first)?;
    let ast = parse(engine, &normalized, &vars)?;
    Ok(Compiled {
        hash: script_hash(&normalized),
        source: normalized,
        ast,
        vars,
    })
}

/// Line of the first float literal (a digit run followed by `.digit` or an exponent), outside
/// strings and comments. A script with strings is refused later, but the scan stays correct.
fn find_float(source: &str) -> Option<usize> {
    let bytes = source.as_bytes();
    let (mut i, mut line, mut block_depth) = (0_usize, 1_usize, 0_usize);
    while i < bytes.len() {
        let b = bytes[i];
        let next = bytes.get(i + 1).copied();
        if b == b'\n' {
            line += 1;
        }
        if block_depth > 0 {
            if b == b'*' && next == Some(b'/') {
                block_depth -= 1;
                i += 1;
            } else if b == b'/' && next == Some(b'*') {
                block_depth += 1;
                i += 1;
            }
        } else if b == b'/' && next == Some(b'/') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        } else if b == b'/' && next == Some(b'*') {
            block_depth = 1;
            i += 1;
        } else if matches!(b, b'"' | b'\'' | b'`') {
            i += 1;
            while i < bytes.len() && bytes[i] != b {
                if bytes[i] == b'\\' {
                    i += 1;
                }
                if bytes.get(i) == Some(&b'\n') {
                    line += 1;
                }
                i += 1;
            }
        } else if b.is_ascii_digit() && (i == 0 || !is_ident(bytes[i - 1])) {
            let start_line = line;
            if b == b'0' && matches!(next, Some(b'x' | b'X' | b'b' | b'B' | b'o' | b'O')) {
                i += 2;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                continue;
            }
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
                i += 1;
            }
            let (here, after) = (bytes.get(i).copied(), bytes.get(i + 1).copied());
            let exponent = matches!(here, Some(b'e' | b'E'))
                && (after.is_some_and(|c| c.is_ascii_digit())
                    || (matches!(after, Some(b'+' | b'-'))
                        && bytes.get(i + 2).is_some_and(u8::is_ascii_digit)));
            let fraction = here == Some(b'.') && after.is_some_and(|c| c.is_ascii_digit());
            if exponent || fraction {
                return Some(start_line);
            }
            continue;
        }
        i += 1;
    }
    None
}

fn is_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// AST checks: allowed calls, integer-only syntax, no property access, no loop without an exit.
/// Returns the `PROPS` indices the script reads, ascending.
fn check_ast(ast: &AST) -> Result<Vec<usize>, RulesError> {
    let fns: Vec<(String, usize)> = ast
        .iter_functions()
        .map(|f| (f.name.to_string(), f.params.len()))
        .collect();
    let mut failure: Option<RulesError> = None;
    let mut used = vec![false; PROPS.len()];
    ast.walk(&mut |path: &[ASTNode<'_>]| {
        let Some(node) = path.last() else { return true };
        failure = match node {
            ASTNode::Expr(Expr::Variable(var, ..)) => {
                if let Some(index) = PROPS.iter().position(|p| *p == var.1.as_str()) {
                    used[index] = true;
                }
                None
            }
            ASTNode::Expr(expr) => check_expr(expr, &fns),
            ASTNode::Stmt(stmt) => check_stmt(stmt, &fns),
            _ => None,
        };
        failure.is_none()
    });
    if let Some(err) = failure {
        return Err(err);
    }
    Ok(used
        .iter()
        .enumerate()
        .filter_map(|(index, read)| read.then_some(index))
        .collect())
}

fn line_of(pos: rhai::Position) -> usize {
    pos.line().unwrap_or(0)
}

fn check_call(
    call: &rhai::FnCallExpr,
    pos: rhai::Position,
    fns: &[(String, usize)],
) -> Option<RulesError> {
    let name: &str = call.name.as_str();
    let arity = call.args.len();
    let symbolic = !name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_');
    if call.is_operator_call() || symbolic {
        return None;
    }
    let known = HOST_FNS
        .iter()
        .chain(INPUT_FNS)
        .any(|(n, a)| *n == name && *a == arity)
        || fns.iter().any(|(n, a)| n == name && *a == arity);
    (!known).then(|| RulesError::UnknownHostCall {
        name: name.to_string(),
        arity,
        line: line_of(pos),
    })
}

fn check_expr(expr: &Expr, fns: &[(String, usize)]) -> Option<RulesError> {
    let forbid = |what: &'static str, pos: rhai::Position| {
        Some(RulesError::Forbidden {
            what,
            line: line_of(pos),
        })
    };
    match expr {
        Expr::FnCall(call, pos) => check_call(call, *pos, fns),
        Expr::MethodCall(_, pos) => forbid("method call (use function-call syntax)", *pos),
        Expr::Property(_, pos) => forbid(
            "property access (the actor's numbers are plain variables)",
            *pos,
        ),
        Expr::StringConstant(_, pos) | Expr::InterpolatedString(_, pos) => forbid("string", *pos),
        Expr::CharConstant(_, pos) => forbid("character literal", *pos),
        // `a..b` with constant ends can fold into a range constant; any other folded constant
        // (array, map) cannot be written without the literals refused here.
        Expr::DynamicConstant(value, pos)
            if !(value.is::<std::ops::Range<i64>>()
                || value.is::<std::ops::RangeInclusive<i64>>()) =>
        {
            forbid("constant array or map", *pos)
        }
        Expr::Array(_, pos) => forbid("array", *pos),
        Expr::Map(_, pos) => forbid("map", *pos),
        Expr::Index(_, _, pos) => forbid("index", *pos),
        _ => None,
    }
}

fn check_stmt(stmt: &Stmt, fns: &[(String, usize)]) -> Option<RulesError> {
    match stmt {
        Stmt::FnCall(call, pos) => check_call(call, *pos, fns),
        Stmt::While(flow, pos) => {
            let endless = matches!(flow.expr, Expr::Unit(_) | Expr::BoolConstant(true, _));
            (endless && !has_exit(&flow.body)).then(|| RulesError::UnboundedLoop {
                line: line_of(*pos),
            })
        }
        _ => None,
    }
}

/// True when the block holds a `break`, `return` or `throw` anywhere (nested loops count: a
/// conservative yes; the operation budget still stops a lie at run time).
fn has_exit(body: &rhai::StmtBlock) -> bool {
    let mut found = false;
    let mut path = Vec::new();
    for stmt in body.statements() {
        stmt.walk(&mut path, &mut |nodes: &[ASTNode<'_>]| {
            match nodes.last() {
                Some(ASTNode::Stmt(Stmt::BreakLoop(_, flags, _)))
                    if flags.contains(ASTFlags::BREAK) =>
                {
                    found = true;
                }
                Some(ASTNode::Stmt(Stmt::Return(..))) => found = true,
                _ => {}
            }
            !found
        });
    }
    found
}

/// Map an engine failure to a typed error.
pub(crate) fn map_eval_error(err: &EvalAltResult) -> RulesError {
    match err {
        EvalAltResult::ErrorTooManyOperations(_) => RulesError::Limit {
            kind: LimitKind::Operations,
        },
        EvalAltResult::ErrorStackOverflow(_) => RulesError::Limit {
            kind: LimitKind::CallDepth,
        },
        EvalAltResult::ErrorDataTooLarge(..) | EvalAltResult::ErrorTooManyVariables(_) => {
            RulesError::Limit {
                kind: LimitKind::Data,
            }
        }
        EvalAltResult::ErrorParsing(..) => RulesError::Limit {
            kind: LimitKind::ExprDepth,
        },
        EvalAltResult::ErrorRuntime(value, _) => {
            if let Some(fault) = value.clone().try_cast::<HostFault>() {
                RulesError::Host(fault)
            } else {
                RulesError::Thrown {
                    value: value.as_int().unwrap_or(0),
                }
            }
        }
        EvalAltResult::ErrorArithmetic(message, _) => RulesError::Arithmetic {
            message: message.clone(),
        },
        EvalAltResult::ErrorInFunctionCall(_, _, inner, _) => map_eval_error(inner),
        EvalAltResult::ErrorFunctionNotFound(signature, pos) => RulesError::UnknownHostCall {
            name: signature.split('(').next().unwrap_or("?").to_string(),
            arity: 0,
            line: line_of(*pos),
        },
        other => RulesError::Runtime {
            message: other.to_string(),
        },
    }
}

#[cfg(test)]
mod script_hash_tests {
    use super::{normalize, script_hash};

    /// Line endings normalize before anything else reads the text: CRLF and
    /// lone CR become LF, so the same script checks out identically on
    /// every platform. The tape header stores this hash.
    #[test]
    fn normalize_unifies_crlf_and_lone_cr_to_lf() {
        assert_eq!(
            normalize("a\r\nb\r\n"),
            "a\nb\n",
            "crlf pairs collapse via script"
        );
        assert_eq!(normalize("a\rb"), "a\nb", "lone cr becomes lf via script");
        assert_eq!(normalize("a\nb"), "a\nb", "lf passes through via script");
        assert_eq!(normalize(""), "", "empty stays empty via script");
        assert_eq!(
            normalize("no endings here"),
            "no endings here",
            "plain text passes through via script"
        );
    }

    /// The hash callers store (`tape` header, `Compiled.hash`) is stable
    /// across checkout line endings yet still distinguishes real edits.
    #[test]
    fn script_hash_is_stable_across_endings_and_sensitive_to_edits() {
        let unix = "fn on_tick(me) {\n}";
        assert_eq!(
            script_hash(unix),
            script_hash("fn on_tick(me) {\r\n}"),
            "crlf hashes like lf via script"
        );
        assert_eq!(
            script_hash(unix),
            script_hash("fn on_tick(me) {\r}"),
            "lone cr hashes like lf via script"
        );
        assert_eq!(
            script_hash(unix),
            script_hash(unix),
            "same text hashes identically via script"
        );
        assert_ne!(
            script_hash(unix),
            script_hash("fn on_tick(me) {\n}\n"),
            "trailing newline moves the hash via script"
        );
        assert_ne!(
            script_hash(unix),
            script_hash("fn on_tick(other) {\n}"),
            "renamed binding moves the hash via script"
        );
    }
}
