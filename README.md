```
            ███                █████  ███     ██████     ██████ 
           ░░░                ░░███  ░░░     ███░░███   ███░░███
 ████████  ████  ████████   ███████  ████   ░███ ░░░   ░███ ░░░ 
░░███░░███░░███ ░░███░░███ ███░░███ ░░███  ███████    ███████   
 ░███ ░░░  ░███  ░███ ░███░███ ░███  ░███ ░░░███░    ░░░███░    
 ░███      ░███  ░███ ░███░███ ░███  ░███   ░███       ░███     
 █████     █████ ░███████ ░░████████ █████  █████      █████    
░░░░░     ░░░░░  ░███░░░   ░░░░░░░░ ░░░░░  ░░░░░      ░░░░░     
                 ░███                                           
                 █████                                          
                ░░░░░
```

A terminal UI for navigating git diffs, designed for a tmux panel workflow where you monitor AI agent changes on one side while working on the other.

Uses [difftastic](https://difftastic.wilfred.me/) for structural, syntax-aware diffs with ANSI color output via an in-process Rust dependency.

## Install

```
cargo install --path .
```

This puts `ripdiff` in `~/.cargo/bin/`.

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
| `Tab` / `Shift-Tab` | Toggle focus between panels |
| `h` / `?` | Open or close help |
| `t` | Toggle between inline and side-by-side diff |
| `r` | Force refresh |
| `q` / `Esc` | Quit |

### File List Panel

| Key | Action |
|-----|--------|
| `j` / `↓` | Move file selection down |
| `k` / `↑` | Move file selection up |
| `→` | Switch to diff panel |
| `gg` / `G` | Jump to top / bottom of file list |
| `s` / `S` | Toggle selected file staged / toggle all files staged |
| `Space e` | Hide / show file list sidebar |
| `Enter` | Toggle diff visibility for selected file |

### Diff Panel

| Key | Action |
|-----|--------|
| `j` / `↓` | Scroll down one line |
| `k` / `↑` | Scroll up one line |
| `←` | Switch to file list |
| `Ctrl-d` / `Ctrl-u` | Scroll half page down / up |
| `gg` / `G` | Jump to top / bottom of diff |
| `s` / `S` | Toggle selected file staged / toggle all files staged |
| `]` / `[` | Jump to next / previous hunk |
| `Space e` | Hide / show file list sidebar |
| `Enter` | Toggle diff visibility for selected file |

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
  ripdiff  [repo: myproject]   main  3 files changed  mode: inline  panel: files
  M src/main.rs  +5-2 │ src/main.rs
  A src/lib.rs   +3   │
  M README.md    +1-1 │   fn main() {
  ? new_file.rs  +12  │ -     println!("old");
                      │ +     println!("new");
                      │   }
```

- 25% left: file list with status indicators (M/A/D/R/?) and stage markers (`●` staged, `○` unstaged, `◐` mixed)
- 75% right: diff output with scrollbar
- `h` opens a help popup with keybinding and symbol descriptions
- Minimal borders — just a vertical divider between panels
- Auto-refreshes on `.git/index` changes and every 500ms
