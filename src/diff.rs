use crate::event::Event;
use crate::git::repo_has_head;
use anyhow::{Context, Result};
use ratatui::text::{Line, Text};
use std::path::{Path, PathBuf};
use std::process::Command;
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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

    pub fn request(&self, request: DiffRequest, is_untracked: bool) {
        let repo_root = self.repo_root.clone();
        let tx = self.tx.clone();

        tokio::task::spawn_blocking(move || {
            let result = fetch_diff(
                &repo_root,
                &request.path,
                request.panel_width,
                &request.mode,
                is_untracked,
            )
            .map_err(|err| err.to_string());

            let _ = tx.send(Event::DiffLoaded { request, result });
        });
    }
}

/// Fetch diff for a single file using difftastic as GIT_EXTERNAL_DIFF.
/// Falls back to plain `git diff` if difft is not available.
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

    let has_head = repo_has_head(repo_root)?;

    let output_bytes = match try_difft(repo_root, file_path, panel_width, mode, has_head) {
        Ok(bytes) if !bytes.is_empty() => bytes,
        _ => plain_git_diff(repo_root, file_path, has_head)?,
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
            Style::default()
                .fg(Color::Green)
                .add_modifier(ratatui::style::Modifier::BOLD),
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

fn try_difft(
    repo_root: &Path,
    file_path: &str,
    panel_width: u16,
    mode: &DiffMode,
    has_head: bool,
) -> Result<Vec<u8>> {
    collect_diff_output(repo_root, file_path, has_head, |command| {
        command
            .env("GIT_EXTERNAL_DIFF", "difft")
            .env("DFT_COLOR", "always")
            .env("DFT_DISPLAY", mode.as_dft_display())
            .env("DFT_WIDTH", panel_width.to_string());
    })
    .context("Failed to run git with difft")
}

fn plain_git_diff(repo_root: &Path, file_path: &str, has_head: bool) -> Result<Vec<u8>> {
    collect_diff_output(repo_root, file_path, has_head, |command| {
        command.arg("--color=always");
    })
    .context("Failed to run git diff")
}

fn collect_diff_output<F>(
    repo_root: &Path,
    file_path: &str,
    has_head: bool,
    configure: F,
) -> Result<Vec<u8>>
where
    F: Fn(&mut Command),
{
    let diff_specs = if has_head {
        vec![DiffSpec {
            staged: false,
            against_head: true,
        }]
    } else {
        vec![
            DiffSpec {
                staged: true,
                against_head: false,
            },
            DiffSpec {
                staged: false,
                against_head: false,
            },
        ]
    };

    let mut output = Vec::new();
    for spec in diff_specs {
        let chunk = run_git_diff(repo_root, file_path, &spec, &configure)?;
        if chunk.is_empty() {
            continue;
        }
        if !output.is_empty() && !output.ends_with(b"\n") {
            output.push(b'\n');
        }
        output.extend(chunk);
    }

    Ok(output)
}

#[derive(Clone, Copy)]
struct DiffSpec {
    staged: bool,
    against_head: bool,
}

fn run_git_diff<F>(
    repo_root: &Path,
    file_path: &str,
    spec: &DiffSpec,
    configure: &F,
) -> Result<Vec<u8>>
where
    F: Fn(&mut Command),
{
    let mut command = Command::new("git");
    command.current_dir(repo_root);
    command.arg("diff");
    if spec.staged {
        command.arg("--cached");
    }
    if spec.against_head {
        command.arg("HEAD");
    }
    configure(&mut command);
    command.args(["--", file_path]);

    let output = command.output()?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Ok(Vec::new())
    }
}

fn parse_ansi_to_lines(bytes: &[u8]) -> Result<DiffContent> {
    use ansi_to_tui::IntoText;

    let text: Text = bytes.into_text().context("Failed to parse ANSI output")?;

    let lines: Vec<Line<'static>> = text
        .lines
        .into_iter()
        .map(|line| {
            let spans: Vec<ratatui::text::Span<'static>> = line
                .spans
                .into_iter()
                .map(|span| ratatui::text::Span::styled(span.content.into_owned(), span.style))
                .collect();
            Line::from(spans)
        })
        .collect();

    Ok(DiffContent { lines })
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
}
