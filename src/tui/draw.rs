//! Rendering: the project list, footer and every popup.

use super::deps_view::draw_deps;
use super::envs_view::status_span;
use super::*;

impl App {
    pub(super) fn draw(&mut self, f: &mut Frame) {
        let [header, body, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .areas(f.area());

        // " Rattle  Projects  Environments  ~/.rattle/projects.toml", the tab shown highlighted.
        let tab = |label: &'static str, active: bool| {
            if active {
                Span::raw(label).bold().reversed()
            } else {
                Span::raw(label).dark_gray()
            }
        };
        f.render_widget(
            Line::from(vec![
                " Rattle ".bold().reversed(),
                Span::raw(" "),
                tab(" Projects ", self.tab == Tab::Projects),
                Span::raw(" "),
                tab(" Environments ", self.tab == Tab::Envs),
                Span::raw("  "),
                Span::raw(self.file.display().to_string()).dark_gray(),
            ]),
            header,
        );

        match self.tab {
            Tab::Projects => {
                self.draw_list(f, body);
                self.draw_footer(f, footer);
            }
            Tab::Envs => {
                self.draw_envs_tab(f, body);
                self.draw_envs_footer(f, footer);
            }
        }

        // Keep the help scroll within what fits, so `k` responds right after
        // scrolling past the end.
        if let Mode::Help { scroll } = &mut self.mode {
            let rows = help_rows(f.area());
            *scroll = (*scroll).min(KEYS.len().saturating_sub(rows));
        }

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
            Mode::Deps(view) => draw_deps(f, view, self.job.as_ref().map(|j| j.status_line())),
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
            Mode::Help { scroll } => {
                // As tall as the terminal allows; scroll with j/k when it doesn't fit.
                let area = popup(f.area(), 60, KEYS.len() as u16 + 2);
                let rows = help_rows(f.area());
                let max_scroll = KEYS.len().saturating_sub(rows);
                let mut title = " Keys ".to_string();
                if max_scroll > 0 {
                    let up = if *scroll > 0 { "↑" } else { " " };
                    let down = if *scroll < max_scroll { "↓" } else { " " };
                    title.push_str(&format!("{up}{down} j/k "));
                }
                let lines: Vec<Line> = KEYS
                    .iter()
                    .skip(*scroll)
                    .take(rows)
                    .map(|(k, d)| match *k {
                        "" => Line::from(format!(" {d}").cyan().bold()),
                        _ => Line::from(vec![format!("   {k:<10}").bold(), Span::raw(*d)]),
                    })
                    .collect();
                f.render_widget(Clear, area);
                f.render_widget(
                    Paragraph::new(lines).block(Block::bordered().title(title)),
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
        let env_width = self
            .visible
            .iter()
            .filter_map(|&i| self.project_envs.get(&self.store.projects[i].name))
            .map(|e| e.chars().count())
            .max()
            .unwrap_or(0);
        let items: Vec<ListItem> = self
            .visible
            .iter()
            .map(|&i| {
                let p = &self.store.projects[i];
                let mut spans = vec![Span::raw(format!("{:<width$}  ", p.name)).bold()];
                // [work ●]: the project's environment and whether it runs.
                if env_width > 0 {
                    match self.project_envs.get(&p.name) {
                        Some(env) => {
                            let pad = env_width - env.chars().count();
                            spans.push(Span::raw(format!("[{env} ")).cyan());
                            spans.push(status_span(self.env_status.get(env), false));
                            spans.push(Span::raw(format!("]{:pad$}  ", "")).cyan());
                        }
                        None => spans.push(Span::raw(format!("{:w$}  ", "", w = env_width + 4))),
                    }
                }
                spans.push(Span::raw(p.path.clone()).dark_gray());
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
        let line = if let Some(job) = &self.job {
            Line::from(format!(" {}", job.status_line()).yellow())
        } else if matches!(self.mode, Mode::Filter) {
            Line::from(vec![
                " / ".bold().reversed(),
                Span::raw(format!(" {}", self.filter)),
            ])
        } else if let Some((msg, error)) = &self.status {
            let span = Span::raw(format!(" {msg}"));
            Line::from(if *error { span.red() } else { span.green() })
        } else {
            let mut spans = Vec::new();
            let mut width = usize::from(area.width);
            if !self.filter.is_empty() {
                let filter = format!(" filter: {} ", self.filter);
                width = width.saturating_sub(filter.chars().count());
                spans.push(filter.yellow());
            }
            spans.push(fit_hints(width, FOOTER_HINTS).dark_gray());
            Line::from(spans)
        };
        f.render_widget(line, area);
    }
}

/// How many key rows the help popup can show in `screen`.
fn help_rows(screen: Rect) -> usize {
    let area = popup(screen, 60, KEYS.len() as u16 + 2);
    usize::from(area.height.saturating_sub(2)).max(1)
}

/// Footer hints with a priority (0 = always shown), in display order.
const FOOTER_HINTS: &[(&str, u8)] = &[
    ("Enter cd", 0),
    ("/ filter", 1),
    ("a add", 3),
    ("n new", 3),
    ("p deps", 3),
    ("Tab envs", 1),
    ("s scan", 2),
    ("e edit", 2),
    (", settings", 3),
    ("? keys", 0),
    ("q quit", 0),
];

/// The most hints that fit in `width` columns: lower-priority ones go first,
/// the rest keep their order.
pub(super) fn fit_hints(width: usize, hints: &[(&str, u8)]) -> String {
    let max = hints.iter().map(|(_, p)| *p).max().unwrap_or(0);
    let line = |limit: u8| {
        let shown: Vec<&str> = hints
            .iter()
            .filter(|(_, p)| *p <= limit)
            .map(|(h, _)| *h)
            .collect();
        format!(" {}", shown.join(" · "))
    };
    (0..=max)
        .rev()
        .map(line)
        .find(|l| l.chars().count() <= width)
        .unwrap_or_else(|| line(0))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footer_drops_low_priority_hints_first() {
        let full = fit_hints(200, FOOTER_HINTS);
        assert!(
            full.contains(", settings") && full.contains("q quit"),
            "{full}"
        );
        assert_eq!(
            fit_hints(50, FOOTER_HINTS),
            " Enter cd · / filter · Tab envs · ? keys · q quit"
        );
        assert_eq!(fit_hints(40, FOOTER_HINTS), " Enter cd · ? keys · q quit");
        let essentials = fit_hints(0, FOOTER_HINTS);
        assert_eq!(essentials, " Enter cd · ? keys · q quit", "always shown");
        for width in 0..120 {
            let line = fit_hints(width, FOOTER_HINTS);
            let limit = width.max(essentials.chars().count());
            assert!(line.chars().count() <= limit, "{width}: {line}");
        }
    }
}
