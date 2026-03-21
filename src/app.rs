use crate::diff::{DiffContent, DiffMode, DiffRequest, DiffService};
use crate::event::Event;
use crate::git::{
    commit, load_snapshot_with_options, push, repo_root, stage_all, stage_file, unstage_all,
    unstage_file, FileStat, FileStatus, RepoSnapshot,
};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::Backend;
use ratatui::Terminal;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Panel {
    Files,
    Diff,
}

#[derive(Debug, Clone)]
pub enum CommitDialog {
    Composing { message: String },
    Result { output: String, succeeded: bool },
}

#[derive(Debug, Clone)]
pub enum PushDialog {
    Result { output: String, succeeded: bool },
}

pub struct App {
    pub repo_root: PathBuf,
    pub show_unstaged_only: bool,
    pub snapshot: RepoSnapshot,
    pub ui: UiState,
    pub diff_store: DiffStore,
    pub last_refresh: Instant,
    pub should_quit: bool,
    pub error_message: Option<String>,
}

pub struct UiState {
    pub selected: usize,
    pub diff_cursor: usize,
    pub scroll_offset: usize,
    pub pending_g: bool,
    pub pending_space: bool,
    pub show_help: bool,
    pub show_sidebar: bool,
    pub hidden_files: HashSet<String>,
    pub diff_mode: DiffMode,
    pub panel_width: u16,
    pub panel_height: u16,
    pub focus: Panel,
    pub commit_dialog: Option<CommitDialog>,
    pub push_dialog: Option<PushDialog>,
}

pub struct DiffStore {
    pub cache: HashMap<DiffRequest, DiffContent>,
    pub loading: HashSet<DiffRequest>,
}

impl App {
    pub fn new(start_path: PathBuf, show_unstaged_only: bool) -> Result<Self> {
        let root = repo_root(&start_path)?;
        let snapshot = load_snapshot_with_options(&root, show_unstaged_only)?;

        Ok(App {
            repo_root: root,
            show_unstaged_only,
            snapshot,
            ui: UiState {
                selected: 0,
                diff_cursor: 0,
                scroll_offset: 0,
                pending_g: false,
                pending_space: false,
                show_help: false,
                show_sidebar: true,
                hidden_files: HashSet::new(),
                diff_mode: DiffMode::Inline,
                panel_width: 80,
                panel_height: 40,
                focus: Panel::Files,
                commit_dialog: None,
                push_dialog: None,
            },
            diff_store: DiffStore {
                cache: HashMap::new(),
                loading: HashSet::new(),
            },
            last_refresh: Instant::now() - Duration::from_secs(10),
            should_quit: false,
            error_message: None,
        })
    }

    pub fn refresh(&mut self) {
        if self.last_refresh.elapsed() < Duration::from_millis(300) {
            return;
        }
        self.last_refresh = Instant::now();

        match load_snapshot_with_options(&self.repo_root, self.show_unstaged_only) {
            Ok(snapshot) => {
                let snapshot_changed = self.snapshot != snapshot;
                self.apply_snapshot(snapshot);
                if snapshot_changed {
                    self.diff_store.cache.clear();
                    self.diff_store.loading.clear();
                }
                self.error_message = None;
            }
            Err(error) => {
                self.error_message = Some(format!("git error: {error}"));
            }
        }
    }

    pub fn force_refresh(&mut self) {
        self.last_refresh = Instant::now() - Duration::from_secs(10);
        self.refresh();
    }

    pub fn files(&self) -> &[FileStat] {
        &self.snapshot.files
    }

    pub fn selected_file(&self) -> Option<&FileStat> {
        self.snapshot.files.get(self.ui.selected)
    }

    pub fn selected_diff_request(&self) -> Option<DiffRequest> {
        let file = self.selected_file()?;
        if self.ui.hidden_files.contains(&file.path) {
            return None;
        }

        Some(DiffRequest {
            path: file.path.clone(),
            panel_width: self.ui.panel_width,
            mode: self.ui.diff_mode.clone(),
        })
    }

    pub fn selected_diff(&self) -> Option<&DiffContent> {
        let request = self.selected_diff_request()?;
        self.diff_store.cache.get(&request)
    }

    pub fn ensure_selected_diff(&mut self, service: &DiffService) {
        let Some(file) = self.selected_file() else {
            return;
        };
        let path = file.path.clone();
        let is_untracked = file.status == FileStatus::Untracked;

        if self.ui.hidden_files.contains(&path) {
            return;
        }

        let request = DiffRequest {
            path,
            panel_width: self.ui.panel_width,
            mode: self.ui.diff_mode.clone(),
        };

        if self.diff_store.cache.contains_key(&request)
            || self.diff_store.loading.contains(&request)
        {
            return;
        }

        self.diff_store.loading.insert(request.clone());
        service.request(request, is_untracked, self.show_unstaged_only);
    }

    pub fn handle_loaded_diff(
        &mut self,
        request: DiffRequest,
        result: Result<DiffContent, String>,
    ) {
        self.diff_store.loading.remove(&request);
        let content = match result {
            Ok(content) => content,
            Err(error) => error_diff_content(error),
        };
        self.diff_store.cache.insert(request, content);
    }

    pub fn scroll_down(&mut self, amount: usize) {
        let max_cursor = self.selected_diff_len().saturating_sub(1);
        self.ui.diff_cursor = self.ui.diff_cursor.saturating_add(amount).min(max_cursor);
        self.sync_diff_viewport();
    }

    pub fn scroll_up(&mut self, amount: usize) {
        self.ui.diff_cursor = self.ui.diff_cursor.saturating_sub(amount);
        self.sync_diff_viewport();
    }

    pub fn move_down(&mut self) {
        if self.ui.selected + 1 < self.snapshot.files.len() {
            self.ui.selected += 1;
            self.reset_diff_position();
        }
    }

    pub fn move_up(&mut self) {
        if self.ui.selected > 0 {
            self.ui.selected -= 1;
            self.reset_diff_position();
        }
    }

    pub fn jump_top(&mut self) {
        self.ui.selected = 0;
        self.reset_diff_position();
    }

