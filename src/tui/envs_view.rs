//! The environments popup (`v`): start, stop, open and create dev environments.

use super::draw::popup;
use super::*;
use crate::config::EnvDef;
use crate::envs::Status;

pub(super) struct EnvsView {
    pub(super) list: ListState,
    /// Logs of one environment, shown instead of the list: (env, lines, scroll).
    pub(super) logs: Option<(String, Vec<String>, usize)>,
    /// Creating a new environment: its name, then its project folders.
    pub(super) new: Option<NewEnv>,
}

pub(super) enum NewEnv {
    Name(String),
    Dirs { name: String, dirs: String },
}

pub(super) enum EnvsAction {
    Nothing,
    Close,
    Up(String),
    Stop(String),
    Restart(String),
    Logs(String),
    /// Open the environment's link number `usize` (0 = the first).
    Open(String, usize),
    Shell(String),
    Config(String),
    Create {
        name: String,
        dirs: Vec<String>,
    },
}

impl EnvsView {
    pub(super) fn new() -> Self {
        let mut list = ListState::default();
        list.select(Some(0));
        Self {
            list,
            logs: None,
            new: None,
        }
    }

    pub(super) fn on_key(&mut self, key: KeyEvent, envs: &[EnvDef]) -> EnvsAction {
        if let Some((_, lines, scroll)) = &mut self.logs {
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => {
                    *scroll = (*scroll + 1).min(lines.len().saturating_sub(1))
                }
                KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
                KeyCode::Char('G') | KeyCode::End => *scroll = lines.len().saturating_sub(1),
                KeyCode::Char('g') | KeyCode::Home => *scroll = 0,
                _ => self.logs = None,
            }
            return EnvsAction::Nothing;
        }
        if let Some(new) = &mut self.new {
            let text = match new {
                NewEnv::Name(text) | NewEnv::Dirs { dirs: text, .. } => text,
            };
            match key.code {
                KeyCode::Esc => self.new = None,
                KeyCode::Backspace => {
                    text.pop();
                }
                KeyCode::Char(c) => text.push(c),
                KeyCode::Enter => match std::mem::replace(new, NewEnv::Name(String::new())) {
                    NewEnv::Name(name) => {
                        *new = NewEnv::Dirs {
                            name: name.trim().to_string(),
                            dirs: String::new(),
                        }
                    }
                    NewEnv::Dirs { name, dirs } => {
                        self.new = None;
                        let dirs = dirs
                            .split(',')
                            .map(|d| d.trim().to_string())
                            .filter(|d| !d.is_empty())
                            .collect();
                        return EnvsAction::Create { name, dirs };
                    }
                },
                _ => {}
            }
            return EnvsAction::Nothing;
        }

        let selected = self
            .list
            .selected()
            .and_then(|i| envs.get(i))
            .map(|e| e.name.clone());
        let with = |f: fn(String) -> EnvsAction| selected.clone().map_or(EnvsAction::Nothing, f);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => EnvsAction::Close,
            KeyCode::Down | KeyCode::Char('j') => {
                self.list.select_next();
                EnvsAction::Nothing
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.list.select_previous();
                EnvsAction::Nothing
            }
            KeyCode::Char('n') => {
                self.new = Some(NewEnv::Name(String::new()));
                EnvsAction::Nothing
            }
            KeyCode::Char('u') => with(EnvsAction::Up),
            KeyCode::Char('s') => with(EnvsAction::Stop),
            KeyCode::Char('r') => with(EnvsAction::Restart),
            KeyCode::Char('l') => with(EnvsAction::Logs),
            KeyCode::Char('x') => with(EnvsAction::Shell),
            KeyCode::Char('c') => with(EnvsAction::Config),
            KeyCode::Char('o') | KeyCode::Enter => with(|e| EnvsAction::Open(e, 0)),
            KeyCode::Char(c @ '1'..='9') => selected.map_or(EnvsAction::Nothing, |e| {
                EnvsAction::Open(e, c as usize - '1' as usize)
            }),
            _ => EnvsAction::Nothing,
        }
    }
}

impl App {
    /// Re-reads which environment each project is in, and starts reading
    /// every environment's status in the background.
    pub(super) fn refresh_envs(&mut self) {
        let config = Config::load(&config::path_for(&self.file)).unwrap_or_default();
        self.project_envs = self
            .store
            .projects
            .iter()
            .filter_map(|p| envs::env_of(&config, p).map(|e| (p.name.clone(), e.name.clone())))
            .collect();
        if config.envs.is_empty() {
            self.env_status.clear();
            return;
        }
        let (tx, rx) = mpsc::channel();
        let file = self.file.clone();
        std::thread::spawn(move || {
            let statuses = config
                .envs
                .iter()
                .map(|e| {
                    (
                        e.name.clone(),
                        envs::status(&config, &envs::env_dir(&config, &file, &e.name)),
                    )
                })
                .collect();
            let _ = tx.send(statuses);
        });
        self.env_status_rx = Some(rx);
    }

