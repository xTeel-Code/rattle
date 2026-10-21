//! Registry operations shared by the CLI commands and the TUI.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::{Config, DeleteFiles};
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
    Ok(Project {
        name,
        path: path.to_string_lossy().into_owned(),
    })
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
                report
                    .warnings
                    .push(format!("skipping '{raw}': not an accessible directory"));
                continue;
            }
        };
        let mut dirs = Vec::new();
        if let Err(e) = find_projects(&root, config.max_depth, config, &mut dirs) {
            report.warnings.push(format!("skipping '{raw}': {e}"));
            continue;
        }
        let mut dirs: Vec<PathBuf> = dirs
            .into_iter()
            .filter_map(|p| std::fs::canonicalize(p).ok())
            .collect();
        dirs.sort();

        for dir in dirs {
            let path = dir.to_string_lossy().into_owned();
            if known.contains(&path) {
                report.already += 1;
                continue;
            }
            let name = dir
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            if let Some(existing) = store.find(&name) {
                report.warnings.push(format!(
                    "skipping {path}: name '{name}' is already used by {}",
                    existing.path
                ));
                continue;
            }
            let project = Project {
                name,
                path: path.clone(),
            };
            store.projects.push(project.clone());
            report.added.push(project);
            known.insert(path);
        }
    }
    report
}

