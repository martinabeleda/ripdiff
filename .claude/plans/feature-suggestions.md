# ripdiff Feature Suggestions

## Priority Ranking

| Priority | Feature | Effort | Impact |
|----------|---------|--------|--------|
| 🔥 | Prefetch adjacent diffs (#13) | Low | High |
| 🔥 | File search/filter (#1) | Medium | High |
| 🔥 | Copy path to clipboard (#7) | Low | Medium |
| 🔥 | Mouse support (#12) | Low | Medium |
| ⭐ | Diff statistics summary (#2) | Low | Medium |
| ⭐ | File discard (#4) | Medium | High |
| ⭐ | Watch pause toggle (#8) | Low | Medium |
| ⭐ | Commit log viewer (#15) | Medium | High |
| 💎 | Hunk-level staging (#3) | High | Very High |
| 💎 | Commit range diffs (#10) | High | High |
| 💎 | Config file (#9) | Medium | Medium |

---

## 1. File Search / Fuzzy Filter (`/` key)

When you have many changed files, there's no way to quickly find one. Adding a `/` keybinding to open a filter input (like vim's `/` or fzf) that narrows the file list as you type would be very useful, especially in large agent-driven changesets.

**Files to modify:** `app.rs` (new `FilterDialog` state, key handling), `ui.rs` (render filter input overlay, filtered file list)

## 2. Diff Statistics Summary Bar

Show a summary like `12 files changed, +340 -87` in the status bar (bottom). The data is already available in `RepoSnapshot` — just needs aggregation and rendering. Gives instant context on change magnitude.

**Files to modify:** `ui.rs` (`render_statusline` — aggregate `FileStat.additions`/`deletions` across all files)

## 3. Hunk-level Staging (partial staging)

Currently staging is file-level only (`git add`). Supporting `git add -p` style hunk-level staging would be a killer feature — select a hunk in the diff panel and stage just that hunk. This is the main gap vs. tools like lazygit/magit.

**Files to modify:** `git.rs` (new `stage_hunk` function using `git apply --cached`), `app.rs` (hunk selection state, key handling for staging hunks), `diff.rs` (extract hunk boundaries from diff output), `ui.rs` (hunk highlight/selection rendering)

**Approach:** Generate a patch for the selected hunk and pipe it to `git apply --cached`. Need to track hunk boundaries in `DiffContent` alongside the rendered lines.

## 4. File Discard / Checkout (`d` key)

Add the ability to discard changes for a file (`git checkout -- <file>` / `git restore <file>`) with a confirmation prompt. Currently you can stage/unstage but can't revert unwanted changes without leaving the TUI.

**Files to modify:** `git.rs` (new `discard_file` function), `app.rs` (new `ConfirmDialog` state, `d` key handling), `ui.rs` (render confirmation dialog)

## 5. Stash Support (`z` key prefix)

Add `zz` to stash, `zp` to pop, `zl` to list stashes. Stashing is a common workflow when reviewing agent changes — you might want to stash some changes before testing.

**Files to modify:** `git.rs` (new `stash`, `stash_pop`, `stash_list` functions), `app.rs` (pending `z` key sequence, stash dialog state), `ui.rs` (stash list overlay)

## 6. Diff Word-wrap / Horizontal Scroll

Long lines get truncated in the diff panel. Adding horizontal scroll (`H`/`L` or arrow keys when in diff) or optional word-wrap toggle would help with minified files or long lines.

**Files to modify:** `app.rs` (new `horizontal_offset` in `UiState`, key handling for horizontal scroll), `ui.rs` (`render_diff_panel` — apply horizontal offset to visible lines)

## 7. Copy File Path to Clipboard (`y` key)

Press `y` to yank the selected file path to the system clipboard. Useful in the tmux panel workflow described in the README — quickly grab a path to use in the other pane.

**Files to modify:** `app.rs` (`y` key handler), new clipboard utility (use `xclip`/`pbcopy`/`wl-copy` via `Command`, or add `arboard` crate dependency)

## 8. Watch Mode Indicator & Manual Pause

Show a visual indicator that file-watching is active (e.g., a small `👁` icon in the title bar). Add a keybinding to pause/resume auto-refresh — useful when an agent is rapidly writing files and you want to freeze the view to read a diff.

**Files to modify:** `app.rs` (new `paused` bool in `App`, modify `refresh` to respect it, key handler for toggle), `ui.rs` (`render_title` — show pause/watch indicator)

## 9. Configurable Color Theme / `.ripdiffrc`

Colors are hardcoded (e.g., `Color::Rgb(22, 48, 30)` for addition backgrounds). A config file (TOML) supporting custom themes would let users match their terminal theme. Could also persist preferences like default diff mode and sidebar visibility.

**Files to modify:** New `config.rs` module (parse TOML config from `~/.config/ripdiff/config.toml`), `ui.rs` (replace hardcoded colors with config lookups), `main.rs` (load config on startup), `Cargo.toml` (add `toml` crate)

## 10. Diff for Specific Commits / Ranges

Currently only shows uncommitted changes. Adding `ripdiff HEAD~3..HEAD` or `ripdiff <commit>` to review committed diffs would make it useful beyond just watching agent progress — general-purpose commit review.

**Files to modify:** `main.rs` (new CLI arg for commit range), `git.rs` (new `load_snapshot_for_range` using `git diff <range> --numstat`), `diff.rs` (new `fetch_diff_for_range` using revision pairs), `app.rs` (store range context)

## 11. Binary File Detection & Preview

Binary files (images, compiled assets) currently show `[binary file]` or error. Could show file size, type info, and for images, a sixel/kitty protocol inline preview on supported terminals.

**Files to modify:** `diff.rs` (`show_new_file` — detect binary via `tree_magic_mini` already in deps, show metadata), `ui.rs` (optional image protocol rendering)

## 12. Mouse Support

Ratatui supports mouse events via crossterm. Click to select files, scroll with mouse wheel in the diff panel. Low effort, high QoL for users who aren't vim-native.

**Files to modify:** `main.rs` (enable mouse capture in crossterm setup), `event.rs` (handle `CrosstermEvent::Mouse` variants), `app.rs` (new `handle_mouse` method — map click coordinates to file selection or diff scroll)

## 13. Prefetch Adjacent Diffs

Currently only the selected file's diff is computed. Prefetching diffs for files ±1 from the selection would make navigation feel instant. The `DiffService` async architecture already supports this — just call `ensure_diff` for neighbors.

**Files to modify:** `app.rs` (new `ensure_adjacent_diffs` method called alongside `ensure_selected_diff`, request diffs for `selected ± 1`)

## 14. Branch Switching / Checkout

A branch picker dialog (like the commit dialog) to switch branches. Useful when reviewing agent work across multiple branches.

**Files to modify:** `git.rs` (new `list_branches`, `checkout_branch` functions), `app.rs` (new `BranchDialog` state, key handling), `ui.rs` (branch picker overlay with scrollable list)

## 15. Commit Log Viewer

A `l` keybinding to show recent commit history (like `git log --oneline`). Helps understand context when reviewing agent progress — "what did the agent already commit?"

**Files to modify:** `git.rs` (new `recent_commits` function), `app.rs` (new `LogDialog` state, `l` key handler), `ui.rs` (log overlay with scrollable list)
