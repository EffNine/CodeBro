//! Adapter between CodeBro's input semantics and the upstream
//! `xai-ratatui-textarea` widget.
//!
//! The textarea owns all text-editing state (cursor, selection, undo,
//! wrapping, grapheme safety). CodeBro retains authority over:
//!
//! - Task submission (Enter / Ctrl+Enter)
//! - Global shortcuts (Ctrl+P, Ctrl+C without selection, etc.)
//! - Slash-command autocomplete navigation
//! - Secure-input mode
//! - History navigation (when autocomplete is not active)
//!
//! The adapter exposes a small surface so the rest of the TUI does not
//! need to know about `TextArea` internals.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use xai_ratatui_textarea::{ClipboardProvider, MouseAction, TextArea, TextAreaState};

use crate::tui::app::TuiApp;
use crate::tui::dashboard::Dashboard;

/// Result of processing a keyboard event through the adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputResult {
    /// The key was consumed by the textarea or by a higher-priority overlay.
    Consumed,
    /// The user pressed Enter (not Shift+Enter) and the input should be
    /// submitted. Only emitted when no autocomplete is active and secure
    /// input is not in progress.
    Submit,
    /// The user pressed Ctrl+Enter: insert a newline into the textarea.
    Newline,
}

/// Result of processing a mouse event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseResult {
    Consumed,
    ScrollUp,
    ScrollDown,
}

/// Thin adapter around [`TextArea`].
#[derive(Debug)]
pub struct InputAdapter {
    inner: TextArea,
    state: TextAreaState,
}

impl InputAdapter {
    pub fn new() -> Self {
        Self {
            inner: TextArea::new(),
            state: TextAreaState::default(),
        }
    }

    // ── Text access ───────────────────────────────────────────────────────

    /// Returns the raw text buffer. Use this for submission, history storage,
    /// and session export — never render it directly; the widget handles
    /// wrapping and display projection.
    pub fn text(&self) -> &str {
        self.inner.text()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Clear the buffer and reset scroll state.
    pub fn clear(&mut self) {
        self.inner.set_text("");
        self.state = TextAreaState::default();
    }

    /// Insert raw text at the current cursor position. Used for bracketed
    /// paste and programmatic inserts.
    pub fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.inner.insert_str(text);
    }

    /// Set the full buffer (e.g. when restoring from history).
    pub fn set_text(&mut self, text: &str) {
        self.inner.set_text(text);
        // Move cursor to end so subsequent edits append rather than insert
        // at the beginning (the textarea's set_text preserves the existing
        // cursor position, clamped to [0, len]).
        self.inner.set_cursor(text.len());
        self.state = TextAreaState::default();
    }

    // ── Selection ─────────────────────────────────────────────────────────

    /// Whether there is an active selection.
    pub fn has_selection(&self) -> bool {
        self.inner.selection_range().is_some()
    }

    /// Returns the currently selected text, if any.
    pub fn selected_text(&self) -> Option<String> {
        self.inner.selected_text()
    }

    /// Take clipboard text after a mouse-driven selection was finished.
    /// Returns `None` if nothing was copied.
    pub fn take_clipboard(&mut self) -> Option<String> {
        self.inner.take_clipboard()
    }

    // ── Keyboard ──────────────────────────────────────────────────────────

    /// Process a key event. Returns `InputResult` describing what the host
    /// should do next.
    ///
    /// Shortcuts that belong to CodeBro are intercepted before reaching the
    /// textarea. When autocomplete is active, arrow keys navigate the list
    /// instead of moving the cursor. Secure input is handled by the caller
    /// and this method is never invoked while it is active.
    pub fn handle_key(&mut self, key: KeyEvent, dashboard: &Dashboard) -> InputResult {
        // Autocomplete owns arrow / page / home / end / enter / esc while open.
        if !dashboard.autocomplete.is_empty() {
            return InputResult::Consumed;
        }

        match key.kind {
            KeyEventKind::Press => {}
            _ => return InputResult::Consumed,
        }

        // Intercept CodeBro global shortcuts.
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            // Ctrl+C with selection → copy; without selection → cancel (handled by caller).
            if matches!(key.code, KeyCode::Char('c')) {
                return InputResult::Consumed;
            }
            // Ctrl+P → command palette (handled by caller).
            if matches!(key.code, KeyCode::Char('p')) {
                return InputResult::Consumed;
            }
            // Ctrl+Enter → submit (handled by caller).
            if matches!(key.code, KeyCode::Enter) {
                return InputResult::Submit;
            }
            // Ctrl+V → paste from system clipboard.
            if matches!(key.code, KeyCode::Char('v')) {
                self.inner.input(key);
                return InputResult::Consumed;
            }
            // All other Ctrl+ shortcuts belong to CodeBro.
            return InputResult::Consumed;
        }

