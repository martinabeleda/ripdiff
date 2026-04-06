use crate::event::Event;
use crate::git::repo_has_head;
use anyhow::{Context, Result};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tokio::sync::mpsc;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use vendored_difftastic::{
    ChangeSpan, DiffRequest as SemanticDiffRequest, DiffStatus, SemanticLine,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DiffMode {
    Inline,
    SideBySide,
}

impl DiffMode {
    pub fn label(&self) -> &'static str {
        match self {
            DiffMode::Inline => "inline",
            DiffMode::SideBySide => "side-by-side",
        }
    }

    pub fn toggle(&self) -> Self {
        match self {
            DiffMode::Inline => DiffMode::SideBySide,
            DiffMode::SideBySide => DiffMode::Inline,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DiffRequest {
    pub path: String,
    pub panel_width: u16,
    pub mode: DiffMode,
}

#[derive(Debug, Clone)]
pub struct DiffContent {
    pub lines: Vec<Line<'static>>,
}

#[derive(Clone)]
pub struct DiffService {
    repo_root: PathBuf,
    tx: mpsc::UnboundedSender<Event>,
}

impl DiffService {
    pub fn new(repo_root: PathBuf, tx: mpsc::UnboundedSender<Event>) -> Self {
        Self { repo_root, tx }
    }

    pub fn request(&self, request: DiffRequest, is_untracked: bool, show_unstaged_only: bool) {
        let repo_root = self.repo_root.clone();
        let tx = self.tx.clone();

        tokio::task::spawn_blocking(move || {
            let result = fetch_diff_with_options(
                &repo_root,
                &request.path,
                request.panel_width,
                &request.mode,
                is_untracked,
                show_unstaged_only,
            )
            .map_err(|err| err.to_string());

            let _ = tx.send(Event::DiffLoaded { request, result });
        });
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn fetch_diff(
    repo_root: &Path,
    file_path: &str,
    panel_width: u16,
    mode: &DiffMode,
    is_untracked: bool,
) -> Result<DiffContent> {
    fetch_diff_with_options(repo_root, file_path, panel_width, mode, is_untracked, false)
}

pub fn fetch_diff_with_options(
    repo_root: &Path,
    file_path: &str,
    panel_width: u16,
    mode: &DiffMode,
    is_untracked: bool,
    show_unstaged_only: bool,
) -> Result<DiffContent> {
    if is_untracked {
        return show_new_file(repo_root, file_path);
    }

    let has_head = repo_has_head(repo_root)?;
    render_tracked_diff(
        repo_root,
        file_path,
        panel_width,
        mode,
        has_head,
        show_unstaged_only,
    )
}

fn show_new_file(repo_root: &Path, file_path: &str) -> Result<DiffContent> {
    let full_path = repo_root.join(file_path);
    let content = std::fs::read_to_string(&full_path).unwrap_or_else(|_| "[binary file]".into());

    let mut lines = vec![
        Line::from(Span::styled(
            format!("new file: {file_path}"),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    for (i, text) in content.lines().enumerate() {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:>4} ", i + 1),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(format!("+ {text}"), Style::default().fg(Color::Green)),
        ]));
    }

    Ok(DiffContent { lines })
}

fn render_tracked_diff(
    repo_root: &Path,
    file_path: &str,
    panel_width: u16,
    mode: &DiffMode,
    has_head: bool,
    show_unstaged_only: bool,
) -> Result<DiffContent> {
    let mut lines = Vec::new();
    let diff_specs = if show_unstaged_only {
        vec![DiffSpec::IndexToWorktree]
    } else if has_head {
        vec![DiffSpec::HeadToWorktree]
    } else {
        vec![DiffSpec::EmptyToIndex, DiffSpec::IndexToWorktree]
    };

    for spec in diff_specs {
        let chunk = render_diff_spec(repo_root, file_path, panel_width, mode, spec)?;
        if chunk.is_empty() {
            continue;
        }

        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.extend(chunk);
    }

    Ok(DiffContent { lines })
}

#[derive(Clone, Copy)]
enum DiffSpec {
    HeadToWorktree,
    EmptyToIndex,
    IndexToWorktree,
}

impl DiffSpec {
    fn label(self) -> &'static str {
        match self {
            DiffSpec::HeadToWorktree => "HEAD ↔ worktree",
            DiffSpec::EmptyToIndex => "(empty) ↔ index",
            DiffSpec::IndexToWorktree => "index ↔ worktree",
        }
    }
}

fn render_diff_spec(
    repo_root: &Path,
    file_path: &str,
    panel_width: u16,
    mode: &DiffMode,
    spec: DiffSpec,
) -> Result<Vec<Line<'static>>> {
    let (lhs, rhs) = match spec {
        DiffSpec::HeadToWorktree => (
            git_revision_bytes(repo_root, "HEAD", file_path)?,
            worktree_bytes(repo_root, file_path)?,
        ),
        DiffSpec::EmptyToIndex => (None, git_index_bytes(repo_root, file_path)?),
        DiffSpec::IndexToWorktree => (
            git_index_bytes(repo_root, file_path)?,
            worktree_bytes(repo_root, file_path)?,
        ),
    };

    if lhs == rhs {
        return Ok(Vec::new());
    }

    let lhs_bytes = lhs.as_deref().unwrap_or(&[]);
    let rhs_bytes = rhs.as_deref().unwrap_or(&[]);
    let semantic = vendored_difftastic::diff_bytes_semantic(SemanticDiffRequest {
        display_path: file_path,
        lhs_path: lhs.as_ref().map(|_| Path::new(file_path)),
        rhs_path: rhs.as_ref().map(|_| Path::new(file_path)),
        lhs_bytes,
        rhs_bytes,
    })
    .context("Failed to render semantic diff")?;

    let lhs_text = String::from_utf8_lossy(lhs_bytes);
    let rhs_text = String::from_utf8_lossy(rhs_bytes);
    let lhs_lines: Vec<&str> = lhs_text.lines().collect();
    let rhs_lines: Vec<&str> = rhs_text.lines().collect();

    let mut rendered = vec![Line::from(Span::styled(
        format!("{file_path} ({})", spec.label()),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))];

    if semantic.status == DiffStatus::Binary {
        rendered.push(Line::from(Span::styled(
            "[binary diff]",
            Style::default().fg(Color::Yellow),
        )));
        return Ok(rendered);
    }

    if semantic.status == DiffStatus::Created {
        for (i, text) in rhs_lines.iter().enumerate() {
            rendered.push(render_single_side_line(
                '+',
                i as u32,
                text,
                Color::Green,
                &[],
            ));
        }
        return Ok(rendered);
    }

    if semantic.status == DiffStatus::Deleted {
        for (i, text) in lhs_lines.iter().enumerate() {
            rendered.push(render_single_side_line(
                '-',
                i as u32,
                text,
                Color::Red,
                &[],
            ));
        }
        return Ok(rendered);
    }

    for chunk in semantic.chunks {
        for line in chunk.lines {
            rendered.extend(render_semantic_line(
                &line,
                &lhs_lines,
                &rhs_lines,
                panel_width,
                mode,
            ));
        }
    }

    if rendered.len() == 1 {
        rendered.push(Line::from("[no visible changes]"));
    }

    Ok(rendered)
}

fn render_semantic_line(
    line: &SemanticLine,
    lhs_lines: &[&str],
    rhs_lines: &[&str],
    panel_width: u16,
    mode: &DiffMode,
) -> Vec<Line<'static>> {
    match mode {
        DiffMode::Inline => {
            let mut out = Vec::new();
            if let Some(lhs_line) = line.lhs_line {
                out.push(render_single_side_line(
                    '-',
                    lhs_line,
                    line_at(lhs_lines, lhs_line),
                    Color::Red,
                    &line.lhs_changes,
                ));
            }
            if let Some(rhs_line) = line.rhs_line {
                out.push(render_single_side_line(
                    '+',
                    rhs_line,
                    line_at(rhs_lines, rhs_line),
                    Color::Green,
                    &line.rhs_changes,
                ));
            }
            out
        }
        DiffMode::SideBySide => vec![render_side_by_side_line(
            line,
            lhs_lines,
            rhs_lines,
            panel_width,
        )],
    }
}

fn render_single_side_line(
    prefix: char,
    line_num: u32,
    text: &str,
    color: Color,
    changes: &[ChangeSpan],
) -> Line<'static> {
    let mut spans = vec![Span::styled(
        format!("{prefix}{:>4} ", line_num.saturating_add(1)),
        Style::default().fg(Color::DarkGray),
    )];
    spans.extend(styled_text_with_changes(text, color, changes));
    Line::from(spans)
}

