mod config;
mod store;

use std::collections::HashSet;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use config::Config;
use store::{Project, Store};

const HELP: &str = "\
Rattle - Project Manager Written In Rust
Usage: rattle [command] [projectName | filePath]
help\t- shows this message
show\t- will show your projects
create\t- will register a project in folder of your choosing (req. filePath)
delete\t- will delete project of your choosing (req. projectName)
scan\t- will register every project found in your scan folders
\t  rattle scan [-n | --dry-run] [folder...]
\t  (folders default to scan_dirs in ~/.rattle/config.toml)

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
    if raw.is_empty() {
        return Err("no path given".into());
    }
    let path = std::fs::canonicalize(expand_tilde(&raw))
        .map_err(|e| format!("cannot access '{raw}': {e}"))?;
    if !path.is_dir() {
        return Err(format!("'{}' is not a directory", path.display()).into());
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or("cannot derive a project name from that path")?;

    let mut store = Store::load(file)?;
    store.add(Project { name: name.clone(), path: path.to_string_lossy().into_owned() })?;
    store.save(file)?;
    println!("Added '{name}' -> {}", path.display());
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
    let mut known: HashSet<String> = store.projects.iter().map(|p| p.path.clone()).collect();
    let (mut added, mut already) = (0, 0);

    for raw in &roots {
        let root = match std::fs::canonicalize(expand_tilde(raw)) {
            Ok(r) if r.is_dir() => r,
            _ => {
                eprintln!("warning: skipping '{raw}': not an accessible directory");
                continue;
            }
        };
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir() && !is_hidden(p))
            .filter(|p| config.markers.iter().any(|m| p.join(m).exists()))
            .collect();
        dirs.sort();

        for dir in dirs {
            let dir = std::fs::canonicalize(&dir)?;
            let path = dir.to_string_lossy().into_owned();
            if known.contains(&path) {
                already += 1;
                continue;
            }
            let name = dir.file_name().unwrap_or_default().to_string_lossy().into_owned();
            if let Some(existing) = store.find(&name) {
                eprintln!("warning: skipping {path}: name '{name}' is already used by {}", existing.path);
                continue;
            }
            println!(" + {name}  {path}");
            store.add(Project { name, path: path.clone() })?;
            known.insert(path);
            added += 1;
        }
    }

    if added > 0 && !dry_run {
        store.save(file)?;
    }
    let verb = if dry_run { "Would add" } else { "Added" };
    println!("{verb} {added} project(s), {already} already registered");
    Ok(())
}

fn is_hidden(p: &Path) -> bool {
    p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))
}

fn prompt(msg: &str) -> io::Result<String> {
    print!("{msg}");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

fn expand_tilde(p: &str) -> PathBuf {
    match p.strip_prefix("~") {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            store::home_dir().join(rest.trim_start_matches('/'))
        }
        _ => PathBuf::from(p),
    }
}
