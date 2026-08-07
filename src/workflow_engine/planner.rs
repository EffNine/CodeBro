//! Workflow Planner — composes Intent Plans into Workflow Plans.
//!
use super::dependency;
use super::diagnostics::WorkflowDiagnostics;
use super::ordering;
/// Receives IntentPlan + RecommendationSet, produces WorkflowPlan.
/// Never executes, never mutates state.
use super::types::*;
use super::validator;
use crate::intent_engine::IntentPlan;
use crate::recommendation_engine::{
    Recommendation, RecommendationConfidence, RecommendationSet, RecommendationType,
};

/// The main workflow planner.
///
/// Stateless: produces deterministic output from deterministic input.
#[derive(Debug, Clone, Default)]
pub struct WorkflowPlanner {
    _private: (),
}

impl WorkflowPlanner {
    pub fn new() -> Self {
        WorkflowPlanner { _private: () }
    }

    /// Plan a workflow from an Intent Plan and optional Recommendations.
    ///
    /// Returns a WorkflowResult containing the plan and metadata.
    pub fn plan(
        &self,
        intent_plan: &IntentPlan,
        recommendations: Option<&RecommendationSet>,
        diagnostics: &WorkflowDiagnostics,
    ) -> WorkflowResult {
        diagnostics.record_planning_started();

        // Validate inputs
        let validation_issues = validator::validate_inputs(intent_plan, recommendations);
        if !validation_issues.is_empty() {
            diagnostics.record_validation_failure(&validation_issues);
            let plan = WorkflowPlan::new(
                Self::generate_plan_id(intent_plan),
                &intent_plan.id,
                vec![],
                vec![],
                ExecutionStrategy::Sequential,
                validation_issues,
                vec![],
            );
            return WorkflowResult::new(plan, WorkflowMetadata::default());
        }

        // Generate steps from intent plan commands
        let mut steps = self.generate_steps_from_commands(intent_plan);

        // Generate steps from recommendations
        if let Some(recs) = recommendations {
            let rec_steps = self.generate_steps_from_recommendations(recs, intent_plan);
            steps.extend(rec_steps);
        }

        // Detect and add dependencies
        let dependencies = dependency::build_dependencies(&steps);

        // Validate the complete plan
        let issues = validator::validate_plan(&steps, &dependencies);
        let warnings = validator::generate_warnings(&steps, &dependencies);

        // Apply ordering
        let ordered_steps = ordering::topological_sort(steps, &dependencies);

        // Determine execution strategy
        let strategy = self.determine_strategy(&ordered_steps, &dependencies);

        // Build the plan
        let plan = WorkflowPlan::new(
            Self::generate_plan_id(intent_plan),
            &intent_plan.id,
            ordered_steps,
            dependencies,
            strategy,
            issues,
            warnings,
        );

        diagnostics.record_planning_completed(&plan);

        WorkflowResult::new(
            plan,
            WorkflowMetadata {
                source_intent: intent_plan.id.clone(),
                source_recommendation_count: recommendations.map(|r| r.len()).unwrap_or(0),
                planner_version: "1.0.0".to_string(),
                planning_rules_applied: vec![
                    "command_extraction".to_string(),
                    "dependency_analysis".to_string(),
                ],
            },
        )
    }

