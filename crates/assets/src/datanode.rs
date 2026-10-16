//! datanode: indent-tree parser. Hierarchy = indent, tokens = whitespace split, errors carry line.

use std::fmt;

/// Plain data node: tokens + children in document order.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// Whitespace-separated tokens of the node's own line.
    pub tokens: Vec<String>,
    /// Nodes one indent level deeper, in document order.
    pub children: Vec<Node>,
}

impl Node {
    /// Token `i` of this node's line, if the line has one.
    pub fn token(&self, i: usize) -> Option<&str> {
        self.tokens.get(i).map(|s| s.as_str())
    }
    /// Token `i` parsed as `f32`; `None` when absent or not a number.
    pub fn value(&self, i: usize) -> Option<f32> {
        self.tokens.get(i)?.parse::<f32>().ok()
    }
    /// Direct children whose first token == name, in document order.
    pub fn find(&self, name: &str) -> Vec<&Node> {
        self.children
            .iter()
            .filter(|c| c.token(0) == Some(name))
            .collect()
    }
}

/// Indent-tree parse failure; every variant names its 1-based line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataNodeError {
    /// Indentation uses an odd number of spaces (use a tab or two spaces).
    OddSpaces {
        /// 1-based line number.
        line: usize,
    },
    /// The line holds only whitespace after its indent.
    EmptyTokenLine {
        /// 1-based line number.
        line: usize,
    },
    /// The indent is deeper than one level below its parent.
    IndentJump {
        /// 1-based line number.
        line: usize,
        /// Depth of the open parent chain.
        from: usize,
        /// Depth the line asked for.
        to: usize,
    },
    /// An indented line has no parent line above it.
    NoParent {
        /// 1-based line number.
        line: usize,
    },
}

impl fmt::Display for DataNodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OddSpaces { line } => {
                write!(f, "datanode:{line}: odd spaces, use tab or 2-space")
            }
            Self::EmptyTokenLine { line } => write!(f, "datanode:{line}: empty token line"),
            Self::IndentJump { line, from, to } => {
                write!(f, "datanode:{line}: indent jump {from}->{to}")
            }
            Self::NoParent { line } => write!(f, "datanode:{line}: no parent"),
        }
    }
}

impl std::error::Error for DataNodeError {}

/// Indent level and the text after the indent; `None` for an odd space count.
fn indent_level(line: &str) -> Option<(usize, &str)> {
    let mut tabs = 0usize;
    let mut spaces = 0usize;
    for c in line.chars() {
        match c {
            '\t' => tabs += 1,
            ' ' => spaces += 1,
            _ => break,
        }
    }
    if !spaces.is_multiple_of(2) {
        return None;
    }
    let level = tabs + spaces / 2;
    Some((level, &line[tabs + spaces..]))
}

