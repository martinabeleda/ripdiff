mod app;
mod diff;
mod event;
mod git;
mod ui;

use anyhow::Result;
use app::App;
use clap::Parser;
use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use diff::DiffService;
use git::git_dir;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio::sync::watch;

#[derive(Parser, Debug)]
#[command(name = "ripdiff", about = "Terminal UI for navigating git diffs")]
struct Args {
    /// Path to the git repository (defaults to current directory)
    #[arg(value_name = "PATH")]
    path: Option<PathBuf>,

    /// Show only unstaged changes
    #[arg(short = 'u', long)]
    unstaged_only: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let start_path = args
        .path
        .unwrap_or_else(|| std::env::current_dir().unwrap());

    // Build App before touching the terminal so errors are readable
    let app = App::new(start_path.clone(), args.unstaged_only)?;

    // Watch git index and commit message files for auto-refresh
    let repo_git_dir = git_dir(&app.repo_root)?;
    let watch_paths = vec![
        repo_git_dir.join("index"),
        repo_git_dir.join("COMMIT_EDITMSG"),
    ];

    let (tx, rx) = mpsc::unbounded_channel();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let diff_service = DiffService::new(app.repo_root.clone(), tx.clone());

    let event_task = event::spawn_event_producer(tx.clone(), shutdown_rx.clone());
    let tick_task = event::spawn_tick_producer(tx.clone(), shutdown_rx.clone());
    let watcher_task = event::spawn_watcher(tx, watch_paths, shutdown_rx);

    // Terminal setup
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Panic hook: restore terminal before printing the panic
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(info);
    }));

    let result = app::run(&mut terminal, app, rx, diff_service).await;

    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(std::time::Duration::from_millis(300), event_task).await;
    let _ = tokio::time::timeout(std::time::Duration::from_millis(300), tick_task).await;
    let _ = tokio::time::timeout(std::time::Duration::from_millis(300), watcher_task).await;

    // Cleanup
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    match result {
        Ok(()) => std::process::exit(0),
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Args;
    use clap::Parser;
    use std::path::PathBuf;

    #[test]
    fn parses_positional_path_argument() {
        let args = Args::parse_from(["ripdiff", "/tmp/repo"]);

        assert_eq!(args.path, Some(PathBuf::from("/tmp/repo")));
    }

    #[test]
    fn defaults_path_when_not_provided() {
        let args = Args::parse_from(["ripdiff"]);

        assert_eq!(args.path, None);
        assert!(!args.unstaged_only);
    }

    #[test]
    fn parses_unstaged_only_flag() {
        let args = Args::parse_from(["ripdiff", "--unstaged-only"]);

        assert!(args.unstaged_only);
    }

    #[test]
    fn parses_unstaged_only_short_flag() {
        let args = Args::parse_from(["ripdiff", "-u"]);

        assert!(args.unstaged_only);
    }

    #[test]
    fn rejects_legacy_named_path_argument() {
        let result = Args::try_parse_from(["ripdiff", "--path", "/tmp/repo"]);

        assert!(result.is_err());
    }
}
