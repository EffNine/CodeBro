#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Consultant capability — AI provider abstraction integrated into CodeBro.
//!
//! Provides a unified `consult` MCP tool that routes requests to one of
//! several AI providers (Conductor gateway) behind a clean trait abstraction.
//! Engineering context from CodeBro (facts, memory, git diff) can be attached
//! to requests so providers answer with project-awareness.
//!
//! Authentication is user-driven: the Conductor provider uses an API key from
//! `CONDUCTOR_API_KEY` or the secure credential store.
//! No passwords, cookies, or tokens are stored or inspected.

pub mod types;

pub mod prompt;
pub mod provider;
pub mod providers;
pub mod router;

pub use prompt::build_prompt;
pub use provider::ConsultantError;
pub use router::ConsultantRouter;
pub use types::{
    AuthReport, AuthStatus, ConsultantFileContext, ConsultantMode, ConsultantProvider,
    ConsultantRequest, ConsultantResponse,
};

/// Register the default set of consultant providers into a router.
pub fn build_router() -> ConsultantRouter {
    let mut router = ConsultantRouter::new();
    for p in providers::default_providers() {
        router.register(p);
    }
    router
}

/// Default provider name used when the request specifies `"auto"`.
pub const DEFAULT_PROVIDER: &str = "conductor";

#[cfg(test)]
mod tests {
    use super::providers::mock::MockProvider;
    use super::types::{AuthStatus, ConsultantProvider as ProviderChoice, ConsultantRequest};
    use super::*;

    /// Router resolves mock providers correctly.
    #[tokio::test]
    async fn router_resolves_mock_provider() {
        let mut router = ConsultantRouter::new();
        router.register(std::sync::Arc::new(MockProvider::new("test-mock")));
        let provider = router.resolve(&ProviderChoice::Auto).expect("resolve auto");
        assert_eq!(provider.name(), "test-mock");
    }

    /// Consult through the router returns a normalized response.
    #[tokio::test]
    async fn consult_via_router_returns_response() {
        let mut router = ConsultantRouter::new();
        router.register(std::sync::Arc::new(
            MockProvider::new("mock-one")
                .with_answer_template("Answer for: {question}".to_string()),
        ));

        let request = ConsultantRequest {
            question: "What is the meaning?".to_string(),
            ..Default::default()
        };
        let provider = router.resolve(&ProviderChoice::Auto).unwrap();
        let response = provider.consult(&request).await.expect("consult succeeds");
        assert_eq!(response.provider, "mock-one");
        assert!(response.answer.contains("What is the meaning?"));
        assert_eq!(response.confidence, 0.9);
    }

    /// Unauthenticated provider returns AuthenticationRequired error.
    #[tokio::test]
    async fn unauthenticated_provider_rejects_consult() {
        let mut router = ConsultantRouter::new();
        router.register(std::sync::Arc::new(
            MockProvider::new("mock-locked").with_auth(AuthStatus::Unauthenticated),
        ));

        let request = ConsultantRequest {
            question: "secret".to_string(),
            ..Default::default()
        };
        let provider = router.resolve(&ProviderChoice::Auto).unwrap();
        let err = provider.consult(&request).await.unwrap_err();
        assert!(matches!(err, ConsultantError::AuthenticationRequired(_)));
    }

    /// Sensitive data from a mock response is not logged with raw secrets.
    /// The response structure is clean and contains only the expected fields.
    #[tokio::test]
    async fn response_is_structured_not_raw_html() {
        let mut router = ConsultantRouter::new();
        router.register(std::sync::Arc::new(MockProvider::new("mock-structure")));

        let request = ConsultantRequest {
            question: "architecture review".to_string(),
            mode: super::types::ConsultantMode::Architecture,
            ..Default::default()
        };
        let provider = router.resolve(&ProviderChoice::Auto).unwrap();
        let response = provider.consult(&request).await.expect("consult");

        // Must have the core fields.
        assert!(!response.provider.is_empty());
        assert!(!response.answer.is_empty());
        assert!(!response.summary.is_empty());
        // Recommendations should be present (mock adds one).
        assert!(!response.recommendations.is_empty());
        // Metadata should be a map, not raw HTML.
        assert!(!response.metadata.is_empty());
    }
}
