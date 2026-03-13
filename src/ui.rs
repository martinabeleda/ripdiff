use crate::app::{App, Panel};
use crate::git::{FileStat, FileStatus};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation,
        ScrollbarState,
    },
    Frame,
};
use unicode_width::UnicodeWidthStr;

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let root_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    render_title(frame, app, root_chunks[0]);
    render_body(frame, app, root_chunks[1]);
}

fn render_title(frame: &mut Frame, app: &App, area: Rect) {
    let repo_name = app
        .repo_root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("?");

    let changed = app.files().len();
    let mode_label = app.ui.diff_mode.label();

    let title_text = if let Some(error) = &app.error_message {
        format!("  ripdiff  [{repo_name}]  ERROR: {error}  ")
    } else {
        let panel_label = match app.ui.focus {
            Panel::Files => "files",
            Panel::Diff => "diff",
        };
        format!(
            "  ripdiff  [repo: {repo_name}]  {changed} file{} changed  mode: {mode_label}  panel: {panel_label}  │  Tab/h/l:panel  j/k:nav  gg/G:top/bottom  s/S:stage-toggle  []:hunk  <Space>e:sidebar  t:mode  r:refresh  q:quit",
            if changed == 1 { "" } else { "s" },
        )
    };

    let style = Style::default()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD);

    frame.render_widget(Paragraph::new(title_text).style(style), area);
}

fn render_body(frame: &mut Frame, app: &App, area: Rect) {
    if !app.ui.show_sidebar {
        render_diff_panel(frame, app, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(25), Constraint::Percentage(75)])
        .split(area);

    render_file_list(frame, app, chunks[0]);
    render_diff_panel(frame, app, chunks[1]);
}

fn render_file_list(frame: &mut Frame, app: &App, area: Rect) {
    let content_width = area.width.saturating_sub(1) as usize;
    let items: Vec<ListItem> = app
        .files()
        .iter()
        .enumerate()
        .map(|(index, file)| {
            let hidden = app.ui.hidden_files.contains(&file.path);
            let status_color = match file.status {
                FileStatus::Added => Color::Green,
                FileStatus::Deleted => Color::Red,
                FileStatus::Modified => Color::Yellow,
                FileStatus::Renamed => Color::Cyan,
                FileStatus::Untracked => Color::Magenta,
                FileStatus::Unknown => Color::White,
            };

            let icon = file_stage_icon(file);
            let icon_text = icon.map(|(symbol, _)| symbol).unwrap_or(" ");
            let stats_text_len =
                plain_text_width(&format_stat_spans(file.additions, file.deletions));
            let reserved_width = 6usize
                .saturating_add(stats_text_len)
                .saturating_add(display_width(icon_text));
            let display_path =
                shorten_path(&file.path, content_width.saturating_sub(reserved_width));
            let visibility = if hidden { "⊘" } else { " " };
            let stat_spans = format_stat_spans(file.additions, file.deletions);
            let stage_padding = file_stage_padding(content_width, visibility, &display_path, file);

            let mut spans = vec![
                Span::raw(format!("{visibility} ")),
                Span::styled(file.status.symbol(), Style::default().fg(status_color)),
                Span::raw(" "),
            ];

            if index == app.ui.selected {
                spans.push(Span::styled(
                    display_path,
                    Style::default().add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::raw(display_path));
            }

            spans.push(Span::raw("  "));
            spans.extend(stat_spans);
            spans.push(Span::raw(" ".repeat(stage_padding)));

            if let Some((symbol, color)) = icon {
                spans.push(Span::styled(symbol, Style::default().fg(color)));
            }

            ListItem::new(Line::from(spans))
        })
        .collect();

    let border_color = if app.ui.focus == Panel::Files {
        Color::Cyan
    } else {
        Color::DarkGray
    };
    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(border_color));

    if items.is_empty() {
        let message = Paragraph::new(Text::from(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  no changes",
                Style::default().fg(Color::DarkGray),
            )),
        ]))
        .block(block);
        frame.render_widget(message, area);
        return;
    }

    let list = List::new(items).block(block).highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    );

    let mut state = ListState::default();
    state.select(Some(app.ui.selected));
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_diff_panel(frame: &mut Frame, app: &App, area: Rect) {
    let file_path = app
        .selected_file()
        .map(|file| file.path.clone())
        .unwrap_or_default();
    let is_hidden = app.ui.hidden_files.contains(&file_path);

    let header_color = if app.ui.focus == Panel::Diff {
        Color::Cyan
    } else {
        Color::DarkGray
    };
    let header = if file_path.is_empty() {
        "diff".to_string()
    } else {
        file_path.clone()
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" {header}"),
            Style::default()
                .fg(header_color)
                .add_modifier(Modifier::BOLD),
        )),
        chunks[0],
    );

    let content_area = chunks[1];
    let block = Block::default();

    if app.files().is_empty() {
        let message = Paragraph::new(Text::from(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No changes detected. Working tree is clean.",
                Style::default().fg(Color::DarkGray),
            )),
        ]))
        .block(block);
        frame.render_widget(message, content_area);
        return;
    }

    if is_hidden {
        let message = Paragraph::new(Text::from(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  [hidden - press Space to show]",
                Style::default().fg(Color::DarkGray),
            )),
        ]))
        .block(block);
        frame.render_widget(message, content_area);
        return;
    }

    let loading_lines = [Line::from(Span::styled(
        "  loading...",
        Style::default().fg(Color::DarkGray),
    ))];

    let total_lines = app
        .selected_diff()
        .map(|diff| diff.lines.len())
        .unwrap_or(loading_lines.len());
    let inner_height = content_area.height as usize;
    let scroll = app
        .ui
        .scroll_offset
        .min(total_lines.saturating_sub(inner_height));
    let end = scroll.saturating_add(inner_height).min(total_lines);
    let is_diff_focused = app.ui.focus == Panel::Diff;

    let visible_lines = if let Some(diff) = app.selected_diff() {
        let cursor = app.ui.diff_cursor.saturating_sub(scroll);
        style_visible_diff_lines(&diff.lines[scroll..end], is_diff_focused, Some(cursor))
    } else {
        style_visible_diff_lines(&loading_lines, is_diff_focused, Some(0))
    };

    frame.render_widget(
        Paragraph::new(Text::from(visible_lines)).block(block),
        content_area,
    );

    if total_lines > inner_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
        let mut scrollbar_state = ScrollbarState::new(total_lines).position(scroll);
        let scrollbar_area = Rect {
            x: content_area.right() - 1,
            y: content_area.y,
            width: 1,
            height: content_area.height,
        };
        frame.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
    }
}

