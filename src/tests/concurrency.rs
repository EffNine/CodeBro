//! Concurrency and thread-safety tests for the P6 decision pipeline.
//!
//! These tests verify that all engines are safe to use from multiple threads
//! simultaneously without data races or inconsistent state.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use codebro::intent_engine::{IntentClassifier, IntentPlan, IntentType, IntentResolver};
use codebro::recommendation_engine::{RecommendationContext, RecommendationEngine, RecommendationSet};
use codebro::workflow_engine::{WorkflowDiagnostics, WorkflowPlanner, WorkflowResult};
use codebro::adaptive_validation::{AdaptiveDiagnostics, AdaptiveValidationEngine, ValidationConfig, ValidationResult};
use codebro::preference_engine::{PreferenceSet, PreferenceValue};
use codebro::integration_pipeline::IntegrationPipeline;

// ─── Thread-Safety Tests ─────────────────────────────────────────────────────

#[test]
fn test_intent_classifier_thread_safe() {
    let classifier = IntentClassifier::new();
    let arc_classifier = Arc::new(classifier);

    let mut handles = vec![];
    for i in 0..10 {
        let clone = arc_classifier.clone();
        handles.push(thread::spawn(move || {
            let input = format!("Change model to gpt-4o-{}", i);
            let plan = clone.classify(&input);
            assert_eq!(plan.intent_type, IntentType::Preference);
        }));
    }

    for handle in handles {
        handle.join().expect("thread panicked");
    }
}

#[test]
fn test_recommendation_engine_thread_safe() {
    let engine = RecommendationEngine::new();
    let arc_engine = Arc::new(engine);

    let mut handles = vec![];
    for i in 0..10 {
        let clone = arc_engine.clone();
        let plan = IntentPlan::new(
            format!("plan-{}", i),
            &format!("Enable dark theme {}", i),
            IntentType::Configuration,
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
        handles.push(thread::spawn(move || {
            let result = clone.recommend(&plan, &context);
            assert!(!result.is_empty());
        }));
    }

    for handle in handles {
        handle.join().expect("thread panicked");
    }
}

#[test]
fn test_workflow_planner_thread_safe() {
    let planner = WorkflowPlanner::new();
    let arc_planner = Arc::new(planner);

    let mut handles = vec![];
    for i in 0..10 {
        let clone = arc_planner.clone();
        let plan = IntentPlan::new(
            format!("plan-{}", i),
            &format!("Change model to gpt-4o-{}", i),
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Model change",
            vec!["Rule match".to_string()],
            vec![codebro::intent_engine::IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: format!("gpt-4o-{}", i),
                reason: "User requested".to_string(),
            }],
        );
        let diag = WorkflowDiagnostics::new(100);
        handles.push(thread::spawn(move || {
            let result = clone.plan(&plan, None, &diag);
            assert!(result.plan.is_valid);
        }));
    }

    for handle in handles {
        handle.join().expect("thread panicked");
    }
}

#[test]
fn test_adaptive_validation_thread_safe() {
    let engine = AdaptiveValidationEngine::new();
    let arc_engine = Arc::new(engine);
    let config = ValidationConfig::new();

    let mut handles = vec![];
    for i in 0..10 {
        let clone = arc_engine.clone();
        let plan = IntentPlan::new(
            format!("plan-{}", i),
            &format!("Change model to gpt-4o-{}", i),
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Model change",
            vec!["Rule match".to_string()],
            vec![codebro::intent_engine::IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: format!("gpt-4o-{}", i),
                reason: "User requested".to_string(),
            }],
        );
        let diag = AdaptiveDiagnostics::new(100);
        handles.push(thread::spawn(move || {
            let report = clone.validate(&plan, None, None, &config, &diag);
            assert_eq!(report.result, ValidationResult::Pass);
        }));
    }

    for handle in handles {
        handle.join().expect("thread panicked");
    }
}

#[test]
fn test_integration_pipeline_thread_safe() {
    let pipeline = IntegrationPipeline::new();
    let arc_pipeline = Arc::new(pipeline);

    let mut prefs = PreferenceSet::new();
    prefs.add(codebro::preference_engine::Preference::new(
        "model",
        codebro::preference_engine::PreferenceCategory::Model,
        PreferenceValue::String("gpt-4o".to_string()),
        "Default model",
        codebro::preference_engine::PreferenceOrigin::Default,
    ));
    let arc_prefs = Arc::new(prefs);
    let config = ValidationConfig::new();

    let mut handles = vec![];
    for i in 0..10 {
        let clone = arc_pipeline.clone();
        let prefs_clone = arc_prefs.clone();
        handles.push(thread::spawn(move || {
            let result = clone.run(
                &format!("Change model to gpt-4o-{}", i),
                &prefs_clone,
                &config,
            );
            assert!(result.intent_plan.intent_type == IntentType::Preference
                || result.intent_plan.intent_type == IntentType::Unknown);
        }));
    }

    for handle in handles {
        handle.join().expect("thread panicked");
    }
}

