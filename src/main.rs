mod config;
mod deps;
mod envs;
mod ops;
mod settings;
mod store;
mod templates;
mod tui;

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use config::{Config, DeleteFiles};
use store::Store;

const HELP: &str = "\
Rattle: all your projects in one place, without hunting for folders.

Usage: rattle [command] [options]

Without a command, rattle opens the TUI; press ? there for its keys. Enter in
the TUI prints the chosen project's path, so `cd \"$(rattle)\"` jumps into it.

Projects
  show                       list projects                  (aliases: list, ls)
  create [path]              register a folder, e.g. `create .`   (alias: add)
  delete <name | number>     remove an entry; files stay  (aliases: remove, rm)
    --files                  also delete the folder (see delete_files in config)
  scan [folder...]           register projects in folders (default: scan_dirs)
    -n, --dry-run            only show what would be added
    -d, --depth N            search N levels deep (default: max_depth)
  prune [-n | --dry-run]     remove entries whose folder no longer exists

New projects and dependencies
  new <name>                 create a project from a template and register it
    -t, --template T         the template (default: asks, or empty)
    --in <folder>            where (default: new_project_dir, or 1st scan dir)
    --list                   show the templates
  deps [project] [list]      list dependencies; project is a name, number or
                             folder (default: the current folder)
  deps [project] add [--dev] <package>...   add, with the project's own tool
  deps [project] remove <package>...        remove

Dev environments (one container per group of projects; alias: envs)
  env [list]                 list environments and whether they run
  env create <name>          create one; adds [envs.<name>] to the config
    --dir <folder>           whose projects belong to it (repeatable)
    -t, --template T         its env template (default: bindle)
  env up | stop | restart <name>   start (mounting its projects), stop, restart
  env logs <name> [-f]       show its logs (-f follows them)
  env shell <name> [project] a shell in its container, in the project's folder
  env open <name> [link] [--project P]   open a link (default: the editor)
  env config <name>          edit its settings (.env)
  env path <name>            print the folder with its files
  env remove <name>          remove it: container, files, config (volumes stay)

Settings and info
  config                     edit config.toml in your editor
  config path                print where config.toml is
  tui                        open the TUI
  help                       show this help (also -h, --help)
  version                    print the version (also -V, --version)

