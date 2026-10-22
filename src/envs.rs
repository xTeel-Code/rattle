//! Dev environments: one container per group of projects (`[envs.<name>]`),
//! made from an env template and run with `docker compose`.
//!
//! An environment's files live in `<envs_dir>/<name>/`: the template's
//! `compose.yaml` and `.env` (with a unique name and free ports filled in),
//! plus a `compose.override.yaml` that Rattle regenerates on every start to
//! mount the environment's projects into the container.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::config::{Config, EnvDef};
use crate::ops::expand_tilde;
use crate::store::{Project, Store};

/// How to make and reach one kind of environment. Every field can be set or
/// overridden in `[env_templates.<name>]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvTemplate {
    pub name: String,
    /// `compose.yaml` to start from: a URL or a local path.
    pub compose: String,
    /// Copied to `.env`: a URL or a local path (empty: start with no `.env`).
    pub env_file: String,
    /// The compose service projects are mounted into and shells open in.
    pub service: String,
    /// Where projects are mounted inside the container.
    pub workdir: String,
    /// `.env` variable that names the environment (namespaces its volumes).
    pub name_var: String,
    /// The first environment gets `port_base`, the next `port_base + port_step`, …
    pub port_base: u16,
    pub port_step: u16,
    /// `.env` variables set to the environment's base port plus an offset.
    pub ports: Vec<(String, u16)>,
    /// Named URLs, with `${VAR}` from `.env` and `${FOLDER}` (the project's
    /// folder inside the container) filled in.
    pub links: Vec<(String, String)>,
    /// User and shell for `rattle env shell` (empty user: the image's default).
    pub shell_user: String,
    pub shell: String,
}

const BINDLE_RAW: &str = "https://raw.githubusercontent.com/suckharder/bindle-le/main/template";

/// The built-in templates: bindle-le (a browser VS Code box).
pub fn builtin() -> Vec<EnvTemplate> {
    let ports = [
        ("PORT_CODE", 0),
        ("PORT_HTTP", 2),
        ("PORT_HTTPS", 3),
        ("PORT_FILES", 4),
        ("PORT_DEV_FROM", 10),
        ("PORT_DEV_TO", 19),
        ("PORT_SSH", 22),
    ];
    let links = [
        ("editor", "http://127.0.0.1:${PORT_CODE}/?folder=${FOLDER}"),
        ("files", "http://127.0.0.1:${PORT_FILES}"),
    ];
    vec![EnvTemplate {
        name: "bindle".into(),
        compose: format!("{BINDLE_RAW}/compose.yaml"),
        env_file: format!("{BINDLE_RAW}/.env.example"),
        service: "bindle".into(),
        workdir: "/workspace".into(),
        name_var: "COMPOSE_PROJECT_NAME".into(),
        port_base: 18000,
        port_step: 100,
        ports: ports.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        links: links
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        shell_user: "vscode".into(),
        shell: "bash".into(),
    }]
}

/// Built-in templates with the config's `[env_templates.*]` applied, plus any new ones.
pub fn templates(config: &Config) -> Vec<EnvTemplate> {
    let mut list = builtin();
    for (name, field, value) in &config.env_templates {
        let i = match list.iter().position(|t| t.name == *name) {
            Some(i) => i,
            None => {
                list.push(EnvTemplate {
                    name: name.clone(),
                    compose: String::new(),
                    env_file: String::new(),
                    service: "app".into(),
                    workdir: "/workspace".into(),
                    name_var: "COMPOSE_PROJECT_NAME".into(),
                    port_base: 0,
                    port_step: 100,
                    ports: Vec::new(),
                    links: Vec::new(),
                    shell_user: String::new(),
                    shell: "sh".into(),
                });
                list.len() - 1
            }
        };
        let t = &mut list[i];
        let set = |pairs: &mut Vec<(String, String)>, key: &str| {
            pairs.retain(|(k, _)| k != key);
            pairs.push((key.to_string(), value.clone()));
        };
        match field.as_str() {
            "compose" => t.compose = value.clone(),
            "env_file" => t.env_file = value.clone(),
            "service" => t.service = value.clone(),
            "workdir" => t.workdir = value.clone(),
            "name_var" => t.name_var = value.clone(),
            "shell_user" => t.shell_user = value.clone(),
            "shell" => t.shell = value.clone(),
            "port_base" => t.port_base = value.parse().unwrap_or(t.port_base),
            "port_step" => t.port_step = value.parse().unwrap_or(t.port_step),
            f if f.starts_with("ports.") => {
                if let Ok(offset) = value.parse() {
                    let var = &f["ports.".len()..];
                    t.ports.retain(|(k, _)| k != var);
                    t.ports.push((var.to_string(), offset));
                }
            }
            f if f.starts_with("links.") => set(&mut t.links, &f["links.".len()..]),
            _ => {}
        }
    }
    list
}

