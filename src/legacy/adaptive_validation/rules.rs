//! Adaptive Validation Rules — deterministic validation rules.
//!
/// Each rule is a pure function: input → Option<(RiskLevel, String)>.
/// No state, no side effects, no LLM calls.
use super::types::*;

/// A single validation rule.
pub struct ValidationRule {
    pub rule_id: String,
    pub description: String,
    pub category: ValidationCategory,
    pub severity: RiskLevel,
    pub block_on_failure: bool,
    pub evaluate: Box<dyn Fn(&str) -> (bool, Vec<String>) + Send + Sync>,
}

impl ValidationRule {
    pub fn new(
        rule_id: &str,
        description: &str,
        category: ValidationCategory,
        severity: RiskLevel,
        block_on_failure: bool,
        evaluate: impl Fn(&str) -> (bool, Vec<String>) + Send + Sync + 'static,
    ) -> Self {
        ValidationRule {
            rule_id: rule_id.to_string(),
            description: description.to_string(),
            category,
            severity,
            block_on_failure,
            evaluate: Box::new(evaluate),
        }
    }

    /// Evaluate this rule against the given input.
    pub fn evaluate(&self, input: &str) -> (bool, Vec<String>) {
        (self.evaluate)(input)
    }
}

/// Returns all registered validation rules.
pub fn all_rules() -> Vec<ValidationRule> {
    vec![
        // ─── Workflow Rules ─────────────────────────────────────────────────
        ValidationRule::new(
            "rule-empty-workflow",
            "Empty Workflow",
            ValidationCategory::Workflow,
            RiskLevel::High,
            true,
            |input| {
                if input.trim().is_empty() {
                    (false, vec!["Workflow is empty".to_string()])
                } else {
                    (true, vec![])
                }
            },
        ),
        ValidationRule::new(
            "rule-invalid-workflow",
            "Invalid Workflow",
            ValidationCategory::Workflow,
            RiskLevel::High,
            true,
            |input| {
                if input.contains("invalid") || input.contains("corrupt") {
                    (
                        false,
                        vec!["Workflow contains invalid operations".to_string()],
                    )
                } else {
                    (true, vec![])
                }
            },
        ),
        // ─── Intent Rules ───────────────────────────────────────────────────
        ValidationRule::new(
            "rule-ambiguous-intent",
            "Ambiguous Intent",
            ValidationCategory::Intent,
            RiskLevel::Medium,
            false,
            |input| {
                if input.contains("ambiguous") || input.contains("unclear") {
                    (false, vec!["Intent is ambiguous".to_string()])
                } else {
                    (true, vec![])
                }
            },
        ),
        ValidationRule::new(
            "rule-low-confidence-intent",
            "Low Confidence Intent",
            ValidationCategory::Intent,
            RiskLevel::Medium,
            false,
            |input| {
                if input.contains("low_confidence") {
                    (
                        false,
                        vec!["Intent confidence is below threshold".to_string()],
                    )
                } else {
                    (true, vec![])
                }
            },
        ),
        // ─── Recommendation Rules ───────────────────────────────────────────
        ValidationRule::new(
            "rule-conflicting-recommendation",
            "Conflicting Recommendation",
            ValidationCategory::Recommendation,
            RiskLevel::Medium,
            false,
            |input| {
                if input.contains("conflict") || input.contains("contradicts") {
                    (
                        false,
                        vec!["Recommendation conflicts with existing preferences".to_string()],
                    )
                } else {
                    (true, vec![])
                }
            },
        ),
        ValidationRule::new(
            "rule-low-confidence-recommendation",
            "Low Confidence Recommendation",
            ValidationCategory::Recommendation,
            RiskLevel::Low,
            false,
            |input| {
                if input.contains("low_confidence") {
                    (false, vec!["Recommendation confidence is low".to_string()])
                } else {
                    (true, vec![])
                }
            },
        ),
        // ─── Dependency Rules ───────────────────────────────────────────────
        ValidationRule::new(
            "rule-missing-dependency",
            "Missing Dependency",
            ValidationCategory::Dependencies,
            RiskLevel::High,
            true,
            |input| {
                if input.contains("missing_dep") {
                    (false, vec!["Workflow has missing dependencies".to_string()])
                } else {
                    (true, vec![])
                }
            },
        ),
        ValidationRule::new(
            "rule-dependency-cycle",
            "Dependency Cycle",
            ValidationCategory::Dependencies,
            RiskLevel::Critical,
            true,
            |input| {
                if input.contains("cycle") {
                    (false, vec!["Dependency cycle detected".to_string()])
                } else {
                    (true, vec![])
                }
            },
        ),
        ValidationRule::new(
            "rule-invalid-dependency-order",
            "Invalid Dependency Order",
            ValidationCategory::Dependencies,
            RiskLevel::Medium,
            false,
            |input| {
                if input.contains("invalid_order") {
                    (false, vec!["Dependency order is invalid".to_string()])
                } else {
                    (true, vec![])
                }
            },
        ),
        // ─── Policy Rules ───────────────────────────────────────────────────
        ValidationRule::new(
            "rule-policy-violation",
            "Policy Violation",
            ValidationCategory::Policy,
            RiskLevel::High,
            true,
            |input| {
                if input.contains("policy_violation") {
                    (false, vec!["Workflow violates policy rules".to_string()])
                } else {
                    (true, vec![])
                }
            },
        ),
        ValidationRule::new(
            "rule-preference-policy",
            "Preference Policy",
            ValidationCategory::Policy,
            RiskLevel::Medium,
            false,
            |input| {
                if input.contains("preference_policy") {
                    (
                        false,
                        vec!["Preference policy constraint violated".to_string()],
                    )
                } else {
                    (true, vec![])
                }
            },
        ),
        // ─── Preference Rules ───────────────────────────────────────────────
        ValidationRule::new(
            "rule-unsafe-preference-combination",
            "Unsafe Preference Combination",
            ValidationCategory::Preference,
            RiskLevel::High,
            true,
            |input| {
                if input.contains("unsafe_combo") {
                    (false, vec!["Preference combination is unsafe".to_string()])
                } else {
                    (true, vec![])
                }
            },
        ),
        ValidationRule::new(
            "rule-invalid-preference-value",
            "Invalid Preference Value",
            ValidationCategory::Preference,
            RiskLevel::Medium,
            false,
            |input| {
                if input.contains("invalid_value") {
                    (false, vec!["Preference value is invalid".to_string()])
                } else {
                    (true, vec![])
                }
            },
        ),
        // ─── Conflict Rules ─────────────────────────────────────────────────
        ValidationRule::new(
            "rule-duplicate-command",
            "Duplicate Command",
            ValidationCategory::Conflict,
            RiskLevel::Medium,
            false,
            |input| {
                if input.contains("duplicate") {
                    (false, vec!["Duplicate commands detected".to_string()])
                } else {
                    (true, vec![])
                }
            },
        ),
        ValidationRule::new(
            "rule-conflicting-commands",
            "Conflicting Commands",
            ValidationCategory::Conflict,
            RiskLevel::High,
            true,
            |input| {
                if input.contains("conflict") {
                    (false, vec!["Conflicting commands detected".to_string()])
                } else {
                    (true, vec![])
                }
            },
        ),
        // ─── Risk Rules ─────────────────────────────────────────────────────
        ValidationRule::new(
            "rule-high-risk-operation",
            "High Risk Operation",
            ValidationCategory::Risk,
            RiskLevel::High,
            false,
            |input| {
                if input.contains("high_risk") {
                    (false, vec!["Operation has high risk profile".to_string()])
                } else {
                    (true, vec![])
                }
            },
        ),
        ValidationRule::new(
            "rule-irreversible-operation",
            "Irreversible Operation",
            ValidationCategory::Risk,
            RiskLevel::Medium,
            false,
            |input| {
                if input.contains("irreversible") {
                    (false, vec!["Operation is irreversible".to_string()])
                } else {
                    (true, vec![])
                }
            },
        ),
        // ─── Confidence Rules ───────────────────────────────────────────────
        ValidationRule::new(
            "rule-low-overall-confidence",
            "Low Overall Confidence",
            ValidationCategory::Confidence,
            RiskLevel::Medium,
            false,
            |input| {
                if input.contains("low_confidence") {
                    (
                        false,
                        vec!["Overall confidence is below threshold".to_string()],
                    )
                } else {
                    (true, vec![])
                }
            },
        ),
        // ─── Approval Rules ─────────────────────────────────────────────────
        ValidationRule::new(
            "rule-approval-not-ready",
            "Approval Not Ready",
            ValidationCategory::ApprovalReadiness,
            RiskLevel::High,
            true,
            |input| {
                if input.contains("not_ready") {
                    (
                        false,
                        vec!["Workflow is not ready for approval".to_string()],
                    )
                } else {
                    (true, vec![])
                }
            },
        ),
    ]
}

/// Evaluate all rules against the given input.
pub fn evaluate_all(input: &str) -> Vec<(ValidationRule, bool, Vec<String>)> {
    all_rules()
        .into_iter()
        .map(|rule| {
            let (passed, evidence) = rule.evaluate(input);
            (rule, passed, evidence)
        })
        .collect()
}

/// Find rules that failed.
pub fn find_failed_rules(input: &str) -> Vec<ValidationRule> {
    evaluate_all(input)
        .into_iter()
        .filter(|(_, passed, _)| !passed)
        .map(|(rule, _, _)| rule)
        .collect()
}
