//! Workflow Engine Types — core data model.
//!
//! All types are immutable, serializable, and deterministic.
//! No timestamps, no UUIDs, no randomness.

use serde::{Deserialize, Serialize};
use std::fmt;

// ─── Workflow Stage ─────────────────────────────────────────────────────────

/// Logical stage in a workflow.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WorkflowStage {
    /// Preparation stage — setup, validation, resource checking.
    Preparation,
    /// Main execution stage — primary command execution.
    Execution,
    /// Validation stage — verify results, run checks.
    Validation,
    /// Cleanup stage — teardown, cleanup, notification.
    Cleanup,
    /// Rollback stage — undo changes if needed.
    Rollback,
}

impl fmt::Display for WorkflowStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkflowStage::Preparation => write!(f, "preparation"),
            WorkflowStage::Execution => write!(f, "execution"),
            WorkflowStage::Validation => write!(f, "validation"),
            WorkflowStage::Cleanup => write!(f, "cleanup"),
            WorkflowStage::Rollback => write!(f, "rollback"),
        }
    }
}

// ─── Execution Strategy ──────────────────────────────────────────────────────

/// How workflow steps are executed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionStrategy {
    /// Execute steps sequentially.
    Sequential,
    /// Execute independent steps in parallel.
    Parallel,
    /// Execute based on dependency graph.
    DependencyOrdered,
}

impl fmt::Display for ExecutionStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExecutionStrategy::Sequential => write!(f, "sequential"),
            ExecutionStrategy::Parallel => write!(f, "parallel"),
            ExecutionStrategy::DependencyOrdered => write!(f, "dependency_ordered"),
        }
    }
}

// ─── Workflow Step ───────────────────────────────────────────────────────────

/// A single step in a workflow plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowStep {
    /// Stable step identifier (deterministic, not random).
    pub step_id: String,
    /// Display name for the step.
    pub name: String,
    /// Command to execute.
    pub command: String,
    /// Stage this step belongs to.
    pub stage: WorkflowStage,
    /// Priority (lower = earlier).
    pub priority: u32,
    /// Dependencies on other step IDs.
    pub dependencies: Vec<String>,
    /// Whether this step requires explicit approval.
    pub requires_approval: bool,
    /// Estimated cost impact.
    pub estimated_cost: f64,
    /// Whether this step is reversible.
    pub reversible: bool,
    /// Human-readable description.
    pub description: String,
}

impl WorkflowStep {
    /// Create a new workflow step with deterministic ID.
    pub fn new(
        name: &str,
        command: &str,
        stage: WorkflowStage,
        priority: u32,
        dependencies: Vec<String>,
        requires_approval: bool,
        estimated_cost: f64,
        reversible: bool,
        description: &str,
    ) -> Self {
        WorkflowStep {
            step_id: Self::generate_step_id(name),
            name: name.to_string(),
            command: command.to_string(),
            stage,
            priority,
            dependencies,
            requires_approval,
            estimated_cost,
            reversible,
            description: description.to_string(),
        }
    }

    /// Generate a deterministic step ID from the name.
    fn generate_step_id(name: &str) -> String {
        // Use a simple hash-like deterministic ID based on name
        let mut hash: u64 = 14695981039346656037;
        for byte in name.bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(1099511628211);
        }
        format!("step_{:x}", hash)
    }
}

// ─── Workflow Dependency ─────────────────────────────────────────────────────

/// A dependency relationship between two workflow steps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowDependency {
    pub from_step: String,
    pub to_step: String,
    pub dependency_type: DependencyType,
}

/// Type of dependency relationship.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DependencyType {
    /// Must complete before (hard dependency).
    MustCompleteBefore,
    /// Should complete before (soft dependency).
    ShouldCompleteBefore,
    /// Can run in parallel (no dependency).
    Independent,
}

impl fmt::Display for DependencyType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DependencyType::MustCompleteBefore => write!(f, "must_complete_before"),
            DependencyType::ShouldCompleteBefore => write!(f, "should_complete_before"),
            DependencyType::Independent => write!(f, "independent"),
        }
    }
}

// ─── Workflow Issue ──────────────────────────────────────────────────────────