create, delete and new ask for a missing argument. Projects are stored in
~/.rattle/projects.toml (or $RATTLE_FILE), the config in config.toml next to it.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let file = std::env::var_os("RATTLE_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(store::default_path);

    let result = match args.first().map(String::as_str) {
        Some("show") | Some("list") | Some("ls") => show(&file),
        Some("create") | Some("add") => create(&file, args.get(1)),
        Some("delete") | Some("remove") | Some("rm") => delete(&file, &args[1..]),
        Some("scan") => scan(&file, &args[1..]),
        Some("prune") => prune(&file, &args[1..]),
        Some("new") => new_project(&file, &args[1..]),
        Some("deps") => deps_command(&file, &args[1..]),
        Some("env") | Some("envs") => env_command(&file, &args[1..]),
        Some("config") => edit_config(&file, args.get(1).map(String::as_str)),
        Some("tui") => tui::run(&file),
        // The TUI draws on stderr, so stdout may be captured by `$(rattle)`.
        None if io::stdin().is_terminal() && io::stderr().is_terminal() => tui::run(&file),
        Some("version") | Some("-V") | Some("--version") => {
            println!("rattle {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("help") | Some("-h") | Some("--help") | None => {
            println!("{HELP}");
            Ok(())
        }
        Some(other) => {
            eprintln!("unknown command: {other}\n");
            println!("{HELP}");
            return ExitCode::FAILURE;
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("rattle: {e}");
            ExitCode::FAILURE
        }
    }
}

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn show(file: &Path) -> Result<()> {
    let store = Store::load(file)?;
    if store.projects.is_empty() {
        println!("No projects yet. Add one with: rattle create <path>");
        return Ok(());
    }
    let width = store
        .projects
        .iter()
        .map(|p| p.name.len())
        .max()
        .unwrap_or(0);
    for (i, p) in store.projects.iter().enumerate() {
        println!(" {i}. {:<width$}  {}", p.name, p.path);
    }
    Ok(())
}

fn create(file: &Path, arg: Option<&String>) -> Result<()> {
    let raw = match arg {
        Some(a) => a.clone(),
        None => prompt("Write a Path: ")?,
    };
    let project = ops::resolve_project(&raw)?;
    let mut store = Store::load(file)?;
    let msg = format!("Added '{}' -> {}", project.name, project.path);
    store.add(project)?;
    store.save(file)?;
    println!("{msg}");
    Ok(())
}

fn delete(file: &Path, args: &[String]) -> Result<()> {
    let with_files = args.iter().any(|a| a == "--files" || a == "-f");
    let name = match args.iter().find(|a| !a.starts_with('-')) {
        Some(a) => a.clone(),
        None => prompt("Project Name: ")?,
    };
    if let Some(bad) = args
        .iter()
        .find(|a| a.starts_with('-') && *a != "--files" && *a != "-f")
    {
        return Err(format!("unknown delete option: {bad}").into());
    }
    let mut store = Store::load(file)?;
    // Accept either the project name or its index from `rattle show`.
    let name = match name.parse::<usize>() {
        Ok(i) if store.find(&name).is_none() && i < store.projects.len() => {
            store.projects[i].name.clone()
        }
        _ => name,
    };
    if with_files {
        return delete_with_files(file, store, &name);
    }
    let removed = store
        .remove(&name)
        .ok_or_else(|| format!("no project named '{name}'"))?;
    store.save(file)?;
    println!(
        "Removed '{}' (files at {} were left untouched)",
        removed.name, removed.path
    );
    Ok(())
}

/// `rattle delete <name> --files`: removes the entry and the folder itself,
/// as allowed by `delete_files`, after an interactive confirmation.
fn delete_with_files(file: &Path, mut store: Store, name: &str) -> Result<()> {
    let config_file = config::path_for(file);
    let config = Config::load(&config_file)?;
    if config.delete_files == DeleteFiles::Off {
        return Err(format!(
            "deleting folders is turned off; set delete_files = \"trash\" (or \"permanent\") in {}",
            config_file.display()
        )
        .into());
    }
    let project = store
        .find(name)
        .cloned()
        .ok_or_else(|| format!("no project named '{name}'"))?;
    let path = ops::deletable_folder(&project, &store, &config, file)?;
    if !io::stdin().is_terminal() {
        return Err("deleting a folder needs an interactive terminal to confirm".into());
    }

    let confirmed = match config.delete_files {
        DeleteFiles::Trash => confirm(&format!("Move {} to the Trash? [y/N] ", path.display()))?,
        _ => {
            println!(
                "This permanently deletes {} and everything in it.",
                path.display()
            );
            prompt(&format!("Type '{name}' to confirm: "))? == name
        }
    };
    if !confirmed {
        println!("Cancelled; nothing was deleted.");
        return Ok(());
    }
    ops::delete_folder(&path, config.delete_files)?;
    store.remove(name);
    store.save(file)?;
    let how = if config.delete_files == DeleteFiles::Trash {
        "moved to the Trash"
    } else {
        "deleted"
    };
    println!("Removed '{name}'; {} was {how}", path.display());
    Ok(())
}

fn scan(file: &Path, args: &[String]) -> Result<()> {
    let config_file = config::path_for(file);
    let mut config = Config::load(&config_file)?;

    let mut dry_run = false;
    let mut roots = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-n" | "--dry-run" => dry_run = true,
            "-d" | "--depth" => {
                config.max_depth = args
                    .next()
                    .and_then(|d| d.parse().ok())
                    .ok_or("--depth needs a number, e.g. --depth 2")?;
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown scan option: {other}").into())
            }
            other => roots.push(other.to_string()),
        }
    }
    if roots.is_empty() {
        roots = config.scan_dirs.clone();
    }
    if roots.is_empty() {
        return Err(format!(
            "nothing to scan; pass a folder (rattle scan ~/code) or set scan_dirs in {}",
            config_file.display()
        )
        .into());
    }

    let mut store = Store::load(file)?;
    let report = ops::scan_roots(&mut store, &roots, &config);
    for w in &report.warnings {
        eprintln!("warning: {w}");
    }
    for p in &report.added {
        println!(" + {}  {}", p.name, p.path);
    }
    if !report.added.is_empty() && !dry_run {
        store.save(file)?;
    }
    let verb = if dry_run { "Would add" } else { "Added" };
    println!(
        "{verb} {} project(s), {} already registered",
        report.added.len(),
        report.already
    );

    // Offer, never force, to drop entries whose folder is gone.
    let missing: Vec<String> = store.missing().iter().map(|p| p.name.clone()).collect();
    if !missing.is_empty() {
        println!(
            "{} registered project(s) no longer exist: {}",
            missing.len(),
            missing.join(", ")
        );
        if !dry_run && io::stdin().is_terminal() {
            if confirm("Remove them from Rattle? (folders are already gone) [y/N] ")? {
                let gone = store.prune_missing();
                store.save(file)?;
                println!(
                    "Removed {} entr{}",
                    gone.len(),
                    if gone.len() == 1 { "y" } else { "ies" }
                );
            }
        } else {
            println!("Run `rattle prune` to remove them.");
        }
    }
    Ok(())
}

