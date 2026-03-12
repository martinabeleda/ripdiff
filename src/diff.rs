use anyhow::{Context, Result};
use ratatui::text::{Line, Text};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub enum DiffMode {
    Inline,
    SideBySide,
}

impl DiffMode {
    pub fn as_dft_display(&self) -> &'static str {
        match self {
            DiffMode::Inline => "inline",
            DiffMode::SideBySide => "side-by-side-show-both",
        }
    }

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

#[derive(Debug, Clone)]
pub struct DiffContent {
    pub lines: Vec<Line<'static>>,
}

/// Fetch diff for a single file using difftastic as GIT_EXTERNAL_DIFF.
/// Falls back to plain `git diff HEAD` if difft is not available.
/// For untracked files, shows full file content as new.
pub fn fetch_diff(
    repo_root: &Path,
    file_path: &str,
    panel_width: u16,
    mode: &DiffMode,
    is_untracked: bool,
) -> Result<DiffContent> {
    if is_untracked {
        return show_new_file(repo_root, file_path);
    }

    // Try difftastic first
    let difft_result = try_difft(repo_root, file_path, panel_width, mode);

    let output_bytes = match difft_result {
        Ok(bytes) if !bytes.is_empty() => bytes,
        _ => {
            // Fall back to plain git diff
            plain_git_diff(repo_root, file_path)?
        }
    };

    parse_ansi_to_lines(&output_bytes)
}

fn show_new_file(repo_root: &Path, file_path: &str) -> Result<DiffContent> {
    use ratatui::style::{Color, Style};
    use ratatui::text::Span;

    let full_path = repo_root.join(file_path);
    let content = std::fs::read_to_string(&full_path).unwrap_or_else(|_| "[binary file]".into());

    let mut lines = vec![
        Line::from(Span::styled(
            format!("new file: {file_path}"),
            Style::default().fg(Color::Green).add_modifier(ratatui::style::Modifier::BOLD),
        )),
        Line::from(""),
    ];

    for (i, text) in content.lines().enumerate() {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:>4} ", i + 1),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                format!("+ {text}"),
                Style::default().fg(Color::Green),
            ),
        ]));
    }

    Ok(DiffContent { lines })
}

fn try_difft(
    repo_root: &Path,
    file_path: &str,
    panel_width: u16,
    mode: &DiffMode,
) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .current_dir(repo_root)
        .env("GIT_EXTERNAL_DIFF", "difft")
        .env("DFT_COLOR", "always")
        .env("DFT_DISPLAY", mode.as_dft_display())
        .env("DFT_WIDTH", panel_width.to_string())
        .args(["diff", "HEAD", "--", file_path])
        .output()
        .context("Failed to run git with difft")?;

    Ok(output.stdout)
}

fn plain_git_diff(repo_root: &Path, file_path: &str) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .current_dir(repo_root)
        .args(["diff", "HEAD", "--color=always", "--", file_path])
        .output()
        .context("Failed to run git diff")?;

    Ok(output.stdout)
}

fn parse_ansi_to_lines(bytes: &[u8]) -> Result<DiffContent> {
    use ansi_to_tui::IntoText;

    let text: Text = bytes
        .into_text()
        .context("Failed to parse ANSI output")?;

    // Convert to 'static lifetime by cloning
    let lines: Vec<Line<'static>> = text
        .lines
        .into_iter()
        .map(|line| {
            let spans: Vec<ratatui::text::Span<'static>> = line
                .spans
                .into_iter()
                .map(|span| ratatui::text::Span::styled(
                    span.content.into_owned(),
                    span.style,
                ))
                .collect();
            Line::from(spans)
        })
        .collect();

    Ok(DiffContent { lines })
}