    /// Generate workflow steps from intent plan commands.
    fn generate_steps_from_commands(&self, plan: &IntentPlan) -> Vec<WorkflowStep> {
        let mut steps = Vec::new();

        for (i, cmd) in plan.required_commands.iter().enumerate() {
            let step = match cmd {
                crate::intent_engine::IntentCommand::UpdateModelPreference {
                    key,
                    new_value,
                    reason,
                } => WorkflowStep::new(
                    &format!("Update model preference: {}", key),
                    &format!("update_preference:{}={}", key, new_value),
                    WorkflowStage::Execution,
                    i as u32,
                    vec![],
                    true,
                    0.0,
                    true,
                    reason,
                ),
                crate::intent_engine::IntentCommand::UpdateLanguagePreference {
                    key,
                    new_value,
                    reason,
                } => WorkflowStep::new(
                    &format!("Update language preference: {}", key),
                    &format!("update_preference:{}={}", key, new_value),
                    WorkflowStage::Execution,
                    i as u32,
                    vec![],
                    true,
                    0.0,
                    true,
                    reason,
                ),
                crate::intent_engine::IntentCommand::UpdateCostPreference {
                    key,
                    new_value,
                    reason,
                } => WorkflowStep::new(
                    &format!("Update cost preference: {}", key),
                    &format!("update_preference:{}={}", key, new_value),
                    WorkflowStage::Execution,
                    i as u32,
                    vec![],
                    true,
                    0.5,
                    true,
                    reason,
                ),
                crate::intent_engine::IntentCommand::UpdateApprovalPreference {
                    key,
                    new_value,
                    reason,
                } => WorkflowStep::new(
                    &format!("Update approval preference: {}", key),
                    &format!("update_preference:{}={}", key, new_value),
                    WorkflowStage::Execution,
                    i as u32,
                    vec![],
                    true,
                    0.0,
                    true,
                    reason,
                ),
                crate::intent_engine::IntentCommand::ExecuteWorkflow {
                    workflow_id,
                    reason,
                } => WorkflowStep::new(
                    &format!("Execute workflow: {}", workflow_id),
                    &format!("run_workflow:{}", workflow_id),
                    WorkflowStage::Execution,
                    i as u32,
                    vec![],
                    true,
                    1.0,
                    false,
                    reason,
                ),
                crate::intent_engine::IntentCommand::ExecuteCommand { command, reason } => {
                    WorkflowStep::new(
                        &format!("Execute command"),
                        command,
                        WorkflowStage::Execution,
                        i as u32,
                        vec![],
                        true,
                        0.0,
                        false,
                        reason,
                    )
                }
                crate::intent_engine::IntentCommand::AnswerQuestion { .. }
                | crate::intent_engine::IntentCommand::ProvideHelp { .. } => {
                    continue; // Informational, no workflow step needed
                }
            };
            steps.push(step);
        }

        steps
    }

    /// Generate workflow steps from recommendations.
    fn generate_steps_from_recommendations(
        &self,
        recs: &RecommendationSet,
        intent_plan: &IntentPlan,
    ) -> Vec<WorkflowStep> {
        let mut steps = Vec::new();
        let base_index = intent_plan.required_commands.len() as u32;

        for (i, rec) in recs.recommendations.iter().enumerate() {
            if let (Some(ref key), Some(ref value)) = (&rec.target_key, &rec.target_value) {
                let step = WorkflowStep::new(
                    &format!("Apply recommendation: {}", rec.title),
                    &format!("apply_recommendation:{}={}", key, value),
                    WorkflowStage::Execution,
                    base_index + i as u32,
                    vec![],
                    rec.is_strong(),
                    0.0,
                    true,
                    &rec.explanation,
                );
                steps.push(step);
            }
        }

        steps
    }

    /// Determine the execution strategy based on step characteristics.
    fn determine_strategy(
        &self,
        steps: &[WorkflowStep],
        dependencies: &[WorkflowDependency],
    ) -> ExecutionStrategy {
        // If there are no dependencies and all steps are independent, use parallel
        if dependencies.is_empty() && steps.len() > 1 {
            // Check if all steps are reversible (safe for parallel)
            if steps.iter().all(|s| s.reversible) {
                return ExecutionStrategy::Parallel;
            }
        }

        // If there are dependencies, use dependency-ordered
        if !dependencies.is_empty() {
            return ExecutionStrategy::DependencyOrdered;
        }

        // Default to sequential
        ExecutionStrategy::Sequential
    }

