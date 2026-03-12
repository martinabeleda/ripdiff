use crate::diff::{DiffContent, DiffMode, fetch_diff};
use crate::event::Event;
use crate::git::{FileStat, FileStatus, list_changed_files, repo_root};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::collections::{HashMap, HashSet};
use std::io::Stdout;
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
    pub files: Vec<FileStat>,
    pub selected: usize,
    pub scroll_offset: usize,
    pub diff_cache: HashMap<String, DiffContent>,
    pub hidden_files: HashSet<String>,
    pub diff_mode: DiffMode,
    pub panel_width: u16,
    pub panel_height: u16,
    pub focus: Panel,
    pub last_refresh: Instant,
    pub should_quit: bool,
    pub error_message: Option<String>,
}

impl App {
    pub fn new(start_path: PathBuf) -> Result<Self> {
        let root = repo_root(&start_path)?;
        let mut app = App {
            repo_root: root,
            files: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            diff_cache: HashMap::new(),
            hidden_files: HashSet::new(),
            diff_mode: DiffMode::Inline,
            panel_width: 80,
            panel_height: 40,
            focus: Panel::Files,
            last_refresh: Instant::now() - Duration::from_secs(10),
            should_quit: false,
            error_message: None,
        };
        app.refresh();
        Ok(app)
    }

    pub fn refresh(&mut self) {
        if self.last_refresh.elapsed() < Duration::from_millis(300) {
            return;
        }
        self.last_refresh = Instant::now();
        self.diff_cache.clear();

        match list_changed_files(&self.repo_root) {
            Ok(files) => {
                // Keep selection in bounds
                if files.is_empty() {
                    self.selected = 0;
                } else if self.selected >= files.len() {
                    self.selected = files.len() - 1;
                }
                self.files = files;
                self.error_message = None;
            }
            Err(e) => {
                self.error_message = Some(format!("git error: {e}"));
            }
        }
    }

    pub fn force_refresh(&mut self) {
        self.last_refresh = Instant::now() - Duration::from_secs(10);
        self.refresh();
    }

    pub fn selected_file(&self) -> Option<&FileStat> {
        self.files.get(self.selected)
    }

    pub fn get_diff(&mut self) -> Option<&DiffContent> {
        let file = self.files.get(self.selected)?;
        let path = file.path.clone();

        if self.hidden_files.contains(&path) {
            return None;
        }

        if !self.diff_cache.contains_key(&path) {
            let is_untracked = file.status == FileStatus::Untracked;
            let result = fetch_diff(&self.repo_root, &path, self.panel_width, &self.diff_mode, is_untracked);
            match result {
                Ok(content) => {
                    self.diff_cache.insert(path.clone(), content);
                }
                Err(e) => {
                    // Insert empty content with error message
                    use ratatui::style::{Color, Style};
                    use ratatui::text::{Line, Span};
                    let error_line =
                        Line::from(Span::styled(format!("Error: {e}"), Style::default().fg(Color::Red)));
                    self.diff_cache.insert(
                        path.clone(),
                        DiffContent { lines: vec![error_line] },
                    );
                }
            }
        }

        self.diff_cache.get(&path)
    }

    pub fn scroll_down(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(amount);
    }

