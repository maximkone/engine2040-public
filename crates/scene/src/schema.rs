//! Versioned component schemas plus per-version migrations.
//!
//! Consumer: [`crate::prefab`] validates and migrates every component
//! through [`SchemaRegistry`]; game templates author components and boot new
//! games on them. Units: components (named payloads) and versions
//! (`u32`, 1-based). Determinism: migration is pure and registry order is a
//! `BTreeMap`, so the same input always yields byte-identical output.
//! Errors: [`SchemaError`]; unknown versions refuse loudly, failed
//! migrations name their cause.

use std::collections::BTreeMap;
use std::fmt;

/// Longest accepted component name in bytes.
pub const MAX_COMPONENT_NAME_LEN: usize = 64;
/// Largest accepted component payload in bytes.
pub const MAX_PAYLOAD_LEN: usize = 4096;
/// Longest migration-failure cause kept in an error.
pub const MAX_CAUSE_LEN: usize = 256;

/// One versioned component value: a named payload at an explicit version.
///
/// The payload is opaque canonical text to this module (authored JSON in
/// practice); the registry gates the version and runs the migration chain
/// but never parses the payload itself.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VersionedComponent {
    /// Component name, e.g. `"label"`.
    pub component: String,
    /// Version the payload was written at (1-based).
    pub version: u32,
    /// Canonical payload text at [`VersionedComponent::version`].
    pub payload: String,
}

/// Migration for one version step: payload at `from` becomes payload at
/// `from + 1`. `Err` carries a [`SchemaError`] (usually
/// [`SchemaError::MigrationFailed`); [`SchemaRegistry::migrate_value`]
/// re-wraps it with the step's component and versions and truncates the
/// cause to [`MAX_CAUSE_LEN`] bytes.
pub type MigrateFn = fn(&str) -> Result<String, SchemaError>;

/// Versioned component descriptor: one migration step of one component.
///
/// A component at version `V` below current carries `{ version: V,
/// migrate: Some(f) }`; the current version carries `migrate: None`. The
/// registry chains the steps forward; an incomplete chain refuses loudly
/// instead of guessing.
#[derive(Clone, Copy, Debug)]
pub struct Schema {
    /// Component name, e.g. `"label"`.
    pub component: &'static str,
    /// Version this descriptor migrates from (1-based).
    pub version: u32,
    /// Migration to `version + 1`; `None` only on the current version.
    pub migrate: Option<MigrateFn>,
}

/// What went wrong validating or migrating a component.
///
/// Every variant names the component (and versions where they apply) so
/// editors and template authors can quote the refusal in diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchemaError {
    /// A well-formed version this registry cannot serve: an unknown
    /// component, or a version above current. Never guess: update the
    /// reader instead of loading.
    UnknownVersion {
        /// Component named by the value.
        component: String,
        /// Version stamped on the value.
        got: u32,
        /// Newest version this registry serves (0 for unknown components).
        current: u32,
    },
    /// A registered migration step failed, or the chain has a gap.
    MigrationFailed {
        /// Component being migrated.
        component: String,
        /// Version migrated from.
        from: u32,
        /// Version migrated to.
        to: u32,
        /// Why, truncated to [`MAX_CAUSE_LEN`] bytes.
        cause: String,
    },
    /// A descriptor was malformed (empty or bad name, version 0, or a
    /// duplicate `(component, version)` pair).
    BadDescriptor {
        /// Static reason; the offending descriptor is never echoed raw.
        reason: &'static str,
    },
    /// A component value was malformed (empty or bad name, version 0, or a
    /// payload over [`MAX_PAYLOAD_LEN`]).
    BadComponent {
        /// Static reason; the offending value is never echoed raw.
        reason: &'static str,
    },
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownVersion {
                component,
                got,
                current,
            } => write!(
                f,
                "unknown {component} version v{got}; this registry serves v{current} \
                 (update the reader instead of loading)"
            ),
            Self::MigrationFailed {
                component,
                from,
                to,
                cause,
            } => write!(f, "{component} migration v{from} to v{to} failed: {cause}"),
            Self::BadDescriptor { reason } => write!(f, "invalid schema descriptor: {reason}"),
            Self::BadComponent { reason } => write!(f, "invalid component value: {reason}"),
        }
    }
}

