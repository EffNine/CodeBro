//! Workflow Validator — validates workflow plans for correctness.
//!
/// Detects duplicate steps, invalid commands, dependency failures,
/// conflicts, and unsupported workflows.
/// Returns structured issues.
use super::types::*;
use crate::intent_engine::IntentPlan;
use crate::recommendation_engine::RecommendationSet;

/// Validate input intent plan and recommendations.
pub fn validate_inputs(
    intent_plan: &IntentPlan,
    recommendations: Option<&RecommendationSet>,
) -> Vec<WorkflowIssue> {
    let mut issues = Vec::new();

    // Check for empty workflow
    if intent_plan.required_commands.is_empty() {
        if recommendations.map(|r| r.is_empty()).unwrap_or(true) {
            issues.push(WorkflowIssue::EmptyWorkflow);
        }
    }

    // Check for unknown intent type with no actionable commands
    if matches!(
        intent_plan.intent_type,
        crate::intent_engine::IntentType::Unknown
    ) && intent_plan.required_commands.is_empty()
    {
        issues.push(WorkflowIssue::UnsupportedWorkflow {
            reason: "Unknown intent with no commands".to_string(),
        });
    }

    // Validate recommendations if present
    if let Some(recs) = recommendations {
        for rec in &recs.recommendations {
            if rec.title.is_empty() {
                issues.push(WorkflowIssue::InvalidCommand {
                    step_id: "recommendation".to_string(),
                    reason: "Empty recommendation title".to_string(),
                });
            }
        }
    }

    issues
}

/// Validate a complete workflow plan.
pub fn validate_plan(
    steps: &[WorkflowStep],
    dependencies: &[WorkflowDependency],
) -> Vec<WorkflowIssue> {
    let mut issues = Vec::new();

    // Check for duplicates
    let mut seen_ids = std::collections::HashSet::new();
    for step in steps {
        if !seen_ids.insert(&step.step_id) {
            issues.push(WorkflowIssue::DuplicateStep {
                step_id: step.step_id.clone(),
            });
        }
    }

    // Check for dependency cycles
    if super::dependency::has_cycles(steps, dependencies) {
        // Find the cycle
        let cycle_steps = find_cycle_steps(steps, dependencies);
        issues.push(WorkflowIssue::DependencyCycle { steps: cycle_steps });
    }

    // Check for missing dependencies
    let step_ids: std::collections::HashSet<&str> =
        steps.iter().map(|s| s.step_id.as_str()).collect();
    for step in steps {
        for dep_id in &step.dependencies {
            if !step_ids.contains(dep_id.as_str()) {
                issues.push(WorkflowIssue::MissingDependency {
                    step_id: step.step_id.clone(),
                    missing: dep_id.clone(),
                });
            }
        }
    }

    // Check for conflicting commands
    check_conflicting_commands(steps, &mut issues);

    issues
}

/// Generate non-fatal warnings for the workflow.
pub fn generate_warnings(
    steps: &[WorkflowStep],
    dependencies: &[WorkflowDependency],
) -> Vec<WorkflowWarning> {
    let mut warnings = Vec::new();

    // Warn about long dependency chains
    let depth = super::dependency::calculate_depth(steps, dependencies);
    if depth > 5 {
        warnings.push(WorkflowWarning {
            warning_id: "long_chain".to_string(),
            message: format!("Long dependency chain detected: depth {}", depth),
            severity: WarningSeverity::Medium,
            step_id: None,
        });
    }

    // Warn about steps without dependencies in parallel mode
    if dependencies.is_empty() && steps.len() > 3 {
        warnings.push(WorkflowWarning {
            warning_id: "parallel_warning".to_string(),
            message: "Many independent steps detected — consider parallel execution".to_string(),
            severity: WarningSeverity::Info,
            step_id: None,
        });
    }

    // Warn about irreversible steps
    let irreversible_count = steps.iter().filter(|s| !s.reversible).count();
    if irreversible_count > 0 {
        warnings.push(WorkflowWarning {
            warning_id: "irreversible".to_string(),
            message: format!("{} irreversible step(s) detected", irreversible_count),
            severity: WarningSeverity::Low,
            step_id: None,
        });
    }

    warnings
}

