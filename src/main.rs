mod config;
mod ops;
mod store;
mod tui;

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use config::Config;
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
scan\t- will register every project found in your scan folders
\t  rattle scan [-n | --dry-run] [folder...]
\t  (folders default to scan_dirs in ~/.rattle/config.toml)

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
        Some("delete") | Some("remove") | Some("rm") => delete(&file, args.get(1)),
        Some("scan") => scan(&file, &args[1..]),
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

fn delete(file: &Path, arg: Option<&String>) -> Result<()> {
    let name = match arg {
        Some(a) => a.clone(),
        None => prompt("Project Name: ")?,
    };
    let mut store = Store::load(file)?;
    // Accept either the project name or its index from `rattle show`.
    let name = match name.parse::<usize>() {
        Ok(i) if store.find(&name).is_none() && i < store.projects.len() => {
            store.projects[i].name.clone()
        }
        _ => name,
    };
    let removed = store.remove(&name).ok_or_else(|| format!("no project named '{name}'"))?;
    store.save(file)?;
    println!("Removed '{}' (files at {} were left untouched)", removed.name, removed.path);
    Ok(())
}

fn scan(file: &Path, args: &[String]) -> Result<()> {
    let dry_run = args.iter().any(|a| a == "-n" || a == "--dry-run");
    let config_file = file.with_file_name("config.toml");
    let config = Config::load(&config_file)?;

    let mut roots: Vec<String> = args.iter().filter(|a| !a.starts_with('-')).cloned().collect();
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
    let report = ops::scan_roots(&mut store, &roots, &config.markers);
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
    Ok(())
}

fn prompt(msg: &str) -> io::Result<String> {
    print!("{msg}");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().to_string())
}
