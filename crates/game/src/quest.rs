//! quest: ladder as data-driven quest chain.
//! Consumer: modes read Quest beside ladder; fold advances QuestState.
//! Owner: this module owns chain plus cursors. Units: ids/cursors integers.
//! Determinism: integer ordered fold, same script same bytes, no clock.
//! Errors: QuestError at every boundary.
// PORT(sinbad/SUQS@c1a53c06c5ae1d53941ab323b4574bf9e28a87a5:Source/SUQS/Public/SuqsQuest.h,SuqsQuestState.h,SuqsObjectiveState.h,SuqsProgression.h): chain plus cursors plus bytes.
/// Payload version written by [`QuestState::to_bytes`].
pub const QUEST_VERSION: u8 = 1;
/// Exact state bytes: version (1) + objective (4) + task (4) + done (1).
pub const QUEST_BYTES: usize = 10;
/// Quest failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum QuestError {
    Empty,
    Duplicate(u32),
    UnknownRef(u32),
    OutOfOrder,
    BadSyntax,
    BadLength(usize),
    FutureVersion(u8),
}
impl core::fmt::Display for QuestError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => write!(f, "quest chain is empty"),
            Self::Duplicate(id) => write!(f, "duplicate quest id {id}"),
            Self::UnknownRef(id) => write!(f, "unknown prerequisite {id}"),
            Self::OutOfOrder => write!(f, "event out of order"),
            Self::BadSyntax => write!(f, "bad quest ron"),
            Self::BadLength(len) => write!(f, "quest state length {len}"),
            Self::FutureVersion(v) => write!(f, "quest state version {v} newer than v1"),
        }
    }
}
impl std::error::Error for QuestError {}
/// One task: a single rung win.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Task {
    pub id: u32,
}
/// One objective: ordered tasks gated by one prerequisite ref.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Objective {
    pub id: u32,
    pub requires: u32,
    pub tasks: Vec<Task>,
}
/// One quest: ordered objectives forming the ladder chain.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Quest {
    pub id: u32,
    pub objectives: Vec<Objective>,
}
/// One completion event: objective plus task ids just won.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct QuestEvent(pub u32, pub u32);
/// Runtime cursors into [`Quest`], separate from the definition.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct QuestState {
    pub objective: u32,
    pub task: u32,
    pub done: bool,
}
impl Quest {
    /// Validate: non-empty, unique ids, known prerequisite refs.
    pub fn validate(&self) -> Result<(), QuestError> {
        if self.objectives.is_empty() {
            return Err(QuestError::Empty);
        }
        let mut seen = Vec::new();
        for (i, a) in self.objectives.iter().enumerate() {
            if a.tasks.is_empty() {
                return Err(QuestError::Empty);
            }
            if seen.contains(&a.id) {
                return Err(QuestError::Duplicate(a.id));
            }
            seen.push(a.id);
            for t in &a.tasks {
                if seen.contains(&t.id) {
                    return Err(QuestError::Duplicate(t.id));
                }
                seen.push(t.id);
            }
            if a.requires != 0 && !self.objectives.iter().take(i).any(|o| o.id == a.requires) {
                return Err(QuestError::UnknownRef(a.requires));
            }
        }
        Ok(())
    }
}
/// Parse ladder RON: quest id plus one triple per objective.
pub fn parse_quest_ron(text: &str) -> Result<Quest, QuestError> {
    if !text.contains("objectives") {
        return Err(QuestError::BadSyntax);
    }
    let nums = numbers(text);
    if nums.len() < 4 || (nums.len() - 1) % 3 != 0 {
        return Err(QuestError::BadSyntax);
    }
    let mut objectives = Vec::new();
    let mut k = 1;
    while k + 2 < nums.len() {
        objectives.push(Objective {
            id: nums[k],
            requires: nums[k + 1],
            tasks: Vec::from([Task { id: nums[k + 2] }]),
        });
        k += 3;
    }
    if objectives.is_empty() {
        return Err(QuestError::Empty);
    }
    let quest = Quest {
        id: nums[0],
        objectives,
    };
    quest.validate()?;
    Ok(quest)
}
fn numbers(text: &str) -> Vec<u32> {
    text.split(|ch: char| !ch.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect()
}
impl QuestState {
    /// Fold a script in order; a skip or a done quest fails. No allocation.
    pub fn apply_event(&mut self, quest: &Quest, events: &[QuestEvent]) -> Result<(), QuestError> {
        for event in events {
            if self.done {
                return Err(QuestError::OutOfOrder);
            }
            let oi = usize::try_from(self.objective).unwrap_or(usize::MAX);
            let ti = usize::try_from(self.task).unwrap_or(usize::MAX);
            let Some(obj) = quest.objectives.get(oi) else {
                return Err(QuestError::OutOfOrder);
            };
            let Some(task) = obj.tasks.get(ti) else {
                return Err(QuestError::OutOfOrder);
            };
            if event.0 != obj.id || event.1 != task.id {
                return Err(QuestError::OutOfOrder);
            }
            let tasks = u32::try_from(obj.tasks.len()).unwrap_or(u32::MAX);
            let objs = u32::try_from(quest.objectives.len()).unwrap_or(u32::MAX);
            if self.task + 1 < tasks {
                self.task += 1;
            } else if self.objective + 1 < objs {
                self.objective += 1;
                self.task = 0;
            } else {
                self.done = true;
            }
        }
        Ok(())
    }
    /// Fixed little-endian encode beside the ladder seam. Pure.
    #[must_use]
    pub fn to_bytes(self) -> [u8; QUEST_BYTES] {
        let mut out = [0u8; QUEST_BYTES];
        out[0] = QUEST_VERSION;
        out[1..5].copy_from_slice(&self.objective.to_le_bytes());
        out[5..9].copy_from_slice(&self.task.to_le_bytes());
        out[9] = u8::from(self.done);
        out
    }
    /// Decode exactly [`QUEST_BYTES`] at version [`QUEST_VERSION`]. Pure.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, QuestError> {
        if bytes.len() != QUEST_BYTES {
            return Err(QuestError::BadLength(bytes.len()));
        }
        if bytes[0] != QUEST_VERSION {
            return Err(QuestError::FutureVersion(bytes[0]));
        }
        let mut quad = [0u8; 4];
        quad.copy_from_slice(&bytes[1..5]);
        let objective = u32::from_le_bytes(quad);
        quad.copy_from_slice(&bytes[5..9]);
        let task = u32::from_le_bytes(quad);
        Ok(Self {
            objective,
            task,
            done: bytes[9] != 0,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    const GOOD: &str = "(id: 7, objectives: [(id: 10, requires: 0, tasks: [100]), (id: 11, requires: 10, tasks: [101]), (id: 12, requires: 11, tasks: [102])])";
    fn script() -> Vec<QuestEvent> {
        Vec::from([
            QuestEvent(10, 100),
            QuestEvent(11, 101),
            QuestEvent(12, 102),
        ])
    }
    #[test]
    fn quest_chain_validates() {
        let quest = parse_quest_ron(GOOD).expect("good ron loads");
        assert_eq!(quest.objectives.len(), 3);
        let unknown = GOOD.replace("requires: 10", "requires: 99");
        assert_eq!(parse_quest_ron(&unknown), Err(QuestError::UnknownRef(99)));
        let bare = Quest {
            id: 7,
            objectives: Vec::new(),
        };
        assert_eq!(bare.validate(), Err(QuestError::Empty));
        let dup = GOOD.replace("[101]", "[100]");
        assert_eq!(parse_quest_ron(&dup), Err(QuestError::Duplicate(100)));
    }
    #[test]
    fn objectives_complete_in_order() {
        let quest = parse_quest_ron(GOOD).expect("good ron loads");
        let mut state = QuestState::default();
        let skip = QuestEvent(11, 101);
        assert_eq!(
            state.apply_event(&quest, &[skip]),
            Err(QuestError::OutOfOrder)
        );
        assert_eq!((state.objective, state.task), (0, 0));
        assert!(state.apply_event(&quest, &[QuestEvent(10, 100)]).is_ok());
        assert_eq!((state.objective, state.task, state.done), (1, 0, false));
        assert!(state.apply_event(&quest, &script()[1..]).is_ok());
        assert!(state.done);
    }
    #[test]
    fn quest_replay_is_bit_exact() {
        let quest = parse_quest_ron(GOOD).expect("good ron loads");
        let events = script();
        let mut first = QuestState::default();
        first
            .apply_event(&quest, &events)
            .expect("in-order completes");
        let mut second = QuestState::default();
        second
            .apply_event(&quest, &events)
            .expect("replay completes");
        assert_eq!(first, second);
        assert_eq!(first.to_bytes(), second.to_bytes());
        assert_eq!(
            QuestState::from_bytes(&first.to_bytes()).expect("roundtrip"),
            first
        );
    }
}
