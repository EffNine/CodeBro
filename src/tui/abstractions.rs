//! Shared TUI abstractions for navigation, modal state, selection, and scrolling.
//!
//! These primitives are the single authority for their domains — every overlay,
//! picker, list, and input area reuses them rather than duplicating logic.

use ratatui::layout::Rect;

// ─── ModalState ────────────────────────────────────────────────────────────────

/// Which modal/overlay is currently focused. The stack preserves the previous
/// state so Esc restores the right layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModalState {
    /// No overlay is open; the chat/input area is active.
    None,
    /// Slash / // autocomplete dropdown is visible above the input.
    Autocomplete,
    /// Command palette (Ctrl+P) is open.
    CommandPalette,
    /// Provider picker (//provider) is open.
    ProviderPicker,
    /// Model picker (//model) is open.
    ModelPicker,
    /// Interactive settings panel is open.
    Settings,
    /// Live PTY console overlay (Ctrl+K / //console) is open.
    Console,
    /// Agents status panel (Ctrl+A) is open.
    Agents,
    /// Task graph panel (Ctrl+G) is open.
    TaskGraph,
    /// Metrics panel (Ctrl+E) is open.
    Metrics,
    /// Coordination panel is open.
    Coordination,
    /// Memory panel (Ctrl+M) is open.
    Memory,
    /// Trace/activity panel (Ctrl+T) is open.
    Trace,
    /// A destructive confirmation prompt is pending (y/n).
    Confirmation,
    /// Masked API-key input is active (highest priority).
    SecureInput,
}

impl ModalState {
    /// Whether any modal is currently open.
    pub fn is_some(&self) -> bool {
        !matches!(self, ModalState::None)
    }

    /// Priority ranking: higher values win key-handling precedence.
    pub fn priority(&self) -> u8 {
        match self {
            ModalState::None => 0,
            ModalState::Autocomplete => 10,
            ModalState::CommandPalette => 20,
            ModalState::ProviderPicker => 30,
            ModalState::ModelPicker => 35,
            ModalState::Settings => 40,
            ModalState::Console => 50,
            ModalState::Agents => 60,
            ModalState::TaskGraph => 60,
            ModalState::Metrics => 60,
            ModalState::Coordination => 60,
            ModalState::Memory => 60,
            ModalState::Trace => 60,
            ModalState::Confirmation => 70,
            ModalState::SecureInput => 100,
        }
    }

    /// Human-readable label for debugging.
    pub fn label(&self) -> &'static str {
        match self {
            ModalState::None => "none",
            ModalState::Autocomplete => "autocomplete",
            ModalState::CommandPalette => "command-palette",
            ModalState::ProviderPicker => "provider-picker",
            ModalState::ModelPicker => "model-picker",
            ModalState::Settings => "settings",
            ModalState::Console => "console",
            ModalState::Agents => "agents",
            ModalState::TaskGraph => "task-graph",
            ModalState::Metrics => "metrics",
            ModalState::Coordination => "coordination",
            ModalState::Memory => "memory",
            ModalState::Trace => "trace",
            ModalState::Confirmation => "confirmation",
            ModalState::SecureInput => "secure-input",
        }
    }
}

// ─── SelectionModel ────────────────────────────────────────────────────────────

/// A rect-like range in the virtual chat line buffer. Rows are 0-based within
/// the rendered chat lines; columns are grapheme-cluster positions on each row.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SelectionRange {
    pub start_row: u16,
    pub start_col: u16,
    pub end_row: u16,
    pub end_col: u16,
    /// Whether a selection has been initiated (start point set).
    pub active: bool,
}

impl SelectionRange {
    /// Whether a selection is currently active.
    pub fn is_some(&self) -> bool {
        self.active
    }

    /// Normalize so (start_row, start_col) <= (end_row, end_col) lexicographically.
    pub fn normalize(&mut self) {
        if (self.end_row, self.end_col) < (self.start_row, self.start_col) {
            std::mem::swap(&mut self.start_row, &mut self.end_row);
            std::mem::swap(&mut self.start_col, &mut self.end_col);
        }
    }

    /// Extract plain text from a list of virtual chat lines.
    pub fn extract(&self, lines: &[String]) -> String {
        if !self.is_some() {
            return String::new();
        }
        let start_row = self.start_row.min(self.end_row) as usize;
        let end_row = self.start_row.max(self.end_row) as usize;
        let start_col = self.start_col.min(self.end_col) as usize;
        let end_col = self.start_col.max(self.end_col) as usize;
        let mut out = String::new();
        for (i, line) in lines.iter().enumerate() {
            if i < start_row || i > end_row {
                continue;
            }
            let chars: Vec<char> = line.chars().collect();
            let row_start = if i == start_row { start_col } else { 0 };
            let row_end = if i == end_row {
                end_col.min(chars.len())
            } else {
                chars.len()
            };
            let segment: String = chars[row_start..row_end].iter().collect();
            if !segment.is_empty() {
                out.push_str(&segment);
            }
            if i < end_row {
                out.push('\n');
            }
        }
        out
    }
}

