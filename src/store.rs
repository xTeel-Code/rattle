//! Project registry stored at `~/.rattle/projects.toml`.
//!
//! Format (one table per project):
//!
//! ```toml
//! [name]
//! path = "/absolute/path/to/name"
//! ```
//!
//! Unquoted values (`path=/foo`) written by the old Python version are
//! still accepted when reading.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Default)]
pub struct Store {
    pub projects: Vec<Project>,
}

impl Store {
    /// Loads the registry, creating `~/.rattle/projects.toml` if it is missing.
    pub fn load(file: &Path) -> io::Result<Self> {
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir)?;
        }
        if !file.exists() {
            fs::write(file, "")?;
        }
        Ok(Self::parse(&fs::read_to_string(file)?))
    }

    pub fn save(&self, file: &Path) -> io::Result<()> {
        // Write to a temp file and rename so a crash never truncates the registry.
        let tmp = file.with_extension("toml.tmp");
        fs::write(&tmp, self.serialize())?;
        fs::rename(tmp, file)
    }

    pub fn parse(text: &str) -> Self {
        let mut projects = Vec::new();
        let mut current: Option<String> = None;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(header) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                current = Some(unquote(header.trim()));
            } else if let Some((key, value)) = line.split_once('=') {
                if key.trim() == "path" {
                    if let Some(name) = current.take() {
                        projects.push(Project { name, path: unquote(value.trim()) });
                    }
                }
            }
        }
        Self { projects }
    }

    pub fn serialize(&self) -> String {
        self.projects
            .iter()
            .map(|p| format!("[{}]\npath = {}\n", key(&p.name), quote(&p.path)))
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn find(&self, name: &str) -> Option<&Project> {
        self.projects.iter().find(|p| p.name == name)
    }

    pub fn add(&mut self, project: Project) -> Result<(), String> {
        if self.find(&project.name).is_some() {
            return Err(format!("project '{}' already exists", project.name));
        }
        self.projects.push(project);
        Ok(())
    }

    pub fn remove(&mut self, name: &str) -> Option<Project> {
        let idx = self.projects.iter().position(|p| p.name == name)?;
        Some(self.projects.remove(idx))
    }
}

/// Default registry location: `~/.rattle/projects.toml`.
pub fn default_path() -> PathBuf {
    home_dir().join(".rattle").join("projects.toml")
}

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn key(name: &str) -> String {
    let bare = !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if bare { name.to_string() } else { quote(name) }
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn unquote(s: &str) -> String {
    match s.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        Some(inner) => inner.replace("\\\"", "\"").replace("\\\\", "\\"),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str, path: &str) -> Project {
        Project { name: name.into(), path: path.into() }
    }

    #[test]
    fn parses_legacy_python_format() {
        let s = Store::parse("[foo]\npath=/home/me/foo\n[bar]\npath=/home/me/bar");
        assert_eq!(s.projects, vec![p("foo", "/home/me/foo"), p("bar", "/home/me/bar")]);
    }

    #[test]
    fn roundtrips() {
        let mut s = Store::default();
        s.add(p("plain", "/a/plain")).unwrap();
        s.add(p("my project", "/a/my \"quoted\" project")).unwrap();
        let back = Store::parse(&s.serialize());
        assert_eq!(back.projects, s.projects);
    }

    #[test]
    fn rejects_duplicates_and_removes() {
        let mut s = Store::default();
        s.add(p("x", "/x")).unwrap();
        assert!(s.add(p("x", "/y")).is_err());
        assert_eq!(s.remove("x"), Some(p("x", "/x")));
        assert_eq!(s.remove("x"), None);
    }
}
