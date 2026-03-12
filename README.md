# ripdiff

Real-time git diffs for monitoring agents. A terminal UI for navigating git diffs, designed for a tmux panel workflow where you monitor AI agent changes on one side while working on the other.

Uses [difftastic](https://difftastic.wilfred.me/) for structural, syntax-aware diffs with ANSI color output. Falls back to plain `git diff` if difft is not installed.

## Install

```
cargo install --path .
```

This puts `ripdiff` in `~/.cargo/bin/`.

For best results, install difftastic:

```
cargo install difftastic
```

## Usage

Run inside any git repo with uncommitted changes:

```
ripdiff
```

Or point it at a specific repo:

```
ripdiff --path /some/repo
```

## Key Bindings

### Global

| Key | Action |
|-----|--------|
| `Ctrl-j` | Switch focus to diff panel |
| `Ctrl-k` | Switch focus to file list |
| `t` | Toggle between inline and side-by-side diff |
| `r` | Force refresh |
| `q` / `Esc` | Quit |

### File List Panel

| Key | Action |
|-----|--------|
| `j` / `↓` | Move file selection down |
| `k` / `↑` | Move file selection up |
| `g` / `G` | Jump to top / bottom of file list |
| `Space` / `Enter` | Toggle diff visibility for selected file |

### Diff Panel

| Key | Action |
|-----|--------|
| `j` / `↓` | Scroll down one line |
| `k` / `↑` | Scroll up one line |
| `Ctrl-d` / `Ctrl-u` | Scroll half page down / up |
| `g` / `G` | Jump to top / bottom of diff |
| `]` / `[` | Jump to next / previous hunk |
| `Space` / `Enter` | Toggle diff visibility for selected file |

## Quick Test

```
cd $(mktemp -d)
git init && git commit --allow-empty -m "init"
echo "hello" > test.txt
git add test.txt
ripdiff
```

Edit a file in another terminal — the diff auto-updates within ~1 second.

## Layout

```
┌──────────────────────────────────────────────────────────────┐
│  ripdiff  [repo: myproject]  3 files changed  mode: inline   │
├──────────────────┬───────────────────────────────────────────┤
│ M src/main.rs +5-2│  src/main.rs                             │
│ A src/lib.rs  +3  │                                          │
│ M README.md   +1-1│  <difftastic output here>                │
│                   │                                          │
└──────────────────┴───────────────────────────────────────────┘
```

- 25% left panel: file list with status and stats
- 75% right panel: diff output with scrollbar
- Auto-refreshes on `.git/index` changes and every 500ms
