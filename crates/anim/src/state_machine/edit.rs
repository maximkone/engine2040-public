//! Undoable edits over a [`StateMachineDef`]: every edit returns its exact
//! inverse, a refused edit changes nothing, and the document keeps the last
//! good machine when a reload fails.

use super::{ParamDef, StateDef, StateMachineDef, StateMachineError, TransitionDef};

/// One edit; indices are list positions, states and parameters keep their names.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Edit {
    /// Insert a state at `index` (`index == len` appends).
    InsertState { index: usize, def: StateDef },
    /// Remove the state at `index`; refused while a transition or the start uses it.
    RemoveState { index: usize },
    /// Replace the state at `index`, same name only.
    ReplaceState { index: usize, def: StateDef },
    /// Rename the state at `index`; transitions and the start follow.
    RenameState { index: usize, to: String },
    /// Insert a transition at `index`.
    InsertTransition { index: usize, def: TransitionDef },
    /// Remove the transition at `index`.
    RemoveTransition { index: usize },
    /// Replace the transition at `index`.
    ReplaceTransition { index: usize, def: TransitionDef },
    /// Insert a parameter at `index`.
    InsertParam { index: usize, def: ParamDef },
    /// Remove the parameter at `index`; refused while a condition reads it.
    RemoveParam { index: usize },
    /// Replace the parameter at `index`, same name only.
    ReplaceParam { index: usize, def: ParamDef },
    /// Choose the starting state.
    SetStart { name: String },
    /// Replace the whole machine (a graph load).
    ReplaceAll { def: Box<StateMachineDef> },
}

fn at<T>(list: &[T], what: &'static str, index: usize) -> Result<(), StateMachineError> {
    if index < list.len() {
        Ok(())
    } else {
        Err(StateMachineError::IndexOutOfRange { what, index })
    }
}

fn insert_at<T>(
    list: &mut Vec<T>,
    what: &'static str,
    index: usize,
    item: T,
) -> Result<(), StateMachineError> {
    if index > list.len() {
        return Err(StateMachineError::IndexOutOfRange { what, index });
    }
    list.insert(index, item);
    Ok(())
}

fn cond_reads(cond: &super::Cond, name: &str) -> bool {
    use super::CondOf;
    match cond {
        CondOf::Param { param, .. } => param == name,
        CondOf::Not(inner) => cond_reads(inner, name),
        CondOf::All(kids) | CondOf::Any(kids) => kids.iter().any(|k| cond_reads(k, name)),
        CondOf::Always | CondOf::TimeAtLeast(_) => false,
    }
}

impl StateMachineDef {
    /// Apply `edit` if the result validates; return the inverse edit.
    /// On any error the machine is exactly as before.
    pub fn apply(&mut self, edit: Edit) -> Result<Edit, StateMachineError> {
        let mut next = self.clone();
        let inverse = next.apply_unchecked(edit)?;
        next.validate()?;
        *self = next;
        Ok(inverse)
    }

    fn apply_unchecked(&mut self, edit: Edit) -> Result<Edit, StateMachineError> {
        match edit {
            Edit::InsertState { index, def } => self.insert_state(index, def),
            Edit::RemoveState { index } => self.remove_state(index),
            Edit::ReplaceState { index, def } => self.replace_state(index, def),
            Edit::RenameState { index, to } => self.rename_state(index, to),
            Edit::InsertTransition { index, def } => self.insert_transition(index, def),
            Edit::RemoveTransition { index } => self.remove_transition(index),
            Edit::ReplaceTransition { index, def } => self.replace_transition(index, def),
            Edit::InsertParam { index, def } => self.insert_param(index, def),
            Edit::RemoveParam { index } => self.remove_param(index),
            Edit::ReplaceParam { index, def } => self.replace_param(index, def),
            Edit::SetStart { name } => {
                let old = std::mem::replace(&mut self.start, name);
                Ok(Edit::SetStart { name: old })
            }
            Edit::ReplaceAll { def } => {
                let old = std::mem::replace(self, *def);
                Ok(Edit::ReplaceAll { def: Box::new(old) })
            }
        }
    }

    fn insert_state(&mut self, index: usize, def: StateDef) -> Result<Edit, StateMachineError> {
        insert_at(&mut self.states, "state", index, def)?;
        Ok(Edit::RemoveState { index })
    }

    fn remove_state(&mut self, index: usize) -> Result<Edit, StateMachineError> {
        at(&self.states, "state", index)?;
        let name = self.states[index].name.clone();
        let used = self.start == name
            || self
                .transitions
                .iter()
                .any(|t| t.from == name || t.to == name);
        if used {
            return Err(StateMachineError::InUse {
                what: "state",
                name,
            });
        }
        Ok(Edit::InsertState {
            index,
            def: self.states.remove(index),
        })
    }

    fn replace_state(&mut self, index: usize, def: StateDef) -> Result<Edit, StateMachineError> {
        at(&self.states, "state", index)?;
        if self.states[index].name != def.name {
            return Err(StateMachineError::RenameViaReplace(def.name));
        }
        let old = std::mem::replace(&mut self.states[index], def);
        Ok(Edit::ReplaceState { index, def: old })
    }

