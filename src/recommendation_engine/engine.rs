//! Recommendation Engine — main orchestration module.
//!
use super::filter;
use super::ranking;
/// Consumes Intent Plans and produces optional, deterministic recommendations.
///
/// This module is an observer: it never modifies state, never executes commands,
/// and never bypasses the Approval Gate.
use super::rules;
use super::types::*;
use crate::intent_engine::IntentPlan;

/// The main recommendation engine.
///
/// Stateless observer that converts Intent Plans into RecommendationSets.
#[derive(Debug, Clone, Default)]
pub struct RecommendationEngine {
    _private: (),
}

impl RecommendationEngine {
    pub fn new() -> Self {
        RecommendationEngine { _private: () }
    }

    /// Process an Intent Plan and produce a RecommendationSet.
    ///
    /// Returns an empty set if no recommendations apply.
    /// The original plan is never modified.
    pub fn recommend(
        &self,
        plan: &IntentPlan,
        context: &RecommendationContext,
    ) -> RecommendationSet {
        let mut result = RecommendationSet::new(&plan.id);

        // Collect raw recommendations from all rules
        let mut raw: Vec<Recommendation> = Vec::new();

        // Add recommendations from the main goal text
        let goal_recs = rules::generate_from_rules(&plan.detected_goal, &plan.id, context);
        raw.extend(goal_recs);

        // Add recommendations from each command's reason
        for cmd in &plan.required_commands {
            let cmd_recs = rules::generate_from_commands(cmd, &plan.id, context);
            raw.extend(cmd_recs);
        }

        // Add recommendations based on intent type
        let type_recs = rules::generate_from_intent_type(&plan.intent_type, &plan.id, context);
        raw.extend(type_recs);

        // Rank recommendations
        let ranked = ranking::rank(raw.clone());

        // Filter recommendations
        let filtered = filter::filter(ranked, context);

        result.filtered_count = raw.len().saturating_sub(filtered.len());
        result.recommendations = filtered;

        result
    }

    /// Check if the engine has any recommendations for this plan.
    pub fn has_recommendations(&self, plan: &IntentPlan, context: &RecommendationContext) -> bool {
        !self.recommend(plan, context).is_empty()
    }

    /// Get recommendation count without generating the full set.
    pub fn count_recommendations(
        &self,
        plan: &IntentPlan,
        context: &RecommendationContext,
    ) -> usize {
        self.recommend(plan, context).len()
    }
}

/// Extend rules module to handle command-specific and type-specific recommendations.
#[allow(dead_code)]
mod rules_ext {
    use super::*;

    /// Generate recommendations based on command kinds in the plan.
    pub fn generate_from_commands(
        command: &crate::intent_engine::IntentCommand,
        intent_id: &str,
        context: &RecommendationContext,
    ) -> Vec<Recommendation> {
        let mut recs = Vec::new();

        match command {
            crate::intent_engine::IntentCommand::ExecuteCommand { command, reason } => {
                let cmd_lower = command.to_lowercase();
                if cmd_lower.contains("cargo") || cmd_lower.contains("rustup") {
                    recs.push(Recommendation::new(
                        RecommendationType::Integration,
                        "Enable Rust Toolchain",
                        "Cargo commands detected — consider enabling Rust-specific tooling.",
                        vec!["Detected cargo command execution".to_string()],
                        RecommendationConfidence::High(0.85),
                        "rule-cargo-detect",
                        Some("language_rust".to_string()),
                        Some("true".to_string()),
                        intent_id,
                    ));
                }
                if cmd_lower.contains("python") || cmd_lower.contains("pip") {
                    recs.push(Recommendation::new(
                        RecommendationType::Integration,
                        "Enable Python Toolchain",
                        "Python commands detected — consider enabling Python-specific tooling.",
                        vec!["Detected python/pip command execution".to_string()],
                        RecommendationConfidence::High(0.85),
                        "rule-python-detect",
                        Some("language_python".to_string()),
                        Some("true".to_string()),
                        intent_id,
                    ));
                }
                if cmd_lower.contains("test") {
                    recs.push(Recommendation::new(
                        RecommendationType::Workflow,
                        "Enable Test Runner",
                        "Test commands detected — consider enabling integrated test runner.",
                        vec!["Detected test command execution".to_string()],
                        RecommendationConfidence::Medium(0.75),
                        "rule-test-detect",
                        Some("test_runner_integration".to_string()),
                        Some("true".to_string()),
                        intent_id,
                    ));
                }
            }
            crate::intent_engine::IntentCommand::UpdateModelPreference { new_value, .. } => {
                let val_lower = new_value.to_lowercase();
                if val_lower.contains("claude") || val_lower.contains("anthropic") {
                    recs.push(Recommendation::new(
                        RecommendationType::General,
                        "Enable Claude-Specific Settings",
                        "Claude model detected — consider enabling Claude-optimized settings.",
                        vec!["Model preference set to Claude".to_string()],
                        RecommendationConfidence::Medium(0.70),
                        "rule-claude-detect",
                        Some("model_claude_optimized".to_string()),
                        Some("true".to_string()),
                        intent_id,
                    ));
                }
                if val_lower.contains("gpt") || val_lower.contains("openai") {
                    recs.push(Recommendation::new(
                        RecommendationType::General,
                        "Enable GPT-Specific Settings",
                        "GPT model detected — consider enabling GPT-optimized settings.",
                        vec!["Model preference set to GPT".to_string()],
                        RecommendationConfidence::Medium(0.70),
                        "rule-gpt-detect",
                        Some("model_gpt_optimized".to_string()),
                        Some("true".to_string()),
                        intent_id,
                    ));
                }
            }
            crate::intent_engine::IntentCommand::UpdateApprovalPreference { new_value, .. } => {
                if *new_value {
                    recs.push(Recommendation::new(
                        RecommendationType::General,
                        "Enable Approval Automation",
                        "Auto-approve enabled — consider setting confidence thresholds for automatic approvals.",
                        vec!["Approval preference set to true".to_string()],
                        RecommendationConfidence::Medium(0.65),
                        "rule-auto-approve",
                        Some("auto_approve_threshold".to_string()),
                        Some("0.8".to_string()),
                        intent_id,
                    ));
                }
            }
            _ => {}
        }

        recs
    }

