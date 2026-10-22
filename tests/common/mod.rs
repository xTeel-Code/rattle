//! Shared helpers for the end-to-end tests: an isolated home folder per test,
//! running the real `rattle` binary, and driving the TUI inside tmux.

#![allow(dead_code)] // Each test file uses a different subset.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

pub const BIN: &str = env!("CARGO_BIN_EXE_rattle");

/// A throwaway home folder with its own registry and config.
/// `HOME` points here, so nothing touches the real `~/.rattle`.
pub struct Sandbox {
    pub root: PathBuf,
    pub home: PathBuf,
}

impl Sandbox {
    pub fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("rattle-e2e-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let home = root.join("home");
        std::fs::create_dir_all(home.join(".rattle")).unwrap();
        // Canonical, so paths compare equal to what rattle stores (/private/var on macOS).
        let root = root.canonicalize().unwrap();
        let home = root.join("home");
        Self { root, home }
    }

    pub fn registry(&self) -> PathBuf {
        self.home.join(".rattle/projects.toml")
    }

    pub fn config_file(&self) -> PathBuf {
        self.home.join(".rattle/config.toml")
    }

    pub fn write_config(&self, text: &str) {
        std::fs::write(self.config_file(), text).unwrap();
    }

    pub fn config_text(&self) -> String {
        std::fs::read_to_string(self.config_file()).unwrap_or_default()
    }