fn prune(file: &Path, args: &[String]) -> Result<()> {
    let dry_run = match args.first().map(String::as_str) {
        None => false,
        Some("-n") | Some("--dry-run") => true,
        Some(other) => return Err(format!("unknown prune option: {other}").into()),
    };
    let mut store = Store::load(file)?;
    let gone: Vec<_> = if dry_run {
        store.missing().into_iter().cloned().collect()
    } else {
        store.prune_missing()
    };
    for p in &gone {
        println!(" - {}  {}", p.name, p.path);
    }
    if !gone.is_empty() && !dry_run {
        store.save(file)?;
    }
    let verb = if dry_run { "Would remove" } else { "Removed" };
    println!("{verb} {} missing project(s)", gone.len());
    Ok(())
}

fn new_project(file: &Path, args: &[String]) -> Result<()> {
    let config = Config::load(&config::path_for(file))?;
    let (mut name, mut template, mut parent, mut list) = (None, None, None, false);
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-l" | "--list" => list = true,
            "-t" | "--template" => {
                template = Some(args.next().ok_or("--template needs a name")?.clone())
            }
            "--in" => parent = Some(args.next().ok_or("--in needs a folder")?.clone()),
            other if other.starts_with('-') => {
                return Err(format!("unknown new option: {other}").into())
            }
            other => name = Some(other.to_string()),
        }
    }
    let all = templates::all(&config);
    if list {
        let width = all.iter().map(|t| t.name.len()).max().unwrap_or(0);
        for t in &all {
            println!(" {:<width$}  {}", t.name, t.command);
        }
        return Ok(());
    }

    let name = match name {
        Some(n) => n,
        None => prompt("Project name: ")?,
    };
    templates::validate_name(&name)?;
    let template = match template {
        Some(t) => templates::find(&config, &t)?,
        None if io::stdin().is_terminal() => {
            for (i, t) in all.iter().enumerate() {
                println!(" {i}. {:<11} {}", t.name, t.command);
            }
            let pick = prompt("Template (number or name, default empty): ")?;
            match pick.parse::<usize>() {
                Ok(i) if i < all.len() => all[i].clone(),
                _ if pick.is_empty() => templates::find(&config, "empty")?,
                _ => templates::find(&config, &pick)?,
            }
        }
        None => templates::find(&config, "empty")?,
    };
    // A folder given with --in is relative to where rattle runs, like `create`.
    let parent = templates::parent_dir(&config, parent.as_deref())?;

    let mut store = Store::load(file)?;
    // Check first, so "Creating…" is only printed when it is really going to run.
    let mut prepared = templates::prepare(&store, &template, &name, &parent)?;
    println!(
        "Creating '{name}' in {} with template '{}'…",
        parent.display(),
        template.name
    );
    let output = prepared
        .command
        .output()
        .map_err(|e| format!("could not run template '{}': {e}", template.name))?;
    let project = templates::finish(&mut store, &template.name, &prepared.target, &output)?;
    store.save(file)?;
    println!("Created and added '{}' -> {}", project.name, project.path);
    Ok(())
}