pub fn find_template(config: &Config, name: &str) -> Result<EnvTemplate, String> {
    let list = templates(config);
    let names: Vec<&str> = list.iter().map(|t| t.name.as_str()).collect();
    list.iter()
        .find(|t| t.name == name)
        .cloned()
        .ok_or_else(|| {
            format!(
                "unknown env template '{name}' (available: {})",
                names.join(", ")
            )
        })
}

pub fn find_env<'a>(config: &'a Config, name: &str) -> Result<&'a EnvDef, String> {
    config.envs.iter().find(|e| e.name == name).ok_or_else(|| {
        let names: Vec<&str> = config.envs.iter().map(|e| e.name.as_str()).collect();
        if names.is_empty() {
            format!("no environment named '{name}'; create one with: rattle env create {name}")
        } else {
            format!(
                "no environment named '{name}' (configured: {})",
                names.join(", ")
            )
        }
    })
}

/// Environment names become folder and compose project names.
pub fn validate_name(name: &str) -> Result<(), String> {
    let ok = name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if name.is_empty() || !ok || name.starts_with('-') {
        return Err(format!(
            "invalid environment name '{name}': use lowercase letters, digits and '-'"
        ));
    }
    Ok(())
}

pub fn envs_dir(config: &Config, registry: &Path) -> PathBuf {
    match &config.envs_dir {
        Some(dir) => expand_tilde(dir),
        None => registry.with_file_name("envs"),
    }
}

pub fn env_dir(config: &Config, registry: &Path, name: &str) -> PathBuf {
    envs_dir(config, registry).join(name)
}

pub fn is_created(dir: &Path) -> bool {
    dir.join("compose.yaml").is_file()
}

/// The environment `project` belongs to: listed by name, or inside one of its
/// `dirs`. The first match in config order wins.
pub fn env_of<'a>(config: &'a Config, project: &Project) -> Option<&'a EnvDef> {
    let path = Path::new(&project.path);
    config.envs.iter().find(|env| {
        env.projects.contains(&project.name)
            || env
                .dirs
                .iter()
                .filter_map(|d| std::fs::canonicalize(expand_tilde(d)).ok())
                .any(|d| path.starts_with(d))
    })
}

/// Host folders to mount and where they appear in the container: each of the
/// environment's `dirs`, plus listed projects not already inside one.
pub fn mounts(env: &EnvDef, template: &EnvTemplate, store: &Store) -> Vec<(PathBuf, String)> {
    if !env.mount {
        return Vec::new();
    }
    let mut hosts: Vec<(PathBuf, String)> = Vec::new();
    for dir in &env.dirs {
        if let Ok(path) = std::fs::canonicalize(expand_tilde(dir)) {
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            hosts.push((path, name));
        }
    }
    for name in &env.projects {
        let Some(path) = store
            .find(name)
            .and_then(|p| std::fs::canonicalize(&p.path).ok())
        else {
            continue;
        };
        if !hosts.iter().any(|(h, _)| path.starts_with(h)) {
            hosts.push((path, name.clone()));
        }
    }
    let mut out: Vec<(PathBuf, String)> = Vec::new();
    for (host, name) in hosts {
        // Two folders with the same name get name, name-2, …
        let workdir = template.workdir.trim_end_matches('/');
        let mut target = format!("{workdir}/{name}");
        let mut n = 2;
        while out.iter().any(|(_, t)| *t == target) {
            target = format!("{workdir}/{name}-{n}");
            n += 1;
        }
        out.push((host, target));
    }
    out
}