#[test]
fn test_concurrent_pipeline_runs_no_data_race() {
    let pipeline = IntegrationPipeline::new();
    let arc_pipeline = Arc::new(pipeline);

    let mut prefs = PreferenceSet::new();
    prefs.add(codebro::preference_engine::Preference::new(
        "model",
        codebro::preference_engine::PreferenceCategory::Model,
        PreferenceValue::String("gpt-4o".to_string()),
        "Default model",
        codebro::preference_engine::PreferenceOrigin::Default,
    ));
    let arc_prefs = Arc::new(prefs);
    let config = ValidationConfig::new();

    let mut handles = vec![];
    for i in 0..20 {
        let clone = arc_pipeline.clone();
        let prefs_clone = arc_prefs.clone();
        handles.push(thread::spawn(move || {
            for j in 0..50 {
                let input = format!("Change model to gpt-4o-{}-{}", i, j);
                let _result = clone.run(&input, &prefs_clone, &config);
            }
        }));
    }

    for handle in handles {
        handle.join().expect("thread panicked");
    }
}

// ─── Stress Tests ────────────────────────────────────────────────────────────

#[test]
fn test_stress_intent_classification() {
    let classifier = IntentClassifier::new();
    let inputs = vec![
        "Change the model to gpt-4o",
        "Enable dark theme",
        "Run the test workflow",
        "What is rust?",
        "help",
        "Configure the system",
        "Execute cargo test",
        "Switch to vim mode",
        "Use claude-3-opus",
        "Change language to japanese",
    ];

    for input in inputs {
        let plan = classifier.classify(input);
        assert!(!plan.id.is_empty());
        assert!(!plan.detected_goal.is_empty());
    }
}

#[test]
fn test_stress_recommendation_generation() {
    let engine = RecommendationEngine::new();
    let inputs = vec![
        "Enable dark theme",
        "Use vim mode",
        "Configure git integration",
        "Enable LSP features",
        "Make it fast",
        "Use rust",
        "Enable accessibility",
    ];

    for input in inputs {
        let plan = IntentPlan::new(
            "stress-test".to_string(),
            input,
            IntentType::Configuration,
            "configuration",
            false,
            0.0,
            0.8,
            false,
            None,
            "Stress test",
            vec!["Rule match".to_string()],
            vec![],
        );
        let context = RecommendationContext::new();
        let result = engine.recommend(&plan, &context);
        assert!(!result.intent_id.is_empty());
    }
}

#[test]
fn test_stress_workflow_planning() {
    let planner = WorkflowPlanner::new();
    let inputs = vec![
        "Change the model to gpt-4o",
        "Change the model to claude-3-opus",
        "Change language to french",
        "Enable auto approve",
    ];

    for input in inputs {
        let plan = IntentPlan::new(
            "stress-plan".to_string(),
            input,
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Stress test",
            vec!["Rule match".to_string()],
            vec![codebro::intent_engine::IntentCommand::UpdateModelPreference {
                key: "model".to_string(),
                new_value: input.replace("Change the model to ", ""),
                reason: "User requested".to_string(),
            }],
        );
        let diag = WorkflowDiagnostics::new(100);
        let result = planner.plan(&plan, None, &diag);
        assert!(result.plan.plan_id.starts_with("plan_"));
    }
}

#[test]
fn test_stress_validation() {
    let engine = AdaptiveValidationEngine::new();
    let inputs = vec![
        "Change the model to gpt-4o",
        "Enable dark theme",
        "Run the test workflow",
    ];

    for input in inputs {
        let plan = IntentPlan::new(
            "stress-val".to_string(),
            input,
            IntentType::Preference,
            "preference_engine",
            true,
            0.0,
            0.9,
            false,
            None,
            "Stress test",
            vec!["Rule match".to_string()],
            vec![],
        );
        let config = ValidationConfig::new();
        let diag = AdaptiveDiagnostics::new(100);
        let report = engine.validate(&plan, None, None, &config, &diag);
        assert!(!report.report_id.is_empty());
    }
}

// ─── Determinism Tests ───────────────────────────────────────────────────────

#[test]
fn test_deterministic_intent_classification() {
    let classifier = IntentClassifier::new();
    let input = "Change the model to gpt-4o";

    let plan1 = classifier.classify(input);
    let plan2 = classifier.classify(input);

    assert_eq!(plan1.intent_type, plan2.intent_type);
    assert_eq!(plan1.confidence, plan2.confidence);
    assert_eq!(plan1.required_commands.len(), plan2.required_commands.len());
    assert_eq!(plan1.ambiguity, plan2.ambiguity);
}

