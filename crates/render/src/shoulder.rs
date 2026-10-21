//! shoulder: the shoulder camera's geometry, shared by the renderer and the
//! headless crates that place a fight inside a room.
//!
//! Owner: engine-render. Consumer: the v1 `procedural_arena` renderer builds
//! its eye from these numbers; the game's page duel reads them to prove the
//! eye still has floor behind the seat it follows. Units: meters. Determinism
//! scope: constants only, no state and no clock. Errors: none, the module
//! cannot fail. Budget: compile-time only.

/// Third-person camera: how far behind the human seat the eye sits, meters. Far enough that the
/// seat's own body reads as a silhouette in the lower frame instead of filling it.
pub const SHOULDER_BACK_M: f32 = 5.2;
/// Sideways offset of the eye (over the human's right shoulder), meters. The seats stand close
/// (about a meter apart), so most of the offset is what keeps the human from hiding the rival:
/// the shot is three-quarter, not straight down the pair's own line.
pub const SHOULDER_SIDE_M: f32 = 2.4;
/// Height of the eye, meters.
pub const SHOULDER_EYE_M: f32 = 2.3;
/// Seat gap above which the camera pulls back and widens so both stay framed, meters.
pub const SHOULDER_GAP_M: f32 = 10.0;
/// Largest shake the camera takes at full hit weight, meters.
pub const SHOULDER_SHAKE_M: f32 = 0.085;
/// Nearest the shoulder camera ever gets to what it looks at, meters: a seat that turns its
/// back on the rival would otherwise put the eye past the rival, looking back at it.
pub const SHOULDER_MIN_SHOT_M: f32 = 4.0;
/// How far above the midpoint of the two seats the eye looks, meters: chest height, so both
/// seats sit in frame with the floor and the wall behind them.
pub const SHOULDER_LOOK_UP_M: f32 = 1.15;
