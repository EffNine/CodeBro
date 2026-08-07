//! Adaptive Validation Engine — main orchestration.
//!
use super::diagnostics::AdaptiveDiagnostics;
/// Receives complete pipeline state and produces ValidationReport.
/// Never modifies input state.
use super::types::*;
use super::validator::Validator;
use crate::intent_engine::{IntentCommand, IntentPlan, IntentType};
use crate::recommendation_engine::RecommendationSet;
use crate::workflow_engine::{ExecutionStrategy, WorkflowPlan};

/// The main adaptive validation engine.
///
/// Stateless observer that validates the complete pipeline state.
#[derive(Debug, Clone, Default)]
pub struct AdaptiveValidationEngine {
    _private: (),
}

impl AdaptiveValidationEngine {
    pub fn new() -> Self {
        AdaptiveValidationEngine { _private: () }
    }

    /// Validate the complete pipeline state.
    ///
    /// Returns a ValidationReport with the validation result.
    pub fn validate(
        &self,
        intent_plan: &IntentPlan,
        recommendations: Option<&RecommendationSet>,
        workflow_plan: Option<&WorkflowPlan>,
        config: &ValidationConfig,
        diagnostics: &AdaptiveDiagnostics,
    ) -> ValidationReport {
        diagnostics.record_validation_started();

        let mut report = ValidationReport::new(
            format!("validation_{}", intent_plan.id),
            ValidationResult::Pass,
        );

        // Build composite input for validation
        let mut composite_input = String::new();
        composite_input.push_str(&intent_plan.detected_goal);
        composite_input.push(' ');
        composite_input.push_str(&intent_plan.reasoning);

        if let Some(recs) = recommendations {
            for rec in &recs.recommendations {
                composite_input.push_str(&format!(" {}:", rec.title));
            }
        }

        if let Some(workflow) = workflow_plan {
            composite_input.push_str(&format!(" steps={}", workflow.total_steps));
            composite_input.push_str(&format!(" valid={}", workflow.is_valid));
            if !workflow.issues.is_empty() {
                composite_input.push_str(" invalid_workflow");
            }
        }

        // Run validation
        let validator = Validator::new();
        let validation_report = validator.validate(&composite_input, config);

        // Merge results
        report.result = validation_report.result.clone();
        report.issues = validation_report.issues;
        report.warnings = validation_report.warnings;
        report.evidence = validation_report.evidence;
        report.max_risk_level = validation_report.max_risk_level;
        report.avg_confidence = validation_report.avg_confidence;
        report.update_summary();

        diagnostics.record_validation_completed(&report);

        report
    }

    /// Check if the pipeline is ready for approval.
    pub fn is_approval_ready(
        &self,
        intent_plan: &IntentPlan,
        recommendations: Option<&RecommendationSet>,
        workflow_plan: Option<&WorkflowPlan>,
        config: &ValidationConfig,
    ) -> bool {
        let report = self.validate(
            intent_plan,
            recommendations,
            workflow_plan,
            config,
            &AdaptiveDiagnostics::new(100),
        );
        !report.blocks_approval()
    }

    /// Get validation summary for display.
    pub fn get_summary(
        &self,
        intent_plan: &IntentPlan,
        recommendations: Option<&RecommendationSet>,
        workflow_plan: Option<&WorkflowPlan>,
        config: &ValidationConfig,
    ) -> ValidationSummary {
        let report = self.validate(
            intent_plan,
            recommendations,
            workflow_plan,
            config,
            &AdaptiveDiagnostics::new(100),
        );
        ValidationSummary::from_report(&report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engine_creation() {
        let _engine = AdaptiveValidationEngine::new();
    }

    #[test]
    fn test_validate_normal_pipeline() {
        let engine = AdaptiveValidationEngine::new();
        let intent = IntentPlan::new(
            "test-1".to_string(),
            "Change the model to gpt-4o",
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Model change",
            vec!["Rule match".to_string()],
            vec![IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "gpt-4o".to_string(),
                reason: "User requested".to_string(),
            }],
        );
        let config = ValidationConfig::new();
        let diag = AdaptiveDiagnostics::new(100);
        let report = engine.validate(&intent, None, None, &config, &diag);
        assert_eq!(report.result, ValidationResult::Pass);
    }

    #[test]
    fn test_validate_with_workflow() {
        let engine = AdaptiveValidationEngine::new();
        let intent = IntentPlan::new(
            "test-2".to_string(),
            "Change model",
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Change",
            vec![],
            vec![IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "gpt-4o".to_string(),
                reason: "R".to_string(),
            }],
        );
        let workflow = WorkflowPlan::new(
            "wf-1".to_string(),
            "test-2",
            vec![],
            vec![],
            ExecutionStrategy::Sequential,
            vec![],
            vec![],
        );
        let config = ValidationConfig::new();
        let diag = AdaptiveDiagnostics::new(100);
        let report = engine.validate(&intent, None, Some(&workflow), &config, &diag);
        assert_eq!(report.result, ValidationResult::Pass);
    }

    #[test]
    fn test_validate_is_read_only() {
        let engine = AdaptiveValidationEngine::new();
        let intent = IntentPlan::new(
            "test-3".to_string(),
            "Change model",
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Change",
            vec![],
            vec![IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "gpt-4o".to_string(),
                reason: "R".to_string(),
            }],
        );
        let config = ValidationConfig::new();
        let diag = AdaptiveDiagnostics::new(100);
        let _report = engine.validate(&intent, None, None, &config, &diag);
        // Intent must not be mutated
        assert_eq!(intent.required_commands.len(), 1);
    }

    #[test]
    fn test_validate_deterministic() {
        let engine = AdaptiveValidationEngine::new();
        let intent = IntentPlan::new(
            "test-4".to_string(),
            "Change model",
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Change",
            vec![],
            vec![IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "gpt-4o".to_string(),
                reason: "R".to_string(),
            }],
        );
        let config = ValidationConfig::new();
        let diag = AdaptiveDiagnostics::new(100);
        let report1 = engine.validate(&intent, None, None, &config, &diag);
        let report2 = engine.validate(&intent, None, None, &config, &diag);
        assert_eq!(report1.result, report2.result);
        assert_eq!(report1.issues.len(), report2.issues.len());
    }

    #[test]
    fn test_is_approval_ready() {
        let engine = AdaptiveValidationEngine::new();
        let intent = IntentPlan::new(
            "test-5".to_string(),
            "Change model",
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Change",
            vec![],
            vec![IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "gpt-4o".to_string(),
                reason: "R".to_string(),
            }],
        );
        let config = ValidationConfig::new();
        let ready = engine.is_approval_ready(&intent, None, None, &config);
        assert!(ready);
    }

    #[test]
    fn test_get_summary() {
        let engine = AdaptiveValidationEngine::new();
        let intent = IntentPlan::new(
            "test-6".to_string(),
            "Change model",
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Change",
            vec![],
            vec![IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "gpt-4o".to_string(),
                reason: "R".to_string(),
            }],
        );
        let config = ValidationConfig::new();
        let summary = engine.get_summary(&intent, None, None, &config);
        assert_eq!(summary.result, "PASS");
        assert!(summary.approval_ready);
    }
}