impl std::error::Error for SchemaError {}

/// Registry of versioned component descriptors plus the migration runner.
///
/// Steps iterate by component name then version (`BTreeMap`), so migration
/// output never depends on registration order.
#[derive(Clone, Debug, Default)]
pub struct SchemaRegistry {
    steps: BTreeMap<&'static str, BTreeMap<u32, Option<MigrateFn>>>,
}

impl SchemaRegistry {
    /// Empty registry; serves nothing until descriptors register.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registry with the shipped v1 components: `"label"` (display text).
    pub fn builtin() -> Self {
        let mut registry = Self::new();
        // The builtin descriptor is fixed valid input, so registration
        // cannot fail; a failure would be a code bug, not user data.
        registry
            .register(Schema {
                component: "label",
                version: 1,
                migrate: None,
            })
            .expect("builtin label v1 is a valid descriptor");
        // The eight typed kinds are fixed valid descriptors too; the
        // `builtin_registry_serves_all_eight_at_v1` test pins that they land.
        let _registered = crate::component::register_builtin(&mut registry);
        registry
    }

    /// Register one descriptor. Rejects empty or bad names, version 0 and
    /// duplicate `(component, version)` pairs.
    pub fn register(&mut self, schema: Schema) -> Result<(), SchemaError> {
        check_component_name(schema.component)
            .map_err(|e| SchemaError::BadDescriptor { reason: e.reason() })?;
        if schema.version == 0 {
            return Err(SchemaError::BadDescriptor {
                reason: "schema version is 1-based; version 0 is never valid",
            });
        }
        let chain = self.steps.entry(schema.component).or_default();
        if chain.contains_key(&schema.version) {
            return Err(SchemaError::BadDescriptor {
                reason: "duplicate descriptor for this component version",
            });
        }
        chain.insert(schema.version, schema.migrate);
        Ok(())
    }

    /// Newest registered version of a component, if the component is known.
    pub fn current_version(&self, component: &str) -> Option<u32> {
        self.steps.get(component)?.keys().max().copied()
    }

    /// Validate a value and migrate its payload forward to current.
    ///
    /// Values already at current pass through unchanged (same bytes). Gaps
    /// in the chain and failing steps refuse with [`SchemaError`]; unknown
    /// components or versions above current refuse as unknown, never guess.
    pub fn migrate_value(
        &self,
        value: &VersionedComponent,
    ) -> Result<VersionedComponent, SchemaError> {
        check_value(value)?;
        let Some(chain) = self.steps.get(value.component.as_str()) else {
            return Err(SchemaError::UnknownVersion {
                component: value.component.clone(),
                got: value.version,
                current: 0,
            });
        };
        // Registration rejects version 0, so a stored chain is never empty.
        let current = chain.keys().max().copied().unwrap_or(0);
        if value.version > current {
            return Err(SchemaError::UnknownVersion {
                component: value.component.clone(),
                got: value.version,
                current,
            });
        }
        let mut payload = value.payload.clone();
        let mut from = value.version;
        while from < current {
            let step = chain.get(&from).copied().flatten().ok_or_else(|| {
                SchemaError::MigrationFailed {
                    component: value.component.clone(),
                    from,
                    to: from + 1,
                    cause: format!("no migration step registered for v{from}"),
                }
            })?;
            payload = step(&payload).map_err(|error| {
                let cause = match error {
                    SchemaError::MigrationFailed { cause, .. } => cause,
                    other => other.to_string(),
                };
                SchemaError::MigrationFailed {
                    component: value.component.clone(),
                    from,
                    to: from + 1,
                    cause: truncate_cause(cause),
                }
            })?;
            if payload.len() > MAX_PAYLOAD_LEN {
                return Err(SchemaError::MigrationFailed {
                    component: value.component.clone(),
                    from,
                    to: from + 1,
                    cause: "migrated payload exceeds the payload bound".into(),
                });
            }
            from += 1;
        }
        Ok(VersionedComponent {
            component: value.component.clone(),
            version: current,
            payload,
        })
    }
}

