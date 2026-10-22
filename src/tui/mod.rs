//! Interactive full-screen project browser.
//!
//! Drawn on stderr so the selected path can be printed to stdout on exit,
//! which makes `cd "$(rattle)"` work.

use std::collections::HashMap;
use std::io::{self, Stderr};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, Clear as ClearScreen, ClearType, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::config::{self, Config, DeleteFiles};
use crate::deps::{self, Dep, Manager};
use crate::envs;
use crate::ops;
use crate::settings::{FormAction, SettingsForm};
use crate::store::{Project, Store};
use crate::templates::{self, Template};

mod deps_view;
mod draw;
mod envs_view;
mod job;

use deps_view::{DepsAction, DepsView};
use envs_view::EnvsView;
use job::{Done, Job, JobKind};

type Term = Terminal<CrosstermBackend<Stderr>>;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The `?` key list; entries with an empty key are section headings.
const KEYS: &[(&str, &str)] = &[
    ("", "Both tabs"),
    ("Tab / v", "switch Projects ↔ Environments"),
    ("↑/↓ j/k", "move   g/G first / last"),
    ("?", "toggle this help"),
    ("q", "quit"),
    ("", "Projects"),
    ("Enter", "print path & quit (cd \"$(rattle)\")"),
    ("/", "filter"),
    ("a", "add project folder"),
    ("n", "new project from a template"),
    ("p", "dependencies: list, add, remove"),
    ("o", "open project in its environment"),
    ("e", "open in editor (config, $VISUAL, $EDITOR)"),
    ("d", "delete entry (files are kept)"),
    ("D", "delete entry and folder (delete_files)"),
    ("X", "remove entries whose folder is gone"),
    ("s", "scan folders from config.toml"),
    ("c", "edit config.toml (reloads & rescans)"),
    (",", "settings form"),
    ("Esc", "clear the filter, else quit"),
    ("", "Environments"),
    ("u s r", "start, stop, restart"),
    ("o 1-9", "open a link (starts it if needed)"),
    ("x / l", "shell in it / its logs"),
    ("c", "edit its settings (.env)"),
    ("n", "new environment"),
    ("d / m", "change its folders / toggle mounting"),
    ("D", "remove it (container, files; volumes stay)"),
    ("Esc", "back to Projects"),
];

/// Enter presses this soon after the TUI opens are leftovers from launching it.
const STARTUP_ENTER_GUARD: Duration = Duration::from_millis(500);
/// Each ignored Enter extends the guard by this much, covering key repeat.
const ENTER_REPEAT_GAP: Duration = Duration::from_millis(250);

enum Mode {
    Normal,
    Filter,
    Add(String),
    ConfirmDelete,
    ConfirmPrune,
    /// Deleting the selected project's folder; `input` is the typed name
    /// (only needed for permanent deletion).
    ConfirmDeleteFiles {
        path: PathBuf,
        how: DeleteFiles,
        input: String,
    },
    /// `n`, step 1: typing the new project's name.
    NewName {
        input: String,
        parent: PathBuf,
    },
    /// `n`, step 2: picking a template.
    NewTemplate {
        name: String,
        parent: PathBuf,
        templates: Vec<Template>,
        list: ListState,
    },
    /// `p`: the selected project's dependencies.
    Deps(Box<DepsView>),
    /// `,`: the settings form.
    Settings(Box<SettingsForm>),
    /// `?`: the key list, scrolled by `scroll` rows when it doesn't fit.
    Help {
        scroll: usize,
    },
}

struct App {
    file: PathBuf,
    store: Store,
    filter: String,
    /// Indices into `store.projects` that match `filter`.
    visible: Vec<usize>,
    list: ListState,
    mode: Mode,
    status: Option<(String, bool)>,
    chosen: Option<String>,
    quit: bool,
    /// A template or package-manager command running in the background.
    job: Option<Job>,
    /// The environment each project belongs to (project name → env name).
    project_envs: HashMap<String, String>,
    /// Last known status of each environment.
    env_status: HashMap<String, envs::Status>,
    /// Statuses being read on a background thread (`docker compose ps` is slow).
    env_status_rx: Option<Receiver<Vec<(String, envs::Status)>>>,
    /// Which tab is shown: Projects or Environments.
    tab: Tab,
    /// The Environments tab's state (kept while switching tabs).
    envs_view: EnvsView,
    /// config.toml, read once and on changes (see [`App::reload_config_cache`]).
    config: Config,
    /// Statuses were asked for while a refresh was still running.
    env_refresh_again: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Projects,
    Envs,
}

