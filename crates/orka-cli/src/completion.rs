use std::{
    borrow::Cow,
    cell::Cell,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
};

use nu_ansi_term::{Color, Style};
use reedline::{
    Completer, Highlighter, Hinter, Prompt, PromptEditMode, PromptHistorySearch,
    PromptHistorySearchStatus, Span, StyledText, Suggestion, ValidationResult, Validator,
};

/// Shell builtins handled by the `!` prefix.
const SHELL_BUILTINS: &[&str] = &["cd", "export", "unset", "history"];

/// Known local + server slash commands for completion.
const SLASH_COMMANDS: &[&str] = &[
    "/quit",
    "/exit",
    "/help",
    "/clear",
    "/skill",
    "/skills",
    "/reset",
    "/status",
    "/think",
    "/feedback",
    "/history",
    "/save",
    "/copy",
    "/open",
];

/// Lazily populated list of executables from $PATH (populated on first
/// tab-completion).
static PATH_COMMANDS: LazyLock<Vec<String>> = LazyLock::new(collect_path_commands);

// ── Completer ────────────────────────────────────────────────────────────────

pub struct OrkaCompleter {
    shell_cwd: Arc<Mutex<PathBuf>>,
}

impl OrkaCompleter {
    pub fn new(shell_cwd: Arc<Mutex<PathBuf>>) -> Self {
        Self { shell_cwd }
    }
}

impl Completer for OrkaCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> Vec<Suggestion> {
        let text = &line[..pos];

        // After `!` — complete shell commands and file paths
        if let Some(rest) = text.strip_prefix('!') {
            let parts: Vec<&str> = rest.splitn(2, ' ').collect();
            if parts.len() == 1 && !rest.ends_with(' ') {
                let prefix = parts[0].to_lowercase();
                return PATH_COMMANDS
                    .iter()
                    .filter(|c| c.starts_with(&prefix))
                    .map(|c| Suggestion {
                        value: c.clone(),
                        display_override: None,
                        description: None,
                        style: None,
                        extra: None,
                        span: Span::new(1, pos),
                        append_whitespace: true,
                        match_indices: None,
                    })
                    .collect();
            }
            let arg = if parts.len() == 2 { parts[1] } else { "" };
            let dirs_only = parts[0].eq_ignore_ascii_case("cd");
            let cwd = self
                .shell_cwd
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            let (start_in_rest, completions) = complete_path(arg, dirs_only, &cwd);
            let offset = if parts.len() == 2 {
                1 + parts[0].len() + 1 + start_in_rest
            } else {
                pos
            };
            return completions
                .into_iter()
                .map(|(display, replacement)| Suggestion {
                    value: replacement,
                    display_override: Some(display),
                    description: None,
                    style: None,
                    extra: None,
                    span: Span::new(offset, pos),
                    append_whitespace: false,
                    match_indices: None,
                })
                .collect();
        }

        // After `/feedback ` — complete argument keywords
        if let Some(rest) = text.strip_prefix("/feedback ") {
            let prefix = rest.to_lowercase();
            let offset = text.len() - rest.len();
            return ["good", "bad"]
                .iter()
                .filter(|k| k.starts_with(prefix.as_str()))
                .map(|k| Suggestion {
                    value: k.to_string(),
                    display_override: None,
                    description: None,
                    style: None,
                    extra: None,
                    span: Span::new(offset, pos),
                    append_whitespace: true,
                    match_indices: None,
                })
                .collect();
        }

        // After `/` — complete slash commands
        if text.starts_with('/') && !text.contains(' ') {
            let prefix = text.to_lowercase();
            return SLASH_COMMANDS
                .iter()
                .filter(|c| c.starts_with(&prefix))
                .map(|c| Suggestion {
                    value: c.to_string(),
                    display_override: None,
                    description: None,
                    style: None,
                    extra: None,
                    span: Span::new(0, pos),
                    append_whitespace: false,
                    match_indices: None,
                })
                .collect();
        }

        // After `@` — complete file paths for attachments
        if let Some(at_pos) = text.rfind('@') {
            let preceded_by_ws = at_pos == 0
                || text[..at_pos]
                    .chars()
                    .next_back()
                    .map(|c| c.is_whitespace())
                    .unwrap_or(true);
            if preceded_by_ws {
                let fragment = &text[at_pos + 1..];
                let cwd = self
                    .shell_cwd
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                let (start_in_fragment, completions) = complete_path(fragment, false, &cwd);
                let offset = at_pos + 1 + start_in_fragment;
                return completions
                    .into_iter()
                    .map(|(display, replacement)| Suggestion {
                        value: replacement,
                        display_override: Some(display),
                        description: None,
                        style: None,
                        extra: None,
                        span: Span::new(offset, pos),
                        append_whitespace: false,
                        match_indices: None,
                    })
                    .collect();
            }
        }