/// State for mouse-driven text selection in the chat viewport.
#[derive(Debug, Clone, Default)]
pub struct SelectionModel {
    pub range: SelectionRange,
    pub is_selecting: bool,
}

impl SelectionModel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_active(&self) -> bool {
        self.range.is_some() && !self.is_selecting
    }

    pub fn clear(&mut self) {
        self.range = SelectionRange::default();
        self.is_selecting = false;
    }

    /// Update the selection end point from a mouse event.
    pub fn update_end(&mut self, row: u16, col: u16) {
        if self.range.is_some() {
            self.range.end_row = row;
            self.range.end_col = col;
            self.range.normalize();
        }
    }

    /// Begin a new selection at (row, col).
    pub fn begin(&mut self, row: u16, col: u16) {
        self.range = SelectionRange {
            start_row: row,
            start_col: col,
            end_row: row,
            end_col: col,
            active: true,
        };
        self.is_selecting = true;
    }

    /// Finalize the selection (mouse button released).
    pub fn finish(&mut self) {
        self.is_selecting = false;
        // If start == end, clear the selection.
        if self.range.start_row == self.range.end_row && self.range.start_col == self.range.end_col
        {
            self.clear();
        }
    }
}

// ─── ScrollbackState ───────────────────────────────────────────────────────────

/// Follow mode for the chat scrollback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FollowMode {
    #[default]
    /// Always follow live content (auto-scroll to bottom).
    Follow,
    /// Stay detached once the user scrolls up; show an indicator.
    Detached,
}

/// State for the chat viewport scrollback.
#[derive(Debug, Clone)]
pub struct ScrollbackState {
    /// How many lines to scroll up from the bottom (0 = live view).
    pub offset_from_bottom: usize,
    /// Whether the viewport should auto-follow new content.
    pub follow_mode: FollowMode,
    /// Total number of virtual lines in the chat (updated each render).
    pub total_lines: usize,
}

impl Default for ScrollbackState {
    fn default() -> Self {
        Self {
            offset_from_bottom: 0,
            follow_mode: FollowMode::Follow,
            total_lines: 0,
        }
    }
}

impl ScrollbackState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the viewport is currently detached (user scrolled up).
    pub fn is_detached(&self) -> bool {
        self.offset_from_bottom > 0
    }

    /// Return to the live view (bottom).
    pub fn follow(&mut self) {
        self.offset_from_bottom = 0;
        self.follow_mode = FollowMode::Follow;
    }

    /// Detach from live view.
    pub fn detach(&mut self) {
        self.follow_mode = FollowMode::Detached;
    }

    /// Scroll up by `lines` (increase offset).
    pub fn scroll_up(&mut self, lines: usize) {
        self.offset_from_bottom = self.offset_from_bottom.saturating_add(lines);
        self.detach();
    }

    /// Scroll down by `lines` (decrease offset).
    pub fn scroll_down(&mut self, lines: usize) {
        self.offset_from_bottom = self.offset_from_bottom.saturating_sub(lines);
        if self.offset_from_bottom == 0 {
            self.follow_mode = FollowMode::Follow;
        }
    }

    /// Page up / page down.
    pub fn page_up(&mut self, page_size: usize) {
        self.scroll_up(page_size);
    }

    pub fn page_down(&mut self, page_size: usize) {
        self.scroll_down(page_size);
    }

    /// Jump to the top (maximum scroll).
    pub fn go_top(&mut self) {
        self.offset_from_bottom = self.total_lines;
        self.detach();
    }

    /// Jump to the bottom.
    pub fn go_bottom(&mut self) {
        self.follow();
    }

    /// Clamp offset to valid range based on total lines and viewport height.
    pub fn clamp(&mut self, viewport_height: usize) {
        let max_offset = self.total_lines.saturating_sub(viewport_height);
        self.offset_from_bottom = self.offset_from_bottom.min(max_offset);
    }

    /// Whether to show the "new activity" indicator.
    pub fn show_new_activity_indicator(&self) -> bool {
        self.is_detached() && self.total_lines > 0
    }
}

// ─── SelectableListState (re-export for convenience) ──────────────────────────

// The core SelectableListState is defined in dashboard.rs; this module provides
// the higher-level abstractions. We re-export the shared navigation helpers.
pub use crate::tui::dashboard::{
    nav_clamp, nav_page, nav_wrap, SelectableListState, NAV_PAGE_SIZE,
};

// ─── ClipboardProvider ─────────────────────────────────────────────────────────

/// Trait for clipboard operations. Implementations vary by platform.
pub trait ClipboardProvider: Send + Sync {
    fn copy(&self, text: &str) -> bool;
    fn paste(&self) -> Option<String>;
    fn name(&self) -> &str;
}