/// `rattle deps [project] [list | add [--dev] <pkg>... | remove <pkg>...]`
fn deps_command(file: &Path, args: &[String]) -> Result<()> {
    const ACTIONS: &[&str] = &["list", "add", "remove", "rm"];
    // The project is optional: without one, the current folder is used.
    let (target, rest) = match args.first() {
        Some(a) if !ACTIONS.contains(&a.as_str()) => (Some(a.as_str()), &args[1..]),
        _ => (None, args),
    };
    let dir = match target {
        None | Some(".") => std::env::current_dir()?,
        Some(t) => {
            let store = Store::load(file)?;
            let by_index = t.parse::<usize>().ok().and_then(|i| store.projects.get(i));
            match store.find(t).or(by_index) {
                Some(p) => PathBuf::from(&p.path),
                None if Path::new(t).is_dir() => PathBuf::from(t),
                None => return Err(format!("no project or folder named '{t}'").into()),
            }
        }
    };
    let manager = deps::detect(&dir).ok_or_else(|| {
        format!(
            "no supported manifest in {} (Cargo.toml, package.json, pyproject.toml, go.mod)",
            dir.display()
        )
    })?;

    let action = rest.first().map(String::as_str).unwrap_or("list");
    let dev = rest.iter().any(|a| a == "--dev" || a == "-D");
    let packages: Vec<String> = rest
        .iter()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .cloned()
        .collect();
    match action {
        "list" => {
            let deps = deps::list(&dir, manager)?;
            let noun = if deps.len() == 1 {
                "dependency"
            } else {
                "dependencies"
            };
            println!(
                "{} · {} ({}) · {} {noun}",
                dir.display(),
                manager.name(),
                manager.manifest(),
                deps.len()
            );
            let width = deps.iter().map(|d| d.name.len()).max().unwrap_or(0);
            for d in deps {
                let group = d.group.label();
                let group = if group.is_empty() {
                    String::new()
                } else {
                    format!("  [{group}]")
                };
                println!(" {:<width$}  {}{group}", d.name, d.spec);
            }
            return Ok(());
        }
        "add" | "remove" | "rm" if packages.is_empty() => {
            return Err(format!("usage: rattle deps [project] {action} <package>...").into());
        }
        "add" => {
            run_visible(deps::add_command(manager, &dir, &packages, dev))?;
        }
        _ => {
            let listed = deps::list(&dir, manager).unwrap_or_default();
            for name in &packages {
                // Remove from the group it is listed in (dev, build, optional, …).
                let group = listed
                    .iter()
                    .find(|d| d.name == *name)
                    .map(|d| d.group.clone());
                let group = group.unwrap_or(if dev {
                    deps::Group::Dev
                } else {
                    deps::Group::Normal
                });
                run_visible(deps::remove_command(manager, &dir, name, &group))?;
            }
        }
    }
    Ok(())
}

/// Runs a package-manager command with its output shown, for the CLI.
const ENV_USAGE: &str = "\
usage: rattle env [list]
       rattle env create <name> [-t | --template T] [--dir folder]...
       rattle env up | stop | restart <name>
       rattle env logs <name> [-f]
       rattle env shell <name> [project]
       rattle env open <name> [link] [--project P]
       rattle env config | path | remove <name>";

