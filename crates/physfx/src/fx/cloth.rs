//! cloth: the cloth entry of the fx layer. A cape or a banner hangs in its own soft-body
//! world, stepped by [`PhysFx::step`] at the same fixed 60 Hz as the ragdolls.
//!
//! A cloth is not a body in the fx slab: it has no collider, no hash and no share of the body
//! cap. It costs one slot of [`MAX_CLOTHS`] and is drawn from [`SoftCloth::particles`].

use super::PhysFx;
use crate::soft::{ClothDesc, ClothError, SoftCloth, SoftReceipt};

impl PhysFx {
    /// Hangs a cloth in the world and returns its stable slot. Refused past
    /// [`MAX_CLOTHS`](crate::MAX_CLOTHS) or with a bad desc; a refused spawn leaves the world
    /// unchanged. Cloths are never recycled.
    pub fn spawn_cloth(&mut self, desc: ClothDesc) -> Result<usize, ClothError> {
        self.soft.spawn(desc)
    }

    /// Every cloth, oldest first.
    #[must_use]
    pub fn cloths(&self) -> &[SoftCloth] {
        self.soft.cloths()
    }

    /// The cloth in `slot`, or `None` when the world holds no such cloth.
    #[must_use]
    pub fn cloth(&self, slot: usize) -> Option<&SoftCloth> {
        self.soft.cloth(slot)
    }

    /// The receipt of the cloth in `slot` at the last step, or `None`.
    #[must_use]
    pub fn cloth_receipt(&self, slot: usize) -> Option<SoftReceipt> {
        self.soft.cloth(slot).map(SoftCloth::receipt)
    }
}
