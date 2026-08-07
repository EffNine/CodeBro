#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Error classification for the reliability layer.
//!
//! Provides structured runtime error categories that enable informed
//! recovery decisions, retry policies, and escalation paths.

use serde::{Deserialize, Serialize};

/// The category of a runtime error.
///
/// Each category has associated metadata:
/// - `is_retryable`: Whether the error can be recovered by retrying.
/// - `escalation_level`: How urgently the error needs attention (0-3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeErrorCategory {
    /// Provider timed out waiting for LLM response.
    ProviderTimeout,
    /// Provider returned a rate-limit error (429).
    ProviderRateLimit,
    /// Provider authentication failed (401).
    ProviderAuthFailure,
    /// Provider network error (connection refused, DNS failure, etc.).
    ProviderNetworkError,
    /// Tool execution timed out.
    ToolTimeout,
    /// Tool execution returned an error.
    ToolExecutionError,
    /// Tool requires permission that was denied.
    ToolPermissionDenied,
    /// System memory limit exceeded.
    SystemMemoryLimit,
    /// System shutdown requested.
    SystemShutdown,
    /// Operation was cancelled by the user.
    SystemCancellation,
    /// Unclassified error.
    Unknown,
}

impl RuntimeErrorCategory {
    /// Returns whether this error category can be recovered by retrying.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            RuntimeErrorCategory::ProviderTimeout
                | RuntimeErrorCategory::ProviderRateLimit
                | RuntimeErrorCategory::ProviderNetworkError
                | RuntimeErrorCategory::ToolTimeout
                | RuntimeErrorCategory::Unknown
        )
    }

    /// Returns the escalation level: 0 = informational, 3 = critical.
    pub fn escalation_level(&self) -> u32 {
        match self {
            RuntimeErrorCategory::ProviderTimeout => 1,
            RuntimeErrorCategory::ProviderRateLimit => 1,
            RuntimeErrorCategory::ProviderAuthFailure => 3,
            RuntimeErrorCategory::ProviderNetworkError => 2,
            RuntimeErrorCategory::ToolTimeout => 1,
            RuntimeErrorCategory::ToolExecutionError => 1,
            RuntimeErrorCategory::ToolPermissionDenied => 2,
            RuntimeErrorCategory::SystemMemoryLimit => 3,
            RuntimeErrorCategory::SystemShutdown => 2,
            RuntimeErrorCategory::SystemCancellation => 0,
            RuntimeErrorCategory::Unknown => 1,
        }
    }

    /// Returns a human-readable label for this category.
    pub fn label(&self) -> &'static str {
        match self {
            RuntimeErrorCategory::ProviderTimeout => "provider_timeout",
            RuntimeErrorCategory::ProviderRateLimit => "provider_rate_limit",
            RuntimeErrorCategory::ProviderAuthFailure => "provider_auth_failure",
            RuntimeErrorCategory::ProviderNetworkError => "provider_network_error",
            RuntimeErrorCategory::ToolTimeout => "tool_timeout",
            RuntimeErrorCategory::ToolExecutionError => "tool_execution_error",
            RuntimeErrorCategory::ToolPermissionDenied => "tool_permission_denied",
            RuntimeErrorCategory::SystemMemoryLimit => "system_memory_limit",
            RuntimeErrorCategory::SystemShutdown => "system_shutdown",
            RuntimeErrorCategory::SystemCancellation => "system_cancellation",
            RuntimeErrorCategory::Unknown => "unknown",
        }
    }
}

/// A structured runtime error that includes classification metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeError {
    /// The original error message.
    pub message: String,
    /// The classified error category.
    pub category: RuntimeErrorCategory,
    /// The source of the error (e.g., "provider", "tool:read_file", "system").
    pub source: String,
    /// The correlation ID for tracing.
    pub correlation_id: String,
}

impl RuntimeError {
    /// Creates a new runtime error with the given message, category, and source.
    /// Generates a correlation ID based on the source and category.
    pub fn new(message: &str, category: RuntimeErrorCategory, source: &str) -> Self {
        let label = category.label().to_string();
        RuntimeError {
            message: message.to_string(),
            category,
            source: source.to_string(),
            correlation_id: format!("{}-{}", source.replace(':', "-"), label),
        }
    }

