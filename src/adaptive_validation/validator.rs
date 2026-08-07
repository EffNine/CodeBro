//! Adaptive Validation Validator — main validation orchestration.
//!
use super::confidence::ConfidenceEvaluator;
use super::policy::PolicyEngine;
use super::risk::RiskAssessor;
use super::rules;
/// Combines rules, policies, confidence, and risk into a complete validation.
use super::types::*;

/// Main validator that orchestrates all validation checks.
#[derive(Debug, Clone, Default)]
pub struct Validator {
    pub policy_engine: PolicyEngine,
    pub confidence_evaluator: ConfidenceEvaluator,
    pub risk_assessor: RiskAssessor,
}

impl Validator {
    pub fn new() -> Self {
        Validator {
            policy_engine: PolicyEngine::new(),
            confidence_evaluator: ConfidenceEvaluator::new(),
            risk_assessor: RiskAssessor::new(),
        }
    }

    /// Validate the complete pipeline input.
    pub fn validate(&self, input: &str, config: &ValidationConfig) -> ValidationReport {
        let mut report = ValidationReport::new("validation-1".to_string(), ValidationResult::Pass);

        // Evaluate rules
        let rule_results = rules::evaluate_all(input);
        for (rule, passed, evidence) in rule_results {
            report.evidence.record_check(passed);
            if !passed {
                report.add_issue(ValidationIssue::new(
                    &rule.category,
                    rule.severity.clone(),
                    &rule.description,
                    evidence,
                    &format!("Review: {}", rule.description),
                    rule.block_on_failure,
                ));
            }
        }

        // Evaluate policies
        let policy_results = self.policy_engine.evaluate(input);
        for (policy, passed) in policy_results {
            report.evidence.record_policy_evaluation();
            if !passed {
                report.add_issue(ValidationIssue::new(
                    &ValidationCategory::Policy,
                    RiskLevel::High,
                    &format!("Policy '{}' violated", policy.name),
                    vec![format!("Policy: {}", policy.description)],
                    "Review and adjust policy or input",
                    true,
                ));
            }
        }

        // Evaluate confidence
        let confidence = self.confidence_evaluator.evaluate(input, config);
        report.evidence.record_confidence_calculation();
        report.avg_confidence = confidence;

        if confidence < config.min_confidence {
            report.add_warning(ValidationWarning::new(
                ValidationCategory::Confidence,
                &format!(
                    "Low confidence: {:.2} < {}",
                    confidence, config.min_confidence
                ),
                RiskLevel::Medium,
            ));
        }

        // Assess risk
        let risk = self
            .risk_assessor
            .assess(input, &report.issues, &report.warnings);
        report.evidence.record_risk_assessment();

        if !self.risk_assessor.is_acceptable(&risk, config) {
            report.add_issue(ValidationIssue::new(
                &ValidationCategory::Risk,
                risk.clone(),
                "Risk level exceeds threshold",
                vec![self.risk_assessor.mitigation_suggestion(&risk)],
                &self.risk_assessor.mitigation_suggestion(&risk),
                true,
            ));
        }

        // Determine overall result
        report.result = self.determine_result(&report, config);
        report.update_summary();

        report
    }

    /// Determine overall validation result.
    fn determine_result(
        &self,
        report: &ValidationReport,
        config: &ValidationConfig,
    ) -> ValidationResult {
        // Check for blocking issues
        let blocking_issues = report.issues.iter().filter(|i| i.blocks_approval).count();
        if blocking_issues > 0 {
            return ValidationResult::Reject;
        }

        // Check for critical risk
        if report.max_risk_level == RiskLevel::Critical {
            return ValidationResult::Reject;
        }

        // Check for too many issues
        if report.issues.len() >= config.max_issues_before_reject {
            return ValidationResult::Reject;
        }

        // Check for low confidence
        if report.avg_confidence < config.min_confidence {
            return ValidationResult::RequiresClarification;
        }

        // Check for warnings
        if !report.warnings.is_empty() && config.block_on_warnings {
            return ValidationResult::RequiresClarification;
        }

        // All checks passed
        if report.warnings.is_empty() {
            ValidationResult::Pass
        } else {
            ValidationResult::PassWithWarnings
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validator_creation() {
        let _validator = Validator::new();
    }

    #[test]
    fn test_validator_normal_input() {
        let validator = Validator::new();
        let config = ValidationConfig::new();
        let report = validator.validate("normal input", &config);
        assert_eq!(report.result, ValidationResult::Pass);
        assert!(report.issues.is_empty());
    }

    #[test]
    fn test_validator_low_confidence() {
        let validator = Validator::new();
        let config = ValidationConfig::new().with_min_confidence(0.9);
        let report = validator.validate("low_confidence input", &config);
        // Should have warnings but may not reject depending on other factors
        assert!(!report.warnings.is_empty() || report.result != ValidationResult::Pass);
    }

    #[test]
    fn test_validator_with_policy_failure() {
        let mut validator = Validator::new();
        validator.policy_engine.register(Policy::new(
            "fail-policy",
            "Fail Policy",
            "Will fail",
            vec![PolicyRule::new(
                "r1",
                "Test",
                ValidationCategory::Policy,
                RiskLevel::High,
                true,
                RuleEvaluation::Boolean(false),
            )],
        ));
        let config = ValidationConfig::new();
        let report = validator.validate("input", &config);
        assert_eq!(report.result, ValidationResult::Reject);
    }

    #[test]
    fn test_validator_deterministic() {
        let validator = Validator::new();
        let config = ValidationConfig::new();
        let report1 = validator.validate("test input", &config);
        let report2 = validator.validate("test input", &config);
        assert_eq!(report1.result, report2.result);
        assert_eq!(report1.issues.len(), report2.issues.len());
    }
}