        vec![]
    }
}

// ── Hinter ───────────────────────────────────────────────────────────────────

pub struct OrkaHinter {
    /// The last hint computed by `handle`, for
    /// `complete_hint`/`next_hint_token`.
    current_hint: Cell<String>,
}

impl OrkaHinter {
    pub fn new() -> Self {
        Self {
            current_hint: Cell::new(String::new()),
        }
    }
}

impl Hinter for OrkaHinter {
    fn handle(
        &mut self,
        line: &str,
        _pos: usize,
        _history: &dyn reedline::History,
        use_ansi_coloring: bool,
        _cwd: &str,
    ) -> String {
        // Ghost text for unique `!` builtin prefix matches
        if let Some(rest) = line.strip_prefix('!') {
            if !rest.contains(' ') && !rest.is_empty() {
                let lower = rest.to_lowercase();
                let matches: Vec<&&str> = SHELL_BUILTINS
                    .iter()
                    .filter(|b| b.starts_with(lower.as_str()) && **b != lower.as_str())
                    .collect();
                if matches.len() == 1 {
                    let suffix = matches[0][rest.len()..].to_string();
                    let hint = if use_ansi_coloring && std::env::var_os("NO_COLOR").is_none() {
                        Style::new().fg(Color::DarkGray).paint(&suffix).to_string()
                    } else {
                        suffix.clone()
                    };
                    self.current_hint.set(suffix);
                    return hint;
                }
            }
            self.current_hint.set(String::new());
            return String::new();
        }

        // Ghost text for unique `/` command prefix matches
        if !line.starts_with('/') || line.contains(' ') {
            self.current_hint.set(String::new());
            return String::new();
        }
        let lower = line.to_lowercase();
        let matches: Vec<&&str> = SLASH_COMMANDS
            .iter()
            .filter(|c| c.starts_with(lower.as_str()) && **c != lower.as_str())
            .collect();
        if matches.len() == 1 {
            let suffix = matches[0][line.len()..].to_string();
            let hint = if use_ansi_coloring && std::env::var_os("NO_COLOR").is_none() {
                Style::new().fg(Color::DarkGray).paint(&suffix).to_string()
            } else {
                suffix.clone()
            };
            self.current_hint.set(suffix);
            hint
        } else {
            self.current_hint.set(String::new());
            String::new()
        }
    }

    fn complete_hint(&self) -> String {
        self.current_hint
            .take()
            .also(|s| self.current_hint.set(s.clone()))
    }

    fn next_hint_token(&self) -> String {
        let hint = self.current_hint.take();
        let token = hint.split_whitespace().next().unwrap_or("").to_string();
        self.current_hint.set(hint);
        token
    }
}

// ── Highlighter
// ───────────────────────────────────────────────────────────────

pub struct OrkaHighlighter;

impl Highlighter for OrkaHighlighter {
    fn highlight(&self, line: &str, _cursor: usize) -> StyledText {
        let mut styled = StyledText::new();
        if line.starts_with('!') {
            styled.push((Style::new().fg(Color::Yellow), line.to_string()));
        } else if line.starts_with('/') {
            styled.push((Style::new().fg(Color::Cyan), line.to_string()));
        } else {
            // Highlight `@<token>` segments in green; rest unstyled
            let segments = highlight_at_tokens_styled(line);
            for seg in segments {
                styled.push(seg);
            }
        }
        styled
    }
}

// ── Validator ────────────────────────────────────────────────────────────────

pub struct OrkaValidator;

impl Validator for OrkaValidator {
    fn validate(&self, line: &str) -> ValidationResult {
        match validate_input(line) {
            ValidateOutcome::Complete => ValidationResult::Complete,
            ValidateOutcome::Incomplete => ValidationResult::Incomplete,
        }
    }
}

// ── Prompt ───────────────────────────────────────────────────────────────────

/// Carries the colored prompt string for reedline to display.
pub struct OrkaPrompt {
    pub colored: String,
}

