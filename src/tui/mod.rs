#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
pub mod animation;
pub mod app;
pub mod dashboard;
pub mod diff_view;
pub mod events;
pub mod markdown;
pub mod tool_parser;
pub mod ui;

pub use app::TuiApp;
pub use ui::run;