/// Check for conflicting commands (same key, different values).
fn check_conflicting_commands(steps: &[WorkflowStep], issues: &mut Vec<WorkflowIssue>) {
    let mut key_values: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();

    for step in steps {
        // Extract key from command if it follows update_preference:key=value pattern
        if step.command.starts_with("update_preference:") {
            let rest = &step.command["update_preference:".len()..];
            if let Some(eq_pos) = rest.find('=') {
                let key = &rest[..eq_pos];
                key_values
                    .entry(key.to_string())
                    .or_insert_with(Vec::new)
                    .push(step.step_id.clone());
            }
        }
    }

    for (key, step_ids) in key_values {
        if step_ids.len() > 1 {
            // Multiple steps updating same key — potential conflict
            for i in 0..step_ids.len() {
                for j in (i + 1)..step_ids.len() {
                    issues.push(WorkflowIssue::ConflictingCommands {
                        step1: step_ids[i].clone(),
                        step2: step_ids[j].clone(),
                        reason: format!("Both steps update preference key: {}", key),
                    });
                }
            }
        }
    }
}

/// Find steps involved in a dependency cycle.
fn find_cycle_steps(steps: &[WorkflowStep], dependencies: &[WorkflowDependency]) -> Vec<String> {
    let mut cycle = Vec::new();
    let mut visited = std::collections::HashSet::new();
    let mut rec_stack = std::collections::HashSet::new();

    let step_ids: Vec<String> = steps.iter().map(|s| s.step_id.clone()).collect();

    for start in &step_ids {
        if !visited.contains(start) {
            if find_cycle_dfs(
                start,
                dependencies,
                &mut visited,
                &mut rec_stack,
                &mut cycle,
            ) {
                break;
            }
        }
    }

    cycle
}