fn styled_text_with_changes(
    text: &str,
    base_color: Color,
    changes: &[ChangeSpan],
) -> Vec<Span<'static>> {
    if changes.is_empty() {
        return vec![Span::styled(
            text.to_owned(),
            Style::default().fg(base_color),
        )];
    }

    let mut sorted = changes.to_vec();
    sorted.sort_by_key(|change| change.start_col);

    let mut spans = Vec::new();
    let mut cursor = 0usize;

    for change in sorted {
        let start = byte_index_for_display_col(text, change.start_col as usize);
        let end = byte_index_for_display_col(text, change.end_col as usize).max(start);

        if start > cursor {
            spans.push(Span::styled(
                text[cursor..start].to_owned(),
                Style::default().fg(base_color),
            ));
        }

        if end > start {
            spans.push(Span::styled(
                text[start..end].to_owned(),
                Style::default()
                    .fg(accent_color(base_color))
                    .add_modifier(Modifier::BOLD),
            ));
            cursor = end;
        }
    }

    if cursor < text.len() {
        spans.push(Span::styled(
            text[cursor..].to_owned(),
            Style::default().fg(base_color),
        ));
    }

    if spans.is_empty() {
        vec![Span::styled(
            text.to_owned(),
            Style::default().fg(base_color),
        )]
    } else {
        spans
    }
}