/// `rattle env …`: dev environments, one container per group of projects.
fn env_command(file: &Path, args: &[String]) -> Result<()> {
    let config_file = config::path_for(file);
    let config = Config::load(&config_file)?;
    let store = Store::load(file)?;
    let action = args.first().map(String::as_str).unwrap_or("list");
    let rest = args.get(1..).unwrap_or_default();
    let name = || -> Result<String> {
        rest.iter()
            .find(|a| !a.starts_with('-'))
            .cloned()
            .ok_or_else(|| format!("{action} needs an environment name\n{ENV_USAGE}").into())
    };

    match action {
        "list" | "ls" => env_list(file, &config, &store),
        "create" => env_create(file, &config_file, &config, rest),
        "up" | "start" => {
            let env = envs::find_env(&config, &name()?)?;
            let up = envs::prepare_up(&config, file, env, &store)?;
            if up.mounts_changed {
                println!(
                    "Mounts changed: the container will be recreated. Runtime installs not listed\n\
                     in its .env (INSTALL_PACKAGES, …) will be gone; volumes are kept."
                );
            }
            run_visible(up.command)?;
            println!(
                "'{}' is running with {} mounted folder(s).",
                env.name, up.mounts
            );
            print_links(file, &config, env, &store, None)
        }
        "stop" | "restart" => {
            let env = envs::find_env(&config, &name()?)?;
            let dir = envs::env_dir(&config, file, &env.name);
            if !envs::is_created(&dir) {
                return Err(format!("environment '{}' isn't created yet", env.name).into());
            }
            run_visible(envs::compose(&config, &dir, &[action]))
        }
        "logs" => {
            let env = envs::find_env(&config, &name()?)?;
            let dir = envs::env_dir(&config, file, &env.name);
            let mut args = vec!["logs", "--tail", "200"];
            if rest.iter().any(|a| a == "-f" || a == "--follow") {
                args.push("--follow");
            }
            run_visible(envs::compose(&config, &dir, &args))
        }
        "shell" => {
            let env = envs::find_env(&config, &name()?)?;
            let template = envs::find_template(&config, &env.template)?;
            let dir = envs::env_dir(&config, file, &env.name);
            let folder = match rest.iter().filter(|a| !a.starts_with('-')).nth(1) {
                Some(p) => {
                    let project = store
                        .find(p)
                        .ok_or_else(|| format!("no project named '{p}'"))?;
                    envs::project_folder(env, &template, &store, project)
                }
                None => template.workdir.clone(),
            };
            run_visible(envs::shell_command(&config, &dir, &template, &folder))
        }
        "open" => {
            // <name> [link] [--project P], where P is not a positional.
            let (mut positional, mut project) = (Vec::new(), None);
            let mut args = rest.iter();
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "-p" | "--project" => {
                        let p = args.next().ok_or("--project needs a project name")?;
                        project = Some(
                            store
                                .find(p)
                                .ok_or_else(|| format!("no project named '{p}'"))?,
                        );
                    }
                    other if other.starts_with('-') => {
                        return Err(format!("unknown env open option: {other}").into())
                    }
                    other => positional.push(other.to_string()),
                }
            }
            let env_name = positional
                .first()
                .ok_or_else(|| format!("open needs an environment name\n{ENV_USAGE}"))?;
            let env = envs::find_env(&config, env_name)?;
            let link = positional.get(1).cloned();
            let links = env_links(file, &config, env, &store, project)?;
            let (label, url) = match &link {
                Some(l) => links.iter().find(|(n, _)| n == l).ok_or_else(|| {
                    let names: Vec<&str> = links.iter().map(|(n, _)| n.as_str()).collect();
                    format!("no link '{l}' (available: {})", names.join(", "))
                })?,
                None => links
                    .first()
                    .ok_or("this environment's template has no links")?,
            };
            println!("Opening {label}: {url}");
            run_visible(envs::open_url(&config, url))
        }
        "config" => {
            let env = envs::find_env(&config, &name()?)?;
            let dir = envs::env_dir(&config, file, &env.name);
            let env_file = dir.join(".env");
            if !env_file.exists() {
                return Err(format!("environment '{}' isn't created yet", env.name).into());
            }
            let (mut cmd, editor) =
                ops::editor_command(config.editor.as_deref(), &dir, &env_file, true);
            let status = cmd
                .status()
                .map_err(|e| format!("could not run '{editor}': {e}"))?;
            if !status.success() {
                return Err(format!("'{editor}' exited with {status}").into());
            }
            println!("Saved. Apply it with: rattle env up {}", env.name);
            Ok(())
        }
        "path" => {
            let env = envs::find_env(&config, &name()?)?;
            println!("{}", envs::env_dir(&config, file, &env.name).display());
            Ok(())
        }
        "remove" | "rm" => {
            let env = envs::find_env(&config, &name()?)?;
            let dir = envs::env_dir(&config, file, &env.name);
            if !io::stdin().is_terminal() {
                return Err(
                    "removing an environment needs an interactive terminal to confirm".into(),
                );
            }
            println!(
                "This removes environment '{}': its container, its files in {}\n\
                 and its [envs.{}] section. Its volumes (code and settings inside the\n\
                 box) are kept: creating '{}' again reattaches them.",
                env.name,
                dir.display(),
                env.name,
                env.name
            );
            if prompt(&format!("Type '{}' to confirm: ", env.name))? != env.name {
                println!("Cancelled; nothing was removed.");
                return Ok(());
            }
            if envs::is_created(&dir) {
                // `down` without -v: the container goes, the volumes stay.
                run_visible(envs::compose(&config, &dir, &["down"]))?;
            }
            envs::forget(&config, file, &env.name)?;
            println!("Removed environment '{}'.", env.name);
            Ok(())
        }
        _ => Err(format!("unknown env command: {action}\n{ENV_USAGE}").into()),
    }
}

