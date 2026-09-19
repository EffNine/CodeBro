#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Provider abstraction for the Consultant capability.
//!
//! Every concrete provider implements this trait. The MCP tool and router talk
//! only to this trait, never to a concrete provider directly.

use anyhow::Result;

use super::types::{AuthStatus, ConsultantRequest, ConsultantResponse};

/// Core provider trait. Implement this to add a new consultant provider.
#[async_trait::async_trait]
pub trait ConsultantProvider: Send + Sync {
    /// Human-readable provider name (e.g. `"conductor"`).
    fn name(&self) -> &str;

    /// Run a consultation. Returns a normalized response.
    async fn consult(
        &self,
        request: &ConsultantRequest,
    ) -> Result<ConsultantResponse, ConsultantError>;

    /// Current authentication status for this provider.
    fn auth_status(&self) -> AuthStatus;

    /// URL the user should open to authenticate (printed by `auth login`).
    fn login_url(&self) -> &str;

    /// A short, actionable error message when the provider is not authenticated.
    fn unauthenticated_hint(&self) -> String {
        format!(
            "{} is not authenticated. Run `codebro auth login {}` to start the login flow.",
            self.name().to_ascii_uppercase(),
            self.name()
        )
    }
}

/// Error type specific to consultant operations.
#[derive(thiserror::Error, Debug)]
pub enum ConsultantError {
    #[error("authentication required: {0}")]
    AuthenticationRequired(String),

    #[error("provider error: {0}")]
    Provider(String),

    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("configuration error: {0}")]
    Config(String),
}

impl From<anyhow::Error> for ConsultantError {
    fn from(err: anyhow::Error) -> Self {
        ConsultantError::Provider(err.to_string())
    }
}