fn style_visible_diff_lines(
    lines: &[Line<'_>],
    is_diff_focused: bool,
    selected_line: Option<usize>,
) -> Vec<Line<'static>> {
    lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let background =
                resolve_diff_line_background(line, is_diff_focused && selected_line == Some(index));
            let spans = line
                .spans
                .iter()
                .map(|span| Span::styled(span.content.to_string(), span.style.bg(background)))
                .collect::<Vec<_>>();
            Line::from(spans)
        })
        .collect()
}

fn resolve_diff_line_background(line: &Line<'_>, is_cursor_line: bool) -> Color {
    match (detect_diff_line_kind(line), is_cursor_line) {
        (Some(DiffLineKind::Addition), true) => Color::Rgb(22, 48, 30),
        (Some(DiffLineKind::Deletion), true) => Color::Rgb(52, 24, 24),
        (Some(DiffLineKind::Addition), false) => Color::Rgb(12, 26, 18),
        (Some(DiffLineKind::Deletion), false) => Color::Rgb(30, 14, 14),
        (None, true) => Color::Rgb(34, 39, 49),
        (None, false) => Color::Reset,
    }
}

fn detect_diff_line_kind(line: &Line<'_>) -> Option<DiffLineKind> {
    let has_green = line
        .spans
        .iter()
        .any(|span| is_addition_color(span.style.fg));
    let has_red = line
        .spans
        .iter()
        .any(|span| is_deletion_color(span.style.fg));

    match (has_green, has_red) {
        (true, false) => Some(DiffLineKind::Addition),
        (false, true) => Some(DiffLineKind::Deletion),
        _ => None,
    }
}

fn is_addition_color(color: Option<Color>) -> bool {
    matches!(
        color,
        Some(Color::Green)
            | Some(Color::LightGreen)
            | Some(Color::Indexed(2))
            | Some(Color::Indexed(10))
    )
}

fn is_deletion_color(color: Option<Color>) -> bool {
    matches!(
        color,
        Some(Color::Red)
            | Some(Color::LightRed)
            | Some(Color::Indexed(1))
            | Some(Color::Indexed(9))
    )
}

#[derive(Clone, Copy)]
enum DiffLineKind {
    Addition,
    Deletion,
}

fn shorten_path(path: &str, max_len: usize) -> String {
    if path.len() <= max_len || max_len < 4 {
        return path.to_string();
    }
    let keep = max_len.saturating_sub(3);
    let start = path.len() - keep;
    format!("...{}", &path[start..])
}

fn format_stat_spans(additions: u32, deletions: u32) -> Vec<Span<'static>> {
    let green = Style::default().fg(Color::Green);
    let red = Style::default().fg(Color::Red);
    match (additions, deletions) {
        (0, 0) => vec![],
        (additions, 0) => vec![Span::styled(format!("+{additions}"), green)],
        (0, deletions) => vec![Span::styled(format!("-{deletions}"), red)],
        (additions, deletions) => vec![
            Span::styled(format!("+{additions}"), green),
            Span::styled(format!("-{deletions}"), red),
        ],
    }
}

fn file_stage_icon(file: &FileStat) -> Option<(&'static str, Color)> {
    match (file.has_staged_changes, file.has_unstaged_changes) {
        (true, true) => Some(("◐", Color::Cyan)),
        (true, false) => Some(("●", Color::Green)),
        (false, true) => Some(("○", Color::Yellow)),
        (false, false) => None,
    }
}

fn file_stage_padding(
    area_width: usize,
    visibility: &str,
    display_path: &str,
    file: &FileStat,
) -> usize {
    let base_len = 4usize
        .saturating_add(display_width(visibility))
        .saturating_add(display_width(display_path))
        .saturating_add(plain_text_width(&format_stat_spans(
            file.additions,
            file.deletions,
        )));
    let icon_len = file_stage_icon(file)
        .map(|(symbol, _)| display_width(symbol))
        .unwrap_or(1);

    area_width.saturating_sub(base_len.saturating_add(icon_len).saturating_add(1))
}

fn plain_text_width(spans: &[Span<'_>]) -> usize {
    spans
        .iter()
        .map(|span| display_width(span.content.as_ref()))
        .sum()
}

fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_stage_padding_is_stable_for_hidden_symbol() {
        let file = FileStat {
            path: "tracked.txt".to_string(),
            additions: 3,
            deletions: 1,
            status: FileStatus::Modified,
            has_staged_changes: true,
            has_unstaged_changes: false,
            content_signature: None,
        };

        let visible_padding = file_stage_padding(30, " ", "tracked.txt", &file);
        let hidden_padding = file_stage_padding(30, "⊘", "tracked.txt", &file);

        assert_eq!(visible_padding, hidden_padding);
    }
}
