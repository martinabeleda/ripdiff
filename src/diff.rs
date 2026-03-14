use crate::event::Event;
use crate::git::repo_has_head;
use anyhow::{Context, Result};
use difftastic::{render_diff_from_paths, RenderDisplayMode, RenderOptions};
use ratatui::text::{Line, Text};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;
use tokio::sync::mpsc;

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
    let output_bytes = render_tracked_diff(repo_root, file_path, panel_width, mode, has_head)?;

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

fn render_tracked_diff(
    repo_root: &Path,
    file_path: &str,
    panel_width: u16,
    mode: &DiffMode,
    has_head: bool,
) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let diff_specs = if has_head {
        vec![DiffSpec::HeadToWorktree]
    } else {
        vec![DiffSpec::EmptyToIndex, DiffSpec::IndexToWorktree]
    };

    for spec in diff_specs {
        let chunk = render_diff_spec(repo_root, file_path, panel_width, mode, spec)?;
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
enum DiffSpec {
    HeadToWorktree,
    EmptyToIndex,
    IndexToWorktree,
}

fn render_diff_spec(
    repo_root: &Path,
    file_path: &str,
    panel_width: u16,
    mode: &DiffMode,
    spec: DiffSpec,
) -> Result<Vec<u8>> {
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

    let scratch = TempDir::new().context("Failed to create scratch directory for difftastic")?;
    let lhs_path = write_snapshot(&scratch, "lhs", file_path, lhs.as_deref())?;
    let rhs_path = write_snapshot(&scratch, "rhs", file_path, rhs.as_deref())?;

    let rendered = render_diff_from_paths(
        file_path,
        lhs_path.as_deref(),
        rhs_path.as_deref(),
        RenderOptions {
            display_mode: match mode {
                DiffMode::Inline => RenderDisplayMode::Inline,
                DiffMode::SideBySide => RenderDisplayMode::SideBySide,
            },
            terminal_width: usize::from(panel_width),
        },
    );

    Ok(rendered.into_bytes())
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

fn write_snapshot(
    scratch: &TempDir,
    side: &str,
    display_path: &str,
    bytes: Option<&[u8]>,
) -> Result<Option<PathBuf>> {
    let Some(bytes) = bytes else {
        return Ok(None);
    };

    let file_name = Path::new(display_path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("file");
    let path = scratch.path().join(format!("{side}-{file_name}"));
    fs::write(&path, bytes).with_context(|| format!("Failed to write {side} snapshot"))?;
    Ok(Some(path))
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