    /// Generate recommendations based on intent type.
    pub fn generate_from_intent_type(
        intent_type: &crate::intent_engine::IntentType,
        intent_id: &str,
        context: &RecommendationContext,
    ) -> Vec<Recommendation> {
        let mut recs = Vec::new();

        match intent_type {
            crate::intent_engine::IntentType::Preference => {
                recs.push(Recommendation::new(
                    RecommendationType::General,
                    "Review Preference Impact",
                    "Preference changes affect the entire session — consider reviewing all pending changes before approval.",
                    vec!["Preference intent detected".to_string()],
                    RecommendationConfidence::Low(0.50),
                    "rule-preference-review",
                    None,
                    None,
                    intent_id,
                ));
            }
            crate::intent_engine::IntentType::Execution => {
                recs.push(Recommendation::new(
                    RecommendationType::General,
                    "Execution Preview",
                    "Execution intents may have side effects — review the command before approval.",
                    vec!["Execution intent detected".to_string()],
                    RecommendationConfidence::Low(0.55),
                    "rule-execution-review",
                    None,
                    None,
                    intent_id,
                ));
            }
            crate::intent_engine::IntentType::Workflow => {
                recs.push(Recommendation::new(
                    RecommendationType::Workflow,
                    "Workflow Safety Check",
                    "Workflow execution may affect multiple files — consider enabling dry-run mode.",
                    vec!["Workflow intent detected".to_string()],
                    RecommendationConfidence::Medium(0.65),
                    "rule-workflow-safety",
                    Some("workflow_dry_run".to_string()),
                    Some("true".to_string()),
                    intent_id,
                ));
            }
            crate::intent_engine::IntentType::Question => {
                recs.push(Recommendation::new(
                    RecommendationType::General,
                    "Related Documentation",
                    "Questions may benefit from related documentation links.",
                    vec!["Question intent detected".to_string()],
                    RecommendationConfidence::Low(0.45),
                    "rule-question-docs",
                    None,
                    None,
                    intent_id,
                ));
            }
            crate::intent_engine::IntentType::Help => {
                recs.push(Recommendation::new(
                    RecommendationType::General,
                    "Quick Reference",
                    "Help requests may benefit from a quick reference card.",
                    vec!["Help intent detected".to_string()],
                    RecommendationConfidence::Low(0.40),
                    "rule-help-reference",
                    None,
                    None,
                    intent_id,
                ));
            }
            _ => {}
        }

        recs
    }
}