    /// Creates `rel` (and parents) under home; returns its path.
    pub fn dir(&self, rel: &str) -> PathBuf {
        let path = self.home.join(rel);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    /// Writes a file under home, creating parent folders.
    pub fn file(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.home.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path
    }

    /// Writes an executable shell script under home.
    pub fn script(&self, rel: &str, body: &str) -> PathBuf {
        let path = self.file(rel, &format!("#!/bin/sh\n{body}\n"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    /// Environment every rattle process in this sandbox gets.
    pub fn env(&self) -> Vec<(String, String)> {
        vec![
            ("HOME".into(), self.home.display().to_string()),
            ("RATTLE_FILE".into(), self.registry().display().to_string()),
            ("VISUAL".into(), String::new()),
            ("EDITOR".into(), String::new()),
        ]
    }

    pub fn cmd(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(BIN);
        cmd.args(args)
            .envs(self.env())
            .current_dir(&self.home)
            .stdin(Stdio::null());
        cmd
    }

    pub fn run(&self, args: &[&str]) -> Run {
        Run::from(self.cmd(args).output().unwrap())
    }

    /// Names of registered projects, in registry order.
    pub fn registered(&self) -> Vec<String> {
        std::fs::read_to_string(self.registry())
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.strip_prefix('[').and_then(|l| l.strip_suffix(']')))
            .map(|n| n.trim_matches('"').to_string())
            .collect()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

pub struct Run {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

impl From<Output> for Run {
    fn from(o: Output) -> Self {
        Self {
            ok: o.status.success(),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        }
    }
}

impl Run {
    /// Asserts success and returns stdout.
    #[track_caller]
    pub fn ok(self) -> String {
        assert!(
            self.ok,
            "expected success\nstdout: {}\nstderr: {}",
            self.stdout, self.stderr
        );
        self.stdout
    }

    /// Asserts failure and returns stderr.
    #[track_caller]
    pub fn err(self) -> String {
        assert!(
            !self.ok,
            "expected failure\nstdout: {}\nstderr: {}",
            self.stdout, self.stderr
        );
        self.stderr
    }
}

/// Whether `program` can be run (for tests that need cargo, uv, npm, …).
pub fn has_tool(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// The TUI running in a private tmux server. Its stdout goes to a file,
/// so `Enter` (print the path and quit) can be checked.
pub struct Tui {
    socket: String,
    out: PathBuf,
}

impl Tui {
    /// Starts `rattle` in tmux, or returns `None` (skipping the test) when
    /// tmux is missing. With `RATTLE_REQUIRE_TMUX=1` (set in CI) it panics instead.
    pub fn start(sb: &Sandbox, name: &str, extra_env: &[(&str, &str)]) -> Option<Tui> {
        Self::start_sized(sb, name, extra_env, (120, 30))
    }

    /// Like [`Tui::start`], in a terminal of `(columns, rows)`.
    pub fn start_sized(
        sb: &Sandbox,
        name: &str,
        extra_env: &[(&str, &str)],
        (columns, rows): (u16, u16),
    ) -> Option<Tui> {
        if !Command::new("tmux")
            .arg("-V")
            .output()
            .is_ok_and(|o| o.status.success())
        {
            assert!(
                std::env::var_os("RATTLE_REQUIRE_TMUX").is_none(),
                "tmux is required but missing"
            );
            eprintln!("skipping TUI test '{name}': tmux not installed");
            return None;
        }
        let socket = format!("rattle-e2e-{name}-{}", std::process::id());
        let out = sb.root.join(format!("tui-{name}.out"));
        let mut env: Vec<(String, String)> = sb.env();
        env.extend(
            extra_env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string())),
        );
        let assignments: Vec<String> = env.iter().map(|(k, v)| format!("{k}='{v}'")).collect();
        // Keep the pane open after rattle exits, so the screen can still be read.
        let script = format!(
            "{} '{}' > '{}'; echo EXIT=$? >> '{}'; sleep 600",
            assignments.join(" "),
            BIN,
            out.display(),
            out.display()
        );
        let _ = Command::new("tmux")
            .args(["-L", &socket, "kill-server"])
            .output();
        let started = Command::new("tmux")
            .args(["-L", &socket, "new-session", "-d"])
            .args(["-x", &columns.to_string(), "-y", &rows.to_string()])
            .arg(script)
            .current_dir(&sb.home)
            .status()
            .unwrap();
        assert!(started.success(), "could not start tmux");
        Some(Tui { socket, out })
    }

    fn tmux(&self, args: &[&str]) -> Output {
        Command::new("tmux")
            .args(["-L", &self.socket])
            .args(args)
            .output()
            .unwrap()
    }

    /// Sends tmux key names, e.g. `["j", "Enter", "Escape"]`.
    pub fn keys(&self, keys: &[&str]) {
        for key in keys {
            self.tmux(&["send-keys", key]);
        }
    }

    /// Types `text` literally.
    pub fn type_text(&self, text: &str) {
        self.tmux(&["send-keys", "-l", text]);
    }

    pub fn screen(&self) -> String {
        String::from_utf8_lossy(&self.tmux(&["capture-pane", "-p"]).stdout).into_owned()
    }

    /// Waits until the screen's line containing `row` satisfies `ok`; panics
    /// with the screen after 10s. For checks within a row, which a capture
    /// can catch half painted.
    #[track_caller]
    pub fn wait_for_row(&self, row: &str, ok: impl Fn(&str) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let screen = self.screen();
            if screen.lines().find(|l| l.contains(row)).is_some_and(&ok) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "row {row:?} never matched; screen:\n{screen}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Waits until `needle` is on screen; panics with the screen after 10s.
    #[track_caller]
    pub fn wait_for(&self, needle: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let screen = self.screen();
            if screen.contains(needle) {
                return screen;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {needle:?}; screen:\n{screen}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Waits until rattle has exited; returns what it printed to stdout.
    #[track_caller]
    pub fn wait_exit(&self) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let out = std::fs::read_to_string(&self.out).unwrap_or_default();
            if out.contains("EXIT=") {
                return out;
            }
            assert!(
                Instant::now() < deadline,
                "rattle did not exit; screen:\n{}",
                self.screen()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    pub fn has_exited(&self) -> bool {
        std::fs::read_to_string(&self.out)
            .unwrap_or_default()
            .contains("EXIT=")
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = self.tmux(&["kill-server"]);
    }
}

/// `path` as rattle stores it (canonical).
pub fn canon(path: &Path) -> String {
    path.canonicalize().unwrap().display().to_string()
}
