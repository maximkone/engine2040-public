//! max_distance: the per-particle Max Distance a sheet may hang from its rest position.
//!
//! One limit per particle, so a painted gradient is data and not a second solver. Zero means
//! the tether reach applies and no extra limit is set, which keeps a plain cape on its PX35 bar.

use super::{dot, sub, Cloth, ClothError, MAX_SIZE};

impl Cloth {
    /// Give every particle its own Max Distance, in meters from its rest position: one value
    /// per particle, so a painted gradient is data and not a second solver. Zero means the
    /// tether reach applies and no extra limit is set.
    ///
    /// # Errors
    /// [`ClothError::BadParam`] when the count is wrong or a value is not a finite number in
    /// `0..=MAX_SIZE`.
    pub fn set_max_distance_gradient(&mut self, values: &[f32]) -> Result<(), ClothError> {
        if values.len() != self.pos.len() {
            return Err(ClothError::BadParam("max distance"));
        }
        for &v in values {
            if !v.is_finite() || !(0.0..=MAX_SIZE).contains(&v) {
                return Err(ClothError::BadParam("max distance"));
            }
        }
        self.max_distance.copy_from_slice(values);
        self.wake();
        Ok(())
    }

    /// The Max Distance of one particle, meters; zero when none is set.
    pub fn max_distance_of(&self, particle: usize) -> f32 {
        self.max_distance.get(particle).copied().unwrap_or(0.0)
    }

    /// Hold every particle inside its Max Distance: the distance from its rest position. The
    /// limit is per particle, so a gradient across the sheet is one data row and not a second
    /// solver.
    pub(in crate::cloth) fn solve_max_distance(&mut self) {
        for i in 0..self.pos.len() {
            let limit = self.max_distance[i];
            if limit <= 0.0 || self.pinned[i] {
                continue;
            }
            let rest = self.rest[i];
            let away = sub(self.pos[i], rest);
            let d2 = dot(away, away);
            if d2 <= limit * limit {
                continue;
            }
            let k = limit / d2.sqrt();
            self.pos[i] = [
                rest[0] + away[0] * k,
                rest[1] + away[1] * k,
                rest[2] + away[2] * k,
            ];
        }
    }
}
