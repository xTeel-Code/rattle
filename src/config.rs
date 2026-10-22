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

# Dev environments (`v` in the TUI, `rattle env`): each is its own container,
# so e.g. work and personal projects never share one. Projects inside an
# environment's `dirs` (or listed in `projects`) belong to it and are mounted
# into its container. Create one with `rattle env create work`.
# [envs.work]
# dirs = [\"~/code/work\"]
# template = \"bindle\"     # built in: github.com/suckharder/bindle-le
# mount = true             # false: a plain box, code lives only inside it
#
# Environment files go to envs_dir (default: envs/ next to this file),
# `docker` is the container CLI used for them, and `browser` opens their
# links (default: the system's `open` / `xdg-open`).
# envs_dir = \"~/.rattle/envs\"
# docker = \"docker\"
# browser = \"open -a Firefox\"
#
# Every built-in template setting can be overridden, or a new template added:
# [env_templates.bindle]
# compose = \"~/templates/bindle/compose.yaml\"   # a URL or a local file
# port_base = 18000
# [env_templates.bindle.links]
# editor = \"http://127.0.0.1:${PORT_CODE}/?folder=${FOLDER}\"

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
    /// `[envs.<name>]` dev environments, in file order.
    pub envs: Vec<EnvDef>,
    /// `[env_templates.<name>]` settings as (template, field, raw value), e.g.
    /// ("bindle", "ports.PORT_CODE", "0"). Applied over the built-in templates.
    pub env_templates: Vec<(String, String, String)>,
    /// Where environment files live (default: `envs/` next to the registry).
    pub envs_dir: Option<String>,
    /// The container CLI: `docker` (default) or a compatible one.
    pub docker: String,
    /// Command that opens environment links (default: `open` / `xdg-open`).
    pub browser: Option<String>,
}

/// A dev environment from `[envs.<name>]`: one container that a group of
/// projects belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvDef {
    pub name: String,
    /// Which env template it is made from (default: "bindle").
    pub template: String,
    /// Projects inside these folders belong to this environment.
    pub dirs: Vec<String>,
    /// More projects by name.
    pub projects: Vec<String>,
    /// Mount those projects into the container (default: true).
    pub mount: bool,
}