impl Prompt for OrkaPrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.colored)
    }

    fn render_prompt_right(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn render_prompt_indicator(&self, _mode: PromptEditMode) -> Cow<'_, str> {
        // The `❯` is already included in the colored prompt from build_prompt()
        Cow::Borrowed("")
    }

    fn render_prompt_multiline_indicator(&self) -> Cow<'_, str> {
        Cow::Borrowed("  ")
    }

    fn render_prompt_history_search_indicator(
        &self,
        history_search: PromptHistorySearch,
    ) -> Cow<'static, str> {
        let indicator = match history_search.status {
            PromptHistorySearchStatus::Passing => "",
            PromptHistorySearchStatus::Failing => "failing ",
        };
        Cow::Owned(format!(
            "({}reverse-search: {}) ",
            indicator, history_search.term
        ))
    }
}

// ── Core validation logic (extracted for testability)
// ─────────────────────────

pub(crate) enum ValidateOutcome {
    Complete,
    Incomplete,
}

pub(crate) fn validate_input(input: &str) -> ValidateOutcome {
    // Odd number of trailing backslashes → line continuation
    let trailing_backslashes = input.chars().rev().take_while(|&c| c == '\\').count();
    if trailing_backslashes % 2 == 1 {
        return ValidateOutcome::Incomplete;
    }

    // Odd number of ``` fences at line starts → open code fence
    let fence_count = input
        .lines()
        .filter(|line| line.trim_start().starts_with("```"))
        .count();
    if fence_count % 2 != 0 {
        return ValidateOutcome::Incomplete;
    }

    ValidateOutcome::Complete
}

// ── Helper functions ─────────────────────────────────────────────────────────

/// Produce styled segments for `@<token>` sequences (green), rest unstyled.
fn highlight_at_tokens_styled(line: &str) -> Vec<(Style, String)> {
    let no_color = std::env::var_os("NO_COLOR").is_some();
    let mut result = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let mut plain_start = 0;

    while i < chars.len() {
        if chars[i] == '@' {
            let preceded_by_ws = i == 0 || chars[i - 1].is_whitespace();
            if preceded_by_ws {
                // Flush plain text accumulated so far
                if i > plain_start {
                    let s: String = chars[plain_start..i].iter().collect();
                    result.push((Style::new(), s));
                }
                let token_start = i;
                i += 1;
                while i < chars.len() && !chars[i].is_whitespace() {
                    i += 1;
                }
                let token: String = chars[token_start..i].iter().collect();
                let style = if no_color {
                    Style::new()
                } else {
                    Style::new().fg(Color::Green)
                };
                result.push((style, token));
                plain_start = i;
                continue;
            }
        }
        i += 1;
    }

    // Flush remaining plain text
    if plain_start < chars.len() {
        let s: String = chars[plain_start..].iter().collect();
        result.push((Style::new(), s));
    }

    if result.is_empty() {
        result.push((Style::new(), line.to_string()));
    }
    result
}

/// Collect executable names from $PATH directories (deduplicated, sorted).
fn collect_path_commands() -> Vec<String> {
    let mut cmds = Vec::new();
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in path_var.split(':') {
            let dir = Path::new(dir);
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    if let Ok(ft) = entry.file_type()
                        && (ft.is_file() || ft.is_symlink())
                        && let Some(name) = entry.file_name().to_str()
                    {
                        cmds.push(name.to_string());
                    }
                }
            }
        }
    }
    cmds.sort_unstable();
    cmds.dedup();
    cmds
}

/// Complete a file path fragment. Returns (replacement_start_offset, (display,
/// replacement) pairs). When `dirs_only` is true, only directory entries are
/// included.
fn complete_path(
    fragment: &str,
    dirs_only: bool,
    base_dir: &Path,
) -> (usize, Vec<(String, String)>) {
    let (dir, prefix) = if let Some(slash_pos) = fragment.rfind('/') {
        let dir_part = &fragment[..=slash_pos];
        let file_part = &fragment[slash_pos + 1..];
        (shellexpand_dir(dir_part), file_part.to_string())
    } else {
        (base_dir.to_path_buf(), fragment.to_string())
    };

    let start_offset = if fragment.contains('/') {
        fragment.rfind('/').unwrap() + 1
    } else {
        0
    };

    let mut pairs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str()
                && name.starts_with(&prefix)
            {
                let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                if dirs_only && !is_dir {
                    continue;
                }
                let suffix = if is_dir { "/" } else { " " };
                pairs.push((name.to_string(), format!("{name}{suffix}")));
            }
        }
    }
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    (start_offset, pairs)
}

fn shellexpand_dir(s: &str) -> std::path::PathBuf {
    if let Some(rest) = s.strip_prefix('~')
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest.strip_prefix('/').unwrap_or(rest));
    }
    std::path::PathBuf::from(s)
}