    /// Returns whether this error can be recovered by retrying.
    pub fn is_retryable(&self) -> bool {
        self.category.is_retryable()
    }

    /// Returns the escalation level for this error.
    pub fn escalation_level(&self) -> u32 {
        self.category.escalation_level()
    }
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}] {} ({})",
            self.category.label(),
            self.message,
            self.source
        )
    }
}

impl std::error::Error for RuntimeError {}

/// Classifies an error message into a `RuntimeErrorCategory`.
///
/// Uses keyword matching against common error patterns.
/// Falls back to `Unknown` if no pattern matches.
pub fn classify_error(message: &str) -> RuntimeErrorCategory {
    let msg = message.to_lowercase();

    if msg.contains("timeout") || msg.contains("timed out") || msg.contains("deadline exceeded") {
        if msg.contains("provider")
            || msg.contains("api")
            || msg.contains("request")
            || msg.contains("http")
        {
            return RuntimeErrorCategory::ProviderTimeout;
        }
        return RuntimeErrorCategory::ToolTimeout;
    }

    if msg.contains("429") || msg.contains("rate limit") || msg.contains("too many requests") {
        return RuntimeErrorCategory::ProviderRateLimit;
    }

    if msg.contains("401")
        || msg.contains("unauthorized")
        || msg.contains("auth")
        || msg.contains("api key")
    {
        return RuntimeErrorCategory::ProviderAuthFailure;
    }

    if msg.contains("network")
        || msg.contains("connection refused")
        || msg.contains("dns")
        || msg.contains("connect")
        || msg.contains("url")
        || msg.contains("reqwest")
    {
        return RuntimeErrorCategory::ProviderNetworkError;
    }

    if msg.contains("permission") || msg.contains("denied") || msg.contains("forbidden") {
        return RuntimeErrorCategory::ToolPermissionDenied;
    }

    if msg.contains("memory") || msg.contains("oom") || msg.contains("out of memory") {
        return RuntimeErrorCategory::SystemMemoryLimit;
    }

    if msg.contains("shutdown") || msg.contains("cancelled") || msg.contains("canceled") {
        return RuntimeErrorCategory::SystemCancellation;
    }

    if msg.contains("tool")
        || msg.contains("execution")
        || msg.contains("command")
        || msg.contains("exit")
    {
        return RuntimeErrorCategory::ToolExecutionError;
    }

    RuntimeErrorCategory::Unknown
}