pub fn run(file: &Path) -> Result<()> {
    let store = Store::load(file)?;
    let mut app = App {
        file: file.to_path_buf(),
        store,
        filter: String::new(),
        visible: Vec::new(),
        list: ListState::default(),
        mode: Mode::Normal,
        status: None,
        chosen: None,
        quit: false,
        job: None,
        project_envs: HashMap::new(),
        env_status: HashMap::new(),
        env_status_rx: None,
        tab: Tab::Projects,
        envs_view: EnvsView::new(),
        config: Config::load(&config::path_for(file))?,
        env_refresh_again: false,
    };
    app.refilter();

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore();
        default_hook(info);
    }));

    let mut term = setup()?;
    let result = app.auto_scan(&mut term).and_then(|()| {
        app.refresh_envs();
        app.event_loop(&mut term)
    });
    restore()?;
    result?;

    if let Some(path) = app.chosen {
        println!("{path}");
    }
    Ok(())
}

/// Enter, or what an Enter typed before raw mode arrives as: the terminal
/// turns it into a newline, which crossterm reports as Ctrl+J (or Ctrl+M).
fn is_enter_like(key: &KeyEvent) -> bool {
    key.code == KeyCode::Enter
        || (key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('j' | 'm')))
}

fn setup() -> io::Result<Term> {
    enable_raw_mode()?;
    execute!(io::stderr(), EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(io::stderr()))
}

fn restore() -> io::Result<()> {
    disable_raw_mode()?;
    execute!(io::stderr(), LeaveAlternateScreen)
}