/// Where `project` is inside its environment's container.
pub fn container_folder(
    mounts: &[(PathBuf, String)],
    template: &EnvTemplate,
    project: &Project,
) -> String {
    let path = Path::new(&project.path);
    mounts
        .iter()
        .find_map(|(host, target)| {
            let rel = path.strip_prefix(host).ok()?;
            let rel = rel.to_string_lossy();
            Some(if rel.is_empty() {
                target.clone()
            } else {
                format!("{target}/{rel}")
            })
        })
        .unwrap_or_else(|| template.workdir.clone())
}

const OVERRIDE_FILE: &str = "compose.override.yaml";
const OVERRIDE_HEADER: &str = "# Generated by Rattle";

/// `compose.override.yaml` mounting `mounts` into the template's service.
pub fn override_yaml(
    env: &EnvDef,
    template: &EnvTemplate,
    mounts: &[(PathBuf, String)],
) -> Option<String> {
    if mounts.is_empty() {
        return None;
    }
    // JSON strings are valid YAML double-quoted strings.
    let quote = |s: &str| serde_json::to_string(s).unwrap_or_default();
    let mut yaml = format!(
        "{OVERRIDE_HEADER} from [envs.{}] in config.toml; rewritten on every start.\n\
         services:\n  {}:\n    volumes:\n",
        env.name, template.service
    );
    for (host, target) in mounts {
        yaml.push_str(&format!(
            "      - type: bind\n        source: {}\n        target: {}\n",
            quote(&host.to_string_lossy()),
            quote(target)
        ));
    }
    Some(yaml)
}

/// Writes (or removes) the generated override. Returns whether it changed
/// compared to the one the container was last started with.
pub fn write_override(dir: &Path, yaml: Option<&str>) -> std::io::Result<bool> {
    let path = dir.join(OVERRIDE_FILE);
    let old = std::fs::read_to_string(&path).ok();
    match (yaml, &old) {
        (Some(new), Some(old)) if new == old => Ok(false),
        (Some(new), _) => {
            std::fs::write(&path, new)?;
            Ok(old.is_some())
        }
        // Only ever remove our own file, never a user's override.
        (None, Some(old)) if old.starts_with(OVERRIDE_HEADER) => {
            std::fs::remove_file(&path)?;
            Ok(true)
        }
        (None, _) => Ok(false),
    }
}

/// `KEY=value` pairs of a `.env` file (comments and quotes stripped).
pub fn read_env_file(path: &Path) -> Vec<(String, String)> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            let value = match value.find(" #") {
                Some(i) => &value[..i],
                None => value,
            };
            Some((
                key.trim().to_string(),
                value.trim().trim_matches(['"', '\'']).to_string(),
            ))
        })
        .collect()
}

