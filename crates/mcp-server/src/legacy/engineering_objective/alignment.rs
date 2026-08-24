//! Deterministic scope-control and lazy-execution policy.
//!
//! These are small, deterministic decision rules that encode the
//! "Lazy by Default" engineering principle:
//!
//! > CodeBro prefers the smallest correct change, reuses existing project
//! > capabilities, avoids speculative abstractions, validates its work,
//! > and stops when the requested outcome is achieved.
//!
//! They are policy components, not an ML scorer. They never block
//! execution and never override user intent — they make the runtime's own
//! behavior consistent and testable.

use serde::{Deserialize, Serialize};

/// Scope classification for a change candidate relative to a task.
///
/// Used by the anti-scope-creep rule as **advisory guidance**: `Required`
/// candidates are most likely in scope, `Recommended` may be mentioned but
/// not modified without justification/approval, and `Unrelated` is left
/// alone.
///
/// # Safety
///
/// `ChangeScope` is a lexical heuristic, never semantic authorization.
/// Token overlap is not proof that a change is required, and a weak lexical
/// match must never authorize destructive or high-impact behavior.
/// Consequential actions still require explicit confirmation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ChangeScope {
    /// The candidate most likely satisfies the task (advisory).
    Required,
    /// The candidate is plausibly related but not clearly required.
    Recommended,
    /// The candidate is unrelated to the task.
    Unrelated,
}

impl ChangeScope {
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeScope::Required => "required",
            ChangeScope::Recommended => "recommended",
            ChangeScope::Unrelated => "unrelated",
        }
    }
}

/// Classify the scope of a change candidate for a task.
///
/// Deterministic token overlap: a candidate whose identifier appears in the
/// task (or shares strong tokens with it) is classified `Required`; candidates
/// sharing generic engineering vocabulary are `Recommended`; everything else
/// is `Unrelated`.
///
/// **Advisory only.** This is a heuristic that may guide scope discipline; it
/// must never be treated as proof of semantic requirement and must never be
/// used to authorize destructive behavior.
pub fn classify_change_scope(task: &str, candidate_scope: &str) -> ChangeScope {
    let task = task.to_lowercase();
    let candidate = candidate_scope.to_lowercase();

    if candidate.is_empty() {
        return ChangeScope::Unrelated;
    }

    let candidate_tokens: Vec<&str> = candidate
        .split(['/', '.', '-', '_', ' '])
        .filter(|t| t.len() > 2)
        .collect();
    if candidate_tokens.is_empty() {
        return ChangeScope::Unrelated;
    }

    let direct = candidate_tokens
        .iter()
        .filter(|t| task.contains(**t))
        .count();
    if direct > 0 {
        return ChangeScope::Required;
    }

    // Shared generic engineering vocabulary → recommend, don't require.
    const GENERIC: &[&str] = &[
        "implement",
        "refactor",
        "test",
        "fix",
        "module",
        "component",
        "feature",
        "bug",
        "document",
        "config",
        "error",
        "handler",
        "service",
        "engine",
        "runtime",
        "context",
        "provider",
    ];
    let recommended = candidate_tokens
        .iter()
        .filter(|t| GENERIC.contains(t))
        .count();
    if recommended > 0 {
        return ChangeScope::Recommended;
    }

    ChangeScope::Unrelated
}

/// The lazy-execution policy: deterministic rules behind the
/// `Inspect → Understand → Retrieve → Reuse → Change → Validate → Stop`
/// workflow.
///
/// These rules describe the execution philosophy. They are advisory: they
/// guide reuse preference, scope control, and the stop condition, but they
/// are never semantic authorization for destructive actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LazyExecutionPolicy {
    /// Design budget target (tokens) for the always-on objective block.
    pub objective_budget_tokens: usize,
    /// Maximum conversation messages injected per task (enforced).
    pub max_conversation_messages: usize,
    /// Maximum conversation tokens injected per task (enforced).
    pub max_conversation_tokens: usize,
}

impl Default for LazyExecutionPolicy {
    fn default() -> Self {
        LazyExecutionPolicy {
            objective_budget_tokens: 300,
            max_conversation_messages: 20,
            max_conversation_tokens: 1500,
        }
    }
}