fn accent_color(base: Color) -> Color {
    match base {
        Color::Red => Color::LightRed,
        Color::Green => Color::LightGreen,
        other => other,
    }
}

fn byte_index_for_display_col(text: &str, target_col: usize) -> usize {
    if target_col == 0 {
        return 0;
    }

    let mut col = 0usize;
    for (idx, ch) in text.char_indices() {
        let width = ch.width().unwrap_or(0).max(1);
        if col >= target_col {
            return idx;
        }
        col += width;
        if col >= target_col {
            return idx + ch.len_utf8();
        }
    }
    text.len()
}

fn render_side_by_side_line(
    line: &SemanticLine,
    lhs_lines: &[&str],
    rhs_lines: &[&str],
    panel_width: u16,
) -> Line<'static> {
    let total_width = usize::from(panel_width).max(40);
    let separator = " │ ";
    let side_width = total_width.saturating_sub(separator.len()) / 2;

    let lhs_text = line
        .lhs_line
        .map(|n| format!("-{:>4} {}", n.saturating_add(1), line_at(lhs_lines, n)))
        .unwrap_or_default();
    let rhs_text = line
        .rhs_line
        .map(|n| format!("+{:>4} {}", n.saturating_add(1), line_at(rhs_lines, n)))
        .unwrap_or_default();

    let lhs = pad_to_width(&truncate_to_width(&lhs_text, side_width), side_width);
    let rhs = pad_to_width(&truncate_to_width(&rhs_text, side_width), side_width);

    Line::from(vec![
        Span::styled(lhs, Style::default().fg(Color::Red)),
        Span::styled(separator, Style::default().fg(Color::DarkGray)),
        Span::styled(rhs, Style::default().fg(Color::Green)),
    ])
}

fn truncate_to_width(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if text.width() <= width {
        return text.to_owned();
    }

    let mut out = String::new();
    let mut used = 0usize;
    let budget = width.saturating_sub(1);

    for ch in text.chars() {
        let ch_width = ch.width().unwrap_or(0).max(1);
        if used + ch_width > budget {
            break;
        }
        out.push(ch);
        used += ch_width;
    }

    out.push('…');
    out
}

fn pad_to_width(text: &str, width: usize) -> String {
    let current = text.width();
    if current >= width {
        return text.to_owned();
    }

    let mut out = String::with_capacity(text.len() + (width - current));
    out.push_str(text);
    out.push_str(&" ".repeat(width - current));
    out
}

