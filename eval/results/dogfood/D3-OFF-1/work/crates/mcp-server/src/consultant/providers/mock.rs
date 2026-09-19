#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Mock consultant provider for testing.
//!
//! Returns deterministic, predictable answers so tests can verify the full
//! MCP → router → provider → response pipeline without network access.

use anyhow::Result;
use async_trait::async_trait;

use super::super::provider::{ConsultantError, ConsultantProvider};
use super::super::types::{AuthStatus, ConsultantRequest, ConsultantResponse};

#[derive(Clone)]
pub struct MockProvider {
    name: String,
    auth_status: AuthStatus,
    /// Optional custom answer template. Supports `{question}` placeholder.
    answer_template: String,
}

impl MockProvider {
    pub fn new(name: impl Into<String>) -> Self {
        MockProvider {
            name: name.into(),
            auth_status: AuthStatus::Authenticated,
            answer_template: "Mock answer from {provider} for: {question}".to_string(),
        }
    }

    pub fn with_auth(mut self, status: AuthStatus) -> Self {
        self.auth_status = status;
        self
    }

    pub fn with_answer_template(mut self, template: impl Into<String>) -> Self {
        self.answer_template = template.into();
        self
    }
}

#[async_trait]
impl ConsultantProvider for MockProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn consult(
        &self,
        request: &ConsultantRequest,
    ) -> Result<ConsultantResponse, ConsultantError> {
        if !matches!(self.auth_status, AuthStatus::Authenticated) {
            return Err(ConsultantError::AuthenticationRequired(
                self.unauthenticated_hint(),
            ));
        }

        let answer = self
            .answer_template
            .replace("{provider}", &self.name)
            .replace("{question}", &request.question);

        let summary = answer.chars().take(100).collect::<String>();
        Ok(ConsultantResponse {
            provider: self.name.clone(),
            model: "mock/v1".to_string(),
            answer,
            summary,
            recommendations: vec![format!("Mock recommendation for {} mode", request.mode)],
            risks: vec![],
            confidence: 0.9,
            metadata: {
                let mut map = serde_json::Map::new();
                map.insert(
                    "mode".to_string(),
                    serde_json::Value::String(request.mode.to_string()),
                );
                map.insert(
                    "provider".to_string(),
                    serde_json::Value::String(self.name.clone()),
                );
                map
            },
        })
    }

    fn auth_status(&self) -> AuthStatus {
        self.auth_status.clone()
    }

    fn login_url(&self) -> &str {
        "https://example.com/mock-login"
    }
}