    pub(super) fn tick_env_status(&mut self) {
        let Some(rx) = &self.env_status_rx else {
            return;
        };
        match rx.try_recv() {
            Ok(statuses) => {
                self.env_status = statuses.into_iter().collect();
                self.env_status_rx = None;
            }
            Err(mpsc::TryRecvError::Disconnected) => self.env_status_rx = None,
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }

    /// The links of `env`, pointing at `project`'s folder (else the workdir).
    pub(super) fn env_links(
        &self,
        config: &Config,
        env: &EnvDef,
        project: Option<&Project>,
    ) -> Vec<(String, String)> {
        let Ok(template) = envs::find_template(config, &env.template) else {
            return Vec::new();
        };
        let folder = match project {
            Some(p) => envs::project_folder(env, &template, &self.store, p),
            None => template.workdir.clone(),
        };
        envs::links(
            &template,
            &envs::env_dir(config, &self.file, &env.name),
            &folder,
        )
    }

    /// Opens `url` now if `env` runs, else starts it first and opens it after.
    fn open_in_env(&mut self, config: &Config, env: &EnvDef, url: String) {
        if self.env_status.get(&env.name) == Some(&Status::Running) {
            return self.open_url(config, &url);
        }
        self.start_env(config, env, Some(url));
    }

    pub(super) fn open_url(&mut self, config: &Config, url: &str) {
        match envs::open_url(config, url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(_) => self.set_status(format!("opened {url}"), false),
            Err(e) => self.set_status(format!("could not open {url}: {e}"), true),
        }
    }

    fn start_env(&mut self, config: &Config, env: &EnvDef, open: Option<String>) {
        let up = match envs::prepare_up(config, &self.file, env, &self.store) {
            Ok(up) => up,
            Err(e) => return self.set_status(e, true),
        };
        let mut label = format!("starting '{}'", env.name);
        if up.mounts_changed {
            label.push_str(" (mounts changed: recreating the container)");
        }
        self.spawn_env_job(up.command, label, JobKind::Env { open });
    }

    fn spawn_env_job(&mut self, cmd: std::process::Command, label: String, kind: JobKind) {
        match Job::spawn(cmd, label, kind) {
            Ok(job) => {
                self.status = None;
                self.job = Some(job);
            }
            Err(e) => self.set_status(e, true),
        }
    }

    /// `o` on a project: its environment's first link, at the project's folder.
    pub(super) fn open_project_in_env(&mut self) {
        let Some(i) = self.selected() else { return };
        let project = self.store.projects[i].clone();
        let Some(env_name) = self.project_envs.get(&project.name).cloned() else {
            return self.set_status(
                format!(
                    "'{}' isn't in an environment; add its folder to dirs in [envs.<name>] (v)",
                    project.name
                ),
                true,
            );
        };
        let config = Config::load(&config::path_for(&self.file)).unwrap_or_default();
        let Ok(env) = envs::find_env(&config, &env_name).cloned() else {
            return;
        };
        match self
            .env_links(&config, &env, Some(&project))
            .into_iter()
            .next()
        {
            Some((_, url)) => self.open_in_env(&config, &env, url),
            None => self.set_status(format!("environment '{env_name}' has no links"), true),
        }
    }

    pub(super) fn on_envs_action(&mut self, action: EnvsAction, term: &mut Term) -> Result<()> {
        let config = Config::load(&config::path_for(&self.file)).unwrap_or_default();
        let env_named = |name: &str| envs::find_env(&config, name).cloned();
        let dir_of = |name: &str| envs::env_dir(&config, &self.file, name);
        match action {
            EnvsAction::Nothing => {}
            EnvsAction::Close => self.mode = Mode::Normal,
            EnvsAction::Up(name) => {
                if let Ok(env) = env_named(&name) {
                    self.start_env(&config, &env, None);
                }
            }
            EnvsAction::Stop(name) | EnvsAction::Restart(name)
                if !envs::is_created(&dir_of(&name)) =>
            {
                self.set_status(
                    format!("'{name}' isn't created yet; press u to create and start it"),
                    true,
                )
            }
            EnvsAction::Stop(name) => {
                let cmd = envs::compose(&config, &dir_of(&name), &["stop"]);
                self.spawn_env_job(
                    cmd,
                    format!("stopping '{name}'"),
                    JobKind::Env { open: None },
                );
            }
            EnvsAction::Restart(name) => {
                let cmd = envs::compose(&config, &dir_of(&name), &["restart"]);
                self.spawn_env_job(
                    cmd,
                    format!("restarting '{name}'"),
                    JobKind::Env { open: None },
                );
            }
            EnvsAction::Logs(name) => {
                let cmd = envs::compose(
                    &config,
                    &dir_of(&name),
                    &["logs", "--tail", "300", "--no-color"],
                );
                self.spawn_env_job(
                    cmd,
                    format!("reading logs of '{name}'"),
                    JobKind::EnvLogs { env: name },
                );
            }
            EnvsAction::Open(name, n) => {
                if let Ok(env) = env_named(&name) {
                    match self.env_links(&config, &env, None).into_iter().nth(n) {
                        Some((_, url)) => self.open_in_env(&config, &env, url),
                        None => self.set_status(format!("'{name}' has no link {}", n + 1), true),
                    }
                }
            }
            EnvsAction::Shell(name) => {
                if self.env_status.get(&name) != Some(&Status::Running) {
                    self.set_status(format!("'{name}' isn't running; press u to start it"), true);
                } else if let Ok(env) = env_named(&name) {
                    let template = envs::find_template(&config, &env.template)?;
                    let mut cmd =
                        envs::shell_command(&config, &dir_of(&name), &template, &template.workdir);
                    if let Err(e) = self.run_suspended(&mut cmd, term)? {
                        self.set_status(format!("could not run {}: {e}", config.docker), true);
                    }
                }
            }
            EnvsAction::Config(name) => {
                let dir = dir_of(&name);
                if !envs::is_created(&dir) {
                    self.set_status(format!("'{name}' isn't created yet"), true);
                } else if self.open_editor(&dir, &dir.join(".env"), true, term)? {
                    self.set_status(
                        format!("saved '{name}' settings; press r to restart it with them"),
                        false,
                    );
                }
            }
            EnvsAction::Create { name, dirs } => self.create_env(&config, &name, dirs, term)?,
        }
        Ok(())
    }

    fn create_env(
        &mut self,
        config: &Config,
        name: &str,
        dirs: Vec<String>,
        term: &mut Term,
    ) -> Result<()> {
        if let Err(e) = envs::validate_name(name) {
            self.set_status(e, true);
            return Ok(());
        }
        let existing = config.envs.iter().find(|e| e.name == name).cloned();
        let env = existing.clone().unwrap_or_else(|| EnvDef {
            dirs,
            ..EnvDef::new(name)
        });
        // Downloading the template's two files only takes a moment.
        self.set_status(format!("creating environment '{name}'…"), false);
        term.draw(|f| self.draw(f))?;
        match envs::create(config, &self.file, &env) {
            Ok(_) => {
                if existing.is_none() {
                    if let Err(e) = config::append_env(&config::path_for(&self.file), &env) {
                        self.set_status(
                            format!("created, but could not add it to config: {e}"),
                            true,
                        );
                        return Ok(());
                    }
                }
                self.refresh_envs();
                self.set_status(
                    format!("created '{name}'; press c to review its settings, u to start it"),
                    false,
                );
            }
            Err(e) => self.set_status(e, true),
        }
        Ok(())
    }

    /// Handles a finished environment job (see [`App::tick_job`]).
    pub(super) fn finish_env_job(&mut self, job: &Job, output: &std::process::Output) {
        let failed = || deps::summarize(&job.program, output).err();
        match &job.kind {
            JobKind::Env { open, .. } => {
                if job.cancelled {
                    self.set_status(format!("cancelled {}", job.label), true);
                } else if let Some(e) = failed() {
                    self.set_status(e, true);
                } else {
                    self.set_status(format!("{}: done", job.label), false);
                    if let Some(url) = open {
                        let config =
                            Config::load(&config::path_for(&self.file)).unwrap_or_default();
                        self.open_url(&config, url);
                    }
                }
                self.refresh_envs();
            }
            JobKind::EnvLogs { env } => {
                if let Some(e) = failed() {
                    return self.set_status(e, true);
                }
                let text = String::from_utf8_lossy(&output.stdout);
                let lines: Vec<String> = text.lines().map(str::to_string).collect();
                let scroll = lines.len().saturating_sub(10);
                if let Mode::Envs(view) = &mut self.mode {
                    view.logs = Some((env.clone(), lines, scroll));
                }
            }
            _ => {}
        }
    }

    pub(super) fn draw_envs(&self, f: &mut Frame, view: &EnvsView) {
        let area = popup(f.area(), 90, f.area().height * 4 / 5);
        f.render_widget(Clear, area);
        let block = Block::bordered().title(" Environments ");
        let inner = block.inner(area);
        f.render_widget(block, area);
        let [body, detail, footer] = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(2),
            Constraint::Length(1),
        ])
        .areas(inner);
        let config = Config::load(&config::path_for(&self.file)).unwrap_or_default();

