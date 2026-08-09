//! Cooperative task cancellation.
//!
//! A `CancellationToken` is a cheap, shared, non-blocking flag that the TUI
//! flips when the user presses `Ctrl+C`. Long-running work (LLM streaming,
//! PTY processes, verification) polls `is_cancelled()` between steps and
//! stops promptly. This is the single cancellation authority for a task; it is
//! not a general-purpose async primitive and never blocks the event loop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// A shared cancellation flag for one in-flight task.
#[derive(Clone, Debug)]
pub struct CancellationToken {
    inner: Arc<AtomicBool>,
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

impl CancellationToken {
    /// Create a new, un-cancelled token.
    pub fn new() -> Self {
        CancellationToken {
            inner: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Signal cancellation. Idempotent.
    pub fn cancel(&self) {
        self.inner.store(true, Ordering::SeqCst);
    }

    /// Whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.inner.load(Ordering::SeqCst)
    }

    /// Reset the token to un-cancelled. Used when reusing a token slot for a
    /// new task after a completed/cancelled one.
    pub fn reset(&self) {
        self.inner.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_starts_uncancelled() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
    }

    #[test]
    fn test_token_cancel() {
        let token = CancellationToken::new();
        token.cancel();
        assert!(token.is_cancelled());
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn test_token_reset() {
        let token = CancellationToken::new();
        token.cancel();
        assert!(token.is_cancelled());
        token.reset();
        assert!(!token.is_cancelled());
    }

    #[test]
    fn test_token_clone_shares_state() {
        let token = CancellationToken::new();
        let clone = token.clone();
        clone.cancel();
        assert!(token.is_cancelled());
    }
}