#[test]
fn test_deterministic_recommendation_generation() {
    let engine = RecommendationEngine::new();
    let plan = IntentPlan::new(
        "det-rec".to_string(),
        "Enable dark theme",
        IntentType::Configuration,
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
    for (r1, r2) in result1.recommendations.iter().zip(result2.recommendations.iter()) {
        assert_eq!(r1.title, r2.title);
        assert_eq!(r1.rec_type, r2.rec_type);
        assert!((r1.confidence.score() - r2.confidence.score()).abs() < 0.001);
    }
}

#[test]
fn test_deterministic_workflow_planning() {
    let planner = WorkflowPlanner::new();
    let plan = IntentPlan::new(
        "det-wf".to_string(),
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
        vec![codebro::intent_engine::IntentCommand::UpdateModelPreference {
            key: "model".to_string(),
            new_value: "gpt-4o".to_string(),
            reason: "User requested".to_string(),
        }],
    );
    let diag = WorkflowDiagnostics::new(100);

    let result1 = planner.plan(&plan, None, &diag);
    let result2 = planner.plan(&plan, None, &diag);

    assert_eq!(result1.plan.plan_id, result2.plan.plan_id);
    assert_eq!(result1.plan.total_steps, result2.plan.total_steps);
    assert_eq!(result1.plan.is_valid, result2.plan.is_valid);
    assert_eq!(result1.plan.strategy, result2.plan.strategy);
}

#[test]
fn test_deterministic_validation() {
    let engine = AdaptiveValidationEngine::new();
    let plan = IntentPlan::new(
        "det-val".to_string(),
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
        vec![],
    );
    let config = ValidationConfig::new();
    let diag = AdaptiveDiagnostics::new(100);

    let report1 = engine.validate(&plan, None, None, &config, &diag);
    let report2 = engine.validate(&plan, None, None, &config, &diag);

    assert_eq!(report1.result, report2.result);
    assert_eq!(report1.issues.len(), report2.issues.len());
    assert_eq!(report1.warnings.len(), report2.warnings.len());
}

#[test]
fn test_deterministic_pipeline() {
    let pipeline = IntegrationPipeline::new();
    let mut prefs = PreferenceSet::new();
    prefs.add(codebro::preference_engine::Preference::new(
        "model",
        codebro::preference_engine::PreferenceCategory::Model,
        PreferenceValue::String("gpt-4o".to_string()),
        "Default model",
        codebro::preference_engine::PreferenceOrigin::Default,
    ));
    let config = ValidationConfig::new();

    let result1 = pipeline.run("Change the model to gpt-4o", &prefs, &config);
    let result2 = pipeline.run("Change the model to gpt-4o", &prefs, &config);

    assert_eq!(result1.intent_plan.intent_type, result2.intent_plan.intent_type);
    assert_eq!(result1.resolved_commands.len(), result2.resolved_commands.len());
    assert_eq!(result1.workflow_result.plan.is_valid, result2.workflow_result.plan.is_valid);
    assert_eq!(result1.validation_report.result, result2.validation_report.result);
}

// ─── Error Handling Tests ────────────────────────────────────────────────────

#[test]
fn test_pipeline_handles_empty_input() {
    let pipeline = IntegrationPipeline::new();
    let prefs = PreferenceSet::default();
    let config = ValidationConfig::new();

    let result = pipeline.run("", &prefs, &config);
    assert_eq!(result.intent_plan.intent_type, IntentType::Unknown);
    assert!(result.ambiguity_result.is_ambiguous);
}

#[test]
fn test_pipeline_handles_whitespace_input() {
    let pipeline = IntegrationPipeline::new();
    let prefs = PreferenceSet::default();
    let config = ValidationConfig::new();

    let result = pipeline.run("   \n\t  ", &prefs, &config);
    assert_eq!(result.intent_plan.intent_type, IntentType::Unknown);
}

#[test>
fn test_pipeline_handles_random_garbage() {
    let pipeline = IntegrationPipeline::new();
    let prefs = PreferenceSet::default();
    let config = ValidationConfig::new();

    let result = pipeline.run("xyz123!@#$%^&*()", &prefs, &config);
    assert_eq!(result.intent_plan.intent_type, IntentType::Unknown);
    assert!(result.confidence_result.score < 0.5);
}

#[test]
fn test_pipeline_preserves_all_stages_output() {
    let pipeline = IntegrationPipeline::new();
    let mut prefs = PreferenceSet::new();
    prefs.add(codebro::preference_engine::Preference::new(
        "model",
        codebro::preference_engine::PreferenceCategory::Model,
        PreferenceValue::String("gpt-4o".to_string()),
        "Default model",
        codebro::preference_engine::PreferenceOrigin::Default,
    ));
    let config = ValidationConfig::new();

    let result = pipeline.run("Change the model to gpt-4o", &prefs, &config);

    // All stages should produce output
    assert!(!result.intent_plan.id.is_empty());
    assert!(!result.confidence_result.reasoning.is_empty());
    assert!(!result.workflow_result.plan.plan_id.is_empty());
    assert!(!result.validation_report.report_id.is_empty());
}

#[test]
fn test_pipeline_duration_is_reasonable() {
    let pipeline = IntegrationPipeline::new();
    let prefs = PreferenceSet::default();
    let config = ValidationConfig::new();

    let start = std::time::Instant::now();
    let result = pipeline.run("Change the model to gpt-4o", &prefs, &config);
    let elapsed = start.elapsed();

    // Should complete in under 100ms
    assert!(elapsed < Duration::from_millis(100));
    assert!(result.total_duration < Duration::from_millis(100));
}