/// Why a component name was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ComponentNameError {
    /// The name is empty.
    Empty,
    /// The name is longer than [`MAX_COMPONENT_NAME_LEN`].
    TooLong,
    /// The name has a byte outside `a-z A-Z 0-9 - _ .`.
    NotPortable,
}

impl ComponentNameError {
    /// The fixed refusal text carried into [`SchemaError`].
    const fn reason(self) -> &'static str {
        match self {
            Self::Empty => "component name must not be empty",
            Self::TooLong => "component name exceeds the name bound",
            Self::NotPortable => "component name must be ASCII alphanumeric plus - _ .",
        }
    }
}

impl fmt::Display for ComponentNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.reason())
    }
}

impl std::error::Error for ComponentNameError {}

/// Validate a descriptor or value component name: non-empty, bounded, and
/// plain ASCII (`a-z A-Z 0-9 - _ .`) so names stay portable across files.
fn check_component_name(name: &str) -> Result<(), ComponentNameError> {
    if name.is_empty() {
        return Err(ComponentNameError::Empty);
    }
    if name.len() > MAX_COMPONENT_NAME_LEN {
        return Err(ComponentNameError::TooLong);
    }
    let plain = name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.');
    if !plain {
        return Err(ComponentNameError::NotPortable);
    }
    Ok(())
}

/// Validate a component value's shape (name, 1-based version, payload
/// bound) ahead of any migration.
fn check_value(value: &VersionedComponent) -> Result<(), SchemaError> {
    check_component_name(&value.component)
        .map_err(|e| SchemaError::BadComponent { reason: e.reason() })?;
    if value.version == 0 {
        return Err(SchemaError::BadComponent {
            reason: "component version is 1-based; version 0 is never valid",
        });
    }
    if value.payload.len() > MAX_PAYLOAD_LEN {
        return Err(SchemaError::BadComponent {
            reason: "component payload exceeds the payload bound",
        });
    }
    Ok(())
}

