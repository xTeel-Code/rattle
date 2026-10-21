//! The dependency popup (`p`): list, add and remove a project's dependencies.

use super::draw::popup;
use super::*;

pub(super) struct DepsView {
    project: String,
    dir: PathBuf,
    manager: Manager,
    deps: Vec<Dep>,
    list: ListState,
    /// Typing packages to add; `true` adds them as dev dependencies.
    input: Option<(String, bool)>,
    confirm_remove: bool,
    pub(super) message: Option<(String, bool)>,
}

pub(super) enum DepsAction {
    Nothing,
    Close,
    Run(std::process::Command),
}

impl DepsView {
    pub(super) fn open(project: &str, dir: &Path) -> std::result::Result<Self, String> {
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

    pub(super) fn reload(&mut self) {
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

    pub(super) fn on_key(&mut self, key: KeyEvent) -> DepsAction {
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

pub(super) fn draw_deps(f: &mut Frame, view: &DepsView) {
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
