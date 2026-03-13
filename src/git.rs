use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;

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
    pub has_staged_changes: bool,
    pub has_unstaged_changes: bool,
    pub content_signature: Option<FileContentSignature>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileContentSignature {
    pub len: u64,
    pub modified_unix_nanos: u128,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoSnapshot {
    pub branch: Option<String>,
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

pub fn stage_file(repo_root: &Path, path: &str) -> Result<()> {
    let output = Command::new("git")
        .current_dir(repo_root)
        .args(["add", "--", path])
        .output()
        .with_context(|| format!("Failed to run git add -- {path}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("{}", stderr.trim());
    }

    Ok(())
}

pub fn stage_all(repo_root: &Path) -> Result<()> {
    let output = Command::new("git")
        .current_dir(repo_root)
        .args(["add", "--all"])
        .output()
        .context("Failed to run git add --all")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("{}", stderr.trim());
    }

    Ok(())
}

pub fn unstage_file(repo_root: &Path, path: &str) -> Result<()> {
    if repo_has_head(repo_root)? {
        run_git_ok(
            repo_root,
            &["restore", "--staged", "--", path],
            &format!("Failed to run git restore --staged -- {path}"),
        )
    } else {
        run_git_ok(
            repo_root,
            &["rm", "--cached", "--", path],
            &format!("Failed to run git rm --cached -- {path}"),
        )
    }
}

pub fn unstage_all(repo_root: &Path) -> Result<()> {
    if repo_has_head(repo_root)? {
        run_git_ok(
            repo_root,
            &["restore", "--staged", "--", "."],
            "Failed to run git restore --staged -- .",
        )
    } else {
        run_git_ok(
            repo_root,
            &["rm", "-r", "--cached", "--", "."],
            "Failed to run git rm -r --cached -- .",
        )
    }
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
    let branch = current_branch(repo_root)?;
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
        file.content_signature = file_content_signature(repo_root.join(&file.path));
    }

    Ok(RepoSnapshot { branch, files })
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

fn run_git_ok(repo: &Path, args: &[&str], context: &str) -> Result<()> {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .context(context.to_string())?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("{}", stderr.trim());
    }

    Ok(())
}

fn current_branch(repo_root: &Path) -> Result<Option<String>> {
    let branch = run_git_utf8(repo_root, &["branch", "--show-current"])?;
    if !branch.is_empty() {
        return Ok(Some(branch));
    }

    let output = Command::new("git")
        .current_dir(repo_root)
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .context("Failed to run git rev-parse --short HEAD")?;

    if !output.status.success() {
        return Ok(None);
    }

    let short_head = String::from_utf8(output.stdout)
        .map(|text| text.trim().to_string())
        .context("git output not UTF-8")?;

    if short_head.is_empty() {
        Ok(None)
    } else {
        Ok(Some(format!("detached@{short_head}")))
    }
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
                        has_staged_changes: false,
                        has_unstaged_changes: true,
                        content_signature: None,
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
        has_staged_changes: kind == "u" || has_index_change(xy),
        has_unstaged_changes: kind == "u" || has_worktree_change(xy),
        content_signature: None,
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
        has_staged_changes: has_index_change(xy),
        has_unstaged_changes: has_worktree_change(xy),
        content_signature: None,
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

fn has_index_change(xy: &str) -> bool {
    xy.chars()
        .next()
        .map(is_status_change_char)
        .unwrap_or(false)
}

fn has_worktree_change(xy: &str) -> bool {
    xy.chars()
        .nth(1)
        .map(is_status_change_char)
        .unwrap_or(false)
}

fn is_status_change_char(ch: char) -> bool {
    !matches!(ch, '.' | ' ')
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

fn file_content_signature(path: PathBuf) -> Option<FileContentSignature> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    let modified_unix_nanos = modified.duration_since(UNIX_EPOCH).ok()?.as_nanos();

    Some(FileContentSignature {
        len: metadata.len(),
        modified_unix_nanos,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::thread;
    use std::time::Duration;
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
        assert!(files[0].has_staged_changes);
        assert!(files[0].has_unstaged_changes);
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
        assert!(!files[0].has_staged_changes);
        assert!(files[0].has_unstaged_changes);
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
        assert!(files[0].has_staged_changes);
        assert!(!files[0].has_unstaged_changes);
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
        assert!(files[0].has_staged_changes);
        assert!(files[0].has_unstaged_changes);
        assert_eq!(files[0].additions, 1);
        assert_eq!(files[0].deletions, 0);
    }

    #[test]
    fn list_changed_files_marks_mixed_staged_and_unstaged_changes() {
        let temp = init_repo();
        fs::write(temp.path().join("tracked.txt"), "before\n").expect("fixture should be written");
        run_git(temp.path(), &["add", "tracked.txt"]);
        run_git_with_identity(temp.path(), &["commit", "-qm", "init"]);

        fs::write(temp.path().join("tracked.txt"), "staged\n")
            .expect("staged edit should be written");
        run_git(temp.path(), &["add", "tracked.txt"]);
        fs::write(temp.path().join("tracked.txt"), "staged\nunstaged\n")
            .expect("unstaged edit should be written");

        let files = list_changed_files(temp.path()).expect("changed files should load");

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "tracked.txt");
        assert!(files[0].has_staged_changes);
        assert!(files[0].has_unstaged_changes);
    }

    #[test]
    fn stage_file_stages_selected_path() {
        let temp = init_repo();
        fs::write(temp.path().join("tracked.txt"), "before\n").expect("fixture should be written");
        run_git(temp.path(), &["add", "tracked.txt"]);
        run_git_with_identity(temp.path(), &["commit", "-qm", "init"]);

        fs::write(temp.path().join("tracked.txt"), "before\nafter\n")
            .expect("fixture should update");

        stage_file(temp.path(), "tracked.txt").expect("file should stage");

        let files = list_changed_files(temp.path()).expect("changed files should load");
        assert_eq!(files.len(), 1);
        assert!(files[0].has_staged_changes);
        assert!(!files[0].has_unstaged_changes);
    }

    #[test]
    fn stage_all_stages_all_paths() {
        let temp = init_repo();
        fs::write(temp.path().join("tracked.txt"), "before\n").expect("fixture should be written");
        run_git(temp.path(), &["add", "tracked.txt"]);
        run_git_with_identity(temp.path(), &["commit", "-qm", "init"]);

        fs::write(temp.path().join("tracked.txt"), "before\nafter\n")
            .expect("tracked update should be written");
        fs::write(temp.path().join("new.txt"), "hello\n")
            .expect("untracked file should be written");

        stage_all(temp.path()).expect("all files should stage");

        let files = list_changed_files(temp.path()).expect("changed files should load");
        assert_eq!(files.len(), 2);
        assert!(files.iter().all(|file| file.has_staged_changes));
        assert!(files.iter().all(|file| !file.has_unstaged_changes));
    }

    #[test]
    fn unstage_file_restores_selected_path_to_unstaged() {
        let temp = init_repo();
        fs::write(temp.path().join("tracked.txt"), "before\n").expect("fixture should be written");
        run_git(temp.path(), &["add", "tracked.txt"]);
        run_git_with_identity(temp.path(), &["commit", "-qm", "init"]);

        fs::write(temp.path().join("tracked.txt"), "before\nafter\n")
            .expect("fixture should update");
        run_git(temp.path(), &["add", "tracked.txt"]);

        unstage_file(temp.path(), "tracked.txt").expect("file should unstage");

        let files = list_changed_files(temp.path()).expect("changed files should load");
        assert_eq!(files.len(), 1);
        assert!(!files[0].has_staged_changes);
        assert!(files[0].has_unstaged_changes);
    }

    #[test]
    fn unstage_all_restores_all_paths_to_unstaged() {
        let temp = init_repo();
        fs::write(temp.path().join("tracked.txt"), "before\n").expect("fixture should be written");
        run_git(temp.path(), &["add", "tracked.txt"]);
        run_git_with_identity(temp.path(), &["commit", "-qm", "init"]);

        fs::write(temp.path().join("tracked.txt"), "before\nafter\n")
            .expect("tracked update should be written");
        fs::write(temp.path().join("new.txt"), "hello\n")
            .expect("untracked file should be written");
        run_git(temp.path(), &["add", "--all"]);

        unstage_all(temp.path()).expect("all files should unstage");

        let files = list_changed_files(temp.path()).expect("changed files should load");
        assert_eq!(files.len(), 2);
        let tracked = files
            .iter()
            .find(|file| file.path == "tracked.txt")
            .expect("tracked file should remain");
        let untracked = files
            .iter()
            .find(|file| file.path == "new.txt")
            .expect("new file should remain");

        assert!(!tracked.has_staged_changes);
        assert!(tracked.has_unstaged_changes);
        assert!(!untracked.has_staged_changes);
        assert!(untracked.has_unstaged_changes);
        assert_eq!(untracked.status, FileStatus::Untracked);
    }

    #[test]
    fn stage_file_stages_remaining_changes_for_partially_staged_file() {
        let temp = init_repo();
        fs::write(temp.path().join("tracked.txt"), "one\ntwo\n")
            .expect("fixture should be written");
        run_git(temp.path(), &["add", "tracked.txt"]);
        run_git_with_identity(temp.path(), &["commit", "-qm", "init"]);

        fs::write(temp.path().join("tracked.txt"), "ONE\ntwo\nthree\n")
            .expect("updated content should be written");
        run_git(temp.path(), &["add", "tracked.txt"]);
        fs::write(temp.path().join("tracked.txt"), "ONE\nTWO\nthree\n")
            .expect("partial unstaged edit should be written");

        let before = list_changed_files(temp.path()).expect("changed files should load");
        assert_eq!(before.len(), 1);
        assert!(before[0].has_staged_changes);
        assert!(before[0].has_unstaged_changes);

        stage_file(temp.path(), "tracked.txt").expect("remaining changes should stage");

        let after = list_changed_files(temp.path()).expect("changed files should load");
        assert_eq!(after.len(), 1);
        assert!(after[0].has_staged_changes);
        assert!(!after[0].has_unstaged_changes);
    }

    #[test]
    fn snapshot_changes_when_file_content_changes_without_stat_delta() {
        let temp = init_repo();
        fs::write(temp.path().join("tracked.txt"), "before\nsame\n")
            .expect("fixture should be written");
        run_git(temp.path(), &["add", "tracked.txt"]);
        run_git_with_identity(temp.path(), &["commit", "-qm", "init"]);

        fs::write(temp.path().join("tracked.txt"), "alpha\nsame\n")
            .expect("first edit should be written");
        let first = load_snapshot(temp.path()).expect("snapshot should load");

        thread::sleep(Duration::from_millis(5));

        fs::write(temp.path().join("tracked.txt"), "bravo\nsame\n")
            .expect("second edit should be written");
        let second = load_snapshot(temp.path()).expect("snapshot should load");

        assert_ne!(first, second);
        assert_eq!(first.files[0].additions, second.files[0].additions);
        assert_eq!(first.files[0].deletions, second.files[0].deletions);
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
