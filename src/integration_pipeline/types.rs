//! Integration Pipeline Types — core data model for the decision pipeline.
//!
//! All types are immutable, serializable, and auditable.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;

use crate::adaptive_validation::{ValidationReport, ValidationResult};
use crate::intent_engine::{
    AmbiguityResult, ConfidenceResult, IntentPlan, IntentType, ResolvedCommand,
};
use crate::recommendation_engine::RecommendationSet;
use crate::workflow_engine::WorkflowResult;

/// Complete result of running the integration pipeline.
///
/// Contains all stages' outputs for auditing and replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineResult {
    pub user_input: String,
    pub intent_plan: IntentPlan,
    pub ambiguity_result: AmbiguityResult,
    pub confidence_result: ConfidenceResult,
    pub resolved_commands: Vec<ResolvedCommand>,
    pub recommendation_set: RecommendationSet,
    pub workflow_result: WorkflowResult,
    pub validation_report: ValidationReport,
    pub previews: Vec<crate::intent_engine::ApprovalPreview>,
    pub classify_duration: Duration,
    pub total_duration: Duration,
}

impl PipelineResult {
    pub fn new(
        user_input: String,
        intent_plan: IntentPlan,
        ambiguity_result: AmbiguityResult,
        confidence_result: ConfidenceResult,
        resolved_commands: Vec<ResolvedCommand>,
        recommendation_set: RecommendationSet,
        workflow_result: WorkflowResult,
        validation_report: ValidationReport,
        previews: Vec<crate::intent_engine::ApprovalPreview>,
        classify_duration: Duration,
        total_duration: Duration,
    ) -> Self {
        PipelineResult {
            user_input,
            intent_plan,
            ambiguity_result,
            confidence_result,
            resolved_commands,
            recommendation_set,
            workflow_result,
            validation_report,
            previews,
            classify_duration,
            total_duration,
        }
    }

    /// Check if the pipeline result is ready for approval.
    pub fn is_approval_ready(&self) -> bool {
        !self.ambiguity_result.is_ambiguous
            && self.confidence_result.is_confident()
            && self.validation_report.result == ValidationResult::Pass
            && self.workflow_result.plan.is_valid
            && !self.workflow_result.plan.issues.iter().any(|i| matches!(i, crate::workflow_engine::WorkflowIssue::DependencyCycle { .. }))
            // Informational intents (questions, help) don't need approval
            && !matches!(self.intent_plan.intent_type, IntentType::Question | IntentType::Help)
    }

    /// Get the overall status of the pipeline.
    pub fn status(&self) -> PipelineStatus {
        if self.ambiguity_result.is_ambiguous {
            PipelineStatus::Ambiguous
        } else if !self.confidence_result.is_confident() {
            PipelineStatus::LowConfidence
        } else if self.validation_report.blocks_approval() {
            PipelineStatus::ValidationFailed
        } else if !self.workflow_result.plan.is_valid {
            PipelineStatus::WorkflowInvalid
        } else if self.is_approval_ready() {
            PipelineStatus::Ready
        } else {
            PipelineStatus::Unknown
        }
    }

    /// Get human-readable summary.
    pub fn summary(&self) -> String {
        format!(
            "Pipeline [{}]: intent={}, ambiguity={}, confidence={:.2}, \
             workflow_valid={}, validation={}, approval_ready={}",
            self.intent_plan.id,
            self.intent_plan.intent_type,
            self.ambiguity_result.is_ambiguous,
            self.confidence_result.score,
            self.workflow_result.plan.is_valid,
            self.validation_report.result,
            self.is_approval_ready(),
        )
    }
}

impl fmt::Display for PipelineResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.summary())
    }
}

/// Overall pipeline status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PipelineStatus {
    Ready,
    Ambiguous,
    LowConfidence,
    ValidationFailed,
    WorkflowInvalid,
    Unknown,
}

impl fmt::Display for PipelineStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PipelineStatus::Ready => write!(f, "READY"),
            PipelineStatus::Ambiguous => write!(f, "AMBIGUOUS"),
            PipelineStatus::LowConfidence => write!(f, "LOW_CONFIDENCE"),
            PipelineStatus::ValidationFailed => write!(f, "VALIDATION_FAILED"),
            PipelineStatus::WorkflowInvalid => write!(f, "WORKFLOW_INVALID"),
            PipelineStatus::Unknown => write!(f, "UNKNOWN"),
        }
    }
}

