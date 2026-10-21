//! User configuration stored at `~/.rattle/config.toml` (next to the registry).
//!
//! ```toml
//! # Folders `rattle scan` searches for projects.
//! scan_dirs = ["~/code", "~/work"]
//!
//! # How many folder levels below each scan dir to search.
//! max_depth = 3
//!
//! # Rescan scan_dirs every time the TUI opens.
//! auto_scan = true
//!
//! # A folder counts as a project if it contains any of these.
//! markers = [".git", "Cargo.toml", "pyproject.toml", "package.json"]
//!
//! # Folder names that are never searched.
//! ignore = ["node_modules", "target", ".venv"]
//! ```

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::store::unquote;

pub const DEFAULT_MARKERS: &[&str] = &[
    ".git",
    "Cargo.toml",
    "pyproject.toml",
    "package.json",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "CMakeLists.txt",
    "Makefile",
];

/// Folder names that are never searched (hidden folders are always skipped too).
pub const DEFAULT_IGNORE: &[&str] = &[
    "node_modules",
    "target",
    "venv",
    "build",
    "dist",
    "vendor",
    "Library",
    "Applications",
];

pub const DEFAULT_MAX_DEPTH: usize = 3;

/// What deleting a project may do to its folder (`delete_files` setting).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeleteFiles {
    /// Only registry entries are ever removed.
    #[default]
    Off,
    /// Folders are moved to the Trash.
    Trash,
    /// Folders are deleted for good, after typing the project name.
    Permanent,
}

impl DeleteFiles {
    pub const ALL: [Self; 3] = [Self::Off, Self::Trash, Self::Permanent];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Trash => "trash",
            Self::Permanent => "permanent",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.as_str() == s)
    }
}

const TEMPLATE: &str = "\
# Folders `rattle scan` searches for projects, e.g. [\"~/code\", \"~/work\"].
scan_dirs = []

# How many folder levels below each scan dir to search.
# Searching stops at the first folder that is a project.
max_depth = 3

# Rescan scan_dirs every time the TUI opens.
auto_scan = true

# Editor for `e` (open a project) and `c` (edit this file).
# Defaults to $VISUAL, then $EDITOR, then vi. GUI editors (zed, code, cursor,
# subl, …) get --wait automatically when editing this file, so \"zed\" is enough.
# editor = \"hx\"

# What deleting a project's folder (`D` in the TUI, `rattle delete --files`) does:
# \"off\" never touches files, \"trash\" moves the folder to the Trash,
# \"permanent\" deletes it for good after you type the project name.
delete_files = \"off\"

# Where `rattle new` / `n` creates projects. Defaults to the first scan dir.
# new_project_dir = \"~/code\"

# Extra templates for `rattle new`, or overrides of the built-in ones
# (empty, rust, rust-lib, python, python-lib, node, go). {name} is the
# project name; the command runs in new_project_dir and must create {name}.
# templates.web = \"npm create vite@latest {name} -- --template react-ts\"

# Uncomment to override the defaults shown here.
# A folder counts as a project if it contains any of these.
# markers = [\".git\", \"Cargo.toml\", \"pyproject.toml\", \"package.json\", \"go.mod\",
#            \"pom.xml\", \"build.gradle\", \"CMakeLists.txt\", \"Makefile\"]
# Folder names that are never searched (hidden folders are always skipped).
# ignore = [\"node_modules\", \"target\", \"venv\", \"build\", \"dist\", \"vendor\",
#           \"Library\", \"Applications\"]
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub scan_dirs: Vec<String>,
    pub max_depth: usize,
    pub auto_scan: bool,
    pub markers: Vec<String>,
    pub ignore: Vec<String>,
    /// Overrides `$VISUAL` / `$EDITOR` when set.
    pub editor: Option<String>,
    pub delete_files: DeleteFiles,
    /// Where `rattle new` creates projects (default: the first scan dir).
    pub new_project_dir: Option<String>,
    /// User-defined `templates.<name> = "command"`, in file order.
    pub templates: Vec<(String, String)>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            scan_dirs: Vec::new(),
            max_depth: DEFAULT_MAX_DEPTH,
            auto_scan: true,
            markers: DEFAULT_MARKERS.iter().map(|m| m.to_string()).collect(),
            ignore: DEFAULT_IGNORE.iter().map(|m| m.to_string()).collect(),
            editor: None,
            delete_files: DeleteFiles::Off,
            new_project_dir: None,
            templates: Vec::new(),
        }
    }
}

/// The config lives next to the registry: `~/.rattle/config.toml` by default.
pub fn path_for(registry: &Path) -> PathBuf {
    registry.with_file_name("config.toml")
}

impl Config {
    /// Loads the config, writing a commented template if it is missing.
    pub fn load(file: &Path) -> io::Result<Self> {
        if !file.exists() {
            if let Some(dir) = file.parent() {
                fs::create_dir_all(dir)?;
            }
            fs::write(file, TEMPLATE)?;
        }
        Ok(Self::parse(&fs::read_to_string(file)?))
    }