    /// Generate a deterministic plan ID from the intent plan.
    fn generate_plan_id(intent_plan: &IntentPlan) -> String {
        // Use a deterministic hash based on intent ID and command count
        let mut hash: u64 = 14695981039346656037;
        for byte in intent_plan.id.bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(1099511628211);
        }
        hash ^= intent_plan.required_commands.len() as u64;
        hash = hash.wrapping_mul(1099511628211);
        format!("plan_{:x}", hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_planner_creation() {
        let _planner = WorkflowPlanner::new();
    }

    #[test]
    fn test_planner_empty_plan() {
        let planner = WorkflowPlanner::new();
        let intent = IntentPlan::new(
            "test-1".to_string(),
            "xyz random",
            crate::intent_engine::IntentType::Unknown,
            "unknown",
            false,
            0.0,
            0.1,
            true,
            Some("No match".to_string()),
            "No classification",
            vec![],
            vec![],
        );
        let diag = WorkflowDiagnostics::new(100);
        let result = planner.plan(&intent, None, &diag);
        assert!(!result.plan.is_valid);
    }

    #[test]
    fn test_planner_preference_change() {
        let planner = WorkflowPlanner::new();
        let intent = IntentPlan::new(
            "test-2".to_string(),
            "Change the model to gpt-4o",
            crate::intent_engine::IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Model change",
            vec!["Rule match".to_string()],
            vec![crate::intent_engine::IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "gpt-4o".to_string(),
                reason: "User requested".to_string(),
            }],
        );
        let diag = WorkflowDiagnostics::new(100);
        let result = planner.plan(&intent, None, &diag);
        assert!(result.plan.is_valid);
        assert_eq!(result.plan.total_steps, 1);
        assert!(result.approval_required);
    }

    #[test]
    fn test_planner_deterministic() {
        let planner = WorkflowPlanner::new();
        let intent = IntentPlan::new(
            "test-3".to_string(),
            "Change the model to gpt-4o",
            crate::intent_engine::IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Model change",
            vec!["Rule match".to_string()],
            vec![crate::intent_engine::IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "gpt-4o".to_string(),
                reason: "User requested".to_string(),
            }],
        );
        let diag = WorkflowDiagnostics::new(100);
        let result1 = planner.plan(&intent, None, &diag);
        let result2 = planner.plan(&intent, None, &diag);

        assert_eq!(result1.plan.plan_id, result2.plan.plan_id);
        assert_eq!(result1.plan.total_steps, result2.plan.total_steps);
        assert_eq!(result1.plan.is_valid, result2.plan.is_valid);
        assert_eq!(result1.plan.strategy, result2.plan.strategy);
    }

    #[test]
    fn test_planner_no_state_mutation() {
        let planner = WorkflowPlanner::new();
        let intent = IntentPlan::new(
            "test-4".to_string(),
            "Change the model",
            crate::intent_engine::IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Change",
            vec![],
            vec![crate::intent_engine::IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "gpt-4o".to_string(),
                reason: "R".to_string(),
            }],
        );
        let diag = WorkflowDiagnostics::new(100);

        let _result = planner.plan(&intent, None, &diag);

        // Intent must not be mutated
        assert_eq!(intent.required_commands.len(), 1);
        assert!(matches!(
            &intent.required_commands[0],
            crate::intent_engine::IntentCommand::UpdateModelPreference { .. }
        ));
    }

    #[test]
    fn test_planner_with_recommendations() {
        let planner = WorkflowPlanner::new();
        let intent = IntentPlan::new(
            "test-5".to_string(),
            "Change model and apply recommendations",
            crate::intent_engine::IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Model change",
            vec!["Rule match".to_string()],
            vec![crate::intent_engine::IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "gpt-4o".to_string(),
                reason: "User requested".to_string(),
            }],
        );
        let mut rec_set = RecommendationSet::new("test-5");
        rec_set.add(Recommendation::new(
            RecommendationType::General,
            "Test Rec",
            "Test",
            vec![],
            RecommendationConfidence::High(0.9),
            "rule-1",
            Some("setting1".to_string()),
            Some("value1".to_string()),
            "test-5",
        ));
        let diag = WorkflowDiagnostics::new(100);
        let result = planner.plan(&intent, Some(&rec_set), &diag);
        assert!(result.plan.total_steps >= 1);
    }
}