impl App {
    fn event_loop(&mut self, term: &mut Term) -> Result<()> {
        // The Enter that launched rattle can reach the TUI too (key repeat while
        // it is still held, or typed twice). Enter picks a project and quits, so
        // the TUI would vanish before it was seen. Ignore Enter right after
        // start, and keep ignoring it for as long as it keeps repeating.
        let mut enter_guard = Instant::now() + STARTUP_ENTER_GUARD;
        while !self.quit {
            term.draw(|f| self.draw(f))?;
            // While a job runs or statuses load, wake up regularly to animate
            // it and notice it finishing.
            if self.job.is_some() || self.env_status_rx.is_some() {
                self.tick_job();
                self.tick_env_status();
                if !event::poll(Duration::from_millis(100))? {
                    continue;
                }
            }
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                if !is_enter_like(&key) {
                    // Any other key means someone is using the TUI: Enter counts from now on.
                    enter_guard = Instant::now();
                } else if matches!(self.mode, Mode::Normal) && Instant::now() < enter_guard {
                    enter_guard = Instant::now() + ENTER_REPEAT_GAP;
                    continue;
                }
                self.on_key(key, term)?;
            }
        }
        Ok(())
    }

    fn selected(&self) -> Option<usize> {
        self.list
            .selected()
            .and_then(|i| self.visible.get(i).copied())
    }

    fn refilter(&mut self) {
        let needle = self.filter.to_lowercase();
        self.visible = (0..self.store.projects.len())
            .filter(|&i| {
                let p = &self.store.projects[i];
                p.name.to_lowercase().contains(&needle) || p.path.to_lowercase().contains(&needle)
            })
            .collect();
        let sel = self
            .list
            .selected()
            .unwrap_or(0)
            .min(self.visible.len().saturating_sub(1));
        self.list.select((!self.visible.is_empty()).then_some(sel));
    }

    fn set_status(&mut self, msg: impl Into<String>, error: bool) {
        self.status = Some((msg.into(), error));
    }

    /// Re-reads config.toml into the cache. Everything in the TUI uses the
    /// cache; this runs at start and whenever the config may have changed
    /// (`c`, `,`, `s`, deleting, environment edits). On error the old one stays.
    pub(super) fn reload_config_cache(&mut self) -> io::Result<()> {
        self.config = Config::load(&config::path_for(&self.file))?;
        Ok(())
    }

    /// [`App::reload_config_cache`], reporting a failure in the status line.
    /// Returns whether the config could be read.
    pub(super) fn refresh_config(&mut self) -> bool {
        match self.reload_config_cache() {
            Ok(()) => true,
            Err(e) => {
                self.set_status(format!("cannot read config: {e}"), true);
                false
            }
        }
    }

    /// Changes the registry through [`Store::update`], which re-reads it under
    /// a lock, so changes another rattle made meanwhile are kept, then shows
    /// the result. `None` (with an error in the status line) if saving failed.
    fn update_store<T>(&mut self, change: impl FnOnce(&mut Store) -> T) -> Option<T> {
        match Store::update(&self.file, change) {
            Ok((store, out)) => {
                self.store = store;
                self.refilter();
                Some(out)
            }
            Err(e) => {
                self.set_status(format!("could not save registry: {e}"), true);
                None
            }
        }
    }

    fn on_key(&mut self, key: KeyEvent, term: &mut Term) -> Result<()> {
        let ctrl_c =
            key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c');
        if let Some(job) = &mut self.job {
            // Only cancelling (or quitting) while something runs in the background.
            if !job.can_cancel() {
                if ctrl_c || key.code == KeyCode::Esc {
                    let label = job.label.clone();
                    self.set_status(
                        format!("{label} can't be stopped halfway; it finishes shortly"),
                        false,
                    );
                }
                return Ok(());
            }
            if ctrl_c {
                // Leave nothing running behind.
                job.force();
                self.quit = true;
            } else if key.code == KeyCode::Esc {
                job.cancel();
            }
            return Ok(());
        }
        if ctrl_c {
            self.quit = true;
            return Ok(());
        }
        let selected_name = self.selected().map(|i| self.store.projects[i].name.clone());
        match &mut self.mode {
            // j/k scroll (draw clamps it to what fits); any other key closes.
            Mode::Help { scroll } => match key.code {
                KeyCode::Down | KeyCode::Char('j') => *scroll += 1,
                KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
                _ => self.mode = Mode::Normal,
            },
            Mode::ConfirmDelete => {
                self.mode = Mode::Normal;
                if matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y')) {
                    if let Some(name) = selected_name {
                        if self.update_store(|s| s.remove(&name)).is_some() {
                            self.set_status(format!("removed '{name}' (files kept)"), false);
                        }
                    }
                }
            }
            Mode::ConfirmPrune => {
                self.mode = Mode::Normal;
                if matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y')) {
                    if let Some(gone) = self.update_store(Store::prune_missing) {
                        self.set_status(
                            format!("removed {} missing project(s)", gone.len()),
                            false,
                        );
                    }
                }
            }
            Mode::ConfirmDeleteFiles { path, how, input } => {
                let confirmed = match (*how, key.code) {
                    (DeleteFiles::Trash, KeyCode::Char('y' | 'Y')) => true,
                    (DeleteFiles::Permanent, KeyCode::Char(c)) => {
                        input.push(c);
                        return Ok(());
                    }
                    (DeleteFiles::Permanent, KeyCode::Backspace) => {
                        input.pop();
                        return Ok(());
                    }
                    (DeleteFiles::Permanent, KeyCode::Enter) => {
                        selected_name.as_deref() == Some(input.as_str())
                    }
                    _ => false,
                };
                let (path, how) = (path.clone(), *how);
                self.mode = Mode::Normal;
                if confirmed {
                    self.delete_files(&path, how);
                } else {
                    self.set_status("cancelled; nothing was deleted", false);
                }
            }
            Mode::Settings(form) => match form.on_key(key) {
                FormAction::Nothing => {}
                FormAction::Close => self.mode = Mode::Normal,
                FormAction::Save(changes) => {
                    self.mode = Mode::Normal;
                    self.save_settings(&changes, term)?;
                }
            },
            Mode::Deps(view) => match view.on_key(key) {
                DepsAction::Nothing => {}
                DepsAction::Close => self.mode = Mode::Normal,
                DepsAction::Run(cmd) => self.run_deps(cmd),
            },
            Mode::NewName { input, parent } => match key.code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Char(c) => input.push(c),
                KeyCode::Enter => {
                    let name = input.trim().to_string();
                    if let Err(e) = templates::validate_name(&name) {
                        self.set_status(e, true);
                        return Ok(());
                    }
                    let parent = parent.clone();
                    let templates = templates::all(&self.config);
                    let mut list = ListState::default();
                    list.select(Some(0));
                    self.mode = Mode::NewTemplate {
                        name,
                        parent,
                        templates,
                        list,
                    };
                }
                _ => {}
            },
            Mode::NewTemplate {
                name,
                parent,
                templates,
                list,
            } => match key.code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Down | KeyCode::Char('j') => list.select_next(),
                KeyCode::Up | KeyCode::Char('k') => list.select_previous(),
                KeyCode::Enter => {
                    let Some(template) = list.selected().and_then(|i| templates.get(i)).cloned()
                    else {
                        return Ok(());
                    };
                    let (name, parent) = (name.clone(), parent.clone());
                    self.mode = Mode::Normal;
                    self.create_project(&name, &template, &parent);
                }
                _ => {}
            },
            Mode::Add(input) => match key.code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Char(c) => input.push(c),
                KeyCode::Enter => {
                    let raw = input.trim().to_string();
                    self.mode = Mode::Normal;
                    self.add(&raw);
                }
                _ => {}
            },
            Mode::Filter => match key.code {
                KeyCode::Esc => {
                    self.filter.clear();
                    self.mode = Mode::Normal;
                    self.refilter();
                }
                KeyCode::Enter => self.mode = Mode::Normal,
                KeyCode::Backspace => {
                    self.filter.pop();
                    self.refilter();
                }
                KeyCode::Up | KeyCode::Down => self.on_normal_key(key, term)?,
                KeyCode::Char(c) => {
                    self.filter.push(c);
                    self.refilter();
                }
                _ => {}
            },
            Mode::Normal if self.tab == Tab::Envs => self.on_envs_key(key, term)?,
            Mode::Normal => self.on_normal_key(key, term)?,
        }
        Ok(())
    }

    fn on_normal_key(&mut self, key: KeyEvent, term: &mut Term) -> Result<()> {
        self.status = None;
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Esc if !self.filter.is_empty() => {
                self.filter.clear();
                self.refilter();
            }
            KeyCode::Esc => self.quit = true,
            KeyCode::Down | KeyCode::Char('j') => self.list.select_next(),
            KeyCode::Up | KeyCode::Char('k') => self.list.select_previous(),
            KeyCode::Home | KeyCode::Char('g') => self.list.select_first(),
            KeyCode::End | KeyCode::Char('G') => self.list.select_last(),
            KeyCode::Char('/') => self.mode = Mode::Filter,
            KeyCode::Char('a') => self.mode = Mode::Add(String::new()),
            KeyCode::Char('?') => self.mode = Mode::Help { scroll: 0 },
            KeyCode::Char('s') => self.scan(),
            KeyCode::Char('d') | KeyCode::Delete if self.selected().is_some() => {
                self.mode = Mode::ConfirmDelete
            }
            KeyCode::Char('e') => {
                if let Some(i) = self.selected() {
                    let path = PathBuf::from(&self.store.projects[i].path);
                    self.open_editor(&path, Path::new("."), false, term)?;
                }
            }
            KeyCode::Char('c') => self.edit_config(term)?,
            KeyCode::Char(',') => {
                // The form shows the file as it is now, edits elsewhere included.
                if self.refresh_config() {
                    self.mode = Mode::Settings(Box::new(SettingsForm::new(&self.config)));
                }
            }
            KeyCode::Char('D') => self.start_delete_files(),
            KeyCode::Char('v') | KeyCode::Tab | KeyCode::BackTab => {
                self.refresh_envs();
                self.tab = Tab::Envs;
            }
            KeyCode::Char('o') => self.open_project_in_env(),
            KeyCode::Char('p') => {
                if let Some(i) = self.selected() {
                    let p = &self.store.projects[i];
                    match DepsView::open(&p.name, Path::new(&p.path)) {
                        Ok(view) => self.mode = Mode::Deps(Box::new(view)),
                        Err(e) => self.set_status(e, true),
                    }
                }
            }
            KeyCode::Char('n') => {
                let parent = templates::parent_dir(&self.config, None);
                match parent {
                    Ok(parent) => {
                        self.mode = Mode::NewName {
                            input: String::new(),
                            parent,
                        }
                    }
                    Err(e) => self.set_status(e, true),
                }
            }
            KeyCode::Char('X') => {
                if self.store.missing().is_empty() {
                    self.set_status("no missing projects", false);
                } else {
                    self.mode = Mode::ConfirmPrune;
                }
            }
            KeyCode::Enter => {
                if let Some(i) = self.selected() {
                    self.chosen = Some(self.store.projects[i].path.clone());
                    self.quit = true;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn add(&mut self, raw: &str) {
        let project = match ops::resolve_project(raw) {
            Ok(p) => p,
            Err(e) => return self.set_status(e, true),
        };
        let name = project.name.clone();
        match self.update_store(|s| s.add(project)) {
            Some(Ok(())) => {}
            Some(Err(e)) => return self.set_status(e, true),
            None => return,
        }
        self.filter.clear();
        self.refilter();
        self.refresh_envs();
        self.list.select(
            self.visible
                .iter()
                .position(|&i| self.store.projects[i].name == name),
        );
        self.set_status(format!("added '{name}'"), false);
    }

    /// Runs a package-manager command for the open dependency view, showing
    /// progress first since installs can take a while, then reloads the list.
    /// Starts a package-manager command for the open dependency view in the
    /// background; the view reloads when it finishes.
    fn run_deps(&mut self, cmd: std::process::Command) {
        let label = deps::describe(&cmd);
        let message = match Job::spawn(cmd, label.clone(), JobKind::Deps) {
            Ok(job) => {
                self.job = Some(job);
                (format!("running {label}…"), false)
            }
            Err(e) => (e, true),
        };
        if let Mode::Deps(view) = &mut self.mode {
            view.message = Some(message);
        }
    }

    /// Starts the template in the background; the project is registered when it finishes.
    fn create_project(&mut self, name: &str, template: &Template, parent: &Path) {
        let prepared = match templates::prepare(&self.store, template, name, parent) {
            Ok(p) => p,
            Err(e) => return self.set_status(e, true),
        };
        let label = format!("creating '{name}' with template '{}'", template.name);
        let kind = JobKind::NewProject {
            name: name.to_string(),
            template: prepared.template,
            target: prepared.target,
        };
        match Job::spawn(prepared.command, label, kind) {
            Ok(job) => {
                self.status = None;
                self.job = Some(job);
            }
            Err(e) => self.set_status(e, true),
        }
    }

    /// Checks the background job; handles its result once it has finished.
    fn tick_job(&mut self) {
        let Some(done) = self.job.as_mut().and_then(Job::poll) else {
            return;
        };
        let job = self.job.take().expect("polled job exists");
        let output = match done {
            Done::Output(output) => output,
            Done::Task(result) => return self.finish_task(job, result),
        };
        if matches!(
            job.kind,
            JobKind::Env { .. } | JobKind::EnvLogs { .. } | JobKind::EnvRemove { .. }
        ) {
            return self.finish_env_job(&job, &output);
        }
        match job.kind {
            JobKind::Env { .. }
            | JobKind::EnvLogs { .. }
            | JobKind::EnvRemove { .. }
            | JobKind::DeleteFolder { .. }
            | JobKind::EnvCreate { .. } => {}
            JobKind::NewProject {
                name,
                template,
                target,
            } => {
                if job.cancelled {
                    let leftover = if target.exists() {
                        format!("; {} may be partly created", target.display())
                    } else {
                        String::new()
                    };
                    return self.set_status(format!("cancelled creating '{name}'{leftover}"), true);
                }
                match self.update_store(|s| templates::finish(s, &template, &target, &output)) {
                    None => {}
                    Some(Err(e)) => self.set_status(e, true),
                    Some(Ok(project)) => {
                        self.filter.clear();
                        self.refilter();
                        self.list.select(
                            self.visible
                                .iter()
                                .position(|&i| self.store.projects[i].name == project.name),
                        );
                        let parent = target.parent().unwrap_or(&target);
                        self.set_status(format!("created '{name}' in {}", parent.display()), false);
                        self.refresh_envs();
                    }
                }
            }
            JobKind::Deps => {
                let message = if job.cancelled {
                    (format!("cancelled {}", job.label), true)
                } else {
                    match deps::summarize(&job.program, &output) {
                        Ok(last) => (format!("{}: {last}", job.label), false),
                        Err(e) => (e, true),
                    }
                };
                if let Mode::Deps(view) = &mut self.mode {
                    view.reload();
                    view.message = Some(message);
                }
            }
        }
    }

    /// `D`: checks that the selected folder may be deleted, then asks to confirm.
    fn start_delete_files(&mut self) {
        let Some(i) = self.selected() else { return };
        // Deleting must follow the config as it is now, not as it was.
        if !self.refresh_config() {
            return;
        }
        let config = self.config.clone();
        if config.delete_files == DeleteFiles::Off {
            return self.set_status(
                "deleting folders is off; set delete_files = \"trash\" in config (c)",
                true,
            );
        }
        match ops::deletable_folder(&self.store.projects[i], &self.store, &config, &self.file) {
            Ok(path) => {
                self.mode = Mode::ConfirmDeleteFiles {
                    path,
                    how: config.delete_files,
                    input: String::new(),
                }
            }
            Err(e) => self.set_status(e, true),
        }
    }

    /// Deletes the folder in the background (a big one takes a while), then
    /// the entry (see [`App::finish_task`]).
    fn delete_files(&mut self, path: &Path, how: DeleteFiles) {
        let Some(i) = self.selected() else { return };
        let name = self.store.projects[i].name.clone();
        let verb = if how == DeleteFiles::Trash {
            "moving to the Trash"
        } else {
            "deleting"
        };
        let path = path.to_path_buf();
        self.status = None;
        self.job = Some(Job::spawn_task(
            format!("{verb} '{name}'"),
            JobKind::DeleteFolder { name, how },
            move || ops::delete_folder(&path, how),
        ));
    }

    /// Handles a finished task: a folder deletion or an environment creation.
    fn finish_task(&mut self, job: Job, result: std::result::Result<(), String>) {
        if let Err(e) = result {
            return self.set_status(e, true);
        }
        match job.kind {
            JobKind::DeleteFolder { name, how } => {
                if self.update_store(|s| s.remove(&name)).is_none() {
                    return;
                }
                let what = if how == DeleteFiles::Trash {
                    "moved to the Trash"
                } else {
                    "deleted"
                };
                self.set_status(format!("removed '{name}'; folder {what}"), false);
            }
            JobKind::EnvCreate { env, in_config } => self.finish_create_env(&env, in_config),
            _ => {}
        }
    }

    /// Runs a scan before the first key press when `auto_scan` is on and
    /// scan_dirs are set; stays quiet unless something was found or went wrong.
    fn auto_scan(&mut self, term: &mut Term) -> Result<()> {
        let config = self.config.clone();
        if !config.auto_scan || config.scan_dirs.is_empty() {
            return Ok(());
        }
        self.set_status("scanning…", false);
        term.draw(|f| self.draw(f))?;
        self.status = None;
        self.run_scan(&config, true);
        Ok(())
    }

    fn scan(&mut self) {
        // `s` picks up config changes made elsewhere (e.g. new scan_dirs).
        if !self.refresh_config() {
            return;
        }
        let config = self.config.clone();
        if config.scan_dirs.is_empty() {
            let config_file = config::path_for(&self.file);
            let msg = format!("no scan_dirs set; add them in {}", config_file.display());
            return self.set_status(msg, true);
        }
        self.run_scan(&config, false);
    }

    fn run_scan(&mut self, config: &Config, quiet: bool) {
        let Some(report) = self.update_store(|s| ops::scan_roots(s, &config.scan_dirs, config))
        else {
            return;
        };
        self.refresh_envs();
        let missing = self.store.missing().len();
        if quiet && report.added.is_empty() && report.warnings.is_empty() && missing == 0 {
            return;
        }
        let mut msg = format!(
            "{}: added {}, {} already registered",
            if quiet { "auto-scan" } else { "scan" },
            report.added.len(),
            report.already
        );
        if missing > 0 {
            msg.push_str(&format!(" · {missing} missing, press X to remove"));
        }
        if let Some(first) = report.warnings.first() {
            msg.push_str(&format!(" — {} warning(s): {first}", report.warnings.len()));
        }
        self.set_status(msg, !report.warnings.is_empty());
    }

    /// Leaves the TUI, runs an interactive `cmd` (editor, shell) in the
    /// terminal, then comes back. The inner result is whether it could start.
    fn run_suspended(
        &mut self,
        cmd: &mut std::process::Command,
        term: &mut Term,
    ) -> Result<io::Result<std::process::ExitStatus>> {
        restore()?;
        let status = cmd.status();
        enable_raw_mode()?;
        execute!(
            io::stderr(),
            EnterAlternateScreen,
            ClearScreen(ClearType::All)
        )?;
        // Not `term.clear()`: it asks the terminal for the cursor position over
        // stdout, which never answers when stdout is captured (`cd "$(rattle)"`).
        // A fresh Terminal has empty buffers, so the next draw repaints everything.
        *term = Terminal::new(CrosstermBackend::new(io::stderr()))?;
        Ok(status)
    }

    /// Suspends the TUI, runs the editor on `target` inside `dir`, then restores
    /// the TUI. `wait` makes GUI editors block until the file is closed.
    /// Returns whether the editor could be started.
    fn open_editor(
        &mut self,
        dir: &Path,
        target: &Path,
        wait: bool,
        term: &mut Term,
    ) -> Result<bool> {
        // From the cache, which is reloaded whenever the config is edited here.
        let configured = self.config.editor.clone();
        let (mut cmd, editor) = ops::editor_command(configured.as_deref(), dir, target, wait);
        match self.run_suspended(&mut cmd, term)? {
            Ok(_) => Ok(true),
            Err(e) => {
                self.set_status(format!("could not run '{editor}': {e}"), true);
                Ok(false)
            }
        }
    }

    /// Opens config.toml in the editor, then reloads it and rescans so changes
    /// (e.g. new scan_dirs) show up immediately.
    fn edit_config(&mut self, term: &mut Term) -> Result<()> {
        let config_file = config::path_for(&self.file);
        // Creates the commented template on first use, so there is something to edit.
        if let Err(e) = Config::load(&config_file) {
            self.set_status(format!("cannot create config: {e}"), true);
            return Ok(());
        }
        let dir = config_file.parent().unwrap_or(Path::new(".")).to_path_buf();
        if self.open_editor(&dir, &config_file, true, term)? {
            self.reload_config("config reloaded");
        }
        Ok(())
    }

    /// Writes the settings form's changes into config.toml, then reloads.
    fn save_settings(&mut self, changes: &[(&str, String)], term: &mut Term) -> Result<()> {
        if let Err(e) = config::save_values(&config::path_for(&self.file), changes) {
            self.set_status(format!("could not save settings: {e}"), true);
            return Ok(());
        }
        self.set_status("saved; scanning…", false);
        term.draw(|f| self.draw(f))?;
        self.reload_config(&format!("settings saved ({} changed)", changes.len()));
        Ok(())
    }

    /// Rereads config.toml after it changed and rescans when auto_scan is on,
    /// so e.g. new scan_dirs show up immediately. `done` starts the status line.
    fn reload_config(&mut self, done: &str) {
        if let Err(e) = self.reload_config_cache() {
            return self.set_status(format!("cannot read config: {e}"), true);
        }
        let config = self.config.clone();
        self.refresh_envs();
        if config.scan_dirs.is_empty() {
            self.set_status(
                format!("{done}; set scan_dirs to find projects automatically"),
                false,
            );
        } else if config.auto_scan {
            self.run_scan(&config, false);
            if let Some((msg, _)) = &mut self.status {
                *msg = format!("{done} · {msg}");
            }
        } else {
            self.set_status(format!("{done} (auto_scan is off; press s to scan)"), false);
        }
    }
}
