//! Integration Pipeline — end-to-end orchestration of the P6 decision engines.
//!
//! This module wires together the four P6 foundation engines into a single
//! deterministic pipeline:
//!
//! ```text
//! User Input
//!   ↓
//! IntentEngine (classify + resolve + preview + ambiguity + confidence)
//!   ↓
//! RecommendationEngine (observe → RecommendationSet)
//!   ↓
//! WorkflowEngine (plan → WorkflowPlan)
//!   ↓
//! AdaptiveValidationEngine (validate → ValidationReport)
//!   ↓
//! ApprovalPreview (read-only summary)
//!   ↓
//! PipelineResult (immutable, serializable, auditable)
//! ```
//!
//! Design rules:
//! - Never owns state
//! - Never mutates preferences
//! - Never executes commands
//! - Never bypasses Approval Gate
//! - Deterministic: same input → same output
//! - Thread-safe: all public methods are Send + Sync
//! - Fully testable: every stage is independently mockable

use std::collections::HashMap;

use crate::intent_engine::{
    AmbiguityDetector, ApprovalPreviewGenerator, ConfidenceModel, IntentClassifier, IntentPlan,
    IntentResolver,
};
use crate::preference_engine::{PreferenceSet, PreferenceStore};
use crate::recommendation_engine::{
    RecommendationContext, RecommendationEngine, RecommendationSet,
};
use crate::workflow_engine::{WorkflowDiagnostics, WorkflowPlan, WorkflowPlanner, WorkflowResult};

pub mod types;

pub use types::*;

/// The main integration pipeline.
///
/// Stateless orchestrator that wires all P6 engines together.
#[derive(Debug, Clone, Default)]
pub struct IntegrationPipeline {
    _private: (),
}

impl IntegrationPipeline {
    pub fn new() -> Self {
        IntegrationPipeline { _private: () }
    }

    /// Run the complete decision pipeline for a single user input.
    ///
    /// Returns a `PipelineResult` containing all stages' outputs.
    /// No state is modified; all outputs are immutable.
    pub fn run(
        &self,
        input: &str,
        preferences: &PreferenceSet,
        validation_config: &crate::adaptive_validation::ValidationConfig,
    ) -> PipelineResult {
        let stage_timings = std::time::Instant::now();

        // Stage 1: Intent Classification
        let classifier = IntentClassifier::new();
        let intent_plan = classifier.classify(input);
        let classify_time = stage_timings.elapsed();

        // Stage 2: Ambiguity Detection
        let ambiguity_detector = AmbiguityDetector::new();
        let ambiguity_result = ambiguity_detector.detect(&intent_plan);

        // Stage 3: Confidence Scoring
        let confidence_model = ConfidenceModel::new();
        let confidence_result = confidence_model.compute(&intent_plan);

        // Stage 4: Intent Resolution
        let resolver = IntentResolver::new();
        let resolved_commands = resolver.resolve(&intent_plan);

        // Stage 5: Recommendation Generation
        let rec_engine = RecommendationEngine::new();
        let rec_context = self.build_recommendation_context(preferences);
        let recommendation_set = rec_engine.recommend(&intent_plan, &rec_context);

        // Stage 6: Workflow Planning
        let planner = WorkflowPlanner::new();
        let workflow_diag = WorkflowDiagnostics::new(100);
        let workflow_result = planner.plan(&intent_plan, Some(&recommendation_set), &workflow_diag);

        // Stage 7: Adaptive Validation
        let validation_engine = crate::adaptive_validation::AdaptiveValidationEngine::new();
        let validation_diag = crate::adaptive_validation::AdaptiveDiagnostics::new(100);
        let validation_report = validation_engine.validate(
            &intent_plan,
            Some(&recommendation_set),
            Some(&workflow_result.plan),
            validation_config,
            &validation_diag,
        );

        // Stage 8: Approval Preview
        let preview_gen = ApprovalPreviewGenerator::new();
        let current_values = self.extract_current_values(preferences);
        let previews = preview_gen.generate_batch(&resolved_commands, &current_values);

        let total_time = stage_timings.elapsed();

        PipelineResult::new(
            input.to_string(),
            intent_plan,
            ambiguity_result,
            confidence_result,
            resolved_commands,
            recommendation_set,
            workflow_result,
            validation_report,
            previews,
            classify_time,
            total_time,
        )
    }

    /// Run the pipeline and return only the approval-ready summary.
    pub fn run_for_approval(
        &self,
        input: &str,
        preferences: &PreferenceSet,
        validation_config: &crate::adaptive_validation::ValidationConfig,
    ) -> ApprovalSummary {
        let result = self.run(input, preferences, validation_config);
        ApprovalSummary::from_pipeline_result(result)
    }

