//! Panel inventory: self-registering dock panels over the shell.
//!
//! The consumer is the dock startup plus the frame budget table: both read
//! the registry instead of a hand list, so a new panel module appears with
//! one register call and no call-site edit. Units are dock ids and titles.
//! Determinism: registry order is registration order, never hash order.
//! Errors: registration never fails; duplicates keep the first entry.
//!
//! PORT(ChenlizheMe/Infernux@088897d:python/infernux/engine/ui/panel_registry.py
//! `@editor_panel` plus `PanelRegistry.apply_all`) -> plain `fn register`
//! over the existing `Panel` enum, no macro and no reflection.

use super::{Panel, Shell};

/// One dock panel as seen in the window and the menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PanelDescriptor {
    /// Stable id, e.g. `"hierarchy"`.
    pub id: &'static str,
    /// Tab title, matches [`Panel::title`].
    pub title: &'static str,
    /// Menu path, e.g. `"View/Hierarchy"`.
    pub menu_path: &'static str,
    /// The dock tab this entry opens.
    pub panel: Panel,
}

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

    /// Entry count.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no panel is registered.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entries in render order.
    pub fn iter(&self) -> impl Iterator<Item = &PanelDescriptor> {
        self.entries.iter()
    }

    /// Panels in render order.
    pub fn panels(&self) -> Vec<Panel> {
        self.entries.iter().map(|entry| entry.panel).collect()
    }

    /// Open every registered panel through the shell; idempotent.
    pub fn apply_all(&self, shell: &mut Shell) {
        for entry in &self.entries {
            shell.open_panel(entry.panel);
        }
    }
}

/// The seven dock panels in default order.
pub fn builtin_registry() -> PanelRegistry {
    let mut registry = PanelRegistry::new();
    registry.register(PanelDescriptor {
        id: "hierarchy",
        title: "Hierarchy",
        menu_path: "View/Hierarchy",
        panel: Panel::Hierarchy,
    });
    registry.register(PanelDescriptor {
        id: "inspector",
        title: "Inspector",
        menu_path: "View/Inspector",
        panel: Panel::Inspector,
    });
    registry.register(PanelDescriptor {
        id: "viewport",
        title: "Viewport",
        menu_path: "View/Viewport",
        panel: Panel::Viewport,
    });
    registry.register(PanelDescriptor {
        id: "findings",
        title: "Findings",
        menu_path: "View/Findings",
        panel: Panel::Findings,
    });
    registry.register(PanelDescriptor {
        id: "tuning",
        title: "Tuning",
        menu_path: "View/Tuning",
        panel: Panel::Tuning,
    });
    registry.register(PanelDescriptor {
        id: "templates",
        title: "Templates",
        menu_path: "View/Templates",
        panel: Panel::Templates,
    });
    registry.register(PanelDescriptor {
        id: "log",
        title: "Log",
        menu_path: "View/Log",
        panel: Panel::Log,
    });
    registry
}

/// Ensure every built-in panel is open; idempotent and allocation-free on
/// repeat calls, so the per-frame dock pass can call it directly.
pub fn ensure_builtin_panels(shell: &mut Shell) {
    for panel in [
        Panel::Hierarchy,
        Panel::Inspector,
        Panel::Viewport,
        Panel::Findings,
        Panel::Tuning,
        Panel::Templates,
        Panel::Log,
    ] {
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
            title: "Notes",
            menu_path: "View/Notes",
            panel: Panel::Log,
        });
        registry.register(PanelDescriptor {
            id: "fresh-notes",
            title: "Notes",
            menu_path: "View/Notes",
            panel: Panel::Log,
        });
        assert_eq!(registry.len(), 1);
        registry.apply_all(&mut shell);
        assert!(shell.is_open(Panel::Log));
        assert_eq!(builtin_registry().len(), 7);
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
