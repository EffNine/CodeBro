//! Adaptive Validation Policy — externalized policy management.
//!
/// Policies are loaded externally and can be extended without code changes.
/// This module provides the policy engine for validation rules.
use super::types::*;

/// Policy engine for managing validation policies.
#[derive(Debug, Clone, Default)]
pub struct PolicyEngine {
    policies: Vec<Policy>,
}

impl PolicyEngine {
    pub fn new() -> Self {
        PolicyEngine {
            policies: Vec::new(),
        }
    }

    /// Register a policy.
    pub fn register(&mut self, policy: Policy) {
        self.policies.push(policy);
    }

    /// Get all enabled policies.
    pub fn enabled_policies(&self) -> Vec<&Policy> {
        self.policies.iter().filter(|p| p.is_enabled()).collect()
    }

    /// Evaluate all policies against input.
    pub fn evaluate(&self, input: &str) -> Vec<(&Policy, bool)> {
        self.enabled_policies()
            .iter()
            .map(|policy| {
                let all_rules_passed = policy.rules.iter().all(|rule| match &rule.evaluation {
                    RuleEvaluation::Boolean(val) => *val,
                    RuleEvaluation::ConfidenceThreshold { min } => *min <= 1.0,
                    RuleEvaluation::RiskThreshold { max } => max.score() >= 100,
                    RuleEvaluation::Custom(_) => true,
                });
                (*policy, all_rules_passed)
            })
            .collect()
    }

    /// Check if any policy fails.
    pub fn has_failures(&self, input: &str) -> bool {
        !self.evaluate(input).iter().all(|(_, passed)| *passed)
    }

    /// Get policy failures.
    pub fn get_failures(&self, input: &str) -> Vec<&Policy> {
        self.evaluate(input)
            .into_iter()
            .filter(|(_, passed)| !*passed)
            .map(|(policy, _)| policy)
            .collect()
    }
}

/// Default policies for standard validation.
pub fn default_policies() -> Vec<Policy> {
    vec![
        Policy::new(
            "policy-basic-safety",
            "Basic Safety",
            "Ensures basic safety checks pass",
            vec![
                PolicyRule::new(
                    "rule-no-empty-workflow",
                    "Workflow must not be empty",
                    ValidationCategory::Workflow,
                    RiskLevel::High,
                    true,
                    RuleEvaluation::Boolean(true),
                ),
                PolicyRule::new(
                    "rule-no-cycles",
                    "No dependency cycles allowed",
                    ValidationCategory::Dependencies,
                    RiskLevel::Critical,
                    true,
                    RuleEvaluation::Boolean(true),
                ),
            ],
        ),
        Policy::new(
            "policy-confidence-threshold",
            "Confidence Threshold",
            "Ensures minimum confidence levels",
            vec![PolicyRule::new(
                "rule-min-confidence",
                "Minimum confidence threshold",
                ValidationCategory::Confidence,
                RiskLevel::Medium,
                false,
                RuleEvaluation::ConfidenceThreshold { min: 0.5 },
            )],
        ),
        Policy::new(
            "policy-risk-limits",
            "Risk Limits",
            "Ensures risk levels are within bounds",
            vec![PolicyRule::new(
                "rule-max-risk",
                "Maximum risk level",
                ValidationCategory::Risk,
                RiskLevel::High,
                true,
                RuleEvaluation::RiskThreshold {
                    max: RiskLevel::High,
                },
            )],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_policy_engine_creation() {
        let engine = PolicyEngine::new();
        assert!(engine.enabled_policies().is_empty());
    }

    #[test]
    fn test_policy_engine_register() {
        let mut engine = PolicyEngine::new();
        let policy = Policy::new("p1", "Test", "Test policy", vec![]);
        engine.register(policy);
        assert_eq!(engine.enabled_policies().len(), 1);
    }

    #[test]
    fn test_policy_engine_evaluate_all_pass() {
        let engine = PolicyEngine::new();
        let results = engine.evaluate("normal input");
        assert!(results.iter().all(|(_, passed)| *passed));
    }

    #[test]
    fn test_default_policies() {
        let policies = default_policies();
        assert_eq!(policies.len(), 3);
    }

    #[test]
    fn test_policy_rule_evaluate_boolean() {
        let rule = PolicyRule::new(
            "r1",
            "Test",
            ValidationCategory::Workflow,
            RiskLevel::Low,
            false,
            RuleEvaluation::Boolean(true),
        );
        match &rule.evaluation {
            RuleEvaluation::Boolean(val) => assert!(*val),
            _ => panic!("Expected Boolean"),
        }
    }

    #[test]
    fn test_policy_rule_evaluate_confidence() {
        let rule = PolicyRule::new(
            "r2",
            "Test",
            ValidationCategory::Confidence,
            RiskLevel::Medium,
            false,
            RuleEvaluation::ConfidenceThreshold { min: 0.5 },
        );
        match &rule.evaluation {
            RuleEvaluation::ConfidenceThreshold { min } => assert!((*min - 0.5).abs() < 0.001),
            _ => panic!("Expected ConfidenceThreshold"),
        }
    }
}
