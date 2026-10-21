//! Interactive full-screen project browser.
//!
//! Drawn on stderr so the selected path can be printed to stdout on exit,
//! which makes `cd "$(rattle)"` work.

use std::io::{self, Stderr};
use std::path::{Path, PathBuf};

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::config::{self, Config};
use crate::ops;
use crate::store::Store;

type Term = Terminal<CrosstermBackend<Stderr>>;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const KEYS: &[(&str, &str)] = &[
    ("↑/↓ j/k", "move"),
    ("g/G", "first / last"),
    ("Enter", "print path & quit (cd \"$(rattle)\")"),
    ("/", "filter"),
    ("a", "add project folder"),
    ("d", "delete entry (files are kept)"),
    ("s", "scan folders from config.toml"),
    ("e", "open in $EDITOR"),
    ("c", "edit config.toml (reloads & rescans)"),
    ("?", "toggle this help"),
    ("q / Esc", "quit"),
];

enum Mode {
    Normal,
    Filter,
    Add(String),
    ConfirmDelete,
    Help,
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
    let result = app.auto_scan(&mut term).and_then(|()| app.event_loop(&mut term));
    restore()?;
    result?;

    if let Some(path) = app.chosen {
        println!("{path}");
    }
    Ok(())
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
        while !self.quit {
            term.draw(|f| self.draw(f))?;
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    self.on_key(key, term)?;
                }
            }
        }
        Ok(())
    }

    fn selected(&self) -> Option<usize> {
        self.list.selected().and_then(|i| self.visible.get(i).copied())
    }

    fn refilter(&mut self) {
        let needle = self.filter.to_lowercase();
        self.visible = (0..self.store.projects.len())
            .filter(|&i| {
                let p = &self.store.projects[i];
                p.name.to_lowercase().contains(&needle) || p.path.to_lowercase().contains(&needle)
            })
            .collect();
        let sel = self.list.selected().unwrap_or(0).min(self.visible.len().saturating_sub(1));
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
                    self.open_editor(&path, Path::new("."), term)?;
                }
            }
            KeyCode::Char('c') => self.edit_config(term)?,
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
        self.list.select(self.visible.iter().position(|&i| self.store.projects[i].name == name));
        self.set_status(format!("added '{name}'"), false);
    }

    /// Runs a scan before the first key press when `auto_scan` is on and
    /// scan_dirs are set; stays quiet unless something was found or went wrong.
    fn auto_scan(&mut self, term: &mut Term) -> Result<()> {
        let config_file = config::path_for(&self.file);
        let Ok(config) = Config::load(&config_file) else { return Ok(()) };
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
        if quiet && report.added.is_empty() && report.warnings.is_empty() {
            return;
        }
        let mut msg = format!(
            "{}: added {}, {} already registered",
            if quiet { "auto-scan" } else { "scan" },
            report.added.len(),
            report.already
        );
        if let Some(first) = report.warnings.first() {
            msg.push_str(&format!(" — {} warning(s): {first}", report.warnings.len()));
        }
        self.set_status(msg, !report.warnings.is_empty());
    }

    /// Suspends the TUI, runs the editor on `target` inside `dir`, then restores
    /// the TUI. Returns whether the editor could be started.
    fn open_editor(&mut self, dir: &Path, target: &Path, term: &mut Term) -> Result<bool> {
        let (mut cmd, editor) = ops::editor_command(dir, target);
        restore()?;
        let status = cmd.status();
        enable_raw_mode()?;
        execute!(io::stderr(), EnterAlternateScreen)?;
        term.clear()?;

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
        if !self.open_editor(&dir, &config_file, term)? {
            return Ok(());
        }

        let config = match Config::load(&config_file) {
            Ok(c) => c,
            Err(e) => {
                self.set_status(format!("cannot read config: {e}"), true);
                return Ok(());
            }
        };
        if config.scan_dirs.is_empty() {
            self.set_status("config reloaded; set scan_dirs to find projects automatically", false);
        } else if config.auto_scan {
            self.run_scan(&config, false);
            if let Some((msg, _)) = &mut self.status {
                *msg = format!("config reloaded · {msg}");
            }
        } else {
            self.set_status("config reloaded (auto_scan is off; press s to scan)", false);
        }
        Ok(())
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
                let name = self.selected().map(|i| self.store.projects[i].name.as_str());
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
            title = format!(" Projects ({}/{}) ", self.visible.len(), self.store.projects.len());
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
            Line::from(vec![" / ".bold().reversed(), Span::raw(format!(" {}", self.filter))])
        } else if let Some((msg, error)) = &self.status {
            let span = Span::raw(format!(" {msg}"));
            Line::from(if *error { span.red() } else { span.green() })
        } else {
            let mut spans = Vec::new();
            if !self.filter.is_empty() {
                spans.push(format!(" filter: {} ", self.filter).yellow());
            }
            spans.push(
                " Enter cd · / filter · a add · d delete · s scan · e edit · c config · ? help · q quit"
                    .dark_gray(),
            );
            Line::from(spans)
        };
        f.render_widget(line, area);
    }
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
