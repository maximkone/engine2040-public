//! Panel inventory: self-registering dock panels over the shell.
//!
//! The consumer is the dock startup plus the per-frame dock pass: both read
//! the [`BUILTIN`] table (via [`builtin_registry`] or [`ensure_builtin_panels`]),
//! so a new panel module appears with one table row and no call-site edit.
//! Units are dock ids. Determinism: registry order is registration order,
//! never hash order. Errors: registration never fails; duplicates keep the
//! first entry.
//!
//! PORT(ChenlizheMe/Infernux@088897d:python/infernux/engine/ui/panel_registry.py
//! `@editor_panel` plus `PanelRegistry.apply_all`) -> plain `fn register`
//! over the existing `Panel` enum, no macro and no reflection.

use super::{Panel, Shell};

/// One dock panel: a stable id plus the dock tab it opens. Titles render
/// through [`Panel::title`], so the descriptor carries no title or menu copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PanelDescriptor {
    /// Stable id, e.g. `"hierarchy"`.
    pub id: &'static str,
    /// The dock tab this entry opens.
    pub panel: Panel,
}

/// The seven dock panels in default order: the single source of truth behind
/// [`builtin_registry`] and [`ensure_builtin_panels`].
pub const BUILTIN: [(&str, Panel); 7] = [
    ("hierarchy", Panel::Hierarchy),
    ("inspector", Panel::Inspector),
    ("viewport", Panel::Viewport),
    ("findings", Panel::Findings),
    ("tuning", Panel::Tuning),
    ("templates", Panel::Templates),
    ("log", Panel::Log),
];

/// Inventory of dock panels in render order.
#[derive(Clone, Debug, Default)]
pub struct PanelRegistry {
    entries: Vec<PanelDescriptor>,
}

impl PanelRegistry {
    /// Empty inventory.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register one panel module; duplicates keep the first entry.
    pub fn register(&mut self, desc: PanelDescriptor) {
        if !self.entries.iter().any(|entry| entry.id == desc.id) {
            self.entries.push(desc);
        }
    }

    /// Panels in render order.
    pub fn panels(&self) -> Vec<Panel> {
        self.entries.iter().map(|entry| entry.panel).collect()
    }
}

/// The seven dock panels in default order.
pub fn builtin_registry() -> PanelRegistry {
    let mut registry = PanelRegistry::new();
    for (id, panel) in BUILTIN {
        registry.register(PanelDescriptor { id, panel });
    }
    registry
}

/// Ensure every built-in panel is open; idempotent and allocation-free on
/// repeat calls, so the per-frame dock pass can call it directly.
pub fn ensure_builtin_panels(shell: &mut Shell) {
    for (_, panel) in BUILTIN {
        shell.open_panel(panel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_module_registers_once_and_applies() {
        let mut shell = Shell::new();
        assert!(shell.close_panel(Panel::Log));
        let mut registry = PanelRegistry::new();
        registry.register(PanelDescriptor {
            id: "fresh-notes",
            panel: Panel::Log,
        });
        registry.register(PanelDescriptor {
            id: "fresh-notes",
            panel: Panel::Log,
        });
        assert_eq!(registry.panels(), vec![Panel::Log]);
        for panel in registry.panels() {
            shell.open_panel(panel);
        }
        assert!(shell.is_open(Panel::Log));
        assert_eq!(builtin_registry().panels(), Panel::all());
        assert_eq!(builtin_registry().panels().len(), 7);
    }

    #[test]
    fn ensure_reopens_missing_and_stays_put() {
        let mut shell = Shell::new();
        assert!(shell.close_panel(Panel::Log));
        ensure_builtin_panels(&mut shell);
        assert!(shell.is_open(Panel::Log));
        let len = shell.open_panels.len();
        ensure_builtin_panels(&mut shell);
        assert_eq!(shell.open_panels.len(), len);
    }
}