fn line_at<'a>(lines: &'a [&str], line_num: u32) -> &'a str {
    lines.get(line_num as usize).copied().unwrap_or("")
}

fn git_revision_bytes(
    repo_root: &Path,
    revision: &str,
    file_path: &str,
) -> Result<Option<Vec<u8>>> {
    git_object_bytes(repo_root, &format!("{revision}:{file_path}"))
}

fn git_index_bytes(repo_root: &Path, file_path: &str) -> Result<Option<Vec<u8>>> {
    git_object_bytes(repo_root, &format!(":{file_path}"))
}

fn git_object_bytes(repo_root: &Path, spec: &str) -> Result<Option<Vec<u8>>> {
    let output = Command::new("git")
        .current_dir(repo_root)
        .args(["show", spec])
        .output()
        .with_context(|| format!("Failed to run git show {spec}"))?;

    if output.status.success() {
        Ok(Some(output.stdout))
    } else {
        Ok(None)
    }
}

fn worktree_bytes(repo_root: &Path, file_path: &str) -> Result<Option<Vec<u8>>> {
    let path = repo_root.join(file_path);
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).context("Failed to read worktree file"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn run_git(repo: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(repo)
            .args(args)
            .status()
            .expect("git command should start");
        assert!(status.success(), "git command failed: git {:?}", args);
    }

    fn init_repo() -> TempDir {
        let temp = TempDir::new().expect("temp dir should be created");
        run_git(temp.path(), &["init", "-q"]);
        temp
    }

    #[test]
    fn fetch_diff_shows_staged_content_in_unborn_repo() {
        let temp = init_repo();
        fs::write(temp.path().join("staged.txt"), "hello\n").expect("fixture should be written");
        run_git(temp.path(), &["add", "staged.txt"]);

        let diff = fetch_diff(temp.path(), "staged.txt", 80, &DiffMode::Inline, false)
            .expect("diff should load");
        let rendered = diff
            .lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            rendered.contains("staged.txt"),
            "expected diff to mention the staged file, got: {rendered}"
        );
        assert!(
            rendered.contains("hello"),
            "expected diff to include staged content, got: {rendered}"
        );
    }

    #[test]
    fn fetch_diff_with_options_omits_staged_hunks_when_showing_unstaged_only() {
        let temp = init_repo();
        fs::write(temp.path().join("tracked.txt"), "base\n").expect("fixture should be written");
        run_git(temp.path(), &["add", "tracked.txt"]);
        run_git(
            temp.path(),
            &[
                "-c",
                "user.name=Test User",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-qm",
                "init",
            ],
        );

        fs::write(temp.path().join("tracked.txt"), "staged\n")
            .expect("staged edit should be written");
        run_git(temp.path(), &["add", "tracked.txt"]);
        fs::write(temp.path().join("tracked.txt"), "staged\nunstaged\n")
            .expect("unstaged edit should be written");

        let full = fetch_diff_with_options(
            temp.path(),
            "tracked.txt",
            80,
            &DiffMode::Inline,
            false,
            false,
        )
        .expect("full diff should load");
        let unstaged_only = fetch_diff_with_options(
            temp.path(),
            "tracked.txt",
            80,
            &DiffMode::Inline,
            false,
            true,
        )
        .expect("unstaged-only diff should load");

        let render = |diff: DiffContent| {
            diff.lines
                .iter()
                .map(|line| {
                    line.spans
                        .iter()
                        .map(|span| span.content.as_ref())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let full_rendered = render(full);
        let unstaged_rendered = render(unstaged_only);

        assert!(
            full_rendered.contains("base"),
            "expected full diff to include staged hunk context, got: {full_rendered}"
        );
        assert!(
            unstaged_rendered.contains("unstaged"),
            "expected unstaged-only diff to include worktree hunk, got: {unstaged_rendered}"
        );
        assert!(
            !unstaged_rendered.contains("base"),
            "expected unstaged-only diff to omit staged hunk, got: {unstaged_rendered}"
        );
    }
}