/// A validation issue found during workflow planning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WorkflowIssue {
    /// Duplicate step detected.
    DuplicateStep { step_id: String },
    /// Invalid command type.
    InvalidCommand { step_id: String, reason: String },
    /// Dependency cycle detected.
    DependencyCycle { steps: Vec<String> },
    /// Missing dependency.
    MissingDependency { step_id: String, missing: String },
    /// Conflicting commands.
    ConflictingCommands {
        step1: String,
        step2: String,
        reason: String,
    },
    /// Empty workflow.
    EmptyWorkflow,
    /// Unsupported workflow type.
    UnsupportedWorkflow { reason: String },
    /// Invalid dependency ordering.
    InvalidDependencyOrder { step_id: String, reason: String },
}

impl fmt::Display for WorkflowIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkflowIssue::DuplicateStep { step_id } => write!(f, "Duplicate step: {}", step_id),
            WorkflowIssue::InvalidCommand { step_id, reason } => {
                write!(f, "Invalid command in step {}: {}", step_id, reason)
            }
            WorkflowIssue::DependencyCycle { steps } => {
                write!(f, "Dependency cycle detected: {:?}", steps)
            }
            WorkflowIssue::MissingDependency { step_id, missing } => {
                write!(f, "Missing dependency '{}' for step {}", missing, step_id)
            }
            WorkflowIssue::ConflictingCommands {
                step1,
                step2,
                reason,
            } => {
                write!(
                    f,
                    "Conflicting commands: {} vs {} — {}",
                    step1, step2, reason
                )
            }
            WorkflowIssue::EmptyWorkflow => write!(f, "Empty workflow"),
            WorkflowIssue::UnsupportedWorkflow { reason } => {
                write!(f, "Unsupported workflow: {}", reason)
            }
            WorkflowIssue::InvalidDependencyOrder { step_id, reason } => {
                write!(f, "Invalid dependency order for {}: {}", step_id, reason)
            }
        }
    }
}

// ─── Workflow Warning ────────────────────────────────────────────────────────

/// A non-fatal warning during workflow planning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowWarning {
    pub warning_id: String,
    pub message: String,
    pub severity: WarningSeverity,
    pub step_id: Option<String>,
}

/// Severity of a workflow warning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WarningSeverity {
    Info,
    Low,
    Medium,
    High,
}

impl fmt::Display for WorkflowWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.severity, self.message)
    }
}

impl fmt::Display for WarningSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WarningSeverity::Info => write!(f, "INFO"),
            WarningSeverity::Low => write!(f, "LOW"),
            WarningSeverity::Medium => write!(f, "MEDIUM"),
            WarningSeverity::High => write!(f, "HIGH"),
        }
    }
}

// ─── Workflow Plan ───────────────────────────────────────────────────────────

/// Complete workflow plan produced by the planner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowPlan {
    /// Deterministic plan identifier.
    pub plan_id: String,
    /// Original intent ID this plan addresses.
    pub intent_id: String,
    /// Ordered list of steps.
    pub steps: Vec<WorkflowStep>,
    /// Dependency graph.
    pub dependencies: Vec<WorkflowDependency>,
    /// Execution strategy.
    pub strategy: ExecutionStrategy,
    /// Validation issues found.
    pub issues: Vec<WorkflowIssue>,
    /// Warnings (non-fatal).
    pub warnings: Vec<WorkflowWarning>,
    /// Total estimated cost.
    pub total_estimated_cost: f64,
    /// Total estimated steps.
    pub total_steps: usize,
    /// Whether the plan is valid and can proceed.
    pub is_valid: bool,
    /// Deterministic summary.
    pub summary: String,
}

impl WorkflowPlan {
    pub fn new(
        plan_id: String,
        intent_id: &str,
        steps: Vec<WorkflowStep>,
        dependencies: Vec<WorkflowDependency>,
        strategy: ExecutionStrategy,
        issues: Vec<WorkflowIssue>,
        warnings: Vec<WorkflowWarning>,
    ) -> Self {
        let total_estimated_cost: f64 = steps.iter().map(|s| s.estimated_cost).sum();
        let total_steps = steps.len();
        let is_valid = issues.is_empty() && !Self::has_cycles(&steps, &dependencies);

        let summary = if is_valid {
            format!(
                "Valid workflow: {} steps, strategy={}, cost={:.2}",
                total_steps, strategy, total_estimated_cost
            )
        } else {
            format!(
                "Invalid workflow: {} issues, {} warnings",
                issues.len(),
                warnings.len()
            )
        };

        WorkflowPlan {
            plan_id,
            intent_id: intent_id.to_string(),
            steps,
            dependencies,
            strategy,
            issues,
            warnings,
            total_estimated_cost,
            total_steps,
            is_valid,
            summary,
        }
    }

