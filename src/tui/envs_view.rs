//! The Environments tab: create, start, stop, open, change and remove dev
//! environments (one container per group of projects).

use super::draw::fit_hints;
use super::*;
use crate::config::EnvDef;
use crate::envs::Status;

pub(super) struct EnvsView {
    pub(super) list: ListState,
    /// Logs of one environment, shown in the detail pane: (env, lines, scroll).
    pub(super) logs: Option<(String, Vec<String>, usize)>,
    /// A line being typed in the footer.
    pub(super) input: Option<Input>,
}

pub(super) enum Input {
    /// `n`, step 1: the new environment's name.
    NewName(String),
    /// `n`, step 2: its project folders.
    NewDirs { name: String, dirs: String },
    /// `d`: an environment's project folders.
    EditDirs { env: String, dirs: String },
    /// `D`: the environment's name, typed to confirm removing it.
    ConfirmRemove { env: String, typed: String },
}

pub(super) enum EnvsAction {
    Nothing,
    /// Back to the Projects tab.
    Leave,
    Help,
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
    SetDirs {
        env: String,
        dirs: Vec<String>,
    },
    ToggleMount(String),
    Remove(String),
}

const HINTS: &[(&str, u8)] = &[
    ("u start", 0),
    ("s stop", 1),
    ("r restart", 2),
    ("o open", 0),
    ("x shell", 2),
    ("l logs", 2),
    ("c settings", 3),
    ("n new", 1),
    ("d folders", 3),
    ("m mount", 4),
    ("D remove", 4),
    ("Tab projects", 0),
    ("? keys", 0),
];

fn split_dirs(text: &str) -> Vec<String> {
    text.split(',')
        .map(|d| d.trim().to_string())
        .filter(|d| !d.is_empty())
        .collect()
}

impl EnvsView {
    pub(super) fn new() -> Self {
        let mut list = ListState::default();
        list.select(Some(0));
        Self {
            list,
            logs: None,
            input: None,
        }
    }

