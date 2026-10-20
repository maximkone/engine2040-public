//! W5: the dock's asset browser tab (files in a folder, import through EI3).
//!
//! The consumer is the dock (`shell_ui`); the owner is [`BrowserTab`].
//! Units: one folder path and up to 200 file rows. Determinism: rows sort by
//! name; the only write is the `Shell::import_glb` batch an Import click emits
//! (one journal entry). Errors are typed [`BrowserError`]; a failed scan keeps
//! the previous rows.

use std::fmt;
use std::path::PathBuf;

use crate::shell::Shell;

/// Why a browser scan or import was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowserError {
    /// A folder or file could not be read.
    Read {
        /// The path that failed.
        path: String,
        /// OS report.
        reason: String,
    },
    /// The row is not a glTF file (images are listed read-only).
    NotGltf {
        /// Row name shown to the user.
        name: String,
    },
}

impl fmt::Display for BrowserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, reason } => write!(f, "read {path}: {reason}"),
            Self::NotGltf { name } => write!(f, "{name} is not a glTF file"),
        }
    }
}

impl std::error::Error for BrowserError {}

/// One file row: a path, its display name and whether it imports or is an image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserRow {
    /// Absolute or folder-relative path.
    pub path: PathBuf,
    /// File name shown in the list.
    pub name: String,
    /// `"gltf"` (importable) or `"image"` (read-only).
    pub kind: &'static str,
}

/// Scan `dir` for `.glb`/`.gltf` files and images, sorted by name, capped at
/// 200 rows. Nothing is written.
pub fn scan_browser_dir(dir: &str) -> Result<Vec<BrowserRow>, BrowserError> {
    let entries = std::fs::read_dir(dir).map_err(|error| BrowserError::Read {
        path: dir.to_owned(),
        reason: error.to_string(),
    })?;
    let mut rows = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| BrowserError::Read {
            path: dir.to_owned(),
            reason: error.to_string(),
        })?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
            continue;
        };
        let kind = match extension.to_ascii_lowercase().as_str() {
            "glb" | "gltf" => "gltf",
            "png" | "jpg" | "jpeg" => "image",
            _ => continue,
        };
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("?")
            .to_owned();
        rows.push(BrowserRow { path, name, kind });
        if rows.len() >= 200 {
            break;
        }
    }
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(rows)
}

/// Import one glTF row through the EI3 `import_glb` path at the origin.
/// Returns the batch's node count; an image row is refused.
pub fn import_browser_row(shell: &mut Shell, row: &BrowserRow) -> Result<usize, BrowserError> {
    if row.kind != "gltf" {
        return Err(BrowserError::NotGltf {
            name: row.name.clone(),
        });
    }
    let path = row.path.display().to_string();
    let bytes = std::fs::read(&row.path).map_err(|error| BrowserError::Read {
        path: path.clone(),
        reason: error.to_string(),
    })?;
    let applied = shell
        .import_glb(&path, &bytes, [0.0, 0.0, 0.0])
        .map_err(|error| BrowserError::Read {
            path: path.clone(),
            reason: error.to_string(),
        })?;
    Ok(applied.effects.len())
}

/// The Browser tab state: folder text, last rows and last notice.
#[derive(Clone, Debug, Default)]
pub struct BrowserTab {
    dir: String,
    rows: Vec<BrowserRow>,
    status: Option<String>,
}

impl BrowserTab {
    /// Start on the current folder with no rows; the first show scans.
    pub fn new() -> Self {
        Self {
            dir: ".".to_owned(),
            rows: Vec::new(),
            status: None,
        }
    }

    /// Set the folder text; call [`scan`](Self::scan) to read it.
    pub fn set_dir(&mut self, dir: impl Into<String>) {
        self.dir = dir.into();
    }

    /// Rows from the last successful scan.
    pub fn rows(&self) -> &[BrowserRow] {
        &self.rows
    }

    /// Last notice (scan error or import outcome).
    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    /// Rescan the folder, replacing rows only on success.
    pub fn scan(&mut self) {
        match scan_browser_dir(&self.dir) {
            Ok(rows) => {
                self.status = Some(format!("{} file(s) in {}", rows.len(), self.dir));
                self.rows = rows;
            }
            Err(error) => self.status = Some(error.to_string()),
        }
    }

    /// Draw the tab. An Import click emits one `import_glb` batch.
    pub fn show(&mut self, ui: &mut egui::Ui, shell: &mut Shell) {
        if self.status.is_none() {
            self.scan();
        }
        let mut rescan = false;
        ui.horizontal(|ui| {
            ui.label("folder");
            ui.add(egui::TextEdit::singleline(&mut self.dir).desired_width(360.0));
            rescan = ui.button("Scan").clicked();
        });
        if rescan {
            self.scan();
        }
        if self.rows.is_empty() {
            ui.label("no glTF or image files: set a folder and press Scan");
        }
        let mut import: Option<usize> = None;
        egui::ScrollArea::vertical()
            .max_height(360.0)
            .show(ui, |ui| {
                for (index, row) in self.rows.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("{} [{}]", row.name, row.kind));
                        if row.kind == "gltf" && ui.button("Import").clicked() {
                            import = Some(index);
                        }
                    });
                }
            });
        if let Some(index) = import {
            if let Some(row) = self.rows.get(index).cloned() {
                match import_browser_row(shell, &row) {
                    Ok(nodes) => {
                        let line = format!("import {}: {nodes} node(s)", row.name);
                        shell.log_message(line.clone());
                        self.status = Some(line);
                    }
                    Err(error) => {
                        shell.log_message(format!("ERR import {}: {error}", row.name));
                        self.status = Some(error.to_string());
                    }
                }
            }
        }
        if let Some(status) = self.status.as_deref() {
            ui.label(status);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangle_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("e2040-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("Triangle.gltf"),
            include_bytes!("../../../fixtures/khronos/Triangle.gltf"),
        )
        .unwrap();
        dir
    }

    #[test]
    fn scan_finds_a_gltf_and_import_is_one_batch() {
        let dir = triangle_dir("browser");
        let rows = scan_browser_dir(dir.to_str().unwrap()).unwrap();
        assert_eq!(rows.len(), 1, "one glTF row: {rows:?}");
        assert_eq!(rows[0].name, "Triangle.gltf");
        assert_eq!(rows[0].kind, "gltf");

        let mut shell = Shell::new();
        let base = shell.journal().past_len();
        let nodes = import_browser_row(&mut shell, &rows[0]).unwrap();
        assert!(nodes > 0, "import created nodes");
        assert_eq!(shell.journal().past_len(), base + 1, "one undo step");
        shell.undo().unwrap();
        assert_eq!(shell.journal().past_len(), base);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_folder_is_a_typed_error_and_keeps_rows() {
        let mut tab = BrowserTab::new();
        tab.set_dir("Z:\\e2040-no-such-folder");
        tab.scan();
        assert!(tab.rows().is_empty());
        assert!(tab.status().is_some_and(|status| status.contains("read")));
        let dir = triangle_dir("browser-keep");
        tab.set_dir(dir.to_str().unwrap());
        tab.scan();
        assert_eq!(tab.rows().len(), 1);
        tab.set_dir("Z:\\e2040-no-such-folder");
        tab.scan();
        assert_eq!(tab.rows().len(), 1, "a failed scan keeps the last rows");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
