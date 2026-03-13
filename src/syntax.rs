use std::path::Path;
use std::sync::OnceLock;

use syntect::easy::HighlightLines;
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};
use syntect::util::{as_24_bit_terminal_escaped, LinesWithEndings};

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME: OnceLock<Theme> = OnceLock::new();

pub fn highlight_new_file(file_path: &str, content: &str) -> String {
    let syntax = syntax_for_content(file_path, content);
    let mut highlighter = HighlightLines::new(syntax, theme());
    let mut out = String::new();

    out.push_str(&ansi("1;32", &format!("new file: {file_path}")));
    out.push_str("\n\n");

    for (index, line) in LinesWithEndings::from(content).enumerate() {
        out.push_str(&ansi("90", &format!("{:>4} ", index + 1)));
        out.push_str(&ansi("32", "+ "));
        out.push_str(&highlight_line(&mut highlighter, line));
    }

    if !content.ends_with('\n') && !content.is_empty() {
        out.push('\n');
    }

    out
}

pub fn highlight_unified_diff(file_path: &str, diff: &str) -> String {
    let syntax = syntax_for_content(file_path, diff);
    let mut highlighter = HighlightLines::new(syntax, theme());
    let mut out = String::new();

    for line in LinesWithEndings::from(diff) {
        let rendered = if let Some(content) = line.strip_prefix('+') {
            if line.starts_with("+++") {
                ansi("1;32", line)
            } else {
                format!(
                    "{}{}",
                    ansi("32", "+"),
                    highlight_line(&mut highlighter, content)
                )
            }
        } else if let Some(content) = line.strip_prefix('-') {
            if line.starts_with("---") {
                ansi("1;31", line)
            } else {
                format!(
                    "{}{}",
                    ansi("31", "-"),
                    highlight_line(&mut highlighter, content)
                )
            }
        } else if let Some(content) = line.strip_prefix(' ') {
            format!(" {}", highlight_line(&mut highlighter, content))
        } else if line.starts_with("@@") {
            ansi("1;35", line)
        } else if line.starts_with("diff --git")
            || line.starts_with("index ")
            || line.starts_with("new file mode ")
            || line.starts_with("deleted file mode ")
            || line.starts_with("similarity index ")
            || line.starts_with("rename from ")
            || line.starts_with("rename to ")
        {
            ansi("1;36", line)
        } else if line.starts_with('\\') {
            ansi("33", line)
        } else {
            line.to_string()
        };

        out.push_str(&rendered);
    }

    out
}

fn syntax_set() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme() -> &'static Theme {
    THEME.get_or_init(|| {
        let themes = ThemeSet::load_defaults();
        themes
            .themes
            .get("base16-ocean.dark")
            .cloned()
            .unwrap_or_default()
    })
}

fn syntax_for_content<'a>(file_path: &str, content: &'a str) -> &'static SyntaxReference {
    let syntax_set = syntax_set();
    let path = Path::new(file_path);

    if let Some(syntax) = syntax_set.find_syntax_for_file(path).ok().flatten() {
        return syntax;
    }

    if let Some(first_line) = content.lines().next() {
        if let Some(syntax) = syntax_set.find_syntax_by_first_line(first_line) {
            return syntax;
        }
    }

    syntax_set.find_syntax_plain_text()
}

fn highlight_line(highlighter: &mut HighlightLines<'_>, line: &str) -> String {
    match highlighter.highlight_line(line, syntax_set()) {
        Ok(ranges) => as_24_bit_terminal_escaped(&ranges[..], false),
        Err(_) => line.to_string(),
    }
}

fn ansi(code: &str, text: &str) -> String {
    format!("\x1b[{code}m{text}\x1b[0m")
}