pub use rules_ext::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engine_creation() {
        let _engine = RecommendationEngine::new();
    }

    #[test]
    fn test_engine_empty_plan() {
        let engine = RecommendationEngine::new();
        let plan = IntentPlan::new(
            "test-1".to_string(),
            "xyz random gibberish",
            crate::intent_engine::IntentType::Unknown,
            "unknown",
            false,
            0.0,
            0.1,
            true,
            Some("No match".to_string()),
            "No classification",
            vec!["No rule matched".to_string()],
            vec![],
        );
        let context = RecommendationContext::new();
        let result = engine.recommend(&plan, &context);
        assert!(result.is_empty());
    }

    #[test]
    fn test_engine_preference_model_change() {
        let engine = RecommendationEngine::new();
        let plan = IntentPlan::new(
            "test-2".to_string(),
            "Change the model to claude-3-opus",
            crate::intent_engine::IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Model preference update",
            vec!["Rule match: model change".to_string()],
            vec![crate::intent_engine::IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: "claude-3-opus".to_string(),
                reason: "User requested".to_string(),
            }],
        );
        let context = RecommendationContext::new();
        let result = engine.recommend(&plan, &context);
        assert!(!result.is_empty());
    }

    #[test]
    fn test_engine_dark_theme() {
        let engine = RecommendationEngine::new();
        let plan = IntentPlan::new(
            "test-3".to_string(),
            "Enable dark theme",
            crate::intent_engine::IntentType::Configuration,
            "configuration",
            false,
            0.0,
            0.8,
            false,
            None,
            "Dark theme configuration",
            vec!["Rule match: dark theme".to_string()],
            vec![],
        );
        let mut context = RecommendationContext::new();
        context.include_low_confidence = true;
        let result = engine.recommend(&plan, &context);
        assert!(
            !result.is_empty(),
            "Expected recommendations for dark theme, got {}",
            result.len()
        );
        assert!(result
            .recommendations
            .iter()
            .any(|r| matches!(r.rec_type, RecommendationType::Appearance)));
    }

    #[test]
    fn test_engine_vim_mode() {
        let engine = RecommendationEngine::new();
        let plan = IntentPlan::new(
            "test-4".to_string(),
            "Enable vim mode",
            crate::intent_engine::IntentType::Configuration,
            "configuration",
            false,
            0.0,
            0.85,
            false,
            None,
            "Vim mode configuration",
            vec!["Rule match: vim mode".to_string()],
            vec![],
        );
        let context = RecommendationContext::new();
        let result = engine.recommend(&plan, &context);
        assert!(!result.is_empty());
        assert!(result
            .recommendations
            .iter()
            .any(|r| matches!(r.rec_type, RecommendationType::Keyboard)));
    }

    #[test]
    fn test_engine_no_state_mutation() {
        let engine = RecommendationEngine::new();
        let mut context = RecommendationContext::new();
        context
            .preferences
            .insert("model".to_string(), "gpt-4o".to_string());

        let plan = IntentPlan::new(
            "test-5".to_string(),
            "Change the model to claude",
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
                new_value: "claude".to_string(),
                reason: "User requested".to_string(),
            }],
        );

        let _result = engine.recommend(&plan, &context);

        // Context must not be mutated
        assert_eq!(
            context.preferences.get("model"),
            Some(&"gpt-4o".to_string())
        );
    }

    #[test]
    fn test_engine_deterministic() {
        let engine = RecommendationEngine::new();
        let plan = IntentPlan::new(
            "test-6".to_string(),
            "Enable dark theme",
            crate::intent_engine::IntentType::Configuration,
            "configuration",
            false,
            0.0,
            0.8,
            false,
            None,
            "Dark theme",
            vec!["Rule match".to_string()],
            vec![],
        );
        let context = RecommendationContext::new();

        let result1 = engine.recommend(&plan, &context);
        let result2 = engine.recommend(&plan, &context);

        assert_eq!(result1.len(), result2.len());
        for (r1, r2) in result1
            .recommendations
            .iter()
            .zip(result2.recommendations.iter())
        {
            assert_eq!(r1.title, r2.title);
            assert_eq!(r1.rec_type, r2.rec_type);
            assert!((r1.confidence.score() - r2.confidence.score()).abs() < 0.001);
        }
    }

    #[test]
    fn test_has_recommendations_true() {
        let engine = RecommendationEngine::new();
        let plan = IntentPlan::new(
            "test-7".to_string(),
            "Enable vim mode",
            crate::intent_engine::IntentType::Configuration,
            "configuration",
            false,
            0.0,
            0.85,
            false,
            None,
            "Vim mode",
            vec!["Rule match".to_string()],
            vec![],
        );
        let context = RecommendationContext::new();
        assert!(engine.has_recommendations(&plan, &context));
    }

    #[test]
    fn test_has_recommendations_false() {
        let engine = RecommendationEngine::new();
        let plan = IntentPlan::new(
            "test-8".to_string(),
            "xyz random gibberish",
            crate::intent_engine::IntentType::Unknown,
            "unknown",
            false,
            0.0,
            0.1,
            true,
            Some("No match".to_string()),
            "No classification",
            vec!["No rule matched".to_string()],
            vec![],
        );
        let context = RecommendationContext::new();
        assert!(!engine.has_recommendations(&plan, &context));
    }

    #[test]
    fn test_count_recommendations() {
        let engine = RecommendationEngine::new();
        let plan = IntentPlan::new(
            "test-9".to_string(),
            "Enable git integration",
            crate::intent_engine::IntentType::Configuration,
            "configuration",
            false,
            0.0,
            0.8,
            false,
            None,
            "Git integration",
            vec!["Rule match".to_string()],
            vec![],
        );
        let context = RecommendationContext::new();
        let count = engine.count_recommendations(&plan, &context);
        assert!(count >= 1);
    }
}