    pub(super) fn selected<'a>(&self, envs: &'a [EnvDef]) -> Option<&'a EnvDef> {
        self.list.selected().and_then(|i| envs.get(i))
    }

    pub(super) fn on_key(&mut self, key: KeyEvent, envs: &[EnvDef]) -> EnvsAction {
        if let Some((_, lines, scroll)) = &mut self.logs {
            let last = lines.len().saturating_sub(1);
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => *scroll = (*scroll + 1).min(last),
                KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
                KeyCode::Char('G') | KeyCode::End => *scroll = last,
                KeyCode::Char('g') | KeyCode::Home => *scroll = 0,
                _ => self.logs = None,
            }
            return EnvsAction::Nothing;
        }
        if self.input.is_some() {
            return self.on_input_key(key);
        }

        let selected = self.selected(envs).map(|e| e.name.clone());
        let with = |f: fn(String) -> EnvsAction| selected.clone().map_or(EnvsAction::Nothing, f);
        match key.code {
            KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab | KeyCode::Char('v') => {
                EnvsAction::Leave
            }
            KeyCode::Char('?') => EnvsAction::Help,
            KeyCode::Down | KeyCode::Char('j') => {
                self.list.select_next();
                EnvsAction::Nothing
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.list.select_previous();
                EnvsAction::Nothing
            }
            KeyCode::Char('g') | KeyCode::Home => {
                self.list.select_first();
                EnvsAction::Nothing
            }
            KeyCode::Char('G') | KeyCode::End => {
                self.list.select_last();
                EnvsAction::Nothing
            }
            KeyCode::Char('n') => {
                self.input = Some(Input::NewName(String::new()));
                EnvsAction::Nothing
            }
            KeyCode::Char('d') => {
                if let Some(env) = self.selected(envs) {
                    self.input = Some(Input::EditDirs {
                        env: env.name.clone(),
                        dirs: env.dirs.join(", "),
                    });
                }
                EnvsAction::Nothing
            }
            KeyCode::Char('D') => {
                if let Some(env) = selected {
                    self.input = Some(Input::ConfirmRemove {
                        env,
                        typed: String::new(),
                    });
                }
                EnvsAction::Nothing
            }
            KeyCode::Char('u') => with(EnvsAction::Up),
            KeyCode::Char('s') => with(EnvsAction::Stop),
            KeyCode::Char('r') => with(EnvsAction::Restart),
            KeyCode::Char('l') => with(EnvsAction::Logs),
            KeyCode::Char('x') => with(EnvsAction::Shell),
            KeyCode::Char('c') => with(EnvsAction::Config),
            KeyCode::Char('m') => with(EnvsAction::ToggleMount),
            KeyCode::Char('o') | KeyCode::Enter => with(|e| EnvsAction::Open(e, 0)),
            KeyCode::Char(c @ '1'..='9') => selected.map_or(EnvsAction::Nothing, |e| {
                EnvsAction::Open(e, c as usize - '1' as usize)
            }),
            _ => EnvsAction::Nothing,
        }
    }

    /// Typing in the footer; Enter moves to the next step or finishes.
    fn on_input_key(&mut self, key: KeyEvent) -> EnvsAction {
        match key.code {
            KeyCode::Esc => self.input = None,
            KeyCode::Backspace => {
                if let Some(input) = &mut self.input {
                    input_text(input).pop();
                }
            }
            KeyCode::Char(c) => {
                if let Some(input) = &mut self.input {
                    input_text(input).push(c);
                }
            }
            KeyCode::Enter => {
                return match self.input.take() {
                    Some(Input::NewName(name)) => {
                        self.input = Some(Input::NewDirs {
                            name: name.trim().to_string(),
                            dirs: String::new(),
                        });
                        EnvsAction::Nothing
                    }
                    Some(Input::NewDirs { name, dirs }) => EnvsAction::Create {
                        name,
                        dirs: split_dirs(&dirs),
                    },
                    Some(Input::EditDirs { env, dirs }) => EnvsAction::SetDirs {
                        env,
                        dirs: split_dirs(&dirs),
                    },
                    Some(Input::ConfirmRemove { env, typed }) if typed == env => {
                        EnvsAction::Remove(env)
                    }
                    Some(Input::ConfirmRemove { .. }) => {
                        // A wrong name cancels: nothing to undo.
                        EnvsAction::Nothing
                    }
                    None => EnvsAction::Nothing,
                };
            }
            _ => {}
        }
        EnvsAction::Nothing
    }
}

/// The text being edited by `input`.
fn input_text(input: &mut Input) -> &mut String {
    match input {
        Input::NewName(text)
        | Input::NewDirs { dirs: text, .. }
        | Input::EditDirs { dirs: text, .. }
        | Input::ConfirmRemove { typed: text, .. } => text,
    }
}

/// `path` with the home folder written as `~`.
fn tilde(path: &Path) -> String {
    let home = crate::store::home_dir();
    match path.strip_prefix(&home) {
        Ok(rel) if !home.as_os_str().is_empty() => format!("~/{}", rel.display()),
        _ => path.display().to_string(),
    }
}

impl App {
    fn config(&self) -> Config {
        Config::load(&config::path_for(&self.file)).unwrap_or_default()
    }