/// Platform-agnostic clipboard that tries available backends.
#[derive(Debug, Clone)]
pub struct SystemClipboard;

impl ClipboardProvider for SystemClipboard {
    fn copy(&self, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        for cmd in ["pbcopy", "xclip", "wl-copy"] {
            let mut c = std::process::Command::new(cmd);
            if cmd == "xclip" {
                c.arg("-selection").arg("clipboard");
            }
            let mut child = match c.stdin(std::process::Stdio::piped()).spawn() {
                Ok(ch) => ch,
                Err(_) => continue,
            };
            use std::io::Write;
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            if child.wait().map(|s| s.success()).unwrap_or(false) {
                return true;
            }
        }
        false
    }

    fn paste(&self) -> Option<String> {
        for cmd in ["pbpaste", "xclip", "wl-paste"] {
            let mut c = std::process::Command::new(cmd);
            if cmd == "xclip" {
                c.args(["-selection", "clipboard", "-o"]);
            }
            match c.output() {
                Ok(out) if out.status.success() => {
                    let text = String::from_utf8_lossy(&out.stdout);
                    let text = text.trim_end_matches('\n').to_string();
                    return Some(text);
                }
                _ => continue,
            }
        }
        None
    }

    fn name(&self) -> &str {
        "system"
    }
}

// ─── CommandMatcher ────────────────────────────────────────────────────────────

/// Fuzzy/prefix matcher for commands. Uses CodeBro's existing ranking.
#[derive(Debug, Clone)]
pub struct CommandMatcher {
    pub entries: Vec<(String, &'static str)>,
}

impl CommandMatcher {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Query the matcher with a filter string.
    pub fn query(&self, filter: &str) -> Vec<&(String, &'static str)> {
        if filter.is_empty() {
            return self.entries.iter().collect();
        }
        let f = filter.to_lowercase();
        self.entries
            .iter()
            .filter(|(cmd, desc)| {
                cmd.to_lowercase().contains(&f) || desc.to_lowercase().contains(&f)
            })
            .collect()
    }

    /// Register commands from the registry.
    pub fn register(
        &mut self,
        specs: impl Iterator<Item = &'static crate::tui::commands::CommandSpec>,
    ) {
        self.entries = specs
            .map(|s| (s.command.to_string(), s.description))
            .collect();
    }
}

impl Default for CommandMatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_selection_range_normalize() {
        let mut r = SelectionRange {
            start_row: 5,
            start_col: 10,
            end_row: 2,
            end_col: 3,
            active: true,
        };
        r.normalize();
        assert_eq!(r.start_row, 2);
        assert_eq!(r.start_col, 3);
        assert_eq!(r.end_row, 5);
        assert_eq!(r.end_col, 10);
    }

    #[test]
    fn test_selection_extract_basic() {
        let range = SelectionRange {
            start_row: 0,
            start_col: 0,
            end_row: 0,
            end_col: 5,
            active: true,
        };
        let lines = vec!["hello".to_string(), "world".to_string()];
        assert_eq!(range.extract(&lines), "hello");
    }

    #[test]
    fn test_selection_extract_multiline() {
        let range = SelectionRange {
            start_row: 1,
            start_col: 0,
            end_row: 2,
            end_col: 3,
            active: true,
        };
        let lines = vec![
            "header".to_string(),
            "line one".to_string(),
            "line two three".to_string(),
        ];
        let result = range.extract(&lines);
        assert!(result.contains("line one"));
        assert!(
            result.contains("lin"),
            "expected 'lin' from cols 0-3 of 'line two three'"
        );
    }

    #[test]
    fn test_scrollback_follow_detach() {
        let mut sb = ScrollbackState::new();
        assert!(!sb.is_detached());
        sb.scroll_up(5);
        assert!(sb.is_detached());
        sb.follow();
        assert!(!sb.is_detached());
    }

    #[test]
    fn test_scrollback_clamp() {
        let mut sb = ScrollbackState::new();
        sb.total_lines = 100;
        sb.clamp(20);
        assert_eq!(sb.offset_from_bottom, 0);
        sb.offset_from_bottom = 200;
        sb.clamp(20);
        assert_eq!(sb.offset_from_bottom, 80);
    }

    #[test]
    fn test_modal_state_priorities() {
        assert!(ModalState::SecureInput.priority() > ModalState::CommandPalette.priority());
        assert!(ModalState::CommandPalette.priority() > ModalState::Autocomplete.priority());
        assert!(ModalState::None.priority() == 0);
    }

    #[test]
    fn test_clipboard_headless() {
        let cb = SystemClipboard;
        // Empty copy should return false (early exit).
        assert!(!cb.copy(""));
        // On headless, paste returns None.
        assert!(cb.paste().is_none());
    }
}