/// Parse indent-tree text. Root is synthetic (empty tokens). Never panics.
pub fn parse(text: &str) -> Result<Node, DataNodeError> {
    let mut root = Node {
        tokens: vec![],
        children: vec![],
    };
    // Index paths, never raw pointers: (level, arena idx) stack.
    let mut arena: Vec<Node> = vec![];
    let mut stack: Vec<(usize, usize)> = vec![]; // (level, arena idx)
    for (ln, raw) in text.split('\n').enumerate() {
        let line_no = ln + 1;
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        let (level, rest) = indent_level(line).ok_or(DataNodeError::OddSpaces { line: line_no })?;
        if rest.trim().is_empty() {
            return Err(DataNodeError::EmptyTokenLine { line: line_no });
        }
        if level > stack.len() {
            return Err(DataNodeError::IndentJump {
                line: line_no,
                from: stack.len(),
                to: level,
            });
        }
        while stack.len() > level {
            stack.pop();
        }
        let node = Node {
            tokens: rest.split_whitespace().map(|s| s.to_string()).collect(),
            children: vec![],
        };
        arena.push(node);
        let idx = arena.len() - 1;
        if level == 0 {
            // top-level order tracked; attached to root later
            stack.push((level, idx));
        } else {
            let parent_idx = stack
                .last()
                .map(|(_, i)| *i)
                .ok_or(DataNodeError::NoParent { line: line_no })?;
            stack.push((level, idx));
            let _ = parent_idx;
        }
    }
    // Rebuild tree from arena + levels: second pass is simpler and safe.
    // Re-parse levels deterministically into tree.
    let mut levels: Vec<(usize, Node)> = vec![];
    for (ln, raw) in text.split('\n').enumerate() {
        let line_no = ln + 1;
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        let (level, rest) = indent_level(line).ok_or(DataNodeError::OddSpaces { line: line_no })?;
        levels.push((
            level,
            Node {
                tokens: rest.split_whitespace().map(|s| s.to_string()).collect(),
                children: vec![],
            },
        ));
    }
    fn attach(nodes: &mut Vec<(usize, Node)>, level: usize) -> Vec<Node> {
        let mut out = vec![];
        while !nodes.is_empty() && nodes[0].0 == level {
            let (_, mut n) = nodes.remove(0);
            if !nodes.is_empty() && nodes[0].0 > level {
                n.children = attach(nodes, level + 1);
            }
            out.push(n);
        }
        // mis-nesting was rejected earlier; consumed orphans skipped
        out
    }
    let mut work = levels;
    root.children = attach(&mut work, 0);
    let _ = arena;
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_parse_children() {
        let t = "ship\n\tattrs\n\t\tshields 500\n\tweapons 2\nfleet\n";
        let r = parse(t).unwrap();
        assert_eq!(r.children.len(), 2);
        assert_eq!(r.children[0].token(0), Some("ship"));
        assert_eq!(r.children[0].children[0].token(0), Some("attrs"));
        assert_eq!(r.children[0].children[0].children[0].value(1), Some(500.0));
    }

    #[test]
    fn malformed_indent_err_with_line() {
        let bad = "a\n   b\n"; // 3 spaces
        let e = parse(bad).unwrap_err();
        assert_eq!(e, DataNodeError::OddSpaces { line: 2 });
        assert!(e.to_string().contains(":2:"), "{e}");
        let jump = "a\n\t\tb\n"; // 0 -> 2
        let e2 = parse(jump).unwrap_err();
        assert_eq!(
            e2,
            DataNodeError::IndentJump {
                line: 2,
                from: 1,
                to: 2
            }
        );
        assert!(e2.to_string().contains(":2:"), "{e2}");
    }

    #[test]
    fn blank_lines_skip_and_errors_print_their_line() {
        assert_eq!(parse("a\n \t \n\n# note\nb\n").unwrap().children.len(), 2);
        let odd = parse("a\n b\n").unwrap_err();
        assert_eq!(
            odd.to_string(),
            "datanode:2: odd spaces, use tab or 2-space"
        );
        let jump = DataNodeError::IndentJump {
            line: 5,
            from: 1,
            to: 3,
        };
        assert_eq!(jump.to_string(), "datanode:5: indent jump 1->3");
        assert_eq!(
            DataNodeError::EmptyTokenLine { line: 9 }.to_string(),
            "datanode:9: empty token line"
        );
        assert_eq!(
            DataNodeError::NoParent { line: 4 }.to_string(),
            "datanode:4: no parent"
        );
    }

    #[test]
    fn value_parse() {
        let n = Node {
            tokens: vec!["hp".into(), "42.5".into(), "x".into()],
            children: vec![],
        };
        assert_eq!(n.value(1), Some(42.5));
        assert_eq!(n.value(2), None);
        assert_eq!(n.value(9), None);
    }

    #[test]
    fn determinism() {
        let t = "a\n\tb 1\n\tc 2\n";
        let (r1, r2) = (parse(t).unwrap(), parse(t).unwrap());
        assert_eq!(r1, r2);
    }
}
