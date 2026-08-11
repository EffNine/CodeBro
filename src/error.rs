#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CodeBroError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("Provider error: {0}")]
    Provider(String),

    #[error("Tool error: {0}")]
    Tool(String),

    #[error("Permission error: {0}")]
    Permission(String),

    #[error("Patch error: {0}")]
    Patch(String),

    #[error("Index error: {0}")]
    Index(String),

    #[error("Context error: {0}")]
    Context(String),

    #[error("Scan error: {0}")]
    Scan(String),

    #[error("Session error: {0}")]
    Session(String),

    #[error("Config error: {0}")]
    Config(String),
}

pub type Result<T> = std::result::Result<T, CodeBroError>;
