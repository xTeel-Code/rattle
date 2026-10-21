//! Interactive full-screen project browser.
//!
//! Drawn on stderr so the selected path can be printed to stdout on exit,
//! which makes `cd "$(rattle)"` work.

use std::io::{self, Stderr};
use std::path::{Path, PathBuf};
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
use crate::ops;
use crate::settings::{FormAction, SettingsForm};
use crate::store::Store;
use crate::templates::{self, Template};

type Term = Terminal<CrosstermBackend<Stderr>>;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const KEYS: &[(&str, &str)] = &[
    ("↑/↓ j/k", "move"),
    ("g/G", "first / last"),
    ("Enter", "print path & quit (cd \"$(rattle)\")"),
    ("/", "filter"),
    ("a", "add project folder"),
    ("n", "new project from a template"),
    ("p", "dependencies: list, add, remove"),
    ("d", "delete entry (files are kept)"),
    ("D", "delete entry and folder (delete_files)"),
    ("s", "scan folders from config.toml"),
    ("e", "open in editor (config, $VISUAL, $EDITOR)"),
    ("c", "edit config.toml (reloads & rescans)"),
    (",", "settings form"),
    ("X", "remove entries whose folder is gone"),
    ("?", "toggle this help"),
    ("q / Esc", "quit"),
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
    Help,
}

struct DepsView {
    project: String,
    dir: PathBuf,
    manager: Manager,
    deps: Vec<Dep>,
    list: ListState,
    /// Typing packages to add; `true` adds them as dev dependencies.
    input: Option<(String, bool)>,
    confirm_remove: bool,
    message: Option<(String, bool)>,
}

enum DepsAction {
    Nothing,
    Close,
    Run(std::process::Command),
}

impl DepsView {
    fn open(project: &str, dir: &Path) -> std::result::Result<Self, String> {
        let manager = deps::detect(dir).ok_or_else(|| {
            format!("'{project}' has no supported manifest (Cargo.toml, package.json, pyproject.toml, go.mod)")
        })?;
        let mut view = Self {
            project: project.to_string(),
            dir: dir.to_path_buf(),
            manager,
            deps: Vec::new(),
            list: ListState::default(),
            input: None,
            confirm_remove: false,
            message: None,
        };
        view.reload();
        Ok(view)
    }

    fn reload(&mut self) {
        match deps::list(&self.dir, self.manager) {
            Ok(list) => {
                self.deps = list;
                let sel = self
                    .list
                    .selected()
                    .unwrap_or(0)
                    .min(self.deps.len().saturating_sub(1));
                self.list.select((!self.deps.is_empty()).then_some(sel));
            }
            Err(e) => self.message = Some((e, true)),
        }
    }

