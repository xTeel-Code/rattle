//! Registry operations shared by the CLI commands and the TUI.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::Config;
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

/// Registers project folders found up to `config.max_depth` levels below
/// `roots`. A folder is a project if it contains one of `config.markers`;
/// the search does not descend into projects, hidden folders or folders
/// named in `config.ignore`. Only mutates `store`; saving is up to the caller.
pub fn scan_roots(store: &mut Store, roots: &[String], config: &Config) -> ScanReport {
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
        let mut dirs = Vec::new();
        if let Err(e) = find_projects(&root, config.max_depth, config, &mut dirs) {
            report.warnings.push(format!("skipping '{raw}': {e}"));
            continue;
        }
        let mut dirs: Vec<PathBuf> =
            dirs.into_iter().filter_map(|p| std::fs::canonicalize(p).ok()).collect();
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

/// Collects project folders below `dir`, at most `depth` levels down.
/// Errors only for `dir` itself; unreadable subfolders are skipped.
fn find_projects(dir: &Path, depth: usize, config: &Config, out: &mut Vec<PathBuf>) -> io::Result<()> {
    if depth == 0 {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else { continue };
        // Symlinked folders may be projects but are never descended into (avoids loops).
        let is_dir = kind.is_dir() || (kind.is_symlink() && path.is_dir());
        if !is_dir || is_hidden(&path) || is_ignored(&path, &config.ignore) {
            continue;
        }
        if config.markers.iter().any(|m| path.join(m).exists()) {
            out.push(path);
        } else if kind.is_dir() {
            let _ = find_projects(&path, depth - 1, config, out);
        }
    }
    Ok(())
}

fn is_ignored(p: &Path, ignore: &[String]) -> bool {
    p.file_name().is_some_and(|n| ignore.iter().any(|i| n == i.as_str()))
}

/// Builds the command that opens `target` from inside `dir`, using the
/// configured editor, else `$VISUAL`, else `$EDITOR`, else `vi`.
/// With `wait`, GUI editors get their wait flag so the command only returns
/// once the file is closed. Also returns the editor setting, for error messages.
pub fn editor_command(
    configured: Option<&str>,
    dir: &Path,
    target: &Path,
    wait: bool,
) -> (Command, String) {
    let from_env = |var| std::env::var(var).ok();
    let editor = [configured.map(str::to_string), from_env("VISUAL"), from_env("EDITOR")]
        .into_iter()
        .flatten()
        .find(|e| !e.trim().is_empty())
        .unwrap_or_else(|| "vi".into());
    let parts: Vec<&str> = editor.split_whitespace().collect();
    let program = parts.first().copied().unwrap_or("vi");
    let mut cmd = Command::new(program);
    cmd.args(&parts[1..]);
    let has_wait = parts.iter().any(|a| *a == "-w" || *a == "--wait");
    if wait && !has_wait && is_gui_editor(program) {
        cmd.arg("--wait");
    }
    cmd.arg(target).current_dir(dir);
    (cmd, editor)
}

/// GUI editors return immediately unless given `--wait`; terminal editors block anyway.
fn is_gui_editor(program: &str) -> bool {
    const GUI: &[&str] =
        &["zed", "code", "code-insiders", "codium", "cursor", "windsurf", "subl", "mate"];
    // Zed's bundled CLI is called `cli`, e.g. /Applications/Zed.app/Contents/MacOS/cli.
    let name = Path::new(program).file_name().and_then(|n| n.to_str()).unwrap_or(program);
    GUI.contains(&name) || program.contains("Zed.app/")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn editor_args(editor: &str, wait: bool) -> Vec<String> {
        let (cmd, _) = editor_command(Some(editor), Path::new("."), Path::new("f.toml"), wait);
        cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn wait_flag_only_for_gui_editors_when_waiting() {
        assert_eq!(editor_args("zed", true), vec!["--wait", "f.toml"]);
        assert_eq!(editor_args("zed", false), vec!["f.toml"]);
        assert_eq!(editor_args("zed -w", true), vec!["-w", "f.toml"]);
        assert_eq!(editor_args("/Applications/Zed.app/Contents/MacOS/cli", true), vec!["--wait", "f.toml"]);
        assert_eq!(editor_args("/usr/local/bin/code --new-window", true), vec!["--new-window", "--wait", "f.toml"]);
        assert_eq!(editor_args("hx", true), vec!["f.toml"]);
    }

    /// Builds a throwaway folder tree; paths ending in `/` are folders, others files.
    fn tree(name: &str, paths: &[&str]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("rattle-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for p in paths {
            let full = root.join(p);
            if p.ends_with('/') {
                std::fs::create_dir_all(full).unwrap();
            } else {
                std::fs::create_dir_all(full.parent().unwrap()).unwrap();
                std::fs::write(full, "").unwrap();
            }
        }
        root
    }

    fn names(report: &ScanReport) -> Vec<&str> {
        report.added.iter().map(|p| p.name.as_str()).collect()
    }

    #[test]
    fn deep_scan_stops_at_projects_and_skips_ignored() {
        let root = tree(
            "deep",
            &[
                "flat/.git/",
                "flat/packages/inner/Cargo.toml", // inside a project: not registered
                "org/team/nested/package.json",   // depth 3: found
                "a/b/c/too-deep/.git/",           // depth 4: beyond max_depth
                "node_modules/dep/package.json",  // ignored folder
                ".cache/hidden/.git/",            // hidden folder
                "plain/notes/",                   // no marker anywhere
            ],
        );
        let roots = [root.display().to_string()];
        let mut store = Store::default();
        let report = scan_roots(&mut store, &roots, &Config::default());
        assert_eq!(names(&report), vec!["flat", "nested"]);

        let deeper = Config { max_depth: 4, ..Config::default() };
        let report = scan_roots(&mut store, &roots, &deeper);
        assert_eq!((names(&report), report.already), (vec!["too-deep"], 2));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn depth_one_matches_direct_children_only() {
        let root = tree("shallow", &["top/.git/", "group/child/.git/"]);
        let config = Config { max_depth: 1, ..Config::default() };
        let report = scan_roots(&mut Store::default(), &[root.display().to_string()], &config);
        assert_eq!(names(&report), vec!["top"]);
        std::fs::remove_dir_all(root).unwrap();
    }
}
