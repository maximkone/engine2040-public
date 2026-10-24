//! error: the typed refusals of the IBL lane: missing HDRI, bad input, device fault.

use super::*;

// ---------------------------------------------------------------------------
// Errors: typed; missing HDRI refuses, bad input names the cause, device
// failures name the call or the validation scope.
// ---------------------------------------------------------------------------

/// IBL failures. `MissingHdri` is the cook-time refusal when no HDRI bytes
/// were supplied; decode problems wrap the R2 error they came from.
/// `Invalid` covers unusable caller input; `Device`/`Validation` (behind
/// `window`) cover device calls and the wgpu validation scope. No path
/// ever reports silent success.
#[derive(Clone, PartialEq, Debug)]
pub enum IblError {
    /// No HDRI bytes at all; cook with the Poly Haven fixture first.
    MissingHdri,
    /// The R2 HDRI decoder refused the bytes.
    #[cfg(feature = "v1")]
    Hdri(crate::v1_ibl_shadow::HdriError),
    /// A direction or roughness input failed R2 validation.
    #[cfg(feature = "v1")]
    Sample(crate::v1_ibl_shadow::SampleError),
    /// Caller input is unusable; reason carried.
    Invalid(String),
    /// A device or queue call failed (map, poll, texture write); message
    /// carried. Only behind `window`, where a live device is in hand.
    #[cfg(feature = "gpu")]
    Device(String),
    /// A wgpu validation error fired inside the submit scope; kept loud
    /// with the scope debug text. Only behind `window`.
    #[cfg(feature = "gpu")]
    Validation(String),
}

impl fmt::Display for IblError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingHdri => write!(f, "ibl needs HDRI bytes: none supplied"),
            #[cfg(feature = "v1")]
            Self::Hdri(e) => write!(f, "ibl hdri: {e}"),
            #[cfg(feature = "v1")]
            Self::Sample(e) => write!(f, "ibl sample: {e}"),
            Self::Invalid(e) => write!(f, "ibl input: {e}"),
            #[cfg(feature = "gpu")]
            Self::Device(e) => write!(f, "ibl device: {e}"),
            #[cfg(feature = "gpu")]
            Self::Validation(e) => write!(f, "ibl validation: {e}"),
        }
    }
}

impl std::error::Error for IblError {}

#[cfg(feature = "v1")]
impl From<crate::v1_ibl_shadow::HdriError> for IblError {
    fn from(e: crate::v1_ibl_shadow::HdriError) -> Self {
        Self::Hdri(e)
    }
}

#[cfg(feature = "v1")]
impl From<crate::v1_ibl_shadow::SampleError> for IblError {
    fn from(e: crate::v1_ibl_shadow::SampleError) -> Self {
        Self::Sample(e)
    }
}