/// Collects project folders below `dir`, at most `depth` levels down.
/// Errors only for `dir` itself; unreadable subfolders are skipped.
fn find_projects(
    dir: &Path,
    depth: usize,
    config: &Config,
    out: &mut Vec<PathBuf>,
) -> io::Result<()> {
    if depth == 0 {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
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
    p.file_name()
        .is_some_and(|n| ignore.iter().any(|i| n == i.as_str()))
}

/// Checks that `project`'s folder is safe to delete and returns its real path.
/// Refuses folders that are, or contain, the home folder, a top-level folder,
/// a scan dir, Rattle's own data, or other registered projects.
pub fn deletable_folder(
    project: &Project,
    store: &Store,
    config: &Config,
    registry: &Path,
) -> Result<PathBuf, String> {
    let path = std::fs::canonicalize(&project.path)
        .map_err(|e| format!("cannot access {}: {e}", project.path))?;
    if !path.is_dir() {
        return Err(format!("{} is not a folder", path.display()));
    }
    // Is `other` this folder or somewhere inside it?
    let covers = |other: &Path| std::fs::canonicalize(other).is_ok_and(|o| o.starts_with(&path));
    // Is this folder strictly inside `base`?
    let inside =
        |base: &Path| std::fs::canonicalize(base).is_ok_and(|b| path.starts_with(&b) && path != b);

    if path.components().count() <= 2 {
        return Err(format!(
            "refusing to delete top-level folder {}",
            path.display()
        ));
    }
    // Allow-list rather than a list of system folders: only places projects live.
    let allowed_place =
        inside(&store::home_dir()) || config.scan_dirs.iter().any(|d| inside(&expand_tilde(d)));
    if !allowed_place {
        return Err(format!(
            "refusing to delete {}: only folders inside your home folder or a scan folder can be deleted",
            path.display()
        ));
    }
    if covers(&store::home_dir()) {
        return Err(format!(
            "refusing to delete {}: it is or contains your home folder",
            path.display()
        ));
    }
    if let Some(dir) = config.scan_dirs.iter().find(|d| covers(&expand_tilde(d))) {
        return Err(format!(
            "refusing to delete {}: it is or contains scan folder {dir}",
            path.display()
        ));
    }
    if registry.parent().is_some_and(covers) {
        return Err(format!(
            "refusing to delete {}: it contains Rattle's own data",
            path.display()
        ));
    }
    let nested: Vec<&str> = store
        .projects
        .iter()
        .filter(|p| p.name != project.name && covers(Path::new(&p.path)))
        .map(|p| p.name.as_str())
        .collect();
    if !nested.is_empty() {
        return Err(format!(
            "refusing to delete {}: it contains other registered projects ({}); remove those first",
            path.display(),
            nested.join(", ")
        ));
    }
    Ok(path)
}

/// Moves `path` to the Trash or deletes it for good, depending on `mode`.
pub fn delete_folder(path: &Path, mode: DeleteFiles) -> Result<(), String> {
    match mode {
        DeleteFiles::Off => Err("deleting folders is turned off (delete_files = \"off\")".into()),
        DeleteFiles::Trash => {
            #[cfg(not(target_os = "macos"))]
            let trash = trash::TrashContext::default();
            // Finder's method may pop up a permission dialog, which would hang the TUI.
            #[cfg(target_os = "macos")]
            let trash = {
                use trash::macos::{DeleteMethod, TrashContextExtMacos};
                let mut trash = trash::TrashContext::default();
                trash.set_delete_method(DeleteMethod::NsFileManager);
                trash
            };
            trash
                .delete(path)
                .map_err(|e| format!("could not move {} to the Trash: {e}", path.display()))
        }
        DeleteFiles::Permanent => std::fs::remove_dir_all(path)
            .map_err(|e| format!("could not delete {}: {e}", path.display())),
    }
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
    let editor = [
        configured.map(str::to_string),
        from_env("VISUAL"),
        from_env("EDITOR"),
    ]
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
    const GUI: &[&str] = &[
        "zed",
        "code",
        "code-insiders",
        "codium",
        "cursor",
        "windsurf",
        "subl",
        "mate",
    ];
    // Zed's bundled CLI is called `cli`, e.g. /Applications/Zed.app/Contents/MacOS/cli.
    let name = Path::new(program)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(program);
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
    p.file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with('.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor_args(editor: &str, wait: bool) -> Vec<String> {
        let (cmd, _) = editor_command(Some(editor), Path::new("."), Path::new("f.toml"), wait);
        cmd.get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn wait_flag_only_for_gui_editors_when_waiting() {
        assert_eq!(editor_args("zed", true), vec!["--wait", "f.toml"]);
        assert_eq!(editor_args("zed", false), vec!["f.toml"]);
        assert_eq!(editor_args("zed -w", true), vec!["-w", "f.toml"]);
        assert_eq!(
            editor_args("/Applications/Zed.app/Contents/MacOS/cli", true),
            vec!["--wait", "f.toml"]
        );
        assert_eq!(
            editor_args("/usr/local/bin/code --new-window", true),
            vec!["--new-window", "--wait", "f.toml"]
        );
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

        let deeper = Config {
            max_depth: 4,
            ..Config::default()
        };
        let report = scan_roots(&mut store, &roots, &deeper);
        assert_eq!((names(&report), report.already), (vec!["too-deep"], 2));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn depth_one_matches_direct_children_only() {
        let root = tree("shallow", &["top/.git/", "group/child/.git/"]);
        let config = Config {
            max_depth: 1,
            ..Config::default()
        };
        let report = scan_roots(
            &mut Store::default(),
            &[root.display().to_string()],
            &config,
        );
        assert_eq!(names(&report), vec!["top"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn deletable_folder_refuses_dangerous_targets() {
        let root = tree(
            "delete",
            &["scan/proj/.git/", "scan/outer/inner/.git/", "scan/reg/"],
        );
        let at = |rel: &str| root.join(rel).display().to_string();
        let project = |name: &str, rel: &str| Project {
            name: name.into(),
            path: at(rel),
        };
        let mut store = Store::default();
        for (name, rel) in [
            ("proj", "scan/proj"),
            ("outer", "scan/outer"),
            ("inner", "scan/outer/inner"),
        ] {
            store.add(project(name, rel)).unwrap();
        }
        let config = Config {
            scan_dirs: vec![at("scan")],
            ..Config::default()
        };
        let registry = root.join("scan/reg/projects.toml");
        let check = |p: &Project| deletable_folder(p, &store, &config, &registry);

        assert!(check(&project("proj", "scan/proj")).is_ok());
        assert!(check(&project("inner", "scan/outer/inner")).is_ok());
        let refused = |p: Project, why: &str| {
            let err = check(&p).unwrap_err();
            assert!(err.contains(why), "{err}");
        };
        refused(
            project("outer", "scan/outer"),
            "other registered projects (inner)",
        );
        refused(project("scan", "scan"), "scan folder");
        refused(project("reg", "scan/reg"), "Rattle's own data");
        refused(
            Project {
                name: "root".into(),
                path: "/".into(),
            },
            "top-level",
        );
        // /tmp is top-level on Linux, but /private/tmp on macOS, where only the
        // home/scan-dir allow-list refuses it. Either way it must be refused.
        refused(
            Project {
                name: "tmp".into(),
                path: "/tmp".into(),
            },
            "refusing to delete",
        );
        refused(
            Project {
                name: "usr".into(),
                path: "/usr/local".into(),
            },
            "only folders inside",
        );
        refused(project("ghost", "scan/ghost"), "cannot access");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn delete_folder_off_never_touches_files() {
        let root = tree("delete-off", &["proj/file.txt"]);
        assert!(delete_folder(&root.join("proj"), DeleteFiles::Off).is_err());
        assert!(root.join("proj/file.txt").exists());
        delete_folder(&root.join("proj"), DeleteFiles::Permanent).unwrap();
        assert!(!root.join("proj").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