fn env_list(file: &Path, config: &Config, store: &Store) -> Result<()> {
    if config.envs.is_empty() {
        println!("No environments yet. Create one with: rattle env create <name> --dir <folder>");
        return Ok(());
    }
    let width = config.envs.iter().map(|e| e.name.len()).max().unwrap_or(0);
    for env in &config.envs {
        let dir = envs::env_dir(config, file, &env.name);
        let status = envs::status(config, &dir);
        let members = envs::members(config, env, store).len();
        let port = envs::find_template(config, &env.template)
            .ok()
            .and_then(|t| envs::base_port(&t, &dir))
            .map(|p| format!("  ports from {p}"))
            .unwrap_or_default();
        let note = match &status {
            envs::Status::Unknown(why) => format!("  ({why})"),
            _ => String::new(),
        };
        println!(
            " {:<width$}  {:<11}  {}  {members} project(s){port}{note}",
            env.name,
            status.label(),
            env.template
        );
    }
    Ok(())
}

fn env_create(file: &Path, config_file: &Path, config: &Config, rest: &[String]) -> Result<()> {
    let (mut name, mut template, mut dirs) = (None, None, Vec::new());
    let mut rest = rest.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "-t" | "--template" => {
                template = Some(rest.next().ok_or("--template needs a name")?.clone())
            }
            "--dir" => {
                let dir = rest.next().ok_or("--dir needs a folder")?;
                let path = std::fs::canonicalize(ops::expand_tilde(dir))
                    .map_err(|e| format!("cannot access '{dir}': {e}"))?;
                // Written as ~/… when under home, so the config stays readable.
                let home = std::fs::canonicalize(store::home_dir()).unwrap_or_default();
                dirs.push(match path.strip_prefix(&home) {
                    Ok(rel) if !home.as_os_str().is_empty() => format!("~/{}", rel.display()),
                    _ => path.display().to_string(),
                });
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown env create option: {other}").into())
            }
            other => name = Some(other.to_string()),
        }
    }
    let name = name.ok_or_else(|| format!("create needs an environment name\n{ENV_USAGE}"))?;
    envs::validate_name(&name)?;

    // Add the environment to config.toml unless it is already defined there.
    let env = match config.envs.iter().find(|e| e.name == name) {
        Some(env) if template.is_some() || !dirs.is_empty() => {
            return Err(format!(
                "'{}' is already in {}; edit --template/--dir there",
                env.name,
                config_file.display()
            )
            .into())
        }
        Some(env) => env.clone(),
        None => {
            let env = config::EnvDef {
                template: template.unwrap_or_else(|| "bindle".into()),
                dirs,
                ..config::EnvDef::new(&name)
            };
            envs::find_template(config, &env.template)?;
            env
        }
    };
    println!(
        "Creating environment '{name}' from template '{}'…",
        env.template
    );
    let made = envs::create(config, file, &env)?;
    if !config.envs.iter().any(|e| e.name == name) {
        config::append_env(config_file, &env)?;
        println!("Added [envs.{name}] to {}", config_file.display());
    }
    println!("Files: {}", made.dir.display());
    for (key, value) in &made.vars {
        println!("  {key}={value}");
    }
    if env.dirs.is_empty() {
        println!("Tip: add the folders whose projects belong here to `dirs` in [envs.{name}].");
    }
    println!(
        "Next: review its settings (rattle env config {name}), then start it (rattle env up {name})."
    );
    Ok(())
}