/// Bound a migration-failure cause, keeping a UTF-8 boundary.
fn truncate_cause(mut cause: String) -> String {
    cause.truncate(cause.floor_char_boundary(MAX_CAUSE_LEN));
    cause
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gadget_v1_to_v2(payload: &str) -> Result<String, SchemaError> {
        let failed = |cause: String| SchemaError::MigrationFailed {
            component: "gadget".to_string(),
            from: 1,
            to: 2,
            cause,
        };
        let mut value: serde_json::Value =
            serde_json::from_str(payload).map_err(|error| failed(error.to_string()))?;
        let fields = value
            .as_object_mut()
            .ok_or_else(|| failed("gadget v1 must be an object".to_string()))?;
        if !fields.get("level").is_some_and(|level| level.is_u64()) {
            return Err(failed("gadget v1 needs a u64 level".to_string()));
        }
        fields.insert("tier".to_string(), serde_json::Value::from(0u64));
        serde_json::to_string(&value).map_err(|error| failed(error.to_string()))
    }

    fn gadget_v2_to_v3(payload: &str) -> Result<String, SchemaError> {
        let failed = |cause: String| SchemaError::MigrationFailed {
            component: "gadget".to_string(),
            from: 2,
            to: 3,
            cause,
        };
        let mut value: serde_json::Value =
            serde_json::from_str(payload).map_err(|error| failed(error.to_string()))?;
        let fields = value
            .as_object_mut()
            .ok_or_else(|| failed("gadget v2 must be an object".to_string()))?;
        if !fields.get("tier").is_some_and(|tier| tier.is_u64()) {
            return Err(failed("gadget v2 needs a u64 tier".to_string()));
        }
        fields.insert("polish".to_string(), serde_json::Value::from(true));
        serde_json::to_string(&value).map_err(|error| failed(error.to_string()))
    }

    /// Registry with a two-step gadget chain, registered out of order to
    /// prove migration never depends on registration order.
    fn gadget_registry() -> SchemaRegistry {
        let mut registry = SchemaRegistry::new();
        registry
            .register(Schema {
                component: "gadget",
                version: 3,
                migrate: None,
            })
            .unwrap();
        registry
            .register(Schema {
                component: "gadget",
                version: 1,
                migrate: Some(gadget_v1_to_v2),
            })
            .unwrap();
        registry
            .register(Schema {
                component: "gadget",
                version: 2,
                migrate: Some(gadget_v2_to_v3),
            })
            .unwrap();
        registry
    }

    fn gadget_v1() -> VersionedComponent {
        VersionedComponent {
            component: "gadget".to_string(),
            version: 1,
            payload: r#"{"level":3}"#.to_string(),
        }
    }

    #[test]
    fn builtin_serves_label_v1_unchanged() {
        let registry = SchemaRegistry::builtin();
        assert_eq!(registry.current_version("label"), Some(1));
        let value = VersionedComponent {
            component: "label".to_string(),
            version: 1,
            payload: "Grand Hall".to_string(),
        };
        assert_eq!(registry.migrate_value(&value).unwrap(), value);
    }

    #[test]
    fn unknown_component_and_future_version_refuse_loudly() {
        let registry = SchemaRegistry::builtin();
        let unknown = VersionedComponent {
            component: "gadget".to_string(),
            version: 1,
            payload: "{}".to_string(),
        };
        let error = registry.migrate_value(&unknown).unwrap_err();
        assert_eq!(
            error,
            SchemaError::UnknownVersion {
                component: "gadget".to_string(),
                got: 1,
                current: 0,
            }
        );
        assert!(error.to_string().contains("gadget"));

        let future = VersionedComponent {
            component: "label".to_string(),
            version: 2,
            payload: "x".to_string(),
        };
        let error = registry.migrate_value(&future).unwrap_err();
        assert_eq!(
            error,
            SchemaError::UnknownVersion {
                component: "label".to_string(),
                got: 2,
                current: 1,
            }
        );
        // The refusal is untouched input: the value still migrates nowhere.
        assert!(error.to_string().contains("v2"));
        assert!(error.to_string().contains("v1"));
    }

    #[test]
    fn malformed_values_refuse_before_migration() {
        let registry = SchemaRegistry::builtin();
        for bad in [
            VersionedComponent {
                component: String::new(),
                version: 1,
                payload: "x".to_string(),
            },
            VersionedComponent {
                component: "has space".to_string(),
                version: 1,
                payload: "x".to_string(),
            },
            VersionedComponent {
                component: "label".to_string(),
                version: 0,
                payload: "x".to_string(),
            },
            VersionedComponent {
                component: "label".to_string(),
                version: 1,
                payload: "x".repeat(MAX_PAYLOAD_LEN + 1),
            },
        ] {
            assert!(
                matches!(
                    registry.migrate_value(&bad).unwrap_err(),
                    SchemaError::BadComponent { .. }
                ),
                "refuses {bad:?}"
            );
        }
    }

    #[test]
    fn bad_descriptors_refuse_at_registration() {
        let mut registry = SchemaRegistry::new();
        assert!(matches!(
            registry
                .register(Schema {
                    component: "",
                    version: 1,
                    migrate: None,
                })
                .unwrap_err(),
            SchemaError::BadDescriptor { .. }
        ));
        assert!(matches!(
            registry
                .register(Schema {
                    component: "gadget",
                    version: 0,
                    migrate: None,
                })
                .unwrap_err(),
            SchemaError::BadDescriptor { .. }
        ));
        registry
            .register(Schema {
                component: "gadget",
                version: 1,
                migrate: None,
            })
            .unwrap();
        assert!(matches!(
            registry
                .register(Schema {
                    component: "gadget",
                    version: 1,
                    migrate: None,
                })
                .unwrap_err(),
            SchemaError::BadDescriptor { .. }
        ));
    }

    #[test]
    fn two_step_chain_migrates_to_exact_bytes_twice() {
        let registry = gadget_registry();
        assert_eq!(registry.current_version("gadget"), Some(3));
        let first = registry.migrate_value(&gadget_v1()).unwrap();
        assert_eq!(first.version, 3);
        // serde_json sorts object keys, so the migrated bytes are fixed.
        assert_eq!(first.payload, r#"{"level":3,"polish":true,"tier":0}"#);
        // Migration output is hashed-state-relevant: run twice, same bytes.
        let second = registry.migrate_value(&gadget_v1()).unwrap();
        assert_eq!(first, second);

        // Mid-chain values join late and land on the same bytes.
        let mid = VersionedComponent {
            component: "gadget".to_string(),
            version: 2,
            payload: r#"{"level":3,"tier":0}"#.to_string(),
        };
        assert_eq!(registry.migrate_value(&mid).unwrap(), first);
    }

    #[test]
    fn failing_step_names_its_cause() {
        fn always_no(payload: &str) -> Result<String, SchemaError> {
            let _ = payload;
            Err(SchemaError::MigrationFailed {
                component: "gadget".to_string(),
                from: 1,
                to: 2,
                cause: "gadget forge is cold".to_string(),
            })
        }
        let mut registry = SchemaRegistry::new();
        registry
            .register(Schema {
                component: "gadget",
                version: 1,
                migrate: Some(always_no),
            })
            .unwrap();
        registry
            .register(Schema {
                component: "gadget",
                version: 2,
                migrate: None,
            })
            .unwrap();
        let error = registry.migrate_value(&gadget_v1()).unwrap_err();
        assert_eq!(
            error,
            SchemaError::MigrationFailed {
                component: "gadget".to_string(),
                from: 1,
                to: 2,
                cause: "gadget forge is cold".to_string(),
            }
        );
        assert!(error.to_string().contains("gadget forge is cold"));
    }

    #[test]
    fn long_causes_truncate_on_a_char_boundary() {
        fn loud(payload: &str) -> Result<String, SchemaError> {
            let _ = payload;
            // Long cause with multi-byte chars near the cut point.
            Err(SchemaError::MigrationFailed {
                component: "gadget".to_string(),
                from: 1,
                to: 2,
                cause: format!("é{}!", "x".repeat(MAX_CAUSE_LEN)),
            })
        }
        let mut registry = SchemaRegistry::new();
        registry
            .register(Schema {
                component: "gadget",
                version: 1,
                migrate: Some(loud),
            })
            .unwrap();
        registry
            .register(Schema {
                component: "gadget",
                version: 2,
                migrate: None,
            })
            .unwrap();
        let error = registry.migrate_value(&gadget_v1()).unwrap_err();
        let SchemaError::MigrationFailed { cause, .. } = error else {
            panic!("expected a migration failure");
        };
        assert!(cause.len() <= MAX_CAUSE_LEN);
        assert!(cause.is_char_boundary(cause.len()));
    }

    #[test]
    fn chain_gap_refuses_instead_of_guessing() {
        let mut registry = SchemaRegistry::new();
        registry
            .register(Schema {
                component: "gadget",
                version: 1,
                migrate: Some(gadget_v1_to_v2),
            })
            .unwrap();
        // v3 registered with no v2 step: the v1 value climbs to v2, then
        // the missing step refuses loudly.
        registry
            .register(Schema {
                component: "gadget",
                version: 3,
                migrate: None,
            })
            .unwrap();
        let error = registry.migrate_value(&gadget_v1()).unwrap_err();
        assert_eq!(
            error,
            SchemaError::MigrationFailed {
                component: "gadget".to_string(),
                from: 2,
                to: 3,
                cause: "no migration step registered for v2".to_string(),
            }
        );
    }

    #[test]
    fn oversize_migration_output_refuses() {
        fn greedy(payload: &str) -> Result<String, SchemaError> {
            let _ = payload;
            Ok("x".repeat(MAX_PAYLOAD_LEN + 1))
        }
        let mut registry = SchemaRegistry::new();
        registry
            .register(Schema {
                component: "gadget",
                version: 1,
                migrate: Some(greedy),
            })
            .unwrap();
        registry
            .register(Schema {
                component: "gadget",
                version: 2,
                migrate: None,
            })
            .unwrap();
        assert!(matches!(
            registry.migrate_value(&gadget_v1()).unwrap_err(),
            SchemaError::MigrationFailed { .. }
        ));
    }
}
