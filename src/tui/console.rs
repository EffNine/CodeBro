//! Live PTY console for the TUI.
//!
//! A `PtyConsole` is the append-only, bounded, scrollable buffer that renders
//! a task's PTY output. It satisfies the Live Task Console contract:
//!
//! - output is append-only, never replaced or cleared;
//! - ANSI color/control sequences are preserved and rendered;
//! - output remains available after the task completes;
//! - the engineer can scroll and select any line without interrupting the
//!   running process;
//! - the buffer is bounded so long-running output cannot grow memory without
//!   limit.

use std::collections::VecDeque;
use std::time::Instant;

use ansi_to_tui::IntoText;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use crate::tui::ui::truncate_to;

/// Maximum characters retained per console. Keeps the buffer bounded while
/// preserving a long scrollback for inspection.
pub const MAX_CONSOLE_CHARS: usize = 512_000;

/// The status of a PTY-backed process in the console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleStatus {
    /// The process is still running; output is live.
    Running,
    /// The process exited with the given code (0 = success).
    Exited { exit_code: i32 },
    /// The task was cancelled by the user (Ctrl+C).
    Cancelled,
    /// The process exceeded its timeout.
    TimedOut,
    /// The task failed to start or hit a fatal error.
    Error,
}

impl ConsoleStatus {
    /// A short, muted status label for the console header line.
    pub fn label(&self) -> &'static str {
        match self {
            ConsoleStatus::Running => "running",
            ConsoleStatus::Exited { exit_code } => {
                if *exit_code == 0 {
                    "passed"
                } else {
                    "failed"
                }
            }
            ConsoleStatus::Cancelled => "cancelled",
            ConsoleStatus::TimedOut => "timed out",
            ConsoleStatus::Error => "error",
        }
    }

    pub fn color(&self) -> Color {
        match self {
            ConsoleStatus::Running => Color::Blue,
            ConsoleStatus::Exited { exit_code } => {
                if *exit_code == 0 {
                    Color::Green
                } else {
                    Color::Red
                }
            }
            ConsoleStatus::Cancelled => Color::Yellow,
            ConsoleStatus::TimedOut => Color::Yellow,
            ConsoleStatus::Error => Color::Red,
        }
    }
}

/// An append-only, bounded console for one task's PTY output.
#[derive(Debug, Clone)]
pub struct PtyConsole {
    /// Stable identifier used by PTY events to route output here.
    pub id: String,
    /// Human-readable label (e.g. the command line).
    pub label: String,
    chunks: VecDeque<String>,
    total_chars: usize,
    /// Current process status.
    pub status: ConsoleStatus,
    /// When the process was started.
    pub started: Instant,
}

impl PtyConsole {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        PtyConsole {
            id: id.into(),
            label: label.into(),
            chunks: VecDeque::new(),
            total_chars: 0,
            status: ConsoleStatus::Running,
            started: Instant::now(),
        }
    }

    /// Append a live output chunk. Drops the oldest chunks once the buffer cap
    /// is exceeded so memory stays bounded while scrollback stays available.
    pub fn append(&mut self, content: &str) {
        let len = content.chars().count();
        self.chunks.push_back(content.to_string());
        self.total_chars += len;
        while self.total_chars > MAX_CONSOLE_CHARS && self.chunks.len() > 1 {
            if let Some(dropped) = self.chunks.pop_front() {
                self.total_chars = self.total_chars.saturating_sub(dropped.chars().count());
            }
        }
    }

    /// Mark the process as finished with the given status.
    pub fn finish(&mut self, status: ConsoleStatus) {
        self.status = status;
    }

    /// Whether the console is still receiving live output.
    pub fn is_running(&self) -> bool {
        self.status == ConsoleStatus::Running
    }

    /// The full buffered output as one string (bounded).
    pub fn text(&self) -> String {
        let mut out = String::new();
        for chunk in &self.chunks {
            out.push_str(chunk);
        }
        out
    }

    /// Total characters retained.
    pub fn len_chars(&self) -> usize {
        self.total_chars
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// Render the console content into ratatui lines, preserving ANSI.
    /// Lines are wrapped to `width` characters so nothing overflows.
    pub fn render_lines(&self, width: usize) -> Vec<Line<'static>> {
        let text = self.text();
        if text.is_empty() {
            return Vec::new();
        }
        match text.into_text() {
            Ok(parsed) => {
                let mut lines = Vec::new();
                for line in parsed.lines {
                    let spans: Vec<Span<'static>> = line
                        .spans
                        .into_iter()
                        .map(|s| Span::styled(s.content, s.style))
                        .collect();
                    let content_len: usize = spans.iter().map(|s| s.content.chars().count()).sum();
                    if content_len > width && width > 0 {
                        for wrapped in wrap_spans(&spans, width) {
                            lines.push(wrapped);
                        }
                    } else {
                        lines.push(Line::from(spans));
                    }
                }
                lines
            }
            Err(_) => text
                .lines()
                .map(|l| {
                    Line::from(Span::styled(
                        crate::tui::ui::truncate_to(l, width),
                        Style::default().fg(Color::Gray),
                    ))
                })
                .collect(),
        }
    }
}