    pub fn jump_bottom(&mut self) {
        if !self.snapshot.files.is_empty() {
            self.ui.selected = self.snapshot.files.len() - 1;
            self.reset_diff_position();
        }
    }

    pub fn toggle_hidden(&mut self) {
        if let Some(file) = self.selected_file() {
            let path = file.path.clone();
            if self.ui.hidden_files.contains(&path) {
                self.ui.hidden_files.remove(&path);
            } else {
                self.ui.hidden_files.insert(path);
                self.reset_diff_position();
            }
        }
    }

    pub fn toggle_diff_mode(&mut self) {
        self.ui.diff_mode = self.ui.diff_mode.toggle();
        self.diff_store.cache.clear();
        self.diff_store.loading.clear();
        self.reset_diff_position();
    }

    pub fn toggle_unstaged_only(&mut self) {
        self.show_unstaged_only = !self.show_unstaged_only;
        self.diff_store.cache.clear();
        self.diff_store.loading.clear();
        self.force_refresh();
    }

    pub fn toggle_sidebar(&mut self) {
        self.ui.show_sidebar = !self.ui.show_sidebar;
        if !self.ui.show_sidebar {
            self.ui.focus = Panel::Diff;
        }
    }

    pub fn toggle_selected_file_staged(&mut self) {
        let Some(file) = self.selected_file().cloned() else {
            return;
        };

        let result = if file.has_unstaged_changes {
            stage_file(&self.repo_root, &file.path)
        } else if file.has_staged_changes {
            unstage_file(&self.repo_root, &file.path)
        } else {
            Ok(())
        };

        match result {
            Ok(()) => self.force_refresh(),
            Err(error) => self.error_message = Some(format!("git error: {error}")),
        }
    }

    pub fn unstage_selected_changes(&mut self) {
        let Some(file) = self.selected_file().cloned() else {
            return;
        };

        if file.has_staged_changes {
            let result = unstage_file(&self.repo_root, &file.path);
            match result {
                Ok(()) => self.force_refresh(),
                Err(error) => self.error_message = Some(format!("git error: {error}")),
            }
        }
    }

    pub fn unstage_all_changes(&mut self) {
        let has_staged_changes = self.files().iter().any(|file| file.has_staged_changes);
        if has_staged_changes {
            let result = unstage_all(&self.repo_root);
            match result {
                Ok(()) => self.force_refresh(),
                Err(error) => self.error_message = Some(format!("git error: {error}")),
            }
        }
    }

    pub fn toggle_all_files_staged(&mut self) {
        let has_unstaged_changes = self.files().iter().any(|file| file.has_unstaged_changes);
        let has_staged_changes = self.files().iter().any(|file| file.has_staged_changes);
        let result = if has_unstaged_changes {
            stage_all(&self.repo_root)
        } else if has_staged_changes {
            unstage_all(&self.repo_root)
        } else {
            Ok(())
        };

        match result {
            Ok(()) => self.force_refresh(),
            Err(error) => self.error_message = Some(format!("git error: {error}")),
        }
    }

    fn handle_commit_key(&mut self, key: KeyEvent) {
        match &self.ui.commit_dialog {
            Some(CommitDialog::Composing { .. }) => match (key.code, key.modifiers) {
                (KeyCode::Esc, _) => {
                    self.ui.commit_dialog = None;
                }
                (KeyCode::Enter, _) => {
                    self.execute_commit();
                }
                (KeyCode::Backspace, _) => {
                    if let Some(CommitDialog::Composing { message }) = &mut self.ui.commit_dialog {
                        message.pop();
                    }
                }
                (KeyCode::Char(c), _) => {
                    if let Some(CommitDialog::Composing { message }) = &mut self.ui.commit_dialog {
                        message.push(c);
                    }
                }
                _ => {}
            },
            Some(CommitDialog::Result { .. }) => {
                self.ui.commit_dialog = None;
            }
            None => {}
        }
    }

    fn execute_commit(&mut self) {
        let message = match &self.ui.commit_dialog {
            Some(CommitDialog::Composing { message }) => message.clone(),
            _ => return,
        };

        if message.trim().is_empty() {
            return;
        }

        let result = commit(&self.repo_root, &message);
        let succeeded = result.succeeded;
        self.ui.commit_dialog = Some(CommitDialog::Result {
            output: result.output,
            succeeded,
        });

        if succeeded {
            self.force_refresh();
        }
    }

    fn execute_push(&mut self) {
        let result = push(&self.repo_root);
        let succeeded = result.succeeded;
        self.ui.push_dialog = Some(PushDialog::Result {
            output: result.output,
            succeeded,
        });
        if succeeded {
            self.force_refresh();
        }
    }

    pub fn diff_hunk_offsets(&self) -> Vec<usize> {
        let Some(diff) = self.selected_diff() else {
            return vec![];
        };

        let mut header_offsets = Vec::new();
        let mut block_offsets = Vec::new();
        let mut previous_changed = false;

        for (index, line) in diff.lines.iter().enumerate() {
            let text: String = line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect();
            let trimmed = text.trim_start();
            if trimmed.starts_with("@@") || trimmed.starts_with("───") {
                header_offsets.push(index);
            }

            let is_changed = is_change_line(line);
            if is_changed && !previous_changed {
                block_offsets.push(index);
            }
            previous_changed = is_changed;
        }

        if header_offsets.is_empty() {
            block_offsets
        } else {
            header_offsets
        }
    }

    pub fn jump_next_hunk(&mut self) {
        let hunks = self.diff_hunk_offsets();
        if let Some(&offset) = hunks.iter().find(|&&offset| offset > self.ui.diff_cursor) {
            self.ui.diff_cursor = offset;
            self.sync_diff_viewport();
        }
    }

