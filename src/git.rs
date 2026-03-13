use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Unknown,
}

impl FileStatus {
    pub fn symbol(&self) -> &'static str {
        match self {
            FileStatus::Modified => "M",
            FileStatus::Added => "A",
            FileStatus::Deleted => "D",
            FileStatus::Renamed => "R",
            FileStatus::Untracked => "?",
            FileStatus::Unknown => "?",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStat {
    pub path: String,
    pub additions: u32,
    pub deletions: u32,
    pub status: FileStatus,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoSnapshot {
    pub files: Vec<FileStat>,
}

pub fn repo_root(start: &Path) -> Result<PathBuf> {
    run_git_utf8(start, &["rev-parse", "--show-toplevel"])
        .map(PathBuf::from)
        .context("Failed to resolve repository root")
}

pub fn git_dir(start: &Path) -> Result<PathBuf> {
    run_git_utf8(start, &["rev-parse", "--absolute-git-dir"])
        .map(PathBuf::from)
        .context("Failed to resolve git directory")
}

pub fn repo_has_head(repo_root: &Path) -> Result<bool> {
    let output = Command::new("git")
        .current_dir(repo_root)
        .args(["rev-parse", "--verify", "HEAD"])
        .output()
        .context("Failed to run git rev-parse --verify HEAD")?;

    Ok(output.status.success())
}

pub fn load_snapshot(repo_root: &Path) -> Result<RepoSnapshot> {
    let mut files = parse_status_porcelain(repo_root)?;
    let mut stats = parse_numstat(repo_root, true)?;

    for (path, diff_stat) in parse_numstat(repo_root, false)? {
        let entry = stats.entry(path).or_default();
        entry.additions = entry.additions.saturating_add(diff_stat.additions);
        entry.deletions = entry.deletions.saturating_add(diff_stat.deletions);
    }

    for file in &mut files {
        if let Some(diff_stat) = stats.remove(&file.path) {
            file.additions = diff_stat.additions;
            file.deletions = diff_stat.deletions;
        } else if file.status == FileStatus::Untracked {
            file.additions = count_lines(repo_root.join(&file.path));
        }
    }

    Ok(RepoSnapshot { files })
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn list_changed_files(repo_root: &Path) -> Result<Vec<FileStat>> {
    Ok(load_snapshot(repo_root)?.files)
}

fn run_git_utf8(repo: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .with_context(|| format!("Failed to run git {}", args.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("{}", stderr.trim());
    }

    String::from_utf8(output.stdout)
        .map(|text| text.trim().to_string())
        .context("git output not UTF-8")
}

fn parse_status_porcelain(repo_root: &Path) -> Result<Vec<FileStat>> {
    let output = Command::new("git")
        .current_dir(repo_root)
        .args([
            "status",
            "--porcelain=v2",
            "--find-renames",
            "--untracked-files=all",
            "-z",
        ])
        .output()
        .context("Failed to run git status --porcelain=v2")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("{}", stderr.trim());
    }

    let records = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .collect::<Vec<_>>();

    let mut files = Vec::new();
    let mut index = 0;

    while index < records.len() {
        let record = std::str::from_utf8(records[index]).context("git status output not UTF-8")?;
        match record.chars().next() {
            Some('1') | Some('u') => {
                if let Some(file) = parse_regular_status(record) {
                    files.push(file);
                }
                index += 1;
            }
            Some('2') => {
                if let Some(file) = parse_rename_status(record) {
                    files.push(file);
                }
                index += 2;
            }
            Some('?') => {
                if let Some(path) = record.strip_prefix("? ") {
                    files.push(FileStat {
                        path: path.to_string(),
                        additions: 0,
                        deletions: 0,
                        status: FileStatus::Untracked,
                    });
                }
                index += 1;
            }
            Some('!') => index += 1,
            _ => index += 1,
        }
    }

    Ok(files)
}

fn parse_regular_status(record: &str) -> Option<FileStat> {
    let mut parts = record.splitn(3, ' ');
    let kind = parts.next()?;
    let xy = parts.next()?;
    let field_count = if kind == "u" { 11 } else { 9 };
    let path = nth_space_field(record, field_count)?.to_string();
    let status = if kind == "u" {
        FileStatus::Modified
    } else {
        status_from_xy(xy)
    };

    Some(FileStat {
        path,
        additions: 0,
        deletions: 0,
        status,
    })
}

fn parse_rename_status(record: &str) -> Option<FileStat> {
    let xy = record.split(' ').nth(1)?;
    let path = nth_space_field(record, 10)?.to_string();

    Some(FileStat {
        path,
        additions: 0,
        deletions: 0,
        status: if matches!(status_from_xy(xy), FileStatus::Unknown) {
            FileStatus::Renamed
        } else {
            status_from_xy(xy)
        },
    })
}

fn nth_space_field(record: &str, field_index: usize) -> Option<&str> {
    record.splitn(field_index, ' ').nth(field_index - 1)
}

fn status_from_xy(xy: &str) -> FileStatus {
    let chars = xy.chars().collect::<Vec<_>>();
    let x = chars.first().copied().unwrap_or('.');
    let y = chars.get(1).copied().unwrap_or('.');

    if matches!(x, 'R') || matches!(y, 'R') {
        FileStatus::Renamed
    } else if matches!(x, 'A') || matches!(y, 'A') {
        FileStatus::Added
    } else if matches!(x, 'D') || matches!(y, 'D') {
        FileStatus::Deleted
    } else if matches!(x, 'M' | 'T' | 'U') || matches!(y, 'M' | 'T' | 'U') {
        FileStatus::Modified
    } else {
        FileStatus::Unknown
    }
}

#[derive(Default)]
struct DiffStat {
    additions: u32,
    deletions: u32,
}

fn parse_numstat(repo_root: &Path, staged: bool) -> Result<HashMap<String, DiffStat>> {
    let mut command = Command::new("git");
    command.current_dir(repo_root);
    command.arg("diff");
    if staged {
        command.arg("--cached");
    }
    command.args(["--numstat", "-z"]);

    let output = command
        .output()
        .context("Failed to run git diff --numstat")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("{}", stderr.trim());
    }

    let records = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .collect::<Vec<_>>();

    let mut stats = HashMap::new();
    let mut index = 0;
    while index < records.len() {
        let text = std::str::from_utf8(records[index]).context("git numstat output not UTF-8")?;
        let Some((additions, rest)) = text.split_once('\t') else {
            index += 1;
            continue;
        };
        let Some((deletions, path)) = rest.split_once('\t') else {
            index += 1;
            continue;
        };

        let path = if path.is_empty() {
            let Some(new_path_record) = records.get(index + 2) else {
                index += 1;
                continue;
            };
            index += 3;
            std::str::from_utf8(new_path_record).context("git numstat output not UTF-8")?
        } else {
            index += 1;
            path
        };

        stats.insert(
            path.to_string(),
            DiffStat {
                additions: additions.parse::<u32>().unwrap_or(0),
                deletions: deletions.parse::<u32>().unwrap_or(0),
            },
        );
    }

    Ok(stats)
}

fn count_lines(path: PathBuf) -> u32 {
    std::fs::read_to_string(path)
        .map(|content| content.lines().count() as u32)
        .unwrap_or(0)
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

    fn run_git_with_identity(repo: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(repo)
            .args([
                "-c",
                "user.name=Test User",
                "-c",
                "user.email=test@example.com",
            ])
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
    fn list_changed_files_uses_new_path_for_renames() {
        let temp = init_repo();
        fs::write(temp.path().join("old.txt"), "before\n").expect("fixture should be written");
        run_git(temp.path(), &["add", "old.txt"]);
        run_git_with_identity(temp.path(), &["commit", "-qm", "init"]);

        run_git(temp.path(), &["mv", "old.txt", "new.txt"]);
        fs::write(temp.path().join("new.txt"), "before\nafter\n")
            .expect("rename target should update");

        let files = list_changed_files(temp.path()).expect("changed files should load");

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "new.txt");
        assert_eq!(files[0].status, FileStatus::Renamed);
    }

    #[test]
    fn list_changed_files_preserves_spaces_in_tracked_paths() {
        let temp = init_repo();
        let path = "two words.txt";
        fs::write(temp.path().join(path), "before\n").expect("fixture should be written");
        run_git(temp.path(), &["add", path]);
        run_git_with_identity(temp.path(), &["commit", "-qm", "init"]);

        fs::write(temp.path().join(path), "before\nafter\n").expect("fixture should update");

        let files = list_changed_files(temp.path()).expect("changed files should load");

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, path);
        assert_eq!(files[0].status, FileStatus::Modified);
        assert_eq!(files[0].additions, 1);
    }

    #[test]
    fn list_changed_files_includes_staged_files_in_unborn_repo() {
        let temp = init_repo();
        fs::write(temp.path().join("staged.txt"), "hello\n").expect("fixture should be written");
        run_git(temp.path(), &["add", "staged.txt"]);

        let files = list_changed_files(temp.path()).expect("changed files should load");

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "staged.txt");
        assert_eq!(files[0].status, FileStatus::Added);
        assert_eq!(files[0].additions, 1);
    }

    #[test]
    fn list_changed_files_tracks_stats_for_renamed_files() {
        let temp = init_repo();
        fs::write(temp.path().join("old.txt"), "before\n").expect("fixture should be written");
        run_git(temp.path(), &["add", "old.txt"]);
        run_git_with_identity(temp.path(), &["commit", "-qm", "init"]);

        run_git(temp.path(), &["mv", "old.txt", "new name.txt"]);
        fs::write(temp.path().join("new name.txt"), "before\nafter\n")
            .expect("rename target should update");

        let files = list_changed_files(temp.path()).expect("changed files should load");

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "new name.txt");
        assert_eq!(files[0].status, FileStatus::Renamed);
        assert_eq!(files[0].additions, 1);
        assert_eq!(files[0].deletions, 0);
    }

    #[test]
    fn git_dir_resolves_linked_worktree_gitdir() {
        let temp = init_repo();
        fs::create_dir(temp.path().join("nested")).expect("nested dir should exist");
        let worktree_path = temp.path().join("nested").join("wt");

        run_git_with_identity(temp.path(), &["commit", "--allow-empty", "-qm", "init"]);
        run_git(
            temp.path(),
            &[
                "worktree",
                "add",
                worktree_path.to_str().expect("utf-8 path"),
                "-q",
            ],
        );

        let resolved = git_dir(&worktree_path).expect("git dir should resolve");

        assert!(resolved.is_dir(), "expected git dir to be a directory");
        assert_ne!(resolved, worktree_path.join(".git"));
    }
}