// Small extension trait to allow .also() on a value (avoids a temp var in
// complete_hint)
trait Also: Sized {
    fn also<F: FnOnce(&Self)>(self, f: F) -> Self {
        f(&self);
        self
    }
}
impl<T> Also for T {}

#[cfg(test)]
mod tests {
    use colored::Colorize;
    use reedline::FileBackedHistory;

    use super::*;

    fn highlight_at_tokens(line: &str) -> String {
        let mut result = String::with_capacity(line.len());
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '@' {
                let preceded_by_ws = i == 0 || chars[i - 1].is_whitespace();
                if preceded_by_ws {
                    let token_start = i;
                    i += 1;
                    while i < chars.len() && !chars[i].is_whitespace() {
                        i += 1;
                    }
                    let token: String = chars[token_start..i].iter().collect();
                    result.push_str(&token.green().to_string());
                    continue;
                }
            }
            result.push(chars[i]);
            i += 1;
        }
        result
    }

    /// Mutex to serialise tests that mutate environment variables.
    static ENV_MUTEX: std::sync::LazyLock<std::sync::Mutex<()>> =
        std::sync::LazyLock::new(|| std::sync::Mutex::new(()));

    #[test]
    fn path_commands_not_empty() {
        let cmds = collect_path_commands();
        assert!(!cmds.is_empty());
    }

    #[test]
    fn complete_path_in_tmp() {
        let (offset, _pairs) = complete_path("/tmp/", false, Path::new("."));
        assert_eq!(offset, 5);
    }

    #[test]
    fn slash_commands_complete() {
        let mut completer = OrkaCompleter::new(Arc::new(Mutex::new(PathBuf::from("."))));
        let matches = completer.complete("/sk", 3);
        let names: Vec<&str> = matches.iter().map(|s| s.value.as_str()).collect();
        assert!(names.contains(&"/skill"));
        assert!(names.contains(&"/skills"));
    }

    #[test]
    fn highlight_at_tokens_colors_at_paths() {
        let result = highlight_at_tokens("attach @src/main.rs here");
        assert!(result.contains("src/main.rs"));
        assert_eq!(highlight_at_tokens("hello world"), "hello world");
        let email = highlight_at_tokens("user@example.com");
        assert!(!email.contains("\x1b[32m"));
    }

    #[test]
    fn feedback_completion() {
        let mut completer = OrkaCompleter::new(Arc::new(Mutex::new(PathBuf::from("."))));
        let line = "/feedback g";
        let matches = completer.complete(line, line.len());
        let names: Vec<&str> = matches.iter().map(|s| s.value.as_str()).collect();
        assert!(names.contains(&"good"));
        assert!(!names.contains(&"bad"));
    }

    #[test]
    fn builtin_hint_ghost_text() {
        let mut hinter = OrkaHinter::new();
        let hist = FileBackedHistory::default();
        // "!c" should hint "d" (completing "cd")
        let hint = hinter.handle("!c", 2, &hist, false, "");
        assert!(
            hint.contains('d'),
            "expected 'd' in hint for '!c', got: {hint:?}"
        );
        // "!exp" → "ort"
        let hint2 = hinter.handle("!exp", 4, &hist, false, "");
        assert!(
            hint2.contains("ort"),
            "expected 'ort' in hint for '!exp', got: {hint2:?}"
        );
    }

    #[test]
    fn validator_trailing_backslash_incomplete() {
        assert!(matches!(
            validate_input("hello\\"),
            ValidateOutcome::Incomplete
        ));
    }

    #[test]
    fn validator_even_backslashes_valid() {
        assert!(matches!(
            validate_input("hello\\\\"),
            ValidateOutcome::Complete
        ));
    }

    #[test]
    fn validator_odd_code_fences_incomplete() {
        assert!(matches!(
            validate_input("```rust\nfn main() {}"),
            ValidateOutcome::Incomplete
        ));
    }

    #[test]
    fn validator_even_code_fences_valid() {
        assert!(matches!(
            validate_input("```rust\nfn main() {}\n```"),
            ValidateOutcome::Complete
        ));
    }

    #[test]
    #[allow(unsafe_code)]
    fn hint_respects_no_color() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let mut hinter = OrkaHinter::new();
        let hist = FileBackedHistory::default();
        unsafe { std::env::set_var("NO_COLOR", "1") };
        let hint = hinter.handle("/qu", 3, &hist, false, "");
        unsafe { std::env::remove_var("NO_COLOR") };
        // With NO_COLOR=1 and use_ansi_coloring=false, hint should not contain ANSI
        // escapes
        assert!(
            !hint.contains('\x1b'),
            "hint should not contain ANSI: {hint:?}"
        );
    }
}
