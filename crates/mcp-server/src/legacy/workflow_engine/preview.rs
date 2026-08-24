//! Workflow Preview — human-readable workflow preview generation.
//!
/// Generates a readable summary of a workflow plan for the Approval Gate.
/// Read-only, no mutations.
use super::types::*;

/// Generate a human-readable preview of a workflow plan.
pub fn generate_preview(plan: &WorkflowPlan) -> String {
    let mut lines: Vec<String> = Vec::new();

    lines.push("=".repeat(60));
    lines.push(format!("  Workflow Plan: {}", plan.plan_id));
    lines.push(format!("  Intent ID: {}", plan.intent_id));
    lines.push("=".repeat(60));
    lines.push(String::new());

    // Summary
    lines.push(format!("Strategy: {}", plan.strategy));
    lines.push(format!("Total Steps: {}", plan.total_steps));
    lines.push(format!("Estimated Cost: ${:.2}", plan.total_estimated_cost));
    lines.push(format!("Valid: {}", plan.is_valid));
    lines.push(String::new());

    // Steps
    lines.push("-".repeat(60));
    lines.push("  STEPS".to_string());
    lines.push("-".repeat(60));

    for (i, step) in plan.steps.iter().enumerate() {
        lines.push(format!("[{}] {}", i + 1, step.name));
        lines.push(format!("     Stage: {}", step.stage));
        lines.push(format!("     Command: {}", step.command));
        lines.push(format!("     Priority: {}", step.priority));
        if !step.dependencies.is_empty() {
            lines.push(format!("     Dependencies: {:?}", step.dependencies));
        }
        lines.push(format!(
            "     Approval Required: {}",
            step.requires_approval
        ));
        lines.push(format!("     Reversible: {}", step.reversible));
        if step.estimated_cost > 0.0 {
            lines.push(format!("     Estimated Cost: ${:.2}", step.estimated_cost));
        }
        lines.push(String::new());
    }

    // Issues
    if !plan.issues.is_empty() {
        lines.push("-".repeat(60));
        lines.push("  ISSUES".to_string());
        lines.push("-".repeat(60));
        for issue in &plan.issues {
            lines.push(format!("  [ERROR] {}", issue));
        }
        lines.push(String::new());
    }

    // Warnings
    if !plan.warnings.is_empty() {
        lines.push("-".repeat(60));
        lines.push("  WARNINGS".to_string());
        lines.push("-".repeat(60));
        for warning in &plan.warnings {
            lines.push(format!("  [{}] {}", warning.severity, warning.message));
        }
        lines.push(String::new());
    }

    // Dependencies
    if !plan.dependencies.is_empty() {
        lines.push("-".repeat(60));
        lines.push("  DEPENDENCIES".to_string());
        lines.push("-".repeat(60));
        for dep in &plan.dependencies {
            lines.push(format!(
                "  {} → {} ({})",
                dep.from_step, dep.to_step, dep.dependency_type
            ));
        }
        lines.push(String::new());
    }

    // Approval summary
    let approval_steps: Vec<_> = plan.steps.iter().filter(|s| s.requires_approval).collect();
    lines.push("-".repeat(60));
    lines.push("  APPROVAL SUMMARY".to_string());
    lines.push("-".repeat(60));
    lines.push(format!(
        "  Steps requiring approval: {}",
        approval_steps.len()
    ));
    lines.push(format!("  Total steps: {}", plan.total_steps));
    if plan.is_valid {
        lines.push("  Status: READY FOR APPROVAL".to_string());
    } else {
        lines.push("  Status: BLOCKED — Fix issues before approval".to_string());
    }
    lines.push("=".repeat(60));

    lines.join("\n")
}

/// Generate a compact preview for quick display.
pub fn generate_compact_preview(plan: &WorkflowPlan) -> String {
    let mut summary = format!(
        "Workflow [{}]: {} steps, strategy={}, valid={}, cost=${:.2}",
        plan.plan_id, plan.total_steps, plan.strategy, plan.is_valid, plan.total_estimated_cost
    );

    if !plan.issues.is_empty() {
        summary.push_str(&format!(", issues={}", plan.issues.len()));
    }
    if !plan.warnings.is_empty() {
        summary.push_str(&format!(", warnings={}", plan.warnings.len()));
    }

    summary
}

