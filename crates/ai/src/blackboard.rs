//! blackboard: BT/GOAP shared state. Bake planners output tables; runtime reads ints.

use engine_core::Fnv64;

/// Task status (BT Running resumes next tick, never blocks).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskStatus {
    /// Node succeeded.
    Success,
    /// Node failed.
    Failure,
    /// Node needs more ticks (resume with same blackboard).
    Running,
}

/// Sorted key-value store (string keys, u64 values). Iteration is insertion
/// order for determinism; lookups are linear (NPC counts are tiny).
pub struct Blackboard {
    keys: Vec<String>,
    vals: Vec<u64>,
}

impl Blackboard {
    /// Construct empty.
    pub fn new() -> Self {
        Self {
            keys: vec![],
            vals: vec![],
        }
    }
    /// Set a key (insert or overwrite, never reorders).
    pub fn set(&mut self, key: &str, val: u64) {
        if let Some(i) = self.keys.iter().position(|k| k == key) {
            self.vals[i] = val;
        } else {
            self.keys.push(key.to_string());
            self.vals.push(val);
        }
    }
    /// Get a key (None when absent, never panics).
    pub fn get(&self, key: &str) -> Option<u64> {
        self.keys
            .iter()
            .position(|k| k == key)
            .map(|i| self.vals[i])
    }
    /// FNV-1a64 over keys+vals (npc_trace gate input).
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv64::new();
        for (k, v) in self.keys.iter().zip(self.vals.iter()) {
            h.write_str(k);
            h.write_u64(*v);
        }
        h.finish()
    }
}

impl Default for Blackboard {
    fn default() -> Self {
        Self::new()
    }
}

/// GOAP-lite: pick up to `max` action indices ordered by (cost, index).
/// Cap is 256 (dogoap pattern). Deterministic stable order.
pub fn goap_plan(costs: &[u32], max: usize) -> Vec<usize> {
    let cap = max.min(256).min(costs.len());
    let mut idx: Vec<usize> = (0..costs.len()).collect();
    idx.sort_by(|&a, &b| costs[a].cmp(&costs[b]).then(a.cmp(&b)));
    idx.truncate(cap);
    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_overwrite() {
        let mut b = Blackboard::new();
        assert_eq!(b.get("fear"), None);
        b.set("fear", 7);
        b.set("trust", 3);
        b.set("fear", 9);
        assert_eq!((b.get("fear"), b.get("trust")), (Some(9), Some(3)));
        assert_eq!(b.state_hash(), b.state_hash());
    }

    #[test]
    fn goap_orders_by_cost_then_idx() {
        assert_eq!(goap_plan(&[5, 1, 1, 9], 10), vec![1, 2, 0, 3]);
        assert_eq!(goap_plan(&[5, 1, 1, 9], 2), vec![1, 2]);
        assert!(goap_plan(&[1; 300], 999).len() <= 256);
    }
}
