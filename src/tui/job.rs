//! A command running in the background (template, package manager), so the
//! TUI keeps drawing a spinner and can cancel it with Esc.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// What to do with the result once the command has finished.
pub(super) enum JobKind {
    /// `n`: register the project the template created.
    NewProject {
        name: String,
        template: String,
        target: PathBuf,
    },
    /// `p`: reload the dependency view.
    Deps,
    /// An environment start/stop/restart; afterwards refresh statuses and,
    /// if set, open this URL.
    Env { open: Option<String> },
    /// `l`: show the environment's logs.
    EnvLogs { env: String },
}

pub(super) struct Job {
    child: Child,
    stdout: Option<JoinHandle<Vec<u8>>>,
    stderr: Option<JoinHandle<Vec<u8>>>,
    started: Instant,
    /// Shown next to the spinner, e.g. "cargo add serde".
    pub(super) label: String,
    pub(super) program: String,
    pub(super) kind: JobKind,
    pub(super) cancelled: bool,
    /// Exit status, and when it was first seen.
    exited: Option<(ExitStatus, Instant)>,
}

/// How long to wait for output after the command exited (see [`Job::poll`]).
const PIPE_GRACE: Duration = Duration::from_secs(2);

const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

impl Job {
    pub(super) fn spawn(mut cmd: Command, label: String, kind: JobKind) -> Result<Job, String> {
        let program = cmd.get_program().to_string_lossy().into_owned();
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Its own process group, so cancelling also stops what it started (npm, cargo, …).
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        let mut child = cmd.spawn().map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => format!("{program} is not installed (or not on PATH)"),
            _ => format!("could not run {program}: {e}"),
        })?;
        // Drain both pipes on threads, so a chatty command never blocks on a full pipe.
        let drain = |pipe: Option<Box<dyn Read + Send>>| {
            pipe.map(|mut p| {
                std::thread::spawn(move || {
                    let mut buf = Vec::new();
                    let _ = p.read_to_end(&mut buf);
                    buf
                })
            })
        };
        let stdout = drain(
            child
                .stdout
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        );
        let stderr = drain(
            child
                .stderr
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        );
        Ok(Job {
            child,
            stdout,
            stderr,
            started: Instant::now(),
            label,
            program,
            kind,
            cancelled: false,
            exited: None,
        })
    }

    /// The command's output once it has exited, without blocking.
    ///
    /// A process the command left behind can keep its output pipes open after
    /// it exited; joining the readers would then freeze the TUI. So the output
    /// is collected once the readers are done, or given up after a grace period.
    pub(super) fn poll(&mut self) -> Option<Output> {
        let status = match self.exited {
            Some((status, _)) => status,
            None => {
                let status = self.child.try_wait().ok().flatten()?;
                self.exited = Some((status, Instant::now()));
                status
            }
        };
        let done = |h: &Option<JoinHandle<Vec<u8>>>| h.as_ref().is_none_or(|h| h.is_finished());
        let gave_up = self.exited.is_some_and(|(_, at)| at.elapsed() > PIPE_GRACE);
        if !(done(&self.stdout) && done(&self.stderr)) && !gave_up {
            return None;
        }
        let collect = |h: Option<JoinHandle<Vec<u8>>>| {
            h.filter(|h| h.is_finished())
                .and_then(|h| h.join().ok())
                .unwrap_or_default()
        };
        Some(Output {
            status,
            stdout: collect(self.stdout.take()),
            stderr: collect(self.stderr.take()),
        })
    }

    /// Stops the command and everything it started.
    pub(super) fn cancel(&mut self) {
        self.cancelled = true;
        #[cfg(unix)]
        {
            let group = format!("-{}", self.child.id());
            let killed = Command::new("kill").args(["-TERM", "--", &group]).output();
            if killed.is_ok_and(|o| o.status.success()) {
                return;
            }
        }
        let _ = self.child.kill();
    }

    pub(super) fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// "⠹ cargo add serde · 3s · Esc cancels"
    pub(super) fn status_line(&self) -> String {
        let elapsed = self.elapsed();
        let frame = SPINNER[(elapsed.as_millis() / 100) as usize % SPINNER.len()];
        if self.cancelled {
            format!("{frame} cancelling {}…", self.label)
        } else {
            format!(
                "{frame} {} · {}s · Esc cancels",
                self.label,
                elapsed.as_secs()
            )
        }
    }
}