impl LazyExecutionPolicy {
    /// Prefer reuse when an existing implementation candidate shares strong
    /// tokens with the task. Encodes: *"Does this already exist? Can I
    /// reuse an existing abstraction?"*.
    pub fn prefers_reuse(&self, task: &str, existing_candidates: &[String]) -> bool {
        let task = task.to_lowercase();
        existing_candidates.iter().any(|candidate| {
            let candidate = candidate.to_lowercase();
            let tokens: Vec<&str> = candidate
                .split(['/', '.', '-', '_', ' '])
                .filter(|t| t.len() > 3)
                .collect();
            tokens.iter().filter(|t| task.contains(**t)).count() >= 2
        })
    }

    /// Scope-control: only `Required` changes should be executed automatically.
    pub fn scope(&self, task: &str, candidate_scope: &str) -> ChangeScope {
        classify_change_scope(task, candidate_scope)
    }

    /// Stop condition: once the requested outcome is achieved *and*
    /// validation passes, the task is done. No unsolicited follow-up work.
    pub fn should_stop(&self, task: &str, outcome: &str, validation_passed: bool) -> bool {
        if !validation_passed {
            return false;
        }
        if outcome.trim().is_empty() {
            return false;
        }
        let task_tokens: Vec<String> = task
            .to_lowercase()
            .split_whitespace()
            .map(|t| t.to_string())
            .collect();
        let outcome_lower = outcome.to_lowercase();
        task_tokens
            .iter()
            .any(|t| t.len() > 3 && outcome_lower.contains(t.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scope_required_when_candidate_in_task() {
        assert_eq!(
            classify_change_scope("fix the auth module", "auth/module.rs"),
            ChangeScope::Required
        );
        assert_eq!(
            classify_change_scope(
                "refactor provider routing",
                "src/provider_runtime/routing.rs"
            ),
            ChangeScope::Required
        );
    }

    #[test]
    fn test_scope_recommended_on_generic_vocabulary() {
        assert_eq!(
            classify_change_scope("add a login page", "service.rs"),
            ChangeScope::Recommended
        );
    }

    #[test]
    fn test_scope_unrelated_is_not_required() {
        // Lexical heuristics are advisory: unrelated must never be conflated
        // with required (which could otherwise authorize destructive work).
        assert_ne!(ChangeScope::Unrelated, ChangeScope::Required);
        assert_ne!(
            classify_change_scope("fix the auth bug", "calendar.rs"),
            ChangeScope::Required
        );
    }

    #[test]
    fn test_scope_deterministic() {
        for i in 0..10 {
            assert_eq!(
                classify_change_scope("fix auth", "auth.rs"),
                classify_change_scope("fix auth", "auth.rs")
            );
            let _ = i;
        }
    }

    #[test]
    fn test_prefers_reuse_strong_overlap() {
        let policy = LazyExecutionPolicy::default();
        assert!(policy.prefers_reuse(
            "fix the authentication bug in the auth module",
            &[
                "src/auth/module.rs".to_string(),
                "src/db/pool.rs".to_string()
            ],
        ));
        assert!(
            !policy.prefers_reuse("add a calendar widget", &["src/auth/module.rs".to_string()],)
        );
    }

    #[test]
    fn test_prefers_reuse_is_deterministic() {
        let policy = LazyExecutionPolicy::default();
        let task = "refactor provider routing in provider_runtime";
        let candidates = vec![
            "src/provider_runtime/routing.rs".to_string(),
            "src/auth/module.rs".to_string(),
        ];
        for _ in 0..10 {
            assert_eq!(
                policy.prefers_reuse(task, &candidates),
                policy.prefers_reuse(task, &candidates)
            );
        }
    }

    #[test]
    fn test_should_stop_on_validated_outcome() {
        let policy = LazyExecutionPolicy::default();
        assert!(policy.should_stop("fix the auth bug", "fixed the auth bug", true,));
        // Empty outcome is never a stop.
        assert!(!policy.should_stop("fix the auth bug", "", true));
    }

    #[test]
    fn test_failed_validation_prevents_stop() {
        let policy = LazyExecutionPolicy::default();
        // A validated completed outcome can stop; a failed validation must
        // prevent the stop condition.
        assert!(!policy.should_stop("fix the auth bug", "fixed the auth bug", false));
    }

    #[test]
    fn test_default_budgets() {
        let policy = LazyExecutionPolicy::default();
        assert_eq!(policy.objective_budget_tokens, 300);
        assert_eq!(policy.max_conversation_messages, 20);
        assert_eq!(policy.max_conversation_tokens, 1500);
    }
}
