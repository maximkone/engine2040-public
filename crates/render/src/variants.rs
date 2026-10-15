//! variants: shader variant manifest with a hard count gate.
//!
//! Owner: render lane (RA7); consumers are the RA2 executor pipeline path
//! (`cache::PipelineKey` takes the sorted define set this manifest owns) and
//! the ED1/HR5 frame loop, which stages reloads at exact frame boundaries.
//! Units: counts (variants, defines) and frame indices (`u64`, never time).
//! Determinism: variants iterate in name order; define pairs sort by
//! (name, value), so identical inputs give identical keys on any host.
//! Errors: typed [`VariantError`]; over-cap names the refused variant and
//! off-frame swaps name the expected frame. Blocking work (compile, IO)
//! stays with the caller; this module only validates and stages.

use std::collections::BTreeMap;
use std::fmt;

// ---------------------------------------------------------------------------
// Bounds: every external size is validated before it is stored.
// ---------------------------------------------------------------------------

/// Hard cap on enumerated shader variants. A new variant past this cap fails
/// as [`VariantError::OverCap`] until a reviewer raises the cap: variant
/// explosion (one PBR flag doubling the set silently) must stay visible.
pub const MAX_VARIANTS: usize = 32;
/// Most static `name=value` defines folded into one variant.
pub const MAX_DEFINES_PER_VARIANT: usize = 8;
/// Most bytes in one variant name.
pub const MAX_VARIANT_NAME_BYTES: usize = 96;
/// Most bytes in one define name or value.
pub const MAX_DEFINE_BYTES: usize = 64;

// ---------------------------------------------------------------------------
// Error: every refusal names what to fix.
// ---------------------------------------------------------------------------

/// Typed variant-manifest failure. Refusals never mutate the manifest, and
/// off-frame swaps never touch render state: the caller retries at the
/// named frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VariantError {
    /// Variant name is empty.
    EmptyName,
    /// Variant name exceeds [`MAX_VARIANT_NAME_BYTES`]; length carried.
    NameTooLong {
        /// Offending byte count.
        len: usize,
    },
    /// A variant carries more than [`MAX_DEFINES_PER_VARIANT`] defines.
    TooManyDefines {
        /// Variant that carried them.
        variant: String,
        /// Offending define count.
        count: usize,
    },
    /// A define name is empty or not `ASCII [A-Za-z_][A-Za-z0-9_]*`.
    BadDefineName {
        /// Variant that carried it.
        variant: String,
        /// Offending name.
        name: String,
    },
    /// A define name or value exceeds [`MAX_DEFINE_BYTES`].
    DefineTooLong {
        /// Variant that carried it.
        variant: String,
        /// Offending name (or the name whose value overflowed).
        name: String,
    },
    /// A define value is empty or carries NUL, `;` or a line break.
    BadDefineValue {
        /// Variant that carried it.
        variant: String,
        /// Offending define name.
        name: String,
    },
    /// The manifest already lists this name.
    DuplicateVariant {
        /// Refused name.
        name: String,
    },
    /// The manifest is at its cap: review the new variant, then raise the
    /// cap deliberately. Names the variant that did not fit.
    OverCap {
        /// Refused variant name.
        variant: String,
        /// Cap that refused it.
        cap: usize,
    },
    /// No variant lists this name.
    UnknownVariant {
        /// Requested name.
        name: String,
    },
    /// A staged swap was applied off its frame boundary. Render state is
    /// untouched; retry at the expected frame.
    WrongFrame {
        /// Frame the swap was staged for.
        expected: u64,
        /// Frame the caller offered.
        got: u64,
    },
}

impl fmt::Display for VariantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName => write!(f, "variant name is empty"),
            Self::NameTooLong { len } => {
                write!(
                    f,
                    "variant name {len} bytes exceeds {MAX_VARIANT_NAME_BYTES}"
                )
            }
            Self::TooManyDefines { variant, count } => write!(
                f,
                "variant {variant} carries {count} defines (max {MAX_DEFINES_PER_VARIANT})"
            ),
            Self::BadDefineName { variant, name } => {
                write!(f, "variant {variant} has a bad define name {name:?}")
            }
            Self::DefineTooLong { variant, name } => write!(
                f,
                "variant {variant} define {name:?} exceeds {MAX_DEFINE_BYTES} bytes"
            ),
            Self::BadDefineValue { variant, name } => {
                write!(f, "variant {variant} define {name:?} has a bad value")
            }
            Self::DuplicateVariant { name } => {
                write!(f, "variant {name} is already in the manifest")
            }
            Self::OverCap { variant, cap } => write!(
                f,
                "variant {variant} refused: manifest at cap {cap} (review, then raise)"
            ),
            Self::UnknownVariant { name } => {
                write!(f, "unknown variant {name}")
            }
            Self::WrongFrame { expected, got } => write!(
                f,
                "staged swap is for frame {expected}, offered frame {got}"
            ),
        }
    }
}