fn env_links(
    file: &Path,
    config: &Config,
    env: &config::EnvDef,
    store: &Store,
    project: Option<&store::Project>,
) -> Result<Vec<(String, String)>> {
    let template = envs::find_template(config, &env.template)?;
    let folder = match project {
        Some(p) => envs::project_folder(env, &template, store, p),
        None => template.workdir.clone(),
    };
    Ok(envs::links(
        &template,
        &envs::env_dir(config, file, &env.name),
        &folder,
    ))
}

fn print_links(
    file: &Path,
    config: &Config,
    env: &config::EnvDef,
    store: &Store,
    project: Option<&store::Project>,
) -> Result<()> {
    for (label, url) in env_links(file, config, env, store, project)? {
        println!("  {label}: {url}");
    }
    Ok(())
}

fn run_visible(mut cmd: std::process::Command) -> Result<()> {
    println!("$ {}", deps::describe(&cmd));
    let program = cmd.get_program().to_string_lossy().into_owned();
    let status = cmd
        .status()
        .map_err(|e| format!("could not run {program}: {e}"))?;
    if !status.success() {
        return Err(format!("{program} exited with {status}").into());
    }
    Ok(())
}

fn confirm(msg: &str) -> io::Result<bool> {
    Ok(matches!(prompt(msg)?.as_str(), "y" | "Y" | "yes"))
}

fn edit_config(file: &Path, arg: Option<&str>) -> Result<()> {
    let config_file = config::path_for(file);
    // Creates the commented template on first use.
    let config = Config::load(&config_file)?;
    match arg {
        Some("path") => println!("{}", config_file.display()),
        None => {
            let dir = config_file.parent().unwrap_or(Path::new("."));
            let (mut cmd, editor) =
                ops::editor_command(config.editor.as_deref(), dir, &config_file, true);
            let status = cmd
                .status()
                .map_err(|e| format!("could not run '{editor}': {e}"))?;
            if !status.success() {
                return Err(format!("'{editor}' exited with {status}").into());
            }
        }
        Some(other) => {
            return Err(format!("unknown config option: {other} (try: rattle config path)").into())
        }
    }
    Ok(())
}

fn prompt(msg: &str) -> io::Result<String> {
    print!("{msg}");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().to_string())
}
