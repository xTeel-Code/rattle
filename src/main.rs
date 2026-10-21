mod config;
mod deps;
mod ops;
mod store;
mod templates;
mod tui;

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use config::{Config, DeleteFiles};
use store::Store;

const HELP: &str = "\
Rattle - Project Manager Written In Rust
Usage: rattle [command] [projectName | filePath]
(no command)\t- opens the interactive TUI
tui\t- opens the interactive TUI
help\t- shows this message
show\t- will show your projects
create\t- will register a project in folder of your choosing (req. filePath)
delete\t- will delete project of your choosing (req. projectName)
\t  rattle delete <name> --files also deletes the folder (needs delete_files in config)
scan\t- will register every project found in your scan folders
\t  rattle scan [-n | --dry-run] [-d | --depth N] [folder...]
\t  (folders default to scan_dirs, depth to max_depth in ~/.rattle/config.toml)
prune\t- removes entries whose folder no longer exists (rattle prune [-n | --dry-run])
new\t- creates a project from a template and adds it
\t  rattle new <name> [-t | --template T] [--in folder]   (rattle new --list shows templates)
deps\t- lists, adds or removes a project's dependencies with its package manager
\t  rattle deps [project] [list | add [--dev] <pkg>... | remove <pkg>...]
\t  (project defaults to the current folder; cargo, npm/pnpm/yarn/bun, uv/poetry, go)
config\t- opens config.toml in $VISUAL / $EDITOR (`rattle config path` prints its path)

In the TUI, Enter prints the selected path on exit, so `cd \"$(rattle)\"` works.
Arguments that are omitted are asked for interactively.
Registry: ~/.rattle/projects.toml (override with RATTLE_FILE)
Config:   config.toml next to the registry";

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
        Some("config") => edit_config(&file, args.get(1).map(String::as_str)),
        Some("tui") => tui::run(&file),
        // The TUI draws on stderr, so stdout may be captured by `$(rattle)`.
        None if io::stdin().is_terminal() && io::stderr().is_terminal() => tui::run(&file),
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
    let width = store.projects.iter().map(|p| p.name.len()).max().unwrap_or(0);
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
    if let Some(bad) = args.iter().find(|a| a.starts_with('-') && *a != "--files" && *a != "-f") {
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
    let removed = store.remove(&name).ok_or_else(|| format!("no project named '{name}'"))?;
    store.save(file)?;
    println!("Removed '{}' (files at {} were left untouched)", removed.name, removed.path);
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
    let project = store.find(name).cloned().ok_or_else(|| format!("no project named '{name}'"))?;
    let path = ops::deletable_folder(&project, &store, &config, file)?;
    if !io::stdin().is_terminal() {
        return Err("deleting a folder needs an interactive terminal to confirm".into());
    }

    let confirmed = match config.delete_files {
        DeleteFiles::Trash => confirm(&format!("Move {} to the Trash? [y/N] ", path.display()))?,
        _ => {
            println!("This permanently deletes {} and everything in it.", path.display());
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
    let how = if config.delete_files == DeleteFiles::Trash { "moved to the Trash" } else { "deleted" };
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
            other if other.starts_with('-') => return Err(format!("unknown scan option: {other}").into()),
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
    println!("{verb} {} project(s), {} already registered", report.added.len(), report.already);

    // Offer, never force, to drop entries whose folder is gone.
    let missing: Vec<String> = store.missing().iter().map(|p| p.name.clone()).collect();
    if !missing.is_empty() {
        println!("{} registered project(s) no longer exist: {}", missing.len(), missing.join(", "));
        if !dry_run && io::stdin().is_terminal() {
            if confirm("Remove them from Rattle? (folders are already gone) [y/N] ")? {
                let gone = store.prune_missing();
                store.save(file)?;
                println!("Removed {} entr{}", gone.len(), if gone.len() == 1 { "y" } else { "ies" });
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
            "-t" | "--template" => template = Some(args.next().ok_or("--template needs a name")?.clone()),
            "--in" => parent = Some(args.next().ok_or("--in needs a folder")?.clone()),
            other if other.starts_with('-') => return Err(format!("unknown new option: {other}").into()),
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
    println!("Creating '{name}' in {} with template '{}'…", parent.display(), template.name);
    let project = templates::create(&mut store, &template, &name, &parent)?;
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
        format!("no supported manifest in {} (Cargo.toml, package.json, pyproject.toml, go.mod)", dir.display())
    })?;

    let action = rest.first().map(String::as_str).unwrap_or("list");
    let dev = rest.iter().any(|a| a == "--dev" || a == "-D");
    let packages: Vec<String> = rest.iter().skip(1).filter(|a| !a.starts_with('-')).cloned().collect();
    match action {
        "list" => {
            let deps = deps::list(&dir, manager)?;
            let noun = if deps.len() == 1 { "dependency" } else { "dependencies" };
            println!("{} · {} ({}) · {} {noun}", dir.display(), manager.name(), manager.manifest(), deps.len());
            let width = deps.iter().map(|d| d.name.len()).max().unwrap_or(0);
            for d in deps {
                let group = d.group.label();
                let group = if group.is_empty() { String::new() } else { format!("  [{group}]") };
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
                let group = listed.iter().find(|d| d.name == *name).map(|d| d.group.clone());
                let group = group.unwrap_or(if dev { deps::Group::Dev } else { deps::Group::Normal });
                run_visible(deps::remove_command(manager, &dir, name, &group))?;
            }
        }
    }
    Ok(())
}

/// Runs a package-manager command with its output shown, for the CLI.
fn run_visible(mut cmd: std::process::Command) -> Result<()> {
    println!("$ {}", deps::describe(&cmd));
    let program = cmd.get_program().to_string_lossy().into_owned();
    let status = cmd.status().map_err(|e| format!("could not run {program}: {e}"))?;
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
            let status = cmd.status().map_err(|e| format!("could not run '{editor}': {e}"))?;
            if !status.success() {
                return Err(format!("'{editor}' exited with {status}").into());
            }
        }
        Some(other) => return Err(format!("unknown config option: {other} (try: rattle config path)").into()),
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
