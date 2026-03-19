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

A terminal UI for watching and reviewing agent progress, designed for a tmux panel workflow where you monitor agent changes on one side while working on the other.

Uses [difftastic](https://difftastic.wilfred.me/) for structural, syntax-aware diffs with ANSI color output via an in-process Rust dependency.

![ripdiff demo](docs/media/strava-demo.gif)

## Install

### Install from crates.io

```bash
cargo install --locked ripdiff
```

This installs `ripdiff` into `~/.cargo/bin/`.

### Install from source (local checkout)

```bash
cargo install --path .
```

## Releasing

Maintainer release instructions live in [RELEASING.md](/home/mabeleda/Development/ripdiff/RELEASING.md).

## Usage

Run inside any git repo with uncommitted changes:

```
ripdiff

# or
rd
```

Or point it at a specific repo:

```
ripdiff /some/repo
```

Show only unstaged changes:

```
ripdiff -u

# or
ripdiff --unstaged-only
```

## Key Bindings

### Global

| Key | Action |
|-----|--------|
| `Tab` / `Shift-Tab` | Toggle focus between panels |
| `h` / `?` | Open or close help |
| `t` | Toggle between inline and side-by-side diff |
| `u` | Toggle between all changes and unstaged-only changes |
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
