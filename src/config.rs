//! User configuration stored at `~/.rattle/config.toml` (next to the registry).
//!
//! ```toml
//! # Folders whose direct subfolders `rattle scan` checks for projects.
//! scan_dirs = ["~/code", "~/work"]
//!
//! # A subfolder counts as a project if it contains any of these.
//! markers = [".git", "Cargo.toml", "pyproject.toml", "package.json"]
//! ```

use std::fs;
use std::io;
use std::path::Path;

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

const TEMPLATE: &str = "\
# Folders whose direct subfolders `rattle scan` checks for projects.
scan_dirs = []

# A subfolder counts as a project if it contains any of these.
# Uncomment to override the defaults shown here.
# markers = [\".git\", \"Cargo.toml\", \"pyproject.toml\", \"package.json\", \"go.mod\",
#            \"pom.xml\", \"build.gradle\", \"CMakeLists.txt\", \"Makefile\"]
";

#[derive(Debug, PartialEq, Eq)]
pub struct Config {
    pub scan_dirs: Vec<String>,
    pub markers: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            scan_dirs: Vec::new(),
            markers: DEFAULT_MARKERS.iter().map(|m| m.to_string()).collect(),
        }
    }
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
                    Some((k, v)) => (k.trim().to_string(), v.trim().to_string()),
                    None => continue,
                },
            };
            if value.starts_with('[') && !value.ends_with(']') {
                pending = Some((key, value));
                continue;
            }
            let list = parse_list(&value);
            match key.as_str() {
                "scan_dirs" => config.scan_dirs = list,
                "markers" => config.markers = list,
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

/// Drops a trailing `# comment`, ignoring `#` inside quoted strings.
fn strip_comment(line: &str) -> &str {
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
}