    pub fn parse(text: &str) -> Self {
        let mut config = Self::default();
        let mut pending: Option<(String, String)> = None;
        // Current `[table]`, so `[templates]` + `rust = "..."` means `templates.rust`.
        let mut table = String::new();
        for line in text.lines() {
            let line = strip_comment(line).trim();
            if line.is_empty() {
                continue;
            }
            // Multi-line arrays: keep collecting until the closing bracket.
            let (key, value) = match pending.take() {
                Some((key, mut acc)) => {
                    acc.push_str(line);
                    (key, acc)
                }
                None => match line.split_once('=') {
                    Some((k, v)) if table.is_empty() => (k.trim().to_string(), v.trim().to_string()),
                    Some((k, v)) => (format!("{table}.{}", k.trim()), v.trim().to_string()),
                    None => {
                        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                            table = name.trim().to_string();
                        }
                        continue;
                    }
                },
            };
            if value.starts_with('[') && !value.ends_with(']') {
                pending = Some((key, value));
                continue;
            }
            match key.as_str() {
                "scan_dirs" => {
                    config.scan_dirs = parse_list(&value).into_iter().map(home_relative).collect()
                }
                "editor" => {
                    let editor = unquote(&value);
                    config.editor = (!editor.trim().is_empty()).then_some(editor);
                }
                "new_project_dir" => {
                    let dir = unquote(&value);
                    config.new_project_dir = (!dir.trim().is_empty()).then(|| home_relative(dir));
                }
                k if k.starts_with("templates.") => {
                    let name = unquote(&k["templates.".len()..]);
                    let command = unquote(&value);
                    config.templates.retain(|(n, _)| *n != name);
                    if !command.trim().is_empty() {
                        config.templates.push((name, command));
                    }
                }
                "delete_files" => {
                    // Anything unrecognised stays "off": never delete files by accident.
                    config.delete_files = DeleteFiles::parse(&unquote(&value)).unwrap_or_default();
                }
                "markers" => config.markers = parse_list(&value),
                "ignore" => config.ignore = parse_list(&value),
                "max_depth" => {
                    if let Ok(depth) = value.parse() {
                        config.max_depth = depth;
                    }
                }
                "auto_scan" => match value.as_str() {
                    "true" => config.auto_scan = true,
                    "false" => config.auto_scan = false,
                    _ => {}
                },
                _ => {}
            }
        }
        config
    }
}

fn parse_list(value: &str) -> Vec<String> {
    let inner = value
        .strip_prefix('[')
        .and_then(|v| v.strip_suffix(']'))
        .unwrap_or(value);
    inner
        .split(',')
        .map(|item| unquote(item.trim()))
        .filter(|item| !item.is_empty())
        .collect()
}

/// Relative scan dirs mean "under home", not "under wherever rattle was started".
fn home_relative(dir: String) -> String {
    if dir.starts_with('/') || dir.starts_with('~') {
        dir
    } else {
        format!("~/{dir}")
    }
}

/// Drops a trailing `# comment`, ignoring `#` inside quoted strings.
pub(crate) fn strip_comment(line: &str) -> &str {
    let mut in_str = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '#' if !in_str => return &line[..i],
            _ => {}
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_parses_to_defaults() {
        assert_eq!(Config::parse(TEMPLATE), Config::default());
    }

    #[test]
    fn parses_single_and_multi_line_arrays() {
        let c = Config::parse(
            "scan_dirs = [\"~/code\", \"/srv/#work\"] # roots\nmarkers = [\n  \".git\",\n  \"Cargo.toml\",\n]\n",
        );
        assert_eq!(c.scan_dirs, vec!["~/code", "/srv/#work"]);
        assert_eq!(c.markers, vec![".git", "Cargo.toml"]);
    }

    #[test]
    fn parses_scalars_and_ignores_bad_values() {
        let c = Config::parse("max_depth = 5\nauto_scan = false\nignore = [\"tmp\"]\n");
        assert_eq!((c.max_depth, c.auto_scan, c.ignore), (5, false, vec!["tmp".to_string()]));

        let c = Config::parse("max_depth = deep\nauto_scan = maybe\n");
        assert_eq!((c.max_depth, c.auto_scan), (DEFAULT_MAX_DEPTH, true));
    }

    #[test]
    fn parses_editor() {
        assert_eq!(Config::parse("editor = \"code -w\"\n").editor.as_deref(), Some("code -w"));
        assert_eq!(Config::parse("editor = \"\"\n").editor, None);
        assert_eq!(Config::parse("# editor = \"hx\"\n").editor, None);
    }

    #[test]
    fn delete_files_defaults_to_off_on_bad_values() {
        let parse = |s: &str| Config::parse(&format!("delete_files = {s}\n")).delete_files;
        assert_eq!(parse("\"trash\""), DeleteFiles::Trash);
        assert_eq!(parse("\"permanent\""), DeleteFiles::Permanent);
        assert_eq!(parse("\"yes\""), DeleteFiles::Off);
        assert_eq!(parse("true"), DeleteFiles::Off);
    }

    #[test]
    fn parses_templates_as_dotted_keys_and_table() {
        let c = Config::parse(
            "new_project_dir = \"code\"\ntemplates.web = \"npm create vite {name}\"\n\
             [templates]\nrust = \"cargo new --vcs none {name}\"\nweb = \"pnpm create vite {name}\"\n",
        );
        assert_eq!(c.new_project_dir.as_deref(), Some("~/code"));
        assert_eq!(
            c.templates,
            vec![
                ("rust".to_string(), "cargo new --vcs none {name}".to_string()),
                ("web".to_string(), "pnpm create vite {name}".to_string()),
            ]
        );
    }

    #[test]
    fn relative_scan_dirs_are_under_home() {
        let c = Config::parse("scan_dirs = [\"Documents\", \"~/code\", \"/srv\"]\n");
        assert_eq!(c.scan_dirs, vec!["~/Documents", "~/code", "/srv"]);
    }
}
