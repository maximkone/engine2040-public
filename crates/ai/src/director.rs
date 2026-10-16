//! director: pacing as data. Tables + tension scalar, never hardcoded spawns.

/// Intensity budget: accumulates pressure, spends on spawns.
#[derive(Clone, Debug)]
pub struct Budget {
    pub intensity: f32,
    pub cap: f32,
}

impl Budget {
    pub fn new(cap: f32) -> Self {
        Self {
            intensity: 0.0,
            cap: cap.max(1.0),
        }
    }
    /// Add pressure (clamped to cap). Pure, deterministic.
    pub fn add(&mut self, x: f32) {
        self.intensity = (self.intensity + x.max(0.0)).min(self.cap);
    }
    /// Spend if affordable. Returns true on success.
    pub fn spend(&mut self, cost: f32) -> bool {
        if cost <= self.intensity {
            self.intensity -= cost;
            true
        } else {
            false
        }
    }
    /// Tension scalar 0..1 = intensity / cap. Never hardcoded spawns.
    pub fn tension(&self) -> f32 {
        (self.intensity / self.cap).clamp(0.0, 1.0)
    }
}

/// Spawn table row: min tension gate + cost + label. Data, not code.
#[derive(Clone, Debug)]
pub struct SpawnRow {
    pub at_tension: f32,
    pub cost: f32,
    pub label: String,
}

/// Situation-placed spawn: first affordable row whose gate <= tension, in table order.
pub fn pick_spawn(budget: &mut Budget, table: &[SpawnRow]) -> Option<String> {
    let t = budget.tension();
    for row in table {
        if t >= row.at_tension && budget.spend(row.cost) {
            return Some(row.label.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_clamps_and_tension() {
        let mut b = Budget::new(100.0);
        b.add(40.0);
        assert_eq!(b.tension(), 0.4);
        b.add(999.0);
        assert_eq!(b.tension(), 1.0);
        assert!(b.spend(60.0));
        assert!(!b.spend(60.0));
    }

    #[test]
    fn spawn_tables_not_hardcode() {
        let table = vec![
            SpawnRow {
                at_tension: 0.8,
                cost: 60.0,
                label: "horde".into(),
            },
            SpawnRow {
                at_tension: 0.3,
                cost: 20.0,
                label: "scout".into(),
            },
        ];
        let mut b = Budget::new(100.0);
        b.add(35.0);
        assert_eq!(pick_spawn(&mut b, &table), Some("scout".into()));
        assert_eq!(pick_spawn(&mut b, &table), None); // spent, tension dropped
        b.add(90.0);
        assert_eq!(pick_spawn(&mut b, &table), Some("horde".into()));
    }

    #[test]
    fn determinism_x2() {
        let run = || {
            let mut b = Budget::new(100.0);
            b.add(50.0);
            (b.tension(), b.spend(20.0), b.tension())
        };
        assert_eq!(format!("{:?}", run()), format!("{:?}", run()));
    }
}
