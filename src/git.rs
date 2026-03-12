use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone)]
pub struct FileStat {
    pub path: String,
    pub additions: u32,
    pub deletions: u32,
    pub status: FileStatus,
}

pub fn repo_root(start: &Path) -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["-C", start.to_str().unwrap_or("."), "rev-parse", "--show-toplevel"])
        .output()
        .context("Failed to run git rev-parse")?;

    if !output.status.success() {
        anyhow::bail!("Not inside a git repository");
    }

    let path = String::from_utf8(output.stdout)
        .context("git output not UTF-8")?
        .trim()
        .to_string();

    Ok(PathBuf::from(path))
}

pub fn list_changed_files(repo_root: &Path) -> Result<Vec<FileStat>> {
    // Get numstat for additions/deletions
    let numstat = Command::new("git")
        .args(["-C", repo_root.to_str().unwrap_or("."), "diff", "HEAD", "--numstat"])
        .output()
        .context("Failed to run git diff --numstat")?;

    // Also get unstaged changes not in HEAD (new untracked won't show, but staged+unstaged will)
    let name_status = Command::new("git")
        .args(["-C", repo_root.to_str().unwrap_or("."), "diff", "HEAD", "--name-status"])
        .output()
        .context("Failed to run git diff --name-status")?;

    let numstat_str = String::from_utf8_lossy(&numstat.stdout);
    let name_status_str = String::from_utf8_lossy(&name_status.stdout);

    // Build status map
    let mut status_map: std::collections::HashMap<String, FileStatus> =
        std::collections::HashMap::new();
    for line in name_status_str.lines() {
        let parts: Vec<&str> = line.splitn(2, '\t').collect();
        if parts.len() < 2 {
            continue;
        }
        let code = parts[0].trim();
        let path = parts[1].trim().to_string();
        let status = match code.chars().next() {
            Some('M') => FileStatus::Modified,
            Some('A') => FileStatus::Added,
            Some('D') => FileStatus::Deleted,
            Some('R') => FileStatus::Renamed,
            _ => FileStatus::Unknown,
        };
        status_map.insert(path, status);
    }

    let mut files = Vec::new();

    for line in numstat_str.lines() {
        let parts: Vec<&str> = line.splitn(3, '\t').collect();
        if parts.len() < 3 {
            continue;
        }
        // Binary files show "-" for additions/deletions
        let additions = parts[0].trim().parse::<u32>().unwrap_or(0);
        let deletions = parts[1].trim().parse::<u32>().unwrap_or(0);
        let path = parts[2].trim().to_string();
        let status = status_map
            .remove(&path)
            .unwrap_or(FileStatus::Unknown);

        files.push(FileStat {
            path,
            additions,
            deletions,
            status,
        });
    }

    // Also include untracked files
    let untracked = Command::new("git")
        .current_dir(repo_root)
        .args(["ls-files", "--others", "--exclude-standard"])
        .output()
        .context("Failed to run git ls-files")?;

    let untracked_str = String::from_utf8_lossy(&untracked.stdout);
    for line in untracked_str.lines() {
        let path = line.trim().to_string();
        if path.is_empty() {
            continue;
        }
        // Count lines in the file for additions stat
        let full_path = repo_root.join(&path);
        let additions = std::fs::read_to_string(&full_path)
            .map(|c| c.lines().count() as u32)
            .unwrap_or(0);

        files.push(FileStat {
            path,
            additions,
            deletions: 0,
            status: FileStatus::Untracked,
        });
    }

    Ok(files)
}
