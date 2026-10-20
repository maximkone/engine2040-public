//! The one sorted-and-unique gate for net's validated key runs: input tapes,
//! interest rosters, snapshot slots and delta rows. Each caller keeps its own
//! size and range rules and maps one failing key to its own error, so the
//! order rule lives once instead of one copy per site.

/// Keys must strictly ascend, which makes a run sorted and unique in one
/// forward pass. Feed keys in wire order; `push` answers false on the first
/// repeat or backward step.
#[derive(Debug, Default)]
pub(crate) struct SortedUnique<K> {
    prev: Option<K>,
}

impl<K: Ord> SortedUnique<K> {
    pub(crate) const fn new() -> Self {
        Self { prev: None }
    }

    /// Record `key`; false when it does not strictly exceed the key before.
    pub(crate) fn push(&mut self, key: K) -> bool {
        let ascending = self.prev.as_ref().is_none_or(|prev| *prev < key);
        self.prev = Some(key);
        ascending
    }
}

#[cfg(test)]
mod tests {
    use super::SortedUnique;

    #[test]
    fn gate_takes_strict_ascending() {
        let mut gate = SortedUnique::new();
        assert!(gate.push(3u8));
        assert!(gate.push(5));
        assert!(gate.push(9));
    }

    #[test]
    fn gate_refuses_a_repeat() {
        let mut gate = SortedUnique::new();
        assert!(gate.push(4u32));
        assert!(!gate.push(4));
    }

    #[test]
    fn gate_refuses_a_backward_step_on_a_tuple_key() {
        let mut gate = SortedUnique::new();
        assert!(gate.push((7u32, 1u8)));
        assert!(!gate.push((7, 0)));
    }

    #[test]
    fn gate_takes_an_empty_run() {
        let gate: SortedUnique<u8> = SortedUnique::new();
        assert_eq!(gate.prev, None);
    }
}
