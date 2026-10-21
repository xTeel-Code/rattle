//! Creating new projects from templates (`rattle new`, `n` in the TUI).
//!
//! A template is a shell command run in the parent folder; `{name}` is
//! replaced by the project name and the command must create `{name}/`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::config::Config;
use crate::ops;
use crate::store::{Project, Store};

/// Built-in templates; `templates.<name>` in the config overrides or extends them.
pub const BUILTIN: &[(&str, &str)] = &[
    ("empty", "mkdir {name} && git init -q {name}"),
    ("rust", "cargo new {name}"),
    ("rust-lib", "cargo new --lib {name}"),
    ("python", "uv init {name}"),
    ("python-lib", "uv init --lib {name}"),
    (
        "node",
        "mkdir {name} && cd {name} && npm init -y >/dev/null && git init -q",
    ),
    (
        "go",
        "mkdir {name} && cd {name} && go mod init {name} && git init -q",
    ),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    pub name: String,
    pub command: String,
}

/// Built-ins first (with config overrides applied), then the user's own.
pub fn all(config: &Config) -> Vec<Template> {
    let custom = |name: &str| {
        config
            .templates
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, c)| c.clone())
    };
    let mut list: Vec<Template> = BUILTIN
        .iter()
        .map(|(name, cmd)| Template {
            name: name.to_string(),
            command: custom(name).unwrap_or(cmd.to_string()),
        })
        .collect();
    for (name, command) in &config.templates {
        if !BUILTIN.iter().any(|(b, _)| b == name) {
            list.push(Template {
                name: name.clone(),
                command: command.clone(),
            });
        }
    }
    list
}

pub fn find(config: &Config, name: &str) -> Result<Template, String> {
    let list = all(config);
    let names: Vec<&str> = list.iter().map(|t| t.name.as_str()).collect();
    list.iter()
        .find(|t| t.name == name)
        .cloned()
        .ok_or_else(|| {
            format!(
                "unknown template '{name}' (available: {})",
                names.join(", ")
            )
        })
}

/// Project names become folder names and are substituted into shell commands.
pub fn validate_name(name: &str) -> Result<(), String> {
    let ok_chars = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if name.is_empty() || !ok_chars || name.starts_with(['.', '-']) {
        return Err(format!(
            "invalid project name '{name}': use letters, digits, '-', '_' or '.', not starting with '.' or '-'"
        ));
    }
    Ok(())
}

/// Folder new projects go into: `explicit`, else `new_project_dir`, else the first scan dir.
pub fn parent_dir(config: &Config, explicit: Option<&str>) -> Result<PathBuf, String> {
    let raw = explicit
        .map(str::to_string)
        .or_else(|| config.new_project_dir.clone())
        .or_else(|| config.scan_dirs.first().cloned())
        .ok_or("no folder for new projects; set new_project_dir in config or pass --in <folder>")?;
    Ok(ops::expand_tilde(&raw))
}

/// Runs `template` to create `parent/name`, then registers it in `store`
/// (the caller saves). Fails if the folder already exists or the command fails.
pub fn create(
    store: &mut Store,
    template: &Template,
    name: &str,
    parent: &Path,
) -> Result<Project, String> {
    validate_name(name)?;
    if store.find(name).is_some() {
        return Err(format!("a project named '{name}' is already registered"));
    }
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    let target = parent.join(name);
    if target.exists() {
        return Err(format!("{} already exists", target.display()));
    }

    // validate_name() keeps the name shell-safe; quote it anyway.
    let command = template.command.replace("{name}", &format!("'{name}'"));
    let output = Command::new("sh")
        .arg("-c")
        .arg(&command)
        .current_dir(parent)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run template '{}': {e}", template.name))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("no error output");
        return Err(format!(
            "template '{}' failed ({}): {reason}",
            template.name, output.status
        ));
    }
    if !target.is_dir() {
        return Err(format!(
            "template '{}' finished but did not create {}",
            template.name,
            target.display()
        ));
    }

    let project = ops::resolve_project(&target.to_string_lossy())?;
    store.add(project.clone())?;
    Ok(project)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_overrides_and_extends_builtins() {
        let config = Config {
            templates: vec![
                ("rust".into(), "cargo new --vcs none {name}".into()),
                ("web".into(), "x {name}".into()),
            ],
            ..Config::default()
        };
        let list = all(&config);
        assert_eq!(list.len(), BUILTIN.len() + 1);
        assert_eq!(
            find(&config, "rust").unwrap().command,
            "cargo new --vcs none {name}"
        );
        assert_eq!(find(&config, "web").unwrap().command, "x {name}");
        assert!(find(&config, "nope")
            .unwrap_err()
            .contains("available: empty, rust"));
    }

    #[test]
    fn rejects_unsafe_names() {
        for bad in ["", ".hidden", "-rf", "a b", "a/b", "x;rm", "$(id)"] {
            assert!(validate_name(bad).is_err(), "{bad:?} should be rejected");
        }
        for good in ["app", "my-app_2", "v1.0"] {
            assert!(validate_name(good).is_ok(), "{good:?} should be accepted");
        }
    }

    #[test]
    fn creates_and_registers_project() {
        let parent = std::env::temp_dir().join(format!("rattle-test-new-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&parent);
        let mut store = Store::default();
        let make = Template {
            name: "t".into(),
            command: "mkdir {name} && touch {name}/README".into(),
        };

        let p = create(&mut store, &make, "demo", &parent).unwrap();
        assert_eq!(p.name, "demo");
        assert!(parent.join("demo/README").exists());
        assert_eq!(store.projects.len(), 1);

        assert!(create(&mut store, &make, "demo", &parent)
            .unwrap_err()
            .contains("already registered"));
        let fail = Template {
            name: "f".into(),
            command: "echo boom >&2; exit 3".into(),
        };
        assert!(create(&mut store, &fail, "other", &parent)
            .unwrap_err()
            .contains("boom"));
        let lazy = Template {
            name: "l".into(),
            command: "true".into(),
        };
        assert!(create(&mut store, &lazy, "lazy", &parent)
            .unwrap_err()
            .contains("did not create"));
        std::fs::remove_dir_all(parent).unwrap();
    }
}