    /// Check if the pipeline result is ready for approval.
    pub fn is_approval_ready(
        &self,
        input: &str,
        preferences: &PreferenceSet,
        validation_config: &crate::adaptive_validation::ValidationConfig,
    ) -> bool {
        let result = self.run(input, preferences, validation_config);
        result.is_approval_ready()
    }

    /// Get a human-readable summary of the pipeline result.
    pub fn get_summary(
        &self,
        input: &str,
        preferences: &PreferenceSet,
        validation_config: &crate::adaptive_validation::ValidationConfig,
    ) -> String {
        let result = self.run(input, preferences, validation_config);
        result.to_string()
    }

    fn build_recommendation_context(&self, preferences: &PreferenceSet) -> RecommendationContext {
        let mut prefs = HashMap::new();
        for pref in &preferences.preferences {
            prefs.insert(pref.key.clone(), pref.value.to_string());
        }
        RecommendationContext::new().with_preferences(prefs)
    }

    fn extract_current_values(&self, preferences: &PreferenceSet) -> HashMap<String, String> {
        let mut values = HashMap::new();
        for pref in &preferences.preferences {
            values.insert(pref.key.clone(), pref.value.to_string());
        }
        values
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent_engine::IntentType;
    use crate::preference_engine::{PreferenceOrigin, PreferenceValue};
    use crate::workflow_engine::ExecutionStrategy;

    fn make_test_preferences() -> PreferenceSet {
        let mut set = PreferenceSet::new();
        set.add(crate::preference_engine::Preference::new(
            "model",
            crate::preference_engine::PreferenceCategory::Model,
            PreferenceValue::String("gpt-4o".to_string()),
            "Default model",
            PreferenceOrigin::Default,
        ));
        set.add(crate::preference_engine::Preference::new(
            "provider",
            crate::preference_engine::PreferenceCategory::Provider,
            PreferenceValue::String("openai".to_string()),
            "Default provider",
            PreferenceOrigin::Default,
        ));
        set
    }

    #[test]
    fn test_pipeline_creation() {
        let _pipeline = IntegrationPipeline::new();
    }

    #[test]
    fn test_pipeline_preference_change() {
        let pipeline = IntegrationPipeline::new();
        let prefs = make_test_preferences();
        let config = crate::adaptive_validation::ValidationConfig::new();

        let result = pipeline.run("Change the model to gpt-4o", &prefs, &config);

        assert_eq!(result.intent_plan.intent_type, IntentType::Preference);
        assert!(!result.resolved_commands.is_empty());
        assert!(result.workflow_result.plan.is_valid);
        assert_eq!(
            result.validation_report.result,
            crate::adaptive_validation::ValidationResult::Pass
        );
        assert!(result.is_approval_ready());
    }

    #[test]
    fn test_pipeline_ambiguous_input() {
        let pipeline = IntegrationPipeline::new();
        let prefs = make_test_preferences();
        let config = crate::adaptive_validation::ValidationConfig::new();

        let result = pipeline.run("Use Claude.", &prefs, &config);

        assert!(result.ambiguity_result.is_ambiguous);
        assert!(!result.is_approval_ready());
    }

    #[test]
    fn test_pipeline_help_request() {
        let pipeline = IntegrationPipeline::new();
        let prefs = make_test_preferences();
        let config = crate::adaptive_validation::ValidationConfig::new();

        let result = pipeline.run("help", &prefs, &config);

        assert_eq!(result.intent_plan.intent_type, IntentType::Help);
        assert!(!result.resolved_commands[0].requires_approval());
    }

    #[test]
    fn test_pipeline_question() {
        let pipeline = IntegrationPipeline::new();
        let prefs = make_test_preferences();
        let config = crate::adaptive_validation::ValidationConfig::new();

        let result = pipeline.run("What is rust?", &prefs, &config);

        assert_eq!(result.intent_plan.intent_type, IntentType::Question);
        assert!(!result.resolved_commands[0].requires_approval());
        // Questions are informational - they don't proceed to approval
        assert!(!result.is_approval_ready());
    }

    #[test]
    fn test_pipeline_workflow_request() {
        let pipeline = IntegrationPipeline::new();
        let prefs = make_test_preferences();
        let config = crate::adaptive_validation::ValidationConfig::new();

        let result = pipeline.run("Run the test workflow", &prefs, &config);

        assert_eq!(result.intent_plan.intent_type, IntentType::Workflow);
        assert!(result.resolved_commands[0].requires_approval());
    }

    #[test]
    fn test_pipeline_deterministic() {
        let pipeline = IntegrationPipeline::new();
        let prefs = make_test_preferences();
        let config = crate::adaptive_validation::ValidationConfig::new();

        let result1 = pipeline.run("Change the model to gpt-4o", &prefs, &config);
        let result2 = pipeline.run("Change the model to gpt-4o", &prefs, &config);

        assert_eq!(
            result1.intent_plan.intent_type,
            result2.intent_plan.intent_type
        );
        assert_eq!(
            result1.resolved_commands.len(),
            result2.resolved_commands.len()
        );
        assert_eq!(
            result1.workflow_result.plan.is_valid,
            result2.workflow_result.plan.is_valid
        );
        assert_eq!(
            result1.validation_report.result,
            result2.validation_report.result
        );
    }

    #[test]
    fn test_pipeline_no_state_mutation() {
        let pipeline = IntegrationPipeline::new();
        let prefs = make_test_preferences();
        let config = crate::adaptive_validation::ValidationConfig::new();

        let before_count = prefs.len();
        let _result = pipeline.run("Change the model to gpt-4o", &prefs, &config);

        assert_eq!(prefs.len(), before_count);
    }

    #[test]
    fn test_pipeline_empty_input() {
        let pipeline = IntegrationPipeline::new();
        let prefs = make_test_preferences();
        let config = crate::adaptive_validation::ValidationConfig::new();

        let result = pipeline.run("   ", &prefs, &config);

        assert_eq!(result.intent_plan.intent_type, IntentType::Unknown);
        assert!(result.ambiguity_result.is_ambiguous);
    }

    #[test]
    fn test_pipeline_run_for_approval() {
        let pipeline = IntegrationPipeline::new();
        let prefs = make_test_preferences();
        let config = crate::adaptive_validation::ValidationConfig::new();

        let summary = pipeline.run_for_approval("Change the model to gpt-4o", &prefs, &config);

        assert!(!summary.intent_type.is_empty());
        assert!(summary.is_ready_for_approval);
    }

    #[test]
    fn test_pipeline_is_approval_ready_true() {
        let pipeline = IntegrationPipeline::new();
        let prefs = make_test_preferences();
        let config = crate::adaptive_validation::ValidationConfig::new();

        assert!(pipeline.is_approval_ready("Change the model to gpt-4o", &prefs, &config,));
    }

    #[test]
    fn test_pipeline_is_approval_ready_false() {
        let pipeline = IntegrationPipeline::new();
        let prefs = make_test_preferences();
        let config = crate::adaptive_validation::ValidationConfig::new();

        assert!(!pipeline.is_approval_ready("Use Claude.", &prefs, &config,));
    }

    #[test]
    fn test_pipeline_get_summary() {
        let pipeline = IntegrationPipeline::new();
        let prefs = make_test_preferences();
        let config = crate::adaptive_validation::ValidationConfig::new();

        let summary = pipeline.get_summary("Change the model to gpt-4o", &prefs, &config);

        assert!(!summary.is_empty());
        assert!(summary.contains("intent="));
    }

    #[test]
    fn test_pipeline_serializable_result() {
        let pipeline = IntegrationPipeline::new();
        let prefs = make_test_preferences();
        let config = crate::adaptive_validation::ValidationConfig::new();

        let result = pipeline.run("Change the model to gpt-4o", &prefs, &config);

        let json = serde_json::to_string(&result).expect("should serialize");
        let deserialized: PipelineResult = serde_json::from_str(&json).expect("should deserialize");

        assert_eq!(
            deserialized.intent_plan.intent_type,
            result.intent_plan.intent_type
        );
        assert_eq!(
            deserialized.validation_report.result,
            result.validation_report.result
        );
    }

    #[test]
    fn test_pipeline_recommendations_generated() {
        let pipeline = IntegrationPipeline::new();
        let prefs = make_test_preferences();
        let config = crate::adaptive_validation::ValidationConfig::new();

        let result = pipeline.run("Enable dark theme", &prefs, &config);

        assert!(!result.recommendation_set.is_empty());
    }

    #[test]
    fn test_pipeline_workflow_steps_created() {
        let pipeline = IntegrationPipeline::new();
        let prefs = make_test_preferences();
        let config = crate::adaptive_validation::ValidationConfig::new();

        let result = pipeline.run("Change the model to gpt-4o", &prefs, &config);

        assert!(result.workflow_result.plan.total_steps >= 1);
    }

    #[test]
    fn test_pipeline_validation_passes() {
        let pipeline = IntegrationPipeline::new();
        let prefs = make_test_preferences();
        let config = crate::adaptive_validation::ValidationConfig::new();

        let result = pipeline.run("Change the model to gpt-4o", &prefs, &config);

        assert_eq!(
            result.validation_report.result,
            crate::adaptive_validation::ValidationResult::Pass
        );
    }

    #[test]
    fn test_pipeline_preview_generated() {
        let pipeline = IntegrationPipeline::new();
        let prefs = make_test_preferences();
        let config = crate::adaptive_validation::ValidationConfig::new();

        let result = pipeline.run("Change the model to gpt-4o", &prefs, &config);

        assert!(!result.previews.is_empty());
        assert_eq!(result.previews[0].command_kind, "update_model_preference");
    }
}