/// Wrap a line of styled spans to `width` columns, keeping styles per span.
fn wrap_spans(spans: &[Span<'static>], width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut current_len = 0usize;
    for span in spans {
        let content = span.content.clone();
        for ch in content.chars() {
            if current_len >= width {
                lines.push(Line::from(std::mem::take(&mut current)));
                current_len = 0;
            }
            let s = span.style.clone();
            current.push(Span::styled(ch.to_string(), s));
            current_len += 1;
        }
    }
    if !current.is_empty() {
        lines.push(Line::from(current));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_console_append_and_text() {
        let mut c = PtyConsole::new("c1", "cargo test");
        assert!(c.is_empty());
        c.append("running…\n");
        c.append("ok\n");
        assert_eq!(c.text(), "running…\nok\n");
        assert!(!c.is_empty());
    }

    #[test]
    fn test_console_finish_status() {
        let mut c = PtyConsole::new("c1", "cmd");
        assert!(c.is_running());
        c.finish(ConsoleStatus::Exited { exit_code: 0 });
        assert!(!c.is_running());
        assert_eq!(c.status.label(), "passed");
        c.finish(ConsoleStatus::Exited { exit_code: 2 });
        assert_eq!(c.status.label(), "failed");
        c.finish(ConsoleStatus::Cancelled);
        assert_eq!(c.status.label(), "cancelled");
    }

    #[test]
    fn test_console_buffer_is_bounded() {
        let mut c = PtyConsole::new("c1", "cmd");
        // Exceed the cap with many chunks.
        let big = "x".repeat(10_000);
        for _ in 0..100 {
            c.append(&big);
        }
        assert!(
            c.len_chars() <= MAX_CONSOLE_CHARS,
            "buffer exceeded cap: {}",
            c.len_chars()
        );
        assert!(c.chunks.len() > 1);
    }

    #[test]
    fn test_console_render_preserves_ansi() {
        let mut c = PtyConsole::new("c1", "cmd");
        c.append("\x1b[31mred\x1b[0m\n");
        let lines = c.render_lines(80);
        assert!(!lines.is_empty());
        // The ANSI content survives (rendered as a red span).
        let joined = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .fold(String::new(), |acc, s| format!("{}{}", acc, s.content));
        assert!(joined.contains("red"));
    }

    #[test]
    fn test_console_render_wraps_long_lines() {
        let mut c = PtyConsole::new("c1", "cmd");
        c.append(&"x".repeat(200));
        let lines = c.render_lines(20);
        let total: usize = lines.iter().map(|l| l.width()).sum();
        assert!(total >= 200, "wrapped lines must keep content: {}", total);
        assert!(lines.len() > 1);
    }

    #[test]
    fn test_console_stress_stays_bounded_preserves_recent() {
        let mut c = PtyConsole::new("c1", "stress");
        let chunk = "0123456789abcdef".repeat(10_000); // 160k chars
        for _ in 0..500 {
            c.append(&chunk);
        }
        assert!(
            c.len_chars() <= MAX_CONSOLE_CHARS,
            "buffer grew unbounded: {}",
            c.len_chars()
        );
        let text = c.text();
        assert!(
            text.chars().count() <= MAX_CONSOLE_CHARS,
            "text() must stay bounded: {}",
            text.chars().count()
        );
        // Ring-buffer semantics: the most recent output survives eviction.
        assert!(
            text.ends_with("0123456789abcdef"),
            "recent output must be preserved after eviction"
        );
    }

    #[test]
    fn test_console_append_does_not_break_ansi_lifecycle() {
        // Truncation must not corrupt the render path: after heavy eviction the
        // console still renders and finishes cleanly.
        let mut c = PtyConsole::new("c1", "cmd");
        let big = "x".repeat(20_000);
        for _ in 0..100 {
            c.append(&big);
        }
        c.finish(ConsoleStatus::Exited { exit_code: 0 });
        let lines = c.render_lines(80);
        assert!(!lines.is_empty());
        assert_eq!(c.status.label(), "passed");
    }
}