/// Creates a `RuntimeError` from a message string by classifying it.
pub fn from_message(message: &str, source: &str) -> RuntimeError {
    let category = classify_error(message);
    RuntimeError::new(message, category, source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_provider_timeout() {
        assert_eq!(
            classify_error("request timed out after 30s"),
            RuntimeErrorCategory::ProviderTimeout
        );
        assert_eq!(
            classify_error("deadline exceeded: provider request"),
            RuntimeErrorCategory::ProviderTimeout
        );
    }

    #[test]
    fn test_classify_tool_timeout() {
        assert_eq!(
            classify_error("command timed out after 60s"),
            RuntimeErrorCategory::ToolTimeout
        );
    }

    #[test]
    fn test_classify_rate_limit() {
        assert_eq!(
            classify_error("429 Too Many Requests"),
            RuntimeErrorCategory::ProviderRateLimit
        );
        assert_eq!(
            classify_error("rate limit exceeded"),
            RuntimeErrorCategory::ProviderRateLimit
        );
    }

    #[test]
    fn test_classify_auth_failure() {
        assert_eq!(
            classify_error("401 Unauthorized"),
            RuntimeErrorCategory::ProviderAuthFailure
        );
        assert_eq!(
            classify_error("invalid api key"),
            RuntimeErrorCategory::ProviderAuthFailure
        );
    }

    #[test]
    fn test_classify_network_error() {
        assert_eq!(
            classify_error("connection refused"),
            RuntimeErrorCategory::ProviderNetworkError
        );
        assert_eq!(
            classify_error("reqwest::Error: Io(Custom { kind: NotFound, ... })"),
            RuntimeErrorCategory::ProviderNetworkError
        );
    }

    #[test]
    fn test_classify_permission_denied() {
        assert_eq!(
            classify_error("permission denied: Operation not permitted"),
            RuntimeErrorCategory::ToolPermissionDenied
        );
    }

    #[test]
    fn test_classify_memory_limit() {
        assert_eq!(
            classify_error("out of memory"),
            RuntimeErrorCategory::SystemMemoryLimit
        );
        assert_eq!(
            classify_error("oom-killer invoked"),
            RuntimeErrorCategory::SystemMemoryLimit
        );
    }

    #[test]
    fn test_classify_cancellation() {
        assert_eq!(
            classify_error("operation cancelled"),
            RuntimeErrorCategory::SystemCancellation
        );
        assert_eq!(
            classify_error("shutdown requested"),
            RuntimeErrorCategory::SystemCancellation
        );
    }

    #[test]
    fn test_classify_tool_execution_error() {
        assert_eq!(
            classify_error("command exited with code 1"),
            RuntimeErrorCategory::ToolExecutionError
        );
    }

    #[test]
    fn test_classify_unknown() {
        assert_eq!(
            classify_error("some random error"),
            RuntimeErrorCategory::Unknown
        );
        assert_eq!(classify_error(""), RuntimeErrorCategory::Unknown);
    }

    #[test]
    fn test_is_retryable() {
        assert!(RuntimeErrorCategory::ProviderTimeout.is_retryable());
        assert!(RuntimeErrorCategory::ProviderRateLimit.is_retryable());
        assert!(RuntimeErrorCategory::ProviderNetworkError.is_retryable());
        assert!(RuntimeErrorCategory::ToolTimeout.is_retryable());
        assert!(RuntimeErrorCategory::Unknown.is_retryable());

        assert!(!RuntimeErrorCategory::ProviderAuthFailure.is_retryable());
        assert!(!RuntimeErrorCategory::ToolPermissionDenied.is_retryable());
        assert!(!RuntimeErrorCategory::SystemMemoryLimit.is_retryable());
        assert!(!RuntimeErrorCategory::SystemCancellation.is_retryable());
        assert!(!RuntimeErrorCategory::ToolExecutionError.is_retryable());
    }

    #[test]
    fn test_escalation_levels() {
        assert_eq!(
            RuntimeErrorCategory::ProviderAuthFailure.escalation_level(),
            3
        );
        assert_eq!(
            RuntimeErrorCategory::SystemMemoryLimit.escalation_level(),
            3
        );
        assert_eq!(
            RuntimeErrorCategory::ProviderNetworkError.escalation_level(),
            2
        );
        assert_eq!(
            RuntimeErrorCategory::ToolPermissionDenied.escalation_level(),
            2
        );
        assert_eq!(RuntimeErrorCategory::SystemShutdown.escalation_level(), 2);
        assert_eq!(RuntimeErrorCategory::ProviderTimeout.escalation_level(), 1);
        assert_eq!(
            RuntimeErrorCategory::ProviderRateLimit.escalation_level(),
            1
        );
        assert_eq!(RuntimeErrorCategory::ToolTimeout.escalation_level(), 1);
        assert_eq!(
            RuntimeErrorCategory::ToolExecutionError.escalation_level(),
            1
        );
        assert_eq!(RuntimeErrorCategory::Unknown.escalation_level(), 1);
        assert_eq!(
            RuntimeErrorCategory::SystemCancellation.escalation_level(),
            0
        );
    }

    #[test]
    fn test_runtime_error_display() {
        let err = RuntimeError::new(
            "request timed out",
            RuntimeErrorCategory::ProviderTimeout,
            "provider",
        );
        let display = format!("{}", err);
        assert!(display.contains("provider_timeout"));
        assert!(display.contains("request timed out"));
        assert!(display.contains("provider"));
    }

    #[test]
    fn test_from_message() {
        let err = from_message("request timed out", "main");
        assert_eq!(err.category, RuntimeErrorCategory::ProviderTimeout);
        assert_eq!(err.source, "main");
    }

    #[test]
    fn test_runtime_error_correlation_id() {
        let err = RuntimeError::new(
            "timeout",
            RuntimeErrorCategory::ProviderTimeout,
            "provider:openai",
        );
        assert!(err.correlation_id.contains("provider-openai"));
        assert!(err.correlation_id.contains("provider_timeout"));
    }
}