    fn rename_state(&mut self, index: usize, to: String) -> Result<Edit, StateMachineError> {
        at(&self.states, "state", index)?;
        let old = std::mem::replace(&mut self.states[index].name, to.clone());
        for t in &mut self.transitions {
            if t.from == old {
                t.from.clone_from(&to);
            }
            if t.to == old {
                t.to.clone_from(&to);
            }
        }
        if self.start == old {
            self.start = to;
        }
        Ok(Edit::RenameState { index, to: old })
    }

    fn insert_transition(
        &mut self,
        index: usize,
        def: TransitionDef,
    ) -> Result<Edit, StateMachineError> {
        insert_at(&mut self.transitions, "transition", index, def)?;
        Ok(Edit::RemoveTransition { index })
    }

    fn remove_transition(&mut self, index: usize) -> Result<Edit, StateMachineError> {
        at(&self.transitions, "transition", index)?;
        Ok(Edit::InsertTransition {
            index,
            def: self.transitions.remove(index),
        })
    }

    fn replace_transition(
        &mut self,
        index: usize,
        def: TransitionDef,
    ) -> Result<Edit, StateMachineError> {
        at(&self.transitions, "transition", index)?;
        let old = std::mem::replace(&mut self.transitions[index], def);
        Ok(Edit::ReplaceTransition { index, def: old })
    }

    fn insert_param(&mut self, index: usize, def: ParamDef) -> Result<Edit, StateMachineError> {
        insert_at(&mut self.params, "parameter", index, def)?;
        Ok(Edit::RemoveParam { index })
    }

    fn remove_param(&mut self, index: usize) -> Result<Edit, StateMachineError> {
        at(&self.params, "parameter", index)?;
        let name = self.params[index].name.clone();
        if self
            .transitions
            .iter()
            .any(|t| cond_reads(&t.condition, &name))
        {
            return Err(StateMachineError::InUse {
                what: "parameter",
                name,
            });
        }
        Ok(Edit::InsertParam {
            index,
            def: self.params.remove(index),
        })
    }

    fn replace_param(&mut self, index: usize, def: ParamDef) -> Result<Edit, StateMachineError> {
        at(&self.params, "parameter", index)?;
        if self.params[index].name != def.name {
            return Err(StateMachineError::RenameViaReplace(def.name));
        }
        let old = std::mem::replace(&mut self.params[index], def);
        Ok(Edit::ReplaceParam { index, def: old })
    }
}

/// A machine with undo and redo, and the last good version kept on failure.
#[derive(Clone, Debug)]
pub struct Document {
    def: StateMachineDef,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
}

impl Document {
    /// Start from a validated machine.
    pub fn new(def: StateMachineDef) -> Result<Self, StateMachineError> {
        def.validate()?;
        Ok(Self {
            def,
            undo: Vec::new(),
            redo: Vec::new(),
        })
    }

    /// The current good machine.
    pub fn def(&self) -> &StateMachineDef {
        &self.def
    }

    /// Edits that can be undone.
    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    /// Apply an edit; a new edit clears redo. A refused edit leaves everything unchanged.
    pub fn apply(&mut self, edit: Edit) -> Result<(), StateMachineError> {
        let inverse = self.def.apply(edit)?;
        self.undo.push(inverse);
        self.redo.clear();
        Ok(())
    }

    /// Replace the whole machine if it validates (the reload path): on failure
    /// the last good machine stays and the error comes back.
    pub fn replace(&mut self, def: StateMachineDef) -> Result<(), StateMachineError> {
        self.apply(Edit::ReplaceAll { def: Box::new(def) })
    }

    /// Undo the last edit exactly.
    pub fn undo(&mut self) -> Result<(), StateMachineError> {
        let edit = self.undo.pop().ok_or(StateMachineError::NothingToUndo)?;
        match self.def.apply(edit.clone()) {
            Ok(inverse) => {
                self.redo.push(inverse);
                Ok(())
            }
            Err(error) => {
                self.undo.push(edit);
                Err(error)
            }
        }
    }

    /// Redo the last undone edit.
    pub fn redo(&mut self) -> Result<(), StateMachineError> {
        let edit = self.redo.pop().ok_or(StateMachineError::NothingToUndo)?;
        match self.def.apply(edit.clone()) {
            Ok(inverse) => {
                self.undo.push(inverse);
                Ok(())
            }
            Err(error) => {
                self.redo.push(edit);
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Source;
    use super::*;

    fn solo_machine() -> StateMachineDef {
        let mut def = StateMachineDef::new("solo", "solo");
        def.states.push(StateDef {
            name: "solo".into(),
            source: Source::Clip(0),
            length_ticks: 20,
            looping: true,
        });
        def
    }

    /// The start state is in use even when no transition names it: removing
    /// it is refused and the document is unchanged. Callers (authors through
    /// `Document`) see `InUse`, never a machine with a dangling start.
    #[test]
    fn removing_the_start_state_is_refused_with_no_transitions() {
        let mut doc = Document::new(solo_machine()).expect("solo validates");
        let before = doc.def().digest();
        assert!(matches!(
            doc.apply(Edit::RemoveState { index: 0 }),
            Err(StateMachineError::InUse { what: "state", .. })
        ));
        assert_eq!(doc.def().digest(), before, "refused edit changes nothing");
        assert_eq!(doc.undo_depth(), 0);
        assert_eq!(doc.def().start, "solo");
    }
}