/// Generate an approval summary for the Approval Gate.
pub fn generate_approval_summary(plan: &WorkflowPlan) -> String {
    let mut lines = Vec::new();

    lines.push(format!("Approval Request: {}", plan.plan_id));
    lines.push(format!("  Intent: {}", plan.intent_id));
    lines.push(format!("  Strategy: {}", plan.strategy));
    lines.push(format!(
        "  Steps: {} total, {} require approval",
        plan.total_steps,
        plan.steps.iter().filter(|s| s.requires_approval).count()
    ));
    lines.push(format!(
        "  Estimated Cost: ${:.2}",
        plan.total_estimated_cost
    ));
    lines.push(format!(
        "  Reversible: {}",
        plan.steps.iter().all(|s| s.reversible)
    ));

    if plan.is_valid {
        lines.push("  Status: APPROVAL RECOMMENDED".to_string());
    } else {
        lines.push("  Status: APPROVAL BLOCKED".to_string());
        for issue in &plan.issues {
            lines.push(format!("    - {}", issue));
        }
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_preview_valid_plan() {
        let plan = WorkflowPlan::new(
            "preview-1".to_string(),
            "intent-1",
            vec![
                WorkflowStep::new(
                    "Step 1",
                    "cmd1",
                    WorkflowStage::Preparation,
                    0,
                    vec![],
                    false,
                    0.0,
                    true,
                    "Prep",
                ),
                WorkflowStep::new(
                    "Step 2",
                    "cmd2",
                    WorkflowStage::Execution,
                    1,
                    vec!["step_1".to_string()],
                    true,
                    0.5,
                    true,
                    "Exec",
                ),
            ],
            vec![WorkflowDependency {
                from_step: "step_1".to_string(),
                to_step: "step_2".to_string(),
                dependency_type: DependencyType::MustCompleteBefore,
            }],
            ExecutionStrategy::DependencyOrdered,
            vec![],
            vec![],
        );
        let preview = generate_preview(&plan);
        assert!(preview.contains("preview-1"));
        assert!(preview.contains("cmd1"));
        assert!(preview.contains("cmd2"));
        assert!(preview.contains("READY FOR APPROVAL"));
    }

    #[test]
    fn test_generate_preview_invalid_plan() {
        let plan = WorkflowPlan::new(
            "preview-2".to_string(),
            "intent-2",
            vec![],
            vec![],
            ExecutionStrategy::Sequential,
            vec![WorkflowIssue::EmptyWorkflow],
            vec![],
        );
        let preview = generate_preview(&plan);
        assert!(preview.contains("BLOCKED"));
    }

    #[test]
    fn test_generate_compact_preview() {
        let plan = WorkflowPlan::new(
            "compact-1".to_string(),
            "intent-1",
            vec![WorkflowStep::new(
                "S1",
                "cmd1",
                WorkflowStage::Execution,
                0,
                vec![],
                false,
                0.0,
                true,
                "",
            )],
            vec![],
            ExecutionStrategy::Sequential,
            vec![],
            vec![],
        );
        let compact = generate_compact_preview(&plan);
        assert!(compact.contains("compact-1"));
        assert!(compact.contains("1 steps"));
    }

    #[test]
    fn test_generate_approval_summary() {
        let plan = WorkflowPlan::new(
            "approval-1".to_string(),
            "intent-1",
            vec![WorkflowStep::new(
                "S1",
                "cmd1",
                WorkflowStage::Execution,
                0,
                vec![],
                true,
                0.0,
                true,
                "",
            )],
            vec![],
            ExecutionStrategy::Sequential,
            vec![],
            vec![],
        );
        let summary = generate_approval_summary(&plan);
        assert!(summary.contains("approval-1"));
        assert!(summary.contains("APPROVAL RECOMMENDED"));
    }
}
