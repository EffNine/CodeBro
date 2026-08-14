//! CodeBro TUI Adapter — bridges OpenCode-derived TUI to CodeBro backend
//!
//! Protocol: newline-delimited JSON over stdio
//!   TUI → Backend: {"id": N, "cmd": "session.list", "payload": {...}}
//!   Backend → TUI: {"id": N, "result": {...}} or {"id": N, "error": "..."}
//!   Backend → TUI: {"event": {"type": "session.next.text.delta", ...}}

pub mod bridge;
pub mod handlers;
pub mod protocol;

pub use bridge::TuiBridge;