    fn on_key(&mut self, key: KeyEvent) -> DepsAction {
        if let Some((text, dev)) = &mut self.input {
            match key.code {
                KeyCode::Esc => self.input = None,
                KeyCode::Backspace => {
                    text.pop();
                }
                KeyCode::Char(c) => text.push(c),
                KeyCode::Enter => {
                    let packages: Vec<String> =
                        text.split_whitespace().map(str::to_string).collect();
                    let dev = *dev;
                    self.input = None;
                    if !packages.is_empty() {
                        return DepsAction::Run(deps::add_command(
                            self.manager,
                            &self.dir,
                            &packages,
                            dev,
                        ));
                    }
                }
                _ => {}
            }
            return DepsAction::Nothing;
        }
        if self.confirm_remove {
            self.confirm_remove = false;
            let dep = self.list.selected().and_then(|i| self.deps.get(i));
            return match (key.code, dep) {
                (KeyCode::Char('y' | 'Y'), Some(dep)) => DepsAction::Run(deps::remove_command(
                    self.manager,
                    &self.dir,
                    &dep.name,
                    &dep.group,
                )),
                _ => {
                    self.message = Some(("cancelled".into(), false));
                    DepsAction::Nothing
                }
            };
        }
        self.message = None;
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return DepsAction::Close,
            KeyCode::Down | KeyCode::Char('j') => self.list.select_next(),
            KeyCode::Up | KeyCode::Char('k') => self.list.select_previous(),
            KeyCode::Char('a') => self.input = Some((String::new(), false)),
            KeyCode::Char('A') => self.input = Some((String::new(), true)),
            KeyCode::Char('x') | KeyCode::Char('d') | KeyCode::Delete
                if self.list.selected().is_some() =>
            {
                self.confirm_remove = true
            }
            KeyCode::Char('r') => {
                self.reload();
                self.message.get_or_insert(("reloaded".into(), false));
            }
            _ => {}
        }
        DepsAction::Nothing
    }
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
    };
    app.refilter();

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore();
        default_hook(info);
    }));

    let mut term = setup()?;
    let result = app
        .auto_scan(&mut term)
        .and_then(|()| app.event_loop(&mut term));
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

    fn save(&mut self) {
        if let Err(e) = self.store.save(&self.file) {
            self.set_status(format!("could not save registry: {e}"), true);
        }
    }

    fn on_key(&mut self, key: KeyEvent, term: &mut Term) -> Result<()> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return Ok(());
        }
        let selected_name = self.selected().map(|i| self.store.projects[i].name.clone());
        match &mut self.mode {
            Mode::Help => self.mode = Mode::Normal,
            Mode::ConfirmDelete => {
                if matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y')) {
                    if let Some(i) = self.selected() {
                        let p = self.store.projects.remove(i);
                        self.set_status(format!("removed '{}' (files kept)", p.name), false);
                        self.save();
                        self.refilter();
                    }
                }
                self.mode = Mode::Normal;
            }
            Mode::ConfirmPrune => {
                if matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y')) {
                    let gone = self.store.prune_missing();
                    self.save();
                    self.refilter();
                    self.set_status(format!("removed {} missing project(s)", gone.len()), false);
                }
                self.mode = Mode::Normal;
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
                DepsAction::Run(cmd) => self.run_deps(cmd, term)?,
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
                    let templates = Config::load(&config::path_for(&self.file))
                        .map(|c| templates::all(&c))
                        .unwrap_or_default();
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
                    self.create_project(&name, &template, &parent, term)?;
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
            KeyCode::Char('?') => self.mode = Mode::Help,
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
            KeyCode::Char(',') => match Config::load(&config::path_for(&self.file)) {
                Ok(config) => self.mode = Mode::Settings(Box::new(SettingsForm::new(&config))),
                Err(e) => self.set_status(format!("cannot read config: {e}"), true),
            },
            KeyCode::Char('D') => self.start_delete_files(),
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
                let parent = Config::load(&config::path_for(&self.file))
                    .map_err(|e| e.to_string())
                    .and_then(|c| templates::parent_dir(&c, None));
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
        if let Err(e) = self.store.add(project) {
            return self.set_status(e, true);
        }
        self.save();
        self.filter.clear();
        self.refilter();
        self.list.select(
            self.visible
                .iter()
                .position(|&i| self.store.projects[i].name == name),
        );
        self.set_status(format!("added '{name}'"), false);
    }

    /// Runs a package-manager command for the open dependency view, showing
    /// progress first since installs can take a while, then reloads the list.
    fn run_deps(&mut self, cmd: std::process::Command, term: &mut Term) -> Result<()> {
        let what = deps::describe(&cmd);
        if let Mode::Deps(view) = &mut self.mode {
            view.message = Some((format!("running {what}…"), false));
        }
        term.draw(|f| self.draw(f))?;
        let result = deps::run_captured(cmd);
        if let Mode::Deps(view) = &mut self.mode {
            view.reload();
            view.message = Some(match result {
                Ok(last) => (format!("{what}: {last}"), false),
                Err(e) => (e, true),
            });
        }
        Ok(())
    }

    /// Runs the template (which may take a while), then registers and selects the project.
    fn create_project(
        &mut self,
        name: &str,
        template: &Template,
        parent: &Path,
        term: &mut Term,
    ) -> Result<()> {
        self.set_status(
            format!("creating '{name}' with template '{}'…", template.name),
            false,
        );
        term.draw(|f| self.draw(f))?;
        match templates::create(&mut self.store, template, name, parent) {
            Ok(project) => {
                self.save();
                self.filter.clear();
                self.refilter();
                self.list.select(
                    self.visible
                        .iter()
                        .position(|&i| self.store.projects[i].name == project.name),
                );
                self.set_status(format!("created '{name}' in {}", parent.display()), false);
            }
            Err(e) => self.set_status(e, true),
        }
        Ok(())
    }

    /// `D`: checks that the selected folder may be deleted, then asks to confirm.
    fn start_delete_files(&mut self) {
        let Some(i) = self.selected() else { return };
        let config_file = config::path_for(&self.file);
        let config = match Config::load(&config_file) {
            Ok(c) => c,
            Err(e) => return self.set_status(format!("cannot read config: {e}"), true),
        };
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

    fn delete_files(&mut self, path: &Path, how: DeleteFiles) {
        let Some(i) = self.selected() else { return };
        if let Err(e) = ops::delete_folder(path, how) {
            return self.set_status(e, true);
        }
        let p = self.store.projects.remove(i);
        self.save();
        self.refilter();
        let what = if how == DeleteFiles::Trash {
            "moved to the Trash"
        } else {
            "deleted"
        };
        self.set_status(format!("removed '{}'; folder {what}", p.name), false);
    }

    /// Runs a scan before the first key press when `auto_scan` is on and
    /// scan_dirs are set; stays quiet unless something was found or went wrong.
    fn auto_scan(&mut self, term: &mut Term) -> Result<()> {
        let config_file = config::path_for(&self.file);
        let Ok(config) = Config::load(&config_file) else {
            return Ok(());
        };
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
        let config_file = config::path_for(&self.file);
        let config = match Config::load(&config_file) {
            Ok(c) => c,
            Err(e) => return self.set_status(format!("cannot read config: {e}"), true),
        };
        if config.scan_dirs.is_empty() {
            let msg = format!("no scan_dirs set; add them in {}", config_file.display());
            return self.set_status(msg, true);
        }
        self.run_scan(&config, false);
    }

    fn run_scan(&mut self, config: &Config, quiet: bool) {
        let report = ops::scan_roots(&mut self.store, &config.scan_dirs, config);
        if !report.added.is_empty() {
            self.save();
        }
        self.refilter();
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
        // Read on every use so a changed `editor` setting applies without a restart.
        let configured = Config::load(&config::path_for(&self.file))
            .ok()
            .and_then(|c| c.editor);
        let (mut cmd, editor) = ops::editor_command(configured.as_deref(), dir, target, wait);
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

        match status {
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
        let config = match Config::load(&config::path_for(&self.file)) {
            Ok(c) => c,
            Err(e) => return self.set_status(format!("cannot read config: {e}"), true),
        };
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

    fn draw(&mut self, f: &mut Frame) {
        let [header, body, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .areas(f.area());

        f.render_widget(
            Line::from(vec![
                " Rattle ".bold().reversed(),
                Span::raw(" "),
                Span::raw(self.file.display().to_string()).dark_gray(),
            ]),
            header,
        );

        self.draw_list(f, body);
        self.draw_footer(f, footer);

        match &self.mode {
            Mode::Add(input) => {
                let area = popup(f.area(), 70, 3);
                f.render_widget(Clear, area);
                f.render_widget(
                    Paragraph::new(input.as_str())
                        .block(Block::bordered().title(" Add project folder (Enter / Esc) ")),
                    area,
                );
                let x = area.x + 1 + input.chars().count() as u16;
                f.set_cursor_position((x.min(area.right() - 2), area.y + 1));
            }
            Mode::ConfirmDelete => {
                let name = self
                    .selected()
                    .map(|i| self.store.projects[i].name.as_str());
                let area = popup(f.area(), 50, 4);
                f.render_widget(Clear, area);
                f.render_widget(
                    Paragraph::new(vec![
                        Line::from(format!("Remove '{}' from Rattle?", name.unwrap_or(""))),
                        Line::from("Files on disk are kept.  [y] yes  [any] no".dark_gray()),
                    ])
                    .block(Block::bordered().title(" Delete ").red()),
                    area,
                );
            }
            Mode::Deps(view) => draw_deps(f, view),
            Mode::Settings(form) => {
                let area = popup(f.area(), 85, form.height());
                f.render_widget(Clear, area);
                form.draw(f, area);
            }
            Mode::NewName { input, parent } => {
                let area = popup(f.area(), 70, 3);
                f.render_widget(Clear, area);
                let title = format!(" New project in {} (Enter / Esc) ", parent.display());
                f.render_widget(
                    Paragraph::new(input.as_str()).block(Block::bordered().title(title)),
                    area,
                );
                let x = area.x + 1 + input.chars().count() as u16;
                f.set_cursor_position((x.min(area.right() - 2), area.y + 1));
            }
            Mode::NewTemplate {
                name,
                templates,
                list,
                ..
            } => {
                let width = templates.iter().map(|t| t.name.len()).max().unwrap_or(0);
                let items: Vec<ListItem> = templates
                    .iter()
                    .map(|t| {
                        ListItem::new(Line::from(vec![
                            Span::raw(format!("{:<width$}  ", t.name)).bold(),
                            Span::raw(t.command.clone()).dark_gray(),
                        ]))
                    })
                    .collect();
                let area = popup(f.area(), 80, templates.len() as u16 + 2);
                f.render_widget(Clear, area);
                let title = format!(" Template for '{name}' (Enter / Esc) ");
                let widget = List::new(items)
                    .block(Block::bordered().title(title))
                    .highlight_style(Style::new().reversed())
                    .highlight_symbol("▶ ");
                f.render_stateful_widget(widget, area, &mut list.clone());
            }
            Mode::ConfirmDeleteFiles { path, how, input } => {
                let name = self
                    .selected()
                    .map(|i| self.store.projects[i].name.as_str())
                    .unwrap_or("");
                let path = path.display().to_string();
                let lines = match how {
                    DeleteFiles::Permanent => vec![
                        Line::from("Permanently delete this folder and everything in it?"),
                        Line::from(path).bold(),
                        Line::from("This cannot be undone.".red().bold()),
                        Line::from(format!(
                            "Type '{name}' and press Enter (anything else cancels):"
                        )),
                        Line::from(format!("> {input}")),
                    ],
                    _ => vec![
                        Line::from("Move this folder to the Trash?"),
                        Line::from(path).bold(),
                        Line::from(format!("'{name}' is also removed from Rattle.")),
                        Line::from("[y] yes  [any] no".dark_gray()),
                    ],
                };
                let width = popup(f.area(), 70, 0).width;
                let area = popup(f.area(), 70, wrapped_height(&lines, width));
                f.render_widget(Clear, area);
                f.render_widget(
                    Paragraph::new(lines)
                        .wrap(Wrap { trim: false })
                        .block(Block::bordered().title(" Delete folder ").red()),
                    area,
                );
            }
            Mode::ConfirmPrune => {
                const SHOWN: usize = 8;
                let missing = self.store.missing();
                let mut lines = vec![Line::from(format!(
                    "Remove {} missing project(s) from Rattle?",
                    missing.len()
                ))];
                lines.extend(
                    missing
                        .iter()
                        .take(SHOWN)
                        .map(|p| Line::from(format!("  {}", p.name)).dark_gray()),
                );
                if missing.len() > SHOWN {
                    lines.push(
                        Line::from(format!("  … and {} more", missing.len() - SHOWN)).dark_gray(),
                    );
                }
                lines.push(Line::from(
                    "Only entries are removed.  [y] yes  [any] no".dark_gray(),
                ));
                let area = popup(f.area(), 50, lines.len() as u16 + 2);
                f.render_widget(Clear, area);
                f.render_widget(
                    Paragraph::new(lines).block(Block::bordered().title(" Remove missing ").red()),
                    area,
                );
            }
            Mode::Help => {
                let area = popup(f.area(), 60, KEYS.len() as u16 + 2);
                f.render_widget(Clear, area);
                let lines: Vec<Line> = KEYS
                    .iter()
                    .map(|(k, d)| Line::from(vec![format!(" {k:<10}").bold(), Span::raw(*d)]))
                    .collect();
                f.render_widget(
                    Paragraph::new(lines).block(Block::bordered().title(" Keys ")),
                    area,
                );
            }
            Mode::Normal | Mode::Filter => {}
        }
    }

    fn draw_list(&mut self, f: &mut Frame, area: Rect) {
        let width = self
            .visible
            .iter()
            .map(|&i| self.store.projects[i].name.chars().count())
            .max()
            .unwrap_or(0);
        let items: Vec<ListItem> = self
            .visible
            .iter()
            .map(|&i| {
                let p = &self.store.projects[i];
                let mut spans = vec![
                    Span::raw(format!("{:<width$}  ", p.name)).bold(),
                    Span::raw(p.path.clone()).dark_gray(),
                ];
                if !Path::new(&p.path).is_dir() {
                    spans.push(Span::raw("  missing").red());
                }
                ListItem::new(Line::from(spans))
            })
            .collect();

        let mut title = format!(" Projects ({}) ", self.visible.len());
        if !self.filter.is_empty() || matches!(self.mode, Mode::Filter) {
            title = format!(
                " Projects ({}/{}) ",
                self.visible.len(),
                self.store.projects.len()
            );
        }

        if items.is_empty() {
            let hint = if self.store.projects.is_empty() {
                "No projects yet. Press 'a' to add a folder or 's' to scan your scan_dirs."
            } else {
                "No project matches the filter."
            };
            f.render_widget(
                Paragraph::new(hint.dark_gray())
                    .wrap(Wrap { trim: true })
                    .block(Block::bordered().title(title)),
                area,
            );
            return;
        }

        let list = List::new(items)
            .block(Block::bordered().title(title))
            .highlight_style(Style::new().reversed())
            .highlight_symbol("▶ ");
        f.render_stateful_widget(list, area, &mut self.list);
    }

    fn draw_footer(&self, f: &mut Frame, area: Rect) {
        let line = if matches!(self.mode, Mode::Filter) {
            Line::from(vec![
                " / ".bold().reversed(),
                Span::raw(format!(" {}", self.filter)),
            ])
        } else if let Some((msg, error)) = &self.status {
            let span = Span::raw(format!(" {msg}"));
            Line::from(if *error { span.red() } else { span.green() })
        } else {
            let mut spans = Vec::new();
            if !self.filter.is_empty() {
                spans.push(format!(" filter: {} ", self.filter).yellow());
            }
            spans.push(
                " Enter cd · / filter · a add · n new · p deps · s scan · e edit · , settings · ? all keys · q quit"
                    .dark_gray(),
            );
            Line::from(spans)
        };
        f.render_widget(line, area);
    }
}

fn draw_deps(f: &mut Frame, view: &DepsView) {
    let area = popup(f.area(), 85, f.area().height * 4 / 5);
    f.render_widget(Clear, area);
    let title = format!(
        " Dependencies · {} · {} ({}) ",
        view.project,
        view.manager.name(),
        view.manager.manifest()
    );
    let block = Block::bordered().title(title);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let [body, footer] = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(inner);

    if view.deps.is_empty() {
        f.render_widget(
            Paragraph::new("No dependencies yet. Press a to add one.".dark_gray()),
            body,
        );
    } else {
        let width = view
            .deps
            .iter()
            .map(|d| d.name.chars().count())
            .max()
            .unwrap_or(0);
        let items: Vec<ListItem> = view
            .deps
            .iter()
            .map(|d| {
                let mut spans = vec![
                    Span::raw(format!("{:<width$}  ", d.name)).bold(),
                    Span::raw(d.spec.clone()).dark_gray(),
                ];
                let group = d.group.label();
                if !group.is_empty() {
                    spans.push(Span::raw(format!("  {group}")).yellow());
                }
                ListItem::new(Line::from(spans))
            })
            .collect();
        let list = List::new(items)
            .highlight_style(Style::new().reversed())
            .highlight_symbol("▶ ");
        f.render_stateful_widget(list, body, &mut view.list.clone());
    }

    let line = if let Some((text, dev)) = &view.input {
        let label = if *dev { " add dev: " } else { " add: " };
        let x = footer.x + (label.chars().count() + text.chars().count()) as u16;
        f.set_cursor_position((x.min(footer.right().saturating_sub(1)), footer.y));
        Line::from(vec![label.bold(), Span::raw(text.clone())])
    } else if view.confirm_remove {
        let name = view
            .list
            .selected()
            .and_then(|i| view.deps.get(i))
            .map(|d| d.name.as_str())
            .unwrap_or("");
        Line::from(format!(" Remove {name}?  [y] yes  [any] no").red())
    } else if let Some((msg, error)) = &view.message {
        let span = Span::raw(format!(" {msg}"));
        Line::from(if *error { span.red() } else { span.green() })
    } else {
        Line::from(" a add · A add dev · x remove · r reload · Esc close".dark_gray())
    };
    f.render_widget(line, footer);
}

/// Rows a bordered paragraph of `lines` needs at `width` columns, wrapping included.
fn wrapped_height(lines: &[Line], width: u16) -> u16 {
    let inner = usize::from(width.saturating_sub(2)).max(1);
    let rows: usize = lines.iter().map(|l| l.width().div_ceil(inner).max(1)).sum();
    u16::try_from(rows + 2).unwrap_or(u16::MAX)
}

/// A rectangle centred in `area`, `percent_x` wide and `height` rows tall.
fn popup(area: Rect, percent_x: u16, height: u16) -> Rect {
    let width = (area.width * percent_x / 100).max(20).min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}
