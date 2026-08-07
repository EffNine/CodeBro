//! Adaptive Validation Risk — risk assessment and scoring.
//!
/// Calculates risk levels for validation outcomes.
use super::types::*;

/// Risk assessor for validation.
#[derive(Debug, Clone, Default)]
pub struct RiskAssessor {
    _private: (),
}

impl RiskAssessor {
    pub fn new() -> Self {
        RiskAssessor { _private: () }
    }

    /// Assess risk for a validation input.
    pub fn assess(
        &self,
        input: &str,
        issues: &[ValidationIssue],
        warnings: &[ValidationWarning],
    ) -> RiskLevel {
        let mut max_risk = RiskLevel::Info;

        // Consider issues
        for issue in issues {
            if issue.severity.score() > max_risk.score() {
                max_risk = issue.severity.clone();
            }
        }

        // Consider warnings
        for warning in warnings {
            if warning.risk_level.score() > max_risk.score() {
                max_risk = warning.risk_level.clone();
            }
        }

        // Additional risk factors from input
        if input.contains("high_risk") || input.contains("dangerous") {
            if RiskLevel::High.score() > max_risk.score() {
                max_risk = RiskLevel::High;
            }
        }

        if input.contains("critical") || input.contains("unsafe") {
            if RiskLevel::Critical.score() > max_risk.score() {
                max_risk = RiskLevel::Critical;
            }
        }

        max_risk
    }

    /// Check if risk is within acceptable bounds.
    pub fn is_acceptable(&self, risk: &RiskLevel, config: &ValidationConfig) -> bool {
        risk.score() <= config.max_risk_level.score()
    }

    /// Get risk mitigation suggestion.
    pub fn mitigation_suggestion(&self, risk: &RiskLevel) -> String {
        match risk {
            RiskLevel::Info => "No action required".to_string(),
            RiskLevel::Low => "Monitor and review if issues escalate".to_string(),
            RiskLevel::Medium => "Review recommended before proceeding".to_string(),
            RiskLevel::High => "Review required before proceeding".to_string(),
            RiskLevel::Critical => "Immediate review required — do not proceed".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_assess_no_issues() {
        let assessor = RiskAssessor::new();
        let issues = vec![];
        let warnings = vec![];
        let risk = assessor.assess("normal input", &issues, &warnings);
        assert_eq!(risk, RiskLevel::Info);
    }

    #[test]
    fn test_assess_with_issues() {
        let assessor = RiskAssessor::new();
        let issues = vec![ValidationIssue::new(
            &ValidationCategory::Workflow,
            RiskLevel::High,
            "Test issue",
            vec!["evidence".to_string()],
            "Fix it",
            false,
        )];
        let warnings = vec![];
        let risk = assessor.assess("input", &issues, &warnings);
        assert_eq!(risk, RiskLevel::High);
    }

    #[test]
    fn test_assess_with_warnings() {
        let assessor = RiskAssessor::new();
        let issues = vec![];
        let warnings = vec![ValidationWarning::new(
            ValidationCategory::Risk,
            "Test warning",
            RiskLevel::Medium,
        )];
        let risk = assessor.assess("input", &issues, &warnings);
        assert_eq!(risk, RiskLevel::Medium);
    }

    #[test]
    fn test_is_acceptable() {
        let assessor = RiskAssessor::new();
        let config = ValidationConfig::new().with_max_risk_level(RiskLevel::High);
        assert!(assessor.is_acceptable(&RiskLevel::Low, &config));
        assert!(assessor.is_acceptable(&RiskLevel::High, &config));
        assert!(!assessor.is_acceptable(&RiskLevel::Critical, &config));
    }

    #[test]
    fn test_mitigation_suggestion() {
        let assessor = RiskAssessor::new();
        assert_eq!(
            assessor.mitigation_suggestion(&RiskLevel::Info),
            "No action required"
        );
        assert_eq!(
            assessor.mitigation_suggestion(&RiskLevel::Critical),
            "Immediate review required — do not proceed"
        );
    }
}