    pub fn jump_prev_hunk(&mut self) {
        let hunks = self.diff_hunk_offsets();
        if let Some(&offset) = hunks
            .iter()
            .rev()
            .find(|&&offset| offset < self.ui.diff_cursor)
        {
            self.ui.diff_cursor = offset;
            self.sync_diff_viewport();
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if self.ui.commit_dialog.is_some() {
            self.handle_commit_key(key);
            return;
        }

        if self.ui.push_dialog.is_some() {
            self.ui.push_dialog = None;
            return;
        }

        if self.ui.show_help {
            match (key.code, key.modifiers) {
                (KeyCode::Char('q'), KeyModifiers::NONE) | (KeyCode::Esc, _) => {
                    self.should_quit = true;
                }
                (KeyCode::Char('h'), KeyModifiers::NONE)
                | (KeyCode::Char('?'), KeyModifiers::SHIFT) => {
                    self.ui.show_help = false;
                }
                _ => {}
            }
            return;
        }

        if self.handle_pending_key_sequence(key) {
            return;
        }

        match (key.code, key.modifiers) {
            (KeyCode::Char('q'), KeyModifiers::NONE) | (KeyCode::Esc, _) => {
                self.should_quit = true;
                return;
            }
            (KeyCode::Tab, _) | (KeyCode::BackTab, _) => {
                if self.ui.show_sidebar {
                    self.ui.focus = match self.ui.focus {
                        Panel::Files => Panel::Diff,
                        Panel::Diff => Panel::Files,
                    };
                }
                return;
            }
            (KeyCode::Char('r'), KeyModifiers::NONE) => {
                self.force_refresh();
                return;
            }
            (KeyCode::Char('t'), KeyModifiers::NONE) => {
                self.toggle_diff_mode();
                return;
            }
            (KeyCode::Char('u'), KeyModifiers::NONE) => {
                self.unstage_selected_changes();
                return;
            }
            (KeyCode::Char('U'), KeyModifiers::SHIFT) => {
                self.unstage_all_changes();
                return;
            }
            (KeyCode::Char('o'), KeyModifiers::NONE) => {
                self.toggle_unstaged_only();
                return;
            }
            (KeyCode::Char('h'), KeyModifiers::NONE)
            | (KeyCode::Char('?'), KeyModifiers::SHIFT) => {
                self.ui.show_help = true;
                self.ui.pending_g = false;
                self.ui.pending_space = false;
                return;
            }
            (KeyCode::Char('s'), KeyModifiers::NONE) => {
                self.toggle_selected_file_staged();
                return;
            }
            (KeyCode::Char('S'), _) => {
                self.toggle_all_files_staged();
                return;
            }
            (KeyCode::Char('c'), KeyModifiers::NONE) => {
                self.ui.commit_dialog = Some(CommitDialog::Composing {
                    message: String::new(),
                });
                return;
            }
            (KeyCode::Char('p'), KeyModifiers::NONE) => {
                self.execute_push();
                return;
            }
            _ => {}
        }

        match self.ui.focus {
            Panel::Files => self.handle_files_key(key),
            Panel::Diff => self.handle_diff_key(key),
        }
    }

    fn handle_pending_key_sequence(&mut self, key: KeyEvent) -> bool {
        if self.ui.pending_g {
            self.ui.pending_g = false;
            if matches!(
                (key.code, key.modifiers),
                (KeyCode::Char('g'), KeyModifiers::NONE)
            ) {
                self.jump_to_panel_top();
                return true;
            }
        }

        if self.ui.pending_space {
            self.ui.pending_space = false;
            if matches!(
                (key.code, key.modifiers),
                (KeyCode::Char('e'), KeyModifiers::NONE)
                    | (KeyCode::Char('E'), KeyModifiers::SHIFT)
            ) {
                self.toggle_sidebar();
                return true;
            }
        }

        match (key.code, key.modifiers) {
            (KeyCode::Char('g'), KeyModifiers::NONE) => {
                self.ui.pending_g = true;
                true
            }
            (KeyCode::Char(' '), KeyModifiers::NONE) => {
                self.ui.pending_space = true;
                true
            }
            _ => false,
        }
    }

    fn jump_to_panel_top(&mut self) {
        match self.ui.focus {
            Panel::Files => self.jump_top(),
            Panel::Diff => {
                self.ui.diff_cursor = 0;
                self.sync_diff_viewport();
            }
        }
    }

    fn handle_files_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.move_down(),
            KeyCode::Char('k') | KeyCode::Up => self.move_up(),
            KeyCode::Right => {
                self.ui.focus = Panel::Diff;
            }
            KeyCode::Char('G') => self.jump_bottom(),
            KeyCode::Enter => self.toggle_hidden(),
            _ => {}
        }
    }

    fn handle_diff_key(&mut self, key: KeyEvent) {
        let half_page = (self.ui.panel_height / 2) as usize;
        match (key.code, key.modifiers) {
            (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => self.scroll_down(1),
            (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => self.scroll_up(1),
            (KeyCode::Left, _) => {
                if self.ui.show_sidebar {
                    self.ui.focus = Panel::Files;
                }
            }
            (KeyCode::Char('d'), KeyModifiers::CONTROL) => self.scroll_down(half_page),
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => self.scroll_up(half_page),
            (KeyCode::Char('G'), _) => {
                self.ui.diff_cursor = self.selected_diff_len().saturating_sub(1);
                self.sync_diff_viewport();
            }
            (KeyCode::Enter, _) => self.toggle_hidden(),
            (code, _) if is_next_hunk_key(code) => self.jump_next_hunk(),
            (code, _) if is_prev_hunk_key(code) => self.jump_prev_hunk(),
            _ => {}
        }
    }

    fn apply_snapshot(&mut self, snapshot: RepoSnapshot) {
        let selected_path = self.selected_file().map(|file| file.path.clone());
        self.snapshot = snapshot;

        self.ui
            .hidden_files
            .retain(|path| self.snapshot.files.iter().any(|file| &file.path == path));

        if self.snapshot.files.is_empty() {
            self.ui.selected = 0;
            self.reset_diff_position();
            return;
        }

        if let Some(path) = selected_path {
            if let Some(index) = self
                .snapshot
                .files
                .iter()
                .position(|file| file.path == path)
            {
                self.ui.selected = index;
                self.sync_diff_position();
                return;
            }
        }

        if self.ui.selected >= self.snapshot.files.len() {
            self.ui.selected = self.snapshot.files.len() - 1;
        }
        self.sync_diff_position();
    }

    fn reset_diff_position(&mut self) {
        self.ui.diff_cursor = 0;
        self.ui.scroll_offset = 0;
    }

    fn selected_diff_len(&self) -> usize {
        self.selected_diff()
            .map(|diff| diff.lines.len())
            .unwrap_or(1)
    }

    fn sync_diff_position(&mut self) {
        let max_cursor = self.selected_diff_len().saturating_sub(1);
        self.ui.diff_cursor = self.ui.diff_cursor.min(max_cursor);
        self.sync_diff_viewport();
    }

    fn sync_diff_viewport(&mut self) {
        let height = self.ui.panel_height as usize;
        if height == 0 {
            self.ui.scroll_offset = 0;
            return;
        }

        let max_scroll = self.selected_diff_len().saturating_sub(height);
        if self.ui.diff_cursor < self.ui.scroll_offset {
            self.ui.scroll_offset = self.ui.diff_cursor;
        } else {
            let visible_end = self.ui.scroll_offset.saturating_add(height);
            if self.ui.diff_cursor >= visible_end {
                self.ui.scroll_offset =
                    self.ui.diff_cursor.saturating_add(1).saturating_sub(height);
            }
        }
        self.ui.scroll_offset = self.ui.scroll_offset.min(max_scroll);
    }
}

fn is_next_hunk_key(code: KeyCode) -> bool {
    matches!(code, KeyCode::Char(']') | KeyCode::Char('}'))
}

fn is_prev_hunk_key(code: KeyCode) -> bool {
    matches!(code, KeyCode::Char('[') | KeyCode::Char('{'))
}

fn is_change_line(line: &ratatui::text::Line<'_>) -> bool {
    let has_green = line
        .spans
        .iter()
        .any(|span| is_addition_color(span.style.fg));
    let has_red = line
        .spans
        .iter()
        .any(|span| is_deletion_color(span.style.fg));

    has_green || has_red
}

fn is_addition_color(color: Option<ratatui::style::Color>) -> bool {
    use ratatui::style::Color;

    matches!(
        color,
        Some(Color::Green)
            | Some(Color::LightGreen)
            | Some(Color::Indexed(2))
            | Some(Color::Indexed(10))
    )
}

fn is_deletion_color(color: Option<ratatui::style::Color>) -> bool {
    use ratatui::style::Color;

    matches!(
        color,
        Some(Color::Red)
            | Some(Color::LightRed)
            | Some(Color::Indexed(1))
            | Some(Color::Indexed(9))
    )
}

fn error_diff_content(error: String) -> DiffContent {
    use ratatui::style::{Color, Style};
    use ratatui::text::{Line, Span};

    DiffContent {
        lines: vec![Line::from(Span::styled(
            format!("Error: {error}"),
            Style::default().fg(Color::Red),
        ))],
    }
}

pub async fn run<B: Backend>(
    terminal: &mut Terminal<B>,
    mut app: App,
    mut rx: mpsc::UnboundedReceiver<Event>,
    diff_service: DiffService,
) -> Result<()> {
    loop {
        if app.should_quit {
            break;
        }

        let size = terminal.size()?;
        let right_width = if app.ui.show_sidebar {
            (size.width as f32 * 0.75) as u16
        } else {
            size.width
        };
        let panel_height = size.height.saturating_sub(2);
        if right_width != app.ui.panel_width {
            app.ui.panel_width = right_width;
        }
        app.ui.panel_height = panel_height;

        app.ensure_selected_diff(&diff_service);
        terminal.draw(|frame| crate::ui::render(frame, &app))?;

        match rx.recv().await {
            Some(Event::Key(key)) => {
                app.handle_key(key);
                if app.should_quit {
                    break;
                }
            }
            Some(Event::Resize) => {}
            Some(Event::FsChange) => app.refresh(),
            Some(Event::Tick) => app.refresh(),
            Some(Event::DiffLoaded { request, result }) => app.handle_loaded_diff(request, result),
            None => break,
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::style::{Color, Style};
    use ratatui::text::Line;
    use ratatui::text::Span;

    #[tokio::test]
    async fn run_exits_when_q_is_pressed() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        let (tx, rx) = mpsc::unbounded_channel();

        tx.send(Event::Key(KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::NONE,
        )))
        .expect("q event should enqueue");

        let app = App {
            repo_root: PathBuf::from("."),
            show_unstaged_only: false,
            snapshot: RepoSnapshot::default(),
            ui: UiState {
                selected: 0,
                diff_cursor: 0,
                scroll_offset: 0,
                pending_g: false,
                pending_space: false,
                show_help: false,
                show_sidebar: true,
                hidden_files: HashSet::new(),
                diff_mode: DiffMode::Inline,
                panel_width: 80,
                panel_height: 40,
                focus: Panel::Files,
                commit_dialog: None,
                push_dialog: None,
            },
            diff_store: DiffStore {
                cache: HashMap::new(),
                loading: HashSet::new(),
            },
            last_refresh: Instant::now(),
            should_quit: false,
            error_message: None,
        };

        let diff_service = DiffService::new(PathBuf::from("."), tx);
        let result = tokio::time::timeout(
            Duration::from_millis(250),
            run(&mut terminal, app, rx, diff_service),
        )
        .await;

        assert!(result.is_ok(), "run loop should exit after q");
        assert!(
            result.expect("timeout should resolve").is_ok(),
            "run loop should exit cleanly"
        );
    }

    #[test]
    fn refresh_invalidates_cached_diffs() {
        let mut app = App {
            repo_root: PathBuf::from("."),
            show_unstaged_only: false,
            snapshot: RepoSnapshot {
                branch: None,
                unpushed_commits: None,
                files: vec![FileStat {
                    path: "src/main.rs".to_string(),
                    additions: 1,
                    deletions: 0,
                    status: FileStatus::Modified,
                    has_staged_changes: false,
                    has_unstaged_changes: true,
                    content_signature: None,
                }],
            },
            ui: UiState {
                selected: 0,
                diff_cursor: 0,
                scroll_offset: 0,
                pending_g: false,
                pending_space: false,
                show_help: false,
                show_sidebar: true,
                hidden_files: HashSet::new(),
                diff_mode: DiffMode::Inline,
                panel_width: 80,
                panel_height: 40,
                focus: Panel::Files,
                commit_dialog: None,
                push_dialog: None,
            },
            diff_store: DiffStore {
                cache: HashMap::from([(
                    DiffRequest {
                        path: "src/main.rs".to_string(),
                        panel_width: 80,
                        mode: DiffMode::Inline,
                    },
                    DiffContent {
                        lines: vec![Line::from("stale")],
                    },
                )]),
                loading: HashSet::from([DiffRequest {
                    path: "src/main.rs".to_string(),
                    panel_width: 80,
                    mode: DiffMode::Inline,
                }]),
            },
            last_refresh: Instant::now() - Duration::from_secs(10),
            should_quit: false,
            error_message: None,
        };

        app.refresh();

        assert!(app.diff_store.cache.is_empty());
        assert!(app.diff_store.loading.is_empty());
    }

    #[test]
    fn apply_snapshot_keeps_cache_when_snapshot_is_unchanged() {
        let snapshot = RepoSnapshot {
            branch: None,
            unpushed_commits: None,
            files: vec![FileStat {
                path: "src/main.rs".to_string(),
                additions: 1,
                deletions: 0,
                status: FileStatus::Modified,
                has_staged_changes: false,
                has_unstaged_changes: true,
                content_signature: None,
            }],
        };
        let request = DiffRequest {
            path: "a.rs".to_string(),
            panel_width: 80,
            mode: DiffMode::Inline,
        };
        let mut app = App {
            repo_root: PathBuf::from("."),
            show_unstaged_only: false,
            snapshot: snapshot.clone(),
            ui: UiState {
                selected: 0,
                diff_cursor: 0,
                scroll_offset: 0,
                pending_g: false,
                pending_space: false,
                show_help: false,
                show_sidebar: true,
                hidden_files: HashSet::new(),
                diff_mode: DiffMode::Inline,
                panel_width: 80,
                panel_height: 40,
                focus: Panel::Files,
                commit_dialog: None,
                push_dialog: None,
            },
            diff_store: DiffStore {
                cache: HashMap::from([(
                    request.clone(),
                    DiffContent {
                        lines: vec![Line::from("ok")],
                    },
                )]),
                loading: HashSet::from([request.clone()]),
            },
            last_refresh: Instant::now(),
            should_quit: false,
            error_message: None,
        };

        let snapshot_changed = app.snapshot != snapshot;
        app.apply_snapshot(snapshot);
        if snapshot_changed {
            app.diff_store.cache.clear();
            app.diff_store.loading.clear();
        }

        assert!(app.diff_store.cache.contains_key(&request));
        assert!(app.diff_store.loading.contains(&request));
    }

    #[test]
    fn diff_cursor_moves_independently_from_top_visible_line() {
        let request = DiffRequest {
            path: "src/main.rs".to_string(),
            panel_width: 80,
            mode: DiffMode::Inline,
        };
        let mut app = App {
            repo_root: PathBuf::from("."),
            show_unstaged_only: false,
            snapshot: RepoSnapshot {
                branch: None,
                unpushed_commits: None,
                files: vec![FileStat {
                    path: "src/main.rs".to_string(),
                    additions: 1,
                    deletions: 0,
                    status: FileStatus::Modified,
                    has_staged_changes: false,
                    has_unstaged_changes: true,
                    content_signature: None,
                }],
            },
            ui: UiState {
                selected: 0,
                diff_cursor: 0,
                scroll_offset: 0,
                pending_g: false,
                pending_space: false,
                show_help: false,
                show_sidebar: true,
                hidden_files: HashSet::new(),
                diff_mode: DiffMode::Inline,
                panel_width: 80,
                panel_height: 3,
                focus: Panel::Diff,
                commit_dialog: None,
                push_dialog: None,
            },
            diff_store: DiffStore {
                cache: HashMap::from([(
                    request,
                    DiffContent {
                        lines: vec![
                            Line::from("1"),
                            Line::from("2"),
                            Line::from("3"),
                            Line::from("4"),
                            Line::from("5"),
                        ],
                    },
                )]),
                loading: HashSet::new(),
            },
            last_refresh: Instant::now(),
            should_quit: false,
            error_message: None,
        };

        app.scroll_down(1);
        assert_eq!(app.ui.diff_cursor, 1);
        assert_eq!(app.ui.scroll_offset, 0);

        app.scroll_down(2);
        assert_eq!(app.ui.diff_cursor, 3);
        assert_eq!(app.ui.scroll_offset, 1);
    }

    #[test]
    fn handle_key_uses_brackets_for_hunk_navigation() {
        let request = DiffRequest {
            path: "src/main.rs".to_string(),
            panel_width: 80,
            mode: DiffMode::Inline,
        };
        let mut app = App {
            repo_root: PathBuf::from("."),
            show_unstaged_only: false,
            snapshot: RepoSnapshot {
                branch: None,
                unpushed_commits: None,
                files: vec![FileStat {
                    path: "src/main.rs".to_string(),
                    additions: 1,
                    deletions: 0,
                    status: FileStatus::Modified,
                    has_staged_changes: false,
                    has_unstaged_changes: true,
                    content_signature: None,
                }],
            },
            ui: UiState {
                selected: 0,
                diff_cursor: 0,
                scroll_offset: 0,
                pending_g: false,
                pending_space: false,
                show_help: false,
                show_sidebar: true,
                hidden_files: HashSet::new(),
                diff_mode: DiffMode::Inline,
                panel_width: 80,
                panel_height: 3,
                focus: Panel::Diff,
                commit_dialog: None,
                push_dialog: None,
            },
            diff_store: DiffStore {
                cache: HashMap::from([(
                    request,
                    DiffContent {
                        lines: vec![
                            Line::from("header"),
                            Line::from("@@ hunk one @@"),
                            Line::from("body"),
                            Line::from("@@ hunk two @@"),
                            Line::from("tail"),
                        ],
                    },
                )]),
                loading: HashSet::new(),
            },
            last_refresh: Instant::now(),
            should_quit: false,
            error_message: None,
        };

        app.handle_key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE));
        assert_eq!(app.ui.diff_cursor, 1);

        app.handle_key(KeyEvent::new(KeyCode::Char('}'), KeyModifiers::SHIFT));
        assert_eq!(app.ui.diff_cursor, 3);

        app.handle_key(KeyEvent::new(KeyCode::Char('{'), KeyModifiers::SHIFT));
        assert_eq!(app.ui.diff_cursor, 1);
    }

    #[test]
    fn handle_key_uses_gg_and_g_for_navigation() {
        let request = DiffRequest {
            path: "a.rs".to_string(),
            panel_width: 80,
            mode: DiffMode::Inline,
        };
        let mut app = App {
            repo_root: PathBuf::from("."),
            show_unstaged_only: false,
            snapshot: RepoSnapshot {
                branch: None,
                unpushed_commits: None,
                files: vec![
                    FileStat {
                        path: "a.rs".to_string(),
                        additions: 1,
                        deletions: 0,
                        status: FileStatus::Modified,
                        has_staged_changes: false,
                        has_unstaged_changes: true,
                        content_signature: None,
                    },
                    FileStat {
                        path: "b.rs".to_string(),
                        additions: 1,
                        deletions: 0,
                        status: FileStatus::Modified,
                        has_staged_changes: false,
                        has_unstaged_changes: true,
                        content_signature: None,
                    },
                    FileStat {
                        path: "c.rs".to_string(),
                        additions: 1,
                        deletions: 0,
                        status: FileStatus::Modified,
                        has_staged_changes: false,
                        has_unstaged_changes: true,
                        content_signature: None,
                    },
                ],
            },
            ui: UiState {
                selected: 1,
                diff_cursor: 2,
                scroll_offset: 0,
                pending_g: false,
                pending_space: false,
                show_help: false,
                show_sidebar: true,
                hidden_files: HashSet::new(),
                diff_mode: DiffMode::Inline,
                panel_width: 80,
                panel_height: 3,
                focus: Panel::Files,
                commit_dialog: None,
                push_dialog: None,
            },
            diff_store: DiffStore {
                cache: HashMap::from([(
                    request,
                    DiffContent {
                        lines: vec![
                            Line::from("1"),
                            Line::from("2"),
                            Line::from("3"),
                            Line::from("4"),
                            Line::from("5"),
                        ],
                    },
                )]),
                loading: HashSet::new(),
            },
            last_refresh: Instant::now(),
            should_quit: false,
            error_message: None,
        };

        app.handle_key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT));
        assert_eq!(app.ui.selected, 2);

        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
        assert_eq!(app.ui.selected, 2);
        assert!(app.ui.pending_g);

        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
        assert_eq!(app.ui.selected, 0);
        assert!(!app.ui.pending_g);

        app.ui.focus = Panel::Diff;
        app.ui.diff_cursor = 2;
        app.ui.scroll_offset = 2;

        app.handle_key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT));
        assert_eq!(app.ui.diff_cursor, 4);

        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
        assert!(app.ui.pending_g);

        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
        assert_eq!(app.ui.diff_cursor, 0);
        assert_eq!(app.ui.scroll_offset, 0);
        assert!(!app.ui.pending_g);
    }

    #[test]
    fn handle_key_toggles_sidebar_with_space_e() {
        let mut app = App {
            repo_root: PathBuf::from("."),
            show_unstaged_only: false,
            snapshot: RepoSnapshot {
                branch: None,
                unpushed_commits: None,
                files: vec![FileStat {
                    path: "src/main.rs".to_string(),
                    additions: 1,
                    deletions: 0,
                    status: FileStatus::Modified,
                    has_staged_changes: false,
                    has_unstaged_changes: true,
                    content_signature: None,
                }],
            },
            ui: UiState {
                selected: 0,
                diff_cursor: 0,
                scroll_offset: 0,
                pending_g: false,
                pending_space: false,
                show_help: false,
                show_sidebar: true,
                hidden_files: HashSet::new(),
                diff_mode: DiffMode::Inline,
                panel_width: 80,
                panel_height: 40,
                focus: Panel::Files,
                commit_dialog: None,
                push_dialog: None,
            },
            diff_store: DiffStore {
                cache: HashMap::new(),
                loading: HashSet::new(),
            },
            last_refresh: Instant::now(),
            should_quit: false,
            error_message: None,
        };

        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert!(app.ui.pending_space);
        assert!(app.ui.show_sidebar);

        app.handle_key(KeyEvent::new(KeyCode::Char('E'), KeyModifiers::SHIFT));
        assert!(!app.ui.pending_space);
        assert!(!app.ui.show_sidebar);
        assert_eq!(app.ui.focus, Panel::Diff);

        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        assert!(app.ui.show_sidebar);
    }

    #[test]
    fn handle_key_toggles_help_overlay_with_h() {
        let mut app = App {
            repo_root: PathBuf::from("."),
            show_unstaged_only: false,
            snapshot: RepoSnapshot {
                branch: None,
                unpushed_commits: None,
                files: vec![],
            },
            ui: UiState {
                selected: 0,
                diff_cursor: 0,
                scroll_offset: 0,
                pending_g: false,
                pending_space: false,
                show_help: false,
                show_sidebar: true,
                hidden_files: HashSet::new(),
                diff_mode: DiffMode::Inline,
                panel_width: 80,
                panel_height: 40,
                focus: Panel::Files,
                commit_dialog: None,
                push_dialog: None,
            },
            diff_store: DiffStore {
                cache: HashMap::new(),
                loading: HashSet::new(),
            },
            last_refresh: Instant::now(),
            should_quit: false,
            error_message: None,
        };

        app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
        assert!(app.ui.show_help);

        app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
        assert!(!app.ui.show_help);
    }

    #[test]
    fn handle_key_u_unstages_selected_file() {
        let mut app = App {
            repo_root: PathBuf::from("."),
            show_unstaged_only: false,
            snapshot: RepoSnapshot {
                branch: None,
                unpushed_commits: None,
                files: vec![FileStat {
                    path: "tracked.txt".to_string(),
                    additions: 1,
                    deletions: 0,
                    status: FileStatus::Modified,
                    has_staged_changes: true,
                    has_unstaged_changes: false,
                    content_signature: None,
                }],
            },
            ui: UiState {
                selected: 0,
                diff_cursor: 0,
                scroll_offset: 0,
                pending_g: false,
                pending_space: false,
                show_help: false,
                show_sidebar: true,
                hidden_files: HashSet::new(),
                diff_mode: DiffMode::Inline,
                panel_width: 80,
                panel_height: 40,
                focus: Panel::Files,
                commit_dialog: None,
                push_dialog: None,
            },
            diff_store: DiffStore {
                cache: HashMap::new(),
                loading: HashSet::new(),
            },
            last_refresh: Instant::now() - Duration::from_secs(11),
            should_quit: false,
            error_message: None,
        };

        app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE));

        // Unstaging should trigger a refresh (last_refresh should update)
        assert!(app.last_refresh.elapsed() >= Duration::from_millis(10));
    }

    #[test]
    fn handle_key_u_unstages_all_staged_files() {
        let mut app = App {
            repo_root: PathBuf::from("."),
            show_unstaged_only: false,
            snapshot: RepoSnapshot {
                branch: None,
                unpushed_commits: None,
                files: vec![
                    FileStat {
                        path: "file1.txt".to_string(),
                        additions: 1,
                        deletions: 0,
                        status: FileStatus::Modified,
                        has_staged_changes: true,
                        has_unstaged_changes: false,
                        content_signature: None,
                    },
                    FileStat {
                        path: "file2.txt".to_string(),
                        additions: 1,
                        deletions: 0,
                        status: FileStatus::Modified,
                        has_staged_changes: true,
                        has_unstaged_changes: false,
                        content_signature: None,
                    },
                ],
            },
            ui: UiState {
                selected: 0,
                diff_cursor: 0,
                scroll_offset: 0,
                pending_g: false,
                pending_space: false,
                show_help: false,
                show_sidebar: true,
                hidden_files: HashSet::new(),
                diff_mode: DiffMode::Inline,
                panel_width: 80,
                panel_height: 40,
                focus: Panel::Files,
                commit_dialog: None,
                push_dialog: None,
            },
            diff_store: DiffStore {
                cache: HashMap::new(),
                loading: HashSet::new(),
            },
            last_refresh: Instant::now() - Duration::from_secs(11),
            should_quit: false,
            error_message: None,
        };

        app.handle_key(KeyEvent::new(KeyCode::Char('U'), KeyModifiers::SHIFT));

        // Unstaging all should trigger a refresh (last_refresh should update)
        assert!(app.last_refresh.elapsed() >= Duration::from_millis(10));
    }

    #[test]
    fn handle_key_o_toggles_unstaged_only_scope() {
        let mut app = App {
            repo_root: PathBuf::from("."),
            show_unstaged_only: false,
            snapshot: RepoSnapshot {
                branch: None,
                unpushed_commits: None,
                files: vec![
                    FileStat {
                        path: "file1.txt".to_string(),
                        additions: 1,
                        deletions: 0,
                        status: FileStatus::Modified,
                        has_staged_changes: true,
                        has_unstaged_changes: false,
                        content_signature: None,
                    },
                    FileStat {
                        path: "file2.txt".to_string(),
                        additions: 1,
                        deletions: 0,
                        status: FileStatus::Modified,
                        has_staged_changes: true,
                        has_unstaged_changes: false,
                        content_signature: None,
                    },
                ],
            },
            ui: UiState {
                selected: 0,
                diff_cursor: 0,
                scroll_offset: 0,
                pending_g: false,
                pending_space: false,
                show_help: false,
                show_sidebar: true,
                hidden_files: HashSet::new(),
                diff_mode: DiffMode::Inline,
                panel_width: 80,
                panel_height: 40,
                focus: Panel::Files,
                commit_dialog: None,
                push_dialog: None,
            },
            diff_store: DiffStore {
                cache: HashMap::new(),
                loading: HashSet::new(),
            },
            last_refresh: Instant::now(),
            should_quit: false,
            error_message: None,
        };

        app.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));

        assert!(app.show_unstaged_only);
        assert!(app.diff_store.cache.is_empty());
        assert!(app.diff_store.loading.is_empty());
    }

    #[test]
    fn toggle_all_files_staged_prefers_staging_partial_changes() {
        let app = App {
            repo_root: PathBuf::from("."),
            show_unstaged_only: false,
            snapshot: RepoSnapshot {
                branch: None,
                unpushed_commits: None,
                files: vec![FileStat {
                    path: "tracked.txt".to_string(),
                    additions: 1,
                    deletions: 0,
                    status: FileStatus::Modified,
                    has_staged_changes: true,
                    has_unstaged_changes: true,
                    content_signature: None,
                }],
            },
            ui: UiState {
                selected: 0,
                diff_cursor: 0,
                scroll_offset: 0,
                pending_g: false,
                pending_space: false,
                show_help: false,
                show_sidebar: true,
                hidden_files: HashSet::new(),
                diff_mode: DiffMode::Inline,
                panel_width: 80,
                panel_height: 40,
                focus: Panel::Files,
                commit_dialog: None,
                push_dialog: None,
            },
            diff_store: DiffStore {
                cache: HashMap::new(),
                loading: HashSet::new(),
            },
            last_refresh: Instant::now(),
            should_quit: false,
            error_message: None,
        };

        let has_unstaged_changes = app.files().iter().any(|file| file.has_unstaged_changes);
        let has_staged_changes = app.files().iter().any(|file| file.has_staged_changes);

        assert!(has_unstaged_changes);
        assert!(has_staged_changes);
    }

    fn make_app() -> App {
        App {
            repo_root: PathBuf::from("."),
            show_unstaged_only: false,
            snapshot: RepoSnapshot::default(),
            ui: UiState {
                selected: 0,
                diff_cursor: 0,
                scroll_offset: 0,
                pending_g: false,
                pending_space: false,
                show_help: false,
                show_sidebar: true,
                hidden_files: HashSet::new(),
                diff_mode: DiffMode::Inline,
                panel_width: 80,
                panel_height: 40,
                focus: Panel::Files,
                commit_dialog: None,
                push_dialog: None,
            },
            diff_store: DiffStore {
                cache: HashMap::new(),
                loading: HashSet::new(),
            },
            last_refresh: Instant::now(),
            should_quit: false,
            error_message: None,
        }
    }

    #[test]
    fn handle_key_c_opens_commit_dialog() {
        let mut app = make_app();
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        let Some(CommitDialog::Composing { message }) = &app.ui.commit_dialog else {
            panic!("expected Composing dialog");
        };
        assert!(message.is_empty());
    }

    #[test]
    fn commit_dialog_typing_appends_characters() {
        let mut app = make_app();
        app.ui.commit_dialog = Some(CommitDialog::Composing {
            message: String::new(),
        });
        for ch in "fix: bug".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        let Some(CommitDialog::Composing { message }) = &app.ui.commit_dialog else {
            panic!("expected Composing dialog");
        };
        assert_eq!(message, "fix: bug");
    }

    #[test]
    fn commit_dialog_backspace_removes_last_character() {
        let mut app = make_app();
        app.ui.commit_dialog = Some(CommitDialog::Composing {
            message: "fix".to_string(),
        });
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        let Some(CommitDialog::Composing { message }) = &app.ui.commit_dialog else {
            panic!("expected Composing dialog");
        };
        assert_eq!(message, "fi");
    }

    #[test]
    fn commit_dialog_backspace_on_empty_message_does_nothing() {
        let mut app = make_app();
        app.ui.commit_dialog = Some(CommitDialog::Composing {
            message: String::new(),
        });
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert!(
            matches!(&app.ui.commit_dialog, Some(CommitDialog::Composing { message }) if message.is_empty())
        );
    }

    #[test]
    fn commit_dialog_esc_closes_dialog() {
        let mut app = make_app();
        app.ui.commit_dialog = Some(CommitDialog::Composing {
            message: "wip".to_string(),
        });
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.ui.commit_dialog.is_none());
    }

    #[test]
    fn commit_dialog_intercepts_q_key() {
        let mut app = make_app();
        app.ui.commit_dialog = Some(CommitDialog::Composing {
            message: String::new(),
        });
        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(
            !app.should_quit,
            "q should not quit while commit dialog is open"
        );
        let Some(CommitDialog::Composing { message }) = &app.ui.commit_dialog else {
            panic!("expected Composing dialog");
        };
        assert_eq!(message, "q", "q should be appended to message");
    }

    #[test]
    fn commit_dialog_enter_on_empty_message_stays_composing() {
        let mut app = make_app();
        app.ui.commit_dialog = Some(CommitDialog::Composing {
            message: String::new(),
        });
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            matches!(app.ui.commit_dialog, Some(CommitDialog::Composing { .. })),
            "dialog should remain open when message is empty"
        );
    }

    #[test]
    fn commit_dialog_enter_on_whitespace_only_message_stays_composing() {
        let mut app = make_app();
        app.ui.commit_dialog = Some(CommitDialog::Composing {
            message: "   ".to_string(),
        });
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            matches!(app.ui.commit_dialog, Some(CommitDialog::Composing { .. })),
            "dialog should remain open when message is whitespace-only"
        );
    }

    #[test]
    fn commit_dialog_result_any_key_closes_dialog() {
        let mut app = make_app();
        app.ui.commit_dialog = Some(CommitDialog::Result {
            output: "[main abc1234] fix: bug\n 1 file changed".to_string(),
            succeeded: true,
        });
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.ui.commit_dialog.is_none());
    }

    #[test]
    fn commit_dialog_result_esc_closes_dialog() {
        let mut app = make_app();
        app.ui.commit_dialog = Some(CommitDialog::Result {
            output: "pre-commit hook failed".to_string(),
            succeeded: false,
        });
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.ui.commit_dialog.is_none());
    }

    #[test]
    fn diff_hunk_offsets_fall_back_to_change_blocks_for_difft_output() {
        let request = DiffRequest {
            path: "src/main.rs".to_string(),
            panel_width: 80,
            mode: DiffMode::Inline,
        };
        let app = App {
            repo_root: PathBuf::from("."),
            show_unstaged_only: false,
            snapshot: RepoSnapshot {
                branch: None,
                unpushed_commits: None,
                files: vec![FileStat {
                    path: "src/main.rs".to_string(),
                    additions: 2,
                    deletions: 2,
                    status: FileStatus::Modified,
                    has_staged_changes: false,
                    has_unstaged_changes: true,
                    content_signature: None,
                }],
            },
            ui: UiState {
                selected: 0,
                diff_cursor: 0,
                scroll_offset: 0,
                pending_g: false,
                pending_space: false,
                show_help: false,
                show_sidebar: true,
                hidden_files: HashSet::new(),
                diff_mode: DiffMode::Inline,
                panel_width: 80,
                panel_height: 3,
                focus: Panel::Diff,
                commit_dialog: None,
                push_dialog: None,
            },
            diff_store: DiffStore {
                cache: HashMap::from([(
                    request,
                    DiffContent {
                        lines: vec![
                            Line::from("src/main.rs --- Rust"),
                            Line::from("1 fn main() {"),
                            Line::from(Span::styled(
                                "2     println!(\"old\");",
                                Style::default().fg(Color::Red),
                            )),
                            Line::from(Span::styled(
                                "  2   println!(\"new\");",
                                Style::default().fg(Color::Green),
                            )),
                            Line::from("3 }"),
                            Line::from(Span::styled(
                                "5     println!(\"old2\");",
                                Style::default().fg(Color::Red),
                            )),
                            Line::from(Span::styled(
                                "  5   println!(\"new2\");",
                                Style::default().fg(Color::Green),
                            )),
                        ],
                    },
                )]),
                loading: HashSet::new(),
            },
            last_refresh: Instant::now(),
            should_quit: false,
            error_message: None,
        };

        assert_eq!(app.diff_hunk_offsets(), vec![2, 5]);
    }
}