fn find_cycle_dfs(
    node: &str,
    dependencies: &[WorkflowDependency],
    visited: &mut std::collections::HashSet<String>,
    rec_stack: &mut std::collections::HashSet<String>,
    cycle: &mut Vec<String>,
) -> bool {
    visited.insert(node.to_string());
    rec_stack.insert(node.to_string());

    let neighbors: Vec<&str> = dependencies
        .iter()
        .filter(|d| d.from_step == node)
        .map(|d| d.to_step.as_str())
        .collect();

    for neighbor in neighbors {
        if !visited.contains(neighbor) {
            if find_cycle_dfs(neighbor, dependencies, visited, rec_stack, cycle) {
                cycle.push(node.to_string());
                return true;
            }
        } else if rec_stack.contains(neighbor) {
            cycle.push(node.to_string());
            cycle.push(neighbor.to_string());
            return true;
        }
    }

    rec_stack.remove(node);
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_step(id: &str, command: &str, deps: Vec<&str>) -> WorkflowStep {
        WorkflowStep {
            step_id: id.to_string(),
            name: id.to_string(),
            command: command.to_string(),
            stage: WorkflowStage::Execution,
            priority: 0,
            dependencies: deps.into_iter().map(|s| s.to_string()).collect(),
            requires_approval: false,
            estimated_cost: 0.0,
            reversible: true,
            description: "Test".to_string(),
        }
    }

    #[test]
    fn test_validate_empty_inputs() {
        let issues = validate_inputs(
            &IntentPlan::new(
                "test".to_string(),
                "",
                crate::intent_engine::IntentType::Unknown,
                "unknown",
                false,
                0.0,
                0.1,
                true,
                None,
                "Empty",
                vec![],
                vec![],
            ),
            None,
        );
        assert!(!issues.is_empty());
    }

    #[test]
    fn test_validate_valid_plan() {
        let steps = vec![
            make_step("a", "cmd_a", vec![]),
            make_step("b", "cmd_b", vec!["a"]),
        ];
        let deps = vec![WorkflowDependency {
            from_step: "a".to_string(),
            to_step: "b".to_string(),
            dependency_type: DependencyType::MustCompleteBefore,
        }];
        let issues = validate_plan(&steps, &deps);
        assert!(issues.is_empty());
    }

    #[test]
    fn test_validate_duplicate_steps() {
        let steps = vec![
            make_step("dup", "cmd_1", vec![]),
            make_step("dup", "cmd_2", vec![]),
        ];
        let issues = validate_plan(&steps, &[]);
        assert!(!issues.is_empty());
        assert!(issues
            .iter()
            .any(|i| matches!(i, WorkflowIssue::DuplicateStep { .. })));
    }

    #[test]
    fn test_validate_cycle() {
        let steps = vec![
            make_step("a", "cmd_a", vec!["b"]),
            make_step("b", "cmd_b", vec!["a"]),
        ];
        let deps = vec![
            WorkflowDependency {
                from_step: "a".to_string(),
                to_step: "b".to_string(),
                dependency_type: DependencyType::MustCompleteBefore,
            },
            WorkflowDependency {
                from_step: "b".to_string(),
                to_step: "a".to_string(),
                dependency_type: DependencyType::MustCompleteBefore,
            },
        ];
        let issues = validate_plan(&steps, &deps);
        assert!(!issues.is_empty());
        assert!(issues
            .iter()
            .any(|i| matches!(i, WorkflowIssue::DependencyCycle { .. })));
    }

    #[test]
    fn test_validate_missing_dependency() {
        let steps = vec![make_step("a", "cmd_a", vec!["missing"])];
        let deps: Vec<WorkflowDependency> = vec![];
        let issues = validate_plan(&steps, &deps);
        assert!(!issues.is_empty());
        assert!(issues
            .iter()
            .any(|i| matches!(i, WorkflowIssue::MissingDependency { .. })));
    }

    #[test]
    fn test_generate_warnings_empty() {
        let steps: Vec<WorkflowStep> = vec![];
        let warnings = generate_warnings(&steps, &[]);
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_generate_warnings_long_chain() {
        let steps = vec![
            make_step("a", "cmd_a", vec![]),
            make_step("b", "cmd_b", vec!["a"]),
            make_step("c", "cmd_c", vec!["b"]),
            make_step("d", "cmd_d", vec!["c"]),
            make_step("e", "cmd_e", vec!["d"]),
            make_step("f", "cmd_f", vec!["e"]),
        ];
        let deps = vec![
            WorkflowDependency {
                from_step: "a".to_string(),
                to_step: "b".to_string(),
                dependency_type: DependencyType::MustCompleteBefore,
            },
            WorkflowDependency {
                from_step: "b".to_string(),
                to_step: "c".to_string(),
                dependency_type: DependencyType::MustCompleteBefore,
            },
            WorkflowDependency {
                from_step: "c".to_string(),
                to_step: "d".to_string(),
                dependency_type: DependencyType::MustCompleteBefore,
            },
            WorkflowDependency {
                from_step: "d".to_string(),
                to_step: "e".to_string(),
                dependency_type: DependencyType::MustCompleteBefore,
            },
            WorkflowDependency {
                from_step: "e".to_string(),
                to_step: "f".to_string(),
                dependency_type: DependencyType::MustCompleteBefore,
            },
        ];
        let warnings = generate_warnings(&steps, &deps);
        assert!(!warnings.is_empty());
        assert!(warnings.iter().any(|w| w.warning_id == "long_chain"));
    }

    #[test]
    fn test_validate_conflicting_commands() {
        let steps = vec![
            WorkflowStep {
                step_id: "a".to_string(),
                name: "a".to_string(),
                command: "update_preference:model=gpt-4o".to_string(),
                stage: WorkflowStage::Execution,
                priority: 0,
                dependencies: vec![],
                requires_approval: false,
                estimated_cost: 0.0,
                reversible: true,
                description: "A".to_string(),
            },
            WorkflowStep {
                step_id: "b".to_string(),
                name: "b".to_string(),
                command: "update_preference:model=claude".to_string(),
                stage: WorkflowStage::Execution,
                priority: 1,
                dependencies: vec![],
                requires_approval: false,
                estimated_cost: 0.0,
                reversible: true,
                description: "B".to_string(),
            },
        ];
        let issues = validate_plan(&steps, &[]);
        assert!(!issues.is_empty());
        assert!(issues
            .iter()
            .any(|i| matches!(i, WorkflowIssue::ConflictingCommands { .. })));
    }
}