impl std::error::Error for VariantError {}

// ---------------------------------------------------------------------------
// Variant + manifest: static defines enumerated under a count gate.
// ---------------------------------------------------------------------------

/// One shader variant: a name plus its sorted, deduplicated static
/// `name=value` defines. The define set feeds `cache::PipelineKey::new`
/// once the orchestrator wires this module into the crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderVariant {
    /// Manifest-unique name, e.g. `forward-pbr/shadows-ibl`.
    pub name: String,
    /// Sorted, deduplicated `(name, value)` static defines.
    pub defines: Vec<(String, String)>,
}

/// Enumerated static-define variants under a hard count cap. Variants
/// iterate in name order, so receipts and keys are stable across hosts.
#[derive(Debug, Clone, Default)]
pub struct VariantManifest {
    variants: BTreeMap<String, ShaderVariant>,
    cap: usize,
}

impl VariantManifest {
    /// Empty manifest capped at [`MAX_VARIANTS`].
    pub fn new() -> Self {
        Self::with_cap(MAX_VARIANTS)
    }

    /// Empty manifest with an explicit cap (reviewed raises go through
    /// this constructor; a zero cap refuses every variant).
    pub fn with_cap(cap: usize) -> Self {
        Self {
            variants: BTreeMap::new(),
            cap,
        }
    }

    /// The count gate: most variants this manifest will list.
    pub fn cap(&self) -> usize {
        self.cap
    }

    /// Listed variant count, in variants.
    pub fn len(&self) -> usize {
        self.variants.len()
    }

    /// True when no variant is listed.
    pub fn is_empty(&self) -> bool {
        self.variants.is_empty()
    }

    /// True when the manifest lists this name.
    pub fn contains(&self, name: &str) -> bool {
        self.variants.contains_key(name)
    }

    /// The listed variant under this name, if any.
    pub fn get(&self, name: &str) -> Option<&ShaderVariant> {
        self.variants.get(name)
    }

    /// Listed names in stable (lexicographic) order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.variants.keys().map(String::as_str)
    }

    /// List one variant. The define pairs sort here, so callers may pass
    /// any order and still enumerate the same variant. Over-cap and
    /// malformed inputs fail without touching the manifest.
    pub fn add_variant(
        &mut self,
        name: &str,
        defines: &[(&str, &str)],
    ) -> Result<(), VariantError> {
        if name.is_empty() {
            return Err(VariantError::EmptyName);
        }
        if name.len() > MAX_VARIANT_NAME_BYTES {
            return Err(VariantError::NameTooLong { len: name.len() });
        }
        if self.variants.contains_key(name) {
            return Err(VariantError::DuplicateVariant {
                name: name.to_string(),
            });
        }
        // The gate fires before define validation so the count receipt is
        // exact: at cap, every new name refuses as over-cap.
        if self.variants.len() >= self.cap {
            return Err(VariantError::OverCap {
                variant: name.to_string(),
                cap: self.cap,
            });
        }
        if defines.len() > MAX_DEFINES_PER_VARIANT {
            return Err(VariantError::TooManyDefines {
                variant: name.to_string(),
                count: defines.len(),
            });
        }
        let mut owned = Vec::with_capacity(defines.len());
        for (key, value) in defines {
            check_define(name, key, value)?;
            owned.push((key.to_string(), value.to_string()));
        }
        owned.sort();
        owned.dedup();
        self.variants.insert(
            name.to_string(),
            ShaderVariant {
                name: name.to_string(),
                defines: owned,
            },
        );
        Ok(())
    }

    /// Deterministic key fragment for one variant: `name(k=v,k=v)` with
    /// the stored (sorted) define order. Unknown names refuse loudly.
    pub fn key_string(&self, name: &str) -> Result<String, VariantError> {
        let variant = self
            .variants
            .get(name)
            .ok_or_else(|| VariantError::UnknownVariant {
                name: name.to_string(),
            })?;
        let mut key = String::with_capacity(name.len() + 2);
        key.push_str(name);
        key.push('(');
        for (i, (define, value)) in variant.defines.iter().enumerate() {
            if i > 0 {
                key.push(',');
            }
            key.push_str(define);
            key.push('=');
            key.push_str(value);
        }
        key.push(')');
        Ok(key)
    }

    /// The shipped forward-PBR set: the only production shader
    /// (`FORWARD_PBR_WGSL`) under its static feature flags. Four variants,
    /// far under the cap; new flags add rows here and hit the gate first.
    pub fn shipped_forward_pbr() -> Self {
        let mut manifest = Self::new();
        // Shipped rows are fixed by review; a failure here is a programming
        // bug, not user input.
        let rows: &[(&str, &[(&str, &str)])] = &[
            ("forward-pbr/base", &[]),
            ("forward-pbr/shadows", &[("SHADOWS", "1")]),
            ("forward-pbr/ibl", &[("IBL", "1")]),
            ("forward-pbr/shadows-ibl", &[("SHADOWS", "1"), ("IBL", "1")]),
        ];
        for (name, defines) in rows {
            manifest
                .add_variant(name, defines)
                .expect("shipped manifest fits its cap");
        }
        manifest
    }
}

