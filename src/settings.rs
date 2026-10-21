//! The TUI settings form (`,`): edits config.toml in place, keeping comments.

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::config::{toml_list, toml_string, Config};

#[derive(Clone, Copy)]
enum Kind {
    /// Free text; empty means "use the default", shown as the placeholder.
    Text(&'static str),
    /// Comma-separated list of strings.
    List,
    /// A whole number of at least 1.
    Number,
    Bool,
    Choice(&'static [&'static str]),
}

struct Field {
    key: &'static str,
    label: &'static str,
    help: &'static str,
    kind: Kind,
    value: String,
    original: String,
}

pub struct SettingsForm {
    fields: Vec<Field>,
    list: ListState,
    /// The text being typed into the selected field.
    editing: Option<String>,
    message: Option<(String, bool)>,
    confirm_discard: bool,
}

pub enum FormAction {
    Nothing,
    Close,
    /// Save these (key, TOML value) pairs, then close.
    Save(Vec<(&'static str, String)>),
}

const DELETE_CHOICES: &[&str] = &["off", "trash", "permanent"];

impl SettingsForm {
    pub fn new(config: &Config) -> Self {
        let field = |key, label, help, kind, value: String| Field {
            key,
            label,
            help,
            kind,
            original: value.clone(),
            value,
        };
        let fields = vec![
            field(
                "scan_dirs",
                "Scan folders",
                "Folders searched for projects, comma-separated. Relative paths are under your home folder.",
                Kind::List,
                config.scan_dirs.join(", "),
            ),
            field(
                "new_project_dir",
                "New projects in",
                "Where n / rattle new creates projects. Empty uses the first scan folder.",
                Kind::Text("first scan folder"),
                config.new_project_dir.clone().unwrap_or_default(),
            ),
            field(
                "max_depth",
                "Search depth",
                "How many folder levels below each scan folder are searched.",
                Kind::Number,
                config.max_depth.to_string(),
            ),
            field(
                "auto_scan",
                "Auto-scan",
                "Rescan the scan folders every time the TUI opens.",
                Kind::Bool,
                config.auto_scan.to_string(),
            ),
            field(
                "editor",
                "Editor",
                "Used by e and c. Empty uses $VISUAL / $EDITOR. GUI editors (zed, code, …) get --wait for c.",
                Kind::Text("$VISUAL / $EDITOR"),
                config.editor.clone().unwrap_or_default(),
            ),
            field(
                "delete_files",
                "Delete folders",
                "What D does: off (never), trash (move to Trash), permanent (after typing the name).",
                Kind::Choice(DELETE_CHOICES),
                config.delete_files.as_str().to_string(),
            ),
            field(
                "markers",
                "Project markers",
                "A folder is a project if it contains any of these, comma-separated.",
                Kind::List,
                config.markers.join(", "),
            ),
            field(
                "ignore",
                "Ignored folders",
                "Folder names that are never searched, comma-separated.",
                Kind::List,
                config.ignore.join(", "),
            ),
        ];
        let mut list = ListState::default();
        list.select(Some(0));
        Self {
            fields,
            list,
            editing: None,
            message: None,
            confirm_discard: false,
        }
    }

    /// Changed fields as (key, TOML value).
    fn changes(&self) -> Vec<(&'static str, String)> {
        self.fields
            .iter()
            .filter(|f| f.value != f.original)
            .map(|f| {
                let value = match f.kind {
                    Kind::List => toml_list(
                        &f.value
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect::<Vec<_>>(),
                    ),
                    Kind::Text(_) | Kind::Choice(_) => toml_string(f.value.trim()),
                    Kind::Number | Kind::Bool => f.value.clone(),
                };
                (f.key, value)
            })
            .collect()
    }

    fn selected(&mut self) -> &mut Field {
        let i = self.list.selected().unwrap_or(0).min(self.fields.len() - 1);
        &mut self.fields[i]
    }

    pub fn on_key(&mut self, key: KeyEvent) -> FormAction {
        if let Some(buf) = &mut self.editing {
            match key.code {
                KeyCode::Esc => self.editing = None,
                KeyCode::Backspace => {
                    buf.pop();
                }
                KeyCode::Char(c) => buf.push(c),
                KeyCode::Enter => {
                    let buf = buf.trim().to_string();
                    let field = self.selected();
                    if matches!(field.kind, Kind::Number)
                        && !buf.parse::<usize>().is_ok_and(|n| n >= 1)
                    {
                        self.message = Some(("enter a whole number of at least 1".into(), true));
                        return FormAction::Nothing;
                    }
                    field.value = buf;
                    self.editing = None;
                    self.message = None;
                }
                _ => {}
            }
            return FormAction::Nothing;
        }

        let discard_pending = std::mem::take(&mut self.confirm_discard);
        // `s` or Ctrl-s.
        if key.code == KeyCode::Char('s') {
            let changes = self.changes();
            if changes.is_empty() {
                self.message = Some(("nothing changed".into(), false));
                return FormAction::Nothing;
            }
            return FormAction::Save(changes);
        }
        self.message = None;
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                if self.changes().is_empty() || discard_pending {
                    return FormAction::Close;
                }
                self.confirm_discard = true;
                self.message = Some((
                    "unsaved changes: s saves, Esc again discards them".into(),
                    true,
                ));
            }
            KeyCode::Down | KeyCode::Char('j') => self.list.select_next(),
            KeyCode::Up | KeyCode::Char('k') => self.list.select_previous(),
            KeyCode::Enter | KeyCode::Char(' ') => {
                let field = self.selected();
                match field.kind {
                    Kind::Bool => field.value = (field.value != "true").to_string(),
                    Kind::Choice(options) => {
                        let at = options.iter().position(|o| *o == field.value).unwrap_or(0);
                        field.value = options[(at + 1) % options.len()].to_string();
                    }
                    _ => self.editing = Some(field.value.clone()),
                }
            }
            _ => {}
        }
        FormAction::Nothing
    }