    /// Check for dependency cycles using DFS.
    fn has_cycles(steps: &[WorkflowStep], dependencies: &[WorkflowDependency]) -> bool {
        let mut adj: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for dep in dependencies {
            adj.entry(dep.to_step.clone())
                .or_insert_with(Vec::new)
                .push(dep.from_step.clone());
        }

        let mut visited = std::collections::HashSet::new();
        let mut rec_stack = std::collections::HashSet::new();

        fn dfs(
            node: &str,
            adj: &std::collections::HashMap<String, Vec<String>>,
            visited: &mut std::collections::HashSet<String>,
            rec_stack: &mut std::collections::HashSet<String>,
        ) -> bool {
            visited.insert(node.to_string());
            rec_stack.insert(node.to_string());

            if let Some(neighbors) = adj.get(node) {
                for neighbor in neighbors {
                    if !visited.contains(neighbor) {
                        if dfs(neighbor, adj, visited, rec_stack) {
                            return true;
                        }
                    } else if rec_stack.contains(neighbor) {
                        return true;
                    }
                }
            }

            rec_stack.remove(node);
            false
        }

        for step in steps {
            if !visited.contains(&step.step_id) {
                if dfs(&step.step_id, &adj, &mut visited, &mut rec_stack) {
                    return true;
                }
            }
        }

        false
    }
}

// ─── Workflow Metadata ───────────────────────────────────────────────────────

/// Metadata about the workflow planning process.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkflowMetadata {
    pub source_intent: String,
    pub source_recommendation_count: usize,
    pub planner_version: String,
    pub planning_rules_applied: Vec<String>,
}

// ─── Workflow Result ─────────────────────────────────────────────────────────

/// Result of workflow planning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowResult {
    pub plan: WorkflowPlan,
    pub metadata: WorkflowMetadata,
    pub validation_passed: bool,
    pub approval_required: bool,
}

impl WorkflowResult {
    pub fn new(plan: WorkflowPlan, metadata: WorkflowMetadata) -> Self {
        let validation_passed = !plan
            .issues
            .iter()
            .any(|i| matches!(i, WorkflowIssue::DependencyCycle { .. }));
        let approval_required = plan.steps.iter().any(|s| s.requires_approval);
        WorkflowResult {
            plan,
            metadata,
            validation_passed,
            approval_required,
        }
    }
}

// ─── Rollback Plan ───────────────────────────────────────────────────────────

/// Plan for undoing workflow changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackPlan {
    pub reverse_steps: Vec<WorkflowStep>,
    pub strategy: RollbackStrategy,
}

/// How to perform rollback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RollbackStrategy {
    /// Undo steps in reverse order.
    ReverseOrder,
    /// Execute dedicated rollback commands.
    DedicatedCommands,
    /// Restore from snapshot.
    SnapshotRestore,
}

// ─── Workflow Summary ────────────────────────────────────────────────────────

/// Human-readable summary of a workflow plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowSummary {
    pub total_steps: usize,
    pub total_cost: f64,
    pub strategy: String,
    pub issue_count: usize,
    pub warning_count: usize,
    pub approval_required: bool,
    pub is_valid: bool,
    pub stages: Vec<String>,
}

impl WorkflowSummary {
    pub fn from_plan(plan: &WorkflowPlan) -> Self {
        let mut stages: Vec<String> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for step in &plan.steps {
            let stage = step.stage.to_string();
            if seen.insert(stage.clone()) {
                stages.push(stage);
            }
        }
        stages.sort();

        WorkflowSummary {
            total_steps: plan.total_steps,
            total_cost: plan.total_estimated_cost,
            strategy: plan.strategy.to_string(),
            issue_count: plan.issues.len(),
            warning_count: plan.warnings.len(),
            approval_required: plan.steps.iter().any(|s| s.requires_approval),
            is_valid: plan.is_valid,
            stages,
        }
    }
}
