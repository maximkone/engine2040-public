//! inherit: copy-from resolver. Topological order, last key wins, std-only.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Content def: id + optional parent link + ordered fields (last wins on dup key).
#[derive(Clone, Debug)]
pub struct Def {
    pub id: String,
    pub parent: Option<String>,
    pub fields: Vec<(String, String)>,
}

/// Resolved def: flattened fields sorted by key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub id: String,
    pub fields: Vec<(String, String)>,
}

/// Why copy-from resolution refused its defs; every variant names the culprit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InheritError {
    /// A def has an empty id.
    EmptyId,
    /// Two defs share one id.
    DuplicateId {
        /// The repeated id.
        id: String,
    },
    /// A def names a parent that no def declares.
    MissingParent {
        /// The def holding the dangling link.
        id: String,
        /// The parent id nothing declares.
        parent: String,
    },
    /// The parent links form a cycle through this id.
    Cycle {
        /// Lexically first id on the cycle.
        at: String,
    },
}

impl fmt::Display for InheritError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyId => write!(f, "empty id"),
            Self::DuplicateId { id } => write!(f, "duplicate id '{id}'"),
            Self::MissingParent { id, parent } => {
                write!(f, "missing parent '{parent}' for '{id}'")
            }
            Self::Cycle { at } => write!(f, "cycle detected at '{at}'"),
        }
    }
}

impl std::error::Error for InheritError {}

/// Resolve copy-from inheritance. Output in topo-lexical order. Errors name the culprit.
pub fn resolve(defs: &[Def]) -> Result<Vec<Resolved>, InheritError> {
    let mut index: BTreeMap<&str, &Def> = BTreeMap::new();
    for d in defs {
        if d.id.is_empty() {
            return Err(InheritError::EmptyId);
        }
        if index.insert(d.id.as_str(), d).is_some() {
            return Err(InheritError::DuplicateId { id: d.id.clone() });
        }
    }
    for d in defs {
        if let Some(p) = &d.parent {
            if !index.contains_key(p.as_str()) {
                return Err(InheritError::MissingParent {
                    id: d.id.clone(),
                    parent: p.clone(),
                });
            }
        }
    }
    // Kahn topo with lexical tie-break.
    let mut indeg: BTreeMap<&str, usize> = BTreeMap::new();
    let mut kids: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for d in defs {
        indeg.entry(d.id.as_str()).or_insert(0);
        if let Some(p) = &d.parent {
            *indeg.entry(d.id.as_str()).or_insert(0) += 1;
            kids.entry(p.as_str()).or_default().push(d.id.as_str());
        }
    }
    let mut ready: BTreeSet<&str> = indeg
        .iter()
        .filter(|(_, &n)| n == 0)
        .map(|(&k, _)| k)
        .collect();
    let mut order: Vec<&str> = vec![];
    while let Some(&id) = ready.iter().next() {
        ready.remove(id);
        order.push(id);
        if let Some(ks) = kids.get(id) {
            for k in ks {
                let Some(e) = indeg.get_mut(k) else {
                    return Err(InheritError::Cycle {
                        at: (*k).to_string(),
                    });
                };
                *e -= 1;
                if *e == 0 {
                    ready.insert(k);
                }
            }
        }
    }
    if order.len() != index.len() {
        let mut rest: Vec<&str> = index
            .keys()
            .filter(|k| !order.contains(k))
            .copied()
            .collect();
        rest.sort_unstable();
        return Err(InheritError::Cycle {
            at: rest[0].to_string(),
        });
    }
    let mut maps: BTreeMap<&str, BTreeMap<String, String>> = BTreeMap::new();
    let mut out = vec![];
    for id in order {
        let d = index[id];
        let mut m: BTreeMap<String, String> = d
            .parent
            .as_ref()
            .map(|p| maps[p.as_str()].clone())
            .unwrap_or_default();
        for (k, v) in &d.fields {
            m.insert(k.clone(), v.clone());
        }
        maps.insert(id, m.clone());
        out.push(Resolved {
            id: id.to_string(),
            fields: m.into_iter().collect(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(id: &str, parent: Option<&str>, fields: &[(&str, &str)]) -> Def {
        Def {
            id: id.into(),
            parent: parent.map(|s| s.into()),
            fields: fields
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn child_inherits_plus_overrides() {
        let r = resolve(&[
            def("base", None, &[("a", "1"), ("b", "2")]),
            def("child", Some("base"), &[("b", "3"), ("c", "4")]),
        ])
        .unwrap();
        let c = r.iter().find(|x| x.id == "child").unwrap();
        assert_eq!(
            c.fields,
            vec![
                ("a".to_string(), "1".to_string()),
                ("b".to_string(), "3".to_string()),
                ("c".to_string(), "4".to_string())
            ]
        );
    }

    #[test]
    fn missing_parent_err() {
        let e = resolve(&[def("x", Some("nope"), &[])]).unwrap_err();
        assert_eq!(
            e,
            InheritError::MissingParent {
                id: "x".into(),
                parent: "nope".into()
            }
        );
        assert!(e.to_string().contains("missing parent 'nope'"), "{e}");
    }

    #[test]
    fn cycle_err() {
        let e = resolve(&[def("a", Some("b"), &[]), def("b", Some("a"), &[])]).unwrap_err();
        assert!(matches!(e, InheritError::Cycle { .. }), "{e}");
        assert!(e.to_string().contains("cycle detected"), "{e}");
        let e2 = resolve(&[def("a", Some("a"), &[])]).unwrap_err();
        assert!(e2.to_string().contains("cycle detected"), "{e2}");
    }

    #[test]
    fn empty_and_duplicate_ids_refuse() {
        assert_eq!(
            resolve(&[def("", None, &[])]).unwrap_err(),
            InheritError::EmptyId
        );
        assert_eq!(
            resolve(&[def("a", None, &[]), def("a", None, &[])]).unwrap_err(),
            InheritError::DuplicateId { id: "a".into() }
        );
    }

    #[test]
    fn order_deterministic_x2() {
        let a = vec![
            def("root", None, &[("v", "0")]),
            def("b", Some("root"), &[("v", "1")]),
            def("c", Some("root"), &[("v", "2")]),
            def("d", Some("b"), &[("v", "3")]),
            def("e", Some("c"), &[("v", "4")]),
        ];
        let mut b = a.clone();
        b.reverse();
        let (ra, rb) = (resolve(&a).unwrap(), resolve(&b).unwrap());
        assert_eq!(ra, rb);
        let pos = |id: &str| ra.iter().position(|x| x.id == id).unwrap();
        assert!(pos("root") < pos("b") && pos("root") < pos("c") && pos("b") < pos("d"));
    }
}