    pub fn draw(&self, f: &mut Frame, area: Rect) {
        let block = Block::bordered().title(" Settings · config.toml ");
        let inner = block.inner(area);
        f.render_widget(block, area);
        let [body, help, footer] = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(2),
            Constraint::Length(1),
        ])
        .areas(inner);

        let label_width = self.fields.iter().map(|f| f.label.len()).max().unwrap_or(0);
        let selected = self.list.selected().unwrap_or(0);
        let items: Vec<ListItem> = self
            .fields
            .iter()
            .enumerate()
            .map(|(i, field)| {
                let changed = if field.value != field.original {
                    "*"
                } else {
                    " "
                };
                let mut spans = vec![
                    Span::raw(changed).yellow(),
                    Span::raw(format!("{:<label_width$}  ", field.label)).bold(),
                ];
                match (&self.editing, i == selected) {
                    (Some(buf), true) => spans.push(Span::raw(format!("{buf}█"))),
                    _ if field.value.is_empty() => {
                        let placeholder = match field.kind {
                            Kind::Text(p) => p,
                            _ => "(none)",
                        };
                        spans.push(Span::raw(placeholder).dark_gray().italic());
                    }
                    _ => spans.push(Span::raw(field.value.clone())),
                }
                ListItem::new(Line::from(spans))
            })
            .collect();
        let list = List::new(items)
            .highlight_style(Style::new().reversed())
            .highlight_symbol("▶ ");
        f.render_stateful_widget(list, body, &mut self.list.clone());

        let help_text = self.fields.get(selected).map(|f| f.help).unwrap_or("");
        f.render_widget(
            Paragraph::new(help_text.dark_gray()).wrap(Wrap { trim: true }),
            help,
        );

        let line = match (&self.message, &self.editing) {
            (Some((msg, error)), _) => {
                let span = Span::raw(format!(" {msg}"));
                Line::from(if *error { span.red() } else { span.green() })
            }
            (None, Some(_)) => Line::from(" Enter keep · Esc cancel edit".dark_gray()),
            (None, None) => Line::from(
                " Enter edit / toggle · s save · Esc close · templates: edit with c".dark_gray(),
            ),
        };
        f.render_widget(line, footer);
    }

    /// Rows the form wants: fields + help + footer + borders.
    pub fn height(&self) -> u16 {
        self.fields.len() as u16 + 5
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyEvent;

    fn press(form: &mut SettingsForm, code: KeyCode) -> FormAction {
        form.on_key(KeyEvent::from(code))
    }

    fn type_text(form: &mut SettingsForm, text: &str) {
        for c in text.chars() {
            press(form, KeyCode::Char(c));
        }
    }

    #[test]
    fn edits_toggles_and_saves_only_changes() {
        let mut form = SettingsForm::new(&Config::default());
        // scan_dirs: type a list.
        press(&mut form, KeyCode::Enter);
        type_text(&mut form, "~/code, Documents ,");
        press(&mut form, KeyCode::Enter);
        // max_depth: invalid, then valid.
        press(&mut form, KeyCode::Down);
        press(&mut form, KeyCode::Down);
        press(&mut form, KeyCode::Enter);
        press(&mut form, KeyCode::Backspace);
        type_text(&mut form, "0");
        press(&mut form, KeyCode::Enter);
        assert!(
            form.editing.is_some(),
            "0 is rejected and editing continues"
        );
        press(&mut form, KeyCode::Backspace);
        type_text(&mut form, "4");
        press(&mut form, KeyCode::Enter);
        // auto_scan toggles; delete_files cycles off -> trash.
        press(&mut form, KeyCode::Down);
        press(&mut form, KeyCode::Char(' '));
        press(&mut form, KeyCode::Down);
        press(&mut form, KeyCode::Down);
        press(&mut form, KeyCode::Enter);

        let FormAction::Save(changes) = press(&mut form, KeyCode::Char('s')) else {
            panic!("expected save")
        };
        assert_eq!(
            changes,
            vec![
                ("scan_dirs", "[\"~/code\", \"Documents\"]".to_string()),
                ("max_depth", "4".to_string()),
                ("auto_scan", "false".to_string()),
                ("delete_files", "\"trash\"".to_string()),
            ]
        );
    }

    #[test]
    fn escape_with_changes_asks_before_discarding() {
        let mut form = SettingsForm::new(&Config::default());
        assert!(
            matches!(press(&mut form, KeyCode::Char('s')), FormAction::Nothing),
            "nothing to save"
        );
        for _ in 0..3 {
            press(&mut form, KeyCode::Down);
        }
        press(&mut form, KeyCode::Enter); // toggle auto_scan
        assert!(matches!(
            press(&mut form, KeyCode::Esc),
            FormAction::Nothing
        ));
        assert!(form.confirm_discard);
        assert!(matches!(press(&mut form, KeyCode::Esc), FormAction::Close));
    }
}
