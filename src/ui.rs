use crate::app::{App, Panel};
use crate::git::FileStatus;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
};

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    // Title bar (1 line) + main area
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
        .and_then(|n| n.to_str())
        .unwrap_or("?");

    let changed = app.files.len();
    let mode_label = app.diff_mode.label();

    let title_text = if let Some(err) = &app.error_message {
        format!("  ripdiff  [{}]  ERROR: {}  ", repo_name, err)
    } else {
        let panel_label = match app.focus {
            Panel::Files => "files",
            Panel::Diff => "diff",
        };
        format!(
            "  ripdiff  [repo: {}]  {} file{} changed  mode: {}  panel: {}  │  Tab/h/l:panel  j/k:nav  []:hunk  t:mode  r:refresh  q:quit",
            repo_name,
            changed,
            if changed == 1 { "" } else { "s" },
            mode_label,
            panel_label,
        )
    };

    let style = Style::default()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD);

    frame.render_widget(
        Paragraph::new(title_text).style(style),
        area,
    );
}

fn render_body(frame: &mut Frame, app: &mut App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(25), Constraint::Percentage(75)])
        .split(area);

    render_file_list(frame, app, chunks[0]);
    render_diff_panel(frame, app, chunks[1]);
}

fn render_file_list(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .files
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let hidden = app.hidden_files.contains(&f.path);
            let status_color = match f.status {
                FileStatus::Added => Color::Green,
                FileStatus::Deleted => Color::Red,
                FileStatus::Modified => Color::Yellow,
                FileStatus::Renamed => Color::Cyan,
                FileStatus::Untracked => Color::Magenta,
                FileStatus::Unknown => Color::White,
            };

            // Shorten path for display
            let display_path = shorten_path(&f.path, (area.width as usize).saturating_sub(12));

            let eye = if hidden { "⊘" } else { " " };
            let stat_spans = format_stat_spans(f.additions, f.deletions);

            let line = if i == app.selected {
                let mut spans = vec![
                    Span::raw(format!("{eye} ")),
                    Span::styled(
                        f.status.symbol(),
                        Style::default().fg(status_color),
                    ),
                    Span::raw(" "),
                    Span::styled(
                        display_path,
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("  "),
                ];
                spans.extend(stat_spans);
                Line::from(spans)
            } else {
                let mut spans = vec![
                    Span::raw(format!("{eye} ")),
                    Span::styled(
                        f.status.symbol(),
                        Style::default().fg(status_color),
                    ),
                    Span::raw(" "),
                    Span::raw(display_path),
                    Span::raw("  "),
                ];
                spans.extend(stat_spans);
                Line::from(spans)
            };

            ListItem::new(line)
        })
        .collect();

    let no_changes = items.is_empty();

    let border_color = if app.focus == Panel::Files { Color::Cyan } else { Color::DarkGray };
    // Only draw right border as a vertical divider between panels
    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(border_color));

    if no_changes {
        let msg = Paragraph::new(Text::from(vec![
            Line::from(""),
            Line::from(Span::styled("  no changes", Style::default().fg(Color::DarkGray))),
        ]))
        .block(block);
        frame.render_widget(msg, area);
        return;
    }

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        );

    let mut state = ListState::default();
    state.select(Some(app.selected));

    frame.render_stateful_widget(list, area, &mut state);
}

fn render_diff_panel(frame: &mut Frame, app: &mut App, area: Rect) {
    let file_path = app
        .selected_file()
        .map(|f| f.path.clone())
        .unwrap_or_default();

    let is_hidden = app.hidden_files.contains(&file_path);

    // File name header line at top of diff panel
    let header_color = if app.focus == Panel::Diff { Color::Cyan } else { Color::DarkGray };
    let header = if file_path.is_empty() {
        "diff".to_string()
    } else {
        file_path.clone()
    };

    // Split area into header (1 line) + diff content
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    // Render file name header
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" {header}"),
            Style::default().fg(header_color).add_modifier(Modifier::BOLD),
        )),
        chunks[0],
    );

    let content_area = chunks[1];

    // No border block for diff content
    let block = Block::default();

    if app.files.is_empty() {
        let msg = Paragraph::new(Text::from(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No changes detected. Working tree is clean.",
                Style::default().fg(Color::DarkGray),
            )),
        ]))
        .block(block);
        frame.render_widget(msg, content_area);
        return;
    }

    if is_hidden {
        let msg = Paragraph::new(Text::from(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  [hidden — press Space to show]",
                Style::default().fg(Color::DarkGray),
            )),
        ]))
        .block(block);
        frame.render_widget(msg, content_area);
        return;
    }

    let diff_lines = app
        .get_diff()
        .map(|d| d.lines.clone())
        .unwrap_or_else(|| {
            vec![Line::from(Span::styled(
                "  loading...",
                Style::default().fg(Color::DarkGray),
            ))]
        });

    let total_lines = diff_lines.len();
    let inner_height = content_area.height as usize;

    // Clamp scroll offset
    let max_scroll = total_lines.saturating_sub(inner_height);
    let scroll = app.scroll_offset.min(max_scroll);
    app.scroll_offset = scroll;

    let para = Paragraph::new(Text::from(diff_lines))
        .block(block)
        .scroll((scroll as u16, 0));

    frame.render_widget(para, content_area);

    // Scrollbar
    if total_lines > inner_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
        let mut scrollbar_state =
            ScrollbarState::new(total_lines).position(scroll);
        let scrollbar_area = Rect {
            x: content_area.right() - 1,
            y: content_area.y,
            width: 1,
            height: content_area.height,
        };
        frame.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
    }
}

fn shorten_path(path: &str, max_len: usize) -> String {
    if path.len() <= max_len || max_len < 4 {
        return path.to_string();
    }
    let keep = max_len.saturating_sub(3);
    let start = path.len() - keep;
    format!("…{}", &path[start..])
}

fn format_stat_spans(add: u32, del: u32) -> Vec<Span<'static>> {
    let green = Style::default().fg(Color::Green);
    let red = Style::default().fg(Color::Red);
    match (add, del) {
        (0, 0) => vec![],
        (a, 0) => vec![Span::styled(format!("+{a}"), green)],
        (0, d) => vec![Span::styled(format!("-{d}"), red)],
        (a, d) => vec![
            Span::styled(format!("+{a}"), green),
            Span::styled(format!("-{d}"), red),
        ],
    }
}