    /// Re-reads which environment each project is in, and starts reading
    /// every environment's status in the background.
    pub(super) fn refresh_envs(&mut self) {
        let config = self.config();
        self.project_envs = self
            .store
            .projects
            .iter()
            .filter_map(|p| envs::env_of(&config, p).map(|e| (p.name.clone(), e.name.clone())))
            .collect();
        // Keep the selection on an existing environment.
        let count = config.envs.len();
        let sel = self
            .envs_view
            .list
            .selected()
            .unwrap_or(0)
            .min(count.saturating_sub(1));
        self.envs_view.list.select((count > 0).then_some(sel));
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
                    "'{}' isn't in an environment; add its folder in the Environments tab (Tab, d)",
                    project.name
                ),
                true,
            );
        };
        let config = self.config();
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

    /// A key in the Environments tab.
    pub(super) fn on_envs_key(&mut self, key: KeyEvent, term: &mut Term) -> Result<()> {
        let config = self.config();
        let removing = matches!(self.envs_view.input, Some(Input::ConfirmRemove { .. }));
        let action = self.envs_view.on_key(key, &config.envs);
        self.status = None;
        if removing && self.envs_view.input.is_none() && matches!(action, EnvsAction::Nothing) {
            self.set_status("cancelled; nothing was removed", false);
        }
        self.on_envs_action(&config, action, term)
    }

    fn on_envs_action(
        &mut self,
        config: &Config,
        action: EnvsAction,
        term: &mut Term,
    ) -> Result<()> {
        let env_named = |name: &str| envs::find_env(config, name).cloned();
        let dir_of = |name: &str| envs::env_dir(config, &self.file, name);
        let table = |name: &str| format!("envs.{name}");
        match action {
            EnvsAction::Nothing => {}
            EnvsAction::Leave => self.tab = Tab::Projects,
            EnvsAction::Help => self.mode = Mode::Help { scroll: 0 },
            EnvsAction::Up(name) => {
                if let Ok(env) = env_named(&name) {
                    self.start_env(config, &env, None);
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
                let cmd = envs::compose(config, &dir_of(&name), &["stop"]);
                self.spawn_env_job(
                    cmd,
                    format!("stopping '{name}'"),
                    JobKind::Env { open: None },
                );
            }
            EnvsAction::Restart(name) => {
                let cmd = envs::compose(config, &dir_of(&name), &["restart"]);
                self.spawn_env_job(
                    cmd,
                    format!("restarting '{name}'"),
                    JobKind::Env { open: None },
                );
            }
            EnvsAction::Logs(name) => {
                let cmd = envs::compose(
                    config,
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
                    match self.env_links(config, &env, None).into_iter().nth(n) {
                        Some((_, url)) => self.open_in_env(config, &env, url),
                        None => self.set_status(format!("'{name}' has no link {}", n + 1), true),
                    }
                }
            }
            EnvsAction::Shell(name) => {
                if self.env_status.get(&name) != Some(&Status::Running) {
                    self.set_status(format!("'{name}' isn't running; press u to start it"), true);
                } else if let Ok(env) = env_named(&name) {
                    let template = envs::find_template(config, &env.template)?;
                    let mut cmd =
                        envs::shell_command(config, &dir_of(&name), &template, &template.workdir);
                    if let Err(e) = self.run_suspended(&mut cmd, term)? {
                        self.set_status(format!("could not run {}: {e}", config.docker), true);
                    }
                }
            }
            EnvsAction::Config(name) => {
                let dir = dir_of(&name);
                if !envs::is_created(&dir) {
                    self.set_status(
                        format!("'{name}' isn't created yet; press u to start it"),
                        true,
                    );
                } else if self.open_editor(&dir, &dir.join(".env"), true, term)? {
                    self.set_status(
                        format!("saved '{name}' settings; press r to restart it with them"),
                        false,
                    );
                }
            }
            EnvsAction::Create { name, dirs } => self.create_env(config, &name, dirs, term)?,
            EnvsAction::SetDirs { env, dirs } => {
                let value = config::toml_list(&dirs);
                match config::save_table_value(
                    &config::path_for(&self.file),
                    &table(&env),
                    "dirs",
                    &value,
                ) {
                    Ok(()) => {
                        self.refresh_envs();
                        self.set_status(self.apply_hint(&env, "folders saved"), false);
                    }
                    Err(e) => self.set_status(format!("could not save folders: {e}"), true),
                }
            }
            EnvsAction::ToggleMount(name) => {
                let Ok(env) = env_named(&name) else {
                    return Ok(());
                };
                let mount = !env.mount;
                match config::save_table_value(
                    &config::path_for(&self.file),
                    &table(&name),
                    "mount",
                    &mount.to_string(),
                ) {
                    Ok(()) => {
                        self.refresh_envs();
                        let what = if mount {
                            "projects will be mounted"
                        } else {
                            "projects won't be mounted"
                        };
                        self.set_status(self.apply_hint(&name, what), false);
                    }
                    Err(e) => self.set_status(format!("could not save: {e}"), true),
                }
            }
            EnvsAction::Remove(name) => {
                let dir = dir_of(&name);
                if envs::is_created(&dir) {
                    // `down` without -v: the container goes, the volumes stay.
                    let cmd = envs::compose(config, &dir, &["down"]);
                    self.spawn_env_job(
                        cmd,
                        format!("removing '{name}'"),
                        JobKind::EnvRemove { env: name },
                    );
                } else {
                    self.finish_remove(&name);
                }
            }
        }
        Ok(())
    }

    /// "folders saved; press r to restart 'work' with them" (when it runs).
    fn apply_hint(&self, env: &str, done: &str) -> String {
        if self.env_status.get(env) == Some(&Status::Running) {
            format!("{done}; press r to restart '{env}' with them")
        } else {
            format!("{done}; they apply when '{env}' starts")
        }
    }

    fn finish_remove(&mut self, name: &str) {
        let config = self.config();
        match envs::forget(&config, &self.file, name) {
            Ok(()) => {
                self.refresh_envs();
                self.set_status(
                    format!("removed '{name}'; its volumes are kept (creating '{name}' again reattaches them)"),
                    false,
                );
            }
            Err(e) => self.set_status(e, true),
        }
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
                // Select the new environment.
                let index = self.config().envs.iter().position(|e| e.name == name);
                self.envs_view.list.select(index);
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
            JobKind::Env { open } => {
                if job.cancelled {
                    self.set_status(format!("cancelled {}", job.label), true);
                } else if let Some(e) = failed() {
                    self.set_status(e, true);
                } else {
                    self.set_status(format!("{}: done", job.label), false);
                    if let Some(url) = open {
                        let config = self.config();
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
                self.envs_view.logs = Some((env.clone(), lines, scroll));
            }
            JobKind::EnvRemove { env } => {
                if job.cancelled {
                    self.set_status(
                        format!("cancelled removing '{env}'; nothing was removed"),
                        true,
                    );
                } else if let Some(e) = failed() {
                    self.set_status(
                        format!("could not remove the container, so '{env}' was kept: {e}"),
                        true,
                    );
                } else {
                    self.finish_remove(env);
                }
            }
            _ => {}
        }
    }

    /// The Environments tab: the list on the left, details of the selected one on the right.
    pub(super) fn draw_envs_tab(&self, f: &mut Frame, area: Rect) {
        let config = self.config();
        let view = &self.envs_view;
        let [list_area, detail_area] = if area.width >= 90 {
            Layout::horizontal([Constraint::Percentage(40), Constraint::Min(0)]).areas(area)
        } else {
            Layout::vertical([Constraint::Percentage(40), Constraint::Min(0)]).areas(area)
        };

        let title = format!(" Environments ({}) ", config.envs.len());
        if config.envs.is_empty() {
            f.render_widget(
                Paragraph::new("No environments yet.".dark_gray())
                    .block(Block::bordered().title(title)),
                list_area,
            );
            let help = vec![
                Line::from("An environment is a container for a group of projects,"),
                Line::from("e.g. \"work\" for ~/code/work and \"personal\" for side projects."),
                Line::from("Each has its own tools, settings and storage."),
                Line::from(""),
                Line::from(vec!["Press ".into(), "n".bold(), " to create one.".into()]),
            ];
            f.render_widget(
                Paragraph::new(help)
                    .wrap(Wrap { trim: true })
                    .block(Block::bordered().title(" About ")),
                detail_area,
            );
            return;
        }

        let width = config.envs.iter().map(|e| e.name.len()).max().unwrap_or(0);
        let items: Vec<ListItem> = config
            .envs
            .iter()
            .map(|env| {
                let members = self
                    .project_envs
                    .values()
                    .filter(|e| **e == env.name)
                    .count();
                ListItem::new(Line::from(vec![
                    Span::raw(format!("{:<width$}  ", env.name)).bold(),
                    status_span(self.env_status.get(&env.name), true),
                    Span::raw(format!(" {members} project(s)")).dark_gray(),
                ]))
            })
            .collect();
        let list = List::new(items)
            .block(Block::bordered().title(title))
            .highlight_style(Style::new().reversed())
            .highlight_symbol("▶ ");
        f.render_stateful_widget(list, list_area, &mut view.list.clone());

        let Some(env) = view.selected(&config.envs) else {
            return;
        };
        if let Some((name, lines, scroll)) = &view.logs {
            let text: Vec<Line> = lines
                .iter()
                .skip(*scroll)
                .map(|l| Line::from(l.as_str()))
                .collect();
            f.render_widget(
                Paragraph::new(text).block(
                    Block::bordered().title(format!(" logs: {name} · j/k scroll · any key back ")),
                ),
                detail_area,
            );
            return;
        }

        let label = |s: &str| Span::raw(format!("{s:<10}")).dark_gray();
        let dir = envs::env_dir(&config, &self.file, &env.name);
        let mut members: Vec<&str> = self
            .project_envs
            .iter()
            .filter(|(_, e)| **e == env.name)
            .map(|(p, _)| p.as_str())
            .collect();
        members.sort_unstable();
        let mut lines = vec![
            Line::from(vec![
                label("status"),
                status_span(self.env_status.get(&env.name), true),
            ]),
            Line::from(vec![label("template"), Span::raw(env.template.clone())]),
            Line::from(vec![
                label("folders"),
                Span::raw(if env.dirs.is_empty() {
                    "none yet".to_string()
                } else {
                    env.dirs.join(", ")
                }),
                "  (d)".dark_gray(),
            ]),
            Line::from(vec![
                label("mount"),
                Span::raw(if env.mount {
                    "yes: its projects are mounted"
                } else {
                    "no: code lives only inside"
                }),
                "  (m)".dark_gray(),
            ]),
            Line::from(vec![
                label("projects"),
                Span::raw(if members.is_empty() {
                    "none".to_string()
                } else {
                    members.join(", ")
                }),
            ]),
            Line::from(vec![label("files"), Span::raw(tilde(&dir))]),
        ];
        let links = self.env_links(&config, env, None);
        for (i, (name, url)) in links.iter().enumerate() {
            let head = if i == 0 { label("links") } else { label("") };
            lines.push(Line::from(vec![
                head,
                format!("{} ", i + 1).bold(),
                Span::raw(format!("{name}  {url}")),
            ]));
        }
        if let Some(Status::Unknown(why)) = self.env_status.get(&env.name) {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                label("docker"),
                Span::raw(why.clone()).yellow(),
            ]));
        }
        f.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(Block::bordered().title(format!(" {} ", env.name))),
            detail_area,
        );
    }

    /// The footer of the Environments tab: a running job, the line being
    /// typed, the last message, or the keys.
    pub(super) fn draw_envs_footer(&self, f: &mut Frame, area: Rect) {
        let line = if let Some(job) = &self.job {
            Line::from(format!(" {}", job.status_line()).yellow())
        } else if let Some(input) = &self.envs_view.input {
            let (label, text) = match input {
                Input::NewName(text) => (" New environment name: ".to_string(), text),
                Input::NewDirs { name, dirs } => (
                    format!(" Folders for '{name}' (comma-separated, ~ ok, Enter to skip): "),
                    dirs,
                ),
                Input::EditDirs { env, dirs } => {
                    (format!(" Folders for '{env}' (comma-separated): "), dirs)
                }
                Input::ConfirmRemove { env, typed } => (
                    format!(" Remove '{env}' (container and files; volumes stay)? Type '{env}': "),
                    typed,
                ),
            };
            let x = area.x + (label.chars().count() + text.chars().count()) as u16;
            f.set_cursor_position((x.min(area.right().saturating_sub(1)), area.y));
            let label = if matches!(input, Input::ConfirmRemove { .. }) {
                label.red().bold()
            } else {
                label.bold()
            };
            Line::from(vec![label, Span::raw(text.clone())])
        } else if let Some((msg, error)) = &self.status {
            let span = Span::raw(format!(" {msg}"));
            Line::from(if *error { span.red() } else { span.green() })
        } else {
            Line::from(fit_hints(usize::from(area.width), HINTS).dark_gray())
        };
        f.render_widget(line, area);
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