/// Approval-ready summary for the TUI and Approval Gate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalSummary {
    pub intent_type: String,
    pub detected_goal: String,
    pub confidence: f64,
    pub is_ambiguous: bool,
    pub ambiguity_reason: Option<String>,
    pub clarification_questions: Vec<String>,
    pub workflow_steps: usize,
    pub workflow_valid: bool,
    pub workflow_issues: usize,
    pub validation_result: String,
    pub validation_issues: usize,
    pub validation_warnings: usize,
    pub is_ready_for_approval: bool,
    pub recommendations_count: usize,
    pub estimated_cost: f64,
    pub preview_commands: Vec<String>,
}

impl ApprovalSummary {
    pub fn from_pipeline_result(result: PipelineResult) -> Self {
        let clarification_questions = if result.ambiguity_result.is_ambiguous {
            result.ambiguity_result.clarification_questions.clone()
        } else {
            Vec::new()
        };

        let preview_commands: Vec<String> = result
            .previews
            .iter()
            .map(|p| format!("{}: {}", p.command_kind, p.requested_change))
            .collect();

        ApprovalSummary {
            intent_type: result.intent_plan.intent_type.to_string(),
            detected_goal: result.intent_plan.detected_goal.clone(),
            confidence: result.confidence_result.score,
            is_ambiguous: result.ambiguity_result.is_ambiguous,
            ambiguity_reason: result.ambiguity_result.reason.clone(),
            clarification_questions,
            workflow_steps: result.workflow_result.plan.total_steps,
            workflow_valid: result.workflow_result.plan.is_valid,
            workflow_issues: result.workflow_result.plan.issues.len(),
            validation_result: result.validation_report.result.to_string(),
            validation_issues: result.validation_report.issues.len(),
            validation_warnings: result.validation_report.warnings.len(),
            is_ready_for_approval: result.is_approval_ready(),
            recommendations_count: result.recommendation_set.len(),
            estimated_cost: result.workflow_result.plan.total_estimated_cost,
            preview_commands,
        }
    }
}