    pub fn scroll_up(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.files.len() {
            self.selected += 1;
            self.scroll_offset = 0;
        }
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            self.scroll_offset = 0;
        }
    }

    pub fn jump_top(&mut self) {
        self.selected = 0;
        self.scroll_offset = 0;
    }

    pub fn jump_bottom(&mut self) {
        if !self.files.is_empty() {
            self.selected = self.files.len() - 1;
            self.scroll_offset = 0;
        }
    }

    pub fn toggle_hidden(&mut self) {
        if let Some(file) = self.files.get(self.selected) {
            let path = file.path.clone();
            if self.hidden_files.contains(&path) {
                self.hidden_files.remove(&path);
            } else {
                self.hidden_files.insert(path);
                self.scroll_offset = 0;
            }
        }
    }

    pub fn toggle_diff_mode(&mut self) {
        self.diff_mode = self.diff_mode.toggle();
        self.diff_cache.clear();
        self.scroll_offset = 0;
    }

    /// Find hunk boundary lines in the current diff.
    /// Hunk boundaries are lines starting with "@@" (unified diff) or
    /// lines that contain a filename header from difftastic.
    pub fn diff_hunk_offsets(&self) -> Vec<usize> {
        let path = match self.files.get(self.selected) {
            Some(f) => &f.path,
            None => return vec![],
        };
        let diff = match self.diff_cache.get(path) {
            Some(d) => d,
            None => return vec![],
        };

        let mut offsets = Vec::new();
        for (i, line) in diff.lines.iter().enumerate() {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            let trimmed = text.trim_start();
            // Unified diff hunk headers, or difftastic section markers
            if trimmed.starts_with("@@") || trimmed.starts_with("───") {
                offsets.push(i);
            }
        }
        offsets
    }

    pub fn jump_next_hunk(&mut self) {
        let hunks = self.diff_hunk_offsets();
        if let Some(&offset) = hunks.iter().find(|&&o| o > self.scroll_offset) {
            self.scroll_offset = offset;
        }
    }

    pub fn jump_prev_hunk(&mut self) {
        let hunks = self.diff_hunk_offsets();
        if let Some(&offset) = hunks.iter().rev().find(|&&o| o < self.scroll_offset) {
            self.scroll_offset = offset;
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        // Global keys (work in any panel)
        match (key.code, key.modifiers) {
            (KeyCode::Char('q'), KeyModifiers::NONE) | (KeyCode::Esc, _) => {
                self.should_quit = true;
                return;
            }
            (KeyCode::Tab, _) => {
                self.focus = match self.focus {
                    Panel::Files => Panel::Diff,
                    Panel::Diff => Panel::Files,
                };
                return;
            }
            (KeyCode::BackTab, _) => {
                self.focus = match self.focus {
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

        match self.focus {
            Panel::Files => self.handle_files_key(key),
            Panel::Diff => self.handle_diff_key(key),
        }
    }

    fn handle_files_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.move_down(),
            KeyCode::Char('k') | KeyCode::Up => self.move_up(),
            KeyCode::Char('l') | KeyCode::Right => { self.focus = Panel::Diff; }
            KeyCode::Char('g') => self.jump_top(),
            KeyCode::Char('G') => self.jump_bottom(),
            KeyCode::Char(' ') | KeyCode::Enter => self.toggle_hidden(),
            _ => {}
        }
    }

    fn handle_diff_key(&mut self, key: KeyEvent) {
        let half_page = (self.panel_height / 2) as usize;
        match (key.code, key.modifiers) {
            // Line-by-line scrolling
            (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => self.scroll_down(1),
            (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => self.scroll_up(1),
            // Back to file list
            (KeyCode::Char('h'), KeyModifiers::NONE) | (KeyCode::Left, _) => { self.focus = Panel::Files; }
            // Half-page scrolling
            (KeyCode::Char('d'), KeyModifiers::CONTROL) => self.scroll_down(half_page),
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => self.scroll_up(half_page),
            // Top / bottom of diff
            (KeyCode::Char('g'), KeyModifiers::NONE) => { self.scroll_offset = 0; }
            (KeyCode::Char('G'), KeyModifiers::NONE) => { self.scroll_offset = usize::MAX; }
            // Hunk jumping
            (KeyCode::Char(']'), KeyModifiers::NONE) => self.jump_next_hunk(),
            (KeyCode::Char('['), KeyModifiers::NONE) => self.jump_prev_hunk(),
            // Toggle visibility still works
            (KeyCode::Char(' '), _) | (KeyCode::Enter, _) => self.toggle_hidden(),
            _ => {}
        }
    }
}

pub async fn run(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    mut app: App,
    mut rx: mpsc::UnboundedReceiver<Event>,
) -> Result<()> {
    loop {
        // Update panel width based on terminal size
        let size = terminal.size()?;
        let right_width = (size.width as f32 * 0.75) as u16;
        let panel_height = size.height.saturating_sub(3); // title + borders
        if right_width != app.panel_width {
            app.panel_width = right_width;
            app.diff_cache.clear();
        }
        app.panel_height = panel_height;

        // Pre-fetch diff for selected file
        let _ = app.get_diff();

        terminal.draw(|frame| crate::ui::render(frame, &mut app))?;

        if app.should_quit {
            break;
        }

        match rx.recv().await {
            Some(Event::Key(key)) => app.handle_key(key),
            Some(Event::Resize) => {
                app.diff_cache.clear();
            }
            Some(Event::FsChange) => app.refresh(),
            Some(Event::Tick) => app.refresh(),
            None => break,
        }
    }

    Ok(())
}