impl EnvDef {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            template: "bindle".into(),
            dirs: Vec::new(),
            projects: Vec::new(),
            mount: true,
        }
    }
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
            envs: Vec::new(),
            env_templates: Vec::new(),
            envs_dir: None,
            docker: "docker".into(),
            browser: None,
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
                    Some((k, v)) if table.is_empty() => {
                        (k.trim().to_string(), v.trim().to_string())
                    }
                    Some((k, v)) => (format!("{table}.{}", k.trim()), v.trim().to_string()),
                    None => {
                        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']'))
                        {
                            table = name.trim().to_string();
                        }
                        continue;
                    }
                },
            };
            if value.starts_with('[') && bracket_balance(&value) > 0 {
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
                "envs_dir" => {
                    let dir = unquote(&value);
                    config.envs_dir = (!dir.trim().is_empty()).then(|| home_relative(dir));
                }
                "browser" => {
                    let browser = unquote(&value);
                    config.browser = (!browser.trim().is_empty()).then_some(browser);
                }
                "docker" => {
                    let docker = unquote(&value);
                    if !docker.trim().is_empty() {
                        config.docker = docker;
                    }
                }
                k if k.starts_with("envs.") => {
                    let Some((name, field)) = k["envs.".len()..].split_once('.') else {
                        continue;
                    };
                    let name = unquote(name);
                    let env = match config.envs.iter().position(|e| e.name == name) {
                        Some(i) => &mut config.envs[i],
                        None => {
                            config.envs.push(EnvDef::new(&name));
                            config.envs.last_mut().unwrap()
                        }
                    };
                    match field {
                        "template" => env.template = unquote(&value),
                        "dirs" => {
                            env.dirs = parse_list(&value).into_iter().map(home_relative).collect()
                        }
                        "projects" => env.projects = parse_list(&value),
                        "mount" => env.mount = value != "false",
                        _ => {}
                    }
                }
                k if k.starts_with("env_templates.") => {
                    if let Some((name, field)) = k["env_templates.".len()..].split_once('.') {
                        config.env_templates.push((
                            unquote(name),
                            field.to_string(),
                            unquote(&value),
                        ));
                    }
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

/// The temp file a config write goes through before its rename: per process,
/// so two rattles saving at once never share a half-written file.
fn tmp_path(file: &Path) -> PathBuf {
    file.with_extension(format!("toml.{}.tmp", std::process::id()))
}

/// Adds an `[envs.<name>]` section at the end of the config file.
pub fn append_env(file: &Path, env: &EnvDef) -> io::Result<()> {
    let mut text = fs::read_to_string(file).unwrap_or_default();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&format!(
        "\n[envs.{}]\ntemplate = {}\ndirs = {}\n",
        env.name,
        toml_string(&env.template),
        toml_list(&env.dirs)
    ));
    let tmp = tmp_path(file);
    fs::write(&tmp, text)?;
    fs::rename(tmp, file)
}

/// Writes `updates` (key, already-formatted TOML value) into the config file,
/// keeping comments and every other line as they are.
pub fn save_values(file: &Path, updates: &[(&str, String)]) -> io::Result<()> {
    let text = fs::read_to_string(file).unwrap_or_default();
    let tmp = tmp_path(file);
    fs::write(&tmp, set_values(&text, updates))?;
    fs::rename(tmp, file)
}

/// Replaces each top-level `key = …` line (including multi-line arrays) with
/// the new value, or adds it before the first `[table]` if the key is absent.
pub fn set_values(text: &str, updates: &[(&str, String)]) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    for (key, value) in updates {
        let new_line = format!("{key} = {value}");
        match find_top_level_key(&lines, key) {
            Some((start, end)) => {
                lines.splice(start..=end, [new_line]);
            }
            None => {
                // Next to its commented-out example (`# editor = "hx"`) if there is one.
                let header = first_table_header(&lines).unwrap_or(lines.len());
                let example = lines[..header].iter().position(|l| {
                    l.trim_start().strip_prefix('#').is_some_and(|rest| {
                        rest.trim_start()
                            .strip_prefix(key)
                            .is_some_and(|r| r.trim_start().starts_with('='))
                    })
                });
                lines.insert(example.map_or(header, |i| i + 1), new_line);
            }
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Line range (first, last) of `key`'s top-level assignment.
fn find_top_level_key(lines: &[String], key: &str) -> Option<(usize, usize)> {
    let mut depth = 0i32;
    for (i, raw) in lines.iter().enumerate() {
        let line = strip_comment(raw).trim();
        if depth > 0 {
            depth += bracket_balance(line);
            continue;
        }
        if is_table_header(line) {
            return None;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let balance = bracket_balance(v);
        if unquote(k.trim()) == *key {
            // Extend over the rest of a multi-line array.
            let mut end = i;
            let mut open = balance;
            while open > 0 && end + 1 < lines.len() {
                end += 1;
                open += bracket_balance(strip_comment(&lines[end]));
            }
            return Some((i, end));
        }
        depth = balance.max(0);
    }
    None
}

/// Line range of `[table]`: its header, and the end (exclusive) before the
/// next table header or the end of the file.
fn table_span(lines: &[String], table: &str) -> Option<(usize, usize)> {
    let header = lines.iter().position(|raw| {
        let line = strip_comment(raw).trim();
        is_table_header(line) && line[1..line.len() - 1].trim() == table
    })?;
    let end = first_table_header(&lines[header + 1..]).map_or(lines.len(), |i| header + 1 + i);
    Some((header, end))
}

/// Sets `key = value` inside `[table]`, replacing the existing assignment
/// (multi-line arrays included) or adding it after the header. `None` if
/// there is no such table.
pub fn set_table_value(text: &str, table: &str, key: &str, value: &str) -> Option<String> {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let (header, end) = table_span(&lines, table)?;
    let new_line = format!("{key} = {value}");
    match find_top_level_key(&lines[header + 1..end], key) {
        Some((start, last)) => {
            lines.splice(header + 1 + start..=header + 1 + last, [new_line]);
        }
        None => lines.insert(header + 1, new_line),
    }
    let mut out = lines.join("\n");
    out.push('\n');
    Some(out)
}

/// Removes `[table]` and everything in it (and a blank line left before it).
pub fn remove_table(text: &str, table: &str) -> Option<String> {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let (mut header, mut end) = table_span(&lines, table)?;
    // Keep the blank lines that separate it from the next table…
    while end > header + 1 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    // …and drop the one in front of it instead.
    if header > 0 && lines[header - 1].trim().is_empty() {
        header -= 1;
    }
    lines.drain(header..end);
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    Some(out)
}

/// [`set_table_value`] on the config file. Errors if the table is missing.
pub fn save_table_value(file: &Path, table: &str, key: &str, value: &str) -> io::Result<()> {
    let text = fs::read_to_string(file)?;
    let new = set_table_value(&text, table, key, value).ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, format!("no [{table}] in config"))
    })?;
    let tmp = tmp_path(file);
    fs::write(&tmp, new)?;
    fs::rename(tmp, file)
}

/// [`remove_table`] on the config file. Returns whether the table was there.
pub fn delete_table(file: &Path, table: &str) -> io::Result<bool> {
    let text = fs::read_to_string(file)?;
    let Some(new) = remove_table(&text, table) else {
        return Ok(false);
    };
    let tmp = tmp_path(file);
    fs::write(&tmp, new)?;
    fs::rename(tmp, file)?;
    Ok(true)
}

fn first_table_header(lines: &[String]) -> Option<usize> {
    let mut depth = 0i32;
    for (i, raw) in lines.iter().enumerate() {
        let line = strip_comment(raw).trim();
        if depth > 0 {
            depth += bracket_balance(line);
        } else if is_table_header(line) {
            return Some(i);
        } else if let Some((_, v)) = line.split_once('=') {
            depth = bracket_balance(v).max(0);
        }
    }
    None
}

fn is_table_header(line: &str) -> bool {
    line.starts_with('[') && line.ends_with(']') && !line.contains('=')
}

/// `[` minus `]` outside of strings.
pub(crate) fn bracket_balance(s: &str) -> i32 {
    let mut n = 0;
    outside_strings(s, |_, c| {
        match c {
            '[' => n += 1,
            ']' => n -= 1,
            _ => {}
        }
        true
    });
    n
}

/// Calls `f(index, char)` for every character of `s` that is outside TOML
/// strings ("…" with `\` escapes, or '…'); `f` returns false to stop. The one
/// place that knows TOML quoting, for comments, brackets and lists alike.
fn outside_strings(s: &str, mut f: impl FnMut(usize, char) -> bool) {
    let (mut quote, mut escaped) = (None::<char>, false);
    for (i, c) in s.char_indices() {
        match quote {
            Some(_) if escaped => escaped = false,
            Some('"') if c == '\\' => escaped = true,
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None => {
                if !f(i, c) {
                    return;
                }
            }
        }
    }
}

/// The strings of a TOML array (comments already removed), e.g.
/// `["a, b", "c\"d"]` → `a, b` and `c"d`. Strings inside inline tables
/// (`{ include-group = "dev" }`) are skipped.
pub(crate) fn string_array(text: &str) -> Vec<String> {
    let (mut items, mut current) = (Vec::new(), String::new());
    let (mut quote, mut escaped, mut braces) = (None::<char>, false, 0i32);
    for c in text.chars() {
        match quote {
            Some(_) if escaped => {
                current.push(c);
                escaped = false;
            }
            Some('"') if c == '\\' => escaped = true,
            Some(q) if c == q => {
                if braces == 0 {
                    items.push(std::mem::take(&mut current));
                }
                current.clear();
                quote = None;
            }
            Some(_) => current.push(c),
            None => match c {
                '"' | '\'' => quote = Some(c),
                '{' => braces += 1,
                '}' => braces -= 1,
                _ => {}
            },
        }
    }
    items
}

/// A TOML string literal.
pub fn toml_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// A TOML array of strings.
pub fn toml_list(items: &[String]) -> String {
    let items: Vec<String> = items.iter().map(|i| toml_string(i)).collect();
    format!("[{}]", items.join(", "))
}

/// `["a", "b, c"]` → `a`, `b, c` (a lone string → itself), empty ones dropped.
fn parse_list(value: &str) -> Vec<String> {
    let items = if value.trim_start().starts_with('[') {
        string_array(value)
    } else {
        vec![unquote(value.trim())]
    };
    items.into_iter().filter(|item| !item.is_empty()).collect()
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
    let mut end = line.len();
    outside_strings(line, |i, c| {
        if c == '#' {
            end = i;
        }
        c != '#'
    });
    &line[..end]
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
        assert_eq!(
            (c.max_depth, c.auto_scan, c.ignore),
            (5, false, vec!["tmp".to_string()])
        );

        let c = Config::parse("max_depth = deep\nauto_scan = maybe\n");
        assert_eq!((c.max_depth, c.auto_scan), (DEFAULT_MAX_DEPTH, true));
    }

    #[test]
    fn parses_editor() {
        assert_eq!(
            Config::parse("editor = \"code -w\"\n").editor.as_deref(),
            Some("code -w")
        );
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
                (
                    "rust".to_string(),
                    "cargo new --vcs none {name}".to_string()
                ),
                ("web".to_string(), "pnpm create vite {name}".to_string()),
            ]
        );
    }

    #[test]
    fn set_values_keeps_comments_and_handles_multiline_arrays() {
        let text = "# my roots\nscan_dirs = [\n  \"~/a\",\n  \"~/b\",\n]\n# depth\nmax_depth = 3 # comment\n\
                    # editor = \"hx\"\n[templates]\nweb = \"x {name}\"\n";
        let out = set_values(
            text,
            &[
                ("scan_dirs", toml_list(&["~/code".to_string()])),
                ("max_depth", "5".into()),
                ("editor", toml_string("zed")),
            ],
        );
        assert_eq!(
            out,
            "# my roots\nscan_dirs = [\"~/code\"]\n# depth\nmax_depth = 5\n# editor = \"hx\"\n\
             editor = \"zed\"\n[templates]\nweb = \"x {name}\"\n"
        );
        let c = Config::parse(&out);
        assert_eq!(
            (c.scan_dirs, c.max_depth, c.editor.as_deref()),
            (vec!["~/code".to_string()], 5, Some("zed"))
        );
        assert_eq!(
            c.templates,
            vec![("web".to_string(), "x {name}".to_string())]
        );
    }

    #[test]
    fn set_values_inserts_below_commented_example() {
        let out = set_values(
            "# Editor help.\n# editor = \"hx\"\n\n# editors_note = 1\nmax_depth = 3\n",
            &[("editor", toml_string("zed"))],
        );
        assert_eq!(out, "# Editor help.\n# editor = \"hx\"\neditor = \"zed\"\n\n# editors_note = 1\nmax_depth = 3\n");
    }

    #[test]
    fn set_values_appends_when_missing_and_ignores_table_keys() {
        let out = set_values(
            "[templates]\nauto_scan = \"not top level\"\n",
            &[("auto_scan", "false".into())],
        );
        assert_eq!(
            out,
            "auto_scan = false\n[templates]\nauto_scan = \"not top level\"\n"
        );
        assert_eq!(
            set_values("", &[("max_depth", "2".into())]),
            "max_depth = 2\n"
        );
    }

    #[test]
    fn parses_envs_and_env_template_overrides() {
        let c = Config::parse(
            "docker = \"podman\"\nenvs_dir = \"boxes\"\n\
             [envs.work]\ndirs = [\n  \"code/work\",\n  \"/srv/jobs\",\n]\nprojects = [\"infra\"]\n\
             [envs.personal]\ntemplate = \"mine\"\nmount = false\n\
             [env_templates.bindle]\nport_base = 19000\n\
             [env_templates.bindle.ports]\nPORT_CODE = 0\n",
        );
        assert_eq!(c.docker, "podman");
        assert_eq!(c.envs_dir.as_deref(), Some("~/boxes"));
        let work = EnvDef {
            dirs: vec!["~/code/work".into(), "/srv/jobs".into()],
            projects: vec!["infra".into()],
            ..EnvDef::new("work")
        };
        let personal = EnvDef {
            template: "mine".into(),
            mount: false,
            ..EnvDef::new("personal")
        };
        assert_eq!(c.envs, vec![work, personal]);
        assert_eq!(
            c.env_templates,
            vec![
                (
                    "bindle".to_string(),
                    "port_base".to_string(),
                    "19000".to_string()
                ),
                (
                    "bindle".to_string(),
                    "ports.PORT_CODE".to_string(),
                    "0".to_string()
                ),
            ]
        );
    }

    #[test]
    fn table_values_are_set_and_tables_removed_in_place() {
        let text =
            "scan_dirs = [\"code\"]\n\n[envs.work]\n# my work stuff\ntemplate = \"bindle\"\n\
                    dirs = [\n  \"~/a\",\n  \"~/b\",\n]\n\n[envs.personal]\ndirs = [\"~/side\"]\n";
        let out = set_table_value(text, "envs.work", "dirs", "[\"~/code/work\"]").unwrap();
        assert_eq!(
            out,
            "scan_dirs = [\"code\"]\n\n[envs.work]\n# my work stuff\ntemplate = \"bindle\"\n\
             dirs = [\"~/code/work\"]\n\n[envs.personal]\ndirs = [\"~/side\"]\n"
        );
        // Missing key: added right after the header; other tables untouched.
        let out = set_table_value(&out, "envs.personal", "mount", "false").unwrap();
        assert!(
            out.contains("[envs.personal]\nmount = false\ndirs = [\"~/side\"]\n"),
            "{out}"
        );
        let c = Config::parse(&out);
        assert_eq!(c.envs[0].dirs, vec!["~/code/work"]);
        assert!(!c.envs[1].mount);
        assert!(set_table_value(&out, "envs.ghost", "dirs", "[]").is_none());

        let removed = remove_table(&out, "envs.work").unwrap();
        assert_eq!(
            removed,
            "scan_dirs = [\"code\"]\n\n[envs.personal]\nmount = false\ndirs = [\"~/side\"]\n"
        );
        assert_eq!(
            remove_table(&removed, "envs.personal").unwrap(),
            "scan_dirs = [\"code\"]\n"
        );
        assert!(remove_table(&removed, "envs.ghost").is_none());
    }

    #[test]
    fn lists_keep_commas_escaped_quotes_and_brackets_inside_strings() {
        let c = Config::parse(
            "scan_dirs = [\"~/My, Stuff\", \"~/a \\\"b\\\" # c\"]  # real comment\n\
             ignore = [\n  \"x]y\",   # a ] in a string\n  \"z\",\n]\n\
             editor = \"code \\\"--wait\\\" # not a comment\"\n",
        );
        assert_eq!(c.scan_dirs, vec!["~/My, Stuff", "~/a \"b\" # c"]);
        assert_eq!(c.ignore, vec!["x]y", "z"]);
        assert_eq!(c.editor.as_deref(), Some("code \"--wait\" # not a comment"));
        assert_eq!(strip_comment("a = \"x\\\"#y\" # gone"), "a = \"x\\\"#y\" ");
    }

    #[test]
    fn relative_scan_dirs_are_under_home() {
        let c = Config::parse("scan_dirs = [\"Documents\", \"~/code\", \"/srv\"]\n");
        assert_eq!(c.scan_dirs, vec!["~/Documents", "~/code", "/srv"]);
    }
}
