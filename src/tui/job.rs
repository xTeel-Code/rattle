//! Slow work in the background, so the TUI keeps drawing a spinner: a command
//! (template, package manager, docker; Esc cancels it) or a task on a thread
//! (deleting a folder, creating an environment).

use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::config::{DeleteFiles, EnvDef};

/// What to do with the result once the work has finished.
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
    /// `D` in Environments: the container is being removed; then forget it.
    EnvRemove { env: String },
    /// `D` in Projects (a task): the folder is being deleted; then the entry.
    DeleteFolder { name: String, how: DeleteFiles },
    /// `n` in Environments (a task): its files are being fetched; then it is
    /// added to the config unless it was already there.
    EnvCreate { env: EnvDef, in_config: bool },
}

/// A finished job's result.
pub(super) enum Done {
    /// A command exited: its output.
    Output(Output),
    /// A task returned.
    Task(Result<(), String>),
}

enum Work {
    Process {
        child: Child,
        stdout: Option<JoinHandle<Vec<u8>>>,
        stderr: Option<JoinHandle<Vec<u8>>>,
        /// Exit status, and when it was first seen.
        exited: Option<(ExitStatus, Instant)>,
    },
    Task(Option<JoinHandle<Result<(), String>>>),
}

pub(super) struct Job {
    work: Work,
    started: Instant,
    /// Shown next to the spinner, e.g. "cargo add serde".
    pub(super) label: String,
    pub(super) program: String,
    pub(super) kind: JobKind,
    pub(super) cancelled: bool,
    /// When Esc first asked the command to stop (SIGTERM).
    cancelled_at: Option<Instant>,
    /// SIGKILL was sent.
    forced: bool,
}

/// How long to wait for output after the command exited (see [`Job::poll`]).
const PIPE_GRACE: Duration = Duration::from_secs(2);
/// How long a cancelled command gets to stop before it is killed.
const FORCE_AFTER: Duration = Duration::from_secs(3);

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
        Ok(Job::new(
            Work::Process {
                child,
                stdout,
                stderr,
                exited: None,
            },
            label,
            program,
            kind,
        ))
    }

    /// Runs `task` on a thread. Tasks can't be cancelled: stopping a delete or
    /// a download halfway would leave things half done.
    pub(super) fn spawn_task(
        label: String,
        kind: JobKind,
        task: impl FnOnce() -> Result<(), String> + Send + 'static,
    ) -> Job {
        Job::new(
            Work::Task(Some(std::thread::spawn(task))),
            label,
            String::new(),
            kind,
        )
    }

    fn new(work: Work, label: String, program: String, kind: JobKind) -> Job {
        Job {
            work,
            started: Instant::now(),
            label,
            program,
            kind,
            cancelled: false,
            cancelled_at: None,
            forced: false,
        }
    }

    /// The result once the work is done, without blocking.
    ///
    /// For a command: a process it left behind can keep its output pipes open
    /// after it exited; joining the readers would then freeze the TUI. So the
    /// output is collected once the readers are done, or given up after a
    /// grace period. A cancelled command that ignores SIGTERM is killed here.
    pub(super) fn poll(&mut self) -> Option<Done> {
        if self
            .cancelled_at
            .is_some_and(|at| at.elapsed() > FORCE_AFTER)
        {
            self.force();
        }
        match &mut self.work {
            Work::Task(handle) => {
                if !handle.as_ref()?.is_finished() {
                    return None;
                }
                let result = handle.take()?.join();
                Some(Done::Task(
                    result.unwrap_or_else(|_| Err("the task crashed".into())),
                ))
            }
            Work::Process {
                child,
                stdout,
                stderr,
                exited,
            } => {
                let status = match exited {
                    Some((status, _)) => *status,
                    None => {
                        let status = child.try_wait().ok().flatten()?;
                        *exited = Some((status, Instant::now()));
                        status
                    }
                };
                let done =
                    |h: &Option<JoinHandle<Vec<u8>>>| h.as_ref().is_none_or(|h| h.is_finished());
                let gave_up = exited.is_some_and(|(_, at)| at.elapsed() > PIPE_GRACE);
                if !(done(stdout) && done(stderr)) && !gave_up {
                    return None;
                }
                let collect = |h: Option<JoinHandle<Vec<u8>>>| {
                    h.filter(|h| h.is_finished())
                        .and_then(|h| h.join().ok())
                        .unwrap_or_default()
                };
                Some(Done::Output(Output {
                    status,
                    stdout: collect(stdout.take()),
                    stderr: collect(stderr.take()),
                }))
            }
        }
    }

    pub(super) fn can_cancel(&self) -> bool {
        matches!(self.work, Work::Process { .. })
    }

    /// Esc: asks the command (and everything it started) to stop with SIGTERM;
    /// a second Esc, or [`FORCE_AFTER`] without stopping, kills them.
    pub(super) fn cancel(&mut self) {
        if !self.can_cancel() {
            return;
        }
        if self.cancelled {
            return self.force();
        }
        self.cancelled = true;
        self.cancelled_at = Some(Instant::now());
        self.signal("-TERM");
    }

    /// Kills the command and everything it started (SIGKILL), at once.
    pub(super) fn force(&mut self) {
        if self.forced || !self.can_cancel() {
            return;
        }
        self.cancelled = true;
        self.forced = true;
        self.cancelled_at = None;
        self.signal("-KILL");
    }

    fn signal(&mut self, signal: &str) {
        let Work::Process { child, .. } = &mut self.work else {
            return;
        };
        #[cfg(unix)]
        {
            let group = format!("-{}", child.id());
            let sent = Command::new("kill").args([signal, "--", &group]).output();
            if sent.is_ok_and(|o| o.status.success()) {
                return;
            }
        }
        let _ = signal;
        let _ = child.kill();
    }

    pub(super) fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// "⠹ cargo add serde · 3s · Esc cancels"
    pub(super) fn status_line(&self) -> String {
        let elapsed = self.elapsed();
        let frame = SPINNER[(elapsed.as_millis() / 100) as usize % SPINNER.len()];
        if self.forced {
            format!("{frame} stopping {}…", self.label)
        } else if self.cancelled {
            format!("{frame} cancelling {}… (Esc again to force)", self.label)
        } else if self.can_cancel() {
            format!(
                "{frame} {} · {}s · Esc cancels",
                self.label,
                elapsed.as_secs()
            )
        } else {
            format!("{frame} {} · {}s", self.label, elapsed.as_secs())
        }
    }
}
