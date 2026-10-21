//! Rendering: the project list, footer and every popup.

use super::deps_view::draw_deps;
use super::*;

impl App {
    pub(super) fn draw(&mut self, f: &mut Frame) {
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

/// Rows a bordered paragraph of `lines` needs at `width` columns, wrapping included.
fn wrapped_height(lines: &[Line], width: u16) -> u16 {
    let inner = usize::from(width.saturating_sub(2)).max(1);
    let rows: usize = lines.iter().map(|l| l.width().div_ceil(inner).max(1)).sum();
    u16::try_from(rows + 2).unwrap_or(u16::MAX)
}

/// A rectangle centred in `area`, `percent_x` wide and `height` rows tall.
pub(super) fn popup(area: Rect, percent_x: u16, height: u16) -> Rect {
    let width = (area.width * percent_x / 100).max(20).min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}
