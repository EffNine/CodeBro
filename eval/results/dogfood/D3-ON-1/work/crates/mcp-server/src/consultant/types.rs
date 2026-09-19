#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Consultant request and response types.
//!
//! These are the structured types exchanged between the MCP tool handler and
//! the provider abstraction. All fields are optional except `question` so the
//! request model can be extended progressively without breaking existing calls.

use serde::{Deserialize, Serialize};

/// The AI provider to consult.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ConsultantProvider {
    #[default]
    Auto,
    #[serde(rename = "conductor")]
    Conductor,
}

impl std::fmt::Display for ConsultantProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConsultantProvider::Auto => write!(f, "auto"),
            ConsultantProvider::Conductor => write!(f, "conductor"),
        }
    }
}

/// The consultation mode — shapes the provider's prompting strategy.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ConsultantMode {
    #[default]
    Architecture,
    Debugging,
    CodeReview,
    Planning,
    Research,
    SecondOpinion,
}

impl std::fmt::Display for ConsultantMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConsultantMode::Architecture => write!(f, "architecture"),
            ConsultantMode::Debugging => write!(f, "debugging"),
            ConsultantMode::CodeReview => write!(f, "code_review"),
            ConsultantMode::Planning => write!(f, "planning"),
            ConsultantMode::Research => write!(f, "research"),
            ConsultantMode::SecondOpinion => write!(f, "second_opinion"),
        }
    }
}

/// A single file context entry attached to a consultation request.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConsultantFileContext {
    /// Path relative to the workspace root.
    pub path: String,
    /// File contents (truncated server-side if very large).
    pub content: String,
}

/// The full consultation request.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ConsultantRequest {
    /// Provider to use. `"auto"` selects the first authenticated provider.
    #[serde(default)]
    pub provider: ConsultantProvider,
    /// Consultation mode.
    #[serde(default)]
    pub mode: ConsultantMode,
    /// The question or task to consult on.
    #[serde(default)]
    pub question: String,
    /// Optional explicit context text (supplements automatic context).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// Optional explicit file contexts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<ConsultantFileContext>,
    /// Whether to include the current git diff in the context.
    #[serde(default)]
    pub include_git_diff: bool,
    /// Whether to include project facts and memory in the context.
    #[serde(default)]
    pub include_project_context: bool,
    /// Maximum answer length in characters (0 = provider default).
    #[serde(default)]
    pub max_answer_length: usize,
}

/// Normalized consultation response from any provider.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConsultantResponse {
    /// The provider that answered.
    pub provider: String,
    /// The model or service name used.
    #[serde(default)]
    pub model: String,
    /// The full answer text.
    pub answer: String,
    /// Short executive summary (derived from answer when available).
    #[serde(default)]
    pub summary: String,
    /// Actionable recommendations, when the provider produced any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recommendations: Vec<String>,
    /// Potential risks or concerns raised.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub risks: Vec<String>,
    /// Provider-assigned confidence in [0.0, 1.0].
    #[serde(default)]
    pub confidence: f64,
    /// Additional provider-specific metadata.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub metadata: serde_json::Map<String, serde_json::Value>,
}

impl ConsultantResponse {
    /// Build a response from a plain-text answer with sensible defaults.
    pub fn simple(provider: impl Into<String>, answer: impl Into<String>) -> Self {
        let answer = answer.into();
        let first_line = answer
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .chars()
            .take(200)
            .collect::<String>();
        ConsultantResponse {
            provider: provider.into(),
            model: String::new(),
            answer,
            summary: first_line,
            recommendations: Vec::new(),
            risks: Vec::new(),
            confidence: 0.5,
            metadata: serde_json::Map::new(),
        }
    }
}

/// Authentication status for a single provider.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub enum AuthStatus {
    /// Session is authenticated and ready.
    Authenticated,
    /// A session exists but appears expired; re-authentication recommended.
    Expired,
    /// No session found; run `codebro auth login <provider>` first.
    Unauthenticated,
}

impl std::fmt::Display for AuthStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthStatus::Authenticated => write!(f, "authenticated"),
            AuthStatus::Expired => write!(f, "expired"),
            AuthStatus::Unauthenticated => write!(f, "unauthenticated"),
        }
    }
}

/// Status report returned by `auth status`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthReport {
    pub provider: String,
    pub status: AuthStatus,
    pub login_url: String,
    pub session_age_seconds: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_display_values() {
        assert_eq!(ConsultantProvider::Auto.to_string(), "auto");
        assert_eq!(ConsultantProvider::Conductor.to_string(), "conductor");
    }

    #[test]
    fn mode_display_values() {
        assert_eq!(ConsultantMode::Architecture.to_string(), "architecture");
        assert_eq!(ConsultantMode::Debugging.to_string(), "debugging");
        assert_eq!(ConsultantMode::CodeReview.to_string(), "code_review");
        assert_eq!(ConsultantMode::Planning.to_string(), "planning");
        assert_eq!(ConsultantMode::Research.to_string(), "research");
        assert_eq!(ConsultantMode::SecondOpinion.to_string(), "second_opinion");
    }

    #[test]
    fn auth_status_display() {
        assert_eq!(AuthStatus::Authenticated.to_string(), "authenticated");
        assert_eq!(AuthStatus::Expired.to_string(), "expired");
        assert_eq!(AuthStatus::Unauthenticated.to_string(), "unauthenticated");
    }

    #[test]
    fn consultant_response_simple_has_summary_from_first_line() {
        let resp = ConsultantResponse::simple("mock", "This is the answer.\n\nMore details here.");
        assert_eq!(resp.provider, "mock");
        assert_eq!(resp.summary, "This is the answer.");
        assert!(resp.recommendations.is_empty());
        assert!(resp.risks.is_empty());
        assert!(resp.metadata.is_empty());
    }

    #[test]
    fn request_serializes_roundtrip() {
        let req = ConsultantRequest {
            provider: ConsultantProvider::Conductor,
            mode: ConsultantMode::Architecture,
            question: "Should we split the runtime?".to_string(),
            context: Some("some context".to_string()),
            files: vec![],
            include_git_diff: true,
            include_project_context: true,
            max_answer_length: 2000,
        };
        let json = serde_json::to_string(&req).expect("serialize");
        let decoded: ConsultantRequest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.provider, ConsultantProvider::Conductor);
        assert_eq!(decoded.mode, ConsultantMode::Architecture);
        assert_eq!(decoded.question, "Should we split the runtime?");
        assert!(decoded.include_git_diff);
        assert!(decoded.include_project_context);
        assert_eq!(decoded.max_answer_length, 2000);
    }

    #[test]
    fn request_defaults_are_sane() {
        let req: ConsultantRequest = serde_json::from_str("{}").expect("parse empty");
        assert_eq!(req.provider, ConsultantProvider::Auto);
        assert_eq!(req.mode, ConsultantMode::Architecture);
        assert!(req.question.is_empty());
        assert!(!req.include_git_diff);
        assert!(!req.include_project_context);
    }
}
