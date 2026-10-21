//! Registry operations shared by the CLI commands and the TUI.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::store::{self, Project, Store};

/// Turns a user-supplied folder path into a project entry (name = folder name).
pub fn resolve_project(raw: &str) -> Result<Project, String> {
    if raw.is_empty() {
        return Err("no path given".into());
    }
    let path = std::fs::canonicalize(expand_tilde(raw))
        .map_err(|e| format!("cannot access '{raw}': {e}"))?;
    if !path.is_dir() {
        return Err(format!("'{}' is not a directory", path.display()));
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or("cannot derive a project name from that path")?;
    Ok(Project { name, path: path.to_string_lossy().into_owned() })
}

#[derive(Debug, Default)]
pub struct ScanReport {
    pub added: Vec<Project>,
    pub already: usize,
    pub warnings: Vec<String>,
}

/// Registers direct subfolders of `roots` that contain one of `markers`.
/// Only mutates `store`; saving is up to the caller.
pub fn scan_roots(store: &mut Store, roots: &[String], markers: &[String]) -> ScanReport {
    let mut report = ScanReport::default();
    let mut known: HashSet<String> = store.projects.iter().map(|p| p.path.clone()).collect();

    for raw in roots {
        let root = match std::fs::canonicalize(expand_tilde(raw)) {
            Ok(r) if r.is_dir() => r,
            _ => {
                report.warnings.push(format!("skipping '{raw}': not an accessible directory"));
                continue;
            }
        };
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(e) => {
                report.warnings.push(format!("skipping '{raw}': {e}"));
                continue;
            }
        };
        let mut dirs: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir() && !is_hidden(p))
            .filter(|p| markers.iter().any(|m| p.join(m).exists()))
            .filter_map(|p| std::fs::canonicalize(p).ok())
            .collect();
        dirs.sort();

        for dir in dirs {
            let path = dir.to_string_lossy().into_owned();
            if known.contains(&path) {
                report.already += 1;
                continue;
            }
            let name = dir.file_name().unwrap_or_default().to_string_lossy().into_owned();
            if let Some(existing) = store.find(&name) {
                report.warnings.push(format!(
                    "skipping {path}: name '{name}' is already used by {}",
                    existing.path
                ));
                continue;
            }
            let project = Project { name, path: path.clone() };
            store.projects.push(project.clone());
            report.added.push(project);
            known.insert(path);
        }
    }
    report
}

pub fn expand_tilde(p: &str) -> PathBuf {
    match p.strip_prefix("~") {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            store::home_dir().join(rest.trim_start_matches('/'))
        }
        _ => PathBuf::from(p),
    }
}

fn is_hidden(p: &Path) -> bool {
    p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))
}