/// Validate one define pair: ASCII names, bounded sizes, values with no
/// NUL/`;`/line-break (WGSL is line-oriented; `;` would smuggle a second
/// directive through a `-D` splice).
fn check_define(variant: &str, name: &str, value: &str) -> Result<(), VariantError> {
    let bad_name = || VariantError::BadDefineName {
        variant: variant.to_string(),
        name: name.to_string(),
    };
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err(bad_name());
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(bad_name());
    }
    if !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
        return Err(bad_name());
    }
    if name.len() > MAX_DEFINE_BYTES {
        return Err(VariantError::DefineTooLong {
            variant: variant.to_string(),
            name: name.to_string(),
        });
    }
    if value.len() > MAX_DEFINE_BYTES {
        return Err(VariantError::DefineTooLong {
            variant: variant.to_string(),
            name: name.to_string(),
        });
    }
    if value.is_empty()
        || value.contains('\0')
        || value.contains(';')
        || value.contains('\n')
        || value.contains('\r')
    {
        return Err(VariantError::BadDefineValue {
            variant: variant.to_string(),
            name: name.to_string(),
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Frame-exact staging: a variant swap bound to one frame index.
// ---------------------------------------------------------------------------

/// A variant swap staged for exactly one frame index. The render loop
/// applies it only when its counter equals [`StagedVariantSwap::frame`];
/// anything else refuses as [`VariantError::WrongFrame`] with both frames
/// named, leaving render state untouched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedVariantSwap {
    /// Variant to install at the boundary.
    pub variant: String,
    /// Frame index the swap lands on.
    pub frame: u64,
}

impl StagedVariantSwap {
    /// Stage a swap for one frame index. Empty variant names refuse here;
    /// unknown names refuse at apply time against the manifest.
    pub fn new(variant: &str, frame: u64) -> Result<Self, VariantError> {
        if variant.is_empty() {
            return Err(VariantError::EmptyName);
        }
        if variant.len() > MAX_VARIANT_NAME_BYTES {
            return Err(VariantError::NameTooLong { len: variant.len() });
        }
        Ok(Self {
            variant: variant.to_string(),
            frame,
        })
    }

    /// True only at the exact staged frame: no early apply, no late catch-up.
    pub fn is_due(&self, frame: u64) -> bool {
        self.frame == frame
    }

    /// Check the offered frame against the staged one. Unknown variants
    /// refuse against the manifest first so a typo never waits for a frame.
    pub fn check<'m>(
        &self,
        manifest: &'m VariantManifest,
        frame: u64,
    ) -> Result<&'m ShaderVariant, VariantError> {
        let variant = manifest
            .get(&self.variant)
            .ok_or_else(|| VariantError::UnknownVariant {
                name: self.variant.clone(),
            })?;
        if frame != self.frame {
            return Err(VariantError::WrongFrame {
                expected: self.frame,
                got: frame,
            });
        }
        Ok(variant)
    }
}