        match key.code {
            KeyCode::Enter => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.inner.input(key);
                    InputResult::Consumed
                } else {
                    InputResult::Submit
                }
            }
            KeyCode::Up | KeyCode::Down => {
                if self.text().is_empty() {
                    InputResult::Consumed
                } else {
                    self.inner.input(key);
                    InputResult::Consumed
                }
            }
            KeyCode::Tab => InputResult::Consumed,
            KeyCode::Esc => {
                if self.has_selection() {
                    self.inner.clear_selection();
                }
                InputResult::Consumed
            }
            _ => {
                self.inner.input(key);
                InputResult::Consumed
            }
        }
    }

    // ── Mouse ─────────────────────────────────────────────────────────────

    /// Process a mouse event. The caller must pass the textarea's render
    /// area so screen→buffer coordinate mapping is correct.
    pub fn handle_mouse(&mut self, event: MouseEvent, area: Rect) -> MouseResult {
        use crossterm::event::MouseEventKind;
        match event.kind {
            MouseEventKind::ScrollDown => {
                let _ = self.inner.handle_mouse(event, area, self.state);
                MouseResult::Consumed
            }
            MouseEventKind::ScrollUp => {
                let _ = self.inner.handle_mouse(event, area, self.state);
                MouseResult::Consumed
            }
            _ => match self.inner.handle_mouse(event, area, self.state) {
                MouseAction::SelectionFinished => MouseResult::Consumed,
                _ => MouseResult::Consumed,
            },
        }
    }

    /// Check whether a mouse event falls within the textarea area.
    pub fn should_consume_mouse(&self, event: &MouseEvent, area: Rect) -> bool {
        event.row >= area.y && event.row < area.y + area.height
    }

    // ── Rendering ─────────────────────────────────────────────────────────

    /// Render the textarea into the buffer. The `area` must match the
    /// dimensions passed to [`handle_mouse`] so coordinate mapping stays
    /// consistent.
    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        use ratatui::widgets::WidgetRef as _;
        (&self.inner).render_ref(area, buf);
    }

    /// Update the internal scroll state after a render cycle.
    pub fn sync_state(&mut self) {
        // TextAreaState.scroll is updated inside render_ref; nothing extra
        // to do here beyond keeping the field in sync.
    }

    /// Get the current scroll offset for external use (e.g. cursor positioning).
    pub fn scroll(&self) -> u16 {
        self.state.scroll
    }

    /// Set the scroll offset explicitly (e.g. after a history restore).
    pub fn set_scroll(&mut self, scroll: u16) {
        self.state.scroll = scroll;
    }

    /// Byte cursor position in the buffer.
    pub fn cursor(&self) -> usize {
        self.inner.cursor()
    }

    /// Set the byte cursor position in the buffer.
    pub fn set_cursor(&mut self, pos: usize) {
        self.inner.set_cursor(pos);
    }

    #[cfg(test)]
    pub fn inner_mut(&mut self) -> &mut xai_ratatui_textarea::TextArea {
        &mut self.inner
    }

    /// Cursor position in screen coordinates, or `None` if off-screen.
    pub fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.inner.cursor_pos_with_state(area, self.state)
    }

    /// Byte length of the buffer text.
    pub fn len(&self) -> usize {
        self.inner.text().len()
    }

    /// Returns `true` if the buffer text starts with the given prefix.
    pub fn starts_with(&self, prefix: &str) -> bool {
        self.inner.text().starts_with(prefix)
    }

    /// Trim whitespace from the buffer text.
    pub fn trim(&self) -> &str {
        self.inner.text().trim()
    }
}

impl Default for InputAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn test_backspace_through_adapter() {
        let mut adapter = InputAdapter::new();
        adapter.set_text("hello");
        assert_eq!(adapter.text(), "hello");
        let bs = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
        adapter.inner_mut().input(bs);
        assert_eq!(adapter.text(), "hell", "backspace through inner_mut");
    }

    #[test]
    fn test_backspace_via_handle_key() {
        let mut app = crate::tui::TuiApp::new().expect("app");
        app.input.set_text("hello");
        let bs = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
        let result = app.input.handle_key(bs, &app.dashboard);
        assert_eq!(result, InputResult::Consumed);
        assert_eq!(app.input.text(), "hell", "backspace via handle_key");
    }
}