/// Sets `vars` in `.env` text: replaces existing `KEY=…` lines (keeping a
/// trailing comment) and appends missing ones.
pub fn set_env_vars(text: &str, vars: &[(String, String)]) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    for (key, value) in vars {
        let found = lines.iter().position(|l| {
            l.trim_start()
                .strip_prefix(key.as_str())
                .is_some_and(|r| r.trim_start().starts_with('='))
        });
        match found {
            Some(i) => {
                // Keep a trailing comment, including the padding in front of it.
                let after = lines[i].split_once('=').map(|(_, v)| v).unwrap_or_default();
                let comment = after
                    .find(" #")
                    .map(|c| &after[after[..=c].trim_end().len()..])
                    .unwrap_or_default()
                    .to_string();
                lines[i] = format!("{key}={value}{comment}");
            }
            None => lines.push(format!("{key}={value}")),
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// The base port of an existing environment, from its `.env`.
fn base_of(template: &EnvTemplate, env_vars: &[(String, String)]) -> Option<u16> {
    let (var, offset) = template.ports.iter().min_by_key(|(_, o)| *o)?;
    let value: u16 = env_vars.iter().find(|(k, _)| k == var)?.1.parse().ok()?;
    value.checked_sub(*offset)
}

/// The base port an environment was given, read from its `.env`.
pub fn base_port(template: &EnvTemplate, dir: &Path) -> Option<u16> {
    base_of(template, &read_env_file(&dir.join(".env")))
}

/// The first port range not used by another environment and free on this machine.
pub fn assign_base(
    template: &EnvTemplate,
    used: &[u16],
    is_free: impl Fn(u16) -> bool,
) -> Result<u16, String> {
    if template.ports.is_empty() {
        return Ok(template.port_base);
    }
    for slot in 0..200u32 {
        let Ok(base) =
            u16::try_from(u32::from(template.port_base) + slot * u32::from(template.port_step))
        else {
            break;
        };
        if used.contains(&base) {
            continue;
        }
        let ports: Option<Vec<u16>> = template
            .ports
            .iter()
            .map(|(_, o)| base.checked_add(*o))
            .collect();
        if ports.is_some_and(|ports| ports.into_iter().all(&is_free)) {
            return Ok(base);
        }
        if template.port_step == 0 {
            break;
        }
    }
    Err(format!(
        "no free port range for template '{}'",
        template.name
    ))
}

pub fn port_is_free(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// Base ports used by the other environments in `envs_dir`.
fn used_bases(config: &Config, registry: &Path, template: &EnvTemplate) -> Vec<u16> {
    let Ok(entries) = std::fs::read_dir(envs_dir(config, registry)) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| base_of(template, &read_env_file(&e.path().join(".env"))))
        .collect()
}

#[derive(Debug)]
pub struct Created {
    pub dir: PathBuf,
    pub vars: Vec<(String, String)>,
}

/// Creates `env`'s files from its template: fetches `compose.yaml` and `.env`,
/// then names the environment and gives it free ports.
pub fn create(config: &Config, registry: &Path, env: &EnvDef) -> Result<Created, String> {
    validate_name(&env.name)?;
    let template = find_template(config, &env.template)?;
    if template.compose.is_empty() {
        return Err(format!(
            "env template '{}' has no compose file set",
            template.name
        ));
    }
    let dir = env_dir(config, registry, &env.name);
    if is_created(&dir) {
        return Err(format!(
            "environment '{}' already exists in {}",
            env.name,
            dir.display()
        ));
    }
    let base = assign_base(
        &template,
        &used_bases(config, registry, &template),
        port_is_free,
    )?;
    let fresh = !dir.exists();
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let result = (|| {
        fetch(&template.compose, &dir.join("compose.yaml"))?;
        let env_text = if template.env_file.is_empty() {
            String::new()
        } else {
            let tmp = dir.join(".env.template");
            fetch(&template.env_file, &tmp)?;
            let text = std::fs::read_to_string(&tmp).map_err(|e| e.to_string())?;
            let _ = std::fs::remove_file(tmp);
            text
        };
        let mut vars = vec![(template.name_var.clone(), format!("rattle-{}", env.name))];
        vars.extend(
            template
                .ports
                .iter()
                .map(|(k, o)| (k.clone(), (base + o).to_string())),
        );
        std::fs::write(dir.join(".env"), set_env_vars(&env_text, &vars))
            .map_err(|e| e.to_string())?;
        Ok(vars)
    })();
    match result {
        Ok(vars) => Ok(Created { dir, vars }),
        Err(e) => {
            // Only clean up a folder this call created.
            if fresh {
                let _ = std::fs::remove_dir_all(&dir);
            }
            Err(e)
        }
    }
}

/// Copies a local file, or downloads a URL with curl.
fn fetch(source: &str, dest: &Path) -> Result<(), String> {
    if source.starts_with("http://") || source.starts_with("https://") {
        let out = Command::new("curl")
            .args(["-fsSL", "-o"])
            .arg(dest)
            .arg(source)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("could not run curl to download {source}: {e}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(format!("could not download {source}: {}", err.trim()));
        }
        Ok(())
    } else {
        std::fs::copy(expand_tilde(source), dest)
            .map(|_| ())
            .map_err(|e| format!("could not copy {source}: {e}"))
    }
}

pub struct Up {
    /// `docker compose up -d`.
    pub command: Command,
    /// The mounts differ from the container's: it will be recreated.
    pub mounts_changed: bool,
    pub mounts: usize,
}

/// Regenerates the mounts for `env`, then returns the command that starts it.
pub fn prepare_up(
    config: &Config,
    registry: &Path,
    env: &EnvDef,
    store: &Store,
) -> Result<Up, String> {
    let template = find_template(config, &env.template)?;
    let dir = env_dir(config, registry, &env.name);
    if !is_created(&dir) {
        return Err(format!(
            "environment '{}' isn't created yet; run: rattle env create {}",
            env.name, env.name
        ));
    }
    let mounts = mounts(env, &template, store);
    let yaml = override_yaml(env, &template, &mounts);
    let mounts_changed = write_override(&dir, yaml.as_deref())
        .map_err(|e| format!("cannot write {}: {e}", dir.join(OVERRIDE_FILE).display()))?;
    Ok(Up {
        command: compose(config, &dir, &["up", "-d"]),
        mounts_changed,
        mounts: mounts.len(),
    })
}

/// Forgets environment `name` once its container is gone: deletes its files
/// (compose.yaml, .env, …) and its `[envs.<name>]` section. Docker volumes are
/// not touched, so creating an environment with the same name reattaches them.
pub fn forget(config: &Config, registry: &Path, name: &str) -> Result<(), String> {
    // The name builds the folder path below; never act on an odd one.
    validate_name(name)?;
    let dir = env_dir(config, registry, name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)
            .map_err(|e| format!("cannot remove {}: {e}", dir.display()))?;
    }
    crate::config::delete_table(&crate::config::path_for(registry), &format!("envs.{name}"))
        .map_err(|e| format!("cannot update config: {e}"))?;
    Ok(())
}

/// Where `project` is inside `env`'s container (its workdir if not mounted).
pub fn project_folder(
    env: &EnvDef,
    template: &EnvTemplate,
    store: &Store,
    project: &Project,
) -> String {
    container_folder(&mounts(env, template, store), template, project)
}

/// `docker compose <args>` run in the environment's folder.
pub fn compose(config: &Config, dir: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(&config.docker);
    cmd.arg("compose").args(args).current_dir(dir);
    cmd
}

/// An interactive shell in the container, in `folder`.
pub fn shell_command(config: &Config, dir: &Path, template: &EnvTemplate, folder: &str) -> Command {
    let mut args = vec!["exec", "-w", folder];
    if !template.shell_user.is_empty() {
        args.extend(["-u", template.shell_user.as_str()]);
    }
    args.extend([template.service.as_str(), template.shell.as_str()]);
    compose(config, dir, &args)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    NotCreated,
    Stopped,
    Running,
    /// Docker isn't installed or running, or the status couldn't be read.
    Unknown(String),
}

impl Status {
    pub fn label(&self) -> &str {
        match self {
            Self::NotCreated => "not created",
            Self::Stopped => "stopped",
            Self::Running => "running",
            Self::Unknown(_) => "unknown",
        }
    }
}

pub fn status(config: &Config, dir: &Path) -> Status {
    if !is_created(dir) {
        return Status::NotCreated;
    }
    let out = match compose(config, dir, &["ps", "--all", "--format", "json"])
        .stdin(Stdio::null())
        .output()
    {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Status::Unknown(format!("{} is not installed", config.docker));
        }
        Err(e) => return Status::Unknown(e.to_string()),
    };
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let line = err
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("docker failed");
        return Status::Unknown(line.trim().to_string());
    }
    parse_ps(&String::from_utf8_lossy(&out.stdout))
}