// ---------------------------------------------------------------------------
// Tests: counts, gates, keys and frame boundaries.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_manifest_posts_four_variants_under_cap() {
        let manifest = VariantManifest::shipped_forward_pbr();
        assert_eq!(manifest.len(), 4);
        assert!(manifest.len() < manifest.cap());
        assert_eq!(manifest.cap(), MAX_VARIANTS);
        assert!(manifest.contains("forward-pbr/base"));
        assert!(manifest.contains("forward-pbr/shadows-ibl"));
    }

    #[test]
    fn new_variant_past_cap_fails_loudly_naming_it() {
        let mut manifest = VariantManifest::with_cap(2);
        manifest.add_variant("a/base", &[]).expect("fits");
        manifest
            .add_variant("a/shadows", &[("SHADOWS", "1")])
            .expect("fits");
        let refused = manifest.add_variant("a/ibl", &[("IBL", "1")]);
        assert_eq!(
            refused,
            Err(VariantError::OverCap {
                variant: "a/ibl".to_string(),
                cap: 2,
            })
        );
        // The refusal stores nothing.
        assert_eq!(manifest.len(), 2);
        assert!(!manifest.contains("a/ibl"));
    }

    #[test]
    fn duplicate_and_malformed_variants_refuse() {
        let mut manifest = VariantManifest::new();
        manifest.add_variant("dup/base", &[]).expect("fits");
        assert_eq!(
            manifest.add_variant("dup/base", &[]),
            Err(VariantError::DuplicateVariant {
                name: "dup/base".to_string(),
            })
        );
        assert_eq!(manifest.add_variant("", &[]), Err(VariantError::EmptyName));
        assert!(matches!(
            manifest.add_variant("bad/define", &[("9LIVES", "1")]),
            Err(VariantError::BadDefineName { .. })
        ));
        assert!(matches!(
            manifest.add_variant("bad/value", &[("OK", "1;2")]),
            Err(VariantError::BadDefineValue { .. })
        ));
        let many: Vec<(&str, &str)> = vec![("D", "1"); MAX_DEFINES_PER_VARIANT + 1];
        assert!(matches!(
            manifest.add_variant("too/many", &many),
            Err(VariantError::TooManyDefines { .. })
        ));
        // Only the first row landed.
        assert_eq!(manifest.len(), 1);
    }

    #[test]
    fn key_string_is_stable_under_define_permutation() {
        let mut manifest = VariantManifest::new();
        manifest
            .add_variant("mix/full", &[("SHADOWS", "1"), ("IBL", "1")])
            .expect("fits");
        // Insert order never matters: pairs sort at insert.
        let mut permuted = VariantManifest::new();
        permuted
            .add_variant("mix/full", &[("IBL", "1"), ("SHADOWS", "1")])
            .expect("fits");
        assert_eq!(
            manifest.key_string("mix/full"),
            permuted.key_string("mix/full")
        );
        assert_eq!(
            manifest.key_string("mix/full").expect("listed"),
            "mix/full(IBL=1,SHADOWS=1)"
        );
        assert_eq!(
            manifest.key_string("ghost"),
            Err(VariantError::UnknownVariant {
                name: "ghost".to_string(),
            })
        );
    }

    #[test]
    fn staged_swap_lands_only_on_its_exact_frame() {
        let manifest = VariantManifest::shipped_forward_pbr();
        let staged = StagedVariantSwap::new("forward-pbr/ibl", 41).expect("stages");
        assert!(!staged.is_due(40));
        assert!(staged.is_due(41));
        // Early and late both refuse naming the expected frame.
        assert_eq!(
            staged.check(&manifest, 40),
            Err(VariantError::WrongFrame {
                expected: 41,
                got: 40,
            })
        );
        assert_eq!(
            staged.check(&manifest, 42),
            Err(VariantError::WrongFrame {
                expected: 41,
                got: 42,
            })
        );
        // The exact boundary resolves the variant.
        let landed = staged.check(&manifest, 41).expect("lands on frame 41");
        assert_eq!(landed.name, "forward-pbr/ibl");
        assert_eq!(landed.defines, vec![("IBL".to_string(), "1".to_string())]);
    }

    #[test]
    fn staged_swap_with_unknown_variant_refuses_before_frame_check() {
        let manifest = VariantManifest::shipped_forward_pbr();
        let staged = StagedVariantSwap::new("ghost/fx", 7).expect("stages");
        assert_eq!(
            staged.check(&manifest, 7),
            Err(VariantError::UnknownVariant {
                name: "ghost/fx".to_string(),
            })
        );
        assert_eq!(StagedVariantSwap::new("", 7), Err(VariantError::EmptyName));
    }
}