impl fmt::Display for ApprovalSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Intent: {}", self.intent_type)?;
        writeln!(f, "Goal: {}", self.detected_goal)?;
        writeln!(f, "Confidence: {:.2}", self.confidence)?;
        writeln!(f, "Ambiguous: {}", self.is_ambiguous)?;
        if self.is_ambiguous {
            if let Some(reason) = &self.ambiguity_reason {
                writeln!(f, "Reason: {}", reason)?;
            }
            for q in &self.clarification_questions {
                writeln!(f, "  Q: {}", q)?;
            }
        }
        writeln!(
            f,
            "Workflow: {} steps, valid={}",
            self.workflow_steps, self.workflow_valid
        )?;
        writeln!(f, "Validation: {}", self.validation_result)?;
        writeln!(f, "Ready for approval: {}", self.is_ready_for_approval)?;
        writeln!(f, "Recommendations: {}", self.recommendations_count)?;
        writeln!(f, "Estimated cost: ${:.2}", self.estimated_cost)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_result_creation() {
        let intent = IntentPlan::new(
            "test-1".to_string(),
            "Change model",
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Test",
            vec!["Rule match".to_string()],
            vec![],
        );
        let result = PipelineResult::new(
            "test input".to_string(),
            intent,
            AmbiguityResult::clear(),
            ConfidenceResult::new(0.9, vec!["test".to_string()], "test"),
            vec![],
            RecommendationSet::new("test-1"),
            WorkflowResult::new(
                crate::workflow_engine::WorkflowPlan::new(
                    "plan-1".to_string(),
                    "test-1",
                    vec![],
                    vec![],
                    crate::workflow_engine::ExecutionStrategy::Sequential,
                    vec![],
                    vec![],
                ),
                crate::workflow_engine::WorkflowMetadata::default(),
            ),
            ValidationReport::new("val-1".to_string(), ValidationResult::Pass),
            vec![],
            Duration::from_millis(1),
            Duration::from_millis(1),
        );

        assert!(result.is_approval_ready());
        assert_eq!(result.status(), PipelineStatus::Ready);
    }

    #[test]
    fn test_pipeline_result_ambiguous() {
        let intent = IntentPlan::new(
            "test-2".to_string(),
            "Use Claude.",
            IntentType::Unknown,
            "unknown",
            false,
            0.0,
            0.1,
            true,
            Some("Ambiguous".to_string()),
            "No match",
            vec!["No match".to_string()],
            vec![],
        );
        let result = PipelineResult::new(
            "ambiguous input".to_string(),
            intent,
            AmbiguityResult::ambiguous("Vague model reference", vec!["Which model?".to_string()]),
            ConfidenceResult::new(0.1, vec![], "low"),
            vec![],
            RecommendationSet::new("test-2"),
            WorkflowResult::new(
                crate::workflow_engine::WorkflowPlan::new(
                    "plan-2".to_string(),
                    "test-2",
                    vec![],
                    vec![],
                    crate::workflow_engine::ExecutionStrategy::Sequential,
                    vec![],
                    vec![],
                ),
                crate::workflow_engine::WorkflowMetadata::default(),
            ),
            ValidationReport::new("val-2".to_string(), ValidationResult::Pass),
            vec![],
            Duration::from_millis(1),
            Duration::from_millis(1),
        );

        assert!(!result.is_approval_ready());
        assert_eq!(result.status(), PipelineStatus::Ambiguous);
    }

    #[test]
    fn test_approval_summary_display() {
        let intent = IntentPlan::new(
            "test-3".to_string(),
            "Change model to gpt-4o",
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Test",
            vec!["Rule match".to_string()],
            vec![],
        );
        let pipeline_result = PipelineResult::new(
            "test input".to_string(),
            intent,
            AmbiguityResult::clear(),
            ConfidenceResult::new(0.9, vec![], "high"),
            vec![],
            RecommendationSet::new("test-3"),
            WorkflowResult::new(
                crate::workflow_engine::WorkflowPlan::new(
                    "plan-3".to_string(),
                    "test-3",
                    vec![],
                    vec![],
                    crate::workflow_engine::ExecutionStrategy::Sequential,
                    vec![],
                    vec![],
                ),
                crate::workflow_engine::WorkflowMetadata::default(),
            ),
            ValidationReport::new("val-3".to_string(), ValidationResult::Pass),
            vec![],
            Duration::from_millis(1),
            Duration::from_millis(1),
        );

        let summary = ApprovalSummary::from_pipeline_result(pipeline_result);
        let display = format!("{}", summary);
        assert!(display.contains("Goal:"));
        assert!(display.contains("gpt-4o"));
        assert!(display.contains("Ready"));
    }

    #[test]
    fn test_pipeline_result_serializable() {
        let intent = IntentPlan::new(
            "test-4".to_string(),
            "Test input",
            IntentType::Question,
            "question_engine",
            false,
            0.0,
            0.8,
            false,
            None,
            "Test",
            vec![],
            vec![],
        );
        let result = PipelineResult::new(
            "test input".to_string(),
            intent,
            AmbiguityResult::clear(),
            ConfidenceResult::new(0.8, vec![], "test"),
            vec![],
            RecommendationSet::new("test-4"),
            WorkflowResult::new(
                crate::workflow_engine::WorkflowPlan::new(
                    "plan-4".to_string(),
                    "test-4",
                    vec![],
                    vec![],
                    crate::workflow_engine::ExecutionStrategy::Sequential,
                    vec![],
                    vec![],
                ),
                crate::workflow_engine::WorkflowMetadata::default(),
            ),
            ValidationReport::new("val-4".to_string(), ValidationResult::Pass),
            vec![],
            Duration::from_millis(1),
            Duration::from_millis(1),
        );

        let json = serde_json::to_string(&result).expect("should serialize");
        let deserialized: PipelineResult = serde_json::from_str(&json).expect("should deserialize");

        assert_eq!(deserialized.user_input, result.user_input);
        assert_eq!(
            deserialized.intent_plan.intent_type,
            result.intent_plan.intent_type
        );
        assert_eq!(
            deserialized.validation_report.result,
            result.validation_report.result
        );
    }
}