/// `docker compose ps --format json`: a JSON array (older Compose) or one
/// object per line (newer).
pub fn parse_ps(text: &str) -> Status {
    let text = text.trim();
    let containers: Vec<serde_json::Value> = if text.starts_with('[') {
        serde_json::from_str(text).unwrap_or_default()
    } else {
        text.lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    };
    let running = containers.iter().any(|c| {
        c.get("State")
            .and_then(|s| s.as_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("running"))
    });
    if running {
        Status::Running
    } else {
        Status::Stopped
    }
}

/// The template's links with `.env` values and `folder` filled in.
pub fn links(template: &EnvTemplate, dir: &Path, folder: &str) -> Vec<(String, String)> {
    let mut vars = read_env_file(&dir.join(".env"));
    vars.push(("FOLDER".into(), folder.to_string()));
    template
        .links
        .iter()
        .map(|(name, url)| (name.clone(), expand(url, &vars)))
        .collect()
}

/// Replaces `${VAR}` with its value (unknown variables are left as they are).
fn expand(text: &str, vars: &[(String, String)]) -> String {
    let mut out = text.to_string();
    for (key, value) in vars {
        out = out.replace(&format!("${{{key}}}"), value);
    }
    out
}

/// Opens a URL with the configured `browser` command, else the system default.
pub fn open_url(config: &Config, url: &str) -> Command {
    let default = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let browser = config.browser.as_deref().unwrap_or(default);
    let mut parts = browser.split_whitespace();
    let mut cmd = Command::new(parts.next().unwrap_or(default));
    cmd.args(parts).arg(url).stdin(Stdio::null());
    cmd
}

