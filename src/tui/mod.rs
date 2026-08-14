#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
pub mod abstractions;
pub mod actions;
pub mod animation;
pub mod app;
pub mod commands;
pub mod console;
pub mod dashboard;
pub mod diff_view;
pub mod events;
pub mod markdown;
pub mod textarea_adapter;
pub mod theme;
pub mod ui;

/// Re-exported from the agent runtime. The ReAct tool-call parser is owned by
/// the agent layer, not the TUI.
pub use crate::agent::tool_parser;

pub use app::TuiApp;
pub use ui::run;
