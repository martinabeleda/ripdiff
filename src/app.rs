use crate::diff::{DiffContent, DiffMode, DiffRequest, DiffService};
use crate::event::Event;
use crate::git::{load_snapshot, repo_root, FileStat, FileStatus, RepoSnapshot};
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

pub struct App {
    pub repo_root: PathBuf,
    pub snapshot: RepoSnapshot,
    pub ui: UiState,
    pub diff_store: DiffStore,
    pub last_refresh: Instant,
    pub should_quit: bool,
    pub error_message: Option<String>,
}

pub struct UiState {
    pub selected: usize,
    pub scroll_offset: usize,
    pub hidden_files: HashSet<String>,
    pub diff_mode: DiffMode,
    pub panel_width: u16,
    pub panel_height: u16,
    pub focus: Panel,
}

pub struct DiffStore {
    pub cache: HashMap<DiffRequest, DiffContent>,
    pub loading: HashSet<DiffRequest>,
}

impl App {
    pub fn new(start_path: PathBuf) -> Result<Self> {
        let root = repo_root(&start_path)?;
        let snapshot = load_snapshot(&root)?;

        Ok(App {
            repo_root: root,
            snapshot,
            ui: UiState {
                selected: 0,
                scroll_offset: 0,
                hidden_files: HashSet::new(),
                diff_mode: DiffMode::Inline,
                panel_width: 80,
                panel_height: 40,
                focus: Panel::Files,
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

        match load_snapshot(&self.repo_root) {
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

    pub fn selected_diff_is_loading(&self) -> bool {
        self.selected_diff_request()
            .map(|request| self.diff_store.loading.contains(&request))
            .unwrap_or(false)
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
        service.request(request, is_untracked);
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
        self.ui.scroll_offset = self.ui.scroll_offset.saturating_add(amount);
    }

    pub fn scroll_up(&mut self, amount: usize) {
        self.ui.scroll_offset = self.ui.scroll_offset.saturating_sub(amount);
    }

    pub fn move_down(&mut self) {
        if self.ui.selected + 1 < self.snapshot.files.len() {
            self.ui.selected += 1;
            self.ui.scroll_offset = 0;
        }
    }

    pub fn move_up(&mut self) {
        if self.ui.selected > 0 {
            self.ui.selected -= 1;
            self.ui.scroll_offset = 0;
        }
    }

    pub fn jump_top(&mut self) {
        self.ui.selected = 0;
        self.ui.scroll_offset = 0;
    }

    pub fn jump_bottom(&mut self) {
        if !self.snapshot.files.is_empty() {
            self.ui.selected = self.snapshot.files.len() - 1;
            self.ui.scroll_offset = 0;
        }
    }

    pub fn toggle_hidden(&mut self) {
        if let Some(file) = self.selected_file() {
            let path = file.path.clone();
            if self.ui.hidden_files.contains(&path) {
                self.ui.hidden_files.remove(&path);
            } else {
                self.ui.hidden_files.insert(path);
                self.ui.scroll_offset = 0;
            }
        }
    }

    pub fn toggle_diff_mode(&mut self) {
        self.ui.diff_mode = self.ui.diff_mode.toggle();
        self.diff_store.cache.clear();
        self.diff_store.loading.clear();
        self.ui.scroll_offset = 0;
    }

    pub fn diff_hunk_offsets(&self) -> Vec<usize> {
        let Some(diff) = self.selected_diff() else {
            return vec![];
        };

        let mut offsets = Vec::new();
        for (index, line) in diff.lines.iter().enumerate() {
            let text: String = line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect();
            let trimmed = text.trim_start();
            if trimmed.starts_with("@@") || trimmed.starts_with("───") {
                offsets.push(index);
            }
        }
        offsets
    }

    pub fn jump_next_hunk(&mut self) {
        let hunks = self.diff_hunk_offsets();
        if let Some(&offset) = hunks.iter().find(|&&offset| offset > self.ui.scroll_offset) {
            self.ui.scroll_offset = offset;
        }
    }

    pub fn jump_prev_hunk(&mut self) {
        let hunks = self.diff_hunk_offsets();
        if let Some(&offset) = hunks
            .iter()
            .rev()
            .find(|&&offset| offset < self.ui.scroll_offset)
        {
            self.ui.scroll_offset = offset;
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        match (key.code, key.modifiers) {
            (KeyCode::Char('q'), KeyModifiers::NONE) | (KeyCode::Esc, _) => {
                self.should_quit = true;
                return;
            }
            (KeyCode::Tab, _) | (KeyCode::BackTab, _) => {
                self.ui.focus = match self.ui.focus {
                    Panel::Files => Panel::Diff,
                    Panel::Diff => Panel::Files,
                };
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
            _ => {}
        }

        match self.ui.focus {
            Panel::Files => self.handle_files_key(key),
            Panel::Diff => self.handle_diff_key(key),
        }
    }

    fn handle_files_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.move_down(),
            KeyCode::Char('k') | KeyCode::Up => self.move_up(),
            KeyCode::Char('l') | KeyCode::Right => {
                self.ui.focus = Panel::Diff;
            }
            KeyCode::Char('g') => self.jump_top(),
            KeyCode::Char('G') => self.jump_bottom(),
            KeyCode::Char(' ') | KeyCode::Enter => self.toggle_hidden(),
            _ => {}
        }
    }

    fn handle_diff_key(&mut self, key: KeyEvent) {
        let half_page = (self.ui.panel_height / 2) as usize;
        match (key.code, key.modifiers) {
            (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => self.scroll_down(1),
            (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => self.scroll_up(1),
            (KeyCode::Char('h'), KeyModifiers::NONE) | (KeyCode::Left, _) => {
                self.ui.focus = Panel::Files;
            }
            (KeyCode::Char('d'), KeyModifiers::CONTROL) => self.scroll_down(half_page),
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => self.scroll_up(half_page),
            (KeyCode::Char('g'), KeyModifiers::NONE) => {
                self.ui.scroll_offset = 0;
            }
            (KeyCode::Char('G'), KeyModifiers::NONE) => {
                self.ui.scroll_offset = usize::MAX;
            }
            (KeyCode::Char(']'), KeyModifiers::NONE) => self.jump_next_hunk(),
            (KeyCode::Char('['), KeyModifiers::NONE) => self.jump_prev_hunk(),
            (KeyCode::Char(' '), _) | (KeyCode::Enter, _) => self.toggle_hidden(),
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
                return;
            }
        }

        if self.ui.selected >= self.snapshot.files.len() {
            self.ui.selected = self.snapshot.files.len() - 1;
        }
    }
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
        let right_width = (size.width as f32 * 0.75) as u16;
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
    use ratatui::text::Line;

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
            snapshot: RepoSnapshot::default(),
            ui: UiState {
                selected: 0,
                scroll_offset: 0,
                hidden_files: HashSet::new(),
                diff_mode: DiffMode::Inline,
                panel_width: 80,
                panel_height: 40,
                focus: Panel::Files,
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
            snapshot: RepoSnapshot {
                files: vec![FileStat {
                    path: "src/main.rs".to_string(),
                    additions: 1,
                    deletions: 0,
                    status: FileStatus::Modified,
                }],
            },
            ui: UiState {
                selected: 0,
                scroll_offset: 0,
                hidden_files: HashSet::new(),
                diff_mode: DiffMode::Inline,
                panel_width: 80,
                panel_height: 40,
                focus: Panel::Files,
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
            files: vec![FileStat {
                path: "src/main.rs".to_string(),
                additions: 1,
                deletions: 0,
                status: FileStatus::Modified,
            }],
        };
        let request = DiffRequest {
            path: "src/main.rs".to_string(),
            panel_width: 80,
            mode: DiffMode::Inline,
        };
        let mut app = App {
            repo_root: PathBuf::from("."),
            snapshot: snapshot.clone(),
            ui: UiState {
                selected: 0,
                scroll_offset: 0,
                hidden_files: HashSet::new(),
                diff_mode: DiffMode::Inline,
                panel_width: 80,
                panel_height: 40,
                focus: Panel::Files,
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
}
