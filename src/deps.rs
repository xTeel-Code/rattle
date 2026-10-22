//! A project's dependencies: listed by reading its manifest, added and
//! removed through its own package manager (`cargo add`, `npm install`, …).

use std::path::Path;
use std::process::{Command, Output};

use crate::config::{bracket_balance, string_array, strip_comment};
use crate::store::unquote;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Manager {
    Cargo,
    Npm,
    Pnpm,
    Yarn,
    Bun,
    Uv,
    Poetry,
    Go,
}

impl Manager {
    pub fn name(self) -> &'static str {
        match self {
            Self::Cargo => "cargo",
            Self::Npm => "npm",
            Self::Pnpm => "pnpm",
            Self::Yarn => "yarn",
            Self::Bun => "bun",
            Self::Uv => "uv",
            Self::Poetry => "poetry",
            Self::Go => "go",
        }
    }

    pub fn manifest(self) -> &'static str {
        match self {
            Self::Cargo => "Cargo.toml",
            Self::Npm | Self::Pnpm | Self::Yarn | Self::Bun => "package.json",
            Self::Uv | Self::Poetry => "pyproject.toml",
            Self::Go => "go.mod",
        }
    }
}

/// Picks the package manager from the project's files (lockfiles decide between
/// npm/pnpm/yarn/bun and uv/poetry).
pub fn detect(dir: &Path) -> Option<Manager> {
    let has = |f: &str| dir.join(f).exists();
    if has("Cargo.toml") {
        Some(Manager::Cargo)
    } else if has("package.json") {
        Some(if has("pnpm-lock.yaml") {
            Manager::Pnpm
        } else if has("yarn.lock") {
            Manager::Yarn
        } else if has("bun.lockb") || has("bun.lock") {
            Manager::Bun
        } else {
            Manager::Npm
        })
    } else if has("pyproject.toml") {
        let poetry = has("poetry.lock")
            || std::fs::read_to_string(dir.join("pyproject.toml"))
                .is_ok_and(|t| t.contains("[tool.poetry"));
        Some(if poetry { Manager::Poetry } else { Manager::Uv })
    } else if has("go.mod") {
        Some(Manager::Go)
    } else {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Group {
    Normal,
    Dev,
    Build,
    /// A named dependency group (uv `[dependency-groups]`, poetry groups).
    Named(String),
    /// An optional extra (`[project.optional-dependencies]`, npm optional/peer).
    Optional(String),
    /// Listed but not removable as a group (cargo target deps, go indirect).
    Other(String),
}

impl Group {
    pub fn label(&self) -> String {
        match self {
            Self::Normal => String::new(),
            Self::Dev => "dev".into(),
            Self::Build => "build".into(),
            Self::Named(g) => format!("group {g}"),
            Self::Optional(g) => format!("optional {g}"),
            Self::Other(g) => g.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dep {
    pub name: String,
    pub spec: String,
    pub group: Group,
}

pub fn list(dir: &Path, manager: Manager) -> Result<Vec<Dep>, String> {
    let path = dir.join(manager.manifest());
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok(match manager {
        Manager::Cargo => parse_cargo(&text),
        Manager::Npm | Manager::Pnpm | Manager::Yarn | Manager::Bun => parse_package_json(&text)?,
        Manager::Uv | Manager::Poetry => parse_pyproject(&text),
        Manager::Go => parse_go_mod(&text),
    })
}

/// The command that adds `packages` (as dev dependencies with `dev`).
pub fn add_command(manager: Manager, dir: &Path, packages: &[String], dev: bool) -> Command {
    let (program, mut args): (&str, Vec<&str>) = match manager {
        Manager::Cargo => ("cargo", vec!["add"]),
        Manager::Npm => ("npm", vec!["install"]),
        Manager::Pnpm => ("pnpm", vec!["add"]),
        Manager::Yarn => ("yarn", vec!["add"]),
        Manager::Bun => ("bun", vec!["add"]),
        Manager::Uv => ("uv", vec!["add"]),
        Manager::Poetry => ("poetry", vec!["add"]),
        Manager::Go => ("go", vec!["get"]),
    };
    if dev {
        args.extend(match manager {
            Manager::Npm => vec!["--save-dev"],
            Manager::Pnpm => vec!["-D"],
            Manager::Poetry => vec!["--group", "dev"],
            Manager::Go => vec![], // Go has no dev dependencies.
            _ => vec!["--dev"],
        });
    }
    let mut cmd = Command::new(program);
    cmd.args(args).args(packages).current_dir(dir);
    cmd
}

/// The command that removes `name` from the group it was listed in.
pub fn remove_command(manager: Manager, dir: &Path, name: &str, group: &Group) -> Command {
    let mut cmd = Command::new(manager.name());
    cmd.current_dir(dir);
    match manager {
        Manager::Cargo => {
            cmd.args(["remove", name]);
            match group {
                Group::Dev => cmd.arg("--dev"),
                Group::Build => cmd.arg("--build"),
                _ => &mut cmd,
            };
        }
        Manager::Npm => {
            cmd.args(["uninstall", name]);
        }
        Manager::Pnpm | Manager::Yarn | Manager::Bun => {
            cmd.args(["remove", name]);
        }
        Manager::Uv => {
            cmd.args(["remove", name]);
            match group {
                Group::Dev => cmd.arg("--dev"),
                Group::Named(g) => cmd.args(["--group", g]),
                Group::Optional(g) => cmd.args(["--optional", g]),
                _ => &mut cmd,
            };
        }
        Manager::Poetry => {
            cmd.args(["remove", name]);
            match group {
                Group::Dev => cmd.args(["--group", "dev"]),
                Group::Named(g) => cmd.args(["--group", g]),
                _ => &mut cmd,
            };
        }
        Manager::Go => {
            cmd.args(["get", &format!("{name}@none")]);
        }
    }
    cmd
}

/// "cargo add serde --dev", for status messages.
pub fn describe(cmd: &Command) -> String {
    let mut parts = vec![cmd.get_program().to_string_lossy().into_owned()];
    parts.extend(cmd.get_args().map(|a| a.to_string_lossy().into_owned()));
    parts.join(" ")
}

/// Sums up a finished package-manager run: its last output line on success,
/// or the last error line on failure.
pub fn summarize(program: &str, output: &Output) -> Result<String, String> {
    let last_line = |bytes: &[u8]| {
        String::from_utf8_lossy(bytes)
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .map(|l| l.trim().to_string())
    };
    if output.status.success() {
        Ok(last_line(&output.stderr)
            .or_else(|| last_line(&output.stdout))
            .unwrap_or_else(|| "done".into()))
    } else {
        let reason = last_line(&output.stderr)
            .or_else(|| last_line(&output.stdout))
            .unwrap_or_default();
        Err(format!("{program} failed ({}): {reason}", output.status))
    }
}

fn parse_cargo(text: &str) -> Vec<Dep> {
    let mut deps = Vec::new();
    let mut section: Option<Group> = None;
    // `[dependencies.serde]` tables: the name, its group, and the version once seen.
    let mut table: Option<(String, Group, String)> = None;
    let flush = |table: &mut Option<(String, Group, String)>, deps: &mut Vec<Dep>| {
        if let Some((name, group, spec)) = table.take() {
            deps.push(Dep { name, spec, group });
        }
    };
    let group_of = |kind: &str| match kind {
        "dependencies" => Some(Group::Normal),
        "dev-dependencies" => Some(Group::Dev),
        "build-dependencies" => Some(Group::Build),
        _ => None,
    };
    for raw in text.lines() {
        let line = strip_comment(raw).trim();
        if let Some(header) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            flush(&mut table, &mut deps);
            let header = header.trim();
            section = None;
            if let Some(g) = group_of(header) {
                section = Some(g);
            } else if let Some((kind, name)) = header
                .split_once('.')
                .filter(|(k, _)| group_of(k).is_some())
            {
                table = Some((unquote(name), group_of(kind).unwrap(), "*".into()));
            } else if header.starts_with("target.")
                && group_of(header.rsplit('.').next().unwrap_or("")).is_some()
            {
                section = Some(Group::Other("target".into()));
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (unquote(key.trim()), value.trim());
        if let Some((_, _, spec)) = &mut table {
            if key == "version" {
                *spec = unquote(value);
            }
        } else if let Some(group) = &section {
            // Dotted keys (`serde.workspace = true`, `serde.version = "1"`) are
            // fields of dependency `serde`, not dependencies of their own.
            let (name, field) = match key.split_once('.') {
                Some((name, field)) => (unquote(name), Some(field)),
                None => (key.clone(), None),
            };
            let spec = match field {
                None => cargo_spec(value),
                Some("version") => unquote(value),
                Some("workspace") => "workspace".into(),
                Some(_) => "*".into(),
            };
            match deps
                .iter_mut()
                .find(|d| d.name == name && d.group == *group)
            {
                // A later field refines the spec only if it says something.
                Some(dep) if spec != "*" => dep.spec = spec,
                Some(_) => {}
                None => deps.push(Dep {
                    name,
                    spec,
                    group: group.clone(),
                }),
            }
        }
    }
    flush(&mut table, &mut deps);
    deps
}

/// `"1.0"` → `1.0`; `{ version = "1", features = [..] }` → `1`; path/git deps say so.
fn cargo_spec(value: &str) -> String {
    if value.starts_with('"') {
        return unquote(value);
    }
    let field = |name: &str| {
        let rest = &value[value.find(name)? + name.len()..];
        let rest = rest
            .trim_start()
            .strip_prefix('=')?
            .trim_start()
            .strip_prefix('"')?;
        Some(rest[..rest.find('"')?].to_string())
    };
    field("version")
        .or_else(|| field("path").map(|p| format!("path {p}")))
        .or_else(|| field("git").map(|g| format!("git {g}")))
        .or_else(|| value.contains("workspace").then(|| "workspace".into()))
        .unwrap_or_else(|| "*".into())
}

fn parse_package_json(text: &str) -> Result<Vec<Dep>, String> {
    let json: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("invalid package.json: {e}"))?;
    let sections = [
        ("dependencies", Group::Normal),
        ("devDependencies", Group::Dev),
        ("peerDependencies", Group::Optional("peer".into())),
        ("optionalDependencies", Group::Optional("optional".into())),
    ];
    let mut deps = Vec::new();
    for (field, group) in sections {
        if let Some(map) = json.get(field).and_then(|v| v.as_object()) {
            for (name, spec) in map {
                let spec = spec.as_str().unwrap_or("*").to_string();
                deps.push(Dep {
                    name: name.clone(),
                    spec,
                    group: group.clone(),
                });
            }
        }
    }
    Ok(deps)
}

fn parse_pyproject(text: &str) -> Vec<Dep> {
    let lines: Vec<&str> = text.lines().collect();
    let mut deps = Vec::new();
    let mut table = String::new();
    let mut i = 0;
    while i < lines.len() {
        let line = strip_comment(lines[i]).trim();
        i += 1;
        if let Some(header) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            table = header.trim().to_string();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (unquote(key.trim()), value.trim());

        // PEP 621 / PEP 735: arrays of requirement strings.
        let array_group = match (table.as_str(), key.as_str()) {
            ("project", "dependencies") => Some(Group::Normal),
            ("project.optional-dependencies", extra) => Some(Group::Optional(extra.to_string())),
            ("dependency-groups", "dev") => Some(Group::Dev),
            ("dependency-groups", group) => Some(Group::Named(group.to_string())),
            _ => None,
        };
        if let Some(group) = array_group {
            for req in take_string_array(value, &lines, &mut i) {
                let (name, spec) = split_requirement(&req);
                deps.push(Dep {
                    name,
                    spec,
                    group: group.clone(),
                });
            }
            continue;
        }

        // Poetry: `name = "^1.0"` or `name = { version = "^1.0", … }` tables.
        let poetry_group = match table.as_str() {
            "tool.poetry.dependencies" => Some(Group::Normal),
            "tool.poetry.dev-dependencies" => Some(Group::Dev),
            t => t
                .strip_prefix("tool.poetry.group.")
                .and_then(|t| t.strip_suffix(".dependencies"))
                .map(|g| {
                    if g == "dev" {
                        Group::Dev
                    } else {
                        Group::Named(g.to_string())
                    }
                }),
        };
        if let Some(group) = poetry_group {
            if key != "python" {
                deps.push(Dep {
                    name: key,
                    spec: cargo_spec(value),
                    group,
                });
            }
        }
    }
    deps
}

/// Collects the strings of a TOML array that starts in `first` and may
/// continue on the following lines (advancing `i` past them).
fn take_string_array(first: &str, lines: &[&str], i: &mut usize) -> Vec<String> {
    let mut text = strip_comment(first).to_string();
    while bracket_balance(&text) > 0 && *i < lines.len() {
        text.push('\n');
        text.push_str(strip_comment(lines[*i]));
        *i += 1;
    }
    string_array(&text)
}

/// `requests[socks]>=2.31; python_version < "3.13"` → (`requests`, `[socks]>=2.31; …`).
fn split_requirement(req: &str) -> (String, String) {
    let req = req.trim();
    let end = req
        .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
        .unwrap_or(req.len());
    (req[..end].to_string(), req[end..].trim().to_string())
}

fn parse_go_mod(text: &str) -> Vec<Dep> {
    let mut deps = Vec::new();
    let mut in_block = false;
    for raw in text.lines() {
        let line = raw.trim();
        let entry = if in_block {
            if line.starts_with(')') {
                in_block = false;
                continue;
            }
            line
        } else if let Some(rest) = line.strip_prefix("require") {
            let rest = rest.trim();
            if rest.starts_with('(') {
                in_block = true;
                continue;
            }
            rest
        } else {
            continue;
        };
        let (entry, indirect) = match entry.split_once("//") {
            Some((e, comment)) => (e.trim(), comment.contains("indirect")),
            None => (entry, false),
        };
        let mut parts = entry.split_whitespace();
        if let (Some(name), Some(version)) = (parts.next(), parts.next()) {
            let group = if indirect {
                Group::Other("indirect".into())
            } else {
                Group::Normal
            };
            deps.push(Dep {
                name: name.into(),
                spec: version.into(),
                group,
            });
        }
    }
    deps
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dep(name: &str, spec: &str, group: Group) -> Dep {
        Dep {
            name: name.into(),
            spec: spec.into(),
            group,
        }
    }

    #[test]
    fn parses_cargo_toml() {
        let deps = parse_cargo(
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n\n[dependencies]\nserde = { version = \"1.0\", features = [\"derive\"] }\n\
             anyhow = \"1\" # errors\nlocal = { path = \"../local\" }\n\n[dev-dependencies]\ntempfile = \"3\"\n\n\
             [dependencies.tokio]\nversion = \"1.40\"\nfeatures = [\"full\"]\n\n[target.'cfg(unix)'.dependencies]\nlibc = \"0.2\"\n",
        );
        assert_eq!(
            deps,
            vec![
                dep("serde", "1.0", Group::Normal),
                dep("anyhow", "1", Group::Normal),
                dep("local", "path ../local", Group::Normal),
                dep("tempfile", "3", Group::Dev),
                dep("tokio", "1.40", Group::Normal),
                dep("libc", "0.2", Group::Other("target".into())),
            ]
        );
    }

    #[test]
    fn parses_dotted_cargo_keys_as_one_dependency() {
        let deps = parse_cargo(
            "[dependencies]\nserde.workspace = true\nregex.version = \"1.10\"\nregex.features = [\"std\"]\n\
             [dev-dependencies]\nserde.workspace = true\n",
        );
        assert_eq!(
            deps,
            vec![
                dep("serde", "workspace", Group::Normal),
                dep("regex", "1.10", Group::Normal),
                dep("serde", "workspace", Group::Dev),
            ]
        );
        // Removing uses the real name, not "serde.workspace".
        assert_eq!(
            describe(&remove_command(
                Manager::Cargo,
                Path::new("."),
                &deps[0].name,
                &deps[0].group
            )),
            "cargo remove serde"
        );
    }

    #[test]
    fn parses_package_json() {
        let deps = parse_package_json(
            r#"{"name":"x","dependencies":{"react":"^18.3.1"},"devDependencies":{"vite":"^5.4.0"}}"#,
        )
        .unwrap();
        assert_eq!(
            deps,
            vec![
                dep("react", "^18.3.1", Group::Normal),
                dep("vite", "^5.4.0", Group::Dev)
            ]
        );
        assert!(parse_package_json("{ not json").is_err());
    }

    #[test]
    fn parses_pyproject_uv_and_poetry() {
        let uv = parse_pyproject(
            "[project]\nname = \"x\"\ndependencies = [\n    \"requests[socks]>=2.31\",  # http\n    \"rich\",\n]\n\n\
             [project.optional-dependencies]\nfast = [\"orjson>=3\"]\n\n\
             [dependency-groups]\ndev = [\"pytest>=8\", { include-group = \"lint\" }]\nlint = [\"ruff\"]\n",
        );
        assert_eq!(
            uv,
            vec![
                dep("requests", "[socks]>=2.31", Group::Normal),
                dep("rich", "", Group::Normal),
                dep("orjson", ">=3", Group::Optional("fast".into())),
                dep("pytest", ">=8", Group::Dev),
                dep("ruff", "", Group::Named("lint".into())),
            ]
        );
        let poetry = parse_pyproject(
            "[tool.poetry.dependencies]\npython = \"^3.12\"\nhttpx = \"^0.27\"\n\n\
             [tool.poetry.group.dev.dependencies]\npytest = { version = \"^8.0\" }\n",
        );
        assert_eq!(
            poetry,
            vec![
                dep("httpx", "^0.27", Group::Normal),
                dep("pytest", "^8.0", Group::Dev)
            ]
        );
    }

    #[test]
    fn parses_go_mod() {
        let deps = parse_go_mod(
            "module example.com/x\n\ngo 1.23\n\nrequire github.com/spf13/cobra v1.8.1\n\nrequire (\n\
             \tgolang.org/x/sync v0.8.0\n\tgithub.com/inconshreveable/mousetrap v1.1.0 // indirect\n)\n",
        );
        assert_eq!(
            deps,
            vec![
                dep("github.com/spf13/cobra", "v1.8.1", Group::Normal),
                dep("golang.org/x/sync", "v0.8.0", Group::Normal),
                dep(
                    "github.com/inconshreveable/mousetrap",
                    "v1.1.0",
                    Group::Other("indirect".into())
                ),
            ]
        );
    }

    #[test]
    fn builds_add_and_remove_commands() {
        let d = Path::new(".");
        let pkgs = vec!["serde".to_string()];
        assert_eq!(
            describe(&add_command(Manager::Cargo, d, &pkgs, true)),
            "cargo add --dev serde"
        );
        assert_eq!(
            describe(&add_command(Manager::Npm, d, &pkgs, true)),
            "npm install --save-dev serde"
        );
        assert_eq!(
            describe(&add_command(Manager::Poetry, d, &pkgs, true)),
            "poetry add --group dev serde"
        );
        assert_eq!(
            describe(&add_command(Manager::Go, d, &pkgs, true)),
            "go get serde"
        );
        assert_eq!(
            describe(&remove_command(Manager::Cargo, d, "x", &Group::Build)),
            "cargo remove x --build"
        );
        assert_eq!(
            describe(&remove_command(
                Manager::Uv,
                d,
                "x",
                &Group::Optional("fast".into())
            )),
            "uv remove x --optional fast"
        );
        assert_eq!(
            describe(&remove_command(Manager::Go, d, "a/b", &Group::Normal)),
            "go get a/b@none"
        );
    }
}