/// Projects that belong to `env`.
pub fn members<'a>(config: &Config, env: &EnvDef, store: &'a Store) -> Vec<&'a Project> {
    store
        .projects
        .iter()
        .filter(|p| env_of(config, p).is_some_and(|e| e.name == env.name))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bindle() -> EnvTemplate {
        builtin().remove(0)
    }

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rattle-envs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    #[test]
    fn config_overrides_and_adds_templates() {
        let config = Config {
            env_templates: vec![
                ("bindle".into(), "port_base".into(), "19000".into()),
                ("bindle".into(), "ports.PORT_CODE".into(), "1".into()),
                ("bindle".into(), "links.files".into(), "http://x".into()),
                ("mine".into(), "compose".into(), "~/box/compose.yaml".into()),
                ("mine".into(), "service".into(), "dev".into()),
            ],
            ..Config::default()
        };
        let b = find_template(&config, "bindle").unwrap();
        assert_eq!(b.port_base, 19000);
        assert!(b.ports.contains(&("PORT_CODE".into(), 1)) && b.ports.len() == 7);
        assert_eq!(
            b.links.iter().find(|(k, _)| k == "files").unwrap().1,
            "http://x"
        );
        let mine = find_template(&config, "mine").unwrap();
        assert_eq!((mine.service.as_str(), mine.ports.len()), ("dev", 0));
        assert!(find_template(&config, "nope")
            .unwrap_err()
            .contains("available: bindle, mine"));
    }

    #[test]
    fn projects_belong_to_the_first_matching_env() {
        let root = tmp("member");
        for d in ["work/api", "side/game", "other/infra"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        let p = |name: &str, rel: &str| Project {
            name: name.into(),
            path: root.join(rel).display().to_string(),
        };
        let config = Config {
            envs: vec![
                EnvDef {
                    dirs: vec![root.join("work").display().to_string()],
                    projects: vec!["infra".into()],
                    ..EnvDef::new("work")
                },
                EnvDef {
                    dirs: vec![root.join("side").display().to_string()],
                    ..EnvDef::new("personal")
                },
            ],
            ..Config::default()
        };
        let env = |pr: &Project| env_of(&config, pr).map(|e| e.name.clone());
        assert_eq!(env(&p("api", "work/api")).as_deref(), Some("work"));
        assert_eq!(env(&p("game", "side/game")).as_deref(), Some("personal"));
        assert_eq!(env(&p("infra", "other/infra")).as_deref(), Some("work"));
        assert_eq!(env(&p("loose", "other")), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mounts_dirs_and_listed_projects_without_clashes() {
        let root = tmp("mounts");
        for d in ["a/code", "b/code", "x/infra", "a/code/inside"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        let mut store = Store::default();
        store
            .add(Project {
                name: "infra".into(),
                path: root.join("x/infra").display().to_string(),
            })
            .unwrap();
        store
            .add(Project {
                name: "inside".into(),
                path: root.join("a/code/inside").display().to_string(),
            })
            .unwrap();
        let env = EnvDef {
            dirs: vec![
                root.join("a/code").display().to_string(),
                root.join("b/code").display().to_string(),
            ],
            projects: vec!["infra".into(), "inside".into(), "ghost".into()],
            ..EnvDef::new("work")
        };
        let m = mounts(&env, &bindle(), &store);
        let targets: Vec<&str> = m.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(
            targets,
            vec!["/workspace/code", "/workspace/code-2", "/workspace/infra"]
        );
        let inside = store.find("inside").unwrap();
        assert_eq!(
            container_folder(&m, &bindle(), inside),
            "/workspace/code/inside"
        );

        let yaml = override_yaml(&env, &bindle(), &m).unwrap();
        assert!(
            yaml.starts_with(OVERRIDE_HEADER) && yaml.contains("  bindle:\n    volumes:\n"),
            "{yaml}"
        );
        assert!(yaml.contains(&format!("source: \"{}\"", root.join("x/infra").display())));
        assert!(override_yaml(
            &EnvDef {
                mount: false,
                ..env.clone()
            },
            &bindle(),
            &[]
        )
        .is_none());

        // Rewritten only when it changes; a user's own override is never removed.
        assert!(
            !write_override(&root, Some(&yaml)).unwrap(),
            "first write: nothing to recreate"
        );
        assert!(!write_override(&root, Some(&yaml)).unwrap());
        assert!(write_override(&root, Some("# Generated by Rattle\nother\n")).unwrap());
        assert!(write_override(&root, None).unwrap());
        std::fs::write(root.join(OVERRIDE_FILE), "services: {}\n").unwrap();
        assert!(!write_override(&root, None).unwrap());
        assert!(root.join(OVERRIDE_FILE).exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn env_file_values_are_set_keeping_comments() {
        let text = "# box\nCOMPOSE_PROJECT_NAME=bindle-01\nPORT_CODE=18000       # BASE+0 editor\n#PASSWORD=x\n";
        let out = set_env_vars(
            text,
            &[
                ("COMPOSE_PROJECT_NAME".into(), "rattle-work".into()),
                ("PORT_CODE".into(), "18100".into()),
                ("PASSWORD".into(), "secret".into()),
            ],
        );
        assert_eq!(
            out,
            "# box\nCOMPOSE_PROJECT_NAME=rattle-work\nPORT_CODE=18100       # BASE+0 editor\n#PASSWORD=x\nPASSWORD=secret\n"
        );
        let dir = tmp("envfile");
        std::fs::write(dir.join(".env"), &out).unwrap();
        let vars = read_env_file(&dir.join(".env"));
        assert!(vars.contains(&("PORT_CODE".into(), "18100".into())));
        assert!(!vars.iter().any(|(_, v)| v.contains('#')));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn port_ranges_skip_used_and_busy_ones() {
        let t = bindle();
        assert_eq!(assign_base(&t, &[], |_| true), Ok(18000));
        assert_eq!(assign_base(&t, &[18000, 18100], |_| true), Ok(18200));
        // Something else listens on 18004 (FileBrowser's port in slot 0).
        assert_eq!(assign_base(&t, &[], |p| p != 18004), Ok(18100));
        assert!(assign_base(&t, &[], |_| false).is_err());
        let vars = vec![("PORT_CODE".to_string(), "18300".to_string())];
        assert_eq!(base_of(&t, &vars), Some(18300));
    }

    #[test]
    fn reads_compose_ps_in_both_formats() {
        assert_eq!(
            parse_ps(r#"[{"Service":"bindle","State":"running"}]"#),
            Status::Running
        );
        assert_eq!(
            parse_ps("{\"State\":\"exited\"}\n{\"State\":\"running\"}\n"),
            Status::Running
        );
        assert_eq!(parse_ps(r#"{"State":"exited"}"#), Status::Stopped);
        assert_eq!(parse_ps(""), Status::Stopped);
    }

    #[test]
    fn links_fill_in_env_values_and_folder() {
        let dir = tmp("links");
        std::fs::write(dir.join(".env"), "PORT_CODE=18100\nPORT_FILES=18104\n").unwrap();
        let links = links(&bindle(), &dir, "/workspace/code/api");
        assert_eq!(
            links,
            vec![
                (
                    "editor".to_string(),
                    "http://127.0.0.1:18100/?folder=/workspace/code/api".to_string()
                ),
                ("files".to_string(), "http://127.0.0.1:18104".to_string()),
            ]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn create_fetches_files_names_and_assigns_ports() {
        let root = tmp("create");
        std::fs::write(root.join("compose.yaml"), "services:\n  bindle: {}\n").unwrap();
        std::fs::write(
            root.join("example.env"),
            "COMPOSE_PROJECT_NAME=bindle-01\nPORT_CODE=18000 # editor\n",
        )
        .unwrap();
        let config = Config {
            envs_dir: Some(root.join("envs").display().to_string()),
            env_templates: vec![
                (
                    "bindle".into(),
                    "compose".into(),
                    root.join("compose.yaml").display().to_string(),
                ),
                (
                    "bindle".into(),
                    "env_file".into(),
                    root.join("example.env").display().to_string(),
                ),
                // Ports nobody uses on a test machine.
                ("bindle".into(), "port_base".into(), "41000".into()),
            ],
            ..Config::default()
        };
        let registry = root.join("projects.toml");
        let made = create(&config, &registry, &EnvDef::new("work")).unwrap();
        assert!(made.dir.join("compose.yaml").is_file());
        let env = read_env_file(&made.dir.join(".env"));
        assert!(env.contains(&("COMPOSE_PROJECT_NAME".into(), "rattle-work".into())));
        assert!(env.contains(&("PORT_CODE".into(), "41000".into())));

        let second = create(&config, &registry, &EnvDef::new("personal")).unwrap();
        assert!(
            read_env_file(&second.dir.join(".env")).contains(&("PORT_CODE".into(), "41100".into()))
        );
        assert!(create(&config, &registry, &EnvDef::new("work"))
            .unwrap_err()
            .contains("already exists"));
        assert!(create(&config, &registry, &EnvDef::new("Bad Name")).is_err());

        let broken = Config {
            env_templates: vec![(
                "bindle".into(),
                "compose".into(),
                "/no/such/compose.yaml".into(),
            )],
            ..config.clone()
        };
        assert!(create(&broken, &registry, &EnvDef::new("ghost"))
            .unwrap_err()
            .contains("could not copy"));
        assert!(
            !root.join("envs/ghost").exists(),
            "a failed create leaves nothing behind"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