        if let Some((env, lines, scroll)) = &view.logs {
            let text: Vec<Line> = lines
                .iter()
                .skip(*scroll)
                .map(|l| Line::from(l.as_str()))
                .collect();
            f.render_widget(
                Paragraph::new(text)
                    .block(Block::new().title(format!(" logs: {env} (j/k scroll, any key back) "))),
                body,
            );
        } else if config.envs.is_empty() {
            f.render_widget(
                Paragraph::new(
                    "No environments yet. Press n to create one: a container for a group of \
                     projects, e.g. \"work\" with ~/code/work.",
                )
                .wrap(Wrap { trim: true })
                .dark_gray(),
                body,
            );
        } else {
            let width = config.envs.iter().map(|e| e.name.len()).max().unwrap_or(0);
            let items: Vec<ListItem> = config
                .envs
                .iter()
                .map(|env| {
                    let status = self.env_status.get(&env.name);
                    let members = self
                        .project_envs
                        .values()
                        .filter(|e| **e == env.name)
                        .count();
                    let dirs = if env.dirs.is_empty() {
                        "no folders".to_string()
                    } else {
                        env.dirs.join(", ")
                    };
                    ListItem::new(Line::from(vec![
                        Span::raw(format!("{:<width$}  ", env.name)).bold(),
                        status_span(status, true),
                        Span::raw(format!("  {members} project(s)  ")),
                        Span::raw(format!("{} · {dirs}", env.template)).dark_gray(),
                    ]))
                })
                .collect();
            let list = List::new(items)
                .highlight_style(Style::new().reversed())
                .highlight_symbol("▶ ");
            f.render_stateful_widget(list, body, &mut view.list.clone());

            // Links of the selected environment, numbered for 1-9.
            if let Some(env) = view.list.selected().and_then(|i| config.envs.get(i)) {
                let links = self.env_links(&config, env, None);
                let text: Vec<String> = links
                    .iter()
                    .enumerate()
                    .map(|(i, (name, url))| format!("{} {name}: {url}", i + 1))
                    .collect();
                let mut lines = vec![Line::from(text.join("   ")).dark_gray()];
                if let Some(Status::Unknown(why)) = self.env_status.get(&env.name) {
                    lines.push(Line::from(format!("docker: {why}")).yellow());
                }
                f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), detail);
            }
        }

        let line = if let Some(job) = &self.job {
            Line::from(format!(" {}", job.status_line()).yellow())
        } else if let Some(new) = &view.new {
            let (label, text) = match new {
                NewEnv::Name(text) => (" New environment name: ".to_string(), text),
                NewEnv::Dirs { name, dirs } => (
                    format!(" Folders for '{name}' (comma-separated, ~ ok, Enter to skip): "),
                    dirs,
                ),
            };
            let x = footer.x + (label.chars().count() + text.chars().count()) as u16;
            f.set_cursor_position((x.min(footer.right().saturating_sub(1)), footer.y));
            Line::from(vec![label.bold(), Span::raw(text.clone())])
        } else if let Some((msg, error)) = &self.status {
            let span = Span::raw(format!(" {msg}"));
            Line::from(if *error { span.red() } else { span.green() })
        } else {
            Line::from(
                " u start · s stop · r restart · o/1-9 open · x shell · l logs · c settings · n new · Esc close"
                    .dark_gray(),
            )
        };
        f.render_widget(line, footer);
    }
}

/// "● running" / "○ stopped" / "? unknown"; `long` adds the word.
pub(super) fn status_span(status: Option<&Status>, long: bool) -> Span<'static> {
    let (dot, label) = match status {
        Some(Status::Running) => ("●", "running"),
        Some(Status::Stopped) => ("○", "stopped"),
        Some(Status::NotCreated) => ("·", "not created"),
        Some(Status::Unknown(_)) => ("?", "unknown"),
        None => ("…", "checking"),
    };
    let text = if long {
        format!("{dot} {label:<11}")
    } else {
        dot.to_string()
    };
    match status {
        Some(Status::Running) => Span::raw(text).green(),
        Some(Status::Unknown(_)) => Span::raw(text).yellow(),
        _ => Span::raw(text).dark_gray(),
    }
}
